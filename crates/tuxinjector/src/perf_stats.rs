// Performance stats -- background sysfs/procfs polling and per-frame
// lock-free FPS/frame-time counters.
//
// Thanks to MangoHud, who's logic i used below
//
// https://github.com/flightlessmango/MangoHud

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::fs;

extern crate libc;

// --- Snapshot ---

#[derive(Clone)]
pub struct PerfSnapshot {
    pub fps: f32,
    pub frame_time_ms: f32,
    pub frame_times: Vec<f32>,    // last <=128 frame times (ms), oldest first
    pub cpu_usage_pct: f32,
    pub cpu_freq_mhz: u32,
    pub cpu_temp_c: f32,
    pub gpu_usage_pct: f32,
    pub gpu_temp_c: f32,
    pub gpu_vram_used_mb: u64,
    pub gpu_vram_total_mb: u64,
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
}

// --- Internal ---

struct SysInfo {
    cpu_usage_pct: f32,
    cpu_freq_mhz: u32,
    cpu_temp_c: f32,
    gpu_usage_pct: f32,
    gpu_temp_c: f32,
    gpu_vram_used_mb: u64,
    gpu_vram_total_mb: u64,
    ram_used_mb: u64,
    ram_total_mb: u64,
}

impl Default for SysInfo {
    fn default() -> Self {
        Self {
            cpu_usage_pct: 0.0,
            cpu_freq_mhz: 0,
            cpu_temp_c: 0.0,
            gpu_usage_pct: 0.0,
            gpu_temp_c: 0.0,
            gpu_vram_used_mb: 0,
            gpu_vram_total_mb: 0,
            ram_used_mb: 0,
            ram_total_mb: 0,
        }
    }
}

// --- PerfStats ---

pub struct PerfStats {
    sysinfo: RwLock<SysInfo>,
    frame_count: AtomicU64,
    window_start_ns: AtomicU64,
    fps_bits: AtomicU32,        // f32 stored as bits (lock-free)
    last_frame_ns: AtomicU64,
    ft_ms_bits: AtomicU32,      // last frame time as f32 bits
    frame_times: Mutex<VecDeque<f32>>,
    stop: Arc<AtomicBool>,
}

impl PerfStats {
    pub fn new() -> Arc<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stats = Arc::new(Self {
            sysinfo: RwLock::new(SysInfo::default()),
            frame_count: AtomicU64::new(0),
            window_start_ns: AtomicU64::new(0),
            fps_bits: AtomicU32::new(0.0f32.to_bits()),
            last_frame_ns: AtomicU64::new(0),
            ft_ms_bits: AtomicU32::new(0.0f32.to_bits()),
            frame_times: Mutex::new(VecDeque::with_capacity(128)),
            stop: Arc::clone(&stop),
        });

        let bg = Arc::clone(&stats);
        thread::Builder::new()
            .name("tuxinjector-perf".into())
            .spawn(move || poll_sysinfo(bg))
            .ok();

        stats
    }

    // Called once per swap from the render thread
    pub fn record_frame(&self) {
        let now = monotonic_ns();

        let prev = self.last_frame_ns.swap(now, Ordering::Relaxed);
        if prev != 0 {
            let ft = (now.saturating_sub(prev)) as f32 / 1_000_000.0;
            self.ft_ms_bits.store(ft.to_bits(), Ordering::Relaxed);

            // ring buffer -- try_lock so we never stall the game thread
            if let Ok(mut buf) = self.frame_times.try_lock() {
                if buf.len() >= 128 {
                    buf.pop_front();
                }
                buf.push_back(ft);
            }
        }

        let n = self.frame_count.fetch_add(1, Ordering::Relaxed) + 1;
        let win_start = self.window_start_ns.load(Ordering::Relaxed);
        if win_start == 0 {
            self.window_start_ns.store(now, Ordering::Relaxed);
        } else if now.saturating_sub(win_start) >= 1_000_000_000 {
            // one second elapsed, update FPS
            let elapsed = now.saturating_sub(win_start) as f32 / 1_000_000_000.0;
            let fps = n as f32 / elapsed;
            self.fps_bits.store(fps.to_bits(), Ordering::Relaxed);
            self.frame_count.store(0, Ordering::Relaxed);
            self.window_start_ns.store(now, Ordering::Relaxed);
        }
    }

    pub fn snapshot(&self) -> PerfSnapshot {
        let fps = f32::from_bits(self.fps_bits.load(Ordering::Relaxed));
        let ft_ms = f32::from_bits(self.ft_ms_bits.load(Ordering::Relaxed));

        let fts = self.frame_times
            .lock()
            .map(|buf| buf.iter().copied().collect::<Vec<_>>())
            .unwrap_or_default();

        let info = self.sysinfo.read().unwrap_or_else(|e| e.into_inner());
        PerfSnapshot {
            fps,
            frame_time_ms: ft_ms,
            frame_times: fts,
            cpu_usage_pct: info.cpu_usage_pct,
            cpu_freq_mhz: info.cpu_freq_mhz,
            cpu_temp_c: info.cpu_temp_c,
            gpu_usage_pct: info.gpu_usage_pct,
            gpu_temp_c: info.gpu_temp_c,
            gpu_vram_used_mb: info.gpu_vram_used_mb,
            gpu_vram_total_mb: info.gpu_vram_total_mb,
            ram_used_mb: info.ram_used_mb,
            ram_total_mb: info.ram_total_mb,
        }
    }
}

impl Drop for PerfStats {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

// --- Background poller ---

#[cfg(target_os = "linux")]
struct CpuJiffies {
    total: u64,
    idle: u64,
}

#[cfg(target_os = "linux")]
fn poll_sysinfo(stats: Arc<PerfStats>) {
    // discover sysfs paths once at startup
    let cpu_temp = find_hwmon_file("k10temp", &["temp2_input", "temp1_input"]);
    let gpu_temp = find_hwmon_file("amdgpu", &["temp1_input"]);
    let (gpu_busy, vram_used, vram_total) = find_amd_drm_paths();

    let mut prev_cpu: Option<CpuJiffies> = None;

    loop {
        if stats.stop.load(Ordering::Relaxed) {
            break;
        }

        // cpu
        let cur_cpu = read_proc_stat();
        let cpu_pct = match (&prev_cpu, &cur_cpu) {
            (Some(prev), Some(cur)) => calc_cpu_usage(prev, cur),
            _ => 0.0,
        };
        prev_cpu = cur_cpu;

        let freq = avg_cpu_freq_mhz();

        let cpu_t = cpu_temp.as_deref().and_then(read_millicelsius).unwrap_or(0.0);
        let gpu_pct = gpu_busy.as_deref().and_then(read_u32_file).map(|v| v as f32).unwrap_or(0.0);
        let gpu_t = gpu_temp.as_deref().and_then(read_millicelsius).unwrap_or(0.0);
        let vram_u = vram_used.as_deref().and_then(read_u64_file).map(|b| b / (1024*1024)).unwrap_or(0);
        let vram_t = vram_total.as_deref().and_then(read_u64_file).map(|b| b / (1024*1024)).unwrap_or(0);
        let (ram_u, ram_t) = read_ram_mb();

        if let Ok(mut si) = stats.sysinfo.write() {
            si.cpu_usage_pct = cpu_pct;
            si.cpu_freq_mhz = freq;
            si.cpu_temp_c = cpu_t;
            si.gpu_usage_pct = gpu_pct;
            si.gpu_temp_c = gpu_t;
            si.gpu_vram_used_mb = vram_u;
            si.gpu_vram_total_mb = vram_t;
            si.ram_used_mb = ram_u;
            si.ram_total_mb = ram_t;
        }

        thread::sleep(Duration::from_millis(500));
    }
}

// TODO: macOS - use IOKit / host_statistics for CPU/GPU/RAM stats
#[cfg(target_os = "macos")]
fn poll_sysinfo(stats: Arc<PerfStats>) {
    loop {
        if stats.stop.load(Ordering::Relaxed) { break; }
        thread::sleep(Duration::from_millis(500));
    }
}

// --- monotonic clock ---

#[cfg(target_os = "linux")]
fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

#[cfg(target_os = "macos")]
fn monotonic_ns() -> u64 {
    use std::sync::OnceLock;

    #[repr(C)]
    struct MachTimebaseInfo { numer: u32, denom: u32 }
    extern "C" {
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
    }

    static INFO: OnceLock<(u32, u32)> = OnceLock::new();
    let (numer, denom) = *INFO.get_or_init(|| {
        let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
        unsafe { mach_timebase_info(&mut info) };
        (info.numer, info.denom)
    });

    let ticks = unsafe { mach_absolute_time() };
    ticks * numer as u64 / denom as u64
}

// --- sysfs / procfs helpers ---

#[cfg(target_os = "linux")]
fn read_proc_stat() -> Option<CpuJiffies> {
    let content = fs::read_to_string("/proc/stat").ok()?;
    let line = content.lines().next()?;
    let vals: Vec<u64> = line.split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    if vals.len() < 5 { return None; }
    // user, nice, system, idle, iowait, ...
    let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
    let total: u64 = vals.iter().take(8).sum();
    Some(CpuJiffies { total, idle })
}

#[cfg(target_os = "linux")]
fn calc_cpu_usage(prev: &CpuJiffies, cur: &CpuJiffies) -> f32 {
    let dt = cur.total.saturating_sub(prev.total) as f32;
    let di = cur.idle.saturating_sub(prev.idle) as f32;
    if dt == 0.0 { return 0.0; }
    ((dt - di) / dt * 100.0).clamp(0.0, 100.0)
}

#[cfg(target_os = "linux")]
fn avg_cpu_freq_mhz() -> u32 {
    let cpu_dir = Path::new("/sys/devices/system/cpu");
    let Ok(entries) = fs::read_dir(cpu_dir) else { return 0 };
    let mut total_khz: u64 = 0;
    let mut n: u32 = 0;
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let name = fname.to_string_lossy();
        if !name.starts_with("cpu") || !name[3..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let freq_path = entry.path().join("cpufreq/scaling_cur_freq");
        if let Ok(s) = fs::read_to_string(&freq_path) {
            if let Ok(khz) = s.trim().parse::<u64>() {
                total_khz += khz;
                n += 1;
            }
        }
    }
    if n == 0 { 0 } else { (total_khz / n as u64 / 1000) as u32 }
}

#[cfg(target_os = "linux")]
fn find_hwmon_dir(name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir("/sys/class/hwmon").ok()?.flatten() {
        let p = entry.path();
        if let Ok(n) = fs::read_to_string(p.join("name")) {
            if n.trim() == name {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn find_hwmon_file(hwmon: &str, candidates: &[&str]) -> Option<PathBuf> {
    let dir = find_hwmon_dir(hwmon)?;
    for &name in candidates {
        let p = dir.join(name);
        if p.exists() { return Some(p); }
    }
    None
}

#[cfg(target_os = "linux")]
fn read_millicelsius(path: &Path) -> Option<f32> {
    let s = fs::read_to_string(path).ok()?;
    let mc: i64 = s.trim().parse().ok()?;
    Some(mc as f32 / 1000.0)
}

#[cfg(target_os = "linux")]
fn read_u32_file(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn read_u64_file(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(target_os = "linux")]
fn find_amd_drm_paths() -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return (None, None, None);
    };
    for entry in entries.flatten() {
        let dev = entry.path().join("device");
        let busy = dev.join("gpu_busy_percent");
        if busy.exists() {
            let vu = dev.join("mem_info_vram_used");
            let vt = dev.join("mem_info_vram_total");
            return (
                Some(busy),
                vu.exists().then_some(vu),
                vt.exists().then_some(vt),
            );
        }
    }
    (None, None, None)
}

#[cfg(target_os = "linux")]
fn read_ram_mb() -> (u64, u64) {
    let Ok(content) = fs::read_to_string("/proc/meminfo") else { return (0, 0) };
    let mut total_kb = 0u64;
    let mut avail_kb = 0u64;
    for line in content.lines() {
        if line.starts_with("MemTotal:") {
            total_kb = meminfo_val(line);
        } else if line.starts_with("MemAvailable:") {
            avail_kb = meminfo_val(line);
        }
    }
    let used = total_kb.saturating_sub(avail_kb);
    (used / 1024, total_kb / 1024)
}

#[cfg(target_os = "linux")]
fn meminfo_val(line: &str) -> u64 {
    line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0)
}
