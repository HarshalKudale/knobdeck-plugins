//! WLED setup: find devices (mDNS plus any typed addresses), check they answer, save the list.

// Shared with the plugin; setup uses only discovery and names.
#[allow(dead_code)]
#[path = "../wled.rs"]
mod wled;

use std::time::Duration;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let typed: Vec<String> = ctx.input_str("addresses").split([',', ' ', '\n']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        for a in &typed {
            if let Err(e) = wled::name(a) {
                return ctx.fail(format!("{a}: {e}. Check the address (WLED ▸ Config ▸ WiFi Setup shows it) and that the device is on."));
            }
        }
        let mut all = typed.clone();
        for a in wled::discover(Duration::from_secs(3)) {
            if !all.contains(&a) {
                all.push(a);
            }
        }
        let named: Vec<String> = all.iter().filter_map(|a| wled::name(a).ok().map(|n| format!("{n} ({a})"))).collect();
        if named.is_empty() {
            return ctx.fail("No WLED device found. Make sure it is on and on the same network as this computer, or type its address.");
        }
        ctx.log(format!("[setup] devices: {}", named.join(", ")));
        // Only typed addresses are saved: discovered devices are found again at every start, so a
        // device that gets a new address from the router keeps working.
        knobdeck_sdk::save_env(&[("WLED_DEVICES", &typed.join(","))])?;
        Ok(())
    })
}
