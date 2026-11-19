//! Now Playing plugin for KnobDeck: what any media player is playing (title, artist, cover) and
//! play/pause, next, previous and seeking, through the OS's own media sessions.

mod media;

use base64::Engine as _;
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use media::{Cmd, Session};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Mutex;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);
const AUTO: &str = "__auto__";
/// Cover size sent to KnobDeck (keys are smaller still).
const ART_PX: u32 = 144;

/// Session for an action: the chosen player, or with "Whatever is playing" the playing one (else
/// the system's current one).
fn session(inputs: &Value) -> Result<Session> {
    let pick = inputs["player"].as_str().unwrap_or(AUTO);
    let list = media::sessions()?;
    if pick.is_empty() || pick == AUTO {
        return list.into_iter().next().ok_or_else(|| Error::msg("Nothing is playing"));
    }
    list.into_iter().find(|s| s.id == pick).ok_or_else(|| Error::msg("That player is closed"))
}

fn control(ctx: &Ctx, cmd: Cmd) -> Result<()> {
    let s = session(&ctx.inputs)?;
    media::control(&s.id, cmd)
}

fn player_input() -> Value {
    input::query_select("player", "Player", "players", &[]).with("default", AUTO)
}

fn track_label(s: &Session) -> String {
    match (s.title.is_empty(), s.artist.is_empty()) {
        (true, _) => s.app.clone(),
        (false, true) => s.title.clone(),
        (false, false) => format!("{}\n{}", s.title, s.artist),
    }
}

/// Cover art as a small JPEG data URI, cached per track (a track's cover is fetched once).
static ART: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

fn art_bytes(s: &Session) -> Option<Vec<u8>> {
    match s.art_url.as_deref() {
        Some(u) if u.starts_with("file://") => {
            let path = u.trim_start_matches("file://");
            std::fs::read(percent_decode(path)).ok()
        }
        Some(u) if u.starts_with("http://") || u.starts_with("https://") => {
            let r = ureq::get(u).timeout(Duration::from_secs(4)).call().ok()?;
            let mut buf = vec![];
            r.into_reader().take(8_000_000).read_to_end(&mut buf).ok()?;
            Some(buf)
        }
        Some(u) if u.starts_with("data:") => base64::engine::general_purpose::STANDARD.decode(u.split_once(',')?.1).ok(),
        _ => media::art(s),
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn shrink(bytes: &[u8]) -> Option<String> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.thumbnail(ART_PX, ART_PX).to_rgb8();
    let mut out = std::io::Cursor::new(vec![]);
    small.write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
    Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(out.into_inner())))
}

fn art(s: &Session) -> Option<String> {
    let key = format!("{}\u{1}{}\u{1}{}\u{1}{}", s.id, s.title, s.artist, s.art_url.as_deref().unwrap_or(""));
    if let Some(hit) = ART.lock().unwrap().as_ref().and_then(|m| m.get(&key)) {
        return hit.clone();
    }
    let uri = art_bytes(s).and_then(|b| shrink(&b));
    let mut g = ART.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if m.len() > 32 {
        m.clear();
    }
    m.insert(key, uri.clone());
    uri
}

/// "3:07", "1:02:03".
fn clock(secs: f64) -> String {
    let t = secs.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

fn play_label(playing: bool) -> &'static str {
    if playing {
        "playing"
    } else {
        "paused"
    }
}

fn main() {
    knobdeck_sdk::load_env();
    Plugin::new(&format!("now-playing-{}", std::env::consts::OS), "Now Playing")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description(if cfg!(target_os = "macos") {
            "What Music or Spotify is playing, with its cover, and play/pause, next, previous and seeking."
        } else {
            "What any media player is playing, with its cover, and play/pause, next, previous and seeking: browsers, Spotify, VLC and every app with media controls."
        })
        .icon("app-icon.svg")
        .query("players", |q| {
            let list = media::sessions().or_else(|e| q.fail(e.message().to_string()))?;
            let mut opts = vec![json!({"value": AUTO, "label": "Whatever is playing"})];
            opts.extend(list.into_iter().map(|s| json!({"value": s.id, "label": s.app})));
            Ok(Value::Array(opts))
        })
        .action(
            Action::button("now-playing", "Now Playing")
                .icon("disc-3")
                .inputs(vec![player_input(), input::checkbox("cover", "Show the cover").default_value(true), input::checkbox("press", "Press to play / pause").default_value(true)])
                .live(POLL, |ic| {
                    let Ok(s) = session(&ic.inputs) else { return Ok(Visual::new().label("Nothing playing").icon("disc-3")) };
                    let icon = if ic.input_bool("cover") { art(&s) } else { None };
                    Ok(Visual::new().label(track_label(&s)).icon(icon.unwrap_or_else(|| if s.playing { "disc-3".into() } else { "pause".into() })))
                })
                .on_press(|ctx| if ctx.input_bool("press") { control(ctx, Cmd::PlayPause) } else { Ok(()) }),
        )
        .action(
            Action::toggle("play-pause", "Play / Pause")
                .icon("play")
                .inputs(vec![player_input()])
                .state("paused", "Play", "play")
                .state("playing", "Pause", "pause")
                .live(POLL, |ic| Ok(Visual::new().state(play_label(session(&ic.inputs).is_ok_and(|s| s.playing)))))
                .on_press(|ctx| {
                    let s = session(&ctx.inputs)?;
                    media::control(&s.id, Cmd::PlayPause)?;
                    ctx.set_state(play_label(!s.playing));
                    Ok(())
                }),
        )
        .action(Action::button("next", "Next Track").icon("skip-forward").inputs(vec![player_input()]).on_press(|ctx| control(ctx, Cmd::Next)))
        .action(Action::button("previous", "Previous Track").icon("skip-back").inputs(vec![player_input()]).on_press(|ctx| control(ctx, Cmd::Previous)))
        .action(
            Action::slider("position", "Position", 0.0, 100.0)
                .icon("fast-forward")
                .unit("%")
                .inputs(vec![player_input(), input::text("step", "Seconds per dial step").placeholder("5").default_value("5")])
                .live(POLL, |ic| {
                    let Ok(s) = session(&ic.inputs) else { return Ok(Visual::new().state(0)) };
                    Ok(match (s.position, s.length) {
                        (Some(p), Some(l)) => Visual::new().state((p / l * 100.0).clamp(0.0, 100.0).round()).label(clock(p)),
                        _ => Visual::new().state(0),
                    })
                })
                .on_adjust(|ctx| {
                    let s = session(&ctx.inputs)?;
                    let pos = s.position.ok_or_else(|| Error::msg(format!("{} doesn't report the position", s.app)))?;
                    let target = match (ctx.value, s.length) {
                        // Dragging: a share of the track.
                        (Some(v), Some(l)) => v / 100.0 * l,
                        (Some(_), None) => return ctx.fail(format!("{} doesn't report the track length", s.app)),
                        // Dial or +/-: seconds per step.
                        (None, _) => pos + ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(5.0) * ctx.delta.unwrap_or(0.0),
                    };
                    let target = target.clamp(0.0, s.length.unwrap_or(f64::MAX));
                    media::control(&s.id, Cmd::Seek(target - pos))
                }),
        )
        .action(
            Action::wheel("players-wheel", "Players Wheel")
                .icon("disc-3")
                .on_open(|o| {
                    let list = media::sessions().or_else(|e| o.fail(e.message().to_string()))?;
                    Ok(list.into_iter().take(16).map(|s| json!({"id": s.id, "label": format!("{}\n{}", s.app, s.title), "icon": if s.playing { "pause" } else { "play" }})).collect())
                })
                .on_child_press(|ctx| {
                    let Some(id) = ctx.child_id.clone() else { return Ok(()) };
                    media::control(&id, Cmd::PlayPause)
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_paths() {
        let s = Session { id: "x".into(), app: "VLC".into(), title: "Song".into(), artist: "Band".into(), playing: true, ..Default::default() };
        assert_eq!(clock(187.4), "3:07");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(track_label(&s), "Song\nBand");
        assert_eq!(track_label(&Session { title: String::new(), ..s.clone() }), "VLC");
        assert_eq!(percent_decode("/home/a%20b/c%C3%A9.jpg"), "/home/a b/cé.jpg");
        assert_eq!(percent_decode("/x%2"), "/x%2");
    }

    #[test]
    fn covers_shrink() {
        let img = image::RgbImage::from_pixel(600, 400, image::Rgb([200, 30, 30]));
        let mut png = std::io::Cursor::new(vec![]);
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let uri = shrink(&png.into_inner()).unwrap();
        assert!(uri.starts_with("data:image/jpeg;base64,"));
        assert!(uri.len() < 20_000);
    }
}
