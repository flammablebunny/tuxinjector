use tuxinjector_config::key_names::{keycode_to_name, keys_to_combo_string, parse_key_combo_str};
use tuxinjector_config::types::{AltSecondaryMode, HotkeyConfig};
use tuxinjector_config::Config;

#[derive(Default)]
pub struct HotkeysState {
    pub selected: Option<usize>,
    pub key_input: String,
    pub key_error: Option<String>,
    pub capturing: bool,
    // Some(true) = started, Some(false) = stopped. cleared after reading
    pub capture_toggled: Option<bool>,
    // which exclusion slot is being rebound (if any)
    pub capturing_exclusion: Option<usize>,
}

pub fn render(
    ui: &imgui::Ui,
    config: &mut Config,
    dirty: &mut bool,
    state: &mut HotkeysState,
    captured_key: Option<u32>,
) {
    // warn about non-ascii mode names on the default font
    if config.theme.font_path.is_empty() {
        let non_ascii = config.modes.iter().any(|m| m.id.chars().any(|c| !c.is_ascii()))
            || config.hotkeys.mode_hotkeys.iter().any(|h| {
                h.main_mode.chars().any(|c| !c.is_ascii())
                    || h.secondary_mode.chars().any(|c| !c.is_ascii())
            });
        if non_ascii {
            crate::widgets::text_wrapped_colored(
                ui,
                [1.0, 0.85, 0.0, 1.0],
                "Some mode names contain characters the default font cant display. Select a system font in General > Font.",
            );
            ui.dummy([0.0, 4.0]);
        }
    }

    // No game-state mod -> "Required Game States" conditions can never match.
    if crate::state_mod_status() == crate::StateModStatus::Missing {
        crate::widgets::text_wrapped_colored(
            ui,
            [1.0, 0.5, 0.2, 1.0],
            "No game-state mod detected. Hotkeys with a \"Required Game States\" condition \
             will never fire (only \"Any\" works). Install the Hermes or State Output mods \
             so tuxinjector can read the instance state.",
        );
        ui.dummy([0.0, 4.0]);
    }

    if let Some(idx) = state.selected {
        if idx >= config.hotkeys.mode_hotkeys.len() {
            state.selected = None;
        }
    }

    // feed captured keys into the right place
    if let Some(raw) = captured_key {
        if state.capturing {
            state.capturing_exclusion = None;

            if let Some(idx) = state.selected {
                if idx < config.hotkeys.mode_hotkeys.len() {
                    let hk = &mut config.hotkeys.mode_hotkeys[idx];
                    hk.keys.push(raw);
                    hk.keys.sort();
                    hk.keys.dedup();
                    state.key_input = keys_to_combo_string(&hk.keys);
                    state.key_error = None;
                    *dirty = true;
                }
            }
        } else if let Some(excl_idx) = state.capturing_exclusion.take() {
            state.capture_toggled = Some(false);

            if let Some(idx) = state.selected {
                if idx < config.hotkeys.mode_hotkeys.len() {
                    let excls = &mut config.hotkeys.mode_hotkeys[idx].conditions.exclusions;
                    if excl_idx < excls.len() {
                        excls[excl_idx] = raw;
                        *dirty = true;
                    }
                }
            }
        }
    }

    ui.columns(2, "hotkeys_cols", true);

    // left side: hotkey list
    ui.text("Hotkey List");
    ui.separator();

    for (i, hk) in config.hotkeys.mode_hotkeys.iter().enumerate() {
        let keys_str = if hk.keys.is_empty() {
            "(no keys)".to_string()
        } else {
            keys_to_combo_string(&hk.keys)
        };
        let label = format!("{}: {} -> {}", keys_str, hk.main_mode, hk.secondary_mode);
        if ui.selectable_config(&label)
            .selected(state.selected == Some(i))
            .build()
        {
            state.selected = Some(i);
            state.key_input = keys_to_combo_string(&config.hotkeys.mode_hotkeys[i].keys);
            state.key_error = None;
            if state.capturing {
                state.capturing = false;
                state.capture_toggled = Some(false);
            }
            state.capturing_exclusion = None;
        }
    }

    ui.dummy([0.0, 8.0]);
    if ui.button("Add Hotkey") {
        config.hotkeys.mode_hotkeys.push(HotkeyConfig::default());
        let new_idx = config.hotkeys.mode_hotkeys.len() - 1;
        state.selected = Some(new_idx);
        state.key_input = String::new();
        state.key_error = None;
        if state.capturing {
            state.capturing = false;
            state.capture_toggled = Some(false);
        }
        state.capturing_exclusion = None;
        *dirty = true;
    }

    // right side: editor
    ui.next_column();
    if let Some(idx) = state.selected {
        if idx < config.hotkeys.mode_hotkeys.len() {
            let modes: Vec<String> = config.modes.iter().map(|m| m.id.clone()).collect();
            hotkey_editor(ui, &mut config.hotkeys.mode_hotkeys[idx], idx, dirty, state, &modes);

            ui.dummy([0.0, 12.0]);
            if ui.button("Remove Hotkey") {
                config.hotkeys.mode_hotkeys.remove(idx);
                state.selected = None;
                if state.capturing {
                    state.capturing = false;
                    state.capture_toggled = Some(false);
                }
                state.capturing_exclusion = None;
                *dirty = true;
            }
        }
    } else {
        ui.text("Select a hotkey to edit.");
    }

    ui.columns(1, "hotkeys_cols_end", false);
}

fn hotkey_editor(
    ui: &imgui::Ui,
    hk: &mut HotkeyConfig,
    idx: usize,
    dirty: &mut bool,
    state: &mut HotkeysState,
    mode_names: &[String],
) {
    // key combo text input
    ui.text("Keys:");
    ui.set_next_item_width(200.0);
    if ui
        .input_text(format!("##hk_keys_{idx}"), &mut state.key_input)
        .hint("e.g. ctrl+F1")
        .build()
    {
        match parse_key_combo_str(&state.key_input) {
            Ok(parsed) => {
                hk.keys = parsed;
                state.key_error = None;
                *dirty = true;
            }
            Err(e) => {
                state.key_error = Some(e);
            }
        }
    }

    ui.same_line();
    if state.capturing {
        ui.text_colored([1.0, 1.0, 0.0, 1.0], "Press keys...");
        ui.same_line();
        if ui.button(format!("Done##hk_done_{idx}")) {
            state.capturing = false;
            state.capture_toggled = Some(false);
        }
    } else {
        if ui.button(format!("Capture##hk_cap_{idx}")) {
            hk.keys.clear();
            state.key_input.clear();
            state.key_error = None;
            state.capturing = true;
            state.capture_toggled = Some(true);
            state.capturing_exclusion = None;
        }
    }

    if let Some(ref err) = state.key_error {
        ui.text_colored([1.0, 0.0, 0.0, 1.0], err);
    }

    ui.dummy([0.0, 8.0]);

    // mode selectors
    ui.text("Main Mode:");
    ui.same_line();
    if let Some(_token) = ui.begin_combo(format!("##hk_main_{idx}"), &hk.main_mode) {
        for name in mode_names {
            if ui.selectable_config(name)
                .selected(hk.main_mode == *name)
                .build()
            {
                hk.main_mode = name.clone();
                *dirty = true;
            }
        }
    }

    ui.text("Secondary Mode:");
    ui.same_line();
    if let Some(_token) = ui.begin_combo(format!("##hk_sec_{idx}"), &hk.secondary_mode) {
        for name in mode_names {
            if ui.selectable_config(name)
                .selected(hk.secondary_mode == *name)
                .build()
            {
                hk.secondary_mode = name.clone();
                *dirty = true;
            }
        }
    }

    // trigger options
    ui.dummy([0.0, 8.0]);
    if ui.checkbox(
        format!("Trigger On Release##hk_tor_{idx}"),
        &mut hk.trigger_on_release,
    ) {
        *dirty = true;
    }

    ui.text("Debounce (ms):");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, &format!("##hk_debounce_{idx}"), &mut hk.debounce, 0, 2000, "%d ms") {
        *dirty = true;
    }

    if ui.checkbox(
        format!("Block Key From Game##hk_block_{idx}"),
        &mut hk.block_key_from_game,
    ) {
        *dirty = true;
    }
    if ui.checkbox(
        format!("Allow Exit To Fullscreen Regardless Of Game State##hk_exit_{idx}"),
        &mut hk.allow_exit_to_fullscreen_regardless_of_game_state,
    ) {
        *dirty = true;
    }

    // alt secondary modes
    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Alt Secondary Modes");
    let mut rm_alt = None;
    for (i, alt) in hk.alt_secondary_modes.iter_mut().enumerate() {
        if ui.collapsing_header(
            format!("Alt #{}##alt_sec_{}_{}", i, idx, i),
            imgui::TreeNodeFlags::empty(),
        ) {
            let mut alt_str = keys_to_combo_string(&alt.keys);
            ui.text("Keys:");
            ui.set_next_item_width(180.0);
            if ui
                .input_text(format!("##alt_keys_{}_{}", idx, i), &mut alt_str)
                .hint("e.g. shift+F1")
                .build()
            {
                if let Ok(parsed) = parse_key_combo_str(&alt_str) {
                    alt.keys = parsed;
                    *dirty = true;
                }
            }

            ui.text("Mode:");
            ui.same_line();
            if let Some(_token) =
                ui.begin_combo(format!("##alt_mode_{}_{}", idx, i), &alt.mode)
            {
                for name in mode_names {
                    if ui.selectable_config(format!("{name}##alt_sel_{idx}_{i}"))
                        .selected(alt.mode == *name)
                        .build()
                    {
                        alt.mode = name.clone();
                        *dirty = true;
                    }
                }
            }
            if ui.small_button(format!("Remove Alt##rm_alt_{}_{}", idx, i)) {
                rm_alt = Some(i);
                *dirty = true;
            }
        }
    }
    if let Some(ri) = rm_alt {
        hk.alt_secondary_modes.remove(ri);
    }
    if ui.small_button(format!("Add Alt Secondary##add_alt_{idx}")) {
        hk.alt_secondary_modes.push(AltSecondaryMode::default());
        *dirty = true;
    }

    // game state conditions
    ui.dummy([0.0, 8.0]);
    if ui.collapsing_header(
        format!("Required Game States##req_states_{idx}"),
        imgui::TreeNodeFlags::empty(),
    ) {
        game_state_checkboxes(ui, &mut hk.conditions.game_state, dirty, idx);
    }

    if ui.collapsing_header(
        format!("Exclusion Keys##excl_keys_{idx}"),
        imgui::TreeNodeFlags::empty(),
    ) {
        exclusion_keys(ui, hk, idx, dirty, state);
    }
}

// empty vec = fire in any state. inworld split uses GLFW cursor state
// which is legal per speedrun.com rules (no mod data)
fn game_state_checkboxes(ui: &imgui::Ui, states: &mut Vec<String>, dirty: &mut bool, idx: usize) {
    let mut any = states.is_empty();
    if ui.checkbox(format!("Any##gs_any_{idx}"), &mut any) {
        if any {
            states.clear();
        } else {
            states.push("wall".to_string());
            states.push("inworld,cursor_free".to_string());
            states.push("inworld,cursor_grabbed".to_string());
            states.push("title".to_string());
            states.push("generating".to_string());
            states.push("waiting".to_string());
        }
        *dirty = true;
    }

    let _dis = ui.begin_disabled(any);
    state_cb(ui, "Wall Screen", "wall", states, dirty, idx);
    state_cb(ui, "In World (Playing)", "inworld,cursor_grabbed", states, dirty, idx);
    state_cb(ui, "In World (Menu/Chat/Inv.)", "inworld,cursor_free", states, dirty, idx);
    state_cb(ui, "Title Screen", "title", states, dirty, idx);
    gen_cb(ui, states, dirty, idx);
}

fn state_cb(
    ui: &imgui::Ui,
    label: &str,
    val: &str,
    states: &mut Vec<String>,
    dirty: &mut bool,
    idx: usize,
) {
    let mut on = states.iter().any(|s| s == val);
    if ui.checkbox(format!("{label}##gs_{val}_{idx}"), &mut on) {
        if on {
            states.push(val.to_string());
        } else {
            states.retain(|s| s != val);
        }
        *dirty = true;
    }
}

// "generating" and "waiting" are grouped together as "World Generation"
fn gen_cb(ui: &imgui::Ui, states: &mut Vec<String>, dirty: &mut bool, idx: usize) {
    let mut on = states.iter().any(|s| s == "generating" || s == "waiting");
    if ui.checkbox(format!("World Generation##gs_gen_{idx}"), &mut on) {
        if on {
            if !states.iter().any(|s| s == "generating") {
                states.push("generating".to_string());
            }
            if !states.iter().any(|s| s == "waiting") {
                states.push("waiting".to_string());
            }
        } else {
            states.retain(|s| s != "generating" && s != "waiting");
        }
        *dirty = true;
    }
}

fn exclusion_keys(
    ui: &imgui::Ui,
    hk: &mut HotkeyConfig,
    _idx: usize,
    dirty: &mut bool,
    state: &mut HotkeysState,
) {
    ui.text("Hotkey is suppressed when any of these keys are held.");

    let mut rm_excl = None;
    for (i, &key) in hk.conditions.exclusions.iter().enumerate() {
        let is_cap = state.capturing_exclusion == Some(i);
        let btn_txt = if is_cap {
            format!("Press a key...##excl_btn_{i}")
        } else {
            format!("{}##excl_btn_{i}", keycode_to_name(key))
        };
        if is_cap {
            ui.text_colored([1.0, 1.0, 0.0, 1.0], "");
            ui.same_line();
        }
        if ui.button(&btn_txt) {
            if is_cap {
                state.capturing_exclusion = None;
                state.capture_toggled = Some(false);
            } else {
                state.capturing = false;
                state.capturing_exclusion = Some(i);
                state.capture_toggled = Some(true);
            }
        }
        ui.same_line();
        if ui.small_button(format!("\u{00d7}##excl_rm_{i}")) {
            rm_excl = Some(i);
            if state.capturing_exclusion == Some(i) {
                state.capturing_exclusion = None;
                state.capture_toggled = Some(false);
            }
            *dirty = true;
        }
    }
    if let Some(i) = rm_excl {
        hk.conditions.exclusions.remove(i);
    }

    if ui.small_button("+ Add Exclusion") {
        hk.conditions.exclusions.push(0);
        let new_i = hk.conditions.exclusions.len() - 1;
        state.capturing = false;
        state.capturing_exclusion = Some(new_i);
        state.capture_toggled = Some(true);
        *dirty = true;
    }
}
