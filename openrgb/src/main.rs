//! OpenRGB plugin for KnobDeck: profiles, colour and brightness of the RGB devices OpenRGB
//! controls, through OpenRGB's SDK server (TCP 6742).

mod sdk;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, load_env, Action, Ctx, Error, Plugin, Result, Value, Visual};
use sdk::{Client, Controller, Rgb};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

const ALL: &str = "__all__";
const POLL: Duration = Duration::from_secs(3);

fn host() -> String {
    env_or("OPENRGB_HOST", "127.0.0.1")
}

fn port() -> u16 {
    env_or("OPENRGB_PORT", "").trim().parse().unwrap_or(sdk::DEFAULT_PORT)
}

static CLIENT: Mutex<Option<Client>> = Mutex::new(None);

/// Run `f` on the shared connection; a dropped connection is opened again once.
fn with<T>(mut f: impl FnMut(&mut Client) -> std::result::Result<T, String>) -> Result<T> {
    let mut slot = CLIENT.lock().unwrap();
    if let Some(c) = slot.as_mut() {
        match f(c) {
            Ok(v) => return Ok(v),
            Err(_) => *slot = None, // stale (OpenRGB restarted): reconnect below
        }
    }
    let mut c = Client::connect(&host(), port()).map_err(Error::msg)?;
    let out = f(&mut c).map_err(Error::msg);
    *slot = Some(c);
    out
}

/// A device input value: `<index>:<name>`. Indexes move when devices come and go, so the name
/// decides when they disagree.
fn device_value(i: usize, c: &Controller) -> String {
    format!("{i}:{}", c.name)
}

fn resolve(pick: &str, list: &[Controller]) -> Result<Vec<usize>> {
    if list.is_empty() {
        return Err(Error::msg("OpenRGB has no devices"));
    }
    if pick.is_empty() || pick == ALL {
        return Ok((0..list.len()).collect());
    }
    let (idx, name) = pick.split_once(':').unwrap_or(("", pick));
    if let Ok(i) = idx.parse::<usize>() {
        if list.get(i).is_some_and(|c| c.name == name) {
            return Ok(vec![i]);
        }
    }
    list.iter().position(|c| c.name == name).map(|i| vec![i]).ok_or_else(|| Error::msg(format!("OpenRGB has no device \"{name}\" now")))
}

/// Brightness the plugin applied per device (by device value), with the colours it scales, for
/// devices whose mode has no brightness of its own.
struct Dimmed {
    base: Vec<Rgb>,
    pct: f64,
}
static DIMMED: Mutex<Option<HashMap<String, Dimmed>>> = Mutex::new(None);

fn dimmed<T>(f: impl FnOnce(&mut HashMap<String, Dimmed>) -> T) -> T {
    f(DIMMED.lock().unwrap().get_or_insert_with(HashMap::new))
}

/// The colours a device shows now: its LEDs in a per-LED mode, the mode's colours otherwise.
fn current_colors(c: &Controller) -> Vec<Rgb> {
    match c.mode() {
        Some(m) if m.color_mode == sdk::COLOR_MODE_MODE_SPECIFIC => m.colors.clone(),
        _ => c.colors.clone(),
    }
}

/// Show `colors` (one per LED, or a single colour for all) on device `i`.
fn paint(cl: &mut Client, i: usize, colors: &[Rgb]) -> std::result::Result<(), String> {
    let mut c = cl.controller(i as u32)?;
    if c.mode().map(|m| m.color_mode) != Some(sdk::COLOR_MODE_PER_LED) && c.mode().map(|m| m.color_mode) != Some(sdk::COLOR_MODE_MODE_SPECIFIC) {
        // Effect modes take no colour: switch to the device's Direct / Custom / Static mode.
        cl.set_custom_mode(i as u32)?;
        c = cl.controller(i as u32)?;
    }
    let pick = |n: usize| -> Vec<Rgb> { (0..n).map(|k| colors.get(k).or(colors.last()).copied().unwrap_or([0, 0, 0])).collect() };
    let Some(m) = c.mode().cloned() else { return Err(format!("{} has no modes", c.name)) };
    match m.color_mode {
        sdk::COLOR_MODE_PER_LED if c.leds > 0 => cl.update_leds(i as u32, &pick(c.leds)),
        sdk::COLOR_MODE_MODE_SPECIFIC => {
            let n = m.colors.len().max(m.colors_min as usize).max(1);
            let mut m = m;
            m.colors = pick(n);
            cl.update_mode(i as u32, c.active_mode, &m)
        }
        _ => Err(format!("{} can't take a colour in its \"{}\" mode", c.name, m.name)),
    }
}

/// Mode brightness range, if the active mode has one.
fn mode_brightness(c: &Controller) -> Option<(u32, u32, u32)> {
    let m = c.mode()?;
    (m.flags & sdk::MODE_FLAG_HAS_BRIGHTNESS != 0 && m.brightness_max != m.brightness_min).then_some((m.brightness_min, m.brightness_max, m.brightness))
}

fn pct_of(min: u32, max: u32, v: u32) -> f64 {
    let (lo, hi) = (min.min(max) as f64, min.max(max) as f64);
    let p = (v as f64 - lo) / (hi - lo) * 100.0;
    if max < min {
        100.0 - p
    } else {
        p
    }
    .clamp(0.0, 100.0)
}

fn value_of(min: u32, max: u32, pct: f64) -> u32 {
    (min as f64 + (max as f64 - min as f64) * pct / 100.0).round() as u32
}

/// The brightness a device is at, in %.
fn brightness_of(i: usize, c: &Controller) -> f64 {
    if let Some((min, max, v)) = mode_brightness(c) {
        return pct_of(min, max, v);
    }
    dimmed(|d| d.get(&device_value(i, c)).map(|x| x.pct)).unwrap_or(100.0)
}

fn set_brightness(cl: &mut Client, i: usize, pct: f64) -> std::result::Result<(), String> {
    let c = cl.controller(i as u32)?;
    if let Some((min, max, _)) = mode_brightness(&c) {
        let mut m = c.mode().cloned().unwrap();
        m.brightness = value_of(min, max, pct);
        return cl.update_mode(i as u32, c.active_mode, &m);
    }
    // No brightness in this mode: scale the colours the device had at 100 %.
    let key = device_value(i, &c);
    let now = current_colors(&c);
    let base = dimmed(|d| match d.get(&key) {
        Some(x) if now == x.base.iter().map(|b| sdk::scale(*b, x.pct)).collect::<Vec<_>>() => x.base.clone(),
        // First time, or the colours changed since (OpenRGB, a profile): they are the 100 % base.
        _ => now.clone(),
    });
    if base.iter().all(|c| *c == [0, 0, 0]) {
        return Err(format!("{} is black: set a colour first", c.name));
    }
    let scaled: Vec<Rgb> = base.iter().map(|b| sdk::scale(*b, pct)).collect();
    paint(cl, i, &scaled)?;
    dimmed(|d| d.insert(key, Dimmed { base, pct }));
    Ok(())
}

fn device_input() -> Value {
    input::query_select("device", "Device", "devices", &[]).with("default", ALL)
}

fn controllers() -> Result<Vec<Controller>> {
    with(|c| c.controllers())
}

fn targets(inputs: &Value) -> Result<Vec<usize>> {
    resolve(inputs["device"].as_str().unwrap_or(ALL), &controllers()?)
}

fn load_profile(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(Error::msg("Choose a profile"));
    }
    with(|c| c.load_profile(name))?;
    dimmed(|d| d.clear());
    Ok(())
}

fn step(ctx: &Ctx) -> f64 {
    ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(5.0)
}

fn main() {
    load_env();
    Plugin::new(&format!("openrgb-{}", std::env::consts::OS), "OpenRGB")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("RGB lighting through OpenRGB: load profiles, set a colour on every device or one, and a brightness dial.")
        .icon("app-icon.svg")
        .setup("openrgb-setup", None)
        .setup_inputs(vec![
            input::text("host", "Host").placeholder("127.0.0.1").default_value("127.0.0.1").with("help", "Where OpenRGB runs. Turn on its SDK server: SDK Server tab ▸ Start Server (OpenRGB's settings can start it whenever OpenRGB starts, or run OpenRGB with --server)."),
            input::text("port", "Port (optional)").placeholder("6742"),
        ])
        .query("devices", |q| {
            let list = controllers().or_else(|e| q.fail(e.message().to_string()))?;
            let mut opts = vec![json!({"value": ALL, "label": "All devices"})];
            opts.extend(list.iter().enumerate().map(|(i, c)| json!({"value": device_value(i, c), "label": c.name})));
            Ok(Value::Array(opts))
        })
        .query("profiles", |q| {
            let list = with(|c| c.profiles()).or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(list.into_iter().map(|p| json!({"value": p, "label": p})).collect()))
        })
        .action(Action::button("profile", "Load Profile").icon("bookmark").inputs(vec![input::query_select("profile", "Profile", "profiles", &[])]).on_press(|ctx| load_profile(&ctx.input_str("profile"))))
        .action(
            Action::wheel("profiles-wheel", "Profiles Wheel")
                .icon("bookmark")
                .on_open(|o| {
                    let list = with(|c| c.profiles()).or_else(|e| o.fail(e.message().to_string()))?;
                    if list.is_empty() {
                        return o.fail("OpenRGB has no saved profiles (Save Profile in OpenRGB)");
                    }
                    Ok(list.into_iter().take(16).map(|p| json!({"id": p, "label": p, "icon": "bookmark"})).collect())
                })
                .on_child_press(|ctx| match ctx.child_id.as_deref() {
                    Some(p) => load_profile(p),
                    None => Ok(()),
                }),
        )
        .action(
            Action::button("set-color", "Set Color")
                .icon("paintbrush")
                .inputs(vec![device_input(), input::text("color", "Color (#RRGGBB)").placeholder("#FF8800").default_value("#FF8800").with("help", "#000000 turns the LEDs off.")])
                .on_press(|ctx| {
                    let Some(rgb) = sdk::hex(&ctx.input_str("color")) else {
                        return ctx.fail("Color must look like #FF8800");
                    };
                    let ids = targets(&ctx.inputs)?;
                    let mut failed = vec![];
                    with(|cl| {
                        failed.clear();
                        for &i in &ids {
                            match paint(cl, i, &[rgb]) {
                                Ok(()) => {
                                    if let Ok(c) = cl.controller(i as u32) {
                                        dimmed(|d| d.remove(&device_value(i, &c)));
                                    }
                                }
                                Err(e) => failed.push(e),
                            }
                        }
                        // One device that can't take a colour (no Direct / Static mode) doesn't stop the rest.
                        if failed.len() == ids.len() {
                            return Err(failed.join("; "));
                        }
                        Ok(())
                    })?;
                    for e in failed {
                        ctx.log(e);
                    }
                    Ok(())
                }),
        )
        .action(
            Action::slider("brightness", "Brightness", 0.0, 100.0)
                .icon("sun")
                .unit("%")
                .inputs(vec![device_input(), input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(POLL, |ic| {
                    let list = controllers()?;
                    let i = resolve(ic.input_str("device").as_str(), &list)?[0];
                    Ok(Visual::new().state(brightness_of(i, &list[i]).round()))
                })
                .on_adjust(|ctx| {
                    let list = controllers()?;
                    let ids = resolve(ctx.input_str("device").as_str(), &list)?;
                    let target = match ctx.value {
                        Some(v) => v,
                        None => brightness_of(ids[0], &list[ids[0]]) + step(ctx) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(0.0, 100.0);
                    let mut errors = vec![];
                    with(|cl| {
                        errors.clear();
                        for &i in &ids {
                            if let Err(e) = set_brightness(cl, i, target) {
                                errors.push(e);
                            }
                        }
                        if errors.len() == ids.len() {
                            return Err(errors.join("; "));
                        }
                        Ok(())
                    })?;
                    for e in errors {
                        ctx.log(e);
                    }
                    ctx.fan_out_by(&["device"], target);
                    Ok(())
                }),
        )
        .run()
}

#[cfg(test)]
mod mock;
#[cfg(test)]
mod mock_server;
