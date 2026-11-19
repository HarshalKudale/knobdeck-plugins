//! Network setup: only the Pi-hole key needs it. A given address is checked by logging in;
//! the password goes to the credential store (an empty field removes one saved before).

#[allow(dead_code)]
#[path = "../homelab.rs"]
mod homelab;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let url = ctx.input_str("pihole_url").trim().to_string();
        let password = ctx.input_str("pihole_password");
        if !url.is_empty() {
            let base = homelab::base_url(&url);
            if let Err(e) = homelab::pihole_login(&base, &password) {
                return ctx.fail(e.message().to_string());
            }
            ctx.log(format!("[setup] Pi-hole at {base}: signed in"));
        }
        knobdeck_sdk::secrets::set("PIHOLE_PASSWORD", &password)?;
        knobdeck_sdk::save_env(&[("PIHOLE_URL", &url)])?;
        Ok(())
    })
}
