//! GLES2/EGL rendering backend for Nintendo Switch homebrew.
//!
//! Structurally a sibling of `vitagl`: same module split, same handle plumbing
//! into `RenderObjectHandle` / `RenderingComponent`. The difference is that
//! devkitPro ships Mesa, so this talks to real GLES2 through EGL rather than
//! going through vitaGL's GL-on-SceGxm translation -- which is also why the
//! shaders here are GLSL ES rather than the Vita backend's Cg.
mod factory;
mod gles;
mod imgui_renderer;
mod material;
mod render_object;
mod render_target;
mod shader;
mod switchgl_engine;
mod texture;

pub(super) use render_object::SwitchGLRenderObject;

pub use render_target::SwitchGLRenderTarget;
pub use switchgl_engine::SwitchGLRenderingEngine;
