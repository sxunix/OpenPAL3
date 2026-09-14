#version 100
// GLSL ES 1.00 port of vulkan/shaders/particle.vert. Per-particle colour and
// alpha arrive in TEXCOORD2: RGB packed as the integer 0xRRGGBB in the first
// float (24 bits are exact in f32), alpha in the second.
uniform mat4 modelMatrix;
uniform mat4 viewMatrix;
uniform mat4 projectionMatrix;
uniform vec4 tint;

attribute vec3 position;
attribute vec2 texcoord;
attribute vec2 texcoord2;

varying vec2 v_texcoord;
varying vec4 v_color;

void main() {
    vec4 mvPosition = viewMatrix * (modelMatrix * vec4(position, 1.0));
    gl_Position = projectionMatrix * mvPosition;
    v_texcoord = texcoord;

    float packed = texcoord2.x;
    float r = floor(packed / 65536.0);
    float g = floor((packed - r * 65536.0) / 256.0);
    float b = packed - r * 65536.0 - g * 256.0;
    v_color = vec4(r / 255.0, g / 255.0, b / 255.0, texcoord2.y) * tint;
}
