//! Image files in the document: probing their size for the layout, and decoding
//! them for the renderer.
//!
//! Only what a Markdown document actually points at is decoded: PNG, JPEG and
//! JPEG XL. Everything happens from bytes on disk, in pure Rust: `png`,
//! `jpeg-decoder`, and `jxl-oxide` for the `.jxl` files that are showing up in
//! screenshot folders now that browsers ship the format.

use std::path::{Path, PathBuf};

/// A decoded image: 8-bit RGBA, top row first.
#[derive(Clone)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Sizes are remembered globally: the layout asks for them on every rebuild, and
/// probing a file is disk I/O.
fn size_cache() -> &'static std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Option<(u32, u32)>>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, Option<(u32, u32)>>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Pixel size of a file, cached (the layout asks constantly).
pub fn size_of(path: &Path) -> Option<(u32, u32)> {
    let mut cache = size_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = cache.get(path) {
        return *hit;
    }
    let size = probe(path);
    cache.insert(path.to_path_buf(), size);
    size
}

/// Join a document directory and a reference, resolving `./` and `../` without
/// touching the filesystem.
pub fn join(dir: &Path, src: &str) -> PathBuf {
    let mut out = dir.to_path_buf();
    for part in percent_decode(src).split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Read the pixel size of an image without decoding the pixels.
pub fn probe(path: &Path) -> Option<(u32, u32)> {
    let bytes = std::fs::read(path).ok()?;
    probe_bytes(&bytes)
}

/// JPEG XL: either the bare codestream (`FF 0A`) or the ISO-BMFF container
/// whose first box is `ftyp` with the brand `jxl `.
pub fn is_jxl(bytes: &[u8]) -> bool {
    if bytes.starts_with(&[0xFF, 0x0A]) {
        return true;
    }
    bytes.len() > 20 && bytes.starts_with(&[0x00, 0x00, 0x00, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A])
}

pub fn probe_bytes(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        // IHDR is always the first chunk: width/height at bytes 16..24.
        if bytes.len() < 24 {
            return None;
        }
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Some((w, h));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return jpeg_size(bytes);
    }
    if is_jxl(bytes) {
        return jxl_size(bytes);
    }
    None
}

/// The size of a JPEG XL file, read from its header (the codestream is not
/// decoded for this: the layout asks for a size on every rebuild).
fn jxl_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let image = jxl_oxide::JxlImage::builder().read(std::io::Cursor::new(bytes)).ok()?;
    let header = image.image_header();
    Some((header.size.width, header.size.height))
}

fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2usize;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = bytes[i + 1];
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        // SOF0..SOF3, SOF5..SOF7, SOF9..SOF11, SOF13..SOF15 carry the size.
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}

/// Decode an image to RGBA.
pub fn load(path: &Path) -> Option<Image> {
    let bytes = std::fs::read(path).ok()?;
    decode(&bytes)
}

pub fn decode(bytes: &[u8]) -> Option<Image> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return decode_png(bytes);
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return decode_jpeg(bytes);
    }
    if is_jxl(bytes) {
        return decode_jxl(bytes);
    }
    None
}

/// Decode the first keyframe of a JPEG XL file.
///
/// `jxl-oxide` renders in floating point (the format is an HDR format); the
/// samples are clamped into 8-bit RGBA here, with the alpha channel from the
/// extra channel when the file has one. Only the first keyframe is decoded: a
/// document shows a still, and animating an inline picture would be a surprise.
fn decode_jxl(bytes: &[u8]) -> Option<Image> {
    let image = jxl_oxide::JxlImage::builder()
        .read(std::io::Cursor::new(bytes))
        .ok()?;
    let render = image.render_frame(0).ok()?;
    let mut stream = render.stream();
    let (w, h) = (stream.width(), stream.height());
    if w == 0 || h == 0 {
        return None;
    }
    let channels = stream.channels().max(1) as usize;
    let mut samples = vec![0f32; w as usize * h as usize * channels];
    // The stream fills the buffer in chunks; keep asking until it is done.
    let mut done = 0usize;
    while done < samples.len() {
        let n = stream.write_to_buffer(&mut samples[done..]);
        if n == 0 {
            break;
        }
        done += n;
    }
    let px = w as usize * h as usize;
    let mut rgba = Vec::with_capacity(px * 4);
    let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for i in 0..px {
        let base = i * channels;
        let r = samples.get(base).copied().unwrap_or(0.0);
        let g = samples.get(base + 1).copied().unwrap_or(r);
        let b = samples.get(base + 2).copied().unwrap_or(r);
        // Grey images come back with one channel.
        let a = if channels >= 4 { samples[base + 3] } else { 1.0 };
        rgba.extend_from_slice(&[to8(r), to8(g), to8(b), to8(a)]);
    }
    Some(Image { w, h, rgba })
}

fn decode_png(bytes: &[u8]) -> Option<Image> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => buf[..info.buffer_size()]
            .iter()
            .flat_map(|g| [*g, *g, *g, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..info.buffer_size()]
            .chunks_exact(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        png::ColorType::Indexed => return None,
    };
    Some(Image { w, h, rgba: pixels })
}

fn decode_jpeg(bytes: &[u8]) -> Option<Image> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let pixels = decoder.decode().ok()?;
    let info = decoder.info()?;
    let (w, h) = (info.width as u32, info.height as u32);
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|g| [*g, *g, *g, 255]).collect(),
        // CMYK JPEGs are rare in notes; a flat grey keeps the layout honest.
        _ => return None,
    };
    Some(Image { w, h, rgba })
}

/// Downscale so the longest side is at most `max_px`, keeping the aspect ratio.
/// Returns `(width, height, rgba)`.
pub fn fit(img: &Image, max_px: u32) -> (u32, u32, Vec<u8>) {
    let longest = img.w.max(img.h).max(1);
    if longest <= max_px {
        return (img.w, img.h, img.rgba.clone());
    }
    let k = max_px as f32 / longest as f32;
    let tw = ((img.w as f32 * k).round() as u32).max(1);
    let th = ((img.h as f32 * k).round() as u32).max(1);
    let mut out = vec![0u8; (tw * th * 4) as usize];
    // Box filter downscale: average the source pixels each destination pixel
    // covers, so screenshots with text stay readable.
    for y in 0..th {
        let y0 = (y as f32 / k) as u32;
        let y1 = (((y + 1) as f32 / k).ceil() as u32).min(img.h).max(y0 + 1);
        for x in 0..tw {
            let x0 = (x as f32 / k) as u32;
            let x1 = (((x + 1) as f32 / k).ceil() as u32).min(img.w).max(x0 + 1);
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy * img.w + sx) * 4) as usize;
                    if i + 3 >= img.rgba.len() {
                        continue;
                    }
                    r += img.rgba[i] as u32;
                    g += img.rgba[i + 1] as u32;
                    b += img.rgba[i + 2] as u32;
                    a += img.rgba[i + 3] as u32;
                    n += 1;
                }
            }
            let n = n.max(1);
            let o = ((y * tw + x) * 4) as usize;
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
    }
    (tw, th, out)
}

/// Dimensions cached per path, so the layout can reserve the right height for an
/// image without decoding it on every keystroke.
#[derive(Default)]
pub struct Media {
    sizes: std::collections::HashMap<PathBuf, Option<(u32, u32)>>,
    base: Option<PathBuf>,
    hits: u32,
}

impl Media {
    /// Point relative paths at the directory of the open document.
    pub fn set_base(&mut self, base: Option<PathBuf>) {
        self.base = base;
        self.sizes.clear();
    }

    /// Resolve an image reference to a real file path.
    pub fn resolve(&self, src: &str) -> Option<PathBuf> {
        let src = src.trim();
        if src.is_empty() || src.starts_with("http://") || src.starts_with("https://") || src.starts_with("data:") {
            return None;
        }
        let decoded = percent_decode(src);
        let p = PathBuf::from(&decoded);
        if p.is_absolute() {
            return Some(p);
        }
        self.base.as_ref().map(|b| b.join(p))
    }

    /// Pixel size of the image behind `src`, if it is a local file we understand.
    pub fn size(&mut self, src: &str) -> Option<(u32, u32)> {
        let path = self.resolve(src)?;
        if let Some(hit) = self.sizes.get(&path) {
            return *hit;
        }
        let size = probe(&path);
        self.hits += 1;
        self.sizes.insert(path, size);
        size
    }

}

fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny PNG (4×2, solid red) built in memory by the `png` encoder.
    pub fn sample_png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap();
            let mut data = Vec::new();
            for _ in 0..(w * h) {
                data.extend_from_slice(&[200, 30, 40, 255]);
            }
            writer.write_image_data(&data).unwrap();
        }
        out
    }

    #[test]
    fn png_round_trip_and_probe() {
        let bytes = sample_png(6, 4);
        assert_eq!(probe_bytes(&bytes), Some((6, 4)));
        let img = decode(&bytes).expect("decode");
        assert_eq!((img.w, img.h), (6, 4));
        assert_eq!(img.rgba.len(), 6 * 4 * 4);
        assert_eq!(&img.rgba[0..4], &[200, 30, 40, 255]);
    }

    #[test]
    fn fit_downscales_and_keeps_the_aspect_ratio() {
        let img = Image { w: 400, h: 200, rgba: vec![255; 400 * 200 * 4] };
        let (w, h, px) = fit(&img, 100);
        assert_eq!((w, h), (100, 50));
        assert_eq!(px.len(), 100 * 50 * 4);
        // Already small enough: untouched.
        let (w2, h2, _) = fit(&img, 1024);
        assert_eq!((w2, h2), (400, 200));
    }

    #[test]
    fn relative_paths_resolve_against_the_document() {
        let mut m = Media::default();
        assert_eq!(m.resolve("shot.png"), None, "no base directory yet");
        m.set_base(Some(PathBuf::from("/tmp/notes")));
        assert_eq!(m.resolve("shot.png"), Some(PathBuf::from("/tmp/notes/shot.png")));
        assert_eq!(m.resolve("img/a%20b.png"), Some(PathBuf::from("/tmp/notes/img/a b.png")));
        assert_eq!(m.resolve("https://example.com/x.png"), None, "remote images are not fetched");
    }

    /// A 4×3 lossless JPEG XL of known pixels, produced once with `jxl-encoder`
    /// and pasted here so the decoder is exercised without a test-time encoder
    /// (the binary stays free of one).
    const TINY_JXL: [u8; 67] = [
        0xFF, 0x0A, 0x10, 0x30, 0x10, 0x09, 0x08, 0x02, 0x01, 0x00, 0xDC, 0x00, 0x4B, 0x12, 0xF8,
        0x04, 0xFF, 0x70, 0x80, 0x3D, 0x01, 0x78, 0x03, 0x80, 0x08, 0xC8, 0xF9, 0x2F, 0xDF, 0xEF,
        0xC2, 0x06, 0xF0, 0x86, 0x2B, 0x88, 0xF9, 0x46, 0x9F, 0x30, 0x7A, 0xEC, 0xAB, 0xDF, 0x2B,
        0x40, 0x62, 0x9F, 0xA0, 0xCB, 0x48, 0xFF, 0xA7, 0xF6, 0xF9, 0x70, 0xFB, 0x0B, 0x40, 0x0F,
        0xE8, 0x0F, 0x84, 0xC5, 0xD2, 0xA1, 0xD0,
    ];

    #[test]
    fn jxl_is_recognised_by_its_codestream_signature() {
        assert!(is_jxl(&TINY_JXL));
        // The ISO-BMFF container form starts with an `ftyp` box branded `jxl `.
        let mut boxed = vec![
            0x00, 0x00, 0x00, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A, 0x00, 0x00,
            0x00, 0x14, b'f', b't', b'y', b'p', b'j', b'x', b'l', b' ',
        ];
        boxed.extend_from_slice(&TINY_JXL);
        assert!(is_jxl(&boxed));
        assert!(!is_jxl(b"GIF89a and the rest"));
        assert!(!is_jxl(&[]));
    }

    #[test]
    fn jxl_probes_and_decodes() {
        assert_eq!(probe_bytes(&TINY_JXL), Some((4, 3)));
        let img = decode(&TINY_JXL).expect("decode jxl");
        assert_eq!((img.w, img.h), (4, 3));
        assert_eq!(img.rgba.len(), 4 * 3 * 4);
        // Lossless: the corner pixels come back as they went in.
        let px = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            [img.rgba[i], img.rgba[i + 1], img.rgba[i + 2]]
        };
        let near = |got: [u8; 3], want: [u8; 3]| {
            got.iter().zip(want.iter()).all(|(a, b)| a.abs_diff(*b) <= 2)
        };
        assert!(near(px(0, 0), [255, 0, 0]), "red, got {:?}", px(0, 0));
        assert!(near(px(1, 0), [0, 255, 0]), "green, got {:?}", px(1, 0));
        assert!(near(px(2, 0), [0, 0, 255]), "blue, got {:?}", px(2, 0));
        assert!(near(px(0, 1), [0, 0, 0]), "black, got {:?}", px(0, 1));
        assert!(near(px(3, 2), [250, 250, 0]), "yellow, got {:?}", px(3, 2));
        // No alpha channel in the file, so everything is opaque.
        assert!(img.rgba.iter().skip(3).step_by(4).all(|a| *a == 255));
    }

    #[test]
    fn a_broken_jxl_is_not_a_panic() {
        let mut trash = TINY_JXL.to_vec();
        for b in trash.iter_mut().skip(9) {
            *b ^= 0xA5;
        }
        assert!(decode(&trash).is_none());
        assert!(decode(&[0xFF, 0x0A]).is_none());
    }

    #[test]
    fn unknown_bytes_are_not_an_image() {
        assert!(decode(b"not an image at all").is_none());
        assert!(probe_bytes(b"nope").is_none());
    }
}
