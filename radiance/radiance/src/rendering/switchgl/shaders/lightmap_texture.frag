#version 100
precision mediump float;

// 贴图和 UV 的配对（和 Vulkan 的 lightmap_texture.frag 一致）：
// 材质的贴图向量是 [光照贴图, 漫反射]，switchgl 按序绑到纹理单元 0/1，
// 所以 texSampler = 光照贴图、texSampler2 = 漫反射；而主 UV 走漫反射
// （会平铺、超出 0..1）、次 UV 走光照贴图（烘焙时展好、都在 0..1）。
// 这里以前是 texSampler 配 v_texcoord、texSampler2 配 v_texcoord2，
// 两样都接反了：拿平铺的漫反射 UV 去采光照贴图图集，正是 Vulkan 那边注释里
// 说的「黑块」成因；抠图判据也落在光照贴图那条恒为 255 的 alpha 上，从不生效。
uniform sampler2D texSampler;   // 光照贴图
uniform sampler2D texSampler2;  // 漫反射贴图

varying vec2 v_texcoord;        // 主 UV = 漫反射
varying vec2 v_texcoord2;       // 次 UV = 光照贴图

void main() {
    vec4 color = texture2D(texSampler2, v_texcoord);
    if (color.a < 0.4) {
        discard;
    }
    vec4 lightmap = texture2D(texSampler, v_texcoord2);
    // 烘焙光照按双倍叠上去，和 Vulkan 那支在 PAL3 参数下（1.5 × 1.333）一致。
    gl_FragColor = vec4(color.rgb * lightmap.rgb * 2.0, 1.0);
}
