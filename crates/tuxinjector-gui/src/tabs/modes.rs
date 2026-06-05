use tuxinjector_config::types::{
    BackgroundTransitionType, GameTransitionType, ModeConfig, OverlayTransitionType,
};
use tuxinjector_config::Config;

pub fn render(
    ui: &imgui::Ui,
    config: &mut Config,
    dirty: &mut bool,
    selected: &mut Option<usize>,
) {
    if let Some(idx) = *selected {
        if idx >= config.modes.len() {
            *selected = None;
        }
    }

    ui.columns(2, "mode_cols", true);

    ui.text("Mode List");
    ui.separator();

    for (i, mode) in config.modes.iter().enumerate() {
        let lbl = if mode.id.is_empty() {
            format!("Mode {}", i)
        } else {
            mode.id.clone()
        };
        if ui.selectable_config(&lbl)
            .selected(*selected == Some(i))
            .build()
        {
            *selected = Some(i);
        }
    }

    ui.dummy([0.0, 8.0]);
    if ui.button("Add Mode") {
        config.modes.push(ModeConfig::default());
        *selected = Some(config.modes.len() - 1);
        *dirty = true;
    }

    ui.next_column();

    if let Some(idx) = *selected {
        if idx < config.modes.len() {
            mode_editor(ui, &mut config.modes[idx], dirty);

            ui.dummy([0.0, 12.0]);
            if ui.button("Remove Mode") {
                config.modes.remove(idx);
                *selected = None;
                *dirty = true;
            }
        }
    } else {
        ui.text("Select a mode to edit.");
    }

    ui.columns(1, "mode_cols_end", false);
}

fn mode_editor(ui: &imgui::Ui, mode: &mut ModeConfig, dirty: &mut bool) {
    ui.text("ID:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    if ui.input_text("##mode_id", &mut mode.id).build() {
        *dirty = true;
    }

    // -- size --
    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Size");

    if ui.checkbox("Use Relative Size##mode_relsize", &mut mode.use_relative_size) {
        *dirty = true;
    }

    if mode.use_relative_size {
        ui.text("Relative Width:");
        ui.same_line();
        ui.set_next_item_width(200.0);
        if crate::widgets::slider_float(ui, "##mode_rw", &mut mode.relative_width, 0.0, 1.0, "%.2f") {
            *dirty = true;
        }

        ui.text("Relative Height:");
        ui.same_line();
        ui.set_next_item_width(200.0);
        if crate::widgets::slider_float(ui, "##mode_rh", &mut mode.relative_height, 0.0, 1.0, "%.2f") {
            *dirty = true;
        }
    }

    if !mode.use_relative_size {
        ui.text("Width:");
        ui.same_line();
        ui.set_next_item_width(100.0);
        if crate::widgets::slider_int(ui, "##mode_w", &mut mode.width, 0, 32768, "%d px") {
            *dirty = true;
        }
        ui.same_line();
        ui.text("Height:");
        ui.same_line();
        ui.set_next_item_width(100.0);
        if crate::widgets::slider_int(ui, "##mode_h", &mut mode.height, 0, 32768, "%d px") {
            *dirty = true;
        }

        if mode.width > 16384 || mode.height > 16384 {
            crate::widgets::text_wrapped_colored(
                ui,
                [1.0, 0.31, 0.31, 1.0],
                "WARNING: USING ANY RESOLUTIONS ABOVE 16384 IN A RUN IS NOT SPEEDRUN.COM LEGAL", 
                // i dont really like the wording i used here, maybe change this?
            );
        }
    }

    // expression overrides for width/height
    ui.text("Width Expr:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    if ui.input_text("##mode_wexpr", &mut mode.width_expr).build() {
        *dirty = true;
    }

    ui.text("Height Expr:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    if ui.input_text("##mode_hexpr", &mut mode.height_expr).build() {
        *dirty = true;
    }

    if !mode.width_expr.is_empty() || !mode.height_expr.is_empty() {
        crate::widgets::text_wrapped_colored(
            ui,
            [0.5, 0.5, 0.5, 1.0],
            "Expressions override Width/Height when set",
        );
    }

    // -- transitions --
    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Transitions");

    let game_cur = format!("{:?}", mode.game_transition);
    ui.text("Game Transition:");
    ui.same_line();
    if let Some(_token) = ui.begin_combo("##game_transition", &game_cur) {
        for t in &[GameTransitionType::Cut, GameTransitionType::Bounce] {
            let lbl = format!("{:?}", t);
            if ui.selectable_config(&lbl)
                .selected(mode.game_transition == *t)
                .build()
            {
                mode.game_transition = *t;
                *dirty = true;
            }
        }
    }

    let overlay_cur = format!("{:?}", mode.overlay_transition);
    ui.text("Overlay Transition:");
    ui.same_line();
    if let Some(_token) = ui.begin_combo("##overlay_transition", &overlay_cur) {
        for t in &[OverlayTransitionType::Cut] {
            let lbl = format!("{:?}", t);
            if ui.selectable_config(&lbl)
                .selected(mode.overlay_transition == *t)
                .build()
            {
                mode.overlay_transition = *t;
                *dirty = true;
            }
        }
    }

    let bg_cur = format!("{:?}", mode.background_transition);
    ui.text("Background Transition:");
    ui.same_line();
    if let Some(_token) = ui.begin_combo("##bg_transition", &bg_cur) {
        for t in &[BackgroundTransitionType::Cut] {
            let lbl = format!("{:?}", t);
            if ui.selectable_config(&lbl)
                .selected(mode.background_transition == *t)
                .build()
            {
                mode.background_transition = *t;
                *dirty = true;
            }
        }
    }

    ui.text("Duration (ms):");
    ui.same_line();
    ui.set_next_item_width(80.0);
    if crate::widgets::slider_int(ui, "##mode_tdur", &mut mode.transition_duration_ms, 0, 5000, "%d ms") {
        *dirty = true;
    }

    ui.dummy([0.0, 4.0]);
    ui.text("Ease In Power:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    if crate::widgets::slider_float(ui, "##mode_eip", &mut mode.ease_in_power, 0.1, 10.0, "%.1f") {
        *dirty = true;
    }

    ui.text("Ease Out Power:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    if crate::widgets::slider_float(ui, "##mode_eop", &mut mode.ease_out_power, 0.1, 10.0, "%.1f") {
        *dirty = true;
    }

    ui.text("Bounce Count:");
    ui.same_line();
    ui.set_next_item_width(80.0);
    if crate::widgets::slider_int(ui, "##mode_bc", &mut mode.bounce_count, 0, 20, "%d") {
        *dirty = true;
    }

    ui.text("Bounce Intensity:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    if crate::widgets::slider_float(ui, "##mode_bi", &mut mode.bounce_intensity, 0.0, 1.0, "%.2f") {
        *dirty = true;
    }

    ui.text("Bounce Duration (ms):");
    ui.same_line();
    ui.set_next_item_width(80.0);
    if crate::widgets::slider_int(ui, "##mode_bd", &mut mode.bounce_duration_ms, 0, 2000, "%d ms") {
        *dirty = true;
    }

    // -- animation flags --
    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Animation Flags");

    if ui.checkbox("Relative Stretching##mode_relstretch", &mut mode.relative_stretching) {
        *dirty = true;
    }
    if ui.checkbox("Skip Animate X##mode_skipx", &mut mode.skip_animate_x) {
        *dirty = true;
    }
    if ui.checkbox("Skip Animate Y##mode_skipy", &mut mode.skip_animate_y) {
        *dirty = true;
    }
    if ui.checkbox("Slide Mirrors In##mode_slidemirrors", &mut mode.slide_mirrors_in) {
        *dirty = true;
    }

    // -- sub-editors --
    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Background");
    bg_editor(ui, &mut mode.background, dirty);

    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Border");
    border_editor(ui, &mut mode.border, dirty);

    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Stretch");
    stretch_editor(ui, &mut mode.stretch, dirty);

    // overlay ID lists
    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Overlays");

    string_list(ui, "Mirrors", &mut mode.mirror_ids, "mirror", dirty);
    string_list(ui, "Mirror Groups", &mut mode.mirror_group_ids, "mirror_group", dirty);
    string_list(ui, "Images", &mut mode.image_ids, "image", dirty);
    string_list(ui, "Window Overlays", &mut mode.window_overlay_ids, "window_overlay", dirty);
    string_list(ui, "Text Overlays", &mut mode.text_overlay_ids, "text_overlay", dirty);

    // -- sensitivity override --
    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Sensitivity Override");

    if ui.checkbox("Enable Sensitivity Override##mode_sensov", &mut mode.sensitivity_override_enabled) {
        *dirty = true;
    }

    if mode.sensitivity_override_enabled {
        ui.text("Sensitivity:");
        ui.same_line();
        ui.set_next_item_width(200.0);
        if crate::widgets::slider_float(ui, "##mode_sens", &mut mode.mode_sensitivity, 0.1, 5.0, "%.2fx") {
            *dirty = true;
        }

        if ui.checkbox("Separate X/Y##mode_sepxy", &mut mode.separate_xy_sensitivity) {
            *dirty = true;
        }

        if mode.separate_xy_sensitivity {
            ui.text("X:");
            ui.same_line();
            ui.set_next_item_width(200.0);
            if crate::widgets::slider_float(ui, "##mode_sensx", &mut mode.mode_sensitivity_x, 0.1, 5.0, "%.2fx") {
                *dirty = true;
            }
            ui.text("Y:");
            ui.same_line();
            ui.set_next_item_width(200.0);
            if crate::widgets::slider_float(ui, "##mode_sensy", &mut mode.mode_sensitivity_y, 0.1, 5.0, "%.2fx") {
                *dirty = true;
            }
        }
    }
}

// editable list of string IDs (mirrors, images, etc)
fn string_list(
    ui: &imgui::Ui,
    label: &str,
    items: &mut Vec<String>,
    prefix: &str,
    dirty: &mut bool,
) {
    ui.text(label);
    let mut rm = None;
    for (i, item) in items.iter_mut().enumerate() {
        ui.set_next_item_width(160.0);
        if ui.input_text(format!("##{}_{}", prefix, i), item).build() {
            *dirty = true;
        }
        ui.same_line();
        if ui.small_button(format!("X##{}_{}_rm", prefix, i)) {
            rm = Some(i);
            *dirty = true;
        }
    }
    if let Some(idx) = rm {
        items.remove(idx);
    }
    if ui.small_button(format!("Add {}##{}_add", label, prefix)) {
        items.push(String::new());
        *dirty = true;
    }
}

fn bg_editor(
    ui: &imgui::Ui,
    bg: &mut tuxinjector_config::types::BackgroundConfig,
    dirty: &mut bool,
) {
    ui.text("Mode:");
    ui.same_line();
    if let Some(_token) = ui.begin_combo("##bg_mode", &bg.selected_mode) {
        for m in &["none", "color", "image", "gradient"] {
            if ui.selectable_config(*m)
                .selected(bg.selected_mode == *m)
                .build()
            {
                bg.selected_mode = (*m).to_string();
                *dirty = true;
            }
        }
    }

    if bg.selected_mode == "color" {
        let mut rgba = [bg.color.r, bg.color.g, bg.color.b, bg.color.a];
        ui.text("Color:");
        ui.same_line();
        if ui.color_edit4("##bg_color", &mut rgba) {
            bg.color.r = rgba[0];
            bg.color.g = rgba[1];
            bg.color.b = rgba[2];
            bg.color.a = rgba[3];
            *dirty = true;
        }
    } else if bg.selected_mode == "image" {
        ui.text("Image:");
        ui.same_line();
        ui.set_next_item_width(200.0);
        if ui.input_text("##bg_image_path", &mut bg.image).build() {
            *dirty = true;
        }
        ui.text("(absolute path or ~/...; relative names won't resolve)");

        ui.text("Fit:");
        ui.same_line();
        ui.set_next_item_width(200.0);
        let fits = [
            ("fill", "Fill (cover, crop)"),
            ("fit", "Fit (letterbox)"),
            ("stretch", "Stretch"),
            ("center", "Center"),
        ];
        let cur = fits.iter().find(|(k, _)| *k == bg.image_fit)
            .map(|(_, label)| *label)
            .unwrap_or("Fill (cover, crop)");
        if let Some(_token) = ui.begin_combo("##bg_image_fit", cur) {
            for (key, label) in fits {
                if ui.selectable_config(label)
                    .selected(bg.image_fit == key)
                    .build()
                {
                    bg.image_fit = key.to_string();
                    *dirty = true;
                }
            }
        }

        if bg.image_fit == "fit" || bg.image_fit == "center" {
            let mut rgba = [bg.color.r, bg.color.g, bg.color.b, bg.color.a];
            ui.text("Matte:");
            ui.same_line();
            if ui.color_edit4("##bg_image_matte", &mut rgba) {
                bg.color.r = rgba[0];
                bg.color.g = rgba[1];
                bg.color.b = rgba[2];
                bg.color.a = rgba[3];
                *dirty = true;
            }
        }
    } else if bg.selected_mode == "gradient" {
        use tuxinjector_config::types::GradientColorStop;

        // need at least 2 stops
        while bg.gradient_stops.len() < 2 {
            bg.gradient_stops.push(GradientColorStop::default());
        }
        if bg.gradient_stops.len() >= 2 && bg.gradient_stops[1].position == 0.0 {
            bg.gradient_stops[1].position = 1.0;
        }

        let c0 = &mut bg.gradient_stops[0].color;
        let mut rgba0 = [c0.r, c0.g, c0.b, c0.a];
        ui.text("Color 1:");
        ui.same_line();
        if ui.color_edit4("##bg_grad_c1", &mut rgba0) {
            c0.r = rgba0[0]; c0.g = rgba0[1];
            c0.b = rgba0[2]; c0.a = rgba0[3];
            *dirty = true;
        }

        let c1 = &mut bg.gradient_stops[1].color;
        let mut rgba1 = [c1.r, c1.g, c1.b, c1.a];
        ui.text("Color 2:");
        ui.same_line();
        if ui.color_edit4("##bg_grad_c2", &mut rgba1) {
            c1.r = rgba1[0]; c1.g = rgba1[1];
            c1.b = rgba1[2]; c1.a = rgba1[3];
            *dirty = true;
        }

        ui.text("Angle:");
        ui.same_line();
        ui.set_next_item_width(200.0);
        if crate::widgets::slider_float(ui, "##bg_grad_angle", &mut bg.gradient_angle, 0.0, 360.0, "%.0f deg") {
            *dirty = true;
        }

        ui.text("Animation Speed:");
        ui.same_line();
        ui.set_next_item_width(200.0);
        if crate::widgets::slider_float(ui, "##bg_grad_speed", &mut bg.gradient_animation_speed, 0.0, 10.0, "%.1f") {
            *dirty = true;
        }

        if ui.checkbox("Color Fade##bg_grad_fade", &mut bg.gradient_color_fade) {
            *dirty = true;
        }
    }
}

fn border_editor(
    ui: &imgui::Ui,
    border: &mut tuxinjector_config::types::BorderConfig,
    dirty: &mut bool,
) {
    if ui.checkbox("Enable Border##mode_brd", &mut border.enabled) {
        *dirty = true;
    }

    if border.enabled {
        let mut rgba = [border.color.r, border.color.g, border.color.b, border.color.a];
        ui.text("Color:");
        ui.same_line();
        if ui.color_edit4("##mode_brd_color", &mut rgba) {
            border.color.r = rgba[0];
            border.color.g = rgba[1];
            border.color.b = rgba[2];
            border.color.a = rgba[3];
            *dirty = true;
        }

        ui.text("Width:");
        ui.same_line();
        ui.set_next_item_width(80.0);
        if crate::widgets::slider_int(ui, "##mode_brd_w", &mut border.width, 0, 100, "%d px") {
            *dirty = true;
        }

        ui.text("Radius:");
        ui.same_line();
        ui.set_next_item_width(80.0);
        if crate::widgets::slider_int(ui, "##mode_brd_r", &mut border.radius, 0, 200, "%d px") {
            *dirty = true;
        }
    }
}

fn stretch_editor(
    ui: &imgui::Ui,
    stretch: &mut tuxinjector_config::types::StretchConfig,
    dirty: &mut bool,
) {
    if ui.checkbox("Enable Stretch##mode_stretch", &mut stretch.enabled) {
        *dirty = true;
    }

    if stretch.enabled {
        ui.text("X:");
        ui.same_line();
        ui.set_next_item_width(80.0);
        if crate::widgets::slider_int(ui, "##stretch_x", &mut stretch.x, -10000, 10000, "%d") {
            *dirty = true;
        }
        ui.same_line();
        ui.text("Y:");
        ui.same_line();
        ui.set_next_item_width(80.0);
        if crate::widgets::slider_int(ui, "##stretch_y", &mut stretch.y, -10000, 10000, "%d") {
            *dirty = true;
        }

        ui.text("Width:");
        ui.same_line();
        ui.set_next_item_width(100.0);
        if crate::widgets::slider_int(ui, "##stretch_w", &mut stretch.width, 0, 32768, "%d px") {
            *dirty = true;
        }
        ui.same_line();
        ui.text("Height:");
        ui.same_line();
        ui.set_next_item_width(100.0);
        if crate::widgets::slider_int(ui, "##stretch_h", &mut stretch.height, 0, 32768, "%d px") {
            *dirty = true;
        }

        ui.text("Width Expr:");
        ui.same_line();
        ui.set_next_item_width(120.0);
        if ui.input_text("##stretch_wexpr", &mut stretch.width_expr).build() {
            *dirty = true;
        }

        ui.text("Height Expr:");
        ui.same_line();
        ui.set_next_item_width(120.0);
        if ui.input_text("##stretch_hexpr", &mut stretch.height_expr).build() {
            *dirty = true;
        }

        ui.text("X Expr:");
        ui.same_line();
        ui.set_next_item_width(120.0);
        if ui.input_text("##stretch_xexpr", &mut stretch.x_expr).build() {
            *dirty = true;
        }

        ui.text("Y Expr:");
        ui.same_line();
        ui.set_next_item_width(120.0);
        if ui.input_text("##stretch_yexpr", &mut stretch.y_expr).build() {
            *dirty = true;
        }
    }
}
