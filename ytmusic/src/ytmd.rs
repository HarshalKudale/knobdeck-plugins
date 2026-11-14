//! YouTube Music Desktop App (YTMDesktop 2.x) Companion Server: REST at
//! `http://<host>:9863/api/v1` with a token the user approves in the app, and the Socket.IO
//! realtime feed at `/api/v1/realtime` for state updates. Shared by the plugin and its setup.

use knobdeck_sdk::{json, Error, Result, Value};
use std::time::Duration;

pub const APP_ID: &str = "knobdeck";
pub const APP_NAME: &str = "KnobDeck";
pub const DEFAULT_PORT: &str = "9863";

pub const NOT_RUNNING: &str = "YouTube Music Desktop isn't running, or its companion server is off (Settings ▸ Integrations ▸ Companion server)";
pub const UNAUTHORIZED: &str = "YouTube Music Desktop no longer accepts KnobDeck's token. Run setup again";

#[derive(Clone)]
pub struct Conn {
    pub host: String,
    pub port: String,
    pub token: String,
}

impl Conn {
    pub fn from_env() -> Result<Conn> {
        let token = knobdeck_sdk::secrets::env_or_stored("YTMD_TOKEN");
        if token.is_empty() {
            return Err(Error::msg("Run setup to connect to YouTube Music Desktop"));
        }
        Ok(Conn { host: host_or_default(&knobdeck_sdk::env_or("YTMD_HOST", "")), port: port_or_default(&knobdeck_sdk::env_or("YTMD_PORT", "")), token })
    }

    pub fn base(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    fn req(&self, method: &str, path: &str, timeout: u64) -> ureq::Request {
        let r = ureq::request(method, &format!("{}/api/v1/{path}", self.base())).timeout(Duration::from_secs(timeout));
        if self.token.is_empty() {
            r
        } else {
            // The companion server takes the bare token, no "Bearer".
            r.set("Authorization", &self.token)
        }
    }

    pub fn get(&self, path: &str) -> Result<Value> {
        reply(self.req("GET", path, if path == "playlists" { 35 } else { 4 }).call())
    }

    pub fn command(&self, command: &str, data: Option<Value>) -> Result<()> {
        let mut body = json!({"command": command});
        if let Some(d) = data {
            body["data"] = d;
        }
        match self.req("POST", "command", 4).send_json(body) {
            // A command replies 204 with no body.
            Ok(_) => Ok(()),
            Err(e) => reply(Err(e)).map(|_| ()),
        }
    }

    /// Step 1 of sign-in: a code the app shows in its approval dialog.
    pub fn request_code(&self, version: &str) -> Result<String> {
        let v = reply(self.req("POST", "auth/requestcode", 5).send_json(json!({"appId": APP_ID, "appName": APP_NAME, "appVersion": version})))?;
        v["code"].as_str().map(String::from).ok_or_else(|| Error::msg("YouTube Music Desktop sent no code"))
    }

    /// Step 2: waits (up to 30 s) for the user to click Allow in the app.
    pub fn request_token(&self, code: &str) -> Result<String> {
        let v = reply(self.req("POST", "auth/request", 40).send_json(json!({"appId": APP_ID, "code": code})))?;
        v["token"].as_str().map(String::from).ok_or_else(|| Error::msg("KnobDeck was not allowed in YouTube Music Desktop"))
    }
}

pub fn host_or_default(h: &str) -> String {
    // IPv4 on purpose: the server is unreachable over IPv6 "localhost" on Windows.
    let h = h.trim();
    if h.is_empty() || h.eq_ignore_ascii_case("localhost") {
        "127.0.0.1".into()
    } else {
        h.to_string()
    }
}

pub fn port_or_default(p: &str) -> String {
    let p = p.trim();
    if p.is_empty() {
        DEFAULT_PORT.into()
    } else {
        p.to_string()
    }
}

fn reply(r: std::result::Result<ureq::Response, ureq::Error>) -> Result<Value> {
    match r {
        Ok(resp) => {
            let text = resp.into_string().unwrap_or_default();
            if text.trim().is_empty() {
                Ok(Value::Null)
            } else {
                serde_json::from_str(&text).map_err(|e| Error::msg(format!("YouTube Music Desktop sent an invalid reply: {e}")))
            }
        }
        Err(ureq::Error::Status(401, _)) => Err(Error::msg(UNAUTHORIZED)),
        Err(ureq::Error::Status(403, resp)) => {
            let m = server_message(resp);
            knobdeck_sdk::log(format!("403: {m}"));
            Err(Error::msg("YouTube Music Desktop refused. Turn on Settings ▸ Integrations ▸ Enable companion authorization, then run setup again"))
        }
        Err(ureq::Error::Status(429, _)) => Err(Error::msg("YouTube Music Desktop is busy (too many requests). Try again in a moment")),
        Err(ureq::Error::Status(code, resp)) => {
            let m = server_message(resp);
            Err(Error::msg(format!("YouTube Music Desktop: {} ({code})", if m.is_empty() { "request failed" } else { &m })))
        }
        Err(e) => {
            knobdeck_sdk::log(format!("request failed: {e}"));
            Err(Error::msg(NOT_RUNNING))
        }
    }
}

fn server_message(resp: ureq::Response) -> String {
    let body = resp.into_string().unwrap_or_default();
    serde_json::from_str::<Value>(&body).ok().and_then(|v| v["message"].as_str().map(String::from)).unwrap_or(body)
}

/// One Socket.IO (Engine.IO v4) packet from the realtime feed, as far as the plugin cares.
#[derive(Debug, PartialEq)]
pub enum Packet {
    /// `2`: the server's ping; answer `3`.
    Ping,
    /// `40…`: connected to the namespace.
    Connected,
    /// `44…`: the namespace refused (bad token).
    Refused(String),
    /// `42["state-update", {...}]`.
    State(Value),
    Other,
}

pub fn parse_packet(text: &str) -> Packet {
    if text == "2" {
        return Packet::Ping;
    }
    if text.starts_with("40") {
        return Packet::Connected;
    }
    if let Some(rest) = text.strip_prefix("44") {
        return Packet::Refused(serde_json::from_str::<Value>(rest).ok().and_then(|v| v["message"].as_str().map(String::from)).unwrap_or_else(|| rest.to_string()));
    }
    if let Some(rest) = text.strip_prefix("42") {
        if let Ok(Value::Array(a)) = serde_json::from_str::<Value>(rest) {
            if a.first().and_then(Value::as_str) == Some("state-update") {
                return Packet::State(a.get(1).cloned().unwrap_or(Value::Null));
            }
        }
    }
    Packet::Other
}

/// Run the realtime feed until it closes: `on_state` gets every state update. Returns why it
/// stopped.
pub fn realtime(conn: &Conn, on_state: &dyn Fn(Value)) -> std::result::Result<(), String> {
    use std::net::TcpStream;
    use tungstenite::Message;
    let url = format!("ws://{}:{}/api/v1/realtime/?EIO=4&transport=websocket", conn.host, conn.port);
    let addr = format!("{}:{}", conn.host, conn.port);
    let stream = TcpStream::connect(&addr).map_err(|e| e.to_string())?;
    let (mut ws, _) = tungstenite::client(url.as_str(), stream).map_err(|e| e.to_string())?;
    // Engine.IO "open" (0{…}) first, then connect to the default namespace with the token.
    let mut joined = false;
    loop {
        let msg = ws.read().map_err(|e| e.to_string())?;
        let Message::Text(text) = msg else {
            if matches!(msg, Message::Close(_)) {
                return Ok(());
            }
            continue;
        };
        if text.starts_with('0') && !joined {
            ws.send(Message::Text(format!("40{}", json!({"token": conn.token})))).map_err(|e| e.to_string())?;
            joined = true;
            continue;
        }
        match parse_packet(&text) {
            Packet::Ping => ws.send(Message::Text("3".into())).map_err(|e| e.to_string())?,
            Packet::Refused(m) => return Err(format!("refused: {m}")),
            Packet::State(s) => on_state(s),
            Packet::Connected | Packet::Other => {}
        }
    }
}

// ---------- State helpers ----------

pub fn playing(s: &Value) -> bool {
    s["player"]["trackState"] == 1
}

pub fn volume(s: &Value) -> f64 {
    s["player"]["volume"].as_f64().unwrap_or(0.0).clamp(0.0, 100.0)
}

/// 0 none, 1 all, 2 one (-1 / missing: unknown).
pub fn repeat_mode(s: &Value) -> i64 {
    s["player"]["queue"]["repeatMode"].as_i64().unwrap_or(-1)
}

/// The like status: 0 dislike, 1 indifferent, 2 like.
pub fn like_status(s: &Value) -> i64 {
    s["video"]["likeStatus"].as_i64().unwrap_or(1)
}

pub fn title(s: &Value) -> String {
    s["video"]["title"].as_str().unwrap_or("").to_string()
}

pub fn author(s: &Value) -> String {
    s["video"]["author"].as_str().unwrap_or("").to_string()
}

/// The smallest thumbnail at least 60 px wide (key-sized), else any.
pub fn thumbnail(s: &Value) -> Option<String> {
    let list = s["video"]["thumbnails"].as_array()?;
    let mut sorted: Vec<&Value> = list.iter().collect();
    sorted.sort_by_key(|t| t["width"].as_i64().unwrap_or(0));
    sorted.iter().find(|t| t["width"].as_i64().unwrap_or(0) >= 60).or(sorted.last()).and_then(|t| t["url"].as_str()).map(String::from)
}

/// Queue items after the playing one (index into items, title, author).
pub fn up_next(s: &Value) -> Vec<(usize, String, String)> {
    let q = &s["player"]["queue"];
    let items: Vec<Value> = q["items"].as_array().cloned().unwrap_or_default().into_iter().chain(q["automixItems"].as_array().cloned().unwrap_or_default()).collect();
    let current = q["selectedItemIndex"].as_i64().unwrap_or(-1);
    items.iter().enumerate().filter(|(i, _)| *i as i64 > current).map(|(i, it)| (i, it["title"].as_str().unwrap_or("").to_string(), it["author"].as_str().unwrap_or("").to_string())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packets() {
        assert_eq!(parse_packet("2"), Packet::Ping);
        assert_eq!(parse_packet("40{\"sid\":\"x\"}"), Packet::Connected);
        assert_eq!(parse_packet("44{\"message\":\"bad token\"}"), Packet::Refused("bad token".into()));
        assert_eq!(parse_packet("42[\"state-update\",{\"player\":{\"trackState\":1}}]"), Packet::State(json!({"player": {"trackState": 1}})));
        assert_eq!(parse_packet("42[\"playlist-created\",{}]"), Packet::Other);
    }

    #[test]
    fn state() {
        let s = json!({
            "player": {"trackState": 1, "volume": 40, "queue": {"repeatMode": 2, "selectedItemIndex": 0,
                "items": [{"title": "A", "author": "x"}, {"title": "B", "author": "y"}], "automixItems": [{"title": "C", "author": "z"}]}},
            "video": {"title": "A", "author": "x", "likeStatus": 2, "thumbnails": [{"url": "big", "width": 544}, {"url": "small", "width": 60}, {"url": "tiny", "width": 32}]}
        });
        assert!(playing(&s));
        assert_eq!(volume(&s), 40.0);
        assert_eq!(repeat_mode(&s), 2);
        assert_eq!(like_status(&s), 2);
        assert_eq!(thumbnail(&s).as_deref(), Some("small"));
        assert_eq!(up_next(&s), vec![(1, "B".into(), "y".into()), (2, "C".into(), "z".into())]);
        assert_eq!(host_or_default("localhost"), "127.0.0.1");
    }
}
