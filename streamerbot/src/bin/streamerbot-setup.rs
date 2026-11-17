//! Streamer.bot setup: save where its WebSocket server listens (password in the OS credential store).

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let get = |key: &str, default: &str| match ctx.inputs.get(key) {
            None | Some(knobdeck_sdk::Value::Null) => default.to_string(),
            Some(_) => ctx.input_str(key).trim().to_string(),
        };
        let (host, port, password) = (get("host", "127.0.0.1"), get("port", "8080"), get("password", ""));
        if host.is_empty() {
            return ctx.fail("Host cannot be empty");
        }
        if !matches!(port.parse::<u32>(), Ok(1..=65535)) {
            return ctx.fail("Invalid port number");
        }
        knobdeck_sdk::save_env(&[("STREAMERBOT_HOST", &host), ("STREAMERBOT_PORT", &port)])?;
        knobdeck_sdk::secrets::set("STREAMERBOT_PASSWORD", &password)?;
        ctx.log(format!("[setup] Streamer.bot: ws://{host}:{port}/"));
        Ok(())
    })
}
