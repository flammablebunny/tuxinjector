// Config loading and hot-reload integration.
// Spawns a LuaRuntime thread for the Lua VM and hotkey callbacks.

use std::path::PathBuf;
use std::sync::Arc;

use tuxinjector_config::ConfigWatcher;

use crate::state;

// bundled default init.lua, written to disk on first run
const DEFAULT_INIT_LUA: &str = include_str!("../../../assets/default.lua");

// look for the config file in the usual places
fn find_config_path() -> Option<PathBuf> {
    // 1. explicit override
    if let Ok(path) = std::env::var("TUXINJECTOR_CONFIG_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            tracing::info!(path = %p.display(), "config from TUXINJECTOR_CONFIG_PATH");
            return Some(p);
        }
        tracing::warn!(path = %p.display(), "TUXINJECTOR_CONFIG_PATH set but file doesn't exist");
    }

    // 2. XDG config home
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let dir = PathBuf::from(xdg).join("tuxinjector");
        if let Some(found) = check_dir(&dir) { return Some(found); }
    }

    // 3. data dir fallback (macOS: ~/.config/tuxinjector, Linux: ~/.local/share/tuxinjector)
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(home).join(crate::data_subpath());
        if let Some(found) = check_dir(&dir) { return Some(found); }
    }

    // 4. cwd as last resort
    check_dir(&PathBuf::from("."))
}

fn check_dir(dir: &PathBuf) -> Option<PathBuf> {
    for name in &["init.lua", "config.lua", "tuxinjector.lua"] {
        let p = dir.join(name);
        if p.exists() {
            tracing::info!(path = %p.display(), "found config");
            return Some(p);
        }
    }
    None
}

fn bindings_to_tuples(bindings: &[tuxinjector_lua::LuaActionBinding]) -> Vec<(Vec<i32>, u64, bool)> {
    bindings.iter()
        .map(|b| (b.key_combo.clone(), b.callback_id, b.block_from_game))
        .collect()
}

// write the default init.lua if nothing exists yet
fn write_default_config() -> Option<PathBuf> {
    let dir = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("tuxinjector")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/tuxinjector")
    } else {
        tracing::warn!("can't determine config dir (HOME/XDG_CONFIG_HOME unset)");
        return None;
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(dir = %dir.display(), error = %e, "failed to create config dir");
        return None;
    }

    let path = dir.join("init.lua");
    if path.exists() {
        return Some(path); // don't overwrite
    }

    // On macOS, rewrite data paths from ~/.local/share/tuxinjector to ~/.config/tuxinjector
    #[cfg(target_os = "macos")]
    let content = DEFAULT_INIT_LUA.replace(
        "~/.local/share/tuxinjector",
        "~/.config/tuxinjector",
    );
    #[cfg(target_os = "linux")]
    let content = DEFAULT_INIT_LUA;

    match std::fs::write(&path, content) {
        Ok(()) => {
            tracing::info!(path = %path.display(), "wrote default init.lua");
            Some(path)
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to write default init.lua");
            None
        }
    }
}


// Load config, spawn Lua runtime, start hot-reload watcher.
pub fn init_config() -> Option<ConfigWatcher> {
    let tx = state::init_or_get();
    let snapshot = Arc::clone(&tx.config);

    let path = match find_config_path() {
        Some(p) => p,
        None => {
            // nothing on disk, write the default and use it
            match write_default_config() {
                Some(p) => p,
                None => {
                    tracing::info!("no config found, couldn't write default -- using built-in defaults");
                    return None;
                }
            }
        }
    };

    if let Some(dir) = path.parent() {
        let _ = tx.config_dir.set(dir.to_path_buf());
    }

    match std::fs::read_to_string(&path) {
        Ok(src) => {
            boot_lua(tx, &snapshot, &path, &src);
        }
        Err(e) => {
            tracing::error!(path = %path.display(), error = %e, "failed to read config, using defaults");
        }
    }

    // watcher uses a Lua-delegating parser for hot-reload
    let parser: tuxinjector_config::ConfigParser = Box::new(|source: &str| {
        let tx = state::get();
        let cfg = if let Some(runtime) = tx.lua_runtime.get() {
            let update = runtime.reload(source.to_string())?;

            *tx.lua_bindings.lock().unwrap() = Some(bindings_to_tuples(&update.bindings));
            tracing::debug!(lua_actions = update.bindings.len(), "Lua reload: updated bindings");

            update.config
        } else {
            tuxinjector_lua::load_lua_config(source).map_err(|e| format!("{e}"))?
        };

        crate::apply_log_filter(&cfg);
        crate::overlay_gen::generate_overlay(&cfg);

        Ok(cfg)
    });

    match ConfigWatcher::new(path.clone(), snapshot, parser) {
        Ok(mut watcher) => {
            // watch all .lua files so require()'d modules also trigger reload
            watcher.set_watch_all_files(true);
            if let Err(e) = watcher.start() {
                tracing::error!(error = %e, "failed to start config watcher");
                return None;
            }
            Some(watcher)
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to create config watcher");
            None
        }
    }
}

// spawn Lua runtime, publish initial config, stash bindings
fn boot_lua(
    tx: &'static state::TuxinjectorState,
    snapshot: &Arc<tuxinjector_config::ConfigSnapshot>,
    path: &PathBuf,
    source: &str,
) {
    match tuxinjector_lua::LuaRuntime::spawn(source.to_string()) {
        Ok((runtime, update)) => {
            let cfg = update.config;

            tracing::info!(
                path = %path.display(),
                modes = cfg.modes.len(),
                mirrors = cfg.overlays.mirrors.len(),
                images = cfg.overlays.images.len(),
                hotkeys = cfg.hotkeys.mode_hotkeys.len(),
                lua_actions = update.bindings.len(),
                "config loaded via Lua runtime"
            );

            crate::apply_log_filter(&cfg);
            crate::overlay_gen::generate_overlay(&cfg);

            snapshot.publish(cfg);

            if !update.bindings.is_empty() {
                *tx.lua_bindings.lock().unwrap() = Some(bindings_to_tuples(&update.bindings));
            }

            let _ = tx.lua_runtime.set(runtime);
        }
        Err(e) => {
            tracing::error!(
                path = %path.display(),
                error = %e,
                "Lua runtime failed to spawn, using defaults"
            );
        }
    }
}
