uniform sampler2D texSampler: TEXUNIT0;
uniform sampler2D texSampler2: TEXUNIT1;

float4 main(
	float2 texcoord : TEXCOORD0,
	float2 texcoord2 : TEXCOORD1
) {
	// 贴图向量是 [光照贴图, 漫反射] ⇒ texSampler 是光照贴图、texSampler2 是漫反射；
	// 主 UV（TEXCOORD0）走漫反射、次 UV（TEXCOORD1）走光照贴图。以前两个 UV 接反了。
	float4 lightMap = tex2D(texSampler, texcoord2);
	float4 color = tex2D(texSampler2, texcoord);
	if (color.a < 0.9) {
        discard;
    }

    float4 outColor = (lightMap * 1.5 + 0.15) * color;
	return outColor;
}
