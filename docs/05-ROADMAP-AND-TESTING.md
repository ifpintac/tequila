# 05 — Roadmap, Acceptance Criteria, and Test Strategy

The build order, with explicit "done" definitions, and the correctness regime that keeps a renderer honest.

---

## 0. How to work through this

Each milestone below lists tasks sized to be one focused work session each, and an **exit criterion** that is objectively checkable. Do not start milestone N+1 until milestone N's exit criterion passes in CI.

Two rules that matter more than they look:

- **Write the test before the feature wherever the test is analytic.** For a renderer, "does this BSDF sample correctly" has a mathematical answer (§3.2). Discovering a sampling bug through a golden-image diff six months later costs a hundred times more than the χ² test that would have caught it immediately.
- **Keep a benchmark scene rendering from M0 onward.** Performance regressions are invisible until they are catastrophic. Track samples/sec on every commit.

---

## 1. Milestones

### M0 — Renderer proof of concept
*Goal: prove the core feel exists before building an application around it.*

- [x] Project skeleton, workspace, CI (build + test + clippy/lint on Linux/macOS/Windows).
- [x] Math foundation: vectors, transforms, ONB, sampling warps, RNG (PCG32), Sobol′ + Owen scrambling (§01 §4.1).
- [x] Embree integration: build a BVH from a hard-coded triangle mesh, trace rays.
- [x] Camera: thin-lens perspective ray generation with pixel filter importance sampling.
- [x] Diffuse (Oren-Nayar), GGX specular dielectric, GGX metal — with `eval`/`sample`/`pdf` and the BSDF trait (§01 §3.3).
- [x] Environment map loading (EXR/HDR via OIIO) + 2D CDF importance sampling (§01 §5.2).
- [x] Path-tracing integrator with NEE and MIS (§01 §3.1).
- [x] Progressive accumulation buffer, tile scheduler, worker pool.
- [x] Minimal window with the viewport, orbit/pan/zoom, and the HUD's sample counter.
- [x] OIDN integration with albedo/normal guides.
- [x] Analytic tests: white furnace, χ² for all three BSDFs, MIS weight validation.

**Exit criterion:** a sphere on a ground plane under an HDRI, orbitable at ≥ 30 fps with a denoised image that is visually converged within 2 seconds; furnace and χ² tests green.

#### M0 implementation notes

Recorded per §2 ("if the implementation diverges for a good reason, change the spec in the same commit"). Four deviations, all deliberate:

1. **Sampler dimension allocation.** §01 §4.2 allocates 6 dimensions per bounce from dimension 6. The implementation allocates **8 per bounce from dimension 8**, so that each bounce lands inside one Sobol′ padding group and its light-selection, light-sample, lobe-selection and direction samples stay jointly stratified. A 6-wide allocation straddles group boundaries and silently loses that for half of all bounces.

2. **Image I/O.** §02 §5 specifies OpenImageIO. M0 uses pure-Rust decoders for Radiance and OpenEXR, which is all the environment loader needs and keeps M0 free of a third native dependency. OIIO arrives with the texture cache at M2, where its tiled MIP pipeline is the actual reason to want it.

3. **Colour management.** §01 §7 specifies OpenColorIO v2 with the ACES studio config. M0 implements the same display transforms analytically (AgX, ACES, Khronos PBR Neutral, sRGB, Raw). OCIO arrives at M3; the analytic versions then serve as the reference to check it against.

4. **Dielectric energy compensation.** §02 §2.3 bans Schlick's approximation from shading. The split-sum energy table needs a Fresnel term that is affine in `F0`, which only Schlick is — but at dielectric `F0` values Schlick's grazing error is 0.06, a 3% energy leak in the layered furnace test. Dielectrics therefore get a third table axis over IOR and are integrated against the **exact** equations; conductors keep the 2D split-sum, where `F0` is high enough that Schlick's error stays under a percent.

The throughput gap reported at the end of M0 is closed, and the report was
wrong. It measured the demonstration scene, which is not what §15's targets
are stated against — they are stated against `bench/simple_product`, which did
not exist. It does now (`tequila --bench simple_product`), and on 16 cores
every stated target is met:

| Metric | Target | Measured |
|---|---|---|
| Samples/sec, `simple_product`, 1080p | >= 8 M | **10.4 M** |
| Time to first pixels after a camera move | <= 50 ms | **17 ms** |
| Interactive frame rate | >= 30 fps | **59** |
| BVH refit after a transform change, 5M tris | <= 20 ms | **0.4 ms** |
| Scene load, 5M-triangle assembly | <= 15 s | **0.46 s** |

Two things are worth recording about how those numbers were reached, because
both are ways a benchmark can lie.

The first-pixels figure was initially reported as 249 ms, a fivefold miss. It
was measuring a full-resolution pass. §15's number is about what a user waits
for *after a camera move*, and a camera move puts the scheduler into the
interactive regime at reduced resolution. Measuring the wrong regime produced a
number nobody ever waits for.

The `bvh build` figure of 441 ms was initially compared against §15's 200 ms. That
200 ms is for a **rebuild after a transform change** — and a transform change no
longer rebuilds anything, it refits, in 0.4 ms. The initial build belongs under
scene load, against 15 s. A benchmark that compares a number to the wrong target
raises false alarms, which is only slightly better than raising none.

Investigating the build time did produce one finding worth keeping. Embree's
build quality is the whole story for BLAS construction and almost irrelevant for
the TLAS: dropping the per-mesh trees from HIGH to MEDIUM takes `cad_assembly`
from 441 ms to 71 ms but costs 30% of traversal throughput, while the top-level
setting is within noise either way. HIGH is kept — a scene is built once and
traced for minutes — but the trade is now measured rather than assumed.

The remaining scenes in §4.1 (`glass_jewelry`, `textured`, `interior`,
`animation`) measure features that do not exist yet. Adding empty placeholders
would produce numbers that look like measurements and are not.

The CI performance gates of §4.2 are still to build: the benchmark reports, and
nothing yet fails on a regression.

The other M0 concern — that adaptive sampling was retiring pixels with nothing
checking the result — is now closed.
`adaptive_sampling_converges_to_the_same_image_as_brute_force` renders one
scene twice, once with retirement disabled, and compares **only the pixels that
retired**: they are the ones the criterion made a claim about, and including
the rest would dilute the signal with ordinary Monte Carlo noise. They agree
with the converged reference to an RMS relative error of 0.002, worst case
0.013.

The reported convergence percentage could never have caught a problem here,
because it is computed from the same variance estimate it would have to audit.
A comparison against a run that does not use that estimate is the only check
that does not assume the thing it is testing.

### M1 — Scene, document, and I/O foundation
- [x] `SceneDocument`, `SceneTree`, `GeometryStore` with instancing (§03 §2).
- [x] Command bus, undo/redo, dirty classification, command journal (§04 §8).
- [x] Import: OBJ, STL, glTF/GLB with the full post-process pipeline (§03 §6.1). **FBX is out of scope** — see note 2.
- [x] Instancing detection by content hash.
- [x] Scene units with both conversion modes (§03 §3).
- [x] `.tequila` container: write, read, schema validation, version migration hook (§03 §4).
- [x] Scene tab: tree, visibility, lock, rename, search, group/ungroup, reparent.
- [x] Transform gizmo with snapping and numeric entry (§04 §5).
- [x] Selection model with all four levels and viewport↔tree sync (§04 §4).
- [x] Autosave + crash recovery from the journal.

**Exit criterion:** import a multi-part glTF assembly, reorganize it, save, reopen, and get a byte-identical document; undo/redo across 50 mixed operations leaves the document identical to the start.

**Met.** `crates/tequila-format/tests/round_trip.rs` imports a four-node glTF
assembly, groups, renames, hides, locks, moves, adds metadata, unlinks a
material and converts units, then asserts the second save is byte-for-byte the
first. `crates/tequila-scene/tests/undo.rs` runs fifty mixed operations and
asserts the document returns to its starting state — see note 1 for what
"identical" means there.

#### M1 implementation notes

Recorded per §2. Six deviations, all deliberate:

1. **"Identical" for undo excludes the ID allocators.** §03 §2.3 forbids reusing
   an ID, so undoing a command that created a node cannot rewind the counter —
   redoing and then editing would collide with the very IDs it handed back. A
   fully undone document is therefore not `==` its starting state; it has spent
   more IDs. `SceneDocument::describes_the_same_scene_as` is the comparison the
   exit criterion is written against. Strict `==` remains correct for the
   save/reopen round trip, where the counters are in the file.

2. **No Assimp, and FBX is out of scope.** §03 §6.1 names Assimp and eight
   formats. OBJ, STL and glTF/GLB are implemented directly, for the same
   reason the HDR and EXR readers are (M0 note 2): the library crates carry no
   third-party Rust dependencies, and a C++ build dependency to reach three
   formats of a few hundred lines each is a poor trade.

   **FBX is a separate decision, and the answer is no.** It is Autodesk's
   proprietary format — undocumented, with no published specification, and
   reachable only through their own SDK or a reverse-engineered reader.
   §0.2 already puts the equivalent proprietary CAD formats out of scope
   for exactly this reason, and FBX belongs with them. The cost to a user is
   small: every tool that writes FBX also writes glTF, OBJ or USD, so this is
   one export menu rather than a lost model. The importer refuses it by name
   and says which formats to use instead, rather than failing as though the
   file were corrupt.

   PLY, 3DS, Collada and 3MF remain M8's business alongside the CAD kernels.

3. **Instancing hashes topology, not positions.** §03 §6.4 says to hash
   `(positions, indices)` after translating to the centroid. That cannot work
   in `f32`: subtracting a centroid of magnitude 5,000 leaves about a
   thousandth of a unit of rounding, so two copies of a part placed apart in an
   assembly hash differently — the exact case the hash exists to catch. The
   hash is over the index buffer and channel presence, and §6.4's required
   full-buffer confirmation is what decides, with a tolerance scaled by both
   the shape's size and its distance from the origin.

4. **Node transforms are stored as a matrix.** §03 §4.2 sketches a
   `{t, r, s}` triple. A TRS cannot represent a shear, and nested non-uniform
   scales in an imported assembly produce one. Decomposing would be lossy, and
   a lossy transform does not merely break the byte-identical round trip — it
   moves the user's geometry.

5. **Tangents are not MikkTSpace.** §03 §6.1 point 4 asks for MikkTSpace. What
   is implemented is the conventional per-triangle accumulation with
   Gram-Schmidt orthogonalization, which agrees on smooth surfaces and diverges
   at UV seams and mirrored islands. Nothing consumes tangents until M2's
   normal mapping, so the difference is not yet observable; **M2 must replace
   this before it is.**

6. **glTF's `metallicFactor` is thresholded.** §03 §6.1 point 8 requires glTF's
   PBR parameters to be mapped exactly. Base colour, roughness and emission
   are. Metalness is a continuous blend in glTF and `MaterialKind` is still a
   discrete choice, so it thresholds at 0.5 and warns when the value falls in
   the ambiguous middle. M2 gives `base_metalness` a real parameter and this
   becomes exact; until then an imported metal also ignores glTF's colour tint,
   since the conductor's reflectance comes from its complex IOR.

One correctness note against the dirty taxonomy, fixed rather than deferred:
a gizmo drag issues a transform change per frame, and the renderer was
rebuilding every per-mesh BLAS for each one. `docs/01` §9.3 classifies an
instance transform as needing the TLAS rebuilt and nothing else, which is now
what happens. Measured on a 4,000-instance, 40-mesh assembly: 54 ms to rebuild
against 3.3 ms to update in place, which is the difference between a drag that
stutters and one that does not.

Both of M1's known gaps are now closed. The file dialog is the platform's own
(via `rfd` — the app crate already depends on eframe and egui, so the
no-third-party-crates rule was never true of it; it holds for the libraries,
which is where it matters). And the transform gizmo moves a whole selection,
because `scene.set_transform` takes a transform per node, so a multi-part move
is one command and one undo step.

### M2 — Material system (the headline feature)
- [x] OpenPBR Surface: all lobes, layering, energy compensation LUTs (§02 §2).
      Subsurface is a random walk (§01 §6.4, since September 26): the
      subsurface share of the base enters the part through a white
      Lambertian transmission, walks a homogeneous medium converted from
      colour and radius by Chiang et al.'s fit to Christensen & Burley,
      with throughput-weighted channel sampling and Henyey-Greenstein
      scattering, and leaves through the part's own surface. Tested for
      energy (a white part in a white world reads 0.999), for colour (a thick
      part returns what it was set to, within 5%) and for translucency (a thin
      plate lit from behind glows in front, where the diffuse stand-in stayed
      black). The same walk scatters inside clear volumes
      (`transmission_scatter`): a milky liquid, a smoky glass.
- [x] Material catalog: all 34 types as presets with their exposed-parameter sets (§02 §3).
- [x] χ² sampling tests over the whole catalog and a roughness sweep (§3.2),
      added to the existing analytic suite rather than beside it.
- [x] The document, the command bus, the undo journal, the file format and both
      importers carry the full parameter block. `Param` is the one reflection
      table they all read, so a new parameter is a row in it rather than four
      matching arms that can drift apart.
- [x] Material store, instances, linking/unlinking, reference counting (§02 §8.2).
- [x] Material graph data model + compiler to the register-machine IR (§02 §4.3),
      wired through the document, the file format, and the integrator: a
      compiled program runs at the shading point and its textures reach the
      image.
- [x] Material tab: parameter panels driven by the preset's exposed set;
      reference-count badge; `material.set_type` with best-effort parameter
      transfer; the §4.2 texture toggle beside every parameter a map can drive.
- [~] Texture pipeline (§02 §5). MIP pyramids with footprint-driven filtering,
      colour-space assignment by map role, UDIM tile sets, and a
      memory-budgeted LRU cache — in Rust rather than over OIIO, which is a
      deliberate deviation recorded in `crates/tequila-texture/src/mip.rs`.
      **Outstanding:** tile-level residency and async load.
      *Done 2026-10-30:* formats beyond PNG/EXR/HDR, all in pure Rust
      behind `tequila_texture::image::Image::load` — JPEG, PSD and PSB;
      TGA and BMP read by hand (`tga.rs`, `bmp.rs`); TIFF at 8, 16 and 32
      bits through the `tiff` crate; WebP lossy and lossless with alpha
      (`image-webp`); GIF's first frame; DDS, its header read by hand and
      BC1–BC7 decoded by `bcdec_rs`; JPEG XL (`jxl-oxide`); and SVG
      rasterized by resvg with the system's fonts (`svg.rs`), a physical
      width and height giving a label its print size as a PSD's DPI does.
      The texture, pin and label pickers and the importers' format checks
      take them from the one list (`Image::DECODABLE`); the texture-pack
      picker and window drop take the map formats among them. HEIC and
      AVIF are not read: their decoders are C libraries.
- [x] Projection modes incl. tri-planar (§02 §5.3), with derivative-aware
      cylindrical seam handling and real-world units. The Mapping 2D node
      exists in the graph; its *editor* arrives with the graph editor in M6.
- [x] Image texture nodes + the core utility nodes (mix, multiply, add, invert,
      remap, clamp).
- [x] Bump / normal mapping with ray-differential-scaled gradients (§02 §5.4),
      including Bump Add summing gradients rather than heights.
      *Correction, M6:* until then this was the math and its tests only —
      nothing in the renderer called it, and every normal map and bump
      rendered flat. `render-core/src/surface_bump.rs` now applies both in
      the path tracer, with a Bump node giving heights a depth in scene
      units. Tangents come per triangle from the UVs, not MikkTSpace (see
      note 5 above), and the Solid viewport does not show bump yet.
- [x] Library window (§04 §9): categories, search over name/tags/category,
      favorites, recently-used, in-project, and thumbnails rendered by the real
      renderer, lazily and on a per-frame budget. A writable user library
      ("Save to library…") and the right-click menu have since landed, and
      *since September 26* thumbnails are kept on disk per build of the
      application (`thumbnail::DiskCache`), so the library is rendered once
      rather than on every launch, and a new build renders it afresh.
- [x] **Drag-and-drop application with the full modifier semantics and hover
      preview** (§04 §3): Alt unlinks, Shift widens to the group, Ctrl to
      everything sharing the material, and the floating label states the
      consequence before the drop.
- [x] Tequila Standard Library v1: 59 materials, all as JSON presets.

**Exit criterion:** drag five different materials onto parts of an imported product model; each applies in ≤ 100 ms with correct linking behavior, and the χ² test passes for every material type in the catalog.

**Met.** Both halves are tests: `tequila-format/tests/m2_exit_criterion.rs` for
the application and linking behaviour, and
`tests/analytic/tests/chi_squared.rs` for the sampling.

### M3 — Environment, camera, and look
- [x] Environment tab: brightness, contrast, size, height, rotation; three background modes.
- [x] Ground plane: shadows, occlusion shadows, reflections, flatten, size; shadow-catcher alpha.
- [x] Physical lights: area, point, spot, IES with correct sampling (§01 §5.1).
- [x] Light BVH / alias table selection (§01 §5.3); light visibility flags; light layers.
- [x] Camera tab: all four lens types, focal length/FOV, DOF with Keep Focus, ground grid.
- [x] Match Perspective (2- and 3-point).
- [x] Multiple cameras, switching, saved views.
- [x] OCIO v2 pipeline, ACEScg working space, display transforms (§01 §7). The ACES 1.3 studio config develops end to end, `BuiltinTransform` included; the ACES 2.0 output transforms are reported, not built.
- [x] Image tab: Basic and Photographic image styles, applied as a re-tonemap with **no re-render** (§01 §9.3).
- [x] Lighting tab: the five presets + Custom, wired to the integrator config table (§01 §11).

**Exit criterion:** a product scene lit only by an HDRI reads as a photograph; dragging the exposure slider updates at 60 fps with zero accumulation loss.

**Met**, in the two halves the criterion is stated in.
`crates/tequila-render-cpu/tests/m3_exit_criterion.rs` renders a product lit
by nothing but the studio environment and checks the properties the word
"photograph" stands on: exposed for its own highlight, nothing clips to flat
white; nothing on the product is black, including the silhouette; the product
casts a contact shadow rather than floating; and the noise falls by more than
half for eight times the samples.

The second half is a measurement, and it was failing badly — the re-develop
took **381 ms** at 1080p, twenty-three times its budget. Four changes, each
measured, brought it to **15.1 ms** on sixteen cores:

| Change | 1080p, AgX |
|---|---|
| As written | 381 ms |
| Parallel over pixels | 33 ms |
| Writing into a buffer instead of collecting into one | 17 ms |
| sRGB encoded by a table of code boundaries, not `powf` | 16 ms |
| `exp2` of the exposure hoisted out of the loop, and a fast `log2` for AgX | 15.1 ms |

The second of those is worth singling out. A parallel `collect` into a
`Vec<u8>` cannot know each worker's share in advance, so it builds per-worker
buffers and concatenates them — and that cost more than the tone mapping it was
carrying. The measured floor with no tone curve at all was 12 ms before the
change and 4.7 ms after.

The test does **not** assert 16.6 ms, and says why in its own comments: under
`cargo test`, with a dozen other binaries competing for the same cores, the
same code measures 18 to 20 ms. An absolute wall-clock assertion would pass or
fail on machine load, which is a test everyone learns to ignore. What it
asserts instead survives contention because both sides are measured under it —
the full pass against the same pass with no tone curve — and every one of the
three regressions above would have failed that bound.

Zero accumulation loss is structural rather than measured, and had quietly
broken when the image style moved into the document: a fresh snapshot is a
fresh `Arc`, the render thread compares scenes by pointer, so rebuilding the
snapshot *is* a restart whatever its contents. `Project::apply` now consults
§9.3's dirty taxonomy and an edit at or below `Dirty::Display` never rebuilds.
`an_image_style_edit_does_not_disturb_the_render` asserts both directions.

#### M3 implementation notes

**Colour management develops through the ACES studio config, and the one gap
left is ACES 2.0.** ACEScg has been the working space since M0 and the five
display transforms are analytic, as `docs/05` §1 note 3 intended. The OCIO v2
side reads a config and evaluates it — a YAML subset, the
`.cube`/`.spi1d`/`.spimtx` LUT formats, and the transform graph in both
directions — and `a_config_that_encodes_srgb_matches_the_analytic_transform`
checks the result against the analytic transform that note 3 said would be the
reference.

It now also reads what makes a config a v2 one: display-referred colour spaces
against the CIE XYZ D65 connection space, view transforms, shared views,
looks, aliases, data spaces and the active display and view lists. And it
implements `BuiltinTransform`, the registry OCIO v2 moved the ACES transforms
into (`tequila-color/src/ocio/builtin.rs`): the ACES 1.0 SDR and 1.1 HDR output
transforms written out from the Academy's CTL — RRT, the C5 and C9 splines, the
Single Stage Tone Scale, surround and gamut-limit steps, the D60 and D65
simulations — the ACES log encodings, the 1.3 Reference Gamut Compression and
the blue-light fix, the display encodings (sRGB, Rec.1886, gamma 2.2/2.6, PQ,
HLG, DCDM) and the ARRI, Sony, RED, Canon, Panasonic and Apple log cameras.
`tests/aces_studio_config.rs` loads a hand-written subset of
`studio-config-v1.0.0_aces-v1.3_ocio-v2.1` (the published file could not be
fetched, so the fixture is written to its structure and names), builds all 47 of
its views and develops an image through the default one.

The maths is checked against values whose source is stated in each test: the
AP0/AP1 matrices and ACEScct/ACEScc breakpoints of the Academy specifications,
the camera vendors' own matrices and grey code values, the sRGB, PQ and HLG
standards, and the SSTS reproducing the RRT's published coefficients. The
output transforms' grey, black and peak values are *derived* from the CTL
constants, not read from a published table, and the tests say so. What has not
been done is to run the real published config, or to compare against OCIO's
own output; both want a machine with OCIO on it.

The ACES 2.0 output transforms of OCIO 2.4 (`..._2.0` styles) are still
*reported* as unsupported, by name, and the built-in transforms stay available.
Their tone scale, chroma compression and gamut compression are a body of work
of their own, and an approximation would be the quietly wrong picture OCIO
exists to prevent. An ACES view also costs more than the analytic transforms —
about 0.6 µs a pixel on one core, some 75 ms for a 1080p re-develop on sixteen
— because it evaluates the CTL rather than a LUT baked from it.

Two smaller gaps, both carried forward deliberately:

- **Light layers** are stored per light, serialized, and exposed in the panel,
  but nothing yet *isolates* a layer's contribution. Doing that means light
  path expressions and AOVs, which are M4's §12, and building half of it here
  would be a second implementation to throw away.
- **The HDRI Editor** of §1.5 — the pin-based lighting compositor — is not
  started. It is marked Pro in the spec and is a tab of its own rather than a
  part of any item above.

Three findings worth recording, because each was invisible until something
specific was checked:

1. The light hierarchy's emission cone was parameterised with a falloff of up
   to π, so `cos(θ′)` went negative for a point light seen from behind its
   arbitrary axis — a *negative* importance, which made one child's share of a
   split exceed one and stopped the traversal probabilities being a
   distribution. The analytic sum over leaves was still exactly 1, which is why
   the first version of the test passed. The test now checks both that the
   probabilities sum to one and that the frequency each light is actually
   chosen with matches the probability it reports; only the second catches it,
   and the estimator divides by the reported value.

2. Giving an analytic light a visible emissive proxy doubles it. The proxy is
   found by BSDF sampling *and* the analytic light by next-event estimation, so
   a softbox came out exactly twice as bright the moment it was made visible —
   a change that looks like it should only affect what is in frame.
   `Instance::light_proxy` gates the proxy's emission to paths the camera can
   still see along.

3. The ground plane was reallocated on every snapshot, which broke the `Arc`
   identity the backend uses to decide it can update transforms in place. One
   mismatch sends the whole scene down the rebuild path, once a frame, with
   nothing failing. A *fitted* ground size is now measured once and held until
   a geometry edit clears the cache, while a size the user typed still takes
   effect at once.

### M4 — Render output
- [x] Render settings: max samples, max time, custom control with every parameter (§01 §11).
- [x] Adaptive sampling with per-pixel variance halting and the converged-percentage HUD readout.
- [x] Firefly control: path regularization, outlier rejection with the clamped-energy AOV, optional clamping.
- [x] All AOVs via light path expressions, including spec-compliant **Cryptomatte** (§01 §12).
- [~] Render layers with the three alpha modes.
- [x] Region rendering.
- [x] Output formats: PNG, JPEG, TIFF/TIFF32, EXR (half/full, multi-layer), PSD/PSD16/PSD32 layered.
- [x] Print-size calculator; filename templating; metadata sidecars (XMP + simple).
- [x] Render window, queue with persistence, and gallery with A/B compare (§04 §11).
- [x] Denoiser: guide prefiltering, blend strength, cadence control.

**Exit criterion:** render a 4K image with 12 passes; open the EXR in a compositor and confirm Cryptomatte mattes select correctly and that Beauty = sum of its component passes within floating-point tolerance.

**Met.** `crates/tequila-format/tests/m4_exit_criterion.rs` renders the scene,
writes the EXR, and then reads it back — and the reading half deliberately
reaches into none of Tequila's own structures. It takes a file's channels and
a part's name, hashes the name the way the file's own metadata says to, walks
the ranked layers and gathers coverage. That procedure *is* the Cryptomatte
format; a compositor's node does the same thing, so if it produces the right
matte here it will produce the right matte there.

`the_full_4k_render` does the criterion's own numbers and is `#[ignore]`d
because half a minute and two gigabytes do not belong in the path of every
`cargo test`. Run by hand, 3840 × 2160 at twelve passes takes **25.7 s** and
writes a **122 MB** file, and the worst component mismatch across all 8.3 M
pixels is **7.4 × 10⁻⁴** — that is half-float quantization and nothing else,
since `passes.rs` holds the same identity to 1 × 10⁻⁴ before the file rounds
it.

ZIP compression paid for itself twice over. The file went from 1471 MB to
122 MB, and the render went from **46.8 s to 25.7 s**, because at that size
writing 1.5 GB to disk cost more than compressing 1.5 GB did.

#### M4 implementation notes

**Render layers are the one incomplete item, and the gap is specific.** All
three alpha-edge modes of §1.8 are implemented and tested — Straight,
Premultiplied, Aliased — over any layer's coverage buffer. What is not built is
the other half of the spec's sentence: assigning *parts or models to named
layers rendered as separable images*. That is a scene-subset visibility
mechanism rather than an output one, it needs a place in the document and the
tree panel, and it is the natural companion to the light-layer isolation
carried over from M3. Both are listed for M6.

Three findings worth recording:

1. **An NFA accepts on the state that consumed the last event, not on where it
   ended up.** The first light-path expression matcher tested acceptance
   against the state set it had just stepped *into*, which is the set of places
   the machine could go next — so `C.*L` matched nothing at all, because after
   consuming the light there is nowhere left to go. `Lpe::step` now returns the
   pair `(consumed, next)` and acceptance is tested against the first.

2. **§4.6's order independence applies to the Cryptomatte tally.** The first
   version ranked identifiers into six fixed slots as samples arrived, which
   means a seventh identifier covering most of a pixel could never displace six
   holding one sample each — the ranking depended on arrival order, and
   arrival order is a tile-scheduling detail. Replaced with a per-pixel tally
   ranked once at the end, so the same pixel produces the same mattes however
   the tiles were handed out.

3. **Straight alpha has to carry edge colour outwards, not just divide.** The
   renderer produces premultiplied colour; straight alpha divides coverage back
   out, and at a pixel with zero coverage there is nothing to divide. Leaving
   those black gives exactly the dark fringe that straight alpha exists to
   prevent, so the edge colour is extended into them.

Two of the tests here were wrong before the code was, and both were fixed by
being restated rather than loosened. The clown-colour test demanded every
adjacent pair of identifiers differ by more than 0.1, which is demanding that a
hash not behave like a hash — at 256 identifiers about one collision is
expected. It now asserts a mean adjacent distance and a bound on how many pairs
may be close. The coverage test asserted a pixel's Cryptomatte coverage sums to
one; it sums to the *geometry* fraction, and it is now compared against the
framebuffer's own alpha.

### M5 — Animation and physics (deferred; optional)

**Deferred by the project owner, and not on the critical path.** Tequila is
being built for still product renders first, and every item below serves a
different product. Nothing in M6 to M9 depends on any of it: the animation
document would be a new top-level member of the scene document, the timeline a
new panel, and motion blur an addition to the integrator — none of which
anything else reads.

Two consequences worth stating so they are not discovered later. §01 §8.4's
multi-segment motion blur is the one piece that touches the renderer's hot
path, so taking it later means revisiting the integrator rather than
retro-fitting a panel. And the Animation nodes of M6 (Color Fade, Curve Fade)
are animation by another name; they are deferred with the rest.

Picked up when stills are done and someone wants a turntable. The shortest
useful subset is the animation document, part and camera tracks, a timeline,
the wizard's turntable and exploded view, and sequence rendering — physics and
Alembic deformation import serve neither.

- [ ] Animation document, tracks, keyframes, interpolation modes, evaluation at time *t*.
- [ ] Part, camera, and environment animations.
- [ ] Timeline UI with blocks; curve editor with per-keyframe toggle.
- [ ] Motion blur (transform and deformation) via Embree multi-segment motion (§01 §8.4).
- [ ] Motion ease curves.
- [ ] Animation Wizard: turntable, exploded view, fly-through, part reveal, studio sequence.
- [ ] Physics: Rapier integration, rigid bodies, mass/friction/restitution, joint constraints (hinge, slider, ball, fixed), bake-to-keyframes.
- [ ] Animation rendering: per-frame output, samples-per-frame consistency, resumable sequences.
- [ ] Deformation animation import (Alembic/FBX).

**Exit criterion:** a 10-second exploded-view animation renders with temporally stable denoising (no visible flicker) and correct motion blur.

### M6 — Pro-tier parity
- [x] Material Graph editor: full node palette, live node previews, groups, reroutes, solo (§04 §7).
      Reroutes carry their source's type and are joined out before
      compiling, so a bent wire compiles to exactly the straight one
      (tested, mapping-through-a-reroute included). Groups are titled boxes
      saved with the graph and outside its content hash; dragging the title
      carries the nodes, Tidy refits each round what it held. Solo shows one
      node's output on the model, unlit, as a view override — never saved,
      never undone, cleared when the editor closes.
- [x] All remaining procedural texture nodes with the algorithms in §02 §6.
      *Done:* Wood (Advanced) — uneven years, earlywood/latewood, ray
      flecks, knots, planks; Occlusion — per vertex, against the part
      itself, only for parts whose material asks, cached per mesh and
      radius; Vertex Color — glTF COLOR_0 and OBJ's `v x y z r g b`,
      carried through welding, crease splitting, part splitting,
      displacement and `.fsmesh`, and keeping differently painted copies
      from being taken for instances. *Volume Map since September 26:*
      OpenVDB files through `vdb-rs` (vendored, with a fix for half-float
      grids), read into a dense grid over the active box (held to 256³) and
      fitted to the part's bounding box; the node drives a new Density
      parameter, and the walk tracks the medium with null collisions against
      the grid's densest. Tested with files written by OpenVDB 12.0 itself
      and against renders (a fog sphere clears an absorbing cube's corners,
      a constant grid matches the same Density, a patchy scattering medium
      conserves a white world). A scattering event inside an index-matched
      medium (a Scattering Medium's) samples the sky and the lights directly
      — through the medium, by ratio tracking where it varies — weighted
      against the phase function by MIS, so smoke in sunlight is lit rather
      than speckled (tested: the median pixel was 0 against a mean of 0.53
      before). Null collisions stay on their ray instead of retracing it,
      tracking steps through a majorant grid of 8³-voxel blocks (each
      block's bound tested never to fall below a lookup, and the test caught
      a bound without its one-voxel rim), and long walks and deep shadow rays
      end by Russian roulette: a dense cloud renders in 43 s where it took
      193. The Material tab's sphere fits the grid to itself, and the GPU
      Solid view marches it through the part (a 3D texture, front to back,
      with a few taps towards the key light for self-shadowing), so a VDB
      shows its shape in every view. The walk may take 16384 steps: at 256,
      dense milk in a glass-sized ball kept 0.83 of a white world's light,
      now 0.98 to 1.00. *Open:* the CPU Solid fallback still shows such a
      medium at its described density.
- [ ] Geometry nodes: Displace with adaptive tessellation, Flakes, Fuzz, Bubbles.
      *Done:* Displace, Flakes, and Bubbles — real air pockets rather than
      §4.4's density field, which reads as fuzzy specks: inside-out spheres
      in the part's own material, so a ray meeting one leaves the liquid
      into air with the right refraction and total-internal-reflection rim,
      placed wholly inside the (closed) part without overlapping, seeded
      and cached. *Open:* geometric Fuzz (shading-only fuzz is the fuzz
      lobe); curve primitives are not in the Embree integration yet.
- [ ] ~~Animation nodes (Color Fade, Curve Fade)~~ — deferred with M5.
- [x] HDRI Editor: light/image/gradient pins, sun & sky (Hosek-Wilkie), export to EXR, async CDF rebuild.
      *Hosek–Wilkie since September 26:* the authors' 1.4a reference code
      and data, ported (`render-core/src/hosek.rs`) and held to that code's
      own output — compiled unmodified, 540 sky and 264 sun values — to nine
      digits (`tests/hosek_reference.rs`). The sun is their spectral solar
      function through the CIE observer, checked to share the RGB sky's
      units (their spectral sky converted the same way lands within 1.5% of
      their RGB sky in 21 of 24 cases). A sun below the horizon is outside
      the model and is faded from its sunset sky, a stated approximation.
      *Before that:* light, image and gradient pins with add/cover
      blending and a lighting/background split (a backdrop gradient changes
      what is behind the product without touching its lighting); export of
      the composite to `.exr` (32-bit, so a sun is not clipped at half-float's
      65504) or `.hdr` at 1K–8K; skies and images built on a worker thread
      while the last environment keeps being served. The sky was then a
      single-scattering model, which is why the item stayed open until
      Hosek–Wilkie's own replaced it.
- [x] Labels and decals: full layer stack, Move Texture gizmo, PSD DPI sizing, video labels, Labels AOV.
      *Done 2026-10-20:* PSD and PSB artwork by its merged image, or
      composited from its layers when it was saved without one; and video
      labels: a frame picked from an MP4, MOV and the rest the system plays,
      decoded by the system (Media Foundation, AVFoundation) and kept as a
      PNG beside the project (`tequila_texture::video`). *Before:* an ordered stack of labels per material,
      each with its own projection, opacity, roughness and a Foil finish
      (metal where the artwork covers); a placement gizmo — click or drag
      to put the artwork there, Ctrl + scroll to size it, Shift + scroll to
      turn it, one undo step per gesture; PNG and JPEG artwork arrives at
      the print size its file records (`pHYs`, JFIF density; since
      2026-10-30 a TIFF's and a BMP's resolution, and an SVG's width and
      height in millimetres, inches or points, too); and a Labels
      pass (RGBA, premultiplied, coverage as alpha) in the EXR. A label that
      plays as video, frame by frame through an animation, waits on
      animation (parked).
- [x] Cutaway: plane/box/sphere/cylinder/mesh cutters, cap shading.
      A per-ray CSG walk inside the intersector, so every query — camera,
      bounce, shadow, picking, the fast preview — sees the cut model: hits
      inside a cutter are stepped over, and a cutter's boundary is a surface
      (the cap) where the next surface beyond it is met from behind, which is
      exactly "inside a closed solid". Caps are drawn at the cut, in the cut
      part's material or a colour of the cutter's. Cutaway nodes are placed
      sized to the model and facing the camera; a part given the Cutaway
      material cuts with its own mesh (a small BVH, parity for inside). The
      ground is never cut. The Solid view clips against the analytic cutters
      and fills the opening flat in the cap colour; mesh cutters are the
      renderer's alone.
- [x] Measurements tool with snapping to BREP edges and vertices.
      *Without BREP*, which is out of scope: the edges are recovered from
      the tessellation — a crease over 25° or a boundary, on position-welded
      vertices, so a CAD mesh split at every crease still has its edges —
      and a tessellator's interior lines are never offered. Snaps to a
      corner, then an edge's middle, then along an edge, then the surface,
      by distance on screen. Measurements are saved, anchored in their
      parts' own space so they follow a moved part, and read in the scene's
      unit.
- [x] Multi-Materials; Model Sets with Always-Visible support; Studios.
      One Studios tab (`U`). Variants write the active material into their
      parts, so nothing else has to ask which variant is in use; a copy of
      the active variant is made by the existing Unlink, in the same undo
      step. A node in a model set shows only with one of its sets or an
      always-visible one; a node in none always shows. A Studio holds any
      of camera, environment, image style, model set and variant choices,
      and applies them as one step. *Not yet:* Studio thumbnails and drag
      reordering (arrows stand in).
- [x] Render Layers: parts and models assigned to named layers rendered as separable images (M4 built the three alpha modes).
      A layer is its own render of the same shot, to the samples the main
      image reached, with every non-member hidden from the camera and only
      the camera: they still shadow, light and reflect, so layers stack
      back without holes. Written as `<image>_<layer>` in the main image's
      format and alpha mode. The integrator now honours `visible_to_camera`
      on camera rays, which nothing did before.
- [x] Light layer isolation, on M4's light-path machinery (M3 stores the layers).
      Light events carry a group — layers 0–7, the environment (with sky,
      pins and the shadow-caught floor), glowing materials — matched by
      `L0`…`L7`, `LE`, `LM`. A render with passes gets one per group the
      scene uses; they sum to Beauty, which a test holds to 1e-4. "Keep
      light groups for relighting" keeps each group denoised after the
      render, and the Render window rebalances and tints them live and
      saves `…_relit`. Found on the way: the shadow catcher reached the
      image and no pass, so Beauty had no floor.
- [x] Material Templates (rule-based bulk assignment with a preview table).
      Rules by part name, current material name (wildcards, not regex — a
      CAD part list is full of regex syntax, and no regex engine is
      bundled), current base colour within a tolerance (where a CAD colour
      lands), or a metadata field such as the CAD layer. First match
      decides; the preview lists every part that would change and which
      rule decided it; applying is one undo step. Saved with the project.

**Exit criterion:** reproduce three reference product renders (a consumer electronic, a bottle with liquid, a piece of jewellery) end-to-end using only Tequila features.

*Met, 2026-09-25:* `scripts/reference/render.py` builds all three through
the scripting channel and nothing else — a spirits bottle (solid glass, an
amber liquid modelled into it, a wrapped paper label with a foil layer, an
oak stopper, backlit), a solitaire (18k gold, a round brilliant, the
Jewelry preset in a light tent) and a Quest 3 from a glTF (not shipped). The
bottle could not be made before this: it needed solid glass that did not tint
twice, a priority medium stack for the liquid in its glass (`docs/01` §6.1),
index-matched interfaces that pass light, and renders written opaque.
*Since September 26* a diamond has fire: dispersion is implemented (below,
under M10's spectral item).

### M7 — Distribution and automation
- [x] Configurator: wizard, option model, validity rules, custom layout.
      Options switch a Multi-Material's variant, the Model Set or the Studio;
      rules exclude or require a choice, and a pick the rest rules out moves
      the others to the nearest valid configuration (`tequila_scene::configurator`).
- [x] Presentation mode (full-screen interactive): Workspace → Present or F5;
      the picker over the rendered view, placed left, right or along the bottom.
- [x] Web export: live-3D and pre-rendered modes; static output folder.
      The live view is Tequila's own WebGL2 viewer (`crates/tequila-format/web/`)
      rather than three.js: no library, no CDN, and it opens from the disk.
      It draws the glTF export (`tequila_format::gltf`: OpenPBR mapped to
      glTF PBR and its extensions, procedurals, triplanar images and labels
      baked by the renderer's own `shade` into the mesh's UVs or an atlas,
      Multi-Materials as `KHR_materials_variants`), lit by the environment
      prefiltered for GGX plus spherical harmonics, developed with the
      document's display transform, with a baked ground shadow. Photos are
      one queued render per combination, named as the page expects.
- [x] Batch render of all configuration combinations with time estimation:
      "Render all N…" estimates from finished renders' seconds per sample per
      pixel (or the time limit), then queues one captured job per combination,
      named by `{configuration}`; `queue configurations=true` from a script.
- [x] Spin-viewer (XR) export with the embeddable JS viewer (§03 §8):
      File > Export spin viewer, or `export format=spin`. Columns round and
      rows of tilt from the current camera, queued as renders (PNG cut out
      when transparent); a folder with `index.html`, `spin.js` (also a
      `<tequila-spin>` element for embedding), `manifest.json`/`.js`,
      `EMBED.txt`, and `sprites.jpg` made once the frames are in. Drag,
      touch, inertia, zoom, auto-turn, reset. WebP frames are not written;
      JPEG and PNG are.
- [x] Headless CLI renderer (stills; animation waits for M5): a project's
      camera, Studio, model set or configuration (`--camera`, `--studio`,
      `--model-set`, `--configuration`), `--all-configurations` named by
      `{configuration}`, PNG or JPEG, `--silhouette`, and the exports with
      their renders done in-process: `--export-glb`, `--export-web --photos`,
      `--export-spin`. `crates/tequila-cli/tests/headless_configurator.rs`
      runs the exit criterion below at a small size.
- [x] Network rendering: dispatcher, worker protocol, job distribution, monitor UI.
      `tequila-network`: `tequila-cli --worker` serves on a port with
      the farm's token; a dispatcher (`--farm`, or the Render window's
      Network rendering section, or scripting's `queue` with `network`)
      packs the project with every file it reads - textures, HDRI,
      backplate, image pins, IES profiles, each Studio's - and queues one
      job per picture; each node takes the next when free and renders it as
      `tequila -i` would (its own Cycles device, or Tequila's renderer
      where it has no Cycles). Mutual HMAC-SHA256 challenge-response (the
      token never crosses the wire), every frame signed against alteration,
      replay and reflection; bundle members confined to their job folder.
      A dropped job goes to another node (three tries); unreachable nodes
      are retried with backoff and shown offline with the reason. Finished
      pictures land in the render folder and the Render window's filmstrip.
      Not done: splitting one picture across nodes (each still is one
      job), and animation frames (M5 is parked). Unencrypted by design:
      for a trusted network or a VPN.
- [x] Python API over the command bus; `docs/scripting-api.md` published.
      The `tequila` module (standard library only) speaks the scripting
      protocol, to the open application (`connect()`) or to a headless
      Tequila of its own (`open()`, which runs `tequila --serve`: the
      command bus with no window, answering with the application's own query
      code). `commands` is generated from the catalog, every command with
      the journal's example of it, and a test fails when the generated
      module or reference falls behind the build. Not PyO3 embedding: no
      Python inside the application, nothing to build against a Python, any
      3.10+ works, and a script that crashes takes only itself down.
      Embedding remains possible on the same protocol if wanted.

**Exit criterion:** a scripted job renders 24 colourway/camera combinations unattended and produces a working web configurator, with no UI interaction after launch.

### M8 — CAD-grade import
- [x] OCCT integration behind the `tequila-import` boundary. (2026-10-13:
      OCCT 8.0.1 in a helper program, `native/occt` → `occt/tequila-occt.exe`
      in the package, so a file OCCT chokes on costs that process, not the
      window. STEP reads through XDE (assembly, placements, product and solid
      names, sRGB colours) and BRepMesh at the same presets, laid out exactly
      as truck's reading; truck reads the file when the helper is missing or
      fails, and the scene says so. A 57 MB AP242 assembly in metres that
      truck read with 500 faces missing reads in ~40 s with none missing.)
- [x] STEP and IGES import via XDE: hierarchy, names, colours, metadata, layers.
      (2026-10-30: STEP's metadata and layers through XDE: `SetMetaMode`,
      product metadata and `XCAFDoc_LayerTool`, in the helper's answer as
      each body's and product's `attributes`, `layers` and `hidden` (every
      layer it is on switched off), which an older answer simply lacks.
      Attributes - part number, description, material, properties of the
      user's own - are the node's metadata, the Metadata panel's (Scene tab);
      layers are a list of their own on each node (`layers` in the project,
      absent when empty; a project that kept them as the metadata field
      `Layer` opens with them moved across), shown and hidden as groups by
      the Scene tab's Layers list (`scene.set_layer_visibility`, one undo
      step), read by a search or a Material Templates rule as the field
      `Layer`, and given to scripts on every node and as the scene's
      `layers`. truck reads the same entities, so a file reads the same
      without OCCT. IGES through XDE as well (`IGESCAFControl_Reader`:
      names, colours, levels as layers named "Level 5"), but only for a file
      Tequila's own reader cannot read whole - a surface it does not read, a
      face it cannot divide - since the two lay a surface model out
      differently and updating or re-dividing a model matches its parts by
      name; Tequila's reader reads levels too (one, or a 406 form 1 list),
      and gathers only faces on the same levels into a body. IGES has no
      attributes for either reader to give.)
      (Without OCCT, 2026-10-11: STEP keeps its B-rep - the file's bytes in
      the project, `sources/` - for Re-Tessellate (Draft/Medium/High/Ultra,
      Scene tab > Tessellation, an update that keeps every part's work), and
      its B-rep edges, seams left out, which View > Model edges draws in Solid
      and Material Preview; other meshes show their creases there.)
      *Partial:* STEP through truck (pure Rust, no OCCT): the assembly
      hierarchy with its placements, bodies as named parts, body and face
      colours, the file's length unit. IGES (2026-10-11) through Tequila's
      own reader, no OCCT: trimmed surfaces (144) over NURBS, planes, ruled,
      revolved and tabulated surfaces; MSBO solids (186) over NURBS and the
      analytic surfaces; the curves that bound them; matrices, colours,
      names, subfigures (meshed once, placed by each instance), units. Each
      face divided in its own parameters (constrained Delaunay over its
      loops and a curvature grid, exact normals) at the same presets, its
      bytes kept for Re-Tessellate, its boundaries as model edges with seams
      left out; faces meeting at their corners gathered into bodies.
- [x] BREP retention + serialization into the project: the STEP file's own
      bytes as a `sources/` member rather than OCCT's `.brep` (no OCCT), which
      truck re-reads exactly; kept up to 64 MB a file.
- [x] Tessellation presets and the Re-Tessellate tool with async rebuild:
      §6.2's Draft/Medium/High/Ultra linear deflections, on a worker thread,
      applied as an update so materials, labels and names stay.
- [x] BREP edge extraction for wireframe rendering: truck's edge polylines,
      seams left out, stored with the mesh (`.fsmesh` bit 3) and drawn by
      View > Model edges. Measurement snapping still uses the mesh's creases.
- [x] Re-import reconciliation by path key, preserving materials/labels/animation (§03 §2.3).
      *September 27* (`tequila-scene/src/reimport.rs`): every import
      records each node's path key and, on its top nodes, the file. Reload
      from file / Update from another file (the Scene tree's menu, File,
      and `reimport` in scripting) matches by key, then by a name unique on
      both sides with a CAD copy number ignored; a model's one top node is
      keyed as the model, since Fusion renames it after the document on
      every save-as. Matched nodes keep id, name, material, visibility and
      settings and take the new geometry, placement within the model and
      CAD attributes; the model keeps its place. New nodes are added, gone
      ones hidden and listed with an offer to delete. One undoable,
      journalled command. Checked on four revisions of a real Fusion
      export: v2.4 to v3.1 and v3.1 to V3.4 match every part.
- [x] USD import **and export** with MaterialX/OpenPBR round-trip, in the
      text form (`.usda`; binary `.usdc` is refused with the usdcat line
      that converts it). Export: Xforms, Meshes, the camera, Materials with
      both a UsdPreviewSurface and an `ND_open_pbr_surface_surfaceshader`
      carrying every parameter, plus a graph material's whole definition
      when it has no images. Import: Blender-style xformOps, Z up, units,
      vertex/faceVarying/indexed primvars, OpenPBR or preview materials.
      Checked both ways against Blender 5.0's USD (Pixar's library). Not
      followed: references, payloads, variants; Multi-Materials are not
      written as variant sets.
- [x] IFC import, IFC2x3 and IFC4, through the STEP reader's Part 21
      parser: the spatial structure (project, site, building, storeys) as
      groups and every element with a body as a part; extruded profiles
      (rectangle, circle, ellipse, hollow, I, L, polylines and indexed
      polycurves with voids), faceted B-reps, surface models, triangulated
      and polygonal face sets, mapped items. Openings are cut, and walls
      trimmed by half-spaces (bounded or not), exactly: what is taken away is
      convex or split into convex parts, and each cut is closed with its
      reveal (tests check volume and area). Styles become materials (named
      as the model names them, transparent ones glass); GlobalId, type and
      property sets become searchable metadata; units, Z up and far-off
      georeferenced coordinates are handled. Not read: swept disks,
      revolved solids, NURBS (`IfcAdvancedBrep`) - listed in a warning.
- [x] KeyShot scenes, `.ksp` and `.bip` (2026-10-30; `docs/keyshot-import.md`):
      opened as a project of their own (File > Open, a drop, the start
      screen, `tequila -i`) or imported into one (File > Import, the
      scripting `import`). Tequila's own reader of the token stream
      (`tequila-import/src/bip.rs`): every token skipped by its size when
      unknown and said in a warning, the compressed chunk inflated, fuzzed
      by truncation and corruption. The scene tree with its names,
      placements and hidden variants; an object's trimmed NURBS faces
      divided by the IGES divider at the import presets, in parallel,
      turned to look the way KeyShot's own mesh of the object looks,
      their boundaries kept as model edges; materials
      through the `.kmp` converter with labels, measured metals and
      emissive power; multi-materials as variants; HDRI-editor
      environments rebuilt from their pins; every camera with its lens,
      orthographic frame and depth of field; the image style and the
      render's size and samples. Where KeyShot's own units are not known
      (pin sizes, gradient stops, tone mapping) the reading is a best guess
      and the import's warnings say so.
- [x] Metadata panel, metadata-based selection, CSV export. The Scene tab
      searches `Field = value` (also `!=`, `:` contains, wildcards), a panel
      edits the selection's fields as one undo step, and Export CSV writes the
      tree with the fields chosen as columns.
- [x] Blender LiveLinking add-on + protocol documentation (§03 §7):
      `docs/livelink-protocol.md`, `integrations/blender/tequila_livelink`
      (send, auto sync, camera mirror). A sync is one `scene.reimport`, so it
      is one undo step and matched objects keep their Tequila materials.
      Over the scripting channel's token-guarded loopback socket rather than
      a separate pipe (the protocol document says why). Tested headless
      against Blender 5.0: send, Tequila material kept through a change,
      subdivision, delete hidden, camera.

**Exit criterion:** import a 5M-triangle STEP assembly in ≤ 15 s with hierarchy and colours intact; change the model upstream, re-import, and confirm every material assignment survives.

### M9 — GPU acceleration

**Decided 2026-10-11: the graphics card is Cycles'.** Since October 10
Tequila embeds Cycles (Apache-2.0, through the `tequila_cycles` C shim)
as its default engine: OptiX or CUDA on NVIDIA cards, Metal on Apple
silicon once the Mac build exists (`docs/MACOS_PORT.md`), HIP where Cycles
is built for it, and the processor otherwise. That is the GPU backend this
milestone set out to write, with hardware ray tracing, texture handling,
multiple devices and fifteen years of production behind it, so the items
below are answered by it rather than built a second time.

The divergence risk §01 §10.1 plans against - two shading implementations
drifting apart - is met from the other side: Tequila's materials and
texture nodes run Cycles' own algorithms, ported, and the translation maps
each onto the identical Cycles node (`docs/02` §6.6), and
`blender_nodes_shade_alike_in_both_engines` holds 35 of them to a
correlation of 0.97 between the engines (they come out at 0.999 or
better). Measured the day of the decision, the demo scene in
plastic, metal and diffuse at 512 samples, 320×200, sRGB: mean absolute
difference 0.0075, 0.0065 and 0.0079 between Tequila's renderer and
Cycles on the CPU, 3-4% of pixels more than 5% apart (silhouettes and
highlight edges). Not the §01 §10.4 per-pixel gate, which assumed one
integrator under two backends; a statistical one between two integrators.

Tequila's own renderer stays: it is the reference, it makes what Cycles
in this build does not (layers, regions, SPPM caustics), the app sends
those renders to it automatically (`batch::engine_for`), and it renders
where Cycles is not installed.

Passes and light groups come from Cycles' film (`batch::cycles_passes`),
named and laid out as Tequila's are: depth (the distance from the camera,
from Cycles' position pass), normal, position, albedo, clown, the object
and material Cryptomattes (Cycles hashes the names Tequila gives its
objects and shaders), the light groups for relighting - with what Tequila
lays over Cycles' picture, the floor's reflection and the backdrop, as one
more, `Other`, so they add up to it - and Diffuse, Lighting,
GlobalIllumination, Reflection and Refraction from Cycles' diffuse, glossy
and transmission passes, direct and indirect, by colour. Caustics,
AmbientOcclusion, Shadow and Labels stay Tequila's renderer's, and the job
says so. `cycles_keeps_passes_tequila_would` holds the depth and normal to
what Tequila's tracer finds along the same rays, and the groups to the
picture.

Where the two still part, by design (each held by a test in `app/src/batch.rs`):

- **Dispersion.** This Cycles has none, so clear glass with an Abbe number
  is three glasses, one per colour channel at that channel's index
  (`dispersion::channel_iors`): fringes in three colours where Tequila's
  renderer has a spectrum. Glass with a coat, sheen, subsurface, emission,
  labels or anisotropy disperses in Tequila's renderer only.
- **Spot falloff.** Cycles fades a spot by one smoothstep from the outer
  angle; Tequila's renderer raises it to the falloff exponent. At an exponent
  of one they are the same curve; otherwise the blend loses as much light
  across the edge, and a steeper exponent's edge is a little brighter at its
  outer end in Cycles. An edge that already spans most of the cone cannot
  widen past it, and comes out brighter.
- **Light source materials.** Both sample a glowing surface as a light,
  Tequila's renderer through a tree of the glowing triangles
  (`render-core/src/emitters.rs`), so the light a glow throws on what is near
  it agrees with firefly rejection on. Tequila's renderer still leaves to
  the bounces alone a glow a graph drives, a mixed surface's, a part some
  kind of ray passes through, and every glow in a scene with a cutaway; they
  are noisier there than in Cycles. Off, neither lets a diffuse bounce see
  it.
- **The sun** is a light with no geometry in Tequila's renderer, as every
  lamp is: a mirror does not show it, where Cycles' does.

- [~] Shading layer ported to Slang (or the chosen shared-source mechanism), compiling to both targets (§01 §10.1).
      *Superseded:* one set of algorithms (Cycles') in both engines; no
      Slang port.
- [~] wgpu compute wavefront path tracer: stage kernels, SoA path state, material sorting.
      *Superseded* by Cycles' own GPU integrator.
- [x] OptiX backend (NVIDIA hardware RT): Cycles' OptiX device.
- [x] HIP backend (AMD): Cycles' HIP device, loaded at run time, with
      Blender 5.0.1's HIP kernels, on Windows and Linux; the GPU denoiser
      takes OIDN's HIP device on an AMD card. *Done 2026-10-30, untested on
      an AMD card here* (an RTX 2060 machine). HIP RT, AMD's hardware ray
      tracing, is left off: it needs AMD's HIP RT library beside Cycles.
- [~] Texture streaming with LRU device residency; graceful CPU fallback on VRAM exhaustion.
      Cycles' texture handling; a render with no usable card falls back to
      Cycles on the processor (`batch::engine_for`).
- [~] Backend parity test suite green (§3.5): the per-node comparison runs
      in the app's tests wherever Cycles is installed, and the whole-image
      agreement is measured above; neither is a CI gate yet, which needs
      the Cycles library on the CI machine.
- [x] Multi-GPU support: the graphics-card engine renders on every card of
      the best kind there is (OptiX, else CUDA, else HIP; Metal on a Mac)
      together, as Cycles' multi-device, as Blender does with several
      enabled. `TEQUILA_CYCLES_ONE_DEVICE` keeps it to the first, for a card
      that misbehaves. *Done 2026-10-30; one card here, so the multi-device
      path is Cycles' own, unexercised by this build's tests.*

**Exit criterion:** GPU backend produces images matching CPU within tolerance on the parity suite, at ≥ 8× CPU throughput on the benchmark scene.

### M10 — Beyond parity (optional)
- [x] Spectral rendering promoted to default for dispersive materials.
      *Dispersion done, September 26:* a path meeting glass with an Abbe
      number takes one wavelength, weighted by the colour it stands for, and
      refracts by a two-term Cauchy index fitted to the IOR and Abbe number.
      That is still what an RGB render does (Spectral off).
      *Spectral, October 30* (`tequila-render-core/src/spectral.rs`): the
      Render window's **Spectral** — Auto (the default: spectral when a
      material disperses, RGB otherwise), On, Off — saved as the render
      settings' `spectral`, followed by the viewport, greyed out for Cycles.
      A spectral path carries four wavelengths from the camera (Wilkie et
      al.'s hero-wavelength sampling): one drawn from the sampler's
      wavelength dimension and three a quarter, half and three quarters
      further round the sampling distribution, which is Radziszewski et al.'s
      sech² for the eye with an eighth by AP1's blue response, which the
      eye's alone starves (a white path's blue strayed 47% RMS, now 11%; red
      13%, green 5%). Every colour the path meets is read at the four as
      Jakob & Hanika's sigmoid-polynomial spectrum: a BSDF's weight and a
      volume's transmittance as reflectances, light — the environment,
      emission, light samples — as unbounded colours at half brightness, a
      random walk's medium made per wavelength from its colour and radius
      (Chiang's remap is not linear: made per channel and upsampled, a milky
      medium was dE 7 off). D65, the space's white, is folded into the
      film's response rather than into the lights, so a white light is a
      flat spectrum and a white furnace is white; radiance goes to the film
      through the CIE functions (Wyman's fit) into AP1, normalized so a flat
      spectrum is (1, 1, 1), and keeps its sign, since AP1's responses dip
      below zero (clipped per path, a blue light was 30% too bright). A
      dispersive surface refracts by the hero's index; the path keeps all
      four off the surface and drops the other three, the hero carrying on
      for four, when it goes through, as light sampled through a rough one
      is the hero's alone. Firefly rejection judges a spectral path by the
      light it carries over its wavelengths' luminance, signed. An RGB
      render is bit-for-bit what it was: the integrator is one generic
      function, and its RGB instance does the old arithmetic in the old
      order (checked by hashing every catalog material's render, passes,
      caustics and floor included, before and after).
      *The table* is generated, not shipped: three 64³ cubes of
      coefficients fitted by Gauss-Newton on the CIELAB difference under
      D65, warm-started along brightness from a fifth of the way up, as
      `rgb2spec` does, with one change: a step that makes the fit worse is
      halved, since much of AP1 is outside the spectral locus and there the
      full steps diverged (a line of oranges fitted as blues). It is built
      the first time a render is spectral — before the render's workers
      start, on plain threads — in 0.6 s in a debug build over 16 cores;
      shipped, it would be 9 MB of floats that compress badly. Round trip,
      integrated finely (`tests/spectral.rs`): reflectances over sRGB's
      gamut, mean dE 0.044 and at most 2.3 (a yellow-white brighter than a
      reflectance can be); lights over it, mean 0.041, at most 0.83; the
      whole AP1 cube, mean 3.2, at most 93 at its green primary, which is
      not a colour. Measured: a white furnace (1.0002, 1.0001, 1.0002); five
      coloured balls within dE 0.07 of RGB's; sRGB's primaries as lights
      within dE 0.64; every catalog material in the studio within dE 0.52
      of its RGB render, the Gem aside; a dispersive ball in a white world
      (0.998, 1.007, 0.997), and with firefly rejection (0.996, 1.005,
      0.999); a small light through flint 1.08 saturated (beyond sRGB),
      0.12 through plain glass. Colour noise through a flint ball in the
      studio at 16 samples: 0.39 with four wavelengths, 0.52 with the hero
      alone, 0.85 by RGB's one wavelength at the glass. Cost on
      `simple_product`, nothing in it dispersive, spectral forced on: 1.21×
      RGB's time (4.50 against 5.45 Msamples/s at 320×180, debug build). *Open:* thin films interfere at three
      wavelengths inside the BSDF, whose result is then upsampled; a rough
      dispersive surface drops the companions rather than weighing them by
      spectral MIS; the companions reflect off a dispersive surface by the
      hero's Fresnel; caustic photons, and light sampled from inside a
      scattering medium, stay RGB and are upsampled where the path reads
      them; Cycles renders RGB.
- [x] Manifold Next Event Estimation as an unbiased alternative to SPPM
      caustics: Caustics Off / Photons / Manifold (`caustics_method`, the
      CLI's `--caustics-method`), a lamp's light through smooth glass solved
      for at every surface (`integrator/manifold.rs`, ported from Cycles'
      `mnee.h`), Cycles given its own MNEE by the same switch. Measured: a
      point light, the sun, and a point light through water in glass match
      their closed forms to four digits; an area lamp through a lens matches
      brute-force path tracing within 1.3%; behind a perfume flacon (glass,
      scent, scent, glass) it finds 45% of the light path tracing does.
      Studio test (a glass ball and the flacon under a spot and a softbox,
      640 x 420, 20 s each on the CPU): 22 samples against the photons' 52
      and 124 with caustics off; its caustics are sharper than the photons'
      but further from its own 160 s render (RMS 8.1 against 4.0 over the
      picture, 10.9 against 3.7 in the ball's caustic, 0 to 255) - a walk
      costs tens of rays, and a sample takes one lamp of the two - so at
      equal time it is not yet the less noisy. Cycles' own MNEE there: 86
      samples, RMS 4.3 in the ball's caustic, with blotches in it and
      nothing through the flacon. *Open:* one path per lamp point, so where
      several paths through glass meet (a ball's focal spot, the flacon's
      lens) the caustic is darker than the truth, as in Cycles; a sample's
      cost; no environment or glowing-part MNEE, no internal reflections,
      mirror caustics or frosted glass, no dispersion in the solved path.
- [x] Post-render relighting UI driven by light-group AOVs: "Keep light
      groups for relighting" and the Render window's Relight section,
      which rebalances and tints each group live and saves `…_relit`
      (see M6's light layer isolation).
- [ ] AI image operators (restyle/background/upscale) as post-render operations with depth/normal/Cryptomatte conditioning.
- [ ] Collaborative scene review (the open file format makes this tractable).

---

## 2. Definition of done (applies to every task)

A task is done when: the code is written and reviewed; unit tests cover the new logic; any renderer change has an analytic or golden-image test; performance budgets still pass; the feature is reachable from both the UI and the Python API if it is a scene mutation; user-facing strings are written; and the relevant spec document is updated if behavior deviates from what it describes. **The spec is a living document — if the implementation diverges for a good reason, change the spec in the same commit.**

---

## 3. Test strategy

Rendering bugs are unusually expensive because they are often invisible until compared against ground truth, and because they compound. The regime below is not optional overhead; it is what makes the project maintainable past M4.

### 3.1 Analytic correctness — white furnace tests

For each BSDF lobe, place the surface in a uniform environment of radiance 1 with no other lighting. A perfectly white, energy-conserving BSDF must return exactly 1 at every viewing angle and every roughness.

```
for lobe in all_lobes:
  for roughness in [0.0, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0]:
    for cos_theta in [0.05 … 1.0]:
      assert |furnace(lobe, roughness, cos_theta) - 1.0| < 0.005
```

This catches: missing energy compensation, wrong Smith masking, incorrect Fresnel, bad normalization. Run it for the layered stack too (coat over metal, fuzz over diffuse) — layering is where energy is most often lost.

### 3.2 χ² sampling tests

For each BSDF and each light type, verify that `sample()` and `pdf()` agree. Generate N samples, bin them over a spherical parameterization, compute the expected count per bin by integrating `pdf()` over the bin, and apply Pearson's χ² test at p = 0.01.

This is the highest-value test in the entire suite. A `pdf()` that disagrees with `sample()` produces images that are wrong in a way no visual inspection reliably catches — slightly too bright at certain roughnesses, subtly wrong under MIS — and it will otherwise be found months later. Mitsuba's test suite is the reference implementation of this technique; mirror its structure.

Run for: every BSDF lobe at 6 roughness values, the layered stack, every light type, the environment sampler, and every phase function.

### 3.3 Integrator validation

- **MIS consistency**: render a scene with lights spanning many sizes and materials spanning many roughnesses using (a) NEE only, (b) BSDF sampling only, (c) MIS. All three must converge to the same image within 1% mean error. Disagreement localizes the bug immediately.
- **Bounce convergence**: radiance must increase monotonically and converge as max bounces increases.
- **Reciprocity**: `f(wo, wi) == f(wi, wo)` for all non-transmissive lobes.
- **Zero-variance sanity**: a scene lit by a single delta light with a Lambertian surface has an analytic solution; assert against it exactly.

### 3.4 Golden-image regression

A scene per material type, per texture node, per light type, per lens type, and per major feature (labels, cutaway, displacement, SSS, caustics, motion blur). Rendered at a fixed sample count and seed; compared against a stored reference.

- Metric: **FLIP** [Andersson et al. 2020] (perceptually meaningful for renders) with a per-test threshold, plus a hard cap on maximum per-pixel difference.
- Store references as EXR (scene-linear) plus a PNG for human review.
- On failure, CI publishes a side-by-side and difference image as an artifact — reviewing a diff image is how these get triaged quickly.
- Reference images are regenerated only by an explicit, reviewed commit that states why the change is correct.

### 3.5 Backend parity

Every golden scene renders on CPU and every available GPU backend at the same seed and sample count. Assert mean absolute error < 0.002 and max per-pixel error < 0.02 in scene-linear. χ² tests run against both backends' shading code.

### 3.6 Determinism

Render the same scene twice in the same process, and once in a fresh process. All three must be **bit-identical**. Additionally render with 1, 4, and 16 threads and assert bit-identical results — this catches order-dependent accumulation, which is the usual culprit.

### 3.7 File format round-trip

For every scene in `tests/scenes/`: load → save → load, and assert the two documents are structurally equal. Additionally, load a document written by an older format version and assert the migration produces the expected document. Include a fuzzing pass over malformed archives asserting graceful errors rather than panics.

### 3.8 UI/API parity

Enumerate the command catalog at build time. Assert every command has (a) a Python binding, and (b) at least one UI entry point registered. A command reachable from only one is a CI failure. This is what prevents the scripting API from silently rotting.

*(a) is `tequila-scripting`'s `bindings` tests: the generated module and reference must match the build, and every command must have an example and a description. (b) is not yet enforced.*

### 3.9 Viewport/final consistency

Render a scene through the interactive viewport path at N samples and through the final-render path at N samples with the same seed; assert they match within tolerance (§01 §9.6).

### 3.10 Import fidelity

For each supported format, import a reference file and assert: expected node count and hierarchy, expected triangle count, correct units, correct instancing detection, and material parameter round-trip for glTF/USD (where OpenPBR round-trip should be near-exact).

---

## 4. Performance testing

### 4.1 Benchmark scenes

| Scene | Characteristics | Purpose |
|---|---|---|
| `bench/simple_product` | 200k tris, 8 materials, 1 HDRI | The interactivity baseline |
| `bench/cad_assembly` | 5M tris, 400 parts, heavy instancing | Import, BVH, memory |
| `bench/glass_jewelry` | High transmission depth, caustics, dispersion | Worst-case transport |
| `bench/interior` | Enclosed, lit through a portal | Interior sampling, portals |
| `bench/textured` | 40 × 4K textures | Texture cache |
| `bench/animation` | 240 frames, moving parts | Motion blur, temporal denoise |

### 4.2 Tracked metrics (every commit, on fixed CI hardware)

Samples/sec · time to first pixel after camera move · interactive fps · time to visually converged · scene load time · BVH build and refit time · material-change latency · peak memory. Store history and plot it; a graph of samples/sec over the project's life is the most useful single artifact for catching slow regressions.

**Gate:** a >10% regression on any tracked metric fails CI. Regressions may be accepted, but only deliberately, with a note in the commit explaining the trade.

### 4.3 Targets

Reproduced from §01 §15 — these are the numbers CI asserts:

| Metric | CPU (16-core) | GPU (mid-range RTX) |
|---|---|---|
| Time to first pixels after camera move | ≤ 50 ms | ≤ 33 ms |
| Interactive fps | ≥ 30 | ≥ 60 |
| Samples/sec (simple_product, 1080p) | ≥ 8 M | ≥ 80 M |
| Time to visually converged | ≤ 4 s | ≤ 1 s |
| Material drag-drop → updated preview | ≤ 100 ms | ≤ 50 ms |
| Load 5M-tri CAD assembly | ≤ 15 s | ≤ 15 s |
| BVH refit after transform (5M tris) | ≤ 20 ms | ≤ 20 ms |

---

## 5. Release engineering

- **Platforms:** Windows 10+, macOS 12+ (Apple Silicon and Intel), Linux (glibc 2.31+). CI builds all three per commit.
- **Packaging:** MSI/NSIS, signed and notarized `.dmg`, AppImage + Flatpak. Bundle Python and the runtime dependencies; the user should never install anything separately.
- **Versioning:** semver on the application; independent semver on the `.tequila` format; the format version is recorded in every file.
- **Nightlies** from `main`; stable releases from release branches.
- **Crash reporting:** opt-in, with the command journal attached (scrubbed of file paths and asset contents) — the journal makes crashes reproducible, which is a large advantage of the command-bus architecture.
- **`THIRD_PARTY_LICENSES.md`** generated and verified in CI.

---

## 6. Risk register

| Risk | Impact | Mitigation |
|---|---|---|
| Interactivity target missed on complex scenes | Fatal to the product's premise | Build the progressive scheduler and dirty taxonomy in M0/M1, not later; benchmark from day one; reduced-resolution interactive mode is non-optional |
| CPU/GPU shading divergence | Silent wrong images, huge debugging cost | Shared shading source from the start of M9; parity tests in CI; never allow "temporary" duplicate implementations |
| OCCT complexity and LGPL obligations | Schedule slip; licence confusion | Isolate behind one crate; defer to M8; do not fork OCCT |
| Material graph compiler scope creep | M2/M6 slip | Ship the register-machine IR first; Slang only when GPU work starts |
| Denoiser artifacts on animation | Visible flicker, unusable video | Motion-vector AOV and temporal mode planned in M5, tested on `bench/animation` |
| Scene units mishandled | Physically wrong lighting that users cannot diagnose | Both conversion modes implemented and tested in M1; unit-dependent quantities enumerated in §03 §3.2 |
| Feature breadth overwhelming quality | A wide, shallow, buggy application | Milestone exit criteria are hard gates; do not begin M(N+1) with M(N) red |
