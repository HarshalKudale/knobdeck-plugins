//! TeamSpeak setup: which client (5/6 Remote Apps or 3 ClientQuery), where it listens, and the
//! TeamSpeak 3 ClientQuery API key (kept in the OS credential store). TeamSpeak 5/6 issue their
//! own key when the user allows KnobDeck; the plugin keeps that one.

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let get = |key: &str, default: &str| match ctx.inputs.get(key) {
            None | Some(knobdeck_sdk::Value::Null) => default.to_string(),
            Some(_) => ctx.input_str(key).trim().to_string(),
        };
        let client = get("client", "remote-apps");
        let (host, port, apikey) = (get("host", "127.0.0.1"), get("port", ""), get("apikey", ""));
        if !matches!(client.as_str(), "remote-apps" | "clientquery") {
            return ctx.fail("Choose a TeamSpeak version");
        }
        if host.is_empty() {
            return ctx.fail("Host cannot be empty");
        }
        if !port.is_empty() && !matches!(port.parse::<u32>(), Ok(1..=65535)) {
            return ctx.fail("Invalid port number");
        }
        if client == "clientquery" && apikey.is_empty() {
            return ctx.fail("TeamSpeak 3 needs its ClientQuery API key (Settings ▸ Addons ▸ ClientQuery ▸ Settings)");
        }
        knobdeck_sdk::save_env(&[("TEAMSPEAK_CLIENT", &client), ("TEAMSPEAK_HOST", &host), ("TEAMSPEAK_PORT", &port)])?;
        if client == "clientquery" {
            knobdeck_sdk::secrets::set("TEAMSPEAK3_CLIENTQUERY_KEY", &apikey)?;
        }
        ctx.log(format!("[setup] TeamSpeak: {client} at {host}:{}", if port.is_empty() { "default port" } else { &port }));
        Ok(())
    })
}
