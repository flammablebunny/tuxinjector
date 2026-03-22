use tuxinjector_config::types::BrowserOverlayConfig;
use tuxinjector_config::Config;

/// Returns true if changes should be applied immediately (live preview)
pub fn render(
    ui: &imgui::Ui,
    config: &mut Config,
    dirty: &mut bool,
    selected: &mut Option<usize>,
) -> bool {
    let mut live_changed = false;
    if let Some(idx) = *selected {
        if idx >= config.overlays.browser_overlays.len() {
            *selected = None;
        }
    }

    ui.columns(2, "bo_cols", true);

    ui.text("Browser Overlay List");
    ui.separator();

    for (i, bo) in config.overlays.browser_overlays.iter().enumerate() {
        let lbl = if bo.name.is_empty() {
            format!("Browser {}", i)
        } else {
            bo.name.clone()
        };
        if ui.selectable_config(&lbl)
            .selected(*selected == Some(i))
            .build()
        {
            *selected = Some(i);
        }
    }

    ui.dummy([0.0, 8.0]);
    if ui.button("Add Browser Overlay") {
        config.overlays.browser_overlays.push(BrowserOverlayConfig::default());
        *selected = Some(config.overlays.browser_overlays.len() - 1);
        *dirty = true;
    }

    ui.next_column();

    if let Some(idx) = *selected {
        if idx < config.overlays.browser_overlays.len() {
            live_changed |= bo_editor(ui, &mut config.overlays.browser_overlays[idx], idx, dirty);
            ui.dummy([0.0, 12.0]);
            if ui.button("Remove Browser Overlay") {
                config.overlays.browser_overlays.remove(idx);
                *selected = None;
                *dirty = true;
            }
        }
    } else {
        ui.text("Select a browser overlay to edit.");
    }

    ui.columns(1, "bo_cols_end", false);
    live_changed
}

fn bo_editor(
    ui: &imgui::Ui,
    bo: &mut BrowserOverlayConfig,
    idx: usize,
    dirty: &mut bool,
) -> bool {
    let mut live = false;
    ui.text("Name:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    if ui.input_text(format!("##bo_name_{idx}"), &mut bo.name).build() {
        *dirty = true;
    }

    ui.text("URL:");
    ui.same_line();
    ui.set_next_item_width(-1.0);
    if ui.input_text(format!("##bo_url_{idx}"), &mut bo.url)
        .hint("https://example.com")
        .build()
    {
        *dirty = true;
    }

    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Dimensions");

    ui.text("Width:");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if crate::widgets::slider_int(ui, &format!("##bo_w_{idx}"), &mut bo.width, 100, 1920, "%d px") {
        *dirty = true;
    }
    ui.same_line();
    ui.text("Height:");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if crate::widgets::slider_int(ui, &format!("##bo_h_{idx}"), &mut bo.height, 100, 1080, "%d px") {
        *dirty = true;
    }

    ui.text("FPS:");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if crate::widgets::slider_int(ui, &format!("##bo_fps_{idx}"), &mut bo.fps, 1, 60, "%d") {
        *dirty = true;
    }

    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Rendering");

    ui.text("X:");
    ui.same_line();
    ui.set_next_item_width(80.0);
    if crate::widgets::slider_int(ui, &format!("##bo_x_{idx}"), &mut bo.x, -200, 1920, "%d") {
        *dirty = true; live = true;
    }
    ui.same_line();
    ui.text("Y:");
    ui.same_line();
    ui.set_next_item_width(80.0);
    if crate::widgets::slider_int(ui, &format!("##bo_y_{idx}"), &mut bo.y, -200, 1080, "%d") {
        *dirty = true; live = true;
    }

    ui.text("Scale:");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if crate::widgets::slider_float(ui, &format!("##bo_scale_{idx}"), &mut bo.scale, 0.1, 5.0, "%.2fx") {
        *dirty = true; live = true;
    }
    ui.same_line();
    ui.text("Opacity:");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if crate::widgets::slider_float(ui, &format!("##bo_opacity_{idx}"), &mut bo.opacity, 0.0, 1.0, "%.2f") {
        *dirty = true; live = true;
    }

    if ui.checkbox(format!("Transparent Background##bo_transp_{idx}"), &mut bo.transparent_background) {
        *dirty = true;
    }
    if ui.checkbox(format!("Pixelated Scaling##bo_pix_{idx}"), &mut bo.pixelated_scaling) {
        *dirty = true;
    }

    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Crop");

    ui.text("Top:");
    ui.same_line();
    ui.set_next_item_width(60.0);
    if crate::widgets::slider_int(ui, &format!("##bo_ct_{idx}"), &mut bo.crop_top, 0, 500, "%d") {
        *dirty = true;
    }
    ui.same_line();
    ui.text("Bottom:");
    ui.same_line();
    ui.set_next_item_width(60.0);
    if crate::widgets::slider_int(ui, &format!("##bo_cb_{idx}"), &mut bo.crop_bottom, 0, 500, "%d") {
        *dirty = true;
    }
    ui.same_line();
    ui.text("Left:");
    ui.same_line();
    ui.set_next_item_width(60.0);
    if crate::widgets::slider_int(ui, &format!("##bo_cl_{idx}"), &mut bo.crop_left, 0, 500, "%d") {
        *dirty = true;
    }
    ui.same_line();
    ui.text("Right:");
    ui.same_line();
    ui.set_next_item_width(60.0);
    if crate::widgets::slider_int(ui, &format!("##bo_cr_{idx}"), &mut bo.crop_right, 0, 500, "%d") {
        *dirty = true;
    }

    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Custom CSS");
    ui.set_next_item_width(-1.0);
    if ui.input_text_multiline(
        format!("##bo_css_{idx}"),
        &mut bo.custom_css,
        [0.0, 80.0],
    ).build() {
        *dirty = true;
    }

    live
}
