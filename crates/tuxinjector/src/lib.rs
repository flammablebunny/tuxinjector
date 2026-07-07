// tuxinjector - minecraft speedrunning overlay
// hooks into dlsym to grab EGL/GLX swap + GLFW input via LD_PRELOAD

#[cfg(target_os = "linux")]
mod app_capture;
#[cfg(target_os = "macos")]
#[path = "app_capture_macos.rs"]
mod app_capture;
#[cfg(target_os = "linux")]
mod companion_clipboard;
mod browser_capture;
mod config_init;
mod cursor_image;
mod cursor_system;
mod cursor_trail;
mod dlsym_hook;
mod overlay_gen;
mod lua_writer;
mod gl_resolve;
mod glfw_hook;
mod gui_renderer;
mod input_handler;
mod mirror_capture;
mod nbb_client;
mod nbb_data;
mod nbb_format;
mod nbb_overlay;
pub mod mode_system;
mod overlay;
mod perf_stats;
mod plugin_loader;
mod plugin_registry;
mod render_thread;
mod state;
mod state_watcher;
mod swap_hook;
mod text_rasterizer;
mod tux_log;
#[allow(dead_code)]
mod virtual_fb;
mod viewport_hook;
#[cfg(target_os = "linux")]
mod wayland_hook;
mod window_state;

// liblogger loader: see crates/tuxinjector/src/liblogger.rs for details.
// To disable liblogger, delete the `liblogger::load();` line in the ctor below.
#[allow(dead_code)]
mod liblogger;

use std::sync::OnceLock;
use std::sync::Mutex;

use ctor::{ctor, dtor};
use tracing_subscriber::EnvFilter;

// needs to outlive everything or hot-reload breaks
static CONFIG_WATCHER: Mutex<Option<tuxinjector_config::ConfigWatcher>> = Mutex::new(None);

// macOS uses ~/.config/tuxinjector for everything; Linux uses ~/.local/share/tuxinjector for data
pub(crate) fn data_subpath() -> &'static str {
    #[cfg(target_os = "macos")]
    { ".config/tuxinjector" }
    #[cfg(target_os = "linux")]
    { ".local/share/tuxinjector" }
}

// handle for swapping the tracing filter at runtime from the debug tab
static LOG_FILTER_UPDATER: OnceLock<Box<dyn Fn(EnvFilter) + Send + Sync>> = OnceLock::new();

// Rebuild the STDERR tracing filter based on which debug checkboxes are ticked.
// Each checkbox gates debug logging for a specific subsystem. Baseline is warn+
// so routine info logging stays in the dedicated file log (see tux_log) instead
// of bleeding into the game's stderr / the Minecraft launcher log; the file
// layer has its own fixed info filter and is unaffected by this.
pub(crate) fn apply_log_filter(cfg: &tuxinjector_config::Config) {
    let Some(update) = LOG_FILTER_UPDATER.get() else { return };

    let d = &cfg.advanced.debug;

    let mut parts: Vec<&str> = vec![
        "tuxinjector=warn",
        "tuxinjector_config=warn",
        "tuxinjector_gl_interop=warn",
        "tuxinjector_render=warn",
        // config code is always surfaced on stderr (Minecraft log)
        "tuxinjector::config_code=info",
    ];

    if d.log_mode_switch {
        parts.push("tuxinjector::overlay=debug");
        parts.push("tuxinjector::swap_hook=debug");
        parts.push("tuxinjector::viewport_hook=debug");
    }
    if d.log_animation {
        parts.push("tuxinjector::mode_system=debug");
    }
    if d.log_hotkey {
        parts.push("tuxinjector::input_handler=debug");
    }
    if d.log_window_overlay {
        parts.push("tuxinjector::app_capture=debug");
        parts.push("tuxinjector::window_state=debug");
    }
    if d.log_file_monitor {
        parts.push("tuxinjector_config=debug");
        parts.push("tuxinjector::config_init=debug");
    }
    if d.log_image_monitor {
        parts.push("tuxinjector_render=debug");
    }
    if d.log_performance {
        parts.push("tuxinjector::perf_stats=debug");
    }
    if d.log_texture_ops {
        parts.push("tuxinjector_gl_interop=debug");
    }
    if d.log_gui {
        parts.push("tuxinjector::gui_renderer=debug");
    }
    if d.log_init {
        parts.push("tuxinjector::config_init=debug");
        parts.push("tuxinjector::dlsym_hook=debug");
        parts.push("tuxinjector::gl_resolve=debug");
    }
    if d.log_overlay {
        parts.push("tuxinjector::overlay=debug");
        parts.push("tuxinjector::cursor_system=debug");
    }
    let combined = parts.join(",");
    match EnvFilter::try_new(&combined) {
        Ok(f) => update(f),
        Err(e) => tracing::warn!("apply_log_filter: bad directive: {e}"),
    }
}


#[ctor]
fn init() {
    // Bring up the dedicated support log + tracing subscriber FIRST, before
    // liblogger, so liblogger's own initialisation is captured in latest.log.
    // tux_log owns the reloadable stderr filter; it hands back the reload
    // closure that apply_log_filter() uses to hot-swap debug levels at runtime.
    let updater = tux_log::init();
    LOG_FILTER_UPDATER.get_or_init(|| updater);

    /// Load liblogger before anything else.
    /// To fully disable liblogger, delete this folowing single line. (Doing so makes any
    /// subsequent run / match ILLEGAL on speedrun.com / MCSR Ranked.)
    liblogger::load();
    tracing::info!("liblogger initialised");

    // clear the preload env var so child processes don't get us injected too
    unsafe {
        #[cfg(target_os = "linux")]
        libc::unsetenv(b"LD_PRELOAD\0".as_ptr() as *const libc::c_char);
        #[cfg(target_os = "macos")]
        libc::unsetenv(b"DYLD_INSERT_LIBRARIES\0".as_ptr() as *const libc::c_char);
    }

    #[cfg(target_os = "linux")]
    tracing::info!("tuxinjector: loaded via LD_PRELOAD");
    #[cfg(target_os = "macos")]
    tracing::info!("tuxinjector: loaded via DYLD_INSERT_LIBRARIES");

    let watcher = config_init::init_config();
    if let Ok(mut guard) = CONFIG_WATCHER.lock() {
        *guard = watcher;
    }
    tracing::info!("config loaded");

    // watches wpstateout.txt for game state changes
    state_watcher::spawn_state_watcher();
    tracing::info!("state watcher spawned");

    // background-check GitHub for a newer tuxinjector build. Staged on disk and
    // applied on next launch (or via the GUI's "Restart Now") -- never forced.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        tuxinjector_gui::spawn_update_check(env!("CARGO_PKG_VERSION"));
        tracing::info!("update check spawned");
    }

    tracing::info!("tuxinjector init complete");
}

#[dtor]
fn on_unload() {
    // Force-terminate: our background threads (config-watcher, state-watcher,
    // perf-stats, lua runtime) block on I/O or sleep loops. Rust statics
    // don't drop on exit, so these threads never get a stop signal.
    // _exit() kills all threads immediately.

    // Rotate the live latest.log to a timestamped archive before we tear down.
    // Safe to call once; nothing writes the log after this.
    tux_log::rotate_on_shutdown();

    #[cfg(target_os = "linux")]
    unsafe { libc::_exit(0); }
}
