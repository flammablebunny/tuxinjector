# OpenGL Rendering

The overlay renders directly into the game's OpenGL backbuffer - no separate render thread, no off-screen compositing, no Vulkan layer. Everything is drawn on the game thread inside the SwapBuffers hook, right before the frame is presented.

---

## Render Pipeline

```
Every frame (inside hooked SwapBuffers):

1. first_frame_init()                        // One-time deferred init (shaders, FBOs, etc.)
2. frame_limit(fps_limit, spin_threshold)    // Optional FPS limiter
3. render_overlay():
   a. capture_original_size_if_needed()      // Read GL_VIEWPORT for physical surface size
   b. poll_borderless_toggle()               // Check for pending borderless window toggles
   c. process_lua_commands()                 // Dispatch queued Lua actions
   d. center_game_content() [if mode resize] // FBO blit for undersized/oversized modes
   e. glViewport(0, 0, width, height)        // Set viewport to physical surface size
   f. overlay.render_and_composite(w, h)     // Build scene -> draw elements -> GUI on top
4. select_swap_ptr()                         // Choose original or RTLD_NEXT swap fn
5. real_swap(display, surface)               // Forward to real SwapBuffers
```

---

## Shader Programs

5 shader programs, written as GLSL 300 ES and patched to GLSL 1.20 at runtime on macOS (Apple's GL 2.1 compatibility context):

| Program | Vertex | Fragment | Used For |
|---------|--------|----------|----------|
| Solid | Full-screen triangle from `gl_VertexID` | `uniform vec4 uColor` | Solid color backgrounds |
| Gradient | Full-screen triangle + UV pass | Animated gradient (6 modes) | Gradient backgrounds |
| Border | Full-screen triangle | Border distance field | Mirror/image borders |
| Passthrough | Quad vertices + UVs | `texture(uTexture, uv)` | Textured quads (mirrors, images) |
| Filter | Quad vertices + UVs | Color-key, sensitivity, gamma, custom GLSL | Filtered mirrors (chroma key, color matching, custom shaders) |

### Vertex Generation

The solid, gradient, and border shaders use a full-screen triangle generated from `gl_VertexID`, no vertex buffers needed:

```glsl
vec2 pos = vec2(
    float((gl_VertexID & 1) * 4 - 1),
    float((gl_VertexID & 2) * 2 - 1)
);
gl_Position = vec4(pos, 0.0, 1.0);
```

3 vertices cover the whole screen, and the scissor test clips to the element's bounding rect. Avoids per-element VAO setup entirely.

The passthrough and filter shaders use a quad VBO with position and UV attributes. UVs are transformed via `uv_rect` for subregion sampling from game textures.

---

## Scene Composition

Each frame builds a `SceneDescription` with a flat list of `SceneElement` variants:

```
SceneDescription {
    clear_color:  [f32; 4]
    elements:     Vec<SceneElement>
    time:         f32               // Seconds since overlay init (for custom shaders)
}

SceneElement:
    | SolidRect   { x, y, w, h, color }
    | Gradient    { color1, color2, angle, time, animation_type, scissor }
    | Border      { x, y, w, h, border_width, radius, color }
    | Textured    { x, y, w, h, pixels, tex_width, tex_height, circle_clip,
                    nearest_filter, filter_*, custom_shader }
    | TextureRef  { x, y, w, h, gl_texture, tex_width, tex_height, flip_v,
                    circle_clip, nearest_filter, filter_*, uv_rect, custom_shader }
    | GuiOverlay  { pixels, width, height }
    | ClearRect   { x, y, w, h }
```

`Textured` and `TextureRef` both carry filter fields: `filter_target_colors`, `filter_output_color`, `filter_sensitivity`, `filter_color_passthrough`, `filter_border_color`, `filter_border_width`, `filter_gamma_mode`, and an optional `custom_shader` for user GLSL.

Elements are drawn back-to-front, each dispatched to the right shader program.

---

## Zero-Copy Mirror Rendering

Mirrors capture a region of the game's framebuffer and display it elsewhere on screen. There are two rendering paths depending on the mirror config:

### TextureRef Path (Zero-Copy)

Single-input mirrors bind the game's FBO texture directly via `SceneElement::TextureRef`. No GPU copy, no PBO readback, no CPU involvement:

```
Game renders to Sodium's FBO -> texture ID stored
Mirror render: bind texture ID -> sample with UV subregion -> draw quad

Cost: 0 extra GPU copies, 0 CPU readback
```

`uv_rect` specifies the subregion to sample, calculated from the mirror's capture coordinates relative to the game's render resolution. `flip_v: true` because GL framebuffer textures have bottom-up orientation.

### Textured Path (CPU Readback)

Multi-input mirrors (combining multiple capture regions) and mirrors with filter effects that need CPU-side color matching use PBO async readback:

```
1. Bind game FBO as GL_READ_FRAMEBUFFER
2. glReadPixels -> PBO (async, non-blocking)
3. Map PBO -> CPU pixel buffer
4. has_matching_pixels() -> check filter visibility (CPU)
5. Upload to texture -> SceneElement::Textured
```

Multi-input mirrors use `capture_multi_from()` which blits multiple source regions into a single FBO before readback.

---

## Game FBO Discovery

Minecraft (via Sodium/Iris) renders to an internal FBO, not the default framebuffer. Tuxinjector finds the game's render FBO by scanning FBO IDs and checking their color attachment dimensions:

```
find_game_fbo_and_texture(gl, mode_w, mode_h):
    for id in 1..=64:
        if glIsFramebuffer(id) == 0: continue
        glBindFramebuffer(GL_FRAMEBUFFER, id)
        if glCheckFramebufferStatus != GL_FRAMEBUFFER_COMPLETE: continue

        obj_type = glGetFramebufferAttachmentParameteriv(COLOR_ATTACHMENT0, OBJECT_TYPE)
        if obj_type != GL_TEXTURE: continue

        tex_name = glGetFramebufferAttachmentParameteriv(COLOR_ATTACHMENT0, OBJECT_NAME)
        tex_w, tex_h = glGetTexLevelParameteriv(tex_name, GL_TEXTURE_WIDTH/HEIGHT)

        if tex_w == mode_w && tex_h == mode_h:
            return (id, tex_name)

    return (0, 0)
```

The returned FBO and texture IDs are used for zero-copy mirror rendering and content centering.

---

## Mode System & Content Centering

The mode system lets you switch between viewport resolutions (e.g. 1920x1080 fullscreen vs 640x480 for practice). When the mode doesn't match the physical surface size, the game content gets centered with an FBO blit:

```
center_game_content(gl, mode_w, mode_h, surface_w, surface_h):
    1. Compute src/dst offsets for centering
    2. Copy game pixels from backbuffer -> temp FBO
    3. Clear backbuffer to black
    4. Blit temp FBO -> centered position in backbuffer
```

For oversized modes (resolution larger than the surface), the game's internal FBO is used as the read source and the center slice is extracted.

---

## FPS Limiter

On Linux, uses `clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME)` for the bulk of the wait, then a spin-loop to absorb scheduler jitter. On macOS, falls back to `thread_sleep` with equivalent logic:

```
frame_limit(fps_limit, spin_threshold_us):
    frame_ns = 1_000_000_000 / fps_limit
    target = NEXT_FRAME_NS (monotonic timestamp)

    if target > now:
        sleep until (target - spin_threshold)    // clock_nanosleep absolute
        spin until target                         // spin_loop() for sub-ms accuracy

    // Advance target; resync if more than one frame behind
    NEXT_FRAME_NS = if stutter then now + frame_ns else target + frame_ns
```

Spin threshold is configurable - higher values give more precise timing at the cost of CPU usage.

---

## GL State Management

The renderer saves and restores all GL state it touches, so the overlay can't corrupt the game's rendering:

| State | Saved Before | Restored After |
|-------|-------------|----------------|
| Current program | `glGetIntegerv(GL_CURRENT_PROGRAM)` | `glUseProgram(saved)` |
| Active texture | `glGetIntegerv(GL_ACTIVE_TEXTURE)` | `glActiveTexture(saved)` |
| Bound textures | Per-unit `GL_TEXTURE_BINDING_2D` | `glBindTexture` |
| Bound FBOs | `GL_DRAW/READ_FRAMEBUFFER_BINDING` | `glBindFramebuffer` |
| Viewport | `GL_VIEWPORT` | `glViewport` |
| Scissor | `GL_SCISSOR_BOX` + enabled state | `glScissor` + enable/disable |
| Blend | Enabled + func/equation | `glBlendFunc` + enable/disable |
| Depth test | Enabled state | Enable/disable |
| Cull face | Enabled state | Enable/disable |
| VAO | `GL_VERTEX_ARRAY_BINDING` | `glBindVertexArray` |
| VBO | `GL_ARRAY_BUFFER_BINDING` | `glBindBuffer` |

All of this lives in `gl_state.rs` and wraps every `draw_scene()` call.
