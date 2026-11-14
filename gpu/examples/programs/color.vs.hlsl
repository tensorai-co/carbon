// Coloured vertices, tinted and moved by a constant buffer the draws change.
cbuffer Draw : register(b0)
{
    float4 tint;
    float4 offset;
};

struct Out
{
    float4 position : SV_Position;
    float4 color : COLOR0;
};

Out main(float3 position : POSITION, float4 color : COLOR0)
{
    Out o;
    o.position = float4(position + offset.xyz, 1.0);
    o.color = color * tint;
    return o;
}
