//! NVIDIA GPUs through NVML, the driver's own monitoring library (`nvml.dll` on Windows,
//! `libnvidia-ml.so.1` on Linux), loaded at run time: without an NVIDIA driver it is simply absent.

use crate::sensors::{Kind, Role, Sensor};
use std::ffi::{c_char, c_void, CStr};
use std::sync::OnceLock;

type Dev = *mut c_void;
type Ret = i32;

struct Nvml {
    count: unsafe extern "C" fn(*mut u32) -> Ret,
    handle: unsafe extern "C" fn(u32, *mut Dev) -> Ret,
    name: unsafe extern "C" fn(Dev, *mut c_char, u32) -> Ret,
    uuid: unsafe extern "C" fn(Dev, *mut c_char, u32) -> Ret,
    temperature: unsafe extern "C" fn(Dev, u32, *mut u32) -> Ret,
    utilization: unsafe extern "C" fn(Dev, *mut [u32; 2]) -> Ret,
    memory: unsafe extern "C" fn(Dev, *mut [u64; 3]) -> Ret,
    /// Newer drivers: used memory without the driver's own reservation (what nvidia-smi shows).
    memory_v2: Option<unsafe extern "C" fn(Dev, *mut MemoryV2) -> Ret>,
    power: unsafe extern "C" fn(Dev, *mut u32) -> Ret,
    power_limit: unsafe extern "C" fn(Dev, *mut u32) -> Ret,
    fan_count: Option<unsafe extern "C" fn(Dev, *mut u32) -> Ret>,
    fan_v2: Option<unsafe extern "C" fn(Dev, u32, *mut u32) -> Ret>,
    fan: unsafe extern "C" fn(Dev, *mut u32) -> Ret,
}

#[repr(C)]
#[derive(Default)]
struct MemoryV2 {
    version: u32,
    total: u64,
    reserved: u64,
    free: u64,
    used: u64,
}

const LIBS: &[&str] = if cfg!(windows) {
    &["nvml.dll", "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll"]
} else {
    &["libnvidia-ml.so.1", "libnvidia-ml.so"]
};

fn load() -> Option<Nvml> {
    // SAFETY: loading the NVIDIA driver's library; the handle is leaked so the function pointers
    // stay valid for the life of the process.
    let lib = LIBS.iter().find_map(|p| unsafe { libloading::Library::new(p) }.ok())?;
    let lib: &'static libloading::Library = Box::leak(Box::new(lib));
    unsafe {
        macro_rules! sym {
            ($n:literal) => {
                *lib.get($n).ok()?
            };
        }
        let init: unsafe extern "C" fn() -> Ret = sym!(b"nvmlInit_v2\0");
        if init() != 0 {
            return None; // library present, no NVIDIA GPU / driver not loaded
        }
        Some(Nvml {
            count: sym!(b"nvmlDeviceGetCount_v2\0"),
            handle: sym!(b"nvmlDeviceGetHandleByIndex_v2\0"),
            name: sym!(b"nvmlDeviceGetName\0"),
            uuid: sym!(b"nvmlDeviceGetUUID\0"),
            temperature: sym!(b"nvmlDeviceGetTemperature\0"),
            utilization: sym!(b"nvmlDeviceGetUtilizationRates\0"),
            memory: sym!(b"nvmlDeviceGetMemoryInfo\0"),
            memory_v2: lib.get(b"nvmlDeviceGetMemoryInfo_v2\0").ok().map(|s| *s),
            power: sym!(b"nvmlDeviceGetPowerUsage\0"),
            power_limit: sym!(b"nvmlDeviceGetEnforcedPowerLimit\0"),
            fan_count: lib.get(b"nvmlDeviceGetNumFans\0").ok().map(|s| *s),
            fan_v2: lib.get(b"nvmlDeviceGetFanSpeed_v2\0").ok().map(|s| *s),
            fan: sym!(b"nvmlDeviceGetFanSpeed\0"),
        })
    }
}

static NVML: OnceLock<Option<Nvml>> = OnceLock::new();

fn text(f: unsafe extern "C" fn(Dev, *mut c_char, u32) -> Ret, dev: Dev) -> Option<String> {
    let mut buf = [0 as c_char; 96];
    // SAFETY: NVML writes a NUL-terminated string of at most `len` bytes.
    (unsafe { f(dev, buf.as_mut_ptr(), buf.len() as u32) } == 0).then(|| unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned())
}

/// Temperature, load, VRAM, power and fans of every NVIDIA GPU.
pub fn sensors() -> Vec<Sensor> {
    let Some(n) = NVML.get_or_init(load) else { return vec![] };
    let mut out = vec![];
    let mut count = 0u32;
    // SAFETY: NVML calls with valid out-pointers on a handle NVML handed out.
    unsafe {
        if (n.count)(&mut count) != 0 {
            return out;
        }
        for i in 0..count {
            let mut dev: Dev = std::ptr::null_mut();
            if (n.handle)(i, &mut dev) != 0 {
                continue;
            }
            let name = text(n.name, dev).unwrap_or_else(|| format!("NVIDIA GPU {}", i + 1));
            let key = text(n.uuid, dev).unwrap_or_else(|| i.to_string());
            let id = |what: &str| format!("nvml:{key}:{what}");
            let p = if i == 0 { 0 } else { 2 };
            let mut v = 0u32;
            if (n.temperature)(dev, 0, &mut v) == 0 {
                out.push(Sensor::new(id("temp"), &name, "Temperature", Kind::Temp, v as f64).role(Role::GpuTemp, p));
            }
            let mut u = [0u32; 2];
            if (n.utilization)(dev, &mut u) == 0 {
                out.push(Sensor::new(id("load"), &name, "Load", Kind::Load, u[0] as f64).role(Role::GpuLoad, p));
            }
            let mut m2 = MemoryV2 { version: std::mem::size_of::<MemoryV2>() as u32 | (2 << 24), ..Default::default() };
            let mut m = [0u64; 3]; // total, free, used (bytes)
            let mem = match n.memory_v2 {
                Some(f) if f(dev, &mut m2) == 0 => Some((m2.used, m2.total)),
                _ => ((n.memory)(dev, &mut m) == 0).then_some((m[2], m[0])),
            };
            if let Some((used, total)) = mem {
                let mb = |b: u64| b as f64 / 1048576.0;
                out.push(Sensor::new(id("memory"), &name, "Memory used", Kind::Memory, mb(used)).max(Some(mb(total))).role(Role::GpuMemory, p));
            }
            if (n.power)(dev, &mut v) == 0 {
                let mut limit = 0u32;
                let limit = ((n.power_limit)(dev, &mut limit) == 0).then_some(limit as f64 / 1000.0);
                out.push(Sensor::new(id("power"), &name, "Power", Kind::Power, v as f64 / 1000.0).max(limit).role(Role::GpuPower, p));
            }
            let mut fans = 0u32;
            match (n.fan_count, n.fan_v2) {
                (Some(c), Some(f)) if c(dev, &mut fans) == 0 => {
                    for f_i in 0..fans {
                        if f(dev, f_i, &mut v) == 0 {
                            let label = if fans == 1 { "Fan".to_string() } else { format!("Fan {}", f_i + 1) };
                            out.push(Sensor::new(id(&format!("fan{f_i}")), &name, label, Kind::FanPct, v as f64).role(Role::GpuFan, p + f_i as u8));
                        }
                    }
                }
                _ => {
                    if (n.fan)(dev, &mut v) == 0 {
                        out.push(Sensor::new(id("fan0"), &name, "Fan", Kind::FanPct, v as f64).role(Role::GpuFan, p));
                    }
                }
            }
        }
    }
    out
}
