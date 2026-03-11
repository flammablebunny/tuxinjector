// CPU text rasterizer with per-entry hash caching.
// Uses ab_glyph to render text into RGBA buffers; only re-rasterizes when
// the config hash changes.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use ab_glyph::{Font, FontArc, GlyphId, ScaleFont};
use tuxinjector_config::types::TextOverlayConfig;

struct CachedEntry {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
    hash: u64,
}

pub struct TextOverlayCache {
    entries: HashMap<String, CachedEntry>,
    fonts: HashMap<String, FontArc>,
}

impl TextOverlayCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            fonts: HashMap::new(),
        }
    }

    /// Returns (pixels, width, height) for the given text overlay config.
    /// Only re-rasterizes when something actually changed.
    pub fn get_or_rasterize(
        &mut self,
        id: &str,
        cfg: &TextOverlayConfig,
        theme_font: &str,
    ) -> Option<(&[u8], u32, u32)> {
        if cfg.text.is_empty() {
            return None;
        }

        let hash = cfg_hash(cfg, theme_font);

        let stale = match self.entries.get(id) {
            Some(e) if e.hash == hash => false,
            _ => true,
        };

        if stale {
            let font_path = if cfg.font_path.is_empty() { theme_font } else { &cfg.font_path };
            let font = self.get_font(font_path)?;
            let (px, w, h) = rasterize(&font, cfg);

            self.entries.insert(id.to_owned(), CachedEntry { pixels: px, w, h, hash });
        }

        let e = self.entries.get(id)?;
        Some((&e.pixels, e.w, e.h))
    }

    fn get_font(&mut self, path: &str) -> Option<FontArc> {
        if let Some(f) = self.fonts.get(path) {
            return Some(f.clone());
        }
        let font = load_font_file(path)?;
        self.fonts.insert(path.to_owned(), font.clone());
        Some(font)
    }
}

fn rasterize(font: &FontArc, cfg: &TextOverlayConfig) -> (Vec<u8>, u32, u32) {
    let scale = ab_glyph::PxScale::from(cfg.font_size as f32);
    let scaled = font.as_scaled(scale);

    // measure
    let text_w: f32 = cfg.text.chars()
        .map(|ch| scaled.h_advance(font.glyph_id(ch)))
        .sum();

    let asc = scaled.ascent();
    let desc = scaled.descent();
    let text_h = asc - desc;

    let pad = cfg.padding.max(0) as u32;
    let img_w = (text_w.ceil() as u32 + pad * 2).max(1);
    let img_h = (text_h.ceil() as u32 + pad * 2).max(1);

    let mut pixels = vec![0u8; (img_w * img_h * 4) as usize];

    // background fill
    if cfg.background.enabled {
        let bg = to_rgba8(&cfg.background.color, cfg.background.opacity);
        for px in pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&bg);
        }
    }

    // render glyphs
    let fg = to_rgba8(&cfg.color, 1.0);
    let mut cur_x = pad as f32;
    let baseline = pad as f32 + asc;

    for ch in cfg.text.chars() {
        let gid: GlyphId = font.glyph_id(ch);
        let adv = scaled.h_advance(gid);

        let glyph = gid.with_scale_and_position(
            scaled.scale(),
            ab_glyph::point(cur_x, baseline),
        );

        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();

            outlined.draw(|x, y, cov| {
                let px = bounds.min.x as i32 + x as i32;
                let py = bounds.min.y as i32 + y as i32;
                if px >= 0 && (px as u32) < img_w
                    && py >= 0 && (py as u32) < img_h
                {
                    let idx = ((py as u32 * img_w + px as u32) * 4) as usize;
                    let alpha = (cov * 255.0).round() as u8;
                    blend_px(&mut pixels, idx, fg, alpha);
                }
            });
        }

        cur_x += adv;
    }

    (pixels, img_w, img_h)
}

// Hash all rendering params so we can detect changes cheaply
fn cfg_hash(cfg: &TextOverlayConfig, theme_font: &str) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    cfg.text.hash(&mut h);
    cfg.font_size.hash(&mut h);
    // hash floats as bits to avoid the f32-isn't-Hash problem
    cfg.color.r.to_bits().hash(&mut h);
    cfg.color.g.to_bits().hash(&mut h);
    cfg.color.b.to_bits().hash(&mut h);
    cfg.color.a.to_bits().hash(&mut h);
    cfg.padding.hash(&mut h);
    cfg.background.enabled.hash(&mut h);
    cfg.background.color.r.to_bits().hash(&mut h);
    cfg.background.color.g.to_bits().hash(&mut h);
    cfg.background.color.b.to_bits().hash(&mut h);
    cfg.background.opacity.to_bits().hash(&mut h);
    let effective_font = if cfg.font_path.is_empty() { theme_font } else { &cfg.font_path };
    effective_font.hash(&mut h);
    h.finish()
}

fn blend_px(buf: &mut [u8], idx: usize, color: [u8; 4], alpha: u8) {
    if alpha == 0 || idx + 3 >= buf.len() {
        return;
    }
    let a = alpha as f32 / 255.0;
    let inv = 1.0 - a;
    buf[idx]     = (color[0] as f32 * a + buf[idx]     as f32 * inv).round() as u8;
    buf[idx + 1] = (color[1] as f32 * a + buf[idx + 1] as f32 * inv).round() as u8;
    buf[idx + 2] = (color[2] as f32 * a + buf[idx + 2] as f32 * inv).round() as u8;
    buf[idx + 3] = buf[idx + 3].max(alpha);
}

fn to_rgba8(c: &tuxinjector_core::Color, opacity: f32) -> [u8; 4] {
    [
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        (opacity * 255.0).round() as u8,
    ]
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_owned()
}

fn load_font_file(path: &str) -> Option<FontArc> {
    if path.is_empty() {
        return resolve_fallback_font();
    }
    let full = expand_tilde(path);
    let data = std::fs::read(&full).ok()?;
    FontArc::try_from_vec(data).ok()
}

fn resolve_fallback_font() -> Option<FontArc> {
    crate::overlay_gen::resolve_fallback_font()
}
