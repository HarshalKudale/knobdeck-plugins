//! Display brightness, per monitor.
//!
//! - **KDE Plasma 6.2+**: PowerDevil's `org.kde.ScreenBrightness` service, which already drives
//!   laptop backlights and DDC/CI monitors (and shows Plasma's own OSD).
//! - **Elsewhere (GNOME)**: the laptop panel through logind's `Session.SetBrightness` on the
//!   backlight device (no root needed), external monitors over DDC/CI on `/dev/i2c-*` (VCP 0x10),
//!   the way ddcutil does it. That needs the `i2c-dev` module and access to the bus (the `i2c`
//!   group, or the uaccess rule ddcutil installs).

use knobdeck_sdk::{Error, Result};
use linux_desktop::bus;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// Stable key: KDE's label, the connector name ("eDP-1", "DP-2") elsewhere.
    pub id: String,
    pub name: String,
    pub internal: bool,
}

const KDE: &str = "org.kde.ScreenBrightness";
const KDE_PATH: &str = "/org/kde/ScreenBrightness";
const KDE_DISPLAY: &str = "org.kde.ScreenBrightness.Display";

/// `KNOBDECK_BRIGHTNESS=sysfs` forces the non-KDE route (for testing it on Plasma).
fn use_kde() -> bool {
    std::env::var("KNOBDECK_BRIGHTNESS").as_deref() != Ok("sysfs") && bus::session_has_owner(KDE)
}

pub fn monitors() -> Result<Vec<Monitor>> {
    if use_kde() {
        Ok(kde::displays()?.into_iter().map(|d| d.monitor).collect())
    } else {
        Ok(sysfs::outputs().into_iter().map(|o| o.monitor).collect())
    }
}

/// Brightness in percent.
pub fn get(id: &str) -> Result<f64> {
    if use_kde() {
        kde::get(id)
    } else {
        sysfs::get(id)
    }
}

pub fn set(id: &str, percent: f64) -> Result<()> {
    let p = percent.clamp(0.0, 100.0);
    if use_kde() {
        kde::set(id, p)
    } else {
        sysfs::set(id, p)
    }
}

/// Labels made unique: a second "Dell U2723QE" becomes "Dell U2723QE (2)".
pub fn unique(names: Vec<String>) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    names
        .into_iter()
        .map(|n| {
            let c = seen.entry(n.clone()).or_insert(0);
            *c += 1;
            if *c == 1 {
                n
            } else {
                format!("{n} ({c})")
            }
        })
        .collect()
}

mod kde {
    use super::*;

    pub struct Display {
        pub monitor: Monitor,
        pub path: String,
    }

    pub fn displays() -> Result<Vec<Display>> {
        let conn = bus::session()?;
        let names: Vec<String> = bus::get(&conn, KDE, KDE_PATH, KDE, "DisplaysDBusNames")?;
        let mut raw = vec![];
        for n in names {
            let path = format!("{KDE_PATH}/{n}");
            let label: String = bus::get(&conn, KDE, &path, KDE_DISPLAY, "Label").unwrap_or_else(|_| n.clone());
            let internal: bool = bus::get(&conn, KDE, &path, KDE_DISPLAY, "IsInternal").unwrap_or(false);
            raw.push((label, internal, path));
        }
        let ids = unique(raw.iter().map(|(l, _, _)| l.clone()).collect());
        Ok(raw.into_iter().zip(ids).map(|((_, internal, path), id)| Display { monitor: Monitor { name: id.clone(), id, internal }, path }).collect())
    }

    fn find(id: &str) -> Result<Display> {
        displays()?.into_iter().find(|d| d.monitor.id == id).ok_or_else(|| Error::msg(format!("Display {id} isn't connected (or has no brightness control)")))
    }

    pub fn get(id: &str) -> Result<f64> {
        let d = find(id)?;
        let conn = bus::session()?;
        let (cur, max): (i32, i32) = (bus::get(&conn, KDE, &d.path, KDE_DISPLAY, "Brightness")?, bus::get(&conn, KDE, &d.path, KDE_DISPLAY, "MaxBrightness")?);
        Ok(if max > 0 { cur as f64 * 100.0 / max as f64 } else { 0.0 })
    }

    pub fn set(id: &str, percent: f64) -> Result<()> {
        let d = find(id)?;
        let conn = bus::session()?;
        let max: i32 = bus::get(&conn, KDE, &d.path, KDE_DISPLAY, "MaxBrightness")?;
        let value = (percent / 100.0 * max as f64).round() as i32;
        // Flags 0: Plasma shows its brightness OSD, as for the keyboard's brightness keys.
        Ok(bus::call0(&conn, KDE, &d.path, KDE_DISPLAY, "SetBrightness", &(value, 0u32))?)
    }
}

mod sysfs {
    use super::*;

    pub struct Output {
        pub monitor: Monitor,
        /// Internal panel: the backlight device's name; external: its I²C bus.
        pub backlight: Option<String>,
        pub i2c: Option<PathBuf>,
    }

    fn read(p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok().map(|s| s.trim().to_string())
    }

    /// The preferred backlight device (firmware over platform over raw, like GNOME and logind).
    fn backlight() -> Option<String> {
        let mut list: Vec<(u8, String)> = std::fs::read_dir("/sys/class/backlight")
            .ok()?
            .flatten()
            .map(|e| {
                let rank = match read(&e.path().join("type")).as_deref() {
                    Some("firmware") => 0,
                    Some("platform") => 1,
                    _ => 2,
                };
                (rank, e.file_name().to_string_lossy().into_owned())
            })
            .collect();
        list.sort();
        list.into_iter().next().map(|(_, n)| n)
    }

    /// The monitor's name from its EDID (display product name descriptor 0xFC).
    pub fn edid_name(edid: &[u8]) -> Option<String> {
        if edid.len() < 128 {
            return None;
        }
        (0..4).map(|i| 54 + i * 18).find_map(|o| {
            let d = &edid[o..o + 18];
            (d[0] == 0 && d[1] == 0 && d[3] == 0xFC).then(|| String::from_utf8_lossy(&d[5..18]).split('\n').next().unwrap_or("").trim().to_string()).filter(|s| !s.is_empty())
        })
    }

    /// Connected outputs: "card1-eDP-1" → id "eDP-1".
    pub fn outputs() -> Vec<Output> {
        let mut out = vec![];
        let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return out };
        let mut entries: Vec<PathBuf> = dir.flatten().map(|e| e.path()).filter(|p| p.join("status").exists()).collect();
        entries.sort();
        for p in entries {
            if read(&p.join("status")).as_deref() != Some("connected") {
                continue;
            }
            let full = p.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let connector = full.split_once('-').map_or(full.clone(), |(_, c)| c.to_string());
            if connector.starts_with("Writeback") {
                continue;
            }
            let internal = ["eDP", "LVDS", "DSI"].iter().any(|k| connector.starts_with(k));
            let edid = std::fs::read(p.join("edid")).unwrap_or_default();
            let name = edid_name(&edid).map(|n| format!("{n} ({connector})")).unwrap_or_else(|| connector.clone());
            let (backlight, i2c) = if internal { (backlight(), None) } else { (None, ddc::bus_for(&p, &edid)) };
            out.push(Output { monitor: Monitor { id: connector, name, internal }, backlight, i2c });
        }
        out
    }

    fn find(id: &str) -> Result<Output> {
        outputs().into_iter().find(|o| o.monitor.id == id).ok_or_else(|| Error::msg(format!("Display {id} isn't connected")))
    }

    fn backlight_values(dev: &str) -> Result<(f64, f64)> {
        let base = Path::new("/sys/class/backlight").join(dev);
        let num = |f: &str| read(&base.join(f)).and_then(|s| s.parse::<f64>().ok()).ok_or_else(|| Error::msg(format!("Can't read the {dev} backlight")));
        Ok((num("actual_brightness").or_else(|_| num("brightness"))?, num("max_brightness")?))
    }

    pub fn get(id: &str) -> Result<f64> {
        let o = find(id)?;
        if let Some(dev) = &o.backlight {
            let (cur, max) = backlight_values(dev)?;
            return Ok(if max > 0.0 { cur * 100.0 / max } else { 0.0 });
        }
        let bus = o.i2c.ok_or_else(|| no_ddc(id))?;
        let (cur, max) = ddc::cached_get(&bus)?;
        Ok(if max > 0 { cur as f64 * 100.0 / max as f64 } else { 0.0 })
    }

    pub fn set(id: &str, percent: f64) -> Result<()> {
        let o = find(id)?;
        if let Some(dev) = &o.backlight {
            let (_, max) = backlight_values(dev)?;
            // Never 0 on a panel: some backlights switch off completely.
            let value = ((percent / 100.0 * max).round() as u32).max(1);
            let conn = bus::system()?;
            return Ok(bus::call0(&conn, "org.freedesktop.login1", "/org/freedesktop/login1/session/auto", "org.freedesktop.login1.Session", "SetBrightness", &("backlight", dev.as_str(), value))?);
        }
        let bus = o.i2c.ok_or_else(|| no_ddc(id))?;
        let (_, max) = ddc::cached_get(&bus)?;
        ddc::set(&bus, (percent / 100.0 * max as f64).round() as u16)
    }

    fn no_ddc(id: &str) -> Error {
        Error::msg(format!(
            "No DDC/CI access to {id}. Load the i2c-dev module (sudo modprobe i2c-dev, and add it to /etc/modules-load.d) and give your user access to /dev/i2c-* (sudo usermod -aG i2c $USER, or install ddcutil's udev rule), then log in again. Some monitors need DDC/CI turned on in their own menu"
        ))
    }
}

/// DDC/CI over Linux i2c-dev.
mod ddc {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    const I2C_SLAVE: libc::c_ulong = 0x0703;
    const DDC_ADDR: u16 = 0x37;
    const EDID_ADDR: u16 = 0x50;

    fn open(bus: &Path, addr: u16) -> std::io::Result<File> {
        let f = OpenOptions::new().read(true).write(true).open(bus)?;
        if unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE as _, addr as libc::c_ulong) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(f)
    }

    /// The connector's own `ddc` link, else the `/dev/i2c-*` bus whose EDID matches (NVIDIA's
    /// driver doesn't link them).
    pub fn bus_for(connector: &Path, edid: &[u8]) -> Option<PathBuf> {
        if let Ok(link) = std::fs::read_link(connector.join("ddc")) {
            let name = link.file_name()?.to_string_lossy().into_owned();
            return Some(PathBuf::from("/dev").join(name));
        }
        if edid.len() < 128 {
            return None;
        }
        let mut buses: Vec<PathBuf> = std::fs::read_dir("/dev").ok()?.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("i2c-"))).collect();
        buses.sort();
        buses.into_iter().find(|b| read_edid(b).is_some_and(|e| e[..] == edid[..128]))
    }

    fn read_edid(bus: &Path) -> Option<[u8; 128]> {
        let mut f = open(bus, EDID_ADDR).ok()?;
        f.write_all(&[0]).ok()?;
        let mut buf = [0u8; 128];
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    fn io(e: std::io::Error, bus: &Path) -> Error {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            Error::msg(format!("No permission to use {}: add your user to the i2c group (sudo usermod -aG i2c $USER) and log in again", bus.display()))
        } else {
            Error::msg(format!("The monitor on {} didn't answer over DDC/CI ({e}). Check that DDC/CI is on in the monitor's menu", bus.display()))
        }
    }

    /// Recent reads per bus: DDC is slow and some monitors misbehave when asked too often.
    static CACHE: Mutex<Option<HashMap<PathBuf, (Instant, u16, u16)>>> = Mutex::new(None);

    pub fn cached_get(bus: &Path) -> Result<(u16, u16)> {
        if let Some((at, c, m)) = CACHE.lock().unwrap().as_ref().and_then(|m| m.get(bus)).copied() {
            if at.elapsed() < Duration::from_secs(3) {
                return Ok((c, m));
            }
        }
        let (c, m) = get(bus)?;
        CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(bus.to_path_buf(), (Instant::now(), c, m));
        Ok((c, m))
    }

    fn get(bus: &Path) -> Result<(u16, u16)> {
        let mut last = Error::msg("no reply");
        for _ in 0..3 {
            let mut f = open(bus, DDC_ADDR).map_err(|e| io(e, bus))?;
            f.write_all(&packet::get_request(0x10)).map_err(|e| io(e, bus))?;
            std::thread::sleep(Duration::from_millis(50));
            let mut buf = [0u8; 12];
            if let Err(e) = f.read(&mut buf) {
                last = io(e, bus);
            } else if let Some(v) = packet::parse_reply(&buf, 0x10) {
                return Ok(v);
            } else {
                last = Error::msg(format!("The monitor on {} sent an unreadable DDC/CI reply", bus.display()));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
        Err(last)
    }

    pub fn set(bus: &Path, value: u16) -> Result<()> {
        let mut f = open(bus, DDC_ADDR).map_err(|e| io(e, bus))?;
        f.write_all(&packet::set_request(0x10, value)).map_err(|e| io(e, bus))?;
        if let Some(e) = CACHE.lock().unwrap().as_mut().and_then(|m| m.get_mut(bus)) {
            *e = (Instant::now(), value, e.2);
        }
        // Monitors need a pause before the next command.
        std::thread::sleep(Duration::from_millis(50));
        Ok(())
    }
}

/// DDC/CI (VESA MCCS) messages to the monitor's address 0x37, starting at the source byte 0x51.
pub mod packet {
    /// XOR checksum over the destination address byte (0x6E) and the message.
    fn checksum(bytes: &[u8]) -> u8 {
        bytes.iter().fold(0x6E, |a, b| a ^ b)
    }

    /// "Get VCP feature" for `code`.
    pub fn get_request(code: u8) -> Vec<u8> {
        let mut m = vec![0x51, 0x82, 0x01, code];
        m.push(checksum(&m));
        m
    }

    /// "Set VCP feature" `code` to `value`.
    pub fn set_request(code: u8, value: u16) -> Vec<u8> {
        let mut m = vec![0x51, 0x84, 0x03, code, (value >> 8) as u8, value as u8];
        m.push(checksum(&m));
        m
    }

    /// `(current, max)` from a "VCP feature reply": `… 0x02 result code type maxH maxL curH curL chk`.
    pub fn parse_reply(buf: &[u8], code: u8) -> Option<(u16, u16)> {
        let i = (0..buf.len().saturating_sub(7)).find(|&i| buf[i] == 0x02 && buf[i + 2] == code)?;
        if buf[i + 1] != 0 {
            return None; // the monitor doesn't support this feature
        }
        let max = u16::from_be_bytes([buf[i + 4], buf[i + 5]]);
        let cur = u16::from_be_bytes([buf[i + 6], buf[i + 7]]);
        (max > 0).then_some((cur, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddc_packets() {
        assert_eq!(packet::get_request(0x10), vec![0x51, 0x82, 0x01, 0x10, 0xAC]);
        assert_eq!(packet::set_request(0x10, 50), vec![0x51, 0x84, 0x03, 0x10, 0x00, 0x32, 0x6E ^ 0x51 ^ 0x84 ^ 0x03 ^ 0x10 ^ 0x32]);
        // A reply as read from i2c-dev: source address, length, opcode 0x02, result 0, code 0x10, type, max 100, current 40.
        assert_eq!(packet::parse_reply(&[0x6E, 0x88, 0x02, 0x00, 0x10, 0x00, 0x00, 0x64, 0x00, 0x28, 0x00], 0x10), Some((40, 100)));
        assert_eq!(packet::parse_reply(&[0x6E, 0x88, 0x02, 0x01, 0x10, 0x00, 0x00, 0x64, 0x00, 0x28, 0x00], 0x10), None);
        assert_eq!(packet::parse_reply(&[0; 11], 0x10), None);
    }

    #[test]
    fn names() {
        assert_eq!(unique(vec!["A".into(), "B".into(), "A".into()]), vec!["A", "B", "A (2)"]);
        let mut edid = vec![0u8; 128];
        edid[54 + 18..54 + 18 + 18].copy_from_slice(&[0, 0, 0, 0xFC, 0, b'L', b'G', b' ', b'U', b'L', b'T', b'R', b'A', b'\n', b' ', b' ', b' ', b' ']);
        assert_eq!(sysfs::edid_name(&edid).as_deref(), Some("LG ULTRA"));
    }

    /// Reads (never sets) every display's brightness through both routes.
    #[test]
    #[ignore = "needs displays"]
    fn live_read() {
        for m in monitors().unwrap() {
            eprintln!("{m:?}: {:?}", get(&m.id));
        }
        std::env::set_var("KNOBDECK_BRIGHTNESS", "sysfs");
        for m in monitors().unwrap() {
            eprintln!("sysfs {m:?}: {:?}", get(&m.id));
        }
    }
}

#[cfg(test)]
mod live_set {
    /// Sets DP-4 over raw DDC to 35 %, reads it back from the monitor, restores the old value.
    #[test]
    #[ignore = "changes a real monitor's brightness"]
    fn ddc_round_trip() {
        std::env::set_var("KNOBDECK_BRIGHTNESS", "sysfs");
        let before = super::get("DP-4").unwrap();
        super::set("DP-4", 35.0).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(4)); // past the read cache
        let after = super::get("DP-4").unwrap();
        super::set("DP-4", before).unwrap();
        eprintln!("before {before}, read back {after}");
        assert_eq!(after.round(), 35.0);
    }
}
