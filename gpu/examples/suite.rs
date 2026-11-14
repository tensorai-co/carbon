//! gpu's conformance suite: small programs whose results are known exactly, run on a device and checked word for
//! word. Every backend must pass all of it, with the same results. Shared by `examples/conformance.rs` (every device
//! this host has) and `tests/` (in `cargo test`).
//!
//! The programs are GLSL in `programs/`, compiled to SPIR-V by `programs/build.sh`; the `.spv` files are checked in.

use std::time::Duration;

use gpu::{Command, Device, Range, Submit};

/// One test: it passes (or says the device doesn't do what it tests yet), or says what was wrong.
type Test = fn(&mut Device) -> Result<Ran, String>;

/// How a test that didn't fail went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    Passed,
    /// The device doesn't do this yet, and says so.
    Unsupported(String),
}

pub struct Outcome {
    pub name: &'static str,
    pub result: Result<Ran, String>,
}

pub const PROGRAMS: &[(&str, &[u8])] = &[
    ("copy", include_bytes!("programs/copy.spv")),
    ("reduce", include_bytes!("programs/reduce.spv")),
    ("atomics", include_bytes!("programs/atomics.spv")),
    ("subgroup", include_bytes!("programs/subgroup.spv")),
    ("stages", include_bytes!("programs/stages.spv")),
    ("branches", include_bytes!("programs/branches.spv")),
];

/// The draw tests' shaders.
pub const SHADERS: &[(&str, &[u8])] = &[
    ("colors.vert", include_bytes!("programs/colors.vert.spv")),
    ("colors.frag", include_bytes!("programs/colors.frag.spv")),
    ("textured.vert", include_bytes!("programs/textured.vert.spv")),
    ("textured.frag", include_bytes!("programs/textured.frag.spv")),
    ("cube.vert", include_bytes!("programs/cube.vert.spv")),
    ("cube.frag", include_bytes!("programs/cube.frag.spv")),
];

#[path = "draws.rs"]
mod draws;

const TIMEOUT: Duration = Duration::from_secs(20);

/// Every test, in order.
pub fn run(device: &mut Device) -> Vec<Outcome> {
    let tests: &[(&'static str, Test)] = &[
        ("copy", copy),
        ("reduce", reduce),
        ("atomics", atomics),
        ("subgroup", subgroup),
        ("fences", fences),
        ("branches", branches),
        ("rules", rules),
        ("more rules", more_rules),
        ("triangles", draws::triangles),
        ("depth", draws::depth),
        ("texture", draws::texture),
        ("cube", draws::cube),
    ];
    tests.iter().map(|(name, test)| Outcome { name, result: test(device) }).collect()
}

/// A compute test: it ran, or says what was wrong.
fn passed(r: Result<(), String>) -> Result<Ran, String> {
    r.map(|()| Ran::Passed)
}

fn program(name: &str) -> &'static [u8] {
    PROGRAMS.iter().find(|(n, _)| *n == name).map(|(_, b)| *b).unwrap_or(&[])
}

fn e(e: gpu::Error) -> String {
    e.message
}

/// Deterministic values: the same on every run and every host.
fn values(n: usize, seed: u32, modulo: u32) -> Vec<u32> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 8) % modulo
        })
        .collect()
}

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_le_bytes()).collect()
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn upload(d: &mut Device, words: &[u32]) -> Result<Range, String> {
    let size = (words.len() * 4) as u64;
    let memory = d.alloc(size).map_err(e)?;
    d.write(memory, 0, &bytes(words)).map_err(e)?;
    Ok(Range { memory, offset: 0, size })
}

fn zeroed(d: &mut Device, count: usize) -> Result<Range, String> {
    upload(d, &vec![0; count])
}

fn download(d: &mut Device, r: Range) -> Result<Vec<u32>, String> {
    let mut out = vec![0u8; r.size as usize];
    d.read(r.memory, r.offset, &mut out).map_err(e)?;
    Ok(words(&out))
}

/// Submits commands and waits for them to finish.
fn run_now(d: &mut Device, commands: &[Command]) -> Result<(), String> {
    let fence = d.fence().map_err(e)?;
    d.submit(Submit { commands, wait: &[], signal: &[(fence, 1)] }).map_err(e)?;
    if !d.wait(fence, 1, TIMEOUT).map_err(e)? {
        return Err("the work didn't finish in time".into());
    }
    Ok(())
}

fn compare(what: &str, got: &[u32], want: &[u32]) -> Result<(), String> {
    if got.len() != want.len() {
        return Err(format!("{what}: {} words, wanted {}", got.len(), want.len()));
    }
    let wrong: Vec<usize> = (0..got.len()).filter(|&i| got[i] != want[i]).collect();
    match wrong.first() {
        None => Ok(()),
        Some(&i) => Err(format!(
            "{what}: {} of {} words wrong, first at {i}: {:#x}, wanted {:#x}",
            wrong.len(),
            got.len(),
            got[i],
            want[i]
        )),
    }
}

fn copy(d: &mut Device) -> Result<Ran, String> {
    passed(copy_(d))
}

fn copy_(d: &mut Device) -> Result<(), String> {
    let n = 1024;
    let src = values(n, 1, u32::MAX);
    let p = d.pipeline_spirv(program("copy"), "main").map_err(e)?;
    let a = upload(d, &src)?;
    let b = zeroed(d, 2 * n)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![a, b], groups: [(n / 64) as u32, 1, 1] }])?;
    let want: Vec<u32> = (0..n).flat_map(|i| [src[i].wrapping_mul(2), i as u32]).collect();
    compare("dst", &download(d, b)?, &want)
}

fn reduce(d: &mut Device) -> Result<Ran, String> {
    passed(reduce_(d))
}

fn reduce_(d: &mut Device) -> Result<(), String> {
    let groups = 32;
    let src = values(128 * groups, 2, 1 << 20);
    let p = d.pipeline_spirv(program("reduce"), "main").map_err(e)?;
    let a = upload(d, &src)?;
    let sums = zeroed(d, groups)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![a, sums], groups: [groups as u32, 1, 1] }])?;
    let want: Vec<u32> = src.chunks(128).map(|c| c.iter().fold(0u32, |s, x| s.wrapping_add(*x))).collect();
    compare("sums", &download(d, sums)?, &want)
}

fn atomics(d: &mut Device) -> Result<Ran, String> {
    passed(atomics_(d))
}

fn atomics_(d: &mut Device) -> Result<(), String> {
    let n = 64 * 64;
    let src = values(n, 3, 1 << 24);
    let p = d.pipeline_spirv(program("atomics"), "main").map_err(e)?;
    let a = upload(d, &src)?;
    let hist = zeroed(d, 18)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![a, hist], groups: [64, 1, 1] }])?;
    let mut want = vec![0u32; 18];
    for v in &src {
        want[0] = want[0].wrapping_add(*v);
        want[1] = want[1].max(*v);
        want[2 + (*v & 15) as usize] += 1;
    }
    compare("total, maximum and bins", &download(d, hist)?, &want)
}

fn subgroup(d: &mut Device) -> Result<Ran, String> {
    passed(subgroup_(d))
}

fn subgroup_(d: &mut Device) -> Result<(), String> {
    let n = 256;
    let width = d.info().subgroup_size as usize;
    let src = values(n, 4, 1 << 16);
    let p = d.pipeline_spirv(program("subgroup"), "main").map_err(e)?;
    let a = upload(d, &src)?;
    let out = zeroed(d, 3 * n)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![a, out], groups: [(n / 64) as u32, 1, 1] }])?;
    let mut want = Vec::new();
    for i in 0..n {
        let base = i / width * width;
        let sum = src[base..base + width].iter().fold(0u32, |s, x| s.wrapping_add(*x));
        let neighbour = src[base + (i - base + 1) % width];
        want.extend([sum, sum, neighbour]);
    }
    compare("shuffled sums, reductions and neighbours", &download(d, out)?, &want)
}

/// Three submissions ordered only by a fence, the first held back until the host raises it; with a fill and a copy.
fn fences(d: &mut Device) -> Result<Ran, String> {
    passed(fences_(d))
}

fn fences_(d: &mut Device) -> Result<(), String> {
    let n = 128;
    let src = values(n, 5, 1 << 20);
    let p = d.pipeline_spirv(program("stages"), "main").map_err(e)?;
    let first = upload(d, &[3, 1])?;
    let second = upload(d, &[2, 5])?;
    let a = upload(d, &src)?;
    let t1 = zeroed(d, n)?;
    let t2 = zeroed(d, n)?;
    let out = zeroed(d, 2 * n)?;
    let f = d.fence().map_err(e)?;
    let groups = [(n / 32) as u32, 1, 1];
    d.submit(Submit {
        commands: &[Command::Dispatch { pipeline: p, bindings: vec![first, a, t1], groups }],
        wait: &[(f, 1)],
        signal: &[(f, 2)],
    })
    .map_err(e)?;
    d.submit(Submit {
        commands: &[
            Command::Dispatch { pipeline: p, bindings: vec![second, t1, t2], groups },
            Command::Fill { range: out, value: 0xdead_beef },
            Command::Copy { from: t2, to: out.memory, at: 0 },
        ],
        wait: &[(f, 2)],
        signal: &[(f, 3)],
    })
    .map_err(e)?;
    std::thread::sleep(Duration::from_millis(20));
    let early = d.fence_value(f).map_err(e)?;
    if early != 0 {
        return Err(format!("the fence reached {early} before the host raised it to 1"));
    }
    d.signal(f, 1).map_err(e)?;
    if !d.wait(f, 3, TIMEOUT).map_err(e)? {
        return Err(format!("the fence stopped at {}, never reaching 3", d.fence_value(f).map_err(e)?));
    }
    let mut want: Vec<u32> = src.iter().map(|x| x.wrapping_mul(3).wrapping_add(1).wrapping_mul(2).wrapping_add(5)).collect();
    want.extend(std::iter::repeat_n(0xdead_beef, n));
    compare("the chain's output", &download(d, out)?, &want)
}

fn branches(d: &mut Device) -> Result<Ran, String> {
    passed(branches_(d))
}

fn branches_(d: &mut Device) -> Result<(), String> {
    let n = 128;
    let src: Vec<u32> = (0..n as u32).map(|i| (i * 37 + 11) % 5000).collect();
    let p = d.pipeline_spirv(program("branches"), "main").map_err(e)?;
    let a = upload(d, &src)?;
    let steps = zeroed(d, n)?;
    let floats = zeroed(d, 4 * n)?;
    let signed = zeroed(d, n)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![a, steps, floats, signed], groups: [(n / 64) as u32, 1, 1] }])?;
    let mut want_steps = Vec::new();
    let mut want_signed = Vec::new();
    let mut want_floats = Vec::new();
    for i in 0..n as u32 {
        let mut x = src[i as usize] + 1;
        let mut count = 0u32;
        while x != 1 {
            x = if x & 1 == 0 { x / 2 } else { x.wrapping_mul(3).wrapping_add(1) };
            count += 1;
            if count > 1000 {
                break;
            }
        }
        want_steps.push(count);
        let s = i as i32 - 32;
        let local = [s, -s, s / 3, s.rem_euclid(5)];
        let acc: i32 = local.iter().enumerate().filter(|(_, v)| **v >= 0).map(|(k, v)| v * (k as i32 + 1)).sum();
        want_signed.push(acc as u32);
        let f = i as f32 * 0.5;
        let z = if i % 2 == 0 { f + 1.0 } else { -f };
        let w = if i % 7 == 3 { acc as f32 } else { 99.0 };
        want_floats.extend([f, f * f, z, w].map(f32::to_bits));
    }
    compare("steps", &download(d, steps)?, &want_steps)?;
    compare("signed", &download(d, signed)?, &want_signed)?;
    compare("floats", &download(d, floats)?, &want_floats)
}

/// The IR's own rules, written as IR directly: division and remainder by zero, shifts past the width, float to
/// integer saturation, scans, and subgroup operations in divergent control flow. Each backend must give what the
/// IR documents, computed here in Rust.
fn rules(d: &mut Device) -> Result<Ran, String> {
    passed(rules_(d))
}

fn rules_(d: &mut Device) -> Result<(), String> {
    use gpu::ir::{BinOp, Builtin, Cmp, Entry, Op, Region, Scalar, Space, Stmt, SubgroupOp, Type, UnOp, Value};

    const LANES: usize = 32;
    const OUT: usize = 16;
    let xs = [0, 1, 7, 100, 0xffff_fff0, 0x8000_0000, 12345, 0x7fff_ffff];
    let ys = [0, 1, 3, 31, 32, 33, 7, 0xffff_ffff];
    let fs = [3.7f32, -3.7, 1e10, -1e10, f32::NAN, 0.5, -0.0, 2_147_483_520.0];
    let x: Vec<u32> = (0..LANES).map(|i| xs[i % 8]).collect();
    let y: Vec<u32> = (0..LANES).map(|i| ys[(i * 5 + i / 8) % 8]).collect();
    let f: Vec<f32> = (0..LANES).map(|i| fs[(i * 3) % 8]).collect();

    let mut e = Entry::new("rules", [LANES as u32, 1, 1]);
    e.bindings.push(gpu::ir::Binding { set: 0, binding: 0, kind: gpu::ir::BindingKind::Storage { writable: false } });
    e.bindings.push(gpu::ir::Binding { set: 0, binding: 1, kind: gpu::ir::BindingKind::Storage { writable: true } });
    e.workgroup.push(Region { size: 4, align: 4 });
    let mut b: Vec<Stmt> = Vec::new();
    fn l(e: &mut Entry, b: &mut Vec<Stmt>, ty: Type, op: Op) -> Value {
        let v = e.value(ty);
        b.push(Stmt::Let(v, op));
        v
    }
    let u = Type::U32;
    let input = l(&mut e, &mut b, Type::Ptr(Space::Device), Op::Binding(0));
    let output = l(&mut e, &mut b, Type::Ptr(Space::Device), Op::Binding(1));
    let wg = l(&mut e, &mut b, Type::Ptr(Space::Workgroup), Op::Workgroup(0));
    let lane = l(&mut e, &mut b, u, Op::Builtin(Builtin::LocalIndex));
    let lane64 = l(&mut e, &mut b, Type::Scalar(Scalar::U64), Op::Convert(lane, Scalar::U64));
    let k4 = l(&mut e, &mut b, Type::Scalar(Scalar::U64), Op::Const(Scalar::U64, 4));
    let k64 = l(&mut e, &mut b, Type::Scalar(Scalar::U64), Op::Const(Scalar::U64, (OUT * 4) as u64));
    let at = l(&mut e, &mut b, Type::Scalar(Scalar::U64), Op::Binary(BinOp::Mul, lane64, k4));
    let px = l(&mut e, &mut b, Type::Ptr(Space::Device), Op::Offset(input, at));
    let row = l(&mut e, &mut b, Type::Scalar(Scalar::U64), Op::Binary(BinOp::Mul, lane64, k64));
    let out = l(&mut e, &mut b, Type::Ptr(Space::Device), Op::Offset(output, row));
    let field = |e: &mut Entry, b: &mut Vec<Stmt>, base: Value, k: u64| {
        let n = l(e, b, Type::Scalar(Scalar::U64), Op::Const(Scalar::U64, k));
        l(e, b, Type::Ptr(Space::Device), Op::Offset(base, n))
    };
    let xv = l(&mut e, &mut b, u, Op::Load(px, u));
    let py = field(&mut e, &mut b, px, (LANES * 4) as u64);
    let yv = l(&mut e, &mut b, u, Op::Load(py, u));
    let pf = field(&mut e, &mut b, px, (LANES * 8) as u64);
    let fv = l(&mut e, &mut b, Type::F32, Op::Load(pf, Type::F32));
    let xi = l(&mut e, &mut b, Type::I32, Op::Bitcast(xv, Type::I32));
    let yi = l(&mut e, &mut b, Type::I32, Op::Bitcast(yv, Type::I32));
    let store = |e: &mut Entry, b: &mut Vec<Stmt>, k: u64, v: Value| {
        let p = field(e, b, out, k * 4);
        b.push(Stmt::Store(p, v));
    };
    let r = l(&mut e, &mut b, u, Op::Binary(BinOp::Div, xv, yv));
    store(&mut e, &mut b, 0, r);
    let r = l(&mut e, &mut b, u, Op::Binary(BinOp::Rem, xv, yv));
    store(&mut e, &mut b, 1, r);
    let r = l(&mut e, &mut b, Type::I32, Op::Binary(BinOp::Div, xi, yi));
    store(&mut e, &mut b, 2, r);
    let r = l(&mut e, &mut b, Type::I32, Op::Binary(BinOp::Rem, xi, yi));
    store(&mut e, &mut b, 3, r);
    let r = l(&mut e, &mut b, u, Op::Binary(BinOp::Shl, xv, yv));
    store(&mut e, &mut b, 4, r);
    let r = l(&mut e, &mut b, Type::I32, Op::Binary(BinOp::Shr, xi, yi));
    store(&mut e, &mut b, 5, r);
    let r = l(&mut e, &mut b, Type::I32, Op::Convert(fv, Scalar::I32));
    store(&mut e, &mut b, 6, r);
    let r = l(&mut e, &mut b, u, Op::Convert(fv, Scalar::U32));
    store(&mut e, &mut b, 7, r);
    let r = l(&mut e, &mut b, u, Op::Subgroup(SubgroupOp::InclusiveScan(BinOp::Add), Some(xv), None));
    store(&mut e, &mut b, 8, r);
    let r = l(&mut e, &mut b, u, Op::Subgroup(SubgroupOp::ExclusiveScan(BinOp::Add), Some(xv), None));
    store(&mut e, &mut b, 9, r);
    // Divergent: odd and even lanes each see only their own half.
    let one = l(&mut e, &mut b, u, Op::Const(Scalar::U32, 1));
    let low = l(&mut e, &mut b, u, Op::Binary(BinOp::And, lane, one));
    let odd = l(&mut e, &mut b, Type::BOOL, Op::Compare(Cmp::Eq, low, one));
    let (mut then, mut otherwise) = (Vec::new(), Vec::new());
    let r = l(&mut e, &mut then, u, Op::Subgroup(SubgroupOp::BroadcastFirst, Some(xv), None));
    store(&mut e, &mut then, 10, r);
    let r = l(&mut e, &mut otherwise, u, Op::Subgroup(SubgroupOp::BroadcastFirst, Some(yv), None));
    store(&mut e, &mut otherwise, 10, r);
    b.push(Stmt::If { cond: odd, then, otherwise });
    let four = l(&mut e, &mut b, u, Op::Const(Scalar::U32, 4));
    let two = l(&mut e, &mut b, u, Op::Const(Scalar::U32, 2));
    let m4 = l(&mut e, &mut b, u, Op::Binary(BinOp::Rem, lane, four));
    let is2 = l(&mut e, &mut b, Type::BOOL, Op::Compare(Cmp::Eq, m4, two));
    let (mut then, mut otherwise) = (Vec::new(), Vec::new());
    let el = l(&mut e, &mut then, Type::BOOL, Op::Subgroup(SubgroupOp::Elect, None, None));
    let zero = l(&mut e, &mut then, u, Op::Const(Scalar::U32, 0));
    let r = l(&mut e, &mut then, u, Op::Select(el, one, zero));
    store(&mut e, &mut then, 11, r);
    let seven = l(&mut e, &mut otherwise, u, Op::Const(Scalar::U32, 7));
    store(&mut e, &mut otherwise, 11, seven);
    b.push(Stmt::If { cond: is2, then, otherwise });
    // A counter in workgroup memory: zeroed by lane 0, then every lane adds one.
    let zero = l(&mut e, &mut b, u, Op::Const(Scalar::U32, 0));
    let first = l(&mut e, &mut b, Type::BOOL, Op::Compare(Cmp::Eq, lane, zero));
    b.push(Stmt::If { cond: first, then: vec![Stmt::Store(wg, zero)], otherwise: Vec::new() });
    let barrier = gpu::ir::Barrier { execution: true, workgroup_memory: true, device_memory: false };
    b.push(Stmt::Barrier(barrier));
    l(&mut e, &mut b, u, Op::Atomic(gpu::ir::AtomicOp::Add, wg, one, Scalar::U32));
    b.push(Stmt::Barrier(barrier));
    let r = l(&mut e, &mut b, u, Op::Load(wg, u));
    store(&mut e, &mut b, 12, r);
    let k7 = l(&mut e, &mut b, u, Op::Const(Scalar::U32, 7));
    let v4 = l(&mut e, &mut b, Type::Vector(Scalar::U32, 4), Op::Construct(vec![xv, yv, lane, k7]));
    let six = l(&mut e, &mut b, u, Op::Const(Scalar::U32, 6));
    let idx = l(&mut e, &mut b, u, Op::Binary(BinOp::Rem, lane, six));
    let r = l(&mut e, &mut b, u, Op::ExtractDynamic(v4, idx));
    store(&mut e, &mut b, 13, r);
    let r = l(&mut e, &mut b, Type::I32, Op::Unary(UnOp::Abs, xi));
    store(&mut e, &mut b, 14, r);
    let twenty = l(&mut e, &mut b, u, Op::Const(Scalar::U32, 20));
    let few = l(&mut e, &mut b, Type::BOOL, Op::Compare(Cmp::Lt, lane, twenty));
    let mut then = Vec::new();
    let r = l(&mut e, &mut then, u, Op::Subgroup(SubgroupOp::Reduce(BinOp::Max), Some(xv), None));
    store(&mut e, &mut then, 15, r);
    b.push(Stmt::If { cond: few, then, otherwise: Vec::new() });
    e.body = b;

    if d.info().subgroup_size as usize != LANES {
        return Err(format!("written for {LANES}-wide subgroups, this device's are {}", d.info().subgroup_size));
    }
    let p = d.pipeline(&e).map_err(e_)?;
    let mut input_words = x.clone();
    input_words.extend(&y);
    input_words.extend(f.iter().map(|v| v.to_bits()));
    let a = upload(d, &input_words)?;
    let o = zeroed(d, LANES * OUT)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![a, o], groups: [1, 1, 1] }])?;
    let got = download(d, o)?;

    let mut want = vec![0u32; LANES * OUT];
    let mut scan = 0u32;
    let max_low = x[..20].iter().copied().max().unwrap_or(0);
    for i in 0..LANES {
        let (xu, yu) = (x[i], y[i]);
        let (xs, ys) = (xu as i32, yu as i32);
        let w = &mut want[i * OUT..(i + 1) * OUT];
        w[0] = xu.checked_div(yu).unwrap_or(0);
        w[1] = xu.checked_rem(yu).unwrap_or(0);
        w[2] = if ys == 0 { 0 } else { xs.wrapping_div(ys) as u32 };
        w[3] = if ys == 0 { 0 } else { xs.wrapping_rem(ys) as u32 };
        w[4] = xu.wrapping_shl(yu);
        w[5] = xs.wrapping_shr(yu) as u32;
        w[6] = f[i] as i32 as u32;
        w[7] = f[i] as u32;
        w[9] = scan;
        scan = scan.wrapping_add(xu);
        w[8] = scan;
        w[10] = if i % 2 == 1 { x[1] } else { y[0] };
        w[11] = if i % 4 == 2 { (i == 2) as u32 } else { 7 };
        w[12] = LANES as u32;
        w[13] = [xu, yu, i as u32, 7][if i % 6 < 4 { i % 6 } else { 0 }];
        w[14] = xs.unsigned_abs();
        w[15] = if i < 20 { max_low } else { 0 };
    }
    const NAMES: [&str; OUT] = [
        "u32 div", "u32 rem", "i32 div", "i32 rem", "shl", "i32 shr", "f32 to i32", "f32 to u32", "inclusive scan",
        "exclusive scan", "broadcast first", "elect", "workgroup atomic", "dynamic lane", "abs", "divergent max",
    ];
    let mut wrong = Vec::new();
    for i in 0..LANES {
        for k in 0..OUT {
            if got[i * OUT + k] != want[i * OUT + k] {
                wrong.push(format!("lane {i} {}: {:#x}, wanted {:#x}", NAMES[k], got[i * OUT + k], want[i * OUT + k]));
            }
        }
    }
    if wrong.is_empty() { Ok(()) } else { Err(format!("{} wrong:\n    {}", wrong.len(), wrong.join("\n    "))) }
}

fn e_(err: gpu::Error) -> String {
    err.message
}

/// More of the IR's rules, written as IR, for what the SPIR-V programs don't reach: compare-exchange, 3-lane vectors
/// in memory (12 bytes, and not the word after), 64-bit and boolean values moved between lanes, a float remainder,
/// booleans as floats, signed division's one overflow, and a barrier that only orders memory.
fn more_rules(d: &mut Device) -> Result<Ran, String> {
    passed(more_rules_(d))
}

fn more_rules_(d: &mut Device) -> Result<(), String> {
    use gpu::ir::{AtomicOp, Barrier, BinOp, Builtin, Cmp, Entry, Op, Scalar, Space, Stmt, SubgroupOp, Type, Value};

    const LANES: usize = 32;
    const OUT: usize = 14;
    let fs = [3.7f32, -3.7, 1e10, -1e10, f32::NAN, 0.5, -0.0, 2_147_483_520.0];
    let x: Vec<u32> = values(LANES + 2, 7, 1000);
    let f: Vec<f32> = (0..LANES).map(|i| fs[(i * 3) % 8]).collect();

    let mut e = Entry::new("more rules", [LANES as u32, 1, 1]);
    e.bindings.push(gpu::ir::Binding { set: 0, binding: 0, kind: gpu::ir::BindingKind::Storage { writable: false } });
    e.bindings.push(gpu::ir::Binding { set: 0, binding: 1, kind: gpu::ir::BindingKind::Storage { writable: true } });
    let mut b: Vec<Stmt> = Vec::new();
    fn l(e: &mut Entry, b: &mut Vec<Stmt>, ty: Type, op: Op) -> Value {
        let v = e.value(ty);
        b.push(Stmt::Let(v, op));
        v
    }
    let (u, u64t) = (Type::U32, Type::Scalar(Scalar::U64));
    let ptr = Type::Ptr(Space::Device);
    let k = |e: &mut Entry, b: &mut Vec<Stmt>, v: u64| l(e, b, u64t, Op::Const(Scalar::U64, v));
    let input = l(&mut e, &mut b, ptr, Op::Binding(0));
    let output = l(&mut e, &mut b, ptr, Op::Binding(1));
    let lane = l(&mut e, &mut b, u, Op::Builtin(Builtin::LocalIndex));
    let lane64 = l(&mut e, &mut b, u64t, Op::Convert(lane, Scalar::U64));
    let k4 = k(&mut e, &mut b, 4);
    let at = l(&mut e, &mut b, u64t, Op::Binary(BinOp::Mul, lane64, k4));
    let px = l(&mut e, &mut b, ptr, Op::Offset(input, at));
    let krow = k(&mut e, &mut b, (OUT * 4) as u64);
    let row = l(&mut e, &mut b, u64t, Op::Binary(BinOp::Mul, lane64, krow));
    let out = l(&mut e, &mut b, ptr, Op::Offset(output, row));
    let field = |e: &mut Entry, b: &mut Vec<Stmt>, n: usize| {
        let n = k(e, b, n as u64 * 4);
        l(e, b, ptr, Op::Offset(out, n))
    };
    let store = |e: &mut Entry, b: &mut Vec<Stmt>, n: usize, v: Value| {
        let p = field(e, b, n);
        b.push(Stmt::Store(p, v));
    };
    let c = |e: &mut Entry, b: &mut Vec<Stmt>, s: Scalar, v: u64| {
        let t = Type::Scalar(s);
        l(e, b, t, Op::Const(s, v))
    };

    // Compare-exchange on this lane's own word (field 2, zeroed): 0 -> 5 succeeds, then 9 -> 6 fails.
    let slot = field(&mut e, &mut b, 2);
    let (zero, five, nine, six) =
        (c(&mut e, &mut b, Scalar::U32, 0), c(&mut e, &mut b, Scalar::U32, 5), c(&mut e, &mut b, Scalar::U32, 9), c(&mut e, &mut b, Scalar::U32, 6));
    let r = l(&mut e, &mut b, u, Op::CompareExchange(slot, zero, five, Scalar::U32));
    store(&mut e, &mut b, 0, r);
    let r = l(&mut e, &mut b, u, Op::CompareExchange(slot, nine, six, Scalar::U32));
    store(&mut e, &mut b, 1, r);
    // Only memory is ordered: every lane's exchange is seen before it reads its word back.
    b.push(Stmt::Barrier(Barrier { execution: false, workgroup_memory: false, device_memory: true }));
    let r = l(&mut e, &mut b, u, Op::Atomic(AtomicOp::Add, slot, zero, Scalar::U32));
    store(&mut e, &mut b, 2, r);

    // A 3-lane vector read from the input at this lane's word, plus one, stored at fields 3..6; field 6 stays 0.
    let uv3 = Type::Vector(Scalar::U32, 3);
    let v = l(&mut e, &mut b, uv3, Op::Load(px, uv3));
    let one = c(&mut e, &mut b, Scalar::U32, 1);
    let ones = l(&mut e, &mut b, uv3, Op::Construct(vec![one, one, one]));
    let v = l(&mut e, &mut b, uv3, Op::Binary(BinOp::Add, v, ones));
    let p = field(&mut e, &mut b, 3);
    b.push(Stmt::Store(p, v));

    // A 64-bit value from the next lane over (lane ^ 1): high word the lane, low word three times it.
    let k32 = k(&mut e, &mut b, 32);
    let hi = l(&mut e, &mut b, u64t, Op::Binary(BinOp::Shl, lane64, k32));
    let k3 = k(&mut e, &mut b, 3);
    let lo = l(&mut e, &mut b, u64t, Op::Binary(BinOp::Mul, lane64, k3));
    let wide = l(&mut e, &mut b, u64t, Op::Binary(BinOp::Or, hi, lo));
    let moved = l(&mut e, &mut b, u64t, Op::Subgroup(SubgroupOp::ShuffleXor, Some(wide), Some(one)));
    let r = l(&mut e, &mut b, u, Op::Convert(moved, Scalar::U32));
    store(&mut e, &mut b, 7, r);
    let r = l(&mut e, &mut b, u64t, Op::Binary(BinOp::Shr, moved, k32));
    let r = l(&mut e, &mut b, u, Op::Convert(r, Scalar::U32));
    store(&mut e, &mut b, 8, r);

    // A boolean from the next lane over, and a boolean as a float.
    let low = l(&mut e, &mut b, u, Op::Binary(BinOp::And, lane, one));
    let odd = l(&mut e, &mut b, Type::BOOL, Op::Compare(Cmp::Eq, low, one));
    let theirs = l(&mut e, &mut b, Type::BOOL, Op::Subgroup(SubgroupOp::ShuffleXor, Some(odd), Some(one)));
    let r = l(&mut e, &mut b, u, Op::Convert(theirs, Scalar::U32));
    store(&mut e, &mut b, 9, r);
    let r = l(&mut e, &mut b, Type::F32, Op::Convert(odd, Scalar::F32));
    store(&mut e, &mut b, 10, r);

    // A float remainder, with the dividend's sign.
    let kf = k(&mut e, &mut b, (LANES as u64 + 2) * 4);
    let fbase = l(&mut e, &mut b, ptr, Op::Offset(input, kf));
    let pf = l(&mut e, &mut b, ptr, Op::Offset(fbase, at));
    let fv = l(&mut e, &mut b, Type::F32, Op::Load(pf, Type::F32));
    let onehalf = c(&mut e, &mut b, Scalar::F32, 1.5f32.to_bits() as u64);
    let r = l(&mut e, &mut b, Type::F32, Op::Binary(BinOp::Rem, fv, onehalf));
    store(&mut e, &mut b, 11, r);

    // The one signed division that overflows: MIN / -1 wraps to MIN, and its remainder is 0.
    let min = c(&mut e, &mut b, Scalar::I32, 0xFFFF_FFFF_8000_0000);
    let neg = l(&mut e, &mut b, Type::I32, Op::Bitcast(lane, Type::I32));
    let all = c(&mut e, &mut b, Scalar::I32, u64::MAX);
    let minus_one = l(&mut e, &mut b, Type::I32, Op::Binary(BinOp::Or, neg, all));
    let r = l(&mut e, &mut b, Type::I32, Op::Binary(BinOp::Div, min, minus_one));
    store(&mut e, &mut b, 12, r);
    let r = l(&mut e, &mut b, Type::I32, Op::Binary(BinOp::Rem, min, minus_one));
    store(&mut e, &mut b, 13, r);
    e.body = b;

    if d.info().subgroup_size as usize != LANES {
        return Err(format!("written for {LANES}-wide subgroups, this device's are {}", d.info().subgroup_size));
    }
    let p = d.pipeline(&e).map_err(e_)?;
    let mut input_words = x.clone();
    input_words.extend(f.iter().map(|v| v.to_bits()));
    let a = upload(d, &input_words)?;
    let o = zeroed(d, LANES * OUT)?;
    run_now(d, &[Command::Dispatch { pipeline: p, bindings: vec![a, o], groups: [1, 1, 1] }])?;
    let got = download(d, o)?;

    let mut want = vec![0u32; LANES * OUT];
    for i in 0..LANES {
        let w = &mut want[i * OUT..(i + 1) * OUT];
        w[0] = 0;
        w[1] = 5;
        w[2] = 5;
        w[3] = input_words[i] + 1;
        w[4] = input_words[i + 1] + 1;
        w[5] = input_words[i + 2] + 1;
        w[6] = 0;
        w[7] = ((i ^ 1) * 3) as u32;
        w[8] = (i ^ 1) as u32;
        w[9] = u32::from((i ^ 1) % 2 == 1);
        w[10] = if i % 2 == 1 { 1.0f32.to_bits() } else { 0 };
        w[11] = (f[i] % 1.5).to_bits();
        w[12] = 0x8000_0000;
        w[13] = 0;
    }
    const NAMES: [&str; OUT] = [
        "exchange", "failed exchange", "exchanged word", "vec3 x", "vec3 y", "vec3 z", "after the vec3",
        "u64 shuffle low", "u64 shuffle high", "bool shuffle", "bool to f32", "f32 rem", "i32 MIN / -1",
        "i32 MIN % -1",
    ];
    let nan = |w: u32| f32::from_bits(w).is_nan();
    let mut wrong = Vec::new();
    for i in 0..LANES {
        for k in 0..OUT {
            let (g, w) = (got[i * OUT + k], want[i * OUT + k]);
            // Any NaN is the remainder of a NaN.
            if g != w && !(k == 11 && nan(g) && nan(w)) {
                wrong.push(format!("lane {i} {}: {g:#x}, wanted {w:#x}", NAMES[k]));
            }
        }
    }
    if wrong.is_empty() { Ok(()) } else { Err(format!("{} wrong:\n    {}", wrong.len(), wrong.join("\n    "))) }
}
