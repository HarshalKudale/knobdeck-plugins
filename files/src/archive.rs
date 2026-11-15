//! Zip and unzip (`zip`, deflate).

use crate::paths::{self, free_path};
use knobdeck_sdk::{Error, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

fn err(e: impl std::fmt::Display) -> Error {
    Error::msg(e.to_string())
}

/// Zip the items into one archive next to the first: `name.zip` for one item, `Archive.zip`
/// for several. Folders are added with their contents.
pub fn zip(items: &[PathBuf]) -> Result<PathBuf> {
    let first = items.first().ok_or_else(|| Error::msg("Select files first"))?;
    let dir = paths::parent(first)?;
    let stem = if items.len() == 1 { paths::name(first) } else { "Archive".into() };
    let stem = if items.len() == 1 && first.is_file() { paths::stem(first) } else { stem };
    let target = free_path(dir, &stem, "zip");
    let file = std::fs::File::create(&target).map_err(paths::io)?;
    let mut w = zip::ZipWriter::new(file);
    let r = (|| {
        for item in items {
            add(&mut w, item, &paths::name(item))?;
        }
        w.finish().map_err(err)?;
        Ok(())
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&target);
    }
    r.map(|_| target)
}

fn options(p: &Path) -> SimpleFileOptions {
    let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    #[cfg(unix)]
    let o = {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(p) {
            Ok(m) => o.unix_permissions(m.permissions().mode()),
            Err(_) => o,
        }
    };
    #[cfg(not(unix))]
    let _ = p;
    o
}

fn add(w: &mut zip::ZipWriter<std::fs::File>, p: &Path, name: &str) -> Result<()> {
    let meta = std::fs::symlink_metadata(p).map_err(paths::io)?;
    if meta.is_dir() {
        w.add_directory(format!("{name}/"), options(p)).map_err(err)?;
        let mut entries: Vec<_> = std::fs::read_dir(p).map_err(paths::io)?.filter_map(|e| e.ok()).collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            add(w, &e.path(), &format!("{name}/{}", e.file_name().to_string_lossy()))?;
        }
    } else if meta.is_file() {
        w.start_file(name, options(p)).map_err(err)?;
        let mut f = std::fs::File::open(p).map_err(paths::io)?;
        std::io::copy(&mut f, w).map_err(paths::io)?;
        w.flush().map_err(paths::io)?;
    }
    // Symlinks and special files are skipped.
    Ok(())
}

/// Extract into a new folder named after the archive. Entries that would land outside it are
/// refused by the zip crate.
pub fn unzip(p: &Path) -> Result<PathBuf> {
    let file = std::fs::File::open(p).map_err(paths::io)?;
    let mut a = zip::ZipArchive::new(file).map_err(|e| Error::msg(format!("{} isn't a zip file ({e})", paths::name(p))))?;
    let target = free_path(paths::parent(p)?, &paths::stem(p), "");
    std::fs::create_dir(&target).map_err(paths::io)?;
    if let Err(e) = a.extract(&target) {
        let _ = std::fs::remove_dir_all(&target);
        return Err(err(e));
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_and_unzip_a_folder() {
        let d = std::env::temp_dir().join(format!("kd-files-zip-{}", std::process::id()));
        let src = d.join("docs");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), "world").unwrap();
        let z = zip(&[src.clone()]).unwrap();
        assert_eq!(z, d.join("docs.zip"));
        let out = unzip(&z).unwrap();
        assert_eq!(out, d.join("docs (2)"));
        assert_eq!(std::fs::read_to_string(out.join("docs/sub/b.txt")).unwrap(), "world");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
