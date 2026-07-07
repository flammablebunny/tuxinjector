// Application-level input handler. Wires the hotkey engine, key rebinder,
// and mouse sensitivity into the InputHandler trait from tuxinjector-input.

use std::sync::Arc;

use crossbeam_channel::Sender;
use tuxinjector_config::ConfigSnapshot;
use tuxinjector_input::callbacks::InputHandler;
use tuxinjector_input::{HotkeyAction, HotkeyEngine, KeyRebinder, SensitivityState};

use crate::state;

// registered with the input crate on first frame
pub struct TuxinjectorInputHandler {
    hotkeys: HotkeyEngine,
    rebinder: KeyRebinder,
    sens: SensitivityState,
    config: Arc<ConfigSnapshot>,
    cfg_version: u64,
    lua_tx: Option<Sender<u64>>,
}

impl TuxinjectorInputHandler {
    pub fn new(config: Arc<ConfigSnapshot>) -> Self {
        let cfg = config.load();

        let mut hotkeys = HotkeyEngine::new();
        hotkeys.update_from_config(&cfg);

        let mut rebinder = KeyRebinder::new();
        rebinder.update_from_config(&cfg.input.key_rebinds);
        tuxinjector_input::update_key_rebinds(&rebinder.active_rebinds());

        let mut sens = SensitivityState::new();
        sens.set_base_sensitivity(cfg.input.mouse_sensitivity);

        tuxinjector_input::set_key_repeat(
            cfg.input.key_repeat_enabled,
            cfg.input.key_repeat_start_delay,
            cfg.input.key_repeat_delay,
        );

        Self {
            hotkeys,
            rebinder,
            sens,
            config,
            cfg_version: 0,
            lua_tx: None,
        }
    }

    pub fn set_lua_callback_channel(&mut self, tx: Sender<u64>) {
        self.lua_tx = Some(tx);
    }

    pub fn register_lua_actions(&mut self, bindings: &[(Vec<i32>, u64, bool)]) {
        self.hotkeys.update_lua_actions(bindings);
    }

    // check if config changed and reload hotkeys/rebinds/sensitivity
    fn maybe_reload(&mut self) {
        let ver = self.config.version();
        if ver != self.cfg_version {
            self.cfg_version = ver;
            let cfg = self.config.load();
            self.hotkeys.update_from_config(&cfg);
            self.rebinder.update_from_config(&cfg.input.key_rebinds);
            tuxinjector_input::update_key_rebinds(&self.rebinder.active_rebinds());
            self.sens.set_base_sensitivity(cfg.input.mouse_sensitivity);
            tuxinjector_input::set_key_repeat(
                cfg.input.key_repeat_enabled,
                cfg.input.key_repeat_start_delay,
                cfg.input.key_repeat_delay,
            );

            // pick up Lua action bindings stashed by the reload path
            if let Some(bindings) = state::get().lua_bindings.lock().unwrap().take() {
                self.hotkeys.update_lua_actions(&bindings);
                tracing::debug!(count = bindings.len(), "reloaded Lua action bindings");
            }
        }

        // sync game state for hotkey + rebind conditions
        if let Ok(gs) = state::get().game_state.lock() {
            self.hotkeys.set_game_state(&gs);
            if self.rebinder.set_game_state(&gs) {
                tuxinjector_input::update_key_rebinds(&self.rebinder.active_rebinds());
            }
        }

        // If there's no live game-state mod hooked up, treat state conditions as
        // "Any" - otherwise state-conditioned hotkeys would never match and just
        // silently do nothing.
        let state_live = matches!(
            tuxinjector_gui::state_mod_status(),
            tuxinjector_gui::StateModStatus::Hermes | tuxinjector_gui::StateModStatus::StateOutput
        );
        self.hotkeys.set_state_available(state_live);
    }

    fn dispatch(&mut self, action: &HotkeyAction) {
        match action {
            HotkeyAction::SwitchMode { main, secondary, .. } => {
                tracing::debug!(main, secondary, "hotkey: switch mode");
                let mut target = String::new();
                if let Some(lock) = state::get().overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        // toggle: if already in secondary, go back to main.
                        // otherwise (including if in a different mode), go to secondary.
                        let current = overlay.effective_mode_id();
                        let in_secondary = !secondary.is_empty() && current == secondary.as_str();
                        let t = if in_secondary {
                            main.clone()
                        } else if secondary.is_empty() {
                            // no secondary = simple toggle with initial mode
                            if current == main.as_str() {
                                overlay.initial_mode_id().to_owned()
                            } else {
                                main.clone()
                            }
                        } else {
                            secondary.clone()
                        };
                        overlay.switch_mode(&t);
                        target = t;
                    }
                }
                if !target.is_empty() {
                    tuxinjector_lua::update_mode_name(&target);
                }
                // apply per-mode sensitivity directly (avoids INPUT_HANDLER deadlock)
                if !target.is_empty() {
                    let cfg = self.config.load();
                    if let Some(mode) = cfg.modes.iter().find(|m| m.id == target) {
                        if mode.sensitivity_override_enabled {
                            let sep = if mode.separate_xy_sensitivity {
                                Some((mode.mode_sensitivity_x, mode.mode_sensitivity_y))
                            } else {
                                None
                            };
                            self.sens.set_mode_override(mode.mode_sensitivity, sep);
                        } else {
                            self.sens.clear_mode_override();
                        }
                    }
                }
            }
            HotkeyAction::ToggleSensitivity { sensitivity, separate_xy, x, y } => {
                let sep = if *separate_xy { Some((*x, *y)) } else { None };
                self.sens.toggle_hotkey_override(*sensitivity, sep);
            }
            HotkeyAction::ToggleGui => {
                tracing::debug!("hotkey: toggle GUI");
                if let Some(lock) = state::get().overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        overlay.toggle_gui();
                    }
                }
            }
            HotkeyAction::ToggleImageOverlays => {
                tracing::debug!("hotkey: toggle images");
                if let Some(lock) = state::get().overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        overlay.toggle_image_overlays();
                    }
                }
            }
            HotkeyAction::ToggleWindowOverlays => {
                tracing::debug!("hotkey: toggle windows");
                if let Some(lock) = state::get().overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        overlay.toggle_window_overlays();
                    }
                }
            }
            HotkeyAction::ToggleBorderless => {
                tracing::debug!("hotkey: toggle borderless");
                crate::viewport_hook::request_borderless_toggle();
            }
            HotkeyAction::ToggleAppVisibility => {
                tracing::debug!("hotkey: toggle app visibility");
                if let Some(lock) = state::get().overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        overlay.toggle_app_visibility();
                    }
                }
            }
            HotkeyAction::ToggleNinjabrainOverlay => {
                let vis = !crate::nbb_overlay::NBB_OVERLAY_VISIBLE
                    .load(std::sync::atomic::Ordering::Relaxed);
                crate::nbb_overlay::NBB_OVERLAY_VISIBLE
                    .store(vis, std::sync::atomic::Ordering::Relaxed);
                tracing::debug!(visible = vis, "hotkey: toggle ninjabrain overlay");
            }
            HotkeyAction::LaunchApps { nbb, paceman } => {
                tracing::debug!(nbb, paceman, "hotkey: launch companion apps");
                tuxinjector_gui::tabs::apps::request_launch(*nbb, *paceman);
            }
            HotkeyAction::Custom(name) => {
                tracing::debug!(name, "hotkey: custom action");
            }
            HotkeyAction::LuaCallback(id) => {
                tracing::info!(id, "hotkey: Lua callback fired");
                if let Some(ref tx) = self.lua_tx {
                    let _ = tx.try_send(*id);
                } else {
                    tracing::warn!(id, "Lua callback channel not wired, dropping");
                }
            }
        }
    }
}

impl InputHandler for TuxinjectorInputHandler {
    fn handle_key(&mut self, key: i32, scancode: i32, action: i32, mods: i32) -> (bool, i32) {
        self.maybe_reload();
        self.hotkeys.set_current_mode(&tuxinjector_lua::get_mode_name());

        // forward key press/release to embedded companion apps (XTEST injection)
        if action == 1 || action == 0 {
            crate::app_capture::push_app_key(key, scancode, mods, action == 1);
        }

        // live shift/alt state drives the keyboard layers; when it changes,
        // re-publish the active table so glfwGetKey polling stays in sync
        if self.rebinder.set_mods(mods) {
            tuxinjector_input::update_key_rebinds(&self.rebinder.active_rebinds());
        }

        let orig = key;
        let remapped = self.rebinder.remap_key(key, scancode);

        // GUI key capture mode - grab the key for the hotkey editor,
        // but not if a text field is focused (let the user type normally)
        if tuxinjector_input::is_gui_capture_mode() && action == 1 /* PRESS */
            && !tuxinjector_input::gui_wants_keyboard()
        {
            tuxinjector_input::push_captured_key(orig);
            return (true, remapped);
        }

        if tuxinjector_input::gui_is_visible() {
            // always check hotkeys so toggle-GUI can close the GUI
            let (consumed, actions) = self.hotkeys.process_key(orig, scancode, action, mods);

            if consumed {
                for a in &actions { self.dispatch(a); }
                return (true, remapped);
            }

            // Escape closes the GUI overlay
            if orig == tuxinjector_input::glfw_types::GLFW_KEY_ESCAPE && action == 1 {
                if let Some(lock) = state::get().overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        overlay.toggle_gui();
                    }
                }
                return (true, remapped);
            }

            // forward to GUI so characters like '/' work in text fields
            let pressed = action != 0;
            tuxinjector_input::push_gui_key(remapped, mods, pressed);
            return (true, remapped);
        }

        let (consumed, actions) = self.hotkeys.process_key(remapped, scancode, action, mods);
        for a in &actions { self.dispatch(a); }

        (consumed, remapped)
    }

    fn handle_mouse_button(&mut self, button: i32, action: i32, mods: i32) -> (bool, i32) {
        use tuxinjector_input::glfw_types::MOUSE_BUTTON_OFFSET;

        self.maybe_reload();
        self.hotkeys.set_current_mode(&tuxinjector_lua::get_mode_name());

        if self.rebinder.set_mods(mods) {
            tuxinjector_input::update_key_rebinds(&self.rebinder.active_rebinds());
        }

        let encoded = button + MOUSE_BUTTON_OFFSET;
        let mut remapped = self.rebinder.remap_key(encoded, 0);

        // no forward match - try reverse (e.g. "RShift -> Mouse5" means
        // pressing Mouse5 should act as RShift for hotkey purposes)
        if remapped == encoded {
            let rev = self.rebinder.reverse_remap_key(encoded);
            if rev != encoded { remapped = rev; }
        }

        // GUI capture mode - capture mouse3+ (middle, side buttons)
        if tuxinjector_input::is_gui_capture_mode() && action == 1 && button >= 2 {
            tuxinjector_input::push_captured_key(encoded);
            return (true, encoded);
        }

        if tuxinjector_input::gui_is_visible() {
            if button == 0 {
                if action == 1 { tuxinjector_input::push_gui_button_press(); }
                else if action == 0 { tuxinjector_input::push_gui_button_release(); }
                tuxinjector_input::push_gui_button_mods(mods);
            } else if button == 1 {
                if action == 1 { tuxinjector_input::push_gui_rbutton_press(); }
                else if action == 0 { tuxinjector_input::push_gui_rbutton_release(); }
            }
            return (true, encoded);
        }

        // Settings GUI is closed, but the update launch popup may be up. Route a
        // left click to imgui only when the cursor is actually over the popup, so
        // clicks anywhere else still reach the game.
        if button == 0 && tuxinjector_input::popup_capturing_mouse() {
            if action == 1 { tuxinjector_input::push_gui_button_press(); }
            else if action == 0 { tuxinjector_input::push_gui_button_release(); }
            tuxinjector_input::push_gui_button_mods(mods);
            return (true, encoded);
        }

        let (consumed, actions) = self.hotkeys.process_key(remapped, 0, action, mods);
        for a in &actions { self.dispatch(a); }
        if consumed { return (true, encoded); }

        // remapped - callback layer handles mouse->mouse and mouse->key forwarding
        if remapped != encoded {
            return (false, remapped);
        }

        (false, button)
    }

    fn handle_cursor_pos(&mut self, x: f64, y: f64) -> Option<(f64, f64)> {
        // reset tracking on cursor recapture so we don't get a huge delta spike
        if tuxinjector_input::callbacks::take_cursor_recaptured() {
            self.sens.reset_tracking();
            tracing::debug!("cursor recaptured: sensitivity tracking reset");
        }

        // When the GUI is open, don't forward cursor movement to the game
        // (prevents the in-game camera from moving while navigating the menu)
        if tuxinjector_input::gui_is_visible() {
            return None;
        }

        if tuxinjector_input::is_cursor_captured() {
            // FPS/relative mode - apply sensitivity scaling
            Some(self.sens.scale_cursor(x, y))
        } else {
            // menu / absolute mode. Defer to the shared transform so this
            // callback path and the glfwGetCursorPos poll path stay in sync.
            Some(unsafe { crate::viewport_hook::cursor_screen_to_game(x, y) })
        }
    }

    fn handle_scroll(&mut self, x: f64, y: f64) -> bool {
        if tuxinjector_input::gui_is_visible() {
            tuxinjector_input::push_gui_scroll(x as f32, y as f32);
            return true;
        }
        false
    }

    fn set_mode_sensitivity(&mut self, s: f32, separate: Option<(f32, f32)>) {
        tracing::debug!(s, ?separate, "set_mode_sensitivity");
        self.sens.set_mode_override(s, separate);
    }

    fn clear_mode_sensitivity(&mut self) {
        tracing::debug!("clear_mode_sensitivity");
        self.sens.clear_mode_override();
    }
}
