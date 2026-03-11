// Manages mode transitions and produces per-frame FrameLayout instructions
// for the overlay pipeline to render.

use std::time::Instant;

use tuxinjector_config::types::{
    BackgroundConfig, BorderConfig, Config, EyeZoomConfig, GradientAnimationType, ImageConfig,
    MirrorBorderType, MirrorConfig, MirrorRenderConfig, ModeConfig, TextOverlayConfig,
    WindowOverlayConfig,
};
use tuxinjector_config::expr::{evaluate_expression, is_expression};
use tuxinjector_core::geometry::{
    GameViewportGeometry, RelativeTo, resolve_relative_position,
};
use tuxinjector_core::transition::{self, EasingType, TransitionState};

// --- Layout output types ---

#[derive(Debug, Clone)]
pub struct FrameLayout {
    pub viewport_x: f32,
    pub viewport_y: f32,
    pub viewport_width: f32,
    pub viewport_height: f32,
    pub background: BackgroundSpec,
    pub mirrors: Vec<MirrorLayout>,
    pub images: Vec<ImageLayout>,
    pub text_overlays: Vec<TextOverlayLayout>,
    pub window_overlays: Vec<WindowOverlayLayout>,
    pub border: Option<BorderLayout>,
    pub eyezoom: Option<EyeZoomLayout>,
}

#[derive(Debug, Clone)]
pub struct MirrorLayout {
    pub mirror_id: String,
    pub render_x: f32,
    pub render_y: f32,
    pub render_width: f32,
    pub render_height: f32,
    pub circle_clip: bool,
    pub nearest_filter: bool,

    // color filter (empty target_colors = passthrough)
    pub filter_target_colors: Vec<[f32; 4]>,
    pub filter_output_color: [f32; 4],
    pub filter_sensitivity: f32,
    pub filter_color_passthrough: bool,

    // dynamic border
    pub filter_border_color: [f32; 4],
    pub filter_border_width: i32,
    pub filter_gamma_mode: i32, // 0=Auto, 1=sRGB, 2=Linear

    // static border
    pub static_border_enabled: bool,
    pub static_border_color: [f32; 4],
    pub static_border_width: f32,
    pub static_border_radius: f32,
    pub static_border_rect_w: f32,    // 0 = use render_width
    pub static_border_rect_h: f32,    // 0 = use render_height
    pub static_border_offset_x: f32,
    pub static_border_offset_y: f32,
    pub custom_shader: String, // empty = use built-in
}

#[derive(Debug, Clone)]
pub struct ImageLayout {
    pub image_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub opacity: f32,
}

#[derive(Debug, Clone)]
pub struct TextOverlayLayout {
    pub text_overlay_id: String,
    pub x: f32,
    pub y: f32,
    pub opacity: f32,
}

#[derive(Debug, Clone)]
pub struct WindowOverlayLayout {
    pub wo_id: String,
    pub x: f32,
    pub y: f32,
    pub scale: f32,
    pub opacity: f32,
    pub crop_top: i32,
    pub crop_bottom: i32,
    pub crop_left: i32,
    pub crop_right: i32,
    pub pixelated_scaling: bool,
    pub border: Option<BorderLayout>,
}

#[derive(Debug, Clone)]
pub enum BackgroundSpec {
    None,
    SolidColor([f32; 4]),
    Gradient {
        stops: Vec<([f32; 4], f32)>,
        angle: f32,
        animation: GradientAnimationType,
        speed: f32,
    },
    Image {
        path: String,
    },
}

#[derive(Debug, Clone)]
pub struct BorderLayout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub border_width: f32,
    pub radius: f32,
    pub color: [f32; 4],
}

#[derive(Debug, Clone)]
pub struct EyeZoomLayout {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    // mirror/overlay rect, scales proportionally to screen height
    pub mirror_x: f32,
    pub mirror_y: f32,
    pub mirror_w: f32,
    pub mirror_h: f32,
    // source region in game framebuffer
    pub src_x: i32,
    pub src_y: i32,
    pub src_w: i32,
    pub src_h: i32,
    pub clone_width: i32,
    pub overlay_width: i32,
    pub box_height: f32,
    pub grid_color1: [f32; 4],
    pub grid_color2: [f32; 4],
    pub center_line_color: [f32; 4],
}

// --- Active transition ---

#[derive(Debug, Clone)]
pub struct ActiveTransition {
    pub from_mode: String,
    pub to_mode: String,
    pub state: TransitionState,
    pub started_at: Instant,
    pub duration_ms: u32,
    pub is_bounce: bool,
    pub ease_in_power: f32,
    pub ease_out_power: f32,
    pub bounce_count: i32,
    pub bounce_intensity: f32,
    pub bounce_duration_ms: u32,
    pub skip_animate_x: bool,
    pub skip_animate_y: bool,
}

// --- ModeSystem ---

pub struct ModeSystem {
    cur_mode: String,
    initial_mode: String, // for toggle fallback
    target_mode: Option<String>,
    transition: Option<ActiveTransition>,
    screen_w: u32,
    screen_h: u32,
    settled_w: f32,  // "from" dims for the next transition
    settled_h: f32,
}

impl ModeSystem {
    pub fn new(default_mode: &str) -> Self {
        Self {
            cur_mode: default_mode.to_string(),
            initial_mode: default_mode.to_string(),
            target_mode: None,
            transition: None,
            screen_w: 1920,
            screen_h: 1080,
            settled_w: 1920.0,
            settled_h: 1080.0,
        }
    }

    pub fn update_screen_size(&mut self, w: u32, h: u32) {
        self.screen_w = w;
        self.screen_h = h;
        // seed settled dims so the first transition doesn't animate from (0,0)
        if self.settled_w <= 0.0 && self.transition.is_none() {
            self.settled_w = w as f32;
            self.settled_h = h as f32;
        }
    }

    pub fn current_mode_id(&self) -> &str {
        &self.cur_mode
    }

    // target if transitioning, otherwise settled mode
    pub fn effective_mode_id(&self) -> &str {
        self.target_mode.as_deref().unwrap_or(&self.cur_mode)
    }

    pub fn initial_mode_id(&self) -> &str {
        &self.initial_mode
    }

    pub fn dimensions_for_mode(&self, mode_id: &str, config: &Config) -> Option<(u32, u32)> {
        let mode = find_mode(config, mode_id)?;
        let (w, h) = self.resolve_dimensions(mode);
        Some((w.round() as u32, h.round() as u32))
    }

    pub fn is_transitioning(&self) -> bool {
        self.transition.is_some()
    }

    // --- Mode switching ---

    pub fn switch_mode(&mut self, target_id: &str, config: &Config) {
        if target_id == self.effective_mode_id() {
            return;
        }

        let target = match find_mode(config, target_id) {
            Some(m) => m,
            None => {
                tracing::warn!(mode = target_id, "switch_mode: not found in config");
                return;
            }
        };

        // figure out the "from" dimensions (may be mid-transition)
        let (from_w, from_h) = if let Some(ref t) = self.transition {
            let (cw, ch) = t.state.current_size(
                EasingType::EaseOut, t.ease_in_power, t.ease_out_power,
            );
            (cw.max(1) as f32, ch.max(1) as f32)
        } else {
            let sw = self.screen_w as f32;
            let sh = self.screen_h as f32;
            (
                if self.settled_w > 0.0 { self.settled_w } else { sw },
                if self.settled_h > 0.0 { self.settled_h } else { sh },
            )
        };

        let (to_w, to_h) = self.resolve_dimensions(target);
        let from_mode = self.effective_mode_id().to_string();

        let is_bounce = !config.display.disable_animations
            && matches!(
                target.game_transition,
                tuxinjector_config::types::GameTransitionType::Bounce
            );

        if is_bounce && target.transition_duration_ms > 0 {
            let dur = target.transition_duration_ms as u32;

            let mut ts = TransitionState::default();
            ts.start(
                from_w.round() as i32, from_h.round() as i32,
                to_w.round() as i32, to_h.round() as i32,
                dur,
            );

            self.transition = Some(ActiveTransition {
                from_mode,
                to_mode: target_id.to_string(),
                state: ts,
                started_at: Instant::now(),
                duration_ms: dur,
                is_bounce,
                ease_in_power: target.ease_in_power,
                ease_out_power: target.ease_out_power,
                bounce_count: target.bounce_count,
                bounce_intensity: target.bounce_intensity,
                bounce_duration_ms: target.bounce_duration_ms as u32,
                skip_animate_x: target.skip_animate_x,
                skip_animate_y: target.skip_animate_y,
            });
            self.target_mode = Some(target_id.to_string());
        } else {
            // instant cut
            self.transition = None;
            self.target_mode = None;
            self.cur_mode = target_id.to_string();
            self.settled_w = to_w;
            self.settled_h = to_h;
        };
    }

    // --- Per-frame tick ---

    pub fn tick(&mut self, config: &Config) -> FrameLayout {
        let (vp_w, vp_h) = self.advance_transition(config);

        // use target mode for scene building during transitions
        let active_id = self.target_mode.as_deref().unwrap_or(&self.cur_mode);

        let mode = find_mode(config, active_id)
            .or_else(|| find_mode(config, &self.cur_mode));

        let mode = match mode {
            Some(m) => m,
            None => return self.empty_layout(vp_w, vp_h),
        };

        // center viewport in the physical surface
        let vp_x = ((self.screen_w as f32 - vp_w) / 2.0).max(0.0);
        let vp_y = (self.screen_h as f32 - vp_h) / 2.0;
        let (vp_x, vp_y, vp_w, vp_h) = self.apply_stretch(mode, vp_x, vp_y, vp_w, vp_h);

        let viewport = GameViewportGeometry {
            game_w: vp_w.round() as i32,
            game_h: vp_h.round() as i32,
            final_x: vp_x.round() as i32,
            final_y: vp_y.round() as i32,
            final_w: vp_w.round() as i32,
            final_h: vp_h.round() as i32,
        };

        let bg = build_background(&mode.background);

        let eyezoom = if mode.enable_eyezoom {
            build_eyezoom(
                &config.overlays.eyezoom,
                vp_x, vp_w,
                self.screen_w as f32, self.screen_h as f32,
            )
        } else {
            None
        };

        let mirrors = self.build_mirrors(config, mode, &viewport, eyezoom.as_ref());
        let images = self.build_images(config, mode, &viewport, eyezoom.as_ref());
        let texts = self.build_text_overlays(config, mode, &viewport);
        let woverlays = self.build_window_overlays(config, mode, &viewport);
        let border = if mode.border.enabled {
            Some(build_border(&mode.border, vp_x, vp_y, vp_w, vp_h))
        } else {
            None
        };

        if tracing::enabled!(tracing::Level::DEBUG) {
            tracing::debug!(
                mode = active_id,
                vp_w, vp_h, vp_x, vp_y,
                mirror_count = mirrors.len(),
                mode_mirror_ids = ?mode.mirror_ids,
                "tick: layout computed"
            );
        }

        FrameLayout {
            viewport_x: vp_x,
            viewport_y: vp_y,
            viewport_width: vp_w,
            viewport_height: vp_h,
            background: bg,
            mirrors,
            images,
            text_overlays: texts,
            window_overlays: woverlays,
            border,
            eyezoom,
        }
    }

    // --- Dimension resolution ---

    pub fn resolve_dimensions(&self, mode: &ModeConfig) -> (f32, f32) {
        let sw = self.screen_w as i32;
        let sh = self.screen_h as i32;

        // macOS Retina: mode dimensions are in GLFW window points, but
        // screen_w/screen_h are framebuffer pixels. Scale absolute mode
        // dimensions by the content scale factor so they match.
        #[cfg(target_os = "macos")]
        let (scale_x, scale_y) = unsafe { crate::viewport_hook::content_scale() };
        #[cfg(target_os = "linux")]
        let (scale_x, scale_y) = (1.0f32, 1.0f32);

        let w = if !mode.width_expr.is_empty() && is_expression(&mode.width_expr) {
            let res = evaluate_expression(&mode.width_expr, sw, sh);
            if let Err(ref e) = res {
                tracing::warn!(
                    mode = %mode.id, expr = %mode.width_expr, sw, sh,
                    error = %e, fallback = mode.width,
                    "resolve_dimensions: width expression failed"
                );
            }
            res.unwrap_or(mode.width) as f32
        } else if mode.use_relative_size {
            self.screen_w as f32 * mode.relative_width
        } else if mode.width > 0 {
            mode.width as f32 * scale_x
        } else {
            self.screen_w as f32
        };

        let h = if !mode.height_expr.is_empty() && is_expression(&mode.height_expr) {
            let res = evaluate_expression(&mode.height_expr, sw, sh);
            if let Err(ref e) = res {
                tracing::warn!(
                    mode = %mode.id, expr = %mode.height_expr, sw, sh,
                    error = %e, fallback = mode.height,
                    "resolve_dimensions: height expression failed"
                );
            }
            res.unwrap_or(mode.height) as f32
        } else if mode.use_relative_size {
            self.screen_h as f32 * mode.relative_height
        } else if mode.height > 0 {
            mode.height as f32 * scale_y
        } else {
            self.screen_h as f32
        };

        tracing::debug!(
            mode = %mode.id, w, h, sw, sh,
            width_expr = %mode.width_expr,
            height_expr = %mode.height_expr,
            "resolve_dimensions"
        );

        (w, h)
    }

    // --- Internals ---

    fn advance_transition(&mut self, config: &Config) -> (f32, f32) {
        if let Some(ref mut t) = self.transition {
            let still_going = t.state.update();

            let (mut iw, mut ih) = t.state.current_size(
                EasingType::EaseOut, t.ease_in_power, t.ease_out_power,
            );

            // bounce overlay on the interpolated size
            if t.bounce_count > 0 && t.state.progress < 1.0 {
                let bt = transition::bounce(
                    t.state.progress, t.bounce_count, t.bounce_intensity,
                );
                let fw = t.state.from_w as f32;
                let fh = t.state.from_h as f32;
                let tw = t.state.to_w as f32;
                let th = t.state.to_h as f32;

                if !t.skip_animate_x {
                    iw = (fw + (tw - fw) * bt).round() as i32;
                }
                if !t.skip_animate_y {
                    ih = (fh + (th - fh) * bt).round() as i32;
                }
            }

            // skip_animate_x/y: snap that axis to target immediately
            if t.skip_animate_x { iw = t.state.to_w; }
            if t.skip_animate_y { ih = t.state.to_h; }

            if !still_going {
                // transition complete
                let final_id = t.to_mode.clone();
                let final_w = t.state.to_w as f32;
                let final_h = t.state.to_h as f32;
                self.transition = None;
                self.target_mode = None;
                self.cur_mode = final_id;
                self.settled_w = final_w;
                self.settled_h = final_h;
                return (final_w, final_h);
            }

            return (iw as f32, ih as f32);
        }

        // no transition -- just return current mode dims
        let dims = if let Some(mode) = find_mode(config, &self.cur_mode) {
            self.resolve_dimensions(mode)
        } else {
            (self.screen_w as f32, self.screen_h as f32)
        };

        self.settled_w = dims.0;
        self.settled_h = dims.1;
        dims
    }

    fn apply_stretch(
        &self, mode: &ModeConfig,
        vp_x: f32, vp_y: f32, vp_w: f32, vp_h: f32,
    ) -> (f32, f32, f32, f32) {
        let st = &mode.stretch;
        if !st.enabled {
            return (vp_x, vp_y, vp_w, vp_h);
        }

        let sw = self.screen_w as i32;
        let sh = self.screen_h as i32;
        let s_w = resolve_stretch_dim(&st.width_expr, st.width, sw, sh);
        let s_h = resolve_stretch_dim(&st.height_expr, st.height, sw, sh);
        let s_x = resolve_stretch_dim(&st.x_expr, st.x, sw, sh);
        let s_y = resolve_stretch_dim(&st.y_expr, st.y, sw, sh);

        let fw = if s_w > 0 { s_w as f32 } else { vp_w };
        let fh = if s_h > 0 { s_h as f32 } else { vp_h };
        (vp_x + s_x as f32, vp_y + s_y as f32, fw, fh)
    }

    fn build_mirrors(
        &self,
        config: &Config,
        mode: &ModeConfig,
        viewport: &GameViewportGeometry,
        eyezoom: Option<&EyeZoomLayout>,
    ) -> Vec<MirrorLayout> {
        let mut out = Vec::with_capacity(mode.mirror_ids.len());
        let gamma = config.display.mirror_gamma_mode as i32;
        let gs = config.display.game_gui_scale;

        for mid in &mode.mirror_ids {
            let mirror = match find_mirror(config, mid) {
                Some(m) => m,
                None => continue,
            };
            if let Some(ml) = self.layout_mirror(mid, mirror, viewport, gamma, eyezoom, gs) {
                out.push(ml);
            }
        }

        // mirror groups
        for gid in &mode.mirror_group_ids {
            if let Some(group) = find_mirror_group(config, gid) {
                for item in &group.mirrors {
                    if !item.enabled { continue; }
                    if let Some(mirror) = find_mirror(config, &item.mirror_id) {
                        // build hybrid config: group anchor + mirror scale,
                        // combined offsets for positioning
                        let mut hybrid = mirror.output.clone();
                        hybrid.relative_to = group.output.relative_to.clone();
                        hybrid.x = group.output.x + item.offset_x;
                        hybrid.y = group.output.y + item.offset_y;
                        hybrid.use_relative_position = group.output.use_relative_position;
                        hybrid.relative_x = group.output.relative_x;
                        hybrid.relative_y = group.output.relative_y;

                        // per-item sizing overrides
                        if item.width_percent != 1.0 || item.height_percent != 1.0 {
                            let bsx = if mirror.output.separate_scale {
                                mirror.output.scale_x
                            } else {
                                mirror.output.scale
                            };
                            let bsy = if mirror.output.separate_scale {
                                mirror.output.scale_y
                            } else {
                                mirror.output.scale
                            };
                            hybrid.separate_scale = true;
                            hybrid.scale_x = bsx * item.width_percent;
                            hybrid.scale_y = bsy * item.height_percent;
                        }

                        if let Some(ml) = self.layout_mirror_from_render(
                            &item.mirror_id, mirror, &hybrid, viewport, gamma, eyezoom, gs,
                        ) {
                            out.push(ml);
                        }
                    }
                }
            }
        }

        out
    }

    fn layout_mirror(
        &self,
        mid: &str,
        mirror: &MirrorConfig,
        viewport: &GameViewportGeometry,
        gamma: i32,
        eyezoom: Option<&EyeZoomLayout>,
        gui_scale: u32,
    ) -> Option<MirrorLayout> {
        self.layout_mirror_from_render(mid, mirror, &mirror.output, viewport, gamma, eyezoom, gui_scale)
    }

    fn layout_mirror_from_render(
        &self,
        mid: &str,
        mirror: &MirrorConfig,
        render: &MirrorRenderConfig,
        viewport: &GameViewportGeometry,
        gamma: i32,
        eyezoom: Option<&EyeZoomLayout>,
        gui_scale: u32,
    ) -> Option<MirrorLayout> {
        let anchor = parse_relative_to(&render.relative_to);

        let (sx, sy) = if render.separate_scale {
            (render.scale_x, render.scale_y)
        } else {
            (render.scale, render.scale)
        };

        let base_w = if render.output_width > 0 {
            render.output_width as f32
        } else {
            mirror.capture_width as f32 * sx
        };
        let base_h = if render.output_height > 0 {
            render.output_height as f32
        } else {
            mirror.capture_height as f32 * sy
        };

        // eyezoom_link: position + size come from the eyezoom panel
        let (pos_x, pos_y, bw, bh) = if render.eyezoom_link {
            if let Some(ez) = eyezoom {
                (ez.mirror_x, ez.mirror_y, ez.mirror_w, ez.mirror_h)
            } else {
                // no eyezoom active - nothing to link to, skip
                return None;
            }
        } else if render.use_relative_position {
            let rx = viewport.final_x as f32 + viewport.final_w as f32 * render.relative_x;
            let ry = viewport.final_y as f32 + viewport.final_h as f32 * render.relative_y;
            (rx, ry, base_w, base_h)
        } else {
            let (ax, ay) = resolve_relative_position(
                anchor, render.x, render.y,
                self.screen_w as i32, self.screen_h as i32,
                viewport, base_w as i32, base_h as i32,
                gui_scale, display_scale(),
            );
            (ax as f32, ay as f32, base_w, base_h)
        };

        let target_colors: Vec<[f32; 4]> = if mirror.raw_output || !mirror.colors.enabled {
            Vec::new()
        } else {
            mirror.colors.target_colors
                .iter()
                .take(4)
                .map(|c| [c.r, c.g, c.b, c.a])
                .collect()
        };

        let dyn_border_w = if matches!(mirror.border.r#type, MirrorBorderType::Dynamic) {
            mirror.border.dynamic_thickness
        } else {
            0
        };

        Some(MirrorLayout {
            mirror_id: mid.to_string(),
            render_x: pos_x,
            render_y: pos_y,
            render_width: bw,
            render_height: bh,
            circle_clip: mirror.clip_circle,
            nearest_filter: mirror.nearest_filter,

            filter_target_colors: target_colors,
            filter_output_color: {
                let c = &mirror.colors.output;
                [c.r, c.g, c.b, c.a]
            },
            filter_sensitivity: mirror.color_sensitivity,
            filter_color_passthrough: mirror.color_passthrough,

            filter_border_color: {
                let c = &mirror.colors.border;
                [c.r, c.g, c.b, c.a]
            },
            filter_border_width: dyn_border_w,
            filter_gamma_mode: gamma,

            static_border_enabled: matches!(mirror.border.r#type, MirrorBorderType::Static),
            static_border_color: {
                let c = &mirror.border.static_color;
                [c.r, c.g, c.b, c.a]
            },
            static_border_width: mirror.border.static_thickness as f32,
            static_border_radius: mirror.border.static_radius as f32,
            static_border_rect_w: mirror.border.static_width as f32,
            static_border_rect_h: mirror.border.static_height as f32,
            static_border_offset_x: mirror.border.static_offset_x as f32,
            static_border_offset_y: mirror.border.static_offset_y as f32,
            custom_shader: mirror.shader.clone(),
        })
    }

    fn build_images(
        &self,
        config: &Config,
        mode: &ModeConfig,
        viewport: &GameViewportGeometry,
        eyezoom: Option<&EyeZoomLayout>,
    ) -> Vec<ImageLayout> {
        let mut out = Vec::with_capacity(mode.image_ids.len());
        let gs = config.display.game_gui_scale;

        for img_id in &mode.image_ids {
            let img = match find_image(config, img_id) {
                Some(i) => i,
                None => continue,
            };

            let (ax, ay, w, h) = if img.eyezoom_link {
                if let Some(ez) = eyezoom {
                    (ez.mirror_x, ez.mirror_y, ez.mirror_w, ez.mirror_h)
                } else {
                    // eyezoom isn't active, nothing to anchor to
                    continue;
                }
            } else {
                let anchor = parse_relative_to(&img.relative_to);
                let iw = if img.output_width > 0 { img.output_width } else { 0 };
                let ih = if img.output_height > 0 { img.output_height } else { 0 };
                let (ax, ay) = resolve_relative_position(
                    anchor, img.x, img.y,
                    self.screen_w as i32, self.screen_h as i32,
                    viewport, iw, ih, gs, display_scale(),
                );
                (ax as f32, ay as f32, 0.0, 0.0)
            };

            out.push(ImageLayout {
                image_id: img_id.clone(),
                x: ax, y: ay,
                width: w, height: h,
                opacity: img.opacity,
            });
        }

        out
    }

    fn build_text_overlays(
        &self,
        config: &Config,
        mode: &ModeConfig,
        viewport: &GameViewportGeometry,
    ) -> Vec<TextOverlayLayout> {
        let mut out = Vec::with_capacity(mode.text_overlay_ids.len());
        let gs = config.display.game_gui_scale;

        for tid in &mode.text_overlay_ids {
            let tcfg = match find_text_overlay(config, tid) {
                Some(t) => t,
                None => continue,
            };

            let anchor = parse_relative_to(&tcfg.relative_to);
            let (ax, ay) = resolve_relative_position(
                anchor, tcfg.x, tcfg.y,
                self.screen_w as i32, self.screen_h as i32,
                viewport, 0, 0, gs, display_scale(),
            );

            out.push(TextOverlayLayout {
                text_overlay_id: tid.clone(),
                x: ax as f32,
                y: ay as f32,
                opacity: tcfg.opacity,
            });
        }

        out
    }

    fn build_window_overlays(
        &self,
        config: &Config,
        mode: &ModeConfig,
        viewport: &GameViewportGeometry,
    ) -> Vec<WindowOverlayLayout> {
        let mut out = Vec::with_capacity(mode.window_overlay_ids.len());
        let gs = config.display.game_gui_scale;

        for woid in &mode.window_overlay_ids {
            let wcfg = match find_window_overlay(config, woid) {
                Some(w) => w,
                None => continue,
            };

            let anchor = parse_relative_to(&wcfg.relative_to);
            let (ax, ay) = resolve_relative_position(
                anchor, wcfg.x, wcfg.y,
                self.screen_w as i32, self.screen_h as i32,
                viewport, 0, 0, gs, display_scale(),
            );

            let border = if wcfg.border.enabled {
                Some(BorderLayout {
                    x: ax as f32,
                    y: ay as f32,
                    width: 0.0,  // filled at render time from capture dimensions
                    height: 0.0,
                    border_width: wcfg.border.width as f32,
                    radius: wcfg.border.radius as f32,
                    color: wcfg.border.color.to_array(),
                })
            } else {
                None
            };

            out.push(WindowOverlayLayout {
                wo_id: woid.clone(),
                x: ax as f32,
                y: ay as f32,
                scale: wcfg.scale,
                opacity: wcfg.opacity,
                crop_top: wcfg.crop_top,
                crop_bottom: wcfg.crop_bottom,
                crop_left: wcfg.crop_left,
                crop_right: wcfg.crop_right,
                pixelated_scaling: wcfg.pixelated_scaling,
                border,
            });
        }

        out
    }

    fn empty_layout(&self, vp_w: f32, vp_h: f32) -> FrameLayout {
        FrameLayout {
            viewport_x: 0.0,
            viewport_y: 0.0,
            viewport_width: vp_w,
            viewport_height: vp_h,
            background: BackgroundSpec::None,
            mirrors: Vec::new(),
            images: Vec::new(),
            text_overlays: Vec::new(),
            window_overlays: Vec::new(),
            border: None,
            eyezoom: None,
        }
    }
}

// --- Background ---

fn build_background(bg: &BackgroundConfig) -> BackgroundSpec {
    match bg.selected_mode.as_str() {
        "gradient" if !bg.gradient_stops.is_empty() => {
            let stops = bg.gradient_stops
                .iter()
                .map(|s| (s.color.to_array(), s.position))
                .collect();
            BackgroundSpec::Gradient {
                stops,
                angle: bg.gradient_angle,
                animation: bg.gradient_animation,
                speed: bg.gradient_animation_speed,
            }
        }
        "color" => BackgroundSpec::SolidColor(bg.color.to_array()),
        "image" if !bg.image.is_empty() => BackgroundSpec::Image { path: bg.image.clone() },
        _ => BackgroundSpec::None,
    }
}

// --- Border ---

fn build_border(b: &BorderConfig, vp_x: f32, vp_y: f32, vp_w: f32, vp_h: f32) -> BorderLayout {
    BorderLayout {
        x: vp_x, y: vp_y,
        width: vp_w, height: vp_h,
        border_width: b.width as f32,
        radius: b.radius as f32,
        color: b.color.to_array(),
    }
}

// --- EyeZoom ---

// Build the eyezoom layout in the margin between screen edge and viewport
fn build_eyezoom(
    ez: &EyeZoomConfig,
    vp_x: f32, _vp_w: f32,
    _screen_w: f32, screen_h: f32,
) -> Option<EyeZoomLayout> {
    let space = vp_x as i32;
    if space <= 0 {
        return None;
    }

    // proportional margins (1/36 and 1/8 reproduce the old 40px/180px at 1440p)
    let h_margin = screen_h as i32 / 36 + ez.horizontal_margin;
    let v_margin = screen_h as i32 / 8 + ez.vertical_margin;

    let zoom_w = space - 2 * h_margin;
    if zoom_w <= 20 {
        return None;
    }

    let zoom_h = (screen_h as i32 - 2 * v_margin).max((0.2 * screen_h as f64) as i32);

    let zoom_x = h_margin as f32;
    let zoom_y = v_margin as f32;

    let tex_w = ez.window_width;
    let tex_h = ez.window_height;
    let src_cx = tex_w / 2;
    let src_cy = tex_h / 2;
    let total_clone = ez.clone_width * 2;
    let src_x = src_cx - ez.clone_width;
    let src_y = src_cy - ez.clone_height / 2;

    let labels_per_side = ez.clone_width;
    let ov_width = if ez.overlay_width < 0 {
        labels_per_side
    } else {
        ez.overlay_width.min(labels_per_side)
    };

    let box_h = if ez.link_rect_to_font {
        ez.text_font_size as f32 * 1.2
    } else {
        ez.rect_height as f32
    };

    // mirror rect - ratios reproduce the old 1440p layout (90, 480, 900, 500)
    let sh = screen_h;
    let mirror_x = sh / 16.0;
    let mirror_y = sh / 3.0;
    let mirror_w = sh * 5.0 / 8.0;
    let mirror_h = sh * 25.0 / 72.0;

    Some(EyeZoomLayout {
        x: zoom_x,
        y: zoom_y,
        width: zoom_w as f32,
        height: zoom_h as f32,
        mirror_x, mirror_y, mirror_w, mirror_h,
        src_x,
        src_y,
        src_w: total_clone,
        src_h: ez.clone_height,
        clone_width: total_clone,
        overlay_width: ov_width,
        box_height: box_h,
        grid_color1: [ez.grid_color1.r, ez.grid_color1.g, ez.grid_color1.b, ez.grid_color1_opacity],
        grid_color2: [ez.grid_color2.r, ez.grid_color2.g, ez.grid_color2.b, ez.grid_color2_opacity],
        center_line_color: [ez.center_line_color.r, ez.center_line_color.g, ez.center_line_color.b, ez.center_line_color_opacity],
    })
}

// --- Expression-based stretch ---

fn resolve_stretch_dim(expr: &str, fallback: i32, sw: i32, sh: i32) -> i32 {
    if !expr.is_empty() && is_expression(expr) {
        evaluate_expression(expr, sw, sh).unwrap_or(fallback)
    } else {
        fallback
    }
}

// --- Config lookups ---

fn find_mode<'a>(config: &'a Config, id: &str) -> Option<&'a ModeConfig> {
    config.modes.iter().find(|m| m.id == id)
}

fn find_mirror<'a>(config: &'a Config, name: &str) -> Option<&'a MirrorConfig> {
    config.overlays.mirrors.iter().find(|m| m.name == name)
}

fn find_mirror_group<'a>(
    config: &'a Config, name: &str,
) -> Option<&'a tuxinjector_config::types::MirrorGroupConfig> {
    config.overlays.mirror_groups.iter().find(|g| g.name == name)
}

fn find_image<'a>(config: &'a Config, name: &str) -> Option<&'a ImageConfig> {
    config.overlays.images.iter().find(|i| i.name == name)
}

fn find_text_overlay<'a>(config: &'a Config, name: &str) -> Option<&'a TextOverlayConfig> {
    config.overlays.text_overlays.iter().find(|t| t.name == name)
}

fn find_window_overlay<'a>(config: &'a Config, name: &str) -> Option<&'a WindowOverlayConfig> {
    config.overlays.window_overlays.iter().find(|w| w.name == name)
}

// macOS Retina: pie chart offsets must be scaled by content_scale
pub fn display_scale() -> f32 {
    #[cfg(target_os = "macos")]
    {
        let (sx, _) = unsafe { crate::viewport_hook::content_scale() };
        sx
    }
    #[cfg(target_os = "linux")]
    { 1.0 }
}

// --- RelativeTo parsing ---

// NOTE: falls back to TopLeftScreen for unknown values
pub fn parse_relative_to(s: &str) -> RelativeTo {
    match s {
        "topLeftScreen" => RelativeTo::TopLeftScreen,
        "topCenterScreen" => RelativeTo::TopCenterScreen,
        "topRightScreen" => RelativeTo::TopRightScreen,
        "centerScreen" => RelativeTo::CenterScreen,
        "bottomLeftScreen" => RelativeTo::BottomLeftScreen,
        "bottomCenterScreen" => RelativeTo::BottomCenterScreen,
        "bottomRightScreen" => RelativeTo::BottomRightScreen,
        "topLeftViewport" => RelativeTo::TopLeftViewport,
        "topCenterViewport" => RelativeTo::TopCenterViewport,
        "topRightViewport" => RelativeTo::TopRightViewport,
        "centerViewport" => RelativeTo::CenterViewport,
        "bottomLeftViewport" => RelativeTo::BottomLeftViewport,
        "bottomCenterViewport" => RelativeTo::BottomCenterViewport,
        "bottomRightViewport" => RelativeTo::BottomRightViewport,
        "pieLeft" => RelativeTo::PieLeft,
        "pieRight" => RelativeTo::PieRight,
        other => {
            tracing::warn!(anchor = other, "unknown relative_to, defaulting to topLeftScreen");
            RelativeTo::TopLeftScreen
        }
    }
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use tuxinjector_config::types::*;

    fn test_config(modes: Vec<ModeConfig>) -> Config {
        Config { modes, ..Config::default() }
    }

    fn fullscreen_mode() -> ModeConfig {
        ModeConfig {
            id: "Fullscreen".into(),
            width: 1920,
            height: 1080,
            ..ModeConfig::default()
        }
    }

    fn half_mode() -> ModeConfig {
        ModeConfig {
            id: "Half".into(),
            use_relative_size: true,
            relative_width: 0.5,
            relative_height: 0.5,
            game_transition: GameTransitionType::Bounce,
            transition_duration_ms: 300,
            ..ModeConfig::default()
        }
    }

    #[test]
    fn new_mode_system_defaults() {
        let ms = ModeSystem::new("Fullscreen");
        assert_eq!(ms.current_mode_id(), "Fullscreen");
        assert!(!ms.is_transitioning());
    }

    #[test]
    fn resolve_absolute_dimensions() {
        let ms = ModeSystem::new("Fullscreen");
        let mode = fullscreen_mode();
        let (w, h) = ms.resolve_dimensions(&mode);
        assert_eq!(w, 1920.0);
        assert_eq!(h, 1080.0);
    }

    #[test]
    fn resolve_relative_dimensions() {
        let mut ms = ModeSystem::new("Half");
        ms.update_screen_size(1920, 1080);
        let mode = half_mode();
        let (w, h) = ms.resolve_dimensions(&mode);
        assert_eq!(w, 960.0);
        assert_eq!(h, 540.0);
    }

    #[test]
    fn resolve_expression_dimensions() {
        let mut ms = ModeSystem::new("Expr");
        ms.update_screen_size(1920, 1080);

        let mode = ModeConfig {
            id: "Expr".into(),
            width_expr: "screenWidth - 200".into(),
            height_expr: "screenHeight / 2".into(),
            ..ModeConfig::default()
        };

        let (w, h) = ms.resolve_dimensions(&mode);
        assert_eq!(w, 1720.0);
        assert_eq!(h, 540.0);
    }

    #[test]
    fn switch_mode_cut() {
        let config = test_config(vec![
            fullscreen_mode(),
            ModeConfig {
                id: "Small".into(),
                width: 800,
                height: 600,
                game_transition: GameTransitionType::Cut,
                ..ModeConfig::default()
            },
        ]);

        let mut ms = ModeSystem::new("Fullscreen");
        ms.update_screen_size(1920, 1080);
        ms.tick(&config);

        ms.switch_mode("Small", &config);
        assert!(!ms.is_transitioning());
        assert_eq!(ms.current_mode_id(), "Small");
    }

    #[test]
    fn switch_mode_bounce_starts_transition() {
        let config = test_config(vec![fullscreen_mode(), half_mode()]);

        let mut ms = ModeSystem::new("Fullscreen");
        ms.update_screen_size(1920, 1080);
        ms.tick(&config);

        ms.switch_mode("Half", &config);
        assert!(ms.is_transitioning());
    }

    #[test]
    fn tick_no_transition_returns_settled() {
        let config = test_config(vec![fullscreen_mode()]);
        let mut ms = ModeSystem::new("Fullscreen");
        ms.update_screen_size(1920, 1080);

        let layout = ms.tick(&config);
        assert_eq!(layout.viewport_width, 1920.0);
        assert_eq!(layout.viewport_height, 1080.0);
        assert_eq!(layout.viewport_x, 0.0);
        assert_eq!(layout.viewport_y, 0.0);
    }

    #[test]
    fn background_solid_color() {
        let config = test_config(vec![ModeConfig {
            id: "BG".into(),
            width: 1920,
            height: 1080,
            background: BackgroundConfig {
                selected_mode: "color".into(),
                color: tuxinjector_core::Color::WHITE,
                ..BackgroundConfig::default()
            },
            ..ModeConfig::default()
        }]);

        let mut ms = ModeSystem::new("BG");
        ms.update_screen_size(1920, 1080);
        let layout = ms.tick(&config);

        match layout.background {
            BackgroundSpec::SolidColor(c) => assert_eq!(c, [1.0, 1.0, 1.0, 1.0]),
            _ => panic!("expected SolidColor"),
        }
    }

    #[test]
    fn background_gradient() {
        let config = test_config(vec![ModeConfig {
            id: "Grad".into(),
            width: 1920,
            height: 1080,
            background: BackgroundConfig {
                selected_mode: "gradient".into(),
                gradient_stops: vec![
                    GradientColorStop {
                        color: tuxinjector_core::Color::BLACK,
                        position: 0.0,
                    },
                    GradientColorStop {
                        color: tuxinjector_core::Color::WHITE,
                        position: 1.0,
                    },
                ],
                gradient_angle: 45.0,
                gradient_animation: GradientAnimationType::Rotate,
                gradient_animation_speed: 2.0,
                ..BackgroundConfig::default()
            },
            ..ModeConfig::default()
        }]);

        let mut ms = ModeSystem::new("Grad");
        ms.update_screen_size(1920, 1080);
        let layout = ms.tick(&config);

        match layout.background {
            BackgroundSpec::Gradient { stops, angle, animation, speed } => {
                assert_eq!(stops.len(), 2);
                assert_eq!(angle, 45.0);
                assert_eq!(animation, GradientAnimationType::Rotate);
                assert_eq!(speed, 2.0);
            }
            _ => panic!("expected Gradient"),
        }
    }

    #[test]
    fn border_when_enabled() {
        let config = test_config(vec![ModeConfig {
            id: "Bordered".into(),
            width: 800,
            height: 600,
            border: BorderConfig {
                enabled: true,
                color: tuxinjector_core::Color::WHITE,
                width: 4,
                radius: 8,
            },
            ..ModeConfig::default()
        }]);

        let mut ms = ModeSystem::new("Bordered");
        ms.update_screen_size(1920, 1080);
        let layout = ms.tick(&config);

        let border = layout.border.expect("border should be present");
        assert_eq!(border.border_width, 4.0);
        assert_eq!(border.radius, 8.0);
    }

    #[test]
    fn no_border_when_disabled() {
        let config = test_config(vec![fullscreen_mode()]);
        let mut ms = ModeSystem::new("Fullscreen");
        ms.update_screen_size(1920, 1080);
        let layout = ms.tick(&config);
        assert!(layout.border.is_none());
    }

    #[test]
    fn parse_all_relative_to_variants() {
        assert_eq!(parse_relative_to("topLeftScreen"), RelativeTo::TopLeftScreen);
        assert_eq!(parse_relative_to("topCenterScreen"), RelativeTo::TopCenterScreen);
        assert_eq!(parse_relative_to("topRightScreen"), RelativeTo::TopRightScreen);
        assert_eq!(parse_relative_to("centerScreen"), RelativeTo::CenterScreen);
        assert_eq!(parse_relative_to("bottomLeftScreen"), RelativeTo::BottomLeftScreen);
        assert_eq!(parse_relative_to("bottomCenterScreen"), RelativeTo::BottomCenterScreen);
        assert_eq!(parse_relative_to("bottomRightScreen"), RelativeTo::BottomRightScreen);
        assert_eq!(parse_relative_to("topLeftViewport"), RelativeTo::TopLeftViewport);
        assert_eq!(parse_relative_to("topCenterViewport"), RelativeTo::TopCenterViewport);
        assert_eq!(parse_relative_to("topRightViewport"), RelativeTo::TopRightViewport);
        assert_eq!(parse_relative_to("centerViewport"), RelativeTo::CenterViewport);
        assert_eq!(parse_relative_to("bottomLeftViewport"), RelativeTo::BottomLeftViewport);
        assert_eq!(parse_relative_to("bottomCenterViewport"), RelativeTo::BottomCenterViewport);
        assert_eq!(parse_relative_to("bottomRightViewport"), RelativeTo::BottomRightViewport);
        assert_eq!(parse_relative_to("pieLeft"), RelativeTo::PieLeft);
        assert_eq!(parse_relative_to("pieRight"), RelativeTo::PieRight);
        assert_eq!(parse_relative_to("garbage"), RelativeTo::TopLeftScreen);
    }

    #[test]
    fn switch_to_same_mode_is_noop() {
        let config = test_config(vec![fullscreen_mode()]);
        let mut ms = ModeSystem::new("Fullscreen");
        ms.update_screen_size(1920, 1080);
        ms.tick(&config);

        ms.switch_mode("Fullscreen", &config);
        assert!(!ms.is_transitioning());
        assert_eq!(ms.current_mode_id(), "Fullscreen");
    }

    #[test]
    fn switch_to_unknown_mode_is_noop() {
        let config = test_config(vec![fullscreen_mode()]);
        let mut ms = ModeSystem::new("Fullscreen");
        ms.update_screen_size(1920, 1080);

        ms.switch_mode("NonExistent", &config);
        assert!(!ms.is_transitioning());
        assert_eq!(ms.current_mode_id(), "Fullscreen");
    }

    #[test]
    fn mirror_layout_absolute_position() {
        let mirror = MirrorConfig {
            name: "eye".into(),
            capture_width: 50,
            capture_height: 50,
            output: MirrorRenderConfig {
                x: 100,
                y: 200,
                scale: 2.0,
                relative_to: "topLeftScreen".into(),
                ..MirrorRenderConfig::default()
            },
            ..MirrorConfig::default()
        };

        let config = Config {
            overlays: tuxinjector_config::OverlaysConfig {
                mirrors: vec![mirror],
                ..Default::default()
            },
            modes: vec![ModeConfig {
                id: "Test".into(),
                width: 1920,
                height: 1080,
                mirror_ids: vec!["eye".into()],
                ..ModeConfig::default()
            }],
            ..Config::default()
        };

        let mut ms = ModeSystem::new("Test");
        ms.update_screen_size(1920, 1080);
        let layout = ms.tick(&config);

        assert_eq!(layout.mirrors.len(), 1);
        let m = &layout.mirrors[0];
        assert_eq!(m.mirror_id, "eye");
        assert_eq!(m.render_x, 100.0);
        assert_eq!(m.render_y, 200.0);
        assert_eq!(m.render_width, 100.0);
        assert_eq!(m.render_height, 100.0);
    }

    #[test]
    fn stretch_adjusts_viewport() {
        let config = test_config(vec![ModeConfig {
            id: "Stretched".into(),
            width: 1920,
            height: 1080,
            stretch: StretchConfig {
                enabled: true,
                x: 10,
                y: 20,
                width: 1600,
                height: 900,
                ..StretchConfig::default()
            },
            ..ModeConfig::default()
        }]);

        let mut ms = ModeSystem::new("Stretched");
        ms.update_screen_size(1920, 1080);
        let layout = ms.tick(&config);

        assert_eq!(layout.viewport_width, 1600.0);
        assert_eq!(layout.viewport_height, 900.0);
        assert_eq!(layout.viewport_x, 10.0);
        assert_eq!(layout.viewport_y, 20.0);
    }
}
