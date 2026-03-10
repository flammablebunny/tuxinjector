// **A LOT OF THE FOLLOWING CODE WAS WRITTEN/REORGANIZED BY AN LLM**
//
// ===========================================================================
// Module: gl_resolve — GL Function Pointer Resolution
// ===========================================================================
//
// Resolves all required OpenGL function pointers via a captured
// eglGetProcAddress or glXGetProcAddressARB entry point.
//
// Core entry points are mandatory — resolution failure is fatal.
// → Extension entry points (EXT_memory_object / EXT_semaphore) are
//   Option-wrapped, as their availability is not guaranteed.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicPtr, Ordering};

pub type EglGetProcAddressFn = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;

// Linux: EGL/GLX proc address
#[cfg(target_os = "linux")]
static EGL_GET_PROC_ADDRESS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
#[cfg(target_os = "linux")]
static GLX_GET_PROC_ADDRESS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// macOS: GLFW proc address (GLFW wraps CGL internally)
#[cfg(target_os = "macos")]
static GLFW_GET_PROC_ADDRESS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "linux")]
pub fn store_egl_get_proc_address(ptr: *mut c_void) {
    EGL_GET_PROC_ADDRESS.store(ptr, Ordering::Release);
}

#[cfg(target_os = "linux")]
pub fn store_glx_get_proc_address(ptr: *mut c_void) {
    GLX_GET_PROC_ADDRESS.store(ptr, Ordering::Release);
}

#[cfg(target_os = "macos")]
pub fn store_glfw_get_proc_address(ptr: *mut c_void) {
    GLFW_GET_PROC_ADDRESS.store(ptr, Ordering::Release);
}

// try EGL first, then GLX (Linux), then GLFW GPA (macOS)
pub fn get_proc_address_fn() -> Option<EglGetProcAddressFn> {
    #[cfg(target_os = "linux")]
    {
        let egl = EGL_GET_PROC_ADDRESS.load(Ordering::Acquire);
        if !egl.is_null() {
            return Some(unsafe { std::mem::transmute(egl) });
        }
        let glx = GLX_GET_PROC_ADDRESS.load(Ordering::Acquire);
        if !glx.is_null() {
            return Some(unsafe { std::mem::transmute(glx) });
        }
    }
    #[cfg(target_os = "macos")]
    {
        let glfw = GLFW_GET_PROC_ADDRESS.load(Ordering::Acquire);
        if !glfw.is_null() {
            return Some(unsafe { std::mem::transmute(glfw) });
        }
    }
    None
}


// -- GL Type Aliases ------------------------------------------------------

#[allow(non_camel_case_types)]
type GLenum = u32;
#[allow(non_camel_case_types)]
type GLuint = u32;
#[allow(non_camel_case_types)]
type GLint = i32;
#[allow(non_camel_case_types)]
type GLsizei = i32;
#[allow(non_camel_case_types)]
type GLboolean = u8;
#[allow(non_camel_case_types)]
type GLbitfield = u32;
#[allow(non_camel_case_types)]
type GLfloat = f32;
#[allow(non_camel_case_types)]
type GLclampf = f32;
#[allow(non_camel_case_types)]
type GLsizeiptr = isize;
#[allow(non_camel_case_types)]
type GLchar = c_char;
#[allow(non_camel_case_types)]
type GLuint64 = u64;

macro_rules! gl_fn_type {
    ($name:ident => unsafe fn($($arg:ident : $arg_ty:ty),* $(,)?) $(-> $ret:ty)?) => {
        #[allow(non_camel_case_types, dead_code)]
        type $name = unsafe extern "C" fn($($arg : $arg_ty),*) $(-> $ret)?;
    };
}

// -- Core GL Function Pointer Types ---------------------------------------

gl_fn_type!(PfnGlGenTextures          => unsafe fn(n: GLsizei, textures: *mut GLuint));
gl_fn_type!(PfnGlBindTexture          => unsafe fn(target: GLenum, texture: GLuint));
gl_fn_type!(PfnGlDeleteTextures       => unsafe fn(n: GLsizei, textures: *const GLuint));
gl_fn_type!(PfnGlTexImage2D           => unsafe fn(target: GLenum, level: GLint, internal_format: GLint, width: GLsizei, height: GLsizei, border: GLint, format: GLenum, ty: GLenum, pixels: *const c_void));
gl_fn_type!(PfnGlTexSubImage2D        => unsafe fn(target: GLenum, level: GLint, xoffset: GLint, yoffset: GLint, width: GLsizei, height: GLsizei, format: GLenum, ty: GLenum, pixels: *const c_void));
gl_fn_type!(PfnGlTexParameteri        => unsafe fn(target: GLenum, pname: GLenum, param: GLint));
gl_fn_type!(PfnGlCopyTexSubImage2D   => unsafe fn(target: GLenum, level: GLint, xoffset: GLint, yoffset: GLint, x: GLint, y: GLint, width: GLsizei, height: GLsizei));

gl_fn_type!(PfnGlEnable               => unsafe fn(cap: GLenum));
gl_fn_type!(PfnGlDisable              => unsafe fn(cap: GLenum));
gl_fn_type!(PfnGlBlendFuncSeparate    => unsafe fn(src_rgb: GLenum, dst_rgb: GLenum, src_alpha: GLenum, dst_alpha: GLenum));
gl_fn_type!(PfnGlDrawBuffer           => unsafe fn(mode: GLenum));
gl_fn_type!(PfnGlReadBuffer           => unsafe fn(mode: GLenum));
gl_fn_type!(PfnGlDrawBuffers          => unsafe fn(n: GLsizei, bufs: *const GLenum));
gl_fn_type!(PfnGlViewport             => unsafe fn(x: GLint, y: GLint, width: GLsizei, height: GLsizei));
gl_fn_type!(PfnGlScissor              => unsafe fn(x: GLint, y: GLint, width: GLsizei, height: GLsizei));
gl_fn_type!(PfnGlClear                => unsafe fn(mask: GLbitfield));
gl_fn_type!(PfnGlClearColor           => unsafe fn(r: GLclampf, g: GLclampf, b: GLclampf, a: GLclampf));

gl_fn_type!(PfnGlUseProgram           => unsafe fn(program: GLuint));
gl_fn_type!(PfnGlGetIntegerv          => unsafe fn(pname: GLenum, data: *mut GLint));
gl_fn_type!(PfnGlGetString            => unsafe fn(name: GLenum) -> *const c_char);

gl_fn_type!(PfnGlGenVertexArrays      => unsafe fn(n: GLsizei, arrays: *mut GLuint));
gl_fn_type!(PfnGlBindVertexArray       => unsafe fn(array: GLuint));

gl_fn_type!(PfnGlGenBuffers           => unsafe fn(n: GLsizei, buffers: *mut GLuint));
gl_fn_type!(PfnGlBindBuffer           => unsafe fn(target: GLenum, buffer: GLuint));
gl_fn_type!(PfnGlBufferData           => unsafe fn(target: GLenum, size: GLsizeiptr, data: *const c_void, usage: GLenum));

gl_fn_type!(PfnGlDrawArrays           => unsafe fn(mode: GLenum, first: GLint, count: GLsizei));
gl_fn_type!(PfnGlEnableVertexAttribArray  => unsafe fn(index: GLuint));
gl_fn_type!(PfnGlVertexAttribPointer  => unsafe fn(index: GLuint, size: GLint, ty: GLenum, normalized: GLboolean, stride: GLsizei, pointer: *const c_void));

gl_fn_type!(PfnGlCreateShader         => unsafe fn(ty: GLenum) -> GLuint);
gl_fn_type!(PfnGlShaderSource         => unsafe fn(shader: GLuint, count: GLsizei, string: *const *const GLchar, length: *const GLint));
gl_fn_type!(PfnGlCompileShader        => unsafe fn(shader: GLuint));
gl_fn_type!(PfnGlCreateProgram        => unsafe fn() -> GLuint);
gl_fn_type!(PfnGlAttachShader         => unsafe fn(program: GLuint, shader: GLuint));
gl_fn_type!(PfnGlLinkProgram          => unsafe fn(program: GLuint));
gl_fn_type!(PfnGlGetUniformLocation   => unsafe fn(program: GLuint, name: *const GLchar) -> GLint);
gl_fn_type!(PfnGlUniform1i            => unsafe fn(location: GLint, v0: GLint));
gl_fn_type!(PfnGlUniform1f            => unsafe fn(location: GLint, v0: GLfloat));
gl_fn_type!(PfnGlUniform4f            => unsafe fn(location: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat, v3: GLfloat));
gl_fn_type!(PfnGlActiveTexture        => unsafe fn(texture: GLenum));
gl_fn_type!(PfnGlColorMask            => unsafe fn(r: GLboolean, g: GLboolean, b: GLboolean, a: GLboolean));
gl_fn_type!(PfnGlBindFramebuffer      => unsafe fn(target: GLenum, framebuffer: GLuint));
gl_fn_type!(PfnGlGenFramebuffers      => unsafe fn(n: GLsizei, framebuffers: *mut GLuint));
gl_fn_type!(PfnGlDeleteFramebuffers   => unsafe fn(n: GLsizei, framebuffers: *const GLuint));
gl_fn_type!(PfnGlFramebufferTexture2D => unsafe fn(target: GLenum, attachment: GLenum, textarget: GLenum, texture: GLuint, level: GLint));
gl_fn_type!(PfnGlBindRenderbuffer     => unsafe fn(target: GLenum, renderbuffer: GLuint));
gl_fn_type!(PfnGlGenRenderbuffers     => unsafe fn(n: GLsizei, renderbuffers: *mut GLuint));
gl_fn_type!(PfnGlDeleteRenderbuffers  => unsafe fn(n: GLsizei, renderbuffers: *const GLuint));
gl_fn_type!(PfnGlRenderbufferStorage  => unsafe fn(target: GLenum, internalformat: GLenum, width: GLsizei, height: GLsizei));
gl_fn_type!(PfnGlFramebufferRenderbuffer => unsafe fn(target: GLenum, attachment: GLenum, renderbuffertarget: GLenum, renderbuffer: GLuint));
gl_fn_type!(PfnGlCheckFramebufferStatus => unsafe fn(target: GLenum) -> GLenum);
gl_fn_type!(PfnGlBlitFramebuffer      => unsafe fn(src_x0: GLint, src_y0: GLint, src_x1: GLint, src_y1: GLint, dst_x0: GLint, dst_y0: GLint, dst_x1: GLint, dst_y1: GLint, mask: GLbitfield, filter: GLenum));
gl_fn_type!(PfnGlReadPixels           => unsafe fn(x: GLint, y: GLint, width: GLsizei, height: GLsizei, format: GLenum, ty: GLenum, pixels: *mut c_void));
gl_fn_type!(PfnGlGetError             => unsafe fn() -> GLenum);
gl_fn_type!(PfnGlFlush               => unsafe fn());
gl_fn_type!(PfnGlFinish              => unsafe fn());
gl_fn_type!(PfnGlDeleteBuffers        => unsafe fn(n: GLsizei, buffers: *const GLuint));
gl_fn_type!(PfnGlMapBuffer            => unsafe fn(target: GLenum, access: GLenum) -> *mut c_void);
gl_fn_type!(PfnGlUnmapBuffer          => unsafe fn(target: GLenum) -> GLboolean);

// -- Extension Function Pointer Types (may be null) ----------------------

gl_fn_type!(PfnGlCreateMemoryObjectsEXT => unsafe fn(n: GLsizei, memory_objects: *mut GLuint));
gl_fn_type!(PfnGlImportMemoryFdEXT      => unsafe fn(memory: GLuint, size: GLuint64, handle_type: GLenum, fd: GLint));
gl_fn_type!(PfnGlTexStorageMem2DEXT     => unsafe fn(target: GLenum, levels: GLsizei, internal_format: GLenum, width: GLsizei, height: GLsizei, memory: GLuint, offset: GLuint64));
gl_fn_type!(PfnGlCreateSemaphoresEXT    => unsafe fn(n: GLsizei, semaphores: *mut GLuint));
gl_fn_type!(PfnGlImportSemaphoreFdEXT   => unsafe fn(semaphore: GLuint, handle_type: GLenum, fd: GLint));
gl_fn_type!(PfnGlWaitSemaphoreEXT       => unsafe fn(semaphore: GLuint, num_buffer_barriers: GLuint, buffers: *const GLuint, num_texture_barriers: GLuint, textures: *const GLuint, src_layouts: *const GLenum));
gl_fn_type!(PfnGlSignalSemaphoreEXT     => unsafe fn(semaphore: GLuint, num_buffer_barriers: GLuint, buffers: *const GLuint, num_texture_barriers: GLuint, textures: *const GLuint, dst_layouts: *const GLenum));

// -- GlFunctions Struct --------------------------------------------------
//
// Aggregated GL function pointers. Core fields are mandatory — a missing
// core function triggers a panic during resolve(). Extension fields are
// Option<T> to permit graceful degradation on unsupported drivers.
#[allow(dead_code)]
pub struct GlFunctions {
    // -- Texture Operations --
    pub gen_textures:      PfnGlGenTextures,
    pub bind_texture:      PfnGlBindTexture,
    pub delete_textures:   PfnGlDeleteTextures,
    pub tex_image_2d:      PfnGlTexImage2D,
    pub tex_sub_image_2d:  PfnGlTexSubImage2D,
    pub tex_parameter_i:   PfnGlTexParameteri,
    pub copy_tex_sub_image_2d: PfnGlCopyTexSubImage2D,

    // -- Render State --
    pub enable:               PfnGlEnable,
    pub disable:              PfnGlDisable,
    pub blend_func_separate:  PfnGlBlendFuncSeparate,
    pub draw_buffer:          PfnGlDrawBuffer,
    pub read_buffer:          PfnGlReadBuffer,
    pub draw_buffers:         PfnGlDrawBuffers,
    pub viewport:             PfnGlViewport,
    pub scissor:              PfnGlScissor,
    pub clear:                PfnGlClear,
    pub clear_color:          PfnGlClearColor,

    // -- Shader & Program Pipeline --
    pub use_program:           PfnGlUseProgram,
    pub get_integer_v:         PfnGlGetIntegerv,
    pub get_string:            PfnGlGetString,
    pub create_shader:         PfnGlCreateShader,
    pub shader_source:         PfnGlShaderSource,
    pub compile_shader:        PfnGlCompileShader,
    pub create_program:        PfnGlCreateProgram,
    pub attach_shader:         PfnGlAttachShader,
    pub link_program:          PfnGlLinkProgram,
    pub get_uniform_location:  PfnGlGetUniformLocation,
    pub uniform_1i:            PfnGlUniform1i,
    pub uniform_1f:            PfnGlUniform1f,
    pub uniform_4f:            PfnGlUniform4f,
    pub active_texture:        PfnGlActiveTexture,
    pub color_mask:            PfnGlColorMask,
    pub bind_framebuffer:          PfnGlBindFramebuffer,
    pub gen_framebuffers:          PfnGlGenFramebuffers,
    pub delete_framebuffers:       PfnGlDeleteFramebuffers,
    pub framebuffer_texture_2d:    PfnGlFramebufferTexture2D,
    pub bind_renderbuffer:         PfnGlBindRenderbuffer,
    pub gen_renderbuffers:         PfnGlGenRenderbuffers,
    pub delete_renderbuffers:      PfnGlDeleteRenderbuffers,
    pub renderbuffer_storage:      PfnGlRenderbufferStorage,
    pub framebuffer_renderbuffer:  PfnGlFramebufferRenderbuffer,
    pub check_framebuffer_status:  PfnGlCheckFramebufferStatus,
    pub blit_framebuffer:          PfnGlBlitFramebuffer,
    pub read_pixels:               PfnGlReadPixels,
    pub get_error:                 PfnGlGetError,
    pub flush:                     PfnGlFlush,
    pub finish:                    PfnGlFinish,

    // -- Vertex Array Objects --
    pub gen_vertex_arrays:  PfnGlGenVertexArrays,
    pub bind_vertex_array:  PfnGlBindVertexArray,

    // -- Buffer Objects --
    pub gen_buffers:    PfnGlGenBuffers,
    pub bind_buffer:    PfnGlBindBuffer,
    pub buffer_data:    PfnGlBufferData,
    pub delete_buffers: PfnGlDeleteBuffers,
    pub map_buffer:     PfnGlMapBuffer,
    pub unmap_buffer:   PfnGlUnmapBuffer,

    // -- Draw Calls & Vertex Attributes --
    pub draw_arrays:               PfnGlDrawArrays,
    pub enable_vertex_attrib_array: PfnGlEnableVertexAttribArray,
    pub vertex_attrib_pointer:     PfnGlVertexAttribPointer,

    // -- EXT_memory_object / EXT_semaphore (availability not guaranteed) --
    pub create_memory_objects_ext:  Option<PfnGlCreateMemoryObjectsEXT>,
    pub import_memory_fd_ext:       Option<PfnGlImportMemoryFdEXT>,
    pub tex_storage_mem_2d_ext:     Option<PfnGlTexStorageMem2DEXT>,
    pub create_semaphores_ext:      Option<PfnGlCreateSemaphoresEXT>,
    pub import_semaphore_fd_ext:    Option<PfnGlImportSemaphoreFdEXT>,
    pub wait_semaphore_ext:         Option<PfnGlWaitSemaphoreEXT>,
    pub signal_semaphore_ext:       Option<PfnGlSignalSemaphoreEXT>,
}

// -- Resolution Helpers --------------------------------------------------

// Panics if the function pointer resolves to null.
unsafe fn must_resolve<F>(gpa: EglGetProcAddressFn, name: &[u8]) -> F {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    let ptr = gpa(name.as_ptr() as *const c_char);
    assert!(
        !ptr.is_null(),
        "tuxinjector: required GL function missing: {}",
        std::str::from_utf8(&name[..name.len() - 1]).unwrap_or("<invalid>")
    );
    std::mem::transmute_copy(&ptr)
}

// Returns None if the extension function is unavailable.
unsafe fn try_resolve<F: Copy>(gpa: EglGetProcAddressFn, name: &[u8]) -> Option<F> {
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

impl GlFunctions {
    // Resolve all GL function pointers. Requires a current GL context.
    // → Core functions: panic on failure.
    // → Extension functions: return None on failure.
    pub unsafe fn resolve(gpa: EglGetProcAddressFn) -> Self {
        GlFunctions {
            gen_textures:      resolve!(required gpa, "glGenTextures"),
            bind_texture:      resolve!(required gpa, "glBindTexture"),
            delete_textures:   resolve!(required gpa, "glDeleteTextures"),
            tex_image_2d:      resolve!(required gpa, "glTexImage2D"),
            tex_sub_image_2d:  resolve!(required gpa, "glTexSubImage2D"),
            tex_parameter_i:   resolve!(required gpa, "glTexParameteri"),
            copy_tex_sub_image_2d: resolve!(required gpa, "glCopyTexSubImage2D"),

            enable:              resolve!(required gpa, "glEnable"),
            disable:             resolve!(required gpa, "glDisable"),
            blend_func_separate: resolve!(required gpa, "glBlendFuncSeparate"),
            draw_buffer:         resolve!(required gpa, "glDrawBuffer"),
            read_buffer:         resolve!(required gpa, "glReadBuffer"),
            draw_buffers:        resolve!(required gpa, "glDrawBuffers"),
            viewport:            resolve!(required gpa, "glViewport"),
            scissor:             resolve!(required gpa, "glScissor"),
            clear:               resolve!(required gpa, "glClear"),
            clear_color:         resolve!(required gpa, "glClearColor"),

            use_program:          resolve!(required gpa, "glUseProgram"),
            get_integer_v:        resolve!(required gpa, "glGetIntegerv"),
            get_string:           resolve!(required gpa, "glGetString"),
            create_shader:        resolve!(required gpa, "glCreateShader"),
            shader_source:        resolve!(required gpa, "glShaderSource"),
            compile_shader:       resolve!(required gpa, "glCompileShader"),
            create_program:       resolve!(required gpa, "glCreateProgram"),
            attach_shader:        resolve!(required gpa, "glAttachShader"),
            link_program:         resolve!(required gpa, "glLinkProgram"),
            get_uniform_location: resolve!(required gpa, "glGetUniformLocation"),
            uniform_1i:           resolve!(required gpa, "glUniform1i"),
            uniform_1f:           resolve!(required gpa, "glUniform1f"),
            uniform_4f:           resolve!(required gpa, "glUniform4f"),
            active_texture:       resolve!(required gpa, "glActiveTexture"),
            color_mask:           resolve!(required gpa, "glColorMask"),
            bind_framebuffer:         resolve!(required gpa, "glBindFramebuffer"),
            gen_framebuffers:         resolve!(required gpa, "glGenFramebuffers"),
            delete_framebuffers:      resolve!(required gpa, "glDeleteFramebuffers"),
            framebuffer_texture_2d:   resolve!(required gpa, "glFramebufferTexture2D"),
            bind_renderbuffer:        resolve!(required gpa, "glBindRenderbuffer"),
            gen_renderbuffers:        resolve!(required gpa, "glGenRenderbuffers"),
            delete_renderbuffers:     resolve!(required gpa, "glDeleteRenderbuffers"),
            renderbuffer_storage:     resolve!(required gpa, "glRenderbufferStorage"),
            framebuffer_renderbuffer: resolve!(required gpa, "glFramebufferRenderbuffer"),
            check_framebuffer_status: resolve!(required gpa, "glCheckFramebufferStatus"),
            blit_framebuffer:         resolve!(required gpa, "glBlitFramebuffer"),
            read_pixels:              resolve!(required gpa, "glReadPixels"),
            get_error:                resolve!(required gpa, "glGetError"),
            flush:                    resolve!(required gpa, "glFlush"),
            finish:                   resolve!(required gpa, "glFinish"),

            // macOS GL 2.1 compat needs GL_APPLE_vertex_array_object extension
            #[cfg(target_os = "macos")]
            gen_vertex_arrays:  resolve!(required gpa, "glGenVertexArraysAPPLE"),
            #[cfg(target_os = "macos")]
            bind_vertex_array:  resolve!(required gpa, "glBindVertexArrayAPPLE"),
            #[cfg(target_os = "linux")]
            gen_vertex_arrays:  resolve!(required gpa, "glGenVertexArrays"),
            #[cfg(target_os = "linux")]
            bind_vertex_array:  resolve!(required gpa, "glBindVertexArray"),

            gen_buffers:    resolve!(required gpa, "glGenBuffers"),
            bind_buffer:    resolve!(required gpa, "glBindBuffer"),
            buffer_data:    resolve!(required gpa, "glBufferData"),
            delete_buffers: resolve!(required gpa, "glDeleteBuffers"),
            map_buffer:     resolve!(required gpa, "glMapBuffer"),
            unmap_buffer:   resolve!(required gpa, "glUnmapBuffer"),

            draw_arrays:                resolve!(required gpa, "glDrawArrays"),
            enable_vertex_attrib_array: resolve!(required gpa, "glEnableVertexAttribArray"),
            vertex_attrib_pointer:      resolve!(required gpa, "glVertexAttribPointer"),

            // extensions - these might not be there and that's ok sometimes :clueless:
            create_memory_objects_ext: resolve!(optional gpa, "glCreateMemoryObjectsEXT"),
            import_memory_fd_ext:      resolve!(optional gpa, "glImportMemoryFdEXT"),
            tex_storage_mem_2d_ext:    resolve!(optional gpa, "glTexStorageMem2DEXT"),
            create_semaphores_ext:     resolve!(optional gpa, "glCreateSemaphoresEXT"),
            import_semaphore_fd_ext:   resolve!(optional gpa, "glImportSemaphoreFdEXT"),
            wait_semaphore_ext:        resolve!(optional gpa, "glWaitSemaphoreEXT"),
            signal_semaphore_ext:      resolve!(optional gpa, "glSignalSemaphoreEXT"),
        }
    }
}
