//! Display brightness, per display.
//!
//! - **Built-in display** (and Apple displays that macOS dims itself): `DisplayServicesGetBrightness`
//!   / `DisplayServicesSetBrightness` from the private DisplayServices framework (what the
//!   brightness keys and Control Center use), resolved with `dlsym` at call time.
//! - **External monitors on Apple Silicon**: DDC/CI (VCP 0x10) through the monitor's
//!   `DCPAVServiceProxy` and IOKit's `IOAVServiceWriteI2C` / `IOAVServiceReadI2C`, as
//!   MonitorControl and Lunar do. Each proxy is paired with the framebuffer listed before it in the
//!   registry, whose product attributes are matched to the CoreGraphics display.
//!   Intel Macs get a clear error for external monitors.

use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use knobdeck_sdk::{Error, Result};
use objc2::rc::autoreleasepool;
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSNumber, NSString};
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// "builtin", or "<vendor>-<model>-<serial>" in hex (stable across reconnects).
    pub id: String,
    pub name: String,
    pub internal: bool,
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGDisplayIsBuiltin(display: u32) -> u32;
    fn CGDisplayVendorNumber(display: u32) -> u32;
    fn CGDisplayModelNumber(display: u32) -> u32;
    fn CGDisplaySerialNumber(display: u32) -> u32;
}

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IORegistryGetRootEntry(main_port: u32) -> u32;
    fn IORegistryEntryCreateIterator(entry: u32, plane: *const c_char, options: u32, iter: *mut u32) -> i32;
    fn IOIteratorNext(iter: u32) -> u32;
    fn IOObjectRelease(obj: u32) -> i32;
    fn IORegistryEntryGetName(entry: u32, name: *mut c_char) -> i32;
    fn IORegistryEntryCreateCFProperty(entry: u32, key: CFStringRef, alloc: *const c_void, options: u32) -> *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const c_void);
}

fn sym(lib: &CStr, name: &CStr) -> Option<*mut c_void> {
    let h = unsafe { libc::dlopen(lib.as_ptr(), libc::RTLD_LAZY) };
    if h.is_null() {
        return None;
    }
    let p = unsafe { libc::dlsym(h, name.as_ptr()) };
    (!p.is_null()).then_some(p)
}

const DISPLAY_SERVICES: &CStr = c"/System/Library/PrivateFrameworks/DisplayServices.framework/DisplayServices";
const IOKIT: &CStr = c"/System/Library/Frameworks/IOKit.framework/IOKit";

struct Display {
    monitor: Monitor,
    cg: u32,
    vendor: u32,
    model: u32,
    serial: u32,
}

/// display id → name, from NSScreen (see screens.rs on reading NSScreen off the main thread).
fn screen_names() -> HashMap<u32, String> {
    // SAFETY: read-only NSScreen access; plugin handlers have no main run loop to hop to.
    let mtm = unsafe { MainThreadMarker::new_unchecked() };
    autoreleasepool(|_| {
        let key = NSString::from_str("NSScreenNumber");
        NSScreen::screens(mtm)
            .iter()
            .filter_map(|s| {
                let id = s.deviceDescription().objectForKey(&key)?.downcast_ref::<NSNumber>()?.unsignedIntValue();
                Some((id, s.localizedName().to_string()))
            })
            .collect()
    })
}

fn displays() -> Vec<Display> {
    let mut ids = [0u32; 16];
    let mut n = 0u32;
    if unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut n) } != 0 {
        return vec![];
    }
    let names = screen_names();
    let mut seen: HashMap<String, usize> = HashMap::new();
    ids[..n as usize]
        .iter()
        .map(|&cg| {
            let internal = unsafe { CGDisplayIsBuiltin(cg) } != 0;
            let (vendor, model, serial) = unsafe { (CGDisplayVendorNumber(cg), CGDisplayModelNumber(cg), CGDisplaySerialNumber(cg)) };
            let mut id = if internal { "builtin".to_string() } else { format!("{vendor:x}-{model:x}-{serial:x}") };
            let c = seen.entry(id.clone()).or_insert(0);
            *c += 1;
            if *c > 1 {
                id = format!("{id}-{c}"); // two identical monitors without serial numbers
            }
            let name = names.get(&cg).cloned().unwrap_or_else(|| if internal { "Built-in Display".into() } else { "Display".into() });
            Display { monitor: Monitor { id, name, internal }, cg, vendor, model, serial }
        })
        .collect()
}

pub fn monitors() -> Result<Vec<Monitor>> {
    Ok(displays().into_iter().map(|d| d.monitor).collect())
}

fn find(id: &str) -> Result<Display> {
    displays().into_iter().find(|d| d.monitor.id == id).ok_or_else(|| Error::msg("That display isn't connected"))
}

pub fn get(id: &str) -> Result<f64> {
    let d = find(id)?;
    if let Some(v) = native_get(d.cg) {
        return Ok(v as f64 * 100.0);
    }
    let (cur, max) = ddc::cached_get(&d)?;
    Ok(if max > 0 { cur as f64 * 100.0 / max as f64 } else { 0.0 })
}

pub fn set(id: &str, percent: f64) -> Result<()> {
    let p = percent.clamp(0.0, 100.0);
    let d = find(id)?;
    if native_get(d.cg).is_some() {
        return native_set(d.cg, (p / 100.0) as f32);
    }
    let (_, max) = ddc::cached_get(&d)?;
    ddc::set(&d, (p / 100.0 * max as f64).round() as u16)
}

/// The brightness macOS itself controls (built-in panels, Studio Display / Pro Display XDR), 0–1.
fn native_get(cg: u32) -> Option<f32> {
    let f = sym(DISPLAY_SERVICES, c"DisplayServicesGetBrightness")?;
    let f: extern "C" fn(u32, *mut f32) -> i32 = unsafe { std::mem::transmute(f) };
    let mut v = 0f32;
    (f(cg, &mut v) == 0).then_some(v)
}

fn native_set(cg: u32, v: f32) -> Result<()> {
    let f = sym(DISPLAY_SERVICES, c"DisplayServicesSetBrightness").ok_or_else(|| Error::msg("This macOS no longer offers the brightness call the brightness keys use"))?;
    let f: extern "C" fn(u32, f32) -> i32 = unsafe { std::mem::transmute(f) };
    match f(cg, v.clamp(0.0, 1.0)) {
        0 => Ok(()),
        e => Err(Error::msg(format!("macOS refused the brightness change (error {e})"))),
    }
}

/// DDC/CI on Apple Silicon.
mod ddc {
    use super::*;

    const DDC_ADDR: u32 = 0x37;
    const DDC_DATA: u32 = 0x51;

    type CreateWithService = extern "C" fn(*const c_void, u32) -> *const c_void;
    type WriteI2C = extern "C" fn(*const c_void, u32, u32, *const c_void, u32) -> i32;
    type ReadI2C = extern "C" fn(*const c_void, u32, u32, *mut c_void, u32) -> i32;

    /// An `IOAVService` (a CF object, released on drop).
    struct Service(*const c_void);
    impl Drop for Service {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }

    fn name_of(entry: u32) -> String {
        let mut buf = [0 as c_char; 128];
        if unsafe { IORegistryEntryGetName(entry, buf.as_mut_ptr()) } != 0 {
            return String::new();
        }
        unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
    }

    fn property(entry: u32, key: &str) -> Option<CFType> {
        let k = CFString::new(key);
        let raw = unsafe { IORegistryEntryCreateCFProperty(entry, k.as_concrete_TypeRef(), std::ptr::null(), 0) };
        (!raw.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(raw) })
    }

    fn dict_num(d: &CFDictionary<CFString, CFType>, key: &str) -> Option<i64> {
        d.find(CFString::new(key)).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_i64())
    }

    /// (vendor, model, serial) of a framebuffer, from its `DisplayAttributes.ProductAttributes`.
    fn product(entry: u32) -> Option<(u32, u32, u32)> {
        let attrs = property(entry, "DisplayAttributes")?.downcast::<CFDictionary>()?;
        let attrs: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_get_rule(attrs.as_concrete_TypeRef()) };
        let prod = attrs.find(CFString::new("ProductAttributes"))?.downcast::<CFDictionary>()?;
        let prod: CFDictionary<CFString, CFType> = unsafe { CFDictionary::wrap_under_get_rule(prod.as_concrete_TypeRef()) };
        Some((dict_num(&prod, "LegacyManufacturerID")? as u32, dict_num(&prod, "ProductID")? as u32, dict_num(&prod, "SerialNumber").unwrap_or(0) as u32))
    }

    /// External AV service proxies with the product of the framebuffer before each, in registry order.
    fn services() -> Result<Vec<(Option<(u32, u32, u32)>, Service)>> {
        let create: CreateWithService = unsafe { std::mem::transmute(sym(IOKIT, c"IOAVServiceCreateWithService").ok_or_else(|| Error::msg("External monitor brightness needs a Mac with Apple silicon"))?) };
        let root = unsafe { IORegistryGetRootEntry(0) };
        let mut iter = 0u32;
        // kIORegistryIterateRecursively = 1.
        if unsafe { IORegistryEntryCreateIterator(root, c"IOService".as_ptr(), 1, &mut iter) } != 0 {
            return Err(Error::msg("Couldn't read the I/O Registry"));
        }
        let mut out = vec![];
        let mut last_fb: Option<(u32, u32, u32)> = None;
        loop {
            let e = unsafe { IOIteratorNext(iter) };
            if e == 0 {
                break;
            }
            let name = name_of(e);
            if name == "AppleCLCD2" || name == "IOMobileFramebufferShim" {
                last_fb = product(e);
            } else if name == "DCPAVServiceProxy" {
                let external = property(e, "Location").and_then(|v| v.downcast::<CFString>()).is_some_and(|s| s.to_string() == "External");
                if external {
                    let s = create(std::ptr::null(), e);
                    if !s.is_null() {
                        out.push((last_fb.take(), Service(s)));
                    }
                }
            }
            unsafe { IOObjectRelease(e) };
        }
        unsafe { IOObjectRelease(iter) };
        Ok(out)
    }

    /// The service for display `d`: matched by vendor + model (+ serial when both have one); with
    /// one external monitor, that one.
    fn service_for(d: &Display) -> Result<Service> {
        let mut list = services()?;
        if list.is_empty() {
            return Err(Error::msg(format!("{} can't be dimmed over DDC/CI (no connection found; HDMI on some Macs, docks and DisplayLink adapters don't pass it through)", d.monitor.name)));
        }
        let exact = list.iter().position(|(p, _)| p.is_some_and(|(v, m, s)| v == d.vendor && m == d.model && (s == 0 || d.serial == 0 || s == d.serial)));
        let externals = displays().into_iter().filter(|x| !x.monitor.internal && native_get(x.cg).is_none()).count();
        let i = exact.or((list.len() == 1 && externals == 1).then_some(0)).ok_or_else(|| Error::msg(format!("Couldn't tell which connection {} is on", d.monitor.name)))?;
        Ok(list.swap_remove(i).1)
    }

    fn fns() -> Result<(WriteI2C, ReadI2C)> {
        let w = sym(IOKIT, c"IOAVServiceWriteI2C").ok_or_else(|| Error::msg("External monitor brightness needs a Mac with Apple silicon"))?;
        let r = sym(IOKIT, c"IOAVServiceReadI2C").ok_or_else(|| Error::msg("External monitor brightness needs a Mac with Apple silicon"))?;
        Ok(unsafe { (std::mem::transmute::<*mut c_void, WriteI2C>(w), std::mem::transmute::<*mut c_void, ReadI2C>(r)) })
    }

    fn write(s: &Service, msg: &[u8]) -> Result<()> {
        let (w, _) = fns()?;
        // The 0x51 source byte goes in the "data address" argument; the rest is the payload.
        let body = &msg[1..];
        // Monitors often drop the first write after idling: send it twice, as MonitorControl does.
        let mut ok = false;
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(10));
            ok |= w(s.0, DDC_ADDR, DDC_DATA, body.as_ptr() as *const c_void, body.len() as u32) == 0;
        }
        if ok {
            Ok(())
        } else {
            Err(Error::msg("The monitor didn't accept the DDC/CI command (turn DDC/CI on in its menu)"))
        }
    }

    fn get(d: &Display) -> Result<(u16, u16)> {
        let s = service_for(d)?;
        let (_, r) = fns()?;
        for _ in 0..3 {
            write(&s, &packet::get_request_arm64(0x10))?;
            std::thread::sleep(Duration::from_millis(50));
            let mut buf = [0u8; 12];
            if r(s.0, DDC_ADDR, DDC_DATA, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) == 0 {
                if let Some(v) = packet::parse_reply(&buf, 0x10) {
                    return Ok(v);
                }
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        Err(Error::msg(format!("{} didn't report its brightness over DDC/CI (turn DDC/CI on in its menu)", d.monitor.name)))
    }

    static CACHE: Mutex<Option<HashMap<String, (Instant, u16, u16)>>> = Mutex::new(None);

    pub fn cached_get(d: &Display) -> Result<(u16, u16)> {
        if let Some((at, c, m)) = CACHE.lock().unwrap().as_ref().and_then(|m| m.get(&d.monitor.id)).copied() {
            if at.elapsed() < Duration::from_secs(3) {
                return Ok((c, m));
            }
        }
        let (c, m) = get(d)?;
        CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(d.monitor.id.clone(), (Instant::now(), c, m));
        Ok((c, m))
    }

    pub fn set(d: &Display, value: u16) -> Result<()> {
        let s = service_for(d)?;
        write(&s, &packet::set_request(0x10, value))?;
        if let Some(e) = CACHE.lock().unwrap().as_mut().and_then(|m| m.get_mut(&d.monitor.id)) {
            *e = (Instant::now(), value, e.2);
        }
        std::thread::sleep(Duration::from_millis(50));
        Ok(())
    }
}

/// DDC/CI (VESA MCCS) messages to the monitor's address 0x37, starting at the source byte 0x51
/// (the same code as linux-tools' brightness.rs).
pub mod packet {
    fn checksum(bytes: &[u8]) -> u8 {
        bytes.iter().fold(0x6E, |a, b| a ^ b)
    }

    #[cfg(test)] // the standard framing, kept to show what the Apple silicon one differs from
    pub fn get_request(code: u8) -> Vec<u8> {
        let mut m = vec![0x51, 0x82, 0x01, code];
        m.push(checksum(&m));
        m
    }

    /// The read request as MonitorControl and Lunar frame it for `IOAVServiceWriteI2C`: their
    /// checksum for a one-byte request leaves out the 0x51 source byte, and Apple silicon monitors
    /// answer that one.
    pub fn get_request_arm64(code: u8) -> Vec<u8> {
        let mut m = vec![0x51, 0x82, 0x01, code];
        m.push(checksum(&m[1..]));
        m
    }

    pub fn set_request(code: u8, value: u16) -> Vec<u8> {
        let mut m = vec![0x51, 0x84, 0x03, code, (value >> 8) as u8, value as u8];
        m.push(checksum(&m));
        m
    }

    /// `(current, max)` from a "VCP feature reply": `… 0x02 result code type maxH maxL curH curL chk`.
    pub fn parse_reply(buf: &[u8], code: u8) -> Option<(u16, u16)> {
        let i = (0..buf.len().saturating_sub(7)).find(|&i| buf[i] == 0x02 && buf[i + 2] == code)?;
        if buf[i + 1] != 0 {
            return None;
        }
        let max = u16::from_be_bytes([buf[i + 4], buf[i + 5]]);
        let cur = u16::from_be_bytes([buf[i + 6], buf[i + 7]]);
        (max > 0).then_some((cur, max))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ddc_packets() {
        assert_eq!(super::packet::get_request(0x10), vec![0x51, 0x82, 0x01, 0x10, 0xAC]);
        assert_eq!(super::packet::get_request_arm64(0x10), vec![0x51, 0x82, 0x01, 0x10, 0x6E ^ 0x82 ^ 0x01 ^ 0x10]);
        assert_eq!(super::packet::parse_reply(&[0x6E, 0x88, 0x02, 0x00, 0x10, 0x00, 0x00, 0x64, 0x00, 0x28, 0x00], 0x10), Some((40, 100)));
    }

    /// Reads (never sets) every display's brightness.
    #[test]
    #[ignore = "needs displays"]
    fn live_read() {
        for m in super::monitors().unwrap() {
            eprintln!("{m:?}: {:?}", super::get(&m.id));
        }
    }
}
