//! GitHub REST and GraphQL with the user's token (shared with the setup program).
//!
//! Polite polling: every GET keeps its `ETag` and asks again with `If-None-Match` (a 304 costs no
//! rate limit), honours `X-Poll-Interval` (notifications), and once GitHub says the rate limit is
//! used up (or sends `Retry-After`), nothing is asked until it resets: live keys keep their last
//! value meanwhile.

use knobdeck_sdk::{env_or, secrets, Error, Result, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The token (OAuth sign-in or personal access token) in the credential store. Not `GITHUB_TOKEN`,
/// which many developers have set in their shell for other tools.
pub const TOKEN_KEY: &str = "KNOBDECK_GITHUB_TOKEN";
/// Classic OAuth scopes the sign-in asks for: `repo` (private repositories: pull requests,
/// workflow runs, re-runs, dispatches) and `notifications`.
pub const SCOPES: &str = "repo notifications";
const UA: &str = "KnobDeck-GitHub-Plugin";
const TIMEOUT: Duration = Duration::from_secs(10);

/// KnobDeck's GitHub OAuth app (build-time CI variable GITHUB_CLIENT_ID), overridable at run time.
pub fn client_id() -> String {
    env_or("GITHUB_CLIENT_ID", option_env!("GITHUB_CLIENT_ID").unwrap_or(""))
}

pub fn api_base() -> String {
    env_or("GITHUB_API_BASE", "https://api.github.com")
}

/// github.com: the device sign-in endpoints and the pages keys open.
pub fn web_base() -> String {
    env_or("GITHUB_WEB_BASE", "https://github.com")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Percent-encode a query or path component.
pub fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

struct Cached {
    etag: Option<String>,
    body: Value,
    next: bool,
    /// Don't ask again before this (X-Poll-Interval, or a short floor for unconditional replies).
    not_before: u64,
}

#[derive(Default)]
struct State {
    cache: HashMap<String, Cached>,
    /// Per API base: no requests before this time, and why.
    blocked: HashMap<String, (u64, String)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    f(STATE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(State::default))
}

/// A reply: the JSON body and whether a `Link: rel="next"` page follows.
#[derive(Clone, Debug)]
pub struct Reply {
    pub body: Value,
    pub next: bool,
}

/// How a failed reply reads, and whether it blocks further calls until a time.
#[derive(Debug, PartialEq)]
pub struct Failure {
    pub message: String,
    pub blocked_until: Option<u64>,
    pub signed_out: bool,
}

fn hhmm(epoch: u64) -> String {
    let left = epoch.saturating_sub(now());
    if left < 90 {
        format!("{left} s")
    } else {
        format!("{} min", left.div_ceil(60))
    }
}

/// Read a failed reply. `header` reads one response header.
pub fn failure(code: u16, header: &dyn Fn(&str) -> Option<String>, body: &Value) -> Failure {
    let message = body["message"].as_str().unwrap_or("").to_string();
    let remaining = header("x-ratelimit-remaining");
    let reset = header("x-ratelimit-reset").and_then(|v| v.parse::<u64>().ok());
    let retry = header("retry-after").and_then(|v| v.parse::<u64>().ok());
    let plain = |m: String| Failure { message: m, blocked_until: None, signed_out: false };
    match code {
        401 => Failure { message: "GitHub refused the token (revoked or expired) — run setup again".into(), blocked_until: None, signed_out: true },
        403 | 429 if remaining.as_deref() == Some("0") || retry.is_some() || message.to_lowercase().contains("rate limit") => {
            let until = match (retry, reset) {
                (Some(s), _) => now() + s,
                (None, Some(r)) => r,
                // Secondary limit without a hint: GitHub asks for at least a minute.
                _ => now() + 60,
            };
            Failure { message: format!("GitHub rate limit reached; trying again in {}", hhmm(until)), blocked_until: Some(until), signed_out: false }
        }
        403 => plain(format!("GitHub refused: {} (the token lacks a permission for this)", if message.is_empty() { "forbidden" } else { &message })),
        404 => plain("Not found on GitHub (check the repository and workflow, and that the token can see them)".into()),
        422 => plain(format!("GitHub rejected the request: {}", if message.is_empty() { "invalid" } else { &message })),
        _ => plain(format!("GitHub said: {} (HTTP {code})", if message.is_empty() { "error" } else { &message })),
    }
}

/// The API at one base with one token. `Client::get()` is the plugin's.
#[derive(Clone)]
pub struct Client {
    pub api: String,
    pub token: String,
}

impl Client {
    /// The plugin's client: the stored token, or a "run setup" error.
    pub fn get() -> Result<Client> {
        let token = secrets::env_or_stored(TOKEN_KEY);
        if token.is_empty() {
            knobdeck_sdk::mark_setup_required("Not signed in to GitHub");
            return Err(Error::msg("Not signed in to GitHub — run the plugin's setup"));
        }
        Ok(Client { api: api_base(), token })
    }

    fn request(&self, method: &str, url: &str) -> ureq::Request {
        ureq::request(method, url)
            .timeout(TIMEOUT)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Accept", "application/vnd.github+json")
            .set("X-GitHub-Api-Version", "2022-11-28")
            .set("User-Agent", UA)
    }

    fn check_blocked(&self) -> Option<String> {
        with_state(|s| match s.blocked.get(&self.api) {
            Some((until, why)) if *until > now() => Some(why.clone()),
            _ => None,
        })
    }

    fn fail(&self, code: u16, r: ureq::Response) -> Error {
        let headers: HashMap<String, String> = r.headers_names().into_iter().filter_map(|n| r.header(&n).map(|v| (n.to_lowercase(), v.to_string()))).collect();
        let body: Value = r.into_json().unwrap_or(Value::Null);
        let f = failure(code, &|h| headers.get(h).cloned(), &body);
        if let Some(until) = f.blocked_until {
            with_state(|s| s.blocked.insert(self.api.clone(), (until, f.message.clone())));
        }
        if f.signed_out {
            knobdeck_sdk::mark_setup_required("GitHub refused the token");
        }
        Error::msg(f.message)
    }

    /// GET `path` (from the API base), conditional on the last ETag. While rate-limited or inside
    /// the poll interval, the last reply is returned without asking.
    pub fn get_reply(&self, path: &str) -> Result<Reply> {
        let url = format!("{}{path}", self.api);
        let (etag, cached, due) = with_state(|s| match s.cache.get(&url) {
            Some(c) => (c.etag.clone(), Some(Reply { body: c.body.clone(), next: c.next }), c.not_before <= now()),
            None => (None, None, true),
        });
        if let Some(why) = self.check_blocked() {
            return cached.ok_or_else(|| Error::msg(why));
        }
        if let (Some(c), false) = (&cached, due) {
            return Ok(c.clone());
        }
        let mut req = self.request("GET", &url);
        if let Some(e) = &etag {
            req = req.set("If-None-Match", e);
        }
        match req.call() {
            Ok(r) if r.status() == 304 => {
                let poll = r.header("x-poll-interval").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                with_state(|s| s.cache.get_mut(&url).map(|c| c.not_before = now() + poll));
                cached.ok_or_else(|| Error::msg("GitHub sent an empty reply"))
            }
            Ok(r) => {
                let etag = r.header("etag").map(String::from);
                let next = r.header("link").is_some_and(|l| l.contains("rel=\"next\""));
                let poll = r.header("x-poll-interval").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
                let body: Value = r.into_json().map_err(|e| Error::msg(format!("GitHub sent unreadable JSON ({e})")))?;
                // Without an ETag every ask costs; keep the reply at least 30 s then.
                let not_before = now() + if etag.is_some() { poll } else { poll.max(30) };
                with_state(|s| s.cache.insert(url, Cached { etag, body: body.clone(), next, not_before }));
                Ok(Reply { body, next })
            }
            // ureq 2 treats 304 as a success; this arm is kept for proxies that turn it into an error.
            Err(ureq::Error::Status(304, _)) => cached.ok_or_else(|| Error::msg("GitHub sent an empty reply")),
            Err(ureq::Error::Status(code, r)) => Err(self.fail(code, r)),
            Err(e) => Err(Error::msg(format!("GitHub is unreachable ({e})"))),
        }
    }

    pub fn get_json(&self, path: &str) -> Result<Value> {
        self.get_reply(path).map(|r| r.body)
    }

    /// POST a JSON body; returns the status and the body (`Null` when empty).
    pub fn post(&self, path: &str, body: &Value) -> Result<(u16, Value)> {
        if let Some(why) = self.check_blocked() {
            return Err(Error::msg(why));
        }
        match self.request("POST", &format!("{}{path}", self.api)).send_json(body.clone()) {
            Ok(r) => {
                let code = r.status();
                let text = r.into_string().unwrap_or_default();
                Ok((code, text.parse().unwrap_or(Value::Null)))
            }
            Err(ureq::Error::Status(code, r)) => Err(self.fail(code, r)),
            Err(e) => Err(Error::msg(format!("GitHub is unreachable ({e})"))),
        }
    }

    /// One GraphQL query (GraphQL has no ETags; callers cache its replies).
    pub fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        let (_, reply) = self.post("/graphql", &knobdeck_sdk::json!({"query": query, "variables": variables}))?;
        if let Some(e) = reply["errors"][0]["message"].as_str() {
            return Err(Error::msg(format!("GitHub said: {e}")));
        }
        Ok(reply["data"].clone())
    }
}

/// "owner/name" from what the user typed or picked (also accepts a github.com URL or `.git`).
pub fn repo(raw: &str) -> Result<String> {
    let s = raw.trim().trim_end_matches('/');
    let s = s.strip_prefix("https://github.com/").or_else(|| s.strip_prefix("github.com/")).unwrap_or(s);
    let s = s.strip_suffix(".git").unwrap_or(s);
    let ok = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match s.split_once('/') {
        Some((o, n)) if ok(o) && ok(n) && n != "." && n != ".." => Ok(format!("{o}/{n}")),
        _ if s.is_empty() => Err(Error::msg("Choose a repository")),
        _ => Err(Error::msg(format!("\"{}\" is not a repository (use owner/name)", raw.trim()))),
    }
}

// ---- Device sign-in (setup) -------------------------------------------------------------------

/// What one poll of the device flow's token endpoint said.
#[derive(Debug, PartialEq)]
pub enum Poll {
    Pending,
    SlowDown(u64),
    Token(String),
    Failed(String),
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<Value> {
    // GitHub answers form-encoded unless asked for JSON, and reports flow errors with HTTP 200.
    match ureq::post(url).timeout(Duration::from_secs(20)).set("Accept", "application/json").set("User-Agent", UA).send_form(form) {
        Ok(r) => r.into_json().map_err(|e| Error::msg(format!("GitHub sent unreadable JSON ({e})"))),
        Err(ureq::Error::Status(code, r)) => {
            let body: Value = r.into_json().unwrap_or(Value::Null);
            Ok(if body.is_object() { body } else { knobdeck_sdk::json!({"error": format!("HTTP {code}")}) })
        }
        Err(e) => Err(Error::msg(format!("GitHub is unreachable ({e})"))),
    }
}

/// Start the device flow: the reply has `device_code`, `user_code`, `verification_uri`,
/// `interval`, `expires_in`.
pub fn device_start(web: &str, client_id: &str) -> Result<Value> {
    let r = post_form(&format!("{web}/login/device/code"), &[("client_id", client_id), ("scope", SCOPES)])?;
    if r["device_code"].as_str().is_none() {
        let why = r["error_description"].as_str().or(r["error"].as_str()).unwrap_or("no device code");
        return Err(Error::msg(format!("Could not start GitHub sign-in: {why}")));
    }
    Ok(r)
}

pub fn device_poll(web: &str, client_id: &str, device_code: &str) -> Result<Poll> {
    let r = post_form(&format!("{web}/login/oauth/access_token"), &[("client_id", client_id), ("device_code", device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")])?;
    if let Some(t) = r["access_token"].as_str() {
        return Ok(Poll::Token(t.to_string()));
    }
    Ok(match r["error"].as_str().unwrap_or("") {
        "authorization_pending" => Poll::Pending,
        "slow_down" => Poll::SlowDown(r["interval"].as_u64().unwrap_or(0)),
        "access_denied" => Poll::Failed("Sign-in was declined".into()),
        "expired_token" => Poll::Failed("The sign-in code expired — run setup again".into()),
        "device_flow_disabled" => Poll::Failed("Device sign-in is turned off for this GitHub app".into()),
        other => Poll::Failed(format!("GitHub sign-in failed: {}", r["error_description"].as_str().unwrap_or(if other.is_empty() { "unknown error" } else { other }))),
    })
}

/// Poll until the user approves (or the code expires). `interval` and `expires_in` come from
/// [`device_start`]'s reply.
pub fn device_wait(web: &str, client_id: &str, device_code: &str, mut interval: u64, expires_in: u64) -> Result<String> {
    let deadline = now() + expires_in.min(900);
    while now() < deadline {
        std::thread::sleep(Duration::from_secs(interval.max(1)));
        match device_poll(web, client_id, device_code)? {
            Poll::Pending => {}
            Poll::SlowDown(n) => interval = if n > interval { n } else { interval + 5 },
            Poll::Token(t) => return Ok(t),
            Poll::Failed(m) => return Err(Error::msg(m)),
        }
    }
    Err(Error::msg("GitHub sign-in timed out — run setup again"))
}

#[cfg(test)]
pub mod mock {
    //! A tiny HTTP server answering canned replies in order and recording each request.
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    pub struct Mock {
        pub base: String,
        pub requests: mpsc::Receiver<String>,
    }

    /// `replies`: (status line code, extra header lines, body).
    pub fn serve(replies: Vec<(u16, &'static str, &'static str)>) -> Mock {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (code, headers, body) in replies {
                let (mut s, _) = listener.accept().unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let (mut req, mut len) = (String::new(), 0);
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    req.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut b = vec![0u8; len];
                r.read_exact(&mut b).unwrap();
                req.push_str(&String::from_utf8_lossy(&b));
                let _ = tx.send(req);
                let reason = match code {
                    200 => "OK",
                    201 => "Created",
                    204 => "No Content",
                    304 => "Not Modified",
                    _ => "Error",
                };
                let body = if code == 204 || code == 304 { "" } else { body };
                write!(s, "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        Mock { base, requests: rx }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    fn client(base: &str) -> Client {
        Client { api: base.to_string(), token: "t0k".into() }
    }

    #[test]
    fn repo_names() {
        assert_eq!(repo(" Knobdeck/knobdeck-plugins ").unwrap(), "Knobdeck/knobdeck-plugins");
        assert_eq!(repo("https://github.com/rust-lang/rust.git").unwrap(), "rust-lang/rust");
        assert_eq!(repo("a/b/").unwrap(), "a/b");
        assert!(repo("nope").is_err());
        assert!(repo("a/b c").is_err());
        assert!(repo("a/..").is_err());
        assert!(repo("").unwrap_err().message().contains("Choose"));
    }

    #[test]
    fn failures_read_well() {
        let none = |_: &str| None;
        assert!(failure(401, &none, &json!({})).signed_out);
        let f = failure(403, &|h| match h {
            "x-ratelimit-remaining" => Some("0".into()),
            "x-ratelimit-reset" => Some((now() + 120).to_string()),
            _ => None,
        }, &json!({"message": "API rate limit exceeded"}));
        assert!(f.blocked_until.is_some() && f.message.contains("rate limit"), "{f:?}");
        let f = failure(429, &|h| (h == "retry-after").then(|| "30".to_string()), &json!({}));
        assert!(f.blocked_until.unwrap() >= now() + 29);
        let f = failure(403, &none, &json!({"message": "Resource not accessible by personal access token"}));
        assert!(f.blocked_until.is_none() && f.message.contains("not accessible"));
        assert!(failure(404, &none, &json!({})).message.contains("Not found"));
    }

    #[test]
    fn etag_is_sent_and_304_reuses_the_reply() {
        let m = mock::serve(vec![(200, "ETag: \"abc\"\r\nLink: <x>; rel=\"next\"\r\n", r#"[{"id":1}]"#), (304, "", "")]);
        let c = client(&m.base);
        let r = c.get_reply("/notifications").unwrap();
        assert!(r.next && r.body[0]["id"] == 1);
        let first = m.requests.recv().unwrap();
        assert!(first.starts_with("GET /notifications ") && first.to_lowercase().contains("authorization: bearer t0k"), "{first}");
        assert!(!first.to_lowercase().contains("if-none-match"));
        let r = c.get_reply("/notifications").unwrap();
        assert_eq!(r.body[0]["id"], 1);
        let second = m.requests.recv().unwrap().to_lowercase();
        assert!(second.contains("if-none-match: \"abc\""), "{second}");
    }

    #[test]
    fn poll_interval_is_honoured() {
        let m = mock::serve(vec![(200, "ETag: \"e\"\r\nX-Poll-Interval: 60\r\n", "[]")]);
        let c = client(&m.base);
        c.get_reply("/notifications?x").unwrap();
        // Inside the interval: answered from the cache (the mock would refuse a second connection).
        assert!(c.get_reply("/notifications?x").unwrap().body.as_array().unwrap().is_empty());
    }

    #[test]
    fn rate_limit_blocks_until_reset() {
        let m = mock::serve(vec![(403, "X-RateLimit-Remaining: 0\r\nX-RateLimit-Reset: 9999999999\r\n", r#"{"message":"API rate limit exceeded"}"#)]);
        let c = client(&m.base);
        assert!(c.get_reply("/user").unwrap_err().message().contains("rate limit"));
        // No second request goes out (the mock serves one).
        assert!(c.get_reply("/user").unwrap_err().message().contains("rate limit"));
        assert!(c.post("/x", &json!({})).unwrap_err().message().contains("rate limit"));
    }

    #[test]
    fn device_flow_polls_until_token() {
        let m = mock::serve(vec![
            (200, "", r#"{"device_code":"dc","user_code":"ABCD-1234","verification_uri":"https://github.com/login/device","interval":1,"expires_in":900}"#),
            (200, "", r#"{"error":"authorization_pending"}"#),
            (200, "", r#"{"access_token":"gho_x","token_type":"bearer","scope":"repo,notifications"}"#),
        ]);
        let d = device_start(&m.base, "cid").unwrap();
        let start = m.requests.recv().unwrap();
        assert!(start.starts_with("POST /login/device/code ") && start.contains("client_id=cid") && (start.contains("scope=repo+notifications") || start.contains("scope=repo%20notifications")), "{start}");
        assert!(start.to_lowercase().contains("accept: application/json"));
        let t = device_wait(&m.base, "cid", d["device_code"].as_str().unwrap(), 1, 900).unwrap();
        assert_eq!(t, "gho_x");
        let poll = m.requests.recv().unwrap();
        assert!(poll.starts_with("POST /login/oauth/access_token ") && poll.contains("device_code=dc") && poll.contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"), "{poll}");
    }

    #[test]
    fn device_errors() {
        let m = mock::serve(vec![(200, "", r#"{"error":"access_denied"}"#), (200, "", r#"{"error":"slow_down","interval":10}"#)]);
        assert_eq!(device_poll(&m.base, "c", "d").unwrap(), Poll::Failed("Sign-in was declined".into()));
        assert_eq!(device_poll(&m.base, "c", "d").unwrap(), Poll::SlowDown(10));
    }
}
