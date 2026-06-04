// Bridges Minecraft's clipboard (e.g. the F3+C coordinate copy) into tux's
// private companion Xvfb. tux owns the CLIPBOARD/PRIMARY selections on that
// server and serves the latest text, so stock Ninjabrain Bot — which reads the
// X clipboard - receives the coordinates even though it lives on a different X
// server than the game. Fed by the glfwSetClipboardString hook.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, CreateWindowAux, EventMask, PropMode, SelectionNotifyEvent,
    WindowClass, SELECTION_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::CURRENT_TIME;

static PENDING: Mutex<Option<String>> = Mutex::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Publish new clipboard text to the companion X server. Called from the
/// glfwSetClipboardString hook when Minecraft copies (e.g. F3+C coordinates).
pub fn set_clipboard(text: String) {
    if let Ok(mut p) = PENDING.lock() {
        *p = Some(text);
    }
    ensure_started();
}

fn ensure_started() {
    if STARTED.load(Ordering::Acquire) {
        return;
    }
    // Need the Xvfb up first; a later set_clipboard call will start us.
    let display = match tuxinjector_gui::companion_xserver::display() {
        Some(d) => d,
        None => return,
    };
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::Builder::new()
        .name("companion-clipboard".into())
        .spawn(move || owner_loop(display))
        .ok();
}

fn intern(conn: &impl Connection, name: &[u8]) -> u32 {
    conn.intern_atom(false, name)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0)
}

fn owner_loop(dpy_num: u32) {
    let disp = format!(":{dpy_num}");
    let (conn, screen_num) = match x11rb::connect(Some(&disp)) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(%e, "companion clipboard: connect failed");
            return;
        }
    };
    let root = conn.setup().roots[screen_num].root;

    let owner = match conn.generate_id() {
        Ok(id) => id,
        Err(_) => return,
    };
    if conn
        .create_window(0, owner, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0,
            &CreateWindowAux::new())
        .is_err()
    {
        return;
    }

    let clipboard = intern(&conn, b"CLIPBOARD");
    let primary = u32::from(AtomEnum::PRIMARY);
    let utf8 = intern(&conn, b"UTF8_STRING");
    let targets = intern(&conn, b"TARGETS");
    let string_atom = u32::from(AtomEnum::STRING);
    let _ = conn.flush();
    tracing::info!("companion clipboard owner started on :{}", dpy_num);

    let mut current: Option<String> = None;

    loop {
        // Publish any newly-copied text by claiming the selections.
        if let Some(text) = PENDING.lock().ok().and_then(|mut p| p.take()) {
            current = Some(text);
            if clipboard != 0 {
                let _ = conn.set_selection_owner(owner, clipboard, CURRENT_TIME);
            }
            let _ = conn.set_selection_owner(owner, primary, CURRENT_TIME);
            let _ = conn.flush();
            tracing::debug!("companion clipboard: published selection");
        }

        // Service selection requests from NBB (reading the clipboard).
        while let Ok(Some(ev)) = conn.poll_for_event() {
            match ev {
                Event::SelectionRequest(req) => {
                    let prop = serve_request(
                        &conn, req.requestor, req.target, req.property,
                        current.as_deref(), utf8, string_atom, targets,
                    );
                    let notify = SelectionNotifyEvent {
                        response_type: SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: req.time,
                        requestor: req.requestor,
                        selection: req.selection,
                        target: req.target,
                        property: prop,
                    };
                    let _ = conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify);
                    let _ = conn.flush();
                }
                Event::SelectionClear(_) => {
                    current = None;
                }
                _ => {}
            }
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}

// Fulfils one SelectionRequest; returns the property to report in SelectionNotify
// (the request's property on success, 0 = refused).
fn serve_request(
    conn: &impl Connection,
    requestor: u32,
    target: u32,
    property: u32,
    text: Option<&str>,
    utf8: u32,
    string_atom: u32,
    targets: u32,
) -> u32 {
    // Obsolete clients send property = None; reply on the target atom instead.
    let property = if property == 0 { target } else { property };
    let text = match text {
        Some(t) => t,
        None => return 0,
    };

    if targets != 0 && target == targets {
        let list = [targets, utf8, string_atom];
        if conn
            .change_property32(PropMode::REPLACE, requestor, property,
                u32::from(AtomEnum::ATOM), &list)
            .is_ok()
        {
            return property;
        }
        return 0;
    }

    if (utf8 != 0 && target == utf8) || target == string_atom {
        if conn
            .change_property8(PropMode::REPLACE, requestor, property, target, text.as_bytes())
            .is_ok()
        {
            return property;
        }
    }

    0
}
