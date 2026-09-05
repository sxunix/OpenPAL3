#version 450
#extension GL_ARB_separate_shader_objects : enable

// 粒子：逐颗的颜色和 alpha 通过 TEXCOORD2 传进来。
//
// 顶点布局里没有颜色分量，但 TEXCOORD2 是现成接通的两个 float，所以
// 把 RGB 打包进第一个（0xRRGGBB 当整数，24 位在 f32 里精确可表示）、
// alpha 放第二个。gbParticleGroup 的四条曲线里 RgbColor 和
// VertexAlpha 都是按粒子自己的年龄取值的，一组里每颗都不一样，整块
// 材质的 tint 表达不了。

layout(set = 0, binding = 0) uniform PerFrameUbo {
    mat4 view;
    mat4 proj;
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
layout(location = 2) in vec2 inTexCoord;
layout(location = 3) in vec2 inPacked;

layout(location = 0) out vec2 fragTexCoord;
layout(location = 1) out vec4 fragColor;

mat4 clip = mat4(vec4(1.0, 0.0, 0.0, 0.0),
                 vec4(0.0, -1.0, 0.0, 0.0),
                 vec4(0.0, 0.0, 0.5, 0.5),
                 vec4(0.0, 0.0, 0, 1.0));

void main() {
    vec4 world = vec4(position, 1.0) * perInstanceUbo.model;
    gl_Position = world * perFrameUbo.view * perFrameUbo.proj * clip;
    fragTexCoord = inTexCoord * mat.uv_xform.xy + mat.uv_xform.zw;

    float packed = inPacked.x;
    float r = floor(packed / 65536.0);
    float g = floor((packed - r * 65536.0) / 256.0);
    float b = packed - r * 65536.0 - g * 256.0;
    fragColor = vec4(r / 255.0, g / 255.0, b / 255.0, inPacked.y) * mat.tint;
}
