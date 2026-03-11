# sleep

Pauses the Lua thread for the given number of milliseconds. This blocks the
whole VM thread, so other keybind callbacks and listeners get delayed until
it's done.

### Example

```lua
-- Show companion apps for 30 seconds then hide them again
tx.bind("F3+C", function()
    tx.toggle_app_visibility()
    tx.sleep(30000)
    tx.toggle_app_visibility()
end)
```

### Arguments

  - `ms`: number

### Return values

None

> This function cannot be called during config-time.
