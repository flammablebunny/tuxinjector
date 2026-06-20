// GLFW callback interception: stashes the game's real callbacks,
// installs our wrappers that process input before forwarding.
// Just another "LLM Asssisted" file, nothing to see here jojoe

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};

use parking_lot::Mutex;
use tracing::{debug, trace};

use crate::glfw_types::*;

// --- stored game callbacks ---

static GAME_KEY_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static GAME_MOUSE_BTN_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static GAME_CURSOR_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static GAME_SCROLL_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

// --- real glfwSetXxx function pointers (resolved from dlsym) ---

static REAL_SET_KEY_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_SET_MOUSE_BTN_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_SET_CURSOR_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_SET_SCROLL_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_SET_INPUT_MODE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

// glfwSetCursorPos + glfwGetWindowSize: stashed so we can re-center the
// cursor on menu open to prevent intentional cursor misplacement (ICM).
static REAL_SET_CURSOR_POS: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_GET_WINDOW_SIZE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

pub fn store_real_set_cursor_pos(ptr: *mut c_void) {
    if !ptr.is_null() {
        REAL_SET_CURSOR_POS.store(ptr, Ordering::Release);
    }
}

pub fn store_real_get_window_size(ptr: *mut c_void) {
    if !ptr.is_null() {
        REAL_GET_WINDOW_SIZE.store(ptr, Ordering::Release);
    }
}

unsafe fn center_cursor(window: GlfwWindow) {
    type GetWinSizeFn = unsafe extern "C" fn(GlfwWindow, *mut i32, *mut i32);

    let get_ptr = REAL_GET_WINDOW_SIZE.load(Ordering::Acquire);
    if get_ptr.is_null() {
        debug!("center_cursor: real ptrs not resolved yet, skipping");
        return;
    }

    let get_fn: GetWinSizeFn = std::mem::transmute(get_ptr);

    let (mut ww, mut wh) = (0i32, 0i32);
    get_fn(window, &mut ww, &mut wh);
    if ww <= 0 || wh <= 0 { return; }

    let cx = ww as f64 / 2.0;
    let cy = wh as f64 / 2.0;
    warp_cursor(cx, cy);
    debug!(cx, cy, ww, wh, "cursor re-centered (menu transition, ICM prevention)");
}

pub fn warp_cursor(x: f64, y: f64) {
    type SetCursorPosFn = unsafe extern "C" fn(GlfwWindow, f64, f64);

    let win = GLFW_WINDOW.load(Ordering::Acquire);
    let ptr = REAL_SET_CURSOR_POS.load(Ordering::Acquire);
    if win.is_null() || ptr.is_null() {
        return;
    }
    let set_fn: SetCursorPosFn = unsafe { std::mem::transmute(ptr) };
    unsafe { set_fn(win, x, y) };
}

static REAL_GET_KEY_SCANCODE: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

pub fn store_real_get_key_scancode(ptr: *mut c_void) {
    if !ptr.is_null() {
        REAL_GET_KEY_SCANCODE.store(ptr, Ordering::Release);
    }
}
/// Look up the platform scancode for a GLFW key via the real glfwGetKeyScancode.
/// Returns None if the function pointer hasn't been resolved yet or the key has
/// no scancode on the current layout.
pub fn canonical_scancode(key: i32) -> Option<i32> {
    let ptr = REAL_GET_KEY_SCANCODE.load(Ordering::Acquire);
    if ptr.is_null() { return None; }
    type GetKeyScancodeFn = unsafe extern "C" fn(i32) -> i32;
    let f: GetKeyScancodeFn = unsafe { std::mem::transmute(ptr) };
    let sc = unsafe { f(key) };
    if sc > 0 { Some(sc) } else { None }
}

static VIRTUAL_KEYS: Mutex<Option<std::collections::HashMap<i32, std::collections::HashSet<i32>>>> =
    Mutex::new(None);

fn track_virtual_key(physical: i32, logical: i32, action: i32) {
    let mut guard = VIRTUAL_KEYS.lock();
    let map = guard.get_or_insert_with(std::collections::HashMap::new);
    if action == 1 /* PRESS */ {
        map.entry(logical).or_default().insert(physical);
    } else if action == 0 /* RELEASE */ {
        if let Some(set) = map.get_mut(&logical) {
            set.remove(&physical);
            if set.is_empty() {
                map.remove(&logical);
            }
        }
    }
}

pub fn is_key_pressed(key: i32) -> bool {
    let guard = VIRTUAL_KEYS.lock();
    guard.as_ref().map_or(false, |m| {
        m.get(&key).map_or(false, |s| !s.is_empty())
    })
}

// --- "block key from game" hotkeys ---
//
// A hotkey with `block_key_from_game` set must keep its key from reaching the
// game while it's actively held. Suppressing the callback event isn't enough:
// Minecraft reads many binds by polling glfwGetKey, so a consumed key still
// leaks via the poll. We track the logical keys currently owned by such a
// hotkey; glfwGetKey reports them released and the key callback drops their
// press/repeat/release toward the game. Keyed by logical key so it lines up
// with what both the engine matched on and what the game polls.
static BLOCKED_FROM_GAME: Mutex<Option<std::collections::HashSet<i32>>> = Mutex::new(None);

pub fn mark_blocked_from_game(key: i32) {
    BLOCKED_FROM_GAME
        .lock()
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(key);
}

/// Remove a key from the blocked set; returns whether it had been blocked.
pub fn unmark_blocked_from_game(key: i32) -> bool {
    BLOCKED_FROM_GAME
        .lock()
        .as_mut()
        .map_or(false, |s| s.remove(&key))
}

pub fn is_blocked_from_game(key: i32) -> bool {
    BLOCKED_FROM_GAME
        .lock()
        .as_ref()
        .map_or(false, |s| s.contains(&key))
}

fn clear_blocked_from_game() {
    if let Some(s) = BLOCKED_FROM_GAME.lock().as_mut() {
        s.clear();
    }
}

// Remembers which logical key each physical key was mapped to at PRESS
// time. Without this, if the rebinder's decision depends on game state
// (e.g. chat vs game) and the state changes between PRESS and RELEASE,
// the two events get routed to different logical keys and MC ends up
// with the original logical key "stuck" pressed forever. This map forces
// the RELEASE (and any intervening REPEATs) to emit the same logical key
// the PRESS emitted. 
static PRESS_MAPPING: Mutex<Option<std::collections::HashMap<i32, i32>>> =
    Mutex::new(None);

fn record_press_mapping(physical: i32, logical: i32) {
    // No-op for pass-through keys -- keeps the map small (only rebound
    // keys take space) and avoids locking on the hot path for every press.
    if physical == logical { return; }
    let mut guard = PRESS_MAPPING.lock();
    let map = guard.get_or_insert_with(std::collections::HashMap::new);
    map.insert(physical, logical);
}

fn consume_press_mapping(physical: i32) -> Option<i32> {
    let mut guard = PRESS_MAPPING.lock();
    guard.as_mut().and_then(|m| m.remove(&physical))
}

fn peek_press_mapping(physical: i32) -> Option<i32> {
    let guard = PRESS_MAPPING.lock();
    guard.as_ref().and_then(|m| m.get(&physical)).copied()
}

/// Inject a synthetic press+release into the game's callback. GL thread only.
pub unsafe fn press_key_to_game(key: i32) {
    let win = GLFW_WINDOW.load(Ordering::Acquire);
    if win.is_null() {
        return;
    }
    fwd_key(win, key, 0, 1, 0); // PRESS
    fwd_key(win, key, 0, 0, 0); // RELEASE
}

// --- self-driven key repeat (per-key) ---
//
// Repeat rate can't be a global override on Linux: keys like Escape must keep
// the OS cadence (a 5 ms repeat would re-open/close the pause menu). So it's
// opted in per physical key (configured in the rebinds section). For a key in
// the table we swallow its OS GLFW_REPEAT and re-emit at that key's own start
// delay + interval, driven once per frame from the swap hook on the game thread.

// physical key -> (start_delay_ms, interval_ms)
static KR_TABLE: Mutex<Option<std::collections::HashMap<i32, (u32, u32)>>> = Mutex::new(None);

struct KrHeld {
    logical: i32,
    scancode: i32,
    mods: i32,
    interval: std::time::Duration,
    next: std::time::Instant,
}
static KR_HELD: Mutex<Option<std::collections::HashMap<i32, KrHeld>>> = Mutex::new(None);

/// Set the per-key custom-repeat table: `(physical_key, start_delay_ms, interval_ms)`.
/// Keys absent from the table keep the game's native OS repeat.
pub fn set_key_repeat_table(entries: &[(i32, i32, i32)]) {
    let mut map = std::collections::HashMap::new();
    for &(key, start, interval) in entries {
        if interval > 0 {
            map.insert(key, (start.max(0) as u32, interval.max(1) as u32));
        }
    }
    *KR_TABLE.lock() = if map.is_empty() { None } else { Some(map) };
    kr_clear(); // drop stale holds; they re-register on next press
}

fn kr_config_for(physical: i32) -> Option<(u32, u32)> {
    KR_TABLE.lock().as_ref().and_then(|m| m.get(&physical).copied())
}

/// True if this physical key has custom repeat (so its OS repeat is swallowed).
pub fn key_repeat_enabled_for(physical: i32) -> bool {
    KR_TABLE.lock().as_ref().map_or(false, |m| m.contains_key(&physical))
}

fn kr_note_press(physical: i32, logical: i32, scancode: i32, mods: i32) {
    let Some((start, interval)) = kr_config_for(physical) else { return };
    let next = std::time::Instant::now() + std::time::Duration::from_millis(start as u64);
    let mut g = KR_HELD.lock();
    g.get_or_insert_with(std::collections::HashMap::new).insert(
        physical,
        KrHeld {
            logical,
            scancode,
            mods,
            interval: std::time::Duration::from_millis(interval as u64),
            next,
        },
    );
}

fn kr_note_release(physical: i32) {
    if let Some(m) = KR_HELD.lock().as_mut() {
        m.remove(&physical);
    }
}

fn kr_clear() {
    if let Some(m) = KR_HELD.lock().as_mut() {
        m.clear();
    }
}

/// Emit self-driven key repeats. Call once per frame on the game thread.
pub unsafe fn tick_key_repeat() {
    if GUI_VISIBLE.load(Ordering::Relaxed) {
        return;
    }
    let now = std::time::Instant::now();
    let win = GLFW_WINDOW.load(Ordering::Acquire);
    if win.is_null() {
        return;
    }

    // Collect under the lock; emit to the game after releasing it.
    let mut emit: Vec<(i32, i32, i32)> = Vec::new();
    {
        let mut g = KR_HELD.lock();
        let Some(m) = g.as_mut() else { return };
        for h in m.values_mut() {
            let mut n = 0;
            while now >= h.next && n < 8 {
                emit.push((h.logical, h.scancode, h.mods));
                h.next += h.interval;
                n += 1;
            }
            // Resync if we fell badly behind (lag / alt-tab) to avoid a burst.
            if now > h.next + h.interval * 8 {
                h.next = now + h.interval;
            }
        }
    }
    for (logical, scancode, mods) in emit {
        fwd_key(win, logical, scancode, crate::glfw_types::GLFW_REPEAT, mods);
    }
}

// --- mouse position tracking ---

static MOUSE_X: AtomicU64 = AtomicU64::new(0);
static MOUSE_Y: AtomicU64 = AtomicU64::new(0);
static RAW_MOUSE_X: AtomicU64 = AtomicU64::new(0);
static RAW_MOUSE_Y: AtomicU64 = AtomicU64::new(0);

/// Last mouse position in game coordinates (post-sensitivity)
pub fn mouse_position() -> (f64, f64) {
    let x = f64::from_bits(MOUSE_X.load(Ordering::Relaxed));
    let y = f64::from_bits(MOUSE_Y.load(Ordering::Relaxed));
    (x, y)
}

/// Last raw window mouse position (pre-sensitivity)
pub fn raw_mouse_position() -> (f64, f64) {
    let x = f64::from_bits(RAW_MOUSE_X.load(Ordering::Relaxed));
    let y = f64::from_bits(RAW_MOUSE_Y.load(Ordering::Relaxed));
    (x, y)
}

/// Seed the raw mouse position from the GLFW window center if no cursor
/// event has been received yet. Makes sure imgui can establish hover on
/// the first GUI frame (fixes the scroll-after-click issue).
pub fn seed_raw_mouse_if_stale(window_w: u32, window_h: u32) {
    let (rx, ry) = raw_mouse_position();
    if rx == 0.0 && ry == 0.0 && window_w > 0 && window_h > 0 {
        let cx = window_w as f64 / 2.0;
        let cy = window_h as f64 / 2.0;
        RAW_MOUSE_X.store(cx.to_bits(), Ordering::Relaxed);
        RAW_MOUSE_Y.store(cy.to_bits(), Ordering::Relaxed);
    }
}

// --- key rebind maps ---

// reverse: (to_key, from_key) - for glfwGetKey lookups
static REVERSE_REBINDS: std::sync::OnceLock<parking_lot::Mutex<Vec<(i32, i32)>>> =
    std::sync::OnceLock::new();

// forward: (from_key, to_key) - for char callback remapping
static FORWARD_REBINDS: std::sync::OnceLock<parking_lot::Mutex<Vec<(i32, i32)>>> =
    std::sync::OnceLock::new();

fn rev_rebinds() -> &'static parking_lot::Mutex<Vec<(i32, i32)>> {
    REVERSE_REBINDS.get_or_init(|| parking_lot::Mutex::new(Vec::new()))
}

fn fwd_rebinds() -> &'static parking_lot::Mutex<Vec<(i32, i32)>> {
    FORWARD_REBINDS.get_or_init(|| parking_lot::Mutex::new(Vec::new()))
}

/// Update both rebind lookup tables from active (from, to) pairs
pub fn update_key_rebinds(rebinds: &[(i32, i32)]) {
    let reversed: Vec<(i32, i32)> = rebinds.iter().map(|&(f, t)| (t, f)).collect();
    *rev_rebinds().lock() = reversed;
    *fwd_rebinds().lock() = rebinds.to_vec();
}

/// Map a logical keycode back to physical. Returns `key` if no rebind active
pub fn physical_key_for(key: i32) -> i32 {
    let map = rev_rebinds().lock();
    map.iter()
        .find(|(to, _)| *to == key)
        .map(|(_, from)| *from)
        .unwrap_or(key)
}

fn fwd_remap(key: i32) -> Option<i32> {
    let map = fwd_rebinds().lock();
    map.iter()
        .find(|(from, _)| *from == key)
        .map(|(_, to)| *to)
}

/// True if `key` is the *source* of an active rebind (it's been remapped away).
/// Such a key must read as released when the game polls it directly, otherwise
/// its raw physical state leaks through alongside the logical key it maps to.
pub fn is_rebind_source(key: i32) -> bool {
    fwd_remap(key).is_some()
}

// ── Codepoint ↔ GLFW Keycode Conversion ─────────────────────────────────
// The next 2 functions were organised by an LLM

// Maps a Unicode codepoint → (glfw_key, shifted).
fn cp_to_glfw(cp: u32) -> Option<(i32, bool)> {
    match cp {
        // Lowercase a–z → GLFW_KEY_A–Z (65–90)
        97..=122 => Some(((cp - 32) as i32, false)),
        // Uppercase A–Z
        65..=90 => Some((cp as i32, true)),
        // Digits 0–9
        48..=57 => Some((cp as i32, false)),
        // Shifted digit-row symbols
        33 => Some((49, true)),  // !
        64 => Some((50, true)),  // @
        35 => Some((51, true)),  // #
        36 => Some((52, true)),  // $
        37 => Some((53, true)),  // %
        94 => Some((54, true)),  // ^
        38 => Some((55, true)),  // &
        42 => Some((56, true)),  // *
        40 => Some((57, true)),  // (
        41 => Some((48, true)),  // )
        // Punctuation (unshifted)
        32 => Some((32, false)),   // space
        39 => Some((39, false)),   // '
        44 => Some((44, false)),   // ,
        45 => Some((45, false)),   // -
        46 => Some((46, false)),   // .
        47 => Some((47, false)),   // /
        59 => Some((59, false)),   // ;
        61 => Some((61, false)),   // =
        91 => Some((91, false)),   // [
        92 => Some((92, false)),   // backslash
        93 => Some((93, false)),   // ]
        96 => Some((96, false)),   // `
        // Punctuation (shifted counterparts)
        34 => Some((39, true)),    // "
        60 => Some((44, true)),    // <
        95 => Some((45, true)),    // _
        62 => Some((46, true)),    // >
        63 => Some((47, true)),    // ?
        58 => Some((59, true)),    // :
        43 => Some((61, true)),    // +
        123 => Some((91, true)),   // {
        124 => Some((92, true)),   // |
        125 => Some((93, true)),   // }
        126 => Some((96, true)),   // ~
        _ => None,
    }
}

// GLFW keycode → Unicode codepoint.
fn glfw_to_cp(key: i32, shifted: bool) -> Option<u32> {
    match key {
        65..=90 => Some(if shifted { key as u32 } else { (key + 32) as u32 }),
        48..=57 => {
            if shifted {
                match key {
                    49 => Some(33),  // !
                    50 => Some(64),  // @
                    51 => Some(35),  // #
                    52 => Some(36),  // $
                    53 => Some(37),  // %
                    54 => Some(94),  // ^
                    55 => Some(38),  // &
                    56 => Some(42),  // *
                    57 => Some(40),  // (
                    48 => Some(41),  // )
                    _ => None,
                }
            } else {
                Some(key as u32)
            }
        }
        32 => Some(32),
        39 => Some(if shifted { 34 } else { 39 }),   // ' / "
        44 => Some(if shifted { 60 } else { 44 }),   // , / <
        45 => Some(if shifted { 95 } else { 45 }),   // - / _
        46 => Some(if shifted { 62 } else { 46 }),   // . / >
        47 => Some(if shifted { 63 } else { 47 }),   // / / ?
        59 => Some(if shifted { 58 } else { 59 }),   // ; / :
        61 => Some(if shifted { 43 } else { 61 }),   // = / +
        91 => Some(if shifted { 123 } else { 91 }),  // [ / {
        92 => Some(if shifted { 124 } else { 92 }),  // \ / |
        93 => Some(if shifted { 125 } else { 93 }),  // ] / }
        96 => Some(if shifted { 126 } else { 96 }),  // ` / ~
        _ => None,  // non-printable (F-keys, arrows, modifiers, etc)
    }
}

// remap a codepoint through forward rebinds. returns 0 to suppress.
fn remap_cp(codepoint: u32) -> u32 {
    let (from_key, shifted) = match cp_to_glfw(codepoint) {
        Some(pair) => pair,
        None => return codepoint, // non-ASCII or unmapped, pass through
    };

    let to_key = match fwd_remap(from_key) {
        Some(k) => k,
        None => return codepoint, // no rebind
    };

    // convert target back to codepoint, suppress if it has no char representation
    glfw_to_cp(to_key, shifted).unwrap_or(0)
}

// --- cursor capture state ---

use std::sync::atomic::AtomicBool;

// true when cursor is GLFW_CURSOR_DISABLED (FPS mode)
static CURSOR_CAPTURED: AtomicBool = AtomicBool::new(false);

// set on capture transition so sensitivity can reset
static CURSOR_RECAPTURED: AtomicBool = AtomicBool::new(false);

// true when we forced cursor to NORMAL for the GUI
static GUI_FORCED_CURSOR: AtomicBool = AtomicBool::new(false);

static GLFW_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

pub fn is_cursor_captured() -> bool {
    CURSOR_CAPTURED.load(Ordering::Relaxed)
}

pub fn take_cursor_recaptured() -> bool {
    CURSOR_RECAPTURED.swap(false, Ordering::Relaxed)
}

/// Force cursor visible so the GUI overlay can be used
pub unsafe fn force_cursor_visible() {
    use crate::glfw_types::{GLFW_CURSOR, GLFW_CURSOR_NORMAL};

    if !CURSOR_CAPTURED.load(Ordering::Relaxed) {
        return; // already visible
    }

    let win = GLFW_WINDOW.load(Ordering::Acquire);
    let ptr = REAL_SET_INPUT_MODE.load(Ordering::Acquire);
    if win.is_null() || ptr.is_null() {
        return;
    }

    let real_fn: crate::glfw_types::GlfwSetInputModeFn = std::mem::transmute(ptr);
    real_fn(win, GLFW_CURSOR, GLFW_CURSOR_NORMAL);
    GUI_FORCED_CURSOR.store(true, Ordering::Relaxed);
    debug!("force_cursor_visible: cursor set to NORMAL for GUI");
}

/// Give cursor back to the game after GUI closes
pub unsafe fn restore_game_cursor() {
    use crate::glfw_types::{GLFW_CURSOR, GLFW_CURSOR_DISABLED};

    if !GUI_FORCED_CURSOR.swap(false, Ordering::Relaxed) {
        return; // wasn't forced by us
    }

    let win = GLFW_WINDOW.load(Ordering::Acquire);
    let ptr = REAL_SET_INPUT_MODE.load(Ordering::Acquire);
    if win.is_null() || ptr.is_null() {
        return;
    }

    let real_fn: crate::glfw_types::GlfwSetInputModeFn = std::mem::transmute(ptr);
    real_fn(win, GLFW_CURSOR, GLFW_CURSOR_DISABLED);
    // re-flag and signal recapture so sensitivity resets
    CURSOR_CAPTURED.store(true, Ordering::Relaxed);
    CURSOR_RECAPTURED.store(true, Ordering::Relaxed);
    RECENTER_AFTER_GUI.store(true, Ordering::Relaxed);
    debug!("restore_game_cursor: cursor set back to DISABLED; armed GUI-ICM recenter");
}

// Set when the tux GUI hands the cursor back to the game; consumed on the next
// menu open to recenter once (GUI-ICM prevention).
static RECENTER_AFTER_GUI: AtomicBool = AtomicBool::new(false);

pub fn glfw_window_handle() -> crate::glfw_types::GlfwWindow {
    GLFW_WINDOW.load(Ordering::Acquire)
}

/// Current logical (screen-coordinate) window size via the real glfwGetWindowSize.
/// On Wayland this is the surface-local size, i.e. the space the locked-pointer
/// cursor-position hint lives in. Returns None until the real fn / window are known.
pub fn window_logical_size() -> Option<(i32, i32)> {
    let win = glfw_window_handle();
    let ptr = REAL_GET_WINDOW_SIZE.load(Ordering::Acquire);
    if win.is_null() || ptr.is_null() {
        return None;
    }
    type GetWinSizeFn = unsafe extern "C" fn(GlfwWindow, *mut i32, *mut i32);
    let f: GetWinSizeFn = unsafe { std::mem::transmute(ptr) };
    let (mut w, mut h) = (0i32, 0i32);
    unsafe { f(win, &mut w, &mut h) };
    if w > 0 && h > 0 { Some((w, h)) } else { None }
}

// --- GUI input state ---

static GUI_VISIBLE: AtomicBool = AtomicBool::new(false);
static GUI_WANTS_KB: AtomicBool = AtomicBool::new(false);
static GUI_BTN_PRESSED: AtomicBool = AtomicBool::new(false);
static GUI_BTN_RELEASED: AtomicBool = AtomicBool::new(false);
static GUI_RBTN_PRESSED: AtomicBool = AtomicBool::new(false);
static GUI_RBTN_RELEASED: AtomicBool = AtomicBool::new(false);
static GUI_BTN_MODS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static GUI_SCROLL: Mutex<(f32, f32)> = Mutex::new((0.0, 0.0));

pub fn set_gui_visible(visible: bool) {
    GUI_VISIBLE.store(visible, Ordering::Relaxed);
    // drop tracked key-repeat holds so they don't resume firing after the GUI
    // closes (the release may have gone to imgui while the GUI was open)
    if visible {
        kr_clear();
        // and drop any block-from-game holds: their release may go to imgui, so
        // they'd otherwise stay stuck "released" for the game after the GUI shuts
        clear_blocked_from_game();
    }
}

pub fn gui_is_visible() -> bool {
    GUI_VISIBLE.load(Ordering::Relaxed)
}

pub fn set_gui_wants_keyboard(wants: bool) {
    GUI_WANTS_KB.store(wants, Ordering::Relaxed);
}

/// true when an imgui text field has focus
pub fn gui_wants_keyboard() -> bool {
    GUI_WANTS_KB.load(Ordering::Relaxed)
}

pub fn push_gui_button_press() {
    GUI_BTN_PRESSED.store(true, Ordering::Relaxed);
}

pub fn push_gui_button_release() {
    GUI_BTN_RELEASED.store(true, Ordering::Relaxed);
}

pub fn push_gui_rbutton_press() {
    GUI_RBTN_PRESSED.store(true, Ordering::Relaxed);
}

pub fn push_gui_rbutton_release() {
    GUI_RBTN_RELEASED.store(true, Ordering::Relaxed);
}

pub fn push_gui_button_mods(mods: i32) {
    GUI_BTN_MODS.store(mods, Ordering::Relaxed);
}

pub fn take_gui_button_mods() -> i32 {
    GUI_BTN_MODS.swap(0, Ordering::Relaxed)
}

pub fn take_gui_button_press() -> bool {
    GUI_BTN_PRESSED.swap(false, Ordering::Relaxed)
}

pub fn take_gui_button_release() -> bool {
    GUI_BTN_RELEASED.swap(false, Ordering::Relaxed)
}

pub fn take_gui_rbutton_press() -> bool {
    GUI_RBTN_PRESSED.swap(false, Ordering::Relaxed)
}

pub fn take_gui_rbutton_release() -> bool {
    GUI_RBTN_RELEASED.swap(false, Ordering::Relaxed)
}

// --- key capture mode (for the hotkey picker in settings) ---

static GUI_CAPTURE_MODE: AtomicBool = AtomicBool::new(false);
static GUI_CAPTURED_KEY: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(i32::MIN);

pub fn set_gui_capture_mode(enabled: bool) {
    GUI_CAPTURE_MODE.store(enabled, Ordering::Relaxed);
    if !enabled {
        GUI_CAPTURED_KEY.store(i32::MIN, Ordering::Relaxed);
    }
}

pub fn is_gui_capture_mode() -> bool {
    GUI_CAPTURE_MODE.load(Ordering::Relaxed)
}

pub fn push_captured_key(keycode: i32) {
    GUI_CAPTURED_KEY.store(keycode, Ordering::Relaxed);
}

pub fn take_captured_key() -> Option<i32> {
    let v = GUI_CAPTURED_KEY.swap(i32::MIN, Ordering::Relaxed);
    if v == i32::MIN { None } else { Some(v) }
}

pub fn push_gui_scroll(dx: f32, dy: f32) {
    let mut g = GUI_SCROLL.lock();
    g.0 += dx;
    g.1 += dy;
}

pub fn take_gui_scroll() -> (f32, f32) {
    let mut g = GUI_SCROLL.lock();
    let val = *g;
    *g = (0.0, 0.0);
    val
}

// --- GUI key and text queues ---

// (glfw_key, glfw_mods, pressed)
static GUI_KEY_QUEUE: Mutex<Vec<(i32, i32, bool)>> = Mutex::new(Vec::new());
static GUI_CHAR_QUEUE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub fn push_gui_key(key: i32, mods: i32, pressed: bool) {
    GUI_KEY_QUEUE.lock().push((key, mods, pressed));
}

pub fn take_gui_keys() -> Vec<(i32, i32, bool)> {
    let mut q = GUI_KEY_QUEUE.lock();
    q.drain(..).collect()
}

pub fn push_gui_char(codepoint: u32) {
    GUI_CHAR_QUEUE.lock().push(codepoint);
}

pub fn take_gui_text() -> String {
    let mut q = GUI_CHAR_QUEUE.lock();
    q.drain(..).filter_map(char::from_u32).collect()
}

// --- char callback interception ---

static GAME_CHAR_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_SET_CHAR_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

pub fn store_real_set_char_callback(ptr: *mut c_void) {
    debug!("storing real glfwSetCharCallback at {:?}", ptr);
    REAL_SET_CHAR_CB.store(ptr, Ordering::Release);
}

// captures text for imgui when GUI is open, applies rebinds otherwise
pub unsafe extern "C" fn tuxinjector_char_callback(window: GlfwWindow, codepoint: u32) {
    if GUI_VISIBLE.load(Ordering::Relaxed) {
        // always pass chars to imgui - capture mode only needs the key callback,
        // not the char callback. blocking chars here breaks text field input.
        GUI_CHAR_QUEUE.lock().push(codepoint);
        return;
    }

    let remapped = remap_cp(codepoint);
    if remapped == 0 {
        return; // rebind suppressed this char
    }

    let ptr = GAME_CHAR_CB.load(Ordering::Acquire);
    if !ptr.is_null() {
        let cb: GlfwCharCallback = std::mem::transmute(ptr);
        if let Some(f) = cb {
            f(window, remapped);
        }
    }
}

pub unsafe fn intercept_set_char_callback(
    window: GlfwWindow,
    callback: GlfwCharCallback,
) -> GlfwCharCallback {
    let game_ptr: *mut c_void = std::mem::transmute(callback);
    let old = GAME_CHAR_CB.swap(game_ptr, Ordering::AcqRel);
    let old_cb: GlfwCharCallback = std::mem::transmute(old);

    debug!("intercepted glfwSetCharCallback: game={:?}", game_ptr);

    let real_ptr = REAL_SET_CHAR_CB.load(Ordering::Acquire);
    if !real_ptr.is_null() {
        let real_fn: crate::glfw_types::GlfwSetCharCallbackFn = std::mem::transmute(real_ptr);
        real_fn(window, Some(tuxinjector_char_callback));
    }

    old_cb
}

// --- char_mods callback interception ---

static GAME_CHAR_MODS_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static REAL_SET_CHAR_MODS_CB: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

pub fn store_real_set_char_mods_callback(ptr: *mut c_void) {
    debug!("storing real glfwSetCharModsCallback at {:?}", ptr);
    REAL_SET_CHAR_MODS_CB.store(ptr, Ordering::Release);
}

pub unsafe extern "C" fn tuxinjector_char_mods_callback(
    window: GlfwWindow,
    codepoint: u32,
    mods: i32,
) {
    if GUI_VISIBLE.load(Ordering::Relaxed) {
        GUI_CHAR_QUEUE.lock().push(codepoint);
        return;
    }

    let remapped = remap_cp(codepoint);
    if remapped == 0 {
        return;
    }

    let ptr = GAME_CHAR_MODS_CB.load(Ordering::Acquire);
    if !ptr.is_null() {
        let cb: GlfwCharModsCallback = std::mem::transmute(ptr);
        if let Some(f) = cb {
            f(window, remapped, mods);
        }
    }
}

pub unsafe fn intercept_set_char_mods_callback(
    window: GlfwWindow,
    callback: GlfwCharModsCallback,
) -> GlfwCharModsCallback {
    let game_ptr: *mut c_void = std::mem::transmute(callback);
    let old = GAME_CHAR_MODS_CB.swap(game_ptr, Ordering::AcqRel);
    let old_cb: GlfwCharModsCallback = std::mem::transmute(old);

    debug!(
        "intercepted glfwSetCharModsCallback: game={:?}",
        game_ptr
    );

    let real_ptr = REAL_SET_CHAR_MODS_CB.load(Ordering::Acquire);
    if !real_ptr.is_null() {
        let real_fn: crate::glfw_types::GlfwSetCharModsCallbackFn =
            std::mem::transmute(real_ptr);
        real_fn(window, Some(tuxinjector_char_mods_callback));
    }

    old_cb
}

// --- InputHandler trait ---

/// Trait for processing intercepted input before it hits the game
pub trait InputHandler: Send {
    /// Returns (consumed, forward_key)
    fn handle_key(&mut self, key: i32, scancode: i32, action: i32, mods: i32) -> (bool, i32);

    /// Returns (consumed, forward_button)
    fn handle_mouse_button(&mut self, button: i32, action: i32, mods: i32) -> (bool, i32);

    /// None = consume, Some((x,y)) = forward to game
    fn handle_cursor_pos(&mut self, x: f64, y: f64) -> Option<(f64, f64)>;

    /// true = consume scroll event
    fn handle_scroll(&mut self, x: f64, y: f64) -> bool;

    fn set_mode_sensitivity(&mut self, _s: f32, _separate: Option<(f32, f32)>) {}

    fn clear_mode_sensitivity(&mut self) {}
}

static INPUT_HANDLER: Mutex<Option<Box<dyn InputHandler + Send>>> = Mutex::new(None);

// --- wrapper callbacks ---

pub unsafe extern "C" fn tuxinjector_key_callback(
    window: GlfwWindow,
    key: i32,
    scancode: i32,
    action: i32,
    mods: i32,
) {
    // HACK: Raw event trace -- enable `tuxinjector_input=trace` to surface this.
    // Useful when diagnosing chord bugs (e.g. F3+C): confirms the OS is
    // actually delivering a keydown for both keys while held together.
    trace!(key, scancode, action, mods, "[KEY] raw event");

    let (consumed, proposed_fwd_key) = {
        let mut guard = INPUT_HANDLER.lock();
        if let Some(ref mut handler) = *guard {
            handler.handle_key(key, scancode, action, mods)
        } else {
            (false, key)
        }
    };

    use crate::glfw_types::{MOUSE_BUTTON_OFFSET, GLFW_PRESS, GLFW_RELEASE, GLFW_REPEAT};

    // Sticky-key guard: the rebinder may pick different logical keys for
    // the same physical key if game state changes between PRESS and RELEASE.
    // Remember the PRESS mapping and force RELEASE + REPEAT to use it.
    let fwd_key = match action {
        GLFW_PRESS => {
            record_press_mapping(key, proposed_fwd_key);
            proposed_fwd_key
        }
        GLFW_RELEASE => {
            // Consume the mapping so a future PRESS without an earlier
            // matching release (e.g. key sticks physically) gets a fresh
            // mapping.
            consume_press_mapping(key).unwrap_or(proposed_fwd_key)
        }
        GLFW_REPEAT => {
            peek_press_mapping(key).unwrap_or(proposed_fwd_key)
        }
        _ => proposed_fwd_key,
    };

    if fwd_key < MOUSE_BUTTON_OFFSET {
        track_virtual_key(key, fwd_key, action);
    }

    // A "block key from game" hotkey owns this key for the whole hold: the
    // engine consumes the PRESS, so also swallow the following REPEATs and the
    // RELEASE toward the game (and glfwGetKey reports it released). Without this
    // the key leaks via OS repeat + state polling even though the press fired.
    let block_from_game = match action {
        GLFW_PRESS => {
            if consumed { mark_blocked_from_game(fwd_key); }
            consumed
        }
        GLFW_REPEAT => is_blocked_from_game(fwd_key),
        GLFW_RELEASE => unmark_blocked_from_game(fwd_key),
        _ => false,
    };

    if !consumed && !block_from_game {
        if fwd_key >= MOUSE_BUTTON_OFFSET {
            // key was remapped to a mouse button
            fwd_mouse_btn(window, fwd_key - MOUSE_BUTTON_OFFSET, action, mods);

            // GLFW doesn't always emit modifier events on the same frame as
            // mouse clicks, so forward the modifier key too
            if is_modifier(key) {
                fwd_key_fn(window, key, scancode, action, mods);
            }
        } else {
            let fwd_scancode = if fwd_key != key {
                canonical_scancode(fwd_key).unwrap_or(scancode)
            } else {
                scancode
            };
            // Per-key custom repeat: track holds (keyed by physical key) and
            // swallow this key's OS repeats so tick_key_repeat() re-emits at the
            // key's configured rate. Keys without custom repeat pass through.
            match action {
                GLFW_PRESS => kr_note_press(key, fwd_key, fwd_scancode, mods),
                GLFW_RELEASE => kr_note_release(key),
                _ => {}
            }
            if action == GLFW_REPEAT && key_repeat_enabled_for(key) {
                // swallowed; tick_key_repeat() drives this key's repeats instead
            } else {
                fwd_key_fn(window, fwd_key, fwd_scancode, action, mods);
            }
        }

        // when a non-char key gets rebound to a char key, inject the char event
        // so the game's text input still works
        if action == GLFW_PRESS && fwd_key != key {
            let orig_printable = glfw_to_cp(key, false).is_some();
            if !orig_printable {
                let shifted = (mods & GLFW_MOD_SHIFT) != 0;
                if let Some(cp) = glfw_to_cp(fwd_key, shifted) {
                    fwd_char(window, cp, mods);
                }
            }
        }
    }
}

fn is_modifier(key: i32) -> bool {
    use crate::glfw_types::*;
    matches!(key,
        GLFW_KEY_LEFT_SHIFT | GLFW_KEY_RIGHT_SHIFT |
        GLFW_KEY_LEFT_CONTROL | GLFW_KEY_RIGHT_CONTROL |
        GLFW_KEY_LEFT_ALT | GLFW_KEY_RIGHT_ALT |
        GLFW_KEY_LEFT_SUPER | GLFW_KEY_RIGHT_SUPER
    )
}

pub unsafe extern "C" fn tuxinjector_mouse_button_callback(
    window: GlfwWindow,
    button: i32,
    action: i32,
    mods: i32,
) {
    use crate::glfw_types::MOUSE_BUTTON_OFFSET;
    trace!(button, action, mods, "mouse button event");

    let (consumed, fwd_btn) = {
        let mut guard = INPUT_HANDLER.lock();
        if let Some(ref mut handler) = *guard {
            handler.handle_mouse_button(button, action, mods)
        } else {
            (false, button)
        }
    };

    if !consumed {
        if fwd_btn >= MOUSE_BUTTON_OFFSET {
            fwd_mouse_btn(window, fwd_btn - MOUSE_BUTTON_OFFSET, action, mods);
        } else if fwd_btn != button {
            // remapped to a keyboard key
            fwd_key_fn(window, fwd_btn, 0, action, mods);
        } else {
            fwd_mouse_btn(window, button, action, mods);
        }
    }
}

pub unsafe extern "C" fn tuxinjector_cursor_pos_callback(
    window: GlfwWindow,
    xpos: f64,
    ypos: f64,
) {
    let captured = CURSOR_CAPTURED.load(Ordering::Relaxed);
    trace!(xpos, ypos, cursor_captured = captured, "cursor pos event");

    // stash raw position for GUI mouse input
    RAW_MOUSE_X.store(xpos.to_bits(), Ordering::Relaxed);
    RAW_MOUSE_Y.store(ypos.to_bits(), Ordering::Relaxed);

    let result = {
        let mut guard = INPUT_HANDLER.lock();
        if let Some(ref mut handler) = *guard {
            handler.handle_cursor_pos(xpos, ypos)
        } else {
            Some((xpos, ypos))
        }
    };

    if let Some((x, y)) = result {
        // store scaled position for fake cursor alignment
        MOUSE_X.store(x.to_bits(), Ordering::Relaxed);
        MOUSE_Y.store(y.to_bits(), Ordering::Relaxed);
        fwd_cursor(window, x, y);
    }
}

pub unsafe extern "C" fn tuxinjector_scroll_callback(
    window: GlfwWindow,
    xoffset: f64,
    yoffset: f64,
) {
    trace!(xoffset, yoffset, "scroll event");

    let consumed = {
        let mut guard = INPUT_HANDLER.lock();
        if let Some(ref mut handler) = *guard {
            handler.handle_scroll(xoffset, yoffset)
        } else {
            false
        }
    };

    if !consumed {
        fwd_scroll(window, xoffset, yoffset);
    }
}

// --- forwarding helpers ---
// these call into the game's stashed callbacks

unsafe fn fwd_key(
    window: GlfwWindow,
    key: i32,
    scancode: i32,
    action: i32,
    mods: i32,
) {
    let ptr = GAME_KEY_CB.load(Ordering::Acquire);
    if !ptr.is_null() {
        let cb: GlfwKeyCallback = std::mem::transmute(ptr);
        if let Some(f) = cb {
            f(window, key, scancode, action, mods);
        }
    }
}

// NOTE: same as fwd_key but used from the wrapper callback path
// to keep naming consistent with the rest of the forwarding fns
unsafe fn fwd_key_fn(
    window: GlfwWindow,
    key: i32,
    scancode: i32,
    action: i32,
    mods: i32,
) {
    let ptr = GAME_KEY_CB.load(Ordering::Acquire);
    if !ptr.is_null() {
        let cb: GlfwKeyCallback = std::mem::transmute(ptr);
        if let Some(f) = cb {
            f(window, key, scancode, action, mods);
        }
    }
}

unsafe fn fwd_mouse_btn(
    window: GlfwWindow,
    button: i32,
    action: i32,
    mods: i32,
) {
    let ptr = GAME_MOUSE_BTN_CB.load(Ordering::Acquire);
    if !ptr.is_null() {
        let cb: GlfwMouseButtonCallback = std::mem::transmute(ptr);
        if let Some(f) = cb {
            f(window, button, action, mods);
        }
    }
}

// inject a char event into both char and char_mods callbacks
unsafe fn fwd_char(window: GlfwWindow, codepoint: u32, mods: i32) {
    let p1 = GAME_CHAR_CB.load(Ordering::Acquire);
    if !p1.is_null() {
        let cb: GlfwCharCallback = std::mem::transmute(p1);
        if let Some(f) = cb {
            f(window, codepoint);
        }
    }
    let p2 = GAME_CHAR_MODS_CB.load(Ordering::Acquire);
    if !p2.is_null() {
        let cb: GlfwCharModsCallback = std::mem::transmute(p2);
        if let Some(f) = cb {
            f(window, codepoint, mods);
        }
    }
}

unsafe fn fwd_cursor(window: GlfwWindow, x: f64, y: f64) {
    let ptr = GAME_CURSOR_CB.load(Ordering::Acquire);
    if !ptr.is_null() {
        let cb: GlfwCursorPosCallback = std::mem::transmute(ptr);
        if let Some(f) = cb {
            f(window, x, y);
        }
    }
}

unsafe fn fwd_scroll(window: GlfwWindow, x: f64, y: f64) {
    let ptr = GAME_SCROLL_CB.load(Ordering::Acquire);
    if !ptr.is_null() {
        let cb: GlfwScrollCallback = std::mem::transmute(ptr);
        if let Some(f) = cb {
            f(window, x, y);
        }
    }
}

// --- public API for the dlsym hook layer ---

pub fn store_real_set_key_callback(ptr: *mut c_void) {
    debug!("storing real glfwSetKeyCallback at {:?}", ptr);
    REAL_SET_KEY_CB.store(ptr, Ordering::Release);
}

pub fn store_real_set_mouse_button_callback(ptr: *mut c_void) {
    debug!("storing real glfwSetMouseButtonCallback at {:?}", ptr);
    REAL_SET_MOUSE_BTN_CB.store(ptr, Ordering::Release);
}

pub fn store_real_set_cursor_pos_callback(ptr: *mut c_void) {
    debug!("storing real glfwSetCursorPosCallback at {:?}", ptr);
    REAL_SET_CURSOR_CB.store(ptr, Ordering::Release);
}

pub fn store_real_set_scroll_callback(ptr: *mut c_void) {
    debug!("storing real glfwSetScrollCallback at {:?}", ptr);
    REAL_SET_SCROLL_CB.store(ptr, Ordering::Release);
}

pub fn store_real_set_input_mode(ptr: *mut c_void) {
    debug!("storing real glfwSetInputMode at {:?}", ptr);
    REAL_SET_INPUT_MODE.store(ptr, Ordering::Release);
}

/// Intercepts glfwSetInputMode to track cursor capture state
pub unsafe fn intercept_set_input_mode(window: GlfwWindow, mode: i32, value: i32) {
    use crate::glfw_types::{GLFW_CURSOR, GLFW_CURSOR_DISABLED};

    debug!(mode, value, "glfwSetInputMode intercepted");

    GLFW_WINDOW.store(window, Ordering::Release);

    let mut should_center_after = false;

    if mode == GLFW_CURSOR {
        let captured = value == GLFW_CURSOR_DISABLED;
        let was = CURSOR_CAPTURED.swap(captured, Ordering::Relaxed);
        // signal sensitivity reset on fresh capture
        if captured && !was {
            CURSOR_RECAPTURED.store(true, Ordering::Relaxed);
        }
        debug!(value, captured, was_captured = was, "glfwSetInputMode: cursor capture state updated");

        // don't let the game re-capture while we're forcing cursor visible for the GUI
        if GUI_FORCED_CURSOR.load(Ordering::Relaxed) && captured {
            debug!("glfwSetInputMode: blocked CURSOR_DISABLED while GUI cursor is forced");
            return;
        }

        if was && !captured {
            should_center_after = RECENTER_AFTER_GUI.swap(false, Ordering::Relaxed);
        }
    }

    let real_ptr = REAL_SET_INPUT_MODE.load(Ordering::Acquire);
    if real_ptr.is_null() {
        tracing::warn!(mode, value, "glfwSetInputMode: real function pointer is null -- call dropped!");
    } else {
        let real_fn: crate::glfw_types::GlfwSetInputModeFn = std::mem::transmute(real_ptr);
        real_fn(window, mode, value);
    }

    // Recenter only when armed by restore_game_cursor (GUI-ICM prevention).
    if should_center_after {
        center_cursor(window);
    }
}

pub unsafe fn intercept_set_key_callback(
    window: GlfwWindow,
    callback: GlfwKeyCallback,
) -> GlfwKeyCallback {
    let game_ptr: *mut c_void = std::mem::transmute(callback);
    let old = GAME_KEY_CB.swap(game_ptr, Ordering::AcqRel);
    let old_cb: GlfwKeyCallback = std::mem::transmute(old);

    debug!("intercepted glfwSetKeyCallback: game={:?}", game_ptr);

    let real_ptr = REAL_SET_KEY_CB.load(Ordering::Acquire);
    if !real_ptr.is_null() {
        let real_fn: GlfwSetKeyCallbackFn = std::mem::transmute(real_ptr);
        real_fn(window, Some(tuxinjector_key_callback));
    }

    old_cb
}

pub unsafe fn intercept_set_mouse_button_callback(
    window: GlfwWindow,
    callback: GlfwMouseButtonCallback,
) -> GlfwMouseButtonCallback {
    let game_ptr: *mut c_void = std::mem::transmute(callback);
    let old = GAME_MOUSE_BTN_CB.swap(game_ptr, Ordering::AcqRel);
    let old_cb: GlfwMouseButtonCallback = std::mem::transmute(old);

    debug!(
        "intercepted glfwSetMouseButtonCallback: game={:?}",
        game_ptr
    );

    let real_ptr = REAL_SET_MOUSE_BTN_CB.load(Ordering::Acquire);
    if !real_ptr.is_null() {
        let real_fn: GlfwSetMouseButtonCallbackFn = std::mem::transmute(real_ptr);
        real_fn(window, Some(tuxinjector_mouse_button_callback));
    }

    old_cb
}

pub unsafe fn intercept_set_cursor_pos_callback(
    window: GlfwWindow,
    callback: GlfwCursorPosCallback,
) -> GlfwCursorPosCallback {
    let game_ptr: *mut c_void = std::mem::transmute(callback);
    let old = GAME_CURSOR_CB.swap(game_ptr, Ordering::AcqRel);
    let old_cb: GlfwCursorPosCallback = std::mem::transmute(old);

    debug!(
        "intercepted glfwSetCursorPosCallback: game={:?}",
        game_ptr
    );

    let real_ptr = REAL_SET_CURSOR_CB.load(Ordering::Acquire);
    if !real_ptr.is_null() {
        let real_fn: GlfwSetCursorPosCallbackFn = std::mem::transmute(real_ptr);
        real_fn(window, Some(tuxinjector_cursor_pos_callback));
    }

    old_cb
}

pub unsafe fn intercept_set_scroll_callback(
    window: GlfwWindow,
    callback: GlfwScrollCallback,
) -> GlfwScrollCallback {
    let game_ptr: *mut c_void = std::mem::transmute(callback);
    let old = GAME_SCROLL_CB.swap(game_ptr, Ordering::AcqRel);
    let old_cb: GlfwScrollCallback = std::mem::transmute(old);

    debug!(
        "intercepted glfwSetScrollCallback: game={:?}",
        game_ptr
    );

    let real_ptr = REAL_SET_SCROLL_CB.load(Ordering::Acquire);
    if !real_ptr.is_null() {
        let real_fn: GlfwSetScrollCallbackFn = std::mem::transmute(real_ptr);
        real_fn(window, Some(tuxinjector_scroll_callback));
    }

    old_cb
}

// --- handler registration ---

/// Register the input handler. Called once during init.
pub fn register_input_handler(handler: Box<dyn InputHandler + Send>) {
    debug!("registering input handler");
    *INPUT_HANDLER.lock() = Some(handler);
}

pub fn unregister_input_handler() {
    debug!("unregistering input handler");
    *INPUT_HANDLER.lock() = None;
}

pub fn set_mode_sensitivity(s: f32, separate: Option<(f32, f32)>) {
    if let Some(ref mut handler) = *INPUT_HANDLER.lock() {
        handler.set_mode_sensitivity(s, separate);
    }
}

pub fn clear_mode_sensitivity() {
    if let Some(ref mut handler) = *INPUT_HANDLER.lock() {
        handler.clear_mode_sensitivity();
    }
}
