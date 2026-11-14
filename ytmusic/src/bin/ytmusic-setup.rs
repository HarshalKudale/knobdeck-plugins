//! YouTube Music Desktop setup: ask the app for a token (the user clicks Allow in its dialog) and
//! save it.

#[allow(dead_code)]
#[path = "../ytmd.rs"]
mod ytmd;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let host = ytmd::host_or_default(&ctx.input_str("host"));
        let port = ytmd::port_or_default(&ctx.input_str("port"));
        let conn = ytmd::Conn { host: host.clone(), port: port.clone(), token: String::new() };
        let code = match conn.request_code(env!("CARGO_PKG_VERSION")) {
            Ok(c) => c,
            Err(e) => return ctx.fail(e.message()),
        };
        ctx.log(format!("[setup] code {code}: waiting for Allow in YouTube Music Desktop"));
        let token = match conn.request_token(&code) {
            Ok(t) => t,
            Err(e) if e.message() == ytmd::NOT_RUNNING => return ctx.fail("YouTube Music Desktop did not answer in time. Click Allow in its dialog within 30 seconds, then run setup again."),
            Err(e) => return ctx.fail(e.message()),
        };
        let conn = ytmd::Conn { token, ..conn };
        if let Err(e) = conn.get("state") {
            return ctx.fail(e.message());
        }
        knobdeck_sdk::secrets::set("YTMD_TOKEN", &conn.token)?;
        knobdeck_sdk::save_env(&[("YTMD_HOST", &host), ("YTMD_PORT", &port)])?;
        Ok(())
    })
}
