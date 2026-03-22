// Grabs companion app pixels via X11 GetImage and draws them as GL textures
// inside the game's backbuffer. Uses override_redirect + offscreen positioning
// to hide the window from the user. XComposite is intentionally NOT used
// because AWT stops repainting after composite redirect on XWayland.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::res::{ClientIdMask, ClientIdSpec, ConnectionExt as ResExt};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as XprotoExt,
    ImageFormat, PropMode, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

type X11Res<T> = Result<T, Box<dyn std::error::Error>>;

const CAPTURE_INTERVAL: Duration = Duration::from_millis(100);

// Key events queued for forwarding to companion apps via stdin.
// (x11_keycode, x11_modifier_mask, jnh_keycode)
// Next function is llm assisted or wtv
static APP_KEY_QUEUE: std::sync::Mutex<Vec<(u8, u16, i32)>> = std::sync::Mutex::new(Vec::new());

/// Queue a key press for forwarding to companion apps via stdin.
/// `key` is the GLFW key constant, `scancode` is the X11 keycode.
pub fn push_app_key(key: i32, scancode: i32, mods: i32, pressed: bool) {
    if !pressed { return; }
    if scancode <= 0 || scancode > 255 { return; }
    let x11_mods = glfw_mods_to_x11(mods);
    let jnh_code = glfw_key_to_jnh(key);
    if let Ok(mut q) = APP_KEY_QUEUE.lock() {
        q.push((scancode as u8, x11_mods, jnh_code));
    }
}

fn glfw_mods_to_x11(mods: i32) -> u16 {
    let mut s = 0u16;
    if mods & 0x1 != 0 { s |= 1; }   // Shift
    if mods & 0x2 != 0 { s |= 4; }   // Control
    if mods & 0x4 != 0 { s |= 8; }   // Alt → Mod1
    if mods & 0x8 != 0 { s |= 64; }  // Super → Mod4
    s
}

/// Convert GLFW key constant to JNativeHook virtual keycode (AT scancode set 1).
/// Returns -1 for unknown keys. This lets NBB match hotkeys without rawCode storage.
fn glfw_key_to_jnh(key: i32) -> i32 {
    match key {
        256 => 0x0001,  // Escape
        290 => 0x003B, 291 => 0x003C, 292 => 0x003D, 293 => 0x003E, // F1-F4
        294 => 0x003F, 295 => 0x0040, 296 => 0x0041, 297 => 0x0042, // F5-F8
        298 => 0x0043, 299 => 0x0044, 300 => 0x0057, 301 => 0x0058, // F9-F12

        96  => 0x0029, // `
        49  => 0x0002, 50 => 0x0003, 51 => 0x0004, 52 => 0x0005, // 1-4
        53  => 0x0006, 54 => 0x0007, 55 => 0x0008, 56 => 0x0009, // 5-8
        57  => 0x000A, 48 => 0x000B, // 9, 0
        45  => 0x000C, // -
        61  => 0x000D, // =
        259 => 0x000E, // Backspace

        258 => 0x000F, // Tab
        81  => 0x0010, 87 => 0x0011, 69 => 0x0012, 82 => 0x0013, // Q W E R
        84  => 0x0014, 89 => 0x0015, 85 => 0x0016, 73 => 0x0017, // T Y U I
        79  => 0x0018, 80 => 0x0019, // O P
        91  => 0x001A, 93 => 0x001B, 92 => 0x002B, // [ ] backslash

        280 => 0x003A, // Caps Lock
        65  => 0x001E, 83 => 0x001F, 68 => 0x0020, 70 => 0x0021, // A S D F
        71  => 0x0022, 72 => 0x0023, 74 => 0x0024, 75 => 0x0025, // G H J K
        76  => 0x0026, 59 => 0x0027, 39 => 0x0028, // L ; '
        257 => 0x001C, // Enter

        90  => 0x002C, 88 => 0x002D, 67 => 0x002E, 86 => 0x002F, // Z X C V
        66  => 0x0030, 78 => 0x0031, 77 => 0x0032, // B N M
        44  => 0x0033, 46 => 0x0034, 47 => 0x0035, // , . /

        340 => 0x002A, 344 => 0x0036, // L/R Shift
        341 => 0x001D, 345 => 0x0E1D, // L/R Control
        342 => 0x0038, 346 => 0x0E38, // L/R Alt
        32  => 0x0039, // Space

        // Numpad
        282 => 0x0045,   // Num Lock
        331 => 0x0E35,   // KP /
        332 => 0x0037,   // KP *
        333 => 0x004A,   // KP -
        334 => 0x004E,   // KP +
        335 => 0x0E1C,   // KP Enter
        330 => 0x0053,   // KP .
        320 => 0x0052, 321 => 0x004F, 322 => 0x0050, 323 => 0x0051, // KP 0-3
        324 => 0x004B, 325 => 0x004C, 326 => 0x004D, // KP 4-6
        327 => 0x0047, 328 => 0x0048, 329 => 0x0049, // KP 7-9

        // Navigation
        265 => 0xE048, 264 => 0xE050, 263 => 0xE04B, 262 => 0xE04D, // Up Down Left Right
        266 => 0xE049, 267 => 0xE051, // Page Up, Page Down
        268 => 0xE047, 269 => 0xE04F, // Home, End
        260 => 0xE052, 261 => 0xE053, // Insert, Delete

        _ => -1,
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
    last_capture: Instant,
    sibling_windows: Vec<Window>,
    // after initial capture period, we unmap all windows so tiling WMs
    // don't show them. periodically remap briefly to refresh the capture.
    went_offscreen_at: Option<Instant>,
    hidden: bool,
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
        tracing::info!(visible = self.visible, "toggled anchored app visibility");
    }

    /// Forward queued key events to all companion apps (anchored + detached) via stdin.
    /// Stdin is the primary key delivery mechanism — JNativeHook/XRecord is unreliable
    /// on XWayland for certain keys.
    pub fn forward_pending_keys(&self) {
        let all_pids: Vec<u32> = tuxinjector_gui::running_apps::list()
            .iter()
            .map(|a| a.pid)
            .collect();
        if all_pids.is_empty() { return; }

        let events: Vec<(u8, u16, i32)> = match APP_KEY_QUEUE.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => return,
        };
        if events.is_empty() { return; }

        for (keycode, mods, jnh_code) in &events {
            let line = format!("KEY {} {} {}\n", keycode, mods, jnh_code);
            for &pid in &all_pids {
                tuxinjector_gui::running_apps::write_stdin(pid, line.as_bytes());
            }
        }
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
            let siblings: Vec<Window> = all_wins[1..].to_vec();

            let mut unmapped = false;
            if let Some(conn) = self.conn.as_ref() {
                for &w in &all_wins {
                    let _ = conn.unmap_window(w);
                }
                let _ = conn.flush();
                unmapped = true;
            }

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
                    last_capture: Instant::now() - CAPTURE_INTERVAL,
                    sibling_windows: siblings,
                    went_offscreen_at: None,
                    hidden: false,
                },
            );
        } else {
            // periodically hunt for new sibling windows java might have spawned
            if self.frame % 120 == 0 {
                self.unmap_new_siblings(pid);
            }
            // re-enforce offscreen positioning - java swing loves to reposition
            // its own windows during init, which undoes our setup_offscreen
            if self.frame % 60 == 0 {
                self.enforce_offscreen(pid);
            }
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
                        e.went_offscreen_at = Some(Instant::now());
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

        // rate-limited pixel capture
        {
            let conn = self.conn.as_ref()?;
            let entry = self.embedded.get_mut(&pid)?;
            do_capture(conn, entry);
        }

        let entry = self.embedded.get(&pid)?;
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
        match x11rb::connect(None) {
            Ok((conn, screen)) => {
                self.conn = Some(conn);
                self.screen_num = screen;
                true
            }
            Err(e) => {
                tracing::warn!(%e, "failed to connect to X11");
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

    // after 3 seconds, kill the external window. the captured pixels
    // persist in the overlay and the java process stays alive.
    fn enforce_offscreen(&mut self, pid: u32) {
        let entry = match self.embedded.get_mut(&pid) {
            Some(e) if matches!(e.phase, CapturePhase::Offscreen) => e,
            _ => return,
        };

        if entry.hidden { return; }

        let age = entry.went_offscreen_at
            .map(|t| t.elapsed())
            .unwrap_or_default();

        if age.as_secs() >= 1 && entry.pixels.is_some() {
            tracing::info!(pid, "killing app windows (captured enough frames)");
            if let Some(conn) = self.conn.as_ref() {
                let _ = conn.destroy_window(entry.window);
                for &w in &entry.sibling_windows {
                    let _ = conn.destroy_window(w);
                }
                let _ = conn.flush();
            }
            entry.hidden = true;
        }
    }

    fn unmap_new_siblings(&mut self, pid: u32) {
        let capture_win = match self.embedded.get(&pid) {
            Some(e) => e.window,
            None => return,
        };
        let known: Vec<Window> = {
            let entry = self.embedded.get(&pid).unwrap();
            std::iter::once(capture_win)
                .chain(entry.sibling_windows.iter().copied())
                .collect()
        };

        let all_wins = if let Some(conn) = self.conn.as_ref() {
            find_all_via_xres(conn, self.screen_num, pid)
        } else {
            return;
        };

        let entry = match self.embedded.get_mut(&pid) {
            Some(e) => e,
            None => return,
        };

        for w in &all_wins {
            if !known.contains(w) {
                if let Some(conn) = self.conn.as_ref() {
                    // full offscreen treatment, not just unmap - java swing will
                    // remap its own windows if we only unmap them
                    let _ = setup_offscreen(conn, *w, false);
                }
                entry.sibling_windows.push(*w);
            }
        }
    }
}

fn do_capture(conn: &RustConnection, entry: &mut EmbeddedWindow) {
    let now = Instant::now();
    if now.duration_since(entry.last_capture) < CAPTURE_INTERVAL && entry.pixels.is_some() {
        return;
    }

    // after the window is killed, the last captured frame persists
    if entry.hidden { return; }

    let (w, h) = match win_size(conn, entry.window) {
        Some(wh) => wh,
        None => return,
    };
    if w == 0 || h == 0 {
        return;
    }

    match conn.get_image(ImageFormat::Z_PIXMAP, entry.window, 0, 0, w as u16, h as u16, !0) {
        Ok(cookie) => match cookie.reply() {
            Ok(reply) => {
                entry.pixels = Some(bgra_to_rgba(&reply.data, reply.depth));
                entry.width = w;
                entry.height = h;
                entry.last_capture = now;
            }
            Err(_) => {}
        },
        Err(_) => {}
    }
}

fn setup_offscreen(conn: &RustConnection, win: Window, already_unmapped: bool) -> X11Res<()> {
    if !already_unmapped {
        conn.unmap_window(win)?.check()?;
    }

    // override_redirect so the WM doesn't manage it
    conn.change_window_attributes(
        win,
        &ChangeWindowAttributesAux::new().override_redirect(1),
    )?.check()?;

    // set window type to UTILITY — tiling WMs (hyprland) auto-float these
    if let (Some(wm_type), Some(utility)) = (
        intern(conn, b"_NET_WM_WINDOW_TYPE"),
        intern(conn, b"_NET_WM_WINDOW_TYPE_UTILITY"),
    ) {
        let _ = conn.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            win, wm_type,
            x11rb::protocol::xproto::AtomEnum::ATOM,
            &[utility],
        );
    }

    // also tell WMs to skip taskbar/pager
    if let (Some(state), Some(skip_tb), Some(skip_pg)) = (
        intern(conn, b"_NET_WM_STATE"),
        intern(conn, b"_NET_WM_STATE_SKIP_TASKBAR"),
        intern(conn, b"_NET_WM_STATE_SKIP_PAGER"),
    ) {
        let _ = conn.change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            win, state,
            x11rb::protocol::xproto::AtomEnum::ATOM,
            &[skip_tb, skip_pg],
        );
    }

    conn.configure_window(
        win,
        &ConfigureWindowAux::new().x(-32000).y(-32000),
    )?.check()?;

    conn.map_window(win)?.check()?;
    conn.flush()?;
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
