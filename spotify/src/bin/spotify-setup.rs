//! Spotify setup: save the user's own Spotify app's client id, then sign in through the browser
//! (PKCE). The token goes to the plugin, which keeps it in the OS credential store. An empty client
//! id finishes setup for the playback keys only (they need no account) and forgets any sign-in.

#[path = "../web.rs"]
#[allow(dead_code)]
mod web;

const AUTH_URL: &str = "https://accounts.spotify.com/authorize";
/// Must be registered in the Spotify app as `http://127.0.0.1:47811/callback`.
const REDIRECT_PORT: u16 = 47811;
const SCOPES: &str = "user-read-playback-state user-modify-playback-state user-read-currently-playing user-library-read user-library-modify playlist-read-private playlist-read-collaborative playlist-modify-private playlist-modify-public";

/// A Spotify client id is 32 hex characters.
fn looks_like_client_id(s: &str) -> bool {
    s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let id = ctx.input_str("client_id").trim().to_string();
        if id.is_empty() {
            knobdeck_sdk::save_env(&[("SPOTIFY_CLIENT_ID", "")])?;
            knobdeck_sdk::secrets::set(web::TOKEN_KEY, "")?;
            ctx.log("[setup] no Spotify app: Play / Pause, Next, Previous and Now Playing work; the other keys need your app's client ID");
            return Ok(());
        }
        if !looks_like_client_id(&id) {
            return ctx.fail("That doesn't look like a Spotify client ID (32 letters and digits, from your app's Settings page)");
        }
        // The sign-in below reads SPOTIFY_CLIENT_ID.
        std::env::set_var("SPOTIFY_CLIENT_ID", &id);
        web::auth().authorize_pkce(AUTH_URL, SCOPES, REDIRECT_PORT, &[])?;
        // In setup the token is still in its hand-off file, so the plugin's check passes here too.
        let me = web::get("/me")?;
        ctx.log(format!("[setup] signed in to Spotify as {}", me["display_name"].as_str().or(me["id"].as_str()).unwrap_or("?")));
        knobdeck_sdk::save_env(&[("SPOTIFY_CLIENT_ID", &id)])?;
        Ok(())
    })
}
