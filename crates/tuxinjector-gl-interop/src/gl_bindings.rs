// More LLM code, i know, i know. I swear im not always this lazy
//
// ===========================================================================
// Module: gl_bindings — OpenGL Constants, Types & Function Pointer Typedefs
// ===========================================================================
//
// Hand-rolled GL bindings rather than a full binding crate — we only
// require a small subset, and must avoid symbol collisions with the
// game's own GL loader.

use std::ffi::{c_char, c_void};

// -- Type Aliases --------------------------------------------------------

#[allow(non_camel_case_types)]
pub type GLenum = u32;
#[allow(non_camel_case_types)]
pub type GLuint = u32;
#[allow(non_camel_case_types)]
pub type GLint = i32;
#[allow(non_camel_case_types)]
pub type GLsizei = i32;
#[allow(non_camel_case_types)]
pub type GLboolean = u8;
#[allow(non_camel_case_types)]
pub type GLbitfield = u32;
#[allow(non_camel_case_types)]
pub type GLfloat = f32;
#[allow(non_camel_case_types)]
pub type GLsizeiptr = isize;
#[allow(non_camel_case_types)]
pub type GLchar = c_char;
#[allow(non_camel_case_types)]
pub type GLuint64 = u64;

// -- GL Constants --------------------------------------------------------

pub const GL_TEXTURE_2D: GLenum = 0x0DE1;

pub const GL_RGBA: GLenum = 0x1908;
pub const GL_RGBA8: GLenum = 0x8058;
pub const GL_UNSIGNED_BYTE: GLenum = 0x1401;

pub const GL_BLEND: GLenum = 0x0BE2;
pub const GL_DEPTH_TEST: GLenum = 0x0B71;
pub const GL_SCISSOR_TEST: GLenum = 0x0C11;
pub const GL_STENCIL_TEST: GLenum = 0x0B90;
pub const GL_CULL_FACE: GLenum = 0x0B44;

pub const GL_SRC_ALPHA: GLenum = 0x0302;
pub const GL_ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
pub const GL_ZERO: GLenum = 0;
pub const GL_ONE: GLenum = 1;

pub const GL_FUNC_ADD: GLenum = 0x8006;
pub const GL_BLEND_EQUATION_RGB: GLenum = 0x8009;
pub const GL_BLEND_EQUATION_ALPHA: GLenum = 0x883D;

pub const GL_TRIANGLES: GLenum = 0x0004;
pub const GL_TRIANGLE_STRIP: GLenum = 0x0005;

pub const GL_ARRAY_BUFFER: GLenum = 0x8892;
pub const GL_ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;

pub const GL_STATIC_DRAW: GLenum = 0x88E4;
pub const GL_DYNAMIC_DRAW: GLenum = 0x88E8;

pub const GL_VERTEX_SHADER: GLenum = 0x8B31;
pub const GL_FRAGMENT_SHADER: GLenum = 0x8B30;

pub const GL_COMPILE_STATUS: GLenum = 0x8B81;
pub const GL_LINK_STATUS: GLenum = 0x8B82;
pub const GL_INFO_LOG_LENGTH: GLenum = 0x8B84;

pub const GL_FLOAT: GLenum = 0x1406;

pub const GL_TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub const GL_TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub const GL_TEXTURE_WRAP_S: GLenum = 0x2802;
pub const GL_TEXTURE_WRAP_T: GLenum = 0x2803;
pub const GL_NEAREST: GLint = 0x2600;
pub const GL_LINEAR: GLint = 0x2601;
pub const GL_CLAMP_TO_EDGE: GLint = 0x812F;
pub const GL_CLAMP_TO_BORDER: GLint = 0x812D;

pub const GL_UNPACK_ROW_LENGTH: GLenum = 0x0CF2;
pub const GL_UNPACK_SKIP_ROWS: GLenum = 0x0CF3;
pub const GL_UNPACK_SKIP_PIXELS: GLenum = 0x0CF4;
pub const GL_UNPACK_ALIGNMENT: GLenum = 0x0CF5;

pub const GL_CURRENT_PROGRAM: GLenum = 0x8B8D;
pub const GL_TEXTURE_BINDING_2D: GLenum = 0x8069;
pub const GL_ACTIVE_TEXTURE: GLenum = 0x84E0;
pub const GL_ARRAY_BUFFER_BINDING: GLenum = 0x8894;
pub const GL_ELEMENT_ARRAY_BUFFER_BINDING: GLenum = 0x8895;
pub const GL_VERTEX_ARRAY_BINDING: GLenum = 0x85B5;
pub const GL_VIEWPORT: GLenum = 0x0BA2;
pub const GL_SCISSOR_BOX: GLenum = 0x0C10;
pub const GL_BLEND_DST_RGB: GLenum = 0x80C8;
pub const GL_BLEND_SRC_RGB: GLenum = 0x80C9;
pub const GL_BLEND_DST_ALPHA: GLenum = 0x80CA;
pub const GL_BLEND_SRC_ALPHA: GLenum = 0x80CB;
pub const GL_FRAMEBUFFER_BINDING: GLenum = 0x8CA6;
pub const GL_FRAMEBUFFER_SRGB: GLenum = 0x8DB9;
pub const GL_COLOR_WRITEMASK: GLenum = 0x0C23;

// Additional state we save/restore so MC 1.21+ RenderSystem caches stay in
// sync with actual GL state. Without these, certain glyphs (numbers/letters) / draw calls
// in game/chat/debug menus silently render with the wrong state, or dont render at all.
pub const GL_DEPTH_FUNC: GLenum = 0x0B74;
pub const GL_DEPTH_WRITEMASK: GLenum = 0x0B72;
pub const GL_STENCIL_FUNC: GLenum = 0x0B92;
pub const GL_STENCIL_REF: GLenum = 0x0B97;
pub const GL_STENCIL_VALUE_MASK: GLenum = 0x0B93;
pub const GL_STENCIL_WRITEMASK: GLenum = 0x0B98;
pub const GL_STENCIL_FAIL: GLenum = 0x0B94;
pub const GL_STENCIL_PASS_DEPTH_FAIL: GLenum = 0x0B95;
pub const GL_STENCIL_PASS_DEPTH_PASS: GLenum = 0x0B96;
pub const GL_STENCIL_BACK_FUNC: GLenum = 0x8800;
pub const GL_STENCIL_BACK_REF: GLenum = 0x8CA3;
pub const GL_STENCIL_BACK_VALUE_MASK: GLenum = 0x8CA4;
pub const GL_STENCIL_BACK_WRITEMASK: GLenum = 0x8CA5;
pub const GL_STENCIL_BACK_FAIL: GLenum = 0x8801;
pub const GL_STENCIL_BACK_PASS_DEPTH_FAIL: GLenum = 0x8802;
pub const GL_STENCIL_BACK_PASS_DEPTH_PASS: GLenum = 0x8803;
pub const GL_BLEND_COLOR: GLenum = 0x8005;
pub const GL_FRONT: GLenum = 0x0404;
pub const GL_BACK: GLenum = 0x0405;
pub const GL_DRAW_FRAMEBUFFER: GLenum = 0x8CA9;
pub const GL_READ_FRAMEBUFFER: GLenum = 0x8CA8;
pub const GL_READ_FRAMEBUFFER_BINDING: GLenum = 0x8CAA;
pub const GL_PIXEL_UNPACK_BUFFER: GLenum = 0x88EC;
pub const GL_PIXEL_UNPACK_BUFFER_BINDING: GLenum = 0x88EF;
pub const GL_PIXEL_PACK_BUFFER_BINDING: GLenum = 0x88ED;

pub const GL_TEXTURE0: GLenum = 0x84C0;

pub const GL_FRAMEBUFFER: GLenum = 0x8D40;

pub const GL_TRUE: GLboolean = 1;
pub const GL_FALSE: GLboolean = 0;

pub const GL_COLOR_BUFFER_BIT: GLbitfield = 0x00004000;

pub const GL_HANDLE_TYPE_OPAQUE_FD_EXT: GLenum = 0x9586;
pub const GL_TEXTURE_TILING_EXT: GLenum = 0x9580;
pub const GL_OPTIMAL_TILING_EXT: GLenum = 0x9584;

// -- Function Pointer Type Definitions -----------------------------------

macro_rules! gl_fn_type {
    ($name:ident => unsafe fn($($arg:ident : $arg_ty:ty),* $(,)?) $(-> $ret:ty)?) => {
        #[allow(non_camel_case_types, dead_code)]
        pub type $name = unsafe extern "C" fn($($arg : $arg_ty),*) $(-> $ret)?;
    };
}

// Core
gl_fn_type!(PfnGlGetError             => unsafe fn() -> GLenum);
gl_fn_type!(PfnGlGetIntegerv          => unsafe fn(pname: GLenum, data: *mut GLint));
gl_fn_type!(PfnGlGetString            => unsafe fn(name: GLenum) -> *const c_char);

// Textures
gl_fn_type!(PfnGlGenTextures          => unsafe fn(n: GLsizei, textures: *mut GLuint));
gl_fn_type!(PfnGlBindTexture          => unsafe fn(target: GLenum, texture: GLuint));
gl_fn_type!(PfnGlDeleteTextures       => unsafe fn(n: GLsizei, textures: *const GLuint));
gl_fn_type!(PfnGlTexImage2D           => unsafe fn(target: GLenum, level: GLint, internal_format: GLint, width: GLsizei, height: GLsizei, border: GLint, format: GLenum, ty: GLenum, pixels: *const c_void));
gl_fn_type!(PfnGlTexSubImage2D        => unsafe fn(target: GLenum, level: GLint, xoffset: GLint, yoffset: GLint, width: GLsizei, height: GLsizei, format: GLenum, ty: GLenum, pixels: *const c_void));
gl_fn_type!(PfnGlTexParameteri        => unsafe fn(target: GLenum, pname: GLenum, param: GLint));
gl_fn_type!(PfnGlActiveTexture        => unsafe fn(texture: GLenum));
gl_fn_type!(PfnGlBindSampler          => unsafe fn(unit: GLuint, sampler: GLuint));

// State
gl_fn_type!(PfnGlEnable               => unsafe fn(cap: GLenum));
gl_fn_type!(PfnGlDisable              => unsafe fn(cap: GLenum));
gl_fn_type!(PfnGlIsEnabled            => unsafe fn(cap: GLenum) -> GLboolean);
gl_fn_type!(PfnGlBlendFuncSeparate    => unsafe fn(src_rgb: GLenum, dst_rgb: GLenum, src_alpha: GLenum, dst_alpha: GLenum));
gl_fn_type!(PfnGlBlendEquationSeparate => unsafe fn(mode_rgb: GLenum, mode_alpha: GLenum));
gl_fn_type!(PfnGlViewport             => unsafe fn(x: GLint, y: GLint, width: GLsizei, height: GLsizei));
gl_fn_type!(PfnGlScissor              => unsafe fn(x: GLint, y: GLint, width: GLsizei, height: GLsizei));
gl_fn_type!(PfnGlColorMask            => unsafe fn(r: GLboolean, g: GLboolean, b: GLboolean, a: GLboolean));
gl_fn_type!(PfnGlDepthFunc            => unsafe fn(func: GLenum));
gl_fn_type!(PfnGlDepthMask            => unsafe fn(flag: GLboolean));
gl_fn_type!(PfnGlStencilFuncSeparate  => unsafe fn(face: GLenum, func: GLenum, ref_: GLint, mask: GLuint));
gl_fn_type!(PfnGlStencilOpSeparate    => unsafe fn(face: GLenum, sfail: GLenum, dpfail: GLenum, dppass: GLenum));
gl_fn_type!(PfnGlStencilMaskSeparate  => unsafe fn(face: GLenum, mask: GLuint));
gl_fn_type!(PfnGlBlendColor           => unsafe fn(red: GLfloat, green: GLfloat, blue: GLfloat, alpha: GLfloat));
gl_fn_type!(PfnGlGetFloatv            => unsafe fn(pname: GLenum, data: *mut GLfloat));

// Shaders/Programs
gl_fn_type!(PfnGlUseProgram           => unsafe fn(program: GLuint));
gl_fn_type!(PfnGlCreateShader         => unsafe fn(ty: GLenum) -> GLuint);
gl_fn_type!(PfnGlDeleteShader         => unsafe fn(shader: GLuint));
gl_fn_type!(PfnGlShaderSource         => unsafe fn(shader: GLuint, count: GLsizei, string: *const *const GLchar, length: *const GLint));
gl_fn_type!(PfnGlCompileShader        => unsafe fn(shader: GLuint));
gl_fn_type!(PfnGlGetShaderiv          => unsafe fn(shader: GLuint, pname: GLenum, params: *mut GLint));
gl_fn_type!(PfnGlGetShaderInfoLog     => unsafe fn(shader: GLuint, buf_size: GLsizei, length: *mut GLsizei, info_log: *mut GLchar));
gl_fn_type!(PfnGlCreateProgram        => unsafe fn() -> GLuint);
gl_fn_type!(PfnGlDeleteProgram        => unsafe fn(program: GLuint));
gl_fn_type!(PfnGlAttachShader         => unsafe fn(program: GLuint, shader: GLuint));
gl_fn_type!(PfnGlLinkProgram          => unsafe fn(program: GLuint));
gl_fn_type!(PfnGlGetProgramiv         => unsafe fn(program: GLuint, pname: GLenum, params: *mut GLint));
gl_fn_type!(PfnGlGetProgramInfoLog    => unsafe fn(program: GLuint, buf_size: GLsizei, length: *mut GLsizei, info_log: *mut GLchar));
gl_fn_type!(PfnGlGetUniformLocation   => unsafe fn(program: GLuint, name: *const GLchar) -> GLint);
gl_fn_type!(PfnGlUniform1i            => unsafe fn(location: GLint, v0: GLint));
gl_fn_type!(PfnGlUniform1f            => unsafe fn(location: GLint, v0: GLfloat));
gl_fn_type!(PfnGlUniform2f            => unsafe fn(location: GLint, v0: GLfloat, v1: GLfloat));
gl_fn_type!(PfnGlUniform4f            => unsafe fn(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat, v3: GLfloat));

// Clear
gl_fn_type!(PfnGlClear                => unsafe fn(mask: GLbitfield));
gl_fn_type!(PfnGlClearColor           => unsafe fn(red: GLfloat, green: GLfloat, blue: GLfloat, alpha: GLfloat));

// VAO
gl_fn_type!(PfnGlGenVertexArrays      => unsafe fn(n: GLsizei, arrays: *mut GLuint));
gl_fn_type!(PfnGlDeleteVertexArrays   => unsafe fn(n: GLsizei, arrays: *const GLuint));
gl_fn_type!(PfnGlBindVertexArray      => unsafe fn(array: GLuint));

// Buffers
gl_fn_type!(PfnGlGenBuffers           => unsafe fn(n: GLsizei, buffers: *mut GLuint));
gl_fn_type!(PfnGlDeleteBuffers        => unsafe fn(n: GLsizei, buffers: *const GLuint));
gl_fn_type!(PfnGlBindBuffer           => unsafe fn(target: GLenum, buffer: GLuint));
gl_fn_type!(PfnGlBufferData           => unsafe fn(target: GLenum, size: GLsizeiptr, data: *const c_void, usage: GLenum));

// Drawing / vertex attribs
gl_fn_type!(PfnGlDrawArrays           => unsafe fn(mode: GLenum, first: GLint, count: GLsizei));
gl_fn_type!(PfnGlEnableVertexAttribArray  => unsafe fn(index: GLuint));
gl_fn_type!(PfnGlDisableVertexAttribArray => unsafe fn(index: GLuint));
gl_fn_type!(PfnGlVertexAttribPointer  => unsafe fn(index: GLuint, size: GLint, ty: GLenum, normalized: GLboolean, stride: GLsizei, pointer: *const c_void));
gl_fn_type!(PfnGlBindAttribLocation   => unsafe fn(program: GLuint, index: GLuint, name: *const GLchar));

// Framebuffer
gl_fn_type!(PfnGlBindFramebuffer      => unsafe fn(target: GLenum, framebuffer: GLuint));

// EXT_memory_object / EXT_memory_object_fd
gl_fn_type!(PfnGlCreateMemoryObjectsEXT => unsafe fn(n: GLsizei, memory_objects: *mut GLuint));
gl_fn_type!(PfnGlDeleteMemoryObjectsEXT => unsafe fn(n: GLsizei, memory_objects: *const GLuint));
gl_fn_type!(PfnGlImportMemoryFdEXT      => unsafe fn(memory: GLuint, size: GLuint64, handle_type: GLenum, fd: GLint));
gl_fn_type!(PfnGlTexStorageMem2DEXT     => unsafe fn(target: GLenum, levels: GLsizei, internal_format: GLenum, width: GLsizei, height: GLsizei, memory: GLuint, offset: GLuint64));
gl_fn_type!(PfnGlTextureParameteriEXT   => unsafe fn(texture: GLuint, target: GLenum, pname: GLenum, param: GLint));

// Pixel store
gl_fn_type!(PfnGlPixelStorei => unsafe fn(pname: GLenum, param: GLint));

// Sync
gl_fn_type!(PfnGlFinish => unsafe fn());
gl_fn_type!(PfnGlFlush  => unsafe fn());

pub type GetProcAddrFn = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;

// All the GL entry points we use. Core ones panic if missing,
// extension ones are Option so we can gracefully degrade.
#[allow(dead_code)]
pub struct GlFns {
    // Core
    pub get_error: PfnGlGetError,
    pub get_integer_v: PfnGlGetIntegerv,
    pub get_string: PfnGlGetString,

    // Textures
    pub gen_textures: PfnGlGenTextures,
    pub bind_texture: PfnGlBindTexture,
    pub delete_textures: PfnGlDeleteTextures,
    pub tex_image_2d: PfnGlTexImage2D,
    pub tex_sub_image_2d: PfnGlTexSubImage2D,
    pub tex_parameter_i: PfnGlTexParameteri,
    pub active_texture: PfnGlActiveTexture,
    // GL 3.3+: a bound sampler object overrides a texture's own params. MC 1.21's
    // Blaze3D binds these; if we don't clear it our mip-less textures become
    // sampler-incomplete and sample black. Optional in case of an ancient context.
    pub bind_sampler: Option<PfnGlBindSampler>,

    // State
    pub enable: PfnGlEnable,
    pub disable: PfnGlDisable,
    pub is_enabled: PfnGlIsEnabled,
    pub blend_func_separate: PfnGlBlendFuncSeparate,
    pub blend_equation_separate: PfnGlBlendEquationSeparate,
    pub viewport: PfnGlViewport,
    pub scissor: PfnGlScissor,
    pub color_mask: PfnGlColorMask,
    pub depth_func: PfnGlDepthFunc,
    pub depth_mask: PfnGlDepthMask,
    pub stencil_func_separate: PfnGlStencilFuncSeparate,
    pub stencil_op_separate: PfnGlStencilOpSeparate,
    pub stencil_mask_separate: PfnGlStencilMaskSeparate,
    pub blend_color: PfnGlBlendColor,
    pub get_float_v: PfnGlGetFloatv,

    // Shaders/Programs
    pub use_program: PfnGlUseProgram,
    pub create_shader: PfnGlCreateShader,
    pub delete_shader: PfnGlDeleteShader,
    pub shader_source: PfnGlShaderSource,
    pub compile_shader: PfnGlCompileShader,
    pub get_shader_iv: PfnGlGetShaderiv,
    pub get_shader_info_log: PfnGlGetShaderInfoLog,
    pub create_program: PfnGlCreateProgram,
    pub delete_program: PfnGlDeleteProgram,
    pub attach_shader: PfnGlAttachShader,
    pub link_program: PfnGlLinkProgram,
    pub get_program_iv: PfnGlGetProgramiv,
    pub get_program_info_log: PfnGlGetProgramInfoLog,
    pub get_uniform_location: PfnGlGetUniformLocation,
    pub uniform_1i: PfnGlUniform1i,
    pub uniform_1f: PfnGlUniform1f,
    pub uniform_2f: PfnGlUniform2f,
    pub uniform_4f: PfnGlUniform4f,

    // Clear
    pub clear: PfnGlClear,
    pub clear_color: PfnGlClearColor,

    // VAO
    pub gen_vertex_arrays: PfnGlGenVertexArrays,
    pub delete_vertex_arrays: PfnGlDeleteVertexArrays,
    pub bind_vertex_array: PfnGlBindVertexArray,

    // Buffers
    pub gen_buffers: PfnGlGenBuffers,
    pub delete_buffers: PfnGlDeleteBuffers,
    pub bind_buffer: PfnGlBindBuffer,
    pub buffer_data: PfnGlBufferData,

    // Drawing
    pub draw_arrays: PfnGlDrawArrays,
    pub enable_vertex_attrib_array: PfnGlEnableVertexAttribArray,
    pub disable_vertex_attrib_array: PfnGlDisableVertexAttribArray,
    pub vertex_attrib_pointer: PfnGlVertexAttribPointer,
    pub bind_attrib_location: PfnGlBindAttribLocation,

    // Framebuffer
    pub bind_framebuffer: PfnGlBindFramebuffer,

    // Pixel store
    pub pixel_store_i: PfnGlPixelStorei,

    // EXT_memory_object / EXT_memory_object_fd (may not exist)
    pub create_memory_objects_ext: Option<PfnGlCreateMemoryObjectsEXT>,
    pub delete_memory_objects_ext: Option<PfnGlDeleteMemoryObjectsEXT>,
    pub import_memory_fd_ext: Option<PfnGlImportMemoryFdEXT>,
    pub tex_storage_mem_2d_ext: Option<PfnGlTexStorageMem2DEXT>,
    pub texture_parameter_i_ext: Option<PfnGlTextureParameteriEXT>,

    // Sync
    pub finish: PfnGlFinish,
    pub flush: PfnGlFlush,
}

// Resolve a required GL entry point. Panics if not found -- we're dead without these.
unsafe fn must_resolve<F>(gpa: GetProcAddrFn, name: &[u8]) -> F {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    let ptr = gpa(name.as_ptr() as *const c_char);
    assert!(
        !ptr.is_null(),
        "tuxinjector-gl-interop: failed to resolve required GL function: {}",
        std::str::from_utf8(&name[..name.len() - 1]).unwrap_or("<invalid>")
    );
    std::mem::transmute_copy(&ptr)
}

// Try to resolve an extension entry point. Returns None if the driver doesn't have it.
unsafe fn try_resolve<F: Copy>(gpa: GetProcAddrFn, name: &[u8]) -> Option<F> {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    let ptr = gpa(name.as_ptr() as *const c_char);
    if ptr.is_null() {
        tracing::debug!(
            name = std::str::from_utf8(&name[..name.len() - 1]).unwrap_or("?"),
            "extension function not available"
        );
        None
    } else {
        Some(std::mem::transmute_copy(&ptr))
    }
}

macro_rules! resolve {
    (required $gpa:expr, $name:literal) => {
        unsafe { must_resolve($gpa, concat!($name, "\0").as_bytes()) }
    };
    (optional $gpa:expr, $name:literal) => {
        unsafe { try_resolve($gpa, concat!($name, "\0").as_bytes()) }
    };
}

impl GlFns {
    pub unsafe fn resolve(get_proc: GetProcAddrFn) -> Self {
        Self {
            // Core
            get_error: resolve!(required get_proc, "glGetError"),
            get_integer_v: resolve!(required get_proc, "glGetIntegerv"),
            get_string: resolve!(required get_proc, "glGetString"),

            // Textures
            gen_textures: resolve!(required get_proc, "glGenTextures"),
            bind_texture: resolve!(required get_proc, "glBindTexture"),
            delete_textures: resolve!(required get_proc, "glDeleteTextures"),
            tex_image_2d: resolve!(required get_proc, "glTexImage2D"),
            tex_sub_image_2d: resolve!(required get_proc, "glTexSubImage2D"),
            tex_parameter_i: resolve!(required get_proc, "glTexParameteri"),
            active_texture: resolve!(required get_proc, "glActiveTexture"),
            bind_sampler: resolve!(optional get_proc, "glBindSampler"),

            // State
            enable: resolve!(required get_proc, "glEnable"),
            disable: resolve!(required get_proc, "glDisable"),
            is_enabled: resolve!(required get_proc, "glIsEnabled"),
            blend_func_separate: resolve!(required get_proc, "glBlendFuncSeparate"),
            blend_equation_separate: resolve!(required get_proc, "glBlendEquationSeparate"),
            viewport: resolve!(required get_proc, "glViewport"),
            scissor: resolve!(required get_proc, "glScissor"),
            color_mask: resolve!(required get_proc, "glColorMask"),
            depth_func: resolve!(required get_proc, "glDepthFunc"),
            depth_mask: resolve!(required get_proc, "glDepthMask"),
            stencil_func_separate: resolve!(required get_proc, "glStencilFuncSeparate"),
            stencil_op_separate: resolve!(required get_proc, "glStencilOpSeparate"),
            stencil_mask_separate: resolve!(required get_proc, "glStencilMaskSeparate"),
            blend_color: resolve!(required get_proc, "glBlendColor"),
            get_float_v: resolve!(required get_proc, "glGetFloatv"),

            // Shaders/Programs
            use_program: resolve!(required get_proc, "glUseProgram"),
            create_shader: resolve!(required get_proc, "glCreateShader"),
            delete_shader: resolve!(required get_proc, "glDeleteShader"),
            shader_source: resolve!(required get_proc, "glShaderSource"),
            compile_shader: resolve!(required get_proc, "glCompileShader"),
            get_shader_iv: resolve!(required get_proc, "glGetShaderiv"),
            get_shader_info_log: resolve!(required get_proc, "glGetShaderInfoLog"),
            create_program: resolve!(required get_proc, "glCreateProgram"),
            delete_program: resolve!(required get_proc, "glDeleteProgram"),
            attach_shader: resolve!(required get_proc, "glAttachShader"),
            link_program: resolve!(required get_proc, "glLinkProgram"),
            get_program_iv: resolve!(required get_proc, "glGetProgramiv"),
            get_program_info_log: resolve!(required get_proc, "glGetProgramInfoLog"),
            get_uniform_location: resolve!(required get_proc, "glGetUniformLocation"),
            uniform_1i: resolve!(required get_proc, "glUniform1i"),
            uniform_1f: resolve!(required get_proc, "glUniform1f"),
            uniform_2f: resolve!(required get_proc, "glUniform2f"),
            uniform_4f: resolve!(required get_proc, "glUniform4f"),

            // Clear
            clear: resolve!(required get_proc, "glClear"),
            clear_color: resolve!(required get_proc, "glClearColor"),

            // VAO — macOS GL 2.1 compat needs GL_APPLE_vertex_array_object extension
            #[cfg(target_os = "macos")]
            gen_vertex_arrays: resolve!(required get_proc, "glGenVertexArraysAPPLE"),
            #[cfg(target_os = "macos")]
            delete_vertex_arrays: resolve!(required get_proc, "glDeleteVertexArraysAPPLE"),
            #[cfg(target_os = "macos")]
            bind_vertex_array: resolve!(required get_proc, "glBindVertexArrayAPPLE"),
            #[cfg(target_os = "linux")]
            gen_vertex_arrays: resolve!(required get_proc, "glGenVertexArrays"),
            #[cfg(target_os = "linux")]
            delete_vertex_arrays: resolve!(required get_proc, "glDeleteVertexArrays"),
            #[cfg(target_os = "linux")]
            bind_vertex_array: resolve!(required get_proc, "glBindVertexArray"),

            // Buffers
            gen_buffers: resolve!(required get_proc, "glGenBuffers"),
            delete_buffers: resolve!(required get_proc, "glDeleteBuffers"),
            bind_buffer: resolve!(required get_proc, "glBindBuffer"),
            buffer_data: resolve!(required get_proc, "glBufferData"),

            // Drawing
            draw_arrays: resolve!(required get_proc, "glDrawArrays"),
            enable_vertex_attrib_array: resolve!(required get_proc, "glEnableVertexAttribArray"),
            disable_vertex_attrib_array: resolve!(required get_proc, "glDisableVertexAttribArray"),
            vertex_attrib_pointer: resolve!(required get_proc, "glVertexAttribPointer"),
            bind_attrib_location: resolve!(required get_proc, "glBindAttribLocation"),

            // Framebuffer
            bind_framebuffer: resolve!(required get_proc, "glBindFramebuffer"),

            // Pixel store
            pixel_store_i: resolve!(required get_proc, "glPixelStorei"),

            // Extensions (might not be there)
            create_memory_objects_ext: resolve!(optional get_proc, "glCreateMemoryObjectsEXT"),
            delete_memory_objects_ext: resolve!(optional get_proc, "glDeleteMemoryObjectsEXT"),
            import_memory_fd_ext: resolve!(optional get_proc, "glImportMemoryFdEXT"),
            tex_storage_mem_2d_ext: resolve!(optional get_proc, "glTexStorageMem2DEXT"),
            texture_parameter_i_ext: resolve!(optional get_proc, "glTextureParameteriEXT"),

            // Sync
            finish: resolve!(required get_proc, "glFinish"),
            flush: resolve!(required get_proc, "glFlush"),
        }
    }

    // True if all EXT_memory_object_fd functions resolved
    pub fn has_memory_object_ext(&self) -> bool {
        self.create_memory_objects_ext.is_some()
            && self.import_memory_fd_ext.is_some()
            && self.tex_storage_mem_2d_ext.is_some()
    }
}
