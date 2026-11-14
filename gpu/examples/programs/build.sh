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
