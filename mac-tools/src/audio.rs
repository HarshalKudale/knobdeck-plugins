//! CoreAudio: list / switch the default output and input device, mute the
//! microphone and set its input level, and a device's volume and mute (an output's volume through
//! AudioToolbox's virtual main volume, the one the menu bar's volume slider sets). (Per-app volume, which Win Tools offers, needs an audio driver
//! on macOS, so it is not part of Mac Tools.)

use knobdeck_sdk::{Error, Result};
use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::c_void;

const fn fourcc(s: &[u8; 4]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | s[3] as u32
}

const SYSTEM_OBJECT: u32 = 1;
const DEVICES: u32 = fourcc(b"dev#");
const DEFAULT_OUTPUT: u32 = fourcc(b"dOut");
const DEFAULT_INPUT: u32 = fourcc(b"dIn ");
const DEFAULT_SYSTEM_OUTPUT: u32 = fourcc(b"sOut");
const NAME: u32 = fourcc(b"lnam");
const UID: u32 = fourcc(b"uid ");
/// `kAudioHardwareServiceDeviceProperty_VirtualMainVolume` (AudioToolbox).
const VIRTUAL_MAIN_VOLUME: u32 = fourcc(b"vmvc");
const STREAMS: u32 = fourcc(b"stm#");
const MUTE: u32 = fourcc(b"mute");
const VOLUME: u32 = fourcc(b"volm");
const SCOPE_GLOBAL: u32 = fourcc(b"glob");
const SCOPE_INPUT: u32 = fourcc(b"inpt");
const SCOPE_OUTPUT: u32 = fourcc(b"outp");
const ELEMENT_MAIN: u32 = 0;

#[repr(C)]
struct Address {
    selector: u32,
    scope: u32,
    element: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioObjectHasProperty(obj: u32, addr: *const Address) -> u8;
    fn AudioObjectIsPropertySettable(obj: u32, addr: *const Address, settable: *mut u8) -> i32;
    fn AudioObjectGetPropertyDataSize(obj: u32, addr: *const Address, qsize: u32, qdata: *const c_void, size: *mut u32) -> i32;
    fn AudioObjectGetPropertyData(obj: u32, addr: *const Address, qsize: u32, qdata: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    fn AudioObjectSetPropertyData(obj: u32, addr: *const Address, qsize: u32, qdata: *const c_void, size: u32, data: *const c_void) -> i32;
}

#[link(name = "AudioToolbox", kind = "framework")]
extern "C" {
    fn AudioHardwareServiceHasProperty(obj: u32, addr: *const Address) -> u8;
    fn AudioHardwareServiceIsPropertySettable(obj: u32, addr: *const Address, settable: *mut u8) -> i32;
    fn AudioHardwareServiceGetPropertyData(obj: u32, addr: *const Address, qsize: u32, qdata: *const c_void, size: *mut u32, data: *mut c_void) -> i32;
    fn AudioHardwareServiceSetPropertyData(obj: u32, addr: *const Address, qsize: u32, qdata: *const c_void, size: u32, data: *const c_void) -> i32;
}

fn addr(selector: u32, scope: u32, element: u32) -> Address {
    Address { selector, scope, element }
}

fn get<T: Copy + Default>(obj: u32, a: &Address) -> Option<T> {
    let mut v = T::default();
    let mut size = std::mem::size_of::<T>() as u32;
    let st = unsafe { AudioObjectGetPropertyData(obj, a, 0, std::ptr::null(), &mut size, &mut v as *mut T as *mut c_void) };
    (st == 0).then_some(v)
}

fn set<T: Copy>(obj: u32, a: &Address, v: T) -> Result {
    let st = unsafe { AudioObjectSetPropertyData(obj, a, 0, std::ptr::null(), std::mem::size_of::<T>() as u32, &v as *const T as *const c_void) };
    if st == 0 {
        Ok(())
    } else {
        Err(Error::msg(format!("CoreAudio refused the change (OSStatus {st})")))
    }
}

fn has(obj: u32, a: &Address) -> bool {
    unsafe { AudioObjectHasProperty(obj, a) != 0 }
}

fn settable(obj: u32, a: &Address) -> bool {
    let mut s = 0u8;
    has(obj, a) && unsafe { AudioObjectIsPropertySettable(obj, a, &mut s) } == 0 && s != 0
}

fn scope(input: bool) -> u32 {
    if input {
        SCOPE_INPUT
    } else {
        SCOPE_OUTPUT
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub id: u32,
    pub name: String,
}

fn name(id: u32) -> String {
    let a = addr(NAME, SCOPE_GLOBAL, ELEMENT_MAIN);
    let mut s: CFStringRef = std::ptr::null();
    let mut size = std::mem::size_of::<CFStringRef>() as u32;
    let st = unsafe { AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut size, &mut s as *mut _ as *mut c_void) };
    if st != 0 || s.is_null() {
        return format!("Device {id}");
    }
    unsafe { CFString::wrap_under_create_rule(s) }.to_string()
}

fn has_streams(id: u32, input: bool) -> bool {
    let a = addr(STREAMS, scope(input), ELEMENT_MAIN);
    let mut size = 0u32;
    unsafe { AudioObjectGetPropertyDataSize(id, &a, 0, std::ptr::null(), &mut size) == 0 && size > 0 }
}

/// Devices that can play (`input = false`) or record (`input = true`).
pub fn devices(input: bool) -> Vec<Device> {
    let a = addr(DEVICES, SCOPE_GLOBAL, ELEMENT_MAIN);
    let mut size = 0u32;
    if unsafe { AudioObjectGetPropertyDataSize(SYSTEM_OBJECT, &a, 0, std::ptr::null(), &mut size) } != 0 {
        return vec![];
    }
    let mut ids = vec![0u32; size as usize / 4];
    if unsafe { AudioObjectGetPropertyData(SYSTEM_OBJECT, &a, 0, std::ptr::null(), &mut size, ids.as_mut_ptr() as *mut c_void) } != 0 {
        return vec![];
    }
    ids.truncate(size as usize / 4);
    ids.into_iter().filter(|&id| has_streams(id, input)).map(|id| Device { id, name: name(id) }).collect()
}

pub fn default_device(input: bool) -> Option<u32> {
    get::<u32>(SYSTEM_OBJECT, &addr(if input { DEFAULT_INPUT } else { DEFAULT_OUTPUT }, SCOPE_GLOBAL, ELEMENT_MAIN)).filter(|&id| id != 0)
}

/// Make `id` the default device; outputs also become the alert/system-sound device.
pub fn set_default(input: bool, id: u32) -> Result {
    if input {
        return set(SYSTEM_OBJECT, &addr(DEFAULT_INPUT, SCOPE_GLOBAL, ELEMENT_MAIN), id);
    }
    set(SYSTEM_OBJECT, &addr(DEFAULT_OUTPUT, SCOPE_GLOBAL, ELEMENT_MAIN), id)?;
    let _ = set(SYSTEM_OBJECT, &addr(DEFAULT_SYSTEM_OUTPUT, SCOPE_GLOBAL, ELEMENT_MAIN), id);
    Ok(())
}

// ── Microphone mute and input level ──
// Only the device's own controls are used: a mic without a mute control (many USB mics) can't be
// muted here, rather than being faked by setting its level to 0.

/// Input elements that carry the control: the main element, else channels 1 and 2.
fn control_elements(id: u32, selector: u32) -> Vec<u32> {
    elements(id, selector, SCOPE_INPUT)
}

/// Elements of `scope` that carry the control: the main element, else channels 1 and 2.
fn elements(id: u32, selector: u32, sc: u32) -> Vec<u32> {
    if settable(id, &addr(selector, sc, ELEMENT_MAIN)) {
        return vec![ELEMENT_MAIN];
    }
    (1..=2).filter(|&ch| settable(id, &addr(selector, sc, ch))).collect()
}

fn mic() -> Result<u32> {
    default_device(true).ok_or_else(|| Error::msg("No microphone is selected"))
}

/// Whether the default microphone's mute control is on (`None`: no mic, or it has no mute control).
pub fn mic_muted() -> Option<bool> {
    let id = default_device(true)?;
    let el = *control_elements(id, MUTE).first()?;
    get::<u32>(id, &addr(MUTE, SCOPE_INPUT, el)).map(|m| m != 0)
}

/// Flip the default microphone's mute control; returns the new muted state.
pub fn toggle_mic_mute() -> Result<bool> {
    let id = mic()?;
    let mutes = control_elements(id, MUTE);
    if mutes.is_empty() {
        return Err(Error::msg("This microphone has no mute control macOS can switch"));
    }
    let target = !mic_muted().unwrap_or(false);
    for el in mutes {
        set(id, &addr(MUTE, SCOPE_INPUT, el), target as u32)?;
    }
    Ok(target)
}

/// The default microphone's input level in percent (`None`: no mic, or no level control).
pub fn mic_level() -> Option<i64> {
    let id = default_device(true)?;
    let el = *control_elements(id, VOLUME).first()?;
    get::<f32>(id, &addr(VOLUME, SCOPE_INPUT, el)).map(|v| (v as f64 * 100.0).round() as i64)
}

/// Set the default microphone's input level (0–100); returns the level set.
pub fn set_mic_level(percent: f64) -> Result<i64> {
    let id = mic()?;
    let els = control_elements(id, VOLUME);
    if els.is_empty() {
        return Err(Error::msg("This microphone's input level can't be changed from macOS"));
    }
    let p = percent.round().clamp(0.0, 100.0);
    for el in els {
        set(id, &addr(VOLUME, SCOPE_INPUT, el), (p / 100.0) as f32)?;
    }
    Ok(p as i64)
}

// ── Any device's volume and mute (Device Volume, Mute Device, Push to Talk) ──
// Devices are picked by their UID, which stays the same across restarts and reconnects (the
// numeric AudioObjectID doesn't).

/// The device setting's value for "whatever the default device is".
pub const DEFAULT_DEVICE: &str = "__default__";

/// A device's persistent UID (`kAudioDevicePropertyDeviceUID`).
pub fn uid(id: u32) -> Option<String> {
    let a = addr(UID, SCOPE_GLOBAL, ELEMENT_MAIN);
    let mut s: CFStringRef = std::ptr::null();
    let mut size = std::mem::size_of::<CFStringRef>() as u32;
    let st = unsafe { AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut size, &mut s as *mut _ as *mut c_void) };
    (st == 0 && !s.is_null()).then(|| unsafe { CFString::wrap_under_create_rule(s) }.to_string())
}

/// The device a setting names: its UID, or the default device.
fn resolve(input: bool, key: &str) -> Result<u32> {
    if key.is_empty() || key == DEFAULT_DEVICE {
        return default_device(input).ok_or_else(|| Error::msg(if input { "No microphone is selected" } else { "No output device is selected" }));
    }
    devices(input).into_iter().find(|d| uid(d.id).as_deref() == Some(key)).map(|d| d.id).ok_or_else(|| Error::msg("That device isn't connected"))
}

/// Whether the device named by `key` is the default one.
pub fn is_default(input: bool, key: &str) -> bool {
    !key.is_empty() && default_device(input).and_then(uid).as_deref() == Some(key)
}

/// Make the device named by `key` (a UID) the default.
pub fn set_default_uid(input: bool, key: &str) -> Result {
    set_default(input, resolve(input, key)?)
}

pub fn device_muted(input: bool, key: &str) -> Result<bool> {
    let id = resolve(input, key)?;
    let el = *elements(id, MUTE, scope(input)).first().ok_or_else(|| Error::msg("This device has no mute control macOS can switch"))?;
    Ok(get::<u32>(id, &addr(MUTE, scope(input), el)).is_some_and(|m| m != 0))
}

pub fn set_device_mute(input: bool, key: &str, muted: bool) -> Result {
    let id = resolve(input, key)?;
    let els = elements(id, MUTE, scope(input));
    if els.is_empty() {
        return Err(Error::msg("This device has no mute control macOS can switch"));
    }
    for el in els {
        set(id, &addr(MUTE, scope(input), el), muted as u32)?;
    }
    Ok(())
}

/// Flip the device's mute; returns the new muted state.
pub fn toggle_device_mute(input: bool, key: &str) -> Result<bool> {
    let target = !device_muted(input, key)?;
    set_device_mute(input, key, target)?;
    Ok(target)
}

/// An output's virtual main volume (all channels at once, keeping their balance), if it has one.
fn virtual_main(id: u32) -> Option<Address> {
    let a = addr(VIRTUAL_MAIN_VOLUME, SCOPE_OUTPUT, ELEMENT_MAIN);
    let mut s = 0u8;
    (unsafe { AudioHardwareServiceHasProperty(id, &a) } != 0 && unsafe { AudioHardwareServiceIsPropertySettable(id, &a, &mut s) } == 0 && s != 0).then_some(a)
}

/// The device's volume in percent.
pub fn device_volume(input: bool, key: &str) -> Result<i64> {
    let id = resolve(input, key)?;
    let v = if let Some(a) = (!input).then(|| virtual_main(id)).flatten() {
        let (mut v, mut size) = (0f32, 4u32);
        let st = unsafe { AudioHardwareServiceGetPropertyData(id, &a, 0, std::ptr::null(), &mut size, &mut v as *mut f32 as *mut c_void) };
        (st == 0).then_some(v)
    } else {
        elements(id, VOLUME, scope(input)).first().and_then(|&el| get::<f32>(id, &addr(VOLUME, scope(input), el)))
    };
    v.map(|v| (v as f64 * 100.0).round() as i64).ok_or_else(|| Error::msg("This device's volume can't be changed from macOS"))
}

/// Set the device's volume (0–100); returns the volume set.
pub fn set_device_volume(input: bool, key: &str, percent: f64) -> Result<i64> {
    let id = resolve(input, key)?;
    let p = percent.round().clamp(0.0, 100.0);
    let v = (p / 100.0) as f32;
    if let Some(a) = (!input).then(|| virtual_main(id)).flatten() {
        let st = unsafe { AudioHardwareServiceSetPropertyData(id, &a, 0, std::ptr::null(), 4, &v as *const f32 as *const c_void) };
        if st != 0 {
            return Err(Error::msg(format!("CoreAudio refused the change (OSStatus {st})")));
        }
        return Ok(p as i64);
    }
    let els = elements(id, VOLUME, scope(input));
    if els.is_empty() {
        return Err(Error::msg("This device's volume can't be changed from macOS"));
    }
    for el in els {
        set(id, &addr(VOLUME, scope(input), el), v)?;
    }
    Ok(p as i64)
}

/// Nudge the device's volume by `delta` percent; returns the volume set.
pub fn adjust_device_volume(input: bool, key: &str, delta: f64) -> Result<i64> {
    let now = device_volume(input, key)? as f64;
    set_device_volume(input, key, now + delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_outputs_with_names() {
        let outs = devices(false);
        eprintln!("outputs: {:?}\ninputs: {:?}\nmic muted: {:?}", outs, devices(true), mic_muted());
        eprintln!("default output volume: {:?}, muted: {:?}", device_volume(false, DEFAULT_DEVICE), device_muted(false, DEFAULT_DEVICE));
        if let Some(def) = default_device(false) {
            assert!(outs.iter().any(|d| d.id == def), "default output is listed");
            let u = uid(def).expect("every device has a UID");
            assert!(is_default(false, &u) && resolve(false, &u).ok() == Some(def));
        }
    }
}
