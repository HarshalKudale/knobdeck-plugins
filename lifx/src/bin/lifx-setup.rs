//! LIFX setup: find lights (broadcast plus any typed addresses), check they answer, save the list.

// Shared with the plugin; setup uses only discovery and reads.
#[allow(dead_code)]
#[path = "../lan.rs"]
mod lan;

use std::time::Duration;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let typed: Vec<String> = ctx.input_str("addresses").split([',', ' ', '\n']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        for a in &typed {
            if lan::resolve(a).is_none() {
                return ctx.fail(format!("{a} isn't an address (type it like 192.168.1.60)"));
            }
        }
        let lights = lan::discover(&typed, Duration::from_secs(3));
        for a in &typed {
            if !lights.iter().any(|l| Some(l.addr.ip()) == lan::resolve(a).map(|s| s.ip())) {
                return ctx.fail(format!("{a} did not answer. Check the address in the LIFX app (light ▸ Settings ▸ About) and that the light is on."));
            }
        }
        let named: Vec<String> = lights.iter().map(|l| format!("{} ({})", lan::get(l).map(|s| s.label).unwrap_or_default(), l.addr.ip())).collect();
        if named.is_empty() {
            return ctx.fail("No LIFX light found. Make sure it is on and on the same network as this computer (a firewall must let UDP port 56700 through), or type its address.");
        }
        ctx.log(format!("[setup] lights: {}", named.join(", ")));
        // Only typed addresses are saved: other lights are found again at every start.
        knobdeck_sdk::save_env(&[("LIFX_ADDRESSES", &typed.join(","))])?;
        Ok(())
    })
}
