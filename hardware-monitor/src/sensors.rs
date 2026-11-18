//! The sensor model shared by every OS, the per-OS readers behind one seam, a short cache so many
//! keys reading at once cost one scan, and the "auto" picks (CPU temperature, GPU load, …) that
//! keep a layout working on another machine.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// macOS reports no GPU load / VRAM / fan percent sensors.
#[cfg_attr(target_os = "macos", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// °C
    Temp,
    /// RPM
    Fan,
    /// Percent of the fan's top speed (NVML, LibreHardwareMonitor "Control").
    FanPct,
    /// Watts
    Power,
    /// Percent busy
    Load,
    /// MB in use (`max` = total MB when known)
    Memory,
}

/// A role an "auto" pick looks for; lower `priority` wins.
#[cfg_attr(target_os = "macos", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    CpuTemp,
    CpuPower,
    GpuTemp,
    GpuLoad,
    GpuMemory,
    GpuPower,
    GpuFan,
    Fan,
}

#[derive(Clone, Debug)]
pub struct Sensor {
    /// Stable across restarts (never a hwmonN number): what a layout saves.
    pub id: String,
    /// The device it belongs to ("CPU", "NVIDIA GeForce RTX 4080", "NVMe Samsung 990 PRO").
    pub group: String,
    /// The sensor's own name ("Tctl", "Temperature", "Fan 1").
    pub name: String,
    pub kind: Kind,
    pub value: f64,
    /// A known top of scale (critical temperature, fan max, power limit, VRAM total).
    pub max: Option<f64>,
    pub role: Option<(Role, u8)>,
}

impl Sensor {
    pub fn new(id: impl Into<String>, group: impl Into<String>, name: impl Into<String>, kind: Kind, value: f64) -> Self {
        Sensor { id: id.into(), group: group.into(), name: name.into(), kind, value, max: None, role: None }
    }
    pub fn max(mut self, max: Option<f64>) -> Self {
        self.max = max.filter(|m| m.is_finite() && *m > 0.0);
        self
    }
    pub fn role(mut self, role: Role, priority: u8) -> Self {
        self.role = Some((role, priority));
        self
    }
    pub fn title(&self) -> String {
        if self.name.is_empty() {
            self.group.clone()
        } else {
            format!("{} · {}", self.group, self.name)
        }
    }
}

/// Every sensor this machine reports, plus why some may be missing (Windows without
/// LibreHardwareMonitor).
#[derive(Clone, Default)]
pub struct Scan {
    pub sensors: Vec<Sensor>,
    pub hint: Option<String>,
}

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod os;
#[cfg(target_os = "macos")]
#[path = "mac.rs"]
mod os;
#[cfg(windows)]
#[path = "win.rs"]
mod os;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod os {
    pub fn scan() -> super::Scan {
        super::Scan { sensors: vec![], hint: Some("Sensors aren't supported on this OS".into()) }
    }
}

/// Readings younger than this are shared; keys refresh every 2 s, so one scan serves them all.
const FRESH: Duration = Duration::from_millis(900);
static CACHE: Mutex<Option<(Instant, Arc<Scan>)>> = Mutex::new(None);
/// One scan at a time: a second caller waits for it instead of scanning again.
static SCANNING: Mutex<()> = Mutex::new(());

pub fn scan() -> Arc<Scan> {
    let fresh = || CACHE.lock().unwrap().as_ref().filter(|(at, _)| at.elapsed() < FRESH).map(|(_, s)| s.clone());
    if let Some(s) = fresh() {
        return s;
    }
    let _one = SCANNING.lock().unwrap();
    if let Some(s) = fresh() {
        return s;
    }
    let s = Arc::new(os::scan());
    *CACHE.lock().unwrap() = Some((Instant::now(), s.clone()));
    s
}

/// The "auto" picks offered first in the sensor list: (value, label, role).
pub const AUTO: &[(&str, &str, Role)] = &[
    ("auto:cpu-temp", "CPU temperature", Role::CpuTemp),
    ("auto:cpu-power", "CPU package power", Role::CpuPower),
    ("auto:gpu-temp", "GPU temperature", Role::GpuTemp),
    ("auto:gpu-load", "GPU load", Role::GpuLoad),
    ("auto:gpu-memory", "GPU memory used", Role::GpuMemory),
    ("auto:gpu-power", "GPU power", Role::GpuPower),
    ("auto:gpu-fan", "GPU fan", Role::GpuFan),
    ("auto:fan", "System fan", Role::Fan),
];

pub fn auto_label(id: &str) -> Option<&'static str> {
    AUTO.iter().find(|(v, ..)| *v == id).map(|(_, l, _)| *l)
}

/// Short tile names for the auto picks.
pub fn auto_short(id: &str) -> Option<&'static str> {
    Some(match id {
        "auto:cpu-temp" => "CPU",
        "auto:cpu-power" => "CPU",
        "auto:gpu-temp" | "auto:gpu-load" | "auto:gpu-power" => "GPU",
        "auto:gpu-memory" => "VRAM",
        "auto:gpu-fan" => "GPU fan",
        "auto:fan" => "Fan",
        _ => return None,
    })
}

/// The sensor `id` names (a real id, or an auto pick) in `scan`.
pub fn find<'a>(scan: &'a Scan, id: &str) -> Option<&'a Sensor> {
    if let Some((_, _, role)) = AUTO.iter().find(|(v, ..)| *v == id) {
        return scan.sensors.iter().filter(|s| s.role.is_some_and(|(r, _)| r == *role)).min_by_key(|s| s.role.map(|(_, p)| p));
    }
    scan.sensors.iter().find(|s| s.id == id)
}

/// Why `id` has no reading, as a sentence for the key.
pub fn missing(scan: &Scan, id: &str) -> String {
    let what = auto_label(id).map(|l| format!("No {} sensor", l.to_lowercase())).unwrap_or_else(|| "Sensor not found".to_string());
    match &scan.hint {
        Some(h) if !id.starts_with("auto:gpu") => format!("{what}. {h}"),
        _ => what,
    }
}

pub fn fahrenheit(c: f64) -> f64 {
    c * 9.0 / 5.0 + 32.0
}

/// The value as the key shows it ("54 °C", "1 250 RPM", "28.5 W", "1.2 / 16 GB").
pub fn format(s: &Sensor, unit_f: bool) -> String {
    let v = s.value;
    match s.kind {
        Kind::Temp if unit_f => format!("{:.0} °F", fahrenheit(v)),
        Kind::Temp => format!("{v:.0} °C"),
        Kind::Fan => format!("{v:.0} RPM"),
        Kind::FanPct | Kind::Load => format!("{v:.0}%"),
        Kind::Power if v < 10.0 => format!("{v:.1} W"),
        Kind::Power => format!("{v:.0} W"),
        Kind::Memory => {
            let gb = |mb: f64| if mb >= 10240.0 { format!("{:.0}", mb / 1024.0) } else { format!("{:.1}", mb / 1024.0) };
            match s.max {
                Some(t) => format!("{} / {} GB", gb(v), gb(t)),
                None if v < 1024.0 => format!("{v:.0} MB"),
                None => format!("{} GB", gb(v)),
            }
        }
    }
}

/// The value in the unit the user types thresholds in (°C or °F for temperatures).
pub fn comparable(s: &Sensor, unit_f: bool) -> f64 {
    if s.kind == Kind::Temp && unit_f {
        fahrenheit(s.value)
    } else {
        s.value
    }
}

/// Default gauge scale for a sensor (in °C for temperatures).
pub fn scale(s: &Sensor) -> (f64, f64) {
    match s.kind {
        Kind::Temp => (20.0, s.max.filter(|m| (60.0..=130.0).contains(m)).unwrap_or(100.0)),
        Kind::Load | Kind::FanPct => (0.0, 100.0),
        Kind::Fan => (0.0, s.max.unwrap_or(3000.0)),
        Kind::Power => (0.0, s.max.unwrap_or(if s.role.is_some_and(|(r, _)| r == Role::GpuPower) { 300.0 } else { 150.0 })),
        Kind::Memory => (0.0, s.max.unwrap_or(8192.0)),
    }
}

pub fn icon(kind: Kind) -> &'static str {
    match kind {
        Kind::Temp => "thermometer",
        Kind::Fan | Kind::FanPct => "fan",
        Kind::Power => "zap",
        Kind::Load => "cpu",
        Kind::Memory => "memory-stick",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_picks_lowest_priority() {
        let scan = Scan {
            sensors: vec![
                Sensor::new("a", "ACPI", "", Kind::Temp, 40.0).role(Role::CpuTemp, 8),
                Sensor::new("b", "CPU", "Tctl", Kind::Temp, 50.0).role(Role::CpuTemp, 1),
                Sensor::new("c", "GPU", "", Kind::Temp, 60.0).role(Role::GpuTemp, 0),
            ],
            hint: None,
        };
        assert_eq!(find(&scan, "auto:cpu-temp").unwrap().id, "b");
        assert_eq!(find(&scan, "c").unwrap().value, 60.0);
        assert!(find(&scan, "auto:fan").is_none());
        assert_eq!(missing(&scan, "auto:fan"), "No system fan sensor");
    }

    #[test]
    fn formats() {
        let t = Sensor::new("t", "CPU", "", Kind::Temp, 50.125);
        assert_eq!(format(&t, false), "50 °C");
        assert_eq!(format(&t, true), "122 °F");
        assert_eq!(format(&Sensor::new("p", "", "", Kind::Power, 8.44), false), "8.4 W");
        assert_eq!(format(&Sensor::new("m", "", "", Kind::Memory, 1270.0).max(Some(16376.0)), false), "1.2 / 16 GB");
        assert_eq!(format(&Sensor::new("m", "", "", Kind::Memory, 512.0), false), "512 MB");
    }
}
