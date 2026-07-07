use tuxinjector_config::Config;

pub fn render(ui: &imgui::Ui, config: &mut Config, dirty: &mut bool) {
    let dbg = &mut config.advanced.debug;

    ui.separator();
    ui.text("Display");

    if ui.checkbox("Show Performance Overlay", &mut dbg.show_performance_overlay) {
        *dirty = true;
    }
    if dbg.show_performance_overlay {
        ui.text("Position:");
        ui.same_line();
        use tuxinjector_config::types::PerfOverlayPosition::*;
        let preview = format!("{:?}", dbg.perf_overlay_position);
        if let Some(_token) = ui.begin_combo("##perf_overlay_pos", &preview) {
            for v in [TopLeft, TopRight, BottomLeft, BottomRight] {
                let lbl = format!("{v:?}");
                if ui.selectable_config(&lbl)
                    .selected(dbg.perf_overlay_position == v)
                    .build()
                {
                    dbg.perf_overlay_position = v;
                    *dirty = true;
                }
            }
        }
    }
    if ui.checkbox("Show Profiler", &mut dbg.show_profiler) {
        *dirty = true;
    }
    if dbg.show_profiler {
        ui.text("Profiler Scale:");
        ui.same_line();
        if crate::widgets::slider_float(ui, "##profiler_scale", &mut dbg.profiler_scale, 0.1, 4.0, "%.1fx") {
            *dirty = true;
        }
    }
    if ui.checkbox("Show Hotkey Debug", &mut dbg.show_hotkey_debug) { *dirty = true; }
    if ui.checkbox("Show Texture Grid", &mut dbg.show_texture_grid) { *dirty = true; }

    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.text("Rendering");

    if ui.checkbox("Delay Rendering Until Finished", &mut dbg.delay_rendering_until_finished) {
        *dirty = true;
    }
    if ui.checkbox("Delay Rendering Until Blitted", &mut dbg.delay_rendering_until_blitted) {
        *dirty = true;
    }

    // log categories - two columns to save some vertical space
    // TODO: make sure all of these actually link to something 
    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.text("Log Categories");

    ui.columns(2, "log_cols", true);

    if ui.checkbox("Mode Switch", &mut dbg.log_mode_switch) { *dirty = true; }
    if ui.checkbox("Animation", &mut dbg.log_animation) { *dirty = true; }
    if ui.checkbox("Hotkey", &mut dbg.log_hotkey) { *dirty = true; }
    if ui.checkbox("Window Overlay", &mut dbg.log_window_overlay) { *dirty = true; }
    if ui.checkbox("File Monitor", &mut dbg.log_file_monitor) { *dirty = true; }

    ui.next_column();

    if ui.checkbox("Image Monitor", &mut dbg.log_image_monitor) { *dirty = true; }
    if ui.checkbox("Performance", &mut dbg.log_performance) { *dirty = true; }
    if ui.checkbox("Texture Ops", &mut dbg.log_texture_ops) { *dirty = true; }
    if ui.checkbox("GUI", &mut dbg.log_gui) { *dirty = true; }
    if ui.checkbox("Init", &mut dbg.log_init) { *dirty = true; }
    if ui.checkbox("Overlay", &mut dbg.log_overlay) { *dirty = true; }

    ui.columns(1, "log_cols_end", false);
}
