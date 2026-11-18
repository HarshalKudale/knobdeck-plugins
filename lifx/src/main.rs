//! LIFX plugin for KnobDeck: LIFX bulbs, strips and panels over the LIFX LAN protocol (UDP).
//! Lights are found by broadcast; setup also accepts addresses by hand. No account or cloud.

mod lan;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, load_env, Action, Ctx, Error, Plugin, Result, Value, Visual};
use lan::{Hsbk, Light, LightState};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const ALL: &str = "__all__";
const POLL: Duration = Duration::from_secs(3);
/// The white range LIFX lights accept (a light clamps to its own range).
const K_MIN: f64 = 1500.0;
const K_MAX: f64 = 9000.0;

/// Known lights with their labels. Filled by discovery plus setup's addresses.
static LIGHTS: Mutex<Vec<(Light, String)>> = Mutex::new(Vec::new());

fn configured() -> Vec<String> {
    env_or("LIFX_ADDRESSES", "").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Re-scan, once at a time: callers that wait for a scan in progress use its result.
fn refresh() -> Vec<(Light, String)> {
    static SCAN: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = SCAN.lock().unwrap();
    if last.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
        return LIGHTS.lock().unwrap().clone();
    }
    let found: Vec<(Light, String)> = lan::discover(&configured(), Duration::from_millis(1200))
        .into_iter()
        .map(|l| {
            let label = lan::get(&l).map(|s| s.label).ok().filter(|s| !s.is_empty()).unwrap_or_else(|| l.addr.ip().to_string());
            (l, label)
        })
        .collect();
    *LIGHTS.lock().unwrap() = found.clone();
    *last = Some(Instant::now());
    found
}

fn known(rescan: bool) -> Vec<Light> {
    let cached = LIGHTS.lock().unwrap().clone();
    if rescan || cached.is_empty() { refresh() } else { cached }.into_iter().map(|(l, _)| l).collect()
}

fn targets(inputs: &Value) -> Result<Vec<Light>> {
    let pick = inputs["light"].as_str().unwrap_or(ALL);
    let mut all = known(false);
    if !(pick.is_empty() || pick == ALL) {
        if !all.iter().any(|l| l.id() == pick) {
            all = known(true); // it may have a new address
        }
        all.retain(|l| l.id() == pick);
    }
    if all.is_empty() {
        return Err(Error::msg(if pick.is_empty() || pick == ALL {
            "No LIFX light found: make sure it is on and on the same network, or add its address in setup".to_string()
        } else {
            format!("LIFX light {pick} not found (is it on?)")
        }));
    }
    Ok(all)
}

fn first(inputs: &Value) -> Result<LightState> {
    lan::get(&targets(inputs)?[0])
}

/// Change each targeted light's color from its own current one (so a hue dial keeps each light's
/// brightness), turning it on.
fn change(inputs: &Value, f: impl Fn(&mut Hsbk)) -> Result<()> {
    for l in targets(inputs)? {
        let s = lan::get(&l)?;
        let mut c = s.color;
        f(&mut c);
        lan::set_color(&l, &c, 0)?;
        if !s.on {
            lan::set_power(&l, true, 0)?;
        }
    }
    Ok(())
}

fn light_input() -> Value {
    input::query_select("light", "Light", "lights", &[]).with("default", ALL)
}

fn step(ctx: &Ctx, default: f64) -> f64 {
    ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(default)
}

fn on_label(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// A white light has no saturation; its key shows the white temperature.
fn is_white(c: &Hsbk) -> bool {
    c.saturation == 0
}

fn main() {
    load_env();
    if std::env::var("KNOBDECK_CODEGEN").as_deref() != Ok("1") {
        std::thread::spawn(refresh); // warm the list so the first press doesn't wait for discovery
    }
    Plugin::new(&format!("lifx-{}", std::env::consts::OS), "LIFX")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("LIFX bulbs, strips and panels on your network: power, brightness, white temperature, hue, saturation and color. No account needed.")
        .icon("lamp-floor")
        .setup("lifx-setup", None)
        .setup_inputs(vec![input::text("addresses", "Light addresses (optional)").placeholder("192.168.1.60").with("help", "Lights on this network are found automatically. Add addresses only for lights that are not found (for example on another subnet).")])
        .query("lights", |_| {
            let mut opts = vec![json!({"value": ALL, "label": "All lights"})];
            opts.extend(refresh().into_iter().map(|(l, n)| json!({"value": l.id(), "label": format!("{n} ({})", l.addr.ip())})));
            Ok(Value::Array(opts))
        })
        .action(
            Action::toggle("power", "Power")
                .icon("lightbulb")
                .inputs(vec![light_input()])
                .state("off", "Off", "lightbulb-off")
                .state("on", "On", "lightbulb")
                .live(POLL, |ic| Ok(Visual::new().state(on_label(first(&ic.inputs)?.on))))
                .on_press(|ctx| {
                    let on = !first(&ctx.inputs)?.on;
                    for l in targets(&ctx.inputs)? {
                        lan::set_power(&l, on, 0)?;
                    }
                    ctx.fan_out_by(&["light"], on_label(on));
                    Ok(())
                }),
        )
        .action(
            Action::slider("brightness", "Brightness", 1.0, 100.0)
                .icon("sun")
                .unit("%")
                .inputs(vec![light_input(), input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(POLL, |ic| Ok(Visual::new().state(first(&ic.inputs)?.color.bri_pct())))
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => first(&ctx.inputs)?.color.bri_pct() + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(1.0, 100.0);
                    change(&ctx.inputs, |c| c.brightness = lan::pct_to_u16(target))?;
                    ctx.fan_out_by(&["light"], target);
                    Ok(())
                }),
        )
        .action(Action::button("set-brightness", "Set Brightness").icon("sun").inputs(vec![light_input(), input::text("value", "Brightness (%)").placeholder("50").default_value("50")]).on_press(|ctx| {
            let Some(v) = ctx.input_f64("value").filter(|v| (1.0..=100.0).contains(v)) else {
                return ctx.fail("Brightness must be 1–100");
            };
            change(&ctx.inputs, |c| c.brightness = lan::pct_to_u16(v))
        }))
        .action(
            Action::slider("temperature", "Color Temperature", K_MIN, K_MAX)
                .icon("thermometer")
                .unit("K")
                .step(100.0)
                .inputs(vec![light_input(), input::text("step", "Step (K)").placeholder("100").default_value("100")])
                .live(POLL, |ic| {
                    let c = first(&ic.inputs)?.color;
                    Ok(if is_white(&c) { Visual::new().state(c.kelvin) } else { Visual::new().label("Color") })
                })
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => first(&ctx.inputs)?.color.kelvin as f64 + step(ctx, 100.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(K_MIN, K_MAX);
                    change(&ctx.inputs, |c| {
                        c.saturation = 0;
                        c.kelvin = target as u16;
                    })?;
                    ctx.fan_out_by(&["light"], target);
                    Ok(())
                }),
        )
        .action(Action::button("set-temperature", "Set White").icon("thermometer").inputs(vec![light_input(), input::text("value", "Temperature (K, 1500–9000)").placeholder("4000").default_value("4000")]).on_press(|ctx| {
            let Some(k) = ctx.input_f64("value").filter(|k| (K_MIN..=K_MAX).contains(k)) else {
                return ctx.fail("Temperature must be 1500–9000 K");
            };
            change(&ctx.inputs, |c| {
                c.saturation = 0;
                c.kelvin = k.round() as u16;
            })
        }))
        .action(
            Action::slider("hue", "Hue", 0.0, 359.0)
                .icon("palette")
                .unit("°")
                .inputs(vec![light_input(), input::text("step", "Step (°)").placeholder("10").default_value("10")])
                .live(POLL, |ic| {
                    let c = first(&ic.inputs)?.color;
                    Ok(if is_white(&c) { Visual::new().label("White") } else { Visual::new().state(c.hue_deg()) })
                })
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => first(&ctx.inputs)?.color.hue_deg() + step(ctx, 10.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .rem_euclid(360.0); // the hue wraps around
                    change(&ctx.inputs, |c| {
                        c.hue = lan::deg_to_u16(target);
                        if c.saturation == 0 {
                            c.saturation = 65535; // from white, turning the hue means full color
                        }
                    })?;
                    ctx.fan_out_by(&["light"], target);
                    Ok(())
                }),
        )
        .action(
            Action::slider("saturation", "Saturation", 0.0, 100.0)
                .icon("droplet")
                .unit("%")
                .inputs(vec![light_input(), input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(POLL, |ic| Ok(Visual::new().state(first(&ic.inputs)?.color.sat_pct())))
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => first(&ctx.inputs)?.color.sat_pct() + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(0.0, 100.0);
                    change(&ctx.inputs, |c| c.saturation = lan::pct_to_u16(target))?;
                    ctx.fan_out_by(&["light"], target);
                    Ok(())
                }),
        )
        .action(
            Action::button("set-color", "Set Color")
                .icon("paintbrush")
                .inputs(vec![
                    light_input(),
                    input::text("color", "Color (#RRGGBB)").placeholder("#FF8800").default_value("#FF8800"),
                    input::text("duration", "Fade (seconds)").placeholder("0").default_value("0"),
                ])
                .on_press(|ctx| {
                    let Some(want) = lan::hex_to_hsbk(&ctx.input_str("color"), 3500) else {
                        return ctx.fail("Color must look like #FF8800");
                    };
                    if want.brightness == 0 {
                        return ctx.fail("Black would only dim the light: use Power to turn it off");
                    }
                    let ms = (ctx.input_f64("duration").unwrap_or(0.0).clamp(0.0, 3600.0) * 1000.0) as u32;
                    for l in targets(&ctx.inputs)? {
                        let s = lan::get(&l)?;
                        lan::set_color(&l, &Hsbk { kelvin: s.color.kelvin, ..want }, ms)?;
                        if !s.on {
                            lan::set_power(&l, true, 0)?;
                        }
                    }
                    Ok(())
                }),
        )
        .run()
}
