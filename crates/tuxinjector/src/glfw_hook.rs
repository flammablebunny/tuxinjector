// PLT-level #[no_mangle] exports for GLFW functions.
//
// LWJGL3 loads libglfw.so with RTLD_DEEPBIND which bypasses our dlsym hook,
// but PLT exports bind before RTLD_DEEPBIND creates a private scope.
// So we win.

use std::ffi::{c_char, c_double, c_int, c_void};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, Ordering};

use tuxinjector_input::callbacks;
use tuxinjector_input::glfw_types::{
    GlfwCharCallback, GlfwCharModsCallback, GlfwCursorPosCallback, GlfwKeyCallback,
    GlfwMouseButtonCallback, GlfwScrollCallback, GlfwWindow,
};

use crate::viewport_hook;

// --- glfwGetProcAddress ---

type GlfwGetProcAddressFn = unsafe extern "C" fn(name: *const c_char) -> *mut c_void;

static REAL_GLFW_GET_PROC_ADDRESS: OnceLock<GlfwGetProcAddressFn> = OnceLock::new();

pub fn store_real_glfw_get_proc_address(ptr: *mut c_void) {
    if !ptr.is_null() {
        let f: GlfwGetProcAddressFn = unsafe { std::mem::transmute(ptr) };
        REAL_GLFW_GET_PROC_ADDRESS.get_or_init(|| f);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glfwGetProcAddress(name: *const c_char) -> *mut c_void {
    // log once to stderr so we know our hook is alive (before tracing is up)
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| eprintln!("[tuxinjector] glfwGetProcAddress hooked"));
    let real = REAL_GLFW_GET_PROC_ADDRESS.get_or_init(|| {
        let ptr = libc::dlsym(
            libc::RTLD_NEXT,
            b"glfwGetProcAddress\0".as_ptr() as *const c_char,
        );
        if ptr.is_null() {
            tracing::error!("glfwGetProcAddress PLT: real symbol not found via RTLD_NEXT");
            std::mem::transmute::<*mut c_void, GlfwGetProcAddressFn>(std::ptr::null_mut())
        } else {
            tracing::info!("glfwGetProcAddress PLT: resolved real via RTLD_NEXT");
            std::mem::transmute(ptr)
        }
    });
    if (*real as usize) == 0 {
        return std::ptr::null_mut();
    }

    if name.is_null() {
        return real(name);
    }

    // intercept GL functions that we need to hook for mode system / viewport
    // Nvim claude autocomplete please bless me :prayge:
    let bytes = std::ffi::CStr::from_ptr(name).to_bytes();
    match bytes {
        b"glViewport" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_viewport(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glViewport): returning hook");
            viewport_hook::glViewport as *mut c_void
        }
        b"glScissor" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_scissor(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glScissor): returning hook");
            viewport_hook::glScissor as *mut c_void
        }
        b"glBindFramebuffer" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_bind_framebuffer(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glBindFramebuffer): returning hook");
            viewport_hook::glBindFramebuffer as *mut c_void
        }
        b"glBindFramebufferEXT" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_bind_framebuffer_ext(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glBindFramebufferEXT): returning hook");
            viewport_hook::glBindFramebufferEXT as *mut c_void
        }
        b"glBindFramebufferARB" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_bind_framebuffer_arb(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glBindFramebufferARB): returning hook");
            viewport_hook::glBindFramebufferARB as *mut c_void
        }
        b"glDrawBuffer" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_draw_buffer(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glDrawBuffer): returning hook");
            viewport_hook::glDrawBuffer as *mut c_void
        }
        b"glReadBuffer" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_read_buffer(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glReadBuffer): returning hook");
            viewport_hook::glReadBuffer as *mut c_void
        }
        b"glDrawBuffers" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_draw_buffers(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glDrawBuffers): returning hook");
            viewport_hook::glDrawBuffers as *mut c_void
        }
        b"glBlitFramebuffer" => {
            let real_ptr = real(name);
            if !real_ptr.is_null() { viewport_hook::store_real_gl_blit_framebuffer(real_ptr); }
            tracing::info!(?real_ptr, "glfwGetProcAddress(glBlitFramebuffer): returning hook");
            viewport_hook::glBlitFramebuffer as *mut c_void
        }
        _ => real(name),
    }
}

// PLT callback overrides - all the same pattern so we macro it.
// Each one: log once that we resolved via RTLD_NEXT, then delegate to
// our intercept layer in tuxinjector-input.
macro_rules! plt_callback_hook {
    ($fn_name:ident, $cb_type:ty, $real_name:literal, $intercept:path) => {
        #[no_mangle]
        pub unsafe extern "C" fn $fn_name(
            window: GlfwWindow,
            callback: $cb_type,
        ) -> $cb_type {
            static LOGGED: OnceLock<()> = OnceLock::new();
            LOGGED.get_or_init(|| {
                let ptr = libc::dlsym(
                    libc::RTLD_NEXT,
                    concat!($real_name, "\0").as_ptr() as *const libc::c_char,
                );
                if ptr.is_null() {
                    tracing::error!(concat!($real_name, " PLT: real not found"));
                } else {
                    tracing::info!(concat!($real_name, " PLT: resolved"));
                }
            });
            $intercept(window, callback)
        }
    };
}

plt_callback_hook!(glfwSetKeyCallback, GlfwKeyCallback, "glfwSetKeyCallback", callbacks::intercept_set_key_callback);
plt_callback_hook!(glfwSetMouseButtonCallback, GlfwMouseButtonCallback, "glfwSetMouseButtonCallback", callbacks::intercept_set_mouse_button_callback);
plt_callback_hook!(glfwSetCursorPosCallback, GlfwCursorPosCallback, "glfwSetCursorPosCallback", callbacks::intercept_set_cursor_pos_callback);
plt_callback_hook!(glfwSetScrollCallback, GlfwScrollCallback, "glfwSetScrollCallback", callbacks::intercept_set_scroll_callback);
plt_callback_hook!(glfwSetCharCallback, GlfwCharCallback, "glfwSetCharCallback", callbacks::intercept_set_char_callback);
// LWJGL3 uses CharMods instead of plain Char
plt_callback_hook!(glfwSetCharModsCallback, GlfwCharModsCallback, "glfwSetCharModsCallback", callbacks::intercept_set_char_mods_callback);

// tracks cursor-capture state so sensitivity scaling knows FPS vs menu
#[no_mangle]
pub unsafe extern "C" fn glfwSetInputMode(window: GlfwWindow, mode: i32, value: i32) {
    static LOGGED: OnceLock<()> = OnceLock::new();
    LOGGED.get_or_init(|| {
        let ptr = libc::dlsym(libc::RTLD_NEXT, b"glfwSetInputMode\0".as_ptr() as *const libc::c_char);
        if ptr.is_null() {
            tracing::error!("glfwSetInputMode PLT: real not found");
        } else {
            tracing::info!("glfwSetInputMode PLT: resolved");
        }
    });
    callbacks::intercept_set_input_mode(window, mode, value)
}

// --- glfwGetCursorPos ---

// NOTE: must use the bundled libglfw pointer, not RTLD_NEXT, because
// the system libglfw doesn't know LWJGL3's window handles
static REAL_GET_CURSOR_POS_PTR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn store_real_get_cursor_pos(ptr: *mut c_void) {
    if !ptr.is_null() {
        REAL_GET_CURSOR_POS_PTR.store(ptr, Ordering::Release);
    }
}

// in FPS mode we return our tracked position; in menu mode we
// forward to the bundled libglfw with viewport offset correction
#[no_mangle]
pub unsafe extern "C" fn glfwGetCursorPos(
    window: GlfwWindow,
    xpos: *mut c_double,
    ypos: *mut c_double,
) {
    if callbacks::is_cursor_captured() {
        let (mx, my) = callbacks::mouse_position();
        if !xpos.is_null() { *xpos = mx; }
        if !ypos.is_null() { *ypos = my; }
    } else {
        let ptr = REAL_GET_CURSOR_POS_PTR.load(Ordering::Acquire);
        if !ptr.is_null() {
            let real: unsafe extern "C" fn(*mut c_void, *mut c_double, *mut c_double) =
                std::mem::transmute(ptr);
            real(window, xpos, ypos);
            // apply viewport centering offset so game gets viewport-space coords
            let (mw, mh) = crate::viewport_hook::get_mode_size();
            let (ow, oh) = crate::viewport_hook::get_original_size();
            if mw > 0 && ow > 0 && (mw != ow || mh != oh) {
                let cx = (ow as f64 - mw as f64) / 2.0;
                let cy = (oh as f64 - mh as f64) / 2.0;
                if !xpos.is_null() { *xpos -= cx; }
                if !ypos.is_null() { *ypos -= cy; }
            }
        } else {
            tracing::warn!("glfwGetCursorPos: bundled ptr not stored yet, returning zeros");
            if !xpos.is_null() { *xpos = 0.0; }
            if !ypos.is_null() { *ypos = 0.0; }
        }
    }
}

// --- glfwGetKey ---

type GlfwGetKeyFn = unsafe extern "C" fn(GlfwWindow, c_int) -> c_int;

static REAL_GET_KEY_PTR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn store_real_get_key(ptr: *mut c_void) {
    if !ptr.is_null() {
        REAL_GET_KEY_PTR.store(ptr, Ordering::Release);
    }
}

// applies reverse rebind so polling checks the physical key
#[no_mangle]
pub unsafe extern "C" fn glfwGetKey(window: GlfwWindow, key: c_int) -> c_int {
    use tuxinjector_input::glfw_types::MOUSE_BUTTON_OFFSET;

    // when GUI is open, report all keys as released to prevent input leak
    if callbacks::gui_is_visible() {
        return 0;
    }

    let ptr = REAL_GET_KEY_PTR.load(Ordering::Acquire);
    if ptr.is_null() {
        tracing::warn!("glfwGetKey: bundled ptr not stored yet, returning 0");
        return 0;
    }

    let physical = callbacks::physical_key_for(key);

    // rebind landed on a mouse button - route through glfwGetMouseButton
    if physical >= MOUSE_BUTTON_OFFSET as i32 {
        let mb_ptr = REAL_GET_MOUSE_BUTTON_PTR.load(Ordering::Acquire);
        if mb_ptr.is_null() { return 0; }
        let real_mb: GlfwGetMouseButtonFn = std::mem::transmute(mb_ptr);
        return real_mb(window, physical - MOUSE_BUTTON_OFFSET as c_int);
    }

    let real: GlfwGetKeyFn = std::mem::transmute(ptr);
    real(window, physical)
}

// --- glfwGetMouseButton ---

type GlfwGetMouseButtonFn = unsafe extern "C" fn(GlfwWindow, c_int) -> c_int;

static REAL_GET_MOUSE_BUTTON_PTR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn store_real_get_mouse_button(ptr: *mut c_void) {
    if !ptr.is_null() {
        REAL_GET_MOUSE_BUTTON_PTR.store(ptr, Ordering::Release);
    }
}

// reverse rebind for mouse button polling
#[no_mangle]
pub unsafe extern "C" fn glfwGetMouseButton(window: GlfwWindow, button: c_int) -> c_int {
    use tuxinjector_input::glfw_types::MOUSE_BUTTON_OFFSET;

    // when GUI is open, report all buttons as released to prevent ICM etc.
    if callbacks::gui_is_visible() {
        return 0; // GLFW_RELEASE
    }

    let ptr = REAL_GET_MOUSE_BUTTON_PTR.load(Ordering::Acquire);
    if ptr.is_null() {
        tracing::warn!("glfwGetMouseButton: bundled ptr not stored yet, returning 0");
        return 0;
    }

    // reverse lookup: if this button is the target of a key->mouse rebind,
    // poll the physical key instead
    let encoded = button + MOUSE_BUTTON_OFFSET as c_int;
    let physical = callbacks::physical_key_for(encoded);
    if physical != encoded && physical < MOUSE_BUTTON_OFFSET as i32 {
        let key_ptr = REAL_GET_KEY_PTR.load(Ordering::Acquire);
        if !key_ptr.is_null() {
            let real_key: GlfwGetKeyFn = std::mem::transmute(key_ptr);
            return real_key(window, physical);
        }
        return 0;
    }

    let real: GlfwGetMouseButtonFn = std::mem::transmute(ptr);
    real(window, button)
}
