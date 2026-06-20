// Wayland cursor-centering hook (Linux/native-Wayland only).
//
// Native-Wayland Minecraft has a long-standing cursor-displacement bug: when a
// menu opens (cursor mode -> NORMAL) the compositor leaves the pointer wherever
// it physically was instead of re-centering it the way XWarpPointer does on
// Xwayland. Open menu, move mouse, reopen menu -> cursor keeps its old spot.
//
// Root cause, verified against the *exact* bundled GLFW source (stock glfw/glfw
// @3eaf1255, matched via the `libglfw.so.git` marker in LWJGL's natives jar):
//   * `_glfwSetCursorPosWayland` is a pure no-op (just emits FEATURE_UNAVAILABLE).
//   * `lockPointer` calls `lock_pointer` with region=NULL and NEVER calls
//     `zwp_locked_pointer_v1::set_cursor_position_hint`.
//   * `unlockPointer` only destroys the lock -- no hint either.
// With no hint, the compositor has nowhere to place the cursor on unlock.
//
// waywall fixes this as the *parent compositor* by forcing the host locked
// pointer's hint to center. We do the in-process equivalent: interpose
// libwayland-client's `wl_proxy_marshal_array_flags` (the non-variadic delegate
// that `wl_proxy_marshal_flags` packs its varargs into), and when the game
// marshals `lock_pointer`, inject a `set_cursor_position_hint(center)` on the
// freshly created locked pointer. MC commits its surface every frame, latching
// the hint, so every subsequent unlock re-centers -- exactly like Xwayland.
//
// Interposition is sound here: in libwayland-client.so the call
// `wl_proxy_marshal_flags -> wl_proxy_marshal_array_flags` goes through the PLT
// and the lib carries no -Bsymbolic flag, and tux strips RTLD_DEEPBIND from
// dlopen, so our preloaded symbol wins.

use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicPtr, Ordering};

use tuxinjector_input::callbacks;

// zwp_pointer_constraints_v1 request opcodes
const PC_LOCK_POINTER: u32 = 1;
// zwp_locked_pointer_v1 request opcodes
const LP_SET_CURSOR_POSITION_HINT: u32 = 1;
const LP_DESTROY: u32 = 0;

type MarshalArrayFlagsFn = unsafe extern "C" fn(
    *mut c_void,   // proxy
    u32,           // opcode
    *const c_void, // interface (non-null only for constructor requests)
    u32,           // version
    u32,           // flags
    *mut c_void,   // union wl_argument *args
) -> *mut c_void;

type GetClassFn = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type GetVersionFn = unsafe extern "C" fn(*mut c_void) -> u32;

// Resolved real symbols, cached as raw pointers. We only ever transmute a
// confirmed-non-null pointer into a fn type -- transmuting null into a
// (non-nullable) fn pointer is UB and lets the optimizer delete null guards.
static REAL_MARSHAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static REAL_GET_CLASS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static REAL_GET_VERSION: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
// Cached handle to the already-mapped libwayland-client (see wl_lib).
static WL_HANDLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// The most recently created locked pointer, so we can cheaply (pointer-compare)
// notice if the game ever sets its own hint, and clear on destroy.
static LAST_LOCKED: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// Handle to the already-loaded libwayland-client. RTLD_NOLOAD returns a handle
// to the existing mapping (never loads a fresh copy) and dlsym on it searches
// that library directly -- reliable regardless of whether GLFW dlopened it
// RTLD_LOCAL. RTLD_NEXT would only walk the global scope and miss a symbol that
// lives in a private (local) group, returning null. Returns null only if the
// library isn't mapped yet, in which case the caller retries on a later call.
unsafe fn wl_lib() -> *mut c_void {
    let h = WL_HANDLE.load(Ordering::Acquire);
    if !h.is_null() {
        return h;
    }
    let opened = libc::dlopen(
        b"libwayland-client.so.0\0".as_ptr() as *const c_char,
        libc::RTLD_NOLOAD | libc::RTLD_NOW,
    );
    if !opened.is_null() {
        // Keep the handle for the process lifetime (one extra refcount on a
        // library the game keeps mapped anyway) so our fn pointers stay valid.
        WL_HANDLE.store(opened, Ordering::Release);
    }
    opened
}

unsafe fn wl_dlsym(name: &[u8]) -> *mut c_void {
    let h = wl_lib();
    if h.is_null() {
        return std::ptr::null_mut();
    }
    libc::dlsym(h, name.as_ptr() as *const c_char)
}

// Resolve+cache a real symbol from libwayland-client. Never caches null, so a
// call that runs before the library is mapped retries cleanly later.
unsafe fn resolve_cached(cache: &AtomicPtr<c_void>, name: &[u8]) -> *mut c_void {
    let p = cache.load(Ordering::Acquire);
    if !p.is_null() {
        return p;
    }
    let resolved = wl_dlsym(name);
    if !resolved.is_null() {
        cache.store(resolved, Ordering::Release);
    }
    resolved
}

unsafe fn real_marshal() -> Option<MarshalArrayFlagsFn> {
    let p = resolve_cached(&REAL_MARSHAL, b"wl_proxy_marshal_array_flags\0");
    if p.is_null() {
        None
    } else {
        Some(std::mem::transmute::<*mut c_void, MarshalArrayFlagsFn>(p))
    }
}

unsafe fn proxy_class_is(proxy: *mut c_void, want: &[u8]) -> bool {
    if proxy.is_null() {
        return false;
    }
    let p = resolve_cached(&REAL_GET_CLASS, b"wl_proxy_get_class\0");
    if p.is_null() {
        return false;
    }
    let f: GetClassFn = std::mem::transmute(p);
    let cls = f(proxy);
    !cls.is_null() && CStr::from_ptr(cls).to_bytes() == want
}

unsafe fn proxy_version(proxy: *mut c_void) -> u32 {
    let p = resolve_cached(&REAL_GET_VERSION, b"wl_proxy_get_version\0");
    if p.is_null() {
        return 1;
    }
    let f: GetVersionFn = std::mem::transmute(p);
    f(proxy)
}

#[inline]
fn wl_fixed_from_f64(d: f64) -> i32 {
    (d * 256.0).round() as i32
}

// Inject `set_cursor_position_hint(center)` on a freshly locked pointer.
unsafe fn inject_center_hint(locked_ptr: *mut c_void) {
    let Some((w, h)) = callbacks::window_logical_size() else {
        return;
    };
    let fx = wl_fixed_from_f64(w as f64 / 2.0);
    let fy = wl_fixed_from_f64(h as f64 / 2.0);

    // union wl_argument is pointer-sized (8 bytes); the fixed value lives in the
    // low 32 bits on little-endian x86_64, which is what libwayland reads as .f.
    let mut args: [u64; 2] = [(fx as u32) as u64, (fy as u32) as u64];

    let Some(real) = real_marshal() else {
        return;
    };
    real(
        locked_ptr,
        LP_SET_CURSOR_POSITION_HINT,
        std::ptr::null(),
        proxy_version(locked_ptr),
        0,
        args.as_mut_ptr() as *mut c_void,
    );
    tracing::info!(cx = w / 2, cy = h / 2, "[WL] injected set_cursor_position_hint (center)");
}

#[no_mangle]
pub unsafe extern "C" fn wl_proxy_marshal_array_flags(
    proxy: *mut c_void,
    opcode: u32,
    interface: *const c_void,
    version: u32,
    flags: u32,
    args: *mut c_void,
) -> *mut c_void {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| eprintln!("[tuxinjector] wl_proxy_marshal_array_flags hooked"));

    // If we can't resolve the real symbol we've already interposed and cannot
    // transparently forward -- but this only happens if libwayland-client isn't
    // mapped, which is impossible if the game is calling us through it. Return
    // null rather than invoke an invalid (possibly-null) pointer.
    let Some(real) = real_marshal() else {
        return std::ptr::null_mut();
    };

    // Cheap pointer-compare confirmation: did the game set its own hint, or
    // destroy the locked pointer we're tracking?
    if !proxy.is_null() && proxy == LAST_LOCKED.load(Ordering::Acquire) {
        if opcode == LP_SET_CURSOR_POSITION_HINT {
            tracing::info!("[WL] game set its OWN cursor-position hint (unexpected)");
        } else if opcode == LP_DESTROY {
            LAST_LOCKED.store(std::ptr::null_mut(), Ordering::Release);
        }
    }

    let ret = real(proxy, opcode, interface, version, flags, args);

    // lock_pointer is opcode 1 and constructs a zwp_locked_pointer_v1 (non-null
    // interface). Gate cheaply on those before paying for a class lookup.
    if opcode == PC_LOCK_POINTER
        && !interface.is_null()
        && !ret.is_null()
        && proxy_class_is(proxy, b"zwp_pointer_constraints_v1")
    {
        LAST_LOCKED.store(ret, Ordering::Release);
        inject_center_hint(ret);
    }

    ret
}
