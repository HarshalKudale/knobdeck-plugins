//! DAW Control (Mackie) plugin for KnobDeck: KnobDeck is a Mackie Control surface any DAW that
//! supports one can use (Logic, Cubase, Studio One, REAPER, Ableton Live, Bitwig, Ardour, FL
//! Studio…). Transport, jog wheel, faders, V-Pots, banks and the strip buttons, with the LEDs,
//! fader positions, track names and time display the DAW sends back. macOS and Linux get a virtual
//! "KnobDeck Mackie" port pair; Windows has no virtual ports, so setup names two existing ones
//! (a loopMIDI pair).

mod mcu;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PORT: &str = "KnobDeck Mackie";
const POLL: Duration = Duration::from_millis(200);
/// A fader stays touched this long after its last move, so the DAW doesn't snap it back between
/// dial ticks (and records automation as one touch).
const TOUCH_HOLD: Duration = Duration::from_millis(400);

static MODEL: Mutex<Option<mcu::Model>> = Mutex::new(None);
static LINK: Mutex<Option<Link>> = Mutex::new(None);
static LAST_TRY: Mutex<Option<(Instant, String)>> = Mutex::new(None);
/// When each touched fader (0–7, master) was last moved.
static TOUCHED: Mutex<[Option<Instant>; 9]> = Mutex::new([None; 9]);

struct Link {
    out: Arc<midi_io::Output>,
    _input: midi_io::Input,
}

fn with_model<T>(f: impl FnOnce(&mcu::Model) -> T) -> T {
    let mut m = MODEL.lock().unwrap();
    f(m.get_or_insert_with(mcu::Model::default))
}

/// The configured ports: (to the DAW, from the DAW); empty = KnobDeck's virtual port.
fn ports() -> (String, String) {
    (env_or("DAW_MACKIE_OUTPUT", "").trim().to_string(), env_or("DAW_MACKIE_INPUT", "").trim().to_string())
}

fn open() -> std::result::Result<Link, String> {
    let (to_daw, from_daw) = ports();
    if !midi_io::HAS_VIRTUAL && (to_daw.is_empty() || from_daw.is_empty()) {
        return Err("Run setup and name the two MIDI ports (Windows has no virtual ports: create a pair with loopMIDI)".into());
    }
    let out = Arc::new(if to_daw.is_empty() { midi_io::Output::open_virtual(PORT, PORT)? } else { midi_io::Output::connect(PORT, &to_daw)? });
    let reply_to = out.clone();
    let on_message = move |m: &[u8]| {
        let reply = MODEL.lock().unwrap().get_or_insert_with(mcu::Model::default).apply(m);
        if let Some(r) = reply {
            let _ = reply_to.send(&r);
        }
    };
    let input = if from_daw.is_empty() { midi_io::Input::open_virtual(PORT, PORT, on_message)? } else { midi_io::Input::connect(PORT, &from_daw, on_message)? };
    Ok(Link { out, _input: input })
}

/// The way to the DAW, opened on first use (and again a few seconds after a failure).
fn output() -> Result<Arc<midi_io::Output>> {
    let mut link = LINK.lock().unwrap();
    if let Some(l) = link.as_ref() {
        return Ok(l.out.clone());
    }
    let mut last = LAST_TRY.lock().unwrap();
    if let Some((t, e)) = last.as_ref() {
        if t.elapsed() < Duration::from_secs(3) {
            return Err(Error::msg(e.clone()));
        }
    }
    match open() {
        Ok(l) => {
            let out = l.out.clone();
            *link = Some(l);
            *last = None;
            Ok(out)
        }
        Err(e) => {
            *last = Some((Instant::now(), e.clone()));
            Err(Error::msg(e))
        }
    }
}

fn send(messages: &[&[u8]]) -> Result<()> {
    let out = output()?;
    for m in messages {
        if let Err(e) = out.send(m) {
            // An existing port that went away: open it again next time.
            *LINK.lock().unwrap() = None;
            return Err(Error::msg(e));
        }
    }
    Ok(())
}

/// Press and release a button, as a surface does.
fn tap(note: u8) -> Result<()> {
    send(&[&mcu::button(note, true)])?;
    std::thread::sleep(Duration::from_millis(30));
    send(&[&mcu::button(note, false)])
}

fn strip(inputs: &Value) -> usize {
    match inputs["strip"].as_str().unwrap_or("1") {
        "master" => mcu::MASTER,
        s => s.parse::<usize>().ok().filter(|s| (1..=8).contains(s)).unwrap_or(1) - 1,
    }
}

fn strip_input(master: bool) -> Value {
    let mut opts: Vec<(&str, &str)> = vec![("1", "Strip 1"), ("2", "Strip 2"), ("3", "Strip 3"), ("4", "Strip 4"), ("5", "Strip 5"), ("6", "Strip 6"), ("7", "Strip 7"), ("8", "Strip 8")];
    if master {
        opts.push(("master", "Master"));
    }
    input::select("strip", "Strip", &opts).default_value("1").with("help", "The channel strip in the DAW's current bank of 8 (Bank / Track moves it).")
}

fn step(ctx: &Ctx, default: f64) -> f64 {
    ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(default)
}

/// Turn ticks from a dial: a turn's delta, or the change of a drag since the last value.
fn ticks(ctx: &Ctx, default_step: f64, from: f64) -> i32 {
    match ctx.value {
        Some(v) => (v - from).round() as i32,
        None => (ctx.delta.unwrap_or(0.0) * step(ctx, default_step)).round() as i32,
    }
}

/// The track name the LCD shows over a strip, as a second line.
fn with_name(base: &str, s: usize) -> String {
    let name = if s == mcu::MASTER { String::new() } else { with_model(|m| m.strip_name(s)) };
    if name.is_empty() {
        base.to_string()
    } else {
        format!("{base}\n{name}")
    }
}

/// Keep the link open (the DAW can only find the virtual ports while they exist).
fn alive() -> Result<()> {
    output().map(|_| ())
}

/// A toggle whose state is a DAW LED: transport keys (fixed note) and strip keys (note + strip).
fn led_toggle(id: &str, label: &str, note: u8, per_strip: bool, off: (&'static str, &'static str), on: (&'static str, &'static str)) -> Action {
    let note_of = move |inputs: &Value| if per_strip { note + strip(inputs) as u8 } else { note };
    let mut a = Action::toggle(id, label).icon(on.1).state("off", off.0, off.1).state("on", on.0, on.1);
    if per_strip {
        a = a.inputs(vec![strip_input(false)]);
    }
    a.live(POLL, move |ic| {
        alive()?;
        let lit = with_model(|m| m.lit(note_of(&ic.inputs)));
        let mut v = Visual::new().state(if lit { "on" } else { "off" });
        if per_strip {
            v = v.label(with_name(if lit { on.0 } else { off.0 }, strip(&ic.inputs)));
        }
        Ok(v)
    })
    .on_press(move |ctx| {
        let n = note_of(&ctx.inputs);
        let before = with_model(|m| m.lit(n));
        tap(n)?;
        // The DAW answers with the LED; show it as soon as it does.
        let deadline = Instant::now() + Duration::from_millis(300);
        while with_model(|m| m.lit(n)) == before && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(15));
        }
        let lit = with_model(|m| m.lit(n));
        if per_strip {
            ctx.fan_out_by(&["strip"], if lit { "on" } else { "off" });
        } else {
            ctx.fan_out(if lit { "on" } else { "off" });
        }
        Ok(())
    })
}

/// Releases fader touches once a fader has been still for [`TOUCH_HOLD`].
fn touch_watcher() {
    loop {
        std::thread::sleep(Duration::from_millis(100));
        let due: Vec<usize> = {
            let mut t = TOUCHED.lock().unwrap();
            (0..9)
                .filter(|&i| match t[i] {
                    Some(at) if at.elapsed() >= TOUCH_HOLD => {
                        t[i] = None;
                        true
                    }
                    _ => false,
                })
                .collect()
        };
        for s in due {
            let _ = send(&[&mcu::touch(s, false)]);
        }
    }
}

fn move_fader(s: usize, value: u16) -> Result<()> {
    let first = TOUCHED.lock().unwrap()[s].replace(Instant::now()).is_none();
    if first {
        send(&[&mcu::touch(s, true)])?;
    }
    send(&[&mcu::fader(s, value)])?;
    // The DAW doesn't echo a touched fader: keep what was sent.
    MODEL.lock().unwrap().get_or_insert_with(mcu::Model::default).faders[s] = Some(value);
    Ok(())
}

fn bank_options() -> &'static [(&'static str, &'static str)] {
    &[("bank-right", "Next bank (8 tracks)"), ("bank-left", "Previous bank (8 tracks)"), ("channel-right", "Next track"), ("channel-left", "Previous track")]
}

fn bank_note(choice: &str) -> u8 {
    match choice {
        "bank-left" => mcu::BANK_LEFT,
        "channel-right" => mcu::CHANNEL_RIGHT,
        "channel-left" => mcu::CHANNEL_LEFT,
        _ => mcu::BANK_RIGHT,
    }
}

fn button_options() -> Vec<(String, String)> {
    mcu::BUTTONS.iter().map(|(n, l)| (n.to_string(), l.to_string())).collect()
}

fn main() {
    knobdeck_sdk::load_env();
    let os = std::env::consts::OS;
    if std::env::var("KNOBDECK_CODEGEN").as_deref() != Ok("1") {
        std::thread::spawn(|| {
            let _ = output(); // the virtual ports exist from the start, for the DAW to find
            touch_watcher();
        });
    }
    let button_opts = button_options();
    let button_refs: Vec<(&str, &str)> = button_opts.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let windows_help = if midi_io::HAS_VIRTUAL {
        "Leave both empty to use KnobDeck's own \"KnobDeck Mackie\" port (pick it as the Mackie Control's input and output in the DAW)."
    } else {
        "Windows has no virtual MIDI ports: create two in loopMIDI (e.g. \"KnobDeck to DAW\" and \"DAW to KnobDeck\"), name them here, and pick them as the Mackie Control's input and output in the DAW."
    };
    Plugin::new(&format!("daw-mackie-{os}"), "DAW Control (Mackie)")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[os]) // one package per OS
        .author("KnobDeck")
        .description("Control your DAW as a Mackie Control surface: transport, jog wheel, faders, V-Pots, banks, mute / solo / record arm / select, with the DAW's LEDs, fader positions, track names and time display.")
        .icon("sliders-vertical")
        .setup("daw-mackie-setup", None)
        .setup_inputs(vec![
            input::text("output", "Port to the DAW (optional on macOS / Linux)").placeholder("KnobDeck to DAW").with("help", windows_help),
            input::text("input", "Port from the DAW (optional on macOS / Linux)").placeholder("DAW to KnobDeck"),
        ])
        .action(led_toggle("play", "Play", mcu::PLAY, false, ("Play", "play"), ("Playing", "play")))
        .action(led_toggle("stop", "Stop", mcu::STOP, false, ("Stop", "square"), ("Stopped", "square")))
        .action(led_toggle("record", "Record", mcu::RECORD, false, ("Record", "circle"), ("Recording", "circle-dot")))
        .action(led_toggle("rewind", "Rewind", mcu::REWIND, false, ("Rewind", "rewind"), ("Rewinding", "rewind")))
        .action(led_toggle("fast-forward", "Fast Forward", mcu::FAST_FORWARD, false, ("Fast Forward", "fast-forward"), ("Forwarding", "fast-forward")))
        .action(led_toggle("cycle", "Loop", mcu::CYCLE, false, ("Loop Off", "repeat"), ("Loop On", "repeat")))
        .action(led_toggle("mute", "Mute Track", mcu::MUTE, true, ("Mute", "volume-2"), ("Muted", "volume-x")))
        .action(led_toggle("solo", "Solo Track", mcu::SOLO, true, ("Solo", "headphones"), ("Solo On", "headphones")))
        .action(led_toggle("rec-arm", "Record Arm Track", mcu::REC, true, ("Arm", "circle"), ("Armed", "circle-dot")))
        .action(led_toggle("select", "Select Track", mcu::SELECT, true, ("Select", "square-dashed"), ("Selected", "square-check")))
        .action(
            Action::slider("fader", "Fader", 0.0, 100.0)
                .icon("sliders-vertical")
                .unit("%")
                .inputs(vec![strip_input(true), input::text("step", "Step (%)").placeholder("2").default_value("2")])
                .live(POLL, |ic| {
                    alive()?;
                    let s = strip(&ic.inputs);
                    let pos = with_model(|m| m.faders[s]);
                    let v = Visual::new().label(with_name(if s == mcu::MASTER { "Master" } else { "Fader" }, s));
                    Ok(match pos {
                        Some(p) => v.state(mcu::fader_to_pct(p)),
                        None => v,
                    })
                })
                .on_adjust(|ctx| {
                    let s = strip(&ctx.inputs);
                    let now = with_model(|m| m.faders[s]).map(mcu::fader_to_pct).or_else(|| ctx.state.as_deref().and_then(|v| v.parse().ok())).unwrap_or(0.0);
                    let target = match ctx.value {
                        Some(v) => v,
                        None => now + step(ctx, 2.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .clamp(0.0, 100.0);
                    move_fader(s, mcu::pct_to_fader(target))?;
                    ctx.fan_out_by(&["strip"], mcu::fader_to_pct(mcu::pct_to_fader(target)));
                    Ok(())
                }),
        )
        .action(
            Action::slider("vpot", "V-Pot", 0.0, 11.0)
                .icon("circle-gauge")
                .step(1.0)
                .inputs(vec![strip_input(false), input::text("step", "Ticks per step").placeholder("1").default_value("1")])
                .live(POLL, |ic| {
                    alive()?;
                    let s = strip(&ic.inputs);
                    let v = Visual::new().label(with_name("V-Pot", s));
                    Ok(match with_model(|m| m.rings[s]) {
                        Some(r) => v.state(r),
                        None => v,
                    })
                })
                .on_adjust(|ctx| {
                    let s = strip(&ctx.inputs);
                    let ring = with_model(|m| m.rings[s]).map(f64::from).unwrap_or(6.0);
                    if let Some(m) = mcu::vpot(s, ticks(ctx, 1.0, ring)) {
                        send(&[&m])?;
                    }
                    Ok(())
                }),
        )
        .action(
            Action::slider("jog", "Jog Wheel", 0.0, 100.0)
                .icon("disc-3")
                .step(1.0)
                .inputs(vec![input::text("step", "Ticks per step").placeholder("1").default_value("1")])
                .live(POLL, |_| {
                    alive()?;
                    let t = with_model(|m| m.time());
                    Ok(Visual::new().label(if t.is_empty() { "Jog".to_string() } else { t }))
                })
                .on_adjust(|ctx| {
                    let last = ctx.state.as_deref().and_then(|v| v.parse::<f64>().ok()).unwrap_or(50.0);
                    if let Some(m) = mcu::jog(ticks(ctx, 1.0, last)) {
                        send(&[&m])?;
                    }
                    if let Some(v) = ctx.value {
                        ctx.set_state(v);
                    }
                    Ok(())
                }),
        )
        .action(
            Action::button("bank", "Bank / Track")
                .icon("arrow-right-left")
                .inputs(vec![input::select("move", "Move", bank_options()).default_value("bank-right")])
                .on_press(|ctx| tap(bank_note(&ctx.input_str("move")))),
        )
        .action(
            Action::slider("bank-dial", "Bank / Track (dial)", 0.0, 100.0)
                .icon("arrow-right-left")
                .step(1.0)
                .inputs(vec![input::select("by", "Turn moves by", &[("channel", "One track"), ("bank", "A bank of 8")]).default_value("channel")])
                .on_adjust(|ctx| {
                    let last = ctx.state.as_deref().and_then(|v| v.parse::<f64>().ok()).unwrap_or(50.0);
                    let n = ticks(ctx, 1.0, last).clamp(-8, 8);
                    let bank = ctx.input_str("by") == "bank";
                    let note = match (bank, n > 0) {
                        (true, true) => mcu::BANK_RIGHT,
                        (true, false) => mcu::BANK_LEFT,
                        (false, true) => mcu::CHANNEL_RIGHT,
                        (false, false) => mcu::CHANNEL_LEFT,
                    };
                    for _ in 0..n.abs() {
                        tap(note)?;
                    }
                    if let Some(v) = ctx.value {
                        ctx.set_state(v);
                    }
                    Ok(())
                }),
        )
        .action(
            Action::hold("button", "Mackie Button")
                .icon("square")
                .released("Button", "square")
                .held("Pressed", "square-check")
                .inputs(vec![input::select("note", "Button", &button_refs).default_value("80")])
                .live(POLL, |ic| {
                    alive()?;
                    let Some(n) = ic.input_str("note").parse::<u8>().ok() else { return Ok(Visual::new()) };
                    let name = mcu::BUTTONS.iter().find(|(b, _)| *b == n).map(|(_, l)| l.to_string()).unwrap_or_else(|| format!("Button {n}"));
                    Ok(Visual::new().label(name).icon(if with_model(|m| m.lit(n)) { "square-check" } else { "square" }))
                })
                .on_hold_start(|ctx| {
                    let Ok(n) = ctx.input_str("note").parse::<u8>() else { return ctx.fail("Choose a button") };
                    send(&[&mcu::button(n, true)])
                })
                .on_hold_end(|ctx| {
                    let Ok(n) = ctx.input_str("note").parse::<u8>() else { return Ok(()) };
                    send(&[&mcu::button(n, false)])
                }),
        )
        .action(
            Action::button("time", "Time Display")
                .icon("clock")
                .live(POLL, |_| {
                    alive()?;
                    let (t, heard) = with_model(|m| (m.time(), m.heard));
                    Ok(Visual::new().label(if !t.is_empty() {
                        t
                    } else if heard {
                        "—".into()
                    } else {
                        "No DAW yet".into()
                    }))
                })
                .on_press(|_| tap(mcu::SMPTE_BEATS)),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    #[test]
    fn strips_from_inputs() {
        assert_eq!(strip(&json!({"strip": "3"})), 2);
        assert_eq!(strip(&json!({"strip": "master"})), mcu::MASTER);
        assert_eq!(strip(&json!({})), 0);
        assert_eq!(strip(&json!({"strip": "12"})), 0);
    }

    /// Linux, real ALSA: a stand-in DAW on the other end of the "KnobDeck Mackie" ports. It sees the
    /// surface's presses and fader moves, and its LEDs, fader, LCD and time reach the model; the
    /// handshake is answered.
    #[cfg(target_os = "linux")]
    #[test]
    fn fake_daw_over_virtual_ports() {
        use std::sync::mpsc::channel;
        if midi_io::outputs().is_err() {
            eprintln!("no ALSA sequencer here: skipped");
            return;
        }
        std::env::remove_var("DAW_MACKIE_OUTPUT");
        std::env::remove_var("DAW_MACKIE_INPUT");
        output().unwrap();
        let name = format!("{PORT}:{PORT}");
        let (tx, rx) = channel();
        let _daw_in = midi_io::Input::connect("Fake DAW", &name, move |m| tx.send(m.to_vec()).unwrap()).unwrap();
        let daw_out = midi_io::Output::connect("Fake DAW", &name).unwrap();
        let next = || rx.recv_timeout(Duration::from_secs(2)).unwrap();

        // Surface → DAW.
        tap(mcu::PLAY).unwrap();
        assert_eq!(next(), vec![0x90, 0x5E, 0x7F]);
        assert_eq!(next(), vec![0x90, 0x5E, 0x00]);
        move_fader(2, 9000).unwrap();
        assert_eq!(next(), vec![0x90, 0x6A, 0x7F], "touch first");
        assert_eq!(next(), mcu::fader(2, 9000).to_vec());
        std::thread::spawn(touch_watcher);
        assert_eq!(next(), vec![0x90, 0x6A, 0x00], "touch released after the fader rests");
        send(&[&mcu::jog(-3).unwrap()]).unwrap();
        assert_eq!(next(), vec![0xB0, 0x3C, 0x43]);

        // DAW → surface.
        daw_out.send(&[0xF0, 0, 0, 0x66, 0x14, 0x00, 0xF7]).unwrap();
        let reply = next();
        assert_eq!(&reply[..6], &[0xF0, 0, 0, 0x66, 0x14, 0x01], "handshake answered");
        daw_out.send(&[0x90, mcu::MUTE + 1, 0x7F]).unwrap();
        daw_out.send(&mcu::fader(0, 16383)).unwrap();
        let mut lcd = vec![0xF0, 0, 0, 0x66, 0x14, 0x12, 7];
        lcd.extend(b"Guitar ");
        lcd.push(0xF7);
        daw_out.send(&lcd).unwrap();
        daw_out.send(&[0xB0, 0x40, b'7']).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !with_model(|m| m.lit(mcu::MUTE + 1) && m.faders[0] == Some(16383) && m.strip_name(1) == "Guitar" && m.time() == "7") {
            assert!(Instant::now() < deadline, "the DAW's state never reached the model");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(with_name("Muted", 1), "Muted\nGuitar");
    }
}
