// Direct GL overlay renderer - draws scene elements straight into the
// game's backbuffer. Everything runs on the game thread inside the
// SwapBuffers hook.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::ptr;

use crate::compositor::link_program;
use crate::gl_bindings::*;
use crate::gl_state;

// ---- Shaders (GLSL 300 ES) ----
//
// We use 300 ES because that's what Minecraft targets. The fullscreen-triangle
// trick (gl_VertexID) avoids needing a VBO for solid/gradient/border draws.

const SOLID_VERT: &str = r#"#version 300 es
precision highp float;
void main() {
    vec2 pos = vec2(
        float((gl_VertexID & 1) * 4 - 1),
        float((gl_VertexID & 2) * 2 - 1)
    );
    gl_Position = vec4(pos, 0.0, 1.0);
}
"#;

const SOLID_FRAG: &str = r#"#version 300 es
precision highp float;
uniform vec4 uColor;
out vec4 FragColor;
void main() {
    FragColor = uColor;
}
"#;

const GRADIENT_VERT: &str = r#"#version 300 es
precision highp float;
out vec2 fragUV;
void main() {
    vec2 pos = vec2(
        float((gl_VertexID & 1) * 4 - 1),
        float((gl_VertexID & 2) * 2 - 1)
    );
    gl_Position = vec4(pos, 0.0, 1.0);
    // GL y-flip: NDC y=-1 is screen bottom, but UV y=0 should be screen top
    fragUV = vec2(pos.x * 0.5 + 0.5, 0.5 - pos.y * 0.5);
}
"#;

const GRADIENT_FRAG: &str = r#"#version 300 es
precision highp float;
uniform vec4 uColor1;
uniform vec4 uColor2;
uniform float uAngle;
uniform float uTime;
uniform int uAnimationType;
in vec2 fragUV;
out vec4 FragColor;
void main() {
    vec2 uv = fragUV;
    vec2 centred = uv - 0.5;
    float t = 0.0;
    if (uAnimationType == 1) {
        float a = uAngle + uTime;
        t = dot(uv, vec2(cos(a), sin(a)));
    } else if (uAnimationType == 2) {
        float cosA = cos(uAngle);
        float sinA = sin(uAngle);
        t = fract(dot(uv, vec2(cosA, sinA)) + uTime * 0.2);
    } else if (uAnimationType == 3) {
        float cosA = cos(uAngle);
        float sinA = sin(uAngle);
        float base = dot(uv, vec2(cosA, sinA));
        float perp = dot(uv, vec2(-sinA, cosA));
        t = base + 0.05 * sin(perp * 12.0 + uTime * 3.0);
    } else if (uAnimationType == 4) {
        float r = length(centred) * 2.0;
        float a = atan(centred.y, centred.x);
        t = fract(r + a / 6.28318 + uTime * 0.3);
    } else if (uAnimationType == 5) {
        float cosA = cos(uAngle);
        float sinA = sin(uAngle);
        float base = dot(uv, vec2(cosA, sinA));
        float fade = sin(uTime) * 0.5 + 0.5;
        t = mix(base, fade, 0.5);
    } else {
        float cosA = cos(uAngle);
        float sinA = sin(uAngle);
        t = dot(uv, vec2(cosA, sinA));
    }
    t = clamp(t, 0.0, 1.0);
    FragColor = mix(uColor1, uColor2, t);
}
"#;

const BORDER_VERT: &str = r#"#version 300 es
precision highp float;
uniform vec2 uResolution;
out vec2 fragCoord;
void main() {
    vec2 pos = vec2(
        float((gl_VertexID & 1) * 4 - 1),
        float((gl_VertexID & 2) * 2 - 1)
    );
    gl_Position = vec4(pos, 0.0, 1.0);
    // GL y-flip: map so fragCoord y=0 = screen top
    fragCoord = vec2(pos.x * 0.5 + 0.5, 0.5 - pos.y * 0.5) * uResolution;
}
"#;

const BORDER_FRAG: &str = r#"#version 300 es
precision highp float;
uniform vec4 uColor;
uniform vec4 uRect;
uniform float uBorderWidth;
uniform float uRadius;
uniform vec2 uResolution;
in vec2 fragCoord;
out vec4 FragColor;

float sdRoundedRect(vec2 p, vec2 halfSize, float r) {
    vec2 q = abs(p) - halfSize + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 centre = uRect.xy + uRect.zw * 0.5;
    vec2 halfSize = uRect.zw * 0.5;
    float r = min(uRadius, min(halfSize.x, halfSize.y));
    float d = sdRoundedRect(fragCoord - centre, halfSize, r);
    float halfBW = uBorderWidth * 0.5;
    float outer_mask = 1.0 - smoothstep(halfBW - 0.5, halfBW + 0.5, d);
    float inner_mask = smoothstep(-halfBW - 0.5, -halfBW + 0.5, d);
    float mask = outer_mask * inner_mask;
    if (mask <= 0.0) {
        discard;
    }
    FragColor = vec4(uColor.rgb, uColor.a * mask);
}
"#;

const PASSTHROUGH_VERT: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 aPos;
layout(location = 1) in vec2 aTexCoord;
out vec2 fragTexCoord;
void main() {
    gl_Position = vec4(aPos, 0.0, 1.0);
    fragTexCoord = aTexCoord;
}
"#;

const PASSTHROUGH_FRAG: &str = r#"#version 300 es
precision highp float;
uniform sampler2D uTexture;
uniform int uCircleClip;
in vec2 fragTexCoord;
out vec4 FragColor;
void main() {
    if (uCircleClip != 0) {
        vec2 uv = fragTexCoord - vec2(0.5);
        if (dot(uv, uv) > 0.25) {
            discard;
        }
    }
    FragColor = texture(uTexture, fragTexCoord);
}
"#;

const FILTER_VERT: &str = r#"#version 300 es
precision highp float;
layout(location = 0) in vec2 aPos;
layout(location = 1) in vec2 aTexCoord;
out vec2 fragTexCoord;
void main() {
    gl_Position = vec4(aPos, 0.0, 1.0);
    fragTexCoord = aTexCoord;
}
"#;

const FILTER_FRAG: &str = r#"#version 300 es
precision highp float;
uniform sampler2D uTexture;
uniform vec4 uTargetColors[4];
uniform vec4 uOutputColor;
uniform vec4 uBorderColor;
uniform float uSensitivity;
uniform int uColorCount;
uniform int uColorPassthrough;
uniform int uBorderWidth;
uniform vec2 uScreenPixel;
uniform int uGammaMode;
uniform vec4 uUVBounds;
in vec2 fragTexCoord;
out vec4 FragColor;

vec4 sampleTex(vec2 uv) {
    if (uUVBounds.z > 0.0) {
        if (uv.x < uUVBounds.x || uv.x > uUVBounds.z ||
            uv.y < uUVBounds.y || uv.y > uUVBounds.w) {
            return vec4(0.0);
        }
    }
    return texture(uTexture, uv);
}

vec3 SRGBToLinear(vec3 c) {
    bvec3 cutoff = lessThanEqual(c, vec3(0.04045));
    vec3 low = c / 12.92;
    vec3 high = pow((c + 0.055) / 1.055, vec3(2.4));
    return mix(high, low, vec3(cutoff));
}

bool matchesTarget(vec3 rgb) {
    vec3 rgbLinear = SRGBToLinear(rgb);
    for (int i = 0; i < 4; i++) {
        if (i >= uColorCount) break;
        vec3 targetSRGB = uTargetColors[i].rgb;
        vec3 targetLinear = SRGBToLinear(targetSRGB);
        float dist;
        if (uGammaMode == 2) {
            dist = distance(rgb, targetLinear);
        } else if (uGammaMode == 1) {
            dist = distance(rgbLinear, targetLinear);
        } else {
            float distSRGB = distance(rgb, targetSRGB);
            float distLinear = distance(rgbLinear, targetLinear);
            dist = min(distSRGB, distLinear);
        }
        if (dist < uSensitivity) {
            return true;
        }
    }
    return false;
}

void main() {
    vec4 texel = sampleTex(fragTexCoord);
    if (uColorCount <= 0) {
        FragColor = texel;
        return;
    }
    if (matchesTarget(texel.rgb)) {
        if (uColorPassthrough != 0) {
            FragColor = vec4(texel.rgb, 1.0);
        } else {
            FragColor = vec4(uOutputColor.rgb, 1.0);
        }
        return;
    }
    if (uBorderWidth > 0) {
        vec2 texSize = vec2(textureSize(uTexture, 0));
        for (int dx = -uBorderWidth; dx <= uBorderWidth; dx++) {
            for (int dy = -uBorderWidth; dy <= uBorderWidth; dy++) {
                if (dx == 0 && dy == 0) continue;
                vec2 offset = vec2(float(dx), float(dy)) * uScreenPixel;
                vec2 neighborUV = fragTexCoord + offset;
                vec2 snapped = (floor(neighborUV * texSize) + 0.5) / texSize;
                vec3 neighbor = sampleTex(snapped).rgb;
                if (matchesTarget(neighbor)) {
                    FragColor = uBorderColor;
                    return;
                }
            }
        }
    }
    discard;
}
"#;

// ---- Uniform location caches ----

struct SolidLocs {
    color: GLint,
}

struct GradientLocs {
    color1: GLint,
    color2: GLint,
    angle: GLint,
    time: GLint,
    anim_type: GLint,
}

struct BorderLocs {
    color: GLint,
    rect: GLint,
    border_w: GLint,
    radius: GLint,
    resolution: GLint,
}

struct PassthroughLocs {
    tex: GLint,
    circle_clip: GLint,
}

struct FilterLocs {
    tex: GLint,
    target_colors: [GLint; 4],
    output_color: GLint,
    border_color: GLint,
    sensitivity: GLint,
    color_count: GLint,
    color_passthrough: GLint,
    border_w: GLint,
    screen_px: GLint,
    gamma_mode: GLint,
    uv_bounds: GLint,
}

// ---- GlOverlayRenderer ----

// How often (in frames) we re-query the game's GL state for corrections.
// State at SwapBuffers is pretty stable per-screen, so no need to do it every frame.
const STATE_CACHE_INTERVAL: u64 = 120;

// Compiled custom fragment shader with cached locations
struct CustomProgram {
    id: GLuint,
    tex_loc: GLint,
    time_loc: GLint,
    res_loc: GLint,
    src_hash: u64,
}

pub struct GlOverlayRenderer {
    solid_prog: GLuint,
    gradient_prog: GLuint,
    border_prog: GLuint,
    passthrough_prog: GLuint,
    filter_prog: GLuint,

    solid_locs: SolidLocs,
    gradient_locs: GradientLocs,
    border_locs: BorderLocs,
    pt_locs: PassthroughLocs,
    filt_locs: FilterLocs,

    quad_vao: GLuint,
    quad_vbo: GLuint,

    gui_tex: GLuint,
    gui_w: u32,
    gui_h: u32,

    tex_pool: Vec<GLuint>,
    pool_idx: usize,

    custom_progs: HashMap<String, CustomProgram>,

    // cached game state for zero-query restore on most frames
    cached_state: Option<gl_state::TargetedGlState>,
    state_frame: u64,
}

impl GlOverlayRenderer {
    /// Compile all shader programs and create shared GL objects
    pub unsafe fn new(gl: &GlFns) -> Result<Self, String> {
        let solid_prog = link_program(gl, SOLID_VERT, SOLID_FRAG)?;
        let gradient_prog = link_program(gl, GRADIENT_VERT, GRADIENT_FRAG)?;
        let border_prog = link_program(gl, BORDER_VERT, BORDER_FRAG)?;
        let passthrough_prog = link_program(gl, PASSTHROUGH_VERT, PASSTHROUGH_FRAG)?;
        let filter_prog = link_program(gl, FILTER_VERT, FILTER_FRAG)?;

        let solid_locs = SolidLocs {
            color: (gl.get_uniform_location)(solid_prog, b"uColor\0".as_ptr() as *const _),
        };

        let gradient_locs = GradientLocs {
            color1: (gl.get_uniform_location)(gradient_prog, b"uColor1\0".as_ptr() as *const _),
            color2: (gl.get_uniform_location)(gradient_prog, b"uColor2\0".as_ptr() as *const _),
            angle: (gl.get_uniform_location)(gradient_prog, b"uAngle\0".as_ptr() as *const _),
            time: (gl.get_uniform_location)(gradient_prog, b"uTime\0".as_ptr() as *const _),
            anim_type: (gl.get_uniform_location)(gradient_prog, b"uAnimationType\0".as_ptr() as *const _),
        };

        let border_locs = BorderLocs {
            color: (gl.get_uniform_location)(border_prog, b"uColor\0".as_ptr() as *const _),
            rect: (gl.get_uniform_location)(border_prog, b"uRect\0".as_ptr() as *const _),
            border_w: (gl.get_uniform_location)(border_prog, b"uBorderWidth\0".as_ptr() as *const _),
            radius: (gl.get_uniform_location)(border_prog, b"uRadius\0".as_ptr() as *const _),
            resolution: (gl.get_uniform_location)(border_prog, b"uResolution\0".as_ptr() as *const _),
        };

        let pt_locs = PassthroughLocs {
            tex: (gl.get_uniform_location)(passthrough_prog, b"uTexture\0".as_ptr() as *const _),
            circle_clip: (gl.get_uniform_location)(passthrough_prog, b"uCircleClip\0".as_ptr() as *const _),
        };

        let filt_locs = FilterLocs {
            tex: (gl.get_uniform_location)(filter_prog, b"uTexture\0".as_ptr() as *const _),
            target_colors: [
                (gl.get_uniform_location)(filter_prog, b"uTargetColors[0]\0".as_ptr() as *const _),
                (gl.get_uniform_location)(filter_prog, b"uTargetColors[1]\0".as_ptr() as *const _),
                (gl.get_uniform_location)(filter_prog, b"uTargetColors[2]\0".as_ptr() as *const _),
                (gl.get_uniform_location)(filter_prog, b"uTargetColors[3]\0".as_ptr() as *const _),
            ],
            output_color: (gl.get_uniform_location)(filter_prog, b"uOutputColor\0".as_ptr() as *const _),
            border_color: (gl.get_uniform_location)(filter_prog, b"uBorderColor\0".as_ptr() as *const _),
            sensitivity: (gl.get_uniform_location)(filter_prog, b"uSensitivity\0".as_ptr() as *const _),
            color_count: (gl.get_uniform_location)(filter_prog, b"uColorCount\0".as_ptr() as *const _),
            color_passthrough: (gl.get_uniform_location)(filter_prog, b"uColorPassthrough\0".as_ptr() as *const _),
            border_w: (gl.get_uniform_location)(filter_prog, b"uBorderWidth\0".as_ptr() as *const _),
            screen_px: (gl.get_uniform_location)(filter_prog, b"uScreenPixel\0".as_ptr() as *const _),
            gamma_mode: (gl.get_uniform_location)(filter_prog, b"uGammaMode\0".as_ptr() as *const _),
            uv_bounds: (gl.get_uniform_location)(filter_prog, b"uUVBounds\0".as_ptr() as *const _),
        };

        // shared textured-quad VAO/VBO - updated per element
        let mut quad_vao: GLuint = 0;
        let mut quad_vbo: GLuint = 0;
        (gl.gen_vertex_arrays)(1, &mut quad_vao);
        (gl.gen_buffers)(1, &mut quad_vbo);

        (gl.bind_vertex_array)(quad_vao);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, quad_vbo);

        // 6 verts * 4 floats each, we reupload every draw
        let buf_sz = (6 * 4 * std::mem::size_of::<f32>()) as GLsizeiptr;
        (gl.buffer_data)(GL_ARRAY_BUFFER, buf_sz, ptr::null(), GL_DYNAMIC_DRAW);

        let stride = (4 * std::mem::size_of::<f32>()) as GLsizei;
        (gl.enable_vertex_attrib_array)(0);
        (gl.vertex_attrib_pointer)(0, 2, GL_FLOAT, GL_FALSE, stride, ptr::null());
        (gl.enable_vertex_attrib_array)(1);
        (gl.vertex_attrib_pointer)(1, 2, GL_FLOAT, GL_FALSE, stride, (2 * std::mem::size_of::<f32>()) as *const c_void);

        (gl.bind_vertex_array)(0);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, 0);

        // persistent GUI texture
        let mut gui_tex: GLuint = 0;
        (gl.gen_textures)(1, &mut gui_tex);

        tracing::info!("GlOverlayRenderer initialized (5 programs)");

        Ok(Self {
            solid_prog,
            gradient_prog,
            border_prog,
            passthrough_prog,
            filter_prog,
            solid_locs,
            gradient_locs,
            border_locs,
            pt_locs,
            filt_locs,
            quad_vao,
            quad_vbo,
            gui_tex,
            gui_w: 0,
            gui_h: 0,
            tex_pool: Vec::new(),
            pool_idx: 0,
            custom_progs: HashMap::new(),
            cached_state: None,
            state_frame: 0,
        })
    }

    /// Recompile custom shaders when their source changes, clean up stale ones
    pub unsafe fn update_custom_shaders(&mut self, gl: &GlFns, shaders: &HashMap<String, String>) {
        // nuke programs that aren't in the new set
        let stale: Vec<String> = self.custom_progs.keys()
            .filter(|name| !shaders.contains_key(*name))
            .cloned()
            .collect();
        for name in stale {
            if let Some(p) = self.custom_progs.remove(&name) {
                (gl.delete_program)(p.id);
                tracing::info!(name = %name, "deleted custom shader");
            }
        }

        for (name, source) in shaders {
            let mut hasher = DefaultHasher::new();
            source.hash(&mut hasher);
            let hash = hasher.finish();

            if let Some(existing) = self.custom_progs.get(name) {
                if existing.src_hash == hash {
                    continue; // no change
                }
                (gl.delete_program)(existing.id);
            }

            match link_program(gl, PASSTHROUGH_VERT, source) {
                Ok(prog) => {
                    let tex_loc = (gl.get_uniform_location)(prog, b"uTexture\0".as_ptr() as *const _);
                    let time_loc = (gl.get_uniform_location)(prog, b"uTime\0".as_ptr() as *const _);
                    let res_loc = (gl.get_uniform_location)(prog, b"uResolution\0".as_ptr() as *const _);
                    self.custom_progs.insert(name.clone(), CustomProgram {
                        id: prog,
                        tex_loc,
                        time_loc,
                        res_loc,
                        src_hash: hash,
                    });
                    tracing::info!(name = %name, "compiled custom shader");
                }
                Err(e) => {
                    tracing::error!(name = %name, error = %e, "custom shader compile failed");
                    self.custom_progs.remove(name);
                }
            }
        }
    }

    /// Draw the full scene into the game's backbuffer
    pub unsafe fn draw_scene(
        &mut self,
        gl: &GlFns,
        elements: &[SceneElement],
        vp_w: u32,
        vp_h: u32,
        scene_time: f32,
    ) {
        if vp_w == 0 || vp_h == 0 {
            return;
        }

        let vp = [0, 0, vp_w as GLint, vp_h as GLint];

        // periodically re-query the game's state so our corrections stay accurate
        if self.cached_state.is_none()
            || self.state_frame % STATE_CACHE_INTERVAL == 0
        {
            self.cached_state = Some(gl_state::save_targeted_state(gl));
        }
        self.state_frame += 1;

        gl_state::set_compositor_state(gl, vp);

        // log GL context info on first frame - helps debug driver quirks
        static DIAG_ONCE: std::sync::Once = std::sync::Once::new();
        DIAG_ONCE.call_once(|| {
            while (gl.get_error)() != 0 {} // drain any stale errors

            let ver = (gl.get_string)(0x1F02 /* GL_VERSION */);
            let ren = (gl.get_string)(0x1F01 /* GL_RENDERER */);
            let ver_s = if !ver.is_null() { std::ffi::CStr::from_ptr(ver).to_string_lossy() } else { "null".into() };
            let ren_s = if !ren.is_null() { std::ffi::CStr::from_ptr(ren).to_string_lossy() } else { "null".into() };
            tracing::info!(%ver_s, %ren_s, vp_w, vp_h,
                solid = self.solid_prog, gradient = self.gradient_prog,
                border = self.border_prog, passthrough = self.passthrough_prog,
                filter = self.filter_prog, quad_vao = self.quad_vao, quad_vbo = self.quad_vbo,
                "draw_scene: GL context info");

            // quick sanity check that our VAO can actually draw something
            (gl.use_program)(self.solid_prog);
            (gl.bind_vertex_array)(self.quad_vao);
            (gl.draw_arrays)(GL_TRIANGLES, 0, 3);
            let e = (gl.get_error)();
            tracing::info!(quad_vao = self.quad_vao, draw_err = e,
                "draw_scene: VAO test (0=ok)");
        });

        // every ~300 frames, log what we're actually drawing
        if self.state_frame % 300 == 1 {
            let n_solid = elements.iter().filter(|e| matches!(e, SceneElement::SolidRect { .. })).count();
            let n_grad = elements.iter().filter(|e| matches!(e, SceneElement::Gradient { .. })).count();
            let n_tex = elements.iter().filter(|e| matches!(e, SceneElement::Textured { .. })).count();
            let n_ref = elements.iter().filter(|e| matches!(e, SceneElement::TextureRef { .. })).count();
            let n_border = elements.iter().filter(|e| matches!(e, SceneElement::Border { .. })).count();
            tracing::info!(n_solid, n_grad, n_tex, n_ref, n_border, total = elements.len(),
                vp_w, vp_h, "draw_scene: element breakdown");
        }

        // reset pixel unpack - Sodium sometimes leaves garbage here
        (gl.pixel_store_i)(GL_UNPACK_ROW_LENGTH, 0);
        (gl.pixel_store_i)(GL_UNPACK_SKIP_ROWS, 0);
        (gl.pixel_store_i)(GL_UNPACK_SKIP_PIXELS, 0);
        (gl.pixel_store_i)(GL_UNPACK_ALIGNMENT, 4);

        self.pool_idx = 0;

        let w = vp_w as f32;
        let h = vp_h as f32;

        for elem in elements {
            match elem {
                SceneElement::SolidRect { x, y, w: rw, h: rh, color } => {
                    self.draw_solid(gl, *x, *y, *rw, *rh, color, w, h);
                }
                SceneElement::Gradient { color1, color2, angle, time, animation_type, scissor } => {
                    self.draw_gradient(gl, color1, color2, *angle, *time, *animation_type, scissor.as_ref(), h);
                }
                SceneElement::Border { x, y, w: bw, h: bh, border_width, radius, color } => {
                    self.draw_border(gl, *x, *y, *bw, *bh, *border_width, *radius, color, w, h);
                }
                SceneElement::Textured {
                    x, y, w: tw, h: th,
                    tex_width, tex_height, pixels,
                    circle_clip, nearest_filter,
                    filter_target_colors, filter_output_color, filter_sensitivity,
                    filter_color_passthrough, filter_border_color,
                    filter_border_width, filter_gamma_mode,
                    custom_shader,
                } => {
                    if pixels.is_empty() || *tex_width == 0 || *tex_height == 0 {
                        continue;
                    }
                    if let Some(ref shader_name) = custom_shader {
                        self.draw_custom_textured(
                            gl, pixels, *tex_width, *tex_height,
                            *x, *y, *tw, *th, *nearest_filter,
                            shader_name, scene_time, w, h,
                        );
                    } else {
                        self.draw_textured(
                            gl, pixels, *tex_width, *tex_height,
                            *x, *y, *tw, *th,
                            *circle_clip, *nearest_filter,
                            filter_target_colors, filter_output_color, *filter_sensitivity,
                            *filter_color_passthrough, filter_border_color,
                            *filter_border_width, *filter_gamma_mode,
                            w, h,
                        );
                    }
                }
                SceneElement::GuiOverlay { pixels, width, height } => {
                    if pixels.is_empty() || *width == 0 || *height == 0 {
                        continue;
                    }
                    self.draw_gui_overlay(gl, pixels, *width, *height, w, h);
                }
                SceneElement::ClearRect { x, y, w: cw, h: ch } => {
                    self.draw_clear_rect(gl, *x, *y, *cw, *ch, h);
                }
                SceneElement::TextureRef {
                    x, y, w: rw, h: rh,
                    gl_texture, tex_width, tex_height,
                    flip_v, circle_clip, nearest_filter,
                    filter_target_colors, filter_output_color, filter_sensitivity,
                    filter_color_passthrough, filter_border_color,
                    filter_border_width, filter_gamma_mode,
                    uv_rect,
                    custom_shader,
                } => {
                    if *gl_texture == 0 || *tex_width == 0 || *tex_height == 0 {
                        continue;
                    }
                    if let Some(ref shader_name) = custom_shader {
                        self.draw_custom_tex_ref(
                            gl, *gl_texture, *tex_width, *tex_height,
                            *x, *y, *rw, *rh,
                            *flip_v, *nearest_filter,
                            shader_name, scene_time, w, h,
                            uv_rect.as_ref(),
                        );
                    } else {
                        self.draw_tex_ref(
                            gl, *gl_texture, *tex_width, *tex_height,
                            *x, *y, *rw, *rh,
                            *flip_v, *circle_clip, *nearest_filter,
                            filter_target_colors, filter_output_color, *filter_sensitivity,
                            *filter_color_passthrough, filter_border_color,
                            *filter_border_width, *filter_gamma_mode,
                            w, h,
                            uv_rect.as_ref(),
                        );
                    }
                }
            }
        }

        // fast restore + targeted corrections for things MC does differently
        gl_state::restore_minecraft_state(gl, vp);

        if let Some(ref cached) = self.cached_state {
            gl_state::restore_targeted_corrections(gl, cached);
        }
    }

    /// Force re-query next frame (e.g. after a resolution change)
    pub fn invalidate_gl_state_cache(&mut self) {
        self.cached_state = None;
    }

    // ---- Per-element draw helpers ----

    unsafe fn draw_solid(
        &self, gl: &GlFns,
        x: f32, y: f32, w: f32, h: f32,
        color: &[f32; 4],
        _vp_w: f32, vp_h: f32,
    ) {
        (gl.use_program)(self.solid_prog);
        (gl.uniform_4f)(self.solid_locs.color, color[0], color[1], color[2], color[3]);

        // GL origin is bottom-left, so flip y
        let gl_y = (vp_h - (y + h)) as GLint;
        (gl.enable)(GL_SCISSOR_TEST);
        (gl.scissor)(x as GLint, gl_y, w as GLsizei, h as GLsizei);

        // need a VAO bound or macOS GL 2.1 refuses to draw
        (gl.bind_vertex_array)(self.quad_vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 3);

        (gl.disable)(GL_SCISSOR_TEST);
    }

    unsafe fn draw_gradient(
        &self, gl: &GlFns,
        c1: &[f32; 4], c2: &[f32; 4],
        angle: f32, time: f32, anim_type: i32,
        scissor: Option<&[f32; 4]>, vp_h: f32,
    ) {
        (gl.use_program)(self.gradient_prog);
        (gl.uniform_4f)(self.gradient_locs.color1, c1[0], c1[1], c1[2], c1[3]);
        (gl.uniform_4f)(self.gradient_locs.color2, c2[0], c2[1], c2[2], c2[3]);
        (gl.uniform_1f)(self.gradient_locs.angle, angle);
        (gl.uniform_1f)(self.gradient_locs.time, time);
        (gl.uniform_1i)(self.gradient_locs.anim_type, anim_type);

        if let Some(s) = scissor {
            let gl_y = (vp_h - (s[1] + s[3])) as GLint;
            (gl.enable)(GL_SCISSOR_TEST);
            (gl.scissor)(s[0] as GLint, gl_y, s[2] as GLsizei, s[3] as GLsizei);
        }

        (gl.bind_vertex_array)(self.quad_vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 3);

        if scissor.is_some() {
            (gl.disable)(GL_SCISSOR_TEST);
        }
    }

    unsafe fn draw_border(
        &self, gl: &GlFns,
        x: f32, y: f32, w: f32, h: f32,
        bw: f32, radius: f32,
        color: &[f32; 4],
        vp_w: f32, vp_h: f32,
    ) {
        (gl.use_program)(self.border_prog);
        (gl.uniform_4f)(self.border_locs.color, color[0], color[1], color[2], color[3]);
        (gl.uniform_4f)(self.border_locs.rect, x, y, w, h);
        (gl.uniform_1f)(self.border_locs.border_w, bw);
        (gl.uniform_1f)(self.border_locs.radius, radius);
        (gl.uniform_2f)(self.border_locs.resolution, vp_w, vp_h);

        (gl.bind_vertex_array)(self.quad_vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 3);
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_textured(
        &mut self, gl: &GlFns,
        pixels: &[u8], tw: u32, th: u32,
        x: f32, y: f32, w: f32, h: f32,
        circle_clip: bool, nearest: bool,
        target_colors: &[[f32; 4]],
        output_color: &[f32; 4],
        sensitivity: f32,
        color_passthrough: bool,
        border_color: &[f32; 4],
        border_w: i32,
        gamma_mode: i32,
        vp_w: f32, vp_h: f32,
    ) {
        let tex = self.acquire_tex(gl);

        let filt = if nearest { GL_NEAREST } else { GL_LINEAR };
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filt);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filt);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        (gl.tex_image_2d)(
            GL_TEXTURE_2D, 0, GL_RGBA8 as GLint,
            tw as GLsizei, th as GLsizei, 0,
            GL_RGBA, GL_UNSIGNED_BYTE,
            pixels.as_ptr() as *const c_void,
        );

        let verts = quad_vertices(x, y, w, h, vp_w, vp_h);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, self.quad_vbo);
        (gl.buffer_data)(
            GL_ARRAY_BUFFER,
            std::mem::size_of_val(&verts) as GLsizeiptr,
            verts.as_ptr() as *const c_void,
            GL_DYNAMIC_DRAW,
        );

        let use_filter = !target_colors.is_empty();

        if use_filter {
            self.bind_filter_uniforms(gl, target_colors, output_color, sensitivity,
                color_passthrough, border_color, border_w, gamma_mode, w, h, None);
        } else {
            (gl.use_program)(self.passthrough_prog);
            (gl.uniform_1i)(self.pt_locs.tex, 0);
            (gl.uniform_1i)(self.pt_locs.circle_clip, circle_clip as GLint);
        }

        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.bind_vertex_array)(self.quad_vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 6);
        (gl.bind_vertex_array)(0);
    }

    unsafe fn draw_gui_overlay(
        &mut self, gl: &GlFns,
        pixels: &[u8], width: u32, height: u32,
        vp_w: f32, vp_h: f32,
    ) {
        // reallocate texture if size changed
        if width != self.gui_w || height != self.gui_h {
            (gl.bind_texture)(GL_TEXTURE_2D, self.gui_tex);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (gl.tex_image_2d)(
                GL_TEXTURE_2D, 0, GL_RGBA8 as GLint,
                width as GLsizei, height as GLsizei, 0,
                GL_RGBA, GL_UNSIGNED_BYTE,
                pixels.as_ptr() as *const c_void,
            );
            self.gui_w = width;
            self.gui_h = height;
        } else {
            // same size, just sub-upload
            (gl.bind_texture)(GL_TEXTURE_2D, self.gui_tex);
            (gl.tex_sub_image_2d)(
                GL_TEXTURE_2D, 0, 0, 0,
                width as GLsizei, height as GLsizei,
                GL_RGBA, GL_UNSIGNED_BYTE,
                pixels.as_ptr() as *const c_void,
            );
        }

        // fullscreen quad for the GUI layer
        let verts = quad_vertices(0.0, 0.0, vp_w, vp_h, vp_w, vp_h);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, self.quad_vbo);
        (gl.buffer_data)(
            GL_ARRAY_BUFFER,
            std::mem::size_of_val(&verts) as GLsizeiptr,
            verts.as_ptr() as *const c_void,
            GL_DYNAMIC_DRAW,
        );

        (gl.use_program)(self.passthrough_prog);
        (gl.uniform_1i)(self.pt_locs.tex, 0);
        (gl.uniform_1i)(self.pt_locs.circle_clip, 0);

        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, self.gui_tex);
        (gl.bind_vertex_array)(self.quad_vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 6);
        (gl.bind_vertex_array)(0);
    }

    unsafe fn draw_clear_rect(
        &self, gl: &GlFns,
        x: f32, y: f32, w: f32, h: f32,
        vp_h: f32,
    ) {
        // punch a transparent hole in the backbuffer
        let gl_y = (vp_h - (y + h)) as GLint;
        (gl.enable)(GL_SCISSOR_TEST);
        (gl.scissor)(x as GLint, gl_y, w as GLsizei, h as GLsizei);

        // need alpha writes on and blending off for a clean clear
        (gl.color_mask)(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
        (gl.disable)(GL_BLEND);
        (gl.clear_color)(0.0, 0.0, 0.0, 0.0);
        (gl.clear)(GL_COLOR_BUFFER_BIT);

        // put overlay state back
        (gl.enable)(GL_BLEND);
        (gl.color_mask)(GL_TRUE, GL_TRUE, GL_TRUE, GL_FALSE);
        (gl.disable)(GL_SCISSOR_TEST);
    }

    // ---- Zero-copy texture ref draws (mirrors) ----

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_tex_ref(
        &mut self, gl: &GlFns,
        texture: GLuint, tw: u32, th: u32,
        x: f32, y: f32, w: f32, h: f32,
        flip_v: bool, circle_clip: bool, nearest: bool,
        target_colors: &[[f32; 4]],
        output_color: &[f32; 4],
        sensitivity: f32,
        color_passthrough: bool,
        border_color: &[f32; 4],
        border_w: i32,
        gamma_mode: i32,
        vp_w: f32, vp_h: f32,
        uv_rect: Option<&[f32; 4]>,
    ) {
        let filt = if nearest { GL_NEAREST } else { GL_LINEAR };
        let use_filter = !target_colors.is_empty();
        let has_pad = use_filter && border_w > 0;
        let has_uv = uv_rect.is_some();

        (gl.bind_texture)(GL_TEXTURE_2D, texture);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filt);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filt);

        // CLAMP_TO_BORDER gives us transparent padding for filter border searches,
        // but only when we're not using uv_rect (shader-side clamping handles that)
        if has_pad && !has_uv {
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_BORDER);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_BORDER);
        } else {
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        }

        let cw = tw as f32;
        let ch = th as f32;
        let verts = quad_vertices_ext(x, y, w, h, cw, ch, vp_w, vp_h, flip_v, has_pad, uv_rect);

        (gl.bind_buffer)(GL_ARRAY_BUFFER, self.quad_vbo);
        (gl.buffer_data)(
            GL_ARRAY_BUFFER,
            std::mem::size_of_val(&verts) as GLsizeiptr,
            verts.as_ptr() as *const c_void,
            GL_DYNAMIC_DRAW,
        );

        if use_filter {
            self.bind_filter_uniforms(gl, target_colors, output_color, sensitivity,
                color_passthrough, border_color, border_w, gamma_mode, w, h, uv_rect);
        } else {
            (gl.use_program)(self.passthrough_prog);
            (gl.uniform_1i)(self.pt_locs.tex, 0);
            (gl.uniform_1i)(self.pt_locs.circle_clip, circle_clip as GLint);
        }

        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, texture);
        (gl.bind_vertex_array)(self.quad_vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 6);
        (gl.bind_vertex_array)(0);

        // don't leave CLAMP_TO_BORDER on somebody else's texture
        if has_pad && !has_uv {
            (gl.bind_texture)(GL_TEXTURE_2D, texture);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        }
    }

    // ---- Custom shader draws ----

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_custom_textured(
        &mut self, gl: &GlFns,
        pixels: &[u8], tw: u32, th: u32,
        x: f32, y: f32, w: f32, h: f32,
        nearest: bool,
        shader_name: &str, time: f32,
        vp_w: f32, vp_h: f32,
    ) {
        // grab uniform locs before mutable borrow via acquire_tex
        let (prog, tex_loc, time_loc, res_loc) = match self.custom_progs.get(shader_name) {
            Some(p) => (p.id, p.tex_loc, p.time_loc, p.res_loc),
            None => {
                // fall back to passthrough if shader is missing
                self.draw_textured(
                    gl, pixels, tw, th, x, y, w, h,
                    false, nearest, &[], &[0.0; 4], 0.0, false, &[0.0; 4], 0, 0,
                    vp_w, vp_h,
                );
                return;
            }
        };

        let tex = self.acquire_tex(gl);
        let filt = if nearest { GL_NEAREST } else { GL_LINEAR };
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filt);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filt);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
        (gl.tex_image_2d)(
            GL_TEXTURE_2D, 0, GL_RGBA8 as GLint,
            tw as GLsizei, th as GLsizei, 0,
            GL_RGBA, GL_UNSIGNED_BYTE,
            pixels.as_ptr() as *const c_void,
        );

        let verts = quad_vertices(x, y, w, h, vp_w, vp_h);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, self.quad_vbo);
        (gl.buffer_data)(
            GL_ARRAY_BUFFER,
            std::mem::size_of_val(&verts) as GLsizeiptr,
            verts.as_ptr() as *const c_void,
            GL_DYNAMIC_DRAW,
        );

        (gl.use_program)(prog);
        if tex_loc >= 0 { (gl.uniform_1i)(tex_loc, 0); }
        if time_loc >= 0 { (gl.uniform_1f)(time_loc, time); }
        if res_loc >= 0 { (gl.uniform_2f)(res_loc, w, h); }

        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, tex);
        (gl.bind_vertex_array)(self.quad_vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 6);
        (gl.bind_vertex_array)(0);
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_custom_tex_ref(
        &mut self, gl: &GlFns,
        texture: GLuint, tw: u32, th: u32,
        x: f32, y: f32, w: f32, h: f32,
        flip_v: bool, nearest: bool,
        shader_name: &str, time: f32,
        vp_w: f32, vp_h: f32,
        uv_rect: Option<&[f32; 4]>,
    ) {
        let (prog, tex_loc, time_loc, res_loc) = match self.custom_progs.get(shader_name) {
            Some(p) => (p.id, p.tex_loc, p.time_loc, p.res_loc),
            None => {
                self.draw_tex_ref(
                    gl, texture, tw, th, x, y, w, h,
                    flip_v, false, nearest, &[], &[0.0; 4], 0.0, false, &[0.0; 4], 0, 0,
                    vp_w, vp_h, uv_rect,
                );
                return;
            }
        };

        let filt = if nearest { GL_NEAREST } else { GL_LINEAR };
        (gl.bind_texture)(GL_TEXTURE_2D, texture);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, filt);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, filt);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        (gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);

        let cw = tw as f32;
        let ch = th as f32;
        let verts = quad_vertices_ext(x, y, w, h, cw, ch, vp_w, vp_h, flip_v, false, uv_rect);
        (gl.bind_buffer)(GL_ARRAY_BUFFER, self.quad_vbo);
        (gl.buffer_data)(
            GL_ARRAY_BUFFER,
            std::mem::size_of_val(&verts) as GLsizeiptr,
            verts.as_ptr() as *const c_void,
            GL_DYNAMIC_DRAW,
        );

        (gl.use_program)(prog);
        if tex_loc >= 0 { (gl.uniform_1i)(tex_loc, 0); }
        if time_loc >= 0 { (gl.uniform_1f)(time_loc, time); }
        if res_loc >= 0 { (gl.uniform_2f)(res_loc, w, h); }

        (gl.active_texture)(GL_TEXTURE0);
        (gl.bind_texture)(GL_TEXTURE_2D, texture);
        (gl.bind_vertex_array)(self.quad_vao);
        (gl.draw_arrays)(GL_TRIANGLES, 0, 6);
        (gl.bind_vertex_array)(0);
    }

    // ---- Shared filter uniform setup ----

    #[allow(clippy::too_many_arguments)]
    unsafe fn bind_filter_uniforms(
        &self, gl: &GlFns,
        target_colors: &[[f32; 4]],
        output_color: &[f32; 4],
        sensitivity: f32,
        color_passthrough: bool,
        border_color: &[f32; 4],
        border_w: i32,
        gamma_mode: i32,
        w: f32, h: f32,
        uv_rect: Option<&[f32; 4]>,
    ) {
        (gl.use_program)(self.filter_prog);
        (gl.uniform_1i)(self.filt_locs.tex, 0);

        for i in 0..4 {
            if i < target_colors.len() {
                let c = &target_colors[i];
                (gl.uniform_4f)(self.filt_locs.target_colors[i], c[0], c[1], c[2], c[3]);
            } else {
                (gl.uniform_4f)(self.filt_locs.target_colors[i], 0.0, 0.0, 0.0, 0.0);
            }
        }
        (gl.uniform_4f)(self.filt_locs.output_color,
            output_color[0], output_color[1], output_color[2], output_color[3]);
        (gl.uniform_4f)(self.filt_locs.border_color,
            border_color[0], border_color[1], border_color[2], border_color[3]);
        (gl.uniform_1f)(self.filt_locs.sensitivity, sensitivity);
        (gl.uniform_1i)(self.filt_locs.color_count, target_colors.len().min(4) as GLint);
        (gl.uniform_1i)(self.filt_locs.color_passthrough, color_passthrough as GLint);
        (gl.uniform_1i)(self.filt_locs.border_w, border_w);

        if w > 0.0 && h > 0.0 {
            (gl.uniform_2f)(self.filt_locs.screen_px, 1.0 / w, 1.0 / h);
        } else {
            (gl.uniform_2f)(self.filt_locs.screen_px, 0.0, 0.0);
        }
        (gl.uniform_1i)(self.filt_locs.gamma_mode, gamma_mode);

        // UV bounds for shader-side clamping (game-texture-direct path)
        if let Some(uv) = uv_rect {
            (gl.uniform_4f)(self.filt_locs.uv_bounds, uv[0], uv[1], uv[2], uv[3]);
        } else {
            (gl.uniform_4f)(self.filt_locs.uv_bounds, 0.0, 0.0, 0.0, 0.0);
        }
    }

    // ---- Texture pool ----

    unsafe fn acquire_tex(&mut self, gl: &GlFns) -> GLuint {
        if self.pool_idx < self.tex_pool.len() {
            let t = self.tex_pool[self.pool_idx];
            self.pool_idx += 1;
            return t;
        }
        // need a new one
        let mut t: GLuint = 0;
        (gl.gen_textures)(1, &mut t);
        self.tex_pool.push(t);
        self.pool_idx += 1;
        t
    }

    pub unsafe fn destroy(&mut self, gl: &GlFns) {
        for prog in [
            self.solid_prog, self.gradient_prog, self.border_prog,
            self.passthrough_prog, self.filter_prog,
        ] {
            if prog != 0 { (gl.delete_program)(prog); }
        }
        self.solid_prog = 0;
        self.gradient_prog = 0;
        self.border_prog = 0;
        self.passthrough_prog = 0;
        self.filter_prog = 0;

        if self.quad_vbo != 0 {
            (gl.delete_buffers)(1, &self.quad_vbo);
            self.quad_vbo = 0;
        }
        if self.quad_vao != 0 {
            (gl.delete_vertex_arrays)(1, &self.quad_vao);
            self.quad_vao = 0;
        }
        if self.gui_tex != 0 {
            (gl.delete_textures)(1, &self.gui_tex);
            self.gui_tex = 0;
        }
        if !self.tex_pool.is_empty() {
            (gl.delete_textures)(self.tex_pool.len() as GLsizei, self.tex_pool.as_ptr());
            self.tex_pool.clear();
        }
    }
}

// ---- Helpers ----

/// Turn a pixel-space rect into NDC quad vertices: [x, y, u, v] * 6
fn quad_vertices(x: f32, y: f32, w: f32, h: f32, vp_w: f32, vp_h: f32) -> [f32; 24] {
    let x0 = (x / vp_w) * 2.0 - 1.0;
    let x1 = ((x + w) / vp_w) * 2.0 - 1.0;
    // y=0 is screen top -> NDC +1, y+h is lower
    let y0 = 1.0 - (y / vp_h) * 2.0;
    let y1 = 1.0 - ((y + h) / vp_h) * 2.0;

    #[rustfmt::skip]
    let v = [
        x0, y0,  0.0, 0.0,
        x1, y0,  1.0, 0.0,
        x1, y1,  1.0, 1.0,
        x0, y0,  0.0, 0.0,
        x1, y1,  1.0, 1.0,
        x0, y1,  0.0, 1.0,
    ];
    v
}

/// Extended version with V-flip, border-padding UV extension, and UV subregion remap.
fn quad_vertices_ext(
    x: f32, y: f32, w: f32, h: f32,
    cw: f32, ch: f32,
    vp_w: f32, vp_h: f32,
    flip_v: bool, border_pad: bool,
    uv_rect: Option<&[f32; 4]>,
) -> [f32; 24] {
    let x0 = (x / vp_w) * 2.0 - 1.0;
    let x1 = ((x + w) / vp_w) * 2.0 - 1.0;
    let y0 = 1.0 - (y / vp_h) * 2.0;
    let y1 = 1.0 - ((y + h) / vp_h) * 2.0;

    // base UVs, optionally extended beyond [0,1] for border padding
    let (mut u0, mut u1, mut v0, mut v1) = if border_pad && cw > 0.0 && ch > 0.0 {
        let px = (w - cw) / 2.0;
        let py = (h - ch) / 2.0;
        (-px / cw, 1.0 + px / cw, -py / ch, 1.0 + py / ch)
    } else {
        (0.0, 1.0, 0.0, 1.0)
    };

    // remap into the game-texture subregion
    if let Some(&[su0, sv0, su1, sv1]) = uv_rect {
        let ru = su1 - su0;
        let rv = sv1 - sv0;
        u0 = su0 + u0 * ru;
        u1 = su0 + u1 * ru;
        v0 = sv0 + v0 * rv;
        v1 = sv0 + v1 * rv;
    }

    let (tv0, tv1) = if flip_v { (v1, v0) } else { (v0, v1) };

    #[rustfmt::skip]
    let v = [
        x0, y0,  u0, tv0,
        x1, y0,  u1, tv0,
        x1, y1,  u1, tv1,
        x0, y0,  u0, tv0,
        x1, y1,  u1, tv1,
        x0, y1,  u0, tv1,
    ];
    v
}

// ---- Scene types ----

/// One drawable overlay element
#[derive(Clone, Debug)]
pub enum SceneElement {
    SolidRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 4],
    },
    Gradient {
        color1: [f32; 4],
        color2: [f32; 4],
        angle: f32,
        time: f32,
        animation_type: i32,
        scissor: Option<[f32; 4]>,
    },
    Border {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        border_width: f32,
        radius: f32,
        color: [f32; 4],
    },
    Textured {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        tex_width: u32,
        tex_height: u32,
        pixels: Vec<u8>,
        circle_clip: bool,
        nearest_filter: bool,
        filter_target_colors: Vec<[f32; 4]>,
        filter_output_color: [f32; 4],
        filter_sensitivity: f32,
        filter_color_passthrough: bool,
        filter_border_color: [f32; 4],
        filter_border_width: i32,
        filter_gamma_mode: i32,
        custom_shader: Option<String>,
    },
    GuiOverlay {
        pixels: Vec<u8>,
        width: u32,
        height: u32,
    },
    ClearRect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    },
    // Zero-copy mirror - references an existing GL texture directly
    TextureRef {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        gl_texture: u32,
        tex_width: u32,
        tex_height: u32,
        flip_v: bool,
        circle_clip: bool,
        nearest_filter: bool,
        filter_target_colors: Vec<[f32; 4]>,
        filter_output_color: [f32; 4],
        filter_sensitivity: f32,
        filter_color_passthrough: bool,
        filter_border_color: [f32; 4],
        filter_border_width: i32,
        filter_gamma_mode: i32,
        /// UV subregion [u_min, v_min, u_max, v_max], None = full texture
        uv_rect: Option<[f32; 4]>,
        custom_shader: Option<String>,
    },
}

/// Full overlay frame ready to render
#[derive(Clone, Debug)]
pub struct SceneDescription {
    pub clear_color: [f32; 4],
    /// Elements drawn back-to-front
    pub elements: Vec<SceneElement>,
    /// Seconds since overlay init, passed to custom shader uTime
    pub time: f32,
}

impl SceneDescription {
    /// Quick structural fingerprint for skipping unchanged frames.
    /// Not cryptographic, just needs to detect changes.
    pub fn fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        for c in &self.clear_color {
            c.to_bits().hash(&mut h);
        }
        h.write_usize(self.elements.len());

        for e in &self.elements {
            match e {
                SceneElement::SolidRect { x, y, w, h: eh, color } => {
                    0u8.hash(&mut h);
                    x.to_bits().hash(&mut h);
                    y.to_bits().hash(&mut h);
                    w.to_bits().hash(&mut h);
                    eh.to_bits().hash(&mut h);
                    for c in color { c.to_bits().hash(&mut h); }
                }
                SceneElement::Gradient { color1, color2, angle, time, animation_type, scissor } => {
                    1u8.hash(&mut h);
                    for c in color1 { c.to_bits().hash(&mut h); }
                    for c in color2 { c.to_bits().hash(&mut h); }
                    angle.to_bits().hash(&mut h);
                    time.to_bits().hash(&mut h);
                    animation_type.hash(&mut h);
                    if let Some(s) = scissor {
                        for v in s { v.to_bits().hash(&mut h); }
                    }
                }
                SceneElement::Border { x, y, w, h: eh, border_width, radius, color } => {
                    2u8.hash(&mut h);
                    x.to_bits().hash(&mut h);
                    y.to_bits().hash(&mut h);
                    w.to_bits().hash(&mut h);
                    eh.to_bits().hash(&mut h);
                    border_width.to_bits().hash(&mut h);
                    radius.to_bits().hash(&mut h);
                    for c in color { c.to_bits().hash(&mut h); }
                }
                SceneElement::Textured { x, y, w, h: eh, tex_width, tex_height, pixels, .. } => {
                    3u8.hash(&mut h);
                    x.to_bits().hash(&mut h);
                    y.to_bits().hash(&mut h);
                    w.to_bits().hash(&mut h);
                    eh.to_bits().hash(&mut h);
                    tex_width.hash(&mut h);
                    tex_height.hash(&mut h);
                    hash_pixel_sample(pixels, &mut h);
                }
                SceneElement::GuiOverlay { pixels, width, height } => {
                    4u8.hash(&mut h);
                    width.hash(&mut h);
                    height.hash(&mut h);
                    hash_pixel_sample(pixels, &mut h);
                }
                SceneElement::ClearRect { x, y, w, h: eh } => {
                    5u8.hash(&mut h);
                    x.to_bits().hash(&mut h);
                    y.to_bits().hash(&mut h);
                    w.to_bits().hash(&mut h);
                    eh.to_bits().hash(&mut h);
                }
                SceneElement::TextureRef { x, y, w, h: eh, gl_texture, tex_width, tex_height, .. } => {
                    6u8.hash(&mut h);
                    x.to_bits().hash(&mut h);
                    y.to_bits().hash(&mut h);
                    w.to_bits().hash(&mut h);
                    eh.to_bits().hash(&mut h);
                    gl_texture.hash(&mut h);
                    tex_width.hash(&mut h);
                    tex_height.hash(&mut h);
                    // mirror content changes every frame, always treat as dirty
                    h.write_u64(0xDEAD_BEEF_CAFE_BABE);
                }
            }
        }
        h.finish()
    }
}

/// Sample a few bytes from a pixel buffer for fingerprinting.
/// Not meant to be thorough - just catch obvious changes fast.
fn hash_pixel_sample(pixels: &[u8], h: &mut DefaultHasher) {
    h.write_usize(pixels.len());
    if pixels.is_empty() { return; }
    let step = 4096;
    let mut off = 0;
    while off + 8 <= pixels.len() {
        h.write(&pixels[off..off + 8]);
        off += step;
    }
    // always include the tail
    if pixels.len() >= 8 {
        h.write(&pixels[pixels.len() - 8..]);
    }
}
