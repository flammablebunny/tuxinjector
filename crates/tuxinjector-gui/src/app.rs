use imgui::{Condition, StyleColor};
use tuxinjector_config::Config;

use crate::tabs::apps::AppsState;
use crate::tabs::general::GeneralState;
use crate::tabs::hotkeys::HotkeysState;
use crate::tabs::key_rebinds::KeyRebindsState;
use crate::tabs::plugins::PluginsState; // still needed for plugin API

pub struct SettingsOutput {
    pub saved_config: Option<Config>,
    pub wants_key_capture: bool,
    pub profile_switch: Option<String>,
    pub profile_create: Option<String>,
    pub profile_delete: Option<String>,
    pub profile_rename: Option<(String, String)>,
    // snapshot of draft before profile switch so the current one keeps edits
    pub pre_switch_draft: Option<Config>,
}

pub struct SettingsApp {
    visible: bool,
    draft: Config,
    dirty: bool,
    pub selected_mode_idx: Option<usize>,
    pub selected_mirror_idx: Option<usize>,
    pub selected_image_idx: Option<usize>,
    pub selected_window_overlay_idx: Option<usize>,
    pub selected_browser_overlay_idx: Option<usize>,
    pub profile_list: Vec<String>,
    pub new_profile_name: String,
    profile_switch: Option<String>,
    profile_create: Option<String>,
    profile_delete: Option<String>,
    profile_rename: Option<(String, String)>,
    general_state: GeneralState,
    hotkeys_state: HotkeysState,
    key_rebinds_state: KeyRebindsState,
    apps_state: AppsState,
    plugins_state: PluginsState,
    font_cache: Option<Vec<(String, String)>>,
    confirm_reset: bool,
}

impl SettingsApp {
    pub fn new(initial_config: Config) -> Self {
        Self {
            visible: false,
            draft: initial_config,
            dirty: false,
            selected_mode_idx: None,
            selected_mirror_idx: None,
            selected_image_idx: None,
            selected_window_overlay_idx: None,
            selected_browser_overlay_idx: None,
            profile_list: Vec::new(),
            new_profile_name: String::new(),
            profile_switch: None,
            profile_create: None,
            profile_delete: None,
            profile_rename: None,
            general_state: GeneralState::default(),
            hotkeys_state: HotkeysState::default(),
            key_rebinds_state: KeyRebindsState::default(),
            apps_state: AppsState::default(),
            plugins_state: PluginsState::default(),
            font_cache: None,
            confirm_reset: false,
        }
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        if !self.visible {
            // clean up capture states so keys don't get swallowed
            self.general_state.cancel();
            self.hotkeys_state.capturing = false;
            self.hotkeys_state.capture_toggled = Some(false);
            self.key_rebinds_state.cancel();
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn update_profile(&mut self, name: String) {
        self.draft.profile = name;
    }

    // Slam draft unconditionally (profile switch)
    pub fn force_update_config(&mut self, cfg: Config) {
        self.draft = cfg;
        self.dirty = false;
    }

    // Only overwrite if user hasn't touched anything
    pub fn update_config(&mut self, cfg: Config) {
        if !self.dirty {
            self.draft = cfg;
        }
    }

    pub fn update_loaded_plugins(&mut self, summaries: Vec<crate::tabs::plugins::PluginSummary>) {
        self.plugins_state.loaded_plugins = summaries;
    }

    pub fn take_plugin_actions(&mut self) -> Vec<crate::tabs::plugins::PluginAction> {
        std::mem::take(&mut self.plugins_state.actions)
    }

    pub fn render(&mut self, ui: &imgui::Ui, captured_key: Option<u32>) -> SettingsOutput {
        if !self.visible {
            return SettingsOutput {
                saved_config: None,
                wants_key_capture: false,
                profile_switch: None,
                profile_create: None,
                profile_delete: None,
                profile_rename: None,
                pre_switch_draft: None,
            };
        }

        let mut saved = None;

        // scale relative to 1080p baseline
        let [dw, dh] = ui.io().display_size;
        let scale = (dh / 1080.0).max(1.0);
        let def_w = dw.min(850.0 * scale);
        let def_h = dh.min(650.0 * scale);

        let _bg = ui.push_style_color(StyleColor::WindowBg, [0.10, 0.08, 0.14, 0.96]);
        let _border = ui.push_style_color(StyleColor::Border, [0.50, 0.30, 0.70, 0.39]);

        ui.window("Tux Injector")
            .size([def_w, def_h], Condition::FirstUseEver)
            .size_constraints([400.0 * scale, 300.0 * scale], [dw, dh])
            .resizable(true)
            .build(|| {
                if let Some(_tab_bar) = ui.tab_bar("settings_tabs") {
                    if let Some(_tab) = ui.tab_item("General") {
                        crate::tabs::general::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.general_state,
                            captured_key,
                            &self.profile_list,
                            &mut self.new_profile_name,
                            &mut self.profile_switch,
                            &mut self.profile_create,
                            &mut self.profile_delete,
                            &mut self.profile_rename,
                        );
                    }
                    if let Some(_tab) = ui.tab_item("Modes") {
                        crate::tabs::modes::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.selected_mode_idx,
                        );
                    }
                    if let Some(_tab) = ui.tab_item("Mirrors") {
                        crate::tabs::mirrors::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.selected_mirror_idx,
                        );
                    }
                    if let Some(_tab) = ui.tab_item("Images") {
                        crate::tabs::images::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.selected_image_idx,
                        );
                    }
                    if let Some(_tab) = ui.tab_item("Overlays") {
                        crate::tabs::window_overlays::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.selected_window_overlay_idx,
                        );
                    }
                    if let Some(_tab) = ui.tab_item("Hotkeys") {
                        crate::tabs::hotkeys::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.hotkeys_state,
                            captured_key,
                        );
                    }
                    if let Some(_tab) = ui.tab_item("Mouse") {
                        crate::tabs::mouse::render(ui, &mut self.draft, &mut self.dirty);
                    }
                    if let Some(_tab) = ui.tab_item("EyeZoom") {
                        crate::tabs::eyezoom::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.font_cache,
                        );
                    }
                    if let Some(_tab) = ui.tab_item("Rebinds") {
                        crate::tabs::key_rebinds::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.key_rebinds_state,
                            captured_key,
                        );
                    }
                    // if let Some(_tab) = ui.tab_item("Cursors") {
                    //     crate::tabs::cursors::render(ui, &mut self.draft, &mut self.dirty);
                    // }
                    if let Some(_tab) = ui.tab_item("Appearance") {
                        crate::tabs::appearance::render(ui, &mut self.draft, &mut self.dirty);
                    }
                    if let Some(_tab) = ui.tab_item("Debug") {
                        crate::tabs::debug::render(ui, &mut self.draft, &mut self.dirty);
                    }
                    if let Some(_tab) = ui.tab_item("Apps") {
                        crate::tabs::apps::render(ui, &mut self.apps_state);
                    }
                    #[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
                    if let Some(_tab) = ui.tab_item("Browser") {
                        let live = crate::tabs::browser_overlays::render(
                            ui,
                            &mut self.draft,
                            &mut self.dirty,
                            &mut self.selected_browser_overlay_idx,
                        );
                        if live {
                            saved = Some(self.draft.clone());
                        }
                    }
                    #[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
                    if let Some(_tab) = ui.tab_item("Plugins") {
                        crate::tabs::plugins::render(ui, &mut self.plugins_state);
                    }
                }

                ui.separator();

                // -- footer bar --
                let footer_y = ui.cursor_screen_pos()[1];
                let win_pos = ui.window_pos();
                let win_sz = ui.window_size();
                let style = ui.clone_style();
                let right_edge = win_pos[0] + win_sz[0] - style.window_padding[0];

                if self.dirty {
                    if ui.button("Save") {
                        saved = Some(self.draft.clone());
                        self.dirty = false;
                        self.confirm_reset = false;
                    }
                    ui.same_line();
                    if ui.button("Discard") {
                        self.dirty = false;
                        self.confirm_reset = false;
                    }
                    ui.same_line();
                }

                if ui.button("Close") {
                    self.visible = false;
                    self.general_state.cancel();
                    self.hotkeys_state.capturing = false;
                    self.hotkeys_state.capture_toggled = Some(false);
                    self.key_rebinds_state.cancel();
                    self.confirm_reset = false;
                }

                if self.dirty {
                    ui.same_line();
                    if self.confirm_reset {
                        ui.text_colored([1.0, 0.3, 0.3, 1.0], "Reset to defaults -- click Save to confirm");
                    } else {
                        ui.text_colored([1.0, 0.78, 0.2, 1.0], "Unsaved changes");
                    }
                }

                // right-aligned reset button
                {
                    let txt = "Reset to Defaults";
                    let pad = style.frame_padding[0] * 2.0;
                    let btn_w = ui.calc_text_size(txt)[0] + pad;
                    ui.set_cursor_screen_pos([right_edge - btn_w, footer_y]);
                    if ui.button(txt) {
                        let prof = self.draft.profile.clone();
                        self.draft = Config::default();
                        self.draft.profile = prof;
                        self.dirty = true;
                        self.confirm_reset = true;
                    }
                }
            });

        let wants_capture = self.general_state.is_capturing()
            || self.hotkeys_state.capturing
            || self.hotkeys_state.capturing_exclusion.is_some()
            || self.key_rebinds_state.is_capturing();

        // clear toggle signals now that we've read them
        self.general_state.capture_toggled = None;
        self.hotkeys_state.capture_toggled = None;

        let profile_switch = self.profile_switch.take();
        let profile_create = self.profile_create.take();
        let profile_delete = self.profile_delete.take();
        let profile_rename = self.profile_rename.take();

        // save draft before switching so the old profile keeps the user's edits
        let pre_switch = if profile_switch.is_some() || profile_create.is_some() {
            Some(self.draft.clone())
        } else {
            None
        };

        SettingsOutput {
            saved_config: saved,
            wants_key_capture: wants_capture,
            profile_switch,
            profile_create,
            profile_delete,
            profile_rename,
            pre_switch_draft: pre_switch,
        }
    }
}
