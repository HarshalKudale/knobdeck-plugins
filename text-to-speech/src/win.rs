//! Windows: WinRT `Windows.Media.SpeechSynthesis.SpeechSynthesizer` (every installed voice,
//! the natural ones included) synthesizes to a stream that a `MediaPlayer` plays.

use crate::{Settings, Voice};
use std::sync::Mutex;
use windows::core::HSTRING;
use windows::Media::Core::MediaSource;
use windows::Media::Playback::{MediaPlaybackState, MediaPlayer};
use windows::Media::SpeechSynthesis::SpeechSynthesizer;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

static PLAYER: Mutex<Option<MediaPlayer>> = Mutex::new(None);

fn init() {
    // SAFETY: joins the MTA; a thread already in another apartment keeps it (the error is ignored).
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn werr(e: windows::core::Error) -> String {
    format!("Windows speech: {}", e.message())
}

pub fn speak(text: &str, s: &Settings) -> Result<(), String> {
    init();
    stop()?;
    let synth = SpeechSynthesizer::new().map_err(werr)?;
    if !s.voice.is_empty() {
        let voices = SpeechSynthesizer::AllVoices().map_err(werr)?;
        let id = HSTRING::from(s.voice.as_str());
        let voice = voices.into_iter().find(|v| v.Id().map(|i| i == id).unwrap_or(false)).ok_or_else(|| "That voice isn't installed any more. Pick another one".to_string())?;
        synth.SetVoice(&voice).map_err(werr)?;
    }
    let options = synth.Options().map_err(werr)?;
    options.SetSpeakingRate(s.rate.clamp(0.5, 3.0)).map_err(werr)?;
    let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text)).map_err(werr)?.get().map_err(werr)?;
    let content_type = stream.ContentType().map_err(werr)?;
    let source = MediaSource::CreateFromStream(&stream, &content_type).map_err(werr)?;
    let player = MediaPlayer::new().map_err(werr)?;
    player.SetVolume((s.volume / 100.0).clamp(0.0, 1.0)).map_err(werr)?;
    player.SetSource(&source).map_err(werr)?;
    player.Play().map_err(werr)?;
    *PLAYER.lock().unwrap() = Some(player);
    Ok(())
}

pub fn stop() -> Result<(), String> {
    init();
    if let Some(p) = PLAYER.lock().unwrap().take() {
        let _ = p.Pause();
        let _ = p.Close();
    }
    Ok(())
}

pub fn speaking() -> bool {
    init();
    let g = PLAYER.lock().unwrap();
    let Some(p) = g.as_ref() else { return false };
    matches!(
        p.PlaybackSession().and_then(|s| s.PlaybackState()),
        Ok(st) if st == MediaPlaybackState::Playing || st == MediaPlaybackState::Opening || st == MediaPlaybackState::Buffering
    )
}

pub fn voices() -> Result<Vec<Voice>, String> {
    init();
    let mut out = vec![];
    for v in SpeechSynthesizer::AllVoices().map_err(werr)? {
        let id = v.Id().map_err(werr)?.to_string();
        let name = v.DisplayName().map_err(werr)?.to_string();
        let lang = v.Language().map(|l| l.to_string()).unwrap_or_default();
        out.push(Voice { id, label: if lang.is_empty() { name } else { format!("{name} ({lang})") } });
    }
    Ok(out)
}
