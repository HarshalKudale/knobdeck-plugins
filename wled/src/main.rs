//! WLED plugin for KnobDeck: LED strips and matrices running WLED, over the device's JSON API.
//! Devices are found by mDNS; setup also accepts addresses by hand.

mod wled;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, load_env, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use wled::SegTarget;

const ALL: &str = "__all__";
const POLL: Duration = Duration::from_secs(3);

/// Known devices: (address, name). Filled from setup's list plus discovery.
static DEVICES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn configured() -> Vec<String> {
    env_or("WLED_DEVICES", "").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

/// Re-scan: configured addresses plus whatever answers mDNS now. Once at a time: callers that
/// wait for a scan in progress use its result.
fn refresh() -> Vec<(String, String)> {
    static SCAN: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = SCAN.lock().unwrap();
    if last.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
        return DEVICES.lock().unwrap().clone();
    }
    let mut addrs = configured();
    for a in wled::discover(Duration::from_millis(1500)) {
        if !addrs.contains(&a) {
            addrs.push(a);
        }
    }
    let found: Vec<(String, String)> = addrs.into_iter().filter_map(|a| wled::name(&a).ok().map(|n| (a, n))).collect();
    *DEVICES.lock().unwrap() = found.clone();
    *last = Some(Instant::now());
    found
}

fn known() -> Vec<(String, String)> {
    let cached = DEVICES.lock().unwrap().clone();
    if cached.is_empty() {
        refresh()
    } else {
        cached
    }
}

/// The addresses an action targets: one device, or every known device.
fn targets(inputs: &Value) -> Result<Vec<String>> {
    let pick = inputs["device"].as_str().unwrap_or(ALL);
    let list: Vec<String> = if pick.is_empty() || pick == ALL { known().into_iter().map(|(a, _)| a).collect() } else { vec![pick.to_string()] };
    if list.is_empty() {
        return Err(Error::msg("No WLED device found: make sure it is on and on the same network, or add its address in setup"));
    }
    Ok(list)
}

fn first(inputs: &Value) -> Result<String> {
    Ok(targets(inputs)?.remove(0))
}

fn first_state(inputs: &Value) -> Result<Value> {
    wled::state(&first(inputs)?)
}

fn seg_target(inputs: &Value) -> SegTarget {
    SegTarget::parse(inputs["segment"].as_str().unwrap_or(""))
}

/// The segment a key shows (of the first targeted device).
fn shown(inputs: &Value) -> Result<Value> {
    Ok(wled::shown_segment(&first_state(inputs)?, seg_target(inputs)))
}

fn set_all(inputs: &Value, patch: Value) -> Result<()> {
    for a in targets(inputs)? {
        wled::set(&a, patch.clone())?;
    }
    Ok(())
}

/// Change `fields` on the targeted segments of every targeted device.
fn set_segments(inputs: &Value, fields: Value) -> Result<()> {
    let target = seg_target(inputs);
    for a in targets(inputs)? {
        let st = if target == SegTarget::All { Some(wled::state(&a)?) } else { None };
        wled::set(&a, json!({"seg": wled::seg_patch(target, &fields, st.as_ref())}))?;
    }
    Ok(())
}

/// Effect names per device, read once (they change only with a firmware update).
fn effects(addr: &str) -> Result<Vec<String>> {
    static CACHE: Mutex<Vec<(String, Vec<String>)>> = Mutex::new(Vec::new());
    if let Some((_, list)) = CACHE.lock().unwrap().iter().find(|(a, _)| a == addr) {
        return Ok(list.clone());
    }
    let list = wled::effects(addr)?;
    CACHE.lock().unwrap().push((addr.to_string(), list.clone()));
    Ok(list)
}

fn device_input() -> Value {
    input::query_select("device", "Device", "devices", &[]).with("default", ALL)
}

fn segment_input() -> Value {
    input::query_select("segment", "Segment", "segments", &["device"]).with("default", "selected")
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

fn index_options(list: Vec<String>) -> Value {
    Value::Array(list.into_iter().enumerate().filter(|(_, n)| !n.is_empty() && n != "-" && n != "RSVD").map(|(i, n)| json!({"value": i.to_string(), "label": n})).collect())
}

fn query_fail<T>(q: &knobdeck_sdk::QueryCtx, e: Error) -> Result<T> {
    q.fail(e.message().to_string())
}

/// A 0–255 segment value (`sx` speed, `ix` intensity) on a dial, shown in %.
fn level_slider(id: &str, label: &str, field: &'static str, icon: &str) -> Action {
    Action::slider(id, label, 0.0, 100.0)
        .icon(icon)
        .unit("%")
        .inputs(vec![device_input(), segment_input(), input::text("step", "Step (%)").placeholder("5").default_value("5")])
        .live(POLL, move |ic| Ok(Visual::new().state(wled::to_pct(shown(&ic.inputs)?[field].as_f64().unwrap_or(128.0)))))
        .on_adjust(move |ctx| {
            let target = match ctx.value {
                Some(v) => v,
                None => wled::to_pct(shown(&ctx.inputs)?[field].as_f64().unwrap_or(128.0)) + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0),
            }
            .round()
            .clamp(0.0, 100.0);
            set_segments(&ctx.inputs, json!({ field: wled::from_pct(target) }))?;
            ctx.fan_out_by(&["device", "segment"], target);
            Ok(())
        })
}

fn main() {
    load_env();
    if std::env::var("KNOBDECK_CODEGEN").as_deref() != Ok("1") {
        std::thread::spawn(refresh); // warm the list so the first press doesn't wait for discovery
    }
    Plugin::new(&format!("wled-{}", std::env::consts::OS), "WLED")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("LED strips and matrices running WLED: power, brightness, presets, effects with speed and intensity, palettes, color and segments.")
        .icon("rainbow")
        .setup("wled-setup", None)
        .setup_inputs(vec![input::text("addresses", "Device addresses (optional)").placeholder("192.168.1.50, wled-desk.local").with("help", "WLED devices on this network are found automatically. Add addresses only for devices that are not found.")])
        .query("devices", |_| {
            let mut opts = vec![json!({"value": ALL, "label": "All devices"})];
            opts.extend(refresh().into_iter().map(|(a, n)| json!({"value": a, "label": format!("{n} ({a})")})));
            Ok(Value::Array(opts))
        })
        .query("segments", |q| {
            let st = first_state(&q.inputs).or_else(|e| query_fail(q, e))?;
            let mut opts = vec![json!({"value": "selected", "label": "Selected segments"}), json!({"value": "all", "label": "All segments"})];
            for s in st["seg"].as_array().into_iter().flatten() {
                let Some(id) = s["id"].as_i64() else { continue };
                let name = s["n"].as_str().filter(|n| !n.is_empty()).map(String::from).unwrap_or_else(|| format!("Segment {id}"));
                opts.push(json!({"value": id.to_string(), "label": name}));
            }
            Ok(Value::Array(opts))
        })
        .query("effects", |q| Ok(index_options(first(&q.inputs).and_then(|a| effects(&a)).or_else(|e| query_fail(q, e))?)))
        .query("palettes", |q| Ok(index_options(first(&q.inputs).and_then(|a| wled::palettes(&a)).or_else(|e| query_fail(q, e))?)))
        .query("presets", |q| {
            let list = first(&q.inputs).and_then(|a| wled::presets(&a)).or_else(|e| query_fail(q, e))?;
            Ok(Value::Array(list.into_iter().map(|(id, n)| json!({"value": id.to_string(), "label": n})).collect()))
        })
        .action(
            Action::toggle("power", "Power")
                .icon("lightbulb")
                .inputs(vec![device_input()])
                .state("off", "Off", "lightbulb-off")
                .state("on", "On", "lightbulb")
                .live(POLL, |ic| Ok(Visual::new().state(on_label(first_state(&ic.inputs)?["on"] == true))))
                .on_press(|ctx| {
                    let on = first_state(&ctx.inputs)?["on"] != true;
                    set_all(&ctx.inputs, json!({"on": on}))?;
                    ctx.fan_out_by(&["device"], on_label(on));
                    Ok(())
                }),
        )
        .action(
            Action::slider("brightness", "Brightness", 0.0, 100.0)
                .icon("sun")
                .unit("%")
                .inputs(vec![device_input(), input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(POLL, |ic| {
                    let s = first_state(&ic.inputs)?;
                    Ok(Visual::new().state(if s["on"] == true { wled::to_pct(s["bri"].as_f64().unwrap_or(0.0)) } else { 0.0 }))
                })
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => {
                            let s = first_state(&ctx.inputs)?;
                            let now = if s["on"] == true { wled::to_pct(s["bri"].as_f64().unwrap_or(0.0)) } else { 0.0 };
                            now + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0)
                        }
                    }
                    .round()
                    .clamp(0.0, 100.0);
                    let patch = if target <= 0.0 { json!({"on": false}) } else { json!({"on": true, "bri": wled::from_pct(target).max(1)}) };
                    set_all(&ctx.inputs, patch)?;
                    ctx.fan_out_by(&["device"], target);
                    Ok(())
                }),
        )
        .action(Action::button("set-brightness", "Set Brightness").icon("sun").inputs(vec![device_input(), input::text("value", "Brightness (%)").placeholder("50").default_value("50")]).on_press(|ctx| {
            let Some(v) = ctx.input_f64("value").filter(|v| (1.0..=100.0).contains(v)) else {
                return ctx.fail("Brightness must be 1–100");
            };
            set_all(&ctx.inputs, json!({"on": true, "bri": wled::from_pct(v).max(1)}))
        }))
        .action(
            Action::button("preset", "Preset")
                .icon("bookmark")
                .inputs(vec![device_input(), input::query_select("preset", "Preset", "presets", &["device"])])
                .live(POLL, |ic| {
                    let active = first_state(&ic.inputs)?["ps"].as_i64().map(|p| p.to_string()) == Some(ic.input_str("preset"));
                    Ok(Visual::new().icon(if active { "bookmark-check" } else { "bookmark" }))
                })
                .on_press(|ctx| {
                    let Some(id) = ctx.input_str("preset").parse::<i64>().ok().filter(|id| *id > 0) else {
                        return ctx.fail("Choose a preset");
                    };
                    set_all(&ctx.inputs, json!({"ps": id}))
                }),
        )
        .action(
            Action::wheel("presets-wheel", "Presets Wheel")
                .icon("bookmark")
                .inputs(vec![device_input()])
                .on_open(|o| {
                    let list = first(&o.inputs).and_then(|a| wled::presets(&a)).or_else(|e| o.fail(e.message().to_string()))?;
                    Ok(list.into_iter().take(16).map(|(id, n)| json!({"id": id.to_string(), "label": n, "icon": "bookmark"})).collect())
                })
                .on_child_press(|ctx| {
                    let Some(id) = ctx.child_id.as_deref().and_then(|c| c.parse::<i64>().ok()) else { return Ok(()) };
                    set_all(&ctx.inputs, json!({"ps": id}))
                }),
        )
        .action(
            Action::button("effect", "Effect")
                .icon("sparkles")
                .inputs(vec![device_input(), segment_input(), input::query_select("effect", "Effect", "effects", &["device"])])
                .live(POLL, |ic| {
                    let active = shown(&ic.inputs)?["fx"].as_i64().map(|f| f.to_string()) == Some(ic.input_str("effect"));
                    Ok(Visual::new().icon(if active { "sparkles" } else { "sparkle" }))
                })
                .on_press(|ctx| {
                    let Ok(fx) = ctx.input_str("effect").parse::<i64>() else {
                        return ctx.fail("Choose an effect");
                    };
                    set_segments(&ctx.inputs, json!({"fx": fx}))
                }),
        )
        .action(
            Action::slider("effect-dial", "Effect (dial)", 0.0, 255.0)
                .icon("sparkles")
                .step(1.0)
                .inputs(vec![device_input(), segment_input()])
                .live(POLL, |ic| {
                    let a = first(&ic.inputs)?;
                    let fx = wled::shown_segment(&wled::state(&a)?, seg_target(&ic.inputs))["fx"].as_i64().unwrap_or(0);
                    let name = effects(&a)?.get(fx as usize).cloned().unwrap_or_else(|| format!("Effect {fx}"));
                    Ok(Visual::new().state(fx).label(name))
                })
                .on_adjust(|ctx| {
                    let a = first(&ctx.inputs)?;
                    let effects = effects(&a)?;
                    let last = effects.len().saturating_sub(1) as f64;
                    let target = match ctx.value {
                        Some(v) => v,
                        None => wled::shown_segment(&wled::state(&a)?, seg_target(&ctx.inputs))["fx"].as_f64().unwrap_or(0.0) + ctx.delta.unwrap_or(0.0).signum(),
                    }
                    .round()
                    .clamp(0.0, last);
                    set_segments(&ctx.inputs, json!({"fx": target as i64}))?;
                    let name = effects.get(target as usize).cloned().unwrap_or_default();
                    ctx.set_visual(Visual::new().state(target).label(name));
                    Ok(())
                }),
        )
        .action(
            Action::wheel("effects-wheel", "Effects Wheel")
                .icon("sparkles")
                .inputs(vec![device_input(), segment_input(), input::text("names", "Effects (comma separated, optional)").placeholder("Rainbow, Fire 2012, Colorloop").with("help", "Leave empty for the first 16 effects.")])
                .on_open(|o| {
                    let list = first(&o.inputs).and_then(|a| effects(&a)).or_else(|e| o.fail(e.message().to_string()))?;
                    let wanted: Vec<String> = o.input_str("names").split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect();
                    let pick: Vec<(usize, String)> = list.into_iter().enumerate().filter(|(_, n)| !n.is_empty() && n != "-" && n != "RSVD").filter(|(_, n)| wanted.is_empty() || wanted.contains(&n.to_lowercase())).take(16).collect();
                    Ok(pick.into_iter().map(|(i, n)| json!({"id": i.to_string(), "label": n, "icon": "sparkles"})).collect())
                })
                .on_child_press(|ctx| {
                    let Some(fx) = ctx.child_id.as_deref().and_then(|c| c.parse::<i64>().ok()) else { return Ok(()) };
                    set_segments(&ctx.inputs, json!({"fx": fx}))
                }),
        )
        .action(level_slider("speed", "Effect Speed", "sx", "gauge"))
        .action(level_slider("intensity", "Effect Intensity", "ix", "flame"))
        .action(
            Action::button("palette", "Palette")
                .icon("palette")
                .inputs(vec![device_input(), segment_input(), input::query_select("palette", "Palette", "palettes", &["device"])])
                .live(POLL, |ic| {
                    let active = shown(&ic.inputs)?["pal"].as_i64().map(|f| f.to_string()) == Some(ic.input_str("palette"));
                    Ok(Visual::new().icon(if active { "palette" } else { "swatch-book" }))
                })
                .on_press(|ctx| {
                    let Ok(pal) = ctx.input_str("palette").parse::<i64>() else {
                        return ctx.fail("Choose a palette");
                    };
                    set_segments(&ctx.inputs, json!({"pal": pal}))
                }),
        )
        .action(
            Action::button("set-color", "Set Color")
                .icon("paintbrush")
                .inputs(vec![
                    device_input(),
                    segment_input(),
                    input::text("color", "Color (#RRGGBB)").placeholder("#FF8800").default_value("#FF8800"),
                    input::select("slot", "Color slot", &[("0", "Primary"), ("1", "Secondary"), ("2", "Tertiary")]).default_value("0"),
                ])
                .on_press(|ctx| {
                    let Some(rgb) = wled::hex(&ctx.input_str("color")) else {
                        return ctx.fail("Color must look like #FF8800");
                    };
                    // `col` takes up to three colors; `[]` leaves a slot as it is.
                    let mut col = vec![json!([]), json!([]), json!([])];
                    let slot = ctx.input_str("slot").parse::<usize>().unwrap_or(0).min(2);
                    col[slot] = json!(rgb);
                    col.truncate(slot + 1);
                    set_segments(&ctx.inputs, json!({"col": col}))?;
                    if slot == 0 {
                        set_all(&ctx.inputs, json!({"on": true}))?;
                    }
                    Ok(())
                }),
        )
        .action(
            Action::button("select-segment", "Select Segment")
                .icon("layers")
                .inputs(vec![input::query_select("device", "Device", "devices", &[]), input::query_select("segment", "Segment", "segments", &["device"])])
                .live(POLL, |ic| {
                    let SegTarget::One(id) = seg_target(&ic.inputs) else { return Ok(Visual::new()) };
                    let st = first_state(&ic.inputs)?;
                    let sel = st["seg"].as_array().into_iter().flatten().any(|s| s["id"].as_i64() == Some(id) && s["sel"] == true);
                    Ok(Visual::new().icon(if sel { "layers" } else { "square-dashed" }))
                })
                .on_press(|ctx| {
                    let SegTarget::One(id) = seg_target(&ctx.inputs) else {
                        return ctx.fail("Choose one segment");
                    };
                    // Only this segment stays selected, so "Selected segments" keys act on it.
                    for a in targets(&ctx.inputs)? {
                        let ids = wled::segment_ids(&wled::state(&a)?);
                        let seg: Vec<Value> = ids.into_iter().map(|i| json!({"id": i, "sel": i == id})).collect();
                        wled::set(&a, json!({"seg": seg}))?;
                    }
                    Ok(())
                }),
        )
        .run()
}
