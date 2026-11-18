//! Neovim's msgpack-RPC over its `--listen` address: a Unix socket on Linux and macOS, a named
//! pipe on Windows, or `host:port`. One short connection per call; only the API's own requests
//! (`nvim_command`, `nvim_exec_lua`, `nvim_get_mode`), never typed keys.

use knobdeck_sdk::{Error, Result};
use rmpv::Value;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const TIMEOUT: Duration = Duration::from_secs(4);

#[derive(Clone, Debug, PartialEq)]
pub enum Addr {
    Unix(PathBuf),
    Pipe(String),
    Tcp(String),
}

impl std::fmt::Display for Addr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Addr::Unix(p) => write!(f, "{}", p.display()),
            Addr::Pipe(p) | Addr::Tcp(p) => write!(f, "{p}"),
        }
    }
}

/// An address as `--listen` / `v:servername` writes it.
pub fn parse_addr(s: &str) -> Option<Addr> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower.starts_with(r"\\.\pipe\") || lower.starts_with("//./pipe/") {
        return Some(Addr::Pipe(s.replace('/', "\\")));
    }
    // host:port (IPv4, name or [IPv6]) with a numeric port and no path separator.
    if let Some((host, port)) = s.rsplit_once(':') {
        if !host.is_empty() && !host.contains('/') && !host.contains('\\') && port.parse::<u16>().is_ok() && !(host.len() == 1 && cfg!(windows)) {
            return Some(Addr::Tcp(s.to_string()));
        }
    }
    if cfg!(windows) {
        // A bare name means a pipe (`nvim --listen mypipe` makes \\.\pipe\mypipe).
        return Some(Addr::Pipe(if s.contains('\\') { s.to_string() } else { format!(r"\\.\pipe\{s}") }));
    }
    Some(Addr::Unix(PathBuf::from(s)))
}

/// `nvim.<pid>.<n>`: the name Neovim gives its default server.
pub fn is_default_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("nvim.") else { return false };
    let mut parts = rest.split('.');
    let (Some(pid), Some(n), None) = (parts.next(), parts.next(), parts.next()) else { return false };
    !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
}

/// The pid in a default server name.
pub fn pid_of(addr: &Addr) -> Option<u32> {
    let s = addr.to_string();
    let name = s.rsplit(['/', '\\']).next()?;
    is_default_name(name).then(|| name.split('.').nth(1)?.parse().ok()).flatten()
}

/// Every default server socket / pipe that exists, newest first (not yet checked for life).
pub fn discover() -> Vec<Addr> {
    let mut found: Vec<(SystemTime, Addr)> = vec![];
    if cfg!(windows) {
        if let Ok(rd) = std::fs::read_dir(r"\\.\pipe\") {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if is_default_name(&name) {
                    found.push((SystemTime::UNIX_EPOCH, Addr::Pipe(format!(r"\\.\pipe\{name}"))));
                }
            }
        }
        // Pipes have no times: the highest pid is a fair guess at the newest.
        found.sort_by_key(|(_, a)| std::cmp::Reverse(pid_of(a)));
        return found.into_iter().map(|(_, a)| a).collect();
    }
    // stdpath("run"): $XDG_RUNTIME_DIR, else a private folder under $TMPDIR/nvim.$USER/ (macOS, and
    // Linux without XDG_RUNTIME_DIR).
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
    let mut dirs: Vec<PathBuf> = vec![];
    if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
        dirs.push(run.into());
    }
    let mut tmps: Vec<PathBuf> = vec![];
    if let Some(t) = std::env::var_os("TMPDIR").filter(|v| !v.is_empty()) {
        tmps.push(t.into());
    }
    tmps.push("/tmp".into());
    for t in tmps {
        let base = t.join(format!("nvim.{user}"));
        dirs.push(base.clone());
        if let Ok(rd) = std::fs::read_dir(&base) {
            dirs.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            if !is_default_name(&e.file_name().to_string_lossy()) {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            #[cfg(unix)]
            {
                use std::os::unix::fs::FileTypeExt;
                if !meta.file_type().is_socket() {
                    continue;
                }
            }
            let addr = Addr::Unix(e.path());
            if !found.iter().any(|(_, a)| *a == addr) {
                found.push((meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), addr));
            }
        }
    }
    found.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    found.into_iter().map(|(_, a)| a).collect()
}

trait Stream: Read + Write {}
impl<T: Read + Write> Stream for T {}

fn connect(addr: &Addr) -> std::io::Result<Box<dyn Stream>> {
    match addr {
        #[cfg(unix)]
        Addr::Unix(p) => {
            let s = std::os::unix::net::UnixStream::connect(p)?;
            s.set_read_timeout(Some(TIMEOUT))?;
            s.set_write_timeout(Some(TIMEOUT))?;
            Ok(Box::new(s))
        }
        #[cfg(not(unix))]
        Addr::Unix(_) => Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "Unix sockets need Linux or macOS")),
        // A named pipe opens like a file.
        Addr::Pipe(p) => Ok(Box::new(std::fs::OpenOptions::new().read(true).write(true).open(p)?)),
        Addr::Tcp(a) => {
            use std::net::ToSocketAddrs;
            let sa = a.to_socket_addrs()?.next().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no address"))?;
            let s = std::net::TcpStream::connect_timeout(&sa, Duration::from_secs(2))?;
            s.set_read_timeout(Some(TIMEOUT))?;
            s.set_write_timeout(Some(TIMEOUT))?;
            Ok(Box::new(s))
        }
    }
}

/// One connection to a Neovim.
pub struct Client {
    stream: BufReader<Box<dyn Stream>>,
    next_id: u32,
}

/// "Vim:E492: Not an editor command: foo" → "E492: Not an editor command: foo".
pub fn clean_error(e: &Value) -> String {
    let msg = match e {
        Value::Array(a) => a.get(1).and_then(|m| m.as_str()).map(String::from).unwrap_or_else(|| e.to_string()),
        Value::String(s) => s.as_str().unwrap_or("").to_string(),
        other => other.to_string(),
    };
    let msg = msg.strip_prefix("Vim:").unwrap_or(&msg);
    msg.lines().find(|l| !l.trim().is_empty()).unwrap_or(msg).trim().to_string()
}

impl Client {
    pub fn open(addr: &Addr) -> Result<Client> {
        let s = connect(addr).map_err(|e| Error::msg(format!("Can't reach Neovim at {addr}: {e}")))?;
        Ok(Client { stream: BufReader::new(s), next_id: 1 })
    }

    pub fn call(&mut self, method: &str, args: Vec<Value>) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let req = Value::Array(vec![Value::from(0), Value::from(id), Value::from(method), Value::Array(args)]);
        let mut buf = vec![];
        rmpv::encode::write_value(&mut buf, &req).map_err(|e| Error::msg(e.to_string()))?;
        let w = self.stream.get_mut();
        w.write_all(&buf).and_then(|_| w.flush()).map_err(|e| Error::msg(format!("Neovim closed the connection: {e}")))?;
        loop {
            let msg = rmpv::decode::read_value(&mut self.stream).map_err(|e| Error::msg(format!("Neovim didn't answer: {e}")))?;
            let Value::Array(parts) = msg else { continue };
            // [1, id, error, result]; notifications ([2, …]) are skipped.
            if parts.len() == 4 && parts[0].as_u64() == Some(1) && parts[1].as_u64() == Some(id as u64) {
                let mut it = parts.into_iter().skip(2);
                let (err, res) = (it.next().unwrap_or(Value::Nil), it.next().unwrap_or(Value::Nil));
                if !err.is_nil() {
                    return Err(Error::msg(clean_error(&err)));
                }
                return Ok(res);
            }
        }
    }

    pub fn command(&mut self, cmd: &str) -> Result<()> {
        self.call("nvim_command", vec![Value::from(cmd)]).map(|_| ())
    }

    pub fn lua(&mut self, code: &str, args: Vec<Value>) -> Result<Value> {
        self.call("nvim_exec_lua", vec![Value::from(code), Value::Array(args)])
    }

    /// `(mode, blocking)`. Answered at once even while Neovim waits for input.
    pub fn mode(&mut self) -> Result<(String, bool)> {
        let v = self.call("nvim_get_mode", vec![])?;
        let get = |k: &str| v.as_map().and_then(|m| m.iter().find(|(key, _)| key.as_str() == Some(k)).map(|(_, v)| v.clone()));
        Ok((get("mode").and_then(|m| m.as_str().map(String::from)).unwrap_or_default(), get("blocking").and_then(|b| b.as_bool()).unwrap_or(false)))
    }

    /// Fails when Neovim waits at a prompt (other requests would wait with it).
    pub fn ready(&mut self) -> Result<()> {
        match self.mode()? {
            (_, true) => Err(Error::msg("Neovim is waiting for input (a prompt or a pending key)")),
            _ => Ok(()),
        }
    }
}

/// `nvim_get_mode`'s short mode as the name Neovim's own mode line shows.
pub fn mode_name(mode: &str) -> &'static str {
    match mode {
        "n" | "niI" | "niR" | "niV" | "nt" | "ntT" => "NORMAL",
        m if m.starts_with("no") => "PENDING",
        "v" | "vs" => "VISUAL",
        "V" | "Vs" => "V-LINE",
        "\u{16}" | "\u{16}s" => "V-BLOCK",
        "s" => "SELECT",
        "S" => "S-LINE",
        "\u{13}" => "S-BLOCK",
        m if m.starts_with('i') => "INSERT",
        "Rv" | "Rvc" | "Rvx" => "V-REPLACE",
        m if m.starts_with('R') => "REPLACE",
        m if m.starts_with('c') => "COMMAND",
        "r" | "rm" | "r?" => "PROMPT",
        "!" => "SHELL",
        "t" => "TERMINAL",
        _ => "NORMAL",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(is_default_name("nvim.12345.0"));
        assert!(!is_default_name("nvim.12345"));
        assert!(!is_default_name("nvim.abc.0"));
        assert!(!is_default_name("nvimx.1.0"));
        assert_eq!(pid_of(&Addr::Unix("/run/user/1000/nvim.4242.0".into())), Some(4242));
        assert_eq!(pid_of(&Addr::Pipe(r"\\.\pipe\nvim.77.0".into())), Some(77));
        assert_eq!(pid_of(&Addr::Unix("/tmp/my.sock".into())), None);
    }

    #[test]
    fn addresses() {
        assert_eq!(parse_addr(r"\\.\pipe\nvim.1.0"), Some(Addr::Pipe(r"\\.\pipe\nvim.1.0".into())));
        assert_eq!(parse_addr("127.0.0.1:6666"), Some(Addr::Tcp("127.0.0.1:6666".into())));
        assert_eq!(parse_addr("localhost:6666"), Some(Addr::Tcp("localhost:6666".into())));
        assert_eq!(parse_addr("  "), None);
        #[cfg(unix)]
        assert_eq!(parse_addr("/tmp/nvim.sock"), Some(Addr::Unix("/tmp/nvim.sock".into())));
    }

    #[test]
    fn modes() {
        assert_eq!(mode_name("n"), "NORMAL");
        assert_eq!(mode_name("i"), "INSERT");
        assert_eq!(mode_name("\u{16}"), "V-BLOCK");
        assert_eq!(mode_name("no"), "PENDING");
        assert_eq!(mode_name("c"), "COMMAND");
        assert_eq!(mode_name("Rv"), "V-REPLACE");
    }

    #[test]
    fn errors() {
        let e = Value::Array(vec![Value::from(0), Value::from("Vim:E492: Not an editor command: foo")]);
        assert_eq!(clean_error(&e), "E492: Not an editor command: foo");
    }

    /// A fake Neovim on a socket: answers after an unrelated notification.
    #[cfg(unix)]
    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("kd-nvim-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("nvim.1.0");
        let _ = std::fs::remove_file(&path);
        let l = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let req = rmpv::decode::read_value(&mut s).unwrap();
            let id = req[1].clone();
            assert_eq!(req[2].as_str(), Some("nvim_get_mode"));
            let note = Value::Array(vec![Value::from(2), Value::from("redraw"), Value::Array(vec![])]);
            let res = Value::Array(vec![Value::from(1), id, Value::Nil, Value::Map(vec![(Value::from("mode"), Value::from("i")), (Value::from("blocking"), Value::from(false))])]);
            rmpv::encode::write_value(&mut s, &note).unwrap();
            rmpv::encode::write_value(&mut s, &res).unwrap();
        });
        let mut c = Client::open(&Addr::Unix(path.clone())).unwrap();
        assert_eq!(c.mode().unwrap(), ("i".to_string(), false));
        t.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
