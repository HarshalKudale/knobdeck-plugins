//! The link to the KnobDeck browser extension (Knobdeck/knobdeck-browser-companion).
//!
//! The plugin listens on `127.0.0.1` for WebSocket connections from the extension (a browser
//! extension can dial out but not listen). Each connected browser says who it is (`hello`),
//! then the plugin sends it one request at a time per action and waits for the reply.
//! The wire format is `knobdeck-companion/PROTOCOL.md`.
//!
//! Only extensions may connect: the handshake is refused unless its `Origin` is a
//! `chrome-extension://` / `moz-extension://` one, so a web page can't drive the browser.

use knobdeck_sdk::{env_or, json, log, Value};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::{accept_hdr, http, Error as WsError, Message};

pub const DEFAULT_PORT: &str = "32147";
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// The extension pings every 20 s; a connection silent for this long is gone.
const SILENCE_LIMIT: Duration = Duration::from_secs(75);

/// A browser that is connected right now.
#[derive(Clone)]
pub struct Info {
    pub id: String,
    /// `chrome`, `msedge`, `brave`, `firefox`, `zen`, or what the extension made of an unknown one.
    pub key: String,
    pub label: String,
}

pub struct Item {
    pub title: String,
    pub url: String,
}

pub enum CallError {
    /// The browser has no extension API for this action; the caller may fall back.
    Unsupported,
    Failed(String),
}

type Pending = Arc<Mutex<HashMap<u64, Sender<Value>>>>;

struct Conn {
    info: Info,
    generation: u64,
    outbox: Sender<String>,
    pending: Pending,
}

static CONNS: Mutex<Vec<Conn>> = Mutex::new(Vec::new());
static COUNTER: AtomicU64 = AtomicU64::new(1);

pub fn port() -> String {
    env_or("BROWSER_PORT", DEFAULT_PORT)
}

/// Start accepting extension connections (once, at plugin start).
pub fn serve() {
    let port = port();
    let listener = match TcpListener::bind(format!("127.0.0.1:{port}")) {
        Ok(l) => l,
        Err(e) => {
            log(format!("browser extension link: cannot listen on 127.0.0.1:{port}: {e}"));
            return;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let port = port.clone();
            std::thread::spawn(move || {
                let _ = connection(stream, &port);
            });
        }
    });
}

/// The connected browsers, oldest first.
pub fn connected() -> Vec<Info> {
    CONNS.lock().unwrap().iter().map(|c| c.info.clone()).collect()
}

pub fn info(id: &str) -> Option<Info> {
    CONNS.lock().unwrap().iter().find(|c| c.info.id == id).map(|c| c.info.clone())
}

/// Run `action` in the browser `id` and wait for its answer.
pub fn call(id: &str, action: &str, params: Value) -> Result<Value, CallError> {
    let (outbox, pending) = {
        let conns = CONNS.lock().unwrap();
        let c = conns.iter().find(|c| c.info.id == id).ok_or_else(|| CallError::Failed("That browser is not connected. Open it and check the KnobDeck extension is installed and enabled.".into()))?;
        (c.outbox.clone(), c.pending.clone())
    };
    let request_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    pending.lock().unwrap().insert(request_id, tx);
    let sent = outbox.send(json!({"id": request_id, "action": action, "params": params}).to_string());
    let reply = if sent.is_ok() { rx.recv_timeout(CALL_TIMEOUT).ok() } else { None };
    pending.lock().unwrap().remove(&request_id);
    let reply = reply.ok_or_else(|| CallError::Failed("The browser did not answer. Is it still running?".into()))?;
    if reply["ok"].as_bool() == Some(true) {
        return Ok(reply["result"].clone());
    }
    if reply["unsupported"].as_bool() == Some(true) {
        return Err(CallError::Unsupported);
    }
    Err(CallError::Failed(reply["error"].as_str().unwrap_or("The browser refused the action").to_string()))
}

/// `[{title, url}]` from an extension answer.
pub fn items(result: &Value) -> Vec<Item> {
    result["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|i| {
            let url = i["url"].as_str().filter(|u| !u.is_empty())?;
            let title = i["title"].as_str().filter(|t| !t.is_empty()).unwrap_or(url);
            Some(Item { title: title.into(), url: url.into() })
        })
        .collect()
}

fn refuse(status: u16, why: &str) -> ErrorResponse {
    http::Response::builder().status(status).body(Some(why.to_string())).unwrap_or_else(|_| http::Response::new(None))
}

fn is_extension_origin(origin: &str) -> bool {
    origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://")
}

fn is_own_host(host: &str, port: &str) -> bool {
    [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")].iter().any(|h| h == host)
}

fn short(v: &Value, max: usize) -> String {
    v.as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(max).collect()
}

fn connection(stream: TcpStream, port: &str) -> Result<(), WsError> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let port = port.to_string();
    let mut ws = accept_hdr(stream, move |req: &Request, resp: Response| {
        let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
        if !is_extension_origin(header("origin")) || !is_own_host(header("host"), &port) {
            return Err(refuse(403, "Only the KnobDeck browser extension may connect."));
        }
        Ok(resp)
    })
    .map_err(|e| match e {
        tungstenite::HandshakeError::Failure(e) => e,
        tungstenite::HandshakeError::Interrupted(_) => WsError::Io(ErrorKind::TimedOut.into()),
    })?;

    // The first message must be `hello` and arrive promptly.
    let hello = loop {
        match ws.read()? {
            Message::Text(t) => break serde_json_value(&t),
            Message::Close(_) => return Ok(()),
            _ => {}
        }
    };
    let id = short(&hello["id"], 64);
    if hello["type"] != "hello" || id.is_empty() {
        let _ = ws.close(None);
        return Ok(());
    }
    let key = short(&hello["browser"], 32);
    let label = Some(short(&hello["name"], 48)).filter(|l| !l.is_empty()).unwrap_or_else(|| key.clone());
    ws.get_ref().set_read_timeout(Some(Duration::from_millis(25)))?;

    let (outbox, inbox) = mpsc::channel::<String>();
    let pending: Pending = Arc::default();
    let generation = COUNTER.fetch_add(1, Ordering::SeqCst);
    {
        let mut conns = CONNS.lock().unwrap();
        conns.retain(|c| c.info.id != id); // a reconnect replaces its stale entry (its thread then ends)
        conns.push(Conn { info: Info { id: id.clone(), key, label }, generation, outbox, pending: pending.clone() });
    }
    ws.send(Message::text(json!({"type": "ready", "protocol": 1}).to_string()))?;

    let mut last_heard = Instant::now();
    let result = (|| -> Result<(), WsError> {
        loop {
            loop {
                match inbox.try_recv() {
                    Ok(text) => ws.send(Message::text(text))?,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(()), // replaced by a newer connection
                }
            }
            match ws.read() {
                Ok(Message::Text(t)) => {
                    last_heard = Instant::now();
                    let msg = serde_json_value(&t);
                    if msg["type"] == "ping" {
                        ws.send(Message::text(json!({"type": "pong"}).to_string()))?;
                    } else if let Some(reply_to) = msg["id"].as_u64() {
                        if let Some(tx) = pending.lock().unwrap().remove(&reply_to) {
                            let _ = tx.send(msg);
                        }
                    }
                }
                Ok(Message::Close(_)) => return Ok(()),
                Ok(_) => last_heard = Instant::now(),
                Err(WsError::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if last_heard.elapsed() > SILENCE_LIMIT {
                        return Ok(());
                    }
                }
                Err(e) => return Err(e),
            }
        }
    })();
    let mut conns = CONNS.lock().unwrap();
    conns.retain(|c| c.generation != generation);
    result
}

fn serde_json_value(text: &str) -> Value {
    text.parse::<Value>().unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_extensions_on_our_own_host_may_connect() {
        assert!(is_extension_origin("chrome-extension://abcdefghijklmnop"));
        assert!(is_extension_origin("moz-extension://1234-5678"));
        assert!(!is_extension_origin("https://evil.example"));
        assert!(!is_extension_origin(""));
        assert!(is_own_host("127.0.0.1:32147", "32147"));
        assert!(is_own_host("localhost:32147", "32147"));
        assert!(!is_own_host("evil.example:32147", "32147"));
        assert!(!is_own_host("127.0.0.1:1", "32147"));
    }
}
