//! Blender plugin for KnobDeck: drives Blender through the KnobDeck add-on for Blender
//! (`Knobdeck/knobdeck-blender-companion`), which links to KnobDeck's companion hub and runs each
//! action with Blender's own Python API (`bpy`), never with keystrokes. The plugin opens no port.
//!
//! The add-on sends a `state` event whenever what the keys show changes (mode, active object,
//! frame, playback, shading, overlays, brush, transform); the plugin keeps the last one and pushes
//! it to the placed keys. Several Blender windows (processes) can link at once: actions go to the
//! one that changed last, which is the one being worked in.

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{companion, json, Action, Ctx, Error, Plugin, Result, Runtime, Value, Visual};
use std::collections::HashMap;
use std::sync::Mutex;

const NOT_LINKED: &str = "Blender isn't linked to KnobDeck. Install the KnobDeck add-on in Blender (Edit ▸ Preferences ▸ Get Extensions, search \"KnobDeck\") and keep Blender open.";

/// Linked Blender processes (link ids, oldest first) and the one that reported a change last.
static LINKS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static ACTIVE: Mutex<Option<String>> = Mutex::new(None);
/// The last `state` event of each link.
static STATES: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);
/// What was last pushed to the keys, so unchanged values aren't sent again.
static PUSHED: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

fn target_link() -> Option<String> {
    let links = LINKS.lock().unwrap();
    let active = ACTIVE.lock().unwrap().clone().filter(|a| links.contains(a));
    active.or_else(|| links.last().cloned())
}

/// The target Blender's last state (`Null` when none is linked).
fn state() -> Value {
    let Some(link) = target_link() else { return Value::Null };
    STATES.lock().unwrap().as_ref().and_then(|m| m.get(&link).cloned()).unwrap_or(Value::Null)
}

/// Keep LINKS / ACTIVE / STATES in step with the hub's events. Returns whether the target's state changed.
fn on_companion(update: companion::Update) -> bool {
    let mut links = LINKS.lock().unwrap();
    match update {
        companion::Update::Connected(l) => {
            links.retain(|x| x != &l.link);
            links.push(l.link);
            false
        }
        companion::Update::Disconnected(g) => {
            links.retain(|x| x != &g.link);
            let mut active = ACTIVE.lock().unwrap();
            if active.as_deref() == Some(&g.link) {
                *active = None;
            }
            if let Some(m) = STATES.lock().unwrap().as_mut() {
                m.remove(&g.link);
            }
            true
        }
        companion::Update::Event(e) if e.event == "state" => {
            if !links.contains(&e.link) {
                links.push(e.link.clone());
            }
            *ACTIVE.lock().unwrap() = Some(e.link.clone());
            STATES.lock().unwrap().get_or_insert_with(HashMap::new).insert(e.link, e.payload);
            true
        }
        companion::Update::Event(_) => false,
    }
}

/// Run `action` in the target Blender and return its result.
fn call_with(action: &str, params: Value, timeout_ms: u64) -> Result<Value> {
    let link = target_link().ok_or_else(|| Error::msg(NOT_LINKED))?;
    match companion::request_with_timeout(Some(&link), action, params, Some(timeout_ms)) {
        Ok(answer) => companion::check(answer).map_err(|e| Error::msg(format!("Blender: {}", e.message()))),
        Err(e) => Err(Error::msg(format!("Blender didn't answer: {}", e.message()))),
    }
}

fn call(action: &str, params: Value) -> Result<Value> {
    call_with(action, params, 5000)
}

// ── Keys from the state ────────────────────────────────────────────────────────

fn on_off(v: &Value) -> &'static str {
    if v.as_bool().unwrap_or(false) { "on" } else { "off" }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// Rounded for a key: whole numbers without decimals, others to 3 places.
fn fmt(v: f64) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

/// The status key's label: "Sculpt · Cube".
fn status_label(s: &Value) -> String {
    if s.is_null() {
        return "Blender offline".into();
    }
    let mode = s["mode_label"].as_str().unwrap_or("Blender");
    match s["object"].as_str() {
        Some(o) if !o.is_empty() => format!("{mode} · {o}"),
        _ => mode.to_string(),
    }
}

const AXES: [&str; 3] = ["x", "y", "z"];
const PROPS: [&str; 3] = ["location", "rotation", "scale"];

fn transform_value(s: &Value, property: &str, axis: &str) -> Option<f64> {
    let i = AXES.iter().position(|a| *a == axis)?;
    num(&s["transform"][property][i])
}

/// Every (key, state) the keys should show for `s`.
fn key_states(s: &Value) -> Vec<(String, String)> {
    let mut out = vec![("status".to_string(), status_label(s))];
    if s.is_null() {
        return out;
    }
    out.push(("play-pause".into(), if s["playing"].as_bool().unwrap_or(false) { "playing" } else { "paused" }.into()));
    out.push(("overlays".into(), on_off(&s["overlays"]).into()));
    out.push(("xray".into(), on_off(&s["xray"]).into()));
    if let Some(f) = num(&s["frame"]) {
        out.push(("frame".into(), fmt(f)));
    }
    if let Some(v) = num(&s["brush"]["size"]) {
        out.push(("brush-size".into(), fmt(v)));
    }
    if let Some(v) = num(&s["brush"]["strength"]) {
        out.push(("brush-strength".into(), fmt((v * 100.0).round())));
    }
    for p in PROPS {
        for a in AXES {
            if let Some(v) = transform_value(s, p, a) {
                out.push((format!("transform:{p}:{a}"), fmt(v)));
            }
        }
    }
    out
}

/// Push what changed since the last push.
fn push_state(rt: &Runtime) {
    let s = state();
    let mut pushed = PUSHED.lock().unwrap();
    let pushed = pushed.get_or_insert_with(HashMap::new);
    for (key, value) in key_states(&s) {
        if pushed.get(&key) == Some(&value) {
            continue;
        }
        if key == "status" {
            rt.notify_visual("status", Visual::new().label(&value));
        } else if let Some(rest) = key.strip_prefix("transform:") {
            let (p, a) = rest.split_once(':').unwrap();
            rt.notify_matching("transform", &json!({"property": p, "axis": a}), &value);
        } else {
            rt.notify(&key, &value);
        }
        pushed.insert(key, value);
    }
}

// ── Actions ────────────────────────────────────────────────────────────────────

fn button(id: &str, label: &str, icon: &str, action: &'static str, params: Value) -> Action {
    Action::button(id, label).icon(icon).on_press(move |_| call(action, params.clone()).map(|_| ()))
}

fn toggle(id: &'static str, label: &str, icon: &str, action: &'static str, states: [(&'static str, &'static str, &'static str); 2]) -> Action {
    let mut a = Action::toggle(id, label).icon(icon);
    for (value, label, icon) in states {
        a = a.state(value, label, icon);
    }
    a.initial_with(move |_| key_states(&state()).into_iter().find(|(k, _)| k == id).map(|(_, v)| v))
        .on_press(move |ctx| {
            let r = call(action, json!({"value": "toggle"}))?;
            if let Some(on) = r["value"].as_bool() {
                ctx.set_state(if on { states[1].0 } else { states[0].0 });
            }
            Ok(())
        })
}

/// A step from the `step` input, else `default`.
fn step_of(ctx: &Ctx, default: f64) -> f64 {
    ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(default)
}

/// `{value}` for a drag, `{delta}` (ticks × step) for a turn; `None` when neither came.
fn adjust_params(ctx: &Ctx, step: f64) -> Option<Value> {
    match (ctx.value, ctx.delta) {
        (Some(v), _) => Some(json!({"value": v})),
        (None, Some(d)) => Some(json!({"delta": d * step})),
        _ => None,
    }
}

fn shading_options() -> &'static [(&'static str, &'static str)] {
    &[("next", "Next mode"), ("previous", "Previous mode"), ("WIREFRAME", "Wireframe"), ("SOLID", "Solid"), ("MATERIAL", "Material Preview"), ("RENDERED", "Rendered")]
}

fn mode_options() -> &'static [(&'static str, &'static str)] {
    &[
        ("OBJECT", "Object Mode"),
        ("EDIT", "Edit Mode"),
        ("SCULPT", "Sculpt Mode"),
        ("VERTEX_PAINT", "Vertex Paint"),
        ("WEIGHT_PAINT", "Weight Paint"),
        ("TEXTURE_PAINT", "Texture Paint"),
        ("POSE", "Pose Mode"),
    ]
}

/// Workspaces of the target Blender, as `[{value, label}]`.
fn workspaces() -> Result<Vec<Value>> {
    let r = call("workspaces", json!({}))?;
    Ok(r["items"].as_array().cloned().unwrap_or_default())
}

fn main() {
    Plugin::new(&format!("blender-{}", std::env::consts::OS), "Blender")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Control Blender through the KnobDeck add-on: playback and frames, workspaces, viewport shading, overlays and X-ray, render, save, undo, brush size and strength, and the selected objects' transform on a dial.")
        .icon("app-icon.svg")
        .ack_timeout_ms(5000)
        // Timeline
        .action(
            toggle("play-pause", "Play / Pause", "play", "playback", [("paused", "Play", "play"), ("playing", "Pause", "pause")]),
        )
        .action(
            Action::slider("frame", "Frame", 0.0, 10000.0)
                .icon("film")
                .step(1.0)
                .inputs(vec![input::text("step", "Frames per tick").placeholder("1").default_value("1")])
                .initial_with(|_| num(&state()["frame"]).map(fmt))
                .on_adjust(|ctx| {
                    let Some(params) = adjust_params(ctx, step_of(ctx, 1.0).round().max(1.0)) else { return Ok(()) };
                    let r = call("frame", params)?;
                    if let Some(f) = num(&r["value"]) {
                        ctx.set_state(fmt(f));
                    }
                    Ok(())
                }),
        )
        .action(button("jump-start", "Jump to Start", "skip-back", "jump", json!({"to": "start"})))
        .action(button("jump-end", "Jump to End", "skip-forward", "jump", json!({"to": "end"})))
        .action(button("previous-keyframe", "Previous Keyframe", "chevron-first", "jump", json!({"to": "previous_keyframe"})))
        .action(button("next-keyframe", "Next Keyframe", "chevron-last", "jump", json!({"to": "next_keyframe"})))
        // Workspaces and modes
        .action(
            Action::button("workspace", "Workspace")
                .icon("layout-panel-top")
                .inputs(vec![input::query_select("workspace", "Workspace", "workspaces", &[])])
                .on_press(|ctx| {
                    let name = ctx.input_str("workspace");
                    if name.trim().is_empty() {
                        return ctx.fail("Choose a workspace");
                    }
                    call("workspace", json!({"name": name})).map(|_| ())
                }),
        )
        .action(
            Action::wheel("workspace-wheel", "Workspaces")
                .icon("layout-panel-top")
                .on_open(|ctx| {
                    let items = workspaces()?;
                    if items.is_empty() {
                        return ctx.fail("Blender has no workspaces");
                    }
                    Ok(items.into_iter().take(16).map(|i| json!({"id": i["value"], "label": i["label"], "icon": "layout-panel-top"})).collect())
                })
                .on_child_press(|ctx| call("workspace", json!({"name": ctx.child_id.clone().unwrap_or_default()})).map(|_| ())),
        )
        .action(
            Action::button("object-mode", "Object Interaction Mode")
                .icon("mouse-pointer-2")
                .inputs(vec![input::select("mode", "Mode", mode_options()).default_value("OBJECT")])
                .on_press(|ctx| call("mode", json!({"mode": ctx.input_str("mode")})).map(|_| ())),
        )
        .action(
            Action::button("status", "Mode and Object")
                .icon("info")
                .live(std::time::Duration::from_secs(30), |_| Ok(Visual::new().label(status_label(&state()))))
                .on_press(|_| call("state", json!({})).map(|_| ())),
        )
        // Viewport
        .action(
            Action::button("shading", "Viewport Shading")
                .icon("contrast")
                .inputs(vec![input::select("shading", "Shading", shading_options()).default_value("next")])
                .on_press(|ctx| call("shading", json!({"type": ctx.input_str("shading")})).map(|_| ())),
        )
        .action(toggle("overlays", "Overlays", "layers", "overlays", [("off", "Overlays Off", "layers"), ("on", "Overlays On", "layers")]))
        .action(toggle("xray", "X-Ray", "scan", "xray", [("off", "X-Ray Off", "scan"), ("on", "X-Ray On", "scan-eye")]))
        // Render and file
        .action(button("render-image", "Render Image", "camera", "render", json!({"animation": false})))
        .action(button("render-animation", "Render Animation", "clapperboard", "render", json!({"animation": true})))
        .action(Action::button("save", "Save").icon("save").on_press(|_| call_with("save", json!({}), 30000).map(|_| ())))
        .action(button("undo", "Undo", "undo-2", "undo", json!({})))
        .action(button("redo", "Redo", "redo-2", "redo", json!({})))
        // Dials
        .action(
            Action::slider("brush-size", "Brush Size", 1.0, 1000.0)
                .icon("brush")
                .unit("px")
                .step(1.0)
                .inputs(vec![input::text("step", "Pixels per tick").placeholder("5").default_value("5")])
                .initial_with(|_| num(&state()["brush"]["size"]).map(fmt))
                .on_adjust(|ctx| {
                    let Some(params) = adjust_params(ctx, step_of(ctx, 5.0)) else { return Ok(()) };
                    let mut params = params;
                    params["property"] = json!("size");
                    let r = call("brush", params)?;
                    if let Some(v) = num(&r["value"]) {
                        ctx.set_state(fmt(v));
                    }
                    Ok(())
                }),
        )
        .action(
            Action::slider("brush-strength", "Brush Strength", 0.0, 100.0)
                .icon("brush")
                .unit("%")
                .step(1.0)
                .inputs(vec![input::text("step", "Percent per tick").placeholder("5").default_value("5")])
                .initial_with(|_| num(&state()["brush"]["strength"]).map(|v| fmt((v * 100.0).round())))
                .on_adjust(|ctx| {
                    // The add-on takes strength as 0–1, the key shows percent.
                    let params = match (ctx.value, ctx.delta) {
                        (Some(v), _) => json!({"property": "strength", "value": v / 100.0}),
                        (None, Some(d)) => json!({"property": "strength", "delta": d * step_of(ctx, 5.0) / 100.0}),
                        _ => return Ok(()),
                    };
                    let r = call("brush", params)?;
                    if let Some(v) = num(&r["value"]) {
                        ctx.set_state(fmt((v * 100.0).round()));
                    }
                    Ok(())
                }),
        )
        .action(
            Action::slider("transform", "Object Transform", -10000.0, 10000.0)
                .icon("move-3d")
                .step(0.01)
                .inputs(vec![
                    input::select("property", "Property", &[("location", "Location"), ("rotation", "Rotation (degrees)"), ("scale", "Scale")]).default_value("location"),
                    input::select("axis", "Axis", &[("x", "X"), ("y", "Y"), ("z", "Z")]).default_value("x"),
                    input::text("step", "Step per tick").placeholder("0.1 m, 5°, 0.05"),
                ])
                .initial_with(|ic| {
                    let (p, a) = (ic.input_str("property"), ic.input_str("axis"));
                    transform_value(&state(), if p.is_empty() { "location" } else { &p }, if a.is_empty() { "x" } else { &a }).map(fmt)
                })
                .on_adjust(|ctx| {
                    let property = match ctx.input_str("property") {
                        p if p.is_empty() => "location".to_string(),
                        p => p,
                    };
                    let axis = match ctx.input_str("axis") {
                        a if a.is_empty() => "x".to_string(),
                        a => a,
                    };
                    let default = match property.as_str() {
                        "rotation" => 5.0,
                        "scale" => 0.05,
                        _ => 0.1,
                    };
                    let Some(mut params) = adjust_params(ctx, step_of(ctx, default)) else { return Ok(()) };
                    params["property"] = json!(property);
                    params["axis"] = json!(axis);
                    let r = call("transform", params)?;
                    if let Some(v) = num(&r["value"]) {
                        ctx.fan_out_by(&["property", "axis"], fmt(v));
                    }
                    Ok(())
                }),
        )
        .query("workspaces", |_| Ok(Value::Array(workspaces()?)))
        // Blender links through KnobDeck's companion hub; this keeps track of them.
        .companions()
        .on_event(|ev, rt| {
            if let Some(update) = companion::parse(ev) {
                if on_companion(update) {
                    push_state(rt);
                }
            }
            Ok(())
        })
        .on_ready(|rt| {
            // Blenders that linked before this process started (a plugin restart): ask the hub,
            // then ask the newest for its state.
            if rt.has_feature(knobdeck_sdk::protocol::features::COMPANION_HUB) {
                if let Ok(links) = companion::links() {
                    *LINKS.lock().unwrap() = links.into_iter().map(|l| l.link).collect();
                }
                if let Some(link) = target_link() {
                    if let Ok(s) = call("state", json!({})) {
                        STATES.lock().unwrap().get_or_insert_with(HashMap::new).insert(link, s);
                    }
                }
            }
            push_state(rt);
            Ok(())
        })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(id: &str) -> companion::Link {
        companion::Link { link: id.into(), instance: id.into(), app: "blender".into(), app_version: None, name: None, version: None, features: vec![] }
    }

    fn event(link: &str, payload: Value) -> companion::Update {
        companion::Update::Event(companion::LinkEvent { link: link.into(), event: "state".into(), payload })
    }

    #[test]
    fn the_blender_that_changed_last_is_driven() {
        LINKS.lock().unwrap().clear();
        *ACTIVE.lock().unwrap() = None;
        assert_eq!(target_link(), None);
        assert!(state().is_null());
        on_companion(companion::Update::Connected(link("a")));
        on_companion(companion::Update::Connected(link("b")));
        assert_eq!(target_link().as_deref(), Some("b"), "the newest link");
        assert!(on_companion(event("a", json!({"frame": 7}))));
        assert_eq!(target_link().as_deref(), Some("a"));
        assert_eq!(state()["frame"], 7);
        on_companion(companion::Update::Connected(link("c")));
        assert_eq!(target_link().as_deref(), Some("a"), "a new link doesn't take over");
        on_companion(companion::Update::Disconnected(companion::LinkGone { link: "a".into() }));
        assert_eq!(target_link().as_deref(), Some("c"));
        assert!(state().is_null(), "c sent no state yet");
    }

    #[test]
    fn keys_follow_the_state() {
        let s = json!({
            "mode_label": "Sculpt", "object": "Cube", "frame": 12, "playing": true, "overlays": false, "xray": true,
            "brush": {"size": 70, "strength": 0.5},
            "transform": {"location": [1.0, 2.5, -0.1234], "rotation": [0.0, 90.0, 0.0], "scale": [1, 1, 1]}
        });
        let k: HashMap<String, String> = key_states(&s).into_iter().collect();
        assert_eq!(k["status"], "Sculpt · Cube");
        assert_eq!(k["play-pause"], "playing");
        assert_eq!(k["overlays"], "off");
        assert_eq!(k["xray"], "on");
        assert_eq!(k["frame"], "12");
        assert_eq!(k["brush-size"], "70");
        assert_eq!(k["brush-strength"], "50");
        assert_eq!(k["transform:location:y"], "2.5");
        assert_eq!(k["transform:location:z"], "-0.123");
        assert_eq!(k["transform:rotation:y"], "90");
        let offline: HashMap<String, String> = key_states(&Value::Null).into_iter().collect();
        assert_eq!(offline.len(), 1);
        assert_eq!(offline["status"], "Blender offline");
        let no_object = json!({"mode_label": "Object", "object": null});
        assert_eq!(status_label(&no_object), "Object");
    }
}
