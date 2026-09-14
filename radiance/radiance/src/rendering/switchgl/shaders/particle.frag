#version 100
precision mediump float;

uniform sampler2D texSampler;

varying vec2 v_texcoord;
varying vec4 v_color;

void main() {
    // Blending comes from the material's pipeline state (additive or alpha);
    // like the Vulkan program, output premultiplied RGB so both share it.
    vec4 c = texture2D(texSampler, v_texcoord) * v_color;
    gl_FragColor = vec4(c.rgb * c.a, c.a);
}
