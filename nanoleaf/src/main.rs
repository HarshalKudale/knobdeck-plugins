//! Nanoleaf plugin for KnobDeck: power, brightness, colour temperature, colour and effects for
//! Nanoleaf panels and lightstrips, over the device's local OpenAPI.

#[allow(dead_code)] // discovery and pairing are for setup
mod leaf;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use leaf::Device;
use std::time::Duration;

const POLL: Duration = Duration::from_secs(3);
const ALL: &str = "__all__";

/// Paired devices (address list from setup, token per device from the credential store).
fn paired() -> Vec<Device> {
    env_or("NANOLEAF_DEVICES", "")
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .filter_map(|a| {
            let token = knobdeck_sdk::secrets::env_or_stored(&leaf::token_key(a));
            (!token.is_empty()).then(|| Device { addr: a.to_string(), token })
        })
        .collect()
}

fn targets(inputs: &Value) -> Result<Vec<Device>> {
    let all = paired();
    if all.is_empty() {
        knobdeck_sdk::mark_setup_required("Pair a Nanoleaf device in setup");
        return Err(Error::msg("Run setup to pair a Nanoleaf device"));
    }
    let pick = inputs["device"].as_str().unwrap_or(ALL);
    if pick.is_empty() || pick == ALL {
        return Ok(all);
    }
    let one: Vec<Device> = all.into_iter().filter(|d| d.addr == pick).collect();
    if one.is_empty() {
        return Err(Error::msg(format!("{pick} isn't paired: run setup again")));
    }
    Ok(one)
}

fn first(inputs: &Value) -> Result<Value> {
    targets(inputs)?[0].info()
}

fn set_all(inputs: &Value, state: Value) -> Result<()> {
    for d in targets(inputs)? {
        d.set(state.clone())?;
    }
    Ok(())
}

fn device_input() -> Value {
    input::query_select("device", "Device", "devices", &[]).with("default", ALL)
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

/// Effects of the first targeted device.
fn effects(inputs: &Value) -> Result<Vec<String>> {
    targets(inputs)?[0].effects()
}

fn main() {
    knobdeck_sdk::load_env();
    Plugin::new(&format!("nanoleaf-{}", std::env::consts::OS), "Nanoleaf")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Nanoleaf Shapes, Elements, Canvas, Lines and lightstrips: power, brightness, color temperature, color and effects.")
        .icon("app-icon.svg")
        .setup("nanoleaf-setup", None)
        .setup_inputs(vec![input::text("addresses", "Device addresses (optional)").placeholder("192.168.1.30").with(
            "help",
            "Devices on this network are found automatically. When setup starts, hold each device's power button for 5–7 seconds until its lights flash: that lets KnobDeck pair with it.",
        )])
        .query("devices", |_| {
            let mut opts = vec![json!({"value": ALL, "label": "All devices"})];
            for d in paired() {
                let name = d.info().ok().and_then(|i| i["name"].as_str().map(String::from)).unwrap_or_else(|| d.addr.clone());
                opts.push(json!({"value": d.addr, "label": format!("{name} ({})", d.addr)}));
            }
            Ok(Value::Array(opts))
        })
        .query("effects", |q| {
            let list = effects(&q.inputs).or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(list.into_iter().map(|e| json!({"value": e, "label": e})).collect()))
        })
        .action(
            Action::toggle("power", "Power")
                .icon("lightbulb")
                .inputs(vec![device_input()])
                .state("off", "Off", "lightbulb-off")
                .state("on", "On", "lightbulb")
                .live(POLL, |ic| Ok(Visual::new().state(on_label(first(&ic.inputs)?["state"]["on"]["value"] == true))))
                .on_press(|ctx| {
                    let on = first(&ctx.inputs)?["state"]["on"]["value"] != true;
                    set_all(&ctx.inputs, json!({"on": {"value": on}}))?;
                    ctx.fan_out_by(&["device"], on_label(on));
                    Ok(())
                }),
        )
        .action(
            Action::slider("brightness", "Brightness", 0.0, 100.0)
                .icon("sun")
                .unit("%")
                .inputs(vec![device_input(), input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(POLL, |ic| Ok(Visual::new().state(first(&ic.inputs)?["state"]["brightness"]["value"].as_i64().unwrap_or(0))))
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => first(&ctx.inputs)?["state"]["brightness"]["value"].as_f64().unwrap_or(0.0) + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(0.0, 100.0);
                    set_all(&ctx.inputs, json!({"on": {"value": target > 0.0}, "brightness": {"value": target as i64, "duration": 0}}))?;
                    ctx.fan_out_by(&["device"], target);
                    Ok(())
                }),
        )
        .action(
            Action::slider("temperature", "Color Temperature", 1200.0, 6500.0)
                .icon("thermometer")
                .unit("K")
                .step(100.0)
                .inputs(vec![device_input(), input::text("step", "Step (K)").placeholder("100").default_value("100")])
                .live(POLL, |ic| {
                    let s = first(&ic.inputs)?;
                    Ok(Visual::new().state(s["state"]["ct"]["value"].as_i64().unwrap_or(4000)))
                })
                .on_adjust(|ctx| {
                    let s = first(&ctx.inputs)?;
                    let (lo, hi) = (s["state"]["ct"]["min"].as_f64().unwrap_or(1200.0), s["state"]["ct"]["max"].as_f64().unwrap_or(6500.0));
                    let target = match ctx.value {
                        Some(v) => v,
                        None => s["state"]["ct"]["value"].as_f64().unwrap_or(4000.0) + step(ctx, 100.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(lo, hi);
                    set_all(&ctx.inputs, json!({"ct": {"value": target as i64}}))?;
                    ctx.fan_out_by(&["device"], target);
                    Ok(())
                }),
        )
        .action(Action::button("set-color", "Set Color").icon("palette").inputs(vec![device_input(), input::text("color", "Color (#RRGGBB)").placeholder("#FF8800").default_value("#FF8800")]).on_press(|ctx| {
            let Some((h, s, b)) = leaf::hex_to_hsb(&ctx.input_str("color")) else {
                return ctx.fail("Color must look like #FF8800");
            };
            set_all(&ctx.inputs, json!({"on": {"value": true}, "hue": {"value": h as i64}, "sat": {"value": s as i64}, "brightness": {"value": b.max(1.0) as i64}}))
        }))
        .action(
            Action::button("effect", "Play Effect")
                .icon("sparkles")
                .inputs(vec![device_input(), input::query_select("effect", "Effect", "effects", &["device"])])
                .live(POLL, |ic| {
                    let current = first(&ic.inputs)?["effects"]["select"].as_str().unwrap_or("").to_string();
                    Ok(Visual::new().icon(if current == ic.input_str("effect") { "sparkles" } else { "sparkle" }))
                })
                .on_press(|ctx| {
                    let name = ctx.input_str("effect");
                    if name.is_empty() {
                        return ctx.fail("Choose an effect");
                    }
                    for d in targets(&ctx.inputs)? {
                        d.select_effect(&name)?;
                    }
                    Ok(())
                }),
        )
        .action(
            Action::wheel("effects-wheel", "Effects Wheel")
                .icon("sparkles")
                .inputs(vec![device_input()])
                .on_open(|o| {
                    let list = effects(&o.inputs).or_else(|e| o.fail(e.message().to_string()))?;
                    Ok(list.into_iter().take(16).map(|e| json!({"id": e, "label": e, "icon": "sparkles"})).collect())
                })
                .on_child_press(|ctx| {
                    let Some(name) = ctx.child_id.clone() else { return Ok(()) };
                    for d in targets(&ctx.inputs)? {
                        d.select_effect(&name)?;
                    }
                    Ok(())
                }),
        )
        .run()
}
