//! Game Launcher plugin for KnobDeck — launch installed Steam and Epic Games Store games from a
//! key or a wheel, and open the launchers' pages.

mod library;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, open_url, Action, Error, Plugin, Result, Value, Visual};
use library::Game;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Reading every manifest is quick, but keys and wheels share one scan for a little while.
static GAMES: Mutex<Option<(Instant, Vec<Game>)>> = Mutex::new(None);

fn games() -> Vec<Game> {
    if let Some((at, g)) = GAMES.lock().unwrap().as_ref() {
        if at.elapsed() < Duration::from_secs(20) {
            return g.clone();
        }
    }
    let g = library::all_games();
    *GAMES.lock().unwrap() = Some((Instant::now(), g.clone()));
    g
}

fn launch(key: &str) -> Result<()> {
    let url = library::launch_url(key).ok_or_else(|| Error::msg("Choose a game"))?;
    open_url(&url).map_err(|e| Error::msg(format!("Could not start the launcher: {}", e.message())))
}

fn store_label(store: &str) -> &'static str {
    match store {
        "epic" => "Epic",
        _ => "Steam",
    }
}

/// Games filtered by store, sorted by most recently played (then name) or by name.
fn listed(store: &str, sort: &str) -> Vec<Game> {
    let mut v: Vec<Game> = games().into_iter().filter(|g| store.is_empty() || store == "all" || g.store == store).collect();
    v.sort_by_key(|g| g.name.to_lowercase());
    if sort != "name" {
        v.sort_by(|a, b| b.last_played.cmp(&a.last_played));
    }
    v
}

/// Launcher pages: (value, label, URL, OSes).
const PAGES: &[(&str, &str, &str, &[&str])] = &[
    ("steam-library", "Steam ▸ Library", "steam://open/games", &["windows", "macos", "linux"]),
    ("steam-store", "Steam ▸ Store", "steam://store", &["windows", "macos", "linux"]),
    ("steam-friends", "Steam ▸ Friends", "steam://open/friends", &["windows", "macos", "linux"]),
    ("steam-downloads", "Steam ▸ Downloads", "steam://open/downloads", &["windows", "macos", "linux"]),
    ("steam-settings", "Steam ▸ Settings", "steam://open/settings", &["windows", "macos", "linux"]),
    ("steam-bigpicture", "Steam ▸ Big Picture", "steam://open/bigpicture", &["windows", "macos", "linux"]),
    ("epic-store", "Epic Games ▸ Store", "com.epicgames.launcher://store", &["windows", "macos"]),
];

fn pages() -> Vec<(&'static str, &'static str)> {
    PAGES.iter().filter(|p| p.3.contains(&std::env::consts::OS)).map(|p| (p.0, p.1)).collect()
}

fn stores() -> Vec<(&'static str, &'static str)> {
    if cfg!(target_os = "linux") {
        vec![("all", "All"), ("steam", "Steam")]
    } else {
        vec![("all", "All"), ("steam", "Steam"), ("epic", "Epic Games")]
    }
}

fn main() {
    let store_opts = stores();
    let page_opts = pages();
    Plugin::new(&format!("game-launcher-{}", std::env::consts::OS), "Game Launcher")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description(if cfg!(target_os = "linux") { "Launch your installed Steam games from a key or a wheel." } else { "Launch your installed Steam and Epic Games Store games from a key or a wheel." })
        .icon("gamepad-2")
        .query("games", |_| {
            let mut v = games();
            v.sort_by_key(|g| g.name.to_lowercase());
            if v.is_empty() {
                return Err(Error::msg("No installed games found"));
            }
            Ok(Value::Array(v.into_iter().map(|g| json!({"value": g.key, "label": format!("{} · {}", g.name, store_label(g.store))})).collect()))
        })
        .action(
            Action::button("launch", "Launch Game")
                .icon("gamepad-2")
                .inputs(vec![input::query_select("game", "Game", "games", &[]), input::checkbox("show_name", "Show the game's name").default_value(true)])
                // The game's icon and name, refreshed now and then (the library cache can change).
                .live(Duration::from_secs(300), |ic| {
                    let key = ic.input_str("game");
                    let Some(g) = games().into_iter().find(|g| g.key == key) else { return Ok(Visual::new()) };
                    let mut v = Visual::new();
                    if let Some(uri) = g.icon.as_deref().and_then(library::icon_uri) {
                        v = v.icon(uri);
                    }
                    if ic.input_bool("show_name") {
                        v = v.label(g.name);
                    }
                    Ok(v)
                })
                .on_press(|ctx| launch(&ctx.input_str("game"))),
        )
        .action(
            Action::wheel("games-wheel", "Games Wheel")
                .icon("layout-grid")
                .inputs(vec![
                    input::select("store", "Store", &store_opts).with("default", "all"),
                    input::select("sort", "Show", &[("recent", "Recently played"), ("name", "A to Z")]).with("default", "recent"),
                ])
                .on_open(|o| {
                    let list = listed(&o.input_str("store"), &o.input_str("sort"));
                    if list.is_empty() {
                        return o.fail("No installed games found");
                    }
                    Ok(list
                        .into_iter()
                        .take(16)
                        .map(|g| {
                            let icon = g.icon.as_deref().and_then(library::icon_uri).unwrap_or_else(|| "gamepad-2".into());
                            json!({"id": g.key, "label": g.name, "icon": icon})
                        })
                        .collect())
                })
                .on_child_press(|ctx| match ctx.child_id.as_deref() {
                    Some(key) => launch(key),
                    None => Ok(()),
                }),
        )
        .action(Action::button("open-page", "Open Launcher Page").icon("library").inputs(vec![input::select("page", "Page", &page_opts).with("default", "steam-library")]).on_press(|ctx| {
            let want = ctx.input_str("page");
            let page = PAGES.iter().find(|p| p.0 == if want.is_empty() { "steam-library" } else { want.as_str() }).ok_or_else(|| Error::msg("Choose a page"))?;
            open_url(page.2).map_err(|e| Error::msg(format!("Could not open the launcher: {}", e.message())))
        }))
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_urls() {
        assert!(pages().iter().any(|p| p.0 == "steam-library"));
        assert_eq!(pages().iter().any(|p| p.0 == "epic-store"), !cfg!(target_os = "linux"));
    }
}
