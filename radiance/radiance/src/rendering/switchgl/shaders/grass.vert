#version 100
// GrassWind vertex stage: sinusoidal sway driven by elapsed time; tip weight
// comes from texcoord2.x, coverage from texcoord2.y (same contract as the
// Vulkan grass.vert; strength/speed ride in the material's uv_xform slot).
uniform mat4 modelMatrix;
uniform mat4 viewMatrix;
uniform mat4 projectionMatrix;
uniform vec4 uvXform;         // x = strength, y = speed (grass reuses the slot)
uniform float timeSec;

attribute vec3 position;
attribute vec2 texcoord;
attribute vec2 texcoord2;

varying vec2 v_texcoord;
varying float v_coverage;
varying float v_viewDepth;

void main() {
    vec4 world = modelMatrix * vec4(position, 1.0);

    float tipWeight = clamp(texcoord2.x, 0.0, 1.0);
    float phase = timeSec * uvXform.y + (world.x + world.z) * 0.012;
    float sway = (sin(phase) + 0.3 * sin(phase * 2.7)) * uvXform.x * tipWeight;
    world.x += sway;
    world.z += sway * 0.5;

    vec4 viewPos = viewMatrix * world;
    gl_Position = projectionMatrix * viewPos;
    v_viewDepth = -viewPos.z;
    v_texcoord = texcoord;
    v_coverage = texcoord2.y;
}
