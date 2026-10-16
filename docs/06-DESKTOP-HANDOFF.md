# Desktop continuation — 2026-09-15

## Starting point

Reviewed the supplied `tequila-spec.zip` and the current source. The latest
commit at handoff was `e6ea10e` (color ramp), following procedural textures and
the explicit decision to defer M5. The attached specification is reference
material; the owner's current focus is still rendering, UX and performance.
Claude's separate window was not accessible through the available tools.

The baseline release workspace tests passed. Historical benchmark numbers in
the roadmap have not been remeasured on this session's machine.

## First desktop pass

- Portable Windows package with a GUI executable, optional CLI, runtime DLLs,
  material assets and notices. The packaging script also creates a shortcut.
- Save on an untitled project opens Save As. Failed saves show an error.
- Native save/discard/cancel prompts protect closing and replacing a project.
  A cancelled Save As or failed save prevents closing. Closing during export
  asks before stopping the job.
- Project name and unsaved marker in the window title.
- Viewport pause/resume via toolbar and P, without resetting accumulation.
  Display edits remain available; scene edits are deferred until resume.
- Automatic viewport suspension during export and minimization. Final exports
  still run while minimized. Manual pause remains in effect after restoring.
- Idle/suspended UI polling at 200 ms; active render polling at 16 ms instead
  of an unconditional repaint loop. Input can still request an immediate frame.
- Pin the neutral dark theme once at startup rather than rebuilding the style
  and font map on every frame.

Pause happens between complete passes; it is not tile-level cancellation.
The render algorithm and output quality settings are unchanged. This reduces
unneeded background work; it does not establish a faster samples/sec result.

## Validation

- Baseline: `cargo test --workspace --release` passed before these edits.
- Updated app: `cargo test --release --offline -p tequila-app`, 79 passed.
  The added integration test verifies suspension, preserved sample counts,
  display edits during pause, and deferred request processing after resume.
- Release app and CLI built successfully; Windows package includes 59 presets.
- Packaged CLI imported the bracket fixture and rendered a denoised 320 x 200
  PNG from a working directory outside the repository.
- Packaged GUI launched, reported a responsive `Untitled — Tequila` window,
  and closed normally with exit code 0. No full interactive visual QA was
  possible with the available Windows tools; save-dialog flows still need
  hands-on verification.
- `git diff --check` passed. Workspace `cargo fmt --all -- --check` still
  reports formatting differences, including untouched source files. No
  repository-wide reformat was applied during this handoff.

Build output: `dist/Tequila-Windows-20260915-020729/Tequila.exe`, with the
matching ZIP alongside it. `Tequila.lnk` at the repository root points there.

## Next development priorities

### Subsequent UI pass

- Separated application actions from a wrapping viewport toolbar. Import,
  Save and Render remain directly accessible in the header; snapping lives
  in its own popup. Added framing and sidebar toggles to the toolbar.
- Added session workspace presets for Studio, Materials, Lighting, Camera &
  composition and Presentation. These presets do not alter the scene.
- All six inspector sections now occupy fixed positions in a two-by-three
  grid; scroll position is independent per section. Narrow windows show one
  sidebar at a time so controls cannot consume the entire viewport.
- Increased body text to 13 px and interactive rows to 28 px, and repaired
  the strong-text palette (it previously inherited dark text from the active
  widget color). The surrounding palette stays neutral for color judgment.
- Material previews default to 96 px. Category and size controls are compact;
  matching counts and assignment hints make the library easier to scan.
- Removed construction of a complete temporary LibraryTab on every frame.
  This removes repeated library disk reads and studio-environment setup.
- Thumbnail rendering now uses one background job at a time, on a dedicated
  single-thread Rayon pool, and prioritizes
  the first filtered entries. The UI uploads completed previews without
  waiting for rendering. A regression test checks that a preview retains its
  content key when the library order changes during a job.
- At compact widths the transform tools become a dropdown so the viewport
  toolbar can stay short while preserving access to Move, Rotate and Scale.
- Detailed HUD hidden initially; compact progress remains in the status bar.
  The startup demonstration is framed more closely.

Validation: 80 application release tests passed after the thumbnail changes.
The app's own screenshot facility supports visual QA through
`TEQUILA_UI_CAPTURE` (PNG output) and `TEQUILA_UI_WIDTH` (window width).
These environment variables are opt-in; normal launches do not capture or
close automatically. Interface checks use full and narrow window screenshots.
The updated package supersedes the earlier build linked above; the root
shortcut is updated automatically by the packaging script.

Final UI build: `dist/Tequila-Windows-20260915-083414/Tequila.exe` and its
matching ZIP. The 80 application tests passed with the dedicated preview
worker. Final screenshots at 1440 and 900 logical pixels verified the layout;
both capture runs exited normally. Preview tiles populate progressively to
keep their CPU use bounded. No renderer throughput speedup is claimed.

### Remaining work

1. Exercise this build interactively on real product assemblies and measure
   navigation latency, material-edit latency, memory and idle CPU use.
2. Finish useful still-product material workflows: decals, HDRI pins, graph
   editing, reusable scene setups and material variants.
3. Correct feature claims: caustics is a stored setting without its solver;
   subsurface scattering is approximated by diffuse shading; GPU is a stub.
4. Profile before selecting the GPU implementation and scheduling it against
   the still-render feature work. Keep CPU output as the correctness reference.

The original roadmap remains the source for wider product scope. This pass
does not reinstate animation, physics or video features.

Reference consulted: [KeyShot general preferences](https://manuals.keyshot.com/keyshot2024/manual/general-preferences.html)
documents pausing real-time rendering to release CPU resources. Tequila's
original UX spec already defines P as pause/resume.

### Printed labels and material editing — September 15 follow-up

The Material inspector now has a Labels section. Select a UV-mapped part,
choose **Add PNG label**, and expand **Placement & finish** to adjust UV scale,
offset, rotation, opacity and roughness. The uppermost row renders on top;
arrows reorder labels and the checkbox temporarily hides them. The existing
material linking badge and Unlink action apply to labels too.

This is a printed-color overlay workflow, not arbitrary multilayer BSDFs.
It reduces metalness and transmission under opaque ink. UV coordinates are
required; planar placement, automatic fitting and a viewport placement gizmo
remain future work. Artwork does not tile. Scale/rotation use the UV origin.
The small preview sphere shows the base material; inspect labels in the viewport.
Label PNG files remain external project assets: keep them with the project.
Saving the base material to the material library does not include labels.

Implementation: ordered graph decals, alpha-preserving PNG decode,
premultiplied-alpha MIP filtering, scene texture requirements, command undo/redo,
project JSON and journal round trips. Existing files without labels still open.
Do not resave labeled projects in older Tequila builds, which lack label support.

Material preview rendering now runs on a dedicated background CPU worker and
discards stale results during slider edits. This removes synchronous thumbnail
work from the UI frame; it is not a measured renderer throughput improvement.
Capture diagnostics optionally accept TEQUILA_UI_MATERIAL to show the inspector.

### Transform and grid correction

Gizmo starts now use world transforms and results are converted back through
parent transforms before issuing commands. This fixes world/local axis drift
inside rotated and nonuniformly scaled assemblies. Picking uses the mouse-down
position rather than the later position at which dragging crosses its threshold.
Plane scale handles now scale their two axes, preserving the plane normal;
plane snapping also preserves its normal coordinate. Axis labels are visible.

The optional grid is an XZ measurement overlay at Y=0, not renderable geometry.
It remains off by default. Lines crossing the camera plane are clipped instead
of dropped, and the spacing/plane caption stays in the viewport corner. Grid
visibility and transform snapping remain separate, explained in their tooltips.

### Solid startup and viewport performance

Solid (formerly Fast) is now the startup default. Rendered mode and final
image export retain the path tracer. Solid uses a pinhole camera regardless
of render depth of field, presents one sample immediately, refines to four
samples instead of 32, and rests. Dragging uses one sample at a bounded
300,000-pixel working resolution, returning to full resolution on release.
Superseded solid frames are discarded. Framebuffer storage is reused across
camera/transform requests of the same size.

Camera-only commands no longer rebuild the scene snapshot; the renderer
recognizes the unchanged snapshot and retains its acceleration structure.
This removes per-orbit assembly traversal and scene rebuild work. Regression
checks cover default mode, working-resolution bounds, buffer reuse, camera
snapshot reuse, and depth-of-field independence. This is still CPU preview
shading, not Blender's GPU raster viewport; no Blender-equivalent FPS is claimed.
The grid was also reduced in density and contrast after native screenshot review.

Validation artifacts for this pass: target/decal-final-tests.log (labels,
formats, texture filtering and rendering checks), target/transform-tests.log
(parent transforms, plane scale and undo catalog), target/plane-tests.log
(plane snapping), target/label-gc-test.log (label assets survive save-time
collection), and target/solid-tests.log (Solid mode and renderer regressions).
The 320x240 demo benchmark measured a 6.04 ms Solid pass versus 31.33 ms path
tracing during a build; this is a small-scene comparison, not app FPS or a
before/after throughput claim. Primary improvements are less refinement work,
reused buffers and avoiding scene work for camera edits.

Current distribution: dist/Tequila-Windows-20260915-212523/Tequila.exe.
Root Tequila.lnk points at this build. Executable hashes were matched against
the release binary. Native captures: target/labels-ui.png and
 target/solid-sample-ui.png. Latest app checks: target/solid-final-app-tests.log.

### Label placement without UVs — September 18 follow-up

The September 15 entry above records "UV coordinates are required; planar
placement, automatic fitting and a viewport placement gizmo remain future
work." Planar placement and automatic fitting are now done; the gizmo is not.

A label added to a part with no UV set is projected onto it instead: flat down
the part's shortest axis, so it lands on the widest face, sized to 45% of that
face and centred on it, in scene units rather than repeats. The Placement row
offers **On UV map**, **Project flat** and **Wrap around**; UV is disabled with
an explanation when the part has no layout to follow, rather than offered and
silently doing nothing. `Fit to part` re-runs the fit; `Reset placement` returns
to it rather than to a one-unit square. Existing UV-mapped parts are untouched —
the fit declines when the mesh has UVs, so an authored layout is never
overridden unasked.

This mattered because STL carries no UV coordinates and most exported OBJ
meshes carry none either, which is the whole of the currently supported import
list bar glTF. `crates/tequila-render-cpu/tests/projected_label.rs` renders a
UV-less quad with and without a fitted label; under the previous UV-only
placement the label contributes nothing to the image at all, which is what the
test reports when the fit is removed.

Two supporting corrections found on the way, both pre-existing:

- The renderer shaded in world space while `docs/02` §6, and the code comment
  beside it, said object space. Procedurals and projections slid through a part
  as the part moved. `SceneSnapshot::shading_point` now converts, footprint
  included. Covered by `tests/object_space.rs`.
- `MaterialGraph::content_hash` did not cover a mapping's scale, offset or
  rotation, nor a procedural's parameters, and `snapshot::compile_for` reuses
  the cached program whenever that hash matches. Editing a texture's scale
  moved the control and left the picture unchanged.

`Mapping` gained `axis` (which axis a planar projection looks down and a
cylindrical one wraps about) and `pivot` (what a rotation turns about). Both
default to the previous behaviour, and both file readers fall back per
projection, so older documents load unchanged.

Remaining on labels: the viewport placement gizmo, artwork tiling, and
non-square fitting to the artwork's own aspect ratio.

### Viewport label placement — September 18

The last of the three items the September 15 entry listed as future work. A
label's row in the Labels panel has a **⊹ Place** button; with it held down,
clicking the part in the viewport puts the artwork's centre under the cursor
and dragging slides it around. Escape or the button again ends the mode, which
says so in a banner across the bottom of the viewport the whole time it is on.

While placing, the artwork's border is drawn on the part as a frame — the
projection run backwards — so its size and rotation are visible without
rendering. The frame is drawn on the face turned towards the camera; drawn
through the middle of the part it is half buried in the geometry and reads as
unrelated to the artwork on the surface.

Clicking a flat label re-chooses its face from the surface normal, because
clicking the side of a box means the side of the box. A wrapped label keeps its
axis, which belongs to the part rather than to where the user clicked. Clicking
a different part, or the backdrop, does nothing: that point is in another object
space and means nothing in this one. A UV label has no frame to move, so the
button is disabled with an explanation rather than present and inert.

The button is offered per label and the mode is per label, so the material
travels with the index — label 0 of the part being placed and label 0 of
whichever material the panel is showing are different labels.

Placement is one undo step per drag, as a slider is. Placing a label where it
already sits produces no command, so holding the pointer still does not fill
the undo history.

`placement_command` is the testable part: which part was hit, and getting the
point from world space into the part's own. Its tests use a part that is both
moved and rotated, because at the origin with no rotation the two spaces are
the same and every conversion bug is invisible.

Known limits: the projection is axis-aligned, so a label on a curved surface
takes the nearest principal axis rather than the exact surface normal; a
cylindrical label straddling the seam has a continuous frame but discontinuous
coordinates; artwork still does not tile, and the fit is square rather than the
artwork's own aspect ratio.

`TEQUILA_UI_LABEL=<png>` joins the existing capture variables: with
`TEQUILA_UI_CAPTURE` and `TEQUILA_UI_MATERIAL` it adds a label to the
selected part, switches it to projected placement and arms the mode, so a
screenshot run can show the frame. A label otherwise arrives only through a
file dialog, which a capture run cannot open.

### Feature claims corrected — September 19

Remaining-work item 3 above: "Correct feature claims: caustics is a stored
setting without its solver; subsurface scattering is approximated by diffuse
shading; GPU is a stub." Audited and corrected, and the corrections are tests
rather than prose so they cannot rot.

**Eleven parameters are exposed in the inspector and read by no code that
shades.** Found by two independent methods that agree: a static search for
field reads in the lobes and the renderer, and a behavioural test that moves
each parameter and compares a fingerprint of the BSDF's evaluation, sampling
and emission. The behavioural test is the authority — it correctly cleared
emission, which the static search had wrongly flagged because `emission()` is
a method.

The eleven: transmission depth, transmission scatter and its anisotropy, Abbe
dispersion, subsurface radius and its anisotropy, thin-film weight, thickness
and IOR, anisotropy rotation, and cutout opacity. They span sixteen material
types; `Translucent` offered Radius as one of its five headline controls.

`Param::unimplemented()` names each with the reason, the Material tab labels
them "· not yet" in italics with the reason on hover, and three tests keep the
list honest in both directions: an exposed parameter must shade or be listed,
a listed parameter must not shade (or the inspector is greying out something
that works), and the separate list of parameters that are implemented but
masked by their own type's defaults must stay accurate.

That last list is worth reading on its own: Glass and Gem expose Color, which
does nothing because transmission is 1; Translucent exposes Color, replaced
entirely by its subsurface colour; RealCloth exposes Anisotropy, which the fuzz
lobe drowns. Those are consequences of the shading model rather than missing
features, so they are documented rather than marked.

**Caustics** is a stored setting whose solver does not exist. The flag is kept,
because it is written to project files and a scene that asked for caustics
should still say so when §6.5's photon pass lands, but the Jewelry preset no
longer promises caustics — it promises deep transmission chains and says
caustics resolve slowly. `tests/unimplemented_settings.rs` renders a glass
sphere over a diffuse floor twice and requires the flag to change nothing; when
the pass lands, it fails and names what to correct.

**Subsurface scattering**: the engine's own docs were already honest ("the
right colour, the wrong transport"). What was not honest was the inspector,
which is what the parameter marking above fixes.

**GPU**: no GPU code exists in the workspace at all — not a stub, not a
feature flag, nothing. There is no claim in the application to correct. The
roadmap still lists it.

### Lighting pins — September 19

`docs/01` §1.5's HDRI Editor, the part of it that matters for product shots.
The September 15 note listed "HDRI pins" among the unfinished still-product
workflows and the roadmap recorded the editor as not started.

Why this rather than an HDRI library: no environment maps ship and none can.
§0.2 forbids bundling KeyShot's and CC0 replacements are a download, so every
scene is lit by the built-in procedural studio or by a file the user found.
`docs/02` §420 gives the way out — studio setups that ship as "just pin
definitions in a small file". Three presets do exactly that.

**A pin is paint on the environment, not a new light.** `docs/01` §295 fixes
this: pins "composite into the environment as an ordered layer stack evaluated
into the same equirectangular texture, after which the CDF is rebuilt". So a
pin is importance-sampled by machinery that already exists, casts the soft
shadow an area light casts, and appears in reflections because it is genuinely
in the image the surface reflects. It is an angular rectangle or ellipse with a
soft edge, rolled about its own axis, and its coverage is exactly zero outside
its angular size — a falloff with a tail raises the whole sky and gives the
flat grey wash that makes a studio render look unlit.

**Aiming by clicking the model** is the editor's defining gesture and is
implemented. Nobody thinks "the key belongs at bearing 43, elevation 31"; they
point at where the highlight should be. A specular highlight is the light seen
mirrored in the surface, so the pin goes to the view direction reflected about
the surface normal — taken back through the environment's own rotation, since
pins live in the environment's space and it turns under the scene. Forgetting
that puts the highlight off by exactly the rotation, which looks like the click
missing rather than like a missing transform.

The panel names what a photographer moves: bearing and height rather than a
direction vector, size in degrees because how big the box looks from the
product is what sets the shadow's softness, and a diffuser control.

Tests worth knowing about. The eight unit tests of the compositor read the
image back through the same mapping they wrote it with, so they agree with
themselves whatever the convention is; flipping Z leaves all eight passing and
fails only the ninth, which goes through `Environment` and reports the bare sky
where the pin should be. The ninth also requires over half of environment
samples to land on the pin, because a pin the CDF cannot find renders as noise.
`EnvironmentCache`'s key includes the pins, and dropping them makes the
environment read identically with and without a light.

Verified in the app: the three-point preset takes the demo sphere from flat and
muddy to lit, with separate key and fill highlights. Solid mode shows no change
and should not — it shades from a fixed rig and never reads the environment.

**Lights that do not show.** A pin painted onto the environment lights the
scene and appears behind it, which for a product on a clean sweep is not
wanted. The eye beside each pin takes it out of the picture; it goes on
lighting the scene and appearing in reflections. Off is the default, because
that is the better-looking render.

Switching the background to a solid colour is *not* the same thing, and the
difference is the reason the flag exists rather than a note telling people to
do that. §1.5 replaces the view along the whole specular chain, so a flat
backdrop takes the softbox reflections off chrome along with it — a chrome
product ends up reflecting a dead grey field. A hidden pin leaves only the
primary ray's view. Three render tests hold the three claims apart: a solid
background leaves the light on the product alone, a mirror reflects the
backdrop rather than the pin, and a hidden pin still lights and still
reflects.

That asymmetry is deliberate and worth remembering when touching
`integrator.rs`. A backplate *should* appear in chrome, or the product looks
pasted onto the photograph. A softbox should not appear behind the product but
should appear in it. They are different rules and the escape branch now carries
both.

Remaining on §1.5: sun and sky (Hosek-Wilkie), and the 360° canvas with
draggable pins. The per-pin controls and aiming by clicking the model cover the
workflow without it.

### Sun and sky — September 20

*Superseded September 26:* the sky is now Hosek-Wilkie's own model, ported
from the authors' reference code and tested against it; see `docs/05`'s
HDRI Editor item. The note below describes the single-scattering sky it
replaced.

`docs/05` asks for sun & sky by Hosek-Wilkie. This is **not** that, and the
difference is deliberate enough to record.

Hosek-Wilkie is an analytic fit: a Perez-style formula whose nine coefficients
per channel come out of a large table interpolated over turbidity, albedo and
solar elevation. The table *is* the model. Writing one out from memory would
give a sky that is subtly wrong in a way nothing in this repository could
detect — the output is a smooth gradient either way and there is no reference
render to compare against. That is precisely the sort of unverifiable claim the
September 19 audit was about.

So `render-core/src/sky.rs` is single-scattering atmospherics: Rayleigh and Mie
integrated along the view ray through an exponential atmosphere. Few constants,
all physical — the scattering coefficients follow 1/λ⁴, the scale heights are
the atmosphere's — and the sky's behaviour falls out rather than being
tabulated. Which is what makes it testable. A blue zenith, a paler horizon, a
sun that reddens as it sets, a forward-scattered glow around it, haze that
desaturates: each is a consequence, each is a test, and a mistake in the
physics fails one of them.

What it is not: multiply-scattered. Real skylight bounces, and single
scattering under-lights deep twilight and heavy haze. That is the main thing
Hosek-Wilkie buys, and `docs/05`'s item stays open.

Two findings worth keeping:

- **The sun's disc needs its solid angle.** Radiance is power per unit solid
  angle, and the first version divided by nothing — giving a "sun" a few times
  brighter than its own glare rather than thousands. In the image the disc is
  spread over exactly the texel that contains it, divided by *that* texel's
  solid angle, so its power does not change with resolution. A test renders at
  128 and 512 wide and compares; dividing by the sun's solid angle instead
  makes the flux scale with the texel, which is a scene that gets brighter
  when you change a setting nobody associates with brightness.

- **The horizon has to be continuous.** Ground and sky are different
  calculations meeting at elevation zero, and switching between them there drew
  a hard line across the render. Visible in the first golden-hour screenshot;
  invisible to every test until one walked down through the horizon. The ground
  now *starts* from the horizon sky and darkens towards albedo-times-skylight
  as the view turns down, which is both what aerial perspective does and
  continuous by construction.

**Speed, which no test could have told me about.** The first version took
1.6 seconds to build a 512x256 sky, and the environment cache rebuilds on
every slider frame, so the controls would have been unusable. Three fixes
took it to 55 ms: the optical depth towards the sun is read from a table
instead of marched per sample; the table is built once per process rather
than once per call, because it depends only on the shape of the atmosphere
and nothing a sky carries; and the image's rows run in parallel. The
per-call build also made `radiance` cost half a million operations to answer
one direction — a sweep test took 95 seconds; it now takes 0.08.
`examples/sky_bench.rs` is how to check this after changing the model.

**The horizon was under-sampled**, and the test that found it is the one
worth copying. Twenty-four evenly spaced steps are fine looking up, where a
ray crosses 60 km of atmosphere, and wrong along the horizon, where it
crosses 600 — the first step was 25 km long and stepped over the dense air
where nearly all the scattering happens. The sky quadrupled in brightness
over the first two degrees. Quadratic step spacing fixed it.

The test could not be "nothing changes fast", because near a low sun the sky
genuinely changes by a tenth of itself per degree from extinction. It
measures the worst step at one degree and at a quarter of one, and requires
the finer sweep to be much smoother: a gradient's biggest jump shrinks with
the sampling, a discontinuity does not move. Two wrong diagnoses came before
that — I blamed the Mie aureole, then the table's resolution — and printing
the actual profile settled it in one run.

**The cache is keyed in two layers**, and it has to be. The base image — a
sky, a loaded file or the built-in studio, with contrast applied — is keyed on
everything *except* the pins; the environment is keyed on all of it. Keyed
together, dragging a pin's brightness regenerated the sky twice a frame, once
for the lighting image and once for the camera's. Split, a pin drag is 18 ms
and a sky drag 79 ms, against 87 ms cold and 0.3 ms for no change at all.
`examples/environment_bench.rs` in `tequila-scene` prints those four.

The regression test asserts this by *identity* rather than by a stopwatch: the
base image is shared, so a rebuild would give a different `Arc`. A timing test
would say the same thing less reliably and fail on a loaded machine.

The panel offers the two controls an outdoor photographer has — where the sun
is, and how much haze — plus four named times of day. Sun position uses the
same bearing-and-height controls as the light pins. A sky replaces the
environment image; pins composite on top of it exactly as they do over a studio
or an HDRI.

### The headless renderer was ignoring the project — September 20

Found by rendering a real model end to end rather than by a test failing.
`assets/samples/glasses.gltf` through the CLI produces a clean product shot in
about three seconds, which is the good news. The bad news was two gaps either
side of it.

**The CLI built `studio_environment()` unconditionally.** A project set up in
the application with a sky, a loaded HDRI or a stack of light pins rendered
from the command line came out lit by the built-in studio instead. Nothing
reported it — the command succeeded and the file appeared, lit the wrong way.
It now goes through `EnvironmentCache`, which is the thing that knows how to
turn a document's environment into one.

**The new lighting was reachable only from the interface**, which is this
project's oldest recurring defect wearing the other hat. `--pins` and `--sky`
now select the studio setups and times of day by name, and an unknown name is
an error rather than a silent fallback to the default.

The presets moved out of `app/src/pin_panel.rs` and `app/src/sky_panel.rs` into
`tequila-render-core`'s `pins` and `sky` modules, where both the panel and the
renderer can read them. A studio setup is lighting, not interface. The panels
now call `studio_presets()` and `time_of_day()` rather than holding their own
copies, so there is nothing to drift.

`crates/tequila-cli/tests/headless_lighting.rs` runs the binary — the defect
was in the glue between the document and the renderer, which is the one place
a unit test of either side cannot reach. Restoring the hardcoded environment
makes both tests report byte-identical images.

**The sky was three stops too bright**, found by measuring those renders
rather than looking at them. A default sky was ten times the built-in
studio's mean radiance and midday thirty-six times, so a golden-hour render
clipped 30% of its pixels to white. `sun_intensity` was set by eye — "about
1.0 at the zenith" — and is now scaled so a default sky opens at about the
studio's exposure. Nothing clips now.

The spread between times of day is deliberately kept: midday is 3.6x the
studio and overcast 0.4x, because midday really is brighter. Flattening that
so every preset exposed identically would be throwing away the physics to
save a slider. `examples/sky_levels.rs` prints the ratios and a test holds
them inside bounds wide enough for real daylight and narrow enough to catch
an order of magnitude.

Worth recording that the same measurement corrected me: I had written that
light pins blew out the lenses. They clip nothing — the pins render is 1.5%
brighter than no pins, because pins are hidden from the background by
default and the product is a small part of the frame.

### glTF textures were never read — September 20

A real model settled it. `scene.gltf` from a Sketchfab download — a Meta Quest
3, one mesh, 76k triangles, with base colour, metallic-roughness and normal
maps — imported as flat white. The glTF reader parsed UV coordinates and never
looked at `images`, `textures` or any of the texture references in a material.
Neither did the OBJ reader look at `map_Kd`. So every textured model, which is
what a downloaded asset almost always is, arrived untextured and nothing said
so.

Base colour and normal maps now come in. The pieces:

- `ImportPayload` carries `textures`, and the Import command interns them
  before the materials whose graphs refer to them. Undo takes them back out.
- The reader records URIs, not paths. Reading a model is defined to touch no
  filesystem, so resolving a URI against the directory the file came from
  happens in `import`, which is the only part that knows it. Percent-encoding
  is undone, so a map named with a space resolves.
- Ids are reserved like every other key, and `MaterialGraph::remap_textures`
  renumbers the reader's local indices onto them. The reader cannot see the
  document, which is what keeps an import undoable and scriptable.
- A named image that is not on disk is a warning, not a failure: the geometry
  is the part that cannot be replaced, and §5.1's placeholder rule leaves the
  material its own colour.

**Metallic-roughness now reads too**, through a new `NodeKind::Channel`. glTF
packs roughness in green and metalness in blue, so the map is attached twice
and each parameter takes its own channel; fed whole to either it would arrive
as the luminance of all three, which is neither. The map is imported as
`MapRole::Data`, not colour — these are numbers the shading model reads, and
decoding them through an sRGB curve would bend every roughness in the model.

glTF *multiplies* the map by the factor rather than replacing it, and both
factors default to 1 when absent, so a `Constant` and a `Multiply` are inserted
when the factor is not 1 and skipped when it is. The factor is also the
texture's fallback, so a map that fails to load leaves the material exactly as
the file described it.

Worth knowing what this changed. Before it, a textured glTF with no factors
written — which is the common case — imported as **fully metallic and fully
rough**, because that is what glTF's defaults mean with no map to modulate
them. Every such model rendered as dull grey metal. The Quest 3's own map says
metalness 0.05 and roughness 0.42: white plastic. The render got brighter and
glossier when this landed, and that is the correct answer rather than a
regression — I misread it as one before measuring the map.

### The material catalog, audited — September 20

`crates/tequila-render-cpu/tests/every_material_renders.rs` renders all
thirty-four presets and checks each is not black, not featureless, and not
identical to another entry. Three findings, all legitimate, all now recorded
with reasons in the test's own exemption lists:

- **Cutaway renders black.** It zeroes base weight, specular weight *and*
  opacity, and opacity is one of the eleven unimplemented parameters — so the
  half that makes it vanish does nothing while the half that darkens it works.
  A black hole rather than a window. The September 19 audit listed opacity as
  unimplemented without noticing this followed from it.
- **Advanced and Generic** render identically, which is what "nothing hidden"
  means; they differ in which controls the panel shows.
- **The four light presets** share one set of defaults, because a spotlight
  differs from a point light in the light node, not in the material.

The lists check themselves in both directions: an entry excused for being
black that stops being black fails the test, because a stale excuse hides the
next regression.


### Where the notes above are out of date — September 26

The sections above are a log, and several of their "not yet" lines have
since been done. As of this date:

- **Labels** no longer need UVs: planar, cylindrical and box projection, a
  placement gizmo (click or drag; Ctrl + scroll sizes, Shift + scroll turns),
  a Foil finish, print size read from PNG and JPEG artwork, and a Labels pass
  in EXR and PSD output.
- **The Material tab's sphere** shows labels, patterns and the project's own
  textures, not only the base material. **Save to library** keeps patterns;
  pictures and labels stay with their project.
- **The library** is procedural where it should be: nineteen presets use
  patterns (weave, leather, wood, marble, concrete, cast and textured
  finishes). Thumbnails are shot at true size on a 5 cm ball, denoised, in a
  studio of their own, and kept on disk: the shelf fills at once from the
  second launch of a build (`%LOCALAPPDATA%\Tequila\thumbnails`).
- **Glass and liquids**: a liquid modelled into its glass keeps its colour
  (a priority medium stack), solid glass no longer tints twice, and an
  index-matched interface passes light.
- **Renders are opaque** unless alpha is asked for; render layers keep alpha.
- **The command line renders a project as the application does**: camera,
  lighting, look, resolution, samples, denoising and alpha, with flags
  overriding. (September 20 fixed only its environment.)
- **The Cutaway material** no longer renders black: opacity is implemented,
  and cutaways are a CSG walk in the intersector.
- **The application opens on a 10 cm ball**, not a 2 m one, so real-world
  materials look right on it.
- **Launch with "Tequila (newest)"**: it always opens the latest package.

- **Subsurface scattering is a real random walk** (September 26): light
  enters the part, scatters inside it and leaves elsewhere, so thin parts
  glow when lit from behind and edges soften. Subsurface Radius and
  Anisotropy now work. The quick previews show the subsurface colour.

- **Clear volumes scatter** (Scatter and Scatter Anisotropy): a milky
  liquid or a smoky glass is a walk inside a refracting surface, the same
  transport as subsurface.

- **Dispersion** (the Abbe number): diamonds and crystal glass split light
  into colour. Gem defaults to a diamond's Abbe number of 55; clear glass
  types take it too.

  Firefly rejection no longer tints a dispersive stone: it judges each
  path by the light it carries rather than by its colour's luminance, which
  had clipped green wavelengths hardest and turned diamonds lavender.

- **Anisotropy has a direction** (Rotation): a brushed finish runs along the
  part's own tangents, or its UVs where it has none, and turns with the
  part; Rotation turns it about the surface, 0 to 1 over a half turn. glTF's
  `KHR_materials_anisotropy` rotation is read.

Every material parameter is now implemented; nothing is greyed out.

- **Volumes from OpenVDB** (the Volume Map node): on a Scattering or
  Translucent Medium, the new Density control's "tex" menu takes a `.vdb`
  file — smoke, cloud, a swirl in a liquid — fitted to the part's bounding
  box. Houdini's half-float files included. Density on its own is a
  thickness control: 1 as described, 0 clear, 2 twice as thick. Smoke and
  cloud are lit by the sun and lights at every scattering event, so a
  sunlit cloud renders bright and shadowed rather than dark and speckled.

- **Colours are what they look like.** The colour pickers showed the
  rendering-space (ACEScg) numbers as if they were sRGB, so a picked red
  rendered brighter and redder than its swatch; they now convert both ways.
  glTF colour factors and OBJ `Kd` are converted on import the same way, as
  their textures already were. The original presets, whose colours had been
  written as sRGB and read as ACEScg, were converted (at the user's say-so)
  so they render as the colours they were written as; saturated ones are a
  little less intense than before.

- **The library has 101 presets** (from 59): a Gem shelf (diamond, cubic
  zirconia, ruby, sapphire, emerald, amethyst, citrine, peridot, topaz,
  garnet — each with its real index and Abbe number, so they have fire), a
  Liquid shelf (whole and skimmed milk from Jensen et al.'s measured
  coefficients, orange juice, coffee, red wine, lager, olive oil, honey,
  cola), a Translucent shelf (candle wax, beeswax, jade, soap, porcelain,
  gummy), more metals (silver, platinum, rose gold, nickel, black chrome,
  gunmetal, bead-blasted aluminium, polished brass), coloured and
  soft-touch plastics, a red lens and a blue acrylic, and LED and screen
  emitters. The Liquid type now shows Scatter, for the cloudy ones, and
  the Solid view and quick previews show a cloudy volume as cloudy — milk
  was previewing as clear water.

- **STEP import** (`.step`, `.stp`), in pure Rust through truck and
  ruststep rather than the OpenCASCADE the roadmap planned. Each solid body
  becomes a part under its own name, grouped under the product; a body's
  colour becomes a plastic material, and a face coloured differently from
  its body becomes a part of its own. The file's length unit is honoured.
  One vendored fix: a placement that leaves out its reference direction
  (which the standard allows) turned every circle along X into NaN, and a
  real wrench lost every face this way; see
  `vendor/truck-stepio/TEQUILA-PATCH.md`. A face truck cannot mesh is
  dropped with a warning rather than failing the file. Assemblies keep
  their structure: each component where the assembly places it, through
  sub-assemblies, and a part used several times meshed once and shared.
  *Not yet:* placements written as mapped items (older AP203 files; the
  import says how many), IGES, layers, metadata, B-rep retention.
  *Since 2026-10-13* STEP reads through OpenCASCADE 8.0.1 where its helper
  is installed (`native/occt/build-occt.bat`, packaged into `occt/`): every
  face of files truck mangles, with the same parts, names and colours, and
  truck as the fallback. The status line shows OCCT's reading percentage,
  then faces divided, with the running time.

- **Imports are at their real size.** A unit the file states — STEP's,
  glTF's metres, or one chosen in the import options — now converts the
  geometry into the document's unit. Before, the unit was reported and
  nothing used it: a part exported in millimetres came in a thousand times
  too large for a renderer whose presets, focal lengths and bump depths
  are in metres. A guessed unit (OBJ, STL) is still not applied.

- **STEP imports are faster and no longer freeze the window.** The file is
  parsed by a small hand-written Part 21 reader (`tequila-import/src/part21.rs`)
  that builds the same syntax tree ruststep did, at about 100 MB/s rather
  than 7, and also reads the apostrophes (`''`) and empty lists ruststep
  rejected. Bodies are meshed in parallel, and an import from the menu is
  read on a worker thread and applied when it lands; the status line says
  "Importing …" meanwhile. Scripted imports stay synchronous, so a script
  gets its result in the reply.

- **Real-world sizes hold on scaled imports.** A planar, cylindrical,
  spherical or tri-planar mapping with real-world units measured in the
  mesh's own units, so on a glTF from a centimetre tool (a centimetre mesh
  under a 1/100 node) every real-world size came out a hundred times too
  small: a 1 mm strap weave averaged to flat grey. Mappings now measure in
  scene units, as the unmapped pattern path already did. Found recreating a
  Meta Quest 3 product shot (`renders/Quest3 recreation/`), which also
  showed the working studio recipe for a white-page product shot with glass
  in it: background mode Environment plus a white gradient pin that shows
  only in the background (so reflections keep seeing the studio), rather
  than a solid background colour, which glass and mirrors reflect.

- **PBR texture packs** (File → Import texture pack, the top bar's
  Texture pack…, the Library panel's + Import texture pack…, the Material
  tab's Import texture pack…, a folder or ZIP dropped on the window, or the
  scripting op `texture_set`; all go through `app/src/texture_pack.rs`).
  From the menus and a drop the material goes on the selected parts; from
  the Material tab, on every part using the material shown; with nothing
  selected it is only added, and the Material tab's name box lists every
  material in the project to pick it from. A folder or ZIP from ambientCG, Poly Haven, Megascans,
  CGBookcase, textures.com or a Substance export becomes a material: the
  role of each file is read from the last role word in its name (so
  `metal_plate_diff_2k` is a colour map), the best resolution and format of
  each map is chosen, the OpenGL normal map is preferred and a DirectX-only
  one is converted beside it with its green channel flipped, gloss is
  inverted into roughness, packed ARM/ORM maps are split into their
  channels, and a height map becomes a bump where there is no normal map.
  All maps share one mapping. On parts with UVs made for texturing (the
  starter ball, glTF, OBJ) it follows them, repeated to keep the texture in
  proportion (twice round a sphere, once pole to pole — how Poly Haven and
  ambientCG render their previews); on STEP bodies and parts without UVs it
  is tri-planar, one repeat as wide as the parts. Texture size / Texture
  repeats sit near the top of the Material tab. September 27 fixes, found
  comparing against the packs' own previews: normal maps under a projection
  were decoded in the mesh's UV frame (every bump leaned the wrong way on
  tri-planar parts); colour textures and HDRIs were used in sRGB primaries
  as if ACEScg (oversaturated: beige stains came out orange); greyscale EXRs
  (Poly Haven's roughness) failed to load, leaving the default roughness;
  and a fixed 0.5 m repeat magnified packs five times on the 10 cm ball. A ZIP
  unpacks into a folder beside itself, since the project stores paths.
  Save to library keeps the maps: they are copied beside the entry
  (`library.rs` `from_graph_with_pictures`, `save_entry`), and applying the
  entry renumbers them into the project (`for_document`), so a downloaded
  material is reusable everywhere. *Not yet:* reading a pack's real-world size
  from its metadata.

- **Caustics are real** (Lighting → Transport → Caustics, on in the Jewelry
  preset; `--caustics` on the command line). Light through solid glass,
  liquids and gems, and off mirrors, is traced from the lights as photons
  each sample and lands as focused patterns on the floor and on parts, the
  shadow catcher included. The older notes below saying caustics is a
  stored setting without a solver are out of date. Thin-walled Glass passes
  light straight through, so it makes none; use Glass Solid.

- **Updating a model from its file** (right-click a part in the Scene tree:
  Reload from file, or Update from another file…; File → Reload model from
  file; scripting `reimport`). Parts keep their materials, labels, names,
  visibility and settings and take the new geometry; new parts are added,
  parts no longer in the file are hidden and listed in a Model updated
  window that offers to delete them. Models imported before this build
  record no file, so Reload is greyed out on them; Update from another file
  still works, matching by the tree as it stands. Journalled imports now
  keep their textures (they were dropped on replay).

- **Turning a model a quarter turn** (right-click it in the Scene tree:
  Turn). Upright stands a CAD model drawn Z up, which arrives lying on its
  back since Tequila is Y up and STEP does not say which way up it was
  drawn (Onshape, NX, CATIA and Fusion set to Z up write Z up; SolidWorks
  and Fusion set to Y up write Y up, so it is not done on import). The other
  three turn it 90° around the vertical, forward, or onto its side. Each
  turns about the model's middle and keeps its lowest point where it was,
  on the floor if it was on the floor, in one undo step. A STEP or IGES
  import's status line points at it.

- **A whole-app review (2026-10-14)**, each finding checked in the code
  before it was fixed. What a person would notice:
  - *Big assemblies stay responsive.* Environment, lighting, cutaway and
    grouping edits no longer reconvert and re-upload every mesh (a slider
    drag froze a 700,000-triangle model); the Solid view looks shader
    uniforms up once rather than ~20 times per part per frame; the Scene
    tree lays out only the rows in view; the Split menu item counts pieces
    once, not every frame the menu is open.
  - *Renders.* The denoiser's guide prefilter had been reading normals as
    a colour picture, clamping every negative component, which softened
    edges in every denoised render; it now cleans them as normals. Cycles
    is given the Clamp, firefly-rejection and prefilter settings (Strength,
    which its denoiser lacks, is greyed out there). A GPU render whose
    file could not be written is no longer re-rendered on the processor.
    The floor-reflection pass after a Cycles render stops on Cancel or the
    time limit. EXR files with passes carry the finished (denoised, cut
    out, alpha-mode) image as their beauty. Depth of field turned off in
    the live Cycles view turns off. Sidecar .txt/.xmp files of an earlier
    render are not overwritten by a later one of the same name.
  - *Imports.* Curved STL files are smoothed (facet normals only orient
    triangles now); keycaps sharing a shape but not a legend keep their
    own UVs; glTF 32-bit indices are exact; OBJ material files with spaces
    in their names or in subfolders are found, and maps named by another
    machine's absolute path are found beside the model; turned models sit
    on the floor by their lowest point; a part that becomes several pieces
    on Update keeps its material on them.
  - *Safety.* Recover no longer replays onto the wrong document or deletes
    the crash files when it cannot open them; a second window no longer
    deletes the first one's crash journal; saves are flushed to disk
    before replacing the old file and refuse (rather than corrupt) a
    project past what a ZIP can hold; updating a library preset keeps the
    pictures projects still use.
  - *Smaller.* Undoing a group delete keeps its place; removing a camera
    above the active one keeps the same camera active; H no longer toggles
    the HUD while typing; the assistant's Undo request can no longer undo
    your own edits; a busy plugin can no longer freeze the window.
  - *Not done, on purpose:* auto-save still writes on the window's thread
    (moving it needs the journal cut at the copy); snapshot rebuilds per
    gizmo-drag frame and per-frame selection bookkeeping on 10,000-part
    selections; vertex snap is still a linear scan.
