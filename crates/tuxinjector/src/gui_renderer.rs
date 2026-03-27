// ImGui settings overlay, rendered into the game's GL backbuffer.

use std::sync::Arc;
use std::time::Instant;

use imgui::{Condition, FontSource, StyleColor};
use tuxinjector_config::ConfigSnapshot;
use tuxinjector_gui::SettingsApp;

use crate::gl_resolve::EglGetProcAddressFn;
use crate::state;

struct ActiveToast {
    msg: String,
    color: [f32; 4],
    remaining: u32,
}

/// Per-frame input events for the GUI
#[derive(Default)]
pub struct GuiInput {
    pub pointer_pos: Option<(f32, f32)>,
    pub pointer_button_pressed: bool,
    pub pointer_button_released: bool,
    pub rbutton_pressed: bool,
    pub rbutton_released: bool,
    // GLFW mod bitmask so Ctrl+Click works on sliders
    pub pointer_button_mods: i32,
    pub scroll_delta: (f32, f32),
    // (glfw_key, pressed, glfw_mods)
    pub keys: Vec<(i32, bool, i32)>,
    pub text: String,
}

pub struct GuiRenderer {
    imgui: imgui::Context,
    renderer: imgui_glow_renderer::AutoRenderer,
    app: SettingsApp,
    config: Arc<ConfigSnapshot>,
    gpa: EglGetProcAddressFn,
    last_render: Instant,
    visible: bool,
    mouse_held: bool,
    rmouse_held: bool,
    last_mods: i32,
    toasts: Vec<ActiveToast>,
    cached_perf: Option<crate::perf_stats::PerfSnapshot>,
    perf_time: Instant,
    cur_theme: String,
    cur_font: String,
}

// Safety: only ever used on the GL render thread. The Mutex wrapper on
// OverlayState satisfies OnceLock's Send bound, not for cross-thread use.
unsafe impl Send for GuiRenderer {}

impl GuiRenderer {
    pub fn new(config: Arc<ConfigSnapshot>, gpa: EglGetProcAddressFn) -> Self {
        let cfg = config.load();
        let mut app = SettingsApp::new((**cfg).clone());

        if let Some(dir) = state::get().config_dir.get() {
            app.profile_list = crate::lua_writer::list_profiles(dir);
        }

        let mut imgui = imgui::Context::create();
        imgui.set_ini_filename(None); // don't litter imgui.ini into the game dir
        imgui.set_log_filename(None);
        imgui.set_clipboard_backend(SystemClipboard);

        let font_path = cfg.theme.font_path.clone();
        add_font(&mut imgui, &font_path);

        let theme = cfg.theme.appearance.theme.clone();
        apply_theme(imgui.style_mut(), &theme);

        let glow_ctx = unsafe {
            glow::Context::from_loader_function(|name| {
                let cname = std::ffi::CString::new(name).unwrap();
                gpa(cname.as_ptr()) as *const _
            })
        };

        let renderer = imgui_glow_renderer::AutoRenderer::new(glow_ctx, &mut imgui)
            .expect("failed to init imgui-glow-renderer");

        let now = Instant::now();
        Self {
            imgui,
            renderer,
            app,
            config,
            gpa,
            last_render: now,
            visible: false,
            mouse_held: false,
            rmouse_held: false,
            last_mods: 0,
            toasts: Vec::new(),
            cached_perf: None,
            perf_time: now,
            cur_theme: theme,
            cur_font: font_path,
        }
    }

    pub fn is_visible(&self) -> bool { self.visible }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
        self.app.toggle();
    }

    pub fn render(
        &mut self,
        vp_w: u32,
        vp_h: u32,
        input: &GuiInput,
        perf: Option<&crate::perf_stats::PerfSnapshot>,
    ) {
        let cfg = self.config.load();

        // hot-swap theme if it has changed
        if cfg.theme.appearance.theme != self.cur_theme {
            apply_theme(self.imgui.style_mut(), &cfg.theme.appearance.theme);
            self.cur_theme = cfg.theme.appearance.theme.clone();
        }

        // hot-swap font
        if cfg.theme.font_path != self.cur_font {
            self.cur_font = cfg.theme.font_path.clone();
            self.imgui.fonts().clear();
            add_font(&mut self.imgui, &self.cur_font);

            // need to recreate renderer to re-upload font atlas
            let glow_ctx = unsafe {
                glow::Context::from_loader_function(|name| {
                    let cname = std::ffi::CString::new(name).unwrap();
                    (self.gpa)(cname.as_ptr()) as *const _
                })
            };
            self.renderer = imgui_glow_renderer::AutoRenderer::new(glow_ctx, &mut self.imgui)
                .expect("failed to reinit imgui-glow-renderer after font change");
        }

        // throttle perf updates to 250ms so it's not a flickering mess
        if let Some(snap) = perf {
            let now = Instant::now();
            if now.duration_since(self.perf_time).as_millis() >= 250
                || self.cached_perf.is_none()
            {
                self.cached_perf = Some(snap.clone());
                self.perf_time = now;
            }
        } else {
            self.cached_perf = None;
        }

        let show_perf = self.cached_perf.is_some() && cfg.advanced.debug.show_performance_overlay;

        // drain toast queue
        for t in tuxinjector_gui::toast::drain() {
            let col = t.color
                .map(|[r, g, b, a]| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0])
                .unwrap_or([1.0, 1.0, 1.0, 1.0]);
            self.toasts.push(ActiveToast { msg: t.message, color: col, remaining: 300 });
        }
        let has_toasts = !self.toasts.is_empty();

        // bail early if there's nothing to draw
        if (!self.visible && !show_perf && !has_toasts) || vp_w == 0 || vp_h == 0 {
            return;
        }

        if self.visible {
            self.app.update_config((**cfg).clone());
        }

        // scale relative to 1080p as baseline
        let res_scale = vp_h as f32 / 1080.0;
        let gui_scale = (cfg.theme.appearance.gui_scale * res_scale).clamp(0.5, 4.0);

        let now = Instant::now();
        let dt = now.duration_since(self.last_render).as_secs_f32();
        self.last_render = now;

        // --- configure IO before new_frame ---
        {
            let io = self.imgui.io_mut();
            io.display_size = [vp_w as f32, vp_h as f32];
            io.display_framebuffer_scale = [1.0, 1.0];
            io.delta_time = dt.max(0.0001); // imgui panics on zero dt
            io.font_global_scale = gui_scale;

            // key repeat (ms -> s), 0 = imgui default
            if cfg.input.key_repeat_start_delay > 0 {
                io.key_repeat_delay = cfg.input.key_repeat_start_delay as f32 / 1000.0;
            }
            if cfg.input.key_repeat_delay > 0 {
                io.key_repeat_rate = cfg.input.key_repeat_delay as f32 / 1000.0;
            }

            for &(glfw_key, pressed, mods) in &input.keys {
                self.last_mods = mods;
                if let Some(imgui_key) = glfw_to_imgui_key(glfw_key) {
                    io.add_key_event(imgui_key, pressed);
                }
            }

            // mouse clicks carry mods too (fixes Ctrl+Click on sliders)
            if input.pointer_button_pressed || input.pointer_button_released {
                if input.pointer_button_mods != 0 || input.keys.is_empty() {
                    self.last_mods = input.pointer_button_mods;
                }
            }

            // update modifier keys so Ctrl state stays consistent
            // note: we intentionally don't forward Shift to imgui because it
            // converts Shift+Scroll to horizontal scroll, which is probably unwanted.
            // i guess im doing this now, so credit to justin
            {
                use tuxinjector_input::glfw_types::*;
                io.add_key_event(imgui::Key::ModCtrl, (self.last_mods & GLFW_MOD_CONTROL) != 0);
                io.add_key_event(imgui::Key::ModAlt, (self.last_mods & GLFW_MOD_ALT) != 0);
                io.add_key_event(imgui::Key::ModSuper, (self.last_mods & GLFW_MOD_SUPER) != 0);
            }

            if let Some((x, y)) = input.pointer_pos {
                io.add_mouse_pos_event([x, y]);
            }

            if input.pointer_button_pressed { self.mouse_held = true; }
            if input.pointer_button_released { self.mouse_held = false; }
            io.add_mouse_button_event(imgui::MouseButton::Left, self.mouse_held);

            if input.rbutton_pressed { self.rmouse_held = true; }
            if input.rbutton_released { self.rmouse_held = false; }
            io.add_mouse_button_event(imgui::MouseButton::Right, self.rmouse_held);

            if input.scroll_delta.0 != 0.0 || input.scroll_delta.1 != 0.0 {
                io.add_mouse_wheel_event([input.scroll_delta.0, input.scroll_delta.1]);
            }

            for ch in input.text.chars() {
                io.add_input_character(ch);
            }
        }

        // --- build UI ---
        let captured_key = if self.visible {
            tuxinjector_input::take_captured_key().map(|k| k as u32)
        } else {
            None
        };

        let perf_snap = self.cached_perf.clone();
        let mut app_out = None;

        {
            let ui = self.imgui.new_frame();

            if show_perf {
                if let Some(ref snap) = perf_snap {
                    draw_perf_overlay(ui, snap, cfg.advanced.debug.perf_overlay_position);
                }
            }

            if self.visible {
                // feed plugin summaries into the GUI
                if let Some(reg_lock) = state::get().plugins.get() {
                    if let Ok(reg) = reg_lock.lock() {
                        let summaries = reg.summaries().into_iter()
                            .map(|s| tuxinjector_gui::tabs::plugins::PluginSummary {
                                name: s.name, version: s.version,
                                description: s.description, enabled: s.enabled,
                                settings_schema: s.settings_schema,
                            })
                            .collect();
                        self.app.update_loaded_plugins(summaries);
                    }
                }

                app_out = Some(self.app.render(ui, captured_key));
            }

            draw_toasts(ui, &self.toasts);
        }

        // --- GPU render ---
        let draw_data = self.imgui.render();
        self.renderer.render(draw_data).expect("imgui-glow render failed");

        // --- post-render ---
        for t in &mut self.toasts {
            t.remaining = t.remaining.saturating_sub(1);
        }
        self.toasts.retain(|t| t.remaining > 0);

        // handle plugin actions from the GUI
        {
            let actions = self.app.take_plugin_actions();
            if !actions.is_empty() {
                if let Some(reg_lock) = state::get().plugins.get() {
                    if let Ok(mut reg) = reg_lock.lock() {
                        for action in &actions {
                            match action {
                                tuxinjector_gui::tabs::plugins::PluginAction::SetEnabled { name, enabled } => {
                                    reg.set_enabled(name, *enabled);
                                }
                                tuxinjector_gui::tabs::plugins::PluginAction::Reload => {
                                    let saved = crate::plugin_loader::load_plugin_settings();
                                    let loaded = crate::plugin_loader::discover_and_load(&saved);
                                    *reg = crate::plugin_registry::PluginRegistry::new(loaded, saved);
                                    tracing::info!("plugins: reloaded from disk via GUI");
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(out) = app_out {
            // --- normal save: write to the CORRECT file for the active profile ---
            if let Some(new_cfg) = out.saved_config {
                self.config.publish(new_cfg.clone());
                crate::overlay_gen::generate_overlay(&new_cfg);
                if let Some(dir) = state::get().config_dir.get() {
                    crate::lua_writer::save_current_config(&new_cfg, dir);
                } else {
                    tracing::warn!("can't save config: config dir unknown");
                }
            }

            // --- profile management ---
            let mut need_refresh = false;

            if let Some(ref name) = out.profile_create {
                if let Some(dir) = state::get().config_dir.get() {
                    // save the current profile's draft to its own file
                    let draft = out.pre_switch_draft.as_ref()
                        .cloned()
                        .unwrap_or_else(|| (**self.config.load()).clone());
                    crate::lua_writer::save_current_config(&draft, dir);

                    // new profile starts from defaults, saved to profiles/<name>.lua only
                    let mut new_cfg = tuxinjector_config::Config::default();
                    new_cfg.profile = name.clone();
                    crate::lua_writer::save_profile(&new_cfg, dir, name);
                    crate::lua_writer::save_active_profile(dir, name);
                    self.config.publish(new_cfg.clone());
                    self.app.force_update_config(new_cfg);
                    need_refresh = true;
                    tracing::info!(profile = name, "profile created with defaults");
                }
            }

            if let Some(ref target) = out.profile_switch {
                if let Some(dir) = state::get().config_dir.get() {
                    // auto-save current profile's draft to its own file
                    let draft = out.pre_switch_draft.as_ref()
                        .cloned()
                        .unwrap_or_else(|| (**self.config.load()).clone());
                    crate::lua_writer::save_current_config(&draft, dir);

                    if target.is_empty() {
                        // default profile - load init.lua
                        let init = dir.join("init.lua");
                        match std::fs::read_to_string(&init) {
                            Ok(src) => match tuxinjector_lua::load_lua_config(&src) {
                                Ok(mut c) => {
                                    c.profile = String::new();
                                    crate::lua_writer::save_active_profile(dir, "");
                                    self.config.publish(c.clone());
                                    self.app.force_update_config(c);
                                    tracing::info!("switched to default profile");
                                }
                                Err(e) => tracing::error!(error = %e, "failed to parse init.lua"),
                            },
                            Err(e) => tracing::error!(error = %e, "failed to read init.lua"),
                        }
                    } else if let Some(src) = crate::lua_writer::load_profile_source(dir, target) {
                        match tuxinjector_lua::load_lua_config(&src) {
                            Ok(mut c) => {
                                c.profile = target.clone();
                                crate::lua_writer::save_active_profile(dir, target);
                                self.config.publish(c.clone());
                                self.app.force_update_config(c);
                                tracing::info!(profile = target, "switched profile");
                            }
                            Err(e) => tracing::error!(profile = target, error = %e, "failed to load profile"),
                        }
                    } else {
                        tracing::warn!(profile = target, "profile file not found");
                    }
                    need_refresh = true;
                }
            }

            if let Some(ref name) = out.profile_delete {
                if let Some(dir) = state::get().config_dir.get() {
                    crate::lua_writer::delete_profile(dir, name);
                    // if deleting the active profile, fall back to default
                    let cur = self.config.load();
                    if cur.profile == *name {
                        let init = dir.join("init.lua");
                        match std::fs::read_to_string(&init)
                            .map_err(|e| e.to_string())
                            .and_then(|s| tuxinjector_lua::load_lua_config(&s).map_err(|e| e.to_string()))
                        {
                            Ok(mut c) => {
                                c.profile = String::new();
                                crate::lua_writer::save_active_profile(dir, "");
                                self.config.publish(c.clone());
                                self.app.force_update_config(c);
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "failed to reload default after delete");
                                let mut c = (**cur).clone();
                                c.profile = String::new();
                                crate::lua_writer::save_active_profile(dir, "");
                                self.config.publish(c.clone());
                                self.app.force_update_config(c);
                            }
                        }
                    }
                    need_refresh = true;
                    tracing::info!(profile = name, "profile deleted");
                }
            }

            if let Some((ref old, ref new)) = out.profile_rename {
                if let Some(dir) = state::get().config_dir.get() {
                    crate::lua_writer::rename_profile(dir, old, new);
                    let cur = self.config.load();
                    if cur.profile == *old {
                        let mut c = (**cur).clone();
                        c.profile = new.clone();
                        crate::lua_writer::save_active_profile(dir, new);
                        self.config.publish(c.clone());
                        self.app.force_update_config(c);
                    }
                    need_refresh = true;
                    tracing::info!(old = old, new = new, "profile renamed");
                }
            }

            if need_refresh {
                if let Some(dir) = state::get().config_dir.get() {
                    self.app.profile_list = crate::lua_writer::list_profiles(dir);
                }
            }

            tuxinjector_input::set_gui_capture_mode(out.wants_key_capture);
            tuxinjector_input::set_gui_wants_keyboard(self.imgui.io().want_capture_keyboard);

            // sync if app closed itself via Close button
            if !self.app.is_visible() && self.visible {
                self.visible = false;
                tuxinjector_input::set_gui_visible(false);
                tuxinjector_input::set_gui_wants_keyboard(false);
                unsafe { tuxinjector_input::restore_game_cursor(); }
            }
        }
    }
}

// --- perf overlay ---

fn draw_perf_overlay(
    ui: &imgui::Ui,
    snap: &crate::perf_stats::PerfSnapshot,
    pos: tuxinjector_config::types::PerfOverlayPosition,
) {
    use tuxinjector_config::types::PerfOverlayPosition::*;

    let [dw, dh] = ui.io().display_size;
    let (pivot, position) = match pos {
        TopLeft => ([0.0, 0.0], [8.0, 8.0]),
        TopRight => ([1.0, 0.0], [dw - 8.0, 8.0]),
        BottomLeft => ([0.0, 1.0], [8.0, dh - 8.0]),
        BottomRight => ([1.0, 1.0], [dw - 8.0, dh - 8.0]),
    };

    let _bg = ui.push_style_color(StyleColor::WindowBg, [0.08, 0.06, 0.12, 0.82]);
    let _border = ui.push_style_color(StyleColor::Border, [0.39, 0.24, 0.59, 0.24]);

    ui.window("##perf_overlay")
        .title_bar(false)
        .resizable(false)
        .movable(false)
        .scroll_bar(false)
        .always_auto_resize(true)
        .position(position, Condition::Always)
        .position_pivot(pivot)
        .build(|| {
            let col = fps_color(snap.fps);
            ui.text_colored(col, format!("FPS: {:.0}   {:.2} ms", snap.fps, snap.frame_time_ms));

            draw_frame_graph(ui, &snap.frame_times);

            ui.separator();

            ui.text_colored(
                [0.63, 0.78, 1.0, 1.0],
                format!("CPU  {:5.1}%  {:4} MHz  {:4.1} \u{00B0}C",
                    snap.cpu_usage_pct, snap.cpu_freq_mhz, snap.cpu_temp_c),
            );

            ui.text_colored(
                [0.78, 0.63, 1.0, 1.0],
                format!("GPU  {:5.1}%           {:4.1} \u{00B0}C",
                    snap.gpu_usage_pct, snap.gpu_temp_c),
            );
            if snap.gpu_vram_total_mb > 0 {
                ui.text_colored(
                    [0.78, 0.63, 1.0, 1.0],
                    format!("VRAM {}/{} MB", snap.gpu_vram_used_mb, snap.gpu_vram_total_mb),
                );
            }

            ui.separator();

            ui.text_colored(
                [0.63, 0.90, 0.63, 1.0],
                format!("RAM  {}/{} MB", snap.ram_used_mb, snap.ram_total_mb),
            );
        });
}

fn draw_frame_graph(ui: &imgui::Ui, times: &[f32]) {
    if times.is_empty() { return; }

    let gw = 200.0f32;
    let gh = 40.0f32;

    let cursor = ui.cursor_screen_pos();
    ui.dummy([gw, gh]);

    let dl = ui.get_window_draw_list();
    let bar_w = gw / times.len() as f32;

    for (i, &ft) in times.iter().enumerate() {
        // 33.3ms (30fps) = full bar height
        let h = (ft / 33.3 * gh).clamp(1.0, gh);
        let x = cursor[0] + i as f32 * bar_w;
        let y_top = cursor[1] + gh - h;
        let y_bot = cursor[1] + gh;
        let col = fps_color(1000.0 / ft.max(0.001));
        let bits = imgui::ImColor32::from(col).to_bits();
        dl.add_rect_filled_multicolor(
            [x, y_top],
            [x + (bar_w - 0.5).max(0.5), y_bot],
            bits, bits, bits, bits,
        );
    }
}

// --- toast overlay ---

fn draw_toasts(ui: &imgui::Ui, toasts: &[ActiveToast]) {
    if toasts.is_empty() { return; }

    let [dw, dh] = ui.io().display_size;

    for (i, toast) in toasts.iter().enumerate() {
        let alpha = (toast.remaining.min(60) as f32 / 60.0).clamp(0.0, 1.0);

        let _bg = ui.push_style_color(StyleColor::WindowBg, [0.08, 0.06, 0.12, 0.82 * alpha]);
        let _border = ui.push_style_color(StyleColor::Border, [0.39, 0.24, 0.59, 0.24 * alpha]);

        let y = dh - 16.0 - i as f32 * 44.0;

        ui.window(format!("##toast_{i}"))
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .scroll_bar(false)
            .always_auto_resize(true)
            .position([dw / 2.0, y], Condition::Always)
            .position_pivot([0.5, 1.0])
            .build(|| {
                let mut c = toast.color;
                c[3] *= alpha;
                ui.text_colored(c, &toast.msg);
            });
    }
}

fn fps_color(fps: f32) -> [f32; 4] {
    if fps >= 60.0 {
        [0.39, 0.86, 0.39, 1.0] // green
    } else if fps >= 30.0 {
        [1.0, 0.78, 0.20, 1.0] // yellow
    } else {
        [1.0, 0.31, 0.31, 1.0] // red
    }
}

// -- Font Loading --
//
// yeahhhhhh, this was also written by an LLM, i swear not all of my code is like this :sleepy:

// Unicode block coverage ranges for glyph atlas generation.
// Provides broad multilingual support: Latin, Cyrillic, Greek,
// Hebrew, Thai, Tamil, Kannada, and common symbol blocks.
static GLYPH_RANGES: &[u32] = &[
    0x0020, 0x00FF, // Basic Latin + Latin-1 Supplement
    0x0100, 0x024F, // Latin Extended-A + B
    0x0370, 0x03FF, // Greek & Coptic
    0x0400, 0x052F, // Cyrillic + Supplement
    0x0590, 0x05FF, // Hebrew
    0x0B80, 0x0BFF, // Tamil
    0x0C80, 0x0CFF, // Kannada
    0x0E00, 0x0E7F, // Thai
    0x1E00, 0x1EFF, // Latin Extended Additional
    0x2000, 0x206F, // General Punctuation
    0x20A0, 0x20CF, // Currency Symbols
    0x2100, 0x214F, // Letterlike Symbols
    0x2190, 0x21FF, // Arrows
    0x2500, 0x257F, // Box Drawing
    0x25A0, 0x25FF, // Geometric Shapes
    0x2DE0, 0x2DFF, // Cyrillic Extended-A
    0xA640, 0xA69F, // Cyrillic Extended-B
    0,              // Null terminator
];

fn glyph_ranges() -> imgui::FontGlyphRanges {
    imgui::FontGlyphRanges::from_slice(GLYPH_RANGES)
}

fn add_font(imgui: &mut imgui::Context, path: &str) {
    let loaded = if !path.is_empty() {
        match std::fs::read(path) {
            Ok(data) => {
                imgui.fonts().add_font(&[FontSource::TtfData {
                    data: &data,
                    size_pixels: 16.0,
                    config: Some(imgui::FontConfig {
                        oversample_h: 2,
                        oversample_v: 2,
                        glyph_ranges: glyph_ranges(),
                        ..imgui::FontConfig::default()
                    }),
                }]);
                true
            }
            Err(e) => {
                tracing::warn!("failed to load font {path}: {e}");
                false
            }
        }
    } else {
        false
    };

    if !loaded {
        imgui.fonts().add_font(&[FontSource::DefaultFontData {
            config: Some(imgui::FontConfig {
                size_pixels: 16.0,
                oversample_h: 2,
                oversample_v: 2,
                glyph_ranges: glyph_ranges(),
                ..imgui::FontConfig::default()
            }),
        }]);
    }
}

// --- themes ---

fn apply_theme(style: &mut imgui::Style, name: &str) {
    match name {
        "Dracula" => apply_dracula(style),
        "Catppuccin" => apply_catppuccin(style),
        _ => apply_purple(style), // default, because i like purple
    }
    apply_spacing(style);
}

fn apply_spacing(s: &mut imgui::Style) {
    s.item_spacing = [8.0, 4.0];
    s.window_padding = [10.0, 10.0];
    s.frame_padding = [6.0, 3.0];
    s.cell_padding = [10.0, 5.0];
    s.window_rounding = 8.0;
    s.frame_rounding = 4.0;
    s.grab_rounding = 4.0;
    s.tab_rounding = 4.0;
}

fn apply_purple(style: &mut imgui::Style) {
    style.use_dark_colors();
    let c = &mut style.colors;
    c[StyleColor::WindowBg as usize] = [0.10, 0.08, 0.14, 1.00];
    c[StyleColor::ChildBg as usize] = [0.10, 0.08, 0.14, 1.00];
    c[StyleColor::PopupBg as usize] = [0.10, 0.08, 0.14, 0.94];
    c[StyleColor::MenuBarBg as usize] = [0.14, 0.11, 0.20, 1.00];
    c[StyleColor::Border as usize] = [0.50, 0.30, 0.70, 0.50];
    c[StyleColor::BorderShadow as usize] = [0.00, 0.00, 0.00, 0.00];
    c[StyleColor::Text as usize] = [0.95, 0.90, 1.00, 1.00];
    c[StyleColor::TextDisabled as usize] = [0.50, 0.45, 0.60, 1.00];
    c[StyleColor::TextSelectedBg as usize] = [0.55, 0.35, 0.75, 0.35];
    c[StyleColor::FrameBg as usize] = [0.20, 0.15, 0.28, 0.54];
    c[StyleColor::FrameBgHovered as usize] = [0.60, 0.40, 0.80, 0.54];
    c[StyleColor::FrameBgActive as usize] = [0.60, 0.40, 0.80, 0.67];
    c[StyleColor::TitleBg as usize] = [0.10, 0.08, 0.14, 1.00];
    c[StyleColor::TitleBgActive as usize] = [0.20, 0.15, 0.28, 1.00];
    c[StyleColor::TitleBgCollapsed as usize] = [0.10, 0.08, 0.14, 0.51];
    c[StyleColor::Button as usize] = [0.55, 0.35, 0.75, 0.40];
    c[StyleColor::ButtonHovered as usize] = [0.65, 0.45, 0.85, 1.00];
    c[StyleColor::ButtonActive as usize] = [0.75, 0.55, 0.95, 1.00];
    c[StyleColor::Header as usize] = [0.55, 0.35, 0.75, 0.31];
    c[StyleColor::HeaderHovered as usize] = [0.65, 0.45, 0.85, 0.80];
    c[StyleColor::HeaderActive as usize] = [0.65, 0.45, 0.85, 1.00];
    c[StyleColor::Tab as usize] = [0.20, 0.15, 0.28, 0.86];
    c[StyleColor::TabHovered as usize] = [0.65, 0.45, 0.85, 0.80];
    c[StyleColor::TabActive as usize] = [0.55, 0.35, 0.75, 1.00];
    c[StyleColor::SliderGrab as usize] = [0.65, 0.45, 0.85, 1.00];
    c[StyleColor::SliderGrabActive as usize] = [0.75, 0.55, 0.95, 1.00];
    c[StyleColor::CheckMark as usize] = [0.80, 0.60, 1.00, 1.00];
    c[StyleColor::Separator as usize] = [0.50, 0.30, 0.70, 0.50];
    c[StyleColor::SeparatorHovered as usize] = [0.65, 0.45, 0.85, 0.78];
    c[StyleColor::SeparatorActive as usize] = [0.65, 0.45, 0.85, 1.00];
    c[StyleColor::ScrollbarBg as usize] = [0.10, 0.08, 0.14, 0.53];
    c[StyleColor::ScrollbarGrab as usize] = [0.40, 0.25, 0.55, 1.00];
    c[StyleColor::ScrollbarGrabHovered as usize] = [0.55, 0.35, 0.75, 1.00];
    c[StyleColor::ScrollbarGrabActive as usize] = [0.65, 0.45, 0.85, 1.00];
    c[StyleColor::ResizeGrip as usize] = [0.55, 0.35, 0.75, 0.20];
    c[StyleColor::ResizeGripHovered as usize] = [0.65, 0.45, 0.85, 0.67];
    c[StyleColor::ResizeGripActive as usize] = [0.75, 0.55, 0.95, 0.95];
    c[StyleColor::TableHeaderBg as usize] = [0.18, 0.14, 0.24, 1.00];
    c[StyleColor::TableBorderStrong as usize] = [0.50, 0.30, 0.70, 0.50];
    c[StyleColor::TableBorderLight as usize] = [0.40, 0.25, 0.55, 0.30];
    c[StyleColor::TableRowBg as usize] = [0.00, 0.00, 0.00, 0.00];
    c[StyleColor::TableRowBgAlt as usize] = [1.00, 1.00, 1.00, 0.04];
    c[StyleColor::DragDropTarget as usize] = [0.80, 0.60, 1.00, 0.90];
    c[StyleColor::NavHighlight as usize] = [0.65, 0.45, 0.85, 1.00];
    c[StyleColor::NavWindowingHighlight as usize] = [1.00, 1.00, 1.00, 0.70];
    c[StyleColor::NavWindowingDimBg as usize] = [0.80, 0.80, 0.80, 0.20];
    c[StyleColor::PlotLines as usize] = [0.65, 0.45, 0.85, 1.00];
    c[StyleColor::PlotLinesHovered as usize] = [0.80, 0.60, 1.00, 1.00];
    c[StyleColor::PlotHistogram as usize] = [0.55, 0.35, 0.75, 1.00];
    c[StyleColor::PlotHistogramHovered as usize] = [0.75, 0.55, 0.95, 1.00];
    c[StyleColor::ModalWindowDimBg as usize] = [0.00, 0.00, 0.00, 0.50];
}

fn apply_dracula(style: &mut imgui::Style) {
    style.use_dark_colors();
    let c = &mut style.colors;
    c[StyleColor::WindowBg as usize] = [0.16, 0.16, 0.21, 1.00];
    c[StyleColor::ChildBg as usize] = [0.16, 0.16, 0.21, 1.00];
    c[StyleColor::PopupBg as usize] = [0.16, 0.16, 0.21, 0.94];
    c[StyleColor::Border as usize] = [0.27, 0.29, 0.40, 1.00];
    c[StyleColor::Text as usize] = [0.97, 0.98, 0.98, 1.00];
    c[StyleColor::TextDisabled as usize] = [0.38, 0.42, 0.53, 1.00];
    c[StyleColor::FrameBg as usize] = [0.27, 0.29, 0.40, 0.54];
    c[StyleColor::FrameBgHovered as usize] = [0.35, 0.38, 0.53, 0.54];
    c[StyleColor::FrameBgActive as usize] = [0.55, 0.48, 0.76, 0.67];
    c[StyleColor::TitleBg as usize] = [0.16, 0.16, 0.21, 1.00];
    c[StyleColor::TitleBgActive as usize] = [0.16, 0.16, 0.21, 1.00];
    c[StyleColor::TitleBgCollapsed as usize] = [0.16, 0.16, 0.21, 0.51];
    c[StyleColor::Button as usize] = [0.55, 0.48, 0.76, 0.40];
    c[StyleColor::ButtonHovered as usize] = [0.55, 0.48, 0.76, 1.00];
    c[StyleColor::ButtonActive as usize] = [0.98, 0.47, 0.60, 1.00];
    c[StyleColor::Header as usize] = [0.55, 0.48, 0.76, 0.31];
    c[StyleColor::HeaderHovered as usize] = [0.55, 0.48, 0.76, 0.80];
    c[StyleColor::HeaderActive as usize] = [0.55, 0.48, 0.76, 1.00];
    c[StyleColor::Tab as usize] = [0.27, 0.29, 0.40, 0.86];
    c[StyleColor::TabHovered as usize] = [0.55, 0.48, 0.76, 0.80];
    c[StyleColor::TabActive as usize] = [0.55, 0.48, 0.76, 1.00];
    c[StyleColor::SliderGrab as usize] = [0.55, 0.48, 0.76, 1.00];
    c[StyleColor::SliderGrabActive as usize] = [0.98, 0.47, 0.60, 1.00];
    c[StyleColor::CheckMark as usize] = [0.31, 0.98, 0.48, 1.00];
    c[StyleColor::Separator as usize] = [0.27, 0.29, 0.40, 1.00];
    c[StyleColor::ModalWindowDimBg as usize] = [0.00, 0.00, 0.00, 0.50];
}

fn apply_catppuccin(style: &mut imgui::Style) {
    style.use_dark_colors();
    let c = &mut style.colors;
    c[StyleColor::WindowBg as usize] = [0.12, 0.12, 0.18, 1.00];
    c[StyleColor::ChildBg as usize] = [0.12, 0.12, 0.18, 1.00];
    c[StyleColor::PopupBg as usize] = [0.12, 0.12, 0.18, 0.94];
    c[StyleColor::Border as usize] = [0.27, 0.28, 0.35, 1.00];
    c[StyleColor::Text as usize] = [0.81, 0.84, 0.96, 1.00];
    c[StyleColor::TextDisabled as usize] = [0.42, 0.44, 0.53, 1.00];
    c[StyleColor::FrameBg as usize] = [0.17, 0.18, 0.25, 0.54];
    c[StyleColor::FrameBgHovered as usize] = [0.53, 0.56, 0.89, 0.54];
    c[StyleColor::FrameBgActive as usize] = [0.53, 0.56, 0.89, 0.67];
    c[StyleColor::TitleBg as usize] = [0.12, 0.12, 0.18, 1.00];
    c[StyleColor::TitleBgActive as usize] = [0.12, 0.12, 0.18, 1.00];
    c[StyleColor::TitleBgCollapsed as usize] = [0.12, 0.12, 0.18, 0.51];
    c[StyleColor::Button as usize] = [0.53, 0.56, 0.89, 0.40];
    c[StyleColor::ButtonHovered as usize] = [0.53, 0.56, 0.89, 1.00];
    c[StyleColor::ButtonActive as usize] = [0.95, 0.55, 0.66, 1.00];
    c[StyleColor::Header as usize] = [0.53, 0.56, 0.89, 0.31];
    c[StyleColor::HeaderHovered as usize] = [0.53, 0.56, 0.89, 0.80];
    c[StyleColor::HeaderActive as usize] = [0.53, 0.56, 0.89, 1.00];
    c[StyleColor::Tab as usize] = [0.17, 0.18, 0.25, 0.86];
    c[StyleColor::TabHovered as usize] = [0.53, 0.56, 0.89, 0.80];
    c[StyleColor::TabActive as usize] = [0.53, 0.56, 0.89, 1.00];
    c[StyleColor::SliderGrab as usize] = [0.53, 0.56, 0.89, 1.00];
    c[StyleColor::SliderGrabActive as usize] = [0.95, 0.55, 0.66, 1.00];
    c[StyleColor::CheckMark as usize] = [0.65, 0.89, 0.63, 1.00];
    c[StyleColor::Separator as usize] = [0.27, 0.28, 0.35, 1.00];
    c[StyleColor::ModalWindowDimBg as usize] = [0.00, 0.00, 0.00, 0.50];
}

// --- GLFW key → imgui key mapping ---

fn glfw_to_imgui_key(key: i32) -> Option<imgui::Key> {
    match key {
        32 => Some(imgui::Key::Space),
        39 => Some(imgui::Key::Apostrophe),
        44 => Some(imgui::Key::Comma),
        45 => Some(imgui::Key::Minus),
        46 => Some(imgui::Key::Period),
        47 => Some(imgui::Key::Slash),
        48 => Some(imgui::Key::Alpha0), 49 => Some(imgui::Key::Alpha1),
        50 => Some(imgui::Key::Alpha2), 51 => Some(imgui::Key::Alpha3),
        52 => Some(imgui::Key::Alpha4), 53 => Some(imgui::Key::Alpha5),
        54 => Some(imgui::Key::Alpha6), 55 => Some(imgui::Key::Alpha7),
        56 => Some(imgui::Key::Alpha8), 57 => Some(imgui::Key::Alpha9),
        59 => Some(imgui::Key::Semicolon),
        61 => Some(imgui::Key::Equal),
        65 => Some(imgui::Key::A), 66 => Some(imgui::Key::B), 67 => Some(imgui::Key::C),
        68 => Some(imgui::Key::D), 69 => Some(imgui::Key::E), 70 => Some(imgui::Key::F),
        71 => Some(imgui::Key::G), 72 => Some(imgui::Key::H), 73 => Some(imgui::Key::I),
        74 => Some(imgui::Key::J), 75 => Some(imgui::Key::K), 76 => Some(imgui::Key::L),
        77 => Some(imgui::Key::M), 78 => Some(imgui::Key::N), 79 => Some(imgui::Key::O),
        80 => Some(imgui::Key::P), 81 => Some(imgui::Key::Q), 82 => Some(imgui::Key::R),
        83 => Some(imgui::Key::S), 84 => Some(imgui::Key::T), 85 => Some(imgui::Key::U),
        86 => Some(imgui::Key::V), 87 => Some(imgui::Key::W), 88 => Some(imgui::Key::X),
        89 => Some(imgui::Key::Y), 90 => Some(imgui::Key::Z),
        91 => Some(imgui::Key::LeftBracket),
        92 => Some(imgui::Key::Backslash),
        93 => Some(imgui::Key::RightBracket),
        96 => Some(imgui::Key::GraveAccent),
        // Special Charecters 
        256 => Some(imgui::Key::Escape),
        257 => Some(imgui::Key::Enter),
        258 => Some(imgui::Key::Tab),
        259 => Some(imgui::Key::Backspace),
        260 => Some(imgui::Key::Insert),
        261 => Some(imgui::Key::Delete),
        262 => Some(imgui::Key::RightArrow),
        263 => Some(imgui::Key::LeftArrow),
        264 => Some(imgui::Key::DownArrow),
        265 => Some(imgui::Key::UpArrow),
        266 => Some(imgui::Key::PageUp),
        267 => Some(imgui::Key::PageDown),
        268 => Some(imgui::Key::Home),
        269 => Some(imgui::Key::End),
        280 => Some(imgui::Key::CapsLock),
        281 => Some(imgui::Key::ScrollLock),
        282 => Some(imgui::Key::NumLock),
        283 => Some(imgui::Key::PrintScreen),
        284 => Some(imgui::Key::Pause),
        // Function Keys
        290 => Some(imgui::Key::F1), 291 => Some(imgui::Key::F2),
        292 => Some(imgui::Key::F3), 293 => Some(imgui::Key::F4),
        294 => Some(imgui::Key::F5), 295 => Some(imgui::Key::F6),
        296 => Some(imgui::Key::F7), 297 => Some(imgui::Key::F8),
        298 => Some(imgui::Key::F9), 299 => Some(imgui::Key::F10),
        300 => Some(imgui::Key::F11), 301 => Some(imgui::Key::F12),
        // Modifier Keys
        340 => Some(imgui::Key::LeftShift), 341 => Some(imgui::Key::LeftCtrl),
        342 => Some(imgui::Key::LeftAlt),   343 => Some(imgui::Key::LeftSuper),
        344 => Some(imgui::Key::RightShift), 345 => Some(imgui::Key::RightCtrl),
        346 => Some(imgui::Key::RightAlt),   347 => Some(imgui::Key::RightSuper),
        _ => None,
    }
}

// clipboard backend for imgui - shells out to system tools.
// tries wl-paste/wl-copy first (wayland), falls back to xclip (X11).
struct SystemClipboard;

impl imgui::ClipboardBackend for SystemClipboard {
    fn get(&mut self) -> Option<String> {
        // try wayland first, then x11
        std::process::Command::new("wl-paste")
            .arg("--no-newline")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .or_else(|| {
                std::process::Command::new("xclip")
                    .args(["-selection", "clipboard", "-o"])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
            })
            .and_then(|o| String::from_utf8(o.stdout).ok())
    }

    fn set(&mut self, value: &str) {
        // try wayland first
        if let Ok(mut child) = std::process::Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(value.as_bytes());
            }
            return;
        }
        // fall back to xclip
        if let Ok(mut child) = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(value.as_bytes());
            }
        }
    }
}
