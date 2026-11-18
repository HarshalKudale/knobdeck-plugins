//! macOS: Apple silicon temperatures through IOHIDEventSystemClient (private, unchanged since
//! the first M1; resolved with `dlsym` so its absence is an empty list, never a crash), and the
//! SMC through IOKit's public `IOConnectCallStructMethod` (fans and power on every Mac, the
//! temperatures of Intel Macs).

use crate::sensors::{Kind, Role, Scan, Sensor};
use core_foundation::base::TCFType;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use std::ffi::{c_char, c_void, CStr};
use std::sync::{Mutex, OnceLock};

type CFTypeRef = *const c_void;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingService(main_port: u32, matching: *mut c_void) -> u32;
    fn IOServiceOpen(service: u32, task: u32, kind: u32, conn: *mut u32) -> i32;
    fn IOObjectRelease(obj: u32) -> i32;
    fn IOConnectCallStructMethod(conn: u32, selector: u32, input: *const c_void, in_size: usize, output: *mut c_void, out_size: *mut usize) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFArrayGetCount(a: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(a: CFTypeRef, i: isize) -> CFTypeRef;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
}

extern "C" {
    fn mach_task_self() -> u32;
}

const IOKIT: &CStr = c"/System/Library/Frameworks/IOKit.framework/IOKit";

fn sym(name: &CStr) -> Option<*mut c_void> {
    let h = unsafe { libc::dlopen(IOKIT.as_ptr(), libc::RTLD_LAZY) };
    if h.is_null() {
        return None;
    }
    let p = unsafe { libc::dlsym(h, name.as_ptr()) };
    (!p.is_null()).then_some(p)
}

// ── Apple silicon: IOHIDEventSystemClient ──

struct Hid {
    client: CFTypeRef,
    copy_services: unsafe extern "C" fn(CFTypeRef) -> CFTypeRef,
    copy_property: unsafe extern "C" fn(CFTypeRef, CFStringRef) -> CFTypeRef,
    copy_event: unsafe extern "C" fn(CFTypeRef, i64, i32, i64) -> CFTypeRef,
    float_value: unsafe extern "C" fn(CFTypeRef, i32) -> f64,
}
// SAFETY: the client is created once and only read from; IOHID clients are thread-safe for these calls.
unsafe impl Send for Hid {}
unsafe impl Sync for Hid {}

const EVENT_TEMPERATURE: i64 = 15; // kIOHIDEventTypeTemperature

fn hid() -> Option<&'static Hid> {
    static HID: OnceLock<Option<Hid>> = OnceLock::new();
    HID.get_or_init(|| unsafe {
        let create = std::mem::transmute::<*mut c_void, unsafe extern "C" fn(CFTypeRef) -> CFTypeRef>(sym(c"IOHIDEventSystemClientCreate")?);
        let set_matching = std::mem::transmute::<*mut c_void, unsafe extern "C" fn(CFTypeRef, CFTypeRef) -> i32>(sym(c"IOHIDEventSystemClientSetMatching")?);
        type CopyServices = unsafe extern "C" fn(CFTypeRef) -> CFTypeRef;
        type CopyProperty = unsafe extern "C" fn(CFTypeRef, CFStringRef) -> CFTypeRef;
        type CopyEvent = unsafe extern "C" fn(CFTypeRef, i64, i32, i64) -> CFTypeRef;
        type FloatValue = unsafe extern "C" fn(CFTypeRef, i32) -> f64;
        let h = Hid {
            client: std::ptr::null(),
            copy_services: std::mem::transmute::<*mut c_void, CopyServices>(sym(c"IOHIDEventSystemClientCopyServices")?),
            copy_property: std::mem::transmute::<*mut c_void, CopyProperty>(sym(c"IOHIDServiceClientCopyProperty")?),
            copy_event: std::mem::transmute::<*mut c_void, CopyEvent>(sym(c"IOHIDServiceClientCopyEvent")?),
            float_value: std::mem::transmute::<*mut c_void, FloatValue>(sym(c"IOHIDEventGetFloatValue")?),
        };
        let client = create(std::ptr::null());
        if client.is_null() {
            return None;
        }
        // Apple's vendor page 0xff00, usage 5: the temperature sensors.
        let matching = CFDictionary::from_CFType_pairs(&[
            (CFString::new("PrimaryUsagePage").as_CFType(), CFNumber::from(0xff00i32).as_CFType()),
            (CFString::new("PrimaryUsage").as_CFType(), CFNumber::from(5i32).as_CFType()),
        ]);
        set_matching(client, matching.as_concrete_TypeRef() as CFTypeRef);
        Some(Hid { client, ..h })
    })
    .as_ref()
}

/// (sensor name, °C) for every Apple silicon temperature sensor.
fn hid_temps() -> Vec<(String, f64)> {
    let Some(h) = hid() else { return vec![] };
    let mut out = vec![];
    // SAFETY: Copy-rule CF objects are released; Get-rule values are borrowed from the array.
    unsafe {
        let services = (h.copy_services)(h.client);
        if services.is_null() {
            return out;
        }
        let key = CFString::new("Product");
        for i in 0..CFArrayGetCount(services) {
            let svc = CFArrayGetValueAtIndex(services, i);
            let name = (h.copy_property)(svc, key.as_concrete_TypeRef());
            if name.is_null() {
                continue;
            }
            let name_s = if CFGetTypeID(name) == CFStringGetTypeID() { Some(CFString::wrap_under_create_rule(name as CFStringRef).to_string()) } else {
                CFRelease(name);
                None
            };
            let Some(name) = name_s else { continue };
            let event = (h.copy_event)(svc, EVENT_TEMPERATURE, 0, 0);
            if event.is_null() {
                continue;
            }
            let v = (h.float_value)(event, (EVENT_TEMPERATURE as i32) << 16);
            CFRelease(event);
            if v > -40.0 && v < 150.0 {
                out.push((name, v));
            }
        }
        CFRelease(services);
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Group and role for an Apple silicon sensor name.
fn hid_group(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("tdie") || n.contains("acc mtr") || n.contains("pacc") || n.contains("eacc") {
        "CPU"
    } else if n.contains("gpu") {
        "GPU"
    } else if n.contains("nand") {
        "SSD"
    } else if n.contains("battery") || n.contains("gas gauge") {
        "Battery"
    } else {
        "SoC"
    }
}

fn hid_sensors(out: &mut Vec<Sensor>) {
    let temps = hid_temps();
    let mut seen: Vec<String> = vec![];
    for (name, v) in &temps {
        let mut id = format!("hid:{name}");
        let mut k = 2;
        while seen.contains(&id) {
            id = format!("hid:{name}#{k}");
            k += 1;
        }
        seen.push(id.clone());
        out.push(Sensor::new(id, hid_group(name), name, Kind::Temp, *v));
    }
    // The hottest CPU die / GPU sensor, as Stats and iStat show "CPU" and "GPU".
    let hottest = |pred: &dyn Fn(&str) -> bool| temps.iter().filter(|(n, _)| pred(&n.to_lowercase())).map(|(_, v)| *v).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
    let cpu = hottest(&|n| n.contains("tdie")).or_else(|| hottest(&|n| n.contains("acc mtr")));
    if let Some(v) = cpu {
        out.push(Sensor::new("hid:cpu-hottest", "CPU", "Hottest core", Kind::Temp, v).role(Role::CpuTemp, 0));
    }
    if let Some(v) = hottest(&|n| n.contains("gpu")) {
        out.push(Sensor::new("hid:gpu-hottest", "GPU", "Hottest sensor", Kind::Temp, v).role(Role::GpuTemp, 0));
    }
}

// ── SMC ──

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Vers {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct PLimit {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    mem: u32,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KeyInfo {
    size: u32,
    kind: u32,
    attributes: u8,
}
/// `SMCParamStruct`, the AppleSMC user client's in/out structure.
#[repr(C)]
#[derive(Default, Clone, Copy)]
struct Param {
    key: u32,
    vers: Vers,
    plimit: PLimit,
    info: KeyInfo,
    result: u8,
    status: u8,
    command: u8,
    data32: u32,
    bytes: [u8; 32],
}
const _: () = assert!(std::mem::size_of::<Param>() == 80);

const SMC_CALL: u32 = 2; // kSMCHandleYPCEvent
const READ_KEY: u8 = 5;
const KEY_INFO: u8 = 9;

struct Smc(u32);

fn smc() -> Option<&'static Mutex<Smc>> {
    static SMC: OnceLock<Option<Mutex<Smc>>> = OnceLock::new();
    SMC.get_or_init(|| unsafe {
        let svc = IOServiceGetMatchingService(0, IOServiceMatching(c"AppleSMC".as_ptr()));
        if svc == 0 {
            return None;
        }
        let mut conn = 0u32;
        let r = IOServiceOpen(svc, mach_task_self(), 0, &mut conn);
        IOObjectRelease(svc);
        (r == 0 && conn != 0).then(|| Mutex::new(Smc(conn)))
    })
    .as_ref()
}

fn fourcc(s: &str) -> u32 {
    s.bytes().take(4).fold(0, |a, b| (a << 8) | b as u32)
}

impl Smc {
    fn call(&self, input: &Param) -> Option<Param> {
        let mut out = Param::default();
        let mut size = std::mem::size_of::<Param>();
        // SAFETY: both buffers are `Param`-sized; the connection is the open AppleSMC client.
        let r = unsafe { IOConnectCallStructMethod(self.0, SMC_CALL, input as *const Param as *const c_void, size, &mut out as *mut Param as *mut c_void, &mut size) };
        (r == 0 && out.result == 0).then_some(out)
    }

    /// A key's value as a number, decoding the SMC's fixed-point and float types.
    fn read(&self, key: &str) -> Option<f64> {
        let info = self.call(&Param { key: fourcc(key), command: KEY_INFO, ..Default::default() })?.info;
        let data = self.call(&Param { key: fourcc(key), command: READ_KEY, info: KeyInfo { size: info.size, ..Default::default() }, ..Default::default() })?;
        let b = &data.bytes;
        let kind = info.kind.to_be_bytes();
        Some(match &kind {
            b"sp78" => i16::from_be_bytes([b[0], b[1]]) as f64 / 256.0,
            b"fpe2" => u16::from_be_bytes([b[0], b[1]]) as f64 / 4.0,
            b"flt " => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
            b"ui8 " => b[0] as f64,
            b"ui16" => u16::from_be_bytes([b[0], b[1]]) as f64,
            b"ui32" => u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f64,
            _ => return None,
        })
    }
}

/// Intel Mac temperature keys worth showing: (key, group, name, CPU/GPU priority).
type TempKey = (&'static str, &'static str, &'static str, Option<(Role, u8)>);
const INTEL_TEMPS: &[TempKey] = &[
    ("TC0F", "CPU", "Die", Some((Role::CpuTemp, 0))),
    ("TC0D", "CPU", "Die", Some((Role::CpuTemp, 1))),
    ("TCXC", "CPU", "PECI", Some((Role::CpuTemp, 2))),
    ("TC0P", "CPU", "Proximity", Some((Role::CpuTemp, 3))),
    ("TG0D", "GPU", "Die", Some((Role::GpuTemp, 0))),
    ("TGDD", "GPU", "Die", Some((Role::GpuTemp, 1))),
    ("TG0P", "GPU", "Proximity", Some((Role::GpuTemp, 2))),
    ("TPCD", "Mainboard", "Platform Controller Hub", None),
    ("TM0P", "Memory", "Proximity", None),
    ("Th0H", "Mainboard", "Heatsink", None),
    ("TA0P", "Mainboard", "Ambient", None),
    ("TB0T", "Battery", "Temperature", None),
    ("TH0P", "Drive", "Proximity", None),
];

fn smc_sensors(out: &mut Vec<Sensor>) {
    let Some(smc) = smc() else { return };
    let smc = smc.lock().unwrap();
    let fans = smc.read("FNum").unwrap_or(0.0) as u32;
    for i in 0..fans.min(8) {
        let Some(v) = smc.read(&format!("F{i}Ac")) else { continue };
        let max = smc.read(&format!("F{i}Mx"));
        let name = if fans == 1 { "Fan".to_string() } else { format!("Fan {}", i + 1) };
        out.push(Sensor::new(format!("smc:F{i}Ac"), "Fans", name, Kind::Fan, v).max(max).role(Role::Fan, i as u8));
    }
    if cfg!(target_arch = "x86_64") {
        for (key, group, name, role) in INTEL_TEMPS {
            let Some(v) = smc.read(key).filter(|v| *v > 0.0 && *v < 130.0) else { continue };
            let mut s = Sensor::new(format!("smc:{key}"), *group, *name, Kind::Temp, v);
            if let Some((r, p)) = role {
                s = s.role(*r, *p);
            }
            out.push(s);
        }
        for (key, name, role) in [("PCPC", "CPU package (cores)", Some((Role::CpuPower, 1))), ("PCPT", "CPU package", Some((Role::CpuPower, 0))), ("PCPG", "GPU", Some((Role::GpuPower, 1)))] {
            if let Some(v) = smc.read(key).filter(|v| *v >= 0.0 && *v < 1000.0) {
                let mut s = Sensor::new(format!("smc:{key}"), "Power", name, Kind::Power, v);
                if let Some((r, p)) = role {
                    s = s.role(r, p);
                }
                out.push(s);
            }
        }
    }
    if let Some(v) = smc.read("PSTR").filter(|v| *v >= 0.0 && *v < 1000.0) {
        out.push(Sensor::new("smc:PSTR", "Power", "System total", Kind::Power, v));
    }
}

pub fn scan() -> Scan {
    let mut sensors = vec![];
    if cfg!(target_arch = "aarch64") {
        hid_sensors(&mut sensors);
    }
    smc_sensors(&mut sensors);
    let hint = sensors.is_empty().then(|| "This Mac reports no sensors to apps".to_string());
    Scan { sensors, hint }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fourcc() {
        assert_eq!(super::fourcc("TC0P"), 0x5443_3050);
    }

    #[test]
    #[ignore = "reads this Mac's sensors"]
    fn live_scan() {
        for s in super::scan().sensors {
            eprintln!("{:<24} {:<40} {:>8.2} {:?}", s.id, s.title(), s.value, s.role);
        }
    }
}
