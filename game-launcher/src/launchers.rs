//! More launchers, each read from its own files and started through its own URL scheme or
//! command line:
//!
//! - GOG Galaxy (Windows): `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>` (written by Galaxy and
//!   by GOG's offline installers); started with `GalaxyClient.exe /command=runGame /gameId=<id>
//!   /path="<dir>"`, or the game's own exe when Galaxy isn't installed.
//! - Ubisoft Connect (Windows): its `Uplay Install <id>` uninstall entries; `uplay://launch/<id>/0`.
//! - Heroic (every OS): its Epic (Legendary), GOG, Amazon (Nile) and sideloaded libraries in its
//!   config folder; `heroic://launch?appName=<app>&runner=<runner>`.
//! - Lutris (Linux): the `games` table of its `pga.db`; `lutris:rungameid/<id>`.

use crate::library::Game;
use knobdeck_sdk::Value;
use std::path::{Path, PathBuf};

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

// ---------- Heroic ----------

/// Heroic's config folders that exist (native and Flatpak).
fn heroic_roots() -> Vec<PathBuf> {
    if let Some(p) = std::env::var_os("KNOBDECK_HEROIC_CONFIG") {
        return vec![p.into()]; // tests
    }
    let mut v: Vec<PathBuf> = vec![];
    #[cfg(windows)]
    if let Some(a) = std::env::var_os("APPDATA") {
        v.push(PathBuf::from(a).join("heroic"));
    }
    #[cfg(target_os = "macos")]
    v.push(crate::library::home().join("Library/Application Support/heroic"));
    #[cfg(target_os = "linux")]
    {
        let h = crate::library::home();
        v.push(std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| h.join(".config")).join("heroic"));
        v.push(h.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"));
    }
    v.into_iter().filter(|p| p.is_dir()).collect()
}

fn heroic(runner: &str, app: &str, title: Option<&str>, dir: Option<PathBuf>, exe: Option<PathBuf>) -> Game {
    Game { key: format!("heroic:{runner}:{app}"), name: title.filter(|t| !t.is_empty()).unwrap_or(app).to_string(), store: "heroic", last_played: 0, icon: None, dir, exe }
}

/// Games installed through Heroic under `root` (its config folder).
pub fn heroic_games_in(root: &Path) -> Vec<Game> {
    let mut v = vec![];
    // Epic, through Legendary: `{ "<app name>": { title, install_path, is_dlc, … } }`.
    if let Some(Value::Object(m)) = read_json(&root.join("legendaryConfig/legendary/installed.json")) {
        for (app, g) in m {
            if g["is_dlc"] == true {
                continue;
            }
            v.push(heroic("legendary", &app, g["title"].as_str(), g["install_path"].as_str().map(PathBuf::from), None));
        }
    }
    // GOG: `{ "installed": [ { appName, install_path, … } ] }`; titles in the library cache.
    if let Some(inst) = read_json(&root.join("gog_store/installed.json")) {
        let lib = read_json(&root.join("store_cache/gog_library.json")).unwrap_or_default();
        let title = |app: &str| lib["games"].as_array().and_then(|a| a.iter().find(|g| g["app_name"] == app)).and_then(|g| g["title"].as_str().map(String::from));
        for g in inst["installed"].as_array().into_iter().flatten() {
            let Some(app) = g["appName"].as_str() else { continue };
            v.push(heroic("gog", app, title(app).as_deref(), g["install_path"].as_str().map(PathBuf::from), None));
        }
    }
    // Amazon, through Nile: `[ { id, path, … } ]`; titles in `library.json` (`[ { product: { id, title } } ]`).
    if let Some(Value::Array(inst)) = read_json(&root.join("nile_config/nile/installed.json")) {
        let lib = read_json(&root.join("nile_config/nile/library.json")).unwrap_or_default();
        let title = |id: &str| lib.as_array().and_then(|a| a.iter().find(|g| g["product"]["id"] == id)).and_then(|g| g["product"]["title"].as_str().map(String::from));
        for g in inst {
            let Some(id) = g["id"].as_str() else { continue };
            v.push(heroic("nile", id, title(id).as_deref(), g["path"].as_str().map(PathBuf::from), None));
        }
    }
    // Sideloaded apps: `{ "games": [ { app_name, title, is_installed, install: { executable } } ] }`.
    // Browser apps have no executable; their folder may be shared, so only the exe counts.
    if let Some(lib) = read_json(&root.join("sideload_apps/library.json")) {
        for g in lib["games"].as_array().into_iter().flatten() {
            let Some(app) = g["app_name"].as_str() else { continue };
            if g["is_installed"] == false {
                continue;
            }
            let exe = g["install"]["executable"].as_str().filter(|e| !e.is_empty()).map(PathBuf::from);
            v.push(heroic("sideload", app, g["title"].as_str(), None, exe));
        }
    }
    v
}

pub fn heroic_games() -> Vec<Game> {
    let mut v: Vec<Game> = vec![];
    for root in heroic_roots() {
        for g in heroic_games_in(&root) {
            if !v.iter().any(|x| x.key == g.key) {
                v.push(g);
            }
        }
    }
    v
}

// ---------- Lutris ----------

#[cfg(target_os = "linux")]
fn lutris_dbs() -> Vec<PathBuf> {
    if let Some(p) = std::env::var_os("KNOBDECK_LUTRIS_DB") {
        return vec![p.into()]; // tests
    }
    let h = crate::library::home();
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| h.join(".local/share"));
    [data.join("lutris/pga.db"), h.join(".var/app/net.lutris.Lutris/data/lutris/pga.db")].into_iter().filter(|p| p.is_file()).collect()
}

/// Installed games in a Lutris `pga.db`. Steam games Lutris lists (runner `steam`) are left
/// out: the Steam store already has them.
#[cfg(target_os = "linux")]
pub fn lutris_games_in(db: &Path) -> Vec<Game> {
    use rusqlite::{Connection, OpenFlags};
    let Ok(c) = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX) else { return vec![] };
    let _ = c.busy_timeout(std::time::Duration::from_millis(500));
    let icons = db.parent().and_then(Path::parent).map(|share| share.join("icons/hicolor/128x128/apps"));
    let Ok(mut q) = c.prepare("SELECT id, name, slug, directory, lastplayed FROM games WHERE installed = 1 AND COALESCE(runner, '') <> 'steam'") else { return vec![] };
    let rows = q.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<i64>>(4)?))
    });
    let Ok(rows) = rows else { return vec![] };
    rows.flatten()
        .map(|(id, name, slug, dir, last)| {
            let slug = slug.unwrap_or_default();
            let icon = icons.as_ref().map(|d| d.join(format!("lutris_{slug}.png"))).filter(|p| !slug.is_empty() && p.is_file());
            Game {
                key: format!("lutris:{id}"),
                name: name.filter(|n| !n.is_empty()).unwrap_or_else(|| slug.clone()),
                store: "lutris",
                last_played: last.unwrap_or(0).max(0) as u64,
                icon,
                dir: dir.filter(|d| !d.is_empty()).map(PathBuf::from),
                exe: None,
            }
        })
        .collect()
}

pub fn lutris_games() -> Vec<Game> {
    #[cfg(target_os = "linux")]
    {
        let mut v: Vec<Game> = vec![];
        for db in lutris_dbs() {
            for g in lutris_games_in(&db) {
                if !v.iter().any(|x| x.key == g.key) {
                    v.push(g);
                }
            }
        }
        v
    }
    #[cfg(not(target_os = "linux"))]
    vec![]
}

// ---------- GOG Galaxy and Ubisoft Connect (Windows registry) ----------

pub fn gog_games() -> Vec<Game> {
    #[cfg(windows)]
    {
        os::gog_games()
    }
    #[cfg(not(windows))]
    vec![]
}

pub fn ubisoft_games() -> Vec<Game> {
    #[cfg(windows)]
    {
        os::ubisoft_games()
    }
    #[cfg(not(windows))]
    vec![]
}

/// Start a GOG game: through GOG Galaxy when it's installed (so it tracks play time and syncs
/// saves), else the game's own exe from its registry entry.
pub fn launch_gog(id: &str) -> knobdeck_sdk::Result<()> {
    #[cfg(windows)]
    {
        os::launch_gog(id)
    }
    #[cfg(not(windows))]
    {
        let _ = id;
        Err(knobdeck_sdk::Error::msg("GOG games start through GOG Galaxy on Windows"))
    }
}

#[cfg(windows)]
mod os {
    use super::Game;
    use knobdeck_sdk::{Error, Result};
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::System::Registry::{RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_SZ};

    const GOG_GAMES: &[&str] = &["SOFTWARE\\WOW6432Node\\GOG.com\\Games", "SOFTWARE\\GOG.com\\Games"];
    const GALAXY_PATHS: &[&str] = &["SOFTWARE\\WOW6432Node\\GOG.com\\GalaxyClient\\paths", "SOFTWARE\\GOG.com\\GalaxyClient\\paths"];
    const UNINSTALL: &str = "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall";

    /// A string value of `HKLM\<key>`.
    fn reg_str(key: &str, value: &str) -> Option<String> {
        let mut buf = vec![0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &HSTRING::from(key), &HSTRING::from(value), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr().cast()), Some(&mut len)) };
        if r.is_err() {
            return None;
        }
        let n = (len as usize / 2).saturating_sub(1).min(buf.len());
        Some(String::from_utf16_lossy(&buf[..n])).filter(|s| !s.is_empty())
    }

    /// The subkey names of `HKLM\<key>`.
    fn subkeys(key: &str) -> Vec<String> {
        let mut h = HKEY::default();
        if unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(key), 0, KEY_READ, &mut h) }.is_err() {
            return vec![];
        }
        let mut v = vec![];
        for i in 0.. {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if unsafe { RegEnumKeyExW(h, i, PWSTR(name.as_mut_ptr()), &mut len, None, PWSTR::null(), None, None) }.is_err() {
                break;
            }
            v.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        let _ = unsafe { RegCloseKey(h) };
        v
    }

    /// `(registry key, game id)` of every GOG game.
    fn gog_entries() -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = vec![];
        for base in GOG_GAMES {
            for id in subkeys(base) {
                if !v.iter().any(|(_, x)| *x == id) {
                    v.push((format!("{base}\\{id}"), id));
                }
            }
        }
        v
    }

    pub fn gog_games() -> Vec<Game> {
        gog_entries()
            .into_iter()
            // DLCs have their own entries without an exe.
            .filter(|(k, _)| reg_str(k, "exe").is_some())
            .map(|(k, id)| Game {
                name: reg_str(&k, "gameName").unwrap_or_else(|| id.clone()),
                key: format!("gog:{id}"),
                store: "gog",
                last_played: 0,
                icon: None,
                dir: reg_str(&k, "path").map(PathBuf::from),
                exe: None,
            })
            .collect()
    }

    pub fn launch_gog(id: &str) -> Result<()> {
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::msg("Choose a game"));
        }
        let Some((key, _)) = gog_entries().into_iter().find(|(_, x)| x == id) else { return Err(Error::msg("This GOG game isn't installed any more")) };
        let dir = reg_str(&key, "path");
        let galaxy = GALAXY_PATHS.iter().find_map(|k| reg_str(k, "client")).map(|c| PathBuf::from(c).join("GalaxyClient.exe")).filter(|p| p.is_file());
        let started = match (galaxy, &dir) {
            (Some(g), Some(dir)) => std::process::Command::new(g).arg("/command=runGame").arg(format!("/gameId={id}")).raw_arg(format!("/path=\"{dir}\"")).spawn(),
            _ => {
                let exe = reg_str(&key, "exe").ok_or_else(|| Error::msg("This GOG game has no exe in its registry entry"))?;
                let mut c = std::process::Command::new(&exe);
                if let Some(p) = reg_str(&key, "launchParam") {
                    c.raw_arg(p);
                }
                if let Some(wd) = reg_str(&key, "workingDir").or(dir) {
                    c.current_dir(wd);
                }
                c.spawn()
            }
        };
        started.map(|_| ()).map_err(|e| Error::msg(format!("Could not start the game: {e}")))
    }

    /// Ubisoft Connect's `Uplay Install <id>` uninstall entries.
    pub fn ubisoft_games() -> Vec<Game> {
        subkeys(UNINSTALL)
            .into_iter()
            .filter_map(|k| {
                let id = k.strip_prefix("Uplay Install ")?.trim().to_string();
                if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let key = format!("{UNINSTALL}\\{k}");
                let dir = reg_str(&key, "InstallLocation").map(PathBuf::from);
                let name = reg_str(&key, "DisplayName").or_else(|| dir.as_ref().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| id.clone());
                Some(Game { key: format!("ubisoft:{id}"), name, store: "ubisoft", last_played: 0, icon: None, dir, exe: None })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("knobdeck-games-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn heroic_libraries() {
        let r = fixture("heroic");
        put(&r, "legendaryConfig/legendary/installed.json", r#"{"Sugar": {"app_name": "Sugar", "title": "Rocket League", "install_path": "/games/RocketLeague", "is_dlc": false}, "Dlc1": {"title": "A DLC", "is_dlc": true}}"#);
        put(&r, "gog_store/installed.json", r#"{"installed": [{"appName": "1207658924", "install_path": "/games/Witcher", "platform": "windows"}]}"#);
        put(&r, "store_cache/gog_library.json", r#"{"games": [{"app_name": "1207658924", "title": "The Witcher"}]}"#);
        put(&r, "nile_config/nile/installed.json", r#"[{"id": "amzn1.adg.product.x", "path": "/games/Amz", "version": "1"}]"#);
        put(&r, "nile_config/nile/library.json", r#"[{"product": {"id": "amzn1.adg.product.x", "title": "Amazon Game"}}]"#);
        put(&r, "sideload_apps/library.json", r#"{"games": [{"app_name": "abc", "title": "My Game", "is_installed": true, "install": {"executable": "/opt/mygame/run.sh"}}, {"app_name": "web", "title": "Web", "is_installed": true, "install": {}}]}"#);
        let g = heroic_games_in(&r);
        let keys: Vec<(&str, &str)> = g.iter().map(|g| (g.key.as_str(), g.name.as_str())).collect();
        assert_eq!(
            keys,
            [("heroic:legendary:Sugar", "Rocket League"), ("heroic:gog:1207658924", "The Witcher"), ("heroic:nile:amzn1.adg.product.x", "Amazon Game"), ("heroic:sideload:abc", "My Game"), ("heroic:sideload:web", "Web")]
        );
        assert_eq!(g[0].dir.as_deref(), Some(Path::new("/games/RocketLeague")));
        assert_eq!((g[3].dir.as_deref(), g[3].exe.as_deref()), (None, Some(Path::new("/opt/mygame/run.sh"))));
        assert_eq!(crate::library::launch_url(&g[1].key).as_deref(), Some("heroic://launch?appName=1207658924&runner=gog"));
        assert!(heroic_games_in(&r.join("missing")).is_empty());
        let _ = std::fs::remove_dir_all(r);
    }

    /// A pga.db made with Lutris's own schema (columns from `lutris/database/schema.py`).
    #[cfg(target_os = "linux")]
    #[test]
    fn lutris_db() {
        let r = fixture("lutris");
        let db = r.join("lutris/pga.db");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(
            "CREATE TABLE games (id INTEGER PRIMARY KEY, name TEXT, sortname TEXT, slug TEXT, installer_slug TEXT, parent_slug TEXT, platform TEXT, runner TEXT, executable TEXT, directory TEXT, updated DATETIME, lastplayed INTEGER, installed INTEGER, installed_at INTEGER, year INTEGER, configpath TEXT, has_custom_banner INTEGER, has_custom_icon INTEGER, has_custom_coverart_big INTEGER, playtime REAL, service TEXT, service_id TEXT, discord_id TEXT);
             INSERT INTO games (id, name, slug, runner, directory, lastplayed, installed) VALUES (3, 'Celeste', 'celeste', 'linux', '/home/u/Games/celeste', 1790000000, 1);
             INSERT INTO games (id, name, slug, runner, directory, lastplayed, installed) VALUES (4, 'Not installed', 'ni', 'wine', '', NULL, 0);
             INSERT INTO games (id, name, slug, runner, directory, lastplayed, installed) VALUES (5, 'Dota 2', 'dota-2', 'steam', '', NULL, 1);
             INSERT INTO games (id, name, slug, runner, directory, lastplayed, installed) VALUES (6, 'Overwatch', 'overwatch', 'wine', '/home/u/Games/battlenet', NULL, 1);",
        )
        .unwrap();
        drop(c);
        put(&r, "icons/hicolor/128x128/apps/lutris_celeste.png", "png");
        let g = lutris_games_in(&db);
        assert_eq!(g.iter().map(|g| (g.key.as_str(), g.name.as_str(), g.last_played)).collect::<Vec<_>>(), [("lutris:3", "Celeste", 1790000000), ("lutris:6", "Overwatch", 0)]);
        assert_eq!(g[0].dir.as_deref(), Some(Path::new("/home/u/Games/celeste")));
        assert!(g[0].icon.as_ref().is_some_and(|p| p.ends_with("lutris_celeste.png")) && g[1].icon.is_none());
        assert_eq!(crate::library::launch_url(&g[0].key).as_deref(), Some("lutris:rungameid/3"));
        let _ = std::fs::remove_dir_all(r);
    }
}
