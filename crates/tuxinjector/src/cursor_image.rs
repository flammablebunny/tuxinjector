// Loading for the custom-cursor system. PNG/GIF/JPEG go through the shared
// image_loader; .cur/.ico get their own little parser because the hotspot
// lives in the ICONDIR entry and the image crate won't hand it to us.
// All pure CPU here (no GL/GLFW), so it's easy to unit-test.

use std::path::{Path, PathBuf};

use tuxinjector_render::image_loader::{self, ImageData};

pub struct CursorImage {
    pub rgba: Vec<u8>,
    pub w: u32,
    pub h: u32,
    // Hotspot in file pixels. Only set for .cur files -- icons stash
    // planes/bpp in those same bytes. Takes priority over the config sliders.
    pub file_hotspot: Option<(u32, u32)>,
}

pub fn cursors_dir() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".config/tuxinjector/cursors"))
        .unwrap_or_default()
}

/// Find the file for a configured cursor name: a direct path, an exact file
/// name in the cursors folder, or a bare stem probed against known extensions.
pub fn resolve_cursor_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    let direct = PathBuf::from(name);
    if direct.is_absolute() && direct.exists() {
        return Some(direct);
    }
    let dir = cursors_dir();
    let exact = dir.join(name);
    if exact.exists() {
        return Some(exact);
    }
    // bare stem -- try the extensions we know how to load
    for ext in ["png", "cur", "ico", "gif", "jpg", "jpeg"] {
        let p = dir.join(format!("{name}.{ext}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub fn load_cursor_image(path: &Path) -> Option<CursorImage> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if ext == "cur" || ext == "ico" {
        let bytes = std::fs::read(path).ok()?;
        return parse_ico_cur(&bytes);
    }

    match image_loader::load_image(path) {
        Ok(ImageData::Static(img)) => Some(CursorImage {
            rgba: img.pixels,
            w: img.width,
            h: img.height,
            file_hotspot: None,
        }),
        Ok(ImageData::Animated { frames, .. }) => {
            let f = frames.into_iter().next()?;
            Some(CursorImage {
                rgba: f.pixels,
                w: f.width,
                h: f.height,
                file_hotspot: None,
            })
        }
        Err(e) => {
            tracing::warn!(?path, %e, "cursor image load failed");
            None
        }
    }
}

/// Resize to size x size -- cursors are square, matching toolscreen's CopyImage
/// call. Any file hotspot gets scaled by the same factors.
pub fn scale_to(img: CursorImage, size: u32) -> CursorImage {
    let size = size.max(1);
    if img.w == size && img.h == size {
        return img;
    }
    let src = match image::RgbaImage::from_raw(img.w, img.h, img.rgba) {
        Some(s) => s,
        None => return CursorImage { rgba: Vec::new(), w: 0, h: 0, file_hotspot: None },
    };
    // Pick the filter by direction, not size. Downscale -> Lanczos3, since it
    // stays sharp (Triangle blurs, Nearest aliases and drops pixels). Upscale
    // -> Nearest so the pixel edges stay crisp instead of going soft.
    let filter = if size < img.w.max(img.h) {
        image::imageops::FilterType::Lanczos3
    } else {
        image::imageops::FilterType::Nearest
    };
    let out = image::imageops::resize(&src, size, size, filter);
    let hotspot = img.file_hotspot.map(|(hx, hy)| {
        (
            (hx as f32 * size as f32 / img.w as f32).round() as u32,
            (hy as f32 * size as f32 / img.h as f32).round() as u32,
        )
    });
    CursorImage { rgba: out.into_raw(), w: size, h: size, file_hotspot: hotspot }
}

// --- .cur / .ico container ---

fn parse_ico_cur(bytes: &[u8]) -> Option<CursorImage> {
    if bytes.len() < 6 {
        return None;
    }
    let reserved = u16::from_le_bytes([bytes[0], bytes[1]]);
    let kind = u16::from_le_bytes([bytes[2], bytes[3]]);
    let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
    if reserved != 0 || (kind != 1 && kind != 2) || count == 0 {
        return None;
    }

    // pick the largest entry (0 in the byte-sized dims means 256)
    let mut best: Option<(u32, usize)> = None;
    for i in 0..count {
        let off = 6 + i * 16;
        if off + 16 > bytes.len() {
            break;
        }
        let w = if bytes[off] == 0 { 256 } else { bytes[off] as u32 };
        let h = if bytes[off + 1] == 0 { 256 } else { bytes[off + 1] as u32 };
        if best.map_or(true, |(area, _)| w * h > area) {
            best = Some((w * h, off));
        }
    }
    let (_, off) = best?;

    let hx = u16::from_le_bytes([bytes[off + 4], bytes[off + 5]]) as u32;
    let hy = u16::from_le_bytes([bytes[off + 6], bytes[off + 7]]) as u32;
    let size = u32::from_le_bytes(bytes[off + 8..off + 12].try_into().ok()?) as usize;
    let data_off = u32::from_le_bytes(bytes[off + 12..off + 16].try_into().ok()?) as usize;
    let payload = bytes.get(data_off..data_off.checked_add(size)?)?;
    let file_hotspot = (kind == 2).then_some((hx, hy));

    // PNG-compressed entry (common for large modern cursors)
    if payload.starts_with(&[0x89, b'P', b'N', b'G']) {
        let img = image::load_from_memory_with_format(payload, image::ImageFormat::Png)
            .ok()?
            .to_rgba8();
        let (w, h) = img.dimensions();
        return Some(CursorImage { rgba: img.into_raw(), w, h, file_hotspot });
    }

    decode_ico_dib(payload).map(|(rgba, w, h)| CursorImage { rgba, w, h, file_hotspot })
}

// Decode the DIB payload of an .ico/.cur entry: BITMAPINFOHEADER with doubled
// height, color (XOR) bitmap bottom-up, then a 1-bpp AND transparency mask.
fn decode_ico_dib(dib: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    if dib.len() < 40 {
        return None;
    }
    let header_size = u32::from_le_bytes(dib[0..4].try_into().ok()?) as usize;
    let width = i32::from_le_bytes(dib[4..8].try_into().ok()?);
    let height2 = i32::from_le_bytes(dib[8..12].try_into().ok()?);
    let bpp = u16::from_le_bytes(dib[14..16].try_into().ok()?) as usize;
    let compression = u32::from_le_bytes(dib[16..20].try_into().ok()?);
    if compression != 0 || width <= 0 || height2 <= 0 || header_size < 40 {
        return None;
    }
    let w = width as usize;
    let h = (height2 / 2) as usize;
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        return None;
    }

    // palette follows the header for indexed formats
    let palette_len = match bpp {
        1 | 4 | 8 => {
            let colors = u32::from_le_bytes(dib[32..36].try_into().ok()?) as usize;
            if colors == 0 { 1usize << bpp } else { colors }
        }
        _ => 0,
    };
    let palette_off = header_size;
    let xor_off = palette_off + palette_len * 4;
    let xor_stride = (w * bpp + 31) / 32 * 4;
    let and_off = xor_off + xor_stride * h;
    let and_stride = (w + 31) / 32 * 4;
    if dib.len() < and_off + and_stride * h {
        return None;
    }

    let palette = |idx: usize| -> [u8; 3] {
        let p = palette_off + idx * 4;
        if p + 3 <= dib.len() {
            [dib[p + 2], dib[p + 1], dib[p]] // BGRX -> RGB
        } else {
            [0, 0, 0]
        }
    };
    let and_bit = |x: usize, y: usize| -> bool {
        let row = and_off + (h - 1 - y) * and_stride; // bottom-up
        (dib[row + x / 8] >> (7 - x % 8)) & 1 == 1
    };

    let mut rgba = vec![0u8; w * h * 4];
    let mut any_alpha = false;

    for y in 0..h {
        let xor_row = xor_off + (h - 1 - y) * xor_stride; // bottom-up
        for x in 0..w {
            let idx = (y * w + x) * 4;
            let masked = and_bit(x, y);
            match bpp {
                32 => {
                    let p = xor_row + x * 4;
                    rgba[idx] = dib[p + 2];
                    rgba[idx + 1] = dib[p + 1];
                    rgba[idx + 2] = dib[p];
                    rgba[idx + 3] = dib[p + 3];
                    if dib[p + 3] != 0 {
                        any_alpha = true;
                    }
                }
                24 => {
                    let p = xor_row + x * 3;
                    rgba[idx] = dib[p + 2];
                    rgba[idx + 1] = dib[p + 1];
                    rgba[idx + 2] = dib[p];
                    rgba[idx + 3] = if masked { 0 } else { 255 };
                }
                1 => {
                    // toolscreen's monochrome table: AND=1,XOR=0 transparent;
                    // AND=0 -> palette color; AND=1,XOR=1 is "invert screen",
                    // which a system cursor can't do -- approximate as white.
                    let xor = (dib[xor_row + x / 8] >> (7 - x % 8)) & 1 == 1;
                    match (masked, xor) {
                        (true, false) => {} // transparent, already zeroed
                        (true, true) => {
                            rgba[idx..idx + 4].copy_from_slice(&[255, 255, 255, 255]);
                        }
                        (false, xor) => {
                            let c = palette(xor as usize);
                            rgba[idx..idx + 3].copy_from_slice(&c);
                            rgba[idx + 3] = 255;
                        }
                    }
                }
                4 | 8 => {
                    let pi = if bpp == 8 {
                        dib[xor_row + x] as usize
                    } else {
                        let b = dib[xor_row + x / 2];
                        (if x % 2 == 0 { b >> 4 } else { b & 0x0F }) as usize
                    };
                    let c = palette(pi);
                    rgba[idx..idx + 3].copy_from_slice(&c);
                    rgba[idx + 3] = if masked { 0 } else { 255 };
                }
                _ => return None,
            }
        }
    }

    // 32-bpp DIBs with an all-zero alpha channel use the AND mask instead
    if bpp == 32 && !any_alpha {
        for y in 0..h {
            for x in 0..w {
                let idx = (y * w + x) * 4;
                rgba[idx + 3] = if and_bit(x, y) { 0 } else { 255 };
            }
        }
    }

    Some((rgba, w as u32, h as u32))
}

// --- procedural crosshair fallback ---

// Kept as the "always have something" fallback when a configured cursor
// name fails to resolve/load (toolscreen falls back to any cached cursor).
pub fn gen_crosshair(size: u32) -> CursorImage {
    let size = size.max(8);
    let mut px = vec![0u8; (size * size * 4) as usize];
    let center = size / 2;
    let thick = 1u32.max(size / 16);
    let gap = size / 6;
    let outline = 1u32;

    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let dx = (x as i32 - center as i32).unsigned_abs();
            let dy = (y as i32 - center as i32).unsigned_abs();

            let on_h = dy < thick && dx > gap && dx < center;
            let on_v = dx < thick && dy > gap && dy < center;
            let on_dot = dx < thick && dy < thick;

            if on_h || on_v || on_dot {
                px[idx] = 255;
                px[idx + 1] = 255;
                px[idx + 2] = 255;
                px[idx + 3] = 255;
            } else {
                // outline pixels
                let near_h = dy <= thick + outline && dx > gap.saturating_sub(outline) && dx < center + outline;
                let near_v = dx <= thick + outline && dy > gap.saturating_sub(outline) && dy < center + outline;
                let near_dot = dx <= thick + outline && dy <= thick + outline;

                if near_h || near_v || near_dot {
                    px[idx + 3] = 200; // semi-transparent black outline
                }
            }
        }
    }
    CursorImage {
        rgba: px,
        w: size,
        h: size,
        file_hotspot: Some((size / 2, size / 2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Hand-build a .cur: ICONDIR + one entry + 1-bpp DIB exercising all four
    // AND/XOR classes on a 2x2 image.
    fn mono_cur_fixture() -> Vec<u8> {
        let w = 2u32;
        let h = 2u32;
        let mut dib = Vec::new();
        // BITMAPINFOHEADER
        dib.extend_from_slice(&40u32.to_le_bytes()); // biSize
        dib.extend_from_slice(&(w as i32).to_le_bytes());
        dib.extend_from_slice(&((h * 2) as i32).to_le_bytes()); // doubled
        dib.extend_from_slice(&1u16.to_le_bytes()); // planes
        dib.extend_from_slice(&1u16.to_le_bytes()); // bpp
        dib.extend_from_slice(&[0u8; 24]); // compression..importantcolors
        // palette: black, white
        dib.extend_from_slice(&[0, 0, 0, 0, 255, 255, 255, 0]);
        // XOR bitmap, bottom-up, 4-byte stride. layout (top-down):
        //   row0: x0 XOR=0, x1 XOR=1
        //   row1: x0 XOR=0, x1 XOR=1
        dib.extend_from_slice(&[0b0100_0000, 0, 0, 0]); // bottom row (y=1)
        dib.extend_from_slice(&[0b0100_0000, 0, 0, 0]); // top row (y=0)
        // AND bitmap: row0 = 1,1 (masked); row1 = 0,0 (opaque)
        dib.extend_from_slice(&[0b0000_0000, 0, 0, 0]); // bottom row (y=1)
        dib.extend_from_slice(&[0b1100_0000, 0, 0, 0]); // top row (y=0)

        let mut cur = Vec::new();
        cur.extend_from_slice(&0u16.to_le_bytes());
        cur.extend_from_slice(&2u16.to_le_bytes()); // type: cursor
        cur.extend_from_slice(&1u16.to_le_bytes()); // count
        cur.push(w as u8);
        cur.push(h as u8);
        cur.push(0); // colors
        cur.push(0); // reserved
        cur.extend_from_slice(&1u16.to_le_bytes()); // hotspot x
        cur.extend_from_slice(&0u16.to_le_bytes()); // hotspot y
        cur.extend_from_slice(&(dib.len() as u32).to_le_bytes());
        cur.extend_from_slice(&22u32.to_le_bytes()); // data offset
        cur.extend_from_slice(&dib);
        cur
    }

    #[test]
    fn cur_mono_decode_and_hotspot() {
        let img = parse_ico_cur(&mono_cur_fixture()).expect("parse");
        assert_eq!((img.w, img.h), (2, 2));
        assert_eq!(img.file_hotspot, Some((1, 0)));
        let px = |x: usize, y: usize| -> [u8; 4] {
            let i = (y * 2 + x) * 4;
            img.rgba[i..i + 4].try_into().unwrap()
        };
        assert_eq!(px(0, 0), [0, 0, 0, 0], "AND=1 XOR=0 = transparent");
        assert_eq!(px(1, 0), [255, 255, 255, 255], "AND=1 XOR=1 (invert) ~ white");
        assert_eq!(px(0, 1), [0, 0, 0, 255], "AND=0 XOR=0 = black");
        assert_eq!(px(1, 1), [255, 255, 255, 255], "AND=0 XOR=1 = white");
    }

    #[test]
    fn cur_png_payload() {
        // a 4x4 red PNG wrapped in a .cur container
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();

        let mut cur = Vec::new();
        cur.extend_from_slice(&0u16.to_le_bytes());
        cur.extend_from_slice(&2u16.to_le_bytes());
        cur.extend_from_slice(&1u16.to_le_bytes());
        cur.extend_from_slice(&[4, 4, 0, 0]);
        cur.extend_from_slice(&2u16.to_le_bytes()); // hotspot x
        cur.extend_from_slice(&3u16.to_le_bytes()); // hotspot y
        cur.extend_from_slice(&(png.len() as u32).to_le_bytes());
        cur.extend_from_slice(&22u32.to_le_bytes());
        cur.extend_from_slice(&png);

        let img = parse_ico_cur(&cur).expect("parse");
        assert_eq!((img.w, img.h), (4, 4));
        assert_eq!(img.file_hotspot, Some((2, 3)));
        assert_eq!(&img.rgba[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn scale_doubles_hotspot() {
        let img = CursorImage {
            rgba: vec![255; 32 * 32 * 4],
            w: 32,
            h: 32,
            file_hotspot: Some((8, 16)),
        };
        let out = scale_to(img, 64);
        assert_eq!((out.w, out.h), (64, 64));
        assert_eq!(out.file_hotspot, Some((16, 32)));
        assert_eq!(out.rgba.len(), 64 * 64 * 4);
    }

    #[test]
    fn crosshair_has_visible_pixels() {
        let img = gen_crosshair(32);
        assert_eq!((img.w, img.h), (32, 32));
        assert!(img.rgba.chunks(4).any(|p| p[3] == 255));
        assert_eq!(img.file_hotspot, Some((16, 16)));
    }
}
