//! FBO-backed offscreen color target for the switchgl backend.
//!
//! The color attachment is a plain GL texture, so `imgui_texture_id` follows
//! the backend's existing convention (TexID == GL texture name) and the imgui
//! renderer can sample it with no extra registration step.

use crate::rendering::RenderTarget;

use super::gles::*;

pub struct SwitchGLRenderTarget {
    fbo: u32,
    color: u32,
    depth: u32,
    extent: (u32, u32),
}

impl SwitchGLRenderTarget {
    pub fn new(width: u32, height: u32) -> Self {
        let mut target = Self {
            fbo: 0,
            color: 0,
            depth: 0,
            extent: (0, 0),
        };
        target.recreate(width.max(1), height.max(1));
        target
    }

    pub fn framebuffer(&self) -> u32 {
        self.fbo
    }

    fn recreate(&mut self, width: u32, height: u32) {
        unsafe {
            self.destroy();

            glGenTextures(1, &mut self.color);
            glBindTexture(GL_TEXTURE_2D, self.color);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as i32,
                width as i32,
                height as i32,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                std::ptr::null(),
            );
            glBindTexture(GL_TEXTURE_2D, 0);

            glGenRenderbuffers(1, &mut self.depth);
            glBindRenderbuffer(GL_RENDERBUFFER, self.depth);
            glRenderbufferStorage(
                GL_RENDERBUFFER,
                GL_DEPTH_COMPONENT16,
                width as i32,
                height as i32,
            );
            glBindRenderbuffer(GL_RENDERBUFFER, 0);

            glGenFramebuffers(1, &mut self.fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, self.fbo);
            glFramebufferTexture2D(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_TEXTURE_2D,
                self.color,
                0,
            );
            glFramebufferRenderbuffer(
                GL_FRAMEBUFFER,
                GL_DEPTH_ATTACHMENT,
                GL_RENDERBUFFER,
                self.depth,
            );
            let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
            if status != GL_FRAMEBUFFER_COMPLETE {
                log::error!("render target framebuffer incomplete: 0x{:x}", status);
            }
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
        }
        self.extent = (width, height);
    }

    unsafe fn destroy(&mut self) {
        unsafe {
            if self.fbo != 0 {
                glDeleteFramebuffers(1, &self.fbo);
                self.fbo = 0;
            }
            if self.color != 0 {
                glDeleteTextures(1, &self.color);
                self.color = 0;
            }
            if self.depth != 0 {
                glDeleteRenderbuffers(1, &self.depth);
                self.depth = 0;
            }
        }
    }
}

impl RenderTarget for SwitchGLRenderTarget {
    fn extent(&self) -> (u32, u32) {
        self.extent
    }

    fn resize(&mut self, width: u32, height: u32) {
        if self.extent == (width, height) {
            return;
        }
        self.recreate(width.max(1), height.max(1));
    }

    fn imgui_texture_id(&self) -> u64 {
        self.color as u64
    }

    fn as_switchgl_mut(&mut self) -> Option<&mut SwitchGLRenderTarget> {
        Some(self)
    }
}

impl Drop for SwitchGLRenderTarget {
    fn drop(&mut self) {
        unsafe { self.destroy() };
    }
}
