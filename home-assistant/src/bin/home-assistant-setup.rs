//! Home Assistant setup: check the URL and token answer, then save them.

#[allow(dead_code)]
#[path = "../ha.rs"]
mod ha;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let url = ctx.input_str("url");
        let token = ctx.input_str("token");
        if url.trim().is_empty() {
            return ctx.fail("Enter your Home Assistant URL, like http://homeassistant.local:8123");
        }
        if token.trim().is_empty() {
            return ctx.fail("Paste a long-lived access token (Home Assistant ▸ your profile ▸ Security ▸ Long-lived access tokens).");
        }
        let conn = ha::Conn::new(&url, &token);
        if let Err(e) = conn.check() {
            return ctx.fail(e.message());
        }
        let count = ha::states(&conn).map(|s| s.len()).unwrap_or(0);
        ctx.log(format!("[setup] connected to {} ({count} entities)", conn.url));
        knobdeck_sdk::save_env(&[("HA_URL", &conn.url), ("HA_TOKEN", &conn.token)])?;
        Ok(())
    })
}
