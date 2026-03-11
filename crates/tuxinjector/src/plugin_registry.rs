// Plugin registry -- holds loaded plugins, dispatches per-frame calls,
// collects overlay submissions, and broadcasts events.

use std::collections::HashMap;
use std::ffi::CString;

use tuxinjector_plugin_api::{FrameContext, PluginEvent};

use crate::plugin_loader::{self, LoadedPlugin, PluginSettings};

#[derive(Debug, Clone)]
pub struct PluginSummary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub settings_schema: String,
}

pub struct PluginRegistry {
    plugins: Vec<LoadedPlugin>,
    saved: HashMap<String, PluginSettings>,
}

impl PluginRegistry {
    pub fn new(
        plugins: Vec<LoadedPlugin>,
        saved: HashMap<String, PluginSettings>,
    ) -> Self {
        Self { plugins, saved }
    }

    // Tick all enabled plugins and collect their overlay submissions
    pub fn on_frame(
        &mut self,
        screen_width: u32,
        screen_height: u32,
        vp_x: f32,
        vp_y: f32,
        vp_w: f32,
        vp_h: f32,
        current_mode: &str,
        game_state: &str,
        frame_num: u64,
        dt_ms: f32,
    ) -> Vec<CollectedSubmission> {
        let mode_c = CString::new(current_mode).unwrap_or_default();
        let state_c = CString::new(game_state).unwrap_or_default();

        let ctx = FrameContext {
            screen_width,
            screen_height,
            viewport_x: vp_x,
            viewport_y: vp_y,
            viewport_width: vp_w,
            viewport_height: vp_h,
            current_mode: mode_c.as_ptr(),
            game_state: state_c.as_ptr(),
            frame_number: frame_num,
            delta_time_ms: dt_ms,
        };

        let mut out = Vec::new();

        for plugin in &mut self.plugins {
            if !plugin.enabled {
                continue;
            }

            let pname = plugin.name.clone();
            let subs = plugin.on_frame(&ctx);

            for sub in subs {
                if sub.pixels.is_null() || sub.pixel_len == 0 || sub.width == 0 || sub.height == 0 {
                    continue;
                }

                let expected = sub.width as usize * sub.height as usize * 4;
                if sub.pixel_len < expected {
                    tracing::warn!(
                        plugin = %pname,
                        expected, actual = sub.pixel_len,
                        "plugin submission pixel data too small, skipping"
                    );
                    continue;
                }

                // copy pixels since the plugin might free them after returning
                let pixels = unsafe {
                    std::slice::from_raw_parts(sub.pixels, expected)
                }.to_vec();

                out.push(CollectedSubmission {
                    x: sub.x,
                    y: sub.y,
                    width: sub.width,
                    height: sub.height,
                    pixels,
                });
            }
        }

        out
    }

    pub fn broadcast_event(&mut self, event: &PluginEvent) {
        for p in &mut self.plugins {
            if p.enabled {
                p.on_event(event);
            }
        }
    }

    pub fn broadcast_mode_switch(&mut self, from: &str, to: &str) {
        let from_c = CString::new(from).unwrap_or_default();
        let to_c = CString::new(to).unwrap_or_default();
        let ev = PluginEvent::ModeSwitch {
            from: from_c.as_ptr(),
            to: to_c.as_ptr(),
        };
        self.broadcast_event(&ev);
    }

    pub fn summaries(&self) -> Vec<PluginSummary> {
        self.plugins
            .iter()
            .map(|p| PluginSummary {
                name: p.name.clone(),
                version: p.version.clone(),
                description: p.description.clone(),
                enabled: p.enabled,
                settings_schema: p.settings_schema(),
            })
            .collect()
    }

    // Enable/disable a plugin by name and persist the change
    pub fn set_enabled(&mut self, name: &str, on: bool) {
        for p in &mut self.plugins {
            if p.name == name {
                p.enabled = on;
            }
        }
        let entry = self.saved.entry(name.to_string()).or_insert(PluginSettings {
            enabled: on,
            settings: HashMap::new(),
        });
        entry.enabled = on;
        plugin_loader::save_plugin_settings(&self.saved);
    }
}

pub struct CollectedSubmission {
    pub x: f32,
    pub y: f32,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
