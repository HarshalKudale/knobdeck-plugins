//! Wi-Fi networks and VPNs through NetworkManager's D-Bus API (saved connections, activate /
//! deactivate). NetworkManager asks polkit; a user at the machine is allowed by default.

use super::{Conn, Kind};
use knobdeck_sdk::{Error, Result};
use linux_desktop::bus;
use std::collections::HashMap;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const DEVICE: &str = "org.freedesktop.NetworkManager.Device";
/// `NM_DEVICE_TYPE_WIFI`.
const DEVICE_WIFI: u32 = 2;

fn conn() -> Result<zbus::blocking::Connection> {
    let c = bus::system()?;
    if !bus::system_has_owner(NM) {
        return Err(Error::msg("NetworkManager is not running"));
    }
    Ok(c)
}

fn matches(kind: Kind, ty: &str) -> bool {
    match kind {
        Kind::Wifi => ty == "802-11-wireless",
        Kind::Vpn => ty == "vpn" || ty == "wireguard",
    }
}

fn text(v: Option<&OwnedValue>) -> String {
    v.and_then(|v| String::try_from(v.try_clone().ok()?).ok())
        .unwrap_or_default()
}

/// Active connections: uuid → (active object path, state). State 2 is "activated", 1 "activating".
fn active(c: &zbus::blocking::Connection) -> Result<HashMap<String, (OwnedObjectPath, u32)>> {
    let paths: Vec<OwnedObjectPath> = bus::get(c, NM, NM_PATH, NM, "ActiveConnections")?;
    let mut out = HashMap::new();
    for p in paths {
        let (Ok(uuid), Ok(state)) = (
            bus::get::<String>(c, NM, p.as_str(), ACTIVE, "Uuid"),
            bus::get::<u32>(c, NM, p.as_str(), ACTIVE, "State"),
        ) else {
            continue;
        };
        out.insert(uuid, (p, state));
    }
    Ok(out)
}

/// Saved connections of `kind`: (uuid, name, settings path).
fn saved(
    c: &zbus::blocking::Connection,
    kind: Kind,
) -> Result<Vec<(String, String, OwnedObjectPath)>> {
    let paths: Vec<OwnedObjectPath> =
        bus::call(c, NM, SETTINGS_PATH, SETTINGS, "ListConnections", &())?;
    let mut out = Vec::new();
    for p in paths {
        let Ok(s) = bus::call::<_, HashMap<String, HashMap<String, OwnedValue>>>(
            c,
            NM,
            p.as_str(),
            "org.freedesktop.NetworkManager.Settings.Connection",
            "GetSettings",
            &(),
        ) else {
            continue;
        };
        let Some(meta) = s.get("connection") else {
            continue;
        };
        if matches(kind, &text(meta.get("type"))) {
            out.push((text(meta.get("uuid")), text(meta.get("id")), p));
        }
    }
    out.sort_by_key(|c| c.1.to_lowercase());
    Ok(out)
}

pub fn list(kind: Kind) -> Result<Vec<Conn>> {
    let c = conn()?;
    let act = active(&c)?;
    Ok(saved(&c, kind)?
        .into_iter()
        .map(|(uuid, name, _)| Conn {
            active: act.get(&uuid).is_some_and(|(_, st)| (1..=2).contains(st)),
            id: uuid,
            name,
        })
        .collect())
}

/// The first Wi-Fi device (Wi-Fi connections need one to activate on).
fn wifi_device(c: &zbus::blocking::Connection) -> Result<OwnedObjectPath> {
    let devices: Vec<OwnedObjectPath> = bus::call(c, NM, NM_PATH, NM, "GetDevices", &())?;
    devices
        .into_iter()
        .find(|d| {
            bus::get::<u32>(c, NM, d.as_str(), DEVICE, "DeviceType").ok() == Some(DEVICE_WIFI)
        })
        .ok_or_else(|| Error::msg("No Wi-Fi adapter found"))
}

pub fn set(kind: Kind, id: &str, on: bool) -> Result<()> {
    let c = conn()?;
    let act = active(&c)?;
    if !on {
        let Some((path, _)) = act.get(id) else {
            return Ok(());
        };
        bus::call0(
            &c,
            NM,
            NM_PATH,
            NM,
            "DeactivateConnection",
            &(ObjectPath::from(path),),
        )?;
        return Ok(());
    }
    let (_, name, path) = saved(&c, kind)?
        .into_iter()
        .find(|(u, _, _)| u == id)
        .ok_or_else(|| Error::msg("That connection no longer exists in NetworkManager"))?;
    let root = ObjectPath::try_from("/").expect("valid path");
    let device = match kind {
        Kind::Wifi => wifi_device(&c)?,
        // VPN and WireGuard connections bring up their own interface.
        Kind::Vpn => OwnedObjectPath::from(root.clone()),
    };
    let act = bus::call::<_, OwnedObjectPath>(
        &c,
        NM,
        NM_PATH,
        NM,
        "ActivateConnection",
        &(ObjectPath::from(&path), ObjectPath::from(&device), root),
    )
    .map_err(|e| Error::msg(format!("{name}: {}", e.message())))?;
    // Activation runs in the background: wait for it to finish, so a failure shows on the key.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::time::Instant::now() < until {
        match bus::get::<u32>(&c, NM, act.as_str(), ACTIVE, "State") {
            Ok(2) => return Ok(()),
            Ok(0 | 1) => std::thread::sleep(std::time::Duration::from_millis(250)),
            _ => break,
        }
    }
    Err(Error::msg(format!(
        "{name}: NetworkManager could not connect"
    )))
}
