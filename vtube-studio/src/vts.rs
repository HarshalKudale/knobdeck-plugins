//! VTube Studio's public WebSocket API (Settings ▸ Start API, default `ws://127.0.0.1:8001`).
//!
//! Every message is `{apiName: "VTubeStudioPublicAPI", apiVersion: "1.0", requestID, messageType,
//! data}`; the answer has the same `requestID` and `<Name>Response` (or `APIError {errorID,
//! message}`) as its type. A plugin first asks for a token (`AuthenticationTokenRequest`: VTube
//! Studio shows the user an Allow / Deny popup and answers only once they choose) and then, on each
//! new connection, authenticates with it (`AuthenticationRequest`).
//!
//! One connection is kept open and used for one request at a time. The token request runs on its
//! own connection in a background thread, so no action blocks on the popup.

use serde_json::{json, Value};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

type Socket = WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

pub const PLUGIN_NAME: &str = "KnobDeck";
pub const PLUGIN_DEVELOPER: &str = "KnobDeck";

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub host: String,
    pub port: String,
}

/// Where the token lives: (read, write). The plugin uses the OS credential store; tests a variable.
pub type TokenStore = (fn() -> String, fn(&str));
pub static TOKEN_STORE: OnceLock<TokenStore> = OnceLock::new();

fn token() -> String {
    TOKEN_STORE.get().map(|s| (s.0)()).unwrap_or_default()
}

fn keep_token(t: &str) {
    if let Some(s) = TOKEN_STORE.get() {
        (s.1)(t)
    }
}

static CONN: Mutex<Option<(Endpoint, Socket)>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);
static ASKING: AtomicBool = AtomicBool::new(false);
static DENIED: Mutex<Option<String>> = Mutex::new(None);
const TIMEOUT: Duration = Duration::from_secs(5);
const ALLOW: &str = "Allow KnobDeck in VTube Studio's popup, then try again";

fn envelope(request_id: &str, message_type: &str, data: Value) -> String {
    json!({"apiName": "VTubeStudioPublicAPI", "apiVersion": "1.0", "requestID": request_id, "messageType": message_type, "data": data}).to_string()
}

fn read(ws: &mut Socket, deadline: Instant) -> Result<Value, String> {
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => return t.parse::<Value>().map_err(|e| e.to_string()),
            Ok(Message::Close(_)) => return Err("VTube Studio closed the connection".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if Instant::now() > deadline {
                    return Err("VTube Studio didn't answer".into());
                }
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Send one request on `ws` and wait for its answer's `data`.
fn request(ws: &mut Socket, message_type: &str, data: Value, wait: Duration) -> Result<Value, String> {
    let id = format!("kd{}", NEXT.fetch_add(1, Ordering::Relaxed));
    ws.send(Message::text(envelope(&id, message_type, data))).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + wait;
    loop {
        let m = read(ws, deadline)?;
        if m["requestID"] != id.as_str() {
            continue; // events and stray answers
        }
        if m["messageType"] == "APIError" {
            return Err(m["data"]["message"].as_str().unwrap_or("VTube Studio reported an error").to_string());
        }
        return Ok(m["data"].clone());
    }
}

fn connect(ep: &Endpoint) -> Result<Socket, String> {
    let unreachable = || format!("Can't reach VTube Studio at {}:{}. Start VTube Studio and turn on Settings ▸ Start API", ep.host, ep.port);
    let addr = format!("{}:{}", ep.host, ep.port).to_socket_addrs().map_err(|_| unreachable())?.next().ok_or_else(unreachable)?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|_| unreachable())?;
    stream.set_read_timeout(Some(Duration::from_millis(250))).ok();
    let req = format!("ws://{}:{}/", ep.host, ep.port).into_client_request().map_err(|e| e.to_string())?;
    let (ws, _) = tungstenite::client(req, tungstenite::stream::MaybeTlsStream::Plain(stream)).map_err(|_| unreachable())?;
    Ok(ws)
}

/// Ask VTube Studio for a token in the background (once at a time); the user allows it in a popup.
fn ask_for_token(ep: &Endpoint) {
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let ep = ep.clone();
    std::thread::spawn(move || {
        let result = connect(&ep).and_then(|mut ws| request(&mut ws, "AuthenticationTokenRequest", json!({"pluginName": PLUGIN_NAME, "pluginDeveloper": PLUGIN_DEVELOPER}), Duration::from_secs(120)));
        match result {
            Ok(d) => match d["authenticationToken"].as_str() {
                Some(t) if !t.is_empty() => {
                    keep_token(t);
                    *DENIED.lock().unwrap() = None;
                }
                _ => *DENIED.lock().unwrap() = Some("VTube Studio sent no token".into()),
            },
            Err(e) => *DENIED.lock().unwrap() = Some(format!("VTube Studio didn't allow KnobDeck ({e}). Press again to ask once more")),
        }
        ASKING.store(false, Ordering::SeqCst);
    });
}

fn open(ep: &Endpoint) -> Result<Socket, String> {
    let mut ws = connect(ep)?;
    let t = token();
    if t.is_empty() {
        if let Some(denied) = DENIED.lock().unwrap().take() {
            return Err(denied); // shown once; the next press asks again
        }
        ask_for_token(ep);
        return Err(ALLOW.into());
    }
    let r = request(&mut ws, "AuthenticationRequest", json!({"pluginName": PLUGIN_NAME, "pluginDeveloper": PLUGIN_DEVELOPER, "authenticationToken": t}), TIMEOUT)?;
    if r["authenticated"] != true {
        // Revoked in VTube Studio (Settings ▸ plugins): ask again.
        keep_token("");
        ask_for_token(ep);
        return Err(ALLOW.into());
    }
    Ok(ws)
}

/// One request; returns the answer's `data`.
pub fn call(ep: &Endpoint, message_type: &str, data: Value) -> Result<Value, String> {
    let mut guard = CONN.lock().unwrap();
    for attempt in 0..2 {
        if guard.as_ref().is_none_or(|(e, _)| e != ep) {
            *guard = Some((ep.clone(), open(ep)?));
        }
        let ws = &mut guard.as_mut().unwrap().1;
        match request(ws, message_type, data.clone(), TIMEOUT) {
            Ok(d) => return Ok(d),
            Err(e) if attempt == 0 && (e.contains("closed") || e.contains("didn't answer") || e.contains("IO error") || e.contains("Trying to work with closed")) => *guard = None,
            Err(e) => {
                if e.contains("closed") {
                    *guard = None;
                }
                return Err(e);
            }
        }
    }
    unreachable!()
}

#[cfg(test)]
pub fn reset() {
    *CONN.lock().unwrap() = None;
}

/// Map `pct` (0–100) onto a parameter's range.
pub fn scale(pct: f64, min: f64, max: f64) -> f64 {
    min + (max - min) * pct.clamp(0.0, 100.0) / 100.0
}

/// A parameter value as 0–100 % of its range.
pub fn percent(value: f64, min: f64, max: f64) -> f64 {
    if max <= min {
        0.0
    } else {
        ((value - min) / (max - min) * 100.0).clamp(0.0, 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static TOKEN: Mutex<String> = Mutex::new(String::new());

    #[test]
    fn ranges() {
        assert_eq!(scale(50.0, -30.0, 30.0), 0.0);
        assert_eq!(scale(150.0, 0.0, 1.0), 1.0);
        assert_eq!(percent(0.25, 0.0, 1.0), 25.0);
        assert_eq!(percent(1.0, 1.0, 1.0), 0.0);
    }

    /// A stand-in VTube Studio: hands out a token, checks it, then serves the requests the
    /// actions send, with an unrelated event in between.
    #[test]
    fn talks_to_vtube_studio() {
        TOKEN_STORE.set((|| TOKEN.lock().unwrap().clone(), |t| *TOKEN.lock().unwrap() = t.to_string())).ok();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let (seen_tx, seen_rx) = std::sync::mpsc::channel::<Value>();
        std::thread::spawn(move || {
            for sock in listener.incoming() {
                let seen_tx = seen_tx.clone();
                std::thread::spawn(move || {
                    let mut ws = tungstenite::accept(sock.unwrap()).unwrap();
                    while let Ok(msg) = ws.read() {
                        let Message::Text(t) = msg else { continue };
                        let m: Value = t.parse().unwrap();
                        assert_eq!(m["apiName"], "VTubeStudioPublicAPI");
                        let reply = |ty: &str, data: Value| {
                            json!({"apiName": "VTubeStudioPublicAPI", "apiVersion": "1.0", "timestamp": 1, "requestID": m["requestID"], "messageType": ty, "data": data}).to_string()
                        };
                        let out = match m["messageType"].as_str().unwrap() {
                            "AuthenticationTokenRequest" => reply("AuthenticationTokenResponse", json!({"authenticationToken": "tok-1"})),
                            "AuthenticationRequest" => reply("AuthenticationResponse", json!({"authenticated": m["data"]["authenticationToken"] == "tok-1", "reason": ""})),
                            "HotkeysInCurrentModelRequest" => {
                                ws.send(Message::text(json!({"apiName": "VTubeStudioPublicAPI", "messageType": "TestEvent", "requestID": "x", "data": {}}).to_string())).unwrap();
                                reply(
                                    "HotkeysInCurrentModelResponse",
                                    json!({"modelLoaded": true, "modelName": "Akari", "modelID": "m1", "availableHotkeys": [{"name": "Wave", "type": "TriggerAnimation", "hotkeyID": "h1"}]}),
                                )
                            }
                            "HotkeyTriggerRequest" => reply("HotkeyTriggerResponse", json!({"hotkeyID": m["data"]["hotkeyID"]})),
                            _ => reply("APIError", json!({"errorID": 2, "message": "Unknown request"})),
                        };
                        seen_tx.send(m).unwrap();
                        ws.send(Message::text(out)).unwrap();
                    }
                });
            }
        });
        let ep = Endpoint { host: "127.0.0.1".into(), port };
        // No token yet: the first call asks for one in the background.
        assert_eq!(call(&ep, "HotkeysInCurrentModelRequest", json!({})).unwrap_err(), ALLOW);
        for _ in 0..100 {
            if !TOKEN.lock().unwrap().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(*TOKEN.lock().unwrap(), "tok-1");
        let hk = call(&ep, "HotkeysInCurrentModelRequest", json!({})).unwrap();
        assert_eq!(hk["availableHotkeys"][0]["hotkeyID"], "h1");
        assert_eq!(call(&ep, "HotkeyTriggerRequest", json!({"hotkeyID": "h1"})).unwrap()["hotkeyID"], "h1");
        assert_eq!(call(&ep, "Nope", json!({})).unwrap_err(), "Unknown request");
        reset();
        let types: Vec<String> = seen_rx.try_iter().map(|m| m["messageType"].as_str().unwrap().to_string()).collect();
        assert_eq!(types, ["AuthenticationTokenRequest", "AuthenticationRequest", "HotkeysInCurrentModelRequest", "HotkeyTriggerRequest", "Nope"]);
    }
}
