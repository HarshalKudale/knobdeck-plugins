//! The plugin against the stand-in server.

use super::*;
use crate::mock_server::start;

#[test]
fn parses_every_protocol_version() {
    for proto in [1, 2, 3, 4, 5] {
        let (port, _) = start(proto);
        let mut c = Client::connect("127.0.0.1", port).unwrap();
        assert_eq!(c.proto, proto.min(sdk::CLIENT_PROTOCOL));
        let list = c.controllers().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Keyboard");
        assert_eq!(list[0].vendor, "Vendor");
        assert_eq!(list[0].location, "HID: /dev/hidraw0");
        assert_eq!(list[0].leds, 5);
        assert_eq!(list[0].mode().unwrap().name, "Rainbow Wave");
        assert_eq!(list[1].mode().unwrap().colors, vec![[1, 2, 3]]);
        if proto >= 2 {
            assert_eq!(c.profiles().unwrap(), vec!["Gaming", "Work"]);
        } else {
            assert!(c.profiles().is_err());
        }
    }
}

/// The plugin's own functions against the mock: colour, brightness and profiles.
#[test]
fn actions_against_the_mock_server() {
    let (port, world) = start(4);
    std::env::set_var("OPENRGB_HOST", "127.0.0.1");
    std::env::set_var("OPENRGB_PORT", port.to_string());

    let list = controllers().unwrap();
    assert_eq!(resolve(ALL, &list).unwrap(), vec![0, 1]);
    assert_eq!(resolve("1:RAM Stick", &list).unwrap(), vec![1]);
    assert_eq!(resolve("0:RAM Stick", &list).unwrap(), vec![1]); // moved: found by name
    assert!(resolve("3:Mouse", &list).is_err());

    // Set colour on all: the keyboard leaves its effect for Direct, the RAM's Static takes it.
    with(|cl| {
        for i in 0..2 {
            paint(cl, i, &[[255, 128, 0]])?;
        }
        Ok(())
    })
    .unwrap();
    controllers().unwrap(); // a round trip: the server has handled the writes before it
    {
        let w = world.lock().unwrap();
        assert_eq!(w.client_names, vec!["KnobDeck"]);
        assert_eq!(w.devs[0].modes[w.devs[0].active].name, "Direct");
        assert_eq!(w.devs[0].leds, vec![[255, 128, 0]; 5]);
        assert_eq!(w.devs[1].modes[0].colors, vec![[255, 128, 0]]);
    }

    // Brightness: the RAM's mode has its own; the keyboard's Direct mode is scaled.
    with(|cl| set_brightness(cl, 1, 50.0)).unwrap();
    with(|cl| set_brightness(cl, 0, 50.0)).unwrap();
    controllers().unwrap();
    {
        let w = world.lock().unwrap();
        assert_eq!(w.devs[1].modes[0].bri.2, 128);
        assert_eq!(w.devs[1].modes[0].colors, vec![[255, 128, 0]]);
        assert_eq!(w.devs[0].leds, vec![[128, 64, 0]; 5]);
    }
    let list = controllers().unwrap();
    assert_eq!(brightness_of(0, &list[0]).round(), 50.0);
    assert_eq!(brightness_of(1, &list[1]).round(), 50.0);
    // Back up to 100 %: from the remembered colour, not the dimmed one.
    with(|cl| set_brightness(cl, 0, 100.0)).unwrap();
    controllers().unwrap();
    assert_eq!(world.lock().unwrap().devs[0].leds, vec![[255, 128, 0]; 5]);
    with(|cl| set_brightness(cl, 0, 0.0)).unwrap();
    with(|cl| set_brightness(cl, 0, 20.0)).unwrap();
    controllers().unwrap();
    assert_eq!(world.lock().unwrap().devs[0].leds, vec![[51, 26, 0]; 5]);

    // Profiles.
    assert_eq!(with(|c| c.profiles()).unwrap(), vec!["Gaming", "Work"]);
    load_profile("Work").unwrap();
    controllers().unwrap();
    assert_eq!(world.lock().unwrap().loaded, vec!["Work"]);
    assert!(load_profile(" ").is_err());
}

#[test]
fn colors() {
    assert_eq!(sdk::hex("#ff8800"), Some([255, 136, 0]));
    assert_eq!(sdk::hex("FF8800"), Some([255, 136, 0]));
    assert_eq!(sdk::hex("#f80"), None);
    assert_eq!(sdk::scale([200, 100, 0], 50.0), [100, 50, 0]);
    assert_eq!(pct_of(0, 255, 128).round(), 50.0);
    assert_eq!(value_of(0, 255, 50.0), 128);
    assert_eq!(pct_of(255, 0, 0), 100.0); // inverted ranges exist (lower is brighter)
}
