use tuxinjector_config::Config;

pub fn render(ui: &imgui::Ui, config: &mut Config, dirty: &mut bool) {
    ui.separator(); ui.text("Mouse Sensitivity");

    ui.dummy([0.0, 8.0]);
    ui.text("Global Sensitivity:");
    ui.set_next_item_width(-1.0);
    if crate::widgets::slider_float_log(ui, "##global_sens", &mut config.input.mouse_sensitivity, 0.1, 20.0, "%.2fx") {
        *dirty = true;
    }

    // per-mode overrides (editable here, but the toggle lives in the Modes tab too)
    ui.dummy([0.0, 16.0]);
    ui.separator(); ui.text("Per-Mode Overrides");
    ui.text("Enable sensitivity overrides in individual modes on the Modes tab.");

    ui.dummy([0.0, 8.0]);
    for (i, mode) in config.modes.iter_mut().enumerate() {
        let lbl = if mode.id.is_empty() {
            format!("Mode {}", i)
        } else {
            mode.id.clone()
        };

        if ui.collapsing_header(
            format!("{lbl}##mouse_mode_{i}"),
            imgui::TreeNodeFlags::empty(),
        ) {
            if ui.checkbox(
                format!("Enable Override##mode_override_{i}"),
                &mut mode.sensitivity_override_enabled,
            ) {
                *dirty = true;
            }

            if mode.sensitivity_override_enabled {
                ui.set_next_item_width(-1.0);
                if crate::widgets::slider_float_log(ui, &format!("##mode_sens_{i}"), &mut mode.mode_sensitivity, 0.1, 20.0, "%.2fx") {
                    *dirty = true;
                }

                if ui.checkbox(
                    format!("Separate X/Y Sensitivity##sep_xy_{i}"),
                    &mut mode.separate_xy_sensitivity,
                ) {
                    *dirty = true;
                }

                if mode.separate_xy_sensitivity {
                    ui.text("X:");
                    ui.same_line();
                    ui.set_next_item_width(-1.0);
                    if crate::widgets::slider_float_log(ui, &format!("##mode_sens_x_{i}"), &mut mode.mode_sensitivity_x, 0.1, 20.0, "%.2fx") {
                        *dirty = true;
                    }
                    ui.text("Y:");
                    ui.same_line();
                    ui.set_next_item_width(-1.0);
                    if crate::widgets::slider_float_log(ui, &format!("##mode_sens_y_{i}"), &mut mode.mode_sensitivity_y, 0.1, 20.0, "%.2fx") {
                        *dirty = true;
                    }
                }
            }
        }
    }

    // sensitivity hotkeys - mostly a niche feature for testing
    ui.dummy([0.0, 16.0]);
    ui.separator(); ui.text("Sensitivity Hotkeys");

    let mut rm = None;
    for (i, hk) in config.input.sensitivity_hotkeys.iter_mut().enumerate() {
        if ui.collapsing_header(
            format!("Sensitivity Hotkey {i}##sens_hk_{i}"),
            imgui::TreeNodeFlags::empty(),
        ) {
            ui.text("Keys:");
            ui.same_line();
            let keys_str = hk.keys
                .iter()
                .map(|k| format!("{:#X}", k))
                .collect::<Vec<_>>()
                .join(", ");
            ui.text(&keys_str);

            ui.set_next_item_width(-1.0);
            if crate::widgets::slider_float_log(ui, &format!("##sens_hk_val_{i}"), &mut hk.sensitivity, 0.1, 20.0, "%.2fx") {
                *dirty = true;
            }

            if ui.checkbox(format!("Separate X/Y##sens_sep_{i}"), &mut hk.separate_xy) {
                *dirty = true;
            }
            if hk.separate_xy {
                ui.text("X:");
                ui.same_line();
                ui.set_next_item_width(-1.0);
                if crate::widgets::slider_float_log(ui, &format!("##sens_hk_x_{i}"), &mut hk.sensitivity_x, 0.1, 20.0, "%.2fx") {
                    *dirty = true;
                }
                ui.text("Y:");
                ui.same_line();
                ui.set_next_item_width(-1.0);
                if crate::widgets::slider_float_log(ui, &format!("##sens_hk_y_{i}"), &mut hk.sensitivity_y, 0.1, 20.0, "%.2fx") {
                    *dirty = true;
                }
            }

            if ui.checkbox(format!("Toggle##sens_toggle_{i}"), &mut hk.toggle) {
                *dirty = true;
            }

            ui.text("Debounce (ms):");
            ui.same_line();
            ui.set_next_item_width(120.0);
            if crate::widgets::slider_int(ui, &format!("##sens_debounce_{i}"), &mut hk.debounce, 0, 2000, "%d ms") {
                *dirty = true;
            }

            if ui.button(format!("Remove##sens_rm_{i}")) {
                rm = Some(i);
                *dirty = true;
            }
            if ui.is_item_hovered() {
                ui.tooltip_text("Delete this sensitivity hotkey");
            }
        }
    }

    if let Some(idx) = rm {
        config.input.sensitivity_hotkeys.remove(idx);
    }

    if ui.button("Add Sensitivity Hotkey") {
        config
            .input.sensitivity_hotkeys
            .push(tuxinjector_config::types::SensitivityHotkeyConfig::default());
        *dirty = true;
    }
}
