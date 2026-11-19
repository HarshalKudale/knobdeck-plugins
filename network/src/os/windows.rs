//! Windows: saved Wi-Fi profiles through the Native Wi-Fi API (`WlanGetProfileList`,
//! `WlanConnect`, `WlanDisconnect`) and VPN phonebook entries through RAS (`RasEnumEntriesW`,
//! `RasEnumConnectionsW`, `RasDialW` with the entry's saved credentials, `RasHangUpW`): the same
//! entries as Settings ▸ Network ▸ VPN.

use super::{Conn, Kind};
use knobdeck_sdk::{Error, Result};
use windows::core::{GUID, PCWSTR};
use windows::Win32::Foundation::{BOOL, HANDLE};
use windows::Win32::NetworkManagement::Rras::{
    RasDialW, RasEnumConnectionsW, RasEnumEntriesW, RasGetEntryDialParamsW, RasHangUpW, HRASCONN,
    RASCONNW, RASDIALPARAMSW, RASENTRYNAMEW,
};
use windows::Win32::NetworkManagement::WiFi::*;

const ERROR_BUFFER_TOO_SMALL: u32 = 603;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

// ── Wi-Fi ──

/// An open WLAN client handle, closed on drop.
struct Wlan(HANDLE);

impl Wlan {
    fn open() -> Result<Wlan> {
        let (mut version, mut h) = (0u32, HANDLE::default());
        let rc = unsafe { WlanOpenHandle(2, None, &mut version, &mut h) };
        if rc != 0 {
            return Err(Error::msg(format!(
                "The WLAN service isn't available (error {rc}); is there a Wi-Fi adapter?"
            )));
        }
        Ok(Wlan(h))
    }

    /// The first Wi-Fi interface's GUID.
    fn interface(&self) -> Result<GUID> {
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let rc = unsafe { WlanEnumInterfaces(self.0, None, &mut list) };
        if rc != 0 || list.is_null() {
            return Err(Error::msg(format!(
                "Could not list Wi-Fi adapters (error {rc})"
            )));
        }
        let found = unsafe {
            ((*list).dwNumberOfItems > 0).then(|| (*list).InterfaceInfo[0].InterfaceGuid)
        };
        unsafe { WlanFreeMemory(list as *const _) };
        found.ok_or_else(|| Error::msg("No Wi-Fi adapter found"))
    }

    fn profiles(&self, guid: &GUID) -> Result<Vec<String>> {
        let mut list: *mut WLAN_PROFILE_INFO_LIST = std::ptr::null_mut();
        let rc = unsafe { WlanGetProfileList(self.0, guid, None, &mut list) };
        if rc != 0 || list.is_null() {
            return Err(Error::msg(format!(
                "Could not read the saved Wi-Fi networks (error {rc})"
            )));
        }
        let names = unsafe {
            let n = (*list).dwNumberOfItems as usize;
            let first = (*list).ProfileInfo.as_ptr();
            (0..n)
                .map(|i| from_wide(&(*first.add(i)).strProfileName))
                .collect()
        };
        unsafe { WlanFreeMemory(list as *const _) };
        Ok(names)
    }

    /// The profile the interface is connected with, if any.
    fn current(&self, guid: &GUID) -> Option<String> {
        let (mut size, mut data) = (0u32, std::ptr::null_mut());
        let rc = unsafe {
            WlanQueryInterface(
                self.0,
                guid,
                wlan_intf_opcode_current_connection,
                None,
                &mut size,
                &mut data,
                None,
            )
        };
        if rc != 0 || data.is_null() {
            return None;
        }
        let attrs = data as *const WLAN_CONNECTION_ATTRIBUTES;
        let name = unsafe {
            ((*attrs).isState == wlan_interface_state_connected)
                .then(|| from_wide(&(*attrs).strProfileName))
        };
        unsafe { WlanFreeMemory(data as *const _) };
        name
    }
}

impl Drop for Wlan {
    fn drop(&mut self) {
        unsafe { WlanCloseHandle(self.0, None) };
    }
}

fn wifi_list() -> Result<Vec<Conn>> {
    let w = Wlan::open()?;
    let guid = w.interface()?;
    let current = w.current(&guid);
    Ok(w.profiles(&guid)?
        .into_iter()
        .map(|p| Conn {
            active: current.as_deref() == Some(p.as_str()),
            id: p.clone(),
            name: p,
        })
        .collect())
}

fn wifi_set(profile: &str, on: bool) -> Result<()> {
    let w = Wlan::open()?;
    let guid = w.interface()?;
    if !on {
        if w.current(&guid).as_deref() != Some(profile) {
            return Ok(());
        }
        let rc = unsafe { WlanDisconnect(w.0, &guid, None) };
        return if rc == 0 {
            Ok(())
        } else {
            Err(Error::msg(format!("Could not disconnect (error {rc})")))
        };
    }
    let name = wide(profile);
    let params = WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: wlan_connection_mode_profile,
        strProfile: PCWSTR(name.as_ptr()),
        pDot11Ssid: std::ptr::null_mut(),
        pDesiredBssidList: std::ptr::null_mut(),
        dot11BssType: dot11_BSS_type_any,
        dwFlags: 0,
    };
    let rc = unsafe { WlanConnect(w.0, &guid, &params, None) };
    if rc != 0 {
        return Err(Error::msg(format!(
            "{profile}: could not connect (error {rc}); is the network in range?"
        )));
    }
    Ok(())
}

// ── VPN (RAS) ──

fn ras_entries() -> Result<Vec<String>> {
    let mut buf = vec![RASENTRYNAMEW {
        dwSize: std::mem::size_of::<RASENTRYNAMEW>() as u32,
        ..Default::default()
    }];
    let (mut bytes, mut count) = (
        (buf.len() * std::mem::size_of::<RASENTRYNAMEW>()) as u32,
        0u32,
    );
    let mut rc = unsafe {
        RasEnumEntriesW(
            PCWSTR::null(),
            PCWSTR::null(),
            Some(buf.as_mut_ptr()),
            &mut bytes,
            &mut count,
        )
    };
    if rc == ERROR_BUFFER_TOO_SMALL {
        let n = (bytes as usize).div_ceil(std::mem::size_of::<RASENTRYNAMEW>());
        buf = vec![
            RASENTRYNAMEW {
                dwSize: std::mem::size_of::<RASENTRYNAMEW>() as u32,
                ..Default::default()
            };
            n
        ];
        rc = unsafe {
            RasEnumEntriesW(
                PCWSTR::null(),
                PCWSTR::null(),
                Some(buf.as_mut_ptr()),
                &mut bytes,
                &mut count,
            )
        };
    }
    if rc != 0 {
        return Err(Error::msg(format!(
            "Could not list VPN connections (RAS error {rc})"
        )));
    }
    Ok(buf[..count as usize]
        .iter()
        .map(|e| from_wide(&e.szEntryName))
        .collect())
}

fn ras_connections() -> Result<Vec<(String, HRASCONN)>> {
    let blank = || RASCONNW {
        dwSize: std::mem::size_of::<RASCONNW>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut buf = vec![blank()];
    let (mut bytes, mut count) = (std::mem::size_of::<RASCONNW>() as u32, 0u32);
    let mut rc = unsafe { RasEnumConnectionsW(Some(buf.as_mut_ptr()), &mut bytes, &mut count) };
    if rc == ERROR_BUFFER_TOO_SMALL {
        let n = (bytes as usize).div_ceil(std::mem::size_of::<RASCONNW>());
        buf = (0..n).map(|_| blank()).collect();
        rc = unsafe { RasEnumConnectionsW(Some(buf.as_mut_ptr()), &mut bytes, &mut count) };
    }
    if rc != 0 {
        return Err(Error::msg(format!(
            "Could not read VPN connections (RAS error {rc})"
        )));
    }
    Ok(buf[..count as usize]
        .iter()
        .map(|c| (from_wide(&c.szEntryName), c.hrasconn))
        .collect())
}

fn vpn_list() -> Result<Vec<Conn>> {
    let up = ras_connections()?;
    Ok(ras_entries()?
        .into_iter()
        .map(|e| Conn {
            active: up.iter().any(|(n, _)| *n == e),
            id: e.clone(),
            name: e,
        })
        .collect())
}

fn vpn_set(entry: &str, on: bool) -> Result<()> {
    if !on {
        for (_, h) in ras_connections()?.into_iter().filter(|(n, _)| n == entry) {
            unsafe { RasHangUpW(h) };
        }
        return Ok(());
    }
    if ras_connections()?.iter().any(|(n, _)| n == entry) {
        return Ok(());
    }
    // The entry's saved user name and password, as Windows' own VPN flyout uses them.
    let mut params: RASDIALPARAMSW = unsafe { std::mem::zeroed() };
    params.dwSize = std::mem::size_of::<RASDIALPARAMSW>() as u32;
    let name: Vec<u16> = entry
        .encode_utf16()
        .take(params.szEntryName.len() - 1)
        .collect();
    params.szEntryName[..name.len()].copy_from_slice(&name);
    let mut has_password = BOOL(0);
    let rc = unsafe { RasGetEntryDialParamsW(PCWSTR::null(), &mut params, &mut has_password) };
    if rc != 0 {
        return Err(Error::msg(format!(
            "{entry}: no such VPN connection (RAS error {rc})"
        )));
    }
    let mut h = HRASCONN::default();
    // No notifier: RasDialW returns once connected or failed.
    let rc = unsafe { RasDialW(None, PCWSTR::null(), &params, 0, None, &mut h) };
    if rc != 0 {
        if !h.is_invalid() {
            unsafe { RasHangUpW(h) };
        }
        let hint = if has_password.as_bool() {
            ""
        } else {
            " Connect once from Windows' VPN settings and save the sign-in info."
        };
        return Err(Error::msg(format!(
            "{entry}: could not connect (RAS error {rc}).{hint}"
        )));
    }
    Ok(())
}

pub fn list(kind: Kind) -> Result<Vec<Conn>> {
    match kind {
        Kind::Wifi => wifi_list(),
        Kind::Vpn => vpn_list(),
    }
}

pub fn set(kind: Kind, id: &str, on: bool) -> Result<()> {
    match kind {
        Kind::Wifi => wifi_set(id, on),
        Kind::Vpn => vpn_set(id, on),
    }
}
