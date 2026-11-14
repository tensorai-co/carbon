// Group-shared memory and a barrier, structured buffers and atomics. The same results as "dxbc shared".
struct Pair
{
    uint a;
    uint b;
};

StructuredBuffer<Pair> pairs : register(t0);
RWStructuredBuffer<uint> mirrored : register(u0);
RWStructuredBuffer<uint> counters : register(u1);
groupshared uint sums[64];

[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID, uint lid : SV_GroupIndex)
{
    Pair p = pairs[id.x];
    sums[lid] = p.a + p.b;
    GroupMemoryBarrierWithGroupSync();
    uint old;
    InterlockedAdd(counters[0], p.a, old);
    InterlockedMax(counters[1], p.b);
    mirrored[id.x] = sums[63 - lid];
}
