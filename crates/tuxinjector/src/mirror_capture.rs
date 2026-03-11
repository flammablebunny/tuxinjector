// Blits game framebuffer regions into per-mirror FBOs with double-buffered
// PBO async readback so we don't stall the GPU.

use crate::gl_resolve::GlFunctions;

// GL constants we need (not pulling in a full binding crate for these)
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_RGBA: u32 = 0x1908;
const GL_RGBA8: u32 = 0x8058;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_COLOR_BUFFER_BIT: u32 = 0x00004000;
const GL_LINEAR: u32 = 0x2601;
const GL_NEAREST: u32 = 0x2600;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_CLAMP_TO_EDGE: u32 = 0x812F;
const GL_PIXEL_PACK_BUFFER: u32 = 0x88EB;
const GL_STREAM_READ: u32 = 0x88E1;
const GL_READ_ONLY: u32 = 0x88B8;

// --- CaptureTarget ---

// FBO + texture + double-buffered PBOs for one mirror
struct CaptureTarget {
    fbo: u32,
    tex: u32,
    w: u32,
    h: u32,
    pbo: [u32; 2],
    pbo_idx: usize,    // which PBO we're writing to (other is for readback)
    frame_cnt: u32,    // need at least 1 frame before reading back
}

impl CaptureTarget {
    unsafe fn new(gl: &GlFunctions, w: u32, h: u32) -> Self {
        let mut fbo = 0u32;
        let mut tex = 0u32;

        (gl.gen_framebuffers)(1, &mut fbo);
        (gl.gen_textures)(1, &mut tex);

        // color attachment
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.tex_image_2d)(
            GL_TEXTURE_2D, 0, GL_RGBA8 as i32,
            w as i32, h as i32, 0,
            GL_RGBA, GL_UNSIGNED_BYTE, std::ptr::null(),
        );
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as i32);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as i32);
        (gl.bind_texture)(GL_TEXTURE_2D, 0);

        (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
        (gl.framebuffer_texture_2d)(
            GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0,
        );

        let status = (gl.check_framebuffer_status)(GL_FRAMEBUFFER);
        if status != GL_FRAMEBUFFER_COMPLETE {
            tracing::error!(status, "mirror capture FBO incomplete");
        }

        (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);

        // double-buffered PBOs for async readback
        let mut pbo = [0u32; 2];
        (gl.gen_buffers)(2, pbo.as_mut_ptr());
        let buf_sz = (w as isize) * (h as isize) * 4;
        for &p in &pbo {
            (gl.bind_buffer)(GL_PIXEL_PACK_BUFFER, p);
            (gl.buffer_data)(GL_PIXEL_PACK_BUFFER, buf_sz, std::ptr::null(), GL_STREAM_READ);
        }
        (gl.bind_buffer)(GL_PIXEL_PACK_BUFFER, 0);

        Self { fbo, tex, w, h, pbo, pbo_idx: 0, frame_cnt: 0 }
    }

    unsafe fn destroy(&self, gl: &GlFunctions) {
        (gl.delete_framebuffers)(1, &self.fbo);
        (gl.delete_textures)(1, &self.tex);
        (gl.delete_buffers)(2, self.pbo.as_ptr());
    }
}

// --- MirrorCaptureState ---

pub struct MirrorCaptureState {
    pub name: String,
    tgt: CaptureTarget,
    pixels: Vec<u8>,
    dirty: bool,
    multi_px: Vec<Vec<u8>>,  // per-input-region buffers for multi-input mirrors
    multi_dirty: bool,
}

impl MirrorCaptureState {
    pub unsafe fn new(gl: &GlFunctions, name: &str, w: u32, h: u32) -> Self {
        let tgt = CaptureTarget::new(gl, w, h);
        let sz = (w as usize) * (h as usize) * 4;
        Self {
            name: name.to_string(),
            tgt,
            pixels: vec![0u8; sz],
            dirty: false,
            multi_px: Vec::new(),
            multi_dirty: false,
        }
    }

    pub fn texture_id(&self) -> u32 {
        self.tgt.tex
    }

    pub fn capture_dimensions(&self) -> (u32, u32) {
        (self.tgt.w, self.tgt.h)
    }

    // for filtered mirrors that need CPU-side visibility checks
    pub fn check_pixels(&self) -> Option<&[u8]> {
        if self.dirty || self.tgt.frame_cnt > 1 {
            Some(&self.pixels)
        } else {
            None
        }
    }

    // Caller must save/restore FBO bindings
    pub unsafe fn capture_from(
        &mut self,
        gl: &GlFunctions,
        src_x: i32, src_y: i32,
        src_w: i32, src_h: i32,
        source_fbo: Option<u32>,
        nearest: bool,
        skip_readback: bool,
    ) {
        let read_fbo = source_fbo.unwrap_or_else(|| {
            if crate::virtual_fb::is_active() {
                let fbo = crate::virtual_fb::virtual_fbo();
                if fbo != 0 { fbo } else { 0 }
            } else {
                0
            }
        });

        let dw = self.tgt.w as i32;
        let dh = self.tgt.h as i32;

        // fast path: same-size unfiltered mirrors can skip the DRAW FBO bind
        if skip_readback && src_w == dw && src_h == dh {
            (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, read_fbo);
            (gl.bind_texture)(GL_TEXTURE_2D, self.tgt.tex);
            (gl.copy_tex_sub_image_2d)(GL_TEXTURE_2D, 0, 0, 0, src_x, src_y, dw, dh);
            return;
        }

        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, read_fbo);
        (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, self.tgt.fbo);

        let filter = if nearest || (src_w == dw && src_h == dh) {
            GL_NEAREST
        } else {
            GL_LINEAR
        };
        (gl.blit_framebuffer)(
            src_x, src_y, src_x + src_w, src_y + src_h,
            0, 0, dw, dh,
            GL_COLOR_BUFFER_BIT, filter,
        );

        // zero-copy path: FBO texture already has the content
        if skip_readback {
            return;
        }

        // --- PBO async readback ---
        let wr_pbo = self.tgt.pbo[self.tgt.pbo_idx];
        let rd_pbo = self.tgt.pbo[1 - self.tgt.pbo_idx];

        // kick off async readback into current PBO
        (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, self.tgt.fbo);
        (gl.bind_buffer)(GL_PIXEL_PACK_BUFFER, wr_pbo);
        (gl.read_pixels)(0, 0, dw, dh, GL_RGBA, GL_UNSIGNED_BYTE, std::ptr::null_mut());

        // map *previous* PBO to get last frame's data
        if self.tgt.frame_cnt > 0 {
            (gl.bind_buffer)(GL_PIXEL_PACK_BUFFER, rd_pbo);
            let mapped = (gl.map_buffer)(GL_PIXEL_PACK_BUFFER, GL_READ_ONLY);
            if !mapped.is_null() {
                let n = (dw as usize) * (dh as usize) * 4;
                std::ptr::copy_nonoverlapping(
                    mapped as *const u8, self.pixels.as_mut_ptr(), n,
                );
                (gl.unmap_buffer)(GL_PIXEL_PACK_BUFFER);

                // GL gives us bottom-up rows, flip to top-down
                flip_rows_inplace(&mut self.pixels, dw as usize, dh as usize);

                self.dirty = true;
            }
        }

        (gl.bind_buffer)(GL_PIXEL_PACK_BUFFER, 0);

        self.tgt.pbo_idx = 1 - self.tgt.pbo_idx;
        self.tgt.frame_cnt = self.tgt.frame_cnt.saturating_add(1);

        // periodic diagnostic so we can sanity-check captures
        static CAP_DIAG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let cnt = CAP_DIAG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if cnt % 120 == 0 {
            let row_bytes = (dw * 4) as usize;
            let mid = (dh as usize / 2) * row_bytes + (dw as usize / 2) * 4;
            let sample = if mid + 4 <= self.pixels.len() {
                [self.pixels[mid], self.pixels[mid+1], self.pixels[mid+2], self.pixels[mid+3]]
            } else {
                [0, 0, 0, 0]
            };
            let any_nonzero = self.pixels.iter().any(|&b| b != 0);
            tracing::debug!(
                mirror = %self.name,
                src_x, src_y, src_w, src_h,
                dw, dh, read_fbo,
                center_rgba = ?sample,
                any_nonzero,
                pbo_frame = self.tgt.frame_cnt,
                "mirror_capture: diagnostic"
            );
        }
    }

    // Multi-input capture with sync readback (for tiny mirrors where the stall is negligible).
    // Caller must save/restore FBO bindings.
    pub unsafe fn capture_multi_from(
        &mut self,
        gl: &GlFunctions,
        inputs: &[(i32, i32)], // (src_x, src_y) per region
        src_w: i32, src_h: i32,
        source_fbo: Option<u32>,
        nearest: bool,
    ) {
        let read_fbo = source_fbo.unwrap_or_else(|| {
            if crate::virtual_fb::is_active() {
                let fbo = crate::virtual_fb::virtual_fbo();
                if fbo != 0 { fbo } else { 0 }
            } else {
                0
            }
        });

        let dw = self.tgt.w as i32;
        let dh = self.tgt.h as i32;
        let buf_sz = (dw as usize) * (dh as usize) * 4;
        let half = dh as usize / 2;

        self.multi_px.clear();

        for &(sx, sy) in inputs {
            (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, read_fbo);
            (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, self.tgt.fbo);

            let filter = if nearest || (src_w == dw && src_h == dh) {
                GL_NEAREST
            } else {
                GL_LINEAR
            };
            (gl.blit_framebuffer)(
                sx, sy, sx + src_w, sy + src_h,
                0, 0, dw, dh,
                GL_COLOR_BUFFER_BIT, filter,
            );

            // sync readback (tiny mirrors, negligible stall)
            (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, self.tgt.fbo);
            (gl.bind_buffer)(GL_PIXEL_PACK_BUFFER, 0);

            let mut buf = vec![0u8; buf_sz];
            (gl.read_pixels)(
                0, 0, dw, dh,
                GL_RGBA, GL_UNSIGNED_BYTE,
                buf.as_mut_ptr() as *mut std::ffi::c_void,
            );

            // flip GL bottom-up to top-down
            let row_bytes = (dw * 4) as usize;
            let ptr = buf.as_mut_ptr();
            for row in 0..half {
                let top = row * row_bytes;
                let bot = (dh as usize - 1 - row) * row_bytes;
                std::ptr::swap_nonoverlapping(ptr.add(top), ptr.add(bot), row_bytes);
            }

            self.multi_px.push(buf);
        }

        self.multi_dirty = true;
    }

    // Clone the multi-input pixel data (persists across frame-skipped captures)
    pub fn peek_multi_pixels(&self) -> Option<Vec<(u32, u32, Vec<u8>)>> {
        if self.multi_px.is_empty() {
            None
        } else {
            let w = self.tgt.w;
            let h = self.tgt.h;
            Some(self.multi_px.iter().map(|p| (w, h, p.clone())).collect())
        }
    }

    pub unsafe fn resize(&mut self, gl: &GlFunctions, w: u32, h: u32) {
        if self.tgt.w == w && self.tgt.h == h {
            return;
        }
        self.tgt.destroy(gl);
        self.tgt = CaptureTarget::new(gl, w, h);
        self.pixels = vec![0u8; (w as usize) * (h as usize) * 4];
        self.dirty = false;
    }

    pub unsafe fn destroy(&self, gl: &GlFunctions) {
        self.tgt.destroy(gl);
    }
}

// flip RGBA rows in-place (GL gives bottom-up, we want top-down)
fn flip_rows_inplace(buf: &mut [u8], w: usize, h: usize) {
    let row_bytes = w * 4;
    let half = h / 2;
    let ptr = buf.as_mut_ptr();
    for row in 0..half {
        let top = row * row_bytes;
        let bot = (h - 1 - row) * row_bytes;
        unsafe {
            std::ptr::swap_nonoverlapping(ptr.add(top), ptr.add(bot), row_bytes);
        }
    }
}

// --- MirrorCaptureManager ---

pub struct MirrorCaptureManager {
    captures: Vec<MirrorCaptureState>,
}

impl MirrorCaptureManager {
    pub fn new() -> Self {
        Self { captures: Vec::new() }
    }

    // Reconcile with desired mirror list (create/resize/destroy as needed)
    pub unsafe fn sync_mirrors(
        &mut self,
        gl: &GlFunctions,
        desired: &[(&str, u32, u32)],
    ) {
        // drop mirrors that are no longer wanted
        self.captures.retain(|c| {
            let keep = desired.iter().any(|(name, _, _)| *name == c.name);
            if !keep {
                unsafe { c.destroy(gl) };
            }
            keep
        });

        for &(name, w, h) in desired {
            if let Some(existing) = self.captures.iter_mut().find(|c| c.name == name) {
                unsafe { existing.resize(gl, w, h) };
            } else {
                self.captures.push(unsafe { MirrorCaptureState::new(gl, name, w, h) });
            }
        }
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut MirrorCaptureState> {
        self.captures.iter_mut().find(|c| c.name == name)
    }

    pub unsafe fn destroy(&self, gl: &GlFunctions) {
        for c in &self.captures {
            c.destroy(gl);
        }
    }
}
