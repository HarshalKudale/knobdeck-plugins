//! Installing `KnobDeck.lrplugin` into Lightroom Classic's Modules folder, where Lightroom loads
//! plug-ins by itself at start-up (no Plug-in Manager ▸ Add needed):
//! Windows `%APPDATA%\Adobe\Lightroom\Modules`, macOS `~/Library/Application Support/Adobe/Lightroom/Modules`.
//! The plug-in comes from the latest release of `Knobdeck/knobdeck-lightroom-companion`.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const RELEASE_ZIP: &str = "https://github.com/Knobdeck/knobdeck-lightroom-companion/releases/latest/download/KnobDeck.lrplugin.zip";
pub const FOLDER: &str = "KnobDeck.lrplugin";
const TOOLKIT_ID: &str = "com.knobdeck.lightroom";

pub fn modules_dir() -> Result<PathBuf, String> {
    if cfg!(windows) {
        let appdata = std::env::var_os("APPDATA").ok_or("APPDATA is not set")?;
        Ok(PathBuf::from(appdata).join("Adobe").join("Lightroom").join("Modules"))
    } else if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        Ok(PathBuf::from(home).join("Library/Application Support/Adobe/Lightroom/Modules"))
    } else {
        Err("Lightroom Classic runs on Windows and macOS only".into())
    }
}

/// The release zip's bytes: from `KNOBDECK_LIGHTROOM_PLUGIN_ZIP` (a file path or URL) when set,
/// else the latest release.
pub fn download() -> Result<Vec<u8>, String> {
    let src = std::env::var("KNOBDECK_LIGHTROOM_PLUGIN_ZIP").unwrap_or_else(|_| RELEASE_ZIP.into());
    if !src.starts_with("http://") && !src.starts_with("https://") {
        return std::fs::read(&src).map_err(|e| format!("{src}: {e}"));
    }
    let resp = ureq::get(&src).timeout(std::time::Duration::from_secs(60)).call().map_err(|e| format!("Couldn't download the KnobDeck plug-in for Lightroom ({e})"))?;
    let mut bytes = vec![];
    resp.into_reader().take(20 << 20).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Unpack `zip` (holding `KnobDeck.lrplugin/…`) into `modules`, replacing an older KnobDeck
/// plug-in there but never another plug-in that happens to use the folder name.
pub fn install(zip: &[u8], modules: &Path) -> Result<PathBuf, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| format!("The plug-in download isn't a zip ({e})"))?;
    std::fs::create_dir_all(modules).map_err(|e| format!("{}: {e}", modules.display()))?;
    let dest = modules.join(FOLDER);
    if dest.exists() {
        let info = std::fs::read_to_string(dest.join("Info.lua")).unwrap_or_default();
        if !info.contains(TOOLKIT_ID) {
            return Err(format!("{} holds another plug-in; move it away and run setup again", dest.display()));
        }
    }
    let staging = modules.join(format!("{FOLDER}.new"));
    let _ = std::fs::remove_dir_all(&staging);
    let mut files = 0;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = PathBuf::from(f.name());
        let mut parts = name.components();
        if parts.next() != Some(Component::Normal(FOLDER.as_ref())) {
            continue;
        }
        let rel: PathBuf = parts.collect();
        if rel.as_os_str().is_empty() || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            continue;
        }
        let out = staging.join(&rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut data = vec![];
        f.read_to_end(&mut data).map_err(|e| e.to_string())?;
        std::fs::write(&out, data).map_err(|e| format!("{}: {e}", out.display()))?;
        files += 1;
    }
    let info = std::fs::read_to_string(staging.join("Info.lua")).unwrap_or_default();
    if files == 0 || !info.contains(TOOLKIT_ID) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err("The download has no KnobDeck.lrplugin in it".into());
    }
    let old = modules.join(format!("{FOLDER}.old"));
    let _ = std::fs::remove_dir_all(&old);
    if dest.exists() {
        std::fs::rename(&dest, &old).map_err(|e| format!("Couldn't replace {} ({e}); quit Lightroom and try again", dest.display()))?;
    }
    std::fs::rename(&staging, &dest).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&old);
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(vec![]));
        for (name, body) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn installs_replaces_and_refuses_strangers() {
        let dir = std::env::temp_dir().join(format!("kd-lr-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let good = zip(&[
            ("KnobDeck.lrplugin/Info.lua", "LrToolkitIdentifier = 'com.knobdeck.lightroom'"),
            ("KnobDeck.lrplugin/Bridge.lua", "v1"),
            ("KnobDeck.lrplugin/../../evil.lua", "x"),
            ("README.md", "ignored"),
        ]);
        let dest = install(&good, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("Bridge.lua")).unwrap(), "v1");
        assert!(!dir.join("evil.lua").exists() && !dir.parent().unwrap().join("evil.lua").exists());
        assert!(!dir.join("README.md").exists());
        let newer = zip(&[("KnobDeck.lrplugin/Info.lua", "'com.knobdeck.lightroom'"), ("KnobDeck.lrplugin/Bridge.lua", "v2")]);
        install(&newer, &dir).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("Bridge.lua")).unwrap(), "v2");
        assert!(!dir.join("KnobDeck.lrplugin.old").exists());
        assert!(install(&zip(&[("Other/Info.lua", "x")]), &dir).unwrap_err().contains("no KnobDeck.lrplugin"));
        std::fs::write(dest.join("Info.lua"), "someone else's").unwrap();
        assert!(install(&newer, &dir).unwrap_err().contains("another plug-in"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
