//! The sound engine: open outputs (one mixer per device), the sounds playing per placed key, the
//! master volume and a cache of sound files read into memory.

use rodio::decoder::LoopedDecoder;
use rodio::{Decoder, Player};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant, SystemTime};

#[cfg(target_os = "linux")]
use crate::out_pulse as out;
#[cfg(not(target_os = "linux"))]
use crate::out_cpal as out;

/// The output-device value meaning "the system's default output".
pub const DEFAULT: &str = "default";
/// An output with nothing playing closes after this long (so the device can sleep, and a new
/// default device is picked up).
const IDLE_CLOSE: Duration = Duration::from_secs(30);
/// Files larger than this aren't kept in memory between plays.
const CACHE_FILE_MAX: u64 = 64 << 20;
const CACHE_TOTAL_MAX: usize = 256 << 20;

type Bytes = Cursor<Arc<[u8]>>;

struct Voice {
    players: Vec<Player>,
    /// The sound's own volume, 0..=1 (the master volume multiplies it).
    gain: f32,
    looping: bool,
}

struct OpenOutput {
    out: out::Output,
    last_used: Instant,
}

#[derive(Default)]
struct Engine {
    outputs: HashMap<String, OpenOutput>,
    /// Per placed key (instance id), with the device each player plays on.
    voices: HashMap<String, Vec<(String, Voice)>>,
    cache: HashMap<PathBuf, (SystemTime, u64, Arc<[u8]>)>,
    master: Option<f32>,
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);
static REAPER: Once = Once::new();

fn reap() {
    let mut g = ENGINE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(e) = g.as_mut() {
        e.reap();
    }
}

fn with<T>(f: impl FnOnce(&mut Engine) -> T) -> T {
    REAPER.call_once(|| {
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_secs(5));
            reap();
        });
    });
    let mut g = ENGINE.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(Engine::default))
}

/// What to play: one press of a Play Sound key.
pub struct Sound {
    pub path: PathBuf,
    /// One or two output devices.
    pub outputs: Vec<String>,
    /// 0..=1.
    pub gain: f32,
    pub looping: bool,
}

/// Decode `data` (the file's extension as a hint): a one-shot source, or a looping one.
enum Source {
    Once(Decoder<Bytes>),
    Looped(LoopedDecoder<Bytes>),
}

fn hint(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn decode(data: Arc<[u8]>, ext: &str, looping: bool) -> Result<Source, String> {
    let len = data.len() as u64;
    let b = Decoder::builder().with_data(Cursor::new(data)).with_byte_len(len).with_seekable(true).with_gapless(true);
    let b = if ext.is_empty() { b } else { b.with_hint(ext) };
    let r = if looping { b.build_looped().map(Source::Looped) } else { b.build().map(Source::Once) };
    r.map_err(|e| format!("Can't play this file: {}. Use mp3, wav, ogg or flac.", e.to_string().trim_end_matches('.')))
}

impl Engine {
    fn master(&mut self) -> f32 {
        *self.master.get_or_insert_with(load_master)
    }

    fn read(&mut self, path: &Path) -> Result<Arc<[u8]>, String> {
        let meta = std::fs::metadata(path).map_err(|_| format!("No sound file at {}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{} isn't a file", path.display()));
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if let Some((t, l, d)) = self.cache.get(path) {
            if *t == mtime && *l == meta.len() {
                return Ok(d.clone());
            }
        }
        let data: Arc<[u8]> = std::fs::read(path).map_err(|e| format!("Can't read {}: {e}", path.display()))?.into();
        if meta.len() <= CACHE_FILE_MAX {
            if self.cache.values().map(|c| c.2.len()).sum::<usize>() + data.len() > CACHE_TOTAL_MAX {
                self.cache.clear();
            }
            self.cache.insert(path.to_path_buf(), (mtime, meta.len(), data.clone()));
        }
        Ok(data)
    }

    fn output(&mut self, id: &str) -> Result<&mut OpenOutput, String> {
        if !self.outputs.contains_key(id) {
            let out = out::open(id)?;
            self.outputs.insert(id.to_string(), OpenOutput { out, last_used: Instant::now() });
        }
        let o = self.outputs.get_mut(id).unwrap();
        o.last_used = Instant::now();
        Ok(o)
    }

    fn play(&mut self, key: &str, s: &Sound) -> Result<(), String> {
        let data = self.read(&s.path)?;
        let ext = hint(&s.path);
        // Decode once up front so a bad file is an error before anything starts.
        let mut sources = vec![];
        for _ in &s.outputs {
            sources.push(decode(data.clone(), &ext, s.looping)?);
        }
        let master = self.master();
        let mut started = vec![];
        for (dev, src) in s.outputs.iter().zip(sources) {
            let o = self.output(dev)?;
            let p = Player::connect_new(o.out.mixer());
            p.set_volume(s.gain * master);
            match src {
                Source::Once(d) => p.append(d),
                Source::Looped(d) => p.append(d),
            }
            started.push((dev.clone(), Voice { players: vec![p], gain: s.gain, looping: s.looping }));
        }
        self.voices.entry(key.to_string()).or_default().extend(started);
        Ok(())
    }

    fn playing(&self, key: &str) -> bool {
        self.voices.get(key).is_some_and(|v| v.iter().any(|(_, v)| v.players.iter().any(|p| !p.empty())))
    }

    fn looping(&self, key: &str) -> bool {
        self.voices.get(key).is_some_and(|v| v.iter().any(|(_, v)| v.looping && v.players.iter().any(|p| !p.empty())))
    }

    /// Dropping a player stops its sound.
    fn stop(&mut self, key: &str) {
        self.voices.remove(key);
    }

    fn reap(&mut self) {
        for list in self.voices.values_mut() {
            for (_, v) in list.iter_mut() {
                v.players.retain(|p| !p.empty());
            }
            list.retain(|(_, v)| !v.players.is_empty());
        }
        self.voices.retain(|_, l| !l.is_empty());
        let busy: Vec<String> = self.voices.values().flatten().map(|(d, _)| d.clone()).collect();
        for (dev, o) in self.outputs.iter_mut() {
            if busy.contains(dev) {
                o.last_used = Instant::now();
            }
        }
        self.outputs.retain(|_, o| o.last_used.elapsed() < IDLE_CLOSE);
    }

    fn set_master(&mut self, m: f32) {
        self.master = Some(m);
        for (_, v) in self.voices.values().flatten() {
            for p in &v.players {
                p.set_volume(v.gain * m);
            }
        }
        save_master(m);
    }
}

fn master_file() -> PathBuf {
    knobdeck_sdk::data_file("master-volume")
}

fn load_master() -> f32 {
    std::fs::read_to_string(master_file()).ok().and_then(|s| s.trim().parse::<f32>().ok()).map(|v| v.clamp(0.0, 1.0)).unwrap_or(1.0)
}

fn save_master(m: f32) {
    let _ = std::fs::write(master_file(), format!("{m}"));
}

/// How a Play Sound press behaves when the sound is already playing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    /// Start another copy on top.
    Overlap,
    /// Stop it and start from the beginning.
    Restart,
    /// Stop it (press again to play).
    StopIfPlaying,
    /// Loop until pressed again.
    ToggleLoop,
}

impl Mode {
    pub fn parse(s: &str) -> Mode {
        match s {
            "overlap" => Mode::Overlap,
            "stop" => Mode::StopIfPlaying,
            "loop" => Mode::ToggleLoop,
            _ => Mode::Restart,
        }
    }
}

/// A Play Sound press. Returns whether the key's sound is playing afterwards.
/// Open the outputs a sound needs without holding the engine's lock (opening can take a moment,
/// and Stop All must never wait for it).
fn open_outputs(ids: &[String]) -> Result<(), String> {
    for id in ids {
        if with(|e| e.outputs.contains_key(id)) {
            continue;
        }
        let out = out::open(id)?;
        with(|e| {
            e.outputs.entry(id.clone()).or_insert(OpenOutput { out, last_used: Instant::now() });
        });
    }
    Ok(())
}

pub fn press(key: &str, mode: Mode, mut s: Sound) -> Result<bool, String> {
    open_outputs(&s.outputs)?;
    with(|e| {
        s.looping = mode == Mode::ToggleLoop;
        match mode {
            Mode::Overlap => {}
            Mode::Restart => e.stop(key),
            Mode::StopIfPlaying | Mode::ToggleLoop => {
                let on = if mode == Mode::ToggleLoop { e.looping(key) } else { e.playing(key) };
                e.stop(key);
                if on {
                    return Ok(false);
                }
            }
        }
        e.play(key, &s).map(|_| true)
    })
}

/// Hold to Play: start on hold.
pub fn start(key: &str, s: Sound) -> Result<(), String> {
    open_outputs(&s.outputs)?;
    with(|e| {
        e.stop(key);
        e.play(key, &s)
    })
}

pub fn stop(key: &str) {
    with(|e| e.stop(key));
}

pub fn stop_all() {
    with(|e| e.voices.clear());
}

pub fn master() -> f32 {
    with(|e| e.master())
}

pub fn set_master(m: f32) {
    with(|e| e.set_master(m.clamp(0.0, 1.0)));
}

/// `(value, label)` options for an output picker: the default first.
pub fn output_options() -> Result<Vec<(String, String)>, String> {
    let mut opts = vec![(DEFAULT.to_string(), match out::default_name() {
        Some(n) => format!("Default output ({n})"),
        None => "Default output".to_string(),
    })];
    opts.extend(out::devices()?);
    Ok(opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source as _;

    /// A 16-bit PCM WAV of `frames` frames of a quiet square wave.
    pub fn wav(rate: u32, channels: u16, frames: u32) -> Vec<u8> {
        let data_len = frames * channels as u32 * 2;
        let mut v = vec![];
        v.extend(b"RIFF");
        v.extend((36 + data_len).to_le_bytes());
        v.extend(b"WAVEfmt ");
        v.extend(16u32.to_le_bytes());
        v.extend(1u16.to_le_bytes());
        v.extend(channels.to_le_bytes());
        v.extend(rate.to_le_bytes());
        v.extend((rate * channels as u32 * 2).to_le_bytes());
        v.extend((channels * 2).to_le_bytes());
        v.extend(16u16.to_le_bytes());
        v.extend(b"data");
        v.extend(data_len.to_le_bytes());
        for i in 0..frames * channels as u32 {
            let s: i16 = if (i / 50) % 2 == 0 { 1000 } else { -1000 };
            v.extend(s.to_le_bytes());
        }
        v
    }

    #[test]
    fn decodes_wav_once_and_looped() {
        let data: Arc<[u8]> = wav(44_100, 2, 4410).into();
        let Source::Once(d) = decode(data.clone(), "wav", false).unwrap() else { panic!() };
        assert_eq!((d.channels().get(), d.sample_rate().get()), (2, 44_100));
        assert_eq!(d.count(), 8820, "every sample comes out once");
        let Source::Looped(l) = decode(data, "wav", true).unwrap() else { panic!() };
        assert_eq!(l.take(20_000).count(), 20_000, "a looped sound keeps going");
    }

    #[test]
    fn rejects_non_audio() {
        let junk: Arc<[u8]> = b"this is not audio at all, just some text".to_vec().into();
        assert!(decode(junk, "mp3", false).err().unwrap().contains("mp3, wav, ogg or flac"));
    }

    #[test]
    fn modes() {
        assert_eq!(Mode::parse("overlap"), Mode::Overlap);
        assert_eq!(Mode::parse("stop"), Mode::StopIfPlaying);
        assert_eq!(Mode::parse("loop"), Mode::ToggleLoop);
        assert_eq!(Mode::parse(""), Mode::Restart);
    }

    /// Mixing two sounds into one stereo 48 kHz output, as each output device does.
    #[test]
    fn mixes_and_resamples() {
        let (mixer, out) = rodio::mixer::mixer(std::num::NonZero::new(2).unwrap(), std::num::NonZero::new(48_000).unwrap());
        let p = Player::connect_new(&mixer);
        p.set_volume(0.5);
        let Source::Once(d) = decode(wav(22_050, 1, 2205).into(), "wav", false).unwrap() else { panic!() };
        p.append(d);
        let n = out.take(48_000).filter(|s| s.abs() > 0.0).count();
        // 0.1 s of mono 22.05 kHz becomes ~0.1 s of stereo 48 kHz.
        assert!((8_000..=10_500).contains(&n), "{n}");
    }

    /// tests/fixtures: 0.3 s tones in mp3, ogg (vorbis), flac and wav, made with ffmpeg.
    #[test]
    fn decodes_fixture_files() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
        assert_eq!(files.len(), 4);
        for f in files {
            let p = f.path();
            let data: Arc<[u8]> = std::fs::read(&p).unwrap().into();
            let Source::Once(d) = decode(data, &hint(&p), false).unwrap_or_else(|e| panic!("{}: {e}", p.display())) else { panic!() };
            assert!(d.take(1000).count() == 1000, "{}", p.display());
        }
    }

    /// Lists the output devices (read-only). `cargo test -p soundboard -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lists_outputs() {
        for (v, l) in output_options().unwrap() {
            println!("{v}  =  {l}");
        }
    }
}
