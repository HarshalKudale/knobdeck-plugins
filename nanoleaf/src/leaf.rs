//! Nanoleaf OpenAPI (`http://<device>:16021/api/v1/<token>/…`), pairing and mDNS discovery
//! (`_nanoleafapi._tcp`). Shared by the plugin and its setup program. Works with Shapes, Elements,
//! Canvas, Lines, Light Panels and the Essentials lightstrip / bulbs that run the OpenAPI.

use knobdeck_sdk::{json, Error, Result, Value};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

pub const PORT: u16 = 16021;
const SERVICE: &[u8] = b"\x0c_nanoleafapi\x04_tcp\x05local\x00";
/// Message setup and the plugin show when a device refuses to pair.
pub const NOT_PAIRING: &str = "not in pairing mode";

/// Devices answering an mDNS query within `wait` (see elgato-lights: a query from a port other
/// than 5353 is answered by unicast, so the reply's source address is the device).
pub fn discover(wait: Duration) -> Vec<String> {
    let Ok(sock) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else { return vec![] };
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

pub fn base(addr: &str) -> String {
    if addr.contains(':') {
        format!("http://{addr}/api/v1")
    } else {
        format!("http://{addr}:{PORT}/api/v1")
    }
}

/// Secret name holding a device's token.
pub fn token_key(addr: &str) -> String {
    format!("NANOLEAF_TOKEN_{}", addr.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect::<String>())
}

fn unreachable(addr: &str, e: ureq::Error) -> Error {
    match e {
        ureq::Error::Status(401, _) => Error::msg(format!("{addr} no longer accepts KnobDeck's token: run setup again")),
        e => Error::msg(format!("Nanoleaf {addr} did not answer ({e})")),
    }
}

/// Ask for a token. Works only while the device is in pairing mode (power button held 5–7 s).
pub fn pair(addr: &str) -> Result<String> {
    match ureq::post(&format!("{}/new", base(addr))).timeout(Duration::from_secs(3)).call() {
        Ok(r) => {
            let v: Value = r.into_json().map_err(|e| Error::msg(format!("{addr} sent an invalid reply: {e}")))?;
            v["auth_token"].as_str().map(String::from).ok_or_else(|| Error::msg(format!("{addr} sent no token")))
        }
        Err(ureq::Error::Status(403, _)) => Err(Error::msg(NOT_PAIRING)),
        Err(e) => Err(unreachable(addr, e)),
    }
}

pub struct Device {
    pub addr: String,
    pub token: String,
}

impl Device {
    fn url(&self, path: &str) -> String {
        format!("{}/{}/{path}", base(&self.addr), self.token)
    }

    /// Everything about the device: `name`, `state`, `effects`…
    pub fn info(&self) -> Result<Value> {
        ureq::get(&self.url("")).timeout(Duration::from_secs(3)).call().map_err(|e| unreachable(&self.addr, e))?.into_json().map_err(|e| Error::msg(format!("{} sent an invalid reply: {e}", self.addr)))
    }

    fn put(&self, path: &str, body: Value) -> Result<()> {
        ureq::put(&self.url(path)).timeout(Duration::from_secs(3)).send_json(body).map_err(|e| unreachable(&self.addr, e))?;
        Ok(())
    }

    /// Change the state: `{"on": {"value": true}}`, `{"brightness": {"value": 40}}`…
    pub fn set(&self, state: Value) -> Result<()> {
        self.put("state", state)
    }

    pub fn select_effect(&self, name: &str) -> Result<()> {
        self.put("effects", json!({"select": name}))
    }

    pub fn effects(&self) -> Result<Vec<String>> {
        let v: Value = ureq::get(&self.url("effects/effectsList")).timeout(Duration::from_secs(3)).call().map_err(|e| unreachable(&self.addr, e))?.into_json().map_err(|e| Error::msg(format!("{} sent an invalid reply: {e}", self.addr)))?;
        Ok(v.as_array().map(|a| a.iter().filter_map(|e| e.as_str().map(String::from)).collect()).unwrap_or_default())
    }
}

/// `#RRGGBB` → (hue 0-360, saturation 0-100, brightness 0-100).
pub fn hex_to_hsb(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    let (r, g, b) = (c(0)?, c(2)?, c(4)?);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let hue = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let sat = if max == 0.0 { 0.0 } else { d / max * 100.0 };
    Some((hue.round(), sat.round(), (max * 100.0).round()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        assert_eq!(base("10.0.0.5"), "http://10.0.0.5:16021/api/v1");
        assert_eq!(base("127.0.0.1:9000"), "http://127.0.0.1:9000/api/v1");
        assert_eq!(token_key("10.0.0.5"), "NANOLEAF_TOKEN_10_0_0_5");
        assert_eq!(hex_to_hsb("#00FF00"), Some((120.0, 100.0, 100.0)));
        assert_eq!(hex_to_hsb("bad"), None);
    }
}
