// Game-state detection from an mcsr mod's instance output.
//
// Source priority (all in the Minecraft run dir = our process CWD, since we're
// injected into the game; Prism/MultiMC set cwd to <instance>/minecraft/):
//   1. hermes/state.json   (Hermes - the modern source)
//   2. wpstateout.txt      (State Output - backup)
// A source must be *live*, not merely present: a disabled/crashed mod leaves a
// stale file behind (Hermes a frozen hermes/alive heartbeat, State Output a
// days-old wpstateout.txt), and we treat that as no source. The GUI status
// reflects this live/missing state every tick. The stderr warning, though,
// only fires once, and only if NO live source ever shows up within MOD_WARN_GRACE
// of load -- otherwise we'd nag on every launch (and every trip back to the menu)
// while the JVM + mod are still coming up, even when the mod IS installed.
//
// NOTE: "Using wpstateout.txt (State Output and previously WorldPreview),
// state.json (Hermes), record.json (SpeedRunIGT), or other mod-outputted
// instance state as a performant replacement for checks possible in the
// unmodified game is permitted" (a.8.14.a)
//
// Fires Lua tx.listen("state", fn) events on changes.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use tuxinjector_gui::StateModStatus;

const POLL_MS: Duration = Duration::from_millis(50);

// Hermes rewrites hermes/alive ~1x/sec; more than this without a refresh means
// it's disabled/crashed/removed.
const HERMES_STALE_MS: i128 = 3000;

// Grace period before we'll warn that no state mod is present. The JVM + mod
// take a while to load and start writing/heartbeating, so a missing source
// during the first minute is just "still starting up", not "not installed".
const MOD_WARN_GRACE: Duration = Duration::from_secs(60);

// Captured at process load (start of `spawn_state_watcher`, in the LD_PRELOAD
// ctor) so State Output liveness can mean "written since we loaded" - see
// `wpstateout_fresh`.
static LOAD_TIME: OnceLock<SystemTime> = OnceLock::new();

#[derive(Clone)]
enum StateSource {
    Hermes(PathBuf),
    WpStateOut(PathBuf),
}

impl StateSource {
    fn path(&self) -> &Path {
        match self {
            StateSource::Hermes(p) | StateSource::WpStateOut(p) => p,
        }
    }
}

fn find_live_source() -> Option<StateSource> {
    let cwd = std::env::current_dir().ok()?;

    let hermes_dir = cwd.join("hermes");
    let hermes = hermes_dir.join("state.json");
    if hermes.exists() && hermes_alive(&hermes_dir) {
        return Some(StateSource::Hermes(hermes));
    }

    let wp = cwd.join("wpstateout.txt");
    if wp.exists() && wpstateout_fresh(&wp) {
        return Some(StateSource::WpStateOut(wp));
    }

    None
}

fn hermes_alive(hermes_dir: &Path) -> bool {
    let bytes = match std::fs::read(hermes_dir.join("alive")) {
        Ok(b) if b.len() >= 16 => b,
        _ => return false,
    };
    let pid = u64::from_be_bytes(bytes[0..8].try_into().unwrap());
    let beat = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
    if pid != std::process::id() as u64 {
        return false;
    }
    (now_ms() - beat as i128).abs() < HERMES_STALE_MS
}

fn wpstateout_fresh(path: &Path) -> bool {
    let Some(&load) = LOAD_TIME.get() else {
        return true; // load time not recorded (shouldn't happen): don't false-warn
    };
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|mtime| mtime >= load)
        .unwrap_or(false)
}

fn now_ms() -> i128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as i128)
        .unwrap_or(0)
}

// Hermes screen classes 
const HERMES_LOADING_SCREEN: &str = "net.minecraft.class_3928";
const HERMES_WALL_SCREEN: &str = "me.contaria.seedqueue.gui.wall.SeedQueueWallScreen";
const HERMES_WALL_PREVIEW: &str = "me.contaria.seedqueue.gui.wall.SeedQueuePreview";

// Derive a canonical state from Hermes' state.json. None = unparseable/empty
// (caller keeps the last good state).
fn parse_hermes(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let j: serde_json::Value = serde_json::from_str(t).ok()?;
    if !j.is_object() {
        return None;
    }
    let in_world = j.get("world").map_or(false, |w| !w.is_null());
    let screen_class = j
        .get("screen")
        .and_then(|s| s.get("class"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    match screen_class {
        HERMES_LOADING_SCREEN => return Some("generating".into()),
        HERMES_WALL_SCREEN | HERMES_WALL_PREVIEW => return Some("wall".into()),
        _ => {}
    }
    Some(if in_world {
        inworld_state()
    } else {
        "title".into()
    })
}

// Map a raw wpstateout line to a canonical state string.
fn to_game_state(raw: &str) -> String {
    let tag = raw.split(',').next().unwrap_or("").trim();
    match tag {
        "wall" => "wall".into(),
        "title" => "title".into(),
        "waiting" => "waiting".into(),
        "generating" | "previewing" => "generating".into(),
        "inworld" => inworld_state(),
        _ => "title".into(),
    }
}

// In-world: tag the live cursor-grab state so hotkeys can distinguish
// playing (grabbed) from menu/chat/inventory (free).
fn inworld_state() -> String {
    if tuxinjector_input::is_cursor_captured() {
        "inworld,cursor_grabbed".into()
    } else {
        "inworld,cursor_free".into()
    }
}

pub fn spawn_state_watcher() {
    // Stamp load time here (runs in the LD_PRELOAD ctor, before the game writes
    // anything this session) for `wpstateout_fresh`.
    let _ = LOAD_TIME.set(SystemTime::now());
    let _ = std::thread::Builder::new()
        .name("state-watcher".into())
        .spawn(watcher_loop);
}

fn watcher_loop() {
    let mut last_state = String::new();
    let mut logged_label: Option<&'static str> = None;
    // Seeing a live source even once proves the mod is there, so we never warn.
    // We only warn (once) if nothing ever showed up within the startup grace.
    let mut ever_seen = false;
    let mut warned = false;

    loop {
        match find_live_source() {
            Some(s) => {
                ever_seen = true;
                let (label, status) = match s {
                    StateSource::Hermes(_) => ("hermes/state.json", StateModStatus::Hermes),
                    StateSource::WpStateOut(_) => {
                        ("wpstateout.txt (State Output)", StateModStatus::StateOutput)
                    }
                };
                tuxinjector_gui::set_state_mod_status(status);
                if logged_label != Some(label) {
                    logged_label = Some(label);
                    tracing::info!(path = %s.path().display(), "state watcher: using {label}");
                }

                match std::fs::read_to_string(s.path()) {
                    Ok(content) => {
                        let state = match s {
                            StateSource::Hermes(_) => parse_hermes(&content),
                            StateSource::WpStateOut(_) => {
                                let t = content.trim();
                                (!t.is_empty()).then(|| to_game_state(t))
                            }
                        };
                        if let Some(state) = state {
                            if state != last_state {
                                last_state = state.clone();
                                tracing::debug!(state = %state, "game state change");
                            }
                            // Re-assert every tick (not only on change): game_state
                            // is shared and another writer could clobber it; without
                            // re-asserting, the clobber would stick.
                            push_state(&state);
                        }
                    }
                    // A racing read (file removed between liveness check and read)
                    // just falls through; next tick re-evaluates and warns.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => tracing::warn!(error = %e, "state watcher: read error"),
                }
            }
            None => {
                logged_label = None;
                tuxinjector_gui::set_state_mod_status(StateModStatus::Missing);
                // Only nag if a source never appeared and the grace period has
                // passed -- skips the normal JVM/mod-load startup window.
                let waited = LOAD_TIME
                    .get()
                    .and_then(|t| t.elapsed().ok())
                    .unwrap_or(Duration::ZERO);
                if !ever_seen && !warned && waited >= MOD_WARN_GRACE {
                    warned = true;
                    eprintln!(
                        "tuxinjector: no live game-state mod detected - neither \
                         hermes/state.json (Hermes) nor wpstateout.txt (State \
                         Output) is present and being updated in the instance \
                         directory. A stale leftover file from a previous run or a \
                         disabled mod counts as absent. State-conditioned hotkeys \
                         will fire regardless of game state (their conditions are \
                         treated as \"Any\"), and other state-based features won't \
                         work. Install or re-enable the Hermes or State Output mod."
                    );
                }
            }
        }

        std::thread::sleep(POLL_MS);
    }
}

fn push_state(state: &str) {
    let tx = crate::state::get();

    // update hotkey engine's condition state
    if let Ok(mut guard) = tx.game_state.lock() {
        *guard = state.to_string();
    }

    // lua shared state + runtime notification
    if tuxinjector_lua::update_game_state(state) {
        if let Some(rt) = tx.lua_runtime.get() {
            let _ = rt.state_event_tx.try_send(state.to_string());
        }
    }
}
