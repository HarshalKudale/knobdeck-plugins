//! The connection to the sim: opens it on demand and retries in the background while the sim is
//! closed, maps event names to client events, keeps every watched SimVar's latest value (data
//! requests re-made after each reconnect), turns SimConnect's asynchronous exceptions into
//! errors on the event / SimVar that caused them, and stops watching SimVars nothing reads.
//! Pure logic over the [`Link`] trait; time comes in as arguments so tests can drive it.

use crate::sim::{exception_name, Connector, Link, LinkError, OpenError, Rate, Recv, NOT_RUNNING};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Background retry while the sim is closed.
pub const RETRY: Duration = Duration::from_secs(3);
/// A press retries at most this often.
pub const RETRY_ON_DEMAND: Duration = Duration::from_millis(500);
/// A SimVar no tile has read for this long stops being requested.
pub const IDLE: Duration = Duration::from_secs(30);
/// Client event id of the "Pause" system event subscription.
pub const PAUSE_EVENT: u32 = 1;
/// Most messages handled per pump, so one call can't spin forever.
const PUMP_MAX: usize = 4096;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct VarKey {
    pub name: String,
    pub unit: String,
}

struct Ev {
    id: u32,
    mapped: bool,
    error: Option<String>,
}

struct Var {
    def: u32,
    rate: Rate,
    value: Option<f64>,
    error: Option<String>,
    used: Instant,
    /// Defined and requested on the current connection.
    active: bool,
}

enum Sent {
    Event(String),
    Var(VarKey),
}

pub struct Engine {
    connector: Box<dyn Connector>,
    link: Option<Box<dyn Link>>,
    last_try: Option<Instant>,
    /// Why there is no connection, for the user.
    down: String,
    app: Option<String>,
    events: HashMap<String, Ev>,
    vars: HashMap<VarKey, Var>,
    sends: HashMap<u32, Sent>,
    paused: Option<bool>,
    next_id: u32,
}

/// A K-event name as the sim knows it: "K:AP_MASTER" / "ap_master" → "AP_MASTER". Custom
/// events of add-ons ("Namespace.Event") and "#0x…" ids pass as written.
pub fn event_name(raw: &str) -> Result<String, String> {
    let s = raw.trim();
    let s = s.strip_prefix("K:").or_else(|| s.strip_prefix("k:")).unwrap_or(s).trim();
    if s.is_empty() {
        return Err("Enter an event name, like AP_MASTER".into());
    }
    if s.len() > 64 || !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '#')) {
        return Err(format!("\"{s}\" isn't an event name (letters, digits and _ only, like AP_MASTER)"));
    }
    Ok(if s.contains('.') || s.starts_with('#') { s.to_string() } else { s.to_ascii_uppercase() })
}

/// A SimVar and its unit: "A:PLANE ALTITUDE" / "feet" → ("PLANE ALTITUDE", "feet").
pub fn var_key(name: &str, unit: &str) -> Result<VarKey, String> {
    let n = name.trim();
    let n = n.strip_prefix("A:").or_else(|| n.strip_prefix("a:")).unwrap_or(n).trim();
    let u = unit.trim();
    if n.is_empty() {
        return Err("Enter a SimVar, like PLANE ALTITUDE".into());
    }
    if u.is_empty() {
        return Err("Enter the SimVar's unit, like feet".into());
    }
    if n.len() > 128 || !n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | ':' | '_')) {
        return Err(format!("\"{n}\" isn't a SimVar name (like PLANE ALTITUDE or COM ACTIVE FREQUENCY:1)"));
    }
    if u.len() > 64 || !u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '/' | '_' | '-' | '.')) {
        return Err(format!("\"{u}\" isn't a unit (like feet, knots, percent, bool)"));
    }
    Ok(VarKey { name: n.to_ascii_uppercase(), unit: u.to_string() })
}

impl Engine {
    pub fn new(connector: Box<dyn Connector>) -> Self {
        Self {
            connector,
            link: None,
            last_try: None,
            down: NOT_RUNNING.into(),
            app: None,
            events: HashMap::new(),
            vars: HashMap::new(),
            sends: HashMap::new(),
            paused: None,
            next_id: 100,
        }
    }

    pub fn connected(&self) -> bool {
        self.link.is_some()
    }

    /// The sim's application name while connected.
    pub fn app(&self) -> Option<&str> {
        self.app.as_deref()
    }

    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    /// Background work, every few tens of ms: read what the sim sent, retry a closed sim while
    /// `want` (tiles are placed), and drop SimVars nothing reads.
    pub fn tick(&mut self, now: Instant, want: bool) {
        if self.link.is_none() {
            if want && self.last_try.is_none_or(|t| now.duration_since(t) >= RETRY) {
                self.connect(now);
            }
        }
        if self.link.is_some() {
            self.pump();
            self.collect(now);
        }
    }

    /// Connect now unless a try was made very recently; the reason when there is no connection.
    pub fn ensure(&mut self, now: Instant) -> Result<(), String> {
        if self.link.is_none() && self.last_try.is_none_or(|t| now.duration_since(t) >= RETRY_ON_DEMAND) {
            self.connect(now);
        }
        if self.link.is_some() {
            Ok(())
        } else {
            Err(self.down.clone())
        }
    }

    fn connect(&mut self, now: Instant) {
        self.last_try = Some(now);
        match self.connector.open() {
            Ok(link) => {
                self.link = Some(link);
                self.sends.clear();
                // A new session (maybe another sim or aircraft): names get a fresh chance.
                for ev in self.events.values_mut() {
                    ev.mapped = false;
                    ev.error = None;
                }
                for v in self.vars.values_mut() {
                    v.error = None;
                }
                if let Some(l) = self.link.as_mut() {
                    if l.subscribe(PAUSE_EVENT, "Pause").is_err() {
                        self.paused = None;
                    }
                }
                let keys: Vec<VarKey> = self.vars.keys().cloned().collect();
                for k in keys {
                    self.activate(&k);
                }
                self.pump();
            }
            Err(OpenError::NotRunning) => self.down = NOT_RUNNING.into(),
            Err(OpenError::Unavailable(m)) => self.down = m,
        }
    }

    /// The connection is gone: forget everything that belonged to it.
    fn lost(&mut self) {
        self.link = None;
        self.app = None;
        self.paused = None;
        self.sends.clear();
        self.down = NOT_RUNNING.into();
        for v in self.vars.values_mut() {
            v.value = None;
            v.active = false;
        }
        for ev in self.events.values_mut() {
            ev.mapped = false;
        }
    }

    /// A link call's error as the user's message; a lost connection is dropped.
    fn fail(&mut self, e: LinkError) -> String {
        match e {
            LinkError::Lost => {
                self.lost();
                NOT_RUNNING.into()
            }
            LinkError::Failed(m) => m,
        }
    }

    /// Define and request one SimVar on the current connection.
    fn activate(&mut self, key: &VarKey) {
        let Some(var) = self.vars.get(key) else { return };
        let (def, rate) = (var.def, var.rate);
        let Some(link) = self.link.as_mut() else { return };
        let result = link.add_var(def, &key.name, &key.unit).and_then(|send| link.request(def, Some(rate)).map(|_| send));
        match result {
            Ok(send) => {
                self.sends.insert(send, Sent::Var(key.clone()));
                if let Some(v) = self.vars.get_mut(key) {
                    v.active = true;
                }
            }
            Err(e) => {
                let m = self.fail(e);
                if let Some(v) = self.vars.get_mut(key) {
                    if self.link.is_some() {
                        v.error = Some(m);
                    }
                }
            }
        }
    }

    /// Handle everything the sim sent.
    pub fn pump(&mut self) {
        for _ in 0..PUMP_MAX {
            let Some(link) = self.link.as_mut() else { return };
            match link.next() {
                Ok(Some(r)) => self.handle(r),
                Ok(None) => return,
                Err(LinkError::Lost) => {
                    self.lost();
                    return;
                }
                Err(LinkError::Failed(_)) => return,
            }
        }
    }

    fn handle(&mut self, r: Recv) {
        match r {
            Recv::Open { app } => self.app = Some(app),
            Recv::Quit => self.lost(),
            Recv::Data { request, value } => {
                if let Some(v) = self.vars.values_mut().find(|v| v.def == request) {
                    v.value = Some(value);
                }
            }
            Recv::Event { id: PAUSE_EVENT, data } => self.paused = Some(data != 0),
            Recv::Event { .. } | Recv::Other => {}
            Recv::Exception { code, send_id } => match self.sends.remove(&send_id) {
                Some(Sent::Event(name)) => {
                    if let Some(ev) = self.events.get_mut(&name) {
                        ev.error = Some(if code == 7 {
                            format!("Flight Simulator doesn't know the event {name}")
                        } else {
                            format!("Flight Simulator refused the event {name} ({})", exception_name(code))
                        });
                    }
                }
                Some(Sent::Var(key)) => {
                    if let Some(v) = self.vars.get_mut(&key) {
                        v.error = Some(if code == 7 {
                            format!("Flight Simulator doesn't know {} in {}", key.name, key.unit)
                        } else {
                            format!("Flight Simulator refused {} in {} ({})", key.name, key.unit, exception_name(code))
                        });
                        v.value = None;
                    }
                }
                None => {}
            },
        }
    }

    /// Stop requesting SimVars no tile has read for [`IDLE`].
    fn collect(&mut self, now: Instant) {
        let idle: Vec<VarKey> = self.vars.iter().filter(|(_, v)| now.duration_since(v.used) >= IDLE).map(|(k, _)| k.clone()).collect();
        for k in idle {
            if let Some(v) = self.vars.remove(&k) {
                if v.active {
                    if let Some(link) = self.link.as_mut() {
                        let r = link.request(v.def, None).and_then(|_| link.clear_var(v.def));
                        if let Err(LinkError::Lost) = r {
                            self.lost();
                        }
                    }
                }
            }
        }
    }

    /// Send event `name` (already checked by [`event_name`]) with its data, connecting first.
    pub fn send(&mut self, now: Instant, name: &str, data: [u32; 5]) -> Result<(), String> {
        self.ensure(now)?;
        if !self.events.contains_key(name) {
            let id = self.id();
            self.events.insert(name.to_string(), Ev { id, mapped: false, error: None });
        }
        let ev = &self.events[name];
        if let Some(e) = &ev.error {
            return Err(e.clone());
        }
        let (id, mapped) = (ev.id, ev.mapped);
        let link = self.link.as_mut().ok_or_else(|| NOT_RUNNING.to_string())?;
        if !mapped {
            match link.map_event(id, name) {
                Ok(send) => {
                    self.sends.insert(send, Sent::Event(name.to_string()));
                    if let Some(ev) = self.events.get_mut(name) {
                        ev.mapped = true;
                    }
                }
                Err(e) => return Err(self.fail(e)),
            }
        }
        let link = self.link.as_mut().ok_or_else(|| NOT_RUNNING.to_string())?;
        link.transmit(id, data).map_err(|e| self.fail(e))
    }

    /// The sim's complaint about an event name, once it has sent one.
    pub fn event_error(&self, name: &str) -> Option<String> {
        self.events.get(name).and_then(|e| e.error.clone())
    }

    /// A SimVar's latest value (`None` until the sim sends the first one). Starts watching it.
    /// Never connects: the background tick does that while tiles are placed.
    pub fn value(&mut self, now: Instant, key: &VarKey, rate: Rate) -> Result<Option<f64>, String> {
        if !self.vars.contains_key(key) {
            let def = self.id();
            self.vars.insert(key.clone(), Var { def, rate, value: None, error: None, used: now, active: false });
            if self.link.is_some() {
                self.activate(key);
            }
        }
        let v = self.vars.get_mut(key).ok_or_else(|| NOT_RUNNING.to_string())?;
        v.used = now;
        if let Some(e) = &v.error {
            return Err(e.clone());
        }
        if self.link.is_none() {
            return Err(self.down.clone());
        }
        Ok(self.vars[key].value)
    }

    /// Whether the sim is paused, once it has said so since connecting.
    pub fn paused(&self) -> Result<Option<bool>, String> {
        if self.link.is_none() {
            return Err(self.down.clone());
        }
        Ok(self.paused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// What the fake sim saw and will answer.
    #[derive(Default)]
    struct World {
        running: bool,
        missing_dll: bool,
        opens: u32,
        calls: Vec<String>,
        inbox: VecDeque<Recv>,
        /// The next `next()` reports the connection lost.
        break_pipe: bool,
        send_id: u32,
        /// Sim event names it knows; others raise exception 7 like MSFS does.
        known_events: Vec<&'static str>,
        /// The last send id per event / var name, for tests to aim exceptions.
        sent: HashMap<String, u32>,
    }

    type Shared = Arc<Mutex<World>>;

    struct FakeConnector(Shared);
    struct FakeLink(Shared);

    impl Connector for FakeConnector {
        fn open(&mut self) -> Result<Box<dyn Link>, OpenError> {
            let mut w = self.0.lock().unwrap();
            w.opens += 1;
            if w.missing_dll {
                return Err(OpenError::Unavailable("SimConnect.dll not found".into()));
            }
            if !w.running {
                return Err(OpenError::NotRunning);
            }
            w.inbox.push_back(Recv::Open { app: "KittyHawk".into() });
            Ok(Box::new(FakeLink(self.0.clone())))
        }
    }

    impl FakeLink {
        fn call(&self, s: String) -> Result<u32, LinkError> {
            let mut w = self.0.lock().unwrap();
            if !w.running {
                return Err(LinkError::Lost);
            }
            w.send_id += 1;
            w.calls.push(s);
            Ok(w.send_id)
        }
    }

    impl Link for FakeLink {
        fn map_event(&mut self, id: u32, name: &str) -> Result<u32, LinkError> {
            let send = self.call(format!("map {id} {name}"))?;
            let mut w = self.0.lock().unwrap();
            w.sent.insert(name.to_string(), send);
            if !w.known_events.contains(&name) {
                w.inbox.push_back(Recv::Exception { code: 7, send_id: send });
            }
            Ok(send)
        }
        fn transmit(&mut self, id: u32, data: [u32; 5]) -> Result<(), LinkError> {
            self.call(format!("transmit {id} {:?}", data)).map(|_| ())
        }
        fn add_var(&mut self, def: u32, name: &str, unit: &str) -> Result<u32, LinkError> {
            let send = self.call(format!("add {def} {name} ({unit})"))?;
            self.0.lock().unwrap().sent.insert(name.to_string(), send);
            Ok(send)
        }
        fn request(&mut self, def: u32, rate: Option<Rate>) -> Result<(), LinkError> {
            self.call(format!("request {def} {rate:?}")).map(|_| ())
        }
        fn clear_var(&mut self, def: u32) -> Result<(), LinkError> {
            self.call(format!("clear {def}")).map(|_| ())
        }
        fn subscribe(&mut self, id: u32, name: &str) -> Result<(), LinkError> {
            self.call(format!("subscribe {id} {name}")).map(|_| ())
        }
        fn next(&mut self) -> Result<Option<Recv>, LinkError> {
            let mut w = self.0.lock().unwrap();
            if w.break_pipe {
                w.break_pipe = false;
                w.running = false;
                return Err(LinkError::Lost);
            }
            Ok(w.inbox.pop_front())
        }
    }

    fn setup(running: bool) -> (Engine, Shared) {
        let w = Arc::new(Mutex::new(World { running, known_events: vec!["AP_MASTER", "HEADING_BUG_SET", "GEAR_TOGGLE"], ..Default::default() }));
        (Engine::new(Box::new(FakeConnector(w.clone()))), w)
    }

    fn key(name: &str, unit: &str) -> VarKey {
        var_key(name, unit).unwrap()
    }

    fn calls(w: &Shared) -> Vec<String> {
        std::mem::take(&mut w.lock().unwrap().calls)
    }

    /// Deliver a value for the SimVar the engine defined last under `name`.
    fn deliver(w: &Shared, e: &Engine, k: &VarKey, value: f64) {
        let def = e.vars[k].def;
        w.lock().unwrap().inbox.push_back(Recv::Data { request: def, value });
    }

    #[test]
    fn clear_errors_while_the_sim_is_closed() {
        let (mut e, w) = setup(false);
        let t = Instant::now();
        assert_eq!(e.send(t, "AP_MASTER", [0; 5]), Err(NOT_RUNNING.into()));
        assert_eq!(e.value(t, &key("AUTOPILOT MASTER", "bool"), Rate::Frame), Err(NOT_RUNNING.into()));
        assert_eq!(e.paused(), Err(NOT_RUNNING.into()));
        w.lock().unwrap().missing_dll = true;
        assert_eq!(e.send(t + RETRY_ON_DEMAND, "AP_MASTER", [0; 5]), Err("SimConnect.dll not found".into()));
    }

    #[test]
    fn maps_an_event_once_and_sends_its_data() {
        let (mut e, w) = setup(true);
        let t = Instant::now();
        e.send(t, "HEADING_BUG_SET", [270, 0, 0, 0, 0]).unwrap();
        e.send(t, "HEADING_BUG_SET", [90, 0, 0, 0, 0]).unwrap();
        let c = calls(&w);
        assert_eq!(c[0], "subscribe 1 Pause");
        assert_eq!(c.iter().filter(|c| c.starts_with("map")).count(), 1);
        assert!(c[1].starts_with("map ") && c[1].ends_with(" HEADING_BUG_SET"));
        assert!(c[2].ends_with("[270, 0, 0, 0, 0]"));
        assert!(c[3].ends_with("[90, 0, 0, 0, 0]"));
        assert_eq!(e.app(), Some("KittyHawk"));
    }

    #[test]
    fn values_arrive_by_request_id() {
        let (mut e, w) = setup(true);
        let t = Instant::now();
        e.ensure(t).unwrap();
        let hdg = key("AUTOPILOT HEADING LOCK DIR", "degrees");
        let alt = key("AUTOPILOT ALTITUDE LOCK VAR", "feet");
        assert_eq!(e.value(t, &hdg, Rate::Frame), Ok(None));
        assert_eq!(e.value(t, &alt, Rate::Second), Ok(None));
        let c = calls(&w);
        assert!(c.iter().any(|c| c.contains("add") && c.contains("AUTOPILOT HEADING LOCK DIR (degrees)")));
        assert!(c.iter().any(|c| c.starts_with("request") && c.ends_with("Some(Second)")));
        deliver(&w, &e, &hdg, 270.0);
        deliver(&w, &e, &alt, 5500.0);
        e.tick(t, true);
        assert_eq!(e.value(t, &hdg, Rate::Frame), Ok(Some(270.0)));
        assert_eq!(e.value(t, &alt, Rate::Second), Ok(Some(5500.0)));
    }

    #[test]
    fn reconnects_after_the_sim_quits() {
        let (mut e, w) = setup(true);
        let t = Instant::now();
        e.ensure(t).unwrap();
        let gear = key("GEAR HANDLE POSITION", "bool");
        e.value(t, &gear, Rate::Frame).unwrap();
        e.send(t, "GEAR_TOGGLE", [0; 5]).unwrap();
        deliver(&w, &e, &gear, 1.0);
        e.tick(t, true);
        assert_eq!(e.value(t, &gear, Rate::Frame), Ok(Some(1.0)));

        // The sim quits: values go, errors say so.
        {
            let mut wl = w.lock().unwrap();
            wl.inbox.push_back(Recv::Quit);
            wl.running = false;
        }
        e.tick(t, true);
        assert!(!e.connected());
        assert_eq!(e.value(t, &gear, Rate::Frame), Err(NOT_RUNNING.into()));
        assert_eq!(e.send(t, "GEAR_TOGGLE", [0; 5]), Err(NOT_RUNNING.into()));

        // Background retries wait for RETRY, and only while tiles want the sim.
        let opens = w.lock().unwrap().opens;
        w.lock().unwrap().running = true;
        calls(&w);
        e.tick(t + Duration::from_secs(1), true);
        assert_eq!(w.lock().unwrap().opens, opens, "no retry before RETRY");
        e.tick(t + RETRY * 2, false);
        assert_eq!(w.lock().unwrap().opens, opens, "no retry without tiles");
        e.tick(t + RETRY * 2, true);
        assert!(e.connected());

        // Watched SimVars are defined and requested again; events are mapped again on use.
        let c = calls(&w);
        assert!(c.iter().any(|c| c.contains("add") && c.contains("GEAR HANDLE POSITION")));
        assert_eq!(e.value(t, &gear, Rate::Frame), Ok(None));
        e.send(t + RETRY * 2, "GEAR_TOGGLE", [0; 5]).unwrap();
        assert!(calls(&w).iter().any(|c| c.starts_with("map") && c.ends_with("GEAR_TOGGLE")));
    }

    #[test]
    fn a_broken_pipe_drops_the_connection() {
        let (mut e, w) = setup(true);
        let t = Instant::now();
        e.ensure(t).unwrap();
        w.lock().unwrap().break_pipe = true;
        e.tick(t, true);
        assert!(!e.connected());
        assert_eq!(e.send(t, "AP_MASTER", [0; 5]), Err(NOT_RUNNING.into()));
    }

    #[test]
    fn a_press_retries_sooner_than_the_background() {
        let (mut e, w) = setup(false);
        let t = Instant::now();
        assert!(e.ensure(t).is_err());
        w.lock().unwrap().running = true;
        assert!(e.ensure(t + Duration::from_millis(100)).is_err(), "not twice in a row");
        assert!(e.ensure(t + RETRY_ON_DEMAND).is_ok());
    }

    #[test]
    fn unknown_events_and_simvars_report_themselves() {
        let (mut e, w) = setup(true);
        let t = Instant::now();
        // The sim answers a mapping of an unknown name with an exception, after the fact.
        e.send(t, "NOT_AN_EVENT", [0; 5]).unwrap();
        e.tick(t, true);
        assert_eq!(e.send(t, "NOT_AN_EVENT", [0; 5]), Err("Flight Simulator doesn't know the event NOT_AN_EVENT".into()));
        assert!(e.send(t, "AP_MASTER", [0; 5]).is_ok(), "other events still work");

        let bad = key("NOT A VAR", "feet");
        assert_eq!(e.value(t, &bad, Rate::Frame), Ok(None));
        let send = w.lock().unwrap().sent["NOT A VAR"];
        w.lock().unwrap().inbox.push_back(Recv::Exception { code: 7, send_id: send });
        e.tick(t, true);
        assert_eq!(e.value(t, &bad, Rate::Frame), Err("Flight Simulator doesn't know NOT A VAR in feet".into()));
    }

    #[test]
    fn idle_simvars_stop_being_requested() {
        let (mut e, w) = setup(true);
        let t = Instant::now();
        e.ensure(t).unwrap();
        let ias = key("AIRSPEED INDICATED", "knots");
        let alt = key("INDICATED ALTITUDE", "feet");
        e.value(t, &ias, Rate::Frame).unwrap();
        e.value(t, &alt, Rate::Frame).unwrap();
        let ias_def = e.vars[&ias].def;
        calls(&w);
        e.value(t + IDLE, &alt, Rate::Frame).unwrap();
        e.tick(t + IDLE, true);
        let c = calls(&w);
        assert_eq!(c, vec![format!("request {ias_def} None"), format!("clear {ias_def}")]);
        assert!(!e.vars.contains_key(&ias));
        assert!(e.vars.contains_key(&alt));
    }

    #[test]
    fn pause_state_from_the_system_event() {
        let (mut e, w) = setup(true);
        let t = Instant::now();
        e.ensure(t).unwrap();
        assert_eq!(e.paused(), Ok(None));
        w.lock().unwrap().inbox.push_back(Recv::Event { id: PAUSE_EVENT, data: 1 });
        e.tick(t, true);
        assert_eq!(e.paused(), Ok(Some(true)));
        w.lock().unwrap().inbox.push_back(Recv::Event { id: PAUSE_EVENT, data: 0 });
        e.tick(t, true);
        assert_eq!(e.paused(), Ok(Some(false)));
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(event_name("K:ap_master"), Ok("AP_MASTER".into()));
        assert_eq!(event_name(" XPNDR_SET "), Ok("XPNDR_SET".into()));
        assert_eq!(event_name("MobiFlight.MyEvent"), Ok("MobiFlight.MyEvent".into()));
        assert!(event_name("").is_err());
        assert!(event_name("AP MASTER").is_err());
        assert!(event_name("(>K:AP_MASTER)").is_err());
        assert_eq!(var_key("A:plane altitude", "feet").unwrap().name, "PLANE ALTITUDE");
        assert_eq!(var_key("COM ACTIVE FREQUENCY:1", "MHz").unwrap().unit, "MHz");
        assert!(var_key("PLANE ALTITUDE", "").is_err());
        assert!(var_key("", "feet").is_err());
        assert!(var_key("PLANE;ALTITUDE", "feet").is_err());
        assert!(var_key("PLANE ALTITUDE", "feet\"").is_err());
    }
}
