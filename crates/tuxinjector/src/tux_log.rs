// Dedicated tuxinjector support log.
//
// Writes a human-readable session log to <config_dir>/logs/latest.log that
// captures startup, hook installation, mode/setting changes, liblogger init,
// and Rust-side panics (crash capture). Near the top of every session we emit
// a reversible "config share code" -- base64url(minified JSON of Config) --
// that a companion website can decode to display every setting.
//
// Rotation mimics Minecraft / Toolscreen: while running we write latest.log;
// a leftover latest.log from a previous (possibly crashed) session is rotated
// to a timestamped filename on the next startup, and the live log is rotated
// on clean shutdown from the dtor.
//
// NOTE: this module never installs OS signal handlers. The JVM relies on
// SIGSEGV/SIGBUS for its own machinery; intercepting them breaks the game.
// Crash capture is the Rust panic hook + leftover-log rotation, nothing more.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;
use tracing_subscriber::{reload, EnvFilter};

// The live log file handle. Kept alive for the whole process; the fmt layer
// holds its own clone via MakeWriter, this one is for the explicit final flush
// on shutdown and so the file stays open.
static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

// Path of the live latest.log, stashed so rotate_on_shutdown can rename it.
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

// Ensures rotate_on_shutdown does its work exactly once.
static ROTATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// File-layer filter: INFO and above for all of our own crates. This is the
// fixed filter for the support log -- it is intentionally NOT reloadable, so
// the support log always captures lifecycle events regardless of the GUI's
// debug-checkbox state (which only ever drives the stderr layer).
const FILE_LAYER_FILTER: &str = "tuxinjector=info,tuxinjector_config=info,\
tuxinjector_input=info,tuxinjector_lua=info,tuxinjector_gl_interop=info,\
tuxinjector_render=info,tuxinjector_core=info";

// ---------------------------------------------------------------------------
// A MakeWriter that writes to a shared, cloned File handle behind a Mutex.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct FileWriter {
    file: std::sync::Arc<Mutex<File>>,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileWriter {
    type Writer = FileWriterGuard;
    fn make_writer(&'a self) -> Self::Writer {
        FileWriterGuard { file: self.file.clone() }
    }
}

struct FileWriterGuard {
    file: std::sync::Arc<Mutex<File>>,
}

impl Write for FileWriterGuard {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.file.lock() {
            Ok(mut f) => f.write(buf),
            Err(_) => Ok(buf.len()),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.lock() {
            Ok(mut f) => f.flush(),
            Err(_) => Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

/// Initialise the dedicated support log + the global tracing subscriber.
///
/// MUST be called at the very top of the ctor, BEFORE `liblogger::load()`, so
/// liblogger's own initialisation is captured. Returns the reload closure that
/// `apply_log_filter` uses to hot-swap the *stderr* layer's filter at runtime;
/// the caller stashes it in `LOG_FILTER_UPDATER`. If anything fails we still
/// install a stderr-only subscriber so the rest of the program keeps logging.
pub fn init() -> Box<dyn Fn(EnvFilter) + Send + Sync> {
    // Reloadable stderr filter. Default is warn+ so our verbose instrumentation
    // doesn't bleed into the game's stderr (which Prism/MultiMC captures into the
    // Minecraft log) -- the dedicated file log captures info+. RUST_LOG still
    // overrides, and the GUI debug checkboxes raise stderr per-subsystem.
    let initial = EnvFilter::from_default_env()
        .add_directive("tuxinjector=warn".parse().unwrap())
        // ...but always surface the config code on stderr (Minecraft launcher log).
        .add_directive("tuxinjector::config_code=info".parse().unwrap());
    let (reload_filter, reload_handle) = reload::Layer::new(initial);
    let updater: Box<dyn Fn(EnvFilter) + Send + Sync> = Box::new(move |f: EnvFilter| {
        let _ = reload_handle.reload(f);
    });

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_filter(reload_filter);

    // Try to open the file log; on any failure, fall back to stderr-only.
    let logs_dir = match logs_dir() {
        Some(d) => d,
        None => {
            tracing_subscriber::registry().with(stderr_layer).init();
            tracing::warn!("tux_log: could not resolve logs dir, file log disabled");
            return updater;
        }
    };

    if let Err(e) = fs::create_dir_all(&logs_dir) {
        tracing_subscriber::registry().with(stderr_layer).init();
        tracing::warn!(dir = %logs_dir.display(), error = %e, "tux_log: mkdir failed, file log disabled");
        return updater;
    }

    let latest = logs_dir.join("latest.log");

    // Rotate a leftover latest.log (previous session, possibly a crash) to a
    // timestamped name derived from ITS mtime, preserving the crash log.
    rotate_leftover(&latest);

    let file = match OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&latest)
    {
        Ok(f) => f,
        Err(e) => {
            tracing_subscriber::registry().with(stderr_layer).init();
            tracing::warn!(path = %latest.display(), error = %e, "tux_log: open latest.log failed, file log disabled");
            return updater;
        }
    };

    // Keep a process-lifetime clone for explicit flush/rotation on shutdown.
    if let Ok(file2) = file.try_clone() {
        let _ = LOG_FILE.set(Mutex::new(Some(file2)));
    } else {
        let _ = LOG_FILE.set(Mutex::new(None));
    }
    let _ = LOG_PATH.set(latest.clone());

    let shared = std::sync::Arc::new(Mutex::new(file));
    let file_writer = FileWriter { file: shared };

    let file_filter = EnvFilter::try_new(FILE_LAYER_FILTER)
        .unwrap_or_else(|_| EnvFilter::default().add_directive(LevelFilter::INFO.into()));

    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_writer(file_writer)
        .with_filter(file_filter);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();

    install_panic_hook();
    write_banner(&latest);

    updater
}

// ---------------------------------------------------------------------------
// shutdown rotation
// ---------------------------------------------------------------------------

/// Flush + rename latest.log -> <timestamp-now>.log. Called from the dtor
/// before _exit. Idempotent: only the first call does any work.
pub fn rotate_on_shutdown() {
    use std::sync::atomic::Ordering;
    if ROTATED.swap(true, Ordering::SeqCst) {
        return;
    }

    // Final flush of the live handle.
    if let Some(m) = LOG_FILE.get() {
        if let Ok(mut guard) = m.lock() {
            if let Some(f) = guard.as_mut() {
                let _ = f.flush();
            }
            // Drop the handle so the rename is clean.
            *guard = None;
        }
    }

    let Some(latest) = LOG_PATH.get() else { return };
    if !latest.exists() {
        return;
    }

    let stamp = local_timestamp(now_epoch_secs());
    if let Some(dir) = latest.parent() {
        let target = unique_path(dir, &stamp);
        let _ = fs::rename(latest, target);
    }
}

// ---------------------------------------------------------------------------
// config id + registry upload
// ---------------------------------------------------------------------------

// The config id logs under this target. The stderr filter whitelists it at info
// (see init / apply_log_filter) so the one line reaches the game's stderr -- and
// thus the Minecraft launcher log -- without opening the floodgates.
pub const SHARE_TARGET: &str = "tuxinjector::config_code";

/// Deterministic 8-char id for a config: base62 of a 64-bit FNV-1a hash of the
/// canonical JSON. Same settings -> same id. A companion website resolves the id
/// against the registry that tux uploads the full config to.
pub fn config_id(cfg: &tuxinjector_config::Config) -> String {
    let json = serde_json::to_string(cfg).unwrap_or_default();
    base62(fnv1a64(json.as_bytes()), 8)
}

/// Print the config id (in both logs) and best-effort upload the full config to
/// the registry so a website can resolve the id -> settings. The upload only
/// happens when TUXINJECTOR_CONFIG_REGISTRY (a base URL) is set; otherwise just
/// the id is shown.
///
/// Registry contract (you host this):
///   PUT  <base>/<id>   body = config JSON   -> store JSON under the id
///   GET  <base>/<id>                        -> return the stored JSON
/// The website takes an id from the user, GETs it, and renders every setting.
pub fn log_config_id(cfg: &tuxinjector_config::Config) {
    let id = config_id(cfg);
    tracing::info!(target: SHARE_TARGET, "config id: {id}");
    upload_config(id, cfg);
}

fn upload_config(id: String, cfg: &tuxinjector_config::Config) {
    let Ok(base) = std::env::var("TUXINJECTOR_CONFIG_REGISTRY") else {
        return; // no registry configured -> id is shown but nothing is uploaded
    };
    let base = base.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return;
    }
    let Ok(json) = serde_json::to_string(cfg) else {
        return;
    };
    let url = format!("{base}/{id}");
    // Detached, best-effort, in-process (ureq -> no subprocess / SIGCHLD races).
    let _ = std::thread::Builder::new()
        .name("tux-config-upload".into())
        .spawn(move || {
            let agent = ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(10))
                .build();
            match agent
                .put(&url)
                .set("Content-Type", "application/json")
                .set("User-Agent", "tuxinjector")
                .send_string(&json)
            {
                Ok(_) => tracing::debug!(url = %url, "config registry: uploaded"),
                Err(e) => {
                    tracing::debug!(url = %url, error = %e, "config registry: upload failed")
                }
            }
        });
}

// ---------------------------------------------------------------------------
// config diff
// ---------------------------------------------------------------------------

/// Diff two configs and log each changed leaf path at INFO. Logs nothing if
/// the configs are identical. Intended for the settings-instrumentation agent.
#[allow(dead_code)] // wired into config-publish sites by a later agent
pub fn log_config_change(old: &tuxinjector_config::Config, new: &tuxinjector_config::Config) {
    let (Ok(ov), Ok(nv)) = (serde_json::to_value(old), serde_json::to_value(new)) else {
        return;
    };
    if ov == nv {
        return;
    }
    let mut changes: Vec<(String, String, String)> = Vec::new();
    diff_value("", &ov, &nv, &mut changes);
    for (path, old_s, new_s) in changes {
        tracing::info!(path = %path, old = %old_s, new = %new_s, "setting changed");
    }
}

fn diff_value(
    path: &str,
    old: &serde_json::Value,
    new: &serde_json::Value,
    out: &mut Vec<(String, String, String)>,
) {
    use serde_json::Value;
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            // union of keys
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                let av = a.get(k).unwrap_or(&Value::Null);
                let bv = b.get(k).unwrap_or(&Value::Null);
                diff_value(&child, av, bv, out);
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a == b {
                return;
            }
            let n = a.len().max(b.len());
            for i in 0..n {
                let child = format!("{path}[{i}]");
                let av = a.get(i).unwrap_or(&Value::Null);
                let bv = b.get(i).unwrap_or(&Value::Null);
                diff_value(&child, av, bv, out);
            }
        }
        _ => {
            if old != new {
                out.push((path.to_string(), leaf_str(old), leaf_str(new)));
            }
        }
    }
}

fn leaf_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

// ---------------------------------------------------------------------------
// internals: paths / rotation helpers
// ---------------------------------------------------------------------------

// <config_dir>/logs, falling back to ~/.config/tuxinjector/logs. We avoid the
// global state singleton here because init() runs before config_init, so the
// config_dir isn't published yet; we replicate the same resolution order.
fn logs_dir() -> Option<PathBuf> {
    let base = config_base_dir()?;
    Some(base.join("logs"))
}

fn config_base_dir() -> Option<PathBuf> {
    // explicit config-file override -> its parent dir
    if let Ok(path) = std::env::var("TUXINJECTOR_CONFIG_PATH") {
        let p = PathBuf::from(path);
        if let Some(parent) = p.parent() {
            if !parent.as_os_str().is_empty() {
                return Some(parent.to_path_buf());
            }
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("tuxinjector"));
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            // Match the documented fallback: ~/.config/tuxinjector
            return Some(PathBuf::from(home).join(".config/tuxinjector"));
        }
    }
    None
}

// Rename a leftover latest.log to <its-mtime>.log so a crashed session's log
// survives the next launch.
fn rotate_leftover(latest: &Path) {
    if !latest.exists() {
        return;
    }
    let secs = mtime_epoch_secs(latest).unwrap_or_else(now_epoch_secs);
    let stamp = local_timestamp(secs);
    if let Some(dir) = latest.parent() {
        let target = unique_path(dir, &stamp);
        let _ = fs::rename(latest, target);
    }
}

// dir/<stamp>.log, adding _N if it already exists.
fn unique_path(dir: &Path, stamp: &str) -> PathBuf {
    let first = dir.join(format!("{stamp}.log"));
    if !first.exists() {
        return first;
    }
    for n in 1..1000 {
        let p = dir.join(format!("{stamp}_{n}.log"));
        if !p.exists() {
            return p;
        }
    }
    dir.join(format!("{stamp}_{}.log", now_epoch_secs()))
}

fn mtime_epoch_secs(p: &Path) -> Option<u64> {
    let meta = fs::metadata(p).ok()?;
    let mtime = meta.modified().ok()?;
    mtime.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// internals: timestamp formatting (dep-free, UTC)
// ---------------------------------------------------------------------------

// Local-time breakdown via libc::localtime_r -- matches Minecraft / Toolscreen,
// which stamp logs in local time. libc is already a dependency, so this stays
// dep-free. Falls back to UTC (civil_from_epoch) only if localtime_r fails,
// which shouldn't happen on a normal Linux/macOS host.
fn broken_down(epoch_secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    #[cfg(unix)]
    unsafe {
        let t = epoch_secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&t, &mut tm).is_null() {
            return (
                tm.tm_year as i64 + 1900,
                (tm.tm_mon + 1) as u32,
                tm.tm_mday as u32,
                tm.tm_hour as u32,
                tm.tm_min as u32,
                tm.tm_sec as u32,
            );
        }
    }
    civil_from_epoch(epoch_secs)
}

// Filename stamp "YYYY-MM-DD_HH-MM-SS" in local time (never a ':' -- the colon
// isn't filesystem-safe).
fn local_timestamp(epoch_secs: u64) -> String {
    let (y, mo, d, h, mi, s) = broken_down(epoch_secs);
    format!("{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}-{s:02}")
}

// Human-readable local time for the banner lines.
fn human_timestamp(epoch_secs: u64) -> String {
    let (y, mo, d, h, mi, s) = broken_down(epoch_secs);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
}

// days-from-civil inverse: epoch seconds -> (year, month, day, hour, min, sec).
// Uses Howard Hinnant's well-known algorithm.
fn civil_from_epoch(epoch_secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (epoch_secs / 86_400) as i64;
    let secs_of_day = (epoch_secs % 86_400) as u32;
    let hour = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    let sec = secs_of_day % 60;

    // shift epoch to be relative to 0000-03-01
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as i64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };

    (year, month, day, hour, min, sec)
}

// ---------------------------------------------------------------------------
// internals: banner + panic hook
// ---------------------------------------------------------------------------

fn write_banner(path: &Path) {
    let now = now_epoch_secs();
    tracing::info!("========================================");
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "tuxinjector log session"
    );
    tracing::info!(
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        "platform"
    );
    tracing::info!(started = %human_timestamp(now), "session start");
    tracing::info!(log = %path.display(), "log file");
    tracing::info!("========================================");
}

fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic payload>".to_string()
        };
        tracing::error!(location = %location, message = %msg, "tuxinjector panic");
        // chain to the previous hook (default backtrace printer, etc.)
        prev(info);
    }));
}

// ---------------------------------------------------------------------------
// internals: config id (fnv-1a 64 + base62, dep-free)
// ---------------------------------------------------------------------------

// FNV-1a, 64-bit.
fn fnv1a64(data: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// Encode the low bits of `v` as exactly `n` base62 chars (0-9 A-Z a-z). 8 chars
// is ~47.6 bits of the hash -- plenty to make collisions vanishingly unlikely
// for a config registry while staying short and case-mixed.
fn base62(mut v: u64, n: usize) -> String {
    const ALPHABET: &[u8; 62] =
        b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut buf = vec![b'0'; n];
    for slot in buf.iter_mut().rev() {
        *slot = ALPHABET[(v % 62) as usize];
        v /= 62;
    }
    String::from_utf8(buf).expect("base62 alphabet is ascii")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_known_dates() {
        // 0 -> 1970-01-01 00:00:00
        assert_eq!(civil_from_epoch(0), (1970, 1, 1, 0, 0, 0));
        // 1700000000 -> 2023-11-14 22:13:20 UTC
        assert_eq!(civil_from_epoch(1_700_000_000), (2023, 11, 14, 22, 13, 20));
    }

    #[test]
    fn timestamp_has_no_colon() {
        // Value is local-tz dependent, so assert the filesystem-safe shape, not
        // an exact string: "YYYY-MM-DD_HH-MM-SS" is 19 chars with no ':'.
        let s = local_timestamp(1_700_000_000);
        assert!(!s.contains(':'), "filename stamp must be filesystem-safe: {s}");
        assert_eq!(s.len(), 19, "unexpected stamp shape: {s}");
    }

    #[test]
    fn fnv64_stable() {
        // FNV-1a 64 offset basis for the empty input.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
    }

    #[test]
    fn base62_shape() {
        // Always exactly n chars, only the base62 alphabet.
        let s = base62(0, 8);
        assert_eq!(s, "00000000");
        let s = base62(u64::MAX, 8);
        assert_eq!(s.len(), 8);
        assert!(s.bytes().all(|c| c.is_ascii_alphanumeric()));
        // distinct inputs -> distinct low-bit encodings
        assert_ne!(base62(1, 8), base62(2, 8));
    }

    #[test]
    fn config_id_is_8_chars_and_deterministic() {
        let cfg = tuxinjector_config::Config::default();
        let a = config_id(&cfg);
        let b = config_id(&cfg);
        assert_eq!(a.len(), 8, "id must be 8 chars, got {a:?}");
        assert_eq!(a, b, "same config must yield the same id");
        assert!(a.bytes().all(|c| c.is_ascii_alphanumeric()));

        // a changed setting must change the id
        let mut cfg2 = tuxinjector_config::Config::default();
        cfg2.input.mouse_sensitivity = 2.5;
        assert_ne!(config_id(&cfg2), a, "changed config should change the id");
    }
}
