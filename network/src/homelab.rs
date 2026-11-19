//! Pi-hole (v6 REST API: `/api/auth`, `/api/dns/blocking`, `/api/stats/summary`) and Uptime Kuma
//! (a public status page: `/api/status-page/<slug>` and `/api/status-page/heartbeat/<slug>`).

use knobdeck_sdk::{env_or, json, Error, Result, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(6))
        .build()
}

/// "pi.hole" → "http://pi.hole"; trailing "/" and "/admin" dropped.
pub fn base_url(text: &str) -> String {
    let t = text.trim().trim_end_matches('/');
    let t = t.strip_suffix("/admin").unwrap_or(t);
    if t.contains("://") {
        t.to_string()
    } else {
        format!("http://{t}")
    }
}

// ── Pi-hole ──

static SID: Mutex<Option<String>> = Mutex::new(None);

fn pihole_base() -> Result<String> {
    let b = env_or("PIHOLE_URL", "");
    if b.trim().is_empty() {
        return Err(Error::msg("Add your Pi-hole's address in setup"));
    }
    Ok(base_url(&b))
}

/// Log in: a session id, or `None` when the Pi-hole has no password.
pub fn pihole_login(base: &str, password: &str) -> Result<Option<String>> {
    let r = agent()
        .post(&format!("{base}/api/auth"))
        .send_json(json!({"password": password}));
    let v: Value = match r {
        Ok(r) => r
            .into_json()
            .map_err(|e| Error::msg(format!("Pi-hole sent an invalid reply: {e}")))?,
        Err(ureq::Error::Status(401, _)) => {
            return Err(Error::msg(
                "Pi-hole refused the password (use the web password or an app password)",
            ))
        }
        Err(ureq::Error::Status(404, _)) => {
            return Err(Error::msg(
                "This doesn't look like Pi-hole v6 (no /api/auth); Pi-hole v5 isn't supported",
            ))
        }
        Err(ureq::Error::Status(429, _)) => {
            return Err(Error::msg(
                "Pi-hole: too many login attempts, wait a moment",
            ))
        }
        Err(ureq::Error::Status(code, _)) => {
            return Err(Error::msg(format!("Pi-hole error {code}")))
        }
        Err(_) => return Err(Error::msg(format!("Pi-hole unreachable at {base}"))),
    };
    let s = &v["session"];
    if s["valid"] != true {
        return Err(Error::msg(
            s["message"]
                .as_str()
                .map(|m| format!("Pi-hole: {m}"))
                .unwrap_or_else(|| "Pi-hole refused the password".into()),
        ));
    }
    Ok(s["sid"].as_str().map(String::from))
}

/// One API request with the cached session, logging in again once when it expired.
fn pihole(method: &str, path: &str, body: Option<Value>) -> Result<Value> {
    let base = pihole_base()?;
    for attempt in 0..2 {
        let cached = SID.lock().unwrap().clone();
        let sid = match cached {
            Some(s) => s,
            None => {
                // An empty session id: the Pi-hole has no password.
                let s = pihole_login(
                    &base,
                    &knobdeck_sdk::secrets::env_or_stored("PIHOLE_PASSWORD"),
                )?
                .unwrap_or_default();
                *SID.lock().unwrap() = Some(s.clone());
                s
            }
        };
        let mut req = agent().request(method, &format!("{base}{path}"));
        if !sid.is_empty() {
            req = req.set("X-FTL-SID", &sid);
        }
        let r = match &body {
            Some(b) => req.send_json(b.clone()),
            None => req.call(),
        };
        match r {
            Ok(r) => {
                return r
                    .into_json()
                    .map_err(|e| Error::msg(format!("Pi-hole sent an invalid reply: {e}")))
            }
            Err(ureq::Error::Status(401, _)) if attempt == 0 => *SID.lock().unwrap() = None,
            Err(ureq::Error::Status(code, r)) => {
                let msg = r
                    .into_json::<Value>()
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(String::from))
                    .unwrap_or_default();
                return Err(Error::msg(
                    format!("Pi-hole error {code} {msg}").trim().to_string(),
                ));
            }
            Err(_) => return Err(Error::msg(format!("Pi-hole unreachable at {base}"))),
        }
    }
    Err(Error::msg("Pi-hole refused the session"))
}

#[derive(Debug, PartialEq)]
pub struct Blocking {
    pub on: bool,
    /// Seconds until blocking switches back, when a timer runs.
    pub timer: Option<f64>,
}

pub fn parse_blocking(v: &Value) -> Blocking {
    Blocking {
        on: v["blocking"] == "enabled",
        timer: v["timer"].as_f64(),
    }
}

pub fn blocking() -> Result<Blocking> {
    Ok(parse_blocking(&pihole("GET", "/api/dns/blocking", None)?))
}

/// Switch blocking; `for_secs` > 0 switches it back after that long.
pub fn set_blocking(on: bool, for_secs: u64) -> Result<Blocking> {
    let timer = if for_secs > 0 {
        json!(for_secs)
    } else {
        Value::Null
    };
    Ok(parse_blocking(&pihole(
        "POST",
        "/api/dns/blocking",
        Some(json!({"blocking": on, "timer": timer})),
    )?))
}

/// (percent blocked today, blocked, total).
pub fn summary() -> Result<(f64, u64, u64)> {
    let v = pihole("GET", "/api/stats/summary", None)?;
    let q = &v["queries"];
    Ok((
        q["percent_blocked"].as_f64().unwrap_or(0.0),
        q["blocked"].as_u64().unwrap_or(0),
        q["total"].as_u64().unwrap_or(0),
    ))
}

// ── Uptime Kuma ──

/// "http://kuma:3001/status/home" → ("http://kuma:3001", "home").
pub fn status_page(url: &str) -> Result<(String, String)> {
    let u = base_url(url);
    let (base, slug) = u.split_once("/status/").ok_or_else(|| {
        Error::msg("Enter the status page's address, like http://kuma.local:3001/status/home")
    })?;
    let slug = slug.split(['/', '?', '#']).next().unwrap_or("");
    if slug.is_empty() {
        return Err(Error::msg(
            "The status page address has no page name after /status/",
        ));
    }
    Ok((base.to_string(), slug.to_string()))
}

fn kuma_get(url: &str) -> Result<Value> {
    match agent().get(url).call() {
        Ok(r) => r
            .into_json()
            .map_err(|e| Error::msg(format!("Uptime Kuma sent an invalid reply: {e}"))),
        Err(ureq::Error::Status(404, _)) => Err(Error::msg("No such Uptime Kuma status page")),
        Err(ureq::Error::Status(code, _)) => Err(Error::msg(format!("Uptime Kuma error {code}"))),
        Err(_) => Err(Error::msg("Uptime Kuma unreachable")),
    }
}

type Names = HashMap<String, (Instant, HashMap<String, String>)>;
static NAMES: Mutex<Option<Names>> = Mutex::new(None);

/// Monitor id → name, read every 10 minutes (names rarely change).
fn monitor_names(base: &str, slug: &str) -> Result<HashMap<String, String>> {
    let key = format!("{base}/{slug}");
    if let Some((t, m)) = NAMES
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|n| n.get(&key))
        .cloned()
    {
        if t.elapsed() < Duration::from_secs(600) {
            return Ok(m);
        }
    }
    let v = kuma_get(&format!("{base}/api/status-page/{slug}"))?;
    let mut m = HashMap::new();
    for g in v["publicGroupList"].as_array().into_iter().flatten() {
        for mon in g["monitorList"].as_array().into_iter().flatten() {
            if let (Some(id), Some(name)) = (mon["id"].as_i64(), mon["name"].as_str()) {
                m.insert(id.to_string(), name.to_string());
            }
        }
    }
    NAMES
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(key, (Instant::now(), m.clone()));
    Ok(m)
}

#[derive(Debug, PartialEq)]
pub struct Health {
    pub up: usize,
    pub total: usize,
    /// Names of monitors whose latest beat is down (status 0).
    pub down: Vec<String>,
}

/// Latest beat per monitor: 0 down, 1 up, 2 pending, 3 maintenance (counted as not down).
pub fn parse_heartbeats(v: &Value, names: &HashMap<String, String>) -> Health {
    let mut h = Health {
        up: 0,
        total: 0,
        down: vec![],
    };
    let mut ids: Vec<(&String, &Value)> = v["heartbeatList"]
        .as_object()
        .map(|m| m.iter().collect())
        .unwrap_or_default();
    ids.sort_by_key(|(id, _)| id.parse::<i64>().unwrap_or(0));
    for (id, beats) in ids {
        let Some(last) = beats.as_array().and_then(|b| b.last()) else {
            continue;
        };
        h.total += 1;
        match last["status"].as_i64() {
            Some(0) => h
                .down
                .push(names.get(id).cloned().unwrap_or_else(|| format!("#{id}"))),
            _ => h.up += 1,
        }
    }
    h
}

pub fn health(page: &str) -> Result<Health> {
    let (base, slug) = status_page(page)?;
    let beats = kuma_get(&format!("{base}/api/status-page/heartbeat/{slug}"))?;
    let names = monitor_names(&base, &slug).unwrap_or_default();
    Ok(parse_heartbeats(&beats, &names))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(base_url("pi.hole/admin/"), "http://pi.hole");
        assert_eq!(
            base_url("https://192.168.1.2:8443"),
            "https://192.168.1.2:8443"
        );
        assert_eq!(
            status_page("kuma.lan:3001/status/home?x=1").unwrap(),
            ("http://kuma.lan:3001".into(), "home".into())
        );
        assert!(status_page("http://kuma.lan:3001/dashboard").is_err());
    }

    #[test]
    fn pihole_replies() {
        assert_eq!(
            parse_blocking(&json!({"blocking": "disabled", "timer": 299.5})),
            Blocking {
                on: false,
                timer: Some(299.5)
            }
        );
        assert_eq!(
            parse_blocking(&json!({"blocking": "enabled", "timer": null})),
            Blocking {
                on: true,
                timer: None
            }
        );
    }

    #[test]
    fn kuma_beats() {
        let names: HashMap<String, String> = [
            ("1".to_string(), "NAS".to_string()),
            ("2".to_string(), "Plex".to_string()),
        ]
        .into();
        let v = json!({"heartbeatList": {"2": [{"status": 1}, {"status": 0}], "1": [{"status": 1}], "3": [{"status": 3}], "4": []}});
        assert_eq!(
            parse_heartbeats(&v, &names),
            Health {
                up: 2,
                total: 3,
                down: vec!["Plex".into()]
            }
        );
    }
}
