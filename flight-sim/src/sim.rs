//! The slice of SimConnect the plugin uses, as a trait: the Windows build implements it over
//! `SimConnect.dll` (`simconnect.rs`), tests over a fake, and other OSes with a stub that
//! reports the sim isn't running.

/// The error every action shows while no sim answers.
pub const NOT_RUNNING: &str = "Flight Simulator isn't running";

/// A failed SimConnect call.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkError {
    /// The connection is gone (the sim quit or crashed); reconnect.
    Lost,
    /// This call failed; the connection is fine.
    Failed(String),
}

/// How often the sim sends a SimVar: every simulation frame it changed in, or once a second.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rate {
    Frame,
    Second,
}

/// A message from the sim (`SimConnect_GetNextDispatch`), reduced to what the plugin reads.
#[derive(Debug, Clone, PartialEq)]
pub enum Recv {
    /// Connected; the sim's application name ("KittyHawk" is MSFS 2020, "SunRise" MSFS 2024).
    Open { app: String },
    /// The sim is closing.
    Quit,
    /// A SimVar's value, for the data request with this id.
    Data { request: u32, value: f64 },
    /// A subscribed system event.
    Event { id: u32, data: u32 },
    /// An earlier call failed: SimConnect's exception code, and the send id of that call.
    Exception { code: u32, send_id: u32 },
    Other,
}

/// One open SimConnect connection.
pub trait Link: Send {
    /// Map client event `id` to the sim's event `name`; returns the call's send id.
    fn map_event(&mut self, id: u32, name: &str) -> Result<u32, LinkError>;
    /// Send client event `id` to the user's aircraft with up to five data values.
    fn transmit(&mut self, id: u32, data: [u32; 5]) -> Result<(), LinkError>;
    /// Define data definition `def` as one SimVar in `unit` (FLOAT64); returns the send id.
    fn add_var(&mut self, def: u32, name: &str, unit: &str) -> Result<u32, LinkError>;
    /// Ask for definition `def` on the user's aircraft (request id = `def`), or stop with `None`.
    fn request(&mut self, def: u32, rate: Option<Rate>) -> Result<(), LinkError>;
    fn clear_var(&mut self, def: u32) -> Result<(), LinkError>;
    /// Subscribe client event `id` to a system event ("Pause").
    fn subscribe(&mut self, id: u32, name: &str) -> Result<(), LinkError>;
    /// The next waiting message, `None` when there is none.
    fn next(&mut self) -> Result<Option<Recv>, LinkError>;
}

/// Why no connection could be opened.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenError {
    NotRunning,
    /// Something the user can fix (SimConnect.dll not found), as a sentence.
    Unavailable(String),
}

pub trait Connector: Send {
    fn open(&mut self) -> Result<Box<dyn Link>, OpenError>;
}

/// SimConnect exception codes the plugin explains (SIMCONNECT_EXCEPTION).
pub fn exception_name(code: u32) -> &'static str {
    match code {
        1 => "error",
        2 => "size mismatch",
        3 => "unrecognized id",
        7 => "name not recognized",
        9 => "event id used twice",
        12 => "too many requests",
        18 => "invalid data type",
        20 => "data error",
        25 => "illegal operation",
        _ => "SimConnect exception",
    }
}

const RECV_EXCEPTION: u32 = 1;
const RECV_OPEN: u32 = 2;
const RECV_QUIT: u32 = 3;
const RECV_EVENT: u32 = 4;
const RECV_SIMOBJECT_DATA: u32 = 8;

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

/// One SIMCONNECT_RECV packet: header { dwSize, dwVersion, dwID } then the message's fields.
pub fn parse_recv(b: &[u8]) -> Recv {
    match u32_at(b, 8) {
        Some(RECV_EXCEPTION) => match (u32_at(b, 12), u32_at(b, 16)) {
            (Some(code), Some(send_id)) => Recv::Exception { code, send_id },
            _ => Recv::Other,
        },
        Some(RECV_OPEN) => {
            let name = b.get(12..(12 + 256).min(b.len())).unwrap_or(&[]);
            let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            Recv::Open { app: String::from_utf8_lossy(&name[..end]).into_owned() }
        }
        Some(RECV_QUIT) => Recv::Quit,
        // { uGroupID, uEventID, dwData }
        Some(RECV_EVENT) => match (u32_at(b, 16), u32_at(b, 20)) {
            (Some(id), Some(data)) => Recv::Event { id, data },
            _ => Recv::Other,
        },
        // { dwRequestID, dwObjectID, dwDefineID, dwFlags, dwentrynumber, dwoutof, dwDefineCount, dwData… }
        Some(RECV_SIMOBJECT_DATA) => match (u32_at(b, 12), b.get(40..48)) {
            (Some(request), Some(d)) => Recv::Data { request, value: f64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]) },
            _ => Recv::Other,
        },
        _ => Recv::Other,
    }
}

/// The connector of an OS where Flight Simulator doesn't run (the plugin still builds there, so it
/// can be driven over its protocol in tests).
#[cfg(not(windows))]
pub struct Offline;

#[cfg(not(windows))]
impl Connector for Offline {
    fn open(&mut self) -> Result<Box<dyn Link>, OpenError> {
        Err(OpenError::NotRunning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(id: u32, fields: &[u8]) -> Vec<u8> {
        let mut b = vec![];
        b.extend(((12 + fields.len()) as u32).to_le_bytes());
        b.extend(6u32.to_le_bytes());
        b.extend(id.to_le_bytes());
        b.extend(fields);
        b
    }

    fn words(w: &[u32]) -> Vec<u8> {
        w.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    #[test]
    fn parses_dispatch_packets() {
        assert_eq!(parse_recv(&packet(3, &[])), Recv::Quit);
        assert_eq!(parse_recv(&packet(1, &words(&[7, 42, 1]))), Recv::Exception { code: 7, send_id: 42 });
        assert_eq!(parse_recv(&packet(4, &words(&[1, 1, 1]))), Recv::Event { id: 1, data: 1 });
        let mut open = b"KittyHawk".to_vec();
        open.resize(256, 0);
        open.extend(words(&[11, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(parse_recv(&packet(2, &open)), Recv::Open { app: "KittyHawk".into() });
        let mut data = words(&[105, 0, 105, 0, 0, 0, 1]);
        data.extend(118.025f64.to_le_bytes());
        assert_eq!(parse_recv(&packet(8, &data)), Recv::Data { request: 105, value: 118.025 });
        // Short or unknown packets are ignored, never read past their end.
        assert_eq!(parse_recv(&packet(8, &words(&[105, 0]))), Recv::Other);
        assert_eq!(parse_recv(&packet(1, &words(&[7]))), Recv::Other);
        assert_eq!(parse_recv(&packet(99, &[])), Recv::Other);
        assert_eq!(parse_recv(&[1, 2]), Recv::Other);
    }
}
