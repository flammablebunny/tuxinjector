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
#[cfg(target_os = "linux")]
const RTLD_NEXT: *mut c_void = -1isize as *mut c_void;

// --- resolving the *real* dlsym ---

static REAL_DLSYM: OnceLock<DlsymFn> = OnceLock::new();

// Linux: we need dlvsym to get the real dlsym since we've replaced it.
// Try GLIBC_2.34 first (newer), fall back to 2.2.5.
#[cfg(target_os = "linux")]
fn resolve_real_dlsym() -> DlsymFn {
    extern "C" {
        fn dlvsym(
            handle: *mut c_void,
            symbol: *const c_char,
            version: *const c_char,
        ) -> *mut c_void;
    }

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

// macOS: __interpose redirects our own dlsym calls too, so RTLD_NEXT would
// recurse. Read the real address straight from the interpose struct instead.
#[cfg(target_os = "macos")]
fn resolve_real_dlsym() -> DlsymFn {
    let ptr = macos_interpose::original_dlsym_ptr();
    if ptr.is_null() {
        panic!("tuxinjector: __interpose original dlsym pointer is null");
    }
    eprintln!("[tuxinjector] resolve_real_dlsym: real dlsym at {ptr:p}");
    unsafe { std::mem::transmute::<*mut c_void, DlsymFn>(ptr) }
}

fn real_dlsym() -> DlsymFn {
    *REAL_DLSYM.get_or_init(resolve_real_dlsym)
}

// Resolve a symbol via the real dlsym. Name must be NUL-terminated.
// Linux uses RTLD_NEXT, macOS uses RTLD_DEFAULT (OpenGL.framework
// isn't visible through RTLD_NEXT).
pub(crate) fn resolve_real_symbol(name: &[u8]) -> *mut c_void {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    #[cfg(target_os = "macos")]
    let handle = libc::RTLD_DEFAULT;
    #[cfg(target_os = "linux")]
    let handle = RTLD_NEXT;
    unsafe { real_dlsym()(handle, name.as_ptr() as *const c_char) }
}

// Resolve via RTLD_DEFAULT (global scope). Name must be NUL-terminated.
#[cfg(target_os = "linux")]
pub(crate) fn resolve_real_symbol_default(name: &[u8]) -> *mut c_void {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    #[cfg(target_os = "macos")]
    let handle = libc::RTLD_DEFAULT;
    #[cfg(target_os = "linux")]
    let handle = std::ptr::null_mut(); // glibc: RTLD_DEFAULT is just NULL
    unsafe { real_dlsym()(handle, name.as_ptr() as *const c_char) }
}

/// Same but from a specific handle.
#[cfg(target_os = "linux")]
pub(crate) fn resolve_real_symbol_from(handle: *mut c_void, name: &[u8]) -> *mut c_void {
    debug_assert!(name.last() == Some(&0), "name must be NUL-terminated");
    unsafe { real_dlsym()(handle, name.as_ptr() as *const c_char) }
}

// --- dlopen hook ---

#[cfg(target_os = "linux")]
type DlopenFn = unsafe extern "C" fn(*const c_char, libc::c_int) -> *mut c_void;
#[cfg(target_os = "linux")]
static REAL_DLOPEN: OnceLock<DlopenFn> = OnceLock::new();

// Strip RTLD_DEEPBIND so LWJGL3 JNI libs see our #[no_mangle] GL/GLFW hooks.
// macOS doesn't have RTLD_DEEPBIND at all.
#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn dlopen(path: *const c_char, flags: libc::c_int) -> *mut c_void {
    let real = REAL_DLOPEN.get_or_init(|| {
        let ptr = real_dlsym()(RTLD_NEXT, b"dlopen\0".as_ptr() as *const c_char);
        assert!(!ptr.is_null(), "tuxinjector: can't resolve real dlopen");
        std::mem::transmute(ptr)
    });

    #[cfg(target_os = "linux")]
    let clean = {
        let c = flags & !(libc::RTLD_DEEPBIND as libc::c_int);
        if c != flags { tracing::debug!("dlopen: stripped RTLD_DEEPBIND"); }
        c
    };
    #[cfg(target_os = "macos")]
    let clean = flags;

    real(path, clean)
}

// --- the big dlsym hook ---
// Linux: LD_PRELOAD shadows the real dlsym
// macOS: __DATA,__interpose makes dyld redirect calls from all images

unsafe fn dlsym_hook_impl(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    if symbol.is_null() {
        return real_dlsym()(handle, symbol);
    }

    let name = unsafe { CStr::from_ptr(symbol) };
    let bytes = name.to_bytes();

    // resolve real ptr, stash it, hand back our hook
    macro_rules! hook {
        ($store:expr, $replacement:expr) => {{
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() { $store(real_ptr); }
            $replacement as *mut c_void
        }};
    }

    match bytes {
        // -- EGL --
        #[cfg(target_os = "linux")]
        b"eglGetProcAddress" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                gl_resolve::store_egl_get_proc_address(real_ptr);
                tracing::info!("hooked eglGetProcAddress");
            }
            hooked_egl_get_proc_address as *mut c_void
        }

        #[cfg(target_os = "linux")]
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

        // -- GLX --
        #[cfg(target_os = "linux")]
        b"glXGetProcAddressARB" => {
            // RTLD_NEXT to skip our own #[no_mangle] PLT export and get
            // the real glXGetProcAddressARB from libGL/libGLX
            let real_ptr = real_dlsym()(libc::RTLD_NEXT, symbol);
            if !real_ptr.is_null() {
                gl_resolve::store_glx_get_proc_address(real_ptr);
                tracing::info!("hooked glXGetProcAddressARB");
            } else {
                tracing::warn!("glXGetProcAddressARB: RTLD_NEXT returned null");
            }
            hooked_egl_get_proc_address as *mut c_void
        }

        #[cfg(target_os = "linux")]
        b"glXSwapBuffers" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                swap_hook::store_real_glx_swap(real_ptr);

                // RTLD_NEXT to skip our PLT export and get the real one
                let gpa = real_dlsym()(libc::RTLD_NEXT, b"glXGetProcAddressARB\0".as_ptr() as *const c_char);
                if !gpa.is_null() {
                    gl_resolve::store_glx_get_proc_address(gpa);
                    tracing::debug!("got glXGetProcAddressARB (fallback via glXSwapBuffers hook)");
                }

                tracing::info!("hooked glXSwapBuffers");
            }
            swap_hook::hooked_glx_swap_buffers as *mut c_void
        }

        // -- CGL --
        #[cfg(target_os = "macos")]
        b"CGLFlushDrawable" => {
            hook!(swap_hook::store_real_cgl_flush, swap_hook::hooked_cgl_flush)
        }

        // -- glfwSwapBuffers (macOS swap entry point) --
        #[cfg(target_os = "macos")]
        b"glfwSwapBuffers" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                swap_hook::store_real_glfw_swap(real_ptr);
                eprintln!("[tuxinjector] hooked glfwSwapBuffers via dlsym interpose");
                tracing::info!("hooked glfwSwapBuffers via dlsym interpose");
            }
            swap_hook::hooked_glfw_swap_buffers as *mut c_void
        }

        // GLFW proc address
        b"glfwGetProcAddress" => {
            let real_ptr = real_dlsym()(handle, symbol);
            if !real_ptr.is_null() {
                glfw_hook::store_real_glfw_get_proc_address(real_ptr);
                // macOS: GLFW GPA is our main way to get GL fn ptrs
                #[cfg(target_os = "macos")]
                gl_resolve::store_glfw_get_proc_address(real_ptr);
                tracing::info!("hooked glfwGetProcAddress");
            }
            glfw_hook::glfwGetProcAddress as *mut c_void
        }

        // -- GL viewport & framebuffer hooks --
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

        // -- GLFW input callbacks --
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

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    dlsym_hook_impl(handle, symbol)
}

// macOS entry point -- dyld routes ALL dlsym calls here via __interpose
#[cfg(target_os = "macos")]
pub(crate) unsafe extern "C" fn hooked_dlsym_macos(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    use std::sync::atomic::{AtomicU32, Ordering as AtOrd};
    static CALL_N: AtomicU32 = AtomicU32::new(0);
    let n = CALL_N.fetch_add(1, AtOrd::Relaxed);
    if n < 10 {
        if !symbol.is_null() {
            let s = std::ffi::CStr::from_ptr(symbol);
            eprintln!("[tuxinjector] dlsym #{n}: {s:?}");
        } else {
            eprintln!("[tuxinjector] dlsym #{n}: (null symbol)");
        }
    }
    dlsym_hook_impl(handle, symbol)
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

// Same idea as the dlsym hook but for eglGetProcAddress / glXGetProcAddressARB.
// Intercepts GL function pointer queries so viewport/framebuffer hooks land.
#[cfg(target_os = "linux")]
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
        // GPA not available yet (pre-context) - fall back to dlsym so GL
        // context creation can still resolve functions (fixes NVIDIA GLX)
        libc::dlsym(libc::RTLD_DEFAULT, name)
    }
}

// --- macOS __interpose for dlsym ---
// LWJGL calls dlsym(glfw_handle, "glfwSwapBuffers") with a specific handle,
// which #[no_mangle] exports can't catch. __interpose redirects everything.
#[cfg(target_os = "macos")]
mod macos_interpose {
    use std::ffi::{c_char, c_void};

    type DlsymFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    #[repr(C)]
    struct DyldInterpose {
        replacement: DlsymFn,
        original: DlsymFn,
    }
    unsafe impl Sync for DyldInterpose {}

    #[used]
    #[link_section = "__DATA,__interpose"]
    static INTERPOSE_DLSYM: DyldInterpose = DyldInterpose {
        replacement: super::hooked_dlsym_macos,
        original: dlsym,
    };

    // dyld filled this in during BIND, so it's the real libSystem dlsym
    pub(super) fn original_dlsym_ptr() -> *mut c_void {
        unsafe {
            // read_volatile forces an actual load from memory -- the struct
            // lives in __DATA,__interpose and dyld patches it at load time
            std::mem::transmute::<DlsymFn, *mut c_void>(
                core::ptr::read_volatile(core::ptr::addr_of!(INTERPOSE_DLSYM.original))
            )
        }
    }
}
