//! REAPER's web browser interface (Preferences ▸ Control/OSC/web ▸ Web browser interface): a GET to
//! `/_/CMD;CMD;…` runs the commands in order and answers one tab-separated line per reply. Shared
//! by the plugin and its setup program.
//!
//! The commands used (documented in the `main.js` REAPER ships with its web pages):
//! - `TRANSPORT` → `TRANSPORT \t playstate \t position_seconds \t isRepeatOn \t position_string \t position_string_beats`
//!   (playstate 0 stopped, 1 playing, 2 paused, 5 recording, 6 record paused);
//! - `GET/REPEAT` → `GET/REPEAT \t 0|1`; `SET/REPEAT/-1` toggles it;
//! - `TRACK` → one `TRACK \t number \t name \t flags \t volume \t pan \t …` line per track (0 is the
//!   master; flags &2 selected, &8 muted, &16 soloed, &32 solo-in-place, &64 record armed; volume 1
//!   is 0 dB; pan −1…1);
//! - `SET/TRACK/n/VOL/v` (absolute gain), `…/PAN/p` (absolute; a leading `-` or `+` makes it
//!   relative), `…/MUTE|SOLO|RECARM|SEL/0|1`;
//! - `MARKER` → `MARKER_LIST`, `MARKER \t name \t id \t position [\t color]`…, `MARKER_LIST_END`;
//! - `SET/POS_STR/m<id>` moves the edit cursor to a marker;
//! - a numeric or named (`_SWS_…`, `_RS…`) command id runs that action; `GET/<id>` →
//!   `CMDSTATE \t <id> \t state` (−1 no on/off state, 0 off, 1 on).

use base64::Engine;
use knobdeck_sdk::{Error, Result};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(3);
/// Appended to every write, so its reply carries the new state.
pub const STATE_CMDS: &str = "TRANSPORT;GET/REPEAT;TRACK";

/// Where the web interface listens, and its optional login (Preferences ▸ Control/OSC/web ▸
/// Web browser interface ▸ Username:password).
#[derive(Clone, Debug)]
pub struct Conn {
    pub base: String,
    pub user: String,
    pub password: String,
}

impl Conn {
    pub fn new(host: &str, port: &str, user: &str, password: &str) -> Self {
        Conn { base: format!("http://{}:{}", host.trim(), port.trim()), user: user.to_string(), password: password.to_string() }
    }

    /// Run `cmds` (`;`-separated) and return the reply's text.
    pub fn send(&self, cmds: &str) -> Result<String> {
        let mut req = ureq::get(&format!("{}/_/{cmds}", self.base)).timeout(TIMEOUT);
        if !self.user.is_empty() || !self.password.is_empty() {
            let token = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.user, self.password));
            req = req.set("Authorization", &format!("Basic {token}"));
        }
        match req.call() {
            Ok(r) => r.into_string().map_err(|e| Error::msg(format!("REAPER sent an unreadable reply: {e}"))),
            Err(ureq::Error::Status(401, _)) => Err(Error::msg("REAPER rejected the username / password (check them in KnobDeck setup)")),
            Err(ureq::Error::Status(404, _)) => Err(Error::msg(format!("{} is not REAPER's web interface (HTTP 404)", self.base))),
            Err(ureq::Error::Status(code, _)) => Err(Error::msg(format!("REAPER answered HTTP {code}"))),
            Err(e) => Err(Error::msg(format!(
                "REAPER is not reachable at {}: open REAPER and enable Preferences ▸ Control/OSC/web ▸ Web browser interface ({e})",
                self.base
            ))),
        }
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let s = Snapshot::parse(&self.send(STATE_CMDS)?);
        if s.transport.is_none() {
            return Err(Error::msg(format!("{} did not answer like REAPER's web interface", self.base)));
        }
        Ok(s)
    }

    /// Run `cmds`, then read the state in the same request.
    pub fn write(&self, cmds: &str) -> Result<Snapshot> {
        Ok(Snapshot::parse(&self.send(&format!("{cmds};{STATE_CMDS}"))?))
    }

    pub fn markers(&self) -> Result<Vec<Marker>> {
        Ok(parse_markers(&self.send("MARKER")?))
    }

    /// An action's on/off state (`None` when it has none).
    pub fn command_state(&self, id: &str) -> Result<Option<bool>> {
        let reply = self.send(&format!("GET/{id}"))?;
        Ok(reply.lines().map(fields).find(|f| f.first() == Some(&"CMDSTATE")).and_then(|f| f.get(2)?.trim().parse::<i64>().ok()).filter(|s| *s >= 0).map(|s| s > 0))
    }
}

fn fields(line: &str) -> Vec<&str> {
    line.trim_end_matches('\r').split('\t').collect()
}

/// Names come with tabs, newlines and backslashes escaped.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub enum Play {
    Stopped,
    Playing,
    Paused,
    Recording,
    RecordPaused,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transport {
    pub play: Play,
    pub seconds: f64,
    pub position: String,
    pub beats: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub number: u32,
    pub name: String,
    pub flags: u32,
    /// Gain: 1 is 0 dB.
    pub volume: f64,
    /// −1 (left) … 1 (right).
    pub pan: f64,
}

impl Track {
    pub fn selected(&self) -> bool {
        self.flags & 2 != 0
    }
    pub fn muted(&self) -> bool {
        self.flags & 8 != 0
    }
    pub fn soloed(&self) -> bool {
        self.flags & (16 | 32) != 0
    }
    pub fn armed(&self) -> bool {
        self.flags & 64 != 0
    }
    pub fn db(&self) -> f64 {
        gain_to_db(self.volume)
    }
    /// "3: Vocals"; the master track is "Master", an unnamed track "Track 3".
    pub fn title(&self) -> String {
        match (self.number, self.name.trim()) {
            (0, _) => "Master".into(),
            (n, "") => format!("Track {n}"),
            (n, name) => format!("{n}: {name}"),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub transport: Option<Transport>,
    pub repeat: bool,
    pub tracks: Vec<Track>,
}

impl Snapshot {
    pub fn parse(reply: &str) -> Snapshot {
        let mut s = Snapshot::default();
        for line in reply.lines() {
            let f = fields(line);
            match f[0] {
                "TRANSPORT" if f.len() >= 5 => {
                    let play = match f[1].trim().parse::<i64>().unwrap_or(0) {
                        1 => Play::Playing,
                        2 => Play::Paused,
                        5 => Play::Recording,
                        6 => Play::RecordPaused,
                        _ => Play::Stopped,
                    };
                    s.transport = Some(Transport { play, seconds: f[2].parse().unwrap_or(0.0), position: f[4].to_string(), beats: f.get(5).unwrap_or(&"").to_string() });
                }
                "GET/REPEAT" if f.len() >= 2 => s.repeat = f[f.len() - 1].trim() == "1",
                "TRACK" if f.len() >= 6 => {
                    let Ok(number) = f[1].parse() else { continue };
                    s.tracks.push(Track { number, name: unescape(f[2]), flags: f[3].parse().unwrap_or(0), volume: f[4].parse().unwrap_or(1.0), pan: f[5].parse().unwrap_or(0.0) });
                }
                _ => {}
            }
        }
        s
    }

    pub fn track(&self, number: u32) -> Option<&Track> {
        self.tracks.iter().find(|t| t.number == number)
    }

    pub fn selected(&self) -> Vec<&Track> {
        self.tracks.iter().filter(|t| t.selected()).collect()
    }

    /// The tracks a key targets: `"selected"` (every selected track) or a track number.
    pub fn targets(&self, pick: &str) -> Result<Vec<&Track>> {
        let pick = pick.trim();
        if pick.is_empty() || pick == "selected" {
            let sel = self.selected();
            if sel.is_empty() {
                return Err(Error::msg("No track is selected in REAPER"));
            }
            return Ok(sel);
        }
        let n: u32 = pick.parse().map_err(|_| Error::msg("Choose a track"))?;
        self.track(n).map(|t| vec![t]).ok_or_else(|| Error::msg(format!("REAPER has no track {n}")))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub name: String,
    pub id: u32,
    pub seconds: f64,
}

pub fn parse_markers(reply: &str) -> Vec<Marker> {
    reply
        .lines()
        .map(fields)
        .filter(|f| f[0] == "MARKER" && f.len() >= 4)
        .filter_map(|f| Some(Marker { name: unescape(f[1]), id: f[2].parse().ok()?, seconds: f[3].parse().unwrap_or(0.0) }))
        .collect()
}

/// Bottom of the volume dial: at or below it the track is silent (−inf).
pub const MIN_DB: f64 = -60.0;
pub const MAX_DB: f64 = 12.0;

pub fn gain_to_db(gain: f64) -> f64 {
    if gain <= 0.0 {
        MIN_DB
    } else {
        (20.0 * gain.log10()).clamp(MIN_DB, MAX_DB)
    }
}

pub fn db_to_gain(db: f64) -> f64 {
    if db <= MIN_DB {
        0.0
    } else {
        10f64.powf(db.min(MAX_DB) / 20.0)
    }
}

/// Set a track's volume (dB on the dial, sent as gain: an unsigned value is absolute).
pub fn vol_cmd(track: u32, db: f64) -> String {
    format!("SET/TRACK/{track}/VOL/{:.6}", db_to_gain(db))
}

/// Set a track's pan absolutely (−1…1). A value with a leading `-` is read as relative, so a pan
/// to the left is "centre, then move left by that much" within the same request.
pub fn pan_cmd(track: u32, pan: f64) -> String {
    let pan = pan.clamp(-1.0, 1.0);
    if pan >= 0.0 {
        format!("SET/TRACK/{track}/PAN/{pan:.4}")
    } else {
        format!("SET/TRACK/{track}/PAN/0;SET/TRACK/{track}/PAN/-{:.4}", -pan)
    }
}

/// A command id REAPER can run: a number, or a named id (`_SWS_SAVESEL`, `_RS…`, `_Script…`).
pub fn valid_command_id(id: &str) -> bool {
    let id = id.trim();
    match id.strip_prefix('_') {
        Some(rest) => !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
        None => !id.is_empty() && id.len() <= 6 && id.chars().all(|c| c.is_ascii_digit()),
    }
}

#[cfg(test)]
pub mod mock {
    //! A small stand-in for REAPER's web interface: answers `/_/…` in REAPER's reply format and
    //! keeps the state the commands change.
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    pub struct State {
        pub play: i64,
        pub repeat: bool,
        /// (name, flags, volume, pan); index 0 is the master.
        pub tracks: Vec<(String, u32, f64, f64)>,
        pub markers: Vec<(String, u32, f64)>,
        pub position: f64,
        /// Action ids run, in order.
        pub ran: Vec<String>,
        pub cmdstate: Vec<(String, i64)>,
        /// Every request path received.
        pub requests: Vec<String>,
        pub auth: Option<String>,
    }

    pub fn start(state: State) -> (Conn, Arc<Mutex<State>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let st = Arc::new(Mutex::new(state));
        let shared = st.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    continue;
                }
                let mut auth = None;
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.strip_prefix("Authorization: Basic ").or_else(|| h.strip_prefix("authorization: Basic ")) {
                        auth = Some(v.trim().to_string());
                    }
                }
                let path = first.split_whitespace().nth(1).unwrap_or("").to_string();
                let mut s = shared.lock().unwrap();
                let (status, body) = if s.auth.is_some() && s.auth != auth {
                    ("401 Unauthorized", String::new())
                } else if let Some(cmds) = path.strip_prefix("/_/") {
                    s.requests.push(cmds.to_string());
                    ("200 OK", cmds.split(';').filter(|c| !c.is_empty()).map(|c| run(&mut s, c)).collect::<String>())
                } else {
                    ("404 Not Found", String::new())
                };
                drop(s);
                let mut out = stream;
                let _ = write!(out, "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (Conn::new("127.0.0.1", &port.to_string(), "", ""), st)
    }

    fn track_line(n: usize, t: &(String, u32, f64, f64)) -> String {
        format!("TRACK\t{n}\t{}\t{}\t{:.6}\t{:.6}\t-1500\t-1500\t1.000000\t0\t0\t0\t1\t0\n", t.0.replace('\t', "\\t"), t.1, t.2, t.3)
    }

    fn run(s: &mut State, cmd: &str) -> String {
        let p: Vec<&str> = cmd.split('/').collect();
        match p.as_slice() {
            ["TRANSPORT"] => {
                let pos = format!("{}:{:06.3}", (s.position / 60.0) as u64, s.position % 60.0);
                format!("TRANSPORT\t{}\t{:.6}\t{}\t{pos}\t1.1.00\n", s.play, s.position, s.repeat as u8)
            }
            ["GET", "REPEAT"] => format!("GET/REPEAT\t{}\n", s.repeat as u8),
            ["SET", "REPEAT", v] => {
                s.repeat = if v.starts_with('-') { !s.repeat } else { *v == "1" };
                String::new()
            }
            ["NTRACK"] => format!("NTRACK\t{}\n", s.tracks.len().saturating_sub(1)),
            ["TRACK"] => s.tracks.iter().enumerate().map(|(n, t)| track_line(n, t)).collect(),
            ["MARKER"] => {
                let mut out = String::from("MARKER_LIST\n");
                for m in &s.markers {
                    out += &format!("MARKER\t{}\t{}\t{:.6}\t0\n", m.0, m.1, m.2);
                }
                out + "MARKER_LIST_END\n"
            }
            ["SET", "POS_STR", v] => {
                if let Some(m) = v.strip_prefix('m').and_then(|id| s.markers.iter().find(|m| m.1.to_string() == id)) {
                    s.position = m.2;
                }
                String::new()
            }
            ["SET", "TRACK", n, what, v] => {
                let Some(t) = n.parse::<usize>().ok().and_then(|n| s.tracks.get_mut(n)) else { return String::new() };
                let bit = |w: &str| match w {
                    "MUTE" => 8,
                    "SOLO" => 16,
                    "RECARM" => 64,
                    "SEL" => 2,
                    _ => 0,
                };
                match *what {
                    "VOL" => t.2 = v.parse().unwrap(),
                    "PAN" => {
                        let x: f64 = v.parse().unwrap();
                        t.3 = if v.starts_with(['+', '-']) { (t.3 + x).clamp(-1.0, 1.0) } else { x };
                    }
                    w if bit(w) != 0 => {
                        let on = if v.starts_with('-') { t.1 & bit(w) == 0 } else { *v == "1" };
                        t.1 = if on { t.1 | bit(w) } else { t.1 & !bit(w) };
                    }
                    _ => {}
                }
                String::new()
            }
            ["GET", id] => {
                let st = s.cmdstate.iter().find(|(c, _)| c == id).map(|(_, v)| *v).unwrap_or(-1);
                format!("CMDSTATE\t{id}\t{st}\n")
            }
            [id] if valid_command_id(id) => {
                s.ran.push(id.to_string());
                match *id {
                    "1007" => s.play = 1,
                    "1008" | "40073" => s.play = if s.play == 1 { 2 } else { 1 },
                    "1013" => s.play = 5,
                    "1016" => s.play = 0,
                    "40297" => s.tracks.iter_mut().for_each(|t| t.1 &= !2),
                    _ => {}
                }
                String::new()
            }
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> mock::State {
        mock::State {
            play: 0,
            tracks: vec![("MASTER".into(), 0, 1.0, 0.0), ("Vocals".into(), 2, 0.5, 0.0), ("Gtr\tL".into(), 8 | 64, 1.0, -0.5), ("".into(), 0, 0.0, 0.25)],
            markers: vec![("Verse".into(), 1, 10.0), ("Chorus".into(), 2, 30.5)],
            cmdstate: vec![("40364".into(), 1), ("_SWS_ABOUT".into(), -1)],
            ..Default::default()
        }
    }

    #[test]
    fn reads_state() {
        let (c, _) = mock::start(session());
        let s = c.snapshot().unwrap();
        assert_eq!(s.transport.as_ref().unwrap().play, Play::Stopped);
        assert_eq!(s.tracks.len(), 4);
        assert_eq!(s.tracks[2].name, "Gtr\tL");
        assert!(s.tracks[2].muted() && s.tracks[2].armed() && !s.tracks[2].soloed());
        assert!((s.tracks[1].db() - -6.0206).abs() < 0.01);
        assert_eq!(s.tracks[3].db(), MIN_DB);
        assert_eq!(s.tracks[0].title(), "Master");
        assert_eq!(s.tracks[3].title(), "Track 3");
        assert_eq!(s.targets("selected").unwrap().iter().map(|t| t.number).collect::<Vec<_>>(), vec![1]);
        assert_eq!(s.targets("2").unwrap()[0].name, "Gtr\tL");
        assert!(s.targets("9").is_err());
    }

    #[test]
    fn writes_and_reads_back() {
        let (c, st) = mock::start(session());
        let s = c.write(&vol_cmd(1, 0.0)).unwrap();
        assert!((s.tracks[1].volume - 1.0).abs() < 1e-6);
        let s = c.write(&pan_cmd(1, -0.3)).unwrap();
        assert!((s.tracks[1].pan - -0.3).abs() < 1e-4);
        let s = c.write(&pan_cmd(2, 0.4)).unwrap();
        assert!((s.tracks[2].pan - 0.4).abs() < 1e-4);
        let s = c.write("SET/TRACK/2/MUTE/0;SET/TRACK/1/SOLO/1").unwrap();
        assert!(!s.tracks[2].muted() && s.tracks[1].soloed());
        let s = c.write("1007").unwrap();
        assert_eq!(s.transport.unwrap().play, Play::Playing);
        let s = c.write("SET/REPEAT/-1").unwrap();
        assert!(s.repeat);
        c.write("_SWS_ABOUT").unwrap();
        assert_eq!(st.lock().unwrap().ran, vec!["1007", "_SWS_ABOUT"]);
        assert_eq!(c.command_state("40364").unwrap(), Some(true));
        assert_eq!(c.command_state("_SWS_ABOUT").unwrap(), None);
    }

    #[test]
    fn markers_and_positions() {
        let (c, st) = mock::start(session());
        let m = c.markers().unwrap();
        assert_eq!(m.iter().map(|m| (m.name.as_str(), m.id)).collect::<Vec<_>>(), vec![("Verse", 1), ("Chorus", 2)]);
        let s = c.write("SET/POS_STR/m2").unwrap();
        assert_eq!(s.transport.unwrap().seconds, 30.5);
        assert_eq!(st.lock().unwrap().position, 30.5);
    }

    #[test]
    fn auth_and_errors() {
        let mut state = session();
        state.auth = Some(base64::engine::general_purpose::STANDARD.encode("me:pw"));
        let (c, _) = mock::start(state);
        assert!(c.snapshot().unwrap_err().message().contains("username"));
        let ok = Conn { user: "me".into(), password: "pw".into(), ..c };
        assert!(ok.snapshot().is_ok());
        let dead = Conn::new("127.0.0.1", "1", "", "");
        assert!(dead.snapshot().unwrap_err().message().contains("Web browser interface"));
    }

    #[test]
    fn conversions() {
        assert_eq!(db_to_gain(MIN_DB), 0.0);
        assert!((db_to_gain(-6.0206) - 0.5).abs() < 1e-4);
        assert!((gain_to_db(db_to_gain(3.0)) - 3.0).abs() < 1e-9);
        assert_eq!(vol_cmd(3, 0.0), "SET/TRACK/3/VOL/1.000000");
        assert_eq!(pan_cmd(1, -2.0), "SET/TRACK/1/PAN/0;SET/TRACK/1/PAN/-1.0000");
        assert!(valid_command_id("40044") && valid_command_id("_SWS_SAVESEL") && valid_command_id(" _RS7d3c_abc "));
        assert!(!valid_command_id("") && !valid_command_id("_") && !valid_command_id("1;2") && !valid_command_id("SET/POS/1") && !valid_command_id("_a/b"));
    }

    #[test]
    fn parses_real_shaped_replies() {
        // A reply as REAPER sends it: CRLF-free lines, trailing fields, unknown lines ignored.
        let s = Snapshot::parse("TRANSPORT\t5\t12.345000\t1\t0:12.345\t7.2.36\nGET/REPEAT\t1\nTRACK\t0\tMASTER\t512\t1.000000\t0.000000\t-1500\t-1500\t1.000000\t0\t0\t0\t1\t0\nNTRACK\t0\n");
        let t = s.transport.unwrap();
        assert_eq!((t.play, t.position.as_str(), t.beats.as_str()), (Play::Recording, "0:12.345", "7.2.36"));
        assert!(s.repeat);
        assert_eq!(s.tracks.len(), 1);
        assert!(Snapshot::parse("<html>").transport.is_none());
    }
}
