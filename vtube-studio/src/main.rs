//! VTube Studio plugin for KnobDeck: trigger the model's hotkeys, load models, toggle expressions,
//! move / resize / rotate the model on a dial, drive a tracking parameter from a dial, and show the
//! current model, all through VTube Studio's public WebSocket API (no keystrokes).

mod vts;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::collections::HashMap;
use std::sync::{Mutex, Once};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);
const TOKEN_KEY: &str = "VTUBE_STUDIO_TOKEN";
/// KnobDeck's own custom parameters, `KnobDeckDial1`…`KnobDeckDial4`, created on first use.
const OWN_PARAM: &str = "KnobDeckDial";

fn endpoint() -> vts::Endpoint {
    vts::Endpoint { host: env_or("VTUBE_STUDIO_HOST", "127.0.0.1"), port: env_or("VTUBE_STUDIO_PORT", "8001") }
}

fn call(message_type: &str, data: Value) -> Result<Value> {
    vts::call(&endpoint(), message_type, data).map_err(Error::msg)
}

fn list(r: &Value, key: &str) -> Vec<Value> {
    r[key].as_array().cloned().unwrap_or_default()
}

fn required(ctx: &Ctx, key: &str, what: &str) -> Result<String> {
    let v = ctx.input_str(key);
    if v.trim().is_empty() {
        return ctx.fail(format!("Choose {what}"));
    }
    Ok(v.trim().to_string())
}

fn expressions() -> Result<Vec<Value>> {
    Ok(list(&call("ExpressionStateRequest", json!({"details": false}))?, "expressions"))
}

fn expression_active(file: &str) -> Result<bool> {
    Ok(expressions()?.iter().any(|e| e["file"] == file && e["active"] == true))
}

/// Injected parameter values VTube Studio must be sent again at least once a second, or it hands
/// the parameter back to face tracking. `name → value`.
static HELD: Mutex<Option<HashMap<String, f64>>> = Mutex::new(None);
static KEEPER: Once = Once::new();

fn inject(name: &str, value: f64, hold: bool) -> Result<()> {
    call("InjectParameterDataRequest", json!({"faceFound": false, "mode": "set", "parameterValues": [{"id": name, "value": value}]}))?;
    let mut held = HELD.lock().unwrap();
    let map = held.get_or_insert_with(HashMap::new);
    if hold {
        map.insert(name.to_string(), value);
    } else {
        map.remove(name);
    }
    drop(held);
    if hold {
        KEEPER.call_once(|| {
            std::thread::spawn(|| loop {
                std::thread::sleep(Duration::from_millis(500));
                let values: Vec<Value> = HELD.lock().unwrap().iter().flatten().map(|(id, v)| json!({"id": id, "value": v})).collect();
                if !values.is_empty() {
                    let _ = call("InjectParameterDataRequest", json!({"faceFound": false, "mode": "set", "parameterValues": values}));
                }
            });
        });
    }
    Ok(())
}

/// (value, min, max) of a tracking parameter; a held value wins over what VTube Studio reports.
fn parameter(name: &str) -> Result<(f64, f64, f64)> {
    let p = match call("ParameterValueRequest", json!({"name": name})) {
        Err(_) if name.starts_with(OWN_PARAM) => {
            // KnobDeck's own custom parameter: create it the first time (0–1), then read it.
            call("ParameterCreationRequest", json!({"parameterName": name, "explanation": "Set by a KnobDeck dial", "min": 0, "max": 1, "defaultValue": 0}))?;
            call("ParameterValueRequest", json!({"name": name}))?
        }
        other => other?,
    };
    let (min, max) = (p["min"].as_f64().unwrap_or(0.0), p["max"].as_f64().unwrap_or(1.0));
    let held = HELD.lock().unwrap().as_ref().and_then(|m| m.get(name).copied());
    Ok((held.or(p["value"].as_f64()).unwrap_or(min), min, max))
}

/// The model's position on a −100…100 scale per axis: size as VTube Studio has it (−100…100),
/// x / y × 100 (−1…1 is the window's width / height), rotation in degrees / 1.8 (±180°).
fn axis_to_scale(axis: &str, pos: &Value) -> f64 {
    match axis {
        "x" => pos["positionX"].as_f64().unwrap_or(0.0) * 100.0,
        "y" => pos["positionY"].as_f64().unwrap_or(0.0) * 100.0,
        "rotation" => {
            // VTube Studio reports 0…360°; the dial shows −180…180°.
            let r = pos["rotation"].as_f64().unwrap_or(0.0).rem_euclid(360.0);
            (if r > 180.0 { r - 360.0 } else { r }) / 1.8
        }
        _ => pos["size"].as_f64().unwrap_or(0.0),
    }
}

fn move_request(axis: &str, scaled: f64, seconds: f64) -> Value {
    let (field, value) = match axis {
        "x" => ("positionX", scaled / 100.0),
        "y" => ("positionY", scaled / 100.0),
        "rotation" => ("rotation", scaled * 1.8),
        _ => ("size", scaled),
    };
    json!({"timeInSeconds": seconds, "valuesAreRelativeToModel": false, field: value})
}

fn step(ctx: &Ctx, default: f64) -> f64 {
    ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(default)
}

fn main() {
    knobdeck_sdk::load_env();
    vts::TOKEN_STORE
        .set((
            || knobdeck_sdk::secrets::env_or_stored(TOKEN_KEY),
            |t| {
                if let Err(e) = knobdeck_sdk::secrets::set(TOKEN_KEY, t) {
                    knobdeck_sdk::log(format!("couldn't keep VTube Studio's token: {e}"));
                }
            },
        ))
        .ok();
    let os = std::env::consts::OS;
    Plugin::new(&format!("vtube-studio-{os}"), "VTube Studio")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[os]) // one package per OS
        .author("KnobDeck")
        .description("VTube Studio hotkeys, models and expressions on keys; move, resize and rotate the model or drive a tracking parameter on a dial; the current model on a key.")
        .icon("sparkles")
        .setup("vtube-studio-setup", None)
        .setup_inputs(vec![
            input::text("host", "Host").placeholder("127.0.0.1").default_value("127.0.0.1"),
            input::text("port", "Port")
                .placeholder("8001")
                .default_value("8001")
                .with("help", "VTube Studio ▸ Settings ▸ Start API shows the port. The first time, VTube Studio asks you to allow KnobDeck."),
        ])
        .query("hotkeys", |q| {
            let r = call("HotkeysInCurrentModelRequest", json!({})).or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(
                list(&r, "availableHotkeys")
                    .iter()
                    .map(|h| {
                        let name = h["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("(unnamed)");
                        json!({"value": h["hotkeyID"], "label": format!("{name} ({})", h["type"].as_str().unwrap_or(""))})
                    })
                    .collect(),
            ))
        })
        .query("models", |q| {
            let r = call("AvailableModelsRequest", json!({})).or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(list(&r, "availableModels").iter().map(|m| json!({"value": m["modelID"], "label": m["modelName"]})).collect()))
        })
        .query("expressions", |q| {
            let list = expressions().or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(list.iter().map(|e| json!({"value": e["file"], "label": e["name"]})).collect()))
        })
        .query("parameters", |q| {
            let r = call("InputParameterListRequest", json!({})).or_else(|e| q.fail(e.message().to_string()))?;
            let mut out: Vec<Value> =
                (1..=4).map(|i| json!({"value": format!("{OWN_PARAM}{i}"), "label": format!("{OWN_PARAM}{i} (KnobDeck's own: map it to a model parameter in VTube Studio)")})).collect();
            out.extend(
                list(&r, "customParameters")
                    .iter()
                    .filter(|p| !p["name"].as_str().unwrap_or("").starts_with(OWN_PARAM))
                    .map(|p| json!({"value": p["name"], "label": format!("{} (custom)", p["name"].as_str().unwrap_or(""))})),
            );
            out.extend(list(&r, "defaultParameters").iter().map(|p| json!({"value": p["name"], "label": p["name"]})));
            Ok(Value::Array(out))
        })
        .action(
            Action::button("hotkey", "Trigger Hotkey")
                .icon("zap")
                .inputs(vec![input::query_combobox("hotkey", "Hotkey", "hotkeys").with("help", "A hotkey of the model loaded now (VTube Studio's own, not a keyboard shortcut).")])
                .on_press(|ctx| {
                    let id = required(ctx, "hotkey", "a hotkey")?;
                    call("HotkeyTriggerRequest", json!({"hotkeyID": id})).map(|_| ())
                }),
        )
        .action(
            Action::toggle("model", "Load Model")
                .icon("user-round")
                .state("other", "Load Model", "user-round")
                .state("loaded", "Loaded", "circle-check")
                .inputs(vec![input::query_combobox("model", "Model", "models")])
                .live(POLL, |ic| {
                    let r = call("CurrentModelRequest", json!({}))?;
                    Ok(Visual::new().state(if r["modelID"] == ic.input_str("model").as_str() { "loaded" } else { "other" }))
                })
                .on_press(|ctx| {
                    let id = required(ctx, "model", "a model")?;
                    call("ModelLoadRequest", json!({"modelID": id}))?;
                    ctx.fan_out_by(&["model"], "loaded");
                    Ok(())
                }),
        )
        .action(
            Action::toggle("expression", "Toggle Expression")
                .icon("smile")
                .state("off", "Expression Off", "meh")
                .state("on", "Expression On", "smile")
                .inputs(vec![input::query_combobox("expression", "Expression", "expressions"), input::text("fade", "Fade (seconds)").placeholder("0.25").default_value("0.25")])
                .live(POLL, |ic| Ok(Visual::new().state(if expression_active(&ic.input_str("expression"))? { "on" } else { "off" })))
                .on_press(|ctx| {
                    let file = required(ctx, "expression", "an expression")?;
                    let on = !expression_active(&file)?;
                    let fade = ctx.input_f64("fade").unwrap_or(0.25).clamp(0.0, 2.0);
                    call("ExpressionActivationRequest", json!({"expressionFile": file, "active": on, "fadeTime": fade}))?;
                    ctx.fan_out_by(&["expression"], if on { "on" } else { "off" });
                    Ok(())
                }),
        )
        .action(
            Action::slider("move-model", "Move / Resize Model", -100.0, 100.0)
                .icon("move")
                .inputs(vec![
                    input::select("axis", "What the dial changes", &[("size", "Size"), ("x", "Left / right"), ("y", "Down / up"), ("rotation", "Rotation (±180°)")]).default_value("size"),
                    input::text("step", "Step").placeholder("2").default_value("2"),
                ])
                .live(POLL, |ic| {
                    let r = call("CurrentModelRequest", json!({}))?;
                    Ok(Visual::new().state(axis_to_scale(&ic.input_str("axis"), &r["modelPosition"]).round()))
                })
                .on_adjust(|ctx| {
                    let axis = ctx.input_str("axis");
                    let target = match ctx.value {
                        Some(v) => v,
                        None => axis_to_scale(&axis, &call("CurrentModelRequest", json!({}))?["modelPosition"]) + step(ctx, 2.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .clamp(-100.0, 100.0);
                    call("MoveModelRequest", move_request(&axis, target, 0.0))?;
                    ctx.fan_out_by(&["axis"], target.round());
                    Ok(())
                }),
        )
        .action(
            Action::slider("parameter", "Parameter Dial", 0.0, 100.0)
                .icon("sliders-horizontal")
                .unit("%")
                .inputs(vec![
                    input::query_combobox("parameter", "Parameter", "parameters")
                        .with("help", "A tracking parameter (custom ones are made by plugins or in VTube Studio). The dial sets it across its range, in place of face tracking."),
                    input::checkbox("hold", "Keep the value (VTube Studio returns a parameter to tracking a second after the last value)").with("default", true),
                    input::text("step", "Step (%)").placeholder("5").default_value("5"),
                ])
                .live(POLL, |ic| {
                    let name = ic.input_str("parameter");
                    if name.trim().is_empty() {
                        return Err(Error::msg("Choose a parameter"));
                    }
                    let (v, min, max) = parameter(name.trim())?;
                    Ok(Visual::new().state(vts::percent(v, min, max).round()))
                })
                .on_adjust(|ctx| {
                    let name = required(ctx, "parameter", "a parameter")?;
                    let (v, min, max) = parameter(&name)?;
                    let pct = match ctx.value {
                        Some(p) => p,
                        None => vts::percent(v, min, max) + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .clamp(0.0, 100.0);
                    inject(&name, vts::scale(pct, min, max), ctx.inputs.get("hold").is_none_or(|h| h.is_null() || h == true))?;
                    ctx.fan_out_by(&["parameter"], pct.round());
                    Ok(())
                }),
        )
        .action(
            Action::button("current-model", "Current Model")
                .icon("user-round")
                .live(POLL, |_| {
                    let r = call("CurrentModelRequest", json!({}))?;
                    Ok(Visual::new().label(if r["modelLoaded"] == true { r["modelName"].as_str().unwrap_or("—").to_string() } else { "No model".into() }))
                })
                .on_press(|_| Ok(())),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_axes() {
        let pos = json!({"positionX": 0.5, "positionY": -0.25, "rotation": 90.0, "size": -40.0});
        assert_eq!(axis_to_scale("x", &pos), 50.0);
        assert_eq!(axis_to_scale("rotation", &pos), 50.0);
        assert_eq!(axis_to_scale("size", &pos), -40.0);
        assert_eq!(axis_to_scale("rotation", &json!({"rotation": 270.0})), -50.0);
        assert_eq!(move_request("y", -25.0, 0.0), json!({"timeInSeconds": 0.0, "valuesAreRelativeToModel": false, "positionY": -0.25}));
        assert_eq!(move_request("rotation", 50.0, 0.0)["rotation"], 90.0);
    }
}
