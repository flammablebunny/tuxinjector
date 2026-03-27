// macOS app capture via CoreGraphics (+ HW capture / screencapture fallback).
// CG grabs from the compositor so windows can be behind MC.
// Needs Screen Recording permission or CG just returns NULL.

use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_void};
use std::sync::OnceLock;
use std::sync::Condvar;
use std::time::{Duration, Instant};

const CAPTURE_INTERVAL: Duration = Duration::from_millis(100);

// TODO: Probably change the following timings and how often it refreshes.
// It works on my M1 Macbook Air with no lag, so it should be fine, but idk.

// When >0, screencapture refreshes every frame. Set to 100 when input is forwarded to NBB.
static SCREENCAP_BURST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

// key events waiting to be forwarded to companion apps over stdin
// tuple: (scancode, x11_modifier_mask, jnh_keycode)
static APP_KEY_QUEUE: std::sync::Mutex<Vec<(u8, u16, i32)>> = std::sync::Mutex::new(Vec::new());

// queue a key event for companion apps (forwarded next frame)
pub fn push_app_key(key: i32, scancode: i32, mods: i32, pressed: bool) {
    if !pressed { return; }
    if scancode <= 0 || scancode > 255 { return; }
    let x11_mods = glfw_mods_to_x11(mods);
    let jnh_code = glfw_key_to_jnh(key);
    if let Ok(mut q) = APP_KEY_QUEUE.lock() {
        q.push((scancode as u8, x11_mods, jnh_code));
    }
}

// same encoding as the Linux side - NBB expects X11-style modifier mask
fn glfw_mods_to_x11(mods: i32) -> u16 {
    let mut s = 0u16;
    if mods & 0x1 != 0 { s |= 1; }   // Shift
    if mods & 0x2 != 0 { s |= 4; }   // Control
    if mods & 0x4 != 0 { s |= 8; }   // Alt = Mod1
    if mods & 0x8 != 0 { s |= 64; }  // Super = Mod4
    s
}

// **THIS FOLLOWING FUNCTION WAS ORGANISED BY AN LLM**
// GLFW key -> JNativeHook virtual keycode (AT scancode set 1)
fn glfw_key_to_jnh(key: i32) -> i32 {
    match key {
        256 => 0x0001,  // Escape
        290 => 0x003B, 291 => 0x003C, 292 => 0x003D, 293 => 0x003E, // F1-F4
        294 => 0x003F, 295 => 0x0040, 296 => 0x0041, 297 => 0x0042, // F5-F8
        298 => 0x0043, 299 => 0x0044, 300 => 0x0057, 301 => 0x0058, // F9-F12

        96  => 0x0029, // `
        49  => 0x0002, 50 => 0x0003, 51 => 0x0004, 52 => 0x0005, // 1-4
        53  => 0x0006, 54 => 0x0007, 55 => 0x0008, 56 => 0x0009, // 5-8
        57  => 0x000A, 48 => 0x000B, // 9, 0
        45  => 0x000C, // -
        61  => 0x000D, // =
        259 => 0x000E, // Backspace

        258 => 0x000F, // Tab
        81  => 0x0010, 87 => 0x0011, 69 => 0x0012, 82 => 0x0013, // Q W E R
        84  => 0x0014, 89 => 0x0015, 85 => 0x0016, 73 => 0x0017, // T Y U I
        79  => 0x0018, 80 => 0x0019, // O P
        91  => 0x001A, 93 => 0x001B, 92 => 0x002B, // [ ] backslash

        280 => 0x003A, // Caps Lock
        65  => 0x001E, 83 => 0x001F, 68 => 0x0020, 70 => 0x0021, // A S D F
        71  => 0x0022, 72 => 0x0023, 74 => 0x0024, 75 => 0x0025, // G H J K
        76  => 0x0026, 59 => 0x0027, 39 => 0x0028, // L ; '
        257 => 0x001C, // Enter

        90  => 0x002C, 88 => 0x002D, 67 => 0x002E, 86 => 0x002F, // Z X C V
        66  => 0x0030, 78 => 0x0031, 77 => 0x0032, // B N M
        44  => 0x0033, 46 => 0x0034, 47 => 0x0035, // , . /

        340 => 0x002A, 344 => 0x0036, // L/R Shift
        341 => 0x001D, 345 => 0x0E1D, // L/R Control
        342 => 0x0038, 346 => 0x0E38, // L/R Alt
        32  => 0x0039, // Space

        // Numpad
        282 => 0x0045,   // Num Lock
        331 => 0x0E35,   // KP /
        332 => 0x0037,   // KP *
        333 => 0x004A,   // KP -
        334 => 0x004E,   // KP +
        335 => 0x0E1C,   // KP Enter
        330 => 0x0053,   // KP .
        320 => 0x0052, 321 => 0x004F, 322 => 0x0050, 323 => 0x0051, // KP 0-3
        324 => 0x004B, 325 => 0x004C, 326 => 0x004D, // KP 4-6
        327 => 0x0047, 328 => 0x0048, 329 => 0x0049, // KP 7-9

        // Navigation
        265 => 0xE048, 264 => 0xE050, 263 => 0xE04B, 262 => 0xE04D, // Up Down Left Right
        266 => 0xE049, 267 => 0xE051, // Page Up, Page Down
        268 => 0xE047, 269 => 0xE04F, // Home, End
        260 => 0xE052, 261 => 0xE053, // Insert, Delete

        _ => -1,
    }
}

// --- CoreGraphics / CoreFoundation FFI (all resolved via dlsym) ---

type CFTypeRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFNumberRef = *const c_void;
type CFDataRef = *const c_void;
type CGImageRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint { x: f64, y: f64 }

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize { width: f64, height: f64 }

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect { origin: CGPoint, size: CGSize }

impl CGRect {
    // CGRectNull -- CG interprets this as "use the window's own bounds"
    const NULL: Self = Self {
        origin: CGPoint { x: f64::INFINITY, y: f64::INFINITY },
        size: CGSize { width: 0.0, height: 0.0 },
    };
}

// CGWindowListOption
const K_CG_WINDOW_LIST_OPTION_ALL: u32 = 0;
const K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY: u32 = 1 << 0;
const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;

// CGWindowImageOption
const K_CG_WINDOW_IMAGE_DEFAULT: u32 = 0;
const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
const K_CG_WINDOW_IMAGE_SHOULD_BE_OPAQUE: u32 = 1 << 1;

const K_CF_NUMBER_SINT32_TYPE: u64 = 3; // kCFNumberSInt32Type

type CGWindowListCopyWindowInfoFn = unsafe extern "C" fn(u32, u32) -> CFArrayRef;
type CGWindowListCreateImageFn = unsafe extern "C" fn(CGRect, u32, u32, u32) -> CGImageRef;
type CGWindowListCreateImageFromArrayFn = unsafe extern "C" fn(CGRect, CFArrayRef, u32) -> CGImageRef;
type CGImageGetWidthFn = unsafe extern "C" fn(CGImageRef) -> usize;
type CGImageGetHeightFn = unsafe extern "C" fn(CGImageRef) -> usize;
type CGImageGetBytesPerRowFn = unsafe extern "C" fn(CGImageRef) -> usize;
type CGImageGetDataProviderFn = unsafe extern "C" fn(CGImageRef) -> *const c_void;
type CGDataProviderCopyDataFn = unsafe extern "C" fn(*const c_void) -> CFDataRef;
type CFArrayCreateFn = unsafe extern "C" fn(*const c_void, *const *const c_void, isize, *const c_void) -> CFArrayRef;
type CFArrayGetCountFn = unsafe extern "C" fn(CFArrayRef) -> isize;
type CFArrayGetValueAtIndexFn = unsafe extern "C" fn(CFArrayRef, isize) -> *const c_void;
type CFDictionaryGetValueFn = unsafe extern "C" fn(CFDictionaryRef, *const c_void) -> *const c_void;
type CFNumberCreateFn = unsafe extern "C" fn(*const c_void, u64, *const c_void) -> CFNumberRef;
type CFNumberGetValueFn = unsafe extern "C" fn(CFNumberRef, u64, *mut c_void) -> bool;
type CFDataGetBytePtrFn = unsafe extern "C" fn(CFDataRef) -> *const u8;
type CFDataGetLengthFn = unsafe extern "C" fn(CFDataRef) -> isize;
type CFReleaseFn = unsafe extern "C" fn(CFTypeRef);

type CGRectMakeWithDictReprFn = unsafe extern "C" fn(dict: CFDictionaryRef, rect: *mut CGRect) -> bool;
type CGPreflightScreenCaptureAccessFn = unsafe extern "C" fn() -> bool;
type CGRequestScreenCaptureAccessFn = unsafe extern "C" fn() -> bool;

// Private CGS/SLS (CoreGraphics Server / SkyLight) APIs
type CGSConnectionID = u32;
type CGSDefaultConnectionFn = unsafe extern "C" fn() -> CGSConnectionID;
type CGSMoveWindowFn = unsafe extern "C" fn(CGSConnectionID, u32, *const CGPoint) -> i32;

// SLS/CGS hardware window capture - returns CFArray of CGImageRef (not IOSurface).
// Same path SCKit uses internally, works for Metal-backed windows.
type HWCaptureWindowListFn = unsafe extern "C" fn(CGSConnectionID, *const u32, u32, u32) -> CFArrayRef;

// For loading PNG files produced by `screencapture -l` subprocess
type CGDataProviderCreateWithFilenameFn = unsafe extern "C" fn(filename: *const c_char) -> *const c_void;
type CGImageCreateWithPNGDataProviderFn = unsafe extern "C" fn(
    source: *const c_void, decode: *const f64, should_interpolate: bool, intent: i32,
) -> CGImageRef;

// CG/CF function pointers, resolved once via dlsym
struct CgFns {
    window_list_copy_info: CGWindowListCopyWindowInfoFn,
    window_list_create_image: CGWindowListCreateImageFn,
    window_list_create_image_from_array: CGWindowListCreateImageFromArrayFn,
    image_get_width: CGImageGetWidthFn,
    image_get_height: CGImageGetHeightFn,
    image_get_bytes_per_row: CGImageGetBytesPerRowFn,
    image_get_data_provider: CGImageGetDataProviderFn,
    data_provider_copy_data: CGDataProviderCopyDataFn,
    array_create: CFArrayCreateFn,
    array_get_count: CFArrayGetCountFn,
    array_get_value_at_index: CFArrayGetValueAtIndexFn,
    dict_get_value: CFDictionaryGetValueFn,
    number_create: CFNumberCreateFn,
    number_get_value: CFNumberGetValueFn,
    data_get_byte_ptr: CFDataGetBytePtrFn,
    data_get_length: CFDataGetLengthFn,
    release: CFReleaseFn,
    rect_make_with_dict: CGRectMakeWithDictReprFn,
    k_cf_type_array_callbacks: *const c_void, // kCFTypeArrayCallBacks global
    // 10.15+ only, may not exist on older macOS
    preflight_screen_capture: Option<CGPreflightScreenCaptureAccessFn>,
    request_screen_capture: Option<CGRequestScreenCaptureAccessFn>,
    // Private CGS/SLS
    cgs_default_connection: Option<CGSDefaultConnectionFn>,
    cgs_move_window: Option<CGSMoveWindowFn>,
    // HW capture - returns CFArray of CGImageRef
    hw_capture_window_list: Option<HWCaptureWindowListFn>,
    // For loading PNG from screencapture subprocess
    data_provider_create_with_filename: Option<CGDataProviderCreateWithFilenameFn>,
    image_create_with_png: Option<CGImageCreateWithPNGDataProviderFn>,
    // CFString keys from CG globals
    key_owner_pid: *const c_void,
    key_window_number: *const c_void,
    key_window_bounds: *const c_void,
    key_window_layer: *const c_void,
    key_window_name: Option<*const c_void>,
    key_window_owner_name: Option<*const c_void>,
    cf_string_get_cstring: Option<unsafe extern "C" fn(*const c_void, *mut c_char, isize, u32) -> bool>,
}

unsafe impl Send for CgFns {}
unsafe impl Sync for CgFns {}

static CG_FNS: OnceLock<Option<CgFns>> = OnceLock::new();

fn cg() -> Option<&'static CgFns> {
    CG_FNS.get_or_init(|| unsafe { resolve_cg_fns() }).as_ref()
}

unsafe fn resolve_sym<T>(name: &[u8]) -> Option<T> {
    let sym = name.as_ptr() as *const c_char;
    let ptr = libc::dlsym(libc::RTLD_DEFAULT, sym);
    if ptr.is_null() {
        // strip the trailing NUL for readable logs
        tracing::warn!("failed to resolve {}", String::from_utf8_lossy(&name[..name.len()-1]));
        return None;
    }
    Some(std::mem::transmute_copy(&ptr))
}

// CFString globals are ptr-to-ptr, deref once to get the actual CFStringRef
unsafe fn resolve_cf_string_key(name: &[u8]) -> Option<*const c_void> {
    let ptr = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char);
    if ptr.is_null() {
        tracing::warn!("failed to resolve CF key {}", String::from_utf8_lossy(&name[..name.len()-1]));
        None
    } else {
        let val = *(ptr as *const *const c_void);
        if val.is_null() { None } else { Some(val) }
    }
}

unsafe fn resolve_cg_fns() -> Option<CgFns> {
    let k_cf_type_array_callbacks = {
        let ptr = libc::dlsym(libc::RTLD_DEFAULT, b"kCFTypeArrayCallBacks\0".as_ptr() as *const c_char);
        if ptr.is_null() {
            tracing::warn!("failed to resolve kCFTypeArrayCallBacks");
            return None;
        }
        ptr as *const c_void
    };

    Some(CgFns {
        window_list_copy_info: resolve_sym(b"CGWindowListCopyWindowInfo\0")?,
        window_list_create_image: resolve_sym(b"CGWindowListCreateImage\0")?,
        window_list_create_image_from_array: resolve_sym(b"CGWindowListCreateImageFromArray\0")?,
        image_get_width: resolve_sym(b"CGImageGetWidth\0")?,
        image_get_height: resolve_sym(b"CGImageGetHeight\0")?,
        image_get_bytes_per_row: resolve_sym(b"CGImageGetBytesPerRow\0")?,
        image_get_data_provider: resolve_sym(b"CGImageGetDataProvider\0")?,
        data_provider_copy_data: resolve_sym(b"CGDataProviderCopyData\0")?,
        array_create: resolve_sym(b"CFArrayCreate\0")?,
        array_get_count: resolve_sym(b"CFArrayGetCount\0")?,
        array_get_value_at_index: resolve_sym(b"CFArrayGetValueAtIndex\0")?,
        dict_get_value: resolve_sym(b"CFDictionaryGetValue\0")?,
        number_create: resolve_sym(b"CFNumberCreate\0")?,
        number_get_value: resolve_sym(b"CFNumberGetValue\0")?,
        data_get_byte_ptr: resolve_sym(b"CFDataGetBytePtr\0")?,
        data_get_length: resolve_sym(b"CFDataGetLength\0")?,
        release: resolve_sym(b"CFRelease\0")?,
        rect_make_with_dict: resolve_sym(b"CGRectMakeWithDictionaryRepresentation\0")?,
        k_cf_type_array_callbacks,
        preflight_screen_capture: resolve_sym(b"CGPreflightScreenCaptureAccess\0"), // 10.15+
        request_screen_capture: resolve_sym(b"CGRequestScreenCaptureAccess\0"),     // 10.15+
        cgs_default_connection: resolve_sym(b"_CGSDefaultConnection\0"),
        cgs_move_window: resolve_sym(b"CGSMoveWindow\0"),
        // Try SLS (newer macOS) then CGS (older) for hardware capture
        hw_capture_window_list: resolve_sym::<HWCaptureWindowListFn>(b"SLSHWCaptureWindowList\0")
            .or_else(|| resolve_sym(b"CGSHWCaptureWindowList\0")),
        data_provider_create_with_filename: resolve_sym(b"CGDataProviderCreateWithFilename\0"),
        image_create_with_png: resolve_sym(b"CGImageCreateWithPNGDataProvider\0"),
        key_owner_pid: resolve_cf_string_key(b"kCGWindowOwnerPID\0")?,
        key_window_number: resolve_cf_string_key(b"kCGWindowNumber\0")?,
        key_window_bounds: resolve_cf_string_key(b"kCGWindowBounds\0")?,
        key_window_layer: resolve_cf_string_key(b"kCGWindowLayer\0")?,
        key_window_name: resolve_cf_string_key(b"kCGWindowName\0"),
        key_window_owner_name: resolve_cf_string_key(b"kCGWindowOwnerName\0"),
        cf_string_get_cstring: resolve_sym(b"CFStringGetCString\0"),
    })
}

// --- Window discovery ---

// Pull an i32 out of a CG window info dict
unsafe fn dict_get_i32(cg: &CgFns, dict: CFDictionaryRef, key: *const c_void) -> Option<i32> {
    let num_ref = (cg.dict_get_value)(dict, key);
    if num_ref.is_null() { return None; }
    let mut val: i32 = 0;
    if (cg.number_get_value)(num_ref, K_CF_NUMBER_SINT32_TYPE, &mut val as *mut i32 as *mut c_void) {
        Some(val)
    } else {
        None
    }
}

unsafe fn dict_get_string(cg: &CgFns, dict: CFDictionaryRef, key: *const c_void) -> Option<String> {
    let cf_str = (cg.dict_get_value)(dict, key);
    if cf_str.is_null() { return None; }
    let get_cstr = cg.cf_string_get_cstring?;
    let mut buf = [0u8; 256];
    // kCFStringEncodingUTF8 = 0x08000100
    if get_cstr(cf_str, buf.as_mut_ptr() as *mut c_char, 256, 0x08000100) {
        let len = buf.iter().position(|&b| b == 0).unwrap_or(256);
        Some(String::from_utf8_lossy(&buf[..len]).into_owned())
    } else {
        None
    }
}

unsafe fn dict_get_bounds(cg: &CgFns, dict: CFDictionaryRef) -> Option<CGRect> {
    let bounds_dict = (cg.dict_get_value)(dict, cg.key_window_bounds);
    if bounds_dict.is_null() { return None; }
    let mut rect = std::mem::zeroed::<CGRect>();
    if (cg.rect_make_with_dict)(bounds_dict, &mut rect) {
        Some(rect)
    } else {
        None
    }
}

struct CgWindowInfo {
    window_id: u32,
    bounds: CGRect,
    area: f64,
    title: Option<String>,
    owner: Option<String>,
}

// find all CG windows for a PID, sorted by title then newest.
// filters Java class-loader ghost windows and system overlays (layer >= 100).
fn find_windows_by_pid(cg: &CgFns, target_pid: u32) -> Vec<CgWindowInfo> {
    unsafe {
        let info_array = (cg.window_list_copy_info)(K_CG_WINDOW_LIST_OPTION_ALL, 0);
        if info_array.is_null() {
            return Vec::new();
        }

        let count = (cg.array_get_count)(info_array);
        let mut results = Vec::new();

        for i in 0..count {
            let dict = (cg.array_get_value_at_index)(info_array, i);
            if dict.is_null() { continue; }

            let wid = match dict_get_i32(cg, dict, cg.key_window_number) {
                Some(w) if w > 0 => w as u32,
                _ => continue,
            };

            // layers 0-99 = normal + floating/utility. >= 100 = system overlays, skip
            let layer = dict_get_i32(cg, dict, cg.key_window_layer).unwrap_or(-1);
            if layer < 0 || layer >= 100 { continue; }

            let bounds = match dict_get_bounds(cg, dict) {
                Some(b) if b.size.width > 50.0 && b.size.height > 50.0 => b,
                _ => continue,
            };

            let title = cg.key_window_name.and_then(|k| dict_get_string(cg, dict, k));
            let owner = cg.key_window_owner_name.and_then(|k| dict_get_string(cg, dict, k));

            // Skip Java class-loader ghost windows (untitled, layer 0, square ~500x500)
            let is_ghost = title.as_ref().map_or(true, |t| t.is_empty())
                && layer == 0
                && (bounds.size.width - bounds.size.height).abs() < 10.0;
            if is_ghost { continue; }

            // NBB and Minecraft can share the same PID (both "java").
            // Skip windows that are clearly Minecraft (have "Minecraft" in title).
            if title.as_ref().map_or(false, |t| t.contains("Minecraft")) {
                continue;
            }

            let pid = match dict_get_i32(cg, dict, cg.key_owner_pid) {
                Some(p) => p,
                None => continue,
            };
            if pid as u32 != target_pid { continue; }

            // Filter loading screens (small) but allow NBB (420x236)
            if bounds.size.width <= 100.0 || bounds.size.height <= 100.0 { continue; }
            let area = bounds.size.width * bounds.size.height;

            results.push(CgWindowInfo { window_id: wid, bounds, area, title, owner });
        }

        (cg.release)(info_array);

        // dump all windows every 5s while we haven't found anything
        let nbb_empty = results.is_empty();
        if nbb_empty {
            static LAST_DUMP: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
            let should_dump = LAST_DUMP.lock().ok().map(|mut m| {
                let now = Instant::now();
                let do_it = m.as_ref().map_or(true, |prev| now.duration_since(*prev) > Duration::from_secs(5));
                if do_it { *m = Some(now); }
                do_it
            }).unwrap_or(false);
            if should_dump {
                let info2 = (cg.window_list_copy_info)(K_CG_WINDOW_LIST_OPTION_ALL, 0);
                if !info2.is_null() {
                    let c2 = (cg.array_get_count)(info2);
                    for i in 0..c2 {
                        let d = (cg.array_get_value_at_index)(info2, i);
                        if d.is_null() { continue; }
                        let layer = dict_get_i32(cg, d, cg.key_window_layer).unwrap_or(-1);
                        let b = dict_get_bounds(cg, d).unwrap_or(CGRect::NULL);
                        // Skip tiny windows but show everything else (no layer filter)
                        if b.size.width < 50.0 && b.size.height < 50.0 { continue; }
                        let wid = dict_get_i32(cg, d, cg.key_window_number).unwrap_or(0);
                        let pid = dict_get_i32(cg, d, cg.key_owner_pid).unwrap_or(0);
                        let t = cg.key_window_name.and_then(|k| dict_get_string(cg, d, k));
                        let o = cg.key_window_owner_name.and_then(|k| dict_get_string(cg, d, k));
                        tracing::info!(wid, pid, layer,
                            w = b.size.width, h = b.size.height,
                            title = ?t, owner = ?o,
                            "ALL_WINDOWS_DUMP");
                    }
                    (cg.release)(info2 as CFTypeRef);
                }
            }
        }

        // titled windows first, then newest by window ID
        results.sort_by(|a, b| {
            let a_titled = a.title.as_ref().map_or(false, |t| !t.is_empty());
            let b_titled = b.title.as_ref().map_or(false, |t| !t.is_empty());
            b_titled.cmp(&a_titled).then(b.window_id.cmp(&a.window_id))
        });

        // log discovered windows (deduped, only logs when the set changes)
        {
            static LAST_WIDS: std::sync::Mutex<Option<HashMap<u32, Vec<u32>>>> =
                std::sync::Mutex::new(None);
            let current_wids: Vec<u32> = results.iter().map(|w| w.window_id).collect();
            let changed = LAST_WIDS.lock().ok()
                .map(|mut m| {
                    let map = m.get_or_insert_with(HashMap::new);
                    let prev = map.get(&target_pid);
                    let changed = prev.map_or(true, |p| *p != current_wids);
                    if changed { map.insert(target_pid, current_wids.clone()); }
                    changed
                }).unwrap_or(true);
            if changed {
                for w in &results {
                    tracing::info!(target_pid, wid = w.window_id,
                        x = w.bounds.origin.x, y = w.bounds.origin.y,
                        w = w.bounds.size.width, h = w.bounds.size.height,
                        title = ?w.title, owner = ?w.owner,
                        area = w.area,
                        "find_windows_by_pid: window");
                }
                if results.is_empty() {
                    tracing::info!(target_pid, "find_windows_by_pid: no windows");
                }
            }
        }

        results
    }
}

// --- Pixel capture ---

// HW capture path (SLS/CGS). Works for Metal windows where CG returns NULL.
fn capture_via_hw(cg: &CgFns, conn: CGSConnectionID, window_id: u32) -> Option<(Vec<u8>, u32, u32)> {
    let hw_capture = cg.hw_capture_window_list?;

    unsafe {
        let wids: [u32; 1] = [window_id];
        // Try option 0 (default) first
        let images = hw_capture(conn, wids.as_ptr(), 1, 0);
        if images.is_null() {
            tracing::info!(window_id, "HW capture: returned null array");
            return None;
        }

        let count = (cg.array_get_count)(images);
        if count < 1 {
            tracing::info!(window_id, count, "HW capture: empty array");
            (cg.release)(images as CFTypeRef);
            return None;
        }

        // The array contains CGImageRef objects (confirmed by yabai source)
        let image: CGImageRef = (cg.array_get_value_at_index)(images, 0);
        if image.is_null() {
            tracing::info!(window_id, "HW capture: element 0 is null");
            (cg.release)(images as CFTypeRef);
            return None;
        }

        let w = (cg.image_get_width)(image);
        let h = (cg.image_get_height)(image);
        let bytes_per_row = (cg.image_get_bytes_per_row)(image);

        if w == 0 || h == 0 {
            (cg.release)(images as CFTypeRef);
            return None;
        }

        let provider = (cg.image_get_data_provider)(image);
        if provider.is_null() {
            (cg.release)(images as CFTypeRef);
            return None;
        }

        let data = (cg.data_provider_copy_data)(provider);
        if data.is_null() {
            (cg.release)(images as CFTypeRef);
            return None;
        }

        let ptr = (cg.data_get_byte_ptr)(data);
        let len = (cg.data_get_length)(data) as usize;

        // BGRA -> RGBA (same swizzle as the CG path)
        let expected = w * 4;
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let row_start = row * bytes_per_row;
            if row_start + expected > len { break; }
            let row_slice = std::slice::from_raw_parts(ptr.add(row_start), expected);
            for chunk in row_slice.chunks_exact(4) {
                rgba.push(chunk[2]); // R
                rgba.push(chunk[1]); // G
                rgba.push(chunk[0]); // B
                rgba.push(chunk[3]); // A
            }
        }

        (cg.release)(data as CFTypeRef);
        (cg.release)(images as CFTypeRef);

        Some((rgba, w as u32, h as u32))
    }
}

// `screencapture -l <wid>` subprocess fallback. Runs on burst (input forwarded)
// or every 2s idle. Slow but works when all API methods fail.
fn capture_via_screencapture(cg: &CgFns, window_id: u32) -> Option<(Vec<u8>, u32, u32)> {
    let burst = SCREENCAP_BURST.load(std::sync::atomic::Ordering::Relaxed);
    if burst > 0 {
        SCREENCAP_BURST.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        // During burst, throttle to one screencapture per ~100ms (subprocess is slow)
        static LAST_BURST_CALL: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        if let Ok(mut last) = LAST_BURST_CALL.lock() {
            let now = Instant::now();
            if let Some(prev) = last.as_ref() {
                if now.duration_since(*prev) < Duration::from_millis(100) {
                    return None;
                }
            }
            *last = Some(now);
        }
    } else {
        // No burst - baseline refresh every 2 seconds
        static LAST_IDLE_CALL: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        if let Ok(mut last) = LAST_IDLE_CALL.lock() {
            let now = Instant::now();
            if let Some(prev) = last.as_ref() {
                if now.duration_since(*prev) < Duration::from_secs(2) {
                    return None;
                }
            }
            *last = Some(now);
        }
    }

    let create_provider = cg.data_provider_create_with_filename?;
    let create_png_image = cg.image_create_with_png?;

    let path = format!("/tmp/tuxinjector_cap_{}.png\0", window_id);

    // Run screencapture: -l = window ID, -x = no sound, -t png = PNG format
    let output = std::process::Command::new("screencapture")
        .args(&["-l", &window_id.to_string(), "-x", "-t", "png", &path[..path.len()-1]])
        .output();

    match output {
        Ok(o) if o.status.success() => {},
        Ok(o) => {
            tracing::info!(window_id, code = ?o.status.code(),
                "screencapture: exited with error");
            return None;
        }
        Err(e) => {
            tracing::info!(window_id, error = %e, "screencapture: failed to run");
            return None;
        }
    }

    unsafe {
        // Load PNG via CoreGraphics
        let provider = create_provider(path.as_ptr() as *const c_char);
        if provider.is_null() {
            tracing::info!(window_id, "screencapture: CGDataProviderCreateWithFilename returned null");
            let _ = std::fs::remove_file(&path[..path.len()-1]);
            return None;
        }

        // intent 0 = kCGRenderingIntentDefault
        let image = create_png_image(provider, std::ptr::null(), false, 0);
        (cg.release)(provider as CFTypeRef);
        let _ = std::fs::remove_file(&path[..path.len()-1]);

        if image.is_null() {
            tracing::info!(window_id, "screencapture: CGImageCreateWithPNGDataProvider returned null");
            return None;
        }

        let w = (cg.image_get_width)(image);
        let h = (cg.image_get_height)(image);
        let bytes_per_row = (cg.image_get_bytes_per_row)(image);

        if w == 0 || h == 0 {
            (cg.release)(image);
            return None;
        }

        let img_provider = (cg.image_get_data_provider)(image);
        if img_provider.is_null() {
            (cg.release)(image);
            return None;
        }

        let data = (cg.data_provider_copy_data)(img_provider);
        if data.is_null() {
            (cg.release)(image);
            return None;
        }

        let ptr = (cg.data_get_byte_ptr)(data);
        let len = (cg.data_get_length)(data) as usize;

        // CGImage from PNG is typically BGRA premultiplied
        let expected = w * 4;
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let row_start = row * bytes_per_row;
            if row_start + expected > len { break; }
            let row_slice = std::slice::from_raw_parts(ptr.add(row_start), expected);
            // BGRA -> RGBA, force alpha=255 (Java AWT Metal windows have alpha=0)
            for chunk in row_slice.chunks_exact(4) {
                rgba.push(chunk[2]); // R
                rgba.push(chunk[1]); // G
                rgba.push(chunk[0]); // B
                rgba.push(255);      // A - force opaque
            }
        }

        // Log raw BGRA values from the original data BEFORE releasing
        let raw_b = *ptr.add(0);
        let raw_g = *ptr.add(1);
        let raw_r = *ptr.add(2);
        let raw_a = *ptr.add(3);

        (cg.release)(data as CFTypeRef);
        (cg.release)(image);

        tracing::info!(window_id, w, h, raw_r, raw_g, raw_b, raw_a,
            "screencapture: got pixels (raw BGRA[0], alpha forced to 255)");
        Some((rgba, w as u32, h as u32))
    }
}

#[derive(Debug)]
enum CaptureFailure {
    NullImage { methods_tried: u8 },
    ZeroSize(usize, usize),
    NullProvider,
    NullData,
}

unsafe fn try_cg_capture(cg: &CgFns, rect: CGRect, list_opt: u32, wid: u32, img_opt: u32) -> CGImageRef {
    (cg.window_list_create_image)(rect, list_opt, wid, img_opt)
}

// Different CG code path that often works when CGWindowListCreateImage doesn't
unsafe fn try_cg_capture_from_array(cg: &CgFns, rect: CGRect, wid: u32, img_opt: u32) -> CGImageRef {
    let wid_val: i32 = wid as i32;
    let num = (cg.number_create)(
        std::ptr::null(),
        K_CF_NUMBER_SINT32_TYPE,
        &wid_val as *const i32 as *const c_void,
    );
    if num.is_null() { return std::ptr::null(); }

    // wrap the single window ID in a CFArray
    let values: [*const c_void; 1] = [num as *const c_void];
    let arr = (cg.array_create)(
        std::ptr::null(),
        values.as_ptr(),
        1,
        cg.k_cf_type_array_callbacks,
    );
    if arr.is_null() {
        (cg.release)(num as CFTypeRef);
        return std::ptr::null();
    }

    let img = (cg.window_list_create_image_from_array)(rect, arr, img_opt);

    (cg.release)(arr as CFTypeRef);
    (cg.release)(num as CFTypeRef);
    img
}

// Grab window pixels as RGBA. We try a bunch of flag combos because CG is
// picky and different macOS versions want different things. Returns (pixels, w, h, method).
fn capture_window_pixels(cg: &CgFns, window_id: u32, bounds: CGRect) -> Result<(Vec<u8>, u32, u32, u8), CaptureFailure> {
    unsafe {
        let mut method: u8;

        // 1: simplest - just IncludingWindow + CGRectNull
        method = 1;
        let image = try_cg_capture(cg, CGRect::NULL,
            K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
            window_id,
            K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING);
        let m1_ok = !image.is_null();

        // 2: IncludingWindow + explicit bounds (crop to window region)
        let image = if image.is_null() {
            method = 2;
            try_cg_capture(cg, bounds,
                K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
                window_id,
                K_CG_WINDOW_IMAGE_DEFAULT)
        } else { image };
        let m2_ok = !image.is_null() && method == 2;

        // 3: FromArray - different CG code path, sometimes works when above don't
        let image = if image.is_null() {
            method = 3;
            try_cg_capture_from_array(cg, CGRect::NULL, window_id,
                K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING)
        } else { image };
        let m3_ok = !image.is_null() && method == 3;

        // 4: FromArray + ShouldBeOpaque
        let image = if image.is_null() {
            method = 4;
            try_cg_capture_from_array(cg, CGRect::NULL, window_id,
                K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING | K_CG_WINDOW_IMAGE_SHOULD_BE_OPAQUE)
        } else { image };
        let m4_ok = !image.is_null() && method == 4;

        {
            static CG_LOG_COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let n = CG_LOG_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 5 || n % 60 == 0 {
                tracing::info!(window_id,
                    bounds_x = bounds.origin.x, bounds_y = bounds.origin.y,
                    bounds_w = bounds.size.width, bounds_h = bounds.size.height,
                    m1_ok, m2_ok, m3_ok, m4_ok,
                    final_method = method,
                    "CG capture methods: m1={m1_ok} m2={m2_ok} m3={m3_ok} m4={m4_ok}");
            }
        }

        if image.is_null() {
            return Err(CaptureFailure::NullImage { methods_tried: method });
        }

        let w = (cg.image_get_width)(image);
        let h = (cg.image_get_height)(image);
        let bytes_per_row = (cg.image_get_bytes_per_row)(image);

        if w == 0 || h == 0 {
            (cg.release)(image);
            return Err(CaptureFailure::ZeroSize(w, h));
        }

        let provider = (cg.image_get_data_provider)(image);
        if provider.is_null() {
            (cg.release)(image);
            return Err(CaptureFailure::NullProvider);
        }

        let data = (cg.data_provider_copy_data)(provider);
        if data.is_null() {
            (cg.release)(image);
            return Err(CaptureFailure::NullData);
        }

        let ptr = (cg.data_get_byte_ptr)(data);
        let len = (cg.data_get_length)(data) as usize;

        // CG gives us BGRA premultiplied, we need RGBA.
        // bytes_per_row can have padding so we can't just swizzle the whole buffer
        let expected = w * 4;
        let mut rgba = Vec::with_capacity(w * h * 4);

        for row in 0..h {
            let row_start = row * bytes_per_row;
            if row_start + expected > len { break; }
            let row_slice = std::slice::from_raw_parts(ptr.add(row_start), expected);
            // BGRA -> RGBA swizzle
            for chunk in row_slice.chunks_exact(4) {
                rgba.push(chunk[2]);
                rgba.push(chunk[1]);
                rgba.push(chunk[0]);
                rgba.push(chunk[3]);
            }
        }

        (cg.release)(data);
        (cg.release)(image);

        Ok((rgba, w as u32, h as u32, method))
    }
}


// --- Window manipulation via private CGS APIs ---

// CGS connection ID (None if private APIs aren't available)
fn cgs_connection(cg: &CgFns) -> Option<CGSConnectionID> {
    unsafe { cg.cgs_default_connection.map(|f| f()) }
}

// WARNING: moving offscreen stops macOS compositing, so SCKit streams
// will only get blank frames after this. Only use for CG capture.
fn hide_window_offscreen(cg: &CgFns, conn: CGSConnectionID, window_id: u32) {
    if let Some(move_fn) = cg.cgs_move_window {
        let offscreen = CGPoint { x: -20000.0, y: -20000.0 };
        let err = unsafe { move_fn(conn, window_id, &offscreen) };
        if err == 0 {
            tracing::info!(window_id, "moved companion window offscreen via CGS");
        } else {
            tracing::warn!(window_id, err, "CGSMoveWindow failed");
        }
    }
}


// --- Public API - mirrors the Linux app_capture.rs interface ---

pub struct CapturedApp {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub anchor_x: f32,
    pub anchor_y: f32,
}

struct EmbeddedWindow {
    window_id: u32,
    bounds: CGRect, // from CG discovery, used as fallback capture rect
    pixels: Option<Vec<u8>>,
    width: u32,
    height: u32,
    last_capture: Instant,
    found_at_frame: u64,
    logged: bool,
    warned_capture: bool, // don't spam capture failure warnings
    hidden: bool,         // already moved offscreen via CGS?
    // 0=HW/SLS, 1-4=CG, 20=SCKit, 30=screencapture
    working_method: u8,
    // when we first got pixels - for timing the discovery window
    first_capture_at: Option<Instant>,
}

// Java Swing takes a bit to finish painting after the window shows up
const STABILIZATION_FRAMES: u64 = 60;

pub struct AppCaptureManager {
    embedded: HashMap<u32, EmbeddedWindow>,
    search_fails: HashMap<u32, u32>,
    frame: u64,
    visible: bool,
    checked_permissions: bool,
}

impl AppCaptureManager {
    pub fn new() -> Self {
        Self {
            embedded: HashMap::new(),
            search_fails: HashMap::new(),
            frame: 0,
            visible: true,
            checked_permissions: false,
        }
    }

    pub fn known_pids(&self) -> Vec<u32> {
        self.embedded.keys().copied().collect()
    }

    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
    }

    // Drain the key queue and pipe everything to companion apps via stdin
    pub fn forward_pending_keys(&self) {
        let all_pids: Vec<u32> = tuxinjector_gui::running_apps::list()
            .iter()
            .map(|a| a.pid)
            .collect();
        if all_pids.is_empty() { return; }

        let events: Vec<(u8, u16, i32)> = match APP_KEY_QUEUE.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => return,
        };
        if events.is_empty() { return; }

        // Input was forwarded to companion app - trigger screencapture burst
        SCREENCAP_BURST.store(100, std::sync::atomic::Ordering::Relaxed);

        for (keycode, mods, jnh_code) in &events {
            let line = format!("KEY {} {} {}\n", keycode, mods, jnh_code);
            for &pid in &all_pids {
                tuxinjector_gui::running_apps::write_stdin(pid, line.as_bytes());
            }
        }
    }

    // no-op on macOS, no tiling WMs to worry about
    pub fn set_float_hint(&mut self, _pid: u32) {}

    pub fn drop_window(&mut self, pid: u32) {
        self.embedded.remove(&pid);
        self.search_fails.remove(&pid);
    }

    pub fn embed(
        &mut self,
        pid: u32,
        vp_w: u32,
        vp_h: u32,
        anchor: tuxinjector_gui::running_apps::Anchor,
    ) -> Option<CapturedApp> {
        let cg = match cg() {
            Some(c) => c,
            None => {
                // don't spam this, just log occasionally
                if self.frame % 300 == 0 {
                    tracing::warn!("CoreGraphics functions not resolved");
                }
                return None;
            }
        };
        self.frame = self.frame.wrapping_add(1);

        // first time only: check if we have screen recording permission
        if !self.checked_permissions {
            self.checked_permissions = true;
            unsafe {
                if let Some(preflight) = cg.preflight_screen_capture {
                    let ok = preflight();
                    tracing::info!(granted = ok, "CGPreflightScreenCaptureAccess");
                    if !ok {
                        // don't call CGRequestScreenCaptureAccess here, it can
                        // crash in SCKit's XPC cleanup when perms are granted
                        // mid-session. user must grant in System Settings manually.
                        // thanks to slackow for finding this bug
                        tracing::warn!("screen recording permission not granted — \
                            grant in System Settings > Privacy & Security > Screen Recording");
                    }
                } else {
                    tracing::info!("screen capture preflight not available (pre-10.15?)");
                }
            }
        }

        // back off on PIDs we keep failing to find
        let fails = self.search_fails.get(&pid).copied().unwrap_or(0);
        if fails > 500 && self.frame % 60 != 0 {
            return None;
        }

        if !self.embedded.contains_key(&pid) {
            let windows = find_windows_by_pid(cg, pid);
            if windows.is_empty() {
                let f = self.search_fails.entry(pid).or_insert(0);
                *f = f.saturating_add(1);
                if *f == 1 || *f % 300 == 0 {
                    tracing::info!(pid, fails = *f, "no CG windows found for PID");
                }
                return None;
            }

            self.search_fails.remove(&pid);
            // Pick the largest window - Java AWT may create small utility
            // windows alongside the real app window.
            let win = windows.iter()
                .max_by(|a, b| a.area.total_cmp(&b.area))
                .unwrap(); // safe: windows is non-empty
            tracing::info!(pid, window_id = win.window_id, count = windows.len(),
                bounds_w = win.bounds.size.width, bounds_h = win.bounds.size.height,
                "found macOS windows for companion app");

            // Start SCKit streaming capture in the background (for Metal windows)
            maybe_start_sckit(win.window_id, win.bounds);

            self.embedded.insert(pid, EmbeddedWindow {
                window_id: win.window_id,
                bounds: win.bounds,
                pixels: None,
                width: 0,
                height: 0,
                last_capture: Instant::now() - CAPTURE_INTERVAL,
                found_at_frame: self.frame,
                logged: false,
                warned_capture: false,
                hidden: false,
                working_method: 0,
                first_capture_at: None,
            });
        }

        // let the app settle before we start grabbing pixels
        {
            let entry = self.embedded.get_mut(&pid)?;
            let age = self.frame.wrapping_sub(entry.found_at_frame);
            if age < STABILIZATION_FRAMES {
                return None;
            }
            if !entry.logged {
                entry.logged = true;
                tracing::info!(pid, window_id = entry.window_id, "macOS app capture active");
            }
        }

        if !self.visible {
            return None;
        }

        // throttled capture - no point grabbing 60fps from a timer app
        'capture: {
            let entry = self.embedded.get_mut(&pid)?;

            // Java AWT creates small loading windows first, then the real UI.
            // Rediscover every ~0.5s for the first 10s to find the biggest one.
            {
                let wid = entry.window_id;
                let elapsed = entry.first_capture_at.map(|t| t.elapsed());
                let in_discovery_window = elapsed.map_or(true, |e| e < Duration::from_secs(10));
                let stream_alive = SCKIT_STARTING.lock().ok()
                    .map(|s| s.as_ref().map_or(true, |set| set.contains(&wid)))
                    .unwrap_or(true);

                // rediscover when stream died or during the 10s discovery window
                let needs_rediscovery = !stream_alive
                    || (in_discovery_window && self.frame % 30 == 0);

                if needs_rediscovery {
                    let current = find_windows_by_pid(cg, pid);
                    if self.frame % 60 == 0 {
                        let window_ids: Vec<u32> = current.iter().map(|w| w.window_id).collect();
                        tracing::info!(wid, stream_alive, in_discovery_window,
                            ?window_ids, n_windows = current.len(),
                            "rediscovery check");
                    }
                    if let Some(best) = current.iter().max_by(|a, b| a.area.total_cmp(&b.area)) {
                        if best.window_id != wid {
                            tracing::info!(pid, old_wid = wid, new_wid = best.window_id,
                                new_w = best.bounds.size.width, new_h = best.bounds.size.height,
                                has_pixels = entry.pixels.is_some(),
                                "switching to largest window - clearing pixels");
                            entry.window_id = best.window_id;
                            entry.bounds = best.bounds;
                            // Reset pixels - don't keep stale loading screen frames
                            entry.pixels = None;
                            entry.width = 0;
                            entry.height = 0;
                            entry.warned_capture = false;
                            entry.hidden = false;
                            entry.working_method = 0;
                            // Clear gave_up for the NEW window so SCKit gets a fair try
                            if let Ok(mut m) = SCKIT_GAVE_UP.lock() {
                                if let Some(set) = m.as_mut() { set.remove(&best.window_id); }
                            }
                            if let Ok(mut f) = SCKIT_FRAMES.lock() {
                                if let Some(map) = f.as_mut() { map.remove(&wid); }
                            }
                            maybe_start_sckit(entry.window_id, entry.bounds);
                            break 'capture;
                        }
                    }
                    if !stream_alive {
                        let gave_up = SCKIT_GAVE_UP.lock().ok()
                            .map(|m| m.as_ref().map_or(false, |s| s.contains(&wid)))
                            .unwrap_or(false);
                        if gave_up {
                            // SCKit already failed for this window - don't restart,
                            // fall through to the capture poll where CG will handle it.
                            if self.frame % 60 == 0 {
                                tracing::info!(wid, "SCKit gave up on this window, skipping restart - CG will handle it");
                            }
                            // DON'T break - fall through to capture poll below
                        } else if current.iter().any(|w| w.window_id == wid) {
                            // Only restart if the window still exists - don't chase
                            // dead windows (causes futile SCKit setups and crashes)
                            maybe_start_sckit(entry.window_id, entry.bounds);
                            break 'capture;
                        } else {
                            break 'capture;
                        }
                    }
                }
            }

            let now = Instant::now();
            let interval = if entry.working_method == 20 {
                // SCKit streams push frames - just poll the buffer
                Duration::from_millis(33)
            } else {
                CAPTURE_INTERVAL
            };
            if now.duration_since(entry.last_capture) >= interval || entry.pixels.is_none() {
                let wid = entry.window_id;
                let bounds = entry.bounds;

                // try SCKit first, then HW capture, then CG fallbacks.
                // don't fall back while SCKit is still warming up or we'll
                // hide the window before SCKit can ever composite it.
                let sckit_result = get_sckit_frame(wid);
                let sckit_active = SCKIT_STARTING.lock().ok()
                    .and_then(|s| s.as_ref().map(|set| set.contains(&wid)))
                    .unwrap_or(false);
                let sckit_gave_up = SCKIT_GAVE_UP.lock().ok()
                    .map(|m| m.as_ref().map_or(false, |s| s.contains(&wid)))
                    .unwrap_or(false);

                let conn = cgs_connection(cg);
                let hw_result = if sckit_result.is_none() && (!sckit_active || sckit_gave_up) {
                    conn.and_then(|c| capture_via_hw(cg, c, wid))
                } else { None };

                // Log every capture decision (throttled to every 60 frames)
                if self.frame % 60 == 0 || entry.pixels.is_none() {
                    let has_hw = cg.hw_capture_window_list.is_some();
                    let has_conn = conn.is_some();
                    let hw_ok = hw_result.is_some();
                    let sckit_ok = sckit_result.is_some();
                    let has_pixels = entry.pixels.is_some();
                    tracing::info!(pid, wid, sckit_ok, sckit_active, sckit_gave_up,
                        has_hw, has_conn, hw_ok, has_pixels,
                        method = entry.working_method,
                        "capture poll: SCKit={sckit_ok} active={sckit_active} gave_up={sckit_gave_up} \
                         HW={hw_ok} pixels={has_pixels}");
                }

                let capture_result = if let Some((pixels, w, h)) = sckit_result {
                    if self.frame % 60 == 0 || entry.pixels.is_none() {
                        tracing::info!(wid, w, h, "capture: using SCKit frame");
                    }
                    Ok((pixels, w, h, 20u8)) // method 20 = SCKit stream
                } else if let Some((pixels, w, h)) = hw_result {
                    if self.frame % 60 == 0 || entry.pixels.is_none() {
                        tracing::info!(wid, w, h, "capture: using HW capture frame");
                    }
                    Ok((pixels, w, h, 0u8)) // method 0 = HW capture
                } else if !sckit_active || sckit_gave_up {
                    if self.frame % 60 == 0 || entry.pixels.is_none() {
                        tracing::info!(wid, sckit_gave_up, "capture: SCKit inactive or gave up, trying CG then screencapture");
                    }
                    let cg_result = capture_window_pixels(cg, wid, bounds);
                    if cg_result.is_err() && sckit_gave_up {
                        // All API methods failed - try screencapture subprocess as last resort
                        if let Some((pixels, w, h)) = capture_via_screencapture(cg, wid) {
                            Ok((pixels, w, h, 30u8)) // method 30 = screencapture subprocess
                        } else {
                            cg_result // return the CG error
                        }
                    } else {
                        cg_result
                    }
                } else {
                    // SCKit active but no frame yet. If stuck >3s, restart -
                    // SCKit misses the first render if started before the
                    // window has a backing store (Java AWT timing issue).
                    let stuck = SCKIT_STREAM_START.lock().ok()
                        .and_then(|m| m.as_ref().and_then(|map| map.get(&wid).copied()))
                        .map(|started| started.elapsed() > Duration::from_secs(3))
                        .unwrap_or(false);
                    let already_restarted = SCKIT_RESTARTED.lock().ok()
                        .map(|m| m.as_ref().map_or(false, |set| set.contains(&wid)))
                        .unwrap_or(false);

                    if stuck && !already_restarted {
                        tracing::info!(wid, "SCKit stuck (no frames after 3s), restarting stream");
                        // Remove from SCKIT_STARTING so maybe_start_sckit will proceed
                        if let Ok(mut s) = SCKIT_STARTING.lock() {
                            if let Some(set) = s.as_mut() { set.remove(&wid); }
                        }
                        maybe_start_sckit(wid, bounds);
                        // Record restart AFTER maybe_start_sckit (which clears SCKIT_RESTARTED)
                        if let Ok(mut m) = SCKIT_RESTARTED.lock() {
                            m.get_or_insert_with(HashSet::new).insert(wid);
                        }
                        break 'capture;
                    } else if stuck && already_restarted {
                        // SCKit failed twice - give up on it and fall through to CG capture
                        tracing::info!(wid, "SCKit failed after restart, falling back to CG capture");
                        if let Ok(mut s) = SCKIT_STARTING.lock() {
                            if let Some(set) = s.as_mut() { set.remove(&wid); }
                        }
                        {
                            let cg_result = capture_window_pixels(cg, wid, bounds);
                            if cg_result.is_err() {
                                if let Some((pixels, w, h)) = capture_via_screencapture(cg, wid) {
                                    Ok((pixels, w, h, 30u8))
                                } else { cg_result }
                            } else { cg_result }
                        }
                    } else {
                        break 'capture;
                    }
                };

                match capture_result {
                    Ok((pixels, w, h, method)) => {
                        // log content check but don't reject (was blocking valid loading screens)
                        if entry.pixels.is_none() {
                            let has_content = frame_has_content(&pixels, w, h);
                            let r = pixels.get(0).copied().unwrap_or(0);
                            let g = pixels.get(1).copied().unwrap_or(0);
                            let b = pixels.get(2).copied().unwrap_or(0);
                            tracing::info!(wid, w, h, method, has_content, r, g, b,
                                "capture: first frame accepted - pixel[0]=({r},{g},{b}) content={has_content}");
                        } else if self.frame % 60 == 0 {
                            tracing::info!(wid, w, h, method, "capture: accepting frame");
                        }
                        if entry.pixels.is_none() {
                            let method_name = match method {
                                0 => "SLSHWCapture",
                                20 => "SCKit stream",
                                _ => "CGWindowListCreateImage",
                            };
                            tracing::info!(pid, w, h, wid, method, method_name, "first capture succeeded");
                            entry.first_capture_at = Some(Instant::now());
                        }
                        entry.pixels = Some(pixels);
                        entry.width = w;
                        entry.height = h;
                        entry.last_capture = now;
                        entry.working_method = method;
                        // only hide for CG methods (1-4). SCKit/screencapture need
                        // the window composited or they get blank frames.
                        if !entry.hidden && method <= 4 {
                            entry.hidden = true;
                            if let Some(c) = conn {
                                hide_window_offscreen(cg, c, wid);
                            }
                        }
                    }
                    Err(ref fail) => {
                        if self.frame % 60 == 0 || !entry.warned_capture {
                            tracing::info!(wid, ?fail, "capture: CG/HW capture failed");
                        }
                        if !entry.warned_capture {
                            entry.warned_capture = true;
                            match fail {
                                CaptureFailure::NullImage { methods_tried } => {
                                    // try a screen-wide capture to figure out if it's
                                    // a permission problem or something window-specific
                                    let test = unsafe {
                                        let infinite = CGRect {
                                            origin: CGPoint { x: 0.0, y: 0.0 },
                                            size: CGSize { width: 1.0, height: 1.0 },
                                        };
                                        (cg.window_list_create_image)(
                                            infinite,
                                            K_CG_WINDOW_LIST_OPTION_ON_SCREEN_ONLY,
                                            0, // kCGNullWindowID
                                            K_CG_WINDOW_IMAGE_DEFAULT,
                                        )
                                    };
                                    let screen_works = !test.is_null();
                                    if screen_works {
                                        unsafe { (cg.release)(test as CFTypeRef) };
                                    }
                                    // NOTE: if screen_capture_works is false, user needs to grant
                                    // Screen Recording permissions for the java executable 
                                    // in System Settings and then fully restart the app
                                    tracing::warn!(pid, wid, methods_tried,
                                        screen_capture_works = screen_works,
                                        bounds_x = bounds.origin.x,
                                        bounds_y = bounds.origin.y,
                                        bounds_w = bounds.size.width,
                                        bounds_h = bounds.size.height,
                                        "CG capture returned NULL after {methods_tried} methods. \
                                         If screen_capture_works=false, grant Screen Recording \
                                         permission and fully restart (Cmd+Q) the app."
                                    );
                                }
                                CaptureFailure::ZeroSize(w, h) => {
                                    tracing::warn!(pid, wid, w, h, "CG capture returned zero-size image");
                                }
                                CaptureFailure::NullProvider => {
                                    tracing::warn!(pid, wid, "CG capture: null data provider");
                                }
                                CaptureFailure::NullData => {
                                    tracing::warn!(pid, wid, "CG capture: CGDataProviderCopyData returned null");
                                }
                            }
                        }
                        // Java AWT loves recreating windows, check if the ID changed
                        if self.frame % 30 == 0 {
                            let current = find_windows_by_pid(cg, pid);
                            if current.is_empty() {
                                // keep showing last good frame, window will reappear
                                let has_pixels = self.embedded.get(&pid)
                                    .map_or(false, |e| e.pixels.is_some());
                                if !has_pixels {
                                    self.embedded.remove(&pid);
                                    return None;
                                }
                                // Keep showing last good frame
                                break 'capture;
                            }
                            let entry = self.embedded.get_mut(&pid).unwrap();
                            let new_wid = current[0].window_id;
                            if new_wid != entry.window_id {
                                tracing::info!(pid, old_wid = entry.window_id, new_wid,
                                    "window ID changed, re-targeting");
                                let old_wid = entry.window_id;
                                entry.window_id = new_wid;
                                entry.bounds = current[0].bounds;
                                entry.warned_capture = false;
                                entry.hidden = false;
                                entry.working_method = 0;
                                // keep old_wid in SCKIT_STARTING for alive detection
                                if let Ok(mut f) = SCKIT_FRAMES.lock() {
                                    if let Some(map) = f.as_mut() { map.remove(&old_wid); }
                                }
                                maybe_start_sckit(new_wid, current[0].bounds);
                            }
                        }
                    }
                }
            }
        }

        let entry = self.embedded.get(&pid)?;
        let pixels = entry.pixels.as_ref()?;
        if entry.width == 0 || entry.height == 0 {
            return None;
        }

        let (off_x, off_y) = anchor.position(
            vp_w as i32, vp_h as i32,
            entry.width as i32, entry.height as i32,
            0,
        );

        if self.frame % 120 == 0 {
            tracing::info!(pid, wid = entry.window_id,
                app_w = entry.width, app_h = entry.height,
                vp_w, vp_h, off_x, off_y,
                method = entry.working_method,
                pixel_len = pixels.len(),
                "returning CapturedApp: anchor=({off_x},{off_y}) size={}x{}",
                entry.width, entry.height);
        }

        Some(CapturedApp {
            pixels: pixels.clone(),
            width: entry.width,
            height: entry.height,
            anchor_x: off_x as f32,
            anchor_y: off_y as f32,
        })
    }
}

// --- ScreenCaptureKit streaming capture ---
// SCStream approach (like OBS/SlackowWall) for Metal Java AWT windows.

// Latest captured frame per window ID (set by SCStream callback, read by embed())
static SCKIT_FRAMES: std::sync::Mutex<Option<HashMap<u32, (Vec<u8>, u32, u32)>>> =
    std::sync::Mutex::new(None);

// Track which windows have an SCKit stream being set up
static SCKIT_STARTING: std::sync::Mutex<Option<HashSet<u32>>> =
    std::sync::Mutex::new(None);

// delegate address -> window_id (callback needs to know which window)
static DELEGATE_WID_MAP: std::sync::Mutex<Option<HashMap<usize, u32>>> =
    std::sync::Mutex::new(None);

// Serialize SCKit setup (async completion handlers use shared globals)
static SCKIT_SETUP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// Per-window count of consecutive NULL pixel buffers from SCKit callback
static SCKIT_NULL_COUNTS: std::sync::Mutex<Option<HashMap<u32, u32>>> =
    std::sync::Mutex::new(None);

// Track when each SCKit stream was started (for stuck-stream detection)
static SCKIT_STREAM_START: std::sync::Mutex<Option<HashMap<u32, Instant>>> =
    std::sync::Mutex::new(None);

// Track which wids have already been restarted (prevent infinite restart loops)
static SCKIT_RESTARTED: std::sync::Mutex<Option<HashSet<u32>>> =
    std::sync::Mutex::new(None);

// windows where SCKit stopped without delivering valid frames - don't restart
static SCKIT_GAVE_UP: std::sync::Mutex<Option<HashSet<u32>>> =
    std::sync::Mutex::new(None);

// Send+Sync wrapper for raw ObjC pointers in statics
#[derive(Clone, Copy)]
struct RawPtr(*mut c_void);
unsafe impl Send for RawPtr {}
unsafe impl Sync for RawPtr {}

// active SCKit streams: window_id -> (SCStream*, delegate*)
static SCKIT_STREAMS: std::sync::Mutex<Option<HashMap<u32, (RawPtr, RawPtr)>>> =
    std::sync::Mutex::new(None);

// Async completion for SCShareableContent enumeration
static CONTENT_RESULT: (std::sync::Mutex<Option<(RawPtr, bool)>>, Condvar) =
    (std::sync::Mutex::new(None), Condvar::new());

// Async completion for SCStream.startCapture
static START_RESULT: (std::sync::Mutex<Option<bool>>, Condvar) =
    (std::sync::Mutex::new(None), Condvar::new());

// ObjC block ABI structures
#[repr(C)]
struct ObjcBlock {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: *const c_void,
    descriptor: *const BlockDesc,
}

unsafe impl Send for ObjcBlock {}
unsafe impl Sync for ObjcBlock {}

#[repr(C)]
struct BlockDesc {
    reserved: usize,
    size: usize,
}

static BLOCK_DESC: BlockDesc = BlockDesc {
    reserved: 0,
    size: std::mem::size_of::<ObjcBlock>(),
};

// Additional function types for SCKit
type ObjcAllocClassPairFn = unsafe extern "C" fn(*mut c_void, *const c_char, usize) -> *mut c_void;
type ObjcRegClassPairFn = unsafe extern "C" fn(*mut c_void);
type ClassAddMethodFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void, *const c_char) -> bool;
type ClassAddProtocolFn = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;
type ObjcGetProtocolFn = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type DispatchQueueCreateFn = unsafe extern "C" fn(*const c_char, *const c_void) -> *mut c_void;
type CMSampleBufferGetImageBufFn = unsafe extern "C" fn(*const c_void) -> *const c_void;
type CVPixelBufLockFn = unsafe extern "C" fn(*const c_void, u64) -> i32;
type CVPixelBufUnlockFn = unsafe extern "C" fn(*const c_void, u64) -> i32;
type CVPixelBufGetBaseFn = unsafe extern "C" fn(*const c_void) -> *const u8;
type CVPixelBufGetSizeFn = unsafe extern "C" fn(*const c_void) -> usize;

struct ScKitFns {
    cls: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    sel: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    msg: *const c_void, // objc_msgSend - cast per call site
    alloc_cls: ObjcAllocClassPairFn,
    reg_cls: ObjcRegClassPairFn,
    add_method: ClassAddMethodFn,
    add_protocol: ClassAddProtocolFn,
    get_protocol: ObjcGetProtocolFn,
    global_block_isa: *const c_void,
    cm_get_img_buf: CMSampleBufferGetImageBufFn,
    cv_lock: CVPixelBufLockFn,
    cv_unlock: CVPixelBufUnlockFn,
    cv_base: CVPixelBufGetBaseFn,
    cv_w: CVPixelBufGetSizeFn,
    cv_h: CVPixelBufGetSizeFn,
    cv_bpr: CVPixelBufGetSizeFn,
    dq_create: DispatchQueueCreateFn,
    retain: unsafe extern "C" fn(*const c_void) -> *const c_void,
    #[allow(dead_code)]
    release: unsafe extern "C" fn(*const c_void),
}

unsafe impl Send for ScKitFns {}
unsafe impl Sync for ScKitFns {}

static SCKIT_FNS: OnceLock<Option<ScKitFns>> = OnceLock::new();

fn sckit() -> Option<&'static ScKitFns> {
    SCKIT_FNS.get_or_init(|| unsafe { resolve_sckit() }).as_ref()
}

unsafe fn resolve_sckit() -> Option<ScKitFns> {
    // Load ScreenCaptureKit and CoreMedia frameworks
    libc::dlopen(
        b"/System/Library/Frameworks/ScreenCaptureKit.framework/ScreenCaptureKit\0".as_ptr() as _,
        libc::RTLD_LAZY,
    );
    libc::dlopen(
        b"/System/Library/Frameworks/CoreMedia.framework/CoreMedia\0".as_ptr() as _,
        libc::RTLD_LAZY,
    );

    let msg = libc::dlsym(libc::RTLD_DEFAULT, b"objc_msgSend\0".as_ptr() as _);
    let gbi = libc::dlsym(libc::RTLD_DEFAULT, b"_NSConcreteGlobalBlock\0".as_ptr() as _);
    if msg.is_null() || gbi.is_null() { return None; }

    Some(ScKitFns {
        cls: resolve_sym(b"objc_getClass\0")?,
        sel: resolve_sym(b"sel_registerName\0")?,
        msg,
        alloc_cls: resolve_sym(b"objc_allocateClassPair\0")?,
        reg_cls: resolve_sym(b"objc_registerClassPair\0")?,
        add_method: resolve_sym(b"class_addMethod\0")?,
        add_protocol: resolve_sym(b"class_addProtocol\0")?,
        get_protocol: resolve_sym(b"objc_getProtocol\0")?,
        global_block_isa: gbi,
        cm_get_img_buf: resolve_sym(b"CMSampleBufferGetImageBuffer\0")?,
        cv_lock: resolve_sym(b"CVPixelBufferLockBaseAddress\0")?,
        cv_unlock: resolve_sym(b"CVPixelBufferUnlockBaseAddress\0")?,
        cv_base: resolve_sym(b"CVPixelBufferGetBaseAddress\0")?,
        cv_w: resolve_sym(b"CVPixelBufferGetWidth\0")?,
        cv_h: resolve_sym(b"CVPixelBufferGetHeight\0")?,
        cv_bpr: resolve_sym(b"CVPixelBufferGetBytesPerRow\0")?,
        dq_create: resolve_sym(b"dispatch_queue_create\0")?,
        retain: resolve_sym(b"CFRetain\0")?,
        release: resolve_sym(b"CFRelease\0")?,
    })
}

// --- ObjC delegate class for SCStreamOutput protocol ---

static DELEGATE_CLASS: OnceLock<RawPtr> = OnceLock::new();
fn get_delegate_class() -> *mut c_void {
    DELEGATE_CLASS.get_or_init(|| RawPtr(unsafe { register_delegate_class() })).0
}

unsafe fn register_delegate_class() -> *mut c_void {
    let sk = match sckit() {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };

    let ns_object = (sk.cls)(b"NSObject\0".as_ptr() as _);
    let cls = (sk.alloc_cls)(ns_object, b"TuxSCStreamOutput\0".as_ptr() as _, 0);
    if cls.is_null() {
        // Already registered
        return (sk.cls)(b"TuxSCStreamOutput\0".as_ptr() as _);
    }

    // Add SCStreamOutput + SCStreamDelegate protocols
    let p1 = (sk.get_protocol)(b"SCStreamOutput\0".as_ptr() as _);
    if !p1.is_null() { (sk.add_protocol)(cls, p1); }
    let p2 = (sk.get_protocol)(b"SCStreamDelegate\0".as_ptr() as _);
    if !p2.is_null() { (sk.add_protocol)(cls, p2); }

    // stream:didOutputSampleBuffer:ofType:
    // type encoding: v=void @=id :=SEL @=object @=object q=int64(NSInteger)
    (sk.add_method)(
        cls,
        (sk.sel)(b"stream:didOutputSampleBuffer:ofType:\0".as_ptr() as _),
        on_stream_output as *const c_void,
        b"v@:@@q\0".as_ptr() as _,
    );

    // stream:didStopWithError:
    (sk.add_method)(
        cls,
        (sk.sel)(b"stream:didStopWithError:\0".as_ptr() as _),
        on_stream_stop as *const c_void,
        b"v@:@@\0".as_ptr() as _,
    );

    (sk.reg_cls)(cls);
    cls
}

// SCStreamOutput callback - receives video frames from the capture stream
unsafe extern "C" fn on_stream_output(
    self_: *mut c_void,
    _sel: *mut c_void,
    _stream: *mut c_void,
    sample_buffer: *mut c_void,
    output_type: isize,
) {
    if output_type != 0 || sample_buffer.is_null() { return; } // 0 = screen

    let sk = match sckit() {
        Some(s) => s,
        None => return,
    };

    // Look up which window_id this delegate belongs to
    let wid = match DELEGATE_WID_MAP.lock().ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(&(self_ as usize)).copied()))
    {
        Some(w) => w,
        None => return,
    };

    // Diagnostic: log first callback per wid (before any pixel checks)
    {
        static CB_FIRST: std::sync::Mutex<Option<HashSet<u32>>> =
            std::sync::Mutex::new(None);
        let is_first = CB_FIRST.lock().ok()
            .map(|mut m| m.get_or_insert_with(HashSet::new).insert(wid as u32))
            .unwrap_or(false);
        if is_first {
            tracing::info!(wid, "SCKit callback fired (first for this window)");
        }
    }

    // NULL pixel buffer is normal for idle frames (no new content composited)
    let pixel_buffer = (sk.cm_get_img_buf)(sample_buffer);
    if pixel_buffer.is_null() {
        // Count consecutive NULL frames per window for diagnostics
        if let Ok(mut m) = SCKIT_NULL_COUNTS.lock() {
            let count = m.get_or_insert_with(HashMap::new)
                .entry(wid).or_insert(0);
            *count += 1;
            // Log every 100 NULL frames so we can see if SCKit is alive but idle
            if *count % 100 == 0 {
                tracing::info!(wid, null_count = *count,
                    "SCKit: {} consecutive NULL pixel buffers", *count);
            }
        }
        return;
    }

    // Reset NULL counter on valid frame
    if let Ok(mut m) = SCKIT_NULL_COUNTS.lock() {
        if let Some(map) = m.as_mut() { map.remove(&wid); }
    }

    // Lock for reading
    if (sk.cv_lock)(pixel_buffer, 1) != 0 { return; } // 1 = kCVPixelBufferLock_ReadOnly

    let base = (sk.cv_base)(pixel_buffer);
    let w = (sk.cv_w)(pixel_buffer);
    let h = (sk.cv_h)(pixel_buffer);
    let bpr = (sk.cv_bpr)(pixel_buffer);

    // Per-window first-frame log
    {
        static LOGGED_WIDS: std::sync::Mutex<Option<HashSet<u32>>> =
            std::sync::Mutex::new(None);
        let is_new = LOGGED_WIDS.lock().ok()
            .map(|mut m| m.get_or_insert_with(HashSet::new).insert(wid as u32))
            .unwrap_or(false);
        if is_new {
            tracing::info!(wid, w, h, bpr, expected_bpr = w * 4,
                "SCKit first frame for window");
        }
    }

    if !base.is_null() && w > 0 && h > 0 {
        // BGRA -> RGBA, force alpha=255 (Java AWT has transparent areas)
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let row_ptr = base.add(row * bpr);
            for col in 0..w {
                let px = row_ptr.add(col * 4);
                rgba.push(*px.add(2)); // R
                rgba.push(*px.add(1)); // G
                rgba.push(*px);        // B
                rgba.push(255);        // A (force opaque)
            }
        }

        if let Ok(mut frames) = SCKIT_FRAMES.lock() {
            let map = frames.get_or_insert_with(HashMap::new);
            map.insert(wid, (rgba, w as u32, h as u32));
        }
    }

    (sk.cv_unlock)(pixel_buffer, 1);
}

// SCStreamDelegate callback - stream error
unsafe extern "C" fn on_stream_stop(
    self_: *mut c_void,
    _sel: *mut c_void,
    _stream: *mut c_void,
    _error: *mut c_void,
) {
    let wid = DELEGATE_WID_MAP.lock().ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(&(self_ as usize)).copied()));
    match wid {
        Some(wid) => {
            // Check if this stream ever delivered valid frames
            let had_frames = SCKIT_FRAMES.lock().ok()
                .and_then(|f| f.as_ref().map(|m| m.contains_key(&wid)))
                .unwrap_or(false);
            tracing::warn!(wid, had_frames, "SCKit stream stopped");
            if let Ok(mut s) = SCKIT_STARTING.lock() {
                if let Some(set) = s.as_mut() { set.remove(&wid); }
            }
            // no valid frames -> mark as gave_up so we don't loop
            if !had_frames {
                tracing::info!(wid, "SCKit gave up on window (no valid frames before stop)");
                if let Ok(mut m) = SCKIT_GAVE_UP.lock() {
                    m.get_or_insert_with(HashSet::new).insert(wid);
                }
            }
        }
        None => {
            // Orphaned stream stopped - expected after stop_all_sckit_streams
        }
    }
}

// --- Async completion handler blocks ---

unsafe extern "C" fn content_handler_invoke(
    _block: *mut ObjcBlock,
    content: *mut c_void,
    error: *mut c_void,
) {
    // Retain content to survive autorelease pool drain
    if !content.is_null() {
        if let Some(sk) = sckit() { (sk.retain)(content as _); }
    }
    let (lock, cvar) = &CONTENT_RESULT;
    if let Ok(mut r) = lock.lock() {
        *r = Some((RawPtr(content), error.is_null()));
        cvar.notify_one();
    }
}

unsafe extern "C" fn start_handler_invoke(
    _block: *mut ObjcBlock,
    error: *mut c_void,
) {
    let (lock, cvar) = &START_RESULT;
    if let Ok(mut r) = lock.lock() {
        *r = Some(error.is_null());
        cvar.notify_one();
    }
}

// --- Stream setup (runs on background thread) ---

fn sckit_stream_setup(window_id: u32, width: u32, height: u32) -> Result<(), String> {
    // Serialize setup (completion handler globals are shared)
    let _guard = SCKIT_SETUP_LOCK.lock().map_err(|e| format!("lock: {e}"))?;

    let sk = sckit().ok_or("SCKit functions not resolved")?;

    unsafe {
        // Typed objc_msgSend casts
        type Send0 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void;
        type Send1P = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
        type Send1I = unsafe extern "C" fn(*mut c_void, *mut c_void, isize);
        type Send1B = unsafe extern "C" fn(*mut c_void, *mut c_void, i8);
        type Send3P = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
        type SendAddOut = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, isize, *mut c_void, *mut *mut c_void) -> bool;
        type SendGetU32 = unsafe extern "C" fn(*mut c_void, *mut c_void) -> u32;
        type SendGetIsize = unsafe extern "C" fn(*mut c_void, *mut c_void) -> isize;
        type SendEnum = unsafe extern "C" fn(*mut c_void, *mut c_void, i8, i8, *const ObjcBlock);
        type SendStart = unsafe extern "C" fn(*mut c_void, *mut c_void, *const ObjcBlock);

        let s0: Send0 = std::mem::transmute(sk.msg);
        let s1p: Send1P = std::mem::transmute(sk.msg);
        let s1i: Send1I = std::mem::transmute(sk.msg);
        let s1b: Send1B = std::mem::transmute(sk.msg);
        let s3p: Send3P = std::mem::transmute(sk.msg);
        let s_add: SendAddOut = std::mem::transmute(sk.msg);
        let s_u32: SendGetU32 = std::mem::transmute(sk.msg);
        let s_isz: SendGetIsize = std::mem::transmute(sk.msg);
        let s_enum: SendEnum = std::mem::transmute(sk.msg);
        let s_start: SendStart = std::mem::transmute(sk.msg);

        let sel = |n: &[u8]| (sk.sel)(n.as_ptr() as _);
        let cls = |n: &[u8]| (sk.cls)(n.as_ptr() as _);

        // --- 1. Enumerate SCShareableContent ---
        let sc_cls = cls(b"SCShareableContent\0");
        if sc_cls.is_null() {
            return Err("SCShareableContent not found (macOS < 12.3?)".into());
        }

        let handler = ObjcBlock {
            isa: sk.global_block_isa,
            flags: 1 << 28, // BLOCK_IS_GLOBAL
            reserved: 0,
            invoke: content_handler_invoke as *const c_void,
            descriptor: &BLOCK_DESC,
        };

        { let (l, _) = &CONTENT_RESULT; *l.lock().unwrap() = None; }

        // [SCShareableContent getShareableContentExcludingDesktopWindows:NO
        //                     onScreenWindowsOnly:NO completionHandler:block]
        s_enum(
            sc_cls,
            sel(b"getShareableContentExcludingDesktopWindows:onScreenWindowsOnly:completionHandler:\0"),
            0, 0, &handler,
        );

        // Wait for completion
        let content = {
            let (lock, cvar) = &CONTENT_RESULT;
            let mut r = lock.lock().unwrap();
            let timeout = Duration::from_secs(5);
            while r.is_none() {
                let (nr, to) = cvar.wait_timeout(r, timeout).unwrap();
                r = nr;
                if to.timed_out() { return Err("SCShareableContent timed out".into()); }
            }
            let (ptr, ok) = r.take().unwrap();
            if !ok || ptr.0.is_null() { return Err("SCShareableContent failed".into()); }
            ptr.0
        };

        // --- 2. Find matching SCWindow by windowID ---
        let windows = s0(content, sel(b"windows\0"));
        if windows.is_null() {
            return Err("windows array is nil".into());
        }

        let count = s_isz(windows, sel(b"count\0"));
        let mut target: *mut c_void = std::ptr::null_mut();

        for i in 0..count {
            let win: *mut c_void = s1p(windows, sel(b"objectAtIndex:\0"), i as *mut c_void);
            if win.is_null() { continue; }
            let wid = s_u32(win, sel(b"windowID\0"));
            if wid == window_id {
                target = win;
                break;
            }
        }

        if target.is_null() {
            return Err(format!("SCWindow not found for wid {window_id}"));
        }

        tracing::info!(window_id, "SCKit: found SCWindow");

        // --- 3. Create SCContentFilter(desktopIndependentWindow:) ---
        let filter_cls = cls(b"SCContentFilter\0");
        let filter = s1p(
            s0(filter_cls, sel(b"alloc\0")),
            sel(b"initWithDesktopIndependentWindow:\0"),
            target,
        );
        if filter.is_null() {
            return Err("SCContentFilter init failed".into());
        }

        // --- 4. Create SCStreamConfiguration ---
        let config = s0(s0(cls(b"SCStreamConfiguration\0"), sel(b"alloc\0")), sel(b"init\0"));
        if config.is_null() {
            return Err("SCStreamConfiguration init failed".into());
        }

        s1i(config, sel(b"setWidth:\0"), width as isize);
        s1i(config, sel(b"setHeight:\0"), height as isize);
        // kCVPixelFormatType_32BGRA = 'BGRA' = 0x42475241
        // Without this, SCKit defaults to YUV 420v and bpr != w*4
        s1i(config, sel(b"setPixelFormat:\0"), 0x42475241_isize);
        s1b(config, sel(b"setShowsCursor:\0"), 0);
        s1b(config, sel(b"setCapturesAudio:\0"), 0);
        s1b(config, sel(b"setScalesToFit:\0"), 1);
        s1i(config, sel(b"setQueueDepth:\0"), 6);

        // Set minimum frame interval to 1/30s (same as SlackowWall).
        // CMTime { value: 1, timescale: 30, flags: 1 (valid), epoch: 0 }
        #[repr(C)]
        struct CMTime { value: i64, timescale: i32, flags: u32, epoch: i64 }
        type SendCMTime = unsafe extern "C" fn(*mut c_void, *mut c_void, CMTime);
        let s_cmtime: SendCMTime = std::mem::transmute(sk.msg);
        s_cmtime(config, sel(b"setMinimumFrameInterval:\0"),
            CMTime { value: 1, timescale: 30, flags: 1, epoch: 0 });

        // --- 5. Create delegate ---
        let del_cls = get_delegate_class();
        if del_cls.is_null() {
            return Err("delegate class failed".into());
        }

        let delegate = s0(s0(del_cls, sel(b"alloc\0")), sel(b"init\0"));
        if delegate.is_null() {
            return Err("delegate alloc failed".into());
        }

        // Store window_id mapping for the callback
        {
            let mut map = DELEGATE_WID_MAP.lock().unwrap();
            map.get_or_insert_with(HashMap::new).insert(delegate as usize, window_id);
        }

        // --- 6. Create SCStream ---
        let stream = s3p(
            s0(cls(b"SCStream\0"), sel(b"alloc\0")),
            sel(b"initWithFilter:configuration:delegate:\0"),
            filter, config, delegate,
        );
        if stream.is_null() {
            return Err("SCStream init failed".into());
        }

        // --- 7. Add stream output ---
        let queue = (sk.dq_create)(b"com.tuxinjector.sckit\0".as_ptr() as _, std::ptr::null());
        let mut err_ptr: *mut c_void = std::ptr::null_mut();
        let ok = s_add(
            stream,
            sel(b"addStreamOutput:type:sampleHandlerQueue:error:\0"),
            delegate, 0, // SCStreamOutputType.screen = 0
            queue, &mut err_ptr,
        );
        if !ok {
            return Err("addStreamOutput failed".into());
        }

        // --- 8. Start capture ---
        { let (l, _) = &START_RESULT; *l.lock().unwrap() = None; }

        let start_block = ObjcBlock {
            isa: sk.global_block_isa,
            flags: 1 << 28,
            reserved: 0,
            invoke: start_handler_invoke as *const c_void,
            descriptor: &BLOCK_DESC,
        };

        s_start(stream, sel(b"startCaptureWithCompletionHandler:\0"), &start_block);

        let started = {
            let (lock, cvar) = &START_RESULT;
            let mut r = lock.lock().unwrap();
            let timeout = Duration::from_secs(5);
            while r.is_none() {
                let (nr, to) = cvar.wait_timeout(r, timeout).unwrap();
                r = nr;
                if to.timed_out() { return Err("startCapture timed out".into()); }
            }
            r.take().unwrap()
        };

        if !started {
            return Err("startCapture failed".into());
        }

        tracing::info!(window_id, width, height, "SCKit stream active");

        // a newer maybe_start_sckit may have cleared SCKIT_STARTING
        // while we were setting up - if so, orphan this stream
        let still_wanted = SCKIT_STARTING.lock().ok()
            .map(|s| s.as_ref().map_or(false, |set| set.contains(&window_id)))
            .unwrap_or(false);

        if !still_wanted {
            tracing::info!(window_id, "SCKit setup superseded, orphaning stream");
            // don't call stopCapture - window may be dead. just unmap delegate
            if let Ok(mut m) = DELEGATE_WID_MAP.lock() {
                if let Some(map) = m.as_mut() { map.remove(&(delegate as usize)); }
            }
            // stream/delegate leaked - no release, no stopCapture
            return Err("superseded by newer setup".into());
        }

        // Store stream+delegate so we can stop them later
        if let Ok(mut m) = SCKIT_STREAMS.lock() {
            m.get_or_insert_with(HashMap::new)
                .insert(window_id, (RawPtr(stream), RawPtr(delegate)));
        }

        // intentionally leak ObjC objects (~1KB). releasing during rapid Java AWT
        // window lifecycle crashes at objc_release+0x8.

        Ok(())
    }
}

// kick off SCKit stream setup on a background thread
fn maybe_start_sckit(window_id: u32, bounds: CGRect) {
    {
        let mut s = SCKIT_STARTING.lock().unwrap();
        let set = s.get_or_insert_with(HashSet::new);
        if set.contains(&window_id) { return; }
        // macOS only delivers frames to one stream at a time
        drop(s); // release lock before calling stop
        stop_all_sckit_streams();
        let mut s = SCKIT_STARTING.lock().unwrap();
        let set = s.get_or_insert_with(HashSet::new);
        set.insert(window_id);
    }

    // point dimensions - SCKit handles Retina scaling, may come back at 2x
    let w = bounds.size.width as u32;
    let h = bounds.size.height as u32;

    // Record start time for stuck-stream detection
    if let Ok(mut m) = SCKIT_STREAM_START.lock() {
        m.get_or_insert_with(HashMap::new).insert(window_id, Instant::now());
    }

    tracing::info!(window_id, w, h, "starting SCKit stream setup");

    std::thread::spawn(move || {
        match sckit_stream_setup(window_id, w, h) {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(window_id, error = %e, "SCKit setup failed");
                if let Ok(mut s) = SCKIT_STARTING.lock() {
                    if let Some(set) = s.as_mut() { set.remove(&window_id); }
                }
            }
        }
    });
}

// orphan all active SCKit streams. we never call stopCapture because the
// underlying window may already be dead - causes objc_release+0x8 crash.
// orphaned streams stop on their own via on_stream_stop.
fn stop_all_sckit_streams() {
    // Clear SCKIT_STARTING to cancel any in-flight setup threads.
    if let Ok(mut s) = SCKIT_STARTING.lock() {
        if let Some(set) = s.as_mut() { set.clear(); }
    }

    // Reset restart tracker, gave-up tracker, and start times for fresh tracking
    if let Ok(mut m) = SCKIT_RESTARTED.lock() {
        if let Some(set) = m.as_mut() { set.clear(); }
    }
    if let Ok(mut m) = SCKIT_GAVE_UP.lock() {
        if let Some(set) = m.as_mut() { set.clear(); }
    }
    if let Ok(mut m) = SCKIT_STREAM_START.lock() {
        if let Some(map) = m.as_mut() { map.clear(); }
    }

    // Drain SCKIT_STREAMS.
    let streams: Vec<(u32, *mut c_void, *mut c_void)> = {
        let mut guard = match SCKIT_STREAMS.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match guard.as_mut() {
            Some(map) => map.drain().map(|(wid, (s, d))| (wid, s.0, d.0)).collect(),
            None => return,
        }
    };

    for (wid, _stream, delegate) in streams {
        tracing::info!(wid, "orphaning SCKit stream");
        // Remove delegate mapping so orphaned callbacks become no-ops
        if let Ok(mut m) = DELEGATE_WID_MAP.lock() {
            if let Some(map) = m.as_mut() { map.remove(&(delegate as usize)); }
        }
        // Clean up frame buffer
        if let Ok(mut f) = SCKIT_FRAMES.lock() {
            if let Some(map) = f.as_mut() { map.remove(&wid); }
        }
        // stream/delegate leaked - no Drop, no release
    }
}

// latest SCKit frame for a window (called from render thread)
fn get_sckit_frame(window_id: u32) -> Option<(Vec<u8>, u32, u32)> {
    SCKIT_FRAMES.lock().ok()?.as_ref()?.get(&window_id).cloned()
}

// check if frame has real content (not solid color). samples a 4x4 grid,
// needs >= 2 different pixels. Java AWT shows solid white/black during init.
fn frame_has_content(pixels: &[u8], w: u32, h: u32) -> bool {
    let expected = (w as usize) * (h as usize) * 4;
    if w == 0 || h == 0 || pixels.len() < expected { return false; }
    let first_r = pixels[0];
    let first_g = pixels[1];
    let first_b = pixels[2];
    let mut different = 0u32;
    for gy in 0..4u32 {
        for gx in 0..4u32 {
            let x = (gx * 2 + 1) * w / 8;
            let y = (gy * 2 + 1) * h / 8;
            let idx = ((y * w + x) * 4) as usize;
            if idx + 2 >= pixels.len() { continue; }
            if pixels[idx].abs_diff(first_r) > 8
                || pixels[idx + 1].abs_diff(first_g) > 8
                || pixels[idx + 2].abs_diff(first_b) > 8
            {
                different += 1;
            }
        }
    }
    different >= 2
}

