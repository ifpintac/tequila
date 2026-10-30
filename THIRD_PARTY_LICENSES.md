# Third-party licences

Every dependency Tequila links against, and its licence.
`docs/05-ROADMAP-AND-TESTING.md` §5 requires this file to be generated and
verified in CI.

## Linked SDKs

These are prebuilt releases fetched by `scripts/fetch-third-party`. They are
not vendored into the repository.

| Component | Version | Licence | Used for |
|---|---|---|---|
| [Intel Embree](https://github.com/RenderKit/embree) | 4.4.1 | Apache-2.0 | BVH build and ray traversal (`tequila-render-cpu`) |
| [Intel Open Image Denoise](https://github.com/RenderKit/oidn) | 2.5.1 | Apache-2.0 | ML denoising (`tequila-denoise`) |

## Microsoft Visual C++ runtime

`scripts/package-windows.ps1` copies Microsoft's C++ runtime DLLs
(`vcruntime140`, `msvcp140` and their companions) from Visual Studio's
redistributable folder into the Windows package, beside `Tequila.exe`, so it
runs on a PC without the Visual C++ Redistributable installed. They are
Microsoft's, distributed under the Visual Studio licence terms for
redistributable code.

## Cycles

Built by `native/cycles/build-cycles.ps1` into `tequila_cycles.dll`, which a
package carries in its `cycles` folder and Tequila loads only when Cycles is
chosen as the engine. Licence texts are in `licenses/cycles`.

| Component | Licence | Used for |
|---|---|---|
| [Cycles](https://projects.blender.org/blender/cycles) (standalone v5.0.0, engine sources as Blender 5.0.1) | Apache-2.0 | The Cycles render engine; its GPU kernels (`cycles/lib`) are compiled from the same sources |
| `tequila_cycles` (`native/cycles`) | Apache-2.0 | Tequila's C interface over Cycles |
| cuew (in Cycles) | Apache-2.0 | Loading the NVIDIA driver at run time |
| [OpenImageIO](https://github.com/AcademySoftwareFoundation/OpenImageIO) | Apache-2.0 | Cycles' image handling |
| [OpenColorIO](https://github.com/AcademySoftwareFoundation/OpenColorIO) | BSD-3-Clause | Cycles' colour management |
| [OpenEXR](https://github.com/AcademySoftwareFoundation/openexr), [Imath](https://github.com/AcademySoftwareFoundation/Imath) | BSD-3-Clause | EXR support in OpenImageIO |
| [Open PGL](https://github.com/RenderKit/openpgl) | Apache-2.0 | Path guiding |
| [pugixml](https://github.com/zeux/pugixml) | MIT | XML reading |
| libjpeg-turbo, libpng, libtiff, libwebp, OpenJPEG, zlib, zstd | IJG/BSD-3-Clause/Zlib, libpng, libtiff, BSD-3-Clause, BSD-2-Clause, Zlib, BSD-3-Clause | Image formats and compression in OpenImageIO and Cycles |
| pthreads4w | Apache-2.0 | Threads on Windows |

Embree, TBB and Open Image Denoise are the copies listed above, shared with
Tequila's own renderer.

OptiX, the RTX ray-tracing path, is compiled against NVIDIA's OptiX 8.0
headers (github.com/NVIDIA/optix-dev), used at build time only; OptiX itself
runs inside the NVIDIA driver. The headers are under NVIDIA's SDK licence,
not an open-source one, and binaries built with them are distributed on
NVIDIA's terms (`licenses/cycles/NVIDIA-OptiX-SDK-LICENSE.txt`). A build made
without them (`build-cycles.ps1 -NoOptiX`) uses CUDA alone.

## OpenCASCADE

STEP files are read by `tequila-occt.exe` (`native/occt`, built by
`native/occt/build-occt.bat`), a program of its own that a package carries in
its `occt` folder with the libraries below, all dynamically linked and
replaceable. Licence texts are in `occt/licenses`.

| Component | Version | Licence | Used for |
|---|---|---|---|
| [Open CASCADE Technology](https://github.com/Open-Cascade-SAS/OCCT) | 8.0.1 | LGPL-2.1 with the OCCT exception | Reading STEP files and meshing their surfaces |
| `tequila-occt` (`native/occt`) | | Apache-2.0 | Tequila's reader over OpenCASCADE |
| [oneTBB](https://github.com/uxlfoundation/oneTBB) | 2021.13 | Apache-2.0 | OpenCASCADE's threads |
| [jemalloc](https://github.com/jemalloc/jemalloc) | | BSD-2-Clause | OpenCASCADE's memory allocator |
| [FreeImage](https://freeimage.sourceforge.io) | 3.18.0 | FreeImage Public License 1.0 | Loaded by OpenCASCADE's visualisation library; not used for reading |
| [FreeType](https://freetype.org) | 2.13.3 | FreeType License | Loaded by OpenCASCADE's visualisation library; not used for reading |
| [FFmpeg](https://ffmpeg.org) (avcodec, avformat, avutil, swscale) | 3.3.4 | LGPL-2.1 or later (built without GPL parts) | Loaded by OpenCASCADE's visualisation library; not used for reading |
| [OpenVR](https://github.com/ValveSoftware/openvr) | 1.14.15 | BSD-3-Clause | Loaded by OpenCASCADE's visualisation library; not used for reading |

The libraries are OpenCASCADE's own prebuilt release (`opencascade-
release-no-pch.zip` and `3rdparty-vc14-64.zip`, release V8.0.1). The source
of OpenCASCADE and of each LGPL library is available from the projects
above at the versions listed, and on request.

Portions of this software are copyright © 2024 The FreeType Project
(www.freetype.org). All rights reserved.

jemalloc: Copyright (C) 2002-present Jason Evans, Copyright (C) 2007-2012
Mozilla Foundation, Copyright (C) 2009-present Facebook, Inc. All rights
reserved. OpenVR: Copyright (c) 2015, Valve Corporation. All rights
reserved. Both are distributed on these terms:

```text
Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:
1. Redistributions of source code must retain the above copyright notice(s),
   this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright
   notice(s), this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the distribution.
3. (OpenVR only) Neither the name of the copyright holder nor the names of
   its contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER(S) OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.
```

## Rust dependencies

| Crate | Licence | Used for |
|---|---|---|
| `bitflags` | MIT OR Apache-2.0 | BSDF flag sets |
| `rayon` | MIT OR Apache-2.0 | Tile-parallel rendering, table precomputation |
| `exr` | BSD-3-Clause | OpenEXR reading |
| `tiff`, `weezl` | MIT (weezl: MIT OR Apache-2.0) | TIFF textures (`tequila-texture`) and TIFF output (`tequila-format`) |
| `image-webp`, `byteorder-lite`, `quick-error` | MIT OR Apache-2.0 (byteorder-lite: Unlicense OR MIT) | WebP textures |
| `gif`, `color_quant` | MIT OR Apache-2.0 (color_quant: MIT) | GIF textures |
| `bcdec_rs` | MIT | Block-compressed (BC1 to BC7) DDS textures |
| `jxl-oxide`, `jxl-bitstream`, `jxl-coding`, `jxl-color`, `jxl-frame`, `jxl-grid`, `jxl-image`, `jxl-jbr`, `jxl-modular`, `jxl-oxide-common`, `jxl-render`, `jxl-threadpool`, `jxl-vardct` | MIT OR Apache-2.0 | JPEG XL textures |
| `brotli-decompressor`, `alloc-no-stdlib`, `alloc-stdlib`, `tracing`, `tracing-core` | BSD-3-Clause OR MIT (alloc-*: BSD-3-Clause; tracing: MIT) | `jxl-oxide`'s own dependencies |
| `resvg`, `usvg`, `svgtypes`, `simplecss`, `roxmltree`, `kurbo`, `data-url`, `imagesize`, `xmlwriter`, `euclid`, `siphasher`, `pico-args` | MIT OR Apache-2.0 (imagesize, xmlwriter, pico-args: MIT) | SVG artwork, rasterized (`tequila-texture`) |
| `tiny-skia`, `tiny-skia-path`, `arrayref` | BSD-3-Clause (arrayref: BSD-2-Clause) | resvg's rasterizer |
| `fontdb`, `fontconfig-parser`, `rustybuzz`, `ttf-parser`, `unicode-bidi`, `unicode-bidi-mirroring`, `unicode-ccc`, `unicode-properties`, `unicode-script`, `unicode-vo`, `core_maths`, `memmap2` | MIT OR Apache-2.0 (fontdb, fontconfig-parser, rustybuzz, core_maths: MIT) | The system's fonts, for text in SVG artwork |
| `zune-jpeg`, `zune-core`, `rgb`, `strict-num`, `float-cmp` | MIT OR Apache-2.0 OR Zlib (rgb, strict-num, float-cmp: MIT) | Pictures embedded in SVG artwork, and resvg's own dependencies |
| `ureq`, `ureq-proto`, `http`, `httparse`, `itoa`, `utf8-zero`, `base64` | MIT OR Apache-2.0 | HTTPS requests to AI providers (`tequila-ai`) |
| `rustls` | Apache-2.0 OR ISC OR MIT | TLS for those requests |
| `rustls-pki-types`, `zeroize` | MIT OR Apache-2.0 | TLS certificate types; wiping key material |
| `rustls-webpki`, `untrusted` | ISC | Certificate verification |
| `ring` | Apache-2.0 AND ISC | Cryptography for TLS |
| `subtle` | BSD-3-Clause | Constant-time comparisons for TLS |
| `webpki-roots` | CDLA-Permissive-2.0 | Mozilla's root certificates |
| `vdb-rs` (vendored in `vendor/vdb-rs` with one fix, see its `TEQUILA-PATCH.md`) | MIT | Reading OpenVDB volumes for Volume Maps (`tequila-texture`) |
| `blosc-src` | MIT; builds c-blosc (BSD-3-Clause) and LZ4 (BSD-2-Clause) from source | Blosc decompression of VDB files |
| `bitvec`, `glam`, `half`, `bytemuck`, `byteorder`, `flate2`, `thiserror`, `log` | MIT OR Apache-2.0 (bitvec: MIT) | `vdb-rs`'s own dependencies |
| `truck-stepio` (vendored in `vendor/truck-stepio` with one fix, see its `TEQUILA-PATCH.md`), `truck-base`, `truck-geometry`, `truck-geotrait`, `truck-topology`, `truck-modeling`, `truck-polymesh`, `truck-meshalgo`, `truck-derivers` | Apache-2.0 | STEP import: B-rep conversion and tessellation (`tequila-import`) |
| `ruststep`, `ruststep-derive` | Apache-2.0 | Parsing STEP exchange files |
| `spade`, `robust` | MIT OR Apache-2.0 | Constrained Delaunay triangulation of STEP faces |
| `cgmath`, `approx`, `matext4cgmath` | Apache-2.0 | truck's linear algebra |
| `nom`, `derive_more`, `derive-new`, `convert_case`, `winnow`, `branches` | MIT | STEP parsing and derives |
| `Inflector` | BSD-2-Clause | Name conversion in `ruststep-derive` (build time) |
| `mikktspace` | MIT OR Apache-2.0 | MikkTSpace tangents for normal maps (`tequila-scene`), with `glam` 0.15 for its vectors |
| `foldhash` | Zlib | Hashing in `hashbrown` |
| `aho-corasick` | Unlicense OR MIT | Used by `regex` |
| `chrono`, `iana-time-zone`, `itertools`, `regex`, `regex-automata`, `regex-syntax`, `hashbrown`, `allocator-api2`, `array-macro`, `lazy_static`, `rclite`, `katexit`, `minimal-lexical`, `proc-macro-crate`, `proc-macro-error`, `toml_edit`, `toml_datetime` | MIT OR Apache-2.0 | The STEP crates' own dependencies |

## Ported code and data

| Component | Version | Licence | Used for |
|---|---|---|---|
| [Hosek-Wilkie skylight and solar radiance model](https://cgg.mff.cuni.cz/projects/SkylightModelling/) | 1.4a | BSD-3-Clause | The physical sky (`tequila-render-core/src/hosek.rs`, data in `hosek_data.rs`) |
| Cycles' sheen table (`table_sheen_ltc` in `src/scene/shader.tables`) | Cycles 5.0.0 | Apache-2.0 | The fitted linearly transformed cosines of Zeltner, Burley & Chiang's sheen, reproduced unchanged (`tequila-materials/src/lobes/sheen_ltc.rs`), so velvet shades alike in both engines |
| Cycles' random-walk coefficients (`subsurface_random_walk_remap` and the radius scale of `bssrdf_setup_radius`) | Cycles 5.0.0 | Apache-2.0 | Chiang, Kutz & Burley's albedo fit with anisotropy, ported (`walk_coefficients` in `tequila-materials/src/openpbr.rs`), so subsurface shades alike in both engines |

The Hosek-Wilkie model's evaluation is ported from, and its fitted data is
reproduced unchanged from, the authors' reference implementation, under this
licence:

```text
Copyright (c) 2012 - 2013, Lukas Hosek and Alexander Wilkie
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

    * Redistributions of source code must retain the above copyright
      notice, this list of conditions and the following disclaimer.
    * Redistributions in binary form must reproduce the above copyright
      notice, this list of conditions and the following disclaimer in the
      documentation and/or other materials provided with the distribution.
    * None of the names of the contributors may be used to endorse or promote
      products derived from this software without specific prior written
      permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDERS BE LIABLE FOR ANY
DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND
ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## Not bundled

`TEQUILA_SPEC.md` §0.2 rules 3 and 4 forbid shipping third-party licensed
content. Tequila bundles no Pantone, RAL, Mold-Tech, Sorensen, Coloro,
INTERPON or Axalta swatch libraries, and no environment maps or material
files from any other renderer. Environment maps in `assets/library` are CC0,
with provenance recorded in `assets/library/CREDITS.md`.
