use std::cell::RefCell;
use std::rc::Rc;

use imgui::TextureId;

use super::{
    MaterialDef, RenderObjectHandle, RenderTarget, RenderingComponent, Texture, VertexBuffer,
    VideoPlayer, texture::TextureDef,
};

/// imgui textures handed back for release. The backend frees each one only after
/// the GPU frames that might still sample it have finished: a texture can be
/// dropped during update while this frame's draw list already references it.
pub type ImguiTextureReleaseQueue = Rc<RefCell<Vec<(TextureId, Box<dyn Texture>)>>>;

pub trait ComponentFactory {
    fn create_texture(&self, texture_def: &TextureDef) -> Box<dyn Texture>;

    fn create_imgui_texture(
        &self,
        buffer: &[u8],
        row_length: u32,
        width: u32,
        height: u32,
        texture_id: Option<TextureId>,
    ) -> (Box<dyn Texture>, TextureId);

    fn remove_imgui_texture(&self, texture_id: Option<TextureId>);

    /// Where a short-lived imgui texture (a [`super::Sprite`]) goes when its owner
    /// drops it. `None` means the backend frees the texture itself when the
    /// `Box<dyn Texture>` drops (the GL backends: the imgui id is the GL name).
    fn imgui_texture_release_queue(&self) -> Option<ImguiTextureReleaseQueue> {
        None
    }

    fn create_render_object(
        &self,
        vertices: VertexBuffer,
        indices: Vec<u32>,
        material_def: &MaterialDef,
        host_dynamic: bool,
    ) -> RenderObjectHandle;

    fn create_rendering_component(&self, objects: Vec<RenderObjectHandle>) -> RenderingComponent;

    fn create_video_player(&self) -> Box<VideoPlayer>;

    /// Allocate a fresh offscreen color target sized `width` x `height`.
    /// The returned target is owned by the caller; the engine renders into
    /// it via [`RenderingEngine::render_scene_to_target`](super::RenderingEngine::render_scene_to_target).
    ///
    /// Backends that don't yet support offscreen rendering (vitagl) panic
    /// with a clear message — see the per-backend impl for status.
    fn create_render_target(&self, width: u32, height: u32) -> Box<dyn RenderTarget>;
}
