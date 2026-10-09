#version 100
// Shared vertex stage for PAL3's Gouraud-lit programs (actor/geom/prop) on
// GLES2. Ported from the Vulkan pal3_actor/pal3_geom pair: per-vertex
// 2-light Lambert（按亮度取），clamped, texture-modulated in the fragment stage.
// Differences from the Vulkan original, by necessity or convention:
//  * column-vector math (P*V*M) — equivalent to the row-vector original
//    because radiance's row-major Mat44 read untransposed by GL *is* the
//    transpose;
//  * no Vulkan clip matrix (GL NDC wants the projection as-is);
//  * the light table matches the Vulkan original: 16 uploaded lights, the
//    2 brightest picked per vertex;
//  * GLSL ES 100 has no array constructors, so the pick is unrolled.
uniform mat4 modelMatrix;
uniform mat4 viewMatrix;
uniform mat4 projectionMatrix;
uniform vec4 ambientLight;    // rgb = ambient, w = light count
uniform vec4 lightPos[16];     // xyz = world pos, w = outer range
uniform vec4 lightColor[16];   // rgb = color, w = inner range
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
    vec3 ambient = ambientLight.rgb;

    // 挑最亮的两盏（原版 GetEffectOnPoint 的距离项对 PAL3 的无衰减灯恒为 1，
    // 排序只看颜色）。ES 100 没有数组构造式，展开写。
    int count = int(ambientLight.w);
    int best0 = -1; int best1 = -1;
    float r0 = 0.0; float r1 = 0.0;
    for (int i = 0; i < 16; i++) {
        if (i >= count) { break; }
        vec3 c = lightColor[i].rgb;
        float rank = c.r + c.g + c.b;
        if (rank > r0) { r1 = r0; best1 = best0; r0 = rank; best0 = i; }
        else if (rank > r1) { r1 = rank; best1 = i; }
    }

    // geom_2L 里每盏灯各加一次 ambient，所以两盏灯环境项是两份。
    int lightCount = (best0 >= 0 ? 1 : 0) + (best1 >= 0 ? 1 : 0);
    vec3 lit = ambient * float(max(lightCount, 1));
    for (int i = 0; i < 16; i++) {
        if (i == best0 || i == best1) { lit += lightContrib(i, worldPos, N); }
    }

    v_color = clamp(lit, 0.0, 1.0);
    v_texcoord = texcoord * uvXform.xy + uvXform.zw;
}
