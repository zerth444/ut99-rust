Texture2D Surface : register(t0);
Texture2D Lightmaps : register(t1);
SamplerState SurfaceSampler : register(s0);
SamplerState LightSampler : register(s1);

struct Input {
    float3 position : POSITION;
    float2 uv : TEXCOORD0;
    float2 lightUv : TEXCOORD1;
    float4 bounds : TEXCOORD2;
    float4 color : COLOR0;
};

struct Fragment {
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
    float2 lightUv : TEXCOORD1;
    nointerpolation float4 bounds : TEXCOORD2;
    float inverseDepth : TEXCOORD3;
    nointerpolation float4 color : COLOR0;
};

Fragment vertex_main(Input input) {
    Fragment output;
    output.position = float4(input.position.xy, input.position.z * 4.0, 1.0);
    output.uv = input.uv;
    output.lightUv = input.lightUv;
    output.bounds = input.bounds;
    output.inverseDepth = input.position.z;
    output.color = input.color;
    return output;
}

float4 pixel_main(Fragment input) : SV_Target {
    float depth = rcp(max(input.inverseDepth, 0.000001));
    float4 texel = Surface.Sample(SurfaceSampler, input.uv * depth);
    clip(texel.a - input.color.a);
    float3 lighting = 1.0;
    if (input.bounds.w > 0.0) {
        float2 lightUv = clamp(input.lightUv * depth, input.bounds.xy, input.bounds.zw);
        lighting = 4.0 * Lightmaps.SampleLevel(LightSampler, lightUv, 0).rgb;
    } else if (input.bounds.w == 0.0) {
        lighting = lerp(0.62, 1.0, saturate(input.inverseDepth * 680.0));
    }
    return float4(saturate(texel.rgb * input.color.rgb * lighting), 1.0);
}
