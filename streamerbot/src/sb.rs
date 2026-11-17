//! Streamer.bot's WebSocket server (Servers/Clients ▸ WebSocket Server, default
//! `ws://127.0.0.1:8080/`): one connection, kept open, one request at a time.
//!
//! On connect the server sends `Hello`; when authentication is on it carries a salt and a challenge
//! and the client answers `Authenticate` with base64(sha256(base64(sha256(password + salt)) +
//! challenge)). Requests carry an `id`; the answer has the same `id` and `status` ok / error.

use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

type Socket = WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub host: String,
    pub port: String,
    pub password: String,
}

static CONN: Mutex<Option<(Endpoint, Socket)>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);
const TIMEOUT: Duration = Duration::from_secs(5);

fn b64_sha256(data: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(data.as_bytes()))
}

/// The `Authenticate` answer to a `Hello` that asks for one.
pub fn auth_answer(password: &str, salt: &str, challenge: &str) -> String {
    b64_sha256(&format!("{}{challenge}", b64_sha256(&format!("{password}{salt}"))))
}

fn text(ws: &mut Socket) -> Result<Value, String> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => return t.parse::<Value>().map_err(|e| e.to_string()),
            Ok(Message::Close(_)) => return Err("Streamer.bot closed the connection".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if Instant::now() > deadline {
                    return Err("Streamer.bot didn't answer".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn open(ep: &Endpoint) -> Result<Socket, String> {
    let unreachable = || format!("Can't reach Streamer.bot at {}:{}. Start Streamer.bot and turn on its WebSocket Server (Servers/Clients ▸ WebSocket Server)", ep.host, ep.port);
    let addr = format!("{}:{}", ep.host, ep.port).to_socket_addrs().map_err(|_| unreachable())?.next().ok_or_else(unreachable)?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|_| unreachable())?;
    stream.set_read_timeout(Some(TIMEOUT)).ok();
    let req = format!("ws://{}:{}/", ep.host, ep.port).into_client_request().map_err(|e| e.to_string())?;
    let (mut ws, _) = tungstenite::client(req, tungstenite::stream::MaybeTlsStream::Plain(stream)).map_err(|_| unreachable())?;
    let hello = text(&mut ws)?;
    if let Some(auth) = hello.get("authentication").filter(|a| a.is_object()) {
        if ep.password.is_empty() {
            return Err("Streamer.bot's WebSocket Server asks for a password: add it in the plugin's setup".into());
        }
        let answer = auth_answer(&ep.password, auth["salt"].as_str().unwrap_or(""), auth["challenge"].as_str().unwrap_or(""));
        ws.send(Message::text(json!({"request": "Authenticate", "id": "auth", "authentication": answer}).to_string())).map_err(|e| e.to_string())?;
        let r = text(&mut ws)?;
        if r["status"] != "ok" {
            return Err("Streamer.bot refused the password: check it in the plugin's setup".into());
        }
    }
    Ok(ws)
}

/// Send one request (fields besides `request` and `id` in `body`) and return the answer.
pub fn call(ep: &Endpoint, request: &str, mut body: Value) -> Result<Value, String> {
    let mut guard = CONN.lock().unwrap();
    for attempt in 0..2 {
        if guard.as_ref().is_none_or(|(e, _)| e != ep) {
            *guard = Some((ep.clone(), open(ep)?));
        }
        let ws = &mut guard.as_mut().unwrap().1;
        let id = format!("kd{}", NEXT.fetch_add(1, Ordering::Relaxed));
        body["request"] = request.into();
        body["id"] = id.clone().into();
        let sent = ws.send(Message::text(body.to_string())).map_err(|e| e.to_string());
        let answer = sent.and_then(|_| loop {
            let m = text(ws)?;
            if m["id"] == id.as_str() {
                break Ok(m);
            }
            // Events (none subscribed) and stray answers are skipped.
        });
        match answer {
            Ok(m) if m["status"] == "error" => return Err(m["error"].as_str().unwrap_or("Streamer.bot reported an error").to_string()),
            Ok(m) => return Ok(m),
            Err(e) if attempt == 0 && !e.starts_with("Can't reach") => *guard = None, // stale connection: reconnect once
            Err(e) => {
                *guard = None;
                return Err(e);
            }
        }
    }
    unreachable!()
}

/// `key=value` pairs, one per line or separated by `;`, as Streamer.bot action arguments.
pub fn parse_args(raw: &str) -> Result<Value, String> {
    let mut out = serde_json::Map::new();
    for part in raw.split(['\n', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').ok_or_else(|| format!("\"{part}\" isn't key=value"))?;
        let k = k.trim();
        if k.is_empty() {
            return Err(format!("\"{part}\" has no key"));
        }
        out.insert(k.to_string(), Value::String(v.trim().to_string()));
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        assert_eq!(parse_args("a=1; b = two\nc=x=y").unwrap(), json!({"a": "1", "b": "two", "c": "x=y"}));
        assert_eq!(parse_args("  ").unwrap(), json!({}));
        assert!(parse_args("nope").is_err());
        assert!(parse_args("=v").is_err());
    }

    /// A stand-in Streamer.bot server with authentication on: checks the answer, then serves
    /// GetActions and DoAction.
    #[test]
    fn talks_to_a_server_with_authentication() {
        // One test: the connection is shared by the process.
        let none = Endpoint { host: "127.0.0.1".into(), port: "1".into(), password: String::new() };
        assert!(call(&none, "GetActions", json!({})).unwrap_err().starts_with("Can't reach Streamer.bot"));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(sock).unwrap();
            ws.send(Message::text(json!({"request": "Hello", "info": {"name": "Streamer.bot"}, "authentication": {"salt": "s4lt", "challenge": "ch4l"}}).to_string())).unwrap();
            let mut seen = vec![];
            while let Ok(Message::Text(t)) = ws.read() {
                let m: Value = t.parse().unwrap();
                seen.push(m.clone());
                let reply = match m["request"].as_str().unwrap() {
                    "Authenticate" => json!({"id": m["id"], "status": if m["authentication"] == auth_answer("pw", "s4lt", "ch4l") { "ok" } else { "error" }}),
                    "GetActions" => json!({"id": m["id"], "status": "ok", "actions": [{"id": "g1", "name": "Shoutout", "group": "Chat", "enabled": true}]}),
                    "DoAction" => {
                        ws.send(Message::text(json!({"event": {"source": "Raw", "type": "Action"}, "data": {}}).to_string())).unwrap();
                        json!({"id": m["id"], "status": "ok"})
                    }
                    _ => json!({"id": m["id"], "status": "error", "error": "Unknown request"}),
                };
                ws.send(Message::text(reply.to_string())).unwrap();
            }
            seen
        });
        let ep = Endpoint { host: "127.0.0.1".into(), port, password: "pw".into() };
        let r = call(&ep, "GetActions", json!({})).unwrap();
        assert_eq!(r["actions"][0]["name"], "Shoutout");
        call(&ep, "DoAction", json!({"action": {"id": "g1"}, "args": {"user": "kd"}})).unwrap();
        assert_eq!(call(&ep, "Nope", json!({})).unwrap_err(), "Unknown request");
        *CONN.lock().unwrap() = None; // closes the socket: the server loop ends
        let seen = server.join().unwrap();
        assert_eq!(seen[0]["request"], "Authenticate");
        assert_eq!(seen[2]["args"], json!({"user": "kd"}));
    }
}
