#!/bin/sh
# Compiles the conformance programs to SPIR-V with glslang. The .spv files are checked in, so only someone changing a
# program needs glslang; nothing at build or run time does.
set -e
cd "$(dirname "$0")"
for f in *.comp; do
    glslangValidator --quiet --target-env vulkan1.1 -o "${f%.comp}.spv" "$f"
done
for f in *.vert *.frag; do
    glslangValidator --quiet --target-env vulkan1.1 -o "$f.spv" "$f"
done
# And the DXBC programs from HLSL with vkd3d's compiler (vkd3d-compiler, from vkd3d's source; its HLSL is close to
# fxc's). Each .hlsl names its stage in its file name: *.vs.hlsl and *.ps.hlsl are vertex and pixel shaders, the rest
# compute kernels. Like the .spv files, the .dxbc files are checked in.
VKD3D_COMPILER="${VKD3D_COMPILER:-vkd3d-compiler}"
for f in *.hlsl; do
    case "$f" in
        *.vs.hlsl) profile=vs_5_0 ;;
        *.ps.hlsl) profile=ps_5_0 ;;
        *) profile=cs_5_0 ;;
    esac
    "$VKD3D_COMPILER" -x hlsl -b dxbc-tpf -p "$profile" -o "${f%.hlsl}.dxbc" "$f"
done
