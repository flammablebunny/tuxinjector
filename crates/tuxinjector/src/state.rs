// Global singleton state shared across all hook modules.
// Most fields are OnceLock because they get init'd at different
// times during startup (config first, GL on first frame, etc).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;

use tuxinjector_config::ConfigSnapshot;

use crate::gl_resolve::GlFunctions;
use crate::overlay::OverlayState;
use crate::perf_stats::PerfStats;
use crate::plugin_registry::PluginRegistry;

// (key_combo, callback_id, block_from_game)
pub type LuaBindings = Vec<(Vec<i32>, u64, bool)>;

static STATE: OnceLock<TuxinjectorState> = OnceLock::new();

pub struct TuxinjectorState {
    pub config: Arc<ConfigSnapshot>,
    pub gl: OnceLock<GlFunctions>,
    pub overlay: OnceLock<std::sync::Mutex<OverlayState>>,

    pub lua_bindings: std::sync::Mutex<Option<LuaBindings>>,
    pub lua_runtime: OnceLock<tuxinjector_lua::LuaRuntime>,
    pub game_state: std::sync::Mutex<String>,
    pub config_dir: OnceLock<PathBuf>,
    pub perf_stats: OnceLock<Arc<PerfStats>>,
    pub plugins: OnceLock<std::sync::Mutex<PluginRegistry>>,
}

impl TuxinjectorState {
    fn new() -> Self {
        Self {
            config: Arc::new(ConfigSnapshot::default()),
            gl: OnceLock::new(),
            overlay: OnceLock::new(),
            lua_bindings: std::sync::Mutex::new(None),
            lua_runtime: OnceLock::new(),
            game_state: std::sync::Mutex::new(String::new()),
            config_dir: OnceLock::new(),
            perf_stats: OnceLock::new(),
            plugins: OnceLock::new(),
        }
    }
}

// Panics if init_or_get() was never called.
pub fn get() -> &'static TuxinjectorState {
    STATE
        .get()
        .expect("tuxinjector: state not initialised -- was init_or_get() never called?")
}

// Idempotent init, safe to call multiple times.
pub fn init_or_get() -> &'static TuxinjectorState {
    STATE.get_or_init(TuxinjectorState::new)
}
