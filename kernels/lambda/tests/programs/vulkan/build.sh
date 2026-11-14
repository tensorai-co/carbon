#!/bin/sh
# Compiles vulkan.c's kernel to SPIR-V with glslang, as a C array (compute.h, checked in), so building vulkan.c needs
# only a C compiler and Vulkan's headers: it's built inside a debian:13 machine (see vulkan.c).
set -e
cd "$(dirname "$0")"
glslangValidator --quiet --target-env vulkan1.0 -o compute.spv compute.comp
python3 - <<'PY'
import struct
data = open("compute.spv", "rb").read()
words = struct.unpack("<%dI" % (len(data) // 4), data)
rows = ["    " + ", ".join("0x%08x" % w for w in words[i:i + 8]) + "," for i in range(0, len(words), 8)]
open("compute.h", "w").write("// compute.comp as SPIR-V, written by build.sh with glslang: not edited by hand.\n"
                             "static const uint32_t compute_spv[] = {\n" + "\n".join(rows) + "\n};\n")
PY
rm compute.spv
