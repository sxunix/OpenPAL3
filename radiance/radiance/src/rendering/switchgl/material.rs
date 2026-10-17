use std::rc::Rc;
use std::sync::Arc;

use image::RgbaImage;

use crate::rendering::{BlendMode, CullMode, DepthMode, MaterialDef, MaterialParams, TextureDef};

use super::{shader::SwitchGLShader, texture::SwitchGLTexture};

pub struct SwitchGLMaterial {
    name: String,
    shader: Rc<SwitchGLShader>,
    textures: Vec<Rc<SwitchGLTexture>>,
    params: std::cell::Cell<MaterialParams>,
    blend: BlendMode,
    depth: DepthMode,
    cull: CullMode,
}

impl std::fmt::Debug for SwitchGLMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("SwitchGLMaterial({})", self.name))
    }
}

impl SwitchGLMaterial {
    /// `texture_for` resolves each of the material's textures to its GL
    /// object (shared by name, see `SwitchGLComponentFactory`).
    pub fn new(
        def: &MaterialDef,
        shader: Rc<SwitchGLShader>,
        texture_for: impl Fn(&Arc<TextureDef>) -> Rc<SwitchGLTexture>,
    ) -> Self {
        let textures = def.textures().iter().map(|t| texture_for(t)).collect();

        Self {
            name: def.debug_name().to_string(),
            shader,
            textures,
            params: std::cell::Cell::new(*def.params()),
            blend: def.blend(),
            depth: def.depth(),
            cull: def.cull(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn shader(&self) -> &SwitchGLShader {
        &self.shader
    }

    pub fn textures(&self) -> &[Rc<SwitchGLTexture>] {
        &self.textures
    }

    pub fn params(&self) -> MaterialParams {
        self.params.get()
    }

    /// The UV affine `uv = texcoord * scale + offset` (`set_uv_xform`).
    pub fn update_uv_xform(&self, scale: [f32; 2], offset: [f32; 2]) {
        let mut p = self.params.get();
        p.uv_scale = scale;
        p.uv_offset = offset;
        self.params.set(p);
    }

    /// Rewrite only the tint's alpha lane (135/136 actor fade).
    pub fn update_tint_alpha(&self, alpha: f32) {
        let mut p = self.params.get();
        p.tint[3] = alpha.clamp(0., 1.);
        self.params.set(p);
    }

    /// Rewrite only the tint's RGB lanes.
    pub fn update_tint_rgb(&self, rgb: [f32; 3]) {
        let mut p = self.params.get();
        p.tint[..3].copy_from_slice(&rgb);
        self.params.set(p);
    }

    pub fn blend(&self) -> BlendMode {
        self.blend
    }

    pub fn depth(&self) -> DepthMode {
        self.depth
    }

    pub fn cull(&self) -> CullMode {
        self.cull
    }
}

lazy_static::lazy_static! {
    pub(super) static ref TEXTURE_MISSING: RgbaImage =
        image::load_from_memory(radiance_assets::TEXTURE_MISSING_TEXTURE_FILE)
            .unwrap()
            .to_rgba8();
}
