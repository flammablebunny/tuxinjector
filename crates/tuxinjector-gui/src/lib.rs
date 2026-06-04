pub mod app;
#[cfg(target_os = "linux")]
pub mod companion_xserver;
pub mod running_apps;
pub mod tabs;
pub mod toast;
pub mod widgets;

pub use app::{SettingsApp, SettingsOutput};
