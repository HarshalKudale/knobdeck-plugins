//! Home Assistant REST API (`/api/…`) with a long-lived access token. Shared by the plugin and
//! its setup program.

use knobdeck_sdk::{Error, Result, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct Conn {
    pub url: String,
    pub token: String,
}

impl Conn {
    pub fn new(url: &str, token: &str) -> Conn {
        Conn { url: normalize_url(url), token: token.trim().to_string() }
    }

    /// From the saved setup (`HA_URL` in `.env`, `HA_TOKEN` in the OS credential store).
    pub fn from_env() -> Result<Conn> {
        let (url, token) = (knobdeck_sdk::env_or("HA_URL", ""), knobdeck_sdk::secrets::env_or_stored("HA_TOKEN"));
        if url.is_empty() || token.is_empty() {
            return Err(Error::msg("Run setup to connect to Home Assistant"));
        }
        Ok(Conn::new(&url, &token))
    }

    fn req(&self, method: &str, path: &str) -> ureq::Request {
        ureq::request(method, &format!("{}/api/{path}", self.url)).timeout(Duration::from_secs(8)).set("Authorization", &format!("Bearer {}", self.token))
    }

    fn reply(&self, r: std::result::Result<ureq::Response, ureq::Error>) -> Result<Value> {
        match r {
            Ok(resp) => resp.into_json().map_err(|e| Error::msg(format!("Home Assistant sent an invalid reply: {e}"))),
            Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
                Err(Error::msg("Home Assistant refused the access token. Create a new long-lived token and run setup again"))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let body = resp.into_string().unwrap_or_default();
                let msg = serde_message(&body).unwrap_or(body);
                Err(Error::msg(format!("Home Assistant: {} ({code})", if msg.is_empty() { "request failed".into() } else { msg })))
            }
            Err(e) => {
                knobdeck_sdk::log(format!("request failed: {e}"));
                Err(Error::msg(format!("Home Assistant at {} did not answer. Check the URL and that Home Assistant is running", self.url)))
            }
        }
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.reply(self.req("GET", path).call())
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value> {
        self.reply(self.req("POST", path).send_json(body.clone()))
    }

    /// `GET /api/` answers `{"message": "API running."}` for a good URL and token.
    #[allow(dead_code)] // used by setup
    pub fn check(&self) -> Result<()> {
        self.get("").map(|_| ())
    }

    pub fn call(&self, domain: &str, service: &str, data: &Value) -> Result<()> {
        self.post(&format!("services/{domain}/{service}"), data).map(|_| ())
    }
}

fn serde_message(body: &str) -> Option<String> {
    serde_json::from_str::<Value>(body).ok().and_then(|v| v["message"].as_str().map(String::from))
}

/// "homeassistant.local:8123/" → "http://homeassistant.local:8123".
pub fn normalize_url(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    let u = u.strip_suffix("/api").unwrap_or(u);
    if u.contains("://") {
        u.to_string()
    } else {
        format!("http://{u}")
    }
}

/// Every key polls; one `/api/states` reply serves them all for a moment.
static STATES: Mutex<Option<(Instant, Vec<Value>)>> = Mutex::new(None);
const FRESH: Duration = Duration::from_millis(1500);

pub fn states(conn: &Conn) -> Result<Vec<Value>> {
    if let Some((at, v)) = STATES.lock().unwrap().as_ref() {
        if at.elapsed() < FRESH {
            return Ok(v.clone());
        }
    }
    let list = conn.get("states")?.as_array().cloned().unwrap_or_default();
    *STATES.lock().unwrap() = Some((Instant::now(), list.clone()));
    Ok(list)
}

/// Forget the cached states (after a service call, so keys show the change at once).
pub fn invalidate() {
    *STATES.lock().unwrap() = None;
}

pub fn state(conn: &Conn, entity: &str) -> Result<Value> {
    states(conn)?.into_iter().find(|s| s["entity_id"] == entity).ok_or_else(|| Error::msg(format!("Home Assistant has no entity {entity}")))
}

pub fn domain(entity: &str) -> &str {
    entity.split_once('.').map_or("", |(d, _)| d)
}

pub fn friendly_name(s: &Value) -> String {
    s["attributes"]["friendly_name"].as_str().or(s["entity_id"].as_str()).unwrap_or("").to_string()
}

/// Domains a Toggle key can switch.
pub const TOGGLE: &[&str] = &["light", "switch", "fan", "input_boolean", "automation", "cover", "lock", "media_player", "climate", "siren", "humidifier", "valve", "vacuum", "group"];
/// Domains an Activate key runs.
pub const ACTIVATE: &[&str] = &["scene", "script", "automation", "button", "input_button"];
/// Domains a Level dial sets as 0–100 %.
pub const LEVEL: &[&str] = &["light", "cover", "fan", "media_player", "input_number", "number", "valve"];

/// Whether the entity counts as "on" for a Toggle key.
pub fn is_on(s: &Value) -> bool {
    matches!(s["state"].as_str().unwrap_or(""), "on" | "open" | "opening" | "unlocked" | "unlocking" | "playing" | "home" | "heat" | "cool" | "heat_cool" | "auto" | "dry" | "fan_only" | "cleaning")
}

/// The service a Toggle key calls, given the entity's current state.
pub fn toggle_service(entity: &str, s: &Value) -> (&'static str, &'static str) {
    let on = is_on(s);
    match domain(entity) {
        "lock" => ("lock", if on { "lock" } else { "unlock" }),
        "cover" => ("cover", "toggle"),
        "valve" => ("valve", "toggle"),
        "media_player" => ("media_player", "media_play_pause"),
        "vacuum" => ("vacuum", if on { "return_to_base" } else { "start" }),
        "climate" => ("climate", if on { "turn_off" } else { "turn_on" }),
        "automation" => ("automation", "toggle"),
        _ => ("homeassistant", "toggle"),
    }
}

/// The service an Activate key calls.
pub fn activate_service(entity: &str) -> (&'static str, &'static str) {
    match domain(entity) {
        "scene" => ("scene", "turn_on"),
        "script" => ("script", "turn_on"),
        "automation" => ("automation", "trigger"),
        "input_button" => ("input_button", "press"),
        _ => ("button", "press"),
    }
}

/// The entity's level as 0–100 % (None when it has none right now, e.g. a light that is off).
pub fn level_of(s: &Value) -> Option<f64> {
    let a = &s["attributes"];
    match domain(s["entity_id"].as_str().unwrap_or("")) {
        "light" => Some(if s["state"] == "on" { a["brightness"].as_f64().map_or(100.0, |b| b / 255.0 * 100.0) } else { 0.0 }),
        "cover" => a["current_position"].as_f64(),
        "valve" => a["current_position"].as_f64(),
        "fan" => Some(if s["state"] == "on" { a["percentage"].as_f64().unwrap_or(100.0) } else { 0.0 }),
        "media_player" => a["volume_level"].as_f64().map(|v| v * 100.0),
        "input_number" | "number" => {
            let (min, max) = (a["min"].as_f64().unwrap_or(0.0), a["max"].as_f64().unwrap_or(100.0));
            let v: f64 = s["state"].as_str()?.parse().ok()?;
            (max > min).then(|| (v - min) / (max - min) * 100.0)
        }
        _ => None,
    }
    .map(|v| v.round().clamp(0.0, 100.0))
}

/// The service call that sets the entity to `pct` %.
pub fn level_call(s: &Value, pct: f64) -> Result<(&'static str, &'static str, Value)> {
    let entity = s["entity_id"].as_str().unwrap_or("");
    let a = &s["attributes"];
    let pct = pct.round().clamp(0.0, 100.0);
    Ok(match domain(entity) {
        "light" if pct == 0.0 => ("light", "turn_off", knobdeck_sdk::json!({"entity_id": entity})),
        "light" => ("light", "turn_on", knobdeck_sdk::json!({"entity_id": entity, "brightness_pct": pct})),
        "cover" => ("cover", "set_cover_position", knobdeck_sdk::json!({"entity_id": entity, "position": pct})),
        "valve" => ("valve", "set_valve_position", knobdeck_sdk::json!({"entity_id": entity, "position": pct})),
        "fan" if pct == 0.0 => ("fan", "turn_off", knobdeck_sdk::json!({"entity_id": entity})),
        "fan" => ("fan", "set_percentage", knobdeck_sdk::json!({"entity_id": entity, "percentage": pct})),
        "media_player" => ("media_player", "volume_set", knobdeck_sdk::json!({"entity_id": entity, "volume_level": pct / 100.0})),
        d @ ("input_number" | "number") => {
            let (min, max) = (a["min"].as_f64().unwrap_or(0.0), a["max"].as_f64().unwrap_or(100.0));
            let step = a["step"].as_f64().filter(|s| *s > 0.0).unwrap_or(1.0);
            let raw = min + (max - min) * pct / 100.0;
            let value = ((raw - min) / step).round() * step + min;
            (if d == "number" { "number" } else { "input_number" }, "set_value", knobdeck_sdk::json!({"entity_id": entity, "value": value.clamp(min, max)}))
        }
        d => return Err(Error::msg(format!("{d} entities have no level to set"))),
    })
}

/// A Lucide icon for the entity's domain.
pub fn icon_for(entity: &str, on: bool) -> &'static str {
    match domain(entity) {
        "light" => {
            if on {
                "lightbulb"
            } else {
                "lightbulb-off"
            }
        }
        "switch" | "input_boolean" => {
            if on {
                "toggle-right"
            } else {
                "toggle-left"
            }
        }
        "fan" => "fan",
        "cover" | "valve" => "blinds",
        "lock" => {
            if on {
                "lock-open"
            } else {
                "lock"
            }
        }
        "media_player" => "speaker",
        "climate" | "humidifier" => "thermometer",
        "scene" => "sparkles",
        "script" => "scroll-text",
        "automation" => "workflow",
        "button" | "input_button" => "circle-dot",
        "siren" => "siren",
        "vacuum" => "bot",
        "sensor" => "gauge",
        "binary_sensor" => "radio",
        "person" | "device_tracker" => "user",
        "sun" => "sun",
        "weather" => "cloud-sun",
        _ => "house",
    }
}

/// The text a State key shows: the state (or an attribute) with its unit.
pub fn display(s: &Value, attribute: &str) -> String {
    let attribute = attribute.trim();
    if !attribute.is_empty() {
        return match &s["attributes"][attribute] {
            Value::Null => "—".into(),
            Value::String(t) => t.clone(),
            v => v.to_string(),
        };
    }
    let state = s["state"].as_str().unwrap_or("");
    let unit = s["attributes"]["unit_of_measurement"].as_str().unwrap_or("");
    let state = match state {
        "unavailable" => "Unavailable".to_string(),
        "unknown" => "—".to_string(),
        // Round long sensor numbers ("21.456789" → "21.5").
        t => match t.parse::<f64>() {
            Ok(n) if t.contains('.') && t.len() > 6 => format!("{n:.1}"),
            _ => t.replace('_', " "),
        },
    };
    if unit.is_empty() {
        state
    } else if unit == "%" || unit.starts_with('°') {
        format!("{state}{unit}")
    } else {
        format!("{state} {unit}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    #[test]
    fn urls() {
        assert_eq!(normalize_url("homeassistant.local:8123/"), "http://homeassistant.local:8123");
        assert_eq!(normalize_url("https://x.ui.nabu.casa/api"), "https://x.ui.nabu.casa");
        assert_eq!(normalize_url(" http://10.0.0.2:8123 "), "http://10.0.0.2:8123");
    }

    #[test]
    fn levels() {
        let light = json!({"entity_id": "light.desk", "state": "on", "attributes": {"brightness": 128}});
        assert_eq!(level_of(&light), Some(50.0));
        assert_eq!(level_of(&json!({"entity_id": "light.desk", "state": "off", "attributes": {}})), Some(0.0));
        let num = json!({"entity_id": "input_number.t", "state": "20.0", "attributes": {"min": 10, "max": 30, "step": 0.5}});
        assert_eq!(level_of(&num), Some(50.0));
        let (d, s, data) = level_call(&num, 26.0).unwrap();
        assert_eq!((d, s), ("input_number", "set_value"));
        assert_eq!(data["value"], 15.0);
        let (_, s, data) = level_call(&light, 0.0).unwrap();
        assert_eq!(s, "turn_off");
        assert!(data.get("brightness_pct").is_none());
        let mp = json!({"entity_id": "media_player.tv", "state": "playing", "attributes": {"volume_level": 0.25}});
        assert_eq!(level_of(&mp), Some(25.0));
        assert_eq!(level_call(&mp, 40.0).unwrap().2["volume_level"], 0.4);
        assert!(level_call(&json!({"entity_id": "sensor.x", "state": "1"}), 5.0).is_err());
    }

    #[test]
    fn toggles_and_text() {
        let lock = json!({"entity_id": "lock.door", "state": "locked"});
        assert_eq!(toggle_service("lock.door", &lock), ("lock", "unlock"));
        assert_eq!(toggle_service("light.a", &json!({"state": "on"})), ("homeassistant", "toggle"));
        assert_eq!(activate_service("scene.movie"), ("scene", "turn_on"));
        assert_eq!(display(&json!({"state": "21.456789", "attributes": {"unit_of_measurement": "°C"}}), ""), "21.5°C");
        assert_eq!(display(&json!({"state": "450", "attributes": {"unit_of_measurement": "W"}}), ""), "450 W");
        assert_eq!(display(&json!({"state": "on", "attributes": {"temperature": 21}}), "temperature"), "21");
        assert_eq!(display(&json!({"state": "not_home", "attributes": {}}), ""), "not home");
    }
}
