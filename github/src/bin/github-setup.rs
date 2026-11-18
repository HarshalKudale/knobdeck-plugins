//! GitHub setup: a personal access token when one is typed, else GitHub's device sign-in (a page
//! shows the code and opens github.com/login/device). The token goes to the plugin for the OS
//! credential store.

#[path = "../api.rs"]
#[allow(dead_code)]
mod api;

use knobdeck_sdk::{Error, Result};

/// A local page with the code to enter (setup output is shown only when it fails).
fn code_page(code: &str, verify: &str) -> String {
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let (code, verify) = (esc(code), esc(verify));
    format!(
        r#"<!doctype html><meta charset="utf-8"><title>Sign in to GitHub — KnobDeck</title>
<body style="font:16px system-ui,sans-serif;margin:3em auto;max-width:34em;text-align:center">
<h1 style="font-size:1.4em">Sign in to GitHub for KnobDeck</h1>
<p>Enter this code on GitHub:</p>
<p id="code" style="font:600 2.2em ui-monospace,monospace;letter-spacing:.12em;user-select:all">{code}</p>
<p><button id="go" style="font:inherit;padding:.6em 1.2em;cursor:pointer">Copy code and open GitHub</button></p>
<p style="color:#666">or open <a href="{verify}">{verify}</a> and type it in. KnobDeck finishes on its own once you approve.</p>
<script>
document.getElementById("go").onclick = async () => {{
  try {{ await navigator.clipboard.writeText("{code}"); }} catch (e) {{}}
  location.href = "{verify}";
}};
</script>"#
    )
}

fn check(token: &str) -> Result<String> {
    let c = api::Client { api: api::api_base(), token: token.to_string() };
    let me = c.get_json("/user")?;
    me["login"].as_str().map(String::from).ok_or_else(|| Error::msg("GitHub didn't say who this token belongs to"))
}

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let typed = ctx.input_str("token").trim().to_string();
        let token = if !typed.is_empty() {
            typed
        } else {
            let client_id = api::client_id();
            if client_id.is_empty() {
                return ctx.fail("This build has no GitHub sign-in app. Create a personal access token instead: github.com ▸ Settings ▸ Developer settings ▸ Personal access tokens ▸ Fine-grained tokens (repository permissions Actions: read and write, Pull requests, Issues and Metadata: read; for the Notifications key a classic token with the notifications and repo scopes), and paste it in setup.");
            }
            let web = api::web_base();
            let d = api::device_start(&web, &client_id)?;
            let (code, verify) = (d["user_code"].as_str().unwrap_or(""), d["verification_uri"].as_str().unwrap_or("https://github.com/login/device"));
            ctx.log(format!("[setup] enter {code} at {verify}"));
            eprintln!("Sign in at {verify} with the code {code}");
            let page = ctx.data_dir().join("github-sign-in.html");
            match std::fs::write(&page, code_page(code, verify)) {
                Ok(()) => knobdeck_sdk::open_url(&page.to_string_lossy())?,
                Err(_) => knobdeck_sdk::open_url(verify)?,
            }
            let got = api::device_wait(&web, &client_id, d["device_code"].as_str().unwrap_or(""), d["interval"].as_u64().unwrap_or(5), d["expires_in"].as_u64().unwrap_or(900));
            let _ = std::fs::remove_file(&page);
            got?
        };
        let login = match check(&token) {
            Ok(l) => l,
            Err(e) => return ctx.fail(format!("GitHub didn't accept the token: {}", e.message())),
        };
        ctx.log(format!("[setup] signed in to GitHub as {login}"));
        let probe = api::Client { api: api::api_base(), token: token.clone() };
        if let Err(e) = probe.get_json("/notifications?per_page=1") {
            ctx.log(format!("[setup] the Notifications key won't work with this token: {}", e.message()));
        }
        knobdeck_sdk::secrets::set(api::TOKEN_KEY, &token)?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn page_escapes() {
        let p = super::code_page("AB<CD", "https://github.com/login/device");
        assert!(p.contains("AB&lt;CD") && !p.contains("AB<CD"));
    }
}
