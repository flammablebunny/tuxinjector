// GL interop layer - draws our overlay on top of Minecraft's backbuffer
// without the game knowing (hopefully)

pub mod compositor;
pub mod gl_bindings;
pub mod gl_renderer;
pub mod gl_state;

pub use compositor::GlCompositor;
pub use gl_bindings::GlFns;
pub use gl_renderer::{GlOverlayRenderer, SceneDescription, SceneElement, TrailVertex};
pub use gl_state::{restore_gl_state, save_gl_state, SavedGlState};
