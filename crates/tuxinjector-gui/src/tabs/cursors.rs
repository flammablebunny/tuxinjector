use tuxinjector_config::Config;
use tuxinjector_config::types::{CursorConfig, CursorTrailConfig, CursorsConfig};

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Instant;

// cached list of available cursor files from ~/.config/tuxinjector/cursors/
thread_local! {
    static CURSOR_LIST: RefCell<(Vec<String>, Instant)> = RefCell::new((Vec::new(), Instant::now()));
}

fn dirs_cursors() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".config/tuxinjector/cursors"))
        .unwrap_or_default()
}

fn scan_cursors_folder() -> Vec<String> {
    let mut cursors = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dirs_cursors()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            if lower.ends_with(".png") || lower.ends_with(".cur") || lower.ends_with(".ico")
                || lower.ends_with(".gif") || lower.ends_with(".jpg") || lower.ends_with(".jpeg")
            {
                cursors.push(name);
            }
        }
    }
    cursors.sort();
    cursors
}

fn get_cursor_list() -> Vec<String> {
    CURSOR_LIST.with(|cell| {
        let mut pair = cell.borrow_mut();
        // refresh every 2 seconds
        if pair.1.elapsed().as_secs() >= 2 || pair.0.is_empty() {
            pair.0 = scan_cursors_folder();
            pair.1 = Instant::now();
        }
        pair.0.clone()
    })
}

// "cross_inverted-large.cur" -> "Cross inverted large"
fn display_name(file: &str) -> String {
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    let spaced = stem.replace(['_', '-'], " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

pub fn render(ui: &imgui::Ui, config: &mut Config, dirty: &mut bool) {
    // per-state cursor pickers first, then the trail section further down
    if ui.checkbox("Enable Custom Cursors", &mut config.theme.cursors.enabled) {
        *dirty = true;
    }
    ui.text_disabled("Changes the mouse cursor based on the current game state.");

    ui.dummy([0.0, 4.0]);
    if ui.button("Open Cursors Folder") {
        let dir = dirs_cursors();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
    }
    ui.same_line();
    ui.text_disabled(".png .cur .ico .gif .jpg");

    if config.theme.cursors.enabled {
        let cursors = get_cursor_list();
        if cursors.is_empty() {
            ui.text_colored([1.0, 0.8, 0.3, 1.0], "No cursor files found. Place them in:");
            ui.text_disabled(dirs_cursors().to_string_lossy());
        }

        ui.dummy([0.0, 8.0]);
        ui.separator();
        ui.text("Title Screen");
        cursor_editor(ui, "title", &mut config.theme.cursors.title, &cursors, dirty);

        ui.dummy([0.0, 4.0]);
        ui.separator();
        ui.text("Wall Screen");
        cursor_editor(ui, "wall", &mut config.theme.cursors.wall, &cursors, dirty);

        ui.dummy([0.0, 4.0]);
        ui.separator();
        ui.text("In World");
        ui.text_disabled("Also used on waiting, generating and loading screens.");
        cursor_editor(ui, "ingame", &mut config.theme.cursors.ingame, &cursors, dirty);

        ui.dummy([0.0, 8.0]);
        if ui.button("Reset Cursors to Defaults") {
            ui.open_popup("Reset Cursors");
        }
        if let Some(_popup) = ui.begin_popup("Reset Cursors") {
            ui.text("Reset all cursor settings to their default values?");
            if ui.button("Reset") {
                config.theme.cursors = CursorsConfig { enabled: true, ..Default::default() };
                *dirty = true;
                ui.close_current_popup();
            }
            ui.same_line();
            if ui.button("Cancel") {
                ui.close_current_popup();
            }
        }
    }

    // trail is its own feature, independent of the custom cursors above
    ui.dummy([0.0, 12.0]);
    ui.separator();
    ui.text("Cursor Trail");
    trail_editor(ui, &mut config.theme.cursor_trail, dirty);
}

fn cursor_editor(
    ui: &imgui::Ui,
    id: &str,
    cc: &mut CursorConfig,
    cursors: &[String],
    dirty: &mut bool,
) {
    // cursor picker dropdown
    ui.text("Cursor:");
    ui.same_line();
    ui.set_next_item_width(220.0);
    let preview = if cc.cursor_name.is_empty() {
        "(none)".to_string()
    } else {
        display_name(&cc.cursor_name)
    };
    if let Some(_combo) = ui.begin_combo(format!("##cursor_{id}"), &preview) {
        if ui.selectable_config("(none)")
            .selected(cc.cursor_name.is_empty())
            .build()
        {
            cc.cursor_name.clear();
            *dirty = true;
        }
        for name in cursors {
            if ui.selectable_config(format!("{}##{name}", display_name(name)))
                .selected(cc.cursor_name == *name)
                .build()
            {
                cc.cursor_name = name.clone();
                *dirty = true;
            }
        }
    }

    // size slider - clamp at 256 since hardware cursor planes won't go bigger
    ui.text("Size:");
    ui.same_line();
    ui.set_next_item_width(120.0);
    if crate::widgets::slider_int(ui, &format!("##cursor_size_{id}"), &mut cc.cursor_size, 8, 256, "%d px") {
        *dirty = true;
    }

    // hotspot - .cur/.ico carry their own, so those override whatever we set here
    let lower = cc.cursor_name.to_lowercase();
    if lower.ends_with(".cur") || lower.ends_with(".ico") {
        ui.text_disabled("Hotspot comes from the cursor file.");
    } else {
        ui.text("Hotspot:");
        ui.same_line();
        ui.set_next_item_width(70.0);
        if crate::widgets::slider_float(ui, &format!("##cursor_hx_{id}"), &mut cc.hotspot_x, 0.0, 1.0, "X %.2f") {
            *dirty = true;
        }
        ui.same_line();
        ui.set_next_item_width(70.0);
        if crate::widgets::slider_float(ui, &format!("##cursor_hy_{id}"), &mut cc.hotspot_y, 0.0, 1.0, "Y %.2f") {
            *dirty = true;
        }
        ui.same_line();
        if ui.button(format!("Center##{id}")) {
            cc.hotspot_x = 0.5;
            cc.hotspot_y = 0.5;
            *dirty = true;
        }
        ui.same_line();
        if ui.button(format!("Top-Left##{id}")) {
            cc.hotspot_x = 0.0;
            cc.hotspot_y = 0.0;
            *dirty = true;
        }
    }
}

fn trail_editor(ui: &imgui::Ui, trail: &mut CursorTrailConfig, dirty: &mut bool) {
    if ui.checkbox("Enable Cursor Trail", &mut trail.enabled) {
        *dirty = true;
    }
    ui.text_disabled("Draws a trail of fading stamps following the mouse cursor.");
    if !trail.enabled {
        return;
    }

    ui.text("Lifetime:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    if crate::widgets::slider_int(ui, "##trail_lifetime", &mut trail.lifetime_ms, 50, 500, "%d ms") {
        *dirty = true;
    }

    ui.text("Opacity:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    if crate::widgets::slider_float(ui, "##trail_opacity", &mut trail.opacity, 0.0, 1.0, "%.2f") {
        *dirty = true;
    }

    ui.text("Blend Mode:");
    ui.same_line();
    ui.set_next_item_width(160.0);
    let additive = trail.blend_mode == "Additive";
    let preview = if additive { "Additive (Glow)" } else { "Alpha (Solid)" };
    if let Some(_combo) = ui.begin_combo("##trail_blend", preview) {
        if ui.selectable_config("Alpha (Solid)").selected(!additive).build() {
            trail.blend_mode = "Alpha".to_string();
            *dirty = true;
        }
        if ui.selectable_config("Additive (Glow)").selected(additive).build() {
            trail.blend_mode = "Additive".to_string();
            *dirty = true;
        }
    }

    if ui.checkbox("Velocity-reactive size", &mut trail.use_velocity_size) {
        *dirty = true;
    }
    if trail.use_velocity_size {
        ui.text("Intensity:");
        ui.same_line();
        ui.set_next_item_width(160.0);
        if crate::widgets::slider_float(ui, "##trail_vel_intensity", &mut trail.velocity_size_intensity, 0.0, 1.0, "%.2f") {
            *dirty = true;
        }
    }

    ui.text(if trail.use_gradient { "Head color:" } else { "Trail color:" });
    ui.same_line();
    let mut head = [trail.color.r, trail.color.g, trail.color.b];
    if ui.color_edit3("##trail_color", &mut head) {
        trail.color.r = head[0];
        trail.color.g = head[1];
        trail.color.b = head[2];
        *dirty = true;
    }

    if ui.checkbox("Fade to tail color (gradient)", &mut trail.use_gradient) {
        *dirty = true;
    }
    if trail.use_gradient {
        ui.text("Tail color:");
        ui.same_line();
        let mut tail = [trail.tail_color.r, trail.tail_color.g, trail.tail_color.b];
        if ui.color_edit3("##trail_tail_color", &mut tail) {
            trail.tail_color.r = tail[0];
            trail.tail_color.g = tail[1];
            trail.tail_color.b = tail[2];
            *dirty = true;
        }
    }

    if ui.collapsing_header("Advanced Settings", imgui::TreeNodeFlags::empty()) {
        ui.text("Stamp spacing:");
        ui.same_line();
        ui.set_next_item_width(160.0);
        if crate::widgets::slider_int(ui, "##trail_spacing", &mut trail.stamp_spacing_px, 1, 64, "%d px") {
            *dirty = true;
        }

        ui.text("Sprite size:");
        ui.same_line();
        ui.set_next_item_width(160.0);
        if crate::widgets::slider_int(ui, "##trail_sprite_size", &mut trail.sprite_size_px, 4, 256, "%d px") {
            *dirty = true;
        }

        ui.text("Tail size scale:");
        ui.same_line();
        ui.set_next_item_width(160.0);
        if crate::widgets::slider_float(ui, "##trail_tail_scale", &mut trail.tail_size_scale, 0.0, 2.0, "%.2fx") {
            *dirty = true;
        }

        ui.text("Custom sprite (max 256x256, empty = soft dot):");
        ui.set_next_item_width(320.0);
        if ui.input_text("##trail_sprite_path", &mut trail.sprite_path).build() {
            *dirty = true;
        }
        ui.same_line();
        if ui.button("Clear##trail_sprite") {
            trail.sprite_path.clear();
            *dirty = true;
        }
    }

    ui.dummy([0.0, 8.0]);
    if ui.button("Reset Cursor Trail to Defaults") {
        ui.open_popup("Reset Trail");
    }
    if let Some(_popup) = ui.begin_popup("Reset Trail") {
        ui.text("Reset all cursor trail settings to their default values?");
        if ui.button("Reset##trail") {
            *trail = CursorTrailConfig { enabled: true, ..Default::default() };
            *dirty = true;
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel##trail") {
            ui.close_current_popup();
        }
    }
}
