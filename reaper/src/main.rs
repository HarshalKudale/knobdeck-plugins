//! REAPER plugin for KnobDeck: transport, markers, any action by command id, and track volume,
//! pan, mute, solo and record arm, over REAPER's built-in web browser interface.

mod reaper;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, load_env, Action, Ctx, Error, Plugin, Result, Value, Visual};
use reaper::{Conn, Play, Snapshot, Track};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(1);
/// Keys polling in the same second share one request.
const FRESH: Duration = Duration::from_millis(400);

// REAPER's own action ids (Actions ▸ Show action list shows them).
const PLAY: &str = "1007";
const PLAY_PAUSE: &str = "40073";
const STOP: &str = "1016";
const RECORD: &str = "1013";
const GO_START: &str = "40042";
const GO_END: &str = "40043";
const METRONOME: &str = "40364";
const ADD_MARKER: &str = "40157";
const PREV_MARKER: &str = "40172";
const NEXT_MARKER: &str = "40173";
const UNSELECT_ALL: &str = "40297";

fn conn() -> Conn {
    Conn::new(&env_or("REAPER_HOST", "127.0.0.1"), &env_or("REAPER_PORT", "8080"), &env_or("REAPER_USER", ""), &knobdeck_sdk::secrets::env_or_stored("REAPER_PASSWORD"))
}

static CACHE: Mutex<Option<(Instant, Snapshot)>> = Mutex::new(None);

fn remember(s: Snapshot) -> Snapshot {
    if s.transport.is_some() {
        *CACHE.lock().unwrap() = Some((Instant::now(), s.clone()));
    }
    s
}

fn snapshot() -> Result<Snapshot> {
    if let Some((t, s)) = CACHE.lock().unwrap().as_ref() {
        if t.elapsed() < FRESH {
            return Ok(s.clone());
        }
    }
    let r = conn().snapshot();
    if let Err(e) = &r {
        if e.message().contains("username") {
            knobdeck_sdk::mark_setup_required("REAPER rejected the username / password");
        }
    }
    r.map(remember)
}

/// Run commands; the reply's state replaces the cached one.
fn write(cmds: &str) -> Result<Snapshot> {
    conn().write(cmds).map(remember)
}

fn play_state(s: &Snapshot) -> &'static str {
    match s.transport.as_ref().map(|t| &t.play) {
        Some(Play::Playing) => "playing",
        Some(Play::Paused) => "paused",
        Some(Play::Recording) | Some(Play::RecordPaused) => "recording",
        _ => "stopped",
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn track_input() -> Value {
    input::query_select("track", "Track", "tracks", &[]).with("default", "selected")
}

fn pick(inputs: &Value) -> String {
    inputs["track"].as_str().unwrap_or("selected").to_string()
}

/// The first track a key targets (what it shows).
fn shown(inputs: &Value) -> Result<Track> {
    let s = snapshot()?;
    let t = s.targets(&pick(inputs))?[0].clone();
    Ok(t)
}

fn step(ctx: &Ctx, default: f64) -> f64 {
    ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(default)
}

fn step_input(label: &str, default: &str) -> Value {
    input::text("step", label).placeholder(default).default_value(default)
}

/// A button that runs one REAPER action.
fn run_button(id: &str, label: &str, icon: &str, command: &'static str) -> Action {
    Action::button(id, label).icon(icon).on_press(move |_| write(command).map(|_| ()))
}

/// A toggle over one track flag: mute, solo, record arm.
fn track_toggle(id: &str, label: &str, cmd: &'static str, icons: (&str, &str), get: fn(&Track) -> bool) -> Action {
    Action::toggle(id, label)
        .icon(icons.1)
        .inputs(vec![track_input()])
        .state("off", &format!("{label} off"), icons.0)
        .state("on", &format!("{label} on"), icons.1)
        .live(POLL, move |ic| {
            let t = shown(&ic.inputs)?;
            Ok(Visual::new().state(on_off(get(&t))).label(t.title()))
        })
        .on_press(move |ctx| {
            let s = snapshot()?;
            let targets = s.targets(&pick(&ctx.inputs))?;
            // Every target gets the same new value, so a group never ends up half on.
            let on = !get(targets[0]);
            let cmds: Vec<String> = targets.iter().map(|t| format!("SET/TRACK/{}/{cmd}/{}", t.number, on as u8)).collect();
            write(&cmds.join(";"))?;
            ctx.fan_out_by(&["track"], on_off(on));
            Ok(())
        })
}

fn main() {
    load_env();
    let os = std::env::consts::OS;
    Plugin::new(&format!("reaper-{os}"), "REAPER")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[os]) // one package per OS
        .author("KnobDeck")
        .description("REAPER transport with live state, markers, any action by command id, and track volume, pan, mute, solo and record arm on keys and dials, over REAPER's web browser interface.")
        .icon("app-icon.svg")
        .setup("reaper-setup", None)
        .setup_inputs(vec![
            input::text("host", "Host").placeholder("127.0.0.1").default_value("127.0.0.1"),
            input::text("port", "Port")
                .placeholder("8080")
                .default_value("8080")
                .with("help", "In REAPER: Preferences ▸ Control/OSC/web ▸ Add ▸ Web browser interface. The port shown there (8080 unless you changed it)."),
            input::text("user", "Username (optional)").with("help", "Only if you set Username:password on the web interface."),
            input::text("password", "Password (optional)").with("type", "password").with("help", "Kept in this computer's credential store."),
        ])
        .query("tracks", |q| {
            let s = snapshot().or_else(|e| q.fail(e.message().to_string()))?;
            let mut opts = vec![json!({"value": "selected", "label": "Selected tracks"})];
            opts.extend(s.tracks.iter().map(|t| json!({"value": t.number.to_string(), "label": t.title()})));
            Ok(Value::Array(opts))
        })
        .query("markers", |q| {
            let list = conn().markers().or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(
                list.into_iter()
                    .map(|m| {
                        let name = if m.name.trim().is_empty() { format!("Marker {}", m.id) } else { format!("{} ({})", m.name, m.id) };
                        json!({"value": m.id.to_string(), "label": name})
                    })
                    .collect(),
            ))
        })
        // Transport
        .action(
            Action::toggle("play-pause", "Play / Pause")
                .icon("play")
                .state("stopped", "Stopped", "play")
                .state("playing", "Playing", "pause")
                .state("paused", "Paused", "play")
                .state("recording", "Recording", "circle-dot")
                .live(POLL, |_| Ok(Visual::new().state(play_state(&snapshot()?))))
                .on_press(|ctx| {
                    let s = snapshot()?;
                    let cmd = if play_state(&s) == "stopped" { PLAY } else { PLAY_PAUSE };
                    ctx.fan_out(play_state(&write(cmd)?));
                    Ok(())
                }),
        )
        .action(
            Action::button("stop", "Stop").icon("square").on_press(|ctx| {
                ctx.runtime().notify("play-pause", play_state(&write(STOP)?));
                Ok(())
            }),
        )
        .action(
            Action::toggle("record", "Record")
                .icon("circle-dot")
                .state("off", "Record", "circle")
                .state("on", "Recording", "circle-dot")
                .live(POLL, |_| Ok(Visual::new().state(on_off(play_state(&snapshot()?) == "recording"))))
                .on_press(|ctx| {
                    // While recording, the press stops (REAPER keeps or asks about the take as set
                    // in its preferences); otherwise it starts recording.
                    let recording = play_state(&snapshot()?) == "recording";
                    let s = write(if recording { STOP } else { RECORD })?;
                    ctx.fan_out(on_off(play_state(&s) == "recording"));
                    ctx.runtime().notify("play-pause", play_state(&s));
                    Ok(())
                }),
        )
        .action(
            Action::toggle("repeat", "Repeat")
                .icon("repeat")
                .state("off", "Repeat off", "repeat")
                .state("on", "Repeat on", "repeat-1")
                .live(POLL, |_| Ok(Visual::new().state(on_off(snapshot()?.repeat))))
                .on_press(|ctx| {
                    ctx.fan_out(on_off(write("SET/REPEAT/-1")?.repeat));
                    Ok(())
                }),
        )
        .action(
            Action::toggle("metronome", "Metronome")
                .icon("timer")
                .state("off", "Metronome off", "timer-off")
                .state("on", "Metronome on", "timer")
                .live(POLL, |_| Ok(Visual::new().state(on_off(conn().command_state(METRONOME)?.unwrap_or(false)))))
                .on_press(|ctx| {
                    write(METRONOME)?;
                    ctx.fan_out(on_off(conn().command_state(METRONOME)?.unwrap_or(false)));
                    Ok(())
                }),
        )
        .action(run_button("go-start", "Go to Start", "skip-back", GO_START))
        .action(run_button("go-end", "Go to End", "skip-forward", GO_END))
        .action(
            Action::button("position", "Position")
                .icon("clock")
                .inputs(vec![input::select("format", "Show", &[("time", "Time"), ("beats", "Measures.beats")]).default_value("time")])
                .live(Duration::from_millis(500), |ic| {
                    let s = snapshot()?;
                    let t = s.transport.ok_or_else(|| Error::msg("REAPER sent no transport state"))?;
                    Ok(Visual::new().label(if ic.input_str("format") == "beats" { t.beats } else { t.position }))
                })
                .on_press(|_| Ok(())),
        )
        // Markers
        .action(run_button("add-marker", "Add Marker", "map-pin-plus", ADD_MARKER))
        .action(run_button("previous-marker", "Previous Marker", "chevron-left", PREV_MARKER))
        .action(run_button("next-marker", "Next Marker", "chevron-right", NEXT_MARKER))
        .action(Action::button("go-to-marker", "Go to Marker").icon("map-pin").inputs(vec![input::query_select("marker", "Marker", "markers", &[])]).on_press(|ctx| {
            let Ok(id) = ctx.input_str("marker").parse::<u32>() else {
                return ctx.fail("Choose a marker");
            };
            write(&format!("SET/POS_STR/m{id}")).map(|_| ())
        }))
        // Any action
        .action(
            Action::toggle("run-action", "Run Action")
                .icon("zap")
                .inputs(vec![input::text("command", "Command ID").placeholder("40044 or _SWS_SAVESEL").with(
                    "help",
                    "Actions ▸ Show action list ▸ right-click an action ▸ Copy selected action command ID. Numbers and named ids (SWS, scripts, custom actions) both work.",
                )])
                .state("off", "Off", "zap")
                .state("on", "On", "zap")
                .live(POLL, |ic| {
                    let id = ic.input_str("command").trim().to_string();
                    if !reaper::valid_command_id(&id) {
                        return Ok(Visual::new());
                    }
                    // Actions with an on/off state (toggles) show it; others stay as they are.
                    Ok(match conn().command_state(&id)? {
                        Some(on) => Visual::new().state(on_off(on)).icon(if on { "zap" } else { "zap-off" }),
                        None => Visual::new(),
                    })
                })
                .on_press(|ctx| {
                    let id = ctx.input_str("command").trim().to_string();
                    if !reaper::valid_command_id(&id) {
                        return ctx.fail("Enter a command id: a number like 40044 or a named id like _SWS_SAVESEL");
                    }
                    write(&id)?;
                    if let Some(on) = conn().command_state(&id)? {
                        ctx.set_visual(Visual::new().state(on_off(on)).icon(if on { "zap" } else { "zap-off" }));
                    }
                    Ok(())
                }),
        )
        // Tracks
        .action(
            Action::slider("track-volume", "Track Volume", reaper::MIN_DB, reaper::MAX_DB)
                .icon("volume-2")
                .unit("dB")
                .step(0.1)
                .inputs(vec![track_input(), step_input("Step (dB)", "1")])
                .live(POLL, |ic| {
                    let t = shown(&ic.inputs)?;
                    Ok(Visual::new().state((t.db() * 10.0).round() / 10.0).label(t.title()))
                })
                .on_adjust(|ctx| {
                    let s = snapshot()?;
                    let targets = s.targets(&pick(&ctx.inputs))?;
                    let to = |t: &Track| match ctx.value {
                        Some(v) => v,
                        None => t.db() + step(ctx, 1.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .clamp(reaper::MIN_DB, reaper::MAX_DB);
                    let cmds: Vec<String> = targets.iter().map(|t| reaper::vol_cmd(t.number, to(t))).collect();
                    let shown = (to(targets[0]) * 10.0).round() / 10.0;
                    write(&cmds.join(";"))?;
                    ctx.fan_out_by(&["track"], shown);
                    Ok(())
                }),
        )
        .action(
            Action::slider("track-pan", "Track Pan", -100.0, 100.0)
                .icon("move-horizontal")
                .unit("%")
                .step(1.0)
                .inputs(vec![track_input(), step_input("Step (%)", "5")])
                .live(POLL, |ic| {
                    let t = shown(&ic.inputs)?;
                    Ok(Visual::new().state((t.pan * 100.0).round()).label(t.title()))
                })
                .on_adjust(|ctx| {
                    let s = snapshot()?;
                    let targets = s.targets(&pick(&ctx.inputs))?;
                    let to = |t: &Track| match ctx.value {
                        Some(v) => v,
                        None => t.pan * 100.0 + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(-100.0, 100.0);
                    let cmds: Vec<String> = targets.iter().map(|t| reaper::pan_cmd(t.number, to(t) / 100.0)).collect();
                    let shown = to(targets[0]);
                    write(&cmds.join(";"))?;
                    ctx.fan_out_by(&["track"], shown);
                    Ok(())
                }),
        )
        .action(track_toggle("mute", "Mute", "MUTE", ("volume-2", "volume-x"), Track::muted))
        .action(track_toggle("solo", "Solo", "SOLO", ("headphones", "headphones"), Track::soloed))
        .action(track_toggle("record-arm", "Record Arm", "RECARM", ("circle", "circle-dot"), Track::armed))
        .action(
            Action::button("selected-track", "Selected Track")
                .icon("list-music")
                .live(POLL, |_| {
                    let s = snapshot()?;
                    let sel = s.selected();
                    Ok(Visual::new().label(match sel.as_slice() {
                        [] => "No track selected".to_string(),
                        [t] => format!("{}\n{:.1} dB", t.title(), t.db()),
                        [t, ..] => format!("{} +{}", t.title(), sel.len() - 1),
                    }))
                })
                .on_press(|_| Ok(())),
        )
        .action(
            Action::slider("select-track", "Select Track (dial)", 0.0, 256.0)
                .icon("list-music")
                .step(1.0)
                .live(POLL, |_| {
                    let s = snapshot()?;
                    Ok(match s.selected().first() {
                        Some(t) => Visual::new().state(t.number).label(t.title()),
                        None => Visual::new().label("No track selected"),
                    })
                })
                .on_adjust(|ctx| {
                    let s = snapshot()?;
                    let last = s.tracks.iter().map(|t| t.number).max().unwrap_or(0);
                    if last == 0 {
                        return ctx.fail("The project has no tracks");
                    }
                    let now = s.selected().first().map(|t| t.number as f64).unwrap_or(0.0);
                    let target = match ctx.value {
                        Some(v) => v,
                        None => now + ctx.delta.unwrap_or(0.0).signum(),
                    }
                    .round()
                    .clamp(1.0, last as f64) as u32;
                    // Only that track stays selected, so "Selected tracks" keys follow it.
                    let s = write(&format!("{UNSELECT_ALL};SET/TRACK/{target}/SEL/1"))?;
                    let label = s.track(target).map(Track::title).unwrap_or_default();
                    ctx.set_visual(Visual::new().state(target).label(label));
                    Ok(())
                }),
        )
        .run()
}
