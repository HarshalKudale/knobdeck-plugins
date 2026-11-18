//! LibreHardwareMonitor / OpenHardwareMonitor WMI rows → sensors (Windows reads them in
//! `win.rs`; the mapping is plain data, so it is tested on every OS).

use crate::sensors::{Kind, Role, Sensor};

/// One LibreHardwareMonitor / OpenHardwareMonitor sensor row, as read from WMI.
pub struct Row {
    pub identifier: String,
    pub name: String,
    pub sensor_type: String,
    pub parent: String,
    pub value: f64,
    pub max: Option<f64>,
}

/// Turn monitor rows into sensors. `hardware` maps a hardware identifier to its name.
pub fn from_rows(rows: &[Row], hardware: &[(String, String)], source: &str) -> Vec<Sensor> {
    let hw_name = |id: &str| hardware.iter().find(|(i, _)| i == id).map(|(_, n)| n.clone()).unwrap_or_else(|| id.to_string());
    // "GPU Memory Total" lets "GPU Memory Used" show "x / y GB".
    let total = |parent: &str| rows.iter().find(|r| r.parent == parent && r.sensor_type == "SmallData" && r.name == "GPU Memory Total").map(|r| r.value);
    let mut out = vec![];
    for r in rows {
        let hw = r.parent.as_str();
        let cpu = hw.starts_with("/amdcpu") || hw.starts_with("/intelcpu");
        let gpu = hw.starts_with("/gpu");
        let (kind, role) = match r.sensor_type.as_str() {
            "Temperature" => {
                let role = if cpu {
                    match r.name.as_str() {
                        "Core (Tctl/Tdie)" | "CPU Package" => Some((Role::CpuTemp, 0)),
                        "Core Average" | "CPU Cores" => Some((Role::CpuTemp, 1)),
                        "Core Max" => Some((Role::CpuTemp, 2)),
                        _ => None,
                    }
                } else if gpu && r.name == "GPU Core" {
                    Some((Role::GpuTemp, 2))
                } else {
                    None
                };
                (Kind::Temp, role)
            }
            "Fan" => (Kind::Fan, Some(if gpu { (Role::GpuFan, 2) } else { (Role::Fan, if r.name.to_lowercase().contains("cpu") { 0 } else { 1 }) })),
            "Power" => {
                let role = if cpu && (r.name == "Package" || r.name == "CPU Package") {
                    Some((Role::CpuPower, 0))
                } else if gpu && (r.name == "GPU Package" || r.name == "GPU Power") {
                    Some((Role::GpuPower, 2))
                } else {
                    None
                };
                (Kind::Power, role)
            }
            "Load" => (Kind::Load, (gpu && r.name == "GPU Core").then_some((Role::GpuLoad, 2))),
            "SmallData" if gpu && r.name == "GPU Memory Used" => (Kind::Memory, Some((Role::GpuMemory, 2))),
            "Control" => (Kind::FanPct, None),
            _ => continue,
        };
        let max = if kind == Kind::Memory { total(hw) } else { r.max.filter(|_| kind == Kind::Fan) };
        let mut s = Sensor::new(format!("{source}:{}", r.identifier), hw_name(hw), &r.name, kind, r.value).max(max);
        if let Some((role, p)) = role {
            s = s.role(role, p);
        }
        out.push(s);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensors::Scan;

    #[test]
    fn maps_lhm_rows() {
        let row = |id: &str, name: &str, t: &str, parent: &str, v: f64| Row { identifier: id.into(), name: name.into(), sensor_type: t.into(), parent: parent.into(), value: v, max: None };
        let rows = vec![
            row("/amdcpu/0/temperature/2", "Core (Tctl/Tdie)", "Temperature", "/amdcpu/0", 55.0),
            row("/amdcpu/0/power/0", "Package", "Power", "/amdcpu/0", 40.0),
            row("/gpu-nvidia/0/smalldata/1", "GPU Memory Used", "SmallData", "/gpu-nvidia/0", 1200.0),
            row("/gpu-nvidia/0/smalldata/2", "GPU Memory Total", "SmallData", "/gpu-nvidia/0", 16376.0),
            row("/lpc/nct6798d/0/fan/1", "Fan #2", "Fan", "/lpc/nct6798d/0", 900.0),
            row("/lpc/nct6798d/0/voltage/0", "Vcore", "Voltage", "/lpc/nct6798d/0", 1.2),
        ];
        let hw = vec![("/amdcpu/0".to_string(), "AMD Ryzen 9 7950X".to_string())];
        let scan = Scan { sensors: from_rows(&rows, &hw, "lhm"), hint: None };
        assert_eq!(scan.sensors.len(), 4);
        let t = crate::sensors::find(&scan, "auto:cpu-temp").unwrap();
        assert_eq!((t.group.as_str(), t.value), ("AMD Ryzen 9 7950X", 55.0));
        assert_eq!(crate::sensors::find(&scan, "auto:cpu-power").unwrap().value, 40.0);
        assert_eq!(crate::sensors::find(&scan, "auto:gpu-memory").unwrap().max, Some(16376.0));
        assert_eq!(crate::sensors::find(&scan, "auto:fan").unwrap().id, "lhm:/lpc/nct6798d/0/fan/1");
    }
}
