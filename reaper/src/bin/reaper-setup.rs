//! REAPER setup: check that the web browser interface answers at the host and port (with the
//! login, if one is set), then save them; the password goes to the OS credential store.

// Shared with the plugin; setup only checks the connection.
#[allow(dead_code)]
#[path = "../reaper.rs"]
mod reaper;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let get = |key: &str, default: &str| match ctx.inputs.get(key) {
            None | Some(knobdeck_sdk::Value::Null) => default.to_string(),
            Some(_) => ctx.input_str(key).trim().to_string(),
        };
        let (host, port, user, password) = (get("host", "127.0.0.1"), get("port", "8080"), get("user", ""), ctx.input_str("password"));
        if host.is_empty() {
            return ctx.fail("Host cannot be empty");
        }
        if !matches!(port.parse::<u32>(), Ok(1..=65535)) {
            return ctx.fail("Enter a valid port (REAPER ▸ Preferences ▸ Control/OSC/web shows it)");
        }
        let conn = reaper::Conn::new(&host, &port, &user, &password);
        if let Err(e) = conn.snapshot() {
            return ctx.fail(e.message().to_string());
        }
        knobdeck_sdk::save_env(&[("REAPER_HOST", &host), ("REAPER_PORT", &port), ("REAPER_USER", &user)])?;
        knobdeck_sdk::secrets::set("REAPER_PASSWORD", &password)?;
        ctx.log(format!("[setup] REAPER web interface: {}", conn.base));
        Ok(())
    })
}
