//! Text helpers for mac-tools' automation actions, free of macOS APIs (tested on every OS).

use knobdeck_sdk::os::placeholders::Escape;
use knobdeck_sdk::{Error, Result};

/// An AppleScript string literal.
pub fn literal(s: &str) -> String {
    knobdeck_sdk::os::placeholders::expand("{selection:applescript}", Escape::Raw, |_| Ok(s.to_string())).unwrap_or_default()
}

/// The message of an `osascript` / `shortcuts` failure, without the "execution error:" noise.
pub fn clean_error(stderr: &str) -> String {
    let s = stderr.trim();
    let s = s.split_once("execution error: ").map_or(s, |(_, m)| m);
    if s.contains("(-1743)") {
        return "Allow KnobDeck to control that app in System Settings ▸ Privacy & Security ▸ Automation".into();
    }
    s.to_string()
}

pub fn parse_km_macros(xml: &[u8]) -> Result<Vec<(String, String)>> {
    let v = plist::Value::from_reader_xml(xml).map_err(|e| Error::msg(format!("Keyboard Maestro's macro list: {e}")))?;
    let mut out = vec![];
    for group in v.as_array().into_iter().flatten() {
        let Some(g) = group.as_dictionary() else { continue };
        let gname = g.get("name").and_then(|n| n.as_string()).unwrap_or("");
        for m in g.get("macros").and_then(|m| m.as_array()).into_iter().flatten() {
            let Some(m) = m.as_dictionary() else { continue };
            let (Some(uid), Some(name)) = (m.get("uid").and_then(|u| u.as_string()), m.get("name").and_then(|n| n.as_string())) else { continue };
            let label = if gname.is_empty() { name.to_string() } else { format!("{gname} ▸ {name}") };
            out.push((uid.to_string(), label));
        }
    }
    out.sort_by_key(|(_, l)| l.to_lowercase());
    Ok(out)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_escape() {
        assert_eq!(literal("a \"b\"\n"), r#""a \"b\"\n""#);
    }

    #[test]
    fn errors_are_cleaned() {
        assert_eq!(clean_error("0:12: execution error: Finder got an error: nope (-10000)\n"), "Finder got an error: nope (-10000)");
        assert!(clean_error("execution error: Not authorized to send Apple events to Finder. (-1743)").contains("Automation"));
    }

    #[test]
    fn km_macro_list() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><array>
            <dict><key>name</key><string>Global</string><key>macros</key><array>
              <dict><key>name</key><string>Resize</string><key>uid</key><string>U1</string></dict>
              <dict><key>name</key><string>Append</string><key>uid</key><string>U2</string></dict>
            </array></dict></array></plist>"#;
        let m = parse_km_macros(xml).unwrap();
        assert_eq!(m, vec![("U2".into(), "Global ▸ Append".into()), ("U1".into(), "Global ▸ Resize".into())]);
    }
}
