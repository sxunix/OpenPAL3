//! GLES2 imgui renderer for the Switch backend.
//!
//! Follows the same frame contract as the Vulkan renderer: consume the
//! `ImguiFrame` by value, bail if no frame was begun, call `igRender()` (which
//! ends the frame internally -- the marker's later `igEndFrame()` in `Drop` is
//! then a no-op) and walk the draw data. Texture ids follow the factory's
//! convention: `imgui::TextureId` *is* the raw GL texture name, so draw
//! commands bind it directly and the font atlas registers itself the same way.

use std::ffi::CString;

use imgui::{DrawCmd, DrawData, DrawVert, sys};

use crate::imgui::ImguiFrame;

use super::gles::*;

const VERT_SRC: &str = "#version 100\n\
uniform mat4 proj;\n\
attribute vec2 position;\n\
attribute vec2 uv;\n\
attribute vec4 color;\n\
varying vec2 v_uv;\n\
varying vec4 v_color;\n\
void main() {\n\
    v_uv = uv;\n\
    v_color = color;\n\
    gl_Position = proj * vec4(position, 0.0, 1.0);\n\
}\n";

const FRAG_SRC: &str = "#version 100\n\
precision mediump float;\n\
uniform sampler2D tex;\n\
varying vec2 v_uv;\n\
varying vec4 v_color;\n\
void main() {\n\
    gl_FragColor = v_color * texture2D(tex, v_uv);\n\
}\n";

pub struct SwitchGLImguiRenderer {
    program: u32,
    uniform_proj: i32,
    vbo: u32,
    ebo: u32,
    font_texture: u32,
}

impl SwitchGLImguiRenderer {
    /// Requires the imgui `Context` to already be created and current (it is:
    /// `UiManager` is constructed before the rendering engine).
    pub fn new() -> anyhow::Result<Self> {
        let (program, uniform_proj) = unsafe { build_program()? };

        let mut buffers = [0u32; 2];
        unsafe { glGenBuffers(2, buffers.as_mut_ptr()) };

        let mut renderer = Self {
            program,
            uniform_proj,
            vbo: buffers[0],
            ebo: buffers[1],
            font_texture: 0,
        };
        renderer.rebuild_font_atlas();
        Ok(renderer)
    }

    /// (Re)build the font atlas texture and hand its GL name to imgui as the
    /// atlas `TexID`. Also called when a game font is appended at runtime
    /// (`take_atlas_dirty` path via `update_imgui_font_atlas`).
    pub fn rebuild_font_atlas(&mut self) {
        unsafe {
            let fonts = (*sys::igGetIO()).Fonts;
            let mut pixels: *mut u8 = std::ptr::null_mut();
            let (mut w, mut h) = (0i32, 0i32);
            sys::ImFontAtlas_GetTexDataAsRGBA32(
                fonts,
                &mut pixels,
                &mut w,
                &mut h,
                std::ptr::null_mut(),
            );

            if self.font_texture != 0 {
                glDeleteTextures(1, &self.font_texture);
            }
            let mut tex = 0u32;
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as i32,
                w,
                h,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels as *const _,
            );
            glBindTexture(GL_TEXTURE_2D, 0);
            self.font_texture = tex;

            sys::ImFontAtlas_SetTexID(fonts, tex as usize as sys::ImTextureID);
        }
    }

    /// Render the UI recorded since `draw_ui`. Call after the 3D pass, before
    /// the buffer swap. `(fb_width, fb_height)` is the framebuffer extent the
    /// scissor rectangles are flipped against.
    pub fn render(&mut self, frame: ImguiFrame, fb_width: u32, fb_height: u32) {
        if !frame.frame_begun {
            return;
        }

        let draw_data: &DrawData = unsafe {
            sys::igRender();
            &*(sys::igGetDrawData() as *mut DrawData)
        };
        if draw_data.total_idx_count <= 0 {
            return;
        }

        let (fw, fh) = (fb_width as f32, fb_height as f32);
        let [dx, dy] = draw_data.display_pos;
        let [dw, dh] = draw_data.display_size;
        let [sx, sy] = draw_data.framebuffer_scale;
        if dw <= 0.0 || dh <= 0.0 {
            return;
        }

        // Orthographic projection, y-down to match imgui's coordinate space.
        let (l, r, t, b) = (dx, dx + dw, dy, dy + dh);
        #[rustfmt::skip]
        let proj: [f32; 16] = [
            2.0 / (r - l),      0.0,                0.0, 0.0,
            0.0,                2.0 / (t - b),      0.0, 0.0,
            0.0,                0.0,               -1.0, 0.0,
            (r + l) / (l - r), (t + b) / (b - t),   0.0, 1.0,
        ];

        unsafe {
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
            glEnable(GL_BLEND);
            glBlendFuncSeparate(
                GL_SRC_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_ONE,
                GL_ONE_MINUS_SRC_ALPHA,
            );
            glEnable(GL_SCISSOR_TEST);

            glUseProgram(self.program);
            glUniformMatrix4fv(self.uniform_proj, 1, GL_FALSE, proj.as_ptr());
            glActiveTexture(GL_TEXTURE0);

            glBindBuffer(GL_ARRAY_BUFFER, self.vbo);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, self.ebo);

            let stride = std::mem::size_of::<DrawVert>() as i32; // 20 bytes
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, stride, 0 as *const _);
            glEnableVertexAttribArray(1);
            glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, stride, 8 as *const _);
            glEnableVertexAttribArray(2);
            glVertexAttribPointer(2, 4, GL_UNSIGNED_BYTE, GL_TRUE, stride, 16 as *const _);

            for list in draw_data.draw_lists() {
                let vtx = list.vtx_buffer();
                let idx = list.idx_buffer();
                glBufferData(
                    GL_ARRAY_BUFFER,
                    std::mem::size_of_val(vtx) as GLsizeiptr,
                    vtx.as_ptr() as *const _,
                    GL_STREAM_DRAW,
                );
                glBufferData(
                    GL_ELEMENT_ARRAY_BUFFER,
                    std::mem::size_of_val(idx) as GLsizeiptr,
                    idx.as_ptr() as *const _,
                    GL_STREAM_DRAW,
                );

                for cmd in list.commands() {
                    match cmd {
                        DrawCmd::Elements { count, cmd_params } => {
                            let [cx1, cy1, cx2, cy2] = cmd_params.clip_rect;
                            // Clip rect -> framebuffer space, then flip Y (GL
                            // scissor origin is bottom-left).
                            let x = ((cx1 - dx) * sx).max(0.0);
                            let y = ((cy1 - dy) * sy).max(0.0);
                            let x2 = ((cx2 - dx) * sx).min(fw);
                            let y2 = ((cy2 - dy) * sy).min(fh);
                            if x2 <= x || y2 <= y {
                                continue;
                            }
                            glScissor(
                                x as i32,
                                (fh - y2) as i32,
                                (x2 - x) as i32,
                                (y2 - y) as i32,
                            );

                            glBindTexture(GL_TEXTURE_2D, cmd_params.texture_id.id() as u32);

                            // vtx_offset stays 0: the RendererHasVtxOffset
                            // backend flag is never set, and GLES2 has no
                            // base-vertex draw anyway.
                            glDrawElements(
                                GL_TRIANGLES,
                                count as i32,
                                GL_UNSIGNED_SHORT,
                                (cmd_params.idx_offset * std::mem::size_of::<u16>())
                                    as *const _,
                            );
                        }
                        // Would require re-applying our state; nothing in the
                        // codebase emits it, so treat as a no-op.
                        DrawCmd::ResetRenderState => {}
                        // C callbacks recorded by foreign code; none exist here.
                        DrawCmd::RawCallback { .. } => {}
                    }
                }
            }

            // Leave clean state for the next 3D pass.
            glDisable(GL_SCISSOR_TEST);
            glDisable(GL_BLEND);
            glDisableVertexAttribArray(2);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glBindTexture(GL_TEXTURE_2D, 0);
        }
    }
}

impl Drop for SwitchGLImguiRenderer {
    fn drop(&mut self) {
        unsafe {
            if self.font_texture != 0 {
                glDeleteTextures(1, &self.font_texture);
            }
            let buffers = [self.vbo, self.ebo];
            glDeleteBuffers(2, buffers.as_ptr());
            glDeleteProgram(self.program);
        }
    }
}

unsafe fn build_program() -> anyhow::Result<(u32, i32)> {
    unsafe {
        let vs = glCreateShader(GL_VERTEX_SHADER);
        let fs = glCreateShader(GL_FRAGMENT_SHADER);
        let program = glCreateProgram();

        for (shader, src, stage) in [(vs, VERT_SRC, "vertex"), (fs, FRAG_SRC, "fragment")] {
            let c = CString::new(src)?;
            let len = src.len() as i32;
            glShaderSource(shader, 1, &c.as_ptr(), &len);
            glCompileShader(shader);
            let mut status = 0;
            glGetShaderiv(shader, GL_COMPILE_STATUS, &mut status);
            if status != GL_TRUE as i32 {
                let mut log_len = 0;
                glGetShaderiv(shader, GL_INFO_LOG_LENGTH, &mut log_len);
                let mut log = vec![0u8; log_len.max(0) as usize];
                glGetShaderInfoLog(shader, log_len, std::ptr::null_mut(), log.as_mut_ptr() as *mut _);
                anyhow::bail!(
                    "imgui {} shader failed to compile: {}",
                    stage,
                    String::from_utf8_lossy(&log)
                );
            }
            glAttachShader(program, shader);
        }

        let position = CString::new("position")?;
        let uv = CString::new("uv")?;
        let color = CString::new("color")?;
        glBindAttribLocation(program, 0, position.as_ptr());
        glBindAttribLocation(program, 1, uv.as_ptr());
        glBindAttribLocation(program, 2, color.as_ptr());
        glLinkProgram(program);

        let mut status = 0;
        glGetProgramiv(program, GL_LINK_STATUS, &mut status);
        if status != GL_TRUE as i32 {
            anyhow::bail!("imgui shader program failed to link");
        }

        // Shaders can be flagged for deletion now; they die with the program.
        glDeleteShader(vs);
        glDeleteShader(fs);

        glUseProgram(program);
        let tex = CString::new("tex")?;
        glUniform1i(glGetUniformLocation(program, tex.as_ptr()), 0);
        let proj = CString::new("proj")?;
        let uniform_proj = glGetUniformLocation(program, proj.as_ptr());
        glUseProgram(0);

        Ok((program, uniform_proj))
    }
}
