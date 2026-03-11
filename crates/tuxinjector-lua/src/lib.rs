// Lua config system -- evaluates init.lua, exposes the tuxinjector API
// for hotkey binds, runtime commands, etc.

pub mod actions;
pub mod api;
pub mod key_parse;
pub mod loader;
pub mod runtime;

pub use actions::{ActionBuilder, LuaActionBinding, TuxinjectorCommand};
pub use loader::{load_lua_config, load_lua_config_full, LuaConfigError, LuaLoadResult};
pub use key_parse::parse_key_combo;
pub use runtime::{LuaConfigUpdate, LuaRuntime};

// Shared game state between the render thread and the Lua VM.
// Mutexes are fine here since contention is basically zero.
static CURRENT_GAME_STATE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
static CURRENT_MODE_NAME: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

// Width and height packed into one u64 so we can read both atomically.
// Layout: (w << 32) | h
static ACTIVE_RES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// Returns true only when state actually changed, so we don't re-fire listeners
// on duplicate updates.
pub fn update_game_state(new: &str) -> bool {
    match CURRENT_GAME_STATE.lock() {
        Ok(mut g) => {
            if g.as_str() == new {
                return false;
            }
            *g = new.to_string();
            true
        }
        Err(_) => false,
    }
}

pub fn get_game_state() -> String {
    CURRENT_GAME_STATE
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn update_mode_name(id: &str) {
    if let Ok(mut g) = CURRENT_MODE_NAME.lock() {
        *g = id.to_string();
    }
}

pub fn get_mode_name() -> String {
    CURRENT_MODE_NAME
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

pub fn update_active_res(w: u32, h: u32) {
    let packed = ((w as u64) << 32) | (h as u64);
    ACTIVE_RES.store(packed, std::sync::atomic::Ordering::Release);
}

pub fn get_active_res() -> (u32, u32) {
    let packed = ACTIVE_RES.load(std::sync::atomic::Ordering::Acquire);
    let w = (packed >> 32) as u32;
    let h = (packed & 0xFFFFFFFF) as u32;
    (w, h)
}
