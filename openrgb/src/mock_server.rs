//! A stand-in OpenRGB SDK server for the tests and for `cargo run --example mock-server`
//! (to drive the built plugin with `knobdeck-plugin drive`). Its controller encoding is written out field by
//! field here (not with the plugin's `put_mode`), following OpenRGB's
//! `RGBController::GetDeviceDescription` / `GetModeDescription`, so a field-order mistake in the
//! plugin shows as a failure instead of cancelling out.

use crate::sdk::{self, Rgb};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct MMode {
    pub name: &'static str,
    pub flags: u32,
    pub bri: (u32, u32, u32),
    pub colors_min: u32,
    pub color_mode: u32,
    pub colors: Vec<Rgb>,
}

#[derive(Clone)]
pub struct MDev {
    pub name: &'static str,
    pub modes: Vec<MMode>,
    pub active: usize,
    pub leds: Vec<Rgb>,
    pub matrix: bool,
}

pub struct World {
    pub proto: u32,
    pub devs: Vec<MDev>,
    pub profiles: Vec<&'static str>,
    pub loaded: Vec<String>,
    pub client_names: Vec<String>,
}

fn s(out: &mut Vec<u8>, v: &str) {
    out.extend(((v.len() + 1) as u16).to_le_bytes());
    out.extend(v.as_bytes());
    out.push(0);
}
fn u(out: &mut Vec<u8>, v: u32) {
    out.extend(v.to_le_bytes());
}
fn cols(out: &mut Vec<u8>, c: &[Rgb]) {
    out.extend((c.len() as u16).to_le_bytes());
    for c in c {
        out.extend([c[0], c[1], c[2], 0]);
    }
}

fn describe(d: &MDev, proto: u32) -> Vec<u8> {
    let mut b = vec![];
    u(&mut b, 2); // type: motherboard
    s(&mut b, d.name);
    if proto >= 1 {
        s(&mut b, "Vendor");
    }
    s(&mut b, "Description");
    s(&mut b, "1.0");
    s(&mut b, "SN123");
    s(&mut b, "HID: /dev/hidraw0");
    b.extend((d.modes.len() as u16).to_le_bytes());
    b.extend((d.active as i32).to_le_bytes());
    for (i, m) in d.modes.iter().enumerate() {
        s(&mut b, m.name);
        b.extend((i as i32).to_le_bytes()); // value
        u(&mut b, m.flags);
        u(&mut b, 0); // speed_min
        u(&mut b, 100); // speed_max
        if proto >= 3 {
            u(&mut b, m.bri.0);
            u(&mut b, m.bri.1);
        }
        u(&mut b, m.colors_min);
        u(&mut b, m.colors.len() as u32); // colors_max
        u(&mut b, 50); // speed
        if proto >= 3 {
            u(&mut b, m.bri.2);
        }
        u(&mut b, 0); // direction
        u(&mut b, m.color_mode);
        cols(&mut b, &m.colors);
    }
    // One zone holding every LED, optionally with a matrix map and (protocol 4) a segment.
    b.extend(1u16.to_le_bytes());
    s(&mut b, "Zone");
    b.extend(1i32.to_le_bytes());
    let n = d.leds.len() as u32;
    u(&mut b, n);
    u(&mut b, n);
    u(&mut b, n);
    if d.matrix {
        let (h, w) = (2u32, n.div_ceil(2));
        b.extend(((8 + h * w * 4) as u16).to_le_bytes());
        u(&mut b, h);
        u(&mut b, w);
        for k in 0..h * w {
            u(&mut b, k);
        }
    } else {
        b.extend(0u16.to_le_bytes());
    }
    if proto >= 4 {
        b.extend(1u16.to_le_bytes());
        s(&mut b, "Segment");
        b.extend(0i32.to_le_bytes());
        u(&mut b, 0);
        u(&mut b, n);
    }
    b.extend((d.leds.len() as u16).to_le_bytes());
    for i in 0..d.leds.len() {
        s(&mut b, &format!("LED {i}"));
        u(&mut b, i as u32);
    }
    cols(&mut b, &d.leds);
    let mut out = ((b.len() + 4) as u32).to_le_bytes().to_vec();
    out.extend(b);
    out
}

fn reply(st: &mut TcpStream, dev: u32, id: u32, data: &[u8]) {
    st.write_all(&sdk::header(dev, id, data.len())).unwrap();
    st.write_all(data).unwrap();
}

fn serve(mut st: TcpStream, world: Arc<Mutex<World>>) {
    let mut client_proto = 0;
    // An unasked-for notice first: clients must skip it.
    reply(&mut st, 0, sdk::DEVICE_LIST_UPDATED, &[]);
    loop {
        let mut h = [0u8; 16];
        if st.read_exact(&mut h).is_err() {
            return;
        }
        assert_eq!(&h[..4], b"ORGB");
        let dev = u32::from_le_bytes(h[4..8].try_into().unwrap()) as usize;
        let id = u32::from_le_bytes(h[8..12].try_into().unwrap());
        let mut data = vec![0u8; u32::from_le_bytes(h[12..16].try_into().unwrap()) as usize];
        st.read_exact(&mut data).unwrap();
        let mut w = world.lock().unwrap();
        let mut r = sdk::Reader::new(&data);
        match id {
            sdk::REQUEST_PROTOCOL_VERSION => {
                client_proto = r.u32().unwrap().min(w.proto);
                let p = w.proto;
                reply(&mut st, 0, id, &p.to_le_bytes());
            }
            sdk::SET_CLIENT_NAME => w.client_names.push(String::from_utf8_lossy(data.strip_suffix(&[0]).unwrap()).into()),
            sdk::REQUEST_CONTROLLER_COUNT => {
                let n = w.devs.len() as u32;
                reply(&mut st, 0, id, &n.to_le_bytes());
            }
            sdk::REQUEST_CONTROLLER_DATA => {
                let asked = r.u32().unwrap();
                let out = describe(&w.devs[dev], asked.min(client_proto));
                reply(&mut st, dev as u32, id, &out);
            }
            sdk::REQUEST_PROFILE_LIST => {
                let mut b = vec![];
                b.extend((w.profiles.len() as u16).to_le_bytes());
                for p in &w.profiles {
                    s(&mut b, p);
                }
                let mut out = ((b.len() + 4) as u32).to_le_bytes().to_vec();
                out.extend(b);
                reply(&mut st, 0, id, &out);
            }
            sdk::REQUEST_LOAD_PROFILE => w.loaded.push(String::from_utf8_lossy(data.strip_suffix(&[0]).unwrap()).into()),
            sdk::SET_CUSTOM_MODE => {
                let d = &mut w.devs[dev];
                if let Some(k) = d.modes.iter().position(|m| m.name == "Direct" || m.name == "Static") {
                    d.active = k;
                }
            }
            sdk::UPDATE_LEDS => {
                let size = r.u32().unwrap() as usize;
                assert_eq!(size, data.len());
                let c = r.colors().unwrap();
                let d = &mut w.devs[dev];
                assert_eq!(c.len(), d.leds.len());
                d.leds = c;
            }
            sdk::UPDATE_MODE => {
                let size = r.u32().unwrap() as usize;
                assert_eq!(size, data.len());
                let k = r.i32().unwrap() as usize;
                let name = r.string().unwrap();
                r.i32().unwrap();
                let flags = r.u32().unwrap();
                r.skip(8).unwrap();
                let (bmin, bmax) = if client_proto >= 3 { (r.u32().unwrap(), r.u32().unwrap()) } else { (0, 0) };
                let cmin = r.u32().unwrap();
                r.skip(8).unwrap(); // colors_max, speed
                let bri = if client_proto >= 3 { r.u32().unwrap() } else { 0 };
                r.skip(4).unwrap();
                let color_mode = r.u32().unwrap();
                let colors = r.colors().unwrap();
                let d = &mut w.devs[dev];
                assert_eq!(d.modes[k].name, name);
                d.modes[k] = MMode { name: d.modes[k].name, flags, bri: (bmin, bmax, bri), colors_min: cmin, color_mode, colors };
                d.active = k;
            }
            other => panic!("unexpected packet {other}"),
        }
    }
}

/// A server on a free port.
#[allow(dead_code)]
pub fn start(proto: u32) -> (u16, Arc<Mutex<World>>) {
    listen(0, proto)
}

/// A server on `port`.
#[allow(dead_code)]
pub fn serve_on(port: u16, proto: u32) -> Arc<Mutex<World>> {
    listen(port, proto).1
}

fn listen(port: u16, proto: u32) -> (u16, Arc<Mutex<World>>) {
    let keyboard = MDev {
        name: "Keyboard",
        modes: vec![
            MMode { name: "Direct", flags: 1 << 5, bri: (0, 0, 0), colors_min: 0, color_mode: sdk::COLOR_MODE_PER_LED, colors: vec![] },
            MMode { name: "Rainbow Wave", flags: 1 | (1 << 4), bri: (0, 100, 100), colors_min: 0, color_mode: 0, colors: vec![] },
        ],
        active: 1,
        leds: vec![[0, 0, 0]; 5],
        matrix: true,
    };
    let ram = MDev {
        name: "RAM Stick",
        modes: vec![MMode { name: "Static", flags: (1 << 4) | (1 << 6), bri: (0, 255, 255), colors_min: 1, color_mode: sdk::COLOR_MODE_MODE_SPECIFIC, colors: vec![[1, 2, 3]] }],
        active: 0,
        leds: vec![[0, 0, 0]; 2],
        matrix: false,
    };
    let world = Arc::new(Mutex::new(World { proto, devs: vec![keyboard, ram], profiles: vec!["Gaming", "Work"], loaded: vec![], client_names: vec![] }));
    let l = TcpListener::bind(("127.0.0.1", port)).unwrap();
    let port = l.local_addr().unwrap().port();
    let w = world.clone();
    std::thread::spawn(move || {
        for st in l.incoming().flatten() {
            let w = w.clone();
            std::thread::spawn(move || serve(st, w));
        }
    });
    (port, world)
}

