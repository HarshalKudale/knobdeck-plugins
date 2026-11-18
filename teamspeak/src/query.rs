//! TeamSpeak 3 ClientQuery (the ClientQuery plugin that ships with TeamSpeak 3, telnet
//! `127.0.0.1:25639`). Line based: the client greets with a banner ending in `selected
//! schandlerid=N`; each command answers with zero or more data lines, then `error id=0 msg=ok`.
//! Data lines hold records separated by `|`, fields `key=value` separated by spaces, with spaces,
//! pipes and slashes escaped (`\s`, `\p`, `\/`). Since TeamSpeak 3.1 the first command must be
//! `auth apikey=…` (the key is shown in Settings ▸ Addons ▸ ClientQuery ▸ Settings).
//!
//! Commands used: `currentschandlerid` + `use` (the server tab in front), `whoami`,
//! `clientvariable`, `clientupdate client_input_muted|client_output_muted|client_away|
//! client_input_deactivated`, `channelvariable … channel_name`, `clientlist -voice`.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const KEY: &str = "TEAMSPEAK3_CLIENTQUERY_KEY";

#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub host: String,
    pub port: String,
    pub key: String,
}

pub type Record = HashMap<String, String>;

pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('s') => out.push(' '),
            Some('p') => out.push('|'),
            Some('/') => out.push('/'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('a') => out.push('\x07'),
            Some('b') => out.push('\x08'),
            Some('f') => out.push('\x0c'),
            Some('v') => out.push('\x0b'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

pub fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\\' => "\\\\".to_string(),
            '/' => "\\/".to_string(),
            ' ' => "\\s".to_string(),
            '|' => "\\p".to_string(),
            '\n' => "\\n".to_string(),
            '\r' => "\\r".to_string(),
            '\t' => "\\t".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// One data line → its records.
pub fn parse_records(line: &str) -> Vec<Record> {
    line.split('|')
        .map(|rec| {
            rec.split(' ')
                .filter(|f| !f.is_empty())
                .map(|f| match f.split_once('=') {
                    Some((k, v)) => (k.to_string(), unescape(v)),
                    None => (f.to_string(), String::new()),
                })
                .collect()
        })
        .collect()
}

struct Conn {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Conn {
    fn line(&mut self, deadline: Instant) -> Result<String, String> {
        let mut buf = String::new();
        loop {
            match self.reader.read_line(&mut buf) {
                Ok(0) => return Err("TeamSpeak closed the ClientQuery connection".into()),
                Ok(_) if buf.ends_with('\n') => return Ok(buf.trim_matches(['\r', '\n']).to_string()),
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                    if Instant::now() > deadline {
                        return Err("TeamSpeak didn't answer".into());
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
    }

    fn cmd(&mut self, command: &str) -> Result<Vec<Record>, String> {
        self.writer.write_all(format!("{command}\n").as_bytes()).map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut data = vec![];
        loop {
            let line = self.line(deadline)?;
            if line.is_empty() || line.starts_with("notify") || line.starts_with("selected ") {
                continue;
            }
            if let Some(rest) = line.strip_prefix("error ") {
                let r = parse_records(rest).into_iter().next().unwrap_or_default();
                return match r.get("id").map(String::as_str) {
                    Some("0") => Ok(data),
                    _ => Err(r.get("msg").cloned().unwrap_or_else(|| "TeamSpeak reported an error".into())),
                };
            }
            data.extend(parse_records(&line));
        }
    }
}

static CONN: Mutex<Option<(Endpoint, Conn)>> = Mutex::new(None);

fn open(ep: &Endpoint) -> Result<Conn, String> {
    let unreachable = || format!("Can't reach TeamSpeak 3 at {}:{}. Start TeamSpeak 3 and enable its ClientQuery plugin (Settings ▸ Plugins)", ep.host, ep.port);
    let addr = format!("{}:{}", ep.host, ep.port).to_socket_addrs().map_err(|_| unreachable())?.next().ok_or_else(unreachable)?;
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|_| unreachable())?;
    stream.set_read_timeout(Some(Duration::from_millis(200))).ok();
    let mut conn = Conn { reader: BufReader::new(stream.try_clone().map_err(|e| e.to_string())?), writer: stream };
    // Banner: "TS3 Client", "Welcome…", ["Use the "auth" command…",] "selected schandlerid=N".
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match conn.line(deadline) {
            Ok(l) if l.starts_with("selected ") => break,
            Ok(_) => {}
            Err(e) if e == "TeamSpeak didn't answer" => break, // older clients end the banner differently
            Err(e) => return Err(e),
        }
    }
    if ep.key.is_empty() {
        return Err("Enter TeamSpeak 3's ClientQuery API key in the plugin's setup (TeamSpeak 3: Settings ▸ Addons ▸ ClientQuery ▸ Settings)".into());
    }
    conn.cmd(&format!("auth apikey={}", escape(&ep.key))).map_err(|e| format!("TeamSpeak 3 refused the ClientQuery API key ({e}): check it in the plugin's setup"))?;
    Ok(conn)
}

/// Run `commands` in order on the server tab in front, after `currentschandlerid` + `use`.
/// Returns the answer to each.
pub fn run(ep: &Endpoint, commands: &[String]) -> Result<Vec<Vec<Record>>, String> {
    let mut guard = CONN.lock().unwrap();
    for attempt in 0..2 {
        if guard.as_ref().is_none_or(|(e, _)| e != ep) {
            *guard = Some((ep.clone(), open(ep)?));
        }
        let conn = &mut guard.as_mut().unwrap().1;
        let result = (|| {
            let tab = conn.cmd("currentschandlerid")?;
            if let Some(id) = tab.first().and_then(|r| r.get("schandlerid")) {
                conn.cmd(&format!("use schandlerid={id}"))?;
            }
            commands.iter().map(|c| conn.cmd(c)).collect::<Result<Vec<_>, _>>()
        })();
        match result {
            Ok(r) => return Ok(r),
            Err(e) if attempt == 0 && (e.contains("closed") || e.contains("didn't answer") || e.contains("Broken pipe")) => *guard = None,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[derive(Debug, Default, PartialEq)]
pub struct Status {
    pub input_muted: bool,
    pub output_muted: bool,
    pub away: bool,
    pub channel: String,
    pub talking: Vec<String>,
}

fn not_connected(e: String) -> String {
    if e.contains("not connected") {
        "TeamSpeak 3 isn't connected to a server".into()
    } else {
        e
    }
}

/// Our client and channel ids on the server tab in front.
fn whoami(ep: &Endpoint) -> Result<(String, String), String> {
    let r = run(ep, &["whoami".into()]).map_err(not_connected)?;
    let me = r[0].first().cloned().unwrap_or_default();
    Ok((me.get("clid").cloned().unwrap_or_default(), me.get("cid").cloned().unwrap_or_default()))
}

pub fn status(ep: &Endpoint) -> Result<Status, String> {
    let (clid, cid) = whoami(ep)?;
    let r = run(ep, &[format!("clientvariable clid={clid} client_input_muted client_output_muted client_away"), format!("channelvariable cid={cid} channel_name"), "clientlist -voice".into()])
        .map_err(not_connected)?;
    let me = r[0].first().cloned().unwrap_or_default();
    let on = |k: &str| me.get(k).is_some_and(|v| v == "1");
    Ok(Status {
        input_muted: on("client_input_muted"),
        output_muted: on("client_output_muted"),
        away: on("client_away"),
        channel: r[1].first().and_then(|c| c.get("channel_name")).cloned().unwrap_or_default(),
        talking: r[2].iter().filter(|c| c.get("cid") == Some(&cid) && c.get("client_flag_talking").is_some_and(|t| t == "1")).filter_map(|c| c.get("client_nickname").cloned()).collect(),
    })
}

/// `clientupdate <property>=<0|1>`.
pub fn set(ep: &Endpoint, property: &str, on: bool) -> Result<(), String> {
    run(ep, &[format!("clientupdate {property}={}", on as u8)]).map(|_| ()).map_err(not_connected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn escaping() {
        assert_eq!(unescape(r"Raid\sNight\p1\/2"), "Raid Night|1/2");
        assert_eq!(escape("a b|c/d"), r"a\sb\pc\/d");
        let r = parse_records("clid=1 cid=2 client_nickname=A\\sB client_flag_talking=1|clid=3 cid=2 client_nickname=C client_flag_talking=0");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0]["client_nickname"], "A B");
    }

    /// A stand-in TeamSpeak 3 ClientQuery: banner, auth, then the commands `status` and `set` send.
    #[test]
    fn talks_to_a_client_query() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        let server = std::thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let mut w = sock.try_clone().unwrap();
            let r = BufReader::new(sock);
            w.write_all(b"TS3 Client\n\rWelcome to the TeamSpeak 3 ClientQuery interface, type \"help\" for a list of commands and \"help <command>\" for information on a specific command.\n\rUse the \"auth\" command to authenticate yourself. Your API key can be found in the ClientQuery settings.\n\rselected schandlerid=1\n\r").unwrap();
            let mut seen = vec![];
            let mut authed = false;
            for line in r.lines() {
                let line = line.unwrap();
                let line = line.trim_matches(['\r', '\n']).to_string();
                let ok = "error id=0 msg=ok\n\r";
                let reply = if let Some(k) = line.strip_prefix("auth apikey=") {
                    authed = k == "ABCD-1234";
                    if authed {
                        ok.to_string()
                    } else {
                        "error id=256 msg=invalid\\sapikey\n\r".to_string()
                    }
                } else if !authed {
                    "error id=1796 msg=currently\\snot\\spossible\n\r".to_string()
                } else if line == "currentschandlerid" {
                    format!("schandlerid=1\n\r{ok}")
                } else if line == "whoami" {
                    format!("clid=4 cid=7\n\r{ok}")
                } else if line.starts_with("clientvariable clid=4") {
                    format!("clid=4 client_input_muted=1 client_output_muted=0 client_away=0\n\r{ok}")
                } else if line == "channelvariable cid=7 channel_name" {
                    format!("cid=7 channel_name=Raid\\sNight\n\r{ok}")
                } else if line == "clientlist -voice" {
                    format!("clid=4 cid=7 client_database_id=2 client_nickname=me client_type=0 client_flag_talking=0|clid=5 cid=7 client_database_id=3 client_nickname=Alice client_type=0 client_flag_talking=1|clid=6 cid=1 client_database_id=4 client_nickname=Bob client_type=0 client_flag_talking=1\n\r{ok}")
                } else {
                    ok.to_string()
                };
                seen.push(line);
                if w.write_all(reply.as_bytes()).is_err() {
                    break;
                }
            }
            seen
        });
        let bad = Endpoint { host: "127.0.0.1".into(), port: "1".into(), key: "x".into() };
        assert!(status(&bad).unwrap_err().starts_with("Can't reach TeamSpeak 3"));
        let ep = Endpoint { host: "127.0.0.1".into(), port, key: "ABCD-1234".into() };
        let s = status(&ep).unwrap();
        assert_eq!(s, Status { input_muted: true, output_muted: false, away: false, channel: "Raid Night".into(), talking: vec!["Alice".into()] });
        set(&ep, "client_input_muted", false).unwrap();
        *CONN.lock().unwrap() = None;
        let seen = server.join().unwrap();
        assert_eq!(seen[0], "auth apikey=ABCD-1234");
        assert!(seen.contains(&"use schandlerid=1".to_string()));
        assert_eq!(seen.last().unwrap(), "clientupdate client_input_muted=0");
    }
}
