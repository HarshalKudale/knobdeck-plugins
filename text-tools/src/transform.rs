//! The text transforms, pure functions (no OS calls) so they are unit tested on every OS.

use base64::Engine;

pub struct Op {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
}

pub const OPS: &[Op] = &[
    Op { id: "upper", label: "UPPERCASE", icon: "case-upper" },
    Op { id: "lower", label: "lowercase", icon: "case-lower" },
    Op { id: "title", label: "Title Case", icon: "case-sensitive" },
    Op { id: "trim", label: "Trim Spaces", icon: "scissors" },
    Op { id: "slug", label: "Slugify", icon: "link-2" },
    Op { id: "url-encode", label: "URL Encode", icon: "link" },
    Op { id: "url-decode", label: "URL Decode", icon: "unlink" },
    Op { id: "base64-encode", label: "Base64 Encode", icon: "binary" },
    Op { id: "base64-decode", label: "Base64 Decode", icon: "file-text" },
    Op { id: "json-format", label: "Format JSON", icon: "braces" },
    Op { id: "json-minify", label: "Minify JSON", icon: "minimize-2" },
];

pub fn op(id: &str) -> Option<&'static Op> {
    OPS.iter().find(|o| o.id == id)
}

/// Run transform `id` on `text`. An error is a sentence for the key ("Not valid JSON: …").
pub fn apply(id: &str, text: &str) -> Result<String, String> {
    Ok(match id {
        "upper" => text.to_uppercase(),
        "lower" => text.to_lowercase(),
        "title" => title_case(text),
        "trim" => trim(text),
        "slug" => slugify(text),
        "url-encode" => url_encode(text),
        "url-decode" => url_decode(text)?,
        "base64-encode" => base64::engine::general_purpose::STANDARD.encode(text.as_bytes()),
        "base64-decode" => base64_decode(text)?,
        "json-format" => json(text, true)?,
        "json-minify" => json(text, false)?,
        _ => return Err(format!("Unknown transform \"{id}\"")),
    })
}

/// First letter of every word upper case, the rest lower case. A word starts after whitespace or
/// punctuation other than an apostrophe ("don't" stays one word, "well-known" → "Well-Known").
pub fn title_case(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut start = true;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if start {
                out.extend(c.to_uppercase());
            } else {
                out.extend(c.to_lowercase());
            }
            start = false;
        } else {
            out.push(c);
            start = !matches!(c, '\'' | '’');
        }
    }
    out
}

/// Spaces and blank lines off both ends, trailing spaces off every line (indentation is kept).
pub fn trim(text: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().map(str::trim_end).collect();
    lines.join(if text.contains("\r\n") { "\r\n" } else { "\n" })
}

/// Latin letters with accents → their base letters, for slugs ("Crème brûlée" → "creme-brulee").
fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'æ' => "ae",
        'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'œ' => "oe",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
        'ß' => "ss",
        'ţ' | 'ť' | 'ŧ' | 'ț' => "t",
        'þ' => "th",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        _ => return None,
    })
}

/// Lower-case ASCII letters and digits joined by single hyphens: "Hello, World!" → "hello-world".
/// Accented Latin letters lose their accents; other scripts are kept as they are (lower-cased).
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dash = false;
    for c in text.chars().flat_map(char::to_lowercase) {
        let piece: Option<std::borrow::Cow<str>> = if c.is_ascii_alphanumeric() {
            Some(c.to_string().into())
        } else if let Some(f) = fold(c) {
            Some(f.into())
        } else if c.is_alphanumeric() {
            Some(c.to_string().into())
        } else {
            None
        };
        match piece {
            Some(p) => {
                if dash && !out.is_empty() {
                    out.push('-');
                }
                dash = false;
                out.push_str(&p);
            }
            // Apostrophes join: "don't" → "dont".
            None if matches!(c, '\'' | '’') => {}
            None => dash = true,
        }
    }
    out
}

/// Percent-encode everything but RFC 3986's unreserved characters (`A–Z a–z 0–9 - . _ ~`), as
/// `encodeURIComponent` does (it also keeps `!*'()`; this encodes them, which is always safe).
pub fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode `%XX` sequences (and `+` as a space, as in query strings and forms). A `%` that isn't
/// followed by two hex digits stays as it is.
pub fn url_decode(text: &str) -> Result<String, String> {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            if let (Some(h), Some(l)) = (b.get(i + 1).and_then(|&c| hex(c)), b.get(i + 2).and_then(|&c| hex(c))) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8(out).map_err(|_| "The decoded text isn't valid UTF-8 text".to_string())
}

/// Standard or URL-safe Base64, padded or not; spaces and line breaks are ignored.
pub fn base64_decode(text: &str) -> Result<String, String> {
    use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let clean = clean.trim_end_matches('=');
    if clean.is_empty() {
        return Err("Nothing to decode".into());
    }
    let bytes = if clean.contains(['-', '_']) { URL_SAFE_NO_PAD.decode(clean) } else { STANDARD_NO_PAD.decode(clean) }.map_err(|_| "That isn't Base64".to_string())?;
    String::from_utf8(bytes).map_err(|_| "That Base64 holds binary data, not text".to_string())
}

/// Pretty-print (2 spaces) or minify JSON, keeping the key order.
pub fn json(text: &str, pretty: bool) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).map_err(|e| format!("Not valid JSON: {e}"))?;
    Ok(if pretty { serde_json::to_string_pretty(&v) } else { serde_json::to_string(&v) }.unwrap_or_default())
}

/// Words, characters and lines in `text`.
#[derive(Debug, PartialEq, Eq)]
pub struct Counts {
    pub words: usize,
    pub chars: usize,
    pub lines: usize,
}

pub fn counts(text: &str) -> Counts {
    Counts { words: text.split_whitespace().count(), chars: text.chars().filter(|&c| c != '\r').count(), lines: if text.is_empty() { 0 } else { text.lines().count() } }
}

/// "1 word" / "1,234 words".
pub fn plural(n: usize, one: &str, many: &str) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{grouped} {}", if n == 1 { one } else { many })
}

/// One line for a key: the first line of `text`, cut to `max` characters with "…".
pub fn preview(text: &str, max: usize) -> String {
    let line = text.trim().lines().next().unwrap_or("").trim();
    let cut: String = line.chars().take(max).collect();
    if cut.chars().count() < line.chars().count() || text.trim().lines().nth(1).is_some() {
        format!("{cut}…")
    } else {
        cut
    }
}
