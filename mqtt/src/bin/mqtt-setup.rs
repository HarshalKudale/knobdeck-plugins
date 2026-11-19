//! MQTT setup: the broker's address, TLS and login. Connects once with them before saving, so a
//! wrong address or password shows here. The password goes to the credential store.

#[allow(dead_code)]
#[path = "../broker.rs"]
mod broker;

use std::time::Duration;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let host = ctx.input_str("host").trim().to_string();
        let tls = ctx.input_bool("tls");
        let port_text = ctx.input_str("port").trim().to_string();
        if host.is_empty() {
            return ctx.fail("Enter the broker's address (the computer or device that runs Mosquitto, the Home Assistant add-on…)");
        }
        let port = if port_text.is_empty() {
            broker::default_port(tls)
        } else {
            match port_text.parse::<u16>() {
                Ok(p) if p > 0 => p,
                _ => return ctx.fail("Invalid port number"),
            }
        };
        let s = broker::Settings {
            host,
            port,
            tls,
            ca_file: ctx.input_str("ca_file").trim().to_string(),
            username: ctx.input_str("username").trim().to_string(),
            password: ctx.input_str("password"),
            client_id: ctx.input_str("client_id").trim().to_string(),
        };
        if !s.password.is_empty() && s.username.is_empty() {
            return ctx.fail("A password needs a username");
        }
        let conn = match broker::Conn::start(&s) {
            Ok(c) => c,
            Err(e) => return ctx.fail(e),
        };
        if let Err(e) = conn.wait_connected(Duration::from_secs(8)) {
            return ctx.fail(e);
        }
        drop(conn);
        ctx.log(format!("[setup] connected to {}", s.describe()));
        knobdeck_sdk::save_env(&[
            (broker::HOST_KEY, s.host.as_str()),
            (broker::PORT_KEY, port_text.as_str()),
            (broker::TLS_KEY, if tls { "1" } else { "0" }),
            (broker::CA_KEY, s.ca_file.as_str()),
            (broker::USER_KEY, s.username.as_str()),
            (broker::CLIENT_ID_KEY, s.client_id.as_str()),
        ])?;
        // An empty password removes one saved before.
        knobdeck_sdk::secrets::set(broker::PASSWORD_KEY, &s.password)?;
        Ok(())
    })
}
