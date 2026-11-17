//! Web Requests plugin for KnobDeck: call any HTTP endpoint from a key (webhooks for IFTTT, n8n,
//! Zapier, Home Assistant, a local server…), show a value from a JSON reply on a key, and switch
//! something on and off with two requests. URLs, headers and bodies take KnobDeck placeholders.

mod http;

use http::{Reply, Request};
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::os::placeholders::{self, Escape, Name};
use knobdeck_sdk::{json, load_env, Action, Ctx, Error, InitialCtx, Plugin, Result, Value, Visual};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(15);

// ── Inputs ────────────────────────────────────────────────────────────────────

fn method_input(key: &str, label: &str, default: &str) -> Value {
    input::select(key, label, http::METHODS).default_value(default)
}

fn body_type_input() -> Value {
    input::select("body_type", "Body is", &[("json", "JSON"), ("form", "Form (a=1&b=2)"), ("text", "Plain text")]).default_value("json")
}

fn headers_input() -> Value {
    input::textarea("headers", "Headers (optional)")
        .placeholder("Accept: application/json\nX-Api-Key: …")
        .with("help", "One Name: value per line. Keep tokens out of here: save them in this plugin's setup and pick them under Token.")
        .accepts_placeholders()
}

fn token_input() -> Value {
    input::query_select("token", "Token (optional)", "tokens", &[]).with("help", "A token saved in this plugin's setup, sent as a header.")
}

fn token_header_input() -> Value {
    input::text("token_header", "Send the token as")
        .placeholder("Authorization: Bearer")
        .default_value("Authorization: Bearer")
        .with("help", "The header name, a colon, and any text before the token (\"Authorization: Bearer\", \"X-Api-Key:\").")
}

/// The inputs of one request; `prefix` keeps the On and Off requests of a toggle apart.
fn request_inputs(prefix: &str, label: &str, default_method: &str) -> Vec<Value> {
    let k = |s: &str| format!("{prefix}{s}");
    vec![
        method_input(&k("method"), &format!("{label}method"), default_method),
        input::text(&k("url"), &format!("{label}URL")).placeholder("https://example.com/hook").accepts_placeholders(),
        input::textarea(&k("body"), &format!("{label}body (optional)"))
            .placeholder("{\"text\": \"{selection}\"}")
            .with("help", "Placeholders in a JSON body are escaped for JSON, in a form body for a URL.")
            .accepts_placeholders(),
    ]
}

fn show_inputs() -> Vec<Value> {
    vec![
        input::select("show", "After the request, show", &[("none", "Nothing (a check mark on success)"), ("status", "The status code"), ("value", "A value from the JSON reply"), ("text", "The reply's text")])
            .default_value("none"),
        input::text("path", "Value path").placeholder("data.items[0].name").with("showIf", json!({"key": "show", "equals": "value"})),
    ]
}

// ── Building and sending requests ─────────────────────────────────────────────

/// An input's text ("" when absent), for code that has only the inputs.
fn inp(inputs: &Value, key: &str) -> String {
    match &inputs[key] {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

/// The value a placeholder stands for, read from this press's context.
fn context_value(ctx: &Ctx, name: Name) -> String {
    match name {
        Name::Selection => ctx.selection().unwrap_or_default().to_string(),
        Name::Files | Name::File => ctx.files().iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n"),
        Name::Clipboard => ctx.clipboard_text().unwrap_or_default().to_string(),
        Name::App => ctx.window().map(|w| w.app_name.clone().unwrap_or_else(|| w.app_id.clone())).unwrap_or_default(),
        Name::Title => ctx.window().and_then(|w| w.title.clone()).unwrap_or_default(),
        Name::Url => ctx.browser_tab().and_then(|t| t.url.clone()).unwrap_or_default(),
        Name::Document => ctx.document().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        _ => String::new(), // dates, times and input boxes are filled in by the parser
    }
}

/// Expand placeholders with every value escaped for a JSON string (the user writes the quotes).
fn expand_json(ctx: &Ctx, text: &str) -> Result<String> {
    placeholders::expand_mapped(text, Escape::Raw, |n| Ok(context_value(ctx, n)), |_, v| http::json_escape(&v)).map_err(|e| {
        if knobdeck_sdk::os::input_box::is_cancelled(&e) {
            Error::Fail(e.message().into()) // cancelled input box: stop quietly
        } else {
            Error::msg(e.message())
        }
    })
}

/// The token header from setup, if this key picks one.
fn token_header(inputs: &Value) -> Result<Option<(String, String)>> {
    let slot = inp(inputs, "token");
    let slot: usize = match slot.trim().parse() {
        Ok(n) if (1..=http::TOKEN_SLOTS).contains(&n) => n,
        _ => return Ok(None),
    };
    let token = knobdeck_sdk::secrets::env_or_stored(&http::token_key(slot));
    if token.is_empty() {
        return Err(Error::msg(format!("Token {slot} is empty. Save it in this plugin's setup")));
    }
    let spec = inp(inputs, "token_header");
    let spec = if spec.trim().is_empty() { "Authorization: Bearer".to_string() } else { spec };
    let (name, prefix) = spec.split_once(':').unwrap_or((spec.as_str(), ""));
    let name = name.trim();
    if name.is_empty() || name.contains(' ') {
        return Err(Error::msg("\"Send the token as\" must start with a header name, like Authorization: Bearer"));
    }
    let prefix = prefix.trim();
    Ok(Some((name.to_string(), if prefix.is_empty() { token } else { format!("{prefix} {token}") })))
}

/// Build the request `prefix` names from a key press, placeholders expanded.
fn build(ctx: &Ctx, prefix: &str) -> Result<Request> {
    let k = |s: &str| format!("{prefix}{s}");
    let url = ctx.expand_as(&ctx.input_str(&k("url")), Escape::Url)?;
    let mut headers = http::parse_headers(&ctx.expand(&ctx.input_str("headers"))?)?;
    if let Some(h) = token_header(&ctx.inputs)? {
        headers.push(h);
    }
    let method = ctx.input_str(&k("method")).to_uppercase();
    let raw_body = ctx.input_str(&k("body"));
    let (body, content_type) = if raw_body.trim().is_empty() || method == "GET" || method == "HEAD" {
        (None, None)
    } else {
        match ctx.input_str("body_type").as_str() {
            "form" => (Some(ctx.expand_as(&raw_body, Escape::Url)?), Some("application/x-www-form-urlencoded")),
            "text" => (Some(ctx.expand(&raw_body)?), Some("text/plain; charset=utf-8")),
            _ => {
                let body = expand_json(ctx, &raw_body)?;
                if serde_json::from_str::<Value>(&body).is_err() {
                    return Err(Error::msg("The body isn't valid JSON. Check the quotes and commas, or set \"Body is\" to Plain text"));
                }
                (Some(body), Some("application/json"))
            }
        }
    };
    Ok(Request { method, url, headers, body, content_type })
}

/// Build a request from saved inputs only (live keys: no press, so no placeholders that read
/// the screen; dates and times still work).
fn build_static(inputs: &Value, prefix: &str) -> Result<Request> {
    let s = |key: &str| inp(inputs, key);
    let plain = |t: &str, esc| placeholders::expand(t, esc, |_| Ok(String::new())).map_err(|e| Error::msg(e.message()));
    let mut headers = http::parse_headers(&plain(&s("headers"), Escape::Raw)?)?;
    if let Some(h) = token_header(inputs)? {
        headers.push(h);
    }
    let method = s(&format!("{prefix}method")).to_uppercase();
    Ok(Request { method: if method.is_empty() { "GET".into() } else { method }, url: plain(&s(&format!("{prefix}url")), Escape::Url)?, headers, body: None, content_type: None })
}

fn send_ok(req: &Request) -> Result<Reply> {
    let reply = http::send(req, TIMEOUT)?;
    if reply.ok() {
        Ok(reply)
    } else {
        Err(http::status_error(&reply))
    }
}

/// The text a key shows for a reply, per the "show" input.
fn shown(inputs: &Value, reply: &Reply) -> Result<Option<String>> {
    let s = |key: &str| inp(inputs, key);
    Ok(match s("show").as_str() {
        "status" => Some(reply.status.to_string()),
        "text" => Some(http::one_line(&reply.body, 60)),
        "value" => Some(value_at(reply, &s("path"))?),
        _ => None,
    })
}

fn value_at(reply: &Reply, path: &str) -> Result<String> {
    let json = reply.json().ok_or_else(|| Error::msg("The reply isn't JSON"))?;
    http::json_path(&json, path).map(http::display).ok_or_else(|| Error::msg(format!("Nothing at \"{}\" in the reply", path.trim())))
}

// ── Show Value: one fetch per URL and interval, shared by keys ────────────────

struct Cached {
    at: Instant,
    reply: std::result::Result<std::sync::Arc<Reply>, String>,
}

static CACHE: Mutex<Option<HashMap<String, Cached>>> = Mutex::new(None);

fn interval_of(inputs: &Value) -> Duration {
    let secs = inp(inputs, "every").parse::<u64>().unwrap_or(30).max(2);
    Duration::from_secs(secs)
}

/// The last reply for `req`, fetched again when it is older than `max_age`.
fn cached_reply(req: &Request, max_age: Duration) -> Result<std::sync::Arc<Reply>> {
    let key = format!("{} {} {:?}", req.method, req.url, req.headers);
    if let Some(c) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(&key) {
        if c.at.elapsed() < max_age {
            return c.reply.clone().map_err(Error::msg);
        }
    }
    let reply = send_ok(req).map(std::sync::Arc::new).map_err(|e| e.message().to_string());
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, Cached { at: Instant::now(), reply: reply.clone() });
    reply.map_err(Error::msg)
}

fn forget_cache() {
    if let Some(m) = CACHE.lock().unwrap().as_mut() {
        m.clear();
    }
}

fn value_visual(ic: &InitialCtx) -> Result<Visual> {
    let req = build_static(&ic.inputs, "")?;
    let reply = cached_reply(&req, interval_of(&ic.inputs))?;
    let text = value_at(&reply, &ic.input_str("path"))?;
    let label = format!("{}{text}{}", ic.input_str("prefix"), ic.input_str("suffix"));
    let title = ic.input_str("title");
    Ok(Visual::new().label(if title.trim().is_empty() { label } else { format!("{}\n{label}", title.trim()) }))
}

fn toggle_state(ic: &InitialCtx) -> Result<Option<bool>> {
    if ic.input_str("status_url").trim().is_empty() {
        return Ok(None);
    }
    let mut inputs = ic.inputs.clone();
    inputs["status_method"] = json!("GET");
    let req = build_static(&inputs, "status_")?;
    let reply = cached_reply(&req, Duration::from_secs(4))?;
    let json = reply.json().ok_or_else(|| Error::msg("The status reply isn't JSON"))?;
    let path = ic.input_str("status_path");
    let v = http::json_path(&json, &path).ok_or_else(|| Error::msg(format!("Nothing at \"{}\" in the status reply", path.trim())))?;
    Ok(Some(http::is_on(v, &ic.input_str("on_value"))))
}

// ── Plugin ────────────────────────────────────────────────────────────────────

fn main() {
    load_env();
    Plugin::new(&format!("web-requests-{}", std::env::consts::OS), "Web Requests")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Call any web address from a key: webhooks (IFTTT, n8n, Zapier, Make), REST APIs, local servers. Show a value from a JSON reply on a key, and switch things on and off.")
        .icon("globe")
        .setup("web-requests-setup", None)
        .setup_inputs(
            (1..=http::TOKEN_SLOTS)
                .flat_map(|n| {
                    vec![
                        input::text(&http::token_name_key(n), &format!("Token {n} name (optional)")).placeholder(if n == 1 { "GitHub" } else { "" }),
                        input::text(&http::token_key(n), &format!("Token {n}")).with(
                            "help",
                            if n == 1 { "Tokens are kept in this computer's credential store, never in your layout. Leave all empty if your requests need none." } else { "" },
                        ),
                    ]
                })
                .collect(),
        )
        .query("tokens", |_| {
            let mut opts = vec![json!({"value": "", "label": "None"})];
            for n in 1..=http::TOKEN_SLOTS {
                let name = knobdeck_sdk::env_or(&http::token_name_key(n), "");
                if !name.is_empty() || !knobdeck_sdk::secrets::env_or_stored(&http::token_key(n)).is_empty() {
                    opts.push(json!({"value": n.to_string(), "label": if name.is_empty() { format!("Token {n}") } else { name }}));
                }
            }
            Ok(Value::Array(opts))
        })
        .action(
            Action::button("request", "HTTP Request")
                .icon("send")
                .inputs([request_inputs("", "", "POST"), vec![body_type_input(), headers_input(), token_input(), token_header_input()], show_inputs()].concat())
                .on_press(|ctx| {
                    let req = build(ctx, "")?;
                    let reply = send_ok(&req)?;
                    match shown(&ctx.inputs, &reply)? {
                        Some(text) => ctx.set_label(text),
                        None => {
                            ctx.set_icon("check");
                            let rt = ctx.runtime().clone();
                            let id = ctx.instance_id.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(Duration::from_millis(1200));
                                rt.set_visual(&id, Visual::new().icon("send"));
                            });
                        }
                    }
                    ctx.log(format!("{} {} → {}", req.method, req.url, reply.status));
                    Ok(())
                }),
        )
        .action(
            Action::button("value", "Show Value")
                .icon("braces")
                .inputs(vec![
                    input::text("url", "URL").placeholder("https://api.example.com/status").with("help", "Called with GET. {date} and {time} work here; placeholders that read the screen don't (nothing is pressed)."),
                    input::text("path", "Value path").placeholder("data.temperature").with("help", "Where the value sits in the JSON reply: names joined by dots, [0] for the first item of a list. Empty: the whole reply."),
                    input::text("title", "Title (optional)").placeholder("Server"),
                    input::text("prefix", "Before the value (optional)").placeholder("$"),
                    input::text("suffix", "After the value (optional)").placeholder(" °C"),
                    input::select("every", "Refresh every", &[("5", "5 seconds"), ("15", "15 seconds"), ("30", "30 seconds"), ("60", "1 minute"), ("300", "5 minutes"), ("900", "15 minutes")]).default_value("30"),
                    headers_input(),
                    token_input(),
                    token_header_input(),
                ])
                .live(Duration::from_secs(2), value_visual)
                .on_press(|ctx| {
                    // Refresh now.
                    forget_cache();
                    let ic = InitialCtx { instance_id: ctx.instance_id.clone(), action_id: ctx.action_id.clone(), inputs: ctx.inputs.clone() };
                    ctx.set_visual(value_visual(&ic)?);
                    Ok(())
                }),
        )
        .action(
            Action::toggle("toggle", "HTTP Toggle")
                .icon("toggle-right")
                .state("off", "Off", "toggle-left")
                .state("on", "On", "toggle-right")
                .inputs(
                    [
                        request_inputs("on_", "Turn on: ", "POST"),
                        request_inputs("off_", "Turn off: ", "POST"),
                        vec![
                            body_type_input(),
                            headers_input(),
                            token_input(),
                            token_header_input(),
                            input::text("status_url", "Read the state from (optional)")
                                .placeholder("https://example.com/api/state")
                                .with("help", "A URL whose JSON reply says whether it is on. Without it the key just alternates."),
                            input::text("status_path", "State path").placeholder("state").with("help", "true, \"on\", a nonzero number… count as on."),
                            input::text("on_value", "On when the value is (optional)").placeholder("heat"),
                        ],
                    ]
                    .concat(),
                )
                .live(Duration::from_secs(5), |ic| {
                    Ok(match toggle_state(ic)? {
                        Some(on) => Visual::new().state(if on { "on" } else { "off" }),
                        None => Visual::new(),
                    })
                })
                .on_press(|ctx| {
                    let turn_on = ctx.state.as_deref() != Some("on");
                    let req = build(ctx, if turn_on { "on_" } else { "off_" })?;
                    send_ok(&req)?;
                    forget_cache();
                    ctx.set_state(if turn_on { "on" } else { "off" });
                    Ok(())
                }),
        )
        .run()
}
