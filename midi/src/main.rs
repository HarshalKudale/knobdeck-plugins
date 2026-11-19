//! MIDI plugin for KnobDeck: send notes, control changes, program changes and raw messages to any
//! MIDI port, turn a CC on a dial, and show received CCs / notes on a key. macOS and Linux get a
//! virtual "KnobDeck" port pair other apps connect to; Windows has none (see `midi_io`), so there
//! every action sends to / listens on an existing port the user picks (loopMIDI, an interface).

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Plugin, QueryCtx, Result, Value, Visual};
use midi_io::msg::{self, Message};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The name of KnobDeck's own virtual ports (an output others receive from, an input others send to).
const VIRTUAL: &str = "KnobDeck";
const POLL: Duration = Duration::from_millis(250);
/// After a port fails to open, wait this long before trying again (live tiles ask often).
const RETRY: Duration = Duration::from_secs(3);

/// Ports by the value their select holds: `""` is KnobDeck's virtual port.
static OUTPUTS: Mutex<Vec<(String, Arc<midi_io::Output>)>> = Mutex::new(Vec::new());
static INPUTS: Mutex<Vec<(String, midi_io::Input)>> = Mutex::new(Vec::new());
static FAILED: Mutex<Vec<(String, Instant, String)>> = Mutex::new(Vec::new());

/// The last value of each (direction, port, kind, channel, number) seen: what KnobDeck sent and
/// what came in. A dial / toggle shows whichever is newer.
static SEEN: Mutex<Option<HashMap<String, (u16, Instant)>>> = Mutex::new(None);
/// The last message received on each input, described ("CC 7 = 100 · ch 1").
static LAST: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn key(dir: &str, port: &str, kind: &str, ch: u8, num: u8) -> String {
    format!("{dir}|{port}|{kind}|{ch}|{num}")
}

fn remember(k: String, v: u16) {
    SEEN.lock().unwrap().get_or_insert_with(HashMap::new).insert(k, (v, Instant::now()));
}

fn seen(k: &str) -> Option<(u16, Instant)> {
    SEEN.lock().unwrap().as_ref().and_then(|m| m.get(k).copied())
}

fn recently_failed(port: &str, dir: &str) -> Option<String> {
    let id = format!("{dir}|{port}");
    let mut f = FAILED.lock().unwrap();
    f.retain(|(_, t, _)| t.elapsed() < RETRY);
    f.iter().find(|(p, _, _)| *p == id).map(|(_, _, e)| e.clone())
}

fn failed(port: &str, dir: &str, e: &str) {
    FAILED.lock().unwrap().push((format!("{dir}|{port}"), Instant::now(), e.to_string()));
}

fn output(port: &str) -> Result<Arc<midi_io::Output>> {
    if let Some((_, o)) = OUTPUTS.lock().unwrap().iter().find(|(p, _)| p == port) {
        return Ok(o.clone());
    }
    if let Some(e) = recently_failed(port, "out") {
        return Err(Error::msg(e));
    }
    let opened = if port.is_empty() { midi_io::Output::open_virtual(VIRTUAL, VIRTUAL) } else { midi_io::Output::connect(VIRTUAL, port) };
    match opened {
        Ok(o) => {
            let o = Arc::new(o);
            OUTPUTS.lock().unwrap().push((port.to_string(), o.clone()));
            Ok(o)
        }
        Err(e) => {
            failed(port, "out", &e);
            Err(Error::msg(e))
        }
    }
}

/// Send `messages` to `port`, and remember the channel values sent. A port that went away (an
/// interface unplugged) is opened again once.
fn send(port: &str, messages: &[&[u8]]) -> Result<()> {
    let attempt = || -> std::result::Result<(), String> {
        let o = output(port).map_err(|e| e.message().to_string())?;
        messages.iter().try_for_each(|m| o.send(m))
    };
    if let Err(e) = attempt() {
        if port.is_empty() {
            return Err(Error::msg(e));
        }
        OUTPUTS.lock().unwrap().retain(|(p, _)| p != port);
        attempt().map_err(Error::msg)?;
    }
    for m in messages {
        if let Some(parsed) = msg::parse(m) {
            note_value("out", port, parsed);
        }
    }
    Ok(())
}

/// Record a channel message's value under its key.
fn note_value(dir: &str, port: &str, m: Message) {
    match m {
        Message::Cc { ch, controller, value } => remember(key(dir, port, "cc", ch, controller), value as u16),
        Message::NoteOn { ch, note, velocity } => remember(key(dir, port, "note", ch, note), velocity as u16),
        Message::NoteOff { ch, note } => remember(key(dir, port, "note", ch, note), 0),
        Message::Program { ch, program } => remember(key(dir, port, "program", ch, 0), program as u16),
        Message::PitchBend { ch, value } => remember(key(dir, port, "pitch", ch, 0), value),
        _ => {}
    }
}

fn describe(m: &[u8]) -> String {
    let ch = |c: u8| format!(" · ch {}", c + 1);
    match msg::parse(m) {
        Some(Message::NoteOn { ch: c, note, velocity }) => format!("{} on {velocity}{}", msg::note_name(note), ch(c)),
        Some(Message::NoteOff { ch: c, note }) => format!("{} off{}", msg::note_name(note), ch(c)),
        Some(Message::Cc { ch: c, controller, value }) => format!("CC {controller} = {value}{}", ch(c)),
        Some(Message::Program { ch: c, program }) => format!("Program {}{}", program as u16 + 1, ch(c)),
        Some(Message::PitchBend { ch: c, value }) => format!("Bend {}{}", value as i32 - 8192, ch(c)),
        Some(Message::Aftertouch { ch: c, value }) => format!("Pressure {value}{}", ch(c)),
        Some(Message::PolyAftertouch { ch: c, note, value }) => format!("{} pressure {value}{}", msg::note_name(note), ch(c)),
        None if m.first() == Some(&0xF0) => format!("SysEx, {} bytes", m.len()),
        None => m.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "),
    }
}

/// Start listening on `port` (once; kept open while the plugin runs).
fn listen(port: &str) -> Result<()> {
    if INPUTS.lock().unwrap().iter().any(|(p, _)| p == port) {
        return Ok(());
    }
    if let Some(e) = recently_failed(port, "in") {
        return Err(Error::msg(e));
    }
    let name = port.to_string();
    let on_message = move |m: &[u8]| {
        if let Some(parsed) = msg::parse(m) {
            note_value("in", &name, parsed);
        }
        if !matches!(m.first(), Some(0xF8..=0xFF)) {
            LAST.lock().unwrap().get_or_insert_with(HashMap::new).insert(name.clone(), describe(m));
        }
    };
    let opened = if port.is_empty() { midi_io::Input::open_virtual(VIRTUAL, VIRTUAL, on_message) } else { midi_io::Input::connect(VIRTUAL, port, on_message) };
    match opened {
        Ok(i) => {
            INPUTS.lock().unwrap().push((port.to_string(), i));
            Ok(())
        }
        Err(e) => {
            failed(port, "in", &e);
            Err(Error::msg(e))
        }
    }
}

// ---- inputs ----

const CHANNELS: [(&str, &str); 16] = [
    ("1", "Channel 1"),
    ("2", "Channel 2"),
    ("3", "Channel 3"),
    ("4", "Channel 4"),
    ("5", "Channel 5"),
    ("6", "Channel 6"),
    ("7", "Channel 7"),
    ("8", "Channel 8"),
    ("9", "Channel 9"),
    ("10", "Channel 10"),
    ("11", "Channel 11"),
    ("12", "Channel 12"),
    ("13", "Channel 13"),
    ("14", "Channel 14"),
    ("15", "Channel 15"),
    ("16", "Channel 16"),
];

fn port_input() -> Value {
    let i = input::query_select("port", "Send to", "outputs", &[]);
    if midi_io::HAS_VIRTUAL {
        i.with("default", "")
    } else {
        i.with("help", "Windows has no virtual MIDI ports: pick a port, e.g. one made with loopMIDI.")
    }
}

fn in_port_input(key: &str, label: &str) -> Value {
    let i = input::query_select(key, label, "inputs", &[]);
    if midi_io::HAS_VIRTUAL {
        i.with("default", "")
    } else {
        i.with("help", "Windows has no virtual MIDI ports: pick a port, e.g. one made with loopMIDI.")
    }
}

fn feedback_input() -> Value {
    input::query_select("feedback", "Follow values from (optional)", "feedback", &[])
        .with("default", "none")
        .with("help", "Keep the key in step with this controller when it comes in on a port (a DAW or synth sending it back).")
}

fn channel_input() -> Value {
    input::select("channel", "Channel", &CHANNELS).default_value("1")
}

fn number_input(key: &str, label: &str, default: &str) -> Value {
    input::text(key, label).placeholder(default).default_value(default)
}

fn channel(ctx_inputs: &Value) -> u8 {
    ctx_inputs["channel"].as_str().and_then(|c| c.parse::<u8>().ok()).or_else(|| ctx_inputs["channel"].as_u64().map(|c| c as u8)).filter(|c| (1..=16).contains(c)).unwrap_or(1) - 1
}

fn port(inputs: &Value) -> String {
    inputs["port"].as_str().unwrap_or("").to_string()
}

/// A 0–127 number input (`what` names it in the error).
fn byte(inputs: &Value, key: &str, default: u8, what: &str) -> Result<u8> {
    let raw = input_str(inputs, key);
    if raw.trim().is_empty() {
        return Ok(default);
    }
    raw.trim().parse::<u8>().ok().filter(|v| *v <= 127).ok_or_else(|| Error::msg(format!("{what} must be 0–127")))
}

/// An input as text ("" when absent), numbers included.
fn input_str(inputs: &Value, key: &str) -> String {
    match &inputs[key] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}

fn ensure_port(p: &str) -> Result<()> {
    if p.is_empty() && !midi_io::HAS_VIRTUAL {
        return Err(Error::msg(midi_io::NO_VIRTUAL_ON_WINDOWS));
    }
    Ok(())
}

/// The value of a controller: the newer of what KnobDeck sent and what came back on `feedback`.
fn cc_value(inputs: &Value, controller: u8) -> Option<u16> {
    let ch = channel(inputs);
    let sent = seen(&key("out", &port(inputs), "cc", ch, controller));
    let fb = inputs["feedback"].as_str().filter(|f| *f != "none" && !f.is_empty()).map(|f| f.trim_start_matches("in:").to_string());
    let back = fb.and_then(|f| {
        let _ = listen(&f);
        seen(&key("in", &f, "cc", ch, controller))
    });
    match (sent, back) {
        (Some(a), Some(b)) => Some(if b.1 >= a.1 { b.0 } else { a.0 }),
        (a, b) => a.or(b).map(|v| v.0),
    }
}

fn port_options(q: &QueryCtx, list: midi_io::Result<Vec<String>>, virtual_label: &str) -> Result<Value> {
    let list = list.or_else(|e| q.fail(e))?;
    let own = [VIRTUAL.to_string(), format!("{VIRTUAL}:{VIRTUAL}")];
    let mut opts = Vec::new();
    if midi_io::HAS_VIRTUAL {
        opts.push(json!({"value": "", "label": virtual_label}));
    }
    opts.extend(list.into_iter().filter(|p| !own.contains(p)).map(|p| json!({"value": p, "label": p})));
    if opts.is_empty() {
        return q.fail("No MIDI ports: Windows has no virtual ones, so create one with loopMIDI (or Windows MIDI Services) or connect an interface");
    }
    Ok(Value::Array(opts))
}

fn kind_options() -> &'static [(&'static str, &'static str)] {
    &[("cc", "Control Change"), ("note", "Note"), ("program", "Program Change"), ("pitch", "Pitch Bend"), ("any", "Any message")]
}

fn received(inputs: &Value) -> Result<Visual> {
    let p = inputs["port"].as_str().unwrap_or("").to_string();
    ensure_port(&p)?;
    listen(&p)?;
    let kind = inputs["kind"].as_str().filter(|k| !k.is_empty()).unwrap_or("cc");
    if kind == "any" {
        let last = LAST.lock().unwrap().as_ref().and_then(|m| m.get(&p).cloned());
        return Ok(Visual::new().label(last.unwrap_or_else(|| "Waiting…".into())));
    }
    let raw_ch = inputs["channel"].as_str().unwrap_or("any");
    let num = match kind {
        "cc" => byte(inputs, "number", 7, "Controller")?,
        "note" => {
            let t = input_str(inputs, "number");
            if t.trim().is_empty() {
                60
            } else {
                msg::parse_note(&t).ok_or_else(|| Error::msg("Note must be 0–127 or a name like C4"))?
            }
        }
        _ => 0,
    };
    // "Any channel": the newest value on any channel.
    let chans: Vec<u8> = if raw_ch == "any" || raw_ch.is_empty() { (0..16).collect() } else { vec![channel(inputs)] };
    let newest = chans.iter().filter_map(|c| seen(&key("in", &p, kind, *c, num))).max_by_key(|v| v.1).map(|v| v.0);
    let name = match kind {
        "cc" => format!("CC {num}"),
        "note" => msg::note_name(num),
        "program" => "Program".into(),
        _ => "Bend".into(),
    };
    let value = match (kind, newest) {
        (_, None) => "—".to_string(),
        ("note", Some(0)) => "off".into(),
        ("note", Some(v)) => format!("on {v}"),
        ("program", Some(v)) => (v + 1).to_string(),
        ("pitch", Some(v)) => (v as i32 - 8192).to_string(),
        (_, Some(v)) => v.to_string(),
    };
    let lit = matches!((kind, newest), ("note", Some(v)) if v > 0);
    Ok(Visual::new().label(format!("{name}\n{value}")).icon(if lit { "circle-dot" } else { "activity" }))
}

fn main() {
    knobdeck_sdk::load_env();
    let os = std::env::consts::OS;
    if std::env::var("KNOBDECK_CODEGEN").as_deref() != Ok("1") && midi_io::HAS_VIRTUAL {
        // The virtual ports exist from the start, so apps can be connected to them before a key is used.
        std::thread::spawn(|| {
            let _ = output("");
            let _ = listen("");
        });
    }
    Plugin::new(&format!("midi-{os}"), "MIDI")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[os]) // one package per OS
        .author("KnobDeck")
        .description("Send MIDI notes, control changes, program changes and raw messages to any MIDI port, turn a CC on a dial, and show incoming CCs and notes on a key.")
        .icon("app-icon.svg")
        .query("outputs", |q| port_options(q, midi_io::outputs(), "KnobDeck (virtual port)"))
        .query("inputs", |q| port_options(q, midi_io::inputs(), "KnobDeck (virtual port)"))
        .query("feedback", |q| {
            let mut opts = vec![json!({"value": "none", "label": "Nothing (show what was sent)"})];
            if let Ok(Value::Array(list)) = port_options(q, midi_io::inputs(), "KnobDeck (virtual port)") {
                opts.extend(list.into_iter().map(|o| json!({"value": format!("in:{}", o["value"].as_str().unwrap_or("")), "label": o["label"]})));
            }
            Ok(Value::Array(opts))
        })
        .action(
            Action::hold("note", "Note")
                .icon("piano")
                .released("Note", "piano")
                .held("Playing", "music")
                .inputs(vec![
                    port_input(),
                    channel_input(),
                    number_input("note", "Note (number or name)", "C4").with("help", "0–127, or a name: C4 is 60 (middle C), C#4 / Db4 is 61."),
                    number_input("velocity", "Velocity", "100"),
                ])
                .on_hold_start(|ctx| {
                    let (p, ch, note, vel) = note_inputs(ctx)?;
                    send(&p, &[&msg::note_on(ch, note, vel.max(1))])?;
                    ctx.set_state("held");
                    Ok(())
                })
                .on_hold_end(|ctx| {
                    let (p, ch, note, _) = note_inputs(ctx)?;
                    send(&p, &[&msg::note_off(ch, note)])?;
                    ctx.set_state("released");
                    Ok(())
                }),
        )
        .action(
            Action::button("cc", "Control Change")
                .icon("sliders-horizontal")
                .inputs(vec![port_input(), channel_input(), number_input("controller", "Controller (CC number)", "7"), number_input("value", "Value", "127")])
                .on_press(|ctx| {
                    let p = port(&ctx.inputs);
                    ensure_port(&p)?;
                    let cc = byte(&ctx.inputs, "controller", 7, "Controller")?;
                    let v = byte(&ctx.inputs, "value", 127, "Value")?;
                    send(&p, &[&msg::cc(channel(&ctx.inputs), cc, v)])
                }),
        )
        .action(
            Action::toggle("cc-toggle", "CC Toggle")
                .icon("toggle-right")
                .state("off", "Off", "toggle-left")
                .state("on", "On", "toggle-right")
                .inputs(vec![
                    port_input(),
                    channel_input(),
                    number_input("controller", "Controller (CC number)", "64"),
                    number_input("on", "On value", "127"),
                    number_input("off", "Off value", "0"),
                    feedback_input(),
                ])
                .live(POLL, |ic| {
                    let cc = byte(&ic.inputs, "controller", 64, "Controller")?;
                    let off = byte(&ic.inputs, "off", 0, "Off value")? as u16;
                    Ok(match cc_value(&ic.inputs, cc) {
                        Some(v) => Visual::new().state(if v != off { "on" } else { "off" }),
                        None => Visual::new(),
                    })
                })
                .on_press(|ctx| {
                    let p = port(&ctx.inputs);
                    ensure_port(&p)?;
                    let cc = byte(&ctx.inputs, "controller", 64, "Controller")?;
                    let on = byte(&ctx.inputs, "on", 127, "On value")?;
                    let off = byte(&ctx.inputs, "off", 0, "Off value")?;
                    let now_on = cc_value(&ctx.inputs, cc).map(|v| v != off as u16).unwrap_or(ctx.state.as_deref() == Some("on"));
                    send(&p, &[&msg::cc(channel(&ctx.inputs), cc, if now_on { off } else { on })])?;
                    ctx.fan_out_by(&["port", "channel", "controller"], if now_on { "off" } else { "on" });
                    Ok(())
                }),
        )
        .action(
            Action::slider("cc-dial", "CC Dial", 0.0, 127.0)
                .icon("circle-gauge")
                .step(1.0)
                .inputs(vec![port_input(), channel_input(), number_input("controller", "Controller (CC number)", "1"), number_input("step", "Step", "1"), feedback_input()])
                .live(POLL, |ic| {
                    let cc = byte(&ic.inputs, "controller", 1, "Controller")?;
                    Ok(match cc_value(&ic.inputs, cc) {
                        Some(v) => Visual::new().state(v),
                        None => Visual::new(),
                    })
                })
                .on_adjust(|ctx| {
                    let p = port(&ctx.inputs);
                    ensure_port(&p)?;
                    let cc = byte(&ctx.inputs, "controller", 1, "Controller")?;
                    let step = ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(1.0);
                    let now = cc_value(&ctx.inputs, cc).map(f64::from).or_else(|| ctx.state.as_deref().and_then(|s| s.parse().ok())).unwrap_or(0.0);
                    let target = match ctx.value {
                        Some(v) => v,
                        None => now + step * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(0.0, 127.0);
                    send(&p, &[&msg::cc(channel(&ctx.inputs), cc, target as u8)])?;
                    ctx.fan_out_by(&["port", "channel", "controller"], target);
                    Ok(())
                }),
        )
        .action(
            Action::button("program", "Program Change")
                .icon("list-music")
                .inputs(vec![
                    port_input(),
                    channel_input(),
                    number_input("program", "Program (1–128)", "1"),
                    input::text("bank", "Bank (optional, 1–16384)").with("help", "Sends Bank Select (CC 0 and CC 32) first."),
                ])
                .on_press(|ctx| {
                    let p = port(&ctx.inputs);
                    ensure_port(&p)?;
                    let ch = channel(&ctx.inputs);
                    let prog = ctx.input_str("program");
                    let Some(prog) = (if prog.trim().is_empty() { Some(1) } else { prog.trim().parse::<u16>().ok() }).filter(|v| (1..=128).contains(v)) else {
                        return ctx.fail("Program must be 1–128");
                    };
                    let pc = msg::program(ch, (prog - 1) as u8);
                    let bank = ctx.input_str("bank");
                    if bank.trim().is_empty() {
                        return send(&p, &[&pc]);
                    }
                    let Some(b) = bank.trim().parse::<u16>().ok().filter(|v| (1..=16384).contains(v)).map(|b| b - 1) else {
                        return ctx.fail("Bank must be 1–16384");
                    };
                    send(&p, &[&msg::cc(ch, 0, (b >> 7) as u8), &msg::cc(ch, 32, (b & 0x7F) as u8), &pc])
                }),
        )
        .action(
            Action::button("raw", "Send MIDI Message")
                .icon("binary")
                .inputs(vec![
                    port_input(),
                    input::text("bytes", "Bytes (hex)").placeholder("90 3C 64").with("help", "One or more messages in hex, e.g. B0 07 64 for CC 7 = 100, or a SysEx F0 … F7."),
                ])
                .on_press(|ctx| {
                    let p = port(&ctx.inputs);
                    ensure_port(&p)?;
                    let Some(bytes) = msg::parse_hex(&ctx.input_str("bytes")) else {
                        return ctx.fail("Enter bytes in hex, like 90 3C 64");
                    };
                    let messages = msg::split(&bytes).map_err(Error::msg)?;
                    let refs: Vec<&[u8]> = messages.iter().map(Vec::as_slice).collect();
                    send(&p, &refs)
                }),
        )
        .action(
            Action::button("received", "Received Value")
                .icon("activity")
                .inputs(vec![
                    in_port_input("port", "Listen on"),
                    input::select("kind", "Message", kind_options()).default_value("cc"),
                    input::select("channel", "Channel", &[("any", "Any channel")]).with("options", channel_options_with_any()).default_value("any"),
                    input::text("number", "Controller or note").placeholder("7").with("help", "The CC number, or the note (number or name, like C4). Not used for Program Change, Pitch Bend or Any message."),
                ])
                .live(POLL, |ic| received(&ic.inputs))
                .on_press(|_| Ok(())),
        )
        .run()
}

fn channel_options_with_any() -> Value {
    let mut v = vec![json!({"value": "any", "label": "Any channel"})];
    v.extend(CHANNELS.iter().map(|(value, label)| json!({"value": value, "label": label})));
    Value::Array(v)
}

fn note_inputs(ctx: &Ctx) -> Result<(String, u8, u8, u8)> {
    let p = port(&ctx.inputs);
    ensure_port(&p)?;
    let t = ctx.input_str("note");
    let note = if t.trim().is_empty() { 60 } else { msg::parse_note(&t).ok_or_else(|| Error::msg("Note must be 0–127 or a name like C4"))? };
    let vel = byte(&ctx.inputs, "velocity", 100, "Velocity")?;
    Ok((p, channel(&ctx.inputs), note, vel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_messages() {
        assert_eq!(describe(&[0xB0, 7, 100]), "CC 7 = 100 · ch 1");
        assert_eq!(describe(&[0x91, 60, 90]), "C4 on 90 · ch 2");
        assert_eq!(describe(&[0x90, 60, 0]), "C4 off · ch 1");
        assert_eq!(describe(&[0xC0, 4]), "Program 5 · ch 1");
        assert_eq!(describe(&[0xE0, 0, 0x40]), "Bend 0 · ch 1");
        assert_eq!(describe(&[0xF0, 1, 2, 0xF7]), "SysEx, 4 bytes");
    }

    #[test]
    fn channel_and_bytes_from_inputs() {
        assert_eq!(channel(&json!({"channel": "10"})), 9);
        assert_eq!(channel(&json!({})), 0);
        assert_eq!(channel(&json!({"channel": "99"})), 0);
        assert_eq!(byte(&json!({"v": "64"}), "v", 0, "V").unwrap(), 64);
        assert_eq!(byte(&json!({}), "v", 5, "V").unwrap(), 5);
        assert!(byte(&json!({"v": "128"}), "v", 0, "V").is_err());
    }

    /// Linux, real ALSA: a key's CC goes out of the virtual port, and a CC sent into the virtual
    /// input shows on a Received Value tile.
    #[cfg(target_os = "linux")]
    #[test]
    fn virtual_ports_round_trip() {
        if midi_io::outputs().is_err() {
            eprintln!("no ALSA sequencer here: skipped");
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        output("").unwrap();
        let _tap = midi_io::Input::connect("test tap", "KnobDeck:KnobDeck", move |m| tx.send(m.to_vec()).unwrap()).unwrap();
        send("", &[&msg::cc(0, 7, 100)]).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), vec![0xB0, 7, 100]);
        assert_eq!(cc_value(&json!({"port": "", "channel": "1"}), 7), Some(100));

        let tile = json!({"port": "", "kind": "cc", "channel": "any", "number": "11"});
        received(&tile).unwrap(); // starts listening
        let into = midi_io::Output::connect("test source", "KnobDeck:KnobDeck").unwrap();
        into.send(&msg::cc(4, 11, 42)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let v = format!("{:?}", received(&tile).unwrap());
            if v.contains("CC 11\\n42") {
                break;
            }
            assert!(Instant::now() < deadline, "the tile never showed the CC: {v}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
