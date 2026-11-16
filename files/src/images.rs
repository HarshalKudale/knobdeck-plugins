//! Images: convert, resize, rotate, remove metadata. Decoded and encoded in Rust (`image`); HEIC
//! is read on macOS through the system's own converter (`sips`, ImageIO).

use crate::paths::{self, free_path};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{DynamicImage, ImageDecoder, ImageReader};
use knobdeck_sdk::{Error, Result};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// JPEG quality for re-encoded photos.
const QUALITY: u8 = 90;

/// Extensions read as images. HEIC only on macOS, where the system decodes it.
#[cfg(target_os = "macos")]
pub const READS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif"];
#[cfg(not(target_os = "macos"))]
pub const READS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff"];

pub fn is_image(p: &Path) -> bool {
    READS.contains(&paths::ext(p).as_str())
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::msg(e.to_string())
}

/// Decode upright (the EXIF orientation applied).
pub fn open(p: &Path) -> Result<DynamicImage> {
    let ext = paths::ext(p);
    if ext == "heic" || ext == "heif" {
        return open_heic(p);
    }
    let mut dec = ImageReader::open(p).map_err(err)?.with_guessed_format().map_err(err)?.into_decoder().map_err(err)?;
    let orientation = dec.orientation().ok();
    let mut img = DynamicImage::from_decoder(dec).map_err(err)?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    Ok(img)
}

#[cfg(target_os = "macos")]
fn open_heic(p: &Path) -> Result<DynamicImage> {
    let tmp = std::env::temp_dir().join(format!("knobdeck-heic-{}-{}.png", std::process::id(), paths::stem(p)));
    let out = std::process::Command::new("/usr/bin/sips").args(["-s", "format", "png"]).arg(p).arg("--out").arg(&tmp).output().map_err(err)?;
    if !out.status.success() {
        return Err(Error::msg(format!("Couldn't read the HEIC image: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    let img = image::open(&tmp).map_err(err);
    let _ = std::fs::remove_file(&tmp);
    img
}

#[cfg(not(target_os = "macos"))]
fn open_heic(_: &Path) -> Result<DynamicImage> {
    Err(Error::msg("HEIC images can be read on macOS only")) // not offered: READS leaves them out
}

/// What an image is saved as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Png,
    Jpeg,
    Webp,
}

impl Format {
    pub fn parse(s: &str) -> Format {
        match s {
            "jpeg" | "jpg" => Format::Jpeg,
            "webp" => Format::Webp,
            _ => Format::Png,
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
            Format::Webp => "webp",
        }
    }
    /// The format to keep for an input (formats it can't write become PNG).
    fn of(p: &Path) -> Format {
        match paths::ext(p).as_str() {
            "jpg" | "jpeg" | "heic" | "heif" => Format::Jpeg,
            "webp" => Format::Webp,
            _ => Format::Png,
        }
    }
}

/// Save without metadata (the encoders write none). JPEG has no alpha: transparent pixels go
/// on white. WebP is written lossless (the encoder has no lossy mode).
pub fn save(img: &DynamicImage, to: &Path, format: Format) -> Result<()> {
    let file = std::fs::File::create(to).map_err(paths::io)?;
    let mut w = BufWriter::new(file);
    let r = match format {
        Format::Jpeg => flatten(img).write_with_encoder(JpegEncoder::new_with_quality(&mut w, QUALITY)),
        Format::Png => img.write_with_encoder(PngEncoder::new(&mut w)),
        Format::Webp => {
            let img = if img.color().has_alpha() { DynamicImage::ImageRgba8(img.to_rgba8()) } else { DynamicImage::ImageRgb8(img.to_rgb8()) };
            img.write_with_encoder(WebPEncoder::new_lossless(&mut w))
        }
    };
    let r = r.map_err(err).and_then(|_| w.flush().map_err(paths::io));
    if r.is_err() {
        let _ = std::fs::remove_file(to);
    }
    r
}

/// RGB on a white background.
pub fn flatten(img: &DynamicImage) -> DynamicImage {
    if !img.color().has_alpha() {
        return DynamicImage::ImageRgb8(img.to_rgb8());
    }
    let rgba = img.to_rgba8();
    let mut out = image::RgbImage::new(rgba.width(), rgba.height());
    for (o, p) in out.pixels_mut().zip(rgba.pixels()) {
        let a = p[3] as u32;
        for c in 0..3 {
            o[c] = ((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
        }
    }
    DynamicImage::ImageRgb8(out)
}

pub fn convert(p: &Path, to: Format) -> Result<PathBuf> {
    let img = open(p)?;
    let out = free_path(paths::parent(p)?, &paths::stem(p), to.ext());
    save(&img, &out, to)?;
    Ok(out)
}

/// How to resize: a percentage, or the longest edge in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Size {
    Percent(f64),
    MaxEdge(u32),
}

/// The new size; never enlarges with `MaxEdge`.
pub fn target_size(w: u32, h: u32, size: Size) -> (u32, u32) {
    let scale = match size {
        Size::Percent(p) => p / 100.0,
        Size::MaxEdge(m) => (m as f64 / w.max(h) as f64).min(1.0),
    };
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

pub fn resize(p: &Path, size: Size) -> Result<PathBuf> {
    let img = open(p)?;
    let (w, h) = target_size(img.width(), img.height(), size);
    let out_img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
    let format = Format::of(p);
    let out = free_path(paths::parent(p)?, &format!("{} ({w}×{h})", paths::stem(p)), format.ext());
    save(&out_img, &out, format)?;
    Ok(out)
}

pub fn rotate(p: &Path, degrees: u32) -> Result<PathBuf> {
    let img = open(p)?;
    let img = match degrees {
        90 => img.rotate90(),
        180 => img.rotate180(),
        270 => img.rotate270(),
        _ => return Err(Error::msg("Rotate by 90, 180 or 270 degrees")),
    };
    let format = Format::of(p);
    let out = free_path(paths::parent(p)?, &format!("{} (rotated)", paths::stem(p)), format.ext());
    save(&img, &out, format)?;
    Ok(out)
}

/// A copy without metadata (EXIF with location, XMP, IPTC, comments). JPEG and PNG keep their
/// pixels untouched (the metadata blocks are dropped); a rotated JPEG and other formats are
/// re-encoded upright.
pub fn strip(p: &Path) -> Result<PathBuf> {
    let bytes = std::fs::read(p).map_err(paths::io)?;
    let ext = paths::ext(p);
    let dir = paths::parent(p)?;
    let stem = format!("{} (no metadata)", paths::stem(p));
    let lossless = match ext.as_str() {
        "jpg" | "jpeg" if upright(p) => strip_jpeg(&bytes),
        "png" => strip_png(&bytes),
        _ => None,
    };
    if let Some(clean) = lossless {
        let out = free_path(dir, &stem, &ext);
        std::fs::write(&out, clean).map_err(paths::io)?;
        return Ok(out);
    }
    let format = Format::of(p);
    let out = free_path(dir, &stem, format.ext());
    save(&open(p)?, &out, format)?;
    Ok(out)
}

/// No EXIF rotation to keep (dropping EXIF would turn the photo sideways).
fn upright(p: &Path) -> bool {
    let Ok(dec) = ImageReader::open(p).and_then(|r| r.with_guessed_format()).map_err(err).and_then(|r| r.into_decoder().map_err(err)) else { return false };
    let mut dec = dec;
    matches!(dec.orientation(), Ok(image::metadata::Orientation::NoTransforms))
}

/// JPEG without APP1 (EXIF, XMP), APP13 (IPTC) and other APP blocks, and comments. Keeps JFIF
/// (APP0), the ICC color profile (APP2 `ICC_PROFILE`) and Adobe's color transform (APP14).
pub fn strip_jpeg(b: &[u8]) -> Option<Vec<u8>> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return None;
    }
    let mut out = vec![0xFF, 0xD8];
    let mut i = 2;
    while i + 4 <= b.len() {
        if b[i] != 0xFF {
            return None;
        }
        let marker = b[i + 1];
        if marker == 0xFF {
            i += 1; // fill byte
            continue;
        }
        // Start of scan: the rest is image data.
        if marker == 0xDA {
            out.extend_from_slice(&b[i..]);
            return Some(out);
        }
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        let end = i + 2 + len;
        if len < 2 || end > b.len() {
            return None;
        }
        let body = &b[i + 4..end];
        let keep = match marker {
            0xE0 => true,
            0xE2 => body.starts_with(b"ICC_PROFILE\0"),
            0xEE => body.starts_with(b"Adobe"),
            0xE1..=0xEF | 0xFE => false,
            _ => true,
        };
        if keep {
            out.extend_from_slice(&b[i..end]);
        }
        i = end;
    }
    None
}

/// PNG without text chunks, EXIF and the modification time.
pub fn strip_png(b: &[u8]) -> Option<Vec<u8>> {
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !b.starts_with(SIG) {
        return None;
    }
    let mut out = SIG.to_vec();
    let mut i = SIG.len();
    while i + 12 <= b.len() {
        let len = u32::from_be_bytes(b[i..i + 4].try_into().ok()?) as usize;
        let end = i.checked_add(12 + len)?;
        if end > b.len() {
            return None;
        }
        let kind = &b[i + 4..i + 8];
        if !matches!(kind, b"tEXt" | b"zTXt" | b"iTXt" | b"eXIf" | b"tIME") {
            out.extend_from_slice(&b[i..end]);
        }
        if kind == b"IEND" {
            return Some(out);
        }
        i = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(target_size(4000, 3000, Size::Percent(50.0)), (2000, 1500));
        assert_eq!(target_size(4000, 3000, Size::MaxEdge(1000)), (1000, 750));
        assert_eq!(target_size(400, 300, Size::MaxEdge(1000)), (400, 300));
    }

    #[test]
    fn jpeg_loses_exif_keeps_scan() {
        // SOI, APP0 (JFIF), APP1 (Exif), COM, SOS + data, EOI.
        let mut j = vec![0xFF, 0xD8];
        j.extend([0xFF, 0xE0, 0, 7]);
        j.extend(b"JFIF\0");
        j.extend([0xFF, 0xE1, 0, 8]);
        j.extend(b"Exif\0\0");
        j.extend([0xFF, 0xFE, 0, 4, b'h', b'i']);
        j.extend([0xFF, 0xDA, 0, 2, 1, 2, 3, 0xFF, 0xD9]);
        let s = strip_jpeg(&j).unwrap();
        assert!(!s.windows(4).any(|w| w == b"Exif"));
        assert!(s.windows(4).any(|w| w == b"JFIF"));
        assert!(s.ends_with(&[0xFF, 0xDA, 0, 2, 1, 2, 3, 0xFF, 0xD9]));
    }

    #[test]
    fn png_round_trip_without_text() {
        let img = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(3, 2, image::Rgb([9, 8, 7])));
        let mut buf = vec![];
        img.write_with_encoder(PngEncoder::new(&mut buf)).unwrap();
        // Add a tEXt chunk after IHDR (8 + 25 bytes).
        let mut chunk = vec![0, 0, 0, 5];
        chunk.extend(b"tEXta\0bcd");
        chunk.extend([0, 0, 0, 0]);
        let mut with = buf[..33].to_vec();
        with.extend(&chunk);
        with.extend(&buf[33..]);
        let s = strip_png(&with).unwrap();
        assert_eq!(s, buf);
        assert_eq!(image::load_from_memory(&s).unwrap().to_rgb8().get_pixel(0, 0).0, [9, 8, 7]);
    }

    #[test]
    fn convert_resize_rotate_on_disk() {
        let d = std::env::temp_dir().join(format!("kd-files-img-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("pic.png");
        image::RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 128])).save(&src).unwrap();
        let jpg = convert(&src, Format::Jpeg).unwrap();
        assert_eq!(jpg, d.join("pic.jpg"));
        let px = image::open(&jpg).unwrap().to_rgb8().get_pixel(5, 5).0;
        assert!(px[0] > 240 && px[1] > 100 && px[1] < 150, "{px:?}"); // red at half alpha on white
        let small = resize(&src, Size::MaxEdge(10)).unwrap();
        assert_eq!(image::open(&small).unwrap().width(), 10);
        let r = rotate(&src, 90).unwrap();
        assert_eq!(image::open(&r).unwrap().height(), 40);
        let w = convert(&src, Format::Webp).unwrap();
        assert_eq!(image::open(&w).unwrap().width(), 40);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
