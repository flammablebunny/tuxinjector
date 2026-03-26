use tuxinjector_config::Config;

use std::cell::RefCell;
use std::time::Instant;

// cached list of available cursor files from ~/.local/share/tuxinjector/cursors/
thread_local! {
    static CURSOR_LIST: RefCell<(Vec<String>, Instant)> = RefCell::new((Vec::new(), Instant::now()));
}

fn scan_cursors_folder() -> Vec<String> {
    let mut cursors = Vec::new();
    let dir = dirs_cursors();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            if lower.ends_with(".png") || lower.ends_with(".cur")
                || lower.ends_with(".gif") || lower.ends_with(".jpg")
                || lower.ends_with(".jpeg") || lower.ends_with(".webp")
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

fn dirs_cursors() -> String {
    std::env::var("HOME")
        .map(|h| format!("{h}/.config/tuxinjector/cursors"))
        .unwrap_or_default()
}

pub fn render(ui: &imgui::Ui, config: &mut Config, dirty: &mut bool) {
    if ui.checkbox("Enable Custom Cursors", &mut config.theme.cursors.enabled) {
        *dirty = true;
    }

    if !config.theme.cursors.enabled {
        ui.dummy([0.0, 4.0]);
        ui.text("Enable custom cursors to replace the game cursor.");
        return;
    }

    let cursors = get_cursor_list();

    ui.dummy([0.0, 4.0]);
    if cursors.is_empty() {
        ui.text_colored([1.0, 0.8, 0.3, 1.0], "No cursors found.");
        ui.text("Place .png or .cur files in:");
        ui.text_disabled(&dirs_cursors());
    } else {
        ui.text_disabled(&format!("{} cursor(s) found", cursors.len()));
    }

    if ui.button("Open Cursors Folder") {
        let dir = dirs_cursors();
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn();
    }

    ui.dummy([0.0, 8.0]);
    ui.separator(); ui.text("Title Screen");
    cursor_editor(ui, "title", &mut config.theme.cursors.title, &cursors, dirty);

    ui.dummy([0.0, 4.0]);
    ui.separator(); ui.text("Wall / Menu");
    cursor_editor(ui, "wall", &mut config.theme.cursors.wall, &cursors, dirty);

    ui.dummy([0.0, 4.0]);
    ui.separator(); ui.text("In-Game");
    cursor_editor(ui, "ingame", &mut config.theme.cursors.ingame, &cursors, dirty);
}

fn cursor_editor(
    ui: &imgui::Ui,
    id: &str,
    cc: &mut tuxinjector_config::types::CursorConfig,
    cursors: &[String],
    dirty: &mut bool,
) {
    // cursor picker dropdown
    ui.text("Cursor:");
    ui.same_line();
    ui.set_next_item_width(200.0);
    let preview = if cc.cursor_name.is_empty() { "(none)" } else { &cc.cursor_name };
    if let Some(_combo) = ui.begin_combo(format!("##cursor_{id}"), preview) {
        // none option
        if ui.selectable_config("(none)")
            .selected(cc.cursor_name.is_empty())
            .build()
        {
            cc.cursor_name.clear();
            *dirty = true;
        }
        for name in cursors {
            if ui.selectable_config(name)
                .selected(cc.cursor_name == *name)
                .build()
            {
                cc.cursor_name = name.clone();
                *dirty = true;
            }
        }
    }

    // size slider
    ui.text("Size:");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if crate::widgets::slider_int(ui, &format!("##cursor_size_{id}"), &mut cc.cursor_size, 8, 320, "%d px") {
        *dirty = true;
    }

    // hotspot
    ui.text("Hotspot:");
    ui.same_line();
    ui.set_next_item_width(60.0);
    if crate::widgets::slider_float(ui, &format!("##cursor_hx_{id}"), &mut cc.hotspot_x, 0.0, 1.0, "X %.2f") {
        *dirty = true;
    }
    ui.same_line();
    ui.set_next_item_width(60.0);
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
