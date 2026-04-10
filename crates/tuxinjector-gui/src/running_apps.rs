// Tracks companion apps launched from the Apps tab.
// Just a Vec behind a mutex. Keeps it simple.

use std::collections::HashMap;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

// Lock-free counter so other threads (e.g. the swap hook fast path) can
// cheaply check whether any companion apps are registered.
static REGISTERED_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Cheap, lock-free count of currently-registered companion apps.
pub fn registered_count() -> usize {
    REGISTERED_COUNT.load(Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    pub const ALL: &[Anchor] = &[
        Anchor::TopLeft,
        Anchor::Top,
        Anchor::TopRight,
        Anchor::Left,
        Anchor::Center,
        Anchor::Right,
        Anchor::BottomLeft,
        Anchor::Bottom,
        Anchor::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Anchor::TopLeft     => "Top Left",
            Anchor::Top         => "Top",
            Anchor::TopRight    => "Top Right",
            Anchor::Left        => "Left",
            Anchor::Center      => "Center",
            Anchor::Right       => "Right",
            Anchor::BottomLeft  => "Bottom Left",
            Anchor::Bottom      => "Bottom",
            Anchor::BottomRight => "Bottom Right",
        }
    }

    pub fn position(self, vp_w: i32, vp_h: i32, win_w: i32, win_h: i32, margin: i32) -> (i32, i32) {
        let cx = (vp_w - win_w) / 2;
        let cy = (vp_h - win_h) / 2;
        match self {
            Anchor::TopLeft     => (margin, margin),
            Anchor::Top         => (cx, margin),
            Anchor::TopRight    => (vp_w - win_w - margin, margin),
            Anchor::Left        => (margin, cy),
            Anchor::Center      => (cx, cy),
            Anchor::Right       => (vp_w - win_w - margin, cy),
            Anchor::BottomLeft  => (margin, vp_h - win_h - margin),
            Anchor::Bottom      => (cx, vp_h - win_h - margin),
            Anchor::BottomRight => (vp_w - win_w - margin, vp_h - win_h - margin),
        }
    }
}

// TODO: maybe add custom anchors to the function above

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    Anchored(Anchor),
    Detached,
}

#[derive(Clone, Debug)]
pub struct RunningApp {
    pub pid: u32,
    pub name: String,
    pub mode: LaunchMode,
}

fn registry() -> &'static Mutex<Vec<RunningApp>> {
    static REG: OnceLock<Mutex<Vec<RunningApp>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn register(pid: u32, name: impl Into<String>, mode: LaunchMode) {
    if let Ok(mut list) = registry().lock() {
        list.retain(|a| a.pid != pid);
        list.push(RunningApp { pid, name: name.into(), mode });
        REGISTERED_COUNT.store(list.len(), Ordering::Relaxed);
    }
}

pub fn unregister(pid: u32) {
    if let Ok(mut list) = registry().lock() {
        list.retain(|a| a.pid != pid);
        REGISTERED_COUNT.store(list.len(), Ordering::Relaxed);
    }
    unregister_stdin(pid);
}

pub fn list() -> Vec<RunningApp> {
    registry().lock().map(|g| g.clone()).unwrap_or_default()
}

// --- Stdin piping for companion app key forwarding ---

fn stdin_map() -> &'static Mutex<HashMap<u32, ChildStdin>> {
    static MAP: OnceLock<Mutex<HashMap<u32, ChildStdin>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn register_stdin(pid: u32, stdin: ChildStdin) {
    if let Ok(mut map) = stdin_map().lock() {
        map.insert(pid, stdin);
    }
}

pub fn unregister_stdin(pid: u32) {
    if let Ok(mut map) = stdin_map().lock() {
        map.remove(&pid);
    }
}

/// Write a line to a companion app's stdin.
pub fn write_stdin(pid: u32, data: &[u8]) {
    if let Ok(mut map) = stdin_map().lock() {
        if let Some(stdin) = map.get_mut(&pid) {
            let _ = stdin.write_all(data);
            let _ = stdin.flush();
        }
    }
}
