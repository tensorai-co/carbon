// The cube's texture.
Texture2D tex : register(t0);
SamplerState smp : register(s0);

float4 main(float4 position : SV_Position, float2 uv : TEXCOORD0) : SV_Target
{
    return tex.Sample(smp, uv);
}
