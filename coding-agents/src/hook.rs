//! Hook mode: `knobdeck-agent-hook hook claude` (Claude Code hooks, the event as JSON on stdin)
//! and `knobdeck-agent-hook hook codex '<json>'` (Codex's `notify` program, the event as the
//! last argument). It records the session's state for the plugin and, for a Claude Code
//! permission request while a KnobDeck key can answer it, waits for that answer.
//!
//! A hook must never get in the agent's way: every failure exits 0 with no output, which
//! Claude Code and Codex treat as "no opinion".

use crate::store::{self, Pending, Status};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Where the plugin keeps its data: the hook binary is `<data>/bin/knobdeck-agent-hook`.
fn data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("KNOBDECK_PLUGIN_DATA") {
        return Some(PathBuf::from(d));
    }
    let exe = std::env::current_exe().ok()?;
    let bin = exe.parent()?;
    (bin.file_name()? == "bin").then(|| bin.parent().map(Path::to_path_buf)).flatten()
}

pub fn run(args: &[String]) -> ! {
    if let Some(data) = data_dir() {
        let agent = args.first().map(String::as_str).unwrap_or("claude");
        let event = match agent {
            "codex" => args.get(1).and_then(|a| serde_json::from_str::<Value>(a).ok()),
            _ => {
                let mut raw = String::new();
                let _ = std::io::stdin().take(4 * 1024 * 1024).read_to_string(&mut raw);
                serde_json::from_str::<Value>(&raw).ok()
            }
        };
        if let Some(event) = event {
            let out = match agent {
                "codex" => {
                    codex(&data, &event);
                    None
                }
                _ => claude(&data, &event),
            };
            if let Some(out) = out {
                println!("{out}");
            }
        }
    }
    std::process::exit(0)
}

fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}

/// Codex `notify`: `{"type": "agent-turn-complete", "thread-id": …, "cwd": …, …}`.
fn codex(data: &Path, ev: &Value) {
    if s(ev, "type") != "agent-turn-complete" {
        return;
    }
    let id = s(ev, "thread-id");
    if id.is_empty() {
        return;
    }
    let _ = store::update_session(data, "codex", id, |sess| {
        if !s(ev, "cwd").is_empty() {
            sess.cwd = s(ev, "cwd").to_string();
        }
        sess.set_status(Status::Done);
    });
}

/// A Claude Code hook event; returns what to print (only a permission decision).
fn claude(data: &Path, ev: &Value) -> Option<String> {
    let id = s(ev, "session_id");
    if id.is_empty() {
        return None;
    }
    let event = s(ev, "hook_event_name");
    let with = |status: Option<Status>| {
        let _ = store::update_session(data, "claude", id, |sess| {
            if !s(ev, "cwd").is_empty() {
                sess.cwd = s(ev, "cwd").to_string();
            }
            if let Some(t) = ev["session_title"].as_str().filter(|t| !t.is_empty()) {
                sess.title = Some(t.to_string());
            }
            if let Some(st) = status {
                sess.set_status(st);
            }
        });
    };
    match event {
        "SessionStart" => with(Some(Status::Idle)),
        "UserPromptSubmit" => with(Some(Status::Working)),
        // A tool ran: back to work after an approval or a question.
        "PostToolUse" | "PostToolUseFailure" => {
            if store::read_session(data, "claude", id).is_none_or(|sess| sess.status != Status::Working) {
                with(Some(Status::Working));
            }
        }
        "Notification" => {
            let kind = s(ev, "notification_type");
            let msg = s(ev, "message").to_lowercase();
            let status = match kind {
                "permission_prompt" => Some(Status::Approval),
                "idle_prompt" | "elicitation_dialog" | "agent_needs_input" => Some(Status::Input),
                "" if msg.contains("permission") => Some(Status::Approval),
                "" if msg.contains("waiting for your input") => Some(Status::Input),
                _ => None,
            };
            if status.is_some() {
                with(status);
            }
        }
        "Stop" => with(Some(Status::Done)),
        "SessionEnd" => store::remove_session(data, "claude", id),
        "PermissionRequest" => return permission(data, id, ev, &with),
        _ => {}
    }
    None
}

/// What a request is about, for a key: the command, the file, the address…
fn detail(tool_input: &Value) -> String {
    for key in ["command", "file_path", "notebook_path", "url", "pattern", "query", "prompt", "description"] {
        if let Some(v) = tool_input[key].as_str().filter(|v| !v.is_empty()) {
            let v = if key.ends_with("path") { v.rsplit(['/', '\\']).next().unwrap_or(v) } else { v };
            return v.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect();
        }
    }
    String::new()
}

fn wait_secs(data: &Path) -> u64 {
    std::env::set_var("KNOBDECK_PLUGIN_DATA", data);
    knobdeck_sdk::load_env();
    knobdeck_sdk::env_or("APPROVAL_WAIT", "30").trim().parse().unwrap_or(30).min(300)
}

fn permission(data: &Path, session: &str, ev: &Value, with: &dyn Fn(Option<Status>)) -> Option<String> {
    with(Some(Status::Approval));
    let wait = wait_secs(data);
    // Wait only while the plugin runs and a key can answer; otherwise Claude Code asks at once.
    let can_answer = || store::fresh(&store::alive_path(data), Duration::from_secs(6)) && store::fresh(&store::approvals_flag(data), Duration::from_secs(6));
    if wait == 0 || !can_answer() {
        return None;
    }
    let req = Pending {
        id: format!("{}-{}", store::now_ms(), std::process::id()),
        agent: "claude".into(),
        session: session.to_string(),
        tool: s(ev, "tool_name").trim_start_matches("mcp__").to_string(),
        detail: detail(&ev["tool_input"]),
        at: store::now_ms(),
    };
    if store::write_pending(data, &req).is_err() {
        return None;
    }
    let deadline = Instant::now() + Duration::from_secs(wait);
    let answer = loop {
        if let Some(a) = store::read_answer(data, &req.id) {
            break Some(a);
        }
        if Instant::now() >= deadline || !can_answer() {
            break None;
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    store::clear_request(data, &req.id);
    let allow = answer?;
    with(Some(Status::Working));
    let decision = if allow { json!({"behavior": "allow"}) } else { json!({"behavior": "deny", "message": "The user denied this from KnobDeck."}) };
    Some(json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}}).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details() {
        assert_eq!(detail(&json!({"command": "npm  test\n--watch"})), "npm test --watch");
        assert_eq!(detail(&json!({"file_path": "/a/b/main.rs"})), "main.rs");
        assert_eq!(detail(&json!({})), "");
    }

    #[test]
    fn claude_events() {
        let dir = std::env::temp_dir().join(format!("kd-hook-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ev = |name: &str, extra: Value| {
            let mut v = json!({"session_id": "s1", "cwd": "/w/proj", "hook_event_name": name});
            v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            v
        };
        claude(&dir, &ev("UserPromptSubmit", json!({"prompt": "hi"})));
        assert_eq!(store::read_session(&dir, "claude", "s1").unwrap().status, Status::Working);
        claude(&dir, &ev("Notification", json!({"notification_type": "permission_prompt"})));
        assert_eq!(store::read_session(&dir, "claude", "s1").unwrap().status, Status::Approval);
        claude(&dir, &ev("PostToolUse", json!({})));
        assert_eq!(store::read_session(&dir, "claude", "s1").unwrap().status, Status::Working);
        claude(&dir, &ev("Stop", json!({})));
        assert_eq!(store::read_session(&dir, "claude", "s1").unwrap().status, Status::Done);
        // No plugin running: a permission request gets no decision (Claude Code asks as usual).
        assert_eq!(claude(&dir, &ev("PermissionRequest", json!({"tool_name": "Bash", "tool_input": {"command": "ls"}}))), None);
        claude(&dir, &ev("SessionEnd", json!({})));
        assert!(store::read_session(&dir, "claude", "s1").is_none());
        codex(&dir, &json!({"type": "agent-turn-complete", "thread-id": "t1", "cwd": "/w/x"}));
        assert_eq!(store::read_session(&dir, "codex", "t1").unwrap().status, Status::Done);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
