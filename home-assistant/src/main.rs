//! Home Assistant plugin for KnobDeck — toggle and watch entities, run scenes and scripts, set
//! levels on a dial and call any service, over Home Assistant's REST API with a long-lived token.

mod ha;

use ha::Conn;
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, load_env, Action, Ctx, Error, Plugin, Result, Value, Visual};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(2);

fn conn() -> Result<Conn> {
    Conn::from_env()
}

/// A refused token stops the plugin and asks for setup; every other error just shows on the key.
fn guard<T>(r: Result<T>) -> Result<T> {
    if let Err(e) = &r {
        if e.message().contains("refused the access token") {
            knobdeck_sdk::mark_setup_required(e.message());
        }
    }
    r
}

fn entity_of(inputs: &Value) -> Result<String> {
    match inputs["entity"].as_str().map(str::trim) {
        Some(e) if !e.is_empty() => Ok(e.to_string()),
        _ => Err(Error::msg("Choose an entity")),
    }
}

/// Entities of the given domains, as `query_select` options sorted by name.
fn entity_options(domains: Option<&[&str]>) -> Result<Value> {
    let c = guard(conn())?;
    let mut opts: Vec<(String, String)> = guard(ha::states(&c))?
        .iter()
        .filter_map(|s| {
            let id = s["entity_id"].as_str()?;
            domains.is_none_or(|d| d.contains(&ha::domain(id))).then(|| (id.to_string(), format!("{} ({id})", ha::friendly_name(s))))
        })
        .collect();
    opts.sort_by_key(|(_, label)| label.to_lowercase());
    Ok(Value::Array(opts.into_iter().map(|(v, l)| json!({"value": v, "label": l})).collect()))
}

fn toggle_visual(s: &Value, show_name: bool) -> Visual {
    let id = s["entity_id"].as_str().unwrap_or("");
    let on = ha::is_on(s);
    let v = Visual::new().state(if on { "on" } else { "off" }).icon(ha::icon_for(id, on));
    if show_name {
        v.label(ha::friendly_name(s))
    } else {
        v
    }
}

fn press_toggle(c: &Conn, entity: &str) -> Result<Value> {
    let s = ha::state(c, entity)?;
    let (domain, service) = ha::toggle_service(entity, &s);
    c.call(domain, service, &json!({"entity_id": entity}))?;
    ha::invalidate();
    Ok(s)
}

fn press_activate(c: &Conn, entity: &str) -> Result<()> {
    let (domain, service) = ha::activate_service(entity);
    c.call(domain, service, &json!({"entity_id": entity}))
}

/// "light.turn_on" → ("light", "turn_on").
fn parse_service(text: &str) -> Result<(String, String)> {
    let t = text.trim();
    match t.split_once('.') {
        Some((d, s)) if !d.is_empty() && !s.is_empty() && !s.contains('.') => Ok((d.to_string(), s.to_string())),
        _ => Err(Error::msg("Service must look like light.turn_on")),
    }
}

fn service_data(ctx: &Ctx) -> Result<Value> {
    let raw = ctx.input_str("data");
    let mut data = if raw.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str::<Value>(&raw) {
            Ok(v @ Value::Object(_)) => v,
            _ => return Err(Error::msg("Service data must be a JSON object, like {\"brightness_pct\": 50}")),
        }
    };
    let entity = ctx.input_str("entity");
    if !entity.trim().is_empty() && data.get("entity_id").is_none() {
        data["entity_id"] = json!(entity.trim());
    }
    Ok(data)
}

/// Domains the Entities wheel can show (value, label).
const WHEEL_DOMAINS: &[(&str, &str)] = &[
    ("light", "Lights"),
    ("switch", "Switches"),
    ("scene", "Scenes"),
    ("script", "Scripts"),
    ("automation", "Automations"),
    ("fan", "Fans"),
    ("cover", "Covers"),
    ("lock", "Locks"),
    ("media_player", "Media players"),
    ("input_boolean", "Input booleans"),
    ("button", "Buttons"),
];

fn main() {
    load_env();
    Plugin::new(&format!("home-assistant-{}", std::env::consts::OS), "Home Assistant")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Toggle lights and switches, run scenes and scripts, show sensors and set levels in Home Assistant.")
        .icon("house")
        .setup("home-assistant-setup", None)
        .setup_inputs(vec![
            input::text("url", "Home Assistant URL").placeholder("http://homeassistant.local:8123").default_value("http://homeassistant.local:8123"),
            input::text("token", "Long-lived access token").with("help", "In Home Assistant, open your profile ▸ Security ▸ Long-lived access tokens ▸ Create token, and paste it here."),
        ])
        .query("toggle-entities", |_| entity_options(Some(ha::TOGGLE)))
        .query("activate-entities", |_| entity_options(Some(ha::ACTIVATE)))
        .query("level-entities", |_| entity_options(Some(ha::LEVEL)))
        .query("all-entities", |_| entity_options(None))
        .query("services", |_| {
            let c = guard(conn())?;
            let mut opts: Vec<String> = guard(c.get("services"))?
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|d| {
                    let domain = d["domain"].as_str().unwrap_or("").to_string();
                    d["services"].as_object().into_iter().flat_map(|m| m.keys()).map(move |s| format!("{domain}.{s}")).collect::<Vec<_>>()
                })
                .collect();
            opts.sort();
            Ok(Value::Array(opts.into_iter().map(|s| json!({"value": s, "label": s})).collect()))
        })
        .action(
            Action::toggle("toggle", "Toggle Entity")
                .icon("toggle-right")
                .inputs(vec![input::query_select("entity", "Entity", "toggle-entities", &[]), input::checkbox("show_name", "Show the entity's name").default_value(true)])
                .state("off", "Off", "toggle-left")
                .state("on", "On", "toggle-right")
                .live(POLL, |ic| {
                    let c = guard(conn())?;
                    Ok(toggle_visual(&guard(ha::state(&c, &entity_of(&ic.inputs)?))?, ic.input_bool("show_name")))
                })
                .on_press(|ctx| {
                    let c = guard(conn())?;
                    let entity = entity_of(&ctx.inputs)?;
                    let before = guard(press_toggle(&c, &entity))?;
                    // Show the expected state at once; the next poll corrects it.
                    ctx.fan_out_by(&["entity"], if ha::is_on(&before) { "off" } else { "on" });
                    Ok(())
                }),
        )
        .action(Action::button("activate", "Run Scene / Script").icon("sparkles").inputs(vec![input::query_select("entity", "Scene, script, automation or button", "activate-entities", &[])]).on_press(|ctx| {
            let c = guard(conn())?;
            guard(press_activate(&c, &entity_of(&ctx.inputs)?))
        }))
        .action(
            Action::button("state", "Show State")
                .icon("gauge")
                .inputs(vec![
                    input::query_select("entity", "Entity", "all-entities", &[]),
                    input::text("attribute", "Attribute (optional)").placeholder("temperature").with("help", "Show this attribute instead of the state."),
                    input::checkbox("show_name", "Show the entity's name").default_value(false),
                ])
                .live(Duration::from_secs(5), |ic| {
                    let c = guard(conn())?;
                    let s = guard(ha::state(&c, &entity_of(&ic.inputs)?))?;
                    let text = ha::display(&s, &ic.input_str("attribute"));
                    let label = if ic.input_bool("show_name") { format!("{}\n{text}", ha::friendly_name(&s)) } else { text };
                    let id = s["entity_id"].as_str().unwrap_or("");
                    Ok(Visual::new().label(label).icon(ha::icon_for(id, ha::is_on(&s))))
                })
                .on_press(|_| {
                    ha::invalidate();
                    Ok(())
                }),
        )
        .action(
            Action::slider("level", "Level", 0.0, 100.0)
                .icon("sliders-horizontal")
                .unit("%")
                .inputs(vec![
                    input::query_select("entity", "Light, cover, fan, media player or number", "level-entities", &[]),
                    input::text("step", "Step (%)").placeholder("5").default_value("5"),
                ])
                .live(POLL, |ic| {
                    let c = guard(conn())?;
                    let s = guard(ha::state(&c, &entity_of(&ic.inputs)?))?;
                    Ok(match ha::level_of(&s) {
                        Some(v) => Visual::new().state(v),
                        None => Visual::new().label("No level"),
                    })
                })
                .on_adjust(|ctx| {
                    let c = guard(conn())?;
                    let s = guard(ha::state(&c, &entity_of(&ctx.inputs)?))?;
                    let target = match ctx.value {
                        Some(v) => v,
                        None => ha::level_of(&s).unwrap_or(0.0) + ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(5.0) * ctx.delta.unwrap_or(0.0),
                    }
                    .round()
                    .clamp(0.0, 100.0);
                    let (domain, service, data) = ha::level_call(&s, target)?;
                    guard(c.call(domain, service, &data))?;
                    ha::invalidate();
                    ctx.fan_out_by(&["entity"], target);
                    Ok(())
                }),
        )
        .action(
            Action::button("call-service", "Call Service")
                .icon("zap")
                .inputs(vec![
                    input::query_combobox("service", "Service", "services").placeholder("light.turn_on"),
                    input::query_combobox("entity", "Entity (optional)", "all-entities"),
                    input::textarea("data", "Service data (JSON, optional)").placeholder("{\"brightness_pct\": 60, \"color_name\": \"orange\"}"),
                ])
                .on_press(|ctx| {
                    let c = guard(conn())?;
                    let (domain, service) = parse_service(&ctx.input_str("service"))?;
                    guard(c.call(&domain, &service, &service_data(ctx)?))?;
                    ha::invalidate();
                    Ok(())
                }),
        )
        .action(
            Action::wheel("entities-wheel", "Entities Wheel")
                .icon("layout-grid")
                .inputs(vec![
                    input::select("domain", "Show", WHEEL_DOMAINS).with("default", "light"),
                    input::text("filter", "Name contains (optional)").placeholder("Living room"),
                ])
                .on_open(|o| {
                    let c = guard(conn()).or_else(|e| o.fail(e.message().to_string()))?;
                    let domain = o.input_str("domain");
                    let domain = if domain.is_empty() { "light".to_string() } else { domain };
                    let filter = o.input_str("filter").trim().to_lowercase();
                    let mut list: Vec<Value> = guard(ha::states(&c))
                        .or_else(|e| o.fail(e.message().to_string()))?
                        .into_iter()
                        .filter(|s| s["entity_id"].as_str().is_some_and(|id| ha::domain(id) == domain))
                        .filter(|s| filter.is_empty() || ha::friendly_name(s).to_lowercase().contains(&filter))
                        .collect();
                    list.sort_by_key(|s| ha::friendly_name(s).to_lowercase());
                    Ok(list
                        .iter()
                        .take(16)
                        .map(|s| {
                            let id = s["entity_id"].as_str().unwrap_or("");
                            let on = ha::is_on(s);
                            let name = ha::friendly_name(s);
                            let label = if ha::ACTIVATE.contains(&ha::domain(id)) && ha::domain(id) != "automation" { name } else { format!("{name} · {}", if on { "on" } else { "off" }) };
                            json!({"id": id, "label": label, "icon": ha::icon_for(id, on)})
                        })
                        .collect())
                })
                .on_child_press(|ctx| {
                    let c = guard(conn())?;
                    let Some(entity) = ctx.child_id.clone() else { return Ok(()) };
                    if ha::ACTIVATE.contains(&ha::domain(&entity)) && ha::domain(&entity) != "automation" {
                        guard(press_activate(&c, &entity))
                    } else {
                        guard(press_toggle(&c, &entity)).map(|_| ())
                    }
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services() {
        assert_eq!(parse_service(" light.turn_on ").unwrap(), ("light".into(), "turn_on".into()));
        assert!(parse_service("light").is_err());
        assert!(parse_service("a.b.c").is_err());
        assert!(parse_service(".x").is_err());
    }
}
