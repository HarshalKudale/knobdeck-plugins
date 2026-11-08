//! Installed games: Steam (every OS) and the Epic Games Launcher (Windows and macOS), read from
//! the launchers' own files. Games are launched through the launchers' URL schemes, so Steam or
//! Epic starts the game the same way their library's Play button does.

use base64::Engine;
use knobdeck_sdk::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Game {
    /// `steam:<appid>` or `epic:<namespace>:<item id>:<app name>`.
    pub key: String,
    pub name: String,
    pub store: &'static str,
    /// Unix seconds (Steam only; 0 when never played or unknown).
    pub last_played: u64,
    pub icon: Option<PathBuf>,
}

pub fn launch_url(key: &str) -> Option<String> {
    match key.split_once(':')? {
        ("steam", id) if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) => Some(format!("steam://rungameid/{id}")),
        ("epic", rest) => {
            let parts: Vec<&str> = rest.splitn(3, ':').collect();
            (parts.len() == 3 && parts.iter().all(|p| !p.is_empty())).then(|| format!("com.epicgames.launcher://apps/{}%3A{}%3A{}?action=launch&silent=true", enc(parts[0]), enc(parts[1]), enc(parts[2])))
        }
        _ => None,
    }
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

// ---------- Valve KeyValues (.vdf / .acf) ----------

#[derive(Debug, Clone, PartialEq)]
pub enum Kv {
    Str(String),
    Map(BTreeMap<String, Kv>),
}

impl Kv {
    pub fn get(&self, key: &str) -> Option<&Kv> {
        match self {
            // Keys are case-insensitive in Valve's format ("LastPlayed" vs "lastplayed").
            Kv::Map(m) => m.get(key).or_else(|| m.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)),
            Kv::Str(_) => None,
        }
    }
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Kv::Str(s) => Some(s),
            Kv::Map(_) => None,
        }
    }
    pub fn entries(&self) -> Vec<(&String, &Kv)> {
        match self {
            Kv::Map(m) => m.iter().collect(),
            Kv::Str(_) => vec![],
        }
    }
}

/// Parse text KeyValues: `"key" "value"` pairs and `"key" { … }` blocks. Comments (`//`) and
/// conditionals (`[$WIN32]`) are skipped.
pub fn parse_vdf(text: &str) -> Kv {
    let mut tokens = vec![];
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some(o) => s.push(o),
                            None => break,
                        },
                        '"' => break,
                        o => s.push(o),
                    }
                }
                tokens.push(Some(s));
            }
            '{' => tokens.push(None),
            '}' => tokens.push(Some("\u{0}}".into())),
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '[' => {
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                }
            }
            c if c.is_whitespace() => {}
            c => {
                // Unquoted token.
                let mut s = c.to_string();
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '{' || n == '}' || n == '"' {
                        break;
                    }
                    s.push(n);
                    chars.next();
                }
                tokens.push(Some(s));
            }
        }
    }
    let mut it = tokens.into_iter().peekable();
    Kv::Map(block(&mut it))
}

fn block(it: &mut std::iter::Peekable<std::vec::IntoIter<Option<String>>>) -> BTreeMap<String, Kv> {
    let mut m = BTreeMap::new();
    while let Some(tok) = it.next() {
        let Some(key) = tok else { continue };
        if key == "\u{0}}" {
            break;
        }
        match it.next() {
            Some(None) => {
                m.insert(key, Kv::Map(block(it)));
            }
            Some(Some(v)) if v != "\u{0}}" => {
                m.insert(key, Kv::Str(v));
            }
            _ => break,
        }
    }
    m
}

// ---------- Steam ----------

#[cfg(not(windows))]
fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// Steam install folders that exist on this computer.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    if let Some(p) = std::env::var_os("KNOBDECK_STEAM_ROOT") {
        v.push(p.into()); // tests
    }
    #[cfg(windows)]
    {
        if let Some(p) = os::steam_path_from_registry() {
            v.push(p);
        }
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(pf) = std::env::var_os(var) {
                v.push(PathBuf::from(pf).join("Steam"));
            }
        }
    }
    #[cfg(target_os = "macos")]
    v.push(home().join("Library/Application Support/Steam"));
    #[cfg(target_os = "linux")]
    {
        let h = home();
        v.push(h.join(".local/share/Steam"));
        v.push(h.join(".steam/steam"));
        v.push(h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam")); // Flatpak
        v.push(h.join(".var/app/com.valvesoftware.Steam/data/Steam"));
        v.push(h.join("snap/steam/common/.local/share/Steam"));
    }
    let mut seen = vec![];
    v.into_iter()
        .filter(|p| p.join("steamapps").is_dir())
        .filter(|p| {
            let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            let new = !seen.contains(&real);
            seen.push(real);
            new
        })
        .collect()
}

/// Apps Steam installs that aren't games (runtimes, Proton, redistributables).
fn is_tool(appid: &str, name: &str) -> bool {
    const IDS: &[&str] = &["228980", "1070560", "1391110", "1628350", "1493710", "2180100", "2348590", "2805730", "3658110", "1826330", "961940", "1054830", "1113280", "1245040", "1420170", "1580130", "1887720", "2230260"];
    let n = name.to_ascii_lowercase();
    IDS.contains(&appid) || n.starts_with("proton") || n.starts_with("steam linux runtime") || n.starts_with("steamworks") || n.contains("redistributable") || n.starts_with("steamvr")
}

pub fn steam_games() -> Vec<Game> {
    let mut games: Vec<Game> = vec![];
    for root in steam_roots() {
        let mut libs = vec![root.clone()];
        if let Ok(t) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            let kv = parse_vdf(&t);
            if let Some(folders) = kv.get("libraryfolders") {
                for (_, f) in folders.entries() {
                    if let Some(p) = f.str("path") {
                        let p = PathBuf::from(p.replace("\\\\", "\\"));
                        if !libs.contains(&p) {
                            libs.push(p);
                        }
                    }
                }
            }
        }
        for lib in libs {
            let Ok(dir) = std::fs::read_dir(lib.join("steamapps")) else { continue };
            for e in dir.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                    continue;
                }
                let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
                if let Some(g) = steam_game(&parse_vdf(&t), &root) {
                    if !games.iter().any(|x| x.key == g.key) {
                        games.push(g);
                    }
                }
            }
        }
    }
    games
}

fn steam_game(acf: &Kv, root: &Path) -> Option<Game> {
    let app = acf.get("AppState")?;
    let id = app.str("appid")?.to_string();
    let name = app.str("name").unwrap_or(&id).to_string();
    if is_tool(&id, &name) {
        return None;
    }
    Some(Game { key: format!("steam:{id}"), name, store: "steam", last_played: app.str("LastPlayed").and_then(|v| v.parse().ok()).unwrap_or(0), icon: steam_icon(root, &id) })
}

/// The game's small icon from Steam's library cache: `librarycache/<appid>/<sha1>.jpg` (current
/// Steam) or `librarycache/<appid>_icon.jpg` (older), else its header image.
fn steam_icon(root: &Path, id: &str) -> Option<PathBuf> {
    let cache = root.join("appcache/librarycache");
    let dir = cache.join(id);
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if let Some(stem) = n.strip_suffix(".jpg") {
                if stem.len() == 40 && stem.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Some(e.path());
                }
            }
        }
    }
    [cache.join(format!("{id}_icon.jpg")), dir.join("header.jpg"), cache.join(format!("{id}_header.jpg"))].into_iter().find(|p| p.is_file())
}

// ---------- Epic Games Launcher ----------

fn epic_manifest_dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KNOBDECK_EPIC_MANIFESTS") {
        return Some(p.into()); // tests
    }
    #[cfg(windows)]
    return Some(PathBuf::from(std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into())).join("Epic\\EpicGamesLauncher\\Data\\Manifests"));
    #[cfg(target_os = "macos")]
    return Some(home().join("Library/Application Support/Epic/EpicGamesLauncher/Data/Manifests"));
    #[allow(unreachable_code)]
    None
}

pub fn epic_game(m: &Value) -> Option<Game> {
    let (ns, item, app) = (m["CatalogNamespace"].as_str()?, m["CatalogItemId"].as_str()?, m["AppName"].as_str()?);
    // Unreal Engine, plugins and other non-game items are listed too.
    let cats: Vec<&str> = m["AppCategories"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if !cats.is_empty() && !cats.contains(&"games") {
        return None;
    }
    if m["bIsIncompleteInstall"] == true {
        return None;
    }
    Some(Game { key: format!("epic:{ns}:{item}:{app}"), name: m["DisplayName"].as_str().unwrap_or(app).to_string(), store: "epic", last_played: 0, icon: None })
}

pub fn epic_games() -> Vec<Game> {
    let Some(dir) = epic_manifest_dir() else { return vec![] };
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    rd.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "item"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
        .filter_map(|m| epic_game(&m))
        .collect()
}

pub fn all_games() -> Vec<Game> {
    let mut v = steam_games();
    v.extend(epic_games());
    v
}

/// A cached icon file as a data URI (small files only; key and wheel images).
pub fn icon_uri(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok().filter(|b| b.len() < 256 * 1024)?;
    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        _ => "image/jpeg",
    };
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[cfg(windows)]
mod os {
    use std::path::PathBuf;
    use windows::core::w;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    /// `HKCU\Software\Valve\Steam\SteamPath`, written by the Steam client.
    pub fn steam_path_from_registry() -> Option<PathBuf> {
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Valve\\Steam"), w!("SteamPath"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
        if r.is_err() {
            return None;
        }
        let n = (len as usize / 2).saturating_sub(1);
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..n.min(buf.len())]).replace('/', "\\")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use knobdeck_sdk::json;

    const ACF: &str = r#"
"AppState"
{
	"appid"		"730"
	"Universe"		"1"
	"name"		"Counter-Strike 2"
	"LastPlayed"		"1793986248"
	"UserConfig"
	{
		"language"		"english"
	}
}"#;

    #[test]
    fn vdf() {
        let kv = parse_vdf(ACF);
        let app = kv.get("AppState").unwrap();
        assert_eq!(app.str("appid"), Some("730"));
        assert_eq!(app.str("lastplayed"), Some("1793986248"));
        assert_eq!(app.get("UserConfig").unwrap().str("language"), Some("english"));
        let lib = parse_vdf("\"libraryfolders\" { \"0\" { \"path\" \"C:\\\\Program Files (x86)\\\\Steam\" // c\n } \"1\" { \"path\" \"D:\\\\Games\" } }");
        let paths: Vec<_> = lib.get("libraryfolders").unwrap().entries().iter().filter_map(|(_, f)| f.str("path")).map(String::from).collect();
        assert_eq!(paths, ["C:\\Program Files (x86)\\Steam", "D:\\Games"]);
        let g = steam_game(&kv, Path::new("/nonexistent")).unwrap();
        assert_eq!((g.key.as_str(), g.name.as_str(), g.last_played), ("steam:730", "Counter-Strike 2", 1793986248));
        assert!(steam_game(&parse_vdf("\"AppState\" { \"appid\" \"1628350\" \"name\" \"Steam Linux Runtime 3.0 (sniper)\" }"), Path::new("/")).is_none());
    }

    #[test]
    fn epic() {
        let m = json!({"DisplayName": "Fortnite", "AppName": "Fortnite", "CatalogNamespace": "fn", "CatalogItemId": "4fe75bbc5a674f4f9b356b5c90567da5", "AppCategories": ["public", "games", "applications"]});
        let g = epic_game(&m).unwrap();
        assert_eq!(launch_url(&g.key).unwrap(), "com.epicgames.launcher://apps/fn%3A4fe75bbc5a674f4f9b356b5c90567da5%3AFortnite?action=launch&silent=true");
        assert!(epic_game(&json!({"DisplayName": "UE", "AppName": "UE_5.3", "CatalogNamespace": "ue", "CatalogItemId": "x", "AppCategories": ["engines"]})).is_none());
    }

    #[test]
    fn urls() {
        assert_eq!(launch_url("steam:570").as_deref(), Some("steam://rungameid/570"));
        assert_eq!(launch_url("steam:57a"), None);
        assert_eq!(launch_url("epic:a:b"), None);
        assert_eq!(launch_url("other:1"), None);
    }
}
