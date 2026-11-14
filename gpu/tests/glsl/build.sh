#!/bin/sh
# The oracle's side of gpu/tests/glsl.rs: each program here compiled by glslang to SPIR-V, its returns merged and functions inlined by
# spirv-opt (gpu's SPIR-V frontend takes no calls), checked in beside it. Carbon's GLSL compiler reads the source.
set -e
cd "$(dirname "$0")"
for p in *.comp; do
    glslangValidator --quiet -V --target-env vulkan1.0 -o "$p.tmp.spv" "$p"
    spirv-opt --merge-return --inline-entry-points-exhaustive --eliminate-dead-functions "$p.tmp.spv" -o "$p.spv"
    rm "$p.tmp.spv"
done
