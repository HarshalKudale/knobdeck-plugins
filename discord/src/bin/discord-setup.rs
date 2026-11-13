//! Discord first-time setup: authorize through the Discord desktop client (native
//! popup, no browser) with PKCE, exchange the code for tokens straight with Discord
//! (the app is a public client: no secret, no KnobDeck server), and hand them to the
//! plugin, which keeps them in the OS credential store.

#[path = "../common.rs"]
mod common;
#[path = "../rpc.rs"]
mod rpc;

use knobdeck_sdk::{json, Error};

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let client_id = common::client_id();
        let pkce = common::pkce().map_err(Error::msg)?;

        ctx.log("[setup] Connecting to Discord RPC...");
        ctx.log("[setup] An authorization popup will appear inside the Discord client.");

        let rpc = rpc::Rpc::connect(&client_id).map_err(|e| Error::msg(e.message))?;
        let code = rpc
            .request(
                "AUTHORIZE",
                json!({
                    "scopes": common::RPC_SCOPES,
                    "client_id": client_id,
                    "code_challenge": pkce.challenge,
                    "code_challenge_method": "S256",
                }),
            )
            .map_err(|e| Error::msg(authorize_error(&e.message)))?["code"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| Error::msg("Discord AUTHORIZE returned no code — did the user deny the request?"))?;

        let access = common::exchange_code(&code, &pkce.verifier).map_err(Error::msg)?;

        rpc.authenticate(&access).map_err(|e| Error::msg(e.message))?;
        ctx.log("[setup] Discord RPC authorized — token saved.");
        rpc.close();
        ctx.log("[setup] Setup complete! The Discord plugin will start now.");
        Ok(())
    })
}

/// Discord answers `invalid_scope` when the signed-in account may not grant `rpc` to this app:
/// until Discord approves the app for RPC, only its owner and listed App Testers can.
fn authorize_error(message: &str) -> String {
    if message.contains("invalid_scope") {
        format!("{message}\nThis Discord account isn't allowed to authorize KnobDeck's Discord app yet (Discord limits RPC to the app's testers until it approves the app).")
    } else {
        message.to_string()
    }
}
