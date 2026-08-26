use std::rc::Rc;

use image::RgbaImage;

use crate::rendering::{MaterialDef, MaterialParams};

use super::{shader::SwitchGLShader, texture::SwitchGLTexture};

pub struct SwitchGLMaterial {
    name: String,
    shader: Rc<SwitchGLShader>,
    textures: Vec<Rc<SwitchGLTexture>>,
    params: MaterialParams,
}

impl std::fmt::Debug for SwitchGLMaterial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("SwitchGLMaterial({})", self.name))
    }
}

impl SwitchGLMaterial {
    pub fn new(def: &MaterialDef, shader: Rc<SwitchGLShader>) -> Self {
        let textures = def
            .textures()
            .iter()
            .map(|t| {
                t.with_image(|img| {
                    let image = img.unwrap_or(&TEXTURE_MISSING);
                    Rc::new(SwitchGLTexture::new(image.width(), image.height(), image))
                })
            })
            .collect();

        Self {
            name: def.debug_name().to_string(),
            shader,
            textures,
            params: *def.params(),
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

    pub fn params(&self) -> &MaterialParams {
        &self.params
    }
}

lazy_static::lazy_static! {
    static ref TEXTURE_MISSING: RgbaImage =
        image::load_from_memory(radiance_assets::TEXTURE_MISSING_TEXTURE_FILE)
            .unwrap()
            .to_rgba8();
}
