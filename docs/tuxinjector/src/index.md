# Introduction

**Docs Version:** 1.1
**Project:** Tuxinjector - Injection based minecraft speedrunning tool
**Stack:** Rust / OpenGL (GLSL 300 ES on Linux, 1.20 on macOS) / GLFW interception / Lua config / imgui-rs

---

## What is Tuxinjector?

Tuxinjector is a Rust overlay that injects into Minecraft's rendering pipeline on Linux and macOS. It uses `LD_PRELOAD` (Linux) or `DYLD_INSERT_LIBRARIES` (macOS) to hook into the game's OpenGL and GLFW calls, rendering directly into the backbuffer with no external capture or compositing overhead.

## How It Works

Tuxinjector compiles to a per-architecture shared library on Linux (e.g. `tuxinjector_x64.so`, `tuxinjector_aarch64.so`) or a universal binary on macOS (`tuxinjector.dylib`) that gets loaded before the game starts. When the JVM calls `dlsym` to resolve OpenGL and GLFW functions, tuxinjector intercepts those lookups and returns its own wrappers. The wrappers stash the real function pointers and add overlay logic before/after forwarding to the originals.

```
Game launch:
  LD_PRELOAD=tuxinjector_x64.so minecraft        # Linux
  DYLD_INSERT_LIBRARIES=tuxinjector.dylib minecraft  # macOS

1. Game's JVM loads -> dlsym("eglSwapBuffers") -> tuxinjector's hooked dlsym
2. Hooked dlsym: stash real eglSwapBuffers, return hooked_egl_swap_buffers
3. Every frame: game calls hooked_egl_swap_buffers
4. Hook: render_overlay() -> draw scene into backbuffer -> call real eglSwapBuffers
5. Buffer is presented with overlay composited on top
```

Input works the same way - `dlsym("glfwSetKeyCallback")` gets intercepted, the game's callback is stashed, and our wrapper gets installed instead. The wrapper handles hotkeys and key rebinds before forwarding events to the game.

On macOS the hook mechanism is slightly different: `dlsym` itself is interposed via `__DATA,__interpose` (Mach-O linker feature) instead of PLT hooking, and GLSL shaders are patched down from 300 ES to 1.20 at runtime for Apple's GL 2.1 compatibility context.

---

## Crate Structure

Tuxinjector is set up as a Rust workspace split into 11 crates. Splitting things up keeps compile times low and isolates the unsafe GL stuff from everything else.

| Crate | Purpose |
|-------|---------|
| `tuxinjector` | Main library: hooks, overlay state, mode system, mirror capture, plugin loader |
| `tuxinjector-core` | Shared types: Color, geometry, lock-free primitives (RCU) |
| `tuxinjector-config` | Config types, Lua hot-reload, serde defaults |
| `tuxinjector-input` | GLFW callback interception, key rebinding, sensitivity scaling |
| `tuxinjector-render` | Image loading (PNG/JPEG/GIF animation) |
| `tuxinjector-gl-interop` | Direct GL renderer, GL state save/restore, scene compositor |
| `tuxinjector-gui` | imgui-rs settings UI (14 tabs), toast notifications |
| `tuxinjector-lua` | Lua scripting runtime, hotkey actions, config loader |
| `tuxinjector-capture` | Window overlay capture: PipeWire on Linux, CoreGraphics on macOS |
| `tuxinjector-plugin-api` | C ABI plugin trait, `declare_plugin!` macro |
| `imgui-glow-renderer` | Local fork of imgui-glow-renderer with GLSL 1.20 shader path for macOS GL 2.1 |

The split isn't perfect yet - a couple things are probably in misleading places, but it works and thats all that really matters :)

---

## Configuration

Everything is configured through Lua files in `~/.config/tuxinjector/` (same path on both Linux and macOS). The default config is `init.lua`, with additional profiles stored in `profiles/<name>.lua`. It returns a table with nested sub-configs:

```lua
return {
    display = {
        defaultMode = "Fullscreen",
        fpsLimit = 0,
    },
    input = {
        mouseSensitivity = 1.0,
        keyRebinds = { enabled = true, rebinds = { ... } },
    },
    theme = {
        fontPath = "/usr/share/fonts/truetype/DejaVuSans.ttf",
        appearance = { theme = "Purple", guiScale = 0.8 },
    },
    overlays = {
        mirrors = { ... },
        images = { ... },
    },
    modes = { ... },
}
```

Hot-reload is supported - editing any config file (including profile files in `profiles/`) while the game is running applies changes immediately without needing to restart. Profiles can also be switched live through the in-game GUI.
