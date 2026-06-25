// tuxinjector self-updater (Linux + macOS).

use std::ffi::{c_char, c_void, CStr, CString};
use std::io::Read;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

const OWNER: &str = "flammablebunny";
const REPO: &str = "tuxinjector";

// Phase, lock-free so the GUI can poll it every frame.
const PHASE_CHECKING: u8 = 0;
const PHASE_UP_TO_DATE: u8 = 1;
const PHASE_AVAILABLE: u8 = 2;
const PHASE_DOWNLOADING: u8 = 3;
const PHASE_INSTALLED: u8 = 4;
const PHASE_FAILED: u8 = 5;

static PHASE: AtomicU8 = AtomicU8::new(PHASE_CHECKING);

static POPUP_AVAIL_DISMISSED: AtomicBool = AtomicBool::new(false);
static POPUP_DONE_DISMISSED: AtomicBool = AtomicBool::new(false);
static POPUP_INSTALL_STARTED: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct UpdateInfo {
    current: String,
    latest: String,
    download_url: String,
    error: String,
}

static INFO: Mutex<Option<UpdateInfo>> = Mutex::new(None);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installed,
    Failed,
}

fn set_info(f: impl FnOnce(&mut UpdateInfo)) {
    let mut g = INFO.lock().unwrap_or_else(|p| p.into_inner());
    f(g.get_or_insert_with(UpdateInfo::default));
}

/// Snapshot of the current update state, for the GUI to render.
pub fn phase() -> UpdatePhase {
    match PHASE.load(Ordering::Acquire) {
        PHASE_UP_TO_DATE => UpdatePhase::UpToDate,
        PHASE_AVAILABLE => UpdatePhase::Available,
        PHASE_DOWNLOADING => UpdatePhase::Downloading,
        PHASE_INSTALLED => UpdatePhase::Installed,
        PHASE_FAILED => UpdatePhase::Failed,
        _ => UpdatePhase::Checking,
    }
}

pub fn info() -> (String, String, String) {
    INFO.lock()
        .ok()
        .and_then(|g| {
            g.as_ref()
                .map(|i| (i.current.clone(), i.latest.clone(), i.error.clone()))
        })
        .unwrap_or_default()
}

/// Whether the launch popup wants to be on screen. Drives both the renderer's
/// "don't skip drawing" decision and whether clicks get routed to it. The popup
/// is the over-the-game alert shown even when the settings GUI is closed.
pub fn popup_should_show() -> bool {
    match phase() {
        UpdatePhase::Available | UpdatePhase::Downloading => {
            !POPUP_AVAIL_DISMISSED.load(Ordering::Relaxed)
        }
        UpdatePhase::Installed => !POPUP_DONE_DISMISSED.load(Ordering::Relaxed),
        // Only surface a failure here if the user actually started an install from
        // the popup -- a quiet background check failure stays in the tab.
        UpdatePhase::Failed => {
            POPUP_INSTALL_STARTED.load(Ordering::Relaxed)
                && !POPUP_AVAIL_DISMISSED.load(Ordering::Relaxed)
        }
        _ => false,
    }
}

pub fn render_popup(ui: &imgui::Ui) {
    use imgui::Condition;

    const AMBER: [f32; 4] = [1.0, 0.784, 0.196, 1.0];
    const GREEN: [f32; 4] = [0.502, 0.804, 0.502, 1.0];
    const GREY: [f32; 4] = [0.7, 0.7, 0.7, 1.0];
    const RED: [f32; 4] = [0.902, 0.412, 0.412, 1.0];

    let ph = phase();
    let (_, latest, error) = info();
    let [dw, _] = ui.io().display_size;

    // Fixed ID (###) so the window keeps its identity as the label changes.
    let title = match ph {
        UpdatePhase::Installed => "Update ready\u{200b}###tux_update_popup",
        UpdatePhase::Failed => "Update failed\u{200b}###tux_update_popup",
        _ => "Update available\u{200b}###tux_update_popup",
    };

    let mut shown = true;
    let mut dismiss = false;

    ui.window(title)
        .opened(&mut shown)
        .title_bar(true)
        .collapsible(false)
        .resizable(false)
        .movable(false)
        .scroll_bar(false)
        .always_auto_resize(true)
        .position([dw / 2.0, 36.0], Condition::Always)
        .position_pivot([0.5, 0.0])
        .build(|| match ph {
            UpdatePhase::Available => {
                ui.text("A new version of tuxinjector is available.");
                ui.text_colored(AMBER, format!("Update to {latest}"));
                ui.dummy([0.0, 6.0]);
                if ui.button("Update") {
                    POPUP_INSTALL_STARTED.store(true, Ordering::Relaxed);
                    install();
                }
                ui.same_line();
                if ui.button("Close") {
                    dismiss = true;
                }
            }
            UpdatePhase::Downloading => {
                ui.text_colored(AMBER, "Downloading update\u{2026}");
            }
            UpdatePhase::Installed => {
                ui.text_colored(GREEN, format!("tuxinjector {latest} installed."));
                ui.text("Restart your instance to apply the update.");
                ui.dummy([0.0, 6.0]);
                if ui.button("Restart Now") {
                    restart_now();
                }
                ui.same_line();
                if ui.button("Later") {
                    dismiss = true;
                }
            }
            UpdatePhase::Failed => {
                ui.text_colored(RED, "The update failed.");
                if !error.is_empty() {
                    ui.text_colored(GREY, &error);
                }
                ui.dummy([0.0, 6.0]);
                if ui.button("Close") {
                    dismiss = true;
                }
            }
            _ => {}
        });

    // Title-bar X (!shown) or a Close/Later button dismisses the active card.
    if !shown || dismiss {
        match ph {
            UpdatePhase::Installed => POPUP_DONE_DISMISSED.store(true, Ordering::Relaxed),
            _ => POPUP_AVAIL_DISMISSED.store(true, Ordering::Relaxed),
        }
    }
}

// "1.0.10" / "v1.0.10" -> [1, 0, 10]. Trailing junk on a component is dropped.
fn parse_ver(s: &str) -> Vec<u32> {
    s.trim()
        .trim_start_matches(|c: char| !c.is_ascii_digit())
        .split('.')
        .map(|p| {
            p.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse::<u32>().ok())
        .collect()
}

// Vec<u32> compares lexicographically, which is exactly semver-ish ordering.
fn is_newer(latest: &str, current: &str) -> bool {
    parse_ver(latest) > parse_ver(current)
}

// Release asset to fetch. On Linux this matches the architecture THIS build was
// compiled for (not the host's `uname` -- the replacement must match the binary
// we're swapping out); None on an arch we don't ship, so we refuse rather than
// install a mismatched binary. On macOS there's one universal dylib for all Mace (Silicon and Intel).
fn asset_name() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    let name = match std::env::consts::ARCH {
        "x86_64" => "tuxinjector_x64.so",
        "aarch64" => "tuxinjector_aarch64.so",
        "x86" => "tuxinjector_x86.so",
        "arm" => "tuxinjector_aarch32.so",
        _ => return None,
    };
    #[cfg(target_os = "macos")]
    let name = "tuxinjector.dylib";

    Some(name)
}

/// Kick off the background version check. `current_version` is the running
/// build's version (the core crate passes its own CARGO_PKG_VERSION).
pub fn spawn_update_check(current_version: &str) {
    let current = current_version.to_string();
    let _ = std::thread::Builder::new()
        .name("tux-update-check".into())
        .spawn(move || run_check(current));
}

/// Re-run the check (the GUI's "Check" / "Retry" button).
pub fn recheck() {
    let current = INFO
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|i| i.current.clone()))
        .unwrap_or_default();
    if current.is_empty() {
        return;
    }
    // A fresh check should be allowed to re-surface the popup.
    POPUP_AVAIL_DISMISSED.store(false, Ordering::Relaxed);
    POPUP_DONE_DISMISSED.store(false, Ordering::Relaxed);
    POPUP_INSTALL_STARTED.store(false, Ordering::Relaxed);
    PHASE.store(PHASE_CHECKING, Ordering::Release);
    spawn_update_check(&current);
}

fn run_check(current: String) {
    set_info(|i| i.current = current.clone());
    match fetch_latest() {
        // A release exists -- compare and either offer it or report up to date.
        Ok(Some((latest, url))) => {
            set_info(|i| {
                i.latest = latest.clone();
                i.download_url = url;
                i.error.clear();
            });
            PHASE.store(
                if is_newer(&latest, &current) {
                    PHASE_AVAILABLE
                } else {
                    PHASE_UP_TO_DATE
                },
                Ordering::Release,
            );
        }
        // No releases published yet (404): we're as current as anything out there.
        Ok(None) => {
            set_info(|i| i.error.clear());
            PHASE.store(PHASE_UP_TO_DATE, Ordering::Release);
        }
        Err(e) => {
            // Real network/API failure -- surface it quietly with a Retry rather
            // than pretend we're up to date.
            tracing::debug!("updater: check failed: {e}");
            set_info(|i| i.error = e);
            PHASE.store(PHASE_FAILED, Ordering::Release);
        }
    }
}

// Ok(Some((tag, url))) = a release exists for our arch; Ok(None) = repo has no
// releases yet (treat as up to date); Err = a real failure worth retrying.
fn fetch_latest() -> Result<Option<(String, String)>, String> {
    let want = asset_name().ok_or_else(|| {
        format!("no tuxinjector build for this architecture ({})", std::env::consts::ARCH)
    })?;

    let api = format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/latest");
    let resp = match ureq::get(&api)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "tuxinjector")
        .call()
    {
        Ok(r) => r,
        // GitHub returns 404 for "no published release", which isn't an error.
        Err(ureq::Error::Status(404, _)) => return Ok(None),
        Err(e) => return Err(format!("github api: {e}")),
    };
    let json: serde_json::Value = resp.into_json().map_err(|e| format!("api json: {e}"))?;

    let tag = json
        .get("tag_name")
        .and_then(|t| t.as_str())
        .ok_or("release has no tag_name")?
        .to_string();

    let url = json
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(want))
                .and_then(|a| a.get("browser_download_url"))
                .and_then(|u| u.as_str())
                .map(String::from)
        })
        // Fall back to the stable latest-download redirect if the asset list
        // doesn't name it the way we expect.
        .unwrap_or_else(|| {
            format!("https://github.com/{OWNER}/{REPO}/releases/latest/download/{want}")
        });

    Ok(Some((tag, url)))
}

/// Download the new build and swap it over the live library. Spawns a worker so
/// the render thread never blocks on the network. No-op unless an update is
/// pending.
pub fn install() {
    if PHASE.load(Ordering::Acquire) != PHASE_AVAILABLE {
        return;
    }
    PHASE.store(PHASE_DOWNLOADING, Ordering::Release);
    let url = INFO
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|i| i.download_url.clone()))
        .unwrap_or_default();

    let _ = std::thread::Builder::new()
        .name("tux-update-install".into())
        .spawn(move || {
            let Some(target) = current_so_path() else {
                set_info(|i| i.error = "couldn't locate the loaded tuxinjector library".into());
                PHASE.store(PHASE_FAILED, Ordering::Release);
                return;
            };
            match download_and_stage(&url, &target) {
                Ok(()) => {
                    tracing::info!("updater: staged new library at {}", target.display());
                    PHASE.store(PHASE_INSTALLED, Ordering::Release);
                }
                Err(e) => {
                    tracing::error!("updater: install failed: {e}");
                    set_info(|i| i.error = e);
                    PHASE.store(PHASE_FAILED, Ordering::Release);
                }
            }
        });
}

fn download_and_stage(url: &str, target: &Path) -> Result<(), String> {
    let resp = ureq::get(url)
        .set("User-Agent", "tuxinjector")
        .call()
        .map_err(|e| format!("download: {e}"))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read body: {e}"))?;

    validate_payload(&bytes)?;

    let dir = target.parent().ok_or("target library has no parent dir")?;
    let fname = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("tuxinjector");
    // Temp file in the SAME directory so the rename is atomic (same filesystem).
    let tmp = dir.join(format!(".{fname}.new"));

    std::fs::write(&tmp, &bytes).map_err(|e| format!("write temp: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    std::fs::rename(&tmp, target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("replace library (is {} writable?): {e}", target.display())
    })?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_payload(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 1_000_000 || bytes.get(..4) != Some(&b"\x7fELF"[..]) {
        return Err(format!(
            "downloaded file isn't a valid shared object ({} bytes)",
            bytes.len()
        ));
    }
    let want_machine: u16 = match std::env::consts::ARCH {
        "x86_64" => 0x3E,
        "aarch64" => 0xB7,
        "x86" => 0x03,
        "arm" => 0x28,
        _ => 0,
    };
    let got = bytes.get(18..20).map(|b| u16::from_le_bytes([b[0], b[1]]));
    if want_machine != 0 && got != Some(want_machine) {
        return Err(format!(
            "downloaded .so is for the wrong architecture (e_machine={got:?}, \
             expected {want_machine:#06x} for {})",
            std::env::consts::ARCH
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_payload(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 1_000_000 {
        return Err(format!(
            "downloaded file is too small to be a dylib ({} bytes)",
            bytes.len()
        ));
    }
    // CPU_TYPE_* with the 64-bit ABI bit set.
    let want_cpu: u32 = match std::env::consts::ARCH {
        "x86_64" => 0x0100_0007,
        "aarch64" => 0x0100_000C,
        _ => 0,
    };
    match bytes.get(..4).ok_or("truncated download")? {
        // Universal / fat Mach-O (FAT_MAGIC / FAT_MAGIC_64), header is big-endian.
        m @ ([0xCA, 0xFE, 0xBA, 0xBE] | [0xCA, 0xFE, 0xBA, 0xBF]) => {
            macho_fat_has_arch(bytes, want_cpu, m[3] == 0xBF)
        }
        // Thin 64-bit Mach-O (MH_MAGIC_64), native little-endian on disk.
        [0xCF, 0xFA, 0xED, 0xFE] => {
            let cpu = bytes
                .get(4..8)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            if want_cpu != 0 && cpu != Some(want_cpu) {
                return Err(format!(
                    "dylib is for the wrong architecture (cputype={cpu:?}, \
                     expected {want_cpu:#010x} for {})",
                    std::env::consts::ARCH
                ));
            }
            Ok(())
        }
        // Thin 32-bit Mach-O (MH_MAGIC) -- not a target we build, but valid.
        [0xCE, 0xFA, 0xED, 0xFE] => Ok(()),
        other => Err(format!(
            "downloaded file isn't a Mach-O dylib (magic {other:02x?})"
        )),
    }
}

// Confirm a fat Mach-O actually carries a slice for our CPU. fat_header is
// magic(4) + nfat_arch(4), big-endian; each following fat_arch{,_64} starts with
// a big-endian cputype, so we only need to step by the entry size and read it.
#[cfg(target_os = "macos")]
fn macho_fat_has_arch(bytes: &[u8], want_cpu: u32, is64: bool) -> Result<(), String> {
    if want_cpu == 0 {
        return Ok(()); // arch we don't have a constant for; a fat binary likely covers it
    }
    let nfat = bytes
        .get(4..8)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or("truncated fat header")?;
    let entry_size = if is64 { 32 } else { 20 };
    let mut off = 8usize;
    for _ in 0..nfat.min(64) {
        let cputype = bytes
            .get(off..off + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
        if cputype == Some(want_cpu) {
            return Ok(());
        }
        off += entry_size;
    }
    Err(format!(
        "universal dylib has no slice for {} (cputype {want_cpu:#010x})",
        std::env::consts::ARCH
    ))
}

// The on-disk path of the library this code is running from. dladdr (available on
// both Linux and macOS) resolves an address in our own image to its file; the
// injector env var is the fallback.
fn current_so_path() -> Option<PathBuf> {
    // A static lives in our image's data segment, so dladdr resolves to our file.
    static ANCHOR: u8 = 0;
    unsafe {
        let mut dl: libc::Dl_info = std::mem::zeroed();
        let addr = &ANCHOR as *const u8 as *const c_void;
        if libc::dladdr(addr, &mut dl) != 0 && !dl.dli_fname.is_null() {
            if let Ok(s) = CStr::from_ptr(dl.dli_fname).to_str() {
                let p = PathBuf::from(s);
                return std::fs::canonicalize(&p).ok().or(Some(p));
            }
        }
    }

    // Fallback: scan the injector env var (space- and/or colon-separated) for us.
    #[cfg(target_os = "linux")]
    let (env_var, ext) = ("LD_PRELOAD", ".so");
    #[cfg(target_os = "macos")]
    let (env_var, ext) = ("DYLD_INSERT_LIBRARIES", ".dylib");

    let preload = std::env::var(env_var).ok()?;
    for entry in preload.split([' ', ':']) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let p = Path::new(entry);
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.contains("tuxinjector") && name.ends_with(ext) {
                return std::fs::canonicalize(p).ok().or_else(|| Some(p.to_path_buf()));
            }
        }
    }
    None
}

/// Re-exec the game process so the freshly-staged library loads. Returns only on
/// failure (execv replaces the process image).
#[cfg(target_os = "linux")]
pub fn restart_now() {
    unsafe {
        // /proc/self/exe is the real executable (the JVM); /proc/self/cmdline is
        // its argv, NUL-separated. execv keeps the current environment, so the
        // game's LD_PRELOAD is preserved and our new .so loads into the new image.
        let Ok(exe) = std::fs::read_link("/proc/self/exe") else {
            tracing::error!("updater: can't read /proc/self/exe");
            return;
        };
        let Ok(exe_c) = CString::new(exe.as_os_str().as_bytes()) else {
            return;
        };

        let mut argv: Vec<CString> = std::fs::read("/proc/self/cmdline")
            .unwrap_or_default()
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .filter_map(|s| CString::new(s).ok())
            .collect();
        if argv.is_empty() {
            argv.push(exe_c.clone());
        }

        do_execv(&exe_c, &argv);
    }
}

/// Re-exec the game so the freshly-staged dylib loads. macOS has no /proc, so the
/// exec path + argv come from the KERN_PROCARGS2 sysctl. Returns only on failure.
#[cfg(target_os = "macos")]
pub fn restart_now() {
    unsafe {
        // KERN_PROCARGS2 buffer layout:
        //   [int argc][exec_path\0][\0 padding][argv0\0 .. argvN\0][envp...]
        let pid = std::process::id() as libc::c_int;
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];

        let mut size: libc::size_t = 0;
        let probe = libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        );
        if probe != 0 || size < 4 {
            tracing::error!("updater: KERN_PROCARGS2 size probe failed");
            return;
        }

        let mut buf = vec![0u8; size];
        let read = libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            buf.as_mut_ptr() as *mut c_void,
            &mut size,
            std::ptr::null_mut(),
            0,
        );
        if read != 0 {
            tracing::error!("updater: KERN_PROCARGS2 read failed");
            return;
        }
        buf.truncate(size);

        // argc, then the exec path (used as the execv target).
        let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]).max(0) as usize;
        let mut p = 4usize;
        let path_start = p;
        while p < buf.len() && buf[p] != 0 {
            p += 1;
        }
        let Ok(exe_c) = CString::new(&buf[path_start..p]) else {
            return;
        };
        while p < buf.len() && buf[p] == 0 {
            p += 1; // skip the padding NULs between exec_path and argv[0]
        }

        // The argc argument strings.
        let mut argv: Vec<CString> = Vec::with_capacity(argc);
        for _ in 0..argc {
            if p >= buf.len() {
                break;
            }
            let start = p;
            while p < buf.len() && buf[p] != 0 {
                p += 1;
            }
            if let Ok(c) = CString::new(&buf[start..p]) {
                argv.push(c);
            }
            while p < buf.len() && buf[p] == 0 {
                p += 1;
            }
        }
        if argv.is_empty() {
            argv.push(exe_c.clone());
        }

        do_execv(&exe_c, &argv);
    }
}

// execv keeps the current environment, so the game's injector var
// (LD_PRELOAD / DYLD_INSERT_LIBRARIES) is preserved and the freshly-staged
// library loads into the new process image. Returns only if exec fails.
unsafe fn do_execv(exe: &CString, argv: &[CString]) {
    let mut ptrs: Vec<*const c_char> = argv.iter().map(|c| c.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    libc::execv(exe.as_ptr(), ptrs.as_ptr());
    tracing::error!("updater: execv failed: {}", std::io::Error::last_os_error());
}
