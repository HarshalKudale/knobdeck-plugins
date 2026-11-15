//! The browsers the extension reports, and where their programs are. Only the few actions the
//! extension APIs can't do (focus the address bar, find, save, Firefox's internal pages) still
//! send a shortcut, and they need the browser's program to bring it to the front first.

use knobdeck_sdk::{Error, Result};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Chromium,
    Firefox,
}

/// Browsers the plugin can bring to the front by key (what the extension's `browser` says).
pub const KEYS: [&str; 5] = ["chrome", "msedge", "brave", "firefox", "zen"];

pub fn family_of(key: &str) -> Family {
    match key {
        "firefox" | "zen" => Family::Firefox,
        _ => Family::Chromium,
    }
}

pub fn label(key: &str) -> &'static str {
    match key {
        "msedge" => "Microsoft Edge",
        "brave" => "Brave",
        "firefox" => "Firefox",
        "zen" => "Zen",
        _ => "Chrome",
    }
}

#[cfg(not(target_os = "windows"))]
fn env_path(key: &str, default: &str) -> PathBuf {
    PathBuf::from(std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.into()))
}

#[cfg(not(windows))]
fn home() -> PathBuf {
    env_path("HOME", "/")
}

/// `[%ProgramFiles%, %ProgramFiles(x86)%, %LOCALAPPDATA%] / parts`
#[cfg(windows)]
fn install_dirs(parts: &[&str]) -> Vec<PathBuf> {
    let env = |key: &str, default: &str| PathBuf::from(std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| default.into()));
    let join = |base: PathBuf| parts.iter().fold(base, |p, s| p.join(s));
    vec![join(env("ProgramFiles", "C:\\Program Files")), join(env("ProgramFiles(x86)", "C:\\Program Files (x86)")), join(std::env::var("LOCALAPPDATA").unwrap_or_default().into())]
}

/// `.app` bundles in `/Applications` and `~/Applications`, for each bundle name.
#[cfg(target_os = "macos")]
fn app_bundles(names: &[&str]) -> Vec<PathBuf> {
    names.iter().flat_map(|n| [PathBuf::from("/Applications").join(n), home().join("Applications").join(n)]).collect()
}

/// Linux executables: distro packages on PATH (Debian/Ubuntu `.deb`s and Arch packages put a
/// launcher in /usr/bin), the vendors' /opt installs, and Flatpak / Snap exports.
#[cfg(target_os = "linux")]
fn linux_exes(bins: &[&str], extra: &[PathBuf], flatpak_id: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = bins.iter().flat_map(|b| [PathBuf::from("/usr/bin").join(b), PathBuf::from("/usr/local/bin").join(b), PathBuf::from("/snap/bin").join(b)]).collect();
    v.extend(extra.iter().cloned());
    v.push(home().join(".local/share/flatpak/exports/bin").join(flatpak_id));
    v.push(PathBuf::from("/var/lib/flatpak/exports/bin").join(flatpak_id));
    v
}

fn exe_candidates(key: &str) -> Vec<PathBuf> {
    #[cfg(windows)]
    return match key {
        "msedge" => install_dirs(&["Microsoft", "Edge", "Application", "msedge.exe"]),
        "brave" => install_dirs(&["BraveSoftware", "Brave-Browser", "Application", "brave.exe"]),
        "firefox" => install_dirs(&["Mozilla Firefox", "firefox.exe"]),
        "zen" => {
            let mut exe = install_dirs(&["Zen Browser", "zen.exe"]);
            exe.push(PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Programs").join("Zen Browser").join("zen.exe"));
            exe
        }
        _ => install_dirs(&["Google", "Chrome", "Application", "chrome.exe"]),
    };
    #[cfg(target_os = "linux")]
    return {
        let h = home();
        match key {
            "msedge" => linux_exes(&["microsoft-edge-stable", "microsoft-edge"], &["/opt/microsoft/msedge/msedge".into()], "com.microsoft.Edge"),
            "brave" => linux_exes(&["brave-browser", "brave-browser-stable", "brave"], &["/opt/brave.com/brave/brave".into(), "/opt/brave-bin/brave".into()], "com.brave.Browser"),
            "firefox" => linux_exes(&["firefox"], &["/usr/lib/firefox/firefox".into(), "/opt/firefox/firefox".into()], "org.mozilla.firefox"),
            "zen" => linux_exes(&["zen-browser", "zen"], &["/opt/zen-browser-bin/zen-bin".into(), "/opt/zen/zen".into(), h.join(".tarball-installations/zen/zen")], "app.zen_browser.zen"),
            _ => linux_exes(&["google-chrome-stable", "google-chrome"], &["/opt/google/chrome/chrome".into()], "com.google.Chrome"),
        }
    };
    #[cfg(target_os = "macos")]
    return match key {
        "msedge" => app_bundles(&["Microsoft Edge.app"]),
        "brave" => app_bundles(&["Brave Browser.app"]),
        "firefox" => app_bundles(&["Firefox.app"]),
        "zen" => app_bundles(&["Zen.app", "Zen Browser.app"]),
        _ => app_bundles(&["Google Chrome.app"]),
    };
}

/// Window classes / Wayland app ids each browser's windows carry on Linux.
#[cfg(target_os = "linux")]
pub fn window_classes(key: &str) -> &'static [&'static str] {
    match key {
        "msedge" => &["microsoft-edge", "com.microsoft.Edge"],
        "brave" => &["brave-browser", "com.brave.Browser"],
        "firefox" => &["firefox", "org.mozilla.firefox"],
        "zen" => &["zen", "app.zen_browser.zen"],
        _ => &["google-chrome", "com.google.Chrome"],
    }
}

/// The installed program (a `.exe`, a `.app` bundle, a Linux launcher) of a browser key.
pub fn ensure_executable(key: &str) -> Result<String> {
    if !KEYS.contains(&key) {
        return Err(Error::msg(format!("{key} can't be brought to the front for this action")));
    }
    exe_candidates(key).into_iter().find(|c| c.exists()).map(|c| c.to_string_lossy().into_owned()).ok_or_else(|| Error::msg(format!("{} is not installed", label(key))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families() {
        assert!(family_of("firefox") == Family::Firefox);
        assert!(family_of("zen") == Family::Firefox);
        assert!(family_of("brave") == Family::Chromium);
        assert!(family_of("something-else") == Family::Chromium);
    }

    #[test]
    fn unknown_browsers_are_not_guessed() {
        assert!(ensure_executable("opera").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_locations() {
        assert!(exe_candidates("chrome").contains(&PathBuf::from("/usr/bin/google-chrome-stable")));
        assert!(exe_candidates("zen").contains(&PathBuf::from("/usr/bin/zen-browser")));
    }
}
