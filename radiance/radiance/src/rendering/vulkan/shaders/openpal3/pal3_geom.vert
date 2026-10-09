#version 450
#extension GL_ARB_separate_shader_objects : enable

// PAL3 static geometry shader (POL/CVD) — vertex stage. Faithfully reproduces
// the original PAL3 `gfxscript/geom_1L.gbf` / `geom_2L.gbf` (`vs_1_1`) model:
// lighting is computed **per vertex** (Gouraud) and modulated by the texture in
// the fragment stage, exactly like the skin shaders but without bone skinning:
//
//   perLight_i = atten_i * max(N·L_i, 0) * diffuse_i.rgb + ambient.rgb
//   color.rgb  = saturate( SUM_{i in 1..2} perLight_i )   // vs_1_1 clamps COLOR
//   finalPixel = texture * color                          // ColorOp = Modulate
//
// World/normal transform follows radiance's row-vector convention (`v * M`).
// Lightmapped POL surfaces use `LightMapMaterialDef` instead of this path.
layout(set = 0, binding = 0) uniform PerFrameUbo {
    mat4 view;
    mat4 proj;
    vec4 ambient;            // rgb = ambient color, w = light count
    vec4 lightPos[16];       // xyz = world position, w = outer range
    vec4 lightColor[16];     // rgb = color, w = inner range
} perFrameUbo;

layout(set = 1, binding = 0) uniform PerInstanceUbo {
    mat4 model;
} perInstanceUbo;

layout(set = 3, binding = 0) uniform MaterialParams {
    vec4 tint;
    vec4 misc;
    vec4 uv_xform;
} mat;

layout(location = 0) in vec3 position;
layout(location = 1) in vec3 normal;
layout(location = 2) in vec2 inTexCoord;

layout(location = 0) out vec2 fragTexCoord;
layout(location = 1) out vec3 fragWorldPos;
layout(location = 2) out vec3 fragColor;

mat4 clip = mat4(vec4(1.0, 0.0, 0.0, 0.0),
                 vec4(0.0, -1.0, 0.0, 0.0),
                 vec4(0.0, 0.0, 0.5, 0.5),
                 vec4(0.0, 0.0, 0, 1.0));

void main() {
    vec4 world = vec4(position, 1.0) * perInstanceUbo.model;
    gl_Position = world * perFrameUbo.view * perFrameUbo.proj * clip;

    vec3 worldPos = world.xyz;
    vec3 N = normalize(normal * mat3(perInstanceUbo.model));

    // 环境项 = 场景环境光 × 材质 Ambient（PAL3 的材质 Ambient 都是 (1,1,1)，
    // 自发光系数全程 0），所以直接取场景环境光。
    vec3 ambient = perFrameUbo.ambient.rgb;

    // 挑两盏灯。原版排序用 `gbLightObj::GetEffectOnPoint`（gbengine.dll
    // `0x100132a0`）= `min(1, 1/(a0 + a1·d + a2·d²)) × (r+g+b)/3`；PAL3 的 `.lgt`
    // 衰减类型一律是 0 ⇒ `a=(1,0,0)` ⇒ 那个系数恒为 1，**排序与距离无关，按颜色最亮取**。
    int count = int(perFrameUbo.ambient.w);
    int best0 = -1, best1 = -1;
    float r0 = 0.0, r1 = 0.0;
    for (int i = 0; i < count; i++) {
        vec3 c = perFrameUbo.lightColor[i].rgb;
        float rank = c.r + c.g + c.b;
        if (rank > r0) {
            r1 = r0; best1 = best0;
            r0 = rank; best0 = i;
        } else if (rank > r1) {
            r1 = rank; best1 = i;
        }
    }

    // 逐顶点 Gouraud 累加，照 geom_1L / geom_2L.gbf。注意 `geom_2L` 里
    // **每盏灯各加一次 ambient**（`color1 = … + ambient[0]`、`color2 = … + ambient[1]`，
    // 再相加），所以两盏灯时环境项是两份；一盏灯或没灯时是一份
    // （没灯走 `geom.gbf`，顶点色就等于 ambient）。
    int lightCount = (best0 >= 0 ? 1 : 0) + (best1 >= 0 ? 1 : 0);
    vec3 lit = ambient * float(max(lightCount, 1));
    int idx[2] = int[2](best0, best1);
    for (int k = 0; k < 2; k++) {
        int i = idx[k];
        if (i < 0) continue;
        vec3 d = perFrameUbo.lightPos[i].xyz - worldPos;
        float dist = length(d);
        vec3 L = dist > 0.0 ? d / dist : vec3(0.0, 1.0, 0.0);

        float outer = perFrameUbo.lightPos[i].w;
        float inner = perFrameUbo.lightColor[i].w;
        float atten = 1.0;
        if (outer < 1.0e18) {
            float edge0 = max(inner, outer * 0.85);
            atten = 1.0 - smoothstep(edge0, outer, dist);
        }

        lit += perFrameUbo.lightColor[i].rgb * max(dot(N, L), 0.0) * atten;
    }

    // vs_1_1 clamps the interpolated COLOR output to [0,1].
    fragColor = clamp(lit, 0.0, 1.0);

    fragWorldPos = worldPos;
    fragTexCoord = inTexCoord * mat.uv_xform.xy + mat.uv_xform.zw;
}
