//! macOS: the system's speech synthesizer through `/usr/bin/say` (Apple's own front end to it:
//! every installed voice, downloaded Enhanced / Premium ones included). The text goes in on stdin,
//! so its length and its first character don't matter; stopping ends the `say` process.

use crate::{Settings, Voice};
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

const SAY: &str = "/usr/bin/say";
/// `say`'s normal rate, in words per minute.
const NORMAL_WPM: f64 = 180.0;

static CHILD: Mutex<Option<Child>> = Mutex::new(None);

pub fn speak(text: &str, s: &Settings) -> Result<(), String> {
    stop()?;
    let mut cmd = Command::new(SAY);
    cmd.arg("-r").arg(format!("{}", (NORMAL_WPM * s.rate.clamp(0.3, 3.0)).round()));
    if !s.voice.is_empty() {
        cmd.arg("-v").arg(&s.voice);
    }
    let mut child = cmd.arg("-f").arg("-").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| format!("Couldn't run say: {e}"))?;
    // Volume as the synthesizer's embedded command (0.0–1.0), before the text.
    let body = format!("[[volm {:.2}]] {text}", (s.volume / 100.0).clamp(0.0, 1.0));
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(body.as_bytes());
    }
    // A bad voice makes `say` exit at once with a message.
    std::thread::sleep(std::time::Duration::from_millis(80));
    if let Ok(Some(status)) = child.try_wait() {
        if !status.success() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut e, &mut err);
            }
            return Err(if err.trim().is_empty() { format!("say failed ({status})") } else { err.trim().to_string() });
        }
    }
    *CHILD.lock().unwrap() = Some(child);
    Ok(())
}

pub fn stop() -> Result<(), String> {
    if let Some(mut c) = CHILD.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    Ok(())
}

pub fn speaking() -> bool {
    let mut g = CHILD.lock().unwrap();
    match g.as_mut().map(|c| c.try_wait()) {
        Some(Ok(None)) => true,
        Some(_) => {
            *g = None;
            false
        }
        None => false,
    }
}

pub fn voices() -> Result<Vec<Voice>, String> {
    let out = Command::new(SAY).args(["-v", "?"]).output().map_err(|e| format!("Couldn't run say: {e}"))?;
    if !out.status.success() {
        return Err(format!("say -v ? failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(parse_voices(&String::from_utf8_lossy(&out.stdout)))
}

/// `say -v ?` lines: `Name (maybe with spaces)   en_US    # Hello! My name is …`.
pub(crate) fn parse_voices(text: &str) -> Vec<Voice> {
    text.lines()
        .filter_map(|line| {
            let head = line.split('#').next()?.trim_end();
            let (name, locale) = head.rsplit_once(char::is_whitespace)?;
            let name = name.trim();
            (!name.is_empty()).then(|| Voice { id: name.to_string(), label: format!("{name} ({})", locale.trim()) })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn say_voice_list() {
        let v = super::parse_voices("Alex                en_US    # Most people recognize me by my voice.\nBad News            en_US    # The light you see at the end of the tunnel\nEddy (German (Germany)) de_DE    # Hallo! Ich heiße Eddy.\n\n");
        assert_eq!(v.len(), 3);
        assert_eq!((v[0].id.as_str(), v[0].label.as_str()), ("Alex", "Alex (en_US)"));
        assert_eq!(v[1].id, "Bad News");
        assert_eq!(v[2].id, "Eddy (German (Germany))");
    }
}
