// Shared widget helpers used across the settings tabs

use std::cell::RefCell;
use std::path::Path;

use imgui::{SliderFlags, StyleColor, Ui};

// Tracks which slider is currently in inline text-edit mode (right-click to type).
thread_local! {
    static EDITING_SLIDER: RefCell<Option<String>> = RefCell::new(None);
}

// -- Slider wrappers --
// Both support arrow-key stepping when hovered and right-click to type a value.

pub fn slider_int(ui: &Ui, label: &str, val: &mut i32, min: i32, max: i32, fmt: &str) -> bool {
    let editing = EDITING_SLIDER.with(|e| e.borrow().as_deref() == Some(label));

    if editing {
        // inline text input - same look as native Ctrl+Click
        ui.set_keyboard_focus_here();
        let mut changed = imgui::Drag::new(label)
            .range(min, max)
            .speed(1.0)
            .display_format(fmt)
            .build(ui, val);
        if ui.is_item_deactivated() {
            EDITING_SLIDER.with(|e| *e.borrow_mut() = None);
            changed = true;
        }
        return changed;
    }

    let mut changed = ui
        .slider_config(label, min, max)
        .display_format(fmt)
        .flags(SliderFlags::ALWAYS_CLAMP | SliderFlags::NO_INPUT)
        .build(val);

    if ui.is_item_hovered() {
        let step = if ui.io().key_shift { 10 } else { 1 };
        if ui.is_key_pressed(imgui::Key::RightArrow) {
            *val = (*val + step).min(max);
            changed = true;
        }
        if ui.is_key_pressed(imgui::Key::LeftArrow) {
            *val = (*val - step).max(min);
            changed = true;
        }
        if ui.is_mouse_clicked(imgui::MouseButton::Right) {
            EDITING_SLIDER.with(|e| *e.borrow_mut() = Some(label.to_string()));
        }
        ui.tooltip_text("Arrow keys to step, Shift for x10, Right-click to type");
    }

    changed
}

pub fn slider_float(ui: &Ui, label: &str, val: &mut f32, min: f32, max: f32, fmt: &str) -> bool {
    let range = max - min;
    let editing = EDITING_SLIDER.with(|e| e.borrow().as_deref() == Some(label));

    if editing {
        ui.set_keyboard_focus_here();
        let mut changed = imgui::Drag::new(label)
            .range(min, max)
            .speed(range * 0.001)
            .display_format(fmt)
            .build(ui, val);
        if ui.is_item_deactivated() {
            EDITING_SLIDER.with(|e| *e.borrow_mut() = None);
            changed = true;
        }
        return changed;
    }

    let mut changed = ui
        .slider_config(label, min, max)
        .display_format(fmt)
        .flags(SliderFlags::ALWAYS_CLAMP | SliderFlags::NO_INPUT)
        .build(val);

    if ui.is_item_hovered() {
        let step = if ui.io().key_shift { range * 0.1 } else { range * 0.01 };
        if ui.is_key_pressed(imgui::Key::RightArrow) {
            *val = (*val + step).min(max);
            changed = true;
        }
        if ui.is_key_pressed(imgui::Key::LeftArrow) {
            *val = (*val - step).max(min);
            changed = true;
        }
        if ui.is_mouse_clicked(imgui::MouseButton::Right) {
            EDITING_SLIDER.with(|e| *e.borrow_mut() = Some(label.to_string()));
        }
        ui.tooltip_text("Arrow keys to step, Shift for x10, Right-click to type");
    }

    changed
}

// Font picker combo. `cache` is lazily filled on first open.
pub fn font_combo(
    ui: &Ui,
    label: &str,
    font_path: &mut String,
    cache: &mut Option<Vec<(String, String)>>,
) -> bool {
    let fonts = cache.get_or_insert_with(discover_fonts);

    // clear path if the file disappeared
    if !font_path.is_empty() && !Path::new(font_path.as_str()).exists() {
        font_path.clear();
        return true;
    }

    let preview = if font_path.is_empty() {
        "Default (ProggyClean)"
    } else {
        fonts
            .iter()
            .find(|(_, p)| *p == *font_path)
            .map(|(n, _)| n.as_str())
            .unwrap_or(font_path.as_str())
    };

    let mut changed = false;
    ui.set_next_item_width(280.0);
    if let Some(_tok) = ui.begin_combo(label, preview) {
        if ui
            .selectable_config("Default (ProggyClean)")
            .selected(font_path.is_empty())
            .build()
        {
            font_path.clear();
            changed = true;
        }
        for (name, path) in fonts.iter() {
            let sel = *path == *font_path;
            if ui.selectable_config(name).selected(sel).build() {
                *font_path = path.clone();
                changed = true;
            }
        }
    }
    changed
}

// text_colored that wraps instead of clipping
pub fn text_wrapped_colored(ui: &Ui, color: [f32; 4], text: &str) {
    let _c = ui.push_style_color(StyleColor::Text, color);
    let wrap = ui.content_region_avail()[0] + ui.cursor_pos()[0];
    let _w = ui.push_text_wrap_pos_with_pos(wrap);
    ui.text_wrapped(text);
}

// Scan for .ttf/.otf files across system font dirs.
// Falls back to fc-list on NixOS where fonts live in the nix store.
pub fn discover_fonts() -> Vec<(String, String)> {
    let home = std::env::var("HOME").unwrap_or_default();
    #[allow(unused_mut)]
    let mut search_dirs: Vec<String> = vec![
        "/usr/share/fonts".into(),
        "/usr/local/share/fonts".into(),
        format!("{home}/.local/share/fonts"),
        "/run/current-system/sw/share/fonts".into(),
        format!("{home}/.nix-profile/share/fonts"),
    ];
    #[cfg(target_os = "macos")]
    {
        search_dirs.push("/Library/Fonts".into());
        search_dirs.push("/System/Library/Fonts".into());
        search_dirs.push(format!("{home}/Library/Fonts"));
    }

    let mut fonts: Vec<(String, String)> = Vec::new();
    for dir in &search_dirs {
        scan_font_dir(Path::new(dir), &mut fonts);
    }

    // fc-list fallback (needed on NixOS where font paths are weird)
    if fonts.is_empty() {
        if let Ok(output) = std::process::Command::new("fc-list")
            .args(["--format", "%{file}\n"])
            .output()
        {
            if let Ok(stdout) = std::str::from_utf8(&output.stdout) {
                for line in stdout.lines() {
                    let p = Path::new(line);
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    if ext == "ttf" || ext == "otf" {
                        let name = p
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("?")
                            .to_string();
                        fonts.push((name, line.to_string()));
                    }
                }
            }
        }
    }

    fonts.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    fonts.dedup_by(|a, b| a.1 == b.1);
    fonts
}

fn scan_font_dir(dir: &Path, out: &mut Vec<(String, String)>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_font_dir(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let lo = ext.to_lowercase();
            if lo == "ttf" || lo == "otf" {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                if let Some(abs) = path.to_str() {
                    out.push((name, abs.to_string()));
                }
            }
        }
    }
}
