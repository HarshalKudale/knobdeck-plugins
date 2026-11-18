//! TeamSpeak plugin for KnobDeck: microphone and speaker mute, away, push-to-talk, and the current
//! channel / who is talking. TeamSpeak 5 and 6 through their Remote Apps WebSocket (button presses
//! the user binds to TeamSpeak actions, state from its events); TeamSpeak 3 through ClientQuery
//! (direct `clientupdate` commands).

mod query;
mod remote;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, Action, Ctx, Error, Plugin, Result, Visual};
use query::Status;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(1);

fn ts3() -> bool {
    env_or("TEAMSPEAK_CLIENT", "remote-apps") == "clientquery"
}

fn host() -> String {
    env_or("TEAMSPEAK_HOST", "127.0.0.1")
}

fn port() -> String {
    let p = env_or("TEAMSPEAK_PORT", "");
    if !p.is_empty() {
        p
    } else if ts3() {
        "25639".into()
    } else {
        "5899".into()
    }
}

fn ts3_endpoint() -> query::Endpoint {
    query::Endpoint { host: host(), port: port(), key: knobdeck_sdk::secrets::env_or_stored(query::KEY) }
}

/// The current state, from whichever client is set up.
fn status() -> Result<Status> {
    if ts3() {
        return query::status(&ts3_endpoint()).map_err(Error::msg);
    }
    remote::ensure_started(host(), port());
    let m = remote::MODEL.lock().unwrap();
    match m.phase {
        remote::Phase::Ready => {}
        remote::Phase::Waiting => return Err(Error::msg("Allow KnobDeck in TeamSpeak (it asks in a notification, or under Settings ▸ Remote Apps)")),
        remote::Phase::Offline => {
            drop(m);
            return Err(Error::msg(remote::offline_message()));
        }
    }
    let (conn, me) = m.active().ok_or_else(|| Error::msg("TeamSpeak isn't connected to a server"))?;
    let mut talking: Vec<String> = conn.clients.values().filter(|c| c.channel == me.channel && c.talking).map(|c| c.nick.clone()).collect();
    talking.sort();
    Ok(Status { input_muted: me.input_muted, output_muted: me.output_muted, away: me.away, channel: conn.channels.get(&me.channel).cloned().unwrap_or_default(), talking })
}

/// Press and release a Remote Apps button.
fn tap(button: &str) -> Result<()> {
    remote::press(button, true).map_err(Error::msg)?;
    std::thread::sleep(Duration::from_millis(60));
    remote::press(button, false).map_err(Error::msg)
}

#[derive(Clone, Copy)]
struct Flag {
    id: &'static str,
    label: &'static str,
    /// The Remote Apps button name (TeamSpeak 5/6) and the ClientQuery property (TeamSpeak 3).
    button: &'static str,
    property: &'static str,
    /// The TeamSpeak key-binding action to bind the button to.
    binding: &'static str,
    get: fn(&Status) -> bool,
    states: [(&'static str, &'static str, &'static str); 2],
}

fn flip(ctx: &Ctx, f: Flag) -> Result<()> {
    let before = (f.get)(&status()?);
    if ts3() {
        query::set(&ts3_endpoint(), f.property, !before).map_err(Error::msg)?;
    } else {
        tap(f.button)?;
        // TeamSpeak reports the change as an event; wait briefly for it.
        let deadline = Instant::now() + Duration::from_millis(800);
        while (f.get)(&status()?) == before {
            if Instant::now() > deadline {
                return ctx.fail(format!(
                    "TeamSpeak didn't change. Bind this key once: TeamSpeak ▸ Settings ▸ Key Bindings ▸ add \"{}\", then press this KnobDeck key when TeamSpeak waits for one",
                    f.binding
                ));
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }
    ctx.fan_out(f.states[!before as usize].0);
    Ok(())
}

fn flag_action(f: Flag) -> Action {
    let mut a = Action::toggle(f.id, f.label).icon(f.states[1].2);
    for (v, l, i) in f.states {
        a = a.state(v, l, i);
    }
    a.live(POLL, move |_| Ok(Visual::new().state(f.states[(f.get)(&status()?) as usize].0))).on_press(move |ctx| flip(ctx, f))
}

const MIC: Flag = Flag {
    id: "mute-mic",
    label: "Mute Microphone",
    button: "mute-microphone",
    property: "client_input_muted",
    binding: "Toggle Microphone Mute",
    get: |s| s.input_muted,
    states: [("unmuted", "Mic On", "mic"), ("muted", "Mic Muted", "mic-off")],
};
const SPEAKERS: Flag = Flag {
    id: "mute-speakers",
    label: "Mute Speakers",
    button: "mute-speakers",
    property: "client_output_muted",
    binding: "Toggle Sound Mute",
    get: |s| s.output_muted,
    states: [("unmuted", "Sound On", "volume-2"), ("muted", "Sound Muted", "volume-x")],
};
const AWAY: Flag = Flag {
    id: "away",
    label: "Away",
    button: "away",
    property: "client_away",
    binding: "Toggle Away",
    get: |s| s.away,
    states: [("present", "Present", "user-round-check"), ("away", "Away", "moon")],
};

fn push_to_talk(down: bool) -> Result<()> {
    if ts3() {
        // TeamSpeak 3 has no push-to-talk command: deactivating the microphone between presses is
        // the ClientQuery way (capture mode stays continuous / voice activation).
        query::set(&ts3_endpoint(), "client_input_deactivated", !down).map_err(Error::msg)
    } else {
        status()?; // a clear error when TeamSpeak isn't there
        remote::press("push-to-talk", down).map_err(Error::msg)
    }
}

fn main() {
    knobdeck_sdk::load_env();
    let os = std::env::consts::OS;
    Plugin::new(&format!("teamspeak-{os}"), "TeamSpeak")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[os]) // one package per OS
        .author("KnobDeck")
        .description("TeamSpeak microphone and speaker mute, away, push-to-talk, and the current channel and who is talking. TeamSpeak 5/6 (Remote Apps) and TeamSpeak 3 (ClientQuery).")
        .icon("headset")
        .setup("teamspeak-setup", None)
        .setup_inputs(vec![
            input::select("client", "TeamSpeak version", &[("remote-apps", "TeamSpeak 5 or 6 (Remote Apps)"), ("clientquery", "TeamSpeak 3 (ClientQuery)")]).default_value("remote-apps"),
            input::text("host", "Host").placeholder("127.0.0.1").default_value("127.0.0.1"),
            input::text("port", "Port (optional)").placeholder("5899 for TeamSpeak 5/6, 25639 for TeamSpeak 3"),
            input::text("apikey", "TeamSpeak 3 ClientQuery API key").with("type", "password").with("help", "TeamSpeak 3 only: Settings ▸ Addons ▸ ClientQuery ▸ Settings. TeamSpeak 5/6 ask you to allow KnobDeck instead. Kept in this computer's credential store."),
        ])
        .action(flag_action(MIC))
        .action(flag_action(SPEAKERS))
        .action(flag_action(AWAY))
        .action(
            Action::hold("push-to-talk", "Push to Talk")
                .icon("mic")
                .released("Push to Talk", "mic-off")
                .held("Talking", "mic")
                .on_hold_start(|ctx| {
                    push_to_talk(true)?;
                    ctx.set_state("held");
                    Ok(())
                })
                .on_hold_end(|ctx| {
                    push_to_talk(false)?;
                    ctx.set_state("released");
                    Ok(())
                }),
        )
        .action(
            Action::button("channel", "Current Channel")
                .icon("hash")
                .live(POLL, |_| {
                    let s = status()?;
                    Ok(Visual::new().label(if s.channel.is_empty() { "—".to_string() } else { s.channel }))
                })
                .on_press(|_| Ok(())),
        )
        .action(
            Action::button("talking", "Who's Talking")
                .icon("audio-lines")
                .live(Duration::from_millis(500), |_| {
                    let s = status()?;
                    Ok(Visual::new().label(if s.talking.is_empty() { "Quiet".to_string() } else { s.talking.join("\n") }).icon(if s.talking.is_empty() { "audio-lines" } else { "audio-waveform" }))
                })
                .on_press(|_| Ok(())),
        )
        .action(
            Action::hold("button", "TeamSpeak Button (5/6)")
                .icon("square-mouse-pointer")
                .released("TeamSpeak", "square-mouse-pointer")
                .held("Pressed", "square-mouse-pointer")
                .inputs(vec![input::text("name", "Button name").placeholder("whisper-team").with(
                    "help",
                    "TeamSpeak 5/6 only. Bind it to any TeamSpeak action (whisper, a sound, a channel switch): Settings ▸ Key Bindings, then press this key when TeamSpeak waits for one. Held keys stay pressed in TeamSpeak.",
                )])
                .on_hold_start(|ctx| remote_button(ctx, true))
                .on_hold_end(|ctx| remote_button(ctx, false)),
        )
        .run()
}

fn remote_button(ctx: &Ctx, down: bool) -> Result<()> {
    if ts3() {
        return ctx.fail("TeamSpeak 3 has no Remote Apps buttons: use TeamSpeak 5 or 6");
    }
    let name = ctx.input_str("name");
    if name.trim().is_empty() {
        return ctx.fail("Enter a button name");
    }
    status()?;
    remote::press(&format!("custom.{}", name.trim()), down).map_err(Error::msg)?;
    ctx.set_state(if down { "held" } else { "released" });
    Ok(())
}
