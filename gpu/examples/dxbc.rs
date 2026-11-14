//! The suite's DXBC tests: programs assembled with `dxbc_asm`, and the same programs (or close) in HLSL compiled by
//! vkd3d's compiler (`programs/*.hlsl`, by `programs/build.sh`), read by `gpu::dxbc` and run on the device, their
//! results computed here.

use gpu::ir::Stage;
use gpu::{
    Address, Attribute, Bind, ColorAttachment, ColorTarget, Command, Cull, Device, Draw, Filter, Format, FrontFace,
    GraphicsDesc, ImageDesc, Load, Rect, SamplerDesc, Sub, Topology, VertexBuffer, VertexFormat,
};

#[path = "dxbc_asm.rs"]
mod asm;

use asm::*;

use super::{Ran, compare, download, e, passed, run_now, upload, values, zeroed};

fn read(bytes: &[u8]) -> Result<gpu::dxbc::Shader, String> {
    gpu::dxbc::read(bytes).map_err(|e| e.message)
}

fn compiled(name: &str) -> &'static [u8] {
    super::DXBC.iter().find(|(n, _)| *n == name).map_or(&[], |(_, b)| *b)
}

/// The arith test's inputs: floats with fractions, halves that round to even, and two past `i32`'s range.
fn arith_inputs(n: usize) -> (Vec<f32>, Vec<u32>) {
    let bs = values(n, 11, u32::MAX);
    let fs = values(n, 12, 2000)
        .iter()
        .enumerate()
        .map(|(i, v)| match i % 16 {
            0 => 3.0e9,
            1 => -3.0e9,
            2 => 2.5,
            3 => -2.5,
            _ => (*v as f32 - 1000.0) / 8.0,
        })
        .collect();
    (fs, bs)
}

/// The arith test's floats and words, and what the program wrote.
type Arith = (Vec<f32>, Vec<u32>, Vec<u32>);

/// Runs a compute program over the arith test's inputs, `words` results a thread.
fn run_arith(d: &mut Device, bytes: &[u8], words: usize) -> Result<Arith, String> {
    let shader = read(bytes)?;
    let p = d.pipeline(&shader.entry).map_err(e)?;
    let n = 256usize;
    let (fs, bs) = arith_inputs(n);
    let input: Vec<u32> = (0..n).flat_map(|i| [fs[i].to_bits(), bs[i]]).collect();
    let src = upload(d, &input)?;
    let dst = zeroed(d, n * words)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![src, dst], groups: [(n / 64) as u32, 1, 1] }])?;
    Ok((fs, bs, download(d, dst)?))
}

/// `programs/arith.hlsl`: 25 words a thread.
pub fn hlsl_arith(d: &mut Device) -> Result<Ran, String> {
    passed(hlsl_arith_(d))
}

fn hlsl_arith_(d: &mut Device) -> Result<(), String> {
    let (fs, bs, got) = run_arith(d, compiled("arith"), 25)?;
    let fb = f32::to_bits;
    let mut want = Vec::new();
    for (i, (&x, &b)) in fs.iter().zip(&bs).enumerate() {
        let t = i as u32;
        let s = (t & 15) as f32 - 7.5;
        want.extend([
            fb(x + 1.5),
            fb(x * -x),
            fb(x * 2.0 + 0.25),
            fb(x.abs().max(3.0)),
            fb(x - x.floor()),
            fb(x.round_ties_even()),
            fb(x.floor()),
            fb(x.trunc()),
            x as i32 as u32,
            fb(x.clamp(0.0, 1.0)),
            fb(-x.abs()),
            fb(x / 4.0),
            b.wrapping_sub(t),
            b.wrapping_mul(0x10001),
            b.checked_div(t).unwrap_or(0),
            b.checked_rem(t).unwrap_or(0),
            ((b as i32) >> (t & 31)) as u32,
            b >> 4,
            b.count_ones(),
            (b as i32).min(5) as u32,
            b.max(5) ^ 0xff00_ff00,
            if x < 0.0 { u32::MAX } else { 0 },
            if t < 32 { fb(x) } else { fb(7.0) },
            fb(7.5 * s),
            b.wrapping_mul(3).wrapping_add(t),
        ]);
    }
    compare("results", &got, &want)
}

/// `programs/flow.hlsl`: the results of "dxbc flow".
pub fn hlsl_flow(d: &mut Device) -> Result<Ran, String> {
    passed(run_flow(d, compiled("flow")))
}

/// `programs/shared.hlsl`: the results of "dxbc shared".
pub fn hlsl_shared(d: &mut Device) -> Result<Ran, String> {
    passed(run_shared(d, compiled("shared")))
}

/// `programs/quad.vs.hlsl` and `quad.ps.hlsl`: the results of "dxbc draw".
pub fn hlsl_draw(d: &mut Device) -> Result<Ran, String> {
    run_draw(d, compiled("quad.vs"), compiled("quad.ps"))
}

const TID: fn(&str) -> Opnd = |s| src(THREAD_ID, &[], s);

/// Arithmetic, conversions, integer rules and comparisons: each thread's float `a` and word `b`, and 30 results.
pub fn arith(d: &mut Device) -> Result<Ran, String> {
    passed(arith_(d))
}

fn arith_(d: &mut Device) -> Result<(), String> {
    let mut a = Asm::new(5);
    a.op(op::DCL_GLOBAL_FLAGS, 1 << 11, &[], &[])
        .op(op::DCL_RESOURCE_RAW, 0, &[slot(RESOURCE, 0)], &[])
        .op(op::DCL_UAV_RAW, 0, &[slot(UAV, 0)], &[])
        .op(op::DCL_INPUT, 0, &[dst(THREAD_ID, &[], "x")], &[])
        .op(op::DCL_TEMPS, 0, &[], &[4])
        .op(op::DCL_THREAD_GROUP, 0, &[], &[64, 1, 1]);
    let out = |a: &mut Asm, at: u32, v: Opnd, mask: &str| {
        a.i(op::IADD, &[r(0, "z"), rs(0, "y"), l1(at * 4)]);
        a.i(op::STORE_RAW, &[dst(UAV, &[0], mask), rs(0, "z"), v]);
    };
    a.i(op::ISHL, &[r(0, "x"), TID("x"), l1(3)])
        .i(op::LD_RAW, &[r(1, "xy"), rs(0, "x"), src(RESOURCE, &[0], "xyxx")])
        .i(op::IMUL, &[null(), r(0, "y"), TID("x"), l1(128)])
        .i(op::ADD, &[r(2, "x"), rs(1, "x"), f1(1.5)])
        .i(op::MUL, &[r(2, "y"), rs(1, "x"), neg(rs(1, "x"))])
        .i(op::MAD, &[r(2, "z"), rs(1, "x"), f1(2.0), f1(0.25)])
        .i(op::MAX, &[r(2, "w"), abs(rs(1, "x")), f1(3.0)]);
    out(&mut a, 0, rs(2, "xyzw"), "xyzw");
    a.i(op::FRC, &[r(3, "x"), rs(1, "x")])
        .i(op::ROUND_NE, &[r(3, "y"), rs(1, "x")])
        .i(op::ROUND_NI, &[r(3, "z"), rs(1, "x")])
        .i(op::ROUND_Z, &[r(3, "w"), rs(1, "x")]);
    out(&mut a, 4, rs(3, "xyzw"), "xyzw");
    a.i(op::FTOI, &[r(2, "x"), rs(1, "x")])
        .i(op::FTOU, &[r(2, "y"), rs(1, "x")])
        .i(op::UTOF, &[r(2, "z"), rs(1, "y")])
        .op(op::MOV, SAT, &[r(2, "w"), rs(1, "x")], &[]);
    out(&mut a, 8, rs(2, "xyzw"), "xyzw");
    a.i(op::IADD, &[r(2, "x"), rs(1, "y"), neg(TID("x"))])
        .i(op::IMUL, &[r(2, "y"), r(2, "z"), rs(1, "y"), l1(0x10001)])
        .i(op::UDIV, &[r(2, "w"), r(3, "x"), rs(1, "y"), TID("x")]);
    out(&mut a, 12, rs(2, "xyzw"), "xyzw");
    a.i(op::ISHL, &[r(3, "y"), rs(1, "y"), l1(33)])
        .i(op::ISHR, &[r(3, "z"), rs(1, "y"), TID("x")])
        .i(op::USHR, &[r(3, "w"), rs(1, "y"), l1(4)]);
    out(&mut a, 16, rs(3, "xyzw"), "xyzw");
    a.i(op::COUNTBITS, &[r(2, "x"), rs(1, "y")])
        .i(op::IMIN, &[r(2, "y"), rs(1, "y"), l1(5)])
        .i(op::UMAX, &[r(2, "z"), rs(1, "y"), l1(5)])
        .i(op::XOR, &[r(2, "w"), rs(1, "y"), l1(0xff00_ff00)]);
    out(&mut a, 20, rs(2, "xyzw"), "xyzw");
    a.i(op::LT, &[r(2, "x"), rs(1, "x"), f1(0.0)])
        .i(op::IGE, &[r(2, "y"), rs(1, "y"), l1(0)])
        .i(op::ULT, &[r(2, "z"), TID("x"), l1(32)])
        .i(op::MOVC, &[r(2, "w"), rs(2, "z"), rs(1, "x"), f1(7.0)]);
    out(&mut a, 24, rs(2, "xyzw"), "xyzw");
    // The dot product's weights are powers of two, so its products are exact and fusing can't change it.
    a.i(op::DP4, &[r(3, "x"), rs(1, "xxxx"), f4([1.0, 2.0, 4.0, 8.0])])
        .i(op::ROUND_PI, &[r(3, "y"), rs(1, "x")]);
    out(&mut a, 28, rs(3, "xyxx"), "xy");
    a.i(op::RET, &[]);
    let (fs, bs, got) = run_arith(d, &a.finish(&[], &[]), 32)?;
    let n = fs.len();
    let fb = f32::to_bits;
    let mask = |c: bool| if c { u32::MAX } else { 0 };
    let mut want = Vec::new();
    for i in 0..n {
        let (x, b, t) = (fs[i], bs[i], i as u32);
        let wide = (b as i32 as i64) * 0x10001;
        let row = [
            fb(x + 1.5),
            fb(x * -x),
            fb(x * 2.0 + 0.25),
            fb(x.abs().max(3.0)),
            fb(x - x.floor()),
            fb(x.round_ties_even()),
            fb(x.floor()),
            fb(x.trunc()),
            x as i32 as u32,
            x as u32,
            fb(b as f32),
            fb(x.clamp(0.0, 1.0)),
            b.wrapping_sub(t),
            (wide >> 32) as u32,
            wide as u32,
            b.checked_div(t).unwrap_or(u32::MAX),
            b.checked_rem(t).unwrap_or(u32::MAX),
            b << 1,
            ((b as i32) >> (t & 31)) as u32,
            b >> 4,
            b.count_ones(),
            (b as i32).min(5) as u32,
            b.max(5),
            b ^ 0xff00_ff00,
            mask(x < 0.0),
            mask(b as i32 >= 0),
            mask(t < 32),
            if t < 32 { fb(x) } else { fb(7.0) },
            fb(x + x * 2.0 + x * 4.0 + x * 8.0),
            fb(x.ceil()),
            0,
            0,
        ];
        want.extend(row);
    }
    compare("results", &got, &want)
}

/// Control flow: a loop filling an indexable temp, a loop over a switch with a `continue`, fall-through into the
/// default and the immediate constant buffer, an if/else, and a conditional early return.
pub fn flow(d: &mut Device) -> Result<Ran, String> {
    passed(flow_(d))
}

fn flow_(d: &mut Device) -> Result<(), String> {
    let mut a = Asm::new(5);
    a.icb(&[10, 0, 0, 0, 20, 0, 0, 0, 30, 0, 0, 0, 40, 0, 0, 0])
        .op(op::DCL_UAV_RAW, 0, &[slot(UAV, 0)], &[])
        .op(op::DCL_INPUT, 0, &[dst(THREAD_ID, &[], "x")], &[])
        .op(op::DCL_TEMPS, 0, &[], &[3])
        .op(op::DCL_INDEXABLE_TEMP, 0, &[], &[0, 4, 4])
        .op(op::DCL_THREAD_GROUP, 0, &[], &[64, 1, 1]);
    let x0 = |rel: Opnd| with_relative(dst(XTEMP, &[0, 0], "x"), 1, rel);
    let x0s = |rel: Opnd| with_relative(src(XTEMP, &[0, 0], "xxxx"), 1, rel);
    let cond = |n: u32, c: char| one(TEMP, &[n], c);
    // x0[k].x = k * k, k from 0 to 3.
    a.i(op::MOV, &[r(0, "x"), l1(0)])
        .i(op::LOOP, &[])
        .i(op::UGE, &[r(0, "y"), rs(0, "x"), l1(4)])
        .op(op::BREAKC, NONZERO, &[cond(0, 'y')], &[])
        .i(op::IMUL, &[null(), r(1, "x"), rs(0, "x"), rs(0, "x")])
        .i(op::MOV, &[x0(one(TEMP, &[0], 'x')), rs(1, "x")])
        .i(op::IADD, &[r(0, "x"), rs(0, "x"), l1(1)])
        .i(op::ENDLOOP, &[]);
    // For j below i & 7, by j & 3: 0 adds icb[0] (10); 1 continues; 2 and 3 add 100 and fall into the default's
    // 1000; then (unless continued) x0[j & 3] is added.
    a.i(op::MOV, &[r(0, "x"), l1(0)])
        .i(op::MOV, &[r(0, "z"), l1(0)])
        .i(op::AND, &[r(0, "w"), TID("x"), l1(7)])
        .i(op::LOOP, &[])
        .i(op::UGE, &[r(1, "x"), rs(0, "x"), rs(0, "w")])
        .op(op::BREAKC, NONZERO, &[cond(1, 'x')], &[])
        .i(op::AND, &[r(1, "y"), rs(0, "x"), l1(3)])
        .i(op::IADD, &[r(0, "x"), rs(0, "x"), l1(1)])
        .i(op::SWITCH, &[cond(1, 'y')])
        .i(op::CASE, &[l1(0)])
        .i(op::IADD, &[r(0, "z"), rs(0, "z"), with_relative(src(ICB, &[0], "xxxx"), 0, one(TEMP, &[1], 'y'))])
        .i(op::BREAK, &[])
        .i(op::CASE, &[l1(1)])
        .i(op::CONTINUE, &[])
        .i(op::CASE, &[l1(2)])
        .i(op::CASE, &[l1(3)])
        .i(op::IADD, &[r(0, "z"), rs(0, "z"), l1(100)])
        .i(op::DEFAULT, &[])
        .i(op::IADD, &[r(0, "z"), rs(0, "z"), l1(1000)])
        .i(op::BREAK, &[])
        .i(op::ENDSWITCH, &[])
        .i(op::IADD, &[r(0, "z"), rs(0, "z"), x0s(one(TEMP, &[1], 'y'))])
        .i(op::ENDLOOP, &[]);
    a.i(op::AND, &[r(1, "x"), TID("x"), l1(1)])
        .op(op::IF, NONZERO, &[cond(1, 'x')], &[])
        .i(op::ISHL, &[r(0, "z"), rs(0, "z"), l1(1)])
        .i(op::ELSE, &[])
        .i(op::IADD, &[r(0, "z"), rs(0, "z"), l1(7)])
        .i(op::ENDIF, &[])
        .i(op::ISHL, &[r(2, "x"), TID("x"), l1(2)])
        .i(op::IEQ, &[r(1, "z"), TID("x"), l1(63)])
        .op(op::IF, NONZERO, &[cond(1, 'z')], &[])
        .i(op::STORE_RAW, &[dst(UAV, &[0], "x"), rs(2, "x"), l1(0xdead)])
        .i(op::RET, &[])
        .i(op::ENDIF, &[])
        .i(op::STORE_RAW, &[dst(UAV, &[0], "x"), rs(2, "x"), rs(0, "z")])
        .i(op::RET, &[]);
    run_flow(d, &a.finish(&[], &[]))
}

fn run_flow(d: &mut Device, bytes: &[u8]) -> Result<(), String> {
    let shader = read(bytes)?;
    let p = d.pipeline(&shader.entry).map_err(e)?;
    let out = zeroed(d, 128)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![out], groups: [2, 1, 1] }])?;
    let want: Vec<u32> = (0..128u32)
        .map(|i| {
            if i == 63 {
                return 0xdead;
            }
            let mut acc = 0u32;
            for j in 0..(i & 7) {
                let k = j & 3;
                match k {
                    0 => acc += 10,
                    1 => continue,
                    _ => acc += 1100,
                }
                acc += k * k;
            }
            if i & 1 != 0 { acc << 1 } else { acc + 7 }
        })
        .collect();
    compare("results", &download(d, out)?, &want)
}

/// Group-shared memory and barriers, structured buffers, and atomics: each thread writes `a + b` to shared memory,
/// waits for its group, and stores its mirror's; every `a` is summed and every `b`'s maximum kept atomically.
pub fn shared(d: &mut Device) -> Result<Ran, String> {
    passed(shared_(d))
}

fn shared_(d: &mut Device) -> Result<(), String> {
    let mut a = Asm::new(5);
    let lid = |s: &str| src(THREAD_ID_FLAT, &[], s);
    a.op(op::DCL_RESOURCE_STRUCTURED, 0, &[slot(RESOURCE, 0)], &[8])
        .op(op::DCL_UAV_STRUCTURED, 0, &[slot(UAV, 0)], &[4])
        .op(op::DCL_UAV_RAW, 0, &[slot(UAV, 1)], &[])
        .op(op::DCL_TGSM_RAW, 0, &[slot(TGSM, 0)], &[256])
        .op(op::DCL_INPUT, 0, &[dst(THREAD_ID_FLAT, &[], "x")], &[])
        .op(op::DCL_INPUT, 0, &[dst(THREAD_ID, &[], "x")], &[])
        .op(op::DCL_TEMPS, 0, &[], &[3])
        .op(op::DCL_THREAD_GROUP, 0, &[], &[64, 1, 1])
        .i(op::LD_STRUCTURED, &[r(0, "xy"), TID("x"), l1(0), src(RESOURCE, &[0], "xyxx")])
        .i(op::IADD, &[r(0, "z"), rs(0, "x"), rs(0, "y")])
        .i(op::ISHL, &[r(1, "x"), lid("x"), l1(2)])
        .i(op::STORE_RAW, &[dst(TGSM, &[0], "x"), rs(1, "x"), rs(0, "z")])
        .op(op::SYNC, 3 << 11, &[], &[])
        .i(op::IADD, &[r(1, "y"), neg(lid("x")), l1(63)])
        .i(op::ISHL, &[r(1, "y"), rs(1, "y"), l1(2)])
        .i(op::LD_RAW, &[r(1, "z"), rs(1, "y"), src(TGSM, &[0], "xxxx")])
        .i(op::IMM_ATOMIC_IADD, &[r(2, "x"), dst(UAV, &[1], "x"), l1(0), rs(0, "x")])
        .i(op::ATOMIC_UMAX, &[dst(UAV, &[1], "x"), l1(4), rs(0, "y")])
        .i(op::STORE_STRUCTURED, &[dst(UAV, &[0], "x"), TID("x"), l1(0), rs(1, "z")])
        .i(op::RET, &[]);
    run_shared(d, &a.finish(&[], &[]))
}

fn run_shared(d: &mut Device, bytes: &[u8]) -> Result<(), String> {
    let shader = read(bytes)?;
    let p = d.pipeline(&shader.entry).map_err(e)?;
    let n = 128;
    let ab = values(2 * n, 13, 1 << 20);
    let input = upload(d, &ab)?;
    let out = zeroed(d, n)?;
    let counters = zeroed(d, 2)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![input, out, counters], groups: [2, 1, 1] }])?;
    let want: Vec<u32> = (0..n).map(|i| { let m = (i / 64) * 64 + 63 - i % 64; ab[2 * m] + ab[2 * m + 1] }).collect();
    compare("mirrored", &download(d, out)?, &want)?;
    let sum = (0..n).fold(0u32, |s, i| s.wrapping_add(ab[2 * i]));
    let max = (0..n).map(|i| ab[2 * i + 1]).max().unwrap_or(0);
    compare("sum and maximum", &download(d, counters)?, &[sum, max])
}

/// A textured quad from a DXBC vertex shader (a constant buffer's scale and offset) and pixel shader (a sample, the
/// pixel's position and a derivative): the texture test's 4×4 texture over an 8×8 target.
pub fn draw(d: &mut Device) -> Result<Ran, String> {
    let float = 3;
    let mut vs = Asm::new(1);
    vs.op(op::DCL_CONSTANT_BUFFER, 0, &[src(CB, &[0, 1], "xyzw")], &[])
        .op(op::DCL_INPUT, 0, &[dst(INPUT, &[0], "xy")], &[])
        .op(op::DCL_INPUT, 0, &[dst(INPUT, &[1], "xy")], &[])
        .op(op::DCL_OUTPUT_SIV, 0, &[dst(OUTPUT, &[0], "xyzw")], &[1])
        .op(op::DCL_OUTPUT, 0, &[dst(OUTPUT, &[1], "xy")], &[])
        .i(op::MAD, &[dst(OUTPUT, &[0], "xy"), src(INPUT, &[0], "xyxx"), src(CB, &[0, 0], "xyxx"), src(CB, &[0, 0], "zwzz")])
        .i(op::MOV, &[dst(OUTPUT, &[0], "zw"), f4([0.0, 0.0, 0.0, 1.0])])
        .i(op::MOV, &[dst(OUTPUT, &[1], "xy"), src(INPUT, &[1], "xyxx")])
        .i(op::RET, &[]);
    let vs = vs.finish(
        &[
            Sig { name: "POSITION", index: 0, system: 0, component: float, register: 0, mask: 3 },
            Sig { name: "TEXCOORD", index: 0, system: 0, component: float, register: 1, mask: 3 },
        ],
        &[
            Sig { name: "SV_Position", index: 0, system: 1, component: float, register: 0, mask: 15 },
            Sig { name: "TEXCOORD", index: 0, system: 0, component: float, register: 1, mask: 3 },
        ],
    );
    let mut ps = Asm::new(0);
    ps.op(op::DCL_SAMPLER, 0, &[slot(SAMPLER, 0)], &[])
        .op(op::DCL_RESOURCE, 3 << 11, &[slot(RESOURCE, 0)], &[0x5555])
        .op(op::DCL_INPUT_PS_SIV, 4 << 11, &[dst(INPUT, &[0], "xyzw")], &[1])
        .op(op::DCL_INPUT_PS, 2 << 11, &[dst(INPUT, &[1], "xy")], &[])
        .op(op::DCL_OUTPUT, 0, &[dst(OUTPUT, &[0], "xyzw")], &[])
        .op(op::DCL_OUTPUT, 0, &[dst(OUTPUT, &[1], "xyzw")], &[])
        .i(op::SAMPLE, &[dst(OUTPUT, &[0], "xyzw"), src(INPUT, &[1], "xyxx"), src(RESOURCE, &[0], "xyzw"), slot(SAMPLER, 0)])
        .i(op::MOV, &[dst(OUTPUT, &[1], "xy"), src(INPUT, &[0], "xyxx")])
        .i(op::DERIV_RTX, &[dst(OUTPUT, &[1], "z"), src(INPUT, &[1], "xxxx")])
        .i(op::MOV, &[dst(OUTPUT, &[1], "w"), src(INPUT, &[0], "wwww")])
        .i(op::RET, &[]);
    let ps = ps.finish(
        &[
            Sig { name: "SV_Position", index: 0, system: 1, component: float, register: 0, mask: 15 },
            Sig { name: "TEXCOORD", index: 0, system: 0, component: float, register: 1, mask: 3 },
        ],
        &[
            Sig { name: "SV_Target", index: 0, system: 64, component: float, register: 0, mask: 15 },
            Sig { name: "SV_Target", index: 1, system: 64, component: float, register: 1, mask: 15 },
        ],
    );
    run_draw(d, &vs, &ps)
}

fn run_draw(d: &mut Device, vs: &[u8], ps: &[u8]) -> Result<Ran, String> {
    const N: u32 = 8;
    let target = match d.image(ImageDesc::new(Format::Rgba8Unorm, N, N)) {
        Ok(i) => i,
        Err(why) if why.message.contains("aren't on") => return Ok(Ran::Unsupported(why.message)),
        Err(why) => return Err(why.message),
    };
    let (vs, ps) = (read(vs)?, read(ps)?);
    if vs.entry.stage != Stage::Vertex || ps.entry.stage != Stage::Fragment {
        return Err("the stages read wrong".into());
    }
    let info = d.image(ImageDesc::new(Format::Rgba32Float, N, N)).map_err(e)?;
    let tex = d.image(ImageDesc::new(Format::Rgba8Unorm, 4, 4)).map_err(e)?;
    let texels: Vec<u32> = (0..16u32).map(|i| u32::from_le_bytes([(i * 16) as u8, (255 - i * 16) as u8, (i * 5) as u8, 255])).collect();
    let staging = upload(d, &texels)?;
    run_now(d, &[Command::CopyBufferToImage { from: staging, row_pixels: 0, to: tex, sub: Sub::default(), region: Rect { x: 0, y: 0, width: 4, height: 4 } }])?;
    let sampler = d.sampler(SamplerDesc::new(Filter::Nearest, Filter::Nearest, [Address::ClampToEdge; 2])).map_err(e)?;
    let buffers = [VertexBuffer { stride: 16, per_instance: false }];
    let attributes = [
        Attribute { location: 0, buffer: 0, offset: 0, format: VertexFormat::F32(2) },
        Attribute { location: 1, buffer: 0, offset: 8, format: VertexFormat::F32(2) },
    ];
    let colors = [
        ColorTarget { format: Format::Rgba8Unorm, blend: None, write: [true; 4] },
        ColorTarget { format: Format::Rgba32Float, blend: None, write: [true; 4] },
    ];
    let p = d
        .graphics(&GraphicsDesc {
            vertex: &vs.entry,
            fragment: &ps.entry,
            buffers: &buffers,
            attributes: &attributes,
            topology: Topology::TriangleStrip,
            cull: Cull::None,
            front: FrontFace::CounterClockwise,
            depth: None,
            depth_format: None,
            colors: &colors,
        })
        .map_err(e)?;
    // Corners from 0 to 1, scaled by 2 and moved by -1 to cover the target.
    let v: Vec<u32> = [0.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0].map(f32::to_bits).to_vec();
    let vb = upload(d, &v)?;
    let cb = upload(d, &[2.0f32, 2.0, -1.0, -1.0].map(f32::to_bits))?;
    let at = |image| ColorAttachment { image, sub: Sub::default(), load: Load::DontCare };
    run_now(
        d,
        &[
            Command::BeginRender { colors: vec![at(target), at(info)], depth: None },
            Command::Draw(Draw {
                pipeline: p,
                bindings: vec![(0, 0, Bind::Buffer(cb)), (1, 0, Bind::Image(tex)), (2, 0, Bind::Sampler(sampler))],
                vertex_buffers: vec![vb],
                index: None,
                first: 0,
                count: 4,
                vertex_offset: 0,
                first_instance: 0,
                instances: 1,
            }),
            Command::EndRender,
        ],
    )?;
    let read_back = |d: &mut Device, image, words: usize| -> Result<Vec<u32>, String> {
        let out = zeroed(d, words)?;
        run_now(d, &[Command::CopyImageToBuffer { from: image, sub: Sub::default(), region: Rect { x: 0, y: 0, width: N, height: N }, to: out, row_pixels: 0 }])?;
        download(d, out)
    };
    let got = read_back(d, target, (N * N) as usize)?;
    let infos = read_back(d, info, (N * N * 4) as usize)?;
    for y in 0..N {
        for x in 0..N {
            let at = (y * N + x) as usize;
            let want = texels[((y / 2) * 4 + x / 2) as usize];
            if got[at] != want {
                return Err(format!("({x}, {y}) is {:#010x}, wanted texel ({}, {}) {want:#010x}", got[at], x / 2, y / 2));
            }
            let i = [0, 1, 2, 3].map(|c| f32::from_bits(infos[at * 4 + c]));
            let want = [x as f32 + 0.5, y as f32 + 0.5, 1.0 / 8.0, 1.0];
            if i != want {
                return Err(format!("({x}, {y})'s position and derivative {i:?}, wanted {want:?}"));
            }
        }
    }
    Ok(Ran::Passed)
}
