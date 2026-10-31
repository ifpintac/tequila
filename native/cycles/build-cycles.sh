#!/usr/bin/env bash
# Build Cycles (Apache-2.0) with Tequila's C interface: on a Mac,
# libtequila_cycles.dylib for Apple silicon with Metal; on Linux,
# libtequila_cycles.so for x86-64 with CUDA, OptiX and HIP.
#
# The counterpart of build-cycles.ps1, from the same steps: Cycles from the
# standalone repository at v5.0.0, its engine sources brought to Blender
# 5.0.1's, and only the precompiled libraries it needs, at the commit Cycles
# pins. On a Mac there are no GPU kernels to ship: Cycles compiles its Metal
# kernels from source at the first GPU render, which is why the kernel
# sources are copied beside the library. On Linux the GPU kernels are
# Blender 5.0.1's own, which these sources match (the Linux release
# workflow fetches them, as Windows' does).
#
# Usage:  bash native/cycles/build-cycles.sh [work-folder]
# Needs:  a Mac: Xcode Command Line Tools, Git with LFS, Python 3, CMake and
#         Ninja (Homebrew). Linux: GCC 11 or newer, Git with LFS, Python 3,
#         CMake and Ninja. Both: Tequila's own SDKs
#         (bash scripts/fetch-third-party.sh).
# Result: <work>/cycles/build/bin/ with the library and its libraries.
#         Point TEQUILA_CYCLES at that folder, or package it
#         (scripts/package-macos.sh puts it in Contents/Frameworks/cycles,
#         scripts/package-linux.sh in cycles/ beside the programs).
set -euo pipefail

shim="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$shim/../.." && pwd)"
work="${1:-$(cd "$repo/.." && pwd)/cycles-spike}"
mkdir -p "$work"
cd "$work"

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64) platform=macos_arm64 ;;
    Linux-x86_64) platform=linux_x64 ;;
    *) echo "This builds Cycles for Apple silicon Macs and x86-64 Linux; on Windows use build-cycles.ps1." >&2
       exit 1 ;;
esac

# 1. Cycles, standalone, v5.0.0.
if [ ! -d cycles ]; then
    git clone --depth 1 --branch v5.0.0 https://github.com/blender/cycles.git cycles
fi

# 2. Only the precompiled libraries Cycles needs, at the commit it pins.
#    Their files are in Git LFS. The folders are the ones the Windows build
#    takes, matched without regard to case - the Mac's are named in lower
#    case (openimageio), Windows' not always (OpenImageIO) - and sse2neon,
#    which Cycles translates its SSE code to Apple's NEON with. A folder
#    added to the list is fetched on the next run. Set
#    TEQUILA_CYCLES_ALL_LIBS=1 to take every folder (several GB) if CMake
#    asks for one not in the list.
git lfs version >/dev/null 2>&1 || { echo "Git LFS is needed: brew install git-lfs && git lfs install" >&2; exit 1; }
lib="$work/lib/$platform"
commit="$(git -C cycles ls-tree HEAD "lib/$platform" | awk '{print $3}')"
[ -n "$commit" ] || { echo "Cycles does not pin lib/$platform at this tag" >&2; exit 1; }
if [ "$(git -C "$lib" rev-parse -q --verify "$commit^{commit}" 2>/dev/null)" != "$commit" ]; then
    rm -rf "$lib"
    mkdir -p "$lib"
    git -C "$lib" init -q
    git -C "$lib" remote add origin "https://projects.blender.org/blender/lib-$platform.git"
    git -C "$lib" fetch -q --depth 1 --filter=blob:none origin "$commit"
fi
if [ "${TEQUILA_CYCLES_ALL_LIBS:-}" = "1" ]; then
    git -C "$lib" sparse-checkout disable
else
    wanted='^(openimageio|pugixml|openjpeg|jpeg|tiff|webp|png|zlib|openexr|imath|tbb|embree|openimagedenoise|zstd|opencolorio|openpgl|deflate|libdeflate|sse2neon)$'
    folders="$(git -C "$lib" ls-tree --name-only -d "$commit" | grep -i -E "$wanted")"
    echo "Library folders: $(echo $folders)"
    missing=0
    for folder in $folders; do
        [ -d "$lib/$folder" ] || missing=1
    done
    if [ "$missing" = 1 ]; then
        # shellcheck disable=SC2046
        git -C "$lib" sparse-checkout set --no-cone $(printf '/%s/ ' $folders) /.gitattributes
        git -C "$lib" checkout -q "$commit"
    fi
fi
if [ ! -L "cycles/lib/$platform" ]; then
    rm -rf "cycles/lib/$platform"
    ln -s "$lib" "cycles/lib/$platform"
fi

# 3. Engine sources at Blender 5.0.1, leaving the standalone build files and
#    its eager node registration (graph/node_type.h) alone.
if [ ! -d blender501/intern/cycles ]; then
    git clone -q --filter=blob:none --no-checkout --depth 1 --branch v5.0.1 https://github.com/blender/blender.git blender501
    git -C blender501 sparse-checkout set --no-cone /intern/cycles/
    git -C blender501 checkout -q v5.0.1
fi
python3 - <<'PYTHON'
import os, shutil
A, B = 'cycles/src', 'blender501/intern/cycles'
keep = lambda p: p.endswith('CMakeLists.txt') or p.startswith(('cmake/', 'app/', 'hydra/')) or p in ('util/guarded_allocator.h', 'graph/node_type.h')
for d, _, fs in os.walk(B):
    for f in fs:
        src = os.path.join(d, f); p = os.path.relpath(src, B).replace(os.sep, '/'); dst = os.path.join(A, p)
        if os.path.exists(dst) and not keep(p) and open(src, 'rb').read().replace(b'\r\n', b'\n') != open(dst, 'rb').read().replace(b'\r\n', b'\n'):
            shutil.copyfile(src, dst)
PYTHON

# 4. The shim target, once (the same block build-cycles.ps1 adds).
app=cycles/src/app/CMakeLists.txt
if ! grep -q tequila_cycles "$app"; then
    cat >> "$app" <<'CMAKE'

# Tequila's C interface over Cycles, built with the same flags and libraries
# as the standalone app. TEQUILA_SHIM_DIR points at its sources.
if(TEQUILA_SHIM_DIR)
  add_library(tequila_cycles SHARED
    ${TEQUILA_SHIM_DIR}/tequila_cycles.cpp
    ${TEQUILA_SHIM_DIR}/tequila_cycles.h
  )
  target_link_libraries(tequila_cycles PRIVATE ${LIB})
endif()
CMAKE
fi
# Tequila's own block, not any: a work folder that built Cycles for
# Freeshot, as Tequila was called, has Freeshot's, and taking that for this
# one left the library without OpenColorIO, which it refuses to render
# without.
if ! grep -q "tequila_cycles PRIVATE FSC_WITH_OCIO" "$app"; then
    cat >> "$app" <<'CMAKE'

# OpenColorIO for the shim itself: it sets the config Cycles works in.
if(TEQUILA_SHIM_DIR AND WITH_OPENCOLORIO)
  target_include_directories(tequila_cycles PRIVATE ${OPENCOLORIO_INCLUDE_DIRS})
  target_link_libraries(tequila_cycles PRIVATE ${OPENCOLORIO_LIBRARIES})
  target_compile_definitions(tequila_cycles PRIVATE FSC_WITH_OCIO)
endif()
CMAKE
fi

# 4b. On Linux, NVIDIA's OptiX headers, at the version Blender 5.0.1's
#     kernels were built with (as build-cycles.ps1 takes them).
optix="$work/optix-8.0.0"
if [ "$platform" = linux_x64 ] && [ ! -f "$optix/include/optix.h" ]; then
    rm -rf "$optix"
    git clone -q --depth 1 --branch v8.0.0 https://github.com/NVIDIA/optix-dev.git "$optix"
fi

# 5. Configure and build. On a Mac: Metal, arm64, the dylib finding its
#    libraries beside itself (@loader_path). On Linux: CUDA and OptiX for
#    NVIDIA cards and HIP for AMD ones, all three loaded at run time so no
#    vendor's toolkit is needed here or on the computer it runs on, and the
#    library finding its own beside itself ($ORIGIN).
if [ "$platform" = macos_arm64 ]; then
    devices=(-DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_INSTALL_RPATH=@loader_path
             -DWITH_CYCLES_DEVICE_METAL=ON
             -DWITH_CYCLES_DEVICE_CUDA=OFF -DWITH_CYCLES_DEVICE_OPTIX=OFF -DWITH_CYCLES_DEVICE_HIP=OFF -DWITH_CYCLES_DEVICE_ONEAPI=OFF)
else
    devices=('-DCMAKE_INSTALL_RPATH=$ORIGIN'
             -DWITH_CYCLES_DEVICE_CUDA=ON -DWITH_CUDA_DYNLOAD=ON -DWITH_CYCLES_CUDA_BINARIES=OFF
             -DWITH_CYCLES_DEVICE_OPTIX=ON "-DOPTIX_ROOT_DIR=$optix"
             -DWITH_CYCLES_DEVICE_HIP=ON -DWITH_HIP_DYNLOAD=ON -DWITH_CYCLES_HIP_BINARIES=OFF -DWITH_CYCLES_DEVICE_HIPRT=OFF
             -DWITH_CYCLES_DEVICE_ONEAPI=OFF -DWITH_CYCLES_DEVICE_METAL=OFF)
fi
cd cycles
cmake -G Ninja -B build \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
    -DWITH_CYCLES_ALEMBIC=OFF -DWITH_CYCLES_OPENSUBDIV=OFF -DWITH_CYCLES_OPENVDB=OFF -DWITH_CYCLES_NANOVDB=OFF \
    -DWITH_CYCLES_OSL=OFF -DWITH_CYCLES_USD=OFF -DWITH_CYCLES_HYDRA_RENDER_DELEGATE=OFF -DWITH_CYCLES_STANDALONE_GUI=OFF \
    "${devices[@]}" \
    -DWITH_STRICT_BUILD_OPTIONS=ON -DWITH_CYCLES_PATH_GUIDING=ON \
    -DTEQUILA_SHIM_DIR="$shim"
cmake --build build --target tequila_cycles
cd ..

# 6. Beside the library: the shared libraries of Blender's it loads, with
#    Tequila's own Embree, TBB and OIDN in place of Blender's, so one copy
#    of each is loaded (as on Windows).
bin="$work/cycles/build/bin"
# CMake puts a shared library in build/lib on a Mac and on Linux (build/bin
# holds DLLs on Windows): it is gathered into bin with what it loads.
mkdir -p "$bin"
if [ "$platform" = macos_arm64 ]; then
    built=libtequila_cycles.dylib
    cp -a "$work/cycles/build/lib/$built" "$bin/"
    find -L "$lib" -path '*/lib/*.dylib' | grep -v -i -E '/(embree|tbb|openimagedenoise)/' | while read -r dylib; do
        cp -a "$dylib" "$bin/"
    done
    for sdk in "$repo"/third_party/embree-*.arm64.macosx "$repo"/third_party/oidn-*.arm64.macos; do
        if [ -d "$sdk/lib" ]; then cp -a "$sdk"/lib/*.dylib "$bin/"; fi
    done
    # Anything the library still looks for outside its own folder would be
    # missing on another Mac, or after the build folder is gone: list it.
    echo "Libraries $built loads (all should be @rpath/@loader_path or /usr/lib, /System):"
    otool -L "$bin/$built" | tail -n +2

    # 7. The kernel sources the Metal device compiles from at the first GPU
    #    render (Cycles' `source` folder, beside the library).
    rm -rf "$bin/source"
    mkdir -p "$bin/source"
    cp -R cycles/src/kernel "$bin/source/kernel"
    cp -R cycles/src/util "$bin/source/util"
else
    built=libtequila_cycles.so
    found="$(find "$work/cycles/build" -name "$built" -type f | head -1)"
    [ -n "$found" ] || { echo "$built was not built" >&2; exit 1; }
    cp -a "$found" "$bin/"
    # Blender's Linux libraries are mostly static; the shared ones it links
    # are carried, with their symbolic links.
    find -L "$lib" -path '*/lib/*.so*' | grep -v -i -E '/(embree|tbb|openimagedenoise)/' | while read -r shared; do
        cp -a "$shared" "$bin/"
    done
    for sdk in "$repo"/third_party/embree-*.x86_64.linux "$repo"/third_party/oidn-*.x86_64.linux; do
        if [ -d "$sdk/lib" ]; then cp -a "$sdk"/lib/*.so* "$bin/"; fi
    done
    if command -v patchelf >/dev/null; then
        for library in "$bin"/*.so*; do
            if [ -f "$library" ] && [ ! -L "$library" ]; then patchelf --set-rpath '$ORIGIN' "$library"; fi
        done
    fi
    # What the library finds outside its own folder should be the system's
    # own (libc, libstdc++, libm, libdl): anything else would be missing on
    # another computer.
    echo "Libraries $built loads, and from where:"
    LD_LIBRARY_PATH="$bin" ldd "$bin/$built" || true
fi

echo "Built: $bin/$built"
