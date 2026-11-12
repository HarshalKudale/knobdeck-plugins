//! YouTube Music plugin for KnobDeck — controls the YouTube Music Desktop App (YTMDesktop 2.x)
//! through its Companion Server: playback, likes, volume, repeat, shuffle, queue and playlists.

#[allow(dead_code)] // the sign-in calls are used by setup
mod ytmd;

use base64::Engine;
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, load_env, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::io::Read as _;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use ytmd::Conn;

const POLL: Duration = Duration::from_secs(1);

/// The newest state: pushed by the realtime feed, or fetched when the feed is down.
static STATE: Mutex<Option<(Instant, bool, Value)>> = Mutex::new(None);

fn conn() -> Result<Conn> {
    Conn::from_env()
}

fn guard<T>(r: Result<T>) -> Result<T> {
    if let Err(e) = &r {
        if e.message() == ytmd::UNAUTHORIZED {
            knobdeck_sdk::mark_setup_required(e.message());
        }
    }
    r
}

fn state() -> Result<Value> {
    if let Some((at, live, v)) = STATE.lock().unwrap().as_ref() {
        // A realtime state stays good while the feed is up; a fetched one for a second.
        if (*live && at.elapsed() < Duration::from_secs(30)) || at.elapsed() < Duration::from_millis(900) {
            return Ok(v.clone());
        }
    }
    let v = guard(guard(conn())?.get("state"))?;
    *STATE.lock().unwrap() = Some((Instant::now(), false, v.clone()));
    Ok(v)
}

fn command(name: &str, data: Option<Value>) -> Result<()> {
    guard(guard(conn())?.command(name, data))?;
    // The realtime feed sends the new state; without it, fetch fresh on the next read.
    if let Some(s) = STATE.lock().unwrap().as_mut() {
        if !s.1 {
            s.0 = Instant::now() - Duration::from_secs(60);
        }
    }
    Ok(())
}

fn run_feed() {
    let mut refused = 0;
    loop {
        if let Ok(c) = conn() {
            let r = ytmd::realtime(&c, &|s| *STATE.lock().unwrap() = Some((Instant::now(), true, s)));
            if let Some(s) = STATE.lock().unwrap().as_mut() {
                s.1 = false;
            }
            match r {
                Err(e) if e.starts_with("refused") => {
                    refused += 1;
                    knobdeck_sdk::log(format!("realtime: {e}"));
                    if refused >= 3 {
                        knobdeck_sdk::mark_setup_required(ytmd::UNAUTHORIZED);
                        return;
                    }
                }
                Err(_) | Ok(()) => refused = 0,
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// Album art as a data URI (keys take Lucide names or `data:` URIs). The last few are cached.
static ART: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());

fn art(url: &str) -> Option<String> {
    if let Some((_, hit)) = ART.lock().unwrap().iter().find(|(u, _)| u == url) {
        return hit.clone();
    }
    let uri = (|| {
        let resp = ureq::get(url).timeout(Duration::from_secs(5)).call().ok()?;
        let mime = resp.content_type().to_string();
        let mut bytes = vec![];
        std::io::Read::read_to_end(&mut resp.into_reader().take(512 * 1024), &mut bytes).ok()?;
        Some(format!("data:{};base64,{}", if mime.starts_with("image/") { mime } else { "image/jpeg".into() }, base64::engine::general_purpose::STANDARD.encode(bytes)))
    })();
    let mut cache = ART.lock().unwrap();
    if cache.len() >= 8 {
        cache.remove(0);
    }
    cache.push((url.to_string(), uri.clone()));
    uri
}

fn repeat_state(mode: i64) -> &'static str {
    match mode {
        1 => "all",
        2 => "one",
        _ => "none",
    }
}

fn step(ctx: &Ctx, default: f64) -> f64 {
    ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(default)
}

fn button(id: &str, label: &str, icon: &str, cmd: &'static str) -> Action {
    Action::button(id, label).icon(icon).on_press(move |_| command(cmd, None))
}

fn playlists() -> Result<Vec<Value>> {
    Ok(guard(guard(conn())?.get("playlists"))?.as_array().cloned().unwrap_or_default())
}

fn main() {
    load_env();
    Plugin::new(&format!("ytmusic-{}", std::env::consts::OS), "YouTube Music Desktop")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Control the YouTube Music Desktop App: playback, likes, volume, queue and playlists.")
        .icon("music")
        .setup("ytmusic-setup", None)
        .setup_inputs(vec![
            input::text("host", "Address (optional)").placeholder("127.0.0.1").with("help", "Leave empty when YouTube Music Desktop runs on this computer."),
            input::text("port", "Port (optional)").placeholder(ytmd::DEFAULT_PORT),
        ])
        .ack_timeout_ms(8000)
        .query_timeout_ms(40000) // the playlists request can take up to 30 s
        .on_ready(|_| {
            std::thread::spawn(run_feed);
            Ok(())
        })
        .query("playlists", |_| Ok(Value::Array(playlists()?.into_iter().map(|p| json!({"value": p["id"], "label": p["title"]})).collect())))
        .action(
            Action::toggle("play-pause", "Play / Pause")
                .icon("play")
                .state("paused", "Paused", "play")
                .state("playing", "Playing", "pause")
                .live(POLL, |_| Ok(Visual::new().state(if ytmd::playing(&state()?) { "playing" } else { "paused" })))
                .on_press(|ctx| {
                    command("playPause", None)?;
                    ctx.fan_out(if ctx.state.as_deref() == Some("playing") { "paused" } else { "playing" });
                    Ok(())
                }),
        )
        .action(button("next", "Next", "skip-forward", "next"))
        .action(button("previous", "Previous", "skip-back", "previous"))
        .action(
            Action::button("now-playing", "Now Playing")
                .icon("music")
                .inputs(vec![input::checkbox("art", "Show album art").default_value(true)])
                .live(POLL, |ic| {
                    let s = state()?;
                    let t = ytmd::title(&s);
                    if t.is_empty() {
                        return Ok(Visual::new().label("Nothing playing").icon("music"));
                    }
                    let mut v = Visual::new().label(format!("{t}\n{}", ytmd::author(&s)));
                    v = match ytmd::thumbnail(&s).filter(|_| ic.input_bool("art")).and_then(|u| art(&u)) {
                        Some(uri) => v.icon(uri),
                        None => v.icon(if ytmd::playing(&s) { "pause" } else { "play" }),
                    };
                    Ok(v)
                })
                .on_press(|_| command("playPause", None)),
        )
        .action(
            Action::toggle("like", "Like")
                .icon("thumbs-up")
                .state("none", "Not liked", "thumbs-up")
                .state("liked", "Liked", "heart")
                .live(POLL, |_| Ok(Visual::new().state(if ytmd::like_status(&state()?) == 2 { "liked" } else { "none" })))
                .on_press(|ctx| {
                    command("toggleLike", None)?;
                    ctx.fan_out(if ctx.state.as_deref() == Some("liked") { "none" } else { "liked" });
                    Ok(())
                }),
        )
        .action(
            Action::toggle("dislike", "Dislike")
                .icon("thumbs-down")
                .state("none", "Not disliked", "thumbs-down")
                .state("disliked", "Disliked", "thumbs-down")
                .live(POLL, |_| Ok(Visual::new().state(if ytmd::like_status(&state()?) == 0 { "disliked" } else { "none" })))
                .on_press(|_| command("toggleDislike", None)),
        )
        .action(
            Action::slider("volume", "Volume", 0.0, 100.0)
                .icon("volume-2")
                .unit("%")
                .inputs(vec![input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(POLL, |_| Ok(Visual::new().state(ytmd::volume(&state()?).round())))
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => ytmd::volume(&state()?) + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(0.0, 100.0);
                    command("setVolume", Some(json!(target as i64)))?;
                    ctx.fan_out(target);
                    Ok(())
                }),
        )
        .action(
            Action::toggle("mute", "Mute")
                .icon("volume-2")
                .state("unmuted", "Sound on", "volume-2")
                .state("muted", "Muted", "volume-x")
                .on_press(|ctx| {
                    // The state has no mute flag, so the key remembers what it last did.
                    let mute = ctx.state.as_deref() != Some("muted");
                    command(if mute { "mute" } else { "unmute" }, None)?;
                    ctx.fan_out(if mute { "muted" } else { "unmuted" });
                    Ok(())
                }),
        )
        .action(button("shuffle", "Shuffle", "shuffle", "shuffle"))
        .action(
            Action::toggle("repeat", "Repeat")
                .icon("repeat")
                .state("none", "Repeat off", "repeat")
                .state("all", "Repeat all", "repeat")
                .state("one", "Repeat one", "repeat-1")
                .live(POLL, |_| Ok(Visual::new().state(repeat_state(ytmd::repeat_mode(&state()?)))))
                .on_press(|ctx| {
                    let now = ytmd::repeat_mode(&state()?).max(0);
                    let next = (now + 1) % 3;
                    command("repeatMode", Some(json!(next)))?;
                    ctx.fan_out(repeat_state(next));
                    Ok(())
                }),
        )
        .action(Action::button("seek", "Seek").icon("fast-forward").inputs(vec![input::text("seconds", "Seconds (negative goes back)").placeholder("10").default_value("10")]).on_press(|ctx| {
            let by = ctx.input_f64("seconds").unwrap_or(10.0);
            let s = state()?;
            let now = s["player"]["videoProgress"].as_f64().unwrap_or(0.0);
            let len = s["video"]["durationSeconds"].as_f64().unwrap_or(0.0);
            if len <= 0.0 {
                return ctx.fail("Nothing playing");
            }
            command("seekTo", Some(json!((now + by).clamp(0.0, len).round() as i64)))
        }))
        .action(Action::button("play-playlist", "Play Playlist").icon("list-music").inputs(vec![input::query_select("playlist", "Playlist", "playlists", &[])]).on_press(|ctx| {
            let id = ctx.input_str("playlist");
            if id.is_empty() {
                return ctx.fail("Choose a playlist");
            }
            command("changeVideo", Some(json!({"videoId": null, "playlistId": id})))
        }))
        .action(
            Action::wheel("playlists-wheel", "Playlists Wheel")
                .icon("list-music")
                .on_open(|o| {
                    let list = playlists().or_else(|e| o.fail(e.message().to_string()))?;
                    Ok(list.into_iter().take(16).filter_map(|p| Some(json!({"id": p["id"].as_str()?, "label": p["title"], "icon": "list-music"}))).collect())
                })
                .on_child_press(|ctx| match ctx.child_id.as_deref() {
                    Some(id) => command("changeVideo", Some(json!({"videoId": null, "playlistId": id}))),
                    None => Ok(()),
                }),
        )
        .action(
            Action::wheel("queue-wheel", "Up Next Wheel")
                .icon("list-ordered")
                .on_open(|o| {
                    let s = state().or_else(|e| o.fail(e.message().to_string()))?;
                    let next = ytmd::up_next(&s);
                    if next.is_empty() {
                        return o.fail("The queue is empty");
                    }
                    Ok(next.into_iter().take(16).map(|(i, t, a)| json!({"id": i.to_string(), "label": if a.is_empty() { t } else { format!("{t} · {a}") }, "icon": "music"})).collect())
                })
                .on_child_press(|ctx| {
                    let Some(i) = ctx.child_id.as_deref().and_then(|c| c.parse::<i64>().ok()) else { return Err(Error::msg("Unknown queue item")) };
                    command("playQueueIndex", Some(json!(i)))
                }),
        )
        .run()
}
