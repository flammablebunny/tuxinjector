// A private, headless X server (Xvfb) that tuxinjector owns and runs companion
// apps (e.g. Ninjabrain Bot) inside of. Because tux is the X authority on this
// server, synthetic key injection via XTEST is seen by the apps' XRecord-based
// global hotkey listeners (JNativeHook) - which it is NOT on the host's Xwayland.
// The server is headless; tux captures the apps' windows via X11 GetImage and
// composites them into the game, so nothing is ever shown on this display.

use std::process::{Child, Command};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};

struct XServer {
    display: u32,
}

// All companion processes (the Xvfb here, and the apps in tabs::apps) are
// spawned on this one never-exiting thread. PR_SET_PDEATHSIG is delivered when
// the *thread that created the child* exits - a Linux quirk, not when the
// process exits. ensure_started()/launch are reached from the first-frame init,
// which under SeedQueue runs on a short-lived wall-render worker thread;
// spawning there SIGTERMs the companion the instant that worker returns (Xvfb
// then leaves a dead socket every connect hits with ECONNREFUSED). Routing every
// spawn through this permanent thread ties them to the game's lifetime instead.
type SpawnReq = (Command, Sender<std::io::Result<Child>>);

fn spawner() -> &'static Sender<SpawnReq> {
    static TX: OnceLock<Sender<SpawnReq>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<SpawnReq>();
        let _ = std::thread::Builder::new()
            .name("companion-spawner".into())
            .spawn(move || {
                for (mut cmd, reply) in rx {
                    let _ = reply.send(cmd.spawn());
                }
            });
        tx
    })
}

/// Spawn a companion process from the permanent spawner thread, so its
/// PR_SET_PDEATHSIG is tied to the game process - not the caller's (possibly
/// transient) thread. Use this instead of `Command::spawn` for anything that
/// must outlive the call site.
pub fn spawn_companion(cmd: Command) -> std::io::Result<Child> {
    use std::io::{Error, ErrorKind};
    let (tx, rx) = std::sync::mpsc::channel();
    spawner()
        .send((cmd, tx))
        .map_err(|_| Error::new(ErrorKind::Other, "companion spawner thread gone"))?;
    rx.recv()
        .map_err(|_| Error::new(ErrorKind::Other, "companion spawn reply dropped"))?
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
    let display = spawn_xvfb()?;
    guard.replace(XServer { display });
    Some(display)
}

// Resolve the Xvfb binary. 
// On NixOS the game's PATH usually lacks it, so honor an explicit 
// TUXINJECTOR_XVFB=/nix/store/.../bin/Xvfb (set by the wrapper),
// falling back to "Xvfb" on PATH.
fn xvfb_binary() -> String {
    std::env::var("TUXINJECTOR_XVFB").unwrap_or_else(|_| "Xvfb".to_string())
}

fn spawn_xvfb() -> Option<u32> {
    use std::os::unix::net::UnixStream;

    let bin = xvfb_binary();

    // Find a free display number and start Xvfb on it.
    for n in 80u32..140 {
        let sock = format!("/tmp/.X11-unix/X{n}");
        if UnixStream::connect(&sock).is_ok() {
            continue;
        }
        // Clear stale socket/lock so they don't block Xvfb's own startup.
        let _ = std::fs::remove_file(&sock);
        let _ = std::fs::remove_file(format!("/tmp/.X{n}-lock"));

        let err_path = std::env::temp_dir()
            .join(format!("tuxinjector-xvfb-{}-{n}.log", std::process::id()));
        let err_out = std::fs::File::create(&err_path)
            .map(std::process::Stdio::from)
            .unwrap_or_else(|_| std::process::Stdio::null());

        let mut cmd = std::process::Command::new(&bin);
        cmd.arg(format!(":{n}"))
            .arg("-screen").arg("0").arg("1920x1080x24")
            .arg("-ac")
            .arg("-nolisten").arg("tcp")
            .stdout(std::process::Stdio::null())
            .stderr(err_out);
        let mut child = match spawn_companion(cmd) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(%e, bin = %bin,
                    "failed to spawn Xvfb - please install Xvfb. if not on PATH set TUXINJECTOR_XVFB to its path in the wrapper");
                let _ = std::fs::remove_file(&err_path);
                return None;
            }
        };


        let mut exited = None;
        for _ in 0..60 {
            if UnixStream::connect(&sock).is_ok() {
                tracing::info!(display = n, bin = %bin, "companion Xvfb started");
                let watch_err = err_path.clone();
                let _ = std::thread::Builder::new()
                    .name("companion-xvfb-watch".into())
                    .spawn(move || {
                        let status = child.wait();
                        let why = std::fs::read_to_string(&watch_err).unwrap_or_default();
                        let _ = std::fs::remove_file(&watch_err);
                        match status {
                            Ok(s) if s.success() => {
                                tracing::warn!(display = n, "companion Xvfb exited cleanly");
                            }
                            Ok(s) => tracing::error!(
                                display = n, status = %s, xvfb_stderr = %why.trim(),
                                "companion Xvfb DIED - companion apps lose their X server; \
                                 cause is its stderr above (or the exit signal)."),
                            Err(e) => tracing::error!(
                                display = n, %e, xvfb_stderr = %why.trim(),
                                "companion Xvfb vanished (wait() => ECHILD, i.e. the JVM reaped \
                                 it). Empty stderr above => it was killed by a signal; non-empty \
                                 => that's the reason it exited."),
                        }
                    });
                return Some(n);
            }
            if let Ok(Some(status)) = child.try_wait() {
                exited = Some(status); // died on its own - status tells us how
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        // Never became connectable. Surface exactly how it ended + its stderr.
        let how = match exited {
            Some(status) => format!("exited on its own ({status})"),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                "never accepted a connection (killed after 3s)".to_string()
            }
        };
        let why = std::fs::read_to_string(&err_path).unwrap_or_default();
        tracing::error!(display = n, bin = %bin, outcome = %how, xvfb_stderr = %why.trim(),
            "companion Xvfb failed to come up. A 'signal: 15 (SIGTERM)' here means it was \
             killed (PDEATHSIG / parent thread); a non-zero exit + stderr is Xvfb's own error \
             (fonts, GLX/driver env, etc.).");
        let _ = std::fs::remove_file(&err_path);
        let _ = std::fs::remove_file(&sock);
        return None;
    }
    tracing::error!("no free display number for companion Xvfb (:80-:139 all taken)");
    None
}
