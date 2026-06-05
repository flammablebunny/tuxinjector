//! Configuration system: types, defaults, expression parsing, snapshots, hot-reload

pub mod defaults;
pub mod expr;
pub mod hot_reload;
pub mod key_names;
pub mod profile_override;
pub mod snapshot;
pub mod types;

// Re-exports
pub use expr::{evaluate_expression, is_expression, validate_expression};
pub use profile_override::profile_override;
pub use hot_reload::{ConfigParser, ConfigWatcher};
pub use snapshot::ConfigSnapshot;
pub use types::{
    AdvancedConfig, Config, DisplayConfig, GlobalHotkeysConfig, InputConfig, OverlaysConfig,
    ThemeConfig,
};
