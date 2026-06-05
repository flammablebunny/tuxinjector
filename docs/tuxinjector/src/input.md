# Input Interception

All GLFW input goes through tuxinjector for hotkey processing, key rebinding, mouse sensitivity scaling, and GUI overlay routing. It uses the same `dlsym` + PLT hook mechanism from [Injection & Hooking](injection.md), but there's a lot more going on in the actual processing pipeline.

---

## Callback Interception

Minecraft (LWJGL3) registers GLFW callbacks for keyboard, mouse, cursor, and scroll events. We intercept the `glfwSetXxxCallback` calls, stash the game's original callback, and install our own wrapper instead:

```
Game: glfwSetKeyCallback(window, game_key_handler)
Hook: GAME_KEY_CALLBACK = game_key_handler
      real_glfwSetKeyCallback(window, tuxinjector_key_callback)
      return old_game_callback
```

From that point on, GLFW calls our `tuxinjector_key_callback` for every key event. The wrapper decides whether to eat the event (hotkey match, GUI has focus) or forward it to the game's original callback.

### Intercepted Callbacks

| GLFW Function | Wrapper | Purpose |
|--------------|---------|---------|
| `glfwSetKeyCallback` | `tuxinjector_key_callback` | Hotkeys, key rebinding, GUI keyboard input |
| `glfwSetMouseButtonCallback` | `tuxinjector_mouse_button_callback` | Mouse button rebinding, GUI click forwarding |
| `glfwSetCursorPosCallback` | `tuxinjector_cursor_pos_callback` | Mouse sensitivity scaling, GUI cursor tracking |
| `glfwSetScrollCallback` | `tuxinjector_scroll_callback` | Scroll consumption for GUI, hotkey scroll binds |
| `glfwSetCharCallback` | `tuxinjector_char_callback` | Character rebinding, GUI text input |
| `glfwSetCharModsCallback` | `tuxinjector_char_mods_callback` | Same thing but LWJGL3 prefers this variant |
| `glfwSetInputMode` | `intercept_set_input_mode` | Tracks cursor capture state (FPS vs menu) |

---

## Key Event Flow

Every key event runs through a multi-stage pipeline:

```
GLFW key event -> tuxinjector_key_callback:

1. update_key_state(key, action)           // Track pressed keys for Lua get_key()
2. INPUT_HANDLER.handle_key(key, ...)      // Hotkey engine + rebinding
   |-- consumed = true?  -> event swallowed (hotkey matched)
   '-- consumed = false  -> forward_key:
      |-- forward_key >= MOUSE_BUTTON_OFFSET?
      |   -> forward as mouse button event to game
      |   -> if original key is modifier, also forward key event
      '-- forward_key < MOUSE_BUTTON_OFFSET?
          -> forward as key event to game (using remapped keycode)
3. If action == PRESS and key was rebinded:
   '-- If original key has no character but target does:
       -> inject synthetic char event (emitTypedChar)
```

### InputHandler Trait

Input processing is abstracted behind an `InputHandler` trait:

```rust
trait InputHandler: Send {
    fn handle_key(&mut self, key: i32, scancode: i32, action: i32, mods: i32)
        -> (consumed: bool, forward_key: i32);

    fn handle_mouse_button(&mut self, button: i32, action: i32, mods: i32)
        -> (consumed: bool, forward_button: i32);

    fn handle_cursor_pos(&mut self, x: f64, y: f64)
        -> Option<(f64, f64)>;    // None = consume, Some = forward

    fn handle_scroll(&mut self, x: f64, y: f64)
        -> bool;                   // true = consume
}
```

`TuxinjectorInputHandler` checks hotkeys first, then rebinding. If a hotkey matches, the event gets eaten and never reaches the game.

---

## Key Rebinding

The rebinding system translates GLFW keycodes before the game sees them. It handles keyboard-to-keyboard, keyboard-to-mouse, and mouse-to-keyboard, so you can do things like rebind Mouse4 to F3.

### Encoding

Mouse buttons get encoded as `button + MOUSE_BUTTON_OFFSET (400)` so they share the same keycode space as keyboard keys:

| Input | Encoded Value |
|-------|--------------|
| GLFW_KEY_A (65) | 65 |
| GLFW_KEY_F3 (292) | 292 |
| GLFW_MOUSE_BUTTON_1 (0) | 400 |
| GLFW_MOUSE_BUTTON_4 (3) | 403 |
| GLFW_MOUSE_BUTTON_5 (4) | 404 |

### Forward and Reverse Maps

Two lookup maps handle everything:

```
Forward map: (from_key, to_key)    -- used in callbacks and char event remapping
Reverse map: (to_key, from_key)    -- used in glfwGetKey/glfwGetMouseButton polling
```

When a key event comes in, the forward map translates the physical keycode to the logical one. When the game polls a key via `glfwGetKey`, the reverse map goes the other way.

### Per-Context Targets

Each rebind has **two** targets - one for when you're playing (`toKey`) and one for when a menu/chat is open (`toKeyChat`). A target of `0` means **pass the key through unchanged** in that context (rather than "use the other target"), so a chat-only rebind (`toKey = 0`, `toKeyChat` set) is valid and leaves the key untouched in-game.

The context is taken from the [game-state mod](state-mods.md) tag when one is available, and otherwise falls back to the live cursor-capture state (`effective_in_chat` -> `!is_cursor_captured()`): grabbed = in-game, free = chat/menu. This fallback is what makes rebinds work even without a state mod installed.

### Cross-Device Rebinding

Things get interesting when you rebind across device types (mouse to keyboard or vice versa), because `glfwGetKey` and `glfwGetMouseButton` need to be cross-routed:

```
Example: Mouse4 (button 3) rebound to F3 (keycode 292)

Forward map:  (403, 292)    // encoded Mouse4 -> F3
Reverse map:  (292, 403)    // F3 -> encoded Mouse4

Callback path:
  Mouse4 press -> handle_mouse_button(3, ...)
    encoded = 3 + 400 = 403
    remap_key(403) -> 292 (F3)
    forward_key = 292 < MOUSE_BUTTON_OFFSET
    -> forward_key_to_game(window, 292, ...)
    Game sees: F3 pressed

Polling path:
  Game calls glfwGetKey(window, 292)  // "Is F3 held?"
    physical_key_for(292) -> 403 (encoded Mouse4)
    403 >= MOUSE_BUTTON_OFFSET
    -> glfwGetMouseButton(window, 403 - 400 = 3)
    Returns: real mouse button 3 state

Combo keys:
  Mouse4 + C held -> game polls glfwGetKey(F3) + glfwGetKey(C)
    glfwGetKey(F3) -> route to glfwGetMouseButton(3) -> PRESS
    glfwGetKey(C)  -> route to real glfwGetKey(C)    -> PRESS
    Game sees: F3+C held
```

The reverse direction also works - if a keyboard key is rebound to a mouse button, `glfwGetMouseButton` gets routed to poll the keyboard key via `glfwGetKey`.

!!! note "Stable chords with rebound keys"
    The logical key that `glfwGetKey` reports for a physical key is **pinned at PRESS time** and held until release. This stops a RELEASE/REPEAT from desyncing to a different key if the rebind context flips mid-press, and is what makes Minecraft's F3 chords (F3+C, F3+F, …) resolve correctly even when the keys involved are rebound.

---

## Character Rebinding

When a key gets rebound, its character output needs to change too. GLFW fires `CharCallback` separately from `KeyCallback`, so characters need their own rebinding pass:

```
Incoming char 'r' (codepoint 114):
1. codepoint_to_glfw_key(114) -> (82, shifted=false)   // 'r' -> GLFW_KEY_R
2. forward_remap(82) -> Some(292)                        // R -> F3
3. glfw_key_to_codepoint(292, false) -> None              // F3 has no char
4. Return 0 -> suppress char event
```

And when a non-character key (like F1) gets rebound to a character key (like A), the key callback has to inject a synthetic char event manually since GLFW wouldn't normally fire one for the original key.

---

## Mouse Sensitivity Scaling

We intercept `glfwSetCursorPosCallback` and `glfwGetCursorPos` to scale mouse deltas in FPS mode:

```
Cursor event (x, y):
  if cursor_captured:
    delta_x = x - center_x
    delta_y = y - center_y
    scaled_x = center_x + delta_x * sensitivity_x
    scaled_y = center_y + delta_y * sensitivity_y
    forward (scaled_x, scaled_y) to game
  else:
    forward (x, y) unchanged   // Menu mode: no scaling
```

Sensitivity can be set globally, per-mode, or through Lua hotkey actions.

The sub-pixel accumulator (the fractional remainder of a scaled delta) is **reset** whenever sensitivity changes - via a hotkey or a mode switch - and whenever the cursor reverses direction on an axis. This prevents a one-frame jump when you swap sensitivity mid-session, matching Toolscreen's behaviour.

### Cursor State Tracking

`glfwSetInputMode` is intercepted to know whether the cursor is captured (`GLFW_CURSOR_DISABLED`) or free (`GLFW_CURSOR_NORMAL`):

| Cursor Mode | Sensitivity | GUI Cursor |
|------------|-------------|------------|
| `GLFW_CURSOR_DISABLED` (FPS) | Active | Hidden (forced visible when GUI opens) |
| `GLFW_CURSOR_NORMAL` (Menu) | Inactive | Already visible |

When the GUI opens during FPS mode, we force the cursor to `GLFW_CURSOR_NORMAL`. When it closes, we restore `GLFW_CURSOR_DISABLED`. If the game tries to re-capture the cursor while the GUI is open, we just block that call.

### Menu-Open Recentering (ICM)

When the game opens an in-game menu - the cursor transitions from grabbed (`GLFW_CURSOR_DISABLED`) to free (`GLFW_CURSOR_NORMAL`) - Tuxinjector re-centers the cursor using a stashed pointer to the real `glfwSetCursorPos`. This defeats *involuntary cursor manipulation* (ICM): the small leftover cursor movement that can otherwise nudge your view or a menu selection on the frame a menu opens. The real `glfwSetCursorPos` and `glfwGetWindowSize` pointers are stashed specifically for this.

---

## GUI Input Routing

When the GUI overlay is visible, input gets routed to imgui instead of the game:

```
Key event + GUI visible:
  if key_capture_mode:
    push_captured_key(key)        // Record next key press for rebind config
  else if gui_wants_keyboard:
    push_gui_key(key, mods)       // Forward to imgui (text field has focus)
  else:
    check hotkeys only            // GUI toggle, etc.

Mouse button + GUI visible:
  push_gui_button_press/release() // Forward click to imgui

Cursor + GUI visible:
  forward raw position            // imgui needs cursor position for hover/click

Scroll + GUI visible:
  push_gui_scroll(dx, dy)         // Forward scroll to imgui
```

The GUI state flags (`GUI_VISIBLE`, `GUI_WANTS_KEYBOARD`, `GUI_CAPTURE_MODE`) are all atomic booleans, so they're lock-free and can be read from any thread.

A few guarantees while the GUI is open:

- `glfwGetKey` / `glfwGetMouseButton` polling returns `GLFW_RELEASE`, so a key held when you opened the GUI doesn't leak through to the game via polling.
- `Shift` is **not** forwarded as a modifier with scroll events to imgui (otherwise Shift+Scroll becomes horizontal scroll in the GUI).
- `glfwGetCursorPos` applies the viewport-centering offset in non-fullscreen modes, so the game and GUI receive cursor coordinates in the correct viewport space.

---

## Key State Tracking

A `HashSet<i32>` keeps track of which keys are currently held down. Gets updated on every key event and is what the Lua API's `get_key()` function reads from:

```rust
fn update_key_state(key: i32, action: i32) {
    if action == GLFW_PRESS:
        pressed_keys.insert(key)
    if action == GLFW_RELEASE:
        pressed_keys.remove(key)
}

// Lua: tx.get_key("w") -> is_key_pressed(87) -> pressed_keys.contains(87)
```
