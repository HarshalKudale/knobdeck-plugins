//! LIFX LAN protocol (binary, UDP port 56700): discovery by broadcast `GetService`, `LightGet`
//! for color, power and label, `LightSetColor` and `LightSetPower`. Every request waits for its
//! reply (or acknowledgement) and is sent again when none arrives, as UDP may drop it.
//! Shared by the plugin and its setup program.

use knobdeck_sdk::{Error, Result};
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

pub const PORT: u16 = 56700;
const HEADER: usize = 36;

// Message types.
pub const GET_SERVICE: u16 = 2;
pub const STATE_SERVICE: u16 = 3;
pub const ACK: u16 = 45;
pub const LIGHT_GET: u16 = 101;
pub const LIGHT_SET_COLOR: u16 = 102;
pub const LIGHT_STATE: u16 = 107;
pub const LIGHT_SET_POWER: u16 = 117;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Hsbk {
    pub hue: u16,
    pub saturation: u16,
    pub brightness: u16,
    pub kelvin: u16,
}

impl Hsbk {
    pub fn hue_deg(&self) -> f64 {
        (self.hue as f64 * 360.0 / 65536.0).round() % 360.0
    }
    pub fn sat_pct(&self) -> f64 {
        (self.saturation as f64 * 100.0 / 65535.0).round()
    }
    pub fn bri_pct(&self) -> f64 {
        (self.brightness as f64 * 100.0 / 65535.0).round()
    }
}

pub fn deg_to_u16(d: f64) -> u16 {
    ((d.rem_euclid(360.0) / 360.0) * 65536.0).round().min(65535.0) as u16
}

pub fn pct_to_u16(p: f64) -> u16 {
    (p.clamp(0.0, 100.0) / 100.0 * 65535.0).round() as u16
}

#[derive(Clone, Debug, PartialEq)]
pub struct LightState {
    pub color: Hsbk,
    pub on: bool,
    pub label: String,
}

/// A light: where it answers and its MAC (the `target` of every message, and its id here).
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    pub addr: SocketAddr,
    pub mac: [u8; 6],
}

impl Light {
    pub fn id(&self) -> String {
        self.mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
    }
}

/// One message: header fields plus payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    pub source: u32,
    pub target: [u8; 6],
    pub tagged: bool,
    pub ack_required: bool,
    pub res_required: bool,
    pub sequence: u8,
    pub kind: u16,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn encode(&self) -> Vec<u8> {
        let size = (HEADER + self.payload.len()) as u16;
        let mut b = Vec::with_capacity(size as usize);
        b.extend_from_slice(&size.to_le_bytes());
        // protocol 1024, addressable, tagged; origin 0.
        let flags: u16 = 1024 | (1 << 12) | ((self.tagged as u16) << 13);
        b.extend_from_slice(&flags.to_le_bytes());
        b.extend_from_slice(&self.source.to_le_bytes());
        b.extend_from_slice(&self.target);
        b.extend_from_slice(&[0, 0]); // target is 8 bytes; a MAC fills 6
        b.extend_from_slice(&[0; 6]); // reserved
        b.push((self.res_required as u8) | ((self.ack_required as u8) << 1));
        b.push(self.sequence);
        b.extend_from_slice(&[0; 8]); // reserved
        b.extend_from_slice(&self.kind.to_le_bytes());
        b.extend_from_slice(&[0; 2]); // reserved
        b.extend_from_slice(&self.payload);
        b
    }

    pub fn decode(b: &[u8]) -> Option<Packet> {
        if b.len() < HEADER {
            return None;
        }
        let size = u16::from_le_bytes([b[0], b[1]]) as usize;
        let flags = u16::from_le_bytes([b[2], b[3]]);
        if size < HEADER || size > b.len() || flags & 0x0fff != 1024 {
            return None;
        }
        let mut target = [0; 6];
        target.copy_from_slice(&b[8..14]);
        Some(Packet {
            source: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            target,
            tagged: flags & (1 << 13) != 0,
            res_required: b[22] & 1 != 0,
            ack_required: b[22] & 2 != 0,
            sequence: b[23],
            kind: u16::from_le_bytes([b[32], b[33]]),
            payload: b[HEADER..size].to_vec(),
        })
    }
}

fn u16_at(p: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([p[i], p[i + 1]])
}

pub fn encode_hsbk(c: &Hsbk) -> Vec<u8> {
    [c.hue, c.saturation, c.brightness, c.kelvin].iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// `LightState` payload (52 bytes): HSBK, reserved, power, label[32], reserved.
pub fn decode_state(p: &[u8]) -> Option<LightState> {
    if p.len() < 52 {
        return None;
    }
    let color = Hsbk { hue: u16_at(p, 0), saturation: u16_at(p, 2), brightness: u16_at(p, 4), kelvin: u16_at(p, 6) };
    let label = String::from_utf8_lossy(&p[12..44]).trim_end_matches('\0').to_string();
    Some(LightState { color, on: u16_at(p, 10) > 0, label })
}

/// `LightSetColor` payload: reserved, HSBK, duration (ms).
pub fn set_color_payload(c: &Hsbk, duration_ms: u32) -> Vec<u8> {
    let mut p = vec![0];
    p.extend(encode_hsbk(c));
    p.extend_from_slice(&duration_ms.to_le_bytes());
    p
}

/// `LightSetPower` payload: level (0 or 65535), duration (ms).
pub fn set_power_payload(on: bool, duration_ms: u32) -> Vec<u8> {
    let mut p = (if on { 65535u16 } else { 0 }).to_le_bytes().to_vec();
    p.extend_from_slice(&duration_ms.to_le_bytes());
    p
}

/// Who this process is in the `source` field (lights echo it; 0 and 1 are reserved).
fn source() -> u32 {
    std::process::id().wrapping_mul(2654435761).max(2)
}

fn next_seq() -> u8 {
    static SEQ: AtomicU8 = AtomicU8::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn socket() -> Result<UdpSocket> {
    let s = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|e| Error::msg(format!("LIFX: no UDP socket ({e})")))?;
    let _ = s.set_broadcast(true);
    Ok(s)
}

/// `host` or `host:port` → address (the default port is 56700).
pub fn resolve(addr: &str) -> Option<SocketAddr> {
    let a = addr.trim();
    let with_port = if a.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) { a.to_string() } else { format!("{a}:{PORT}") };
    with_port.to_socket_addrs().ok()?.find(SocketAddr::is_ipv4)
}

/// Every light answering `GetService` within `wait`: broadcast on this network plus `extra`
/// addresses by unicast (lights on another subnet, or a test responder).
pub fn discover(extra: &[String], wait: Duration) -> Vec<Light> {
    let Ok(sock) = socket() else { return vec![] };
    let src = source();
    let msg = Packet { source: src, target: [0; 6], tagged: true, ack_required: false, res_required: true, sequence: next_seq(), kind: GET_SERVICE, payload: vec![] }.encode();
    let mut dests: Vec<SocketAddr> = vec![SocketAddr::from((Ipv4Addr::BROADCAST, PORT))];
    dests.extend(extra.iter().filter_map(|a| resolve(a)));
    let mut found: Vec<Light> = vec![];
    let end = Instant::now() + wait;
    let mut buf = [0u8; 1024];
    // Send three times over the wait: UDP broadcasts get lost.
    let mut sends = 0;
    while let Some(left) = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
        if sends < 3 && left <= wait.mul_f32(1.0 - sends as f32 / 3.0) {
            for d in &dests {
                let _ = sock.send_to(&msg, d);
            }
            sends += 1;
        }
        let _ = sock.set_read_timeout(Some(left.min(Duration::from_millis(200))));
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let Some(p) = Packet::decode(&buf[..n]) else { continue };
        // StateService payload: service (1 = UDP), port.
        if p.kind != STATE_SERVICE || p.source != src || p.payload.len() < 5 || p.payload[0] != 1 {
            continue;
        }
        let port = u32::from_le_bytes([p.payload[1], p.payload[2], p.payload[3], p.payload[4]]) as u16;
        let addr = SocketAddr::new(from.ip(), if port == 0 { PORT } else { port });
        if !found.iter().any(|l| l.mac == p.target) {
            found.push(Light { addr, mac: p.target });
        }
    }
    found
}

/// Send `kind` with `payload` to `light` and wait for a reply of type `want` (an `ACK` when only
/// an acknowledgement is asked for); sent up to three times.
fn request(light: &Light, kind: u16, payload: Vec<u8>, want: u16) -> Result<Vec<u8>> {
    let sock = socket()?;
    let src = source();
    let seq = next_seq();
    let msg = Packet { source: src, target: light.mac, tagged: false, ack_required: want == ACK, res_required: want != ACK, sequence: seq, kind, payload }.encode();
    let mut buf = [0u8; 1024];
    for _ in 0..3 {
        sock.send_to(&msg, light.addr).map_err(|e| Error::msg(format!("Could not send to LIFX {}: {e}", light.addr.ip())))?;
        let end = Instant::now() + Duration::from_millis(400);
        while let Some(left) = end.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
            let _ = sock.set_read_timeout(Some(left));
            let Ok((n, _)) = sock.recv_from(&mut buf) else { break };
            if let Some(p) = Packet::decode(&buf[..n]) {
                if p.source == src && p.sequence == seq && p.kind == want {
                    return Ok(p.payload);
                }
            }
        }
    }
    Err(Error::msg(format!("LIFX light {} did not answer", light.addr.ip())))
}

pub fn get(light: &Light) -> Result<LightState> {
    let p = request(light, LIGHT_GET, vec![], LIGHT_STATE)?;
    decode_state(&p).ok_or_else(|| Error::msg(format!("LIFX light {} sent an invalid state", light.addr.ip())))
}

pub fn set_color(light: &Light, c: &Hsbk, duration_ms: u32) -> Result<()> {
    request(light, LIGHT_SET_COLOR, set_color_payload(c, duration_ms), ACK).map(|_| ())
}

pub fn set_power(light: &Light, on: bool, duration_ms: u32) -> Result<()> {
    request(light, LIGHT_SET_POWER, set_power_payload(on, duration_ms), ACK).map(|_| ())
}

/// `#RRGGBB` → HSBK (hue, saturation and brightness from the color; `kelvin` kept).
pub fn hex_to_hsbk(hex: &str, kelvin: u16) -> Option<Hsbk> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    let (r, g, b) = (c(0)?, c(2)?, c(4)?);
    let max = r.max(g).max(b);
    let d = max - r.min(g).min(b);
    let hue = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let sat = if max == 0.0 { 0.0 } else { d / max };
    Some(Hsbk { hue: deg_to_u16(hue), saturation: (sat * 65535.0).round() as u16, brightness: (max * 65535.0).round() as u16, kelvin })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn header_round_trip() {
        let p = Packet { source: 0x1234_5678, target: [0xd0, 0x73, 0xd5, 1, 2, 3], tagged: false, ack_required: true, res_required: false, sequence: 7, kind: LIGHT_SET_COLOR, payload: set_color_payload(&Hsbk { hue: 1, saturation: 2, brightness: 3, kelvin: 3500 }, 0) };
        let b = p.encode();
        assert_eq!(b.len(), 36 + 13);
        assert_eq!(&b[0..2], &(49u16).to_le_bytes());
        assert_eq!(&b[2..4], &[0x00, 0x14]); // protocol 1024 + addressable
        assert_eq!(b[22], 2);
        assert_eq!(&b[32..34], &[102, 0]);
        assert_eq!(Packet::decode(&b), Some(p));
        assert!(Packet::decode(&b[..20]).is_none());
    }

    #[test]
    fn get_service_matches_the_spec() {
        // The documented discovery packet (source 0, sequence 0, tagged): 24 00 00 34 …, type 2.
        let b = Packet { source: 0, target: [0; 6], tagged: true, ack_required: false, res_required: false, sequence: 0, kind: GET_SERVICE, payload: vec![] }.encode();
        let mut want = vec![0x24, 0x00, 0x00, 0x34];
        want.extend([0u8; 28]);
        want.extend([0x02, 0x00, 0x00, 0x00]);
        assert_eq!(b, want);
    }

    #[test]
    fn state_payload() {
        let mut p = encode_hsbk(&Hsbk { hue: 21845, saturation: 65535, brightness: 32768, kelvin: 3500 });
        p.extend([0, 0]);
        p.extend(65535u16.to_le_bytes());
        let mut label = b"Desk".to_vec();
        label.resize(32, 0);
        p.extend(label);
        p.extend([0; 8]);
        let s = decode_state(&p).unwrap();
        assert!(s.on);
        assert_eq!(s.label, "Desk");
        assert_eq!(s.color.hue_deg(), 120.0);
        assert_eq!(s.color.bri_pct(), 50.0);
        assert!(decode_state(&p[..40]).is_none());
    }

    #[test]
    fn conversions() {
        assert_eq!(deg_to_u16(0.0), 0);
        assert_eq!(deg_to_u16(360.0), 0);
        assert_eq!(deg_to_u16(180.0), 32768);
        assert_eq!(deg_to_u16(359.999), 65535);
        assert_eq!(pct_to_u16(100.0), 65535);
        let red = hex_to_hsbk("#FF0000", 3500).unwrap();
        assert_eq!((red.hue, red.saturation, red.brightness, red.kelvin), (0, 65535, 65535, 3500));
        assert_eq!(hex_to_hsbk("#0000FF", 0).unwrap().hue_deg(), 240.0);
        assert!(hex_to_hsbk("#F00", 0).is_none());
        assert_eq!(resolve("127.0.0.1"), Some("127.0.0.1:56700".parse().unwrap()));
        assert_eq!(resolve("127.0.0.1:5000"), Some("127.0.0.1:5000".parse().unwrap()));
    }

    /// A fake bulb on 127.0.0.1: answers GetService, LightGet, SetColor and SetPower like a
    /// real one (each set acknowledged when asked). Returns its address and its state.
    pub fn fake_bulb() -> (SocketAddr, Arc<Mutex<LightState>>) {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = sock.local_addr().unwrap();
        let state = Arc::new(Mutex::new(LightState { color: Hsbk { hue: 0, saturation: 0, brightness: 65535, kelvin: 3500 }, on: false, label: "Fake".into() }));
        let st = state.clone();
        let mac = [0xd0, 0x73, 0xd5, 0xaa, 0xbb, 0xcc];
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok((n, from)) = sock.recv_from(&mut buf) {
                let Some(p) = Packet::decode(&buf[..n]) else { continue };
                let reply = |kind: u16, payload: Vec<u8>| {
                    let r = Packet { source: p.source, target: mac, tagged: false, ack_required: false, res_required: false, sequence: p.sequence, kind, payload };
                    let _ = sock.send_to(&r.encode(), from);
                };
                let mut s = st.lock().unwrap();
                match p.kind {
                    GET_SERVICE => {
                        let mut pl = vec![1];
                        pl.extend((addr.port() as u32).to_le_bytes());
                        reply(STATE_SERVICE, pl);
                    }
                    LIGHT_GET => {
                        let mut pl = encode_hsbk(&s.color);
                        pl.extend([0, 0]);
                        pl.extend((if s.on { 65535u16 } else { 0 }).to_le_bytes());
                        let mut label = s.label.as_bytes().to_vec();
                        label.resize(32, 0);
                        pl.extend(label);
                        pl.extend([0; 8]);
                        reply(LIGHT_STATE, pl);
                    }
                    LIGHT_SET_COLOR if p.payload.len() >= 13 => {
                        let c = &p.payload[1..9];
                        s.color = Hsbk { hue: u16_at(c, 0), saturation: u16_at(c, 2), brightness: u16_at(c, 4), kelvin: u16_at(c, 6) };
                    }
                    LIGHT_SET_POWER if p.payload.len() >= 2 => s.on = u16_at(&p.payload, 0) > 0,
                    _ => {}
                }
                if p.ack_required {
                    reply(ACK, vec![]);
                }
            }
        });
        (addr, state)
    }

    #[test]
    fn against_a_fake_bulb() {
        let (addr, state) = fake_bulb();
        let lights = discover(&[addr.to_string()], Duration::from_millis(600));
        let light = lights.iter().find(|l| l.addr == addr).expect("fake bulb found").clone();
        assert_eq!(light.id(), "d0:73:d5:aa:bb:cc");
        let s = get(&light).unwrap();
        assert_eq!((s.on, s.label.as_str()), (false, "Fake"));
        set_power(&light, true, 0).unwrap();
        set_color(&light, &Hsbk { hue: 100, saturation: 200, brightness: 300, kelvin: 4000 }, 0).unwrap();
        let s = get(&light).unwrap();
        assert!(s.on);
        assert_eq!(s.color, Hsbk { hue: 100, saturation: 200, brightness: 300, kelvin: 4000 });
        assert_eq!(state.lock().unwrap().color.kelvin, 4000);
    }
}
