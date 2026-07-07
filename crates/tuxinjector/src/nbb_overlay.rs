// Ninjabrain Bot overlay renderer. We rasterize the whole panel into one RGBA
// buffer on the CPU and hand it off as a single SceneElement::Textured, only
// re-drawing when the data/config/size actually change. The goal is to match
// toolscreen's ImDrawList output pixel-for-pixel, as far as ab_glyph vs
// stb_truetype lets us.
//
// Layout/metrics/colors/formatting are a straight port of toolscreen's
// RenderNinjabrainOverlay (render.cpp:10347-12266).

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ab_glyph::{Font, FontArc, ScaleFont};
use tuxinjector_config::types::{NinjabrainOverlayConfig, NinjabrainColumnConfig};
use tuxinjector_core::color::Color;
use tuxinjector_gl_interop::SceneElement;

use crate::nbb_data::{self, NinjabrainData};
use crate::nbb_format as fmt;

const BASE_FONT_SIZE: f32 = 64.0;

static OPEN_SANS: &[u8] = include_bytes!("../../../assets/nbb/OpenSans-Regular.ttf");
static BOAT_GRAY: &[u8] = include_bytes!("../../../assets/nbb/boat_gray.png");
static BOAT_BLUE: &[u8] = include_bytes!("../../../assets/nbb/boat_blue.png");
static BOAT_GREEN: &[u8] = include_bytes!("../../../assets/nbb/boat_green.png");
static BOAT_RED: &[u8] = include_bytes!("../../../assets/nbb/boat_red.png");
static ICON_INFO: &[u8] = include_bytes!("../../../assets/nbb/info_icon.png");
static ICON_WARN: &[u8] = include_bytes!("../../../assets/nbb/warning_icon.png");
static ICON_LOCK: &[u8] = include_bytes!("../../../assets/nbb/lock_icon.png");

// Hotkey toggle at runtime - deliberately separate from the client/config so
// flipping it never writes anything back.
pub static NBB_OVERLAY_VISIBLE: AtomicBool = AtomicBool::new(true);

// --- cache ---

struct Rgba {
    px: Vec<u8>,
    w: u32,
    h: u32,
}

pub struct NbbOverlayCache {
    hash: u64,
    panel: Option<Rgba>,
    font: Option<FontArc>,
    font_path: String,
    icons: Option<[Rgba; 7]>, // gray, blue, green, red, info, warn, lock
}

impl NbbOverlayCache {
    pub fn new() -> Self {
        Self { hash: 0, panel: None, font: None, font_path: String::from("\0unset"), icons: None }
    }

    fn font(&mut self, custom_path: &str) -> Option<FontArc> {
        if self.font_path != custom_path {
            self.font_path = custom_path.to_string();
            self.font = None;
            if !custom_path.is_empty() {
                let p = if let Some(rest) = custom_path.strip_prefix("~/") {
                    std::env::var("HOME").map(|h| format!("{h}/{rest}")).unwrap_or_else(|_| custom_path.to_string())
                } else {
                    custom_path.to_string()
                };
                self.font = std::fs::read(p).ok().and_then(|d| FontArc::try_from_vec(d).ok());
                if self.font.is_none() {
                    tracing::warn!(path = custom_path, "nbb: custom font failed to load, using bundled OpenSans");
                }
            }
            if self.font.is_none() {
                self.font = FontArc::try_from_slice(OPEN_SANS).ok();
            }
        }
        self.font.clone()
    }

    fn icons(&mut self) -> &[Rgba; 7] {
        if self.icons.is_none() {
            let dec = |bytes: &[u8]| -> Rgba {
                match image::load_from_memory_with_format(bytes, image::ImageFormat::Png) {
                    Ok(img) => {
                        let img = img.to_rgba8();
                        let (w, h) = img.dimensions();
                        Rgba { px: img.into_raw(), w, h }
                    }
                    Err(_) => Rgba { px: vec![255; 4], w: 1, h: 1 },
                }
            };
            self.icons = Some([
                dec(BOAT_GRAY), dec(BOAT_BLUE), dec(BOAT_GREEN), dec(BOAT_RED),
                dec(ICON_INFO), dec(ICON_WARN), dec(ICON_LOCK),
            ]);
        }
        self.icons.as_ref().unwrap()
    }
}

// --- canvas primitives ---

struct Canvas {
    px: Vec<u8>,
    w: i32,
    h: i32,
}

impl Canvas {
    fn new(w: u32, h: u32) -> Self {
        Self { px: vec![0u8; (w * h * 4) as usize], w: w as i32, h: h as i32 }
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: [f32; 4]) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || c[3] <= 0.0 {
            return;
        }
        let idx = ((y * self.w + x) * 4) as usize;
        // Straight-alpha source-over that actually respects the dest alpha.
        // The naive `src*a + dst*(1-a)` darkens glyph edges toward black wherever
        // the canvas is still transparent, then the GPU applies the coverage a
        // second time when it blends the panel texture. That double-apply is the
        // "ragged low-quality text" artifact we kept chasing.
        let a = c[3].min(1.0);
        let dst = &mut self.px[idx..idx + 4];
        let dst_a = dst[3] as f32 / 255.0;
        let out_a = a + dst_a * (1.0 - a);
        if out_a <= 0.0 {
            return;
        }
        for ch in 0..3 {
            let src_c = c[ch] * 255.0;
            let dst_c = dst[ch] as f32;
            dst[ch] = ((src_c * a + dst_c * dst_a * (1.0 - a)) / out_a)
                .round()
                .min(255.0) as u8;
        }
        dst[3] = (out_a * 255.0).round().min(255.0) as u8;
    }

    /// Axis-aligned fill with optional rounded corners (all four).
    fn fill_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: [f32; 4], radius: f32) {
        if c[3] <= 0.0 {
            return;
        }
        let r = radius.max(0.0).min((x1 - x0).abs() / 2.0).min((y1 - y0).abs() / 2.0);
        let (ix0, iy0) = (x0.round() as i32, y0.round() as i32);
        let (ix1, iy1) = (x1.round() as i32, y1.round() as i32);
        for y in iy0..iy1 {
            for x in ix0..ix1 {
                if r > 0.5 {
                    let fx = x as f32 + 0.5;
                    let fy = y as f32 + 0.5;
                    let cx = fx.clamp(x0 + r, x1 - r);
                    let cy = fy.clamp(y0 + r, y1 - r);
                    let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
                    if d > r {
                        continue;
                    }
                }
                self.blend(x, y, c);
            }
        }
    }

    fn hline(&mut self, x0: f32, x1: f32, y: f32, c: [f32; 4]) {
        let yy = y.round() as i32;
        for x in (x0.round() as i32)..(x1.round() as i32) {
            self.blend(x, yy, c);
        }
    }

    fn stroke_rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, c: [f32; 4]) {
        let w = width.max(1.0);
        self.fill_rect(x0, y0, x1, y0 + w, c, 0.0);
        self.fill_rect(x0, y1 - w, x1, y1, c, 0.0);
        self.fill_rect(x0, y0 + w, x0 + w, y1 - w, c, 0.0);
        self.fill_rect(x1 - w, y0 + w, x1, y1 - w, c, 0.0);
    }

    /// Bilinear blit of an RGBA image scaled into (x, y, w, h), alpha × opacity.
    fn blit(&mut self, img: &Rgba, x: f32, y: f32, w: f32, h: f32, opacity: f32) {
        if img.w == 0 || img.h == 0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let (ix, iy) = (x.round() as i32, y.round() as i32);
        let (iw, ih) = (w.round() as i32, h.round() as i32);
        for dy in 0..ih {
            for dx in 0..iw {
                let u = (dx as f32 + 0.5) / iw as f32 * img.w as f32 - 0.5;
                let v = (dy as f32 + 0.5) / ih as f32 * img.h as f32 - 0.5;
                let (u0, v0) = (u.floor().max(0.0) as u32, v.floor().max(0.0) as u32);
                let (u1, v1) = ((u0 + 1).min(img.w - 1), (v0 + 1).min(img.h - 1));
                let (fu, fv) = (u - u.floor(), v - v.floor());
                let sample = |px: u32, py: u32, ch: usize| -> f32 {
                    img.px[((py * img.w + px) * 4) as usize + ch] as f32
                };
                let mut out = [0f32; 4];
                for (ch, o) in out.iter_mut().enumerate() {
                    let top = sample(u0, v0, ch) * (1.0 - fu) + sample(u1, v0, ch) * fu;
                    let bot = sample(u0, v1, ch) * (1.0 - fu) + sample(u1, v1, ch) * fu;
                    *o = top * (1.0 - fv) + bot * fv;
                }
                self.blend(
                    ix + dx,
                    iy + dy,
                    [out[0] / 255.0, out[1] / 255.0, out[2] / 255.0, out[3] / 255.0 * opacity],
                );
            }
        }
    }
}

// --- text engine ---

struct TextCtx<'a> {
    font: &'a FontArc,
    aa: bool,
    outline_r: i32,
    outline_color: [f32; 4],
}

fn measure_prefix(font: &FontArc, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(ab_glyph::PxScale::from(size));
    let mut w = 0.0;
    let mut prev: Option<ab_glyph::GlyphId> = None;
    for ch in text.chars() {
        let gid = font.glyph_id(ch);
        if let Some(p) = prev {
            w += scaled.kern(p, gid);
        }
        w += scaled.h_advance(gid);
        prev = Some(gid);
    }
    w
}

// Mirrors toolscreen's getOutlineOffsets: r=1 is just the 8 neighbours,
// bigger radii sample a circle ring.
fn outline_offsets(r: i32) -> Vec<(i32, i32)> {
    if r <= 0 {
        return Vec::new();
    }
    if r == 1 {
        return vec![(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    }
    let mut out = Vec::new();
    let samples = (8 * r).max(8);
    for i in 0..samples {
        let a = i as f32 / samples as f32 * std::f32::consts::TAU;
        let p = ((a.cos() * r as f32).round() as i32, (a.sin() * r as f32).round() as i32);
        if p != (0, 0) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

impl<'a> TextCtx<'a> {
    /// y = top of line (ImGui AddText semantics). Optional horizontal clip.
    fn draw(
        &self,
        canvas: &mut Canvas,
        size: f32,
        x: f32,
        y: f32,
        color: [f32; 4],
        text: &str,
        clip: Option<(f32, f32)>,
    ) {
        if self.outline_r > 0 {
            for (dx, dy) in outline_offsets(self.outline_r) {
                self.draw_plain(canvas, size, x + dx as f32, y + dy as f32, self.outline_color, text, clip);
            }
        }
        self.draw_plain(canvas, size, x, y, color, text, clip);
    }

    fn draw_plain(
        &self,
        canvas: &mut Canvas,
        size: f32,
        x: f32,
        y: f32,
        color: [f32; 4],
        text: &str,
        clip: Option<(f32, f32)>,
    ) {
        if color[3] <= 0.0 {
            return;
        }
        let x = x.round();
        let y = y.round();
        let scaled = self.font.as_scaled(ab_glyph::PxScale::from(size));
        let baseline = y + scaled.ascent();
        let mut cur = x;
        let mut prev: Option<ab_glyph::GlyphId> = None;
        for ch in text.chars() {
            let gid = self.font.glyph_id(ch);
            if let Some(p) = prev {
                cur += scaled.kern(p, gid);
            }
            // fontAntialiasing=false is ImGui's PixelSnapH: glyphs land on
            // integer x positions (crisper stems at small sizes) but keep
            // full anti-aliased coverage. It is NOT a coverage threshold.
            let pen_x = if self.aa { cur } else { cur.round() };
            let glyph = gid.with_scale_and_position(scaled.scale(), ab_glyph::point(pen_x, baseline));
            if let Some(outlined) = self.font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, cov| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if let Some((c0, c1)) = clip {
                        if (px as f32) < c0 || px as f32 >= c1 {
                            return;
                        }
                    }
                    if cov > 0.0 {
                        canvas.blend(px, py, [color[0], color[1], color[2], color[3] * cov]);
                    }
                });
            }
            cur += scaled.h_advance(gid);
            prev = Some(gid);
        }
    }

    /// toolscreen drawCenteredSegmentedText: optional two-color split at a
    /// measured pixel offset, centered on the full string (or on part1 when
    /// center_on_part1), clipped to [left, left+width].
    #[allow(clippy::too_many_arguments)]
    fn draw_cell(
        &self,
        canvas: &mut Canvas,
        size: f32,
        left: f32,
        width: f32,
        y: f32,
        text: &str,
        color: [f32; 4],
        split_offset: f32,
        part1_color: [f32; 4],
        center_on_part1: bool,
        left_align: bool,
        left_inset: f32,
    ) {
        let clip = Some((left, left + width));
        let text_w = measure_prefix(self.font, size, text);
        let anchor_w = if center_on_part1 && split_offset > 0.0 { split_offset } else { text_w };
        let text_x = if left_align { left + left_inset } else { left + (width - anchor_w) * 0.5 };
        if split_offset <= 0.0 {
            self.draw(canvas, size, text_x, y, color, text, clip);
            return;
        }
        // walk chars until measured width >= split_offset
        let mut split_byte = text.len();
        for (i, _) in text.char_indices() {
            let w = measure_prefix(self.font, size, &text[..i]);
            if w >= split_offset {
                split_byte = i;
                break;
            }
        }
        let part1 = &text[..split_byte];
        let rest = &text[split_byte..];
        self.draw(canvas, size, text_x, y, part1_color, part1, clip);
        let part1_w = measure_prefix(self.font, size, part1);
        self.draw(canvas, size, text_x + part1_w, y, color, rest, clip);
    }
}

// --- layout model ---

#[derive(Default, Clone)]
struct Cell {
    text: String,
    color: [f32; 4],
    split_offset: f32,
    part1_color: [f32; 4],
    center_on_part1: bool,
}

struct Column {
    header: String,
    width: f32,
    left_align: bool,
    rows: Vec<Cell>,
}

fn col_a(c: &Color, opacity: f32) -> [f32; 4] {
    [c.r, c.g, c.b, c.a * opacity]
}

/// Should the overlay render at all (toolscreen render.cpp:4783-4830)?
pub fn overlay_should_show(cfg: &NinjabrainOverlayConfig, d: &NinjabrainData) -> bool {
    if !cfg.enabled || !NBB_OVERLAY_VISIBLE.load(Ordering::Relaxed) {
        return false;
    }
    if !cfg.allowed_modes.is_empty() {
        let mode = tuxinjector_lua::get_mode_name();
        if !cfg.allowed_modes.iter().any(|m| m == &mode) {
            return false;
        }
    }
    if cfg.hide_if_stale {
        let delay = Duration::from_secs(cfg.hide_if_stale_delay_seconds.max(1) as u64);
        match d.last_update {
            Some(t) if t.elapsed() < delay => {}
            _ => return false,
        }
    }
    let has_triangulation =
        d.valid_prediction && (d.result_type == "TRIANGULATION" || d.result_type == "DIVINE");
    let show_for_boat = cfg.always_show_boat || d.boat_state == "ERROR";
    has_triangulation
        || d.result_type == "FAILED"
        || (d.blind.enabled && d.blind.has_result)
        || !d.information_messages.is_empty()
        || show_for_boat
        || cfg.always_show
}

/// Build the panel + scene element. Returns None when hidden.
pub fn build_nbb_element(
    cache: &mut NbbOverlayCache,
    cfg_raw: &NinjabrainOverlayConfig,
    screen_w: u32,
    screen_h: u32,
) -> Option<SceneElement> {
    let cfg = cfg_raw.sanitized();
    let data = nbb_data::nbb_snapshot();
    if !overlay_should_show(&cfg, &data) {
        cache.panel = None;
        cache.hash = 0;
        return None;
    }

    let hash = state_hash(&cfg, &data, screen_w, screen_h);
    if cache.panel.is_none() || cache.hash != hash {
        let font = cache.font(&cfg.custom_font_path)?;
        // icons borrow: decode up-front
        cache.icons();
        let panel = rasterize_panel(&cfg, &data, &font, cache.icons.as_ref().unwrap());
        cache.panel = Some(panel);
        cache.hash = hash;
    }

    let panel = cache.panel.as_ref()?;
    let (x, y) = anchor_position(&cfg, panel.w, panel.h, screen_w, screen_h);
    Some(SceneElement::Textured {
        // Integer position + nearest filter: the panel is pre-rasterized at
        // final pixel size, so any fractional placement would bilinear-smear
        // the text (ImGui/toolscreen draws pixel-snapped for the same reason).
        x: x.round(),
        y: y.round(),
        w: panel.w as f32,
        h: panel.h as f32,
        tex_width: panel.w,
        tex_height: panel.h,
        pixels: panel.px.clone(),
        circle_clip: false,
        nearest_filter: true,
        filter_target_colors: Vec::new(),
        filter_output_color: [0.0; 4],
        filter_sensitivity: 0.0,
        filter_color_passthrough: false,
        filter_border_color: [0.0; 4],
        filter_border_width: 0,
        filter_gamma_mode: 0,
        custom_shader: None,
    })
}

fn anchor_position(
    cfg: &NinjabrainOverlayConfig,
    w: u32,
    h: u32,
    sw: u32,
    sh: u32,
) -> (f32, f32) {
    let (w, h, sw, sh) = (w as f32, h as f32, sw as f32, sh as f32);
    let (rx, ry) = (cfg.x as f32, cfg.y as f32);
    match cfg.relative_to.as_str() {
        "topRightScreen" => (sw - w - rx, ry),
        "bottomLeftScreen" => (rx, sh - h - ry),
        "bottomRightScreen" => (sw - w - rx, sh - h - ry),
        "centerScreen" => ((sw - w) / 2.0 + rx, (sh - h) / 2.0 + ry),
        _ => (rx, ry), // topLeftScreen
    }
}

fn state_hash(cfg: &NinjabrainOverlayConfig, d: &NinjabrainData, sw: u32, sh: u32) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    // config: serialize via serde_json for simplicity (rare changes)
    if let Ok(s) = serde_json::to_string(cfg) {
        s.hash(&mut h);
    }
    (sw, sh).hash(&mut h);
    // data fields that affect visuals
    d.result_type.hash(&mut h);
    d.prediction_count.hash(&mut h);
    d.eye_count.hash(&mut h);
    d.valid_prediction.hash(&mut h);
    d.player_in_nether.hash(&mut h);
    d.correction_increments_151.hash(&mut h);
    d.boat_state.hash(&mut h);
    for p in &d.predictions {
        p.chunk_x.hash(&mut h);
        p.chunk_z.hash(&mut h);
        p.certainty.to_bits().hash(&mut h);
        p.overworld_distance.to_bits().hash(&mut h);
    }
    for a in &d.prediction_angles {
        a.needed_correction.to_bits().hash(&mut h);
        a.actual_angle.to_bits().hash(&mut h);
        a.valid.hash(&mut h);
    }
    for t in &d.throws {
        t.angle.to_bits().hash(&mut h);
        t.correction.to_bits().hash(&mut h);
        t.error.to_bits().hash(&mut h);
        t.x_in_overworld.to_bits().hash(&mut h);
        t.has_position.hash(&mut h);
        t.correction_increments.hash(&mut h);
    }
    for m in &d.information_messages {
        m.severity.hash(&mut h);
        m.msg_type.hash(&mut h);
        m.message.hash(&mut h);
    }
    d.blind.enabled.hash(&mut h);
    d.blind.has_result.hash(&mut h);
    d.blind.evaluation.hash(&mut h);
    d.blind.x_in_nether.to_bits().hash(&mut h);
    d.blind.highroll_probability.to_bits().hash(&mut h);
    d.last_angle.to_bits().hash(&mut h);
    d.last_angle_without_correction.to_bits().hash(&mut h);
    h.finish()
}

// --- the panel raster (flow layout) ---

#[allow(clippy::too_many_lines)]
fn rasterize_panel(
    cfg: &NinjabrainOverlayConfig,
    d: &NinjabrainData,
    font: &FontArc,
    icons: &[Rgba; 7],
) -> Rgba {
    let scale = cfg.overlay_scale;
    let fs = (BASE_FONT_SIZE * scale).max(1.0);
    let line_h = fs;
    let op = cfg.overlay_opacity;
    let bg_op = cfg.bg_opacity * op;

    let outline_r = cfg.outline_width;
    let tctx = TextCtx {
        font,
        aa: cfg.font_antialiasing,
        outline_r,
        outline_color: col_a(&cfg.outline_color, op),
    };
    let meas = |s: &str| measure_prefix(font, fs, s);

    // metrics
    let col_gap = cfg.results_column_gap.max(0.0) * scale;
    let content_pad_x = (8.0 + outline_r as f32) * scale;
    let side_pad_x = cfg.side_padding.max(0.0) * scale;
    let row_h = line_h + cfg.row_spacing * scale;
    let results_text_offset_y = cfg.row_spacing * scale * 0.5;
    let results_last_row_h = line_h + results_text_offset_y;
    let header_band_h = line_h + cfg.results_header_padding_y * scale * 2.0;
    let cell_pad_x = 8.0 * scale;
    let left_inset = outline_r as f32;
    let detail_row_h = line_h + cfg.throws_row_padding_y * scale * 2.0;
    let throws_header_band_h = line_h + cfg.throws_header_padding_y * scale * 2.0;

    // theme colors (opacity applied)
    let text_col = col_a(&cfg.text_color, op);
    let body_base = if d.result_type == "DIVINE" { &cfg.divine_text_color } else { &cfg.data_color };
    let data_col = col_a(body_base, op);
    let throws_text = col_a(&cfg.throws_text_color, op);
    let grad = |p: f64| -> [f32; 4] {
        let mut c = fmt::gradient_color(p, &cfg.certainty_low_color, &cfg.certainty_mid_color, &cfg.certainty_color);
        c[3] *= op;
        c
    };
    let coord_pos = col_a(&cfg.coord_positive_color, op);
    let coord_neg = col_a(&cfg.coord_negative_color, op);
    let subpix_pos = col_a(&cfg.subpixel_positive_color, op);
    let subpix_neg = col_a(&cfg.subpixel_negative_color, op);

    let has_triangulation =
        d.valid_prediction && (d.result_type == "TRIANGULATION" || d.result_type == "DIVINE");
    let failed = d.result_type == "FAILED";
    let blind = !has_triangulation && !failed && d.blind.enabled && d.blind.has_result;
    let show_top_table = has_triangulation || (!failed && !blind);

    // --- build result columns ---
    let base_rows = {
        let want = cfg.shown_predictions.clamp(1, 5);
        let have = d.prediction_count.min(want);
        if cfg.always_show { want.max(have) } else { have }
    } as usize;
    let data_rows = (d.prediction_count as usize).min(base_rows);

    let mut columns: Vec<Column> = Vec::new();
    for cc in cfg.columns.iter().filter(|c| c.show) {
        let mut col = build_column(cc, cfg, d, data_rows, &grad, data_col, text_col, coord_pos, coord_neg, subpix_pos, subpix_neg, &meas);
        // width: header vs rows vs static samples
        let mut w = meas(&col.header);
        for r in &col.rows {
            w = w.max(meas(&r.text));
        }
        if cfg.static_column_widths {
            let sample: &[&str] = match cc.id.as_str() {
                "coords" => {
                    if cfg.coords_display == "chunk" { &["(-999, -999)"] } else { &["(-99999, -99999)"] }
                }
                "certainty" => &["100.00%"],
                "distance" => &["999999"],
                "nether" => &["(-99999, -99999)"],
                "angle" => {
                    if cfg.show_direction_to_stronghold {
                        if cfg.show_throw_details { &["-359.99 (-> 359.0)"] } else { &["-359.99 (-> 359.0)", "-359.99+999"] }
                    } else if cfg.show_throw_details {
                        &["-359.99"]
                    } else {
                        &["-359.99", "-359.99+999"]
                    }
                }
                _ => &[],
            };
            for s in sample {
                w = w.max(meas(s));
            }
        }
        if cc.static_width > 0 {
            w = cc.static_width as f32 * scale;
        }
        col.width = w;
        columns.push(col);
    }
    let max_rows = columns.iter().map(|c| c.rows.len()).max().unwrap_or(0).max(if show_top_table { base_rows.max(1) } else { 0 });

    let n_cols = columns.len();
    let results_table_w: f32 = columns.iter().map(|c| c.width).sum::<f32>()
        + col_gap * (n_cols.saturating_sub(1)) as f32;
    let results_w = results_table_w + (cfg.results_margin_left + cfg.results_margin_right) * scale;
    let results_body_h = if max_rows > 0 {
        row_h * (max_rows as f32 - 1.0) + results_last_row_h
    } else {
        0.0
    };

    // --- throws table ---
    let throws_visible = cfg.show_throw_details && (show_top_table || failed || blind);
    let info_fs = fs * cfg.information_messages_font_scale;
    let (throws_cols, throws_rows_n, throws_w) = if throws_visible {
        let headers = ["x", "z", "Angle", "Error"];
        let samples = ["-999999.99", "-999999.99", "359.99+999", "-180.0000"];
        let mut ws = [0f32; 4];
        for i in 0..4 {
            ws[i] = meas(headers[i]).max(meas(samples[i])) + cell_pad_x * 2.0;
        }
        let rows = (d.eye_count.max(cfg.eye_throw_rows)) as usize;
        let total: f32 = ws.iter().sum();
        (Some((headers, ws)), rows, total)
    } else {
        (None, 0, 0.0)
    };

    // --- info messages (formatted + wrapped later, width first) ---
    let info_pref_w = cfg.information_messages_min_width.max(120.0) * scale;
    let has_info = !d.information_messages.is_empty();

    // --- failed / blind lines ---
    let failed_lines: Vec<Vec<fmt::TextRun>> = if failed {
        vec![
            vec![fmt::TextRun { text: "Could not determine the stronghold chunk.".into(), color: None }],
            vec![fmt::TextRun { text: "You probably misread one of the eyes.".into(), color: None }],
        ]
    } else {
        Vec::new()
    };

    // content width
    let mut content_w: f32 = 0.0;
    if show_top_table {
        content_w = content_w.max(results_w);
    }
    if throws_visible {
        content_w = content_w.max(throws_w + (cfg.throws_margin_left + cfg.throws_margin_right) * scale);
    }
    if has_info {
        content_w = content_w.max(
            info_pref_w + (cfg.information_messages_margin_left + cfg.information_messages_margin_right) * scale,
        );
    }
    let (blind_lines, blind_colors) = build_blind_lines(cfg, d, &grad, op);
    if failed {
        for l in &failed_lines {
            content_w = content_w.max(meas(&l[0].text) + (cfg.failure_margin_left + cfg.failure_margin_right) * scale);
        }
    }
    if blind {
        for l in &blind_lines {
            let w: f32 = l.iter().map(|r| meas(&r.text)).sum();
            content_w = content_w.max(w + (cfg.blind_margin_left + cfg.blind_margin_right) * scale);
        }
    }
    content_w = content_w.max(line_h); // never zero

    let panel_w = (content_pad_x + side_pad_x) * 2.0 + content_w;
    let content_area_x = content_pad_x + side_pad_x;

    // wrap info messages now that width is known
    let icon_size = info_fs * 0.82 * cfg.information_messages_icon_scale;
    let info_line_gap = 2.0 * scale * cfg.information_messages_font_scale;
    // Wrap at the full available content width — minWidth only guarantees a
    // minimum panel width (it feeds content_w above); wrapping at it made
    // messages fold after a couple of words whenever the results table was
    // wider than the configured minimum.
    let wrap_w = (content_w
        - (cfg.information_messages_margin_left + cfg.information_messages_margin_right) * scale
        - icon_size
        - cfg.information_messages_icon_text_margin * scale)
        .max(40.0);
    let info_msgs: Vec<(usize, Vec<Vec<(String, [f32; 4])>>)> = d
        .information_messages
        .iter()
        .map(|m| {
            let icon = if m.msg_type.to_lowercase().contains("lock") {
                6
            } else if m.severity == "WARNING" || m.severity == "ERROR" {
                5
            } else {
                4
            };
            let runs = fmt::format_info_message(m);
            let lines = wrap_runs(font, info_fs, &runs, wrap_w, col_a(&cfg.data_color, op), op);
            (icon, lines)
        })
        .collect();
    let info_block_h = |msgs: &[(usize, Vec<Vec<(String, [f32; 4])>>)]| -> f32 {
        let mut h = cfg.information_messages_margin_top * scale;
        for (i, (_icon, lines)) in msgs.iter().enumerate() {
            let text_h = lines.len() as f32 * (info_fs + info_line_gap);
            h += text_h.max(icon_size);
            if i + 1 < msgs.len() {
                h += info_line_gap + 1.0;
            }
        }
        h + cfg.information_messages_margin_bottom * scale
    };

    // total height (flow)
    let mut total_h = cfg.content_padding_top * scale;
    let placement = cfg.information_messages_placement.as_str();
    if has_info && placement == "top" {
        total_h += info_block_h(&info_msgs);
    }
    if show_top_table {
        total_h += cfg.results_margin_top * scale + header_band_h + results_body_h + cfg.results_margin_bottom * scale;
    }
    if failed {
        total_h += cfg.failure_margin_top * scale
            + failed_lines.len() as f32 * line_h
            + cfg.failure_line_gap * scale * (failed_lines.len().saturating_sub(1)) as f32
            + cfg.failure_margin_bottom * scale;
    }
    if blind {
        total_h += cfg.blind_margin_top * scale
            + blind_lines.len() as f32 * line_h
            + cfg.blind_line_gap * scale * (blind_lines.len().saturating_sub(1)) as f32
            + cfg.blind_margin_bottom * scale;
    }
    if has_info && placement == "middle" {
        total_h += info_block_h(&info_msgs);
    }
    if throws_visible {
        total_h += cfg.throws_margin_top * scale
            + throws_header_band_h
            + detail_row_h * throws_rows_n as f32
            + cfg.throws_margin_bottom * scale;
    }
    if has_info && placement == "bottom" {
        total_h += info_block_h(&info_msgs);
    }
    total_h += cfg.content_padding_bottom * scale;
    total_h = total_h.max(1.0);

    // --- raster ---
    let mut canvas = Canvas::new(panel_w.ceil() as u32, total_h.ceil() as u32);
    let (surface_left, surface_right) = (0.0, panel_w);

    if cfg.bg_enabled {
        canvas.fill_rect(0.0, 0.0, panel_w, total_h, with_a(col_a(&cfg.bg_color, 1.0), bg_op), cfg.corner_radius * scale);
    }

    let mut y = cfg.content_padding_top * scale;
    let header_fill = with_a(col_a(&cfg.header_fill_color, 1.0), bg_op);
    let divider = with_a(col_a(&cfg.divider_color, 1.0), bg_op);
    let header_divider = with_a(col_a(&cfg.header_divider_color, 1.0), bg_op);

    let mut draw_info_block = |canvas: &mut Canvas, y: &mut f32| {
        if !has_info {
            return;
        }
        *y += cfg.information_messages_margin_top * scale;
        let left = content_area_x + cfg.information_messages_margin_left * scale;
        for (i, (icon, lines)) in info_msgs.iter().enumerate() {
            let text_h = lines.len() as f32 * (info_fs + info_line_gap);
            let row_h = text_h.max(icon_size);
            canvas.blit(&icons[*icon], left, *y + (row_h - icon_size) / 2.0, icon_size, icon_size, op);
            let tx = left + icon_size + cfg.information_messages_icon_text_margin * scale;
            let mut ty = *y + (row_h - text_h) / 2.0;
            for line in lines {
                let mut lx = tx;
                for (text, color) in line {
                    tctx.draw(canvas, info_fs, lx, ty, *color, text, None);
                    lx += measure_prefix(font, info_fs, text);
                }
                ty += info_fs + info_line_gap;
            }
            *y += row_h;
            if i + 1 < info_msgs.len() {
                *y += info_line_gap;
                canvas.hline(surface_left, surface_right, *y, header_divider);
                *y += 1.0;
            }
        }
        *y += cfg.information_messages_margin_bottom * scale;
    };

    if has_info && placement == "top" {
        draw_info_block(&mut canvas, &mut y);
    }

    if show_top_table {
        y += cfg.results_margin_top * scale;
        let band_top = y;
        let header_y = band_top + (header_band_h - line_h) / 2.0;
        let sep_y = band_top + header_band_h;
        let round_top = band_top <= 0.5;
        canvas.fill_rect(
            surface_left, band_top, surface_right, sep_y, header_fill,
            if round_top { cfg.corner_radius * scale } else { 0.0 },
        );
        let table_x = content_area_x + cfg.results_margin_left * scale;
        let mut cx = table_x;
        for (ci, col) in columns.iter().enumerate() {
            tctx.draw_cell(&mut canvas, fs, cx, col.width, header_y, &col.header, text_col, 0.0, text_col, false, false, left_inset);
            cx += col.width + if ci + 1 < n_cols { col_gap } else { 0.0 };
        }
        // boat icon in header band
        let show_for_boat = cfg.always_show_boat || d.boat_state != "NONE";
        if cfg.show_boat_state_in_top_bar && show_for_boat {
            let icon_sz = (cfg.boat_state_size * scale).min(header_band_h);
            let icon_x = surface_right
                - cfg.results_margin_right * scale
                - cfg.boat_state_margin_right * scale
                - icon_sz;
            let icon = match d.boat_state.as_str() {
                "MEASURING" => 1,
                "VALID" => 2,
                "ERROR" => 3,
                _ => 0,
            };
            canvas.blit(&icons[icon], icon_x, band_top + (header_band_h - icon_sz) / 2.0, icon_sz, icon_sz, op);
        }
        canvas.hline(surface_left, surface_right, sep_y, header_divider);

        for ri in 0..max_rows {
            let row_top = sep_y + row_h * ri as f32;
            let is_last = ri + 1 == max_rows;
            let row_bottom = row_top + if is_last { results_last_row_h } else { row_h };
            let row_text_y = row_top + results_text_offset_y;
            if !is_last {
                canvas.hline(surface_left, surface_right, row_bottom, divider);
            }
            let mut rx = table_x;
            for (ci, col) in columns.iter().enumerate() {
                if let Some(cell) = col.rows.get(ri) {
                    tctx.draw_cell(
                        &mut canvas, fs, rx, col.width, row_text_y, &cell.text, cell.color,
                        cell.split_offset, cell.part1_color, cell.center_on_part1,
                        col.left_align, left_inset,
                    );
                }
                rx += col.width + if ci + 1 < n_cols { col_gap } else { 0.0 };
            }
        }
        y = sep_y + results_body_h + cfg.results_margin_bottom * scale;
    }

    if failed {
        y += cfg.failure_margin_top * scale;
        let left = content_area_x + cfg.failure_margin_left * scale;
        for (i, l) in failed_lines.iter().enumerate() {
            tctx.draw(&mut canvas, fs, left, y, data_col, &l[0].text, None);
            y += line_h;
            if i + 1 < failed_lines.len() {
                y += cfg.failure_line_gap * scale;
            }
        }
        y += cfg.failure_margin_bottom * scale;
    }

    if blind {
        y += cfg.blind_margin_top * scale;
        let left = content_area_x + cfg.blind_margin_left * scale;
        for (i, line) in blind_lines.iter().enumerate() {
            let mut lx = left;
            for (ri, run) in line.iter().enumerate() {
                let color = blind_colors
                    .get(i)
                    .and_then(|m| m.get(&ri).copied())
                    .unwrap_or(data_col);
                tctx.draw(&mut canvas, fs, lx, y, color, &run.text, None);
                lx += meas(&run.text);
            }
            y += line_h;
            if i + 1 < blind_lines.len() {
                y += cfg.blind_line_gap * scale;
            }
        }
        y += cfg.blind_margin_bottom * scale;
    }

    if has_info && placement == "middle" {
        draw_info_block(&mut canvas, &mut y);
    }

    if let Some((headers, ws)) = throws_cols {
        y += cfg.throws_margin_top * scale;
        let band_top = y;
        let band_bottom = band_top + throws_header_band_h;
        canvas.fill_rect(surface_left, band_top, surface_right, band_bottom, header_fill, 0.0);
        let sec_left = content_area_x + cfg.throws_margin_left * scale;
        tctx.draw(
            &mut canvas, fs, sec_left + left_inset,
            band_top + (throws_header_band_h - line_h) / 2.0,
            data_col, "Ender eye throws", None,
        );
        y = band_bottom;
        // column header row
        let hdr_top = y;
        canvas.fill_rect(surface_left, hdr_top, surface_right, hdr_top + detail_row_h, with_a(col_a(&cfg.throws_background_color, 1.0), bg_op), 0.0);
        let mut hx = sec_left;
        for (i, hname) in headers.iter().enumerate() {
            tctx.draw_cell(&mut canvas, fs, hx, ws[i], hdr_top + (detail_row_h - line_h) / 2.0, hname, text_col, 0.0, text_col, false, false, left_inset);
            hx += ws[i];
        }
        y += detail_row_h;
        for ri in 0..throws_rows_n {
            let row_top = y;
            canvas.fill_rect(surface_left, row_top, surface_right, row_top + detail_row_h, with_a(col_a(&cfg.throws_background_color, 1.0), bg_op), 0.0);
            let text_y = row_top + (detail_row_h - line_h) / 2.0;
            let cells = throw_row_cells(d, ri, throws_text, text_col, subpix_pos, subpix_neg, &meas);
            let mut rx = sec_left;
            for (i, cell) in cells.iter().enumerate() {
                tctx.draw_cell(
                    &mut canvas, fs, rx, ws[i], text_y, &cell.text, cell.color,
                    cell.split_offset, cell.part1_color, false, false, left_inset,
                );
                rx += ws[i];
            }
            y += detail_row_h;
        }
        y += cfg.throws_margin_bottom * scale;
    }

    if has_info && placement == "bottom" {
        draw_info_block(&mut canvas, &mut y);
    }

    if cfg.border_width > 0 {
        canvas.stroke_rect(
            0.0, 0.0, panel_w, total_h,
            cfg.border_width as f32,
            col_a(&cfg.border_color, op),
        );
    }
    let _ = y;

    Rgba { px: canvas.px, w: canvas.w as u32, h: canvas.h as u32 }
}

fn with_a(mut c: [f32; 4], mul: f32) -> [f32; 4] {
    c[3] *= mul;
    c
}

#[allow(clippy::too_many_arguments)]
fn build_column(
    cc: &NinjabrainColumnConfig,
    cfg: &NinjabrainOverlayConfig,
    d: &NinjabrainData,
    data_rows: usize,
    grad: &dyn Fn(f64) -> [f32; 4],
    data_col: [f32; 4],
    text_col: [f32; 4],
    coord_pos: [f32; 4],
    coord_neg: [f32; 4],
    subpix_pos: [f32; 4],
    subpix_neg: [f32; 4],
    meas: &dyn Fn(&str) -> f32,
) -> Column {
    let mut rows: Vec<Cell> = Vec::new();
    let has_triangulation =
        d.valid_prediction && (d.result_type == "TRIANGULATION" || d.result_type == "DIVINE");

    for ri in 0..data_rows {
        let p = &d.predictions[ri];
        let ang = d.prediction_angles.get(ri);
        let cell = match cc.id.as_str() {
            "coords" => {
                let (x, z) = if cfg.coords_display == "chunk" {
                    (p.chunk_x as i64, p.chunk_z as i64)
                } else {
                    (p.chunk_x as i64 * 16 + 4, p.chunk_z as i64 * 16 + 4)
                };
                let (text, color, split, p1) = fmt::format_coords(x, z, coord_pos, coord_neg);
                let split_offset = if split.is_empty() { 0.0 } else { meas(&split) };
                Cell { text, color, split_offset, part1_color: p1, center_on_part1: false }
            }
            "certainty" => Cell {
                text: format!("{:.1}%", p.certainty * 100.0),
                color: grad(p.certainty),
                ..Cell::default()
            },
            "distance" => Cell {
                text: format!("{}", d.display_distance(p).floor() as i64),
                color: data_col,
                ..Cell::default()
            },
            "nether" => {
                let (x, z) = (p.chunk_x as i64 * 2, p.chunk_z as i64 * 2);
                let (text, color, split, p1) = fmt::format_coords(x, z, coord_pos, coord_neg);
                let split_offset = if split.is_empty() { 0.0 } else { meas(&split) };
                Cell { text, color, split_offset, part1_color: p1, center_on_part1: false }
            }
            "angle" => {
                if let Some(a) = ang.filter(|a| a.valid) {
                    if cfg.show_direction_to_stronghold {
                        let (text, base) = fmt::format_angle_with_direction(a.actual_angle, a.needed_correction);
                        let suffix_col = grad(1.0 - a.needed_correction.abs() / 180.0);
                        Cell {
                            split_offset: meas(&base),
                            text,
                            color: suffix_col,
                            part1_color: data_col,
                            center_on_part1: true,
                        }
                    } else {
                        Cell { text: format!("{:.2}", a.actual_angle), color: data_col, ..Cell::default() }
                    }
                } else if has_triangulation {
                    Cell { text: format!("{:.2}", d.last_angle), color: data_col, ..Cell::default() }
                } else {
                    Cell { text: "-".to_string(), color: data_col, ..Cell::default() }
                }
            }
            _ => Cell::default(),
        };
        rows.push(cell);
    }

    if cc.id == "angle" {
        if data_rows == 0 && d.eye_count == 0 {
            rows.push(Cell { text: "-".to_string(), color: data_col, ..Cell::default() });
        }
        // extra subpixel row when throws table is hidden
        if !cfg.show_throw_details && d.eye_count > 0 {
            let last = &d.throws[d.eye_count as usize - 1];
            let increments = if last.has_correction_increments {
                last.correction_increments
            } else {
                d.correction_increments_151
            };
            let text = fmt::format_subpixel(d.last_angle_without_correction, increments);
            if increments == 0 {
                rows.push(Cell { text, color: text_col, ..Cell::default() });
            } else {
                let base = format!("{:.2}", d.last_angle_without_correction);
                let color = if increments > 0 { subpix_pos } else { subpix_neg };
                rows.push(Cell {
                    split_offset: meas(&base),
                    text,
                    color,
                    part1_color: text_col,
                    center_on_part1: false,
                });
            }
        }
    }

    Column { header: cc.header.clone(), width: 0.0, left_align: cc.id == "angle", rows }
}

fn throw_row_cells(
    d: &NinjabrainData,
    ri: usize,
    throws_text: [f32; 4],
    text_col: [f32; 4],
    subpix_pos: [f32; 4],
    subpix_neg: [f32; 4],
    meas: &dyn Fn(&str) -> f32,
) -> [Cell; 4] {
    let blank = || Cell { text: String::new(), color: throws_text, ..Cell::default() };
    let Some(t) = d.throws.get(ri).filter(|_| ri < d.eye_count as usize) else {
        return [blank(), blank(), blank(), blank()];
    };
    let coord = |v: f64, has: bool| -> Cell {
        Cell {
            text: if has { format!("{v:.2}") } else { "-".to_string() },
            color: throws_text,
            ..Cell::default()
        }
    };
    let increments = if t.has_correction_increments {
        t.correction_increments
    } else if ri + 1 == d.eye_count as usize {
        d.correction_increments_151
    } else {
        0
    };
    let angle_cell = if increments == 0 {
        Cell { text: format!("{:.2}", t.angle_without_correction), color: throws_text, ..Cell::default() }
    } else {
        let base = format!("{:.2}", t.angle_without_correction);
        Cell {
            split_offset: meas(&base),
            text: fmt::format_subpixel(t.angle_without_correction, increments),
            color: if increments > 0 { subpix_pos } else { subpix_neg },
            part1_color: text_col,
            center_on_part1: false,
        }
    };
    let error_cell = Cell {
        text: if t.error.abs() > 1e-9 { format!("{:.4}", t.error) } else { "-".to_string() },
        color: throws_text,
        ..Cell::default()
    };
    [
        coord(t.x_in_overworld, t.has_position),
        coord(t.z_in_overworld, t.has_position),
        angle_cell,
        error_cell,
    ]
}

type BlindColorMap = std::collections::HashMap<usize, [f32; 4]>;

fn build_blind_lines(
    cfg: &NinjabrainOverlayConfig,
    d: &NinjabrainData,
    grad: &dyn Fn(f64) -> [f32; 4],
    op: f32,
) -> (Vec<Vec<fmt::TextRun>>, Vec<BlindColorMap>) {
    if !(d.blind.enabled && d.blind.has_result) {
        return (Vec::new(), Vec::new());
    }
    let _ = cfg;
    let (bucket, words) = fmt::blind_evaluation(&d.blind.evaluation);
    let run = |s: String| fmt::TextRun { text: s, color: None };
    let mut lines = Vec::new();
    let mut colors: Vec<BlindColorMap> = Vec::new();

    let _ = op;
    let mut l1_colors = BlindColorMap::new();
    l1_colors.insert(1, grad(bucket)); // grad already applies opacity
    lines.push(vec![
        run(format!(
            "Blind coords ({}, {}) are ",
            d.blind.x_in_nether.round() as i64,
            d.blind.z_in_nether.round() as i64
        )),
        run(words.to_string()),
    ]);
    colors.push(l1_colors);

    let pct = d.blind.highroll_probability * 100.0;
    let mut l2_colors = BlindColorMap::new();
    l2_colors.insert(0, grad((pct / 10.0).clamp(0.0, 1.0)));
    lines.push(vec![
        run(format!("{pct:.1}%")),
        run(format!(
            " chance of <{} block blind",
            d.blind.highroll_threshold.round() as i64
        )),
    ]);
    colors.push(l2_colors);

    if d.blind.evaluation != "EXCELLENT" {
        lines.push(vec![run(format!(
            "Head {}\u{00B0}, {} blocks away, for better coords",
            d.blind.improve_direction.round() as i64,
            d.blind.improve_distance.round() as i64
        ))]);
        colors.push(BlindColorMap::new());
    }
    (lines, colors)
}

/// Word-wrap colored runs to a max pixel width.
fn wrap_runs(
    font: &FontArc,
    size: f32,
    runs: &[fmt::TextRun],
    max_w: f32,
    default_color: [f32; 4],
    op: f32,
) -> Vec<Vec<(String, [f32; 4])>> {
    let mut lines: Vec<Vec<(String, [f32; 4])>> = vec![Vec::new()];
    let mut cur_w = 0.0;
    for run in runs {
        let color = run
            .color
            .map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, op])
            .unwrap_or(default_color);
        for word in run.text.split_inclusive(' ') {
            let w = measure_prefix(font, size, word);
            if cur_w + w > max_w && cur_w > 0.0 {
                lines.push(Vec::new());
                cur_w = 0.0;
            }
            let line = lines.last_mut().unwrap();
            match line.last_mut() {
                Some((t, c)) if *c == color => t.push_str(word),
                _ => line.push((word.to_string(), color)),
            }
            cur_w += w;
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_ring() {
        assert_eq!(outline_offsets(0).len(), 0);
        assert_eq!(outline_offsets(1).len(), 8);
        let r2 = outline_offsets(2);
        assert!(r2.len() >= 8 && !r2.contains(&(0, 0)));
    }

    #[test]
    fn anchors() {
        let mut cfg = NinjabrainOverlayConfig::default(); // bottomLeftScreen, x=4, y=-5
        let (x, y) = anchor_position(&cfg, 100, 50, 1920, 1080);
        assert_eq!((x, y), (4.0, 1080.0 - 50.0 + 5.0));
        cfg.relative_to = "topRightScreen".into();
        cfg.x = 10;
        cfg.y = 20;
        assert_eq!(anchor_position(&cfg, 100, 50, 1920, 1080), (1810.0, 20.0));
        cfg.relative_to = "centerScreen".into();
        assert_eq!(anchor_position(&cfg, 100, 50, 1920, 1080), (920.0, 535.0));
    }

    #[test]
    fn wrap_and_measure_consistency() {
        let font = FontArc::try_from_slice(OPEN_SANS).unwrap();
        let runs = vec![fmt::TextRun { text: "one two three four five six".into(), color: None }];
        let lines = wrap_runs(&font, 20.0, &runs, 60.0, [1.0; 4], 1.0);
        assert!(lines.len() > 1, "narrow width must wrap");
        for line in &lines {
            let w: f32 = line.iter().map(|(t, _)| measure_prefix(&font, 20.0, t)).sum();
            assert!(w <= 120.0, "line too wide: {w}");
        }
    }

    #[test]
    fn panel_rasterizes_with_data() {
        let mut cfg = NinjabrainOverlayConfig::default();
        cfg.enabled = true;
        cfg.always_show = true;
        cfg.show_boat_state_in_top_bar = true;
        let mut d = NinjabrainData::default();
        d.result_type = "TRIANGULATION".into();
        d.valid_prediction = true;
        d.prediction_count = 2;
        d.predictions = vec![
            crate::nbb_data::NbbPrediction { chunk_x: 10, chunk_z: -20, certainty: 0.93, overworld_distance: 500.0 },
            crate::nbb_data::NbbPrediction { chunk_x: -11, chunk_z: 21, certainty: 0.07, overworld_distance: 700.0 },
        ];
        d.prediction_angles = vec![
            crate::nbb_data::NbbPredictionAngle { actual_angle: -42.5, needed_correction: 3.0, valid: true },
            crate::nbb_data::NbbPredictionAngle::default(),
        ];
        d.eye_count = 1;
        d.throws = vec![crate::nbb_data::NbbThrow {
            x_in_overworld: 12.3, z_in_overworld: -45.6, has_position: true,
            angle: -42.0, angle_without_correction: -42.0, correction: 0.0, error: 0.002,
            ..Default::default()
        }];
        d.information_messages = vec![crate::nbb_data::NbbInfoMessage {
            severity: "WARNING".into(), msg_type: "MISMEASURE".into(), message: "x".into(),
        }];
        d.boat_state = "VALID".into();
        let font = FontArc::try_from_slice(OPEN_SANS).unwrap();
        let icons_placeholder: [Rgba; 7] = std::array::from_fn(|_| Rgba { px: vec![255; 16], w: 2, h: 2 });
        let panel = rasterize_panel(&cfg.sanitized(), &d, &font, &icons_placeholder);
        assert!(panel.w > 100 && panel.h > 50, "panel {}x{}", panel.w, panel.h);
        assert!(panel.px.iter().skip(3).step_by(4).any(|&a| a > 0), "panel has visible pixels");
    }

    #[test]
    fn failed_and_blind_branches() {
        let cfg = NinjabrainOverlayConfig { enabled: true, ..Default::default() }.sanitized();
        let font = FontArc::try_from_slice(OPEN_SANS).unwrap();
        let icons: [Rgba; 7] = std::array::from_fn(|_| Rgba { px: vec![255; 4], w: 1, h: 1 });
        let mut d = NinjabrainData::default();
        d.result_type = "FAILED".into();
        let p = rasterize_panel(&cfg, &d, &font, &icons);
        assert!(p.w > 0 && p.h > 0);
        let mut d = NinjabrainData::default();
        d.blind.enabled = true;
        d.blind.has_result = true;
        d.blind.evaluation = "HIGHROLL_GOOD".into();
        d.blind.highroll_probability = 0.22;
        let p = rasterize_panel(&cfg, &d, &font, &icons);
        assert!(p.w > 0 && p.h > 0);
    }

    #[test]
    fn visibility_gating() {
        let mut cfg = NinjabrainOverlayConfig::default();
        cfg.enabled = true;
        let d = NinjabrainData::default();
        assert!(!overlay_should_show(&cfg, &d), "empty data hides");
        cfg.always_show = true;
        assert!(overlay_should_show(&cfg, &d));
        cfg.always_show = false;
        let mut d2 = NinjabrainData::default();
        d2.result_type = "FAILED".into();
        assert!(overlay_should_show(&cfg, &d2));
        cfg.enabled = false;
        assert!(!overlay_should_show(&cfg, &d2));
    }
}

#[cfg(test)]
mod dump_test {
    use super::*;

    // diagnostic: dump a rendered panel to PNG for visual inspection
    #[test]
    fn dump_panel_png() {
        let dir = std::env::var("NBB_DUMP_DIR").unwrap_or_default();
        if dir.is_empty() { return; }
        let mut cfg = NinjabrainOverlayConfig::default();
        cfg.enabled = true;
        cfg.always_show = true;
        cfg.show_boat_state_in_top_bar = true;
        cfg.overlay_scale = 0.30;
        cfg.font_antialiasing = false;
        let mut d = NinjabrainData::default();
        d.result_type = "TRIANGULATION".into();
        d.valid_prediction = true;
        d.prediction_count = 3;
        d.predictions = vec![
            crate::nbb_data::NbbPrediction { chunk_x: 10, chunk_z: -20, certainty: 0.93, overworld_distance: 500.0 },
            crate::nbb_data::NbbPrediction { chunk_x: -11, chunk_z: 21, certainty: 0.05, overworld_distance: 700.0 },
            crate::nbb_data::NbbPrediction { chunk_x: 12, chunk_z: 22, certainty: 0.02, overworld_distance: 900.0 },
        ];
        d.prediction_angles = vec![
            crate::nbb_data::NbbPredictionAngle { actual_angle: -42.51, needed_correction: 3.2, valid: true },
            crate::nbb_data::NbbPredictionAngle { actual_angle: 12.0, needed_correction: -100.0, valid: true },
            crate::nbb_data::NbbPredictionAngle::default(),
        ];
        d.player_in_nether = false;
        d.eye_count = 2;
        d.throws = vec![
            crate::nbb_data::NbbThrow { x_in_overworld: 123.45, z_in_overworld: -678.9, has_position: true, angle: -42.0, angle_without_correction: -42.0, correction: 0.01, error: 0.0021, ..Default::default() },
            crate::nbb_data::NbbThrow { x_in_overworld: 130.0, z_in_overworld: -650.0, has_position: true, angle: -40.0, angle_without_correction: -40.05, correction: 0.05, error: -0.013, ..Default::default() },
        ];
        d.information_messages = vec![crate::nbb_data::NbbInfoMessage { severity: "WARNING".into(), msg_type: "MISMEASURE".into(), message: String::new() }];
        d.boat_state = "VALID".into();
        let font = FontArc::try_from_slice(OPEN_SANS).unwrap();
        let mut cache = NbbOverlayCache::new();
        let icons = cache.icons().each_ref().map(|i| Rgba { px: i.px.clone(), w: i.w, h: i.h });
        let panel = rasterize_panel(&cfg.sanitized(), &d, &font, &icons);
        // composite onto dark gray so alpha issues are visible, save both
        let mut flat = panel.px.clone();
        for p in flat.chunks_exact_mut(4) {
            let a = p[3] as f32 / 255.0;
            for c in 0..3 { p[c] = (p[c] as f32 * a + 40.0 * (1.0 - a)) as u8; }
            p[3] = 255;
        }
        image::RgbaImage::from_raw(panel.w, panel.h, flat).unwrap()
            .save(format!("{dir}/nbb_panel.png")).unwrap();
        eprintln!("dumped {}x{} panel", panel.w, panel.h);
    }
}
