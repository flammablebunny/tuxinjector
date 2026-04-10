// tuxinjector - minecraft speedrunning overlay
// hooks into dlsym to grab EGL/GLX swap + GLFW input via LD_PRELOAD

#[cfg(target_os = "linux")]
mod app_capture;
#[cfg(target_os = "macos")]
#[path = "app_capture_macos.rs"]
mod app_capture;
mod browser_capture;
mod config_init;
mod dlsym_hook;
mod overlay_gen;
mod lua_writer;
mod gl_resolve;
mod glfw_hook;
mod gui_renderer;
mod input_handler;
mod mirror_capture;
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
#[allow(dead_code)]
mod virtual_fb;
mod viewport_hook;
mod window_state;

// liblogger loader: see crates/tuxinjector/src/liblogger.rs for details.
// To disable liblogger, delete the `liblogger::load();` line in the ctor below.
#[allow(dead_code)]
mod liblogger;

use std::sync::OnceLock;
use std::sync::Mutex;

use ctor::{ctor, dtor};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

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

// Rebuild tracing filter based on which debug checkboxes are ticked.
// Each checkbox gates debug logging for a specific subsystem.
pub(crate) fn apply_log_filter(cfg: &tuxinjector_config::Config) {
    let Some(update) = LOG_FILTER_UPDATER.get() else { return };

    let d = &cfg.advanced.debug;

    let mut parts: Vec<&str> = vec![
        "tuxinjector=info",
        "tuxinjector_config=info",
        "tuxinjector_gl_interop=info",
        "tuxinjector_render=info",
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
    if d.log_cursor_textures {
        parts.push("tuxinjector::overlay=debug");
    }

    let combined = parts.join(",");
    match EnvFilter::try_new(&combined) {
        Ok(f) => update(f),
        Err(e) => tracing::warn!("apply_log_filter: bad directive: {e}"),
    }
}


#[ctor]
fn init() {
    /// Load liblogger before anything else.
    /// To fully disable liblogger, delete this folowing single line. (Doing so makes any
    /// subsequent run / match ILLEGAL on speedrun.com / MCSR Ranked.)
    liblogger::load();

    // clear the preload env var so child processes don't get us injected too
    unsafe {
        #[cfg(target_os = "linux")]
        libc::unsetenv(b"LD_PRELOAD\0".as_ptr() as *const libc::c_char);
        #[cfg(target_os = "macos")]
        libc::unsetenv(b"DYLD_INSERT_LIBRARIES\0".as_ptr() as *const libc::c_char);
    }

    // reloadable filter - we flip debug logging from the GUI at runtime
    let initial = EnvFilter::from_default_env()
        .add_directive("tuxinjector=info".parse().unwrap());

    let (filter_layer, reload_handle) = tracing_subscriber::reload::Layer::new(initial);

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    // stash the reload closure so we can hot-swap filters later
    LOG_FILTER_UPDATER.get_or_init(|| {
        Box::new(move |f: EnvFilter| {
            let _ = reload_handle.reload(f);
        })
    });

    #[cfg(target_os = "linux")]
    tracing::info!("tuxinjector: loaded via LD_PRELOAD");
    #[cfg(target_os = "macos")]
    tracing::info!("tuxinjector: loaded via DYLD_INSERT_LIBRARIES");

    let watcher = config_init::init_config();
    if let Ok(mut guard) = CONFIG_WATCHER.lock() {
        *guard = watcher;
    }

    // watches wpstateout.txt for game state changes
    state_watcher::spawn_state_watcher();
}

#[dtor]
fn on_unload() {
    // Force-terminate: our background threads (config-watcher, state-watcher,
    // perf-stats, lua runtime) block on I/O or sleep loops. Rust statics
    // don't drop on exit, so these threads never get a stop signal.
    // _exit() kills all threads immediately.
    #[cfg(target_os = "linux")]
    unsafe { libc::_exit(0); }
}
