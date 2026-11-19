//! Files plugin for KnobDeck: act on the files selected in File Explorer, Finder or Dolphin.
//! Images (convert, resize, rotate, remove metadata), PDFs (merge, split, images to PDF), zip and
//! unzip, copy paths, rename, move or copy to a folder, new file, and a Files wheel
//! that shows only what fits the selection. All work happens in this process, in Rust; outputs go
//! next to the inputs and never overwrite a file.

mod archive;
mod images;
mod paths;
mod pdf;
mod sys;

use knobdeck_sdk::input::{self, InputExt};
use knobdeck_sdk::{json, Action, Ctx, Error, Need, Plugin, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Finder answers over Apple Events and the first read may wait on the Automation prompt.
const READ_BUDGET: Duration = Duration::from_secs(3);

/// Sort key that orders `page2` before `page10`.
fn natural_key(p: &Path) -> Vec<(u8, u64, String)> {
    let name = paths::name(p).to_lowercase();
    let mut key = vec![];
    let mut chars = name.chars().peekable();
    while let Some(&c) = chars.peek() {
        let digit = c.is_ascii_digit();
        let mut run = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() != digit {
                break;
            }
            run.push(c);
            chars.next();
        }
        key.push(if digit { (0, run.parse().unwrap_or(u64::MAX), String::new()) } else { (1, 0, run) });
    }
    key
}

/// The selection, sorted by name, or the reason there is none.
fn selected(ctx: &Ctx) -> Result<Vec<PathBuf>> {
    let mut f = ctx.files().to_vec();
    if f.is_empty() {
        return Err(Error::msg(ctx.context_error(Need::Files).unwrap_or_else(|| "Select files first".into())));
    }
    f.sort_by_cached_key(|p| natural_key(p));
    Ok(f)
}

fn only(files: Vec<PathBuf>, keep: impl Fn(&Path) -> bool, what: &str) -> Result<Vec<PathBuf>> {
    let v: Vec<PathBuf> = files.into_iter().filter(|p| keep(p)).collect();
    if v.is_empty() {
        return Err(Error::msg(format!("Select {what} first")));
    }
    Ok(v)
}

fn is_pdf(p: &Path) -> bool {
    paths::ext(p) == "pdf"
}

fn is_zip(p: &Path) -> bool {
    paths::ext(p) == "zip"
}

/// The folder an action that makes something should use: a single selected folder, else the
/// folder the window shows, else the selection's folder.
fn work_folder(ctx: &Ctx) -> Result<PathBuf> {
    let files = ctx.files();
    if let [one] = files {
        if one.is_dir() {
            return Ok(one.clone());
        }
    }
    if let Some(f) = ctx.folder() {
        return Ok(f.to_path_buf());
    }
    match files.first() {
        Some(f) => Ok(paths::parent(f)?.to_path_buf()),
        None => Err(Error::msg(ctx.context_error(Need::Files).unwrap_or_else(|| "Open a folder in your file manager first".into()))),
    }
}

/// New names for `files` from a pattern: `{name}` (without extension), `{ext}`, `{n}` (a counter
/// from `start`; `{n:3}` pads to 3 digits). The extension is kept unless the pattern has `{ext}`.
fn rename_targets(files: &[PathBuf], pattern: &str, start: u64) -> Result<Vec<PathBuf>> {
    if pattern.trim().is_empty() {
        return Err(Error::msg("Enter a name pattern"));
    }
    let mut out = vec![];
    for (i, p) in files.iter().enumerate() {
        let n = start + i as u64;
        let mut name = pattern.replace("{name}", &paths::stem(p)).replace("{ext}", &paths::ext(p));
        while let Some(at) = name.find("{n") {
            let Some(len) = name[at..].find('}') else { break };
            let spec = &name[at + 2..at + len];
            let width: usize = spec.strip_prefix(':').and_then(|w| w.parse().ok()).unwrap_or(0);
            if !spec.is_empty() && !spec.starts_with(':') {
                break; // not ours ({name} is gone already; leave other text)
            }
            name.replace_range(at..at + len + 1, &format!("{n:0width$}"));
        }
        if !pattern.contains("{ext}") && !p.is_dir() {
            let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
            if !ext.is_empty() {
                name = format!("{name}.{ext}");
            }
        }
        if name.contains(['/', '\\']) || name.trim().is_empty() || name == "." || name == ".." {
            return Err(Error::msg(format!("\"{name}\" isn't a valid file name")));
        }
        out.push(paths::parent(p)?.join(name));
    }
    let mut seen = std::collections::HashSet::new();
    for t in &out {
        if !seen.insert(t.clone()) {
            return Err(Error::msg(format!("Two files would both be named {}; add {{n}} to the pattern", paths::name(t))));
        }
        if t.exists() && !files.contains(t) {
            return Err(Error::msg(format!("{} already exists", paths::name(t))));
        }
    }
    Ok(out)
}

fn rename(files: &[PathBuf], targets: &[PathBuf]) -> Result {
    // Through temporary names when a new name is another selected file's current one.
    let swaps = targets.iter().zip(files).any(|(t, f)| t != f && files.contains(t));
    let mut from: Vec<PathBuf> = files.to_vec();
    if swaps {
        for (i, f) in files.iter().enumerate() {
            let tmp = paths::parent(f)?.join(format!(".knobdeck-rename-{}-{i}", std::process::id()));
            std::fs::rename(f, &tmp).map_err(paths::io)?;
            from[i] = tmp;
        }
    }
    for (f, t) in from.iter().zip(targets) {
        if f != t {
            std::fs::rename(f, t).map_err(|e| Error::msg(format!("{}: {e}", paths::name(t))))?;
        }
    }
    Ok(())
}

fn copy_text(files: &[PathBuf], what: &str, ctx: &Ctx) -> Result {
    let text = match what {
        "name" => files.iter().map(|p| paths::name(p)).collect::<Vec<_>>().join("\n"),
        "folder" => work_folder(ctx)?.display().to_string(),
        _ => files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"),
    };
    sys::set_clipboard(&text)
}

fn to_folder(ctx: &Ctx, files: &[PathBuf]) -> Result {
    let raw = ctx.expand(&ctx.input_str("folder"))?;
    if raw.trim().is_empty() {
        return Err(Error::msg("Choose a folder in the action's settings"));
    }
    let dest = paths::expand_home(&raw);
    std::fs::create_dir_all(&dest).map_err(|e| Error::msg(format!("{}: {e}", dest.display())))?;
    let copy = ctx.input_str("mode") == "copy";
    paths::each(files, |p| {
        if dest.starts_with(p) {
            return Err(Error::msg("can't go into itself"));
        }
        let ext = if p.is_dir() { String::new() } else { p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default() };
        let stem = if p.is_dir() { paths::name(p) } else { paths::stem(p) };
        let to = paths::free_path(&dest, &stem, &ext);
        if copy { paths::copy_any(p, &to) } else { paths::move_any(p, &to) }.map_err(paths::io)
    })
    .map(|_| ())
}

fn new_item(ctx: &Ctx) -> Result {
    let dir = work_folder(ctx)?;
    let folder = ctx.input_str("kind") == "folder";
    let raw = ctx.expand(&ctx.input_str("name"))?;
    let name = if raw.trim().is_empty() { if folder { "New folder".to_string() } else { "New file.txt".to_string() } } else { raw.trim().to_string() };
    if name.contains(['/', '\\']) {
        return Err(Error::msg("The name can't contain / or \\"));
    }
    let p = Path::new(&name);
    let (stem, ext) = if folder { (name.clone(), String::new()) } else { (paths::stem(p), p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()) };
    let target = paths::free_path(&dir, &stem, &ext);
    if folder {
        std::fs::create_dir(&target).map_err(paths::io)?;
    } else {
        std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(paths::io)?;
    }
    Ok(())
}

/// One operation on the selection, by id: the actions and the wheel's slices share these.
fn run(op: &str, ctx: &Ctx) -> Result {
    let files = selected(ctx);
    match op {
        "convert-image" => {
            let to = images::Format::parse(&ctx.input_str("format"));
            paths::each(&only(files?, images::is_image, "images")?, |p| images::convert(p, to)).map(|_| ())
        }
        "to-png" => paths::each(&only(files?, images::is_image, "images")?, |p| images::convert(p, images::Format::Png)).map(|_| ()),
        "to-jpeg" => paths::each(&only(files?, images::is_image, "images")?, |p| images::convert(p, images::Format::Jpeg)).map(|_| ()),
        "resize-image" => {
            let n = ctx.input_f64("size");
            let size = match ctx.input_str("mode").as_str() {
                "percent" => images::Size::Percent(n.unwrap_or(50.0)),
                _ => images::Size::MaxEdge(n.unwrap_or(1920.0) as u32),
            };
            if matches!(size, images::Size::Percent(p) if p <= 0.0 || p > 1000.0) || matches!(size, images::Size::MaxEdge(0)) {
                return Err(Error::msg("Enter a size above 0"));
            }
            paths::each(&only(files?, images::is_image, "images")?, |p| images::resize(p, size)).map(|_| ())
        }
        "half-size" => paths::each(&only(files?, images::is_image, "images")?, |p| images::resize(p, images::Size::Percent(50.0))).map(|_| ()),
        "rotate-image" => {
            let deg = ctx.input_str("angle").parse().unwrap_or(90);
            paths::each(&only(files?, images::is_image, "images")?, |p| images::rotate(p, deg)).map(|_| ())
        }
        "strip-metadata" => paths::each(&only(files?, images::is_image, "images")?, images::strip).map(|_| ()),
        "merge-pdfs" => pdf::merge(&only(files?, is_pdf, "two or more PDFs")?).map(|_| ()),
        "split-pdf" => paths::each(&only(files?, is_pdf, "a PDF")?, pdf::split).map(|_| ()),
        "images-to-pdf" => pdf::from_images(&only(files?, images::is_image, "images")?).map(|_| ()),
        "zip" => archive::zip(&files?).map(|_| ()),
        "unzip" => paths::each(&only(files?, is_zip, "a zip file")?, archive::unzip).map(|_| ()),
        "copy-path" => copy_text(&files?, &ctx.input_str("what"), ctx),
        "copy-names" => copy_text(&files?, "name", ctx),
        "rename" => {
            let files = files?;
            let pattern = ctx.expand(&ctx.input_str("pattern"))?;
            let start = ctx.input_f64("start").map_or(1, |s| s.max(0.0) as u64);
            let targets = rename_targets(&files, &pattern, start)?;
            rename(&files, &targets)
        }
        "move-copy" => to_folder(ctx, &files?),
        "new-item" => new_item(ctx),
        _ => Err(Error::msg(format!("Unknown file action {op}"))),
    }
}

/// The wheel's slices for a selection: only what fits it.
fn wheel_items(files: &[PathBuf]) -> Vec<(&'static str, &'static str, &'static str)> {
    let images = files.iter().filter(|p| images::is_image(p)).count();
    let pdfs = files.iter().filter(|p| is_pdf(p)).count();
    let zips = files.iter().filter(|p| is_zip(p)).count();
    let mut v = vec![];
    if images > 0 {
        v.extend([("to-png", "To PNG", "image"), ("to-jpeg", "To JPEG", "image"), ("half-size", "Half size", "scaling"), ("rotate", "Rotate right", "rotate-cw"), ("strip-metadata", "Remove metadata", "eraser"), ("images-to-pdf", "Images to PDF", "file-text")]);
    }
    if pdfs >= 2 {
        v.push(("merge-pdfs", "Merge PDFs", "combine"));
    }
    if pdfs >= 1 {
        v.push(("split-pdf", "Split pages", "split"));
    }
    if zips > 0 {
        v.push(("unzip", "Unzip", "package-open"));
    }
    if !files.is_empty() {
        v.extend([("zip", "Zip", "archive"), ("copy-path", "Copy path", "clipboard-copy"), ("copy-names", "Copy name", "clipboard-copy")]);
    }
    v
}

fn files_action(id: &str, label: &str, icon: &str) -> Action {
    let op = id.to_string();
    Action::button(id, label).icon(icon).needs(&[Need::Files]).needs_timeout(READ_BUDGET).on_press(move |ctx| run(&op, ctx))
}

fn main() {
    Plugin::new(&format!("files-{}", std::env::consts::OS), "Files")
        .version(env!("CARGO_PKG_VERSION"))
        .platforms(&[std::env::consts::OS]) // one package per OS
        .author("KnobDeck")
        .description(if cfg!(windows) {
            "Act on the files selected in File Explorer: convert and resize images, merge PDFs, zip, rename, copy paths."
        } else if cfg!(target_os = "macos") {
            "Act on the files selected in Finder: convert and resize images, merge PDFs, zip, rename, copy paths."
        } else {
            "Act on the files selected in Dolphin: convert and resize images, merge PDFs, zip, rename, copy paths."
        })
        .icon("app-icon.svg")
        .action(files_action("convert-image", "Convert Images", "image").inputs(vec![input::select("format", "To", &[("png", "PNG"), ("jpeg", "JPEG"), ("webp", "WebP (lossless)")]).default_value("png")]))
        .action(files_action("resize-image", "Resize Images", "scaling").inputs(vec![
            input::select("mode", "By", &[("edge", "Longest edge (px)"), ("percent", "Percent")]).default_value("edge"),
            input::text("size", "Size").placeholder("1920 or 50").default_value("1920"),
        ]))
        .action(files_action("rotate-image", "Rotate Images", "rotate-cw").inputs(vec![input::select("angle", "Angle", &[("90", "90° right"), ("180", "180°"), ("270", "90° left")]).default_value("90")]))
        .action(files_action("strip-metadata", "Remove Image Metadata", "eraser"))
        .action(files_action("images-to-pdf", "Images to PDF", "file-text"))
        .action(files_action("merge-pdfs", "Merge PDFs", "combine"))
        .action(files_action("split-pdf", "Split PDF into Pages", "split"))
        .action(files_action("zip", "Zip", "archive"))
        .action(files_action("unzip", "Unzip", "package-open"))
        .action(files_action("copy-path", "Copy Path", "clipboard-copy").inputs(vec![input::select("what", "Copy", &[("path", "Full paths"), ("name", "Names"), ("folder", "The folder's path")]).default_value("path")]))
        .action(files_action("rename", "Rename", "pencil").inputs(vec![
            input::text("pattern", "New name").default_value("{name}-{n}").accepts_placeholders(),
            input::text("start", "First number").default_value("1"),
        ]))
        .action(files_action("move-copy", "Move or Copy to Folder", "folder-input").inputs(vec![
            input::select("mode", "Action", &[("move", "Move"), ("copy", "Copy")]).default_value("move"),
            input::text("folder", "Folder").placeholder(if cfg!(windows) { r"C:\Users\me\Pictures" } else { "~/Pictures" }).accepts_placeholders(),
        ]))
        .action(files_action("new-item", "New File in Folder", "file-plus").inputs(vec![
            input::select("kind", "Create", &[("file", "File"), ("folder", "Folder")]).default_value("file"),
            input::text("name", "Name").default_value("New file.txt").accepts_placeholders(),
        ]))
        .action(
            Action::wheel("files-wheel", "Files Wheel")
                .icon("files")
                .on_open(|o| {
                    let files = match o.files() {
                        Ok(f) => f,
                        Err(e) => return o.fail(e.message()),
                    };
                    Ok(wheel_items(&files).into_iter().map(|(id, label, icon)| json!({"id": id, "label": label, "icon": icon})).collect())
                })
                .needs(&[Need::Files])
                .needs_timeout(READ_BUDGET)
                .on_child_press(|ctx| match ctx.child_id.as_deref() {
                    Some("rotate") => paths::each(&only(selected(ctx)?, images::is_image, "images")?, |p| images::rotate(p, 90)).map(|_| ()),
                    Some("copy-path") => copy_text(&selected(ctx)?, "path", ctx),
                    Some(op) => run(op, ctx),
                    None => Ok(()),
                }),
        )
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn natural_order() {
        let mut v = vec![p("/a/page10.pdf"), p("/a/page2.pdf"), p("/a/Page1.pdf")];
        v.sort_by_cached_key(|p| natural_key(p));
        assert_eq!(v, vec![p("/a/Page1.pdf"), p("/a/page2.pdf"), p("/a/page10.pdf")]);
    }

    #[test]
    fn rename_patterns() {
        let files = vec![p("/nowhere/a.jpg"), p("/nowhere/b.png")];
        assert_eq!(rename_targets(&files, "trip-{n:3}", 1).unwrap(), vec![p("/nowhere/trip-001.jpg"), p("/nowhere/trip-002.png")]);
        assert_eq!(rename_targets(&files, "{name} copy.{ext}", 1).unwrap(), vec![p("/nowhere/a copy.jpg"), p("/nowhere/b copy.png")]);
        assert!(rename_targets(&files, "same", 1).is_ok()); // same.jpg, same.png
        assert!(rename_targets(&[p("/nowhere/a.jpg"), p("/nowhere/c.jpg")], "same", 1).is_err());
        assert!(rename_targets(&files, "x/{n}", 1).is_err());
    }

    #[test]
    fn rename_swaps_through_temp_names() {
        let d = std::env::temp_dir().join(format!("kd-files-ren-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("1.txt"), d.join("2.txt"));
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        let files = vec![b.clone(), a.clone()];
        let t = rename_targets(&files, "{n}", 1).unwrap();
        rename(&files, &t).unwrap();
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "b");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "a");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn wheel_fits_selection() {
        let ids = |f: &[PathBuf]| wheel_items(f).into_iter().map(|i| i.0).collect::<Vec<_>>();
        let pics = ids(&[p("/x/a.png")]);
        assert!(pics.contains(&"to-jpeg") && !pics.contains(&"merge-pdfs"));
        let pdfs = ids(&[p("/x/a.pdf"), p("/x/b.pdf")]);
        assert!(pdfs.contains(&"merge-pdfs") && !pdfs.contains(&"to-png"));
        assert!(wheel_items(&[]).len() <= 16 && wheel_items(&[p("/a.png"), p("/b.pdf"), p("/c.zip")]).len() <= 16);
    }
}
