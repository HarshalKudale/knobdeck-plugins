//! The Mackie Control Universal protocol, as DAWs speak it to a control surface: buttons are notes
//! on channel 1 (127 pressed, 0 released) whose LEDs the DAW lights with the same notes; faders are
//! pitch bend on channels 1–8 (9 the master) with a touch note around a move; V-Pots and the jog
//! wheel are relative CCs; the DAW writes the LCD (SysEx 0x12) and the time display (CC 0x40–0x49).

use midi_io::msg;

/// Note numbers of the strip buttons (add the strip, 0–7).
pub const REC: u8 = 0x00;
pub const SOLO: u8 = 0x08;
pub const MUTE: u8 = 0x10;
pub const SELECT: u8 = 0x18;
pub const SMPTE_BEATS: u8 = 0x35;
pub const BANK_LEFT: u8 = 0x2E;
pub const BANK_RIGHT: u8 = 0x2F;
pub const CHANNEL_LEFT: u8 = 0x30;
pub const CHANNEL_RIGHT: u8 = 0x31;
pub const CYCLE: u8 = 0x56;
pub const REWIND: u8 = 0x5B;
pub const FAST_FORWARD: u8 = 0x5C;
pub const STOP: u8 = 0x5D;
pub const PLAY: u8 = 0x5E;
pub const RECORD: u8 = 0x5F;
/// Fader touch notes: 0x68 + strip, 0x70 for the master.
pub const TOUCH: u8 = 0x68;
pub const MASTER: usize = 8;

/// The other buttons, for the generic Mackie Button action: (note, name).
pub const BUTTONS: &[(u8, &str)] = &[
    (0x28, "Assign: Track"),
    (0x29, "Assign: Send"),
    (0x2A, "Assign: Pan / Surround"),
    (0x2B, "Assign: Plug-in"),
    (0x2C, "Assign: EQ"),
    (0x2D, "Assign: Instrument"),
    (0x32, "Flip"),
    (0x33, "Global View"),
    (0x34, "Name / Value"),
    (0x35, "SMPTE / Beats"),
    (0x36, "F1"),
    (0x37, "F2"),
    (0x38, "F3"),
    (0x39, "F4"),
    (0x3A, "F5"),
    (0x3B, "F6"),
    (0x3C, "F7"),
    (0x3D, "F8"),
    (0x3E, "MIDI Tracks"),
    (0x3F, "Inputs"),
    (0x40, "Audio Tracks"),
    (0x41, "Audio Instruments"),
    (0x42, "Aux"),
    (0x43, "Buses"),
    (0x44, "Outputs"),
    (0x45, "User"),
    (0x46, "Shift"),
    (0x47, "Option"),
    (0x48, "Control"),
    (0x49, "Alt / Command"),
    (0x4A, "Automation: Read / Off"),
    (0x4B, "Automation: Write"),
    (0x4C, "Automation: Trim"),
    (0x4D, "Automation: Touch"),
    (0x4E, "Automation: Latch"),
    (0x4F, "Group"),
    (0x50, "Save"),
    (0x51, "Undo"),
    (0x52, "Cancel"),
    (0x53, "Enter"),
    (0x54, "Marker"),
    (0x55, "Nudge"),
    (0x56, "Cycle"),
    (0x57, "Drop"),
    (0x58, "Replace"),
    (0x59, "Click"),
    (0x5A, "Solo (global)"),
    (0x60, "Cursor Up"),
    (0x61, "Cursor Down"),
    (0x62, "Cursor Left"),
    (0x63, "Cursor Right"),
    (0x64, "Zoom"),
    (0x65, "Scrub"),
    (0x66, "User Switch A"),
    (0x67, "User Switch B"),
];

const JOG: u8 = 0x3C;
const VPOT: u8 = 0x10;
const VPOT_RING: u8 = 0x30;
const TIMECODE: u8 = 0x40;
/// SysEx header of a Mackie Control (main unit, model 0x14).
const HEADER: [u8; 5] = [0xF0, 0x00, 0x00, 0x66, 0x14];
/// The serial number this surface reports in the handshake.
const SERIAL: [u8; 7] = *b"KNOBDK1";
pub const LCD_LEN: usize = 112;

pub fn button(note: u8, down: bool) -> [u8; 3] {
    msg::note_on(0, note, if down { 0x7F } else { 0x00 })
}

pub fn touch(strip: usize, down: bool) -> [u8; 3] {
    button(if strip == MASTER { 0x70 } else { TOUCH + strip as u8 }, down)
}

/// A fader position, 0–16383, on strip 0–7 or the master (8).
pub fn fader(strip: usize, value: u16) -> [u8; 3] {
    msg::pitch_bend(strip.min(MASTER) as u8, value)
}

pub fn jog(ticks: i32) -> Option<[u8; 3]> {
    msg::relative(ticks).map(|v| msg::cc(0, JOG, v))
}

pub fn vpot(strip: usize, ticks: i32) -> Option<[u8; 3]> {
    msg::relative(ticks).map(|v| msg::cc(0, VPOT + strip.min(7) as u8, v))
}

pub fn pct_to_fader(pct: f64) -> u16 {
    (pct.clamp(0.0, 100.0) / 100.0 * 16383.0).round() as u16
}

pub fn fader_to_pct(v: u16) -> f64 {
    (v.min(16383) as f64 / 16383.0 * 1000.0).round() / 10.0
}

/// What the DAW has told the surface.
pub struct Model {
    /// LED per note: 0 off, 1 flashing, 127 on.
    pub leds: [u8; 128],
    pub faders: [Option<u16>; 9],
    pub lcd: [u8; LCD_LEN],
    /// Time display digits, rightmost first, as sent (character code, 0x40 = dot).
    pub digits: [u8; 10],
    /// V-Pot LED ring positions (0 = off, 1–11).
    pub rings: [Option<u8>; 8],
    /// Whether anything came from the DAW yet.
    pub heard: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model { leds: [0; 128], faders: [None; 9], lcd: [b' '; LCD_LEN], digits: [0; 10], rings: [None; 8], heard: false }
    }
}

impl Model {
    /// Take in one message from the DAW; returns the reply it asks for (the handshake), if any.
    pub fn apply(&mut self, m: &[u8]) -> Option<Vec<u8>> {
        let &status = m.first()?;
        if status == 0xF0 {
            return self.sysex(m);
        }
        match msg::parse(m)? {
            msg::Message::NoteOn { ch: 0, note, velocity } => self.leds[note as usize] = velocity,
            msg::Message::NoteOff { ch: 0, note } => self.leds[note as usize] = 0,
            msg::Message::PitchBend { ch, value } if (ch as usize) <= MASTER => self.faders[ch as usize] = Some(value),
            msg::Message::Cc { ch: 0, controller, value } => match controller {
                c if (VPOT_RING..VPOT_RING + 8).contains(&c) => self.rings[(c - VPOT_RING) as usize] = Some(value & 0x0F),
                c if (TIMECODE..TIMECODE + 10).contains(&c) => self.digits[(c - TIMECODE) as usize] = value,
                _ => return None,
            },
            _ => return None,
        }
        self.heard = true;
        None
    }

    fn sysex(&mut self, m: &[u8]) -> Option<Vec<u8>> {
        // Our own model (0x14) or the "any Mackie" id some DAWs probe with.
        if m.len() < 7 || m[..4] != HEADER[..4] || !matches!(m[4], 0x14 | 0x10) || m.last() != Some(&0xF7) {
            return None;
        }
        self.heard = true;
        let body = &m[6..m.len() - 1];
        let reply = |cmd: u8, data: &[u8]| {
            let mut r = HEADER.to_vec();
            r.push(cmd);
            r.extend_from_slice(data);
            r.push(0xF7);
            Some(r)
        };
        match m[5] {
            // Device query → host connection query (serial + a challenge the DAW answers).
            0x00 => reply(0x01, &[&SERIAL[..], &[0x01, 0x02, 0x03, 0x04]].concat()),
            // Host connection reply → connection confirmation.
            0x02 => reply(0x03, &SERIAL),
            // Firmware version request.
            0x13 => reply(0x14, b"V1.00"),
            0x12 => {
                let (&offset, text) = body.split_first()?;
                for (i, c) in text.iter().enumerate() {
                    if let Some(slot) = self.lcd.get_mut(offset as usize + i) {
                        *slot = if (0x20..0x7F).contains(c) { *c } else { b' ' };
                    }
                }
                None
            }
            // Faders to minimum / all LEDs off / reset.
            0x61 => {
                self.faders = [Some(0); 9];
                None
            }
            0x62 => {
                self.leds = [0; 128];
                None
            }
            0x63 => {
                *self = Model { heard: true, ..Model::default() };
                None
            }
            _ => None,
        }
    }

    pub fn lit(&self, note: u8) -> bool {
        self.leds[note as usize & 0x7F] > 0
    }

    /// The LCD's top line over strip 0–7 (the track name in most DAWs).
    pub fn strip_name(&self, strip: usize) -> String {
        self.lcd_cell(0, strip)
    }

    /// One 7-character cell of the LCD: row 0 (top) or 1, strip 0–7.
    pub fn lcd_cell(&self, row: usize, strip: usize) -> String {
        let start = row.min(1) * 56 + strip.min(7) * 7;
        String::from_utf8_lossy(&self.lcd[start..start + 7]).trim().to_string()
    }

    /// The time display (timecode or bars / beats), as the DAW wrote it.
    pub fn time(&self) -> String {
        let mut s = String::new();
        for &v in self.digits.iter().rev() {
            let c = v & 0x3F;
            let ch = if v == 0 {
                ' '
            } else if c < 0x20 {
                (c + 0x40) as char
            } else {
                c as char
            };
            s.push(ch);
            if v & 0x40 != 0 {
                s.push('.');
            }
        }
        s.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leds_faders_rings() {
        let mut m = Model::default();
        assert!(!m.heard);
        m.apply(&[0x90, PLAY, 0x7F]);
        m.apply(&[0x90, MUTE + 2, 0x01]);
        assert!(m.lit(PLAY) && m.lit(MUTE + 2) && !m.lit(STOP));
        m.apply(&[0x90, PLAY, 0x00]);
        assert!(!m.lit(PLAY));
        m.apply(&fader(3, 12000));
        m.apply(&fader(MASTER, 16383));
        assert_eq!(m.faders[3], Some(12000));
        assert_eq!(m.faders[MASTER], Some(16383));
        m.apply(&[0xB0, 0x32, 0x16]);
        assert_eq!(m.rings[2], Some(6));
        assert!(m.heard);
    }

    #[test]
    fn lcd_and_time() {
        let mut m = Model::default();
        let mut s = vec![0xF0, 0, 0, 0x66, 0x14, 0x12, 0];
        s.extend(b"Vocals Drums  Bass");
        s.push(0xF7);
        m.apply(&s);
        assert_eq!(m.strip_name(0), "Vocals");
        assert_eq!(m.strip_name(1), "Drums");
        assert_eq!(m.strip_name(2), "Bass");
        assert_eq!(m.strip_name(3), "");
        let mut lower = vec![0xF0, 0, 0, 0x66, 0x14, 0x12, 56 + 7];
        lower.extend(b"  -6.0 ");
        lower.push(0xF7);
        m.apply(&lower);
        assert_eq!(m.lcd_cell(1, 1), "-6.0");
        // Bars / beats "  1.1.1.000" sent rightmost digit first, dots on the 0x40 bit.
        let digits = [b'0', b'0', b'0', b'1' | 0x40, b'1' | 0x40, b'1' | 0x40, b' ', b' ', b' ', b' '];
        for (i, d) in digits.iter().enumerate() {
            m.apply(&[0xB0, 0x40 + i as u8, *d]);
        }
        assert_eq!(m.time(), "1.1.1.000");
    }

    #[test]
    fn handshake() {
        let mut m = Model::default();
        let r = m.apply(&[0xF0, 0, 0, 0x66, 0x14, 0x00, 0xF7]).unwrap();
        assert_eq!(&r[..6], &[0xF0, 0, 0, 0x66, 0x14, 0x01]);
        assert_eq!(&r[6..13], b"KNOBDK1");
        assert_eq!(r.len(), 18);
        let r = m.apply(&[0xF0, 0, 0, 0x66, 0x14, 0x02, b'K', b'N', b'O', b'B', b'D', b'K', b'1', 0, 0, 0, 0, 0xF7]).unwrap();
        assert_eq!(r, [&[0xF0, 0, 0, 0x66, 0x14, 0x03][..], b"KNOBDK1", &[0xF7]].concat());
        assert!(m.apply(&[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]).is_none());
    }

    #[test]
    fn outgoing() {
        assert_eq!(button(PLAY, true), [0x90, 0x5E, 0x7F]);
        assert_eq!(button(PLAY, false), [0x90, 0x5E, 0x00]);
        assert_eq!(touch(2, true), [0x90, 0x6A, 0x7F]);
        assert_eq!(touch(MASTER, false), [0x90, 0x70, 0x00]);
        assert_eq!(fader(0, 16383), [0xE0, 0x7F, 0x7F]);
        assert_eq!(jog(-2), Some([0xB0, 0x3C, 0x42]));
        assert_eq!(vpot(7, 1), Some([0xB0, 0x17, 0x01]));
        assert_eq!(jog(0), None);
        assert_eq!(pct_to_fader(100.0), 16383);
        assert_eq!(fader_to_pct(8192), 50.0);
    }
}
