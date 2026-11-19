//! Finance setup: both keys are optional. A key that is given is checked against its service
//! before it is saved to the credential store; an empty field removes a key saved before.

#[allow(dead_code)]
#[path = "../market.rs"]
mod market;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let gecko = ctx.input_str("coingecko_key").trim().to_string();
        let finnhub = ctx.input_str("finnhub_key").trim().to_string();
        if !gecko.is_empty() {
            if let Err(e) = market::check_gecko_key(&gecko) {
                return ctx.fail(format!("{} (a Demo key from the CoinGecko Developer Dashboard; Pro keys aren't supported)", e.message()));
            }
            ctx.log("[setup] CoinGecko key works");
        }
        if !finnhub.is_empty() {
            if let Err(e) = market::check_finnhub_key(&finnhub) {
                return ctx.fail(e.message().to_string());
            }
            ctx.log("[setup] Finnhub key works");
        }
        knobdeck_sdk::secrets::set("COINGECKO_API_KEY", &gecko)?;
        knobdeck_sdk::secrets::set("FINNHUB_API_KEY", &finnhub)?;
        Ok(())
    })
}
