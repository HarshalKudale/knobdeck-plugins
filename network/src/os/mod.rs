//! Saved Wi-Fi networks and VPNs, through each OS's own network API:
//! Linux NetworkManager (D-Bus), Windows Native Wi-Fi (`wlanapi`) and RAS (`rasapi32`),
//! macOS SystemConfiguration's VPN services through `scutil --nc`.
//! macOS has no Wi-Fi switching here: CoreWLAN needs the network's password again for every join
//! (and Location access to read SSIDs), so that action is disabled on macOS.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Wifi,
    Vpn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Conn {
    /// What the action stores: NetworkManager uuid, Wi-Fi profile name, phonebook entry, service id.
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{list, set};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::{list, set};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod scutil;
#[cfg(target_os = "macos")]
pub use macos::{list, set};
