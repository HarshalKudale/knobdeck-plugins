//! Govee LAN API (UDP, JSON): scan by multicast to 239.255.255.250:4001, commands to the device's
//! port 4003, and every reply (scan and status) to this computer's port 4002. The user turns it
//! on per device in the Govee Home app (device ▸ settings ▸ LAN Control).

use knobdeck_sdk::{json, Error, Result, Value};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

const GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const SCAN_PORT: u16 = 4001;
const REPLY_PORT: u16 = 4002;
const CMD_PORT: u16 = 4003;

#[derive(Clone, Debug, PartialEq)]
pub struct LanDevice {
    pub ip: String,
    /// The device id ("AA:BB:…"), the same one the cloud API uses.
    pub id: String,
    pub sku: String,
}

#[derive(Default)]
struct Inbox {
    /// Devices seen answering a scan, by id.
    devices: HashMap<String, LanDevice>,
    /// Last status reply per IP, with when it arrived.
    status: HashMap<String, (Instant, Value)>,
}

pub struct Lan {
    sock: UdpSocket,
    inbox: Arc<(Mutex<Inbox>, Condvar)>,
}

fn reply_socket() -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    // SO_REUSEADDR only (no SO_REUSEPORT, which would split replies between sockets): Govee's
    // own desktop app or this plugin's setup may hold the port at the same time.
    s.set_reuse_address(true)?;
    s.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, REPLY_PORT)).into())?;
    let s: UdpSocket = s.into();
    let _ = s.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED);
    Ok(s)
}

/// Read one message: `(cmd, data)`.
pub fn parse(buf: &[u8]) -> Option<(String, Value)> {
    let v: Value = serde_json::from_slice(buf).ok()?;
    Some((v["msg"]["cmd"].as_str()?.to_string(), v["msg"]["data"].clone()))
}

impl Lan {
    /// The shared listener (one socket on port 4002 for the plugin's lifetime).
    pub fn get() -> Result<&'static Lan> {
        static LAN: OnceLock<std::result::Result<Lan, String>> = OnceLock::new();
        LAN.get_or_init(|| Lan::start().map_err(|e| format!("Govee LAN control unavailable: port {REPLY_PORT} is busy or blocked ({e})"))).as_ref().map_err(|e| Error::msg(e.clone()))
    }

    fn start() -> std::io::Result<Lan> {
        let sock = reply_socket()?;
        let inbox: Arc<(Mutex<Inbox>, Condvar)> = Arc::default();
        let (rx, ib) = (sock.try_clone()?, inbox.clone());
        std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while let Ok((n, from)) = rx.recv_from(&mut buf) {
                let Some((cmd, data)) = parse(&buf[..n]) else { continue };
                let ip = from.ip().to_string();
                let mut g = ib.0.lock().unwrap();
                match cmd.as_str() {
                    "scan" => {
                        let id = data["device"].as_str().unwrap_or("").to_string();
                        if !id.is_empty() {
                            let ip = data["ip"].as_str().map(String::from).unwrap_or(ip);
                            g.devices.insert(id.clone(), LanDevice { ip, id, sku: data["sku"].as_str().unwrap_or("").to_string() });
                        }
                    }
                    "devStatus" => {
                        g.status.insert(ip, (Instant::now(), data));
                    }
                    _ => {}
                }
                ib.1.notify_all();
            }
        });
        Ok(Lan { sock, inbox })
    }

    /// Ask every device (multicast, plus `extra` addresses by unicast) to announce itself.
    pub fn scan(&self, extra: &[String], wait: Duration) -> Vec<LanDevice> {
        let msg = json!({"msg": {"cmd": "scan", "data": {"account_topic": "reserve"}}}).to_string();
        let _ = self.sock.send_to(msg.as_bytes(), (GROUP, SCAN_PORT));
        for ip in extra {
            let _ = self.sock.send_to(msg.as_bytes(), (ip.as_str(), SCAN_PORT));
        }
        std::thread::sleep(wait);
        let mut v: Vec<LanDevice> = self.inbox.0.lock().unwrap().devices.values().cloned().collect();
        v.sort_by(|a, b| a.ip.cmp(&b.ip));
        v
    }

    /// Devices seen so far (no new scan).
    pub fn known(&self) -> Vec<LanDevice> {
        self.inbox.0.lock().unwrap().devices.values().cloned().collect()
    }

    pub fn send(&self, ip: &str, cmd: &str, data: Value) -> Result<()> {
        let msg = json!({"msg": {"cmd": cmd, "data": data}}).to_string();
        self.sock.send_to(msg.as_bytes(), (ip, CMD_PORT)).map_err(|e| Error::msg(format!("Could not send to {ip}: {e}")))?;
        Ok(())
    }

    /// `{"onOff": 0|1, "brightness": 1-100, "color": {r,g,b}, "colorTemInKelvin": n}`.
    pub fn status(&self, ip: &str) -> Result<Value> {
        let asked = Instant::now();
        self.send(ip, "devStatus", json!({}))?;
        let (lock, cv) = &*self.inbox;
        let mut g = lock.lock().unwrap();
        loop {
            if let Some((at, v)) = g.status.get(ip) {
                if *at >= asked {
                    return Ok(v.clone());
                }
            }
            let left = Duration::from_millis(1500).checked_sub(asked.elapsed()).filter(|d| !d.is_zero()).ok_or_else(|| Error::msg(format!("Govee device {ip} did not answer (is LAN Control on?)")))?;
            g = cv.wait_timeout(g, left).unwrap().0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        let (cmd, data) = parse(br#"{"msg":{"cmd":"scan","data":{"ip":"192.168.1.23","device":"1F:80:C5:32:32:36:72:4E","sku":"H618E"}}}"#).unwrap();
        assert_eq!(cmd, "scan");
        assert_eq!(data["sku"], "H618E");
        assert!(parse(b"nope").is_none());
    }
}
