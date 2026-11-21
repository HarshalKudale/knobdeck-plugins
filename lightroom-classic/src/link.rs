//! The link to the KnobDeck plug-in inside Lightroom Classic.
//!
//! Lightroom's `LrSocket` listens on localhost and each socket only sends or only receives, so the
//! plug-in listens on two ports and this side connects to both: the event port first (Lightroom →
//! KnobDeck: replies and events), then the command port (KnobDeck → Lightroom). It then says
//! `hello <token>`; the first time, Lightroom asks the user to allow KnobDeck (`pending`), then
//! sends `paired` and the hello is repeated. A refused token stays refused until Lightroom
//! restarts (`denied`).
//!
//! Dial turns are coalesced per setting: while one `nudge` / `set` for a setting is unanswered,
//! further ticks add up and go as one when the answer comes, so a fast dial never builds a queue
//! in Lightroom.

use crate::proto::{self, Inbound};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const COMMAND_PORT: u16 = 58770;
pub const EVENT_PORT: u16 = 58771;
const RETRY: Duration = Duration::from_secs(2);
const HELLO_AGAIN: Duration = Duration::from_secs(3);
/// An unanswered dial step stops holding back the next one after this long.
const STUCK: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct Config {
    pub command_port: u16,
    pub event_port: u16,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Offline,
    /// Lightroom shows its Allow dialog.
    Pending,
    Denied,
    Linked { plugin: String, lightroom: String },
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Photo {
    pub name: String,
    /// `pick`, `reject` or `none`.
    pub flag: String,
    pub rating: u8,
    /// `red`, `yellow`, `green`, `blue`, `purple` or `none`.
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    Status(Status),
    Module(String),
    Photo(Option<Photo>),
    Value(String, f64),
    /// A coalesced dial step failed; the key it came from shows the message.
    Failed { instance: String, message: String },
}

pub fn not_linked(status: &Status) -> String {
    match status {
        Status::Pending => "Click Allow in the dialog Lightroom Classic shows to let KnobDeck control it.".into(),
        Status::Denied => "Lightroom Classic refused KnobDeck. Restart Lightroom and click Allow, or run this plugin's setup with \"Pair again\".".into(),
        _ => "Lightroom Classic isn't linked. Open Lightroom Classic with the KnobDeck plug-in installed (this plugin's setup installs it).".into(),
    }
}

/// A dial change for one setting.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Ticks(f64),
    Set(f64),
}

#[derive(Clone)]
struct Adjust {
    step: String,
    panel: String,
    instance: String,
}

#[derive(Default)]
struct Flight {
    since: Option<Instant>,
    set: Option<f64>,
    ticks: f64,
    next: Option<Adjust>,
}

enum Pending {
    Wait(mpsc::Sender<Result<Vec<String>, String>>),
    Hello,
    Fire { param: String, instance: String },
    Ignore,
}

#[derive(Default)]
struct Cache {
    module: Option<String>,
    photo: Option<Photo>,
    values: HashMap<String, f64>,
}

struct Inner {
    writer: Option<TcpStream>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    status: Status,
    cache: Cache,
    flights: HashMap<String, Flight>,
    hello_sent: Option<Instant>,
}

type Listener = Box<dyn Fn(&Update) + Send + Sync>;

pub struct Link {
    cfg: Config,
    watch: Vec<String>,
    inner: Mutex<Inner>,
    listener: Mutex<Option<Listener>>,
}

impl Link {
    pub fn new(cfg: Config, watch: Vec<String>) -> Arc<Link> {
        Arc::new(Link {
            cfg,
            watch,
            inner: Mutex::new(Inner {
                writer: None,
                next_id: 1,
                pending: HashMap::new(),
                status: Status::Offline,
                cache: Cache::default(),
                flights: HashMap::new(),
                hello_sent: None,
            }),
            listener: Mutex::new(None),
        })
    }

    /// Called (off the lock) for every change.
    pub fn on_update(&self, f: impl Fn(&Update) + Send + Sync + 'static) {
        *self.listener.lock().unwrap() = Some(Box::new(f));
    }

    fn emit(&self, updates: Vec<Update>) {
        if let Some(l) = self.listener.lock().unwrap().as_ref() {
            for u in &updates {
                l(u);
            }
        }
    }

    pub fn status(&self) -> Status {
        self.inner.lock().unwrap().status.clone()
    }
    pub fn module(&self) -> Option<String> {
        self.inner.lock().unwrap().cache.module.clone()
    }
    pub fn photo(&self) -> Option<Photo> {
        self.inner.lock().unwrap().cache.photo.clone()
    }
    pub fn value(&self, param: &str) -> Option<f64> {
        self.inner.lock().unwrap().cache.values.get(param).copied()
    }

    /// Connect, and reconnect, until the process ends.
    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || loop {
            if let Err(e) = me.session() {
                knobdeck_sdk::log(format!("lightroom link: {e}"));
            }
            me.disconnected();
            std::thread::sleep(RETRY);
        });
    }

    fn connect(port: u16) -> std::io::Result<TcpStream> {
        let mut last = None;
        for ip in ["127.0.0.1", "::1"] {
            let addr: SocketAddr = format!("{}:{port}", if ip.contains(':') { format!("[{ip}]") } else { ip.into() }).parse().unwrap();
            match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
                Ok(s) => return Ok(s),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap())
    }

    /// One connection: until Lightroom closes it.
    fn session(&self) -> std::io::Result<()> {
        let Ok(mut events) = Self::connect(self.cfg.event_port) else { return Ok(()) }; // Lightroom isn't running
        let commands = Self::connect(self.cfg.command_port)?;
        let _ = commands.set_nodelay(true);
        events.set_read_timeout(Some(Duration::from_millis(500)))?;
        self.inner.lock().unwrap().writer = Some(commands);
        knobdeck_sdk::log(format!("lightroom link: connected to ports {} / {}", self.cfg.command_port, self.cfg.event_port));
        self.hello();
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match events.read(&mut chunk) {
                Ok(0) => return Ok(()),
                Ok(n) => {
                    buf.extend_from_slice(&chunk[..n]);
                    while let Some(i) = buf.iter().position(|b| *b == b'\n') {
                        let line: Vec<u8> = buf.drain(..=i).collect();
                        if !self.on_line(&String::from_utf8_lossy(&line)) {
                            return Ok(());
                        }
                    }
                }
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(e) => return Err(e),
            }
            // No answer to hello (the event socket wasn't ready on Lightroom's side): say it again.
            let again = {
                let i = self.inner.lock().unwrap();
                i.status == Status::Offline && i.hello_sent.is_some_and(|t| t.elapsed() > HELLO_AGAIN)
            };
            if again {
                self.hello();
            }
            if self.inner.lock().unwrap().writer.is_none() {
                return Ok(());
            }
        }
    }

    fn hello(&self) {
        let mut i = self.inner.lock().unwrap();
        i.hello_sent = Some(Instant::now());
        let _ = Self::send_locked(&mut i, Pending::Hello, &["hello", &self.cfg.token, concat!("knobdeck-lightroom-classic/", env!("CARGO_PKG_VERSION"))]);
    }

    /// Write one command; its answer goes to `pending`.
    fn send_locked(i: &mut Inner, pending: Pending, fields: &[&str]) -> Result<u64, String> {
        let id = i.next_id;
        i.next_id += 1;
        let mut line = proto::encode(&[&id.to_string()]);
        line.push('\t');
        line.push_str(&proto::encode(fields));
        line.push('\n');
        let Some(w) = i.writer.as_mut() else { return Err(not_linked(&i.status)) };
        if let Err(e) = w.write_all(line.as_bytes()) {
            i.writer = None;
            return Err(format!("Lightroom Classic closed the link ({e})"));
        }
        i.pending.insert(id, pending);
        Ok(id)
    }

    fn disconnected(&self) {
        let mut ups = vec![];
        {
            let mut i = self.inner.lock().unwrap();
            i.writer = None;
            i.hello_sent = None;
            i.flights.clear();
            for (_, p) in i.pending.drain() {
                if let Pending::Wait(tx) = p {
                    let _ = tx.send(Err("Lightroom Classic closed the link".into()));
                }
            }
            if i.status != Status::Offline {
                i.status = Status::Offline;
                ups.push(Update::Status(Status::Offline));
            }
        }
        self.emit(ups);
    }

    /// One line from Lightroom; false ends the session.
    fn on_line(&self, line: &str) -> bool {
        let Some(msg) = proto::parse(line) else { return true };
        let mut ups = vec![];
        let mut keep = true;
        let mut hello_again = false;
        {
            let mut i = self.inner.lock().unwrap();
            match msg {
                Inbound::Reply { id, result } => match i.pending.remove(&id) {
                    Some(Pending::Wait(tx)) => {
                        let _ = tx.send(result);
                    }
                    Some(Pending::Hello) => match result {
                        Ok(f) => {
                            let s = Status::Linked { plugin: f.first().cloned().unwrap_or_default(), lightroom: f.get(1).cloned().unwrap_or_default() };
                            i.status = s.clone();
                            ups.push(Update::Status(s));
                            let list = self.watch.join(",");
                            let _ = Self::send_locked(&mut i, Pending::Ignore, &["watch", &list]);
                        }
                        Err(e) if e == "pending" => {
                            if i.status != Status::Pending {
                                i.status = Status::Pending;
                                ups.push(Update::Status(Status::Pending));
                            }
                        }
                        Err(e) if e == "denied" => {
                            i.status = Status::Denied;
                            ups.push(Update::Status(Status::Denied));
                        }
                        Err(e) => knobdeck_sdk::log(format!("lightroom link: hello refused: {e}")),
                    },
                    Some(Pending::Fire { param, instance }) => {
                        match result {
                            Ok(f) => {
                                if let Some(v) = f.get(1).and_then(|v| v.parse::<f64>().ok()) {
                                    if i.cache.values.insert(param.clone(), v) != Some(v) {
                                        ups.push(Update::Value(param.clone(), v));
                                    }
                                }
                            }
                            Err(message) => ups.push(Update::Failed { instance, message }),
                        }
                        Self::flush_locked(&mut i, &param, &mut ups);
                    }
                    Some(Pending::Ignore) | None => {}
                },
                Inbound::Event { name, fields } => match name.as_str() {
                    "paired" => hello_again = true,
                    "denied" => {
                        i.status = Status::Denied;
                        ups.push(Update::Status(Status::Denied));
                    }
                    "bye" => keep = false,
                    "module" => {
                        let m = fields.into_iter().next().unwrap_or_default();
                        i.cache.module = Some(m.clone());
                        ups.push(Update::Module(m));
                    }
                    "photo" => {
                        let p = parse_photo(&fields);
                        i.cache.photo = p.clone();
                        ups.push(Update::Photo(p));
                    }
                    "value" => {
                        if let (Some(p), Some(v)) = (fields.first(), fields.get(1).and_then(|v| v.parse::<f64>().ok())) {
                            if i.cache.values.insert(p.clone(), v) != Some(v) {
                                ups.push(Update::Value(p.clone(), v));
                            }
                        }
                    }
                    _ => {}
                },
            }
        }
        if hello_again {
            self.hello();
        }
        self.emit(ups);
        keep
    }

    /// Send what piled up for `param` while its last step was unanswered, or end its flight.
    fn flush_locked(i: &mut Inner, param: &str, ups: &mut Vec<Update>) {
        let Some(f) = i.flights.get_mut(param) else { return };
        let Some(adj) = f.next.take() else {
            i.flights.remove(param);
            return;
        };
        let (fields, set_left): (Vec<String>, bool) = if let Some(v) = f.set.take() {
            (vec!["set".into(), param.into(), proto::num(v), adj.panel.clone()], f.ticks != 0.0)
        } else {
            let t = std::mem::take(&mut f.ticks);
            (vec!["nudge".into(), param.into(), proto::num(t), adj.step.clone(), adj.panel.clone()], false)
        };
        f.since = Some(Instant::now());
        if set_left {
            f.next = Some(adj.clone()); // ticks after an absolute set go next
        }
        let refs: Vec<&str> = fields.iter().map(String::as_str).collect();
        if let Err(message) = Self::send_locked(i, Pending::Fire { param: param.into(), instance: adj.instance.clone() }, &refs) {
            i.flights.remove(param);
            ups.push(Update::Failed { instance: adj.instance, message });
        }
    }

    /// Turn or set a develop setting. Returns at once; the new value arrives as an update.
    pub fn adjust(&self, param: &str, change: Change, step: &str, panel: &str, instance: &str) -> Result<(), String> {
        let mut ups = vec![];
        let r = {
            let mut i = self.inner.lock().unwrap();
            if !matches!(i.status, Status::Linked { .. }) {
                return Err(not_linked(&i.status));
            }
            let f = i.flights.entry(param.to_string()).or_default();
            let busy = f.since.is_some_and(|t| t.elapsed() < STUCK);
            match change {
                Change::Set(v) => {
                    f.set = Some(v);
                    f.ticks = 0.0;
                }
                Change::Ticks(t) => f.ticks += t,
            }
            f.next = Some(Adjust { step: step.into(), panel: panel.into(), instance: instance.into() });
            if !busy {
                Self::flush_locked(&mut i, param, &mut ups);
            }
            Ok(())
        };
        self.emit(ups);
        r
    }

    /// Run a command and wait for its answer.
    pub fn request(&self, fields: &[&str], timeout: Duration) -> Result<Vec<String>, String> {
        let (tx, rx) = mpsc::channel();
        let id = {
            let mut i = self.inner.lock().unwrap();
            if !matches!(i.status, Status::Linked { .. }) {
                return Err(not_linked(&i.status));
            }
            Self::send_locked(&mut i, Pending::Wait(tx), fields)?
        };
        match rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(_) => {
                self.inner.lock().unwrap().pending.remove(&id);
                Err("Lightroom Classic didn't answer in time (a dialog may be open in Lightroom)".into())
            }
        }
    }
}

pub fn parse_photo(f: &[String]) -> Option<Photo> {
    let name = f.first().filter(|n| !n.is_empty())?;
    Some(Photo {
        name: name.clone(),
        flag: f.get(1).cloned().unwrap_or_else(|| "none".into()),
        rating: f.get(2).and_then(|r| r.parse().ok()).unwrap_or(0),
        label: f.get(3).cloned().filter(|l| !l.is_empty()).unwrap_or_else(|| "none".into()),
    })
}
