//! Liblogger loader - speedrun.com / MCSR Ranked legality requirement.
//!
//! Liblogger is open source: <https://github.com/jojoe77777/Toolscreen>
//!
//! The list of allowed (legal) builds lives at:
//! <https://github.com/Minecraft-Java-Edition-Speedrunning/legal-builds/blob/main/legal-dlls.csv>
//!
//! ## How it works
//!
//! At BUILD TIME (see `build.rs`):
//!   1. We pull the `legal-dlls.csv` from the legal-builds repo, find the
//!      latest `liblogger_<arch>.so` row, and read its expected SHA-512.
//!   2. We download the matching binary from the Toolscreen release at
//!      <https://github.com/jojoe77777/Toolscreen/releases/tag/liblogger-legal>.
//!   3. We verify SHA-512 against the CSV. The build refuses to succeed if
//!      they don't match. **No hashes are hardcoded in this source tree.**
//!   4. The verified binary is written to `OUT_DIR` and embedded into the
//!      tuxinjector .so via `include_bytes!` below.
//!
//! At RUNTIME this module just dlopens the embedded binary - no network,
//! no per-frame work, no hash check (verification already happened at build).
//!
//! ## Disabling
//!
//! Delete the single `liblogger::load();` call line in `lib.rs`. The module
//! itself stays compiled (it has `#[allow(dead_code)]` upstream), so the
//! build still succeeds and liblogger never loads.
//!
//! Doing so makes any subsequent run / match ILLEGAL on speedrun.com / MCSR
//! Ranked.
//!
//! ## Build flags
//!
//! - `TUXINJECTOR_LIBLOGGER_REFRESH=1` - force re-fetch of CSV + binary
//! - `TUXINJECTOR_LIBLOGGER_OFFLINE=1` - skip the fetch entirely; an empty
//!   stub is embedded and the runtime detects it and does nothing.

use std::ffi::{CStr, CString};

#[cfg(target_arch = "x86_64")]
const FILENAME: &str = "liblogger_x64.so";
#[cfg(target_arch = "x86")]
const FILENAME: &str = "liblogger_x86.so";
#[cfg(target_arch = "aarch64")]
const FILENAME: &str = "liblogger_arm64.so";
#[cfg(target_arch = "arm")]
const FILENAME: &str = "liblogger_arm32.so";

const BUNDLED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/liblogger.so"));

pub fn load() {
    tracing::info!(
        filename = FILENAME,
        embedded_bytes = BUNDLED.len(),
        "liblogger load() started"
    );

    // Empty bundle = build was done with TUXINJECTOR_LIBLOGGER_OFFLINE=1
    // (or unsupported arch). Nothing to load.
    if BUNDLED.is_empty() {
        tracing::warn!(
            "liblogger not embedded at build time (offline build or unsupported arch); \
             runs will not be legal on speedrun.com / MCSR Ranked"
        );
        eprintln!(
            "tuxinjector: liblogger was not embedded at build time \
             (TUXINJECTOR_LIBLOGGER_OFFLINE was set, or unsupported arch); \
             runs will not be legal on speedrun.com / MCSR Ranked"
        );
        return;
    }

    let dir = std::env::temp_dir().join(format!("tuxinjector-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join(FILENAME);
    tracing::info!(path = %path.display(), "writing embedded liblogger to disk");
    if let Err(e) = std::fs::write(&path, BUNDLED) {
        tracing::error!(path = %path.display(), error = %e, "failed to write liblogger to disk; aborting load");
        return;
    }
    tracing::info!(path = %path.display(), bytes = BUNDLED.len(), "liblogger written; dlopening");

    unsafe {
        let cpath = match CString::new(path.to_str().unwrap_or("")) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(path = %path.display(), error = %e, "liblogger path not a valid CString; aborting load");
                return;
            }
        };
        let handle = libc::dlopen(cpath.as_ptr(), libc::RTLD_NOW);
        if handle.is_null() {
            let err = libc::dlerror();
            if !err.is_null() {
                let msg = CStr::from_ptr(err).to_string_lossy();
                tracing::error!(path = %path.display(), error = %msg, "liblogger dlopen failed");
                eprintln!("liblogger load failed: {}", msg);
            } else {
                tracing::error!(path = %path.display(), "liblogger dlopen returned null (no dlerror)");
            }
        } else {
            tracing::info!(handle = ?handle, "liblogger dlopen succeeded; constructor (DT_INIT) invoked");
        }
    }

    // Linux keeps the library mapped after unlink, so it's safe to delete.
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
    tracing::info!("liblogger load() complete");
}
