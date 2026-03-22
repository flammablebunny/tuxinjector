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

// --- key state tracking ---

static PRESSED_KEYS: Mutex<Option<std::collections::HashSet<i32>>> = Mutex::new(None);

fn track_key(key: i32, action: i32) {
    let mut guard = PRESSED_KEYS.lock();
    let set = guard.get_or_insert_with(std::collections::HashSet::new);
    if action == 1 /* PRESS */ {
        set.insert(key);
    } else if action == 0 /* RELEASE */ {
        set.remove(&key);
    }
}

pub fn is_key_pressed(key: i32) -> bool {
    let guard = PRESSED_KEYS.lock();
    guard.as_ref().map_or(false, |set| set.contains(&key))
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
    debug!("restore_game_cursor: cursor set back to DISABLED");
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
    trace!(key, scancode, action, mods, "key event");

    track_key(key, action);

    let (consumed, fwd_key) = {
        let mut guard = INPUT_HANDLER.lock();
        if let Some(ref mut handler) = *guard {
            handler.handle_key(key, scancode, action, mods)
        } else {
            (false, key)
        }
    };

    if !consumed {
        use crate::glfw_types::MOUSE_BUTTON_OFFSET;
        if fwd_key >= MOUSE_BUTTON_OFFSET {
            // key was remapped to a mouse button
            fwd_mouse_btn(window, fwd_key - MOUSE_BUTTON_OFFSET, action, mods);

            // GLFW doesn't always emit modifier events on the same frame as
            // mouse clicks, so forward the modifier key too
            if is_modifier(key) {
                fwd_key_fn(window, key, scancode, action, mods);
            }
        } else {
            fwd_key_fn(window, fwd_key, scancode, action, mods);
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
    }

    let real_ptr = REAL_SET_INPUT_MODE.load(Ordering::Acquire);
    if real_ptr.is_null() {
        tracing::warn!(mode, value, "glfwSetInputMode: real function pointer is null -- call dropped!");
    } else {
        let real_fn: crate::glfw_types::GlfwSetInputModeFn = std::mem::transmute(real_ptr);
        real_fn(window, mode, value);
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
