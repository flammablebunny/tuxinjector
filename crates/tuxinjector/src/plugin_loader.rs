// Plugin discovery and loading.
// Scans the plugins dir for .so/.dylib files that export a `tx_plugin_register` C ABI entry point.
// Linux: ~/.local/share/tuxinjector/plugins/  macOS: ~/.config/tuxinjector/plugins/

use std::collections::HashMap;
use std::ffi::{c_void, CStr};
use std::path::{Path, PathBuf};

use tuxinjector_plugin_api::{
    FrameContext, OverlaySubmission, PluginEvent, PluginInfo, PluginRegisterFn, PluginSetting,
    PluginVtable, API_VERSION, MAX_SUBMISSIONS,
};

pub struct LoadedPlugin {
    _lib: libloading::Library, // must stay alive for the plugin's lifetime
    pub name: String,
    pub version: String,
    pub description: String,
    vtable: PluginVtable,
    data: *mut c_void,
    pub enabled: bool,
    sub_buf: Vec<OverlaySubmission>,  // scratch buffer for on_frame
}

// SAFETY: Plugin data is only ever accessed from the render thread;
// vtable functions are extern "C" and don't touch Rust state.
unsafe impl Send for LoadedPlugin {}

impl LoadedPlugin {
    pub fn init(&mut self, settings: &HashMap<String, String>) -> Result<(), String> {
        // convert to C strings for the FFI boundary
        let c_pairs: Vec<(std::ffi::CString, std::ffi::CString)> = settings
            .iter()
            .map(|(k, v)| {
                (
                    std::ffi::CString::new(k.as_str()).unwrap(),
                    std::ffi::CString::new(v.as_str()).unwrap(),
                )
            })
            .collect();

        let raw: Vec<PluginSetting> = c_pairs
            .iter()
            .map(|(k, v)| PluginSetting {
                key: k.as_ptr(),
                value: v.as_ptr(),
            })
            .collect();

        let rc = (self.vtable.init)(self.data, raw.as_ptr(), raw.len());
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("plugin '{}' init returned {rc}", self.name))
        }
    }

    pub fn on_frame(&mut self, ctx: &FrameContext) -> &[OverlaySubmission] {
        self.sub_buf.resize_with(MAX_SUBMISSIONS, || OverlaySubmission {
            x: 0.0, y: 0.0,
            width: 0, height: 0,
            pixels: std::ptr::null(),
            pixel_len: 0,
            depth: 0,
            opacity: 0.0,
        });

        let mut count: usize = 0;
        let rc = (self.vtable.on_frame)(
            self.data,
            ctx as *const FrameContext,
            self.sub_buf.as_mut_ptr(),
            MAX_SUBMISSIONS,
            &mut count,
        );

        if rc != 0 {
            tracing::warn!(plugin = %self.name, rc, "on_frame error");
            return &[];
        }

        let n = count.min(MAX_SUBMISSIONS);
        &self.sub_buf[..n]
    }

    pub fn on_event(&mut self, event: &PluginEvent) {
        (self.vtable.on_event)(self.data, event as *const PluginEvent);
    }

    pub fn settings_schema(&self) -> String {
        let mut buf = vec![0u8; 8192];
        let mut len: usize = 0;
        let rc = (self.vtable.get_settings_schema)(
            self.data, buf.as_mut_ptr(), buf.len(), &mut len,
        );
        if rc != 0 || len > buf.len() {
            return "{}".to_string();
        }
        String::from_utf8_lossy(&buf[..len]).into_owned()
    }

    pub fn destroy(&mut self) {
        (self.vtable.destroy)(self.data);
        self.data = std::ptr::null_mut();
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        if !self.data.is_null() {
            self.destroy();
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn plugin_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    let base = home_dir().map(|h| h.join(".config"))?;
    #[cfg(target_os = "linux")]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".local").join("share")))?;
    Some(base.join("tuxinjector").join("plugins"))
}

pub fn plugin_settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".config")))?;
    Some(base.join("tuxinjector").join("plugins.json"))
}

pub fn load_plugin_settings() -> HashMap<String, PluginSettings> {
    let Some(path) = plugin_settings_path() else {
        return HashMap::new();
    };
    if !path.exists() {
        return HashMap::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(e) => {
            tracing::warn!(?path, %e, "couldn't read plugins.json");
            HashMap::new()
        }
    }
}

pub fn save_plugin_settings(settings: &HashMap<String, PluginSettings>) {
    let Some(path) = plugin_settings_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(settings) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!(?path, %e, "couldn't write plugins.json");
            }
        }
        Err(e) => tracing::warn!(%e, "couldn't serialize plugin settings"),
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginSettings {
    pub enabled: bool,
    #[serde(default)]
    pub settings: HashMap<String, String>,
}

// Scan the plugin directory and load everything we find
pub fn discover_and_load(saved: &HashMap<String, PluginSettings>) -> Vec<LoadedPlugin> {
    let Some(dir) = plugin_dir() else {
        tracing::info!("plugin_loader: no data dir, skipping discovery");
        return Vec::new();
    };

    if !dir.exists() {
        tracing::info!(?dir, "plugin_loader: dir doesn't exist, nothing to load");
        return Vec::new();
    }

    let mut plugins = Vec::new();

    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(?dir, %e, "plugin_loader: couldn't read dir");
            return Vec::new();
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
        if path.extension().and_then(|e| e.to_str()) != Some(ext) {
            continue;
        }

        match load_single(&path, saved) {
            Ok(p) => {
                tracing::info!(
                    name = %p.name, version = %p.version, enabled = p.enabled,
                    "plugin_loader: loaded"
                );
                plugins.push(p);
            }
            Err(e) => {
                tracing::error!(?path, %e, "plugin_loader: load failed");
            }
        }
    }

    tracing::info!(count = plugins.len(), "plugin_loader: discovery complete");
    plugins
}

fn load_single(path: &Path, saved: &HashMap<String, PluginSettings>) -> Result<LoadedPlugin, String> {
    // SAFETY: we're trusting user-placed .so files here. maybe change this? linux users should
    // be smart enough, but are mac users?
    let lib = unsafe { libloading::Library::new(path) }
        .map_err(|e| format!("dlopen failed: {e}"))?;

    let register_fn: libloading::Symbol<PluginRegisterFn> = unsafe {
        lib.get(b"tx_plugin_register")
    }
    .map_err(|e| format!("missing tx_plugin_register: {e}"))?;

    let mut info = std::mem::MaybeUninit::<PluginInfo>::zeroed();
    let mut vtable = std::mem::MaybeUninit::<PluginVtable>::zeroed();
    let mut plugin_data: *mut c_void = std::ptr::null_mut();

    let rc = register_fn(
        info.as_mut_ptr(),
        vtable.as_mut_ptr(),
        &mut plugin_data,
    );

    if rc != 0 {
        return Err(format!("tx_plugin_register returned {rc}"));
    }

    let info = unsafe { info.assume_init() };
    let vtable = unsafe { vtable.assume_init() };

    if info.api_version != API_VERSION {
        return Err(format!(
            "API version mismatch: plugin={}, host={API_VERSION}",
            info.api_version
        ));
    }

    let name = unsafe { CStr::from_ptr(info.name) }.to_string_lossy().into_owned();
    let version = unsafe { CStr::from_ptr(info.version) }.to_string_lossy().into_owned();
    let desc = unsafe { CStr::from_ptr(info.description) }.to_string_lossy().into_owned();

    // new plugins default to enabled
    let enabled = saved.get(&name).map(|s| s.enabled).unwrap_or(true);

    let mut plugin = LoadedPlugin {
        _lib: lib,
        name: name.clone(),
        version,
        description: desc,
        vtable,
        data: plugin_data,
        enabled,
        sub_buf: Vec::new(),
    };

    // init with whatever settings we have saved
    let init_settings = saved.get(&name).map(|s| s.settings.clone()).unwrap_or_default();
    if let Err(e) = plugin.init(&init_settings) {
        tracing::error!(plugin = %name, %e, "init failed, disabling");
        plugin.enabled = false;
    }

    Ok(plugin)
}
