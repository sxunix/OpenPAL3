//! The loading screen drawn straight with GL in the middle of a scene build
//! (see [`crate::loading`]): black, the bar filled to the reported fraction,
//! the ornament riding the bar's head. Same art, positions and native-size
//! drawing as the imgui version the script VM shows for the first frame
//! (`SceState::render_loading`), so the bar continues from where that left.

use std::ffi::CString;

use crate::loading::LoadingArt;

use super::gles::*;
use super::texture::SwitchGLTexture;

const VERT: &str = "attribute vec2 position;
attribute vec2 texcoord;
varying vec2 v_uv;
void main() {
    v_uv = texcoord;
    gl_Position = vec4(position, 0.0, 1.0);
}
";

const FRAG: &str = "precision mediump float;
varying vec2 v_uv;
uniform sampler2D tex;
void main() {
    gl_FragColor = texture2D(tex, v_uv);
}
";

/// `UIProgressBar` geometry from `UILoadingFrm::Create`: control at
/// (206,520) 364×28; fill area inset left 2+4, top 2+10−1, bottom 2+6, and
/// as long as the fill art's content (350 px); ornament 88×64 centred on the
/// head, offset (0,−20).
const BAR_AT: (f32, f32) = (206., 520.);
const FILL_LEFT: f32 = 212.;
const FILL_TOP: f32 = 531.;
const FILL_BOTTOM: f32 = 540.;
const FILL_LEN: f32 = 350.;
const ORNAMENT_TOP: f32 = 483.;
const ORNAMENT_HALF_W: f32 = 44.;
const ORNAMENT_SWAP_SEC: f32 = 0.3;

pub struct LoadingPainter {
    program: u32,
    sampler: i32,
    bar: SwitchGLTexture,
    fill: SwitchGLTexture,
    ornament: [SwitchGLTexture; 2],
    sizes: [(f32, f32); 4],
}

impl LoadingPainter {
    pub fn new(art: &LoadingArt) -> Option<Self> {
        let upload = |img: &image::RgbaImage| SwitchGLTexture::new(img.width(), img.height(), img);
        let size = |img: &image::RgbaImage| (img.width() as f32, img.height() as f32);
        let program = unsafe { compile()? };
        let name = CString::new("tex").ok()?;
        Some(Self {
            program,
            sampler: unsafe { glGetUniformLocation(program, name.as_ptr()) },
            bar: upload(&art.bar),
            fill: upload(&art.fill),
            ornament: [upload(&art.ornament[0]), upload(&art.ornament[1])],
            sizes: [
                size(&art.bar),
                size(&art.fill),
                size(&art.ornament[0]),
                size(&art.ornament[1]),
            ],
        })
    }

    /// Clear, draw, and leave presenting to the caller.
    pub fn draw(&self, extent: (u32, u32), progress: f32, elapsed: f32) {
        let (w, h) = (extent.0 as f32, extent.1 as f32);
        // The 800×600 UI space, letterboxed like the imgui overlays.
        let scale = (w / 800.).min(h / 600.);
        let (ox, oy) = ((w - 800. * scale) / 2., (h - 600. * scale) / 2.);
        let ndc = |x: f32, y: f32| {
            (
                (ox + x * scale) / w * 2. - 1.,
                1. - (oy + y * scale) / h * 2.,
            )
        };
        let fill_w = FILL_LEN * progress.clamp(0., 1.);
        let k = (elapsed / ORNAMENT_SWAP_SEC) as usize % 2;
        // (texture, top-left, size in UI px, uv extent)
        let quads = [
            (self.bar.texture_id(), BAR_AT, self.sizes[0], (1., 1.)),
            (
                self.fill.texture_id(),
                (FILL_LEFT, FILL_TOP),
                (fill_w, FILL_BOTTOM - FILL_TOP),
                (fill_w / self.sizes[1].0, 1.),
            ),
            (
                self.ornament[k].texture_id(),
                (FILL_LEFT + fill_w - ORNAMENT_HALF_W, ORNAMENT_TOP),
                self.sizes[2 + k],
                (1., 1.),
            ),
        ];
        unsafe {
            glViewport(0, 0, extent.0 as i32, extent.1 as i32);
            glClearColor(0., 0., 0., 1.);
            glClear(GL_COLOR_BUFFER_BIT);
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_CULL_FACE);
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            glUseProgram(self.program);
            glUniform1i(self.sampler, 0);
            glActiveTexture(GL_TEXTURE0);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glEnableVertexAttribArray(0);
            glEnableVertexAttribArray(1);
            for (tex, (x, y), (qw, qh), (u, v)) in quads {
                if qw <= 0. || qh <= 0. {
                    continue;
                }
                let (x0, y0) = ndc(x, y);
                let (x1, y1) = ndc(x + qw, y + qh);
                let pos: [f32; 8] = [x0, y0, x1, y0, x1, y1, x0, y1];
                let uv: [f32; 8] = [0., 0., u, 0., u, v, 0., v];
                let idx: [u16; 6] = [0, 1, 2, 0, 2, 3];
                glBindTexture(GL_TEXTURE_2D, tex);
                glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, pos.as_ptr() as *const _);
                glVertexAttribPointer(1, 2, GL_FLOAT, GL_FALSE, 0, uv.as_ptr() as *const _);
                glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, idx.as_ptr() as *const _);
            }
            glEnable(GL_DEPTH_TEST);
        }
    }
}

unsafe fn compile() -> Option<u32> {
    unsafe {
        let shader = |kind, src: &str| {
            let s = glCreateShader(kind);
            let c = CString::new(src).ok()?;
            let len = src.len() as i32;
            glShaderSource(s, 1, &c.as_ptr(), &len);
            glCompileShader(s);
            let mut ok = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            (ok == GL_TRUE as i32).then_some(s)
        };
        let vs = shader(GL_VERTEX_SHADER, VERT)?;
        let fs = shader(GL_FRAGMENT_SHADER, FRAG)?;
        let program = glCreateProgram();
        glAttachShader(program, vs);
        glAttachShader(program, fs);
        let position = CString::new("position").ok()?;
        let texcoord = CString::new("texcoord").ok()?;
        glBindAttribLocation(program, 0, position.as_ptr());
        glBindAttribLocation(program, 1, texcoord.as_ptr());
        glLinkProgram(program);
        Some(program)
    }
}
