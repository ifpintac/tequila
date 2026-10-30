#!/usr/bin/env bash
# Fetch the prebuilt SDKs Tequila links against.
#
# Embree and Open Image Denoise ship as prebuilt releases rather than as
# crates. Both are Apache-2.0. They land in third_party/, which is gitignored:
# these are build inputs, not project source.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
third_party="$root/third_party"
mkdir -p "$third_party"

embree_version="4.4.1"
oidn_version="2.5.1"

case "$(uname -s)" in
    Linux*)  embree_asset="embree-${embree_version}.x86_64.linux.tar.gz"
             oidn_asset="oidn-${oidn_version}.x86_64.linux.tar.gz" ;;
    Darwin*) arch="$(uname -m)"
             if [ "$arch" = "arm64" ]; then
                 embree_asset="embree-${embree_version}.arm64.macosx.zip"
                 oidn_asset="oidn-${oidn_version}.arm64.macos.tar.gz"
             else
                 embree_asset="embree-${embree_version}.x86_64.macosx.zip"
                 oidn_asset="oidn-${oidn_version}.x86_64.macos.tar.gz"
             fi ;;
    *) echo "Use scripts/fetch-third-party.ps1 on Windows." >&2; exit 1 ;;
esac

# Each SDK lands in a folder named for its platform (embree-4.4.1.arm64.macosx
# and so on), which is what the build scripts pick by: another platform's
# SDK in the same third_party/ - a checkout copied from Windows - is left
# alone and does not count as present.
fetch() {
    local name="$1" url="$2" asset="$3"
    local expected="${asset%.zip}"
    expected="${expected%.tar.gz}"
    local target="$third_party/$expected"
    if [ -d "$target/lib" ]; then
        echo "$name already present at $target"
        return
    fi
    echo "Downloading $name from $url"
    curl -fsSL "$url" -o "$third_party/$asset"
    echo "Extracting $name"
    local unpacked="$target.unpacking"
    rm -rf "$unpacked"
    mkdir -p "$unpacked"
    case "$asset" in
        *.tar.gz) tar -xzf "$third_party/$asset" -C "$unpacked" ;;
        *.zip)    unzip -q -o "$third_party/$asset" -d "$unpacked" ;;
    esac
    # Some archives hold their files in a folder, named for the archive or
    # not, some not: the SDK is wherever its lib folder is.
    local sdk="$unpacked"
    if [ ! -d "$sdk/lib" ] && [ ! -d "$sdk/lib64" ]; then
        local lib
        lib="$(find "$unpacked" -mindepth 2 -maxdepth 3 -type d \( -name lib -o -name lib64 \) -print -quit)"
        if [ -z "$lib" ]; then
            echo "$asset has no lib folder; it holds:" >&2
            find "$unpacked" -maxdepth 2 >&2
            exit 1
        fi
        sdk="$(dirname "$lib")"
    fi
    # Embree's Linux archive has lib64 where the build scripts and the
    # packaging look for lib.
    if [ ! -d "$sdk/lib" ]; then
        ln -s lib64 "$sdk/lib"
    fi
    mv "$sdk" "$target"
    rm -rf "$unpacked"
    rm "$third_party/$asset"
}

fetch embree \
    "https://github.com/RenderKit/embree/releases/download/v${embree_version}/${embree_asset}" \
    "$embree_asset"
fetch oidn \
    "https://github.com/RenderKit/oidn/releases/download/v${oidn_version}/${oidn_asset}" \
    "$oidn_asset"

echo
echo "Done. Both SDKs are in third_party/ and the build scripts will find them"
echo "there; no environment variables are needed."
