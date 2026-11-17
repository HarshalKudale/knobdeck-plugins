//! Web Requests setup: save the optional tokens (credential store) and their names (`.env`).

#[allow(dead_code)]
#[path = "../http.rs"]
mod http;

fn main() {
    knobdeck_sdk::run_setup(|ctx| {
        let mut names = vec![];
        for n in 1..=http::TOKEN_SLOTS {
            let name = ctx.input_str(&http::token_name_key(n)).trim().to_string();
            let token = ctx.input_str(&http::token_key(n)).trim().to_string();
            if !name.is_empty() && token.is_empty() {
                return ctx.fail(format!("Token {n} has a name but no token. Paste the token, or clear the name"));
            }
            // An empty value removes a token saved before.
            knobdeck_sdk::secrets::set(&http::token_key(n), &token)?;
            names.push((http::token_name_key(n), if token.is_empty() { String::new() } else { name }));
        }
        let saved = names.iter().filter(|(_, v)| !v.is_empty()).count();
        ctx.log(format!("[setup] {saved} named token(s) saved"));
        let pairs: Vec<(&str, &str)> = names.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        knobdeck_sdk::save_env(&pairs)?;
        Ok(())
    })
}
