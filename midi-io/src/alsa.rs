//! Linux: the ALSA sequencer, the MIDI router every Linux desktop has (PipeWire bridges it to its
//! own graph, and DAWs such as Ardour, Bitwig, REAPER and Qtractor list its ports).
//!
//! `libasound.so.2` is loaded at run time with `dlopen`, so the build needs no ALSA headers and a
//! system without it gets a clear error. Only opaque handles and the sequencer event layout are used;
//! `snd_midi_event_*` converts between raw MIDI bytes and events, so no event field but the routing
//! ones is written here.

use crate::{Callback, Result};
use libc::{c_char, c_int, c_long, c_short, c_uint, c_void, pollfd, size_t};
use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

#[repr(C)]
struct Seq(c_void);
#[repr(C)]
struct ClientInfo(c_void);
#[repr(C)]
struct PortInfo(c_void);
#[repr(C)]
struct Codec(c_void);

/// `snd_seq_event_t` (`struct snd_seq_event` of the kernel ABI): 28 bytes, 4-byte aligned.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Event {
    kind: u8,
    flags: u8,
    tag: u8,
    queue: u8,
    time: [u32; 2],
    source: [u8; 2], // client, port
    dest: [u8; 2],
    data: [u32; 3],
}
const _: () = assert!(std::mem::size_of::<Event>() == 28);

const OPEN_DUPLEX: c_int = 3;
const CAP_READ: c_uint = 1 << 0;
const CAP_WRITE: c_uint = 1 << 1;
const CAP_SUBS_READ: c_uint = 1 << 5;
const CAP_SUBS_WRITE: c_uint = 1 << 6;
const CAP_NO_EXPORT: c_uint = 1 << 7;
const TYPE_MIDI_GENERIC: c_uint = 1 << 1;
const TYPE_APPLICATION: c_uint = 1 << 20;
const QUEUE_DIRECT: u8 = 253;
const ADDRESS_SUBSCRIBERS: u8 = 254;
const ADDRESS_UNKNOWN: u8 = 253;
const POLLIN: c_short = libc::POLLIN;

struct Lib {
    seq_open: unsafe extern "C" fn(*mut *mut Seq, *const c_char, c_int, c_int) -> c_int,
    seq_close: unsafe extern "C" fn(*mut Seq) -> c_int,
    seq_set_client_name: unsafe extern "C" fn(*mut Seq, *const c_char) -> c_int,
    seq_client_id: unsafe extern "C" fn(*mut Seq) -> c_int,
    seq_create_simple_port: unsafe extern "C" fn(*mut Seq, *const c_char, c_uint, c_uint) -> c_int,
    seq_connect_to: unsafe extern "C" fn(*mut Seq, c_int, c_int, c_int) -> c_int,
    seq_connect_from: unsafe extern "C" fn(*mut Seq, c_int, c_int, c_int) -> c_int,
    seq_event_output_direct: unsafe extern "C" fn(*mut Seq, *mut Event) -> c_int,
    seq_event_input: unsafe extern "C" fn(*mut Seq, *mut *mut Event) -> c_int,
    seq_nonblock: unsafe extern "C" fn(*mut Seq, c_int) -> c_int,
    seq_poll_descriptors_count: unsafe extern "C" fn(*mut Seq, c_short) -> c_int,
    seq_poll_descriptors: unsafe extern "C" fn(*mut Seq, *mut pollfd, c_uint, c_short) -> c_int,
    client_info_malloc: unsafe extern "C" fn(*mut *mut ClientInfo) -> c_int,
    client_info_free: unsafe extern "C" fn(*mut ClientInfo),
    client_info_set_client: unsafe extern "C" fn(*mut ClientInfo, c_int),
    client_info_get_client: unsafe extern "C" fn(*const ClientInfo) -> c_int,
    client_info_get_name: unsafe extern "C" fn(*const ClientInfo) -> *const c_char,
    query_next_client: unsafe extern "C" fn(*mut Seq, *mut ClientInfo) -> c_int,
    port_info_malloc: unsafe extern "C" fn(*mut *mut PortInfo) -> c_int,
    port_info_free: unsafe extern "C" fn(*mut PortInfo),
    port_info_set_client: unsafe extern "C" fn(*mut PortInfo, c_int),
    port_info_set_port: unsafe extern "C" fn(*mut PortInfo, c_int),
    port_info_get_port: unsafe extern "C" fn(*const PortInfo) -> c_int,
    port_info_get_capability: unsafe extern "C" fn(*const PortInfo) -> c_uint,
    port_info_get_name: unsafe extern "C" fn(*const PortInfo) -> *const c_char,
    query_next_port: unsafe extern "C" fn(*mut Seq, *mut PortInfo) -> c_int,
    midi_event_new: unsafe extern "C" fn(size_t, *mut *mut Codec) -> c_int,
    midi_event_free: unsafe extern "C" fn(*mut Codec),
    midi_event_no_status: unsafe extern "C" fn(*mut Codec, c_int),
    midi_event_encode_byte: unsafe extern "C" fn(*mut Codec, c_int, *mut Event) -> c_int,
    midi_event_decode: unsafe extern "C" fn(*mut Codec, *mut u8, c_long, *const Event) -> c_long,
}

fn lib() -> Result<&'static Lib> {
    static LIB: OnceLock<std::result::Result<Lib, String>> = OnceLock::new();
    LIB.get_or_init(|| unsafe { load() }).as_ref().map_err(Clone::clone)
}

/// A symbol as the function pointer type of the field it fills (both are one pointer wide).
unsafe fn fn_ptr<T: Copy>(p: *mut c_void) -> T {
    const { assert!(std::mem::size_of::<*mut c_void>() == std::mem::size_of::<usize>()) };
    debug_assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<*mut c_void>());
    std::mem::transmute_copy::<*mut c_void, T>(&p)
}

unsafe fn load() -> std::result::Result<Lib, String> {
    let h = libc::dlopen(c"libasound.so.2".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
    if h.is_null() {
        return Err("MIDI needs ALSA's libasound.so.2 (package libasound2 / alsa-lib), which isn't installed".into());
    }
    macro_rules! sym {
        ($name:literal) => {{
            let p = libc::dlsym(h, concat!($name, "\0").as_ptr() as *const c_char);
            if p.is_null() {
                return Err(format!("libasound.so.2 has no {}", $name));
            }
            fn_ptr(p)
        }};
    }
    Ok(Lib {
        seq_open: sym!("snd_seq_open"),
        seq_close: sym!("snd_seq_close"),
        seq_set_client_name: sym!("snd_seq_set_client_name"),
        seq_client_id: sym!("snd_seq_client_id"),
        seq_create_simple_port: sym!("snd_seq_create_simple_port"),
        seq_connect_to: sym!("snd_seq_connect_to"),
        seq_connect_from: sym!("snd_seq_connect_from"),
        seq_event_output_direct: sym!("snd_seq_event_output_direct"),
        seq_event_input: sym!("snd_seq_event_input"),
        seq_nonblock: sym!("snd_seq_nonblock"),
        seq_poll_descriptors_count: sym!("snd_seq_poll_descriptors_count"),
        seq_poll_descriptors: sym!("snd_seq_poll_descriptors"),
        client_info_malloc: sym!("snd_seq_client_info_malloc"),
        client_info_free: sym!("snd_seq_client_info_free"),
        client_info_set_client: sym!("snd_seq_client_info_set_client"),
        client_info_get_client: sym!("snd_seq_client_info_get_client"),
        client_info_get_name: sym!("snd_seq_client_info_get_name"),
        query_next_client: sym!("snd_seq_query_next_client"),
        port_info_malloc: sym!("snd_seq_port_info_malloc"),
        port_info_free: sym!("snd_seq_port_info_free"),
        port_info_set_client: sym!("snd_seq_port_info_set_client"),
        port_info_set_port: sym!("snd_seq_port_info_set_port"),
        port_info_get_port: sym!("snd_seq_port_info_get_port"),
        port_info_get_capability: sym!("snd_seq_port_info_get_capability"),
        port_info_get_name: sym!("snd_seq_port_info_get_name"),
        query_next_port: sym!("snd_seq_query_next_port"),
        midi_event_new: sym!("snd_midi_event_new"),
        midi_event_free: sym!("snd_midi_event_free"),
        midi_event_no_status: sym!("snd_midi_event_no_status"),
        midi_event_encode_byte: sym!("snd_midi_event_encode_byte"),
        midi_event_decode: sym!("snd_midi_event_decode"),
    })
}

fn errno_text(code: c_int) -> String {
    std::io::Error::from_raw_os_error(-code).to_string()
}

/// An open sequencer client. Closed on drop; used by one thread at a time.
struct Handle {
    lib: &'static Lib,
    seq: *mut Seq,
}
unsafe impl Send for Handle {}

impl Handle {
    fn open(client: &str) -> Result<Self> {
        let lib = lib()?;
        let mut seq = std::ptr::null_mut();
        let rc = unsafe { (lib.seq_open)(&mut seq, c"default".as_ptr(), OPEN_DUPLEX, 0) };
        if rc < 0 || seq.is_null() {
            return Err(format!("Couldn't open the ALSA sequencer ({}): is the snd-seq module loaded?", errno_text(rc)));
        }
        let h = Handle { lib, seq };
        let name = CString::new(client).unwrap_or_default();
        unsafe { (lib.seq_set_client_name)(seq, name.as_ptr()) };
        Ok(h)
    }

    fn port(&self, name: &str, caps: c_uint) -> Result<u8> {
        let n = CString::new(name).unwrap_or_default();
        let rc = unsafe { (self.lib.seq_create_simple_port)(self.seq, n.as_ptr(), caps, TYPE_MIDI_GENERIC | TYPE_APPLICATION) };
        if rc < 0 {
            return Err(format!("Couldn't create the MIDI port \"{name}\": {}", errno_text(rc)));
        }
        Ok(rc as u8)
    }

    /// Every port of other clients: (`Client:Port`, client, port, capabilities).
    fn ports(&self) -> Vec<(String, c_int, c_int, c_uint)> {
        let l = self.lib;
        let me = unsafe { (l.seq_client_id)(self.seq) };
        let mut out = Vec::new();
        unsafe {
            let mut ci = std::ptr::null_mut();
            let mut pi = std::ptr::null_mut();
            if (l.client_info_malloc)(&mut ci) < 0 || ci.is_null() {
                return out;
            }
            if (l.port_info_malloc)(&mut pi) < 0 || pi.is_null() {
                (l.client_info_free)(ci);
                return out;
            }
            (l.client_info_set_client)(ci, -1);
            while (l.query_next_client)(self.seq, ci) >= 0 {
                let client = (l.client_info_get_client)(ci);
                if client == 0 || client == me {
                    continue; // System (timer, announcements) and ourselves
                }
                let cname = CStr::from_ptr((l.client_info_get_name)(ci)).to_string_lossy().into_owned();
                (l.port_info_set_client)(pi, client);
                (l.port_info_set_port)(pi, -1);
                while (l.query_next_port)(self.seq, pi) >= 0 {
                    let caps = (l.port_info_get_capability)(pi);
                    if caps & CAP_NO_EXPORT != 0 {
                        continue;
                    }
                    let pname = CStr::from_ptr((l.port_info_get_name)(pi)).to_string_lossy().into_owned();
                    out.push((format!("{cname}:{pname}"), client, (l.port_info_get_port)(pi), caps));
                }
            }
            (l.port_info_free)(pi);
            (l.client_info_free)(ci);
        }
        out
    }

    fn find(&self, name: &str, caps: c_uint) -> Result<(c_int, c_int)> {
        self.ports().into_iter().find(|p| p.0 == name && p.3 & caps == caps).map(|p| (p.1, p.2)).ok_or_else(|| format!("MIDI port \"{name}\" isn't there"))
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { (self.lib.seq_close)(self.seq) };
    }
}

/// A raw-MIDI ⇄ event converter.
struct Coder {
    lib: &'static Lib,
    dev: *mut Codec,
}
unsafe impl Send for Coder {}

impl Coder {
    fn new(lib: &'static Lib, size: usize) -> Result<Self> {
        let mut dev = std::ptr::null_mut();
        let rc = unsafe { (lib.midi_event_new)(size, &mut dev) };
        if rc < 0 || dev.is_null() {
            return Err(format!("ALSA MIDI coder: {}", errno_text(rc)));
        }
        unsafe { (lib.midi_event_no_status)(dev, 1) }; // every decoded message gets its status byte
        Ok(Coder { lib, dev })
    }
}

impl Drop for Coder {
    fn drop(&mut self) {
        unsafe { (self.lib.midi_event_free)(self.dev) };
    }
}

fn list(caps: c_uint) -> Result<Vec<String>> {
    let h = Handle::open("KnobDeck")?;
    Ok(h.ports().into_iter().filter(|p| p.3 & caps == caps).map(|p| p.0).collect())
}

pub fn outputs() -> Result<Vec<String>> {
    list(CAP_WRITE | CAP_SUBS_WRITE)
}

pub fn inputs() -> Result<Vec<String>> {
    list(CAP_READ | CAP_SUBS_READ)
}

pub struct Output(Mutex<(Handle, Coder, u8)>);

impl Output {
    fn new(client: &str, name: &str, target: Option<&str>) -> Result<Self> {
        let h = Handle::open(client)?;
        let port = h.port(name, CAP_READ | CAP_SUBS_READ)?;
        if let Some(t) = target {
            let (c, p) = h.find(t, CAP_WRITE | CAP_SUBS_WRITE)?;
            let rc = unsafe { (h.lib.seq_connect_to)(h.seq, port as c_int, c, p) };
            if rc < 0 {
                return Err(format!("Couldn't connect to MIDI output \"{t}\": {}", errno_text(rc)));
            }
        }
        let coder = Coder::new(h.lib, 4096)?;
        Ok(Output(Mutex::new((h, coder, port))))
    }

    pub fn open_virtual(client: &str, name: &str) -> Result<Self> {
        Self::new(client, name, None)
    }

    pub fn connect(client: &str, port: &str) -> Result<Self> {
        Self::new(client, "out", Some(port))
    }

    pub fn send(&self, message: &[u8]) -> Result<()> {
        let g = self.0.lock().unwrap();
        let (h, coder, port) = (&g.0, &g.1, g.2);
        for &b in message {
            let mut ev = Event::default();
            let done = unsafe { (h.lib.midi_event_encode_byte)(coder.dev, b as c_int, &mut ev) };
            if done < 0 {
                return Err(format!("Not a valid MIDI message: {}", errno_text(done)));
            }
            if done == 0 {
                continue;
            }
            ev.queue = QUEUE_DIRECT;
            ev.source = [0, port];
            ev.dest = [ADDRESS_SUBSCRIBERS, ADDRESS_UNKNOWN];
            let rc = unsafe { (h.lib.seq_event_output_direct)(h.seq, &mut ev) };
            if rc < 0 {
                return Err(format!("Couldn't send MIDI: {}", errno_text(rc)));
            }
        }
        Ok(())
    }
}

pub struct Input {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Input {
    fn new(client: &str, name: &str, source: Option<&str>, cb: Callback) -> Result<Self> {
        let h = Handle::open(client)?;
        let port = h.port(name, CAP_WRITE | CAP_SUBS_WRITE)?;
        if let Some(s) = source {
            let (c, p) = h.find(s, CAP_READ | CAP_SUBS_READ)?;
            let rc = unsafe { (h.lib.seq_connect_from)(h.seq, port as c_int, c, p) };
            if rc < 0 {
                return Err(format!("Couldn't connect to MIDI input \"{s}\": {}", errno_text(rc)));
            }
        }
        unsafe { (h.lib.seq_nonblock)(h.seq, 1) };
        let coder = Coder::new(h.lib, 4096)?;
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::Builder::new().name("midi-in".into()).spawn(move || receive(h, coder, flag, cb)).map_err(|e| e.to_string())?;
        Ok(Input { stop, thread: Some(thread) })
    }

    pub fn open_virtual(client: &str, name: &str, cb: Callback) -> Result<Self> {
        Self::new(client, name, None, cb)
    }

    pub fn connect(client: &str, port: &str, cb: Callback) -> Result<Self> {
        Self::new(client, "in", Some(port), cb)
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn receive(h: Handle, coder: Coder, stop: Arc<AtomicBool>, mut cb: Callback) {
    let l = h.lib;
    let n = unsafe { (l.seq_poll_descriptors_count)(h.seq, POLLIN) }.max(1) as usize;
    let mut fds = vec![pollfd { fd: -1, events: 0, revents: 0 }; n];
    let n = unsafe { (l.seq_poll_descriptors)(h.seq, fds.as_mut_ptr(), n as c_uint, POLLIN) }.max(0) as usize;
    fds.truncate(n);
    let mut buf = vec![0u8; 4096];
    let mut sysex: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        // Wake up now and then to see whether the port was closed.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 200) };
        loop {
            let mut ev: *mut Event = std::ptr::null_mut();
            let rc = unsafe { (l.seq_event_input)(h.seq, &mut ev) };
            if rc == -libc::ENOSPC {
                continue; // the kernel dropped events while we were busy; carry on
            }
            if rc < 0 || ev.is_null() {
                break; // -EAGAIN: nothing more now
            }
            let len = unsafe { (l.midi_event_decode)(coder.dev, buf.as_mut_ptr(), buf.len() as c_long, ev) };
            if len <= 0 {
                continue; // not a MIDI message (port subscribed / unsubscribed…)
            }
            let m = &buf[..len as usize];
            // ALSA hands long SysEx over in chunks: put them back together.
            if !sysex.is_empty() || m[0] == 0xF0 {
                sysex.extend_from_slice(m);
                if sysex.last() == Some(&0xF7) {
                    cb(&sysex);
                    sysex.clear();
                } else if sysex.len() > 65536 {
                    sysex.clear();
                }
                continue;
            }
            cb(m);
        }
    }
}

#[cfg(test)]
mod tests {
    //! Real ALSA loopback: needs /dev/snd/seq (the snd-seq module); skipped where there is none.
    use super::*;
    use std::sync::mpsc::{channel, Receiver};
    use std::time::Duration;

    fn have_seq() -> bool {
        lib().is_ok() && Handle::open("KnobDeck test").is_ok()
    }

    fn collect(rx: &Receiver<Vec<u8>>, n: usize) -> Vec<Vec<u8>> {
        (0..n).filter_map(|_| rx.recv_timeout(Duration::from_secs(2)).ok()).collect()
    }

    fn messages() -> Vec<Vec<u8>> {
        let mut lcd = vec![0xF0, 0x00, 0x00, 0x66, 0x14, 0x12, 0x00];
        lcd.extend(b"Vocals Drums  Bass   ");
        lcd.push(0xF7);
        vec![vec![0x90, 60, 100], vec![0x80, 60, 0], vec![0xB3, 7, 127], vec![0xC0, 5], vec![0xE8, 0x7F, 0x7F], lcd]
    }

    #[test]
    fn virtual_output_to_connected_input() {
        if !have_seq() {
            eprintln!("no ALSA sequencer here: skipped");
            return;
        }
        let out = crate::Output::open_virtual("KnobDeck test out", "loop A").unwrap();
        let name = inputs().unwrap().into_iter().find(|p| p == "KnobDeck test out:loop A").expect("the virtual port is listed");
        let (tx, rx) = channel();
        let _input = crate::Input::connect("KnobDeck test in", &name, move |m| tx.send(m.to_vec()).unwrap()).unwrap();
        for m in messages() {
            out.send(&m).unwrap();
        }
        assert_eq!(collect(&rx, 6), messages());
    }

    #[test]
    fn connected_output_to_virtual_input() {
        if !have_seq() {
            eprintln!("no ALSA sequencer here: skipped");
            return;
        }
        let (tx, rx) = channel();
        let _input = crate::Input::open_virtual("KnobDeck test sink", "loop B", move |m| tx.send(m.to_vec()).unwrap()).unwrap();
        assert!(outputs().unwrap().iter().any(|p| p == "KnobDeck test sink:loop B"));
        let out = crate::Output::connect("KnobDeck test src", "KnobDeck test sink:loop B").unwrap();
        for m in messages() {
            out.send(&m).unwrap();
        }
        assert_eq!(collect(&rx, 6), messages());
    }

    #[test]
    fn missing_port_is_a_clear_error() {
        if !have_seq() {
            return;
        }
        let e = crate::Output::connect("KnobDeck test", "No Such Port 123").err().unwrap();
        assert!(e.contains("isn't there"), "{e}");
    }
}
