#!/usr/bin/env bash
# Package Tequila for Linux on x86-64: a folder to unpack anywhere, and a
# .tar.gz of it - the counterpart of package-windows.ps1 and
# package-macos.sh. Updating is unpacking the newer one in its place: your
# materials, plugins and settings live outside it ($XDG_CONFIG_HOME/Tequila,
# or ~/.config/Tequila), so they stay.
#
# What goes where (everything beside the program, as on Windows):
#   Tequila, tequila-cli   the application and the command-line renderer
#                          (where tequila.open() and Connect an AI look)
#   lib*.so*               Embree, TBB and Open Image Denoise, found by the
#                          programs' rpath ($ORIGIN, .cargo/config.toml)
#   assets/                the material library, studio presets, samples
#   python/, plugins/      the scripting module and the example plugins
#   install.sh             puts Tequila in the desktop's application menu
#   BUILD.txt, licences    which version this is, and whose code is in it
#
# Renders use Tequila's own renderer, and STEP files are read by its own
# reader: Cycles and OpenCASCADE's reader are not built for Linux yet.
#
# Usage:  bash scripts/package-linux.sh [--skip-build]
# Result: dist/Tequila-Linux-<stamp>/Tequila and
#         dist/Tequila-<version>-linux-x86_64.tar.gz
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
if [ "$(uname -s)" != "Linux" ]; then
    echo "This packages for Linux; on Windows use scripts/package-windows.ps1, on a Mac scripts/package-macos.sh." >&2
    exit 1
fi

# A target folder of its own, as on the other systems: a build open from
# target/release must not be replaced under itself.
if [ "${1:-}" != "--skip-build" ]; then
    cargo build --release --target-dir target-package -p tequila-app -p tequila-cli
fi
release="$root/target-package/release"
for required in tequila-app tequila; do
    [ -f "$release/$required" ] || { echo "Missing $release/$required" >&2; exit 1; }
done

stamp="${TEQUILA_PACKAGE_STAMP:-$(date +%Y%m%d-%H%M%S)}"
# The version is when it was built, as on the other systems: 2026.10.30.1200.
version="${TEQUILA_VERSION:-${stamp:0:4}.${stamp:4:2}.${stamp:6:2}.${stamp:9:4}}"
package="$root/dist/Tequila-Linux-$stamp"
out="$package/Tequila"
rm -rf "$package"
mkdir -p "$out"

# The programs.
cp "$release/tequila-app" "$out/Tequila"
cp "$release/tequila" "$out/tequila-cli"
chmod +x "$out/Tequila" "$out/tequila-cli"

# Embree, TBB and OIDN, with their symbolic links, from the SDKs the build
# linked (scripts/fetch-third-party.sh). Each library finds the others
# beside itself.
for sdk in third_party/embree-*.x86_64.linux third_party/oidn-*.x86_64.linux; do
    [ -d "$sdk/lib" ] || { echo "Missing $sdk: run bash scripts/fetch-third-party.sh" >&2; exit 1; }
    cp -a "$sdk"/lib/*.so* "$out/"
done
if command -v patchelf >/dev/null; then
    for library in "$out"/*.so*; do
        if [ -f "$library" ] && [ ! -L "$library" ]; then
            patchelf --set-rpath '$ORIGIN' "$library"
        fi
    done
else
    echo "patchelf not found: the libraries keep the rpath their SDK gave them" >&2
fi

# The material library, the studio presets, the sample scenes (the folder's
# model files are the tests' fixtures), the Python module, the example
# plugins, the guides on the Help menu, and the skill Connect an AI offers.
mkdir -p "$out/assets/samples" "$out/python"
cp -R assets/library "$out/assets/library"
cp -R assets/studios "$out/assets/studios"
cp assets/samples/*.tequila "$out/assets/samples/"
cp python/*.py "$out/python/"
cp -R plugins/examples "$out/plugins"
cp docs/SCRIPTING.md "$out/SCRIPTING.md"
cp docs/plugins.md "$out/PLUGINS.md"
cp docs/materials-authoring.md "$out/MATERIALS.md"
python3 scripts/package-skill.py "$out/Tequila-Skill.zip" >/dev/null
cp LICENSE NOTICE THIRD_PARTY_LICENSES.md TRADEMARKS.md "$out/"

# The licences: the SDKs' own notices, every Rust crate's, and the ones a
# crate compiles in without shipping (scripts/notices).
licenses="$out/licenses"
mkdir -p "$licenses"
for sdk in third_party/*.x86_64.linux; do
    for folder in doc share; do
        if [ -d "$sdk/$folder" ]; then
            mkdir -p "$licenses/$(basename "$sdk")"
            cp -R "$sdk/$folder" "$licenses/$(basename "$sdk")/"
        fi
    done
done
cargo metadata --format-version 1 --filter-platform x86_64-unknown-linux-gnu | python3 -c '
import json, shutil, sys
from pathlib import Path
out = Path(sys.argv[1])
lines = []
for package in json.load(sys.stdin)["packages"]:
    manifest = Path(package["manifest_path"])
    if package["source"] or "vendor" in manifest.parts:
        notices = [f for f in manifest.parent.iterdir() if f.is_file() and f.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT"))]
        if notices:
            folder = out / (package["name"] + "-" + package["version"])
            folder.mkdir(parents=True, exist_ok=True)
            for notice in notices:
                shutil.copy(notice, folder)
        lines.append(package["name"] + " " + package["version"] + ": " + str(package["license"]))
(out / "rust-dependencies.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
' "$licenses"
cp -R scripts/notices/. "$licenses/"

# The icon, drawn by Tequila itself, and a menu entry for install.sh to put
# where the desktop looks.
"$out/Tequila" --write-icon "$out/tequila.png"
cat > "$out/install.sh" <<'INSTALL'
#!/usr/bin/env sh
# Put Tequila in this desktop's application menu, pointing at this folder.
# Run it again after moving the folder. Nothing outside your home is touched.
set -eu
here="$(cd "$(dirname "$0")" && pwd)"
applications="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$applications"
cat > "$applications/tequila.desktop" <<ENTRY
[Desktop Entry]
Type=Application
Name=Tequila
Comment=Photoreal product rendering
Exec="$here/Tequila" %F
Icon=$here/tequila.png
Terminal=false
Categories=Graphics;3DGraphics;
MimeType=application/x-tequila;
StartupWMClass=Tequila
ENTRY
chmod +x "$applications/tequila.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$applications" || true
echo "Tequila is in the application menu."
INSTALL
chmod +x "$out/install.sh"

cat > "$out/START-HERE.txt" <<START
Tequila $version for Linux (x86-64)

Run ./Tequila from this folder. ./install.sh puts it in your desktop's
application menu.

Updating: Help > Check for updates fetches the newer one; unpack it in
place of this folder. Your materials, plugins and settings are kept in
~/.config/Tequila and stay.

Renders use Tequila's own renderer on the processor. Cycles and the
OpenCASCADE STEP reader are not in the Linux version yet: STEP files are read
by Tequila's own reader.

tequila-cli is the command-line renderer; with --worker it is a render node
for network rendering. python/ holds the scripting module.
START

revision="$(git rev-parse HEAD)"
printf 'Source: %s\nVersion: %s\nBuilt: %s\nIncludes current working-tree changes.\n' "$revision" "$version" "$(date +%Y-%m-%dT%H:%M:%S%z)" > "$out/BUILD.txt"

archive="$root/dist/Tequila-$version-linux-x86_64.tar.gz"
tar -czf "$archive" -C "$package" Tequila
echo "Packaged $out"
echo "Archive $archive"
