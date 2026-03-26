// Overlay rendering integration -- bridges config, mode system, mirror capture,
// and the GL renderer. Called each frame from the swap hooks to build a scene
// and draw it into the game's backbuffer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tuxinjector_config::ConfigSnapshot;
use tuxinjector_gl_interop::{GlOverlayRenderer, GlFns as InteropGlFns};
use tuxinjector_render::image_loader::{self, AnimationPlayer, ImageData};

use tuxinjector_core::geometry::{GameViewportGeometry, resolve_relative_position};

use crate::app_capture::AppCaptureManager;
use crate::gl_resolve::{EglGetProcAddressFn, GlFunctions};
use crate::gui_renderer::{GuiInput, GuiRenderer};
use crate::text_rasterizer::TextOverlayCache;

// CPU-side color match check. Samples every `stride`-th pixel for speed.
// Returns true if any pixel is within `sensitivity` euclidean distance
// of any target color in RGB space.
fn has_matching_pixels(pixels: &[u8], targets: &[[f32; 4]], sensitivity: f32, stride: usize) -> bool {
    if targets.is_empty() { return false; }
    let stride = stride.max(1);
    let npx = pixels.len() / 4;
    let mut i = 0;
    while i < npx {
        let off = i * 4;
        let r = pixels[off] as f32 / 255.0;
        let g = pixels[off + 1] as f32 / 255.0;
        let b = pixels[off + 2] as f32 / 255.0;
        for t in targets {
            let dr = r - t[0];
            let dg = g - t[1];
            let db = b - t[2];
            if (dr * dr + dg * dg + db * db).sqrt() < sensitivity {
                return true;
            }
        }
        i += stride;
    }
    false
}

// pad RGBA with transparent border on all sides (for the border shader)
fn pad_pixels(w: u32, h: u32, pixels: &[u8], pad: u32) -> (u32, u32, Vec<u8>) {
    let nw = w + 2 * pad;
    let nh = h + 2 * pad;
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for row in 0..h {
        let src = (row * w * 4) as usize;
        let dst = ((row + pad) * nw + pad) as usize * 4;
        let len = (w * 4) as usize;
        out[dst..dst + len].copy_from_slice(&pixels[src..src + len]);
    }
    (nw, nh, out)
}

use crate::mirror_capture::MirrorCaptureManager;
use crate::mode_system::{BackgroundSpec, FrameLayout, ModeSystem, parse_relative_to};
use crate::state;
use crate::render_thread::{SceneDescription, SceneElement};

const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;

// --- window overlay capture ---

// Wraps MacCapture (macOS) or PipeWire (Linux) behind a common interface
struct WindowCaptureManager {
    #[cfg(target_os = "macos")]
    inner: tuxinjector_capture::mac_capture::MacCapture,
    // TODO: wire up PipeWire capture on Linux (needs portal picker UI)
    #[cfg(target_os = "linux")]
    _placeholder: (),
}

impl WindowCaptureManager {
    fn new() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            inner: tuxinjector_capture::mac_capture::MacCapture::new(),
            #[cfg(target_os = "linux")]
            _placeholder: (),
        }
    }

    // Start/stop capture sessions to match current config
    fn sync_sessions(&mut self, configs: &[tuxinjector_config::types::WindowOverlayConfig]) {
        let wanted: std::collections::HashSet<&str> = configs.iter()
            .filter(|c| !c.name.is_empty())
            .map(|c| c.name.as_str())
            .collect();

        // stop sessions that are no longer wanted
        let active = self.active_sessions();
        for id in &active {
            if !wanted.contains(id.as_str()) {
                self.stop_capture(id);
            }
        }

        // start sessions for new overlays
        for cfg in configs {
            if cfg.name.is_empty() { continue; }
            if active.iter().any(|a| a == &cfg.name) { continue; }

            let title = if cfg.window_title.is_empty() { None } else { Some(cfg.window_title.as_str()) };
            let owner = if cfg.executable_name.is_empty() {
                if cfg.window_class.is_empty() { None } else { Some(cfg.window_class.as_str()) }
            } else {
                Some(cfg.executable_name.as_str())
            };

            if title.is_none() && owner.is_none() { continue; }

            let fps = cfg.fps.max(1) as u32;
            if let Err(e) = self.start_capture(&cfg.name, title, owner, fps) {
                tracing::warn!(id = cfg.name, error = %e, "window overlay: failed to start capture");
            }
        }
    }

    fn start_capture(&mut self, id: &str, title: Option<&str>, owner: Option<&str>, fps: u32)
        -> Result<(), Box<dyn std::error::Error>>
    {
        #[cfg(target_os = "macos")]
        { self.inner.start_capture(id, title, owner, fps) }
        #[cfg(target_os = "linux")]
        { let _ = (id, title, owner, fps); Ok(()) }
    }

    fn stop_capture(&mut self, id: &str) {
        #[cfg(target_os = "macos")]
        { self.inner.stop_capture(id); }
        #[cfg(target_os = "linux")]
        { let _ = id; }
    }

    fn latest_frame(&self, id: &str) -> Option<tuxinjector_capture::CapturedFrame> {
        #[cfg(target_os = "macos")]
        { self.inner.latest_frame(id) }
        #[cfg(target_os = "linux")]
        { let _ = id; None }
    }

    fn active_sessions(&self) -> Vec<String> {
        #[cfg(target_os = "macos")]
        { self.inner.active_sessions() }
        #[cfg(target_os = "linux")]
        { Vec::new() }
    }
}

// --- OverlayState ---

// Owns the GL compositor, mode system, mirror capture, and GUI.
pub struct OverlayState {
    gl_renderer: Option<GlOverlayRenderer>,
    interop_gl: InteropGlFns,
    local_gl: GlFunctions,
    config: Arc<ConfigSnapshot>,
    mode_system: ModeSystem,
    mirrors: MirrorCaptureManager,
    img_cache: ImageCache,
    bg_cache: BgImageCache,
    cursor_cache: CursorCache,
    text_cache: TextOverlayCache,
    gui: GuiRenderer,
    app_capture: AppCaptureManager,
    win_capture: WindowCaptureManager,
    browser_capture: crate::browser_capture::BrowserCaptureManager,
    w: u32,
    h: u32,
    frame: u64,
    t_start: Instant, // for shader uTime
    last_frame: Instant,
    // (texture_id, game_w, game_h) -- set when Sodium's FBO is found
    game_tex: Option<(u32, u32, u32)>,
    images_visible: bool,
    windows_visible: bool,
    // per-mirror UV rects in game texture coords
    game_uv_rects: HashMap<String, [f32; 4]>,
    // cached FBO probe: (fbo, tex, search_w, search_h, frame)
    cached_probe: Option<(u32, u32, u32, u32, u64)>,
    shader_version: u64,
}

// --- image cache ---

struct CachedImage {
    data: ImageData,
    pixels: Vec<u8>, // RGBA, color keys applied
    w: u32,
    h: u32,
    anim: Option<AnimationPlayer>,
    last_anim_time: Instant,
    source: String,
    mtime: Option<std::time::SystemTime>,
    last_check: Instant, // throttled to ~1s
}

// lazy loader with mtime-based hot-reload
struct ImageCache {
    images: HashMap<String, CachedImage>,
}

impl ImageCache {
    fn new() -> Self { Self { images: HashMap::new() } }

    // get or load an image, advance animation, apply color keys
    fn get(
        &mut self,
        id: &str,
        cfg: &tuxinjector_config::Config,
    ) -> Option<(&[u8], u32, u32)> {
        let img_cfg = cfg.overlays.images.iter().find(|i| i.name == id)?;

        let now = Instant::now();
        let needs_load = match self.images.get_mut(id) {
            None => true,
            Some(cached) => {
                if cached.source != img_cfg.path {
                    true
                } else if now.duration_since(cached.last_check) > Duration::from_secs(1) {
                    cached.last_check = now;
                    let expanded = expand_tilde(&img_cfg.path);
                    let cur_mtime = std::fs::metadata(&*expanded)
                        .and_then(|m| m.modified()).ok();
                    cur_mtime != cached.mtime
                } else {
                    false
                }
            }
        };

        if needs_load {
            if img_cfg.path.is_empty() { return None; }

            let expanded = expand_tilde(&img_cfg.path);
            let path = Path::new(expanded.as_ref());
            match image_loader::load_image(path) {
                Ok(data) => {
                    let (w, h) = data.dimensions();
                    if w == 0 || h == 0 {
                        tracing::warn!(id, "image has zero dimensions");
                        return None;
                    }

                    let (pixels, anim) = match &data {
                        ImageData::Static(img) => {
                            let mut px = img.pixels.clone();
                            apply_color_keys(&mut px, img_cfg);
                            (px, None)
                        }
                        ImageData::Animated { frames, loop_count } => {
                            let player = AnimationPlayer::new(frames, *loop_count);
                            let mut px = frames[0].pixels.clone();
                            apply_color_keys(&mut px, img_cfg);
                            (px, Some(player))
                        }
                    };

                    tracing::info!(id, w, h, animated = anim.is_some(), "loaded image overlay");

                    let mtime = std::fs::metadata(path)
                        .and_then(|m| m.modified()).ok();

                    self.images.insert(id.to_string(), CachedImage {
                        data, pixels, w, h, anim,
                        last_anim_time: Instant::now(),
                        source: img_cfg.path.clone(),
                        mtime,
                        last_check: Instant::now(),
                    });
                }
                Err(e) => {
                    tracing::warn!(id, path = %img_cfg.path, %e, "failed to load image");
                    return None;
                }
            }
        }

        let cached = self.images.get_mut(id)?;

        // advance animation if present
        if let Some(ref mut player) = cached.anim {
            let now = Instant::now();
            let dt = now.duration_since(cached.last_anim_time);
            cached.last_anim_time = now;

            if let Some(idx) = player.advance(dt) {
                if let ImageData::Animated { frames, .. } = &cached.data {
                    if idx < frames.len() {
                        cached.pixels = frames[idx].pixels.clone();
                        cached.w = frames[idx].width;
                        cached.h = frames[idx].height;
                        apply_color_keys(&mut cached.pixels, img_cfg);
                    }
                }
            }
        }

        Some((&cached.pixels, cached.w, cached.h))
    }
}

// --- background image cache ---

struct BgImageCache {
    pixels: Option<Vec<u8>>,
    w: u32,
    h: u32,
    loaded_path: String,
    mtime: Option<std::time::SystemTime>,
    last_check: Instant,
}

impl BgImageCache {
    fn new() -> Self {
        Self {
            pixels: None, w: 0, h: 0,
            loaded_path: String::new(),
            mtime: None,
            last_check: Instant::now(),
        }
    }

    fn get(&mut self, path: &str) -> Option<(&[u8], u32, u32)> {
        if path.is_empty() { return None; }

        let now = Instant::now();
        let expanded = expand_tilde(path);
        let needs_load = if self.loaded_path != path || self.pixels.is_none() {
            true
        } else if now.duration_since(self.last_check) > Duration::from_secs(1) {
            self.last_check = now;
            let cur = std::fs::metadata(&*expanded).and_then(|m| m.modified()).ok();
            cur != self.mtime
        } else {
            false
        };

        if needs_load {
            let p = Path::new(expanded.as_ref());
            match image_loader::load_image(p) {
                Ok(data) => {
                    let (w, h) = data.dimensions();
                    if w == 0 || h == 0 {
                        tracing::warn!(path, "bg image has zero dimensions");
                        self.pixels = None;
                        return None;
                    }
                    let px = match &data {
                        ImageData::Static(img) => img.pixels.clone(),
                        ImageData::Animated { frames, .. } => {
                            frames.first().map(|f| f.pixels.clone()).unwrap_or_default()
                        }
                    };
                    self.w = w;
                    self.h = h;
                    self.loaded_path = path.to_string();
                    self.mtime = std::fs::metadata(&*expanded).and_then(|m| m.modified()).ok();
                    self.last_check = now;
                    self.pixels = Some(px);
                }
                Err(e) => {
                    tracing::warn!(path, err = %e, "failed to load bg image");
                    self.pixels = None;
                    self.loaded_path.clear();
                    return None;
                }
            }
        }

        self.pixels.as_deref().map(|px| (px, self.w, self.h))
    }
}

// --- cursor cache ---

fn dirs_cursors_path() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".config/tuxinjector/cursors"))
        .unwrap_or_default()
}

struct CursorCache {
    pixels: Option<Vec<u8>>,
    w: u32,
    h: u32,
    hotspot_x: f32,
    hotspot_y: f32,
    loaded_name: String,
    loaded_size: i32,
}

impl CursorCache {
    fn new() -> Self {
        Self {
            pixels: None, w: 0, h: 0,
            hotspot_x: 0.0, hotspot_y: 0.0,
            loaded_name: String::new(), loaded_size: 0,
        }
    }

    fn get_cursor(&mut self, name: &str, size: i32) -> Option<(&[u8], u32, u32, f32, f32)> {
        if name.is_empty() { return None; }

        if self.loaded_name != name || self.loaded_size != size {
            self.load(name, size);
        }

        self.pixels.as_ref().map(|px| {
            (px.as_slice(), self.w, self.h, self.hotspot_x, self.hotspot_y)
        })
    }

    fn load(&mut self, name: &str, size: i32) {
        self.loaded_name = name.to_string();
        self.loaded_size = size;
        let sz = size.max(8) as u32;

        // search: direct path first, then cursors folder
        let candidates = vec![
            PathBuf::from(name),
            dirs_cursors_path().join(name),
        ];

        for path in &candidates {
            if !path.exists() { continue; }
            match image_loader::load_image(path) {
                Ok(ImageData::Static(img)) => {
                    tracing::info!(?path, w = img.width, h = img.height, "loaded cursor");
                    self.w = img.width;
                    self.h = img.height;
                    self.hotspot_x = img.width as f32 / 2.0;
                    self.hotspot_y = img.height as f32 / 2.0;
                    self.pixels = Some(img.pixels);
                    return;
                }
                Ok(ImageData::Animated { frames, .. }) => {
                    if let Some(frame) = frames.into_iter().next() {
                        self.w = frame.width;
                        self.h = frame.height;
                        self.hotspot_x = frame.width as f32 / 2.0;
                        self.hotspot_y = frame.height as f32 / 2.0;
                        self.pixels = Some(frame.pixels);
                        return;
                    }
                }
                Err(e) => {
                    tracing::warn!(?path, %e, "cursor load failed");
                }
            }
        }

        // procedural crosshair fallback
        tracing::info!(name, sz, "generating procedural crosshair");
        let px = gen_crosshair(sz);
        self.hotspot_x = sz as f32 / 2.0;
        self.hotspot_y = sz as f32 / 2.0;
        self.w = sz;
        self.h = sz;
        self.pixels = Some(px);
    }
}

fn gen_crosshair(size: u32) -> Vec<u8> {
    let mut px = vec![0u8; (size * size * 4) as usize];
    let center = size / 2;
    let thick = 1u32.max(size / 16);
    let gap = size / 6;
    let outline = 1u32;

    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let dx = (x as i32 - center as i32).unsigned_abs();
            let dy = (y as i32 - center as i32).unsigned_abs();

            let on_h = dy < thick && dx > gap && dx < center;
            let on_v = dx < thick && dy > gap && dy < center;
            let on_dot = dx < thick && dy < thick;

            if on_h || on_v || on_dot {
                px[idx] = 255; px[idx + 1] = 255;
                px[idx + 2] = 255; px[idx + 3] = 255;
            } else {
                // outline pixels
                let near_h = dy <= thick + outline && dx > gap.saturating_sub(outline) && dx < center + outline;
                let near_v = dx <= thick + outline && dy > gap.saturating_sub(outline) && dy < center + outline;
                let near_dot = dx <= thick + outline && dy <= thick + outline;

                if (near_h || near_v || near_dot) && !(on_h || on_v || on_dot) {
                    px[idx + 3] = 200; // semi-transparent black outline
                }
            }
        }
    }
    px
}

fn expand_tilde(path: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}").into();
        }
    }
    path.into()
}

fn apply_color_keys(pixels: &mut [u8], cfg: &tuxinjector_config::types::ImageConfig) {
    if !cfg.enable_color_key { return; }

    if !cfg.color_keys.is_empty() {
        let keys: Vec<([f32; 3], f32)> = cfg.color_keys.iter()
            .map(|ck| ([ck.color.r, ck.color.g, ck.color.b], ck.sensitivity))
            .collect();
        image_loader::apply_color_keys(pixels, &keys);
    } else {
        // legacy single color key
        image_loader::apply_color_key(
            pixels,
            [cfg.color_key.r, cfg.color_key.g, cfg.color_key.b],
            cfg.color_key_sensitivity,
        );
    }
}

impl OverlayState {
    pub unsafe fn new(
        gpa: EglGetProcAddressFn,
        config: Arc<ConfigSnapshot>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let interop_gpa: tuxinjector_gl_interop::gl_bindings::GetProcAddrFn =
            std::mem::transmute(gpa);
        let interop_gl = InteropGlFns::resolve(interop_gpa);
        let local_gl = GlFunctions::resolve(gpa);

        let gl_renderer = GlOverlayRenderer::new(&interop_gl)
            .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
        tracing::info!("overlay: GL renderer initialised (direct rendering)");

        let cfg = config.load();
        let default_mode = cfg.modes.first()
            .map(|m| m.id.as_str())
            .unwrap_or("Fullscreen");

        let mode_system = ModeSystem::new(default_mode);
        let mirrors = MirrorCaptureManager::new();
        let gui = GuiRenderer::new(Arc::clone(&config), gpa);

        Ok(Self {
            gl_renderer: Some(gl_renderer),
            interop_gl,
            local_gl,
            config,
            mode_system,
            mirrors,
            img_cache: ImageCache::new(),
            bg_cache: BgImageCache::new(),
            cursor_cache: CursorCache::new(),
            text_cache: TextOverlayCache::new(),
            gui,
            app_capture: AppCaptureManager::new(),
            win_capture: WindowCaptureManager::new(),
            browser_capture: crate::browser_capture::BrowserCaptureManager::new(),
            w: 0, h: 0,
            frame: 0,
            t_start: Instant::now(),
            last_frame: Instant::now(),
            images_visible: true,
            windows_visible: true,
            game_tex: None,
            game_uv_rects: HashMap::new(),
            cached_probe: None,
            shader_version: 0,
        })
    }

    // Build and draw the overlay scene. Called once per frame from the swap hook.
    pub unsafe fn render_and_composite(
        &mut self,
        vp_w: u32,
        vp_h: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if vp_w == 0 || vp_h == 0 { return Ok(()); }

        let t0 = Instant::now();

        if self.w != vp_w || self.h != vp_h {
            self.w = vp_w;
            self.h = vp_h;
            self.mode_system.update_screen_size(vp_w, vp_h);
        }

        let cfg = self.config.load();

        // reload custom shaders on config change
        let cv = self.config.version();
        if cv != self.shader_version {
            self.shader_version = cv;
            self.load_custom_shaders(&cfg);
        }

        let layout = self.mode_system.tick(&cfg);

        // periodic diagnostic log
        static DIAG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let diag_ctr = DIAG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if diag_ctr % 120 == 0 {
            let ids: Vec<&str> = layout.mirrors.iter().map(|m| m.mirror_id.as_str()).collect();
            tracing::debug!(
                frame = diag_ctr,
                has_eyezoom = layout.eyezoom.is_some(),
                mirrors = layout.mirrors.len(),
                ?ids,
                vp_w = layout.viewport_width, vp_h = layout.viewport_height,
                vp_x = layout.viewport_x, vp_y = layout.viewport_y,
                vfb_active = crate::virtual_fb::is_active(),
                vfb_fbo = crate::virtual_fb::virtual_fbo(),
                "overlay: layout"
            );
            if let Some(ref ez) = layout.eyezoom {
                tracing::debug!(
                    ez_x = ez.x, ez_y = ez.y, ez_w = ez.width, ez_h = ez.height,
                    src_x = ez.src_x, src_y = ez.src_y, src_w = ez.src_w, src_h = ez.src_h,
                    "overlay: eyezoom"
                );
            }
        }

        let t_tick = Instant::now();

        // mirrors must run every frame (PBO double-buffer pipeline)
        self.capture_mirrors(&cfg, &layout);

        let t_mirrors = Instant::now();

        // sync window + browser overlay capture sessions with config
        self.win_capture.sync_sessions(&cfg.overlays.window_overlays);
        self.browser_capture.sync_sessions(&cfg.overlays.browser_overlays);

        let mut scene = build_scene(
            &layout, vp_w, vp_h,
            &mut self.mirrors,
            &mut self.img_cache,
            &mut self.bg_cache,
            &mut self.cursor_cache,
            &mut self.text_cache,
            &cfg,
            self.game_tex,
            &self.game_uv_rects,
            self.images_visible,
            self.windows_visible,
            &self.win_capture,
            &self.browser_capture,
            self.t_start.elapsed().as_secs_f32(),
        );

        let t_scene = Instant::now();

        // embed anchored companion app windows
        {
            use tuxinjector_gui::running_apps::LaunchMode;

            let running = tuxinjector_gui::running_apps::list();

            let pids: std::collections::HashSet<u32> = running.iter().map(|a| a.pid).collect();
            let stale: Vec<u32> = self.app_capture.known_pids().into_iter()
                .filter(|p| !pids.contains(p)).collect();
            for pid in stale { self.app_capture.drop_window(pid); }

            for app in &running {
                match app.mode {
                    LaunchMode::Anchored(anchor) => {
                        if !self.windows_visible { continue; }
                        if let Some(cap) = self.app_capture.embed(app.pid, vp_w, vp_h, anchor) {
                            scene.elements.push(SceneElement::Textured {
                                x: cap.anchor_x, y: cap.anchor_y,
                                w: cap.width as f32, h: cap.height as f32,
                                tex_width: cap.width, tex_height: cap.height,
                                pixels: cap.pixels,
                                circle_clip: false, nearest_filter: true,
                                filter_target_colors: Vec::new(),
                                filter_output_color: [0.0; 4],
                                filter_sensitivity: 0.0,
                                filter_color_passthrough: false,
                                filter_border_color: [0.0; 4],
                                filter_border_width: 0,
                                filter_gamma_mode: 0,
                                custom_shader: None,
                            });
                        }
                    }
                    LaunchMode::Detached => {
                        // set _NET_WM_WINDOW_TYPE_UTILITY so tiling WMs float the window
                        self.app_capture.set_float_hint(app.pid);
                    }
                }
            }

            // forward queued keyboard events to embedded windows
            self.app_capture.forward_pending_keys();
        }

        // plugin overlay submissions
        let now = Instant::now();
        let dt_ms = now.duration_since(self.last_frame).as_secs_f32() * 1000.0;
        self.last_frame = now;
        self.frame += 1;

        if let Some(reg_lock) = state::get().plugins.get() {
            if let Ok(mut reg) = reg_lock.lock() {
                let gs = state::get().game_state.lock()
                    .map(|g| g.clone()).unwrap_or_default();
                let cur_mode = self.mode_system.current_mode_id().to_string();

                let subs = reg.on_frame(
                    vp_w, vp_h,
                    layout.viewport_x, layout.viewport_y,
                    layout.viewport_width, layout.viewport_height,
                    &cur_mode, &gs,
                    self.frame, dt_ms,
                );

                if !subs.is_empty() && self.frame % 300 == 0 {
                    tracing::debug!(count = subs.len(), "plugin submissions");
                }
                for sub in subs {
                    scene.elements.push(SceneElement::Textured {
                        x: sub.x, y: sub.y,
                        w: sub.width as f32, h: sub.height as f32,
                        tex_width: sub.width, tex_height: sub.height,
                        pixels: sub.pixels,
                        circle_clip: false, nearest_filter: false,
                        filter_target_colors: Vec::new(),
                        filter_output_color: [0.0; 4],
                        filter_sensitivity: 0.0,
                        filter_color_passthrough: false,
                        filter_border_color: [0.0; 4],
                        filter_border_width: 0,
                        filter_gamma_mode: 0,
                        custom_shader: None,
                    });
                }
            }
        }

        let t_plugins = Instant::now();

        let n_elems = scene.elements.len();

        // draw scene into game's backbuffer
        if let Some(ref mut renderer) = self.gl_renderer {
            renderer.draw_scene(&self.interop_gl, &scene.elements, vp_w, vp_h, scene.time);
        }

        let t_draw = Instant::now();

        // imgui overlay (always on top of everything)
        {
            let (mx, my) = tuxinjector_input::raw_mouse_position();
            let scroll = tuxinjector_input::take_gui_scroll();

            // retina: convert logical points -> framebuffer pixels
            let (sx, sy) = unsafe { crate::viewport_hook::content_scale() };

            let raw_keys = tuxinjector_input::take_gui_keys();
            let keys: Vec<(i32, bool, i32)> = raw_keys.into_iter()
                .map(|(k, mods, pressed)| (k, pressed, mods))
                .collect();

            let gui_input = GuiInput {
                pointer_pos: Some((mx as f32 * sx, my as f32 * sy)),
                pointer_button_pressed: tuxinjector_input::take_gui_button_press(),
                pointer_button_released: tuxinjector_input::take_gui_button_release(),
                rbutton_pressed: tuxinjector_input::take_gui_rbutton_press(),
                rbutton_released: tuxinjector_input::take_gui_rbutton_release(),
                pointer_button_mods: tuxinjector_input::take_gui_button_mods(),
                scroll_delta: scroll,
                keys,
                text: tuxinjector_input::take_gui_text(),
            };

            let perf = if cfg.advanced.debug.show_performance_overlay {
                state::get().perf_stats.get().map(|ps| ps.snapshot())
            } else {
                None
            };

            self.gui.render(vp_w, vp_h, &gui_input, perf.as_ref());
        }

        let t_gui = Instant::now();

        // log per-section timing every 300 frames
        static FTCTR: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let fc = FTCTR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if cfg.advanced.debug.log_performance && fc % 300 == 0 {
            tracing::info!(
                tick_us = t_tick.duration_since(t0).as_micros() as u64,
                mirrors_us = t_mirrors.duration_since(t_tick).as_micros() as u64,
                scene_us = t_scene.duration_since(t_mirrors).as_micros() as u64,
                plugins_us = t_plugins.duration_since(t_scene).as_micros() as u64,
                draw_us = t_draw.duration_since(t_plugins).as_micros() as u64,
                gui_us = t_gui.duration_since(t_draw).as_micros() as u64,
                elements = n_elems,
                total_us = t_gui.duration_since(t0).as_micros() as u64,
                "PERF: render_and_composite"
            );
        }

        Ok(())
    }

    // capture mirror regions from the game's framebuffer
    unsafe fn capture_mirrors(
        &mut self,
        cfg: &tuxinjector_config::Config,
        layout: &FrameLayout,
    ) {
        if layout.mirrors.is_empty() { return; }

        let gl = &self.local_gl;

        let ds = crate::mode_system::display_scale();
        let desired: Vec<(&str, u32, u32)> = layout.mirrors.iter()
            .filter_map(|ml| {
                cfg.overlays.mirrors.iter().find(|m| m.name == ml.mirror_id)
                    .map(|m| {
                        let w = (m.capture_width.max(1) as f32 * ds).round() as u32;
                        let h = (m.capture_height.max(1) as f32 * ds).round() as u32;
                        (ml.mirror_id.as_str(), w, h)
                    })
            })
            .collect();

        self.mirrors.sync_mirrors(gl, &desired);

        // detect game's texture-backed FBO (Sodium), cached to skip per-frame GL queries
        let game_w = layout.viewport_width as i32;
        let game_h = layout.viewport_height as i32;
        let (orig_w, orig_h) = crate::viewport_hook::get_original_size();
        let (mode_w, mode_h) = crate::viewport_hook::get_mode_size();
        let oversized = mode_w > 0 && mode_h > 0
            && crate::viewport_hook::is_oversized(mode_w, mode_h, orig_w, orig_h);

        let search_w = if oversized { mode_w } else { orig_w };
        let search_h = if oversized { mode_h } else { orig_h };

        // re-probe on mode change or every N frames
        const PROBE_INTERVAL: u64 = 120;
        let need_probe = match self.cached_probe {
            None => true,
            Some((_, _, cw, ch, f)) => {
                cw != search_w || ch != search_h
                    || self.frame.wrapping_sub(f) >= PROBE_INTERVAL
            }
        };
        let (game_fbo, game_tex_id) = if need_probe {
            let r = crate::swap_hook::find_game_fbo_and_texture(gl, search_w, search_h);
            self.cached_probe = Some((r.0, r.1, search_w, search_h, self.frame));
            r
        } else {
            let (fbo, tex, _, _, _) = self.cached_probe.unwrap();
            (fbo, tex)
        };

        let source_fbo = if game_fbo != 0 {
            Some(game_fbo)
        } else if oversized && crate::virtual_fb::is_active() {
            let vfb = crate::virtual_fb::virtual_fbo();
            if vfb != 0 { Some(vfb) } else { None }
        } else {
            None
        };

        // store game texture for zero-copy rendering
        let src_w = if oversized { mode_w } else { orig_w };
        let src_h = if oversized { mode_h } else { orig_h };
        self.game_tex = if game_tex_id != 0 {
            Some((game_tex_id, src_w, src_h))
        } else {
            None
        };

        let (vp_off_x, vp_off_y) = if source_fbo.is_some() {
            (0, 0)
        } else {
            ((orig_w as i32 - game_w) / 2, (orig_h as i32 - game_h) / 2)
        };

        // compute UV rects for game-texture-direct path
        self.game_uv_rects.clear();
        if self.game_tex.is_some() {
            let flip_h = if source_fbo.is_some() { game_h } else { orig_h as i32 };
            for ml in &layout.mirrors {
                let mcfg = match cfg.overlays.mirrors.iter().find(|m| m.name == ml.mirror_id) {
                    Some(m) => m,
                    None => continue,
                };
                if mcfg.input.len() != 1 { continue; }
                let inp = &mcfg.input[0];
                let viewport = GameViewportGeometry {
                    game_w, game_h,
                    final_x: vp_off_x, final_y: vp_off_y,
                    final_w: game_w, final_h: game_h,
                };
                let anchor = parse_relative_to(&inp.relative_to);
                let ds = crate::mode_system::display_scale();
                // Scale input offsets and capture size by display_scale on Retina:
                // config values are in MC GUI pixels, framebuffer is Nx on Retina.
                let scaled_x = (inp.x as f32 * ds).round() as i32;
                let scaled_y = (inp.y as f32 * ds).round() as i32;
                let scaled_cw = (mcfg.capture_width as f32 * ds).round() as i32;
                let scaled_ch = (mcfg.capture_height as f32 * ds).round() as i32;
                let (ax, ay) = resolve_relative_position(
                    anchor, scaled_x, scaled_y,
                    orig_w as i32, orig_h as i32, &viewport, 0, 0,
                    cfg.display.game_gui_scale, ds,
                );
                let gl_y = flip_h - (ay + scaled_ch);
                let cw = scaled_cw as f32;
                let ch = scaled_ch as f32;
                let tw = src_w as f32;
                let th = src_h as f32;
                let uv = [ax as f32 / tw, gl_y as f32 / th, (ax as f32 + cw) / tw, (gl_y as f32 + ch) / th];
                self.game_uv_rects.insert(ml.mirror_id.clone(), uv);
            }
        }

        // blit schedules (amortised to avoid ~1ms sync cost per blit on intel xe)
        const READBACK_INTERVAL: u64 = 60;
        const FILTERED_INTERVAL: u64 = 8;
        let do_blit = (self.frame % READBACK_INTERVAL == 0) || self.frame < 4;
        let do_blit_filtered = (self.frame % FILTERED_INTERVAL == 0) || self.frame < 4;

        let has_multi_unfiltered = layout.mirrors.iter().any(|ml| {
            let is_multi = cfg.overlays.mirrors.iter().find(|m| m.name == ml.mirror_id)
                .map(|m| m.input.len() > 1).unwrap_or(false);
            let is_filtered = !ml.filter_target_colors.is_empty();
            is_multi && !is_filtered
        });

        if do_blit || do_blit_filtered || has_multi_unfiltered {
            let t_cap = Instant::now();

            for ml in layout.mirrors.iter() {
                let mcfg = match cfg.overlays.mirrors.iter().find(|m| m.name == ml.mirror_id) {
                    Some(m) => m,
                    None => continue,
                };

                let viewport = GameViewportGeometry {
                    game_w, game_h,
                    final_x: vp_off_x, final_y: vp_off_y,
                    final_w: game_w, final_h: game_h,
                };
                let flip_h = if source_fbo.is_some() { game_h } else { orig_h as i32 };

                let gs = cfg.display.game_gui_scale;
                let ds = crate::mode_system::display_scale();
                // Scale capture dimensions and input offsets by display_scale on Retina
                let scaled_cw = (mcfg.capture_width as f32 * ds).round() as i32;
                let scaled_ch = (mcfg.capture_height as f32 * ds).round() as i32;
                let inputs: Vec<(i32, i32)> = mcfg.input.iter().map(|inp| {
                    let anchor = parse_relative_to(&inp.relative_to);
                    let sx = (inp.x as f32 * ds).round() as i32;
                    let sy = (inp.y as f32 * ds).round() as i32;
                    let (ax, ay) = resolve_relative_position(
                        anchor, sx, sy,
                        orig_w as i32, orig_h as i32, &viewport, 0, 0, gs, ds,
                    );
                    (ax, flip_h - (ay + scaled_ch))
                }).collect();

                if let Some(cap) = self.mirrors.get_mut(&ml.mirror_id) {
                    let is_multi = inputs.len() > 1;
                    let is_filtered = !mcfg.colors.target_colors.is_empty();

                    if is_multi && (!is_filtered || do_blit_filtered) {
                        cap.capture_multi_from(
                            gl, &inputs,
                            scaled_cw, scaled_ch,
                            source_fbo, mcfg.nearest_filter,
                        );
                    } else if !is_multi && is_filtered && do_blit_filtered {
                        let (sx, sy) = inputs.first().copied().unwrap_or((
                            vp_off_x + game_w / 2 - scaled_cw / 2,
                            vp_off_y + game_h / 2 - scaled_ch / 2,
                        ));
                        cap.capture_from(
                            gl, sx, sy,
                            scaled_cw, scaled_ch,
                            source_fbo, mcfg.nearest_filter,
                            false, // need readback for filtered mirrors
                        );
                    } else if do_blit {
                        let (sx, sy) = inputs.first().copied().unwrap_or((
                            vp_off_x + game_w / 2 - scaled_cw / 2,
                            vp_off_y + game_h / 2 - scaled_ch / 2,
                        ));
                        cap.capture_from(
                            gl, sx, sy,
                            scaled_cw, scaled_ch,
                            source_fbo, mcfg.nearest_filter,
                            true, // skip readback for TextureRef
                        );
                    }
                }
            }

            let cap_us = t_cap.elapsed().as_micros();

            static CAP_DIAG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let cd = CAP_DIAG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if cfg.advanced.debug.log_performance && cd % 60 == 0 {
                tracing::info!(
                    cap_us,
                    source_fbo = source_fbo.unwrap_or(0),
                    mirrors = layout.mirrors.len(),
                    game_tex = game_tex_id,
                    "PERF: capture_mirrors (periodic blit)"
                );
            }

            (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, 0);
            (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
        }
    }

    // transition target if bouncing, otherwise current mode
    pub fn effective_mode_id(&self) -> &str {
        self.mode_system.effective_mode_id()
    }

    pub fn initial_mode_id(&self) -> &str {
        self.mode_system.initial_mode_id()
    }

    pub fn switch_mode(&mut self, mode_id: &str) {
        let cfg = self.config.load();
        let (orig_w, orig_h) = crate::viewport_hook::get_original_size();
        if orig_w > 0 && orig_h > 0 {
            self.mode_system.update_screen_size(orig_w, orig_h);
        }
        tracing::info!(
            mode_id, orig_w, orig_h,
            modes = cfg.modes.len(),
            mirrors = cfg.overlays.mirrors.len(),
            "switch_mode"
        );
        let prev = self.mode_system.current_mode_id().to_string();
        self.mode_system.switch_mode(mode_id, &cfg);
        self.cached_probe = None; // mode dims are changing

        if let Some(ref mut r) = self.gl_renderer {
            r.invalidate_gl_state_cache();
        }

        // notify plugins
        if let Some(lock) = state::get().plugins.get() {
            if let Ok(mut reg) = lock.lock() {
                reg.broadcast_mode_switch(&prev, mode_id);
            }
        }

        // pre-warm bg image cache to avoid black flash
        if let Some(m) = cfg.modes.iter().find(|m| m.id == mode_id) {
            if m.background.selected_mode == "image" && !m.background.image.is_empty() {
                let _ = self.bg_cache.get(&m.background.image);
            }
        }

        // fire framebuffer-size event for the new resolution
        if let Some((mw, mh)) = self.mode_system.dimensions_for_mode(mode_id, &cfg) {
            tracing::info!(mode_id, mw, mh, "switch_mode: firing fb resize");
            if mw > 16384 || mh > 16384 {
                tracing::warn!(
                    mode = mode_id, width = mw, height = mh,
                    "WARNING: SPEEDRUN.COM ILLEGALLY SIZED RESOLUTION TOGGLED:"
                );
            }
            unsafe { crate::viewport_hook::fire_framebuffer_resize(mw, mh) };
        } else {
            tracing::warn!(mode_id, "switch_mode: dimensions_for_mode returned None!");
        }
    }

    pub fn toggle_gui(&mut self) {
        self.gui.toggle();
        tuxinjector_input::set_gui_visible(self.gui.is_visible());

        if self.gui.is_visible() {
            unsafe { tuxinjector_input::force_cursor_visible(); }

            // write onboarding sentinel so the startup toast doesn't show again
            if let Some(dir) = crate::state::get().config_dir.get() {
                let sentinel = dir.join(".onboarded");
                if !sentinel.exists() { let _ = std::fs::write(&sentinel, ""); }
            }
        } else {
            tuxinjector_input::set_gui_wants_keyboard(false);
            unsafe { tuxinjector_input::restore_game_cursor(); }
        }
    }

    pub fn toggle_app_visibility(&mut self) {
        self.app_capture.toggle_visibility();
    }

    fn load_custom_shaders(&mut self, cfg: &tuxinjector_config::Config) {
        let mut names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for mirror in &cfg.overlays.mirrors {
            if !mirror.shader.is_empty() { names.insert(mirror.shader.clone()); }
        }

        if names.is_empty() {
            if let Some(ref mut r) = self.gl_renderer {
                unsafe { r.update_custom_shaders(&self.interop_gl, &HashMap::new()); }
            }
            return;
        }

        let shader_dir = match crate::state::get().config_dir.get().map(|p| p.clone()) {
            Some(dir) => dir.join("shaders"),
            None => return,
        };

        let mut sources: HashMap<String, String> = HashMap::new();
        for name in &names {
            let path = shader_dir.join(format!("{name}.glsl"));
            match std::fs::read_to_string(&path) {
                Ok(src) => { sources.insert(name.clone(), src); }
                Err(e) => {
                    tracing::error!(name = %name, path = %path.display(), error = %e,
                        "failed to read custom shader");
                }
            }
        }

        if let Some(ref mut r) = self.gl_renderer {
            unsafe { r.update_custom_shaders(&self.interop_gl, &sources); }
        }
    }

    pub fn toggle_image_overlays(&mut self) {
        self.images_visible = !self.images_visible;
        tracing::info!(visible = self.images_visible, "toggled image overlays");
    }

    pub fn toggle_window_overlays(&mut self) {
        self.windows_visible = !self.windows_visible;
        tracing::info!(visible = self.windows_visible, "toggled window overlays");
    }

}

impl Drop for OverlayState {
    fn drop(&mut self) {
        if let Some(ref mut r) = self.gl_renderer {
            unsafe { r.destroy(&self.interop_gl) };
        }
        unsafe { self.mirrors.destroy(&self.local_gl) };
    }
}

// --- scene building ---

fn build_scene(
    layout: &FrameLayout,
    sw: u32,
    sh: u32,
    mirrors: &mut MirrorCaptureManager,
    img_cache: &mut ImageCache,
    bg_cache: &mut BgImageCache,
    cursor_cache: &mut CursorCache,
    text_cache: &mut TextOverlayCache,
    cfg: &tuxinjector_config::Config,
    game_tex: Option<(u32, u32, u32)>,
    game_uv_rects: &HashMap<String, [f32; 4]>,
    images_visible: bool,
    windows_visible: bool,
    win_capture: &WindowCaptureManager,
    browser_capture: &crate::browser_capture::BrowserCaptureManager,
    time: f32,
) -> SceneDescription {
    let mut elems = Vec::new();

    let vx = layout.viewport_x;
    let vy = layout.viewport_y;
    let vw = layout.viewport_width;
    let vh = layout.viewport_height;
    let sw_f = sw as f32;
    let sh_f = sh as f32;

    // background fills the margins around the viewport
    match &layout.background {
        BackgroundSpec::None => {}
        BackgroundSpec::SolidColor(color) => {
            if vx > 0.0 {
                elems.push(SceneElement::SolidRect { x: 0.0, y: 0.0, w: vx, h: sh_f, color: *color });
            }
            let right = vx + vw;
            if right < sw_f {
                elems.push(SceneElement::SolidRect { x: right, y: 0.0, w: sw_f - right, h: sh_f, color: *color });
            }
            if vy > 0.0 {
                elems.push(SceneElement::SolidRect { x: vx, y: 0.0, w: vw, h: vy, color: *color });
            }
            let bot = vy + vh;
            if bot < sh_f {
                elems.push(SceneElement::SolidRect { x: vx, y: bot, w: vw, h: sh_f - bot, color: *color });
            }
        }
        BackgroundSpec::Gradient { stops, angle, animation, speed } => {
            let c1 = stops.first().map(|(c, _)| *c).unwrap_or([0.0; 4]);
            let c2 = stops.last().map(|(c, _)| *c).unwrap_or([0.0; 4]);

            let anim_type = match animation {
                tuxinjector_config::types::GradientAnimationType::None => 0,
                tuxinjector_config::types::GradientAnimationType::Rotate => 1,
                tuxinjector_config::types::GradientAnimationType::Slide => 2,
                tuxinjector_config::types::GradientAnimationType::Wave => 3,
                tuxinjector_config::types::GradientAnimationType::Spiral => 4,
                tuxinjector_config::types::GradientAnimationType::Fade => 5,
            };

            static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            let elapsed = T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f32() * speed;

            // 4 scissored gradient draws in the margins
            let rects: &[[f32; 4]] = &[
                [0.0, 0.0, vx, sh_f],
                [vx + vw, 0.0, sw_f - (vx + vw), sh_f],
                [vx, 0.0, vw, vy],
                [vx, vy + vh, vw, sh_f - (vy + vh)],
            ];
            for r in rects {
                if r[2] > 0.0 && r[3] > 0.0 {
                    elems.push(SceneElement::Gradient {
                        color1: c1, color2: c2,
                        angle: *angle, time: elapsed,
                        animation_type: anim_type,
                        scissor: Some(*r),
                    });
                }
            }
        }
        BackgroundSpec::Image { path } => {
            // HACK: sample center pixel color and fill margins with it
            // somebody should do proper image stretching here, but this is fine for now
            if let Some((px, _tw, _th)) = bg_cache.get(path) {
                let mid = px.len() / 2;
                let color = if mid + 4 <= px.len() {
                    [px[mid] as f32 / 255.0, px[mid+1] as f32 / 255.0,
                     px[mid+2] as f32 / 255.0, px[mid+3] as f32 / 255.0]
                } else {
                    [0.0, 0.0, 0.0, 1.0]
                };
                if vx > 0.0 {
                    elems.push(SceneElement::SolidRect { x: 0.0, y: 0.0, w: vx, h: sh_f, color });
                }
                let right = vx + vw;
                if right < sw_f {
                    elems.push(SceneElement::SolidRect { x: right, y: 0.0, w: sw_f - right, h: sh_f, color });
                }
                if vy > 0.0 {
                    elems.push(SceneElement::SolidRect { x: vx, y: 0.0, w: vw, h: vy, color });
                }
                let bot = vy + vh;
                if bot < sh_f {
                    elems.push(SceneElement::SolidRect { x: vx, y: bot, w: vw, h: sh_f - bot, color });
                }
            }
        }
    }

    // --- mirrors ---
    for ml in &layout.mirrors {
        let mut rendered = false;
        if let Some(cap) = mirrors.get_mut(&ml.mirror_id) {
            let bpad = ml.filter_border_width.max(0) as u32;

            // multi-input mirrors produce multiple overlapping elements
            if let Some(multi) = cap.peek_multi_pixels() {
                let mut any_match = ml.filter_target_colors.is_empty();
                for (tw, th, pixels) in multi {
                    if !any_match {
                        any_match = has_matching_pixels(&pixels, &ml.filter_target_colors, ml.filter_sensitivity, 4);
                    }
                    let (ftw, fth, fpx, fx, fy, fw, fh) = if bpad > 0 {
                        let (pw, ph, pp) = pad_pixels(tw, th, &pixels, bpad);
                        let sx = pw as f32 / tw as f32;
                        let sy = ph as f32 / th as f32;
                        let nw = ml.render_width * sx;
                        let nh = ml.render_height * sy;
                        (pw, ph, pp,
                         ml.render_x - (nw - ml.render_width) / 2.0,
                         ml.render_y - (nh - ml.render_height) / 2.0,
                         nw, nh)
                    } else {
                        (tw, th, pixels, ml.render_x, ml.render_y, ml.render_width, ml.render_height)
                    };
                    let cs = if ml.custom_shader.is_empty() { None } else { Some(ml.custom_shader.clone()) };
                    elems.push(SceneElement::Textured {
                        x: fx, y: fy, w: fw, h: fh,
                        tex_width: ftw, tex_height: fth, pixels: fpx,
                        circle_clip: ml.circle_clip, nearest_filter: ml.nearest_filter,
                        filter_target_colors: ml.filter_target_colors.clone(),
                        filter_output_color: ml.filter_output_color,
                        filter_sensitivity: ml.filter_sensitivity,
                        filter_color_passthrough: ml.filter_color_passthrough,
                        filter_border_color: ml.filter_border_color,
                        filter_border_width: ml.filter_border_width,
                        filter_gamma_mode: ml.filter_gamma_mode,
                        custom_shader: cs,
                    });
                }
                rendered = if ml.filter_target_colors.is_empty() { true } else { any_match };
            } else {
                // single-input mirror
                let is_filtered = !ml.filter_target_colors.is_empty();

                if is_filtered {
                    // filtered: CPU pixel path with cached data between blits
                    let (tw, th) = cap.capture_dimensions();
                    if let Some(pixels) = cap.check_pixels() {
                        let pixels = pixels.to_vec();
                        let visible = has_matching_pixels(
                            &pixels, &ml.filter_target_colors, ml.filter_sensitivity, 8,
                        );

                        let (ftw, fth, fpx, fx, fy, fw, fh) = if bpad > 0 {
                            let (pw, ph, pp) = pad_pixels(tw, th, &pixels, bpad);
                            let sx = pw as f32 / tw as f32;
                            let sy = ph as f32 / th as f32;
                            let nw = ml.render_width * sx;
                            let nh = ml.render_height * sy;
                            (pw, ph, pp,
                             ml.render_x - (nw - ml.render_width) / 2.0,
                             ml.render_y - (nh - ml.render_height) / 2.0,
                             nw, nh)
                        } else {
                            (tw, th, pixels, ml.render_x, ml.render_y, ml.render_width, ml.render_height)
                        };
                        let cs = if ml.custom_shader.is_empty() { None } else { Some(ml.custom_shader.clone()) };
                        elems.push(SceneElement::Textured {
                            x: fx, y: fy, w: fw, h: fh,
                            tex_width: ftw, tex_height: fth, pixels: fpx,
                            circle_clip: ml.circle_clip, nearest_filter: ml.nearest_filter,
                            filter_target_colors: ml.filter_target_colors.clone(),
                            filter_output_color: ml.filter_output_color,
                            filter_sensitivity: ml.filter_sensitivity,
                            filter_color_passthrough: ml.filter_color_passthrough,
                            filter_border_color: ml.filter_border_color,
                            filter_border_width: ml.filter_border_width,
                            filter_gamma_mode: ml.filter_gamma_mode,
                            custom_shader: cs,
                        });
                        rendered = visible;
                    }
                } else {
                    // unfiltered: zero-copy TextureRef, prefer game texture when available
                    let (tid, _tw, _th, uv) = if let (Some(uv), Some((gt, gw, gh))) =
                        (game_uv_rects.get(&ml.mirror_id), game_tex)
                    {
                        (gt, gw, gh, Some(*uv))
                    } else {
                        let id = cap.texture_id();
                        let (cw, ch) = cap.capture_dimensions();
                        (id, cw, ch, None)
                    };
                    let (cw, ch) = cap.capture_dimensions();
                    if tid != 0 && cw > 0 && ch > 0 {
                        let cs = if ml.custom_shader.is_empty() { None } else { Some(ml.custom_shader.clone()) };
                        elems.push(SceneElement::TextureRef {
                            x: ml.render_x, y: ml.render_y,
                            w: ml.render_width, h: ml.render_height,
                            gl_texture: tid,
                            tex_width: cw, tex_height: ch,
                            flip_v: true,
                            circle_clip: ml.circle_clip,
                            nearest_filter: ml.nearest_filter,
                            filter_target_colors: Vec::new(),
                            filter_output_color: [0.0; 4],
                            filter_sensitivity: 0.0,
                            filter_color_passthrough: false,
                            filter_border_color: [0.0; 4],
                            filter_border_width: 0,
                            filter_gamma_mode: 0,
                            uv_rect: uv,
                            custom_shader: cs,
                        });
                        rendered = true;
                    }
                }
            }
        }

        // placeholder for raw mirrors while data arrives
        if !rendered && ml.filter_target_colors.is_empty() {
            elems.push(SceneElement::SolidRect {
                x: ml.render_x, y: ml.render_y,
                w: ml.render_width, h: ml.render_height,
                color: [0.15, 0.15, 0.2, 0.8],
            });
        }

        // static border around mirror
        if ml.static_border_enabled && rendered {
            let bw = if ml.static_border_rect_w > 0.0 { ml.static_border_rect_w } else { ml.render_width };
            let bh = if ml.static_border_rect_h > 0.0 { ml.static_border_rect_h } else { ml.render_height };
            let cx = (bw - ml.render_width) / 2.0;
            let cy = (bh - ml.render_height) / 2.0;

            elems.push(SceneElement::Border {
                x: ml.render_x - cx + ml.static_border_offset_x,
                y: ml.render_y - cy + ml.static_border_offset_y,
                w: bw, h: bh,
                border_width: ml.static_border_width,
                radius: ml.static_border_radius,
                color: ml.static_border_color,
            });
        }
    }

    // --- images ---
    for il in if images_visible { layout.images.as_slice() } else { &[] } {
        if let Some((px, tw, th)) = img_cache.get(&il.image_id, cfg) {
            // only clone if we need to scale alpha
            let px = if il.opacity < 1.0 {
                let mut buf = px.to_vec();
                let a = (il.opacity * 255.0) as u8;
                for chunk in buf.chunks_exact_mut(4) {
                    chunk[3] = ((chunk[3] as u16 * a as u16) / 255) as u8;
                }
                buf
            } else {
                px.to_vec()
            };

            let (scale, out_w, out_h) = cfg.overlays.images.iter()
                .find(|i| i.name == il.image_id)
                .map(|i| (i.scale, i.output_width, i.output_height))
                .unwrap_or((1.0, 0, 0));

            // priority: eyezoom_link dims > explicit output size > tex*scale
            let rw = if il.width > 0.0 { il.width }
                     else if out_w > 0 { out_w as f32 }
                     else { tw as f32 * scale };
            let rh = if il.height > 0.0 { il.height }
                     else if out_h > 0 { out_h as f32 }
                     else { th as f32 * scale };

            elems.push(SceneElement::Textured {
                x: il.x, y: il.y, w: rw, h: rh,
                tex_width: tw, tex_height: th, pixels: px,
                circle_clip: false, nearest_filter: false,
                filter_target_colors: Vec::new(),
                filter_output_color: [0.0; 4],
                filter_sensitivity: 0.0,
                filter_color_passthrough: false,
                filter_border_color: [0.0; 4],
                filter_border_width: 0,
                filter_gamma_mode: 0,
                custom_shader: None,
            });
        }
    }

    // --- text overlays ---
    let theme_font = &cfg.theme.font_path;
    for tl in &layout.text_overlays {
        let tcfg = match cfg.overlays.text_overlays.iter().find(|t| t.name == tl.text_overlay_id) {
            Some(t) => t,
            None => continue,
        };

        if let Some((px, tw, th)) = text_cache.get_or_rasterize(&tl.text_overlay_id, tcfg, theme_font) {
            let px = if tl.opacity < 1.0 {
                let mut buf = px.to_vec();
                let a = (tl.opacity * 255.0) as u8;
                for chunk in buf.chunks_exact_mut(4) {
                    chunk[3] = ((chunk[3] as u16 * a as u16) / 255) as u8;
                }
                buf
            } else {
                px.to_vec()
            };

            elems.push(SceneElement::Textured {
                x: tl.x, y: tl.y, w: tw as f32, h: th as f32,
                tex_width: tw, tex_height: th, pixels: px,
                circle_clip: false, nearest_filter: false,
                filter_target_colors: Vec::new(),
                filter_output_color: [0.0; 4],
                filter_sensitivity: 0.0,
                filter_color_passthrough: false,
                filter_border_color: [0.0; 4],
                filter_border_width: 0,
                filter_gamma_mode: 0,
                custom_shader: None,
            });

            if tcfg.border.enabled {
                elems.push(SceneElement::Border {
                    x: tl.x, y: tl.y,
                    w: tw as f32, h: th as f32,
                    border_width: tcfg.border.width as f32,
                    radius: tcfg.border.radius as f32,
                    color: tcfg.border.color.to_array(),
                });
            }
        }
    }

    // --- window overlays ---
    if windows_visible {
        for wol in &layout.window_overlays {
            if let Some(frame) = win_capture.latest_frame(&wol.wo_id) {
                let fw = frame.width as i32;
                let fh = frame.height as i32;

                // apply crop
                let cx0 = wol.crop_left.min(fw);
                let cy0 = wol.crop_top.min(fh);
                let cx1 = wol.crop_right.min(fw - cx0);
                let cy1 = wol.crop_bottom.min(fh - cy0);
                let cw = (fw - cx0 - cx1).max(0) as u32;
                let ch = (fh - cy0 - cy1).max(0) as u32;

                if cw == 0 || ch == 0 { continue; }

                // crop the pixel data
                let pixels = if cx0 > 0 || cy0 > 0 || cx1 > 0 || cy1 > 0 {
                    let mut cropped = Vec::with_capacity((cw * ch * 4) as usize);
                    for row in cy0..(fh - cy1) {
                        let src_start = (row as u32 * frame.width + cx0 as u32) as usize * 4;
                        let src_end = src_start + (cw as usize * 4);
                        if src_end <= frame.pixels.len() {
                            cropped.extend_from_slice(&frame.pixels[src_start..src_end]);
                        }
                    }
                    cropped
                } else {
                    frame.pixels
                };

                // apply opacity
                let pixels = if wol.opacity < 1.0 {
                    let a = (wol.opacity * 255.0) as u8;
                    let mut buf = pixels;
                    for chunk in buf.chunks_exact_mut(4) {
                        chunk[3] = ((chunk[3] as u16 * a as u16) / 255) as u8;
                    }
                    buf
                } else {
                    pixels
                };

                let render_w = cw as f32 * wol.scale;
                let render_h = ch as f32 * wol.scale;

                elems.push(SceneElement::Textured {
                    x: wol.x, y: wol.y,
                    w: render_w, h: render_h,
                    tex_width: cw, tex_height: ch,
                    pixels,
                    circle_clip: false,
                    nearest_filter: wol.pixelated_scaling,
                    filter_target_colors: Vec::new(),
                    filter_output_color: [0.0; 4],
                    filter_sensitivity: 0.0,
                    filter_color_passthrough: false,
                    filter_border_color: [0.0; 4],
                    filter_border_width: 0,
                    filter_gamma_mode: 0,
                    custom_shader: None,
                });

                // border
                if let Some(ref border) = wol.border {
                    elems.push(SceneElement::Border {
                        x: wol.x, y: wol.y,
                        w: render_w, h: render_h,
                        border_width: border.border_width,
                        radius: border.radius,
                        color: border.color,
                    });
                }
            }
        }
    }

    // --- browser overlays ---
    // same rendering path as window overlays, just different capture source
    for (bo_idx, bo_cfg) in cfg.overlays.browser_overlays.iter().enumerate() {
        if bo_cfg.url.is_empty() { continue; }
        let bo_key = if bo_cfg.name.is_empty() { format!("__browser_{bo_idx}") } else { bo_cfg.name.clone() };
        if let Some(frame) = browser_capture.latest_frame(&bo_key) {
            let fw = frame.width as i32;
            let fh = frame.height as i32;

            let cx0 = bo_cfg.crop_left.min(fw);
            let cy0 = bo_cfg.crop_top.min(fh);
            let cx1 = bo_cfg.crop_right.min(fw - cx0);
            let cy1 = bo_cfg.crop_bottom.min(fh - cy0);
            let cw = (fw - cx0 - cx1).max(0) as u32;
            let ch = (fh - cy0 - cy1).max(0) as u32;
            if cw == 0 || ch == 0 { continue; }

            let pixels = if cx0 > 0 || cy0 > 0 || cx1 > 0 || cy1 > 0 {
                let mut cropped = Vec::with_capacity((cw * ch * 4) as usize);
                for row in cy0..(fh - cy1) {
                    let src_start = (row as u32 * frame.width + cx0 as u32) as usize * 4;
                    let src_end = src_start + (cw as usize * 4);
                    if src_end <= frame.pixels.len() {
                        cropped.extend_from_slice(&frame.pixels[src_start..src_end]);
                    }
                }
                cropped
            } else {
                frame.pixels.clone()
            };

            let pixels = if bo_cfg.opacity < 1.0 {
                let a = (bo_cfg.opacity * 255.0) as u8;
                let mut buf = pixels;
                for chunk in buf.chunks_exact_mut(4) {
                    chunk[3] = ((chunk[3] as u16 * a as u16) / 255) as u8;
                }
                buf
            } else {
                pixels
            };

            let render_w = cw as f32 * bo_cfg.scale;
            let render_h = ch as f32 * bo_cfg.scale;

            elems.push(SceneElement::Textured {
                x: bo_cfg.x as f32,
                y: bo_cfg.y as f32,
                w: render_w, h: render_h,
                tex_width: cw, tex_height: ch,
                pixels,
                circle_clip: false,
                nearest_filter: bo_cfg.pixelated_scaling,
                filter_target_colors: Vec::new(),
                filter_output_color: [0.0; 4],
                filter_sensitivity: 0.0,
                filter_color_passthrough: false,
                filter_border_color: [0.0; 4],
                filter_border_width: 0,
                filter_gamma_mode: 0,
                custom_shader: None,
            });
        }
    }

    // mode border
    if let Some(ref border) = layout.border {
        elems.push(SceneElement::Border {
            x: border.x, y: border.y,
            w: border.width, h: border.height,
            border_width: border.border_width,
            radius: border.radius,
            color: border.color,
        });
    }

    // custom cursor should only be shown when the game is showing a cursor
    if cfg.theme.cursors.enabled {
        let captured = tuxinjector_input::is_cursor_captured();
        let gs = tuxinjector_lua::get_game_state();

        let cc = if captured {
            None // cursor grabbed = FPS mode, no cursor
        } else {
            match gs.as_str() {
                // inworld with menu open (not grabbed) — show ingame cursor
                s if s.starts_with("inworld") => Some(&cfg.theme.cursors.ingame),
                "wall" => Some(&cfg.theme.cursors.wall),
                "title" | "waiting" => Some(&cfg.theme.cursors.title),
                // generating, previewing, etc -  no cursor
                _ => None,
            }
        };

        if let Some(cc) = cc {
            if !cc.cursor_name.is_empty() {
                if let Some((px, cw, ch, _hx, _hy)) = cursor_cache.get_cursor(&cc.cursor_name, cc.cursor_size) {
                    let (mx, my) = tuxinjector_input::raw_mouse_position();
                    let hx = cw as f32 * cc.hotspot_x;
                    let hy = ch as f32 * cc.hotspot_y;

                    elems.push(SceneElement::Textured {
                        x: mx as f32 - hx, y: my as f32 - hy,
                        w: cw as f32, h: ch as f32,
                        tex_width: cw, tex_height: ch,
                        pixels: px.to_vec(),
                        circle_clip: false, nearest_filter: true,
                        filter_target_colors: Vec::new(),
                        filter_output_color: [0.0; 4],
                        filter_sensitivity: 0.0,
                        filter_color_passthrough: false,
                        filter_border_color: [0.0; 4],
                        filter_border_width: 0,
                        filter_gamma_mode: 0,
                        custom_shader: None,
                    });
                }
            }
        }
    }

    SceneDescription {
        clear_color: [0.0, 0.0, 0.0, 0.0],
        elements: elems,
        time,
    }
}
