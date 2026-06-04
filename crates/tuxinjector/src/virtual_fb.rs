// Virtual backbuffer for oversized modes.
//
// When the game renders at a resolution larger than the physical surface,
// we redirect FBO 0 binds to an offscreen FBO and blit the centered slice
// back to the real default framebuffer at swap time.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use tracing;

use crate::gl_resolve::GlFunctions;

const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_READ_FRAMEBUFFER_BINDING: u32 = 0x8CAA;
const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_DEPTH_STENCIL_ATTACHMENT: u32 = 0x821A;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_RENDERBUFFER: u32 = 0x8D41;
const GL_DEPTH24_STENCIL8: u32 = 0x88F0;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_RGBA8: u32 = 0x8058;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_NEAREST: u32 = 0x2600;
const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;
const GL_SCISSOR_TEST: u32 = 0x0C11;
const GL_BACK: u32 = 0x0405;

static FBO: AtomicU32 = AtomicU32::new(0);
static TEX: AtomicU32 = AtomicU32::new(0);
static RBO: AtomicU32 = AtomicU32::new(0);
static W: AtomicU32 = AtomicU32::new(0);
static H: AtomicU32 = AtomicU32::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static USED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static BYPASS: Cell<bool> = Cell::new(false);
}

// RAII guard that temporarily bypasses the FBO redirect hook.
// Used when we need to bind FBO 0 for real (e.g. blit to screen).
pub struct FbBypassGuard {
    prev: bool,
}

impl Drop for FbBypassGuard {
    fn drop(&mut self) {
        BYPASS.with(|f| f.set(self.prev));
    }
}

pub fn fb_bypass_guard() -> FbBypassGuard {
    let prev = BYPASS.with(|f| {
        let p = f.get();
        f.set(true);
        p
    });
    FbBypassGuard { prev }
}

pub fn should_redirect_default() -> bool {
    if !ACTIVE.load(Ordering::Relaxed) {
        return false;
    }
    if FBO.load(Ordering::Relaxed) == 0 {
        return false;
    }
    !BYPASS.with(|f| f.get())
}

pub fn virtual_fbo() -> u32 {
    FBO.load(Ordering::Relaxed)
}

pub fn is_active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub fn set_active(active: bool) {
    ACTIVE.store(active, Ordering::Release);
    if !active {
        USED.store(false, Ordering::Release);
    }
}

pub fn mark_used() {
    USED.store(true, Ordering::Release);
}

pub fn was_used() -> bool {
    USED.load(Ordering::Acquire)
}

// Make sure the offscreen FBO exists and matches the requested mode size
pub unsafe fn ensure_offscreen(gl: &GlFunctions, mode_w: u32, mode_h: u32) {
    if mode_w == 0 || mode_h == 0 {
        return;
    }

    let cur_w = W.load(Ordering::Relaxed);
    let cur_h = H.load(Ordering::Relaxed);
    let mut fbo = FBO.load(Ordering::Relaxed);
    let mut tex = TEX.load(Ordering::Relaxed);
    let mut rbo = RBO.load(Ordering::Relaxed);

    if fbo != 0 && cur_w == mode_w && cur_h == mode_h {
        ACTIVE.store(true, Ordering::Release);
        return;
    }

    // first time or size changed - (re)create resources
    if fbo == 0 {
        let mut id = 0u32;
        (gl.gen_framebuffers)(1, &mut id);
        fbo = id;
        FBO.store(fbo, Ordering::Relaxed);

        (gl.gen_textures)(1, &mut id);
        tex = id;
        TEX.store(tex, Ordering::Relaxed);

        (gl.gen_renderbuffers)(1, &mut id);
        rbo = id;
        RBO.store(rbo, Ordering::Relaxed);
    }

    // color texture
    (gl.bind_texture)(GL_TEXTURE_2D, tex);
    (gl.tex_image_2d)(
        GL_TEXTURE_2D, 0, GL_RGBA8 as i32,
        mode_w as i32, mode_h as i32, 0,
        GL_RGBA, GL_UNSIGNED_BYTE, std::ptr::null(),
    );
    (gl.tex_parameter_i)(GL_TEXTURE_2D, 0x2801, GL_NEAREST as i32); // MIN_FILTER
    (gl.tex_parameter_i)(GL_TEXTURE_2D, 0x2800, GL_NEAREST as i32); // MAG_FILTER
    (gl.bind_texture)(GL_TEXTURE_2D, 0);

    // depth+stencil RBO
    (gl.bind_renderbuffer)(GL_RENDERBUFFER, rbo);
    (gl.renderbuffer_storage)(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, mode_w as i32, mode_h as i32);
    (gl.bind_renderbuffer)(GL_RENDERBUFFER, 0);

    // attach everything
    (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
    (gl.framebuffer_texture_2d)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
    (gl.framebuffer_renderbuffer)(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rbo);

    let status = (gl.check_framebuffer_status)(GL_FRAMEBUFFER);
    if status != GL_FRAMEBUFFER_COMPLETE {
        tracing::error!(status, "virtual_fb: offscreen FBO incomplete");
        ACTIVE.store(false, Ordering::Release);
    }

    // unbind without triggering the redirect hook
    let _g = fb_bypass_guard();
    (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);

    W.store(mode_w, Ordering::Release);
    H.store(mode_h, Ordering::Release);
    if status == GL_FRAMEBUFFER_COMPLETE {
        ACTIVE.store(true, Ordering::Release);
        USED.store(false, Ordering::Release);
    }

    tracing::debug!(mode_w, mode_h, fbo, tex, rbo, "virtual_fb: offscreen ready");
}

pub unsafe fn destroy_offscreen(gl: &GlFunctions) {
    ACTIVE.store(false, Ordering::Release);
    USED.store(false, Ordering::Release);

    let fbo = FBO.swap(0, Ordering::Relaxed);
    let tex = TEX.swap(0, Ordering::Relaxed);
    let rbo = RBO.swap(0, Ordering::Relaxed);

    if fbo != 0 { (gl.delete_framebuffers)(1, &fbo); }
    if tex != 0 { (gl.delete_textures)(1, &tex); }
    if rbo != 0 { (gl.delete_renderbuffers)(1, &rbo); }

    W.store(0, Ordering::Relaxed);
    H.store(0, Ordering::Relaxed);

    let _g = fb_bypass_guard();
    (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);
    (gl.draw_buffer)(GL_BACK);
    (gl.read_buffer)(GL_BACK);
}

// Blit the centered slice of the offscreen FBO back to the real default framebuffer
pub unsafe fn blit_to_default(
    gl: &GlFunctions,
    mode_w: u32, mode_h: u32,
    orig_w: u32, orig_h: u32,
) {
    if mode_w == 0 || mode_h == 0 || orig_w == 0 || orig_h == 0 {
        return;
    }

    let fbo = FBO.load(Ordering::Relaxed);
    if fbo == 0 {
        return;
    }
    if !USED.load(Ordering::Acquire) {
        tracing::debug!("virtual_fb: skip blit -- offscreen not used this frame");
        return;
    }

    // calculate centered src/dst rects per axis
    let (sx, sw, dx, dw) = if mode_w >= orig_w {
        let off = (mode_w - orig_w) / 2;
        (off as i32, orig_w as i32, 0, orig_w as i32)
    } else {
        let off = (orig_w - mode_w) / 2;
        (0, mode_w as i32, off as i32, mode_w as i32)
    };

    let (sy, sh, dy, dh) = if mode_h >= orig_h {
        let off = (mode_h - orig_h) / 2;
        (off as i32, orig_h as i32, 0, orig_h as i32)
    } else {
        let off = (orig_h - mode_h) / 2;
        (0, mode_h as i32, off as i32, mode_h as i32)
    };

    // save current state
    let mut prev_read = 0i32;
    let mut prev_draw = 0i32;
    let mut prev_draw_buf = 0i32;
    let mut prev_read_buf = 0i32;
    (gl.get_integer_v)(GL_READ_FRAMEBUFFER_BINDING, &mut prev_read);
    (gl.get_integer_v)(GL_DRAW_FRAMEBUFFER_BINDING, &mut prev_draw);
    (gl.get_integer_v)(0x0C02 /* GL_DRAW_BUFFER */, &mut prev_draw_buf);
    (gl.get_integer_v)(0x0C01 /* GL_READ_BUFFER */, &mut prev_read_buf);

    let _g = fb_bypass_guard();
    (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, fbo);
    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
    (gl.read_buffer)(GL_COLOR_ATTACHMENT0);
    (gl.draw_buffer)(GL_BACK);

    (gl.disable)(GL_SCISSOR_TEST);
    (gl.viewport)(0, 0, orig_w as i32, orig_h as i32);

    (gl.blit_framebuffer)(
        sx, sy, sx + sw, sy + sh,
        dx, dy, dx + dw, dy + dh,
        GL_COLOR_BUFFER_BIT, GL_NEAREST,
    );

    // restore
    (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, prev_read as u32);
    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, prev_draw as u32);
    if prev_draw_buf != 0 {
        (gl.draw_buffer)(prev_draw_buf as u32);
    }
    if prev_read_buf != 0 {
        (gl.read_buffer)(prev_read_buf as u32);
    }
}
