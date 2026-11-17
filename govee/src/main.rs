//! Govee plugin for KnobDeck: lights and light strips over Govee's LAN API, or over Govee's cloud
//! API (the user's own key) for models without LAN Control. Scenes need the cloud API.

mod cloud;
mod lan;

use cloud::CloudDevice;
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{env_or, json, Action, Ctx, Error, Plugin, Result, Value, Visual};
use lan::{Lan, LanDevice};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_secs(3);
/// Cloud devices are read at most this often (the key allows 10,000 requests a day).
const CLOUD_TTL: Duration = Duration::from_secs(60);
const ALL: &str = "__all__";

#[derive(Clone, Debug)]
enum Device {
    Lan(LanDevice),
    Cloud(CloudDevice),
}

impl Device {
    fn id(&self) -> &str {
        match self {
            Device::Lan(d) => &d.id,
            Device::Cloud(d) => &d.id,
        }
    }
}

/// Normalised state.
#[derive(Clone, Copy, Debug, Default)]
struct St {
    on: bool,
    brightness: f64,
    kelvin: Option<f64>,
}

static CLOUD_LIST: Mutex<Option<(Instant, Vec<CloudDevice>)>> = Mutex::new(None);
static CLOUD_STATE: Mutex<Option<HashMap<String, (Instant, St)>>> = Mutex::new(None);

fn key() -> String {
    knobdeck_sdk::secrets::env_or_stored("GOVEE_API_KEY")
}

fn extra_addresses() -> Vec<String> {
    env_or("GOVEE_ADDRESSES", "").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

fn cloud_devices() -> Vec<CloudDevice> {
    let key = key();
    if key.is_empty() {
        return vec![];
    }
    let mut g = CLOUD_LIST.lock().unwrap();
    if let Some((at, list)) = g.as_ref() {
        if at.elapsed() < Duration::from_secs(600) {
            return list.clone();
        }
    }
    let list = cloud::devices(&key).unwrap_or_default();
    *g = Some((Instant::now(), list.clone()));
    list
}

/// Every device: LAN first (faster, no quota), then cloud devices not seen on the LAN.
fn devices(rescan: bool) -> Vec<(Device, String)> {
    let mut out = vec![];
    let cloud = cloud_devices();
    let name_of = |id: &str, fallback: &str| cloud.iter().find(|c| c.id == id).map(|c| c.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| fallback.to_string());
    if let Ok(l) = Lan::get() {
        let list = if rescan || l.known().is_empty() { l.scan(&extra_addresses(), Duration::from_millis(1500)) } else { l.known() };
        for d in list {
            let name = name_of(&d.id, &format!("{} ({})", d.sku, d.ip));
            out.push((Device::Lan(d), name));
        }
    }
    for c in &cloud {
        if !out.iter().any(|(d, _)| d.id() == c.id) {
            out.push((Device::Cloud(c.clone()), format!("{} (cloud)", if c.name.is_empty() { &c.sku } else { &c.name })));
        }
    }
    out
}

fn targets(inputs: &Value) -> Result<Vec<Device>> {
    let pick = inputs["device"].as_str().unwrap_or(ALL);
    let mut all: Vec<Device> = devices(false).into_iter().map(|(d, _)| d).collect();
    if !(pick.is_empty() || pick == ALL) {
        if !all.iter().any(|d| d.id() == pick) {
            all = devices(true).into_iter().map(|(d, _)| d).collect(); // it may have a new address
        }
        all.retain(|d| d.id() == pick);
    }
    if all.is_empty() {
        return Err(Error::msg(if pick.is_empty() || pick == ALL {
            "No Govee device found. Turn on LAN Control in the Govee Home app (device ▸ settings), or add your Govee API key in setup".to_string()
        } else {
            format!("Govee device {pick} not found (is it on, with LAN Control on?)")
        }));
    }
    Ok(all)
}

fn read(d: &Device) -> Result<St> {
    match d {
        Device::Lan(l) => {
            let v = Lan::get()?.status(&l.ip)?;
            let k = v["colorTemInKelvin"].as_f64().filter(|k| *k > 0.0);
            Ok(St { on: v["onOff"] == 1, brightness: v["brightness"].as_f64().unwrap_or(0.0), kelvin: k })
        }
        Device::Cloud(c) => {
            if let Some((at, st)) = CLOUD_STATE.lock().unwrap().as_ref().and_then(|m| m.get(&c.id)).copied() {
                if at.elapsed() < CLOUD_TTL {
                    return Ok(st);
                }
            }
            let v = cloud::state(&key(), c)?;
            let st = St { on: v["powerSwitch"] == 1, brightness: v["brightness"].as_f64().unwrap_or(0.0), kelvin: v["colorTemperatureK"].as_f64().filter(|k| *k > 0.0) };
            remember(c, st);
            Ok(st)
        }
    }
}

fn remember(c: &CloudDevice, st: St) {
    CLOUD_STATE.lock().unwrap().get_or_insert_with(HashMap::new).insert(c.id.clone(), (Instant::now(), st));
}

/// Update a cloud device's cached state after a change, so keys follow without another read.
fn patch(d: &Device, f: impl FnOnce(&mut St)) {
    if let Device::Cloud(c) = d {
        let mut st = CLOUD_STATE.lock().unwrap().as_ref().and_then(|m| m.get(&c.id)).map(|(_, s)| *s).unwrap_or_default();
        f(&mut st);
        remember(c, st);
    }
}

fn power(d: &Device, on: bool) -> Result<()> {
    match d {
        Device::Lan(l) => Lan::get()?.send(&l.ip, "turn", json!({"value": on as u8})),
        Device::Cloud(c) => cloud::control(&key(), c, "devices.capabilities.on_off", "powerSwitch", json!(on as u8)),
    }?;
    patch(d, |s| s.on = on);
    Ok(())
}

fn brightness(d: &Device, v: f64) -> Result<()> {
    let v = v.round().clamp(1.0, 100.0) as i64;
    match d {
        Device::Lan(l) => Lan::get()?.send(&l.ip, "brightness", json!({"value": v})),
        Device::Cloud(c) => cloud::control(&key(), c, "devices.capabilities.range", "brightness", json!(v)),
    }?;
    patch(d, |s| s.brightness = v as f64);
    Ok(())
}

fn color(d: &Device, (r, g, b): (u8, u8, u8)) -> Result<()> {
    match d {
        Device::Lan(l) => Lan::get()?.send(&l.ip, "colorwc", json!({"color": {"r": r, "g": g, "b": b}, "colorTemInKelvin": 0})),
        Device::Cloud(c) => cloud::control(&key(), c, "devices.capabilities.color_setting", "colorRgb", json!(cloud::rgb_int(r, g, b))),
    }?;
    patch(d, |s| s.kelvin = None);
    Ok(())
}

fn kelvin(d: &Device, k: f64) -> Result<()> {
    let k = k.round().clamp(2000.0, 9000.0) as i64;
    match d {
        Device::Lan(l) => Lan::get()?.send(&l.ip, "colorwc", json!({"color": {"r": 0, "g": 0, "b": 0}, "colorTemInKelvin": k})),
        Device::Cloud(c) => cloud::control(&key(), c, "devices.capabilities.color_setting", "colorTemperatureK", json!(k)),
    }?;
    patch(d, |s| s.kelvin = Some(k as f64));
    Ok(())
}

fn each(inputs: &Value, f: impl Fn(&Device) -> Result<()>) -> Result<()> {
    for d in targets(inputs)? {
        f(&d)?;
    }
    Ok(())
}

fn first(inputs: &Value) -> Result<St> {
    read(&targets(inputs)?[0])
}

pub fn hex(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some((c(0)?, c(2)?, c(4)?))
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

/// The cloud device a scene action targets (scenes exist only in the cloud API).
fn scene_device(inputs: &Value) -> Result<CloudDevice> {
    if key().is_empty() {
        return Err(Error::msg("Scenes need your Govee API key: add it in setup"));
    }
    let id = inputs["device"].as_str().unwrap_or("");
    cloud_devices().into_iter().find(|c| c.id == id).ok_or_else(|| Error::msg("Choose a device from your Govee account"))
}

fn main() {
    knobdeck_sdk::load_env();
    if std::env::var("KNOBDECK_CODEGEN").as_deref() != Ok("1") {
        std::thread::spawn(|| devices(true)); // warm the list so the first press doesn't wait
    }
    Plugin::new(&format!("govee-{}", std::env::consts::OS), "Govee")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Govee lights and light strips: power, brightness, color, color temperature and scenes. Local LAN control, or your Govee API key for other models.")
        .icon("lamp-ceiling")
        .setup("govee-setup", None)
        .setup_inputs(vec![
            input::text("api_key", "Govee API key (optional)").with("help", "Only for models without LAN Control, and for scenes. Get one in the Govee Home app ▸ Profile ▸ Settings ▸ Apply for API Key. Kept in this computer's credential store."),
            input::text("addresses", "Device addresses (optional)").placeholder("192.168.1.40").with("help", "Devices with LAN Control on are found automatically; type an address only for one that isn't."),
        ])
        .query("devices", |_| {
            let mut opts = vec![json!({"value": ALL, "label": "All devices"})];
            opts.extend(devices(true).into_iter().map(|(d, name)| json!({"value": d.id(), "label": name})));
            Ok(Value::Array(opts))
        })
        .query("cloud-devices", |q| {
            if key().is_empty() {
                return q.fail("Add your Govee API key in setup to use scenes");
            }
            Ok(Value::Array(cloud_devices().into_iter().map(|c| json!({"value": c.id, "label": if c.name.is_empty() { c.sku } else { c.name }})).collect()))
        })
        .query("scenes", |q| {
            let d = scene_device(&q.inputs).or_else(|e| q.fail(e.message().to_string()))?;
            let list = cloud::scenes(&key(), &d).or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(list.into_iter().map(|(n, _)| json!({"value": n, "label": n})).collect()))
        })
        .action(
            Action::toggle("power", "Power")
                .icon("lightbulb")
                .inputs(vec![device_input()])
                .state("off", "Off", "lightbulb-off")
                .state("on", "On", "lightbulb")
                .live(POLL, |ic| Ok(Visual::new().state(on_label(first(&ic.inputs)?.on))))
                .on_press(|ctx| {
                    let on = !first(&ctx.inputs)?.on;
                    each(&ctx.inputs, |d| power(d, on))?;
                    ctx.fan_out_by(&["device"], on_label(on));
                    Ok(())
                }),
        )
        .action(
            Action::slider("brightness", "Brightness", 1.0, 100.0)
                .icon("sun")
                .unit("%")
                .inputs(vec![device_input(), input::text("step", "Step (%)").placeholder("5").default_value("5")])
                .live(POLL, |ic| Ok(Visual::new().state(first(&ic.inputs)?.brightness)))
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => first(&ctx.inputs)?.brightness + step(ctx, 5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(1.0, 100.0);
                    each(&ctx.inputs, |d| brightness(d, target))?;
                    ctx.fan_out_by(&["device"], target);
                    Ok(())
                }),
        )
        .action(
            Action::slider("temperature", "Color Temperature", 2000.0, 9000.0)
                .icon("thermometer")
                .unit("K")
                .step(100.0)
                .inputs(vec![device_input(), input::text("step", "Step (K)").placeholder("200").default_value("200")])
                .live(POLL, |ic| {
                    Ok(match first(&ic.inputs)?.kelvin {
                        Some(k) => Visual::new().state(k),
                        None => Visual::new().label("Color"),
                    })
                })
                .on_adjust(|ctx| {
                    let target = match ctx.value {
                        Some(v) => v,
                        None => first(&ctx.inputs)?.kelvin.unwrap_or(4000.0) + step(ctx, 200.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .clamp(2000.0, 9000.0);
                    each(&ctx.inputs, |d| kelvin(d, target))?;
                    ctx.fan_out_by(&["device"], target.round());
                    Ok(())
                }),
        )
        .action(Action::button("set-color", "Set Color").icon("palette").inputs(vec![device_input(), input::text("color", "Color (#RRGGBB)").placeholder("#FF8800").default_value("#FF8800")]).on_press(|ctx| {
            let Some(rgb) = hex(&ctx.input_str("color")) else {
                return ctx.fail("Color must look like #FF8800");
            };
            each(&ctx.inputs, |d| color(d, rgb))
        }))
        .action(Action::button("set-brightness", "Set Brightness").icon("sun").inputs(vec![device_input(), input::text("value", "Brightness (%)").placeholder("50").default_value("50")]).on_press(|ctx| {
            let Some(v) = ctx.input_f64("value").filter(|v| (1.0..=100.0).contains(v)) else {
                return ctx.fail("Brightness must be 1–100");
            };
            each(&ctx.inputs, |d| brightness(d, v))
        }))
        .action(
            Action::button("scene", "Scene (API key)")
                .icon("sparkles")
                .inputs(vec![input::query_select("device", "Device", "cloud-devices", &[]), input::query_select("scene", "Scene", "scenes", &["device"])])
                .on_press(|ctx| {
                    let d = scene_device(&ctx.inputs)?;
                    let name = ctx.input_str("scene");
                    let value = cloud::scenes(&key(), &d)?.into_iter().find(|(n, _)| *n == name).map(|(_, v)| v).ok_or_else(|| Error::msg(format!("Scene {name} not found on this device")))?;
                    cloud::control(&key(), &d, "devices.capabilities.dynamic_scene", "lightScene", value)
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    #[test]
    fn colors() {
        assert_eq!(super::hex("#FF8800"), Some((255, 136, 0)));
        assert_eq!(super::hex("ff8800"), Some((255, 136, 0)));
        assert_eq!(super::hex("#F80"), None);
    }
}
