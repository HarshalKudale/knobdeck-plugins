//! Twitch EventSub over WebSocket: follows, subs, gifts, raids and cheers on the user's own
//! channel, for the Alert Pager. One connection (Twitch allows three per user and app); the
//! subscriptions are created through Helix with the user's sign-in once the welcome names the
//! session. `session_reconnect` moves to the URL Twitch gives, keeping the subscriptions.

use knobdeck_sdk::{env_or, json, log, Result, Value};
use std::collections::VecDeque;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub const URL: &str = "wss://eventsub.wss.twitch.tv/ws?keepalive_timeout_seconds=30";
/// Twitch sends a keepalive at least this often (the `keepalive_timeout_seconds` above).
const KEEPALIVE: Duration = Duration::from_secs(30);
const RECONNECT_DELAYS_S: [u64; 5] = [2, 5, 10, 30, 60];

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Follow,
    Sub,
    Raid,
    Cheer,
}

impl Kind {
    /// The Alert Pager's checkbox for this kind.
    pub fn input(self) -> &'static str {
        match self {
            Kind::Follow => "follows",
            Kind::Sub => "subs",
            Kind::Raid => "raids",
            Kind::Cheer => "cheers",
        }
    }
}

/// The subscriptions the pager makes: (type, version, condition, kind). Follows need
/// `moderator:read:followers`, subs and gifts `channel:read:subscriptions`, cheers `bits:read`;
/// raids need no scope.
pub fn subscriptions(me: &str) -> Vec<(&'static str, &'static str, Value, Kind)> {
    vec![
        ("channel.follow", "2", json!({"broadcaster_user_id": me, "moderator_user_id": me}), Kind::Follow),
        ("channel.subscribe", "1", json!({"broadcaster_user_id": me}), Kind::Sub),
        ("channel.subscription.message", "1", json!({"broadcaster_user_id": me}), Kind::Sub),
        ("channel.subscription.gift", "1", json!({"broadcaster_user_id": me}), Kind::Sub),
        ("channel.raid", "1", json!({"to_broadcaster_user_id": me}), Kind::Raid),
        ("channel.cheer", "1", json!({"broadcaster_user_id": me}), Kind::Cheer),
    ]
}

fn tier(e: &Value) -> String {
    match e["tier"].as_str() {
        Some("2000") => " (Tier 2)".into(),
        Some("3000") => " (Tier 3)".into(),
        _ => String::new(),
    }
}

/// The pager's text for one notification; `None` for events it doesn't show (a gifted sub's
/// recipient: the gift event already says who gave how many).
pub fn describe(sub_type: &str, e: &Value) -> Option<(Kind, String)> {
    let name = |k: &str| if e["is_anonymous"] == true { "Anonymous".to_string() } else { e[k].as_str().unwrap_or("Someone").to_string() };
    Some(match sub_type {
        "channel.follow" => (Kind::Follow, format!("{} followed", name("user_name"))),
        "channel.subscribe" if e["is_gift"] == true => return None,
        "channel.subscribe" => (Kind::Sub, format!("{} subscribed{}", name("user_name"), tier(e))),
        "channel.subscription.message" => (Kind::Sub, format!("{} resubscribed · {} months{}", name("user_name"), e["cumulative_months"].as_i64().unwrap_or(1), tier(e))),
        "channel.subscription.gift" => {
            let n = e["total"].as_i64().unwrap_or(1);
            (Kind::Sub, format!("{} gifted {n} sub{}{}", name("user_name"), if n == 1 { "" } else { "s" }, tier(e)))
        }
        "channel.raid" => (Kind::Raid, format!("{} raided with {}", e["from_broadcaster_user_name"].as_str().unwrap_or("Someone"), e["viewers"].as_i64().unwrap_or(0))),
        "channel.cheer" => (Kind::Cheer, format!("{} cheered {} bits", name("user_name"), e["bits"].as_i64().unwrap_or(0))),
        _ => return None,
    })
}

/// What the pager shows while there is no alert: connection state, missing permissions.
pub static STATUS: Mutex<String> = Mutex::new(String::new());

pub fn status(s: impl Into<String>) {
    *STATUS.lock().unwrap() = s.into();
}

fn open(url: &str) -> std::result::Result<Socket, String> {
    let req = url.into_client_request().map_err(|e| e.to_string())?;
    let host = req.uri().host().ok_or("no host in the EventSub URL")?.to_string();
    let port = req.uri().port_u16().unwrap_or(if req.uri().scheme_str() == Some("ws") { 80 } else { 443 });
    let addr = (host.as_str(), port).to_socket_addrs().map_err(|e| e.to_string())?.next().ok_or("no address")?;
    let tcp = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    // Short reads let the loop notice a missed keepalive and an unplaced pager.
    tcp.set_read_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
    let (ws, _) = tungstenite::client_tls(req, tcp).map_err(|e| e.to_string())?;
    Ok(ws)
}

/// How one session ended.
enum End {
    /// Twitch asked us to move (`session_reconnect`): subscriptions carry over.
    Moved(String),
    /// Connection lost or keepalive missed: start over with new subscriptions.
    Lost(String),
    /// Nobody needs alerts any more.
    Unwanted,
}

/// Read one session until it ends. `subscribe` runs once with the session id when this is a
/// new session (not a move).
fn session(ws: &mut Socket, fresh: bool, seen: &mut VecDeque<String>, wanted: &dyn Fn() -> bool, subscribe: &dyn Fn(&str), on_alert: &dyn Fn(Kind, String)) -> End {
    let mut last = Instant::now();
    loop {
        if !wanted() {
            let _ = ws.close(None);
            return End::Unwanted;
        }
        let text = match ws.read() {
            Ok(Message::Text(t)) => t,
            Ok(Message::Close(f)) => return End::Lost(format!("closed by Twitch ({})", f.map(|f| f.to_string()).unwrap_or_default())),
            Ok(_) => continue,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if last.elapsed() > KEEPALIVE + Duration::from_secs(10) {
                    return End::Lost("no keepalive from Twitch".into());
                }
                continue;
            }
            Err(e) => return End::Lost(e.to_string()),
        };
        last = Instant::now();
        let Ok(msg) = text.parse::<Value>() else { continue };
        let meta = &msg["metadata"];
        match meta["message_type"].as_str().unwrap_or("") {
            "session_welcome" if fresh => subscribe(msg["payload"]["session"]["id"].as_str().unwrap_or("")),
            "session_reconnect" => return End::Moved(msg["payload"]["session"]["reconnect_url"].as_str().unwrap_or(URL).to_string()),
            "notification" => {
                // Twitch may deliver a message twice.
                let id = meta["message_id"].as_str().unwrap_or("").to_string();
                if !id.is_empty() && seen.contains(&id) {
                    continue;
                }
                seen.push_back(id);
                if seen.len() > 100 {
                    seen.pop_front();
                }
                let p = &msg["payload"];
                if let Some((kind, text)) = describe(p["subscription"]["type"].as_str().unwrap_or(""), &p["event"]) {
                    on_alert(kind, text);
                }
            }
            "revocation" => log(format!("Twitch: subscription {} revoked ({})", msg["payload"]["subscription"]["type"], msg["payload"]["subscription"]["status"])),
            _ => {}
        }
    }
}

/// Keep an EventSub connection while `wanted()` holds (a pager is placed) and, before each new
/// connection, `can_start()` (signed in). `subscribe(session_id)` creates the subscriptions for
/// a new session; `on_alert` gets each alert.
pub fn run(wanted: &dyn Fn() -> bool, can_start: &dyn Fn() -> bool, subscribe: &dyn Fn(&str), on_alert: &dyn Fn(Kind, String)) {
    let mut attempt = 0usize;
    let mut seen = VecDeque::new();
    let mut moved_to: Option<String> = None;
    let mut previous: Option<Socket> = None;
    loop {
        if !wanted() || (moved_to.is_none() && !can_start()) {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }
        let url = moved_to.take().unwrap_or_else(|| env_or("TWITCH_EVENTSUB_URL", URL));
        let fresh = url == env_or("TWITCH_EVENTSUB_URL", URL);
        if fresh {
            status("Connecting…");
        }
        let (end, ws) = match open(&url) {
            Ok(mut ws) => {
                attempt = 0;
                // On a move, the old connection closes only once the new one is open (Twitch's advice).
                if let Some(mut old) = previous.take() {
                    let _ = old.close(None);
                    let _ = old.flush();
                }
                (session(&mut ws, fresh, &mut seen, wanted, subscribe, on_alert), Some(ws))
            }
            Err(e) => (End::Lost(format!("can't connect ({e})")), None),
        };
        previous = None;
        match end {
            End::Moved(to) => {
                moved_to = Some(to);
                previous = ws;
            }
            End::Unwanted => status(""),
            End::Lost(why) => {
                log(format!("Twitch EventSub: {why}"));
                status("Reconnecting…");
                std::thread::sleep(Duration::from_secs(RECONNECT_DELAYS_S[attempt.min(RECONNECT_DELAYS_S.len() - 1)]));
                attempt += 1;
            }
        }
    }
}

/// Create the pager's subscriptions for `session_id` with `post` (a Helix POST of one
/// subscription body). Returns the kinds that were refused (a sign-in without their scope).
pub fn subscribe_all(me: &str, session_id: &str, post: &dyn Fn(Value) -> Result<Value>) -> Vec<Kind> {
    let mut refused: Vec<Kind> = vec![];
    for (ty, version, condition, kind) in subscriptions(me) {
        let body = json!({"type": ty, "version": version, "condition": condition, "transport": {"method": "websocket", "session_id": session_id}});
        if let Err(e) = post(body) {
            log(format!("Twitch: can't subscribe to {ty}: {}", e.message()));
            if !refused.contains(&kind) {
                refused.push(kind);
            }
        }
    }
    status(refused_text(&refused));
    refused
}

/// What the pager says about refused subscriptions (empty when none were).
fn refused_text(refused: &[Kind]) -> String {
    match refused {
        [] => String::new(),
        r if r.len() == 4 => "Can't get alerts — run setup again".to_string(),
        r => format!("Run setup again for {}", r.iter().map(|k| k.input()).collect::<Vec<_>>().join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[test]
    fn notifications_read_well() {
        let d = |t: &str, e: Value| describe(t, &e).map(|(_, s)| s);
        assert_eq!(d("channel.follow", json!({"user_name": "Ana"})).as_deref(), Some("Ana followed"));
        assert_eq!(d("channel.subscribe", json!({"user_name": "Bo", "tier": "2000", "is_gift": false})).as_deref(), Some("Bo subscribed (Tier 2)"));
        assert_eq!(d("channel.subscribe", json!({"user_name": "Bo", "is_gift": true})), None);
        assert_eq!(d("channel.subscription.gift", json!({"user_name": null, "is_anonymous": true, "total": 5, "tier": "1000"})).as_deref(), Some("Anonymous gifted 5 subs"));
        assert_eq!(d("channel.subscription.message", json!({"user_name": "Cy", "cumulative_months": 7, "tier": "1000"})).as_deref(), Some("Cy resubscribed · 7 months"));
        assert_eq!(d("channel.raid", json!({"from_broadcaster_user_name": "Dee", "viewers": 42})).as_deref(), Some("Dee raided with 42"));
        assert_eq!(d("channel.cheer", json!({"user_name": "Eve", "bits": 100, "is_anonymous": false})).as_deref(), Some("Eve cheered 100 bits"));
        assert_eq!(d("channel.update", json!({})), None);
    }

    fn welcome(id: &str) -> String {
        json!({"metadata": {"message_type": "session_welcome"}, "payload": {"session": {"id": id}}}).to_string()
    }

    fn note(id: &str, ty: &str, event: Value) -> String {
        json!({"metadata": {"message_id": id, "message_type": "notification"}, "payload": {"subscription": {"type": ty}, "event": event}}).to_string()
    }

    /// In-test EventSub server: welcome → follow (sent twice) → session_reconnect to a second
    /// server, which welcomes and sends a raid. Subscriptions are made once, for the first session.
    #[test]
    fn session_subscribes_dedupes_and_moves() {
        let second = TcpListener::bind("127.0.0.1:0").unwrap();
        let second_url = format!("ws://127.0.0.1:{}/ws", second.local_addr().unwrap().port());
        let first = TcpListener::bind("127.0.0.1:0").unwrap();
        let first_url = format!("ws://127.0.0.1:{}/ws", first.local_addr().unwrap().port());
        std::thread::spawn(move || {
            let mut ws = tungstenite::accept(first.accept().unwrap().0).unwrap();
            ws.send(Message::text(welcome("s1"))).unwrap();
            let follow = note("m1", "channel.follow", json!({"user_name": "Ana"}));
            ws.send(Message::text(follow.clone())).unwrap();
            ws.send(Message::text(follow)).unwrap();
            ws.send(Message::text(json!({"metadata": {"message_type": "session_reconnect"}, "payload": {"session": {"reconnect_url": second_url}}}).to_string())).unwrap();
            let _ = ws.read();
        });
        std::thread::spawn(move || {
            let mut ws = tungstenite::accept(second.accept().unwrap().0).unwrap();
            ws.send(Message::text(welcome("s2"))).unwrap();
            ws.send(Message::text(note("m2", "channel.raid", json!({"from_broadcaster_user_name": "Dee", "viewers": 3})))).unwrap();
            let _ = ws.read();
        });
        std::env::set_var("TWITCH_EVENTSUB_URL", &first_url);
        let alerts = Arc::new(Mutex::new(vec![]));
        let sessions = Arc::new(Mutex::new(vec![]));
        let done = Arc::new(AtomicBool::new(false));
        let (a, s, d) = (alerts.clone(), sessions.clone(), done.clone());
        std::thread::spawn(move || {
            let on_alert = |_: Kind, t: String| {
                a.lock().unwrap().push(t);
                if a.lock().unwrap().len() == 2 {
                    d.store(true, Ordering::SeqCst);
                }
            };
            run(&|| !d.load(Ordering::SeqCst), &|| true, &|sid: &str| s.lock().unwrap().push(sid.to_string()), &on_alert);
        });
        for _ in 0..100 {
            if done.load(Ordering::SeqCst) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(*alerts.lock().unwrap(), vec!["Ana followed".to_string(), "Dee raided with 3".to_string()]);
        assert_eq!(*sessions.lock().unwrap(), vec!["s1".to_string()]);
    }

    #[test]
    fn refused_scopes_name_what_to_fix() {
        let refused = subscribe_all("1", "s", &|b: Value| if b["type"] == "channel.raid" { Ok(json!({})) } else { Err(knobdeck_sdk::Error::msg("Twitch refused: subscription missing proper authorization")) });
        assert_eq!(refused, vec![Kind::Follow, Kind::Sub, Kind::Cheer]);
        assert_eq!(refused_text(&refused), "Run setup again for follows, subs, cheers");
        assert_eq!(refused_text(&[]), "");
    }

    /// Against the Twitch CLI's mock EventSub server (`twitch event websocket start-server -S`),
    /// subscriptions POSTed to its mock endpoint; trigger events with
    /// `twitch event trigger channel.follow --transport=websocket`. Prints what arrives for 30 s.
    #[test]
    #[ignore]
    fn twitch_cli_mock() {
        std::env::set_var("TWITCH_EVENTSUB_URL", "ws://127.0.0.1:8080/ws");
        let start = Instant::now();
        let post = |b: Value| -> Result<Value> {
            ureq::post("http://127.0.0.1:8080/eventsub/subscriptions").set("Client-Id", "mock").set("Authorization", "Bearer mock").send_json(b).map_err(|e| knobdeck_sdk::Error::msg(e.to_string()))?.into_json().map_err(|e| knobdeck_sdk::Error::msg(e.to_string()))
        };
        std::thread::spawn(move || run(&|| start.elapsed() < Duration::from_secs(30), &|| true, &|sid: &str| println!("refused: {:?}", subscribe_all("12345", sid, &post)), &|k, t| println!("ALERT {k:?}: {t}")));
        std::thread::sleep(Duration::from_secs(30));
    }

    /// Twitch's real EventSub endpoint over TLS: the welcome arrives without signing in.
    #[test]
    #[ignore]
    fn real_twitch_welcome() {
        let mut ws = open(URL).unwrap();
        loop {
            match ws.read() {
                Ok(Message::Text(t)) => {
                    let m: Value = t.parse().unwrap();
                    assert_eq!(m["metadata"]["message_type"], "session_welcome");
                    println!("session {}", m["payload"]["session"]["id"]);
                    return;
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
}
