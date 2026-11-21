//! SimConnect over its C API: `SimConnect.dll`, loaded at run time (a missing DLL is an error the
//! user can act on, never a crash at launch). The DLL isn't bundled: the MSFS SDK licence doesn't
//! clearly allow redistributing it, so it is loaded from where Microsoft's installers put it:
//!
//! 1. `SIMCONNECT_DLL` in the plugin's `.env` (a file or its folder), for unusual installs;
//! 2. `SimConnect.dll` in the plugin's data folder or next to the plugin (a copy the user put there);
//! 3. the running sim's own folder (the Steam edition ships one next to `FlightSimulator.exe`);
//! 4. the MSFS SDK: `%MSFS_SDK%` / `%MSFS2024_SDK%` (also read from the registry, so an SDK
//!    installed after login is found) and their default folders, `SimConnect SDK\lib`. The SDK that
//!    matches the running sim comes first.
//!
//! One connection at a time, used under the engine's lock (SimConnect handles aren't shared
//! between threads concurrently).

use crate::sim::{parse_recv, Connector, Link, LinkError, OpenError, Rate, Recv};
use std::ffi::{c_char, c_void, CString};
use std::path::{Path, PathBuf};
use windows::core::{s, w, HSTRING, PCSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HMODULE};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH};
use windows::Win32::System::Registry::{RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};

type Handle = *mut c_void;
type Hr = i32;

type FnOpen = unsafe extern "system" fn(*mut Handle, *const c_char, *mut c_void, u32, *mut c_void, u32) -> Hr;
type FnClose = unsafe extern "system" fn(Handle) -> Hr;
type FnMap = unsafe extern "system" fn(Handle, u32, *const c_char) -> Hr;
// ObjectID, EventID, dwData, GroupID, Flags
type FnTransmit = unsafe extern "system" fn(Handle, u32, u32, u32, u32, u32) -> Hr;
// ObjectID, EventID, GroupID, Flags, dwData0..4
type FnTransmitEx1 = unsafe extern "system" fn(Handle, u32, u32, u32, u32, u32, u32, u32, u32, u32) -> Hr;
// DefineID, DatumName, UnitsName, DatumType, fEpsilon, DatumID
type FnAddDef = unsafe extern "system" fn(Handle, u32, *const c_char, *const c_char, u32, f32, u32) -> Hr;
type FnClearDef = unsafe extern "system" fn(Handle, u32) -> Hr;
// RequestID, DefineID, ObjectID, Period, Flags, origin, interval, limit
type FnRequest = unsafe extern "system" fn(Handle, u32, u32, u32, u32, u32, u32, u32, u32) -> Hr;
type FnSubscribe = unsafe extern "system" fn(Handle, u32, *const c_char) -> Hr;
type FnNext = unsafe extern "system" fn(Handle, *mut *mut u8, *mut u32) -> Hr;
type FnLastId = unsafe extern "system" fn(Handle, *mut u32) -> Hr;

const E_FAIL: Hr = 0x8000_4005_u32 as i32;
const OBJECT_ID_USER: u32 = 0;
const GROUP_PRIORITY_HIGHEST: u32 = 1;
const EVENT_FLAG_GROUPID_IS_PRIORITY: u32 = 0x10;
const DATATYPE_FLOAT64: u32 = 4;
const UNUSED: u32 = 0xFFFF_FFFF;
const PERIOD_NEVER: u32 = 0;
const PERIOD_SIM_FRAME: u32 = 3;
const PERIOD_SECOND: u32 = 4;
const DATA_REQUEST_FLAG_CHANGED: u32 = 1;
/// Smallest change worth a message (frequencies move by 0.005 MHz, so well under that).
const EPSILON: f32 = 0.0005;

/// The sim executables: MSFS 2024's first, so its SDK is preferred while it runs.
const SIMS: &[(&str, &str)] = &[("flightsimulator2024.exe", "MSFS2024_SDK"), ("flightsimulator.exe", "MSFS_SDK")];
const SDK_DEFAULTS: &[(&str, &str)] = &[("MSFS_SDK", "C:\\MSFS SDK"), ("MSFS2024_SDK", "C:\\MSFS 2024 SDK")];

#[derive(Clone, Copy)]
struct Api {
    open: FnOpen,
    close: FnClose,
    map: FnMap,
    transmit: FnTransmit,
    transmit_ex1: Option<FnTransmitEx1>,
    add_def: FnAddDef,
    clear_def: FnClearDef,
    request: FnRequest,
    subscribe: FnSubscribe,
    next: FnNext,
    last_id: FnLastId,
}

/// Loads the DLL on the first connect and keeps it loaded for the life of the process.
#[derive(Default)]
pub struct Dll {
    api: Option<Api>,
}

/// The running sim's executable path, and the SDK variable that matches it.
fn running_sim() -> Option<(PathBuf, &'static str)> {
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = None;
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase();
            if let Some((_, sdk)) = SIMS.iter().find(|(exe, _)| *exe == name) {
                found = Some((entry.th32ProcessID, *sdk));
                break;
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
        let (pid, sdk) = found?;
        let mut path = PathBuf::new();
        if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
                path = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
            }
            let _ = CloseHandle(h);
        }
        Some((path, sdk))
    }
}

fn reg_string(root: HKEY, key: PCWSTR, value: &str) -> Option<String> {
    let name = HSTRING::from(value);
    let mut buf = [0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(root, key, &name, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
    if r.is_err() {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]);
    (!s.trim().is_empty()).then(|| s.trim().to_string())
}

/// An SDK root from the environment, else from the user's / the machine's saved environment.
fn sdk_root(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| reg_string(HKEY_CURRENT_USER, w!("Environment"), var))
        .or_else(|| reg_string(HKEY_LOCAL_MACHINE, w!("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment"), var))
}

/// Where to look for SimConnect.dll, in order (see the module docs).
fn candidates() -> Vec<PathBuf> {
    let mut out = vec![];
    let dll = |dir: &Path| dir.join("SimConnect.dll");
    if let Ok(p) = std::env::var("SIMCONNECT_DLL") {
        let p = PathBuf::from(p.trim().trim_matches('"'));
        out.push(if p.is_dir() { dll(&p) } else { p });
    }
    out.push(dll(&knobdeck_sdk::data_dir()));
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        out.push(dll(&dir));
    }
    let running = running_sim();
    if let Some(dir) = running.as_ref().and_then(|(p, _)| p.parent()) {
        out.push(dll(dir));
    }
    let mut sdks: Vec<&str> = SDK_DEFAULTS.iter().map(|(v, _)| *v).collect();
    if let Some((_, first)) = running {
        sdks.sort_by_key(|v| *v != first);
    }
    for var in sdks {
        if let Some(root) = sdk_root(var) {
            out.push(Path::new(&root).join("SimConnect SDK").join("lib").join("SimConnect.dll"));
        }
        if let Some((_, default)) = SDK_DEFAULTS.iter().find(|(v, _)| *v == var) {
            out.push(Path::new(default).join("SimConnect SDK").join("lib").join("SimConnect.dll"));
        }
    }
    out
}

fn missing() -> String {
    format!(
        "SimConnect.dll not found. Install the MSFS SDK (in the sim: turn on Developer Mode, then Help ▸ SDK Installer), or copy SimConnect.dll into {}",
        knobdeck_sdk::data_dir().display()
    )
}

fn load() -> Result<Api, String> {
    let module: HMODULE = candidates()
        .into_iter()
        .filter(|p| p.is_file())
        .find_map(|p| unsafe { LoadLibraryExW(&HSTRING::from(p.as_os_str()), None, LOAD_WITH_ALTERED_SEARCH_PATH) }.ok())
        .ok_or_else(missing)?;
    let sym = |name: PCSTR| unsafe { GetProcAddress(module, name) };
    let req = |name: PCSTR| sym(name).ok_or_else(|| "This SimConnect.dll is missing functions KnobDeck needs; use the one from the MSFS SDK".to_string());
    // SAFETY: each symbol has the signature declared in SimConnect.h (__stdcall, which is the
    // platform convention on x64).
    unsafe {
        Ok(Api {
            open: std::mem::transmute::<_, FnOpen>(req(s!("SimConnect_Open"))?),
            close: std::mem::transmute::<_, FnClose>(req(s!("SimConnect_Close"))?),
            map: std::mem::transmute::<_, FnMap>(req(s!("SimConnect_MapClientEventToSimEvent"))?),
            transmit: std::mem::transmute::<_, FnTransmit>(req(s!("SimConnect_TransmitClientEvent"))?),
            transmit_ex1: sym(s!("SimConnect_TransmitClientEvent_EX1")).map(|f| std::mem::transmute::<_, FnTransmitEx1>(f)),
            add_def: std::mem::transmute::<_, FnAddDef>(req(s!("SimConnect_AddToDataDefinition"))?),
            clear_def: std::mem::transmute::<_, FnClearDef>(req(s!("SimConnect_ClearDataDefinition"))?),
            request: std::mem::transmute::<_, FnRequest>(req(s!("SimConnect_RequestDataOnSimObject"))?),
            subscribe: std::mem::transmute::<_, FnSubscribe>(req(s!("SimConnect_SubscribeToSystemEvent"))?),
            next: std::mem::transmute::<_, FnNext>(req(s!("SimConnect_GetNextDispatch"))?),
            last_id: std::mem::transmute::<_, FnLastId>(req(s!("SimConnect_GetLastSentPacketID"))?),
        })
    }
}

impl Connector for Dll {
    fn open(&mut self) -> Result<Box<dyn Link>, OpenError> {
        if self.api.is_none() {
            match load() {
                Ok(api) => self.api = Some(api),
                // With no sim running, that is the first thing to fix.
                Err(_) if running_sim().is_none() => return Err(OpenError::NotRunning),
                Err(e) => return Err(OpenError::Unavailable(e)),
            }
        }
        let api = self.api.ok_or(OpenError::NotRunning)?;
        let mut handle: Handle = std::ptr::null_mut();
        let name = CString::new("KnobDeck").unwrap_or_default();
        let hr = unsafe { (api.open)(&mut handle, name.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), 0) };
        if hr < 0 || handle.is_null() {
            return Err(OpenError::NotRunning);
        }
        Ok(Box::new(Conn { api, handle }))
    }
}

struct Conn {
    api: Api,
    handle: Handle,
}

// The handle is only used under the engine's lock.
unsafe impl Send for Conn {}

impl Drop for Conn {
    fn drop(&mut self) {
        unsafe { (self.api.close)(self.handle) };
    }
}

fn cstr(s: &str) -> Result<CString, LinkError> {
    CString::new(s).map_err(|_| LinkError::Failed(format!("\"{s}\" isn't a valid name")))
}

/// S_OK is fine; E_FAIL is a failed call on a working connection; anything else (a broken pipe,
/// a dead handle) means the connection is gone.
fn check(hr: Hr, what: &str) -> Result<(), LinkError> {
    match hr {
        h if h >= 0 => Ok(()),
        E_FAIL => Err(LinkError::Failed(format!("Flight Simulator refused {what}"))),
        _ => Err(LinkError::Lost),
    }
}

impl Conn {
    fn last_send(&self) -> u32 {
        let mut id = 0u32;
        unsafe { (self.api.last_id)(self.handle, &mut id) };
        id
    }
}

impl Link for Conn {
    fn map_event(&mut self, id: u32, name: &str) -> Result<u32, LinkError> {
        let n = cstr(name)?;
        check(unsafe { (self.api.map)(self.handle, id, n.as_ptr()) }, &format!("the event {name}"))?;
        Ok(self.last_send())
    }

    fn transmit(&mut self, id: u32, d: [u32; 5]) -> Result<(), LinkError> {
        let (h, obj, grp, flags) = (self.handle, OBJECT_ID_USER, GROUP_PRIORITY_HIGHEST, EVENT_FLAG_GROUPID_IS_PRIORITY);
        let hr = match self.api.transmit_ex1 {
            Some(ex1) => unsafe { ex1(h, obj, id, grp, flags, d[0], d[1], d[2], d[3], d[4]) },
            None => unsafe { (self.api.transmit)(h, obj, id, d[0], grp, flags) },
        };
        check(hr, "the event")
    }

    fn add_var(&mut self, def: u32, name: &str, unit: &str) -> Result<u32, LinkError> {
        let (n, u) = (cstr(name)?, cstr(unit)?);
        check(unsafe { (self.api.add_def)(self.handle, def, n.as_ptr(), u.as_ptr(), DATATYPE_FLOAT64, EPSILON, UNUSED) }, name)?;
        Ok(self.last_send())
    }

    fn request(&mut self, def: u32, rate: Option<Rate>) -> Result<(), LinkError> {
        let (period, flags) = match rate {
            None => (PERIOD_NEVER, 0),
            Some(Rate::Frame) => (PERIOD_SIM_FRAME, DATA_REQUEST_FLAG_CHANGED),
            Some(Rate::Second) => (PERIOD_SECOND, 0),
        };
        check(unsafe { (self.api.request)(self.handle, def, def, OBJECT_ID_USER, period, flags, 0, 0, 0) }, "a SimVar request")
    }

    fn clear_var(&mut self, def: u32) -> Result<(), LinkError> {
        check(unsafe { (self.api.clear_def)(self.handle, def) }, "a SimVar")
    }

    fn subscribe(&mut self, id: u32, name: &str) -> Result<(), LinkError> {
        let n = cstr(name)?;
        check(unsafe { (self.api.subscribe)(self.handle, id, n.as_ptr()) }, name)
    }

    fn next(&mut self) -> Result<Option<Recv>, LinkError> {
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut size: u32 = 0;
        let hr = unsafe { (self.api.next)(self.handle, &mut data, &mut size) };
        if hr == E_FAIL {
            return Ok(None);
        }
        check(hr, "a message")?;
        if data.is_null() || size < 12 {
            return Ok(None);
        }
        // SAFETY: SimConnect owns the buffer until the next GetNextDispatch call; it is copied out
        // of before then.
        let bytes = unsafe { std::slice::from_raw_parts(data, size as usize) };
        Ok(Some(parse_recv(bytes)))
    }
}
