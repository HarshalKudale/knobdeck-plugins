//! The tests' stand-in for the KnobDeck plug-in in Lightroom Classic, on fixed ports, to drive the
//! built plugin with `knobdeck-plugin drive`:
//!   cargo run -p lightroom-classic --example fake-lightroom -- 58770 58771
//!   LIGHTROOM_TOKEN=drive-token-0123456789 knobdeck-plugin drive steps.json
//! Prints every command it receives.

#[allow(dead_code)]
#[path = "../src/link.rs"]
mod link;
#[allow(dead_code)]
#[path = "../src/params.rs"]
mod params;
#[allow(dead_code)]
#[path = "../src/proto.rs"]
mod proto;
#[allow(dead_code)]
#[path = "../src/fake.rs"]
mod fake;

fn main() {
    let mut args = std::env::args().skip(1).map(|a| a.parse::<u16>().expect("port"));
    let (cmd, evt) = (args.next().unwrap_or(58770), args.next().unwrap_or(58771));
    let token = std::env::var("LIGHTROOM_TOKEN").unwrap_or_else(|_| "drive-token-0123456789".into());
    let lr = fake::FakeLightroom::serve(&token, false, cmd, evt);
    println!("fake Lightroom on {} / {}, token {token}", lr.command_port, lr.event_port);
    let mut seen = 0;
    loop {
        let log = lr.log();
        for l in &log[seen..] {
            println!("<- {}", l.replace('\t', " | "));
        }
        seen = log.len();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
