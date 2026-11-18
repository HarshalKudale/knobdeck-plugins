//! Hardware Monitor plugin for KnobDeck: live temperatures, fan speeds, power and GPU load / VRAM
//! from the machine's sensors, on a key or a gauge, with an optional alert threshold.
//!
//! Routes (read-only): Linux hwmon sysfs + amdgpu + RAPL; Windows LibreHardwareMonitor's (or
//! OpenHardwareMonitor's) WMI namespace, ACPI thermal zones; macOS IOHIDEventSystemClient (Apple
//! silicon) and the SMC through IOKit; NVIDIA GPUs through NVML on Windows and Linux.

#[cfg(any(windows, target_os = "linux"))]
mod nvml;
#[cfg(any(windows, test))]
mod lhm;
mod sensors;

use base64::Engine as _;
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, InitialCtx, Plugin, Value, Visual};
use sensors::{Kind, Sensor};
use std::time::Duration;

const EVERY: Duration = Duration::from_secs(2);
const UNITS: &[(&str, &str)] = &[("c", "°C"), ("f", "°F")];

fn sensor_input(key: &str, label: &str, default: &str) -> Value {
    input::query_select(key, label, "sensors", &[]).default_value(default)
}

fn common_inputs() -> Vec<Value> {
    vec![
        input::select("unit", "Temperature unit", UNITS).default_value("c"),
        input::text("alert", "Alert at or above").placeholder("Off").with("help", "The key turns red when the reading reaches this value (in the sensor's unit: °C/°F, RPM, W, %, MB)."),
        input::text("name", "Name on the key").placeholder("Automatic"),
    ]
}

/// The short name a tile shows: the user's, the auto pick's, or the sensor's own.
fn short_name(ic: &InitialCtx, id: &str, s: &Sensor) -> String {
    let custom = ic.input_str("name");
    if !custom.trim().is_empty() {
        return custom.trim().to_string();
    }
    if let Some(n) = sensors::auto_short(id) {
        return n.to_string();
    }
    let generic = ["", "Temperature", "Load", "Power", "Fan", "Memory used", "VRAM used", "Composite"];
    if generic.contains(&s.name.as_str()) {
        s.group.clone()
    } else {
        s.name.clone()
    }
}

fn sensor_id(ic: &InitialCtx, key: &str) -> String {
    let id = ic.input_str(key);
    if id.trim().is_empty() {
        (if key == "sensor_b" { "auto:gpu-temp" } else { "auto:cpu-temp" }).to_string()
    } else {
        id
    }
}

fn alerting(ic: &InitialCtx, s: &Sensor) -> bool {
    let unit_f = ic.input_str("unit") == "f";
    ic.input_str("alert").trim().replace(',', ".").parse::<f64>().is_ok_and(|t| sensors::comparable(s, unit_f) >= t)
}

fn svg_uri(svg: String) -> String {
    format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg))
}

/// Ring gauge (linux-tools System Stats style) filled to `frac`, with `text` in the middle.
fn gauge(frac: f64, text: &str, alert: bool) -> String {
    let f = frac.clamp(0.0, 1.0);
    let color = if alert || f >= 0.85 { "#ef4444" } else if f >= 0.6 { "#f59e0b" } else { "#22c55e" };
    let circ = 2.0 * std::f64::consts::PI * 40.0;
    let size = if text.chars().count() > 4 { 22 } else { 28 };
    svg_uri(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><circle cx="50" cy="50" r="40" fill="none" stroke="#888888" stroke-opacity="0.3" stroke-width="10"/><circle cx="50" cy="50" r="40" fill="none" stroke="{color}" stroke-width="10" stroke-linecap="round" stroke-dasharray="{:.1} {circ:.1}" transform="rotate(-90 50 50)"/><text x="50" y="{}" text-anchor="middle" font-family="sans-serif" font-size="{size}" font-weight="600" fill="#888888">{text}</text></svg>"##,
        circ * f,
        50 + size * 2 / 5
    ))
}

/// The number in a gauge's centre ("54°", "34%", "28W", "1.2G", "1250").
fn gauge_text(s: &Sensor, unit_f: bool) -> String {
    let v = sensors::comparable(s, unit_f);
    match s.kind {
        Kind::Temp => format!("{v:.0}°"),
        Kind::Load | Kind::FanPct => format!("{v:.0}%"),
        Kind::Power if v < 10.0 => format!("{v:.1}W"),
        Kind::Power => format!("{v:.0}W"),
        Kind::Memory if v >= 1024.0 => format!("{:.1}G", v / 1024.0),
        Kind::Memory => format!("{v:.0}M"),
        Kind::Fan => format!("{v:.0}"),
    }
}

fn parse(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse().ok()
}

fn readout(ic: &InitialCtx) -> knobdeck_sdk::Result<Visual> {
    let id = sensor_id(ic, "sensor");
    let scan = sensors::scan();
    let Some(s) = sensors::find(&scan, &id) else {
        return Ok(Visual::new().state("normal").label(sensors::missing(&scan, &id)).icon("circle-help"));
    };
    let unit_f = ic.input_str("unit") == "f";
    let alert = alerting(ic, s);
    Ok(Visual::new()
        .state(if alert { "alert" } else { "normal" })
        .label(format!("{}\n{}", short_name(ic, &id, s), sensors::format(s, unit_f)))
        .icon(if alert { "triangle-alert" } else { sensors::icon(s.kind) }))
}

fn gauge_visual(ic: &InitialCtx) -> knobdeck_sdk::Result<Visual> {
    let id = sensor_id(ic, "sensor");
    let scan = sensors::scan();
    let Some(s) = sensors::find(&scan, &id) else {
        return Ok(Visual::new().state("normal").label(sensors::missing(&scan, &id)).icon("circle-help"));
    };
    let unit_f = ic.input_str("unit") == "f";
    let (mut lo, mut hi) = sensors::scale(s);
    if s.kind == Kind::Temp && unit_f {
        (lo, hi) = (sensors::fahrenheit(lo), sensors::fahrenheit(hi));
    }
    let lo = parse(&ic.input_str("min")).unwrap_or(lo);
    let hi = parse(&ic.input_str("max")).unwrap_or(hi);
    let frac = if hi > lo { (sensors::comparable(s, unit_f) - lo) / (hi - lo) } else { 0.0 };
    let alert = alerting(ic, s);
    Ok(Visual::new()
        .state(if alert { "alert" } else { "normal" })
        .label(format!("{} {}", short_name(ic, &id, s), sensors::format(s, unit_f)))
        .icon(gauge(frac, &gauge_text(s, unit_f), alert)))
}

fn pair(ic: &InitialCtx) -> knobdeck_sdk::Result<Visual> {
    let scan = sensors::scan();
    let unit_f = ic.input_str("unit") == "f";
    let line = |key: &str| {
        let id = sensor_id(ic, key);
        match sensors::find(&scan, &id) {
            Some(s) => {
                let name = sensors::auto_short(&id).map(str::to_string).unwrap_or_else(|| if s.name.is_empty() { s.group.clone() } else { s.name.clone() });
                format!("{name} {}", sensors::format(s, unit_f))
            }
            None => format!("{} n/a", sensors::auto_short(&id).unwrap_or("Sensor")),
        }
    };
    Ok(Visual::new().label(format!("{}\n{}", line("sensor"), line("sensor_b"))))
}

fn main() {
    Plugin::new(&format!("hardware-monitor-{}", std::env::consts::OS), "Hardware Monitor")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Live CPU and GPU temperatures, fan speeds, power, GPU load and VRAM from your computer's sensors, on a key or a gauge.")
        .icon("thermometer")
        .query("sensors", |_q| {
            let scan = sensors::scan();
            let mut list: Vec<Value> = sensors::AUTO
                .iter()
                .map(|(v, l, _)| {
                    let now = sensors::find(&scan, v).map(|s| format!(" — {}", sensors::format(s, false))).unwrap_or_else(|| " — not found".into());
                    json!({"value": v, "label": format!("{l} (automatic){now}")})
                })
                .collect();
            list.extend(scan.sensors.iter().map(|s| json!({"value": s.id, "label": format!("{} — {}", s.title(), sensors::format(s, false))})));
            Ok(Value::Array(list))
        })
        .action({
            let mut inputs = vec![sensor_input("sensor", "Sensor", "auto:cpu-temp")];
            inputs.extend(common_inputs());
            Action::button("sensor", "Sensor")
                .icon("thermometer")
                .inputs(inputs)
                .state("normal", "Sensor", "thermometer")
                .state("alert", "Alert", "triangle-alert")
                .live(EVERY, readout)
        })
        .action({
            let mut inputs = vec![sensor_input("sensor", "Sensor", "auto:cpu-temp")];
            inputs.extend(common_inputs());
            inputs.push(input::text("min", "Gauge from").placeholder("Automatic"));
            inputs.push(input::text("max", "Gauge to").placeholder("Automatic"));
            Action::button("gauge", "Sensor Gauge")
                .icon("gauge")
                .inputs(inputs)
                .state("normal", "Sensor Gauge", "gauge")
                .state("alert", "Alert", "triangle-alert")
                .live(EVERY, gauge_visual)
        })
        .action(
            Action::button("pair", "Two Sensors")
                .icon("thermometer")
                .inputs(vec![
                    sensor_input("sensor", "First sensor", "auto:cpu-temp"),
                    sensor_input("sensor_b", "Second sensor", "auto:gpu-temp"),
                    input::select("unit", "Temperature unit", UNITS).default_value("c"),
                ])
                .live(EVERY, pair),
        )
        .run()
}
