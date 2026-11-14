// Control flow: a loop filling a local array, a loop with a continue over a switch reading a table, an if/else, and an
// early return. The same results as the suite's hand-assembled "dxbc flow" (whose continue is inside its switch, which
// HLSL compilers refuse).
RWByteAddressBuffer output : register(u0);

static const uint table[4] = { 10, 20, 30, 40 };

[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID)
{
    uint i = id.x;
    uint squares[4];
    // i >> 16 is always 0 here, but the compiler can't know, so the array stays an array.
    for (uint k = 0; k < 4; k++)
        squares[k] = k * k + (i >> 16);
    uint acc = 0;
    for (uint j = 0; j < (i & 7); j++)
    {
        uint m = j & 3;
        if (m == 1)
            continue;
        switch (m)
        {
        case 0:
            acc += table[m];
            break;
        default:
            acc += 1100;
            break;
        }
        acc += squares[m];
    }
    if (i & 1)
        acc <<= 1;
    else
        acc += 7;
    if (i == 63)
    {
        output.Store(i * 4, 0xdead);
        return;
    }
    output.Store(i * 4, acc);
}
