//! A stand-in for the KnobDeck plug-in inside Lightroom Classic, for the tests and for
//! `cargo run -p lightroom-classic --example fake-lightroom` (to drive the built plugin with
//! `knobdeck-plugin drive`). Like `LrSocket` it listens on two loopback ports, one that only
//! receives commands and one that only sends answers and events, each taking one client at a
//! time. Its develop model is small (a few settings, three photos); the real plug-in's commands
//! are tested against a fake Lightroom in the companion repo, and end to end with this plugin.

use crate::link::{Config, Photo};
use crate::params;
use crate::proto;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct World {
    pub token: String,
    /// Any token waits for approve() (the Allow dialog).
    pub asking: bool,
    pub paired: bool,
    pub module: String,
    pub photo: Option<Photo>,
    pub photos: Vec<String>,
    pub values: HashMap<String, f64>,
    pub log: Vec<String>,
    pub delay: Duration,
    pub drop: bool,
}

#[derive(Clone)]
pub struct FakeLightroom {
    pub command_port: u16,
    pub event_port: u16,
    world: Arc<Mutex<World>>,
    events: Arc<Mutex<Option<TcpStream>>>,
}

impl FakeLightroom {
    pub fn start(token: &str) -> Self {
        Self::serve(token, false, 0, 0)
    }
    pub fn start_asking(token: &str) -> Self {
        Self::serve(token, true, 0, 0)
    }

    pub fn serve(token: &str, asking: bool, command_port: u16, event_port: u16) -> Self {
        let cmd = TcpListener::bind(("127.0.0.1", command_port)).expect("command port");
        let evt = TcpListener::bind(("127.0.0.1", event_port)).expect("event port");
        let mut values = HashMap::new();
        for p in params::PARAMS {
            values.insert(p.id.to_string(), if p.min < 0.0 { 0.0 } else { p.min });
        }
        values.insert("Temperature".into(), 5500.0);
        let world = Arc::new(Mutex::new(World {
            token: token.into(),
            asking,
            paired: false,
            module: "develop".into(),
            photo: Some(Photo { name: "IMG_0001.CR3".into(), flag: "none".into(), rating: 0, label: "none".into() }),
            photos: vec!["IMG_0001.CR3".into(), "IMG_0002.CR3".into(), "IMG_0003.CR3".into()],
            values,
            log: vec![],
            delay: Duration::ZERO,
            drop: false,
        }));
        let me = FakeLightroom {
            command_port: cmd.local_addr().unwrap().port(),
            event_port: evt.local_addr().unwrap().port(),
            world,
            events: Arc::new(Mutex::new(None)),
        };
        let ev = me.events.clone();
        std::thread::spawn(move || {
            for s in evt.incoming().flatten() {
                *ev.lock().unwrap() = Some(s);
            }
        });
        let m = me.clone();
        std::thread::spawn(move || {
            for s in cmd.incoming().flatten() {
                m.world.lock().unwrap().paired = false;
                m.serve_commands(s);
            }
        });
        me
    }

    pub fn config(&self, token: &str) -> Config {
        Config { command_port: self.command_port, event_port: self.event_port, token: token.into() }
    }

    pub fn log(&self) -> Vec<String> {
        self.world.lock().unwrap().log.clone()
    }
    pub fn world(&self, f: impl FnOnce(&mut World)) {
        f(&mut self.world.lock().unwrap())
    }
    pub fn set_delay(&self, d: Duration) {
        self.world.lock().unwrap().delay = d;
    }

    /// The user clicks Allow.
    pub fn approve(&self) {
        {
            let mut w = self.world.lock().unwrap();
            w.asking = false;
        }
        self.send(&["E", "paired"]);
    }

    /// Lightroom quits: both sockets close (it restarts listening right away).
    pub fn drop_connection(&self) {
        self.world.lock().unwrap().drop = true;
        if let Some(s) = self.events.lock().unwrap().take() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn wait(&self, ok: impl Fn() -> bool, what: &str) {
        let end = Instant::now() + Duration::from_secs(10);
        while !ok() {
            assert!(Instant::now() < end, "timed out waiting for: {what}; commands: {:?}", self.log());
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn send(&self, fields: &[&str]) {
        let mut line = proto::encode(fields);
        line.push('\n');
        // The event client may connect a moment after the command client.
        for _ in 0..100 {
            if let Some(s) = self.events.lock().unwrap().as_mut() {
                let _ = s.write_all(line.as_bytes());
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn emit_state(&self) {
        let (module, photo, values) = {
            let w = self.world.lock().unwrap();
            (w.module.clone(), w.photo.clone(), w.values.clone())
        };
        self.send(&["E", "module", &module]);
        match photo {
            Some(p) => self.send(&["E", "photo", &p.name, &p.flag, &p.rating.to_string(), &p.label]),
            None => self.send(&["E", "photo"]),
        }
        let mut keys: Vec<_> = values.keys().cloned().collect();
        keys.sort();
        for k in keys {
            self.send(&["E", "value", &k, &proto::num(values[&k])]);
        }
    }

    fn serve_commands(&self, s: TcpStream) {
        let reader = BufReader::new(s.try_clone().unwrap());
        let _ = s.set_read_timeout(Some(Duration::from_millis(100)));
        let mut lines = reader.lines();
        loop {
            if std::mem::take(&mut self.world.lock().unwrap().drop) {
                let _ = s.shutdown(std::net::Shutdown::Both);
                return;
            }
            let line = match lines.next() {
                Some(Ok(l)) => l,
                Some(Err(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                _ => return,
            };
            let f = proto::split(&line);
            if f.len() < 2 {
                continue;
            }
            let (id, cmd, args) = (&f[0], f[1].as_str(), &f[2..]);
            self.world.lock().unwrap().log.push(f[1..].join("\t"));
            let delay = self.world.lock().unwrap().delay;
            std::thread::sleep(delay);
            let reply = self.run(cmd, args);
            match reply {
                Ok(vals) => {
                    let mut out = vec!["R".to_string(), id.clone(), "ok".into()];
                    out.extend(vals);
                    self.send(&out.iter().map(String::as_str).collect::<Vec<_>>());
                    // The plug-in sends what changed after every command; this sends it all.
                    self.emit_state();
                }
                Err(e) => self.send(&["R", id, "err", &e]),
            }
        }
    }

    fn run(&self, cmd: &str, args: &[String]) -> Result<Vec<String>, String> {
        let mut w = self.world.lock().unwrap();
        let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
        if cmd == "hello" {
            if w.asking {
                return Err("pending".into());
            }
            if arg(0) != w.token && !w.token.is_empty() {
                return Err("denied".into());
            }
            w.paired = true;
            return Ok(vec!["1.0.0".into(), "15.1".into(), "1".into()]);
        }
        if !w.paired {
            return Err("not paired".into());
        }
        let need_photo = |w: &World| if w.photo.is_none() { Err("Select a photo in Lightroom first".to_string()) } else { Ok(()) };
        match cmd {
            "watch" => Ok(vec![args.first().map(|a| a.split(',').count()).unwrap_or(0).to_string()]),
            "state" | "undo" | "redo" | "resetall" | "auto" | "panel" | "tool" | "view" | "rotate" => {
                if !matches!(cmd, "state" | "undo" | "redo" | "panel") {
                    need_photo(&w)?;
                }
                Ok(vec![])
            }
            "nudge" | "set" | "reset" | "get" => {
                need_photo(&w)?;
                let p = arg(0);
                let range = params::find(&p).ok_or(format!("Lightroom has no develop setting called {p}"))?;
                w.module = "develop".into();
                let now = *w.values.get(&p).unwrap_or(&0.0);
                let v = match cmd {
                    "nudge" => {
                        let step = arg(2).parse::<f64>().unwrap_or(if p == "Exposure" { 0.05 } else if p == "Temperature" { 50.0 } else { 1.0 });
                        now + arg(1).parse::<f64>().map_err(|_| "Not a number")? * step
                    }
                    "set" => arg(1).parse::<f64>().map_err(|_| "Not a number")?,
                    "reset" => 0.0,
                    _ => now,
                };
                let v = ((v.clamp(range.min, range.max)) * 10000.0).round() / 10000.0;
                w.values.insert(p.clone(), v);
                Ok(vec![p, proto::num(v)])
            }
            "module" => {
                w.module = arg(0);
                Ok(vec![arg(0)])
            }
            "photo" => {
                let i = w.photos.iter().position(|n| Some(n) == w.photo.as_ref().map(|p| &p.name)).unwrap_or(0);
                let j = if arg(0) == "next" { (i + 1).min(w.photos.len() - 1) } else { i.saturating_sub(1) };
                let name = w.photos[j].clone();
                w.photo = Some(Photo { name: name.clone(), flag: "none".into(), rating: 0, label: "none".into() });
                Ok(vec![name, "none".into(), "0".into(), "none".into()])
            }
            "flag" | "rating" | "label" | "copy" | "paste" => {
                need_photo(&w)?;
                let p = w.photo.as_mut().unwrap();
                Ok(vec![match cmd {
                    "flag" => {
                        p.flag = if arg(0) == p.flag { "none".into() } else { arg(0) };
                        p.flag.clone()
                    }
                    "rating" => {
                        p.rating = match arg(0).as_str() {
                            "up" => (p.rating + 1).min(5),
                            "down" => p.rating.saturating_sub(1),
                            n => n.parse().map_err(|_| "A rating is 0 to 5 stars")?,
                        };
                        p.rating.to_string()
                    }
                    "label" => {
                        p.label = if arg(0) == p.label { "none".into() } else { arg(0) };
                        p.label.clone()
                    }
                    "copy" => p.name.clone(),
                    _ => "1".into(),
                }])
            }
            _ => Err(format!("Lightroom plug-in 1.0.0 has no command {cmd}; update the KnobDeck plug-in")),
        }
    }
}
