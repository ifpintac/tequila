# Tequila on macOS — port plan

Tequila is developed on Windows. Its eventual home is a MacBook (Apple
Silicon, most likely). It has not been built on a Mac yet. This plan comes
from an audit of the code on 2026-10-10. File references are as of that
date.

The Rust code is mostly portable already. Every Windows API call and every
`explorer` launch sits behind a `cfg` gate with a working fallback. The
scripting channel is a loopback TCP socket, the file dialogs are native
(rfd), and the shortcuts already use Cmd through `modifiers.command`. The
work is in linking, packaging, the Cycles build, and a few NVIDIA-only GPU
choices.

## What the Mac needs

- Xcode Command Line Tools, Rust (`rustup`), and CMake, Ninja and Python 3
  (Homebrew).
- Embree 4 and Open Image Denoise 2 for macOS arm64. Run
  `bash scripts/fetch-third-party.sh`; it handles Darwin arm64.
- Cycles built from source against Blender's `lib/macos_arm64` libraries
  (see below). A Mac compiles Cycles' GPU kernels itself, in Metal, at the
  first GPU render, so no kernel files need shipping.

A Mac build has to be made on a Mac. `.github/workflows/ci.yml` already lists
`macos-latest`, but the repository has no remote, so CI has never run.

**Done from Windows, 2026-10-11** (compiled and tested here where the code
runs on every platform; the scripts are written but not yet run on a Mac):
items 1, 2, 3, 4, 6, 7, 10 and 11, and scripts for 9 and 12; 5 was already
in the code. Each is marked below.

**Done from Windows, 2026-10-16**, tested here where the code runs on every
platform. The Mac-only code (`app/src/mac.rs`) and the scripts have not
been compiled or run on a Mac:

- **Quit could lose work.** winit's application menu sends `terminate:`.
  eframe then calls `on_exit` without the window ever being asked to close
  (its own comment says so), and `on_exit` discarded the crash journal. So
  Cmd+Q threw unsaved work away without asking. Now Quit sends
  `performClose:` to the main window, which gets the "Save your changes?"
  question, and `end_session` keeps the journal for any close nobody agreed
  to (the Dock's Quit, logging out), so the work comes back next launch.
- **Trackpad:** a two-finger swipe pans, a pinch zooms, Shift + swipe
  orbits, Cmd + swipe zooms and Alt + swipe changes the focal length
  (`orbit::Gestures`, keyed on egui's point-unit scrolls; Windows sends
  lines and is unchanged).
- **Window:** fitted to the screen on the first frame (`fitted_window`).
- **Keys:** labels say Cmd and Option. Cmd+H is the system's Hide, so hiding
  the selection is Control+H on a Mac.
- **Folders:** settings in `~/Library/Application Support/Tequila`, caches
  in `~/Library/Caches/Tequila` (`library::settings_directory`,
  `cache_directory`). `~/.tequila` keeps the journal and the scripting
  channel, as on Windows.
- **Bundle:** the guides, BUILD.txt and the skill are found in
  `Contents/Resources`, and Cycles is pointed at its kernel sources in
  `Contents/Resources/cycles`.
- **Packaging:** `package-macos.sh` now carries everything the Windows
  package does, draws the icon (`Tequila --write-icon`), and makes
  `dist/Tequila-<version>.dmg`. `native/occt/build-occt.sh` builds the STEP
  reader against Homebrew's OpenCASCADE (7.9). `build-cycles.sh` matches
  Blender's library folders without regard to case, since the Mac's are
  lower case, and lists what the dylib loads.

Still open: the rest of the "not Mac-like" list, and checking all of this on
a Mac.

## Blocks building

1. *Done.* `scripts/fetch-third-party.sh` is executable in git, as are the
   new Mac scripts, and `.gitattributes` keeps every `*.sh` LF in every
   checkout. The script now unpacks each SDK into a folder named for its
   platform, and a copied checkout's Windows SDKs no longer count as
   present.
2. *Done.* The Embree and OIDN build scripts take only the `third_party/`
   SDK whose folder names the target's OS and architecture
   (`CARGO_CFG_TARGET_OS`/`ARCH`).
3. *Done.* `round_to_quarter_hour` no longer warns off Windows.

## Blocks running and packaging

4. *Done.* `.cargo/config.toml` gives macOS binaries the rpaths
   `@executable_path` and `@executable_path/../Frameworks` (Linux:
   `$ORIGIN`), and the build scripts copy the dylibs beside every binary
   they build, by `CARGO_CFG_TARGET_OS`. (A build script's own link
   arguments would reach only its crate's targets, not the app's.)
5. *Already in the code.* **GPU device names.** "Graphics card" means Metal on macOS
   (`RenderEngine::cycles_device`). Device detection counts any GPU type
   (`batch::cycles_here`). The bridge needs no change, since
   `Device::type_from_string("METAL")` works.
6. *Done.* The GPU denoiser asks for `OIDN_DEVICE_TYPE_METAL` on a Mac;
   the packaging script ships every OIDN dylib, the Metal device's
   included.
7. *Done.* Inside a bundle the Cycles library is also looked for in
   `Contents/Frameworks/cycles` (tested); the build script sets the
   `@loader_path` rpath.
8. **Metal kernels.** *Sources shipped, 2026-10-16:* in
   `Contents/Resources/cycles/source`, where Tequila points Cycles. The
   cache: on a Mac, Cycles' `path_cache_get` uses `$XDG_CACHE_HOME` or
   `~/.cache`, not the path `path_init` is given, so compiled kernels
   should already be kept in `~/.cache/cycles`. Check it: a second GPU
   render, in a new launch, should start in seconds.
9. *Written, not yet run:* `native/cycles/build-cycles.sh`, which also
   copies the kernel sources Metal compiles from beside the library (where
   Tequila points Cycles by default). Was:
   **Cycles build scripts are Windows-only** (`native/cycles/build-cycles.ps1`
   and the spike's `build_cycles.bat`). A `build-cycles.sh` is needed. It
   should fetch `lib/macos_arm64` at the commit Cycles pins, build with Xcode
   clang, CMake and Ninja, and set:
   - `WITH_CYCLES_DEVICE_METAL=ON`
   - CUDA, OptiX, HIP and oneAPI off
   - `CMAKE_OSX_ARCHITECTURES=arm64`
   - an `@loader_path` rpath

   The shim's CMake block and its header are portable. Avoid loading two
   copies of Embree or TBB.
10. *Done.* With no output folder and no saved project, renders go to the
    working folder unless it is a disk's root, and then to
    `~/Pictures/Tequila` (`render_window::fallback_directory`, tested).
11. *Done.* The walk up from the executable also looks in each level's
    `Resources`, which in a bundle is `Contents/Resources`.
12. *Written, not yet run:* `scripts/package-macos.sh` builds the bundle
    below, the Python module in Resources (which now finds `tequila-cli`
    in `Contents/MacOS`), ad-hoc signed. Was: **Packaging.**
    `scripts/package-windows.ps1`, `launch-newest.ps1` and the
    `.lnk` shortcut need a `package-macos.sh` counterpart. It should:
    - build `Tequila.app` with an `Info.plist`: bundle ID,
      `NSHighResolutionCapable`, `LSMinimumSystemVersion`, and a `.tequila`
      document type;
    - add an `.icns` icon;
    - put Embree, TBB and the OIDN dylibs in `Frameworks`, fixed with
      `install_name_tool`;
    - sign the app. An ad-hoc signature (`codesign -s -`) is enough for your
      own Mac. Distributing to others needs a Developer ID, the hardened
      runtime and notarization. *Written 2026-10-16, not yet run:*
      `TEQUILA_SIGN_IDENTITY` and `TEQUILA_NOTARY_PROFILE` make the script
      sign for the hardened runtime, then notarize and staple the disk
      image. Since macOS 15 a copy that is not notarized opens only
      through System Settings > Privacy & Security > Open Anyway.

## Works, but not Mac-like

- **Times** are UTC outside Windows (`app/src/clock.rs`). Use
  `localtime_r`/`tm_gmtoff`.
- **API keys** are stored as plain base64 off Windows
  (`crates/tequila-ai/src/secret.rs`). Use the Keychain.
- *Done 2026-10-16.* **Storage** in `~/Library/Application Support` and
  `~/Library/Caches`; `~/.tequila` keeps the journal and `channel.json`.
- **Graphics:** eframe's OpenGL backend and the Solid view's GLSL 330 run on
  macOS's deprecated GL 4.1. Moving to wgpu/Metal is a separate project.
- *Done 2026-10-17.* **Retina:** a Rendered viewport on Cycles renders the
  screen's own pixels (`render_scale` in `app/src/main.rs`); a screen of one
  pixel per point is as before, whatever the text size. Tequila's own
  renderer still traces the points: at a Retina screen's pixels its denoiser
  took 1.7 s a picture, during which the view could not begin to move
  (`retina_viewport_benchmark`). Still slow: at that size the viewport's GPU
  denoiser roughly halves Cycles' samples a second. *Done 2026-10-20:* the
  library's tiles, the Material tab's sphere and the graph editor's swatch are
  rendered at the screen's density (`thumbnail::density`).
- *Done 2026-10-17.* **Orbiting on Cycles** shows pictures as it goes: the
  live driver read Cycles' first picture after a change as empty, and its
  smaller navigating pictures at the view's size; and a camera moving every
  frame restarted Cycles before any picture was made
  (`the_cycles_viewport_shows_pictures_while_orbiting`).
- *Done 2026-10-16.* **Shortcut labels** say Cmd and Option; hiding the
  selection is Control+H, since Cmd+H is the system's Hide.
- *Done 2026-10-16.* **Trackpad:** two-finger pan, pinch zoom, Shift + swipe
  orbit. To check by hand: the speeds, and that a mouse wheel still zooms.
- **Menus:** the menu bar lives inside the window; the global menu is
  winit's (Hide, Quit). *Done 2026-10-16:* Quit asks about unsaved work.
- *Done 2026-10-20.* **Opening files from Finder:** an Apple Event handler
  for "open documents", installed as AppKit finishes launching
  (`listen_for_opened_files` in `app/src/mac.rs`), takes what Finder, the Dock
  and Open With send, at launch and while Tequila runs, through the same
  routes as a dropped file. The Info.plist exports a type for `.tequila` and
  `.freeshot` and offers to open models and materials. The release smoke test
  opens two projects with `open -a`.
- *Done 2026-10-16.* **Window size:** fitted to the screen.
- **Docs** use PowerShell and `python` (Macs have `python3`).

## Suggested order

1. Make it build: items 1–3, then `cargo build` on the Mac with Tequila's
   own engine.
2. Make it run from `cargo run`: rpath (4), output folder (10).
3. Build Cycles for Metal (9, 8, 7) and check that Cycles renders match
   Tequila's, as the Windows tests do (`TEQUILA_CYCLES=... cargo test`).
4. Package `Tequila.app` (12, 11), ad-hoc signed.
5. Polish: Retina viewport, trackpad navigation, Cmd labels, menus, Metal
   denoiser.
