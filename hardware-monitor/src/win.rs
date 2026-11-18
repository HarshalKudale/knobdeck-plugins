//! Windows: CPU, mainboard, drive and GPU sensors from LibreHardwareMonitor's WMI namespace
//! (`root\LibreHardwareMonitor`, or OpenHardwareMonitor's `root\OpenHardwareMonitor`) while the user
//! runs it; Windows itself has no sensor API for normal apps. NVIDIA GPUs through NVML either way.
//! Without either monitor, ACPI thermal zones (`MSAcpi_ThermalZoneTemperature`) when Windows lets a
//! non-admin process read them.

use crate::nvml;
use crate::lhm::{from_rows, Row};
use crate::sensors::{Kind, Role, Scan, Sensor};
use windows::core::{BSTR, VARIANT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE};
use windows::Win32::System::Wmi::{IEnumWbemClassObject, IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE};

const HINT: &str = "For CPU and mainboard sensors, install and run LibreHardwareMonitor (librehardwaremonitor.org)";

fn connect(namespace: &str) -> Option<IWbemServices> {
    // SAFETY: plain COM calls; a thread already in another apartment keeps it (the error is ignored).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let locator: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;
        let svc = locator.ConnectServer(&BSTR::from(namespace), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None).ok()?;
        // RPC_C_AUTHN_WINNT (10), RPC_C_AUTHZ_NONE (0).
        CoSetProxyBlanket(&svc, 10, 0, None, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE).ok()?;
        Some(svc)
    }
}

fn query(svc: &IWbemServices, wql: &str) -> Vec<IWbemClassObject> {
    let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
    // SAFETY: WMI enumeration with valid out-buffers.
    let Ok(e): windows::core::Result<IEnumWbemClassObject> = (unsafe { svc.ExecQuery(&BSTR::from("WQL"), &BSTR::from(wql), flags, None) }) else { return vec![] };
    let mut out = vec![];
    loop {
        let mut objs = [None];
        let mut n = 0u32;
        let _ = unsafe { e.Next(WBEM_INFINITE, &mut objs, &mut n) };
        match objs[0].take() {
            Some(o) if n > 0 => out.push(o),
            _ => break,
        }
    }
    out
}

fn get(o: &IWbemClassObject, name: &str) -> Option<VARIANT> {
    let mut v = VARIANT::default();
    let w: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // SAFETY: `w` is NUL-terminated and outlives the call.
    unsafe { o.Get(windows::core::PCWSTR(w.as_ptr()), 0, &mut v, None, None) }.ok()?;
    Some(v)
}

fn string(o: &IWbemClassObject, name: &str) -> String {
    get(o, name).and_then(|v| BSTR::try_from(&v).ok()).map(|b| b.to_string()).unwrap_or_default()
}

fn float(o: &IWbemClassObject, name: &str) -> Option<f64> {
    f64::try_from(&get(o, name)?).ok()
}

/// Sensors from a running LibreHardwareMonitor or OpenHardwareMonitor.
fn monitor() -> Option<Vec<Sensor>> {
    for (ns, source) in [("ROOT\\LibreHardwareMonitor", "lhm"), ("ROOT\\OpenHardwareMonitor", "ohm")] {
        let Some(svc) = connect(ns) else { continue };
        let rows: Vec<Row> = query(&svc, "SELECT Identifier, Name, SensorType, Parent, Value, Max FROM Sensor")
            .iter()
            .filter_map(|o| {
                Some(Row { identifier: string(o, "Identifier"), name: string(o, "Name"), sensor_type: string(o, "SensorType"), parent: string(o, "Parent"), value: float(o, "Value")?, max: float(o, "Max") })
            })
            .collect();
        if rows.is_empty() {
            continue; // namespace left behind by a monitor that isn't running
        }
        let hardware: Vec<(String, String)> = query(&svc, "SELECT Identifier, Name FROM Hardware").iter().map(|o| (string(o, "Identifier"), string(o, "Name"))).collect();
        return Some(from_rows(&rows, &hardware, source));
    }
    None
}

/// ACPI thermal zones; most PCs answer only to administrators, then this is empty.
fn acpi() -> Vec<Sensor> {
    let Some(svc) = connect("ROOT\\WMI") else { return vec![] };
    query(&svc, "SELECT InstanceName, CurrentTemperature FROM MSAcpi_ThermalZoneTemperature")
        .iter()
        .enumerate()
        .filter_map(|(i, o)| {
            let tenth_k = float(o, "CurrentTemperature")?;
            let c = tenth_k / 10.0 - 273.15;
            (c > 0.0 && c < 150.0).then(|| Sensor::new(format!("acpi:{}", string(o, "InstanceName")), "ACPI", format!("Thermal zone {}", i + 1), Kind::Temp, c).role(Role::CpuTemp, 8))
        })
        .collect()
}

pub fn scan() -> Scan {
    let (mut sensors, hint) = match monitor() {
        Some(s) => (s, None),
        None => (acpi(), Some(HINT.to_string())),
    };
    sensors.extend(nvml::sensors());
    Scan { sensors, hint }
}
