//! The sim's event IDs and SimVars behind each action (MSFS 2020 / 2024 SDK names).

use crate::values::{Spacing, TrimScale};

/// An on/off system: the K-event that toggles it and the Bool SimVar that says whether it is on.
pub struct Switch {
    pub key: &'static str,
    pub label: &'static str,
    pub event: &'static str,
    pub var: &'static str,
}

const fn sw(key: &'static str, label: &'static str, event: &'static str, var: &'static str) -> Switch {
    Switch { key, label, event, var }
}

pub const AP_MASTER: Switch = sw("ap", "AP", "AP_MASTER", "AUTOPILOT MASTER");

pub const AP_MODES: &[Switch] = &[
    sw("hdg", "HDG", "AP_HDG_HOLD", "AUTOPILOT HEADING LOCK"),
    sw("alt", "ALT", "AP_ALT_HOLD", "AUTOPILOT ALTITUDE LOCK"),
    sw("nav", "NAV", "AP_NAV1_HOLD", "AUTOPILOT NAV1 LOCK"),
    sw("apr", "APR", "AP_APR_HOLD", "AUTOPILOT APPROACH HOLD"),
    sw("vs", "VS", "AP_VS_HOLD", "AUTOPILOT VERTICAL HOLD"),
    sw("flc", "FLC", "FLIGHT_LEVEL_CHANGE", "AUTOPILOT FLIGHT LEVEL CHANGE"),
    sw("ias", "IAS", "AP_AIRSPEED_HOLD", "AUTOPILOT AIRSPEED HOLD"),
    sw("bc", "BC", "AP_BC_HOLD", "AUTOPILOT BACKCOURSE HOLD"),
    sw("yd", "YD", "YAW_DAMPER_TOGGLE", "AUTOPILOT YAW DAMPER"),
    sw("fd", "FD", "TOGGLE_FLIGHT_DIRECTOR", "AUTOPILOT FLIGHT DIRECTOR ACTIVE"),
];

pub const AP_MODE_NAMES: &[(&str, &str)] = &[
    ("hdg", "HDG: heading hold"),
    ("alt", "ALT: altitude hold"),
    ("nav", "NAV: nav hold"),
    ("apr", "APR: approach"),
    ("vs", "VS: vertical speed"),
    ("flc", "FLC: flight level change"),
    ("ias", "IAS: airspeed hold"),
    ("bc", "BC: back course"),
    ("yd", "YD: yaw damper"),
    ("fd", "FD: flight director"),
];

pub const LIGHTS: &[Switch] = &[
    sw("landing", "Landing", "LANDING_LIGHTS_TOGGLE", "LIGHT LANDING"),
    sw("taxi", "Taxi", "TOGGLE_TAXI_LIGHTS", "LIGHT TAXI"),
    sw("nav", "Nav", "TOGGLE_NAV_LIGHTS", "LIGHT NAV"),
    sw("beacon", "Beacon", "TOGGLE_BEACON_LIGHTS", "LIGHT BEACON"),
    sw("strobe", "Strobe", "STROBES_TOGGLE", "LIGHT STROBE"),
    sw("panel", "Panel", "PANEL_LIGHTS_TOGGLE", "LIGHT PANEL"),
    sw("logo", "Logo", "TOGGLE_LOGO_LIGHTS", "LIGHT LOGO"),
    sw("wing", "Wing", "TOGGLE_WING_LIGHTS", "LIGHT WING"),
    sw("recognition", "Recognition", "TOGGLE_RECOGNITION_LIGHTS", "LIGHT RECOGNITION"),
    sw("cabin", "Cabin", "TOGGLE_CABIN_LIGHTS", "LIGHT CABIN"),
];

pub const ELECTRICAL: &[Switch] = &[
    sw("battery", "Battery", "TOGGLE_MASTER_BATTERY", "ELECTRICAL MASTER BATTERY"),
    sw("alternator", "Alternator", "TOGGLE_MASTER_ALTERNATOR", "GENERAL ENG MASTER ALTERNATOR:1"),
    sw("avionics", "Avionics", "TOGGLE_AVIONICS_MASTER", "AVIONICS MASTER SWITCH"),
];

pub const PARKING_BRAKE: Switch = sw("parking", "Parking Brake", "PARKING_BRAKES", "BRAKE PARKING POSITION");
pub const SPOILERS_ARM: Switch = sw("arm", "Spoilers Armed", "SPOILERS_ARM_TOGGLE", "SPOILERS ARMED");

/// Select options for a switch table.
pub fn options(list: &'static [Switch]) -> Vec<(&'static str, &'static str)> {
    list.iter().map(|s| (s.key, s.label)).collect()
}

/// The entry `key` picks, else the first (an instance placed before choosing).
pub fn pick<'a, T: Keyed>(list: &'a [T], key: &str) -> &'a T {
    list.iter().find(|s| s.key() == key.trim()).unwrap_or(&list[0])
}

pub trait Keyed {
    fn key(&self) -> &str;
}

impl Keyed for Switch {
    fn key(&self) -> &str {
        self.key
    }
}

/// A value the autopilot holds or a navigation setting: set by an event that takes the value.
pub struct Bug {
    pub key: &'static str,
    pub short: &'static str,
    pub event: &'static str,
    pub var: &'static str,
    pub unit: &'static str,
    /// A heading / course (wraps at 360) rather than a clamped value.
    pub degrees: bool,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub suffix: &'static str,
}

impl Keyed for Bug {
    fn key(&self) -> &str {
        self.key
    }
}

pub const HEADING: Bug = Bug { key: "hdg", short: "HDG", event: "HEADING_BUG_SET", var: "AUTOPILOT HEADING LOCK DIR", unit: "degrees", degrees: true, min: 0.0, max: 360.0, step: 1.0, suffix: "" };
pub const ALTITUDE: Bug = Bug { key: "alt", short: "ALT", event: "AP_ALT_VAR_SET_ENGLISH", var: "AUTOPILOT ALTITUDE LOCK VAR", unit: "feet", degrees: false, min: 0.0, max: 45000.0, step: 100.0, suffix: " ft" };
pub const VERTICAL_SPEED: Bug = Bug { key: "vs", short: "VS", event: "AP_VS_VAR_SET_ENGLISH", var: "AUTOPILOT VERTICAL HOLD VAR", unit: "feet per minute", degrees: false, min: -6000.0, max: 6000.0, step: 100.0, suffix: " fpm" };
pub const SPEED: Bug = Bug { key: "spd", short: "SPD", event: "AP_SPD_VAR_SET", var: "AUTOPILOT AIRSPEED HOLD VAR", unit: "knots", degrees: false, min: 0.0, max: 450.0, step: 1.0, suffix: " kt" };

pub const COURSES: &[Bug] = &[
    Bug { key: "nav1", short: "CRS1", event: "VOR1_SET", var: "NAV OBS:1", unit: "degrees", degrees: true, min: 0.0, max: 360.0, step: 1.0, suffix: "" },
    Bug { key: "nav2", short: "CRS2", event: "VOR2_SET", var: "NAV OBS:2", unit: "degrees", degrees: true, min: 0.0, max: 360.0, step: 1.0, suffix: "" },
    Bug { key: "gps", short: "OBS", event: "GPS_OBS_SET", var: "GPS OBS VALUE", unit: "degrees", degrees: true, min: 0.0, max: 360.0, step: 1.0, suffix: "" },
];

/// A COM or NAV radio.
pub struct Radio {
    pub key: &'static str,
    pub short: &'static str,
    /// Sets the standby frequency in Hz.
    pub set_hz: &'static str,
    pub swap: &'static str,
    pub active: &'static str,
    pub standby: &'static str,
    pub nav: bool,
}

impl Keyed for Radio {
    fn key(&self) -> &str {
        self.key
    }
}

impl Radio {
    pub fn spacing(&self, com_833: bool) -> Spacing {
        match (self.nav, com_833) {
            (true, _) => Spacing::Nav50,
            (false, true) => Spacing::Com833,
            (false, false) => Spacing::Com25,
        }
    }
}

pub const FREQ_UNIT: &str = "MHz";

pub const COMS: &[Radio] = &[
    Radio { key: "com1", short: "COM1", set_hz: "COM_STBY_RADIO_SET_HZ", swap: "COM1_RADIO_SWAP", active: "COM ACTIVE FREQUENCY:1", standby: "COM STANDBY FREQUENCY:1", nav: false },
    Radio { key: "com2", short: "COM2", set_hz: "COM2_STBY_RADIO_SET_HZ", swap: "COM2_RADIO_SWAP", active: "COM ACTIVE FREQUENCY:2", standby: "COM STANDBY FREQUENCY:2", nav: false },
];

pub const NAVS: &[Radio] = &[
    Radio { key: "nav1", short: "NAV1", set_hz: "NAV1_STBY_SET_HZ", swap: "NAV1_RADIO_SWAP", active: "NAV ACTIVE FREQUENCY:1", standby: "NAV STANDBY FREQUENCY:1", nav: true },
    Radio { key: "nav2", short: "NAV2", set_hz: "NAV2_STBY_SET_HZ", swap: "NAV2_RADIO_SWAP", active: "NAV ACTIVE FREQUENCY:2", standby: "NAV STANDBY FREQUENCY:2", nav: true },
];

/// Every radio, for the swap key.
pub fn radios() -> Vec<&'static Radio> {
    COMS.iter().chain(NAVS.iter()).collect()
}

pub const XPDR_VAR: &str = "TRANSPONDER CODE:1";
/// The transponder code SimVar's unit: binary-coded, one nibble per digit.
pub const XPDR_UNIT: &str = "Bco16";

pub struct Trim {
    pub key: &'static str,
    pub label: &'static str,
    pub event: &'static str,
    pub var: &'static str,
    pub scale: TrimScale,
}

impl Keyed for Trim {
    fn key(&self) -> &str {
        self.key
    }
}

pub const TRIMS: &[Trim] = &[
    Trim { key: "elevator", label: "Elevator", event: "ELEVATOR_TRIM_SET", var: "ELEVATOR TRIM PCT", scale: TrimScale::Full },
    Trim { key: "aileron", label: "Aileron", event: "AILERON_TRIM_SET", var: "AILERON TRIM PCT", scale: TrimScale::Percent },
    Trim { key: "rudder", label: "Rudder", event: "RUDDER_TRIM_SET", var: "RUDDER TRIM PCT", scale: TrimScale::Percent },
];

pub const BARO_VAR: &str = "KOHLSMAN SETTING MB:1";
pub const BARO_UNIT: &str = "millibars";

/// How a readout shows its value.
#[derive(Clone, Copy)]
pub enum Show {
    Whole(&'static str),
    Signed(&'static str),
    Heading,
    Rate,
}

pub struct Readout {
    pub key: &'static str,
    pub label: &'static str,
    pub short: &'static str,
    pub var: &'static str,
    pub unit: &'static str,
    pub show: Show,
}

impl Keyed for Readout {
    fn key(&self) -> &str {
        self.key
    }
}

/// Fuel is computed from two SimVars, wind from two; see `main.rs`.
pub const READOUTS: &[Readout] = &[
    Readout { key: "ias", label: "Indicated airspeed", short: "IAS", var: "AIRSPEED INDICATED", unit: "knots", show: Show::Whole(" kt") },
    Readout { key: "tas", label: "True airspeed", short: "TAS", var: "AIRSPEED TRUE", unit: "knots", show: Show::Whole(" kt") },
    Readout { key: "gs", label: "Ground speed", short: "GS", var: "GROUND VELOCITY", unit: "knots", show: Show::Whole(" kt") },
    Readout { key: "alt", label: "Altitude (indicated)", short: "ALT", var: "INDICATED ALTITUDE", unit: "feet", show: Show::Whole(" ft") },
    Readout { key: "agl", label: "Height above ground", short: "AGL", var: "PLANE ALT ABOVE GROUND", unit: "feet", show: Show::Whole(" ft") },
    Readout { key: "vs", label: "Vertical speed", short: "VS", var: "VERTICAL SPEED", unit: "feet per minute", show: Show::Signed(" fpm") },
    Readout { key: "hdg", label: "Heading (magnetic)", short: "HDG", var: "PLANE HEADING DEGREES MAGNETIC", unit: "degrees", show: Show::Heading },
    Readout { key: "fuel", label: "Fuel (% of capacity)", short: "FUEL", var: "FUEL TOTAL QUANTITY", unit: "gallons", show: Show::Whole("%") },
    Readout { key: "rate", label: "Sim rate", short: "RATE", var: "SIMULATION RATE", unit: "number", show: Show::Rate },
    Readout { key: "oat", label: "Outside air temperature", short: "OAT", var: "AMBIENT TEMPERATURE", unit: "celsius", show: Show::Signed(" °C") },
    Readout { key: "wind", label: "Wind", short: "WIND", var: "AMBIENT WIND DIRECTION", unit: "degrees", show: Show::Heading },
];

pub const FUEL_CAPACITY: &str = "FUEL TOTAL CAPACITY";
pub const WIND_SPEED: &str = "AMBIENT WIND VELOCITY";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{event_name, var_key};

    /// Every name in the tables passes the same checks a user's custom one does.
    #[test]
    fn names_are_valid() {
        let switches = AP_MODES.iter().chain(LIGHTS).chain(ELECTRICAL).chain([&AP_MASTER, &PARKING_BRAKE, &SPOILERS_ARM]);
        for s in switches {
            assert_eq!(event_name(s.event).unwrap(), s.event);
            assert_eq!(var_key(s.var, "bool").unwrap().name, s.var);
        }
        for b in COURSES.iter().chain([&HEADING, &ALTITUDE, &VERTICAL_SPEED, &SPEED]) {
            assert_eq!(event_name(b.event).unwrap(), b.event);
            assert!(var_key(b.var, b.unit).is_ok());
            assert!(b.min < b.max && b.step > 0.0);
        }
        for r in radios() {
            for e in [r.set_hz, r.swap] {
                assert_eq!(event_name(e).unwrap(), e);
            }
            assert!(var_key(r.active, FREQ_UNIT).is_ok() && var_key(r.standby, FREQ_UNIT).is_ok());
        }
        for t in TRIMS {
            assert_eq!(event_name(t.event).unwrap(), t.event);
            assert!(var_key(t.var, "percent").is_ok());
        }
        for r in READOUTS {
            assert!(var_key(r.var, r.unit).is_ok());
        }
        assert!(var_key(XPDR_VAR, XPDR_UNIT).is_ok() && var_key(BARO_VAR, BARO_UNIT).is_ok());
    }

    #[test]
    fn mode_names_cover_the_table() {
        assert_eq!(AP_MODE_NAMES.len(), AP_MODES.len());
        for (k, _) in AP_MODE_NAMES {
            assert_eq!(pick(AP_MODES, k).key, *k);
        }
        assert_eq!(pick(LIGHTS, "nope").key, "landing");
        assert_eq!(pick(COMS, "com2").set_hz, "COM2_STBY_RADIO_SET_HZ");
        assert_eq!(pick(NAVS, "nav2").spacing(false), Spacing::Nav50);
        assert_eq!(pick(COMS, "com1").spacing(true), Spacing::Com833);
    }
}
