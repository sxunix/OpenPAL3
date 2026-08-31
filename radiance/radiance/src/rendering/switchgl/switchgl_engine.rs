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
    light_pos: [f32; 64],        // 4x vec4: xyz + outer range
    light_color: [f32; 64],      // 4x vec4: rgb + inner range
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
    /// Bring-up telemetry: scene frames rendered and whether the first
    /// non-empty one has been reported. The log file is the only view into
    /// a run on a console (or an emulator on a locked desktop), and "did
    /// anything reach the draw loop" is the first question it has to answer.
    scene_frames: u64,
    first_draw_logged: bool,
    last_object_count: usize,
}

impl SwitchGLRenderingEngine {
    pub fn new() -> anyhow::Result<Self> {
        let (display, surface, context) = unsafe { init_egl()? };

        unsafe {
            glClearColor(0., 0., 0., 1.);
            glViewport(0, 0, DEFAULT_WIDTH as i32, DEFAULT_HEIGHT as i32);

            let gl_str = |name: u32| -> String {
                let p = glGetString(name);
                if p.is_null() {
                    "<null>".into()
                } else {
                    std::ffi::CStr::from_ptr(p as *const _).to_string_lossy().into_owned()
                }
            };
            let mut max_tex = 0i32;
            glGetIntegerv(GL_MAX_TEXTURE_SIZE, &mut max_tex);
            log::info!(
                "switchgl: vendor={} renderer={} version={} max_texture_size={}",
                gl_str(GL_VENDOR),
                gl_str(GL_RENDERER),
                gl_str(GL_VERSION),
                max_tex
            );
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
            scene_frames: 0,
            first_draw_logged: false,
            last_object_count: 0,
        })
    }
}

impl RenderingEngine for SwitchGLRenderingEngine {
    fn render(&mut self, scene: Option<ComRc<IScene>>, _viewport: Viewport, ui_frame: ImguiFrame) {
        unsafe {
            // A material with DepthMode::TestOnly leaves the depth mask off,
            // and glClear honors the mask -- restore it or the depth buffer
            // stops clearing after the first translucent draw.
            glDepthMask(GL_TRUE);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
        }

        if let Some(scene) = scene {
            self.draw_scene(&scene);
        }

        self.imgui.render(ui_frame, self.extent.0, self.extent.1);

        unsafe {
            // Throttled GL error probe: one line per 600 frames when anything
            // in the frame errored, silent otherwise.
            if self.scene_frames % 600 == 1 {
                let err = glGetError();
                if err != 0 {
                    log::error!("switchgl: glGetError() = {err:#x} (frame {})", self.scene_frames);
                }
            }
            let ok = eglSwapBuffers(self.display, self.surface);
            if ok == 0 && self.scene_frames % 600 == 1 {
                log::error!("switchgl: eglSwapBuffers failed: {:#x}", eglGetError());
            }
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
                        lighting.lights.len().min(16) as f32,
                    ],
                    light_pos: [0.0; 64],
                    light_color: [0.0; 64],
                    fog_color: [0.0; 4],
                    fog_params: [0.0; 4],
                    time_sec: self.start_time.elapsed().as_secs_f32(),
                };
                for (i, l) in lighting.lights.iter().take(16).enumerate() {
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

            self.scene_frames += 1;
            if !objects.is_empty() && !self.first_draw_logged {
                self.first_draw_logged = true;
                let mut by_shader: std::collections::BTreeMap<&str, usize> =
                    std::collections::BTreeMap::new();
                for o in &objects {
                    *by_shader.entry(o.material().shader().program_name()).or_insert(0) += 1;
                }
                log::info!(
                    "switchgl: first scene frame with {} objects ({} visible entities, frame {}) {:?}",
                    objects.len(),
                    scene.visible_entities().len(),
                    self.scene_frames,
                    by_shader
                );
            } else if objects.len() != self.last_object_count {
                let mut by_shader: std::collections::BTreeMap<&str, usize> =
                    std::collections::BTreeMap::new();
                for o in &objects {
                    *by_shader.entry(o.material().shader().program_name()).or_insert(0) += 1;
                }
                log::info!(
                    "switchgl: frame {}, objects {} -> {} ({:?})",
                    self.scene_frames,
                    self.last_object_count,
                    objects.len(),
                    by_shader
                );
            }
            self.last_object_count = objects.len();

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

    // Per-material blend / depth / cull, mirroring the Vulkan pipeline
    // exactly (pipeline.rs): sources are premultiplied by TextureStore, so
    // AlphaTest and AlphaBlend both use ONE / ONE_MINUS_SRC_ALPHA; the
    // fragment shaders keep their cutout discard. Missing all of this was a
    // wall of opaque black wherever the scene expected translucency.
    match material.blend() {
        crate::rendering::BlendMode::Opaque => glDisable(GL_BLEND),
        crate::rendering::BlendMode::AlphaTest | crate::rendering::BlendMode::AlphaBlend => {
            glEnable(GL_BLEND);
            glBlendFuncSeparate(GL_ONE, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
        }
        crate::rendering::BlendMode::Additive => {
            glEnable(GL_BLEND);
            glBlendFuncSeparate(GL_ONE, GL_ONE, GL_ZERO, GL_ONE);
        }
        crate::rendering::BlendMode::Multiply => {
            glEnable(GL_BLEND);
            glBlendFuncSeparate(GL_DST_COLOR, GL_ZERO, GL_ZERO, GL_ONE);
        }
    }
    match material.depth() {
        crate::rendering::DepthMode::TestWrite => {
            glEnable(GL_DEPTH_TEST);
            glDepthMask(GL_TRUE);
        }
        crate::rendering::DepthMode::TestOnly => {
            glEnable(GL_DEPTH_TEST);
            glDepthMask(GL_FALSE);
        }
        crate::rendering::DepthMode::Disabled => {
            glDisable(GL_DEPTH_TEST);
            glDepthMask(GL_FALSE);
        }
    }
    match material.cull() {
        crate::rendering::CullMode::Back => {
            glEnable(GL_CULL_FACE);
            glCullFace(GL_BACK);
        }
        crate::rendering::CullMode::Front => {
            glEnable(GL_CULL_FACE);
            glCullFace(GL_FRONT);
        }
        crate::rendering::CullMode::None => glDisable(GL_CULL_FACE),
    }

    // Frame environment + material params. Locations are -1 (defined no-op)
    // on programs without the uniform, so no per-program branching.
    glUniform4fv(shader.uniform_ambient_light(), 1, env.ambient.as_ptr());
    glUniform4fv(shader.uniform_light_pos(), 16, env.light_pos.as_ptr());
    glUniform4fv(shader.uniform_light_color(), 16, env.light_color.as_ptr());
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
    // radiance's Mat44 is row-major storage of COLUMN-vector matrices
    // (Transform puts translation in [r][3]; the projection puts -1 in
    // [3][2]). GLES2 requires transpose=GL_FALSE, so the raw floats reach
    // the shader as the TRANSPOSE, and `P*V*M*v` there computed
    // (M*V*P)^T * v -- a recognizably-wrong scene: coherent surfaces at
    // warped angles, the camera seemingly inside geometry, while the same
    // camera values rendered correctly on the desktop reference. Transpose
    // on the CPU so the shader's column math sees the real matrices.
    // (Stage-5's claim that "untransposed upload is the transpose, which
    // makes the row-vector math work out" had it backwards: the engine's
    // convention is column vectors, not row vectors.)
    let view_t = Mat44::transposed(view);
    let proj_t = Mat44::transposed(proj);
    let model_t = Mat44::transposed(&obj.model_matrix());
    glUniformMatrix4fv(
        shader.uniform_view_matrix(),
        1,
        GL_FALSE,
        view_t.floats().as_ptr() as *const _,
    );
    glUniformMatrix4fv(
        shader.uniform_projection_matrix(),
        1,
        GL_FALSE,
        proj_t.floats().as_ptr() as *const _,
    );
    glUniformMatrix4fv(
        shader.uniform_model_matrix(),
        1,
        GL_FALSE,
        model_t.floats().as_ptr() as *const _,
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
    // native window handle. Left alone, the NWindow picks its own buffer
    // size (1280x720), while everything here assumes 1920x1080 -- the
    // viewport then overhangs the buffer and the whole frame renders 1.5x
    // too large with the right/bottom cut off (found on the Ryujinx
    // bring-up as a consistent 1.5x stretch in every screenshot).
    let nw = nwindowGetDefault();
    let rc = nwindowSetDimensions(nw, DEFAULT_WIDTH, DEFAULT_HEIGHT);
    if rc != 0 {
        log::warn!("nwindowSetDimensions({DEFAULT_WIDTH}x{DEFAULT_HEIGHT}) failed: {rc:#x}");
    }
    let surface = eglCreateWindowSurface(
        display,
        config,
        nw as EGLNativeWindowType,
        std::ptr::null(),
    );
    if surface == EGL_NO_SURFACE {
        anyhow::bail!("eglCreateWindowSurface failed: 0x{:x}", eglGetError());
    }
    let (mut sw, mut sh): (EGLint, EGLint) = (0, 0);
    eglQuerySurface(display, surface, EGL_WIDTH, &mut sw);
    eglQuerySurface(display, surface, EGL_HEIGHT, &mut sh);
    log::info!("egl surface: {}x{}", sw, sh);

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
