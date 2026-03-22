// Browser overlay capture manager.
//
// The tuxinjector-browser helper binary is embedded in the .so via include_bytes
// and extracted to a temp file at runtime (same pattern as liblogger). Each
// browser overlay gets its own helper process.

use std::collections::HashMap;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;

use crossbeam_channel::{Receiver, Sender};
use tuxinjector_config::types::BrowserOverlayConfig;

// embedded at compile time - build tuxinjector-browser first
#[cfg(target_arch = "x86_64")]
const BROWSER_BIN: &[u8] = include_bytes!("../../../assets/tuxinjector-browser_x64");
#[cfg(target_arch = "aarch64")]
const BROWSER_BIN: &[u8] = include_bytes!("../../../assets/tuxinjector-browser_aarch64");
// 32-bit targets probably won't have webkit2gtk, but keep stubs so it compiles
#[cfg(target_arch = "x86")]
const BROWSER_BIN: &[u8] = &[];
#[cfg(target_arch = "arm")]
const BROWSER_BIN: &[u8] = &[];

// extract once, reuse the path
static HELPER_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

fn extract_helper() -> Option<PathBuf> {
    HELPER_PATH.get_or_init(|| {
        if BROWSER_BIN.is_empty() {
            tracing::warn!("browser helper not available for this architecture");
            return None;
        }

        let dir = std::env::temp_dir().join(format!("tuxinjector-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tuxinjector-browser");

        if let Err(e) = std::fs::write(&path, BROWSER_BIN) {
            tracing::warn!(%e, "failed to extract browser helper");
            return None;
        }

        // make executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
        }

        tracing::info!(path = %path.display(), "browser helper extracted");
        Some(path)
    }).clone()
}

pub struct CapturedFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

struct BrowserSession {
    child: Child,
    frame_rx: Receiver<CapturedFrame>,
    stdin_tx: Option<BufWriter<std::process::ChildStdin>>,
    latest: Option<CapturedFrame>,
    url: String,
    css: String,
    width: i32,
    height: i32,
    fps: i32,
}

pub struct BrowserCaptureManager {
    sessions: HashMap<String, BrowserSession>,
}

impl BrowserCaptureManager {
    pub fn new() -> Self {
        Self { sessions: HashMap::new() }
    }

    pub fn sync_sessions(&mut self, configs: &[BrowserOverlayConfig]) {
        // remove sessions for configs that no longer exist
        let active: Vec<String> = configs.iter().enumerate()
            .filter(|(_, c)| !c.url.is_empty())
            .map(|(i, c)| if c.name.is_empty() { format!("__browser_{i}") } else { c.name.clone() })
            .collect();
        let stale: Vec<String> = self.sessions.keys()
            .filter(|k| !active.contains(k))
            .cloned()
            .collect();
        for name in stale {
            self.stop_session(&name);
        }

        for (i, cfg) in configs.iter().enumerate() {
            if cfg.url.is_empty() { continue; }
            let key = if cfg.name.is_empty() { format!("__browser_{i}") } else { cfg.name.clone() };

            if let Some(session) = self.sessions.get_mut(&key) {
                // push config changes
                if session.url != cfg.url {
                    send_cmd(session, &format!(r#"{{"cmd":"navigate","url":"{}"}}"#,
                        cfg.url.replace('\\', "\\\\").replace('"', "\\\"")));
                    session.url = cfg.url.clone();
                }
                if session.css != cfg.custom_css {
                    send_cmd(session, &format!(r#"{{"cmd":"inject_css","css":"{}"}}"#,
                        cfg.custom_css.replace('\\', "\\\\").replace('"', "\\\"")));
                    session.css = cfg.custom_css.clone();
                }
                if session.width != cfg.width || session.height != cfg.height {
                    send_cmd(session, &format!(r#"{{"cmd":"resize","width":{},"height":{}}}"#,
                        cfg.width, cfg.height));
                    session.width = cfg.width;
                    session.height = cfg.height;
                }
                if session.fps != cfg.fps {
                    send_cmd(session, &format!(r#"{{"cmd":"set_fps","fps":{}}}"#, cfg.fps));
                    session.fps = cfg.fps;
                }

                while let Ok(frame) = session.frame_rx.try_recv() {
                    session.latest = Some(frame);
                }
            } else {
                self.start_session(&key, cfg);
            }
        }
    }

    pub fn latest_frame(&self, name: &str) -> Option<&CapturedFrame> {
        self.sessions.get(name).and_then(|s| s.latest.as_ref())
    }

    fn start_session(&mut self, key: &str, cfg: &BrowserOverlayConfig) {
        let path = match extract_helper() {
            Some(p) => p,
            None => return,
        };

        let mut cmd = Command::new(&path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("LD_PRELOAD")
            .env_remove("DYLD_INSERT_LIBRARIES");

        // NixOS: make sure glib-networking is in GIO_EXTRA_MODULES for TLS.
        // The env might already have gvfs/dconf modules but not the TLS one.
        if let Some(ref tls_path) = find_nix_gio_modules() {
            let existing = std::env::var("GIO_EXTRA_MODULES").unwrap_or_default();
            if !existing.contains("glib-networking") {
                let val = if existing.is_empty() {
                    tls_path.clone()
                } else {
                    format!("{existing}:{tls_path}")
                };
                tracing::info!(path = %val, "setting GIO_EXTRA_MODULES for browser helper");
                cmd.env("GIO_EXTRA_MODULES", val);
            }
        }

        let mut child = match cmd.spawn()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(name = %key, %e, "failed to spawn browser helper");
                return;
            }
        };

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdin = child.stdin.take().unwrap();

        let (tx, rx) = crossbeam_channel::bounded::<CapturedFrame>(2);

        let key_clone = key.to_string();
        std::thread::Builder::new()
            .name(format!("browser-{key}"))
            .spawn(move || read_frames(stdout, tx, &key_clone))
            .ok();

        let key_clone2 = key.to_string();
        std::thread::Builder::new()
            .name(format!("browser-err-{key}"))
            .spawn(move || drain_stderr(stderr, &key_clone2))
            .ok();

        let mut session = BrowserSession {
            child,
            frame_rx: rx,
            stdin_tx: Some(BufWriter::new(stdin)),
            latest: None,
            url: String::new(),
            css: String::new(),
            width: cfg.width,
            height: cfg.height,
            fps: cfg.fps,
        };

        send_cmd(&mut session, &format!(r#"{{"cmd":"resize","width":{},"height":{}}}"#,
            cfg.width, cfg.height));
        send_cmd(&mut session, &format!(r#"{{"cmd":"set_fps","fps":{}}}"#, cfg.fps));
        if !cfg.custom_css.is_empty() {
            send_cmd(&mut session, &format!(r#"{{"cmd":"inject_css","css":"{}"}}"#,
                cfg.custom_css.replace('\\', "\\\\").replace('"', "\\\"")));
            session.css = cfg.custom_css.clone();
        }
        send_cmd(&mut session, &format!(r#"{{"cmd":"navigate","url":"{}"}}"#,
            cfg.url.replace('\\', "\\\\").replace('"', "\\\"")));
        session.url = cfg.url.clone();

        tracing::info!(name = %key, url = %cfg.url, "browser overlay started");
        self.sessions.insert(key.to_string(), session);
    }

    fn stop_session(&mut self, name: &str) {
        if let Some(mut session) = self.sessions.remove(name) {
            send_cmd(&mut session, r#"{"cmd":"close"}"#);
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = session.child.kill();
            tracing::info!(name, "browser overlay stopped");
        }
    }
}

fn send_cmd(session: &mut BrowserSession, json: &str) {
    if let Some(ref mut stdin) = session.stdin_tx {
        if writeln!(stdin, "{json}").is_err() || stdin.flush().is_err() {
            session.stdin_tx = None;
        }
    }
}

fn read_frames(mut stdout: std::process::ChildStdout, tx: Sender<CapturedFrame>, name: &str) {
    let mut header = [0u8; 12];
    let mut count = 0u64;
    loop {
        if stdout.read_exact(&mut header).is_err() {
            tracing::warn!(name, frames = count, "browser frame reader: stdout closed");
            break;
        }
        let w = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let h = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        let len = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);

        if len == 0 || len > 100_000_000 { continue; }

        let mut pixels = vec![0u8; len as usize];
        if stdout.read_exact(&mut pixels).is_err() {
            tracing::warn!(name, "browser frame reader: partial frame read failed");
            break;
        }

        count += 1;
        if count == 1 {
            tracing::info!(name, w, h, len, "browser: first frame received");
        }

        let _ = tx.try_send(CapturedFrame { pixels, width: w, height: h });
    }
}

fn drain_stderr(stderr: std::process::ChildStderr, name: &str) {
    use std::io::BufRead;
    let reader = std::io::BufReader::new(stderr);
    for line in reader.lines() {
        match line {
            Ok(l) => tracing::warn!(name, "browser-helper: {}", l),
            Err(_) => break,
        }
    }
}

// NixOS: glib-networking isn't in the standard path, so WebKitGTK
// can't do HTTPS. Search common locations for the GIO TLS module.
fn find_nix_gio_modules() -> Option<String> {
    // system profile
    let candidates = [
        "/run/current-system/sw/lib/gio/modules",
    ];
    for path in candidates {
        if std::path::Path::new(path).join("libgiognutls.so").exists() {
            return Some(path.to_string());
        }
    }
    // per-user profiles
    if let Ok(home) = std::env::var("HOME") {
        let user_path = format!("{home}/.nix-profile/lib/gio/modules");
        if std::path::Path::new(&user_path).join("libgiognutls.so").exists() {
            return Some(user_path);
        }
    }
    // last resort: scan /nix/store top-level for glib-networking
    if let Ok(entries) = std::fs::read_dir("/nix/store") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.contains("glib-networking") && !name.contains(".drv") {
                let mod_path = entry.path().join("lib/gio/modules");
                if mod_path.join("libgiognutls.so").exists() {
                    return Some(mod_path.to_string_lossy().to_string());
                }
            }
        }
    }
    None
}
