//! Spotify Web API with the user's sign-in (PKCE, no client secret), through the user's own Spotify
//! app (its client id comes from setup: new Spotify apps stay in development mode, limited to 25
//! users, so KnobDeck can't ship one app for everyone). Tokens live in the OS credential store
//! (SDK `oauth`). Shared with the setup program.

use knobdeck_sdk::oauth::{encode, OAuth};
use knobdeck_sdk::{env_or, Error, Result, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
/// Where the SDK's `OAuth` keeps the sign-in in the credential store (setup clears it).
#[allow(dead_code)] // used by setup
pub const TOKEN_KEY: &str = "oauth-token.json";

/// The user's own Spotify app (`SPOTIFY_CLIENT_ID` in `.env`, saved by setup). A client id is
/// public (PKCE has no secret), so it is a setting, not a secret.
pub fn client_id() -> String {
    env_or("SPOTIFY_CLIENT_ID", "")
}

pub fn auth() -> OAuth {
    OAuth::new(&env_or("SPOTIFY_TOKEN_URL", TOKEN_URL), &client_id())
}

pub const NOT_SET_UP: &str = "Set up your Spotify app first: run this plugin's setup with your Spotify app's client ID";

/// Whether setup has run: a client id and a saved sign-in. Checked before every call, so the
/// playback keys (which need neither) never make the host ask for setup.
pub fn ready() -> Result<()> {
    if client_id().is_empty() || !auth().signed_in() {
        return Err(Error::msg(NOT_SET_UP));
    }
    Ok(())
}

fn base() -> String {
    env_or("SPOTIFY_API_BASE", "https://api.spotify.com/v1")
}

/// Turn Spotify's error reply into words a user can act on.
fn api_error(code: u16, body: &Value) -> Error {
    let reason = body["error"]["reason"].as_str().unwrap_or("");
    let message = body["error"]["message"].as_str().unwrap_or("");
    Error::msg(match (code, reason) {
        (403, _) if reason == "PREMIUM_REQUIRED" || message.to_lowercase().contains("premium") => "This needs Spotify Premium".to_string(),
        (403, _) if message.to_lowercase().contains("not registered") || message.to_lowercase().contains("user may not") => {
            "Add your Spotify account under User Management in your Spotify app (developer.spotify.com)".to_string()
        }
        (404, _) | (_, "NO_ACTIVE_DEVICE") => "Start playing Spotify on a device first".to_string(),
        (429, _) => "Spotify is rate-limiting requests; try again in a moment".to_string(),
        _ => format!("Spotify said: {} (HTTP {code})", if message.is_empty() { reason } else { message }),
    })
}

/// One API call; a 401 refreshes the sign-in and retries once. Empty replies (204) are `Null`.
pub fn call(method: &str, path: &str, body: Option<Value>) -> Result<Value> {
    ready()?;
    let auth = auth();
    for attempt in 0..2 {
        let token = if attempt == 0 { auth.access_token()? } else { auth.refresh()?["access_token"].as_str().unwrap_or("").to_string() };
        let req = ureq::request(method, &format!("{}{path}", base())).timeout(Duration::from_secs(10)).set("Authorization", &format!("Bearer {token}"));
        let res = match &body {
            Some(b) => req.send_json(b.clone()),
            None if method == "GET" => req.call(),
            None => req.send_bytes(&[]),
        };
        match res {
            Ok(r) => {
                let text = r.into_string().unwrap_or_default();
                if method != "GET" {
                    forget_player();
                }
                return Ok(if text.trim().is_empty() { Value::Null } else { text.parse().unwrap_or(Value::Null) });
            }
            Err(ureq::Error::Status(401, _)) if attempt == 0 => continue,
            Err(ureq::Error::Status(code, r)) => return Err(api_error(code, &r.into_json().unwrap_or(Value::Null))),
            Err(e) => return Err(Error::msg(format!("Spotify is unreachable ({e})"))),
        }
    }
    Err(Error::msg("Spotify sign-in expired — run setup again"))
}

pub fn get(path: &str) -> Result<Value> {
    call("GET", path, None)
}

/// The playback state, shared by every key that polls within a moment (`Null` when nothing is
/// playing anywhere).
static PLAYER: Mutex<Option<(Instant, Value)>> = Mutex::new(None);

pub fn player() -> Result<Value> {
    if let Some((at, v)) = PLAYER.lock().unwrap().as_ref() {
        if at.elapsed() < Duration::from_millis(1500) {
            return Ok(v.clone());
        }
    }
    let v = get("/me/player")?;
    *PLAYER.lock().unwrap() = Some((Instant::now(), v.clone()));
    Ok(v)
}

/// After a change, the next read fetches fresh state.
fn forget_player() {
    *PLAYER.lock().unwrap() = None;
}

/// URI of the current track (or episode).
pub fn current_track() -> Result<String> {
    let p = player()?;
    p["item"]["uri"].as_str().map(String::from).ok_or_else(|| Error::msg("Nothing is playing"))
}

/// Whether `uri` is in the user's library (Liked Songs for a track).
pub fn saved(uri: &str) -> Result<bool> {
    Ok(get(&format!("/me/library/contains?uris={}", encode(uri)))?[0] == true)
}

/// Save `uri` to the library, or remove it.
pub fn save(uri: &str, keep: bool) -> Result<()> {
    call(if keep { "PUT" } else { "DELETE" }, &format!("/me/library?uris={}", encode(uri)), None).map(|_| ())
}

/// The user's playlists (first 50), `(uri, name)`, plus Liked Songs first.
pub fn playlists() -> Result<Vec<(String, String)>> {
    let me = get("/me")?;
    let mut out = vec![(format!("{}:collection", me["uri"].as_str().unwrap_or("spotify:user:me")), "Liked Songs".to_string())];
    let page = get("/me/playlists?limit=50")?;
    for p in page["items"].as_array().cloned().unwrap_or_default() {
        if let (Some(uri), Some(name)) = (p["uri"].as_str(), p["name"].as_str()) {
            out.push((uri.to_string(), name.to_string()));
        }
    }
    Ok(out)
}

/// Spotify Connect devices: `(id, name, type, active)`.
pub fn devices() -> Result<Vec<(String, String, String, bool)>> {
    Ok(get("/me/player/devices")?["devices"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|d| Some((d["id"].as_str()?.to_string(), d["name"].as_str().unwrap_or("").to_string(), d["type"].as_str().unwrap_or("").to_string(), d["is_active"] == true)))
        .collect())
}

/// A device id for `wanted`: device ids can change when a device restarts, so keys keep the
/// device's name and it is looked up again on every press (an id is accepted too).
pub fn device_id(wanted: &str) -> Result<String> {
    let w = wanted.trim();
    let list = devices()?;
    list.iter()
        .find(|(id, name, _, _)| name.eq_ignore_ascii_case(w) || id == w)
        .map(|(id, ..)| id.clone())
        .ok_or_else(|| Error::msg(format!("Spotify doesn't see \"{w}\" right now: open Spotify on it")))
}

/// Move playback to a device (and keep playing).
pub fn transfer(device: &str) -> Result<()> {
    let id = device_id(device)?;
    call("PUT", "/me/player", Some(knobdeck_sdk::json!({"device_ids": [id], "play": true}))).map(|_| ())
}

/// `https://open.spotify.com/track/ID?si=…`, `spotify:track:ID` → `spotify:track:ID` (tracks and
/// episodes: what the queue takes).
pub fn queue_uri(text: &str) -> Option<String> {
    let t = text.trim();
    let (kind, id) = if let Some(rest) = t.strip_prefix("spotify:") {
        rest.split_once(':')?
    } else {
        let path = t.split_once("open.spotify.com/")?.1;
        let mut parts = path.split(['/', '?', '#']).filter(|p| !p.is_empty());
        let mut kind = parts.next()?;
        if kind.starts_with("intl-") {
            kind = parts.next()?;
        }
        (kind, parts.next()?)
    };
    (matches!(kind, "track" | "episode") && !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| format!("spotify:{kind}:{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    #[test]
    fn errors_read_well() {
        let premium = json!({"error": {"status": 403, "message": "Player command failed: Premium required", "reason": "PREMIUM_REQUIRED"}});
        assert_eq!(api_error(403, &premium).message(), "This needs Spotify Premium");
        assert_eq!(api_error(404, &json!({"error": {"reason": "NO_ACTIVE_DEVICE"}})).message(), "Start playing Spotify on a device first");
        assert_eq!(api_error(400, &json!({"error": {"message": "Bad thing"}})).message(), "Spotify said: Bad thing (HTTP 400)");
    }

    #[test]
    fn queue_links() {
        assert_eq!(queue_uri("spotify:track:4uLU6hMCjMI75M1A2tKUQC").as_deref(), Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC"));
        assert_eq!(queue_uri(" https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC?si=abc ").as_deref(), Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC"));
        assert_eq!(queue_uri("https://open.spotify.com/intl-de/episode/0Q86acNRm6V9GYx55SXKwf").as_deref(), Some("spotify:episode:0Q86acNRm6V9GYx55SXKwf"));
        assert_eq!(queue_uri("https://open.spotify.com/album/1"), None);
        assert_eq!(queue_uri("hello"), None);
    }

    #[test]
    fn not_set_up_without_a_client_id() {
        std::env::remove_var("SPOTIFY_CLIENT_ID");
        assert_eq!(get("/me").unwrap_err().message(), NOT_SET_UP);
    }
}
