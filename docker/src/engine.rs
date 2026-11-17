//! Docker Engine API (also served by Podman) over its local socket: a Unix socket on Linux and
//! macOS (Docker Desktop, rootless Docker, Colima, OrbStack, Podman), the `docker_engine` named
//! pipe on Windows, or `DOCKER_HOST=tcp://…` (plain HTTP). One short HTTP/1.0 request per call,
//! so the reply is never chunked and ends when the engine closes the connection.

use knobdeck_sdk::{Error, Result, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

/// API version every engine since Docker 18.09 / Podman 3 understands.
const API: &str = "v1.40";

#[derive(Clone, Debug, PartialEq)]
pub enum Endpoint {
    Unix(PathBuf),
    Pipe(String),
    Tcp(String),
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Endpoint::Unix(p) => write!(f, "unix://{}", p.display()),
            Endpoint::Pipe(p) => write!(f, "npipe://{p}"),
            Endpoint::Tcp(a) => write!(f, "tcp://{a}"),
        }
    }
}

/// Parse a `DOCKER_HOST`-style address.
pub fn parse_host(s: &str) -> Option<Endpoint> {
    let s = s.trim();
    if let Some(p) = s.strip_prefix("unix://") {
        Some(Endpoint::Unix(PathBuf::from(p)))
    } else if let Some(p) = s.strip_prefix("npipe://") {
        Some(Endpoint::Pipe(p.replace('/', "\\")))
    } else if let Some(a) = s.strip_prefix("tcp://").or_else(|| s.strip_prefix("http://")) {
        Some(Endpoint::Tcp(a.trim_end_matches('/').to_string()))
    } else {
        None
    }
}

/// Where engines usually listen, in the order they are tried.
pub fn candidates() -> Vec<Endpoint> {
    let mut out = vec![];
    for var in ["KNOBDECK_DOCKER_HOST", "DOCKER_HOST"] {
        if let Some(e) = std::env::var(var).ok().as_deref().and_then(parse_host) {
            out.push(e);
        }
    }
    if cfg!(windows) {
        out.push(Endpoint::Pipe(r"\\.\pipe\docker_engine".into()));
        out.push(Endpoint::Pipe(r"\\.\pipe\podman-machine-default".into()));
    } else {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let run = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        out.push(Endpoint::Unix("/var/run/docker.sock".into()));
        if let Some(run) = &run {
            out.push(Endpoint::Unix(run.join("docker.sock"))); // rootless Docker
            out.push(Endpoint::Unix(run.join("podman/podman.sock"))); // podman.socket (user)
        }
        out.push(Endpoint::Unix("/run/podman/podman.sock".into()));
        out.push(Endpoint::Unix(home.join(".docker/run/docker.sock"))); // Docker Desktop (macOS)
        out.push(Endpoint::Unix(home.join(".docker/desktop/docker.sock"))); // Docker Desktop (Linux)
        out.push(Endpoint::Unix(home.join(".orbstack/run/docker.sock")));
        out.push(Endpoint::Unix(home.join(".colima/default/docker.sock")));
        out.push(Endpoint::Unix(home.join(".local/share/containers/podman/machine/podman.sock")));
    }
    out
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

fn connect(ep: &Endpoint) -> std::io::Result<Box<dyn Stream>> {
    match ep {
        #[cfg(unix)]
        Endpoint::Unix(p) => {
            let s = std::os::unix::net::UnixStream::connect(p)?;
            s.set_read_timeout(Some(Duration::from_secs(30)))?;
            Ok(Box::new(s))
        }
        #[cfg(not(unix))]
        Endpoint::Unix(_) => Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Unix sockets need Linux or macOS")),
        // A named pipe opens like a file.
        Endpoint::Pipe(p) => Ok(Box::new(std::fs::OpenOptions::new().read(true).write(true).open(p)?)),
        Endpoint::Tcp(a) => {
            use std::net::ToSocketAddrs;
            let addr = a.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no address"))?;
            let s = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
            s.set_read_timeout(Some(Duration::from_secs(30)))?;
            Ok(Box::new(s))
        }
    }
}

/// Split a raw HTTP reply into status and body.
pub fn parse_reply(raw: &[u8]) -> Result<(u16, Vec<u8>)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(|| Error::msg("The container engine sent an incomplete reply"))?;
    let head = String::from_utf8_lossy(&raw[..end]);
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| Error::msg("The container engine sent an invalid reply"))?;
    let mut body = raw[end + 4..].to_vec();
    if head.lines().any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked") {
        body = dechunk(&body);
    }
    Ok((status, body))
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    while let Some(nl) = b.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&b[..nl]).split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        if size == 0 {
            break;
        }
        let start = nl + 2;
        let stop = (start + size).min(b.len());
        out.extend_from_slice(&b[start..stop]);
        b = &b[(stop + 2).min(b.len())..];
    }
    out
}

pub struct Engine {
    pub endpoint: Endpoint,
}

impl Engine {
    /// The first engine that answers `/_ping`.
    pub fn find() -> Result<Engine> {
        for ep in candidates() {
            let e = Engine { endpoint: ep };
            if e.request("GET", "/_ping").is_ok_and(|(code, _)| code == 200) {
                return Ok(e);
            }
        }
        Err(Error::msg("Docker isn't running (no Docker or Podman engine answered). Start Docker Desktop, the docker service or podman.socket"))
    }

    pub fn request(&self, method: &str, path: &str) -> Result<(u16, Vec<u8>)> {
        let mut s = connect(&self.endpoint).map_err(|e| Error::msg(format!("Could not reach {} ({e})", self.endpoint)))?;
        let host = match &self.endpoint {
            Endpoint::Tcp(a) => a.as_str(),
            _ => "docker",
        };
        let req = format!("{method} /{API}{path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: KnobDeck\r\nContent-Length: 0\r\n\r\n");
        s.write_all(req.as_bytes()).map_err(|e| Error::msg(format!("Container engine request failed: {e}")))?;
        let mut raw = vec![];
        s.read_to_end(&mut raw).map_err(|e| Error::msg(format!("Container engine reply failed: {e}")))?;
        parse_reply(&raw)
    }

    fn json(&self, path: &str) -> Result<Value> {
        let (code, body) = self.request("GET", path)?;
        if code != 200 {
            return Err(api_error(code, &body));
        }
        serde_json::from_slice(&body).map_err(|e| Error::msg(format!("The container engine sent invalid JSON: {e}")))
    }

    pub fn containers(&self) -> Result<Vec<Container>> {
        Ok(self.json("/containers/json?all=1")?.as_array().map(|a| a.iter().map(Container::from_json).collect()).unwrap_or_default())
    }

    /// `start`, `stop`, `restart`, `pause` or `unpause` one container (by name or id).
    pub fn act(&self, verb: &str, container: &str) -> Result<()> {
        let (code, body) = self.request("POST", &format!("/containers/{}/{verb}", encode(container)))?;
        match code {
            200..=299 | 304 => Ok(()), // 304: already in that state
            _ => Err(api_error(code, &body)),
        }
    }
}

fn api_error(code: u16, body: &[u8]) -> Error {
    let msg = serde_json::from_slice::<Value>(body).ok().and_then(|v| v["message"].as_str().map(String::from)).unwrap_or_else(|| String::from_utf8_lossy(body).trim().to_string());
    Error::msg(match code {
        404 => format!("Not found: {msg}"),
        _ if msg.is_empty() => format!("The container engine answered {code}"),
        _ => msg,
    })
}

/// Percent-encode a path segment (container names are `[a-zA-Z0-9_.-]`, ids hex; anything else is escaped).
fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Container {
    pub id: String,
    pub name: String,
    /// `created`, `running`, `paused`, `restarting`, `exited`, `dead`.
    pub state: String,
    /// The engine's own summary: "Up 3 hours (healthy)", "Exited (0) 2 days ago".
    pub status: String,
    pub image: String,
    pub project: Option<String>,
}

impl Container {
    pub fn from_json(v: &Value) -> Container {
        Container {
            id: v["Id"].as_str().unwrap_or("").to_string(),
            name: v["Names"][0].as_str().unwrap_or("").trim_start_matches('/').to_string(),
            state: v["State"].as_str().unwrap_or("").to_lowercase(),
            status: v["Status"].as_str().unwrap_or("").to_string(),
            image: v["Image"].as_str().unwrap_or("").to_string(),
            project: v["Labels"]["com.docker.compose.project"].as_str().or(v["Labels"]["io.podman.compose.project"].as_str()).map(String::from),
        }
    }
    pub fn running(&self) -> bool {
        matches!(self.state.as_str(), "running" | "restarting")
    }
    pub fn health(&self) -> Option<&'static str> {
        ["unhealthy", "healthy", "starting"].into_iter().find(|h| self.status.contains(&format!("({h})")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    #[test]
    fn hosts() {
        assert_eq!(parse_host("unix:///run/x.sock"), Some(Endpoint::Unix("/run/x.sock".into())));
        assert_eq!(parse_host("npipe:////./pipe/docker_engine"), Some(Endpoint::Pipe(r"\\.\pipe\docker_engine".into())));
        assert_eq!(parse_host("tcp://10.0.0.2:2375/"), Some(Endpoint::Tcp("10.0.0.2:2375".into())));
        assert_eq!(parse_host("ssh://x"), None);
    }

    #[test]
    fn replies() {
        let (c, b) = parse_reply(b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\n\r\nOK").unwrap();
        assert_eq!((c, b.as_slice()), (200, b"OK".as_slice()));
        let (c, b) = parse_reply(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n[1,\r\n2\r\n2]\r\n0\r\n\r\n").unwrap();
        assert_eq!((c, b.as_slice()), (200, b"[1,2]".as_slice()));
        assert!(parse_reply(b"garbage").is_err());
        assert_eq!(api_error(404, br#"{"message":"No such container: x"}"#).message(), "Not found: No such container: x");
    }

    #[test]
    fn containers() {
        let c = Container::from_json(&json!({"Id": "abc", "Names": ["/web-1"], "State": "running", "Status": "Up 2 hours (healthy)", "Image": "nginx", "Labels": {"com.docker.compose.project": "site"}}));
        assert_eq!(c.name, "web-1");
        assert!(c.running());
        assert_eq!(c.health(), Some("healthy"));
        assert_eq!(c.project.as_deref(), Some("site"));
        assert_eq!(encode("my app/1"), "my%20app%2F1");
    }
}
