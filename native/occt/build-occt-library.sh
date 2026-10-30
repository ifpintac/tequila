#!/usr/bin/env bash
# Build OpenCASCADE 8.0.1 itself on Linux, for build-occt.sh to build the
# STEP helper against: Windows builds against 8.0.1's release and a Mac
# against Homebrew's, but the Linux distributions Tequila runs on carry 7.5
# or 7.6, older than the helper needs (DESTEP_Parameters arrived in 7.8).
#
# Only what the helper links: the foundation, modelling and data exchange
# libraries and the application framework, with the visualization ones
# XCAF's colours come through. No Draw (Tcl/Tk), no OpenGL, no samples.
#
# Usage:  bash native/occt/build-occt-library.sh [work-folder]
# Needs:  GCC 11 or newer, CMake, Ninja, and FreeType's and X11's
#         development files (apt install libfreetype-dev libx11-dev).
# Result: <work>/install, with include/opencascade and lib. Point OCCT_DIR
#         at it for build-occt.sh.
set -euo pipefail

if [ "$(uname -s)" != "Linux" ]; then
    echo "This builds OpenCASCADE for Linux; on a Mac use Homebrew's (brew install opencascade)." >&2
    exit 1
fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
work="${1:-$here/build/opencascade}"
version=8_0_1
mkdir -p "$work"
cd "$work"

if [ -f install/include/opencascade/Standard.hxx ] && [ -f "install/.built-$version" ]; then
    echo "OpenCASCADE ${version//_/.} already built in $work/install"
    exit 0
fi

if [ ! -d "OCCT-$version" ]; then
    curl -fsSL "https://github.com/Open-Cascade-SAS/OCCT/archive/refs/tags/V$version.tar.gz" -o occt.tar.gz
    tar -xzf occt.tar.gz
    rm occt.tar.gz
fi

cmake -G Ninja -S "OCCT-$version" -B build \
    -DCMAKE_BUILD_TYPE=Release \
    -DINSTALL_DIR="$work/install" \
    -DCMAKE_INSTALL_PREFIX="$work/install" \
    -DBUILD_LIBRARY_TYPE=Shared \
    -DBUILD_MODULE_Draw=OFF \
    -DBUILD_DOC_Overview=OFF \
    -DBUILD_SAMPLES_QT=OFF \
    -DUSE_TK=OFF -DUSE_OPENGL=OFF -DUSE_GLES2=OFF -DUSE_FREEIMAGE=OFF -DUSE_FFMPEG=OFF \
    -DUSE_OPENVR=OFF -DUSE_RAPIDJSON=OFF -DUSE_DRACO=OFF -DUSE_TBB=OFF -DUSE_VTK=OFF \
    -DUSE_FREETYPE=ON -DUSE_XLIB=ON
cmake --build build
cmake --install build
touch "install/.built-$version"
echo "Built OpenCASCADE ${version//_/.} into $work/install"
