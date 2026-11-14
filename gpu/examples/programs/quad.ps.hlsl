// The texture at the interpolated coordinate; and the pixel's position, a derivative and w in a second target.
Texture2D tex : register(t0);
SamplerState smp : register(s0);

struct Out
{
    float4 color : SV_Target0;
    float4 info : SV_Target1;
};

Out main(float4 position : SV_Position, float2 uv : TEXCOORD0)
{
    Out o;
    o.color = tex.Sample(smp, uv);
    o.info = float4(position.xy, ddx(uv.x), position.w);
    return o;
}
