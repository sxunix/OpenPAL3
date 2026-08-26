use crate::rendering::Texture;

use super::gles::*;

pub struct SwitchGLTexture {
    texture_id: u32,
    width: u32,
    height: u32,
}

impl Texture for SwitchGLTexture {
    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }
}

impl SwitchGLTexture {
    pub fn new(width: u32, height: u32, pixels: &[u8]) -> Self {
        let mut texture_id = 0;

        unsafe {
            glGenTextures(1, &mut texture_id);
            glBindTexture(GL_TEXTURE_2D, texture_id);

            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
            // GLES2 only guarantees REPEAT for NPOT textures when both wrap
            // modes are CLAMP_TO_EDGE; PAL3 art is POT, so plain REPEAT is fine
            // and matches how the Vulkan sampler is configured.
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT as i32);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as i32);

            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as i32,
                width as i32,
                height as i32,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_ptr() as *const _,
            );

            glBindTexture(GL_TEXTURE_2D, 0);
        }

        Self {
            texture_id,
            width,
            height,
        }
    }

    pub fn texture_id(&self) -> u32 {
        self.texture_id
    }
}

impl Drop for SwitchGLTexture {
    fn drop(&mut self) {
        unsafe { glDeleteTextures(1, &self.texture_id) };
    }
}
