#version 100
precision mediump float;
// Fragment stage shared by the Gouraud-lit programs: texture * vertex color
// (D3D ColorOp=Modulate), tint, alpha cutout, and linear view-depth fog.
uniform sampler2D texSampler;
uniform vec4 tint;            // rgb * a premodulates, matching the Vulkan port
uniform vec4 materialMisc;    // x = alpha_ref, w = fog exempt
uniform vec4 fogColor;
uniform vec4 fogParams;       // x = enabled, y = start, z = end

varying vec2 v_texcoord;
varying vec3 v_color;
varying float v_viewDepth;

void main() {
    vec4 sampled = texture2D(texSampler, v_texcoord);
    if (sampled.a < materialMisc.x) {
        discard;
    }
    vec3 rgb = sampled.rgb * v_color * tint.rgb * tint.a;
    vec4 outColor = vec4(rgb, sampled.a * tint.a);

    if (fogParams.x > 0.5 && materialMisc.w < 0.5) {
        float vis = clamp((fogParams.z - v_viewDepth)
            / max(fogParams.z - fogParams.y, 1.0e-4), 0.0, 1.0);
        outColor.rgb = mix(fogColor.rgb * outColor.a, outColor.rgb, vis);
    }
    gl_FragColor = outColor;
}
