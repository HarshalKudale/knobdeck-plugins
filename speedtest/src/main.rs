//! Speed Test plugin for KnobDeck: download, upload and latency against Cloudflare's speed test
//! servers, and a live latency tile for any host.

mod test;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{Action, Plugin, Runtime, Visual};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use test::{short, Results};

/// The last finished test, shared by every Speed Test key.
static LAST: Mutex<Option<(Results, Instant)>> = Mutex::new(None);
/// Why the last test failed (cleared by the next success), so the key keeps showing it.
static FAILED: Mutex<Option<String>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
static RT: OnceLock<Runtime> = OnceLock::new();

const SHOW: &[(&str, &str)] = &[("all", "Download, upload and ping"), ("down", "Download"), ("up", "Upload"), ("ping", "Ping")];

fn label(r: &Results, show: &str) -> String {
    match show {
        "down" => format!("↓ {} Mb/s", short(r.down_mbps)),
        "up" => format!("↑ {} Mb/s", short(r.up_mbps)),
        "ping" => format!("{:.0} ms", r.ping_ms),
        _ => format!("↓ {}\n↑ {}\n{:.0} ms", short(r.down_mbps), short(r.up_mbps), r.ping_ms),
    }
}

/// Run a test in the background; every Speed Test key shows "Testing…" and then the result.
fn start(rt: Runtime) {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return; // one test at a time: a second press while testing does nothing
    }
    rt.notify("speed-test", "testing");
    std::thread::spawn(move || {
        let r = test::run();
        RUNNING.store(false, Ordering::SeqCst);
        match r {
            Ok(r) => {
                *LAST.lock().unwrap() = Some((r, Instant::now()));
                *FAILED.lock().unwrap() = None;
                rt.notify("speed-test", "done");
            }
            Err(e) => {
                *FAILED.lock().unwrap() = Some(e.message().to_string());
                rt.notify_visual("speed-test", Visual::new().state("failed").label(e.message()));
            }
        }
    });
}

fn main() {
    knobdeck_sdk::load_env();
    Plugin::new(&format!("speedtest-{}", std::env::consts::OS), "Speed Test")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Test your internet speed from a key (Cloudflare's speed test servers) and watch the latency to any host.")
        .icon("gauge")
        .on_ready(|rt| {
            let _ = RT.set(rt.clone());
            Ok(())
        })
        .action(
            Action::button("speed-test", "Speed Test")
                .icon("gauge")
                .inputs(vec![
                    input::select("show", "Show", SHOW).default_value("all"),
                    input::select("every", "Test again by itself", &[("0", "Never"), ("15", "Every 15 minutes"), ("60", "Every hour"), ("360", "Every 6 hours")]).default_value("0"),
                ])
                .state("idle", "Speed Test", "gauge")
                .state("testing", "Testing…", "loader")
                .state("done", "Speed Test", "gauge")
                .state("failed", "Failed", "circle-alert")
                .live(Duration::from_secs(5), |ic| {
                    if RUNNING.load(Ordering::SeqCst) {
                        return Ok(Visual::new().state("testing").label("Testing…"));
                    }
                    let last = *LAST.lock().unwrap();
                    let every = ic.input_f64("every").unwrap_or(0.0);
                    let due = every > 0.0 && last.is_none_or(|(_, at)| at.elapsed() >= Duration::from_secs_f64(every * 60.0));
                    if let (true, Some(rt)) = (due, RT.get()) {
                        start(rt.clone());
                        return Ok(Visual::new().state("testing").label("Testing…"));
                    }
                    if let Some(why) = FAILED.lock().unwrap().clone() {
                        return Ok(Visual::new().state("failed").label(why));
                    }
                    Ok(match last {
                        Some((r, _)) => Visual::new().state("done").label(label(&r, &ic.input_str("show"))),
                        None => Visual::new().state("idle").label("Speed Test"),
                    })
                })
                .on_press(|ctx| {
                    start(ctx.runtime().clone());
                    Ok(())
                }),
        )
        .action(
            Action::button("latency", "Latency")
                .icon("activity")
                .inputs(vec![input::text("host", "Host").placeholder("one.one.one.one").default_value("one.one.one.one").with("help", "A name or address; port 443 unless you add one (\"192.168.1.1:80\"). Measured as the time to open a connection.")])
                .live(Duration::from_secs(5), |ic| {
                    let host = ic.input_str("host");
                    let host = if host.trim().is_empty() { "one.one.one.one".to_string() } else { host.trim().to_string() };
                    Ok(match test::connect_ms(&host) {
                        Ok(ms) => Visual::new().label(format!("{ms:.0} ms")),
                        Err(_) => Visual::new().label("No reply"),
                    })
                }),
        )
        .run()
}
