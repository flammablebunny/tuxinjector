// GL state save/restore for overlay compositing.
//
// Fast path: we know what MC's state looks like at SwapBuffers, so we can
// skip the expensive glGet* calls and just slam in our state, then restore
// the known defaults. Works 99% of the time.
//
// Full path: queries everything, restores everything. Used as a fallback
// when we don't trust the fast path (e.g. first frame, mod weirdness).

use crate::gl_bindings::*;

// ---- Fast path (no glGet* queries) ----

// Slam in our compositor GL state without saving anything
pub unsafe fn set_compositor_state(gl: &GlFns, vp: [i32; 4]) {
    (gl.enable)(GL_BLEND);
    (gl.blend_equation_separate)(GL_FUNC_ADD, GL_FUNC_ADD);
    (gl.blend_func_separate)(
        GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA,
        GL_ONE, GL_ONE_MINUS_SRC_ALPHA,
    );
    (gl.disable)(GL_DEPTH_TEST);
    (gl.disable)(GL_STENCIL_TEST);
    (gl.disable)(GL_CULL_FACE);
    (gl.disable)(GL_SCISSOR_TEST);
    (gl.disable)(GL_FRAMEBUFFER_SRGB);
    (gl.color_mask)(GL_TRUE, GL_TRUE, GL_TRUE, GL_FALSE);
    (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);
    (gl.viewport)(vp[0], vp[1], vp[2], vp[3]);

    // Ensure pixel unpack reads from CPU memory, not from any leftover PBO
    // (saved & restored at the outer boundary).
    (gl.bind_buffer)(GL_PIXEL_UNPACK_BUFFER, 0);

    // Clear any sampler objects MC left bound. A bound sampler (GL 3.3+)
    // overrides a texture's own min/mag/wrap params; MC 1.21's Blaze3D binds
    // them, which makes our mip-less LINEAR overlay textures sampler-incomplete
    // -> they sample pure black even though the CPU upload is fine. MC re-binds
    // its own samplers per draw, so no restore is needed (same as the PBO above).
    if let Some(bind_sampler) = gl.bind_sampler {
        for unit in 0..8u32 {
            bind_sampler(unit, 0);
        }
    }
}

// Restore what MC usually has at SwapBuffers time.
// Might not be 100% right if a mod is doing something weird.
pub unsafe fn restore_minecraft_state(gl: &GlFns, vp: [i32; 4]) {
    (gl.use_program)(0);
    (gl.bind_vertex_array)(0);
    (gl.bind_buffer)(GL_ARRAY_BUFFER, 0);
    (gl.bind_buffer)(GL_ELEMENT_ARRAY_BUFFER, 0);

    // Disable vertex attrib arrays so they don't interfere with
    // fixed-function / immediate-mode rendering (macOS GL 2.1).
    (gl.disable_vertex_attrib_array)(0);
    (gl.disable_vertex_attrib_array)(1);
    (gl.disable_vertex_attrib_array)(2);
    (gl.active_texture)(GL_TEXTURE0);
    (gl.bind_texture)(GL_TEXTURE_2D, 0);

    // standard alpha blend (what vanilla MC uses)
    (gl.enable)(GL_BLEND);
    (gl.blend_func_separate)(
        GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA,
        GL_ONE, GL_ZERO,
    );

    (gl.enable)(GL_DEPTH_TEST);
    (gl.enable)(GL_CULL_FACE);
    // stencil/scissor/sRGB were already disabled by set_compositor_state

    (gl.color_mask)(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
    (gl.viewport)(vp[0], vp[1], vp[2], vp[3]);

    // pixel unpack defaults
    (gl.pixel_store_i)(GL_UNPACK_ROW_LENGTH, 0);
    (gl.pixel_store_i)(GL_UNPACK_SKIP_ROWS, 0);
    (gl.pixel_store_i)(GL_UNPACK_SKIP_PIXELS, 0);
    (gl.pixel_store_i)(GL_UNPACK_ALIGNMENT, 4);
}

// ---- Targeted path (~10 queries for corrections after the fast restore) ----

// Snapshot of states the fast path might get wrong. We query these once
// every N frames and apply corrections after restore_minecraft_state.
pub struct TargetedGlState {
    pub framebuffer_srgb_enabled: GLboolean,
    pub framebuffer: GLint,
    pub blend_src_rgb: GLint,
    pub blend_dst_rgb: GLint,
    pub blend_src_alpha: GLint,
    pub blend_dst_alpha: GLint,
    pub blend_eq_rgb: GLint,
    pub blend_eq_alpha: GLint,
    pub depth_enabled: GLboolean,
    pub cull_enabled: GLboolean,
}

pub unsafe fn save_targeted_state(gl: &GlFns) -> TargetedGlState {
    let mut s = TargetedGlState {
        framebuffer_srgb_enabled: GL_FALSE,
        framebuffer: 0,
        blend_src_rgb: 0,
        blend_dst_rgb: 0,
        blend_src_alpha: 0,
        blend_dst_alpha: 0,
        blend_eq_rgb: 0,
        blend_eq_alpha: 0,
        depth_enabled: GL_FALSE,
        cull_enabled: GL_FALSE,
    };

    s.framebuffer_srgb_enabled = (gl.is_enabled)(GL_FRAMEBUFFER_SRGB);
    s.depth_enabled = (gl.is_enabled)(GL_DEPTH_TEST);
    s.cull_enabled = (gl.is_enabled)(GL_CULL_FACE);

    (gl.get_integer_v)(GL_FRAMEBUFFER_BINDING, &mut s.framebuffer);
    (gl.get_integer_v)(GL_BLEND_SRC_RGB, &mut s.blend_src_rgb);
    (gl.get_integer_v)(GL_BLEND_DST_RGB, &mut s.blend_dst_rgb);
    (gl.get_integer_v)(GL_BLEND_SRC_ALPHA, &mut s.blend_src_alpha);
    (gl.get_integer_v)(GL_BLEND_DST_ALPHA, &mut s.blend_dst_alpha);
    (gl.get_integer_v)(GL_BLEND_EQUATION_RGB, &mut s.blend_eq_rgb);
    (gl.get_integer_v)(GL_BLEND_EQUATION_ALPHA, &mut s.blend_eq_alpha);

    s
}

// Fix up states that the fast restore got wrong based on what we sampled
pub unsafe fn restore_targeted_corrections(gl: &GlFns, s: &TargetedGlState) {
    // sRGB: fast restore leaves it off
    if s.framebuffer_srgb_enabled != GL_FALSE {
        (gl.enable)(GL_FRAMEBUFFER_SRGB);
    }

    // FBO: fast restore binds 0 (default framebuffer)
    if s.framebuffer != 0 {
        (gl.bind_framebuffer)(GL_FRAMEBUFFER, s.framebuffer as GLuint);
    }

    // depth: fast restore turns it on
    if s.depth_enabled == GL_FALSE {
        (gl.disable)(GL_DEPTH_TEST);
    }

    // cull face: fast restore turns it on
    if s.cull_enabled == GL_FALSE {
        (gl.disable)(GL_CULL_FACE);
    }

    // blend func: fast restore hardcodes standard alpha blend
    let default_blend =
        s.blend_src_rgb == GL_SRC_ALPHA as GLint
        && s.blend_dst_rgb == GL_ONE_MINUS_SRC_ALPHA as GLint
        && s.blend_src_alpha == GL_ONE as GLint
        && s.blend_dst_alpha == GL_ZERO as GLint;
    if !default_blend {
        (gl.blend_func_separate)(
            s.blend_src_rgb as GLenum,
            s.blend_dst_rgb as GLenum,
            s.blend_src_alpha as GLenum,
            s.blend_dst_alpha as GLenum,
        );
    }

    // blend equation: compositor sets FUNC_ADD, fast restore doesn't touch it
    if s.blend_eq_rgb != GL_FUNC_ADD as GLint
        || s.blend_eq_alpha != GL_FUNC_ADD as GLint
    {
        (gl.blend_equation_separate)(
            s.blend_eq_rgb as GLenum,
            s.blend_eq_alpha as GLenum,
        );
    }
}

// ---- Full path (query everything, restore everything) ----

pub struct SavedGlState {
    pub program: GLint,
    pub active_texture: GLint,
    pub texture_2d: GLint,

    pub vao: GLint,
    pub array_buffer: GLint,
    pub element_array_buffer: GLint,

    pub blend_on: GLboolean,
    pub blend_src_rgb: GLint,
    pub blend_dst_rgb: GLint,
    pub blend_src_alpha: GLint,
    pub blend_dst_alpha: GLint,
    pub blend_eq_rgb: GLint,
    pub blend_eq_alpha: GLint,

    pub depth_on: GLboolean,
    pub stencil_on: GLboolean,
    pub cull_on: GLboolean,
    pub scissor_on: GLboolean,

    pub viewport: [GLint; 4],
    pub scissor_box: [GLint; 4],
    pub color_mask: [GLboolean; 4],
    pub framebuffer: GLint,
    pub read_framebuffer: GLint,
    pub srgb_on: GLboolean,
    pub unpack_row_length: GLint,
    pub unpack_skip_rows: GLint,
    pub unpack_skip_pixels: GLint,
    pub unpack_alignment: GLint,
    pub pixel_unpack_buffer: GLint,
    pub pixel_pack_buffer: GLint,

    // depth state — depth_test disabled alone doesn't reset these
    pub depth_func: GLint,
    pub depth_writemask: GLboolean,

    // stencil state (front + back) — disable alone doesn't reset func/op/mask
    pub stencil_func: GLint,
    pub stencil_ref: GLint,
    pub stencil_value_mask: GLint,
    pub stencil_writemask: GLint,
    pub stencil_fail: GLint,
    pub stencil_pass_depth_fail: GLint,
    pub stencil_pass_depth_pass: GLint,
    pub stencil_back_func: GLint,
    pub stencil_back_ref: GLint,
    pub stencil_back_value_mask: GLint,
    pub stencil_back_writemask: GLint,
    pub stencil_back_fail: GLint,
    pub stencil_back_pass_depth_fail: GLint,
    pub stencil_back_pass_depth_pass: GLint,

    pub blend_color: [GLfloat; 4],
}

pub unsafe fn save_gl_state(gl: &GlFns) -> SavedGlState {
    let mut st = SavedGlState {
        program: 0,
        active_texture: 0,
        texture_2d: 0,
        vao: 0,
        array_buffer: 0,
        element_array_buffer: 0,
        blend_on: GL_FALSE,
        blend_src_rgb: 0,
        blend_dst_rgb: 0,
        blend_src_alpha: 0,
        blend_dst_alpha: 0,
        blend_eq_rgb: 0,
        blend_eq_alpha: 0,
        depth_on: GL_FALSE,
        stencil_on: GL_FALSE,
        cull_on: GL_FALSE,
        scissor_on: GL_FALSE,
        viewport: [0; 4],
        scissor_box: [0; 4],
        color_mask: [GL_TRUE; 4],
        framebuffer: 0,
        read_framebuffer: 0,
        srgb_on: GL_FALSE,
        unpack_row_length: 0,
        unpack_skip_rows: 0,
        unpack_skip_pixels: 0,
        unpack_alignment: 4,
        pixel_unpack_buffer: 0,
        pixel_pack_buffer: 0,
        depth_func: 0x0201, // GL_LESS
        depth_writemask: GL_TRUE,
        stencil_func: 0x0207, // GL_ALWAYS
        stencil_ref: 0,
        stencil_value_mask: !0,
        stencil_writemask: !0,
        stencil_fail: 0x1E00,           // GL_KEEP
        stencil_pass_depth_fail: 0x1E00, // GL_KEEP
        stencil_pass_depth_pass: 0x1E00, // GL_KEEP
        stencil_back_func: 0x0207,
        stencil_back_ref: 0,
        stencil_back_value_mask: !0,
        stencil_back_writemask: !0,
        stencil_back_fail: 0x1E00,
        stencil_back_pass_depth_fail: 0x1E00,
        stencil_back_pass_depth_pass: 0x1E00,
        blend_color: [0.0; 4],
    };

    (gl.get_integer_v)(GL_CURRENT_PROGRAM, &mut st.program);
    (gl.get_integer_v)(GL_ACTIVE_TEXTURE as GLenum, &mut st.active_texture);

    // need to switch to TEXTURE0 to read its binding
    (gl.active_texture)(GL_TEXTURE0);
    (gl.get_integer_v)(GL_TEXTURE_BINDING_2D, &mut st.texture_2d);

    (gl.get_integer_v)(GL_VERTEX_ARRAY_BINDING, &mut st.vao);
    (gl.get_integer_v)(GL_ARRAY_BUFFER_BINDING, &mut st.array_buffer);
    (gl.get_integer_v)(GL_ELEMENT_ARRAY_BUFFER_BINDING, &mut st.element_array_buffer);

    st.blend_on = (gl.is_enabled)(GL_BLEND);
    (gl.get_integer_v)(GL_BLEND_SRC_RGB, &mut st.blend_src_rgb);
    (gl.get_integer_v)(GL_BLEND_DST_RGB, &mut st.blend_dst_rgb);
    (gl.get_integer_v)(GL_BLEND_SRC_ALPHA, &mut st.blend_src_alpha);
    (gl.get_integer_v)(GL_BLEND_DST_ALPHA, &mut st.blend_dst_alpha);
    (gl.get_integer_v)(GL_BLEND_EQUATION_RGB, &mut st.blend_eq_rgb);
    (gl.get_integer_v)(GL_BLEND_EQUATION_ALPHA, &mut st.blend_eq_alpha);

    st.depth_on = (gl.is_enabled)(GL_DEPTH_TEST);
    st.stencil_on = (gl.is_enabled)(GL_STENCIL_TEST);
    st.cull_on = (gl.is_enabled)(GL_CULL_FACE);
    st.scissor_on = (gl.is_enabled)(GL_SCISSOR_TEST);

    (gl.get_integer_v)(GL_VIEWPORT, st.viewport.as_mut_ptr());
    (gl.get_integer_v)(GL_SCISSOR_BOX, st.scissor_box.as_mut_ptr());

    // color mask comes back as ints, cast to booleans
    let mut cm = [0i32; 4];
    (gl.get_integer_v)(GL_COLOR_WRITEMASK, cm.as_mut_ptr());
    st.color_mask = [cm[0] as u8, cm[1] as u8, cm[2] as u8, cm[3] as u8];

    (gl.get_integer_v)(GL_FRAMEBUFFER_BINDING, &mut st.framebuffer);
    (gl.get_integer_v)(GL_READ_FRAMEBUFFER_BINDING, &mut st.read_framebuffer);
    st.srgb_on = (gl.is_enabled)(GL_FRAMEBUFFER_SRGB);

    (gl.get_integer_v)(GL_UNPACK_ROW_LENGTH, &mut st.unpack_row_length);
    (gl.get_integer_v)(GL_UNPACK_SKIP_ROWS, &mut st.unpack_skip_rows);
    (gl.get_integer_v)(GL_UNPACK_SKIP_PIXELS, &mut st.unpack_skip_pixels);
    (gl.get_integer_v)(GL_UNPACK_ALIGNMENT, &mut st.unpack_alignment);
    (gl.get_integer_v)(GL_PIXEL_UNPACK_BUFFER_BINDING, &mut st.pixel_unpack_buffer);
    (gl.get_integer_v)(GL_PIXEL_PACK_BUFFER_BINDING, &mut st.pixel_pack_buffer);

    (gl.get_integer_v)(GL_DEPTH_FUNC, &mut st.depth_func);
    let mut depth_wm = 0i32;
    (gl.get_integer_v)(GL_DEPTH_WRITEMASK, &mut depth_wm);
    st.depth_writemask = if depth_wm != 0 { GL_TRUE } else { GL_FALSE };

    (gl.get_integer_v)(GL_STENCIL_FUNC, &mut st.stencil_func);
    (gl.get_integer_v)(GL_STENCIL_REF, &mut st.stencil_ref);
    (gl.get_integer_v)(GL_STENCIL_VALUE_MASK, &mut st.stencil_value_mask);
    (gl.get_integer_v)(GL_STENCIL_WRITEMASK, &mut st.stencil_writemask);
    (gl.get_integer_v)(GL_STENCIL_FAIL, &mut st.stencil_fail);
    (gl.get_integer_v)(GL_STENCIL_PASS_DEPTH_FAIL, &mut st.stencil_pass_depth_fail);
    (gl.get_integer_v)(GL_STENCIL_PASS_DEPTH_PASS, &mut st.stencil_pass_depth_pass);
    (gl.get_integer_v)(GL_STENCIL_BACK_FUNC, &mut st.stencil_back_func);
    (gl.get_integer_v)(GL_STENCIL_BACK_REF, &mut st.stencil_back_ref);
    (gl.get_integer_v)(GL_STENCIL_BACK_VALUE_MASK, &mut st.stencil_back_value_mask);
    (gl.get_integer_v)(GL_STENCIL_BACK_WRITEMASK, &mut st.stencil_back_writemask);
    (gl.get_integer_v)(GL_STENCIL_BACK_FAIL, &mut st.stencil_back_fail);
    (gl.get_integer_v)(GL_STENCIL_BACK_PASS_DEPTH_FAIL, &mut st.stencil_back_pass_depth_fail);
    (gl.get_integer_v)(GL_STENCIL_BACK_PASS_DEPTH_PASS, &mut st.stencil_back_pass_depth_pass);

    (gl.get_float_v)(GL_BLEND_COLOR, st.blend_color.as_mut_ptr());

    st
}

pub unsafe fn restore_gl_state(gl: &GlFns, st: &SavedGlState) {
    (gl.use_program)(st.program as GLuint);

    (gl.active_texture)(GL_TEXTURE0);
    (gl.bind_texture)(GL_TEXTURE_2D, st.texture_2d as GLuint);
    (gl.active_texture)(st.active_texture as GLenum);

    (gl.bind_vertex_array)(st.vao as GLuint);
    (gl.bind_buffer)(GL_ARRAY_BUFFER, st.array_buffer as GLuint);
    (gl.bind_buffer)(GL_ELEMENT_ARRAY_BUFFER, st.element_array_buffer as GLuint);

    if st.blend_on != GL_FALSE { (gl.enable)(GL_BLEND); }
    else { (gl.disable)(GL_BLEND); }
    (gl.blend_func_separate)(
        st.blend_src_rgb as GLenum,
        st.blend_dst_rgb as GLenum,
        st.blend_src_alpha as GLenum,
        st.blend_dst_alpha as GLenum,
    );
    (gl.blend_equation_separate)(
        st.blend_eq_rgb as GLenum,
        st.blend_eq_alpha as GLenum,
    );

    if st.depth_on != GL_FALSE { (gl.enable)(GL_DEPTH_TEST); }
    else { (gl.disable)(GL_DEPTH_TEST); }

    if st.stencil_on != GL_FALSE { (gl.enable)(GL_STENCIL_TEST); }
    else { (gl.disable)(GL_STENCIL_TEST); }

    if st.cull_on != GL_FALSE { (gl.enable)(GL_CULL_FACE); }
    else { (gl.disable)(GL_CULL_FACE); }

    if st.scissor_on != GL_FALSE { (gl.enable)(GL_SCISSOR_TEST); }
    else { (gl.disable)(GL_SCISSOR_TEST); }

    (gl.viewport)(st.viewport[0], st.viewport[1], st.viewport[2], st.viewport[3]);
    (gl.scissor)(st.scissor_box[0], st.scissor_box[1], st.scissor_box[2], st.scissor_box[3]);

    (gl.color_mask)(
        st.color_mask[0], st.color_mask[1],
        st.color_mask[2], st.color_mask[3],
    );

    // restore read + draw FBO separately. MC may have different bindings
    // mid-blit; binding GL_FRAMEBUFFER alone clobbers both to the same value.
    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, st.framebuffer as GLuint);
    (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, st.read_framebuffer as GLuint);

    if st.srgb_on != GL_FALSE { (gl.enable)(GL_FRAMEBUFFER_SRGB); }
    else { (gl.disable)(GL_FRAMEBUFFER_SRGB); }

    (gl.pixel_store_i)(GL_UNPACK_ROW_LENGTH, st.unpack_row_length);
    (gl.pixel_store_i)(GL_UNPACK_SKIP_ROWS, st.unpack_skip_rows);
    (gl.pixel_store_i)(GL_UNPACK_SKIP_PIXELS, st.unpack_skip_pixels);
    (gl.pixel_store_i)(GL_UNPACK_ALIGNMENT, st.unpack_alignment);

    // PBO bindings — MC's font atlas upload uses glTexSubImage2D with a CPU
    // pointer. If we leave any PBO bound to PIXEL_UNPACK_BUFFER, MC's next
    // upload reads from the PBO instead of CPU memory → corrupt glyph data.
    (gl.bind_buffer)(GL_PIXEL_UNPACK_BUFFER, st.pixel_unpack_buffer as GLuint);
    (gl.bind_buffer)(0x88EB /* GL_PIXEL_PACK_BUFFER */, st.pixel_pack_buffer as GLuint);

    (gl.depth_func)(st.depth_func as GLenum);
    (gl.depth_mask)(st.depth_writemask);

    (gl.stencil_func_separate)(
        GL_FRONT,
        st.stencil_func as GLenum,
        st.stencil_ref,
        st.stencil_value_mask as GLuint,
    );
    (gl.stencil_op_separate)(
        GL_FRONT,
        st.stencil_fail as GLenum,
        st.stencil_pass_depth_fail as GLenum,
        st.stencil_pass_depth_pass as GLenum,
    );
    (gl.stencil_mask_separate)(GL_FRONT, st.stencil_writemask as GLuint);

    (gl.stencil_func_separate)(
        GL_BACK,
        st.stencil_back_func as GLenum,
        st.stencil_back_ref,
        st.stencil_back_value_mask as GLuint,
    );
    (gl.stencil_op_separate)(
        GL_BACK,
        st.stencil_back_fail as GLenum,
        st.stencil_back_pass_depth_fail as GLenum,
        st.stencil_back_pass_depth_pass as GLenum,
    );
    (gl.stencil_mask_separate)(GL_BACK, st.stencil_back_writemask as GLuint);

    (gl.blend_color)(
        st.blend_color[0],
        st.blend_color[1],
        st.blend_color[2],
        st.blend_color[3],
    );
}
