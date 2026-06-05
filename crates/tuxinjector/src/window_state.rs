// Window title interception -- derives game state from Minecraft title changes.

use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicPtr, Ordering};

type SetWindowTitleFn = unsafe extern "C" fn(window: *mut c_void, title: *const c_char);

static REAL_FN: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn store_real_set_window_title(ptr: *mut c_void) {
    REAL_FN.store(ptr, Ordering::Release);
}

// "singleplayer" or "multiplayer" in the title -> ingame, otherwise menu
fn title_to_state(title: &str) -> &'static str {
    if title.is_empty() {
        return "";
    }
    let lower = title.to_lowercase();
    if lower.contains("singleplayer") || lower.contains("multiplayer") {
        "ingame"
    } else {
        "menu"
    }
}

pub unsafe extern "C" fn hooked_glfw_set_window_title(
    window: *mut c_void,
    title: *const c_char,
) {
    if !title.is_null() {
        if let Ok(s) = CStr::from_ptr(title).to_str() {
            let state = title_to_state(s);

            // The window title is only a coarse fallback. If a more specific
            // source (wpstateout via state_watcher) is active, it owns the state
            // and we must NOT clobber it — otherwise a title change would flip
            // "inworld,cursor_grabbed" back to "ingame" until the next poll,
            // breaking every state-conditioned hotkey for that window. Only act
            // while the current state is itself title-derived ("" / menu / ingame).
            let cur = tuxinjector_lua::get_game_state();
            let title_derived = matches!(cur.as_str(), "" | "menu" | "ingame");
            if title_derived && cur.as_str() != state {
                tracing::debug!(title = s, game_state = state, "game state changed (title)");
                if let Ok(mut guard) = crate::state::get().game_state.lock() {
                    *guard = state.to_string();
                }
                if tuxinjector_lua::update_game_state(state) {
                    let tx = crate::state::get();
                    if let Some(rt) = tx.lua_runtime.get() {
                        let _ = rt.state_event_tx.try_send(state.to_string());
                    }
                }
            }
        }
    }

    // forward to real GLFW
    let real = REAL_FN.load(Ordering::Acquire);
    if !real.is_null() {
        let f: SetWindowTitleFn = std::mem::transmute(real);
        f(window, title);
    }
}
