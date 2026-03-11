# Injection & Hooking

Tuxinjector injects directly into the game process before any game code runs. On Linux it uses `LD_PRELOAD`, on macOS it uses `DYLD_INSERT_LIBRARIES`. It works by hooking `dlsym` to intercept symbol lookups, giving us full control over GL rendering and input without touching any game files.

---

## Interception Strategy

Minecraft (LWJGL3) resolves its GL and GLFW functions through different paths depending on the platform, and we handle all of them:

### Linux

| Resolution Path | Used By | Interception Method |
|----------------|---------|---------------------|
| `dlsym(RTLD_NEXT, ...)` | EGL/GLX swap, GL functions | Hooked `dlsym` via `dlvsym` |
| `dlopen` + PLT binding | GLFW functions (with `RTLD_DEEPBIND`) | `#[no_mangle]` PLT exports + `dlopen` hook |

LWJGL3 loads `libglfw.so` with `RTLD_DEEPBIND`, which creates a private symbol scope that completely bypasses our `dlsym` hook. The workaround is exporting `#[no_mangle]` symbols at the PLT level, so the linker resolves those before `RTLD_DEEPBIND` can do anything about it.

### macOS

| Resolution Path | Used By | Interception Method |
|----------------|---------|---------------------|
| `dlsym(RTLD_DEFAULT, ...)` | GL/GLFW functions | `__DATA,__interpose` section in Mach-O binary |

On macOS, `dlsym` itself is interposed via the `__DATA,__interpose` linker feature. This is a Mach-O mechanism where we declare a struct pairing our replacement function with the original - the dynamic linker patches all call sites at load time. Unlike PLT hooking on Linux, this works across all loaded images (frameworks, dylibs, the main executable) without needing to strip `RTLD_DEEPBIND` since that flag doesn't exist on macOS.

The real `dlsym` pointer is read via `read_volatile` from the interpose struct to avoid recursion, since `__interpose` redirects ALL images including our own.

---

## dlsym Hook

The core of everything is the `dlsym` hook. Since we've interposed `dlsym` itself, we need a way to call the *real* one without recursing into ourselves.

### Linux: Resolving the Real dlsym

```rust
dlsym {
    resolve_real_dlsym: DlsymFn    // Resolved via dlvsym(RTLD_NEXT, "dlsym", "GLIBC_2.34")
}
```

Tries `GLIBC_2.34` first, falls back to `GLIBC_2.2.5` for older glibc. `dlvsym` (versioned symbol lookup) bypasses our hook since we only interposed the unversioned `dlsym`.

### macOS: Resolving the Real dlsym

```rust
dlsym {
    // Read from the __interpose struct via read_volatile
    // The linker fills in the original pointer at load time
    real_dlsym = read_volatile(addr_of!(INTERPOSE_DLSYM.original))
}
```

### Intercepted Symbols

When the game does `dlsym(handle, "eglSwapBuffers")`, our hook:

1. Calls the real `dlsym` to get the actual function pointer
2. Stashes that pointer in an `AtomicPtr` for later
3. Returns our wrapper function instead

```
dlsym("eglSwapBuffers")  ->  stash real ptr  ->  return hooked_egl_swap_buffers
dlsym("glXSwapBuffers")  ->  stash real ptr  ->  return hooked_glx_swap_buffers
dlsym("glfwSetKeyCallback")  ->  stash real ptr  ->  return hooked_set_key_callback
dlsym("glfwGetKey")  ->  stash real ptr  ->  return glfwGetKey (PLT export)
dlsym("glViewport")  ->  stash real ptr  ->  return glViewport (viewport hook)
...
```

On macOS, the swap hook intercepts `glfwSwapBuffers` instead of EGL/GLX swap functions (since macOS doesn't have EGL or GLX).

Full list of everything we intercept:

| Category | Symbols |
|----------|---------|
| **Swap** | `eglSwapBuffers`, `glXSwapBuffers` (Linux); `glfwSwapBuffers` (macOS); `eglGetProcAddress`, `glXGetProcAddressARB` |
| **GLFW callbacks** | `glfwSetKeyCallback`, `glfwSetMouseButtonCallback`, `glfwSetCursorPosCallback`, `glfwSetScrollCallback`, `glfwSetCharCallback`, `glfwSetCharModsCallback`, `glfwSetInputMode`, `glfwSetFramebufferSizeCallback` |
| **GLFW polling** | `glfwGetKey`, `glfwGetMouseButton`, `glfwGetCursorPos`, `glfwGetFramebufferSize`, `glfwGetProcAddress` |
| **GL functions** | `glViewport`, `glScissor`, `glBindFramebuffer` (+ EXT/ARB), `glDrawBuffer`, `glReadBuffer`, `glDrawBuffers`, `glBlitFramebuffer` |

---

## dlopen Hook (Linux only)

LWJGL3 loads its JNI libraries with `RTLD_DEEPBIND`, which would hide our PLT exports. Pretty simple fix - just strip `RTLD_DEEPBIND` from the flags before forwarding to the real `dlopen`:

```
dlopen(path, flags):
    clean_flags = flags & ~RTLD_DEEPBIND
    return real_dlopen(path, clean_flags)
```

Without `RTLD_DEEPBIND`, LWJGL3's GLFW JNI bindings resolve from the global namespace where our `#[no_mangle]` exports are.

This isn't needed on macOS since `RTLD_DEEPBIND` doesn't exist there.

---

## PLT-Level Exports (Linux only)

GLFW functions that get resolved through direct PLT binding (not `dlsym`) need separate `#[no_mangle]` exports with the same name and ABI:

```rust
#[no_mangle]
pub unsafe extern "C" fn glfwSetKeyCallback(
    window: GlfwWindow,
    callback: GlfwKeyCallback,
) -> GlfwKeyCallback {
    // Resolve real glfwSetKeyCallback via RTLD_NEXT (once)
    // Intercept: stash game callback, install our wrapper
    callbacks::intercept_set_key_callback(window, callback)
}
```

Each export resolves the real function via `libc::dlsym(RTLD_NEXT, ...)` on first call, which also routes through our hooked `dlsym` to store the real pointer as a side-effect.

On macOS, `__DATA,__interpose` handles all symbol interposition uniformly, so PLT exports aren't needed.

---

## glfwGetProcAddress Hook

Minecraft uses `glfwGetProcAddress` to resolve GL function pointers at runtime. We intercept this to hook GL functions that aren't reachable through `dlsym`:

```
Game: glfwGetProcAddress("glViewport")
Hook: call real glfwGetProcAddress("glViewport") -> store real ptr
      return glViewport hook function

Game: glfwGetProcAddress("glBindFramebuffer")
Hook: call real -> store real ptr -> return hook

Game: glfwGetProcAddress("anything_else")
Hook: forward to real unchanged
```

Covers `glViewport`, `glScissor`, `glBindFramebuffer` (and EXT/ARB variants), `glDrawBuffer`, `glReadBuffer`, `glDrawBuffers`, and `glBlitFramebuffer`.

---

## Hook Chaining (Linux only)

When multiple `LD_PRELOAD` libraries hook the same symbols, there are two forwarding modes:

| Mode | Behavior |
|------|----------|
| **Original function** (default) | Resolve the real function directly from the driver library (`libEGL.so`, `libGLX.so`) via `RTLD_NOLOAD`, bypassing other hooks |
| **RTLD_NEXT** | Forward to the next hook in the `LD_PRELOAD` chain |

Original function mode is preferred since it sidesteps compatibility issues with other overlays (MangoHud, etc.) that might also hook swap functions. Configurable via `advanced.disable_hook_chaining`.

---

## First-Frame Initialisation

All the heavy init is deferred to the first frame, when the GL context is actually current. Before that point there's no GL context, so creating shaders/textures/FBOs would just fail.

```
First SwapBuffers call:
    1. Resolve GL function pointers via eglGetProcAddress/glXGetProcAddressARB (Linux)
       or glfwGetProcAddress (macOS)
    2. Create GlOverlayRenderer (compile shaders, allocate FBOs)
    3. Load config from ~/.config/tuxinjector/init.lua
    4. Register input handler with hotkey engine
    5. Discover and load plugins from ~/.local/share/tuxinjector/plugins/
    6. Install inline glViewport/glBindFramebuffer hooks (runtime patching)
    7. INITIALIZED = true
```

On macOS, the shader compilation step also patches all GLSL 300 ES shaders down to GLSL 1.20 at runtime (`in`/`out` -> `attribute`/`varying`, `texture()` -> `texture2D()`, etc.) since Apple's GL context is limited to 2.1.

Every subsequent swap call checks `INITIALIZED` before rendering. If init fails, the game keeps running normally without the overlay.

---

## Function Pointer Storage

All real function pointers live in `AtomicPtr<c_void>` statics with `Ordering::Release` on store and `Ordering::Acquire` on load, so they're guaranteed visible across threads.

```rust
static REAL_EGL_SWAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn store_real_egl_swap(ptr: *mut c_void) {
    REAL_EGL_SWAP.store(ptr, Ordering::Release);
}

// In the hooked function:
let ptr = REAL_EGL_SWAP.load(Ordering::Acquire);
let real_fn: EglSwapBuffersFn = std::mem::transmute(ptr);
real_fn(display, surface)
```

Same pattern for every hooked function. The `AtomicPtr` makes sure pointers stored on the game's main thread are safely readable from the render thread.
