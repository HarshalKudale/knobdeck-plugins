//! Apple Music plugin for KnobDeck: playback, shuffle, repeat, favorites, volume, playlists and
//! what is playing, through Music.app's own AppleScript dictionary. macOS only: Music is.
//!
//! Scripts run through `osascript` (`NSAppleScript` is main-thread only and handlers run on worker
//! threads), as in now-playing and slides. Only Play / Pause and Play Playlist open Music; every
//! other script first checks that Music is running. KnobDeck asks once for the Automation
//! permission (KnobDeck → Music).

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod script;

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("apple-music only runs on macOS");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    app::run()
}

#[cfg(target_os = "macos")]
mod app {
    use crate::script::{self, Status, NOT_RUNNING};
    use base64::Engine as _;
    use knobdeck_sdk::input::{self, InputExt};
    use knobdeck_sdk::{json, Action, Ctx, Error, Plugin, Result, Value, Visual};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const POLL: Duration = Duration::from_secs(2);
    /// Keys polling together share one read.
    const FRESH: Duration = Duration::from_millis(900);
    const ART_PX: u32 = 144;
    const ICON: &str = "music";

    fn osascript(src: &str) -> Result<String> {
        let out = std::process::Command::new("osascript").args(["-e", src]).output().map_err(|e| Error::msg(format!("osascript failed: {e}")))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            knobdeck_sdk::log(format!("Music: {}", err.trim()));
            return Err(Error::msg(script::clean_error(&err)));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim_end_matches('\n').to_string())
    }

    /// Runs `body` in Music only when it already runs.
    fn tell(body: &str) -> Result<String> {
        let out = osascript(&script::guarded(body))?;
        if out == NOT_RUNNING {
            return Err(Error::msg("Music isn't open"));
        }
        invalidate();
        Ok(out)
    }

    /// Runs `body` in Music, opening it first when needed (play actions only).
    fn tell_launching(body: &str) -> Result<String> {
        let out = osascript(&script::launching(body))?;
        invalidate();
        Ok(out)
    }

    static CACHE: Mutex<Option<(Instant, Option<Status>)>> = Mutex::new(None);

    fn invalidate() {
        *CACHE.lock().unwrap() = None;
    }

    /// Music's state; `None` when Music isn't open.
    fn status() -> Result<Option<Status>> {
        if let Some((at, s)) = CACHE.lock().unwrap().as_ref() {
            if at.elapsed() < FRESH {
                return Ok(s.clone());
            }
        }
        let out = osascript(&script::guarded(script::STATUS))?;
        let s = (out != NOT_RUNNING).then(|| script::parse_status(&out));
        *CACHE.lock().unwrap() = Some((Instant::now(), s.clone()));
        Ok(s)
    }

    /// Artwork as a small JPEG data URI, read once per track.
    static ART: Mutex<Option<(String, Option<String>)>> = Mutex::new(None);

    fn artwork(s: &Status) -> Option<String> {
        if s.track_id.is_empty() {
            return None;
        }
        if let Some((id, uri)) = ART.lock().unwrap().as_ref() {
            if *id == s.track_id {
                return uri.clone();
            }
        }
        let path = knobdeck_sdk::data_dir().join("artwork.img");
        let _ = std::fs::create_dir_all(knobdeck_sdk::data_dir());
        let uri = match osascript(&script::save_artwork(&path.to_string_lossy())) {
            Ok(r) if r == "ok" => std::fs::read(&path).ok().and_then(|b| shrink(&b)),
            _ => None,
        };
        let _ = std::fs::remove_file(&path);
        *ART.lock().unwrap() = Some((s.track_id.clone(), uri.clone()));
        uri
    }

    fn shrink(bytes: &[u8]) -> Option<String> {
        let img = image::load_from_memory(bytes).ok()?;
        let small = img.thumbnail(ART_PX, ART_PX).to_rgb8();
        let mut out = std::io::Cursor::new(vec![]);
        small.write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
        Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(out.into_inner())))
    }

    fn play_label(playing: bool) -> &'static str {
        if playing {
            "playing"
        } else {
            "paused"
        }
    }

    fn play_pause() -> Result<()> {
        // Opens Music when it's closed: playing is the one thing worth launching it for.
        tell_launching("playpause").map(|_| ())
    }

    pub fn run() {
        knobdeck_sdk::load_env();
        Plugin::new("apple-music-macos", "Apple Music")
            .version(env!("CARGO_PKG_VERSION"))
            .platforms(&["macos"])
            .author("KnobDeck")
            .description("Music app playback, shuffle, repeat, favorites, volume and playlists, with what is playing and its artwork.")
            .icon(ICON)
            .query("playlists", |q| {
                let out = tell(script::PLAYLISTS).or_else(|e| q.fail(format!("{} (open Music to list playlists)", e.message())))?;
                Ok(Value::Array(script::parse_playlists(&out).into_iter().map(|p| json!({"value": p, "label": p})).collect()))
            })
            .action(
                Action::button("now-playing", "Now Playing")
                    .icon(ICON)
                    .inputs(vec![input::checkbox("artwork", "Show the artwork").default_value(true), input::checkbox("press", "Press to play / pause").default_value(true)])
                    .live(POLL, |ic| {
                        let Some(s) = status()? else { return Ok(Visual::new().label("Music closed").icon(ICON)) };
                        if s.state == "stopped" {
                            return Ok(Visual::new().label("Nothing playing").icon(ICON));
                        }
                        let art = if ic.input_bool("artwork") { artwork(&s) } else { None };
                        Ok(Visual::new().label(script::track_label(&s)).icon(art.unwrap_or_else(|| if s.playing() { ICON.into() } else { "pause".into() })))
                    })
                    .on_press(|ctx| if ctx.input_bool("press") { play_pause() } else { Ok(()) }),
            )
            .action(
                Action::toggle("play-pause", "Play / Pause")
                    .icon("play")
                    .state("paused", "Play", "play")
                    .state("playing", "Pause", "pause")
                    .live(POLL, |_| Ok(Visual::new().state(play_label(status()?.is_some_and(|s| s.playing())))))
                    .on_press(|ctx| {
                        let was = status().ok().flatten().is_some_and(|s| s.playing());
                        play_pause()?;
                        ctx.set_state(play_label(!was));
                        Ok(())
                    }),
            )
            .action(Action::button("next", "Next Track").icon("skip-forward").on_press(|_| tell("next track").map(|_| ())))
            .action(Action::button("previous", "Previous Track").icon("skip-back").on_press(|_| tell("previous track").map(|_| ())))
            .action(
                Action::toggle("shuffle", "Shuffle")
                    .icon("shuffle")
                    .state("off", "Shuffle off", "shuffle")
                    .state("on", "Shuffle on", "shuffle")
                    .live(POLL, |_| Ok(Visual::new().state(if status()?.is_some_and(|s| s.shuffle) { "on" } else { "off" })))
                    .on_press(|ctx| {
                        let on = tell("set shuffle enabled to not shuffle enabled\nreturn shuffle enabled as string")? == "true";
                        ctx.set_state(if on { "on" } else { "off" });
                        Ok(())
                    }),
            )
            .action(
                Action::toggle("repeat", "Repeat")
                    .icon("repeat")
                    .state("off", "Repeat off", "repeat")
                    .state("all", "Repeat all", "repeat")
                    .state("one", "Repeat one", "repeat-1")
                    .live(POLL, |_| Ok(Visual::new().state(status()?.map(|s| s.repeat).filter(|r| ["off", "all", "one"].contains(&r.as_str())).unwrap_or_else(|| "off".into()))))
                    .on_press(|ctx| {
                        let cur = tell("return song repeat as string")?;
                        let next = script::next_repeat(&cur);
                        tell(&format!("set song repeat to {next}"))?;
                        ctx.set_state(next);
                        Ok(())
                    }),
            )
            .action(
                Action::toggle("favorite", "Favorite Track")
                    .icon("heart")
                    .state("off", "Favorite", "heart")
                    .state("on", "Favorited", "heart")
                    .live(POLL, |_| Ok(Visual::new().state(if status()?.is_some_and(|s| s.favorite) { "on" } else { "off" })))
                    .on_press(|ctx| {
                        let on = tell(script::TOGGLE_FAVORITE)? == "true";
                        ctx.set_state(if on { "on" } else { "off" });
                        Ok(())
                    }),
            )
            .action(
                Action::slider("volume", "Music Volume", 0.0, 100.0)
                    .icon("volume-2")
                    .unit("%")
                    .inputs(vec![input::text("step", "Percent per dial step").placeholder("5").default_value("5")])
                    .live(POLL, |_| Ok(Visual::new().state(status()?.map_or(0.0, |s| s.volume).round())))
                    .on_adjust(|ctx: &Ctx| {
                        let target = match ctx.value {
                            Some(v) => v,
                            None => {
                                let cur: f64 = tell("return sound volume as string")?.replace(',', ".").parse().unwrap_or(0.0);
                                cur + ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(5.0) * ctx.delta.unwrap_or(0.0)
                            }
                        };
                        let v = target.clamp(0.0, 100.0).round();
                        tell(&format!("set sound volume to {v}"))?;
                        ctx.set_state(v);
                        Ok(())
                    }),
            )
            .action(
                Action::button("play-playlist", "Play Playlist")
                    .icon("list-music")
                    .inputs(vec![input::query_combobox("playlist", "Playlist", "playlists"), input::checkbox("shuffle", "Shuffle it").default_value(false)])
                    .on_press(|ctx| {
                        let name = ctx.input_str("playlist");
                        if name.trim().is_empty() {
                            return ctx.fail("Choose a playlist");
                        }
                        let lit = script::literal(&name);
                        let shuffle = if ctx.input_bool("shuffle") { "set shuffle enabled to true\n" } else { "" };
                        tell_launching(&format!("if not (exists playlist {lit}) then error \"No playlist named \" & {lit}\n{shuffle}play playlist {lit}")).map(|_| ())
                    }),
            )
            .run()
    }
}
