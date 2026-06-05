# Profiles

Profiles let you keep multiple complete configurations and switch between them.

- The **default** profile is `~/.config/tuxinjector/init.lua`.
- **Named** profiles live in `~/.config/tuxinjector/profiles/<name>.lua`.
- The currently selected profile is tracked in `~/.config/tuxinjector/active_profile.txt` (an empty file means the default).

You can create, rename, delete, and switch profiles from the in-game GUI, and [hot-reload](../api/README.md#hot-reload) applies edits to whichever profile is active.

## Pinning a profile per instance

`active_profile.txt` is shared across every running instance, so if you run more than one Minecraft instance they would normally fight over it. To give each instance its own profile, pin one on the **launch command**:

```
--profile <name>
```

`--profile=<name>` also works, and there's a `TUXINJECTOR_PROFILE` environment-variable fallback. For example, in a Prism wrapper command:

```
env LD_PRELOAD=$HOME/.local/share/tuxinjector/tuxinjector_x64.so TUXINJECTOR_PROFILE=Speedrun
```

!!! note "Minecraft ignores the flag"
    `--profile` is read straight from the game process's command line. Minecraft ignores unrecognized game arguments, so it's safe to append. For a profile name with spaces, quote it: `--profile "My Profile"`.

### Behaviour when pinned

- The pinned profile **overrides** `active_profile.txt` for that instance, for both the initial load and hot-reload.
- It is **never written back** to `active_profile.txt`, so instances don't clobber each other's selection.
- The in-game GUI profile selector is **greyed out** (with a tooltip) for that instance, since switching wouldn't take effect.
- `--profile ''` (empty name) or `TUXINJECTOR_PROFILE=` forces the **default** profile (`init.lua`).
- If the named profile file doesn't exist, Tuxinjector falls back to the default and logs a warning.

### Resolution order

```
--profile=<name>  >  --profile <name>  >  TUXINJECTOR_PROFILE  >  active_profile.txt
```
