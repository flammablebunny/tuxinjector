// macOS window capture via CoreGraphics.
// Background polling thread per session, matches windows by title/owner name.

use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::CapturedFrame;

// --- CG/CF FFI types (all resolved via dlsym, no linking) ---

type CFTypeRef = *const c_void;
type CFArrayRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFNumberRef = *const c_void;
type CFDataRef = *const c_void;
type CGImageRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

impl CGRect {
    // CGRectNull - CG uses the window's own bounds when you pass this
    const NULL: Self = Self {
        origin: CGPoint {
            x: f64::INFINITY,
            y: f64::INFINITY,
        },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    };
}

const K_CG_WINDOW_LIST_OPTION_ALL: u32 = 0;
const K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW: u32 = 1 << 3;
const K_CG_WINDOW_IMAGE_DEFAULT: u32 = 0;
const K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING: u32 = 1 << 0;
const K_CF_NUMBER_SINT32_TYPE: u64 = 3;

type CGWindowListCopyWindowInfoFn = unsafe extern "C" fn(u32, u32) -> CFArrayRef;
type CGWindowListCreateImageFn = unsafe extern "C" fn(CGRect, u32, u32, u32) -> CGImageRef;
type CGImageGetWidthFn = unsafe extern "C" fn(CGImageRef) -> usize;
type CGImageGetHeightFn = unsafe extern "C" fn(CGImageRef) -> usize;
type CGImageGetBytesPerRowFn = unsafe extern "C" fn(CGImageRef) -> usize;
type CGImageGetDataProviderFn = unsafe extern "C" fn(CGImageRef) -> *const c_void;
type CGDataProviderCopyDataFn = unsafe extern "C" fn(*const c_void) -> CFDataRef;
type CFArrayGetCountFn = unsafe extern "C" fn(CFArrayRef) -> isize;
type CFArrayGetValueAtIndexFn = unsafe extern "C" fn(CFArrayRef, isize) -> *const c_void;
type CFDictionaryGetValueFn = unsafe extern "C" fn(CFDictionaryRef, *const c_void) -> *const c_void;
type CFNumberGetValueFn = unsafe extern "C" fn(CFNumberRef, u64, *mut c_void) -> bool;
type CFStringGetCStringFn = unsafe extern "C" fn(*const c_void, *mut c_char, isize, u32) -> bool;
type CFDataGetBytePtrFn = unsafe extern "C" fn(CFDataRef) -> *const u8;
type CFDataGetLengthFn = unsafe extern "C" fn(CFDataRef) -> isize;
type CFReleaseFn = unsafe extern "C" fn(CFTypeRef);

// Resolved CG/CF function pointers + dictionary keys for window info
struct CgFns {
    window_list_copy_info: CGWindowListCopyWindowInfoFn,
    window_list_create_image: CGWindowListCreateImageFn,
    image_get_width: CGImageGetWidthFn,
    image_get_height: CGImageGetHeightFn,
    image_get_bytes_per_row: CGImageGetBytesPerRowFn,
    image_get_data_provider: CGImageGetDataProviderFn,
    data_provider_copy_data: CGDataProviderCopyDataFn,
    array_get_count: CFArrayGetCountFn,
    array_get_value_at_index: CFArrayGetValueAtIndexFn,
    dict_get_value: CFDictionaryGetValueFn,
    number_get_value: CFNumberGetValueFn,
    string_get_cstring: CFStringGetCStringFn,
    data_get_byte_ptr: CFDataGetBytePtrFn,
    data_get_length: CFDataGetLengthFn,
    release: CFReleaseFn,
    // CGWindowInfo dictionary keys
    key_owner_pid: *const c_void,
    key_window_number: *const c_void,
    key_window_name: Option<*const c_void>,
    key_window_owner_name: Option<*const c_void>,
    key_window_layer: *const c_void,
}

unsafe impl Send for CgFns {}
unsafe impl Sync for CgFns {}

static CG_FNS: OnceLock<Option<CgFns>> = OnceLock::new();

fn cg() -> Option<&'static CgFns> {
    CG_FNS
        .get_or_init(|| unsafe { resolve_cg_fns() })
        .as_ref()
}

pub fn cg_available() -> bool {
    cg().is_some()
}

unsafe fn resolve_sym<T>(name: &[u8]) -> Option<T> {
    let ptr = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char);
    if ptr.is_null() {
        tracing::warn!(
            "mac_capture: failed to resolve {}",
            String::from_utf8_lossy(&name[..name.len() - 1])
        );
        return None;
    }
    Some(std::mem::transmute_copy(&ptr))
}

// CFString globals are ptr-to-ptr, so deref once to get the real CFStringRef
unsafe fn resolve_cf_key(name: &[u8]) -> Option<*const c_void> {
    let ptr = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr() as *const c_char);
    if ptr.is_null() {
        return None;
    }
    let val = *(ptr as *const *const c_void);
    if val.is_null() {
        None
    } else {
        Some(val)
    }
}

unsafe fn resolve_cg_fns() -> Option<CgFns> {
    Some(CgFns {
        window_list_copy_info: resolve_sym(b"CGWindowListCopyWindowInfo\0")?,
        window_list_create_image: resolve_sym(b"CGWindowListCreateImage\0")?,
        image_get_width: resolve_sym(b"CGImageGetWidth\0")?,
        image_get_height: resolve_sym(b"CGImageGetHeight\0")?,
        image_get_bytes_per_row: resolve_sym(b"CGImageGetBytesPerRow\0")?,
        image_get_data_provider: resolve_sym(b"CGImageGetDataProvider\0")?,
        data_provider_copy_data: resolve_sym(b"CGDataProviderCopyData\0")?,
        array_get_count: resolve_sym(b"CFArrayGetCount\0")?,
        array_get_value_at_index: resolve_sym(b"CFArrayGetValueAtIndex\0")?,
        dict_get_value: resolve_sym(b"CFDictionaryGetValue\0")?,
        number_get_value: resolve_sym(b"CFNumberGetValue\0")?,
        string_get_cstring: resolve_sym(b"CFStringGetCString\0")?,
        data_get_byte_ptr: resolve_sym(b"CFDataGetBytePtr\0")?,
        data_get_length: resolve_sym(b"CFDataGetLength\0")?,
        release: resolve_sym(b"CFRelease\0")?,
        key_owner_pid: resolve_cf_key(b"kCGWindowOwnerPID\0")?,
        key_window_number: resolve_cf_key(b"kCGWindowNumber\0")?,
        key_window_name: resolve_cf_key(b"kCGWindowName\0"),
        key_window_owner_name: resolve_cf_key(b"kCGWindowOwnerName\0"),
        key_window_layer: resolve_cf_key(b"kCGWindowLayer\0")?,
    })
}

// --- Window discovery helpers ---

unsafe fn dict_get_i32(cg: &CgFns, dict: CFDictionaryRef, key: *const c_void) -> Option<i32> {
    let num_ref = (cg.dict_get_value)(dict, key);
    if num_ref.is_null() {
        return None;
    }
    let mut val: i32 = 0;
    if (cg.number_get_value)(num_ref, K_CF_NUMBER_SINT32_TYPE, &mut val as *mut i32 as *mut c_void)
    {
        Some(val)
    } else {
        None
    }
}

unsafe fn dict_get_string(cg: &CgFns, dict: CFDictionaryRef, key: *const c_void) -> Option<String> {
    let cf_str = (cg.dict_get_value)(dict, key);
    if cf_str.is_null() {
        return None;
    }
    let mut buf = [0u8; 512];
    // kCFStringEncodingUTF8 = 0x08000100
    if (cg.string_get_cstring)(cf_str, buf.as_mut_ptr() as *mut c_char, 512, 0x08000100) {
        let len = buf.iter().position(|&b| b == 0).unwrap_or(512);
        Some(String::from_utf8_lossy(&buf[..len]).into_owned())
    } else {
        None
    }
}

struct MatchCriteria {
    title: Option<String>,
    owner_name: Option<String>,
}

// Find the best matching window ID by title/owner substring match.
// Skips system overlay layers (>=100) and picks the newest match.
fn find_matching_window(cg: &CgFns, criteria: &MatchCriteria) -> Option<u32> {
    unsafe {
        let info_array = (cg.window_list_copy_info)(K_CG_WINDOW_LIST_OPTION_ALL, 0);
        if info_array.is_null() {
            return None;
        }

        let count = (cg.array_get_count)(info_array);
        let mut best: Option<(u32, f64)> = None; // (wid, area)

        for i in 0..count {
            let dict = (cg.array_get_value_at_index)(info_array, i);
            if dict.is_null() {
                continue;
            }

            let wid = match dict_get_i32(cg, dict, cg.key_window_number) {
                Some(w) if w > 0 => w as u32,
                _ => continue,
            };

            // skip system overlay layers
            let layer = dict_get_i32(cg, dict, cg.key_window_layer).unwrap_or(-1);
            if layer < 0 || layer >= 100 {
                continue;
            }

            let title = cg
                .key_window_name
                .and_then(|k| dict_get_string(cg, dict, k));
            let owner = cg
                .key_window_owner_name
                .and_then(|k| dict_get_string(cg, dict, k));

            // check match criteria
            let title_ok = criteria.title.as_ref().map_or(true, |want| {
                title
                    .as_ref()
                    .map_or(false, |t| t.to_lowercase().contains(&want.to_lowercase()))
            });
            let owner_ok = criteria.owner_name.as_ref().map_or(true, |want| {
                owner
                    .as_ref()
                    .map_or(false, |o| o.to_lowercase().contains(&want.to_lowercase()))
            });

            if !title_ok || !owner_ok {
                continue;
            }

            // just take the match with the highest window ID (newest)
            let score = wid as f64;
            if best.as_ref().map_or(true, |(_, s)| score > *s) {
                best = Some((wid, score));
            }
        }

        (cg.release)(info_array);
        best.map(|(wid, _)| wid)
    }
}

// Grab a single frame from the given CG window ID, returns RGBA pixels
fn capture_window(cg: &CgFns, window_id: u32) -> Option<CapturedFrame> {
    unsafe {
        let image = (cg.window_list_create_image)(
            CGRect::NULL,
            K_CG_WINDOW_LIST_OPTION_INCLUDING_WINDOW,
            window_id,
            K_CG_WINDOW_IMAGE_DEFAULT | K_CG_WINDOW_IMAGE_BOUNDS_IGNORE_FRAMING,
        );
        if image.is_null() {
            return None;
        }

        let w = (cg.image_get_width)(image);
        let h = (cg.image_get_height)(image);
        let bpr = (cg.image_get_bytes_per_row)(image);

        if w == 0 || h == 0 {
            (cg.release)(image);
            return None;
        }

        let provider = (cg.image_get_data_provider)(image);
        if provider.is_null() {
            (cg.release)(image);
            return None;
        }

        let data = (cg.data_provider_copy_data)(provider);
        if data.is_null() {
            (cg.release)(image);
            return None;
        }

        let ptr = (cg.data_get_byte_ptr)(data);
        let len = (cg.data_get_length)(data) as usize;

        // BGRA -> RGBA swizzle, force alpha to 255
        let expected_row = w * 4;
        let mut rgba = Vec::with_capacity(w * h * 4);
        for row in 0..h {
            let start = row * bpr;
            if start + expected_row > len {
                break;
            }
            let slice = std::slice::from_raw_parts(ptr.add(start), expected_row);
            for chunk in slice.chunks_exact(4) {
                rgba.push(chunk[2]); // R
                rgba.push(chunk[1]); // G
                rgba.push(chunk[0]); // B
                rgba.push(255); // A (force opaque)
            }
        }

        (cg.release)(data);
        (cg.release)(image);

        Some(CapturedFrame {
            pixels: rgba,
            width: w as u32,
            height: h as u32,
        })
    }
}

// --- Session management ---

struct SharedFrame {
    data: Option<CapturedFrame>,
    updated: Instant,
}

struct CaptureSession {
    shared: Arc<Mutex<SharedFrame>>,
    stop: Arc<AtomicBool>,
    _thread: std::thread::JoinHandle<()>,
}

// One background thread per capture session, polls CG at the configured FPS
pub struct MacCapture {
    sessions: HashMap<String, CaptureSession>,
}

impl MacCapture {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    // Start capturing a window matching the given title/owner criteria.
    // At least one of title or owner_name must be Some.
    pub fn start_capture(
        &mut self,
        id: &str,
        title: Option<&str>,
        owner_name: Option<&str>,
        fps: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.sessions.contains_key(id) {
            tracing::debug!(id, "mac capture session already running");
            return Ok(());
        }

        if title.is_none() && owner_name.is_none() {
            return Err("at least one of title or owner_name must be specified".into());
        }

        let criteria = MatchCriteria {
            title: title.map(|s| s.to_string()),
            owner_name: owner_name.map(|s| s.to_string()),
        };

        let shared = Arc::new(Mutex::new(SharedFrame {
            data: None,
            updated: Instant::now(),
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let shared2 = Arc::clone(&shared);
        let stop2 = Arc::clone(&stop);
        let cap_id = id.to_string();
        let interval = Duration::from_millis(1000 / fps.clamp(1, 240) as u64);

        let handle = std::thread::Builder::new()
            .name(format!("mac-capture-{id}"))
            .spawn(move || {
                capture_loop(&cap_id, &criteria, &shared2, &stop2, interval);
            })
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

        self.sessions.insert(
            id.to_string(),
            CaptureSession {
                shared,
                stop,
                _thread: handle,
            },
        );

        tracing::info!(id, ?title, ?owner_name, fps, "macOS capture session started");
        Ok(())
    }

    pub fn latest_frame(&self, id: &str) -> Option<CapturedFrame> {
        let sess = self.sessions.get(id)?;
        let lock = sess.shared.lock().ok()?;
        let frame = lock.data.as_ref()?;
        Some(CapturedFrame {
            pixels: frame.pixels.clone(),
            width: frame.width,
            height: frame.height,
        })
    }

    pub fn stop_capture(&mut self, id: &str) {
        if let Some(sess) = self.sessions.remove(id) {
            sess.stop.store(true, Ordering::Release);
            tracing::info!(id, "stopped macOS capture");
        }
    }

    pub fn active_sessions(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    // True if we got a frame in the last 2 seconds
    pub fn is_receiving(&self, id: &str) -> bool {
        self.sessions
            .get(id)
            .and_then(|s| s.shared.lock().ok())
            .map(|g| g.updated.elapsed() < Duration::from_secs(2))
            .unwrap_or(false)
    }
}

fn capture_loop(
    id: &str,
    criteria: &MatchCriteria,
    shared: &Arc<Mutex<SharedFrame>>,
    stop: &Arc<AtomicBool>,
    interval: Duration,
) {
    let Some(cg) = cg() else {
        tracing::error!(id, "mac_capture: failed to resolve CoreGraphics functions");
        return;
    };

    let mut current_wid: Option<u32> = None;
    let mut search_timer = Instant::now();

    tracing::debug!(
        id,
        title = ?criteria.title,
        owner = ?criteria.owner_name,
        interval_ms = interval.as_millis(),
        "mac_capture: loop started"
    );

    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }

        // re-search periodically in case the window got recreated
        if current_wid.is_none() || search_timer.elapsed() > Duration::from_secs(3) {
            let new_wid = find_matching_window(cg, criteria);
            if new_wid != current_wid {
                if let Some(wid) = new_wid {
                    tracing::info!(id, wid, "mac_capture: found matching window");
                } else if current_wid.is_some() {
                    tracing::info!(id, "mac_capture: target window lost, searching...");
                }
                current_wid = new_wid;
            }
            search_timer = Instant::now();
        }

        let Some(wid) = current_wid else {
            // no window found yet, poll slower
            std::thread::sleep(Duration::from_millis(500));
            continue;
        };

        match capture_window(cg, wid) {
            Some(frame) => {
                if let Ok(mut g) = shared.lock() {
                    g.data = Some(frame);
                    g.updated = Instant::now();
                }
            }
            None => {
                // capture failed - window probably closed
                tracing::debug!(id, wid, "mac_capture: capture returned null, re-searching");
                current_wid = None;
                continue;
            }
        }

        std::thread::sleep(interval);
    }

    tracing::debug!(id, "mac_capture: loop exited");
}
