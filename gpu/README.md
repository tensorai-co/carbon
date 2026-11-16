# gpu

A machine's GPU. Programs in a machine find a graphics and compute device (a Vulkan device, a DXGI adapter, a CUDA
device, a render node) whichever host they're on, the way a machine gets a network of its own. Behind it is the host's
GPU when there is one and a CPU device when there isn't, so **a machine always has a GPU, and acceleration only makes
it faster**. Both kernels share it, as they share `display`, `net` and `audio`.

Underneath, `gpu` is a GPU driver whose hardware is the host's graphics API, built the way real driver stacks are:
**APIs on top, one device model and one shader IR in the middle, one backend per host below**, all of it Carbon's own
Rust, with nobody else's translation layer or renderer inside.

**Built: the compiler, and compute and draws on Metal and the CPU.** SPIR-V compute, vertex and fragment shaders are
read into gpu's IR, validated, and run on the Mac's GPU (written as AIR, the IR Apple's GPU drivers compile, in the
`.metallib` files Metal loads) and on the CPU device (interpreted, drawing by Vulkan's rasterization rules), giving the
same results word for word on the conformance suite, vkcube's frame among them. lambda's Vulkan driver (step 2) runs
compute and draws: `--gpu` gives a Linux machine a render
node and the driver, `vulkaninfo` in `debian:13` lists a Carbon GPU, and a Vulkan compute program built in the image
gets the same words from it on the M1 and on the CPU device; Debian's `vkcube` spins on the headless display on both,
at the display's 60 Hz on the M1 (over 220 frames a second unpaced, on X11).

```sh
cargo run -p gpu --example gpu-conformance     # the suite on the CPU device and on this host's GPU
cargo test -p gpu -p gpu-ir -p gpu-frontend -p gpu-backend
```

```
 guest drivers      Carbon's own, running in the program, thin: they encode and the host lowers (to do)
                      atlas:  dxgi, d3d11, d3d12, vulkan-1, nvcuda (d3d9, opengl32 later)
                      lambda: a Vulkan driver (ICD) and libcuda.so behind a render node;
                              and a GL vendor library (libGLX_carbon) for GLVND
                    │  one ring per device in the program's memory; a doorbell exit on submit,
                    │  fence wait and present, never per call
                    ▼
 gpu/wire           the ring protocol: each API's calls, encoded as they are (to do)
                    ▼
 gpu/api            each API lowered onto the device, host-side and native: Vulkan (near 1:1),
                    Direct3D 11 (state, hazards, renaming), Direct3D 12, CUDA (memory, modules,
                    launches) (to do)
                    ▼
 gpu                the device model: memory, pipelines, command lists (dispatch, copy, fill),
                    images, samplers, render passes and draws, one queue, 64-bit timeline
                    fences                                (built: compute and draws)
                    │
 gpu-frontend ──► gpu-ir ──► gpu-backend
 spirv            SSA,       air (Metal), cpu (the interpreter)                         (built)
 (dxbc, dxil,     structured spirv (to do)
 ptx to do)       control flow
       ┌────────────┬────────────────────────────────┬─────────────────────┐
       ▼            ▼                                ▼                     ▼
   macos         linux, windows                   software              none
   (Metal)       (Vulkan through the host's       (the CPU device)      (no device)
   built         loader: to do; the CPU device    built
                 until then)
                    │
                    ▼
 display            a window's swapchain image composited with everything else (to do)
```

## Why it's needed

A large browser-engine program starts under atlas with all its processes, and graphics is what's left. On a Windows
machine with no usable GPU it would get Microsoft's software adapter (WARP) under DXGI and Direct3D 11 and never know
the difference. Under atlas, `CreateDXGIFactory` fails (`DXGI_ERROR_UNSUPPORTED`), ANGLE can't make an EGL display,
its own software fallback isn't reached yet, and the GPU process dies. Every Chromium, Electron or game-engine program
meets the same wall, and so do Linux images (GTK 4, Qt Quick, Firefox's WebRender).

Graphics isn't the only caller. A machine running PyTorch, llama.cpp or any other AI workload wants a compute device,
and most of those reach it through CUDA, which never makes a Vulkan call. A design that funnels everything through one
graphics API breaks there, so `gpu` meets programs lower down: at the device, and at their shader code.

## On the command line

Built in `cli`, but for `host=NAME` and `carbon ls`'s column:

| Option | What the machine gets | Says |
| --- | --- | --- |
| `--gpu auto` (default) | the host's GPU, else the CPU device | which, once, on stderr: `gpu: software (no Metal or Vulkan device on this host)` |
| `--gpu host` | the host's GPU | an error if there's none |
| `--gpu host=NAME` | one of several host GPUs, by name or index (`carbon gpu` lists them) | an error if there's none |
| `--gpu software` | the CPU device, the same on every host | |
| `--gpu none` | no device: DXGI enumerates no adapters, no render node, no CUDA device | programs use their own fallback |

```sh
carbon run --gpu auto  setup.exe       # the host's GPU if it has one, else the CPU device, said at start (default)
carbon run --gpu host  setup.exe       # the host's GPU, or fail
carbon run --gpu software debian:13    # a GPU on the CPU, the same on every host
carbon run --gpu none  setup.exe       # no device at all: programs take their own fallback
```

`carbon gpu` lists the host's device and the software one, with the backend each uses and its subgroup width. To
come: `carbon ls` gets a `GPU` column beside `ISOLATION`. Only lambda reaches the device yet; atlas's DXGI is step 5.

## What gpu sees

The question that shapes everything: at the bottom, what does `gpu` get from a program?

**Not one API's calls.** A contract that is "Vulkan, passed through" serves Vulkan programs and whatever lowers to
Vulkan, but only by leaning on other people's translators, and CUDA doesn't lower to it at all.

**Not a virtual NVIDIA or AMD card.** Emulating real hardware means running that vendor's driver in the guest
(NVIDIA's closed userspace, tied to one driver release's ioctls; AMD's ROCm or radv) and taking its finished machine
code. That's the worst input there is for what has to happen next: it's chip-specific, its control flow is flattened
into execution masks, its types are gone, and NVIDIA's (SASS) isn't documented. On a Mac nothing below Metal can be
reached anyway (the GPU's command interface is private IOKit), so whatever the guest produces must be rebuilt as Metal
shaders, and rebuilding them from machine code is decompiling.

**A device, and programs in a portable IR.** Every API already ships its GPU code in a portable form, precisely so a
driver can compile it for whatever chip is there. That's the seam vendors' own drivers sit on, and it's where `gpu`
sits:

| API | What its programs hand the driver | Frontend | State |
| --- | --- | --- | --- |
| Vulkan (both kernels) | SPIR-V | `spirv` | Compute built; vertex and fragment shaders built |
| OpenGL and GLES (both kernels) | GLSL source, compiled by a GLSL frontend of our own | `glsl` | Vertex and fragment shaders and compute kernels, held to glslang; GL 3.3's draws, framebuffer objects and the compatibility profile built, Luanti playing in a world |
| Direct3D 10 and 11 | DXBC | `dxbc` | Vertex, pixel and compute shaders (most of shader model 5.0) |
| Direct3D 12 | DXIL (LLVM bitcode) | `dxil`, a bitcode reader of our own, not LLVM | To do |
| Direct3D 9 | shader model 2 and 3 bytecode | `d3d9` | Later |
| CUDA | PTX, inside fat binaries | `ptx` | To do |

So the guest sees a **Carbon GPU**: a vendor and device ID of its own (as WARP is `0x1414` and virtio-gpu is
`0x1af4`), with the host's real limits. It doesn't pretend to be NVIDIA's or AMD's hardware. CUDA is the one place a
vendor shows through, and only as API facts: our `libcuda` reports a compute capability, because the API has one.
Reporting a known vendor's ID, for programs that gate features on it (a browser's GPU blocklist), is an option a
machine can turn on, never the default.

## The rules gpu keeps

1. **The kernel brings the OS; the image brings the user's software.** GPU drivers are the OS's, so the guest drivers
   are Carbon's: atlas's `dxgi`, `d3d11`, `vulkan-1` and `nvcuda`, and lambda's Vulkan driver and `libcuda.so`, made
   in memory or put in a system layer, the way NVIDIA's container toolkit puts its driver into containers. Mesa,
   ANGLE or SwiftShader in an image or a program are the user's and run as they are. The one exception planned:
   libraries made only of one vendor's machine code (cuBLAS, cuDNN), replaced by ours and said so.
2. **One host, one backend.** A shared part and a folder per host, with every host-specific line inside it
   (`src/macos/`).
3. **Dependencies point one way.** Kernels will reach `gpu` through `machine`, never the host's API. `display`
   composites `gpu` images; `gpu` knows nothing of windows.
4. **A software path always works, and the GPU is only a faster one.** The CPU device is a backend of `gpu`'s own
   compiler, running the same IR, not a second implementation.
5. **It never falls back silently.** `Choice::Auto` on a host with no GPU backend gets the CPU device and
   [`Info::why`](src/lib.rs) says why; `Info::describe` is the line a machine prints at start, as `--isolation auto`
   does.
6. **No dependencies we can do without.** The host's own system libraries (Metal, the Vulkan loader) through plain
   FFI: Metal through the Objective-C runtime (`src/macos/objc.rs`), as `display/macos` reaches Cocoa, and its
   programs as AIR bitcode written here. No bindings crates, no Vulkan-on-Metal layers, no translators, no LLVM: none
   of `ash`, MoltenVK, KosmicKrisp, lavapipe, SwiftShader, DXVK, VKD3D-Proton, SPIRV-Cross or naga. Apple's toolchain
   only in tests, as the oracle. Vulkan's bindings and the wire's encoders will be generated by a small generator of
   our own, from `vk.xml` and the other APIs' descriptions, as lambda's Wayland protocols are from theirs.
7. **One conformance suite per contract.** Every backend runs the same programs: exact results for integer compute,
   copies and fills (the same words), and the API's own precision rules for floating point and rasterization.

## The IR (`gpu-ir`)

A GPU compiler has five inputs and three outputs; as direct translators that would be fifteen of them, and with an IR
in the middle it's five frontends and three backends. (`exec/translate` goes straight from decoder to emitter, which
is right for one input ISA at a time.) The IR is shaped like Mesa's NIR and naga's, both read and neither used. What
every program looks like between a frontend and a backend (`ir/src/lib.rs`), `no_std` and with no `unsafe`:

- **SSA values with structured control flow.** A value is defined once by a `Let`; control flow is a tree of `If`,
  `Loop` (a body, a continuing block, an optional break condition), `Break`, `Continue` and `Return`. Structured,
  because SPIR-V, one of the outputs, needs structure, and AIR's control-flow graph takes it as easily.
- **Variables for what changes**: typed registers, written with `Write` and read with `Read`. SPIR-V's phis and its
  scalar locals become these.
- **Memory is bytes, and pointers are addresses** in a space (device, workgroup, private). Loads and stores give their
  type; `Offset` moves a pointer by a signed or unsigned count of bytes. That's CUDA's model and Vulkan's buffer device
  addresses' (both do pointer arithmetic on 64-bit device pointers); frontends lower typed access chains to it with
  each storage class's layout.
- **Subgroups are first-class**: shuffles, broadcasts, elections, votes, reductions and scans over the active lanes,
  with the width part of the program when it depends on one (`Entry::subgroup_size`). CUDA's warps are 32 wide, as
  Metal's SIMD groups are on Apple silicon.
- **Rules every backend keeps**: integer division and remainder by zero give 0, shift counts are taken modulo the
  width, float to integer rounds toward zero and saturates (NaN gives 0), floats round as IEEE says (no fast math).
- **Stages**: an `Entry` is a compute kernel, a vertex shader or a fragment shader (`Stage`). The graphics stages
  read inputs with `Input` and write outputs with `Output`, by slot: a location (a vertex attribute, a value passed
  to the fragment shader and interpolated smooth, linear or flat, or a colour attachment), the vertex's `Position`, a
  fragment's `FragDepth`. Their builtins are the vertex and instance index, the fragment's coordinate and facing. A
  fragment shader runs in 2×2 quads: `Derivative` is the change to the neighbouring pixel along x or y, `Sample` reads
  an image (its level of detail from the coordinate's derivatives, or given), and `Discard` throws the fragment away.

Values are scalars (bool, 32- and 64-bit integers, f32), vectors of 2 to 4, or pointers; structs, arrays and matrices
aren't values, so frontends split them into their parts, and images and samplers are bindings `Sample` names. `validate` checks every entry point before a backend sees it: each
value defined once and before its uses, every operand's type, `Break` and `Continue` inside loops, every index in
range, booleans kept out of memory, each stage's builtins, inputs, outputs and operations its own. `Entry` prints as text for traces (`GPU_PRINT_IR=1` in `gpu-frontend`'s tests).

To come, as the frontends that need them arrive:

- **A structurizer.** PTX and DXIL arrive unstructured (branches to labels), so one is part of the core, with node
  splitting for the irreducible cases.
- **Constant and push-constant address spaces**, and pointer casts (integers to device pointers and back).
- **Subgroup width emulated** on a host whose subgroups differ from the width a program was written for.
- **Cooperative matrix operations** (PTX's `mma` and `ldmatrix`, Vulkan's cooperative matrices), lowered to Metal's
  `simdgroup_matrix` where it exists and to plain arithmetic where it doesn't.

## Frontends (`gpu-frontend`)

**SPIR-V** (`frontend/src/spirv/`), compute kernels and vertex and fragment shaders as glslang and Vulkan programs
write them:

- Two passes: the module's types, constants, decorations, globals, entry points and blocks first; then each compute,
  vertex and fragment entry point translated from its first block.
- **Control flow** rebuilt from SPIR-V's merge annotations: a region of blocks runs until the merge of the construct
  it's in, the innermost loop's merge (`Break`) or continue target (`Continue`); a selection header becomes an `If`, a
  loop header a `Loop` whose continuing block is the continue construct. `OpSwitch` becomes a chain of `If`s (a case
  falling through translates the next case's blocks again).
- **Phis** become variables, written by each predecessor before its branch.
- **Memory**: storage and uniform buffers are bindings, in (set, binding) order, with their explicit `Offset`,
  `ArrayStride` and `MatrixStride` (column- or row-major) layout; workgroup variables are workgroup regions; function
  and private variables are registers when they're scalars or vectors, private memory (natural layout) otherwise.
  Booleans in memory are 32-bit words. Images, samplers and combined image samplers are bindings too.
- **Stages**: inputs at a location (with their `Flat` or `NoPerspective` decoration) are read once at the start;
  outputs, including `gl_PerVertex`'s members, are registers written out wherever the shader returns, so a shader
  can read back what it wrote, as vkcube's reads `gl_Position`. `gl_PointSize` is a register nothing reads.
- **Instructions**: integer and float arithmetic with SPIR-V's signedness taken from the opcode (bitcasts where the
  operand types differ), `SMod` and `FMod` lowered to remainders, comparisons, conversions, composites and shuffles;
  matrices as their columns, and the matrix instructions (times vector, vector times, times matrix, times scalar,
  outer product, transpose) as vector arithmetic; `GLSL.std.450`'s abs, sign, min, max, clamp, mix, step,
  smoothstep, floor, ceil, trunc, round, fract, sqrt, inverse sqrt, sin, cos, tan, exp, exp2, log, log2, pow (as
  exp2 of y log2 x), radians, degrees, length, distance, dot, cross, normalize, face-forward, reflect and fma (not
  fused); `OpSampledImage` and `OpImageSample*Lod` with a bias or a level; the derivatives (fine, coarse or either, and
  `fwidth`); `OpKill` and `OpTerminateInvocation`; atomics, barriers, and the non-uniform subgroup operations (elect,
  all, any, broadcasts, shuffles, reductions and scans).

Push constant blocks are a uniform binding at a set of their own (`gpu_ir::PUSH_CONSTANTS`), which the API fills with
the bytes pushed, so backends see one more buffer. Not translated yet, each refused with a message naming it: function
calls (inline them with `spirv-opt`), images other than sampled 2D floats (storage images, depth compares, arrays, cubes, 3D), gradients and
offsets on samples, clip and cull distances, stage inputs and outputs that are structs or arrays, 16-bit and 64-bit
floats, `OpSpecConstantOp`, ballots, clustered reductions, and a branch from a nested selection out to an outer one's
merge.

**DXBC** (`frontend/src/dxbc/`), Direct3D 10 and 11's shader models 4.0 to 5.0: vertex, pixel and compute shaders.

- **The container** (`container.rs`): its chunks; the program (`SHEX` or `SHDR`) and the input and output signatures
  (`ISGN`/`ISG1`, `OSGN`/`OSG5`/`OSG1`), each element's semantic, index, system value, component type, register and
  mask. The checksum isn't checked.
- **Tokens** (`tokens.rs`): opcode tokens and their extended tokens (sample offsets), operands with masks, swizzles,
  select-1, modifiers, up to three indices each immediate or relative, immediates, declarations' own words, and the
  immediate constant buffer's custom data.
- **Translation** (`translate.rs`): registers are typeless, so each `r#` is a `vec4<u32>` variable and every
  instruction bitcasts what it reads, one destination component at a time; inputs are read once at the start, and
  outputs written out at each `ret` and the end. A stage's input or output is at the location of its register, so a
  vertex shader's outputs and a pixel shader's inputs meet as D3D's linkage puts them, and an input layout's
  elements are attributes at their registers; the signatures come back with the entry point for the API to link by
  semantic. `SV_Position` (the pixel shader's with D3D's w, the reciprocal of Vulkan's), `SV_VertexID`,
  `SV_InstanceID`, `SV_IsFrontFace`, `SV_Target#`, `SV_Depth` and the compute thread ids. Constant buffers
  (dynamically indexed too), the immediate constant buffer and indexable temps (private memory), group-shared memory.
  The float, integer, bit, conversion and comparison instructions (comparisons give all ones or zero; `udiv` by zero
  gives all ones; `imul` and `umul`'s high words through 64-bit products); `if`/`else`, `loop`, `break(c)`,
  `continue(c)`, `ret(c)`, `discard`, and `switch` as a loop run once whose cases are tests in order (falling
  through), a `continue` inside it a flag the loop turns into the real one. `sample`, `sample_l` and `sample_b` on 2D
  textures; raw and structured buffers (SRVs and UAVs), their atomics, and `sync`.
- **Bindings**: each stage has its own tables of slots, so a binding's set is the kind (`b#` constant buffers 0,
  `t#` resources 1, `s#` samplers 2, `u#` UAVs 3) and its binding the slot.

Not read yet, each refused naming it: `ld` and `resinfo` (the IR has no texel fetch or image size yet), comparison and
gradient samples, texel offsets, texture arrays, cubes and 3D textures, typed buffers and UAVs, clip and cull
distances, geometry and tessellation stages, doubles, shader model 5.1, and `f16tof32`, `firstbit` and the bit-field
instructions. The suite runs DXBC two ways: assembled by hand (`examples/dxbc_asm.rs`), for what HLSL compilers won't
write (a `continue` inside a `switch`, `ftou` and `utof` beside `ftoi`), and compiled from HLSL by vkd3d's compiler
(`examples/programs/*.hlsl`), which holds the reading to a real compiler's output. vkd3d 2.1 miscompiles two casts
(`asuint((float)b)` comes out as `b`, and `(uint)a` as `(int)a`'s `ftoi` when both are in a program), so the HLSL
programs leave them out.

**GLSL** (`frontend/src/glsl/`), OpenGL's and OpenGL ES's source, as programs hand it to `glShaderSource`: GLSL 1.10
to 4.60 and ES 1.00 to 3.20 are read, for vertex and fragment shaders and compute kernels. It's Carbon's GL's
compiler, so no SPIR-V is made on the way: GLSL is structured, as the IR is.

- **The preprocessor and lexer** (`pp.rs`), one pass: comments and line continuations, `#version` (and ES's default
  without one), object- and function-like macros with `##` and arguments over several lines, the conditionals with
  `defined`, `#extension` (those it has, and `require` of others refused), `#error`, and the predefined macros.
- **The parser** (`parse.rs`, `ast.rs`): declarations with every qualifier and `layout`, structs, uniform and buffer
  blocks, functions and prototypes, every statement, and expressions by precedence; a name is a type if GLSL's or a
  struct's declared before it.
- **Lowering** (`lower.rs`, `expr.rs`, `builtins.rs`), type-checking as it goes, one entry for `main`. Every call is
  inlined (GLSL has no recursion; recursion is an error): parameters are variables, `out` and `inout` copied back, the
  body a loop run once whose `return` sets a flag and breaks, each loop it left followed by a check that breaks
  again. Composites are trees of IR values and variables (a matrix its columns), and indexing by a value selects
  among the elements (or addresses memory). `switch` is a loop run once with fallthrough guards, a `continue` inside
  it a flag. Operators as GLSL types them, implicit conversions as each version has them (none in ES), constructors,
  swizzles, short-circuits and `?:` that evaluate only their side when it has effects. The built-ins: angle,
  exponential, common, geometric, matrix (`determinant` and `inverse` by cofactors), vector-relational, bit casts,
  `texture`/`texture2D` (with `Lod` and `Proj`), derivatives, barriers and atomics; `asin`, `acos` and `atan` are
  Abramowitz and Stegun's polynomials (within 4e-8).
- **The interface**: inputs, outputs, built-in variables per stage (`gl_Position`, `gl_FragColor`, `gl_FragData`,
  `gl_FragCoord`, `gl_VertexID`, the compute ids…), plain uniforms in a default block laid out std430 with GL's names
  for its members (`lights[1].color`), samplers, and std140/std430 uniform and buffer blocks, row-major too. `link`
  matches a vertex shader's outputs to a fragment shader's inputs by name, and numbers samplers and blocks across the
  program; each stage's default block is a binding of its own.

Not there yet, each refused naming it: samplers other than `sampler2D` (the IR's images are 2D), `texelFetch` and
`textureSize`, `in`/`out` blocks and arrays of blocks, doubles, a runtime array's `.length()`, images, subroutines,
geometry and tessellation stages, and `gl_PointCoord` and clip distances.

To come: `dxil` (Direct3D 12, LLVM bitcode read by a reader of our own), `ptx` (CUDA), and later `d3d9`.

## Backends (`gpu-backend`)

This crate uses `std`: the CPU backend needs the host's float functions, and both only ever run host-side.

- **`air`** writes each entry point as a `.metallib` of one kernel, which Metal loads with `newLibraryWithData:`:
  nothing on the user's machine compiles source. AIR is LLVM IR in Apple's dialect (a control-flow graph with typed
  pointers in numbered address spaces, kernels described in `!air.kernel` metadata, and barriers, SIMD-group
  operations, atomics and math as `air.*` intrinsics), written as LLVM bitcode in Apple's wrapper. Its parts:
  - `lower`: the IR onto an AIR module. Bindings are `device uchar*` at `[[buffer(n)]]`, then the builtins the
    program reads; a pointer is `i8 addrspace(n)*`, moved by bytes and cast to what each load or store reads (a
    3-lane vector a lane at a time, so it touches its 12 bytes and no more); `If` and `Loop` become branches;
    variables, private regions, and values used outside the block that defines them are allocas, which Metal's
    compiler turns back into registers. Each operation lowers as `xcrun metal` lowers its MSL counterpart:
    `air.convert` for float and integer conversions (which saturate), `air.fmod`, `air.simd_*`, `air.atomic.*`.
  - `bitcode`: the bitstream and LLVM's records in the typed-pointer format Apple's LLVM reads, with no
    abbreviations beyond the two the format needs, so the same program always gives the same bytes.
  - `metallib`: the library's header, its function list (name, type, the bitcode's SHA-256, offsets, versions) and
    its sections.

  It writes compute kernels, vertex functions and fragment functions (each listed in `!air.kernel`, `!air.vertex` or
  `!air.fragment`, with its arguments' and results' descriptors, and its type in the library: kernel 2, vertex 0,
  fragment 1). A vertex function returns a packed struct of its position (y negated: Vulkan's clip space has y down,
  Metal's up) and its `user(locnN)` outputs, and fetches its own attributes (vertex pulling): the vertex buffers are
  buffers after its bindings, read at `vertex_id` or `instance_id` by the pipeline's layout (`air::VertexInput`), so
  Metal needs no vertex descriptor. A fragment function takes each input as an interpolated argument (perspective,
  no-perspective or flat), its position and facing, and textures and samplers as Metal's opaque handles
  (`struct._texture_2d_t` in address space 1, `struct._sampler_t` in 2); it samples with `air.sample_texture_2d`, takes
  derivatives with `air.dfdx` and `air.dfdy`, discards with `air.discard_fragment`, and returns its colours and depth.
  A module is written for macOS 13 (AIR 2.5) in MSL 3.0, which every later macOS loads; one with a barrier that only
  orders memory uses MSL 3.2's fence, and says MSL 3.2 and macOS 15. Not written yet: 64-bit atomics, 64-bit
  SIMD-group reductions (Metal has none; 64-bit shuffles move as pairs of words), and scans other than sums and
  products. The conventions were read from what `xcrun metal -S -emit-llvm` makes of the matching MSL, following
  the [`air`](https://github.com/vertex-language/air) package's findings, and `tests/air.rs` holds every suite
  program's library to Apple's `air-validate` and `air-opt` where the Metal toolchain is installed.
- **`cpu`** interprets the IR a workgroup at a time, every lane in lockstep with a mask of those still running, the
  way a GPU runs a subgroup: barriers are trivially right, and subgroup operations see exactly the lanes active at
  that point. Device memory is the caller's (`cpu::Memory`); workgroup and private memory are zeroed per workgroup.
  The graphics stages run through `cpu::shade`: a group of lanes, each with its inputs and builtins, giving back its
  outputs. A fragment group's lanes are 2×2 quads, so a derivative is the difference between a lane and its
  neighbour; lanes the primitive doesn't cover are helpers, run for their values but never writing memory. Images are
  the caller's, through `cpu::Textures`. Later it's compiled to host code instead, one invocation per SIMD lane, using
  `exec/translate`'s ARM64 and x86-64 emitters where they fit.
- **`spirv`** (to come) writes SPIR-V for the host's Vulkan driver on Linux and Windows. A SPIR-V program's own
  shaders go through the IR too: validated and written again, never passed through blind.

## The device (`gpu`)

`Device::open(Choice)` gives the host's GPU, the CPU device, or none. The device model (`src/lib.rs`):

| Part | What it is |
| --- | --- |
| `Memory` | An allocation; `write` and `read` copy into and out of memory no submission in flight uses |
| `Image` | A 2D image in one format (RGBA8 and BGRA8, unorm and sRGB; R8, RG8 and A8 unorm; R32 and RGBA16 float; RGB10A2 unorm; RG11B10 float; RGBA32 float; D32 float, D16 unorm), with mip levels and array layers. An attachment or a copy is one level of one layer (`Sub`); sampling reads layer 0. A8 can't be drawn into, since Metal can't |
| `Sampler` | Nearest or linear filtering (magnified, minified, and between mip levels), a range of levels of detail, repeat, mirrored repeat, clamp or border addressing with one of three border colours, and a comparison for depth images (the CPU device's sampling does it; shaders reach it with the `dxbc` frontend's `sample_c`) |
| `Pipeline` | A compute program (`pipeline`, `pipeline_spirv`), or a graphics one (`graphics`): a vertex and a fragment entry point, the vertex buffers' layout and attributes, triangle lists, strips or fans, culling and front face, a depth test, and each colour attachment's format, blend and write mask (`src/graphics.rs`) |
| `Command` | `Dispatch` (a pipeline, its bindings as ranges, a grid of workgroups), `Copy`, `Fill` (a 32-bit pattern); `BeginRender` (colour and depth attachments, loaded or cleared), `SetViewport`, `SetScissor`, `Draw` (bindings by set and binding, vertex buffers, an optional 16- or 32-bit index buffer, instances), `EndRender`; copies between memory and one level of one layer of an image; `GenerateMips` (each level a box filter of the one before) |
| `Submit` | Commands for the one queue, in order, after the fences it waits for, raising the fences it signals |
| `Fence` | A 64-bit timeline: raised by submissions or by the host (`signal`), waited on by both (`wait`) |

`Device` checks every argument before its backend sees it (ranges inside their allocations, a dispatch's bindings
matching its pipeline, workgroups within the device's limit; a graphics pipeline's stages agreeing with each other
and with its attributes and attachments; draws only inside a render pass, with every binding the pipeline's stages
use bound to something of its kind, and attachments the pipeline's formats; copies inside their image and range), so a
backend only implements the contract. `gpu::spirv` reads a module's shaders for `graphics`. The rest of the model
comes with the APIs that need it: one GPU address space per context, sampling image arrays and views of parts of
images, more than one queue, barriers and timestamps in command lists, lines and points.

- **Metal** (`src/macos/`): shared buffers (on Apple silicon the GPU and CPU share memory), pipelines loaded from
  AIR libraries (IEEE math, said in the library), one command queue, a command buffer per submission with a compute
  or blit encoder per command, fences as `MTLSharedEvent`s. A fill is a small IR program of its own, since Metal's
  fill takes a byte. Metal tracks hazards between commands on the same buffers, so commands run in order. A failed
  command buffer is reported by the next `submit` or `wait`. `CARBON_TRACE=1` prints a program's IR when Metal won't
  load it. Draws (`src/macos/draw.rs`): textures in the GPU's own memory, sampler states, render pipeline states made
  from two AIR libraries with the depth test as a depth-stencil state, a render encoder for each render pass, each
  stage's bindings set at its own entry point's numbers, and blits between buffers and textures (how a frame is read
  back to present it). Metal has no triangle fans, so a fan is drawn as the triangles it makes, through indices made
  for the draw.
- **The CPU device** (`src/software/`): allocations at addresses of its own above 4 GiB with gaps between them, so a
  null pointer or an overrun faults with the address named; a queue that runs each submission once its waits are met,
  in submission order. Subgroups are 32 wide, as on Apple silicon, so programs see the same width on both. Its draws
  (`raster.rs`) keep Vulkan's rules:
  - vertices fetched by the pipeline's attributes and shaded 32 at a time, each once an instance;
  - triangles assembled (the provoking vertex each one's first), clipped in clip space to `0 <= z <= w` and a guard
    band four viewports wide, taken to framebuffer coordinates by the viewport, and culled by Vulkan's signed area;
  - coverage at pixel centres with edge functions in fixed point, 8 bits of subpixel precision, and the top-left
    rule, so a pixel on an edge two triangles share is drawn once;
  - fragments shaded in 2×2 quads, 8 a group, inputs interpolated perspective-correct, linearly or flat;
  - each result depth-tested, blended and masked in primitive order.

  Images are tightly packed texels (`texel.rs`), sRGB decoded when read and encoded when written. Being interpreted,
  it draws slowly: a reference and a fallback, not a fast path.
- **Linux and Windows** (to come): Vulkan through the host's loader, with shaders as SPIR-V; Direct3D 12 later on
  Windows. Until then those hosts get the CPU device, and say so.

## Guest drivers, lowered on the host (to come)

Guest drivers are built for the program's ISA, so an x86-64 program's run translated on Apple silicon, and anything
heavy in them pays for it. So they only encode: each API call becomes a record on the ring, and the work (Direct3D
11's state and hazard tracking, CUDA's module loading, every shader compile) happens host-side, at native speed, in
`gpu/api`. Calls that need an answer at once don't exit either:

- the device's properties and limits are written into the ring's header when the device is opened;
- each guest driver is handed a range of its context's address space and allocates from it itself, so `cuMemAlloc`
  and `vkAllocateMemory` return a pointer with no exit, and the host learns of them at the next submit;
- mapped memory is the machine's own frames (below), so a map is a pointer, not a call.

The guest drivers live with their kernels and build on `gpu-wire`: atlas's in `atlas/guest`, lambda's in a system
layer it adds to every image, which is new for lambda.

**Direct3D 11** (`api/src/d3d11.rs`) is the first API lowered this way without a wire: atlas's COM methods are host
exports already (as its DirectWrite is), so they read a call's arguments from the program's memory and call
`gpu_api::d3d11::Device` directly, with Direct3D's own structures and numbers. A `Device` is a device and its
immediate context:

- **Resources**: buffers are device memory (constant buffers at Direct3D's largest, 64 KiB, so a shader reading past
  a small one's end reads zeroes rather than faulting); 2D textures are images with their levels and slices; staging
  textures are memory with their subresources tightly packed; dynamic resources keep a host copy for `Map`. Formats:
  RGBA8 and BGRA8 (unorm, sRGB, and BGRX as BGRA), RGBA16 and RGBA32 float, RGB10A2, RG11B10, RG8, R8, A8, R32
  float, D32, D16, and D24S8 as D32. Views: shader resources of textures and of raw or structured buffers, render
  targets and depth-stencils of one level of one slice, UAVs of buffers.
- **States** with Direct3D's defaults: samplers (`D3D11_FILTER` decoded into the device's filters, address modes,
  the nearest of the device's three border colours, comparison), blend (per target, independent or not), depth, and
  rasterizer (cull, front face, scissor).
- **Shaders** from DXBC (`gpu::dxbc`), input layouts checked against the vertex shader's signature, appended offsets
  resolved.
- **The immediate context**: all of a draw's state; draws (indexed, instanced) and dispatches; clears;
  `UpdateSubresource`, `CopyResource`, `CopySubresourceRegion` (an image to an image through memory the submission
  makes), `GenerateMips`, `Map`/`Unmap` and a read-back for presents. Commands are recorded and submitted when
  something needs their results; updates are uploads among them, so a constant buffer mapped with `WRITE_DISCARD`
  between draws is seen by each as it was. Consecutive draws to the same targets are one render pass. A pipeline is
  made at the draw from the state that shapes it and kept; the pixel shader's bindings are moved to sets 4 to 7 (each
  of Direct3D's stages has its own slot tables, the device's two stages share theirs), its inputs linked to the
  vertex shader's outputs by semantic, and unbound slots get zeroes, a transparent black image and a sampler. The
  viewport is flipped (Direct3D's y is up in normalized device coordinates), so windings on the screen are
  Direct3D's.

Not yet: stencil, multisampling, sampling arrays and cubes, views that reinterpret formats, typed buffers and texture
UAVs, lines and points, deferred contexts, queries, and `SV_InstanceID` not counting `StartInstanceLocation` (the
IR's instance index does). `api/tests/d3d11.rs` draws a textured quad, depth with a scissor, culling and a constant
buffer changed between draws, copies and maps, a compute dispatch and mip generation, from HLSL, on the CPU device
and the M1.

## Memory is the machine's memory (to come)

On Apple silicon the GPU and CPU share memory, so device memory can be the machine's own frames, under a `MTLBuffer`
made over them with no copy. The host backs each context's address space with a few large buffers, so a pointer the
guest driver hands out stays the same for as long as its allocation lives, as CUDA requires. On discrete GPUs (Linux
and Windows hosts), device-local memory is the host driver's, and host-visible memory is imported where the driver
allows it (Vulkan's external host memory) and copied at submit where it doesn't.

## Fences are one thing

Vulkan's timeline semaphores, Direct3D 12's fences, CUDA's streams and events and Metal's `MTLSharedEvent` are all the
same monotonic 64-bit counter, so the device model has one kind of fence (built) and every API will wait on it. A wait
the guest driver can't answer from what it already knows is a doorbell exit, and the thread blocks in its kernel like
any other wait.

## Presenting

- **Built (first cut), for lambda:** a swapchain image is a `gpu` image, read back at each present into a buffer of
  the program's window system, which the driver then shows the way Mesa's software drivers do, over the program's own
  connection: on Wayland a `wl_shm` buffer attached to its surface (through the program's `libwayland-client`, on an
  event queue of the driver's, one frame at a time at the compositor's frame callbacks: FIFO); on X11 `PutImage` into
  its window (through its `libxcb`; Xlib's through `XGetXCBConnection`). The libraries are the program's own copies,
  found with `dlopen` by name. One copy a frame, and headless machines, `carbon display screenshot`, host windows and
  later `remote` work unchanged, since it's an ordinary client's buffer to lambda's servers.
- **Later, for zero copies:** shown straight from the GPU (an `IOSurface` under a `CAMetalLayer`, dmabuf on Linux, a
  shared handle on Windows), with the window named to the kernel (its Wayland or X11 id, or a DXGI swap chain's
  `HWND`) and the frame handed to `display` without going through a window system's buffer. That comes with step 8.

## CUDA, and what an AI workload needs

- **The driver API is ours.** Every CUDA program carries NVIDIA's runtime (`libcudart`, usually linked in), which hands
  each fat binary to the driver: `libcuda.so` in lambda, `nvcuda.dll` in atlas. Since we're the driver, we take the
  PTX from it and compile that. A kernel with no PTX fails its launch with an error that names it, never a crash.
- **Libraries made only of one vendor's machine code are replaced.** cuBLAS and cuDNN ship mostly SASS, so there's no
  PTX to compile. Like ZLUDA (also Rust), `gpu` brings its own: thin guest libraries under the same names, encoding
  onto the ring like the drivers, with kernels written in the IR and run host-side. cuBLAS first (PyTorch can't
  multiply matrices without it), then the parts of cuDNN PyTorch calls (it can also run without). A machine says at
  start which libraries it replaced.
- **Other compute APIs come with the rest.** llama.cpp's Vulkan backend, ncnn and IREE are SPIR-V compute over our
  Vulkan; DirectML is Direct3D 12.
- **ROCm and HIP are out.** Their binaries carry only AMD machine code, with no portable IR.

## Inside

```
gpu/
  src/              gpu: the device model, the contract the backends implement, the facade
    macos/          Metal, through the Objective-C runtime
    graphics.rs     images, samplers, graphics pipelines and draws
    software/       the CPU device: memory and its queue (mod.rs), draws (raster.rs), texels and sampling (texel.rs)
  ir/               gpu-ir: the IR, its validator, its text form (no_std)
  frontend/         gpu-frontend: spirv, dxbc and glsl (no_std); dxil, ptx, d3d9 to come
  backend/          gpu-backend: air (lower, bitcode, metallib), cpu; spirv to come
  examples/
    conformance.rs  the suite on every device this host has
    suite.rs        the suite itself, shared with tests/; its draw tests are in draws.rs, its DXBC ones in dxbc.rs
                    (with dxbc_asm.rs, a small assembler)
    programs/       its GLSL and HLSL, and the SPIR-V and DXBC build.sh makes of them with glslang and vkd3d's
                    compiler (checked in)
  tests/            the suite, the device's checks, the AIR libraries against Apple's tools, and GLSL against
                    glslang (glsl.rs, its programs and their SPIR-V in glsl/), in cargo test
```

Beside them, begun: `wire/` (`gpu-wire`, `no_std`: Vulkan's types, written by `wire/gen` from Khronos's `vk.xml` for
the commands the driver has, and the render node's commands; the ring to come), `api/` (`gpu-api`: the service thread
that owns a machine's device; Vulkan, Direct3D 11 and OpenGL lowered onto it, Direct3D 12 and CUDA to come), and
`guest/vulkan` and `guest/gl` (lambda's Vulkan driver and GL vendor library, `no_std` shared libraries built by
lambda's `build.rs` for ARM64 and x86-64 Linux). To come:
cuBLAS and cuDNN's kernels, and the Linux and Windows backends in folders of their own. The CPU device moves into a folder of its own as
rasterization joins it.

## Testing

`programs/build.sh` needs `glslangValidator` and vkd3d's `vkd3d-compiler` (on `PATH`, or `VKD3D_COMPILER=...`), only
for changing a program. On a Mac, vkd3d's release tarball builds the compiler alone: configure it with
`CPPFLAGS=-I/opt/homebrew/include` (SPIR-V's headers), a stub `libvulkan.dylib` on `LDFLAGS` (configure wants a Vulkan
loader for its Direct3D 12 library, which the compiler doesn't use), and Homebrew's bison (`BISON=...` for `make`); make
its generated headers (`include/private/vkd3d_version.h` and the `.tab.h` files) first, then `make vkd3d-compiler`.

The conformance suite (`examples/suite.rs`): each test's results are computed in Rust and compared word for word.

| Test | What it checks |
| --- | --- |
| copy | bindings, the global id, loads and stores |
| reduce | workgroup memory, barriers, a loop with a `continue` |
| atomics | atomic add and max on device memory from every invocation |
| subgroup | shuffles, xor shuffles and a subgroup sum |
| fences | three submissions ordered only by a fence, the first held until the host raises it; a fill and a copy |
| branches | divergent loops with early exits, signed arithmetic, a local array, floats, an early return |
| rules | the IR's own rules, written as IR: division by zero, shifts past the width, saturating conversions, scans, subgroup operations in divergent code, a workgroup atomic |
| more rules | compare-exchange, a barrier that only orders memory, 3-lane vectors in memory (and not the word after), 64-bit and boolean shuffles, booleans as floats, a float remainder, signed `MIN / -1` |
| triangles | the fill rule (two triangles sharing a diagonal through pixel centres, added together: each pixel drawn once), smooth and flat interpolation, the provoking vertex, discard, additive blending, two colour attachments |
| depth | three indexed quads (16-bit indices, the vertex offset choosing each) at depths 0.5, 0.25 and 0.75 with `Less`: the middle one seen and its depth kept; back-face culling with clockwise front faces |
| texture | a 4×4 texture through separate image and sampler bindings, two pixels a texel with nearest filtering, a triangle strip, the coordinate's derivatives and `gl_FragCoord` |
| formats | each new colour format drawn (flat) into level 1 of layer 1 of an image of two levels and two layers, and cleared into level 0 of layer 1, texel bytes exact; layer 0 kept; A8 refused as an attachment |
| mips | an 8×8 RGBA16 float texture's levels made by `GenerateMips`, read back exact; sampled at an explicit level of detail (`programs/levels.frag`) with nearest and linear mipmap filters and a sampler's range |
| border | `ClampToBorder` with opaque white around a 2×2 texture whose coordinates run past both edges |
| dxbc arith | a `cs_5_0` program's float arithmetic with modifiers and saturation, rounding, conversions (saturating), integer rules (`udiv` by zero, `imul`'s high word, shifts past 31), comparisons, `movc` and `dp4`, 30 words a thread |
| dxbc flow | a loop filling an indexable temp, a `switch` in a loop with a `continue`, fall-through into the default and the immediate constant buffer, if/else, and a conditional `ret` |
| dxbc shared | group-shared memory and `sync`, structured buffers in and out, `imm_atomic_iadd` and `atomic_umax` |
| dxbc draw | a textured quad from DXBC vertex and pixel shaders: a constant buffer, attributes by register, `sample`, `SV_Position` in the pixel shader and a derivative |
| hlsl arith, flow, shared, draw | the same from HLSL (`programs/arith.hlsl`, `flow.hlsl`, `shared.hlsl`, `quad.vs.hlsl`, `quad.ps.hlsl`) compiled by vkd3d's compiler: arith with its own 25 words a thread, the others giving the hand-assembled tests' results |
| cube | vkcube's shaders (`programs/cube.*`, as Vulkan-Tools writes them), one frame: its uniform buffer of an MVP matrix and 36 vertices, a combined image sampler, D16 depth, and an orthographic view of one face, whose every pixel is the light's 0.707 times the texture |

Passing on the CPU device, and on an Apple M1's GPU through Metal, draws included. A device without draws would
report them as not there yet (`--` in the example's output) rather than failing. `tests/device.rs` checks the device refuses bad
arguments, and that a program reading past its buffer faults on the CPU device; `tests/air.rs` that every suite
program's library passes Apple's `air-validate` and disassembles with `air-opt` (skipped without the Metal toolchain;
`GPU_AIR_DUMP=dir` keeps the files); `ir/src/tests.rs` checks what the validator refuses; `frontend/src/tests.rs` that
every program reads, and DXBC's signatures and refusals; `tests/air.rs` checks the DXBC programs' libraries too.
`tests/glsl.rs` holds the GLSL compiler to glslang: `tests/glsl/`'s programs (integers and control flow; float
built-ins and matrices; shared memory, barriers and atomics) compiled by both, glslang's SPIR-V (from `build.sh`, its
returns merged and functions inlined by `spirv-opt`) read by the SPIR-V frontend, run on the same input, and every
word compared (floats within 1e-5); the built-ins glslang's side can't be lowered for (inverse trigonometry,
hyperbolics, `determinant` and `inverse`, `refract`, `modf`, `isnan`) and returns from nested loops and `continue`
inside `switch` are checked against Rust; and a GLSL 1.10 program draws a textured quad placed by its uniforms. All
on the CPU device and the M1. `frontend/src/glsl/tests.rs` checks the preprocessor, the interface and linking, and
errors' lines. The suite is run without a machine; PTX programs join it
with their frontend.

## The order of work

Most useful first; each step ends in something a program can show.

**0. Programs' own software paths work.** Not `gpu` work, but before it: a program that brings its own software
renderer must run, as on a Windows machine without a GPU: the browser engine's ANGLE on its own SwiftShader under
atlas, Mesa's llvmpipe and lavapipe in lambda images. That's loader, registry and environment work in atlas. *Done
means:* the browser-engine program draws its window on `--gpu none`, and `vkcube` and `es2gears_wayland` run in
`debian:13`.

**1. The compiler, and compute on Metal.** Done: the IR, the SPIR-V frontend, the AIR and CPU backends, the device
model's compute half on Metal and the CPU device, the suite passing on both. Left from this step:

- Raw device pointers in Metal: a shader turning a 64-bit integer into a `device` pointer (Metal 3's buffer GPU
  addresses), with residency (`useResource`, residency sets) for memory reached only by pointer. CUDA and buffer
  device addresses need it, so it's checked before step 3. In AIR it's an `inttoptr` to `addrspace(1)`; the IR needs
  pointer casts first.
- Pipelines cached by hash in `CARBON_HOME`, so a program's second start builds nothing. A library's bytes are a
  fixed function of its program, so the hash of one is the key; Metal's own compile of AIR to the GPU's code is what
  the cache would save.
- The CPU device on more than one thread (a workgroup per thread, with device atomics as host atomics).
**2. lambda's Vulkan driver, compute only.** The wire, the render node, the guest ICD in a system layer, the Vulkan
lowering; push constants. *Done means:* `vulkaninfo` lists the Carbon GPU in `debian:13`, and llama.cpp's Vulkan
backend runs a small model on the Mac's GPU. Done so far: `--gpu` and `carbon gpu`; `gpu-api`'s service, a thread
that owns the device; `gpu-wire`, Vulkan's types generated from `vk.xml` and the render node's commands; lambda's
render node and its sysfs entry; a system layer of lambda's that lends every image the driver and its manifest; and
the driver's instance, physical device and device, images and buffers, which `vulkaninfo` lists with the M1's name
and the CPU device's; then compute: the driver's memory, buffers, descriptor sets, compute pipelines, command buffers,
queue and fences, sent as `gpu_wire::records` through the render node's doorbell and lowered onto the device by
`gpu_api::vulkan`, push constants included. Memory is the program's own, copied to the device for the ranges a
submission uses and back for those it may write, and a submission is finished when `vkQueueSubmit` returns.
`kernels/lambda/tests/programs/vulkan` checks 2,092 words on both devices. Left: llama.cpp's Vulkan backend, which
wants more of Vulkan (1.1 and its extensions, specialization constants, more of SPIR-V), memory without copies, and
asynchronous submissions.

**3. CUDA.** The `ptx` frontend, `libcuda.so`, then our cuBLAS. *Done means:* CUDA's `vectorAdd` sample, then
PyTorch's tensor operations on `cuda`, then a small model's inference, on the Mac's GPU from a Linux image.

**4. Graphics.** The device model's draw half: render pipelines, rasterization and blending on Metal and in the CPU
device, images and samplers, and window-system surfaces presenting to `display`. *Done means:* `vkcube` in
`debian:13`, then OpenGL of our own (below), then GTK 4 and Firefox with WebRender on it. Done so far: the IR's stages, the SPIR-V
frontend's vertex and fragment shaders, and the device model's draw half on the CPU device, vkcube's shaders drawing
their frame exactly; and lambda's Vulkan driver's graphics (images, samplers, render passes, framebuffers, graphics
pipelines, draws, copies between buffers and images) and its swapchains on Wayland and X11 (above): Debian's own
`vkcube` spins on `--display headless`, on `--wsi wayland` and `--wsi xcb`: at about 18 frames a second at 500×500 on
the CPU device, and drawn by the M1 through Metal (the AIR writer's vertex and fragment functions, Metal's render
pipelines and textures, held to the CPU device by the draw tests) at the display's 60 Hz. The device model's mip levels, array layers, more formats, border colours and comparison
samplers are done (for Direct3D 11 first; lambda's driver still makes images of one level and layer). Left: sampling
image arrays and views of parts of images; points (lines are drawn); GTK 4 and Firefox on our GL.

**OpenGL of our own.** `gpu_api::gl`, lowered on the host as Direct3D 11 is, for both kernels: lambda's through a
thin `libGLX_carbon.so.0` (and an EGL vendor) in the system layer, which the image's GLVND loads because lambda's X
server names `carbon` as the screen's vendor; atlas's through `opengl32` and WGL. A `glsl` frontend into the IR; GL
3.3 core and ES 3.0 first, then the compatibility profile's fixed function as programs ask. Calls are recorded in the
guest and sent on the ring, as every API's are; the state programs read back most is answered there. *Done means:*
Luanti, Debian's `minetest`, plays on the M1 with no Mesa in the image's path. Done so far:
- GLX in lambda's X server (configs, contexts, GLX drawables), with the image's Mesa (llvmpipe) as the oracle:
  `glxinfo`, `glxgears` and Luanti's main menu render through it.
- `libGLX_carbon.so.0` (`guest/gl`): GLVND's vendor ABI (`__glx_Main`), all of GLX 1.4 that GLVND requires (configs
  read from the server and chosen by GLX's rules, contexts, GLX windows, pixmaps and pbuffers made on the server,
  making current on a window, swaps through `PutImage`), and GL's first calls recorded onto the render node's GL
  doorbell (`gpu_wire::gl`).
- `gpu_api::gl`: contexts, each drawable's default framebuffer (rows kept as GL numbers them, turned up at a swap),
  clears whole, scissored and through the write masks, viewport, scissor, capabilities, `glGet*`, errors and
  `glReadPixels`. `kernels/lambda/tests/programs/gl` checks it on both devices: `glxinfo` says `Carbon GPU (Apple
  M1)`, and its 30 checks pass.
- The `glsl` frontend (above), held to glslang on both devices.
- GL 3.3's drawing (`api/src/gl/`): shaders compiled by the `glsl` frontend with GL's info logs, programs linked
  (attributes at their layout's, `glBindAttribLocation`'s or the next free location; the position moved to the
  device's depth range; `gl_FragData` kept to the framebuffer's attachments), uniforms by GL's locations and names
  (`glGetActiveUniform` too), buffers, vertex arrays with any of GL's attribute types (those the device doesn't read
  converted, and the program's own arrays read as the draw waits), generic attribute values, 2D textures from
  every 8-bit, packed, sRGB and float format GL has, mipmaps, filtering and wrapping, and draws, indexed or not,
  instanced, with depth, culling, blending and the write masks. Draws stay in one render pass until something has
  to wait; what each reads that can change (uniforms, copied arrays, converted indices) is copied into the batch's
  arena as it's recorded. `kernels/lambda/tests/programs/gl/draw.c` checks it on both devices (20 checks), and the
  host's own tests draw through the records on the CPU device.
- The compatibility profile (`api/src/gl/compat.rs`, `guest/gl/src/compat.rs`): GLSL 1.20's built-ins in the
  compiler (`gl_Vertex`, `gl_Color`, `gl_Normal`, `gl_MultiTexCoord<n>` at the generic attributes NVIDIA's drivers
  alias them to; the matrices, inverses and transposes, `gl_NormalMatrix` and `gl_TextureMatrix[]` as uniforms GL
  fills at each draw; `gl_FrontColor`, `gl_TexCoord[]` and `gl_FogFragCoord` passed on; `ftransform()`), and GL 2.1's
  state on the host: the matrix stacks, the conventional arrays and current values (as those attributes), each
  unit's texture environment (MODULATE, REPLACE, DECAL, BLEND, ADD and COMBINE's every function, source and operand),
  the alpha test, fog, and `glRectf`. Fixed function is GLSL: a program written from the state that shapes it,
  compiled by the `glsl` frontend and kept by that shape. `GL_EXTENSIONS` lists what's built (VBOs, non-power-of-two
  textures, the combine environments, BGRA arrays…). `kernels/lambda/tests/programs/gl/compat.c` checks it as
  Irrlicht draws (11 checks), and Luanti's main menu renders on it exactly as on Mesa.
- Framebuffer objects and renderbuffers (`api/src/gl/framebuffer.rs`): a texture's level or a renderbuffer as the
  colour attachment, a depth texture or renderbuffer (`D32Float`, whatever depth format is asked for) as the depth
  one, completeness, `glDrawBuffers` (one buffer, or none: a scratch colour image takes the outputs), and blits that
  copy without scaling; depth, half- and full-float textures. Lines and line strips and loops, a pixel wide, on both
  devices (the CPU device draws each as a quad). The compatibility profile's alpha test on a program's own shaders,
  as GL tests their fragments: Luanti's legacy driver leaves transparent texels to it. With these, Luanti plays in a
  world as it does on Mesa: the 3D view drawn into a framebuffer object and post-processed, the wielded hand, the
  selection box and the crosshair. The host's tests draw into a framebuffer object and sample it, draw lines, and
  test alpha.
- OpenGL ES 3.0 contexts (`GLX_EXT_create_context_es2_profile`; a request for 2.0 gets 3.0), with ES's version,
  GLSL ES and extension strings, and sRGB-capable configs on the X server, so SDL programs built for GLES start on it.
  ES 3.0's and GL 3.1's pieces built for them: uniform blocks (std140, bound at draws from the buffers at their binding
  points, read in place when aligned), `glBindBufferBase`/`Range`, mapped buffers (`glMapBufferRange` and its flushes,
  kept in the program and written back), sampler objects, up to four colour attachments with `glDrawBuffers` and
  `glClearBuffer*`, cube maps (six faces side by side in one image, `samplerCube` sampled by GL's face table half a
  texel inside each face), `texelFetch` and `textureSize` (a texel's centre sampled at its level, the size a hidden
  uniform GL fills), packed 10-10-10-2 vertex attributes, `glTexStorage2D`, `glDepthRange`, sync objects (a client wait
  finishes what's recorded), and the stencil calls taken (no stencil yet). The ES functions not built yet each say so
  once (`guest/gl/src/missing.rs`). SuperTuxKart (Debian's GLES build) races on it on the M1: menus, tracks, karts,
  its deferred lighting and the sky. The host's tests cover two draw buffers, a cube map, `texelFetch` and a uniform
  block.
- Left, in order: points (Metal needs the vertex shader's point size); 3D and array textures (the device samples one
  layer); stencil; constant blend colours; transform feedback and queries; drawing faster (draws batched across rings,
  not sent one by one); then the X server names `carbon` for machines with a GPU. Until then a program asks for it
  with `__GLX_VENDOR_LIBRARY_NAME=carbon`; GL says 3.3 and ES 3.0, of which this is what's there.

**5. atlas: DXGI, Direct3D 11 and Vulkan.** DXGI's factory, adapters and flip-model swap chains, the `dxbc` frontend
and the Direct3D 11 lowering, and `vulkan-1`. *Done means:* the browser-engine program on its default path (ANGLE on
Direct3D 11), with no flags. Done so far: atlas's machines get the machine's GPU (`atlas::Options.gpu`); the
device model's mip levels, array layers, formats and samplers Direct3D 11 needs first; and the `dxbc` frontend's
vertex, pixel and compute shaders, held to both devices by the suite. vkd3d's HLSL compiler is the
test oracle (`programs/build.sh`), and `gpu_api::d3d11` is Direct3D 11's devices and immediate contexts on the device.
atlas's DXGI and `d3d11.dll` are COM objects over it (`kernels/atlas/src/os/gfx.rs`), presenting into windows, with
a Direct3D 11 cube test (`kernels/atlas/tests/programs/cube.c`) on both devices. A browser engine's GPU process gets
through ANGLE's Direct3D 11 initialization. Next: the browser engine drawing on its default path.

**6. Linux and Windows hosts.** The `spirv` backend against the host's Vulkan driver; until then those hosts get the
CPU device. *Done means:* the conformance suite passes on each host's GPU.

**7. The rest:** Direct3D 12 with `dxil`, Direct3D 9, `nvcuda.dll` for Windows programs, cuDNN, the CPU device
compiled instead of interpreted.

**8. Faster:** zero-copy presentation to host windows, Vulkan Video and Media Foundation's hardware decode.

Steps 3 and 4 both build on 1 and 2 and not on each other, so they can swap: the browser-engine milestone wants 4 and
5 sooner, AI workloads want 3.

## Risks, checked first

Each of these could change a step, so each gets a small test before the step that depends on it:

1. **Raw device pointers in Metal** (above).
2. **What Metal lacks**: no geometry or tessellation shaders (emulated with compute stages before the vertex stage,
   as other Vulkan-on-Metal layers do), no 24-bit depth with stencil on Apple GPUs (32-bit instead), no lines or
   points wider than 1. Direct3D 11 programs use geometry and tessellation shaders often.
3. **Compile time**: no source is compiled, but Metal still compiles AIR to the GPU's code at pipeline creation,
   which can stall a first frame. The cache covers every start after the first; compiling in the background, before
   a pipeline is needed, covers the first.
4. **Structurizing**: optimizing compilers leave irreducible control flow in PTX and DXIL, and node splitting can
   grow code.
5. **PTX in the builds we target.** PyTorch's CUDA builds generally carry PTX for their newest architecture beside
   SASS for the rest; confirmed for each build before it's promised.
6. **CUDA's licence**: NVIDIA's EULA restricts running CUDA software on other hardware through translation layers.
   Implementing the driver API from its public documentation is what ZLUDA does; checked before the CUDA step ships.
7. **AIR is Apple's, and undocumented.** Everything `air` writes was learned from Apple's own compiler, and a new
   Metal toolchain could change its conventions. What holds it steady: libraries are pinned to AIR 2.5, which macOS
   26 still loads (apps ship metallibs built years before the OS that runs them), and `tests/air.rs` and the suite
   on the GPU show at once if a macOS stops taking what's written.

## Decisions to make

| Question | Options | Recommendation | Blocks |
| --- | --- | --- | --- |
| What the guest sees | a Carbon GPU; a known vendor's ID | a Carbon GPU, with a known vendor's ID as an option a machine turns on | step 2 |
| Its vendor ID | one of our own; virtio's; none (0) | `0x1c0de` for now (`gpu_wire::VENDOR_ID`): above 0xffff, in Khronos's range, so no PCI vendor's workarounds match it; Khronos's registered IDs are the way to make it official | step 2 |
| Where APIs are lowered | host-side, from records on the ring; in the guest drivers | host-side: native speed under translation, one place per API | step 2 |
| Vendor-only libraries (cuBLAS, cuDNN) | replaced by ours, by name, said at start; not supported | replaced: PyTorch can't multiply matrices without cuBLAS | step 3 |
| OpenGL | the image's Zink on our Vulkan; a GL of our own | Decided: our own. GL's driver is the OS's (rule 1); atlas needs one anyway, since Windows programs bring no Mesa; and Zink would ask more of our Vulkan driver, DRI3 of the X server, and translate every shader twice | step 4 |
| Windows host backend | Vulkan through the host's loader; Direct3D 12 | Vulkan first (every desktop GPU's driver has it), Direct3D 12 later | step 6 |
| CUDA or graphics first | step 3 before 4; 4 and 5 first | whichever milestone is closer: neither needs the other | step 3 |
| `--gpu auto`'s fallback | loud, as built; never fall back | the same answer as `--isolation auto`'s | the CLI |
| Several machines on one GPU | each its own device and queues; one shared, scheduled by Carbon | each its own: the host's driver already schedules between processes | step 2 |

## What we read, and don't link

Prior art for each part, read for how it's done and none of it a dependency:

| Project | What to learn from it |
| --- | --- |
| Mesa's NIR and its structurizer | an IR shaped for GPUs, and how many frontends share one |
| naga (wgpu) | a shader IR in Rust, and its writers |
| SPIRV-Cross, dxil-spirv | SPIR-V to other shading languages and DXIL to SPIR-V: the corner cases |
| [`air`](https://github.com/vertex-language/air) | AIR's format, the `.metallib` layout, and Apple's toolchain as the oracle: what `gpu-backend`'s `air` follows |
| DXVK, VKD3D-Proton | Direct3D 11's state and hazard tracking on an explicit API; Direct3D 12's binding model |
| MoltenVK, KosmicKrisp | what Metal lacks and how to emulate it (geometry and tessellation, formats) |
| ZLUDA | CUDA's driver API outside NVIDIA's driver, PTX in fat binaries, replacement libraries |
| lavapipe, llvmpipe, SwiftShader | a CPU device that's fast enough |
| crosvm's `rutabaga_gfx` | virtio-gpu's ioctls, if lambda ever answers them for an image's own Mesa Venus driver |

## Words

| Word | Meaning |
| --- | --- |
| **device** | `gpu::Device`: memory, pipelines, a queue and fences, on one backend |
| **CPU device** | The software backend: the IR interpreted on the host's CPU |
| **IR** | `gpu-ir`: every program between a frontend and a backend |
| **frontend** | Reads one API's GPU code into the IR: `spirv`, `dxbc`, later `dxil`, `ptx` |
| **backend** | Takes the IR somewhere: `air` for Metal, `cpu` for the CPU device, later `spirv` for Vulkan |
| **AIR** | Apple's GPU IR: LLVM IR in Apple's dialect, which Metal loads as `.metallib` libraries |
| **guest driver** | Carbon's driver in the program (a Vulkan ICD, `libcuda.so`, `d3d11`), encoding calls onto the ring |
| **wire** | The ring protocol the guest drivers write and the host reads |
| **Carbon GPU** | The device a guest sees: a vendor and device ID of Carbon's own, with the host's real limits |
