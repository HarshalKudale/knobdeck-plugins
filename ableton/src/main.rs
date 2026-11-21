//! Ableton Live plugin for KnobDeck: drives Live through the KnobDeck Remote Script
//! (`Knobdeck/knobdeck-ableton-companion`), a control surface script that runs inside Live, links
//! to KnobDeck's companion hub and runs each action with Live's own Python API (the Live Object
//! Model: the Song, its tracks, scenes, cue points and devices), never with keystrokes. The
//! plugin opens no port. Setup copies the Remote Script into Live's User Library.
//!
//! The script sends a `state` event whenever what the keys show changes (transport, switches,
//! tempo, the mixer, the selection, the device's parameters); the plugin keeps the last one and
//! pushes what each placed key shows. Several Live processes can link (rare): actions go to the
//! one that changed last.

mod keys;

use keys::{on_off, track_pick, Shown, MAX_DB, MIN_DB, PARAMS_PER_BANK};
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{companion, json, Action, Ctx, Error, Plugin, Result, Runtime, Value, Visual};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

const NOT_LINKED: &str = "Ableton Live isn't linked to KnobDeck. Run this plugin's setup (it installs the KnobDeck Remote Script), then in Live ▸ Settings ▸ Link, Tempo & MIDI choose KnobDeck as a Control Surface.";

/// Linked Live processes (link ids, oldest first) and the one that reported a change last.
static LINKS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ACTIVE: Mutex<Option<String>> = Mutex::new(None);
/// The last `state` event of each link.
static STATES: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);
/// What each placed key was last sent, so unchanged keys aren't sent again.
static PUSHED: Mutex<Option<HashMap<String, Shown>>> = Mutex::new(None);
/// The parameter bank the device dials follow (0-based), and the device it belongs to.
static BANK: Mutex<(String, usize)> = Mutex::new((String::new(), 0));

fn target_link() -> Option<String> {
    let links = LINKS.lock().unwrap();
    let active = ACTIVE.lock().unwrap().clone().filter(|a| links.contains(a));
    active.or_else(|| links.last().cloned())
}

/// The target Live's last state (`Null` when none is linked).
fn state() -> Value {
    let Some(link) = target_link() else { return Value::Null };
    STATES.lock().unwrap().as_ref().and_then(|m| m.get(&link).cloned()).unwrap_or(Value::Null)
}

/// Keep LINKS / ACTIVE / STATES in step with the hub's events. Returns whether the target's state changed.
fn on_companion(update: companion::Update) -> bool {
    let mut links = LINKS.lock().unwrap();
    match update {
        companion::Update::Connected(l) => {
            links.retain(|x| x != &l.link);
            links.push(l.link);
            false
        }
        companion::Update::Disconnected(g) => {
            links.retain(|x| x != &g.link);
            let mut active = ACTIVE.lock().unwrap();
            if active.as_deref() == Some(&g.link) {
                *active = None;
            }
            if let Some(m) = STATES.lock().unwrap().as_mut() {
                m.remove(&g.link);
            }
            true
        }
        companion::Update::Event(e) if e.event == "state" => {
            if !links.contains(&e.link) {
                links.push(e.link.clone());
            }
            *ACTIVE.lock().unwrap() = Some(e.link.clone());
            STATES.lock().unwrap().get_or_insert_with(HashMap::new).insert(e.link, e.payload);
            true
        }
        companion::Update::Event(_) => false,
    }
}

/// Run `action` in the target Live and return its result.
fn call(action: &str, params: Value) -> Result<Value> {
    let link = target_link().ok_or_else(|| Error::msg(NOT_LINKED))?;
    match companion::request_with_timeout(Some(&link), action, params, Some(5000)) {
        Ok(answer) => companion::check(answer).map_err(|e| Error::msg(format!("Live: {}", e.message()))),
        Err(e) => Err(Error::msg(format!("Live didn't answer: {}", e.message()))),
    }
}

/// The shared bank, back to 1 when the controlled device changed.
fn shared_bank(s: &Value) -> usize {
    let mut b = BANK.lock().unwrap();
    let key = keys::device_key(s);
    if b.0 != key {
        *b = (key, 0);
    }
    b.1.min(keys::bank_count(s) - 1)
}

fn visual(sh: &Shown) -> Visual {
    let mut v = Visual::new();
    if let Some(s) = &sh.state {
        v = v.state(s);
    }
    if let Some(l) = &sh.label {
        v = v.label(l);
    }
    v
}

/// Send every placed key what it should show now, where that changed.
fn push_all(rt: &Runtime) {
    let s = state();
    let bank = shared_bank(&s);
    let mut pushed = PUSHED.lock().unwrap();
    let pushed = pushed.get_or_insert_with(HashMap::new);
    let mut seen = vec![];
    for action in keys::DYNAMIC {
        for iid in rt.instances_of(action) {
            let Some(sh) = keys::shown(action, &rt.inputs_of(&iid), &s, bank) else { continue };
            seen.push(iid.clone());
            if pushed.get(&iid) != Some(&sh) {
                rt.set_visual(&iid, visual(&sh));
                pushed.insert(iid, sh);
            }
        }
    }
    pushed.retain(|k, _| seen.contains(k));
}

/// The key's state from the last state event, for a newly placed key.
fn initial(action: &'static str) -> impl Fn(&knobdeck_sdk::InitialCtx) -> Option<String> + Send + Sync + 'static {
    move |ic| {
        let s = state();
        keys::shown(action, &ic.inputs, &s, shared_bank(&s)).and_then(|sh| sh.state)
    }
}

// ── Actions ────────────────────────────────────────────────────────────────────

fn button(id: &'static str, label: &str, icon: &str, action: &'static str, params: Value) -> Action {
    Action::button(id, label).icon(icon).on_press(move |ctx| {
        call(action, params.clone())?;
        push_all(ctx.runtime());
        Ok(())
    })
}

/// A Song switch (metronome, loop, record…) with its state.
fn switch(id: &'static str, label: &str, icons: (&str, &str)) -> Action {
    let name = keys::switch_of(id).expect("a switch");
    Action::toggle(id, label)
        .icon(icons.1)
        .state("off", &format!("{label} Off"), icons.0)
        .state("on", &format!("{label} On"), icons.1)
        .initial_with(initial(id))
        .on_press(move |ctx| {
            let r = call("toggle", json!({"switch": name, "value": "toggle"}))?;
            if let Some(on) = r["value"].as_bool() {
                ctx.fan_out(on_off(on));
            }
            Ok(())
        })
}

fn track_input(with_master: bool) -> Value {
    input::query_select("track", "Track", if with_master { "mixer-tracks" } else { "tracks" }, &[]).with("default", "selected")
}

fn step_input(label: &str, default: &str) -> Value {
    input::text("step", label).placeholder(default).default_value(default)
}

fn step_of(ctx: &Ctx, default: f64) -> f64 {
    ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(default)
}

/// `{value}` for a drag, `{delta}` (ticks × step) for a turn; `None` when neither came.
fn adjust_params(ctx: &Ctx, step: f64) -> Option<Value> {
    match (ctx.value, ctx.delta) {
        (Some(v), _) => Some(json!({"value": v})),
        (None, Some(d)) => Some(json!({"delta": d * step})),
        _ => None,
    }
}

/// Mute / solo / arm of a picked track.
fn track_flag(id: &'static str, label: &str, icons: (&str, &str)) -> Action {
    Action::toggle(id, label)
        .icon(icons.1)
        .inputs(vec![track_input(false)])
        .state("off", &format!("{label} Off"), icons.0)
        .state("on", &format!("{label} On"), icons.1)
        .initial_with(initial(id))
        .on_press(move |ctx| {
            let r = call("track_flag", json!({"track": track_pick(&ctx.inputs), "flag": id, "value": "toggle"}))?;
            if let Some(on) = r["value"].as_bool() {
                ctx.fan_out_by(&["track"], on_off(on));
            }
            Ok(())
        })
}

/// A volume dial in dB: the master's (`fixed = Some("master")`) or a picked track's.
fn volume(id: &'static str, label: &str, fixed: Option<&'static str>) -> Action {
    let mut inputs = vec![step_input("dB per tick", "1")];
    if fixed.is_none() {
        inputs.insert(0, track_input(true));
    }
    Action::slider(id, label, MIN_DB, MAX_DB)
        .icon("volume-2")
        .unit("dB")
        .step(0.1)
        .inputs(inputs)
        .initial_with(initial(id))
        .on_adjust(move |ctx| {
            let Some(mut params) = adjust_params(ctx, step_of(ctx, 1.0)) else { return Ok(()) };
            params["track"] = json!(fixed.unwrap_or_else(|| track_pick(&ctx.inputs)));
            params["control"] = json!("volume");
            call("mixer", params)?;
            push_all(ctx.runtime());
            Ok(())
        })
}

/// Select a track (tracks, then returns) or a scene with a dial.
fn select_dial(id: &'static str, label: &str, icon: &str, what: &'static str) -> Action {
    Action::slider(id, label, 1.0, 999.0)
        .icon(icon)
        .step(1.0)
        .initial_with(initial(id))
        .on_adjust(move |ctx| {
            let params = match (ctx.value, ctx.delta) {
                (Some(v), _) => json!({"what": what, "index": (v.round() - 1.0).max(0.0)}),
                (None, Some(d)) if d != 0.0 => json!({"what": what, "delta": d.signum()}),
                _ => return Ok(()),
            };
            call("select", params)?;
            push_all(ctx.runtime());
            Ok(())
        })
}

/// Slices for a wheel of tracks or scenes around the selected one.
fn wheel_slices(what: &str, icon: &str) -> Result<Vec<Value>> {
    let r = call("wheel", json!({"what": what}))?;
    let items = r["items"].as_array().cloned().unwrap_or_default();
    if items.is_empty() {
        return Err(Error::msg(format!("The set has no {what}")));
    }
    Ok(items
        .into_iter()
        .map(|i| {
            let n = i["index"].as_u64().unwrap_or(0);
            let name = i["name"].as_str().unwrap_or("").trim().to_string();
            let label = if name.is_empty() { format!("Scene {}", n + 1) } else { name };
            json!({"id": n.to_string(), "label": label, "icon": if i["selected"].as_bool() == Some(true) { "circle-check" } else { icon }})
        })
        .collect())
}

fn picked_index(ctx: &Ctx) -> Result<u64> {
    ctx.child_id.as_deref().and_then(|c| c.parse().ok()).ok_or_else(|| Error::msg("Choose an entry on the wheel"))
}

/// Tracks for pickers; `master` adds the Master track (volume / pan).
fn track_options(master: bool) -> Result<Value> {
    let r = call("list", json!({"what": "tracks"}))?;
    let mut opts = vec![json!({"value": "selected", "label": "Selected track"})];
    if master {
        opts.push(json!({"value": "master", "label": "Master"}));
    }
    opts.extend(r["items"].as_array().cloned().unwrap_or_default());
    Ok(Value::Array(opts))
}

fn bank_options() -> Vec<(String, String)> {
    let mut v = vec![("follow".to_string(), "Follow the bank keys".to_string())];
    v.extend((1..=16).map(|b| (b.to_string(), format!("Bank {b} (parameters {}–{})", (b - 1) * 8 + 1, b * 8))));
    v
}

fn move_bank(rt: &Runtime, by: i64) -> Result {
    let s = state();
    if s.is_null() {
        return Err(Error::msg(NOT_LINKED));
    }
    if s.get("device").is_none() {
        return Err(Error::msg("No device is selected in Live"));
    }
    let now = shared_bank(&s) as i64;
    let last = keys::bank_count(&s) as i64 - 1;
    BANK.lock().unwrap().1 = (now + by).clamp(0, last) as usize;
    push_all(rt);
    Ok(())
}

fn main() {
    let os = std::env::consts::OS;
    let banks = bank_options();
    let bank_refs: Vec<(&str, &str)> = banks.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let slots: Vec<(String, String)> = (1..=PARAMS_PER_BANK).map(|s| (s.to_string(), format!("Parameter {s}"))).collect();
    let slot_refs: Vec<(&str, &str)> = slots.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    Plugin::new(&format!("ableton-{os}"), "Ableton Live")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[os]) // one package per OS
        .author("KnobDeck")
        .description("Control Ableton Live through the KnobDeck Remote Script: transport, record, metronome, loop, tempo and tap tempo, the mixer (master and track volume, pan, mute, solo, arm), scenes, track and scene selection, locators, undo, Capture MIDI and the selected device's parameters on dials, with live state on every key.")
        .icon("app-icon.svg")
        .ack_timeout_ms(6000)
        .setup("ableton-setup", None)
        .setup_inputs(vec![
            input::checkbox("install", "Install the KnobDeck Remote Script into Live's User Library")
                .default_value(true)
                .with("help", "Downloads the newest release from github.com/Knobdeck/knobdeck-ableton-companion. Untick it if you copied the KnobDeck folder into Remote Scripts yourself."),
            input::text("user_library", "User Library folder (optional)").with(
                "help",
                "Only if you moved it: the folder Live shows in Settings ▸ Library ▸ Location of User Library. Found by itself otherwise.",
            ),
        ])
        .query("tracks", |q| track_options(false).or_else(|e| q.fail(e.message().to_string())))
        .query("mixer-tracks", |q| track_options(true).or_else(|e| q.fail(e.message().to_string())))
        .query("scenes", |q| {
            let r = call("list", json!({"what": "scenes"})).or_else(|e| q.fail(e.message().to_string()))?;
            let mut opts = vec![json!({"value": "selected", "label": "Selected scene"})];
            opts.extend(r["items"].as_array().cloned().unwrap_or_default());
            Ok(Value::Array(opts))
        })
        // Transport
        .action(
            Action::toggle("play-stop", "Play / Stop")
                .icon("play")
                .state("stopped", "Play", "play")
                .state("playing", "Stop", "square")
                .initial_with(initial("play-stop"))
                .on_press(|ctx| {
                    let r = call("transport", json!({"op": "toggle"}))?;
                    ctx.fan_out(if r["playing"].as_bool() == Some(true) { "playing" } else { "stopped" });
                    Ok(())
                }),
        )
        .action(button("stop", "Stop", "square", "transport", json!({"op": "stop"})))
        .action(button("continue", "Continue Playing", "step-forward", "transport", json!({"op": "continue"})))
        .action(switch("record", "Arrangement Record", ("circle", "circle-dot")))
        .action(switch("session-record", "Session Record", ("circle", "circle-dot")))
        .action(switch("overdub", "MIDI Arrangement Overdub", ("layers", "layers")))
        .action(switch("automation-arm", "Automation Arm", ("activity", "activity")))
        .action(switch("metronome", "Metronome", ("timer-off", "timer")))
        .action(switch("loop", "Loop", ("repeat", "repeat-1")))
        .action(switch("punch-in", "Punch In", ("log-in", "log-in")))
        .action(switch("punch-out", "Punch Out", ("log-out", "log-out")))
        .action(button("capture-midi", "Capture MIDI", "download", "capture_midi", json!({})))
        .action(button("undo", "Undo", "undo-2", "undo", json!({})))
        .action(button("redo", "Redo", "redo-2", "redo", json!({})))
        // Tempo
        .action(button("tap-tempo", "Tap Tempo", "hand", "tap_tempo", json!({})))
        .action(
            Action::slider("tempo", "Tempo", 20.0, 999.0)
                .icon("gauge")
                .unit("BPM")
                .step(0.1)
                .inputs(vec![step_input("BPM per tick", "1")])
                .initial_with(initial("tempo"))
                .on_adjust(|ctx| {
                    let Some(params) = adjust_params(ctx, step_of(ctx, 1.0)) else { return Ok(()) };
                    let r = call("tempo", params)?;
                    if let Some(t) = r["value"].as_f64() {
                        ctx.fan_out(keys::fmt(t));
                    }
                    Ok(())
                }),
        )
        // Locators
        .action(button("previous-locator", "Previous Locator", "chevron-first", "cue", json!({"to": "previous"})))
        .action(button("next-locator", "Next Locator", "chevron-last", "cue", json!({"to": "next"})))
        .action(button("set-locator", "Add / Delete Locator", "map-pin-plus", "cue", json!({"to": "toggle"})))
        // Mixer
        .action(volume("master-volume", "Master Volume", Some("master")))
        .action(volume("track-volume", "Track Volume", None))
        .action(
            Action::slider("track-pan", "Track Pan", -50.0, 50.0)
                .icon("move-horizontal")
                .step(1.0)
                .inputs(vec![track_input(true), step_input("Pan per tick", "2")])
                .initial_with(initial("track-pan"))
                .on_adjust(|ctx| {
                    let Some(mut params) = adjust_params(ctx, step_of(ctx, 2.0)) else { return Ok(()) };
                    params["track"] = json!(track_pick(&ctx.inputs));
                    params["control"] = json!("pan");
                    call("mixer", params)?;
                    push_all(ctx.runtime());
                    Ok(())
                }),
        )
        .action(track_flag("mute", "Mute", ("volume-2", "volume-x")))
        .action(track_flag("solo", "Solo", ("headphones", "headphones")))
        .action(track_flag("arm", "Record Arm", ("circle", "circle-dot")))
        .action(Action::button("selected-track", "Selected Track").icon("list-music").on_press(|_| Ok(())))
        // Session
        .action(
            Action::button("launch-scene", "Launch Scene")
                .icon("play-circle")
                .inputs(vec![input::query_select("scene", "Scene", "scenes", &[]).with("default", "selected")])
                .on_press(|ctx| {
                    call("launch_scene", json!({"scene": keys::scene_pick(&ctx.inputs)}))?;
                    push_all(ctx.runtime());
                    Ok(())
                }),
        )
        .action(button("stop-all-clips", "Stop All Clips", "square-stack", "stop_all_clips", json!({})))
        .action(select_dial("select-track", "Select Track (dial)", "list-music", "track"))
        .action(select_dial("select-scene", "Select Scene (dial)", "rows-3", "scene"))
        .action(
            Action::wheel("tracks-wheel", "Tracks")
                .icon("list-music")
                .on_open(|_| wheel_slices("tracks", "audio-lines"))
                .on_child_press(|ctx| {
                    call("select", json!({"what": "track", "index": picked_index(ctx)?}))?;
                    push_all(ctx.runtime());
                    Ok(())
                }),
        )
        .action(
            Action::wheel("scenes-wheel", "Scenes")
                .icon("rows-3")
                .inputs(vec![input::select("then", "Choosing a scene", &[("select", "Selects it"), ("launch", "Launches it")]).default_value("select")])
                .on_open(|_| wheel_slices("scenes", "clapperboard"))
                .on_child_press(|ctx| {
                    let i = picked_index(ctx)?;
                    if ctx.input_str("then") == "launch" {
                        call("launch_scene", json!({"scene": format!("#{}", i + 1)}))?;
                    } else {
                        call("select", json!({"what": "scene", "index": i}))?;
                    }
                    push_all(ctx.runtime());
                    Ok(())
                }),
        )
        // Device
        .action(Action::button("device", "Selected Device").icon("cpu").on_press(|_| Ok(())))
        .action(
            Action::toggle("device-on", "Device On / Off")
                .icon("power")
                .state("off", "Device Off", "power-off")
                .state("on", "Device On", "power")
                .initial_with(initial("device-on"))
                .on_press(|ctx| {
                    let r = call("device_on", json!({"value": "toggle"}))?;
                    if let Some(on) = r["value"].as_bool() {
                        ctx.fan_out(on_off(on));
                    }
                    Ok(())
                }),
        )
        .action(
            Action::slider("device-parameter", "Device Parameter", 0.0, 100.0)
                .icon("sliders-horizontal")
                .unit("%")
                .step(1.0)
                .inputs(vec![
                    input::select("slot", "Parameter", &slot_refs).default_value("1"),
                    input::select("bank", "Bank", &bank_refs).default_value("follow"),
                    step_input("Percent per tick", "2"),
                ])
                .initial_with(initial("device-parameter"))
                .on_adjust(|ctx| {
                    let Some(mut params) = adjust_params(ctx, step_of(ctx, 2.0)) else { return Ok(()) };
                    let s = state();
                    params["index"] = json!(keys::param_index(&ctx.inputs, shared_bank(&s)));
                    call("device_param", params)?;
                    push_all(ctx.runtime());
                    Ok(())
                }),
        )
        .action(
            Action::slider("selected-parameter", "Selected Parameter", 0.0, 100.0)
                .icon("sliders-horizontal")
                .unit("%")
                .step(1.0)
                .inputs(vec![step_input("Percent per tick", "2")])
                .initial_with(initial("selected-parameter"))
                .on_adjust(|ctx| {
                    let Some(mut params) = adjust_params(ctx, step_of(ctx, 2.0)) else { return Ok(()) };
                    params["selected"] = json!(true);
                    call("device_param", params)?;
                    push_all(ctx.runtime());
                    Ok(())
                }),
        )
        .action(Action::button("previous-bank", "Previous Parameter Bank").icon("chevron-left").on_press(|ctx| move_bank(ctx.runtime(), -1)))
        .action(Action::button("next-bank", "Next Parameter Bank").icon("chevron-right").on_press(|ctx| move_bank(ctx.runtime(), 1)))
        // Live links through KnobDeck's companion hub; this keeps track of it.
        .companions()
        .on_event(|ev, rt| {
            if let Some(update) = companion::parse(ev) {
                if on_companion(update) {
                    push_all(rt);
                }
            }
            Ok(())
        })
        .on_ready(|rt| {
            // A Live that linked before this process started (a plugin restart): ask the hub, then
            // ask the newest for its state.
            if rt.has_feature(knobdeck_sdk::protocol::features::COMPANION_HUB) {
                if let Ok(links) = companion::links() {
                    *LINKS.lock().unwrap() = links.into_iter().filter(|l| l.app == "ableton").map(|l| l.link).collect();
                }
                if let Some(link) = target_link() {
                    if let Ok(s) = call("state", json!({})) {
                        STATES.lock().unwrap().get_or_insert_with(HashMap::new).insert(link, s);
                    }
                }
            }
            push_all(rt);
            // Keys placed later (or with changed inputs) get their labels within half a second;
            // changes in Live arrive with the state events.
            let rt = rt.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_millis(500));
                push_all(&rt);
            });
            Ok(())
        })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(id: &str) -> companion::Link {
        companion::Link { link: id.into(), instance: id.into(), app: "ableton".into(), app_version: None, name: None, version: None, features: vec![] }
    }

    #[test]
    fn the_live_that_changed_last_is_driven() {
        LINKS.lock().unwrap().clear();
        *ACTIVE.lock().unwrap() = None;
        assert_eq!(target_link(), None);
        assert!(state().is_null());
        assert!(call("undo", json!({})).unwrap_err().message().contains("isn't linked"));
        on_companion(companion::Update::Connected(link("a")));
        on_companion(companion::Update::Connected(link("b")));
        assert_eq!(target_link().as_deref(), Some("b"));
        let ev = companion::Update::Event(companion::LinkEvent { link: "a".into(), event: "state".into(), payload: json!({"tempo": 99}) });
        assert!(on_companion(ev));
        assert_eq!(target_link().as_deref(), Some("a"));
        assert_eq!(state()["tempo"], 99);
        on_companion(companion::Update::Disconnected(companion::LinkGone { link: "a".into() }));
        assert_eq!(target_link().as_deref(), Some("b"));
        assert!(state().is_null());
    }

    #[test]
    fn the_shared_bank_resets_for_another_device() {
        let dev = |name: &str| json!({"device": {"name": name, "track": "Bass", "params": vec![json!({}); 20]}});
        *BANK.lock().unwrap() = (keys::device_key(&dev("Operator")), 2);
        assert_eq!(shared_bank(&dev("Operator")), 2);
        assert_eq!(shared_bank(&dev("Wavetable")), 0);
        BANK.lock().unwrap().1 = 9;
        assert_eq!(shared_bank(&dev("Wavetable")), 2, "clamped to the device's banks");
    }
}
