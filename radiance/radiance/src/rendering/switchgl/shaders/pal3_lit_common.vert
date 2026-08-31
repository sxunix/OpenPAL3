#version 100
// Shared vertex stage for PAL3's Gouraud-lit programs (actor/geom/prop) on
// GLES2. Ported from the Vulkan pal3_actor/pal3_geom pair: per-vertex
// 2-nearest-light Lambert, clamped, texture-modulated in the fragment stage.
// Differences from the Vulkan original, by necessity or convention:
//  * column-vector math (P*V*M) — equivalent to the row-vector original
//    because radiance's row-major Mat44 read untransposed by GL *is* the
//    transpose;
//  * no Vulkan clip matrix (GL NDC wants the projection as-is);
//  * the light table matches the Vulkan original: 16 uploaded lights, the
//    2 nearest picked per vertex (was capped at 4 during bring-up, which
//    dropped the key lamps in scenes with more lights and left them dark);
//  * GLSL ES 100 has no array constructors, so the 2-nearest pick is unrolled.
// `ambientFloor` distinguishes actors (0.55) from scenery (0.0).
uniform mat4 modelMatrix;
uniform mat4 viewMatrix;
uniform mat4 projectionMatrix;
uniform vec4 ambientLight;    // rgb = ambient, w = light count
uniform vec4 lightPos[16];     // xyz = world pos, w = outer range
uniform vec4 lightColor[16];   // rgb = color, w = inner range
uniform float ambientFloor;
uniform vec4 uvXform;         // xy = scale, zw = offset

attribute vec3 position;
attribute vec3 normal;
attribute vec2 texcoord;

varying vec2 v_texcoord;
varying vec3 v_color;
varying float v_viewDepth;

vec3 lightContrib(int i, vec3 worldPos, vec3 N) {
    vec3 d = lightPos[i].xyz - worldPos;
    float dist = length(d);
    vec3 L = dist > 0.0 ? d / dist : vec3(0.0, 1.0, 0.0);
    float outer = lightPos[i].w;
    float inner = lightColor[i].w;
    float atten = 1.0;
    if (outer < 1.0e18) {
        float edge0 = max(inner, outer * 0.85);
        atten = 1.0 - smoothstep(edge0, outer, dist);
    }
    return lightColor[i].rgb * max(dot(N, L), 0.0) * atten;
}

void main() {
    vec4 world = modelMatrix * vec4(position, 1.0);
    vec4 viewPos = viewMatrix * world;
    gl_Position = projectionMatrix * viewPos;
    v_viewDepth = -viewPos.z;

    vec3 worldPos = world.xyz;
    vec3 N = normalize(mat3(modelMatrix) * normal);
    vec3 ambient = max(ambientLight.rgb, vec3(ambientFloor));

    // 2-nearest pick over at most 4 lights, unrolled (no arrays in ES 100).
    int count = int(ambientLight.w);
    int best0 = -1; int best1 = -1;
    float d0 = 1.0e30; float d1 = 1.0e30;
    for (int i = 0; i < 16; i++) {
        if (i >= count) { break; }
        float dist = distance(lightPos[i].xyz, worldPos);
        if (dist < d0) { d1 = d0; best1 = best0; d0 = dist; best0 = i; }
        else if (dist < d1) { d1 = dist; best1 = i; }
    }

    vec3 lit = ambient;
    for (int i = 0; i < 16; i++) {
        if (i == best0 || i == best1) { lit += lightContrib(i, worldPos, N); }
    }

    v_color = clamp(lit, 0.0, 1.0);
    v_texcoord = texcoord * uvXform.xy + uvXform.zw;
}
