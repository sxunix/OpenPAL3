use std::ffi::CString;

use crate::rendering::{Shader, ShaderProgram, VertexComponents, shader::ShaderProgramData};

use super::gles::*;

pub struct SwitchGLShader {
    name: String,
    vertex_shader: u32,
    fragment_shader: u32,
    program: u32,
    uniform_model_matrix: i32,
    uniform_view_matrix: i32,
    uniform_projection_matrix: i32,
    // Lighting / material / time uniforms for the lit + grass programs.
    // glGetUniformLocation returns -1 where a program lacks one, and
    // glUniform* on -1 is a defined no-op, so the engine uploads
    // unconditionally.
    uniform_ambient_light: i32,
    uniform_light_pos: i32,
    uniform_light_color: i32,
    uniform_fog_color: i32,
    uniform_fog_params: i32,
    uniform_tint: i32,
    uniform_material_misc: i32,
    uniform_uv_xform: i32,
    uniform_time_sec: i32,
}

impl Shader for SwitchGLShader {
    fn name(&self) -> &str {
        &self.name
    }
}

impl SwitchGLShader {
    pub fn new(shader: ShaderProgram) -> anyhow::Result<Self> {
        let data = get_shader_program_data(shader);
        log::info!("loading shader {}", data.name);

        unsafe {
            let vertex_shader = glCreateShader(GL_VERTEX_SHADER);
            let fragment_shader = glCreateShader(GL_FRAGMENT_SHADER);
            let program = glCreateProgram();

            let vert_src = CString::new(data.vert_src)?;
            let frag_src = CString::new(data.frag_src)?;
            let vsize = data.vert_src.len() as i32;
            let fsize = data.frag_src.len() as i32;

            glShaderSource(vertex_shader, 1, &vert_src.as_ptr(), &vsize);
            glCompileShader(vertex_shader);
            check_shader(vertex_shader, &data.name, "vertex")?;

            glShaderSource(fragment_shader, 1, &frag_src.as_ptr(), &fsize);
            glCompileShader(fragment_shader);
            check_shader(fragment_shader, &data.name, "fragment")?;

            glAttachShader(program, vertex_shader);
            glAttachShader(program, fragment_shader);

            // Attribute locations must be bound *before* linking. The engine's
            // draw loop hardcodes 0/1/2 for position/texcoord/texcoord2.
            let position = CString::new("position")?;
            let texcoord = CString::new("texcoord")?;
            glBindAttribLocation(program, 0, position.as_ptr());
            glBindAttribLocation(program, 1, texcoord.as_ptr());
            if data.components.contains(VertexComponents::TEXCOORD2) {
                let texcoord2 = CString::new("texcoord2")?;
                glBindAttribLocation(program, 2, texcoord2.as_ptr());
            }
            if data.components.contains(VertexComponents::NORMAL) {
                let normal = CString::new("normal")?;
                glBindAttribLocation(program, 3, normal.as_ptr());
            }

            glLinkProgram(program);
            check_program(program, &data.name)?;

            // Sampler uniforms are program state, so the program has to be
            // current before glUniform1i. The Vita path gets away without this
            // because vitaGL defers uniform writes; real GLES2 does not.
            glUseProgram(program);
            let sampler = CString::new("texSampler")?;
            glUniform1i(glGetUniformLocation(program, sampler.as_ptr()), 0);
            if data.components.contains(VertexComponents::TEXCOORD2) {
                let sampler2 = CString::new("texSampler2")?;
                glUniform1i(glGetUniformLocation(program, sampler2.as_ptr()), 1);
            }
            glUseProgram(0);

            let loc = |name: &str| -> i32 {
                let c = CString::new(name).unwrap();
                glGetUniformLocation(program, c.as_ptr())
            };

            // Actors keep PAL3's high material ambient; scenery uses the dim
            // scene ambient as-is. Program state, set once.
            let floor = match shader {
                ShaderProgram::Pal3Actor | ShaderProgram::TexturedDynamicLit => 0.55f32,
                _ => 0.0f32,
            };
            glUseProgram(program);
            glUniform1f(loc("ambientFloor"), floor);
            // Neutral defaults so programs render sanely before the first
            // per-draw upload (and for programs the engine never feeds).
            glUniform4fv(loc("tint"), 1, [1.0f32, 1.0, 1.0, 1.0].as_ptr());
            glUniform4fv(loc("uvXform"), 1, [1.0f32, 1.0, 0.0, 0.0].as_ptr());
            glUniform4fv(loc("materialMisc"), 1, [0.4f32, 1.0, 0.0, 0.0].as_ptr());
            glUseProgram(0);

            Ok(Self {
                name: data.name.to_owned(),
                vertex_shader,
                fragment_shader,
                program,
                uniform_model_matrix: loc("modelMatrix"),
                uniform_view_matrix: loc("viewMatrix"),
                uniform_projection_matrix: loc("projectionMatrix"),
                uniform_ambient_light: loc("ambientLight"),
                uniform_light_pos: loc("lightPos"),
                uniform_light_color: loc("lightColor"),
                uniform_fog_color: loc("fogColor"),
                uniform_fog_params: loc("fogParams"),
                uniform_tint: loc("tint"),
                uniform_material_misc: loc("materialMisc"),
                uniform_uv_xform: loc("uvXform"),
                uniform_time_sec: loc("timeSec"),
            })
        }
    }

    pub fn program(&self) -> u32 {
        self.program
    }

    pub fn uniform_model_matrix(&self) -> i32 {
        self.uniform_model_matrix
    }

    pub fn uniform_view_matrix(&self) -> i32 {
        self.uniform_view_matrix
    }

    pub fn uniform_projection_matrix(&self) -> i32 {
        self.uniform_projection_matrix
    }

    pub fn uniform_ambient_light(&self) -> i32 { self.uniform_ambient_light }
    pub fn uniform_light_pos(&self) -> i32 { self.uniform_light_pos }
    pub fn uniform_light_color(&self) -> i32 { self.uniform_light_color }
    pub fn uniform_fog_color(&self) -> i32 { self.uniform_fog_color }
    pub fn uniform_fog_params(&self) -> i32 { self.uniform_fog_params }
    pub fn uniform_tint(&self) -> i32 { self.uniform_tint }
    pub fn uniform_material_misc(&self) -> i32 { self.uniform_material_misc }
    pub fn uniform_uv_xform(&self) -> i32 { self.uniform_uv_xform }
    pub fn uniform_time_sec(&self) -> i32 { self.uniform_time_sec }
}

impl Drop for SwitchGLShader {
    fn drop(&mut self) {
        unsafe {
            glDeleteShader(self.vertex_shader);
            glDeleteShader(self.fragment_shader);
            glDeleteProgram(self.program);
        }
    }
}

fn get_shader_program_data(shader: ShaderProgram) -> ShaderProgramData {
    // All programs now have real GLES2 implementations. Remaining
    // approximations: TerrainSplat renders its base layer only (no splat
    // blend), and TexturedDynamicLit borrows PAL3's per-vertex Gouraud model
    // in place of actor_lit's per-pixel one.
    match shader {
        ShaderProgram::TexturedNoLight => ShaderProgramData::new(
            "TexturedNoLight",
            include_bytes!("shaders/simple_triangle.vert"),
            include_bytes!("shaders/simple_triangle.frag"),
            VertexComponents::POSITION | VertexComponents::TEXCOORD,
        ),
        ShaderProgram::TexturedLightmap => ShaderProgramData::new(
            "TexturedLightmap",
            include_bytes!("shaders/lightmap_texture.vert"),
            include_bytes!("shaders/lightmap_texture.frag"),
            VertexComponents::POSITION | VertexComponents::TEXCOORD | VertexComponents::TEXCOORD2,
        ),
        ShaderProgram::GradientY => ShaderProgramData::new(
            "GradientY",
            include_bytes!("shaders/gradient_y.vert"),
            include_bytes!("shaders/gradient_y.frag"),
            VertexComponents::POSITION | VertexComponents::TEXCOORD,
        ),
        // actor_lit's per-pixel model is approximated by the same per-vertex
        // Gouraud pair PAL3 actors use (with the actor ambient floor).
        ShaderProgram::TexturedDynamicLit => ShaderProgramData::new(
            "TexturedDynamicLit",
            include_bytes!("shaders/pal3_lit_common.vert"),
            include_bytes!("shaders/pal3_lit_common.frag"),
            VertexComponents::POSITION | VertexComponents::NORMAL | VertexComponents::TEXCOORD,
        ),
        // Terrain splat (multi-layer blend) is approximated as single-texture
        // lit geometry -- still a step up from unlit.
        ShaderProgram::TerrainSplat => ShaderProgramData::new(
            "TerrainSplat",
            include_bytes!("shaders/pal3_lit_common.vert"),
            include_bytes!("shaders/pal3_lit_common.frag"),
            VertexComponents::POSITION | VertexComponents::NORMAL | VertexComponents::TEXCOORD,
        ),
        ShaderProgram::GrassWind => ShaderProgramData::new(
            "GrassWind",
            include_bytes!("shaders/grass.vert"),
            include_bytes!("shaders/grass.frag"),
            VertexComponents::POSITION | VertexComponents::TEXCOORD | VertexComponents::TEXCOORD2,
        ),
        ShaderProgram::Pal3Actor => ShaderProgramData::new(
            "Pal3Actor",
            include_bytes!("shaders/pal3_lit_common.vert"),
            include_bytes!("shaders/pal3_lit_common.frag"),
            VertexComponents::POSITION | VertexComponents::NORMAL | VertexComponents::TEXCOORD,
        ),
        ShaderProgram::Pal3Geom => ShaderProgramData::new(
            "Pal3Geom",
            include_bytes!("shaders/pal3_lit_common.vert"),
            include_bytes!("shaders/pal3_lit_common.frag"),
            VertexComponents::POSITION | VertexComponents::NORMAL | VertexComponents::TEXCOORD,
        ),
        ShaderProgram::Pal3Prop => ShaderProgramData::new(
            "Pal3Prop",
            include_bytes!("shaders/pal3_lit_common.vert"),
            include_bytes!("shaders/pal3_lit_common.frag"),
            VertexComponents::POSITION | VertexComponents::NORMAL | VertexComponents::TEXCOORD,
        ),
    }
}

fn check_shader(shader: u32, name: &str, stage: &str) -> anyhow::Result<()> {
    unsafe {
        let mut status = 0;
        glGetShaderiv(shader, GL_COMPILE_STATUS, &mut status);
        if status != GL_TRUE as i32 {
            anyhow::bail!("{} {} shader failed to compile: {}", name, stage, shader_log(shader));
        }
    }
    Ok(())
}

fn check_program(program: u32, name: &str) -> anyhow::Result<()> {
    unsafe {
        let mut status = 0;
        glGetProgramiv(program, GL_LINK_STATUS, &mut status);
        if status != GL_TRUE as i32 {
            let mut log_length = 0;
            glGetProgramiv(program, GL_INFO_LOG_LENGTH, &mut log_length);
            let mut log = vec![0u8; log_length.max(0) as usize];
            glGetProgramInfoLog(
                program,
                log_length,
                std::ptr::null_mut(),
                log.as_mut_ptr() as *mut _,
            );
            anyhow::bail!(
                "{} program failed to link: {}",
                name,
                String::from_utf8_lossy(&log)
            );
        }
    }
    Ok(())
}

fn shader_log(shader: u32) -> String {
    unsafe {
        let mut log_length = 0;
        glGetShaderiv(shader, GL_INFO_LOG_LENGTH, &mut log_length);
        let mut log = vec![0u8; log_length.max(0) as usize];
        glGetShaderInfoLog(
            shader,
            log_length,
            std::ptr::null_mut(),
            log.as_mut_ptr() as *mut _,
        );
        String::from_utf8_lossy(&log).into_owned()
    }
}
