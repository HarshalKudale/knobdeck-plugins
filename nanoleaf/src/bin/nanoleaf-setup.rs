//! Nanoleaf setup: find the devices (mDNS plus typed addresses) and pair with each one the user
//! puts in pairing mode (power button held 5–7 seconds until the lights flash).

#[allow(dead_code)]
#[path = "../leaf.rs"]
mod leaf;

use std::time::{Duration, Instant};

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        knobdeck_sdk::load_env();
        let typed: Vec<String> = ctx.input_str("addresses").split([',', ' ', '\n']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        let mut all = typed.clone();
        for a in leaf::discover(Duration::from_secs(3)) {
            if !all.contains(&a) {
                all.push(a);
            }
        }
        if all.is_empty() {
            return ctx.fail("No Nanoleaf device found. Make sure it is on and on the same network as this computer, or type its address (Nanoleaf app ▸ device ▸ Settings ▸ Device Info).");
        }
        ctx.log(format!("[setup] found: {}. Hold the power button of each device for 5–7 seconds, until its lights flash.", all.join(", ")));
        // Devices paired before keep their tokens; a device paired now replaces its own.
        let mut paired: Vec<String> = knobdeck_sdk::env_or("NANOLEAF_DEVICES", "").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        let mut new = vec![];
        let end = Instant::now() + Duration::from_secs(60);
        while Instant::now() < end && new.len() < all.len() {
            for a in &all {
                if new.contains(a) {
                    continue;
                }
                match leaf::pair(a) {
                    Ok(token) => {
                        knobdeck_sdk::secrets::set(&leaf::token_key(a), &token)?;
                        ctx.log(format!("[setup] paired with {a}"));
                        new.push(a.clone());
                        if !paired.contains(a) {
                            paired.push(a.clone());
                        }
                    }
                    Err(e) if e.message() == leaf::NOT_PAIRING => {}
                    Err(e) => ctx.log(format!("[setup] {a}: {}", e.message())),
                }
            }
            // Once one device paired, stop 15 seconds into the window instead of waiting a whole
            // minute for devices the user doesn't mean to pair.
            if !new.is_empty() && Instant::now() + Duration::from_secs(45) > end {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        if new.is_empty() {
            return ctx.fail("No device was put in pairing mode within a minute. Hold the device's power button for 5–7 seconds until its lights flash, then run setup again.");
        }
        knobdeck_sdk::save_env(&[("NANOLEAF_DEVICES", &paired.join(","))])?;
        Ok(())
    })
}
