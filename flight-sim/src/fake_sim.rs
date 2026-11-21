//! A pretend Flight Simulator for driving the built plugin over its stdio protocol on a machine
//! without the sim (`KNOBDECK_FAKE_SIM=1`, non-Windows builds only; never shipped). It keeps a
//! small model of an aircraft, applies the K-events the plugin sends to it, raises SimConnect's
//! "name not recognized" exception for unknown events and SimVars, and sends watched SimVars when
//! they change, like SIMCONNECT_PERIOD_SIM_FRAME with the CHANGED flag.

use crate::catalog;
use crate::engine::PAUSE_EVENT;
use crate::sim::{Connector, Link, LinkError, OpenError, Rate, Recv};
use std::collections::{HashMap, VecDeque};

pub struct FakeSim;

impl Connector for FakeSim {
    fn open(&mut self) -> Result<Box<dyn Link>, OpenError> {
        let mut vars: HashMap<String, f64> = HashMap::new();
        for s in catalog::AP_MODES.iter().chain(catalog::LIGHTS).chain(catalog::ELECTRICAL).chain([&catalog::AP_MASTER, &catalog::PARKING_BRAKE, &catalog::SPOILERS_ARM]) {
            vars.insert(s.var.into(), 0.0);
        }
        for (k, v) in [
            ("AUTOPILOT HEADING LOCK DIR", 90.0),
            ("AUTOPILOT ALTITUDE LOCK VAR", 5000.0),
            ("AUTOPILOT VERTICAL HOLD VAR", 0.0),
            ("AUTOPILOT AIRSPEED HOLD VAR", 110.0),
            ("NAV OBS:1", 0.0),
            ("NAV OBS:2", 180.0),
            ("GPS OBS VALUE", 45.0),
            ("COM ACTIVE FREQUENCY:1", 121.5),
            ("COM STANDBY FREQUENCY:1", 118.0),
            ("COM ACTIVE FREQUENCY:2", 122.8),
            ("COM STANDBY FREQUENCY:2", 119.1),
            ("NAV ACTIVE FREQUENCY:1", 110.5),
            ("NAV STANDBY FREQUENCY:1", 113.9),
            ("NAV ACTIVE FREQUENCY:2", 109.3),
            ("NAV STANDBY FREQUENCY:2", 112.0),
            (catalog::XPDR_VAR, 0x1200 as f64),
            ("IS GEAR RETRACTABLE", 1.0),
            ("GEAR HANDLE POSITION", 1.0),
            ("GEAR TOTAL PCT EXTENDED", 100.0),
            ("FLAPS NUM HANDLE POSITIONS", 4.0),
            ("FLAPS HANDLE INDEX", 0.0),
            ("SPOILERS HANDLE POSITION", 0.0),
            ("ELEVATOR TRIM PCT", 0.0),
            ("AILERON TRIM PCT", 0.0),
            ("RUDDER TRIM PCT", 0.0),
            (catalog::BARO_VAR, 1013.25),
            ("SIMULATION RATE", 1.0),
            ("AIRSPEED INDICATED", 120.4),
            ("AIRSPEED TRUE", 126.0),
            ("GROUND VELOCITY", 118.0),
            ("INDICATED ALTITUDE", 3500.0),
            ("PLANE ALT ABOVE GROUND", 2900.0),
            ("VERTICAL SPEED", -350.0),
            ("PLANE HEADING DEGREES MAGNETIC", 87.6),
            ("FUEL TOTAL QUANTITY", 40.0),
            (catalog::FUEL_CAPACITY, 53.0),
            ("AMBIENT TEMPERATURE", 12.0),
            ("AMBIENT WIND DIRECTION", 270.0),
            (catalog::WIND_SPEED, 12.0),
            ("PLANE ALTITUDE", 3512.0),
        ] {
            vars.insert(k.into(), v);
        }
        let mut inbox = VecDeque::new();
        inbox.push_back(Recv::Open { app: "FakeSim".into() });
        Ok(Box::new(World { vars, inbox, ..Default::default() }))
    }
}

#[derive(Default)]
struct World {
    vars: HashMap<String, f64>,
    events: HashMap<u32, String>,
    defs: HashMap<u32, String>,
    /// Watched definitions and the value last sent for each.
    watched: HashMap<u32, Option<f64>>,
    inbox: VecDeque<Recv>,
    send_id: u32,
    paused: bool,
}

impl World {
    fn sent(&mut self) -> u32 {
        self.send_id += 1;
        self.send_id
    }

    fn toggle(&mut self, var: &str) {
        let v = self.vars.entry(var.into()).or_default();
        *v = if *v >= 0.5 { 0.0 } else { 1.0 };
    }

    fn set(&mut self, var: &str, v: f64) {
        self.vars.insert(var.into(), v);
    }

    /// Apply an event; false when the sim wouldn't know it.
    fn apply(&mut self, name: &str, d: [u32; 5]) -> bool {
        let signed = d[0] as i32 as f64;
        let switches = catalog::AP_MODES.iter().chain(catalog::LIGHTS).chain(catalog::ELECTRICAL).chain([&catalog::AP_MASTER, &catalog::PARKING_BRAKE, &catalog::SPOILERS_ARM]);
        if let Some(s) = switches.into_iter().find(|s| s.event == name) {
            self.toggle(s.var);
            return true;
        }
        if let Some(b) = catalog::COURSES.iter().chain([&catalog::HEADING, &catalog::ALTITUDE, &catalog::VERTICAL_SPEED, &catalog::SPEED]).find(|b| b.event == name) {
            self.set(b.var, signed);
            return true;
        }
        for r in catalog::radios() {
            if r.set_hz == name {
                self.set(r.standby, d[0] as f64 / 1e6);
                return true;
            }
            if r.swap == name {
                let (a, s) = (self.vars[r.active], self.vars[r.standby]);
                self.set(r.active, s);
                self.set(r.standby, a);
                return true;
            }
        }
        if let Some(t) = catalog::TRIMS.iter().find(|t| t.event == name) {
            let pct = match t.scale {
                crate::values::TrimScale::Full => signed / 16383.0 * 100.0,
                crate::values::TrimScale::Percent => signed,
            };
            self.set(t.var, pct);
            return true;
        }
        let flaps = self.vars["FLAPS HANDLE INDEX"];
        let n = self.vars["FLAPS NUM HANDLE POSITIONS"];
        match name {
            "XPNDR_SET" => self.set(catalog::XPDR_VAR, d[0] as f64),
            "GEAR_TOGGLE" => {
                self.toggle("GEAR HANDLE POSITION");
                let h = self.vars["GEAR HANDLE POSITION"];
                self.set("GEAR TOTAL PCT EXTENDED", h * 100.0);
            }
            "FLAPS_INCR" => self.set("FLAPS HANDLE INDEX", (flaps + 1.0).min(n)),
            "FLAPS_DECR" => self.set("FLAPS HANDLE INDEX", (flaps - 1.0).max(0.0)),
            "FLAPS_UP" => self.set("FLAPS HANDLE INDEX", 0.0),
            "FLAPS_DOWN" => self.set("FLAPS HANDLE INDEX", n),
            "FLAPS_SET" => self.set("FLAPS HANDLE INDEX", (d[0] as f64 / 16383.0 * n).round()),
            "SPOILERS_SET" => self.set("SPOILERS HANDLE POSITION", d[0] as f64 / 16383.0 * 100.0),
            "KOHLSMAN_SET" => self.set(catalog::BARO_VAR, d[0] as f64 / 16.0),
            "BAROMETRIC" => self.set(catalog::BARO_VAR, 1008.0),
            "SIM_RATE_INCR" => self.set("SIMULATION RATE", (self.vars["SIMULATION RATE"] * 2.0).min(128.0)),
            "SIM_RATE_DECR" => self.set("SIMULATION RATE", (self.vars["SIMULATION RATE"] / 2.0).max(0.25)),
            "PAUSE_TOGGLE" => {
                self.paused = !self.paused;
                self.inbox.push_back(Recv::Event { id: PAUSE_EVENT, data: self.paused as u32 });
            }
            "TOGGLE_FUEL_VALVE_ALL" => {}
            _ => return false,
        }
        true
    }
}

impl Link for World {
    fn map_event(&mut self, id: u32, name: &str) -> Result<u32, LinkError> {
        let send = self.sent();
        self.events.insert(id, name.into());
        if !self.apply_probe(name) {
            self.inbox.push_back(Recv::Exception { code: 7, send_id: send });
        }
        Ok(send)
    }
    fn transmit(&mut self, id: u32, data: [u32; 5]) -> Result<(), LinkError> {
        let name = self.events.get(&id).cloned().ok_or_else(|| LinkError::Failed("unmapped event".into()))?;
        self.apply(&name, data);
        Ok(())
    }
    fn add_var(&mut self, def: u32, name: &str, _unit: &str) -> Result<u32, LinkError> {
        let send = self.sent();
        if !self.vars.contains_key(name) {
            self.inbox.push_back(Recv::Exception { code: 7, send_id: send });
        }
        self.defs.insert(def, name.into());
        Ok(send)
    }
    fn request(&mut self, def: u32, rate: Option<Rate>) -> Result<(), LinkError> {
        match rate {
            Some(_) => self.watched.insert(def, None),
            None => self.watched.remove(&def),
        };
        Ok(())
    }
    fn clear_var(&mut self, def: u32) -> Result<(), LinkError> {
        self.defs.remove(&def);
        Ok(())
    }
    fn subscribe(&mut self, _id: u32, _name: &str) -> Result<(), LinkError> {
        Ok(())
    }
    fn next(&mut self) -> Result<Option<Recv>, LinkError> {
        if let Some(r) = self.inbox.pop_front() {
            return Ok(Some(r));
        }
        for (def, last) in self.watched.iter_mut() {
            let Some(v) = self.defs.get(def).and_then(|n| self.vars.get(n)).copied() else { continue };
            if *last != Some(v) {
                *last = Some(v);
                return Ok(Some(Recv::Data { request: *def, value: v }));
            }
        }
        Ok(None)
    }
}

impl World {
    /// Whether the sim knows an event name (applied to a throwaway copy of the aircraft).
    fn apply_probe(&self, name: &str) -> bool {
        let mut probe = World { vars: self.vars.clone(), ..Default::default() };
        probe.apply(name, [0; 5])
    }
}
