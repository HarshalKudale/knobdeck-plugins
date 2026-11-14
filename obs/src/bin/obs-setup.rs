//! OBS setup: save the connection details collected by the setup page (password in the OS credential store).

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        // JS `inputs.x ?? default`: only a missing/null field takes the default.
        let get = |key: &str, default: &str| match ctx.inputs.get(key) {
            None | Some(knobdeck_sdk::Value::Null) => default.to_string(),
            Some(_) => ctx.input_str(key).trim().to_string(),
        };
        let (host, port, password) = (get("host", "localhost"), get("port", "4455"), get("password", ""));

        if host.is_empty() {
            return ctx.fail("OBS host cannot be empty");
        }
        match port.parse::<u32>() {
            Ok(1..=65535) => {}
            _ => return ctx.fail("Invalid port number"),
        }

        // Host and port are settings; the password goes to the OS credential store.
        knobdeck_sdk::save_env(&[("OBS_HOST", &host), ("OBS_PORT", &port)])?;
        knobdeck_sdk::secrets::set("OBS_PASSWORD", &password)?;
        ctx.log(format!("[setup] OBS connection configured: ws://{host}:{port}"));
        Ok(())
    })
}
