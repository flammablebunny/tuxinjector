// tuxinjector-browser: offscreen web renderer using WebKitGTK

use gtk::prelude::*;
use webkit2gtk::WebViewExt;

use std::cell::RefCell;
use std::io::{self, BufRead, Write};
use std::rc::Rc;

#[derive(serde::Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Command {
    Navigate { url: String },
    Resize { width: i32, height: i32 },
    InjectCss { css: String },
    SetFps { fps: i32 },
    Close,
}

struct State {
    webview: webkit2gtk::WebView,
    width: i32,
    height: i32,
    fps: i32,
    timer_id: Option<glib::SourceId>,
}

fn main() {
    // force X11/XWayland so we can position the window offscreen.
    // wayland compositors clamp positions to visible area.
    std::env::set_var("GDK_BACKEND", "x11");

    // NixOS: GIO needs glib-networking for TLS/HTTPS
    if std::env::var("GIO_EXTRA_MODULES").is_err() {
        if let Some(path) = find_gio_modules() {
            std::env::set_var("GIO_EXTRA_MODULES", &path);
            eprintln!("[tuxinjector-browser] set GIO_EXTRA_MODULES={path}");
        }
    }

    if gtk::init().is_err() {
        eprintln!("[tuxinjector-browser] failed to init GTK");
        std::process::exit(1);
    }

    // Popup = override_redirect on X11, WM won't tile/manage it
    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_default_size(800, 600);
    window.move_(-32000, -32000);

    let webview = webkit2gtk::WebView::new();
    webview.set_size_request(800, 600);
    webview.set_hexpand(true);
    webview.set_vexpand(true);

    // transparent background
    let bg = gdk::RGBA::new(0.0, 0.0, 0.0, 0.0);
    WebViewExt::set_background_color(&webview, &bg);

    if let Some(settings) = WebViewExt::settings(&webview) {
        use webkit2gtk::SettingsExt;
        settings.set_enable_javascript(true);
        settings.set_enable_webgl(true);
    }

    window.add(&webview);
    window.show_all();

    let state = Rc::new(RefCell::new(State {
        webview: webview.clone(),
        width: 800,
        height: 600,
        fps: 15,
        timer_id: None,
    }));

    schedule_capture(&state);

    let (tx, rx) = glib::MainContext::channel::<Command>(glib::Priority::DEFAULT);

    std::thread::Builder::new()
        .name("stdin-reader".into())
        .spawn(move || {
            let stdin = io::stdin();
            for line in stdin.lock().lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                if line.trim().is_empty() { continue; }
                match serde_json::from_str::<Command>(&line) {
                    Ok(cmd) => {
                        let is_close = matches!(cmd, Command::Close);
                        let _ = tx.send(cmd);
                        if is_close { break; }
                    }
                    Err(e) => eprintln!("[tuxinjector-browser] bad command: {e}"),
                }
            }
            let _ = tx.send(Command::Close);
        })
        .expect("failed to spawn stdin reader");

    let state2 = state.clone();
    let window2 = window.clone();
    rx.attach(None, move |cmd: Command| {
        match cmd {
            Command::Navigate { url } => {
                let s = state2.borrow();
                WebViewExt::load_uri(&s.webview, &url);
            }
            Command::Resize { width, height } => {
                let mut s = state2.borrow_mut();
                s.width = width;
                s.height = height;
                window2.set_default_size(width, height);
                window2.resize(width, height);
                s.webview.set_size_request(width, height);
            }
            Command::InjectCss { css } => {
                let s = state2.borrow();
                inject_css_into(&s.webview, &css);
            }
            Command::SetFps { fps } => {
                state2.borrow_mut().fps = fps.max(1);
                schedule_capture(&state2);
            }
            Command::Close => {
                gtk::main_quit();
            }
        }
        glib::ControlFlow::Continue
    });

    gtk::main();
}

fn inject_css_into(webview: &webkit2gtk::WebView, css: &str) {
    let escaped = css.replace('\\', "\\\\").replace('`', "\\`");
    let js = format!(
        r#"(function() {{
            let s = document.getElementById('__tuxinjector_css');
            if (!s) {{ s = document.createElement('style'); s.id = '__tuxinjector_css'; document.head.appendChild(s); }}
            s.textContent = `{escaped}`;
        }})()"#
    );
    WebViewExt::run_javascript(webview, &js, None::<&gio::Cancellable>, |_| {});
}

fn schedule_capture(state: &Rc<RefCell<State>>) {
    let mut s = state.borrow_mut();
    if let Some(id) = s.timer_id.take() {
        glib::SourceId::remove(id);
    }
    let interval_ms = (1000 / s.fps.max(1)) as u64;
    let st = state.clone();
    s.timer_id = Some(glib::timeout_add_local(
        std::time::Duration::from_millis(interval_ms),
        move || {
            do_capture(&st);
            glib::ControlFlow::Continue
        },
    ));
}

fn do_capture(state: &Rc<RefCell<State>>) {
    let s = state.borrow();
    let w = s.width;
    let h = s.height;
    let wv = s.webview.clone();
    drop(s);

    WebViewExt::snapshot(
        &wv,
        webkit2gtk::SnapshotRegion::FullDocument,
        webkit2gtk::SnapshotOptions::TRANSPARENT_BACKGROUND,
        None::<&gio::Cancellable>,
        move |result| {
            let surface = match result {
                Ok(s) => s,
                Err(_) => return,
            };

            // blit snapshot onto an ImageSurface so we can read raw pixels
            let mut img = match cairo::ImageSurface::create(cairo::Format::ARgb32, w, h) {
                Ok(i) => i,
                Err(_) => return,
            };
            {
                let cr = match cairo::Context::new(&img) {
                    Ok(cr) => cr,
                    Err(_) => return,
                };
                cr.set_source_surface(&surface, 0.0, 0.0).ok();
                cr.paint().ok();
            }

            let stride = img.stride() as usize;
            let sw = img.width() as usize;
            let sh = img.height() as usize;
            let out_w = sw.min(w as usize);
            let out_h = sh.min(h as usize);

            let data = match img.data() {
                Ok(d) => d,
                Err(_) => return,
            };

            // cairo BGRA premultiplied -> RGBA straight
            let mut rgba = Vec::with_capacity(out_w * out_h * 4);
            for y in 0..out_h {
                let off = y * stride;
                for x in 0..out_w {
                    let i = off + x * 4;
                    let (b, g, r, a) = (data[i], data[i + 1], data[i + 2], data[i + 3]);
                    if a == 0 {
                        rgba.extend_from_slice(&[0, 0, 0, 0]);
                    } else if a == 255 {
                        rgba.extend_from_slice(&[r, g, b, a]);
                    } else {
                        let af = a as f32 / 255.0;
                        rgba.push((r as f32 / af).min(255.0) as u8);
                        rgba.push((g as f32 / af).min(255.0) as u8);
                        rgba.push((b as f32 / af).min(255.0) as u8);
                        rgba.push(a);
                    }
                }
            }

            let stdout = io::stdout();
            let mut out = stdout.lock();
            let _ = out.write_all(&(out_w as u32).to_le_bytes());
            let _ = out.write_all(&(out_h as u32).to_le_bytes());
            let _ = out.write_all(&(rgba.len() as u32).to_le_bytes());
            let _ = out.write_all(&rgba);
            let _ = out.flush();
        },
    );
}

fn find_gio_modules() -> Option<String> {
    // check system profile first (most common on NixOS)
    let sys = "/run/current-system/sw/lib/gio/modules";
    if std::path::Path::new(sys).join("libgiognutls.so").exists() {
        return Some(sys.to_string());
    }
    // user profile
    if let Ok(home) = std::env::var("HOME") {
        let user = format!("{home}/.nix-profile/lib/gio/modules");
        if std::path::Path::new(&user).join("libgiognutls.so").exists() {
            return Some(user);
        }
    }
    // scan /nix/store for glib-networking
    if let Ok(entries) = std::fs::read_dir("/nix/store") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().contains("glib-networking") && !name.to_string_lossy().contains(".drv") {
                let p = entry.path().join("lib/gio/modules");
                if p.join("libgiognutls.so").exists() {
                    return Some(p.to_string_lossy().to_string());
                }
            }
        }
    }
    None
}
