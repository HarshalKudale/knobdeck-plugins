//! VS Code plugin for KnobDeck — drives VS Code through the KnobDeck VS Code companion extension.
//!
//! Two routes, the first that works:
//! 1. KnobDeck's companion hub: every VS Code window links to KnobDeck and says when it gains
//!    focus; actions go to the window focused last. No port, no password.
//! 2. The extension's HTTP action server, for an extension older than 1.1 (or one with the hub
//!    link off), on `VSCODE_PORT` with an optional password.

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{companion, env_or, json, load_env, Action, Error, Plugin, Result, Value};
use std::sync::Mutex;
use std::time::Duration;

const NAME: &str = "VS Code companion";

fn host() -> String {
    env_or("VSCODE_HOST", "127.0.0.1")
}
fn default_port() -> String {
    env_or("VSCODE_PORT", "32145")
}

fn base_inputs() -> Vec<Value> {
    vec![input::text("port", "Action Port (Optional)").placeholder(&format!("Default {}", default_port()))]
}

/// Port override from the instance inputs, else the configured default.
fn resolve_port(inputs: &Value) -> String {
    match inputs["port"].as_str().map(str::trim) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => default_port(),
    }
}

/// encodeURIComponent
fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn get(path: &str, inputs: &Value, params: &[(&str, &str)]) -> Result<ureq::Response> {
    let port = resolve_port(inputs);
    let mut req = ureq::get(&format!("http://{}:{port}{path}", host())).timeout(Duration::from_secs(10));
    let password = knobdeck_sdk::secrets::env_or_stored("VSCODE_PASSWORD");
    if !password.is_empty() {
        req = req.set("Authorization", &password);
    }
    for (k, v) in params.iter().filter(|(_, v)| !v.is_empty()) {
        req = req.query(k, v);
    }
    match req.call() {
        Ok(r) => Ok(r),
        Err(ureq::Error::Status(401 | 403, _)) => {
            knobdeck_sdk::mark_setup_required(format!("{NAME} rejected the password"));
            Err(Error::msg(format!("{NAME} rejected the configured password.")))
        }
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_json::<Value>().ok().and_then(|b| b["error"].as_str().map(|e| format!(" {e}"))).unwrap_or_default();
            Err(Error::msg(format!("{NAME} returned HTTP {code}.{detail}")))
        }
        Err(e) => Err(Error::msg(format!(
            "No VS Code window is linked to KnobDeck. Install or update the KnobDeck VS Code companion extension (or start its action server on port {port}). ({e})"
        ))),
    }
}

// ── The companion hub ──────────────────────────────────────────────────────────

/// Linked VS Code windows (link ids, oldest first) and the one that had focus last.
static LINKS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static FOCUSED: Mutex<Option<String>> = Mutex::new(None);

/// How long a hub request may take; the extension answers `pending` after 4.5 s.
const HUB_TIMEOUT_MS: u64 = 8000;

/// The window to drive: the one focused last, else the newest link.
fn target_link() -> Option<String> {
    let links = LINKS.lock().unwrap();
    let focused = FOCUSED.lock().unwrap().clone().filter(|f| links.contains(f));
    focused.or_else(|| links.last().cloned())
}

/// Keep LINKS / FOCUSED in step with the hub's events.
fn on_companion(update: companion::Update) {
    let mut links = LINKS.lock().unwrap();
    match update {
        companion::Update::Connected(l) => {
            links.retain(|x| x != &l.link);
            links.push(l.link);
        }
        companion::Update::Disconnected(g) => {
            links.retain(|x| x != &g.link);
            let mut focused = FOCUSED.lock().unwrap();
            if focused.as_deref() == Some(&g.link) {
                *focused = None;
            }
        }
        companion::Update::Event(e) if e.event == "window_focused" => *FOCUSED.lock().unwrap() = Some(e.link),
        companion::Update::Event(_) => {}
    }
}

/// Run `action` in the target window through the hub. `None` when no window is linked (then the
/// HTTP route is tried).
fn hub_call(action: &str, params: Value) -> Option<Result<Value>> {
    let link = target_link()?;
    Some(match companion::request_with_timeout(Some(&link), action, params, Some(HUB_TIMEOUT_MS)) {
        Ok(answer) => companion::check(answer).map_err(|e| Error::msg(format!("VS Code: {}", e.message()))),
        Err(e) => Err(Error::msg(format!("VS Code didn't answer: {}", e.message()))),
    })
}

fn params_object(params: &[(&str, &str)]) -> Value {
    Value::Object(params.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.to_string(), json!(v))).collect())
}

fn execute_command(command_id: &str, inputs: &Value, params: &[(&str, &str)]) -> Result {
    if let Some(r) = hub_call(command_id, params_object(params)) {
        return r.map(|_| ());
    }
    get(&format!("/api/action/{}", enc(command_id)), inputs, params).map(|_| ())
}

/// `[{ value, label }]` from the companion's `query` (hub) or `/api/query/<name>` (HTTP).
fn fetch_options(name: &str, inputs: &Value) -> Result<Vec<Value>> {
    let data: Value = match hub_call("query", json!({ "name": name })) {
        Some(r) => r?,
        None => get(&format!("/api/query/{name}"), inputs, &[])?.into_json().unwrap_or(json!({})),
    };
    Ok(data["items"]
        .as_array()
        .map(|items| items.iter().map(|i| json!({"value": i["value"], "label": i["label"]})).collect())
        .unwrap_or_default())
}

fn require(ctx: &knobdeck_sdk::Ctx, key: &str, what: &str) -> Result<String> {
    match ctx.input_str(key).trim() {
        "" => ctx.fail(format!("{what} is required")),
        v => Ok(v.to_string()),
    }
}

/// A button that runs a companion command with one query-backed select input.
fn picker_button(id: &'static str, label: &str, icon: &str, key: &'static str, input_label: &str, query: &str, what: &'static str, command: &'static str, param: &'static str) -> Action {
    let mut inputs = vec![input::query_select(key, input_label, query, &[])];
    inputs.extend(base_inputs());
    Action::button(id, label).icon(icon).inputs(inputs).on_press(move |ctx| {
        let value = require(ctx, key, what)?;
        execute_command(command, &ctx.inputs, &[(param, &value)])
    })
}

/// A dynamic wheel listing companion options; pressing a child runs `command`.
fn option_wheel(id: &str, label: &str, icon: &'static str, query: &'static str, empty: &'static str, command: &'static str, param: &'static str) -> Action {
    Action::wheel(id, label)
        .icon(icon)
        .inputs(base_inputs())
        .on_open(move |ctx| {
            let items = fetch_options(query, &ctx.inputs)?;
            if items.is_empty() {
                return ctx.fail(empty);
            }
            Ok(items.into_iter().take(16).map(|i| json!({"id": i["value"], "label": i["label"], "icon": icon})).collect())
        })
        .on_child_press(move |ctx| {
            let child = ctx.child_id.clone().unwrap_or_default();
            execute_command(command, &ctx.inputs, &[(param, &child)])
        })
}

// (id, label, commandId, icon) — the category column of the JS table is not emitted to actions.json.
const SIMPLE: &[(&str, &str, &str, &str)] = &[
    ("command-palette", "Command Palette", "vscode.openCommandPalette", "compass"),
    ("quick-open", "Quick Open", "vscode.quickOpen", "search"),
    ("explorer", "Explorer", "vscode.explorer", "folder"),
    ("search", "Search", "vscode.search", "search"),
    ("source-control", "Source Control", "vscode.sourceControl", "git-branch"),
    ("extensions", "Extensions", "vscode.extensions", "puzzle"),
    ("terminal", "Toggle Terminal", "vscode.toggleTerminal", "terminal"),
    ("problems", "Problems", "vscode.problems", "triangle-alert"),
    ("save-all", "Save All", "vscode.saveAll", "save-all"),
    ("format-document", "Format Document", "vscode.formatDocument", "wand-sparkles"),
    ("toggle-sidebar", "Toggle Sidebar", "vscode.toggleSidebar", "panel-left"),
    ("toggle-panel", "Toggle Panel", "vscode.togglePanel", "blocks"),
    ("zen-mode", "Zen Mode", "vscode.zenMode", "scan"),
    ("split-editor", "Split Editor", "vscode.splitEditor", "columns-2"),
    ("new-terminal", "New Terminal", "vscode.newTerminal", "plus"),
    ("stop-debugging", "Stop Debugging", "vscode.stopDebugging", "square"),
];

fn main() {
    load_env();
    knobdeck_sdk::secrets::migrate_env(&["VSCODE_PASSWORD"]);
    let mut plugin = Plugin::new(&format!("vscode-{}", std::env::consts::OS), "VS Code")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Control Visual Studio Code through the KnobDeck VS Code companion extension.")
        .icon("vscode-logo.svg")
        .ack_timeout_ms(5000)
        .setup("vscode-setup", Some("setup.html"));

    for &(id, label, command, icon) in SIMPLE {
        plugin = plugin.action(
            Action::button(id, label).icon(icon).inputs(base_inputs()).on_press(move |ctx| execute_command(command, &ctx.inputs, &[])),
        );
    }

    plugin
        .action(picker_button("run-task", "Run Task", "play", "taskName", "Task", "vscode-get-tasks", "Task", "vscode.runTask", "name"))
        .action(picker_button("start-debugging", "Start Debugging", "bug", "configName", "Launch Configuration", "vscode-get-launch-configs", "Launch configuration", "vscode.startDebugging", "name"))
        .action(picker_button("focus-terminal", "Focus Terminal", "square-terminal", "terminalName", "Terminal", "vscode-get-terminals", "Terminal", "vscode.focusTerminal", "name"))
        .action(picker_button("open-editor", "Open Editor", "file", "editorUri", "Open Editor", "vscode-get-open-editors", "Editor", "vscode.openEditor", "uri"))
        .action({
            let mut inputs = vec![input::text("commandId", "Command ID").placeholder("workbench.action.tasks.runTask")];
            inputs.extend(base_inputs());
            Action::button("custom-command", "Custom Command").icon("sparkles").inputs(inputs).on_press(|ctx| {
                let command = require(ctx, "commandId", "Command ID")?;
                execute_command(&command, &ctx.inputs, &[])
            })
        })
        .action(option_wheel("task-wheel", "Task Wheel", "play", "tasks", "No VS Code tasks are available.", "vscode.runTask", "name"))
        .action(option_wheel("debug-wheel", "Debug Wheel", "bug", "launch-configs", "No VS Code launch configurations are available.", "vscode.startDebugging", "name"))
        .action(option_wheel("open-editor-wheel", "Open Editor Wheel", "file", "open-editors", "No open editors are available.", "vscode.openEditor", "uri"))
        .query("vscode-get-tasks", |ctx| Ok(Value::Array(fetch_options("tasks", &ctx.inputs)?)))
        .query("vscode-get-launch-configs", |ctx| Ok(Value::Array(fetch_options("launch-configs", &ctx.inputs)?)))
        .query("vscode-get-terminals", |ctx| Ok(Value::Array(fetch_options("terminals", &ctx.inputs)?)))
        .query("vscode-get-open-editors", |ctx| Ok(Value::Array(fetch_options("open-editors", &ctx.inputs)?)))
        // Windows link through KnobDeck's companion hub; this keeps track of them.
        .companions()
        .on_event(|ev, _| {
            if let Some(update) = companion::parse(ev) {
                on_companion(update);
            }
            Ok(())
        })
        .on_ready(|rt| {
            // Windows that linked before this process started (a plugin restart): ask the hub.
            if rt.has_feature(knobdeck_sdk::protocol::features::COMPANION_HUB) {
                if let Ok(links) = companion::links() {
                    *LINKS.lock().unwrap() = links.into_iter().map(|l| l.link).collect();
                }
            }
            Ok(())
        })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(id: &str) -> companion::Link {
        companion::Link { link: id.into(), instance: id.into(), app: "vscode".into(), app_version: None, name: None, version: None, features: vec![] }
    }

    #[test]
    fn the_last_focused_window_is_driven_and_closed_ones_are_forgotten() {
        LINKS.lock().unwrap().clear();
        *FOCUSED.lock().unwrap() = None;
        assert_eq!(target_link(), None, "no window: the HTTP route");
        on_companion(companion::Update::Connected(link("a")));
        on_companion(companion::Update::Connected(link("b")));
        assert_eq!(target_link().as_deref(), Some("b"), "nothing focused yet: the newest window");
        on_companion(companion::Update::Event(companion::LinkEvent { link: "a".into(), event: "window_focused".into(), payload: json!({}) }));
        assert_eq!(target_link().as_deref(), Some("a"));
        on_companion(companion::Update::Connected(link("c")));
        assert_eq!(target_link().as_deref(), Some("a"), "a new window doesn't steal focus");
        on_companion(companion::Update::Disconnected(companion::LinkGone { link: "a".into() }));
        assert_eq!(target_link().as_deref(), Some("c"), "the focused window closed: the newest one");
        on_companion(companion::Update::Event(companion::LinkEvent { link: "b".into(), event: "something_else".into(), payload: json!({}) }));
        assert_eq!(target_link().as_deref(), Some("c"));
    }

    #[test]
    fn params_drop_empty_values() {
        assert_eq!(params_object(&[("name", "build"), ("uri", "")]), json!({"name": "build"}));
    }
}
