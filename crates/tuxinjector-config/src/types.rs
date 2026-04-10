// Config types for the tuxinjector overlay.
//
// This is basically a giant tree of serde structs that maps 1:1 to the
// Lua config file. Each struct has its own Default impl with sensible
// fallbacks so missing fields Just Work.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tuxinjector_core::Color;

use crate::defaults;

// --- Enums ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientAnimationType {
    None,
    Rotate,
    Slide,
    Wave,
    Spiral,
    Fade,
}

impl Default for GradientAnimationType {
    fn default() -> Self {
        Self::None
    }
}

// How to interpret gamma when doing color matching on mirrors
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum MirrorGammaMode {
    Auto = 0,
    #[serde(rename = "assumeSrgb")]
    AssumeSrgb = 1,
    #[serde(rename = "assumeLinear")]
    AssumeLinear = 2,
}

impl Default for MirrorGammaMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorBorderType {
    #[serde(alias = "Dynamic")]
    Dynamic,
    #[serde(alias = "Static")]
    Static,
}

impl Default for MirrorBorderType {
    fn default() -> Self {
        Self::Dynamic
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorBorderShape {
    #[serde(alias = "Rectangle")]
    Rectangle,
    #[serde(alias = "Circle")]
    Circle,
}

impl Default for MirrorBorderShape {
    fn default() -> Self {
        Self::Rectangle
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GameTransitionType {
    #[serde(alias = "Cut")]
    Cut,
    #[serde(alias = "Bounce")]
    Bounce,
}

impl Default for GameTransitionType {
    fn default() -> Self {
        Self::Bounce
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OverlayTransitionType {
    Cut,
}

impl Default for OverlayTransitionType {
    fn default() -> Self {
        Self::Cut
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundTransitionType {
    Cut,
}

impl Default for BackgroundTransitionType {
    fn default() -> Self {
        Self::Cut
    }
}

// What to call when another detour already exists on the same symbol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HookChainingNextTarget {
    LatestHook,
    OriginalFunction,
}

impl Default for HookChainingNextTarget {
    fn default() -> Self {
        Self::LatestHook
    }
}

// --- Gradient ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GradientColorStop {
    pub color: Color,
    pub position: f32,
}

impl Default for GradientColorStop {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            position: 0.0,
        }
    }
}

// --- Background ---

// Can be solid color, image, or gradient
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackgroundConfig {
    #[serde(default = "defaults::background_selected_mode")]
    pub selected_mode: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub color: Color,
    #[serde(default)]
    pub gradient_stops: Vec<GradientColorStop>,
    #[serde(default)]
    pub gradient_angle: f32,
    #[serde(default)]
    pub gradient_animation: GradientAnimationType,
    #[serde(default = "defaults::gradient_animation_speed")]
    pub gradient_animation_speed: f32,
    #[serde(default)]
    pub gradient_color_fade: bool,
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            selected_mode: defaults::background_selected_mode(),
            image: String::new(),
            color: Color::TRANSPARENT,
            gradient_stops: Vec::new(),
            gradient_angle: 0.0,
            gradient_animation: GradientAnimationType::None,
            gradient_animation_speed: 1.0,
            gradient_color_fade: false,
        }
    }
}

// --- Mirror capture / render ---

// Where to grab pixels from on the game framebuffer
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MirrorCaptureConfig {
    pub x: i32,
    pub y: i32,
    #[serde(default = "defaults::relative_to_top_left")]
    pub relative_to: String,
}

impl Default for MirrorCaptureConfig {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            relative_to: defaults::relative_to_top_left(),
        }
    }
}

// Where and how to render the captured mirror on screen
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MirrorRenderConfig {
    pub x: i32,
    pub y: i32,
    #[serde(default)]
    pub use_relative_position: bool,
    #[serde(default = "defaults::mirror_render_relative_x")]
    pub relative_x: f32,
    #[serde(default = "defaults::mirror_render_relative_y")]
    pub relative_y: f32,
    #[serde(default = "defaults::scale_one")]
    pub scale: f32,
    #[serde(default)]
    pub separate_scale: bool,
    #[serde(default = "defaults::scale_x_one")]
    pub scale_x: f32,
    #[serde(default = "defaults::scale_y_one")]
    pub scale_y: f32,
    #[serde(default = "defaults::relative_to_top_left")]
    pub relative_to: String,
    /// When non-zero, overrides `capture_width * scale`
    #[serde(default)]
    pub output_width: i32,
    /// When non-zero, overrides `capture_height * scale`
    #[serde(default)]
    pub output_height: i32,
    // When true, eyezoom layout takes over position and size
    #[serde(default)]
    pub eyezoom_link: bool,
}

impl Default for MirrorRenderConfig {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            use_relative_position: false,
            relative_x: 0.5,
            relative_y: 0.5,
            scale: 1.0,
            separate_scale: false,
            scale_x: 1.0,
            scale_y: 1.0,
            relative_to: defaults::relative_to_top_left(),
            output_width: 0,
            output_height: 0,
            eyezoom_link: false,
        }
    }
}

// --- Mirror colors ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MirrorColors {
    // when false, skip color keying entirely (raw passthrough)
    #[serde(default = "defaults::bool_true")]
    pub enabled: bool,
    #[serde(default)]
    pub target_colors: Vec<Color>,
    #[serde(default)]
    pub output: Color,
    #[serde(default)]
    pub border: Color,
}

impl Default for MirrorColors {
    fn default() -> Self {
        Self {
            enabled: true,
            target_colors: Vec::new(),
            output: Color::BLACK,
            border: Color::BLACK,
        }
    }
}

// --- Mirror border ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MirrorBorderConfig {
    #[serde(default)]
    pub r#type: MirrorBorderType,

    #[serde(default = "defaults::mirror_border_dynamic_thickness")]
    pub dynamic_thickness: i32,

    #[serde(default)]
    pub static_shape: MirrorBorderShape,
    #[serde(default = "defaults::color_white")]
    pub static_color: Color,
    #[serde(default = "defaults::mirror_border_static_thickness")]
    pub static_thickness: i32,
    #[serde(default)]
    pub static_radius: i32,
    #[serde(default)]
    pub static_offset_x: i32,
    #[serde(default)]
    pub static_offset_y: i32,
    #[serde(default)]
    pub static_width: i32,
    #[serde(default)]
    pub static_height: i32,
}

impl Default for MirrorBorderConfig {
    fn default() -> Self {
        Self {
            r#type: MirrorBorderType::Dynamic,
            dynamic_thickness: 1,
            static_shape: MirrorBorderShape::Rectangle,
            static_color: Color::WHITE,
            static_thickness: 2,
            static_radius: 0,
            static_offset_x: 0,
            static_offset_y: 0,
            static_width: 0,
            static_height: 0,
        }
    }
}

// --- Mirror ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MirrorConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default = "defaults::mirror_capture_width")]
    pub capture_width: i32,
    #[serde(default = "defaults::mirror_capture_height")]
    pub capture_height: i32,
    #[serde(default)]
    pub input: Vec<MirrorCaptureConfig>,
    #[serde(default)]
    pub output: MirrorRenderConfig,
    #[serde(default)]
    pub colors: MirrorColors,
    #[serde(default = "defaults::mirror_color_sensitivity")]
    pub color_sensitivity: f32,
    #[serde(default)]
    pub border: MirrorBorderConfig,
    #[serde(default = "defaults::mirror_fps")]
    pub fps: i32,
    #[serde(default = "defaults::opacity_one")]
    pub opacity: f32,
    #[serde(default)]
    pub raw_output: bool,
    #[serde(default)]
    pub color_passthrough: bool,
    // clip to inscribed circle (ellipse if not square)
    #[serde(default)]
    pub clip_circle: bool,
    // nearest-neighbour keeps pixels crisp when scaled up
    #[serde(default)]
    pub nearest_filter: bool,
    // custom frag shader name (loads from shaders/<name>.glsl),
    // empty = built-in filter/passthrough
    #[serde(default)]
    pub shader: String,
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            capture_width: 50,
            capture_height: 50,
            input: Vec::new(),
            output: MirrorRenderConfig::default(),
            colors: MirrorColors::default(),
            color_sensitivity: 0.001,
            border: MirrorBorderConfig::default(),
            fps: 0,
            opacity: 1.0,
            raw_output: false,
            color_passthrough: false,
            clip_circle: false,
            nearest_filter: false,
            shader: String::new(),
        }
    }
}

// --- Mirror groups ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MirrorGroupItem {
    #[serde(default)]
    pub mirror_id: String,
    #[serde(default = "defaults::enabled_true")]
    pub enabled: bool,
    #[serde(default = "defaults::width_percent_one")]
    pub width_percent: f32,
    #[serde(default = "defaults::height_percent_one")]
    pub height_percent: f32,
    #[serde(default)]
    pub offset_x: i32,
    #[serde(default)]
    pub offset_y: i32,
}

impl Default for MirrorGroupItem {
    fn default() -> Self {
        Self {
            mirror_id: String::new(),
            enabled: true,
            width_percent: 1.0,
            height_percent: 1.0,
            offset_x: 0,
            offset_y: 0,
        }
    }
}

// Group of mirrors rendered at a shared position
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MirrorGroupConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub output: MirrorRenderConfig,
    #[serde(default)]
    pub mirrors: Vec<MirrorGroupItem>,
}

impl Default for MirrorGroupConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            output: MirrorRenderConfig::default(),
            mirrors: Vec::new(),
        }
    }
}

// --- Image background / stretch / border / color key ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageBackgroundConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub color: Color,
    #[serde(default = "defaults::opacity_one")]
    pub opacity: f32,
}

impl Default for ImageBackgroundConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            color: Color::BLACK,
            opacity: 1.0,
        }
    }
}

// Game viewport stretch/reposition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StretchConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub width_expr: String,
    #[serde(default)]
    pub height_expr: String,
    #[serde(default)]
    pub x_expr: String,
    #[serde(default)]
    pub y_expr: String,
}

impl Default for StretchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            width: 0,
            height: 0,
            x: 0,
            y: 0,
            width_expr: String::new(),
            height_expr: String::new(),
            x_expr: String::new(),
            y_expr: String::new(),
        }
    }
}

// Border around a mode viewport or image overlay
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BorderConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "defaults::color_white")]
    pub color: Color,
    #[serde(default = "defaults::border_width")]
    pub width: i32,
    #[serde(default)]
    pub radius: i32,
}

impl Default for BorderConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            color: Color::WHITE,
            width: 4,
            radius: 0,
        }
    }
}

// Color-key entry for making parts of an image transparent
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ColorKeyConfig {
    #[serde(default)]
    pub color: Color,
    #[serde(default = "defaults::color_key_sensitivity")]
    pub sensitivity: f32,
}

impl Default for ColorKeyConfig {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            sensitivity: 0.05,
        }
    }
}

// --- Image overlay ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ImageConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default = "defaults::scale_one")]
    pub scale: f32,
    #[serde(default = "defaults::relative_to_top_left")]
    pub relative_to: String,
    #[serde(default)]
    pub crop_top: i32,
    #[serde(default)]
    pub crop_bottom: i32,
    #[serde(default)]
    pub crop_left: i32,
    #[serde(default)]
    pub crop_right: i32,
    #[serde(default)]
    pub enable_color_key: bool,
    #[serde(default)]
    pub color_keys: Vec<ColorKeyConfig>,
    // legacy single color key - use color_keys[] instead
    #[serde(default)]
    pub color_key: Color,
    #[serde(default = "defaults::image_color_key_sensitivity")]
    pub color_key_sensitivity: f32,
    #[serde(default = "defaults::opacity_one")]
    pub opacity: f32,
    #[serde(default)]
    pub background: ImageBackgroundConfig,
    #[serde(default)]
    pub pixelated_scaling: bool,
    #[serde(default)]
    pub border: BorderConfig,
    /// When non-zero, overrides `tex_width * scale`
    #[serde(default)]
    pub output_width: i32,
    /// When non-zero, overrides `tex_height * scale`
    #[serde(default)]
    pub output_height: i32,
    // eyezoom layout overrides position/size when true
    #[serde(default)]
    pub eyezoom_link: bool,
}

impl Default for ImageConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            path: String::new(),
            x: 0,
            y: 0,
            scale: 1.0,
            relative_to: defaults::relative_to_top_left(),
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
            enable_color_key: false,
            color_keys: Vec::new(),
            color_key: Color::BLACK,
            color_key_sensitivity: 0.001,
            opacity: 1.0,
            background: ImageBackgroundConfig::default(),
            pixelated_scaling: false,
            border: BorderConfig::default(),
            output_width: 0,
            output_height: 0,
            eyezoom_link: false,
        }
    }
}

// --- Text overlay ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TextOverlayConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub text: String,
    #[serde(default = "defaults::text_overlay_font_size")]
    pub font_size: i32,
    #[serde(default = "defaults::color_white")]
    pub color: Color,
    #[serde(default = "defaults::opacity_one")]
    pub opacity: f32,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default = "defaults::relative_to_top_left")]
    pub relative_to: String,
    // font path override; empty = use the theme's font
    #[serde(default)]
    pub font_path: String,
    #[serde(default = "defaults::text_overlay_padding")]
    pub padding: i32,
    #[serde(default)]
    pub background: ImageBackgroundConfig,
    #[serde(default)]
    pub border: BorderConfig,
}

impl Default for TextOverlayConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            text: String::new(),
            font_size: defaults::text_overlay_font_size(),
            color: Color::WHITE,
            opacity: 1.0,
            x: 0,
            y: 0,
            relative_to: defaults::relative_to_top_left(),
            font_path: String::new(),
            padding: defaults::text_overlay_padding(),
            background: ImageBackgroundConfig::default(),
            border: BorderConfig::default(),
        }
    }
}

// --- Window overlay ---

// Captures an external window and renders it as an overlay
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowOverlayConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub window_title: String,
    #[serde(default)]
    pub window_class: String,
    #[serde(default)]
    pub executable_name: String,
    #[serde(default = "defaults::window_overlay_match_priority")]
    pub window_match_priority: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default = "defaults::scale_one")]
    pub scale: f32,
    #[serde(default = "defaults::relative_to_top_left")]
    pub relative_to: String,
    #[serde(default)]
    pub crop_top: i32,
    #[serde(default)]
    pub crop_bottom: i32,
    #[serde(default)]
    pub crop_left: i32,
    #[serde(default)]
    pub crop_right: i32,
    #[serde(default)]
    pub enable_color_key: bool,
    #[serde(default)]
    pub color_keys: Vec<ColorKeyConfig>,
    // legacy single color key - prefer color_keys[]
    #[serde(default)]
    pub color_key: Color,
    #[serde(default = "defaults::image_color_key_sensitivity")]
    pub color_key_sensitivity: f32,
    #[serde(default = "defaults::opacity_one")]
    pub opacity: f32,
    #[serde(default)]
    pub background: ImageBackgroundConfig,
    #[serde(default)]
    pub pixelated_scaling: bool,
    #[serde(default = "defaults::window_overlay_fps")]
    pub fps: i32,
    #[serde(default = "defaults::window_overlay_search_interval")]
    pub search_interval: i32,
    // capture method ("pipewire" on linux)
    #[serde(default = "defaults::window_overlay_capture_method")]
    pub capture_method: String,
    #[serde(default)]
    pub enable_interaction: bool,
    #[serde(default)]
    pub border: BorderConfig,
}

impl Default for WindowOverlayConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            window_title: String::new(),
            window_class: String::new(),
            executable_name: String::new(),
            window_match_priority: defaults::window_overlay_match_priority(),
            x: 0,
            y: 0,
            scale: 1.0,
            relative_to: defaults::relative_to_top_left(),
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
            enable_color_key: false,
            color_keys: Vec::new(),
            color_key: Color::BLACK,
            color_key_sensitivity: 0.001,
            opacity: 1.0,
            background: ImageBackgroundConfig::default(),
            pixelated_scaling: false,
            fps: 30,
            search_interval: 1000,
            capture_method: defaults::window_overlay_capture_method(),
            enable_interaction: false,
            border: BorderConfig::default(),
        }
    }
}

// Renders a web page offscreen and composites it as an overlay.
// Uses the tuxinjector-browser helper process (WebKitGTK).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BrowserOverlayConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub custom_css: String,
    #[serde(default = "defaults::browser_overlay_width")]
    pub width: i32,
    #[serde(default = "defaults::browser_overlay_height")]
    pub height: i32,
    #[serde(default = "defaults::browser_overlay_fps")]
    pub fps: i32,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default = "defaults::scale_one")]
    pub scale: f32,
    #[serde(default = "defaults::relative_to_top_left")]
    pub relative_to: String,
    #[serde(default = "defaults::opacity_one")]
    pub opacity: f32,
    #[serde(default)]
    pub crop_top: i32,
    #[serde(default)]
    pub crop_bottom: i32,
    #[serde(default)]
    pub crop_left: i32,
    #[serde(default)]
    pub crop_right: i32,
    #[serde(default)]
    pub pixelated_scaling: bool,
    #[serde(default)]
    pub transparent_background: bool,
    #[serde(default)]
    pub border: BorderConfig,
}

impl Default for BrowserOverlayConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            url: String::new(),
            custom_css: String::new(),
            width: 800,
            height: 600,
            fps: 15,
            x: 0,
            y: 0,
            scale: 1.0,
            relative_to: defaults::relative_to_top_left(),
            opacity: 1.0,
            crop_top: 0,
            crop_bottom: 0,
            crop_left: 0,
            crop_right: 0,
            pixelated_scaling: false,
            transparent_background: true,
            border: BorderConfig::default(),
        }
    }
}

// --- Mode ---

// A display mode controls viewport size, which overlays are visible,
// and how transitions animate between modes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModeConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default)]
    pub use_relative_size: bool,
    #[serde(default = "defaults::mode_relative_width")]
    pub relative_width: f32,
    #[serde(default = "defaults::mode_relative_height")]
    pub relative_height: f32,

    #[serde(default)]
    pub width_expr: String,
    #[serde(default)]
    pub height_expr: String,

    #[serde(default)]
    pub background: BackgroundConfig,
    #[serde(default)]
    pub mirror_ids: Vec<String>,
    #[serde(default)]
    pub mirror_group_ids: Vec<String>,
    #[serde(default)]
    pub image_ids: Vec<String>,
    #[serde(default)]
    pub window_overlay_ids: Vec<String>,
    #[serde(default)]
    pub browser_overlay_ids: Vec<String>,
    #[serde(default)]
    pub text_overlay_ids: Vec<String>,
    #[serde(default)]
    pub stretch: StretchConfig,

    #[serde(default)]
    pub game_transition: GameTransitionType,
    #[serde(default)]
    pub overlay_transition: OverlayTransitionType,
    #[serde(default)]
    pub background_transition: BackgroundTransitionType,
    #[serde(default = "defaults::transition_duration_ms")]
    pub transition_duration_ms: i32,

    #[serde(default = "defaults::ease_in_power")]
    pub ease_in_power: f32,
    #[serde(default = "defaults::ease_out_power")]
    pub ease_out_power: f32,
    #[serde(default)]
    pub bounce_count: i32,
    #[serde(default = "defaults::bounce_intensity")]
    pub bounce_intensity: f32,
    #[serde(default = "defaults::bounce_duration_ms")]
    pub bounce_duration_ms: i32,
    #[serde(default)]
    pub relative_stretching: bool,
    #[serde(default)]
    pub skip_animate_x: bool,
    #[serde(default)]
    pub skip_animate_y: bool,

    #[serde(default)]
    pub border: BorderConfig,
    #[serde(default)]
    pub sensitivity_override_enabled: bool,
    #[serde(default = "defaults::mode_sensitivity")]
    pub mode_sensitivity: f32,
    #[serde(default)]
    pub separate_xy_sensitivity: bool,
    #[serde(default = "defaults::mode_sensitivity_x")]
    pub mode_sensitivity_x: f32,
    #[serde(default = "defaults::mode_sensitivity_y")]
    pub mode_sensitivity_y: f32,

    #[serde(default)]
    pub slide_mirrors_in: bool,
    #[serde(default)]
    pub enable_eyezoom: bool,
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            width: 0,
            height: 0,
            use_relative_size: false,
            relative_width: 0.5,
            relative_height: 0.5,
            width_expr: String::new(),
            height_expr: String::new(),
            background: BackgroundConfig::default(),
            mirror_ids: Vec::new(),
            mirror_group_ids: Vec::new(),
            image_ids: Vec::new(),
            window_overlay_ids: Vec::new(),
            browser_overlay_ids: Vec::new(),
            text_overlay_ids: Vec::new(),
            stretch: StretchConfig::default(),
            game_transition: GameTransitionType::Bounce,
            overlay_transition: OverlayTransitionType::Cut,
            background_transition: BackgroundTransitionType::Cut,
            transition_duration_ms: 500,
            ease_in_power: 1.0,
            ease_out_power: 3.0,
            bounce_count: 0,
            bounce_intensity: 0.15,
            bounce_duration_ms: 150,
            relative_stretching: false,
            skip_animate_x: false,
            skip_animate_y: false,
            border: BorderConfig::default(),
            sensitivity_override_enabled: false,
            mode_sensitivity: 1.0,
            separate_xy_sensitivity: false,
            mode_sensitivity_x: 1.0,
            mode_sensitivity_y: 1.0,
            slide_mirrors_in: false,
            enable_eyezoom: false,
        }
    }
}

// --- Hotkeys ---

// Conditions that need to be met for a hotkey to fire
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HotkeyConditions {
    #[serde(default)]
    pub game_state: Vec<String>,
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub exclusions: Vec<u32>,
}

impl Default for HotkeyConditions {
    fn default() -> Self {
        Self {
            game_state: Vec::new(),
            exclusions: Vec::new(),
        }
    }
}

// Alternate secondary mode triggered by a different key combo
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AltSecondaryMode {
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub keys: Vec<u32>,
    #[serde(default)]
    pub mode: String,
}

impl Default for AltSecondaryMode {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            mode: String::new(),
        }
    }
}

// Hotkey that toggles between primary and secondary modes
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HotkeyConfig {
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub keys: Vec<u32>,
    #[serde(default)]
    pub main_mode: String,
    #[serde(default)]
    pub secondary_mode: String,
    #[serde(default)]
    pub alt_secondary_modes: Vec<AltSecondaryMode>,
    #[serde(default)]
    pub conditions: HotkeyConditions,
    #[serde(default = "defaults::hotkey_debounce")]
    pub debounce: i32,
    #[serde(default)]
    pub trigger_on_release: bool,
    #[serde(default)]
    pub block_key_from_game: bool,
    #[serde(default)]
    pub allow_exit_to_fullscreen_regardless_of_game_state: bool,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            main_mode: String::new(),
            secondary_mode: String::new(),
            alt_secondary_modes: Vec::new(),
            conditions: HotkeyConditions::default(),
            debounce: 0,
            trigger_on_release: false,
            block_key_from_game: false,
            allow_exit_to_fullscreen_regardless_of_game_state: false,
        }
    }
}

// Hotkey that overrides mouse sensitivity while held/toggled
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SensitivityHotkeyConfig {
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub keys: Vec<u32>,
    #[serde(default = "defaults::sensitivity_one")]
    pub sensitivity: f32,
    #[serde(default)]
    pub separate_xy: bool,
    #[serde(default = "defaults::sensitivity_one")]
    pub sensitivity_x: f32,
    #[serde(default = "defaults::sensitivity_one")]
    pub sensitivity_y: f32,
    #[serde(default)]
    pub toggle: bool,
    #[serde(default)]
    pub conditions: HotkeyConditions,
    #[serde(default = "defaults::sensitivity_debounce")]
    pub debounce: i32,
}

impl Default for SensitivityHotkeyConfig {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            sensitivity: 1.0,
            separate_xy: false,
            sensitivity_x: 1.0,
            sensitivity_y: 1.0,
            toggle: false,
            conditions: HotkeyConditions::default(),
            debounce: 0,
        }
    }
}

// --- Debug ---

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PerfOverlayPosition {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

// Debug/diagnostic toggles - mostly for development
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DebugGlobalConfig {
    #[serde(default)]
    pub show_performance_overlay: bool,
    #[serde(default)]
    pub perf_overlay_position: PerfOverlayPosition,
    #[serde(default)]
    pub show_profiler: bool,
    #[serde(default = "defaults::profiler_scale")]
    pub profiler_scale: f32,
    #[serde(default)]
    pub show_hotkey_debug: bool,
    #[serde(default)]
    pub fake_cursor: bool,
    #[serde(default)]
    pub show_texture_grid: bool,
    #[serde(default)]
    pub delay_rendering_until_finished: bool,
    #[serde(default)]
    pub delay_rendering_until_blitted: bool,
    #[serde(default)]
    pub virtual_camera_enabled: bool,
    #[serde(default = "defaults::virtual_camera_fps")]
    pub virtual_camera_fps: i32,
    #[serde(default)]
    pub log_mode_switch: bool,
    #[serde(default)]
    pub log_animation: bool,
    #[serde(default)]
    pub log_hotkey: bool,
    #[serde(default)]
    pub log_obs: bool,
    #[serde(default)]
    pub log_window_overlay: bool,
    #[serde(default)]
    pub log_file_monitor: bool,
    #[serde(default)]
    pub log_image_monitor: bool,
    #[serde(default)]
    pub log_performance: bool,
    #[serde(default)]
    pub log_texture_ops: bool,
    #[serde(default)]
    pub log_gui: bool,
    #[serde(default)]
    pub log_init: bool,
    #[serde(default)]
    pub log_cursor_textures: bool,
}

impl Default for DebugGlobalConfig {
    fn default() -> Self {
        Self {
            show_performance_overlay: false,
            perf_overlay_position: PerfOverlayPosition::TopLeft,
            show_profiler: false,
            profiler_scale: 0.8,
            show_hotkey_debug: false,
            fake_cursor: false,
            show_texture_grid: false,
            delay_rendering_until_finished: false,
            delay_rendering_until_blitted: false,
            virtual_camera_enabled: false,
            virtual_camera_fps: 60,
            log_mode_switch: false,
            log_animation: false,
            log_hotkey: false,
            log_obs: false,
            log_window_overlay: false,
            log_file_monitor: false,
            log_image_monitor: false,
            log_performance: false,
            log_texture_ops: false,
            log_gui: false,
            log_init: false,
            log_cursor_textures: false,
        }
    }
}

// --- Cursors ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CursorConfig {
    #[serde(default)]
    pub cursor_name: String,
    #[serde(default = "defaults::cursor_size")]
    pub cursor_size: i32,
    #[serde(default)]
    pub hotspot_x: f32,
    #[serde(default)]
    pub hotspot_y: f32,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            cursor_name: String::new(),
            cursor_size: 64,
            hotspot_x: 0.0,
            hotspot_y: 0.0,
        }
    }
}

// Per-game-state cursor (title screen, wall, ingame)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CursorsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub title: CursorConfig,
    #[serde(default)]
    pub wall: CursorConfig,
    #[serde(default)]
    pub ingame: CursorConfig,
}

impl Default for CursorsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            title: CursorConfig::default(),
            wall: CursorConfig::default(),
            ingame: CursorConfig::default(),
        }
    }
}

// --- EyeZoom ---

// Pixel-inspection overlay for eye measurements.
// Defaults are tuned for 1080p minecraft.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EyeZoomConfig {
    #[serde(default = "defaults::eyezoom_clone_width")]
    pub clone_width: i32,
    #[serde(default = "defaults::eyezoom_overlay_width")]
    pub overlay_width: i32,
    #[serde(default = "defaults::eyezoom_clone_height")]
    pub clone_height: i32,
    #[serde(default = "defaults::eyezoom_stretch_width")]
    pub stretch_width: i32,
    #[serde(default = "defaults::eyezoom_window_width")]
    pub window_width: i32,
    #[serde(default = "defaults::eyezoom_window_height")]
    pub window_height: i32,
    #[serde(default)]
    pub horizontal_margin: i32,
    #[serde(default)]
    pub vertical_margin: i32,
    #[serde(default = "defaults::eyezoom_auto_font_size")]
    pub auto_font_size: bool,
    #[serde(default = "defaults::eyezoom_text_font_size")]
    pub text_font_size: i32,
    #[serde(default)]
    pub text_font_path: String,
    #[serde(default = "defaults::eyezoom_rect_height")]
    pub rect_height: i32,
    #[serde(default = "defaults::eyezoom_link_rect_to_font")]
    pub link_rect_to_font: bool,
    #[serde(default = "defaults::eyezoom_grid_color1")]
    pub grid_color1: Color,
    #[serde(default = "defaults::opacity_one")]
    pub grid_color1_opacity: f32,
    #[serde(default = "defaults::eyezoom_grid_color2")]
    pub grid_color2: Color,
    #[serde(default = "defaults::opacity_one")]
    pub grid_color2_opacity: f32,
    #[serde(default = "defaults::eyezoom_center_line_color")]
    pub center_line_color: Color,
    #[serde(default = "defaults::opacity_one")]
    pub center_line_color_opacity: f32,
    #[serde(default = "defaults::eyezoom_text_color")]
    pub text_color: Color,
    #[serde(default = "defaults::opacity_one")]
    pub text_color_opacity: f32,
    #[serde(default = "defaults::eyezoom_highlight_color")]
    pub highlight_color: Color,
    #[serde(default = "defaults::opacity_one")]
    pub highlight_color_opacity: f32,
    #[serde(default = "defaults::eyezoom_highlight_interval")]
    pub highlight_interval: i32,
    #[serde(default = "defaults::eyezoom_number_style")]
    pub number_style: String,
    #[serde(default)]
    pub slide_zoom_in: bool,
    #[serde(default)]
    pub slide_mirrors_in: bool,
}

impl Default for EyeZoomConfig {
    fn default() -> Self {
        Self {
            clone_width: 30,
            overlay_width: 30,
            clone_height: 1300,
            stretch_width: 810,
            window_width: 384,
            window_height: 16384,
            horizontal_margin: 0,
            vertical_margin: 0,
            auto_font_size: true,
            text_font_size: 42,
            text_font_path: String::new(),
            rect_height: 40,
            link_rect_to_font: false,
            grid_color1: defaults::eyezoom_grid_color1(),
            grid_color1_opacity: 1.0,
            grid_color2: defaults::eyezoom_grid_color2(),
            grid_color2_opacity: 1.0,
            center_line_color: Color::WHITE,
            center_line_color_opacity: 1.0,
            text_color: Color::BLACK,
            text_color_opacity: 1.0,
            highlight_color: defaults::eyezoom_highlight_color(),
            highlight_color_opacity: 1.0,
            highlight_interval: 10,
            number_style: String::from("slackow"),
            slide_zoom_in: true,
            slide_mirrors_in: true,
        }
    }
}

// --- Appearance ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppearanceConfig {
    #[serde(default = "defaults::appearance_theme")]
    pub theme: String,
    #[serde(default = "defaults::custom_colors_empty")]
    pub custom_colors: HashMap<String, Color>,
    #[serde(default = "defaults::gui_scale")]
    pub gui_scale: f32,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "Purple".into(),
            custom_colors: HashMap::new(),
            gui_scale: 0.8,
        }
    }
}

// --- Key rebinding ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyRebind {
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode", serialize_with = "crate::key_names::serialize_keycode")]
    pub from_key: u32,
    // target key during gameplay (cursor grabbed)
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode", serialize_with = "crate::key_names::serialize_keycode")]
    pub to_key: u32,
    // target key in menus/chat (cursor free). 0 = same as to_key
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode", serialize_with = "crate::key_names::serialize_keycode")]
    pub to_key_chat: u32,
    #[serde(default = "defaults::enabled_true")]
    pub enabled: bool,
}

impl Default for KeyRebind {
    fn default() -> Self {
        Self {
            from_key: 0,
            to_key: 0,
            to_key_chat: 0,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyRebindsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub rebinds: Vec<KeyRebind>,
}

impl Default for KeyRebindsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rebinds: Vec::new(),
        }
    }
}

// --- Input config ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct InputConfig {
    #[serde(default = "defaults::mouse_sensitivity")]
    pub mouse_sensitivity: f32,
    #[serde(default)]
    pub windows_mouse_speed: i32,
    #[serde(default)]
    pub allow_cursor_escape: bool,
    #[serde(default)]
    pub key_repeat_start_delay: i32,
    #[serde(default)]
    pub key_repeat_delay: i32,
    #[serde(default)]
    pub key_rebinds: KeyRebindsConfig,
    #[serde(default)]
    pub sensitivity_hotkeys: Vec<SensitivityHotkeyConfig>,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 1.0,
            windows_mouse_speed: 0,
            allow_cursor_escape: true,
            key_repeat_start_delay: 0,
            key_repeat_delay: 0,
            key_rebinds: KeyRebindsConfig::default(),
            sensitivity_hotkeys: Vec::new(),
        }
    }
}

// --- Theme config ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThemeConfig {
    #[serde(default = "defaults::font_path")]
    pub font_path: String,
    #[serde(default)]
    pub appearance: AppearanceConfig,
    #[serde(default)]
    pub cursors: CursorsConfig,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            font_path: String::new(),
            appearance: AppearanceConfig::default(),
            cursors: CursorsConfig {
                enabled: false,
                title: CursorConfig { cursor_name: String::new(), cursor_size: 32, hotspot_x: 0.0, hotspot_y: 0.0 },
                wall: CursorConfig { cursor_name: String::new(), cursor_size: 32, hotspot_x: 0.0, hotspot_y: 0.0 },
                ingame: CursorConfig { cursor_name: String::new(), cursor_size: 32, hotspot_x: 0.5, hotspot_y: 0.5 },
            },
        }
    }
}

// --- Overlays config ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OverlaysConfig {
    #[serde(default)]
    pub mirrors: Vec<MirrorConfig>,
    #[serde(default)]
    pub mirror_groups: Vec<MirrorGroupConfig>,
    #[serde(default)]
    pub images: Vec<ImageConfig>,
    #[serde(default)]
    pub window_overlays: Vec<WindowOverlayConfig>,
    #[serde(default)]
    pub browser_overlays: Vec<BrowserOverlayConfig>,
    #[serde(default)]
    pub text_overlays: Vec<TextOverlayConfig>,
    #[serde(default)]
    pub eyezoom: EyeZoomConfig,
}

impl Default for OverlaysConfig {
    fn default() -> Self {
        Self {
            mirrors: vec![
                // Pie chart -- the F3+S pie area, color passthrough with static circle border
                MirrorConfig {
                    name: "pieChart".into(),
                    capture_width: 319,
                    capture_height: 169,
                    color_passthrough: true,
                    color_sensitivity: 0.001,
                    colors: MirrorColors {
                        enabled: true,
                        target_colors: vec![
                            Color::from_rgba8(70, 206, 102, 255),  // green
                            Color::from_rgba8(236, 110, 78, 255),  // orange
                            Color::from_rgba8(228, 70, 196, 255),  // pink
                            Color::from_rgba8(204, 108, 70, 255),  // brown
                            Color::from_rgba8(70, 76, 70, 255),    // dark green
                        ],
                        output: Color::from_rgba8(239, 187, 187, 255),
                        border: Color::from_rgba8(186, 64, 64, 255),
                    },
                    border: MirrorBorderConfig {
                        r#type: MirrorBorderType::Static,
                        static_shape: MirrorBorderShape::Circle,
                        static_color: Color::from_rgba8(98, 5, 113, 255),
                        static_thickness: 3,
                        static_radius: 171,
                        static_width: 238,
                        static_height: 235,
                        static_offset_x: 1,
                        static_offset_y: -7,
                        dynamic_thickness: 4,
                    },
                    input: vec![MirrorCaptureConfig { x: -238, y: -180, relative_to: "pieLeft".into() }],
                    output: MirrorRenderConfig {
                        x: 410, y: 31,
                        relative_to: "centerViewport".into(),
                        separate_scale: true, scale: 1.14, scale_x: 0.76, scale_y: 1.5,
                        ..MirrorRenderConfig::default()
                    },
                    ..MirrorConfig::default()
                },
                // E counter -- grey F3 text -> white
                MirrorConfig {
                    name: "eCounter".into(),
                    capture_width: 23,
                    capture_height: 7,
                    nearest_filter: true,
                    color_sensitivity: 0.001,
                    colors: MirrorColors {
                        enabled: true,
                        target_colors: vec![Color::from_rgba8(221, 221, 221, 255)],
                        output: Color::WHITE,
                        border: Color::BLACK,
                    },
                    border: MirrorBorderConfig {
                        dynamic_thickness: 2,
                        ..MirrorBorderConfig::default()
                    },
                    input: vec![MirrorCaptureConfig { x: 14, y: 38, relative_to: "topLeftViewport".into() }],
                    output: MirrorRenderConfig {
                        x: 362, y: 169, scale: 8.0,
                        relative_to: "centerViewport".into(),
                        ..MirrorRenderConfig::default()
                    },
                    ..MirrorConfig::default()
                },
                // Blockentities left -- orange -> amber
                MirrorConfig {
                    name: "blockentitiesLeft".into(),
                    capture_width: 11,
                    capture_height: 7,
                    nearest_filter: true,
                    color_sensitivity: 0.001,
                    colors: MirrorColors {
                        enabled: true,
                        target_colors: vec![Color::from_rgba8(233, 109, 77, 255)],
                        output: Color::from_rgba8(243, 169, 78, 255),
                        border: Color::from_rgba8(90, 54, 14, 255),
                    },
                    border: MirrorBorderConfig {
                        dynamic_thickness: 2,
                        ..MirrorBorderConfig::default()
                    },
                    input: vec![MirrorCaptureConfig { x: 0, y: 0, relative_to: "pieLeft".into() }],
                    output: MirrorRenderConfig {
                        x: 416, y: -38, scale: 8.0,
                        relative_to: "centerViewport".into(),
                        ..MirrorRenderConfig::default()
                    },
                    ..MirrorConfig::default()
                },
                // Unspecified left -- green passthrough
                MirrorConfig {
                    name: "unspecifiedLeft".into(),
                    capture_width: 11,
                    capture_height: 7,
                    nearest_filter: true,
                    color_passthrough: true,
                    color_sensitivity: 0.001,
                    colors: MirrorColors {
                        enabled: true,
                        target_colors: vec![
                            Color::from_rgba8(69, 204, 101, 255),
                            Color::from_rgba8(69, 203, 101, 255),
                        ],
                        output: Color::BLACK,
                        border: Color::from_rgba8(51, 88, 48, 255),
                    },
                    border: MirrorBorderConfig {
                        dynamic_thickness: 2,
                        ..MirrorBorderConfig::default()
                    },
                    input: vec![MirrorCaptureConfig { x: 0, y: 0, relative_to: "pieLeft".into() }],
                    output: MirrorRenderConfig {
                        x: 416, y: 23, scale: 8.0,
                        relative_to: "centerViewport".into(),
                        ..MirrorRenderConfig::default()
                    },
                    ..MirrorConfig::default()
                },
                // Mapless -- orange -> white
                MirrorConfig {
                    name: "mapless".into(),
                    capture_width: 19,
                    capture_height: 7,
                    nearest_filter: true,
                    color_sensitivity: 0.001,
                    colors: MirrorColors {
                        enabled: true,
                        target_colors: vec![Color::from_rgba8(233, 109, 77, 255)],
                        output: Color::WHITE,
                        border: Color::BLACK,
                    },
                    border: MirrorBorderConfig {
                        dynamic_thickness: 2,
                        ..MirrorBorderConfig::default()
                    },
                    input: vec![MirrorCaptureConfig { x: 0, y: 0, relative_to: "pieRight".into() }],
                    output: MirrorRenderConfig {
                        x: 71, y: 242, scale: 8.0,
                        relative_to: "bottomRightViewport".into(),
                        ..MirrorRenderConfig::default()
                    },
                    ..MirrorConfig::default()
                },
                // Eye measurement mirror -- raw output for the tall viewport
                MirrorConfig {
                    name: "eyeMirror".into(),
                    capture_width: 60,
                    capture_height: 580,
                    nearest_filter: true,
                    raw_output: true,
                    colors: MirrorColors {
                        enabled: false,
                        ..MirrorColors::default()
                    },
                    input: vec![MirrorCaptureConfig { x: 162, y: 7902, ..MirrorCaptureConfig::default() }],
                    output: MirrorRenderConfig {
                        eyezoom_link: true,
                        ..MirrorRenderConfig::default()
                    },
                    ..MirrorConfig::default()
                },
            ],
            mirror_groups: Vec::new(),
            images: vec![
                ImageConfig {
                    name: "measuringOverlay".into(),
                    #[cfg(target_os = "macos")]
                    path: "~/.config/tuxinjector/images/overlay.png".into(),
                    #[cfg(target_os = "linux")]
                    path: "~/.local/share/tuxinjector/images/overlay.png".into(),
                    eyezoom_link: true,
                    output_width: 900, output_height: 500,
                    ..ImageConfig::default()
                },
            ],
            window_overlays: Vec::new(),
            browser_overlays: Vec::new(),
            text_overlays: Vec::new(),
            eyezoom: EyeZoomConfig::default(),
        }
    }
}

// --- Display config ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DisplayConfig {
    #[serde(default = "defaults::default_mode")]
    pub default_mode: String,
    #[serde(default)]
    pub fps_limit: i32,
    #[serde(default = "defaults::fps_limit_sleep_threshold")]
    pub fps_limit_sleep_threshold: i32,
    #[serde(default)]
    pub mirror_gamma_mode: MirrorGammaMode,
    #[serde(default)]
    pub disable_animations: bool,
    #[serde(default)]
    pub hide_animations_in_game: bool,
    // minecraft's GUI scale setting - pie chart anchors scale with this
    #[serde(default = "defaults::game_gui_scale")]
    pub game_gui_scale: u32,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            default_mode: defaults::default_mode(),
            fps_limit: 0,
            fps_limit_sleep_threshold: 1000,
            mirror_gamma_mode: MirrorGammaMode::Auto,
            disable_animations: false,
            hide_animations_in_game: true,
            game_gui_scale: defaults::game_gui_scale(),
        }
    }
}

// --- Global hotkeys config ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GlobalHotkeysConfig {
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub gui: Vec<u32>,
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub borderless: Vec<u32>,
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub image_overlays: Vec<u32>,
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub window_overlays: Vec<u32>,
    #[serde(default, deserialize_with = "crate::key_names::deserialize_keycode_vec", serialize_with = "crate::key_names::serialize_keycode_vec")]
    pub app_visibility: Vec<u32>,
    #[serde(default)]
    pub mode_hotkeys: Vec<HotkeyConfig>,
}

impl Default for GlobalHotkeysConfig {
    fn default() -> Self {
        Self {
            gui: vec![341, 73], // LCtrl+I
            borderless: Vec::new(),
            image_overlays: Vec::new(),
            window_overlays: Vec::new(),
            app_visibility: Vec::new(),
            mode_hotkeys: vec![
                // Z: Fullscreen <-> Thin
                HotkeyConfig {
                    keys: vec![90],
                    main_mode: "Fullscreen".into(),
                    secondary_mode: "Thin".into(),
                    debounce: 100,
                    block_key_from_game: true,
                    conditions: HotkeyConditions {
                        exclusions: vec![292],
                        ..HotkeyConditions::default()
                    },
                    ..HotkeyConfig::default()
                },
                // J: Fullscreen <-> Tall
                HotkeyConfig {
                    keys: vec![74],
                    main_mode: "Fullscreen".into(),
                    secondary_mode: "Tall".into(),
                    debounce: 100,
                    block_key_from_game: true,
                    conditions: HotkeyConditions {
                        exclusions: vec![292],
                        ..HotkeyConditions::default()
                    },
                    ..HotkeyConfig::default()
                },
                // Left Alt: Fullscreen <-> Wide
                HotkeyConfig {
                    keys: vec![342],
                    main_mode: "Fullscreen".into(),
                    secondary_mode: "Wide".into(),
                    debounce: 100,
                    block_key_from_game: true,
                    conditions: HotkeyConditions {
                        exclusions: vec![292],
                        ..HotkeyConditions::default()
                    },
                    ..HotkeyConfig::default()
                },
            ],
        }
    }
}

// --- Advanced config ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AdvancedConfig {
    #[serde(default = "defaults::disable_hook_chaining")]
    pub disable_hook_chaining: bool,
    #[serde(default)]
    pub hook_chaining_next_target: HookChainingNextTarget,
    #[serde(default)]
    pub disable_fullscreen_prompt: bool,
    #[serde(default)]
    pub disable_configure_prompt: bool,
    #[serde(default)]
    pub debug: DebugGlobalConfig,
}

impl Default for AdvancedConfig {
    fn default() -> Self {
        Self {
            disable_hook_chaining: true,
            hook_chaining_next_target: HookChainingNextTarget::LatestHook,
            disable_fullscreen_prompt: false,
            disable_configure_prompt: false,
            debug: DebugGlobalConfig::default(),
        }
    }
}

// --- Top-level Config ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    #[serde(default = "defaults::config_version")]
    pub config_version: i32,
    #[serde(default)]
    pub modes: Vec<ModeConfig>,
    #[serde(default)]
    pub input: InputConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub overlays: OverlaysConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    #[serde(default)]
    pub hotkeys: GlobalHotkeysConfig,
    #[serde(default)]
    pub advanced: AdvancedConfig,
    // active profile name; empty = default
    #[serde(default)]
    pub profile: String,
}

impl Default for Config {
    fn default() -> Self {
        // shared border for all non-fullscreen modes (purple)
        let mode_border = BorderConfig {
            enabled: true,
            color: Color::from_rgba8(122, 21, 162, 255),
            width: 1,
            radius: 0,
        };

        // shared mode background (purple-to-dark-blue gradient)
        let mode_bg = || BackgroundConfig {
            selected_mode: "gradient".into(),
            gradient_angle: 45.0,
            gradient_animation: GradientAnimationType::Wave,
            gradient_animation_speed: 0.5,
            gradient_stops: vec![
                GradientColorStop {
                    color: Color::from_rgba8(84, 11, 128, 255),
                    position: 0.0,
                },
                GradientColorStop {
                    color: Color::from_rgba8(21, 0, 72, 255),
                    position: 1.0,
                },
            ],
            ..BackgroundConfig::default()
        };

        Self {
            config_version: 1,
            modes: vec![
                ModeConfig {
                    id: "Fullscreen".into(),
                    use_relative_size: true,
                    relative_width: 1.0,
                    relative_height: 1.0,
                    game_transition: GameTransitionType::Bounce,
                    transition_duration_ms: 300,
                    slide_mirrors_in: true,
                    mirror_ids: vec!["mapless".into()],
                    ..ModeConfig::default()
                },
                ModeConfig {
                    id: "Thin".into(),
                    width_expr: "max(330, roundEven(sw / 8))".into(),
                    height_expr: "roundEven(sh * 0.95)".into(),
                    game_transition: GameTransitionType::Bounce,
                    transition_duration_ms: 300,
                    bounce_intensity: 0.02,
                    bounce_duration_ms: 200,
                    slide_mirrors_in: true,
                    mirror_ids: vec![
                        "pieChart".into(), "eCounter".into(),
                        "blockentitiesLeft".into(), "unspecifiedLeft".into(),
                        "mapless".into(),
                    ],
                    border: mode_border.clone(),
                    background: mode_bg(),
                    ..ModeConfig::default()
                },
                ModeConfig {
                    id: "Wide".into(),
                    width_expr: "roundEven(sw * 0.98)".into(),
                    use_relative_size: true,
                    relative_height: 0.25,
                    game_transition: GameTransitionType::Bounce,
                    transition_duration_ms: 300,
                    bounce_intensity: 0.02,
                    bounce_duration_ms: 200,
                    slide_mirrors_in: true,
                    border: mode_border.clone(),
                    background: mode_bg(),
                    ..ModeConfig::default()
                },
                ModeConfig {
                    id: "Tall".into(),
                    width: 384,
                    height: 16384,
                    game_transition: GameTransitionType::Bounce,
                    transition_duration_ms: 300,
                    skip_animate_y: true,
                    slide_mirrors_in: true,
                    enable_eyezoom: true,
                    mirror_ids: vec!["eyeMirror".into(), "eCounter".into()],
                    image_ids: vec!["measuringOverlay".into()],
                    border: mode_border.clone(),
                    background: mode_bg(),
                    ..ModeConfig::default()
                },
                ModeConfig {
                    id: "Preemptive".into(),
                    width: 384,
                    height: 16384,
                    game_transition: GameTransitionType::Bounce,
                    transition_duration_ms: 300,
                    skip_animate_y: true,
                    slide_mirrors_in: true,
                    mirror_ids: vec![
                        "pieChart".into(), "eCounter".into(),
                        "blockentitiesLeft".into(), "unspecifiedLeft".into(),
                    ],
                    border: mode_border,
                    background: mode_bg(),
                    ..ModeConfig::default()
                },
            ],
            input: InputConfig::default(),
            theme: ThemeConfig::default(),
            overlays: OverlaysConfig::default(),
            display: DisplayConfig::default(),
            hotkeys: GlobalHotkeysConfig::default(),
            advanced: AdvancedConfig::default(),
            profile: String::new(),
        }
    }
}
