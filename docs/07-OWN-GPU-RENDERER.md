# 07 — Tequila's renderer on the graphics card, and the default engine

The plan for making Tequila's own renderer the default renderer: what has to be true before it replaces Cycles as the default, the decisions already taken, and the work in order, each step with its test.

**Cycles stays** (owner, 2026-11-17). It remains in Tequila as an engine anyone can choose; it stops being the default. Nothing in this plan removes it.

Written 2026-11-17 against main at `356836f0`. It supersedes the "Decided 2026-10-11: the graphics card is Cycles'" note in `docs/05` M9 as the direction of travel; that note stays true as a description of the build until stage R6 lands.

---

## 0. How to use this

- Work the stages in order (§4). Each has an **exit test**; do not start the next stage until it passes on both reference machines (§6.3), except where a stage says it may run alongside another.
- One step, one pull request. Every pull request states its before-and-after numbers from the bench (R0.1) and its parity result (R0.3, R2.6).
- The decisions in §3 are taken. If one proves wrong, stop, write up what was measured, and ask the owner; do not quietly take another road.
- A tolerance in this document is never loosened to make a test pass. A test that cannot pass is a finding: write it up.
- Keep `docs/01` and `docs/05` true in the same pull request as the change (`docs/SPEC-README.md`, `docs/05` §2).

---

## 1. The goal, and what "Cycles is not needed by default" means

Cycles does one job nothing else in Tequila does: it renders on the graphics card. Everything else it does, Tequila's renderer does already, and several things only Tequila's renderer does (§2.2). So the goal is one sentence: **Tequila's renderer runs on the graphics card, makes the same picture as it does on the processor, and is at least as fast as Cycles on every machine Cycles serves today.**

Two gates, in order:

**G1 — Tequila becomes the default engine.** All of:
1. Speed on the graphics card: on every bench scene, time to equal noise is at most 1.25× Cycles' on the same card, on both reference machines.
2. Speed on the processor: on every bench scene, time to equal noise is at most 1.0× Cycles' on the same processor.
3. Sameness: the graphics-card picture agrees with the processor picture within the parity tolerance (R2.6) on the whole parity set.
4. Look: the owner has looked at the side-by-side sheet (R6.2) of Cycles against Tequila and said yes. Passing numbers are not this gate.
5. Reach: every class of card in the device matrix (R5.3) has a self-test report that passes, or is listed as an accepted loss.
6. Stability: no picture ever waits silently. Opening the Rendered view on a machine that has never run it shows a picture within 1 s (from the processor) and the graphics card's within 5 s.

There is no second gate. Cycles is not removed: after G1 it is the second engine, chosen in the engine menu, as Tequila's renderer is today.

---

## 2. Where we are

### 2.1 Measured, 2026-11-17

**Time to a clean picture** (R0.1's bench, `scripts/bench/run.py`): Ryzen 7 2700X (8 cores, 16 threads), RTX 2060 (driver 610.88), Windows 11, `r0-bench` at its first push. 960×540, no denoiser, adaptive sampling off; seconds of rendering until the noise is 1% of full scale in display values; scene setup and the first render's kernel loading not counted.

| Scene | Cycles, card (OptiX) | Cycles, processor | Tequila, processor | Tequila ÷ Cycles processor |
|---|---|---|---|---|
| simple_product | 1.1 s | 12.4 s | 33.4 s | 2.7 |
| cad_assembly | 1.2 s | 10.9 s | 7.2 s | 0.66 |
| fibre_product | 1.7 s | 19.2 s | 45.2 s | 2.4 |
| subsurface_product | 2.5 s | 24.9 s | 125.6 s | 5.0 |
| lamp_lit | 0.7 s | 8.1 s | 26.6 s | 3.3 |
| Headphones | 0.6 s | 7.4 s | 16.4 s | 2.2 |
| Perfume bottle | 3.0 s | 22.6 s | 35.5 s | 1.6 |
| palma P8 | 8.0 s | 74.2 s | 15.3 s | not comparable: a different picture, below |

Against the card, Tequila's renderer is 6 to 50 times slower. Against Cycles on the same processor it is slower on all but one scene, by 1.6 to 5 times, and **mostly not because a sample costs more**: samples per second are about level (2.9 against 3.0 million on `simple_product`, 2.5 against 2.5 on `fibre_product`), but Tequila's renderer needs one and a half to two times the samples for the same noise. So R1 starts with where its noise per sample comes from, not with making samples cheaper. Its peak memory is a quarter of Cycles' (0.14 GB against 0.52 GB on most scenes). Cycles makes the same noise on the processor as on the card, sample for sample, as the same algorithm should.

Before the bench, equal-sample times measured the one-off way, at 1,024 samples: Headphones 25 s on the card, 122 s for Cycles on the processor and 268 s for Tequila's renderer; the perfume bottle 20, 116 and 130 s; palma P8 34, 302 and 142 s.

**The pictures are not all the same picture.** At 1,024 samples, Tequila's renderer against Cycles on the card, in display-linear values from the 8-bit files: Headphones, mean absolute difference 0.0004, correlation 0.997; perfume bottle, 0.0066 and 0.957; **palma P8, 0.0109 and 0.34**. Cycles shows the palma's clear green shell with the screen lit and the boards behind it; Tequila's renderer shows the shell near black, the screen and everything inside it all but gone. That is one of the three samples the application ships, it is not among the differences `docs/05` M9 lists as by design, and nothing in the test suite caught it. It is why R0.4 exists and why G1.4 is the owner's eyes and not a number.

Earlier figures in the repo agree in kind: the self-test at 64 samples takes Tequila 1.3 s, Cycles on the processor 0.93 s, OptiX 0.12 s, CUDA 0.15 s (`docs/05` §3.11); the fibre scenes take Tequila 131–199 s against 18–28 s on the card (`docs/05` M6).

### 2.2 What Tequila's renderer has, and Cycles in this build does not

Spectral rendering and true dispersion; photon caustics; render layers and regions; the Caustics, Ambient Occlusion, Shadow and Labels passes; denoise strength; the converged-percent readout; a media stack for nested liquids and glass; a rough thin-wall lobe; rounded edges under a normal map; a shadow colour per catcher; unlimited light-link sets; swatches and thumbnails; Intel Macs. Even a "Cycles" picture already leans on Tequila's renderer for picking, the floor reflection, the backplate and finite dome, and the silhouette's alpha.

### 2.3 What only Cycles has

| Only Cycles | Needed for G1? | Answered in |
|---|---|---|
| Rendering on the graphics card (OptiX, CUDA, HIP, Metal) | Yes: this is the plan | R2–R5 |
| Several cards rendering together | Yes, for those who have them | R5.4 |
| Bounce limits per kind (diffuse, glossy, transmission, volume), Filter Glossy, clamping direct light apart, the reflective and refractive caustics switches | Yes: project files carry them | R6.1 |
| Rounded edges seen in reflections (Tequila rounds at the camera's first hit only) | Yes, where it is cheap enough | R3 tier B |

Nothing else. Motion blur, portals and path guiding are in neither engine as Tequila uses them, so they are not part of this plan (§8).

### 2.4 What carrying Cycles costs

About 10,500 lines that exist only to feed it (`native/cycles`, `crates/tequila-render-cycles` of which `translate.rs` is 5,259, `app/src/cycles_view.rs`); a C++ build that takes most of the release build's time, against Blender's prebuilt libraries, NVIDIA's OptiX headers and kernels fetched from a Blender download; a second look to keep in step (58 dual-engine tests that never run in CI because CI does not build Cycles); and more than a dozen recorded workarounds for its behaviour on one device or another, the latest being the five-minute OptiX kernel compile that froze the Rendered view (#161).

### 2.5 Why the renderer is not on the card already

`crates/tequila-render-gpu` is a four-line stub. The integrator is shaped for a processor: one 1,700-line function (`trace` in `crates/tequila-render-core/src/integrator.rs`) over `&dyn Intersector`, with recursion at the first glass and the floor reflection, closures, and heap allocation per path. Two things are already the right shape: a material graph compiles to a flat list of instructions over registers (`tequila-materials/src/graph.rs`, `tequila-render-core/src/shade.rs`), and the surface is a fixed array of five lobes with no allocation (`tequila-materials/src/lobes/openpbr.rs`).

---

## 3. Decisions

**D1 — wgpu, compute shaders, ray queries. Vulkan on Windows and Linux, Metal on a Mac. No DirectX 12.**
wgpu 30 has acceleration structures and ray queries on Vulkan, DX12 and Metal (Metal since v29, macOS 15 and later), behind `EXPERIMENTAL_RAY_QUERY`. It is one API for NVIDIA, AMD, Intel and Apple, and Bevy's path-traced lighting ships on it on Vulkan and Metal. DX12 would add a third backend to test and require shipping DXC, and it lacks storage-buffer binding arrays; every Windows card that can ray trace has a Vulkan driver. Ray-tracing *pipelines* (hit shaders) are not in a wgpu release; nothing here needs them.

**D2 — The kernels are written in WGSL by hand. Invariant 3 becomes "one specification, checked function by function".**
`docs/SPEC-README.md` invariant 3 asks for one shading source. The two tools that could give it are not ready: rust-gpu's maintainers call it "not yet production-ready", it needs a pinned nightly, and its ray queries cannot pass through naga to Metal; Slang's WGSL, Metal and CPU targets are all marked experimental. WGSL through naga is the path wgpu itself tests on every backend. So there are two implementations, and the invariant is kept by machine instead of by construction: every WGSL function that has a Rust twin is run against that twin on the same inputs (R2.4), and that suite runs in CI on a software Vulkan driver. This is far tighter than today's arrangement, where two unrelated integrators are held to a statistical 0.0075. The amendment to invariant 3 lands in the pull request that lands R2.4. Look again at rust-gpu only if it ships a stable-toolchain release and naga parses its ray queries.

**D3 — The processor renderer stays, and gets faster.**
It is the reference the card is tested against, the first picture while the card's pipelines compile, the fallback when a card fails or runs out of memory, and the renderer for machines with no usable card. Once Cycles is gone it is also what replaces Cycles on the processor, so it must not be slower (G1.2, stage R1).

**D4 — Materials are data on the card. No shader is ever compiled because a material changed.**
The card runs one interpreter for the instruction list the processor already runs. The pipelines are compiled once, when the device is made, and they do not depend on the scene. This removes by design the class of stall behind #161.

**D5 — A path is a small state machine; each dispatch advances every live path one segment.**
One segment is: trace, shade, sample the lights with their shadow rays traced inline, choose the next direction. Long walks (subsurface, media) advance a bounded number of steps per dispatch and carry their state. No recursion: the first-glass split and the floor reflection become extra lanes. No sorting by material, since there is one interpreter. Compaction and path regeneration are optimisations to measure after the picture is right (R3 exit), not before.

**D6 — Two ways to intersect behind one interface.**
`trace` and `occluded` in WGSL are either the card's ray queries or a traversal of a tree built on the processor. Cards without ray queries (older NVIDIA GTX and AMD, Macs before macOS 15, Intel Macs) get the second, which Cycles' CUDA numbers say costs perhaps 1.3–2× against hardware. On Apple's M1 and M2 the second may be the faster; choose by measuring on the machine (R5.2).

**D7 — The same random numbers on both.**
The sampler (`tequila-core` `sampler.rs`, `sobol.rs`) is integer arithmetic. Its WGSL port is bit-exact, tested as such, and dimensions are allotted identically. Then a path on the card is the same path as on the processor up to floating-point rounding, and parity can be asserted tightly instead of statistically.

**D8 — The film stays on the processor first.**
The card returns each pass's samples; `tequila-render-core` `Framebuffer` accumulates them with its existing adaptive sampling and firefly rejection. One film implementation, exact parity, and several cards can feed it. Moving accumulation onto the card is an optimisation with its own differential test (R4.6), taken only if readback is measured to be the bottleneck.

**D9 — Photon caustics stay on the processor until last.**
They are an additive layer, and the integrator already draws the line: photons own the paths "light, one or more focusing events, a surface", read at the first surface the camera sees that does not focus light, and the path tracer leaves exactly those out (`integrator/caustics.rs`, `trace_path_with_caustics`). With photons on, the card applies the same leaving-out and the processor traces the photons and the short camera paths that read them. A card-side photon pass is §8.

**D10 — It ships early, as a choice, and Cycles is not touched until G1.**
From the end of R4 the engine list offers "Tequila (graphics card), experimental". Reports from real cards arrive through the self-test (`docs/05` §3.11) while Cycles remains the default.

---

## 4. The stages

Sizes are relative: S is a pull request or two, M a handful, L a dozen or more.

### R0 — Baseline, guardrails, and the look on the processor (M). R0.1 to R0.3 change no behaviour.

**R0.1 The bench measures both engines and equal noise.**
`tequila --bench NAME` with `--engine tequila|cycles`, `--device CPU|GPU` or `--json FILE`, or with a project file for NAME, measures how long an engine takes to a clean picture (`crates/tequila-cli/src/noise_bench.rs`); without them it is the targets report it always was.
- *Built (`r0-bench`):* the generated `simple_product` and `cad_assembly`; `fibre_product` (its body velvet with about 1.4 million real fibres), `subsurface_product` (its body translucent) and `lamp_lit` (three lamps, the environment dark), all in `tequila-scene/src/bench.rs`; and any project file, so the three in `assets/samples`. The enclosed room first planned became `lamp_lit`: what it was to measure is light sampling without the environment's help, and a room needs portals, which neither engine has (§8).
- *Found on the way:* the generated scenes had two floors in one plane, their own ground and the environment's, which a new document has on. They fought, differently in each engine (streaks and black patches in Cycles, rings in Tequila's renderer), and the targets report has measured that since it was written; the environment's ground is now off in them. The targets report also framed its camera on the 80 m ground, so the product was a speck; the engine comparison frames on the product.
- For each: setup time, samples per second, **seconds to equal noise**, peak memory; `scripts/bench/run.py` adds the card's peak memory where `nvidia-smi` can read it.
- **Equal noise, as built.** Two renders at the same sample count with seeds 1 and 2, adaptive sampling and the denoiser off, developed as the application develops them; the noise is the RMS difference of the two in display values over √2. Clean is 0.01 of full scale. A short trial render at 16 samples finds the sample count that reaches it, and the measurement is made there, because both renderers' noise falls 5-12% faster than the square-root rule for each fourfold of samples and a time scaled from far off overstates. This replaces the 16,384-sample references first planned here: no references to make or keep, and each engine is measured against itself, so a wrong picture (R0.4's business) does not count for or against its speed.
- What a Cycles measurement leaves out: the floor reflection the application lays over a Cycles render. It is Tequila's renderer's work and takes no seed, so its noise could not be measured; the row says when a scene had one. Tequila's renderer traces its own floor reflection inside the path, so on such scenes its rows carry work that Cycles' do not.

*Test:* `scripts/bench/run.py` runs on both reference machines; its table replaces §2.1 here and joins `docs/05` §4.

**R0.2 Keep the look Cycles makes.**
The pictures Cycles makes of what Tequila ships, rendered once and kept: the record of the look existing projects were made in, which does not move when Cycles is upgraded. A difference from it is flagged, not failed: the owner decides whether Tequila or Cycles is right (R0.4).
- *As built:* `scripts/look-refs/looks.py make` renders, with Cycles, through the command line's render (the render queue's own, with each project's look and denoising), at 1,024 samples: the three samples and the demo scene in plastic, metal and diffuse at 960×540, every Studio applied to the Headphones, and every library preset on a ball 5 cm across seen as the library's previews see theirs, at 256×256.
- *Made 2026-11-17:* 623 pictures (610 presets, 7 Studios, 3 samples, 3 demo), by Cycles on the RTX 2060 (OptiX; it renders the same pictures as on the processor, sample for sample, R0.1), from main's renderer, in 49 minutes. `look-refs-1.zip`, 39 MB, is on a **draft** release of the repository, which only its writers can see and which starts no release build and no Discord announcement; it must never be published. Its SHA-256 is in `scripts/look-refs/look-refs-1.sha256`, and `compare` refuses a zip that does not match.
- The first plan put it on a published release named `look-refs-1`; the repository is public, and publishing any release builds the Mac and Linux packages and announces them on Discord.

*Test:* `scripts/look-refs/looks.py compare --refs look-refs-1.zip --engine cycles` reports correlation 1.000 and no difference against Cycles itself; with `--engine tequila`, the default, it reports Tequila's renderer against the look, worst first, with sheets of the pictures that differ (R0.4's input).

**R0.3 Name the parity set.**
One list of small scenes that between them use every feature in R3's tiers, each with a fixed seed, size and sample count. R2 starts it with one scene; each R3 step adds the scene for its feature.
- *As built:* `tequila_scene::parity::SET` (`crates/tequila-scene/src/parity.rs`), generated as the benchmark scenes are, each entry saying what it covers. In `tequila-scene` rather than a renderer's crate so both backends' tests read the same list. Its first scene is `grey_product`, `simple_product` in grey diffuse under the default environment, 320×180, 256 samples, seed 7: what R2.5 renders.
- The processor's half of the test, `crates/tequila-render-cpu/tests/parity_set.rs`: every scene renders, finite, not black, and the same twice from its seed. R2.6 adds the card's half beside it.

**R0.4 The look audit, and its repairs, on the processor.**
Run R0.2's comparison with Tequila's renderer against the Cycles archive over the whole set, and put every picture whose correlation is under 0.98 or whose mean differs by more than 5% on one sheet, worst first, for the owner. For each: find the cause, decide with the owner which engine is right, and fix Tequila's renderer where it is the one that is wrong, each fix with a test that holds it (`docs/05` §2). Start with the palma P8 (§2.1): establish whether its dark shell is light not reaching the inside (shadow rays and the lights behind clear plastic), the shell's medium being applied to everything it encloses, or a bounce limit, by rendering it with each of the three changed in turn. Second on the list, found by R0.1's `subsurface_product`: the Translucent type at its defaults (subsurface colour 0.9, 0.75, 0.7; radius 5 cm) comes out pink in Tequila's renderer and mid grey in Cycles, on the same scene and settings; which is right is for the owner to judge against what the type is meant to be. The card is ported from the processor renderer, so a look that is wrong here would be copied faithfully in R3; this step comes first for that reason.

*Exit:* R0.1 numbers recorded for both machines; R0.2 archive published; R0.4's sheet has no picture left that the owner has not either seen fixed or accepted as it is.

### R1 — The processor renderer at least as fast as Cycles on the processor (M). May run alongside R2 and R3.

Measure first. Profile `Headphones` and whichever scenes R0.1 shows slowest against Cycles, with a sampling profiler (`samply`), and work down the profile. The survey's suspects, to confirm or dismiss by measurement, not to fix on sight:
- allocation per path: the two `Registers` vectors made at the top of `trace`, `PathAovs::new(expressions.clone())` per pixel in `tequila-render-cpu/src/renderer.rs`, the vector made when more than 16 lights are sampled;
- dynamic dispatch in the inner loop: `&dyn Intersector`, `&dyn Bsdf`, `&dyn Fn` closures;
- single-ray Embree calls for coherent camera and shadow rays (`embree.rs` `rtcIntersect1`, `rtcOccluded1`);
- the first-hit bevel probes.

*Rule:* a change that does not change the mathematics must leave the picture **bit-identical** at a fixed seed on any number of threads; `rendering_is_bit_identical_across_thread_counts` (`tequila-render-cpu/src/renderer.rs`) and `tracing_is_deterministic` (`integrator.rs`) already assert the second half, so add a fixed-seed image hash for the scenes being optimised and say in the pull request that it did not move. A change that does alter the picture is a different kind of pull request and states why.

- *Found and fixed first (2026-11-18, `r1-noise`):* where the noise is, by `TEQUILA_BENCH_DUMP=DIR`, which makes `--bench` write both seeds' pictures raw, compared region by region against Cycles'. On `simple_product` at 64 samples Tequila's noise was twice Cycles' (0.0212 against 0.0106), almost all of it on the floor, and seven times Cycles' in the floor's highlight of the softbox. Two causes, both changing the noise and not what a picture converges to:
  - *The sampler's padding groups were eight wide*, so a bounce's direction came from Sobol′'s dimensions 4 and 5, a pair that integrates a disc at 256 points only twice as well as random points (dimensions 0 and 1: fourteen times). Now four wide, as Burley recommends, and each two-dimensional draw is a group's leading pair (`tequila-core/src/sampler.rs`). 0.0212 to 0.0185.
  - *The layer stack chose its lobes by their albedo face on*, so a plastic's specular interface, 4% face on and sampled at a 5% floor, was drawn one time in twenty where a floor seen at 75° returns a fifth of its light from it. `OpenPbrBsdf::aim(wo)` re-weighs them by their albedo towards the viewer, through the layers above, as Cycles weighs its closures. 0.0185 to 0.0115, against Cycles' 0.0106; a sample costs about 9% more.
  - Time to clean against Cycles on the processor, before and after: simple_product 2.69 to 1.38, fibre_product 2.35 to 1.19, subsurface_product 5.04 to 3.14, lamp_lit 3.28 to 3.04, Headphones 2.23 to 2.01, Perfume bottle 1.57 to 1.53, cad_assembly 0.67 to 0.59.
  - Left: `lamp_lit` is now within 1.4 times Cycles' samples and its gap is the cost of a sample (2.1 against 4.3 million a second); the Headphones' is still noise (1.7 times the samples, on dark plastics lit by thin strips painted into the environment; Tequila's floor reflection is in its picture and left out of Cycles', so compare without the bottom rows); `subsurface_product` is both.

- *Then the cost of a sample (2026-11-18, `r1-speed`), every change bit-identical:* profiled on the RTX 2060 PC with a sampler built into the bench for the purpose (samply needs administrator rights on Windows), each step checked by rendering two seeds with the build before it and comparing the raw pictures byte for byte (`TEQUILA_BENCH_DUMP`, `cmp`). What the profile found, and what was done:
  - Sobol′'s loop over the index's bits ran all 32 times, the index being shuffled: 10% of `lamp_lit`. Now four reads of a 16 KB table.
  - The specular interface's energy tables were read again for the same `wo` by every evaluation at a shading point and by every layer beneath: 16%. `DielectricSpecular::aim` reads them once (the stack's `aim` calls it), and the stack's two copies of its interface share the reads.
  - An environment turned down to nothing was still sampled, a table search and a texture read per vertex, and read again for every ray that escaped: 13%. Skipped.
  - The two register banks made for every path allocated sixteen slots each, used or not: 3%. They grow on first use.
  - The noise patterns called the C library's `fmodf` on every coordinate to wrap it at 100,000: 6% of the Headphones, whose leather grain is procedural and two thirds of its time. A coordinate nearer than that is returned as it is.
  - Samples a second, before and after: `lamp_lit` 2.18 to 3.57 million (Cycles 4.3), `simple_product` 2.72 to 3.82 (Cycles 3.1), Headphones 2.26 to 2.65 (Cycles 2.65).
  - Leads not taken yet: the Headphones' bump evaluates the height four times a point (central differences) where Cycles does twice more than the point itself (forward), which would change the picture slightly; `lamp_lit`'s light tree (7%), chosen because one of its lamps is weak, is a strategy and not a cost to cut without the noise it saves.

- *Then the subsurface walk (2026-11-18, `r1-walk`):* `subsurface_product`, the furthest behind, spent half its time tracing rays, nearly all of them the walk's: its translucent body took seventy flights a walk on average, and 98.6% of them ended inside the part. Each traced its ray to the nearest surface however far, and only then drew how far it flew. Now a uniform medium's flight is drawn first and its ray traced only that far, as Cycles' walk does: a ray that need only know whether a surface lies within a few millimetres costs a fraction of one that must find the nearest, and the surface it finds is the same one when it is that close.
  - Samples a second 1.24 to 1.83 million (Cycles 2.63); time to clean 70.1 to 47.4 seconds, 2.8 to 1.9 times Cycles'.
  - Not bit-identical, by one change of rule, Cycles': a flight that meets nothing within its length collides where it ends, where one that met nothing at all went to the sky. Inside a closed part they are the same. They differ for a path that got out of its part some other way, through a part pushed into it (the lens and buttons in this body): 197 of 518,400 pixels moved at 16 samples.
  - Measured and not taken: guiding the walk back towards where it came in (Dwivedi sampling, as Cycles' walk does half its turns) converges to the same picture and has 1.8 times less variance in a white furnace, but none on this scene, whose noise is the lighting the walk carries rather than the walk; and building for AVX2 (`x86-64-v3`) made 0 to 4%.
  - Left on this scene: a sample still costs 1.4 times Cycles', and the noise per sample is 1.16 times, measured against a Cycles picture whose body is grey where the material is pink (an audit question, not R1's).

- *Then fireflies (2026-11-18, `r1-clamp`), a change of look as well as of speed:* on `lamp_lit` 87% of the noise, in both engines, was in the brightest 1% of pixels: glints along the bezel and the body's foot, and on the Perfume bottle's glass. Cycles' firefly rejection clamps every indirect contribution at 10, its red, green and blue summed; Tequila's rejected samples far from their pixel's spread and clamped nothing, and its Clamp samples clamped the whole sample. Now each contribution is clamped as it is found, as Cycles clamps, under the same settings (`docs/01` §3.5): firefly rejection clamps indirect light at 10. Not what Cycles clamps: light the camera sees through mirrors and clear glass alone, which counts as direct (a lamp in a mirror is the lamp), and the caustic photons, which are a smooth estimate and not fireflies.
  - Noise at 64 samples: Perfume bottle 0.0163 to 0.0098 (a third of the samples), `lamp_lit` 0.0120 to 0.0095 (0.6 times); `simple_product`, `subsurface_product` and the Headphones unchanged. Displayed light: Perfume bottle 1.1% less, from highlights already at white; `lamp_lit` 0.04%; the rest nothing.
  - Time to clean against Cycles, both measured on the RTX 2060 PC on 2026-11-18 (Tequila / Cycles seconds): palma P8 7.8 / 76.9 (0.10, but #169 is not in this branch and palma's shell is black here), `cad_assembly` 4.8 / 11.7 (0.41), Perfume bottle 12.1 / 23.2 (0.52), `fibre_product` 18.3 / 21.2 (0.86), `simple_product` 13.6 / 12.7 (1.07), `lamp_lit` 9.3 / 8.2 (1.13), Headphones 12.2 / 7.9 (1.54), `subsurface_product` 46.9 / 27.2 (1.72).

- *Then the bump (2026-11-18, `r1-bump`):* the Headphones' surfaces are grained by a procedural pattern, and its bump was half of all the render's time (3.99 million samples a second with it switched off, 2.11 with it). Its slope was measured by central differences, the height evaluated four times a point; now from the height the material's own program already found at the point and two more a tenth of a footprint along, as Cycles' bump does. 2.0 to 2.7 million samples a second; the picture the same within its noise (mean -0.02%).
  - The Headphones' remaining gap is noise, 1.6 times the samples on the product, and not yet explained. A fifth of Tequila's noise there is the floor's reflection, which only Tequila draws. On the product: not the bump (switched off, 0.0099 to 0.0098), not its materials (every part one plain plastic, or all matte: still 1.4 times), not light that has bounced (direct light alone, matte: relative noise 0.0505 against Cycles' 0.0413, the same 1.2 times), not the measurement (each engine's seed-to-seed estimate matches its error against a 512-sample reference: 0.0117 / 0.0111, and Cycles 0.0085 / 0.0084), not the sampler's stratification of the light sample (random instead: 0.0128 to 0.0150), and not the environment's sampling table being filtered (the raw texels: no change). So it is how a matte surface samples a 1024 x 512 environment lit by three thin painted strips, which both engines are meant to do the same way; what Cycles does differently there is the lead.

- *Where R1 stands (2026-11-18, all of the above, #172 to #177):* time to clean against Cycles on the processor, both engines measured the same day on the RTX 2060 PC, with #176 (Cycles given the Translucent body's subsurface colour) in the Cycles build so `subsurface_product` compares like with like. Tequila / Cycles seconds, and the ratio at R0.1 and now:

  | Scene | Tequila | Cycles | R0.1 | now |
  |---|---|---|---|---|
  | palma P8 | 80.8 | 78.6 | 0.21 | 1.03 (with #169 applied; without it the shell renders near black and took 7.4 s) |
  | `cad_assembly` | 4.8 | 10.9 | 0.67 | 0.44 |
  | Perfume bottle | 11.9 | 23.0 | 1.57 | 0.52 |
  | `fibre_product` | 17.8 | 19.2 | 2.35 | 0.93 |
  | `simple_product` | 13.0 | 12.4 | 2.69 | 1.05 |
  | `lamp_lit` | 8.8 | 8.2 | 3.28 | 1.07 |
  | Headphones | 9.3 | 7.3 | 2.23 | 1.27 |
  | `subsurface_product` | 46.2 | 31.8 | 5.04 | 1.45 |

  The account of what is left, scene by scene:
  - `simple_product` and `lamp_lit`, 1.05 and 1.07: within what two runs of the same build differ by on this PC (simple_product's samples a second have read 3.3 to 3.7 million). Level.
  - Headphones, 1.27: noise, about 1.6 times the samples on the product; a sample costs less than Cycles' (3.49 against 2.66 million a second). Not yet explained (above): direct light from a thin-striped environment onto matte parts is 1.2 times noisier, with the method the same as Cycles' as far as its source shows. A fifth of Tequila's noise in this scene is the floor reflection only Tequila draws.
  - `subsurface_product`, 1.45: noise now, 160 samples against Cycles' 112, at the same cost a sample (1.87 against 1.94 million). With Cycles given the right colour, its walk costs it more than it did grey; Tequila's walk through the pink medium, sampled per channel, is the noise to look at next.
  - palma P8, 1.03 with #169 (its clear shell, split by material, is one solid there; this branch predates it): noise, 336 samples against Cycles' 136, at half the cost a sample (2.65 against 1.32 million). The light through the lime shell is the noise to look at.
  - Not yet measured: the MacBook (G1.2 asks for both machines).

- *Then the Headphones and `subsurface_product` (2026-11-19, `r1-env-facing`):*
  - *Where the Headphones' noise was.* With every material plain, then all matte, then direct light alone, Tequila stayed 1.2 times noisier; with light sampling switched off in both engines it was the less noisy (relative 0.120 against Cycles' 0.133). So it was the environment's light sample, and counting where it went said why: 63% of its draws fell below the surface they were lighting - the strips are rim lights behind the product - and the dim front of the sky, which lights what the camera sees, was drawn seldom. Cycles' own source shows it draws the sky as Tequila did; it gets away with it here for reasons not found.
  - *Drawing the sky a surface can see.* The environment cut into 32 by 16 tiles, each a cone of directions, and for each of 384 ways a surface can face, each tile's chance in proportion to its light and the most it can lean onto such a surface, none from a tile wholly behind it (`Environment::sample_facing`). The density is known for any direction and facing, so a bounce that meets the sky is weighed against the sample the surface it left actually took (`sky_facing`). A walk's doorway and its way out draw what they face too: behind the one is the part, and the other is lit from outside alone. Below the horizon 63% to 9% of the draws; matte parts lit directly as Cycles' (0.0426 against 0.0413); unbiased, a matte ball under narrow strips gathering the same light with a quarter of the variance. Firefly rejection takes more from the heavier tail of what bounces now find alone, so the Headphones read 0.1% darker with it on, nothing with it off.
  - *The rest.* A specular interface's compensation lobe drawn a quarter as often (it is found by the interface's own sampling and the light samples anyway): dark glossy plastic 3% less noisy. Subsurface walks guided back towards where they came in, as Cycles' are: 7% less time a sample, the same noise. The Voronoi texture behind the leather grain no longer hashes the cells' colour for a graph that reads only the distance, nor divides to count its 125 cells: a fifth of the Headphones' time to a sixth, bit-identical.
  - Tried and left out: steering the sky away from below a floor for what stands on it (noisier, likely because the floor is fitted to the product and some of the sky below the horizon reaches past its edge); a minimum chance for tiles at a surface's horizon, the balance heuristic, a 256-step cap on walks (Cycles' - faster, but it drops the light of the walks that go deep, 0.9% of the body's).
  - Time to clean against Cycles, both measured on 2026-11-19 on the RTX 2060 PC, Cycles with #176 (Tequila / Cycles seconds): Headphones 7.7 / 7.9 (0.97, was 1.27), `subsurface_product` 37.6 / 39.5 (0.95, was 1.45), `simple_product` 11.3 / 14.0 (0.81), `fibre_product` 18.6 / 22.3 (0.83), `cad_assembly` 5.4 / 11.7 (0.46), Perfume bottle 12.6 / 26.3 (0.48), `lamp_lit` 9.7 / 8.9 (1.09, its sky turned off, so untouched by this). palma P8 measured 1.03 with #169 before this, its shell lit by the sky this improves.

- *Then `lamp_lit`'s cost a sample (2026-11-19, `r1-lamp`):* its noise was level with Cycles' and its gap the cost of a sample (3.1 against 3.96 million a second). Seven cuts, found by profiling, every one of them work done twice or not needed; all but the light tree's leave every picture bit-identical, checked on five bench scenes:
  - The light tree's importance by the cosines of its angles, the sums and differences by the identities, as Cycles' is: an `acos` or `asin` at every node of every descent was 4%. A test holds it to the angles' answer over 200,000 cones, points and bounds; the first version read a point light's whole sphere seen from inside its bounds as no angle at all, which the light-selection test caught.
  - A specular interface reads its tables from its place on their roughness and index axes, found once at a shading point, and two tables read at one place together.
  - The face-on choice of lobes, which `aim` replaces at once, is made only where `aim` declines to choose.
  - Each interface's hemispherical albedo read once a shading point, not once for each layer that asks.
  - A medium's volume boxed, so the stack of media every path carries is small to move; and the two materials a path keeps for evaluating graphs are made when it first meets one, not for every path.
  - Sobol′'s per-group shuffle of the sample index, the same for the group's four dimensions, worked out once a sample per group.
  - Samples a second on `lamp_lit` 2.68 to 3.1 million with the PC busy, 3.7 to 3.8 with it quiet (Cycles 3.9 to 4.1); the same noise, the same picture (mean 0.48902 before and after). The cuts are in what every scene does, so every scene gains some.
  - Time to clean against Cycles, all eight measured on 2026-11-19 on the RTX 2060 PC once it was quiet, with #169 added for palma's sake and #176 in Cycles for `subsurface_product` (Tequila / Cycles seconds; where the two are close, the median of three runs of each): `cad_assembly` 4.7 / 12.2 (0.39), Perfume bottle 11.8 / 26.7 (0.44), `simple_product` 9.1 / 13.9 (0.65), `fibre_product` 17.5 / 21.6 (0.81), `lamp_lit` 8.1 / 9.0 (0.90, was 1.09), Headphones 7.7 / 8.0 (0.96), `subsurface_product` 34.9 / 35.3 (0.99), palma P8 83.4 / 84.2 (0.99). With other builds running, `lamp_lit` read 0.91. No scene now takes longer to clean in Tequila than in Cycles on this PC; `subsurface_product` and palma are level with it.

- *Then palma P8's shell (2026-11-19, `r1-walk-albedo`):* palma's gap was noise alone: Tequila's samples cost half Cycles', and it needed 336 of them to Cycles' 136. 97% of its noise was in the device, 1.4 times Cycles' variance there and the same mean; with the shell's scatter turned off the two were level (1.04), so it was the walk through the shell. The lime shell absorbs blue eighty times faster than green, and a walk drew each flight in a channel chosen by the path's throughput alone: a third of the flights through the shell were blue's, nearly all of them stopping inside it and bending paths the other channels would have carried straight on. Now the channel is chosen by throughput times the medium's albedo, as Cycles' walks choose it (after Chiang, Kutz and Burley 2016); a medium that only absorbs is drawn by throughput as before.
  - Unbiased: with firefly rejection off the device's light is the same to 0.02%, within two seeds' spread. With it on, it is 1.6% brighter, being clamped less.
  - `subsurface_product`, the Headphones and the Perfume bottle are unchanged, to the noise and the mean: their media scatter much the same in every channel.
  - palma P8, with #169 added, Tequila / Cycles median of three: 46.6 / 82.5 seconds (0.56, was 0.99); 152 samples where it needed 336.
  - Found on the way: with firefly rejection off, the device seen through the shell was 16% brighter in Tequila than in Cycles (display mean 0.415 against 0.356). Half of that was Cycles' (#182: the split shell's tint stayed on past its back half; 0.356 to 0.389). The rest was on the circuit board seen through the shell, which neither engine had right: with every limit and blur lifted the two agree there to 0.4%, and over the device to 0.6% (0.430 against 0.428). Tequila lost light because a pass through glass was counted against the general bounce limit as well as the transmission one, against `docs/01` §11: #183, which brings the device to 0.431 and the Perfume bottle's glass level with Cycles'. Cycles loses it to Filter Glossy at 1 (Blender's default, which Tequila sends), which makes the shell rough after the first bounce off the board and takes 44% of the board's light; left as it is, as the look Cycles projects were made in, for R6.1 to map onto Tequila's regularisation.

*Exit:* G1.2 on both machines, or a per-scene account of the remaining gap that the owner accepts.

### R2 — The first picture from the card (M)

**R2.0 Spike, on a branch that is thrown away.**
On the RTX 2060 (Vulkan) and an Apple-silicon Mac (Metal, macOS 15 or later): make a wgpu 30 device with `EXPERIMENTAL_RAY_QUERY`; build a BLAS per mesh and one TLAS for `Headphones`; a compute shader sends a camera ray and eight ambient-occlusion rays per pixel through ray queries; read the buffer back to a PNG. Record: rays per second against Embree's on the same scene, the cold time to compile the pipeline, acceleration-structure build time, card memory.

*Pass:* the picture is right on both machines; rays per second are at least 5× Embree's on the RTX 2060; the cold pipeline compile is under 5 s.
*If Metal's ray queries misbehave* (wgpu issues #9215, #9100 are open or recently closed): do R5.1 first and run the Mac on the processor-built tree; carry on.
*If Vulkan's do:* stop and ask the owner. The alternative is Vulkan and Metal directly (`ash`, `metal`), which is a different plan.

*Measured, November 20, the RTX 2060 half* (branch `r2-spike`, `cargo run --release -p tequila-render-gpu --example r2_spike -- PROJECT 1920 1080 OUT`; driver 610.88, against Embree on the Ryzen 7 2700X's 16 threads through the renderer's own `Intersector`, AO radius 5% of the scene's diagonal):

| Scene | Triangles | Card, M rays/s | Processor, M rays/s | Card ÷ processor |
|---|---|---|---|---|
| Headphones | 102,408 | 2,498 | 72.7 | 34× |
| Perfume bottle | 75,129 | 2,775 | 87.4 | 32× |
| palma P8 | 193,531 | 2,792 | 83.9 | 33× |

- The pictures are the same: hit or miss differs at 0, 1 and 0 pixels of 2 million, and the mean occlusion agrees to four decimals (both draw the same hashed directions).
- Acceleration structures in 0.025 s on the card (one BLAS per mesh, 8 to 151 instances), 0.07–0.19 s in Embree. Card memory 13–34 MiB.
- Cold pipeline compile 0.16 s, for a shader of 100 lines; the megakernel will be far larger, so R2.5 measures it again.
- The processor's figure includes `Hit`'s full bookkeeping (shading normal, UVs, magnitude), which bare traversal does not, so the margin for traversal alone is smaller; it is still several times the 5× the pass asks.
- wgpu 30 details that cost a try each: device creation needs `experimental_features: unsafe { ExperimentalFeatures::enabled() }`; the shader needs `enable wgpu_ray_query;`; the opaque flag is `RAY_FLAG_FORCE_OPAQUE`; `required_limits` must raise the acceleration-structure limits, which default to zero (the spike passes `adapter.limits()`).

*Pass on the RTX 2060.* The Mac half is still to run.

**R2.1 The crate.**
In `crates/tequila-render-gpu` (workspace `rust-version` rises to what wgpu 30 needs; wgpu 22 is already in `Cargo.lock` through another dependency, so pin this crate's version explicitly):
- `device.rs`: choose the adapter, request features and limits (raise `max_buffer_size` and `max_storage_buffer_binding_size`; they default to 256 and 128 MiB), report why a device was refused.
- `scene.rs`: `GpuScene::upload(&SceneSnapshot)`: vertex and index buffers, instances, BLAS and TLAS, material programs, textures, environment tables, light tables.
- `renderer.rs`: the same calls the processor renderer offers (`render_pass`, `render_pass_with_aovs`, `render_pass_until` in `tequila-render-cpu/src/renderer.rs`), so `app/src/orchestrator.rs` and `app/src/batch.rs` can hold either.
- `wgsl/`: one file per module, joined by a small include step. A test validates every module with naga and writes it out as SPIR-V and MSL, so a shader that does not compile fails CI on a machine with no card.

**R2.2 Layouts agree by test.**
Every struct that crosses to the card is `#[repr(C)]` with `bytemuck`. A test reads the WGSL through naga and asserts each struct's size and every member's offset against Rust's.

**R2.3 The sampler, bit-exact.**
Port `sobol.rs` and `sampler.rs`. *Test:* a million (pixel, sample, dimension) triples give identical `u32` on both.

**R2.4 The differential harness.**
A helper, `run_wgsl(function, inputs) -> outputs`, that wraps any WGSL function in a generated compute shader over two buffers. Every later port uses it: random inputs from the existing χ² and furnace tests' ranges, the Rust function as the oracle, agreement to 1e-4 relative (1e-6 absolute near zero). The Linux CI job installs Mesa's software Vulkan driver (`mesa-vulkan-drivers`) and sets `TEQUILA_REQUIRE_GPU_TESTS=1`, under which a missing adapter is a failure, not a skip. This pull request also amends invariant 3 (D2).

**R2.5 First transport.**
Camera (perspective, thin lens), instanced triangles, one grey diffuse surface for everything, the environment sampled by its existing tables (`environment.rs`) with next-event estimation and MIS, Russian roulette and the bounce limit as `integrator.rs` has them. Samples go back to `Framebuffer` (D8).

**R2.6 The parity test.**
`grey_product` from the parity set (R0.3), at its size, samples and seed: mean absolute difference under 0.002 in scene-linear, and at most 0.1% of pixels more than 0.02 apart (those are silhouette pixels where the two intersectors round differently; `docs/01` §10.4's "no pixel" becomes this, in the same pull request).

*Exit:* R2.6 passes on both machines; `tequila -i Headphones.tequila --engine tequila --device GPU` writes the picture (`--device` is read only for Cycles today); R0.1 has a first graphics-card row.

### R3 — Everything the processor renderer does (L)

Each item is: port to WGSL, differential tests for its functions (R2.4), its scene added to the parity set (R0.3) and passing R2.6's tolerance. Tiers are by how many pictures need the thing.

**Tier A — most pictures.**
- The graph interpreter: the twelve instruction kinds, registers as a fixed array in path state (`shade.rs`, `graph.rs`).
- Image textures through texture binding arrays; the texture nodes; then the procedural nodes one family per pull request (`nodes.rs`, and `tequila-render-core/tests` `finishes`, `patterns`, `tiles`, `stochastic` as their oracles).
- The surface: EON diffuse, GGX dielectric and conductor with the energy tables of `energy.rs` as textures, coat, the LTC sheen with its table, rough transmission, thin walls, thin film, flakes, anisotropy, emission, cutout opacity (`lobes/`, `openpbr.rs`). The χ² suite (`tests/analytic/tests/chi_squared.rs`) is run against the WGSL lobes through R2.4.
- Bump and normal maps (`surface_bump.rs`).
- Lights: rectangle and disc by spherical-rectangle sampling, point, spot, IES, distant; the alias table and the light hierarchy; emissive triangles and their tree; light and shadow linking; visibility flags (`light.rs`, `emitters.rs`, `ies.rs`).
- The ground, the shadow catcher, holdout, the floor reflection as an extra lane, backplate, colour and dome backgrounds, pins (`environment.rs`, `pins.rs`, `tequila-render-cpu/src/floor.rs`).

**Tier B — glass, liquids, skin, colour.**
- The media stack (depth four), homogeneous absorption and scattering, the first-glass split as an extra lane.
- Subsurface: the random walk in bounded chunks.
- Spectral mode: four wavelengths in place of three channels, the Jakob–Hanika table (`spectral.rs`) as a 3D texture, the hero wavelength alone after a dispersive refraction.
- Rounded edges (`bevel.rs`): probe rays inline. First match the processor (camera's first hit only). Then measure rounding at every bounce on the card; if it costs under 10%, turn it on for both renderers' pictures to agree with what Cycles showed in reflections, and say so in `docs/01`.
- Cutaways, projected labels, tri-planar and object-space mapping.

**Tier C — the rest.**
- Fibres: curves as procedural boxes in the acceleration structure with the ribbon test in the candidate loop, and the Chiang lobe (`lobes/fibre.rs`).
- Heterogeneous media: the OpenVDB grid as a 3D texture, null-collision tracking in bounded chunks (`density.rs`).
- Manifold caustics (`integrator/manifold.rs`).
- Passes: the light-path matcher as a table in path state (`lpe.rs`, `aov.rs`), the data passes, Cryptomatte, light groups; denoiser guides.
- Regions and render layers.

**Tier D — stays on the processor (D9):** photon caustics, composited.

*Exit:* the whole parity set within tolerance on both machines; `every_material_renders` on the card; all library presets, card against processor, correlate at 0.99 or better; G1.1 holds on both machines. If G1.1 does not hold, now is when to add compaction and path regeneration (D5) and to profile the kernels.

### R4 — In the app (M)

- **R4.1 The engine list.** `RenderEngine` (`tequila-scene/src/render.rs`) gains `TequilaGpu`, key `tequila-gpu`, label "Tequila (graphics card)", marked experimental in the Render window and on the viewport's chip.
- **R4.2 Final renders.** `batch::engine_for` and the fallback at `batch.rs` already step down from Cycles on the card to Cycles on the processor to Tequila; add the step from Tequila on the card to Tequila on the processor. Because both make the same picture, the fallback is silent apart from a line in the job's note.
- **R4.3 The Rendered view.** The render thread in `orchestrator.rs` drives the card's renderer through the same resolution ladder. An edit costs what it changes: a material is a buffer write (target: under 50 ms from drop to picture, `docs/01` §15); a moved part is a new TLAS; changed geometry rebuilds that mesh's BLAS only.
- **R4.4 Never a silent wait.** The processor draws the view from the first frame; the card takes over at a pass boundary once its pipelines exist. The "preparing" note of #161 (`CyclesView::overdue`, `EngineNote::Preparing`) covers any card step over a second. Tests: a device made with a pipeline that sleeps still shows a processor picture within 1 s; a lost device mid-render falls back and finishes.
- **R4.5 Memory.** Estimate the scene's card memory before upload; over budget, stay on the processor and say so in the view (`docs/01` §10.3).
- **R4.6 Readback.** Measure it. Only if it limits the frame rate, accumulate on the card with the film's own mathematics and a differential test against `Framebuffer`.
- **R4.7 The self-test.** `tequila --self-test` gains an entry per adapter for Tequila on the card. Its reference is the processor's reference: one picture, not one per engine.

*Exit:* G1.6 on both machines; the Rendered view on the card orbits at 60 pictures a second or better on the RTX 2060 at 1080p (`docs/01` §15) and reaches a clean picture no later than Cycles does today (0.3–0.5 s, `docs/01` §9). Ship it as experimental in a feature update and ask for self-test reports.

### R5 — Every card Cycles serves today (M)

- **R5.1 The processor-built tree.** A wide BVH built on the processor (evaluate the `obvhs` crate and its licence before writing one), flattened to buffers, traversed in WGSL behind D6's interface; fibres and cutouts as leaf kinds. Parity against the ray-query path on the whole parity set.
- **R5.2 Choosing.** Where both exist, time each for one second in the self-test and remember the faster in Preferences.
- **R5.3 The device matrix.** A self-test report that passes, gathered from users through the experimental release, for each of: NVIDIA RTX; NVIDIA GTX 10 and 16 series; AMD with and without ray-tracing hardware; Intel Arc; Intel integrated; Apple M1 or M2; Apple M3 or later; macOS 14; an Intel Mac; Linux on Mesa. A class with no report is neither passed nor failed: say which are missing.
- **R5.4 Several cards.** One renderer per adapter, each given its own tiles, all feeding the one film (D8).

*Exit:* G1.5.

### R6 — Tequila becomes the default (S)

- **R6.1 Settings keep their meaning.** In both of Tequila's backends: bounce limits per kind (diffuse, glossy, transmission, volume) beside the existing total and transmission limits; Filter Glossy mapped onto the existing roughness regularisation; direct and indirect clamps apart (`RenderSettings::clamp_direct` is passed to Cycles only today); the two caustics switches mapped onto Tequila's caustics settings. The "Advanced (Cycles)" fold in `app/src/lighting_tab.rs` and `render_window.rs` loses the word Cycles.
- **R6.2 The look sheet.** For the R0.2 set plus twenty library materials on the owner's own models: Cycles (from the archive), Tequila on the card, and their difference, side by side on one page. The owner decides (G1.4).
- **R6.3 The switch.** `RenderEngine`'s default becomes `TequilaGpu`: new projects, and the CLI, farm and network when no engine is given. A project saved with `cycles-gpu` or `cycles-cpu` opens with Cycles, as saved, so no existing project changes its look on opening; switching is one choice in the engine menu. Cycles stays in the menu, not labelled "legacy".

*Exit:* G1, and the owner's word.

### R7 — Cycles stays as the second engine (S)

Not a removal (owner, 2026-11-17). After R6, Cycles is kept working as a choice:
- Its build, kernels and packaging stay in the release workflows, and each Cycles upgrade stays its own pull request.
- The dual-engine tests in `batch.rs` stay.
- A Cycles upgrade renders a new look archive (`look-refs-N`) and compares it with the last: a change in Cycles' look is a finding for the owner, like one in Tequila's.
- `docs/01` §10 and `docs/05` M9 say Tequila is the default and Cycles the second engine.

*Test:* both engines render the R0.2 set from the release build on both reference machines.

---

## 5. Tests this plan adds

| Test | Asserts | Runs |
|---|---|---|
| WGSL validates | every module compiles through naga to SPIR-V and MSL | CI, no card |
| Layouts | struct sizes and offsets equal in Rust and WGSL | CI, no card |
| Sampler | bit-exact random numbers | CI, software Vulkan |
| Differential | each WGSL function against its Rust twin, 1e-4 | CI, software Vulkan |
| χ² on the card | the existing sampling tests against the WGSL lobes | CI, software Vulkan |
| Parity set | card against processor, 0.002 mean, 0.1% of pixels over 0.02 | both machines, each pull request |
| Library | all presets correlate 0.99, card against processor | both machines, R3 exit and after |
| Look archive | any engine against Cycles' recorded look | on request, R6.2, and on each Cycles upgrade |
| Bench | the gates' numbers | both machines, each stage exit |
| Never a silent wait | first picture within 1 s; fallback on a lost device | CI |
| Self-test | the card's picture is the processor's | users' machines |

Hosted CI machines have no graphics card and vary too much to gate speed. Speed is measured on the two reference machines and recorded in `docs/05` §4 at each stage exit.

---

## 6. Working rules

### 6.1 The repository's
- `docs/SPEC-README.md` invariants; `docs/05` §0 and §2. Analytic test before the feature.
- Algorithms come from published literature or permissively licensed code (`docs/TEQUILA_SPEC.md` legal rules); anything ported is recorded in `THIRD_PARTY_LICENSES.md`.
- Reference images change only in a commit that says why (`docs/05` §3.4).
- No repository-wide `cargo fmt`.

### 6.2 The owner's
- Nothing merges and nothing is released without the owner's word. Feature updates go out every second Saturday; fixes any day.
- The owner judges the look. A passing test is not "it looks right".
- Reports to the owner carry pictures.

### 6.3 Machines
- An Apple-silicon Mac (Metal), where the work is done.
- A Ryzen 7 2700X with an RTX 2060 on Windows (Vulkan), which runs checks on request: ask for `scripts/bench/run.py` and the parity set by name, and for a cold-cache run whenever a pipeline changes.

### 6.4 Risks, and what to do when one bites

| Risk | Sign | Then |
|---|---|---|
| wgpu's ray queries are experimental and change between releases | a wgpu upgrade breaks the build or a picture | pin wgpu; upgrade in its own pull request with the parity set as the gate |
| Metal ray-query bugs | wrong or missing geometry on the Mac only | R5.1's tree on the Mac; report upstream |
| The two implementations drift | a differential test fails | fix the WGSL to match Rust, never the tolerance |
| Slow first pipeline compile on some driver | R4.4's cold test over 5 s | split the kernel into smaller pipelines; keep the processor's picture up meanwhile |
| The card's memory runs out | allocation fails or the device is lost | R4.5's estimate; fall back; never crash |
| The card is not as fast as OptiX | G1.1 fails at R3 exit | compaction, regeneration, kernel profiling; if still short, bring the numbers to the owner, who may accept a smaller margin |
| A lane's state outgrows memory | card memory at 1080p above 1 GB for path state | trace in tiles of lanes; it is already how big renders must work |

### 6.5 Where the hard parts are

R2.0 to R3 tier B are design-heavy: a subtle mistake there costs days, and they deserve the most careful session. R0, R1, R3's node families and R4 to R7 are well specified by their tests. The pull request that added this document says which model and effort were suggested for each.

---

## 7. Starting

1. Read this, then `docs/01` §3–§10 and `docs/05` §3.
2. R0.1, R0.2, R0.3, each its own pull request; then R0.4, palma P8 first.
3. R2.0's spike on both machines, which can run while R0.4 is being worked; bring its four numbers to the owner before R2.1.
4. R1 can start as soon as R0.1 has numbers, in a session of its own.
5. R3 does not start on a feature whose look R0.4 still has open.

## 8. After Cycles (not part of the goal)

Blue-noise sampling for a better picture at few samples; photon caustics on the card; accumulation and denoising on the card without a round trip; portals (`interior_mode` is read by nothing in the integrator today); motion blur; textured area lights; next-event estimation in media that are not index-matched; path guiding. Each is a way to be better than Cycles was; none is needed to stop needing it.
