//! Soundpad (Windows) through its documented remote control pipe, `\\.\pipe\sp_remote_control`:
//! a request is the bare command text (`DoPlaySound(3, true, true)`), the answer `R-200` on
//! success, `R-<code>: <message>` on failure, or the data asked for (the sound list as XML).

/// One entry of Soundpad's sound list. `index` is the 1-based number DoPlaySound takes.
#[derive(Debug, PartialEq)]
pub struct Sound {
    pub index: u32,
    pub title: String,
}

#[cfg_attr(not(windows), allow(dead_code))]
pub const NOT_RUNNING: &str = "Soundpad isn't running. Start Soundpad, then try again.";

/// The answer to a Do… command: Ok for R-200, else Soundpad's own message.
pub fn check(answer: &str) -> Result<(), String> {
    let a = answer.trim();
    if a.starts_with("R-200") {
        return Ok(());
    }
    let msg = a.split_once(':').map(|(_, m)| m.trim()).filter(|m| !m.is_empty()).unwrap_or(a);
    Err(format!("Soundpad: {msg}"))
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before_ok = i == 0 || rest.as_bytes()[i - 1].is_ascii_whitespace();
        let after = rest[i + name.len()..].trim_start();
        if before_ok {
            if let Some(after) = after.strip_prefix('=') {
                let after = after.trim_start();
                let q = after.chars().next()?;
                if q == '"' || q == '\'' {
                    let body = &after[1..];
                    return body.find(q).map(|end| unescape(&body[..end]));
                }
            }
        }
        rest = &rest[i + name.len()..];
    }
    None
}

/// The `<Sound index=".." title=".." url=".." …/>` entries of GetSoundlist's XML. A sound with no
/// title is named by its file.
pub fn parse_soundlist(xml: &str) -> Vec<Sound> {
    let mut out = vec![];
    let mut rest = xml;
    while let Some(i) = rest.find("<Sound ") {
        rest = &rest[i + 7..];
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        let Some(index) = attr(tag, "index").and_then(|i| i.trim().parse().ok()) else { continue };
        let title = attr(tag, "title").filter(|t| !t.trim().is_empty()).or_else(|| {
            attr(tag, "url").map(|u| {
                let file = u.rsplit(['\\', '/']).next().unwrap_or(&u).to_string();
                file.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(file)
            })
        });
        out.push(Sound { index, title: title.unwrap_or_else(|| format!("Sound {index}")) });
    }
    out
}

/// Where Soundpad plays a sound: `(speakers, microphone)`.
pub fn lines(choice: &str) -> (bool, bool) {
    match choice {
        "speakers" => (true, false),
        "mic" => (false, true),
        _ => (true, true),
    }
}

#[cfg(windows)]
pub fn request(cmd: &str) -> Result<String, String> {
    win::request(cmd)
}

#[cfg(not(windows))]
pub fn request(_cmd: &str) -> Result<String, String> {
    Err("Soundpad is Windows only".into())
}

pub fn run(cmd: &str) -> Result<(), String> {
    check(&request(cmd)?)
}

pub fn sounds() -> Result<Vec<Sound>, String> {
    let xml = request("GetSoundlist()")?;
    if xml.trim_start().starts_with("R-") {
        check(&xml)?;
    }
    Ok(parse_soundlist(&xml))
}

#[cfg(windows)]
mod win {
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;
    use std::sync::Mutex;
    use std::time::Duration;
    use windows::Win32::Foundation::{ERROR_MORE_DATA, HANDLE};
    use windows::Win32::Storage::FileSystem::ReadFile;

    const PIPE: &str = r"\\.\pipe\sp_remote_control";
    /// Soundpad's pipe takes one request at a time.
    static LOCK: Mutex<()> = Mutex::new(());

    fn open() -> Result<std::fs::File, String> {
        for _ in 0..10 {
            match std::fs::OpenOptions::new().read(true).write(true).open(PIPE) {
                Ok(f) => return Ok(f),
                // ERROR_PIPE_BUSY: another client is talking to it.
                Err(e) if e.raw_os_error() == Some(231) => std::thread::sleep(Duration::from_millis(30)),
                Err(_) => return Err(super::NOT_RUNNING.into()),
            }
        }
        Err("Soundpad is busy; try again".into())
    }

    pub fn request(cmd: &str) -> Result<String, String> {
        let _g = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut pipe = open()?;
        pipe.write_all(cmd.as_bytes()).map_err(|e| format!("Soundpad: {e}"))?;
        // One answer is one pipe message; read it whole however long it is.
        let h = HANDLE(pipe.as_raw_handle());
        let mut answer = vec![];
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let mut n = 0u32;
            let r = unsafe { ReadFile(h, Some(&mut buf), Some(&mut n), None) };
            answer.extend_from_slice(&buf[..n as usize]);
            match r {
                Ok(()) => break,
                Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => continue,
                Err(e) => return Err(format!("Soundpad: {}", e.message())),
            }
        }
        Ok(String::from_utf8_lossy(&answer).into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers() {
        assert!(check("R-200").is_ok());
        assert_eq!(check("R-404: Command not found.").unwrap_err(), "Soundpad: Command not found.");
        assert_eq!(check("R-500").unwrap_err(), "Soundpad: R-500");
    }

    #[test]
    fn soundlist() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Soundlist>
  <Sound index="1" url="C:\Sounds\airhorn.mp3" artist="" title="Air horn &amp; echo" duration="0:03" addedOn="2024-01-01" playCount="4"/>
  <Sound index="2" url="C:\Sounds\Ba Dum Tss.wav" duration="0:02"/>
  <Sound index='3' title=''  url="D:/x/y/boo.ogg"/>
  <Sound url="no-index.mp3"/>
</Soundlist>"#;
        assert_eq!(
            parse_soundlist(xml),
            vec![
                Sound { index: 1, title: "Air horn & echo".into() },
                Sound { index: 2, title: "Ba Dum Tss".into() },
                Sound { index: 3, title: "boo".into() },
            ]
        );
        assert!(parse_soundlist("R-200").is_empty());
    }

    #[test]
    fn attr_needs_a_whole_name() {
        assert_eq!(attr(r#"subtitle="x" title="y""#, "title"), Some("y".into()));
    }

    #[test]
    fn where_to_play() {
        assert_eq!(lines("both"), (true, true));
        assert_eq!(lines("speakers"), (true, false));
        assert_eq!(lines("mic"), (false, true));
    }
}
