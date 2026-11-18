//! Soundboard plugin for KnobDeck: play mp3 / wav / ogg / flac files from a key to any output
//! device (and a second one, e.g. a virtual cable into your mic plus your headphones), a master
//! volume dial, and on Windows Soundpad through its remote control pipe.

mod engine;
#[cfg(not(target_os = "linux"))]
mod out_cpal;
#[cfg(target_os = "linux")]
mod out_pulse;
mod soundpad;

use engine::{Mode, Sound, DEFAULT};
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::path::PathBuf;
use std::time::Duration;

const NONE: &str = "";

fn err(e: String) -> Error {
    Error::msg(e)
}

fn options(with_none: bool) -> Result<Value> {
    let mut opts = vec![];
    if with_none {
        opts.push(json!({"value": NONE, "label": "None"}));
    }
    opts.extend(engine::output_options().map_err(err)?.into_iter().map(|(v, l)| json!({"value": v, "label": l})));
    Ok(Value::Array(opts))
}

fn sound_inputs() -> Vec<Value> {
    vec![
        // A text field: the SDK's input validator has no "file" type yet (the host renders one with
        // a Browse button).
        input::text("file", "Sound file (mp3, wav, ogg, flac)").placeholder(if cfg!(windows) { "C:\\Users\\me\\Sounds\\airhorn.mp3" } else { "~/Sounds/airhorn.mp3" }),
        input::query_select("output", "Output device", "outputs", &[]).default_value(DEFAULT),
        input::query_select("output2", "Also play on", "outputs-or-none", &[])
            .default_value(NONE)
            .with("help", "A second device, e.g. a virtual cable into your microphone (VB-Cable, BlackHole, a PipeWire null sink) plus your headphones."),
        input::text("volume", "Volume (%)").placeholder("100").default_value("100"),
    ]
}

fn percent(raw: &str) -> f32 {
    raw.trim().trim_end_matches('%').trim().parse::<f32>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 200.0) / 100.0).unwrap_or(1.0)
}

fn sound(ctx: &Ctx) -> Result<Sound> {
    let file = ctx.input_str("file");
    let file = file.trim().trim_matches('"');
    if file.is_empty() {
        return ctx.fail("Choose a sound file");
    }
    let mut outputs = vec![];
    for key in ["output", "output2"] {
        let v = ctx.input_str(key);
        let v = if key == "output" && v.is_empty() { DEFAULT.to_string() } else { v };
        if !v.is_empty() && !outputs.contains(&v) {
            outputs.push(v);
        }
    }
    let path = match file.strip_prefix("~/") {
        Some(rest) => std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(|h| PathBuf::from(h).join(rest)).unwrap_or_else(|| PathBuf::from(file)),
        None => PathBuf::from(file),
    };
    Ok(Sound { path, outputs, gain: percent(&ctx.input_str("volume")), looping: false })
}

fn soundpad_fail(e: String) -> Error {
    Error::msg(e)
}

fn main() {
    Plugin::new(&format!("soundboard-{}", std::env::consts::OS), "Soundboard")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Play sound files from a key to any output device, or two at once (your headphones and a virtual cable into your mic). Master volume dial; Soundpad on Windows.")
        .icon("audio-lines")
        .query("outputs", |q| options(false).or_else(|e| q.fail(e.message().to_string())))
        .query("outputs-or-none", |q| options(true).or_else(|e| q.fail(e.message().to_string())))
        .query("soundpad-sounds", |q| {
            let list = soundpad::sounds().or_else(|e| q.fail(e))?;
            if list.is_empty() {
                return q.fail("Soundpad's sound list is empty");
            }
            Ok(Value::Array(list.into_iter().map(|s| json!({"value": s.index.to_string(), "label": format!("{} · {}", s.index, s.title)})).collect()))
        })
        .action(
            Action::button("play", "Play Sound")
                .icon("play")
                .inputs(
                    [
                        sound_inputs(),
                        vec![input::select(
                            "mode",
                            "When pressed while playing",
                            &[("restart", "Restart it"), ("stop", "Stop it (press again to play)"), ("overlap", "Play another copy over it"), ("loop", "Loop it until pressed again")],
                        )
                        .default_value("restart")],
                    ]
                    .concat(),
                )
                .on_press(|ctx| {
                    let s = sound(ctx)?;
                    engine::press(&ctx.instance_id, Mode::parse(&ctx.input_str("mode")), s).map_err(err).map(|_| ())
                }),
        )
        .action(
            Action::hold("hold", "Hold to Play")
                .icon("play")
                .released("Hold to play", "play")
                .held("Playing", "volume-2")
                .inputs([sound_inputs(), vec![input::checkbox("loop", "Loop while held").default_value(true)]].concat())
                .on_hold_start(|ctx| {
                    let mut s = sound(ctx)?;
                    s.looping = ctx.input_bool("loop");
                    engine::start(&ctx.instance_id, s).map_err(err)
                })
                .on_hold_end(|ctx| {
                    engine::stop(&ctx.instance_id);
                    Ok(())
                }),
        )
        .action(Action::button("stop-all", "Stop All Sounds").icon("square").on_press(|_| {
            engine::stop_all();
            Ok(())
        }))
        .action(
            Action::slider("master-volume", "Soundboard Volume", 0.0, 100.0)
                .icon("volume-2")
                .unit("%")
                .inputs(vec![input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .initial_with(|_| Some((engine::master() * 100.0).round().to_string()))
                .live(Duration::from_secs(2), |_| Ok(Visual::new().state((engine::master() * 100.0).round())))
                .on_adjust(|ctx| {
                    let step = ctx.input_str("step").trim().parse::<f64>().ok().filter(|s| *s > 0.0).unwrap_or(5.0);
                    let now = engine::master() as f64 * 100.0;
                    let next = match ctx.value {
                        Some(v) => v,
                        None => now + ctx.delta.unwrap_or(0.0) * step,
                    }
                    .clamp(0.0, 100.0)
                    .round();
                    engine::set_master(next as f32 / 100.0);
                    ctx.fan_out(next);
                    Ok(())
                }),
        )
        // Soundpad's own remote control (Windows; Soundpad has no macOS or Linux build).
        .action(
            Action::button("soundpad-play", "Soundpad: Play Sound")
                .icon("play")
                .platforms(&["windows"])
                .inputs(vec![
                    input::query_select("sound", "Sound", "soundpad-sounds", &[]),
                    input::select("lines", "Play on", &[("both", "Speakers and microphone"), ("speakers", "Speakers only"), ("mic", "Microphone only")]).default_value("both"),
                ])
                .on_press(|ctx| {
                    let i: u32 = ctx.input_str("sound").trim().parse().map_err(|_| Error::msg("Choose a Soundpad sound"))?;
                    let (spk, mic) = soundpad::lines(&ctx.input_str("lines"));
                    soundpad::run(&format!("DoPlaySound({i}, {spk}, {mic})")).map_err(soundpad_fail)
                }),
        )
        .action(
            Action::button("soundpad-stop", "Soundpad: Stop")
                .icon("square")
                .platforms(&["windows"])
                .on_press(|_| soundpad::run("DoStopSound()").map_err(soundpad_fail)),
        )
        .action(
            Action::button("soundpad-pause", "Soundpad: Pause / Resume")
                .icon("pause")
                .platforms(&["windows"])
                .on_press(|_| soundpad::run("DoTogglePause()").map_err(soundpad_fail)),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::percent;

    #[test]
    fn volumes() {
        assert_eq!(percent("100"), 1.0);
        assert_eq!(percent(" 50 % "), 0.5);
        assert_eq!(percent(""), 1.0);
        assert_eq!(percent("abc"), 1.0);
        assert_eq!(percent("-5"), 0.0);
        assert_eq!(percent("500"), 2.0);
    }
}
