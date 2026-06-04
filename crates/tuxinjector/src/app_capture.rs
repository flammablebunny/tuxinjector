// Grabs companion app pixels via X11 GetImage and draws them as GL textures
// inside the game's backbuffer. Uses override_redirect + offscreen positioning
// to hide the window from the user. XComposite is intentionally NOT used
// because AWT stops repainting after composite redirect on XWayland.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::res::{ClientIdMask, ClientIdSpec, ConnectionExt as ResExt};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as XprotoExt, ImageFormat, PropMode, Window,
};
use x11rb::protocol::xtest::ConnectionExt as XtestExt;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

type X11Res<T> = Result<T, Box<dyn std::error::Error>>;

const CAPTURE_INTERVAL: Duration = Duration::from_millis(100);

// Key events queued for XTEST injection into tux's private Xvfb. Each entry is
// an (X keycode, pressed) pair, replayed as a real X KeyPress/KeyRelease so the
// app's JNativeHook (XRecord) global hotkeys fire — which they do on a server
// tux owns, unlike the host Xwayland.
static APP_KEY_QUEUE: std::sync::Mutex<Vec<(u8, bool)>> = std::sync::Mutex::new(Vec::new());

// GLFW (Wayland backend) reports evdev scancodes; the X11 keycode is evdev + 8.
// NBB's JNativeHook reports that same X keycode as each hotkey's rawCode, so the
// injected keycode matches the stored binding directly — no GLFW→JNH table.
const EVDEV_X_KEYCODE_OFFSET: i32 = 8;

/// Queue a key press/release for XTEST injection into companion apps (e.g. NBB).
/// `scancode` is the GLFW (evdev) scancode; we convert it to the X11 keycode.
pub fn push_app_key(_key: i32, scancode: i32, _mods: i32, pressed: bool) {
    let keycode = scancode + EVDEV_X_KEYCODE_OFFSET;
    if scancode <= 0 || keycode > 255 {
        return;
    }
    if let Ok(mut q) = APP_KEY_QUEUE.lock() {
        q.push((keycode as u8, pressed));
    }
}

// NOTE: Java Swing apps need a little bit of time to stabilize after we find the window
const STABILIZATION_FRAMES: u64 = 120;

pub struct CapturedApp {
    pub pixels: Vec<u8>, // RGBA
    pub width: u32,
    pub height: u32,
    pub anchor_x: f32,  // viewport-relative
    pub anchor_y: f32,
}

enum CapturePhase {
    Stabilizing { found_at_frame: u64, unmapped: bool },
    Offscreen,
}

struct EmbeddedWindow {
    window: Window,
    phase: CapturePhase,
    pixels: Option<Vec<u8>>,
    width: u32,
    height: u32,
    // background capture thread writes here
    bg_pixels: Option<std::sync::Arc<std::sync::Mutex<BgCapture>>>,
    last_bg_gen: u64,
}

struct BgCapture {
    pixels: Option<Vec<u8>>,
    width: u32,
    height: u32,
    gen: u64,
    stop: bool,
}

pub struct AppCaptureManager {
    conn: Option<RustConnection>,
    screen_num: usize,
    wm_pid_atom: u32,
    wm_type_atom: u32,
    wm_type_utility_atom: u32,
    embedded: HashMap<u32, EmbeddedWindow>,
    search_fails: HashMap<u32, u32>,
    floated_pids: HashSet<u32>,
    frame: u64,
    visible: bool,
}

impl AppCaptureManager {
    pub fn new() -> Self {
        Self {
            conn: None,
            screen_num: 0,
            wm_pid_atom: 0,
            wm_type_atom: 0,
            wm_type_utility_atom: 0,
            embedded: HashMap::new(),
            search_fails: HashMap::new(),
            floated_pids: HashSet::new(),
            frame: 0,
            visible: true,
        }
    }

    pub fn known_pids(&self) -> Vec<u32> {
        self.embedded.keys().copied().collect()
    }

    pub fn toggle_visibility(&mut self) {
        self.visible = !self.visible;
        tracing::debug!(visible = self.visible, "toggled anchored app visibility");
    }

    /// Inject queued key press/release events into tux's private Xvfb via XTEST.
    /// Stock companion apps (NBB) receive real X KeyPress/KeyRelease there, so
    /// their JNativeHook/XRecord global hotkeys fire — no stdin patch needed.
    /// This works only because tux is the X authority on that server (it does
    /// NOT work on the host Xwayland, where these events are never delivered).
    pub fn forward_pending_keys(&mut self) {
        let events: Vec<(u8, bool)> = match APP_KEY_QUEUE.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => return,
        };
        if events.is_empty() { return; }
        if tuxinjector_gui::running_apps::list().is_empty() { return; }
        if !self.ensure_connected() { return; }

        let conn = match self.conn.as_ref() { Some(c) => c, None => return };
        let root = conn.setup().roots[self.screen_num].root;

        // X protocol event codes: KeyPress = 2, KeyRelease = 3.
        const X_KEY_PRESS: u8 = 2;
        const X_KEY_RELEASE: u8 = 3;
        for (keycode, pressed) in &events {
            let type_ = if *pressed { X_KEY_PRESS } else { X_KEY_RELEASE };
            let _ = conn.xtest_fake_input(type_, *keycode, 0, root, 0, 0, 0);
        }
        let _ = conn.flush();
    }

    /// Set _NET_WM_WINDOW_TYPE to UTILITY on detached app windows so tiling WMs float them.
    pub fn set_float_hint(&mut self, pid: u32) {
        if self.floated_pids.contains(&pid) { return; }
        if !self.ensure_connected() { return; }

        // resolve atoms lazily
        if self.wm_type_atom == 0 {
            if let Some(conn) = self.conn.as_ref() {
                self.wm_type_atom = intern(conn, b"_NET_WM_WINDOW_TYPE").unwrap_or(0);
                self.wm_type_utility_atom = intern(conn, b"_NET_WM_WINDOW_TYPE_UTILITY").unwrap_or(0);
            }
        }
        if self.wm_type_atom == 0 || self.wm_type_utility_atom == 0 { return; }

        let wins = self.find_all_windows_by_pid_stateless(pid);
        if wins.is_empty() { return; }

        if let Some(conn) = self.conn.as_ref() {
            for &win in &wins {
                let _ = conn.change_property32(
                    PropMode::REPLACE,
                    win,
                    self.wm_type_atom,
                    AtomEnum::ATOM,
                    &[self.wm_type_utility_atom],
                );
            }
            let _ = conn.flush();
            tracing::debug!(pid, count = wins.len(), "set _NET_WM_WINDOW_TYPE_UTILITY");
        }

        self.floated_pids.insert(pid);
    }

    pub fn drop_window(&mut self, pid: u32) {
        if let Some(entry) = self.embedded.remove(&pid) {
            if let Some(conn) = self.conn.as_ref() {
                if matches!(entry.phase, CapturePhase::Offscreen) {
                    let _ = conn.unmap_window(entry.window);
                    let _ = conn.flush();
                }
            }
        }
        self.search_fails.remove(&pid);
        self.floated_pids.remove(&pid);
    }

    pub fn embed(
        &mut self,
        pid: u32,
        vp_w: u32,
        vp_h: u32,
        anchor: tuxinjector_gui::running_apps::Anchor,
    ) -> Option<CapturedApp> {
        self.frame = self.frame.wrapping_add(1);

        let fails = self.search_fails.get(&pid).copied().unwrap_or(0);
        if fails > 500 && self.frame % 60 != 0 {
            return None;
        }

        if !self.ensure_connected() {
            return None;
        }

        // discover the app window if we haven't cached it yet
        if !self.embedded.contains_key(&pid) {
            let all_wins = self.find_all_windows_by_pid(pid);
            if all_wins.is_empty() {
                let f = self.search_fails.entry(pid).or_insert(0);
                *f = f.saturating_add(1);
                return None;
            }

            self.search_fails.remove(&pid);

            let win = all_wins[0];

            // On the private Xvfb the app is already invisible (headless). Do NOT
            // unmap or move it offscreen — it must stay mapped on-screen within the
            // Xvfb for X11 GetImage to return its pixels.
            let unmapped = false;

            self.embedded.insert(
                pid,
                EmbeddedWindow {
                    window: win,
                    phase: CapturePhase::Stabilizing {
                        found_at_frame: self.frame,
                        unmapped,
                    },
                    pixels: None,
                    width: 0,
                    height: 0,
                    bg_pixels: None,
                    last_bg_gen: 0,
                },
            );
        }

        // state machine: stabilization -> offscreen transition
        {
            let conn = self.conn.as_ref()?;
            let entry = self.embedded.get(&pid)?;
            if let CapturePhase::Stabilizing { found_at_frame, unmapped } = &entry.phase {
                let age = self.frame.wrapping_sub(*found_at_frame);
                if age < STABILIZATION_FRAMES {
                    return None;
                }

                if conn
                    .get_window_attributes(entry.window)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .is_none()
                {
                    self.embedded.remove(&pid);
                    return None;
                }

                let unmapped = *unmapped;
                let win = entry.window;

                match setup_offscreen(conn, win, unmapped) {
                    Ok(()) => {
                        tracing::info!(pid, win, "offscreen capture active");
                        let e = self.embedded.get_mut(&pid)?;
                        e.phase = CapturePhase::Offscreen;
                    }
                    Err(e) => {
                        tracing::warn!(pid, win, %e, "offscreen setup failed");
                        self.embedded.remove(&pid);
                        return None;
                    }
                }
            }
        }

        if !self.visible {
            return None;
        }

        // start background capture thread if not yet running
        {
            let entry = self.embedded.get_mut(&pid)?;
            if entry.bg_pixels.is_none() && matches!(entry.phase, CapturePhase::Offscreen) {
                let shared = std::sync::Arc::new(std::sync::Mutex::new(BgCapture {
                    pixels: None, width: 0, height: 0, gen: 0, stop: false,
                }));
                entry.bg_pixels = Some(shared.clone());
                let win = entry.window;
                let display = tuxinjector_gui::companion_xserver::display_string();
                std::thread::Builder::new()
                    .name("app-capture".into())
                    .spawn(move || bg_capture_loop(win, display, shared))
                    .ok();
            }
        }

        // read latest pixels from background thread (only clone when new data)
        let entry = self.embedded.get_mut(&pid)?;
        if let Some(bg) = &entry.bg_pixels {
            if let Ok(guard) = bg.lock() {
                if guard.gen != entry.last_bg_gen {
                    if let Some(ref px) = guard.pixels {
                        entry.pixels = Some(px.clone());
                        entry.width = guard.width;
                        entry.height = guard.height;
                        entry.last_bg_gen = guard.gen;
                    }
                }
            }
        }

        let pixels = entry.pixels.as_ref()?;
        if entry.width == 0 || entry.height == 0 {
            return None;
        }

        let (off_x, off_y) = anchor.position(
            vp_w as i32, vp_h as i32,
            entry.width as i32, entry.height as i32,
            0,
        );

        Some(CapturedApp {
            pixels: pixels.clone(),
            width: entry.width,
            height: entry.height,
            anchor_x: off_x as f32,
            anchor_y: off_y as f32,
        })
    }

    fn ensure_connected(&mut self) -> bool {
        if self.conn.is_some() {
            return true;
        }
        // Companion apps run on tux's private headless Xvfb, not the host display.
        let disp = match tuxinjector_gui::companion_xserver::display_string() {
            Some(d) => d,
            None => return false, // server not started yet → nothing to capture
        };
        match x11rb::connect(Some(&disp)) {
            Ok((conn, screen)) => {
                self.conn = Some(conn);
                self.screen_num = screen;
                true
            }
            Err(e) => {
                tracing::warn!(%e, display = %disp, "failed to connect to companion X server");
                false
            }
        }
    }

    fn net_wm_pid_atom(&mut self) -> Option<u32> {
        if self.wm_pid_atom != 0 {
            return Some(self.wm_pid_atom);
        }
        let conn = self.conn.as_ref()?;
        let r = conn.intern_atom(false, b"_NET_WM_PID").ok()?.reply().ok()?;
        self.wm_pid_atom = r.atom;
        Some(r.atom)
    }

    fn find_all_windows_by_pid(&mut self, pid: u32) -> Vec<Window> {
        if let Some(conn) = self.conn.as_ref() {
            let wins = find_all_via_xres(conn, self.screen_num, pid);
            if !wins.is_empty() {
                return wins;
            }
        }

        let atom = match self.net_wm_pid_atom() {
            Some(a) => a,
            None => return Vec::new(),
        };
        let conn = match self.conn.as_ref() {
            Some(c) => c,
            None => return Vec::new(),
        };
        let root = conn.setup().roots[self.screen_num].root;
        let mut results = Vec::new();
        find_all_recursive(conn, root, atom, pid, &mut results);
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.into_iter().map(|(w, _)| w).collect()
    }

    // Same as find_all_windows_by_pid but doesn't need &mut self (for set_float_hint)
    fn find_all_windows_by_pid_stateless(&self, pid: u32) -> Vec<Window> {
        let conn = match self.conn.as_ref() {
            Some(c) => c,
            None => return Vec::new(),
        };
        let wins = find_all_via_xres(conn, self.screen_num, pid);
        if !wins.is_empty() {
            return wins;
        }
        if self.wm_pid_atom == 0 { return Vec::new(); }
        let root = conn.setup().roots[self.screen_num].root;
        let mut results = Vec::new();
        find_all_recursive(conn, root, self.wm_pid_atom, pid, &mut results);
        results.sort_by(|a, b| b.1.cmp(&a.1));
        results.into_iter().map(|(w, _)| w).collect()
    }

}

fn bg_capture_loop(
    win: Window,
    display: Option<String>,
    shared: std::sync::Arc<std::sync::Mutex<BgCapture>>,
) {
    let (conn, _) = match x11rb::connect(display.as_deref()) {
        Ok(c) => c,
        Err(_) => return,
    };

    loop {
        std::thread::sleep(CAPTURE_INTERVAL);

        if let Ok(guard) = shared.lock() {
            if guard.stop { return; }
        }

        let (w, h) = match win_size(&conn, win) {
            Some(wh) => wh,
            None => continue,
        };
        if w == 0 || h == 0 { continue; }

        match conn.get_image(ImageFormat::Z_PIXMAP, win, 0, 0, w as u16, h as u16, !0) {
            Ok(cookie) => match cookie.reply() {
                Ok(reply) => {
                    let px = bgra_to_rgba(&reply.data, reply.depth);
                    if let Ok(mut guard) = shared.lock() {
                        guard.pixels = Some(px);
                        guard.width = w;
                        guard.height = h;
                        guard.gen += 1;
                    }
                }
                Err(_) => {}
            },
            Err(_) => {}
        }
    }
}

// No-op on the private Xvfb. Companion apps run on tux's own headless X server,
// so there is no host WM to hide from and no user-visible display to move off of.
// The window is left exactly where AWT mapped it (on-screen within the Xvfb) so
// X11 GetImage can read its pixels. (Previously this unmapped + override-redirected
// + moved the window to -32000,-32000 to hide it on the host display — which would
// put it off the Xvfb screen and break capture.)
fn setup_offscreen(_conn: &RustConnection, _win: Window, _already_unmapped: bool) -> X11Res<()> {
    Ok(())
}

fn intern(conn: &RustConnection, name: &[u8]) -> Option<Atom> {
    Some(conn.intern_atom(false, name).ok()?.reply().ok()?.atom)
}

fn bgra_to_rgba(data: &[u8], depth: u8) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(data.len());
    for chunk in data.chunks_exact(4) {
        rgba.push(chunk[2]); // R
        rgba.push(chunk[1]); // G
        rgba.push(chunk[0]); // B
        rgba.push(if depth >= 32 { chunk[3] } else { 255 });
    }
    rgba
}

fn win_size(conn: &RustConnection, window: Window) -> Option<(u32, u32)> {
    let g = conn.get_geometry(window).ok()?.reply().ok()?;
    Some((g.width as u32, g.height as u32))
}

fn find_all_via_xres(
    conn: &RustConnection,
    screen_num: usize,
    target_pid: u32,
) -> Vec<Window> {
    let specs = [ClientIdSpec {
        client: 0,
        mask: ClientIdMask::LOCAL_CLIENT_PID,
    }];
    let reply = match conn.res_query_client_ids(&specs).ok().and_then(|c| c.reply().ok()) {
        Some(r) => r,
        None => return Vec::new(),
    };

    let id_mask = conn.setup().resource_id_mask;

    let matching_bases: Vec<u32> = reply
        .ids
        .iter()
        .filter(|id| id.value.first().copied() == Some(target_pid))
        .map(|id| id.spec.client)
        .collect();

    if matching_bases.is_empty() {
        return Vec::new();
    }

    let root = conn.setup().roots[screen_num].root;
    let mut candidates: Vec<(Window, u32)> = Vec::new();

    if let Some(tree) = conn.query_tree(root).ok().and_then(|c| c.reply().ok()).map(|r| r.children) {
        for &child in &tree {
            if matching_bases.contains(&(child & !id_mask)) {
                if let Some(area) = input_output_area(conn, child) {
                    candidates.push((child, area));
                }
            }
        }
        if candidates.is_empty() {
            for &child in &tree {
                collect_matching(conn, child, &matching_bases, id_mask, &mut candidates);
            }
        }
    }

    candidates.sort_by(|a, b| b.1.cmp(&a.1));
    candidates.into_iter().map(|(w, _)| w).collect()
}

fn input_output_area(conn: &RustConnection, window: Window) -> Option<u32> {
    use x11rb::protocol::xproto::WindowClass;
    let attrs = conn.get_window_attributes(window).ok()?.reply().ok()?;
    if attrs.class == WindowClass::INPUT_OUTPUT {
        let geom = conn.get_geometry(window).ok()?.reply().ok()?;
        Some(geom.width as u32 * geom.height as u32)
    } else {
        None
    }
}

fn collect_matching(
    conn: &RustConnection,
    window: Window,
    bases: &[u32],
    id_mask: u32,
    out: &mut Vec<(Window, u32)>,
) {
    if bases.contains(&(window & !id_mask)) {
        if let Some(area) = input_output_area(conn, window) {
            out.push((window, area));
        }
    }
    if let Some(tree) = conn.query_tree(window).ok().and_then(|c| c.reply().ok()) {
        for child in tree.children {
            collect_matching(conn, child, bases, id_mask, out);
        }
    }
}

fn find_all_recursive(
    conn: &RustConnection,
    window: Window,
    wm_pid_atom: u32,
    target_pid: u32,
    out: &mut Vec<(Window, u32)>,
) {
    if let Ok(cookie) = conn.get_property(false, window, wm_pid_atom, AtomEnum::CARDINAL, 0, 1) {
        if let Ok(reply) = cookie.reply() {
            if let Some(pid) = reply.value32().and_then(|mut it| it.next()) {
                if pid == target_pid {
                    let area = input_output_area(conn, window).unwrap_or(0);
                    out.push((window, area));
                }
            }
        }
    }

    if let Some(tree) = conn.query_tree(window).ok().and_then(|c| c.reply().ok()) {
        for child in tree.children {
            find_all_recursive(conn, child, wm_pid_atom, target_pid, out);
        }
    }
}
