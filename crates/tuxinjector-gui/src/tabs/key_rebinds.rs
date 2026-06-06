use std::borrow::Cow;

use tuxinjector_config::key_names::{keycode_to_name, parse_key_name};
use tuxinjector_config::types::KeyRebind;
use tuxinjector_config::Config;

// Keyboard layout constants: (label, GLFW keycode, width in key-units).
// Keycode 0 = blank spacer gap.

type K = (&'static str, u32, f32);

const ROW_FN: &[K] = &[
    ("ESC", 256, 1.0), ("", 0, 1.0),
    ("F1", 290, 1.0), ("F2", 291, 1.0), ("F3", 292, 1.0), ("F4", 293, 1.0),
    ("", 0, 0.5),
    ("F5", 294, 1.0), ("F6", 295, 1.0), ("F7", 296, 1.0), ("F8", 297, 1.0),
    ("", 0, 0.5),
    ("F9", 298, 1.0), ("F10", 299, 1.0), ("F11", 300, 1.0), ("F12", 301, 1.0),
];

const ROW_NUM: &[K] = &[
    ("`", 96, 1.0),
    ("1", 49, 1.0), ("2", 50, 1.0), ("3", 51, 1.0), ("4", 52, 1.0),
    ("5", 53, 1.0), ("6", 54, 1.0), ("7", 55, 1.0), ("8", 56, 1.0),
    ("9", 57, 1.0), ("0", 48, 1.0), ("-", 45, 1.0), ("=", 61, 1.0),
    ("BACK", 259, 2.0),
];

const ROW_TOP: &[K] = &[
    ("TAB", 258, 1.5),
    ("Q", 81, 1.0), ("W", 87, 1.0), ("E", 69, 1.0), ("R", 82, 1.0),
    ("T", 84, 1.0), ("Y", 89, 1.0), ("U", 85, 1.0), ("I", 73, 1.0),
    ("O", 79, 1.0), ("P", 80, 1.0), ("[", 91, 1.0), ("]", 93, 1.0),
    ("\\", 92, 1.5),
];

const ROW_HOME: &[K] = &[
    ("CAPS", 280, 1.75),
    ("A", 65, 1.0), ("S", 83, 1.0), ("D", 68, 1.0), ("F", 70, 1.0),
    ("G", 71, 1.0), ("H", 72, 1.0), ("J", 74, 1.0), ("K", 75, 1.0),
    ("L", 76, 1.0), (";", 59, 1.0), ("'", 39, 1.0), ("ENTER", 257, 2.25),
];

const ROW_SHIFT: &[K] = &[
    ("LSHIFT", 340, 2.25),
    ("Z", 90, 1.0), ("X", 88, 1.0), ("C", 67, 1.0), ("V", 86, 1.0),
    ("B", 66, 1.0), ("N", 78, 1.0), ("M", 77, 1.0), (",", 44, 1.0),
    (".", 46, 1.0), ("/", 47, 1.0), ("RSHIFT", 344, 2.75),
];

const ROW_BOTTOM: &[K] = &[
    ("LCTRL", 341, 1.25), ("LWIN", 343, 1.25), ("LALT", 342, 1.25),
    ("SPACE", 32, 6.25),
    ("RALT", 346, 1.25), ("RWIN", 347, 1.25), ("RCTRL", 345, 2.5),
];

const ROW_NAV: &[K] = &[
    ("PRTSC", 283, 1.0), ("SCRLL", 281, 1.0), ("PAUSE", 284, 1.0),
    ("", 0, 0.5),
    ("INS", 260, 1.0), ("HOME", 268, 1.0), ("PGUP", 266, 1.0),
    ("", 0, 0.5),
    ("DEL", 261, 1.0), ("END", 269, 1.0), ("PGDN", 267, 1.0),
];

const ROW_ARROWS: &[K] = &[
    ("LEFT", 263, 1.0), ("DOWN", 264, 1.0), ("UP", 265, 1.0), ("RIGHT", 262, 1.0),
];

const KB_ROWS: &[&[K]] = &[
    ROW_FN, ROW_NUM, ROW_TOP, ROW_HOME, ROW_SHIFT, ROW_BOTTOM,
];

// -- State --

#[derive(Clone, Copy, PartialEq)]
enum CaptureTarget {
    Game,
    Chat,
}

pub struct KeyRebindsState {
    pub selected_key: Option<u32>,
    capturing: Option<CaptureTarget>,
    pub scale: f32,
    game_text: String,
    chat_text: String,
}

impl Default for KeyRebindsState {
    fn default() -> Self {
        Self {
            selected_key: None,
            capturing: None,
            scale: 1.2,
            game_text: String::new(),
            chat_text: String::new(),
        }
    }
}

impl KeyRebindsState {
    pub fn is_capturing(&self) -> bool {
        self.capturing.is_some()
    }

    pub fn cancel(&mut self) {
        self.capturing = None;
    }
}

// -- Config helpers for rebind lookups --

fn find_rebind(config: &Config, from: u32) -> Option<&KeyRebind> {
    config
        .input.key_rebinds.rebinds
        .iter()
        .find(|r| r.from_key == from && r.to_key != 0)
}

fn game_target(config: &Config, from: u32) -> Option<u32> {
    find_rebind(config, from).map(|r| r.to_key)
}

fn chat_target(config: &Config, from: u32) -> Option<u32> {
    find_rebind(config, from).map(|r| r.to_key_chat)
}

fn set_game(config: &mut Config, from: u32, to: u32) {
    config.input.key_rebinds.enabled = true;
    if let Some(r) = config.input.key_rebinds.rebinds.iter_mut().find(|r| r.from_key == from) {
        r.to_key = to;
        r.enabled = true;
    } else {
        config.input.key_rebinds.rebinds.push(KeyRebind {
            from_key: from, to_key: to,
            to_key_chat: 0, enabled: true,
            ..KeyRebind::default()
        });
    }
}

fn set_chat(config: &mut Config, from: u32, to_chat: u32) {
    config.input.key_rebinds.enabled = true;
    if let Some(r) = config.input.key_rebinds.rebinds.iter_mut().find(|r| r.from_key == from) {
        r.to_key_chat = to_chat;
        r.enabled = true;
    } else {
        // needs a game target too - default to same as chat
        config.input.key_rebinds.rebinds.push(KeyRebind {
            from_key: from, to_key: to_chat,
            to_key_chat: to_chat, enabled: true,
            ..KeyRebind::default()
        });
    }
}

fn clear_rebinds(config: &mut Config, from: u32) {
    config.input.key_rebinds.rebinds.retain(|r| r.from_key != from);
}

fn is_enabled(config: &Config, from: u32) -> bool {
    config.input.key_rebinds.rebinds.iter().any(|r| r.from_key == from && r.enabled)
}

fn toggle_enabled(config: &mut Config, from: u32, on: bool) {
    for r in &mut config.input.key_rebinds.rebinds {
        if r.from_key == from {
            r.enabled = on;
        }
    }
}

// -- Per-key custom repeat helpers --

fn find_entry(config: &Config, from: u32) -> Option<&KeyRebind> {
    config.input.key_rebinds.rebinds.iter().find(|r| r.from_key == from)
}

fn repeat_enabled_for(config: &Config, from: u32) -> bool {
    find_entry(config, from).map_or(false, |r| r.repeat_enabled)
}

fn repeat_vals(config: &Config, from: u32) -> (i32, i32) {
    find_entry(config, from)
        .map(|r| (r.repeat_start_delay, r.repeat_delay))
        .unwrap_or((200, 5))
}

// Find the rebind entry for `from`, creating a repeat-only one (no remap) if
// none exists. Enables the rebinds subsystem so the entry takes effect.
fn entry_mut(config: &mut Config, from: u32) -> &mut KeyRebind {
    config.input.key_rebinds.enabled = true;
    if !config.input.key_rebinds.rebinds.iter().any(|r| r.from_key == from) {
        config.input.key_rebinds.rebinds.push(KeyRebind { from_key: from, ..KeyRebind::default() });
    }
    config.input.key_rebinds.rebinds.iter_mut().find(|r| r.from_key == from).unwrap()
}

fn set_repeat_enabled(config: &mut Config, from: u32, on: bool) {
    entry_mut(config, from).repeat_enabled = on;
}

fn set_repeat_vals(config: &mut Config, from: u32, start: i32, interval: i32) {
    let e = entry_mut(config, from);
    e.repeat_start_delay = start;
    e.repeat_delay = interval;
}

// -- Color helpers --

fn rgb(r: u8, g: u8, b: u8) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

fn im_col(r: u8, g: u8, b: u8) -> imgui::ImColor32 {
    imgui::ImColor32::from_rgba(r, g, b, 255)
}

// -- Main render --

pub fn render(
    ui: &imgui::Ui,
    config: &mut Config,
    dirty: &mut bool,
    state: &mut KeyRebindsState,
    captured_key: Option<u32>,
) {
    // route captured key to the right rebind slot
    if let Some(key) = captured_key {
        if let Some(sel) = state.selected_key {
            if let Some(target) = state.capturing {
                match target {
                    CaptureTarget::Game => {
                        set_game(config, sel, key);
                        state.game_text = keycode_to_name(key).to_string();
                    }
                    CaptureTarget::Chat => {
                        set_chat(config, sel, key);
                        state.chat_text = keycode_to_name(key).to_string();
                    }
                }
                state.capturing = None;
                *dirty = true;
            }
        }
    }

    ui.separator(); ui.text("Keyboard Layout");

    // scale slider
    ui.text("Scale:");
    ui.same_line();
    let mut pct = state.scale * 100.0;
    ui.set_next_item_width(150.0);
    if crate::widgets::slider_float(ui, "##kb_scale", &mut pct, 50.0, 250.0, "%.0f%%") {
        state.scale = pct / 100.0;
    }

    ui.dummy([0.0, 4.0]);

    let unit = 48.0 * state.scale;
    let key_h = 48.0 * state.scale;
    let gap = 2.0;

    let start_pos = ui.cursor_screen_pos();

    // use number row to measure keyboard width (it's the widest standard row)
    let kb_w: f32 = ROW_NUM.iter().map(|k| k.2).sum::<f32>() * unit
        + (ROW_NUM.len() as f32 - 1.0) * gap;

    // draw keyboard rows
    for (ri, row) in KB_ROWS.iter().enumerate() {
        draw_key_row(ui, config, state, row, unit, key_h, gap, ri);
    }

    // nav cluster
    ui.dummy([0.0, 6.0]);
    ui.text("Navigation");
    draw_key_row(ui, config, state, ROW_NAV, unit, key_h, gap, 100);
    draw_key_row(ui, config, state, ROW_ARROWS, unit, key_h, gap, 101);

    let after_kb = ui.cursor_pos();

    // mouse diagram goes to the right of the keyboard
    let mx = start_pos[0] + kb_w + 20.0;
    ui.set_cursor_screen_pos([mx, start_pos[1]]);
    draw_mouse(ui, config, state, state.scale);

    // put cursor back below the keyboard
    ui.set_cursor_pos(after_kb);

    // inline editor for selected key
    if state.selected_key.is_some() {
        ui.dummy([0.0, 6.0]);
        ui.separator();
        rebind_editor(ui, config, state, dirty);
    }

    // legend
    ui.dummy([0.0, 8.0]);
    crate::widgets::text_wrapped_colored(
        ui,
        rgb(130, 140, 150),
        "Green = rebind enabled, Red = rebind disabled, gray = no rebind. Click a key to edit",
    );
}

// -- Key row rendering --

fn draw_key_row(
    ui: &imgui::Ui,
    config: &Config,
    state: &mut KeyRebindsState,
    row: &[K],
    unit: f32,
    key_h: f32,
    gap: f32,
    row_i: usize,
) {
    let _spacing = ui.push_style_var(imgui::StyleVar::ItemSpacing([gap, ui.clone_style().item_spacing[1]]));

    for (ci, &(label, code, w)) in row.iter().enumerate() {
        if ci > 0 { ui.same_line(); }

        let kw = w * unit;

        if code == 0 {
            ui.invisible_button(format!("##gap_{row_i}_{ci}"), [kw, key_h]);
            continue;
        }

        let tgt = game_target(config, code);
        let has = tgt.is_some();
        let disabled = has && !is_enabled(config, code);

        let clicked = ui.invisible_button(format!("##key_{code}"), [kw, key_h]);
        let hovered = ui.is_item_hovered();
        let p0 = ui.item_rect_min();
        let p1 = ui.item_rect_max();

        let ct = chat_target(config, code).filter(|&k| k != 0);
        let sel = state.selected_key == Some(code);
        paint_key(ui, p0, p1, label, tgt, ct, has, disabled, sel);

        if hovered && has {
            let gn = tgt.map(keycode_to_name).unwrap_or(Cow::Borrowed("(none)"));
            if let Some(ck) = ct {
                ui.tooltip_text(format!("{} -> {} (game), {} (chat)", label, gn, keycode_to_name(ck)));
            } else {
                ui.tooltip_text(format!("{} -> {}", label, gn));
            }
        }

        if clicked {
            handle_key_click(config, state, code);
        }
    }
}

fn handle_key_click(config: &Config, state: &mut KeyRebindsState, code: u32) {
    if state.selected_key == Some(code) {
        state.selected_key = None;
        state.cancel();
    } else {
        state.selected_key = Some(code);
        state.cancel();
        state.game_text = game_target(config, code)
            .map(|k| keycode_to_name(k).to_string())
            .unwrap_or_default();
        state.chat_text = chat_target(config, code)
            .filter(|&k| k != 0)
            .map(|k| keycode_to_name(k).to_string())
            .unwrap_or_default();
    }
}

fn paint_key(
    ui: &imgui::Ui,
    p0: [f32; 2],
    p1: [f32; 2],
    label: &str,
    game_tgt: Option<u32>,
    chat_tgt: Option<u32>,
    has_rebind: bool,
    disabled: bool,
    selected: bool,
) {
    let bg = if disabled {
        im_col(105, 35, 35)
    } else if has_rebind {
        im_col(35, 105, 35)
    } else {
        im_col(50, 58, 68)
    };
    let border_col = if selected {
        imgui::ImColor32::from_rgba(100, 140, 220, 255)
    } else {
        im_col(62, 70, 80)
    };
    let txt_col = im_col(210, 215, 225);
    let tgt_col = if disabled { txt_col } else { im_col(160, 220, 160) };

    let dl = ui.get_window_draw_list();
    dl.add_rect(p0, p1, bg).filled(true).rounding(3.0).build();
    if selected {
        dl.add_rect(p0, p1, imgui::ImColor32::from_rgba(60, 100, 200, 35))
            .filled(true).rounding(3.0).build();
    }
    dl.add_rect(p0, p1, border_col).rounding(3.0).thickness(1.0).build();

    let kw = p1[0] - p0[0];
    let kh = p1[1] - p0[1];

    if let Some(gt) = game_tgt {
        let cx = p0[0] + kw * 0.5;

        if let Some(ct) = chat_tgt {
            // three-line display: label, C:target, G:target
            let c_str = format!("C:{}", keycode_to_name(ct));
            let g_str = format!("G:{}", keycode_to_name(gt));

            let lsz = ui.calc_text_size(label);
            let csz = ui.calc_text_size(&c_str);
            let gsz = ui.calc_text_size(&g_str);
            let line_gap = 1.0;
            let total = lsz[1] + csz[1] + gsz[1] + line_gap * 2.0;
            let y0 = p0[1] + (kh - total) * 0.5;

            dl.add_text([cx - lsz[0] * 0.5, y0], txt_col, label);
            let cy = y0 + lsz[1] + line_gap;
            dl.add_text([cx - csz[0] * 0.5, cy], tgt_col, &c_str);
            let gy = cy + csz[1] + line_gap;
            dl.add_text([cx - gsz[0] * 0.5, gy], tgt_col, &g_str);
        } else {
            // two-line: label + target
            let tgt_name = keycode_to_name(gt);
            let lsz = ui.calc_text_size(label);
            let tsz = ui.calc_text_size(&*tgt_name);
            let line_gap = 1.0;
            let total = lsz[1] + tsz[1] + line_gap;
            let y0 = p0[1] + (kh - total) * 0.5;

            dl.add_text([cx - lsz[0] * 0.5, y0], txt_col, label);
            dl.add_text([cx - tsz[0] * 0.5, y0 + lsz[1] + line_gap], tgt_col, tgt_name);
        }
    } else {
        // just the label, centered
        let lsz = ui.calc_text_size(label);
        let x = p0[0] + (kw - lsz[0]) * 0.5;
        let y = p0[1] + (kh - lsz[1]) * 0.5;
        dl.add_text([x, y], txt_col, label);
    }
}

// -- Mouse diagram --

fn draw_mouse(
    ui: &imgui::Ui,
    config: &Config,
    state: &mut KeyRebindsState,
    scale: f32,
) {
    ui.text("Mouse");

    let mw = 140.0 * scale;
    let mh = 200.0 * scale;
    let pad = 6.0 * scale;
    let body_r = mw.min(mh) * 0.45;

    let text_min = ui.item_rect_min();
    let text_max = ui.item_rect_max();
    let origin = [text_min[0], text_max[1] + 4.0];
    ui.set_cursor_screen_pos(origin);
    ui.dummy([mw, mh]);

    let body_min = origin;
    let body_max = [origin[0] + mw, origin[1] + mh];

    let inner_min = [body_min[0] + pad, body_min[1] + pad];
    let inner_max = [body_max[0] - pad, body_max[1] - pad];
    let mid_x = (inner_min[0] + inner_max[0]) * 0.5;
    let top_h = (inner_max[1] - inner_min[1]) * 0.52;
    let split_y = inner_min[1] + top_h;

    // wheel region (MB3)
    let ww = (inner_max[0] - inner_min[0]) * 0.16;
    let wh = top_h * 0.55;
    let w_min = [mid_x - ww * 0.5, inner_min[1] + top_h * 0.18];
    let w_max = [mid_x + ww * 0.5, w_min[1] + wh];
    let wgap = 2.0 * scale;

    // MB1 (left) and MB2 (right)
    let l_min = inner_min;
    let l_max = [w_min[0] - wgap, split_y];
    let r_min = [w_max[0] + wgap, inner_min[1]];
    let r_max = [inner_max[0], split_y];

    // MB4/MB5 side buttons
    let sw = (inner_max[0] - inner_min[0]) * 0.32;
    let sh = (inner_max[1] - inner_min[1]) * 0.12;
    let sx0 = inner_min[0] + (inner_max[0] - inner_min[0]) * 0.07;
    let sy0 = inner_min[1] + top_h + (inner_max[1] - inner_min[1] - top_h) * 0.26;
    let sgap = sh * 0.35;
    let s1_min = [sx0, sy0];
    let s1_max = [sx0 + sw, sy0 + sh];
    let s2_min = [sx0, sy0 + sh + sgap];
    let s2_max = [sx0 + sw, sy0 + sh + sgap + sh];

    // draw body outline (scope the DrawListMut so it doesn't conflict with paint_key borrows)
    {
        let dl = ui.get_window_draw_list();
        dl.add_rect(body_min, body_max, im_col(24, 26, 33))
            .rounding(body_r).filled(true).build();
        dl.add_rect(body_min, body_max, im_col(10, 10, 12))
            .rounding(body_r).thickness(1.5).build();

        let lc = im_col(10, 10, 12);
        dl.add_line([mid_x, inner_min[1] + 2.0], [mid_x, split_y - 2.0], lc).build();
        dl.add_line([inner_min[0] + 2.0, split_y], [inner_max[0] - 2.0, split_y], lc).build();
    }

    // interactive mouse button regions
    let btns: [(&str, u32, [f32; 2], [f32; 2]); 5] = [
        ("M1", 400, l_min, l_max),
        ("M2", 401, r_min, r_max),
        ("M3", 402, w_min, w_max),
        ("M5", 404, s1_min, s1_max),
        ("M4", 403, s2_min, s2_max),
    ];

    for &(label, code, bmin, bmax) in &btns {
        let bw = bmax[0] - bmin[0];
        let bh = bmax[1] - bmin[1];
        if bw <= 0.0 || bh <= 0.0 { continue; }

        ui.set_cursor_screen_pos(bmin);
        let clicked = ui.invisible_button(format!("##mkey_{code}"), [bw, bh]);
        let hovered = ui.is_item_hovered();

        let tgt = game_target(config, code);
        let ct = chat_target(config, code).filter(|&k| k != 0);
        let has = tgt.is_some();
        let dis = has && !is_enabled(config, code);
        let sel = state.selected_key == Some(code);
        paint_key(ui, bmin, bmax, label, tgt, ct, has, dis, sel);

        if hovered && has {
            let gn = tgt.map(keycode_to_name).unwrap_or(Cow::Borrowed("(none)"));
            if let Some(ck) = ct {
                ui.tooltip_text(format!("{} -> {} (game), {} (chat)", label, gn, keycode_to_name(ck)));
            } else {
                ui.tooltip_text(format!("{} -> {}", label, gn));
            }
        }

        if clicked {
            handle_key_click(config, state, code);
        }
    }
}

// -- Inline editor panel --

fn rebind_editor(
    ui: &imgui::Ui,
    config: &mut Config,
    state: &mut KeyRebindsState,
    dirty: &mut bool,
) {
    let Some(sel) = state.selected_key else { return };
    let name = keycode_to_name(sel);

    ui.text(format!("Source: {name}"));

    let has = game_target(config, sel).is_some();
    if has {
        let mut on = is_enabled(config, sel);
        if ui.checkbox(format!("Rebind Enabled##rb_en_{sel}"), &mut on) {
            toggle_enabled(config, sel, on);
            *dirty = true;
        }
    } else {
        ui.text_disabled("No rebind set -- assign a target first");
    }

    // chat/text target
    ui.text("Types (Chat/Text):");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if ui.input_text("##rebind_chat", &mut state.chat_text)
        .hint("key name")
        .build()
    {
        if let Some(code) = parse_key_name(&state.chat_text) {
            set_chat(config, sel, code);
            *dirty = true;
        }
    }
    ui.same_line();
    if state.capturing == Some(CaptureTarget::Chat) {
        if ui.button("Cancel##cancel_chat") {
            state.capturing = None;
        }
        ui.same_line();
        ui.text_colored([1.0, 1.0, 0.0, 1.0], "Press a key...");
    } else {
        if ui.button("Capture##cap_chat") {
            state.chat_text.clear();
            state.capturing = Some(CaptureTarget::Chat);
        }
    }

    // game target
    ui.text("Triggers (Game):");
    ui.same_line();
    ui.set_next_item_width(100.0);
    if ui.input_text("##rebind_game", &mut state.game_text)
        .hint("key name")
        .build()
    {
        if let Some(code) = parse_key_name(&state.game_text) {
            set_game(config, sel, code);
            *dirty = true;
        }
    }
    ui.same_line();
    if state.capturing == Some(CaptureTarget::Game) {
        if ui.button("Cancel##cancel_game") {
            state.capturing = None;
        }
        ui.same_line();
        ui.text_colored([1.0, 1.0, 0.0, 1.0], "Press a key...");
    } else {
        if ui.button("Capture##cap_game") {
            state.game_text.clear();
            state.capturing = Some(CaptureTarget::Game);
        }
    }

    // -- per-key custom repeat rate --
    ui.dummy([0.0, 6.0]);
    ui.separator();
    let mut rpt = repeat_enabled_for(config, sel);
    if ui.checkbox(format!("Custom Repeat Rate##rpt_en_{sel}"), &mut rpt) {
        set_repeat_enabled(config, sel, rpt);
        *dirty = true;
    }
    if rpt {
        let (mut start, mut interval) = repeat_vals(config, sel);
        ui.text("Start Delay (ms):");
        ui.same_line();
        ui.set_next_item_width(120.0);
        let start_id = format!("##rpt_start_{sel}");
        if crate::widgets::slider_int(ui, &start_id, &mut start, 0, 2000, "%d ms") {
            set_repeat_vals(config, sel, start, interval);
            *dirty = true;
        }
        ui.text("Repeat Delay (ms):");
        ui.same_line();
        ui.set_next_item_width(120.0);
        let delay_id = format!("##rpt_delay_{sel}");
        if crate::widgets::slider_int(ui, &delay_id, &mut interval, 1, 500, "%d ms") {
            set_repeat_vals(config, sel, start, interval);
            *dirty = true;
        }
        crate::widgets::text_wrapped_colored(
            ui, rgb(130, 140, 150),
            "Lower Repeat Delay = faster. tuxinjector drives this key's repeat \
             instead of the OS, so other keys (Escape, etc.) are unaffected.",
        );
    }

    ui.dummy([0.0, 6.0]);
    if ui.button("Reset") {
        clear_rebinds(config, sel);
        state.game_text.clear();
        state.chat_text.clear();
        *dirty = true;
    }
}
