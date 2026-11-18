//! VTube Studio setup: where its API listens. The token comes from VTube Studio's own Allow popup
//! the first time the plugin connects, and the plugin keeps it in the OS credential store.

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let get = |key: &str, default: &str| match ctx.inputs.get(key) {
            None | Some(knobdeck_sdk::Value::Null) => default.to_string(),
            Some(_) => ctx.input_str(key).trim().to_string(),
        };
        let (host, port) = (get("host", "127.0.0.1"), get("port", "8001"));
        if host.is_empty() {
            return ctx.fail("Host cannot be empty");
        }
        if !matches!(port.parse::<u32>(), Ok(1..=65535)) {
            return ctx.fail("Invalid port number");
        }
        knobdeck_sdk::save_env(&[("VTUBE_STUDIO_HOST", &host), ("VTUBE_STUDIO_PORT", &port)])?;
        ctx.log(format!("[setup] VTube Studio: ws://{host}:{port}/"));
        Ok(())
    })
}
