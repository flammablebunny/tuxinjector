// Key rebinding: translates GLFW keycodes before they enter the input pipeline.
// Supports separate game/chat targets so inventory screens can use different binds.

use tracing::debug;

use tuxinjector_config::types::KeyRebindsConfig;

struct RebindEntry {
    from: i32,
    to_game: i32, // 0 = no rebind in game mode (pass key through unchanged)
    to_chat: i32, // 0 = no rebind in chat/text mode (pass key through unchanged)
}

impl RebindEntry {
    fn target(&self, in_chat: bool) -> Option<i32> {
        let t = if in_chat { self.to_chat } else { self.to_game };
        if t != 0 { Some(t) } else { None }
    }
}

pub struct KeyRebinder {
    on: bool,
    entries: Vec<RebindEntry>,
    // Some(true)  = mod-provided state says cursor-free (chat/inventory/pause)
    // Some(false) = mod-provided state says cursor-grabbed (in-game)
    // None        = no mod state tag -> fall back to live GLFW cursor capture
    in_chat: Option<bool>,
}

impl KeyRebinder {
    pub fn new() -> Self {
        Self {
            on: false,
            entries: Vec::new(),
            in_chat: None,
        }
    }

    pub fn update_from_config(&mut self, config: &KeyRebindsConfig) {
        self.on = config.enabled;
        self.entries.clear();

        for r in &config.rebinds {
            if r.enabled && r.from_key != 0 && (r.to_key != 0 || r.to_key_chat != 0) {
                self.entries.push(RebindEntry {
                    from: r.from_key as i32,
                    to_game: r.to_key as i32,
                    to_chat: r.to_key_chat as i32,
                });
            }
        }

        debug!(
            enabled = self.on,
            count = self.entries.len(),
            "updated key rebinds"
        );
    }

    // returns true if the chat state actually changed
    pub fn set_game_state(&mut self, state: &str) -> bool {
        let new = if state.contains("cursor_free") {
            Some(true)
        } else if state.contains("cursor_grabbed") {
            Some(false)
        } else {
            None // no tag -> effective_in_chat falls back to live cursor state
        };
        if self.in_chat != new {
            self.in_chat = new;
            true
        } else {
            false
        }
    }

    /// Resolve the current "chat mode" flag. Mod-provided state (if any) is
    /// authoritative. Otherwise use the live GLFW cursor capture state: MC
    /// always calls glfwSetInputMode(GLFW_CURSOR, GLFW_CURSOR_NORMAL) when
    /// opening a text-input screen, so "cursor freed" = typing context.
    fn effective_in_chat(&self) -> bool {
        match self.in_chat {
            Some(chat) => chat,
            None => !crate::callbacks::is_cursor_captured(),
        }
    }

    pub fn remap_key(&self, key: i32, scancode: i32) -> i32 {
        if !self.on {
            return key;
        }
        let sc_key = tuxinjector_config::key_names::SCANCODE_OFFSET as i32 + scancode;
        let in_chat = self.effective_in_chat();
        self.entries
            .iter()
            .find(|e| e.from == key || (scancode > 0 && e.from == sc_key))
            .and_then(|e| e.target(in_chat))
            .unwrap_or(key)
    }

    // reverse lookup: find the physical key that maps to this logical key
    pub fn reverse_remap_key(&self, key: i32) -> i32 {
        if !self.on {
            return key;
        }
        let in_chat = self.effective_in_chat();
        self.entries
            .iter()
            .find(|e| e.target(in_chat) == Some(key))
            .map(|e| e.from)
            .unwrap_or(key)
    }

    pub fn is_enabled(&self) -> bool {
        self.on
    }

    // active (from, to) pairs for current state. empty when disabled
    pub fn active_rebinds(&self) -> Vec<(i32, i32)> {
        if self.on {
            let in_chat = self.effective_in_chat();
            self.entries
                .iter()
                .filter_map(|e| e.target(in_chat).map(|t| (e.from, t)))
                .collect()
        } else {
            Vec::new()
        }
    }
}

impl Default for KeyRebinder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuxinjector_config::types::{KeyRebind, KeyRebindsConfig};

    fn mk(from: i32, game: i32) -> RebindEntry {
        RebindEntry { from, to_game: game, to_chat: 0 }
    }

    fn mk_split(from: i32, game: i32, chat: i32) -> RebindEntry {
        RebindEntry { from, to_game: game, to_chat: chat }
    }

    // Pin an explicit game-mode state so tests don't depend on the
    // process-wide CURSOR_CAPTURED atomic (which defaults to false and
    // would otherwise route through the chat target in `effective_in_chat`).
    fn new_in_game() -> KeyRebinder {
        let mut rb = KeyRebinder::new();
        rb.set_game_state("inworld,cursor_grabbed");
        rb
    }

    #[test]
    fn basic_remap() {
        let mut rb = new_in_game();
        rb.on = true;
        rb.entries.push(mk(65, 66)); // A -> B

        assert_eq!(rb.remap_key(65, 0), 66);
    }

    #[test]
    fn no_match_returns_original() {
        let mut rb = new_in_game();
        rb.on = true;
        rb.entries.push(mk(65, 66));

        assert_eq!(rb.remap_key(67, 0), 67); // C unchanged
    }

    #[test]
    fn disabled_returns_original() {
        let mut rb = new_in_game();
        rb.on = false;
        rb.entries.push(mk(65, 66));

        assert_eq!(rb.remap_key(65, 0), 65);
    }

    #[test]
    fn multiple_rebinds() {
        let mut rb = new_in_game();
        rb.on = true;
        rb.entries.push(mk(65, 66)); // A -> B
        rb.entries.push(mk(67, 68)); // C -> D
        rb.entries.push(mk(69, 70)); // E -> F

        assert_eq!(rb.remap_key(65, 0), 66);
        assert_eq!(rb.remap_key(67, 0), 68);
        assert_eq!(rb.remap_key(69, 0), 70);
        assert_eq!(rb.remap_key(71, 0), 71); // G unchanged
    }

    #[test]
    fn reverse_remap() {
        let mut rb = new_in_game();
        rb.on = true;
        rb.entries.push(mk(344, 404)); // RShift -> Mouse5

        assert_eq!(rb.remap_key(344, 0), 404);
        assert_eq!(rb.remap_key(404, 0), 404);
        assert_eq!(rb.reverse_remap_key(404), 344);
        assert_eq!(rb.reverse_remap_key(344), 344);
    }

    #[test]
    fn split_game_chat_targets() {
        let mut rb = new_in_game();
        rb.on = true;
        // O -> Q in game, O -> P in chat
        rb.entries.push(mk_split(79, 81, 80));

        assert_eq!(rb.remap_key(79, 0), 81); // game mode by default

        rb.set_game_state("inworld,cursor_free");
        assert_eq!(rb.remap_key(79, 0), 80); // chat mode

        rb.set_game_state("inworld,cursor_grabbed");
        assert_eq!(rb.remap_key(79, 0), 81); // back to game
    }

    #[test]
    fn chat_zero_passes_key_through_in_chat() {
        // Game-only rebind (to_chat=0): the key must pass through unchanged
        // when the user opens chat / a text field. Previously this fell back
        // to the game target, making game binds fire while typing.
        let mut rb = KeyRebinder::new();
        rb.on = true;
        rb.entries.push(mk(65, 66)); // to_chat = 0

        rb.set_game_state("inworld,cursor_free");
        assert_eq!(rb.remap_key(65, 0), 65); // unchanged in chat

        rb.set_game_state("inworld,cursor_grabbed");
        assert_eq!(rb.remap_key(65, 0), 66); // active in game
    }

    #[test]
    fn game_zero_passes_key_through_in_game() {
        // Chat-only rebind (to_game=0): the key must pass through unchanged
        // during gameplay.
        let mut rb = KeyRebinder::new();
        rb.on = true;
        rb.entries.push(mk_split(65, 0, 66)); // to_game=0, to_chat=66

        rb.set_game_state("inworld,cursor_grabbed");
        assert_eq!(rb.remap_key(65, 0), 65); // unchanged in game

        rb.set_game_state("inworld,cursor_free");
        assert_eq!(rb.remap_key(65, 0), 66); // active in chat
    }

    #[test]
    fn config_update_replaces_bindings() {
        let mut rb = new_in_game();

        rb.on = true;
        rb.entries.push(mk(65, 66));
        assert_eq!(rb.remap_key(65, 0), 66);

        let config = KeyRebindsConfig {
            enabled: true,
            rebinds: vec![
                KeyRebind {
                    from_key: 80,
                    to_key: 81,
                    to_key_chat: 0,
                    enabled: true,
                },
                KeyRebind {
                    from_key: 90,
                    to_key: 91,
                    to_key_chat: 0,
                    enabled: false, // disabled -- skipped
                },
            ],
        };

        rb.update_from_config(&config);

        assert_eq!(rb.remap_key(65, 0), 65); // old rule gone
        assert_eq!(rb.remap_key(80, 0), 81); // new rule
        assert_eq!(rb.remap_key(90, 0), 90); // disabled rule not loaded
    }

    #[test]
    fn chat_only_rebind_passes_through_in_game() {
        let mut rb = new_in_game();
        rb.on = true;
        // to_game = 0, to_chat = 345 (RCtrl) - chat-only rebind
        rb.entries.push(mk_split(65, 0, 345));

        // in game mode: no valid target = passes through original key
        assert_eq!(rb.remap_key(65, 0), 65);

        // in chat mode: remaps to RCtrl
        rb.set_game_state("inworld,cursor_free");
        assert_eq!(rb.remap_key(65, 0), 345);
    }

    #[test]
    fn chat_only_rebind_loads_from_config() {
        let mut rb = new_in_game();
        let config = KeyRebindsConfig {
            enabled: true,
            rebinds: vec![KeyRebind {
                from_key: 65,
                to_key: 0,
                to_key_chat: 345,
                enabled: true,
            }],
        };
        rb.update_from_config(&config);
        assert_eq!(rb.entries.len(), 1); // must not be skipped

        rb.set_game_state("inworld,cursor_free");
        assert_eq!(rb.remap_key(65, 0), 345);
    }

    #[test]
    fn scancode_based_remap() {
        use tuxinjector_config::key_names::SCANCODE_OFFSET;

        let mut rb = new_in_game();
        rb.on = true;
        // scan:30 (A position) -> B
        rb.entries.push(mk(SCANCODE_OFFSET as i32 + 30, 66));

        // GLFW key 65 (A) with scancode 30 should match
        assert_eq!(rb.remap_key(65, 30), 66);
        // different scancode should not match
        assert_eq!(rb.remap_key(65, 31), 65);
        // no scancode should not match
        assert_eq!(rb.remap_key(65, 0), 65);
    }

    #[test]
    fn falls_back_to_cursor_state_without_mod() {
        // No mod providing game_state -> effective_in_chat relies on the
        // live GLFW CURSOR_CAPTURED flag. Default in tests is false, so
        // the rebinder should behave as if in chat (= cursor freed).
        let mut rb = KeyRebinder::new();
        rb.on = true;
        // Game-only rebind: must pass through when cursor is freed.
        rb.entries.push(mk(65, 66));
        assert_eq!(rb.remap_key(65, 0), 65);

        // Setting an explicit game state overrides the fallback.
        rb.set_game_state("inworld,cursor_grabbed");
        assert_eq!(rb.remap_key(65, 0), 66);
    }
}
