// Plugin management tab -- download, install, configure .so plugins
//
// I'd rather personally add plugins to this page, but
// if you really want to submit a pr, go ahead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

struct PluginDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    github_owner: &'static str,
    github_repo: &'static str,
    asset_name: &'static str,
}

const PLUGINS: &[PluginDef] = &[
    PluginDef {
        id: "twitch-chat",
        name: "Twitch Chat",
        description: "Animated Twitch chat overlay rendered directly in-game.",
        github_owner: "flammablebunny",
        github_repo: "tuxinjector-plugin-twitch-chat",
        asset_name: "tuxinjector-twitchchat-plugin.so",
    },
];

#[derive(Debug)]
enum InstallStatus {
    NotInstalled,
    Installed { version: String },
    Downloading,
    Error(String),
}

impl Default for InstallStatus {
    fn default() -> Self { Self::NotInstalled }
}

struct DlResult {
    idx: usize,
    result: Result<String, String>,
}

// sent to the host plugin registry
#[derive(Debug, Clone)]
pub enum PluginAction {
    SetEnabled { name: String, enabled: bool },
    Reload,
}

// read-only snapshot of what's currently loaded
#[derive(Debug, Clone, Default)]
pub struct PluginSummary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub settings_schema: String,
}

pub struct PluginsState {
    statuses: Vec<InstallStatus>,
    dl_tx: mpsc::SyncSender<DlResult>,
    dl_rx: mpsc::Receiver<DlResult>,
    pub loaded_plugins: Vec<PluginSummary>,
    pub actions: Vec<PluginAction>,
    plugin_settings: HashMap<String, HashMap<String, String>>,
    settings_open: Option<String>,
}

impl Default for PluginsState {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(8);
        let n = PLUGINS.len();
        let settings = load_all_settings();
        let mut s = Self {
            statuses: (0..n).map(|_| InstallStatus::default()).collect(),
            dl_tx: tx,
            dl_rx: rx,
            loaded_plugins: Vec::new(),
            actions: Vec::new(),
            plugin_settings: settings,
            settings_open: None,
        };
        for (i, plugin) in PLUGINS.iter().enumerate() {
            s.statuses[i] = probe_installed(plugin.id);
        }
        s
    }
}

fn plugins_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".local").join("share")
        });
    base.join("tuxinjector").join("plugins")
}

fn so_path(id: &str) -> PathBuf { plugins_dir().join(format!("{id}.so")) }
fn ver_path(id: &str) -> PathBuf { plugins_dir().join(format!("{id}.version")) }

fn probe_installed(id: &str) -> InstallStatus {
    let so = so_path(id);
    if so.exists() {
        let ver = std::fs::read_to_string(ver_path(id))
            .unwrap_or_else(|_| "unknown".into())
            .trim()
            .to_string();
        InstallStatus::Installed { version: ver }
    } else {
        InstallStatus::NotInstalled
    }
}

fn fetch_latest_asset(owner: &str, repo: &str, asset: &str) -> Result<(String, String), String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let out = std::process::Command::new("curl")
        .args([
            "-s", "-L",
            "-H", "Accept: application/vnd.github+json",
            "-H", "User-Agent: tuxinjector",
            &url,
        ])
        .output()
        .map_err(|e| format!("curl: {e}"))?;

    if !out.status.success() {
        return Err(format!("curl exited {}", out.status));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("JSON: {e}"))?;

    if let Some(msg) = json["message"].as_str() {
        return Err(format!("GitHub: {msg}"));
    }

    let tag = json["tag_name"]
        .as_str()
        .ok_or("missing tag_name")?
        .to_string();

    let assets = json["assets"].as_array().ok_or("missing assets array")?;
    for a in assets {
        let name = a["name"].as_str().unwrap_or("");
        if name == asset || name.ends_with(".so") {
            let dl = a["browser_download_url"]
                .as_str()
                .ok_or("missing browser_download_url")?
                .to_string();
            return Ok((tag, dl));
        }
    }

    Err(format!("no '{asset}' asset found in latest release"))
}

fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let status = std::process::Command::new("curl")
        .args(["-s", "-L", "-o", dest.to_str().unwrap_or(""), url])
        .status()
        .map_err(|e| format!("curl download: {e}"))?;
    if !status.success() {
        return Err(format!("download failed (curl exited {status})"));
    }
    Ok(())
}

fn install_plugin(
    idx: usize,
    owner: &str,
    repo: &str,
    id: &str,
    asset: &str,
    tx: mpsc::SyncSender<DlResult>,
) {
    let owner = owner.to_owned();
    let repo = repo.to_owned();
    let id = id.to_owned();
    let asset = asset.to_owned();
    std::thread::spawn(move || {
        let result = (|| {
            let (ver, url) = fetch_latest_asset(&owner, &repo, &asset)?;
            let dest = so_path(&id);
            download_file(&url, &dest)?;
            let _ = std::fs::write(ver_path(&id), &ver);
            Ok(ver)
        })();
        let _ = tx.send(DlResult { idx, result });
    });
}

fn settings_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
            PathBuf::from(home).join(".config")
        });
    base.join("tuxinjector").join("plugins.json")
}

fn load_all_settings() -> HashMap<String, HashMap<String, String>> {
    let path = settings_path();
    if !path.exists() { return HashMap::new(); }

    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };
    let obj = match json.as_object() {
        Some(o) => o,
        None => return HashMap::new(),
    };

    let mut out = HashMap::new();
    for (name, val) in obj {
        if let Some(settings) = val.get("settings").and_then(|s| s.as_object()) {
            let mut map = HashMap::new();
            for (k, v) in settings {
                map.insert(k.clone(), v.as_str().unwrap_or("").to_string());
            }
            out.insert(name.clone(), map);
        }
    }
    out
}

fn save_setting(plugin_id: &str, key: &str, value: &str) {
    let path = settings_path();
    let mut json: serde_json::Value = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let entry = json
        .as_object_mut()
        .unwrap()
        .entry(plugin_id)
        .or_insert(serde_json::json!({"enabled": true, "settings": {}}));

    if entry.get("settings").is_none() {
        entry.as_object_mut().unwrap().insert(
            "settings".to_string(),
            serde_json::json!({}),
        );
    }

    entry["settings"][key] = serde_json::Value::String(value.to_string());

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap_or_default());
}

fn uninstall_plugin(id: &str) -> Result<(), String> {
    let so = so_path(id);
    if so.exists() {
        std::fs::remove_file(&so).map_err(|e| format!("remove .so: {e}"))?;
    }
    let _ = std::fs::remove_file(ver_path(id));
    Ok(())
}

pub fn render(ui: &imgui::Ui, state: &mut PluginsState) {
    state.actions.clear();

    // drain completed downloads
    while let Ok(res) = state.dl_rx.try_recv() {
        match res.result {
            Ok(ver) => {
                let name = PLUGINS[res.idx].name;
                super::super::toast::push(format!("{name} installed ({ver})"));
                state.statuses[res.idx] = InstallStatus::Installed { version: ver };
                state.actions.push(PluginAction::Reload);
            }
            Err(e) => {
                let name = PLUGINS[res.idx].name;
                super::super::toast::push_colored(
                    format!("{name} install failed"),
                    [220, 80, 80, 255],
                );
                state.statuses[res.idx] = InstallStatus::Error(e);
            }
        }
    }

    ui.separator(); ui.text("Plugins");
    #[cfg(target_os = "macos")]
    let plugins_help = "Download and manage overlay plugins. \
         Plugins are stored in ~/.config/tuxinjector/plugins/.";
    #[cfg(target_os = "linux")]
    let plugins_help = "Download and manage overlay plugins. \
         Plugins are stored in ~/.local/share/tuxinjector/plugins/.";
    ui.text(plugins_help);
    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.dummy([0.0, 4.0]);

    for (i, plugin) in PLUGINS.iter().enumerate() {
        let loaded = state.loaded_plugins.iter().find(|p| p.name == plugin.id);

        ui.group(|| {
            ui.text(plugin.name);
            ui.text_disabled(plugin.description);
            ui.text_disabled(format!(
                "github.com/{}/{}",
                plugin.github_owner, plugin.github_repo
            ));

            match &state.statuses[i] {
                InstallStatus::NotInstalled => {
                    ui.text_colored([0.5, 0.5, 0.5, 1.0], "Not installed");
                    ui.same_line();
                    if ui.button(format!("Install##plugin_{i}")) {
                        state.statuses[i] = InstallStatus::Downloading;
                        install_plugin(
                            i, plugin.github_owner, plugin.github_repo,
                            plugin.id, plugin.asset_name, state.dl_tx.clone(),
                        );
                    }
                }

                InstallStatus::Downloading => {
                    ui.text("Downloading...");
                }

                InstallStatus::Installed { version } => {
                    let ver = version.clone();
                    let v = if ver.starts_with('v') { ver.to_string() } else { format!("v{ver}") };
                    ui.text_colored([0.5, 0.5, 0.5, 1.0], v);

                    // enable/disable from host
                    if let Some(info) = loaded {
                        ui.same_line();
                        let mut on = info.enabled;
                        if ui.checkbox(format!("##plugin_en_{i}"), &mut on) {
                            state.actions.push(PluginAction::SetEnabled {
                                name: plugin.id.to_string(),
                                enabled: on,
                            });
                        }
                        if info.enabled {
                            ui.same_line();
                            ui.text_colored(
                                [80.0 / 255.0, 200.0 / 255.0, 120.0 / 255.0, 1.0],
                                "Active",
                            );
                        }
                    } else {
                        ui.same_line();
                        ui.text_colored(
                            [200.0 / 255.0, 180.0 / 255.0, 60.0 / 255.0, 1.0],
                            "Restart to load",
                        );
                    }

                    // configure / update / uninstall buttons
                    if ui.button(format!("Configure##plugin_cfg_{i}")) {
                        let open = state.settings_open.as_deref() == Some(plugin.id);
                        state.settings_open = if open { None } else { Some(plugin.id.to_string()) };
                    }
                    ui.same_line();
                    if ui.button(format!("Update##plugin_upd_{i}")) {
                        state.statuses[i] = InstallStatus::Downloading;
                        install_plugin(
                            i, plugin.github_owner, plugin.github_repo,
                            plugin.id, plugin.asset_name, state.dl_tx.clone(),
                        );
                    }
                    ui.same_line();
                    if ui.button(format!("Uninstall##plugin_rm_{i}")) {
                        match uninstall_plugin(plugin.id) {
                            Ok(()) => {
                                state.statuses[i] = InstallStatus::NotInstalled;
                                super::super::toast::push(format!("{} uninstalled", plugin.name));
                                state.actions.push(PluginAction::Reload);
                            }
                            Err(e) => {
                                state.statuses[i] = InstallStatus::Error(e);
                            }
                        }
                    }
                }

                InstallStatus::Error(e) => {
                    let err = e.clone();
                    crate::widgets::text_wrapped_colored(
                        ui,
                        [220.0 / 255.0, 80.0 / 255.0, 80.0 / 255.0, 1.0],
                        &format!("Error: {err}"),
                    );
                    ui.same_line();
                    if ui.button(format!("Retry##plugin_{i}")) {
                        state.statuses[i] = InstallStatus::Downloading;
                        install_plugin(
                            i, plugin.github_owner, plugin.github_repo,
                            plugin.id, plugin.asset_name, state.dl_tx.clone(),
                        );
                    }
                }
            }

            // inline settings panel
            if matches!(&state.statuses[i], InstallStatus::Installed { .. }) {
                let is_open = state.settings_open.as_deref() == Some(plugin.id);
                if is_open {
                    ui.separator();
                    ui.indent();

                    let settings = state
                        .plugin_settings
                        .entry(plugin.id.to_string())
                        .or_default();

                    let schema = loaded
                        .map(|l| &l.settings_schema)
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());

                    if let Some(schema) = schema {
                        if let Some(obj) = schema.as_object() {
                            for (key, def) in obj {
                                let label = def["label"].as_str().unwrap_or(key);
                                let default_val = def["default"].as_str().unwrap_or("");
                                let field_type = def["type"].as_str().unwrap_or("string");

                                ui.text(format!("{label}:"));
                                ui.same_line();

                                let cur = settings
                                    .entry(key.clone())
                                    .or_insert_with(|| {
                                        match &def["default"] {
                                            serde_json::Value::String(s) => s.clone(),
                                            serde_json::Value::Number(n) => n.to_string(),
                                            serde_json::Value::Bool(b) => b.to_string(),
                                            _ => default_val.to_string(),
                                        }
                                    });

                                let changed = match field_type {
                                    "bool" => {
                                        let mut val = cur == "true";
                                        let r = ui.checkbox(
                                            format!("##setting_{}_{}", plugin.id, key),
                                            &mut val,
                                        );
                                        if r { *cur = val.to_string(); }
                                        r
                                    }
                                    "select" => {
                                        let opts: Vec<String> = def["options"]
                                            .as_array()
                                            .map(|arr| {
                                                arr.iter()
                                                    .filter_map(|v| v.as_str().map(String::from))
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                        let mut did_change = false;
                                        if let Some(_tok) = ui.begin_combo(
                                            format!("##{}_{}", plugin.id, key),
                                            cur.as_str(),
                                        ) {
                                            for opt in &opts {
                                                if ui.selectable_config(opt)
                                                    .selected(cur == opt)
                                                    .build()
                                                {
                                                    *cur = opt.clone();
                                                    did_change = true;
                                                }
                                            }
                                        }
                                        did_change
                                    }
                                    _ => {
                                        ui.set_next_item_width(200.0);
                                        ui.input_text(
                                            format!("##{}_{}", plugin.id, key),
                                            cur,
                                        )
                                        .build()
                                    }
                                };

                                if changed {
                                    save_setting(plugin.id, key, cur);
                                }
                            }

                            ui.dummy([0.0, 4.0]);
                            ui.text_disabled("Changes take effect after restart.");
                        }
                    } else {
                        ui.text("No configurable settings.");
                    }

                    ui.unindent();
                }
            }
        });

        ui.dummy([0.0, 4.0]);
    }

    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.dummy([0.0, 4.0]);
    ui.text_disabled(
        "Plugins are .so shared libraries built with the tuxinjector-plugin-api crate. \
         Newly installed plugins require a game restart to activate!",
    );
}
