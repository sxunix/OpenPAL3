use std::cell::{RefCell, RefMut};
use std::rc::Rc;

use crate::{
    math::Mat44,
    rendering::{RenderObject, VertexBuffer, VertexComponents},
};

use super::{gles::*, material::SwitchGLMaterial};

pub struct SwitchGLRenderObject {
    buffers: [u32; 2],
    vertices: RefCell<VertexBuffer>,
    material: Rc<SwitchGLMaterial>,
    indices: Vec<u32>,

    model_matrix: RefCell<Mat44>,
}

impl RenderObject for SwitchGLRenderObject {
    fn update_vertices(&self, updater: &dyn Fn(RefMut<VertexBuffer>)) {
        updater(self.vertices.borrow_mut());
        unsafe {
            let vertices = self.vertices.borrow();
            glBindBuffer(GL_ARRAY_BUFFER, self.buffers[0]);
            glBufferData(
                GL_ARRAY_BUFFER,
                vertices.data().len() as GLsizeiptr,
                vertices.data().as_ptr() as *const _,
                GL_DYNAMIC_DRAW,
            );
            glBindBuffer(GL_ARRAY_BUFFER, 0);
        }
    }

    fn material_debug_name(&self) -> Option<&str> {
        Some(self.material.name())
    }
}

impl SwitchGLRenderObject {
    pub fn new(
        vertices: VertexBuffer,
        indices: Vec<u32>,
        material: Rc<SwitchGLMaterial>,
        host_dynamic: bool,
    ) -> anyhow::Result<Self> {
        let mut buffers = [0u32; 2];

        unsafe {
            glGenBuffers(buffers.len() as i32, buffers.as_mut_ptr());

            glBindBuffer(GL_ARRAY_BUFFER, buffers[0]);
            glBufferData(
                GL_ARRAY_BUFFER,
                vertices.data().len() as GLsizeiptr,
                vertices.data().as_ptr() as *const _,
                if host_dynamic {
                    GL_DYNAMIC_DRAW
                } else {
                    GL_STATIC_DRAW
                },
            );
            glBindBuffer(GL_ARRAY_BUFFER, 0);

            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, buffers[1]);
            glBufferData(
                GL_ELEMENT_ARRAY_BUFFER,
                (std::mem::size_of::<u32>() * indices.len()) as GLsizeiptr,
                indices.as_ptr() as *const _,
                GL_STATIC_DRAW,
            );
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        }

        Ok(Self {
            buffers,
            vertices: RefCell::new(vertices),
            material,
            indices,
            model_matrix: RefCell::new(Mat44::new_identity()),
        })
    }

    pub fn material(&self) -> &SwitchGLMaterial {
        &self.material
    }

    pub fn stride(&self) -> i32 {
        self.vertices.borrow().layout().size() as i32
    }

    pub fn vertex_offset(&self) -> usize {
        self.offset_of(VertexComponents::POSITION).unwrap_or(0)
    }

    pub fn normal_offset(&self) -> usize {
        self.offset_of(VertexComponents::NORMAL).unwrap_or(0)
    }

    pub fn tex_coord_offset(&self) -> usize {
        self.offset_of(VertexComponents::TEXCOORD).unwrap_or(0)
    }

    pub fn tex_coord2_offset(&self) -> usize {
        self.offset_of(VertexComponents::TEXCOORD2).unwrap_or(0)
    }

    /// The Vita backend unwraps these directly, which panics for any layout
    /// missing the component. Several fallback shader programs here declare
    /// components the buffer may not carry, so treat a missing one as offset 0
    /// and let the (ignored) attribute read garbage rather than abort.
    fn offset_of(&self, component: VertexComponents) -> Option<usize> {
        self.vertices
            .borrow()
            .layout()
            .get_offset(component)
            .map(|o| o as usize)
    }

    pub fn has_component(&self, component: VertexComponents) -> bool {
        self.vertices
            .borrow()
            .layout()
            .get_offset(component)
            .is_some()
    }

    pub fn vertex_buffer(&self) -> u32 {
        self.buffers[0]
    }

    pub fn index_buffer(&self) -> u32 {
        self.buffers[1]
    }

    pub fn index_count(&self) -> i32 {
        self.indices.len() as i32
    }

    pub fn set_model_matrix(&self, model_matrix: Mat44) {
        self.model_matrix.replace(model_matrix);
    }

    pub fn model_matrix(&self) -> Mat44 {
        self.model_matrix.borrow().clone()
    }
}

impl Drop for SwitchGLRenderObject {
    fn drop(&mut self) {
        unsafe {
            glDeleteBuffers(self.buffers.len() as i32, self.buffers.as_ptr());
        }
    }
}
