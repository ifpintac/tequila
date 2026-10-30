#!/usr/bin/env bash
# Package Tequila for the Mac it runs on, Apple silicon or Intel:
# Tequila.app, and a disk image to install it from - the Mac's counterpart
# of package-windows.ps1 and the Windows installer. Updating is installing the newer one over it: your
# materials, plugins and settings live outside the application
# (~/Library/Application Support/Tequila), so they stay.
#
# What goes where in the bundle (only code may go in MacOS and Frameworks):
#   Contents/MacOS        Tequila, tequila-cli (the command-line renderer,
#                         where tequila.open() and Connect an AI look), and
#                         occt/ - the STEP reader with its libraries, if built
#   Contents/Frameworks   Embree, TBB, Open Image Denoise (its Metal device
#                         included); cycles/ - Cycles, if built
#   Contents/Resources    the material library, the Python module, the
#                         example plugins, the guides, the Claude skill,
#                         BUILD.txt, the licences, Cycles' kernel sources
#
# Signing. By default ad hoc, which is enough for the Mac it was built on.
# For other Macs, a copy downloaded there opens without a warning only when
# it is signed with an Apple Developer ID (the Apple Developer Program) and
# notarized by Apple. Set, before running this:
#   TEQUILA_SIGN_IDENTITY  the certificate's name, as `security find-identity
#                           -v -p codesigning` lists it: "Developer ID
#                           Application: <name> (<team id>)"
#   TEQUILA_NOTARY_PROFILE a notarytool profile, made once with
#                           `xcrun notarytool store-credentials <profile>
#                           --apple-id <email> --team-id <team id>` (it asks
#                           for an app-specific password from account.apple.com)
# With both, the code is signed for the hardened runtime with a secure
# timestamp, and the disk image is notarized and stapled.
#
# Written on Windows and not yet run on a Mac: expect to adjust it once.
#
# Usage:  bash scripts/package-macos.sh [--skip-build]
# Result: dist/Tequila-macOS-<stamp>/Tequila.app and
#         dist/Tequila-<version>.dmg, or dist/Tequila-<version>-intel.dmg on
#         an Intel Mac. An Intel Mac's has no Cycles: Blender 5, whose
#         Cycles native/cycles builds, makes nothing for Intel Macs, so it
#         renders with Tequila's own engine.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
if [ "$(uname -s)" != "Darwin" ]; then
    echo "This packages for macOS; on Windows use scripts/package-windows.ps1." >&2
    exit 1
fi

# A target folder of its own, as on Windows: a build open from
# target/release must not be replaced under itself.
if [ "${1:-}" != "--skip-build" ]; then
    cargo build --release --target-dir target-package -p tequila-app -p tequila-cli
fi
release="$root/target-package/release"
# Which Mac this is, as the SDKs' folders, Rust's targets and the disk
# image are named for it. Help > Check for updates picks the disk image by
# that name (app/src/update.rs).
case "$(uname -m)" in
    arm64)  arch=arm64;  rust_target=aarch64-apple-darwin; suffix="" ;;
    x86_64) arch=x86_64; rust_target=x86_64-apple-darwin;  suffix="-intel" ;;
    *) echo "No Mac package for $(uname -m)" >&2; exit 1 ;;
esac
for required in tequila-app tequila; do
    [ -f "$release/$required" ] || { echo "Missing $release/$required" >&2; exit 1; }
done

stamp="$(date +%Y%m%d-%H%M%S)"
# The version is when it was built, as on Windows: 2026.10.16.0312.
version="${TEQUILA_VERSION:-$(date +%Y.%m.%d.%H%M)}"
package="$root/dist/Tequila-macOS-$stamp"
app="$package/Tequila.app"
contents="$app/Contents"
resources="$contents/Resources"
mkdir -p "$contents/MacOS" "$contents/Frameworks" "$resources"

# The executables.
cp "$release/tequila-app" "$contents/MacOS/Tequila"
cp "$release/tequila" "$contents/MacOS/tequila-cli"

# Embree, TBB and OIDN, found by the executables' rpath
# (@executable_path/../Frameworks, `.cargo/config.toml`).
for sdk in third_party/embree-*."$arch".macosx third_party/oidn-*."$arch".macos; do
    [ -d "$sdk/lib" ] || { echo "Missing $sdk: run bash scripts/fetch-third-party.sh" >&2; exit 1; }
    cp -a "$sdk"/lib/*.dylib "$contents/Frameworks/"
done

# Cycles, when it has been built (native/cycles/build-cycles.sh): its dylib
# and the libraries it brings in Frameworks/cycles, where Tequila looks in
# a bundle, and the kernel sources Metal compiles from at the first GPU
# render in Resources/cycles/source, where Tequila points Cycles. Embree,
# TBB and OIDN are not copied again: it finds those one folder up.
cycles_work="${TEQUILA_CYCLES_WORK:-$(cd "$root/.." && pwd)/cycles-spike}"
cycles_bin="$cycles_work/cycles/build/bin"
with_cycles=""
if [ -f "$cycles_bin/libtequila_cycles.dylib" ]; then
    with_cycles=1
    mkdir -p "$contents/Frameworks/cycles" "$resources/cycles"
    for f in "$cycles_bin"/*.dylib; do
        case "$(basename "$f")" in
            libembree*|libtbb*|libOpenImageDenoise*) ;;
            *) cp -a "$f" "$contents/Frameworks/cycles/" ;;
        esac
    done
    install_name_tool -add_rpath @loader_path/.. "$contents/Frameworks/cycles/libtequila_cycles.dylib" 2>/dev/null || true
    [ -d "$cycles_bin/source" ] && cp -R "$cycles_bin/source" "$resources/cycles/source"
    echo "Cycles included from $cycles_bin"
else
    echo "Cycles not built; packaging without it (renders use Tequila's own engine)"
fi

# OpenCASCADE's STEP reader, when it has been built (native/occt/
# build-occt.sh): a program of its own with its libraries, where Tequila
# looks for it (beside itself, in occt/). Its licences go to Resources.
occt_built="$root/native/occt/build/occt"
if [ -f "$occt_built/tequila-occt" ]; then
    mkdir -p "$contents/MacOS/occt"
    cp "$occt_built/tequila-occt" "$contents/MacOS/occt/"
    cp -R "$occt_built/libs" "$contents/MacOS/occt/libs"
    mkdir -p "$resources/licenses"
    [ -d "$occt_built/licenses" ] && cp -R "$occt_built/licenses" "$resources/licenses/occt"
    echo "OpenCASCADE STEP reader included from $occt_built"
else
    echo "OpenCASCADE reader not built; STEP files are read by truck alone"
fi

# Resources: the material library and the built-in studio presets beside it (found by walking up from the executable
# into Contents/Resources), the Python module, the example plugins (marked
# as Tequila's own by their .bundled file), the guides on the Help menu,
# and the Claude skill Connect an AI offers.
mkdir -p "$resources/assets" "$resources/python"
cp -R assets/library "$resources/assets/library"
cp -R assets/studios "$resources/assets/studios"
# The sample scenes the start screen opens; the folder's model files are the
# tests' fixtures.
mkdir -p "$resources/assets/samples"
cp assets/samples/*.tequila "$resources/assets/samples/"
cp python/*.py "$resources/python/"
cp -R plugins/examples "$resources/plugins"
cp docs/SCRIPTING.md "$resources/SCRIPTING.md"
cp docs/plugins.md "$resources/PLUGINS.md"
cp docs/materials-authoring.md "$resources/MATERIALS.md"
python3 scripts/package-skill.py "$resources/Tequila-Skill.zip" >/dev/null
cp LICENSE NOTICE THIRD_PARTY_LICENSES.md TRADEMARKS.md "$resources/"

# The licences: the SDKs' own notices, every Rust crate's, and the ones a
# crate compiles in without shipping (scripts/notices).
licenses="$resources/licenses"
mkdir -p "$licenses"
for sdk in third_party/*."$arch".macos*; do
    for folder in doc share; do
        if [ -d "$sdk/$folder" ]; then
            mkdir -p "$licenses/$(basename "$sdk")"
            cp -R "$sdk/$folder" "$licenses/$(basename "$sdk")/"
        fi
    done
done
cargo metadata --format-version 1 --filter-platform "$rust_target" | python3 -c '
import json, shutil, sys
from pathlib import Path
out = Path(sys.argv[1])
lines = []
for package in json.load(sys.stdin)["packages"]:
    manifest = Path(package["manifest_path"])
    # Registry crates, and crates vendored in vendor/ with a fix: both are
    # someone else'"'"'s code in the binary, and both owe a notice.
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

# The icon, drawn by Tequila itself on a Mac application's grid.
iconset="$package/Tequila.iconset"
mkdir -p "$iconset"
"$contents/MacOS/Tequila" --write-icon "$package/icon-1024.png"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$package/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$package/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$resources/Tequila.icns"
rm -rf "$iconset" "$package/icon-1024.png"

cat > "$contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Tequila</string>
    <key>CFBundleDisplayName</key><string>Tequila</string>
    <key>CFBundleIdentifier</key><string>io.github.ifpintac.tequila</string>
    <key>CFBundleExecutable</key><string>Tequila</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${version}</string>
    <key>CFBundleVersion</key><string>${version}</string>
    <key>CFBundleIconFile</key><string>Tequila</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Tequila project</string>
            <key>CFBundleTypeRole</key><string>Editor</string>
            <key>LSHandlerRank</key><string>Owner</string>
            <key>LSItemContentTypes</key><array><string>io.github.ifpintac.tequila.project</string></array>
            <key>CFBundleTypeIconFile</key><string>Tequila</string>
        </dict>
        <dict>
            <key>CFBundleTypeName</key><string>3D model</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>CFBundleTypeExtensions</key>
            <array>
                <string>obj</string><string>stl</string><string>gltf</string><string>glb</string>
                <string>step</string><string>stp</string><string>igs</string><string>iges</string>
                <string>usd</string><string>usda</string><string>ifc</string>
            </array>
        </dict>
        <dict>
            <key>CFBundleTypeName</key><string>Material</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>CFBundleTypeExtensions</key><array><string>kmp</string><string>fsmat</string></array>
        </dict>
    </array>
    <key>UTExportedTypeDeclarations</key>
    <array>
        <dict>
            <key>UTTypeIdentifier</key><string>io.github.ifpintac.tequila.project</string>
            <key>UTTypeDescription</key><string>Tequila project</string>
            <key>UTTypeConformsTo</key><array><string>public.data</string><string>public.content</string></array>
            <key>UTTypeIconFile</key><string>Tequila</string>
            <key>UTTypeTagSpecification</key>
            <dict>
                <key>public.filename-extension</key><array><string>tequila</string><string>freeshot</string></array>
            </dict>
        </dict>
    </array>
</dict>
</plist>
PLIST

revision="$(git rev-parse HEAD)"
# The version, as the disk image is named: Help > Check for updates
# compares it with the latest release's.
printf 'Source: %s\nVersion: %s\nBuilt: %s\nIncludes current working-tree changes.\n' "$revision" "$version" "$(date +%Y-%m-%dT%H:%M:%S%z)" > "$resources/BUILD.txt"

# What a person reads first, in the disk image beside the application.
readme="$package/Read me first.txt"
cat > "$readme" <<'README'
TEQUILA FOR MAC

Drag Tequila into Applications, then open it from there.
To update, drag the newer one in and choose Replace: your materials,
plugins and settings are kept (~/Library/Application Support/Tequila).

If the Mac says Apple could not verify Tequila (a copy that was not
notarized): click Done, open System Settings > Privacy & Security, scroll
to the bottom and click Open Anyway next to Tequila. Once is enough.

Import: Cmd+I (STEP, IGES, glTF, GLB, OBJ, STL, USD, IFC)
New / open / save project: Cmd+N / Cmd+O / Cmd+S (File > Open recent)
Orbit: click and drag. Pan: two-finger swipe. Zoom: pinch.
Shift + swipe orbits too; a mouse pans with its middle button and zooms
with its wheel.
Frame selection: F; frame everything: Shift+F
Materials: drag one from the library onto a part. Dropped on any selected
part it goes on the whole selection; double-click a part to select its
group (a body that came in pieces).
Hide the selection: Control+H (Cmd+H hides the whole application, as in
every Mac application); show everything: Option+H.
AI apps: Assistant > Connect an AI... > Connect the Claude app lets Claude
work in Tequila - materials, procedural materials from a description,
lighting, renders, even plugins.
Plugins: Plugins > Manage plugins... They are written in Python, which a
Mac has once Apple's command line tools are installed (xcode-select
--install in Terminal).
Guides: Help > Guides.
If Tequila closes unexpectedly, the next launch shows what happened (with
macOS's own crash report); Copy for Discord puts it on the clipboard for
#bug-reports. Help > Crash reports.
README
if [ -n "$with_cycles" ]; then
    cat >> "$readme" <<'README'

Faster final renders: Render settings > Engine > Cycles (graphics card).
The first render on the graphics card prepares it for this Mac, which
can take a few minutes; later ones start at once.
README
fi
cp "$readme" "$resources/"

# Signed, the innermost code first and the bundle last: ad hoc, or with a
# Developer ID for the hardened runtime (see the top). The hardened runtime
# needs no exceptions: every library is signed by the same identity, Metal
# compiles its kernels without JIT memory, and plugins run Python as a
# process of its own.
identity="${TEQUILA_SIGN_IDENTITY:--}"
if [ "$identity" = "-" ]; then
    sign=(codesign --force -s -)
else
    sign=(codesign --force --timestamp --options runtime -s "$identity")
fi
while IFS= read -r library; do
    "${sign[@]}" "$library"
done < <(find "$contents/Frameworks" "$contents/MacOS" -name '*.dylib' -type f)
if [ -f "$contents/MacOS/occt/tequila-occt" ]; then "${sign[@]}" "$contents/MacOS/occt/tequila-occt"; fi
"${sign[@]}" "$contents/MacOS/tequila-cli"
"${sign[@]}" "$app"
codesign --verify --deep --strict "$app"
echo "Packaged and signed ($([ "$identity" = "-" ] && echo "ad hoc" || echo "$identity")): $app"

# The disk image: the application, a shortcut to Applications to drag it
# onto, and the read-me.
stage="$package/dmg"
mkdir -p "$stage"
cp -R "$app" "$stage/"
cp "$readme" "$stage/"
ln -s /Applications "$stage/Applications"
dmg="$root/dist/Tequila-$version$suffix.dmg"
rm -f "$dmg"
hdiutil create -quiet -volname "Tequila $version" -srcfolder "$stage" -ov -format UDZO "$dmg"
rm -rf "$stage"
if [ "$identity" != "-" ]; then
    codesign --force --timestamp -s "$identity" "$dmg"
    if [ -n "${TEQUILA_NOTARY_PROFILE:-}" ]; then
        # Apple checks it (a few minutes) and the ticket is stapled to the
        # image, so it opens on a Mac that is offline too.
        xcrun notarytool submit "$dmg" --keychain-profile "$TEQUILA_NOTARY_PROFILE" --wait
        xcrun stapler staple "$dmg"
        spctl --assess --type open --context context:primary-signature --verbose "$dmg"
        echo "Notarized: it opens on any Mac"
    else
        echo "Signed but not notarized: set TEQUILA_NOTARY_PROFILE for other Macs"
    fi
fi
echo "Disk image: $dmg"
