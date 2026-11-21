//! Putting the KnobDeck Remote Script into Live's User Library (setup): find the User Library,
//! fetch the newest release of `Knobdeck/knobdeck-ableton-companion` and unpack its `KnobDeck/`
//! folder into `<User Library>/Remote Scripts/KnobDeck`.

use knobdeck_sdk::{Error, Result, Value};
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub const FOLDER: &str = "KnobDeck";
const RELEASES: &str = "https://api.github.com/repos/Knobdeck/knobdeck-ableton-companion/releases/latest";
const MAX_ZIP: u64 = 16 << 20;

/// Live's default User Library and the usual alternatives, most likely first.
pub fn default_libraries(home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let mut out = vec![];
    if cfg!(windows) {
        let profile = env("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| home.to_path_buf());
        out.push(profile.join("Documents").join("Ableton").join("User Library"));
        // Documents moved to OneDrive (Windows' backup offers it at sign-in).
        for var in ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"] {
            if let Some(od) = env(var) {
                out.push(PathBuf::from(od).join("Documents").join("Ableton").join("User Library"));
            }
        }
    } else {
        out.push(home.join("Music").join("Ableton").join("User Library"));
    }
    out
}

/// Folders where Live keeps its preferences, one per installed version (`Live 12.1.5`).
fn preference_dirs(home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let base = if cfg!(windows) {
        env("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming")).join("Ableton")
    } else {
        home.join("Library").join("Preferences").join("Ableton")
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&base)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("Live ")))
        .collect();
    dirs.sort_by_key(|p| std::cmp::Reverse(version_key(p)));
    dirs.into_iter().map(|d| if cfg!(windows) { d.join("Preferences") } else { d }).collect()
}

fn version_key(p: &Path) -> Vec<u32> {
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.trim_start_matches("Live ").split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect()
}

/// User Library folders named in a Live `Library.cfg` (Live writes it when the user moves the
/// User Library in Settings ▸ Library). Read loosely: every path-like `Value="…"` inside the
/// `<UserLibrary>` element, with and without "User Library" appended; callers keep the first
/// that exists.
pub fn libraries_in_cfg(xml: &str) -> Vec<PathBuf> {
    let Some(start) = xml.find("<UserLibrary") else { return vec![] };
    let end = xml[start..].find("</UserLibrary>").map(|e| start + e).unwrap_or(xml.len());
    let block = &xml[start..end];
    let mut out = vec![];
    let mut rest = block;
    while let Some(i) = rest.find("Value=\"") {
        rest = &rest[i + 7..];
        let Some(j) = rest.find('"') else { break };
        let v = unescape(&rest[..j]);
        rest = &rest[j..];
        let p = PathBuf::from(&v);
        if p.is_absolute() {
            if !v.trim_end_matches(['/', '\\']).ends_with("User Library") {
                out.push(p.join("User Library"));
            }
            out.push(p);
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

/// The User Library to install into: `given` when the user entered one, else the one Live's
/// preferences name, else the default location. It must exist (Live creates it on first start).
pub fn find_user_library(given: &str) -> Result<PathBuf> {
    let given = given.trim();
    if !given.is_empty() {
        let p = PathBuf::from(given);
        return if p.is_dir() { Ok(p) } else { Err(Error::msg(format!("There is no folder at {}. Enter the User Library folder Live shows in Settings ▸ Library.", p.display()))) };
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let home = home();
    let mut candidates = vec![];
    for dir in preference_dirs(&home, &env) {
        if let Ok(xml) = std::fs::read_to_string(dir.join("Library.cfg")) {
            candidates.extend(libraries_in_cfg(&xml));
        }
    }
    candidates.extend(default_libraries(&home, &env));
    candidates.iter().find(|p| p.is_dir()).cloned().ok_or_else(|| {
        Error::msg(format!(
            "Couldn't find Live's User Library (looked in {}). Start Live once so it creates it, or enter the folder Live shows in Settings ▸ Library.",
            candidates.first().map(|p| p.display().to_string()).unwrap_or_default()
        ))
    })
}

/// The newest Remote Script zip: from `KNOBDECK_ABLETON_SCRIPT_ZIP` (a local file, for offline
/// installs and tests) or the companion's latest GitHub release.
pub fn fetch_zip() -> Result<Vec<u8>> {
    if let Ok(path) = std::env::var("KNOBDECK_ABLETON_SCRIPT_ZIP") {
        return std::fs::read(&path).map_err(|e| Error::msg(format!("Couldn't read {path}: {e}")));
    }
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
    let release: Value = agent
        .get(RELEASES)
        .set("User-Agent", "knobdeck-ableton-setup")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| Error::msg(format!("Couldn't reach GitHub for the KnobDeck Remote Script: {e}")))?
        .into_json()
        .map_err(|e| Error::msg(format!("GitHub sent an unreadable answer: {e}")))?;
    let url = release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str().is_some_and(|n| n.starts_with("knobdeck-ableton-") && n.ends_with(".zip")))
        .and_then(|a| a["browser_download_url"].as_str())
        .ok_or_else(|| Error::msg("The latest KnobDeck Remote Script release has no zip; download it from github.com/Knobdeck/knobdeck-ableton-companion/releases"))?;
    let mut bytes = vec![];
    agent
        .get(url)
        .set("User-Agent", "knobdeck-ableton-setup")
        .call()
        .map_err(|e| Error::msg(format!("Couldn't download the KnobDeck Remote Script: {e}")))?
        .into_reader()
        .take(MAX_ZIP)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::msg(format!("Couldn't download the KnobDeck Remote Script: {e}")))?;
    Ok(bytes)
}

/// The `VERSION = "x.y.z"` in a Remote Script's version.py.
pub fn version_in(py: &str) -> Option<String> {
    py.lines().find_map(|l| l.trim().strip_prefix("VERSION").map(str::trim)?.strip_prefix('=').map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string()))
}

/// Unpack the zip's `KnobDeck/` folder into `<remote_scripts>/KnobDeck`, replacing an older copy.
/// Returns the installed version.
pub fn install_zip(zip_bytes: &[u8], remote_scripts: &Path) -> Result<String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| Error::msg(format!("The Remote Script download isn't a zip: {e}")))?;
    std::fs::create_dir_all(remote_scripts).map_err(|e| Error::msg(format!("Couldn't create {}: {e}", remote_scripts.display())))?;
    let staging = remote_scripts.join(format!(".{FOLDER}.knobdeck-new"));
    let _ = std::fs::remove_dir_all(&staging);
    let mut version = None;
    let mut files = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| Error::msg(format!("Broken Remote Script zip: {e}")))?;
        let Some(name) = entry.enclosed_name() else { continue };
        let mut parts = name.components();
        if parts.next() != Some(Component::Normal(FOLDER.as_ref())) || entry.is_dir() {
            continue;
        }
        let rel: PathBuf = parts.collect();
        if rel.as_os_str().is_empty() || rel.components().any(|c| c.as_os_str() == "__pycache__") {
            continue;
        }
        let mut data = vec![];
        entry.read_to_end(&mut data).map_err(|e| Error::msg(format!("Broken Remote Script zip: {e}")))?;
        if rel == Path::new("version.py") {
            version = version_in(&String::from_utf8_lossy(&data));
        }
        let to = staging.join(&rel);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::msg(format!("Couldn't write {}: {e}", dir.display())))?;
        }
        std::fs::write(&to, data).map_err(|e| Error::msg(format!("Couldn't write {}: {e}", to.display())))?;
        files += 1;
    }
    if files == 0 || !staging.join("__init__.py").is_file() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(Error::msg(format!("The download has no {FOLDER}/ Remote Script folder")));
    }
    let target = remote_scripts.join(FOLDER);
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staging);
            Error::msg(format!("Couldn't replace {} (quit Live and run setup again): {e}", target.display()))
        })?;
    }
    std::fs::rename(&staging, &target).map_err(|e| Error::msg(format!("Couldn't move the Remote Script into {}: {e}", target.display())))?;
    Ok(version.unwrap_or_else(|| "unknown".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Cursor::new(vec![]);
        let mut z = zip::ZipWriter::new(&mut out);
        for (name, body) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        out.into_inner()
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kd-ableton-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn installs_and_replaces_the_script_folder() {
        let lib = temp("install");
        let rs = lib.join("Remote Scripts");
        let v1 = zip_of(&[("KnobDeck/__init__.py", "x"), ("KnobDeck/version.py", "VERSION = \"1.0.0\"\n"), ("KnobDeck/old.py", "")]);
        assert_eq!(install_zip(&v1, &rs).unwrap(), "1.0.0");
        assert!(rs.join("KnobDeck/old.py").is_file());
        let v2 = zip_of(&[
            ("KnobDeck/__init__.py", "y"),
            ("KnobDeck/version.py", "VERSION = '1.1.0'"),
            ("KnobDeck/__pycache__/x.pyc", ""),
            ("README.md", "not part of the script"),
            ("../escape.py", "outside"),
        ]);
        assert_eq!(install_zip(&v2, &rs).unwrap(), "1.1.0");
        assert_eq!(std::fs::read_to_string(rs.join("KnobDeck/__init__.py")).unwrap(), "y");
        assert!(!rs.join("KnobDeck/old.py").exists(), "an older copy is replaced, not merged");
        assert!(!rs.join("KnobDeck/__pycache__").exists());
        assert!(!rs.join("README.md").exists() && !lib.join("escape.py").exists());
        assert!(!rs.join(".KnobDeck.knobdeck-new").exists());
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn refuses_a_zip_without_the_script() {
        let lib = temp("bad");
        let rs = lib.join("Remote Scripts");
        assert!(install_zip(&zip_of(&[("Other/__init__.py", "")]), &rs).unwrap_err().message().contains("no KnobDeck/"));
        assert!(install_zip(b"not a zip", &rs).is_err());
        assert!(!rs.join("KnobDeck").exists());
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn reads_library_cfg_loosely() {
        let abs = if cfg!(windows) { r"C:\Music\Ableton" } else { "/Volumes/Music/Ableton" };
        let xml = format!(
            r#"<?xml version="1.0"?><Ableton><UserLibrary><LibraryProject><ProjectName Value="User Library" /><ProjectPath Value="{abs}" /><Rel Value="relative/x"/></LibraryProject></UserLibrary><Other><Path Value="{abs}/nope"/></Other></Ableton>"#
        );
        let got = libraries_in_cfg(&xml);
        assert_eq!(got, vec![PathBuf::from(abs).join("User Library"), PathBuf::from(abs)]);
        assert!(libraries_in_cfg("<Ableton/>").is_empty());
    }

    #[test]
    fn default_library_paths() {
        let env = |k: &str| match k {
            "USERPROFILE" => Some(if cfg!(windows) { r"C:\Users\a".to_string() } else { "/Users/a".to_string() }),
            "OneDrive" => Some(r"C:\Users\a\OneDrive".to_string()),
            _ => None,
        };
        let got = default_libraries(Path::new("/Users/a"), &env);
        if cfg!(windows) {
            assert_eq!(got[0], PathBuf::from(r"C:\Users\a\Documents\Ableton\User Library"));
            assert_eq!(got[1], PathBuf::from(r"C:\Users\a\OneDrive\Documents\Ableton\User Library"));
        } else {
            assert_eq!(got, vec![PathBuf::from("/Users/a/Music/Ableton/User Library")]);
        }
        assert_eq!(version_in("# v\nVERSION = \"2.0.1\"\n").as_deref(), Some("2.0.1"));
        assert_eq!(version_key(Path::new("Live 12.1.5")), vec![12, 1, 5]);
    }
}
