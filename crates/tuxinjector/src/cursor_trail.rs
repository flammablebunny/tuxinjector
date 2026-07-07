// Cursor trail, ported from toolscreen's cursor_trail.cpp.
//
// It's a 512-stamp ring buffer of fading sprites laid down along a quadratic
// Bezier between pointer samples, then flushed as one batched quad draw under
// the imgui GUI. advance() and build_verts() take the time as an argument and
// touch no GL, which keeps them unit-testable -- only ensure_sprite() talks to
// the driver.

use std::time::Instant;

use tuxinjector_config::types::CursorTrailConfig;
use tuxinjector_gl_interop::TrailVertex;
use tuxinjector_render::image_loader::{self, ImageData};

use crate::gl_resolve::GlFunctions;

const MAX_STAMPS: usize = 512;
const MAX_STAMPS_PER_FRAME: usize = 128;
const MAX_SAMPLE_GAP_MS: u64 = 100;
// When the trail wakes up from a dormant stretch (gameplay / GUI) the pointer
// gets yanked to screen center by a warp on menu open. Hold off emitting for
// this long so the warp settles without leaving a streak behind it.
const RESYNC_GRACE_MS: u64 = 120;
const MAX_SPRITE_PX: u32 = 256;
// We render the dot big so the mip chain has something to work with. The
// default 11px stamp is a heavy minification of this, which is exactly the
// case mipmaps exist for.
const PROCEDURAL_SPRITE_SIZE: u32 = 256;

// local GL constants (same convention as overlay.rs)
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_LINEAR: i32 = 0x2601;
const GL_LINEAR_MIPMAP_LINEAR: i32 = 0x2703;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_RGBA: u32 = 0x1908;
const GL_RGBA8: u32 = 0x8058;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_PIXEL_UNPACK_BUFFER: u32 = 0x88EC;
const GL_UNPACK_ROW_LENGTH: u32 = 0x0CF2;
const GL_UNPACK_SKIP_ROWS: u32 = 0x0CF3;
const GL_UNPACK_SKIP_PIXELS: u32 = 0x0CF4;
const GL_UNPACK_ALIGNMENT: u32 = 0x0CF5;

#[derive(Clone, Copy, Default)]
struct TrailStamp {
    x: f32,
    y: f32,
    birth_ms: u64,
    size_boost: f32,
}

pub struct CursorTrail {
    stamps: Vec<TrailStamp>, // fixed MAX_STAMPS ring
    head: usize,
    count: usize,
    samples: [(f32, f32); 3],
    sample_count: usize,
    last_call_ms: u64,
    resync_until_ms: u64,
    last_fb: (u32, u32),
    epoch: Instant,
    sprite_tex: u32,
    sprite_key: Option<String>,
    verts: Vec<TrailVertex>,
}

impl CursorTrail {
    pub fn new() -> Self {
        Self {
            stamps: vec![TrailStamp::default(); MAX_STAMPS],
            head: 0,
            count: 0,
            samples: [(0.0, 0.0); 3],
            sample_count: 0,
            last_call_ms: 0,
            resync_until_ms: 0,
            last_fb: (0, 0),
            epoch: Instant::now(),
            sprite_tex: 0,
            sprite_key: None,
            verts: Vec::new(),
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }

    pub fn reset(&mut self) {
        self.head = 0;
        self.count = 0;
        self.sample_count = 0;
    }

    /// Feed a pointer sample (physical framebuffer pixels) and lay down stamps.
    /// Bails and resets on framebuffer resize, sample gaps > 100 ms, and jumps
    /// bigger than the screen diagonal (the alt-tab / teleport guard). Coming
    /// out of dormancy it holds a settle window so the menu-open centering warp
    /// leaves no streak.
    pub fn advance(
        &mut self,
        pos: (f32, f32),
        now_ms: u64,
        cfg: &CursorTrailConfig,
        fb: (u32, u32),
    ) {
        if fb != self.last_fb {
            self.last_fb = fb;
            self.reset();
        }
        let gap = now_ms.saturating_sub(self.last_call_ms);
        self.last_call_ms = now_ms;
        if gap > MAX_SAMPLE_GAP_MS {
            // Resuming from dormancy (gameplay / GUI). The menu-open cursor
            // centering warps the pointer to the screen center a frame or two
            // from now; connecting across that warp draws a stray streak, so
            // hold a settle window that emits nothing.
            self.sample_count = 0;
            self.resync_until_ms = now_ms + RESYNC_GRACE_MS;
        }
        // During the settle window, keep only the latest position as a lone
        // seed and emit nothing, so the centering warp lands trail-free.
        if now_ms < self.resync_until_ms {
            self.samples[0] = pos;
            self.sample_count = 1;
            return;
        }
        if self.sample_count > 0 {
            let (lx, ly) = self.samples[self.sample_count - 1];
            let jump = ((pos.0 - lx).powi(2) + (pos.1 - ly).powi(2)).sqrt();
            let diag = ((fb.0 as f32).powi(2) + (fb.1 as f32).powi(2)).sqrt();
            if jump > diag {
                self.sample_count = 0;
            }
        }

        // 3-deep shift register of pointer samples
        if self.sample_count < 3 {
            self.samples[self.sample_count] = pos;
            self.sample_count += 1;
        } else {
            self.samples[0] = self.samples[1];
            self.samples[1] = self.samples[2];
            self.samples[2] = pos;
        }
        if self.sample_count < 2 {
            return;
        }

        let (s2x, s2y) = pos;
        let (s1x, s1y) = self.samples[self.sample_count - 2];
        let chord = ((s2x - s1x).powi(2) + (s2y - s1y).powi(2)).sqrt();
        let spacing = cfg.stamp_spacing_px.max(1) as f32;
        let steps = ((chord / spacing) as usize).min(MAX_STAMPS_PER_FRAME);
        if steps == 0 {
            return;
        }

        // quadratic Bezier control point: Catmull-Rom-like tangent once we
        // have 3 samples, plain chord midpoint before that
        let (ctl_x, ctl_y) = if self.sample_count >= 3 {
            let (s0x, s0y) = self.samples[self.sample_count - 3];
            (s1x + (s2x - s0x) * 0.25, s1y + (s2y - s0y) * 0.25)
        } else {
            (0.5 * (s1x + s2x), 0.5 * (s1y + s2y))
        };

        // stamps emitted while moving fast are bigger, and keep their size
        let boost = if cfg.use_velocity_size && gap > 0 {
            let v_px_per_ms = chord / gap as f32;
            1.0 + cfg.velocity_size_intensity * (v_px_per_ms / 2.0).clamp(0.0, 1.0)
        } else {
            1.0
        };

        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let inv = 1.0 - t;
            let x = inv * inv * s1x + 2.0 * inv * t * ctl_x + t * t * s2x;
            let y = inv * inv * s1y + 2.0 * inv * t * ctl_y + t * t * s2y;
            self.stamps[self.head] = TrailStamp { x, y, birth_ms: now_ms, size_boost: boost };
            self.head = (self.head + 1) % MAX_STAMPS;
            self.count = (self.count + 1).min(MAX_STAMPS);
        }
    }

    /// Build NDC vertices for every live stamp. Decay math is toolscreen's:
    /// alpha = (1-age)^2 * opacity, size = base * ((1-age) + tailScale*age).
    pub fn build_verts(
        &mut self,
        now_ms: u64,
        cfg: &CursorTrailConfig,
        fb: (u32, u32),
    ) -> &[TrailVertex] {
        self.verts.clear();
        if fb.0 == 0 || fb.1 == 0 {
            return &self.verts;
        }
        let lifetime = cfg.lifetime_ms.max(1) as f32;
        let base = cfg.sprite_size_px.max(1) as f32;
        let (w, h) = (fb.0 as f32, fb.1 as f32);
        let head_c = cfg.color.to_array();
        let tail_c = cfg.tail_color.to_array();

        for i in 0..self.count {
            let idx = (self.head + MAX_STAMPS - 1 - i) % MAX_STAMPS;
            let stamp = self.stamps[idx];
            let age = now_ms.saturating_sub(stamp.birth_ms) as f32;
            if age >= lifetime {
                continue;
            }
            let t = age / lifetime;
            let inv = 1.0 - t;
            let alpha = inv * inv * cfg.opacity;
            if alpha <= 0.001 {
                continue;
            }
            let size_scale = inv + cfg.tail_size_scale * t;
            let half = base * size_scale * stamp.size_boost * 0.5;
            if half <= 0.5 {
                continue;
            }

            let rgba = if cfg.use_gradient {
                // Mix by how faded the stamp is (1 - alphaCurve), not raw age.
                // Alpha decays as (1-t)^2, so a linear age mix only reached the
                // tail color once the stamp was nearly transparent -- the
                // gradient looked like it did nothing. Weighting the head by
                // the alpha curve makes the tail color appear while the stamp
                // is still visible, and keeps both endpoints exact.
                let head_w = inv * inv;
                let tail_w = 1.0 - head_w;
                [
                    head_c[0] * head_w + tail_c[0] * tail_w,
                    head_c[1] * head_w + tail_c[1] * tail_w,
                    head_c[2] * head_w + tail_c[2] * tail_w,
                    alpha,
                ]
            } else {
                [head_c[0], head_c[1], head_c[2], alpha]
            };

            // pixel rect -> NDC, y flipped
            let x0 = (stamp.x - half) / w * 2.0 - 1.0;
            let x1 = (stamp.x + half) / w * 2.0 - 1.0;
            let y0 = 1.0 - (stamp.y - half) / h * 2.0;
            let y1 = 1.0 - (stamp.y + half) / h * 2.0;

            let v = |px: f32, py: f32, u: f32, vv: f32| TrailVertex {
                pos: [px, py],
                uv: [u, vv],
                rgba,
            };
            self.verts.push(v(x0, y0, 0.0, 0.0));
            self.verts.push(v(x1, y0, 1.0, 0.0));
            self.verts.push(v(x1, y1, 1.0, 1.0));
            self.verts.push(v(x0, y0, 0.0, 0.0));
            self.verts.push(v(x1, y1, 1.0, 1.0));
            self.verts.push(v(x0, y1, 0.0, 1.0));
        }
        &self.verts
    }

    /// Upload (once) and return the sprite texture. Render thread only.
    /// Invalid/oversized custom sprites fall back to the procedural dot.
    pub unsafe fn ensure_sprite(&mut self, gl: &GlFunctions, path: &str) -> u32 {
        if self.sprite_key.as_deref() == Some(path) && self.sprite_tex != 0 {
            return self.sprite_tex;
        }
        let (px, w, h) = load_sprite_pixels(path);
        if self.sprite_tex == 0 {
            let mut t = 0u32;
            (gl.gen_textures)(1, &mut t);
            self.sprite_tex = t;
        }
        // neutralize unpack state (Blaze3D leaks PBO bindings / row length)
        (gl.bind_buffer)(GL_PIXEL_UNPACK_BUFFER, 0);
        (gl.pixel_store_i)(GL_UNPACK_ALIGNMENT, 1);
        (gl.pixel_store_i)(GL_UNPACK_ROW_LENGTH, 0);
        (gl.pixel_store_i)(GL_UNPACK_SKIP_ROWS, 0);
        (gl.pixel_store_i)(GL_UNPACK_SKIP_PIXELS, 0);
        (gl.bind_texture)(GL_TEXTURE_2D, self.sprite_tex);
        (gl.tex_image_2d)(
            GL_TEXTURE_2D, 0, GL_RGBA8 as i32,
            w as i32, h as i32, 0,
            GL_RGBA, GL_UNSIGNED_BYTE,
            px.as_ptr() as *const std::ffi::c_void,
        );
        // Trilinear minification: stamps are usually far smaller than the
        // sprite (default 11px from a 256px dot), and without mipmaps that
        // undersampling aliases into blocky, shimmering dots.
        //
        // Apple's GL 2.1 compat context can't mip a non-power-of-two custom
        // sprite (GL_APPLE_texture_2D_limited_npot has no mip support), so the
        // texture goes mip-incomplete and samples black. Skip mipmaps there and
        // eat a little aliasing instead.
        #[cfg(not(target_os = "macos"))]
        {
            (gl.generate_mipmap)(GL_TEXTURE_2D);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR);
        }
        #[cfg(target_os = "macos")]
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        (gl.bind_texture)(GL_TEXTURE_2D, 0);
        self.sprite_key = Some(path.to_string());
        tracing::debug!(path, w, h, "uploaded cursor-trail sprite");
        self.sprite_tex
    }
}

fn load_sprite_pixels(path: &str) -> (Vec<u8>, u32, u32) {
    if !path.is_empty() {
        match image_loader::load_image(std::path::Path::new(path)) {
            Ok(ImageData::Static(img))
                if img.width <= MAX_SPRITE_PX && img.height <= MAX_SPRITE_PX
                    && img.width > 0 && img.height > 0 =>
            {
                return (img.pixels, img.width, img.height);
            }
            Ok(_) => tracing::warn!(path, "trail sprite rejected (animated or > 256px)"),
            Err(e) => tracing::warn!(path, %e, "trail sprite load failed"),
        }
    }
    procedural_dot(PROCEDURAL_SPRITE_SIZE)
}

// soft white radial dot with quadratic falloff: a = (1 - d)^2
fn procedural_dot(size: u32) -> (Vec<u8>, u32, u32) {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let half = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = (x as f32 - half) / half;
            let dy = (y as f32 - half) / half;
            let d = (dx * dx + dy * dy).sqrt();
            let a = (1.0 - d.clamp(0.0, 1.0)).powi(2);
            let idx = ((y * size + x) * 4) as usize;
            px[idx] = 255;
            px[idx + 1] = 255;
            px[idx + 2] = 255;
            px[idx + 3] = (a * 255.0) as u8;
        }
    }
    (px, size, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> CursorTrailConfig {
        CursorTrailConfig { enabled: true, ..Default::default() }
    }

    // The gradient must actually reach the tail color while the stamp is
    // still visible. With a linear color lerp against the (1-t)^2 alpha decay,
    // the tail color only appeared at near-zero alpha -> invisible gradient.
    #[test]
    fn gradient_reaches_tail_while_visible() {
        use tuxinjector_core::color::Color;
        let mut trail = CursorTrail::new();
        let mut c = cfg();
        c.stamp_spacing_px = 64;
        c.opacity = 1.0;
        c.use_gradient = true;
        c.color = Color::WHITE; // head
        c.tail_color = Color { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }; // tail = red
        trail.advance((0.0, 0.0), 0, &c, (1920, 1080));
        trail.advance((64.0, 0.0), 10, &c, (1920, 1080));

        // half-life: the stamp is still clearly visible (alpha ~0.25)...
        let now = 10 + (c.lifetime_ms as u64) / 2;
        let v = trail.build_verts(now, &c, (1920, 1080))[0];
        assert!(v.rgba[3] >= 0.2, "stamp should still be visible: a={}", v.rgba[3]);
        // ...and should already read as mostly the tail color (red), not pink
        assert!(v.rgba[1] <= 0.4 && v.rgba[2] <= 0.4,
            "tail color not reached while visible: g={} b={}", v.rgba[1], v.rgba[2]);

        // endpoints still exact: newest stamp = head, and head at age 0
        let fresh = {
            let mut t2 = CursorTrail::new();
            t2.advance((0.0, 0.0), 0, &c, (1920, 1080));
            t2.advance((64.0, 0.0), 0, &c, (1920, 1080));
            t2.build_verts(0, &c, (1920, 1080))[0]
        };
        assert_eq!(&fresh.rgba[0..3], &[1.0, 1.0, 1.0], "age 0 must be head color");
    }

    // On menu open the pointer is centered by a programmatic warp; the trail
    // must not connect the pre-warp (displaced) position to the center.
    #[test]
    fn menu_open_centering_warp_leaves_no_streak() {
        let mut trail = CursorTrail::new();
        let mut c = cfg();
        c.stamp_spacing_px = 4;
        let fb = (1920, 1080);
        let center = (960.0, 540.0);

        // resume from dormant gameplay: first visible frame after a long gap,
        // pointer at a displaced spot, then the centering warp jumps to center
        let mut t = 5000u64;
        trail.advance((1500.0, 900.0), t, &c, fb); // resume frame (displaced)
        t += 16;
        trail.advance(center, t, &c, fb); // centering warp -> center
        for _ in 0..10 {
            t += 16; // keep advancing each frame while the cursor rests at center
            trail.advance(center, t, &c, fb);
        }
        assert_eq!(trail.count, 0, "centering warp must leave no stray stamps");

        // real motion after settling trails normally
        t += 16;
        trail.advance((1000.0, 540.0), t, &c, fb); // 40px move from center
        assert!(trail.count > 0, "trailing resumes after the cursor settles");
    }

    #[test]
    fn emits_stamps_along_chord() {
        let mut trail = CursorTrail::new();
        let mut c = cfg();
        c.stamp_spacing_px = 10;
        trail.advance((0.0, 0.0), 0, &c, (1920, 1080));
        trail.advance((100.0, 0.0), 16, &c, (1920, 1080));
        assert_eq!(trail.count, 10, "100px chord / 10px spacing = 10 stamps");
    }

    #[test]
    fn per_frame_stamp_cap() {
        let mut trail = CursorTrail::new();
        let c = cfg(); // spacing 1
        trail.advance((0.0, 0.0), 0, &c, (1920, 1080));
        trail.advance((1900.0, 0.0), 16, &c, (1920, 1080));
        assert_eq!(trail.count, MAX_STAMPS_PER_FRAME);
    }

    #[test]
    fn decay_math_exact() {
        let mut trail = CursorTrail::new();
        let mut c = cfg();
        c.stamp_spacing_px = 64;
        c.opacity = 0.8;
        c.tail_size_scale = 0.5;
        trail.advance((0.0, 0.0), 0, &c, (1920, 1080));
        trail.advance((64.0, 0.0), 10, &c, (1920, 1080));
        assert_eq!(trail.count, 1);

        // at half life: alpha = 0.5^2 * 0.8, size scale = 0.5 + 0.5*0.5
        let now = 10 + (c.lifetime_ms as u64) / 2;
        let verts = trail.build_verts(now, &c, (1920, 1080));
        assert_eq!(verts.len(), 6);
        let alpha = verts[0].rgba[3];
        assert!((alpha - 0.25 * 0.8).abs() < 1e-3, "alpha {alpha}");
        // quad width in pixels = base * scale
        let expect_w = c.sprite_size_px as f32 * 0.75;
        let px_w = (verts[1].pos[0] - verts[0].pos[0]) / 2.0 * 1920.0;
        assert!((px_w - expect_w).abs() < 0.01, "width {px_w} vs {expect_w}");

        // past lifetime: dropped
        let verts = trail.build_verts(10 + c.lifetime_ms as u64, &c, (1920, 1080));
        assert!(verts.is_empty());
    }

    #[test]
    fn resets_on_fb_change_gap_and_jump() {
        let mut trail = CursorTrail::new();
        let mut c = cfg();
        c.stamp_spacing_px = 10;
        trail.advance((0.0, 0.0), 0, &c, (1920, 1080));
        trail.advance((100.0, 0.0), 16, &c, (1920, 1080));
        assert!(trail.count > 0);

        // fb change clears stamps
        trail.advance((100.0, 0.0), 32, &c, (1280, 720));
        assert_eq!(trail.count, 0);

        // movement stamps again
        trail.advance((0.0, 0.0), 48, &c, (1280, 720));
        let stamped = trail.count;
        assert!(stamped > 0);

        // >100ms gap resets sampling: no streak from the stale sample
        trail.advance((500.0, 500.0), 300, &c, (1280, 720));
        assert_eq!(trail.count, stamped, "gap reset means no chord to stamp");

        // jump > diagonal resets sampling
        let before = trail.count;
        trail.advance((10000.0, 10000.0), 316, &c, (1280, 720));
        assert_eq!(trail.count, before, "teleport must not stamp a streak");
    }

    #[test]
    fn velocity_boost_and_gradient() {
        let mut trail = CursorTrail::new();
        let mut c = cfg();
        c.stamp_spacing_px = 64;
        c.use_velocity_size = true;
        c.velocity_size_intensity = 1.0;
        c.use_gradient = true;
        c.tail_color = tuxinjector_core::color::Color::WHITE;
        c.color = tuxinjector_core::color::Color::BLACK;
        trail.advance((0.0, 0.0), 0, &c, (1920, 1080));
        // 64px in 16ms = 4px/ms -> v/2 clamps to 1 -> boost = 2
        trail.advance((64.0, 0.0), 16, &c, (1920, 1080));
        let verts = trail.build_verts(16, &c, (1920, 1080));
        assert_eq!(verts.len(), 6);
        let px_w = (verts[1].pos[0] - verts[0].pos[0]) / 2.0 * 1920.0;
        assert!((px_w - c.sprite_size_px as f32 * 2.0).abs() < 0.01, "boosted width {px_w}");
        // age 0 -> pure head color (black)
        assert_eq!(&verts[0].rgba[0..3], &[0.0, 0.0, 0.0]);
    }

    #[test]
    fn ring_buffer_wraps() {
        let mut trail = CursorTrail::new();
        let c = cfg(); // spacing 1, 128 cap per frame
        let mut t = 0u64;
        let mut x = 0.0f32;
        for _ in 0..10 {
            trail.advance((x, 0.0), t, &c, (100_000, 1080));
            x += 100.0;
            t += 16;
        }
        assert_eq!(trail.count, MAX_STAMPS);
        // build must not panic and yields at most MAX_STAMPS quads
        let verts = trail.build_verts(t, &c.clamped(), (100_000, 1080));
        assert!(verts.len() <= MAX_STAMPS * 6);
    }
}
