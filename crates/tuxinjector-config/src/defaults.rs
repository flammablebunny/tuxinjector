// Serde default-value helpers.
//
// Each fn returns a sensible fallback so serde can fill in missing keys.
// Serde's `default = "path"` attribute demands a function path for every
// single field, so we end up with this parade of one-liners. It is what it is.

use std::collections::HashMap;
use tuxinjector_core::Color;

pub fn color_white() -> Color { Color::WHITE }

pub fn background_selected_mode() -> String { "none".into() }
pub fn relative_to_top_left() -> String { "topLeftScreen".into() }

pub fn mirror_render_relative_x() -> f32 { 0.5 }
pub fn mirror_render_relative_y() -> f32 { 0.5 }

// All return 1.0 but serde needs distinct fn paths. Blame the macro system, not me :woman_shrugging:
pub fn scale_one() -> f32 { 1.0 }
pub fn scale_x_one() -> f32 { 1.0 }
pub fn scale_y_one() -> f32 { 1.0 }

pub fn mirror_capture_width() -> i32 { 300 }
pub fn mirror_capture_height() -> i32 { 200 }

pub fn mirror_color_sensitivity() -> f32 { 0.001 }

pub fn mirror_fps() -> i32 { 0 } // 0 means uncapped

pub fn opacity_one() -> f32 { 1.0 }

pub fn mirror_border_dynamic_thickness() -> i32 { 1 }
pub fn mirror_border_static_thickness() -> i32 { 2 }

pub fn width_percent_one() -> f32 { 1.0 }
pub fn height_percent_one() -> f32 { 1.0 }

pub fn enabled_true() -> bool { true }
pub fn bool_true() -> bool { true }

pub fn border_width() -> i32 { 4 }

pub fn color_key_sensitivity() -> f32 { 0.05 }
pub fn image_color_key_sensitivity() -> f32 { 0.001 }

pub fn window_overlay_match_priority() -> String { "title".into() }
pub fn window_overlay_fps() -> i32 { 30 }
pub fn window_overlay_search_interval() -> i32 { 1000 }
pub fn window_overlay_capture_method() -> String { "pipewire".into() }

pub fn mode_relative_width() -> f32 { 0.5 }
pub fn mode_relative_height() -> f32 { 0.5 }

pub fn transition_duration_ms() -> i32 { 500 }
pub fn ease_in_power() -> f32 { 1.0 }
pub fn ease_out_power() -> f32 { 3.0 }
pub fn bounce_intensity() -> f32 { 0.15 }
pub fn bounce_duration_ms() -> i32 { 150 }

pub fn mode_sensitivity() -> f32 { 1.0 }
pub fn mode_sensitivity_x() -> f32 { 1.0 }
pub fn mode_sensitivity_y() -> f32 { 1.0 }

pub fn gradient_animation_speed() -> f32 { 1.0 }

pub fn hotkey_debounce() -> i32 { 0 }

pub fn sensitivity_one() -> f32 { 1.0 }
pub fn sensitivity_debounce() -> i32 { 0 }

pub fn profiler_scale() -> f32 { 0.8 }
pub fn virtual_camera_fps() -> i32 { 60 }
pub fn cursor_size() -> i32 { 64 }

// --- eyezoom defaults ---
// Tuned for 1080p minecraft. I'll probably want to
// adjust these for other resolutions.

pub fn eyezoom_clone_width() -> i32 { 30 }
pub fn eyezoom_overlay_width() -> i32 { 12 }
pub fn eyezoom_clone_height() -> i32 { 1300 }
pub fn eyezoom_stretch_width() -> i32 { 810 }
pub fn eyezoom_window_width() -> i32 { 384 }
pub fn eyezoom_window_height() -> i32 { 16384 }

pub fn eyezoom_auto_font_size() -> bool { true }
pub fn eyezoom_text_font_size() -> i32 { 42 }
pub fn eyezoom_rect_height() -> i32 { 40 }
pub fn eyezoom_link_rect_to_font() -> bool { false }

// light pink
pub fn eyezoom_grid_color1() -> Color {
    Color { r: 1.0, g: 0.714, b: 0.757, a: 1.0 }
}

// light blue
pub fn eyezoom_grid_color2() -> Color {
    Color { r: 0.678, g: 0.847, b: 0.902, a: 1.0 }
}

pub fn eyezoom_center_line_color() -> Color { Color::WHITE }
pub fn eyezoom_text_color() -> Color { Color::BLACK }

// amber-ish highlight (#FFD54F)
pub fn eyezoom_highlight_color() -> Color {
    Color { r: 1.0, g: 0.835, b: 0.310, a: 1.0 }
}

pub fn eyezoom_highlight_interval() -> i32 { 10 }
pub fn eyezoom_number_style() -> String { String::from("slackow") }

pub fn appearance_theme() -> String { "Purple".into() }
pub fn custom_colors_empty() -> HashMap<String, Color> { HashMap::new() }
pub fn gui_scale() -> f32 { 0.8 }

pub fn game_gui_scale() -> u32 { 3 }
pub fn config_version() -> i32 { 1 }
pub fn default_mode() -> String { "Fullscreen".into() }

// Empty string = imgui's built-in ProggyClean font.
pub fn font_path() -> String { String::new() }

pub fn fps_limit_sleep_threshold() -> i32 { 1000 }
pub fn mouse_sensitivity() -> f32 { 1.0 }

pub fn text_overlay_font_size() -> i32 { 24 }
pub fn text_overlay_padding() -> i32 { 4 }

pub fn disable_hook_chaining() -> bool { true }
