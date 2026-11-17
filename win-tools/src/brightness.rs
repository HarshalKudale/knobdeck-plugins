//! Display brightness, per monitor.
//!
//! - **Laptop panels** (internal / embedded outputs): WMI `root\WMI` `WmiMonitorBrightness` and
//!   `WmiMonitorBrightnessMethods.WmiSetBrightness`, what the Windows brightness slider uses.
//! - **External monitors**: DDC/CI through the Monitor Configuration API (`dxva2`:
//!   `GetMonitorBrightness` / `SetMonitorBrightness` on the physical monitors of an `HMONITOR`).
//!
//! Monitors are found with `QueryDisplayConfig` (friendly name, output technology and the
//! device path used as a stable id) and matched to an `HMONITOR` by GDI device name.

use knobdeck_sdk::{Error, Result};
use windows::core::{BSTR, VARIANT};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, GetMonitorBrightness, GetNumberOfPhysicalMonitorsFromHMONITOR, GetPhysicalMonitorsFromHMONITOR, QueryDisplayConfig, SetMonitorBrightness,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME, PHYSICAL_MONITOR, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::{BOOL, ERROR_SUCCESS, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoSetProxyBlanket, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE};
use windows::Win32::System::Wmi::{IEnumWbemClassObject, IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_GENERIC_FLAG_TYPE, WBEM_INFINITE};

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// The monitor's device path (stable across restarts and re-plugging into the same port).
    pub id: String,
    pub name: String,
    pub internal: bool,
}

struct Target {
    monitor: Monitor,
    /// GDI device name of the source showing it (`\\.\DISPLAY1`).
    gdi: String,
}

fn wide(s: &[u16]) -> String {
    String::from_utf16_lossy(&s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())])
}

fn targets() -> Result<Vec<Target>> {
    let (mut np, mut nm) = (0u32, 0u32);
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm) } != ERROR_SUCCESS {
        return Err(Error::msg("Windows didn't list the displays"));
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
    if unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut np, paths.as_mut_ptr(), &mut nm, modes.as_mut_ptr(), None) } != ERROR_SUCCESS {
        return Err(Error::msg("Windows didn't list the displays"));
    }
    paths.truncate(np as usize);
    let mut out = vec![];
    for (i, p) in paths.iter().enumerate() {
        let mut t = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
        t.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
        t.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
        t.header.adapterId = p.targetInfo.adapterId;
        t.header.id = p.targetInfo.id;
        if unsafe { DisplayConfigGetDeviceInfo(&mut t.header) } != 0 {
            continue;
        }
        let mut s = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        s.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        s.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        s.header.adapterId = p.sourceInfo.adapterId;
        s.header.id = p.sourceInfo.id;
        if unsafe { DisplayConfigGetDeviceInfo(&mut s.header) } != 0 {
            continue;
        }
        let tech = t.outputTechnology;
        let internal = tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED || tech == DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED;
        let friendly = wide(&t.monitorFriendlyDeviceName);
        let name = if !friendly.is_empty() {
            friendly
        } else if internal {
            "Built-in display".to_string()
        } else {
            format!("Display {}", i + 1)
        };
        let path = wide(&t.monitorDevicePath);
        let id = if path.is_empty() { format!("display-{i}") } else { path };
        out.push(Target { monitor: Monitor { id, name, internal }, gdi: wide(&s.viewGdiDeviceName) });
    }
    Ok(out)
}

pub fn monitors() -> Result<Vec<Monitor>> {
    Ok(targets()?.into_iter().map(|t| t.monitor).collect())
}

fn find(id: &str) -> Result<Target> {
    targets()?.into_iter().find(|t| t.monitor.id == id).ok_or_else(|| Error::msg("That display isn't connected"))
}

pub fn get(id: &str) -> Result<f64> {
    let t = find(id)?;
    if t.monitor.internal {
        return wmi::get();
    }
    with_physical(&t, |h| {
        let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
        if unsafe { GetMonitorBrightness(h, &mut min, &mut cur, &mut max) } == 0 {
            return Err(no_ddc(&t.monitor.name));
        }
        Ok(if max > min { (cur.saturating_sub(min)) as f64 * 100.0 / (max - min) as f64 } else { 0.0 })
    })
}

pub fn set(id: &str, percent: f64) -> Result<()> {
    let p = percent.clamp(0.0, 100.0);
    let t = find(id)?;
    if t.monitor.internal {
        return wmi::set(p.round() as u8);
    }
    with_physical(&t, |h| {
        let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
        if unsafe { GetMonitorBrightness(h, &mut min, &mut cur, &mut max) } == 0 {
            return Err(no_ddc(&t.monitor.name));
        }
        let value = min + ((max.saturating_sub(min)) as f64 * p / 100.0).round() as u32;
        if unsafe { SetMonitorBrightness(h, value) } == 0 {
            return Err(no_ddc(&t.monitor.name));
        }
        Ok(())
    })
}

fn no_ddc(name: &str) -> Error {
    Error::msg(format!("{name} doesn't accept brightness changes over DDC/CI. Turn DDC/CI on in the monitor's own menu (some monitors, TVs and docks don't support it)"))
}

unsafe extern "system" fn monitor_cb(hm: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<(HMONITOR, String)>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hm, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        list.push((hm, wide(&info.szDevice)));
    }
    TRUE
}

/// Run `f` on the first physical monitor behind the target's `HMONITOR`, then release them.
fn with_physical<T>(t: &Target, f: impl Fn(windows::Win32::Foundation::HANDLE) -> Result<T>) -> Result<T> {
    let mut list: Vec<(HMONITOR, String)> = vec![];
    let _ = unsafe { EnumDisplayMonitors(HDC::default(), None, Some(monitor_cb), LPARAM(&mut list as *mut _ as isize)) };
    let hm = list.into_iter().find(|(_, g)| *g == t.gdi).map(|(h, _)| h).ok_or_else(|| Error::msg(format!("{} isn't showing anything (is it mirrored or off?)", t.monitor.name)))?;
    let mut n = 0u32;
    unsafe { GetNumberOfPhysicalMonitorsFromHMONITOR(hm, &mut n) }.map_err(|_| no_ddc(&t.monitor.name))?;
    if n == 0 {
        return Err(no_ddc(&t.monitor.name));
    }
    let mut phys = vec![PHYSICAL_MONITOR::default(); n as usize];
    unsafe { GetPhysicalMonitorsFromHMONITOR(hm, &mut phys) }.map_err(|_| no_ddc(&t.monitor.name))?;
    let r = f(phys[0].hPhysicalMonitor);
    let _ = unsafe { DestroyPhysicalMonitors(&phys) };
    r
}

/// The laptop panel's brightness through WMI.
mod wmi {
    use super::*;

    fn services() -> Result<IWbemServices> {
        let werr = |e: windows::core::Error| Error::msg(format!("WMI unavailable: {e}"));
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let locator: IWbemLocator = unsafe { CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER) }.map_err(werr)?;
        let svc = unsafe { locator.ConnectServer(&BSTR::from("ROOT\\WMI"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None) }.map_err(werr)?;
        // RPC_C_AUTHN_WINNT (10), RPC_C_AUTHZ_NONE (0).
        unsafe { CoSetProxyBlanket(&svc, 10, 0, None, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE) }.map_err(werr)?;
        Ok(svc)
    }

    fn first(svc: &IWbemServices, query: &str) -> Result<IWbemClassObject> {
        let flags = WBEM_GENERIC_FLAG_TYPE(WBEM_FLAG_FORWARD_ONLY.0 | WBEM_FLAG_RETURN_IMMEDIATELY.0);
        let e: IEnumWbemClassObject = unsafe { svc.ExecQuery(&BSTR::from("WQL"), &BSTR::from(query), flags, None) }.map_err(|e| Error::msg(format!("WMI query failed: {e}")))?;
        let mut objs = [None];
        let mut n = 0u32;
        let _ = unsafe { e.Next(WBEM_INFINITE, &mut objs, &mut n) };
        objs[0].take().filter(|_| n > 0).ok_or_else(|| Error::msg("This display's brightness can't be set by Windows (no brightness control reported)"))
    }

    pub fn get() -> Result<f64> {
        let svc = services()?;
        let obj = first(&svc, "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE")?;
        let mut v = VARIANT::default();
        unsafe { obj.Get(windows::core::w!("CurrentBrightness"), 0, &mut v, None, None) }.map_err(|e| Error::msg(format!("WMI read failed: {e}")))?;
        Ok(i32::try_from(&v).unwrap_or(0) as f64)
    }

    pub fn set(percent: u8) -> Result<()> {
        let svc = services()?;
        let werr = |e: windows::core::Error| Error::msg(format!("WMI brightness change failed: {e}"));
        let instance = first(&svc, "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active=TRUE")?;
        let mut path = VARIANT::default();
        unsafe { instance.Get(windows::core::w!("__PATH"), 0, &mut path, None, None) }.map_err(werr)?;
        let path = BSTR::try_from(&path).map_err(werr)?;
        let mut class = None;
        unsafe { svc.GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut class), None) }.map_err(werr)?;
        let class: IWbemClassObject = class.ok_or_else(|| Error::msg("WMI has no WmiMonitorBrightnessMethods"))?;
        let mut sig = None;
        unsafe { class.GetMethod(windows::core::w!("WmiSetBrightness"), 0, &mut sig, std::ptr::null_mut()) }.map_err(werr)?;
        let params = unsafe { sig.ok_or_else(|| Error::msg("WMI has no WmiSetBrightness"))?.SpawnInstance(0) }.map_err(werr)?;
        // Timeout (seconds the setting holds; 0 = indefinitely) and Brightness (percent).
        unsafe { params.Put(windows::core::w!("Timeout"), 0, &VARIANT::from(0i32), 0) }.map_err(werr)?;
        unsafe { params.Put(windows::core::w!("Brightness"), 0, &VARIANT::from(percent), 0) }.map_err(werr)?;
        unsafe { svc.ExecMethod(&path, &BSTR::from("WmiSetBrightness"), WBEM_GENERIC_FLAG_TYPE(0), None, &params, None, None) }.map_err(werr)
    }
}

#[cfg(test)]
mod tests {
    /// Reads (never sets) every display's brightness.
    #[test]
    #[ignore = "needs displays"]
    fn live_read() {
        for m in super::monitors().unwrap() {
            eprintln!("{m:?}: {:?}", super::get(&m.id));
        }
    }
}
