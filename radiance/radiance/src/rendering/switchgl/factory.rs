use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};

use imgui::TextureId;

use crate::rendering::{
    ComponentFactory, MaterialDef, RenderObjectHandle, RenderingComponent, ShaderProgram, Texture,
    TextureDef, VertexBuffer, VideoPlayer,
};

use super::{
    material::SwitchGLMaterial, render_object::SwitchGLRenderObject,
    render_target::SwitchGLRenderTarget, shader::SwitchGLShader, texture::SwitchGLTexture,
};

pub struct SwitchGLComponentFactory {
    shaders: RefCell<HashMap<ShaderProgram, Rc<SwitchGLShader>>>,
    /// GL textures by `TextureDef` name, held weakly: every material of a
    /// scene that names the same image shares one upload, and the texture
    /// goes when the last material using it does. Uploading per material
    /// (as before) put the same image on the GPU dozens of times per scene
    /// load. The Vulkan backend caches by name the same way.
    textures: RefCell<HashMap<String, Weak<SwitchGLTexture>>>,
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
            |t| self.texture_for(t),
        ));
        let ro =
            Rc::new(SwitchGLRenderObject::new(vertices, indices, material, host_dynamic).unwrap());
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
        width: u32,
        height: u32,
    ) -> Box<dyn crate::rendering::RenderTarget> {
        Box::new(SwitchGLRenderTarget::new(width, height))
    }
}

impl SwitchGLComponentFactory {
    pub fn new() -> Self {
        Self {
            shaders: RefCell::new(HashMap::new()),
            textures: RefCell::new(HashMap::new()),
        }
    }

    fn texture_for(&self, def: &TextureDef) -> Rc<SwitchGLTexture> {
        let upload = || {
            def.with_image(|img| {
                let image = img.unwrap_or(&super::material::TEXTURE_MISSING);
                Rc::new(SwitchGLTexture::new(image.width(), image.height(), image))
            })
        };
        let name = def.name();
        if name.is_empty() {
            return upload();
        }
        if let Some(t) = self.textures.borrow().get(name).and_then(Weak::upgrade) {
            return t;
        }
        let t = upload();
        let mut textures = self.textures.borrow_mut();
        if textures.len() >= 1024 {
            textures.retain(|_, w| w.strong_count() > 0);
        }
        textures.insert(name.to_string(), Rc::downgrade(&t));
        t
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
