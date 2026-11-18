//! AppleScript text for Music.app and parsing of what it returns. Free of macOS APIs, so the tests
//! run on every OS.

/// Returned by a guarded script when Music isn't open (a `tell` alone would launch it).
pub const NOT_RUNNING: &str = "__not_running__";

/// An AppleScript string literal.
pub fn literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `body` inside `tell application "Music"`, only when Music already runs.
pub fn guarded(body: &str) -> String {
    format!("if application \"Music\" is running then\ntell application \"Music\"\n{body}\nend tell\nelse\nreturn \"{NOT_RUNNING}\"\nend if")
}

/// `body` inside `tell application "Music"`: launches Music when it isn't open (play actions only).
pub fn launching(body: &str) -> String {
    format!("tell application \"Music\"\n{body}\nend tell")
}

/// One read of everything the keys show. `«class pLov»` is the track's favorite flag: "loved" up to
/// macOS 13, renamed "favorited" in macOS 14 with the same code, so the script compiles on both.
pub const STATUS: &str = r#"set st to player state as string
set sh to shuffle enabled as string
set rp to song repeat as string
set vol to sound volume as string
set fav to "false"
set pid to ""
set t to ""
set a to ""
if st is not "stopped" then
try
set pid to persistent ID of current track
set t to name of current track
set a to artist of current track
set fav to («class pLov» of current track) as string
end try
end if
return st & linefeed & sh & linefeed & rp & linefeed & vol & linefeed & fav & linefeed & pid & linefeed & t & linefeed & a"#;

pub const TOGGLE_FAVORITE: &str = "if player state is stopped then error \"Nothing is playing\"\nset «class pLov» of current track to not («class pLov» of current track)\nreturn («class pLov» of current track) as string";

pub const PLAYLISTS: &str = "set AppleScript's text item delimiters to linefeed\nreturn (name of every playlist) as text";

/// Writes the current track's first artwork to `path` (raw image bytes); "none" when it has none.
pub fn save_artwork(path: &str) -> String {
    format!(
        r#"if application "Music" is not running then return "{NOT_RUNNING}"
tell application "Music"
if player state is stopped then return "none"
if (count of artworks of current track) is 0 then return "none"
set d to raw data of artwork 1 of current track
end tell
set f to open for access (POSIX file {}) with write permission
try
set eof f to 0
write d to f
close access f
on error e
close access f
error e
end try
return "ok""#,
        literal(path)
    )
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// "playing", "paused", "stopped", "fast forwarding", "rewinding".
    pub state: String,
    pub shuffle: bool,
    /// "off", "one", "all".
    pub repeat: String,
    pub volume: f64,
    pub favorite: bool,
    pub track_id: String,
    pub title: String,
    pub artist: String,
}

impl Status {
    pub fn playing(&self) -> bool {
        self.state == "playing" || self.state == "fast forwarding" || self.state == "rewinding"
    }
}

pub fn parse_status(out: &str) -> Status {
    let mut l = out.split('\n');
    let mut next = || l.next().unwrap_or("").trim_end_matches('\r').to_string();
    Status {
        state: next(),
        shuffle: next() == "true",
        repeat: next(),
        // AppleScript may write numbers with the user's decimal comma.
        volume: next().replace(',', ".").parse().unwrap_or(0.0),
        favorite: next() == "true",
        track_id: next(),
        title: next(),
        artist: next(),
    }
}

/// off → all → one → off (the order Music's repeat button goes through).
pub fn next_repeat(current: &str) -> &'static str {
    match current {
        "off" => "all",
        "all" => "one",
        _ => "off",
    }
}

pub fn parse_playlists(out: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    out.split('\n').map(|s| s.trim_end_matches('\r')).filter(|s| !s.is_empty() && seen.insert(s.to_string())).map(String::from).collect()
}

/// The message of an `osascript` failure, without the "execution error:" noise.
pub fn clean_error(stderr: &str) -> String {
    let s = stderr.trim();
    if s.contains("(-1743)") {
        return "Allow KnobDeck to control Music in System Settings ▸ Privacy & Security ▸ Automation".into();
    }
    if s.contains("(-600)") {
        return "Music isn't open".into();
    }
    let s = s.split_once("execution error: ").map_or(s, |(_, m)| m);
    // "Music got an error: Nothing is playing (-2700)" → "Nothing is playing".
    let s = s.split_once("got an error: ").map_or(s, |(_, m)| m);
    let s = match s.rfind(" (-") {
        Some(i) if s.ends_with(')') => &s[..i],
        _ => s,
    };
    s.trim_end_matches('.').to_string()
}

pub fn track_label(s: &Status) -> String {
    match (s.title.is_empty(), s.artist.is_empty()) {
        (true, _) => "Music".into(),
        (false, true) => s.title.clone(),
        (false, false) => format!("{}\n{}", s.title, s.artist),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals() {
        assert_eq!(literal(r#"Mix "1" \ 2"#), r#""Mix \"1\" \\ 2""#);
    }

    #[test]
    fn status() {
        let s = parse_status("playing\ntrue\nall\n65\nfalse\nABC123\nSong\nBand");
        assert!(s.playing() && s.shuffle && !s.favorite);
        assert_eq!((s.repeat.as_str(), s.volume, s.track_id.as_str()), ("all", 65.0, "ABC123"));
        assert_eq!(track_label(&s), "Song\nBand");
        let stopped = parse_status("stopped\nfalse\noff\n40,0\nfalse\n\n\n");
        assert!(!stopped.playing());
        assert_eq!(stopped.volume, 40.0);
        assert_eq!(track_label(&stopped), "Music");
    }

    #[test]
    fn repeat_cycle() {
        assert_eq!(next_repeat("off"), "all");
        assert_eq!(next_repeat("all"), "one");
        assert_eq!(next_repeat("one"), "off");
    }

    #[test]
    fn playlists() {
        assert_eq!(parse_playlists("Library\nMusic\n\nChill\nMusic\n"), vec!["Library", "Music", "Chill"]);
    }

    #[test]
    fn errors() {
        assert!(clean_error("execution error: Not authorized to send Apple events to Music. (-1743)").contains("Automation"));
        assert_eq!(clean_error("123:140: execution error: Music got an error: Nothing is playing (-2700)"), "Nothing is playing");
        assert_eq!(clean_error("execution error: Music got an error: Can’t get playlist \"X\". (-1728)"), "Can’t get playlist \"X\"");
    }

    #[test]
    fn scripts_are_guarded() {
        assert!(guarded("playpause").starts_with("if application \"Music\" is running"));
        let art = save_artwork("/tmp/a \"b\".img");
        assert!(art.contains(r#"POSIX file "/tmp/a \"b\".img""#));
        assert!(art.starts_with("if application \"Music\" is not running"));
    }
}
