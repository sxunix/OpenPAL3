use super::VertexComponents;

pub trait Shader {
    fn name(&self) -> &str;
}

#[derive(Copy, Clone, Hash, PartialEq, Eq, Debug)]
pub enum ShaderProgram {
    TexturedNoLight,
    TexturedLightmap,
    /// Per-vertex Y-gradient. Used by the PAL4 nav-mesh floor/wall
    /// debug visualization: the fragment color is `mix(low, high, t)`
    /// where `t = (worldY - y_min) / (y_max - y_min)`. Parameters are
    /// carried through the shared `MaterialParams` UBO via field-reuse
    /// (see `GradientYMaterialDef`). Texture binding is a single 1×1
    /// white dummy so the existing per-material descriptor-set plumbing
    /// (which expects ≥ 1 texture) keeps working unchanged.
    GradientY,

    /// Dynamically-lit textured shader: per-pixel Lambert diffuse summed
    /// over the scene's omni point lights plus a flat ambient term. Used
    /// by PAL3 actors (MV3 meshes carry per-frame vertex normals). Reads
    /// the lighting environment from the per-frame UBO (set 0); requires
    /// the `NORMAL` vertex component.
    TexturedDynamicLit,

    /// PAL3 actor (skin) shader. Faithful reproduction of the original PAL3
    /// `gfxscript/skin_lit*.cg` model: at most the two nearest omni point
    /// lights, a hard N·L clamp at 0, ambient added flat, texture-modulated.
    /// Requires `POSITION | NORMAL | TEXCOORD`. Distinct from
    /// `TexturedDynamicLit` (which sums all lights with a wrap floor; used by
    /// PAL5). Lives under `shaders/openpal3/`.
    Pal3Actor,

    /// PAL3 static geometry shader (POL/CVD). Same single-texture Lambert
    /// model as `Pal3Actor` (`gfxscript/geom_1L/2L`) but for non-skinned
    /// scenery; lightmapped POL surfaces use `TexturedLightmap` instead.
    /// Requires `POSITION | NORMAL | TEXCOORD`. Lives under `shaders/openpal3/`.
    Pal3Geom,

    /// PAL5 terrain multi-layer splat shader. Blends up to four terrain
    /// textures (`texSampler[0..4]`) per-texel by a weight atlas
    /// (`texSampler[4]`, RGBA = the four layers' weights), then applies the
    /// same dynamic Lambert lighting as `TexturedDynamicLit`. Requires
    /// `POSITION | NORMAL | TEXCOORD` (the texcoord carries the per-block
    /// weight-atlas UV; terrain-texture tiling UV is derived from world
    /// position). `MaterialParams.misc.x` carries the active layer count.
    TerrainSplat,

    /// PAL5 grass-wind shader. A single alpha-test textured billboard (like
    /// [`ShaderProgram::TexturedNoLight`]) whose vertex stage sways blade tips
    /// over time for a wind effect. Requires `POSITION | TEXCOORD`; reads the
    /// per-frame `time` uniform and carries wind strength/speed in
    /// `MaterialParams.uv_xform.xy`. The grass billboard texture (real
    /// `cao###` color masked by a blade alpha) is baked CPU-side.
    GrassWind,

    /// 粒子（`.par` 特效）。Textured billboards whose colour and alpha
    /// vary per particle: `gbPartGroupParam`'s `RgbColor` and
    /// `VertexAlpha` curves are sampled against each particle's own age,
    /// so a material-wide tint cannot express them. The vertex layout has
    /// no colour component, so the pair rides in `TEXCOORD2` — RGB packed
    /// into the first float as `0xRRGGBB` (exact in an f32 up to 2^24),
    /// alpha in the second. Requires
    /// `POSITION | TEXCOORD | TEXCOORD2`.
    Particle,
}

pub(crate) struct ShaderProgramData {
    pub(crate) name: &'static str,
    pub(crate) vert_src: &'static [u8],
    pub(crate) frag_src: &'static [u8],
    pub(crate) components: VertexComponents,
}

impl ShaderProgramData {
    pub(crate) fn new(
        name: &'static str,
        vert_src: &'static [u8],
        frag_src: &'static [u8],
        components: VertexComponents,
    ) -> Self {
        Self {
            name,
            vert_src,
            frag_src,
            components,
        }
    }
}
