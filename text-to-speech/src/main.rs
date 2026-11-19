//! Text to Speech plugin for KnobDeck: read the selected text, the clipboard or any text aloud
//! with the OS's own voices, stop it, pick the voice, and turn rate and volume on dials.
//! Windows: WinRT SpeechSynthesizer; macOS: the system synthesizer through `say`; Linux: Speech
//! Dispatcher over SSIP.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod os;
#[cfg(target_os = "macos")]
#[path = "mac.rs"]
mod os;
#[cfg(windows)]
#[path = "win.rs"]
mod os;
// `say`'s voice list parser, tested on every OS.
#[cfg(all(test, not(target_os = "macos")))]
#[allow(dead_code)]
#[path = "mac.rs"]
mod mac_say;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Need, Plugin, Result, Value, Visual};
use std::sync::Mutex;
use std::time::Duration;

/// One voice the OS offers: `id` is what the OS calls it, `label` what the picker shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Voice {
    pub id: String,
    pub label: String,
}

/// The plugin-wide voice, rate (1.0 = normal) and volume (0–100), kept in `settings.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub voice: String,
    pub rate: f64,
    pub volume: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { voice: String::new(), rate: 1.0, volume: 100.0 }
    }
}

const RATE_MIN: f64 = 50.0;
const RATE_MAX: f64 = 300.0;

static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);

fn settings_file() -> std::path::PathBuf {
    knobdeck_sdk::data_dir().join("settings.json")
}

fn parse_settings(text: &str) -> Settings {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let d = Settings::default();
    Settings {
        voice: v["voice"].as_str().unwrap_or("").to_string(),
        rate: v["rate"].as_f64().filter(|r| r.is_finite()).map(|r| r.clamp(RATE_MIN / 100.0, RATE_MAX / 100.0)).unwrap_or(d.rate),
        volume: v["volume"].as_f64().filter(|r| r.is_finite()).map(|r| r.clamp(0.0, 100.0)).unwrap_or(d.volume),
    }
}

fn settings() -> Settings {
    SETTINGS.lock().unwrap().get_or_insert_with(|| std::fs::read_to_string(settings_file()).map(|t| parse_settings(&t)).unwrap_or_default()).clone()
}

fn update(f: impl FnOnce(&mut Settings)) -> Settings {
    let mut s = settings();
    f(&mut s);
    *SETTINGS.lock().unwrap() = Some(s.clone());
    let text = json!({"voice": s.voice, "rate": s.rate, "volume": s.volume}).to_string();
    if let Err(e) = std::fs::write(settings_file(), text) {
        knobdeck_sdk::log(format!("couldn't save settings: {e}"));
    }
    s
}

fn err(e: String) -> Error {
    Error::msg(e)
}

fn voice_options(with_default: bool) -> Result<Value> {
    let mut list = os::voices().map_err(err)?;
    list.sort_by_key(|a| a.label.to_lowercase());
    let mut opts = vec![];
    if with_default {
        opts.push(json!({"value": "", "label": "The voice chosen for the plugin"}));
    }
    opts.extend(list.into_iter().map(|v| json!({"value": v.id, "label": v.label})));
    Ok(Value::Array(opts))
}

/// The text a Speak key reads, placeholders expanded; a clear error when it comes out empty.
fn text_to_speak(ctx: &Ctx) -> Result<String> {
    let raw = ctx.input_str("text");
    let raw = if raw.trim().is_empty() { "{selection}".to_string() } else { raw };
    let text = ctx.expand(&raw)?;
    if !text.trim().is_empty() {
        return Ok(text);
    }
    if raw.contains("{selection}") {
        return ctx.fail(ctx.context_error(Need::Selection).unwrap_or_else(|| "Nothing is selected".into()));
    }
    if raw.contains("{clipboard}") {
        return ctx.fail("The clipboard has no text");
    }
    ctx.fail("Nothing to speak")
}

fn percent_step(ctx: &Ctx, default: f64) -> f64 {
    ctx.input_str("step").trim().trim_end_matches('%').trim().parse::<f64>().ok().filter(|s| *s > 0.0 && s.is_finite()).unwrap_or(default)
}

/// A slider's next value: the dragged one, or the current one nudged by steps.
fn next_value(ctx: &Ctx, now: f64, step: f64, min: f64, max: f64) -> f64 {
    match ctx.value {
        Some(v) => v,
        None => now + ctx.delta.unwrap_or(0.0) * step,
    }
    .clamp(min, max)
    .round()
}

fn step_input(default: &str) -> Value {
    input::text("step", "Step (%)").placeholder(default).default_value(default)
}

fn main() {
    Plugin::new(&format!("text-to-speech-{}", std::env::consts::OS), "Text to Speech")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Read the selected text, the clipboard or your own text aloud with your computer's voices. Stop it, pick the voice, and set rate and volume on dials.")
        .icon("app-icon.svg")
        .query("voices", |q| voice_options(false).or_else(|e| q.fail(e.message().to_string())))
        .query("voices-or-default", |q| voice_options(true).or_else(|e| q.fail(e.message().to_string())))
        .action(
            Action::button("speak", "Speak Text")
                .icon("speech")
                .needs(&[Need::Selection])
                .inputs(vec![
                    input::textarea("text", "Text")
                        .placeholder("{selection}")
                        .default_value("{selection}")
                        .with("help", "What to read aloud: {selection} (the selected text), {clipboard}, your own words, or both, like \"It is {time}\".")
                        .accepts_placeholders(),
                    input::query_select("voice", "Voice", "voices-or-default", &[]).default_value(""),
                    input::select("mode", "When pressed while speaking", &[("stop", "Stop (press again to start over)"), ("restart", "Speak this instead")]).default_value("stop"),
                ])
                .on_press(|ctx| {
                    if os::speaking() && ctx.input_str("mode") != "restart" {
                        return os::stop().map_err(err);
                    }
                    let text = text_to_speak(ctx)?;
                    let mut s = settings();
                    let voice = ctx.input_str("voice");
                    if !voice.is_empty() {
                        s.voice = voice;
                    }
                    os::speak(&text, &s).map_err(err)?;
                    ctx.log(format!("speaking {} characters", text.chars().count()));
                    Ok(())
                }),
        )
        .action(
            Action::toggle("stop", "Stop Speaking")
                .icon("square")
                .state("idle", "Not speaking", "volume-x")
                .state("speaking", "Speaking", "square")
                .live(Duration::from_millis(500), |_| Ok(Visual::new().state(if os::speaking() { "speaking" } else { "idle" })))
                .on_press(|ctx| {
                    os::stop().map_err(err)?;
                    ctx.set_state("idle");
                    Ok(())
                }),
        )
        .action(
            Action::toggle("voice", "Use Voice")
                .icon("user-round")
                .state("off", "Voice", "user-round")
                .state("on", "Voice in use", "user-round-check")
                .inputs(vec![input::query_select("voice", "Voice", "voices", &[]).with("help", "Pressing the key makes this the voice every Speak Text key uses (unless it picks its own).")])
                .live(Duration::from_secs(1), |ic| {
                    let on = !ic.input_str("voice").is_empty() && ic.input_str("voice") == settings().voice;
                    Ok(Visual::new().state(if on { "on" } else { "off" }))
                })
                .on_press(|ctx| {
                    let voice = ctx.input_str("voice");
                    if voice.is_empty() {
                        return ctx.fail("Choose a voice");
                    }
                    update(|s| s.voice = voice);
                    ctx.set_state("on");
                    Ok(())
                }),
        )
        .action(
            Action::slider("rate", "Speech Rate", RATE_MIN, RATE_MAX)
                .icon("gauge")
                .unit("%")
                .inputs(vec![step_input("10")])
                .initial_with(|_| Some((settings().rate * 100.0).round().to_string()))
                .live(Duration::from_secs(2), |_| Ok(Visual::new().state((settings().rate * 100.0).round())))
                .on_adjust(|ctx| {
                    let next = next_value(ctx, settings().rate * 100.0, percent_step(ctx, 10.0), RATE_MIN, RATE_MAX);
                    update(|s| s.rate = next / 100.0);
                    ctx.fan_out(next);
                    Ok(())
                }),
        )
        .action(
            Action::slider("volume", "Speech Volume", 0.0, 100.0)
                .icon("volume-2")
                .unit("%")
                .inputs(vec![step_input("5")])
                .initial_with(|_| Some(settings().volume.round().to_string()))
                .live(Duration::from_secs(2), |_| Ok(Visual::new().state(settings().volume.round())))
                .on_adjust(|ctx| {
                    let next = next_value(ctx, settings().volume, percent_step(ctx, 5.0), 0.0, 100.0);
                    update(|s| s.volume = next);
                    ctx.fan_out(next);
                    Ok(())
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_parse() {
        assert_eq!(parse_settings(""), Settings::default());
        let s = parse_settings(r#"{"voice":"Samantha","rate":9,"volume":-3}"#);
        assert_eq!(s, Settings { voice: "Samantha".into(), rate: 3.0, volume: 0.0 });
        assert_eq!(parse_settings(r#"{"rate":1.5}"#).rate, 1.5);
    }
}
