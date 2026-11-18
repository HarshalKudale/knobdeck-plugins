//! WLED JSON API (`http://<device>/json/…`, `/presets.json`) and discovery over mDNS
//! (`_wled._tcp`). Shared by the plugin and its setup program.

use knobdeck_sdk::{json, Error, Result, Value};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

/// `_wled._tcp.local` as DNS labels.
const SERVICE: &[u8] = b"\x05_wled\x04_tcp\x05local\x00";
const TIMEOUT: Duration = Duration::from_secs(3);

/// Devices answering an mDNS query within `wait` (see elgato-lights: a query from a port other
/// than 5353 is answered by unicast, so the reply's source address is the device).
pub fn discover(wait: Duration) -> Vec<String> {
    let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return vec![] };
    if sock.send_to(&query(), (Ipv4Addr::new(224, 0, 0, 251), 5353)).is_err() {
        return vec![];
    }
    let mut found = vec![];
    let end = Instant::now() + wait;
    let mut buf = [0u8; 1500];
    while let Some(left) = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
        let _ = sock.set_read_timeout(Some(left));
        let Ok((n, SocketAddr::V4(from))) = sock.recv_from(&mut buf) else { break };
        let ip = from.ip().to_string();
        if names_service(&buf[..n]) && !found.contains(&ip) {
            found.push(ip);
        }
    }
    found
}

/// The mDNS question: PTR `_wled._tcp.local`, class IN with the unicast-response bit.
fn query() -> Vec<u8> {
    let mut q = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    q.extend_from_slice(SERVICE);
    q.extend_from_slice(&[0, 12, 0x80, 1]);
    q
}

/// Whether an mDNS reply names the WLED service (anything else on the network is ignored).
fn names_service(reply: &[u8]) -> bool {
    reply.windows(SERVICE.len()).any(|w| w == SERVICE)
}

/// `http://<addr>` (an address may carry its own port: "192.168.1.50:8080").
pub fn base(addr: &str) -> String {
    format!("http://{}", addr.trim().trim_end_matches('/'))
}

fn unreachable(addr: &str, e: ureq::Error) -> Error {
    Error::msg(format!("WLED {addr} did not answer ({e})"))
}

fn get(addr: &str, path: &str) -> Result<Value> {
    ureq::get(&format!("{}{path}", base(addr))).timeout(TIMEOUT).call().map_err(|e| unreachable(addr, e))?.into_json().map_err(|e| Error::msg(format!("{addr} sent an invalid reply: {e}")))
}

/// `/json/info`: `name`, `ver`, `leds.count`, `brand` ("WLED")…
pub fn info(addr: &str) -> Result<Value> {
    get(addr, "/json/info")
}

/// The device's name as set in WLED (Config ▸ User Interface ▸ Server description), checking that
/// the address really is a WLED device.
pub fn name(addr: &str) -> Result<String> {
    let i = info(addr)?;
    if i["ver"].is_null() && i["brand"] != "WLED" {
        return Err(Error::msg(format!("{addr} isn't a WLED device")));
    }
    Ok(i["name"].as_str().filter(|s| !s.is_empty()).unwrap_or("WLED").to_string())
}

/// `/json/state`: `on`, `bri` (0–255), `ps` (active preset, -1 none), `seg` [{id, on, bri, fx,
/// sx, ix, pal, col, sel, n}].
pub fn state(addr: &str) -> Result<Value> {
    get(addr, "/json/state")
}

/// Change part of the state; returns the new state (`"v": true` asks WLED to answer with it).
pub fn set(addr: &str, mut patch: Value) -> Result<Value> {
    patch["v"] = json!(true);
    ureq::post(&format!("{}/json/state", base(addr))).timeout(TIMEOUT).send_json(patch).map_err(|e| unreachable(addr, e))?.into_json().map_err(|e| Error::msg(format!("{addr} sent an invalid reply: {e}")))
}

/// Effect names, by index (`fx`). Names may carry WLED's own `@` metadata, cut off here.
pub fn effects(addr: &str) -> Result<Vec<String>> {
    Ok(names(&get(addr, "/json/effects")?))
}

/// Palette names, by index (`pal`).
pub fn palettes(addr: &str) -> Result<Vec<String>> {
    Ok(names(&get(addr, "/json/palettes")?))
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(|n| n.as_str().unwrap_or("").split('@').next().unwrap_or("").trim().to_string()).collect()).unwrap_or_default()
}

/// Presets saved on the device: (id, name), by id. Preset 0 is WLED's empty placeholder.
pub fn presets(addr: &str) -> Result<Vec<(i64, String)>> {
    Ok(parse_presets(&get(addr, "/presets.json")?))
}

pub fn parse_presets(v: &Value) -> Vec<(i64, String)> {
    let mut out: Vec<(i64, String)> = v
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, p)| {
                    let id: i64 = k.parse().ok().filter(|id| *id > 0)?;
                    let p = p.as_object().filter(|p| !p.is_empty())?;
                    let name = p.get("n").and_then(Value::as_str).filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| format!("Preset {id}"));
                    Some((id, name))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|(id, _)| *id);
    out
}

/// The `seg` part of a state patch that changes `fields` on the segments a key targets:
/// `Some(id)` for one segment, `None` for the segments selected in WLED (WLED applies an object
/// `seg` to every selected segment), or every segment when `all` is set.
pub fn seg_patch(target: SegTarget, fields: &Value, state: Option<&Value>) -> Value {
    let with_id = |id: i64| {
        let mut f = fields.clone();
        f["id"] = json!(id);
        f
    };
    match target {
        SegTarget::Selected => fields.clone(),
        SegTarget::One(id) => json!([with_id(id)]),
        SegTarget::All => Value::Array(segment_ids(state.unwrap_or(&Value::Null)).into_iter().map(with_id).collect()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SegTarget {
    Selected,
    All,
    One(i64),
}

impl SegTarget {
    pub fn parse(s: &str) -> SegTarget {
        match s {
            "all" => SegTarget::All,
            s => s.parse().map(SegTarget::One).unwrap_or(SegTarget::Selected),
        }
    }
}

/// Ids of the segments that exist (WLED lists only active ones; a deleted one has `stop` 0).
pub fn segment_ids(state: &Value) -> Vec<i64> {
    state["seg"].as_array().map(|a| a.iter().filter(|s| s["stop"].as_i64() != Some(0) || s["len"].as_i64().is_some_and(|l| l > 0)).filter_map(|s| s["id"].as_i64()).collect()).unwrap_or_default()
}

/// The segment whose values a key shows: the chosen one, else the first selected one, else the
/// main segment (`mainseg`), else the first.
pub fn shown_segment(state: &Value, target: SegTarget) -> Value {
    let segs = state["seg"].as_array().cloned().unwrap_or_default();
    let by_id = |id: i64| segs.iter().find(|s| s["id"].as_i64() == Some(id)).cloned();
    match target {
        SegTarget::One(id) => by_id(id),
        _ => segs.iter().find(|s| s["sel"] == true).cloned().or_else(|| state["mainseg"].as_i64().and_then(by_id)),
    }
    .or_else(|| segs.first().cloned())
    .unwrap_or(Value::Null)
}

/// 0–255 → 0–100 %.
pub fn to_pct(v: f64) -> f64 {
    (v.clamp(0.0, 255.0) * 100.0 / 255.0).round()
}

/// 0–100 % → 0–255.
pub fn from_pct(p: f64) -> i64 {
    (p.clamp(0.0, 100.0) * 255.0 / 100.0).round() as i64
}

pub fn hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([c(0)?, c(2)?, c(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdns() {
        let q = query();
        assert_eq!(&q[4..6], &[0, 1]); // one question
        assert!(names_service(&q));
        assert!(!names_service(b"\x04_elg\x04_tcp\x05local\x00"));
    }

    #[test]
    fn preset_list() {
        let v = json!({"0": {}, "3": {"n": "Evening", "on": true}, "1": {"n": "Party"}, "2": {"on": true}, "x": {"n": "bad"}});
        assert_eq!(parse_presets(&v), vec![(1, "Party".into()), (2, "Preset 2".into()), (3, "Evening".into())]);
    }

    #[test]
    fn effect_names() {
        assert_eq!(names(&json!(["Solid", "Blink@!,;!,!;!", "Rainbow"])), vec!["Solid", "Blink", "Rainbow"]);
    }

    #[test]
    fn segments() {
        let st = json!({"mainseg": 1, "seg": [{"id": 0, "stop": 30, "sel": false, "fx": 1}, {"id": 1, "stop": 60, "sel": false, "fx": 2}, {"id": 2, "stop": 0, "len": 0}]});
        assert_eq!(segment_ids(&st), vec![0, 1]);
        assert_eq!(shown_segment(&st, SegTarget::Selected)["fx"], 2); // mainseg
        assert_eq!(shown_segment(&st, SegTarget::One(0))["fx"], 1);
        let f = json!({"fx": 5});
        assert_eq!(seg_patch(SegTarget::Selected, &f, None), json!({"fx": 5}));
        assert_eq!(seg_patch(SegTarget::One(1), &f, None), json!([{"fx": 5, "id": 1}]));
        assert_eq!(seg_patch(SegTarget::All, &f, Some(&st)), json!([{"fx": 5, "id": 0}, {"fx": 5, "id": 1}]));
        assert_eq!(SegTarget::parse("all"), SegTarget::All);
        assert_eq!(SegTarget::parse("3"), SegTarget::One(3));
        assert_eq!(SegTarget::parse(""), SegTarget::Selected);
    }

    #[test]
    fn levels() {
        assert_eq!(to_pct(255.0), 100.0);
        assert_eq!(to_pct(128.0), 50.0);
        assert_eq!(from_pct(100.0), 255);
        assert_eq!(from_pct(50.0), 128);
        assert_eq!(hex("#FF8800"), Some([255, 136, 0]));
        assert_eq!(hex("#F80"), None);
    }

    #[test]
    fn addresses() {
        assert_eq!(base("192.168.1.5"), "http://192.168.1.5");
        assert_eq!(base("127.0.0.1:8080/"), "http://127.0.0.1:8080");
    }
}
