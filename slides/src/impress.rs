//! LibreOffice Impress over its Impress Remote protocol (TCP 1599, the protocol of the Impress
//! Remote phone apps): newline-separated commands ended by a blank line. KnobDeck pairs once with
//! a PIN the user types into LibreOffice (Slide Show ▸ Impress Remote); LibreOffice remembers
//! the pairing. One connection is kept open: on connect LibreOffice sends every slide's preview,
//! so connecting per press would be slow.

use knobdeck_sdk::{Error, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

pub const PORT: u16 = 1599;
const CLIENT: &str = "KnobDeck";

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Show {
    pub paired: bool,
    /// LibreOffice is waiting for the user to type the PIN.
    pub validating: bool,
    pub running: bool,
    /// 0-based current slide and slide count while a show runs.
    pub slide: i64,
    pub count: i64,
}

struct Link {
    writer: Option<TcpStream>,
    show: Show,
    /// Bumped by every new connection, so an old reader thread can't reset a new link.
    generation: u64,
}

static LINK: Mutex<Link> = Mutex::new(Link { writer: None, show: Show { paired: false, validating: false, running: false, slide: 0, count: 0 }, generation: 0 });
static CHANGED: Condvar = Condvar::new();
static LAST_TRY: Mutex<Option<Instant>> = Mutex::new(None);

/// The 4-digit PIN KnobDeck pairs with (made once, kept in the data folder).
pub fn pin() -> String {
    let path = knobdeck_sdk::data_file("impress-pin.txt");
    if let Ok(p) = std::fs::read_to_string(&path) {
        let p = p.trim().to_string();
        if p.len() == 4 && p.bytes().all(|b| b.is_ascii_digit()) {
            return p;
        }
    }
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as u64 ^ d.as_secs()) ^ std::process::id() as u64;
    let p = format!("{:04}", (seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 33) % 10000);
    let _ = std::fs::write(&path, &p);
    p
}

/// Apply one message from LibreOffice (its lines, without the blank terminator).
pub fn apply(show: &mut Show, msg: &[String]) {
    let num = |i: usize| msg.get(i).and_then(|s| s.trim().parse::<i64>().ok());
    match msg.first().map(String::as_str) {
        Some("LO_SERVER_SERVER_PAIRED") => {
            show.paired = true;
            show.validating = false;
        }
        Some("LO_SERVER_VALIDATING_PIN") => show.validating = true,
        Some("slideshow_started") => {
            show.running = true;
            show.count = num(1).unwrap_or(0);
            show.slide = num(2).unwrap_or(0);
        }
        Some("slide_updated") => {
            show.running = true;
            show.slide = num(1).unwrap_or(show.slide);
        }
        Some("slideshow_finished") => show.running = false,
        _ => {}
    }
}

fn reader(stream: TcpStream, generation: u64) {
    let mut lines = BufReader::new(stream);
    let mut msg: Vec<String> = vec![];
    let mut line = String::new();
    loop {
        line.clear();
        match lines.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let l = line.trim_end_matches(['\r', '\n']);
        if !l.is_empty() {
            // Previews are long base64 lines; only the first lines of a message matter.
            if msg.len() < 4 {
                msg.push(l.to_string());
            }
            continue;
        }
        if !msg.is_empty() {
            let mut link = LINK.lock().unwrap();
            if link.generation == generation {
                apply(&mut link.show, &msg);
                CHANGED.notify_all();
            }
            msg.clear();
        }
    }
    let mut link = LINK.lock().unwrap();
    if link.generation == generation {
        link.writer = None;
        link.show = Show::default();
        CHANGED.notify_all();
    }
}

fn connect() -> Result<()> {
    {
        let mut last = LAST_TRY.lock().unwrap();
        // Don't hammer a LibreOffice that isn't listening (live keys ask every second).
        if last.is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return Err(Error::msg(NOT_LISTENING));
        }
        *last = Some(Instant::now());
    }
    let addr: SocketAddr = ([127, 0, 0, 1], PORT).into();
    let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400)).map_err(|_| Error::msg(NOT_LISTENING))?;
    let _ = stream.set_nodelay(true);
    let mut w = stream.try_clone().map_err(|e| Error::msg(e.to_string()))?;
    w.write_all(format!("LO_SERVER_CLIENT_PAIR\n{CLIENT}\n{}\n\n", pin()).as_bytes()).map_err(|_| Error::msg(NOT_LISTENING))?;
    let generation = {
        let mut link = LINK.lock().unwrap();
        link.generation += 1;
        link.writer = Some(w);
        link.show = Show::default();
        link.generation
    };
    std::thread::spawn(move || reader(stream, generation));
    *LAST_TRY.lock().unwrap() = None;
    Ok(())
}

pub const NOT_LISTENING: &str = "LibreOffice Impress isn't reachable. Open a presentation, turn on Tools ▸ Options ▸ LibreOffice Impress ▸ General ▸ Enable remote control and Enable insecure WiFi connections (needs Advanced ▸ Enable experimental features), then restart LibreOffice";

fn pair_message() -> String {
    format!("Pair KnobDeck in LibreOffice: Slide Show ▸ Impress Remote, choose KnobDeck and enter PIN {}", pin())
}

/// The current show state, connecting (and pairing) if needed.
pub fn show() -> Result<Show> {
    let mut link = LINK.lock().unwrap();
    if link.writer.is_none() {
        drop(link);
        connect()?;
        link = LINK.lock().unwrap();
        // Wait briefly for the pairing reply and the show state that follows it.
        let end = Instant::now() + Duration::from_millis(1500);
        while link.writer.is_some() && !link.show.paired && !link.show.validating {
            let Some(left) = end.checked_duration_since(Instant::now()) else { break };
            link = CHANGED.wait_timeout(link, left).unwrap().0;
        }
        if link.show.paired {
            let end = Instant::now() + Duration::from_millis(300);
            while !link.show.running {
                let Some(left) = end.checked_duration_since(Instant::now()) else { break };
                link = CHANGED.wait_timeout(link, left).unwrap().0;
            }
        }
    }
    if link.writer.is_none() {
        return Err(Error::msg(NOT_LISTENING));
    }
    if !link.show.paired {
        return Err(Error::msg(pair_message()));
    }
    Ok(link.show.clone())
}

pub fn send(cmd: &[&str]) -> Result<()> {
    show()?;
    let mut link = LINK.lock().unwrap();
    let Some(w) = link.writer.as_mut() else { return Err(Error::msg(NOT_LISTENING)) };
    let text = format!("{}\n\n", cmd.join("\n"));
    if w.write_all(text.as_bytes()).is_err() {
        link.writer = None;
        return Err(Error::msg(NOT_LISTENING));
    }
    Ok(())
}

pub fn running() -> bool {
    LINK.lock().unwrap().show.running || show().is_ok_and(|s| s.running)
}

/// Wait up to `ms` for the show state to change (after a command), so a key shows the new slide.
pub fn settle(ms: u64) {
    let link = LINK.lock().unwrap();
    let _ = CHANGED.wait_timeout(link, Duration::from_millis(ms));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn messages() {
        let mut s = Show::default();
        apply(&mut s, &m(&["LO_SERVER_VALIDATING_PIN"]));
        assert!(s.validating && !s.paired);
        apply(&mut s, &m(&["LO_SERVER_SERVER_PAIRED"]));
        assert!(s.paired && !s.validating);
        apply(&mut s, &m(&["slideshow_started", "12", "0"]));
        assert_eq!((s.running, s.count, s.slide), (true, 12, 0));
        apply(&mut s, &m(&["slide_updated", "3"]));
        assert_eq!(s.slide, 3);
        apply(&mut s, &m(&["slide_preview", "3", "iVBOR…"]));
        assert_eq!(s.slide, 3);
        apply(&mut s, &m(&["slideshow_finished"]));
        assert!(!s.running);
    }
}
