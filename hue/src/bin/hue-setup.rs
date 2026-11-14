//! Philips Hue setup: find the bridge, pair with it (the user presses its link button), save the
//! address, username and bridge id.

#[allow(dead_code)]
#[path = "../hue.rs"]
mod hue;

use std::time::{Duration, Instant};

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let typed = ctx.input_str("bridge").trim().to_string();
        let candidates = if typed.is_empty() { hue::discover(Duration::from_secs(3)) } else { vec![typed.clone()] };
        let bridges: Vec<(String, knobdeck_sdk::Value)> = candidates.into_iter().filter_map(|a| hue::public_config(&a).ok().map(|c| (a, c))).filter(|(_, c)| c["bridgeid"].is_string()).collect();
        let Some((addr, config)) = bridges.into_iter().next() else {
            return ctx.fail(if typed.is_empty() {
                "No Hue Bridge found. Make sure it is on and on the same network as this computer, or type its address (shown in the Hue app ▸ Settings ▸ My Hue system ▸ Bridge).".to_string()
            } else {
                format!("{typed} is not a Hue Bridge or did not answer.")
            });
        };
        let id = config["bridgeid"].as_str().unwrap_or("").to_string();
        let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME"))
            .or_else(|_| std::fs::read_to_string("/etc/hostname").map(|h| h.trim().to_string()))
            .unwrap_or_else(|_| "computer".into());
        // The user may press the button just after starting setup: keep asking for 30 seconds.
        let end = Instant::now() + Duration::from_secs(30);
        let user = loop {
            match hue::pair(&addr, &host) {
                Ok(u) => break u,
                Err(e) if e.message() == hue::LINK_BUTTON && Instant::now() < end => std::thread::sleep(Duration::from_secs(1)),
                Err(e) => return ctx.fail(e.message()),
            }
        };
        ctx.log(format!("[setup] paired with {} ({addr})", config["name"].as_str().unwrap_or("Hue Bridge")));
        // The bridge username is its API key: it goes to the OS credential store.
        knobdeck_sdk::secrets::set("HUE_USER", &user)?;
        knobdeck_sdk::save_env(&[("HUE_BRIDGE", &addr), ("HUE_BRIDGE_ID", &id)])?;
        Ok(())
    })
}
