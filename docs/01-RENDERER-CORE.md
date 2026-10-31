# 01 — Renderer Core Engineering Specification

The physics, algorithms, and architecture of the Tequila renderer. This document specifies *how* to build the light-transport engine, not merely what it should do.

Everything here is from published literature. References are given as `[Author Year]` and resolved in §16. Where a choice exists, this document makes the choice and states why — do not treat alternatives as open questions unless a section explicitly flags one.

---

## 1. Scope and design principles

Tequila's renderer is an **unbiased, physically-based, progressive Monte Carlo path tracer** with optional biased modes that the user opts into explicitly (caustics photon mapping, GI cache, path regularization above a threshold).

Five principles, in priority order. When they conflict, the earlier one wins.

1. **Interactivity is a correctness requirement, not an optimization.** A frame must reach the screen within the interactivity budget (§9.2) regardless of scene complexity. A renderer that produces a beautiful image in 400 ms per frame has failed at the product's core promise; one that produces a noisy-but-converging image in 16 ms has succeeded.
2. **Physical correctness by default.** Energy conservation, reciprocity, and correct units are not negotiable. Every shortcut is opt-in, named, and visible in the UI.
3. **One implementation of the shading math.** CPU and GPU share source (§10). Divergence between backends is a bug.
4. **Determinism.** Same inputs, same seed, same output, bit-for-bit, on a given backend (§4.6).
5. **Progressive refinement, never restart-from-scratch when avoidable.** The dirty-state taxonomy (§9.3) exists to preserve accumulated samples wherever the edit did not invalidate them.

---

## 2. The transport problem

### 2.1 Rendering equation

Outgoing radiance at surface point **x** in direction **ω₀**:

```
L_o(x, ω_o) = L_e(x, ω_o) + ∫_{Ω} f_s(x, ω_i, ω_o) · L_i(x, ω_i) · |cos θ_i| dω_i
```

where `f_s` is the BSDF, `Ω` the sphere of directions (not the hemisphere — transmissive lobes exist), and `θ_i` the angle between `ω_i` and the shading normal.

### 2.2 Path integral formulation

The estimator we actually implement is the path-integral form [Veach 1997]: radiance reaching the sensor is an integral over the space of light-transport paths, and each sample is one path constructed incrementally from the camera. This framing is what makes multiple importance sampling (§3.2) and the AOV decomposition (§12) principled rather than ad hoc.

### 2.3 What "unbiased" buys us and where we abandon it

Unbiased estimators converge to the correct answer as sample count → ∞, and their error is pure variance (noise), which denoisers handle well and which the user perceives as "not done yet" rather than "wrong". Biased techniques trade this for speed and introduce error the user cannot resolve by waiting.

Tequila is unbiased by default. Biased techniques are permitted only where they are:
- explicitly user-enabled (Caustics via SPPM §6.5; GI Cache §11.3), **or**
- imperceptible and bounded (path regularization above bounce 2, §3.5; roughness clamping).

Every biased path must be listed in the render metadata sidecar so an image can be audited later.

---

## 3. The integrator

### 3.1 Algorithm

Unidirectional path tracing from the camera with next-event estimation (NEE) and multiple importance sampling (MIS). This is the workhorse; it handles everything except specular-diffuse-specular (SDS) caustic paths, which §6.5 addresses separately.

```
trace_path(ray, sampler, scene):
    L          = 0            # accumulated radiance
    throughput = 1            # path throughput β
    bounce     = 0
    prev_pdf_solid_angle = ∞   # ∞ ⇒ delta/camera ray: no MIS weight on first hit
    specular_chain = true      # true while every vertex so far was specular
    eta_scale  = 1             # accumulated relative IOR², for RR compensation

    loop:
        hit = intersect(scene, ray)

        # ---- Escaped: environment contribution -----------------------------
        if not hit:
            Le = environment_radiance(ray.direction)
            if Le > 0:
                if specular_chain or bounce == 0:
                    w = 1                                   # no light-sampling counterpart
                else:
                    pdf_light = env_pdf(ray.direction) * light_selection_pdf(ENV)
                    w = mis_power(prev_pdf_solid_angle, pdf_light)
                L += throughput * Le * w
            break

        # ---- Emissive surface hit -----------------------------------------
        if hit.material.is_emissive and dot(hit.ng, -ray.direction) > 0:
            Le = hit.material.emission
            if specular_chain or bounce == 0:
                w = 1
            else:
                pdf_light = light_pdf_solid_angle(hit, ray.origin) * light_selection_pdf(hit.light)
                w = mis_power(prev_pdf_solid_angle, pdf_light)
            L += throughput * Le * w

        if bounce >= max_bounces: break

        # ---- Volume interaction (if inside a medium) ------------------------
        if inside_medium:
            (interacted, L, throughput, ray) = medium_interaction(...)   # §6
            if interacted: continue

        bsdf = evaluate_bsdf_at(hit)          # §02-MATERIAL-SYSTEM, OpenPBR lobes

        # ---- Next event estimation: sample a light ------------------------
        if not bsdf.is_pure_delta:
            (light, p_select)  = select_light(sampler, hit)                    # §5.3
            ls                 = sample_light(light, hit, sampler)             # §5.1/5.2
            if ls.pdf > 0 and not occluded(hit.p, ls.p):
                f          = bsdf.eval(wo, ls.wi) * abs(dot(ls.wi, hit.ns))
                pdf_light  = ls.pdf * p_select
                pdf_bsdf   = bsdf.pdf(wo, ls.wi)
                w          = mis_power(pdf_light, pdf_bsdf)
                L += throughput * f * ls.radiance * w / pdf_light

        # ---- BSDF sampling: extend the path -------------------------------
        bs = bsdf.sample(wo, sampler)                     # returns wi, f, pdf, is_delta, lobe
        if bs.pdf <= 0 or bs.f == 0: break

        throughput *= bs.f * abs(dot(bs.wi, hit.ns)) / bs.pdf
        if bs.is_transmission: eta_scale *= bs.eta * bs.eta

        prev_pdf_solid_angle = bs.is_delta ? ∞ : bs.pdf
        specular_chain       = specular_chain and bs.is_delta
        ray                  = spawn_ray(hit, bs.wi)
        bounce += 1

        # ---- Path regularization (§3.5) ------------------------------------
        if bounce >= regularization_start_bounce:
            bsdf_roughness_floor = max(bsdf_roughness_floor, regularization_alpha)

        # ---- Russian roulette (§3.4) ---------------------------------------
        if bounce >= rr_start_bounce:
            q = clamp(max_component(throughput) * eta_scale, 0, 0.95)
            if sampler.next_1d() >= q: break
            throughput /= q

    return L
```

Implementation notes that matter:

- **Shading vs geometric normals.** Use the shading normal `ns` for BSDF evaluation, the geometric normal `ng` for ray offsetting and for rejecting light samples on the wrong side. Guard against the classic shading-normal energy leak (light arriving below the geometric horizon but above the shading horizon) by zeroing contributions where `sign(dot(wi, ng)) != sign(dot(wi, ns))` for reflection lobes.
- **Ray offsetting.** Do not use a fixed epsilon. Use the robust offset of [Wächter & Binder 2019] — integer-based offset scaled to the magnitude of the intersection point. Fixed epsilons fail at both large scene scale (light leaks) and small (self-intersection acne), and product CAD scenes span both.
- **`eta_scale` in Russian roulette** compensates for the radiance compression that occurs entering a denser medium; omitting it makes glass unnecessarily noisy.

### 3.2 Multiple importance sampling

Combine light sampling and BSDF sampling with the **power heuristic**, exponent β = 2 [Veach & Guibas 1995]:

```
mis_power(pdf_a, pdf_b):
    if pdf_a == ∞: return 1        # delta distribution: no counterpart strategy
    a = pdf_a * pdf_a
    b = pdf_b * pdf_b
    return a / (a + b)
```

Both PDFs must be expressed in the **same measure** — solid angle at the shading point. Converting a light's area-measure PDF to solid angle:

```
pdf_solid_angle = pdf_area * distance² / |cos θ_light|
```

MIS correctness is the single most common source of subtle brightness errors in a path tracer. §05 `tests/analytic` includes a dedicated MIS-weight validation test comparing NEE-only, BSDF-only, and MIS renders of a scene with lights across a wide roughness range; all three must converge to the same result within tolerance.

### 3.3 BSDF interface contract

Every material lobe implements exactly this interface. Nothing else in the renderer may know what kind of material it is dealing with.

```rust
trait Bsdf {
    /// f(wo, wi) — value of the BSDF. Excludes the cosine term.
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum;

    /// PDF of sampling `wi` given `wo`, in solid angle measure. 0 for delta lobes.
    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32;

    /// Importance-sample an incident direction.
    fn sample(&self, wo: Vec3, u: Sample2D, uc: f32) -> Option<BsdfSample>;

    /// Hemispherical-directional reflectance, used for RR, denoiser albedo AOV,
    /// and light-selection heuristics. May be approximate but must be ≤ 1.
    fn albedo(&self, wo: Vec3) -> Spectrum;

    fn flags(&self) -> BsdfFlags;  // REFLECTION | TRANSMISSION | DELTA | GLOSSY | DIFFUSE
}
```

`eval` and `pdf` must be **mutually consistent**: the χ² test in §05 §3.2 samples `sample()` and compares the empirical distribution against `pdf()`. Any mismatch fails the build. This test catches the majority of shading bugs before they reach an image.

### 3.4 Russian roulette

Start at bounce 3 (`rr_start_bounce = 3`; earlier termination adds variance at little speed gain in typical product scenes). Survival probability is the maximum throughput component, scaled by `eta_scale`, clamped to `[0, 0.95]`. Never apply RR before at least one indirect bounce has been taken, or interiors go noisy for no benefit.

### 3.5 Firefly control and path regularization

Fireflies (isolated bright pixels from low-probability high-energy paths) are the number-one perceived quality problem in a progressive renderer, because they persist visually far longer than uniform noise and they break denoisers.

Three-layer defense, in this order:

1. **Path regularization** [Kaplanyan & Dachsbacher 2013]: after bounce `regularization_start_bounce` (default 2), impose a roughness floor on subsequent BSDF lobes: `α = max(α, 0.09)` for glossy lobes; convert perfect-specular lobes to near-specular. This is a small, bounded bias that eliminates most SDS-adjacent fireflies. Expose as a user-visible toggle inside Custom lighting settings; on by default.
2. **Adaptive outlier rejection in accumulation**: maintain per-pixel running mean and variance; reject a sample whose luminance exceeds `mean + k·σ` with `k = 8` **only when the pixel already has ≥ 16 samples**, and log the rejected energy into a separate "clamped energy" AOV so the bias is auditable. Never reject early — you will clip legitimate highlights.
3. **Explicit indirect clamping** (user-facing, default off): hard clamp on indirect radiance. Provided because some users want it; documented as biased.

Do not implement only #3. Clamping alone darkens caustics and specular highlights, which is exactly the look product renderers need.

---

## 4. Sampling

### 4.1 Sampler

**Owen-scrambled Sobol′ sequence** with the hash-based scrambling of [Burley 2020]. This is the right default: better convergence than stratified or Halton for the dimensions a path tracer uses, cheap to evaluate, progressive (any prefix is a valid sequence), and trivially parallel with no shared state.

```
sample_dimension(sample_index, dimension, seed):
    idx = owen_scramble(reverse_bits(sample_index), hash(seed, dimension))
    return sobol(idx, dimension) scrambled by hash(seed, dimension, 1)
```

### 4.2 Dimension allocation

Assign dimensions **statically and consistently** — dynamic dimension allocation destroys the low-discrepancy property that justifies choosing Sobol′ in the first place.

| Dimensions | Use |
|---|---|
| 0–1 | Pixel jitter (filter importance sampling) |
| 2–3 | Lens sample (depth of field) |
| 4 | Time (motion blur shutter) |
| 5 | Wavelength (spectral mode, §7.4) |
| 6 + 6·b + 0 | Light selection at bounce *b* |
| 6 + 6·b + 1,2 | Light surface/direction sample |
| 6 + 6·b + 3 | BSDF lobe selection |
| 6 + 6·b + 4,5 | BSDF direction sample |
| beyond ~64 | Fall back to a hashed uniform RNG (PCG32); the low-discrepancy benefit is negligible that deep |

### 4.3 Screen-space decorrelation

Decorrelate neighbouring pixels by seeding the scrambler with a per-pixel hash. Additionally apply **blue-noise dithered sample offsets** [Heitz & Belcour 2019] for the first two bounces: this does not reduce per-pixel error but redistributes it into a spatial frequency the eye and the denoiser both handle far better. At low sample counts — which is exactly the interactive case — this is a large perceived-quality win for negligible cost.

### 4.4 Adaptive sampling

After a warm-up of `N₀ = 16` samples per pixel, compute per-pixel relative error:

```
error(p) = σ(p) / (sqrt(n(p)) · max(mean_luminance(p), ε))
```

Pixels below the error threshold are retired; the scheduler reallocates their budget to active pixels. Retired pixels are re-activated if a neighbouring pixel's variance rises sharply (prevents premature halting at edges).

Expose the threshold as the "Anti-Aliasing / Quality" control. Track and display the **percentage of pixels converged** in the HUD — a far more meaningful progress signal than sample count, and something KeyShot does not offer.

### 4.5 Pixel filter

Filter importance sampling: sample the pixel-jitter offset from the filter kernel rather than weighting samples afterwards. This keeps the accumulation buffer a simple running mean and makes adaptive sampling and denoising well-defined.

Kernels: Box, Triangle, **Gaussian (default, σ = 0.5)**, Blackman-Harris, Mitchell-Netravali. The "Pixel Filter Size" control (range 1–3, matching KeyShot's) scales the kernel radius.

### 4.6 Determinism

The sample value at `(pixel, sample_index, dimension)` is a pure function of `(pixel, sample_index, dimension, scene_seed)`. It must never depend on thread scheduling, tile order, or arrival order.

Consequence: accumulation must be **order-independent**. Use per-tile deterministic accumulation into a fixed-layout buffer, and sum in a fixed order when combining. Do not use atomic float adds into a shared buffer for the beauty pass — atomic float addition is not associative and will silently break determinism. (Atomics are acceptable for AOVs where exactness is not tested, but prefer consistency.)

CI asserts: rendering the same scene twice at the same sample count produces bit-identical output (§05 §3.6).

---

## 5. Light sampling

### 5.1 Analytic light types

| Type | Sampling strategy | PDF |
|---|---|---|
| **Area (rect/disc)** | Solid-angle sampling of the spherical rectangle [Urena 2013] / spherical cap for discs. Falls back to area sampling when the shading point is very close or coplanar. | Solid angle directly; no `d²/cos` conversion needed, which is why this is worth the complexity — it is dramatically lower variance than area sampling for large nearby lights, the common product-lighting case. |
| **Point** | Delta. Single direction; no MIS partner. | Delta (∞); MIS weight 1 on the light-sampling side, and BSDF sampling can never hit it. |
| **Spot** | Delta direction with cone falloff. Smooth-step penumbra between inner and outer cone angles. | Delta. |
| **IES** | Point or area light modulated by an imported photometric distribution. Parse IESNA LM-63 (1986/1991/1995/2002 profiles). Build a 2D CDF over the goniometric grid and importance-sample it. | 2D CDF PDF × delta position, or × area PDF for an area-IES hybrid. |
| **Emissive geometry** | Uniform area sampling of triangles, chosen through a light BVH of their own (§5.3) weighted by triangle area × emission luminance. A glow the same all over its part only; one a graph drives is found by BSDF sampling alone. | Area → solid angle conversion. |
| **Environment** | §5.2. | §5.2. |

All lights carry: colour (spectrum or RGB), intensity with **physical units** (§7.5), a visibility flag set (camera / diffuse / specular / shadow), and a **light layer ID** (§5.5).

### 5.2 Environment importance sampling

An HDRI with a small bright sun is a high-variance light source unless importance-sampled. This is mandatory, not an optimization; without it every outdoor scene is unusable.

Build, at environment load time (and rebuild on rotation only if using a precomputed world-space table — prefer sampling in local space and transforming, so rotation is free):

1. Compute the luminance image `Y(u,v)` of the equirectangular map.
2. Weight each row by `sin θ` to account for the solid-angle distortion of the equirectangular parameterization. **Forgetting this is the single most common bug in environment sampling** and produces a subtle over-weighting of the poles.
3. Build a marginal CDF over rows and a conditional CDF per row (or, preferably, a **hierarchical warping** structure [Clarberg 2005] which has better cache behaviour and supports the alias method).
4. `pdf_solid_angle(ω) = pdf_image(u,v) / (2π² · sin θ)`.

Additional requirements:

- **Portal sampling** for interior scenes: when the Interior lighting preset is active and the scene has window-like openings, allow the user (or an automatic detector) to designate portals; sample the environment through the portal's solid angle. Without this, interiors lit through windows are pathologically noisy. This is what "Interior Mode" in the Lighting tab (§11) actually means in implementation terms.
- **Separate visible-background lookup**: the environment seen directly by camera rays must be sampled at full resolution with no MIP filtering, while the lighting lookup may use a filtered/blurred version. This is how you get sharp background detail and low-noise lighting simultaneously.
- **HDRI Editor pins** composite into the environment as an ordered layer stack evaluated into the same equirectangular texture, after which the CDF is rebuilt. Pin edits therefore require a CDF rebuild — budget ~10 ms for a 4K environment and do it asynchronously (§9.4).

### 5.3 Many-light sampling

A scene with a hundred emissive parts (LEDs, screens, light strips) must not cost 100× a single-light scene.

Implement a **light BVH with importance-based traversal** [Conty Estevez & Kulla 2018]:
- Build a BVH over all analytic and emissive-geometry lights, storing per-node bounding cone of emission directions and total power.
- At a shading point, traverse stochastically, choosing a child with probability proportional to an importance estimate (power / distance², modulated by the orientation cone and the shading point's normal).
- Return the selected light and the product of traversal probabilities as `p_select`.

For scenes below ~8 lights, use a simple **power-weighted alias table** — it's cheaper and the BVH's benefit is nil. Choose the strategy at scene-build time based on light count and log which was chosen.

As built, emissive triangles have a tree of their own (`emitters.rs`), sampled beside the analytic lights with a shadow ray of its own, always as a tree: two parts millimetres apart see each other's near side a hundred times brighter than the far side, and a power-weighted table picks the near triangles no more often. It is a separate tree because an emitter takes MIS and an analytic light does not: a BSDF ray that hits a glow needs its triangle's `p_select` from the vertex it left, which the tree gives by walking up from the triangle's leaf, and a lamp's can never be asked for.

### 5.4 Light visibility flags

Per light: visible to camera, visible to diffuse, visible to specular/reflections, casts shadows. These enable the studio-photography workflows this application exists for (a big soft key light that appears in reflections but is not itself visible, a fill light that doesn't cast shadows).

### 5.5 Light layers and light path expressions

Assign each light a **layer ID**. During tracing, tag each NEE contribution with the layer of the sampled light and accumulate into per-layer AOVs. This gives:
- Per-light-group AOVs for compositing.
- **Post-render relighting**: adjust the intensity/colour of a light group after rendering by rescaling its AOV. A genuine advantage over KeyShot for look-dev iteration.

Implement the light-path-expression machinery generally enough that the standard AOVs (§12) fall out of the same mechanism: track the path's event string (`C`, `D`, `S`, `L`) and route contributions to AOVs by matching LPE patterns [Heckbert 1990 notation].

---

## 6. Participating media, subsurface, and caustics

### 6.1 Medium representation

A medium has: absorption coefficient `σ_a`, scattering coefficient `σ_s` (both spectral, per scene-unit length), phase function, and optional density grid for heterogeneous media. Derived: `σ_t = σ_a + σ_s`, single-scattering albedo `ρ = σ_s / σ_t`.

Media attach to materials (interior of a Glass Solid, Liquid, Gem, Scattering Medium) and are tracked with a **medium stack** as rays cross interfaces. Maintain the stack by geometric normal orientation, and make it robust to overlapping geometry (common in imported CAD): store `(priority, medium_id)` pairs and let the highest-priority medium win in an overlap — this solves the "glass bottle containing liquid where the surfaces coincide" problem, which every product renderer hits immediately.

### 6.2 Homogeneous media

Distance sampling: `t = -ln(1 - ξ) / σ_t` (spectral: sample a channel by throughput-weighted MIS, then evaluate all channels — this avoids colour noise). Transmittance `T = exp(-σ_t · t)`.

### 6.3 Heterogeneous media

**Delta tracking** for interaction sampling, **ratio tracking** for transmittance estimation (unbiased and lower variance than delta-tracking transmittance) [Novák et al. 2018]. Requires a majorant `σ̄`; use a coarse macro-grid of local majorants rather than a single global one — a global majorant is catastrophically slow for sparse media.

Phase function: **Henyey-Greenstein** with anisotropy `g ∈ (-1, 1)`, importance-sampled analytically.

### 6.4 Subsurface scattering

Use **random-walk (brute-force) subsurface scattering** [Chiang et al. 2016]: enter the surface, perform a volumetric random walk in a dense scattering medium parameterized by user-facing *mean free path* and *subsurface colour*, exit and refract out.

Why not a BSSRDF/dipole: random walk is ground truth, handles thin geometry, arbitrary shapes, and heterogeneous interiors correctly, and it composes naturally with the medium machinery you already need for §6.1–6.3. It is slower, but denoising and the progressive architecture absorb that cost, and it removes an entire category of artifacts. This is one of the places where being newer than KeyShot's lineage is a straightforward advantage.

Parameter conversion: users specify `subsurface_color` and `subsurface_radius` (mean free path per channel, in scene units). Convert to `σ_s`, `σ_a` with the searchlight-configuration fit of [Christensen & Burley 2015] so that the rendered result matches the specified albedo.

### 6.5 Caustics

Path tracing with NEE cannot sample specular-diffuse-specular (SDS) paths: light through a glass onto a table, seen directly, or a gem's internal focusing. BSDF sampling alone finds them with very low probability; this is the classic caustic-noise problem, and it is exactly why KeyShot has a distinct Caustics toggle with a quality slider up to 50.

Tequila's implementation, matching that UI:

- **Caustics off (default in Basic/Product presets):** pure path tracing. Caustics appear only where they are easy (rough transmitters); glossy-focused light is present but noisy paths are regularized away (§3.5).
- *As built, October 31* (`tequila-render-core/src/integrator.rs`: `left_to_bounces`, `sees_glass`, `SeenFrom`; `IntegratorConfig::light_through_glass`): with caustics off, clear glass passes shadow rays (`ShadowGlass::Clear`: straight through, tinted, less what its faces reflect, `Pane::crossing`), and the bounces off a surface whose light sample went through it pass it too - the diffuse ones, and the glossy ones off a reflection rougher than 0.8 - weighed against that sample as in the open. A bounce off a reflection the coat, the dielectric interface or a metal gives (told from the sampled lobe, `OpenPbrBsdf::sample_reflection`), up to a roughness of 0.5 and fading out by 0.8, sees glass that bends light as glass: it passes only thin walls, meets the rest mirror-smooth and sampling no light inside it (roughened by the floor, the glass sampled the lamps past it as well as the bounce meeting them, twice the light), and meets what is past it along the bent path - the sky, a glowing part, and the analytic panels and suns with a disc, which only such a bounce looks for (`Light::met_along`; point, spot and IES lights have nothing to meet and stay their light samples' alone). The light a light sample finds through such glass is shared out so that each direction's is counted once: the lobes that pass the glass take the share their density makes up of the surface's, straight through, weighed against the sample by that density; the rest is shared between the sample's straight line and the bounce's bent path by MIS, the seeing lobes' density against the light's along the way the bounce left, the bounce taking all of it where that straight line is blocked. Through a thin or flat pane the two find nearly the same light and each makes up for the other where it is weak; through a bottle the straight line seldom meets what the bent path meets, and where it does the sample keeps its weight of the light straight through - through a 30° prism with the lamp a metre past it, 2% more than the bent path alone at roughness 0.04, 7% at 0.1, 11% at 0.15. Cycles counts about as much on top of the bent path's light, where this shares it out. Held by `tequila-render-cpu/tests/glossy_reflection_of_glass.rs`: a silver floor of roughness 0.04 shows a lamp through a prism as a mirror shows a glowing panel there (2.40 against 2.41; before, 0.94, in specks, noise 10 against 0.14 now), and a lamp and a glowing panel show alike through the prism and through a flat slab at roughness 0.04 to 0.6 (within 1%, the lamp no noisier; through the slab at 0.04 the lamp had been specks, noise 5.4 against 0.16 now, and through the prism at 0.45 half as bright, in specks); and by the `app` test `a_glass_ball_in_a_glossy_floor_shows_the_softbox_behind_it_in_both_renderers`: a glass ball on a dark glossy floor with a softbox behind it, whose reflection shows the softbox through the ball at 147 and 203 of 255 (roughness 0.04 and 0.1, 640 x 420, 256 samples) against Cycles' 153 and 202, where it had been 38 and 157. In `glass_reflection_pictures` the white ball under the bell is unchanged at 153 (speckle 4.7 on the ball, 5.7 on the puck, 3.0 on the floor; Cycles' 156, 4.0, 3.9, 1.6), and so is the light under the parity slab, 0.950 of the light without it (Cycles' 0.988). The perfume flacon's reflection hardly changes (40 and 41 of 255 at the two roughnesses, Cycles' 40 and 42): few of the floor's bounces through it meet the softbox, and what still differs from Cycles is in its scent, not its floor - Cycles has no medium stack, and the scent's surfaces bend light there as if against air; without the scent the two agree (73 against 74). Costs: one more density at each bounce in a scene with clear glass, and a straight probe where a bounce that saw glass meets a light its light sample could have found. Not done: a lamp seen in a mirror-like surface with no glass between is still found by light samples alone, as specks; and on a path the camera sees along (through mirrors and glass only) a lamp past glass shows by its stand-in or not at all, as before.
- **Caustics on (Jewelry preset / user toggle):** run **Stochastic Progressive Photon Mapping (SPPM)** [Hachisuka & Jensen 2009] as a *supplementary* pass, merged with the path-traced result via the caustic-only light path expression `L S+ D` (light → one or more specular → diffuse → camera). The path tracer handles everything else; SPPM handles only the paths it cannot. This hybrid keeps the main image unbiased and confines SPPM's bias to the caustic component.
  - "Caustics Quality" (1–50) maps to photons emitted per progressive pass: `photons = 100_000 × quality`.
  - Progressive radius reduction with α = 0.7.
- *As built, September 27* (`tequila-render-core/src/integrator/caustics.rs`): photons are traced per progressive sample rather than per pass of many, in Knaus and Zwicker's independent-pass form of progressive photon mapping — each sample reads a fresh set with a radius that shrinks as `(k + 1)^((α − 1)/2)` from 3% of the mirror-smooth objects' bounding radius, and the framebuffer's running mean converges. 100 000 photons a sample (Caustics Quality 1; there is no quality control yet). "Specular" means a mirror-smooth BSDF (every lobe a delta): photons pass through such surfaces and land at the first other one, from the environment (sun and sky included) and the analytic lights, aimed at the smooth objects' bounding sphere; emissive geometry sends none, and its light stays the path tracer's. The camera path reads them at its first surface that is not mirror-smooth, and from there drops light reached through mirror-smooth surfaces alone; glass past that point is not roughened by §3.5's floor, which would otherwise count the same light twice. On a shadow catcher the caustic brightens the backdrop in proportion to the floor's open-sky irradiance. Held by `tequila-render-cpu/tests/caustics.rs`: under a uniform sky the image matches a 2048-sample plain render (nothing lost, nothing counted twice), a lamp through a glass ball makes an even spot, and the photons are the same on any number of threads. Caustic light is routed to the environment's light group.
- **Alternative worth evaluating at M9:** Manifold Next Event Estimation [Hanika et al. 2015] gives unbiased caustics and integrates into the path tracer without a separate photon pass, at the cost of considerably more complex code. Prefer SPPM first; MNEE is a later refinement, not a starting point.
- *As built, October 31* (`tequila-render-core/src/integrator/manifold.rs`, Caustics: Manifold, `IntegratorConfig::caustics_method`): Manifold Next Event Estimation as a second way of finding caustics, beside the photons and chosen in their place (Off / Photons / Manifold). At every surface the camera sees (through mirrors and glass included) that samples the lights, and past a bounce at one such vertex in four, counted four times, the lamp and the point on it next event estimation draws are taken again; the straight line between them seeds a path through the glass in the way, and Hanika, Droske and Fascione's Newton walk on the specular manifold - ported from Cycles' `mnee.h` - moves each crossing along its mesh until the generalized half vector at every one lies along the normal. Its light is the lamp's through each crossing's Fresnel share and tint, absorbed along the legs inside, times the generalized geometric term (the first crossing's area seen from the surface, per unit of the lamp's area or of a sun's solid angle, from the walk's own derivatives). Area, point, spot, IES and sun lamps; the shadow catcher takes the caustic as the photons' does, by its share of the open-sky irradiance. Each crossing's indices come from the media the seed passes (a liquid in glass bends by 1.33 to 1.5), not from the winding. What it counts and what it leaves: shadow rays stop at glass that bends light and pass only thin walls (`ShadowGlass::Unbent`), so a lamp's light through such glass is the manifold sample's alone - no bounce can meet a lamp, so nothing else could find it, and it is weighed against nothing; the environment and glowing parts, which bounces can meet, reach through it by bounces as they always did, weighed by MIS as before (environment MNEE would need the chance of a bounce landing on the solved path; Cycles culls those bounces instead, a bias). Past a manifold sample, glass that bends light samples no lamp: roughened by §3.5's floor it counted the lamp a second time, as specks. Each projection back onto the mesh is aimed from the ray's offset origin: aimed from the point it was offset from, it landed a fixed 0.015 mm beside it near the world's origin, and the walk stalled there (0.6% of the flacon's light found). Its limits: one path per lamp point, the one the straight line's crossings lead to - where two or more paths join the same two points (past a focus, a ball's brightest spot) the others are missed and the caustic there is darker than the truth, as in Cycles; refraction only (no internal reflections, no mirror caustics); mirror-smooth glass on meshes with vertex normals only (frosted glass stays solid to a lamp); dispersion is left out of the solved path (the base index). Held by `tequila-render-cpu/tests/manifold_caustics.rs`: a point light and the sun through a slab, and a point light through water in glass, match their closed forms to four digits; an area lamp through a lens matches brute-force path tracing of a glowing panel within 1.3%; behind a perfume flacon (glass, scent, scent, glass) it finds 45% of the light path tracing does, the rest along paths the straight-line seed does not lead to; nothing changes under a sky or through a thin pane; and a caustic shows on a shadow catcher. The box test is held by `a_segment_meets_a_box_only_where_it_passes_through_it`. What it costs, profiled in a studio of a glass ball and the flacon under a small round lamp and a softbox: the manifold samples took 37% of the render's thread time, and walks that failed 62% of that - most stay far from Snell's law (the worst crossing's half vector over 0.5 along its surface) until their step shrinks below 1e-4, some 33 steps, where walks that land take 6 to 10; seeds whose line met no glass took 6%, samples past a bounce 1% of the calls. So a box test against each bending part's bounds (`SceneSnapshot::bending_bounds`) now answers for the seed ray where the line passes no such glass (4% more samples a second), and every ten steps a walk still that far goes on only half the time and counts twice if it lands, unbiased (23% more; a quarter's chance gave 31% but more specks). Solving for every lamp at each sample, rather than the one next event estimation draws, cost 1.6 times the time for no less error and was not kept: the ball's caustic is the round lamp's, drawn 85% of the time already, and the flacon's is the softbox's, which the seed misses whichever lamp is drawn. Measured fairly (`tequila-render-cpu/tests/manifold_noise.rs`, 320 x 210, the three ways rendered in turns for 10 s each on a shared CPU, against brute-force path tracing of the same studio with its lamps as glowing geometry, no roughness floor, 8192 samples over the picture and 131072 in the two caustics, whose own noise is taken out): the photons took 170 samples, manifold caustics 106. RMS as shown (0 to 255; whole picture / the ball's caustic / the flacon's): photons 9.5 / 3.5 / 6.4, manifold 9.9 / 8.6 / 24.0, caustics off 32.7 / 66.2 / 31.6; relative RMS of the linear luminance, photons 1.31 / 1.26 / 0.10, manifold 1.54 / 3.15 / 0.66. The photons' mean is within 1% of the reference in both caustics; manifold caustics' is 14% short in the ball's and 27% in the flacon's. At 30 s the photons are at 2.4 and 5.8 in the two caustics, and manifold caustics at 60 s still at 8.1 and 23.1: what is left is light the method never finds, not noise - the flacon's two broad lobes, an arc at the edge of the ball's, the softbox's light through the flacon into the ball's shadow - where the straight line to the lamp does not cross the glass the way the light does. So in the studio the photons win at equal time, and keep winning however long it renders; manifold caustics are the sharper only where they find the path, at the ball's bright spot. Cycles' own MNEE there: blotches in the ball's caustic and nothing through the flacon. Cycles: the same switch gives Cycles its own MNEE (objects of glass that bends light cast, every other receives, every lamp casts), and with caustics off its glass passes shadow rays as Tequila's does (`Builder::shadows_through` in `tequila-render-cycles/src/translate.rs`): a Transparent BSDF as clear as `Pane::crossing`, to shadow rays and to diffuse bounces - Cycles' lamps can be hit, and a diffuse bounce refracted into one counted beside the shadow ray that had passed the glass, twice the light in specks; passed straight, it is weighed against the shadow ray by MIS. Glossy bounces still see the glass: passed too, a perfume bottle on a glossy floor (roughness 0.04 and 0.1) was mirrored in it as a clear ghost, without its glass or its scent (`app` test `glass_reflection_pictures`). What a glossy bounce finds of a lamp through the glass is counted on top of the shadow ray's, small beside the glossy lobe's own: the white ball under the glass bell is 1% brighter than with every bounce passed (156 against 155 of 255, Tequila's 153) and as smooth (speckle 4.0 against 3.8), the demo's plastic at roughness 0.2 under a slab 5%; passing shadow rays alone counted the light twice under glass (the ball at 219, 1.9 times the light under the slab, in specks). Cycles' MNEE differs from the port: under a slab lit from the side it finds no path in bands (a quarter of the light under the slab gone, measured) - its walk gives up as soon as any one crossing stops moving, where the port's waits for all of them, which is likely why - and it clamps the geometric term at 2.

---

## 7. Colour and spectral pipeline

### 7.1 Working space

All internal computation is **scene-linear**, in a wide-gamut working space. Use **ACEScg (AP1 primaries)** as the rendering space. Never store or compute on sRGB-encoded values inside the renderer.

### 7.2 Input transforms

Every texture declares its colour space at load:

| Map type | Colour space |
|---|---|
| Base colour, emission colour, specular colour, label colour | sRGB texture (or the file's embedded profile) → ACEScg |
| Roughness, metalness, opacity, bump, displacement, masks | **Raw / linear, no transform** |
| Normal maps | Raw, and remapped `[0,1] → [-1,1]` |
| HDR environments (.hdr/.exr) | Already linear; apply primaries conversion only |

Mis-tagging a roughness map as sRGB is a common and destructive error. The importer must default data maps to Raw and require an explicit override, not the reverse.

### 7.3 Output transforms

Pipeline: `accumulated scene-linear → Image Style (exposure/tone/grade) → OCIO display transform → 8/16/32-bit encode`.

Display transforms available: **ACES 1.3 sRGB**, **AgX**, **Filmic**, **Khronos PBR Neutral**, plain sRGB (for technical/QA work), and Raw. Default: AgX or ACES — both handle the extreme highlight ranges of studio lighting gracefully, which naive sRGB clipping does not. This is a large part of why a physically correct render can still look wrong; the tone mapping is not cosmetic.

32-bit EXR outputs are written **before** the display transform (scene-linear), with the display transform recorded as metadata. 8-bit outputs are written after.

### 7.4 Spectral rendering (optional, on by default for dispersive materials)

RGB rendering cannot produce correct dispersion (prism rainbows, gem fire) or correct thin-film interference. Implement **Hero Wavelength Spectral Sampling** [Wilkie et al. 2014]: carry 4 wavelengths per path (one hero, three stratified companions), with MIS across wavelengths to suppress colour noise.

Enable spectral automatically when a path encounters a material with `dispersion > 0` or a thin-film lobe; otherwise render in RGB for speed. Upsample RGB reflectance to spectra with [Jakob & Hanika 2019], which guarantees the round trip preserves the original RGB under the rendering illuminant.

- *As built, October 30* (`tequila-render-core/src/spectral.rs`): decided per render rather than per path, by the render settings' Spectral — Auto, the default, is spectral when any material disperses (an Abbe number on something that transmits), and thin films alone do not turn it on, since their interference is still evaluated at three wavelengths inside the BSDF. The rendering illuminant is D65, the white of the AP1 space `Spectrum` holds, folded into the film's response so a white light is a flat spectrum. Companions are dropped at a dispersive *transmission*; a reflection off the same surface keeps all four. The coefficient table is fitted at first use rather than shipped. Measurements and what remains open: `docs/05` M10.

### 7.5 Light units

Emission is specified in physically meaningful units and converted internally to radiance:

| Light | User-facing unit | Conversion |
|---|---|---|
| Point / spot | lumens (lm) or candela (cd) | `I [W/sr] = Φ / (4π)` for point; cone-solid-angle for spot |
| Area | lm, or **nits (cd/m²)** for screens/displays | divide by area × π |
| Emissive material | nits, or a unitless multiplier | direct radiance |
| Environment | EV offset from the HDRI's absolute scale | multiplicative |
| Sun & sky | physical (Hosek-Wilkie model output in W/m²/sr) | direct |

Because intensity depends on the scene's scale (§03 §3), rescaling a scene with "Scale Scene" **must** rescale light intensities — lumens and candela by the square of the factor, so the shot keeps its brightness; a panel in nits grows with the scene and needs nothing. This is precisely the behavior KeyShot documents, and it is not optional if physical units are to mean anything. Lengths are stored in metres, so changing only the unit they are shown in ("Keep Scene Size") changes nothing.

---

## 8. Camera model

### 8.1 Thin-lens perspective camera

Parameters: `focal_length_mm`, `sensor_width_mm` (default 36 mm full-frame), `f_number`, `focus_distance`, `aperture_blades` (0 = circular; 3–11 = polygonal bokeh), `aperture_rotation`, `shutter_open/close`.

```
fov_vertical  = 2 · atan(sensor_height_mm / (2 · focal_length_mm))
aperture_radius = focal_length_mm / (2 · f_number)   [converted to scene units]
```

Ray generation: sample the film point via the pixel filter (§4.5), sample the lens aperture uniformly (or over the polygon for bladed apertures), and construct the ray through the point of focus. Note that the coupling `f_number ↔ aperture_radius ↔ blur` must be physical: users of this application think in photographic terms and will notice if f/2.8 does not behave like f/2.8.

**Keep Focus** (from KeyShot 2025.3): when enabled, the focus distance is re-derived from the camera-target distance whenever the camera moves.

### 8.2 Other projections

- **Orthographic**: parallel rays; `ortho_scale` replaces focal length. No DOF (or optional, via a tilt-shift-style model).
- **Shift**: perspective with a film-plane offset `(shift_x, shift_y)` in sensor-width fractions. Keeps verticals parallel — required for architectural/packaging work.
- **Panoramic**: equirectangular (spherical), cylindrical, and fisheye (equidistant / equisolid) variants; plus **stereo panoramic** (ODS) for VR output.

### 8.3 Physical exposure

`EV = log₂(N² / t) - log₂(ISO / 100)` where N is f-number and t shutter time. The exposure scale applied to radiance is `2^(-EV) × calibration`. Exposing exposure this way (rather than as a raw multiplier) means the DOF setting and the brightness respond together the way a real camera does — which is what a product photographer expects.

### 8.4 Motion blur

Sample shutter time `t ∈ [shutter_open, shutter_close]` per camera ray (Sobol dimension 4). Transform-level motion blur: interpolate instance transforms (linear for translation, **slerp for rotation, decomposed TRS** — never lerp raw matrices, which produces shearing artifacts on rotating parts). Deformation blur: interpolate between two vertex-position buffers. Embree supports multi-segment motion blur natively; use it rather than rolling your own.

### 8.5 Ray differentials and texture filtering

Track ray differentials [Igehy 1999] from the camera through the first few bounces. Use the footprint to select MIP level and anisotropic filter width. After a diffuse bounce, ray differentials become meaningless; substitute a heuristic footprint based on path roughness and distance (this is what makes distant textured surfaces stop aliasing without over-blurring).

**"Sharper Texture Filtering"** (KeyShot's control) maps to: enable higher-order anisotropic filtering (up to 16×) and reduce the MIP bias by 0.5. Default off (cheaper, slightly softer); on for final renders.

---

## 9. Progressive rendering and interactivity

This section is the difference between "a path tracer" and "KeyShot". Implement it exactly.

### 9.1 Frame lifecycle

```
Scene Document ──[snapshot on mutation]──▶ Immutable SceneSnapshot (Arc)
                                                    │
                              ┌─────────────────────▼──────────────────────┐
                              │           Render Orchestrator               │
                              │  owns: accumulation buffer, sample counter, │
                              │  adaptive-sampling state, tile queue        │
                              └─────────────────────┬──────────────────────┘
                                    dispatch tiles  │  N worker threads / GPU dispatch
                              ┌─────────────────────▼──────────────────────┐
                              │  Accumulate into HDR buffer + AOV buffers   │
                              └─────────────────────┬──────────────────────┘
                                     on a cadence   │
                              ┌─────────────────────▼──────────────────────┐
                              │  Denoise (throttled) → display transform →  │
                              │  upload to GPU texture → UI draws it        │
                              └────────────────────────────────────────────┘
```

The UI thread **never** waits on a render. It draws the most recent completed display texture. If none is ready, it redraws the previous one. Frame pacing is decoupled from sample production.

### 9.2 Interactivity budget

Two regimes, switched by whether the user is actively manipulating the view:

**Interactive (camera moving, slider dragging):**
- Target 30 fps minimum, 60 preferred.
- Render at reduced resolution (start at 1/4 linear resolution, i.e. 1/16 the pixels) with 1 sample per pixel, upsampled for display.
- Denoise every frame at this low sample count (OIDN is fast enough at reduced resolution; the result is a smooth, "final-looking" image while moving — this is precisely the KeyShot feel).
- Skip: caustics pass, adaptive sampling, high MIP-quality filtering.

**Refining (input idle > 150 ms):**
- Step resolution up: 1/4 → 1/2 → full over the first ~300 ms, restarting accumulation at each step (cheap, few samples lost).
- Accumulate at full resolution indefinitely.
- Denoise on a decaying cadence: every frame for the first 8 samples, then every 2, 4, 8, 16… samples. Denoising cost must not dominate sample production.
- Enable adaptive sampling after 16 spp.

**Hard requirement:** first pixels on screen within **100 ms** of any scene change, on the reference benchmark scene (§05 §4). If a change requires a BVH rebuild that exceeds this, render the previous BVH's result with the changed shading until the rebuild lands (§9.4).

### 9.3 Dirty-state taxonomy — what invalidates what

This table is the heart of feeling fast. Naively restarting accumulation on every change makes the app feel worse than KeyShot regardless of raw ray throughput.

| Change | Action | Accumulated samples |
|---|---|---|
| **Image Style / tone map / exposure** | Re-apply display transform to existing buffer | **Kept** — no re-render at all. Instant. |
| Camera transform | Full restart | Discarded |
| Camera lens parameters | Full restart | Discarded |
| Resolution change | Resize buffers, full restart | Discarded |
| Environment rotation / brightness | Full restart (cheap; rebuild env CDF only if pins changed) | Discarded |
| **Material parameter edit** | Region restart: rasterize (or read from the ID AOV) the screen-space mask of all instances using that material; reset only those pixels | **Kept outside the mask** |
| **Material assignment (drag-drop)** | Region restart over the union of the affected instances' screen bounds | Kept elsewhere |
| Label added / edited | Region restart over that instance | Kept elsewhere |
| Light parameter edit | Region restart over the light's influence estimate; full restart if unbounded | Partially kept |
| Light added/removed | Rebuild light BVH, full restart | Discarded |
| Instance transform | Refit/rebuild TLAS, full restart | Discarded |
| Geometry edit (re-tessellate, normals) | Rebuild that BLAS, full restart | Discarded |
| Visibility toggle | Rebuild TLAS (or mask), full restart | Discarded |
| Selection change | Overlay only | **Kept** |
| Model Set / Studio switch | Full restart | Discarded |

The "region restart" mechanism requires a persistent **instance-ID and material-ID buffer** maintained alongside the beauty buffer. You need those buffers for Cryptomatte anyway (§12), so this costs nothing extra. This one optimization is worth more perceived performance than a 2× raw speedup.

### 9.4 Asynchronous rebuilds

BVH builds, environment CDF builds, texture loads, and BREP re-tessellation happen on background threads. The renderer continues using the previous version until the new one is ready, then swaps atomically. Show a subtle HUD indicator while a rebuild is pending (matching KeyShot's in-view notifications) but never freeze.

### 9.5 Region rendering

A user-defined rectangle limits sample production to that region while keeping the rest of the buffer displayed as-is. Implement as a tile-queue filter, not as a separate code path.

### 9.6 Interactive-vs-final consistency

The image the user sees while iterating must be the same image they get when they render, modulo sample count and denoise strength. Any setting that differs between viewport and final render (e.g. reduced-resolution interactive mode) must converge to identical results at rest. A test in §05 §3.9 asserts that a viewport render at N samples and a final render at N samples match within tolerance.

---

## 10. Backend architecture and CPU/GPU parity

> **As built (2026-10-11):** the GPU backend is Cycles, embedded as the
> default engine, rather than a second integrator written here; Tequila's
> materials run Cycles' own algorithms, so one source of shading math holds
> across engines by sharing Cycles' rather than by a Slang port. The
> measured agreement and what stays with Tequila's own renderer are in
> `docs/05` M9. What follows is the design as planned.

### 10.1 The parity problem

If CPU and GPU have separate BSDF implementations, they *will* diverge, and users *will* find the divergence. Solve it structurally.

**Approach:** write all shading code (BSDF lobes, texture evaluation, procedural textures, light evaluation) once in **Slang**, compiled to CPU (via C++/LLVM target) and GPU (SPIR-V/CUDA/HLSL). Slang exists for exactly this problem and supports the generics and modules a shading system needs.

**Acceptable alternative** if Slang proves impractical: define the shading layer as a restricted subset of the core language with no dynamic allocation, no recursion, and explicit vector types, and compile it to GPU via a source-to-source step. Whichever path is chosen, the invariant is: **one source of truth for shading math.**

The *integrator* may legitimately differ between backends (wavefront on GPU, megakernel/recursive on CPU) because the integrator is control flow, not math. The BSDF must not.

### 10.2 CPU backend

Embree 4 for BVH build and traversal. Rayon (or a custom work-stealing pool) over tiles of 16×16 pixels. Prefer coherent packet/stream tracing for camera and shadow rays; incoherent single-ray tracing for indirect bounces.

### 10.3 GPU backend

**Wavefront path tracing** [Laine et al. 2013]: maintain path state in SoA buffers, and dispatch separate kernels per stage (generate → intersect → shade-by-material → NEE → shadow → extend). Sort/compact rays by material between stages to reduce divergence. This is substantially faster than a megakernel for the many-material scenes typical of product visualization, and it makes the material-specific shading kernels manageable in size.

Memory management: textures stream from a host-side cache with an LRU device residency set; on VRAM exhaustion, fall back to CPU (matching KeyShot's documented behavior) and inform the user in the HUD rather than crashing.

### 10.4 Parity verification

CI renders the parity scene set on both backends at fixed seeds and asserts per-pixel agreement within a tolerance that accounts for legitimate floating-point differences (see §05 §3.5): **mean absolute error < 0.002** in scene-linear, **no pixel differing by more than 0.02**, and matching χ² BSDF test results on both.

---

## 11. Lighting presets → integrator configuration

The Lighting tab presets (§1.6 of the master spec) are a UI over the following exact configuration. This table *is* the implementation.

| Setting | Performance | Basic | Product | Interior | Jewelry |
|---|---|---|---|---|---|
| Max ray bounces | 1 | 2 | 6 | 8 | 12 |
| Global illumination (indirect) | off | off | on | on | on |
| Shadows | off | on | on | on | on |
| Light-source materials (emissive geometry) | off | on | on | on | on |
| Ground illumination (ground plane bounces light) | off | off | off | on | on |
| Caustics (SPPM pass) | off | off | off | off | **on** |
| Interior mode (portal sampling + increased NEE weight) | off | off | off | **on** | **on** |
| Path regularization start bounce | 1 | 1 | 2 | 2 | 3 |
| Adaptive sampling threshold | 0.05 | 0.02 | 0.01 | 0.01 | 0.005 |
| Transmission bounce limit | 0 | 2 | 8 | 8 | 16 |

**Custom** exposes every row individually and can be saved as a named preset.

Note the transmission bounce limit is tracked separately from the general bounce limit — glass and gems need many transmission events but few diffuse ones, and conflating them either makes glass black or makes everything slow. KeyShot's Jewelry preset exists precisely because of this distinction.

### 11.1 GI Cache (optional, biased)

KeyShot's "Global Illumination Cache" is an irradiance-caching scheme that trades accuracy for speed and removes low-frequency indirect noise. Tequila's equivalent: an **irradiance cache with radiance-gradient extrapolation** [Křivánek 2005], used only for the diffuse-indirect component, only when enabled, and always disabled for the final render unless the user explicitly opts in. Flag it in metadata as a biased mode.

Recommendation: implement this last, or not at all. With OIDN denoising and adaptive sampling, the practical benefit is much smaller today than it was when the technique was introduced, and it introduces light-leak artifacts that are hard to explain to users.

---

## 12. AOVs and render passes

Implemented via light path expressions (§5.5), so they are consistent by construction rather than separately hand-computed.

| Pass | Definition (LPE where applicable) |
|---|---|
| Beauty | `C.*` — everything |
| Diffuse | `C<RD>.*` — paths whose first bounce is diffuse reflection |
| Lighting (direct) | `C<RD>L` + `C<RS>L` — direct light only |
| Global Illumination | `C.*[DS].*L` minus direct — indirect only |
| Caustics | `C<RD>[SG]+L` — the SDS-family component |
| Reflection | `C<RS>.*` |
| Refraction | `C<TS>.*` |
| Shadow | Ratio of unshadowed to shadowed direct lighting (not an LPE — computed as a separate accumulation of `visibility` during NEE) |
| Ambient Occlusion | Independent short-ray occlusion estimate, configurable radius |
| Raw | Beauty with no Image Style applied (always the EXR case) |
| Depth | Camera-space distance to first hit; store true distance, not normalized |
| Normals | First-hit shading normal, world or camera space (user-selectable) |
| Position | First-hit world position (add this — KeyShot lacks it and compositors want it) |
| Albedo | First-hit BSDF albedo — also feeds the denoiser |
| Clown / ID | Deterministic pseudo-random colour per material or per object |
| Labels | All label layers composited alone with alpha |
| **Cryptomatte** | Per-object, per-material, and per-asset coverage mattes, [Friedman & Jones 2015] standard: 32-bit float ID hashes with coverage, ranked into `CryptoObject00..02` layers. Must be spec-compliant so Nuke/After Effects/Fusion read it directly. |
| Light group AOVs | One per light layer (§5.5) |
| Clamped energy | Energy removed by firefly rejection — for auditing bias |

Alpha modes for render layers: Straight (unmatted, edge colour extended), Premultiplied (matted with black), Aliased (opaque fringe). Implement as a post-process over the layer's coverage buffer.

The main image also has **Silhouette**, a cut along the product's outline, as a photographer's clipping path. The camera's rays are cast against the geometry alone (`tequila_render_cpu::Silhouette`), so it works the same whichever engine rendered the picture. Inside the outline the rendered picture is kept, glass included with the backdrop it shows. At the edge, the background's share is taken out.

**Coverage through cutouts.** A camera ray's coverage, where its first surface is a cutout, is the expected value: one less the light that gets through every surface along the ray, as a shadow ray's transmittance is (`camera_coverage`). It is not the coin flip that decides where the path goes. That made glass held a fifth present by its opacity map cut out as speckle; it now matches Cycles. A shadow catcher behind the cutouts adds its shadow's coverage on the paths that reach it.

---

## 13. Denoising

**OIDN 2.x** with guide buffers. Non-negotiable details:

- Always pass **albedo** and **normal** guide AOVs. Denoising the beauty pass alone destroys texture detail; with guides it preserves it.
- Guide buffers must themselves be **prefiltered** (OIDN provides this) when the sample count is low, otherwise noise in the guides propagates into the result.
- Use the **HDR** mode and pass scene-linear data; do not denoise after tone mapping.
- **Temporal stability for animation:** denoise each frame independently and you get flicker. Either use OIDN's temporal mode with motion vectors, or accumulate enough samples per frame that the residual noise is below the temporal-artifact threshold. Motion vectors require a `Motion` AOV — add it (screen-space velocity from the previous frame's transforms).
- **Denoise strength/blend** control: expose a 0–1 blend between denoised and raw, because at high sample counts users often prefer a small amount of residual grain to the plastic look of full denoising.
- Denoising is applied **for display and for output, never into the accumulation buffer.** Accumulating denoised results would poison convergence.

---

## 14. Acceleration structures and memory

- **Two-level hierarchy**: BLAS per unique mesh, TLAS over instances. Non-negotiable for CAD scenes with thousands of repeated fasteners.
- **Instancing** must survive the whole pipeline: importer detects it (§03 §6.4), scene graph preserves it, renderer instances it.
- **Refit vs rebuild**: refit the TLAS for small transform changes (animation playback), rebuild on topology change. Refit is roughly an order of magnitude cheaper.
- **On-demand tessellation**: BREP geometry tessellates at a screen-space-error-driven tolerance. Re-tessellate when the camera moves closer by more than a factor threshold (hysteresis to avoid thrashing), asynchronously (§9.4).
- **Texture cache**: OIIO's tiled MIP cache with a configurable memory budget. Product scenes with 4K–8K textures on dozens of parts exceed VRAM easily; a cache with tile-level granularity is the difference between working and not.
- **Memory budget telemetry** in the HUD: geometry, textures, BVH, accumulation buffers, shown separately. Users hitting limits need to know which limit.

---

## 15. Performance targets

Measured on the reference benchmark scene (§05 §4) at 1920×1080. These are requirements, asserted in CI, not aspirations.

| Metric | Target (CPU, 16-core) | Target (GPU, mid-range RTX) |
|---|---|---|
| Time to first displayed pixels after a camera move | ≤ 50 ms | ≤ 33 ms |
| Interactive frame rate while navigating | ≥ 30 fps | ≥ 60 fps |
| Samples/sec at full res, simple product scene | ≥ 8 Mspp/s | ≥ 80 Mspp/s |
| Time to visually converged (denoised, indistinguishable from 4096 spp) | ≤ 4 s | ≤ 1 s |
| Material drag-drop → updated preview | ≤ 100 ms | ≤ 50 ms |
| Scene load: 5M-triangle CAD assembly | ≤ 15 s | ≤ 15 s |
| BVH rebuild after transform change, 5M tris | ≤ 200 ms (refit ≤ 20 ms) | ≤ 100 ms |

If a change regresses any of these by more than 10%, CI fails. Performance is a feature here in a way it is not for an offline renderer.

---

## 16. References

- Kajiya, J. (1986). *The Rendering Equation.* SIGGRAPH.
- Veach, E. (1997). *Robust Monte Carlo Methods for Light Transport Simulation.* PhD thesis, Stanford. — path integral formulation, MIS.
- Veach, E. & Guibas, L. (1995). *Optimally Combining Sampling Techniques for Monte Carlo Rendering.* SIGGRAPH. — power heuristic.
- Heitz, E. (2018). *Sampling the GGX Distribution of Visible Normals.* JCGT. — VNDF sampling.
- Heitz, E. (2014). *Understanding the Masking-Shadowing Function in Microfacet-Based BRDFs.* JCGT.
- Burley, B. (2020). *Practical Hash-based Owen Scrambling.* JCGT. — the sampler.
- Heitz, E. & Belcour, L. (2019). *Distributing Monte Carlo Errors as a Blue Noise in Screen Space.* EGSR.
- Clarberg, P. et al. (2005). *Wavelet Importance Sampling.* SIGGRAPH. — environment sampling.
- Urena, C. et al. (2013). *An Area-Preserving Parametrization for Spherical Rectangles.* EGSR. — area light sampling.
- Conty Estevez, A. & Kulla, C. (2018). *Importance Sampling of Many Lights with Adaptive Tree Splitting.* HPG. — light BVH.
- Novák, J. et al. (2018). *Monte Carlo Methods for Volumetric Light Transport Simulation.* Eurographics STAR. — delta/ratio tracking.
- Chiang, M. et al. (2016). *Practical and Controllable Subsurface Scattering for Production Path Tracing.* SIGGRAPH Talks. — random-walk SSS.
- Christensen, P. & Burley, B. (2015). *Approximate Reflectance Profiles for Efficient Subsurface Scattering.* Pixar Tech Memo. — SSS parameter fitting.
- Hachisuka, T. & Jensen, H.W. (2009). *Stochastic Progressive Photon Mapping.* SIGGRAPH Asia. — caustics.
- Hanika, J. et al. (2015). *Manifold Next Event Estimation.* EGSR. — unbiased caustics alternative.
- Kaplanyan, A. & Dachsbacher, C. (2013). *Path Space Regularization for Holistic and Robust Light Transport.* Eurographics.
- Wilkie, A. et al. (2014). *Hero Wavelength Spectral Sampling.* EGSR.
- Jakob, W. & Hanika, J. (2019). *A Low-Dimensional Function Space for Efficient Spectral Upsampling.* Eurographics.
- Igehy, H. (1999). *Tracing Ray Differentials.* SIGGRAPH.
- Wächter, C. & Binder, N. (2019). *A Fast and Robust Method for Avoiding Self-Intersection.* Ray Tracing Gems.
- Laine, S. et al. (2013). *Megakernels Considered Harmful: Wavefront Path Tracing on GPUs.* HPG.
- Křivánek, J. et al. (2005). *Radiance Caching for Efficient Global Illumination Computation.* IEEE TVCG.
- Friedman, J. & Jones, A. (2015). *Fully Automatic ID Mattes with Support for Motion Blur and Transparency.* — Cryptomatte.
- Hosek, L. & Wilkie, A. (2012). *An Analytic Model for Full Spectral Sky-Dome Radiance.* SIGGRAPH. — sun & sky.
- Pharr, Jakob, Humphreys. *Physically Based Rendering: From Theory to Implementation*, 4th ed. — the standing reference for everything above; freely available online at pbr-book.org.
