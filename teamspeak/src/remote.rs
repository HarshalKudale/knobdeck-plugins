//! TeamSpeak 5 / 6 "Remote Apps" WebSocket (Settings ▸ Remote Apps, default `ws://127.0.0.1:5899`).
//!
//! The client sends `{"type": "auth", "payload": {identifier, version, name, description,
//! content: {apiKey}}}`. With an empty `apiKey` TeamSpeak asks the user to allow the app and then
//! answers `{"type": "auth", "payload": {apiKey, connections: [...]}}`; the key is kept for the next
//! connection. After that TeamSpeak pushes events (`clientSelfPropertyUpdated`, `clientMoved`,
//! `talkStatusChanged`, `connectStatusChanged`, `channels`, ...), which this module folds into a
//! [`Model`]. The only request an app can send is `buttonPress {button, state}`: a named button
//! that the user binds to a TeamSpeak action in Settings ▸ Key Bindings, the way the TeamSpeak
//! Stream Deck plugins work. There is no request that mutes or sets away directly.
//!
//! One background thread owns the socket: it reconnects every few seconds, folds events into the
//! model, and sends queued button presses.

use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, Once};
use std::time::Duration;
use tungstenite::client::IntoClientRequest;
use tungstenite::{Message, WebSocket};

pub const KEY: &str = "TEAMSPEAK_REMOTE_APPS_KEY";
const IDENTIFIER: &str = "app.knobdeck.teamspeak";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Client {
    pub nick: String,
    pub channel: i64,
    pub talking: bool,
    pub input_muted: bool,
    pub output_muted: bool,
    pub away: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Conn {
    /// Our own client id on this server; 0 until the connection is established.
    pub client_id: i64,
    pub channels: HashMap<i64, String>,
    pub clients: HashMap<i64, Client>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phase {
    #[default]
    Offline,
    /// Connected, `auth` sent, waiting for the user to allow KnobDeck in TeamSpeak.
    Waiting,
    Ready,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub phase: Phase,
    pub conns: BTreeMap<i64, Conn>,
    pub error: Option<String>,
}

fn id(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => s == "1" || s == "true",
        _ => false,
    }
}

fn merge_props(c: &mut Client, p: &Value) {
    if let Some(n) = p["nickname"].as_str() {
        c.nick = n.to_string();
    }
    for (key, field) in [("inputMuted", &mut c.input_muted), ("outputMuted", &mut c.output_muted), ("away", &mut c.away)] {
        if !p[key].is_null() {
            *field = truthy(&p[key]);
        }
    }
    if !p["flagTalking"].is_null() {
        c.talking = truthy(&p["flagTalking"]);
    }
}

fn add_channels(conn: &mut Conn, info: &Value) {
    let mut add = |ch: &Value| {
        if let Some(name) = ch["properties"]["name"].as_str() {
            conn.channels.insert(id(&ch["id"]), name.to_string());
        }
    };
    info["rootChannels"].as_array().into_iter().flatten().for_each(&mut add);
    if let Some(subs) = info["subChannels"].as_object() {
        subs.values().filter_map(|v| v.as_array()).flatten().for_each(&mut add);
    }
}

fn add_connection(model: &mut Model, c: &Value) {
    let mut conn = Conn { client_id: id(&c["clientId"]), ..Default::default() };
    add_channels(&mut conn, &c["channelInfos"]);
    for ci in c["clientInfos"].as_array().into_iter().flatten() {
        let mut client = Client { channel: id(&ci["channelId"]), ..Default::default() };
        merge_props(&mut client, &ci["properties"]);
        conn.clients.insert(id(&ci["id"]), client);
    }
    model.conns.insert(id(&c["id"]), conn);
}

impl Model {
    /// Fold one message from TeamSpeak into the model. Returns the API key of an `auth` answer.
    pub fn apply(&mut self, m: &Value) -> Option<String> {
        let p = &m["payload"];
        let conn_id = id(&p["connectionId"]);
        match m["type"].as_str().unwrap_or("") {
            "auth" => {
                self.phase = Phase::Ready;
                self.error = None;
                self.conns.clear();
                p["connections"].as_array().into_iter().flatten().for_each(|c| add_connection(self, c));
                return p["apiKey"].as_str().map(str::to_string);
            }
            "connectStatusChanged" => {
                if id(&p["status"]) == 0 {
                    self.conns.remove(&conn_id);
                } else {
                    let conn = self.conns.entry(conn_id).or_default();
                    let me = id(&p["info"]["clientId"]);
                    if me != 0 {
                        conn.client_id = me;
                    }
                }
            }
            "channels" => add_channels(self.conns.entry(conn_id).or_default(), &p["info"]),
            "channelCreated" | "channelEdited" | "channelPropertiesUpdated" => {
                if let Some(name) = p["properties"]["name"].as_str() {
                    self.conns.entry(conn_id).or_default().channels.insert(id(&p["channelId"]), name.to_string());
                }
            }
            "channelDeleted" => {
                if let Some(c) = self.conns.get_mut(&conn_id) {
                    c.channels.remove(&id(&p["channelId"]));
                }
            }
            "clientMoved" => {
                let conn = self.conns.entry(conn_id).or_default();
                let (client, to) = (id(&p["clientId"]), id(&p["newChannelId"]));
                if to == 0 {
                    conn.clients.remove(&client);
                } else {
                    let c = conn.clients.entry(client).or_default();
                    c.channel = to;
                    if p["properties"].is_object() {
                        merge_props(c, &p["properties"]);
                    }
                }
            }
            "clientPropertiesUpdated" => {
                if let Some(c) = self.conns.get_mut(&conn_id).and_then(|c| c.clients.get_mut(&id(&p["clientId"]))) {
                    merge_props(c, &p["properties"]);
                }
            }
            "talkStatusChanged" => {
                if let Some(c) = self.conns.get_mut(&conn_id).and_then(|c| c.clients.get_mut(&id(&p["clientId"]))) {
                    c.talking = id(&p["status"]) == 1;
                }
            }
            "clientSelfPropertyUpdated" => {
                if let Some(conn) = self.conns.get_mut(&conn_id) {
                    let me = conn.client_id;
                    let c = conn.clients.entry(me).or_default();
                    let v = truthy(&p["newValue"]);
                    match p["flag"].as_str().unwrap_or("") {
                        "inputMuted" => c.input_muted = v,
                        "outputMuted" => c.output_muted = v,
                        "away" => c.away = v,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// The server tab we report on: the first one we are connected to.
    pub fn active(&self) -> Option<(&Conn, &Client)> {
        self.conns.values().filter(|c| c.client_id != 0).find_map(|c| Some((c, c.clients.get(&c.client_id)?)))
    }
}

pub static MODEL: Mutex<Model> = Mutex::new(Model { phase: Phase::Offline, conns: BTreeMap::new(), error: None });
static START: Once = Once::new();
static TX: Mutex<Option<Sender<String>>> = Mutex::new(None);

/// Start the background connection (once). Called by the first action that needs TeamSpeak, so
/// the "allow this app" prompt only shows once KnobDeck has TeamSpeak keys.
pub fn ensure_started(host: String, port: String) {
    START.call_once(|| {
        let (tx, rx) = channel();
        *TX.lock().unwrap() = Some(tx);
        std::thread::spawn(move || loop {
            let key = knobdeck_sdk::secrets::env_or_stored(KEY);
            let result = session(&host, &port, &key, &rx, &|k: Option<&str>| {
                // None: TeamSpeak no longer knows the key (removed under Remote Apps), so ask again.
                if let Err(e) = knobdeck_sdk::secrets::set(KEY, k.unwrap_or("")) {
                    knobdeck_sdk::log(format!("couldn't keep TeamSpeak's key: {e}"));
                }
            });
            {
                let mut m = MODEL.lock().unwrap();
                m.phase = Phase::Offline;
                m.conns.clear();
                m.error = result.err();
            }
            while rx.try_recv().is_ok() {} // presses made while offline are dropped
            std::thread::sleep(Duration::from_secs(3));
        });
    });
}

/// Queue a button press (or release) for TeamSpeak.
pub fn press(button: &str, state: bool) -> Result<(), String> {
    let phase = MODEL.lock().unwrap().phase.clone();
    match phase {
        Phase::Ready => {}
        Phase::Waiting => return Err("Allow KnobDeck in TeamSpeak first (it asks in a notification, or under Settings ▸ Remote Apps)".into()),
        Phase::Offline => return Err(offline_message()),
    }
    let msg = json!({"type": "buttonPress", "payload": {"button": button, "state": state}}).to_string();
    TX.lock().unwrap().as_ref().ok_or("TeamSpeak isn't connected")?.send(msg).map_err(|e| e.to_string())
}

pub fn offline_message() -> String {
    MODEL.lock().unwrap().error.clone().unwrap_or_else(|| "Connecting to TeamSpeak…".into())
}

fn auth_message(key: &str) -> String {
    json!({"type": "auth", "payload": {
        "identifier": IDENTIFIER,
        "version": env!("CARGO_PKG_VERSION"),
        "name": "KnobDeck",
        "description": "Mute, away, push-to-talk and channel readouts on KnobDeck keys and dials",
        "content": {"apiKey": key},
    }})
    .to_string()
}

type Socket = WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

/// One connection, until it drops. `keep` stores a new key (`None` forgets the stored one).
fn session(host: &str, port: &str, key: &str, rx: &Receiver<String>, keep: &dyn Fn(Option<&str>)) -> Result<(), String> {
    let unreachable = || format!("Can't reach TeamSpeak at {host}:{port}. Start TeamSpeak and turn on Settings ▸ Remote Apps");
    let addr = format!("{host}:{port}").to_socket_addrs().map_err(|_| unreachable())?.next().ok_or_else(unreachable)?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|_| unreachable())?;
    let req = format!("ws://{host}:{port}/").into_client_request().map_err(|e| e.to_string())?;
    let (mut ws, _): (Socket, _) = tungstenite::client(req, tungstenite::stream::MaybeTlsStream::Plain(stream)).map_err(|_| unreachable())?;
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_ref() {
        s.set_read_timeout(Some(Duration::from_millis(50))).ok();
    }
    ws.send(Message::text(auth_message(key))).map_err(|e| e.to_string())?;
    MODEL.lock().unwrap().phase = Phase::Waiting;
    loop {
        while let Ok(out) = rx.try_recv() {
            ws.send(Message::text(out)).map_err(|e| e.to_string())?;
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                let Ok(m) = t.parse::<Value>() else { continue };
                let refused = m["status"]["code"].as_i64().is_some_and(|c| c != 0);
                let mut model = MODEL.lock().unwrap();
                if refused && model.phase != Phase::Ready {
                    keep(None);
                    return Err(format!("TeamSpeak refused KnobDeck ({}). Allow it again when TeamSpeak asks", m["status"]["message"].as_str().unwrap_or("not allowed")));
                }
                if let Some(new_key) = model.apply(&m) {
                    drop(model);
                    if !new_key.is_empty() && new_key != key {
                        keep(Some(&new_key));
                    }
                }
            }
            Ok(Message::Close(_)) => return Err("TeamSpeak closed the connection".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(format!("TeamSpeak connection lost: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shapes as TeamSpeak 5 sends them (from the Stream Deck TeamSpeak 5 plugin and the
    /// ts-remote-app / react-teamspeak-remote-app-api type definitions).
    fn auth() -> Value {
        json!({"type": "auth", "payload": {"apiKey": "k-123", "connections": [{
            "id": 1, "clientId": 7, "status": 4, "properties": {},
            "channelInfos": {
                "rootChannels": [{"id": "1", "order": "0", "parentId": "0", "properties": {"name": "Lobby"}}],
                "subChannels": {"1": [{"id": "5", "order": "0", "parentId": "1", "properties": {"name": "Raid"}}]}
            },
            "clientInfos": [
                {"id": 7, "channelId": 1, "properties": {"nickname": "me", "inputMuted": 0, "outputMuted": 1, "away": 0, "flagTalking": 0}},
                {"id": 9, "channelId": 1, "properties": {"nickname": "Alice", "inputMuted": 0, "outputMuted": 0, "away": 0, "flagTalking": 0}}
            ]
        }]}})
    }

    #[test]
    fn folds_events() {
        let mut m = Model::default();
        assert_eq!(m.apply(&auth()).as_deref(), Some("k-123"));
        let (conn, me) = m.active().unwrap();
        assert_eq!(conn.channels[&me.channel], "Lobby");
        assert!(me.output_muted && !me.input_muted);

        m.apply(&json!({"type": "clientSelfPropertyUpdated", "payload": {"connectionId": 1, "flag": "inputMuted", "newValue": true, "oldValue": false}}));
        m.apply(&json!({"type": "clientMoved", "payload": {"connectionId": 1, "clientId": 7, "newChannelId": 5, "oldChannelId": 1, "type": 0, "visibility": 0, "properties": null}}));
        m.apply(&json!({"type": "talkStatusChanged", "payload": {"connectionId": 1, "clientId": 9, "status": 1, "isWhisper": false}}));
        let (conn, me) = m.active().unwrap();
        assert!(me.input_muted);
        assert_eq!(conn.channels[&me.channel], "Raid");
        assert!(conn.clients[&9].talking);

        m.apply(&json!({"type": "clientMoved", "payload": {"connectionId": 1, "clientId": 9, "newChannelId": 0, "oldChannelId": 1, "properties": null}}));
        assert!(!m.active().unwrap().0.clients.contains_key(&9));
        m.apply(&json!({"type": "connectStatusChanged", "payload": {"connectionId": 1, "status": 0, "error": 0, "info": {}}}));
        assert!(m.active().is_none());
    }

    /// A stand-in TeamSpeak: answers `auth` only once, sends an event, and records button presses.
    #[test]
    fn talks_to_a_server() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let (seen_tx, seen_rx) = channel();
        std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(sock).unwrap();
            while let Ok(msg) = ws.read() {
                let Message::Text(t) = msg else { continue };
                let m: Value = t.parse().unwrap();
                if m["type"] == "auth" {
                    assert_eq!(m["payload"]["identifier"], IDENTIFIER);
                    ws.send(Message::text(auth().to_string())).unwrap();
                    ws.send(Message::text(json!({"type": "clientSelfPropertyUpdated", "payload": {"connectionId": 1, "flag": "away", "newValue": true, "oldValue": false}}).to_string())).unwrap();
                }
                seen_tx.send(m).unwrap();
            }
        });
        let (rx_dummy_tx, rx) = channel();
        std::thread::spawn(move || {
            let _keep = rx_dummy_tx;
            let _ = session("127.0.0.1", &port, "", &rx, &|k| assert_eq!(k, Some("k-123")));
        });
        let auth_seen = seen_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(auth_seen["type"], "auth");
        for _ in 0..100 {
            if MODEL.lock().unwrap().active().is_some_and(|(_, me)| me.away) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the model never saw the away event");
    }
}
