// A cube's vertices: a position and texture coordinate, through a matrix the frames change.
cbuffer Frame : register(b0)
{
    row_major float4x4 mvp;
};

struct Out
{
    float4 position : SV_Position;
    float2 uv : TEXCOORD0;
};

Out main(float3 position : POSITION, float2 uv : TEXCOORD0)
{
    Out o;
    o.position = mul(float4(position, 1.0), mvp);
    o.uv = uv;
    return o;
}
