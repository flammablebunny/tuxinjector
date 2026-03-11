// Capture backends - PipeWire on Linux, CoreGraphics on macOS.
// Everything funnels into CapturedFrame (RGBA pixels for GL upload).

#[cfg(all(feature = "pipewire", target_os = "linux"))]
pub mod pipewire_capture;
#[cfg(all(feature = "pipewire", target_os = "linux"))]
mod portal;

#[cfg(target_os = "macos")]
pub mod mac_capture;

pub struct CapturedFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

// just shells out to pw-cli to see if pipewire is alive
#[cfg(all(feature = "pipewire", target_os = "linux"))]
pub fn pipewire_available() -> bool {
    std::process::Command::new("pw-cli")
        .arg("info")
        .arg("0")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(all(feature = "pipewire", target_os = "linux")))]
pub fn pipewire_available() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn mac_capture_available() -> bool {
    mac_capture::cg_available()
}

#[cfg(not(target_os = "macos"))]
pub fn mac_capture_available() -> bool {
    false
}
