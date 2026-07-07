// Ninjabrain Bot SSE client

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::Value;

use crate::nbb_data::{self, NinjabrainData};

const RECONNECT_MS: u64 = 200;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(200);
// NBB can sit idle for a while between events, so keep this generous. A timeout
// here just means "quietly reconnect", not a real error.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

const STREAMS: [(&str, &str); 4] = [
    ("stronghold", "/api/v1/stronghold/events"),
    ("info", "/api/v1/information-messages/events"),
    ("boat", "/api/v1/boat/events"),
    ("blind", "/api/v1/blind/events"),
];

// Status the GUI polls to draw the connection indicator.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NbbState {
    Stopped,
    Connecting,
    Connected,
    Offline,
}

#[derive(Clone)]
pub struct NbbClientStatus {
    pub state: NbbState,
    pub api_base_url: String,
    pub last_error: String,
}

#[derive(Default)]
struct StatusInner {
    running: bool,
    api_base_url: String,
    connected: [bool; 4],
    last_error: String,
}

fn status() -> &'static Mutex<StatusInner> {
    static S: OnceLock<Mutex<StatusInner>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(StatusInner::default()))
}

pub fn get_status() -> NbbClientStatus {
    let s = status().lock().unwrap();
    let state = if !s.running {
        NbbState::Stopped
    } else if s.connected.iter().any(|&c| c) {
        NbbState::Connected
    } else if !s.last_error.is_empty() {
        NbbState::Offline
    } else {
        NbbState::Connecting
    };
    NbbClientStatus {
        state,
        api_base_url: s.api_base_url.clone(),
        last_error: s.last_error.clone(),
    }
}

// Bumped every start/stop. Threads compare against it so a stale generation
// stops touching shared state once a newer session has taken over.
static GENERATION: AtomicU64 = AtomicU64::new(0);

struct Session {
    generation: u64,
    stop: Arc<AtomicBool>,
    sockets: Arc<Mutex<Vec<TcpStream>>>,
}

fn session() -> &'static Mutex<Option<Session>> {
    static S: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

fn is_active(generation: u64) -> bool {
    GENERATION.load(Ordering::Acquire) == generation
}

pub fn normalize_base_url(url: &str) -> String {
    let t = url.trim().trim_end_matches('/');
    if t.is_empty() {
        "http://127.0.0.1:52533".to_string()
    } else {
        t.to_string()
    }
}

/// "http://host:port" -> (host, port). Defaults to port 80 if none given.
fn parse_base_url(base: &str) -> Option<(String, u16)> {
    let rest = base.strip_prefix("http://").unwrap_or(base);
    let rest = rest.split('/').next()?;
    let (host, port) = match rest.rsplit_once(':') {
        Some((h, p)) => (h.to_string(), p.parse().ok()?),
        None => (rest.to_string(), 80),
    };
    if host.is_empty() { None } else { Some((host, port)) }
}

pub fn start(api_base_url: &str) {
    stop();
    let base = normalize_base_url(api_base_url);
    let generation = GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    nbb_data::reset_nbb_data();
    {
        let mut s = status().lock().unwrap();
        *s = StatusInner { running: true, api_base_url: base.clone(), ..Default::default() };
    }
    let stop_flag = Arc::new(AtomicBool::new(false));
    let sockets: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));

    for (idx, (name, path)) in STREAMS.iter().enumerate() {
        let base = base.clone();
        let stop_flag = Arc::clone(&stop_flag);
        let sockets = Arc::clone(&sockets);
        let path = path.to_string();
        let name = name.to_string();
        let _ = std::thread::Builder::new()
            .name(format!("nbb-{name}"))
            .spawn(move || stream_loop(generation, idx, &name, &base, &path, stop_flag, sockets));
    }

    *session().lock().unwrap() = Some(Session { generation, stop: stop_flag, sockets });
    tracing::info!(base, "nbb client started");
}

pub fn stop() {
    if let Some(s) = session().lock().unwrap().take() {
        s.stop.store(true, Ordering::Release);
        for sock in s.sockets.lock().unwrap().drain(..) {
            let _ = sock.shutdown(std::net::Shutdown::Both);
        }
        tracing::info!(generation = s.generation, "nbb client stopped");
    }
    GENERATION.fetch_add(1, Ordering::AcqRel);
    let mut s = status().lock().unwrap();
    s.running = false;
    s.connected = [false; 4];
}

pub fn restart(api_base_url: &str) {
    start(api_base_url);
}

/// Drive lifecycle from the current config; call on start + config publish.
pub fn apply_config(enabled: bool, api_base_url: &str) {
    let (running, current_url) = {
        let s = status().lock().unwrap();
        (s.running, s.api_base_url.clone())
    };
    let want_url = normalize_base_url(api_base_url);
    match (enabled, running) {
        (true, false) => start(&want_url),
        (false, true) => stop(),
        (true, true) if current_url != want_url => restart(&want_url),
        _ => {}
    }
}

// --- per-stream state helpers ---

fn mark_connected(generation: u64, idx: usize) {
    if !is_active(generation) {
        return;
    }
    let mut s = status().lock().unwrap();
    s.connected[idx] = true;
    // error clears only when all four streams are up
    if s.connected.iter().all(|&c| c) {
        s.last_error.clear();
    }
}

fn mark_disconnected(generation: u64, idx: usize, error: &str) {
    if !is_active(generation) {
        return;
    }
    let mut s = status().lock().unwrap();
    s.connected[idx] = false;
    s.last_error = error.to_string();
}

fn clear_stream_data(generation: u64, idx: usize) {
    if !is_active(generation) {
        return;
    }
    nbb_data::modify_nbb_data(|d| match idx {
        0 => nbb_data::clear_stronghold(d),
        1 => nbb_data::clear_info_messages(d),
        2 => nbb_data::clear_boat(d),
        _ => nbb_data::clear_blind(d),
    });
}

fn dispatch_event(generation: u64, idx: usize, data: &str) {
    if !is_active(generation) {
        return;
    }
    let json: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(stream = idx, %e, "nbb: bad event json, dropped");
            return;
        }
    };
    let apply = |d: &mut NinjabrainData| match idx {
        0 => nbb_data::apply_stronghold_event(d, &json),
        1 => nbb_data::apply_info_messages_event(d, &json),
        2 => nbb_data::apply_boat_event(d, &json),
        _ => nbb_data::apply_blind_event(d, &json),
    };
    nbb_data::modify_nbb_data(apply);
}

// --- SSE framing (pure, unit-tested) ---

#[derive(Default)]
pub struct SseAccumulator {
    data: Vec<String>,
}

impl SseAccumulator {
    /// Feed one line (no trailing newline). Returns a complete event's data
    /// on the blank-line terminator.
    pub fn feed_line(&mut self, line: &str) -> Option<String> {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            if self.data.is_empty() {
                return None;
            }
            return Some(std::mem::take(&mut self.data).join("\n"));
        }
        if let Some(rest) = line.strip_prefix("data:") {
            self.data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        }
        // event:/id:/retry:/comment lines ignored
        None
    }
}

// --- stream thread ---

fn stream_loop(
    generation: u64,
    idx: usize,
    name: &str,
    base: &str,
    path: &str,
    stop_flag: Arc<AtomicBool>,
    sockets: Arc<Mutex<Vec<TcpStream>>>,
) {
    let Some((host, port)) = parse_base_url(base) else {
        mark_disconnected(generation, idx, &format!("invalid API base url: {base}"));
        return;
    };
    let mut was_connected = false;

    while !stop_flag.load(Ordering::Acquire) && is_active(generation) {
        match connect_and_read(
            generation, idx, &host, port, base, path, &stop_flag, &sockets, &mut was_connected,
        ) {
            Ok(()) => {} // clean EOF -> reconnect
            Err(e) => {
                if stop_flag.load(Ordering::Acquire) || !is_active(generation) {
                    break;
                }
                if was_connected {
                    tracing::debug!(stream = name, %e, "nbb stream dropped");
                    clear_stream_data(generation, idx);
                    was_connected = false;
                }
                mark_disconnected(generation, idx, &e);
            }
        }
        std::thread::sleep(Duration::from_millis(RECONNECT_MS));
    }
}

#[allow(clippy::too_many_arguments)]
fn connect_and_read(
    generation: u64,
    idx: usize,
    host: &str,
    port: u16,
    base: &str,
    path: &str,
    stop_flag: &AtomicBool,
    sockets: &Mutex<Vec<TcpStream>>,
    was_connected: &mut bool,
) -> Result<(), String> {
    let addr = format!("{host}:{port}");
    let sock_addr = addr
        .parse()
        .or_else(|_| {
            use std::net::ToSocketAddrs;
            addr.to_socket_addrs()
                .map_err(|e| e.to_string())?
                .next()
                .ok_or_else(|| "no address".to_string())
        })
        .map_err(|e: String| format!("resolve {addr}: {e}"))?;

    let mut stream = TcpStream::connect_timeout(&sock_addr, CONNECT_TIMEOUT)
        .map_err(|e| format!("connect {base}: {e}"))?;
    stream.set_read_timeout(Some(READ_TIMEOUT)).ok();
    sockets.lock().unwrap().push(stream.try_clone().map_err(|e| e.to_string())?);

    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nAccept: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);

    // status line + headers; note chunked transfer coding
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    if !line.contains("200") {
        return Err(format!("HTTP error: {}", line.trim()));
    }
    let mut chunked = false;
    loop {
        line.clear();
        reader.read_line(&mut line).map_err(|e| e.to_string())?;
        let l = line.trim();
        if l.is_empty() {
            break;
        }
        if l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked") {
            chunked = true;
        }
    }

    mark_connected(generation, idx);
    *was_connected = true;

    let mut acc = SseAccumulator::default();
    let mut feed = |payload: &str| {
        for l in payload.split('\n') {
            if let Some(event) = acc.feed_line(l) {
                dispatch_event(generation, idx, &event);
            }
        }
    };

    if chunked {
        loop {
            if stop_flag.load(Ordering::Acquire) || !is_active(generation) {
                return Ok(());
            }
            line.clear();
            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("EOF".to_string());
            }
            let size = usize::from_str_radix(line.trim(), 16).map_err(|_| "bad chunk size")?;
            if size == 0 {
                return Err("stream ended".to_string());
            }
            let mut buf = vec![0u8; size + 2]; // chunk + CRLF
            std::io::Read::read_exact(&mut reader, &mut buf).map_err(|e| e.to_string())?;
            feed(&String::from_utf8_lossy(&buf[..size]));
        }
    } else {
        loop {
            if stop_flag.load(Ordering::Acquire) || !is_active(generation) {
                return Ok(());
            }
            line.clear();
            if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("EOF".to_string());
            }
            if let Some(event) = acc.feed_line(line.trim_end_matches('\n')) {
                dispatch_event(generation, idx, &event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_framing() {
        let mut a = SseAccumulator::default();
        assert_eq!(a.feed_line("event: message"), None);
        assert_eq!(a.feed_line("data: {\"a\":1}"), None);
        assert_eq!(a.feed_line(""), Some("{\"a\":1}".to_string()));
        // multi-line data joins with \n
        assert_eq!(a.feed_line("data: line1"), None);
        assert_eq!(a.feed_line("data:line2"), None);
        assert_eq!(a.feed_line("\r"), Some("line1\nline2".to_string()));
        // comments + blank without data emit nothing
        assert_eq!(a.feed_line(": keepalive"), None);
        assert_eq!(a.feed_line(""), None);
        // CRLF handling
        assert_eq!(a.feed_line("data: x\r"), None);
        assert_eq!(a.feed_line(""), Some("x".to_string()));
    }

    #[test]
    fn base_url_parsing() {
        assert_eq!(normalize_base_url("  http://127.0.0.1:52533/ "), "http://127.0.0.1:52533");
        assert_eq!(normalize_base_url(""), "http://127.0.0.1:52533");
        assert_eq!(parse_base_url("http://127.0.0.1:52533"), Some(("127.0.0.1".into(), 52533)));
        assert_eq!(parse_base_url("http://localhost"), Some(("localhost".into(), 80)));
        assert_eq!(parse_base_url("http://"), None);
    }
}
