//! Menu Command: press any item of an app's menu bar through Accessibility, the
//! way a click on it would (Rule 1, route 3: "pressing its menu item through
//! Accessibility"). No keystroke, so it works whatever the item's shortcut is,
//! or when it has none.
//!
//! macOS only: every app's menu bar is exposed to Accessibility (it's how the
//! system draws it at the top of the screen). On Windows and Linux each app owns
//! the menus inside its own window, so there is no equivalent.
//!
//! A command is stored as its path of titles, "File ▸ Export ▸ PNG…", and an app
//! as its bundle id (or "front" for whichever app is in front when it runs).

use crate::ax::{self, Elem};
use crate::windows;
use knobdeck_sdk::{Error, Result};
use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication, NSWorkspace};
use objc2_foundation::NSString;

/// Joins the titles of a menu path.
pub const SEP: &str = " ▸ ";
/// The app input's value for "whichever app is in front".
pub const FRONT: &str = "front";

/// Walk no deeper than this (top menu = 1), and list at most this many items.
const MAX_DEPTH: usize = 5;
const MAX_ITEMS: usize = 4000;

pub struct RunningApp {
    pub bundle_id: String,
    pub name: String,
    pub pid: i32,
}

fn regular_app(app: &NSRunningApplication) -> Option<RunningApp> {
    if app.activationPolicy() != NSApplicationActivationPolicy::Regular {
        return None;
    }
    let bundle_id = app.bundleIdentifier()?.to_string();
    let pid = app.processIdentifier();
    if windows::is_knobdeck_pid(pid) {
        return None;
    }
    let name = app.localizedName().map(|n| n.to_string()).unwrap_or_else(|| bundle_id.clone());
    Some(RunningApp { bundle_id, name, pid })
}

/// Running apps with a menu bar (regular apps), by name, KnobDeck left out. Window owners
/// are read fresh each time; NSWorkspace's list only refreshes when a run loop spins,
/// which a plugin process has none of, so it only adds apps that have no window.
pub fn running_apps() -> Vec<RunningApp> {
    let mut pids = windows::window_app_pids();
    for app in NSWorkspace::sharedWorkspace().runningApplications().iter() {
        let pid = app.processIdentifier();
        if !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    let mut apps: Vec<RunningApp> = pids
        .into_iter()
        .filter_map(NSRunningApplication::runningApplicationWithProcessIdentifier)
        .filter_map(|a| regular_app(&a))
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps.dedup_by(|a, b| a.bundle_id == b.bundle_id);
    apps
}

/// The app element a command runs in: the front app for `FRONT` (or empty), else the
/// running app with that bundle id, brought to the front first so the command acts on
/// its window (menu items usually act on the key window).
pub fn target_app(choice: &str, activate: bool) -> Result<Elem> {
    if choice.is_empty() || choice == FRONT {
        return ax::focused_app();
    }
    ax::ensure_trusted()?;
    let running = NSRunningApplication::runningApplicationsWithBundleIdentifier(&NSString::from_str(choice));
    let app = running.iter().find_map(|a| regular_app(&a).map(|r| (r, a.isActive())));
    let Some((app, active)) = app else {
        return Err(Error::msg(format!("{choice} isn't running")));
    };
    let elem = Elem::app(app.pid);
    if activate && !active {
        if elem.bool("AXHidden") == Some(true) {
            let _ = elem.set_bool("AXHidden", false);
        }
        elem.set_bool("AXFrontmost", true)?;
        // Let the app become key so its menus act on its window.
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(elem)
}

/// One pressable menu item (a leaf: items that open a submenu are walked, not listed).
pub struct MenuEntry {
    pub path: Vec<String>,
    /// "⇧⌘S", when the item has a shortcut.
    pub shortcut: Option<String>,
}

impl MenuEntry {
    pub fn path_text(&self) -> String {
        self.path.join(SEP)
    }
}

/// The app's menu bar items: (top-level menu titles, the bar items).
fn bar_items(app: &Elem) -> Result<Vec<Elem>> {
    let bar = app.elem_attr("AXMenuBar").ok_or_else(|| Error::msg("This app has no menu bar Accessibility can read"))?;
    // The first bar item is the Apple menu: system commands, not the app's.
    Ok(bar.children("AXChildren").into_iter().skip(1).collect())
}

/// An item's submenu (the AXMenu child), when it opens one.
fn submenu(item: &Elem) -> Option<Elem> {
    item.children("AXChildren").into_iter().find(|c| c.string("AXRole").as_deref() == Some("AXMenu"))
}

/// Every leaf item of the app's menus, in menu order.
pub fn list(app: &Elem) -> Result<Vec<MenuEntry>> {
    let mut out = vec![];
    for top in bar_items(app)? {
        let title = top.title();
        if title.is_empty() {
            continue;
        }
        if let Some(menu) = submenu(&top) {
            walk(&menu, &mut vec![title], &mut out);
        }
        if out.len() >= MAX_ITEMS {
            break;
        }
    }
    Ok(out)
}

fn walk(menu: &Elem, path: &mut Vec<String>, out: &mut Vec<MenuEntry>) {
    for item in menu.children("AXChildren") {
        if out.len() >= MAX_ITEMS {
            return;
        }
        let title = item.title();
        if title.trim().is_empty() {
            continue; // separators
        }
        path.push(title);
        match submenu(&item) {
            Some(sub) if path.len() < MAX_DEPTH => walk(&sub, path, out),
            Some(_) => {}
            None => out.push(MenuEntry {
                path: path.clone(),
                shortcut: shortcut_text(item.cmd_char(), item.cmd_modifiers()),
            }),
        }
        path.pop();
    }
}

/// "⇧⌘S" from AXMenuItemCmdChar and AXMenuItemCmdModifiers (bits: 1 ⇧, 2 ⌥, 4 ⌃, 8 no ⌘).
pub fn shortcut_text(ch: Option<String>, modifiers: Option<i64>) -> Option<String> {
    let ch = ch.filter(|c| !c.trim().is_empty())?;
    let m = modifiers.unwrap_or(0);
    let mut s = String::new();
    if m & 4 != 0 {
        s.push('⌃');
    }
    if m & 2 != 0 {
        s.push('⌥');
    }
    if m & 1 != 0 {
        s.push('⇧');
    }
    if m & 8 == 0 {
        s.push('⌘');
    }
    s.push_str(&ch.to_uppercase());
    Some(s)
}

/// "File ▸ Export ▸ PNG…" → ["File", "Export", "PNG…"]. Also accepts ">" between titles
/// when the path was typed by hand.
pub fn parse_path(text: &str) -> Vec<String> {
    let sep = if text.contains('▸') { '▸' } else { '>' };
    text.split(sep).map(|t| t.trim().to_owned()).filter(|t| !t.is_empty()).collect()
}

/// Titles match exactly, else ignoring case and a trailing ellipsis ("Save As…" = "save as...").
pub fn titles_match(actual: &str, wanted: &str) -> bool {
    fn norm(s: &str) -> String {
        s.trim().trim_end_matches('…').trim_end_matches("...").trim().to_lowercase()
    }
    actual == wanted || norm(actual) == norm(wanted)
}

/// Press the item at `path` in the app's menu bar.
pub fn press(app: &Elem, path: &[String]) -> Result {
    let (last, menus) = path.split_last().ok_or_else(|| Error::msg("Pick a menu command first"))?;
    let mut items = bar_items(app)?;
    let mut trail = vec![];
    for title in menus {
        let item = items
            .into_iter()
            .find(|i| titles_match(&i.title(), title))
            .ok_or_else(|| Error::msg(format!("No “{title}” menu{}", in_trail(&trail))))?;
        let menu = submenu(&item).ok_or_else(|| Error::msg(format!("“{title}” has no submenu")))?;
        items = menu.children("AXChildren");
        trail.push(title.clone());
    }
    let item = items
        .into_iter()
        .find(|i| titles_match(&i.title(), last))
        .ok_or_else(|| Error::msg(format!("No “{last}” item{}", in_trail(&trail))))?;
    if item.bool("AXEnabled") == Some(false) {
        return Err(Error::msg(format!("“{last}” is greyed out in the app right now")));
    }
    item.perform("AXPress")
}

fn in_trail(trail: &[String]) -> String {
    if trail.is_empty() {
        " in this app's menu bar".into()
    } else {
        format!(" in {}", trail.join(SEP))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_like_the_menu() {
        assert_eq!(shortcut_text(Some("s".into()), Some(0)).as_deref(), Some("⌘S"));
        assert_eq!(shortcut_text(Some("S".into()), Some(1)).as_deref(), Some("⇧⌘S"));
        assert_eq!(shortcut_text(Some("F".into()), Some(4 | 8)).as_deref(), Some("⌃F"));
        assert_eq!(shortcut_text(None, Some(0)), None);
        assert_eq!(shortcut_text(Some(" ".into()), None), None);
    }

    #[test]
    fn paths_parse_from_the_picker_or_by_hand() {
        assert_eq!(parse_path("File ▸ Export ▸ PNG…"), vec!["File", "Export", "PNG…"]);
        assert_eq!(parse_path("Edit > Find > Find…"), vec!["Edit", "Find", "Find…"]);
        assert!(parse_path("  ").is_empty());
    }

    #[test]
    fn titles_match_loosely() {
        assert!(titles_match("Save As…", "save as..."));
        assert!(titles_match("Export", "Export"));
        assert!(!titles_match("Export", "Import"));
    }

    /// Lists the front app's menus: `cargo test -p mac-tools -- --ignored menus --nocapture`.
    #[test]
    #[ignore]
    fn live_menus_of_the_front_app() {
        let app = ax::focused_app().expect("front app");
        let items = list(&app).expect("menus");
        for e in items.iter().take(60) {
            eprintln!("{}  {}", e.path_text(), e.shortcut.as_deref().unwrap_or(""));
        }
        assert!(!items.is_empty());
        for a in running_apps() {
            eprintln!("{} {} {}", a.name, a.bundle_id, a.pid);
        }
    }
}
