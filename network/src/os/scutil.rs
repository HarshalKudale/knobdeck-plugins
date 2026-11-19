//! `scutil --nc list` output (kept apart from `macos.rs` so it is tested on every OS).

use super::Conn;

/// `* (Connected)    5E0B…-…  VPN (com.wireguard.macos) "Home"   [VPN:com.wireguard.macos]`
pub fn parse_list(text: &str) -> Vec<Conn> {
    text.lines()
        .filter_map(|l| {
            let open = l.find('(')?;
            let close = open + l[open..].find(')')?;
            let state = &l[open + 1..close];
            let rest = l[close + 1..].trim_start();
            let id = rest.split_whitespace().next()?.to_string();
            let q1 = rest.find('"')?;
            let q2 = q1 + 1 + rest[q1 + 1..].find('"')?;
            Some(Conn {
                id,
                name: rest[q1 + 1..q2].to_string(),
                active: state == "Connected" || state == "Connecting",
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn list_lines() {
        let text = "Available network connection services in the current set (*=enabled):\n* (Connected)      5E0B1C2D-0000-0000-0000-000000000001 VPN (com.wireguard.macos) \"Home WG\"            [VPN/com.wireguard.macos]\n* (Disconnected)   A1B2C3D4-0000-0000-0000-000000000002 IPSec              \"Office\"             [IPSec]\n";
        let l = super::parse_list(text);
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].name.as_str(), l[0].active), ("Home WG", true));
        assert_eq!(
            (l[1].id.as_str(), l[1].active),
            ("A1B2C3D4-0000-0000-0000-000000000002", false)
        );
    }
}
