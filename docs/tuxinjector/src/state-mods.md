# Game-State Mods

Some features - most importantly **state-conditioned hotkeys** (a hotkey with a *Required Game States* condition that should only fire on the wall, in-game, etc.) - need to know what the game is currently doing. Tuxinjector reads that from a game-state mod's output file.

## Sources

Tuxinjector looks in the Minecraft instance's run directory (the game's working directory) for, in order:

1. **Hermes** - `hermes/state.json` (preferred, the modern source).
2. **State Output** - `wpstateout.txt` (fallback).

If neither is present, it falls back to a coarse guess from the window title.

## Liveness

A source must be **live**, not just present - a disabled or crashed mod can leave a stale file behind:

- **Hermes** writes a `hermes/alive` heartbeat roughly once a second. It counts as live only if the heartbeat's PID matches the game process and the timestamp is within **3 seconds** of now (matching Toolscreen's check). A leftover `state.json` from a previous run with a frozen `alive` file is treated as **absent**.
- **State Output** has no heartbeat - because a stable in-world state can legitimately goes minutes without a write - it counts as live only if `wpstateout.txt` was written **this session** (its modification time is at or after Tuxinjector loaded).

Liveness is re-checked every poll, so a mod going away mid-session is noticed.

## Missing-mod warning

If there's no **live** source, Tuxinjector:

- prints a one-time warning to stderr (the log), and
- shows an orange banner in the GUI **Hotkeys** tab.

While there's no live source, every state-conditioned hotkey **falls back to acting as if "Any" were selected** - so it still fires regardless of which *Required Game States* are checked, rather than silently never matching. Install or re-enable the **Hermes** mod (or **State Output**) if you want those conditions to actually be enforced.

## States

Canonical state strings used by hotkey conditions and the Lua API:

`wall`, `title`, `waiting`, `generating`, `inworld,cursor_grabbed`, `inworld,cursor_free`

In-world is split by cursor-grab so a hotkey can tell active play (`cursor_grabbed`) from a menu/inventory/chat (`cursor_free`).

## Lua

Read the current state with [`tx.state()`](api/tx_state.md), or react to changes with [`tx.listen("state", fn)`](api/tx_listen.md):

```lua
local tx = require("tuxinjector")
tx.listen("state", function(state)
    tx.log("game state: " .. state)
end)
```
