//! VPN services through `scutil --nc` (SystemConfiguration's SCNetworkConnection, the same
//! services as System Settings ▸ VPN, including Network Extension VPNs). Its output isn't
//! localized. Wi-Fi switching is not offered on macOS (see `mod.rs`).

use super::scutil::parse_list;
use super::{Conn, Kind};
use knobdeck_sdk::{Error, Result};
use std::process::Command;

fn scutil(args: &[&str]) -> Result<String> {
    let out = Command::new("/usr/sbin/scutil")
        .arg("--nc")
        .args(args)
        .output()
        .map_err(|e| Error::msg(format!("scutil: {e}")))?;
    if !out.status.success() {
        return Err(Error::msg(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn list(kind: Kind) -> Result<Vec<Conn>> {
    if kind == Kind::Wifi {
        return Err(Error::msg(
            "Switching Wi-Fi networks isn't available on macOS",
        ));
    }
    Ok(parse_list(&scutil(&["list"])?))
}

pub fn set(kind: Kind, id: &str, on: bool) -> Result<()> {
    if kind == Kind::Wifi {
        return Err(Error::msg(
            "Switching Wi-Fi networks isn't available on macOS",
        ));
    }
    scutil(&[if on { "start" } else { "stop" }, id]).map(|_| ())
}
