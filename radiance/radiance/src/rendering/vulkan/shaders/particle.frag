#version 450
#extension GL_ARB_separate_shader_objects : enable

layout(set = 2, binding = 0) uniform sampler2D texSampler;

layout(location = 0) in vec2 fragTexCoord;
layout(location = 1) in vec4 fragColor;

layout(location = 0) out vec4 outColor;

void main() {
    vec4 texel = texture(texSampler, fragTexCoord);
    // 混合方式由管线状态决定（加法或 alpha），这里只负责调制。
    // 与 pal3_actor 一样输出预乘的 RGB，好让两种混合共用同一套状态。
    vec4 c = texel * fragColor;
    outColor = vec4(c.rgb * c.a, c.a);
}
