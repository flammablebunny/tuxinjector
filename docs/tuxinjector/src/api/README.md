# Lua API Reference

Tuxinjector is configured using the [Lua](https://lua.org) programming language. If you haven't used
Lua before, these are good starting points:

  - [Programming in Lua](https://www.lua.org/pil/contents.html)
  - [Lua 5.1 Reference Manual](https://www.lua.org/manual/5.1/)

> [!CAUTION]
> Lua code executed by tuxinjector is allowed to interact with the host operating
> system in various ways, such as spawning subprocesses. Read other people's code
> and do not blindly copy and paste it into your own configuration. *cough* gore *cough*

> [!WARNING]
> Not everything you can do with tuxinjector's Lua API is legal for speedrun.com
> submissions or MCSR Ranked. If you're unsure whether something is allowed,
> check the rulebook or ask a mod before using it in runs you intend to submit.

# Configuration

By default, tuxinjector reads and executes a configuration file from
`~/.config/tuxinjector/init.lua`. Additional profiles are stored in
`~/.config/tuxinjector/profiles/<name>.lua` and can be switched via the in-game GUI.

The config file has to return a table with all the display, input, overlay,
hotkey, and mode settings. You can also use the API module to register keybindings and call runtime functions:

```lua
local tx = require("tuxinjector")

-- Register keybindings (config-time)
tx.bind("ctrl+F1", function()
    tx.switch_mode("Thin")
end)

-- Return config table
return {
    display = { ... },
    input = { ... },
    overlays = { ... },
    modes = { ... },
}
```

# Hot reload

Tuxinjector watches for changes to any `.lua` file within the configuration
directory (including profile files in `profiles/`). When it detects a change, it
automatically reloads the currently active config - whether that's `init.lua` or
a named profile. The Lua VM is destroyed and recreated, so any state within will
not be transferred to the new configuration.
