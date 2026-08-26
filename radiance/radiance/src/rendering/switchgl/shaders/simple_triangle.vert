#version 100
// GLSL ES 1.00 rewrite of the Cg program the Vita backend uses -- Mesa on
// Switch gives us real GLES2, so the Cg dialect (float4x4/mul/tex2D) is out.
uniform mat4 modelMatrix;
uniform mat4 viewMatrix;
uniform mat4 projectionMatrix;

attribute vec3 position;
attribute vec2 texcoord;

varying vec2 v_texcoord;

void main() {
    vec4 mvPosition = viewMatrix * (modelMatrix * vec4(position, 1.0));
    gl_Position = projectionMatrix * mvPosition;
    v_texcoord = texcoord;
}
