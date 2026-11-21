//! Lightroom Classic plugin for KnobDeck: Develop sliders on dials (live values, press to reset),
//! modules, photo navigation, flags, ratings, colour labels, rotation, copy / paste settings,
//! auto tone / white balance, reset, undo / redo and Develop views on keys.
//!
//! Everything goes through the KnobDeck plug-in for Lightroom Classic
//! (`Knobdeck/knobdeck-lightroom-companion`, `KnobDeck.lrplugin`), which runs each command with
//! the Lightroom Classic SDK (`LrDevelopController`, `LrApplicationView`, `LrSelection`, `LrUndo`,
//! the catalog's photos). Lightroom's `LrSocket` can't speak WebSocket, so the plug-in listens on
//! two loopback ports and this plugin connects to them (see `link.rs`). No keystrokes are sent.

mod link;
mod params;
mod proto;
#[cfg(test)]
mod fake;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, load_env, Action, Ctx, Error, Plugin, Result, Runtime, Value, Visual};
use link::{Change, Link, Photo, Status, Update};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

static LINK: OnceLock<Arc<Link>> = OnceLock::new();
static RT: OnceLock<Runtime> = OnceLock::new();
/// The setting whose panel was revealed last, so turning one dial doesn't keep scrolling Lightroom.
static REVEALED: Mutex<String> = Mutex::new(String::new());

const TIMEOUT: Duration = Duration::from_secs(4);
const TOKEN_KEY: &str = "LIGHTROOM_TOKEN";

/// The Basic panel's sliders, each a dial of its own: (action id, label, parameter, icon, unit).
const BASIC: &[(&str, &str, &str, &str, &str)] = &[
    ("temperature", "Temp", "Temperature", "thermometer", "K"),
    ("tint", "Tint", "Tint", "droplet", ""),
    ("exposure", "Exposure", "Exposure", "sun", "EV"),
    ("contrast", "Contrast", "Contrast", "contrast", ""),
    ("highlights", "Highlights", "Highlights", "sun-medium", ""),
    ("shadows", "Shadows", "Shadows", "moon", ""),
    ("whites", "Whites", "Whites", "circle", ""),
    ("blacks", "Blacks", "Blacks", "circle-dot", ""),
    ("texture", "Texture", "Texture", "grip", ""),
    ("clarity", "Clarity", "Clarity", "focus", ""),
    ("dehaze", "Dehaze", "Dehaze", "cloud-fog", ""),
    ("vibrance", "Vibrance", "Vibrance", "palette", ""),
    ("saturation", "Saturation", "Saturation", "paintbrush", ""),
];

const MODULES: &[(&str, &str)] = &[
    ("library", "Library"),
    ("develop", "Develop"),
    ("map", "Map"),
    ("book", "Book"),
    ("slideshow", "Slideshow"),
    ("print", "Print"),
    ("web", "Web"),
];

const LABELS: &[(&str, &str)] = &[("red", "Red"), ("yellow", "Yellow"), ("green", "Green"), ("blue", "Blue"), ("purple", "Purple"), ("none", "None")];

fn link() -> &'static Arc<Link> {
    LINK.get().expect("link starts in main")
}

fn request(fields: &[&str]) -> Result<Vec<String>> {
    link().request(fields, TIMEOUT).map_err(Error::msg)
}

/// The pairing token Lightroom is asked to allow; made once and kept in the credential store.
fn token() -> String {
    let t = knobdeck_sdk::secrets::env_or_stored(TOKEN_KEY);
    if !t.is_empty() {
        return t;
    }
    use std::hash::BuildHasher;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    // RandomState's keys come from the OS's random source.
    let t: String = (0..4u8).map(|i| format!("{:016x}", std::collections::hash_map::RandomState::new().hash_one((i, now, std::process::id())))).collect();
    if let Err(e) = knobdeck_sdk::secrets::set(TOKEN_KEY, &t) {
        knobdeck_sdk::log(format!("could not keep the pairing token ({}); Lightroom asks again next time", e.message()));
    }
    t
}

fn port(key: &str, default: u16) -> u16 {
    env_or(key, &default.to_string()).trim().parse().ok().filter(|p| *p >= 1024).unwrap_or(default)
}

// ── Showing values ─────────────────────────────────────────────────────────────

/// A develop value for a key: up to 2 decimals, no trailing zeros.
fn fmt(v: f64) -> String {
    proto::num((v * 100.0).round() / 100.0)
}

fn stars(r: u8) -> String {
    (0..5).map(|i| if i < r { '★' } else { '☆' }).collect()
}

fn module_label(m: &str) -> &str {
    MODULES.iter().find(|(id, _)| *id == m).map(|(_, l)| *l).unwrap_or(m)
}

fn status_label() -> String {
    let l = link();
    match l.status() {
        Status::Offline => "Lightroom offline".into(),
        Status::Pending => "Allow KnobDeck in Lightroom".into(),
        Status::Denied => "Refused in Lightroom".into(),
        Status::Linked { .. } => {
            let m = l.module().map(|m| module_label(&m).to_string()).unwrap_or_else(|| "Lightroom".into());
            match l.photo() {
                Some(p) => format!("{m}\n{}", p.name),
                None => m,
            }
        }
    }
}

fn param_of(inputs: &Value) -> String {
    match inputs["param"].as_str() {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => "Exposure".into(),
    }
}

fn str_input(inputs: &Value, key: &str, default: &str) -> String {
    match inputs[key].as_str() {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => default.to_string(),
    }
}

fn flag_state(p: &Option<Photo>) -> String {
    p.as_ref().map(|p| p.flag.clone()).unwrap_or_else(|| "none".into())
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// Push one change to the placed keys.
fn push(rt: &Runtime, u: &Update) {
    match u {
        Update::Value(p, v) => {
            for (id, _, param, _, _) in BASIC {
                if param == p {
                    rt.notify(id, fmt(*v));
                }
            }
            for inst in rt.instances_of("develop-slider") {
                if param_of(&rt.inputs_of(&inst)) == *p {
                    let short = params::find(p).map(|p| p.short).unwrap_or(p);
                    rt.set_visual(&inst, Visual::new().state(fmt(*v)).label(short));
                }
            }
        }
        Update::Module(m) => {
            for inst in rt.instances_of("module") {
                rt.set_state(&inst, on_off(str_input(&rt.inputs_of(&inst), "module", "develop") == *m));
            }
            rt.notify_visual("status", Visual::new().label(status_label()));
        }
        Update::Photo(p) => {
            rt.notify("flag", flag_state(p));
            let r = p.as_ref().map(|p| p.rating).unwrap_or(0);
            rt.notify_visual("rating", Visual::new().label(if p.is_some() { stars(r) } else { "No photo".into() }));
            rt.notify("rating-dial", r);
            let label = p.as_ref().map(|p| p.label.clone()).unwrap_or_default();
            for inst in rt.instances_of("color-label") {
                rt.set_state(&inst, on_off(p.is_some() && str_input(&rt.inputs_of(&inst), "label", "red") == label));
            }
            rt.notify_visual("status", Visual::new().label(status_label()));
        }
        Update::Status(_) => rt.notify_visual("status", Visual::new().label(status_label())),
        Update::Failed { instance, message } => rt.set_error(instance, message),
    }
}

// ── Develop dials ──────────────────────────────────────────────────────────────

fn dial_inputs() -> Vec<Value> {
    vec![
        input::select("steps", "Steps", &[("fine", "Fine (per tick below)"), ("lightroom", "Lightroom's own steps (as its + / - keys)")]).default_value("fine"),
        input::text("step", "Step per tick")
            .placeholder("auto")
            .with("help", "Blank: Exposure 0.05, Temp 50 K on raw photos, most others 1.")
            .with("showIf", json!({"key": "steps", "equals": "fine"})),
        input::checkbox("reveal", "Show the setting's panel in Lightroom").default_value(true),
    ]
}

fn adjust(ctx: &Ctx, param: &str) -> Result {
    let Some(p) = params::find(param) else { return ctx.fail(format!("Not a develop setting: {param}")) };
    let change = match (ctx.value, ctx.delta) {
        (Some(v), _) => Change::Set(v),
        (None, Some(d)) => Change::Ticks(d),
        _ => return Ok(()),
    };
    let step = if ctx.input_str("steps") == "lightroom" {
        "lr".to_string()
    } else {
        ctx.input_f64("step").filter(|s| *s > 0.0).map(proto::num).unwrap_or_default()
    };
    let reveal = ctx.inputs.get("reveal").and_then(Value::as_bool).unwrap_or(true);
    let panel = {
        let mut last = REVEALED.lock().unwrap();
        if reveal && *last != p.id {
            *last = p.id.to_string();
            p.panel
        } else {
            ""
        }
    };
    link().adjust(p.id, change, &step, panel, &ctx.instance_id).map_err(Error::msg)
}

/// Pressing a develop dial resets its setting.
fn reset(ctx: &Ctx, param: &str) -> Result {
    let r = request(&["reset", param])?;
    if let Some(v) = r.get(1).and_then(|v| v.parse::<f64>().ok()) {
        ctx.set_state(fmt(v));
    }
    Ok(())
}

fn basic_dial(id: &'static str, label: &str, param: &'static str, icon: &str, unit: &str) -> Action {
    let p = params::find(param).expect("basic params are in the table");
    let mut a = Action::slider(id, label, p.min, p.max).icon(icon).step(if param == "Exposure" { 0.05 } else { 1.0 });
    if !unit.is_empty() {
        a = a.unit(unit);
    }
    a.inputs(dial_inputs())
        .initial_with(move |_| link().value(param).map(fmt))
        .on_adjust(move |ctx| adjust(ctx, param))
        .on_press(move |ctx| reset(ctx, param))
}

fn develop_slider() -> Action {
    let options: Vec<(String, String)> = params::PARAMS.iter().map(|p| (p.id.to_string(), params::label(p))).collect();
    let opts: Vec<(&str, &str)> = options.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut inputs = vec![input::select("param", "Setting", &opts).default_value("Exposure")];
    inputs.extend(dial_inputs());
    Action::slider("develop-slider", "Develop Slider", -100.0, 100.0)
        .icon("sliders-horizontal")
        .step(1.0)
        .inputs(inputs)
        .live(Duration::from_secs(5), |ic| {
            let p = param_of(&ic.inputs);
            let short = params::find(&p).map(|x| x.short).unwrap_or("?");
            Ok(match link().value(&p) {
                Some(v) => Visual::new().state(fmt(v)).label(short),
                None => Visual::new().label(short),
            })
        })
        .on_adjust(|ctx| adjust(ctx, &param_of(&ctx.inputs)))
        .on_press(|ctx| reset(ctx, &param_of(&ctx.inputs)))
}

// ── Keys ───────────────────────────────────────────────────────────────────────

fn command(id: &str, label: &str, icon: &str, fields: &'static [&'static str]) -> Action {
    Action::button(id, label).icon(icon).on_press(move |_| request(fields).map(|_| ()))
}

fn main() {
    load_env();
    let os = std::env::consts::OS;
    let cfg = link::Config { command_port: port("LIGHTROOM_COMMAND_PORT", link::COMMAND_PORT), event_port: port("LIGHTROOM_EVENT_PORT", link::EVENT_PORT), token: token() };
    let l = Link::new(cfg, params::PARAMS.iter().map(|p| p.id.to_string()).collect());
    l.on_update(|u| {
        if let Some(rt) = RT.get() {
            push(rt, u);
        }
    });
    let _ = LINK.set(l.clone());
    l.start();

    let mut plugin = Plugin::new(&format!("lightroom-classic-{os}"), "Lightroom Classic")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[os]) // one package per OS; shipped for Windows and macOS (scripts/plugins.mjs)
        .author("KnobDeck")
        .description("Lightroom Classic on the dial: every Develop slider with its live value (press to reset), modules, next / previous photo, flags, ratings, colour labels, rotate, copy / paste settings, auto tone, undo and before / after, through the KnobDeck plug-in for Lightroom.")
        .icon("app-icon.svg")
        .ack_timeout_ms(6000)
        .setup("lightroom-classic-setup", None)
        .setup_inputs(vec![
            input::checkbox("install", "Install the KnobDeck plug-in into Lightroom Classic").default_value(true).with(
                "help",
                "Downloads KnobDeck.lrplugin from github.com/Knobdeck/knobdeck-lightroom-companion into Lightroom's Modules folder, where Lightroom loads it by itself. Restart Lightroom afterwards. Untick it if you add the plug-in yourself (File ▸ Plug-in Manager ▸ Add).",
            ),
            input::text("command_port", "Command port").placeholder("58770").default_value("58770").with("help", "The ports shown in Lightroom under File ▸ Plug-in Manager ▸ KnobDeck. Change them there and here only if another app uses them."),
            input::text("event_port", "Event port").placeholder("58771").default_value("58771"),
            input::checkbox("pair_again", "Pair again (Lightroom asks to allow KnobDeck once more)").default_value(false),
        ]);

    for (id, label, param, icon, unit) in BASIC {
        plugin = plugin.action(basic_dial(id, label, param, icon, unit));
    }

    plugin
        .action(develop_slider())
        // Status and modules
        .action(
            Action::button("status", "Lightroom Status")
                .icon("aperture")
                .live(Duration::from_secs(10), |_| Ok(Visual::new().label(status_label())))
                .on_press(|ctx| {
                    request(&["state"])?;
                    ctx.set_label(status_label());
                    Ok(())
                }),
        )
        .action(
            Action::toggle("module", "Module")
                .icon("layout-grid")
                .inputs(vec![input::select("module", "Module", MODULES).default_value("develop")])
                .state("off", "Module", "layout-grid")
                .state("on", "Current module", "layout-dashboard")
                .initial_with(|ic| link().module().map(|m| on_off(m == str_input(&ic.inputs, "module", "develop")).to_string()))
                .on_press(|ctx| {
                    let m = str_input(&ctx.inputs, "module", "develop");
                    request(&["module", &m]).map(|_| ())
                }),
        )
        .action(
            Action::wheel("module-wheel", "Modules")
                .icon("layout-grid")
                .on_open(|_| Ok(MODULES.iter().map(|(id, l)| json!({"id": id, "label": l, "icon": "layout-grid"})).collect()))
                .on_child_press(|ctx| request(&["module", ctx.child_id.as_deref().unwrap_or("develop")]).map(|_| ())),
        )
        // Photos
        .action(command("next-photo", "Next Photo", "chevron-right", &["photo", "next"]))
        .action(command("previous-photo", "Previous Photo", "chevron-left", &["photo", "previous"]))
        .action(
            Action::toggle("flag", "Flag")
                .icon("flag")
                .inputs(vec![input::select("flag", "Flag", &[("pick", "Pick (again: unflag)"), ("reject", "Reject (again: unflag)"), ("none", "Unflag")]).default_value("pick")])
                .state("none", "Unflagged", "flag")
                .state("pick", "Picked", "circle-check")
                .state("reject", "Rejected", "circle-x")
                .initial_with(|_| Some(flag_state(&link().photo())))
                .on_press(|ctx| {
                    let r = request(&["flag", &str_input(&ctx.inputs, "flag", "pick")])?;
                    ctx.fan_out(r.first().cloned().unwrap_or_else(|| "none".into()));
                    Ok(())
                }),
        )
        .action(
            Action::button("rating", "Rating")
                .icon("star")
                .inputs(vec![input::select(
                    "rating",
                    "Rating",
                    &[("0", "No stars"), ("1", "1 star"), ("2", "2 stars"), ("3", "3 stars"), ("4", "4 stars"), ("5", "5 stars"), ("up", "One star more"), ("down", "One star less")],
                )
                .default_value("5")])
                .live(Duration::from_secs(10), |_| Ok(Visual::new().label(link().photo().map(|p| stars(p.rating)).unwrap_or_else(|| "No photo".into()))))
                .on_press(|ctx| {
                    let r = request(&["rating", &str_input(&ctx.inputs, "rating", "5")])?;
                    let n = r.first().and_then(|v| v.parse().ok()).unwrap_or(0);
                    ctx.fan_out_visual(Visual::new().label(stars(n)));
                    Ok(())
                }),
        )
        .action(
            Action::slider("rating-dial", "Rating (dial)", 0.0, 5.0)
                .icon("star")
                .step(1.0)
                .initial_with(|_| link().photo().map(|p| p.rating.to_string()))
                .on_adjust(|ctx| {
                    let now = link().photo().map(|p| p.rating as f64).unwrap_or(0.0);
                    let to = match (ctx.value, ctx.delta) {
                        (Some(v), _) => v,
                        (None, Some(d)) => now + d.signum(),
                        _ => return Ok(()),
                    }
                    .round()
                    .clamp(0.0, 5.0);
                    let r = request(&["rating", &format!("{to}")])?;
                    ctx.fan_out(r.first().cloned().unwrap_or_default());
                    Ok(())
                }),
        )
        .action(
            Action::toggle("color-label", "Color Label")
                .icon("tag")
                .inputs(vec![input::select("label", "Label", LABELS).default_value("red")])
                .state("off", "Label off", "tag")
                .state("on", "Label on", "tags")
                .initial_with(|ic| link().photo().map(|p| on_off(p.label == str_input(&ic.inputs, "label", "red")).to_string()))
                .on_press(|ctx| {
                    let want = str_input(&ctx.inputs, "label", "red");
                    let r = request(&["label", &want])?;
                    ctx.set_state(on_off(r.first().map(String::as_str) == Some(want.as_str())));
                    Ok(())
                }),
        )
        .action(command("rotate-left", "Rotate Left", "rotate-ccw", &["rotate", "left"]))
        .action(command("rotate-right", "Rotate Right", "rotate-cw", &["rotate", "right"]))
        // Develop
        .action(
            Action::button("copy-settings", "Copy Settings").icon("copy").on_press(|ctx| {
                let r = request(&["copy"])?;
                ctx.set_label(format!("Copied\n{}", r.first().cloned().unwrap_or_default()));
                Ok(())
            }),
        )
        .action(
            Action::button("paste-settings", "Paste Settings").icon("clipboard-paste").on_press(|ctx| {
                let r = link().request(&["paste"], Duration::from_secs(15)).map_err(Error::msg)?;
                let n = r.first().cloned().unwrap_or_default();
                ctx.set_label(if n == "1" { "Pasted".to_string() } else { format!("Pasted on {n}") });
                Ok(())
            }),
        )
        .action(command("reset-all", "Reset All Settings", "rotate-ccw-square", &["resetall"]))
        .action(command("auto-tone", "Auto Tone", "wand-sparkles", &["auto", "tone"]))
        .action(command("auto-white-balance", "Auto White Balance", "pipette", &["auto", "wb"]))
        .action(
            Action::button("develop-view", "Before / After")
                .icon("columns-2")
                .inputs(vec![input::select(
                    "view",
                    "View",
                    &[
                        ("toggle", "Before / After and back"),
                        ("before_after_horiz", "Before / After, left / right"),
                        ("before_after_vert", "Before / After, top / bottom"),
                        ("before", "Before only"),
                        ("loupe", "Loupe (after)"),
                    ],
                )
                .default_value("toggle")])
                .on_press(|ctx| request(&["view", &str_input(&ctx.inputs, "view", "toggle")]).map(|_| ())),
        )
        .action(
            Action::button("develop-tool", "Develop Tool")
                .icon("crop")
                .inputs(vec![input::select("tool", "Tool", &[("crop", "Crop"), ("healing", "Healing"), ("redeye", "Red Eye"), ("masking", "Masking"), ("loupe", "None (loupe)")]).default_value("crop")])
                .on_press(|ctx| request(&["tool", &str_input(&ctx.inputs, "tool", "crop")]).map(|_| ())),
        )
        .action(
            Action::button("show-panel", "Show Develop Panel")
                .icon("panel-right")
                .inputs(vec![input::select("panel", "Panel", params::PANELS).default_value("adjustPanel")])
                .on_press(|ctx| request(&["panel", &str_input(&ctx.inputs, "panel", "adjustPanel")]).map(|_| ())),
        )
        .action(command("undo", "Undo", "undo-2", &["undo"]))
        .action(command("redo", "Redo", "redo-2", &["redo"]))
        .on_ready(|rt| {
            let _ = RT.set(rt.clone());
            // Show what arrived before the host was ready.
            let l = link();
            push(rt, &Update::Status(l.status()));
            if let Some(m) = l.module() {
                push(rt, &Update::Module(m));
            }
            push(rt, &Update::Photo(l.photo()));
            for p in params::PARAMS {
                if let Some(v) = l.value(p.id) {
                    push(rt, &Update::Value(p.id.into(), v));
                }
            }
            Ok(())
        })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fake::FakeLightroom;

    fn linked() -> (FakeLightroom, Arc<Link>, Arc<Mutex<Vec<Update>>>) {
        let lr = FakeLightroom::start("t0ken-0123456789abcdef");
        let l = Link::new(lr.config("t0ken-0123456789abcdef"), vec!["Exposure".into(), "Contrast".into(), "Temperature".into()]);
        let seen = Arc::new(Mutex::new(vec![]));
        let s = seen.clone();
        l.on_update(move |u| s.lock().unwrap().push(u.clone()));
        l.start();
        lr.wait(|| matches!(l.status(), Status::Linked { .. }), "linked");
        (lr, l, seen)
    }

    #[test]
    fn pairs_watches_and_reads_state() {
        let (lr, l, _) = linked();
        assert_eq!(l.status(), Status::Linked { plugin: "1.0.0".into(), lightroom: "15.1".into() });
        lr.wait(|| l.module().is_some(), "module");
        assert_eq!(l.module().as_deref(), Some("develop"));
        assert_eq!(l.photo().unwrap().name, "IMG_0001.CR3");
        lr.wait(|| l.value("Temperature").is_some(), "values");
        assert_eq!(l.value("Temperature"), Some(5500.0));
        assert!(lr.log().iter().any(|c| c.starts_with("watch\tExposure,Contrast,Temperature")), "{:?}", lr.log());
    }

    #[test]
    fn approval_flow_pending_then_paired() {
        let lr = FakeLightroom::start_asking("t0ken-0123456789abcdef");
        let l = Link::new(lr.config("t0ken-0123456789abcdef"), vec![]);
        l.start();
        lr.wait(|| l.status() == Status::Pending, "pending");
        assert!(link::not_linked(&l.status()).contains("Allow"));
        assert!(l.request(&["undo"], TIMEOUT).unwrap_err().contains("Allow"));
        lr.approve();
        lr.wait(|| matches!(l.status(), Status::Linked { .. }), "linked after approval");
    }

    #[test]
    fn wrong_token_is_denied() {
        let lr = FakeLightroom::start("the-right-token-0000");
        let l = Link::new(lr.config("a-wrong-token-11111111"), vec![]);
        l.start();
        lr.wait(|| l.status() == Status::Denied, "denied");
        assert!(l.request(&["undo"], TIMEOUT).unwrap_err().contains("refused"));
    }

    #[test]
    fn dial_ticks_coalesce_while_one_is_in_flight() {
        let (lr, l, seen) = linked();
        lr.set_delay(Duration::from_millis(150));
        for _ in 0..10 {
            l.adjust("Exposure", Change::Ticks(1.0), "", "adjustPanel", "k1").unwrap();
        }
        lr.wait(|| l.value("Exposure") == Some(0.5), "all ten ticks land (default step 0.05)");
        let nudges: Vec<String> = lr.log().into_iter().filter(|c| c.starts_with("nudge")).collect();
        assert!(nudges.len() <= 3, "coalesced into few commands: {nudges:?}");
        assert_eq!(nudges[0], "nudge\tExposure\t1\t\tadjustPanel");
        assert!(seen.lock().unwrap().contains(&Update::Value("Exposure".into(), 0.5)));
        // An absolute drag wins over queued ticks.
        l.adjust("Contrast", Change::Ticks(1.0), "5", "", "k2").unwrap();
        l.adjust("Contrast", Change::Ticks(1.0), "5", "", "k2").unwrap();
        l.adjust("Contrast", Change::Set(40.0), "5", "", "k2").unwrap();
        lr.wait(|| l.value("Contrast") == Some(40.0), "set lands");
    }

    #[test]
    fn errors_reach_the_key_and_requests() {
        let (lr, l, seen) = linked();
        lr.world(|w| w.photo = None);
        l.adjust("Exposure", Change::Ticks(1.0), "", "", "k9").unwrap();
        lr.wait(|| seen.lock().unwrap().iter().any(|u| matches!(u, Update::Failed { instance, .. } if instance == "k9")), "failed update");
        assert_eq!(l.request(&["flag", "pick"], TIMEOUT).unwrap_err(), "Select a photo in Lightroom first");
    }

    #[test]
    fn lightroom_quitting_goes_offline_and_relinks() {
        let (lr, l, _) = linked();
        lr.drop_connection();
        lr.wait(|| l.status() == Status::Offline, "offline");
        assert!(l.request(&["undo"], TIMEOUT).unwrap_err().contains("isn't linked"));
        lr.wait(|| matches!(l.status(), Status::Linked { .. }), "linked again");
    }

    #[test]
    fn formatting() {
        assert_eq!(fmt(0.15000000000000002), "0.15");
        assert_eq!(fmt(5500.0), "5500");
        assert_eq!(fmt(-12.0), "-12");
        assert_eq!(stars(3), "★★★☆☆");
        assert_eq!(param_of(&json!({})), "Exposure");
        assert_eq!(param_of(&json!({"param": "Tint"})), "Tint");
        for (_, _, p, _, _) in BASIC {
            assert!(params::find(p).is_some(), "{p}");
        }
    }
}
