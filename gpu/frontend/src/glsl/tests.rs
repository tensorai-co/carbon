//! The GLSL compiler's own tests: what compiles (and validates), what the interface says, what linking does, the
//! preprocessor, and errors with their lines. Running what's compiled, against glslang's SPIR-V of the same source,
//! is `gpu/tests/glsl.rs`'s.

use alloc::string::String;

use super::*;
use gpu_ir::Slot;

fn ok(source: &str, stage: Stage) -> Shader {
    match compile(source, stage, false) {
        Ok(s) => s,
        Err(e) => panic!("{e}\n{source}"),
    }
}

fn fails(source: &str, stage: Stage) -> String {
    match compile(source, stage, false) {
        Ok(_) => panic!("compiled, and shouldn't have:\n{source}"),
        Err(e) => e.message,
    }
}

const CLASSIC_VS: &str = "
attribute vec3 position;
attribute vec2 uv;
uniform mat4 mvp;
varying vec2 v_uv;
void main() {
    v_uv = uv;
    gl_Position = mvp * vec4(position, 1.0);
}
";

const CLASSIC_FS: &str = "
uniform sampler2D tex;
uniform vec4 tint;
varying vec2 v_uv;
void main() {
    gl_FragColor = texture2D(tex, v_uv) * tint;
}
";

#[test]
fn glsl_110_vertex_and_fragment() {
    let v = ok(CLASSIC_VS, Stage::Vertex);
    assert_eq!(v.version, Version { number: 110, es: false });
    let names: Vec<_> = v.interface.inputs.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["position", "uv"]);
    assert_eq!(v.interface.uniforms.len(), 1);
    assert_eq!((v.interface.uniforms[0].name.as_str(), v.interface.uniforms[0].offset, v.interface.uniforms[0].matrix_stride), ("mvp", 0, 16));
    assert_eq!(v.interface.default_block.map(|d| d.1), Some(64));
    assert!(v.entry.outputs.iter().any(|o| o.slot == Slot::Position));
    let f = ok(CLASSIC_FS, Stage::Fragment);
    let sampler = f.interface.uniforms.iter().find(|u| u.name == "tex").unwrap();
    assert!(sampler.binding.is_some());
    let tint = f.interface.uniforms.iter().find(|u| u.name == "tint").unwrap();
    assert_eq!(tint.offset, 0);
    let p = link(&v, &f).unwrap();
    let out = v.interface.outputs.iter().find(|o| o.name == "v_uv").unwrap().io[0];
    let input = f.interface.inputs[0].io[0];
    assert_eq!(p.vertex.outputs[out as usize].slot, p.fragment.inputs[input as usize].slot);
    assert_eq!(p.samplers, [("tex".into(), 0)]);
    // The two default blocks are bound apart.
    let default = |e: &gpu_ir::Entry| e.bindings.iter().find(|b| b.set == SET_DEFAULT).unwrap().binding;
    assert_eq!((default(&p.vertex), default(&p.fragment)), (0, 1));
}

#[test]
fn glsl_330_core_with_blocks_and_locations() {
    let vs = "#version 330 core
layout(location = 2) in vec4 pos;
layout(std140) uniform Camera { mat4 view; vec3 eye; float near; } camera;
out vec3 dir;
flat out int id;
void main() {
    dir = pos.xyz - camera.eye;
    id = gl_VertexID;
    gl_Position = camera.view * pos;
}";
    let v = ok(vs, Stage::Vertex);
    assert_eq!(v.interface.inputs[0].location, Some(2));
    let b = &v.interface.blocks[0];
    assert_eq!(b.name, "Camera");
    let offsets: Vec<u32> = b.members.iter().map(|m| m.2).collect();
    assert_eq!(offsets, [0, 64, 76]);
    assert_eq!(b.size, 80);
    let fs = "#version 330 core
in vec3 dir;
flat in int id;
layout(location = 0) out vec4 color;
void main() { color = vec4(normalize(dir), float(id)); }";
    let f = ok(fs, Stage::Fragment);
    link(&v, &f).unwrap();
}

#[test]
fn glsl_es_100_and_300() {
    let fs = "precision mediump float;
varying lowp vec4 c;
void main() { gl_FragColor = c; }";
    let f = compile(fs, Stage::Fragment, true).unwrap();
    assert_eq!(f.version, Version { number: 100, es: true });
    let fs3 = "#version 300 es
precision highp float;
in vec2 uv;
out vec4 color;
uniform sampler2D s;
void main() { color = texture(s, uv); }";
    ok(fs3, Stage::Fragment);
    // ES has no implicit conversions.
    assert!(fails("#version 300 es\nvoid main() { float x = 1; }", Stage::Vertex).contains("int where float"));
}

#[test]
fn the_language() {
    // Functions with every kind of parameter, early returns from loops, structs, arrays, switch, matrices.
    let src = "#version 430
layout(local_size_x = 8) in;
layout(std430, binding = 0) buffer Out { uint words[]; };
struct Pair { int a; float b[2]; };
const int N = 4;
int find(int xs[N], int x) {
    for (int i = 0; i < N; i++) {
        if (xs[i] == x) return i;
    }
    return -1;
}
void bump(inout int x, out float y) { x += 1; y = float(x) * 0.5; }
Pair make(int a) { Pair p; p.a = a; p.b[0] = 1.0; p.b[1] = 2.0; return p; }
void main() {
    uint i = gl_GlobalInvocationID.x;
    int xs[N] = int[N](3, 1, 4, 1);
    int k = find(xs, int(i));
    float y;
    bump(k, y);
    Pair p = make(k);
    mat2 m = mat2(1.0, 2.0, 3.0, 4.0) * mat2(p.b[0]);
    switch (k) {
        case 0: k = 10;
        case 1: k += 1; break;
        default: k = 99;
    }
    vec3 v = vec3(m[1], y).zyx;
    words[i] = uint(k) + uint(v.x * 100.0) + uint(xs[int(i) % N]);
}";
    let s = ok(src, Stage::Compute);
    assert_eq!(s.entry.workgroup_size, [8, 1, 1]);
    assert_eq!(s.interface.blocks[0].explicit, Some(0));
}

#[test]
fn the_preprocessor() {
    let src = "#version 120
#define SQUARE(x) ((x) * (x))
#define TWO 2
#if TWO > 1 && defined(SQUARE)
#define PICK 1
#elif 0
#error not this
#else
#define PICK 3
#endif
#ifdef GL_ES
fails to parse
#endif
float f = float(SQUARE(TWO)) + float(PICK) + float(__LINE__) \\
    + 0.0;
void main() { gl_FragColor = vec4(f); }";
    ok(src, Stage::Fragment);
    assert!(fails("#version 110\n#error stop here\nvoid main() {}", Stage::Fragment).contains("0:2: error: #error stop here"));
    assert!(fails("void main() {}\n#version 110", Stage::Fragment).contains("#version must come first"));
    assert!(fails("#if 1\nvoid main() {}", Stage::Fragment).contains("#endif"));
}

#[test]
fn errors_say_their_line() {
    let e = fails("void main() {\n    float x = 1.0;\n    x = y;\n}", Stage::Fragment);
    assert!(e.starts_with("0:3: error:") && e.contains("y isn't declared"), "{e}");
    let e = fails("uniform float u;\nvoid main() {\n  u = 1.0;\n}", Stage::Fragment);
    assert!(e.starts_with("0:3:") && e.contains("can't be assigned"), "{e}");
    let e = fails("void f();\nvoid main() { f(); }", Stage::Fragment);
    assert!(e.contains("never defined"), "{e}");
    let e = fails("int f(int x) { return f(x); }\nvoid main() { f(1); }", Stage::Fragment);
    assert!(e.contains("calls itself"), "{e}");
    let e = fails("void main() { break; }", Stage::Fragment);
    assert!(e.contains("break outside"), "{e}");
    let e = fails("void f() { vec3 v = vec2(1.0); }\nvoid main() {}", Stage::Fragment);
    assert!(e.starts_with("0:1:") && e.contains("vec2 where vec3"), "{e}");
    let e = fails("void main() { gl_FragColor = vec4(1.0) }", Stage::Fragment);
    assert!(e.contains("`;` expected"), "{e}");
    assert!(fails("float x;", Stage::Fragment).contains("no main"));
}

#[test]
fn the_compatibility_profile() {
    // Luanti's header: GLSL 1.20 with Irrlicht's names for the compatibility profile's built-ins.
    let vs = "#version 120
#define mWorldViewProj gl_ModelViewProjectionMatrix
#define mTexture (gl_TextureMatrix[0])
#define inVertexPosition gl_Vertex
#define inVertexColor gl_Color
#define inTexCoord0 gl_MultiTexCoord0
varying vec4 varColor;
varying vec2 uv;
void main() {
    gl_Position = mWorldViewProj * inVertexPosition;
    varColor = inVertexColor;
    uv = (mTexture * inTexCoord0).st;
    gl_TexCoord[0] = gl_MultiTexCoord0;
    gl_FrontColor = gl_Color;
}";
    let v = ok(vs, Stage::Vertex);
    let at = |name: &str| v.interface.inputs.iter().find(|i| i.name == name).unwrap().location;
    assert_eq!((at("gl_Vertex"), at("gl_Color"), at("gl_MultiTexCoord0")), (Some(0), Some(3), Some(8)));
    let names: Vec<&str> = v.interface.uniforms.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["gl_ModelViewProjectionMatrix", "gl_TextureMatrix"]);
    let fs = "#version 120
varying vec4 varColor;
varying vec2 uv;
void main() { gl_FragColor = varColor * gl_Color * gl_TexCoord[0]; }";
    let f = ok(fs, Stage::Fragment);
    link(&v, &f).unwrap();
    // ftransform(), and a fragment shader reading gl_Color its vertex shader didn't write.
    let v2 = ok("void main() { gl_Position = ftransform(); }", Stage::Vertex);
    assert!(link(&v2, &ok("void main() { gl_FragColor = gl_Color; }", Stage::Fragment)).unwrap_err().message.contains("gl_Color isn't written"));
    // Not in the core profile, nor in ES.
    assert!(fails("#version 330 core\nvoid main() { gl_Position = gl_Vertex; }", Stage::Vertex).contains("gl_Vertex"));
    ok("#version 150 compatibility\nvoid main() { gl_Position = gl_Vertex; }", Stage::Vertex);
}
