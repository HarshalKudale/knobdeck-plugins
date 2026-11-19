//! MQTT plugin for KnobDeck: publish messages, toggle and dim devices, and show a topic's value on
//! a key, through the user's own MQTT broker (Mosquitto, the Home Assistant add-on, EMQX…). Works
//! with Zigbee2MQTT, Tasmota, Shelly and anything else that speaks MQTT.

mod broker;

use broker::{Conn, Settings};
use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::os::placeholders;
use knobdeck_sdk::{env_or, load_env, Action, Ctx, Error, InitialCtx, Plugin, Result, Value, Visual};
use rumqttc::QoS;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const POLL: Duration = Duration::from_secs(1);

fn settings() -> Settings {
    let tls = env_or(broker::TLS_KEY, "0") == "1";
    Settings {
        host: env_or(broker::HOST_KEY, ""),
        port: env_or(broker::PORT_KEY, "").trim().parse().unwrap_or(broker::default_port(tls)),
        tls,
        ca_file: env_or(broker::CA_KEY, ""),
        username: env_or(broker::USER_KEY, ""),
        password: knobdeck_sdk::secrets::env_or_stored(broker::PASSWORD_KEY),
        client_id: env_or(broker::CLIENT_ID_KEY, ""),
    }
}

/// The session, started at first use.
fn conn() -> Result<Arc<Conn>> {
    static CONN: Mutex<Option<Arc<Conn>>> = Mutex::new(None);
    let mut slot = CONN.lock().unwrap();
    if let Some(c) = slot.as_ref() {
        return Ok(c.clone());
    }
    let c = Arc::new(Conn::start(&settings()).map_err(Error::msg)?);
    *slot = Some(c.clone());
    Ok(c)
}

fn qos(inputs: &Value) -> QoS {
    match inputs["qos"].as_str().unwrap_or("0") {
        "1" => QoS::AtLeastOnce,
        "2" => QoS::ExactlyOnce,
        _ => QoS::AtMostOnce,
    }
}

fn qos_input() -> Value {
    input::select("qos", "QoS", &[("0", "0: at most once"), ("1", "1: at least once"), ("2", "2: exactly once")]).default_value("0")
}

fn retain_input() -> Value {
    input::checkbox("retain", "Retain (the broker keeps the last message for new subscribers)").default_value(false)
}

/// Placeholders expanded. Text without any is sent exactly as typed: the parser reads `}}` as an
/// escaped brace, which would break nested JSON such as `{"color":{"hex":"#FF0000"}}`.
fn fill(ctx: &Ctx, text: &str) -> Result<String> {
    if placeholders::used(text).is_empty() {
        return Ok(text.to_string());
    }
    ctx.expand(text)
}

const BRACES_HELP: &str = " With a placeholder in it, write {{ and }} for literal braces.";

fn publish(ctx: &Ctx, topic_key: &str, payload: &str) -> Result<()> {
    let topic = fill(ctx, ctx.input_str(topic_key).trim())?;
    conn()?.publish(&topic, payload, qos(&ctx.inputs), ctx.input_bool("retain")).map_err(Error::msg)?;
    ctx.log(format!("published to {topic}: {}", one_line(payload, 120)));
    Ok(())
}

fn one_line(s: &str, max: usize) -> String {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max).collect::<String>())
    } else {
        s
    }
}

/// The current value on a state topic (subscribing to it the first time), `None` until a message
/// arrived. Errors only for a bad topic or JSON path, or a broker that can't be reached.
fn read_state(inputs: &Value, topic_key: &str, path_key: &str) -> Result<Option<String>> {
    let topic = inputs[topic_key].as_str().unwrap_or("").trim().to_string();
    let c = conn()?;
    if c.watch(&topic).map_err(Error::msg)? {
        // Just subscribed: give the broker a moment to send the retained state, so a first
        // press toggles from the device's real state.
        if c.wait_connected(Duration::from_secs(3)).is_ok() {
            c.wait_message(&topic, Duration::from_millis(700));
        }
    }
    match c.latest(&topic) {
        Some(p) => Ok(Some(broker::extract(&p, inputs[path_key].as_str().unwrap_or("")).map_err(Error::msg)?)),
        None if !c.is_connected() => Err(Error::msg(c.last_error().map(|e| format!("MQTT broker: {e}")).unwrap_or_else(|| "Connecting to the MQTT broker…".into()))),
        None => Ok(None),
    }
}

fn str_of<'a>(inputs: &'a Value, key: &str) -> &'a str {
    inputs[key].as_str().unwrap_or("")
}

fn has_state_topic(inputs: &Value) -> bool {
    !str_of(inputs, "state_topic").trim().is_empty()
}

fn toggle_on(ic: &InitialCtx) -> Result<Option<bool>> {
    if !has_state_topic(&ic.inputs) {
        return Ok(None);
    }
    Ok(read_state(&ic.inputs, "state_topic", "state_path")?.map(|v| broker::is_on(&v, str_of(&ic.inputs, "on_value"), str_of(&ic.inputs, "on_payload"))))
}

/// Dial range from the key's inputs.
fn range(inputs: &Value) -> (f64, f64) {
    let num = |k: &str, d: f64| str_of(inputs, k).trim().parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(d);
    (num("min", 0.0), num("max", 100.0))
}

fn decimals(inputs: &Value) -> usize {
    str_of(inputs, "decimals").trim().parse::<usize>().unwrap_or(0).min(6)
}

fn to_pct(v: f64, (min, max): (f64, f64)) -> f64 {
    if max == min {
        0.0
    } else {
        ((v - min) / (max - min) * 100.0).clamp(0.0, 100.0)
    }
}

fn from_pct(p: f64, (min, max): (f64, f64)) -> f64 {
    min + (max - min) * p.clamp(0.0, 100.0) / 100.0
}

fn format_number(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s == "-0" {
        "0".into()
    } else {
        s
    }
}

/// The dial's value on the state topic, as a percentage of its range.
fn dial_pct(ic: &InitialCtx) -> Result<Option<f64>> {
    if !has_state_topic(&ic.inputs) {
        return Ok(None);
    }
    let Some(v) = read_state(&ic.inputs, "state_topic", "state_path")? else { return Ok(None) };
    let n: f64 = v.trim().parse().map_err(|_| Error::msg(format!("The state isn't a number: {}", one_line(&v, 40))))?;
    Ok(Some(to_pct(n, range(&ic.inputs))))
}

fn show_value(ic: &InitialCtx) -> Result<Visual> {
    let shown = read_state(&ic.inputs, "topic", "path")?;
    let title = str_of(&ic.inputs, "title").trim();
    let text = match shown {
        Some(v) => format!("{}{}{}", str_of(&ic.inputs, "prefix"), one_line(&v, 40), str_of(&ic.inputs, "suffix")),
        None => "—".into(),
    };
    Ok(Visual::new().label(if title.is_empty() { text } else { format!("{title}\n{text}") }))
}

fn main() {
    load_env();
    Plugin::new(&format!("mqtt-{}", std::env::consts::OS), "MQTT")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Publish messages, toggle and dim devices, and show a topic's value on a key through your MQTT broker. Works with Zigbee2MQTT, Tasmota and Shelly.")
        .icon("app-icon.svg")
        .setup("mqtt-setup", None)
        .setup_inputs(vec![
            input::text("host", "Broker address").placeholder("192.168.1.10 or mqtt.local"),
            input::text("port", "Port (optional)").placeholder("1883, or 8883 with TLS"),
            input::checkbox("tls", "Use TLS (mqtts)").default_value(false),
            input::text("ca_file", "CA certificate file (optional, TLS)").placeholder("/path/to/ca.crt").with("help", "Only for a broker with a self-signed certificate: the PEM file of the CA that signed it. Otherwise this computer's trusted certificates are used."),
            input::text("username", "Username (optional)"),
            input::text("password", "Password (optional)").with("type", "password").with("help", "Kept in this computer's credential store. Leave empty if the broker needs none."),
            input::text("client_id", "Client id (optional)").placeholder("knobdeck-desk").with("help", "Leave empty for a random one. Two clients with the same id knock each other off the broker."),
        ])
        .action(
            Action::button("publish", "Publish")
                .icon("send")
                .inputs(vec![
                    input::text("topic", "Topic").placeholder("cmnd/tasmota_desk/POWER").accepts_placeholders(),
                    input::textarea("payload", "Message").placeholder("TOGGLE").accepts_placeholders().with("help", format!("Sent as it is: text, a number or JSON, e.g. {{\"state\":\"TOGGLE\"}} for Zigbee2MQTT.{BRACES_HELP}")),
                    qos_input(),
                    retain_input(),
                ])
                .on_press(|ctx| {
                    let payload = fill(ctx, &ctx.input_str("payload"))?;
                    publish(ctx, "topic", &payload)?;
                    ctx.set_icon("check");
                    let (rt, id) = (ctx.runtime().clone(), ctx.instance_id.clone());
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(1000));
                        rt.set_visual(&id, Visual::new().icon("send"));
                    });
                    Ok(())
                }),
        )
        .action(
            Action::toggle("toggle", "Toggle")
                .icon("toggle-right")
                .state("off", "Off", "toggle-left")
                .state("on", "On", "toggle-right")
                .inputs(vec![
                    input::text("topic", "Command topic").placeholder("zigbee2mqtt/desk_lamp/set").accepts_placeholders(),
                    input::text("on_payload", "On message").placeholder("{\"state\":\"ON\"}").default_value("ON").accepts_placeholders(),
                    input::text("off_payload", "Off message").placeholder("{\"state\":\"OFF\"}").default_value("OFF").accepts_placeholders(),
                    input::text("state_topic", "State topic (optional)").placeholder("zigbee2mqtt/desk_lamp").with("help", "Where the device reports whether it is on: zigbee2mqtt/<name>, stat/<tasmota>/POWER, shellies/<id>/relay/0. Without it the key just alternates."),
                    input::text("state_path", "State JSON path (optional)").placeholder("state").with("help", "For a JSON state message: where the on/off value is (Zigbee2MQTT: state, Tasmota's STATE topic: POWER)."),
                    input::text("on_value", "On when the state is (optional)").placeholder("ON").with("help", "Leave empty: ON, on, true and 1 count as on."),
                    qos_input(),
                    retain_input(),
                ])
                .live(POLL, |ic| Ok(match toggle_on(ic)? {
                    Some(on) => Visual::new().state(if on { "on" } else { "off" }),
                    None => Visual::new(),
                }))
                .on_press(|ctx| {
                    let ic = InitialCtx { instance_id: ctx.instance_id.clone(), action_id: ctx.action_id.clone(), inputs: ctx.inputs.clone() };
                    let on = toggle_on(&ic).ok().flatten().unwrap_or(ctx.state.as_deref() == Some("on"));
                    let payload = fill(ctx, &ctx.input_str(if on { "off_payload" } else { "on_payload" }))?;
                    publish(ctx, "topic", &payload)?;
                    ctx.set_state(if on { "off" } else { "on" });
                    Ok(())
                }),
        )
        .action(
            Action::button("value", "Topic Value")
                .icon("radio-receiver")
                .inputs(vec![
                    input::text("topic", "Topic").placeholder("zigbee2mqtt/living_room_sensor").with("help", "+ and # work: the key shows the newest message on any matching topic."),
                    input::text("path", "JSON path (optional)").placeholder("temperature").with("help", "Where the value is in a JSON message: names joined by dots, [0] for a list's first item. Empty: the whole message."),
                    input::text("title", "Title (optional)").placeholder("Living room"),
                    input::text("prefix", "Before the value (optional)"),
                    input::text("suffix", "After the value (optional)").placeholder(" °C"),
                ])
                .live(POLL, show_value),
        )
        .action(
            Action::slider("dial", "Value Dial", 0.0, 100.0)
                .icon("sliders-horizontal")
                .unit("%")
                .inputs(vec![
                    input::text("topic", "Command topic").placeholder("zigbee2mqtt/desk_lamp/set").accepts_placeholders(),
                    input::text("payload", "Message").placeholder("{\"brightness\": {value}}").default_value("{value}").accepts_placeholders().with("help", format!("{{value}} is the dial's number. Zigbee2MQTT: {{\"brightness\": {{value}}}} (0–254); Tasmota (cmnd/<name>/Dimmer): {{value}}.{BRACES_HELP}")),
                    input::text("min", "From").placeholder("0").default_value("0"),
                    input::text("max", "To").placeholder("100").default_value("100"),
                    input::text("decimals", "Decimals").placeholder("0").default_value("0"),
                    input::text("step", "Step (% of the range)").placeholder("5").default_value("5"),
                    input::text("state_topic", "State topic (optional)").placeholder("zigbee2mqtt/desk_lamp").with("help", "Where the device reports the current value, so the dial starts from it."),
                    input::text("state_path", "State JSON path (optional)").placeholder("brightness"),
                    qos_input(),
                    retain_input(),
                ])
                .live(POLL, |ic| {
                    let Some(p) = dial_pct(ic)? else { return Ok(Visual::new()) };
                    let v = from_pct(p, range(&ic.inputs));
                    Ok(Visual::new().state(p.round()).label(format_number(v, decimals(&ic.inputs))))
                })
                .on_adjust(|ctx| {
                    let rng = range(&ctx.inputs);
                    let pct = match ctx.value {
                        Some(v) => v,
                        None => {
                            let ic = InitialCtx { instance_id: ctx.instance_id.clone(), action_id: ctx.action_id.clone(), inputs: ctx.inputs.clone() };
                            let now = dial_pct(&ic).ok().flatten().or_else(|| ctx.state.as_deref().and_then(|s| s.parse().ok())).unwrap_or(0.0);
                            let step = ctx.input_f64("step").filter(|s| *s > 0.0).unwrap_or(5.0);
                            now + step * ctx.delta.unwrap_or(0.0)
                        }
                    }
                    .clamp(0.0, 100.0);
                    let dec = decimals(&ctx.inputs);
                    let number = format_number(from_pct(pct, rng), dec);
                    // {value} first (it isn't a placeholder; a number has no braces to escape).
                    let payload = fill(ctx, &ctx.input_str("payload").replace("{value}", &number))?;
                    publish(ctx, "topic", &payload)?;
                    ctx.set_visual(Visual::new().state(pct.round()).label(number));
                    Ok(())
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::TcpListener;

    #[test]
    fn dial_math() {
        assert_eq!(to_pct(127.0, (0.0, 254.0)), 50.0);
        assert_eq!(from_pct(50.0, (0.0, 254.0)), 127.0);
        assert_eq!(format_number(from_pct(33.0, (0.0, 254.0)), 0), "84");
        assert_eq!(format_number(from_pct(50.0, (15.0, 30.0)), 1), "22.5");
        assert_eq!(to_pct(5.0, (5.0, 5.0)), 0.0);
    }

    /// Start rumqttd (a real MQTT broker) on a free local port, with one user.
    fn broker() -> u16 {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let conn = rumqttd::ConnectionSettings {
            connection_timeout_ms: 5000,
            max_payload_size: 64 * 1024,
            max_inflight_count: 100,
            auth: Some(HashMap::from([("knob".to_string(), "s3cret".to_string())])),
            external_auth: None,
            dynamic_filters: true,
        };
        let server = rumqttd::ServerSettings { name: "v4".into(), listen: ([127, 0, 0, 1], port).into(), tls: None, next_connection_delay_ms: 1, connections: conn };
        let config = rumqttd::Config {
            id: 0,
            router: rumqttd::RouterConfig { max_connections: 100, max_outgoing_packet_count: 200, max_segment_size: 1 << 20, max_segment_count: 10, custom_segment: None, initialized_filters: None, shared_subscriptions_strategy: Default::default() },
            v4: Some(HashMap::from([("1".to_string(), server)])),
            v5: None,
            ws: None,
            cluster: None,
            console: None,
            bridge: None,
            prometheus: None,
            metrics: None,
        };
        std::thread::spawn(move || rumqttd::Broker::new(config).start().unwrap());
        for _ in 0..50 {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        port
    }

    fn settings_for(port: u16, user: &str, pass: &str) -> Settings {
        Settings { host: "127.0.0.1".into(), port, username: user.into(), password: pass.into(), ..Default::default() }
    }

    #[test]
    fn against_a_real_broker() {
        let port = broker();
        let device = Conn::start(&settings_for(port, "knob", "s3cret")).unwrap();
        device.wait_connected(Duration::from_secs(5)).unwrap();
        device.watch("zigbee2mqtt/lamp/set").unwrap();

        let keys = Conn::start(&settings_for(port, "knob", "s3cret")).unwrap();
        keys.wait_connected(Duration::from_secs(5)).unwrap();
        keys.watch("zigbee2mqtt/lamp").unwrap();
        keys.watch("tele/+/SENSOR").unwrap();
        std::thread::sleep(Duration::from_millis(300)); // SUBACKs

        // A key publishes a command; the "device" sees it.
        keys.publish("zigbee2mqtt/lamp/set", r#"{"state":"ON"}"#, QoS::AtLeastOnce, false).unwrap();
        assert_eq!(device.wait_message("zigbee2mqtt/lamp/set", Duration::from_secs(5)).as_deref(), Some(r#"{"state":"ON"}"#));

        // The device reports its state; the toggle reads it through the JSON path.
        device.publish("zigbee2mqtt/lamp", r#"{"state":"ON","brightness":200}"#, QoS::AtMostOnce, false).unwrap();
        let st = keys.wait_message("zigbee2mqtt/lamp", Duration::from_secs(5)).unwrap();
        assert!(broker::is_on(&broker::extract(&st, "state").unwrap(), "", "ON"));
        assert_eq!(broker::extract(&keys.latest("zigbee2mqtt/lamp").unwrap(), "brightness").unwrap(), "200");

        // A wildcard state tile shows the newest matching message.
        device.publish("tele/kitchen/SENSOR", r#"{"AM2301":{"Temperature":21.4}}"#, QoS::AtMostOnce, false).unwrap();
        keys.wait_message("tele/+/SENSOR", Duration::from_secs(5)).unwrap();
        assert_eq!(broker::extract(&keys.latest("tele/+/SENSOR").unwrap(), "AM2301.Temperature").unwrap(), "21.4");

        // Wrong password: refused with a clear reason.
        let bad = Conn::start(&settings_for(port, "knob", "nope")).unwrap();
        let err = bad.wait_connected(Duration::from_secs(4)).unwrap_err();
        assert!(err.contains("Can't reach the MQTT broker"), "{err}");
        assert!(bad.publish("a/b", "x", QoS::AtMostOnce, false).is_err());
    }

    #[test]
    fn nothing_listening() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let c = Conn::start(&settings_for(port, "", "")).unwrap();
        let err = c.wait_connected(Duration::from_secs(3)).unwrap_err();
        assert!(err.contains("127.0.0.1"), "{err}");
        assert!(Conn::start(&Settings::default()).is_err());
    }
}
