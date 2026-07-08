// Hotkey matching engine - multi-key combos, release-triggers, debounce

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tracing::debug;

use crate::glfw_types::{GLFW_PRESS, GLFW_RELEASE, GLFW_REPEAT};
use tuxinjector_config::types::{Config, HotkeyConditions};


#[derive(Debug, Clone, PartialEq)]
pub enum HotkeyAction {
    SwitchMode {
        main: String,
        secondary: String,
        // when true, the game-state condition is ignored if pressing this would
        // exit a mode it controls (so you can always get back to fullscreen)
        allow_exit_fullscreen: bool,
    },
    ToggleSensitivity {
        sensitivity: f32,
        separate_xy: bool,
        x: f32,
        y: f32,
    },
    ToggleGui,
    ToggleImageOverlays,
    ToggleWindowOverlays,
    ToggleBorderless,
    ToggleAppVisibility,
    ToggleNinjabrainOverlay,
    // launch the selected companion apps (NinjaBrainBot / Paceman)
    LaunchApps {
        nbb: bool,
        paceman: bool,
    },
    Custom(String),
    // lua callback by registry ID
    LuaCallback(u64),
}

#[derive(Debug, Clone)]
struct Binding {
    keys: Vec<i32>,        // all must be held at once
    action: HotkeyAction,
    on_release: bool,
    block_game: bool,
    debounce_ms: i32,
    conditions: HotkeyConditions,
}

pub struct HotkeyEngine {
    held: HashSet<i32>,
    bindings: Vec<Binding>,
    last_fire: HashMap<usize, Instant>,
    // indices of bindings that already fired for current key combo
    fired: HashSet<usize>,
    game_state: String,
    // which mode we're in right now - needed for the allow_exit_fullscreen bypass
    current_mode: String,
    // do we actually have a live state source (Hermes / State Output)?
    // if not, we can't trust game_state, so we treat conditions as "Any"
    // otherwise state-conditioned hotkeys would just silently never fire
    state_available: bool,
}

impl HotkeyEngine {
    pub fn new() -> Self {
        Self {
            held: HashSet::new(),
            bindings: Vec::new(),
            last_fire: HashMap::new(),
            fired: HashSet::new(),
            game_state: String::new(),
            current_mode: String::new(),
            state_available: false,
        }
    }

    pub fn set_game_state(&mut self, state: &str) {
        if self.game_state != state {
            self.game_state = state.to_string();
        }
    }

    pub fn set_current_mode(&mut self, mode: &str) {
        if self.current_mode != mode {
            self.current_mode = mode.to_string();
        }
    }

    pub fn set_state_available(&mut self, available: bool) {
        self.state_available = available;
    }

    // rebuild bindings from config, but keep lua hotkeys across reloads
    pub fn update_from_config(&mut self, config: &Config) {
        let lua_bindings: Vec<Binding> = self
            .bindings
            .iter()
            .filter(|b| matches!(b.action, HotkeyAction::LuaCallback(_)))
            .cloned()
            .collect();

        self.bindings.clear();
        self.last_fire.clear();

        for hk in &config.hotkeys.mode_hotkeys {
            let keys: Vec<i32> = hk.keys.iter().map(|&k| k as i32).collect();
            if keys.is_empty() {
                continue;
            }
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::SwitchMode {
                    main: hk.main_mode.clone(),
                    secondary: hk.secondary_mode.clone(),
                    allow_exit_fullscreen: hk.allow_exit_to_fullscreen_regardless_of_game_state,
                },
                on_release: hk.trigger_on_release,
                block_game: hk.block_key_from_game,
                debounce_ms: hk.debounce,
                conditions: hk.conditions.clone(),
            });
        }

        for shk in &config.input.sensitivity_hotkeys {
            let keys: Vec<i32> = shk.keys.iter().map(|&k| k as i32).collect();
            if keys.is_empty() {
                continue;
            }
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::ToggleSensitivity {
                    sensitivity: shk.sensitivity,
                    separate_xy: shk.separate_xy,
                    x: shk.sensitivity_x,
                    y: shk.sensitivity_y,
                },
                on_release: false,
                block_game: false,
                debounce_ms: shk.debounce,
                conditions: shk.conditions.clone(),
            });
        }

        // debounce to prevent flicker from key chatter on cheap keyboards
        const TOGGLE_DEBOUNCE: i32 = 150;

        if !config.hotkeys.gui.is_empty() {
            let keys: Vec<i32> = config.hotkeys.gui.iter().map(|&k| k as i32).collect();
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::ToggleGui,
                on_release: false,
                block_game: true,
                debounce_ms: TOGGLE_DEBOUNCE,
                conditions: HotkeyConditions::default(),
            });
        }

        if !config.hotkeys.image_overlays.is_empty() {
            let keys: Vec<i32> = config
                .hotkeys.image_overlays
                .iter()
                .map(|&k| k as i32)
                .collect();
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::ToggleImageOverlays,
                on_release: false,
                block_game: false,
                debounce_ms: TOGGLE_DEBOUNCE,
                conditions: HotkeyConditions::default(),
            });
        }

        if !config.hotkeys.window_overlays.is_empty() {
            let keys: Vec<i32> = config
                .hotkeys.window_overlays
                .iter()
                .map(|&k| k as i32)
                .collect();
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::ToggleWindowOverlays,
                on_release: false,
                block_game: false,
                debounce_ms: TOGGLE_DEBOUNCE,
                conditions: HotkeyConditions::default(),
            });
        }

        if !config.hotkeys.ninjabrain_overlay.is_empty() {
            let keys: Vec<i32> = config
                .hotkeys.ninjabrain_overlay
                .iter()
                .map(|&k| k as i32)
                .collect();
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::ToggleNinjabrainOverlay,
                on_release: false,
                block_game: false,
                debounce_ms: TOGGLE_DEBOUNCE,
                conditions: HotkeyConditions::default(),
            });
        }

        if !config.hotkeys.app_visibility.is_empty() {
            let keys: Vec<i32> = config
                .hotkeys.app_visibility
                .iter()
                .map(|&k| k as i32)
                .collect();
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::ToggleAppVisibility,
                on_release: false,
                block_game: true,
                debounce_ms: TOGGLE_DEBOUNCE,
                conditions: HotkeyConditions::default(),
            });
        }

        if !config.hotkeys.launch_apps.is_empty() {
            let keys: Vec<i32> = config
                .hotkeys.launch_apps
                .iter()
                .map(|&k| k as i32)
                .collect();
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::LaunchApps {
                    nbb: config.hotkeys.launch_nbb,
                    paceman: config.hotkeys.launch_paceman,
                },
                on_release: false,
                block_game: true,
                // launching is heavy; guard against rapid re-fire on key chatter
                debounce_ms: 1000,
                conditions: HotkeyConditions::default(),
            });
        }

        if !config.hotkeys.borderless.is_empty() {
            let keys: Vec<i32> = config
                .hotkeys.borderless
                .iter()
                .map(|&k| k as i32)
                .collect();
            self.bindings.push(Binding {
                keys,
                action: HotkeyAction::ToggleBorderless,
                on_release: false,
                block_game: false,
                debounce_ms: TOGGLE_DEBOUNCE,
                conditions: HotkeyConditions::default(),
            });
        }

        self.bindings.extend(lua_bindings);

        debug!(
            count = self.bindings.len(),
            "rebuilt hotkey bindings from config"
        );
    }

    // Swap out all lua callback bindings at once
    pub fn update_lua_actions(&mut self, entries: &[(Vec<i32>, u64, bool)]) {
        self.bindings
            .retain(|b| !matches!(b.action, HotkeyAction::LuaCallback(_)));

        for (combo, cb_id, block) in entries {
            if combo.is_empty() {
                continue;
            }
            self.bindings.push(Binding {
                keys: combo.clone(),
                action: HotkeyAction::LuaCallback(*cb_id),
                on_release: false,
                block_game: *block,
                debounce_ms: 0,
                conditions: HotkeyConditions::default(),
            });
        }
        if !entries.is_empty() {
            debug!(
                count = entries.len(),
                total = self.bindings.len(),
                "registered Lua action bindings"
            );
        }
    }

    // Feed a key event in. Returns (consumed, triggered_actions).
    // scancode is the physical evdev scancode from GLFW, used for scan:-prefixed bindings.
    pub fn process_key(&mut self, key: i32, scancode: i32, action: i32, _mods: i32) -> (bool, Vec<HotkeyAction>) {
        let sc_key = tuxinjector_config::key_names::SCANCODE_OFFSET as i32 + scancode;
        match action {
            GLFW_PRESS => {
                self.held.insert(key);
                if scancode > 0 { self.held.insert(sc_key); }
                self.fired.clear();
            }
            GLFW_RELEASE => {
                self.held.remove(&key);
                self.held.remove(&sc_key);
                self.fired.clear();
            }
            GLFW_REPEAT => {
                // re-insert in case wayland lost focus; don't clear fired
                self.held.insert(key);
                if scancode > 0 { self.held.insert(sc_key); }
            }
            _ => {}
        };

        let mut out = Vec::new();
        let mut consumed = false;
        let now = Instant::now();

        for (i, bind) in self.bindings.iter().enumerate() {
            if self.fired.contains(&i) {
                continue;
            }

            if self.check_match(bind, key, scancode, action) {
                // debounce check
                if let Some(last) = self.last_fire.get(&i) {
                    if last.elapsed().as_millis() < bind.debounce_ms as u128 {
                        continue;
                    }
                }

                self.last_fire.insert(i, now);
                self.fired.insert(i);
                out.push(bind.action.clone());

                if bind.block_game {
                    consumed = true;
                }
            }
        }

        (consumed, out)
    }

    fn check_match(&self, bind: &Binding, key: i32, scancode: i32, action: i32) -> bool {
        // Only enforce the game-state condition if we actually have a live state
        // source. No Hermes / State Output (or it's stale)? Then treat it as "Any"
        // so these hotkeys still fire instead of being dead.
        if self.state_available && !bind.conditions.game_state.is_empty() {
            // The "allow exit to fullscreen regardless of game state" flag is
            // one-way: it only lets the press through when we're LEAVING the
            // resize mode, never when entering it. Otherwise you'd be able to
            // re-enter the resize in any state, which defeats the whole point.
            // resize_mode is `secondary`, or `main` if there's no secondary (a
            // plain toggle). We're only "leaving" it when we're already in it.
            let exit_bypass = if let HotkeyAction::SwitchMode {
                main, secondary, allow_exit_fullscreen,
            } = &bind.action {
                let resize_mode = if secondary.is_empty() { main } else { secondary };
                *allow_exit_fullscreen && self.current_mode == *resize_mode
            } else {
                false
            };
            if !exit_bypass {
                let ok = bind
                    .conditions
                    .game_state
                    .iter()
                    .any(|s| s == &self.game_state);
                if !ok {
                    return false;
                }
            }
        }

        // exclusion keys (not checked for release-triggers)
        if !bind.on_release && !bind.conditions.exclusions.is_empty() {
            let excluded = bind
                .conditions
                .exclusions
                .iter()
                .any(|&k| self.held.contains(&(k as i32)));
            if excluded {
                return false;
            }
        }

        let all_held = bind.keys.iter().all(|k| self.held.contains(k));

        if bind.on_release {
            // fire when released key was part of the combo and remaining keys still held
            action == GLFW_RELEASE && !all_held && {
                let remaining = bind.keys.iter().filter(|k| self.held.contains(k)).count();
                remaining == bind.keys.len() - 1
            }
        } else {
            // Fire only on a PRESS of one of THIS binding's own keys. Without
            // this, pressing any unrelated key while a held combo is still down
            // re-matches it (all_held stays true) and re-triggers the action.
            let sc_key = tuxinjector_config::key_names::SCANCODE_OFFSET as i32 + scancode;
            action == GLFW_PRESS
                && all_held
                && (bind.keys.contains(&key) || (scancode > 0 && bind.keys.contains(&sc_key)))
        }
    }

    pub fn pressed_keys(&self) -> &HashSet<i32> {
        &self.held
    }

    pub fn clear_pressed(&mut self) {
        self.held.clear();
        self.fired.clear();
    }
}

impl Default for HotkeyEngine {
    fn default() -> Self {
        Self::new()
    }
}

// --- tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glfw_types::GLFW_REPEAT;

    fn make_binding(keys: Vec<i32>, action: HotkeyAction) -> Binding {
        Binding {
            keys,
            action,
            on_release: false,
            block_game: false,
            debounce_ms: 0,
            conditions: HotkeyConditions::default(),
        }
    }

    fn gui_action() -> HotkeyAction {
        HotkeyAction::ToggleGui
    }

    fn switch_action(main: &str, sec: &str) -> HotkeyAction {
        HotkeyAction::SwitchMode {
            main: main.into(),
            secondary: sec.into(),
            allow_exit_fullscreen: false,
        }
    }

    #[test]
    fn single_key_press_triggers() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(make_binding(vec![290], gui_action())); // F1

        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], HotkeyAction::ToggleGui);
    }

    #[test]
    fn multi_key_combo_requires_all_keys() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(make_binding(vec![341, 290], gui_action())); // Ctrl+F1

        let (_, actions) = engine.process_key(341, 0, GLFW_PRESS, 0);
        assert!(actions.is_empty());

        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], HotkeyAction::ToggleGui);
    }

    #[test]
    fn trigger_on_release() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(Binding {
            keys: vec![290],
            action: gui_action(),
            on_release: true,
            block_game: false,
            debounce_ms: 0,
            conditions: HotkeyConditions::default(),
        });

        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert!(actions.is_empty());

        let (_, actions) = engine.process_key(290, 0, GLFW_RELEASE, 0);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], HotkeyAction::ToggleGui);
    }

    #[test]
    fn debounce_prevents_rapid_retriggering() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(Binding {
            keys: vec![290],
            action: gui_action(),
            on_release: false,
            block_game: false,
            debounce_ms: 500,
            conditions: HotkeyConditions::default(),
        });

        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1);

        engine.process_key(290, 0, GLFW_RELEASE, 0);
        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert!(actions.is_empty());
    }

    #[test]
    fn block_from_game_sets_consumed() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(Binding {
            keys: vec![290],
            action: gui_action(),
            on_release: false,
            block_game: true,
            debounce_ms: 0,
            conditions: HotkeyConditions::default(),
        });

        let (consumed, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert!(consumed);
        assert_eq!(actions.len(), 1);
    }

    #[test]
    fn update_from_config_rebuilds_bindings() {
        use tuxinjector_config::types::{Config, HotkeyConfig};

        let mut engine = HotkeyEngine::new();
        engine.bindings.push(make_binding(vec![290], gui_action()));
        assert_eq!(engine.bindings.len(), 1);

        let mut config = Config::default();
        config.hotkeys.mode_hotkeys.clear();
        config.hotkeys.mode_hotkeys.push(HotkeyConfig {
            keys: vec![290],
            main_mode: "wall".into(),
            secondary_mode: "game".into(),
            ..Default::default()
        });
        config.hotkeys.mode_hotkeys.push(HotkeyConfig {
            keys: vec![291],
            main_mode: "zoom".into(),
            secondary_mode: "game".into(),
            ..Default::default()
        });
        config.hotkeys.gui = vec![292];

        engine.update_from_config(&config);
        assert_eq!(engine.bindings.len(), 3);
    }

    #[test]
    fn key_release_clears_pressed_state() {
        let mut engine = HotkeyEngine::new();

        engine.process_key(290, 0, GLFW_PRESS, 0);
        assert!(engine.held.contains(&290));

        engine.process_key(290, 0, GLFW_RELEASE, 0);
        assert!(!engine.held.contains(&290));
    }

    #[test]
    fn multiple_hotkeys_trigger_simultaneously() {
        let mut engine = HotkeyEngine::new();
        engine
            .bindings
            .push(make_binding(vec![290], switch_action("wall", "game")));
        engine.bindings.push(make_binding(vec![290], gui_action()));

        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 2);
    }

    #[test]
    fn no_match_returns_empty() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(make_binding(vec![290], gui_action())); // F1

        let (consumed, actions) = engine.process_key(291, 0, GLFW_PRESS, 0); // F2
        assert!(!consumed);
        assert!(actions.is_empty());
    }

    #[test]
    fn sensitivity_hotkey_produces_correct_action() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(make_binding(
            vec![340], // LShift
            HotkeyAction::ToggleSensitivity {
                sensitivity: 0.5,
                separate_xy: true,
                x: 0.3,
                y: 0.7,
            },
        ));

        let (_, actions) = engine.process_key(340, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1);
        assert_eq!(
            actions[0],
            HotkeyAction::ToggleSensitivity {
                sensitivity: 0.5,
                separate_xy: true,
                x: 0.3,
                y: 0.7,
            }
        );
    }

    #[test]
    fn multi_key_release_trigger_requires_all_held_first() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(Binding {
            keys: vec![341, 290], // Ctrl+F1
            action: gui_action(),
            on_release: true,
            block_game: false,
            debounce_ms: 0,
            conditions: HotkeyConditions::default(),
        });

        let (_, actions) = engine.process_key(341, 0, GLFW_PRESS, 0);
        assert!(actions.is_empty());

        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0); // release-trigger, doesn't fire yet
        assert!(actions.is_empty());

        let (_, actions) = engine.process_key(290, 0, GLFW_RELEASE, 0); // now it fires
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], HotkeyAction::ToggleGui);
    }

    #[test]
    fn clear_pressed_resets_state() {
        let mut engine = HotkeyEngine::new();

        engine.process_key(290, 0, GLFW_PRESS, 0);
        engine.process_key(291, 0, GLFW_PRESS, 0);
        assert_eq!(engine.held.len(), 2);

        engine.clear_pressed();
        assert!(engine.held.is_empty());
        assert!(engine.fired.is_empty());
    }

    #[test]
    fn press_hotkey_while_holding_other_key() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(make_binding(vec![290], gui_action()));

        // hold W with REPEAT flooding
        engine.process_key(87, 0, GLFW_PRESS, 0);
        engine.process_key(87, 0, GLFW_REPEAT, 0);
        engine.process_key(87, 0, GLFW_REPEAT, 0);
        engine.process_key(87, 0, GLFW_REPEAT, 0);

        // F1 while W held -- must fire
        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1, "F1 must fire while W is held");
        assert_eq!(actions[0], HotkeyAction::ToggleGui);
    }

    #[test]
    fn repeat_on_hotkey_key_does_not_retrigger() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(Binding {
            keys: vec![290],
            action: gui_action(),
            on_release: false,
            block_game: false,
            debounce_ms: 0,
            conditions: HotkeyConditions::default(),
        });

        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1);

        // REPEAT must NOT re-fire
        let (_, actions) = engine.process_key(290, 0, GLFW_REPEAT, 0);
        assert!(actions.is_empty(), "REPEAT must not re-trigger the hotkey");
        let (_, actions) = engine.process_key(290, 0, GLFW_REPEAT, 0);
        assert!(actions.is_empty());
    }

    #[test]
    fn other_key_repeat_after_hotkey_release_does_not_fire() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(Binding {
            keys: vec![290],
            action: gui_action(),
            on_release: false,
            block_game: false,
            debounce_ms: 0,
            conditions: HotkeyConditions::default(),
        });

        engine.process_key(87, 0, GLFW_PRESS, 0);   // W down
        engine.process_key(290, 0, GLFW_PRESS, 0);  // F1 down -> fires
        engine.process_key(290, 0, GLFW_RELEASE, 0); // F1 up

        // W REPEATs must not fire F1
        let (_, actions) = engine.process_key(87, 0, GLFW_REPEAT, 0);
        assert!(actions.is_empty(), "W REPEAT after F1 release must not fire F1");
        let (_, actions) = engine.process_key(87, 0, GLFW_REPEAT, 0);
        assert!(actions.is_empty());

        // F1 again should fire
        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1, "F1 must fire again after release");
    }

    #[test]
    fn hold_modifier_spam_action_key_retriggers() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(make_binding(vec![342, 290], gui_action())); // Alt+F1

        // hold Alt
        let (_, actions) = engine.process_key(342, 0, GLFW_PRESS, 0);
        assert!(actions.is_empty());

        // first F1 press
        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1, "Alt+F1 must fire on first press");

        // release F1, alt still held
        engine.process_key(290, 0, GLFW_RELEASE, 0);

        // second F1 press - should fire again without re-pressing alt
        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1, "Alt+F1 must fire again while Alt held");

        // third time for good measure
        engine.process_key(290, 0, GLFW_RELEASE, 0);
        let (_, actions) = engine.process_key(290, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1, "Alt+F1 must fire on third press too");
    }

    fn cond(states: &[&str]) -> HotkeyConditions {
        HotkeyConditions {
            game_state: states.iter().map(|s| s.to_string()).collect(),
            exclusions: Vec::new(),
        }
    }

    // block_game must consume only when the state-conditioned hotkey actually
    // fires. When a live state source says we're in a state the hotkey doesn't
    // allow, the key must pass through to the game (not be swallowed).
    #[test]
    fn block_game_consumes_only_when_state_condition_met() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(Binding {
            keys: vec![293], // F4
            action: switch_action("thin", "game"),
            on_release: false,
            block_game: true,
            debounce_ms: 0,
            conditions: cond(&["world"]),
        });
        engine.set_state_available(true);

        // condition NOT met -> must NOT fire and must NOT consume
        engine.set_game_state("menu");
        let (consumed, actions) = engine.process_key(293, 0, GLFW_PRESS, 0);
        assert!(actions.is_empty(), "must not fire when state condition unmet");
        assert!(!consumed, "must not block key when resize can't trigger");
        engine.process_key(293, 0, GLFW_RELEASE, 0);

        // condition met -> must fire and consume
        engine.set_game_state("world");
        let (consumed, actions) = engine.process_key(293, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1, "must fire when state condition met");
        assert!(consumed, "must block key from game when resize triggers");
    }

    // "Allow exit to fullscreen regardless of game state" must only bypass the
    // condition when leaving the resize (Fullscreen <- Thin), never when
    // entering it (Fullscreen -> Thin). Mirrors the default config where
    // main = Fullscreen, secondary = the resize mode.
    #[test]
    fn allow_exit_bypasses_only_when_leaving_resize() {
        let mut engine = HotkeyEngine::new();
        engine.bindings.push(Binding {
            keys: vec![90], // Z
            action: HotkeyAction::SwitchMode {
                main: "Fullscreen".into(),
                secondary: "Thin".into(),
                allow_exit_fullscreen: true,
            },
            on_release: false,
            block_game: false,
            debounce_ms: 0,
            conditions: cond(&["world"]),
        });
        engine.set_state_available(true);
        engine.set_game_state("menu"); // condition NOT met

        // in Fullscreen, the press would ENTER the resize -> still gated
        engine.set_current_mode("Fullscreen");
        let (_, actions) = engine.process_key(90, 0, GLFW_PRESS, 0);
        assert!(actions.is_empty(), "must not enter resize when state unmet");
        engine.process_key(90, 0, GLFW_RELEASE, 0);

        // in the resize, the press EXITS to fullscreen -> allowed regardless
        engine.set_current_mode("Thin");
        let (_, actions) = engine.process_key(90, 0, GLFW_PRESS, 0);
        assert_eq!(actions.len(), 1, "must allow exit to fullscreen regardless of state");
    }
}
