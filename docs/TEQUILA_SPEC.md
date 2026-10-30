# TEQUILA — Master Specification
### Open-source real-time product rendering. Complete KeyShot feature inventory + system architecture.

**Revision 2** — expanded and corrected. Supersedes revision 1.
**Verified against KeyShot documentation as of September 2026**, including KeyShot Studio 2025.3 (OpenPBR, Cryptomatte) and 2026.1 (Gallery, integrated Render Queue, headless animation, IFC import).

**Companion documents:** [`docs/01-RENDERER-CORE.md`](docs/01-RENDERER-CORE.md) · [`docs/02-MATERIAL-SYSTEM.md`](docs/02-MATERIAL-SYSTEM.md) · [`docs/03-SCENE-AND-IO.md`](docs/03-SCENE-AND-IO.md) · [`docs/04-UX-SPEC.md`](docs/04-UX-SPEC.md) · [`docs/05-ROADMAP-AND-TESTING.md`](docs/05-ROADMAP-AND-TESTING.md)

---

# PART 0 — Ground Rules

## 0.1 What we are building, precisely

KeyShot's source code, renderer internals, and file formats are proprietary and unpublished. A byte-identical clone is therefore impossible, and pursuing one would be the wrong goal anyway. **Tequila targets behavioral parity: the same capabilities, the same workflow, and output of equal or better physical accuracy — implemented independently from published rendering science.**

This is not a compromise. The physics of light transport is public knowledge, extensively published, and in several respects the open literature is *ahead* of where a closed 1990s-lineage renderer sits. Specifically, Tequila should be **better** than KeyShot in these areas, and the specification below commits to them:

| Area | KeyShot | Tequila target | Why we can do better |
|---|---|---|---|
| Surface shading model | 34 discrete, individually-tuned material types; OpenPBR only recently added for interchange | **OpenPBR Surface as the single canonical model**, with all 34 KeyShot-equivalent types expressed as parameter presets over it | One rigorous, energy-conserving, spec'd model instead of 34 hand-tuned ones. Fewer bugs, guaranteed energy conservation, trivially extensible, and interchange-native. See [02-MATERIAL-SYSTEM §2](docs/02-MATERIAL-SYSTEM.md). |
| Color management | ICC-based, limited | **OCIO v2 + ACES**, per-texture input transforms, configurable display transforms (ACES/AgX/Filmic) | Industry-standard, matches downstream compositing pipelines exactly. See [01-RENDERER-CORE §7](docs/01-RENDERER-CORE.md). |
| Dispersion / spectral effects | RGB renderer, limited dispersion | Optional **spectral rendering (Hero Wavelength Spectral Sampling)** for gems, thin films, dispersion | Physically correct rainbow dispersion and thin-film interference rather than an approximation. |
| Subsurface scattering | "Scattering Medium" / "Translucent" types | **Random-walk SSS** (Chiang et al. 2016) as the OpenPBR subsurface lobe | Ground-truth brute-force SSS, no dipole approximation artifacts. |
| File format | Closed `.bip` | **Open, documented, versioned, git-diffable `.tequila`** | Enables scripting, pipeline integration, version control of scenes, third-party tooling. A real competitive advantage. |
| Automation | Python `lux` module | **Python API that is the same command bus the UI uses** | Guarantees API/UI parity by construction; anything clickable is scriptable. |
| Licensing | $1,299/year rental-only subscription | Free, open source | — |

## 0.2 Hard legal rules (do not violate; these are build-blocking)

1. **Never decompile, disassemble, or reverse-engineer KeyShot.** All algorithms come from published literature (cited throughout `docs/01-RENDERER-CORE.md`) or from permissively-licensed open-source implementations.
2. **Never copy KeyShot's icons, artwork, splash screens, marketing copy, or manual text into the product.** Feature *concepts* and workflow structure are unprotectable functional ideas and are fair to mirror; expressive assets are not. All product strings must be independently written.
3. **Never bundle third-party licensed content**: Pantone, RAL, Mold-Tech, Sorensen, Coloro, INTERPON, Axalta swatch/material libraries. Ship the **Tequila Standard Library** (own, openly licensed, procedurally authored — see §5.4) and provide a plugin hook so users may install libraries they have licensed themselves.
4. **Never bundle KeyShot's HDRIs or material files.** Source environment maps from CC0 providers (Poly Haven and similar) with attribution recorded in `assets/library/CREDITS.md`.
5. **Native proprietary CAD parsers are out of scope.** SolidWorks `.sldprt`, CATIA, NX, Creo, and similar binary formats require vendor SDKs. Tequila supports the open exchange path (STEP/IGES/BREP via OCCT) plus universal mesh formats, which covers real-world workflows because every CAD package can export these.
6. Ship `NOTICE` and `TRADEMARKS.md` stating Tequila is an independent project, not affiliated with or endorsed by Luxion or KeyShot, and that any reference to KeyShot in documentation is descriptive and comparative.

## 0.3 What "watertight" means for this specification

Every feature listed in Part 1 has, somewhere in this suite, a specification of **how to build it** — not just that it exists. Where a subsystem is deep, the master document states the requirement and defers the engineering to a companion document, always with an explicit cross-reference. If you find a feature in Part 1 with no corresponding "how", that is a defect in this spec: file it and fix the spec before writing the code.

---

# PART 1 — Complete KeyShot Feature Inventory

Organized as the user encounters it. Every item is a Tequila requirement unless explicitly marked out-of-scope.

## 1.1 The Core Property: the viewport is the renderer

KeyShot's entire reputation rests on one architectural decision: **there is no separate preview and final render.** The 3D viewport runs the path tracer progressively, accumulating samples continuously, so every material and lighting edit is immediately visible in a physically correct image. There is no "set up and hit render and wait" cycle. There is no rasterized approximation that later differs from the final output.

This is the hardest requirement in this document and it constrains everything else. It cannot be added later to a system not designed for it. See [`01-RENDERER-CORE §9`](docs/01-RENDERER-CORE.md) for the progressive scheduler, interactivity budget, and dirty-state taxonomy that make it possible.

Operating modes:
- **CPU mode** — reports frames per second. Universal fallback, always available.
- **GPU mode** — reports samples per second; hardware ray tracing (NVIDIA RTX/OptiX, AMD HIP RT); automatically falls back to CPU on VRAM exhaustion. In-viewport warnings when a feature isn't supported on the GPU path.

## 1.2 Window and panel inventory

| Window | Contents |
|---|---|
| **Real-time View** | Main viewport. Live path-traced image, navigation, selection, right-click context menu. HUD showing accumulated samples, elapsed time, triangle and NURBS counts, resolution, active mode. Optional XYZ coordinate legend. Toast notifications for active states (denoising, HDRI editing, geometry-node evaluation, GPU-unsupported-feature warnings). |
| **Project window** (Space) | Six tabs: **Scene, Material, Environment, Lighting, Camera, Image**. §1.3–§1.9. |
| **Library window** | Drag-and-drop asset browser: Materials, Environments, Backplates, Colors, Textures, Models. Search, categories, favorites. |
| **Toolbar** | Cloud Library · Import · Library · Project · Animation · KeyShotXR · Presentation (Configurator) · KeyVR · Render · Screenshot. Customizable icon size and labels. |
| **Main menu** | File, Edit, View, Scene, Lighting, Camera, Render, Window, Help + Preferences. |
| **Animation Timeline** | Track-based timeline with draggable/resizable animation blocks, curve editor with per-keyframe toggle. |
| **Render window** | Job setup (output, quality, passes, region) plus the integrated **Render Queue**. |
| **Gallery** | In-app browser for completed renders (added 2026.1). |
| **Material Graph** | Dockable node editor: zoom, pan, visual grid, node snapping. |
| **HDRI Editor** | Pin-based lighting compositor over a 360° canvas, with pop-out mode. |

## 1.3 Scene tab — geometry, hierarchy, and structure

- **Scene Tree**: hierarchical parts/subassemblies/groups mirroring the imported CAD structure. Multi-select, rename, search/filter, drag-to-reparent.
- **Visibility**: hide/show per node, isolate-selected, lock.
- **Model Sets** (Pro): named, independently-saved Scene Tree variations inside one project file — e.g. three chassis variants of the same product. Created from a selection or via an Add Model Set dialog. Supports checkbox visibility, per-set locking, drag reorder, rename, an **"Always Visible"** designation for staging geometry (ground props, backdrops) that should appear in every set, and auto-generated thumbnails (re-renderable, size-configurable, high-res option for Configurator use).
- **Scene Units** — critical, and easy to get wrong: units drive model scale, **light intensity**, **material color density/absorption**, and **texture mapping scale**. A new empty scene has non-specific units; units are established on import or set manually (`Edit > Set Scene Units`) with two conversion modes: **Keep Scene Size** (adjusts the unit multiplier — corrects model size, light intensity, color density, camera distance) and **Scale Scene** (preserves dimensions but recalibrates texture mapping scale). Tequila must implement both modes explicitly; see [`03-SCENE-AND-IO §3`](docs/03-SCENE-AND-IO.md).
- **Geometry tools**: Move/rotate/scale gizmo with numeric entry · **Custom Pivot** (Corner, Cylinder, Sphere placement modes; multiple pivots per animated part) · Unwrap UV · Edit Normals · Flip Normals · **Re-Tessellate** (regenerate the render mesh from retained exact BREP geometry at a chosen tolerance — this is why CAD imports stay smooth at any zoom) · Close Mesh · Split Object Surface · Split Separate Objects · Mesh Simplification (decimation).
- **Set Camera Target** (click geometry to set the orbit pivot) · **Match Perspective** (align camera to a photo).
- **Instancing**: repeated geometry shares one mesh with per-instance transforms (importers detect this; Rhino importer instancing was a specific 2025.3 improvement). Essential for memory and BVH build time.
- **CAD metadata**: part metadata imported from CAD (Creo, SOLIDWORKS, NX) is retained and queryable; CSV export of CAD data (2026.1).

## 1.4 Material tab and material system

**The defining interaction:** drag a material thumbnail from the Library and drop it onto a part in the viewport. The material is applied instantly and the path-traced preview updates immediately. Sub-second perceived latency is a hard requirement.

The subtle behavior that everything depends on — and which is the single most commonly misunderstood part of KeyShot — is the **material linking model**: materials are shared instances. Dropping a material onto a part assigns that material *instance* to the part; parts that already shared a material instance change together. "Unlink" creates an independent copy. Getting this wrong makes the app feel broken in ways users can't articulate. Full semantics in [`04-UX-SPEC §3`](docs/04-UX-SPEC.md).

- **In-Project Library**: all materials used in the current scene, deduplicated, with numeric suffixes distinguishing unlinked copies.
- **Parameter panel** per material type; any mappable parameter (color, roughness, bump, opacity, …) carries a texture toggle.
- **Material type switching** with best-effort parameter transfer.
- **Save to library**; **Material Templates** (rule-based assignment, e.g. by CAD color or part-name pattern); **Material Importer** (bulk assignment workflows).
- **Multi-Materials**: several material variants bundled on one part, swappable — the substrate for colorways, Studios, and the Configurator.
- **Material Graph** (Pro): full node editor. Dockable, zoom/pan, grid, node snapping.
- **OpenPBR** (2025.3+): OpenPBR material parameters are preserved on import from Rhino, Blender, and glTF/GLB, and from USD (2026.1).

### 1.4.1 Material type catalog

**Basic (10):** Diffuse · Flat · Metal · Plastic · Paint · Glass · Glass (Solid) · Liquid · Translucent · Thin Film

**Advanced (14):** Advanced · Generic · Anisotropic · Dielectric · Gem · Measured · Metallic Paint · Multi-Layer Optics · Plastic (Cloudy) · Plastic (Transparent) · RealCloth · Scattering Medium · Translucent Medium · Velvet

**Special (6):** Emissive · Toon · Wireframe · X-Ray · Cutaway · Ground

**Light sources (4):** Area Light · Point Light · Spotlight · IES Light

**Manufacturer-certified libraries:** Pantone, RAL, Mold-Tech, Sorensen, Coloro, INTERPON, Axalta — **out of scope** (§0.2 rule 3).

Every one of these types is specified as an exact OpenPBR parameter preset in [`02-MATERIAL-SYSTEM §3`](docs/02-MATERIAL-SYSTEM.md), including the lobe configuration, default values, and which parameters the UI exposes for each.

### 1.4.2 Material Graph node catalog

- **Material nodes** — one per entry in §1.4.1, layerable and blendable.
- **Texture: image-based** — Texture Map · Tiled UV · Tri-Planar · Video Map.
- **Texture: 2D patterns** — Brushed · Mesh · Weave.
- **Texture: 3D procedural** — Brushed (Radial) · Camouflage · Cellular · Color Gradient · Contour · Curvature · Granite · Leather · Marble · Noise (Fractal) · Noise (Texture) · Occlusion · Scratches · Spots · Vertex Color · Volume Map · Wood · Wood (Advanced).
- **Geometry nodes** — Bubbles · Displace · Flakes · Fuzz.
- **Animation nodes** — Color Fade · Curve Fade.
- **Utility nodes** — Bump Add · Color Adjust · Color Composite · Color Invert · Color Key Mask · Color To Number · Color Curve Randomize · Mapping 2D.

Projection/mapping types: Planar · Cylindrical · Spherical · Box/Tri-planar · UV, each with scale, rotation, offset, tiling, and mirror controls. Algorithms for every procedural node are specified in [`02-MATERIAL-SYSTEM §6`](docs/02-MATERIAL-SYSTEM.md).

## 1.5 Environment tab — image-based lighting

KeyShot's lighting is **HDRI-first**: a spherical high-dynamic-range image supplies both the illumination and, optionally, the visible background and reflections. This is why a KeyShot render looks photographic within seconds of loading a scene, and it is the second feature (after drag-drop materials) that defines the product.

- Load `.hdr`, `.exr`, `.hdz` (KeyShot's zipped HDRI+metadata container; Tequila defines an equivalent `.fsenv`, see [`03-SCENE-AND-IO §5`](docs/03-SCENE-AND-IO.md)).
- **Brightness** (uniform exposure), **Contrast** (separation of dark/bright regions).
- **Transform**: Size (world scale of the environment dome), Height (vertical offset relative to ground), Rotation (spin — the primary tool for placing highlights and reflections without moving the camera).
- **Background modes**: Lighting Environment visible (hotkey `E`) · solid Color (`C`) · **Backplate** image, decoupled from the lighting HDRI (`B`).
- **Ground**: Ground Shadows on/off with shadow base color · **Occlusion Ground Shadows** (contact/AO-style rather than drop shadow) · Ground Reflections · **Flatten Ground** (`G`, projects the below-horizon portion of the HDRI flat onto the ground plane so the floor doesn't visibly curve) · Ground Size.
- **HDRI Editor** (Pro) — a lighting compositor, not just a loader:
  - **Pins** placed on a 360° canvas: **Light pins** (position, size, shape incl. rectangular with rounded corners, color, intensity, falloff), **Image pins** (drop an image patch into the environment), **Gradient pins** (background gradients independent of the lighting contribution).
  - Draggable physical **Sun & Sky** with sun size and separate ground-colour control.
  - Per-pin colour adjustment; pop-out canvas; extract pins from legacy environments; export the composite as `.hdr`/`.exr`.
  - All pins and settings embed in the project file rather than living as external assets.
- **Physical lights** in the scene (Area/Point/Spot/IES) combinable with IBL.
- **Light Layers** — isolate and rescale individual light contributions independently (studio-style multi-light control, and the basis for post-render relighting).

## 1.6 Lighting tab — render technique presets

*(This tab was omitted from revision 1 of this spec. It is not cosmetic — it is the global quality/technique controller that determines which light-transport features are enabled.)*

Presets, each a bundle of the settings below:

| Preset | Behavior |
|---|---|
| **Performance** | Disables light-source materials and shadows, reduced ray bounces. For scene setup and fast navigation. |
| **Basic** | Simple direct lighting with shadows. Fast, for models lit by environment alone. |
| **Product** | Direct **and** indirect lighting with shadows. The default for product shots with transparent materials and local lights. |
| **Interior** | Direct and indirect optimized for interior illumination; handles complex enclosed scenes and reduces noise from local lights. |
| **Jewelry** | Interior plus **Ground Illumination**, increased **Ray Bounces**, and **Caustics** enabled. |
| **Custom** | User-defined combination, savable as a named preset. |

Underlying settings exposed in Custom mode: **Ray Bounces** · **Shadow Quality** · **Global Illumination** (indirect light on/off and quality) · **Ground Illumination** (whether the ground plane contributes indirect bounce light) · **Caustics** (specular-to-diffuse light focusing — the manual's own guidance is to always enable it for reflective/refractive materials) · **Interior Mode** (sampling strategy optimized for enclosed spaces) · light-source-material enable/disable · self-shadowing.

Tequila implementation note: these presets map cleanly onto integrator configuration flags — see [`01-RENDERER-CORE §11`](docs/01-RENDERER-CORE.md) for the exact preset→parameter table, including how "Interior Mode" translates to a change in light-sampling strategy and how Caustics selects the SDS-path technique.

## 1.7 Camera tab

- **Lens types**: Perspective · Orthographic · Shift (perspective-corrected/architectural) · Panoramic (spherical/cylindrical/360).
- **Focal length / FOV** in real-world millimetres (14mm–500mm typical range), inversely coupled to field of view.
- **Depth of Field**: focal distance, f-stop, click-to-focus, **Keep Focus lock** (auto-refocus when the camera is repositioned, added 2025.3), DOF quality (a render setting).
- **Match Perspective**: 2-point/3-point alignment to a reference photo with colour-coded vanishing guides (X blue, Z red, Y green).
- **Ground Grid**: 1:1 real-world-scale overlay, density tied to Environment Size.
- **Multiple named cameras**, saved viewpoints, quick switching (`Shift+N`).
- **Physical exposure** (ISO / shutter / f-number) feeding the Image tab's tone mapping.

Navigation controls (replicate exactly — muscle memory is a real switching cost for this audience):

| Action | Binding |
|---|---|
| Orbit | LMB drag |
| Pan | MMB drag |
| Zoom | Scroll wheel |
| Change focal length / FOV | Alt + scroll |
| Dolly (move camera) | Alt + RMB drag |
| Twist / roll | Ctrl(⌘) + Alt + scroll |
| Distance + perspective + FOV together | Shift + Alt + RMB drag |
| Standard views (front/back/L/R/top/bottom/iso) | Ctrl(⌘) + Alt + 1–7 |
| Set focus/orbit target under cursor | Ctrl(⌘) + Alt + RMB click |
| Switch camera | Shift + N |

## 1.8 Render tab and output

- **Render modes**: Default (blocking) · **Background** (Pro — keep working; pauses the live view by default) · **Send to Network** · explicit CPU-core and GPU-device allocation per job.
- **Render Queue** — integrated by default since 2026.1; queue multiple stills/animations/configurations.
- **Quality — Maximum Samples**: Samples (per-pixel accumulation), Samples Per Frame (animation consistency), Pixel Filter Size (reconstruction filter width, 1–3).
- **Quality — Maximum Time**: time-boxed progressive refinement.
- **Quality — Custom Control**: Samples · Ray Bounces · Anti-Aliasing Quality (default 1) · Shadow Quality · Global Illumination Quality · DOF Quality (3–5 for production) · Caustics Quality (max 50) · Sharp Shadows · Sharper Texture Filtering · Global Illumination Cache.
- **Denoise** — ML denoiser, viewport and final.
- **Region Rendering** — render only a user-defined rectangular region.
- **Still image output**: PNG · JPEG (quality slider) · EXR · TIFF · TIFF 32-bit · PSD · PSD 16-bit · PSD 32-bit. All except JPEG support alpha. Resolution presets or manual entry; **print-size calculator** (enter physical size + DPI → required pixel resolution). Filename prefix/template with system or scene revision numbering. Output folder selection. **Metadata sidecar export** as `.xmp` (Adobe-compatible) or simple `.meta`, containing scene info, camera settings, materials, and render quality.
- **Render Layers**: assign parts/models to named layers rendered as separable images, with three alpha-edge modes — Straight Alpha (unmatted; edge colour extended into semi-transparent anti-aliased pixels) · Premultiplied Alpha (matted with black) · Aliased (fully opaque fringe, no compositing gaps).
- **Render Passes** (all EXR): Diffuse · Lighting · Global Illumination · Caustics · Raw (no image style applied) · Reflection · Refraction · Shadow · Ambient Occlusion · Clown (flat per-material ID colours, optional label separation) · Depth · Normals (world or camera space) · Labels · **Cryptomatte** (open standard, added 2025.3 — per-object/material/asset coverage mattes for compositing).
- **Layered PSD output** with 8/16/32-bit depth.

## 1.9 Image tab — look development

- **Resolution**: drives the working viewport resolution and the render aspect ratio.
- **Image Styles** — non-destructive, live in both viewport and final render, saved as swappable presets (and one of the components a Studio bundles):
  - **Basic**: exposure/brightness, contrast, saturation, simple colour controls.
  - **Photographic**: full camera-like tone mapping — highlights/shadows/whites/blacks, colour temperature and tint, vignette, sharpening, bloom/glare, chromatic aberration.
- Tequila extends this with an OCIO display-transform selector (ACES / AgX / Filmic / sRGB) — see [`01-RENDERER-CORE §7`](docs/01-RENDERER-CORE.md).

## 1.10 Animation (Pro)

**Types:** Part Animations (translate/rotate/scale parts and groups — assembly and exploded-view reveals) · Camera Animations (position, target, lens) · Environment Animations (rotation, brightness) · Material Animations (via Color Fade / Curve Fade graph nodes) · Deformation Animations (imported mesh deformation from Alembic/FBX) · **Studio Switch Events** (cut between whole Studio presets on the timeline).

**Tooling:** **Animation Wizard** (guided setup for common patterns — turntable, exploded view) · timeline with draggable/resizable/stackable/sortable blocks (sort by time, type, name) · **Curve Editor** with per-keyframe visibility toggle (2026.1) · **Motion Blur** · **Motion Ease** (acceleration/deceleration curves) · **multiple custom pivot points per animated part** (2026.1).

**Physics Simulation:** rigid-body dynamics with gravity, collision detection, mass, friction, restitution; **jointed objects** with constraint types (hinge, slider, etc.) for mechanically-linked motion that would be impractical to keyframe by hand.

## 1.11 Additional tools

- **Cutaway** — a special material plus a companion tool defining a cutting plane or solid through geometry to reveal interiors, with dedicated cap shading at the cut surface.
- **Measurements** — in-viewport dimensional annotation against the real-world-scaled model.
- **Decals and Labels** — overlay layers on top of a base material, not baked into it:
  - Added via the material panel `+` (Add Label (Texture) / Add Label (Video) / Add Material), by dragging onto the model in the viewport, or by dropping into the layer list.
  - Dropped onto geometry: auto-orients to the surface at the contact point. Dropped into the layer list: default 0°/90° orientation.
  - **Move Texture** gizmo for position, rotation, scale.
  - PSD import reads embedded DPI to preserve intended real-world label size.
  - Per-label projection type, diffuse/opacity/bump maps, and its own material type (plastic/metal/paint/…) governing how light interacts with the label surface.
  - Unlimited labels per material; ordered stack; later labels composite over earlier.
  - Video textures supported (product screens playing UI footage).

## 1.12 Studios (Pro)

A Studio is a saved bundle of `{Camera, Environment, Image Style, Model Set, Multi-Material selection}` — a fully composed "shot" recallable in one click, and the unit that Animation (Studio Switch events) and the Configurator both consume.

Add (captures current camera + environment by default, configurable) · edit each component via dropdowns · drag to reorder · delete · auto thumbnails with adjustable sample count and a high-resolution option for Configurator use · configurable defaults for what a new Studio captures.

## 1.13 Configurator (Pro)

Turns Studios + Multi-Materials + Model Sets into an interactive product configurator.

- **Configurator Wizard**: define parent models and components (what can be swapped), material variations (linked within Multi-Materials), which Studios are selectable presets, and a custom UI layout for the picker panel.
- **Outputs**: (1) **Presentation Mode** — full-screen real-time interactive viewer in-app, CPU mode recommended for touch kiosks; (2) **standalone viewer package** (KeyShot's `.ksp`) with optional password protection, logo, watermark; (3) **web export** — embeddable interactive configurator; (4) **batch image rendering** of every valid configuration combination.
- Tequila note: KeyShot gates camera movement in web/iBook output behind a paid add-on. Tequila should ship full web export including camera movement — there is no reason to gate it.

## 1.14 KeyShotXR — interactive spin viewers

Renders a grid of frames across horizontal rotation × optional vertical tilt × optional zoom levels, packaged into an embeddable HTML/JS drag-to-spin viewer for product pages. Tequila equivalent: frame-grid renderer + a small embeddable JS viewer component (see [`03-SCENE-AND-IO §8`](docs/03-SCENE-AND-IO.md)).

## 1.15 Network rendering

Distributes stills, animation frames, and Configurator batch combinations across networked machines via a monitor/queue. Maps naturally onto a headless CLI renderer plus a job dispatcher. **Headless mode supports animation rendering** as of 2026.1.

## 1.16 Scripting API

Python scripting console plus a `lux` module exposing scene manipulation, material/camera/environment control, render dispatch, and batch operations — used for pipeline automation (render every SKU colourway overnight). Tequila exposes an equivalent `tequila` Python module that is a binding over the same command bus the UI uses, guaranteeing parity ([`04-UX-SPEC §8`](docs/04-UX-SPEC.md)).

## 1.17 AI features (out of scope until M10)

**AI Shots** generative tooling: Restyle, Background, Imagine, Transform, Replace; **Edit Mode** with inpainting, full replacement, and upscaling (2026.1); model switched from FLUX.1 Schnell to a modified Qwen model. Permanent image storage with comparison toggles and drag-and-drop backplates.

These depend on diffusion-model infrastructure orthogonal to the renderer. Note for completeness; do not let them displace core work. If implemented later, the clean integration point is a post-render image operator with access to the beauty pass plus the depth, normal, and Cryptomatte AOVs (which makes region-limited edits reliable).

## 1.18 File format support

**Native CAD** (out of scope, §0.2 rule 5): 3ds Max `.max` · ALIAS `.wire` · AutoCAD `.dwg`/`.dxf` · CATIA `.cgr`/`.catpart`/`.catproduct`/`.catdrawing` · Cinema 4D `.c4d` · Creo `.prt`/`.asm` · Creo View `.pvz`/`.pvs`/`.edz`/`.ed`/`.c3di`/`.ol` · Inventor `.ipt`/`.iam` · Maya `.ma`/`.mb` · NX `.prt` · Onshape · Pro/ENGINEER · Revit `.rvt` · Rhinoceros `.3dm` · SketchUp `.skp` · Solid Edge `.par`/`.asm`/`.psm` · SOLIDWORKS `.sldprt`/`.sldasm`.

**Open exchange (in scope):** STEP `.stp`/`.step` · IGES `.igs`/`.iges` · ACIS `.sat`* · JT `.jt`* · Parasolid `.x_t`* · **IFC** (BIM, 2026.1). *(\*ACIS/JT/Parasolid require additional kernels; STEP and IGES via OCCT are the priority.)*

**Mesh:** 3DS · OBJ · STL · 3MF · PLY.

**Scene/animation interchange:** Alembic `.abc` · Collada `.dae` · FBX `.fbx` · 3DXML · **USD** `.usd`/`.usda`/`.usdc`/`.usdz` · **glTF/GLB**.

**LiveLinking:** 14+ CAD plugins pushing live geometry and material-assignment updates into a running session without re-export. Modern integration plugins: 3ds Max, Blender, Cinema 4D, Maya, SketchUp. Tequila's first target is **Blender**, whose Python API is fully open — protocol in [`03-SCENE-AND-IO §7`](docs/03-SCENE-AND-IO.md).

## 1.19 Preferences and application services

Units and scale · colour management · hotkey editor (Simple Hotkeys, Mouse Actions, Camera Controls; presets emulating other 3D applications; import/export/reset) · folder/path configuration for libraries and output · GPU/CPU device selection · UI theme and scaling · auto-save and crash recovery · undo history depth · Cloud Library account (out of scope for Tequila v1; replace with a local/shared folder library and an optional community index).

---

# PART 2 — System Architecture

## 2.1 Technology stack

| Layer | Choice | Rationale |
|---|---|---|
| Core language | **Rust** | Memory safety in a large multi-threaded renderer, strong FFI, mature graphics ecosystem, fearless concurrency for the progressive scheduler. C++20 is an acceptable alternative if the team prefers it; every dependency below has a C API. **Pick one and do not mix.** |
| CPU ray tracing | **Intel Embree 4** | The industry-standard BVH builder and intersector. Do not write your own BVH; you will not beat it and it is not where the value is. |
| GPU ray tracing | **wgpu** compute (portable baseline) + optional **OptiX** (NVIDIA) and **HIP RT** (AMD) backends behind one trait | Portability first, hardware RT acceleration where available. CPU path always present as fallback. |
| Shading language | **Slang** (or a restricted internal IR) compiled to both CPU and GPU targets | Guarantees one shading implementation across backends — invariant #3. See [`01-RENDERER-CORE §10`](docs/01-RENDERER-CORE.md). |
| Denoising | **Intel Open Image Denoise (OIDN) 2.x**, Apache-2.0 | Production-quality ML denoiser with albedo/normal guide support and CPU+GPU execution. Exact equivalent of KeyShot's AI denoiser. |
| Colour management | **OpenColorIO v2** with the ACES studio config | Industry standard; correct by construction. |
| Image I/O | **OpenImageIO** | EXR/TIFF/PNG/JPEG/PSD read-write, plus its texture cache and MIP pipeline (which you want anyway — see `02-MATERIAL-SYSTEM §5`). |
| CAD kernel | **Open CASCADE Technology (OCCT)**, LGPL-2.1 | The only mature open BREP/NURBS kernel that reads STEP/IGES and re-tessellates at arbitrary tolerance. Isolate behind `tequila-import` to keep LGPL obligations contained. |
| Mesh import | **Assimp** (BSD-3) + **OpenUSD** (Apache-2.0) | Covers OBJ/FBX/3DS/Collada/3MF/STL/glTF and the USD family. |
| Physics | **Rapier** (Rust) or **Bullet/Jolt** (C++) | Rigid bodies + joint constraints for §1.10. |
| UI | **egui** (immediate-mode, wgpu-backed) | Same language as the core, no FFI UI boundary, viewport-embeddable, `egui_node_graph` as a starting point for the Material Graph. Alternative: Qt (C++) if the team prefers retained-mode and native platform integration. |
| Scripting | A `tequila` Python module over the scripting protocol | Exposes the command bus as a `tequila` module, generated from the catalog; also the harness for automated integration tests. Out of process rather than PyO3-embedded: any Python 3.10+, nothing to build against one, and `tequila --serve` gives scripts the bus without a window. |
| Web output | Tequila's own WebGL2 viewer (glTF + generated frames) | Configurator web export and XR spin viewers; no three.js, so no third-party script to ship or update. |

## 2.2 Process and module architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                          Tequila Application                          │
│                                                                        │
│  ┌────────────────┐        commands         ┌──────────────────────┐  │
│  │   UI (egui)     │ ──────────────────────▶ │     Command Bus       │  │
│  │  panels, node   │ ◀──────────────────────  │  validate · apply ·   │  │
│  │  editor, HUD    │      state snapshots    │  undo/redo · journal  │  │
│  └────────────────┘                          └──────────┬───────────┘  │
│           ▲                                              │ mutations     │
│           │                     ┌────────────────────────▼───────────┐  │
│  ┌────────┴───────┐             │        Scene Document               │  │
│  │ Python API      │────────────▶│  nodes · materials · env · cameras  │  │
│  │  (PyO3)         │  commands   │  animation · studios · settings     │  │
│  └────────────────┘             │  + dirty-state tracker              │  │
│                                  └──────────┬─────────────────────────┘  │
│                                             │ immutable snapshot         │
│                                  ┌──────────▼─────────────────────────┐  │
│                                  │       Render Orchestrator           │  │
│                                  │  progressive scheduler · budget ·   │  │
│                                  │  restart policy · AOV accumulation  │  │
│                                  └────┬────────────────────────┬───────┘  │
│                                       │                        │          │
│                        ┌──────────────▼─────┐      ┌───────────▼───────┐  │
│                        │  CPU backend        │      │  GPU backend       │  │
│                        │  Embree + Rayon     │      │  wgpu/OptiX/HIPRT  │  │
│                        └──────────┬─────────┘      └───────────┬───────┘  │
│                                   └────────────┬───────────────┘          │
│                                    ┌───────────▼────────────┐             │
│                                    │  Framebuffer + AOVs     │             │
│                                    └───────────┬────────────┘             │
│                                    ┌───────────▼────────────┐             │
│                                    │  OIDN denoise           │             │
│                                    └───────────┬────────────┘             │
│                                    ┌───────────▼────────────┐             │
│                                    │  OCIO display transform │             │
│                                    │  + Image Style          │             │
│                                    └───────────┬────────────┘             │
│                                       viewport │ / file output            │
└──────────────────────────────────────────────────────────────────────────┘

Supporting subsystems (all consume the Scene Document, all mutate via commands):
  Import pipeline · Animation engine · Physics · Studios/Configurator ·
  Network dispatcher · XR/web exporter · Asset library index
```

**The three rules this diagram encodes:**

1. **Nothing mutates the Scene Document except the Command Bus.** The UI and the Python API are peers, both issuing commands. This is what makes undo, scripting, and automated testing work without duplicated logic.
2. **The renderer never reads the live Scene Document.** It reads an immutable snapshot handed to it by the orchestrator. This is what lets the UI stay responsive while a render is in flight, and it eliminates an entire class of data races.
3. **The dirty-state tracker classifies every mutation** so the orchestrator knows whether to restart accumulation globally, restart a screen region, or continue. Detailed taxonomy in [`01-RENDERER-CORE §9.3`](docs/01-RENDERER-CORE.md).

## 2.3 Repository layout

```
tequila/
├── crates/
│   ├── tequila-core/          # IDs, math, units, colour types, error types
│   ├── tequila-scene/         # Scene Document, command bus, undo, dirty tracking
│   ├── tequila-format/        # .tequila container read/write + schema validation
│   ├── tequila-import/        # OCCT / Assimp / USD → Scene  (LGPL boundary: OCCT)
│   ├── tequila-materials/     # OpenPBR model, graph IR, compiler, procedurals
│   ├── tequila-texture/       # OIIO texture cache, MIP, UDIM, colour-space handling
│   ├── tequila-render-core/   # integrator, sampling, lights, media, camera, AOVs
│   ├── tequila-render-cpu/    # Embree + Rayon backend
│   ├── tequila-render-gpu/    # wgpu / OptiX / HIP RT backends
│   ├── tequila-denoise/       # OIDN wrapper, guide-AOV prep, temporal stability
│   ├── tequila-color/         # OCIO integration, display transforms, image styles
│   ├── tequila-animation/     # timeline, keyframes, curves, physics (Rapier)
│   ├── tequila-studio/        # Studios, Model Sets, Multi-Materials, Configurator
│   ├── tequila-xr/            # spin-viewer frame grids + JS viewer packaging
│   ├── tequila-network/       # job queue, worker protocol, headless CLI
│   ├── tequila-scripting/     # PyO3 bindings over the command bus
│   └── tequila-cli/           # headless renderer binary
├── app/                        # egui desktop application (UI only)
├── assets/library/             # Tequila Standard Library (own content only)
│   ├── materials/  environments/  backplates/  textures/  CREDITS.md
├── docs/                       # this specification suite
├── tests/
│   ├── analytic/               # furnace, χ², MIS-weight, energy-conservation tests
│   ├── golden/                 # reference images + tolerances
│   ├── scenes/                 # sample .tequila scenes
│   └── perf/                   # benchmark scenes + budget assertions
├── NOTICE · TRADEMARKS.md · LICENSE
```

## 2.4 Milestone roadmap (summary — full task breakdown in `docs/05`)

| Milestone | Deliverable | Proves |
|---|---|---|
| **M0** | CPU path tracer, one hard-coded mesh + HDRI, progressive accumulation, orbit/pan/zoom | The core feel exists before anything else is built |
| **M1** | Scene Document, command bus, undo, mesh import, Scene tab, `.tequila` format | The document model is right |
| **M2** | OpenPBR shading, full material type catalog, Library window, drag-and-drop application, textures | The headline feature |
| **M3** | HDRI environment system, ground plane, all lens types, DOF, Image Styles, OCIO | It looks like a product shot |
| **M4** | Full render settings, all AOVs incl. Cryptomatte, layers, region render, OIDN, output formats | It ships images to a pipeline |
| **M5** | Timeline, part/camera/environment animation, wizard, motion blur, physics | Motion |
| **M6** | Material Graph editor, HDRI Editor pins, labels/decals, cutaway, measurements, Studios, Model Sets, Multi-Materials | Pro parity |
| **M7** | Configurator, web export, XR spin viewer, network rendering, Python API | Distribution and automation |
| **M8** | OCCT STEP/IGES import with BREP retention and re-tessellation, USD, Blender LiveLinking | CAD-grade |
| **M9** | GPU backends with verified CPU parity | Speed (decided: Cycles is the GPU engine; see `docs/05` M9) |
| **M10** | Optional: AI image operators; spectral rendering promoted to default for gems | Beyond parity |

## 2.5 Licensing

Tequila itself: **Apache-2.0** (patent grant matters for a renderer) or MIT. OCCT is LGPL-2.1 — link, do not fork, and keep it behind the `tequila-import` boundary. Embree (Apache-2.0), OIDN (Apache-2.0), OIIO (BSD-3), OCIO (BSD-3), Assimp (BSD-3), OpenUSD (Apache-2.0), wgpu (MIT/Apache-2.0), egui (MIT/Apache-2.0) are all compatible. Record every dependency and its licence in `THIRD_PARTY_LICENSES.md` and check it in CI.

---

# PART 3 — Master Feature Checklist

Track implementation against this. Section references point to where the "how" is specified.

### Renderer core → `docs/01`
- [x] Progressive path-traced viewport, CPU backend, never blocks UI (§9)
- [x] Next-event estimation + MIS (power heuristic) (§3)
- [x] Owen-scrambled Sobol sampler (§4) — blue-noise screen-space decorrelation still to do
- [x] Adaptive sampling with per-pixel variance halting (§4.4) - validated against a non-adaptive reference
- [x] Russian roulette + path regularization + firefly suppression (§3.5)
- [x] Environment importance sampling (2D CDF; hierarchical warp still to do) (§5.2)
- [ ] Many-light sampling (light BVH / alias table) (§5.3)
- [ ] Area / point / spot / IES light sampling (§5.1)
- [ ] Participating media: homogeneous + heterogeneous, ratio tracking (§6)
- [ ] Random-walk subsurface scattering (§6.4)
- [ ] Caustics: SPPM or MNEE pass gated by the Caustics setting (§6.5)
- [x] Thin-lens camera: focal length, sensor, f-stop, aperture blades, focus (§8)
- [x] Orthographic / shift / panoramic projections (§8.2)
- [ ] Motion blur (shutter interval sampling) (§8.4)
- [ ] Ray differentials + MIP/anisotropic texture filtering (§8.5)
- [ ] All render passes/AOVs incl. spec-compliant Cryptomatte (§12)
- [x] OIDN denoising with albedo/normal guides (§13) — temporal stability at M5
- [~] Scene-linear ACEScg pipeline + display transforms (§7). OCIO v2 itself at M3.
- [x] Optional spectral rendering (hero wavelength) for dispersion (§7.4): the Render window's Spectral, Auto by default
- [x] Dirty-state taxonomy and restart policy (§9.3)
- [x] Region rendering (§9.5)
- [~] GPU backend with verified statistical parity to CPU (§10): Cycles (OptiX/CUDA) is the GPU engine; measured agreement with Tequila's renderer in `docs/05` M9, not yet a CI gate
- [x] Lighting preset → integrator configuration table (§11)
- [x] Deterministic rendering (§4.6)

### Materials → `docs/02`
- [~] OpenPBR Surface: diffuse, conductor, dielectric and specular-over-diffuse lobes, energy conserving (§2). Remaining lobes at M2.
- [ ] All 34 material types as OpenPBR presets with exposed-parameter sets (§3)
- [ ] 4 light-source material types (§3.4)
- [ ] Material graph IR, compiler, and evaluation (§4.3)
- [ ] All texture nodes: 4 image, 3 pattern, 18 procedural (§6.2–6.4)
- [ ] Utility nodes (§6.5), geometry nodes (§4.4), animation nodes (§4.5)
- [ ] Projection modes: planar/cylindrical/spherical/box/tri-planar/UV (§5.3)
- [ ] Texture cache, MIP pyramids, UDIM, per-map colour space (§5.1–5.2)
- [ ] Bump / normal / displacement, incl. adaptive tessellation (§5.4, §4.4)
- [ ] Label/decal layer stack with per-label material and projection (§7)
- [ ] Multi-Materials (§8)
- [ ] Material linking/unlinking semantics (§8.2, and `docs/04` §3)

### Scene, I/O → `docs/03`
- [x] Scene Document with instancing and stable IDs (§2)
- [x] Scene units, both conversion modes (§3)
- [x] `.tequila` container + JSON schema + round-trip tests (§4) — published at [`docs/file-format.md`](file-format.md), which a test checks against the writer
- [ ] Environment container `.fsenv` (§5)
- [~] Mesh import with normals/smoothing/unit handling (§6.1). OBJ, STL, glTF/GLB done; FBX out of scope as a proprietary Autodesk format, the rest at M8 (`docs/05` M1 note 2)
- [~] BREP import (OCCT) with retained exact geometry and Re-Tessellate (§6.2): retained geometry, Re-Tessellate and B-rep edges done for STEP through truck and for IGES through Tequila's own reader; OCCT (XDE metadata and layers) not
- [x] USD import/export (§6.3): `.usda`, OpenPBR round trip through MaterialX
- [x] IFC import (§1.18 open exchange): structure, bodies with openings cut, styles, property sets as metadata
- [x] Instancing detection on import (§6.4)
- [x] CAD metadata retention + CSV export (§6.5): imported, stored and saved; the Scene tab's Metadata panel edits it, `Field = value` searches select by it, Export CSV writes it
- [x] Blender LiveLinking protocol (§7): `docs/livelink-protocol.md`
- [x] XR spin-viewer export (§8)
- [x] Configurator web export (§9): live 3D in Tequila's own WebGL2 viewer (no three.js), or pre-rendered; a static folder

### UX → `docs/04`
- [~] Window/panel layout and docking (§1). Scene tab, viewport, status bar and menus exist; the six-tab Project window and docking do not
- [ ] Real-time view HUD and overlays (§2)
- [ ] Drag-and-drop material application with correct linking semantics (§3)
- [x] Selection model: part / object / group / material-linked (§4)
- [~] Transform gizmo + custom pivot modes (§5). Move/rotate/scale over a whole selection, grid and angle snapping, snap-to-ground, vertex snap, numeric entry, and Origin/Centre/Corner/Custom pivots done, with custom pivots persisted per node. The universal gizmo is not built, and Cylinder/Sphere pivots wait for M8: they fit an axis to a picked *surface*, which needs the exact CAD geometry rather than its triangles
- [ ] Full navigation and hotkey table, remappable, with presets (§6)
- [ ] Material graph editor interactions (§7)
- [x] Command catalog, undo/redo, journal (§8)
- [ ] Library window: index, search, tags, favorites, thumbnails (§9)
- [ ] Timeline and curve editor interactions (§10)
- [ ] Render window, queue, gallery (§11)
- [ ] Preferences incl. hotkey editor and colour management (§12)
- [ ] Progressive-feedback UX contract (what the user sees while converging) (§13)

### Application features → this document
- [x] Model Sets (§1.3)
- [x] Studios (§1.12)
- [x] Configurator + presentation mode + batch render (§1.13): options switching Multi-Material variants, Model Sets or Studios; excludes/requires rules with nearest-valid resolution; a four-step wizard in the Studios tab; Presentation mode (F5, full screen, the picker over the rendered view); every valid combination queued with a time estimate. The standalone viewer package and web export are separate items.
- [~] Network rendering + headless CLI incl. animation (§1.15): stills and configurator combinations across render nodes (`tequila-cli --worker`, `--farm`, the Render window); animation waits for M5
- [x] Python scripting API (§1.16): the `tequila` module, generated from the catalog, to the application or a headless `tequila --serve`; reference in [`scripting-api.md`](scripting-api.md)
- [ ] Render queue + gallery (§1.8)
- [ ] Cutaway, measurements (§1.11)
- [ ] Animation: all 6 types, wizard, curve editor, motion blur/ease, physics with joints (§1.10)
- [~] Preferences, auto-save, crash recovery (§1.19). Auto-save and crash recovery done; preferences are not

### Quality gates → `docs/05`
- [ ] White furnace test passes for every BSDF lobe (§3.1)
- [ ] χ² sampling tests pass for every BSDF and light type (§3.2)
- [ ] Golden-image suite per material type and per node type (§3.4)
- [ ] CPU/GPU parity within tolerance (§3.5)
- [ ] Determinism test (§3.6)
- [~] Performance budgets enforced in CI (§4). `bench/simple_product` and `bench/cad_assembly` exist and are measured by `tequila --bench`; every §15 target is met on 16 cores. The CI gate that fails on a >10% regression is still to build
- [x] File-format round-trip equality (§3.7)
- [ ] UI/API parity test: every command reachable from both (§3.8)

---

# Appendix — Sources

KeyShot official documentation, accessed September 2026:

[Features](https://www.keyshot.com/features/) · [Material Types](https://manuals.keyshot.com/keyshot2024/manual/material-types.html) · [Material Graph Nodes](https://manual.keyshot.com/manual/materials/material-graph/material-graph-nodes/) · [Texture Node Types](https://manual.keyshot.com/manual/materials/material-graph/material-graph-nodes/texture-node-types/) · [Material Node Types](https://manual.keyshot.com/manual/materials/material-graph/material-graph-nodes/material-node-types/) · [Render Options](https://manual.keyshot.com/manual/render-4/render-options/) · [Layers and Passes](https://manual.keyshot.com/manual/render-4/render-output/layers-and-passes/) · [Still Image Output](https://manual.keyshot.com/manual/render-4/render-output/still-image-output/) · [Region Rendering](https://manual.keyshot.com/manual/render-4/render-output/region-rendering/) · [Environment Settings](https://manual.keyshot.com/manual/environments/environment-settings/) · [HDRI Editor](https://manual.keyshot.com/manual/environments/hdri-editor/) · [Lighting](https://manual.keyshot.com/manual/lighting/) · [Lens Settings](https://manual.keyshot.com/manual/cameras/lens-settings/) · [Perspective](https://manual.keyshot.com/manual/cameras/lens-settings/perspective/) · [Depth of Field](https://manual.keyshot.com/manual/cameras/depth-of-field/) · [Navigating Your Scene](https://manual.keyshot.com/manual/cameras/navigating-your-scene/) · [Animation](https://manual.keyshot.com/manual/animation/) · [Animation Wizard](https://manual.keyshot.com/manual/animation/animation-wizard/) · [Project Window](https://manual.keyshot.com/manual/user-interface/project-window/) · [Material Tab](https://manual.keyshot.com/manual/user-interface/project-window/material-tab/) · [Real-time View](https://manual.keyshot.com/manual/user-interface/real-time-view/) · [Toolbar](https://manual.keyshot.com/manual/user-interface/toolbar/) · [Studios](https://manual.keyshot.com/manual/studios/) · [Model Sets](https://manual.keyshot.com/manual/models-tab/model-sets/) · [Scene Units](https://manual.keyshot.com/manual/models-tab/scene-units/) · [Configurator](https://manual.keyshot.com/manual/configurator/) · [Tools](https://manual.keyshot.com/manual/tools-3/) · [Labels](https://manual.keyshot.com/manual/labels/) · [Supported File Formats](https://manual.keyshot.com/manual/models-tab/supported-file-formats/) · [Hotkeys Preferences](https://manual.keyshot.com/manual/preferences/hotkeys-preferences/) · [Scripting: lux module](https://media.keyshot.com/scripting/doc/2024.1/lux.html)

Release coverage: [KeyShot Studio 2025.3 — OpenPBR, Cryptomatte, performance](https://www.keyshot.com/blog/keyshot-studio-performance-updates-version-2025-3/) · [KeyShot Studio 2026.1 (CG Channel)](https://www.cgchannel.com/2026/03/keyshot-releases-keyshot-studio-2026-1/)

Rendering literature is cited inline in `docs/01-RENDERER-CORE.md`.
