use std::rc::Rc;
use std::time::Instant;

use crosscom::ComRc;

use crate::{
    comdef::{IEntityExt, IScene, ISceneExt},
    imgui::ImguiFrame,
    math::Mat44,
    rendering::{ComponentFactory, RenderingEngine, VertexComponents},
    scene::Viewport,
};

use super::{
    factory::SwitchGLComponentFactory, gles::*, imgui_renderer::SwitchGLImguiRenderer,
    render_object::SwitchGLRenderObject,
};

/// Docked resolution. The Switch also runs handheld at 1280x720; the engine
/// re-tracks through `notify_resized`, but EGL gives us the surface the
/// compositor actually handed out, so this is only the initial guess.
const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;

/// Per-frame lighting/fog/time snapshot uploaded to every lit draw. Uniform
/// locations are -1 on programs that lack them, making the uploads no-ops.
struct FrameEnv {
    ambient: [f32; 4],           // rgb + light count
    light_pos: [f32; 16],        // 4x vec4: xyz + outer range
    light_color: [f32; 16],      // 4x vec4: rgb + inner range
    fog_color: [f32; 4],
    fog_params: [f32; 4],        // enabled, start, end, 0
    time_sec: f32,
}

pub struct SwitchGLRenderingEngine {
    factory: Rc<SwitchGLComponentFactory>,
    imgui: SwitchGLImguiRenderer,
    start_time: Instant,
    display: EGLDisplay,
    surface: EGLSurface,
    context: EGLContext,
    extent: (u32, u32),
}

impl SwitchGLRenderingEngine {
    pub fn new() -> anyhow::Result<Self> {
        let (display, surface, context) = unsafe { init_egl()? };

        unsafe {
            glClearColor(0., 0., 0., 1.);
            glViewport(0, 0, DEFAULT_WIDTH as i32, DEFAULT_HEIGHT as i32);
        }

        // The imgui context is already created and current: UiManager is
        // constructed before the rendering engine (see radiance/mod.rs).
        let imgui = SwitchGLImguiRenderer::new()?;

        Ok(Self {
            factory: Rc::new(SwitchGLComponentFactory::new()),
            imgui,
            start_time: Instant::now(),
            display,
            surface,
            context,
            extent: (DEFAULT_WIDTH, DEFAULT_HEIGHT),
        })
    }
}

impl RenderingEngine for SwitchGLRenderingEngine {
    fn render(&mut self, scene: Option<ComRc<IScene>>, _viewport: Viewport, ui_frame: ImguiFrame) {
        unsafe {
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
        }

        if let Some(scene) = scene {
            self.draw_scene(&scene);
        }

        self.imgui.render(ui_frame, self.extent.0, self.extent.1);

        unsafe {
            eglSwapBuffers(self.display, self.surface);
        }
    }

    fn view_extent(&self) -> (u32, u32) {
        self.extent
    }

    fn component_factory(&self) -> Rc<dyn ComponentFactory> {
        self.factory.clone()
    }

    fn begin_frame(&mut self) {}

    fn end_frame(&mut self) {}

    fn notify_resized(&mut self, logical_size: (u32, u32)) {
        self.extent = logical_size;
        unsafe {
            glViewport(0, 0, logical_size.0 as i32, logical_size.1 as i32);
        }
    }

    fn update_imgui_font_atlas(&mut self, _context: &crate::imgui::ImguiContext) {
        // Only reached when core_engine saw the atlas dirty flag; the atlas
        // itself lives in the (current) imgui context, so no args needed.
        self.imgui.rebuild_font_atlas();
    }

    fn render_scene_to_target(
        &mut self,
        scene: ComRc<IScene>,
        target: &mut dyn crate::rendering::RenderTarget,
    ) {
        let Some(target) = target.as_switchgl_mut() else {
            return;
        };
        let (tw, th) = crate::rendering::RenderTarget::extent(target);
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, target.framebuffer());
            glViewport(0, 0, tw as i32, th as i32);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
        }
        self.draw_scene(&scene);
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, self.extent.0 as i32, self.extent.1 as i32);
        }
    }
}

impl SwitchGLRenderingEngine {
    /// Scene pass shared by the swapchain path and offscreen targets. The
    /// camera's projection is used as-is, so a target whose aspect differs
    /// from the main view renders with the main view's aspect.
    fn draw_scene(&mut self, scene: &ComRc<IScene>) {
        {
            let (view, proj) = {
                let camera = scene.camera();
                let view = Mat44::inversed(camera.transform().matrix());
                let proj = *camera.projection_matrix();
                (view, proj)
            };

            // Snapshot the scene lighting once per frame. The Vulkan path
            // uploads 16 lights and picks 2 in-shader; here the first 4 go up
            // and the shader picks its 2 nearest from those.
            let env = {
                let lighting = scene.lighting();
                let mut env = FrameEnv {
                    ambient: [
                        lighting.ambient[0],
                        lighting.ambient[1],
                        lighting.ambient[2],
                        lighting.lights.len().min(4) as f32,
                    ],
                    light_pos: [0.0; 16],
                    light_color: [0.0; 16],
                    fog_color: [0.0; 4],
                    fog_params: [0.0; 4],
                    time_sec: self.start_time.elapsed().as_secs_f32(),
                };
                for (i, l) in lighting.lights.iter().take(4).enumerate() {
                    env.light_pos[i * 4..i * 4 + 4].copy_from_slice(&[
                        l.position.x,
                        l.position.y,
                        l.position.z,
                        l.range[1],
                    ]);
                    env.light_color[i * 4..i * 4 + 4].copy_from_slice(&[
                        l.color[0],
                        l.color[1],
                        l.color[2],
                        l.range[0],
                    ]);
                }
                if let Some(fog) = &lighting.fog {
                    env.fog_color = [fog.color[0], fog.color[1], fog.color[2], 1.0];
                    env.fog_params = [1.0, fog.start, fog.end, 0.0];
                }
                env
            };

            let objects: Vec<Rc<SwitchGLRenderObject>> = scene
                .visible_entities()
                .iter()
                .filter_map(|e| {
                    e.get_rendering_component()
                        .map(|c| (c, e.world_transform().matrix().clone()))
                })
                .flat_map(|(c, m)| {
                    c.switchgl_render_objects()
                        .iter()
                        .map(|o| {
                            o.set_model_matrix(m.clone());
                            o.clone()
                        })
                        .collect::<Vec<_>>()
                })
                .collect();

            for obj in &objects {
                unsafe {
                    draw_object(obj, &view, &proj, &env);
                }
            }
        }
    }
}

impl Drop for SwitchGLRenderingEngine {
    fn drop(&mut self) {
        unsafe {
            eglMakeCurrent(self.display, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
            eglDestroyContext(self.display, self.context);
            eglDestroySurface(self.display, self.surface);
            eglTerminate(self.display);
        }
    }
}

unsafe fn draw_object(obj: &SwitchGLRenderObject, view: &Mat44, proj: &Mat44, env: &FrameEnv) {
    unsafe {
    let material = obj.material();
    let shader = material.shader();

    glUseProgram(shader.program());

    // Frame environment + material params. Locations are -1 (defined no-op)
    // on programs without the uniform, so no per-program branching.
    glUniform4fv(shader.uniform_ambient_light(), 1, env.ambient.as_ptr());
    glUniform4fv(shader.uniform_light_pos(), 4, env.light_pos.as_ptr());
    glUniform4fv(shader.uniform_light_color(), 4, env.light_color.as_ptr());
    glUniform4fv(shader.uniform_fog_color(), 1, env.fog_color.as_ptr());
    glUniform4fv(shader.uniform_fog_params(), 1, env.fog_params.as_ptr());
    glUniform1f(shader.uniform_time_sec(), env.time_sec);

    let params = material.params();
    glUniform4fv(shader.uniform_tint(), 1, params.tint.as_ptr());
    glUniform4fv(
        shader.uniform_material_misc(),
        1,
        [params.alpha_ref, params.intensity, 0.0, 0.0].as_ptr(),
    );
    glUniformMatrix4fv(
        shader.uniform_view_matrix(),
        1,
        GL_FALSE,
        view.floats().as_ptr() as *const _,
    );
    glUniformMatrix4fv(
        shader.uniform_projection_matrix(),
        1,
        GL_FALSE,
        proj.floats().as_ptr() as *const _,
    );
    glUniformMatrix4fv(
        shader.uniform_model_matrix(),
        1,
        GL_FALSE,
        obj.model_matrix().floats().as_ptr() as *const _,
    );

    let textures = material.textures();
    if !textures.is_empty() {
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, textures[0].texture_id());
    }
    if textures.len() > 1 {
        glActiveTexture(GL_TEXTURE1);
        glBindTexture(GL_TEXTURE_2D, textures[1].texture_id());
    }

    glBindBuffer(GL_ARRAY_BUFFER, obj.vertex_buffer());

    glEnableVertexAttribArray(0);
    glVertexAttribPointer(
        0,
        3,
        GL_FLOAT,
        GL_FALSE,
        obj.stride(),
        obj.vertex_offset() as *const _,
    );

    glEnableVertexAttribArray(1);
    glVertexAttribPointer(
        1,
        2,
        GL_FLOAT,
        GL_FALSE,
        obj.stride(),
        obj.tex_coord_offset() as *const _,
    );

    // Feed attribute 2 whenever the buffer carries a second UV set (grass has
    // one texture but still consumes texcoord2 as wind/coverage weights).
    let has_texcoord2 = obj.has_component(VertexComponents::TEXCOORD2);
    if has_texcoord2 {
        glEnableVertexAttribArray(2);
        glVertexAttribPointer(
            2,
            2,
            GL_FLOAT,
            GL_FALSE,
            obj.stride(),
            obj.tex_coord2_offset() as *const _,
        );
    } else {
        glDisableVertexAttribArray(2);
    }

    // Normals for the lit programs, attribute slot 3.
    if obj.has_component(VertexComponents::NORMAL) {
        glEnableVertexAttribArray(3);
        glVertexAttribPointer(
            3,
            3,
            GL_FLOAT,
            GL_FALSE,
            obj.stride(),
            obj.normal_offset() as *const _,
        );
    } else {
        glDisableVertexAttribArray(3);
    }

    glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, obj.index_buffer());
    glDrawElements(
        GL_TRIANGLES,
        obj.index_count(),
        GL_UNSIGNED_INT,
        std::ptr::null(),
    );
    }
}

unsafe fn init_egl() -> anyhow::Result<(EGLDisplay, EGLSurface, EGLContext)> {
    unsafe {
    let display = eglGetDisplay(EGL_DEFAULT_DISPLAY);
    if display == EGL_NO_DISPLAY {
        anyhow::bail!("eglGetDisplay failed: 0x{:x}", eglGetError());
    }

    if eglInitialize(display, std::ptr::null_mut(), std::ptr::null_mut()) == EGL_FALSE {
        anyhow::bail!("eglInitialize failed: 0x{:x}", eglGetError());
    }

    if eglBindAPI(EGL_OPENGL_ES_API) == EGL_FALSE {
        anyhow::bail!("eglBindAPI failed: 0x{:x}", eglGetError());
    }

    let config_attrs = [
        EGL_SURFACE_TYPE,
        EGL_WINDOW_BIT,
        EGL_RENDERABLE_TYPE,
        EGL_OPENGL_ES2_BIT,
        EGL_RED_SIZE,
        8,
        EGL_GREEN_SIZE,
        8,
        EGL_BLUE_SIZE,
        8,
        EGL_ALPHA_SIZE,
        8,
        EGL_DEPTH_SIZE,
        24,
        EGL_NONE,
    ];

    let mut config: EGLConfig = std::ptr::null_mut();
    let mut num_configs: EGLint = 0;
    if eglChooseConfig(
        display,
        config_attrs.as_ptr(),
        &mut config,
        1,
        &mut num_configs,
    ) == EGL_FALSE
        || num_configs == 0
    {
        anyhow::bail!("eglChooseConfig found no usable config: 0x{:x}", eglGetError());
    }

    // libnx hands us the console's default window; EGL treats it as the
    // native window handle.
    let surface = eglCreateWindowSurface(
        display,
        config,
        nwindowGetDefault() as EGLNativeWindowType,
        std::ptr::null(),
    );
    if surface == EGL_NO_SURFACE {
        anyhow::bail!("eglCreateWindowSurface failed: 0x{:x}", eglGetError());
    }

    let context_attrs = [EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE];
    let context = eglCreateContext(display, config, EGL_NO_CONTEXT, context_attrs.as_ptr());
    if context == EGL_NO_CONTEXT {
        anyhow::bail!("eglCreateContext failed: 0x{:x}", eglGetError());
    }

    if eglMakeCurrent(display, surface, surface, context) == EGL_FALSE {
        anyhow::bail!("eglMakeCurrent failed: 0x{:x}", eglGetError());
    }

    eglSwapInterval(display, 1);

    Ok((display, surface, context))
    }
}
