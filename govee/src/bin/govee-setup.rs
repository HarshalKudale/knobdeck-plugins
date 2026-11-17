//! Govee setup: check the API key (when given) against Govee's cloud, look for LAN devices, and
//! save both. Setup succeeds when at least one device is reachable either way.

#[allow(dead_code)]
#[path = "../cloud.rs"]
mod cloud;
#[allow(dead_code)]
#[path = "../lan.rs"]
mod lan;

use std::time::Duration;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let key = ctx.input_str("api_key").trim().to_string();
        let typed: Vec<String> = ctx.input_str("addresses").split([',', ' ', '\n']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        let mut cloud_count = 0;
        if !key.is_empty() {
            match cloud::devices(&key) {
                Ok(list) => {
                    cloud_count = list.len();
                    ctx.log(format!("[setup] Govee account: {}", list.iter().map(|d| if d.name.is_empty() { d.sku.as_str() } else { d.name.as_str() }).collect::<Vec<_>>().join(", ")));
                }
                Err(e) => return ctx.fail(format!("The API key didn't work: {}", e.message())),
            }
        }
        let lan = match lan::Lan::get() {
            Ok(l) => l.scan(&typed, Duration::from_secs(3)),
            Err(e) => {
                ctx.log(format!("[setup] {}", e.message()));
                vec![]
            }
        };
        ctx.log(format!("[setup] on this network: {}", lan.iter().map(|d| format!("{} ({})", d.sku, d.ip)).collect::<Vec<_>>().join(", ")));
        if lan.is_empty() && cloud_count == 0 {
            return ctx.fail("No Govee device found. Turn on LAN Control for your devices in the Govee Home app (device ▸ settings ▸ LAN Control), or add your Govee API key.");
        }
        // The key is a secret; an empty field removes a key saved before.
        knobdeck_sdk::secrets::set("GOVEE_API_KEY", &key)?;
        knobdeck_sdk::save_env(&[("GOVEE_ADDRESSES", &typed.join(","))])?;
        Ok(())
    })
}
