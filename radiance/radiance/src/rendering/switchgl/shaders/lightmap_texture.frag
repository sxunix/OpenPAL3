#version 100
precision mediump float;

uniform sampler2D texSampler;
uniform sampler2D texSampler2;

varying vec2 v_texcoord;
varying vec2 v_texcoord2;

void main() {
    vec4 color = texture2D(texSampler, v_texcoord);
    if (color.a < 0.4) {
        discard;
    }
    vec4 lightmap = texture2D(texSampler2, v_texcoord2);
    // Baked lightmap is modulated in at double intensity, matching the
    // convention the Vulkan lightmap program uses.
    gl_FragColor = vec4(color.rgb * lightmap.rgb * 2.0, 1.0);
}
