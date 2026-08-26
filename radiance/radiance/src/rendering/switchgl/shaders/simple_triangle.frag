#version 100
precision mediump float;

uniform sampler2D texSampler;

varying vec2 v_texcoord;

void main() {
    vec4 color = texture2D(texSampler, v_texcoord);
    // Same alpha-cutout rule as the Vita/Vulkan programs: PAL3 leans on
    // punch-through alpha rather than blending for foliage and fences.
    if (color.a < 0.4) {
        discard;
    }
    color.a = 1.0;
    gl_FragColor = color;
}
