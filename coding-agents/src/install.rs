//! Connecting the agents (run by setup): copy this plugin's binary to a fixed place in the data
//! folder (`bin/knobdeck-agent-hook`, so plugin updates don't move it), then add KnobDeck's hooks
//! to Claude Code's `settings.json` and KnobDeck's `notify` program to Codex's `config.toml`.
//! Both edits are idempotent and remove only KnobDeck's own entries when undone.

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Marks KnobDeck's entries in the agents' configs.
pub const HOOK_NAME: &str = "knobdeck-agent-hook";

pub fn hook_path(data: &Path) -> PathBuf {
    data.join("bin").join(if cfg!(windows) { format!("{HOOK_NAME}.exe") } else { HOOK_NAME.to_string() })
}

/// The plugin binary next to this program (setup and plugin ship side by side).
pub fn plugin_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let name = if cfg!(windows) { "coding-agents.exe" } else { "coding-agents" };
    let p = exe.with_file_name(name);
    if p.is_file() {
        Ok(p)
    } else {
        Err(format!("{name} isn't next to the setup program"))
    }
}

/// Copy `from` to the hook path when it differs (a plugin update). A copy in use on Windows can't
/// be overwritten but can be renamed, so it is moved aside first.
pub fn refresh_hook_binary(from: &Path, data: &Path) -> std::io::Result<bool> {
    let to = hook_path(data);
    let same = match (fs::read(from), fs::read(&to)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Ok(false);
    }
    fs::create_dir_all(to.parent().unwrap())?;
    if to.exists() {
        let old = to.with_extension("old");
        let _ = fs::remove_file(&old);
        let _ = fs::rename(&to, &old);
    }
    fs::copy(from, &to)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&to, fs::Permissions::from_mode(0o755))?;
    }
    Ok(true)
}

/// The command line an agent runs. Forward slashes on Windows too: Claude Code runs hooks
/// through a shell that would treat backslashes as escapes.
pub fn command(data: &Path, agent: &str) -> String {
    let p = hook_path(data).to_string_lossy().replace('\\', "/");
    format!("\"{p}\" hook {agent}")
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

pub fn claude_settings_path() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR") {
        Some(d) => Some(PathBuf::from(d).join("settings.json")),
        None => Some(home()?.join(".claude").join("settings.json")),
    }
}

pub fn codex_config_path() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME") {
        Some(d) => Some(PathBuf::from(d).join("config.toml")),
        None => Some(home()?.join(".codex").join("config.toml")),
    }
}

/// The Claude Code hooks KnobDeck adds: (event, seconds before Claude Code gives up, async).
fn claude_hooks(approval_wait: u64) -> Vec<(&'static str, u64, bool)> {
    vec![
        ("SessionStart", 10, false),
        ("UserPromptSubmit", 10, false),
        // Only to notice the session is working again; never in the tool's way.
        ("PostToolUse", 10, true),
        ("Notification", 10, false),
        ("PermissionRequest", approval_wait + 15, false),
        ("Stop", 10, false),
        ("SessionEnd", 10, false),
    ]
}

fn is_ours(group: &Value) -> bool {
    group["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| h["command"].as_str().is_some_and(|c| c.contains(HOOK_NAME))))
}

/// `settings` with KnobDeck's hook groups removed, and added again when `install`.
pub fn edit_claude_settings(mut settings: Value, cmd: &str, approval_wait: u64, install: bool) -> Result<Value, String> {
    if !settings.is_object() {
        return Err("Claude Code's settings.json isn't a JSON object".into());
    }
    let hooks = settings.as_object_mut().unwrap().entry("hooks").or_insert_with(|| json!({}));
    let Some(hooks) = hooks.as_object_mut() else {
        return Err("\"hooks\" in Claude Code's settings.json isn't an object".into());
    };
    for groups in hooks.values_mut() {
        if let Some(arr) = groups.as_array_mut() {
            arr.retain(|g| !is_ours(g));
        }
    }
    if install {
        for (event, timeout, is_async) in claude_hooks(approval_wait) {
            let mut h = json!({"type": "command", "command": cmd, "timeout": timeout});
            if is_async {
                h["async"] = json!(true);
            }
            let groups = hooks.entry(event).or_insert_with(|| json!([]));
            match groups.as_array_mut() {
                Some(arr) => arr.push(json!({"hooks": [h]})),
                None => return Err(format!("\"hooks.{event}\" in Claude Code's settings.json isn't a list")),
            }
        }
    }
    hooks.retain(|_, g| g.as_array().is_none_or(|a| !a.is_empty()));
    if hooks.is_empty() {
        settings.as_object_mut().unwrap().remove("hooks");
    }
    Ok(settings)
}

/// Back up a config once before KnobDeck first changes it.
fn backup_once(path: &Path) {
    let bak = path.with_extension(format!("{}.knobdeck-backup", path.extension().and_then(|e| e.to_str()).unwrap_or("bak")));
    if path.is_file() && !bak.exists() {
        let _ = fs::copy(path, bak);
    }
}

pub fn setup_claude(data: &Path, approval_wait: u64, install: bool) -> Result<bool, String> {
    let path = claude_settings_path().ok_or("Couldn't find your home folder")?;
    let current = match fs::read_to_string(&path) {
        Ok(t) if t.trim().is_empty() => Value::Object(Map::new()),
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{} isn't valid JSON ({e}); fix it and set up again", path.display()))?,
        Err(_) if !install => return Ok(false),
        Err(_) => Value::Object(Map::new()),
    };
    let next = edit_claude_settings(current.clone(), &command(data, "claude"), approval_wait, install)?;
    if next == current {
        return Ok(false);
    }
    backup_once(&path);
    let text = serde_json::to_string_pretty(&next).map_err(|e| e.to_string())? + "\n";
    crate::store::write_atomic(&path, text.as_bytes()).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    Ok(true)
}

/// `config.toml` with KnobDeck's `notify` set (or removed when not `install`). Another program
/// already set as `notify` is left alone and reported: Codex runs only one.
pub fn edit_codex_config(text: &str, cmd_path: &str, install: bool) -> Result<String, String> {
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| format!("Codex's config.toml can't be read ({e})"))?;
    let ours = |v: &toml_edit::Item| v.as_array().and_then(|a| a.get(0)).and_then(|v| v.as_str()).is_some_and(|p| p.contains(HOOK_NAME));
    match doc.get("notify") {
        Some(item) if !ours(item) => {
            if install {
                return Err("Codex already runs another program when a turn ends (\"notify\" in ~/.codex/config.toml). Remove it, or untick Codex".into());
            }
            return Ok(text.to_string());
        }
        _ => {}
    }
    if install {
        let mut arr = toml_edit::Array::new();
        arr.push(cmd_path);
        arr.push("hook");
        arr.push("codex");
        // A top-level key: insert it before any [table].
        doc.insert("notify", toml_edit::value(arr));
    } else {
        doc.remove("notify");
    }
    Ok(doc.to_string())
}

pub fn setup_codex(data: &Path, install: bool) -> Result<bool, String> {
    let path = codex_config_path().ok_or("Couldn't find your home folder")?;
    let current = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) if !install => return Ok(false),
        Err(_) if path.parent().is_some_and(|d| !d.exists()) => return Err("Codex isn't set up on this computer (no ~/.codex folder). Run codex once, or untick Codex".into()),
        Err(_) => String::new(),
    };
    let next = edit_codex_config(&current, &hook_path(data).to_string_lossy(), install)?;
    if next == current {
        return Ok(false);
    }
    backup_once(&path);
    crate::store::write_atomic(&path, next.as_bytes()).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_settings_round_trip() {
        let user = json!({"model": "opus", "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}});
        let cmd = "\"/d/bin/knobdeck-agent-hook\" hook claude";
        let installed = edit_claude_settings(user.clone(), cmd, 30, true).unwrap();
        assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(installed["hooks"]["PermissionRequest"][0]["hooks"][0]["timeout"], 45);
        assert_eq!(installed["hooks"]["PostToolUse"][0]["hooks"][0]["async"], true);
        // Installing twice changes nothing.
        assert_eq!(edit_claude_settings(installed.clone(), cmd, 30, true).unwrap(), installed);
        // Removing leaves the user's own hooks and settings exactly as they were.
        assert_eq!(edit_claude_settings(installed, cmd, 30, false).unwrap(), user);
        assert_eq!(edit_claude_settings(json!({}), cmd, 30, false).unwrap(), json!({}));
        assert!(edit_claude_settings(json!([]), cmd, 30, true).is_err());
    }

    #[test]
    fn codex_config_round_trip() {
        let user = "model = \"gpt-5\"\n\n[profiles.fast]\nmodel = \"o4\"\n";
        let added = edit_codex_config(user, "/d/bin/knobdeck-agent-hook", true).unwrap();
        let doc: toml_edit::DocumentMut = added.parse().unwrap();
        assert_eq!(doc["notify"].as_array().unwrap().len(), 3);
        // Still a top-level key, not inside [profiles.fast].
        assert!(doc["profiles"]["fast"].get("notify").is_none());
        assert_eq!(edit_codex_config(&added, "/d/bin/knobdeck-agent-hook", true).unwrap(), added);
        assert_eq!(edit_codex_config(&added, "/d/bin/knobdeck-agent-hook", false).unwrap().trim(), user.trim());
        let other = "notify = [\"my-notifier\"]\n";
        assert!(edit_codex_config(other, "/d/bin/knobdeck-agent-hook", true).is_err());
        assert_eq!(edit_codex_config(other, "/d/bin/knobdeck-agent-hook", false).unwrap(), other);
    }
}
