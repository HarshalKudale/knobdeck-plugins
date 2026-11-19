//! Linux: Speech Dispatcher (the speech service GNOME, KDE and Orca use) over its SSIP protocol on
//! the Unix socket, in pure Rust. One connection stays open: it carries the rate, volume and voice,
//! and its BEGIN / END events say whether something is being spoken.

use crate::{Settings, Voice};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const NOT_INSTALLED: &str = "Speech Dispatcher isn't installed. Install the speech-dispatcher package and a voice (espeak-ng, or piper / RHVoice)";

static CONN: Mutex<Option<Conn>> = Mutex::new(None);
static SPEAKING: AtomicBool = AtomicBool::new(false);
/// The id of the last message queued; only its END / CANCEL ends "speaking".
static LATEST: AtomicU64 = AtomicU64::new(0);

/// One SSIP reply: its code, the data lines before the final line, and the final line's text.
#[derive(Debug)]
pub(crate) struct Reply {
    code: u16,
    data: Vec<String>,
    text: String,
}

impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.code)
    }
}

struct Conn {
    stream: UnixStream,
    replies: Receiver<Reply>,
    /// The voice set on this connection ("" = Speech Dispatcher's default).
    voice: String,
}

/// Where Speech Dispatcher listens: `SPEECHD_ADDRESS=unix_socket:/path`, else
/// `$XDG_RUNTIME_DIR/speech-dispatcher/speechd.sock`.
fn socket_path() -> Result<PathBuf, String> {
    if let Ok(addr) = std::env::var("SPEECHD_ADDRESS") {
        let addr = addr.trim();
        if let Some(p) = addr.strip_prefix("unix_socket:") {
            return Ok(PathBuf::from(p));
        }
        if !addr.is_empty() {
            return Err(format!("SPEECHD_ADDRESS={addr} isn't supported: only unix_socket: addresses are"));
        }
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", uid())));
    Ok(runtime.join("speech-dispatcher").join("speechd.sock"))
}

fn uid() -> String {
    std::fs::metadata("/proc/self").map(|m| std::os::unix::fs::MetadataExt::uid(&m).to_string()).unwrap_or_default()
}

/// Connect, starting Speech Dispatcher (`--spawn`, as libspeechd does) when it isn't running.
fn connect_socket() -> Result<UnixStream, String> {
    let path = socket_path()?;
    if let Ok(s) = UnixStream::connect(&path) {
        return Ok(s);
    }
    if std::env::var_os("SPEECHD_ADDRESS").is_some() {
        return Err(format!("Speech Dispatcher isn't listening on {}", path.display()));
    }
    match std::process::Command::new("speech-dispatcher").arg("--spawn").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(NOT_INSTALLED.into()),
        Err(e) => return Err(format!("Couldn't start Speech Dispatcher: {e}")),
        Ok(_) => {}
    }
    let until = Instant::now() + Duration::from_secs(4);
    loop {
        match UnixStream::connect(&path) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() >= until => return Err(format!("Speech Dispatcher didn't start ({}: {e})", path.display())),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Read replies and events; events update [`SPEAKING`], replies go to the channel.
fn reader(stream: UnixStream, tx: mpsc::Sender<Reply>) {
    let mut lines = BufReader::new(stream).lines();
    let mut data = vec![];
    while let Some(Ok(line)) = lines.next() {
        let line = line.trim_end_matches('\r');
        let (Some(code), sep) = (line.get(..3).and_then(|c| c.parse::<u16>().ok()), line.as_bytes().get(3).copied()) else { continue };
        let rest = line.get(4..).unwrap_or("").to_string();
        if sep == Some(b'-') {
            data.push(rest);
            continue;
        }
        let reply = Reply { code, data: std::mem::take(&mut data), text: rest };
        if (700..800).contains(&code) {
            event(&reply);
        } else if tx.send(reply).is_err() {
            break;
        }
    }
    SPEAKING.store(false, Ordering::SeqCst);
}

fn event(e: &Reply) {
    let id: u64 = e.data.first().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
    match e.code {
        701 | 705 => SPEAKING.store(true, Ordering::SeqCst), // BEGIN, RESUMED
        702..=704 if id == LATEST.load(Ordering::SeqCst) => SPEAKING.store(false, Ordering::SeqCst), // END, CANCELED, PAUSED
        _ => {}
    }
}

impl Conn {
    fn open() -> Result<Conn, String> {
        let stream = connect_socket()?;
        let (tx, rx) = mpsc::channel();
        let read = stream.try_clone().map_err(|e| e.to_string())?;
        std::thread::spawn(move || reader(read, tx));
        let mut c = Conn { stream, replies: rx, voice: String::new() };
        let user = std::env::var("USER").unwrap_or_else(|_| "user".into()).replace([':', ' ', '\r', '\n'], "_");
        c.ok(&format!("SET self CLIENT_NAME {user}:knobdeck:tts"))?;
        c.ok("SET self NOTIFICATION all on")?;
        Ok(c)
    }

    fn send(&mut self, raw: &str) -> Result<Reply, String> {
        self.stream.write_all(raw.as_bytes()).map_err(|e| format!("Speech Dispatcher closed the connection ({e})"))?;
        match self.replies.recv_timeout(REPLY_TIMEOUT) {
            Ok(r) => Ok(r),
            Err(RecvTimeoutError::Timeout) => Err("Speech Dispatcher didn't answer".into()),
            Err(RecvTimeoutError::Disconnected) => Err("Speech Dispatcher closed the connection".into()),
        }
    }

    /// Send one command line; an error reply is an error.
    fn ok(&mut self, line: &str) -> Result<Reply, String> {
        let r = self.send(&format!("{line}\r\n"))?;
        if r.ok() {
            Ok(r)
        } else {
            Err(format!("Speech Dispatcher: {} ({})", r.text, line.split(' ').take(3).collect::<Vec<_>>().join(" ")))
        }
    }

    fn apply(&mut self, s: &Settings) -> Result<(), String> {
        self.ok(&format!("SET self RATE {}", rate(s.rate)))?;
        self.ok(&format!("SET self VOLUME {}", volume(s.volume)))?;
        if s.voice != self.voice && !s.voice.is_empty() {
            let (module, name) = split_voice(&s.voice);
            if !module.is_empty() {
                self.ok(&format!("SET self OUTPUT_MODULE {module}"))?;
            }
            self.ok(&format!("SET self SYNTHESIS_VOICE {name}"))?;
            self.voice = s.voice.clone();
        }
        Ok(())
    }

    fn speak(&mut self, text: &str) -> Result<(), String> {
        let r = self.ok("SPEAK")?;
        if r.code != 230 {
            return Err(format!("Speech Dispatcher: {}", r.text));
        }
        let r = self.send(&speak_data(text))?;
        if !r.ok() {
            return Err(format!("Speech Dispatcher: {}", r.text));
        }
        if let Some(id) = r.data.first().and_then(|s| s.trim().parse().ok()) {
            LATEST.store(id, Ordering::SeqCst);
        }
        SPEAKING.store(true, Ordering::SeqCst); // until its END arrives
        Ok(())
    }
}

/// The message body: lines joined with CRLF, a leading "." doubled, ended by a lone ".".
pub(crate) fn speak_data(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if line.starts_with('.') {
            out.push('.');
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str(".\r\n");
    out
}

/// Speech rate 0.5×–3× → SSIP's -100…100 (0 = normal, 100 = about 3×).
pub(crate) fn rate(r: f64) -> i32 {
    ((r.clamp(0.3, 3.0).ln() / 3f64.ln()) * 100.0).round().clamp(-100.0, 100.0) as i32
}

/// Volume 0–100 % → SSIP's -100…100.
pub(crate) fn volume(v: f64) -> i32 {
    (v.clamp(0.0, 100.0) * 2.0 - 100.0).round() as i32
}

/// A voice id is `module/name` (`espeak-ng/English (America)`).
fn split_voice(id: &str) -> (&str, &str) {
    id.split_once('/').unwrap_or(("", id))
}

/// Run `f` on the shared connection, opening it (again) when needed. A dead connection is retried
/// once; going back to the default voice opens a new one (SSIP has no "default voice" command).
fn with_conn<T>(s: Option<&Settings>, f: impl Fn(&mut Conn) -> Result<T, String>) -> Result<T, String> {
    let mut guard = CONN.lock().unwrap();
    if let (Some(c), Some(s)) = (guard.as_ref(), s) {
        if s.voice.is_empty() && !c.voice.is_empty() {
            *guard = None;
        }
    }
    for attempt in 0..2 {
        if guard.is_none() {
            *guard = Some(Conn::open()?);
        }
        let c = guard.as_mut().unwrap();
        let r = s.map(|s| c.apply(s)).unwrap_or(Ok(())).and_then(|_| f(c));
        match r {
            Err(e) if attempt == 0 && e.contains("closed the connection") => *guard = None,
            r => return r,
        }
    }
    unreachable!()
}

pub fn speak(text: &str, s: &Settings) -> Result<(), String> {
    with_conn(Some(s), |c| {
        c.ok("CANCEL self")?;
        c.speak(text)
    })
}

pub fn stop() -> Result<(), String> {
    if CONN.lock().unwrap().is_none() {
        return Ok(()); // nothing was ever spoken
    }
    with_conn(None, |c| c.ok("CANCEL self").map(|_| ()))?;
    SPEAKING.store(false, Ordering::SeqCst);
    Ok(())
}

pub fn speaking() -> bool {
    SPEAKING.load(Ordering::SeqCst)
}

/// Every voice of every output module, listed on a connection of its own (listing switches the
/// connection's module).
pub fn voices() -> Result<Vec<Voice>, String> {
    let mut c = Conn::open()?;
    let modules = c.ok("LIST OUTPUT_MODULES")?.data;
    let mut out = vec![];
    for m in modules.iter().filter(|m| !m.is_empty() && m.as_str() != "dummy") {
        if c.ok(&format!("SET self OUTPUT_MODULE {m}")).is_err() {
            continue;
        }
        let Ok(list) = c.ok("LIST SYNTHESIS_VOICES") else { continue };
        for line in list.data {
            let mut parts = line.split('\t');
            let name = parts.next().unwrap_or("").trim();
            let lang = parts.next().unwrap_or("").trim();
            if name.is_empty() {
                continue;
            }
            let lang = if lang.is_empty() || lang == "none" { String::new() } else { format!(" ({lang})") };
            out.push(Voice { id: format!("{m}/{name}"), label: format!("{name}{lang} · {m}") });
        }
    }
    let _ = c.ok("QUIT");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;

    #[test]
    fn mappings() {
        assert_eq!(rate(1.0), 0);
        assert_eq!(rate(3.0), 100);
        assert!(rate(0.5) < -50 && rate(0.5) > -70);
        assert_eq!(volume(100.0), 100);
        assert_eq!(volume(50.0), 0);
        assert_eq!(volume(0.0), -100);
        assert_eq!(speak_data("hi\n.dot\nend"), "hi\r\n..dot\r\nend\r\n.\r\n");
        assert_eq!(split_voice("espeak-ng/English (America)"), ("espeak-ng", "English (America)"));
    }

    /// A minimal SSIP server: answers like Speech Dispatcher and sends BEGIN / END events.
    fn mock(listener: UnixListener, log: std::sync::Arc<Mutex<Vec<String>>>) {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { break };
            let log = log.clone();
            std::thread::spawn(move || {
                let mut w = stream.try_clone().unwrap();
                let mut lines = BufReader::new(stream).lines();
                let mut module = String::new();
                while let Some(Ok(line)) = lines.next() {
                    log.lock().unwrap().push(line.clone());
                    let reply: String = match line.as_str() {
                        "SPEAK" => {
                            w.write_all(b"230 OK RECEIVING DATA\r\n").unwrap();
                            let mut body = vec![];
                            for l in lines.by_ref() {
                                let l = l.unwrap();
                                if l == "." {
                                    break;
                                }
                                body.push(l);
                            }
                            log.lock().unwrap().push(format!("BODY {}", body.join("|")));
                            "225-7\r\n225 OK MESSAGE QUEUED\r\n701-7\r\n701-1\r\n701 BEGIN\r\n".into()
                        }
                        "LIST OUTPUT_MODULES" => "250-espeak-ng\r\n250-dummy\r\n250 OK MODULE LIST SENT\r\n".into(),
                        "LIST SYNTHESIS_VOICES" => format!("249-English\ten\tnone\r\n249-{module}Voice\tde\tnone\r\n249 OK VOICE LIST SENT\r\n"),
                        "CANCEL self" => "210 OK CANCELED\r\n702-7\r\n702-1\r\n702 END\r\n".into(),
                        l if l.starts_with("SET self OUTPUT_MODULE ") => {
                            module = l["SET self OUTPUT_MODULE ".len()..].to_string();
                            "216 OK OUTPUT MODULE SET\r\n".into()
                        }
                        l if l.starts_with("SET self RATE -500") => "410 ERR PARAMETER OUT OF RANGE\r\n".into(),
                        l if l.starts_with("SET") => "200 OK\r\n".into(),
                        "QUIT" => {
                            let _ = w.write_all(b"231 HAPPY HACKING\r\n");
                            break;
                        }
                        _ => "300 ERR UNKNOWN COMMAND\r\n".into(),
                    };
                    w.write_all(reply.as_bytes()).unwrap();
                }
            });
        }
    }

    #[test]
    fn ssip_against_a_mock_server() {
        let dir = std::env::temp_dir().join(format!("kd-tts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sock = dir.join("speechd.sock");
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();
        let log = std::sync::Arc::new(Mutex::new(vec![]));
        let l2 = log.clone();
        std::thread::spawn(move || mock(listener, l2));
        std::env::set_var("SPEECHD_ADDRESS", format!("unix_socket:{}", sock.display()));

        let voices = voices().unwrap();
        assert_eq!(voices.len(), 2);
        assert_eq!(voices[0].id, "espeak-ng/English");
        assert_eq!(voices[0].label, "English (en) · espeak-ng");

        let s = Settings { voice: "espeak-ng/English".into(), rate: 1.0, volume: 100.0 };
        speak("Hello\n.world", &s).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(speaking());
        stop().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!speaking());
        let log = log.lock().unwrap().clone();
        assert!(log.iter().any(|l| l.starts_with("SET self CLIENT_NAME ") && l.ends_with(":knobdeck:tts")));
        assert!(log.contains(&"SET self RATE 0".to_string()));
        assert!(log.contains(&"SET self VOLUME 100".to_string()));
        assert!(log.contains(&"SET self SYNTHESIS_VOICE English".to_string()));
        assert!(log.contains(&"BODY Hello|..world".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
