use tuxinjector_config::Config;

const THEMES: &[(&str, &str)] = &[
    ("Purple", "Deep purple tint -- tuxinjector default"),
    ("Dracula", "Classic Dracula palette -- blue-purple"),
    ("Catppuccin", "Catppuccin Mocha -- soft lavender"),
];

pub fn render(ui: &imgui::Ui, config: &mut Config, dirty: &mut bool) {
    ui.separator();
    ui.text("Theme");

    ui.dummy([0.0, 4.0]);
    for &(name, _desc) in THEMES {
        let sel = config.theme.appearance.theme == name;
        if ui.selectable_config(name)
            .selected(sel)
            .size([0.0, 0.0])
            .build()
        {
            if !sel {
                config.theme.appearance.theme = name.to_string();
                *dirty = true;
            }
        }
        ui.same_line();
    }
    ui.new_line();

    if let Some(&(_, desc)) = THEMES.iter().find(|&&(n, _)| n == config.theme.appearance.theme) {
        ui.dummy([0.0, 2.0]);
        ui.text_disabled(desc);
    }

    // // -- Custom color overrides --
    // ui.dummy([0.0, 12.0]);
    // ui.separator();
    // ui.text("Custom Colors");
    // ui.text_disabled("Override specific UI color slots (advanced).");
    // ... (disabled - no functionality yet)

    // -- game gui scale (affects pie chart positioning) --
    ui.dummy([0.0, 16.0]);
    ui.separator();
    ui.text("Game GUI Scale");
    ui.text_disabled("Minecraft GUI scale - affects pie chart anchor offsets.");
    ui.dummy([0.0, 4.0]);
    ui.text("Scale:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    let cur_gs = config.display.game_gui_scale.to_string();
    if let Some(_token) = ui.begin_combo("##game_gui_scale", &cur_gs) {
        for scale in 1u32..=8 {
            let lbl = scale.to_string();
            if ui.selectable_config(&lbl)
                .selected(config.display.game_gui_scale == scale)
                .build()
            {
                config.display.game_gui_scale = scale;
                *dirty = true;
            }
        }
    }

    // -- Mirror gamma --
    ui.dummy([0.0, 16.0]);
    ui.separator();
    ui.text("Mirror Gamma Mode");
    ui.dummy([0.0, 4.0]);
    ui.text("Gamma:");
    ui.same_line();
    let cur = format!("{:?}", config.display.mirror_gamma_mode);
    if let Some(_token) = ui.begin_combo("##gamma_mode", &cur) {
        use tuxinjector_config::types::MirrorGammaMode;
        for mode in &[
            MirrorGammaMode::Auto,
            MirrorGammaMode::AssumeSrgb,
            MirrorGammaMode::AssumeLinear,
        ] {
            let lbl = format!("{:?}", mode);
            if ui.selectable_config(&lbl)
                .selected(config.display.mirror_gamma_mode == *mode)
                .build()
            {
                config.display.mirror_gamma_mode = *mode;
                *dirty = true;
            }
        }
    }
}
