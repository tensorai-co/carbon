// A quad's corners from 0 to 1, scaled and moved by a constant buffer, and its texture coordinates.
cbuffer Transform : register(b0)
{
    float4 scale_offset;
};

struct Out
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
};

Out main(float2 position : POSITION, float2 uv : TEXCOORD0)
{
    Out o;
    o.position = float4(position * scale_offset.xy + scale_offset.zw, 0.0, 1.0);
    o.uv = uv;
    return o;
}
