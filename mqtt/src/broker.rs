//! The broker connection: one MQTT 3.1.1 session (rumqttc's sync client) on a background thread,
//! the topics keys watch, and the last message seen on each topic. Shared with the setup binary,
//! which uses it to check the settings before saving them.

use rumqttc::{Client, Event, MqttOptions, Packet, QoS, TlsConfiguration, Transport};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// `.env` keys (the password is in the credential store under [`PASSWORD_KEY`]).
pub const HOST_KEY: &str = "MQTT_HOST";
pub const PORT_KEY: &str = "MQTT_PORT";
pub const TLS_KEY: &str = "MQTT_TLS";
pub const CA_KEY: &str = "MQTT_CA_FILE";
pub const USER_KEY: &str = "MQTT_USERNAME";
pub const CLIENT_ID_KEY: &str = "MQTT_CLIENT_ID";
pub const PASSWORD_KEY: &str = "MQTT_PASSWORD";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    /// A PEM file with the CA that signed the broker's certificate (self-signed brokers). Empty:
    /// the system's trusted roots.
    pub ca_file: String,
    pub username: String,
    pub password: String,
    /// Empty: `knobdeck-<random>`.
    pub client_id: String,
}

pub fn default_port(tls: bool) -> u16 {
    if tls {
        8883
    } else {
        1883
    }
}

impl Settings {
    pub fn describe(&self) -> String {
        format!("{}://{}:{}", if self.tls { "mqtts" } else { "mqtt" }, self.host, self.port)
    }
}

fn client_id(s: &Settings) -> String {
    if !s.client_id.trim().is_empty() {
        return s.client_id.trim().to_string();
    }
    // Unique per run: two KnobDeck machines on one broker must not kick each other off.
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0) as u64 ^ (std::process::id() as u64) << 20;
    format!("knobdeck-{:08x}", n & 0xffff_ffff)
}

fn tls_config(s: &Settings) -> Result<rustls::ClientConfig, String> {
    let mut roots = rustls::RootCertStore::empty();
    if s.ca_file.trim().is_empty() {
        for cert in rustls_native_certs::load_native_certs().certs {
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            return Err("This computer's trusted certificates could not be read; set the broker's CA file in setup".into());
        }
    } else {
        let pem = std::fs::read(s.ca_file.trim()).map_err(|e| format!("CA file {}: {e}", s.ca_file.trim()))?;
        for cert in rustls_pemfile::certs(&mut pem.as_slice()) {
            let cert = cert.map_err(|e| format!("CA file {}: {e}", s.ca_file.trim()))?;
            roots.add(cert).map_err(|e| format!("CA file {}: {e}", s.ca_file.trim()))?;
        }
        if roots.is_empty() {
            return Err(format!("CA file {} holds no PEM certificate", s.ca_file.trim()));
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    Ok(rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().map_err(|e| e.to_string())?.with_root_certificates(roots).with_no_client_auth())
}

#[derive(Default)]
struct State {
    connected: bool,
    /// Set once the session ends for good (the client was dropped).
    closed: bool,
    last_error: Option<String>,
    /// Topic filters keys watch; subscribed again after every reconnect (clean session).
    filters: Vec<String>,
    /// Last payload per topic, with a sequence number so a wildcard filter shows the newest.
    messages: HashMap<String, (u64, String)>,
    seq: u64,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

pub struct Conn {
    client: Client,
    shared: Arc<Shared>,
    pub settings: Settings,
}

/// Payloads above this are kept cut (a key shows a few words; a camera snapshot isn't for keys).
const MAX_KEPT: usize = 64 * 1024;

impl Conn {
    /// Start the session; it connects (and reconnects) on its own thread.
    pub fn start(settings: &Settings) -> Result<Conn, String> {
        if settings.host.trim().is_empty() {
            return Err("Set up the MQTT broker first (its address)".into());
        }
        let mut opts = MqttOptions::new(client_id(settings), settings.host.trim(), settings.port);
        opts.set_keep_alive(Duration::from_secs(30));
        opts.set_clean_session(true);
        opts.set_max_packet_size(256 * 1024, 256 * 1024);
        if !settings.username.is_empty() {
            opts.set_credentials(settings.username.clone(), settings.password.clone());
        }
        if settings.tls {
            opts.set_transport(Transport::tls_with_config(TlsConfiguration::Rustls(Arc::new(tls_config(settings)?))));
        }
        let (client, mut connection) = Client::new(opts, 64);
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), changed: Condvar::new() });
        let (sh, cl) = (shared.clone(), client.clone());
        std::thread::Builder::new()
            .name("mqtt".into())
            .spawn(move || {
                for event in connection.iter() {
                    match event {
                        Ok(Event::Incoming(Packet::ConnAck(ack))) => {
                            let mut st = sh.state.lock().unwrap();
                            if ack.code == rumqttc::ConnectReturnCode::Success {
                                st.connected = true;
                                st.last_error = None;
                                for f in &st.filters {
                                    let _ = cl.try_subscribe(f.clone(), QoS::AtMostOnce);
                                }
                            } else {
                                st.last_error = Some(connack_text(ack.code));
                            }
                            sh.changed.notify_all();
                        }
                        Ok(Event::Incoming(Packet::Publish(p))) => {
                            let mut payload = String::from_utf8_lossy(&p.payload[..p.payload.len().min(MAX_KEPT)]).into_owned();
                            payload.truncate(MAX_KEPT);
                            let mut st = sh.state.lock().unwrap();
                            st.seq += 1;
                            let seq = st.seq;
                            st.messages.insert(p.topic, (seq, payload));
                            sh.changed.notify_all();
                        }
                        Ok(_) => {}
                        Err(e) => {
                            let mut st = sh.state.lock().unwrap();
                            st.connected = false;
                            // A refused CONNACK already left a clearer reason.
                            if !matches!(e, rumqttc::ConnectionError::ConnectionRefused(_)) || st.last_error.is_none() {
                                st.last_error = Some(error_text(&e));
                            }
                            sh.changed.notify_all();
                            drop(st);
                            std::thread::sleep(Duration::from_secs(2)); // then iterating reconnects
                        }
                    }
                }
                let mut st = sh.state.lock().unwrap();
                st.connected = false;
                st.closed = true;
                sh.changed.notify_all();
            })
            .map_err(|e| e.to_string())?;
        Ok(Conn { client, shared, settings: settings.clone() })
    }

    /// Wait until the broker accepted the session, at most `timeout`.
    pub fn wait_connected(&self, timeout: Duration) -> Result<(), String> {
        let end = Instant::now() + timeout;
        let mut st = self.shared.state.lock().unwrap();
        while !st.connected {
            let now = Instant::now();
            if now >= end || st.closed {
                return Err(match &st.last_error {
                    Some(e) => format!("Can't reach the MQTT broker at {}: {e}", self.settings.describe()),
                    None => format!("The MQTT broker at {} didn't answer", self.settings.describe()),
                });
            }
            st = self.shared.changed.wait_timeout(st, end - now).unwrap().0;
        }
        Ok(())
    }

    pub fn publish(&self, topic: &str, payload: &str, qos: QoS, retain: bool) -> Result<(), String> {
        check_topic(topic)?;
        self.wait_connected(Duration::from_secs(3))?;
        self.client.try_publish(topic, qos, retain, payload.as_bytes().to_vec()).map_err(|e| format!("Couldn't publish: {e}"))
    }

    /// Subscribe to `filter` (once; again after each reconnect). True the first time.
    pub fn watch(&self, filter: &str) -> Result<bool, String> {
        check_filter(filter)?;
        let mut st = self.shared.state.lock().unwrap();
        if !st.filters.iter().any(|f| f == filter) {
            st.filters.push(filter.to_string());
            if st.connected {
                self.client.try_subscribe(filter, QoS::AtMostOnce).map_err(|e| format!("Couldn't subscribe: {e}"))?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// The newest payload on a topic the filter matches.
    pub fn latest(&self, filter: &str) -> Option<String> {
        let st = self.shared.state.lock().unwrap();
        if !filter.contains(['+', '#']) {
            return st.messages.get(filter).map(|(_, p)| p.clone());
        }
        st.messages.iter().filter(|(t, _)| topic_matches(filter, t)).max_by_key(|(_, (s, _))| *s).map(|(_, (_, p))| p.clone())
    }

    /// Wait for a message on `filter` newer than what was there when called.
    pub fn wait_message(&self, filter: &str, timeout: Duration) -> Option<String> {
        let end = Instant::now() + timeout;
        let mut st = self.shared.state.lock().unwrap();
        let start = st.seq;
        loop {
            let newest = st.messages.iter().filter(|(t, (s, _))| *s > start && topic_matches(filter, t)).max_by_key(|(_, (s, _))| *s).map(|(_, (_, p))| p.clone());
            if newest.is_some() {
                return newest;
            }
            let now = Instant::now();
            if now >= end {
                return None;
            }
            st = self.shared.changed.wait_timeout(st, end - now).unwrap().0;
        }
    }

    pub fn is_connected(&self) -> bool {
        self.shared.state.lock().unwrap().connected
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared.state.lock().unwrap().last_error.clone()
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        let _ = self.client.try_disconnect();
    }
}

fn connack_text(code: rumqttc::ConnectReturnCode) -> String {
    use rumqttc::ConnectReturnCode as C;
    match code {
        C::BadUserNamePassword => "the broker refused the username or password".into(),
        C::NotAuthorized => "the broker says this client is not authorized (check the username and password)".into(),
        C::BadClientId => "the broker refused the client id".into(),
        C::ServiceUnavailable => "the broker is unavailable".into(),
        C::RefusedProtocolVersion => "the broker doesn't speak MQTT 3.1.1".into(),
        C::Success => "connected".into(),
    }
}

fn error_text(e: &rumqttc::ConnectionError) -> String {
    use rumqttc::ConnectionError as E;
    match e {
        E::ConnectionRefused(code) => connack_text(*code),
        E::Io(io) => io.to_string(),
        E::NetworkTimeout => "connection timed out".into(),
        other => other.to_string(),
    }
}

/// A topic to publish to: not empty, no wildcards.
pub fn check_topic(topic: &str) -> Result<(), String> {
    if topic.is_empty() {
        return Err("Enter a topic".into());
    }
    if topic.contains(['+', '#']) {
        return Err(format!("Can't publish to \"{topic}\": + and # are for subscribing only"));
    }
    Ok(())
}

/// A topic filter: `+` only as a whole level, `#` only as the last level.
pub fn check_filter(filter: &str) -> Result<(), String> {
    if filter.is_empty() {
        return Err("Enter a topic".into());
    }
    let levels: Vec<&str> = filter.split('/').collect();
    for (i, l) in levels.iter().enumerate() {
        let bad = (l.contains('+') && *l != "+") || (l.contains('#') && (*l != "#" || i + 1 != levels.len()));
        if bad {
            return Err(format!("\"{filter}\" isn't a valid topic filter: + and # must fill a whole level, # only at the end"));
        }
    }
    Ok(())
}

/// MQTT topic matching: `+` is one level, `#` the rest (and the parent itself). Topics starting
/// with `$` (`$SYS/…`) match only filters that name them.
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    if topic.starts_with('$') && !filter.starts_with('$') {
        return false;
    }
    let mut f = filter.split('/');
    let mut t = topic.split('/');
    loop {
        match (f.next(), t.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(a), Some(b)) if a == b => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// The value a key shows: the whole payload, or the value at `path` in a JSON payload
/// (`state`, `ENERGY.Power`, `list[0].name`).
pub fn extract(payload: &str, path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(payload.trim().to_string());
    }
    let v: Value = serde_json::from_str(payload).map_err(|_| "The message isn't JSON, so the JSON path can't be read".to_string())?;
    let found = json_path(&v, path).ok_or_else(|| format!("No \"{path}\" in the message"))?;
    Ok(display(found))
}

fn json_path<'a>(mut cur: &'a Value, path: &str) -> Option<&'a Value> {
    let path = path.strip_prefix("$.").or_else(|| path.strip_prefix('$')).unwrap_or(path);
    for part in path.split('.').filter(|p| !p.is_empty()) {
        let (key, rest) = match part.find('[') {
            Some(i) => (&part[..i], &part[i..]),
            None => (part, ""),
        };
        if !key.is_empty() {
            cur = match cur {
                Value::Object(m) => m.get(key)?,
                Value::Array(a) => a.get(key.parse::<usize>().ok()?)?,
                _ => return None,
            };
        }
        let mut rest = rest;
        while let Some(r) = rest.strip_prefix('[') {
            let close = r.find(']')?;
            cur = cur.as_array()?.get(r[..close].trim().parse::<usize>().ok()?)?;
            rest = &r[close + 1..];
        }
        if !rest.is_empty() {
            return None;
        }
    }
    Some(cur)
}

fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "—".into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            _ => n.to_string(),
        },
        other => other.to_string(),
    }
}

/// Whether a state value means "on": the configured on value (case-insensitive), or with none
/// configured, the on payload or the usual words (`ON`, `on`, `true`, `1`).
pub fn is_on(value: &str, on_value: &str, on_payload: &str) -> bool {
    let v = value.trim();
    if !on_value.trim().is_empty() {
        return v.eq_ignore_ascii_case(on_value.trim());
    }
    v.eq_ignore_ascii_case(on_payload.trim()) || ["on", "true", "1", "yes", "open"].iter().any(|w| v.eq_ignore_ascii_case(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching() {
        assert!(topic_matches("a/b", "a/b"));
        assert!(!topic_matches("a/b", "a/b/c"));
        assert!(topic_matches("a/+/c", "a/x/c"));
        assert!(!topic_matches("a/+/c", "a/x/y/c"));
        assert!(topic_matches("a/#", "a"));
        assert!(topic_matches("a/#", "a/b/c"));
        assert!(topic_matches("#", "x/y"));
        assert!(!topic_matches("#", "$SYS/uptime"));
        assert!(topic_matches("$SYS/#", "$SYS/uptime"));
        assert!(check_filter("a/+/#").is_ok());
        assert!(check_filter("a/b#").is_err());
        assert!(check_filter("a/#/b").is_err());
        assert!(check_topic("a/+").is_err());
    }

    #[test]
    fn paths() {
        let z2m = r#"{"state":"ON","brightness":128,"color":{"x":0.3},"list":[{"n":"a"},{"n":"b"}]}"#;
        assert_eq!(extract(z2m, "state").unwrap(), "ON");
        assert_eq!(extract(z2m, "brightness").unwrap(), "128");
        assert_eq!(extract(z2m, "$.color.x").unwrap(), "0.3");
        assert_eq!(extract(z2m, "list[1].n").unwrap(), "b");
        assert!(extract(z2m, "nope").is_err());
        assert_eq!(extract(" 21.5\n", "").unwrap(), "21.5");
        assert!(extract("ON", "state").is_err());
        let tasmota = r#"{"ENERGY":{"Power":42}}"#;
        assert_eq!(extract(tasmota, "ENERGY.Power").unwrap(), "42");
    }

    #[test]
    fn on_values() {
        assert!(is_on("ON", "", "ON"));
        assert!(is_on("on", "", "{\"state\":\"ON\"}"));
        assert!(!is_on("OFF", "", "ON"));
        assert!(is_on("heat", "heat", "x"));
        assert!(!is_on("on", "heat", "on"));
        assert!(is_on("true", "", ""));
    }
}
