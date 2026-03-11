// EGL/GLX window resize hooks -- shadows wl_egl_window_create/resize,
// glViewport, glBindFramebuffer, etc. to control game rendering dimensions.

use std::ffi::{c_char, c_int, c_uint, c_void};
#[cfg(target_os = "linux")]
use std::ffi::{c_long, c_ulong};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::OnceLock;

// --- Type aliases ---

type WlEglWindowCreateFn = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> *mut c_void;
type WlEglWindowResizeFn = unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int, c_int);
type GlfwFbSizeCb = unsafe extern "C" fn(window: *mut c_void, w: c_int, h: c_int);
type GlfwSetFbSizeCbFn =
    unsafe extern "C" fn(window: *mut c_void, callback: Option<GlfwFbSizeCb>) -> Option<GlfwFbSizeCb>;
type GlfwGetFbSizeFn = unsafe extern "C" fn(window: *mut c_void, w: *mut c_int, h: *mut c_int);
type GlViewportFn = unsafe extern "C" fn(x: c_int, y: c_int, w: c_int, h: c_int);
type GlScissorFn = unsafe extern "C" fn(x: c_int, y: c_int, w: c_int, h: c_int);
type GlBindFramebufferFn = unsafe extern "C" fn(target: c_uint, framebuffer: c_uint);
type GlDrawBufferFn = unsafe extern "C" fn(mode: c_uint);
type GlReadBufferFn = unsafe extern "C" fn(mode: c_uint);
type GlDrawBuffersFn = unsafe extern "C" fn(n: c_int, bufs: *const c_uint);
type GlGetIntegervFn = unsafe extern "C" fn(pname: c_uint, data: *mut c_int);
type GlBlitFramebufferFn = unsafe extern "C" fn(
    src_x0: c_int, src_y0: c_int, src_x1: c_int, src_y1: c_int,
    dst_x0: c_int, dst_y0: c_int, dst_x1: c_int, dst_y1: c_int,
    mask: c_uint, filter: c_uint,
);
type GlxGetProcAddressFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;

// --- GL constants ---

const GL_DRAW_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const GL_READ_FRAMEBUFFER_BINDING: u32 = 0x8CAA;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_BACK: u32 = 0x0405;
const GL_BACK_LEFT: u32 = 0x0402;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;

// X11/GLX path (when GAME_EGL_WINDOW is null)
#[cfg(target_os = "linux")]
type GlxGetCurrentDisplayFn = unsafe extern "C" fn() -> *mut c_void;
#[cfg(target_os = "linux")]
type GlfwGetX11WindowFn     = unsafe extern "C" fn(*mut c_void) -> c_ulong;
#[cfg(target_os = "linux")]
type XResizeWindowFn        = unsafe extern "C" fn(*mut c_void, c_ulong, c_uint, c_uint) -> c_int;
#[cfg(target_os = "linux")]
type XSyncFn                = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
#[cfg(target_os = "linux")]
type XFlushFn               = unsafe extern "C" fn(*mut c_void) -> c_int;
#[cfg(target_os = "linux")]
type XInternAtomFn          = unsafe extern "C" fn(*mut c_void, *const c_char, c_int) -> c_ulong;
#[cfg(target_os = "linux")]
type XSendEventFn           = unsafe extern "C" fn(*mut c_void, c_ulong, c_int, c_long, *const c_void) -> c_int;
#[cfg(target_os = "linux")]
type XDefaultRootWindowFn   = unsafe extern "C" fn(*mut c_void) -> c_ulong;
// GLFW window management (borderless toggle)
type GlfwSetWindowAttribFn  = unsafe extern "C" fn(*mut c_void, c_int, c_int);
#[cfg(target_os = "linux")]
type GlfwSetWindowSizeFn    = unsafe extern "C" fn(*mut c_void, c_int, c_int);
#[cfg(target_os = "linux")]
type GlfwSetWindowPosFn     = unsafe extern "C" fn(*mut c_void, c_int, c_int);
type GlfwGetWindowPosFn     = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int);
type GlfwGetWindowSizeFn    = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int);
#[cfg(target_os = "linux")]
type GlfwGetPrimaryMonitorFn = unsafe extern "C" fn() -> *mut c_void;

#[cfg(target_os = "linux")]
#[repr(C)]
struct GlfwVidMode {
    width: c_int,
    height: c_int,
    red_bits: c_int,
    green_bits: c_int,
    blue_bits: c_int,
    refresh_rate: c_int,
}

#[cfg(target_os = "linux")]
type GlfwGetVideoModeFn     = unsafe extern "C" fn(*mut c_void) -> *const GlfwVidMode;

const GLFW_DECORATED: c_int = 0x00020005;

// --- macOS Cocoa FFI for borderless toggle ---

#[cfg(target_os = "macos")]
type GlfwGetCocoaWindowFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
#[cfg(target_os = "macos")]
type ObjcGetClassFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
#[cfg(target_os = "macos")]
type SelRegisterNameFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;

// ARM64 can't just shove structs through variadic C calls, so we
// transmute objc_msgSend to the exact signature each call needs.
#[cfg(target_os = "macos")]
type MsgSendSetU64 = unsafe extern "C" fn(*mut c_void, *mut c_void, u64);
#[cfg(target_os = "macos")]
type MsgSendGetU64 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> u64;
#[cfg(target_os = "macos")]
type MsgSendGetPtr = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
#[cfg(target_os = "macos")]
type MsgSendSetFrameDisplay = unsafe extern "C" fn(*mut c_void, *mut c_void, NSRect, i8);
#[cfg(target_os = "macos")]
#[cfg(target_arch = "aarch64")]
type MsgSendGetRect = unsafe extern "C" fn(*mut c_void, *mut c_void) -> NSRect;
#[cfg(target_os = "macos")]
#[cfg(target_arch = "x86_64")]
type MsgSendStretGetRect = unsafe extern "C" fn(*mut NSRect, *mut c_void, *mut c_void);

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint { x: f64, y: f64 }
#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize { width: f64, height: f64 }
#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect { origin: NSPoint, size: NSSize }

// --- Cached real function pointers (resolved once) ---

static REAL_WL_EGL_WINDOW_CREATE: OnceLock<Option<WlEglWindowCreateFn>> = OnceLock::new();
static REAL_WL_EGL_WINDOW_RESIZE: OnceLock<Option<WlEglWindowResizeFn>> = OnceLock::new();

#[cfg(target_os = "linux")]
static REAL_GLX_GET_CURRENT_DISPLAY: OnceLock<Option<GlxGetCurrentDisplayFn>> = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_GLFW_GET_X11_WINDOW:     OnceLock<Option<GlfwGetX11WindowFn>>     = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_X_RESIZE_WINDOW:         OnceLock<Option<XResizeWindowFn>>        = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_X_SYNC:                  OnceLock<Option<XSyncFn>>                = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_X_FLUSH:                 OnceLock<Option<XFlushFn>>               = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_X_INTERN_ATOM:           OnceLock<Option<XInternAtomFn>>          = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_X_SEND_EVENT:            OnceLock<Option<XSendEventFn>>           = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_X_DEFAULT_ROOT_WINDOW:   OnceLock<Option<XDefaultRootWindowFn>>   = OnceLock::new();
static REAL_GLFW_SET_WINDOW_ATTRIB:  OnceLock<Option<GlfwSetWindowAttribFn>>  = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_GLFW_SET_WINDOW_SIZE:    OnceLock<Option<GlfwSetWindowSizeFn>>    = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_GLFW_SET_WINDOW_POS:     OnceLock<Option<GlfwSetWindowPosFn>>     = OnceLock::new();
static REAL_GLFW_GET_WINDOW_POS:     OnceLock<Option<GlfwGetWindowPosFn>>     = OnceLock::new();
static REAL_GLFW_GET_WINDOW_SIZE:    OnceLock<Option<GlfwGetWindowSizeFn>>    = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_GLFW_GET_PRIMARY_MONITOR: OnceLock<Option<GlfwGetPrimaryMonitorFn>> = OnceLock::new();
#[cfg(target_os = "linux")]
static REAL_GLFW_GET_VIDEO_MODE:     OnceLock<Option<GlfwGetVideoModeFn>>     = OnceLock::new();

static BORDERLESS_ACTIVE: AtomicBool = AtomicBool::new(false);
static BORDERLESS_TOGGLE_PENDING: AtomicBool = AtomicBool::new(false);
static SAVED_WINDOW_GEOM: std::sync::Mutex<Option<(i32, i32, u32, u32)>> = std::sync::Mutex::new(None);

// SAFETY: these are fn ptrs cached once at init, never written again
#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
struct SendPtr(*mut c_void);
#[cfg(target_os = "macos")]
unsafe impl Send for SendPtr {}
#[cfg(target_os = "macos")]
unsafe impl Sync for SendPtr {}

#[cfg(target_os = "macos")]
static REAL_GLFW_GET_COCOA_WINDOW: OnceLock<Option<GlfwGetCocoaWindowFn>> = OnceLock::new();
#[cfg(target_os = "macos")]
static OBJC_MSG_SEND_PTR: OnceLock<SendPtr> = OnceLock::new();
#[cfg(target_os = "macos")]
static OBJC_GET_CLASS_FN: OnceLock<Option<ObjcGetClassFn>> = OnceLock::new();
#[cfg(target_os = "macos")]
static SEL_REGISTER_NAME_FN: OnceLock<Option<SelRegisterNameFn>> = OnceLock::new();
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
static OBJC_MSG_SEND_STRET_PTR: OnceLock<SendPtr> = OnceLock::new();
#[cfg(target_os = "macos")]
static SAVED_MACOS_STYLE_MASK: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);
#[cfg(target_os = "macos")]
static SAVED_MACOS_FRAME: std::sync::Mutex<Option<NSRect>> = std::sync::Mutex::new(None);
#[cfg(target_os = "macos")]
static SAVED_MACOS_PRESENTATION: std::sync::Mutex<Option<u64>> = std::sync::Mutex::new(None);

fn get_real_wl_egl_window_create() -> Option<WlEglWindowCreateFn> {
    *REAL_WL_EGL_WINDOW_CREATE.get_or_init(|| {
        let name = b"wl_egl_window_create\0";
        let ptr = unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr() as *const libc::c_char) };
        if ptr.is_null() {
            tracing::info!("wl_egl_window_create not found -- pure X11/GLX");
            return None;
        }
        Some(unsafe { std::mem::transmute(ptr) })
    })
}

fn get_real_wl_egl_window_resize() -> Option<WlEglWindowResizeFn> {
    *REAL_WL_EGL_WINDOW_RESIZE.get_or_init(|| {
        let name = b"wl_egl_window_resize\0";
        let ptr = unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr() as *const libc::c_char) };
        if ptr.is_null() {
            tracing::info!("wl_egl_window_resize not found -- pure X11/GLX");
            return None;
        }
        Some(unsafe { std::mem::transmute(ptr) })
    })
}

// --- X11/GLX resolver helpers ---

/// Try RTLD_DEFAULT then dlopen a list of libs. Handles are leaked on purpose.
#[cfg(target_os = "linux")]
pub(crate) unsafe fn dlopen_find(libs: &[&[u8]], sym: &[u8]) -> *mut c_void {
    let ptr = libc::dlsym(libc::RTLD_DEFAULT, sym.as_ptr() as *const libc::c_char);
    if !ptr.is_null() {
        return ptr;
    }
    for &lib in libs {
        let handle = libc::dlopen(
            lib.as_ptr() as *const libc::c_char,
            libc::RTLD_LAZY | libc::RTLD_GLOBAL,
        );
        if handle.is_null() { continue; }
        let p = libc::dlsym(handle, sym.as_ptr() as *const libc::c_char);
        if !p.is_null() { return p; }
    }
    std::ptr::null_mut()
}

#[cfg(target_os = "linux")]
fn find_lib_paths_by_substring(sub: &str) -> Vec<std::ffi::CString> {
    let mut out = Vec::new();
    let maps = match std::fs::read_to_string("/proc/self/maps") {
        Ok(m) => m,
        Err(_) => return out,
    };
    for line in maps.lines() {
        let path = match line.split_whitespace().last() {
            Some(p) => p,
            None => continue,
        };
        if !path.starts_with('/') || !path.contains(sub) {
            continue;
        }
        if let Ok(cs) = std::ffi::CString::new(path) {
            out.push(cs);
        }
    }
    out
}

// check if an address lives inside our .so (i.e. us)
// cargo can produce either "libtuxinjector.so" or "tuxinjector.so"
#[cfg(target_os = "linux")]
fn is_own_library(addr: usize) -> bool {
    unsafe {
        let mut info: libc::Dl_info = std::mem::zeroed();
        if libc::dladdr(addr as *const c_void, &mut info) != 0 && !info.dli_fname.is_null() {
            let name = std::ffi::CStr::from_ptr(info.dli_fname).to_bytes();
            // match "libtuxinjector" (14 bytes) or "/tuxinjector." (13 bytes)
            if name.windows(14).any(|w| w == b"libtuxinjector") {
                return true;
            }
            if name.windows(13).any(|w| w == b"/tuxinjector.") {
                return true;
            }
        }
    }
    false
}

#[cfg(target_os = "linux")]
unsafe fn resolve_gl_sym(sym: &[u8], self_addr: usize) -> *mut u8 {
    // use real_dlsym (via dlvsym) to bypass our own dlsym hook -
    // libc::dlsym goes through our #[no_mangle] dlsym which stores
    // our PLT address into REAL_GL_* OnceLocks as a side effect
    use crate::dlsym_hook::{resolve_real_symbol_default, resolve_real_symbol_from};

    let is_self = |p: *mut c_void| -> bool {
        if p.is_null() { return true; }
        if p as usize == self_addr { return true; }
        let own = is_own_library(p as usize);
        if own {
            tracing::debug!(addr = ?p, "resolve_gl_sym: rejected (own library)");
        }
        own
    };

    // try RTLD_DEFAULT first (via real dlsym, not our hook)
    let ptr = resolve_real_symbol_default(sym);
    if !is_self(ptr) {
        tracing::info!(addr = ?ptr, "resolve_gl_sym: found via RTLD_DEFAULT");
        return ptr as *mut u8;
    }

    let libs: &[&[u8]] = &[
        b"libOpenGL.so.0\0", b"libGL.so.1\0",
        b"libGLX_mesa.so.0\0", b"libGLX.so.0\0", b"libGLdispatch.so.0\0",
    ];
    for &lib in libs {
        let handle = libc::dlopen(lib.as_ptr() as *const libc::c_char, libc::RTLD_LAZY | libc::RTLD_GLOBAL);
        if handle.is_null() { continue; }
        let p = resolve_real_symbol_from(handle, sym);
        if !is_self(p) {
            tracing::info!(addr = ?p, lib = ?std::ffi::CStr::from_ptr(lib.as_ptr() as *const _),
                "resolve_gl_sym: found via dlopen");
            return p as *mut u8;
        }
    }

    // last resort: scan /proc/self/maps for GL libraries
    for sub in ["libOpenGL", "libGL.so", "libGLX", "libGLdispatch"] {
        for cs in find_lib_paths_by_substring(sub) {
            let handle = libc::dlopen(cs.as_ptr(), libc::RTLD_LAZY | libc::RTLD_GLOBAL);
            if handle.is_null() { continue; }
            let p = resolve_real_symbol_from(handle, sym);
            if !is_self(p) {
                return p as *mut u8;
            }
        }
    }

    std::ptr::null_mut()
}

#[cfg(target_os = "linux")]
fn get_glx_current_display() -> Option<GlxGetCurrentDisplayFn> {
    *REAL_GLX_GET_CURRENT_DISPLAY.get_or_init(|| unsafe {
        let ptr = dlopen_find(
            &[b"libGL.so.1\0", b"libGL.so\0", b"libGLX.so.0\0"],
            b"glXGetCurrentDisplay\0",
        );
        if ptr.is_null() {
            tracing::warn!("glXGetCurrentDisplay not found");
            None
        } else {
            Some(std::mem::transmute(ptr))
        }
    })
}

// LWJGL loads GLFW with RTLD_LOCAL so we have to find it by scanning /proc/self/maps
#[cfg(target_os = "linux")]
fn find_glfw_lib_path() -> Option<std::ffi::CString> {
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    for line in maps.lines() {
        let path = line.split_whitespace().last()?;
        if !path.starts_with('/') { continue; }
        let fname = std::path::Path::new(path).file_name()?.to_str()?;
        if fname.starts_with("libglfw") && fname.contains(".so") {
            tracing::info!(path, "found GLFW library in /proc/self/maps");
            return std::ffi::CString::new(path).ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn get_glfw_get_x11_window() -> Option<GlfwGetX11WindowFn> {
    *REAL_GLFW_GET_X11_WINDOW.get_or_init(|| unsafe {
        let mut ptr = libc::dlsym(
            libc::RTLD_DEFAULT,
            b"glfwGetX11Window\0".as_ptr() as *const libc::c_char,
        );
        // slow path: open GLFW by path to bypass RTLD_LOCAL
        if ptr.is_null() {
            if let Some(cs) = find_glfw_lib_path() {
                let handle = libc::dlopen(cs.as_ptr(), libc::RTLD_LAZY);
                if !handle.is_null() {
                    ptr = libc::dlsym(handle, b"glfwGetX11Window\0".as_ptr() as *const libc::c_char);
                    tracing::info!(found = !ptr.is_null(), "glfwGetX11Window via path-based dlopen");
                }
            }
        }
        if ptr.is_null() {
            tracing::warn!("glfwGetX11Window not found -- GLX resize won't work");
            None
        } else {
            Some(std::mem::transmute(ptr))
        }
    })
}

#[cfg(target_os = "linux")]
fn get_x_resize_window() -> Option<XResizeWindowFn> {
    *REAL_X_RESIZE_WINDOW.get_or_init(|| unsafe {
        let ptr = dlopen_find(&[b"libX11.so.6\0", b"libX11.so\0"], b"XResizeWindow\0");
        if ptr.is_null() { tracing::warn!("XResizeWindow not found"); None }
        else { Some(std::mem::transmute(ptr)) }
    })
}

#[cfg(target_os = "linux")]
fn get_x_sync() -> Option<XSyncFn> {
    *REAL_X_SYNC.get_or_init(|| unsafe {
        let ptr = dlopen_find(&[b"libX11.so.6\0", b"libX11.so\0"], b"XSync\0");
        if ptr.is_null() { tracing::warn!("XSync not found"); None }
        else { Some(std::mem::transmute(ptr)) }
    })
}

#[cfg(target_os = "linux")]
fn get_x_flush() -> Option<XFlushFn> {
    *REAL_X_FLUSH.get_or_init(|| unsafe {
        let ptr = dlopen_find(&[b"libX11.so.6\0", b"libX11.so\0"], b"XFlush\0");
        if ptr.is_null() { None } else { Some(std::mem::transmute(ptr)) }
    })
}
#[cfg(target_os = "linux")]
fn get_x_intern_atom() -> Option<XInternAtomFn> {
    *REAL_X_INTERN_ATOM.get_or_init(|| unsafe {
        let ptr = dlopen_find(&[b"libX11.so.6\0", b"libX11.so\0"], b"XInternAtom\0");
        if ptr.is_null() { None } else { Some(std::mem::transmute(ptr)) }
    })
}
#[cfg(target_os = "linux")]
fn get_x_send_event() -> Option<XSendEventFn> {
    *REAL_X_SEND_EVENT.get_or_init(|| unsafe {
        let ptr = dlopen_find(&[b"libX11.so.6\0", b"libX11.so\0"], b"XSendEvent\0");
        if ptr.is_null() { None } else { Some(std::mem::transmute(ptr)) }
    })
}
#[cfg(target_os = "linux")]
fn get_x_default_root_window() -> Option<XDefaultRootWindowFn> {
    *REAL_X_DEFAULT_ROOT_WINDOW.get_or_init(|| unsafe {
        let ptr = dlopen_find(&[b"libX11.so.6\0", b"libX11.so\0"], b"XDefaultRootWindow\0");
        if ptr.is_null() { None } else { Some(std::mem::transmute(ptr)) }
    })
}

// --- GLFW symbol resolver (tries RTLD_DEFAULT, then path-based dlopen) ---

unsafe fn resolve_glfw_sym<T: Copy>(sym: &[u8], lock: &OnceLock<Option<T>>) -> Option<T> {
    *lock.get_or_init(|| {
        let ptr = libc::dlsym(libc::RTLD_DEFAULT, sym.as_ptr() as *const libc::c_char);
        if !ptr.is_null() {
            return Some(std::mem::transmute_copy(&ptr));
        }
        // Linux fallback: LWJGL loads GLFW with RTLD_LOCAL, scan /proc/self/maps
        #[cfg(target_os = "linux")]
        if let Some(cs) = find_glfw_lib_path() {
            let handle = libc::dlopen(cs.as_ptr(), libc::RTLD_LAZY);
            if !handle.is_null() {
                let p = libc::dlsym(handle, sym.as_ptr() as *const libc::c_char);
                if !p.is_null() {
                    return Some(std::mem::transmute_copy(&p));
                }
            }
        }
        tracing::warn!("GLFW symbol not found: {:?}", std::ffi::CStr::from_ptr(sym.as_ptr() as *const _));
        None
    })
}

fn get_glfw_set_window_attrib() -> Option<GlfwSetWindowAttribFn> {
    unsafe { resolve_glfw_sym(b"glfwSetWindowAttrib\0", &REAL_GLFW_SET_WINDOW_ATTRIB) }
}
#[cfg(target_os = "linux")]
fn get_glfw_set_window_size() -> Option<GlfwSetWindowSizeFn> {
    unsafe { resolve_glfw_sym(b"glfwSetWindowSize\0", &REAL_GLFW_SET_WINDOW_SIZE) }
}
#[cfg(target_os = "linux")]
fn get_glfw_set_window_pos() -> Option<GlfwSetWindowPosFn> {
    unsafe { resolve_glfw_sym(b"glfwSetWindowPos\0", &REAL_GLFW_SET_WINDOW_POS) }
}
fn get_glfw_get_window_pos() -> Option<GlfwGetWindowPosFn> {
    unsafe { resolve_glfw_sym(b"glfwGetWindowPos\0", &REAL_GLFW_GET_WINDOW_POS) }
}
fn get_glfw_get_window_size() -> Option<GlfwGetWindowSizeFn> {
    unsafe { resolve_glfw_sym(b"glfwGetWindowSize\0", &REAL_GLFW_GET_WINDOW_SIZE) }
}
#[cfg(target_os = "linux")]
fn get_glfw_get_primary_monitor() -> Option<GlfwGetPrimaryMonitorFn> {
    unsafe { resolve_glfw_sym(b"glfwGetPrimaryMonitor\0", &REAL_GLFW_GET_PRIMARY_MONITOR) }
}
#[cfg(target_os = "linux")]
fn get_glfw_get_video_mode() -> Option<GlfwGetVideoModeFn> {
    unsafe { resolve_glfw_sym(b"glfwGetVideoMode\0", &REAL_GLFW_GET_VIDEO_MODE) }
}

// --- macOS: objc runtime plumbing ---

#[cfg(target_os = "macos")]
fn get_glfw_get_cocoa_window() -> Option<GlfwGetCocoaWindowFn> {
    unsafe { resolve_glfw_sym(b"glfwGetCocoaWindow\0", &REAL_GLFW_GET_COCOA_WINDOW) }
}

#[cfg(target_os = "macos")]
fn get_objc_msg_send() -> *mut c_void {
    OBJC_MSG_SEND_PTR.get_or_init(|| unsafe {
        SendPtr(libc::dlsym(libc::RTLD_DEFAULT, b"objc_msgSend\0".as_ptr() as *const libc::c_char))
    }).0
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn get_objc_msg_send_stret() -> *mut c_void {
    OBJC_MSG_SEND_STRET_PTR.get_or_init(|| unsafe {
        SendPtr(libc::dlsym(libc::RTLD_DEFAULT, b"objc_msgSend_stret\0".as_ptr() as *const libc::c_char))
    }).0
}

#[cfg(target_os = "macos")]
fn get_objc_get_class() -> Option<ObjcGetClassFn> {
    *OBJC_GET_CLASS_FN.get_or_init(|| unsafe {
        let ptr = libc::dlsym(libc::RTLD_DEFAULT, b"objc_getClass\0".as_ptr() as *const libc::c_char);
        if ptr.is_null() { None } else { Some(std::mem::transmute(ptr)) }
    })
}

#[cfg(target_os = "macos")]
fn get_sel_register_name() -> Option<SelRegisterNameFn> {
    *SEL_REGISTER_NAME_FN.get_or_init(|| unsafe {
        let ptr = libc::dlsym(libc::RTLD_DEFAULT, b"sel_registerName\0".as_ptr() as *const libc::c_char);
        if ptr.is_null() { None } else { Some(std::mem::transmute(ptr)) }
    })
}

#[cfg(target_os = "macos")]
unsafe fn sel(name: &[u8]) -> *mut c_void {
    get_sel_register_name().unwrap()(name.as_ptr() as *const c_char)
}

#[cfg(target_os = "macos")]
unsafe fn msg_get_ptr(obj: *mut c_void, sel_name: &[u8]) -> *mut c_void {
    let send: MsgSendGetPtr = std::mem::transmute(get_objc_msg_send());
    send(obj, sel(sel_name))
}

#[cfg(target_os = "macos")]
unsafe fn msg_get_u64(obj: *mut c_void, sel_name: &[u8]) -> u64 {
    let send: MsgSendGetU64 = std::mem::transmute(get_objc_msg_send());
    send(obj, sel(sel_name))
}

#[cfg(target_os = "macos")]
unsafe fn msg_set_u64(obj: *mut c_void, sel_name: &[u8], val: u64) {
    let send: MsgSendSetU64 = std::mem::transmute(get_objc_msg_send());
    send(obj, sel(sel_name), val);
}

#[cfg(target_os = "macos")]
unsafe fn get_ns_app() -> *mut c_void {
    let cls = get_objc_get_class().unwrap()(b"NSApplication\0".as_ptr() as *const c_char);
    msg_get_ptr(cls, b"sharedApplication\0")
}

#[cfg(target_os = "macos")]
unsafe fn get_ns_rect(obj: *mut c_void, sel_name: &[u8]) -> NSRect {
    let s = sel(sel_name);
    #[cfg(target_arch = "aarch64")]
    {
        let send: MsgSendGetRect = std::mem::transmute(get_objc_msg_send());
        send(obj, s)
    }
    #[cfg(target_arch = "x86_64")]
    {
        let stret: MsgSendStretGetRect = std::mem::transmute(get_objc_msg_send_stret());
        let mut rect = std::mem::zeroed::<NSRect>();
        stret(&mut rect, obj, s);
        rect
    }
}

#[cfg(target_os = "macos")]
unsafe fn set_ns_frame(obj: *mut c_void, frame: NSRect) {
    let send: MsgSendSetFrameDisplay = std::mem::transmute(get_objc_msg_send());
    send(obj, sel(b"setFrame:display:\0"), frame, 1);
}

// defer to swap hook - can't mess with the window while we're inside glfwPollEvents
pub fn request_borderless_toggle() {
    BORDERLESS_TOGGLE_PENDING.store(true, Ordering::Release);
}

// check if a toggle was queued and do it
pub unsafe fn poll_borderless_toggle() {
    if BORDERLESS_TOGGLE_PENDING.compare_exchange(
        true, false, Ordering::AcqRel, Ordering::Relaxed,
    ).is_ok() {
        do_toggle_borderless();
    }
}

// Uses waywall's trick: send a dummy 1x1 resize first so GLFW actually
// processes the size change and fires Minecraft's framebuffer callback.
#[cfg(target_os = "linux")]
unsafe fn do_toggle_borderless() {
    let glfw_win = GAME_WINDOW.load(Ordering::Acquire);
    if glfw_win.is_null() {
        tracing::warn!("toggle_borderless: GLFW window not stored yet");
        return;
    }

    let Some(set_attrib) = get_glfw_set_window_attrib() else { return };
    let Some(set_size)   = get_glfw_set_window_size()   else { return };
    let Some(set_pos)    = get_glfw_set_window_pos()    else { return };
    let Some(get_pos)    = get_glfw_get_window_pos()    else { return };
    let Some(get_size)   = get_glfw_get_window_size()   else { return };
    let Some(get_mon)    = get_glfw_get_primary_monitor() else { return };
    let Some(get_mode)   = get_glfw_get_video_mode()    else { return };

    let was_borderless = BORDERLESS_ACTIVE.load(Ordering::Relaxed);

    if !was_borderless {
        // save current geometry
        let (mut x, mut y) = (0i32, 0i32);
        let (mut w, mut h) = (0i32, 0i32);
        get_pos(glfw_win, &mut x, &mut y);
        get_size(glfw_win, &mut w, &mut h);
        tracing::info!(x, y, w, h, "toggle_borderless: saved geometry");
        *SAVED_WINDOW_GEOM.lock().unwrap() = Some((x, y, w as u32, h as u32));

        let monitor = get_mon();
        if monitor.is_null() {
            tracing::warn!("toggle_borderless: glfwGetPrimaryMonitor null");
            return;
        }
        let mode = get_mode(monitor);
        if mode.is_null() {
            tracing::warn!("toggle_borderless: glfwGetVideoMode null");
            return;
        }
        let (sw, sh) = ((*mode).width, (*mode).height);

        set_attrib(glfw_win, GLFW_DECORATED, 0);
        set_pos(glfw_win, 0, 0);

        // dummy 1x1 resize to force GLFW to process the change
        set_size(glfw_win, 1, 1);
        fire_borderless_fb_cb(glfw_win, 1, 1);

        set_size(glfw_win, sw, sh);
        fire_borderless_fb_cb(glfw_win, sw, sh);

        // tell the compositor this is fullscreen (hides bars on Hyprland/XWayland)
        set_x11_fullscreen_state(true);

        tracing::info!(sw, sh, "toggle_borderless: entered borderless fullscreen");
    } else {
        set_x11_fullscreen_state(false);
        set_attrib(glfw_win, GLFW_DECORATED, 1);

        if let Some((x, y, w, h)) = SAVED_WINDOW_GEOM.lock().unwrap().take() {
            set_size(glfw_win, 1, 1);
            fire_borderless_fb_cb(glfw_win, 1, 1);

            set_size(glfw_win, w as c_int, h as c_int);
            set_pos(glfw_win, x, y);
            fire_borderless_fb_cb(glfw_win, w as c_int, h as c_int);
            tracing::info!(x, y, w, h, "toggle_borderless: restored geometry");
        }

        tracing::info!("toggle_borderless: exited borderless");
    }

    BORDERLESS_ACTIVE.store(!was_borderless, Ordering::Relaxed);
}

// macOS: rip off the title bar, hide dock + menu bar, stretch window to fill screen
#[cfg(target_os = "macos")]
unsafe fn do_toggle_borderless() {
    let glfw_win = GAME_WINDOW.load(Ordering::Acquire);
    if glfw_win.is_null() {
        tracing::warn!("toggle_borderless: GLFW window not stored yet");
        return;
    }

    let Some(set_attrib) = get_glfw_set_window_attrib() else { return };
    let Some(get_pos)    = get_glfw_get_window_pos()    else { return };
    let Some(get_size)   = get_glfw_get_window_size()   else { return };
    let Some(get_cocoa)  = get_glfw_get_cocoa_window()  else {
        tracing::warn!("toggle_borderless: glfwGetCocoaWindow not found");
        return;
    };

    if get_objc_msg_send().is_null() || get_sel_register_name().is_none() || get_objc_get_class().is_none() {
        tracing::warn!("toggle_borderless: objc runtime not available");
        return;
    }

    let ns_window = get_cocoa(glfw_win);
    if ns_window.is_null() {
        tracing::warn!("toggle_borderless: glfwGetCocoaWindow returned null");
        return;
    }

    let was_borderless = BORDERLESS_ACTIVE.load(Ordering::Relaxed);

    if !was_borderless {
        // save geometry so we can put it back when they exit borderless
        let (mut x, mut y) = (0i32, 0i32);
        let (mut w, mut h) = (0i32, 0i32);
        get_pos(glfw_win, &mut x, &mut y);
        get_size(glfw_win, &mut w, &mut h);
        tracing::info!(x, y, w, h, "toggle_borderless: saved geometry");
        *SAVED_WINDOW_GEOM.lock().unwrap() = Some((x, y, w as u32, h as u32));

        // save the native cocoa stuff we're about to stomp on
        *SAVED_MACOS_STYLE_MASK.lock().unwrap() = Some(msg_get_u64(ns_window, b"styleMask\0"));
        *SAVED_MACOS_FRAME.lock().unwrap() = Some(get_ns_rect(ns_window, b"frame\0"));
        let ns_app = get_ns_app();
        *SAVED_MACOS_PRESENTATION.lock().unwrap() = Some(msg_get_u64(ns_app, b"presentationOptions\0"));

        // kill the title bar
        set_attrib(glfw_win, GLFW_DECORATED, 0);
        msg_set_u64(ns_window, b"setStyleMask:\0", 0); // NSBorderlessWindowMask

        // grab the full screen rect (including where the menu bar lives)
        let screen = msg_get_ptr(ns_window, b"screen\0");
        let screen_frame = get_ns_rect(screen, b"frame\0");

        set_ns_frame(ns_window, screen_frame);

        // shove dock + menu bar out of the way (AutoHideDock|AutoHideMenuBar)
        msg_set_u64(ns_app, b"setPresentationOptions:\0", 1 | 4);

        // float above menu bar so it doesn't peek through
        msg_set_u64(ns_window, b"setLevel:\0", 3);

        // tell GLFW about the size change (retina: pixels != points)
        let (sx, sy) = content_scale();
        let fb_w = (screen_frame.size.width as f32 * sx) as c_int;
        let fb_h = (screen_frame.size.height as f32 * sy) as c_int;
        fire_borderless_fb_cb(glfw_win, fb_w, fb_h);

        tracing::info!(
            w = screen_frame.size.width, h = screen_frame.size.height,
            fb_w, fb_h, "toggle_borderless: entered borderless (macOS)"
        );
    } else {
        let ns_app = get_ns_app();

        // bring the dock and menu bar back
        if let Some(opts) = SAVED_MACOS_PRESENTATION.lock().unwrap().take() {
            msg_set_u64(ns_app, b"setPresentationOptions:\0", opts);
        }

        // back to normal window level
        msg_set_u64(ns_window, b"setLevel:\0", 0); // NSNormalWindowLevel

        // restore title bar
        if let Some(mask) = SAVED_MACOS_STYLE_MASK.lock().unwrap().take() {
            msg_set_u64(ns_window, b"setStyleMask:\0", mask);
        }

        // let GLFW know too
        set_attrib(glfw_win, GLFW_DECORATED, 1);

        // put window back where it was
        if let Some(frame) = SAVED_MACOS_FRAME.lock().unwrap().take() {
            set_ns_frame(ns_window, frame);
        }

        // poke the fb callback so MC re-renders at the old size
        if let Some((_x, _y, w, h)) = SAVED_WINDOW_GEOM.lock().unwrap().take() {
            fire_borderless_fb_cb(glfw_win, w as c_int, h as c_int);
            tracing::info!(w, h, "toggle_borderless: restored geometry (macOS)");
        }

        tracing::info!("toggle_borderless: exited borderless (macOS)");
    }

    BORDERLESS_ACTIVE.store(!was_borderless, Ordering::Relaxed);
}

unsafe fn fire_borderless_fb_cb(glfw_win: *mut c_void, w: c_int, h: c_int) {
    let cb_ptr = GAME_FB_SIZE_CB.load(Ordering::Acquire);
    if !cb_ptr.is_null() {
        let cb: GlfwFbSizeCb = std::mem::transmute(cb_ptr);
        cb(glfw_win, w, h);
    }
}

// Set/unset _NET_WM_STATE_FULLSCREEN via X11 client message.
// This tells Hyprland/XWayland to hide bars without going through GLFW's
// own fullscreen path (which would fight with our resize hooks).
#[cfg(target_os = "linux")]
unsafe fn set_x11_fullscreen_state(fullscreen: bool) {
    let Some(get_dpy) = get_glx_current_display()   else { return };
    let Some(get_win) = get_glfw_get_x11_window()   else { return };
    let Some(intern)  = get_x_intern_atom()          else { return };
    let Some(send)    = get_x_send_event()           else { return };
    let Some(root_fn) = get_x_default_root_window()  else { return };
    let Some(flush)   = get_x_flush()                else { return };

    let dpy = get_dpy();
    let glfw_win = GAME_WINDOW.load(Ordering::Acquire);
    if dpy.is_null() || glfw_win.is_null() { return; }

    let x11_win = get_win(glfw_win);
    if x11_win == 0 { return; }

    let wm_state = intern(dpy, b"_NET_WM_STATE\0".as_ptr() as *const c_char, 0);
    let wm_fs = intern(dpy, b"_NET_WM_STATE_FULLSCREEN\0".as_ptr() as *const c_char, 0);
    let root = root_fn(dpy);

    #[repr(C)]
    struct XClientMessageEvent {
        type_: c_int,
        serial: c_ulong,
        send_event: c_int,
        display: *mut c_void,
        window: c_ulong,
        message_type: c_ulong,
        format: c_int,
        data: [c_long; 5],
    }

    let action: c_long = if fullscreen { 1 } else { 0 };
    let ev = XClientMessageEvent {
        type_: 33, // ClientMessage
        serial: 0,
        send_event: 1,
        display: dpy,
        window: x11_win,
        message_type: wm_state,
        format: 32,
        data: [action, wm_fs as c_long, 0, 1, 0],
    };

    let mask: c_long = (1 << 20) | (1 << 19); // SubstructureRedirect | SubstructureNotify
    send(dpy, root, 0, mask, &ev as *const _ as *const c_void);
    flush(dpy);

    tracing::debug!(fullscreen, x11_win, "set _NET_WM_STATE_FULLSCREEN");
}

#[cfg(target_os = "linux")]
unsafe fn try_x11_window_resize(mode_w: u32, mode_h: u32, orig_w: u32, orig_h: u32) {
    let Some(get_dpy)  = get_glx_current_display()  else {
        tracing::warn!("x11_resize: glXGetCurrentDisplay not found");
        return;
    };
    let Some(get_win)  = get_glfw_get_x11_window()  else {
        tracing::warn!("x11_resize: glfwGetX11Window not found");
        return;
    };
    let Some(x_resize) = get_x_resize_window()      else {
        tracing::warn!("x11_resize: XResizeWindow not found");
        return;
    };
    let Some(x_sync)   = get_x_sync()               else {
        tracing::warn!("x11_resize: XSync not found");
        return;
    };

    let dpy = get_dpy();
    let glfw_win = GAME_WINDOW.load(Ordering::Acquire);
    if dpy.is_null() || glfw_win.is_null() {
        tracing::warn!(dpy = ?dpy, glfw_win = ?glfw_win, "x11_resize: null display or window");
        return;
    }
    let x11_win = get_win(glfw_win);
    if x11_win == 0 {
        tracing::warn!("x11_resize: glfwGetX11Window returned 0");
        return;
    }

    let (new_w, new_h) = if is_oversized(mode_w, mode_h, orig_w, orig_h) {
        tracing::info!(mode_w, mode_h, "growing X11 window for oversized GLX mode");
        (mode_w, mode_h)
    } else {
        tracing::info!(orig_w, orig_h, "restoring X11 window to physical size");
        (orig_w, orig_h)
    };

    let ret = x_resize(dpy, x11_win, new_w as c_uint, new_h as c_uint);
    x_sync(dpy, 0);
    tracing::info!(new_w, new_h, x11_win, ret, "XResizeWindow done");
}

// macOS no-op: glfwSetWindowSize in fullscreen causes a resize-then-snap-back
// that clobbers our mode dims. Viewport hook + border clearing are enough.
#[cfg(target_os = "macos")]
unsafe fn try_glfw_window_resize(_mode_w: u32, _mode_h: u32, _orig_w: u32, _orig_h: u32) {
    // no-op on purpose
}

#[allow(dead_code)]
pub fn is_glx_path() -> bool {
    GAME_EGL_WINDOW.load(Ordering::Acquire).is_null()
}

pub fn is_gl_viewport_hooked() -> bool {
    GL_VIEWPORT_SEEN.load(Ordering::Relaxed)
}

// Retina scale factor (fb pixels / window points), cached once
pub unsafe fn content_scale() -> (f32, f32) {
    static CACHED: OnceLock<(f32, f32)> = OnceLock::new();
    *CACHED.get_or_init(|| {
        let glfw_win = GAME_WINDOW.load(Ordering::Acquire);
        if glfw_win.is_null() { return (1.0, 1.0); }

        // gotta use the real glfwGetFramebufferSize, not our hooked one
        let real_get_fb = REAL_GET_FB_SIZE.load(Ordering::Acquire);
        if real_get_fb.is_null() { return (1.0, 1.0); }
        let get_fb: GlfwGetFbSizeFn = std::mem::transmute(real_get_fb);
        let (mut fw, mut fh) = (0i32, 0i32);
        get_fb(glfw_win, &mut fw, &mut fh);

        let Some(get_win_size) = get_glfw_get_window_size() else { return (1.0, 1.0) };
        let (mut ww, mut wh) = (0i32, 0i32);
        get_win_size(glfw_win, &mut ww, &mut wh);

        if fw <= 0 || fh <= 0 || ww <= 0 || wh <= 0 { return (1.0, 1.0); }

        let sx = fw as f32 / ww as f32;
        let sy = fh as f32 / wh as f32;
        tracing::info!(sx, sy, fb_w = fw, fb_h = fh, win_w = ww, win_h = wh,
            "content_scale cached");
        (sx, sy)
    })
}

// --- Captured game state ---

static GAME_EGL_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static GAME_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static GAME_FB_SIZE_CB: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static REAL_SET_FB_SIZE_CB: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static REAL_GET_FB_SIZE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// --- Size state (packed into u64 for atomic ops) ---

static ORIGINAL_DIMS: AtomicU64 = AtomicU64::new(0);
static MODE_DIMS: AtomicU64 = AtomicU64::new(0);

#[inline] fn pack(w: u32, h: u32) -> u64 { (w as u64) << 32 | h as u64 }
#[inline] fn unpack(v: u64) -> (u32, u32) { ((v >> 32) as u32, v as u32) }

// --- Setters (called from dlsym_hook) ---

pub fn store_real_set_fb_size_cb(ptr: *mut c_void) {
    REAL_SET_FB_SIZE_CB.store(ptr, Ordering::Release);
}

pub fn store_real_get_fb_size(ptr: *mut c_void) {
    REAL_GET_FB_SIZE.store(ptr, Ordering::Release);
}

// --- PLT hooks: wl_egl_window ---

#[no_mangle]
pub unsafe extern "C" fn wl_egl_window_create(
    surface: *mut c_void, width: c_int, height: c_int,
) -> *mut c_void {
    let Some(real) = get_real_wl_egl_window_create() else {
        return std::ptr::null_mut();
    };
    let window = real(surface, width, height);
    if !window.is_null() {
        GAME_EGL_WINDOW.store(window, Ordering::Release);
        ORIGINAL_DIMS.store(pack(width as u32, height as u32), Ordering::Release);
        tracing::info!(width, height, "captured wl_egl_window (original size stored)");
    }
    window
}

// Clamp EGL surface to physical size during mode resizes so the compositor
// doesn't try to allocate a giant buffer we don't need.
#[no_mangle]
pub unsafe extern "C" fn wl_egl_window_resize(
    window: *mut c_void, width: c_int, height: c_int,
    dx: c_int, dy: c_int,
) {
    let Some(real) = get_real_wl_egl_window_resize() else { return };
    let (mw, mh) = unpack(MODE_DIMS.load(Ordering::Acquire));
    let mw = mw as c_int;
    let mh = mh as c_int;

    if mw > 0 && mh > 0 && window == GAME_EGL_WINDOW.load(Ordering::Acquire) {
        let (ow, oh) = unpack(ORIGINAL_DIMS.load(Ordering::Acquire));
        let ow = ow as c_int;
        let oh = oh as c_int;
        if ow > 0 && oh > 0 {
            tracing::trace!(mw, mh, ow, oh, "wl_egl_window_resize: clamping to physical");
            real(window, ow, oh, dx, dy);
        } else {
            real(window, width, height, dx, dy);
        }
    } else {
        real(window, width, height, dx, dy);
    }
}

// --- glViewport hook ---

static REAL_GL_VIEWPORT: OnceLock<GlViewportFn> = OnceLock::new();
static GL_VIEWPORT_SEEN: AtomicBool = AtomicBool::new(false);
static REAL_GL_SCISSOR: OnceLock<GlScissorFn> = OnceLock::new();
static REAL_GL_BIND_FRAMEBUFFER: OnceLock<GlBindFramebufferFn> = OnceLock::new();
static REAL_GL_BIND_FRAMEBUFFER_COPY: OnceLock<GlBindFramebufferFn> = OnceLock::new();
static REAL_GL_BIND_FRAMEBUFFER_EXT: OnceLock<GlBindFramebufferFn> = OnceLock::new();
static REAL_GL_BIND_FRAMEBUFFER_ARB: OnceLock<GlBindFramebufferFn> = OnceLock::new();
static REAL_GL_DRAW_BUFFER: OnceLock<GlDrawBufferFn> = OnceLock::new();
static REAL_GL_READ_BUFFER: OnceLock<GlReadBufferFn> = OnceLock::new();
static REAL_GL_DRAW_BUFFERS: OnceLock<GlDrawBuffersFn> = OnceLock::new();
static REAL_GL_BLIT_FRAMEBUFFER: OnceLock<GlBlitFramebufferFn> = OnceLock::new();
#[allow(dead_code)]
static REAL_GL_BLIT_FRAMEBUFFER_EXT: OnceLock<GlBlitFramebufferFn> = OnceLock::new();
#[allow(dead_code)]
static REAL_GL_BLIT_FRAMEBUFFER_ARB: OnceLock<GlBlitFramebufferFn> = OnceLock::new();

// rwx copy of Mesa's glViewport, created before we inline-patch it
static REAL_GL_VIEWPORT_COPY: OnceLock<GlViewportFn> = OnceLock::new();

static REAL_GL_GET_INTEGERV: OnceLock<Option<GlGetIntegervFn>> = OnceLock::new();

fn get_gl_get_integerv() -> Option<GlGetIntegervFn> {
    *REAL_GL_GET_INTEGERV.get_or_init(|| unsafe {
        let name = b"glGetIntegerv\0".as_ptr() as *const libc::c_char;
        // RTLD_NEXT works on Linux (Mesa), but on macOS GL lives in
        // OpenGL.framework which is only visible through RTLD_DEFAULT
        #[cfg(target_os = "macos")]
        let ptr = libc::dlsym(libc::RTLD_DEFAULT, name);
        #[cfg(target_os = "linux")]
        let ptr = libc::dlsym(libc::RTLD_NEXT, name);
        if ptr.is_null() { None } else { Some(std::mem::transmute(ptr)) }
    })
}

pub fn store_real_gl_viewport(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_VIEWPORT.get().is_none();
        REAL_GL_VIEWPORT.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_VIEWPORT stored"); }
    }
}

pub fn store_real_gl_scissor(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_SCISSOR.get().is_none();
        REAL_GL_SCISSOR.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_SCISSOR stored"); }
    }
}

pub fn store_real_gl_bind_framebuffer(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_BIND_FRAMEBUFFER.get().is_none();
        REAL_GL_BIND_FRAMEBUFFER.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_BIND_FRAMEBUFFER stored"); }
    }
}

pub fn store_real_gl_bind_framebuffer_ext(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_BIND_FRAMEBUFFER_EXT.get().is_none();
        REAL_GL_BIND_FRAMEBUFFER_EXT.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_BIND_FRAMEBUFFER_EXT stored"); }
    }
}

pub fn store_real_gl_bind_framebuffer_arb(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_BIND_FRAMEBUFFER_ARB.get().is_none();
        REAL_GL_BIND_FRAMEBUFFER_ARB.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_BIND_FRAMEBUFFER_ARB stored"); }
    }
}

pub fn store_real_gl_draw_buffer(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_DRAW_BUFFER.get().is_none();
        REAL_GL_DRAW_BUFFER.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_DRAW_BUFFER stored"); }
    }
}

pub fn store_real_gl_read_buffer(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_READ_BUFFER.get().is_none();
        REAL_GL_READ_BUFFER.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_READ_BUFFER stored"); }
    }
}

pub fn store_real_gl_draw_buffers(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_DRAW_BUFFERS.get().is_none();
        REAL_GL_DRAW_BUFFERS.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_DRAW_BUFFERS stored"); }
    }
}

pub fn store_real_gl_blit_framebuffer(ptr: *mut c_void) {
    if !ptr.is_null() {
        let first = REAL_GL_BLIT_FRAMEBUFFER.get().is_none();
        REAL_GL_BLIT_FRAMEBUFFER.get_or_init(|| unsafe { std::mem::transmute(ptr) });
        if first { tracing::info!(ptr = ?ptr, "REAL_GL_BLIT_FRAMEBUFFER stored"); }
    }
}

// --- Lazy resolvers for GL functions (try stored ptr first, then RTLD_NEXT) ---

macro_rules! lazy_gl_resolve {
    ($fn_name:ident, $lock:ident, $sym:literal, $ty:ty) => {
        fn $fn_name() -> Option<$ty> {
            if let Some(f) = $lock.get() { return Some(*f); }
            let ptr = crate::dlsym_hook::resolve_real_symbol($sym);
            if ptr.is_null() {
                tracing::warn!(concat!(stringify!($fn_name), ": failed to resolve"));
                return None;
            }
            let f: $ty = unsafe { std::mem::transmute(ptr) };
            $lock.get_or_init(|| f);
            Some(f)
        }
    };
}

lazy_gl_resolve!(get_real_gl_draw_buffer, REAL_GL_DRAW_BUFFER, b"glDrawBuffer\0", GlDrawBufferFn);
lazy_gl_resolve!(get_real_gl_read_buffer, REAL_GL_READ_BUFFER, b"glReadBuffer\0", GlReadBufferFn);
lazy_gl_resolve!(get_real_gl_draw_buffers, REAL_GL_DRAW_BUFFERS, b"glDrawBuffers\0", GlDrawBuffersFn);
lazy_gl_resolve!(get_real_gl_blit_framebuffer, REAL_GL_BLIT_FRAMEBUFFER, b"glBlitFramebuffer\0", GlBlitFramebufferFn);
lazy_gl_resolve!(get_real_gl_scissor, REAL_GL_SCISSOR, b"glScissor\0", GlScissorFn);
lazy_gl_resolve!(get_real_gl_bind_framebuffer_ext, REAL_GL_BIND_FRAMEBUFFER_EXT, b"glBindFramebufferEXT\0", GlBindFramebufferFn);
lazy_gl_resolve!(get_real_gl_bind_framebuffer_arb, REAL_GL_BIND_FRAMEBUFFER_ARB, b"glBindFramebufferARB\0", GlBindFramebufferFn);

// Mesa dispatch stub detection: [endbr64] mov rax,[fs:0] ; jmp [rax+imm]
#[cfg(target_os = "linux")]
fn looks_like_dispatch_stub(bytes: &[u8]) -> bool {
    if bytes.len() < 12 { return false; }
    let mut i = 0usize;
    if bytes.starts_with(&[0xf3, 0x0f, 0x1e, 0xfa]) { i += 4; }
    if bytes.len() < i + 9 + 2 { return false; }
    if bytes[i..i + 5] != [0x64, 0x48, 0x8b, 0x04, 0x25] { return false; }
    i += 9;
    if bytes[i] != 0xff { return false; }
    matches!(bytes[i + 1], 0xa0 | 0x60)
}

unsafe fn bind_fb_with(real: GlBindFramebufferFn, target: c_uint, fb: c_uint) {
    let mut actual = fb;
    let mut redirected = false;
    if fb == 0 && crate::virtual_fb::should_redirect_default() {
        let offscreen = crate::virtual_fb::virtual_fbo();
        if offscreen != 0 {
            actual = offscreen;
            redirected = true;
            crate::virtual_fb::mark_used();
        }
    }
    real(target, actual);

    // when redirected to offscreen FBO, fix up draw/read buffers
    if redirected {
        if target == GL_FRAMEBUFFER || target == GL_DRAW_FRAMEBUFFER {
            if let Some(draw_buf) = get_real_gl_draw_buffer() {
                draw_buf(GL_COLOR_ATTACHMENT0);
            }
        }
        if target == GL_FRAMEBUFFER || target == GL_READ_FRAMEBUFFER {
            if let Some(read_buf) = get_real_gl_read_buffer() {
                read_buf(GL_COLOR_ATTACHMENT0);
            }
        }
    }
}

/// Inline-patch Mesa's glViewport with a 12-byte absolute jmp to our hook
#[cfg(target_os = "linux")]
pub unsafe fn install_glviewport_inline_hook() {
    if REAL_GL_VIEWPORT_COPY.get().is_some() { return; }

    let hook_self = hooked_gl_viewport as *const () as usize;

    let from_egl: *mut u8 = REAL_GL_VIEWPORT
        .get()
        .map(|f| std::mem::transmute::<GlViewportFn, usize>(*f) as *mut u8)
        .filter(|&p| !p.is_null() && p as usize != hook_self)
        .unwrap_or(std::ptr::null_mut());

    let from_scan: *mut u8 = resolve_gl_sym(b"glViewport\0", hook_self);

    let real_mesa: *mut u8 = [from_egl, from_scan]
        .iter().copied()
        .find(|&p| !p.is_null())
        .unwrap_or(std::ptr::null_mut());

    tracing::info!(
        real_mesa = ?real_mesa, from_egl = ?from_egl, from_scan = ?from_scan,
        hook_self = ?hook_self,
        "install_glviewport_inline_hook: address resolution"
    );

    if real_mesa.is_null() {
        tracing::warn!("install_glviewport_inline_hook: no real address -- aborting");
        return;
    }

    let head = std::slice::from_raw_parts(real_mesa, 16);
    let copy_len: usize;

    if looks_like_dispatch_stub(head) {
        copy_len = 256;
        tracing::info!("glViewport hook: dispatch stub detected, 256-byte copy");
    } else if head[0..7] == [0x48, 0x81, 0xEC, 0xB8, 0x00, 0x00, 0x00] && head[7] == 0x89 {
        // Mesa 25.x real impl prologue (15-byte clean boundary)
        copy_len = 15;
        tracing::info!("glViewport hook: real impl prologue, 15-byte trampoline");
    } else {
        tracing::warn!(first16 = ?head, "glViewport hook: unknown prologue, skipping");
        return;
    }

    // allocate trampoline page
    let tramp_alloc = if copy_len == 256 { 256 } else { copy_len + 12 };
    let tramp_mem = libc::mmap(
        std::ptr::null_mut(), tramp_alloc,
        libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
        libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0,
    );
    if tramp_mem == libc::MAP_FAILED {
        tracing::warn!("glViewport hook: mmap failed");
        return;
    }

    let tramp = tramp_mem as *mut u8;
    if copy_len == 256 {
        std::ptr::copy_nonoverlapping(real_mesa, tramp, 256);
    } else {
        std::ptr::copy_nonoverlapping(real_mesa, tramp, copy_len);
        let cont_addr = (real_mesa as u64) + copy_len as u64;
        let jmp = tramp.add(copy_len);
        *jmp.add(0) = 0x48; *jmp.add(1) = 0xB8;  // movabs rax, imm64
        *(jmp.add(2) as *mut u64) = cont_addr;
        *jmp.add(10) = 0xFF; *jmp.add(11) = 0xE0; // jmp rax
    }

    tracing::debug!(copy = ?tramp_mem, first12 = ?std::slice::from_raw_parts(real_mesa, 12),
        copy_len, "glViewport hook: trampoline built");

    let copy_fn: GlViewportFn = std::mem::transmute(tramp_mem);
    REAL_GL_VIEWPORT_COPY.get_or_init(|| copy_fn);

    // write 12-byte patch over the original
    let hook_addr = hook_self as u64;
    let patch: [u8; 12] = [
        0x48, 0xB8,
        (hook_addr      ) as u8, (hook_addr >>  8) as u8,
        (hook_addr >> 16) as u8, (hook_addr >> 24) as u8,
        (hook_addr >> 32) as u8, (hook_addr >> 40) as u8,
        (hook_addr >> 48) as u8, (hook_addr >> 56) as u8,
        0xFF, 0xE0,
    ];

    let page_sz = libc::sysconf(libc::_SC_PAGESIZE) as usize;
    let page = (real_mesa as usize) & !(page_sz - 1);
    if libc::mprotect(page as *mut c_void, page_sz, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC) != 0 {
        let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
        tracing::warn!(target = ?real_mesa, errno, "glViewport hook: mprotect failed");
    }
    tracing::warn!(real_mesa = ?real_mesa, hook = ?hook_addr, "glViewport hook: writing 12-byte patch");
    std::ptr::copy_nonoverlapping(patch.as_ptr(), real_mesa, 12);
    libc::mprotect(page as *mut c_void, page_sz, libc::PROT_READ | libc::PROT_EXEC);

    std::sync::atomic::fence(std::sync::atomic::Ordering::SeqCst);

    let verify = std::slice::from_raw_parts(real_mesa, 12);
    tracing::warn!(real_mesa = ?real_mesa, hook = ?hook_addr, copy = ?tramp_mem,
        verify = ?verify, "glViewport hook: patched -- verify bytes should start with 48 B8");
}

/// Same technique for glBindFramebuffer
#[cfg(target_os = "linux")]
pub unsafe fn install_glbindframebuffer_inline_hook() {
    if REAL_GL_BIND_FRAMEBUFFER_COPY.get().is_some() { return; }

    let hook_self = glBindFramebuffer as *const () as usize;
    let from_scan: *mut u8 = resolve_gl_sym(b"glBindFramebuffer\0", hook_self);

    tracing::info!(real_mesa = ?from_scan, hook_self = ?hook_self,
        "install_glbindframebuffer_inline_hook");

    if from_scan.is_null() {
        tracing::warn!("glBindFramebuffer hook: no real address -- aborting");
        return;
    }

    let head = std::slice::from_raw_parts(from_scan, 16);
    if !looks_like_dispatch_stub(head) {
        tracing::warn!(first16 = ?head, "glBindFramebuffer hook: no dispatch stub -- skipping");
        return;
    }

    const COPY_SZ: usize = 256;
    let copy_mem = libc::mmap(
        std::ptr::null_mut(), COPY_SZ,
        libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
        libc::MAP_PRIVATE | libc::MAP_ANONYMOUS, -1, 0,
    );
    if copy_mem == libc::MAP_FAILED {
        tracing::warn!("glBindFramebuffer hook: mmap failed");
        return;
    }

    std::ptr::copy_nonoverlapping(from_scan, copy_mem as *mut u8, COPY_SZ);
    tracing::debug!(copy = ?copy_mem, first12 = ?std::slice::from_raw_parts(from_scan, 12),
        "glBindFramebuffer hook: stub copied");

    let copy_fn: GlBindFramebufferFn = std::mem::transmute(copy_mem);
    REAL_GL_BIND_FRAMEBUFFER_COPY.get_or_init(|| copy_fn);

    let hook_addr = hook_self as u64;
    let patch: [u8; 12] = [
        0x48, 0xB8,
        (hook_addr      ) as u8, (hook_addr >>  8) as u8,
        (hook_addr >> 16) as u8, (hook_addr >> 24) as u8,
        (hook_addr >> 32) as u8, (hook_addr >> 40) as u8,
        (hook_addr >> 48) as u8, (hook_addr >> 56) as u8,
        0xFF, 0xE0,
    ];

    let page_sz = libc::sysconf(libc::_SC_PAGESIZE) as usize;
    let page = (from_scan as usize) & !(page_sz - 1);
    if libc::mprotect(page as *mut c_void, page_sz, libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC) != 0 {
        tracing::warn!("glBindFramebuffer hook: mprotect failed");
        return;
    }

    std::ptr::copy_nonoverlapping(patch.as_ptr(), from_scan, patch.len());
    tracing::info!(real_mesa = ?from_scan, hook = ?hook_addr, copy = ?copy_mem,
        "glBindFramebuffer hook: patched");
}

/// Centers game rendering within the EGL surface during mode resizes
pub unsafe extern "C" fn hooked_gl_viewport(x: c_int, y: c_int, w: c_int, h: c_int) {
    GL_VIEWPORT_SEEN.store(true, Ordering::Relaxed);
    let (mw, mh) = unpack(MODE_DIMS.load(Ordering::Acquire));
    let (ow, oh) = unpack(ORIGINAL_DIMS.load(Ordering::Acquire));
    let mode_w = mw as c_int;
    let mode_h = mh as c_int;
    let orig_w = ow as c_int;
    let orig_h = oh as c_int;

    let oversized = is_oversized(mw, mh, ow, oh);

    if oversized && mode_w > 0 && mode_h > 0 {
        let draw_fbo = get_gl_get_integerv().map(|f| {
            let mut out: c_int = -1;
            f(GL_DRAW_FRAMEBUFFER_BINDING, &mut out);
            out
        });
        tracing::debug!(x, y, w, h, mode_w, mode_h, orig_h,
            draw_fbo = ?draw_fbo, "hooked_gl_viewport: oversized mode diag");
    }

    let (fx, fy, fw, fh) =
        if mode_w > 0 && mode_h > 0 && !oversized
            && (orig_w != mode_w || orig_h != mode_h)
            && x == 0 && y == 0 && w == mode_w && h == mode_h
        {
            // only center on FBO 0 (the backbuffer). Sodium's FBOs match
            // mode_w x mode_h exactly, so centering those shoves the
            // viewport off the edge and you get a black frame.
            let draw_fbo = get_gl_get_integerv().map(|f| {
                let mut out: c_int = -1;
                f(GL_DRAW_FRAMEBUFFER_BINDING, &mut out);
                out
            });
            if draw_fbo == Some(0) {
                let cx = (orig_w - mode_w) / 2;
                let cy = (orig_h - mode_h) / 2;
                static CENTER_LOG: std::sync::Once = std::sync::Once::new();
                CENTER_LOG.call_once(|| {
                    tracing::info!(x, y, w, h, mode_w, mode_h, orig_w, orig_h, cx, cy,
                        "hooked_gl_viewport: centering FBO0 (first)");
                });
                (cx, cy, w, h)
            } else {
                tracing::trace!(x, y, w, h, draw_fbo = ?draw_fbo,
                    "hooked_gl_viewport: undersized custom FBO pass-through");
                (x, y, w, h)
            }
        } else if mode_w > 0 && mode_h > 0 && oversized
            && orig_h > 0
            && x == 0 && y == 0 && w == mode_w && h == mode_h
        {
            // oversized: only center on back buffer (FBO 0)
            let draw_fbo = get_gl_get_integerv().map(|f| unsafe {
                let mut out: c_int = -1;
                f(GL_DRAW_FRAMEBUFFER_BINDING, &mut out);
                out
            });
            if draw_fbo == Some(0) {
                let cx = if mode_w < orig_w { (orig_w - mode_w) / 2 } else { 0 };
                let cy = -((mode_h - orig_h) / 2);
                tracing::info!(x, y, w, h, cx, cy, orig_w, orig_h, mode_w, mode_h,
                    "hooked_gl_viewport: oversized FBO0 center");
                (cx, cy, w, h)
            } else {
                tracing::trace!(x, y, w, h, draw_fbo = ?draw_fbo,
                    "hooked_gl_viewport: oversized custom FBO pass-through");
                (x, y, w, h)
            }
        } else {
            tracing::trace!(
                x, y, w, h, mode_w, mode_h, orig_w, orig_h, oversized,
                real_vp_set = REAL_GL_VIEWPORT.get().is_some(),
                "hooked_gl_viewport: pass-through"
            );
            (x, y, w, h)
        };

    let real: &GlViewportFn = REAL_GL_VIEWPORT_COPY
        .get()
        .or_else(|| REAL_GL_VIEWPORT.get())
        .unwrap_or_else(|| {
            static FALLBACK: OnceLock<GlViewportFn> = OnceLock::new();
            FALLBACK.get_or_init(|| {
                tracing::warn!(x = fx, y = fy, w = fw, h = fh,
                    "hooked_gl_viewport: last-resort RTLD_NEXT resolve");
                let ptr = crate::dlsym_hook::resolve_real_symbol(b"glViewport\0");
                assert!(!ptr.is_null(), "tuxinjector: cannot resolve glViewport");
                unsafe { std::mem::transmute(ptr) }
            })
        });
    real(fx, fy, fw, fh);
}

pub unsafe extern "C" fn hooked_gl_scissor(x: c_int, y: c_int, w: c_int, h: c_int) {
    let (mw, mh) = unpack(MODE_DIMS.load(Ordering::Acquire));
    let (ow, oh) = unpack(ORIGINAL_DIMS.load(Ordering::Acquire));
    let mode_w = mw as c_int;
    let mode_h = mh as c_int;
    let orig_w = ow as c_int;
    let orig_h = oh as c_int;
    let oversized = is_oversized(mw, mh, ow, oh);

    let (sx, sy) =
        if mode_w > 0 && mode_h > 0 && !oversized
            && (orig_w != mode_w || orig_h != mode_h)
            && x == 0 && y == 0 && w == mode_w && h == mode_h
        {
            // same FBO 0 check as viewport -- Sodium's FBOs don't need offsetting
            let draw_fbo = get_gl_get_integerv().map(|f| {
                let mut out: c_int = -1;
                f(GL_DRAW_FRAMEBUFFER_BINDING, &mut out);
                out
            });
            if draw_fbo == Some(0) {
                let cx = (orig_w - mode_w) / 2;
                let cy = (orig_h - mode_h) / 2;
                (cx, cy)
            } else {
                (x, y)
            }
        } else if mode_w > 0 && mode_h > 0 && oversized
            && orig_h > 0
            && x == 0 && y == 0 && w == mode_w && h == mode_h
        {
            let draw_fbo = get_gl_get_integerv().map(|f| unsafe {
                let mut out: c_int = -1;
                f(GL_DRAW_FRAMEBUFFER_BINDING, &mut out);
                out
            });
            if draw_fbo == Some(0) {
                let cx = if mode_w < orig_w { (orig_w - mode_w) / 2 } else { 0 };
                let cy = -((mode_h - orig_h) / 2);
                (cx, cy)
            } else {
                (x, y)
            }
        } else {
            (x, y)
        };

    if let Some(real) = get_real_gl_scissor() {
        real(sx, sy, w, h);
    }
}

// --- PLT overrides ---

#[no_mangle]
pub unsafe extern "C" fn glViewport(x: c_int, y: c_int, w: c_int, h: c_int) {
    let first = REAL_GL_VIEWPORT.get().is_none();
    REAL_GL_VIEWPORT.get_or_init(|| {
        tracing::info!("glViewport PLT: resolving real via RTLD_NEXT");
        let ptr = crate::dlsym_hook::resolve_real_symbol(b"glViewport\0");
        assert!(!ptr.is_null(), "tuxinjector: can't resolve glViewport");
        std::mem::transmute(ptr)
    });
    if first {
        tracing::info!(x, y, w, h, "glViewport PLT: first call");
    }
    hooked_gl_viewport(x, y, w, h);
}

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn glScissor(x: c_int, y: c_int, w: c_int, h: c_int) {
    let first = REAL_GL_SCISSOR.get().is_none();
    REAL_GL_SCISSOR.get_or_init(|| {
        tracing::info!("glScissor PLT: resolving real via RTLD_NEXT");
        let ptr = crate::dlsym_hook::resolve_real_symbol(b"glScissor\0");
        assert!(!ptr.is_null(), "tuxinjector: can't resolve glScissor");
        unsafe { std::mem::transmute(ptr) }
    });
    if first { tracing::info!("glScissor PLT: first call"); }
    hooked_gl_scissor(x, y, w, h);
}

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn glBindFramebuffer(target: c_uint, framebuffer: c_uint) {
    let first = REAL_GL_BIND_FRAMEBUFFER.get().is_none();
    let real = REAL_GL_BIND_FRAMEBUFFER_COPY.get().copied()
        .or_else(|| REAL_GL_BIND_FRAMEBUFFER.get().copied())
        .unwrap_or_else(|| {
            tracing::info!("glBindFramebuffer PLT: resolving via RTLD_NEXT");
            let ptr = crate::dlsym_hook::resolve_real_symbol(b"glBindFramebuffer\0");
            assert!(!ptr.is_null(), "tuxinjector: can't resolve glBindFramebuffer");
            std::mem::transmute(ptr)
        });
    if first {
        tracing::info!(target, framebuffer, "glBindFramebuffer PLT: first call");
    }
    bind_fb_with(real, target, framebuffer);
}

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn glBindFramebufferEXT(target: c_uint, fb: c_uint) {
    let Some(real) = get_real_gl_bind_framebuffer_ext() else { return };
    bind_fb_with(real, target, fb);
}

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn glBindFramebufferARB(target: c_uint, fb: c_uint) {
    let Some(real) = get_real_gl_bind_framebuffer_arb() else { return };
    bind_fb_with(real, target, fb);
}

// GL_BACK -> GL_COLOR_ATTACHMENT0 when virtual FBO is bound
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn glDrawBuffer(mode: c_uint) {
    let Some(real) = get_real_gl_draw_buffer() else { return };
    let mut m = mode;
    if crate::virtual_fb::is_active() {
        if let Some(get_iv) = get_gl_get_integerv() {
            let mut fbo: c_int = -1;
            get_iv(GL_DRAW_FRAMEBUFFER_BINDING, &mut fbo);
            if fbo as u32 == crate::virtual_fb::virtual_fbo()
                && (mode == GL_BACK || mode == GL_BACK_LEFT) {
                m = GL_COLOR_ATTACHMENT0;
            }
        }
    }
    real(m);
}

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn glReadBuffer(mode: c_uint) {
    let Some(real) = get_real_gl_read_buffer() else { return };
    let mut m = mode;
    if crate::virtual_fb::is_active() {
        if let Some(get_iv) = get_gl_get_integerv() {
            let mut fbo: c_int = -1;
            get_iv(GL_READ_FRAMEBUFFER_BINDING, &mut fbo);
            if fbo as u32 == crate::virtual_fb::virtual_fbo()
                && (mode == GL_BACK || mode == GL_BACK_LEFT) {
                m = GL_COLOR_ATTACHMENT0;
            }
        }
    }
    real(m);
}

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn glDrawBuffers(n: c_int, bufs: *const c_uint) {
    let Some(real) = get_real_gl_draw_buffers() else { return };
    if n <= 0 || bufs.is_null() {
        real(n, bufs);
        return;
    }
    let mut mapped = [0u32; 1];
    if n == 1 && crate::virtual_fb::is_active() {
        if let Some(get_iv) = get_gl_get_integerv() {
            let mut fbo: c_int = -1;
            get_iv(GL_DRAW_FRAMEBUFFER_BINDING, &mut fbo);
            if fbo as u32 == crate::virtual_fb::virtual_fbo() {
                let val = *bufs;
                if val == GL_BACK || val == GL_BACK_LEFT {
                    mapped[0] = GL_COLOR_ATTACHMENT0;
                    real(1, mapped.as_ptr());
                    return;
                }
            }
        }
    }
    real(n, bufs);
}

// Centers the final blit into the physical back buffer during mode resizes
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn glBlitFramebuffer(
    sx0: c_int, sy0: c_int, sx1: c_int, sy1: c_int,
    dx0: c_int, dy0: c_int, dx1: c_int, dy1: c_int,
    mask: c_uint, filter: c_uint,
) {
    let Some(real) = get_real_gl_blit_framebuffer() else { return };

    let (mw, mh) = unpack(MODE_DIMS.load(Ordering::Acquire));
    let (ow, oh) = unpack(ORIGINAL_DIMS.load(Ordering::Acquire));
    let mode_w = mw as i32; let mode_h = mh as i32;
    let orig_w = ow as i32; let orig_h = oh as i32;

    if mode_w > 0 && mode_h > 0 && orig_w > 0 && orig_h > 0 {
        let draw_fbo = get_gl_get_integerv().map(|f| {
            let mut out: c_int = -1; f(GL_DRAW_FRAMEBUFFER_BINDING, &mut out); out
        });
        let read_fbo = get_gl_get_integerv().map(|f| {
            let mut out: c_int = -1; f(GL_READ_FRAMEBUFFER_BINDING, &mut out); out
        });

        let src_w = (sx1 - sx0).abs();
        let src_h = (sy1 - sy0).abs();
        let dst_w = (dx1 - dx0).abs();
        let dst_h = (dy1 - dy0).abs();

        let dst_is_phys = dst_w == orig_w && dst_h == orig_h;
        let src_is_mode = (src_w == mode_w || src_w == orig_w) && (src_h == mode_h || src_h == orig_h);

        if draw_fbo == Some(0) && read_fbo != Some(0) && dst_is_phys && src_is_mode {
            // center src/dst rectangles
            let (s_x, s_w, d_x, d_w) = if mode_w >= orig_w {
                ((mode_w - orig_w) / 2, orig_w, 0, orig_w)
            } else {
                (0, mode_w, (orig_w - mode_w) / 2, mode_w)
            };
            let (s_y, s_h, d_y, d_h) = if mode_h >= orig_h {
                ((mode_h - orig_h) / 2, orig_h, 0, orig_h)
            } else {
                (0, mode_h, (orig_h - mode_h) / 2, mode_h)
            };

            let nsx0 = if sx1 >= sx0 { s_x } else { s_x + s_w };
            let nsx1 = if sx1 >= sx0 { s_x + s_w } else { s_x };
            let nsy0 = if sy1 >= sy0 { s_y } else { s_y + s_h };
            let nsy1 = if sy1 >= sy0 { s_y + s_h } else { s_y };
            let ndx0 = if dx1 >= dx0 { d_x } else { d_x + d_w };
            let ndx1 = if dx1 >= dx0 { d_x + d_w } else { d_x };
            let ndy0 = if dy1 >= dy0 { d_y } else { d_y + d_h };
            let ndy1 = if dy1 >= dy0 { d_y + d_h } else { d_y };

            tracing::trace!(
                src = ?(nsx0, nsy0, nsx1, nsy1), dst = ?(ndx0, ndy0, ndx1, ndy1),
                mode_w, mode_h, orig_w, orig_h,
                "glBlitFramebuffer: centered final blit"
            );

            real(nsx0, nsy0, nsx1, nsy1, ndx0, ndy0, ndx1, ndy1, mask, filter);
            return;
        }
    }

    real(sx0, sy0, sx1, sy1, dx0, dy0, dx1, dy1, mask, filter);
}

// --- glXGetProcAddressARB / eglGetProcAddress interception ---
// These intercept GL function lookups so the game gets our hooks instead
// of the real Mesa functions. We store the real ptrs for our own use.

// helper macro to reduce the repetitive if-else chain
macro_rules! intercept_gl_proc {
    ($bytes:expr, $name:literal, $store_fn:ident, $hook_fn:ident, $real_resolver:expr) => {
        if $bytes == $name {
            if let Some(f) = $real_resolver {
                let real_ptr = f;
                tracing::info!(real_ptr = ?real_ptr,
                    concat!("glXGetProcAddressARB(", stringify!($hook_fn), "): storing real, returning hook"));
                $store_fn(real_ptr);
            }
            return $hook_fn as *mut c_void;
        }
    };
}

#[no_mangle]
pub unsafe extern "C" fn glXGetProcAddressARB(name: *const c_char) -> *mut c_void {
    static REAL_GLX_GPA: OnceLock<GlxGetProcAddressFn> = OnceLock::new();
    let real = REAL_GLX_GPA.get_or_init(|| {
        let ptr = libc::dlsym(libc::RTLD_NEXT, b"glXGetProcAddressARB\0".as_ptr() as *const libc::c_char);
        assert!(!ptr.is_null(), "tuxinjector: can't resolve glXGetProcAddressARB");
        tracing::info!("glXGetProcAddressARB PLT: resolved real");
        std::mem::transmute(ptr)
    });

    if !name.is_null() {
        let bytes = std::ffi::CStr::from_ptr(name).to_bytes();
        let gpa = crate::gl_resolve::get_proc_address_fn();

        intercept_gl_proc!(bytes, b"glViewport", store_real_gl_viewport, glViewport, gpa.map(|f| f(name)));
        intercept_gl_proc!(bytes, b"glScissor", store_real_gl_scissor, glScissor, gpa.map(|f| f(name)));
        intercept_gl_proc!(bytes, b"glBindFramebuffer", store_real_gl_bind_framebuffer, glBindFramebuffer, gpa.map(|f| f(name)));
        intercept_gl_proc!(bytes, b"glBindFramebufferEXT", store_real_gl_bind_framebuffer_ext, glBindFramebufferEXT, gpa.map(|f| f(name)));
        intercept_gl_proc!(bytes, b"glBindFramebufferARB", store_real_gl_bind_framebuffer_arb, glBindFramebufferARB, gpa.map(|f| f(name)));
        intercept_gl_proc!(bytes, b"glDrawBuffer", store_real_gl_draw_buffer, glDrawBuffer, gpa.map(|f| f(name)));
        intercept_gl_proc!(bytes, b"glReadBuffer", store_real_gl_read_buffer, glReadBuffer, gpa.map(|f| f(name)));
        intercept_gl_proc!(bytes, b"glDrawBuffers", store_real_gl_draw_buffers, glDrawBuffers, gpa.map(|f| f(name)));
        intercept_gl_proc!(bytes, b"glBlitFramebuffer", store_real_gl_blit_framebuffer, glBlitFramebuffer, gpa.map(|f| f(name)));
    }

    real(name)
}

type EglGetProcAddressFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;

#[no_mangle]
pub unsafe extern "C" fn eglGetProcAddress(name: *const c_char) -> *mut c_void {
    static REAL_EGL_GPA: OnceLock<EglGetProcAddressFn> = OnceLock::new();
    let real = REAL_EGL_GPA.get_or_init(|| {
        let ptr = libc::dlsym(libc::RTLD_NEXT, b"eglGetProcAddress\0".as_ptr() as *const libc::c_char);
        assert!(!ptr.is_null(), "tuxinjector: can't resolve eglGetProcAddress");
        std::mem::transmute(ptr)
    });

    if !name.is_null() {
        let bytes = std::ffi::CStr::from_ptr(name).to_bytes();
        // for EGL, the "real" ptr comes from calling eglGetProcAddress itself
        let resolve = |n: *const c_char| -> *mut c_void { real(n) };

        macro_rules! egl_intercept {
            ($sym:literal, $store:ident, $hook:ident) => {
                if bytes == $sym {
                    let rp = resolve(name);
                    if !rp.is_null() { $store(rp); }
                    tracing::info!(real_ptr = ?rp,
                        concat!("eglGetProcAddress(", stringify!($hook), "): returning hook"));
                    return $hook as *mut c_void;
                }
            };
        }

        egl_intercept!(b"glViewport", store_real_gl_viewport, glViewport);
        egl_intercept!(b"glScissor", store_real_gl_scissor, glScissor);
        egl_intercept!(b"glBindFramebuffer", store_real_gl_bind_framebuffer, glBindFramebuffer);
        egl_intercept!(b"glBindFramebufferEXT", store_real_gl_bind_framebuffer_ext, glBindFramebufferEXT);
        egl_intercept!(b"glBindFramebufferARB", store_real_gl_bind_framebuffer_arb, glBindFramebufferARB);
        egl_intercept!(b"glDrawBuffer", store_real_gl_draw_buffer, glDrawBuffer);
        egl_intercept!(b"glReadBuffer", store_real_gl_read_buffer, glReadBuffer);
        egl_intercept!(b"glDrawBuffers", store_real_gl_draw_buffers, glDrawBuffers);
        egl_intercept!(b"glBlitFramebuffer", store_real_gl_blit_framebuffer, glBlitFramebuffer);
    }

    real(name)
}

// --- glfwSetFramebufferSizeCallback / glfwGetFramebufferSize PLT hooks ---

#[no_mangle]
pub unsafe extern "C" fn glfwSetFramebufferSizeCallback(
    window: *mut c_void, callback: Option<GlfwFbSizeCb>,
) -> Option<GlfwFbSizeCb> {
    static RESOLVED: OnceLock<()> = OnceLock::new();
    RESOLVED.get_or_init(|| {
        let ptr = libc::dlsym(libc::RTLD_NEXT,
            b"glfwSetFramebufferSizeCallback\0".as_ptr() as *const c_char);
        if ptr.is_null() {
            tracing::error!("glfwSetFramebufferSizeCallback: real not found");
        } else {
            tracing::info!("glfwSetFramebufferSizeCallback: PLT resolved");
        }
    });
    hooked_glfw_set_framebuffer_size_callback(window, callback)
}

#[no_mangle]
pub unsafe extern "C" fn glfwGetFramebufferSize(
    window: *mut c_void, width: *mut c_int, height: *mut c_int,
) {
    static RESOLVED: OnceLock<()> = OnceLock::new();
    RESOLVED.get_or_init(|| {
        let ptr = libc::dlsym(libc::RTLD_NEXT,
            b"glfwGetFramebufferSize\0".as_ptr() as *const c_char);
        if ptr.is_null() {
            tracing::error!("glfwGetFramebufferSize: real not found");
        } else {
            tracing::info!("glfwGetFramebufferSize: PLT resolved");
        }
    });
    hooked_glfw_get_framebuffer_size(window, width, height)
}

// --- Public API ---

pub fn get_mode_size() -> (u32, u32) {
    unpack(MODE_DIMS.load(Ordering::Acquire))
}

pub fn is_oversized(mode_w: u32, mode_h: u32, orig_w: u32, orig_h: u32) -> bool {
    let _ = (mode_w, orig_w); // height-only check for now
    orig_h > 0 && mode_h > orig_h.saturating_mul(4)
}

pub fn get_original_size() -> (u32, u32) {
    unpack(ORIGINAL_DIMS.load(Ordering::Acquire))
}

#[allow(dead_code)]
pub fn store_original_size_if_unset(w: u32, h: u32) {
    let (ow, _) = unpack(ORIGINAL_DIMS.load(Ordering::Acquire));
    if ow == 0 {
        ORIGINAL_DIMS.store(pack(w, h), Ordering::Release);
        tracing::info!(w, h, "original surface size captured from GL viewport (fallback)");
    }
}

pub fn force_store_original_size(w: u32, h: u32) {
    let (pw, ph) = unpack(ORIGINAL_DIMS.load(Ordering::Acquire));
    if w != pw || h != ph {
        ORIGINAL_DIMS.store(pack(w, h), Ordering::Release);
        tracing::info!(w, h, pw, ph, "original surface size updated (window resize/fullscreen)");
    }
}

/// Trigger a mode resize: store dims, resize surface, fire GLFW callback
pub unsafe fn fire_framebuffer_resize(mode_w: u32, mode_h: u32) {
    let (orig_w, orig_h) = unpack(ORIGINAL_DIMS.load(Ordering::Acquire));
    let has_vp = REAL_GL_VIEWPORT.get().is_some();

    tracing::info!(mode_w, mode_h, orig_w, orig_h, real_gl_viewport_set = has_vp,
        "fire_framebuffer_resize");

    // 0 means fullscreen -- hooks revert to pass-through
    if orig_w > 0 && mode_w == orig_w && mode_h == orig_h {
        MODE_DIMS.store(0, Ordering::Release);
    } else {
        MODE_DIMS.store(pack(mode_w, mode_h), Ordering::Release);
    }

    // resize GL surface before firing callback
    if orig_w > 0 && orig_h > 0 {
        let egl_win = GAME_EGL_WINDOW.load(Ordering::Acquire);
        if !egl_win.is_null() {
            if let Some(real_resize) = get_real_wl_egl_window_resize() {
                tracing::info!(mode_w, mode_h, orig_w, orig_h,
                    "fire_framebuffer_resize: clamping EGL to physical size");
                real_resize(egl_win, orig_w as c_int, orig_h as c_int, 0, 0);
            }
        } else {
            #[cfg(target_os = "linux")]
            {
                // GLX path: resize X11 window for non-oversized modes
                if !is_oversized(mode_w, mode_h, orig_w, orig_h) {
                    try_x11_window_resize(mode_w, mode_h, orig_w, orig_h);
                }
            }
            #[cfg(target_os = "macos")]
            {
                // no X11 on macOS, goes through GLFW (or just no-ops)
                try_glfw_window_resize(mode_w, mode_h, orig_w, orig_h);
            }
        }
    }

    let oversized = is_oversized(mode_w, mode_h, orig_w, orig_h);
    if let Some(gl) = crate::state::get().gl.get() {
        // inline-patch Mesa GL fns so LWJGL3+RTLD_DEEPBIND hits our hooks (Linux x86_64 only)
        #[cfg(target_os = "linux")]
        {
            if !is_gl_viewport_hooked() {
                install_glviewport_inline_hook();
            }
            if REAL_GL_BIND_FRAMEBUFFER_COPY.get().is_none() {
                install_glbindframebuffer_inline_hook();
            }
        }
        if oversized {
            crate::virtual_fb::ensure_offscreen(gl, mode_w, mode_h);
        } else if crate::virtual_fb::is_active() {
            crate::virtual_fb::destroy_offscreen(gl);
        }
    } else if !oversized {
        crate::virtual_fb::set_active(false);
    }

    // fire GLFW fb-size callback so Minecraft updates its internal state
    let glfw_win = GAME_WINDOW.load(Ordering::Acquire);
    let cb_ptr = GAME_FB_SIZE_CB.load(Ordering::Acquire);
    if !glfw_win.is_null() && !cb_ptr.is_null() {
        tracing::info!(mode_w, mode_h, "firing GLFW framebuffer-size callback");
        let cb: GlfwFbSizeCb = std::mem::transmute(cb_ptr);
        cb(glfw_win, mode_w as c_int, mode_h as c_int);
    } else {
        tracing::warn!(glfw_win = ?glfw_win, cb_ptr = ?cb_ptr,
            "fire_framebuffer_resize: GLFW window/callback not captured yet");
    }
}

// --- Hooked glfwSetFramebufferSizeCallback ---

pub unsafe extern "C" fn hooked_glfw_set_framebuffer_size_callback(
    window: *mut c_void, callback: Option<GlfwFbSizeCb>,
) -> Option<GlfwFbSizeCb> {
    GAME_WINDOW.store(window, Ordering::Release);
    if let Some(cb) = callback {
        GAME_FB_SIZE_CB.store(cb as *mut c_void, Ordering::Release);
        tracing::debug!("captured glfwSetFramebufferSizeCallback");
    }
    let real = REAL_SET_FB_SIZE_CB.load(Ordering::Acquire);
    if real.is_null() { return None; }
    let real_fn: GlfwSetFbSizeCbFn = std::mem::transmute(real);
    // macOS: intercept OS resize events (e.g. fullscreen restore) so they
    // don't clobber our mode. Register our wrapper instead of the game's.
    #[cfg(target_os = "macos")]
    { return real_fn(window, Some(glfw_fb_size_wrapper)); }
    #[cfg(target_os = "linux")]
    real_fn(window, callback)
}

// If a mode is active, absorb the resize silently. No mode = pass through.
#[cfg(target_os = "macos")]
unsafe extern "C" fn glfw_fb_size_wrapper(window: *mut c_void, w: c_int, h: c_int) {
    let (mw, mh) = unpack(MODE_DIMS.load(Ordering::Acquire));
    let wu = w as u32;
    let hu = h as u32;

    if mw > 0 && mh > 0 {
        // mode active -- update orig dims but don't let MC see the real size
        let (ow, oh) = unpack(ORIGINAL_DIMS.load(Ordering::Acquire));
        if wu != ow || hu != oh {
            ORIGINAL_DIMS.store(pack(wu, hu), Ordering::Release);
            tracing::info!(w, h, mw, mh,
                "glfw_fb_size_wrapper: absorbed resize, updated orig, keeping mode");
        }
        return;
    }

    // no mode -- forward to game and update our orig dims
    force_store_original_size(wu, hu);
    let cb_ptr = GAME_FB_SIZE_CB.load(Ordering::Acquire);
    if !cb_ptr.is_null() {
        let cb: GlfwFbSizeCb = std::mem::transmute(cb_ptr);
        cb(window, w, h);
    }
}

// --- Hooked glfwGetFramebufferSize ---

pub unsafe extern "C" fn hooked_glfw_get_framebuffer_size(
    window: *mut c_void, width: *mut c_int, height: *mut c_int,
) {
    let (mw, mh) = unpack(MODE_DIMS.load(Ordering::Acquire));
    if mw > 0 && mh > 0 {
        if !width.is_null()  { *width  = mw as c_int; }
        if !height.is_null() { *height = mh as c_int; }
        return;
    }
    let real = REAL_GET_FB_SIZE.load(Ordering::Acquire);
    if real.is_null() { return; }
    let real_fn: GlfwGetFbSizeFn = std::mem::transmute(real);
    real_fn(window, width, height);
}
