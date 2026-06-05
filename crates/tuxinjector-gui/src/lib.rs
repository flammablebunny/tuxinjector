pub mod app;
#[cfg(target_os = "linux")]
pub mod companion_xserver;
pub mod running_apps;
pub mod state_status;
pub mod tabs;
pub mod toast;
pub mod widgets;

pub use app::{SettingsApp, SettingsOutput};
pub use state_status::{set_state_mod_status, state_mod_status, StateModStatus};
