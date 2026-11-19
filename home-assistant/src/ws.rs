//! Live entity states over Home Assistant's WebSocket API (`/api/websocket`): authenticate with the
//! long-lived token, subscribe to `state_changed`, then read every state once with `get_states`.
//! The connection drops back to REST polling (see `ha::states`) whenever it is down.

use crate::ha::Conn;
use knobdeck_sdk::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}
type Socket = WebSocket<Box<dyn Stream>>;

/// What the feed tells its owner.
pub enum Update {
    /// Every entity's state (after each (re)connect).
    All(Vec<Value>),
    /// One entity changed; `None` = it was removed.
    Changed(String, Option<Value>),
}

/// Why the feed stopped.
#[derive(Debug, PartialEq)]
pub enum Stop {
    /// Home Assistant refused the token (`auth_invalid`).
    Refused(String),
    /// Anything else: unreachable, closed, timed out, a bad reply.
    Lost(String),
}

/// How long a quiet connection lives: ping after `PING`, give up after `DEAD` without any message.
const PING: Duration = Duration::from_secs(30);
const DEAD: Duration = Duration::from_secs(70);

/// `http://host:8123` → `ws://host:8123/api/websocket` (`https` → `wss`).
pub fn ws_url(base: &str) -> Option<String> {
    if let Some(rest) = base.strip_prefix("https://") {
        Some(format!("wss://{}/api/websocket", rest.trim_end_matches('/')))
    } else {
        base.strip_prefix("http://").map(|rest| format!("ws://{}/api/websocket", rest.trim_end_matches('/')))
    }
}

/// Host, port and TLS of a ws(s) URL.
fn host_port(url: &str) -> Option<(String, u16, bool)> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("wss://") { (true, r) } else { (false, url.strip_prefix("ws://")?) };
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    // [v6]:port, host:port, host
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (h, after) = v6.split_once(']')?;
        (h.to_string(), after.strip_prefix(':').and_then(|p| p.parse().ok()))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), Some(p.parse().ok()?)),
            None => (authority.to_string(), None),
        }
    };
    Some((host, port.unwrap_or(if tls { 443 } else { 80 }), tls))
}

fn tls_config() -> Arc<rustls::ClientConfig> {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Arc::new(rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().expect("ring supports the default TLS versions").with_root_certificates(roots).with_no_client_auth())
}

fn open(url: &str) -> Result<Socket, String> {
    let (host, port, tls) = host_port(url).ok_or_else(|| format!("bad URL {url}"))?;
    let addr = (host.as_str(), port).to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("no address")?;
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).map_err(|e| e.to_string())?;
    tcp.set_read_timeout(Some(Duration::from_secs(10))).map_err(|e| e.to_string())?;
    let _ = tcp.set_nodelay(true);
    let stream: Box<dyn Stream> = if tls {
        let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|e| e.to_string())?;
        let conn = rustls::ClientConnection::new(tls_config(), name).map_err(|e| e.to_string())?;
        Box::new(rustls::StreamOwned::new(conn, tcp))
    } else {
        Box::new(tcp)
    };
    let (ws, _) = tungstenite::client(url, stream).map_err(|e| e.to_string())?;
    Ok(ws)
}

fn send(ws: &mut Socket, v: Value) -> Result<(), Stop> {
    ws.send(Message::text(v.to_string())).map_err(|e| Stop::Lost(e.to_string()))
}

/// Read one JSON message; `Ok(None)` on a read timeout. Home Assistant may batch several messages
/// into one JSON array (`coalesce_messages`; only when asked for, but cheap to accept).
fn read(ws: &mut Socket) -> Result<Option<Vec<Value>>, Stop> {
    match ws.read() {
        Ok(Message::Text(t)) => match t.parse::<Value>() {
            Ok(Value::Array(a)) => Ok(Some(a)),
            Ok(v) => Ok(Some(vec![v])),
            Err(e) => Err(Stop::Lost(format!("bad message: {e}"))),
        },
        Ok(Message::Close(_)) => Err(Stop::Lost("Home Assistant closed the connection".into())),
        Ok(_) => Ok(Some(vec![])),
        Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
        Err(e) => Err(Stop::Lost(e.to_string())),
    }
}

/// Wait for the next message (the handshake has its own deadline).
fn next(ws: &mut Socket, until: Instant) -> Result<Value, Stop> {
    loop {
        if let Some(mut list) = read(ws)? {
            if !list.is_empty() {
                return Ok(list.remove(0));
            }
        }
        if Instant::now() > until {
            return Err(Stop::Lost("Home Assistant did not answer in time".into()));
        }
    }
}

/// Connect, authenticate and stream updates until the connection ends. `on_up` runs once the
/// first full state list has arrived. Never returns `Ok`.
pub fn run(conn: &Conn, on_update: &dyn Fn(Update), on_up: &dyn Fn()) -> Stop {
    let Some(url) = ws_url(&conn.url) else { return Stop::Lost(format!("no websocket for {}", conn.url)) };
    let mut ws = match open(&url) {
        Ok(ws) => ws,
        Err(e) => return Stop::Lost(e),
    };
    match session(&mut ws, &conn.token, on_update, on_up) {
        Ok(never) => match never {},
        Err(stop) => {
            let _ = ws.close(None);
            stop
        }
    }
}

enum Never {}

fn session(ws: &mut Socket, token: &str, on_update: &dyn Fn(Update), on_up: &dyn Fn()) -> Result<Never, Stop> {
    let deadline = Instant::now() + Duration::from_secs(15);
    // auth_required → auth → auth_ok | auth_invalid
    let first = next(ws, deadline)?;
    if first["type"] != "auth_required" {
        return Err(Stop::Lost(format!("unexpected greeting {first}")));
    }
    send(ws, json!({"type": "auth", "access_token": token}))?;
    let auth = next(ws, deadline)?;
    match auth["type"].as_str() {
        Some("auth_ok") => {}
        Some("auth_invalid") => return Err(Stop::Refused(auth["message"].as_str().unwrap_or("invalid token").to_string())),
        _ => return Err(Stop::Lost(format!("unexpected auth reply {auth}"))),
    }
    // Subscribe first, then read every state: no change can fall between the two.
    const SUB: u64 = 1;
    const ALL: u64 = 2;
    send(ws, json!({"id": SUB, "type": "subscribe_events", "event_type": "state_changed"}))?;
    send(ws, json!({"id": ALL, "type": "get_states"}))?;
    let mut ping_id = ALL;
    let (mut last_rx, mut last_ping) = (Instant::now(), Instant::now());
    loop {
        let Some(list) = read(ws)? else {
            if last_rx.elapsed() > DEAD {
                return Err(Stop::Lost("no reply to ping".into()));
            }
            if last_ping.elapsed() > PING && last_rx.elapsed() > PING {
                ping_id += 1;
                send(ws, json!({"id": ping_id, "type": "ping"}))?;
                last_ping = Instant::now();
            }
            continue;
        };
        last_rx = Instant::now();
        for m in list {
            match (m["type"].as_str(), m["id"].as_u64()) {
                (Some("event"), Some(SUB)) => {
                    let data = &m["event"]["data"];
                    if let Some(id) = data["entity_id"].as_str() {
                        let new = Some(data["new_state"].clone()).filter(|s| s.is_object());
                        on_update(Update::Changed(id.to_string(), new));
                    }
                }
                (Some("result"), Some(id)) if m["success"] != true => {
                    return Err(Stop::Lost(format!("request {id} failed: {}", m["error"]["message"].as_str().unwrap_or("?"))));
                }
                (Some("result"), Some(ALL)) => {
                    on_update(Update::All(m["result"].as_array().cloned().unwrap_or_default()));
                    on_up();
                }
                _ => {} // subscribe ack, pong
            }
        }
        // A busy instance never times out the read: ping on the clock too.
        if last_ping.elapsed() > PING * 2 {
            ping_id += 1;
            send(ws, json!({"id": ping_id, "type": "ping"}))?;
            last_ping = Instant::now();
        }
    }
}

/// Seconds to wait before reconnect attempt `n` (0-based): 1, 2, 4 … capped at 60.
pub fn backoff(n: u32) -> Duration {
    Duration::from_secs((1u64 << n.min(6)).min(60))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Mutex;

    #[test]
    fn urls() {
        assert_eq!(ws_url("http://homeassistant.local:8123").as_deref(), Some("ws://homeassistant.local:8123/api/websocket"));
        assert_eq!(ws_url("https://x.ui.nabu.casa").as_deref(), Some("wss://x.ui.nabu.casa/api/websocket"));
        assert_eq!(host_port("ws://h:8123/api/websocket"), Some(("h".into(), 8123, false)));
        assert_eq!(host_port("wss://x.ui.nabu.casa/api/websocket"), Some(("x.ui.nabu.casa".into(), 443, true)));
        assert_eq!(host_port("ws://[::1]:8123/api/websocket"), Some(("::1".into(), 8123, false)));
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(20), Duration::from_secs(60));
    }

    /// A mock Home Assistant: the auth handshake, a state list, one change, then a close.
    fn mock(token_ok: bool) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let h = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(s).unwrap();
            let mut got = vec![];
            let mut recv = |ws: &mut WebSocket<TcpStream>| -> Value {
                let v: Value = ws.read().unwrap().into_text().unwrap().parse().unwrap();
                got.push(v.clone());
                v
            };
            ws.send(Message::text(json!({"type": "auth_required", "ha_version": "2026.11.0"}).to_string())).unwrap();
            let auth = recv(&mut ws);
            if !token_ok || auth["access_token"] != "good" {
                ws.send(Message::text(json!({"type": "auth_invalid", "message": "Invalid access token or password"}).to_string())).unwrap();
                let _ = ws.close(None);
                return got;
            }
            ws.send(Message::text(json!({"type": "auth_ok", "ha_version": "2026.11.0"}).to_string())).unwrap();
            let sub = recv(&mut ws);
            let all = recv(&mut ws);
            ws.send(Message::text(json!({"id": sub["id"], "type": "result", "success": true, "result": null}).to_string())).unwrap();
            ws.send(Message::text(json!({"id": all["id"], "type": "result", "success": true, "result": [{"entity_id": "light.desk", "state": "off", "attributes": {}}]}).to_string())).unwrap();
            let ev = |id: &str, new: Value| json!({"id": sub["id"], "type": "event", "event": {"event_type": "state_changed", "data": {"entity_id": id, "old_state": null, "new_state": new}}});
            ws.send(Message::text(ev("light.desk", json!({"entity_id": "light.desk", "state": "on", "attributes": {"brightness": 255}})).to_string())).unwrap();
            ws.send(Message::text(ev("light.gone", Value::Null).to_string())).unwrap();
            let _ = ws.close(None);
            let _ = ws.flush();
            got
        });
        (url, h)
    }

    #[test]
    fn streams_states_from_a_mock() {
        let (url, server) = mock(true);
        let seen = Mutex::new(vec![]);
        let up = Mutex::new(false);
        let stop = run(&Conn::new(&url, "good"), &|u| seen.lock().unwrap().push(match u {
            Update::All(list) => format!("all {}", list.len()),
            Update::Changed(id, s) => format!("{id} {}", s.map_or("removed".into(), |s| s["state"].as_str().unwrap_or("").to_string())),
        }), &|| *up.lock().unwrap() = true);
        assert!(matches!(stop, Stop::Lost(_)), "{stop:?}");
        assert!(*up.lock().unwrap());
        assert_eq!(*seen.lock().unwrap(), vec!["all 1", "light.desk on", "light.gone removed"]);
        let sent = server.join().unwrap();
        assert_eq!(sent[0], json!({"type": "auth", "access_token": "good"}));
        assert_eq!(sent[1]["type"], "subscribe_events");
        assert_eq!(sent[1]["event_type"], "state_changed");
        assert_eq!(sent[2]["type"], "get_states");
    }

    #[test]
    fn refused_token() {
        let (url, server) = mock(false);
        let stop = run(&Conn::new(&url, "bad"), &|_| {}, &|| {});
        assert_eq!(stop, Stop::Refused("Invalid access token or password".into()));
        server.join().unwrap();
    }

    /// wss:// through rustls against a public echo server (network; run with --ignored).
    #[test]
    #[ignore]
    fn tls_handshake() {
        let mut ws = open("wss://echo.websocket.org/").unwrap();
        ws.send(Message::text("hi")).unwrap();
        assert!(ws.read().is_ok());
    }

    #[test]
    fn unreachable() {
        // A port nothing listens on.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(matches!(run(&Conn::new(&format!("http://127.0.0.1:{port}"), "x"), &|_| {}, &|| {}), Stop::Lost(_)));
    }
}
