use image::{ImageFormat, RgbaImage};

use super::{ComponentFactory, Texture, factory::ImguiTextureReleaseQueue};

pub struct Sprite {
    image: image::RgbaImage,
    texture: Option<Box<dyn Texture>>,
    texture_id: imgui::TextureId,
    /// Vulkan registers every imgui texture in a descriptor pool; without
    /// handing it back the pool runs dry after a few thousand sprites (an
    /// SCE dialog choice builds its sprites on every pass through the menu).
    release: Option<ImguiTextureReleaseQueue>,
}

impl Sprite {
    pub fn load_from_image(image: RgbaImage, factory: &dyn ComponentFactory) -> Self {
        let (texture, texture_id) =
            factory.create_imgui_texture(image.as_raw(), 0, image.width(), image.height(), None);

        Self {
            image,
            texture: Some(texture),
            texture_id,
            release: factory.imgui_texture_release_queue(),
        }
    }

    pub fn load_from_buffer(
        buffer: &[u8],
        format: ImageFormat,
        factory: &dyn ComponentFactory,
    ) -> Self {
        let image = image::load_from_memory_with_format(buffer, format)
            .unwrap()
            .to_rgba8();
        Self::load_from_image(image, factory)
    }

    pub fn imgui_texture_id(&self) -> imgui::TextureId {
        self.texture_id
    }

    pub fn width(&self) -> u32 {
        self.image.width()
    }

    pub fn height(&self) -> u32 {
        self.image.height()
    }
}

impl Drop for Sprite {
    fn drop(&mut self) {
        if let (Some(queue), Some(texture)) = (self.release.take(), self.texture.take()) {
            queue.borrow_mut().push((self.texture_id, texture));
        }
    }
}
