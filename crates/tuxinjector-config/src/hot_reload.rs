// Config hot-reload via `notify`.
//
// Watches the config file (or the whole config dir for Lua require() support)
// and re-parses on changes, publishing the new snapshot through RCU.
// Supports profile switching: checks active_profile.txt to determine which
// file to re-read on change.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{error, info, warn};

use crate::snapshot::ConfigSnapshot;
use crate::types::Config;

// 100ms debounce - vim/neovim do atomic rename-writes which
// fire multiple events back to back
const DEBOUNCE: Duration = Duration::from_millis(100);

// Settings-diff logger hook. The diff-logging infra lives in the `tuxinjector`
// crate (which depends on us, so we can't reference it directly). The host
// installs a function pointer at startup; we call it at the publish site so
// every hot-reload records exactly which settings changed.
type ConfigDiffLogger = fn(old: &Config, new: &Config);
static DIFF_LOGGER: OnceLock<ConfigDiffLogger> = OnceLock::new();

/// Install the settings-diff logger called whenever a hot-reload publishes a
/// new config. No-op if called more than once.
pub fn set_hot_reload_logger(f: ConfigDiffLogger) {
    let _ = DIFF_LOGGER.set(f);
}

// Takes config source text, returns parsed Config (or error string).
pub type ConfigParser = Box<dyn Fn(&str) -> Result<Config, String> + Send>;

pub struct ConfigWatcher {
    path: PathBuf,
    snap: Arc<ConfigSnapshot>,
    parser: ConfigParser,
    // when true, watch any .lua file in the dir (for require() deps)
    watch_all: bool,
    // NOTE: kept alive so the watcher doesn't get dropped
    _watcher: Option<RecommendedWatcher>,
}

// Figure out which config file we should actually reload. The in-memory
// override wins (set by the GUI mid-session), otherwise fall back to whatever
// active_profile.txt says on disk. Empty / missing => just use init.lua.
fn resolve_active_config_path(config_dir: &std::path::Path, default_path: &std::path::Path) -> PathBuf {
    let name = match crate::profile_override() {
        Some(n) => n,
        None => std::fs::read_to_string(config_dir.join("active_profile.txt"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
    };
    if !name.is_empty() {
        let profile = config_dir.join("profiles").join(format!("{name}.lua"));
        // Named profile might not exist yet (e.g. just deleted), so guard it
        if profile.exists() {
            return profile;
        }
    }
    default_path.to_path_buf()
}

impl ConfigWatcher {
    pub fn new(
        path: PathBuf,
        snap: Arc<ConfigSnapshot>,
        parser: ConfigParser,
    ) -> Result<Self, notify::Error> {
        Ok(Self {
            path,
            snap,
            parser,
            watch_all: false,
            _watcher: None,
        })
    }

    pub fn set_watch_all_files(&mut self, val: bool) {
        self.watch_all = val;
    }

    // Spawn the background watcher thread.
    pub fn start(&mut self) -> Result<(), notify::Error> {
        let path = self.path.clone();
        let snap = Arc::clone(&self.snap);
        let watch_all = self.watch_all;

        // Swap the parser out so we can move it into the thread
        let parser = std::mem::replace(&mut self.parser, Box::new(|_| Ok(Config::default())));

        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
        let mut watcher = notify::recommended_watcher(tx)?;

        // Watch the parent dir so we catch atomic-rename writes
        let watch_dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        // Recursive so we also catch profile file changes in profiles/
        watcher.watch(&watch_dir, RecursiveMode::Recursive)?;

        info!("Config watcher started for {}", path.display());

        let fname = path
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();

        let config_dir = watch_dir.clone();

        std::thread::Builder::new()
            .name("config-watcher".into())
            .spawn(move || {
                let mut last_reload = Instant::now() - DEBOUNCE * 2; // allow immediate first reload

                for evt in rx.iter() {
                    let evt = match evt {
                        Ok(e) => e,
                        Err(e) => {
                            warn!("File watch error: {e}");
                            continue;
                        }
                    };

                    // only care about writes/creates/renames
                    let dominated = matches!(
                        evt.kind,
                        EventKind::Modify(_) | EventKind::Create(_)
                    );
                    if !dominated { continue; }

                    // filter to our file, or any .lua when watching the whole dir
                    if !watch_all {
                        let ours = evt.paths.iter().any(|p| {
                            p.file_name()
                                .map(|n| n == fname)
                                .unwrap_or(false)
                        });
                        if !ours { continue; }
                    } else {
                        let is_lua = evt.paths.iter().any(|p| {
                            p.extension().map(|ext| ext == "lua").unwrap_or(false)
                        });
                        if !is_lua { continue; }
                    }

                    // debounce
                    let now = Instant::now();
                    if now.duration_since(last_reload) < DEBOUNCE {
                        continue;
                    }
                    last_reload = now;

                    // Resolve which file to read: check active_profile.txt so that
                    // after a GUI profile switch, we re-read the correct file.
                    let active_path = resolve_active_config_path(&config_dir, &path);

                    match std::fs::read_to_string(&active_path) {
                        Ok(src) => match parser(&src) {
                            Ok(cfg) => {
                                info!(path = %active_path.display(), "config hot-reloaded");
                                // diff old vs new BEFORE publishing so we log
                                // exactly which settings the reload changed
                                if let Some(log) = DIFF_LOGGER.get() {
                                    let old = (**snap.load()).clone();
                                    log(&old, &cfg);
                                }
                                snap.publish(cfg);
                            }
                            Err(e) => error!("Failed to parse config: {e}"),
                        },
                        Err(e) => error!("Failed to read config file {}: {e}", active_path.display()),
                    }
                }

                info!("Config watcher thread exiting");
            })
            .map_err(|e| notify::Error::generic(&e.to_string()))?;

        self._watcher = Some(watcher);
        Ok(())
    }
}
