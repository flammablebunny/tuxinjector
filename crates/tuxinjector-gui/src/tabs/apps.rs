// Companion app management tab
// Downloads, launches, and manages JAR apps via GitHub Releases
//
// TODO: Add sandrone once its done ^_^ (or replace nbb with it)

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use super::super::running_apps::{Anchor, LaunchMode};

struct AppDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    github_owner: &'static str,
    github_repo: &'static str,
    launch: LaunchStyle,
}

enum LaunchStyle {
    Anchored,
    Headless { nogui_flag: &'static str },
}

const APPS: &[AppDef] = &[
    AppDef {
        id: "ninjabrainbot",
        name: "NinjaBrainBot",
        description: "Accurate stronghold calculator for Minecraft speedrunning.",
        github_owner: "Ninjabrain1",
        github_repo: "Ninjabrain-Bot",
        launch: LaunchStyle::Anchored,
    },
    AppDef {
        id: "paceman",
        name: "Paceman",
        description: "Standalone application or Julti plugin to track and upload runs to PaceMan.gg.",
        github_owner: "PaceMan-MCSR",
        github_repo: "PaceMan-Tracker",
        launch: LaunchStyle::Headless {
            nogui_flag: "--nogui",
        },
    },
];

#[derive(Debug)]
enum InstallStatus {
    NotInstalled,
    Installed { version: String, jar_path: PathBuf },
    Downloading,
    Error(String),
}

impl Default for InstallStatus {
    fn default() -> Self {
        Self::NotInstalled
    }
}

struct DlResult {
    idx: usize,
    result: Result<(String, PathBuf), String>,
}

pub struct AppsState {
    statuses: Vec<InstallStatus>,
    procs: Vec<Option<std::process::Child>>,
    anchors: Vec<Anchor>,
    autostart: Vec<bool>,
    autostart_done: bool,
    dl_tx: mpsc::SyncSender<DlResult>,
    dl_rx: mpsc::Receiver<DlResult>,
}

impl Default for AppsState {
    fn default() -> Self {
        let (tx, rx) = mpsc::sync_channel(8);
        let n = APPS.len();
        let mut s = Self {
            statuses: (0..n).map(|_| InstallStatus::default()).collect(),
            procs: (0..n).map(|_| None).collect(),
            anchors: vec![Anchor::TopRight; n],
            autostart: vec![false; n],
            autostart_done: false,
            dl_tx: tx,
            dl_rx: rx,
        };

        // probe what's already installed
        for (i, app) in APPS.iter().enumerate() {
            s.statuses[i] = probe_installed(app.id);
            if let Some(anchor) = load_autostart(app.id) {
                s.autostart[i] = true;
                s.anchors[i] = anchor;
            }
        }

        // auto-launch anything marked for startup
        for (i, app) in APPS.iter().enumerate() {
            if !s.autostart[i] { continue; }
            if let InstallStatus::Installed { jar_path, .. } = &s.statuses[i] {
                let jar = jar_path.clone();
                let (extra, mode): (&[&str], LaunchMode) = match &app.launch {
                    LaunchStyle::Anchored => (&[], LaunchMode::Anchored(s.anchors[i])),
                    LaunchStyle::Headless { nogui_flag } => {
                        (&[nogui_flag.as_ref()], LaunchMode::Anchored(s.anchors[i]))
                    }
                };
                do_launch(&jar, app.name, extra, mode, &mut s.procs[i]);
            }
        }
        s.autostart_done = true;

        s
    }
}

fn apps_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    #[cfg(target_os = "macos")]
    let base = ".config/tuxinjector/apps";
    #[cfg(target_os = "linux")]
    let base = ".local/share/tuxinjector/apps";
    PathBuf::from(home).join(base)
}

fn app_jar_path(id: &str) -> PathBuf { apps_dir().join(format!("{id}.jar")) }
fn app_ver_path(id: &str) -> PathBuf { apps_dir().join(format!("{id}.version")) }
fn autostart_path(id: &str) -> PathBuf { apps_dir().join(format!("{id}.autostart")) }

fn load_autostart(id: &str) -> Option<Anchor> {
    let content = std::fs::read_to_string(autostart_path(id)).ok()?;
    parse_anchor(content.trim())
}

fn save_autostart(id: &str, anchor: Anchor) {
    let _ = std::fs::create_dir_all(apps_dir());
    let _ = std::fs::write(autostart_path(id), anchor.label());
}

fn clear_autostart(id: &str) {
    let _ = std::fs::remove_file(autostart_path(id));
}

fn parse_anchor(s: &str) -> Option<Anchor> {
    for &a in Anchor::ALL {
        if a.label().eq_ignore_ascii_case(s) {
            return Some(a);
        }
    }
    None
}

fn probe_installed(id: &str) -> InstallStatus {
    let jar = app_jar_path(id);
    if jar.exists() {
        let ver = std::fs::read_to_string(app_ver_path(id))
            .unwrap_or_else(|_| "unknown".into())
            .trim()
            .to_string();
        InstallStatus::Installed { version: ver, jar_path: jar }
    } else {
        InstallStatus::NotInstalled
    }
}

fn fetch_latest_jar(owner: &str, repo: &str) -> Result<(String, String), String> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases/latest");
    let resp = ureq::get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "tuxinjector")
        .call()
        .map_err(|e| format!("HTTP: {e}"))?;

    let json: serde_json::Value = resp.into_json().map_err(|e| format!("JSON: {e}"))?;

    if let Some(msg) = json["message"].as_str() {
        return Err(format!("GitHub: {msg}"));
    }

    let tag = json["tag_name"]
        .as_str()
        .ok_or("missing tag_name in response")?
        .to_string();

    let assets = json["assets"].as_array().ok_or("missing assets array")?;
    for asset in assets {
        let name = asset["name"].as_str().unwrap_or("");
        if name.ends_with(".jar") && !name.ends_with("-sources.jar") {
            let dl = asset["browser_download_url"]
                .as_str()
                .ok_or("missing browser_download_url")?
                .to_string();
            return Ok((tag, dl));
        }
    }

    Err("no .jar asset found in latest release".into())
}

fn download_jar(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("download: {e}"))?;

    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read: {e}"))?;
    std::fs::write(dest, &bytes).map_err(|e| format!("write: {e}"))?;
    Ok(())
}

fn install_app(
    idx: usize,
    owner: &str,
    repo: &str,
    id: &str,
    tx: mpsc::SyncSender<DlResult>,
) {
    let owner = owner.to_owned();
    let repo = repo.to_owned();
    let id = id.to_owned();
    std::thread::spawn(move || {
        let result = (|| {
            let (ver, url) = fetch_latest_jar(&owner, &repo)?;
            let dest = app_jar_path(&id);
            download_jar(&url, &dest)?;
            let _ = std::fs::write(app_ver_path(&id), &ver);
            Ok((ver, dest))
        })();
        let _ = tx.send(DlResult { idx, result });
    });
}

fn uninstall_app(id: &str) -> Result<(), String> {
    let jar = app_jar_path(id);
    if jar.exists() {
        std::fs::remove_file(&jar).map_err(|e| format!("remove jar: {e}"))?;
    }
    let _ = std::fs::remove_file(app_ver_path(id));
    Ok(())
}

// Build LD_LIBRARY_PATH so Java AWT can find X11/GL libs.
// Scrapes /proc/self/maps for /nix/store/*/lib paths (covers what the game loads),
// reads TUXINJECTOR_X11_LIBS for the additional libs companion apps like nbb need
// (set by the NixOS wrapper), and keeps the inherited LD_LIBRARY_PATH.
#[cfg(target_os = "linux")]
fn nix_ld_path() -> String {
    let mut dirs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if let Ok(maps) = std::fs::read_to_string("/proc/self/maps") {
        for line in maps.lines() {
            let Some(path) = line.split_whitespace().last() else { continue };
            if !path.starts_with("/nix/store/") { continue; }
            let Some((dir, _)) = path.rsplit_once('/') else { continue };
            if dir.ends_with("/lib") && seen.insert(dir.to_string()) {
                dirs.push(dir.to_string());
            }
        }
    }

    // X11 libs that companion apps need but the game doesn't load.
    // Set by the NixOS wrapper via makeLibraryPath.
    if let Ok(x11) = std::env::var("TUXINJECTOR_X11_LIBS") {
        for entry in x11.split(':') {
            if !entry.is_empty() && seen.insert(entry.to_string()) {
                dirs.push(entry.to_string());
            }
        }
    }

    // also keep the inherited LD_LIBRARY_PATH
    if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
        for entry in existing.split(':') {
            if !entry.is_empty() && seen.insert(entry.to_string()) {
                dirs.push(entry.to_string());
            }
        }
    }

    dirs.join(":")
}

// Resolve the java binary: prefer the game's own JVM (via /proc/self/exe on Linux),
// fall back to "java" on PATH.
#[cfg(target_os = "linux")]
fn resolve_java() -> String {
    if let Ok(exe) = std::fs::read_link("/proc/self/exe") {
        let s = exe.to_string_lossy().to_string();
        if s.contains("java") || s.contains("jdk") || s.contains("jre") {
            tracing::info!(java = %s, "using game's JVM for companion app");
            return s;
        }
    }
    "java".to_string()
}

#[cfg(target_os = "macos")]
fn resolve_java() -> String {
    // We're injected into Minecraft which IS a Java process.
    // Use the same JVM binary since "java" may not be on PATH.
    if let Ok(exe) = std::env::current_exe() {
        let s = exe.to_string_lossy().to_string();
        if s.contains("java") || s.contains("jdk") || s.contains("jre") {
            tracing::info!(java = %s, "using game's JVM for companion app");
            return s;
        }
    }
    "java".to_string()
}

#[cfg(target_os = "linux")]
fn launch_app(jar: &Path, extra_args: &[&str]) -> Result<std::process::Child, String> {
    let ld = nix_ld_path();
    let java = resolve_java();

    let mut cmd = std::process::Command::new(&java);
    cmd.arg("-Dswing.defaultlaf=javax.swing.plaf.metal.MetalLookAndFeel")
        .arg("-Dawt.useSystemAAFontSettings=on")
        .arg("-jar")
        .arg(jar)
        .args(extra_args)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("LD_PRELOAD")
        .env("_JAVA_AWT_WM_NONREPARENTING", "1")
        .env("TUXINJECTOR_STDIN_KEYS", "1")
        .env("LD_LIBRARY_PATH", &ld)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map_err(|e| format!("java -jar: {e} -- is java installed?"))
}

#[cfg(target_os = "macos")]
fn launch_app(jar: &Path, extra_args: &[&str]) -> Result<std::process::Child, String> {
    let java = resolve_java();

    let mut cmd = std::process::Command::new(&java);
    cmd.arg("-Dswing.defaultlaf=javax.swing.plaf.metal.MetalLookAndFeel")
        .arg("-jar")
        .arg(jar)
        .args(extra_args)
        .env_remove("DYLD_INSERT_LIBRARIES")
        .env("TUXINJECTOR_STDIN_KEYS", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map_err(|e| format!("java -jar: {e} -- is java installed?"))
}

fn do_launch(
    jar: &Path,
    name: &str,
    extra_args: &[&str],
    mode: LaunchMode,
    slot: &mut Option<std::process::Child>,
) {
    match launch_app(jar, extra_args) {
        Ok(mut child) => {
            let pid = child.id();
            if let Some(stdin) = child.stdin.take() {
                super::super::running_apps::register_stdin(pid, stdin);
            }
            super::super::running_apps::register(pid, name, mode);
            let info = match mode {
                LaunchMode::Anchored(a) => format!("launched (anchored {})", a.label()),
                LaunchMode::Detached => "launched (detached)".to_string(),
            };
            super::super::toast::push(format!("{name} {info}"));
            *slot = Some(child);
        }
        Err(e) => {
            super::super::toast::push_colored(format!("{name}: {e}"), [220, 80, 80, 255]);
        }
    }
}

pub fn render(ui: &imgui::Ui, state: &mut AppsState) {
    // drain download results
    while let Ok(res) = state.dl_rx.try_recv() {
        match res.result {
            Ok((ver, jar_path)) => {
                let name = APPS[res.idx].name;
                super::super::toast::push(format!("{name} installed ({ver})"));
                state.statuses[res.idx] = InstallStatus::Installed { version: ver, jar_path };
            }
            Err(e) => {
                let name = APPS[res.idx].name;
                super::super::toast::push_colored(
                    format!("{name} install failed"),
                    [220, 80, 80, 255],
                );
                state.statuses[res.idx] = InstallStatus::Error(e);
            }
        }
    }

    // reap dead child processes
    for (i, child_opt) in state.procs.iter_mut().enumerate() {
        if let Some(child) = child_opt {
            if let Ok(Some(_)) = child.try_wait() {
                super::super::running_apps::unregister(child.id());
                *child_opt = None;
                super::super::toast::push(format!("{} exited", APPS[i].name));
            }
        }
    }

    ui.separator(); ui.text("Companion Apps");
    ui.text(
        "Download and manage speedrun companion tools. \
         Apps are stored in ~/.config/tuxinjector/apps/.",
    );
    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.dummy([0.0, 4.0]);

    for (i, app) in APPS.iter().enumerate() {
        let running = state.procs[i].is_some();

        ui.group(|| {
            ui.text(app.name);
            ui.text_disabled(app.description);
            ui.text_disabled(format!(
                "github.com/{}/{}",
                app.github_owner, app.github_repo
            ));

            match &state.statuses[i] {
                InstallStatus::NotInstalled => {
                    ui.text_colored([0.5, 0.5, 0.5, 1.0], "Not installed");
                    ui.same_line();
                    if ui.button(format!("Install##app_{i}")) {
                        state.statuses[i] = InstallStatus::Downloading;
                        install_app(i, app.github_owner, app.github_repo, app.id, state.dl_tx.clone());
                    }
                }

                InstallStatus::Downloading => {
                    ui.text("Downloading...");
                }

                InstallStatus::Installed { version, jar_path } => {
                    let jar = jar_path.clone();
                    let ver = version.clone();

                    let v = if ver.starts_with('v') { ver.to_string() } else { format!("v{ver}") };
                    ui.text_colored([0.5, 0.5, 0.5, 1.0], v);

                    if running {
                        ui.same_line();
                        ui.text_colored(
                            [80.0 / 255.0, 200.0 / 255.0, 120.0 / 255.0, 1.0],
                            "Running",
                        );
                        ui.same_line();
                        if ui.button(format!("Stop##app_{i}")) {
                            if let Some(child) = &mut state.procs[i] {
                                super::super::running_apps::unregister(child.id());
                                let _ = child.kill();
                            }
                            state.procs[i] = None;
                            super::super::toast::push(format!("{} stopped", app.name));
                        }
                    } else {
                        launch_buttons(ui, app, &jar, i, state);

                        if ui.button(format!("Update##app_{i}")) {
                            state.statuses[i] = InstallStatus::Downloading;
                            install_app(i, app.github_owner, app.github_repo, app.id, state.dl_tx.clone());
                        }
                        ui.same_line();
                        if ui.button(format!("Uninstall##app_{i}")) {
                            match uninstall_app(app.id) {
                                Ok(()) => {
                                    state.statuses[i] = InstallStatus::NotInstalled;
                                    super::super::toast::push(format!("{} uninstalled", app.name));
                                }
                                Err(e) => {
                                    state.statuses[i] = InstallStatus::Error(e);
                                }
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
                    if ui.button(format!("Retry##app_{i}")) {
                        state.statuses[i] = InstallStatus::Downloading;
                        install_app(i, app.github_owner, app.github_repo, app.id, state.dl_tx.clone());
                    }
                }
            }
        });

        ui.dummy([0.0, 4.0]);
    }

    ui.dummy([0.0, 8.0]);
    ui.separator();
    ui.dummy([0.0, 4.0]);
    ui.text_disabled(
        "Launch -- anchored inside the game window (survives fullscreen). \
         Launch Detached / Launch with GUI -- standalone floating window.",
    );
}

fn launch_buttons(
    ui: &imgui::Ui,
    app: &AppDef,
    jar: &Path,
    idx: usize,
    state: &mut AppsState,
) {
    // autostart toggle
    let mut auto = state.autostart[idx];
    if ui.checkbox(format!("Launch on Startup##auto_{}", app.id), &mut auto) {
        state.autostart[idx] = auto;
        if auto {
            save_autostart(app.id, state.anchors[idx]);
        } else {
            clear_autostart(app.id);
        }
    }
    if ui.is_item_hovered() {
        ui.tooltip_text("Launch app automatically on game start");
    }

    match &app.launch {
        LaunchStyle::Anchored => {
            if ui.button(format!("Launch##launch_{}", app.id)) {
                let a = state.anchors[idx];
                do_launch(jar, app.name, &[], LaunchMode::Anchored(a), &mut state.procs[idx]);
            }
            ui.same_line();
            if ui.button(format!("Launch Detached##detach_{}", app.id)) {
                do_launch(jar, app.name, &[], LaunchMode::Detached, &mut state.procs[idx]);
            }
            ui.same_line();

            // anchor picker
            let cur = &mut state.anchors[idx];
            let prev = *cur;
            if let Some(_token) =
                ui.begin_combo(format!("##anchor_{}", app.id), cur.label())
            {
                for &a in Anchor::ALL {
                    if ui.selectable_config(a.label())
                        .selected(*cur == a)
                        .build()
                    {
                        *cur = a;
                    }
                }
            }
            if *cur != prev && state.autostart[idx] {
                save_autostart(app.id, *cur);
            }
        }
        LaunchStyle::Headless { nogui_flag } => {
            if ui.button(format!("Launch##launch_{}", app.id)) {
                do_launch(
                    jar, app.name, &[nogui_flag],
                    LaunchMode::Anchored(Anchor::TopLeft),
                    &mut state.procs[idx],
                );
            }
            ui.same_line();
            if ui.button(format!("Launch with GUI##gui_{}", app.id)) {
                do_launch(jar, app.name, &[], LaunchMode::Detached, &mut state.procs[idx]);
            }
        }
    }
}

impl Drop for AppsState {
    fn drop(&mut self) {
        for child_opt in &mut self.procs {
            if let Some(child) = child_opt {
                super::super::running_apps::unregister(child.id());
                let _ = child.kill();
            }
        }
    }
}
