//! What each Voicemeeter edition has: its input strips, output buses and the routing buttons on
//! every strip. Parameter names are the Remote API's (`Strip[0].Mute`, `Bus[3].Gain`, `Strip[2].B1`).

/// The edition running (`VBVMR_GetVoicemeeterType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edition {
    Standard,
    Banana,
    Potato,
}

impl Edition {
    pub fn from_type(t: i32) -> Option<Edition> {
        match t {
            1 => Some(Edition::Standard),
            2 => Some(Edition::Banana),
            3 => Some(Edition::Potato),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Edition::Standard => "Voicemeeter",
            Edition::Banana => "Voicemeeter Banana",
            Edition::Potato => "Voicemeeter Potato",
        }
    }

    /// (hardware inputs, virtual inputs).
    fn inputs(self) -> (usize, usize) {
        match self {
            Edition::Standard => (2, 1),
            Edition::Banana => (3, 2),
            Edition::Potato => (5, 3),
        }
    }

    /// (hardware outputs A…, virtual outputs B…).
    fn outputs(self) -> (usize, usize) {
        match self {
            Edition::Standard => (1, 1),
            Edition::Banana => (3, 2),
            Edition::Potato => (5, 3),
        }
    }

    pub fn strip_count(self) -> usize {
        let (h, v) = self.inputs();
        h + v
    }

    pub fn bus_count(self) -> usize {
        let (a, b) = self.outputs();
        a + b
    }

    /// The routing buttons on a strip, which are also the bus names: `A1`… then `B1`….
    pub fn outputs_names(self) -> Vec<String> {
        let (a, b) = self.outputs();
        (1..=a).map(|i| format!("A{i}")).chain((1..=b).map(|i| format!("B{i}"))).collect()
    }

    /// A strip's name when the user gave it no label.
    pub fn strip_default(self, i: usize) -> String {
        let (h, _) = self.inputs();
        if i < h {
            format!("Hardware Input {}", i + 1)
        } else {
            match i - h {
                0 => "Voicemeeter VAIO".into(),
                1 => "Voicemeeter AUX".into(),
                n => format!("Voicemeeter VAIO{}", n + 1),
            }
        }
    }

    /// A bus's name when the user gave it no label (`A1`, `B2`).
    pub fn bus_default(self, i: usize) -> String {
        self.outputs_names().get(i).cloned().unwrap_or_else(|| format!("Bus {}", i + 1))
    }
}

/// `Strip[2].Mute`, `Bus[0].Gain`.
pub fn param(target: Target, field: &str) -> String {
    match target {
        Target::Strip(i) => format!("Strip[{i}].{field}"),
        Target::Bus(i) => format!("Bus[{i}].{field}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Strip(usize),
    Bus(usize),
}

/// Gain in dB, as on Voicemeeter's faders.
pub const GAIN_MIN: f64 = -60.0;
pub const GAIN_MAX: f64 = 12.0;

/// The gain a nudge or a set lands on: clamped to the fader, rounded to 0.1 dB.
pub fn next_gain(current: f64, value: Option<f64>, delta: f64, step: f64) -> f64 {
    let target = value.unwrap_or(current + delta * step);
    (target.clamp(GAIN_MIN, GAIN_MAX) * 10.0).round() / 10.0
}

/// A routing button name the edition has (`A1`, `b2` → `B2`).
pub fn routing(edition: Edition, name: &str) -> Option<String> {
    let n = name.trim().to_ascii_uppercase();
    edition.outputs_names().into_iter().find(|o| *o == n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editions() {
        assert_eq!(Edition::Standard.outputs_names(), ["A1", "B1"]);
        assert_eq!(Edition::Banana.outputs_names(), ["A1", "A2", "A3", "B1", "B2"]);
        assert_eq!(Edition::Potato.outputs_names(), ["A1", "A2", "A3", "A4", "A5", "B1", "B2", "B3"]);
        assert_eq!((Edition::Potato.strip_count(), Edition::Potato.bus_count()), (8, 8));
        assert_eq!(Edition::Banana.strip_default(2), "Hardware Input 3");
        assert_eq!(Edition::Banana.strip_default(4), "Voicemeeter AUX");
        assert_eq!(Edition::Potato.strip_default(7), "Voicemeeter VAIO3");
        assert_eq!(Edition::Banana.bus_default(3), "B1");
        assert_eq!(Edition::from_type(2), Some(Edition::Banana));
        assert_eq!(Edition::from_type(-2), None);
    }

    #[test]
    fn params_and_gain() {
        assert_eq!(param(Target::Strip(2), "B1"), "Strip[2].B1");
        assert_eq!(param(Target::Bus(0), "Gain"), "Bus[0].Gain");
        assert_eq!(next_gain(0.0, None, 3.0, 1.5), 4.5);
        assert_eq!(next_gain(10.0, None, 5.0, 1.0), 12.0);
        assert_eq!(next_gain(0.0, Some(-80.0), 0.0, 1.0), -60.0);
        assert_eq!(next_gain(-6.04, None, 0.0, 1.0), -6.0);
        assert_eq!(routing(Edition::Banana, " b2"), Some("B2".into()));
        assert_eq!(routing(Edition::Standard, "A2"), None);
    }
}
