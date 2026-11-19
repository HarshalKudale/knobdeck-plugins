//! Network plugin for KnobDeck: public IP, Wake-on-LAN, Tailscale (LocalAPI), saved Wi-Fi networks
//! and VPNs (NetworkManager / Native Wi-Fi + RAS / `scutil --nc`), Pi-hole and Uptime Kuma tiles.

mod homelab;
mod os;
mod tailscale;
mod wol;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, load_env, Action, Error, Plugin, Result, Value, Visual};
use os::Kind;
use std::time::Duration;

const FAST: Duration = Duration::from_secs(5);

fn on_label(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

/// A problem shows on the key itself (and the key reads as off).
fn on_key(title: &str, icon: &str, r: Result<Visual>) -> Result<Visual> {
    Ok(r.unwrap_or_else(|e| {
        Visual::new()
            .state("off")
            .label(format!("{title}\n{}", e.message()))
            .icon(icon)
    }))
}

fn public_ip(v6: bool) -> Result<String> {
    let url = if v6 {
        "https://api6.ipify.org"
    } else {
        "https://api.ipify.org"
    };
    let ip = ureq::get(url)
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|_| Error::msg(if v6 { "No IPv6 connection" } else { "Offline" }))?
        .into_string()
        .map_err(|e| Error::msg(e.to_string()))?;
    let ip = ip.trim().to_string();
    if ip.parse::<std::net::IpAddr>().is_err() {
        return Err(Error::msg("The IP service sent an invalid reply"));
    }
    Ok(ip)
}

fn ip_visual(inputs: &Value) -> Result<Visual> {
    let v6 = inputs["family"] == "v6";
    on_key(
        "Public IP",
        "globe",
        public_ip(v6).map(|ip| {
            Visual::new()
                .label(format!("Public IP{}\n{ip}", if v6 { " (v6)" } else { "" }))
                .icon("globe")
        }),
    )
}

// ── Connections (Wi-Fi, VPN) ──

fn conn_options(kind: Kind) -> Result<Value> {
    Ok(Value::Array(
        os::list(kind)?
            .into_iter()
            .map(|c| json!({"value": c.id, "label": c.name}))
            .collect(),
    ))
}

fn conn_state(kind: Kind, id: &str) -> Result<(String, bool)> {
    if id.is_empty() {
        return Err(Error::msg(if kind == Kind::Wifi {
            "Choose a network"
        } else {
            "Choose a VPN"
        }));
    }
    let c = os::list(kind)?
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| Error::msg("Not found: it may have been removed"))?;
    Ok((c.name, c.active))
}

fn conn_action(kind: Kind) -> Action {
    let (id, label, query, icon_on, icon_off) = match kind {
        Kind::Wifi => ("wifi", "Wi-Fi Network", "wifi-networks", "wifi", "wifi-off"),
        Kind::Vpn => ("vpn", "VPN", "vpns", "shield-check", "shield-off"),
    };
    let a = Action::toggle(id, label)
        .icon(icon_on)
        .state("off", "Disconnected", icon_off)
        .state("on", "Connected", icon_on)
        .inputs(vec![input::query_select(
            "connection",
            if kind == Kind::Wifi {
                "Saved network"
            } else {
                "VPN connection"
            },
            query,
            &[],
        )])
        .live(FAST, move |ic| {
            on_key(
                label,
                icon_off,
                conn_state(kind, &ic.input_str("connection"))
                    .map(|(name, on)| Visual::new().state(on_label(on)).label(name)),
            )
        })
        .on_press(move |ctx| {
            let id = ctx.input_str("connection");
            let (_, on) = conn_state(kind, &id)?;
            os::set(kind, &id, !on)?;
            ctx.set_state(on_label(!on));
            Ok(())
        });
    // macOS has no Wi-Fi switching route (see os/mod.rs): disabled there, never deleted.
    if kind == Kind::Wifi && cfg!(target_os = "macos") {
        a.platforms(&["none"])
    } else {
        a
    }
}

// ── Tailscale ──

fn ts_visual() -> Result<Visual> {
    let s = tailscale::status()?;
    let on = s.state == "Running";
    let line = match s.state.as_str() {
        "Running" => s.ip.clone(),
        "NeedsLogin" => "Log in to Tailscale".into(),
        "Stopped" => "Off".into(),
        other => other.to_string(),
    };
    Ok(Visual::new()
        .state(on_label(on))
        .label(format!("Tailscale\n{line}")))
}

fn exit_node_visual(node: &str) -> Result<Visual> {
    if node.is_empty() {
        return Err(Error::msg("Choose an exit node"));
    }
    let s = tailscale::status()?;
    let name = s
        .exit_nodes
        .iter()
        .find(|n| n.id == node)
        .map(|n| {
            if n.online {
                n.name.clone()
            } else {
                format!("{} (offline)", n.name)
            }
        })
        .unwrap_or_else(|| "Not an exit node".into());
    Ok(Visual::new()
        .state(on_label(s.exit_node.as_deref() == Some(node)))
        .label(format!("Exit node\n{name}")))
}

// ── Pi-hole and Uptime Kuma ──

fn pihole_visual() -> Result<Visual> {
    let b = homelab::blocking()?;
    let detail = match (b.on, b.timer) {
        (false, Some(t)) => format!("Off · {}:{:02}", (t as u64) / 60, (t as u64) % 60),
        (false, None) => "Blocking off".into(),
        (true, _) => homelab::summary()
            .map(|(pct, _, _)| format!("{pct:.1}% blocked"))
            .unwrap_or_else(|_| "Blocking".into()),
    };
    Ok(Visual::new()
        .state(on_label(b.on))
        .label(format!("Pi-hole\n{detail}")))
}

fn kuma_visual(page: &str) -> Result<Visual> {
    let h = homelab::health(page)?;
    Ok(if h.down.is_empty() {
        Visual::new()
            .label(format!("All {} up", h.total))
            .icon("circle-check")
    } else {
        Visual::new()
            .label(format!("{} down\n{}", h.down.len(), h.down.join(", ")))
            .icon("circle-x")
    })
}

fn main() {
    load_env();
    Plugin::new(&format!("network-{}", std::env::consts::OS), "Network")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description("Public IP, Wake-on-LAN, Tailscale (on/off, exit node), saved Wi-Fi networks and VPNs, and Pi-hole / Uptime Kuma status.")
        .icon("network")
        // Joining a network or dialling a VPN can take a while.
        .ack_timeout_ms(45_000)
        .setup("network-setup", None)
        .setup_inputs(vec![
            input::text("pihole_url", "Pi-hole address (optional)").placeholder("http://pi.hole").with("help", "Only for the Pi-hole key. Pi-hole v6."),
            input::text("pihole_password", "Pi-hole password (optional)").with("type", "password").with("help", "The web password, or an app password (Pi-hole ▸ Settings ▸ Web interface / API). Kept in this computer's credential store."),
        ])
        .query("wifi-networks", |q| conn_options(Kind::Wifi).or_else(|e| q.fail(e.message().to_string())))
        .query("vpns", |q| conn_options(Kind::Vpn).or_else(|e| q.fail(e.message().to_string())))
        .query("exit-nodes", |q| {
            let s = tailscale::status().or_else(|e| q.fail(e.message().to_string()))?;
            Ok(Value::Array(s.exit_nodes.into_iter().map(|n| json!({"value": n.id, "label": n.name})).collect()))
        })
        .action(
            Action::button("public-ip", "Public IP")
                .icon("globe")
                .inputs(vec![input::select("family", "Address", &[("v4", "IPv4"), ("v6", "IPv6")]).default_value("v4")])
                .live(Duration::from_secs(300), |ic| ip_visual(&ic.inputs))
                // Pressing checks again now.
                .on_press(|ctx| {
                    ctx.set_visual(ip_visual(&ctx.inputs)?);
                    Ok(())
                }),
        )
        .action(
            Action::button("wake-on-lan", "Wake on LAN")
                .icon("power")
                .inputs(vec![
                    input::text("mac", "MAC address").placeholder("AA:BB:CC:DD:EE:FF"),
                    input::text("broadcast", "Broadcast address").placeholder("255.255.255.255").default_value("255.255.255.255").with("help", "Your network's broadcast address (e.g. 192.168.1.255) if the default doesn't reach the computer."),
                    input::select("port", "Port", &[("9", "9 (discard)"), ("7", "7 (echo)")]).default_value("9"),
                ])
                .on_press(|ctx| {
                    let mac = wol::parse_mac(&ctx.input_str("mac")).ok_or_else(|| Error::msg("The MAC address must look like AA:BB:CC:DD:EE:FF"))?;
                    let port = ctx.input_str("port").parse::<u16>().unwrap_or(9);
                    let to = match ctx.input_str("broadcast").trim() {
                        "" => "255.255.255.255".to_string(),
                        b => b.to_string(),
                    };
                    wol::send(&mac, &to, port)
                }),
        )
        .action(
            Action::toggle("tailscale", "Tailscale")
                .icon("network")
                .state("off", "Off", "cloud-off")
                .state("on", "Connected", "network")
                .live(FAST, |_| on_key("Tailscale", "cloud-off", ts_visual()))
                .on_press(|ctx| {
                    let on = tailscale::status()?.state == "Running";
                    tailscale::set_running(!on)?;
                    ctx.fan_out(on_label(!on));
                    Ok(())
                }),
        )
        .action(
            Action::toggle("exit-node", "Tailscale Exit Node")
                .icon("door-open")
                .state("off", "Not in use", "door-closed")
                .state("on", "In use", "door-open")
                .inputs(vec![input::query_select("node", "Exit node", "exit-nodes", &[])])
                .live(FAST, |ic| on_key("Exit node", "door-closed", exit_node_visual(&ic.input_str("node"))))
                .on_press(|ctx| {
                    let node = ctx.input_str("node");
                    if node.is_empty() {
                        return ctx.fail("Choose an exit node");
                    }
                    let using = tailscale::status()?.exit_node.as_deref() == Some(node.as_str());
                    tailscale::set_exit_node(if using { "" } else { &node })?;
                    ctx.set_state(on_label(!using));
                    Ok(())
                }),
        )
        .action(
            Action::wheel("exit-nodes", "Tailscale Exit Nodes")
                .icon("door-open")
                .on_open(|o| {
                    let s = tailscale::status().or_else(|e| o.fail(e.message().to_string()))?;
                    let mut items = vec![json!({"id": "", "label": "No exit node", "icon": if s.exit_node.is_none() { "circle-check" } else { "circle" }})];
                    items.extend(s.exit_nodes.iter().take(15).map(|n| {
                        let icon = if s.exit_node.as_deref() == Some(n.id.as_str()) { "circle-check" } else if n.online { "circle" } else { "circle-off" };
                        json!({"id": n.id, "label": n.name, "icon": icon})
                    }));
                    Ok(items)
                })
                .on_child_press(|ctx| tailscale::set_exit_node(ctx.child_id.as_deref().unwrap_or(""))),
        )
        .action(conn_action(Kind::Wifi))
        .action(conn_action(Kind::Vpn))
        .action(
            Action::toggle("pihole", "Pi-hole Blocking")
                .icon("shield")
                .state("off", "Off", "shield-off")
                .state("on", "Blocking", "shield")
                .inputs(vec![input::text("minutes", "Pause for (minutes)").placeholder("5").default_value("5").with("help", "Pressing turns blocking off for this long; 0 turns it off until pressed again.")])
                .live(Duration::from_secs(10), |_| on_key("Pi-hole", "shield-off", pihole_visual()))
                .on_press(|ctx| {
                    let on = homelab::blocking()?.on;
                    let secs = if on { (ctx.input_f64("minutes").unwrap_or(5.0).max(0.0) * 60.0) as u64 } else { 0 };
                    let b = homelab::set_blocking(!on, secs)?;
                    ctx.fan_out(on_label(b.on));
                    Ok(())
                }),
        )
        .action(
            Action::button("uptime-kuma", "Uptime Kuma Status")
                .icon("activity")
                .inputs(vec![input::text("page", "Status page address").placeholder("http://kuma.local:3001/status/home").with("help", "A public status page (Uptime Kuma ▸ Status Pages).")])
                .live(Duration::from_secs(30), |ic| on_key("Uptime Kuma", "circle-alert", kuma_visual(&ic.input_str("page"))))
                // Pressing opens the status page.
                .on_press(|ctx| {
                    let (base, slug) = homelab::status_page(&ctx.input_str("page"))?;
                    knobdeck_sdk::open_url(&format!("{base}/status/{slug}"))
                }),
        )
        .run()
}
