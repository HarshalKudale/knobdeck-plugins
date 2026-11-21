//! What each placed key shows, from the Remote Script's last `state` event (pure, so it is tested
//! without Live). `Null` state means Live isn't linked.

use knobdeck_sdk::Value;

/// The volume dials' floor: Live's -inf shows as this on the dial.
pub const MIN_DB: f64 = -70.0;
pub const MAX_DB: f64 = 6.0;
pub const PARAMS_PER_BANK: usize = 8;

/// Song switches: (action id, switch name in the Remote Script's `toggle` request and state).
pub const SWITCHES: &[(&str, &str)] = &[
    ("record", "record"),
    ("session-record", "session_record"),
    ("overdub", "overdub"),
    ("metronome", "metronome"),
    ("loop", "loop"),
    ("punch-in", "punch_in"),
    ("punch-out", "punch_out"),
    ("automation-arm", "automation_arm"),
];

/// Every action whose keys show something from the state.
pub const DYNAMIC: &[&str] = &[
    "play-stop",
    "record",
    "session-record",
    "overdub",
    "metronome",
    "loop",
    "punch-in",
    "punch-out",
    "automation-arm",
    "tempo",
    "tap-tempo",
    "master-volume",
    "track-volume",
    "track-pan",
    "mute",
    "solo",
    "arm",
    "selected-track",
    "launch-scene",
    "select-track",
    "select-scene",
    "device",
    "device-on",
    "device-parameter",
    "selected-parameter",
    "previous-bank",
    "next-bank",
];

pub fn switch_of(action: &str) -> Option<&'static str> {
    SWITCHES.iter().find(|(id, _)| *id == action).map(|(_, s)| *s)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Shown {
    pub state: Option<String>,
    pub label: Option<String>,
}

impl Shown {
    fn state(v: impl ToString) -> Self {
        Shown { state: Some(v.to_string()), label: None }
    }
    fn label(l: impl ToString) -> Self {
        Shown { state: None, label: Some(l.to_string()) }
    }
    fn both(v: impl ToString, l: impl ToString) -> Self {
        Shown { state: Some(v.to_string()), label: Some(l.to_string()) }
    }
}

pub fn on_off(v: bool) -> &'static str {
    if v {
        "on"
    } else {
        "off"
    }
}

/// Rounded for a key: whole numbers without decimals, others to 2 places.
pub fn fmt(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        format!("{r}")
    }
}

fn input<'a>(inputs: &'a Value, key: &str, default: &'a str) -> &'a str {
    match inputs[key].as_str().map(str::trim) {
        Some(s) if !s.is_empty() => s,
        _ => default,
    }
}

pub fn track_pick(inputs: &Value) -> &str {
    input(inputs, "track", "selected")
}

pub fn scene_pick(inputs: &Value) -> &str {
    input(inputs, "scene", "selected")
}

/// A track's state entry from a picker value ("selected", "master", "#3" or a name).
pub fn find_track<'a>(s: &'a Value, pick: &str) -> Option<&'a Value> {
    match pick {
        "selected" => s.get("selected_track"),
        "master" => s.get("master"),
        p if p.starts_with('#') => {
            let n: usize = p[1..].parse().ok()?;
            s["tracks"].as_array()?.get(n.checked_sub(1)?)
        }
        p => s["tracks"].as_array().into_iter().chain(s["returns"].as_array()).flatten().find(|t| t["name"].as_str() == Some(p)),
    }
}

fn track_name(pick: &str, t: &Value) -> String {
    if pick == "master" {
        "Master".into()
    } else {
        t["name"].as_str().unwrap_or("").to_string()
    }
}

/// The dial position for a level: its dB, the floor for -inf.
pub fn db_state(level: &Value) -> f64 {
    level["db"].as_f64().unwrap_or(MIN_DB).clamp(MIN_DB, MAX_DB)
}

/// The bank the device dials follow: `inputs.bank` ("follow" or 1…16), else the shared one.
pub fn bank_of(inputs: &Value, shared: usize) -> usize {
    match input(inputs, "bank", "follow") {
        "follow" => shared,
        b => b.parse::<usize>().ok().filter(|b| *b >= 1).map(|b| b - 1).unwrap_or(shared),
    }
}

pub fn slot_of(inputs: &Value) -> usize {
    input(inputs, "slot", "1").parse::<usize>().ok().filter(|s| (1..=PARAMS_PER_BANK).contains(s)).unwrap_or(1)
}

/// The index (after Device On) a device-parameter key controls.
pub fn param_index(inputs: &Value, shared_bank: usize) -> usize {
    bank_of(inputs, shared_bank) * PARAMS_PER_BANK + slot_of(inputs) - 1
}

/// How many banks of 8 the device's parameters fill.
pub fn bank_count(s: &Value) -> usize {
    let n = s["device"]["params"].as_array().map(Vec::len).unwrap_or(0);
    n.div_ceil(PARAMS_PER_BANK).max(1)
}

/// Identifies the controlled device, so the shared bank goes back to 1 when it changes.
pub fn device_key(s: &Value) -> String {
    format!("{}\u{1f}{}", s["device"]["track"].as_str().unwrap_or(""), s["device"]["name"].as_str().unwrap_or(""))
}

fn param_shown(p: &Value) -> Shown {
    let name = p["name"].as_str().unwrap_or("");
    let text = p["display"].as_str().unwrap_or("");
    match p["value"].as_f64() {
        Some(v) => Shown::both(fmt(v), format!("{name}\n{text}")),
        None => Shown::label(format!("{name}\n{text}")),
    }
}

fn scene_label(name: &str, index: Option<u64>) -> String {
    match (name.trim(), index) {
        ("", Some(i)) => format!("Scene {}", i + 1),
        ("", None) => "Scene".into(),
        (n, _) => n.to_string(),
    }
}

/// What a key of `action` with `inputs` shows for state `s`; `None` when it shows nothing live.
pub fn shown(action: &str, inputs: &Value, s: &Value, shared_bank: usize) -> Option<Shown> {
    if s.is_null() {
        return match action {
            "selected-track" | "device" => Some(Shown::label("Live offline")),
            "play-stop" => Some(Shown::state("stopped")),
            a if switch_of(a).is_some() || matches!(a, "mute" | "solo" | "arm" | "device-on") => Some(Shown::state("off")),
            _ => None,
        };
    }
    if let Some(switch) = switch_of(action) {
        return s[switch].as_bool().map(|v| Shown::state(on_off(v)));
    }
    match action {
        "play-stop" => Some(Shown::state(if s["playing"].as_bool().unwrap_or(false) { "playing" } else { "stopped" })),
        "tempo" => s["tempo"].as_f64().map(|t| Shown::state(fmt(t))),
        "tap-tempo" => s["tempo"].as_f64().map(|t| Shown::label(format!("Tap Tempo\n{} BPM", fmt(t)))),
        "master-volume" => {
            let v = &s["master"]["volume"];
            Some(Shown::both(fmt(db_state(v)), format!("Master\n{}", v["display"].as_str().unwrap_or(""))))
        }
        "track-volume" | "track-pan" => {
            let pick = track_pick(inputs);
            let Some(t) = find_track(s, pick) else {
                return Some(Shown::label(format!("{pick}\nnot in this set")));
            };
            let name = track_name(pick, t);
            if action == "track-volume" {
                let v = &t["volume"];
                Some(Shown::both(fmt(db_state(v)), format!("{name}\n{}", v["display"].as_str().unwrap_or(""))))
            } else {
                let p = &t["pan"];
                Some(Shown::both(fmt(p["value"].as_f64().unwrap_or(0.0)), format!("{name}\n{}", p["display"].as_str().unwrap_or(""))))
            }
        }
        "mute" | "solo" | "arm" => {
            let pick = track_pick(inputs);
            let Some(t) = find_track(s, pick) else {
                return Some(Shown { state: Some("off".into()), label: Some(format!("{pick}\nnot in this set")) });
            };
            let name = track_name(pick, t);
            Some(Shown { state: Some(on_off(t[action].as_bool().unwrap_or(false)).into()), label: Some(name) })
        }
        "selected-track" => Some(Shown::label(match s.get("selected_track") {
            Some(t) => format!("{}\n{}", t["name"].as_str().unwrap_or(""), t["volume"]["display"].as_str().unwrap_or("")),
            None => "No track selected".into(),
        })),
        "launch-scene" => {
            let pick = scene_pick(inputs);
            Some(Shown::label(match pick {
                "selected" => match s.get("selected_scene") {
                    Some(sc) => format!("Launch\n{}", scene_label(sc["name"].as_str().unwrap_or(""), sc["index"].as_u64())),
                    None => "Launch Scene".into(),
                },
                p => match p.strip_prefix('#').and_then(|n| n.parse::<u64>().ok()) {
                    Some(n) => format!("Launch\n{}", scene_label(s["scenes"][(n.max(1) - 1) as usize].as_str().unwrap_or(""), Some(n - 1))),
                    None => format!("Launch\n{p}"),
                },
            }))
        }
        "select-track" => s.get("selected_track").map(|t| match t["index"].as_u64() {
            // Tracks then returns, as the dial counts them.
            Some(i) => {
                let offset = if t["kind"] == "return" { s["tracks"].as_array().map(Vec::len).unwrap_or(0) as u64 } else { 0 };
                Shown::both(i + offset + 1, t["name"].as_str().unwrap_or(""))
            }
            None => Shown::label(t["name"].as_str().unwrap_or("")),
        }),
        "select-scene" => s.get("selected_scene").map(|sc| {
            let i = sc["index"].as_u64();
            let label = scene_label(sc["name"].as_str().unwrap_or(""), i);
            match i {
                Some(i) => Shown::both(i + 1, label),
                None => Shown::label(label),
            }
        }),
        "device" => Some(Shown::label(match s.get("device") {
            Some(d) => match d["track"].as_str() {
                Some(t) => format!("{t}\n{}", d["name"].as_str().unwrap_or("")),
                None => d["name"].as_str().unwrap_or("").to_string(),
            },
            None => "No device".into(),
        })),
        "device-on" => s.get("device").map(|d| Shown::state(on_off(d["on"].as_bool().unwrap_or(false)))),
        "device-parameter" => {
            let Some(d) = s.get("device") else { return Some(Shown::label("No device")) };
            let i = param_index(inputs, shared_bank);
            match d["params"].get(i) {
                Some(p) => Some(param_shown(p)),
                None => Some(Shown::label(format!("{}\nno parameter {}", d["name"].as_str().unwrap_or(""), i + 1))),
            }
        }
        "selected-parameter" => Some(match s.get("selected_parameter") {
            Some(p) => param_shown(p),
            None => Shown::label("No parameter\nselected"),
        }),
        "previous-bank" | "next-bank" => {
            s.get("device")?;
            Some(Shown::label(format!("Bank {}/{}", shared_bank.min(bank_count(s) - 1) + 1, bank_count(s))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    fn state() -> Value {
        json!({
            "playing": true, "tempo": 123.5, "record": false, "session_record": true, "overdub": false, "metronome": true,
            "loop": false, "punch_in": false, "punch_out": false, "automation_arm": true,
            "master": {"volume": {"value": 0.85, "db": 0.0, "display": "0.0 dB"}, "pan": {"value": 0.0, "display": "C"}},
            "tracks": [
                {"name": "Drums", "mute": true, "solo": false, "arm": false, "volume": {"db": -6.0, "display": "-6.0 dB"}, "pan": {"value": -10.0, "display": "10L"}},
                {"name": "Bass", "mute": false, "solo": true, "arm": true, "volume": {"db": null, "display": "-inf dB"}, "pan": {"value": 0.0, "display": "C"}}
            ],
            "returns": [{"name": "A-Reverb", "mute": false, "solo": false, "volume": {"db": 2.0, "display": "2.0 dB"}, "pan": {"value": 0.0, "display": "C"}}],
            "scenes": ["Intro", "", "Drop"],
            "selected_track": {"name": "A-Reverb", "kind": "return", "index": 0, "volume": {"db": 2.0, "display": "2.0 dB"}, "pan": {"value": 0.0, "display": "C"}, "mute": false, "solo": false, "arm": null},
            "selected_scene": {"name": "", "index": 1},
            "device": {"name": "Operator", "track": "Bass", "on": true, "params": (0..12).map(|i| json!({"name": format!("P{}", i + 1), "value": i as f64 * 5.0, "display": format!("v{i}")})).collect::<Vec<_>>()},
        })
    }

    fn show(action: &str, inputs: Value) -> Option<Shown> {
        shown(action, &inputs, &state(), 0)
    }

    #[test]
    fn transport_and_switches() {
        assert_eq!(show("play-stop", json!({})), Some(Shown::state("playing")));
        assert_eq!(show("metronome", json!({})), Some(Shown::state("on")));
        assert_eq!(show("record", json!({})), Some(Shown::state("off")));
        assert_eq!(show("session-record", json!({})), Some(Shown::state("on")));
        assert_eq!(show("automation-arm", json!({})), Some(Shown::state("on")));
        assert_eq!(show("tempo", json!({})), Some(Shown::state("123.5")));
        assert_eq!(show("tap-tempo", json!({})).unwrap().label.unwrap(), "Tap Tempo\n123.5 BPM");
        assert_eq!(show("undo", json!({})), None);
    }

    #[test]
    fn mixer_keys_follow_their_track() {
        assert_eq!(show("master-volume", json!({})), Some(Shown::both("0", "Master\n0.0 dB")));
        assert_eq!(show("track-volume", json!({"track": "Drums"})), Some(Shown::both("-6", "Drums\n-6.0 dB")));
        assert_eq!(show("track-volume", json!({"track": "Bass"})), Some(Shown::both("-70", "Bass\n-inf dB")));
        assert_eq!(show("track-volume", json!({})), Some(Shown::both("2", "A-Reverb\n2.0 dB")), "the selected track by default");
        assert_eq!(show("track-volume", json!({"track": "master"})), Some(Shown::both("0", "Master\n0.0 dB")));
        assert_eq!(show("track-pan", json!({"track": "#1"})), Some(Shown::both("-10", "Drums\n10L")));
        assert_eq!(show("mute", json!({"track": "Drums"})), Some(Shown::both("on", "Drums")));
        assert_eq!(show("solo", json!({"track": "Bass"})), Some(Shown::both("on", "Bass")));
        assert_eq!(show("arm", json!({"track": "Drums"})), Some(Shown::both("off", "Drums")));
        assert_eq!(show("mute", json!({"track": "Gone"})), Some(Shown::both("off", "Gone\nnot in this set")));
        assert_eq!(show("selected-track", json!({})), Some(Shown::label("A-Reverb\n2.0 dB")));
    }

    #[test]
    fn scenes_and_selection() {
        assert_eq!(show("launch-scene", json!({})), Some(Shown::label("Launch\nScene 2")));
        assert_eq!(show("launch-scene", json!({"scene": "Drop"})), Some(Shown::label("Launch\nDrop")));
        assert_eq!(show("launch-scene", json!({"scene": "#1"})), Some(Shown::label("Launch\nIntro")));
        assert_eq!(show("select-scene", json!({})), Some(Shown::both("2", "Scene 2")));
        // A return track counts after the tracks.
        assert_eq!(show("select-track", json!({})), Some(Shown::both("3", "A-Reverb")));
    }

    #[test]
    fn device_banks() {
        assert_eq!(show("device", json!({})), Some(Shown::label("Bass\nOperator")));
        assert_eq!(show("device-on", json!({})), Some(Shown::state("on")));
        assert_eq!(show("device-parameter", json!({"slot": "3"})), Some(Shown::both("10", "P3\nv2")));
        assert_eq!(shown("device-parameter", &json!({"slot": "3"}), &state(), 1), Some(Shown::both("50", "P11\nv10")), "follows the shared bank");
        assert_eq!(shown("device-parameter", &json!({"slot": "3", "bank": "1"}), &state(), 1), Some(Shown::both("10", "P3\nv2")), "a fixed bank");
        assert_eq!(show("device-parameter", json!({"slot": "8", "bank": "2"})), Some(Shown::label("Operator\nno parameter 16")));
        assert_eq!(bank_count(&state()), 2);
        assert_eq!(shown("next-bank", &json!({}), &state(), 1), Some(Shown::label("Bank 2/2")));
        assert_eq!(param_index(&json!({"slot": "9"}), 0), 0, "a bad slot is slot 1");
        assert_eq!(show("selected-parameter", json!({})), Some(Shown::label("No parameter\nselected")));
    }

    #[test]
    fn offline() {
        let off = |a: &str| shown(a, &json!({}), &Value::Null, 0);
        assert_eq!(off("selected-track"), Some(Shown::label("Live offline")));
        assert_eq!(off("play-stop"), Some(Shown::state("stopped")));
        assert_eq!(off("loop"), Some(Shown::state("off")));
        assert_eq!(off("mute"), Some(Shown::state("off")));
        assert_eq!(off("tempo"), None);
    }

    #[test]
    fn every_dynamic_action_is_handled() {
        for a in DYNAMIC {
            assert!(shown(a, &json!({}), &state(), 0).is_some(), "{a} shows nothing");
        }
    }
}
