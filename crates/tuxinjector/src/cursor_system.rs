// Custom cursor state machine, ported from toolscreen's SetCursor hook
// (fake_cursor.cpp / HandleSetCursor). Each (name, size, hotspot) is a real
// GLFW cursor object; we pick one per game state (title / wall / else) and
// only call glfwSetCursor when the pick actually changes. GLFW hides the
// cursor on pointer-lock and re-applies it on release, so grab transitions
// take care of themselves.
//
// All of this lives on the game's GLFW/render thread (the swap hook) - that's
// the only thread where GLFW cursor calls are legal.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Mutex, OnceLock};

use tuxinjector_config::types::{Config, CursorConfig, CursorsConfig};

use crate::cursor_image;
use crate::glfw_hook::{self, GlfwImage};

// (name, size, hotspot fractions quantized to 1/1000)
type CursorKey = (String, i32, i32, i32);

struct SendPtr(*mut c_void);
unsafe impl Send for SendPtr {}

struct CursorSystem {
    // null entry = load/create failed; we keep it so we don't retry every frame
    cache: HashMap<CursorKey, SendPtr>,
    applied: Option<CursorKey>,
    // last cursor the game passed to glfwSetCursor, so we can hand it back
    game_cursor: SendPtr,
    // true while we own the window cursor
    active: bool,
    warned_no_fns: bool,
}

fn system() -> &'static Mutex<CursorSystem> {
    static S: OnceLock<Mutex<CursorSystem>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(CursorSystem {
            cache: HashMap::new(),
            applied: None,
            game_cursor: SendPtr(std::ptr::null_mut()),
            active: false,
            warned_no_fns: false,
        })
    })
}

/// Same bucketing as toolscreen's GetSelectedCursor: "title" -> title,
/// "wall" -> wall, everything else (waiting, generating, inworld,*) -> ingame.
/// When the pointer is grabbed the game hides it so we select nothing, unless
/// the tux GUI forced it back on. An empty cursorName means "don't touch the
/// real cursor" for that state.
pub fn select_cursor_state<'a>(
    cfg: &'a CursorsConfig,
    game_state: &str,
    captured: bool,
    gui_open: bool,
) -> Option<&'a CursorConfig> {
    if !cfg.enabled {
        return None;
    }
    if captured && !gui_open {
        return None;
    }
    let cc = match game_state {
        "title" => &cfg.title,
        "wall" => &cfg.wall,
        _ => &cfg.ingame,
    };
    (!cc.cursor_name.is_empty()).then_some(cc)
}

/// Called once per frame from the swap hook. When nothing changed this is
/// basically free - a couple atomic loads, a lock, and one key compare.
pub unsafe fn tick(cfg: &Config, gui_open: bool) {
    let window = tuxinjector_input::callbacks::glfw_window_handle();
    if window.is_null() {
        return;
    }
    let mut sys = system().lock().unwrap();

    if !cfg.theme.cursors.enabled {
        sys.restore(window);
        return;
    }

    let captured = tuxinjector_input::is_cursor_captured();
    if captured && !gui_open {
        // Pointer locked: the game hides the cursor; keep ours installed so
        // GLFW re-shows it the moment the game frees the pointer.
        return;
    }

    let gs = tuxinjector_lua::get_game_state();
    let cc = match select_cursor_state(&cfg.theme.cursors, &gs, captured, gui_open) {
        Some(cc) => cc,
        None => {
            // no cursor configured for this state -> real cursor through
            sys.restore(window);
            return;
        }
    };

    let size = cc.cursor_size.clamp(8, 256);
    let key: CursorKey = (
        cc.cursor_name.clone(),
        size,
        (cc.hotspot_x * 1000.0) as i32,
        (cc.hotspot_y * 1000.0) as i32,
    );

    if sys.active && sys.applied.as_ref() == Some(&key) {
        return;
    }

    let cursor = sys.ensure(&key, cc, size);
    if cursor.is_null() {
        return; // creation failed; leave whatever is installed
    }
    if glfw_hook::real_set_cursor(window, cursor) {
        sys.applied = Some(key);
        sys.active = true;
    }
}

/// Called from the glfwSetCursor interpose with the cursor the game wants.
/// Returns the cursor to actually forward to GLFW.
pub fn on_game_set_cursor(cursor: *mut c_void) -> *mut c_void {
    let mut sys = match system().lock() {
        Ok(s) => s,
        Err(_) => return cursor,
    };
    sys.game_cursor = SendPtr(cursor);
    if sys.active {
        if let Some(ptr) = sys.applied.as_ref().and_then(|k| sys.cache.get(k)) {
            if !ptr.0.is_null() {
                static O: std::sync::Once = std::sync::Once::new();
                O.call_once(|| {
                    tracing::debug!("game called glfwSetCursor; keeping custom cursor installed")
                });
                return ptr.0;
            }
        }
    }
    cursor
}

impl CursorSystem {
    /// Hand the window cursor back to the game and drop our cursor objects.
    /// Safe to call every frame while the feature is off (no-ops).
    unsafe fn restore(&mut self, window: *mut c_void) {
        if self.active {
            // usually null -> default arrow / whatever the game last wanted
            glfw_hook::real_set_cursor(window, self.game_cursor.0);
            self.active = false;
            self.applied = None;
        }
        if !self.cache.is_empty() {
            for (_, ptr) in self.cache.drain() {
                if !ptr.0.is_null() {
                    glfw_hook::real_destroy_cursor(ptr.0);
                }
            }
        }
    }

    unsafe fn ensure(&mut self, key: &CursorKey, cc: &CursorConfig, size: i32) -> *mut c_void {
        if let Some(p) = self.cache.get(key) {
            return p.0;
        }
        if !glfw_hook::cursor_fns_available() {
            if !self.warned_no_fns {
                self.warned_no_fns = true;
                tracing::warn!(
                    "glfwCreateCursor/glfwSetCursor not captured from the game's GLFW; \
                     custom cursors unavailable"
                );
            }
            return std::ptr::null_mut();
        }

        // Bound the cache (slider drags across many sizes); never destroy
        // the currently installed cursor.
        if self.cache.len() > 32 {
            let keep = self.applied.clone();
            let retained: Vec<CursorKey> = self
                .cache
                .keys()
                .filter(|k| Some(*k) != keep.as_ref())
                .cloned()
                .collect();
            for k in retained {
                if let Some(ptr) = self.cache.remove(&k) {
                    if !ptr.0.is_null() {
                        glfw_hook::real_destroy_cursor(ptr.0);
                    }
                }
            }
        }

        let img = cursor_image::resolve_cursor_path(&cc.cursor_name)
            .and_then(|p| cursor_image::load_cursor_image(&p))
            .map(|img| cursor_image::scale_to(img, size as u32))
            // configured-but-unloadable -> "always show something" fallback
            .unwrap_or_else(|| cursor_image::gen_crosshair(size as u32));

        let cursor = if img.rgba.len() == (img.w * img.h * 4) as usize && img.w > 0 {
            let (hx, hy) = img.file_hotspot.unwrap_or((
                (cc.hotspot_x.clamp(0.0, 1.0) * img.w as f32) as u32,
                (cc.hotspot_y.clamp(0.0, 1.0) * img.h as f32) as u32,
            ));
            let gi = GlfwImage {
                width: img.w as i32,
                height: img.h as i32,
                pixels: img.rgba.as_ptr(),
            };
            let ptr = glfw_hook::real_create_cursor(&gi, hx as i32, hy as i32);
            if ptr.is_null() {
                tracing::warn!(name = %cc.cursor_name, size, "glfwCreateCursor failed");
            } else {
                tracing::debug!(name = %cc.cursor_name, size, w = img.w, h = img.h, hx, hy,
                    "created glfw cursor");
            }
            ptr
        } else {
            std::ptr::null_mut()
        };

        self.cache.insert(key.clone(), SendPtr(cursor));
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(enabled: bool) -> CursorsConfig {
        CursorsConfig {
            enabled,
            title: CursorConfig { cursor_name: "t".into(), ..Default::default() },
            wall: CursorConfig { cursor_name: "w".into(), ..Default::default() },
            ingame: CursorConfig { cursor_name: "i".into(), ..Default::default() },
        }
    }

    #[test]
    fn disabled_selects_nothing() {
        assert!(select_cursor_state(&cfg(false), "title", false, false).is_none());
    }

    #[test]
    fn captured_hides_unless_gui_open() {
        let c = cfg(true);
        assert!(select_cursor_state(&c, "inworld,cursor_grabbed", true, false).is_none());
        let sel = select_cursor_state(&c, "inworld,cursor_grabbed", true, true).unwrap();
        assert_eq!(sel.cursor_name, "i");
    }

    #[test]
    fn toolscreen_bucketing() {
        let c = cfg(true);
        assert_eq!(select_cursor_state(&c, "title", false, false).unwrap().cursor_name, "t");
        assert_eq!(select_cursor_state(&c, "wall", false, false).unwrap().cursor_name, "w");
        // waiting/generating/inworld/unknown all fall into the ingame bucket
        for gs in ["waiting", "generating", "inworld,cursor_free", "inworld,cursor_grabbed", "", "junk"] {
            assert_eq!(select_cursor_state(&c, gs, false, false).unwrap().cursor_name, "i", "{gs}");
        }
    }

    #[test]
    fn empty_name_passes_through() {
        let mut c = cfg(true);
        c.title.cursor_name.clear();
        assert!(select_cursor_state(&c, "title", false, false).is_none());
        assert!(select_cursor_state(&c, "wall", false, false).is_some());
    }
}
