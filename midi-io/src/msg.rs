//! Building and reading MIDI 1.0 channel messages. Channels are 0–15 here (users see 1–16).

pub fn note_on(ch: u8, note: u8, velocity: u8) -> [u8; 3] {
    [0x90 | (ch & 0x0F), note & 0x7F, velocity & 0x7F]
}

pub fn note_off(ch: u8, note: u8) -> [u8; 3] {
    [0x80 | (ch & 0x0F), note & 0x7F, 0]
}

pub fn cc(ch: u8, controller: u8, value: u8) -> [u8; 3] {
    [0xB0 | (ch & 0x0F), controller & 0x7F, value & 0x7F]
}

pub fn program(ch: u8, program: u8) -> [u8; 2] {
    [0xC0 | (ch & 0x0F), program & 0x7F]
}

/// Pitch bend, 0–16383 (8192 is the centre).
pub fn pitch_bend(ch: u8, value: u16) -> [u8; 3] {
    let v = value.min(0x3FFF);
    [0xE0 | (ch & 0x0F), (v & 0x7F) as u8, (v >> 7) as u8]
}

/// One received channel message, by kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    /// A note on with velocity 0 counts as a note off, as MIDI says.
    NoteOn { ch: u8, note: u8, velocity: u8 },
    NoteOff { ch: u8, note: u8 },
    Cc { ch: u8, controller: u8, value: u8 },
    Program { ch: u8, program: u8 },
    PitchBend { ch: u8, value: u16 },
    Aftertouch { ch: u8, value: u8 },
    PolyAftertouch { ch: u8, note: u8, value: u8 },
}

pub fn parse(m: &[u8]) -> Option<Message> {
    let status = *m.first()?;
    if !(0x80..0xF0).contains(&status) {
        return None;
    }
    let ch = status & 0x0F;
    let d1 = m.get(1).copied().unwrap_or(0) & 0x7F;
    let d2 = m.get(2).copied().unwrap_or(0) & 0x7F;
    let need = if matches!(status & 0xF0, 0xC0 | 0xD0) { 2 } else { 3 };
    if m.len() < need {
        return None;
    }
    Some(match status & 0xF0 {
        0x80 => Message::NoteOff { ch, note: d1 },
        0x90 if d2 == 0 => Message::NoteOff { ch, note: d1 },
        0x90 => Message::NoteOn { ch, note: d1, velocity: d2 },
        0xA0 => Message::PolyAftertouch { ch, note: d1, value: d2 },
        0xB0 => Message::Cc { ch, controller: d1, value: d2 },
        0xC0 => Message::Program { ch, program: d1 },
        0xD0 => Message::Aftertouch { ch, value: d1 },
        _ => Message::PitchBend { ch, value: (d2 as u16) << 7 | d1 as u16 },
    })
}

const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// A note number from `60` or a name (`C4` is 60, middle C; `C#4`, `Db4`, `C-1` is 0).
pub fn parse_note(text: &str) -> Option<u8> {
    let t = text.trim();
    if let Ok(n) = t.parse::<u8>() {
        return (n <= 127).then_some(n);
    }
    let mut chars = t.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let base: i32 = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let rest = chars.as_str();
    let (shift, octave) = match rest.chars().next() {
        Some('#') | Some('♯') => (1, &rest[rest.chars().next()?.len_utf8()..]),
        Some('b') | Some('♭') => (-1, &rest[rest.chars().next()?.len_utf8()..]),
        _ => (0, rest),
    };
    let octave: i32 = octave.parse().ok()?;
    let n = (octave + 1) * 12 + base + shift;
    (0..=127).contains(&n).then_some(n as u8)
}

/// `60` → `C4`.
pub fn note_name(n: u8) -> String {
    format!("{}{}", NAMES[(n % 12) as usize], n as i32 / 12 - 1)
}

/// Bytes from hex text (`F0 7E 7F 06 01 F7`, commas and `0x` allowed).
pub fn parse_hex(text: &str) -> Option<Vec<u8>> {
    let cleaned = text.replace(',', " ");
    let mut out = Vec::new();
    for tok in cleaned.split_whitespace() {
        let t = tok.trim_start_matches("0x").trim_start_matches("0X");
        if t.is_empty() || t.len() % 2 != 0 {
            return None;
        }
        for i in (0..t.len()).step_by(2) {
            out.push(u8::from_str_radix(t.get(i..i + 2)?, 16).ok()?);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Split hex bytes into whole messages (each status byte starts one; SysEx runs to F7). Running
/// status (data without a status byte) is completed with the last status.
pub fn split(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut last_status = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0xF0 {
            let end = bytes[i..].iter().position(|&x| x == 0xF7).ok_or("SysEx must end with F7")?;
            out.push(bytes[i..=i + end].to_vec());
            i += end + 1;
            last_status = None;
            continue;
        }
        let (status, start) = if b >= 0x80 {
            (b, i + 1)
        } else {
            (last_status.ok_or("Messages must start with a status byte (80–FF)")?, i)
        };
        let len = match status {
            0x80..=0xBF | 0xE0..=0xEF | 0xF2 => 2,
            0xC0..=0xDF | 0xF1 | 0xF3 => 1,
            _ => 0,
        };
        if start + len > bytes.len() {
            return Err(format!("Message {status:02X} needs {len} data byte(s)"));
        }
        if bytes[start..start + len].iter().any(|&d| d >= 0x80) {
            return Err(format!("Message {status:02X}: data bytes must be 00–7F"));
        }
        let mut m = vec![status];
        m.extend_from_slice(&bytes[start..start + len]);
        out.push(m);
        if status < 0xF0 {
            last_status = Some(status);
        }
        i = start + len;
    }
    Ok(out)
}

/// A relative encoder tick count in the "sign bit" form Mackie Control and most DAWs read:
/// 1–63 clockwise, 65–127 (64 + n) counter-clockwise.
pub fn relative(ticks: i32) -> Option<u8> {
    match ticks {
        0 => None,
        t if t > 0 => Some(t.min(63) as u8),
        t => Some(0x40 | (-t).min(63) as u8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_by_name_and_number() {
        assert_eq!(parse_note("60"), Some(60));
        assert_eq!(parse_note("C4"), Some(60));
        assert_eq!(parse_note("c#4"), Some(61));
        assert_eq!(parse_note("Db4"), Some(61));
        assert_eq!(parse_note("C-1"), Some(0));
        assert_eq!(parse_note("G9"), Some(127));
        assert_eq!(parse_note("A9"), None);
        assert_eq!(parse_note("200"), None);
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(0), "C-1");
    }

    #[test]
    fn messages_round_trip() {
        assert_eq!(parse(&note_on(2, 60, 100)), Some(Message::NoteOn { ch: 2, note: 60, velocity: 100 }));
        assert_eq!(parse(&note_on(0, 60, 0)), Some(Message::NoteOff { ch: 0, note: 60 }));
        assert_eq!(parse(&cc(15, 7, 127)), Some(Message::Cc { ch: 15, controller: 7, value: 127 }));
        assert_eq!(parse(&program(0, 5)), Some(Message::Program { ch: 0, program: 5 }));
        assert_eq!(parse(&pitch_bend(8, 16383)), Some(Message::PitchBend { ch: 8, value: 16383 }));
        assert_eq!(parse(&pitch_bend(0, 8192)), Some(Message::PitchBend { ch: 0, value: 8192 }));
        assert_eq!(parse(&[0xF0, 1, 0xF7]), None);
        assert_eq!(parse(&[0x90, 60]), None);
    }

    #[test]
    fn hex_and_split() {
        let b = parse_hex("90 3C 64, 0x80 3c 00 3E 00 F0 7E 7F 06 01 F7 C0 05").unwrap();
        let m = split(&b).unwrap();
        let want: Vec<Vec<u8>> = vec![vec![0x90, 0x3C, 0x64], vec![0x80, 0x3C, 0x00], vec![0x80, 0x3E, 0x00], vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7], vec![0xC0, 0x05]];
        assert_eq!(m, want);
        assert_eq!(parse_hex("9"), None);
        assert_eq!(parse_hex("zz"), None);
        assert!(split(&[0xF0, 1, 2]).is_err());
        assert!(split(&[0x3C]).is_err());
        assert!(split(&[0x90, 0x3C]).is_err());
    }

    #[test]
    fn relative_ticks() {
        assert_eq!(relative(0), None);
        assert_eq!(relative(3), Some(3));
        assert_eq!(relative(-3), Some(0x43));
        assert_eq!(relative(500), Some(63));
    }
}
