// D-Bus portal dance for ScreenCast session setup.
// Talks to org.freedesktop.portal.ScreenCast to get a PipeWire node ID
// for whatever window the user picks. Supports persist tokens so the
// picker can be skipped on subsequent launches.

use std::collections::HashMap;
use std::time::Duration;

use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::Connection;
use dbus::message::MatchRule;
use dbus::Message;

const PORTAL_BUS: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST_IFACE: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST_IFACE: &str = "org.freedesktop.portal.Request";

pub struct PortalSession {
    pub node_id: u32,
    #[allow(dead_code)] // kept alive for the portal session handle
    pub session_path: String,
    pub restore_token: Option<String>,
}

#[derive(Debug)]
pub enum PortalError {
    Dbus(dbus::Error),
    Denied,
    NoStreams,
    BadResponse(String),
}

impl std::fmt::Display for PortalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortalError::Dbus(e) => write!(f, "D-Bus: {e}"),
            PortalError::Denied => write!(f, "user denied capture"),
            PortalError::NoStreams => write!(f, "no streams in response"),
            PortalError::BadResponse(msg) => write!(f, "bad response: {msg}"),
        }
    }
}

impl std::error::Error for PortalError {}

impl From<dbus::Error> for PortalError {
    fn from(e: dbus::Error) -> Self {
        PortalError::Dbus(e)
    }
}

// monotonic token so the portal can match our requests back to us
fn request_token() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!("tuxinjector{}", N.fetch_add(1, Ordering::Relaxed))
}

// dbus unique names look like ":1.42" but the portal wants underscores
fn sender_slug(conn: &Connection) -> String {
    let name = conn.unique_name().to_string();
    name.replace('.', "_").replace(':', "")
}

fn wait_for_signal(
    conn: &Connection,
    rx: &std::sync::mpsc::Receiver<(u32, PropMap)>,
    timeout: Duration,
) -> Result<(u32, PropMap), PortalError> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .unwrap_or(Duration::ZERO);
        if remaining.is_zero() {
            return Err(PortalError::BadResponse("timed out".into()));
        }
        // pump dbus until our signal shows up
        conn.process(Duration::from_millis(100))?;
        if let Ok(result) = rx.try_recv() {
            return Ok(result);
        }
    }
}

fn setup_response_listener(
    conn: &Connection,
    req_path: &str,
) -> Result<std::sync::mpsc::Receiver<(u32, PropMap)>, PortalError> {
    let (tx, rx) = std::sync::mpsc::channel();

    let rule = MatchRule::new_signal(REQUEST_IFACE, "Response")
        .with_path(req_path)
        .static_clone();

    conn.add_match(rule, move |_: (), _conn, msg: &Message| {
        if let Ok((code, results)) = msg.read2::<u32, PropMap>() {
            let _ = tx.send((code, results));
        }
        false
    })
    .map_err(PortalError::Dbus)?;

    Ok(rx)
}

// Three round-trips with the portal: CreateSession -> SelectSources -> Start.
// Verbose but that's the protocol.
pub fn start_screencast(
    restore_token: Option<&str>,
) -> Result<PortalSession, PortalError> {
    let conn = Connection::new_session()?;
    let sender = sender_slug(&conn);
    let proxy = conn.with_proxy(PORTAL_BUS, PORTAL_PATH, Duration::from_secs(10));

    // --- step 1: CreateSession ---
    let tok = request_token();
    let sess_tok = request_token();
    let req_path = format!(
        "/org/freedesktop/portal/desktop/request/{sender}/{tok}"
    );

    let rx = setup_response_listener(&conn, &req_path)?;

    let mut opts: PropMap = HashMap::new();
    opts.insert("handle_token".into(), Variant(Box::new(tok)));
    opts.insert("session_handle_token".into(), Variant(Box::new(sess_tok)));

    let _: (dbus::Path,) = proxy.method_call(
        SCREENCAST_IFACE,
        "CreateSession",
        (opts,),
    )?;

    let (code, results) = wait_for_signal(&conn, &rx, Duration::from_secs(30))?;
    if code != 0 {
        return Err(PortalError::Denied);
    }

    let session_handle = results
        .get("session_handle")
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| PortalError::BadResponse("no session_handle".into()))?;

    tracing::debug!(session = %session_handle, "portal session created");

    // --- step 2: SelectSources ---
    let tok = request_token();
    let req_path = format!(
        "/org/freedesktop/portal/desktop/request/{sender}/{tok}"
    );

    let rx = setup_response_listener(&conn, &req_path)?;

    let mut opts: PropMap = HashMap::new();
    opts.insert("handle_token".into(), Variant(Box::new(tok)));
    opts.insert("types".into(), Variant(Box::new(2u32))); // Window
    opts.insert("persist_mode".into(), Variant(Box::new(2u32))); // persist across restarts

    if let Some(token) = restore_token {
        opts.insert("restore_token".into(), Variant(Box::new(token.to_string())));
    }

    let session_path = dbus::Path::from(session_handle.as_str());
    let _: (dbus::Path,) = proxy.method_call(
        SCREENCAST_IFACE,
        "SelectSources",
        (&session_path, opts),
    )?;

    let (code, _) = wait_for_signal(&conn, &rx, Duration::from_secs(30))?;
    if code != 0 {
        return Err(PortalError::Denied);
    }

    tracing::debug!("portal sources selected");

    // --- step 3: Start ---
    let tok = request_token();
    let req_path = format!(
        "/org/freedesktop/portal/desktop/request/{sender}/{tok}"
    );

    let rx = setup_response_listener(&conn, &req_path)?;

    let mut opts: PropMap = HashMap::new();
    opts.insert("handle_token".into(), Variant(Box::new(tok)));

    let _: (dbus::Path,) = proxy.method_call(
        SCREENCAST_IFACE,
        "Start",
        (&session_path, "", opts),
    )?;

    // longer timeout here - user might take a while picking a window
    let (code, results) = wait_for_signal(&conn, &rx, Duration::from_secs(120))?;
    if code != 0 {
        return Err(PortalError::Denied);
    }

    let node_id = extract_node_id(&results)?;

    let restore_token = results
        .get("restore_token")
        .and_then(|v| v.as_str().map(|s| s.to_string()));

    tracing::info!(node_id, ?restore_token, "portal screencast started");

    Ok(PortalSession {
        node_id,
        session_path: session_handle,
        restore_token,
    })
}

// The streams field is annoyingly nested - portal spec says it's an array
// of (node_id, properties) structs but dbus-rs gives us generic iterators,
// so we just dig until we find something numeric.
fn extract_node_id(results: &PropMap) -> Result<u32, PortalError> {
    let streams = results.get("streams").ok_or(PortalError::NoStreams)?;

    if let Some(iter) = streams.as_iter() {
        for item in iter {
            if let Some(mut inner) = item.as_iter() {
                if let Some(node_ref) = inner.next() {
                    if let Some(id) = node_ref.as_u64() {
                        return Ok(id as u32);
                    }
                    if let Some(id) = node_ref.as_i64() {
                        return Ok(id as u32);
                    }
                }
            }
        }
    }

    Err(PortalError::BadResponse("cant extract node_id".into()))
}
