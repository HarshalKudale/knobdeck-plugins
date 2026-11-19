//! Clocks plugin for KnobDeck — a clock for any time zone, a world-clock wheel, a countdown timer
//! (optionally written to a text file for OBS), a Pomodoro timer and a stopwatch. Time zones come
//! from the OS database (macOS) or a bundled one (Windows).

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const LOCAL: &str = "__local__";
const TICK: Duration = Duration::from_secs(1);

fn now_in(zone: &str) -> Result<jiff::Zoned> {
    let now = jiff::Zoned::now();
    if zone.is_empty() || zone == LOCAL {
        return Ok(now);
    }
    now.in_tz(zone).map_err(|_| Error::msg(format!("Unknown time zone \"{zone}\"")))
}

/// "21:05" or "9:05 PM", optionally with "Tue 3 Nov".
fn clock_text(t: &jiff::Zoned, twelve_hour: bool, date: bool) -> String {
    let time = if twelve_hour { t.strftime("%-I:%M %p").to_string() } else { t.strftime("%H:%M").to_string() };
    if date {
        format!("{time} · {}", t.strftime("%a %-d %b"))
    } else {
        time
    }
}

/// The city part of an IANA name: "America/New_York" → "New York".
fn city(zone: &str) -> String {
    zone.rsplit('/').next().unwrap_or(zone).replace('_', " ")
}

fn zone_input() -> Value {
    input::query_combobox("zone", "Time zone", "get-zones").with("default", LOCAL)
}

fn format_inputs() -> Vec<Value> {
    vec![input::select("format", "Format", &[("24", "24-hour"), ("12", "12-hour")]).with("default", "24"), input::checkbox("date", "Show the date")]
}

fn fmt(t: &jiff::Zoned, inputs: &Value) -> String {
    // Checkbox values arrive as a bool, or as "true" from older hosts.
    let date = inputs["date"] == true || inputs["date"] == "true";
    clock_text(t, inputs["format"] == "12", date)
}

/// mm:ss, or h:mm:ss from an hour up.
fn duration_text(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

/// "5m", "90s", "1h30m", "10:00", "1:30:00", or a bare number of minutes.
fn parse_duration(text: &str) -> Option<Duration> {
    let t = text.trim().to_ascii_lowercase();
    if t.is_empty() {
        return None;
    }
    if t.contains(':') {
        let parts: Vec<u64> = t.split(':').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
        let secs = parts.iter().fold(0, |acc, p| acc * 60 + p);
        return (parts.len() <= 3 && secs > 0).then(|| Duration::from_secs(secs));
    }
    if let Ok(mins) = t.parse::<f64>() {
        return (mins > 0.0).then(|| Duration::from_secs_f64(mins * 60.0));
    }
    let (mut secs, mut num) = (0u64, String::new());
    for c in t.chars().filter(|c| !c.is_whitespace()) {
        match c {
            '0'..='9' => num.push(c),
            'h' | 'm' | 's' => {
                let n: u64 = num.parse().ok()?;
                secs += n * match c {
                    'h' => 3600,
                    'm' => 60,
                    _ => 1,
                };
                num.clear();
            }
            _ => return None,
        }
    }
    (num.is_empty() && secs > 0).then(|| Duration::from_secs(secs))
}

/// A running or paused countdown / stopwatch, per placed key.
#[derive(Clone, Copy, Default)]
struct Watch {
    /// Time counted before the current run.
    banked: Duration,
    /// When the current run started (None = paused / not started).
    since: Option<Instant>,
}

impl Watch {
    fn elapsed(&self) -> Duration {
        self.elapsed_at(Instant::now())
    }
    fn elapsed_at(&self, now: Instant) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }
    fn running(&self) -> bool {
        self.since.is_some()
    }
    fn toggle(&mut self) {
        match self.since.take() {
            Some(s) => self.banked += s.elapsed(),
            None => self.since = Some(Instant::now()),
        }
    }
}

static WATCHES: Mutex<Option<HashMap<String, Watch>>> = Mutex::new(None);

fn watch(iid: &str) -> Watch {
    WATCHES.lock().unwrap().as_ref().and_then(|m| m.get(iid)).copied().unwrap_or_default()
}

fn set_watch(iid: &str, w: Watch) {
    WATCHES.lock().unwrap().get_or_insert_with(HashMap::new).insert(iid.into(), w);
}

fn timer_length(inputs: &Value) -> Result<Duration> {
    parse_duration(inputs["length"].as_str().unwrap_or("")).ok_or_else(|| Error::msg("Set a length like 5m, 90s or 1:30:00"))
}

/// Whether a countdown has run out.
fn timer_done(iid: &str, inputs: &Value) -> Result<bool> {
    Ok(watch(iid).elapsed() >= timer_length(inputs)?)
}

/// Countdown key face: remaining time, or "Time's up" when it ran out.
fn timer_visual(iid: &str, inputs: &Value) -> Result<Visual> {
    let total = timer_length(inputs)?;
    let w = watch(iid);
    let left = total.saturating_sub(w.elapsed());
    Ok(if left.is_zero() {
        Visual::new().label("Time's up").icon("bell-ring")
    } else if w.running() {
        // Round up so it shows 00:01 during its last second, not 00:00.
        Visual::new().label(duration_text(left + Duration::from_millis(999))).icon("timer")
    } else if w.elapsed().is_zero() {
        Visual::new().label(duration_text(total)).icon("timer")
    } else {
        Visual::new().label(format!("{} ⏸", duration_text(left + Duration::from_millis(999)))).icon("timer-off")
    })
}

/// The countdown's text for its file: "mm:ss" (minutes past 59 allowed), "hh:mm:ss", or the
/// tile's own style (mm:ss, h:mm:ss from an hour up). At zero, the end text if one is set.
fn file_text(left: Duration, format: &str, end_text: &str) -> String {
    if left.is_zero() && !end_text.is_empty() {
        return end_text.to_string();
    }
    let s = left.as_secs();
    match format {
        "mmss" => format!("{:02}:{:02}", s / 60, s % 60),
        "hhmmss" => format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60),
        _ => duration_text(left),
    }
}

/// Where a countdown writes its text: the path from settings (relative = inside the plugin's data
/// folder), or `countdown.txt` in the data folder.
fn timer_file(inputs: &Value) -> std::path::PathBuf {
    let p = inputs["file_path"].as_str().unwrap_or("").trim();
    let dir = knobdeck_sdk::data_dir();
    if p.is_empty() {
        dir.join("countdown.txt")
    } else {
        let p = std::path::Path::new(p);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            dir.join(p)
        }
    }
}

/// Last text written per file, so a file is rewritten only when its text changes.
static WRITTEN: Mutex<Option<HashMap<std::path::PathBuf, String>>> = Mutex::new(None);

/// Write the countdown's remaining time to its file (when "Write to a text file" is on).
/// Written to a temporary file first and renamed, so OBS never reads a half-written file.
fn write_timer_file(iid: &str, inputs: &Value) -> Result<()> {
    if !(inputs["to_file"] == true || inputs["to_file"] == "true") {
        return Ok(());
    }
    let total = timer_length(inputs)?;
    let w = watch(iid);
    let mut left = total.saturating_sub(w.elapsed());
    if !left.is_zero() && !w.elapsed().is_zero() {
        left += Duration::from_millis(999); // like the tile: 00:01 during the last second
    }
    let text = file_text(left, inputs["file_format"].as_str().unwrap_or(""), inputs["end_text"].as_str().unwrap_or(""));
    let path = timer_file(inputs);
    let mut written = WRITTEN.lock().unwrap();
    let written = written.get_or_insert_with(HashMap::new);
    if written.get(&path) == Some(&text) && path.exists() {
        return Ok(());
    }
    let fail = |e: std::io::Error| Error::msg(format!("Can't write {}: {e}", path.display()));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let tmp = path.with_extension("knobdeck-tmp");
    std::fs::write(&tmp, &text).map_err(fail)?;
    std::fs::rename(&tmp, &path).map_err(fail)?;
    written.insert(path, text);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Phase {
    #[default]
    Work,
    Short,
    Long,
}

/// Pomodoro settings read from the key's inputs.
struct PomoConfig {
    work: Duration,
    short: Duration,
    long: Duration,
    /// A long break after every N work sessions.
    every: u32,
    /// Start the next phase on its own when one ends.
    auto: bool,
}

impl PomoConfig {
    fn from_inputs(inputs: &Value) -> Result<Self> {
        let len = |key: &str, default: u64, what: &str| match inputs[key].as_str().map(str::trim) {
            None | Some("") => Ok(Duration::from_secs(default * 60)),
            Some(t) => parse_duration(t).ok_or_else(|| Error::msg(format!("Set the {what} length like 25m, 90s or 1:30:00"))),
        };
        let every = match &inputs["every"] {
            Value::Number(n) => n.as_u64(),
            Value::String(s) if s.trim().is_empty() => Some(4),
            Value::String(s) => s.trim().parse().ok(),
            _ => Some(4),
        }
        .filter(|n| (1..=99).contains(n))
        .ok_or_else(|| Error::msg("Set \"Long break every\" to a number of work sessions, like 4"))?;
        Ok(PomoConfig {
            work: len("work", 25, "work")?,
            short: len("short", 5, "short break")?,
            long: len("long", 15, "long break")?,
            every: every as u32,
            auto: inputs["auto"] == true || inputs["auto"] == "true",
        })
    }
    fn length(&self, p: Phase) -> Duration {
        match p {
            Phase::Work => self.work,
            Phase::Short => self.short,
            Phase::Long => self.long,
        }
    }
}

/// One Pomodoro key: the current phase, its watch and the work sessions completed.
#[derive(Clone, Copy, Default, Debug)]
struct Pomo {
    phase: Phase,
    watch: Watch,
    /// Work sessions completed (shown on the key).
    done: u32,
    /// The previous phase just ended and this one waits for a press (no auto-start).
    alert: bool,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch").field("banked", &self.banked).field("running", &self.since.is_some()).finish()
    }
}

impl Pomo {
    /// Move to the next phase; `completed` counts a finished work session. After work comes a
    /// long break every `every`-th session (a skipped one still takes its place in the cycle),
    /// else a short one; after a break comes work. `start_next` starts the next phase then.
    fn finish(&mut self, cfg: &PomoConfig, completed: bool, start_next: Option<Instant>) {
        self.phase = if self.phase == Phase::Work {
            if completed {
                self.done += 1;
            }
            let nth = if completed { self.done } else { self.done + 1 };
            if nth % cfg.every == 0 {
                Phase::Long
            } else {
                Phase::Short
            }
        } else {
            Phase::Work
        };
        self.watch = Watch { banked: Duration::ZERO, since: start_next };
        self.alert = start_next.is_none() && completed;
    }
    /// Advance past phases that ran out by `now` (several when auto-start is on and the plugin
    /// was asleep). Returns whether a phase ended.
    fn advance(&mut self, cfg: &PomoConfig, now: Instant) -> bool {
        let mut ended = false;
        for _ in 0..100 {
            let len = cfg.length(self.phase);
            let elapsed = self.watch.elapsed_at(now);
            if !self.watch.running() || elapsed < len {
                break;
            }
            ended = true;
            // With auto-start the next phase starts where this one ended, not at this tick.
            let next_start = cfg.auto.then(|| now.checked_sub(elapsed - len).unwrap_or(now));
            self.finish(cfg, true, next_start);
        }
        ended
    }
    /// Press: start / pause; after a phase ended (no auto-start), start the next one.
    fn press(&mut self, now: Instant) {
        if self.alert {
            self.alert = false;
            self.watch = Watch { banked: Duration::ZERO, since: Some(now) };
            return;
        }
        match self.watch.since.take() {
            Some(s) => self.watch.banked += now.saturating_duration_since(s),
            None => self.watch.since = Some(now),
        }
    }
    /// Skip to the next phase (a skipped work session isn't counted); it waits for a press
    /// unless the current one was running.
    fn skip(&mut self, cfg: &PomoConfig, now: Instant) {
        let running = self.watch.running();
        self.finish(cfg, false, running.then_some(now));
        self.alert = false;
    }
}

static POMOS: Mutex<Option<HashMap<String, Pomo>>> = Mutex::new(None);

fn with_pomo<T>(iid: &str, f: impl FnOnce(&mut Pomo) -> T) -> T {
    let mut all = POMOS.lock().unwrap();
    f(all.get_or_insert_with(HashMap::new).entry(iid.to_string()).or_default())
}

fn phase_name(p: Phase) -> &'static str {
    match p {
        Phase::Work => "Focus",
        Phase::Short => "Break",
        Phase::Long => "Long break",
    }
}

/// Pomodoro key face: phase, time left and completed work sessions ("✓2").
fn pomo_visual(p: &Pomo, cfg: &PomoConfig, now: Instant) -> Visual {
    let count = if p.done > 0 { format!(" ✓{}", p.done) } else { String::new() };
    let len = cfg.length(p.phase);
    let left = len.saturating_sub(p.watch.elapsed_at(now));
    let icon = if p.phase == Phase::Work { "timer" } else { "coffee" };
    if p.alert {
        let what = if p.phase == Phase::Work { "Focus time" } else { "Break time" };
        return Visual::new().label(format!("{what}{count}")).icon("bell-ring");
    }
    if p.watch.running() {
        Visual::new().label(format!("{} {}{count}", phase_name(p.phase), duration_text(left + Duration::from_millis(999)))).icon(icon)
    } else if p.watch.elapsed_at(now).is_zero() {
        Visual::new().label(format!("{} {}{count}", phase_name(p.phase), duration_text(len))).icon(icon)
    } else {
        Visual::new().label(format!("{} {} ⏸{count}", phase_name(p.phase), duration_text(left + Duration::from_millis(999)))).icon("timer-off")
    }
}

fn pomo_tick(iid: &str, inputs: &Value) -> Result<Visual> {
    let cfg = PomoConfig::from_inputs(inputs)?;
    let now = Instant::now();
    Ok(with_pomo(iid, |p| {
        p.advance(&cfg, now);
        pomo_visual(p, &cfg, now)
    }))
}

fn stopwatch_visual(iid: &str) -> Visual {
    let w = watch(iid);
    let text = duration_text(w.elapsed());
    match (w.running(), w.elapsed().is_zero()) {
        (true, _) => Visual::new().label(text).icon("timer"),
        (false, true) => Visual::new().label("00:00").icon("timer-reset"),
        (false, false) => Visual::new().label(format!("{text} ⏸")).icon("timer-off"),
    }
}

fn show(ctx: &Ctx, v: Visual) {
    ctx.set_visual(v);
}

fn main() {
    Plugin::new(&format!("clocks-{}", std::env::consts::OS), "Clocks")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Clocks for any time zone, a world clock wheel, a countdown timer, a Pomodoro timer and a stopwatch.")
        .icon("clock")
        .query("get-zones", |_| {
            let mut zones: Vec<String> = jiff::tz::db().available().map(|n| n.to_string()).filter(|n| n.contains('/') && !n.starts_with("Etc/")).collect();
            zones.sort();
            let mut opts = vec![json!({"value": LOCAL, "label": "This computer's time zone"})];
            opts.extend(zones.into_iter().map(|z| json!({"value": z, "label": z.replace('_', " ")})));
            Ok(Value::Array(opts))
        })
        .action(
            Action::button("clock", "Clock")
                .icon("clock")
                .inputs([vec![zone_input(), input::text("name", "Name on the key (optional)").placeholder("Tokyo")], format_inputs()].concat())
                // Pushed only when the text changes, so once a minute.
                .live(TICK, |ic| {
                    let zone = ic.input_str("zone");
                    let t = match now_in(&zone) {
                        Ok(t) => t,
                        Err(e) => return Ok(Visual::new().label(e.message().to_string())),
                    };
                    let name = ic.input_str("name");
                    let name = if !name.is_empty() { name } else if zone.is_empty() || zone == LOCAL { String::new() } else { city(&zone) };
                    let text = fmt(&t, &ic.inputs);
                    Ok(Visual::new().label(if name.is_empty() { text } else { format!("{name} {text}") }))
                }),
        )
        .action(
            Action::wheel("world-clock-wheel", "World Clock")
                .icon("globe")
                .inputs([vec![input::textarea("zones", "Time zones, one per line").placeholder("America/New_York\nEurope/London\nAsia/Tokyo")], format_inputs()].concat())
                .on_open(|ctx| {
                    let zones: Vec<String> = ctx.input_str("zones").lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
                    if zones.is_empty() {
                        return ctx.fail("Add some time zones, one per line (e.g. Asia/Tokyo)");
                    }
                    let local_day = jiff::Zoned::now().date();
                    let mut slices = vec![];
                    for (i, z) in zones.iter().take(16).enumerate() {
                        let t = now_in(z).or_else(|e| ctx.fail(e.message().to_string()))?;
                        // Mark another day relative to here: "+1" tomorrow, "-1" yesterday.
                        let day = match t.date().cmp(&local_day) {
                            std::cmp::Ordering::Greater => " +1",
                            std::cmp::Ordering::Less => " -1",
                            _ => "",
                        };
                        let night = !(6..18).contains(&t.hour());
                        slices.push(json!({"id": format!("z{i}"), "label": format!("{} {}{day}", city(z), fmt(&t, &ctx.inputs)), "icon": if night { "moon" } else { "sun" }}));
                    }
                    Ok(slices)
                })
                .on_child_press(|_| Ok(())),
        )
        .action(
            Action::button("timer", "Countdown Timer")
                .icon("timer")
                .inputs(vec![
                    input::text("length", "Length").placeholder("5m, 90s, 1h30m or 10:00").default_value("5m"),
                    input::checkbox("to_file", "Write the time left to a text file (for an OBS text source)"),
                    input::text("file_path", "Text file").placeholder("countdown.txt in the plugin's data folder").with("showIf", json!({"key": "to_file", "truthy": true})),
                    input::select("file_format", "Format in the file", &[("", "mm:ss, h:mm:ss from an hour up"), ("mmss", "mm:ss (minutes past 59)"), ("hhmmss", "hh:mm:ss")])
                        .with("default", "")
                        .with("showIf", json!({"key": "to_file", "truthy": true})),
                    input::text("end_text", "Text at zero (optional)").placeholder("00:00").with("showIf", json!({"key": "to_file", "truthy": true})),
                ])
                .live(TICK, |ic| {
                    if let Err(e) = write_timer_file(&ic.instance_id, &ic.inputs) {
                        return Ok(Visual::new().label(e.message().to_string()));
                    }
                    timer_visual(&ic.instance_id, &ic.inputs).or_else(|e| Ok(Visual::new().label(e.message().to_string())))
                })
                // Press: start / pause; once it has run out, press resets it.
                .on_press(|ctx| {
                    timer_length(&ctx.inputs)?;
                    let mut w = watch(&ctx.instance_id);
                    if timer_done(&ctx.instance_id, &ctx.inputs)? {
                        w = Watch::default();
                    } else {
                        w.toggle();
                    }
                    set_watch(&ctx.instance_id, w);
                    write_timer_file(&ctx.instance_id, &ctx.inputs)?;
                    show(ctx, timer_visual(&ctx.instance_id, &ctx.inputs)?);
                    Ok(())
                }),
        )
        .action(
            Action::button("pomodoro", "Pomodoro")
                .icon("timer")
                .inputs(vec![
                    input::text("work", "Work").placeholder("25m").default_value("25m"),
                    input::text("short", "Short break").placeholder("5m").default_value("5m"),
                    input::text("long", "Long break").placeholder("15m").default_value("15m"),
                    input::text("every", "Long break every (work sessions)").placeholder("4").default_value("4"),
                    input::checkbox("auto", "Start the next phase on its own"),
                    input::text("name", "Name (optional; links Pomodoro Skip / Reset keys)").placeholder("Pomodoro"),
                ])
                .live(TICK, |ic| pomo_tick(&ic.instance_id, &ic.inputs).or_else(|e| Ok(Visual::new().label(e.message().to_string()))))
                // Press: start / pause; when a phase ended, start the next one.
                .on_press(|ctx| {
                    let cfg = PomoConfig::from_inputs(&ctx.inputs).or_else(|e| ctx.fail(e.message().to_string()))?;
                    let now = Instant::now();
                    let v = with_pomo(&ctx.instance_id, |p| {
                        p.advance(&cfg, now);
                        p.press(now);
                        pomo_visual(p, &cfg, now)
                    });
                    show(ctx, v);
                    Ok(())
                }),
        )
        .action(
            Action::button("pomodoro-control", "Pomodoro Skip / Reset")
                .icon("skip-forward")
                .inputs(vec![
                    input::select("op", "Do", &[("skip", "Skip to the next phase"), ("reset", "Reset (back to work, count to 0)")]).with("default", "skip"),
                    input::text("name", "Pomodoro name (as on its key; empty = unnamed ones)").placeholder("Pomodoro"),
                ])
                .on_press(|ctx| {
                    let filter = json!({"name": ctx.input_str("name")});
                    let ids = ctx.runtime().instances_matching("pomodoro", filter.as_object().unwrap());
                    if ids.is_empty() {
                        return ctx.fail("No Pomodoro key with this name is placed");
                    }
                    let reset = ctx.input_str("op") == "reset";
                    let now = Instant::now();
                    for iid in ids {
                        let inputs = ctx.runtime().inputs_of(&iid);
                        let Ok(cfg) = PomoConfig::from_inputs(&inputs) else { continue };
                        let v = with_pomo(&iid, |p| {
                            p.advance(&cfg, now);
                            if reset {
                                *p = Pomo::default();
                            } else {
                                p.skip(&cfg, now);
                            }
                            pomo_visual(p, &cfg, now)
                        });
                        ctx.runtime().set_visual(&iid, v);
                    }
                    Ok(())
                }),
        )
        .action(
            Action::button("stopwatch", "Stopwatch")
                .icon("timer-reset")
                .live(TICK, |ic| Ok(stopwatch_visual(&ic.instance_id)))
                // Press: start → stop → reset.
                .on_press(|ctx| {
                    let mut w = watch(&ctx.instance_id);
                    if !w.running() && !w.elapsed().is_zero() {
                        w = Watch::default();
                    } else {
                        w.toggle();
                    }
                    set_watch(&ctx.instance_id, w);
                    show(ctx, stopwatch_visual(&ctx.instance_id));
                    Ok(())
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("90s"), Some(Duration::from_secs(90)));
        assert_eq!(parse_duration("1h30m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("1h 30m"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("10:00"), Some(Duration::from_secs(600)));
        assert_eq!(parse_duration("1:30:00"), Some(Duration::from_secs(5400)));
        assert_eq!(parse_duration("2.5"), Some(Duration::from_secs(150)));
        for bad in ["", "0", "abc", "5x", "1:2:3:4", "5m3", "0:00"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
        assert_eq!(duration_text(Duration::from_secs(65)), "01:05");
        assert_eq!(duration_text(Duration::from_secs(3725)), "1:02:05");
    }

    #[test]
    fn zones_and_formats() {
        let t = jiff::civil::date(2026, 11, 3).at(21, 5, 0, 0).in_tz("Asia/Kolkata").unwrap();
        assert_eq!(clock_text(&t, false, false), "21:05");
        assert_eq!(clock_text(&t, true, true), "9:05 PM · Tue 3 Nov");
        assert_eq!(city("America/New_York"), "New York");
        assert!(now_in("Not/AZone").is_err());
        // DST: New York is UTC-4 in July and UTC-5 in January.
        let ny = |m| jiff::civil::date(2026, m, 1).at(12, 0, 0, 0).in_tz("America/New_York").unwrap().offset().seconds() / 3600;
        assert_eq!((ny(7), ny(1)), (-4, -5));
    }

    #[test]
    fn watch_pauses_and_resumes() {
        let mut w = Watch::default();
        assert!(!w.running() && w.elapsed().is_zero());
        w.toggle();
        std::thread::sleep(Duration::from_millis(30));
        w.toggle();
        let paused = w.elapsed();
        assert!(paused >= Duration::from_millis(30) && !w.running());
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(w.elapsed(), paused, "a paused watch does not count");
    }

    fn cfg(auto: bool) -> PomoConfig {
        PomoConfig::from_inputs(&json!({"work": "10s", "short": "2s", "long": "5s", "every": "2", "auto": auto})).unwrap()
    }

    #[test]
    fn pomodoro_config() {
        let c = PomoConfig::from_inputs(&json!({})).unwrap();
        assert_eq!((c.work, c.short, c.long, c.every, c.auto), (Duration::from_secs(1500), Duration::from_secs(300), Duration::from_secs(900), 4, false));
        assert!(PomoConfig::from_inputs(&json!({"every": "0"})).is_err());
        assert!(PomoConfig::from_inputs(&json!({"work": "abc"})).is_err());
        assert_eq!(PomoConfig::from_inputs(&json!({"every": 3})).unwrap().every, 3);
    }

    #[test]
    fn pomodoro_phases_wait_for_a_press() {
        let c = cfg(false);
        let t0 = Instant::now();
        let s = |n| t0 + Duration::from_secs(n);
        let mut p = Pomo::default();
        p.press(t0);
        assert!(!p.advance(&c, s(9)));
        assert!(p.advance(&c, s(11)));
        assert_eq!((p.phase, p.done, p.alert, p.watch.running()), (Phase::Short, 1, true, false));
        assert_eq!(pomo_visual(&p, &c, s(11)).label.as_deref(), Some("Break time ✓1"));
        p.press(s(12)); // start the break
        assert!(p.advance(&c, s(14)));
        assert_eq!((p.phase, p.alert), (Phase::Work, true));
        p.press(s(15));
        assert!(p.advance(&c, s(25)));
        // Second work session: a long break (every 2).
        assert_eq!((p.phase, p.done), (Phase::Long, 2));
    }

    #[test]
    fn pomodoro_auto_start_pause_skip() {
        let c = cfg(true);
        let t0 = Instant::now();
        let s = |n| t0 + Duration::from_secs(n);
        let mut p = Pomo::default();
        p.press(t0);
        // Work ends at 10, the short break at 12: at 13 it is 1 s into the second work session.
        p.advance(&c, s(13));
        assert_eq!((p.phase, p.done, p.alert), (Phase::Work, 1, false));
        assert_eq!(p.watch.elapsed_at(s(13)), Duration::from_secs(1));
        p.press(s(13)); // pause
        assert!(!p.advance(&c, s(100)), "a paused phase never ends");
        assert!(pomo_visual(&p, &c, s(100)).label.unwrap().contains('⏸'));
        // Skip a paused work session: not counted, waits paused at the break's full length; it is
        // the 2nd in the cycle, so a long break.
        p.skip(&c, s(100));
        assert_eq!((p.phase, p.done, p.alert, p.watch.running()), (Phase::Long, 1, false, false));
        assert_eq!(pomo_visual(&p, &c, s(100)).label.as_deref(), Some("Long break 00:05 ✓1"));
    }

    #[test]
    fn countdown_file_text() {
        let d = Duration::from_secs;
        assert_eq!(file_text(d(5400), "", ""), "1:30:00");
        assert_eq!(file_text(d(5400), "mmss", ""), "90:00");
        assert_eq!(file_text(d(65), "hhmmss", ""), "00:01:05");
        assert_eq!(file_text(d(0), "mmss", ""), "00:00");
        assert_eq!(file_text(d(0), "mmss", "Starting!"), "Starting!");
    }
}
