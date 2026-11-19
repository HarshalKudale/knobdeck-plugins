//! Wake-on-LAN: the magic packet (6 × 0xFF, then the MAC 16 times) as a UDP broadcast.

use knobdeck_sdk::{Error, Result};
use std::net::UdpSocket;

/// "AA:BB:CC:DD:EE:FF", "aa-bb-cc-dd-ee-ff", "aabb.ccdd.eeff" or "AABBCCDDEEFF".
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let hex: String = text
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.' | ' '))
        .collect();
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(mac)
}

pub fn packet(mac: &[u8; 6]) -> Vec<u8> {
    let mut p = vec![0xFF; 6];
    for _ in 0..16 {
        p.extend_from_slice(mac);
    }
    p
}

/// Send the packet to `to:port` three times (UDP can drop one).
pub fn send(mac: &[u8; 6], to: &str, port: u16) -> Result<()> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| Error::msg(format!("Wake on LAN: {e}")))?;
    sock.set_broadcast(true)
        .map_err(|e| Error::msg(format!("Wake on LAN: {e}")))?;
    let p = packet(mac);
    for _ in 0..3 {
        sock.send_to(&p, (to, port))
            .map_err(|e| Error::msg(format!("Wake on LAN to {to}: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macs() {
        assert_eq!(
            parse_mac("aa-bb-cc-dd-ee-0f"),
            Some([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x0F])
        );
        assert_eq!(parse_mac("AABB.CCDD.EEFF"), parse_mac("AA:BB:CC:DD:EE:FF"));
        assert_eq!(parse_mac("AA:BB:CC:DD:EE"), None);
        assert_eq!(parse_mac("GG:BB:CC:DD:EE:FF"), None);
        let p = packet(&[1, 2, 3, 4, 5, 6]);
        assert_eq!(p.len(), 102);
        assert_eq!(&p[..7], &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 1]);
        assert_eq!(&p[96..], &[1, 2, 3, 4, 5, 6]);
    }

    /// The packet really leaves: a local listener receives it (loopback, and the broadcast address).
    #[test]
    fn sends() {
        let rx = UdpSocket::bind("0.0.0.0:0").unwrap();
        rx.set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let port = rx.local_addr().unwrap().port();
        let mac = [0x10, 0x20, 0x30, 0x40, 0x50, 0x60];
        send(&mac, "127.0.0.1", port).unwrap();
        let mut buf = [0u8; 200];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], packet(&mac).as_slice());
    }
}
