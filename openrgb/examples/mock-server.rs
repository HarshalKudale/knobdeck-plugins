//! The tests' stand-in OpenRGB SDK server on a port, for `knobdeck-plugin drive`:
//!   cargo run -p openrgb --example mock-server -- 16742
//!   OPENRGB_PORT=16742 knobdeck-plugin drive steps.json
//! Prints each device's mode and LED colours once a second.

#[allow(dead_code)]
#[path = "../src/sdk.rs"]
mod sdk;
#[path = "../src/mock_server.rs"]
mod mock_server;

fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(16742);
    let world = mock_server::serve_on(port, 4);
    let mut last = String::new();
    loop {
        let now = {
            let w = world.lock().unwrap();
            let devs: Vec<String> = w.devs.iter().map(|d| format!("{} [{}] leds={:?} mode colors={:?} brightness={}", d.name, d.modes[d.active].name, d.leds, d.modes[d.active].colors, d.modes[d.active].bri.2)).collect();
            format!("{}\nprofiles loaded: {:?}", devs.join("\n"), w.loaded)
        };
        if now != last {
            println!("{now}\n--");
            last = now;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}
