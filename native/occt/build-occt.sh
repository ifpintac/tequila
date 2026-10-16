#!/usr/bin/env bash
# Build tequila-occt on a Mac against Homebrew's OpenCASCADE, and gather it
# with the libraries it loads into native/occt/build/occt - the folder
# scripts/package-macos.sh puts in Tequila.app/Contents/MacOS/occt.
#
# The counterpart of build-occt.bat. Windows builds against OCCT 8.0.1's
# release; Homebrew had 7.9 in October 2026, which has everything the helper
# uses (DESTEP_Parameters arrived in 7.8). Without the helper, Tequila reads
# STEP with truck, its own reader, which gets some CAD packages' files wrong.
#
# Written on Windows and not yet run on a Mac: expect to adjust it once.
#
# Usage:  bash native/occt/build-occt.sh
# Needs:  Xcode Command Line Tools; brew install opencascade dylibbundler
#         (or OCCT_DIR pointing at an OpenCASCADE install).
# Result: native/occt/build/occt/tequila-occt, its libraries in libs/, and
#         their licences in licenses/.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out="$here/build/occt"
if [ "$(uname -s)" != "Darwin" ]; then
    echo "This builds for macOS; on Windows use build-occt.bat." >&2
    exit 1
fi
occt="${OCCT_DIR:-$(brew --prefix opencascade 2>/dev/null || true)}"
if [ ! -d "$occt/include/opencascade" ]; then
    echo "OpenCASCADE not found: brew install opencascade, or set OCCT_DIR" >&2
    exit 1
fi
command -v dylibbundler >/dev/null || { echo "dylibbundler not found: brew install dylibbundler" >&2; exit 1; }

rm -rf "$out"
mkdir -p "$out/libs" "$out/licenses"

# The same libraries build-occt.bat links.
libs=(TKernel TKMath TKG2d TKG3d TKGeomBase TKBRep TKTopAlgo TKMesh TKShHealing TKXSBase TKDE TKDESTEP TKXCAF TKLCAF TKCAF TKCDF TKRWMesh TKService TKV3d TKVCAF)
link=()
for name in "${libs[@]}"; do link+=("-l$name"); done
clang++ -std=c++17 -O2 -DNDEBUG \
    -I"$occt/include/opencascade" "$here/tequila_occt.cpp" \
    -L"$occt/lib" "${link[@]}" \
    -o "$out/tequila-occt"

# Every library it loads that is not the system's, copied into libs/ and
# pointed at from there; dylibbundler signs what it changes (ad hoc).
dylibbundler -od -b -cd -x "$out/tequila-occt" -d "$out/libs/" -p @executable_path/libs/ \
    -s "$occt/lib" -s "$(brew --prefix)/lib"

# Their licence texts: OpenCASCADE's (LGPL 2.1 with its exception) and each
# library's that came with it, by the Homebrew package it is from.
for formula in opencascade $(brew deps --installed opencascade 2>/dev/null); do
    prefix="$(brew --prefix "$formula" 2>/dev/null || true)"
    [ -d "$prefix" ] || continue
    found=()
    while IFS= read -r file; do found+=("$file"); done < <(find "$prefix/" -maxdepth 1 -type f \( -iname 'LICENSE*' -o -iname 'LICENCE*' -o -iname 'COPYING*' -o -iname '*LGPL*' -o -iname '*EXCEPTION*' -o -iname 'NOTICE*' \))
    if [ "${#found[@]}" -gt 0 ]; then
        mkdir -p "$out/licenses/$formula"
        cp "${found[@]}" "$out/licenses/$formula/"
    fi
done

# The one check that matters: it reads a STEP file.
fixture="$here/../../crates/tequila-import/tests/data/two_bodies.step"
answer="$(mktemp -t tequila-occt)"
if "$out/tequila-occt" "$fixture" "$answer" 0.002 0.5 >/dev/null && [ -s "$answer" ]; then
    echo "Built and checked: $out/tequila-occt"
else
    echo "Built, but it did not read $fixture" >&2
    exit 1
fi
rm -f "$answer"
