//! Expose `DISCORD_CLIENT_ID` from `.env` (if present) as a compile-time env var so a
//! shipped binary can fall back to it via `option_env!`. There is no client secret:
//! the Discord app is a public client and the plugin signs in with PKCE.

fn main() {
    println!("cargo:rerun-if-changed=.env");
    let Ok(text) = std::fs::read_to_string(".env") else { return };
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            if k.trim() == "DISCORD_CLIENT_ID" {
                println!("cargo:rustc-env={}={}", k.trim(), v);
            }
        }
    }
}
