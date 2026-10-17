#version 100
uniform mat4 modelMatrix;
uniform mat4 viewMatrix;
uniform mat4 projectionMatrix;
uniform vec4 uvXform;         // xy = scale, zw = offset

attribute vec3 position;
attribute vec2 texcoord;
attribute vec2 texcoord2;

varying vec2 v_texcoord;
varying vec2 v_texcoord2;

void main() {
    vec4 mvPosition = viewMatrix * (modelMatrix * vec4(position, 1.0));
    gl_Position = projectionMatrix * mvPosition;
    v_texcoord = texcoord * uvXform.xy + uvXform.zw;
    v_texcoord2 = texcoord2;
}
