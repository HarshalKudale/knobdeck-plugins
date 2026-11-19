//! Windows (WinMM) and macOS (CoreMIDI) through midir.

use crate::{Callback, Result};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::sync::Mutex;

fn out_client(client: &str) -> Result<MidiOutput> {
    MidiOutput::new(client).map_err(|e| format!("MIDI isn't available: {e}"))
}

fn in_client(client: &str) -> Result<MidiInput> {
    let mut m = MidiInput::new(client).map_err(|e| format!("MIDI isn't available: {e}"))?;
    m.ignore(Ignore::TimeAndActiveSense); // keep SysEx (Mackie Control's displays use it)
    Ok(m)
}

pub fn outputs() -> Result<Vec<String>> {
    let m = out_client("KnobDeck")?;
    Ok(m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
}

pub fn inputs() -> Result<Vec<String>> {
    let m = in_client("KnobDeck")?;
    Ok(m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
}

pub struct Output(Mutex<MidiOutputConnection>);

impl Output {
    #[cfg(unix)]
    pub fn open_virtual(client: &str, name: &str) -> Result<Self> {
        use midir::os::unix::VirtualOutput;
        let c = out_client(client)?.create_virtual(name).map_err(|e| format!("Couldn't create the MIDI port \"{name}\": {e}"))?;
        Ok(Output(Mutex::new(c)))
    }

    #[cfg(not(unix))]
    pub fn open_virtual(_client: &str, _name: &str) -> Result<Self> {
        Err(crate::NO_VIRTUAL_ON_WINDOWS.into())
    }

    pub fn connect(client: &str, port: &str) -> Result<Self> {
        let m = out_client(client)?;
        let p = m.ports().into_iter().find(|p| m.port_name(p).ok().as_deref() == Some(port)).ok_or_else(|| format!("MIDI output \"{port}\" isn't there"))?;
        let c = m.connect(&p, "KnobDeck").map_err(|e| format!("Couldn't open MIDI output \"{port}\": {e}"))?;
        Ok(Output(Mutex::new(c)))
    }

    pub fn send(&self, message: &[u8]) -> Result<()> {
        self.0.lock().unwrap().send(message).map_err(|e| format!("Couldn't send MIDI: {e}"))
    }
}

pub struct Input(#[allow(dead_code)] MidiInputConnection<()>);

impl Input {
    #[cfg(unix)]
    pub fn open_virtual(client: &str, name: &str, mut cb: Callback) -> Result<Self> {
        use midir::os::unix::VirtualInput;
        let c = in_client(client)?.create_virtual(name, move |_, m, _| cb(m), ()).map_err(|e| format!("Couldn't create the MIDI port \"{name}\": {e}"))?;
        Ok(Input(c))
    }

    #[cfg(not(unix))]
    pub fn open_virtual(_client: &str, _name: &str, _cb: Callback) -> Result<Self> {
        Err(crate::NO_VIRTUAL_ON_WINDOWS.into())
    }

    pub fn connect(client: &str, port: &str, mut cb: Callback) -> Result<Self> {
        let m = in_client(client)?;
        let p = m.ports().into_iter().find(|p| m.port_name(p).ok().as_deref() == Some(port)).ok_or_else(|| format!("MIDI input \"{port}\" isn't there"))?;
        let c = m.connect(&p, "KnobDeck", move |_, msg, _| cb(msg), ()).map_err(|e| format!("Couldn't open MIDI input \"{port}\": {e}"))?;
        Ok(Input(c))
    }
}
