//! MIDI ports for the KnobDeck plugins that speak MIDI (`midi`, `daw-mackie`). Not a plugin.
//!
//! - Windows: WinMM through `midir`. Windows has no virtual ports for apps (that needs Windows MIDI
//!   Services or a loopback driver such as loopMIDI), so the user picks an existing port there.
//! - macOS: CoreMIDI through `midir`, with virtual sources / destinations other apps see.
//! - Linux: the ALSA sequencer (`libasound.so.2`, loaded at run time; see `alsa.rs`), with virtual
//!   ports other apps (and PipeWire's MIDI bridge) see.
//!
//! Ports are named by text: `Client:Port` on Linux, the endpoint's name elsewhere. [`find`] matches a
//! saved name against the ports there now, so a name keeps working across reboots.

pub mod msg;

#[cfg(target_os = "linux")]
mod alsa;
#[cfg(target_os = "linux")]
use alsa as os;
#[cfg(not(target_os = "linux"))]
mod portable;
#[cfg(not(target_os = "linux"))]
use portable as os;

pub type Result<T> = std::result::Result<T, String>;

/// Whether this OS lets an app create ports other apps connect to (not Windows).
pub const HAS_VIRTUAL: bool = cfg!(not(windows));

/// Said where a virtual port would be needed on Windows.
pub const NO_VIRTUAL_ON_WINDOWS: &str =
    "Windows has no virtual MIDI ports: install loopMIDI (or use Windows MIDI Services' loopback) and pick its port";

/// The ports this computer has that a message can be sent to (synths, apps' inputs, interfaces).
pub fn outputs() -> Result<Vec<String>> {
    os::outputs()
}

/// The ports this computer has that messages can be received from (controllers, apps' outputs).
pub fn inputs() -> Result<Vec<String>> {
    os::inputs()
}

/// The port in `list` that `wanted` names: the exact name, else the same name in another case, else
/// the only port whose name contains it.
pub fn find(list: &[String], wanted: &str) -> Option<String> {
    let w = wanted.trim();
    if w.is_empty() {
        return None;
    }
    if let Some(p) = list.iter().find(|p| p.as_str() == w) {
        return Some(p.clone());
    }
    let lw = w.to_lowercase();
    if let Some(p) = list.iter().find(|p| p.to_lowercase() == lw) {
        return Some(p.clone());
    }
    let mut hits = list.iter().filter(|p| p.to_lowercase().contains(&lw));
    match (hits.next(), hits.next()) {
        (Some(p), None) => Some(p.clone()),
        _ => None,
    }
}

fn not_found(kind: &str, wanted: &str, list: &[String]) -> String {
    if list.is_empty() {
        format!("MIDI {kind} \"{wanted}\" isn't there: this computer has no MIDI {kind}s now")
    } else {
        format!("MIDI {kind} \"{wanted}\" isn't there (now: {})", list.join(", "))
    }
}

/// An open way out: a virtual port others read, or a connection to an existing port.
pub struct Output(os::Output);

impl Output {
    /// A port named `name` that other apps receive from (macOS, Linux). `client` names this app.
    pub fn open_virtual(client: &str, name: &str) -> Result<Self> {
        if !HAS_VIRTUAL {
            return Err(NO_VIRTUAL_ON_WINDOWS.into());
        }
        os::Output::open_virtual(client, name).map(Output)
    }

    /// Send to the existing port `port` names (see [`find`]).
    pub fn connect(client: &str, port: &str) -> Result<Self> {
        let list = outputs()?;
        let name = find(&list, port).ok_or_else(|| not_found("output", port, &list))?;
        os::Output::connect(client, &name).map(Output)
    }

    /// Send one complete message (a status byte and its data, or a whole SysEx).
    pub fn send(&self, message: &[u8]) -> Result<()> {
        if message.is_empty() {
            return Ok(());
        }
        self.0.send(message)
    }
}

/// An open way in: received messages go to the callback, on a thread of its own, until dropped.
pub struct Input(#[allow(dead_code)] os::Input);

impl Input {
    /// A port named `name` that other apps send to (macOS, Linux).
    pub fn open_virtual(client: &str, name: &str, on_message: impl FnMut(&[u8]) + Send + 'static) -> Result<Self> {
        if !HAS_VIRTUAL {
            return Err(NO_VIRTUAL_ON_WINDOWS.into());
        }
        os::Input::open_virtual(client, name, Box::new(on_message)).map(Input)
    }

    /// Receive from the existing port `port` names (see [`find`]).
    pub fn connect(client: &str, port: &str, on_message: impl FnMut(&[u8]) + Send + 'static) -> Result<Self> {
        let list = inputs()?;
        let name = find(&list, port).ok_or_else(|| not_found("input", port, &list))?;
        os::Input::connect(client, &name, Box::new(on_message)).map(Input)
    }
}

pub(crate) type Callback = Box<dyn FnMut(&[u8]) + Send + 'static>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_matches_exact_case_and_unique_part() {
        let list = vec!["Midi Through:Midi Through Port-0".to_string(), "loopMIDI Port".to_string(), "loopMIDI Port 2".to_string()];
        assert_eq!(find(&list, "loopMIDI Port").as_deref(), Some("loopMIDI Port"));
        assert_eq!(find(&list, "loopmidi port 2").as_deref(), Some("loopMIDI Port 2"));
        assert_eq!(find(&list, "through").as_deref(), Some("Midi Through:Midi Through Port-0"));
        assert_eq!(find(&list, "loop"), None, "two ports contain it");
        assert_eq!(find(&list, ""), None);
    }
}
