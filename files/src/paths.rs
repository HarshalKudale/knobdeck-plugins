//! Output names and file-system helpers. Nothing here overwrites an existing file.

use knobdeck_sdk::{Error, Result};
use std::path::{Path, PathBuf};

/// `dir/stem.ext`, or `dir/stem (2).ext`, `(3)`… when taken. `ext` may be empty (a folder).
pub fn free_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let name = |n: u32| {
        let base = if n == 1 { stem.to_string() } else { format!("{stem} ({n})") };
        if ext.is_empty() {
            base
        } else {
            format!("{base}.{ext}")
        }
    };
    let mut n = 1;
    loop {
        let p = dir.join(name(n));
        if !p.exists() && p.symlink_metadata().is_err() {
            return p;
        }
        n += 1;
    }
}

pub fn stem(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into())
}

pub fn ext(p: &Path) -> String {
    p.extension().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

pub fn name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn parent(p: &Path) -> Result<&Path> {
    p.parent().filter(|d| !d.as_os_str().is_empty()).ok_or_else(|| Error::msg(format!("{} has no folder", p.display())))
}

/// A path the user typed: `~` is the home folder.
pub fn expand_home(s: &str) -> PathBuf {
    let s = s.trim();
    let home = || std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    if s == "~" {
        return home().unwrap_or_else(|| s.into());
    }
    match s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        Some(rest) => home().map(|h| h.join(rest)).unwrap_or_else(|| s.into()),
        None => PathBuf::from(s),
    }
}

/// Copy a file or a folder (recursively) to `to`, which must not exist.
pub fn copy_any(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_any(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Move a file or folder; across drives / file systems, copy then delete.
pub fn move_any(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) if !to.exists() => {
            copy_any(from, to)?;
            if from.is_dir() {
                std::fs::remove_dir_all(from)
            } else {
                std::fs::remove_file(from)
            }
        }
        Err(e) => Err(e),
    }
}

/// Run `f` on every path; one failure doesn't stop the rest. The error lists what failed.
pub fn each<T>(paths: &[PathBuf], mut f: impl FnMut(&Path) -> Result<T>) -> Result<Vec<T>> {
    let mut ok = vec![];
    let mut failed = vec![];
    for p in paths {
        match f(p) {
            Ok(v) => ok.push(v),
            Err(e) => failed.push(format!("{}: {}", name(p), e.message())),
        }
    }
    match failed.len() {
        0 => Ok(ok),
        1 if paths.len() == 1 => Err(Error::msg(failed.remove(0))),
        n => Err(Error::msg(format!("{n} of {} failed. {}", paths.len(), failed.join("; ")))),
    }
}

pub fn io(e: std::io::Error) -> Error {
    Error::msg(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_names_count_up() {
        let d = std::env::temp_dir().join(format!("kd-files-free-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(free_path(&d, "a", "png"), d.join("a.png"));
        std::fs::write(d.join("a.png"), b"").unwrap();
        assert_eq!(free_path(&d, "a", "png"), d.join("a (2).png"));
        assert_eq!(free_path(&d, "dir", ""), d.join("dir"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn home_expands() {
        assert!(!expand_home("~/x").starts_with("~"));
        assert_eq!(expand_home("/tmp/x"), PathBuf::from("/tmp/x"));
    }
}
