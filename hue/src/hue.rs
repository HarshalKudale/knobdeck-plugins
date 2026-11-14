//! Philips Hue Bridge local API (`http://<bridge>/api/<username>/…`), bridge discovery (mDNS
//! `_hue._tcp`, then Signify's discovery service) and link-button pairing. Shared by the plugin
//! and its setup program.

use knobdeck_sdk::{json, Error, Result, Value};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// `_hue._tcp.local` as DNS labels.
const SERVICE: &[u8] = b"\x04_hue\x04_tcp\x05local\x00";

/// Bridges answering an mDNS query within `wait`. A query sent from a port other than 5353 is
/// answered by unicast to that port (RFC 6762 §6.7), so the reply's source address is the bridge.
pub fn discover_mdns(wait: Duration) -> Vec<String> {
    let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return vec![] };
    // Header: id 0, flags 0, one question; question: PTR (12), class IN with the unicast-response bit.
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    q.extend_from_slice(SERVICE);
    q.extend_from_slice(&[0, 12, 0x80, 1]);
    if sock.send_to(&q, (Ipv4Addr::new(224, 0, 0, 251), 5353)).is_err() {
        return vec![];
    }
    let mut found = vec![];
    let end = Instant::now() + wait;
    let mut buf = [0u8; 1500];
    while let Some(left) = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
        let _ = sock.set_read_timeout(Some(left));
        let Ok((n, SocketAddr::V4(from))) = sock.recv_from(&mut buf) else { break };
        let ip = from.ip().to_string();
        if buf[..n].windows(SERVICE.len()).any(|w| w == SERVICE) && !found.contains(&ip) {
            found.push(ip);
        }
    }
    found
}

/// Bridges registered with Signify's discovery service from this network (rate limited; used
/// only when mDNS finds nothing).
pub fn discover_cloud() -> Vec<String> {
    let Ok(r) = ureq::get("https://discovery.meethue.com/").timeout(Duration::from_secs(5)).call() else { return vec![] };
    r.into_json::<Value>().ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().iter().filter_map(|b| b["internalipaddress"].as_str().map(String::from)).collect()
}

pub fn discover(wait: Duration) -> Vec<String> {
    let found = discover_mdns(wait);
    if found.is_empty() {
        discover_cloud()
    } else {
        found
    }
}

fn base(addr: &str) -> String {
    if addr.contains("://") {
        addr.trim_end_matches('/').to_string()
    } else {
        format!("http://{}", addr.trim_end_matches('/'))
    }
}

fn reply(addr: &str, r: std::result::Result<ureq::Response, ureq::Error>) -> Result<Value> {
    match r {
        Ok(resp) => resp.into_json().map_err(|e| Error::msg(format!("The Hue Bridge sent an invalid reply: {e}"))),
        Err(ureq::Error::Status(code, _)) => Err(Error::msg(format!("The Hue Bridge at {addr} refused the request ({code})"))),
        Err(e) => {
            knobdeck_sdk::log(format!("bridge {addr}: {e}"));
            Err(Error::Other(format!("{UNREACHABLE} ({addr})")))
        }
    }
}

pub const UNREACHABLE: &str = "The Hue Bridge did not answer. Check that it is on and on the same network";

/// The first `{"error": …}` in a bridge reply, as a user-facing error.
pub fn bridge_error(v: &Value) -> Option<Error> {
    let errs = match v {
        Value::Array(a) => a.iter().filter_map(|e| e.get("error")).cloned().collect::<Vec<_>>(),
        Value::Object(_) => v.get("error").cloned().into_iter().collect(),
        _ => vec![],
    };
    let e = errs.first()?;
    Some(match e["type"].as_i64() {
        Some(1) => Error::msg(UNAUTHORIZED),
        Some(101) => Error::msg(LINK_BUTTON),
        _ => Error::msg(format!("Hue Bridge: {}", e["description"].as_str().unwrap_or("request failed"))),
    })
}

pub const UNAUTHORIZED: &str = "The Hue Bridge no longer knows KnobDeck. Run setup again to pair";
pub const LINK_BUTTON: &str = "Press the round link button on the Hue Bridge, then run setup again within 30 seconds";

/// Public bridge details (no pairing needed): `{"name", "bridgeid", "modelid", "apiversion", …}`.
pub fn public_config(addr: &str) -> Result<Value> {
    reply(addr, ureq::get(&format!("{}/api/config", base(addr))).timeout(Duration::from_secs(3)).call())
}

/// Ask the bridge for a username. Fails with [`LINK_BUTTON`] until its button has been pressed.
#[allow(dead_code)] // used by setup
pub fn pair(addr: &str, device: &str) -> Result<String> {
    // devicetype is "<app>#<device>", at most 40 characters.
    let device: String = device.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(19).collect();
    let v = reply(addr, ureq::post(&format!("{}/api", base(addr))).timeout(Duration::from_secs(5)).send_json(json!({"devicetype": format!("knobdeck#{device}")})))?;
    if let Some(e) = bridge_error(&v) {
        return Err(e);
    }
    v[0]["success"]["username"].as_str().map(String::from).ok_or_else(|| Error::msg("The Hue Bridge sent no username"))
}

/// A paired bridge. The address can change (DHCP): on a failed request the bridge is found again
/// by its id.
pub struct Bridge {
    pub addr: Mutex<String>,
    pub user: String,
    pub id: String,
}

impl Bridge {
    fn url(&self, path: &str) -> String {
        format!("{}/api/{}/{path}", base(&self.addr.lock().unwrap()), self.user)
    }

    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let attempt = |b: &Bridge| {
            let req = ureq::request(method, &b.url(path)).timeout(Duration::from_secs(4));
            let addr = b.addr.lock().unwrap().clone();
            reply(&addr, match body {
                Some(j) => req.send_json(j.clone()),
                None => req.call(),
            })
        };
        let v = match attempt(self) {
            Err(Error::Other(m)) if m.starts_with(UNREACHABLE) && self.relocate() => attempt(self)?,
            r => r?,
        };
        match bridge_error(&v) {
            Some(e) => Err(e),
            None => Ok(v),
        }
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        self.send("GET", path, None)
    }

    pub fn put(&self, path: &str, body: &Value) -> Result<Value> {
        self.send("PUT", path, Some(body))
    }

    /// Find this bridge again (by id) after its address changed; saves the new address.
    fn relocate(&self) -> bool {
        if self.id.is_empty() {
            return false;
        }
        for addr in discover(Duration::from_millis(1500)) {
            let same = public_config(&addr).is_ok_and(|c| c["bridgeid"].as_str().is_some_and(|id| id.eq_ignore_ascii_case(&self.id)));
            if same {
                knobdeck_sdk::log(format!("bridge moved to {addr}"));
                // save_env replaces .env: keep the bridge id alongside the new address.
                let _ = knobdeck_sdk::save_env(&[("HUE_BRIDGE", &addr), ("HUE_BRIDGE_ID", &self.id)]);
                *self.addr.lock().unwrap() = addr;
                return true;
            }
        }
        false
    }
}

/// A light or a group (room, zone, or group 0 = every light).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Light(String),
    Group(String),
}

impl Target {
    pub fn parse(s: &str) -> Option<Target> {
        match s.trim().split_once(':') {
            Some(("light", id)) if !id.is_empty() => Some(Target::Light(id.into())),
            Some(("group", id)) if !id.is_empty() => Some(Target::Group(id.into())),
            _ if s.trim().is_empty() => Some(Target::Group("0".into())),
            _ => None,
        }
    }
}

/// `lights` and `groups` are fetched once for every key polling within a moment.
static CACHE: Mutex<Vec<(String, Instant, Value)>> = Mutex::new(Vec::new());
const FRESH: Duration = Duration::from_millis(1500);

pub fn cached(b: &Bridge, path: &str) -> Result<Value> {
    if let Some((_, _, v)) = CACHE.lock().unwrap().iter().find(|(p, at, _)| p == path && at.elapsed() < FRESH) {
        return Ok(v.clone());
    }
    let v = b.get(path)?;
    let mut c = CACHE.lock().unwrap();
    c.retain(|(p, _, _)| p != path);
    c.push((path.to_string(), Instant::now(), v.clone()));
    Ok(v)
}

pub fn invalidate() {
    CACHE.lock().unwrap().clear();
}

/// `{"on": bool, "bri": 1-254?, "ct": 153-500?}` for a light (its state) or a group (its last
/// action, with `on` = any light on). Group 0 is not in `/groups`, so it is read directly.
pub fn state(b: &Bridge, t: &Target) -> Result<Value> {
    match t {
        Target::Light(id) => {
            let l = cached(b, "lights")?.get(id).cloned().ok_or_else(|| Error::msg(format!("The Hue Bridge has no light {id}")))?;
            Ok(l["state"].clone())
        }
        Target::Group(id) => {
            let g = if id == "0" { b.get("groups/0")? } else { cached(b, "groups")?.get(id).cloned().ok_or_else(|| Error::msg(format!("The Hue Bridge has no room or zone {id}")))? };
            let mut s = g["action"].clone();
            s["on"] = json!(g["state"]["any_on"].as_bool().unwrap_or(false));
            Ok(s)
        }
    }
}

pub fn set(b: &Bridge, t: &Target, body: &Value) -> Result<()> {
    match t {
        Target::Light(id) => b.put(&format!("lights/{id}/state"), body)?,
        Target::Group(id) => b.put(&format!("groups/{id}/action"), body)?,
    };
    invalidate();
    Ok(())
}

pub fn bri_to_pct(bri: f64) -> f64 {
    (bri / 254.0 * 100.0).round().clamp(0.0, 100.0)
}

pub fn pct_to_bri(pct: f64) -> i64 {
    (pct / 100.0 * 254.0).round().clamp(1.0, 254.0) as i64
}

/// Hue colour temperature is in mireds (153–500, 6500–2000 K).
pub fn kelvin_to_mired(k: f64) -> i64 {
    (1_000_000.0 / k.clamp(2000.0, 6500.0)).round().clamp(153.0, 500.0) as i64
}

pub fn mired_to_kelvin(m: i64) -> i64 {
    ((1_000_000.0 / m.clamp(153, 500) as f64) / 50.0).round() as i64 * 50
}

/// `#RRGGBB` → CIE xy (Signify's RGB-to-xy recipe: sRGB gamma, wide-gamut D65 matrix).
pub fn hex_to_xy(hex: &str) -> Option<[f64; 2]> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let c = |i: usize| {
        u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| {
            let v = v as f64 / 255.0;
            if v > 0.04045 {
                ((v + 0.055) / 1.055).powf(2.4)
            } else {
                v / 12.92
            }
        })
    };
    let (r, g, b) = (c(0)?, c(2)?, c(4)?);
    let x = r * 0.664511 + g * 0.154324 + b * 0.162028;
    let y = r * 0.283881 + g * 0.668433 + b * 0.047685;
    let z = r * 0.000088 + g * 0.072310 + b * 0.986039;
    let sum = x + y + z;
    if sum == 0.0 {
        return Some([0.3127, 0.3290]); // black: keep the white point; brightness does the rest
    }
    let round = |v: f64| (v * 10000.0).round() / 10000.0;
    Some([round(x / sum), round(y / sum)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(Target::parse("light:3"), Some(Target::Light("3".into())));
        assert_eq!(Target::parse("group:1"), Some(Target::Group("1".into())));
        assert_eq!(Target::parse(""), Some(Target::Group("0".into())));
        assert_eq!(Target::parse("lamp"), None);
    }

    #[test]
    fn units() {
        assert_eq!(pct_to_bri(100.0), 254);
        assert_eq!(pct_to_bri(0.0), 1);
        assert_eq!(bri_to_pct(127.0), 50.0);
        assert_eq!(kelvin_to_mired(2000.0), 500);
        assert_eq!(kelvin_to_mired(6500.0), 154);
        assert_eq!(mired_to_kelvin(153), 6550);
        assert_eq!(mired_to_kelvin(kelvin_to_mired(4000.0)), 4000);
    }

    #[test]
    fn colors() {
        let red = hex_to_xy("#FF0000").unwrap();
        assert!((red[0] - 0.7006).abs() < 0.001 && (red[1] - 0.2993).abs() < 0.001, "{red:?}");
        assert_eq!(hex_to_xy("#000000"), Some([0.3127, 0.3290]));
        assert_eq!(hex_to_xy("#12"), None);
    }

    #[test]
    fn errors() {
        assert_eq!(bridge_error(&json!([{"error": {"type": 101, "description": "link button not pressed"}}])).unwrap().message(), LINK_BUTTON);
        assert_eq!(bridge_error(&json!([{"error": {"type": 1, "description": "unauthorized user"}}])).unwrap().message(), UNAUTHORIZED);
        assert!(bridge_error(&json!([{"success": {"/lights/1/state/on": true}}])).is_none());
        assert!(bridge_error(&json!({"1": {"name": "x"}})).is_none());
    }
}
