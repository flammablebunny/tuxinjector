// EyeZoom overlay PNG generator -- alternating grid boxes with distance
// labels and a center crosshair.
//
// Thanks to Priffin, Gore, and qMaxXen for their overlay-gen repos, which i used for inspo
// https://github.com/qMaxXen/overlay-gen
//! https://github.com/Priffin/Pixel-Perfect-Tools
//! https://github.com/arjuncgore/overlay-gen

use std::path::PathBuf;

use ab_glyph::{Font, FontArc, GlyphId, ScaleFont};
use tuxinjector_config::Config;

const OUTPUT_DIR: &str = "images";
const OUTPUT_FILE: &str = "overlay.png";

/// Kicks off overlay gen on a background thread (startup + hot-reload)
pub fn generate_overlay(config: &Config) {
    std::thread::Builder::new()
        .name("ts-overlay-gen".into())
        .spawn({
            let cfg = config.clone();
            move || {
                if let Err(e) = do_generate(&cfg) {
                    tracing::warn!(error = %e, "overlay_gen: failed");
                }
            }
        })
        .ok();
}

pub fn overlay_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    #[cfg(target_os = "macos")]
    let base = format!("{home}/.config");
    #[cfg(target_os = "linux")]
    let base = std::env::var("XDG_DATA_HOME")
        .unwrap_or_else(|_| format!("{home}/.local/share"));
    PathBuf::from(base)
        .join("tuxinjector")
        .join(OUTPUT_DIR)
        .join(OUTPUT_FILE)
}

fn do_generate(config: &Config) -> Result<(), String> {
    let ez = &config.overlays.eyezoom;

    // grid params (mirrors what mode_system::build_eyezoom does)
    let labels_per_side = ez.clone_width;
    let ov_width = if ez.overlay_width < 0 {
        labels_per_side
    } else {
        ez.overlay_width.min(labels_per_side)
    };

    let total_cells = (ez.clone_width * 2).max(1) as u32;

    // figure out what size to render at based on image/mirror config
    let ov_path_str = overlay_path().to_string_lossy().to_string();
    let (img_w, img_h) = find_output_size(config, &ov_path_str, ez, total_cells);
    let cell_w = img_w / total_cells;

    if img_w == 0 || img_h == 0 || img_w > 16384 || img_h > 16384 {
        return Err(format!(
            "overlay_gen: bad dimensions {img_w}x{img_h}"
        ));
    }

    let font = load_font(&ez.text_font_path, &config.theme.font_path);
    let widest_label = ov_width.unsigned_abs().to_string();

    // font size and box height -- linkRectToFont ties them together
    let (font_sz, box_h) = if ez.link_rect_to_font {
        if let Some(ref f) = font {
            let max_tw = cell_w as f32 * 0.85;
            let fs = if ez.auto_font_size {
                auto_fit_font_size(f, &widest_label, max_tw, img_h as f32)
            } else {
                ez.text_font_size as f32
            };
            let scaled = f.as_scaled(fs);
            let th = scaled.ascent() - scaled.descent();
            // 20% vertical padding
            let bh = (th * 1.2).round().max(10.0) as u32;
            (fs, bh)
        } else {
            (ez.text_font_size as f32, 20u32)
        }
    } else {
        let bh = if ez.rect_height > 0 {
            ez.rect_height as u32
        } else {
            (img_h as f32 * 0.08).round().max(10.0) as u32
        };
        let fs = if let Some(ref f) = font {
            let max_tw = cell_w as f32 * 0.85;
            let max_th = bh as f32 * 0.85;
            if ez.auto_font_size {
                auto_fit_font_size(f, &widest_label, max_tw, max_th)
            } else {
                ez.text_font_size as f32
            }
        } else {
            ez.text_font_size as f32
        };
        (fs, bh)
    };

    // transparent RGBA canvas
    let mut px = vec![0u8; (img_w * img_h * 4) as usize];

    let cx = img_w / 2;
    let cy = img_h / 2;
    let grid_top = cy - box_h / 2;

    // colors
    let c1 = color_to_rgba(&ez.grid_color1, ez.grid_color1_opacity);
    let c2 = color_to_rgba(&ez.grid_color2, ez.grid_color2_opacity);
    let hi_color = color_to_rgba(&ez.highlight_color, ez.highlight_color_opacity);
    let hi_interval = ez.highlight_interval.max(0) as u32;
    let xhair_color = color_to_rgba(&ez.center_line_color, ez.center_line_color_opacity);
    let text_rgba = color_to_rgba(&ez.text_color, ez.text_color_opacity);

    // draw alternating grid boxes (highlight every Nth cell)
    for x_off in -ov_width..=ov_width {
        if x_off == 0 {
            continue;
        }

        let num = x_off.unsigned_abs();
        let bi = x_off + labels_per_side - if x_off > 0 { 1 } else { 0 };
        let left = (bi as u32) * cell_w;
        let color = if hi_interval > 0 && num % hi_interval == 0 {
            hi_color
        } else if bi % 2 == 0 {
            c1
        } else {
            c2
        };

        fill_rect(&mut px, img_w, left, grid_top, cell_w, box_h, color);
    }

    // center crosshair (full height, 2px wide)
    let xhair_w = 2u32;
    let xhair_left = cx - xhair_w / 2;
    fill_rect(&mut px, img_w, xhair_left, 0, xhair_w, img_h, xhair_color);

    // text labels
    if let Some(font) = font {

        let scaled = font.as_scaled(font_sz);
        let num_style = ez.number_style.as_str();

        for x_off in -ov_width..=ov_width {
            if x_off == 0 {
                continue;
            }

            let num = x_off.unsigned_abs();
            let bi = x_off + labels_per_side - if x_off > 0 { 1 } else { 0 };
            let box_cx = (bi as f32 + 0.5) * cell_w as f32;
            let box_cy = cy as f32;

            // slackow style: only last digit for non-multiples, full number at multiples
            let is_decade = hi_interval > 0 && num % hi_interval == 0;
            let label = if num_style == "slackow" && !is_decade && num >= 10 {
                (num % 10).to_string()
            } else {
                num.to_string()
            };

            // should we stack digits vertically?
            let stack = label.len() > 1 && match num_style {
                "stacked" => true,
                "compact" => true,
                "slackow" => is_decade, // slackow, thanks for creating this style, i love it
                _ => false, // "horizontal" or anything else
            };

            if stack {
                // vertical stacking, centered in cell
                let digits: Vec<char> = label.chars().collect();
                let n_digits = digits.len() as f32;

                // compact mode shrinks the first digit to 60%
                let is_compact = num_style == "compact";

                let text_h = scaled.ascent() - scaled.descent();
                let small_sz = font_sz * 0.6;
                let small_scaled = font.as_scaled(small_sz);
                let small_h = small_scaled.ascent() - small_scaled.descent();

                let total_h = if is_compact {
                    small_h + text_h * (n_digits - 1.0)
                } else {
                    text_h * n_digits
                };
                let mut cur_y = box_cy - total_h / 2.0;

                for (di, &ch) in digits.iter().enumerate() {
                    let (use_scaled, use_h) = if is_compact && di == 0 {
                        (&small_scaled, small_h)
                    } else {
                        (&scaled, text_h)
                    };

                    let gid = font.glyph_id(ch);
                    let ch_w = use_scaled.h_advance(gid);
                    let gx = box_cx - ch_w / 2.0;
                    let baseline = cur_y + use_scaled.ascent();

                    let glyph = gid.with_scale_and_position(
                        use_scaled.scale(),
                        ab_glyph::point(gx, baseline),
                    );

                    if let Some(outlined) = font.outline_glyph(glyph) {
                        let bounds = outlined.px_bounds();
                        outlined.draw(|x, y, cov| {
                            let px_x = bounds.min.x as i32 + x as i32;
                            let px_y = bounds.min.y as i32 + y as i32;
                            if px_x >= 0 && (px_x as u32) < img_w
                                && px_y >= 0 && (px_y as u32) < img_h
                            {
                                let idx = ((px_y as u32 * img_w + px_x as u32) * 4) as usize;
                                let a = (cov * text_rgba[3] as f32 / 255.0 * 255.0).round() as u8;
                                blend_pixel(&mut px, idx, text_rgba, a);
                            }
                        });
                    }

                    cur_y += use_h;
                }
            } else {
                // horizontal single-line
                let total_w: f32 = label
                    .chars()
                    .map(|ch| scaled.h_advance(font.glyph_id(ch)))
                    .sum();

                let text_h = scaled.ascent() - scaled.descent();
                let mut gx = box_cx - total_w / 2.0;
                let baseline = box_cy - text_h / 2.0 + scaled.ascent();

                for ch in label.chars() {
                    let gid: GlyphId = font.glyph_id(ch);
                    let adv = scaled.h_advance(gid);

                    let glyph = gid.with_scale_and_position(
                        scaled.scale(),
                        ab_glyph::point(gx, baseline),
                    );

                    if let Some(outlined) = font.outline_glyph(glyph) {
                        let bounds = outlined.px_bounds();
                        outlined.draw(|x, y, cov| {
                            let px_x = bounds.min.x as i32 + x as i32;
                            let px_y = bounds.min.y as i32 + y as i32;
                            if px_x >= 0 && (px_x as u32) < img_w
                                && px_y >= 0 && (px_y as u32) < img_h
                            {
                                let idx = ((px_y as u32 * img_w + px_x as u32) * 4) as usize;
                                let a = (cov * text_rgba[3] as f32 / 255.0 * 255.0).round() as u8;
                                blend_pixel(&mut px, idx, text_rgba, a);
                            }
                        });
                    }

                    gx += adv;
                }
            }
        }
    }

    // write the PNG
    let out_path = overlay_path();
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("overlay_gen: mkdir {}: {e}", parent.display()))?;
    }

    let img = image::RgbaImage::from_raw(img_w, img_h, px)
        .ok_or("overlay_gen: failed to create image from pixel buffer")?;

    img.save(&out_path)
        .map_err(|e| format!("overlay_gen: save {}: {e}", out_path.display()))?;

    tracing::info!(
        path = %out_path.display(),
        width = img_w,
        height = img_h,
        "overlay_gen: done"
    );

    Ok(())
}

fn fill_rect(
    buf: &mut [u8],
    stride: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: [u8; 4],
) {
    let total_rows = buf.len() as u32 / (stride * 4);
    for row in y..y.saturating_add(h) {
        if row >= total_rows {
            break;
        }
        for col in x..x.saturating_add(w) {
            if col >= stride {
                break;
            }
            let i = ((row * stride + col) * 4) as usize;
            buf[i]     = color[0];
            buf[i + 1] = color[1];
            buf[i + 2] = color[2];
            buf[i + 3] = color[3];
        }
    }
}

fn blend_pixel(buf: &mut [u8], idx: usize, color: [u8; 4], alpha: u8) {
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

fn color_to_rgba(c: &tuxinjector_core::Color, opacity: f32) -> [u8; 4] {
    [
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
        (opacity * 255.0).round() as u8,
    ]
}

// Font loading with fallbacks: eyezoom path -> global theme -> system fallback
fn load_font(eyezoom_path: &str, global_path: &str) -> Option<FontArc> {
    if !eyezoom_path.is_empty() {
        if let Some(f) = load_font_from_path(eyezoom_path) {
            return Some(f);
        }
    }
    if !global_path.is_empty() {
        if let Some(f) = load_font_from_path(global_path) {
            return Some(f);
        }
    }
    resolve_fallback_font()
}

/// Platform-aware font fallback when no font path is configured.
/// Tries JetBrains Mono Nerd → Inter → ProggyClean via fontconfig (Linux)
/// or known system font paths (macOS).
pub fn resolve_fallback_font() -> Option<FontArc> {
    #[cfg(target_os = "macos")]
    {
        // macOS: check ~/Library/Fonts and /Library/Fonts for user-installed fonts,
        // then fall back to system monospace fonts
        let home = std::env::var("HOME").unwrap_or_default();
        let user_fonts: Vec<String> = ["JetBrainsMono Nerd Font", "JetBrainsMonoNerdFont", "Inter"]
            .iter()
            .flat_map(|name| {
                // User-installed fonts can be .ttf or .otf with various naming
                vec![
                    format!("{}/Library/Fonts/{}-Regular.ttf", home, name.replace(' ', "")),
                    format!("{}/Library/Fonts/{}-Regular.otf", home, name.replace(' ', "")),
                    format!("/Library/Fonts/{}-Regular.ttf", name.replace(' ', "")),
                    format!("/Library/Fonts/{}-Regular.otf", name.replace(' ', "")),
                ]
            })
            .collect();
        for path in &user_fonts {
            if let Some(f) = load_font_from_path(path) {
                return Some(f);
            }
        }
        // System monospace fallbacks
        for path in &[
            "/System/Library/Fonts/SFNSMono.ttf",
            "/System/Library/Fonts/Menlo.ttc",
            "/System/Library/Fonts/Monaco.dfont",
            "/System/Library/Fonts/Helvetica.ttc",
        ] {
            if let Some(f) = load_font_from_path(path) {
                return Some(f);
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        for family in &["JetBrainsMono Nerd Font", "Inter", "ProggyClean"] {
            if let Ok(output) = std::process::Command::new("fc-match")
                .args(["--format=%{file}", family])
                .output()
            {
                if output.status.success() {
                    let p = String::from_utf8_lossy(&output.stdout);
                    let p = p.trim();
                    if !p.is_empty() {
                        if let Some(f) = load_font_from_path(p) {
                            return Some(f);
                        }
                    }
                }
            }
        }
    }
    None
}

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{}{}", home, &path[1..]);
        }
    }
    path.to_owned()
}

fn load_font_from_path(path: &str) -> Option<FontArc> {
    let expanded = expand_tilde(path);
    let data = std::fs::read(&expanded).ok()?;
    FontArc::try_from_vec(data).ok()
}

// Figure out how big the output PNG should be from image/mirror config,
// falling back to default res (1920x1080) if nothing references it
fn find_output_size(
    config: &Config,
    ov_path: &str,
    _ez: &tuxinjector_config::types::EyeZoomConfig,
    total_cells: u32,
) -> (u32, u32) {
    // check if any image overlay references the generated PNG
    for img in &config.overlays.images {
        let p = expand_tilde(&img.path);
        if p == ov_path && img.output_width > 0 && img.output_height > 0 {
            return (img.output_width as u32, img.output_height as u32);
        }
    }

    // eyeMirror with rawOutput=true
    for mirror in &config.overlays.mirrors {
        if mirror.raw_output {
            let out = &mirror.output;
            if out.output_width > 0 && out.output_height > 0 {
                return (out.output_width as u32, out.output_height as u32);
            }
        }
    }

    // fallback: 1920x1080, snapped to clean cell boundaries
    let canvas_w = 1920u32.max(total_cells);
    let w = (canvas_w / total_cells) * total_cells;
    (w, 1080u32)
}

// Binary search for the biggest font size that fits within both bounds
fn auto_fit_font_size(font: &FontArc, text: &str, max_w: f32, max_h: f32) -> f32 {
    let mut lo = 4.0f32;
    let mut hi = max_h.min(200.0);

    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        let scaled = font.as_scaled(mid);

        let w: f32 = text
            .chars()
            .map(|ch| scaled.h_advance(font.glyph_id(ch)))
            .sum();
        let h = scaled.ascent() - scaled.descent();

        if w <= max_w && h <= max_h {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    lo.max(4.0)
}
