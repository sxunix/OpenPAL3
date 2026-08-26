use std::{cell::RefCell, collections::HashMap, rc::Rc};

use imgui::TextureId;

use crate::rendering::{
    ComponentFactory, MaterialDef, RenderObjectHandle, RenderingComponent, ShaderProgram, Texture,
    TextureDef, VertexBuffer, VideoPlayer,
};

use super::{
    material::SwitchGLMaterial, render_object::SwitchGLRenderObject, shader::SwitchGLShader,
    texture::SwitchGLTexture,
};

pub struct SwitchGLComponentFactory {
    shaders: RefCell<HashMap<ShaderProgram, Rc<SwitchGLShader>>>,
}

impl ComponentFactory for SwitchGLComponentFactory {
    fn create_texture(&self, texture_def: &TextureDef) -> Box<dyn Texture> {
        texture_def.with_image(|img| {
            let rgba_image = img.unwrap_or(&TEXTURE_MISSING_IMAGE);
            Box::new(SwitchGLTexture::new(
                rgba_image.width(),
                rgba_image.height(),
                rgba_image,
            )) as Box<dyn Texture>
        })
    }

    fn create_imgui_texture(
        &self,
        buffer: &[u8],
        _row_length: u32,
        width: u32,
        height: u32,
        _texture_id: Option<TextureId>,
    ) -> (Box<dyn Texture>, TextureId) {
        let texture = SwitchGLTexture::new(width, height, buffer);
        let texture_id = texture.texture_id();
        (Box::new(texture), TextureId::new(texture_id as usize))
    }

    fn remove_imgui_texture(&self, _texture_id: Option<TextureId>) {}

    fn create_render_object(
        &self,
        vertices: VertexBuffer,
        indices: Vec<u32>,
        material_def: &MaterialDef,
        host_dynamic: bool,
    ) -> RenderObjectHandle {
        let material = Rc::new(SwitchGLMaterial::new(
            material_def,
            self.create_shader(material_def.shader()),
        ));
        let ro = Rc::new(
            SwitchGLRenderObject::new(vertices, indices, material, host_dynamic).unwrap(),
        );
        RenderObjectHandle::from_switchgl(ro)
    }

    fn create_rendering_component(&self, objects: Vec<RenderObjectHandle>) -> RenderingComponent {
        let mut component = RenderingComponent::new();
        for o in objects {
            component.push_render_object(o);
        }
        component
    }

    fn create_video_player(&self) -> Box<VideoPlayer> {
        Box::new(VideoPlayer::new())
    }

    fn create_render_target(
        &self,
        _width: u32,
        _height: u32,
    ) -> Box<dyn crate::rendering::RenderTarget> {
        unimplemented!("switchgl backend does not support offscreen render targets");
    }
}

impl SwitchGLComponentFactory {
    pub fn new() -> Self {
        Self {
            shaders: RefCell::new(HashMap::new()),
        }
    }

    fn create_shader(&self, shader: ShaderProgram) -> Rc<SwitchGLShader> {
        self.shaders
            .borrow_mut()
            .entry(shader)
            .or_insert_with(|| Rc::new(SwitchGLShader::new(shader).unwrap()))
            .clone()
    }
}

lazy_static::lazy_static! {
    static ref TEXTURE_MISSING_IMAGE: image::RgbaImage =
        image::load_from_memory(radiance_assets::TEXTURE_MISSING_TEXTURE_FILE)
            .unwrap()
            .to_rgba8();
}
