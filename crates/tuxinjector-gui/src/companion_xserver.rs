// A private, headless X server (Xvfb) that tuxinjector owns and runs companion
// apps (e.g. Ninjabrain Bot) inside of. Because tux is the X authority on this
// server, synthetic key injection via XTEST is seen by the apps' XRecord-based
// global hotkey listeners (JNativeHook) - which it is NOT on the host's Xwayland.
// The server is headless; tux captures the apps' windows via X11 GetImage and
// composites them into the game, so nothing is ever shown on this display.

use std::process::Child;
use std::sync::{Mutex, OnceLock};

struct XServer {
    display: u32,
    // Kept alive for the process lifetime; Xvfb is also PDEATHSIG-killed.
    _child: Child,
}

fn slot() -> &'static Mutex<Option<XServer>> {
    static SERVER: OnceLock<Mutex<Option<XServer>>> = OnceLock::new();
    SERVER.get_or_init(|| Mutex::new(None))
}

/// Display number of the companion X server, if it has been started.
pub fn display() -> Option<u32> {
    slot().lock().ok().and_then(|g| g.as_ref().map(|s| s.display))
}

/// `DISPLAY` string (e.g. ":80") for the companion X server, if started.
pub fn display_string() -> Option<String> {
    display().map(|n| format!(":{n}"))
}

/// Ensure the companion Xvfb is running; returns its display number.
/// Idempotent - subsequent calls return the existing server.
pub fn ensure_started() -> Option<u32> {
    let mut guard = slot().lock().ok()?;
    if let Some(s) = guard.as_ref() {
        return Some(s.display);
    }
    let (display, child) = spawn_xvfb()?;
    guard.replace(XServer { display, _child: child });
    Some(display)
}

// Resolve the Xvfb binary. 
// On NixOS the game's PATH usually lacks it, so honor an explicit 
// TUXINJECTOR_XVFB=/nix/store/.../bin/Xvfb (set by the wrapper),
// falling back to "Xvfb" on PATH.
fn xvfb_binary() -> String {
    std::env::var("TUXINJECTOR_XVFB").unwrap_or_else(|_| "Xvfb".to_string())
}

fn spawn_xvfb() -> Option<(u32, Child)> {
    use std::os::unix::process::CommandExt;

    let bin = xvfb_binary();

    // Find a free display number and start Xvfb on it.
    for n in 80u32..140 {
        let sock = format!("/tmp/.X11-unix/X{n}");
        if std::path::Path::new(&sock).exists() {
            continue; // display already in use
        }

        let mut cmd = std::process::Command::new(&bin);
        cmd.arg(format!(":{n}"))
            .arg("-screen").arg("0").arg("1920x1080x24")
            .arg("-ac")
            .arg("-nolisten").arg("tcp")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }

        match cmd.spawn() {
            Ok(child) => {
                // Wait (up to ~3s) for the X socket to appear before using it.
                for _ in 0..60 {
                    if std::path::Path::new(&sock).exists() {
                        tracing::info!(display = n, bin = %bin, "companion Xvfb started");
                        return Some((n, child));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                tracing::warn!(display = n, "Xvfb spawned but socket never appeared");
                let mut child = child;
                let _ = child.kill();
                return None;
            }
            Err(e) => {
                tracing::error!(%e, bin = %bin,
                    "failed to spawn Xvfb - please install Xvfb. if not on PATH set TUXINJECTOR_XVFB to its path in the wrapper");
                return None;
            }
        }
    }
    tracing::error!("no free display number for companion Xvfb (:80-:139 all taken)");
    None
}
