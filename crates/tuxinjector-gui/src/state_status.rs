// Which game-state mod tuxinjector is reading (set by the core's state watcher,
// read by the GUI to warn the user when no source is present). Shared here
// because the GUI crate is a good common dependency both sides can reach.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StateModStatus {
    /// Still probing (mod may not have written its file yet).
    Unknown,
    /// Reading `hermes/state.json`. (Hermes / Hermes Core)
    Hermes,
    /// Reading `wpstateout.txt` (State Output).
    StateOutput,
    /// No *live* game-state source in the instance dir - either no file at all,
    /// or only a stale leftover (dead Hermes heartbeat / pre-load wpstateout.txt).
    Missing,
}

static STATUS: AtomicU8 = AtomicU8::new(0);

pub fn set_state_mod_status(s: StateModStatus) {
    let v = match s {
        StateModStatus::Unknown => 0,
        StateModStatus::Hermes => 1,
        StateModStatus::StateOutput => 2,
        StateModStatus::Missing => 3,
    };
    STATUS.store(v, Ordering::Relaxed);
}

pub fn state_mod_status() -> StateModStatus {
    match STATUS.load(Ordering::Relaxed) {
        1 => StateModStatus::Hermes,
        2 => StateModStatus::StateOutput,
        3 => StateModStatus::Missing,
        _ => StateModStatus::Unknown,
    }
}
