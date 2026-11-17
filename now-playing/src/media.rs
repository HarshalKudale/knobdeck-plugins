//! The OS's media sessions: Windows' system media transport controls (SMTC, every app that shows
//! in the volume flyout), MPRIS on Linux (every player GNOME's and Plasma's media controls show),
//! and on macOS the players with an AppleScript dictionary (Music and Spotify): macOS 15.4 closed
//! the system-wide now-playing API to other apps.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    /// Stable key for the player: SMTC app id, MPRIS bus name without the instance part, or the
    /// macOS app name.
    pub id: String,
    /// Name to show ("Spotify", "Firefox").
    pub app: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Where the cover comes from (MPRIS `mpris:artUrl`, Spotify's artwork URL); Windows reads
    /// the thumbnail from the session itself.
    pub art_url: Option<String>,
    /// Seconds into the track, and the track's length, when the player reports them.
    pub position: Option<f64>,
    pub length: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cmd {
    PlayPause,
    Next,
    Previous,
    /// Seconds, forward or back.
    Seek(f64),
}

pub use imp::*;

#[cfg(windows)]
mod imp {
    use super::{Cmd, Session};
    use knobdeck_sdk::{Error, Result};
    use windows::Media::Control::{GlobalSystemMediaTransportControlsSession as WinSession, GlobalSystemMediaTransportControlsSessionManager as Manager, GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status};
    use windows::Storage::Streams::DataReader;

    fn werr(e: windows::core::Error) -> Error {
        Error::msg(format!("Media controls unavailable: {e}"))
    }

    fn manager() -> Result<Manager> {
        Manager::RequestAsync().and_then(|op| op.get()).map_err(werr)
    }

    /// "Spotify.exe" → "Spotify"; "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic" → "ZuneMusic".
    pub fn app_name(id: &str) -> String {
        let s = id.split('!').next_back().unwrap_or(id);
        let s = s.trim_end_matches(".exe").trim_end_matches(".EXE");
        let s = s.rsplit('.').next().unwrap_or(s);
        s.split('_').next().unwrap_or(s).to_string()
    }

    fn read(s: &WinSession) -> Session {
        let id = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
        let props = s.TryGetMediaPropertiesAsync().and_then(|op| op.get()).ok();
        let timeline = s.GetTimelineProperties().ok().and_then(|t| Some((t.Position().ok()?.Duration as f64 / 1e7, t.EndTime().ok()?.Duration as f64 / 1e7)));
        let text = |f: &dyn Fn(&windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties) -> windows::core::Result<windows::core::HSTRING>| props.as_ref().and_then(|p| f(p).ok()).map(|h| h.to_string()).unwrap_or_default();
        Session {
            app: app_name(&id),
            title: text(&|p| p.Title()),
            artist: text(&|p| p.Artist()),
            playing: s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).is_ok_and(|st| st == Status::Playing),
            art_url: None,
            position: timeline.map(|t| t.0),
            length: timeline.map(|t| t.1).filter(|l| *l > 0.0),
            id,
        }
    }

    fn all() -> Result<(Manager, Vec<WinSession>)> {
        let m = manager()?;
        let list = m.GetSessions().map_err(werr)?.into_iter().collect();
        Ok((m, list))
    }

    /// Every session; the system's current one first.
    pub fn sessions() -> Result<Vec<Session>> {
        let (m, list) = all()?;
        let current = m.GetCurrentSession().ok().and_then(|s| s.SourceAppUserModelId().ok()).map(|h| h.to_string());
        let mut out: Vec<Session> = list.iter().map(read).collect();
        out.sort_by_key(|s| Some(&s.id) != current.as_ref());
        Ok(out)
    }

    fn find(id: &str) -> Result<WinSession> {
        let (_, list) = all()?;
        list.into_iter().find(|s| s.SourceAppUserModelId().is_ok_and(|h| h.to_string() == id)).ok_or_else(|| Error::msg("That player is closed"))
    }

    fn done(what: &str, r: windows::core::Result<windows::Foundation::IAsyncOperation<bool>>) -> Result<()> {
        match r.and_then(|op| op.get()) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::msg(format!("The player ignored {what}"))),
            Err(e) => Err(Error::msg(format!("{what} failed: {e}"))),
        }
    }

    pub fn control(id: &str, cmd: Cmd) -> Result<()> {
        let s = find(id)?;
        match cmd {
            Cmd::PlayPause => done("play/pause", s.TryTogglePlayPauseAsync()),
            Cmd::Next => done("next", s.TrySkipNextAsync()),
            Cmd::Previous => done("previous", s.TrySkipPreviousAsync()),
            Cmd::Seek(secs) => {
                let t = s.GetTimelineProperties().map_err(werr)?;
                let (pos, end) = (t.Position().map_err(werr)?.Duration, t.EndTime().map_err(werr)?.Duration);
                let target = (pos + (secs * 10_000_000.0) as i64).clamp(0, end.max(0));
                done("seek", s.TryChangePlaybackPositionAsync(target))
            }
        }
    }

    /// The session's cover image bytes.
    pub fn art(session: &Session) -> Option<Vec<u8>> {
        let s = find(&session.id).ok()?;
        let thumb = s.TryGetMediaPropertiesAsync().and_then(|op| op.get()).ok()?.Thumbnail().ok()?;
        let stream = thumb.OpenReadAsync().and_then(|op| op.get()).ok()?;
        let size = stream.Size().ok()? as u32;
        if size == 0 || size > 8_000_000 {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream).ok()?;
        reader.LoadAsync(size).and_then(|op| op.get()).ok()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf).ok()?;
        Some(buf)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{Cmd, Session};
    use knobdeck_sdk::{Error, Result};

    /// Players with a scripting dictionary that report what they play.
    const PLAYERS: &[&str] = &["Music", "Spotify"];

    fn osascript(script: &str) -> Result<String> {
        let out = std::process::Command::new("osascript").args(["-e", script]).output().map_err(|e| Error::msg(format!("osascript failed: {e}")))?;
        if !out.status.success() {
            return Err(Error::msg(String::from_utf8_lossy(&out.stderr).trim().to_string()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// Talk to `app` only when it already runs (a `tell` would launch it).
    fn tell(app: &str, body: &str) -> Result<Option<String>> {
        let t = osascript(&format!("if application \"{app}\" is running then\ntell application \"{app}\"\n{body}\nend tell\nelse\nreturn \"__not_running__\"\nend if"))?;
        Ok((t != "__not_running__").then_some(t))
    }

    fn read(app: &str) -> Option<Session> {
        // Spotify reports the length in milliseconds, Music in seconds.
        let (art, length) = if app == "Spotify" { ("artwork url of current track", "((duration of current track) / 1000)") } else { ("\"\"", "(duration of current track)") };
        let t = tell(app, &format!("if player state is stopped then return \"__stopped__\"\nreturn (name of current track) & linefeed & (artist of current track) & linefeed & (player state as string) & linefeed & {art} & linefeed & (player position as string) & linefeed & ({length} as string)")).ok()??;
        if t == "__stopped__" {
            return Some(Session { id: app.into(), app: app.into(), ..Default::default() });
        }
        let mut l = t.split('\n');
        Some(Session {
            id: app.into(),
            app: app.into(),
            title: l.next().unwrap_or("").into(),
            artist: l.next().unwrap_or("").into(),
            playing: l.next() == Some("playing"),
            art_url: l.next().filter(|u| u.starts_with("http")).map(String::from),
            // AppleScript may write numbers with the user's decimal comma.
            position: l.next().and_then(|n| n.replace(',', ".").parse().ok()),
            length: l.next().and_then(|n| n.replace(',', ".").parse().ok()).filter(|l: &f64| *l > 0.0),
        })
    }

    pub fn sessions() -> Result<Vec<Session>> {
        let mut v: Vec<Session> = PLAYERS.iter().filter_map(|a| read(a)).collect();
        v.sort_by_key(|s| !s.playing);
        Ok(v)
    }

    pub fn control(id: &str, cmd: Cmd) -> Result<()> {
        if !PLAYERS.contains(&id) {
            return Err(Error::msg("That player is closed"));
        }
        let body = match cmd {
            Cmd::PlayPause => "playpause".to_string(),
            Cmd::Next => "next track".to_string(),
            Cmd::Previous => "previous track".to_string(),
            Cmd::Seek(s) => format!("set player position to ((player position) + ({s}))"),
        };
        tell(id, &body)?.map(|_| ()).ok_or_else(|| Error::msg(format!("{id} is not open")))
    }

    pub fn art(_: &Session) -> Option<Vec<u8>> {
        None // art_url covers Spotify; Music's artwork data isn't worth decoding from AppleScript
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::{Cmd, Session};
    use knobdeck_sdk::{Error, Result};
    use linux_desktop::bus;
    use std::collections::HashMap;
    use zbus::zvariant::OwnedValue;

    const PREFIX: &str = "org.mpris.MediaPlayer2.";
    const PATH: &str = "/org/mpris/MediaPlayer2";
    const ROOT: &str = "org.mpris.MediaPlayer2";
    const PLAYER: &str = "org.mpris.MediaPlayer2.Player";

    /// "org.mpris.MediaPlayer2.firefox.instance_1_216" → "firefox": the key survives a restart.
    pub fn key_of(name: &str) -> String {
        let rest = name.strip_prefix(PREFIX).unwrap_or(name);
        rest.split(".instance").next().unwrap_or(rest).to_string()
    }

    fn read(conn: &zbus::blocking::Connection, name: &str) -> Option<Session> {
        let meta: HashMap<String, OwnedValue> = bus::get(conn, name, PATH, PLAYER, "Metadata").ok()?;
        let status: String = bus::get(conn, name, PATH, PLAYER, "PlaybackStatus").unwrap_or_default();
        let identity: String = bus::get(conn, name, PATH, ROOT, "Identity").unwrap_or_else(|_| key_of(name));
        let text = |k: &str| meta.get(k).and_then(|v| v.try_clone().ok()).and_then(|v| String::try_from(v).ok()).unwrap_or_default();
        let artist = meta.get("xesam:artist").and_then(|v| v.try_clone().ok()).and_then(|v| Vec::<String>::try_from(v).ok()).map(|a| a.join(", ")).unwrap_or_default();
        // mpris:length is an x (i64) or, from some players, a t (u64), in microseconds.
        let length = meta.get("mpris:length").and_then(|v| v.try_clone().ok()).and_then(|v| i64::try_from(&v).ok().or_else(|| u64::try_from(&v).ok().map(|u| u as i64))).map(|us| us as f64 / 1e6).filter(|l| *l > 0.0);
        let position = bus::get::<i64>(conn, name, PATH, PLAYER, "Position").ok().map(|us| us as f64 / 1e6);
        Some(Session { id: key_of(name), app: identity, title: text("xesam:title"), artist, playing: status == "Playing", art_url: Some(text("mpris:artUrl")).filter(|u| !u.is_empty()), position, length })
    }

    pub fn sessions() -> Result<Vec<Session>> {
        let conn = bus::session()?;
        let mut names = bus::session_names(PREFIX);
        names.sort();
        let mut v: Vec<Session> = names.iter().filter_map(|n| read(&conn, n)).collect();
        // Players with the same key (two Firefox windows) collapse to the first.
        v.dedup_by(|a, b| a.id == b.id);
        v.sort_by_key(|s| !s.playing);
        Ok(v)
    }

    pub fn control(id: &str, cmd: Cmd) -> Result<()> {
        let conn = bus::session()?;
        let name = bus::session_names(PREFIX).into_iter().find(|n| key_of(n) == id).ok_or_else(|| Error::msg("That player is closed"))?;
        match cmd {
            Cmd::PlayPause => bus::call0(&conn, &name, PATH, PLAYER, "PlayPause", &())?,
            Cmd::Next => bus::call0(&conn, &name, PATH, PLAYER, "Next", &())?,
            Cmd::Previous => bus::call0(&conn, &name, PATH, PLAYER, "Previous", &())?,
            Cmd::Seek(s) => bus::call0(&conn, &name, PATH, PLAYER, "Seek", &((s * 1_000_000.0) as i64,))?,
        }
        Ok(())
    }

    pub fn art(_: &Session) -> Option<Vec<u8>> {
        None // MPRIS gives art_url
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn mpris_keys() {
        assert_eq!(super::key_of("org.mpris.MediaPlayer2.firefox.instance_1_216"), "firefox");
        assert_eq!(super::key_of("org.mpris.MediaPlayer2.spotify"), "spotify");
    }

    #[cfg(windows)]
    #[test]
    fn app_names() {
        assert_eq!(super::app_name("Spotify.exe"), "Spotify");
        assert_eq!(super::app_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "ZuneMusic");
    }

    /// Reads the real sessions (read-only; nothing is played or paused).
    #[test]
    #[ignore = "needs a desktop session"]
    fn live_sessions() {
        eprintln!("{:#?}", super::sessions().unwrap());
    }
}
