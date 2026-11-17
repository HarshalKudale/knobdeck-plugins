//! The HTTP side of Web Requests: building a request from an action's inputs, sending it, and
//! reading a value out of the reply. Shared by the plugin and its setup program.

use knobdeck_sdk::{Error, Result, Value};
use std::time::Duration;

/// Saved tokens: up to this many, kept in the OS credential store as `TOKEN_1`… with their
/// names (not secret) in `.env` as `TOKEN_1_NAME`….
pub const TOKEN_SLOTS: usize = 3;

pub fn token_key(slot: usize) -> String {
    format!("TOKEN_{slot}")
}

pub fn token_name_key(slot: usize) -> String {
    format!("TOKEN_{slot}_NAME")
}

pub const METHODS: &[(&str, &str)] = &[("GET", "GET"), ("POST", "POST"), ("PUT", "PUT"), ("PATCH", "PATCH"), ("DELETE", "DELETE"), ("HEAD", "HEAD")];

#[derive(Debug, Default, Clone)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub content_type: Option<&'static str>,
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub status_text: String,
    pub body: String,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    pub fn json(&self) -> Option<Value> {
        serde_json::from_str(&self.body).ok()
    }
}

/// `Name: value` lines. Blank lines and lines starting with `#` are skipped.
pub fn parse_headers(text: &str) -> Result<Vec<(String, String)>> {
    let mut out = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        match line.split_once(':') {
            Some((name, value)) if !name.trim().is_empty() && !name.trim().contains(' ') => out.push((name.trim().to_string(), value.trim().to_string())),
            _ => return Err(Error::msg(format!("Each header goes on its own line as Name: value (\"{line}\" isn't)"))),
        }
    }
    Ok(out)
}

/// Normalize a URL the user typed: `example.com/x` → `https://example.com/x`.
pub fn normalize_url(url: &str) -> Result<String> {
    let url = url.trim();
    if url.is_empty() {
        return Err(Error::msg("Enter the URL to call"));
    }
    if url.contains("://") {
        if url.starts_with("http://") || url.starts_with("https://") {
            Ok(url.to_string())
        } else {
            Err(Error::msg("Only http:// and https:// URLs can be called"))
        }
    } else {
        Ok(format!("https://{url}"))
    }
}

pub fn send(req: &Request, timeout: Duration) -> Result<Reply> {
    let url = normalize_url(&req.url)?;
    let method = if req.method.is_empty() { "GET" } else { req.method.as_str() };
    let mut r = ureq::request(method, &url).timeout(timeout).set("User-Agent", concat!("KnobDeck-WebRequests/", env!("CARGO_PKG_VERSION")));
    let has_ct = req.headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("content-type"));
    for (n, v) in &req.headers {
        r = r.set(n, v);
    }
    if let (Some(ct), false) = (req.content_type, has_ct) {
        if req.body.is_some() {
            r = r.set("Content-Type", ct);
        }
    }
    let res = match &req.body {
        Some(b) => r.send_string(b),
        None => r.call(),
    };
    let resp = match res {
        Ok(resp) | Err(ureq::Error::Status(_, resp)) => resp,
        Err(ureq::Error::Transport(t)) => {
            knobdeck_sdk::log(format!("{method} {url}: {t}"));
            let host = url.split("://").nth(1).and_then(|r| r.split(['/', '?']).next()).unwrap_or(&url);
            return Err(Error::msg(match t.kind() {
                ureq::ErrorKind::Dns => format!("Couldn't find {host}. Check the address"),
                ureq::ErrorKind::ConnectionFailed => format!("{host} refused the connection"),
                ureq::ErrorKind::Io if t.to_string().contains("timed out") => format!("{host} didn't answer in {} s", timeout.as_secs()),
                ureq::ErrorKind::InvalidUrl => "That URL isn't valid".to_string(),
                _ => format!("Couldn't reach {host}: {}", t.message().unwrap_or("connection failed")),
            }));
        }
    };
    let status = resp.status();
    let status_text = resp.status_text().to_string();
    // Cap what we keep: a key shows a few words, not a page.
    let mut body = String::new();
    use std::io::Read;
    let _ = resp.into_reader().take(2 * 1024 * 1024).read_to_string(&mut body);
    Ok(Reply { status, status_text, body })
}

/// The error a failed reply shows on the key: `HTTP 404 Not Found: <first words of the body>`.
pub fn status_error(reply: &Reply) -> Error {
    let snippet = reply
        .json()
        .and_then(|j| ["message", "error", "detail", "error_description"].iter().find_map(|k| j.get(*k).map(display)))
        .unwrap_or_else(|| one_line(&reply.body, 80));
    let head = format!("HTTP {} {}", reply.status, reply.status_text).trim().to_string();
    Error::msg(if snippet.is_empty() { head } else { format!("{head}: {snippet}") })
}

/// Read `path` out of a JSON value: `data.items[0].name`, `$.temp`, `list.2`, `["odd key"]`.
/// An empty path (or `$`) is the whole value.
pub fn json_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path_segments(path)? {
        cur = match seg {
            Seg::Key(k) => match cur {
                Value::Object(m) => m.get(&k)?,
                Value::Array(a) => a.get(k.parse::<usize>().ok()?)?,
                _ => return None,
            },
            Seg::Index(i) => {
                let a = cur.as_array()?;
                // Negative indexes count from the end: [-1] is the last item.
                let idx = if i < 0 { a.len().checked_sub(i.unsigned_abs() as usize)? } else { i as usize };
                a.get(idx)?
            }
        };
    }
    Some(cur)
}

#[derive(Debug, PartialEq)]
enum Seg {
    Key(String),
    Index(i64),
}

fn path_segments(path: &str) -> Option<Vec<Seg>> {
    let p = path.trim();
    let p = p.strip_prefix('$').unwrap_or(p);
    let b: Vec<char> = p.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    let mut key = String::new();
    let flush = |key: &mut String, out: &mut Vec<Seg>| {
        if !key.is_empty() {
            out.push(Seg::Key(std::mem::take(key)));
        }
    };
    while i < b.len() {
        match b[i] {
            '.' => {
                flush(&mut key, &mut out);
                i += 1;
            }
            '[' => {
                flush(&mut key, &mut out);
                let close = b[i..].iter().position(|&c| c == ']')? + i;
                let inner: String = b[i + 1..close].iter().collect();
                let inner = inner.trim();
                if let Some(q) = inner.strip_prefix('"').and_then(|s| s.strip_suffix('"')).or_else(|| inner.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))) {
                    out.push(Seg::Key(q.to_string()));
                } else {
                    out.push(Seg::Index(inner.parse().ok()?));
                }
                i = close + 1;
            }
            c => {
                key.push(c);
                i += 1;
            }
        }
    }
    flush(&mut key, &mut out);
    Some(out)
}

/// A JSON value as key text: strings as they are, whole numbers without `.0`, others compact.
pub fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "—".into(),
        Value::Bool(b) => if *b { "true" } else { "false" }.into(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", f as i64),
            Some(f) => {
                let s = format!("{f:.2}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            None => n.to_string(),
        },
        other => one_line(&other.to_string(), 60),
    }
}

/// The first `max` characters of `text` on one line.
pub fn one_line(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max - 1).collect::<String>())
    } else {
        line
    }
}

/// Escape `s` as the inside of a JSON string literal (no surrounding quotes).
pub fn json_escape(s: &str) -> String {
    let quoted = serde_json::to_string(s).unwrap_or_default();
    quoted[1..quoted.len() - 1].to_string()
}

/// Does a reply value count as "on" for a toggle? `true`, `"on"`, a nonzero number, or the
/// user's own on-value.
pub fn is_on(v: &Value, on_value: &str) -> bool {
    let on_value = on_value.trim();
    if !on_value.is_empty() {
        return display(v).eq_ignore_ascii_case(on_value);
    }
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => matches!(s.to_ascii_lowercase().as_str(), "on" | "true" | "yes" | "1" | "open" | "enabled" | "active" | "playing"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn paths() {
        let v = json!({"data": {"items": [{"name": "a"}, {"name": "b"}]}, "odd.key": 3, "n": 2.5});
        assert_eq!(json_path(&v, "data.items[1].name"), Some(&json!("b")));
        assert_eq!(json_path(&v, "$.data.items.0.name"), Some(&json!("a")));
        assert_eq!(json_path(&v, "data.items[-1].name"), Some(&json!("b")));
        assert_eq!(json_path(&v, "[\"odd.key\"]"), Some(&json!(3)));
        assert_eq!(json_path(&v, ""), Some(&v));
        assert_eq!(json_path(&v, "$"), Some(&v));
        assert_eq!(json_path(&v, "data.missing"), None);
        assert_eq!(json_path(&v, "data.items[9]"), None);
        assert_eq!(json_path(&json!([1, 2]), "[0]"), Some(&json!(1)));
    }

    #[test]
    fn displays() {
        assert_eq!(display(&json!(3.0)), "3");
        assert_eq!(display(&json!(21.456)), "21.46");
        assert_eq!(display(&json!(0.5)), "0.5");
        assert_eq!(display(&json!("hi")), "hi");
        assert_eq!(display(&json!(null)), "—");
    }

    #[test]
    fn headers() {
        let h = parse_headers("Accept: application/json\n\n# note\nX-Key:  abc:def ").unwrap();
        assert_eq!(h, vec![("Accept".into(), "application/json".into()), ("X-Key".into(), "abc:def".into())]);
        assert!(parse_headers("not a header").is_err());
    }

    #[test]
    fn urls() {
        assert_eq!(normalize_url("example.com/x").unwrap(), "https://example.com/x");
        assert_eq!(normalize_url("http://a.local").unwrap(), "http://a.local");
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url(" ").is_err());
    }

    #[test]
    fn toggles() {
        assert!(is_on(&json!(true), ""));
        assert!(is_on(&json!("ON"), ""));
        assert!(!is_on(&json!(0), ""));
        assert!(is_on(&json!("heat"), "heat"));
        assert!(!is_on(&json!("off"), "heat"));
    }

    #[test]
    fn escapes() {
        assert_eq!(json_escape("a \"b\"\n\tc"), "a \\\"b\\\"\\n\\tc");
    }
}
