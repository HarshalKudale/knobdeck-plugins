//! PDF: merge, split into pages, images to one PDF (`lopdf`).

use crate::images;
use crate::paths::{self, free_path};
use knobdeck_sdk::{Error, Result};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn err(e: impl std::fmt::Display) -> Error {
    Error::msg(e.to_string())
}

fn load(p: &Path) -> Result<Document> {
    let mut doc = Document::load(p).map_err(|e| Error::msg(format!("{}: {e}", paths::name(p))))?;
    if doc.is_encrypted() {
        // Many PDFs are encrypted with an empty user password (copy protection only).
        doc.decrypt("").map_err(|_| Error::msg(format!("{} is password protected", paths::name(p))))?;
    }
    Ok(doc)
}

/// Page attributes a page can inherit from its parents in the page tree.
const INHERITED: &[&[u8]] = &[b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Copy inherited attributes onto the page, so it keeps them under a new parent.
fn own_inherited(doc: &Document, page: &mut Dictionary) {
    let mut parent = page.get(b"Parent").and_then(Object::as_reference).ok();
    let mut depth = 0;
    while let Some(id) = parent {
        let Ok(node) = doc.get_dictionary(id) else { break };
        for key in INHERITED {
            if !page.has(key) {
                if let Ok(v) = node.get(key) {
                    page.set(key.to_vec(), v.clone());
                }
            }
        }
        parent = node.get(b"Parent").and_then(Object::as_reference).ok();
        depth += 1;
        if depth > 64 {
            break;
        }
    }
}

/// Merge `inputs` in order into one new PDF next to the first.
pub fn merge(inputs: &[PathBuf]) -> Result<PathBuf> {
    if inputs.len() < 2 {
        return Err(Error::msg("Select two or more PDFs to merge"));
    }
    let mut max_id = 1;
    let mut pages: Vec<(ObjectId, Dictionary)> = vec![];
    let mut objects: BTreeMap<ObjectId, Object> = BTreeMap::new();
    for p in inputs {
        let mut doc = load(p)?;
        doc.renumber_objects_with(max_id);
        max_id = doc.max_id + 1;
        for (_, id) in doc.get_pages() {
            let mut page = doc.get_dictionary(id).map_err(err)?.clone();
            own_inherited(&doc, &mut page);
            pages.push((id, page));
        }
        objects.extend(doc.objects);
    }

    let mut out = Document::with_version("1.7");
    // Keep everything except the old document structure (catalogs, page trees, outlines, which
    // point at the old trees); the pages are added under one new tree.
    for (id, obj) in objects {
        let kind = obj.type_name().unwrap_or(b"");
        if !matches!(kind, b"Catalog" | b"Pages" | b"Page" | b"Outlines" | b"Outline") {
            out.objects.insert(id, obj);
        }
    }
    out.max_id = max_id;
    let pages_id = out.new_object_id();
    let mut kids = vec![];
    for (id, mut page) in pages {
        page.set("Parent", pages_id);
        page.remove(b"StructParents"); // the structure tree isn't carried over
        out.objects.insert(id, Object::Dictionary(page));
        kids.push(Object::Reference(id));
    }
    let count = kids.len() as i64;
    out.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = out.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    out.trailer.set("Root", catalog);
    out.prune_objects();
    out.renumber_objects();
    out.compress();
    let target = free_path(paths::parent(&inputs[0])?, &format!("{} (merged)", paths::stem(&inputs[0])), "pdf");
    out.save(&target).map_err(err)?;
    Ok(target)
}

/// One PDF per page, `name (page 3).pdf`, next to the input. Returns how many were written.
pub fn split(p: &Path) -> Result<usize> {
    let doc = load(p)?;
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    if pages.len() < 2 {
        return Err(Error::msg(format!("{} has only one page", paths::name(p))));
    }
    let dir = paths::parent(p)?;
    let stem = paths::stem(p);
    for &n in &pages {
        let mut one = doc.clone();
        let others: Vec<u32> = pages.iter().copied().filter(|&k| k != n).collect();
        one.delete_pages(&others);
        one.prune_objects();
        one.compress();
        one.save(free_path(dir, &format!("{stem} (page {n})"), "pdf")).map_err(err)?;
    }
    Ok(pages.len())
}

/// Page size in points: the image's shape, its long edge as long as an A4 page's (842 pt).
fn page_size(w: u32, h: u32) -> (f64, f64) {
    let scale = 842.0 / w.max(h) as f64;
    (w as f64 * scale, h as f64 * scale)
}

/// The image as a PDF image XObject: JPEGs as they are (no re-encoding), others as RGB pixels.
fn image_object(p: &Path) -> Result<(Stream, u32, u32)> {
    let ext = paths::ext(p);
    if ext == "jpg" || ext == "jpeg" {
        let reader = image::ImageReader::open(p).map_err(err)?.with_guessed_format().map_err(err)?;
        let mut dec = reader.into_decoder().map_err(err)?;
        use image::ImageDecoder;
        let (w, h) = dec.dimensions();
        let upright = matches!(dec.orientation(), Ok(image::metadata::Orientation::NoTransforms));
        let space = match dec.color_type() {
            image::ColorType::L8 => Some("DeviceGray"),
            image::ColorType::Rgb8 => Some("DeviceRGB"),
            _ => None, // CMYK and others: decode and embed pixels
        };
        if let (Some(space), true) = (space, upright) {
            let bytes = std::fs::read(p).map_err(paths::io)?;
            let dict = dictionary! {
                "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
                "ColorSpace" => space, "BitsPerComponent" => 8, "Filter" => "DCTDecode",
            };
            return Ok((Stream::new(dict, bytes), w, h));
        }
    }
    let rgb = images::flatten(&images::open(p)?).to_rgb8();
    let (w, h) = rgb.dimensions();
    let dict = dictionary! {
        "Type" => "XObject", "Subtype" => "Image", "Width" => w as i64, "Height" => h as i64,
        "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8,
    };
    let mut s = Stream::new(dict, rgb.into_raw());
    s.compress().map_err(err)?;
    Ok((s, w, h))
}

/// The images, one per page in the given order, as one PDF next to the first.
pub fn from_images(inputs: &[PathBuf]) -> Result<PathBuf> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut kids = vec![];
    for p in inputs {
        let (stream, w, h) = image_object(p).map_err(|e| Error::msg(format!("{}: {}", paths::name(p), e.message())))?;
        let (pw, ph) = page_size(w, h);
        let img = doc.add_object(stream);
        let content = format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q");
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), Object::Real(pw as f32), Object::Real(ph as f32)],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => img } },
        });
        kids.push(Object::Reference(page));
    }
    let count = kids.len() as i64;
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.compress();
    let first = inputs.first().ok_or_else(|| Error::msg("Select images first"))?;
    let target = free_path(paths::parent(first)?, &paths::stem(first), "pdf");
    doc.save(&target).map_err(err)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_page_pdf(path: &Path, label: &str) {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = vec![];
        for i in 0..2 {
            let content = doc.add_object(Stream::new(dictionary! {}, format!("% {label} {i}").into_bytes()));
            kids.push(doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id, "Contents" => content }).into());
        }
        // MediaBox inherited from the page tree.
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 2, "MediaBox" => vec![0.into(), 0.into(), 300.into(), 400.into()] }));
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", cat);
        doc.save(path).unwrap();
    }

    #[test]
    fn merge_split_images() {
        let d = std::env::temp_dir().join(format!("kd-files-pdf-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (a, b) = (d.join("a.pdf"), d.join("b.pdf"));
        two_page_pdf(&a, "A");
        two_page_pdf(&b, "B");

        let m = merge(&[a.clone(), b.clone()]).unwrap();
        let doc = Document::load(&m).unwrap();
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 4);
        let first = doc.get_dictionary(pages[&1]).unwrap();
        assert!(first.get(b"MediaBox").is_ok(), "inherited MediaBox kept");
        let last = doc.get_page_content(pages[&4]).unwrap();
        assert_eq!(String::from_utf8_lossy(&last), "% B 1");

        assert_eq!(split(&a).unwrap(), 2);
        let p2 = Document::load(d.join("a (page 2).pdf")).unwrap();
        assert_eq!(p2.get_pages().len(), 1);

        let img = d.join("x.png");
        image::RgbImage::from_pixel(20, 10, image::Rgb([1, 2, 3])).save(&img).unwrap();
        let jpg = d.join("y.jpg");
        image::RgbImage::from_pixel(10, 20, image::Rgb([1, 2, 3])).save(&jpg).unwrap();
        let out = from_images(&[img, jpg]).unwrap();
        assert_eq!(Document::load(&out).unwrap().get_pages().len(), 2);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
