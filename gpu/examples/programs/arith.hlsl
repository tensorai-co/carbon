// Arithmetic, conversions, integer rules and comparisons as HLSL has them: each thread's float a and word b in, 25
// words out. Casts from words to floats and two conversions in one vector are left out: vkd3d 2.1 compiles
// asuint((float)b) to b itself, and takes (uint)a to be (int)a's ftoi when both are in a program.
ByteAddressBuffer input : register(t0);
RWByteAddressBuffer output : register(u0);

[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID)
{
    uint i = id.x;
    float a = asfloat(input.Load(i * 8));
    uint b = input.Load(i * 8 + 4);
    uint at = i * 100;
    // Small halves, so a dot product's terms are exact in any order.
    float s = (float)(i & 15) - 7.5;
    output.Store4(at, asuint(float4(a + 1.5, a * -a, a * 2.0 + 0.25, max(abs(a), 3.0))));
    output.Store4(at + 16, asuint(float4(frac(a), round(a), floor(a), trunc(a))));
    output.Store4(at + 32, uint4(asuint((int)a), asuint(saturate(a)), asuint(-abs(a)), asuint(a / 4.0)));
    output.Store4(at + 48, uint4(b - i, b * 0x10001u, i == 0 ? 0u : b / i, i == 0 ? 0u : b % i));
    output.Store4(at + 64, uint4(asuint(asint(b) >> (i & 31)), b >> 4, countbits(b), asuint(min(asint(b), 5))));
    output.Store4(at + 80, uint4(max(b, 5u) ^ 0xff00ff00u, a < 0.0 ? 0xffffffffu : 0u, asuint(i < 32 ? a : 7.0),
        asuint(dot(float4(s, 2.0, s, 1.0), float4(4.0, s, 0.5, s)))));
    output.Store(at + 96, b * 3u + i);
}
