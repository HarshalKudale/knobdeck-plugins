//! The line protocol of the KnobDeck plug-in for Lightroom Classic (`Protocol.lua` in
//! `Knobdeck/knobdeck-lightroom-companion`). One message per line, tab-separated fields:
//!
//! - to Lightroom (command port): `<id> TAB <command> [TAB <arg>]...`
//! - from Lightroom (event port): `R TAB <id> TAB ok [TAB <value>]...`, `R TAB <id> TAB err TAB <message>`,
//!   `E TAB <event> [TAB <value>]...`
//!
//! Fields escape backslash, tab, line feed and carriage return as `\\`, `\t`, `\n`, `\r`.

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\t' => o.push_str("\\t"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c => o.push(c),
        }
    }
    o
}

pub fn unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('t') => o.push('\t'),
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            Some(c) => o.push(c),
            None => o.push('\\'),
        }
    }
    o
}

pub fn encode<S: AsRef<str>>(fields: &[S]) -> String {
    fields.iter().map(|f| escape(f.as_ref())).collect::<Vec<_>>().join("\t")
}

pub fn split(line: &str) -> Vec<String> {
    line.trim_end_matches(['\r', '\n']).split('\t').map(unescape).collect()
}

/// A number as Lightroom's side writes it (`%.10g`-like: no trailing zeros, no `-0`).
pub fn num(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let r = (v * 1e6).round() / 1e6;
    let s = format!("{r}");
    if s == "-0" { "0".into() } else { s }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Inbound {
    Reply { id: u64, result: Result<Vec<String>, String> },
    Event { name: String, fields: Vec<String> },
}

pub fn parse(line: &str) -> Option<Inbound> {
    let mut f = split(line);
    match f.first().map(String::as_str) {
        Some("R") if f.len() >= 3 => {
            let id = f[1].parse().ok()?;
            let rest = f.split_off(3);
            Some(Inbound::Reply {
                id,
                result: if f[2] == "ok" { Ok(rest) } else { Err(rest.into_iter().next().unwrap_or_else(|| "Lightroom refused it".into())) },
            })
        }
        Some("E") if f.len() >= 2 => {
            let fields = f.split_off(2);
            Some(Inbound::Event { name: f.pop().unwrap(), fields })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_parse() {
        let line = encode(&["7", "set", "a\tb", "c\nd", "back\\slash"]);
        assert!(!line.contains('\n'));
        assert_eq!(split(&format!("{line}\r\n")), vec!["7", "set", "a\tb", "c\nd", "back\\slash"]);
        // The lines the Lua tests check Protocol.lua writes.
        assert_eq!(parse("R\t3\tok\tExposure\t0.5"), Some(Inbound::Reply { id: 3, result: Ok(vec!["Exposure".into(), "0.5".into()]) }));
        assert_eq!(parse("R\t3\terr\tSelect a photo"), Some(Inbound::Reply { id: 3, result: Err("Select a photo".into()) }));
        assert_eq!(parse("R\t4\tok"), Some(Inbound::Reply { id: 4, result: Ok(vec![]) }));
        assert_eq!(parse("E\tvalue\tTint\t-4"), Some(Inbound::Event { name: "value".into(), fields: vec!["Tint".into(), "-4".into()] }));
        assert_eq!(parse("E\tpaired"), Some(Inbound::Event { name: "paired".into(), fields: vec![] }));
        assert_eq!(parse("garbage"), None);
        assert_eq!(parse("R\tx\tok"), None);
        assert_eq!(num(0.15000000000000002), "0.15");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(5500.0), "5500");
    }
}
