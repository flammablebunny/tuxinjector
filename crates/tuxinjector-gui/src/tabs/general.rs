use tuxinjector_config::key_names::keys_to_combo_string;
use tuxinjector_config::Config;

#[derive(Default)]
pub struct GeneralState {
    capturing: Option<&'static str>,
    // Some(true) = capture started, Some(false) = stopped. consumed each frame
    pub capture_toggled: Option<bool>,
    fonts: Option<Vec<(String, String)>>,
    confirm_delete: bool,
    rename_buf: String,
}

impl GeneralState {
    pub fn is_capturing(&self) -> bool {
        self.capturing.is_some()
    }

    pub fn cancel(&mut self) {
        if self.capturing.is_some() {
            self.capturing = None;
            self.capture_toggled = Some(false);
        }
    }
}

pub fn render(
    ui: &imgui::Ui,
    config: &mut Config,
    dirty: &mut bool,
    state: &mut GeneralState,
    captured_key: Option<u32>,
    profile_list: &[String],
    new_profile_name: &mut String,
    profile_switch: &mut Option<String>,
    profile_create: &mut Option<String>,
    profile_delete: &mut Option<String>,
    profile_rename: &mut Option<(String, String)>,
) {
    // -- Profile selector --
    ui.separator(); ui.text("Profile");
    ui.dummy([0.0, 4.0]);

    let cur_label = if config.profile.is_empty() {
        "(Default)".to_owned()
    } else {
        config.profile.clone()
    };

    ui.text("Active Profile:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    if let Some(_token) = ui.begin_combo("##profile_select", &cur_label) {
        if ui.selectable_config("(Default)")
            .selected(config.profile.is_empty())
            .build()
        {
            if !config.profile.is_empty() {
                *profile_switch = Some(String::new());
                state.confirm_delete = false;
            }
        }
        for name in profile_list {
            if ui.selectable_config(name)
                .selected(config.profile == *name)
                .build()
            {
                if config.profile != *name {
                    *profile_switch = Some(name.clone());
                    state.confirm_delete = false;
                }
            }
        }
    }

    ui.same_line();
    if ui.small_button("New##profile_new") {
        ui.open_popup("New Profile");
    }

    if !config.profile.is_empty() {
        ui.same_line();
        if ui.small_button("Rename##profile_ren") {
            state.rename_buf = config.profile.clone();
            ui.open_popup("Rename Profile");
        }

        ui.same_line();
        if state.confirm_delete {
            if ui.small_button("Confirm Delete?##profile_del_confirm") {
                *profile_delete = Some(config.profile.clone());
                state.confirm_delete = false;
            }
            ui.same_line();
            if ui.small_button("Cancel##profile_del_cancel") {
                state.confirm_delete = false;
            }
        } else if ui.small_button("Delete##profile_del") {
            state.confirm_delete = true;
        }
    }

    // new profile popup
    if let Some(_popup) = ui.begin_popup("New Profile") {
        ui.text("Profile Name:");
        ui.set_next_item_width(200.0);
        ui.input_text("##new_profile_name", new_profile_name).build();
        if ui.button("Create") && !new_profile_name.is_empty() {
            *profile_create = Some(new_profile_name.clone());
            new_profile_name.clear();
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel") {
            new_profile_name.clear();
            ui.close_current_popup();
        }
    }

    // rename popup
    if let Some(_popup) = ui.begin_popup("Rename Profile") {
        ui.text("New Name:");
        ui.set_next_item_width(200.0);
        ui.input_text("##rename_profile_name", &mut state.rename_buf).build();
        if ui.button("Rename") && !state.rename_buf.is_empty() && state.rename_buf != config.profile {
            *profile_rename = Some((config.profile.clone(), state.rename_buf.clone()));
            state.rename_buf.clear();
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel##rename_cancel") {
            state.rename_buf.clear();
            ui.close_current_popup();
        }
    }

    ui.dummy([0.0, 8.0]);

    // -- Core stuff --
    ui.separator(); ui.text("Core Settings");
    ui.dummy([0.0, 4.0]);

    ui.text("Default Mode:");
    ui.same_line();
    let mode_names: Vec<String> = config.modes.iter().map(|m| m.id.clone()).collect();
    if let Some(_token) = ui.begin_combo("##default_mode", &config.display.default_mode) {
        for name in &mode_names {
            if ui.selectable_config(name)
                .selected(*name == config.display.default_mode)
                .build()
            {
                config.display.default_mode = name.clone();
                *dirty = true;
            }
        }
    }

    ui.text("FPS Limit:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, "##fps_limit", &mut config.display.fps_limit, 0, 1000, "%d fps") {
        *dirty = true;
    }
    ui.same_line(); ui.text_disabled("(0 = unlimited)");

    // font picker - lazy init
    let fonts = state
        .fonts
        .get_or_insert_with(crate::widgets::discover_fonts);

    // clear if the font file vanished
    if !config.theme.font_path.is_empty() && !std::path::Path::new(&config.theme.font_path).exists() {
        config.theme.font_path = String::new();
        *dirty = true;
    }

    let preview = if config.theme.font_path.is_empty() {
        "Default (ProggyClean)"
    } else {
        fonts
            .iter()
            .find(|(_, p)| *p == config.theme.font_path)
            .map(|(n, _)| n.as_str())
            .unwrap_or(&config.theme.font_path)
    };

    ui.text("Font:");
    ui.same_line();
    ui.set_next_item_width(280.0);
    if let Some(_token) = ui.begin_combo("##font_path", preview) {
        if ui.selectable_config("Default (ProggyClean)")
            .selected(config.theme.font_path.is_empty())
            .build()
        {
            config.theme.font_path = String::new();
            *dirty = true;
        }
        for (name, path) in fonts.iter() {
            let is_sel = *path == config.theme.font_path;
            if ui.selectable_config(name).selected(is_sel).build() {
                config.theme.font_path = path.clone();
                *dirty = true;
            }
        }
    }

    ui.text("GUI Scale:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    if crate::widgets::slider_float(ui, "##gui_scale", &mut config.theme.appearance.gui_scale, 0.75, 2.5, "%.2f")
    {
        *dirty = true;
    }

    ui.dummy([0.0, 8.0]);
    if ui.checkbox("Disable All Animations", &mut config.display.disable_animations) {
        *dirty = true;
    }
    if ui.checkbox(
        "Disable Fullscreen Prompt",
        &mut config.advanced.disable_fullscreen_prompt,
    ) {
        *dirty = true;
    }
    if ui.checkbox(
        "Disable Configure Prompt",
        &mut config.advanced.disable_configure_prompt,
    ) {
        *dirty = true;
    }

    // -- Key repeat --
    ui.dummy([0.0, 12.0]);
    ui.separator(); ui.text("Key Repeat");
    ui.dummy([0.0, 4.0]);

    ui.text("Start Delay (ms):");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, "##repeat_start", &mut config.input.key_repeat_start_delay, 0, 2000, "%d ms") {
        *dirty = true;
    }

    ui.text("Repeat Delay (ms):");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, "##repeat_delay", &mut config.input.key_repeat_delay, 0, 500, "%d ms") {
        *dirty = true;
    }

    // -- Hook chaining --
    ui.dummy([0.0, 12.0]);
    ui.separator(); ui.text("Hook Chaining");
    ui.dummy([0.0, 4.0]);

    if ui.checkbox(
        "Disable Hook Chaining",
        &mut config.advanced.disable_hook_chaining,
    ) {
        *dirty = true;
    }

    if !config.advanced.disable_hook_chaining {
        ui.text("Next Target:");
        ui.same_line();
        let cur = format!("{:?}", config.advanced.hook_chaining_next_target);
        if let Some(_token) = ui.begin_combo("##hook_chain_target", &cur) {
            use tuxinjector_config::types::HookChainingNextTarget;
            if ui.selectable_config("LatestHook")
                .selected(
                    config.advanced.hook_chaining_next_target == HookChainingNextTarget::LatestHook,
                )
                .build()
            {
                config.advanced.hook_chaining_next_target = HookChainingNextTarget::LatestHook;
                *dirty = true;
            }
            if ui.selectable_config("OriginalFunction")
                .selected(
                    config.advanced.hook_chaining_next_target
                        == HookChainingNextTarget::OriginalFunction,
                )
                .build()
            {
                config.advanced.hook_chaining_next_target = HookChainingNextTarget::OriginalFunction;
                *dirty = true;
            }
        }
    }

    // -- Global hotkeys --
    ui.dummy([0.0, 12.0]);
    ui.separator(); ui.text("Global Hotkeys");
    crate::widgets::text_wrapped_colored(
        ui,
        [0.627, 0.569, 0.725, 1.0],
        "These hotkeys trigger global overlay actions regardless of mode",
    );
    ui.dummy([0.0, 4.0]);

    hotkey_row(ui, "GUI Toggle:", &mut config.hotkeys.gui, "gui_hk", dirty, state, captured_key);
    hotkey_row(ui, "Toggle Borderless:", &mut config.hotkeys.borderless, "borderless_hk", dirty, state, captured_key);
    hotkey_row(ui, "Toggle Image Overlays:", &mut config.hotkeys.image_overlays, "img_hk", dirty, state, captured_key);
    hotkey_row(ui, "Toggle Window Overlays:", &mut config.hotkeys.window_overlays, "wo_hk", dirty, state, captured_key);
    hotkey_row(ui, "Toggle App Visibility:", &mut config.hotkeys.app_visibility, "appvis_hk", dirty, state, captured_key);
}

fn hotkey_row(
    ui: &imgui::Ui,
    label: &str,
    keys: &mut Vec<u32>,
    id: &'static str,
    dirty: &mut bool,
    state: &mut GeneralState,
    captured_key: Option<u32>,
) {
    let active = state.capturing == Some(id);

    // accumulate keys while capturing
    if active {
        if let Some(key) = captured_key {
            keys.push(key);
            keys.sort();
            keys.dedup();
            *dirty = true;
        }
    }

    ui.text(label);
    ui.same_line();

    if keys.is_empty() {
        ui.text_colored([0.471, 0.431, 0.549, 1.0], "(none)");
    } else {
        ui.text_colored([0.784, 0.667, 0.941, 1.0], keys_to_combo_string(keys));
    }

    ui.same_line();

    if active {
        ui.text_colored([1.0, 0.784, 0.196, 1.0], "Press keys\u{2026}");
        ui.same_line();
        if ui.button(format!("Done##{id}")) {
            state.capturing = None;
            state.capture_toggled = Some(false);
        }
    } else {
        if ui.small_button(format!("Capture##{id}")) {
            keys.clear();
            *dirty = true;
            state.capturing = Some(id);
            state.capture_toggled = Some(true);
        }
        if !keys.is_empty() {
            ui.same_line();
            if ui.small_button(format!("Clear##{id}")) {
                keys.clear();
                *dirty = true;
            }
        }
    }
}
