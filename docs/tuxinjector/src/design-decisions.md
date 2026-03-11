# Design Decisions

This chapter explains a couple architectural decisions, and why i choose to do what i did.

---

## Why LD_PRELOAD / DYLD_INSERT_LIBRARIES?

`LD_PRELOAD` (Linux) and `DYLD_INSERT_LIBRARIES` (macOS) are the simplest injection mechanisms on their respective platforms. They load tuxinjector's shared library into the game process before any game code runs, with no need for process attachment, memory writes, or elevated privileges.

`ptrace` would let you attach to an already-running process, but it needs `CAP_SYS_PTRACE` or equivalent permissions, gets blocked by security modules (AppArmor, SELinux, Yama), and is way more complex to get right across different kernel versions. Since Minecraft is always launched from a script or launcher anyway, the preload env var can just be set at launch time with zero special permissions.

---

## Why dlsym hooking instead of GOT/PLT patching?

GOT/PLT patching works by modifying the game's Global Offset Table in memory to redirect function calls. It works fine, but you end up needing to parse ELF headers at runtime, find the right GOT entry, and mess with memory pages that might be read-only (`mprotect`).

`dlsym` interposition is way cleaner: on Linux we export our own `dlsym` with `#[no_mangle]`, and the dynamic linker resolves it before `libdl.so`'s version. On macOS we use `__DATA,__interpose`, a Mach-O linker feature that patches all call sites at load time. Either way, every symbol lookup in the process flows through ours, including third-party libraries. None of the ELF parsing or `mprotect` headaches.

The one exception on Linux is `glfwGetProcAddress`, which is handled via PLT exports because LWJGL3's `RTLD_DEEPBIND` would bypass `dlsym` interception for symbols resolved through GLFW's own loader. This isn't needed on macOS since `RTLD_DEEPBIND` doesn't exist there.

---

## Why direct GL rendering instead of Vulkan?

An early version of tuxinjector used a full Vulkan renderer with GL-to-Vulkan interop for compositing. It got removed entirely in favor of direct GL rendering:

| Aspect | Vulkan Renderer | Direct GL |
|--------|----------------|-----------|
| Pipeline sync | Vulkan semaphore + GL fence per frame | None (same context) |
| GPU overhead | ~1.2ms on Intel Arc B580 (Mesa xe) | ~0.1ms |
| Code complexity | ~3000 lines (ash + shaderc + interop) | ~800 lines |
| Driver support | Requires Vulkan + GL interop extensions | OpenGL only |
| Build dependencies | ash, shaderc (C++ compiler required) | None |

The performance root cause on Intel Arc was `glCopyTexSubImage2D` from the game's FBO, which triggered an implicit GPU pipeline sync per frame. Direct GL rendering with zero-copy `TextureRef` eliminates this entirely by binding the game's FBO texture directly without any copy operation.
Not really sure how Vulkan interacts with other Drivers, but GL tends to be better for this kind of stuff (sadly)

---

## Why PLT exports for GLFW? (Linux)

LWJGL3 loads `libglfw.so` with `dlopen(..., RTLD_DEEPBIND)`. `RTLD_DEEPBIND` creates a private symbol scope where the loaded library resolves symbols from its own scope first, then global, which bypasses any `LD_PRELOAD` hooks.

So `dlsym` interception alone isn't enough for GLFW functions. The `#[no_mangle]` PLT exports work because:

1. The `dlopen` hook strips `RTLD_DEEPBIND` from the flags
2. Without `RTLD_DEEPBIND`, the linker resolves GLFW symbols from the global namespace
3. Tuxinjector's `#[no_mangle]` exports are in the global namespace (loaded first via `LD_PRELOAD`)
4. LWJGL3's GLFW calls bind to tuxinjector's wrappers

So it's a dual-path thing: `dlsym` hook catches lookups made through `dlsym()`, and PLT exports catch the rest via direct dynamic linking.

On macOS, none of this is needed - `__DATA,__interpose` handles all symbol interposition uniformly, and `RTLD_DEEPBIND` doesn't exist on Darwin.

---

## Why GLSL patching on macOS?

Apple's OpenGL implementation is stuck at version 2.1 (compatibility context) or 4.1 (core context). Minecraft uses a compatibility context, so we're limited to GL 2.1 and GLSL 1.20. Rather than maintaining two separate shader sets, tuxinjector writes all shaders as GLSL 300 ES and patches them at runtime on macOS:

- `#version 300 es` -> `#version 120` + `#extension GL_EXT_gpu_shader4`
- `in`/`out` -> `attribute`/`varying`
- `texture()` -> `texture2D()`
- `FragColor` -> `gl_FragColor`
- `textureSize()` -> `textureSize2D()`

Ugly but it works, and it means all the shader code is in one place.
