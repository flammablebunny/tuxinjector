# press_key

Sends a fake key press (keydown then keyup) to Minecraft. See
[Key Names](key_names.md) for valid names.

If you pass a combo (e.g. `"ctrl+F3"`), each key gets pressed and released
one at a time.

### Example

```lua
-- Rebind R to open the F3 debug screen
tx.bind("R", function()
    tx.press_key("F3")
end)
```

### Arguments

  - `key`: string

### Return values

None

> This function cannot be called during config-time.
