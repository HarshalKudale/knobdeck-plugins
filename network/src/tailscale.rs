//! Tailscale through its LocalAPI, the interface the `tailscale` CLI and the GUI apps use:
//! `GET /localapi/v0/status`, `PATCH /localapi/v0/prefs` (`WantRunning`, `ExitNodeID`).
//!
//! Where tailscaled listens:
//! - Linux: the Unix socket `/var/run/tailscale/tailscaled.sock`. Changing settings needs root or
//!   the operator (`sudo tailscale set --operator=$USER`, once).
//! - Windows: the named pipe `\\.\pipe\ProtectedPrefix\Administrators\Tailscale\tailscaled`.
//! - macOS: the open-source daemon's `/var/run/tailscaled.socket`; the standalone app's loopback
//!   port and token in `/Library/Tailscale` (`ipnport`, `sameuserproof-<port>`); the App Store
//!   app's `sameuserproof-<port>-<token>` file in its group container.
//!
//! `TAILSCALE_SOCKET` (a Unix socket path, or `host:port`) overrides the search, for tests.

use knobdeck_sdk::{json, Error, Result, Value};
use std::io::{Read, Write};
use std::time::Duration;

const HOST: &str = "local-tailscaled.sock";
const TIMEOUT: Duration = Duration::from_secs(5);

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

/// A connection to tailscaled and the Basic-auth token, if the transport needs one.
fn connect() -> Result<(Box<dyn Stream>, Option<String>)> {
    if let Ok(over) = std::env::var("TAILSCALE_SOCKET") {
        if !over.is_empty() {
            return open_override(&over);
        }
    }
    platform_connect()
}

fn not_running() -> Error {
    Error::msg("Tailscale isn't running on this computer")
}

fn tcp(addr: &str) -> Result<Box<dyn Stream>> {
    let s = std::net::TcpStream::connect(addr).map_err(|_| not_running())?;
    let _ = s.set_read_timeout(Some(TIMEOUT));
    let _ = s.set_write_timeout(Some(TIMEOUT));
    Ok(Box::new(s))
}

#[cfg(unix)]
fn unix(path: &str) -> Result<Box<dyn Stream>> {
    let s = std::os::unix::net::UnixStream::connect(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            Error::msg(format!("No permission to talk to Tailscale ({path})"))
        } else {
            not_running()
        }
    })?;
    let _ = s.set_read_timeout(Some(TIMEOUT));
    let _ = s.set_write_timeout(Some(TIMEOUT));
    Ok(Box::new(s))
}

fn open_override(over: &str) -> Result<(Box<dyn Stream>, Option<String>)> {
    #[cfg(unix)]
    if over.starts_with('/') {
        return Ok((unix(over)?, None));
    }
    Ok((tcp(over)?, None))
}

#[cfg(target_os = "linux")]
fn platform_connect() -> Result<(Box<dyn Stream>, Option<String>)> {
    for p in [
        "/var/run/tailscale/tailscaled.sock",
        "/run/tailscale/tailscaled.sock",
    ] {
        if std::path::Path::new(p).exists() {
            return Ok((unix(p)?, None));
        }
    }
    Err(not_running())
}

#[cfg(windows)]
fn platform_connect() -> Result<(Box<dyn Stream>, Option<String>)> {
    let pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(r"\\.\pipe\ProtectedPrefix\Administrators\Tailscale\tailscaled")
        .map_err(|_| not_running())?;
    Ok((Box::new(pipe), None))
}

#[cfg(target_os = "macos")]
fn platform_connect() -> Result<(Box<dyn Stream>, Option<String>)> {
    // Open-source tailscaled (Homebrew, `tailscaled install-system-daemon`).
    if std::path::Path::new("/var/run/tailscaled.socket").exists() {
        if let Ok(s) = unix("/var/run/tailscaled.socket") {
            return Ok((s, None));
        }
    }
    let (port, token) = mac_port_token().ok_or_else(not_running)?;
    Ok((tcp(&format!("127.0.0.1:{port}"))?, Some(token)))
}

/// The GUI app's loopback port and same-user token.
#[cfg(target_os = "macos")]
fn mac_port_token() -> Option<(u16, String)> {
    // Standalone app (tailscale.com download).
    if let Ok(link) = std::fs::read_link("/Library/Tailscale/ipnport") {
        let port = link.to_string_lossy().to_string();
        if let (Ok(p), Ok(tok)) = (
            port.parse::<u16>(),
            std::fs::read_to_string(format!("/Library/Tailscale/sameuserproof-{port}")),
        ) {
            return Some((p, tok.trim().to_string()));
        }
    }
    // App Store app: ~/Library/Group Containers/*io.tailscale.ipn.macos/sameuserproof-<port>-<token>.
    let home = std::env::var("HOME").ok()?;
    let groups = std::path::Path::new(&home).join("Library/Group Containers");
    for dir in std::fs::read_dir(groups).ok()?.flatten().filter(|d| {
        d.file_name()
            .to_string_lossy()
            .ends_with("io.tailscale.ipn.macos")
    }) {
        for f in std::fs::read_dir(dir.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            if let Some(found) = parse_proof_name(&f.file_name().to_string_lossy()) {
                return Some(found);
            }
        }
    }
    None
}

/// `sameuserproof-51234-abcdef` → (51234, "abcdef").
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn parse_proof_name(name: &str) -> Option<(u16, String)> {
    let rest = name.strip_prefix("sameuserproof-")?;
    let (port, token) = rest.split_once('-')?;
    let port = port.parse().ok()?;
    (!token.is_empty()).then(|| (port, token.to_string()))
}

fn base64(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// One HTTP/1.0 exchange: (status, body). HTTP/1.0 keeps the reply unchunked and closes it.
fn exchange(method: &str, path: &str, body: Option<&Value>) -> Result<(u16, String)> {
    let (mut s, token) = connect()?;
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!("{method} {path} HTTP/1.0\r\nHost: {HOST}\r\nUser-Agent: KnobDeck\r\nContent-Length: {}\r\n", body.len());
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    if let Some(t) = token {
        req.push_str(&format!(
            "Authorization: Basic {}\r\n",
            base64(&format!(":{t}"))
        ));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    s.write_all(req.as_bytes())
        .map_err(|e| Error::msg(format!("Tailscale: {e}")))?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)
        .map_err(|e| Error::msg(format!("Tailscale: {e}")))?;
    parse_response(&raw)
}

pub fn parse_response(raw: &[u8]) -> Result<(u16, String)> {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| Error::msg("Tailscale sent an invalid reply"))?;
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| Error::msg("Tailscale sent an invalid reply"))?;
    let chunked = head.lines().any(|l| {
        l.to_ascii_lowercase().starts_with("transfer-encoding:")
            && l.to_ascii_lowercase().contains("chunked")
    });
    Ok((
        status,
        if chunked {
            dechunk(body)
        } else {
            body.to_string()
        },
    ))
}

fn dechunk(mut body: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size.trim(), 16) else {
            break;
        };
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        body = rest[n..].trim_start_matches("\r\n");
    }
    out
}

fn call(method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    let (status, text) = exchange(method, path, body)?;
    match status {
        200..=299 => serde_json::from_str(&text).map_err(|e| Error::msg(format!("Tailscale sent an invalid reply: {e}"))),
        403 if cfg!(target_os = "linux") => Err(Error::msg("Tailscale lets only root or its operator change settings: run `sudo tailscale set --operator=$USER` once")),
        _ => Err(Error::msg(format!("Tailscale: {}", text.trim().chars().take(200).collect::<String>()))),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Stable node id (what `ExitNodeID` takes).
    pub id: String,
    pub name: String,
    pub online: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    /// "Running", "Stopped", "NeedsLogin", "Starting"…
    pub state: String,
    pub ip: String,
    pub tailnet: String,
    pub exit_node: Option<String>,
    pub exit_nodes: Vec<Node>,
}

fn short_name(peer: &Value) -> String {
    let dns = peer["DNSName"]
        .as_str()
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
        .to_string();
    if dns.is_empty() {
        peer["HostName"].as_str().unwrap_or("?").to_string()
    } else {
        dns
    }
}

pub fn parse_status(v: &Value) -> Status {
    let mut exit_nodes: Vec<Node> = v["Peer"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .filter(|p| p["ExitNodeOption"] == true)
        .map(|p| Node {
            id: p["ID"].as_str().unwrap_or("").to_string(),
            name: short_name(p),
            online: p["Online"] == true,
        })
        .collect();
    exit_nodes.sort_by(|a, b| a.name.cmp(&b.name));
    let in_use = v["ExitNodeStatus"]["ID"]
        .as_str()
        .map(String::from)
        .or_else(|| {
            v["Peer"]
                .as_object()
                .into_iter()
                .flat_map(|m| m.values())
                .find(|p| p["ExitNode"] == true)
                .and_then(|p| p["ID"].as_str().map(String::from))
        });
    Status {
        state: v["BackendState"].as_str().unwrap_or("").to_string(),
        ip: v["Self"]["TailscaleIPs"]
            .as_array()
            .and_then(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .find(|ip| !ip.contains(':'))
                    .or(a.first().and_then(Value::as_str))
            })
            .unwrap_or("")
            .to_string(),
        tailnet: v["CurrentTailnet"]["Name"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        exit_node: in_use.filter(|s| !s.is_empty()),
        exit_nodes,
    }
}

pub fn status() -> Result<Status> {
    Ok(parse_status(&call(
        "GET",
        "/localapi/v0/status?peers=true",
        None,
    )?))
}

pub fn set_running(on: bool) -> Result<()> {
    call(
        "PATCH",
        "/localapi/v0/prefs",
        Some(&json!({"WantRunning": on, "WantRunningSet": true})),
    )
    .map(|_| ())
}

/// Use `id` as the exit node, or none with `""`.
pub fn set_exit_node(id: &str) -> Result<()> {
    call("PATCH", "/localapi/v0/prefs", Some(&json!({"ExitNodeID": id, "ExitNodeIDSet": true, "ExitNodeIP": "", "ExitNodeIPSet": true}))).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies() {
        let (code, body) =
            parse_response(b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"a\":1}")
                .unwrap();
        assert_eq!((code, body.as_str()), (200, "{\"a\":1}"));
        let (_, body) = parse_response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n").unwrap();
        assert_eq!(body, "{\"a\":1}");
        assert!(parse_response(b"garbage").is_err());
    }

    #[test]
    fn status_fields() {
        let v = json!({"BackendState": "Running", "Self": {"TailscaleIPs": ["fd7a:115c::1", "100.64.0.1"]}, "CurrentTailnet": {"Name": "me@example.com"},
            "ExitNodeStatus": {"ID": "nB", "Online": true},
            "Peer": {"k1": {"ID": "nA", "DNSName": "nas.tail1.ts.net.", "ExitNodeOption": true, "Online": true},
                     "k2": {"ID": "nB", "HostName": "vps", "DNSName": "", "ExitNodeOption": true, "Online": false, "ExitNode": true},
                     "k3": {"ID": "nC", "HostName": "phone", "ExitNodeOption": false}}});
        let s = parse_status(&v);
        assert_eq!(
            (s.state.as_str(), s.ip.as_str(), s.exit_node.as_deref()),
            ("Running", "100.64.0.1", Some("nB"))
        );
        assert_eq!(
            s.exit_nodes
                .iter()
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>(),
            ["nas", "vps"]
        );
        assert!(parse_status(&json!({"BackendState": "Stopped"}))
            .exit_node
            .is_none());
    }

    #[test]
    fn helpers() {
        assert_eq!(base64(":tok"), "OnRvaw==");
        assert_eq!(base64(":ab"), "OmFi");
        assert_eq!(
            parse_proof_name("sameuserproof-51234-abc-def"),
            Some((51234, "abc-def".into()))
        );
        assert_eq!(parse_proof_name("other"), None);
    }
}
