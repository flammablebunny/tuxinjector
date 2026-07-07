// Settings tab for the Ninjabrain Bot API overlay. Ported straight from
// toolscreen's tab_ninjabrain_overlay.inl - same widgets, labels and ranges.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tuxinjector_config::types::{default_nbb_columns, NinjabrainOverlayConfig};
use tuxinjector_config::Config;
use tuxinjector_core::color::Color;

// The injector crate owns the actual connection; it pushes status in here and
// drains the restart flag back out. Keeps the GUI free of any socket state.

#[derive(Clone, Default)]
pub struct NbbStatusView {
    /// "stopped" | "connecting" | "connected" | "offline"
    pub state: String,
    pub api_base_url: String,
    pub last_error: String,
    pub overlay_visible: bool,
}

static STATUS: Mutex<Option<NbbStatusView>> = Mutex::new(None);
static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn publish_status(s: NbbStatusView) {
    *STATUS.lock().unwrap() = Some(s);
}

/// Injector crate polls this per frame; true = user asked for a reconnect.
pub fn take_restart_request() -> bool {
    RESTART_REQUESTED.swap(false, Ordering::Relaxed)
}

pub fn render(ui: &imgui::Ui, config: &mut Config, dirty: &mut bool) {
    let nb = &mut config.overlays.ninjabrain;

    if ui.checkbox("Enable overlay", &mut nb.enabled) {
        *dirty = true;
    }
    ui.text_disabled("Renders Ninjabrain Bot's calculator output as a game overlay (API).");
    ui.text_disabled("The embedded Ninjabrain Bot app in the Apps tab is separate.");

    if !nb.enabled {
        return;
    }

    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.text("API");

    let status = STATUS.lock().unwrap().clone().unwrap_or_default();
    ui.text("Status:");
    ui.same_line();
    let (label, color) = match status.state.as_str() {
        "connected" => ("Connected", [0.35, 0.85, 0.35, 1.0]),
        "connecting" => ("Connecting...", [1.0, 0.8, 0.35, 1.0]),
        "offline" => ("Offline", [1.0, 0.45, 0.45, 1.0]),
        _ => ("Stopped", [0.65, 0.65, 0.65, 1.0]),
    };
    ui.text_colored(color, label);

    ui.text("API address:");
    ui.same_line();
    ui.set_next_item_width(240.0);
    if ui.input_text("##nbb_api_url", &mut nb.api_base_url).build() {
        *dirty = true;
    }

    if status.state == "offline" {
        crate::widgets::text_wrapped_colored(
            ui,
            [1.0, 0.6, 0.6, 1.0],
            &format!(
                "Could not reach {}. Start Ninjabrain Bot, then go to Settings -> Advanced -> Enable API. Last error: {}",
                status.api_base_url, status.last_error
            ),
        );
    } else if status.state == "connecting" {
        ui.text_disabled(&format!("Connecting to {}...", status.api_base_url));
    }
    if status.state != "connected" {
        if ui.button("Retry connection") {
            RESTART_REQUESTED.store(true, Ordering::Relaxed);
        }
        ui.text_disabled("In Ninjabrain Bot: Settings -> Advanced -> Enable API.");
    }
    if !status.overlay_visible {
        ui.text_colored([1.0, 0.8, 0.35, 1.0], "Overlay currently hidden by hotkey.");
    }

    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.text("Show in modes");
    // Empty allowed_modes means "everywhere", so the checkbox is inverted.
    let mut all_modes = nb.allowed_modes.is_empty();
    if ui.checkbox("All modes", &mut all_modes) {
        if all_modes {
            nb.allowed_modes.clear();
        } else {
            nb.allowed_modes = config.modes.iter().map(|m| m.id.clone()).collect();
        }
        *dirty = true;
    }
    if !nb.allowed_modes.is_empty() {
        for mode in &config.modes {
            let mut on = nb.allowed_modes.contains(&mode.id);
            if ui.checkbox(format!("{}##nbbmode", mode.id), &mut on) {
                if on {
                    nb.allowed_modes.push(mode.id.clone());
                } else {
                    nb.allowed_modes.retain(|m| m != &mode.id);
                }
                *dirty = true;
            }
        }
    }

    // --- Presets ---
    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.text("Presets");
    ui.text_disabled("Apply a ready-made look. Placement, API address and modes are kept for Compact.");
    if ui.button("Compact") {
        ui.open_popup("Apply Compact Preset");
    }
    ui.same_line();
    if ui.button("Ninjabrain Bot") {
        ui.open_popup("Apply Ninjabrain Bot Preset");
    }
    if let Some(_p) = ui.begin_popup("Apply Compact Preset") {
        ui.text("Replace the overlay settings with the Compact preset?");
        if ui.button("Apply##compact") {
            apply_compact_preset(nb);
            *dirty = true;
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel##compact") {
            ui.close_current_popup();
        }
    }
    if let Some(_p) = ui.begin_popup("Apply Ninjabrain Bot Preset") {
        ui.text("Replace the overlay settings with the Ninjabrain Bot preset?");
        if ui.button("Apply##nbbot") {
            apply_nbbot_preset(nb);
            *dirty = true;
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel##nbbot") {
            ui.close_current_popup();
        }
    }

    // --- Rendering ---
    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.text("Rendering");

    let mut pct = nb.overlay_opacity * 100.0;
    ui.text("Opacity:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    if crate::widgets::slider_float(ui, "##nbb_opacity", &mut pct, 0.0, 100.0, "%.0f%%") {
        nb.overlay_opacity = pct / 100.0;
        *dirty = true;
    }
    let mut bgpct = nb.bg_opacity * 100.0;
    ui.text("BG Opacity:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    if crate::widgets::slider_float(ui, "##nbb_bg_opacity", &mut bgpct, 0.0, 100.0, "%.0f%%") {
        nb.bg_opacity = bgpct / 100.0;
        nb.bg_enabled = nb.bg_opacity > 0.0;
        *dirty = true;
    }
    let mut scale_pct = nb.overlay_scale * 100.0;
    ui.text("Scale:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    if crate::widgets::slider_float(ui, "##nbb_scale", &mut scale_pct, 5.0, 100.0, "%.0f%%") {
        nb.overlay_scale = scale_pct / 100.0;
        *dirty = true;
    }

    if ui.checkbox("Hide when stale", &mut nb.hide_if_stale) {
        *dirty = true;
    }
    if nb.hide_if_stale {
        ui.same_line();
        ui.set_next_item_width(100.0);
        if crate::widgets::slider_int(ui, "##nbb_stale_delay", &mut nb.hide_if_stale_delay_seconds, 1, 3600, "%d s") {
            *dirty = true;
        }
    }

    ui.text("Position X:");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if imgui::Drag::new("##nbb_x").build(ui, &mut nb.x) {
        *dirty = true;
    }
    ui.same_line();
    ui.text("Y:");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if imgui::Drag::new("##nbb_y").build(ui, &mut nb.y) {
        *dirty = true;
    }
    ui.text("Anchor:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    const ANCHORS: [&str; 5] = [
        "topLeftScreen", "topRightScreen", "bottomLeftScreen", "bottomRightScreen", "centerScreen",
    ];
    if let Some(_c) = ui.begin_combo("##nbb_anchor", &nb.relative_to) {
        for a in ANCHORS {
            if ui.selectable_config(a).selected(nb.relative_to == a).build() {
                nb.relative_to = a.to_string();
                *dirty = true;
            }
        }
    }
    ui.text("Font path:");
    ui.same_line();
    ui.set_next_item_width(280.0);
    if ui.input_text("##nbb_font", &mut nb.custom_font_path).build() {
        *dirty = true;
    }
    ui.text_disabled("Absolute .ttf/.otf path. Empty = bundled OpenSans (matches toolscreen).");

    // --- Appearance ---
    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.text("Appearance");

    ui.text("Predictions:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, "##nbb_preds", &mut nb.shown_predictions, 1, 5, "%d") {
        *dirty = true;
    }
    ui.text("Eye throw rows:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, "##nbb_throw_rows", &mut nb.eye_throw_rows, 1, 8, "%d") {
        *dirty = true;
    }
    ui.text("Coordinates:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    let coords_label = if nb.coords_display == "block" { "Block coords" } else { "Chunk" };
    if let Some(_c) = ui.begin_combo("##nbb_coords", coords_label) {
        if ui.selectable_config("Chunk").selected(nb.coords_display == "chunk").build() {
            nb.coords_display = "chunk".into();
            rename_coords_column(nb, "Chunk");
            *dirty = true;
        }
        if ui.selectable_config("Block coords").selected(nb.coords_display == "block").build() {
            nb.coords_display = "block".into();
            rename_coords_column(nb, "Location");
            *dirty = true;
        }
    }
    ui.text("Text outline:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, "##nbb_outline", &mut nb.outline_width, 0, 5, "%d px") {
        *dirty = true;
    }

    if ui.checkbox("Always show", &mut nb.always_show) {
        *dirty = true;
    }
    if ui.checkbox("Show direction to stronghold", &mut nb.show_direction_to_stronghold) {
        *dirty = true;
    }
    if ui.checkbox("Show eye throw details", &mut nb.show_throw_details) {
        *dirty = true;
    }
    if ui.checkbox("Show boat state in top bar", &mut nb.show_boat_state_in_top_bar) {
        *dirty = true;
    }
    if nb.show_boat_state_in_top_bar {
        if ui.checkbox("Always show boat", &mut nb.always_show_boat) {
            *dirty = true;
        }
        ui.text("Boat icon size:");
        ui.same_line();
        ui.set_next_item_width(120.0);
        if crate::widgets::slider_float(ui, "##nbb_boat_size", &mut nb.boat_state_size, 8.0, 96.0, "%.0f px") {
            *dirty = true;
        }
    }
    if ui.checkbox("Anti-aliased text", &mut nb.font_antialiasing) {
        *dirty = true;
    }
    if ui.checkbox("Static column widths", &mut nb.static_column_widths) {
        *dirty = true;
    }

    ui.text("Border width:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, "##nbb_border_w", &mut nb.border_width, 0, 8, "%d px") {
        *dirty = true;
    }
    ui.text("Corner radius:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_float(ui, "##nbb_corner", &mut nb.corner_radius, 0.0, 32.0, "%.0f px") {
        *dirty = true;
    }
    ui.text("Row spacing:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_float(ui, "##nbb_row_spacing", &mut nb.row_spacing, 0.0, 30.0, "%.0f") {
        *dirty = true;
    }
    ui.text("Side padding:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_float(ui, "##nbb_side_pad", &mut nb.side_padding, 0.0, 300.0, "%.0f") {
        *dirty = true;
    }

    // --- Colors ---
    if ui.collapsing_header("Colors", imgui::TreeNodeFlags::empty()) {
        let mut edit = |label: &str, c: &mut Color, dirty: &mut bool| {
            let mut v = [c.r, c.g, c.b, c.a];
            if ui.color_edit4(label, &mut v) {
                (c.r, c.g, c.b, c.a) = (v[0], v[1], v[2], v[3]);
                *dirty = true;
            }
        };
        edit("Background", &mut nb.bg_color, dirty);
        edit("Header fill", &mut nb.header_fill_color, dirty);
        edit("Border", &mut nb.border_color, dirty);
        edit("Divider", &mut nb.divider_color, dirty);
        edit("Header divider", &mut nb.header_divider_color, dirty);
        edit("Text outline", &mut nb.outline_color, dirty);
        edit("Header text", &mut nb.text_color, dirty);
        edit("Data text", &mut nb.data_color, dirty);
        edit("Throws text", &mut nb.throws_text_color, dirty);
        edit("Divine text", &mut nb.divine_text_color, dirty);
        edit("Throws background", &mut nb.throws_background_color, dirty);
        edit("Coord positive", &mut nb.coord_positive_color, dirty);
        edit("Coord negative", &mut nb.coord_negative_color, dirty);
        edit("Certainty high", &mut nb.certainty_color, dirty);
        edit("Certainty mid", &mut nb.certainty_mid_color, dirty);
        edit("Certainty low", &mut nb.certainty_low_color, dirty);
        edit("Subpixel positive", &mut nb.subpixel_positive_color, dirty);
        edit("Subpixel negative", &mut nb.subpixel_negative_color, dirty);
    }

    // --- Columns ---
    if ui.collapsing_header("Columns", imgui::TreeNodeFlags::empty()) {
        let n = nb.columns.len();
        let mut swap: Option<(usize, usize)> = None;
        for i in 0..n {
            let col = &mut nb.columns[i];
            if ui.checkbox(format!("##nbbcol_show_{i}"), &mut col.show) {
                *dirty = true;
            }
            ui.same_line();
            ui.set_next_item_width(120.0);
            if ui.input_text(format!("##nbbcol_hdr_{i}"), &mut col.header).build() {
                *dirty = true;
            }
            ui.same_line();
            ui.set_next_item_width(90.0);
            if imgui::Drag::new(format!("##nbbcol_w_{i}")).range(0, 2000).build(ui, &mut col.static_width) {
                *dirty = true;
            }
            ui.same_line();
            ui.text_disabled(&col.id);
            ui.same_line();
            if i > 0 && ui.small_button(format!("Up##nbbcol_{i}")) {
                swap = Some((i, i - 1));
            }
            ui.same_line();
            if i + 1 < n && ui.small_button(format!("Down##nbbcol_{i}")) {
                swap = Some((i, i + 1));
            }
        }
        if let Some((a, b)) = swap {
            nb.columns.swap(a, b);
            *dirty = true;
        }
    }

    // --- Information messages ---
    if ui.collapsing_header("Information messages", imgui::TreeNodeFlags::empty()) {
        ui.text("Placement:");
        ui.same_line();
        ui.set_next_item_width(220.0);
        let plabel = match nb.information_messages_placement.as_str() {
            "top" => "Top",
            "bottom" => "Bottom",
            _ => "Between results and throws",
        };
        if let Some(_c) = ui.begin_combo("##nbb_info_place", plabel) {
            for (v, l) in [("top", "Top"), ("middle", "Between results and throws"), ("bottom", "Bottom")] {
                if ui.selectable_config(l).selected(nb.information_messages_placement == v).build() {
                    nb.information_messages_placement = v.to_string();
                    *dirty = true;
                }
            }
        }
        ui.text("Font scale:");
        ui.same_line();
        ui.set_next_item_width(140.0);
        if crate::widgets::slider_float(ui, "##nbb_info_fs", &mut nb.information_messages_font_scale, 0.4, 3.0, "%.2fx") {
            *dirty = true;
        }
        ui.text("Min width:");
        ui.same_line();
        ui.set_next_item_width(140.0);
        if crate::widgets::slider_float(ui, "##nbb_info_w", &mut nb.information_messages_min_width, 120.0, 1200.0, "%.0f") {
            *dirty = true;
        }
        ui.text("Icon scale:");
        ui.same_line();
        ui.set_next_item_width(140.0);
        if crate::widgets::slider_float(ui, "##nbb_info_icon", &mut nb.information_messages_icon_scale, 0.25, 4.0, "%.2fx") {
            *dirty = true;
        }
    }

    // --- Reset ---
    ui.dummy([0.0, 12.0]);
    if ui.button("Reset Ninjabrain Overlay to Defaults") {
        ui.open_popup("Reset NBB Overlay");
    }
    if let Some(_p) = ui.begin_popup("Reset NBB Overlay") {
        ui.text("Reset all Ninjabrain overlay settings to their default values?");
        if ui.button("Reset##nbb") {
            *nb = NinjabrainOverlayConfig::default();
            *dirty = true;
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel##nbb") {
            ui.close_current_popup();
        }
    }
}

fn rename_coords_column(nb: &mut NinjabrainOverlayConfig, header: &str) {
    for c in &mut nb.columns {
        if c.id == "coords" && (c.header == "Chunk" || c.header == "Location") {
            c.header = header.to_string();
        }
    }
}

/// Toolscreen compact preset — keeps placement / API / modes / font.
fn apply_compact_preset(nb: &mut NinjabrainOverlayConfig) {
    let keep = nb.clone();
    let c = |r: u8, g: u8, b: u8| Color::from_rgba8(r, g, b, 255);
    *nb = NinjabrainOverlayConfig {
        enabled: keep.enabled,
        x: keep.x,
        y: keep.y,
        relative_to: keep.relative_to.clone(),
        custom_font_path: keep.custom_font_path.clone(),
        api_base_url: keep.api_base_url.clone(),
        allowed_modes: keep.allowed_modes.clone(),
        bg_color: c(0, 0, 0),
        bg_opacity: 0.6,
        corner_radius: 3.0,
        show_throw_details: false,
        border_width: 0,
        header_fill_color: c(0, 0, 0),
        coords_display: "block".to_string(),
        border_color: c(79, 87, 97),
        divider_color: c(61, 69, 79),
        header_divider_color: c(79, 87, 97),
        text_color: c(140, 140, 140),
        throws_text_color: c(255, 255, 255),
        throws_background_color: c(0, 0, 0),
        coord_negative_color: c(204, 110, 114),
        certainty_mid_color: c(255, 189, 43),
        certainty_low_color: c(247, 51, 51),
        subpixel_positive_color: c(117, 204, 108),
        subpixel_negative_color: c(204, 110, 114),
        outline_width: 1,
        overlay_scale: 0.30,
        shown_predictions: 1,
        hide_if_stale: false,
        hide_if_stale_delay_seconds: 10,
        show_boat_state_in_top_bar: false,
        row_spacing: 10.0,
        side_padding: 0.0,
        ..NinjabrainOverlayConfig::default()
    };
    rename_coords_column(nb, "Location");
}

/// Toolscreen "ninjabrainbot" preset — mimics the real NBB window.
fn apply_nbbot_preset(nb: &mut NinjabrainOverlayConfig) {
    let api = nb.api_base_url.clone();
    let c = |r: u8, g: u8, b: u8| Color::from_rgba8(r, g, b, 255);
    *nb = NinjabrainOverlayConfig {
        enabled: true,
        api_base_url: api,
        bg_color: c(55, 60, 66),
        always_show: true,
        shown_predictions: 5,
        show_boat_state_in_top_bar: true,
        font_antialiasing: false,
        row_spacing: 2.0,
        overlay_scale: 0.30,
        relative_to: "topLeftScreen".to_string(),
        x: 0,
        y: 0,
        coords_display: "block".to_string(),
        information_messages_font_scale: 0.75,
        information_messages_icon_scale: 1.3,
        information_messages_min_width: 285.0,
        failure_margin_left: 24.0,
        failure_margin_right: 250.0,
        failure_margin_top: 20.0,
        failure_margin_bottom: 200.0,
        failure_line_gap: 64.0,
        blind_margin_right: 300.0,
        blind_margin_bottom: 250.0,
        hide_if_stale_delay_seconds: 10,
        columns: default_nbb_columns(),
        ..NinjabrainOverlayConfig::default()
    };
    rename_coords_column(nb, "Location");
}
