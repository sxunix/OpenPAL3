#version 100
precision mediump float;

uniform sampler2D texSampler;

varying vec2 v_texcoord;

void main() {
    vec4 color = texture2D(texSampler, v_texcoord);
    // Vertical fade used by PAL3's sky/backdrop quads.
    gl_FragColor = vec4(color.rgb, color.a * (1.0 - v_texcoord.y));
}
