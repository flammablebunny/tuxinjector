# tuxinjector

> **Tuxinjector is NOT legal to use in speedrun.com submissions or MCSR Ranked yet. Do not use it in runs you intend to submit or in ranked matches.**

Tux Injector is an overlay written in Rust that injects into Minecraft's rendering pipeline on Linux and macOS. It uses `LD_PRELOAD` (Linux) or `DYLD_INSERT_LIBRARIES` (macOS) to hook into the game's OpenGL and GLFW calls, rendering directly into the backbuffer with no external capture or compositing overhead.

**[Full documentation](https://flammablebunny.github.io/tuxinjector/)**

---

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

For a deeper look at the injection, rendering, and input systems, check the [architecture docs](https://flammablebunny.github.io/tuxinjector/injection/).

---

## Usage

### Linux (Prism Launcher)

Set a **Wrapper Command** in your instance settings under **Custom Commands**:

```
env LD_PRELOAD=/path/to/tuxinjector_x64.so
```

You can also set `LD_PRELOAD` under the Environment Variables tab instead.

### macOS (Prism Launcher)

Set the environment variable in your instance settings:

```
DYLD_INSERT_LIBRARIES=/path/to/tuxinjector.dylib
```

See the [usage docs](https://flammablebunny.github.io/tuxinjector/usage/) for full setup instructions.

---

## Configuration

Everything is configured through Lua files in `~/.config/tuxinjector/`. The default config is `init.lua`, with additional profiles stored in `profiles/<name>.lua`. It returns a table with nested sub-configs:

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

The [Lua API reference](https://flammablebunny.github.io/tuxinjector/api/) covers all the scripting functions for keybinds, mode switching, sensitivity, and more.

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

---

## Building

### With Nix (recommended)

```bash
nix develop
./build.sh
```

### Without Nix

```bash
# Ensure pkg-config and OpenGL dev headers are installed
./build.sh
```

On Linux this produces an architecture-suffixed binary (e.g. `target/release/tuxinjector_x64.so`, `tuxinjector_aarch64.so`, `tuxinjector_aarch32.so`, or `tuxinjector_x86.so`).
On macOS this produces `target/release/tuxinjector.dylib` as a universal binary (Nix store rpaths are rewritten to system paths automatically).

### Tests

```bash
cargo test  # 153 tests across all crates
```

---

## Thanks

This project would never have been possible without the work of the linux and mcsr communities as a whole, but i would like to give a special thanks to:

- **[tesselslate](https://github.com/tesselslate)** - for [waywall](https://github.com/tesselslate/waywall), which toolscreen was modeled around, and which tux injector's Lua API is based off of.
- **[jojoe77777](https://github.com/jojoe77777)** - for [toolscreen](https://github.com/jojoe77777/ToolScreen), which laid the groundwork and modeled out the idea for what an injection overlay tool should look like, and how it interacts with the game.

And to everyone who tested any early builds of tux injector, which greatly helped find and iron out various bugs from the codebase.
