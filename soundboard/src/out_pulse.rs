//! Output devices on Linux: a playback stream on a PulseAudio sink, over the PulseAudio protocol
//! (pure Rust; PipeWire's `pipewire-pulse` serves the same protocol on GNOME and KDE). Every sink is
//! listed, null sinks and virtual cables included.

use futures::executor::block_on;
use pulseaudio::{protocol, AsPlaybackSource, Client, PlaybackStream};
use rodio::mixer::Mixer;
use std::ffi::CString;
use std::num::NonZero;
use std::time::Duration;

const RATE: u32 = 48_000;
const CHANNELS: u16 = 2;
/// About 40 ms of audio in the server's buffer: quick to start, safe from underruns.
const TARGET_BYTES: u32 = RATE / 25 * CHANNELS as u32 * 4;

pub struct Output {
    mixer: Mixer,
    stream: Option<PlaybackStream>,
    _client: Client,
}

impl Output {
    pub fn mixer(&self) -> &Mixer {
        &self.mixer
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Some(s) = self.stream.take() {
            // Off the caller's thread: a hung server must not hold the engine's lock.
            std::thread::spawn(move || {
                let _ = block_on(s.delete());
            });
        }
    }
}

fn client() -> Result<Client, String> {
    Client::from_env(c"KnobDeck Soundboard").map_err(|e| format!("No PulseAudio / PipeWire sound server: {e}"))
}

/// Run `f` on a thread with a time limit, so a hung server is an error, never a stuck key.
fn timed<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or_else(|_| Err("The sound server didn't answer".into()))
}

fn text(c: &CString) -> String {
    String::from_utf8_lossy(c.as_bytes()).into_owned()
}

/// `(sink name, description)` of every sink.
pub fn devices() -> Result<Vec<(String, String)>, String> {
    timed(|| {
        let c = client()?;
        let sinks = block_on(c.list_sinks()).map_err(|e| format!("Can't list the sound outputs: {e}"))?;
        Ok(sinks.iter().map(|s| (text(&s.name), s.description.as_ref().map(text).unwrap_or_else(|| text(&s.name)))).collect())
    })
}

pub fn default_name() -> Option<String> {
    timed(|| {
        let c = client()?;
        let info = block_on(c.server_info()).map_err(|e| e.to_string())?;
        let name = info.default_sink_name.ok_or("no default")?;
        let sink = block_on(c.sink_info_by_name(name)).map_err(|e| e.to_string())?;
        Ok(sink.description.as_ref().map(text).unwrap_or_else(|| text(&sink.name)))
    })
    .ok()
}

pub fn open(id: &str) -> Result<Output, String> {
    let sink = if id == crate::engine::DEFAULT { protocol::DEFAULT_SINK.to_owned() } else { CString::new(id).map_err(|_| "Bad output name")? };
    let (mixer, mut source) = rodio::mixer::mixer(NonZero::new(CHANNELS).unwrap(), NonZero::new(RATE).unwrap());
    let label = id.to_string();
    timed(move || {
        let c = client()?;
        let mut props = protocol::Props::new();
        props.set(protocol::Prop::MediaName, c"Soundboard".to_owned());
        props.set(protocol::Prop::ApplicationName, c"KnobDeck Soundboard".to_owned());
        let params = protocol::PlaybackStreamParams {
            sample_spec: protocol::SampleSpec { format: protocol::SampleFormat::Float32Le, channels: CHANNELS as u8, sample_rate: RATE },
            channel_map: protocol::ChannelMap::stereo(),
            cvolume: Some(protocol::ChannelVolume::norm(CHANNELS as u8)),
            sink_name: Some(sink),
            buffer_attr: protocol::stream::BufferAttr { target_length: TARGET_BYTES, ..Default::default() },
            flags: protocol::stream::StreamFlags { adjust_latency: true, ..Default::default() },
            props,
            ..Default::default()
        };
        // Silence while nothing plays: returning 0 would end the stream.
        let fill = move |buf: &mut [u8]| {
            for s in buf.chunks_exact_mut(4) {
                s.copy_from_slice(&source.next().unwrap_or(0.0).to_le_bytes());
            }
            buf.len()
        };
        let stream = block_on(c.create_playback_stream(params, fill.as_playback_source())).map_err(|e| format!("Can't play to {label}: {e}"))?;
        Ok(Output { mixer, stream: Some(stream), _client: c })
    })
}
