//! What the hooks and the plugin share, as small JSON files in the plugin's data folder:
//!
//! - `sessions/<agent>-<id>.json`: one coding-agent session and what it is doing;
//! - `approvals/<request>.json`: a permission request waiting for an answer from a key, and
//!   `approvals/<request>.answer` (`allow` / `deny`) once one is pressed;
//! - `plugin.alive`: touched by the running plugin; `approvals.on`: present while a key can
//!   answer requests (the hook only waits for an answer when both are fresh).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Running a turn.
    Working,
    /// Waiting for a tool permission.
    Approval,
    /// Asked the user something (or has waited at the prompt for a while).
    Input,
    /// Finished its turn: the user's move.
    Done,
    /// Nothing going on.
    #[default]
    Idle,
}

impl Status {
    pub fn needs_you(self) -> bool {
        matches!(self, Status::Approval | Status::Input | Status::Done)
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::Working => "Working",
            Status::Approval => "Needs approval",
            Status::Input => "Needs input",
            Status::Done => "Your turn",
            Status::Idle => "Idle",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Status::Working => "loader-circle",
            Status::Approval => "shield-alert",
            Status::Input => "message-circle-question",
            Status::Done => "circle-check",
            Status::Idle => "circle-pause",
        }
    }
    /// Sort order: what needs you first.
    pub fn rank(self) -> u8 {
        match self {
            Status::Approval => 0,
            Status::Input => 1,
            Status::Done => 2,
            Status::Working => 3,
            Status::Idle => 4,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Session {
    pub agent: String,
    pub id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub status: Status,
    /// When anything last changed (ms since the epoch).
    #[serde(default)]
    pub updated: u64,
    /// When the status last changed.
    #[serde(default)]
    pub since: u64,
}

impl Session {
    /// The project folder's name, or the session's own title.
    pub fn name(&self) -> String {
        if let Some(t) = self.title.as_deref().filter(|t| !t.trim().is_empty()) {
            return t.trim().to_string();
        }
        let base = self.cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or_default().to_string();
        if base.is_empty() {
            agent_name(&self.agent).to_string()
        } else {
            base
        }
    }
    pub fn set_status(&mut self, s: Status) {
        if self.status != s {
            self.status = s;
            self.since = now_ms();
        }
        self.updated = now_ms();
    }
}

pub fn agent_name(agent: &str) -> &str {
    match agent {
        "claude" => "Claude Code",
        "codex" => "Codex",
        other => other,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pending {
    pub id: String,
    pub agent: String,
    pub session: String,
    pub tool: String,
    #[serde(default)]
    pub detail: String,
    pub at: u64,
}

impl Pending {
    /// "Bash: npm test" for a key.
    pub fn summary(&self) -> String {
        if self.detail.is_empty() {
            self.tool.clone()
        } else {
            format!("{}: {}", self.tool, self.detail)
        }
    }
}

pub fn sessions_dir(data: &Path) -> PathBuf {
    data.join("sessions")
}

pub fn approvals_dir(data: &Path) -> PathBuf {
    data.join("approvals")
}

/// A file name made only of safe characters (session ids come from the agents).
pub fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(80).collect()
}

fn session_path(data: &Path, agent: &str, id: &str) -> PathBuf {
    sessions_dir(data).join(format!("{}-{}.json", safe(agent), safe(id)))
}

/// Write through a temporary file and a rename, so a reader never sees half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

pub fn read_session(data: &Path, agent: &str, id: &str) -> Option<Session> {
    serde_json::from_slice(&fs::read(session_path(data, agent, id)).ok()?).ok()
}

pub fn write_session(data: &Path, s: &Session) -> std::io::Result<()> {
    write_atomic(&session_path(data, &s.agent, &s.id), &serde_json::to_vec(s).unwrap_or_default())
}

/// Read, change and write back one session (created when new).
pub fn update_session(data: &Path, agent: &str, id: &str, f: impl FnOnce(&mut Session)) -> std::io::Result<Session> {
    let mut s = read_session(data, agent, id).unwrap_or_else(|| Session { agent: agent.into(), id: id.into(), since: now_ms(), ..Default::default() });
    f(&mut s);
    s.updated = now_ms();
    write_session(data, &s)?;
    Ok(s)
}

pub fn remove_session(data: &Path, agent: &str, id: &str) {
    let _ = fs::remove_file(session_path(data, agent, id));
}

fn json_files(dir: &Path) -> impl Iterator<Item = PathBuf> {
    fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json"))
}

pub fn load_sessions(data: &Path) -> Vec<Session> {
    json_files(&sessions_dir(data)).filter_map(|p| serde_json::from_slice(&fs::read(p).ok()?).ok()).collect()
}

/// Waiting permission requests, oldest first (answered ones excluded).
pub fn pending(data: &Path) -> Vec<Pending> {
    let mut v: Vec<Pending> = json_files(&approvals_dir(data))
        .filter(|p| !p.with_extension("answer").exists())
        .filter_map(|p| serde_json::from_slice(&fs::read(p).ok()?).ok())
        .collect();
    v.sort_by_key(|p| p.at);
    v
}

pub fn write_pending(data: &Path, p: &Pending) -> std::io::Result<()> {
    write_atomic(&approvals_dir(data).join(format!("{}.json", safe(&p.id))), &serde_json::to_vec(p).unwrap_or_default())
}

pub fn answer(data: &Path, request: &str, allow: bool) -> std::io::Result<()> {
    write_atomic(&approvals_dir(data).join(format!("{}.answer", safe(request))), if allow { b"allow" } else { b"deny" })
}

/// `Some(true)` allowed, `Some(false)` denied, `None` not answered yet.
pub fn read_answer(data: &Path, request: &str) -> Option<bool> {
    fs::read_to_string(approvals_dir(data).join(format!("{}.answer", safe(request)))).ok().map(|s| s.trim() == "allow")
}

pub fn clear_request(data: &Path, request: &str) {
    let base = approvals_dir(data).join(safe(request));
    let _ = fs::remove_file(base.with_extension("json"));
    let _ = fs::remove_file(base.with_extension("answer"));
}

// ── The plugin's heartbeat ────────────────────────────────────────────────────

pub fn alive_path(data: &Path) -> PathBuf {
    data.join("plugin.alive")
}

pub fn approvals_flag(data: &Path) -> PathBuf {
    data.join("approvals.on")
}

/// A file touched within `max` (by modification time).
pub fn fresh(path: &Path, max: Duration) -> bool {
    fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < max)
}

/// Drop what is left over: requests from hooks that are gone, sessions not heard from in a day.
pub fn prune(data: &Path) {
    let day = 24 * 3600 * 1000;
    let now = now_ms();
    for s in load_sessions(data) {
        if now.saturating_sub(s.updated) > day {
            remove_session(data, &s.agent, &s.id);
        }
    }
    for p in fs::read_dir(approvals_dir(data)).into_iter().flatten().flatten().map(|e| e.path()) {
        if !fresh(&p, Duration::from_secs(15 * 60)) {
            let _ = fs::remove_file(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("kd-agents-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        update_session(&dir, "claude", "a/b", |s| {
            s.cwd = "/home/me/proj".into();
            s.set_status(Status::Working);
        })
        .unwrap();
        let all = load_sessions(&dir);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name(), "proj");
        assert_eq!(all[0].status, Status::Working);
        let p = Pending { id: "r1".into(), agent: "claude".into(), session: "a/b".into(), tool: "Bash".into(), detail: "ls".into(), at: 1 };
        write_pending(&dir, &p).unwrap();
        assert_eq!(pending(&dir).len(), 1);
        answer(&dir, "r1", true).unwrap();
        assert!(pending(&dir).is_empty());
        assert_eq!(read_answer(&dir, "r1"), Some(true));
        clear_request(&dir, "r1");
        assert_eq!(read_answer(&dir, "r1"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn names() {
        let s = Session { agent: "codex".into(), cwd: "C:\\code\\app\\".into(), ..Default::default() };
        assert_eq!(s.name(), "app");
        let s = Session { agent: "codex".into(), ..Default::default() };
        assert_eq!(s.name(), "Codex");
    }
}
