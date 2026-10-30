# Tequila: Product Rendering

**[Download](../../releases/latest)** · **[Discord](https://discord.gg/Na5T2qgQBX)**

**Free, open-source product rendering for Windows and Mac.** Bring in a CAD
model, put real materials on it, light it, and render pictures of the product
- with the viewport rendering the final image live as you work.

Tequila is built to the same workflow as commercial product renderers such as
KeyShot, written independently from published rendering science
([TRADEMARKS.md](TRADEMARKS.md)). It renders through **Cycles**, Blender's
production renderer (Apache-2.0), on the graphics card or the processor, and
has a path tracer of its own as well.

## Download

Get the newest version from the **[Releases page](../../releases/latest)**:

| | File | Needs |
|---|---|---|
| **Windows** | `Tequila-Setup-<version>.exe` | Windows 10 or 11, 64-bit. An NVIDIA graphics card (GTX 900 series or newer) renders on the graphics card; any other computer renders on the processor. |
| **Mac** | `Tequila-<version>.dmg` | A Mac with Apple's own chips (M1 or newer) and a recent macOS. |
| **Linux** | `Tequila-<version>-linux-x86_64.tar.gz` | 64-bit Linux with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 or later). Unpack it anywhere and run `./Tequila`; `./install.sh` adds it to the application menu. Renders with Tequila's own renderer: Cycles is not in the Linux version yet. |

To update, choose **Help > Check for updates**: Tequila fetches the newer
version from this page, checks it against the SHA-256 GitHub lists for it,
and installs it (on a Mac it opens the disk image, for Tequila to be dragged
into Applications). It also looks once a day when it starts, which the same
window can turn off. Or download the newer one and install it over the old.
Either way your materials, plugins and settings are kept. Plugins are written in Python, so
they need Python 3 on the computer (on a Mac, it comes with Apple's command
line tools).

**The first time you open it**, your computer warns you, because Tequila is
not signed with a paid certificate:

- **Windows** shows "Windows protected your PC": click **More info**, then
  **Run anyway**. Signing for Windows, free through the SignPath
  Foundation, is being set up: see the [code signing policy](docs/CODE-SIGNING.md).
- **Mac** says Apple could not verify it: click **Done**, open **System
  Settings > Privacy & Security**, scroll down and click **Open Anyway** next
  to Tequila.

## Community

Questions, bugs, ideas and renders: join the
**[Tequila Discord](https://discord.gg/Na5T2qgQBX)**. Bugs can also go in
[GitHub issues](../../issues).

## What it does

- **Import CAD and 3D models** - STEP (through OpenCASCADE), IGES, glTF/GLB,
  OBJ (with ZBrush polypaint), STL, USD and IFC, with assemblies, names,
  colours and units; very large models; re-tessellate and re-import that
  keep your work. Send models from Blender (live), SolidWorks and ZBrush
  ([integrations](integrations)). Open KeyShot scenes (`.ksp`, `.bip`) with
  their materials, lighting and cameras ([notes](docs/keyshot-import.md)).
- **Materials** - a library of 309 procedural materials (plastics, metals,
  glass, paints with flakes, fabrics, gems, liquids with bubbles, wood,
  stone...), a node editor, labels and decals, texture packs, KeyShot
  material packages (.kmp), colourways and variants. Drop a material on a
  part, on a selection or on a whole group.
- **Lighting** - HDRI environments and an HDRI editor, sun and sky, area,
  spot, point and IES lights, light groups you can relight after rendering,
  and studio setups saved by name.
- **Camera and look** - a photographic camera with depth of field,
  orthographic views, and AgX, ACES and PBR Neutral tone mapping.
- **Floors** - shadow catchers, contact shadows, and objects-only mirror
  reflections.
- **Rendering** - a queue, render layers and passes (Cryptomatte included),
  region renders, EXR/PNG/JPEG, batch renders of every configuration, and
  network rendering across computers.
- **Sharing** - web export (live 3D or pre-rendered), a spin viewer, and glTF
  and USD export; a full-screen presentation mode.
- **Cutaways**, measurements, model sets and a configurator.
- **AI apps and automation** - connect Claude, Cursor, VS Code (GitHub
  Copilot), Codex, Gemini CLI, Windsurf or LM Studio in one click, or any
  other MCP app, and let it set up materials, lighting and renders, make
  materials from a description, or write plugins; a built-in Assistant for
  OpenAI, Claude, Gemini, OpenRouter, Mistral, DeepSeek, xAI, Groq or a local
  model;
  Python scripting, plugins, a command-line renderer, and a Blender live link.

Animation is not part of Tequila yet.

## Building it yourself

Tequila is written in Rust. You need the Rust toolchain and two prebuilt SDKs
(Intel Embree and Open Image Denoise, both Apache-2.0), which a script
fetches into `third_party/`:

```bash
# Windows
pwsh scripts/fetch-third-party.ps1
# macOS and Linux
bash scripts/fetch-third-party.sh

cargo run --release -p tequila-app
```

That runs Tequila with its own renderer. Cycles and the OpenCASCADE STEP
reader are built separately: `native/cycles/build-cycles.ps1` (Windows) or
`build-cycles.sh` (Mac), and `native/occt/build-occt.bat` or `build-occt.sh`.
The installers come from `scripts/package-windows.ps1` with
`scripts/package-installer.ps1`, and `scripts/package-macos.sh`.
[`docs/MACOS_PORT.md`](docs/MACOS_PORT.md) covers the Mac.

The tests run in release mode - the rendering tests integrate hundreds of
thousands of samples each:

```bash
cargo test --workspace --release
```

## Documentation

- [Scripting and AI apps](docs/SCRIPTING.md) - Python, the command line, and
  MCP for AI apps
- [Writing plugins](docs/plugins.md)
- [Making materials](docs/materials-authoring.md)
- [The specification](docs/TEQUILA_SPEC.md) and the [roadmap and status](docs/05-ROADMAP-AND-TESTING.md)
- [Code signing policy](docs/CODE-SIGNING.md)

## Licence

Apache-2.0: see [LICENSE](LICENSE), [NOTICE](NOTICE) and
[THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md). Tequila the software has
nothing to do with tequila the drink ([TRADEMARKS.md](TRADEMARKS.md)).
