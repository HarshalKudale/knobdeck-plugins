//! Constants, PKCE and token storage shared by the plugin and its setup binary.
//!
//! The Discord application is a public client: authorization codes (with their PKCE verifier) and
//! refresh tokens are exchanged straight with Discord, with no client secret and no KnobDeck server.
//!
//! The sign-in lives in the OS credential store (macOS Keychain, Windows Credential Manager, Linux
//! Secret Service). Only the plugin binary touches it: setup leaves a fresh sign-in in a hand-off
//! file (owner-only on Unix) that the plugin moves into the store on its next start. On macOS a
//! keychain item trusts the binary that created it, so a second binary (setup) would make the
//! user approve access each time.
#![allow(dead_code)]

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const APP_ID: &str = "1527677329524916424";
pub const RPC_SCOPES: [&str; 11] = [
    "identify",
    "guilds",
    "messages.read",
    "rpc",
    "rpc.notifications.read",
    "rpc.screenshare.read",
    "rpc.screenshare.write",
    "rpc.video.read",
    "rpc.video.write",
    "rpc.voice.read",
    "rpc.voice.write",
];
const TOKEN_ENDPOINT: &str = "https://discord.com/api/v10/oauth2/token";
/// Setup → plugin hand-off in the plugin data folder; also where builds before 1.2.0 kept the
/// sign-in, so an existing sign-in moves into the credential store the same way.
pub const TOKEN_FILE: &str = "discord-rpc-token.json";
const KEYRING_SERVICE: &str = "KnobDeck Discord plugin";
const KEYRING_USER: &str = "discord-rpc-token";

/// Runtime env var, else the value inlined from `.env` at build time, else KnobDeck's app.
pub fn client_id() -> String {
    std::env::var("DISCORD_CLIENT_ID")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| option_env!("DISCORD_CLIENT_ID").map(String::from))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| APP_ID.into())
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// A fresh PKCE pair (RFC 7636, S256): 64-char verifier from 48 random bytes.
pub fn pkce() -> Result<Pkce, String> {
    let mut bytes = [0u8; 48];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("no system randomness for the Discord sign-in: {e}"))?;
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok(Pkce { verifier, challenge })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// POST a grant to Discord's token endpoint; returns what we store.
fn request_token(grant: &[(&str, &str)]) -> Result<Value, String> {
    let cid = client_id();
    let mut form = vec![("client_id", cid.as_str())];
    form.extend_from_slice(grant);
    let reply: Value = match ureq::post(TOKEN_ENDPOINT).timeout(std::time::Duration::from_secs(15)).send_form(&form) {
        Ok(r) => r.into_json().map_err(|e| format!("Discord token exchange returned invalid JSON: {e}"))?,
        Err(ureq::Error::Status(status, r)) => {
            let v: Value = r.into_json().unwrap_or_default();
            let why = v["error_description"].as_str().or(v["error"].as_str()).map(String::from).unwrap_or_else(|| format!("HTTP {status}"));
            return Err(format!("Discord token exchange failed: {why}"));
        }
        Err(e) => return Err(format!("Discord token exchange failed: {e}")),
    };
    let access = reply["access_token"].as_str().filter(|t| !t.is_empty()).ok_or("Discord token exchange returned no access_token")?;
    Ok(json!({
        "access_token": access,
        "refresh_token": reply["refresh_token"],
        "expires_at": now_secs() + reply["expires_in"].as_u64().unwrap_or(0),
    }))
}

/// Setup: swap the authorization code (and its PKCE verifier) for tokens and leave them for the
/// plugin. Returns the access token.
pub fn exchange_code(code: &str, verifier: &str) -> Result<String, String> {
    let saved = request_token(&[("grant_type", "authorization_code"), ("code", code), ("code_verifier", verifier)])?;
    write_handoff(&saved)?;
    Ok(saved["access_token"].as_str().unwrap_or_default().to_string())
}

/// Renew the access token with the stored refresh token. None if there is no refresh token or
/// Discord refused it (the user must run setup again).
pub fn refresh_token() -> Option<String> {
    let refresh = stored()?["refresh_token"].as_str()?.to_string();
    let saved = request_token(&[("grant_type", "refresh_token"), ("refresh_token", &refresh)])
        .map_err(|e| knobdeck_sdk::log(format!("token refresh failed: {e}")))
        .ok()?;
    store(&saved);
    saved["access_token"].as_str().map(String::from)
}

/// The stored access token, renewed first when it is within a day of expiry.
/// Tokens saved by older builds have no `expires_at` and are used as is.
pub fn load_token() -> Option<String> {
    let v = stored()?;
    let token = v["access_token"].as_str().filter(|t| !t.is_empty())?.to_string();
    match v["expires_at"].as_u64() {
        Some(at) if at < now_secs() + 86_400 => refresh_token().or(Some(token)),
        _ => Some(token),
    }
}

pub fn clear_token() {
    let _ = std::fs::remove_file(token_path());
    if let Ok(e) = keyring_entry() {
        let _ = e.delete_credential();
    }
}

pub fn token_path() -> std::path::PathBuf {
    knobdeck_sdk::data_file(TOKEN_FILE)
}

fn keyring_entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
}

/// The sign-in: a hand-off from setup (moved into the credential store first), else the store.
fn stored() -> Option<Value> {
    if let Some(v) = std::fs::read_to_string(token_path()).ok().and_then(|t| t.parse::<Value>().ok()) {
        store(&v);
        return Some(v);
    }
    keyring_entry().ok()?.get_password().ok()?.parse().ok()
}

/// Save to the credential store and drop the hand-off file. Where there is no store (a Linux
/// session without a Secret Service), keep the file so the plugin still works, and say why.
fn store(v: &Value) {
    match keyring_entry().and_then(|e| e.set_password(&v.to_string())) {
        Ok(()) => {
            let _ = std::fs::remove_file(token_path());
        }
        Err(e) => {
            knobdeck_sdk::log(format!("OS credential store unavailable ({e}); keeping the Discord sign-in in the plugin data folder"));
            if let Err(e) = write_handoff(v) {
                knobdeck_sdk::log(e);
            }
        }
    }
}

/// Write the token file readable by the user only (Unix); Windows' per-user profile ACL covers it.
fn write_handoff(v: &Value) -> Result<(), String> {
    use std::io::Write;
    let path = token_path();
    // Recreated, not truncated: an older file may carry wider permissions.
    let _ = std::fs::remove_file(&path);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&path).map_err(|e| format!("failed to save Discord token: {e}"))?;
    f.write_all(format!("{v:#}").as_bytes()).map_err(|e| format!("failed to save Discord token: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_pair_is_rfc7636_s256() {
        let p = pkce().unwrap();
        assert_eq!(p.verifier.len(), 64);
        assert!(p.verifier.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        assert_eq!(p.challenge, URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes())));
        assert_ne!(p.verifier, pkce().unwrap().verifier);
    }
}
