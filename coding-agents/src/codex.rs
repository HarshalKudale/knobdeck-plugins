//! Codex has no "turn started" event, only `notify` when a turn ends. While a turn runs, Codex
//! appends to its session log (`~/.codex/sessions/YYYY/MM/DD/rollout-…-<id>.jsonl`), so a log
//! written in the last few seconds means that session is working.

use crate::store::{Session, Status};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A log written within this long counts as a running turn.
const ACTIVE: Duration = Duration::from_secs(8);
/// Logs older than this aren't looked at.
const RECENT: Duration = Duration::from_secs(6 * 3600);

fn sessions_root() -> Option<PathBuf> {
    crate::install::codex_config_path().map(|c| c.with_file_name("sessions"))
}

/// Subfolders of `dir`, newest name first.
fn subdirs_desc(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    v.sort();
    v.reverse();
    v
}

/// The session id and folder from a log's first line (`{"type": "session_meta", "payload": {…}}`),
/// falling back to the id at the end of the file name.
fn meta(path: &Path) -> (String, String) {
    let first = fs::File::open(path).ok().and_then(|f| BufReader::new(f).lines().next()).and_then(|l| l.ok()).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(&first).unwrap_or_default();
    let p = &v["payload"];
    let id = p["id"].as_str().map(str::to_string).unwrap_or_else(|| {
        // rollout-2026-11-17T10-00-00-<uuid>.jsonl: the uuid is the last five dash groups.
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let parts: Vec<&str> = stem.split('-').collect();
        parts[parts.len().saturating_sub(5)..].join("-")
    });
    (id, p["cwd"].as_str().unwrap_or_default().to_string())
}

/// Codex sessions whose log was written recently, with `Working` for those written just now.
pub fn active() -> Vec<Session> {
    let Some(root) = sessions_root() else { return vec![] };
    let mut out = vec![];
    // The two newest day folders cover a session that runs past midnight.
    let days: Vec<PathBuf> = subdirs_desc(&root).into_iter().take(1).flat_map(|y| subdirs_desc(&y).into_iter().take(2)).flat_map(|m| subdirs_desc(&m).into_iter().take(2)).take(2).collect();
    for day in days {
        for f in fs::read_dir(&day).into_iter().flatten().flatten().map(|e| e.path()) {
            if f.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let Some(age) = fs::metadata(&f).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()) else { continue };
            if age > RECENT {
                continue;
            }
            let (id, cwd) = meta(&f);
            let modified = SystemTime::now().checked_sub(age).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
            let status = if age < ACTIVE { Status::Working } else { Status::Idle };
            out.push(Session { agent: "codex".into(), id, cwd, title: None, status, updated: modified, since: modified });
        }
    }
    out
}

/// Merge what the logs show into what `notify` recorded: a log written after the last
/// recorded change means a new turn started.
pub fn merge(recorded: &mut Vec<Session>, seen: Vec<Session>) {
    for s in seen {
        match recorded.iter_mut().find(|r| r.agent == "codex" && r.id == s.id) {
            Some(r) => {
                if r.cwd.is_empty() {
                    r.cwd = s.cwd;
                }
                if s.status == Status::Working && s.updated > r.updated + 1000 {
                    r.status = Status::Working;
                    r.since = s.updated;
                    r.updated = s.updated;
                }
            }
            None => recorded.push(s),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges() {
        let mut rec = vec![Session { agent: "codex".into(), id: "a".into(), status: Status::Done, updated: 1000, ..Default::default() }];
        merge(&mut rec, vec![Session { agent: "codex".into(), id: "a".into(), cwd: "/x".into(), status: Status::Working, updated: 5000, ..Default::default() }]);
        assert_eq!(rec[0].status, Status::Working);
        assert_eq!(rec[0].cwd, "/x");
        // A notify after the last write keeps "your turn".
        let mut rec = vec![Session { agent: "codex".into(), id: "a".into(), status: Status::Done, updated: 9000, ..Default::default() }];
        merge(&mut rec, vec![Session { agent: "codex".into(), id: "a".into(), status: Status::Working, updated: 8500, ..Default::default() }]);
        assert_eq!(rec[0].status, Status::Done);
        merge(&mut rec, vec![Session { agent: "codex".into(), id: "b".into(), status: Status::Idle, ..Default::default() }]);
        assert_eq!(rec.len(), 2);
    }

    #[test]
    fn id_from_file_name() {
        let dir = std::env::temp_dir().join(format!("kd-codex-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("rollout-2026-11-17T10-00-00-0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b.jsonl");
        fs::write(&f, "not json\n").unwrap();
        assert_eq!(meta(&f).0, "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b");
        fs::write(&f, "{\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"/w/p\"}}\n").unwrap();
        assert_eq!(meta(&f), ("abc".into(), "/w/p".into()));
        let _ = fs::remove_dir_all(&dir);
    }
}
