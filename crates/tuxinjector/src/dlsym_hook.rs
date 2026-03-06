// dlsym interposition -- intercepts EGL/GLX/GLFW symbol lookups via LD_PRELOAD,
// stashes the real function pointers and returns our hooked versions

use std::ffi::{c_char, c_void, CStr};
use std::sync::OnceLock;

extern crate libc;

use crate::gl_resolve;
use crate::glfw_hook;
use crate::swap_hook;
use crate::viewport_hook;

type DlsymFn = unsafe extern "C" fn(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;

// glibc: RTLD_NEXT = (void *) -1
const RTLD_NEXT: *mut c_void = -1isize as *mut c_void;

// --- resolving the *real* dlsym ---

extern "C" {
    fn dlvsym(
        handle: *mut c_void,
        symbol: *const c_char,
        version: *const c_char,
    ) -> *mut c_void;
}

static REAL_DLSYM: OnceLock<DlsymFn> = OnceLock::new();

// We have to use dlvsym to get the real dlsym because we've
// interposed dlsym itself. Tries GLIBC_2.34 first, 2.2.5 fallback.
fn resolve_real_dlsym() -> DlsymFn {
    const NAME: &[u8] = b"dlsym\0";
    const V234: &[u8] = b"GLIBC_2.34\0";
    const V225: &[u8] = b"GLIBC_2.2.5\0";

    unsafe {
        let sym = NAME.as_ptr() as *const c_char;

        let ptr = dlvsym(RTLD_NEXT, sym, V234.as_ptr() as *const c_char);
        if !ptr.is_null() {
            tracing::debug!("resolved real dlsym via GLIBC_2.34");
            return std::mem::transmute::<*mut c_void, DlsymFn>(ptr);
        }

        let ptr = dlvsym(RTLD_NEXT, sym, V225.as_ptr() as *const c_char);
        if !ptr.is_null() {
            tracing::debug!("resolved real dlsym via GLIBC_2.2.5");
            return std::mem::transmute::<*mut c_void, DlsymFn>(ptr);
        }

        panic!("tuxinjector: can't resolve real dlsym via dlvsym -- game over");
    }
}

fn real_dlsym() -> DlsymFn {
    *REAL_DLSYM.get_or_init(resolve_real_dlsym)
}

/// Resolve a symbol via the real dlsym (RTLD_NEXT). Name must be NUL-terminated.
pub(crate) fn resolve_real_symbol(name: &[u8]) -> *mut c_void {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    unsafe { real_dlsym()(RTLD_NEXT, name.as_ptr() as *const c_char) }
}

/// Resolve via RTLD_DEFAULT (global scope search). Name must be NUL-terminated.
pub(crate) fn resolve_real_symbol_default(name: &[u8]) -> *mut c_void {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    // libc::RTLD_DEFAULT is typically NULL on glibc
    unsafe { real_dlsym()(std::ptr::null_mut(), name.as_ptr() as *const c_char) }
}

/// Same but from a specific handle.
pub(crate) fn resolve_real_symbol_from(handle: *mut c_void, name: &[u8]) -> *mut c_void {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    unsafe { real_dlsym()(handle, name.as_ptr() as *const c_char) }
}

// --- dlopen hook ---

type DlopenFn = unsafe extern "C" fn(*const c_char, libc::c_int) -> *mut c_void;
static REAL_DLOPEN: OnceLock<DlopenFn> = OnceLock::new();

// Strip RTLD_DEEPBIND so LWJGL3 JNI libs resolve from the global namespace,
// which makes our #[no_mangle] GL/GLFW exports visible to them.
#[no_mangle]
pub unsafe extern "C" fn dlopen(path: *const c_char, flags: libc::c_int) -> *mut c_void {
    let real = REAL_DLOPEN.get_or_init(|| {
        let ptr = real_dlsym()(RTLD_NEXT, b"dlopen\0".as_ptr() as *const c_char);
        assert!(!ptr.is_null(), "tuxinjector: can't resolve real dlopen");
        std::mem::transmute(ptr)
    });
    let clean = flags & !(libc::RTLD_DEEPBIND as libc::c_int);
    if clean != flags {
        tracing::debug!("dlopen: stripped RTLD_DEEPBIND");
    }
    real(path, clean)
}

// --- the big dlsym hook ---

#[no_mangle]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    if symbol.is_null() {
        return real_dlsym()(handle, symbol);
    }

    let name = unsafe { CStr::from_ptr(symbol) };
    let bytes = name.to_bytes();

    // somebody should refactor this into a macro, but i'm too lazy to
    // (each arm: resolve real, stash it, return our hook)
    macro_rules! hook {
        ($store:expr, $replacement:expr) => {{
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() { $store(real_ptr); }
            $replacement as *mut c_void
        }};
    }

    match bytes {
        // EGL
        b"eglGetProcAddress" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                gl_resolve::store_egl_get_proc_address(real_ptr);
                tracing::info!("hooked eglGetProcAddress");
            }
            hooked_egl_get_proc_address as *mut c_void
        }

        b"eglSwapBuffers" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                swap_hook::store_real_egl_swap(real_ptr);

                // opportunistically grab eglGetProcAddress from the same handle
                let gpa = real_dlsym()(handle, b"eglGetProcAddress\0".as_ptr() as *const c_char);
                if !gpa.is_null() {
                    gl_resolve::store_egl_get_proc_address(gpa);
                    tracing::debug!("got eglGetProcAddress (fallback via eglSwapBuffers hook)");
                }

                tracing::info!("hooked eglSwapBuffers");
            }
            swap_hook::hooked_egl_swap_buffers as *mut c_void
        }

        // GLX
        b"glXGetProcAddressARB" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                gl_resolve::store_glx_get_proc_address(real_ptr);
                tracing::info!("hooked glXGetProcAddressARB");
            }
            hooked_egl_get_proc_address as *mut c_void
        }

        b"glXSwapBuffers" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                swap_hook::store_real_glx_swap(real_ptr);

                let gpa = real_dlsym()(handle, b"glXGetProcAddressARB\0".as_ptr() as *const c_char);
                if !gpa.is_null() {
                    gl_resolve::store_glx_get_proc_address(gpa);
                    tracing::debug!("got glXGetProcAddressARB (fallback via glXSwapBuffers hook)");
                }

                tracing::info!("hooked glXSwapBuffers");
            }
            swap_hook::hooked_glx_swap_buffers as *mut c_void
        }

        // GLFW proc address
        b"glfwGetProcAddress" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                glfw_hook::store_real_glfw_get_proc_address(real_ptr);
                tracing::info!("hooked glfwGetProcAddress");
            }
            glfw_hook::glfwGetProcAddress as *mut c_void
        }

        // ── GL Viewport & Framebuffer Hooks ──
        // (LLM made or organised. Im too tired to write all of this stuff right now)
        b"glViewport"           => hook!(viewport_hook::store_real_gl_viewport, viewport_hook::glViewport),
        b"glBlitFramebuffer"    => hook!(viewport_hook::store_real_gl_blit_framebuffer, viewport_hook::glBlitFramebuffer),
        b"glScissor"            => hook!(viewport_hook::store_real_gl_scissor, viewport_hook::glScissor),
        b"glBindFramebuffer"    => hook!(viewport_hook::store_real_gl_bind_framebuffer, viewport_hook::glBindFramebuffer),
        b"glBindFramebufferEXT" => hook!(viewport_hook::store_real_gl_bind_framebuffer_ext, viewport_hook::glBindFramebufferEXT),
        b"glBindFramebufferARB" => hook!(viewport_hook::store_real_gl_bind_framebuffer_arb, viewport_hook::glBindFramebufferARB),
        b"glDrawBuffer"         => hook!(viewport_hook::store_real_gl_draw_buffer, viewport_hook::glDrawBuffer),
        b"glReadBuffer"         => hook!(viewport_hook::store_real_gl_read_buffer, viewport_hook::glReadBuffer),
        b"glDrawBuffers"        => hook!(viewport_hook::store_real_gl_draw_buffers, viewport_hook::glDrawBuffers),

        // ── GLFW Input Callbacks ──
        b"glfwSetKeyCallback"             => hook!(tuxinjector_input::callbacks::store_real_set_key_callback, hooked_set_key_callback),
        b"glfwSetMouseButtonCallback"     => hook!(tuxinjector_input::callbacks::store_real_set_mouse_button_callback, hooked_set_mouse_button_callback),
        b"glfwSetCursorPosCallback"       => hook!(tuxinjector_input::callbacks::store_real_set_cursor_pos_callback, hooked_set_cursor_pos_callback),
        b"glfwSetScrollCallback"          => hook!(tuxinjector_input::callbacks::store_real_set_scroll_callback, hooked_set_scroll_callback),
        b"glfwSetCharCallback"            => hook!(tuxinjector_input::callbacks::store_real_set_char_callback, hooked_set_char_callback),
        b"glfwSetCharModsCallback"        => hook!(tuxinjector_input::callbacks::store_real_set_char_mods_callback, hooked_set_char_mods_callback),
        b"glfwSetFramebufferSizeCallback" => hook!(viewport_hook::store_real_set_fb_size_cb, viewport_hook::hooked_glfw_set_framebuffer_size_callback),
        b"glfwGetFramebufferSize"         => hook!(viewport_hook::store_real_get_fb_size, viewport_hook::hooked_glfw_get_framebuffer_size),
        b"glfwSetInputMode"               => hook!(tuxinjector_input::callbacks::store_real_set_input_mode, hooked_set_input_mode),

        // GLFW cursor/key poll - warn if not found since these are important
        b"glfwGetKey" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                crate::glfw_hook::store_real_get_key(real_ptr);
            } else {
                tracing::warn!("glfwGetKey: real symbol not found");
            }
            crate::glfw_hook::glfwGetKey as *mut c_void
        }
        b"glfwGetMouseButton" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                crate::glfw_hook::store_real_get_mouse_button(real_ptr);
            } else {
                tracing::warn!("glfwGetMouseButton: real symbol not found");
            }
            crate::glfw_hook::glfwGetMouseButton as *mut c_void
        }
        b"glfwGetCursorPos" => {
            // must use bundled libglfw - RTLD_NEXT finds the system one
            // which doesn't know LWJGL3's window handles
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                crate::glfw_hook::store_real_get_cursor_pos(real_ptr);
            } else {
                tracing::warn!("glfwGetCursorPos: real symbol not found");
            }
            crate::glfw_hook::glfwGetCursorPos as *mut c_void
        }

        b"glfwSetWindowTitle" => hook!(crate::window_state::store_real_set_window_title, crate::window_state::hooked_glfw_set_window_title),

        _ => real_dlsym()(handle, symbol),
    }
}

// --- hooked GLFW callback wrappers ---
// These are returned from our dlsym hook. They just delegate to
// tuxinjector-input which stores the game's original callback and
// installs our wrapper.

use tuxinjector_input::glfw_types::{
    GlfwCharCallback, GlfwCharModsCallback, GlfwCursorPosCallback, GlfwKeyCallback,
    GlfwMouseButtonCallback, GlfwScrollCallback, GlfwWindow,
};

unsafe extern "C" fn hooked_set_key_callback(
    window: GlfwWindow,
    callback: GlfwKeyCallback,
) -> GlfwKeyCallback {
    tuxinjector_input::callbacks::intercept_set_key_callback(window, callback)
}

unsafe extern "C" fn hooked_set_mouse_button_callback(
    window: GlfwWindow,
    callback: GlfwMouseButtonCallback,
) -> GlfwMouseButtonCallback {
    tuxinjector_input::callbacks::intercept_set_mouse_button_callback(window, callback)
}

unsafe extern "C" fn hooked_set_cursor_pos_callback(
    window: GlfwWindow,
    callback: GlfwCursorPosCallback,
) -> GlfwCursorPosCallback {
    tuxinjector_input::callbacks::intercept_set_cursor_pos_callback(window, callback)
}

unsafe extern "C" fn hooked_set_scroll_callback(
    window: GlfwWindow,
    callback: GlfwScrollCallback,
) -> GlfwScrollCallback {
    tuxinjector_input::callbacks::intercept_set_scroll_callback(window, callback)
}

// routes typed chars to imgui when GUI is open
unsafe extern "C" fn hooked_set_char_callback(
    window: GlfwWindow,
    callback: GlfwCharCallback,
) -> GlfwCharCallback {
    tuxinjector_input::callbacks::intercept_set_char_callback(window, callback)
}

// LWJGL3 uses this one instead of plain glfwSetCharCallback
unsafe extern "C" fn hooked_set_char_mods_callback(
    window: GlfwWindow,
    callback: GlfwCharModsCallback,
) -> GlfwCharModsCallback {
    tuxinjector_input::callbacks::intercept_set_char_mods_callback(window, callback)
}

// tracks cursor capture state (FPS vs menu)
unsafe extern "C" fn hooked_set_input_mode(window: GlfwWindow, mode: i32, value: i32) {
    tuxinjector_input::callbacks::intercept_set_input_mode(window, mode, value);
}

// --- hooked eglGetProcAddress / glXGetProcAddressARB ---

// Intercepts GL function pointer queries so we can hook viewport/framebuffer calls.
// Same idea as the dlsym hook above but for the GL loader path.
unsafe extern "C" fn hooked_egl_get_proc_address(name: *const c_char) -> *mut c_void {
    use std::ffi::CStr;

    if name.is_null() { return std::ptr::null_mut(); }

    let bytes = CStr::from_ptr(name).to_bytes();

    // resolve real + stash it, return our hook instead
    macro_rules! gpa_hook {
        ($store:expr, $hook:expr) => {{
            if let Some(f) = gl_resolve::get_proc_address_fn() {
                $store(f(name));
            }
            return $hook as *mut c_void;
        }};
    }

    match bytes {
        b"glViewport"           => gpa_hook!(viewport_hook::store_real_gl_viewport, viewport_hook::glViewport),
        b"glScissor"            => gpa_hook!(viewport_hook::store_real_gl_scissor, viewport_hook::glScissor),
        b"glBindFramebuffer"    => gpa_hook!(viewport_hook::store_real_gl_bind_framebuffer, viewport_hook::glBindFramebuffer),
        b"glBindFramebufferEXT" => gpa_hook!(viewport_hook::store_real_gl_bind_framebuffer_ext, viewport_hook::glBindFramebufferEXT),
        b"glBindFramebufferARB" => gpa_hook!(viewport_hook::store_real_gl_bind_framebuffer_arb, viewport_hook::glBindFramebufferARB),
        b"glDrawBuffer"         => gpa_hook!(viewport_hook::store_real_gl_draw_buffer, viewport_hook::glDrawBuffer),
        b"glReadBuffer"         => gpa_hook!(viewport_hook::store_real_gl_read_buffer, viewport_hook::glReadBuffer),
        b"glDrawBuffers"        => gpa_hook!(viewport_hook::store_real_gl_draw_buffers, viewport_hook::glDrawBuffers),
        _ => {}
    }

    // everything else just passes through
    if let Some(f) = gl_resolve::get_proc_address_fn() {
        f(name)
    } else {
        std::ptr::null_mut()
    }
}
