//! The Voicemeeter Remote API: `VoicemeeterRemote64.dll`, installed with every Voicemeeter
//! edition, loaded at run time from the install folder (a missing DLL is an error, never a crash
//! at launch). One login per process; all calls go through one lock.

use crate::layout::Edition;
use knobdeck_sdk::{Error, Result};
use std::ffi::{c_char, c_float, c_long, CString};
use std::sync::Mutex;
use windows::core::{s, w, HSTRING, PCSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};

type FnVoid = unsafe extern "system" fn() -> c_long;
type FnLongPtr = unsafe extern "system" fn(*mut c_long) -> c_long;
type FnRun = unsafe extern "system" fn(c_long) -> c_long;
type FnGetF = unsafe extern "system" fn(*const c_char, *mut c_float) -> c_long;
type FnGetS = unsafe extern "system" fn(*const c_char, *mut c_char) -> c_long;
type FnSetF = unsafe extern "system" fn(*const c_char, c_float) -> c_long;
type FnSetS = unsafe extern "system" fn(*const c_char, *const c_char) -> c_long;
type FnMacroGet = unsafe extern "system" fn(c_long, *mut c_float, c_long) -> c_long;
type FnMacroSet = unsafe extern "system" fn(c_long, c_float, c_long) -> c_long;

struct Api {
    login: FnVoid,
    run: FnRun,
    get_type: FnLongPtr,
    dirty: FnVoid,
    get_f: FnGetF,
    get_s: FnGetS,
    set_f: FnSetF,
    set_s: FnSetS,
    macro_get: FnMacroGet,
    macro_set: FnMacroSet,
}

// Plain function pointers into a DLL that stays loaded for the life of the process.
unsafe impl Send for Api {}

static API: Mutex<Option<Api>> = Mutex::new(None);

const NOT_INSTALLED: &str = "Voicemeeter isn't installed (VoicemeeterRemote64.dll not found). Get it from vb-audio.com.";
const NOT_RUNNING: &str = "Voicemeeter isn't running";

/// The install folder from Voicemeeter's uninstall entry, else the default one.
fn dll_paths() -> Vec<String> {
    let mut out = vec![];
    for key in [
        w!("SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\VB:Voicemeeter {17359A74-1236-5467}"),
        w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\VB:Voicemeeter {17359A74-1236-5467}"),
    ] {
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let ok = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key, w!("UninstallString"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
        if ok.is_ok() {
            let s = String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]);
            let exe = s.trim().trim_matches('"');
            if let Some(dir) = std::path::Path::new(exe).parent() {
                out.push(dir.join("VoicemeeterRemote64.dll").display().to_string());
            }
        }
    }
    for base in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(p) = std::env::var(base) {
            out.push(format!("{p}\\VB\\Voicemeeter\\VoicemeeterRemote64.dll"));
        }
    }
    out
}

fn load() -> Result<Api> {
    let module: HMODULE = dll_paths()
        .into_iter()
        .filter(|p| std::path::Path::new(p).exists())
        .find_map(|p| unsafe { LoadLibraryW(&HSTRING::from(p)) }.ok())
        .ok_or_else(|| Error::msg(NOT_INSTALLED))?;
    let sym = |name: PCSTR| unsafe { GetProcAddress(module, name) }.ok_or_else(|| Error::msg(NOT_INSTALLED));
    // SAFETY: each symbol has the signature in VoicemeeterRemote.h.
    unsafe {
        Ok(Api {
            login: std::mem::transmute(sym(s!("VBVMR_Login"))?),
            run: std::mem::transmute(sym(s!("VBVMR_RunVoicemeeter"))?),
            get_type: std::mem::transmute(sym(s!("VBVMR_GetVoicemeeterType"))?),
            dirty: std::mem::transmute(sym(s!("VBVMR_IsParametersDirty"))?),
            get_f: std::mem::transmute(sym(s!("VBVMR_GetParameterFloat"))?),
            get_s: std::mem::transmute(sym(s!("VBVMR_GetParameterStringA"))?),
            set_f: std::mem::transmute(sym(s!("VBVMR_SetParameterFloat"))?),
            set_s: std::mem::transmute(sym(s!("VBVMR_SetParameterStringA"))?),
            macro_get: std::mem::transmute(sym(s!("VBVMR_MacroButton_GetStatus"))?),
            macro_set: std::mem::transmute(sym(s!("VBVMR_MacroButton_SetStatus"))?),
        })
    }
}

/// Run `f` with the logged-in API.
fn with<T>(f: impl FnOnce(&Api) -> Result<T>) -> Result<T> {
    let mut guard = API.lock().unwrap();
    if guard.is_none() {
        let api = load()?;
        // 0: logged in; 1: logged in, Voicemeeter not running yet (calls fail until it starts).
        let r = unsafe { (api.login)() };
        if r < 0 {
            return Err(Error::msg(format!("Could not connect to Voicemeeter (error {r})")));
        }
        *guard = Some(api);
    }
    f(guard.as_ref().unwrap())
}

fn cstr(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| Error::msg("bad parameter name"))
}

/// The edition running; an error when Voicemeeter isn't.
pub fn edition() -> Result<Edition> {
    with(|a| {
        let mut t: c_long = 0;
        let r = unsafe { (a.get_type)(&mut t) };
        if r != 0 {
            return Err(Error::msg(NOT_RUNNING));
        }
        Edition::from_type(t).ok_or_else(|| Error::msg(format!("Unknown Voicemeeter type {t}")))
    })
}

/// Start Voicemeeter (the edition installed last when none is given).
pub fn launch(edition: Option<Edition>) -> Result<()> {
    with(|a| {
        let ty = match edition {
            Some(Edition::Standard) => 1,
            Some(Edition::Banana) => 2,
            // 6: the 64-bit Potato.
            Some(Edition::Potato) | None => 6,
        };
        let mut r = unsafe { (a.run)(ty) };
        if r != 0 && edition.is_none() {
            r = unsafe { (a.run)(2) };
            if r != 0 {
                r = unsafe { (a.run)(1) };
            }
        }
        if r != 0 {
            return Err(Error::msg(format!("Could not start Voicemeeter (error {r})")));
        }
        Ok(())
    })
}

/// Pull changes Voicemeeter made since the last call (the API caches values until asked).
fn refresh(a: &Api) {
    unsafe { (a.dirty)() };
}

pub fn get(name: &str) -> Result<f64> {
    let n = cstr(name)?;
    with(|a| {
        refresh(a);
        let mut v: c_float = 0.0;
        match unsafe { (a.get_f)(n.as_ptr(), &mut v) } {
            0 => Ok(v as f64),
            -1 | -2 => Err(Error::msg(NOT_RUNNING)),
            _ => Err(Error::msg(format!("Voicemeeter has no parameter {name}"))),
        }
    })
}

pub fn get_str(name: &str) -> Result<String> {
    let n = cstr(name)?;
    with(|a| {
        refresh(a);
        let mut buf = [0 as c_char; 512];
        match unsafe { (a.get_s)(n.as_ptr(), buf.as_mut_ptr()) } {
            0 => Ok(unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()),
            -1 | -2 => Err(Error::msg(NOT_RUNNING)),
            _ => Err(Error::msg(format!("Voicemeeter has no parameter {name}"))),
        }
    })
}

pub fn set(name: &str, value: f64) -> Result<()> {
    let n = cstr(name)?;
    with(|a| match unsafe { (a.set_f)(n.as_ptr(), value as c_float) } {
        0 => Ok(()),
        -1 | -2 => Err(Error::msg(NOT_RUNNING)),
        _ => Err(Error::msg(format!("Voicemeeter has no parameter {name}"))),
    })
}

pub fn set_str(name: &str, value: &str) -> Result<()> {
    let (n, v) = (cstr(name)?, cstr(value)?);
    with(|a| match unsafe { (a.set_s)(n.as_ptr(), v.as_ptr()) } {
        0 => Ok(()),
        -1 | -2 => Err(Error::msg(NOT_RUNNING)),
        _ => Err(Error::msg(format!("Voicemeeter has no parameter {name}"))),
    })
}

/// A Macro Buttons button's state (0-based, as numbered in the Macro Buttons app).
pub fn macro_get(button: i32) -> Result<bool> {
    with(|a| {
        let mut v: c_float = 0.0;
        match unsafe { (a.macro_get)(button, &mut v, 0) } {
            0 => Ok(v != 0.0),
            -1 | -2 => Err(Error::msg(NOT_RUNNING)),
            _ => Err(Error::msg(format!("No macro button {button}"))),
        }
    })
}

/// Set a macro button down (true) or up; its script for that change runs.
pub fn macro_set(button: i32, down: bool) -> Result<()> {
    with(|a| match unsafe { (a.macro_set)(button, if down { 1.0 } else { 0.0 }, 0) } {
        0 => Ok(()),
        -1 | -2 => Err(Error::msg(NOT_RUNNING)),
        _ => Err(Error::msg(format!("No macro button {button}"))),
    })
}
