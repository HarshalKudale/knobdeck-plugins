//! Discord first-time setup: authorize through the Discord desktop client (native
//! popup, no browser) with PKCE, exchange the code for tokens straight with Discord
//! (the app is a public client: no secret, no KnobDeck server), and hand them to the
//! plugin, which keeps them in the OS credential store.
//!
//! The setup page may send `client_id`: the user's own Discord app, for when KnobDeck's
//! app isn't approved for RPC yet. Blank means KnobDeck's bundled app.

#[path = "../common.rs"]
mod common;
#[path = "../rpc.rs"]
mod rpc;

use knobdeck_sdk::{json, Error};

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let entered = ctx.input_str("client_id").trim().to_string();
        let custom = !entered.is_empty();
        if custom && !common::is_valid_client_id(&entered) {
            return ctx.fail("That Client ID doesn't look right: it is the 17–20 digit Application ID from the Discord Developer Portal (OAuth2 page).");
        }
        let client_id = if custom { entered } else { common::default_client_id() };
        let pkce = common::pkce().map_err(Error::msg)?;

        ctx.log(format!("[setup] Using {} Discord app ({client_id}).", if custom { "your own" } else { "KnobDeck's" }));
        ctx.log("[setup] Connecting to Discord RPC...");
        ctx.log("[setup] An authorization popup will appear inside the Discord client.");

        let rpc = rpc::Rpc::connect(&client_id).map_err(|e| Error::msg(connect_error(&e.message, custom)))?;
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
            .map_err(|e| Error::msg(authorize_error(&e.message, custom)))?["code"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| Error::msg("Discord AUTHORIZE returned no code — did the user deny the request?"))?;

        let access = common::exchange_code(&client_id, &code, &pkce.verifier).map_err(|e| Error::msg(exchange_error(&e, custom)))?;

        rpc.authenticate(&access).map_err(|e| Error::msg(e.message))?;
        ctx.log("[setup] Discord RPC authorized — token saved.");
        rpc.close();
        ctx.log("[setup] Setup complete! The Discord plugin will start now.");
        Ok(())
    })
}

/// A wrong Client ID makes Discord drop the handshake, which looks like a timeout.
fn connect_error(message: &str, custom: bool) -> String {
    if custom && message.contains("TIMEOUT") {
        format!("{message}\nDiscord didn't accept the connection. Check that Discord is running and the Client ID is your app's Application ID.")
    } else {
        message.to_string()
    }
}

/// Discord answers `invalid_scope` when the signed-in account may not grant `rpc` to this app:
/// until Discord approves the app for RPC, only its owner and listed App Testers can.
fn authorize_error(message: &str, custom: bool) -> String {
    if !message.contains("invalid_scope") {
        message.to_string()
    } else if custom {
        format!("{message}\nDiscord only lets your app's owner (or its App Testers) authorize RPC. Sign in to the Discord app with the account that created the app, or add this account as an App Tester.")
    } else {
        format!("{message}\nThis Discord account isn't allowed to authorize KnobDeck's Discord app (Discord limits RPC to an app's testers until it approves the app). Create your own Discord app and enter its Client ID in setup.")
    }
}

/// Without a secret, Discord only exchanges the code for an app marked as a Public Client.
fn exchange_error(message: &str, custom: bool) -> String {
    if custom && (message.contains("invalid_client") || message.contains("client_secret")) {
        format!("{message}\nTurn on Public Client in your app's OAuth2 settings in the Discord Developer Portal, save, and run setup again.")
    } else {
        message.to_string()
    }
}
