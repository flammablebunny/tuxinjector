// SwapBuffers hooks -- render our overlay then forward to the real swap fn.
// Linux uses EGL/GLX, macOS uses CGL (or glfwSwapBuffers directly).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

use crate::gl_resolve;
use crate::overlay::OverlayState;
use crate::state;

extern crate libc;

// -- Linux EGL/GLX --
#[cfg(target_os = "linux")]
type EglSwapBuffersFn = unsafe extern "C" fn(display: *mut c_void, surface: *mut c_void) -> i32;
#[cfg(target_os = "linux")]
type GlxSwapBuffersFn = unsafe extern "C" fn(display: *mut c_void, drawable: u64);
#[cfg(target_os = "linux")]
static REAL_EGL_SWAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
#[cfg(target_os = "linux")]
static REAL_GLX_SWAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
#[cfg(target_os = "linux")]
static ORIGINAL_EGL_SWAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
#[cfg(target_os = "linux")]
static ORIGINAL_GLX_SWAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// -- macOS CGL --
#[cfg(target_os = "macos")]
type CglFlushDrawableFn = unsafe extern "C" fn(ctx: *mut c_void) -> i32;
#[cfg(target_os = "macos")]
static REAL_CGL_FLUSH: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

static FRAME_COUNT: AtomicU64 = AtomicU64::new(0);

// next frame deadline (monotonic ns). 0 = first frame, haven't set it yet
static NEXT_FRAME_NS: AtomicU64 = AtomicU64::new(0);

#[cfg(target_os = "linux")]
fn clock_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

#[cfg(target_os = "macos")]
fn clock_ns() -> u64 {
    use std::sync::OnceLock;

    #[repr(C)]
    struct MachTimebaseInfo { numer: u32, denom: u32 }
    extern "C" {
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
    }

    static INFO: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *INFO.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        unsafe { mach_timebase_info(&mut info) };
        (info.numer, info.denom)
    });

    let ticks = unsafe { mach_absolute_time() };
    ticks * numer as u64 / denom as u64
}

// Sleep until close to the target, then spin-wait the rest to absorb
// scheduler jitter. spin_threshold_us controls where sleep stops.
fn frame_limit(fps: i32, spin_us: i32) {
    if fps <= 0 {
        NEXT_FRAME_NS.store(0, Ordering::Relaxed);
        return;
    }

    let frame_ns = 1_000_000_000u64 / fps as u64;
    let spin_ns = spin_us.max(0) as u64 * 1_000;
    let now = clock_ns();

    let target = {
        let stored = NEXT_FRAME_NS.load(Ordering::Relaxed);
        if stored == 0 {
            NEXT_FRAME_NS.store(now + frame_ns, Ordering::Relaxed);
            return;
        }
        stored
    };

    if target > now {
        let remaining = target - now;

        if remaining > spin_ns {
            let sleep_until = target - spin_ns;
            precise_sleep_until(sleep_until);
        }

        // spin the last few us to absorb jitter
        while clock_ns() < target {
            std::hint::spin_loop();
        }
    }

    // advance target; resync if we've fallen behind by more than a frame
    let now2 = clock_ns();
    let next = if now2 > target + frame_ns {
        now2 + frame_ns
    } else {
        target + frame_ns
    };
    NEXT_FRAME_NS.store(next, Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
fn precise_sleep_until(target_ns: u64) {
    let ts = libc::timespec {
        tv_sec:  (target_ns / 1_000_000_000) as libc::time_t,
        tv_nsec: (target_ns % 1_000_000_000) as libc::c_long,
    };
    unsafe {
        libc::clock_nanosleep(
            libc::CLOCK_MONOTONIC,
            libc::TIMER_ABSTIME,
            &ts,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(target_os = "macos")]
fn precise_sleep_until(target_ns: u64) {
    // mach_wait_until wants mach absolute time, not ns --
    // convert back using the inverse of the timebase ratio
    use std::sync::OnceLock;

    #[repr(C)]
    struct MachTimebaseInfo { numer: u32, denom: u32 }
    extern "C" {
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
        fn mach_wait_until(deadline: u64) -> i32;
    }

    static INFO: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *INFO.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        unsafe { mach_timebase_info(&mut info) };
        (info.numer, info.denom)
    });

    let ticks = target_ns * denom as u64 / numer as u64;
    unsafe { mach_wait_until(ticks); }
}

// when the GUI is open, don't let the framerate drop below this
// or the UI feels awful
const OVERLAY_MIN_FPS: i32 = 30;

fn effective_fps(configured: i32) -> i32 {
    if configured > 0 && configured < OVERLAY_MIN_FPS && tuxinjector_input::gui_is_visible() {
        OVERLAY_MIN_FPS
    } else {
        configured
    }
}

const LOG_INTERVAL: u64 = 300; // ~5s at 60fps

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static INIT_FAILED: AtomicBool = AtomicBool::new(false);
static INIT_ONCE: OnceLock<()> = OnceLock::new();

// One-time deferred init: resolve GL, create overlay, load plugins.
// Runs on the first SwapBuffers call because we need a GL context.
fn first_frame_init() {
    INIT_ONCE.get_or_init(|| {
        tracing::info!("tuxinjector: first frame -- running deferred init");

        let gpa = gl_resolve::get_proc_address_fn();
        if gpa.is_none() {
            tracing::error!("tuxinjector: neither eglGetProcAddress nor glXGetProcAddressARB available -- can't init");
            INIT_FAILED.store(true, Ordering::Release);
            return;
        }
        let gpa = gpa.unwrap();

        let gl = unsafe { crate::gl_resolve::GlFunctions::resolve(gpa) };
        let tx = state::init_or_get();
        let _ = tx.gl.set(gl);

        let config = std::sync::Arc::clone(&tx.config);
        match unsafe { OverlayState::new(gpa, config) } {
            Ok(overlay) => {
                let _ = tx.overlay.set(std::sync::Mutex::new(overlay));
                INITIALIZED.store(true, Ordering::Release);
                tracing::info!("tuxinjector: overlay ready");

                // show onboarding toast if this is the first time
                let onboarded = state::get()
                    .config_dir.get()
                    .map(|d| d.join(".onboarded").exists())
                    .unwrap_or(false);
                if !onboarded {
                    tuxinjector_gui::toast::push(
                        "Tuxinjector active - press Ctrl+I to open configuration settings",
                    );
                }

                let ps = crate::perf_stats::PerfStats::new();
                let _ = tx.perf_stats.set(ps);

                // discover and load plugins from the plugins dir
                let saved = crate::plugin_loader::load_plugin_settings();
                let loaded = crate::plugin_loader::discover_and_load(&saved);
                let registry = crate::plugin_registry::PluginRegistry::new(loaded, saved);
                let _ = tx.plugins.set(std::sync::Mutex::new(registry));
            }
            Err(e) => {
                tracing::error!("tuxinjector: overlay init failed: {e}");
                INIT_FAILED.store(true, Ordering::Release);
            }
        }

        let input_cfg = std::sync::Arc::clone(&tx.config);
        let mut handler = crate::input_handler::TuxinjectorInputHandler::new(input_cfg);

        if let Some(bindings) = tx.lua_bindings.lock().unwrap().take() {
            tracing::info!(count = bindings.len(), "registering Lua actions with hotkey engine");
            handler.register_lua_actions(&bindings);
        }

        if let Some(runtime) = tx.lua_runtime.get() {
            handler.set_lua_callback_channel(runtime.callback_tx.clone());
            tracing::info!("tuxinjector: Lua callback channel wired up");
        } else {
            tracing::warn!("tuxinjector: no Lua runtime -- hotkey callbacks will be dropped");
        }

        tuxinjector_input::register_input_handler(Box::new(handler));
        tracing::info!("tuxinjector: input handler registered");

        // install inline hooks (creates trampolines), then immediately unpatch
        // since we start in fullscreen - hooks are only patched on mode switch.
        // LWJGL has Mesa's real pointers cached (getProcAddress returns real),
        // so inline hooks are the ONLY interception path for GL calls.
        #[cfg(target_os = "linux")]
        unsafe {
            crate::viewport_hook::install_glviewport_inline_hook();
            crate::viewport_hook::install_glbindframebuffer_inline_hook();
            crate::viewport_hook::unpatch_inline_hooks();
        };
    });
}

// grab the physical surface size from GL_VIEWPORT when no mode resize is active
unsafe fn capture_original_size() {
    let (mw, _) = crate::viewport_hook::get_mode_size();
    if mw > 0 { return; }

    let tx = state::get();
    if let Some(gl) = tx.gl.get() {
        let mut vp = [0i32; 4];
        (gl.get_integer_v)(0x0BA2 /* GL_VIEWPORT */, vp.as_mut_ptr());
        let w = vp[2] as u32;
        let h = vp[3] as u32;
        if w > 0 && h > 0 {
            crate::viewport_hook::force_store_original_size(w, h);
        }
    }
}

// --- child process tracking for tx.exec() ---

static EXEC_CHILDREN: std::sync::Mutex<Vec<(std::process::Child, String)>> =
    std::sync::Mutex::new(Vec::new());

// reap finished child processes so we don't leak zombies
fn reap_children() {
    let mut guard = match EXEC_CHILDREN.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    guard.retain_mut(|(child, name)| {
        match child.try_wait() {
            Ok(Some(status)) => {
                let pid = child.id();
                tracing::info!(pid, name = %name, ?status, "exec: child exited");
                tuxinjector_gui::running_apps::unregister(pid);
                false
            }
            Ok(None) => true,
            Err(e) => {
                tracing::warn!(name = %name, %e, "exec: try_wait error");
                true
            }
        }
    });
}

// dispatch pending commands from the Lua runtime (called each frame)
fn process_lua_commands() {
    reap_children();
    let tx = state::get();
    let runtime = match tx.lua_runtime.get() {
        Some(r) => r,
        None => return,
    };

    let cmds = runtime.drain_commands();
    if cmds.is_empty() { return; }

    for cmd in cmds {
        match cmd {
            tuxinjector_lua::TuxinjectorCommand::SwitchMode(name) => {
                tracing::debug!(mode = %name, "Lua: switch_mode");
                if let Some(lock) = tx.overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        overlay.switch_mode(&name);
                    }
                }
                tuxinjector_lua::update_mode_name(&name);
                apply_mode_sensitivity(&name, &tx.config);
            }
            tuxinjector_lua::TuxinjectorCommand::ToggleMode { main, fallback } => {
                tracing::debug!(main = %main, fallback = %fallback, "Lua: toggle_mode");
                let mut target = String::new();
                if let Some(lock) = tx.overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        let current = overlay.effective_mode_id();
                        target = if current == main.as_str() { fallback.clone() } else { main.clone() };
                        tracing::debug!(
                            effective = %current,
                            target = %target,
                            "toggle_mode: resolved via effective_mode_id"
                        );
                        overlay.switch_mode(&target);
                    }
                }
                if !target.is_empty() {
                    tuxinjector_lua::update_mode_name(&target);
                    apply_mode_sensitivity(&target, &tx.config);
                }
            }
            tuxinjector_lua::TuxinjectorCommand::ToggleGui => {
                tracing::debug!("Lua: toggle_gui");
                if let Some(lock) = tx.overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        overlay.toggle_gui();
                    }
                }
            }
            tuxinjector_lua::TuxinjectorCommand::SetSensitivity(s) => {
                tracing::debug!(sensitivity = s, "Lua: set_sensitivity");
                tuxinjector_input::set_mode_sensitivity(s, None);
            }
            tuxinjector_lua::TuxinjectorCommand::Exec(cmd_str) => {
                let name = cmd_str.split_whitespace()
                    .next()
                    .and_then(|s| s.rsplit('/').next())
                    .unwrap_or("exec")
                    .to_string();

                let mut cmd = std::process::Command::new("sh");
                cmd.arg("-c")
                    .arg(&cmd_str)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(target_os = "linux")]
                {
                    use std::os::unix::process::CommandExt;
                    cmd.env("GDK_BACKEND", "x11")
                        .env("_JAVA_AWT_WM_NONREPARENTING", "1");
                    // auto-kill child when game exits
                    unsafe {
                        cmd.pre_exec(|| {
                            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                            Ok(())
                        });
                    }
                }
                match cmd.spawn()
                {
                    Ok(child) => {
                        let pid = child.id();
                        tuxinjector_gui::running_apps::register(
                            pid,
                            &name,
                            tuxinjector_gui::running_apps::LaunchMode::Anchored(
                                tuxinjector_gui::running_apps::Anchor::TopRight,
                            ),
                        );
                        tracing::info!(pid, name = %name, cmd = %cmd_str, "exec: spawned (anchored top-right)");
                        if let Ok(mut guard) = EXEC_CHILDREN.lock() {
                            guard.push((child, name));
                        }
                    }
                    Err(e) => {
                        tracing::error!(cmd = %cmd_str, %e, "exec: spawn failed");
                    }
                }
            }
            tuxinjector_lua::TuxinjectorCommand::ToggleAppVisibility => {
                tracing::debug!("Lua: toggle_app_visibility");
                if let Some(lock) = tx.overlay.get() {
                    if let Ok(mut overlay) = lock.lock() {
                        overlay.toggle_app_visibility();
                    }
                }
            }
            tuxinjector_lua::TuxinjectorCommand::PressKey(key) => {
                tracing::debug!(key, "Lua: press_key");
                unsafe { tuxinjector_input::press_key_to_game(key); }
            }
            tuxinjector_lua::TuxinjectorCommand::Log(msg) => {
                tracing::info!(target: "lua", "{msg}");
            }
        }
    }
}

// apply per-mode sensitivity override after switching modes
fn apply_mode_sensitivity(mode_id: &str, config: &std::sync::Arc<tuxinjector_config::ConfigSnapshot>) {
    let cfg = config.load();
    if let Some(mode) = cfg.modes.iter().find(|m| m.id == mode_id) {
        if mode.sensitivity_override_enabled {
            let sep = if mode.separate_xy_sensitivity {
                Some((mode.mode_sensitivity_x, mode.mode_sensitivity_y))
            } else {
                None
            };
            tuxinjector_input::set_mode_sensitivity(mode.mode_sensitivity, sep);
        } else {
            tuxinjector_input::clear_mode_sensitivity();
        }
    }
}

// --- GL constants ---

const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_READ_FRAMEBUFFER: u32 = 0x8CA8;
const GL_DRAW_FRAMEBUFFER: u32 = 0x8CA9;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_RGBA8: u32 = 0x8058;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;
const GL_NEAREST: u32 = 0x2600;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_SCISSOR_TEST: u32 = 0x0C11;
const GL_BACK: u32 = 0x0405;
const GL_DRAW_BUFFER: u32 = 0x0C01;
const GL_READ_BUFFER: u32 = 0x0C02;

const GL_FRAMEBUFFER_BINDING: u32 = 0x8CA6;
const GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE: u32 = 0x8CD0;
const GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME: u32 = 0x8CD1;
const GL_TEXTURE: u32 = 0x1702;
const GL_TEXTURE_WIDTH: u32 = 0x1000;
const GL_TEXTURE_HEIGHT: u32 = 0x1001;

// --- game FBO scanning ---
// looks for Sodium's texture-backed FBO by probing IDs 1-64

type GlGetFbAttachParamFn = unsafe extern "C" fn(u32, u32, u32, *mut i32);
type GlGetTexLevelParamFn = unsafe extern "C" fn(u32, i32, u32, *mut i32);
type GlIsFramebufferFn = unsafe extern "C" fn(u32) -> u8;

static GLFB_ATTACH: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
static GLTEX_LEVEL: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
static GL_IS_FB: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

unsafe fn resolve_once<F: Copy>(lock: &std::sync::OnceLock<usize>, name: &[u8]) -> Option<F> {
    let p = *lock.get_or_init(|| crate::dlsym_hook::resolve_real_symbol(name) as usize);
    if p == 0 { return None; }
    Some(std::mem::transmute_copy(&p))
}

/// Find an FBO whose color attachment matches `mode_w x mode_h`.
pub unsafe fn find_game_fbo(
    gl: &crate::gl_resolve::GlFunctions,
    mode_w: u32,
    mode_h: u32,
) -> u32 {
    find_game_fbo_and_texture(gl, mode_w, mode_h).0
}

/// Same as find_game_fbo but also returns the texture ID.
pub unsafe fn find_game_fbo_and_texture(
    gl: &crate::gl_resolve::GlFunctions,
    mode_w: u32,
    mode_h: u32,
) -> (u32, u32) {
    let Some(get_fb_attach) = resolve_once::<GlGetFbAttachParamFn>(&GLFB_ATTACH, b"glGetFramebufferAttachmentParameteriv\0") else {
        return (0, 0);
    };
    let Some(get_tex_level) = resolve_once::<GlGetTexLevelParamFn>(&GLTEX_LEVEL, b"glGetTexLevelParameteriv\0") else {
        return (0, 0);
    };
    let Some(is_fb) = resolve_once::<GlIsFramebufferFn>(&GL_IS_FB, b"glIsFramebuffer\0") else {
        return (0, 0);
    };

    let mut prev_fbo = 0i32;
    (gl.get_integer_v)(GL_FRAMEBUFFER_BINDING, &mut prev_fbo);

    let mut found = (0u32, 0u32);
    for id in 1..=64u32 {
        if is_fb(id) == 0 { continue; }

        (gl.bind_framebuffer)(GL_FRAMEBUFFER, id);
        if (gl.check_framebuffer_status)(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE {
            continue;
        }

        let mut obj_type = 0i32;
        get_fb_attach(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_TYPE, &mut obj_type);
        if obj_type as u32 != GL_TEXTURE { continue; }

        let mut tex = 0i32;
        get_fb_attach(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME, &mut tex);
        if tex <= 0 { continue; }

        // check the texture dimensions
        (gl.bind_texture)(GL_TEXTURE_2D, tex as u32);
        let mut tw = 0i32;
        let mut th = 0i32;
        get_tex_level(GL_TEXTURE_2D, 0, GL_TEXTURE_WIDTH, &mut tw);
        get_tex_level(GL_TEXTURE_2D, 0, GL_TEXTURE_HEIGHT, &mut th);
        (gl.bind_texture)(GL_TEXTURE_2D, 0);

        if tw as u32 == mode_w && th as u32 == mode_h {
            found = (id, tex as u32);
            tracing::debug!(fbo = id, tex, tw, th, "find_game_fbo: match");
            break;
        }
    }

    (gl.bind_framebuffer)(GL_FRAMEBUFFER, prev_fbo as u32);
    found
}

// --- centering game content in oversized/undersized modes ---

// When the game renders at a different resolution than the physical window,
// blit the game content to the correct centered position.
unsafe fn center_game_content(
    gl: &crate::gl_resolve::GlFunctions,
    mode_w: u32,
    mode_h: u32,
    orig_w: u32,
    orig_h: u32,
) {
    static LOG_ONCE: std::sync::Once = std::sync::Once::new();
    LOG_ONCE.call_once(|| {
        tracing::info!(mode_w, mode_h, orig_w, orig_h,
            vfb_active = crate::virtual_fb::is_active(),
            vfb_fbo = crate::virtual_fb::virtual_fbo(),
            "center_game_content: first call");
    });

    // figure out src/dst offsets per axis
    let (src_x, src_y, dst_x, dst_y, bw, bh);

    if mode_w <= orig_w {
        src_x = 0;
        dst_x = (orig_w as i32 - mode_w as i32) / 2;
        bw = mode_w as i32;
    } else {
        src_x = (mode_w as i32 - orig_w as i32) / 2;
        dst_x = 0;
        bw = orig_w as i32;
    }

    if mode_h <= orig_h {
        src_y = 0;
        dst_y = (orig_h as i32 - mode_h as i32) / 2;
        bh = mode_h as i32;
    } else {
        src_y = (mode_h as i32 - orig_h as i32) / 2;
        dst_y = 0;
        bh = orig_h as i32;
    }

    // nothing to do if it's already perfectly aligned
    if src_x == 0 && src_y == 0 && dst_x == 0 && dst_y == 0 {
        return;
    }

    static CENTER_FBO: AtomicU32 = AtomicU32::new(0);
    static CENTER_TEX: AtomicU32 = AtomicU32::new(0);
    static CENTER_TEX_W: AtomicU32 = AtomicU32::new(0);
    static CENTER_TEX_H: AtomicU32 = AtomicU32::new(0);

    let mut fbo = CENTER_FBO.load(Ordering::Relaxed);
    let mut tex = CENTER_TEX.load(Ordering::Relaxed);

    if fbo == 0 {
        let mut ids = [0u32; 1];
        (gl.gen_framebuffers)(1, ids.as_mut_ptr());
        fbo = ids[0];
        CENTER_FBO.store(fbo, Ordering::Relaxed);

        (gl.gen_textures)(1, ids.as_mut_ptr());
        tex = ids[0];
        CENTER_TEX.store(tex, Ordering::Relaxed);
    }

    // resize the temp texture if needed
    let prev_w = CENTER_TEX_W.load(Ordering::Relaxed);
    let prev_h = CENTER_TEX_H.load(Ordering::Relaxed);
    if prev_w != bw as u32 || prev_h != bh as u32 {
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.tex_image_2d)(
            GL_TEXTURE_2D, 0, GL_RGBA8 as i32,
            bw, bh, 0, GL_RGBA, GL_UNSIGNED_BYTE, std::ptr::null(),
        );
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as i32);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as i32);
        (gl.bind_texture)(GL_TEXTURE_2D, 0);

        (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
        (gl.framebuffer_texture_2d)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        let status = (gl.check_framebuffer_status)(GL_FRAMEBUFFER);
        (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);

        if status != GL_FRAMEBUFFER_COMPLETE {
            tracing::error!(status, "center_game_content: FBO incomplete");
            return;
        }
        CENTER_TEX_W.store(bw as u32, Ordering::Relaxed);
        CENTER_TEX_H.store(bh as u32, Ordering::Relaxed);
        tracing::debug!(bw, bh, fbo, tex, "center_game_content: FBO/tex allocated");
    }

    let mut prev_draw = 0i32;
    let mut prev_read = 0i32;
    (gl.get_integer_v)(GL_DRAW_BUFFER, &mut prev_draw);
    (gl.get_integer_v)(GL_READ_BUFFER, &mut prev_read);

    // for oversized modes, read from the game's internal FBO first
    // (Sodium creates one matching mode dimensions), then fall back to
    // virtual_fb or FBO 0
    let read_fbo = if mode_h > orig_h || mode_w > orig_w {
        let game = unsafe { find_game_fbo(gl, mode_w, mode_h) };
        if game != 0 {
            tracing::debug!(game, "center_game_content: reading from game FBO");
            game
        } else if crate::virtual_fb::is_active() {
            let vfb = crate::virtual_fb::virtual_fbo();
            tracing::debug!(vfb, "center_game_content: reading from virtual FBO");
            if vfb != 0 { vfb } else { 0 }
        } else {
            tracing::debug!("center_game_content: reading from FBO 0");
            0
        }
    } else {
        0
    };

    // step 1: copy game pixels -> temp FBO
    (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, read_fbo);
    if read_fbo == 0 {
        (gl.read_buffer)(GL_BACK);
    } else {
        (gl.read_buffer)(GL_COLOR_ATTACHMENT0);
    }
    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, fbo);
    (gl.draw_buffer)(GL_COLOR_ATTACHMENT0);
    (gl.blit_framebuffer)(
        src_x, src_y, src_x + bw, src_y + bh,
        0, 0, bw, bh,
        GL_COLOR_BUFFER_BIT, GL_NEAREST,
    );

    // step 2: clear back buffer fully
    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
    (gl.disable)(GL_SCISSOR_TEST);
    (gl.clear_color)(0.0, 0.0, 0.0, 1.0);
    (gl.viewport)(0, 0, orig_w as i32, orig_h as i32);
    (gl.clear)(GL_COLOR_BUFFER_BIT);

    // step 3: blit temp -> centered position in back buffer
    (gl.bind_framebuffer)(GL_READ_FRAMEBUFFER, fbo);
    (gl.bind_framebuffer)(GL_DRAW_FRAMEBUFFER, 0);
    (gl.read_buffer)(GL_COLOR_ATTACHMENT0);
    (gl.draw_buffer)(GL_BACK);
    (gl.blit_framebuffer)(
        0, 0, bw, bh,
        dst_x, dst_y, dst_x + bw, dst_y + bh,
        GL_COLOR_BUFFER_BIT, GL_NEAREST,
    );

    (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);
    if prev_draw != 0 { (gl.draw_buffer)(prev_draw as u32); }
    if prev_read != 0 { (gl.read_buffer)(prev_read as u32); }

    tracing::debug!(src_x, src_y, dst_x, dst_y, bw, bh, "center_game_content: done");
}

// macOS: black out border areas around the centered viewport (can't resize
// the window in fullscreen so stale pixels linger otherwise)
#[cfg(target_os = "macos")]
unsafe fn clear_mode_borders(
    gl: &crate::gl_resolve::GlFunctions,
    mode_w: u32,
    mode_h: u32,
    orig_w: u32,
    orig_h: u32,
) {
    let cx = (orig_w as i32 - mode_w as i32) / 2;
    let cy = (orig_h as i32 - mode_h as i32) / 2;
    let ow = orig_w as i32;
    let oh = orig_h as i32;
    let mw = mode_w as i32;
    let mh = mode_h as i32;

    // mode fills the window, nothing to do
    if cx <= 0 && cy <= 0 { return; }

    (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);
    (gl.viewport)(0, 0, ow, oh);
    (gl.clear_color)(0.0, 0.0, 0.0, 1.0);
    (gl.enable)(GL_SCISSOR_TEST);
    (gl.color_mask)(1, 1, 1, 1);

    // left border
    if cx > 0 {
        (gl.scissor)(0, 0, cx, oh);
        (gl.clear)(GL_COLOR_BUFFER_BIT);
    }
    // right border
    if cx > 0 {
        (gl.scissor)(cx + mw, 0, ow - cx - mw, oh);
        (gl.clear)(GL_COLOR_BUFFER_BIT);
    }
    // bottom border (between left/right)
    if cy > 0 {
        (gl.scissor)(cx, 0, mw, cy);
        (gl.clear)(GL_COLOR_BUFFER_BIT);
    }
    // top border (between left/right)
    if cy > 0 {
        (gl.scissor)(cx, cy + mh, mw, oh - cy - mh);
        (gl.clear)(GL_COLOR_BUFFER_BIT);
    }

    (gl.disable)(GL_SCISSOR_TEST);
}

// tracks whether the overlay had anything to draw last frame.
// when false AND no state changes pending, we skip the entire overlay pipeline.
static SCENE_ACTIVE: AtomicBool = AtomicBool::new(true);

/// Set by overlay.rs after build_scene — true if there were elements, gui visible, or apps running.
pub fn set_scene_active(active: bool) {
    SCENE_ACTIVE.store(active, Ordering::Relaxed);
}

// main per-frame render path
unsafe fn render_overlay() {
    if !INITIALIZED.load(Ordering::Acquire) { return; }

    // fast path: if scene was empty last frame, gui is hidden, AND there
    // are no companion apps registered, skip the entire overlay pipeline.
    // (companion apps need render_and_composite → app_capture.embed to run
    // so they get discovered, reparented, and embedded — otherwise they
    // never appear until the user opens the gui.)
    let gui_vis = tuxinjector_input::gui_is_visible();
    let has_apps = tuxinjector_gui::running_apps::registered_count() > 0;
    if !SCENE_ACTIVE.load(Ordering::Relaxed) && !gui_vis && !has_apps {
        // still need to process lua commands and poll borderless toggle
        // (these can change state that makes the scene active next frame)
        crate::viewport_hook::poll_borderless_toggle();
        process_lua_commands();
        capture_original_size();
        return;
    }

    capture_original_size();
    crate::viewport_hook::poll_borderless_toggle();
    process_lua_commands();

    let tx = state::get();

    let (w, h) = {
        let (ow, oh) = crate::viewport_hook::get_original_size();
        if ow > 0 && oh > 0 { (ow, oh) } else { return; }
    };

    if let Some(lock) = tx.overlay.get() {
        if let Ok(mut overlay) = lock.lock() {
            if let Some(gl) = tx.gl.get() {
                // bypass virtual FBO so bind_framebuffer(0) hits the real backbuffer
                let _fb_guard = crate::virtual_fb::fb_bypass_guard();

                let (mw, mh) = crate::viewport_hook::get_mode_size();

                if mw > 0 && mh > 0 && (mw != w || mh != h) {
                    let oversized = crate::viewport_hook::is_oversized(mw, mh, w, h);
                    if oversized || !crate::viewport_hook::is_gl_viewport_hooked() {
                        center_game_content(gl, mw, mh, w, h);
                    }

                    #[cfg(target_os = "macos")]
                    if !oversized && crate::viewport_hook::is_gl_viewport_hooked() {
                        clear_mode_borders(gl, mw, mh, w, h);
                    }
                }

                (gl.viewport)(0, 0, w as i32, h as i32);
                if let Err(e) = overlay.render_and_composite(w, h) {
                    tracing::error!("overlay render failed: {e}");
                }
            } else if let Err(e) = overlay.render_and_composite(w, h) {
                tracing::error!("overlay render failed: {e}");
            }

            // keep Lua's active_res() in sync
            let (rw, rh) = crate::viewport_hook::get_mode_size();
            if rw > 0 && rh > 0 {
                tuxinjector_lua::update_active_res(rw, rh);
            } else {
                tuxinjector_lua::update_active_res(w, h);
            }
        }
    }
}

// --- setters (called from dlsym_hook) ---

// -- Linux: stash real EGL/GLX swap ptrs and try to resolve the originals --

#[cfg(target_os = "linux")]
pub fn store_real_egl_swap(ptr: *mut c_void) {
    REAL_EGL_SWAP.store(ptr, Ordering::Release);
    resolve_original_egl_swap();
}

#[cfg(target_os = "linux")]
pub fn store_real_glx_swap(ptr: *mut c_void) {
    REAL_GLX_SWAP.store(ptr, Ordering::Release);
    resolve_original_glx_swap();
}

#[cfg(target_os = "linux")]
fn resolve_original_egl_swap() {
    const LIBS: &[&[u8]] = &[b"libEGL.so.1\0", b"libEGL.so\0"];

    for lib in LIBS {
        let handle = unsafe {
            libc::dlopen(lib.as_ptr() as *const _, libc::RTLD_NOLOAD | libc::RTLD_LAZY)
        };
        if handle.is_null() { continue; }

        let ptr = crate::dlsym_hook::resolve_real_symbol_from(handle, b"eglSwapBuffers\0");
        unsafe { libc::dlclose(handle) };

        if !ptr.is_null() {
            ORIGINAL_EGL_SWAP.store(ptr, Ordering::Release);
            tracing::debug!("resolved original eglSwapBuffers from {:?}", std::str::from_utf8(&lib[..lib.len()-1]));
            return;
        }
    }
    tracing::debug!("couldn't resolve original eglSwapBuffers from lib, using RTLD_NEXT pointer");
}

#[cfg(target_os = "linux")]
fn resolve_original_glx_swap() {
    const LIBS: &[&[u8]] = &[
        b"libGLX.so.0\0",
        b"libGLX.so\0",
        b"libGL.so.1\0",
        b"libGL.so\0",
    ];

    for lib in LIBS {
        let handle = unsafe {
            libc::dlopen(lib.as_ptr() as *const _, libc::RTLD_NOLOAD | libc::RTLD_LAZY)
        };
        if handle.is_null() { continue; }

        let ptr = crate::dlsym_hook::resolve_real_symbol_from(handle, b"glXSwapBuffers\0");
        unsafe { libc::dlclose(handle) };

        if !ptr.is_null() {
            ORIGINAL_GLX_SWAP.store(ptr, Ordering::Release);
            tracing::debug!("resolved original glXSwapBuffers from {:?}", std::str::from_utf8(&lib[..lib.len()-1]));
            return;
        }
    }
    tracing::debug!("couldn't resolve original glXSwapBuffers from lib, using RTLD_NEXT pointer");
}

// -- macOS: stash the real CGL ptr --

#[cfg(target_os = "macos")]
pub fn store_real_cgl_flush(ptr: *mut c_void) {
    REAL_CGL_FLUSH.store(ptr, Ordering::Release);
    tracing::info!("stored real CGLFlushDrawable");
}

// pick driver-direct ptr (skips other hooks) or RTLD_NEXT (goes through
// the chain). macOS doesn't have LD_PRELOAD chaining so this is Linux only.
#[cfg(target_os = "linux")]
fn select_swap_ptr(
    rtld_next: &AtomicPtr<c_void>,
    original: &AtomicPtr<c_void>,
) -> *mut c_void {
    let skip_chain = if INITIALIZED.load(Ordering::Acquire) {
        let cfg = state::get().config.load();
        cfg.advanced.disable_hook_chaining
            || cfg.advanced.hook_chaining_next_target
                == tuxinjector_config::types::HookChainingNextTarget::OriginalFunction
    } else {
        true
    };

    if skip_chain {
        let orig = original.load(Ordering::Acquire);
        if !orig.is_null() { return orig; }
    }

    rtld_next.load(Ordering::Acquire)
}

// --- hooked swap fns ---

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn hooked_egl_swap_buffers(
    display: *mut c_void,
    surface: *mut c_void,
) -> i32 {
    first_frame_init();

    let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    if frame % LOG_INTERVAL == 0 {
        tracing::debug!(frame, "eglSwapBuffers");
    }

    if INITIALIZED.load(Ordering::Acquire) {
        let cfg = state::get().config.load();
        let fps = effective_fps(cfg.display.fps_limit);
        frame_limit(fps, cfg.display.fps_limit_sleep_threshold);
        if let Some(ps) = state::get().perf_stats.get() {
            ps.record_frame();
        }
    }

    render_overlay();

    let ptr = select_swap_ptr(&REAL_EGL_SWAP, &ORIGINAL_EGL_SWAP);
    if ptr.is_null() {
        tracing::error!("hooked_egl_swap_buffers: real pointer is null!");
        return 0;
    }

    let real: EglSwapBuffersFn = std::mem::transmute(ptr);
    real(display, surface)
}

#[cfg(target_os = "linux")]
#[no_mangle]
pub unsafe extern "C" fn hooked_glx_swap_buffers(display: *mut c_void, drawable: u64) {
    first_frame_init();

    let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    if frame % LOG_INTERVAL == 0 {
        tracing::debug!(frame, "glXSwapBuffers");
    }

    if INITIALIZED.load(Ordering::Acquire) {
        let cfg = state::get().config.load();
        let fps = effective_fps(cfg.display.fps_limit);
        frame_limit(fps, cfg.display.fps_limit_sleep_threshold);
        if let Some(ps) = state::get().perf_stats.get() {
            ps.record_frame();
        }
    }

    render_overlay();

    let ptr = select_swap_ptr(&REAL_GLX_SWAP, &ORIGINAL_GLX_SWAP);
    if ptr.is_null() {
        tracing::error!("hooked_glx_swap_buffers: real pointer is null!");
        return;
    }

    let real: GlxSwapBuffersFn = std::mem::transmute(ptr);
    real(display, drawable);
}

// -- glfwSwapBuffers hook (macOS) --
// dlsym interpose stashes the real ptr; PLT export is a fallback

#[cfg(target_os = "macos")]
static REAL_GLFW_SWAP_BUFFERS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

#[cfg(target_os = "macos")]
type GlfwSwapBuffersFn = unsafe extern "C" fn(window: *mut c_void);

#[cfg(target_os = "macos")]
pub fn store_real_glfw_swap(ptr: *mut c_void) {
    REAL_GLFW_SWAP_BUFFERS.store(ptr, Ordering::Release);
    tracing::info!("stored real glfwSwapBuffers");
}

// primary hook -- what LWJGL actually calls after our dlsym interpose
#[cfg(target_os = "macos")]
pub unsafe extern "C" fn hooked_glfw_swap_buffers(window: *mut c_void) {
    first_frame_init();

    let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    if frame % LOG_INTERVAL == 0 {
        tracing::debug!(frame, "glfwSwapBuffers");
    }

    if INITIALIZED.load(Ordering::Acquire) {
        let cfg = state::get().config.load();
        let fps = effective_fps(cfg.display.fps_limit);
        frame_limit(fps, cfg.display.fps_limit_sleep_threshold);
        if let Some(ps) = state::get().perf_stats.get() {
            ps.record_frame();
        }
    }

    render_overlay();

    let ptr = REAL_GLFW_SWAP_BUFFERS.load(Ordering::Acquire);
    if ptr.is_null() {
        tracing::error!("hooked_glfw_swap_buffers: real pointer is null!");
        return;
    }
    let real: GlfwSwapBuffersFn = std::mem::transmute(ptr);
    real(window)
}

// PLT fallback -- if something calls us through flat namespace directly
#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn glfwSwapBuffers(window: *mut c_void) {
    // prefer the stored ptr (from dlsym interpose), fall back to RTLD_NEXT
    let mut ptr = REAL_GLFW_SWAP_BUFFERS.load(Ordering::Acquire);
    if ptr.is_null() {
        ptr = libc::dlsym(
            libc::RTLD_NEXT,
            b"glfwSwapBuffers\0".as_ptr() as *const i8,
        );
        if !ptr.is_null() {
            REAL_GLFW_SWAP_BUFFERS.store(ptr, Ordering::Release);
        }
    }
    hooked_glfw_swap_buffers(window)
}

// -- macOS CGL fallback (only if CGLFlushDrawable gets resolved via dlsym) --

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn hooked_cgl_flush(ctx: *mut c_void) -> i32 {
    first_frame_init();

    let frame = FRAME_COUNT.fetch_add(1, Ordering::Relaxed);
    if frame % LOG_INTERVAL == 0 {
        tracing::debug!(frame, "CGLFlushDrawable");
    }

    if INITIALIZED.load(Ordering::Acquire) {
        let cfg = state::get().config.load();
        let fps = effective_fps(cfg.display.fps_limit);
        frame_limit(fps, cfg.display.fps_limit_sleep_threshold);
        if let Some(ps) = state::get().perf_stats.get() {
            ps.record_frame();
        }
    }

    render_overlay();

    let ptr = REAL_CGL_FLUSH.load(Ordering::Acquire);
    if ptr.is_null() {
        tracing::error!("hooked_cgl_flush: real pointer is null!");
        return -1;
    }

    let real: CglFlushDrawableFn = std::mem::transmute(ptr);
    real(ctx)
}
