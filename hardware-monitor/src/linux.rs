//! Linux: the kernel's hwmon sysfs (`/sys/class/hwmon`: CPU, drives, memory, mainboard, AMD GPUs),
//! the amdgpu device files (load, VRAM), RAPL package power when readable, and NVIDIA through NVML.

use crate::nvml;
use crate::sensors::{Kind, Role, Scan, Sensor};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

const HWMON: &str = "/sys/class/hwmon";
const RAPL: &str = "/sys/class/powercap";

fn read(p: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn num(p: impl AsRef<Path>) -> Option<f64> {
    read(p)?.parse().ok()
}

/// The device a hwmon chip belongs to, named for people.
fn group(chip: &str, dir: &Path) -> String {
    let model = || read(dir.join("device/model")).filter(|m| !m.is_empty());
    match chip {
        "k10temp" | "coretemp" | "zenpower" | "cpu_thermal" | "soc_thermal" => "CPU".into(),
        "amdgpu" => "AMD GPU".into(),
        "nouveau" => "NVIDIA GPU".into(),
        "i915" | "xe" => "Intel GPU".into(),
        "nvme" => model().map(|m| format!("NVMe {m}")).unwrap_or_else(|| "NVMe".into()),
        "drivetemp" => model().map(|m| format!("Drive {m}")).unwrap_or_else(|| "Drive".into()),
        "spd5118" | "jc42" | "ee1004" => "Memory".into(),
        "acpitz" => "ACPI".into(),
        "thinkpad" | "dell_smm" | "asus" | "asus_wmi_sensors" | "asus-ec-sensors" | "applesmc" => format!("Mainboard ({chip})"),
        c if c.starts_with("nct") || c.starts_with("it8") || c.starts_with("w83") || c.starts_with("f71") => format!("Mainboard ({chip})"),
        c if c.starts_with("iwlwifi") || c.starts_with("r8169") || c.starts_with("mt79") || c.starts_with("ath") => "Network".into(),
        c => c.to_string(),
    }
}

/// A stable key for a chip: its device path (PCI / platform address), since hwmonN renumbers.
fn chip_key(chip: &str, dir: &Path) -> String {
    let dev = fs::canonicalize(dir.join("device")).ok().map(|p| p.to_string_lossy().trim_start_matches("/sys/devices/").to_string());
    match dev {
        Some(d) => format!("{chip}@{d}"),
        None => chip.to_string(),
    }
}

/// `temp1`, `fan2`, `power1` … channels present in a chip folder, sorted.
fn channels(dir: &Path, prefix: &str, suffix: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            let ch = n.strip_suffix(suffix)?;
            (ch.starts_with(prefix) && ch[prefix.len()..].chars().all(|c| c.is_ascii_digit())).then(|| ch.to_string())
        })
        .collect();
    v.sort_by_key(|c| c[prefix.len()..].parse::<u32>().unwrap_or(0));
    v.dedup();
    v
}

fn cpu_temp_priority(chip: &str, label: &str) -> Option<u8> {
    match (chip, label) {
        ("k10temp" | "zenpower", "Tdie") => Some(0),
        ("k10temp" | "zenpower", "Tctl") => Some(1),
        ("coretemp", l) if l.starts_with("Package id 0") => Some(0),
        ("coretemp", l) if l.starts_with("Package") => Some(1),
        ("cpu_thermal" | "soc_thermal", _) => Some(3),
        ("acpitz", _) => Some(8),
        _ => None,
    }
}

/// Sensors of one hwmon chip.
pub fn chip(dir: &Path, out: &mut Vec<Sensor>, gpu_index: &mut u32) {
    let Some(chip) = read(dir.join("name")) else { return };
    let mut group = group(&chip, dir);
    let key = chip_key(&chip, dir);
    let id = |ch: &str| format!("hwmon:{key}:{ch}");
    // amdgpu: a discrete card (≥ 2 GB VRAM) wins the GPU picks over an integrated one.
    let vram_total = num(dir.join("device/mem_info_vram_total"));
    let gpu_p = if vram_total.is_some_and(|t| t >= 2.0 * 1024.0 * 1024.0 * 1024.0) { 1 } else { 3 };
    if chip == "amdgpu" {
        *gpu_index += 1;
        if *gpu_index > 1 {
            group = format!("{group} {}", gpu_index);
        }
    }
    for ch in channels(dir, "temp", "_input") {
        let Some(v) = num(dir.join(format!("{ch}_input"))) else { continue };
        let label = read(dir.join(format!("{ch}_label"))).unwrap_or_default();
        let crit = num(dir.join(format!("{ch}_crit"))).or_else(|| num(dir.join(format!("{ch}_max")))).map(|m| m / 1000.0);
        let name = if label.is_empty() { "Temperature".to_string() } else { label.clone() };
        let mut s = Sensor::new(id(&ch), &group, name, Kind::Temp, v / 1000.0).max(crit);
        if let Some(p) = cpu_temp_priority(&chip, &label) {
            s = s.role(Role::CpuTemp, p);
        } else if chip == "amdgpu" && (label == "edge" || label.is_empty()) {
            s = s.role(Role::GpuTemp, gpu_p);
        } else if chip == "nouveau" {
            s = s.role(Role::GpuTemp, 2);
        }
        out.push(s);
    }
    for ch in channels(dir, "fan", "_input") {
        let Some(v) = num(dir.join(format!("{ch}_input"))) else { continue };
        let label = read(dir.join(format!("{ch}_label"))).unwrap_or_else(|| format!("Fan {}", &ch[3..]));
        let max = num(dir.join(format!("{ch}_max")));
        let s = Sensor::new(id(&ch), &group, label.clone(), Kind::Fan, v).max(max);
        out.push(if chip == "amdgpu" {
            s.role(Role::GpuFan, gpu_p)
        } else {
            s.role(Role::Fan, if label.to_lowercase().contains("cpu") { 0 } else { 1 })
        });
    }
    let mut powers = channels(dir, "power", "_input");
    for ch in channels(dir, "power", "_average") {
        if !powers.contains(&ch) {
            powers.push(ch);
        }
    }
    for ch in powers {
        let Some(v) = num(dir.join(format!("{ch}_input"))).or_else(|| num(dir.join(format!("{ch}_average")))) else { continue };
        let label = read(dir.join(format!("{ch}_label"))).unwrap_or_else(|| "Power".into());
        let cap = num(dir.join(format!("{ch}_cap"))).map(|c| c / 1e6);
        let mut s = Sensor::new(id(&ch), &group, label.clone(), Kind::Power, v / 1e6).max(cap);
        if chip == "amdgpu" {
            // An APU's amdgpu PPT is the whole package's power, a fair CPU package reading when
            // RAPL is root-only.
            s = if gpu_p == 1 { s.role(Role::GpuPower, gpu_p) } else { s.role(Role::CpuPower, 5) };
        } else if chip == "zenpower" && label.contains("Core") {
            s = s.role(Role::CpuPower, 2);
        }
        out.push(s);
    }
    if chip == "amdgpu" {
        if let Some(v) = num(dir.join("device/gpu_busy_percent")) {
            out.push(Sensor::new(id("busy"), &group, "Load", Kind::Load, v).role(Role::GpuLoad, gpu_p));
        }
        if let Some(used) = num(dir.join("device/mem_info_vram_used")) {
            let mb = |b: f64| b / 1048576.0;
            out.push(Sensor::new(id("vram"), &group, "VRAM used", Kind::Memory, mb(used)).max(vram_total.map(mb)).role(Role::GpuMemory, gpu_p));
        }
    }
}

/// RAPL package energy counters from the last scan, to turn into watts: path → (µJ, when).
static RAPL_LAST: Mutex<Vec<(PathBuf, f64, Instant)>> = Mutex::new(Vec::new());

/// CPU package power from RAPL (`intel-rapl:N`, also on AMD). Most kernels make `energy_uj`
/// root-only, then this is empty.
fn rapl(out: &mut Vec<Sensor>) {
    let mut last = RAPL_LAST.lock().unwrap();
    let mut next = vec![];
    let mut zones: Vec<PathBuf> = fs::read_dir(RAPL).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().matches(':').count() == 1)).collect();
    zones.sort();
    for (i, z) in zones.iter().enumerate() {
        let name = read(z.join("name")).unwrap_or_default();
        if !name.starts_with("package") {
            continue;
        }
        let Some(e) = num(z.join("energy_uj")) else { continue };
        let now = Instant::now();
        if let Some((_, prev, at)) = last.iter().find(|(p, ..)| p == z) {
            let range = num(z.join("max_energy_range_uj")).unwrap_or(f64::MAX);
            let de = if e >= *prev { e - prev } else { e + range - prev };
            let dt = now.duration_since(*at).as_secs_f64();
            if dt > 0.05 {
                let label = if zones.len() > 1 { format!("Package {i}") } else { "Package".into() };
                out.push(Sensor::new(format!("rapl:{name}"), "CPU", label, Kind::Power, de / 1e6 / dt).role(Role::CpuPower, i as u8));
            }
        }
        next.push((z.clone(), e, now));
    }
    *last = next;
}

pub fn scan() -> Scan {
    let mut sensors = vec![];
    let mut dirs: Vec<PathBuf> = fs::read_dir(HWMON).into_iter().flatten().flatten().map(|e| e.path()).collect();
    dirs.sort_by_key(|p| p.file_name().and_then(|n| n.to_str()?.strip_prefix("hwmon")?.parse::<u32>().ok()).unwrap_or(0));
    let mut gpus = 0;
    for d in dirs {
        chip(&d, &mut sensors, &mut gpus);
    }
    rapl(&mut sensors);
    sensors.extend(nvml::sensors());
    let hint = sensors.is_empty().then(|| "This computer's kernel reports no hardware sensors (/sys/class/hwmon is empty)".to_string());
    Scan { sensors, hint }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake k10temp + amdgpu tree in a temp folder.
    #[test]
    fn reads_fake_hwmon() {
        let root = std::env::temp_dir().join(format!("hwmon-test-{}", std::process::id()));
        let cpu = root.join("hwmon2");
        let gpu = root.join("hwmon7");
        fs::create_dir_all(&cpu).unwrap();
        fs::create_dir_all(gpu.join("device")).unwrap();
        for (f, v) in [("name", "k10temp"), ("temp1_input", "50125"), ("temp1_label", "Tctl"), ("temp3_input", "44875"), ("temp3_label", "Tccd1")] {
            fs::write(cpu.join(f), v).unwrap();
        }
        for (f, v) in [("name", "amdgpu"), ("temp1_input", "46000"), ("temp1_label", "edge"), ("power1_input", "42026000"), ("power1_label", "PPT"), ("fan1_input", "1200"), ("device/gpu_busy_percent", "7"), ("device/mem_info_vram_used", "16711680"), ("device/mem_info_vram_total", "536870912")] {
            fs::write(gpu.join(f), v).unwrap();
        }
        let mut out = vec![];
        let mut n = 0;
        chip(&cpu, &mut out, &mut n);
        chip(&gpu, &mut out, &mut n);
        fs::remove_dir_all(&root).unwrap();
        let scan = Scan { sensors: out, hint: None };
        let t = crate::sensors::find(&scan, "auto:cpu-temp").unwrap();
        assert_eq!((t.name.as_str(), t.value), ("Tctl", 50.125));
        assert_eq!(crate::sensors::find(&scan, "auto:gpu-temp").unwrap().value, 46.0);
        assert_eq!(crate::sensors::find(&scan, "auto:gpu-load").unwrap().value, 7.0);
        let vram = crate::sensors::find(&scan, "auto:gpu-memory").unwrap();
        assert_eq!((vram.value, vram.max), (15.9375, Some(512.0)));
        // integrated GPU: its PPT is the package's power
        assert!((crate::sensors::find(&scan, "auto:cpu-power").unwrap().value - 42.026).abs() < 1e-9);
        assert_eq!(crate::sensors::find(&scan, "auto:gpu-fan").unwrap().value, 1200.0);
    }

    /// The real sensors of this machine (prints them).
    #[test]
    fn live_scan() {
        let s = scan();
        for x in &s.sensors {
            eprintln!("{:<70} {:<40} {:>10.2} {:?} {:?}", x.id, x.title(), x.value, x.kind, x.role);
        }
    }
}
