// Serialize a Config back to init.lua while preserving whatever the user
// had above the `return` statement (comments, require()s, local defs, etc).

use std::path::Path;

use tuxinjector_config::Config;

// --- Public API ---

pub fn write_lua_config(cfg: &Config, path: &Path) {
    let lua_src = match config_to_lua(cfg) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "couldn't serialize config to lua");
            return;
        }
    };

    // keep user preamble intact (comments, require()s, etc)
    let preamble = std::fs::read_to_string(path)
        .map(|s| extract_preamble(&s))
        .unwrap_or_default();

    let out = format!("{preamble}return {lua_src}\n");

    if let Err(e) = std::fs::write(path, &out) {
        tracing::error!(path = %path.display(), error = %e, "failed to write init.lua");
    } else {
        tracing::info!(path = %path.display(), "config saved to init.lua");
    }
}

// --- Profile helpers ---

pub fn list_profiles(config_dir: &Path) -> Vec<String> {
    let dir = config_dir.join("profiles");
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("lua") {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_owned());
                }
            }
        }
    }
    names.sort();
    names
}

pub fn save_profile(cfg: &Config, config_dir: &Path, name: &str) {
    let dir = config_dir.join("profiles");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::error!(error = %e, "failed to create profiles directory");
        return;
    }
    let path = dir.join(format!("{name}.lua"));
    write_lua_config(cfg, &path);
}

pub fn load_profile_source(config_dir: &Path, name: &str) -> Option<String> {
    let path = config_dir.join("profiles").join(format!("{name}.lua"));
    std::fs::read_to_string(&path).ok()
}

pub fn delete_profile(config_dir: &Path, name: &str) {
    let path = config_dir.join("profiles").join(format!("{name}.lua"));
    if let Err(e) = std::fs::remove_file(&path) {
        tracing::warn!(path = %path.display(), error = %e, "failed to delete profile");
    }
}

pub fn rename_profile(config_dir: &Path, old: &str, new: &str) {
    let src = config_dir.join("profiles").join(format!("{old}.lua"));
    let dst = config_dir.join("profiles").join(format!("{new}.lua"));
    if let Err(e) = std::fs::rename(&src, &dst) {
        tracing::warn!(
            old = %src.display(), new = %dst.display(),
            error = %e, "failed to rename profile"
        );
    }
}

// --- Active profile tracking ---

// Persists which profile is active across restarts.
// Empty string = default profile (init.lua).
pub fn save_active_profile(config_dir: &Path, name: &str) {
    let path = config_dir.join("active_profile.txt");
    if let Err(e) = std::fs::write(&path, name) {
        tracing::warn!(path = %path.display(), error = %e, "failed to save active profile");
    }
}

pub fn load_active_profile(config_dir: &Path) -> String {
    let path = config_dir.join("active_profile.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

// Save the config to the correct file based on profile.
// Default profile -> init.lua, named profile -> profiles/<name>.lua.
// NEVER writes a named profile's content to init.lua.
pub fn save_current_config(cfg: &Config, config_dir: &Path) {
    if cfg.profile.is_empty() {
        write_lua_config(cfg, &config_dir.join("init.lua"));
    } else {
        save_profile(cfg, config_dir, &cfg.profile);
    }
}

// --- Preamble extraction ---

// grabs everything before the first line starting with `return`
fn extract_preamble(source: &str) -> String {
    let mut out = String::new();
    for line in source.lines() {
        if line.trim_start().starts_with("return") {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

// --- Lua serialization ---

fn config_to_lua(cfg: &Config) -> Result<String, serde_json::Error> {
    let val = serde_json::to_value(cfg)?;
    let mut buf = String::new();
    format_value(&val, &mut buf, 0);
    Ok(buf)
}

// **THE FOLLOWING CODE WAS WRITTEN BY AN LLM, AGAIN. I AM TOO LAZY TO WRITE THIS ALL MYSELF**
//
// ═══════════════════════════════════════════════════════════════════════════
// Recursive serde_json::Value → Lua Table Serializer
// ═══════════════════════════════════════════════════════════════════════════
//
// Traverses a serde_json::Value tree and emits syntactically valid
// Lua table notation. Handles the following type mappings:
//
//   JSON null    → nil
//   JSON boolean → true / false
//   JSON number  → numeric literal
//   JSON string  → escaped double-quoted string
//   JSON array   → Lua sequence table { ... }
//   JSON object  → Lua keyed table { key = value, ... }
fn format_value(val: &serde_json::Value, buf: &mut String, depth: usize) {
    use serde_json::Value;

    let pad = "    ".repeat(depth);
    let inner_pad = "    ".repeat(depth + 1);

    match val {
        Value::Null => buf.push_str("nil"),
        Value::Bool(b) => buf.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => buf.push_str(&n.to_string()),
        Value::String(s) => {
            buf.push('"');
            for ch in s.chars() {
                match ch {
                    '"' => buf.push_str("\\\""),
                    '\\' => buf.push_str("\\\\"),
                    '\n' => buf.push_str("\\n"),
                    '\r' => buf.push_str("\\r"),
                    '\0' => buf.push_str("\\0"),
                    c => buf.push(c),
                }
            }
            buf.push('"');
        }
        Value::Array(arr) => {
            buf.push_str("{\n");
            for item in arr {
                buf.push_str(&inner_pad);
                format_value(item, buf, depth + 1);
                buf.push_str(",\n");
            }
            buf.push_str(&pad);
            buf.push('}');
        }
        Value::Object(map) => {
            buf.push_str("{\n");
            for (k, v) in map {
                buf.push_str(&inner_pad);
                buf.push_str(k);
                buf.push_str(" = ");
                format_value(v, buf, depth + 1);
                buf.push_str(",\n");
            }
            buf.push_str(&pad);
            buf.push('}');
        }
    }
}
