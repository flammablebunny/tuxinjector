// Lua VM setup, sandboxing, config extraction.
// We nuke os/io/loadfile/dofile so user scripts can't do anything scary.

use mlua::prelude::*;
use tuxinjector_config::Config;

use crate::actions::{LuaActionBinding, TuxinjectorCommand};
use crate::api;

#[derive(Debug)]
pub enum LuaConfigError {
    Lua(mlua::Error),
    NotATable,
}

impl std::fmt::Display for LuaConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lua(e) => write!(f, "Lua error: {e}"),
            Self::NotATable => write!(f, "init.lua must return a table"),
        }
    }
}

impl std::error::Error for LuaConfigError {}

impl From<mlua::Error> for LuaConfigError {
    fn from(e: mlua::Error) -> Self {
        Self::Lua(e)
    }
}

// Config + bindings + commands from a full Lua evaluation
pub struct LuaLoadResult {
    pub config: Config,
    pub action_bindings: Vec<LuaActionBinding>,
    pub commands: Vec<TuxinjectorCommand>,
}

// Simple wrapper when you just want the config and don't care about bindings
pub fn load_lua_config(src: &str) -> Result<Config, LuaConfigError> {
    let res = load_lua_config_full(src)?;
    Ok(res.config)
}

pub fn load_lua_config_full(src: &str) -> Result<LuaLoadResult, LuaConfigError> {
    let lua = create_sandbox()?;
    let api_st = api::install_api(&lua)?;

    let val: LuaValue = lua.load(src).eval()?;

    let config = match val {
        LuaValue::Table(_) => lua.from_value::<Config>(val)?,
        _ => return Err(LuaConfigError::NotATable),
    };

    let st = api_st.borrow();
    let bindings = st.builder.bindings().to_vec();
    let cmds = st.commands.clone();

    Ok(LuaLoadResult {
        config,
        action_bindings: bindings,
        commands: cmds,
    })
}

// Sandbox: nuke dangerous globals, redirect print() to tracing so
// user scripts can't escape but still get debug output
pub(crate) fn create_sandbox() -> Result<Lua, mlua::Error> {
    let lua = Lua::new();

    let g = lua.globals();
    g.set("os", LuaValue::Nil)?;
    g.set("io", LuaValue::Nil)?;
    g.set("loadfile", LuaValue::Nil)?;
    g.set("dofile", LuaValue::Nil)?;

    // print() -> tracing so lua print() calls show up in our logs
    g.set(
        "print",
        lua.create_function(|_, args: LuaMultiValue| {
            let parts: Vec<String> = args
                .into_iter()
                .map(|v| format!("{v:?}"))
                .collect();
            tracing::info!(target: "lua", "{}", parts.join("\t"));
            Ok(())
        })?,
    )?;

    Ok(lua)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table_gives_defaults() {
        let cfg = load_lua_config("return {}").unwrap();
        assert_eq!(cfg.display.default_mode, "Fullscreen");
        assert_eq!(cfg.config_version, 1);
        assert!((cfg.input.mouse_sensitivity - 1.0).abs() < 1e-6);
    }

    #[test]
    fn camel_case_fields_work() {
        let cfg = load_lua_config(
            r#"
            return {
                display = { defaultMode = "Thin", fpsLimit = 60 },
                input = { mouseSensitivity = 0.5 },
                configVersion = 2,
            }
            "#,
        )
        .unwrap();

        assert_eq!(cfg.display.default_mode, "Thin");
        assert!((cfg.input.mouse_sensitivity - 0.5).abs() < 1e-6);
        assert_eq!(cfg.config_version, 2);
        assert_eq!(cfg.display.fps_limit, 60);
    }

    #[test]
    fn modes_array() {
        let cfg = load_lua_config(
            r#"
            return {
                modes = {
                    {
                        id = "Fullscreen",
                        useRelativeSize = true,
                        relativeWidth = 1.0,
                        relativeHeight = 1.0,
                    },
                    {
                        id = "Thin",
                        width = 600,
                        heightExpr = "screenHeight",
                    },
                },
            }
            "#,
        )
        .unwrap();

        assert_eq!(cfg.modes.len(), 2);
        assert_eq!(cfg.modes[0].id, "Fullscreen");
        assert!(cfg.modes[0].use_relative_size);
        assert_eq!(cfg.modes[1].id, "Thin");
        assert_eq!(cfg.modes[1].width, 600);
        assert_eq!(cfg.modes[1].height_expr, "screenHeight");
    }

    #[test]
    fn color_as_array() {
        let cfg = load_lua_config(
            r#"
            return {
                modes = {
                    {
                        id = "Test",
                        border = {
                            enabled = true,
                            color = {255, 128, 0, 230},
                            width = 3,
                        },
                    },
                },
            }
            "#,
        )
        .unwrap();

        let border = &cfg.modes[0].border;
        assert!(border.enabled);
        assert!((border.color.r - 1.0).abs() < 0.01);
        assert!((border.color.g - 0.502).abs() < 0.01);
        assert!((border.color.b - 0.0).abs() < 0.01);
    }

    #[test]
    fn not_a_table_error() {
        let result = load_lua_config("return 42");
        assert!(matches!(result, Err(LuaConfigError::NotATable)));
    }

    #[test]
    fn sandbox_blocks_os() {
        let result = load_lua_config("os.execute('echo pwned'); return {}");
        assert!(result.is_err());
    }

    #[test]
    fn sandbox_blocks_io() {
        let result = load_lua_config("io.open('/etc/passwd'); return {}");
        assert!(result.is_err());
    }

    #[test]
    fn mirrors_config() {
        let cfg = load_lua_config(
            r#"
            return {
                overlays = {
                    mirrors = {
                        {
                            name = "pie",
                            captureWidth = 50,
                            captureHeight = 50,
                            input = {{x = 10, y = 20}},
                            output = {x = 100, y = 200, scale = 2.0},
                            fps = 30,
                        },
                    },
                },
            }
            "#,
        )
        .unwrap();

        assert_eq!(cfg.overlays.mirrors.len(), 1);
        assert_eq!(cfg.overlays.mirrors[0].name, "pie");
        assert_eq!(cfg.overlays.mirrors[0].capture_width, 50);
        assert_eq!(cfg.overlays.mirrors[0].input.len(), 1);
        assert_eq!(cfg.overlays.mirrors[0].input[0].x, 10);
    }

    #[test]
    fn lua_computation_in_config() {
        let cfg = load_lua_config(
            r#"
            local base_sens = 1.0
            local modes = {}
            for i = 1, 3 do
                modes[i] = {
                    id = "Mode" .. i,
                    width = 200 * i,
                }
            end
            return {
                input = { mouseSensitivity = base_sens * 0.5 },
                modes = modes,
            }
            "#,
        )
        .unwrap();

        assert!((cfg.input.mouse_sensitivity - 0.5).abs() < 1e-6);
        assert_eq!(cfg.modes.len(), 3);
        assert_eq!(cfg.modes[0].id, "Mode1");
        assert_eq!(cfg.modes[0].width, 200);
        assert_eq!(cfg.modes[2].id, "Mode3");
        assert_eq!(cfg.modes[2].width, 600);
    }

    #[test]
    fn full_load_with_actions() {
        let result = load_lua_config_full(
            r#"
            local tx = require("tuxinjector")

            tx.bind("F1", function()
                tx.switch_mode("Thin")
            end)

            tx.bind("ctrl+F2", function()
                tx.toggle_gui()
            end, { block = false })

            return {
                display = { defaultMode = "Fullscreen" },
            }
            "#,
        )
        .unwrap();

        assert_eq!(result.config.display.default_mode, "Fullscreen");
        assert_eq!(result.action_bindings.len(), 2);

        // F1 binding
        assert!(result.action_bindings[0].key_combo.contains(&290));
        assert!(result.action_bindings[0].block_from_game);

        // ctrl+F2 binding
        assert!(result.action_bindings[1].key_combo.contains(&291));
        assert!(result.action_bindings[1].key_combo.contains(&341));
        assert!(!result.action_bindings[1].block_from_game);
    }
}
