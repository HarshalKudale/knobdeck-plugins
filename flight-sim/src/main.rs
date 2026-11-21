//! Flight Simulator plugin for KnobDeck: Microsoft Flight Simulator 2020 and 2024 through
//! SimConnect, the sim's own client API (`SimConnect.dll`, loaded at run time from where the
//! MSFS SDK or the sim installed it). Autopilot modes and bugs, radios, transponder, gear, flaps,
//! spoilers, lights, electrical, trim, altimeter, pause and readouts, each with live state from
//! the sim's SimVars. Never sends keystrokes. Windows only, like the sim; it builds elsewhere
//! with a stub connector so it can be driven over its protocol in tests.

mod catalog;
mod engine;
#[cfg(not(windows))]
mod fake_sim;
// Only the Windows connector builds every message and error; elsewhere the stub never connects.
#[cfg_attr(not(windows), allow(dead_code))]
mod sim;
#[cfg(windows)]
mod simconnect;
mod values;

use catalog::{pick, Bug, Radio, Readout, Show, Switch};
use engine::{Engine, VarKey};
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{Action, Ctx, Error, InitialCtx, Plugin, Result, Visual};
use sim::{Connector, Rate};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use values::Spacing;

const POLL: Duration = Duration::from_millis(500);
const TICK: Duration = Duration::from_millis(30);
/// How long a press waits for the sim to show its effect.
const SETTLE: Duration = Duration::from_millis(700);
/// How long a press waits for a SimVar's first value after connecting.
const FIRST_VALUE: Duration = Duration::from_millis(1500);

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();

#[cfg(windows)]
fn connector() -> Box<dyn Connector> {
    Box::new(simconnect::Dll::default())
}

/// Elsewhere the sim never runs; `KNOBDECK_FAKE_SIM=1` swaps in a pretend one for driving the
/// plugin over its protocol in tests.
#[cfg(not(windows))]
fn connector() -> Box<dyn Connector> {
    if std::env::var_os("KNOBDECK_FAKE_SIM").is_some() {
        Box::new(fake_sim::FakeSim)
    } else {
        Box::new(sim::Offline)
    }
}

fn engine() -> MutexGuard<'static, Engine> {
    ENGINE.get_or_init(|| Mutex::new(Engine::new(connector()))).lock().unwrap_or_else(|e| e.into_inner())
}

fn key(var: &str, unit: &str) -> VarKey {
    VarKey { name: var.to_string(), unit: unit.to_string() }
}

/// Send a sim event (K-event) with up to five values.
fn send(event: &str, data: &[i64]) -> Result<()> {
    let mut d = [0u32; 5];
    for (slot, v) in d.iter_mut().zip(data) {
        *slot = values::dword(*v);
    }
    engine().send(Instant::now(), event, d).map_err(Error::msg)
}

/// A SimVar's latest value; `None` until the sim has sent one. An error while disconnected.
fn read_key(k: &VarKey, rate: Rate) -> std::result::Result<Option<f64>, String> {
    let mut e = engine();
    e.pump();
    e.value(Instant::now(), k, rate)
}

fn read(var: &str, unit: &str) -> std::result::Result<Option<f64>, String> {
    read_key(&key(var, unit), Rate::Frame)
}

/// Connect if needed and wait for a SimVar's value (a press that starts from the current value).
fn need(var: &str, unit: &str) -> Result<f64> {
    engine().ensure(Instant::now()).map_err(Error::msg)?;
    let deadline = Instant::now() + FIRST_VALUE;
    loop {
        if let Some(v) = read(var, unit).map_err(Error::msg)? {
            return Ok(v);
        }
        if Instant::now() >= deadline {
            return Err(Error::msg(format!("Flight Simulator sent no {var} yet. Is a flight loaded?")));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// The SimVar's value once it moves off `before`, or what it is after [`SETTLE`].
fn settle(var: &str, unit: &str, before: f64) -> f64 {
    let deadline = Instant::now() + SETTLE;
    loop {
        let now = read(var, unit).ok().flatten();
        if let Some(v) = now {
            if (v - before).abs() > 1e-6 || Instant::now() >= deadline {
                return v;
            }
        } else if Instant::now() >= deadline {
            return before;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// A live value as a tile's visual: `—` while the sim is closed, `?` when the sim rejected the
/// SimVar (a press says why), nothing new while the first value is on its way.
fn shown(short: &str, r: std::result::Result<Option<f64>, String>, f: impl FnOnce(f64) -> Visual) -> Visual {
    match r {
        Ok(Some(v)) => f(v),
        Ok(None) => Visual::new(),
        Err(_) if !engine().connected() => Visual::new().label(format!("{short} —")),
        Err(_) => Visual::new().label(format!("{short} ?")),
    }
}

fn fan_visual(ctx: &Ctx, keys: &[&str], v: Visual) {
    if keys.is_empty() {
        ctx.fan_out_visual(v)
    } else {
        ctx.fan_out_visual_by(keys, v)
    }
}

fn fan(ctx: &Ctx, keys: &[&str], state: &str) {
    if keys.is_empty() {
        ctx.fan_out(state)
    } else {
        ctx.fan_out_by(keys, state)
    }
}

fn on_off(v: f64) -> &'static str {
    if v >= 0.5 {
        "on"
    } else {
        "off"
    }
}

const BOOL: &str = "bool";

/// Toggle a switch and report its real state once the sim shows it.
fn press_switch(ctx: &Ctx, s: &Switch, keys: &[&str]) -> Result {
    let before = need(s.var, BOOL)?;
    send(s.event, &[])?;
    fan(ctx, keys, on_off(settle(s.var, BOOL, before)));
    Ok(())
}

fn live_switch(s: &Switch) -> Result<Visual> {
    Ok(match read(s.var, BOOL).map_err(Error::msg)? {
        Some(v) => Visual::new().state(on_off(v)),
        None => Visual::new(),
    })
}

/// A toggle over one switch of `list`, picked by the `which` input.
fn switch_toggle(id: &str, label: &str, icon: &str, list: &'static [Switch], names: Vec<(&'static str, &'static str)>, which: &str, states: [(&str, &str); 2]) -> Action {
    Action::toggle(id, label)
        .icon(icon)
        .inputs(vec![input::select("which", which, &names).with("default", list[0].key)])
        .state("off", states[0].0, states[0].1)
        .state("on", states[1].0, states[1].1)
        .live(POLL, move |ic: &InitialCtx| live_switch(pick(list, &ic.input_str("which"))))
        .on_press(move |ctx| press_switch(ctx, pick(list, &ctx.input_str("which")), &["which"]))
}

/// A toggle over one fixed switch.
fn single_toggle(id: &str, label: &str, s: &'static Switch, off: (&str, &str), on: (&str, &str)) -> Action {
    Action::toggle(id, label)
        .icon(on.1)
        .state("off", off.0, off.1)
        .state("on", on.0, on.1)
        .live(POLL, move |_| live_switch(s))
        .on_press(move |ctx| press_switch(ctx, s, &[]))
}

fn bug_visual(b: &Bug, v: f64) -> Visual {
    if b.degrees {
        let d = values::wrap_deg(v);
        Visual::new().state(d).label(format!("{} {}", b.short, values::fmt_deg(d)))
    } else {
        let v = v.round();
        Visual::new().state(v).label(format!("{} {}{}", b.short, v, b.suffix))
    }
}

/// Set an autopilot bug / course from the dial: its absolute value, or a nudge by steps.
fn adjust_bug(ctx: &Ctx, b: &Bug, keys: &[&str]) -> Result {
    let cur = need(b.var, b.unit)?;
    let Some(t) = values::target(cur, ctx.value, ctx.delta, b.step) else { return Ok(()) };
    let data = if b.degrees { values::wrap_deg(t) as i64 } else { ((t.clamp(b.min, b.max) / b.step).round() * b.step) as i64 };
    // Event index [1] = 0: the aircraft's main (first) slot.
    send(b.event, &[data, 0])?;
    fan_visual(ctx, keys, bug_visual(b, settle(b.var, b.unit, cur)));
    Ok(())
}

fn bug_slider(id: &str, label: &str, icon: &str, b: &'static Bug) -> Action {
    Action::slider(id, label, b.min, b.max)
        .icon(icon)
        .step(b.step)
        .initial(b.min.max(0.0))
        .unit(if b.degrees { "°" } else { b.suffix.trim() })
        .live(POLL, move |_| Ok(shown(b.short, read(b.var, b.unit), |v| bug_visual(b, v))))
        .on_adjust(move |ctx| adjust_bug(ctx, b, &[]))
}

/// Which part of a frequency a dial turns.
#[derive(Clone, Copy)]
enum Part {
    Mhz,
    Khz,
}

fn freq_visual(r: &Radio, khz: u32, spacing: Spacing, part: Part) -> Visual {
    let state = match part {
        Part::Mhz => khz / 1000,
        Part::Khz => khz % 1000,
    };
    Visual::new().state(state).label(format!("{} {}", r.short, values::fmt_freq(khz, spacing)))
}

fn radio_input(list: &'static [Radio]) -> serde_json::Value {
    let names: Vec<(&str, &str)> = list.iter().map(|r| (r.key, r.short)).collect();
    input::select("radio", "Radio", &names).with("default", list[0].key)
}

/// A standby frequency dial (its MHz or its kHz part) for a COM or NAV radio.
fn freq_slider(id: &str, label: &str, list: &'static [Radio], part: Part) -> Action {
    let nav = list[0].nav;
    let (min, max, step) = match (nav, part) {
        (_, Part::Mhz) => {
            let (lo, hi) = list[0].spacing(false).mhz_range();
            (lo as f64, hi as f64, 1.0)
        }
        (false, Part::Khz) => (0.0, 990.0, 5.0),
        (true, Part::Khz) => (0.0, 950.0, 50.0),
    };
    let mut inputs = vec![radio_input(list)];
    if !nav {
        inputs.push(input::select("spacing", "Channel spacing", &[("25", "25 kHz"), ("8.33", "8.33 kHz")]).with("default", "25"));
    }
    let spacing = move |r: &Radio, i: &str| r.spacing(i == "8.33");
    Action::slider(id, label, min, max)
        .icon(if nav { "radio-tower" } else { "radio" })
        .step(step)
        .unit(match part {
            Part::Mhz => "MHz",
            Part::Khz => "kHz",
        })
        .inputs(inputs)
        .live(POLL, move |ic| {
            let r = pick(list, &ic.input_str("radio"));
            let s = spacing(r, &ic.input_str("spacing"));
            Ok(shown(r.short, read(r.standby, catalog::FREQ_UNIT), |v| freq_visual(r, values::khz_of_mhz(v), s, part)))
        })
        .on_adjust(move |ctx| {
            let r = pick(list, &ctx.input_str("radio"));
            let s = spacing(r, &ctx.input_str("spacing"));
            let mhz = need(r.standby, catalog::FREQ_UNIT)?;
            let cur = values::khz_of_mhz(mhz);
            let next = match (part, ctx.value, ctx.delta.map(values::steps)) {
                (Part::Mhz, Some(v), _) => values::with_mhz(cur, v, s),
                (Part::Khz, Some(v), _) => values::with_khz(cur, v, s),
                (Part::Mhz, None, Some(n)) if n != 0 => values::step_mhz(cur, n, s),
                (Part::Khz, None, Some(n)) if n != 0 => values::step_khz(cur, n, s),
                _ => return Ok(()),
            };
            send(r.set_hz, &[values::hz(next) as i64])?;
            let now = values::khz_of_mhz(settle(r.standby, catalog::FREQ_UNIT, mhz));
            ctx.fan_out_visual_by(&["radio"], freq_visual(r, now, s, part));
            Ok(())
        })
}

/// The transponder code from the sim's BCD16 reading (or a plain number, should a sim send one).
fn xpdr_code(raw: f64) -> Option<u32> {
    values::from_bcd16(raw).or_else(|| values::parse_code(&format!("{:04}", raw.round() as i64)))
}

fn read_code() -> std::result::Result<Option<u32>, String> {
    Ok(read(catalog::XPDR_VAR, catalog::XPDR_UNIT)?.and_then(xpdr_code))
}

fn need_code() -> Result<u32> {
    let raw = need(catalog::XPDR_VAR, catalog::XPDR_UNIT)?;
    xpdr_code(raw).ok_or_else(|| Error::msg(format!("Flight Simulator sent a transponder code KnobDeck can't read ({raw})")))
}

fn set_code(code: u32) -> Result<u32> {
    let before = need(catalog::XPDR_VAR, catalog::XPDR_UNIT)?;
    send("XPNDR_SET", &[values::bcd16(code) as i64])?;
    Ok(xpdr_code(settle(catalog::XPDR_VAR, catalog::XPDR_UNIT, before)).unwrap_or(code))
}

fn place(raw: &str) -> u32 {
    raw.trim().parse::<u32>().unwrap_or(0).min(3)
}

fn digit_visual(code: u32, place: u32) -> Visual {
    Visual::new().state(values::code_digit(code, place)).label(format!("XPDR {code:04}"))
}

/// Flaps as "Flaps 2/4" with the dial at that notch's share of full travel.
fn flaps_visual(index: f64, positions: f64) -> Visual {
    let (i, n) = (index.round().max(0.0), positions.round().max(0.0));
    let pct = if n > 0.0 { (i / n * 100.0).round() } else { 0.0 };
    Visual::new().state(pct).label(if n > 0.0 { format!("Flaps {i}/{n}") } else { "No flaps".into() })
}

const FLAPS_INDEX: &str = "FLAPS HANDLE INDEX";
const FLAPS_COUNT: &str = "FLAPS NUM HANDLE POSITIONS";
const NUMBER: &str = "number";
const PERCENT: &str = "percent";

fn live_flaps() -> Visual {
    let n = read(FLAPS_COUNT, NUMBER);
    shown("Flaps", read(FLAPS_INDEX, NUMBER), |i| match n {
        Ok(Some(n)) => flaps_visual(i, n),
        _ => Visual::new(),
    })
}

/// Report the flaps on every flaps tile once the handle has moved.
fn flaps_after(ctx: &Ctx, before: f64, positions: f64) {
    let now = flaps_visual(settle(FLAPS_INDEX, NUMBER, before), positions);
    ctx.runtime().notify_visual("flaps-step", label_only(now.clone()));
    ctx.runtime().notify_visual("flaps", now);
}

/// A visual's label alone (for a key that shows what a dial shows).
fn label_only(v: Visual) -> Visual {
    Visual { label: v.label, ..Visual::new() }
}

fn baro_label(hpa: f64, unit: &str) -> String {
    if unit == "hpa" {
        format!("{} hPa", hpa.round())
    } else {
        format!("{:.2} inHg", values::inhg(hpa))
    }
}

fn baro_visual(hpa: f64, unit: &str) -> Visual {
    let state = if unit == "hpa" { hpa.round() } else { values::round_to(values::inhg(hpa), 2) };
    Visual::new().state(state).label(baro_label(hpa, unit))
}

/// An altimeter dial in inHg or hPa.
fn baro_slider(id: &str, label: &str, unit: &'static str) -> Action {
    let (min, max, step) = if unit == "hpa" { (948.0, 1050.0, 1.0) } else { (28.0, 31.0, 0.01) };
    Action::slider(id, label, min, max)
        .icon("gauge")
        .step(step)
        .unit(if unit == "hpa" { "hPa" } else { "inHg" })
        .live(POLL, move |_| Ok(shown("BARO", read(catalog::BARO_VAR, catalog::BARO_UNIT), |v| baro_visual(v, unit))))
        .on_adjust(move |ctx| {
            let hpa = need(catalog::BARO_VAR, catalog::BARO_UNIT)?;
            let cur = if unit == "hpa" { hpa.round() } else { values::round_to(values::inhg(hpa), 2) };
            let Some(t) = values::target(cur, ctx.value, ctx.delta, step) else { return Ok(()) };
            let t = t.clamp(min, max);
            let to = if unit == "hpa" { t.round() } else { values::hpa(values::round_to(t, 2)) };
            send("KOHLSMAN_SET", &[values::kohlsman(to) as i64, 0])?;
            ctx.fan_out_visual(baro_visual(settle(catalog::BARO_VAR, catalog::BARO_UNIT, hpa), unit));
            Ok(())
        })
}

fn readout_value(r: &Readout) -> std::result::Result<Option<f64>, String> {
    match r.key {
        "fuel" => {
            let q = read_key(&key(r.var, r.unit), Rate::Second)?;
            let c = read_key(&key(catalog::FUEL_CAPACITY, r.unit), Rate::Second)?;
            Ok(match (q, c) {
                (Some(q), Some(c)) if c > 0.0 => Some(q / c * 100.0),
                (Some(_), Some(_)) => Some(0.0),
                _ => None,
            })
        }
        _ => read(r.var, r.unit),
    }
}

fn readout_text(r: &Readout, v: f64) -> String {
    let body = match r.show {
        Show::Whole(s) => format!("{}{s}", v.round()),
        Show::Signed(s) => format!("{:+}{s}", v.round() as i64),
        Show::Heading => values::fmt_deg(values::wrap_deg(v)),
        Show::Rate => values::fmt_rate(v),
    };
    if r.key == "wind" {
        let speed = read(catalog::WIND_SPEED, "knots").ok().flatten().unwrap_or(0.0);
        return format!("{} {body} {} kt", r.short, speed.round());
    }
    format!("{} {body}", r.short)
}

/// "7000, 0x1200, -500" → up to five event values.
fn parse_values(raw: &str) -> Result<Vec<i64>> {
    let parts: Vec<&str> = raw.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if parts.len() > 5 {
        return Err(Error::msg("An event takes at most five values"));
    }
    parts
        .iter()
        .map(|p| {
            let (neg, digits) = match p.strip_prefix('-') {
                Some(d) => (true, d),
                None => (false, *p),
            };
            let v = match digits.strip_prefix("0x").or_else(|| digits.strip_prefix("0X")) {
                Some(hex) => i64::from_str_radix(hex, 16).ok(),
                None => digits.parse::<i64>().ok(),
            };
            match v {
                Some(v) if v <= u32::MAX as i64 => Ok(if neg { -v } else { v }),
                _ => Err(Error::msg(format!("\"{p}\" isn't a whole number (like 7000, -500 or 0x1200)"))),
            }
        })
        .collect()
}

fn custom_var(ic_var: &str, ic_unit: &str) -> std::result::Result<VarKey, String> {
    engine::var_key(ic_var, if ic_unit.trim().is_empty() { "number" } else { ic_unit })
}

fn custom_text(prefix: &str, v: f64, decimals: &str) -> String {
    let d = decimals.trim().parse::<usize>().unwrap_or(0).min(6);
    format!("{} {v:.d$}", prefix.trim()).trim().to_string()
}

/// Keep the connection up while tiles are placed, and read what the sim sends.
fn background(rt: knobdeck_sdk::Runtime) {
    std::thread::spawn(move || loop {
        {
            let mut e = engine();
            let was = e.connected();
            e.tick(Instant::now(), rt.has_instances());
            if e.connected() != was {
                rt.log(if e.connected() { format!("connected to Flight Simulator ({})", e.app().unwrap_or("SimConnect")) } else { "Flight Simulator disconnected".into() });
            }
        }
        std::thread::sleep(TICK);
    });
}

fn main() {
    knobdeck_sdk::load_env();
    Plugin::new("flight-sim-windows", "Flight Simulator")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&["windows"])
        .author("KnobDeck")
        .description("Microsoft Flight Simulator 2020 and 2024 over SimConnect: autopilot, heading / altitude / VS / speed / course dials, radios, transponder, gear, flaps, lights, trim, altimeter and readouts, with live state.")
        .icon("app-icon.svg")
        .on_ready(|rt| {
            background(rt.clone());
            Ok(())
        })
        // Autopilot
        .action(single_toggle("ap-master", "Autopilot Master", &catalog::AP_MASTER, ("AP Off", "plane"), ("AP On", "plane")))
        .action(switch_toggle("ap-mode", "Autopilot Mode", "navigation", catalog::AP_MODES, catalog::AP_MODE_NAMES.to_vec(), "Mode", [("Off", "circle"), ("On", "circle-dot")]))
        .action(bug_slider("heading-bug", "Heading Bug", "compass", &catalog::HEADING))
        .action(bug_slider("altitude", "Autopilot Altitude", "mountain", &catalog::ALTITUDE))
        .action(bug_slider("vertical-speed", "Vertical Speed", "move-vertical", &catalog::VERTICAL_SPEED))
        .action(bug_slider("speed-bug", "Speed Bug", "gauge", &catalog::SPEED))
        .action(
            Action::slider("course", "Course (OBS)", 0.0, 360.0)
                .icon("locate-fixed")
                .step(1.0)
                .inputs(vec![input::select("course", "Course", &[("nav1", "NAV1 / VOR1"), ("nav2", "NAV2 / VOR2"), ("gps", "GPS OBS")]).with("default", "nav1")])
                .live(POLL, |ic| {
                    let b = pick(catalog::COURSES, &ic.input_str("course"));
                    Ok(shown(b.short, read(b.var, b.unit), |v| bug_visual(b, v)))
                })
                .on_adjust(|ctx| adjust_bug(ctx, pick(catalog::COURSES, &ctx.input_str("course")), &["course"])),
        )
        // Radios
        .action(freq_slider("com-mhz", "COM Standby MHz", catalog::COMS, Part::Mhz))
        .action(freq_slider("com-khz", "COM Standby kHz", catalog::COMS, Part::Khz))
        .action(freq_slider("nav-mhz", "NAV Standby MHz", catalog::NAVS, Part::Mhz))
        .action(freq_slider("nav-khz", "NAV Standby kHz", catalog::NAVS, Part::Khz))
        .action(
            Action::button("radio-swap", "Swap Radio")
                .icon("arrow-left-right")
                .inputs(vec![input::select("radio", "Radio", &[("com1", "COM1"), ("com2", "COM2"), ("nav1", "NAV1"), ("nav2", "NAV2")]).with("default", "com1")])
                .live(POLL, |ic| {
                    let radios = catalog::radios();
                    let r = radios.iter().find(|r| r.key == ic.input_str("radio").trim()).copied().unwrap_or(radios[0]);
                    let s = r.spacing(false);
                    Ok(shown(r.short, read(r.active, catalog::FREQ_UNIT), |v| Visual::new().label(format!("{} {}", r.short, values::fmt_freq(values::khz_of_mhz(v), s)))))
                })
                .on_press(|ctx| {
                    let radios = catalog::radios();
                    let r = radios.iter().find(|r| r.key == ctx.input_str("radio").trim()).copied().unwrap_or(radios[0]);
                    let before = need(r.active, catalog::FREQ_UNIT)?;
                    send(r.swap, &[])?;
                    let now = settle(r.active, catalog::FREQ_UNIT, before);
                    ctx.fan_out_visual_by(&["radio"], Visual::new().label(format!("{} {}", r.short, values::fmt_freq(values::khz_of_mhz(now), r.spacing(false)))));
                    Ok(())
                }),
        )
        .action(
            Action::button("transponder", "Set Transponder")
                .icon("radio")
                .inputs(vec![input::text("code", "Code").placeholder("7000").default_value("1200")])
                .live(POLL, |_| Ok(match read_code() {
                    Ok(Some(c)) => Visual::new().label(format!("XPDR {c:04}")),
                    Ok(None) => Visual::new(),
                    Err(_) => Visual::new().label("XPDR —"),
                }))
                .on_press(|ctx| {
                    let code = values::parse_code(&ctx.input_str("code")).ok_or_else(|| Error::msg("Enter a 4-digit transponder code, each digit 0–7 (like 7000)"))?;
                    let now = set_code(code)?;
                    ctx.fan_out_visual(Visual::new().label(format!("XPDR {now:04}")));
                    ctx.runtime().notify_visual("transponder-digit", Visual::new().label(format!("XPDR {now:04}")));
                    Ok(())
                }),
        )
        .action(
            Action::slider("transponder-digit", "Transponder Digit", 0.0, 7.0)
                .icon("binary")
                .step(1.0)
                .inputs(vec![input::select("place", "Digit", &[("0", "1st (thousands)"), ("1", "2nd (hundreds)"), ("2", "3rd (tens)"), ("3", "4th (ones)")]).with("default", "0")])
                .live(POLL, |ic| {
                    let p = place(&ic.input_str("place"));
                    Ok(match read_code() {
                        Ok(Some(c)) => digit_visual(c, p),
                        Ok(None) => Visual::new(),
                        Err(_) => Visual::new().label("XPDR —"),
                    })
                })
                .on_adjust(|ctx| {
                    let p = place(&ctx.input_str("place"));
                    let code = need_code()?;
                    let next = match (ctx.value, ctx.delta.map(values::steps)) {
                        (Some(v), _) => values::with_digit(code, p, v.round().clamp(0.0, 7.0) as u32),
                        (None, Some(n)) if n != 0 => values::step_digit(code, p, n),
                        _ => return Ok(()),
                    };
                    let now = set_code(next)?;
                    ctx.fan_out_visual_by(&["place"], digit_visual(now, p));
                    ctx.runtime().notify_visual("transponder", Visual::new().label(format!("XPDR {now:04}")));
                    Ok(())
                }),
        )
        // Gear, flaps, spoilers, brakes
        .action(
            Action::toggle("gear", "Landing Gear")
                .icon("plane-landing")
                .state("up", "Gear Up", "plane-takeoff")
                .state("down", "Gear Down", "plane-landing")
                .live(POLL, |_| {
                    let handle = read("GEAR HANDLE POSITION", BOOL).map_err(Error::msg)?;
                    let pct = read("GEAR TOTAL PCT EXTENDED", PERCENT).ok().flatten();
                    Ok(match handle {
                        Some(h) => {
                            let down = h >= 0.5;
                            let moving = pct.is_some_and(|p| p > 1.0 && p < 99.0);
                            Visual::new().state(if down { "down" } else { "up" }).label(if moving { "Gear Moving" } else if down { "Gear Down" } else { "Gear Up" })
                        }
                        None => Visual::new(),
                    })
                })
                .on_press(|ctx| {
                    if need("IS GEAR RETRACTABLE", BOOL)? < 0.5 {
                        return ctx.fail("This aircraft's landing gear doesn't retract");
                    }
                    let before = need("GEAR HANDLE POSITION", BOOL)?;
                    send("GEAR_TOGGLE", &[])?;
                    let down = settle("GEAR HANDLE POSITION", BOOL, before) >= 0.5;
                    ctx.fan_out_visual(Visual::new().state(if down { "down" } else { "up" }).label(if down { "Gear Down" } else { "Gear Up" }));
                    Ok(())
                }),
        )
        .action(
            Action::slider("flaps", "Flaps", 0.0, 100.0)
                .icon("chevrons-down")
                .step(1.0)
                .unit("%")
                .live(POLL, |_| Ok(live_flaps()))
                .on_adjust(|ctx| {
                    let n = need(FLAPS_COUNT, NUMBER)?;
                    if n < 0.5 {
                        return ctx.fail("This aircraft has no flaps");
                    }
                    let positions = n.round() as u32;
                    let index = need(FLAPS_INDEX, NUMBER)?;
                    match (ctx.value, ctx.delta.map(values::steps)) {
                        (Some(v), _) => {
                            let notch = values::flap_notch(v, positions);
                            if notch as f64 == index.round() {
                                ctx.fan_out_visual(flaps_visual(index, n));
                                return Ok(());
                            }
                            send("FLAPS_SET", &[values::flaps_data(notch, positions) as i64])?;
                        }
                        (None, Some(k)) if k != 0 => {
                            for _ in 0..k.unsigned_abs().min(positions as u64) {
                                send(if k > 0 { "FLAPS_INCR" } else { "FLAPS_DECR" }, &[])?;
                            }
                        }
                        _ => return Ok(()),
                    }
                    flaps_after(ctx, index, n);
                    Ok(())
                }),
        )
        .action(
            Action::button("flaps-step", "Flaps Up / Down")
                .icon("chevrons-up-down")
                .inputs(vec![input::select("direction", "Move", &[("down", "Down one notch"), ("up", "Up one notch"), ("full-down", "Full down"), ("full-up", "Full up")]).with("default", "down")])
                .live(POLL, |_| Ok(label_only(live_flaps())))
                .on_press(|ctx| {
                    let n = need(FLAPS_COUNT, NUMBER)?;
                    if n < 0.5 {
                        return ctx.fail("This aircraft has no flaps");
                    }
                    let index = need(FLAPS_INDEX, NUMBER)?;
                    send(
                        match ctx.input_str("direction").trim() {
                            "up" => "FLAPS_DECR",
                            "full-down" => "FLAPS_DOWN",
                            "full-up" => "FLAPS_UP",
                            _ => "FLAPS_INCR",
                        },
                        &[],
                    )?;
                    flaps_after(ctx, index, n);
                    Ok(())
                }),
        )
        .action(
            Action::slider("spoilers", "Spoilers", 0.0, 100.0)
                .icon("fold-vertical")
                .step(10.0)
                .unit("%")
                .live(POLL, |_| Ok(shown("Spoilers", read("SPOILERS HANDLE POSITION", PERCENT), |v| Visual::new().state(v.round()).label(format!("Spoilers {}%", v.round())))))
                .on_adjust(|ctx| {
                    let cur = need("SPOILERS HANDLE POSITION", PERCENT)?;
                    let Some(t) = values::target(cur, ctx.value, ctx.delta, 10.0) else { return Ok(()) };
                    send("SPOILERS_SET", &[values::pos16k(t) as i64])?;
                    let now = settle("SPOILERS HANDLE POSITION", PERCENT, cur);
                    ctx.fan_out_visual(Visual::new().state(now.round()).label(format!("Spoilers {}%", now.round())));
                    Ok(())
                }),
        )
        .action(single_toggle("spoilers-arm", "Spoilers Arm", &catalog::SPOILERS_ARM, ("Spoilers Disarmed", "fold-vertical"), ("Spoilers Armed", "unfold-vertical")))
        .action(single_toggle("parking-brake", "Parking Brake", &catalog::PARKING_BRAKE, ("Brake Off", "circle-parking-off"), ("Brake Set", "circle-parking")))
        // Lights and electrical
        .action(switch_toggle("lights", "Lights", "lightbulb", catalog::LIGHTS, catalog::options(catalog::LIGHTS), "Light", [("Off", "lightbulb-off"), ("On", "lightbulb")]))
        .action(switch_toggle("electrical", "Electrical Master", "zap", catalog::ELECTRICAL, catalog::options(catalog::ELECTRICAL), "Switch", [("Off", "zap-off"), ("On", "zap")]))
        // Trim and altimeter
        .action(
            Action::slider("trim", "Trim", -100.0, 100.0)
                .icon("sliders-horizontal")
                .step(0.5)
                .initial(0)
                .unit("%")
                .inputs(vec![input::select("axis", "Trim", &[("elevator", "Elevator"), ("aileron", "Aileron"), ("rudder", "Rudder")]).with("default", "elevator")])
                .live(POLL, |ic| {
                    let t = pick(catalog::TRIMS, &ic.input_str("axis"));
                    Ok(shown(t.label, read(t.var, PERCENT), |v| trim_visual(t.label, v)))
                })
                .on_adjust(|ctx| {
                    let t = pick(catalog::TRIMS, &ctx.input_str("axis"));
                    let cur = need(t.var, PERCENT)?;
                    let Some(to) = values::target(cur, ctx.value, ctx.delta, 0.5) else { return Ok(()) };
                    send(t.event, &[values::trim_data(to, t.scale) as i64])?;
                    ctx.fan_out_visual_by(&["axis"], trim_visual(t.label, settle(t.var, PERCENT, cur)));
                    Ok(())
                }),
        )
        .action(baro_slider("altimeter-inhg", "Altimeter (inHg)", "inhg"))
        .action(baro_slider("altimeter-hpa", "Altimeter (hPa)", "hpa"))
        .action(
            Action::button("altimeter-preset", "Altimeter STD / Local")
                .icon("gauge-circle")
                .inputs(vec![
                    input::select("mode", "Set to", &[("std", "Standard (29.92 inHg / 1013 hPa)"), ("local", "Local pressure (the sim's BAROMETRIC)")]).with("default", "std"),
                    input::select("units", "Show in", &[("inhg", "inHg"), ("hpa", "hPa")]).with("default", "inhg"),
                ])
                .live(POLL, |ic| {
                    let units = ic.input_str("units");
                    Ok(shown("BARO", read(catalog::BARO_VAR, catalog::BARO_UNIT), |v| Visual::new().label(baro_label(v, &units))))
                })
                .on_press(|ctx| {
                    let before = need(catalog::BARO_VAR, catalog::BARO_UNIT)?;
                    if ctx.input_str("mode").trim() == "local" {
                        send("BAROMETRIC", &[])?;
                    } else {
                        send("KOHLSMAN_SET", &[values::kohlsman(values::STD_HPA) as i64, 0])?;
                    }
                    let now = settle(catalog::BARO_VAR, catalog::BARO_UNIT, before);
                    ctx.fan_out_visual_by(&["units"], Visual::new().label(baro_label(now, &ctx.input_str("units"))));
                    Ok(())
                }),
        )
        // Sim
        .action(
            Action::toggle("pause", "Pause")
                .icon("pause")
                .state("off", "Running", "play")
                .state("on", "Paused", "pause")
                .live(POLL, |_| {
                    engine().pump();
                    Ok(match engine().paused().map_err(Error::msg)? {
                        Some(p) => Visual::new().state(paused_state(p)),
                        None => Visual::new(),
                    })
                })
                .on_press(|ctx| {
                    let before = engine().paused().ok().flatten();
                    send("PAUSE_TOGGLE", &[])?;
                    let deadline = Instant::now() + SETTLE;
                    while Instant::now() < deadline {
                        engine().pump();
                        let now = engine().paused().ok().flatten();
                        if now.is_some() && now != before {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    if let Some(p) = engine().paused().ok().flatten() {
                        ctx.fan_out(paused_state(p));
                    }
                    Ok(())
                }),
        )
        .action(
            Action::button("sim-rate", "Sim Rate")
                .icon("fast-forward")
                .inputs(vec![input::select("direction", "Change", &[("faster", "Faster (×2)"), ("slower", "Slower (÷2)")]).with("default", "faster")])
                .live(POLL, |_| Ok(shown("Rate", read("SIMULATION RATE", NUMBER), |v| Visual::new().label(format!("Rate {}", values::fmt_rate(v))))))
                .on_press(|ctx| {
                    let before = need("SIMULATION RATE", NUMBER)?;
                    send(if ctx.input_str("direction").trim() == "slower" { "SIM_RATE_DECR" } else { "SIM_RATE_INCR" }, &[])?;
                    let now = settle("SIMULATION RATE", NUMBER, before);
                    ctx.fan_out_visual(Visual::new().label(format!("Rate {}", values::fmt_rate(now))));
                    Ok(())
                }),
        )
        .action(
            Action::button("readout", "Flight Readout")
                .icon("gauge")
                .inputs(vec![input::select("value", "Show", &catalog::READOUTS.iter().map(|r| (r.key, r.label)).collect::<Vec<_>>()).with("default", "ias")])
                .live(POLL, |ic| {
                    let r = pick(catalog::READOUTS, &ic.input_str("value"));
                    Ok(shown(r.short, readout_value(r), |v| Visual::new().label(readout_text(r, v))))
                })
                .on_press(|ctx| {
                    let r = pick(catalog::READOUTS, &ctx.input_str("value"));
                    engine().ensure(Instant::now()).map_err(Error::msg)?;
                    readout_value(r).map_err(Error::msg)?;
                    Ok(())
                }),
        )
        // Power users
        .action(
            Action::button("custom-event", "Custom Event")
                .icon("square-function")
                .inputs(vec![
                    input::text("event", "Event ID").placeholder("AP_MASTER").with("help", "Any K-event from the MSFS SDK's Event IDs list, without K: (like TOGGLE_FUEL_VALVE_ALL or AP_ALT_VAR_SET_ENGLISH)."),
                    input::text("values", "Values (optional)").placeholder("10000").with("help", "Up to five whole numbers, separated by commas; 0x… for hex (BCD codes)."),
                ])
                .on_press(|ctx| {
                    let event = engine::event_name(&ctx.input_str("event")).map_err(Error::msg)?;
                    let data = parse_values(&ctx.input_str("values"))?;
                    send(&event, &data)?;
                    // The sim answers an unknown name asynchronously; give it a moment so the
                    // first press already says so.
                    for _ in 0..8 {
                        std::thread::sleep(Duration::from_millis(25));
                        let mut e = engine();
                        e.pump();
                        if let Some(err) = e.event_error(&event) {
                            return Err(Error::msg(err));
                        }
                    }
                    Ok(())
                }),
        )
        .action(
            Action::button("simvar", "SimVar Readout")
                .icon("variable")
                .inputs(vec![
                    input::text("var", "SimVar").placeholder("PLANE ALTITUDE").with("help", "Any A: variable from the MSFS SDK's Simulation Variables list; add :1, :2 for indexed ones."),
                    input::text("unit", "Unit").placeholder("feet").default_value("number"),
                    input::text("decimals", "Decimals").placeholder("0").default_value("0"),
                    input::text("prefix", "Label").placeholder("ALT"),
                ])
                .live(POLL, |ic| {
                    let prefix = ic.input_str("prefix");
                    let short = if prefix.trim().is_empty() { "SimVar".to_string() } else { prefix.trim().to_string() };
                    let Ok(k) = custom_var(&ic.input_str("var"), &ic.input_str("unit")) else { return Ok(Visual::new().label(format!("{short} ?"))) };
                    let decimals = ic.input_str("decimals");
                    Ok(shown(&short, read_key(&k, Rate::Frame), |v| Visual::new().label(custom_text(&prefix, v, &decimals))))
                })
                .on_press(|ctx| {
                    let k = custom_var(&ctx.input_str("var"), &ctx.input_str("unit")).map_err(Error::msg)?;
                    engine().ensure(Instant::now()).map_err(Error::msg)?;
                    let deadline = Instant::now() + FIRST_VALUE;
                    loop {
                        match read_key(&k, Rate::Frame).map_err(Error::msg)? {
                            Some(v) => {
                                ctx.set_label(custom_text(&ctx.input_str("prefix"), v, &ctx.input_str("decimals")));
                                return Ok(());
                            }
                            None if Instant::now() >= deadline => return ctx.fail(format!("Flight Simulator sent no value for {} yet", k.name)),
                            None => std::thread::sleep(Duration::from_millis(25)),
                        }
                    }
                }),
        )
        .run()
}

fn paused_state(paused: bool) -> &'static str {
    if paused {
        "on"
    } else {
        "off"
    }
}

fn trim_visual(label: &str, pct: f64) -> Visual {
    let v = values::round_to(pct, 1);
    Visual::new().state(v).label(format!("{label} {v:+.1}%"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_values() {
        assert_eq!(parse_values("").unwrap(), Vec::<i64>::new());
        assert_eq!(parse_values("7000").unwrap(), vec![7000]);
        assert_eq!(parse_values("0x1200, -500 ,3").unwrap(), vec![0x1200, -500, 3]);
        assert!(parse_values("1,2,3,4,5,6").is_err());
        assert!(parse_values("abc").is_err());
        assert!(parse_values("99999999999").is_err());
    }

    #[test]
    fn transponder_readings() {
        assert_eq!(xpdr_code(0x7000 as f64), Some(7000));
        // A sim that sends the code as a plain number still reads.
        assert_eq!(xpdr_code(1200.0), Some(1200));
    }

    #[test]
    fn visuals() {
        let v = bug_visual(&catalog::HEADING, 0.4);
        assert_eq!((v.state.as_deref(), v.label.as_deref()), (Some("0"), Some("HDG 360°")));
        let v = bug_visual(&catalog::VERTICAL_SPEED, -700.2);
        assert_eq!(v.label.as_deref(), Some("VS -700 fpm"));
        let v = freq_visual(&catalog::COMS[0], 118_025, Spacing::Com25, Part::Khz);
        assert_eq!((v.state.as_deref(), v.label.as_deref()), (Some("25"), Some("COM1 118.025")));
        let v = freq_visual(&catalog::NAVS[1], 110_500, Spacing::Nav50, Part::Mhz);
        assert_eq!((v.state.as_deref(), v.label.as_deref()), (Some("110"), Some("NAV2 110.50")));
        let v = flaps_visual(2.0, 4.0);
        assert_eq!((v.state.as_deref(), v.label.as_deref()), (Some("50"), Some("Flaps 2/4")));
        assert_eq!(baro_label(1013.25, "inhg"), "29.92 inHg");
        assert_eq!(baro_label(1013.25, "hpa"), "1013 hPa");
        assert_eq!(trim_visual("Elevator", 2.46).label.as_deref(), Some("Elevator +2.5%"));
        assert_eq!(custom_text("ALT", 1234.567, "1"), "ALT 1234.6");
        assert_eq!(custom_text("", 3.0, ""), "3");
        assert_eq!(digit_visual(1200, 1).state.as_deref(), Some("2"));
    }
}
