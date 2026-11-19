//! Spotify plugin for KnobDeck. Play/pause, next, previous and now playing go through the OS
//! (no account, setup or Premium needed). Likes, shuffle, repeat, volume, devices, playlists and the
//! queue go through the Spotify Web API with the user's own Spotify app (setup; see web.rs).

mod os;
mod web;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, load_env, Action, Plugin, Result, Value, Visual};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(2);
/// Web API keys poll more slowly: every key costs a request against the user's app's rate limit.
const WEB_POLL: Duration = Duration::from_secs(5);

/// Repeat modes in the order a press cycles through them.
const REPEAT: [&str; 3] = ["off", "context", "track"];

fn playing_label(n: &os::NowPlaying) -> String {
    match (n.title.is_empty(), n.artist.is_empty()) {
        (true, _) => "Spotify".into(),
        (false, true) => n.title.clone(),
        (false, false) => format!("{} — {}", n.title, n.artist),
    }
}

/// Whether the current track is in Liked Songs; asked again when the track changes, else every 30 s.
static LIKED: Mutex<Option<(String, Instant, bool)>> = Mutex::new(None);

fn liked() -> Result<bool> {
    let uri = web::current_track()?;
    if let Some((u, at, v)) = LIKED.lock().unwrap().as_ref() {
        if *u == uri && at.elapsed() < Duration::from_secs(30) {
            return Ok(*v);
        }
    }
    let v = web::saved(&uri)?;
    *LIKED.lock().unwrap() = Some((uri, Instant::now(), v));
    Ok(v)
}

/// Wheel slice id → playlist URI / device name, from each wheel's last open.
static WHEEL: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
static DEVICE_WHEEL: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn playlist_options() -> Result<Value> {
    Ok(Value::Array(web::playlists()?.into_iter().map(|(uri, name)| json!({"value": uri, "label": name})).collect()))
}

fn device_options() -> Result<Value> {
    Ok(Value::Array(web::devices()?.into_iter().map(|(_, name, kind, _)| json!({"value": name, "label": format!("{name} · {kind}")})).collect()))
}

fn device_icon(kind: &str) -> &'static str {
    match kind.to_ascii_lowercase().as_str() {
        "computer" => "laptop",
        "smartphone" => "smartphone",
        "tablet" => "tablet",
        "speaker" | "avr" | "stb" | "audiodongle" => "speaker",
        "tv" | "castvideo" => "tv",
        "automobile" => "car",
        "gameconsole" => "gamepad-2",
        _ => "monitor-speaker",
    }
}

fn play_context(uri: &str) -> Result<()> {
    web::call("PUT", "/me/player/play", Some(json!({"context_uri": uri}))).map(|_| ())
}

fn main() {
    load_env();
    Plugin::new(&format!("spotify-{}", std::env::consts::OS), "Spotify")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Play, pause, skip and now playing for the Spotify desktop app; likes, shuffle, repeat, volume, devices, playlists and the queue with your own Spotify app.")
        .icon("music")
        .ack_timeout_ms(5000)
        .setup("spotify-setup", None)
        .setup_inputs(vec![input::text("client_id", "Your Spotify app's client ID").placeholder("e.g. 0123456789abcdef0123456789abcdef").with(
            "help",
            "Leave empty to use only Play / Pause, Next, Previous and Now Playing (no account needed). For likes, shuffle, repeat, volume, devices, playlists and the queue: create an app at developer.spotify.com/dashboard with the redirect URI http://127.0.0.1:47811/callback and the Web API, then paste its client ID.",
        )])
        .query("get-playlists", |_| playlist_options())
        .query("get-devices", |_| device_options())
        // ── Through the OS: no account, setup or Premium ──
        .action(
            Action::toggle("play-pause", "Play / Pause")
                .icon("play")
                .state("paused", "Paused", "play")
                .state("playing", "Playing", "pause")
                .live(POLL, |_| Ok(match os::now_playing() {
                    Ok(n) => Visual::new().state(if n.playing { "playing" } else { "paused" }),
                    Err(_) => Visual::new().state("paused"),
                }))
                .on_press(|ctx| {
                    os::play_pause()?;
                    ctx.fan_out(if ctx.state.as_deref() == Some("playing") { "paused" } else { "playing" });
                    Ok(())
                }),
        )
        .action(Action::button("next", "Next Track").icon("skip-forward").on_press(|_| os::next()))
        .action(Action::button("previous", "Previous Track").icon("skip-back").on_press(|_| os::previous()))
        .action(
            Action::button("now-playing", "Now Playing")
                .icon("music")
                .live(POLL, |_| Ok(match os::now_playing() {
                    Ok(n) => Visual::new().label(playing_label(&n)).icon(if n.playing { "music" } else { "pause" }),
                    Err(e) => Visual::new().label(e.message().to_string()).icon("music"),
                }))
                .on_press(|_| os::play_pause()),
        )
        // ── Web API: the user's own Spotify app (setup); player commands need Premium ──
        .action(
            Action::toggle("like", "Like Song")
                .icon("heart")
                .state("not-liked", "Not Liked", "heart-plus")
                .state("liked", "Liked", "heart")
                .live(WEB_POLL, |_| Ok(Visual::new().state(if liked()? { "liked" } else { "not-liked" })))
                .on_press(|ctx| {
                    let uri = web::current_track()?;
                    let was = web::saved(&uri)?;
                    web::save(&uri, !was)?;
                    *LIKED.lock().unwrap() = Some((uri, Instant::now(), !was));
                    ctx.fan_out(if was { "not-liked" } else { "liked" });
                    Ok(())
                }),
        )
        .action(
            Action::toggle("shuffle", "Shuffle")
                .icon("shuffle")
                .state("off", "Shuffle Off", "arrow-right")
                .state("on", "Shuffle On", "shuffle")
                .live(WEB_POLL, |_| Ok(Visual::new().state(if web::player()?["shuffle_state"] == true { "on" } else { "off" })))
                .on_press(|ctx| {
                    let on = web::player()?["shuffle_state"] != true;
                    web::call("PUT", &format!("/me/player/shuffle?state={on}"), None)?;
                    ctx.fan_out(if on { "on" } else { "off" });
                    Ok(())
                }),
        )
        .action(
            Action::toggle("repeat", "Repeat")
                .icon("repeat")
                .state("off", "Repeat Off", "arrow-right")
                .state("context", "Repeat All", "repeat")
                .state("track", "Repeat One", "repeat-1")
                .live(WEB_POLL, |_| Ok(Visual::new().state(web::player()?["repeat_state"].as_str().unwrap_or("off"))))
                .on_press(|ctx| {
                    let now = web::player()?["repeat_state"].as_str().unwrap_or("off").to_string();
                    let next = REPEAT[(REPEAT.iter().position(|m| *m == now).unwrap_or(0) + 1) % REPEAT.len()];
                    web::call("PUT", &format!("/me/player/repeat?state={next}"), None)?;
                    ctx.fan_out(next);
                    Ok(())
                }),
        )
        .action(
            Action::slider("volume", "Spotify Volume", 0.0, 100.0)
                .icon("volume-2")
                .unit("%")
                .inputs(vec![input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(WEB_POLL, |_| Ok(match web::player()?["device"]["volume_percent"].as_i64() {
                    Some(v) => Visual::new().state(v),
                    None => Visual::new(),
                }))
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => web::player()?["device"]["volume_percent"].as_f64().unwrap_or(50.0) + ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(0.0, 100.0);
                    web::call("PUT", &format!("/me/player/volume?volume_percent={target}"), None)?;
                    ctx.fan_out(target);
                    Ok(())
                }),
        )
        .action(
            Action::button("device", "Play On Device")
                .icon("monitor-speaker")
                .inputs(vec![input::query_combobox("device", "Device", "get-devices").with("help", "A Spotify Connect device: this computer, a phone, a speaker. It must have Spotify open.")])
                .on_press(|ctx| {
                    let device = ctx.input_str("device");
                    if device.trim().is_empty() {
                        return ctx.fail("Choose a device first");
                    }
                    web::transfer(&device)
                }),
        )
        .action(
            Action::wheel("devices-wheel", "Devices")
                .icon("monitor-speaker")
                .on_open(|o| {
                    let list = web::devices().or_else(|e| o.fail(e.message().to_string()))?;
                    if list.is_empty() {
                        return o.fail("No Spotify devices: open Spotify on one");
                    }
                    let mut map = HashMap::new();
                    let slices = list
                        .into_iter()
                        .take(16)
                        .enumerate()
                        .map(|(i, (_, name, kind, active))| {
                            let id = format!("d{i}");
                            map.insert(id.clone(), name.clone());
                            json!({"id": id, "label": if active { format!("{name} · playing") } else { name }, "icon": device_icon(&kind)})
                        })
                        .collect();
                    *DEVICE_WHEEL.lock().unwrap() = Some(map);
                    Ok(slices)
                })
                .on_child_press(|ctx| {
                    let name = DEVICE_WHEEL.lock().unwrap().as_ref().and_then(|m| m.get(ctx.child_id.as_deref().unwrap_or("")).cloned());
                    match name {
                        Some(n) => web::transfer(&n),
                        None => ctx.fail("That device is no longer available"),
                    }
                }),
        )
        .action(Action::button("play-playlist", "Play Playlist").icon("list-music").inputs(vec![input::query_combobox("playlist", "Playlist", "get-playlists")]).on_press(|ctx| {
            let uri = ctx.input_str("playlist");
            if uri.is_empty() {
                return ctx.fail("Choose a playlist first");
            }
            play_context(&uri)
        }))
        .action(Action::button("add-to-playlist", "Add Song to Playlist").icon("list-plus").inputs(vec![input::query_combobox("playlist", "Playlist", "get-playlists")]).on_press(|ctx| {
            let uri = ctx.input_str("playlist");
            let Some(id) = uri.strip_prefix("spotify:playlist:") else {
                return ctx.fail("Choose one of your playlists (Liked Songs has its own Like key)");
            };
            let track = web::current_track()?;
            web::call("POST", &format!("/playlists/{id}/items"), Some(json!({"uris": [track]}))).map(|_| ())
        }))
        .action(
            Action::button("add-to-queue", "Add to Queue")
                .icon("list-end")
                .inputs(vec![input::text("link", "Song or episode link")
                    .placeholder("https://open.spotify.com/track/…")
                    .accepts_placeholders()
                    .with("help", "A Spotify link or URI (spotify:track:…). {clipboard} queues the link you copied.")])
                .on_press(|ctx| {
                    let text = ctx.expand(&ctx.input_str("link"))?;
                    let Some(uri) = web::queue_uri(&text) else {
                        return ctx.fail("That isn't a Spotify song or episode link");
                    };
                    web::call("POST", &format!("/me/player/queue?uri={}", knobdeck_sdk::oauth::encode(&uri)), None).map(|_| ())
                }),
        )
        .action(
            Action::wheel("playlists-wheel", "Playlists")
                .icon("list-music")
                .on_open(|ctx| {
                    let list = web::playlists().or_else(|e| ctx.fail(e.message().to_string()))?;
                    let mut map = HashMap::new();
                    let slices = list
                        .into_iter()
                        .take(16)
                        .enumerate()
                        .map(|(i, (uri, name))| {
                            let id = format!("p{i}");
                            map.insert(id.clone(), uri);
                            json!({"id": id, "label": name, "icon": if i == 0 { "heart" } else { "list-music" }})
                        })
                        .collect();
                    *WHEEL.lock().unwrap() = Some(map);
                    Ok(slices)
                })
                .on_child_press(|ctx| {
                    let uri = WHEEL.lock().unwrap().as_ref().and_then(|m| m.get(ctx.child_id.as_deref().unwrap_or("")).cloned());
                    match uri {
                        Some(u) => play_context(&u),
                        None => ctx.fail("That playlist is no longer available"),
                    }
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn media_session_manager_answers() {
        // Read-only: asks Windows for its media sessions; Spotify itself may be closed.
        eprintln!("media sessions: {}", super::os::session_count());
        assert!(matches!(super::os::now_playing(), Ok(_) | Err(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mpris_answers() {
        // Read-only: lists MPRIS players; Spotify itself may be closed.
        eprintln!("mpris players: {}", super::os::player_count());
        assert!(matches!(super::os::now_playing(), Ok(_) | Err(_)));
        if let Some((name, n)) = super::os::any_player_now_playing() {
            let n = n.unwrap();
            eprintln!("{name}: {:?} — {:?} playing={}", n.title, n.artist, n.playing);
        }
    }

    #[test]
    fn labels() {
        let n = |t: &str, a: &str| super::os::NowPlaying { title: t.into(), artist: a.into(), playing: true };
        assert_eq!(super::playing_label(&n("Song", "Band")), "Song — Band");
        assert_eq!(super::playing_label(&n("Song", "")), "Song");
        assert_eq!(super::playing_label(&n("", "")), "Spotify");
    }
}
