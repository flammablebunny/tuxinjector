// GL compositor -- blits an overlay texture on top of the game's backbuffer.
//
// Two paths: zero-copy Vulkan interop (GL_EXT_memory_object_fd) or plain
// CPU pixel upload via glTexSubImage2D for drivers that lack the extension.

use std::ffi::c_void;
use std::ptr;

use tracing;

use crate::gl_bindings::*;
use crate::gl_state;

const COMPOSITE_VERT_SRC: &str = r#"#version 300 es
precision highp float;

layout(location = 0) in vec2 aPos;
layout(location = 1) in vec2 aTexCoord;

out vec2 vTexCoord;

void main() {
    vTexCoord = aTexCoord;
    gl_Position = vec4(aPos, 0.0, 1.0);
}
"#;

const COMPOSITE_FRAG_SRC: &str = r#"#version 300 es
precision highp float;

uniform sampler2D uTexture;

in vec2 vTexCoord;
out vec4 FragColor;

void main() {
    vec4 color = texture(uTexture, vTexCoord);
    // kill fully transparent pixels so they don't mess with sRGB blending
    if (color.a < 0.004) discard;
    FragColor = color;
}
"#;

// Fullscreen quad: [x, y, u, v] per vertex, UV v-flipped for GL's bottom-up orientation
#[rustfmt::skip]
const QUAD_VERTS: [f32; 24] = [
    -1.0, -1.0,  0.0, 1.0,
     1.0, -1.0,  1.0, 1.0,
     1.0,  1.0,  1.0, 0.0,
    -1.0, -1.0,  0.0, 1.0,
     1.0,  1.0,  1.0, 0.0,
    -1.0,  1.0,  0.0, 0.0,
];

// Composites our overlay texture onto the game's GL backbuffer
pub struct GlCompositor {
    tex: GLuint,
    mem_obj: GLuint,
    vao: GLuint,
    vbo: GLuint,
    program: GLuint,
    interop: bool,
    width: u32,
    height: u32,
}

impl GlCompositor {
    // Create using Vulkan->GL zero-copy interop (EXT_memory_object_fd)
    pub unsafe fn new_interop(
        gl: &GlFns,
        fd: i32,
        alloc_size: u64,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let create_mem = gl
            .create_memory_objects_ext
            .ok_or("glCreateMemoryObjectsEXT not available")?;
        let import_fd = gl
            .import_memory_fd_ext
            .ok_or("glImportMemoryFdEXT not available")?;
        let tex_storage_mem = gl
            .tex_storage_mem_2d_ext
            .ok_or("glTexStorageMem2DEXT not available")?;

        // flush any leftover errors from the game
        while (gl.get_error)() != 0 {}

        let mut mem_obj: GLuint = 0;
        (create_mem)(1, &mut mem_obj);
        if mem_obj == 0 {
            return Err("glCreateMemoryObjectsEXT returned 0".into());
        }

        // NOTE: GL takes ownership of the fd after this call
        (import_fd)(mem_obj, alloc_size, GL_HANDLE_TYPE_OPAQUE_FD_EXT, fd);

        let err = (gl.get_error)();
        if err != 0 {
            tracing::warn!(gl_error = err, "GL error after glImportMemoryFdEXT");
            return Err(format!("glImportMemoryFdEXT failed with GL error {err}"));
        }

        let mut tex: GLuint = 0;
        (gl.gen_textures)(1, &mut tex);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);

        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);

        (tex_storage_mem)(
            GL_TEXTURE_2D, 1, GL_RGBA8,
            width as GLsizei, height as GLsizei,
            mem_obj, 0,
        );

        let err = (gl.get_error)();
        if err != 0 {
            tracing::warn!(gl_error = err, "GL error after glTexStorageMem2DEXT");
            return Err(format!("glTexStorageMem2DEXT failed with GL error {err}"));
        }

        (gl.bind_texture)(GL_TEXTURE_2D, 0);

        let program = link_program(gl, COMPOSITE_VERT_SRC, COMPOSITE_FRAG_SRC)?;
        let (vao, vbo) = make_quad_vao(gl);

        tracing::info!(width, height, texture = tex, "compositor created (interop path)");

        Ok(Self {
            tex,
            mem_obj,
            vao,
            vbo,
            program,
            interop: true,
            width,
            height,
        })
    }

    // Create a fallback compositor that uploads pixels from the CPU each frame
    pub unsafe fn new_fallback(
        gl: &GlFns,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let mut tex: GLuint = 0;
        (gl.gen_textures)(1, &mut tex);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);

        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);

        (gl.tex_image_2d)(
            GL_TEXTURE_2D, 0, GL_RGBA8 as GLint,
            width as GLsizei, height as GLsizei, 0,
            GL_RGBA, GL_UNSIGNED_BYTE, ptr::null(),
        );

        (gl.bind_texture)(GL_TEXTURE_2D, 0);

        let program = link_program(gl, COMPOSITE_VERT_SRC, COMPOSITE_FRAG_SRC)?;
        let (vao, vbo) = make_quad_vao(gl);

        tracing::info!(width, height, texture = tex, "compositor created (fallback path)");

        Ok(Self {
            tex,
            mem_obj: 0,
            vao,
            vbo,
            program,
            interop: false,
            width,
            height,
        })
    }

    // Upload new pixel data. No-op when using interop path.
    pub unsafe fn update_fallback_pixels(
        &self,
        gl: &GlFns,
        data: *const u8,
        width: u32,
        height: u32,
    ) {
        if self.interop {
            return;
        }

        // save & reset pixel unpack state -- Sodium/Iris like to leave
        // non-default values here and it corrupts our upload
        let mut prev_row_len = 0i32;
        let mut prev_skip_rows = 0i32;
        let mut prev_skip_px = 0i32;
        let mut prev_align = 4i32;
        (gl.get_integer_v)(GL_UNPACK_ROW_LENGTH, &mut prev_row_len);
        (gl.get_integer_v)(GL_UNPACK_SKIP_ROWS, &mut prev_skip_rows);
        (gl.get_integer_v)(GL_UNPACK_SKIP_PIXELS, &mut prev_skip_px);
        (gl.get_integer_v)(GL_UNPACK_ALIGNMENT, &mut prev_align);
        (gl.pixel_store_i)(GL_UNPACK_ROW_LENGTH, 0);
        (gl.pixel_store_i)(GL_UNPACK_SKIP_ROWS, 0);
        (gl.pixel_store_i)(GL_UNPACK_SKIP_PIXELS, 0);
        (gl.pixel_store_i)(GL_UNPACK_ALIGNMENT, 4);

        (gl.bind_texture)(GL_TEXTURE_2D, self.tex);
        (gl.tex_sub_image_2d)(
            GL_TEXTURE_2D, 0, 0, 0,
            width as GLsizei, height as GLsizei,
            GL_RGBA, GL_UNSIGNED_BYTE,
            data as *const c_void,
        );
        (gl.bind_texture)(GL_TEXTURE_2D, 0);

        // put it back how we found it
        (gl.pixel_store_i)(GL_UNPACK_ROW_LENGTH, prev_row_len);
        (gl.pixel_store_i)(GL_UNPACK_SKIP_ROWS, prev_skip_rows);
        (gl.pixel_store_i)(GL_UNPACK_SKIP_PIXELS, prev_skip_px);
        (gl.pixel_store_i)(GL_UNPACK_ALIGNMENT, prev_align);
    }

    // Fast composite - skips glGet* queries, uses known game state at swap time
    pub unsafe fn composite_fast(&self, gl: &GlFns, viewport: [i32; 4]) {
        gl_state::set_compositor_state(gl, viewport);

        (gl.use_program)(self.program);
        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, self.tex);

        let loc = (gl.get_uniform_location)(
            self.program,
            b"uTexture\0".as_ptr() as *const _,
        );
        if loc >= 0 {
            (gl.uniform_1i)(loc, 0);
        }

        (gl.bind_vertex_array)(self.vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 6);

        gl_state::restore_minecraft_state(gl, viewport);
    }

    // Full save/restore composite - slower but safe when we don't know game state
    pub unsafe fn composite(&self, gl: &GlFns) {
        let saved = gl_state::save_gl_state(gl);

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
        (gl.use_program)(self.program);
        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, self.tex);

        let loc = (gl.get_uniform_location)(
            self.program,
            b"uTexture\0".as_ptr() as *const _,
        );
        if loc >= 0 {
            (gl.uniform_1i)(loc, 0);
        }

        (gl.bind_vertex_array)(self.vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 6);
        (gl.bind_vertex_array)(0);
        (gl.bind_texture)(GL_TEXTURE_2D, 0);
        (gl.use_program)(0);

        gl_state::restore_gl_state(gl, &saved);
    }

    pub fn using_interop(&self) -> bool {
        self.interop
    }

    pub fn texture_id(&self) -> GLuint {
        self.tex
    }

    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }

    pub unsafe fn destroy(&mut self, gl: &GlFns) {
        if self.program != 0 {
            (gl.delete_program)(self.program);
            self.program = 0;
        }
        if self.vbo != 0 {
            (gl.delete_buffers)(1, &self.vbo);
            self.vbo = 0;
        }
        if self.vao != 0 {
            (gl.delete_vertex_arrays)(1, &self.vao);
            self.vao = 0;
        }
        if self.tex != 0 {
            (gl.delete_textures)(1, &self.tex);
            self.tex = 0;
        }
        if self.mem_obj != 0 {
            if let Some(delete_mem) = gl.delete_memory_objects_ext {
                (delete_mem)(1, &self.mem_obj);
            }
            self.mem_obj = 0;
        }
    }
}


// Compile a single shader stage

// macOS GL 2.1 only understands GLSL 1.20, our shaders are 300 ES.
// Brute-force string replace to downgrade. Ugly but it works.
#[cfg(target_os = "macos")]
fn patch_glsl_for_macos(source: &str, shader_type: GLenum) -> String {
    let is_frag = shader_type == GL_FRAGMENT_SHADER;
    let mut s = source.to_string();

    // downgrade version line + enable gpu_shader4 so gl_VertexID / textureSize work
    s = s.replace(
        "#version 300 es\nprecision highp float;",
        "#version 120\n#extension GL_EXT_gpu_shader4 : enable",
    );
    // 1.20 doesn't have precision qualifiers
    s = s.replace("precision highp float;\n", "");
    s = s.replace("precision mediump float;\n", "");

    // no layout(location=N) in 1.20, use plain attribute
    s = s.replace("layout(location = 0) in ", "attribute ");
    s = s.replace("layout(location = 1) in ", "attribute ");
    s = s.replace("layout(location = 2) in ", "attribute ");

    if is_frag {
        // prefix with \n so we don't accidentally clobber function params
        s = s.replace("\nin vec2 ", "\nvarying vec2 ");
        s = s.replace("\nin vec3 ", "\nvarying vec3 ");
        s = s.replace("\nin vec4 ", "\nvarying vec4 ");
        // 1.20 writes to gl_FragColor, not a custom out variable
        s = s.replace("out vec4 FragColor;\n", "");
        s = s.replace("FragColor", "gl_FragColor");
        // gpu_shader4 spells it textureSize2D
        s = s.replace("textureSize(", "textureSize2D(");
        // NOTE: must come after textureSize replace or we'd get texture2DSize2D
        s = s.replace("texture(", "texture2D(");
    } else {
        s = s.replace("\nout vec2 ", "\nvarying vec2 ");
        s = s.replace("\nout vec3 ", "\nvarying vec3 ");
        s = s.replace("\nout vec4 ", "\nvarying vec4 ");
    }

    s
}

pub(crate) unsafe fn compile_shader(
    gl: &GlFns,
    shader_type: GLenum,
    source: &str,
) -> Result<GLuint, String> {
    let shader = (gl.create_shader)(shader_type);
    if shader == 0 {
        return Err("glCreateShader returned 0".into());
    }

    #[cfg(target_os = "macos")]
    let source = patch_glsl_for_macos(source, shader_type);

    let src_ptr = source.as_ptr() as *const GLchar;
    let src_len = source.len() as GLint;
    (gl.shader_source)(shader, 1, &src_ptr, &src_len);
    (gl.compile_shader)(shader);

    let mut ok: GLint = 0;
    (gl.get_shader_iv)(shader, GL_COMPILE_STATUS, &mut ok);

    if ok == 0 {
        let mut log_len: GLint = 0;
        (gl.get_shader_iv)(shader, GL_INFO_LOG_LENGTH, &mut log_len);

        let mut buf = vec![0u8; log_len.max(1) as usize];
        (gl.get_shader_info_log)(
            shader, log_len, ptr::null_mut(),
            buf.as_mut_ptr() as *mut GLchar,
        );

        let msg = String::from_utf8_lossy(&buf);
        (gl.delete_shader)(shader);
        return Err(format!("shader compilation failed: {msg}"));
    }

    Ok(shader)
}

// Link a vertex + fragment shader into a program
pub(crate) unsafe fn link_program(gl: &GlFns, vert_src: &str, frag_src: &str) -> Result<GLuint, String> {
    let vs = compile_shader(gl, GL_VERTEX_SHADER, vert_src)?;
    let fs = compile_shader(gl, GL_FRAGMENT_SHADER, frag_src)?;

    let prog = (gl.create_program)();
    if prog == 0 {
        (gl.delete_shader)(vs);
        (gl.delete_shader)(fs);
        return Err("glCreateProgram returned 0".into());
    }

    (gl.attach_shader)(prog, vs);
    (gl.attach_shader)(prog, fs);

    // GLSL 1.20 (macOS) doesn't have layout(location=N) so we bind
    // attribs manually. Harmless no-op on Linux where layouts already work.
    (gl.bind_attrib_location)(prog, 0, b"aPos\0".as_ptr() as *const _);
    (gl.bind_attrib_location)(prog, 1, b"aTexCoord\0".as_ptr() as *const _);

    (gl.link_program)(prog);

    // shaders can be deleted right after linking
    (gl.delete_shader)(vs);
    (gl.delete_shader)(fs);

    let mut ok: GLint = 0;
    (gl.get_program_iv)(prog, GL_LINK_STATUS, &mut ok);
    if ok == 0 {
        let mut log_len: GLint = 0;
        (gl.get_program_iv)(prog, GL_INFO_LOG_LENGTH, &mut log_len);

        let mut buf = vec![0u8; log_len.max(1) as usize];
        (gl.get_program_info_log)(
            prog, log_len, ptr::null_mut(),
            buf.as_mut_ptr() as *mut GLchar,
        );

        let msg = String::from_utf8_lossy(&buf);
        (gl.delete_program)(prog);
        return Err(format!("program link failed: {msg}"));
    }

    Ok(prog)
}

// Set up a VAO+VBO for our fullscreen composite quad
unsafe fn make_quad_vao(gl: &GlFns) -> (GLuint, GLuint) {
    let mut vao: GLuint = 0;
    let mut vbo: GLuint = 0;

    (gl.gen_vertex_arrays)(1, &mut vao);
    (gl.gen_buffers)(1, &mut vbo);

    (gl.bind_vertex_array)(vao);
    (gl.bind_buffer)(GL_ARRAY_BUFFER, vbo);

    let size = std::mem::size_of_val(&QUAD_VERTS) as GLsizeiptr;
    (gl.buffer_data)(
        GL_ARRAY_BUFFER, size,
        QUAD_VERTS.as_ptr() as *const c_void,
        GL_STATIC_DRAW,
    );

    let stride = (4 * std::mem::size_of::<f32>()) as GLsizei;

    // attrib 0: position (xy)
    (gl.enable_vertex_attrib_array)(0);
    (gl.vertex_attrib_pointer)(0, 2, GL_FLOAT, GL_FALSE, stride, ptr::null());

    // attrib 1: texcoord (uv)
    (gl.enable_vertex_attrib_array)(1);
    (gl.vertex_attrib_pointer)(
        1, 2, GL_FLOAT, GL_FALSE, stride,
        (2 * std::mem::size_of::<f32>()) as *const c_void,
    );

    (gl.bind_vertex_array)(0);
    (gl.bind_buffer)(GL_ARRAY_BUFFER, 0);

    (vao, vbo)
}
