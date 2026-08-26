#version 100
precision mediump float;
uniform sampler2D texSampler;
uniform vec4 fogColor;
uniform vec4 fogParams;

varying vec2 v_texcoord;
varying float v_coverage;
varying float v_viewDepth;

void main() {
    vec4 sampled = texture2D(texSampler, v_texcoord);
    float alpha = sampled.a * clamp(v_coverage, 0.0, 1.0);
    if (alpha < 0.4) {
        discard;
    }
    vec4 outColor = vec4(sampled.rgb, 1.0);
    if (fogParams.x > 0.5) {
        float vis = clamp((fogParams.z - v_viewDepth)
            / max(fogParams.z - fogParams.y, 1.0e-4), 0.0, 1.0);
        outColor.rgb = mix(fogColor.rgb, outColor.rgb, vis);
    }
    gl_FragColor = outColor;
}
