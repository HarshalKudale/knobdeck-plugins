//! The OpenRGB SDK protocol (OpenRGB ▸ SDK Server, TCP 6742): 16-byte header `ORGB`, device
//! index, packet id, data length (little endian), then the data. Only what the plugin uses:
//! controller list and data, profiles, custom (direct) mode, LED colors and mode updates.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 6742;
/// Highest protocol version this client parses (OpenRGB 0.9 speaks 4; newer servers drop to
/// the client's version).
pub const CLIENT_PROTOCOL: u32 = 4;

pub const REQUEST_CONTROLLER_COUNT: u32 = 0;
pub const REQUEST_CONTROLLER_DATA: u32 = 1;
pub const REQUEST_PROTOCOL_VERSION: u32 = 40;
pub const SET_CLIENT_NAME: u32 = 50;
/// Sent by the server unasked when devices change; [`Client::request`] skips it.
#[allow(dead_code)]
pub const DEVICE_LIST_UPDATED: u32 = 100;
pub const REQUEST_PROFILE_LIST: u32 = 150;
pub const REQUEST_LOAD_PROFILE: u32 = 152;
pub const UPDATE_LEDS: u32 = 1050;
pub const SET_CUSTOM_MODE: u32 = 1100;
pub const UPDATE_MODE: u32 = 1101;

pub const MODE_FLAG_HAS_BRIGHTNESS: u32 = 1 << 4;
pub const COLOR_MODE_PER_LED: u32 = 1;
pub const COLOR_MODE_MODE_SPECIFIC: u32 = 2;

pub type Rgb = [u8; 3];

#[derive(Clone, Debug, PartialEq)]
pub struct Mode {
    pub name: String,
    pub value: i32,
    pub flags: u32,
    pub speed_min: u32,
    pub speed_max: u32,
    pub brightness_min: u32,
    pub brightness_max: u32,
    pub colors_min: u32,
    pub colors_max: u32,
    pub speed: u32,
    pub brightness: u32,
    pub direction: u32,
    pub color_mode: u32,
    pub colors: Vec<Rgb>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    pub name: String,
    pub vendor: String,
    pub location: String,
    pub active_mode: usize,
    pub modes: Vec<Mode>,
    pub leds: usize,
    pub colors: Vec<Rgb>,
}

impl Controller {
    pub fn mode(&self) -> Option<&Mode> {
        self.modes.get(self.active_mode)
    }
}

// ---- encoding ----

pub fn header(device: u32, id: u32, len: usize) -> [u8; 16] {
    let mut h = [0u8; 16];
    h[..4].copy_from_slice(b"ORGB");
    h[4..8].copy_from_slice(&device.to_le_bytes());
    h[8..12].copy_from_slice(&id.to_le_bytes());
    h[12..16].copy_from_slice(&(len as u32).to_le_bytes());
    h
}

pub fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&((s.len() + 1) as u16).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    out.push(0);
}

pub fn put_colors(out: &mut Vec<u8>, colors: &[Rgb]) {
    out.extend_from_slice(&(colors.len() as u16).to_le_bytes());
    for c in colors {
        out.extend_from_slice(&[c[0], c[1], c[2], 0]);
    }
}

/// A mode as the protocol writes it (in controller data and in UPDATE_MODE).
pub fn put_mode(out: &mut Vec<u8>, m: &Mode, proto: u32) {
    put_str(out, &m.name);
    out.extend_from_slice(&m.value.to_le_bytes());
    let mut u = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    u(m.flags);
    u(m.speed_min);
    u(m.speed_max);
    if proto >= 3 {
        u(m.brightness_min);
        u(m.brightness_max);
    }
    u(m.colors_min);
    u(m.colors_max);
    u(m.speed);
    if proto >= 3 {
        u(m.brightness);
    }
    u(m.direction);
    u(m.color_mode);
    put_colors(out, &m.colors);
}

/// UPDATE_LEDS data: every LED of the device.
pub fn leds_payload(colors: &[Rgb]) -> Vec<u8> {
    let mut body = vec![];
    put_colors(&mut body, colors);
    let mut out = ((body.len() + 4) as u32).to_le_bytes().to_vec();
    out.extend(body);
    out
}

/// UPDATE_MODE data: the mode's index and its fields.
pub fn mode_payload(index: usize, m: &Mode, proto: u32) -> Vec<u8> {
    let mut body = (index as i32).to_le_bytes().to_vec();
    put_mode(&mut body, m, proto);
    let mut out = ((body.len() + 4) as u32).to_le_bytes().to_vec();
    out.extend(body);
    out
}

// ---- decoding ----

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|e| *e <= self.buf.len()).ok_or("OpenRGB sent a short packet")?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    pub fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }
    pub fn string(&mut self) -> Result<String, String> {
        let n = self.u16()? as usize;
        let b = self.take(n)?;
        let b = b.strip_suffix(&[0]).unwrap_or(b);
        Ok(String::from_utf8_lossy(b).into_owned())
    }
    pub fn colors(&mut self) -> Result<Vec<Rgb>, String> {
        let n = self.u16()? as usize;
        (0..n).map(|_| self.take(4).map(|c| [c[0], c[1], c[2]])).collect()
    }
}

fn mode(r: &mut Reader, proto: u32) -> Result<Mode, String> {
    let name = r.string()?;
    let value = r.i32()?;
    let flags = r.u32()?;
    let (speed_min, speed_max) = (r.u32()?, r.u32()?);
    let (brightness_min, brightness_max) = if proto >= 3 { (r.u32()?, r.u32()?) } else { (0, 0) };
    let (colors_min, colors_max) = (r.u32()?, r.u32()?);
    let speed = r.u32()?;
    let brightness = if proto >= 3 { r.u32()? } else { 0 };
    let direction = r.u32()?;
    let color_mode = r.u32()?;
    let colors = r.colors()?;
    Ok(Mode { name, value, flags, speed_min, speed_max, brightness_min, brightness_max, colors_min, colors_max, speed, brightness, direction, color_mode, colors })
}

/// REQUEST_CONTROLLER_DATA reply.
pub fn parse_controller(data: &[u8], proto: u32) -> Result<Controller, String> {
    let mut r = Reader::new(data);
    r.u32()?; // data size
    r.i32()?; // device type
    let name = r.string()?;
    let vendor = if proto >= 1 { r.string()? } else { String::new() };
    r.string()?; // description
    r.string()?; // version
    r.string()?; // serial
    let location = r.string()?;
    let n_modes = r.u16()? as usize;
    let active_mode = r.i32()?.max(0) as usize;
    let modes = (0..n_modes).map(|_| mode(&mut r, proto)).collect::<Result<Vec<_>, _>>()?;
    let n_zones = r.u16()?;
    for _ in 0..n_zones {
        r.string()?; // name
        r.i32()?; // type
        r.skip(12)?; // leds min, max, count
        let matrix = r.u16()? as usize; // bytes: height, width, map
        r.skip(matrix)?;
        if proto >= 4 {
            let segs = r.u16()?;
            for _ in 0..segs {
                r.string()?;
                r.skip(12)?; // type, start, count
            }
        }
    }
    let n_leds = r.u16()? as usize;
    for _ in 0..n_leds {
        r.string()?;
        r.u32()?;
    }
    let colors = r.colors()?;
    Ok(Controller { name, vendor, location, active_mode, modes, leds: n_leds, colors })
}

/// REQUEST_PROFILE_LIST reply.
pub fn parse_profiles(data: &[u8]) -> Result<Vec<String>, String> {
    let mut r = Reader::new(data);
    r.u32()?; // data size
    let n = r.u16()?;
    (0..n).map(|_| r.string()).collect()
}

// ---- connection ----

pub struct Client {
    stream: TcpStream,
    pub proto: u32,
}

impl Client {
    pub fn connect(host: &str, port: u16) -> Result<Client, String> {
        let unreachable = |e: std::io::Error| format!("Can't reach OpenRGB at {host}:{port} ({e}). Start OpenRGB and turn on its SDK server (SDK Server tab ▸ Start Server)");
        let addr = (host, port).to_socket_addrs().map_err(unreachable)?.next().ok_or_else(|| format!("Unknown host {host}"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(unreachable)?;
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        stream.set_nodelay(true).ok();
        let mut c = Client { stream, proto: 0 };
        let server = c.request(0, REQUEST_PROTOCOL_VERSION, &CLIENT_PROTOCOL.to_le_bytes())?;
        // Servers before protocol 1 don't answer this; they'd time out above.
        let server = Reader::new(&server).u32()?;
        c.proto = server.min(CLIENT_PROTOCOL);
        let mut name = b"KnobDeck".to_vec();
        name.push(0);
        c.send(0, SET_CLIENT_NAME, &name)?;
        Ok(c)
    }

    pub fn send(&mut self, device: u32, id: u32, data: &[u8]) -> Result<(), String> {
        let mut pkt = header(device, id, data.len()).to_vec();
        pkt.extend_from_slice(data);
        self.stream.write_all(&pkt).map_err(|e| format!("OpenRGB connection: {e}"))
    }

    /// Send and wait for the reply with the same packet id (skipping device list notices).
    pub fn request(&mut self, device: u32, id: u32, data: &[u8]) -> Result<Vec<u8>, String> {
        self.send(device, id, data)?;
        loop {
            let mut h = [0u8; 16];
            self.stream.read_exact(&mut h).map_err(|e| format!("OpenRGB didn't answer: {e}"))?;
            if &h[..4] != b"ORGB" {
                return Err("Not an OpenRGB SDK server on that port".into());
            }
            let got = u32::from_le_bytes(h[8..12].try_into().unwrap());
            let len = u32::from_le_bytes(h[12..16].try_into().unwrap()) as usize;
            if len > 16 << 20 {
                return Err("OpenRGB sent an oversized packet".into());
            }
            let mut data = vec![0u8; len];
            self.stream.read_exact(&mut data).map_err(|e| format!("OpenRGB connection: {e}"))?;
            if got == id {
                return Ok(data);
            }
            // DEVICE_LIST_UPDATED and anything else unasked for: ignore.
        }
    }

    pub fn count(&mut self) -> Result<u32, String> {
        Reader::new(&self.request(0, REQUEST_CONTROLLER_COUNT, &[])?).u32()
    }

    pub fn controller(&mut self, index: u32) -> Result<Controller, String> {
        let data = self.request(index, REQUEST_CONTROLLER_DATA, &self.proto.to_le_bytes())?;
        parse_controller(&data, self.proto)
    }

    pub fn controllers(&mut self) -> Result<Vec<Controller>, String> {
        let n = self.count()?;
        (0..n).map(|i| self.controller(i)).collect()
    }

    fn need_profiles(&self) -> Result<(), String> {
        if self.proto < 2 {
            return Err("This OpenRGB is too old for profiles over the SDK; update OpenRGB (0.6 or newer)".into());
        }
        Ok(())
    }

    pub fn profiles(&mut self) -> Result<Vec<String>, String> {
        self.need_profiles()?;
        parse_profiles(&self.request(0, REQUEST_PROFILE_LIST, &[])?)
    }

    pub fn load_profile(&mut self, name: &str) -> Result<(), String> {
        self.need_profiles()?;
        let mut data = name.as_bytes().to_vec();
        data.push(0);
        self.send(0, REQUEST_LOAD_PROFILE, &data)
    }

    pub fn set_custom_mode(&mut self, index: u32) -> Result<(), String> {
        self.send(index, SET_CUSTOM_MODE, &[])
    }

    pub fn update_leds(&mut self, index: u32, colors: &[Rgb]) -> Result<(), String> {
        self.send(index, UPDATE_LEDS, &leds_payload(colors))
    }

    pub fn update_mode(&mut self, index: u32, mode_index: usize, m: &Mode) -> Result<(), String> {
        let data = mode_payload(mode_index, m, self.proto);
        self.send(index, UPDATE_MODE, &data)
    }
}

pub fn hex(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([b(0)?, b(2)?, b(4)?])
}

pub fn scale(c: Rgb, pct: f64) -> Rgb {
    let f = (pct / 100.0).clamp(0.0, 1.0);
    c.map(|v| (v as f64 * f).round() as u8)
}
