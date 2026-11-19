//! OpenRGB setup: where its SDK server listens. Connects once and lists the devices, so a server
//! that isn't started shows here.

#[allow(dead_code)]
#[path = "../sdk.rs"]
mod sdk;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let host = match ctx.input_str("host").trim() {
            "" => "127.0.0.1".to_string(),
            h => h.to_string(),
        };
        let port_text = ctx.input_str("port").trim().to_string();
        let port = if port_text.is_empty() {
            sdk::DEFAULT_PORT
        } else {
            match port_text.parse::<u16>() {
                Ok(p) if p > 0 => p,
                _ => return ctx.fail("Invalid port number"),
            }
        };
        let names = sdk::Client::connect(&host, port).and_then(|mut c| c.controllers()).map(|l| l.into_iter().map(|c| c.name).collect::<Vec<_>>());
        match names {
            Ok(n) if n.is_empty() => ctx.log("[setup] OpenRGB answered but has no devices yet (OpenRGB ▸ Rescan Devices)"),
            Ok(n) => ctx.log(format!("[setup] OpenRGB devices: {}", n.join(", "))),
            Err(e) => return ctx.fail(e),
        }
        knobdeck_sdk::save_env(&[("OPENRGB_HOST", host.as_str()), ("OPENRGB_PORT", port_text.as_str())])?;
        Ok(())
    })
}
