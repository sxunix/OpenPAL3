use std::cell::{Cell, RefCell};
use std::sync::Arc;

use crate::rendering::{Texture, TextureDef};

use super::gles::*;

/// A GL texture. Scene textures are created lazily
/// ([`SwitchGLTexture::deferred`]): the image stays on the CPU side until
/// the first draw that binds it. A PAL3 street names well over a thousand
/// textures (mostly per-mesh lightmaps), and uploading them all during the
/// scene load was the biggest single cost of entering one, although only
/// the part in front of the camera is ever needed on the first frames.
pub struct SwitchGLTexture {
    texture_id: Cell<u32>,
    width: Cell<u32>,
    height: Cell<u32>,
    pending: RefCell<Option<Arc<TextureDef>>>,
}

impl Texture for SwitchGLTexture {
    fn width(&self) -> u32 {
        self.width.get()
    }

    fn height(&self) -> u32 {
        self.height.get()
    }
}

impl SwitchGLTexture {
    pub fn new(width: u32, height: u32, pixels: &[u8]) -> Self {
        Self {
            texture_id: Cell::new(upload(width, height, pixels)),
            width: Cell::new(width),
            height: Cell::new(height),
            pending: RefCell::new(None),
        }
    }

    /// A texture whose upload waits for the first [`Self::texture_id`].
    /// Size reads 0 until then.
    pub fn deferred(def: Arc<TextureDef>) -> Self {
        Self {
            texture_id: Cell::new(0),
            width: Cell::new(0),
            height: Cell::new(0),
            pending: RefCell::new(Some(def)),
        }
    }

    /// The GL name, uploading a deferred texture on first use. Must run
    /// with the GL context current (the draw does).
    pub fn texture_id(&self) -> u32 {
        if let Some(def) = self.pending.borrow_mut().take() {
            def.with_image(|img| {
                let image = img.unwrap_or(&super::material::TEXTURE_MISSING);
                self.width.set(image.width());
                self.height.set(image.height());
                self.texture_id
                    .set(upload(image.width(), image.height(), image));
            });
        }
        self.texture_id.get()
    }
}

fn upload(width: u32, height: u32, pixels: &[u8]) -> u32 {
    let _hitch = crate::perf::hitch::span("gl.texture_upload");
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

    texture_id
}

impl Drop for SwitchGLTexture {
    fn drop(&mut self) {
        let id = self.texture_id.get();
        if id != 0 {
            unsafe { glDeleteTextures(1, &id) };
        }
    }
}
