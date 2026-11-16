//! Shared Chrome / Edge / Brave / Firefox / Zen browser controls for KnobDeck. The actions run
//! through the KnobDeck browser extension (Knobdeck/knobdeck-browser-companion) that each browser connects
//! to this plugin: tabs, windows, zoom, bookmarks and history are the browser's own APIs, not
//! shortcuts sent to whichever window has focus.

mod browsers;
mod ext;

use browsers::{ensure_executable, family_of, label, Family, KEYS};
use ext::CallError;
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, load_env, os, send_keys, Action, Error, Plugin, Result, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

static WHEEL_ITEMS: Mutex<Option<HashMap<String, (String, String)>>> = Mutex::new(None); // childId → (browser id, url)
static REPEATS: Mutex<Option<HashMap<String, Arc<AtomicBool>>>> = Mutex::new(None);

fn browser_input() -> Value {
    input::query_combobox("browser", "Browser", "connected_browser")
}

/// The id of the selected connected browser, or "".
fn browser_id(inputs: &Value) -> String {
    match &inputs["browser"] {
        Value::Object(o) => o.get("value").and_then(Value::as_str).unwrap_or("").to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// The selected browser, which must be connected right now.
fn target(inputs: &Value) -> Result<ext::Info> {
    let id = browser_id(inputs);
    if id.is_empty() {
        return Err(Error::msg("Select a connected browser first"));
    }
    ext::info(&id).ok_or_else(|| Error::msg("That browser is not connected. Open it and check the KnobDeck extension is installed and enabled."))
}

fn call(inputs: &Value, action: &str, params: Value) -> Result<Value> {
    ext::call(&target(inputs)?.id, action, params).map_err(|e| match e {
        CallError::Failed(m) => Error::msg(m),
        CallError::Unsupported => Error::msg("This browser's extension can't do that action"),
    })
}

fn activate_browser(key: &str) -> Result {
    let exe = ensure_executable(key)?;
    if os::window::focus_app(&exe, None)? {
        return Ok(());
    }
    // Linux: the exe name often differs from the window class / app id (`google-chrome-stable`
    // vs `google-chrome`, Flatpak ids), so try the browser's known classes too.
    #[cfg(target_os = "linux")]
    for class in browsers::window_classes(key) {
        if os::window::focus_app(class, None)? {
            return Ok(());
        }
    }
    Err(Error::msg(format!("{} window is not open", label(key))))
}

/// Run `action` in the browser through its extension. Only when the browser has no extension API
/// for it (`fallback` is the shortcut) is the browser brought to the front and the shortcut sent:
/// the last resort of CLAUDE.md rule 1.
fn browser_action(inputs: &Value, action: &str, fallback: Option<&str>) -> Result {
    let browser = target(inputs)?;
    match ext::call(&browser.id, action, json!({})) {
        Ok(_) => Ok(()),
        Err(CallError::Failed(m)) => Err(Error::msg(m)),
        Err(CallError::Unsupported) => match fallback {
            Some(keys) if KEYS.contains(&browser.key.as_str()) => {
                activate_browser(&browser.key)?;
                send_keys(keys)
            }
            _ => Err(Error::msg(format!("{} can't do that from the extension", browser.label))),
        },
    }
}

fn start_repeat(instance_id: &str, f: impl Fn() -> Result + Send + 'static) {
    stop_repeat(instance_id);
    let stop = Arc::new(AtomicBool::new(false));
    REPEATS.lock().unwrap().get_or_insert_with(HashMap::new).insert(instance_id.into(), stop.clone());
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(220));
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let _ = f();
    });
}

fn stop_repeat(instance_id: &str) {
    if let Some(stop) = REPEATS.lock().unwrap().as_mut().and_then(|m| m.remove(instance_id)) {
        stop.store(true, Ordering::SeqCst);
    }
}

/// Shortcut for the browsers whose extension API can't do an action (`None`: it can).
type Fallback = fn(Family) -> Option<&'static str>;

/// A button that runs one action in the chosen browser.
fn basic_button(id: &'static str, label: &str, icon: &str, fallback: Fallback) -> Action {
    Action::button(id, label).icon(icon).inputs(vec![browser_input()]).on_press(move |ctx| {
        let family = family_of(&target(&ctx.inputs)?.key);
        browser_action(&ctx.inputs, id, fallback(family))
    })
}

/// Hold-to-repeat action: fires `step` immediately, then every 220 ms while held.
fn hold_repeat(id: &str, label: &str, icon: &str, released: (&str, &str), held: (&str, &str), step: fn(&Value) -> Result) -> Action {
    Action::hold(id, label)
        .icon(icon)
        .inputs(vec![browser_input()])
        .released(released.0, released.1)
        .held(held.0, held.1)
        .on_hold_start(move |ctx| {
            let result: Result = (|| {
                target(&ctx.inputs)?;
                step(&ctx.inputs)?;
                let inputs = ctx.inputs.clone();
                start_repeat(&ctx.instance_id, move || step(&inputs));
                Ok(())
            })();
            if result.is_err() {
                stop_repeat(&ctx.instance_id);
            }
            result?;
            ctx.set_state("held");
            Ok(())
        })
        .on_hold_end(|ctx| {
            stop_repeat(&ctx.instance_id);
            ctx.set_state("released");
            Ok(())
        })
}

/// Page zoom changes by this many percentage points per hold tick.
const ZOOM_TICK: i64 = 10;

/// The page zoom (percent) in the answer to `zoom-set` / `zoom-by`.
fn zoom_percent(answer: &Value) -> Option<i64> {
    answer["percent"].as_f64().map(|p| p.round() as i64)
}

/// A wheel listing recent bookmarks / history; pressing a child opens it.
fn link_wheel(id: &str, label: &str, icon: &str, action: &'static str, child_prefix: &'static str, child_icon: &'static str, empty: &'static str) -> Action {
    Action::wheel(id, label)
        .icon(icon)
        .inputs(vec![browser_input()])
        .on_open(move |ctx| {
            let result: Result<Vec<Value>> = (|| {
                let browser = target(&ctx.inputs)?;
                let items = ext::items(&call(&ctx.inputs, action, json!({}))?);
                if items.is_empty() {
                    return Err(Error::msg(empty.replace("{}", &browser.label)));
                }
                let mut map = WHEEL_ITEMS.lock().unwrap();
                let map = map.get_or_insert_with(HashMap::new);
                map.retain(|k, _| !k.starts_with(child_prefix));
                Ok(items
                    .into_iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let child = format!("{child_prefix}-{i}");
                        map.insert(child.clone(), (browser.id.clone(), item.url.clone()));
                        json!({"id": child, "label": item.title.chars().take(80).collect::<String>(), "icon": child_icon})
                    })
                    .collect::<Vec<_>>())
            })();
            result.or_else(|e: Error| ctx.fail(e.message().to_string()))
        })
        .on_child_press(|ctx| {
            let child = ctx.child_id.clone().unwrap_or_default();
            let item = WHEEL_ITEMS.lock().unwrap().as_ref().and_then(|m| m.get(&child).cloned());
            let (browser, url) = item.ok_or_else(|| Error::msg("That item is no longer available"))?;
            ext::call(&browser, "open-url", json!({"url": url})).map(|_| ()).map_err(|e| match e {
                CallError::Failed(m) => Error::msg(m),
                CallError::Unsupported => Error::msg("This browser's extension can't open links"),
            })
        })
}

/// Actions kept in the code but offered on no OS (CLAUDE.md rule 1): keybind-based. An empty
/// `platforms` means "every OS", so this names none.
const DISABLED: &[&str] = &["none"];

fn off(a: Action) -> Action {
    a.platforms(DISABLED)
}

fn main() {
    load_env();
    ext::serve();
    let mut plugin = Plugin::new(&format!("browser-plugin-{}", std::env::consts::OS), "BrowserPlugin")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Shared Chrome, Edge, Brave, Firefox and Zen controls, wheels, and zoom actions on Windows, macOS and Linux, through the KnobDeck browser extension.")
        .icon("globe")
        .ack_timeout_ms(5000);

    // Every action runs in the extension. The address bar, find and save have no extension API in
    // any browser; they only had a shortcut, so they are disabled (CLAUDE.md rule 1: a keybind
    // belongs in KnobDeck's Keybind action). Firefox/Zen refuse extension tabs on their internal
    // pages (downloads, history, bookmarks manager): those actions report it there.
    let none: Fallback = |_| None;
    let address_bar: Fallback = |_| Some("mod l");
    let find: Fallback = |_| Some("mod f");
    let save: Fallback = |_| Some("mod s");
    let (downloads, history, bookmarks_manager) = (none, none, none);
    let basics: Vec<(&'static str, &str, &str, Fallback)> = vec![
        ("new-tab", "New Tab", "notebook-tabs", none),
        ("new-window", "New Window", "app-window", none),
        ("new-private-window", "New Private / Incognito Window", "app-window", none),
        ("close-tab", "Close Tab", "notebook-tabs", none),
        ("reopen-closed-tab", "Reopen Closed Tab", "notebook-tabs", none),
        ("next-tab", "Next Tab", "notebook-tabs", none),
        ("previous-tab", "Previous Tab", "notebook-tabs", none),
        ("reload-tab", "Reload Tab", "refresh-cw", none),
        ("hard-reload-tab", "Hard Reload", "refresh-cw", none),
        ("go-back", "Back", "arrow-left", none),
        ("go-forward", "Forward", "arrow-right", none),
        ("focus-address-bar", "Focus Address Bar", "search", address_bar),
        ("open-downloads", "Open Downloads", "download", downloads),
        ("open-history", "Open History", "history", history),
        ("open-bookmarks-manager", "Open Bookmarks Manager", "bookmark", bookmarks_manager),
        ("bookmark-page", "Bookmark Page", "bookmark", none),
        ("find-in-page", "Find in Page", "search", find),
        ("print-page", "Print Page", "printer", none),
        ("save-page", "Save Page", "save", save),
        ("toggle-fullscreen", "Toggle Fullscreen", "maximize", none),
        ("stop-loading", "Stop Loading", "circle-stop", none),
    ];
    for (id, label, icon, fallback) in basics {
        let action = basic_button(id, label, icon, fallback);
        plugin = plugin.action(if ["focus-address-bar", "find-in-page", "save-page"].contains(&id) { off(action) } else { action });
    }

    plugin
        .action(off(
            Action::button("custom-keybind", "Custom Keybind")
                .icon("keyboard")
                .inputs(vec![browser_input(), input::keybind("keys", "Keys").placeholder("k")])
                .on_press(|ctx| {
                    let keys = ctx.input_str("keys").trim().to_string();
                    if keys.is_empty() {
                        return Err(Error::msg("Keys are required"));
                    }
                    call(&ctx.inputs, "custom-keybind", json!({"keys": keys})).map(|_| ())
                }),
        ))
        .action(
            Action::button("open-url", "Open URL")
                .icon("link")
                .inputs(vec![browser_input(), input::text("url", "URL").placeholder("https://example.com")])
                .on_press(|ctx| {
                    let url = ctx.input_str("url").trim().to_string();
                    if url.is_empty() {
                        return Err(Error::msg("URL is required"));
                    }
                    call(&ctx.inputs, "open-url", json!({"url": url})).map(|_| ())
                }),
        )
        .action(hold_repeat("hold-next-tab", "Hold Next Tab", "notebook-tabs", ("Idle", "chevron-right"), ("Cycling", "chevrons-right"), |i| call(i, "next-tab", json!({})).map(|_| ())))
        .action(hold_repeat("hold-previous-tab", "Hold Previous Tab", "notebook-tabs", ("Idle", "chevron-left"), ("Cycling", "chevrons-left"), |i| call(i, "previous-tab", json!({})).map(|_| ())))
        .action(hold_repeat("hold-zoom-in", "Hold Zoom In", "zoom-in", ("Idle", "zoom-in"), ("Zooming", "zoom-in"), |i| call(i, "zoom-by", json!({"percent": ZOOM_TICK})).map(|_| ())))
        .action(hold_repeat("hold-zoom-out", "Hold Zoom Out", "zoom-in", ("Idle", "zoom-out"), ("Zooming", "zoom-out"), |i| call(i, "zoom-by", json!({"percent": -ZOOM_TICK})).map(|_| ())))
        .action(
            Action::slider("page-zoom", "Page Zoom", 25.0, 300.0)
                .icon("zoom-in")
                .step(5.0)
                .unit("%")
                .inputs(vec![browser_input()])
                .initial(100)
                .on_adjust(|ctx| {
                    let answer = if let Some(v) = ctx.value {
                        let target = (((v / 5.0).round() as i64) * 5).clamp(25, 300);
                        call(&ctx.inputs, "zoom-set", json!({"percent": target}))?
                    } else {
                        let Some(delta) = ctx.delta else { return Ok(()) };
                        let step = ctx.input_str("step").trim().parse::<i64>().ok().filter(|s| *s != 0).map_or(5, |s| s.abs());
                        call(&ctx.inputs, "zoom-by", json!({"percent": delta as i64 * step}))?
                    };
                    if let Some(percent) = zoom_percent(&answer) {
                        ctx.set_state(percent.clamp(25, 300));
                    }
                    Ok(())
                }),
        )
        .action(link_wheel("bookmarks-wheel", "Bookmarks", "bookmark", "bookmarks", "bookmark", "bookmark", "No {} bookmarks were found"))
        .action(link_wheel("history-wheel", "Recent History", "history", "history", "history", "history", "No recent {} history items were found"))
        .query("connected_browser", |_ctx| {
            let browsers = ext::connected();
            let mut seen: HashMap<String, usize> = HashMap::new();
            Ok(Value::Array(
                browsers
                    .iter()
                    .map(|b| {
                        // Two windows of the same browser (profiles) get told apart by a number.
                        let same = browsers.iter().filter(|o| o.label == b.label).count();
                        let n = seen.entry(b.label.clone()).or_insert(0);
                        *n += 1;
                        let label = if same > 1 { format!("{} ({})", b.label, n) } else { b.label.clone() };
                        let family = if family_of(&b.key) == Family::Firefox { "firefox" } else { "chromium" };
                        json!({"label": label, "value": b.id, "keywords": [b.label, b.key, family]})
                    })
                    .collect(),
            ))
        })
        .run()
}

