//! A local MQTT broker (rumqttd) for trying the plugin with `knobdeck-plugin drive`, plus a fake
//! Zigbee2MQTT lamp: it prints every message on `#` and answers `zigbee2mqtt/lamp/set` with the
//! new state on `zigbee2mqtt/lamp`, like Zigbee2MQTT does.
//!
//!   cargo run -p mqtt --example test-broker -- 18830
//!   MQTT_HOST=127.0.0.1 MQTT_PORT=18830 knobdeck-plugin drive steps.json

use rumqttc::{Client, Event, MqttOptions, Packet, QoS};
use std::collections::HashMap;
use std::time::Duration;

fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(18830);
    let conn = rumqttd::ConnectionSettings { connection_timeout_ms: 5000, max_payload_size: 64 * 1024, max_inflight_count: 100, auth: None, external_auth: None, dynamic_filters: true };
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
    std::thread::sleep(Duration::from_millis(500));

    let (client, mut connection) = Client::new(MqttOptions::new("fake-lamp", "127.0.0.1", port), 16);
    client.subscribe("#", QoS::AtMostOnce).unwrap();
    let mut lamp = serde_json::json!({"state": "ON", "brightness": 127});
    let state = |c: &Client, l: &serde_json::Value| c.publish("zigbee2mqtt/lamp", QoS::AtMostOnce, true, l.to_string()).unwrap();
    state(&client, &lamp);
    println!("broker on 127.0.0.1:{port}");
    for ev in connection.iter() {
        if let Ok(Event::Incoming(Packet::Publish(p))) = ev {
            let text = String::from_utf8_lossy(&p.payload).into_owned();
            println!("{} {}", p.topic, text);
            if p.topic == "zigbee2mqtt/lamp/set" {
                if let Ok(serde_json::Value::Object(m)) = serde_json::from_str(&text) {
                    for (k, v) in m {
                        lamp[k] = v;
                    }
                    state(&client, &lamp);
                }
            }
        }
    }
}
