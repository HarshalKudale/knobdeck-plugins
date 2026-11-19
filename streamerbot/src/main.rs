//! Streamer.bot plugin for KnobDeck: run Streamer.bot actions (with arguments) and code triggers,
//! a wheel of an action group, and show a global variable on a key, over Streamer.bot's own
//! WebSocket server. Streamer.bot then reaches Twitch, YouTube, Kick, OBS and the rest.

mod sb;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use sb::Endpoint;
use std::time::Duration;

fn endpoint() -> Endpoint {
    Endpoint {
        host: env_or("STREAMERBOT_HOST", "127.0.0.1"),
        port: env_or("STREAMERBOT_PORT", "8080"),
        password: knobdeck_sdk::secrets::env_or_stored("STREAMERBOT_PASSWORD"),
    }
}

fn call(request: &str, body: Value) -> Result<Value> {
    sb::call(&endpoint(), request, body).map_err(Error::msg)
}

/// Every action: (id, name, group, enabled).
fn actions() -> Result<Vec<(String, String, String, bool)>> {
    let r = call("GetActions", json!({}))?;
    let mut list: Vec<_> = r["actions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some((
                a["id"].as_str()?.to_string(),
                a["name"].as_str().unwrap_or_default().to_string(),
                a["group"].as_str().unwrap_or_default().to_string(),
                a["enabled"].as_bool().unwrap_or(true),
            ))
        })
        .collect();
    list.sort_by(|a, b| (&a.2, &a.1).cmp(&(&b.2, &b.1)));
    Ok(list)
}

fn args(ctx: &Ctx) -> Result<Value> {
    sb::parse_args(&ctx.expand(&ctx.input_str("args"))?).map_err(Error::msg)
}

fn args_input() -> Value {
    input::text("args", "Arguments (optional)")
        .placeholder("user=somebody; amount=5")
        .with("help", "key=value pairs, separated by ; or new lines. The action reads them as %key%.")
        .accepts_placeholders()
}

fn do_action(id: &str, args: Value) -> Result<()> {
    if id.trim().is_empty() {
        return Err(Error::msg("Choose a Streamer.bot action"));
    }
    call("DoAction", json!({"action": {"id": id.trim()}, "args": args})).map(|_| ())
}

fn main() {
    knobdeck_sdk::load_env();
    Plugin::new("streamerbot-windows", "Streamer.bot")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&["windows"])
        .author("KnobDeck")
        .description("Run Streamer.bot actions and code triggers, a wheel of an action group, and show global variables. Streamer.bot reaches Twitch, YouTube, Kick, OBS and more.")
        .icon("app-icon.svg")
        .setup("streamerbot-setup", None)
        .setup_inputs(vec![
            input::text("host", "Host").placeholder("127.0.0.1").default_value("127.0.0.1"),
            input::text("port", "Port").placeholder("8080").default_value("8080"),
            input::text("password", "Password (only if authentication is on)").with("type", "password").with("help", "Kept in this computer's credential store."),
        ])
        .query("actions", |q| {
            let list = actions().or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(
                list.into_iter()
                    .map(|(id, name, group, enabled)| {
                        let label = if group.is_empty() { name } else { format!("{group} / {name}") };
                        json!({"value": id, "label": if enabled { label } else { format!("{label} (disabled)") }})
                    })
                    .collect(),
            ))
        })
        .query("code-triggers", |q| {
            let r = call("GetCodeTriggers", json!({})).or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(
                r["triggers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|t| {
                        let name = t["name"].as_str()?;
                        let label = match t["category"].as_str().filter(|c| !c.is_empty()) {
                            Some(c) => format!("{c} / {name}"),
                            None => name.to_string(),
                        };
                        Some(json!({"value": name, "label": label}))
                    })
                    .collect(),
            ))
        })
        .query("groups", |q| {
            let mut groups: Vec<String> = actions().or_else(|e| q.fail(e.message().to_string()))?.into_iter().map(|a| a.2).collect();
            groups.sort();
            groups.dedup();
            let mut out = vec![json!({"value": "", "label": "All actions"})];
            out.extend(groups.into_iter().filter(|g| !g.is_empty()).map(|g| json!({"value": g, "label": g})));
            Ok(Value::Array(out))
        })
        .action(
            Action::button("run-action", "Run Action")
                .icon("play")
                .inputs(vec![input::query_combobox("action", "Action", "actions"), args_input()])
                .on_press(|ctx| do_action(&ctx.input_str("action"), args(ctx)?)),
        )
        .action(
            Action::button("code-trigger", "Run Code Trigger")
                .icon("zap")
                .inputs(vec![
                    input::query_combobox("trigger", "Trigger", "code-triggers").with("help", "A custom trigger registered by a C# action (CPH.RegisterCustomTrigger)."),
                    args_input(),
                ])
                .on_press(|ctx| {
                    let name = ctx.input_str("trigger");
                    if name.trim().is_empty() {
                        return ctx.fail("Enter the trigger name");
                    }
                    call("ExecuteCodeTrigger", json!({"triggerName": name.trim(), "args": args(ctx)?})).map(|_| ())
                }),
        )
        .action(
            Action::button("global", "Global Variable")
                .icon("variable")
                .inputs(vec![
                    input::text("variable", "Variable name"),
                    input::checkbox("persisted", "Persisted (saved between restarts)").with("default", true),
                ])
                .live(Duration::from_secs(2), |ic| {
                    let name = ic.input_str("variable");
                    if name.trim().is_empty() {
                        return Err(Error::msg("Enter a variable name"));
                    }
                    let r = call("GetGlobal", json!({"variable": name.trim(), "persisted": ic.input_bool("persisted")}))?;
                    let v = &r["variable"]["value"];
                    let shown = match v {
                        Value::Null => "—".to_string(),
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    Ok(Visual::new().label(format!("{}\n{shown}", name.trim())))
                }),
        )
        .action(
            Action::wheel("actions-wheel", "Actions Wheel")
                .icon("bot")
                .inputs(vec![input::query_select("group", "Action group", "groups", &[])])
                .on_open(|o| {
                    let group = o.input_str("group");
                    let list = actions().or_else(|e| o.fail(e.message().to_string()))?;
                    Ok(list
                        .into_iter()
                        .filter(|a| a.3 && (group.is_empty() || a.2 == group))
                        .take(16)
                        .map(|(id, name, _, _)| json!({"id": id, "label": name, "icon": "play"}))
                        .collect())
                })
                .on_child_press(|ctx| match ctx.child_id.clone() {
                    Some(id) => do_action(&id, json!({})),
                    None => Ok(()),
                }),
        )
        .run()
}
