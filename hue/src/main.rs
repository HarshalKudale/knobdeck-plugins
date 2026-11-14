//! Philips Hue plugin for KnobDeck — lights, rooms and zones on a Hue Bridge: power, brightness,
//! colour temperature, colour and scenes, over the bridge's local API.

mod hue;

use hue::{Bridge, Target};
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, load_env, Action, Error, Plugin, Result, Value, Visual};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(3);

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

fn bridge() -> Result<&'static Bridge> {
    if let Some(b) = BRIDGE.get() {
        return Ok(b);
    }
    let (addr, user) = (env_or("HUE_BRIDGE", ""), knobdeck_sdk::secrets::env_or_stored("HUE_USER"));
    if addr.is_empty() || user.is_empty() {
        return Err(Error::msg("Run setup to pair with your Hue Bridge"));
    }
    Ok(BRIDGE.get_or_init(|| Bridge { addr: Mutex::new(addr), user, id: env_or("HUE_BRIDGE_ID", "") }))
}

/// An unpaired bridge stops the plugin and asks for setup; other errors show on the key.
fn guard<T>(r: Result<T>) -> Result<T> {
    if let Err(e) = &r {
        if e.message() == hue::UNAUTHORIZED {
            knobdeck_sdk::mark_setup_required(e.message());
        }
    }
    r
}

fn target_of(inputs: &Value) -> Result<Target> {
    let raw = inputs["target"].as_str().unwrap_or("");
    Target::parse(raw).ok_or_else(|| Error::msg("Choose a light, room or zone"))
}

fn target_input() -> Value {
    input::query_select("target", "Light, room or zone", "targets", &[]).with("default", "group:0")
}

fn read(inputs: &Value) -> Result<Value> {
    let b = guard(bridge())?;
    guard(hue::state(b, &target_of(inputs)?))
}

fn write(inputs: &Value, body: Value) -> Result<()> {
    let b = guard(bridge())?;
    guard(hue::set(b, &target_of(inputs)?, &body))
}

fn named(map: &Value) -> Vec<(String, Value)> {
    let mut v: Vec<(String, Value)> = map.as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
    v.sort_by_key(|(_, x)| x["name"].as_str().unwrap_or("").to_lowercase());
    v
}

/// Scenes of one room/zone (or every scene), newest name order, as (id, name, group).
fn scenes(b: &Bridge, group: Option<&str>) -> Result<Vec<(String, String, String)>> {
    Ok(named(&hue::cached(b, "scenes")?)
        .into_iter()
        .filter(|(_, s)| s["recycle"] != true) // scenes apps create for themselves
        .map(|(id, s)| (id, s["name"].as_str().unwrap_or("").to_string(), s["group"].as_str().unwrap_or("").to_string()))
        .filter(|(_, _, g)| group.is_none_or(|want| want == "0" || want == g))
        .collect())
}

fn recall_scene(b: &Bridge, scene: &str) -> Result<()> {
    // A room/zone scene is recalled on its group; a light scene on group 0 (every light).
    let group = hue::cached(b, "scenes")?[scene]["group"].as_str().unwrap_or("0").to_string();
    b.put(&format!("groups/{group}/action"), &json!({"scene": scene}))?;
    hue::invalidate();
    Ok(())
}

fn main() {
    load_env();
    knobdeck_sdk::secrets::migrate_env(&["HUE_USER"]);
    Plugin::new(&format!("hue-{}", std::env::consts::OS), "Philips Hue")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Hue lights, rooms and zones: power, brightness, color temperature, color and scenes.")
        .icon("lamp-ceiling")
        .setup("hue-setup", None)
        .setup_inputs(vec![input::text("bridge", "Bridge address (optional)")
            .placeholder("192.168.1.30")
            .with("help", "Press the round link button on your Hue Bridge, then run setup within 30 seconds. The bridge is found automatically; type its address only if it is not.")])
        .query("targets", |_| {
            let b = guard(bridge())?;
            let mut opts = vec![json!({"value": "group:0", "label": "All lights"})];
            for (id, g) in named(&guard(hue::cached(b, "groups"))?) {
                let kind = g["type"].as_str().unwrap_or("");
                if matches!(kind, "Room" | "Zone" | "LightGroup") {
                    opts.push(json!({"value": format!("group:{id}"), "label": format!("{} · {}", g["name"].as_str().unwrap_or(""), if kind == "LightGroup" { "Group" } else { kind })}));
                }
            }
            for (id, l) in named(&guard(hue::cached(b, "lights"))?) {
                opts.push(json!({"value": format!("light:{id}"), "label": format!("{} · Light", l["name"].as_str().unwrap_or(""))}));
            }
            Ok(Value::Array(opts))
        })
        .query("rooms", |_| {
            let b = guard(bridge())?;
            let mut opts = vec![json!({"value": "0", "label": "All scenes"})];
            for (id, g) in named(&guard(hue::cached(b, "groups"))?) {
                if matches!(g["type"].as_str(), Some("Room" | "Zone")) {
                    opts.push(json!({"value": id, "label": g["name"]}));
                }
            }
            Ok(Value::Array(opts))
        })
        .query("scenes", |_| {
            let b = guard(bridge())?;
            let groups = guard(hue::cached(b, "groups"))?;
            Ok(Value::Array(
                guard(scenes(b, None))?
                    .into_iter()
                    .map(|(id, name, g)| {
                        let room = groups[&g]["name"].as_str().unwrap_or("");
                        json!({"value": id, "label": if room.is_empty() { name } else { format!("{name} · {room}") }})
                    })
                    .collect(),
            ))
        })
        .action(
            Action::toggle("power", "Power")
                .icon("lightbulb")
                .inputs(vec![target_input()])
                .state("off", "Off", "lightbulb-off")
                .state("on", "On", "lightbulb")
                .live(POLL, |ic| Ok(Visual::new().state(if read(&ic.inputs)?["on"] == true { "on" } else { "off" })))
                .on_press(|ctx| {
                    let on = read(&ctx.inputs)?["on"] != true;
                    write(&ctx.inputs, json!({"on": on}))?;
                    ctx.fan_out_by(&["target"], if on { "on" } else { "off" });
                    Ok(())
                }),
        )
        .action(
            Action::slider("brightness", "Brightness", 0.0, 100.0)
                .icon("sun")
                .unit("%")
                .inputs(vec![target_input(), input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(POLL, |ic| {
                    let s = read(&ic.inputs)?;
                    Ok(Visual::new().state(if s["on"] == true { hue::bri_to_pct(s["bri"].as_f64().unwrap_or(254.0)) } else { 0.0 }))
                })
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => {
                            let s = read(&ctx.inputs)?;
                            let now = if s["on"] == true { hue::bri_to_pct(s["bri"].as_f64().unwrap_or(254.0)) } else { 0.0 };
                            now + ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(5.0) * ctx.delta.unwrap_or(0.0)
                        }
                    }
                    .round()
                    .clamp(0.0, 100.0);
                    write(&ctx.inputs, if target == 0.0 { json!({"on": false}) } else { json!({"on": true, "bri": hue::pct_to_bri(target)}) })?;
                    ctx.fan_out_by(&["target"], target);
                    Ok(())
                }),
        )
        .action(Action::button("set-brightness", "Set Brightness").icon("sun").inputs(vec![target_input(), input::text("value", "Brightness (%)").placeholder("50").default_value("50")]).on_press(|ctx| {
            let Some(v) = ctx.input_f64("value").filter(|v| (0.0..=100.0).contains(v)) else {
                return ctx.fail("Brightness must be 0–100");
            };
            write(&ctx.inputs, if v == 0.0 { json!({"on": false}) } else { json!({"on": true, "bri": hue::pct_to_bri(v)}) })
        }))
        .action(
            Action::slider("temperature", "Color Temperature", 2000.0, 6500.0)
                .icon("thermometer")
                .unit("K")
                .step(100.0)
                .inputs(vec![target_input(), input::text("step", "Step (K)").placeholder("100").default_value("100")])
                .live(POLL, |ic| {
                    Ok(match read(&ic.inputs)?["ct"].as_i64() {
                        Some(m) => Visual::new().state(hue::mired_to_kelvin(m)),
                        None => Visual::new().label("No temperature"),
                    })
                })
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => hue::mired_to_kelvin(read(&ctx.inputs)?["ct"].as_i64().unwrap_or(366)) as f64 + ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(100.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .clamp(2000.0, 6500.0);
                    write(&ctx.inputs, json!({"on": true, "ct": hue::kelvin_to_mired(target)}))?;
                    ctx.fan_out_by(&["target"], target.round());
                    Ok(())
                }),
        )
        .action(Action::button("set-color", "Set Color").icon("palette").inputs(vec![target_input(), input::text("color", "Color (#RRGGBB)").placeholder("#FF8800").default_value("#FF8800")]).on_press(|ctx| {
            let Some(xy) = hue::hex_to_xy(&ctx.input_str("color")) else {
                return ctx.fail("Color must look like #FF8800");
            };
            write(&ctx.inputs, json!({"on": true, "xy": xy}))
        }))
        .action(Action::button("scene", "Scene").icon("sparkles").inputs(vec![input::query_select("scene", "Scene", "scenes", &[])]).on_press(|ctx| {
            let scene = ctx.input_str("scene");
            if scene.is_empty() {
                return ctx.fail("Choose a scene");
            }
            let b = guard(bridge())?;
            guard(recall_scene(b, &scene))
        }))
        .action(
            Action::wheel("scenes-wheel", "Scenes Wheel")
                .icon("sparkles")
                .inputs(vec![input::query_select("room", "Room or zone", "rooms", &[]).with("default", "0")])
                .on_open(|o| {
                    let b = guard(bridge()).or_else(|e| o.fail(e.message().to_string()))?;
                    let room = o.input_str("room");
                    let list = guard(scenes(b, Some(if room.is_empty() { "0" } else { &room }))).or_else(|e| o.fail(e.message().to_string()))?;
                    Ok(list.into_iter().take(16).map(|(id, name, _)| json!({"id": id, "label": name, "icon": "sparkles"})).collect())
                })
                .on_child_press(|ctx| {
                    let Some(scene) = ctx.child_id.clone() else { return Ok(()) };
                    let b = guard(bridge())?;
                    guard(recall_scene(b, &scene))
                }),
        )
        .run()
}
