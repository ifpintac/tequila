# 02 — Material System Engineering Specification

Shading model, the complete material catalog as concrete parameter presets, the material-graph compiler, the texture pipeline, procedural texture algorithms, and label/decal layering.

---

## 1. Design decision: one model, many presets

KeyShot exposes 34 distinct material types, each with its own hand-tuned implementation and its own parameter panel. This is good UX and questionable engineering — it means 34 separate places for energy-conservation bugs and 34 things to port when the renderer changes.

**Tequila implements exactly one surface shading model — OpenPBR Surface — and expresses all 34 material types as presets over it.** Each preset defines default parameter values, a lobe configuration, and (critically) *which subset of parameters the UI exposes*. To the user, "Metal" and "Velvet" look like two different materials with two different panels. Internally they are one BSDF with different weights.

Why OpenPBR specifically:
- It is an open, published, precisely-specified standard (Adobe + Autodesk, now under ASWF), with a reference implementation and a well-defined layering model.
- It is energy-conserving by construction, including the albedo-scaling compensation between layers that is otherwise a persistent source of "why is my coated material too dark" bugs.
- KeyShot itself adopted OpenPBR for interchange in 2025.3, so materials round-trip between the two products.
- It covers every lobe the 34 types need: fuzz, coat, metal, specular dielectric, transmission, subsurface, diffuse, emission, thin film.

**Consequence for the codebase:** there is exactly one `eval`/`sample`/`pdf` implementation for surfaces. A new material "type" is a data file, not code. This is the single largest complexity reduction available in this project — take it.

---

## 2. The OpenPBR Surface model

### 2.1 Layer stack

Evaluated top to bottom; each layer attenuates what is beneath it:

```
        ┌──────────────────────────────┐
        │  Fuzz            (sheen)      │  fuzz_weight
        ├──────────────────────────────┤
        │  Coat            (dielectric) │  coat_weight   ← clearcoat, with darkening
        ├──────────────────────────────┤
        │  Thin film       (interference)│ thin_film_weight  ← modifies specular below
        ├──────────────────────────────┤
        │  Base substrate:              │
        │    Metal          (conductor)  │  base_metalness
        │    ⊕ Specular     (dielectric) │  specular_weight
        │    ⊕ Transmission (dielectric) │  transmission_weight
        │    ⊕ Subsurface   (random walk)│  subsurface_weight
        │    ⊕ Diffuse      (Oren-Nayar) │  the remainder
        ├──────────────────────────────┤
        │  Emission                     │  emission_luminance
        └──────────────────────────────┘
```

Mixing within the base is by weight, not by adding lobes: `base = lerp(dielectric_base, metal, base_metalness)`, where `dielectric_base` itself mixes transmission / subsurface / diffuse.

### 2.2 Parameter reference

| Parameter | Range | Meaning |
|---|---|---|
| `base_weight` | 0–1 | Overall base layer strength |
| `base_color` | RGB/spectrum | Diffuse albedo, or conductor reflectance when metallic |
| `base_metalness` | 0–1 | 0 = dielectric, 1 = conductor |
| `base_diffuse_roughness` | 0–1 | Oren-Nayar roughness (0 = Lambertian) |
| `specular_weight` | 0–1 | Dielectric specular lobe strength |
| `specular_color` | RGB | Tint on specular reflection (rarely non-white physically) |
| `specular_roughness` | 0–1 | GGX α = roughness² (perceptual mapping) |
| `specular_ior` | 1.0–3.0 | Index of refraction; 1.5 default (plastics/glass) |
| `specular_roughness_anisotropy` | 0–1 | Anisotropy magnitude; direction from `geometry_tangent` |
| `transmission_weight` | 0–1 | Refractive transmission strength |
| `transmission_color` | RGB | Transmitted tint (surface, thin-walled case) |
| `transmission_depth` | ≥0 (scene units) | Distance at which `transmission_color` is reached via Beer-Lambert absorption. 0 = thin/no volume |
| `transmission_scatter` | RGB | Volumetric scattering coefficient inside the medium |
| `transmission_scatter_anisotropy` | -1–1 | HG phase `g` |
| `transmission_dispersion_abbe` | 0–100 | Abbe number; 0 = no dispersion. Triggers spectral mode (§01 §7.4) |
| `medium_priority` | 0–10 | Which medium wins where two overlap (§01 §6.1); Liquid is 1, everything else 0. Tequila's addition: OpenPBR has no nested media |
| `subsurface_weight` | 0–1 | Random-walk SSS strength |
| `subsurface_color` | RGB | Diffuse-equivalent albedo of the SSS medium: what a part much thicker than the radius returns |
| `subsurface_radius` | RGB (scene units) | Per-channel mean free path |
| `subsurface_scatter_anisotropy` | -1–1 | HG `g` inside the medium |
| `coat_weight` | 0–1 | Clearcoat strength |
| `coat_color` | RGB | Coat absorption tint |
| `coat_roughness` | 0–1 | Coat GGX roughness |
| `coat_ior` | 1.0–2.0 | Coat IOR, 1.5 default |
| `coat_darkening` | 0–1 | Physically-correct darkening of the base under a coat (total internal reflection). Keep at 1 for realism |
| `fuzz_weight` | 0–1 | Sheen layer strength |
| `fuzz_color` | RGB | Sheen tint |
| `fuzz_roughness` | 0–1 | Sheen roughness |
| `thin_film_weight` | 0–1 | Interference layer strength |
| `thin_film_thickness` | 0–2000 nm | Film thickness |
| `thin_film_ior` | 1.0–3.0 | Film IOR |
| `flake_weight` | 0–1 | Share of the base that is metal flakes (Metallic Paint; not in OpenPBR) |
| `flake_color` | RGB | Flake tint |
| `flake_size` | scene units | Flake diameter |
| `flake_spread` | 0–1 | Flake tilt, up to 60° |
| `flake_roughness` | 0–1 | Roughness of each flake |
| `displacement_detail` | scene units | Subdivision edge length for Displacement; 0 is bump-only (not in OpenPBR) |
| `emission_luminance` | ≥0 (nits) | Emission in physical units |
| `emission_color` | RGB | Emission chromaticity |
| `geometry_opacity` | 0–1 | Cutout transparency (not refractive) |
| `geometry_thin_walled` | bool | Treat as an infinitely thin shell (no volume, no refraction offset) |
| `geometry_normal` | vector | Shading normal after bump/normal mapping |
| `geometry_coat_normal` | vector | Independent normal for the coat layer (orange peel over smooth base) |
| `geometry_tangent` | vector | Anisotropy direction |

One parameter outside OpenPBR is carried alongside these:

| Parameter | Range | Notes |
|---|---|---|
| `geometry_occlusion` | 0–1 | Baked ambient occlusion; 1 is unoccluded. Scales every lobe. |

A path tracer computes occlusion by tracing it, so baking it into a texture as
well counts the same shadow twice, and OpenPBR is right to leave it out. It is
here because the shadowing an exported asset bakes is usually not shadowing
this renderer can find: the detail it belongs to — screw recesses, panel gaps,
the channel a strap sits in — is in the normal map and not in the mesh, so no
ray can be blocked by it, and the occlusion map is the only record of it that
survives the export. Left unread, every crease in an imported model lights as
if it were flat. The default is 1, so the double-counting happens only where an
artist authored a map and asked for it.

Scaling *every* lobe, rather than the diffuse alone as a real-time renderer
would, follows from §5: Tequila has no analytic direct light to double-shadow,
because all of its light arrives from the environment or from emissive
geometry. It is also the only version that does anything on a metal.

### 2.3 Lobe implementations

| Lobe | Model | Sampling |
|---|---|---|
| Diffuse | Oren-Nayar (reduces to Lambert at roughness 0); use the energy-preserving formulation | Cosine-weighted hemisphere |
| Specular dielectric | GGX/Trowbridge-Reitz microfacet, Smith height-correlated masking-shadowing, exact Fresnel (not Schlick — Schlick errs visibly at high IOR and for the gem case) | VNDF sampling [Heitz 2018] |
| Metal | GGX with **conductor Fresnel** from complex IOR (n, k). Provide measured n/k spectra for common metals (Al, Ag, Au, Cu, Cr, Fe, Ni, Ti, W, Pt) and an artist-friendly `base_color` + `specular_color` (edge tint) parameterization [Gulbrandsen 2014] that maps to n/k | VNDF |
| Transmission | GGX with dielectric Fresnel and refraction; medium attached for volumetric absorption/scattering | VNDF + Fresnel-weighted reflect/refract choice |
| Subsurface | Random walk (§01 §6.4) | Medium sampling |
| Coat | GGX dielectric layer with albedo-scaled attenuation of layers beneath | VNDF |
| Fuzz/sheen | Zeltner sheen (or Conty-Kulla charlie sheen) with an energy-compensation LUT | Analytic sheen sampling |
| Thin film | Airy-summation interference over the specular Fresnel term; spectral when possible, 3-wavelength fallback in RGB mode | Same as underlying specular |

**Energy compensation is mandatory** for GGX at high roughness (multiple-scattering energy loss makes rough metals visibly too dark). Use the Kulla-Conty energy-compensation approach with a precomputed 2D albedo LUT (roughness × cosθ). Without it, rough gold looks brown.

### 2.4 Lobe selection during sampling

Sample lobes proportionally to their estimated contribution:
```
weights = [fuzz_albedo·fuzz_weight,
           coat_albedo·coat_weight,
           base_albedo·(1 - absorbed_above)]
```
Choose one via the `uc` sample, then importance-sample within it. `pdf()` must return the **weighted sum over all lobes**, not just the sampled one, or MIS breaks (this is the most common single-lobe-pdf bug).

---

## 3. Material catalog — exact presets

Each entry: OpenPBR parameter defaults + the parameter set the UI exposes for that type. `—` means the parameter is fixed at its listed value and hidden.

### 3.1 Basic materials

*(Metallic Paint appears here alongside Paint because they share an implementation path; KeyShot categorizes it under Advanced. Category placement is a UI grouping decision only — the total catalog is 34 types plus 4 light types either way.)*

| Type | Key parameter settings | UI exposes |
|---|---|---|
| **Diffuse** | `base_metalness=0`, `specular_weight=0`, `base_diffuse_roughness=0` | Color, Diffuse Roughness, Bump |
| **Flat** | Unlit constant emission (`emission_color=color`, all other lobes 0), not affected by lighting; renders as its colour, still writes depth/ID | Color |
| **Metal** | `base_metalness=1`, complex IOR from a metal preset or from color+edge-tint | Type (Al/Ag/Au/Cu/Cr/…/Custom), Color, Roughness, Anisotropy + rotation, Bump |
| **Plastic** | `base_metalness=0`, `specular_ior=1.5`, `specular_weight=1`, `specular_roughness=0.1` | Diffuse Color, Specular Color, Roughness, IOR, Bump, Opacity |
| **Paint** | Plastic base + `coat_weight=1`, `coat_roughness=0.02` | Base Color, Roughness, Coat Roughness, Coat IOR, Bump |
| **Metallic Paint** | Paint + a flake sub-layer: procedural Flakes node driving a metallic normal perturbation and a second metal lobe | Base Color, Flake Color, Flake Size, Flake Density, Coat Roughness |
| **Glass** | `transmission_weight=1`, `geometry_thin_walled=true`, `transmission_depth=0` | Color, Roughness, IOR, Thickness (for tint) |
| **Glass (Solid)** | `transmission_weight=1`, `geometry_thin_walled=false`, `transmission_depth>0` (Beer-Lambert absorption through real volume) | Color, Transmission Depth, Roughness, IOR, Abbe (dispersion) |
| **Liquid** | Glass (Solid) with liquid IOR default 1.33, medium priority set so it wins over container glass at coincident surfaces (§01 §6.1) | Color, Depth, IOR, Roughness |
| **Translucent** | `subsurface_weight=1`, moderate `subsurface_radius`, `specular_weight=1` | Surface Color, Subsurface Color, Translucency (radius scale), Roughness |
| **Thin Film** | `thin_film_weight=1`, over a metal or dielectric base | Thickness (nm), Film IOR, Base (metal/dielectric), Roughness |

### 3.2 Advanced materials

| Type | Key parameter settings | UI exposes |
|---|---|---|
| **Advanced** | Nothing hidden | **All** OpenPBR parameters |
| **Generic** | Nothing hidden but organized into groups | All, grouped |
| **Anisotropic** | Metal or dielectric with `specular_roughness_anisotropy>0`, tangent from UV or from a radial/linear tangent field | Color, Roughness, Anisotropy, Rotation, Tangent mode (UV/Radial/Linear), Bump |
| **Dielectric** | `transmission_weight=1`, explicit `specular_ior`, no artistic shortcuts | IOR, Absorption Color, Absorption Distance, Roughness, Abbe |
| **Gem** | Dielectric with high IOR (2.42 diamond default), `transmission_dispersion_abbe` set (55 for diamond), **spectral mode forced on**, increased transmission bounce limit | Gem preset (Diamond/Ruby/Sapphire/Emerald/…/Custom), IOR, Abbe, Color, Absorption |
| **Measured** | BRDF from tabulated data (MERL-format or RGL/BTF); evaluate by interpolation, importance-sample via a precomputed 4D CDF or by fitting a GGX proxy for sampling and using MIS to correct | File, Scale, Rotation |
| **Multi-Layer Optics** | Stack of N thin-film layers, Airy summation across the stack | Layer list (thickness, IOR per layer), Base |
| **Plastic (Cloudy)** | Plastic + `subsurface_weight` moderate + small `subsurface_radius` — light penetrates a little, giving the waxy look of ABS/HDPE | Color, Cloudiness (radius), Roughness |
| **Plastic (Transparent)** | Glass Solid with plastic IOR 1.49 and slight absorption | Color, Depth, Roughness, IOR |
| **RealCloth** | Fiber-level: `fuzz_weight` + anisotropic specular oriented along a **woven tangent field** generated by the Weave procedural (§6.3); optional geometric fuzz via the Fuzz geometry node | Weave pattern, Thread color(s), Thread thickness, Fuzz amount, Roughness |
| **Scattering Medium** | Pure volume: surface is a transparent interface, interior has `transmission_scatter` and absorption; heterogeneous if a Volume Map is connected | Scatter Color, Scatter Distance, Absorption, Anisotropy (g), Density (optional map) |
| **Translucent Medium** | Scattering Medium tuned for high-albedo forward scattering (wax, jade, skin-adjacent) | Color, Radius, Anisotropy |
| **Velvet** | `fuzz_weight=1`, high `fuzz_roughness`, dark base | Color, Sheen Color, Sheen Roughness, Base Darkness |

### 3.3 Special materials

| Type | Implementation |
|---|---|
| **Emissive** | `emission_luminance` in nits, `emission_color`. Registered as a light in the light BVH when luminance > 0 and the material is on visible geometry. Options: visible to camera, cast light, double-sided. |
| **Toon** | NPR: quantize `N·L` into user-defined bands with configurable band colors; add outline via a screen-space normal/depth discontinuity pass or a backface-expanded shell. Does not participate in GI as a light. Must be excluded from denoiser guide albedo (flat regions confuse it — pass a synthetic albedo instead). |
| **Wireframe** | Renders edges: compute barycentric distance to the nearest triangle edge (or to the original BREP edge, which is much better for CAD — use retained topology when available), shade edges with an emissive/flat colour and the interior as transparent or as a second material. |
| **X-Ray** | Additive, view-dependent transparency: opacity ∝ `1 - |N·V|^falloff`, accumulating along the view ray. Ignores shadowing. |
| **Cutaway** | Marks geometry as a **cutting volume**: any surface inside the cutaway solid is not rendered; back-facing surfaces of the cut object are capped and shaded with the assigned cap material. Implemented as a per-ray CSG test against the cutaway primitive(s) at intersection time, plus a cap-surface synthesis pass. Supports plane, box, sphere, cylinder and arbitrary-mesh cutters. |
| **Ground** | Infinite plane with: shadow-catcher mode (renders only received shadow as alpha), reflection amount, and optional full material. Shadow-catcher math: `alpha = 1 - (L_with_object / L_without_object)` per pixel, computed by tracking both quantities during NEE. |

### 3.4 Light materials

| Type | Parameters |
|---|---|
| **Area Light** | Shape (rect/disc/sphere/mesh), size, Power (lm) or Luminance (nits), Color/Temperature (K), spread/directionality, visible-to-camera, shadow casting, light layer |
| **Point Light** | Power (lm) or Intensity (cd), Color/Temperature, radius (0 = delta, >0 = soft sphere light) |
| **Spotlight** | Point light + cone angle, penumbra angle, falloff exponent, optional projected texture (gobo) |
| **IES Light** | Point/area light + IES profile file; parse LM-63, build 2D CDF, importance-sample |

All lights share: intensity, colour (RGB or Kelvin blackbody), light layer ID, visibility flags (camera/diffuse/specular/shadow).

---

## 4. Material graph

### 4.1 Data model

```rust
struct MaterialGraph {
    nodes: SlotMap<NodeId, Node>,
    links: Vec<Link>,             // (from_node, from_socket) → (to_node, to_socket)
    output: NodeId,               // the terminal surface/material node
    groups: Vec<NodeGroup>,       // collapsible subgraphs
}

struct Node {
    kind: NodeKind,               // enum over the full catalog, §6
    params: HashMap<ParamId, Value>,   // literal values for unconnected inputs
    position: Vec2,               // editor layout only
    label: Option<String>,
}
```

Sockets are typed: `Float`, `Color`, `Vector`, `Normal`, `Material`, `Bool`. Implicit conversions are the ones Cycles makes between its sockets, so a graph links as it would in Blender: `Float → Color` and `Float → Vector` (broadcast), `Color → Float` (luminance), `Vector → Float` (the average), `Color ↔ Vector` (component for component), `Vector ↔ Normal`. A `Bool` links to nothing else; that is a connection error surfaced in the editor.

### 4.2 Simple panel and graph are the same data

The Material tab's simple parameter panel is a **view over the graph**, not a separate representation:
- A material with no textures is a graph containing one material node with literal parameters.
- Enabling the texture toggle next to a parameter inserts a Texture Map node connected to that input.
- Opening the Material Graph shows exactly that graph.
- Editing the graph and returning to the simple panel shows literals where inputs are unconnected and a "connected" indicator (with a jump-to-node affordance) where they are.

This eliminates the entire class of "the simple panel and the graph disagree" bugs, and it is why §1 insists on one model.

### 4.3 Compilation

Compile each graph once per edit into an evaluation form:

1. **Validate**: no cycles (except through explicitly-allowed feedback-free constructs), all required inputs satisfiable, type-check every link.
2. **Constant-fold**: any subgraph with no varying inputs (no texture lookups, no geometry attributes) evaluates to a constant at compile time.
3. **Topologically sort** and emit either:
   - a **flat instruction list** over a small register machine (MVP; simple, debuggable, fast enough on CPU), or
   - **Slang source** compiled to CPU and GPU targets (M9; required for GPU parity, §01 §10).
4. **Cache** by graph content hash. Recompilation on every slider drag is unacceptable — parameter changes that do not alter topology must patch a uniform block, not recompile.

Emit alongside the compiled shader: the set of required geometry attributes (UV sets, tangents, vertex colours), the set of textures referenced, and whether the material needs a medium — the renderer uses these to configure intersection and shading requirements per material.

### 4.4 Geometry nodes and displacement

Geometry nodes modify the surface itself rather than its shading:

| Node | Implementation |
|---|---|
| **Displace** | True displacement. Two modes: (a) **bump-only approximation** (fast, no silhouette change); (b) **adaptive tessellation** at BVH build time, subdividing to a screen-space-error target and displacing vertices along normals. Displacement changes geometry, so it invalidates the BLAS — this is why 2025.3's "85% faster displacement refresh" was newsworthy; treat displacement rebuild as async (§01 §9.4). |
| **Bubbles** | Procedural spherical inclusions inside a transmissive medium: implemented as a volumetric density field (bubbles as low-density spheres in a Volume Map) rather than real geometry. |
| **Flakes** | Perturbs the shading normal with a Worley-cell-based random flake orientation field, plus an independent metal lobe. No geometry change. |
| **Fuzz** | Fine hair/fiber shell. Two modes: shading-only (sheen lobe with a noise-perturbed normal) and geometric (generate short curve primitives on the surface at BVH build; only for final renders — too expensive interactively). |

### 4.5 Animation nodes

`Color Fade` and `Curve Fade` read the current animation time from the shading context and interpolate. This requires the shading context to carry `time`, and requires that any material with an animation node be marked time-varying so the renderer knows the shader output changes per frame (invalidating cross-frame caching).

---

## 5. Texture pipeline

### 5.1 Loading and caching

Use OpenImageIO's tiled texture cache. Requirements:
- **MIP pyramids** generated on load (or read from pre-mipped .tx files); never point-sample a full-resolution texture for a distant surface.
- **Tile-level residency** with a configurable memory budget; product scenes with 40 × 4K textures must work on 8 GB machines.
- **Async load** with a placeholder (mid-grey for colour, 0.5 for data) until resident — never block the render thread on I/O.
- **UDIM** support (`<UDIM>` / `_1001` tile naming).
- Formats: PNG, JPEG, TIFF, EXR, HDR, TGA, PSD, WebP, plus video (MP4/MOV via ffmpeg) for Video Map nodes.

### 5.2 Colour space handling

Per §01 §7.2. The importer assigns colour spaces by **map role**, not by file format, and defaults data maps to Raw. Store the assigned colour space in the material so it round-trips through the file format.

### 5.3 Projection and mapping

| Mode | Definition |
|---|---|
| **UV** | Standard `mesh.uv[set]`. Multiple UV sets supported; node selects the set. |
| **Planar** | Project along an axis; `u = p·X̂`, `v = p·Ŷ` in the mapping node's local frame. |
| **Cylindrical** | `u = atan2(p.z, p.x)/2π`, `v = p.y`. Seam handling: derivative-aware to avoid a MIP artifact line at the wrap. |
| **Spherical** | Equirectangular from the object centre. |
| **Box / Tri-planar** | Three planar projections blended by `|n|^k` (k ≈ 4–8 for a tight blend). The workhorse for CAD geometry, which usually has no UVs. |
| **Node (Camera/Screen)** | Screen-space projection — needed for backplate-matched labels. |

Shared controls per mapping: `scale (u,v,w)`, `offset`, `rotation`, `repeat/tile`, `mirror`, `flip`, and a real-world-units toggle (texture scale in mm/inches rather than UV units — essential when scene units matter, and something users of CAD-derived scenes expect).

### 5.4 Bump, normal, displacement

- **Bump**: perturb the shading normal from the gradient of a scalar height field. Compute the gradient by finite differences in texture space using ray differentials for the step size (screen-size-independent bump, avoiding the classic "bump gets stronger when you zoom out" artifact).
- **Normal map**: tangent-space by default; require a tangent frame (generate MikkTSpace-compatible tangents at import so normal maps baked in other tools match).
- **Bump Add** node: combine bump layers by summing *gradients*, not by summing heights. Summing heights is wrong and produces flattened detail; this is a real and common bug.
- **Displacement**: §4.4.

### 5.5 Video textures

Frame-accurate lookup by animation time; decode on a background thread with a small ring buffer of decoded frames. Mark the material time-varying.

---

## 6. Procedural texture catalog — algorithms

All procedurals evaluate in **object space by default** (so they don't swim when the object moves) with a mapping node available to change that. All are seedable so duplicated parts differ.

### 6.1 Noise primitives (shared building blocks)

| Primitive | Algorithm |
|---|---|
| Perlin / gradient noise | Standard 3D gradient noise, quintic interpolant |
| Simplex noise | Simplex lattice; cheaper in 3D/4D, no axis-aligned artifacts |
| Worley / cellular | Feature points per cell, F1/F2/F2−F1 distance metrics, Euclidean/Manhattan/Chebyshev |
| fBm | `Σ amplitude^i · noise(frequency^i · p)` over N octaves, with lacunarity and gain |
| Turbulence | fBm of `|noise|` |
| Ridged multifractal | `Σ (1 - |noise|)²` weighted per octave |

### 6.2 3D procedural nodes

| Node | Algorithm |
|---|---|
| **Noise (Fractal)** | fBm with exposed octaves, lacunarity, gain, and a noise-basis selector |
| **Noise (Texture)** | Single-octave value/gradient noise with contrast and bias controls |
| **Cellular** | Worley with selectable distance metric and F-combination; outputs distance, cell ID (for random per-cell colour), and edge mask |
| **Marble** | `sin(p·axis·frequency + turbulence(p)·distortion)` mapped through a colour gradient — the classic vein formula; expose axis, frequency, distortion, and the gradient |
| **Wood** | Radial rings: `r = length(p.xy)` (about a configurable grain axis), `rings = frac(r·frequency + fbm(p)·distortion)`, mapped through a gradient; plus an axial grain noise stretched along the grain direction |
| **Wood (Advanced)** | Adds: growth-ring width variation, earlywood/latewood contrast, ray flecks (anisotropic streaks), knots (radial distortion around seeded points), and per-board offset/rotation randomization |
| **Granite** | Multi-scale Worley cells with per-cell random colours from a palette, plus a fine noise overlay and a speckle threshold |
| **Leather** | Worley cell structure for the pebble pattern (cells = grain bumps), with a crack network along cell boundaries (`F2−F1` thresholded) and a fine wrinkle noise; outputs colour and height |
| **Scratches** | Anisotropic line field: sample N seeded line segments per cell with random orientation within a user-set angular spread; distance-to-nearest-line thresholded and modulated by a length/width distribution. Outputs a height/mask for roughness and bump |
| **Spots** | Worley cells thresholded at a coverage parameter, with size variance and edge softness |
| **Camouflage** | 3–5 layers of thresholded fBm at different scales, each assigned a palette colour, composited in order |
| **Contour** | Banding of an input field (default: distance from a plane, or a connected scalar): `frac(field · frequency)` shaped into lines with width and softness. Used for topographic and technical looks |
| **Curvature** | Geometric: estimate mean curvature at the shading point from the second fundamental form (or, cheaply and robustly, from the divergence of the normal field over a small neighbourhood computed at tessellation time and stored as a vertex attribute). Outputs convex/concave/both masks. **The workhorse for automatic edge wear** — expose radius and contrast |
| **Occlusion** | Trace N short rays over the hemisphere at shading time, radius-limited; return the occluded fraction. Cache per-vertex when the geometry is static to avoid per-sample cost |
| **Volume Map** | 3D texture lookup from a voxel grid (VDB or a dense grid), trilinear filtered |
| **Vertex Color** | Read a named per-vertex colour attribute |
| **Brushed (Radial)** | Anisotropic 1D noise in polar coordinates about a centre axis — concentric brushing. Outputs an anisotropy direction field as well as a bump |
| **Contour/Brushed shared note** | Both must output a **tangent direction** usable by the anisotropic specular lobe, not just a scalar |

### 6.3 2D pattern nodes

| Node | Algorithm |
|---|---|
| **Brushed** | Anisotropic noise: 1D noise stretched along a direction; outputs bump + tangent field |
| **Weave** | Procedural over/under warp-weft pattern with configurable thread count, thread width, gap, weave type (plain/twill/satin), and per-thread colour variance. Outputs colour, height, and a **per-thread tangent** (which is what makes RealCloth's anisotropic sheen read correctly) |
| **Mesh** | Perforated grid/mesh pattern: hole shape (round/square/hex), pitch, and opacity output for cutouts |

### 6.4 Image texture nodes

**Texture Map** (standard), **Tiled UV** (explicit tiling controls with per-tile offset/rotation randomization to break repetition), **Tri-Planar** (§5.3), **Video Map** (§5.5).

### 6.5 Utility nodes

| Node | Function |
|---|---|
| Color Adjust | Hue, saturation, value, gamma, contrast, brightness, levels |
| Color Composite | Blend two inputs with a mode (mix, add, multiply, screen, overlay, difference, dodge, burn, …) and a factor/mask |
| Color Invert | `1 - c` |
| Color Key Mask | Chroma-key: mask where input is within tolerance of a key colour |
| Color To Number | Luminance or a selected channel → scalar |
| Color Curve Randomize | Per-instance random remap of a colour ramp — gives variation across duplicated parts from one material |
| Bump Add | Gradient-space sum (§5.4) |
| Mapping 2D | UV transform: scale, rotate, offset, tile, mirror |

Add these (KeyShot lacks them; they cost little and are frequently needed): **Math** (add/sub/mul/div/pow/min/max/clamp/step/smoothstep), **Range/Remap**, **Mix**, **Fresnel** (facing-ratio), **Geometry Info** (position, normal, incoming vector, object index, random-per-instance), **Ramp** (colour ramp with editable stops), **Switch**.

### 6.6 Blender's shader nodes

Tequila also has Blender's own shader nodes (`tequila_materials::nodes::ShaderNode`), evaluated with Cycles' algorithms. These are ported to Rust in `tequila_texture::cycles` from Cycles' SVM kernels, which are Apache-2.0 like Tequila. The nodes are:

- **Input:** Texture Coordinate, Geometry, Layer Weight, Fresnel, Object Info, Light Path (Is Camera, Shadow, Diffuse, Glossy and Transmission Ray: 1 for the kind of ray that reached the point, a switch between what the camera, a shadow, a bounce, a reflection and glass see).
- **Texture:** Noise (1–4D, all five fractal kinds, distortion, colour), Voronoi (1–4D, F1/F2/smooth F1/distance to edge/n-sphere radius, four metrics, fractal), Wave, Magic, Brick, Checker, Gradient, White Noise, Gabor (2D and 3D; value, phase and intensity: the streaks of brushed metal, wood figure and fibres).
- **Colour:** Mix (float, vector, and colour with Blender's 19 blend modes), Hue/Saturation/Value, Brightness/Contrast, Gamma, Invert.
- **Converter:** Math (all 41 operations), Vector Math (29), Map Range (4 interpolations), Clamp, Separate/Combine XYZ, Separate/Combine Color (RGB/HSV/HSL).
- **Vector:** Mapping (point, texture, vector, normal).

A node can have several outputs, and a link names which one it leaves from (`Link::from_socket`). A texture's Vector input reads object space in scene units when nothing is wired to it, as Tequila's own patterns do. Layer Weight and Fresnel read the shading normal and the view. Geometry and Texture Coordinate give world vectors in Tequila's frame (Y up) and UVs with v up, as Blender has them.

Both engines run the same algorithms, and the Cycles translation maps every node onto the identical Cycles node. Each node's output therefore lands on the same pixels at the same levels in both: `app/src/batch.rs` `blender_nodes_shade_alike_in_both_engines` checks 37 cases at a correlation of at least 0.97, and they come out at 0.999 or better.

Tequila's own Curvature and Occlusion nodes reach Cycles as the numbers Tequila's renderer measured a vertex (`Mesh::curvature`, `Mesh::occlusion`), sent as a vertex attribute and shaped the same way (`curvature_and_occlusion_reach_cycles`): Cycles' own Ambient Occlusion and Pointiness measure something else, and the two renderers would part on every crease.

**Mixing surfaces.** A **Shader** node is another surface, with the same parameter inputs as the material's own. Wired into a **Mix Shader** (its `Fac` the share of the shader) or an **Add Shader**, and that into the surface's **Layer** input, it mixes with the material's own surface; a Shader node's own Layer mixes it in turn. Two surfaces of a kind - neither or both metal, see-through, under the skin, coated, fuzzed or thin-walled, by the same amount (`same_lobes`) - mix as one, each parameter mixed (`MaterialGraph::mixed_per_parameter`), which is exact for them and costs nothing. Others are each a program of their own (`CompiledLayer`): Tequila's renderer shows a path one or the other as often as `Fac` says, or, added, each half the time counted twice (`shade::choose_layer`), which is the mix and the sum on average; Cycles mixes or adds their two closures. The other surface is a plain one of its parameters: labels, bump, flakes and a glass's dispersion are the material's own surface's. `mixed_shaders_shade_alike_in_both_renderers` holds a plastic mixed with a metal, and a glow added, to the same colour in both.

A node can be **muted**, as in Blender. Each of its outputs then passes on the first wire into it that can carry that type, or nothing (`MaterialGraph::without_muted`).

---

## 7. Labels and decals

Labels are ordered layers *on top of* a base material — not modifications to it.

### 7.1 Data model

```rust
struct LabelLayer {
    source: LabelSource,          // Texture | Video | full Material
    projection: ProjectionMode,   // planar (default), tri-planar, UV, node/camera
    transform: TextureTransform,  // position, rotation, scale — the "Move Texture" gizmo
    maps: { color, opacity, bump, roughness, metalness },
    material_type: MaterialPreset,  // label surface is its own material (plastic/metal/paint…)
    two_sided: bool,
    depth_offset: f32,            // avoids z-fighting on coincident surfaces
}
```

### 7.2 Shading

At shading time, evaluate layers bottom-up: each label's `opacity` (from its opacity map × its alpha × its projection footprint) composites its BSDF parameters over the accumulated ones. Because each label carries its own material type, a metallic foil sticker on a matte plastic case shades correctly — this is what makes the feature worth having.

Labels outside their projection footprint contribute nothing (opacity 0), and a label's projection must be clamped or set to not-wrap by default. Wrapping labels around the whole object because the projection tiles is the most common user complaint about this feature in other tools; default `repeat = false`.

### 7.3 Placement

- **Drop on geometry in the viewport**: ray-pick the surface point and normal, place a planar projection centred at the hit point, oriented with the surface normal, with the up-vector derived from the camera's up projected onto the surface plane (this gives the "auto-orients but sometimes surprising rotation" behavior users expect; make the rotation immediately editable via the gizmo).
- **Drop into the layer list**: default orientation (axis-aligned, 0°), no auto-placement.
- **PSD DPI**: read the embedded resolution and size the label in real-world units so a label authored at 300 dpi arrives at its intended physical size. Requires scene units to be meaningful (§03 §3).

### 7.4 Labels AOV

All labels composited alone with alpha, per §01 §12. Requires tracking label contribution separately during shading — cheap if done as a parallel accumulation, painful if retrofitted. Build it in from the start.

---

## 8. Multi-materials, linking, and the Tequila Standard Library

### 8.1 Multi-Materials

A Multi-Material is an ordered set of material variants assigned to the same part(s), with one active. Studios and the Configurator select among them. Model:

```rust
struct MultiMaterial { id, name, variants: Vec<MaterialRef>, active: usize }
```

Switching the active variant is a scene mutation classified as a material change in the dirty taxonomy (§01 §9.3) — region restart, not full restart.

### 8.2 Material linking semantics (critical)

Materials are **shared instances**, referenced by parts. This is the behavior everything downstream depends on:

- Assigning material M to part P sets `P.material = M`. Every part referencing M shows the change when M is edited.
- **Unlink** creates `M' = deep_copy(M)` and sets `P.material = M'`, named `M 2` (auto-incrementing suffix). Only P changes thereafter.
- **Link to** sets a part's material to an existing instance.
- Deleting the last reference to a material removes it from the in-project library unless the user pinned it.
- The in-project library lists instances, with a reference count shown.

Full interaction rules, including what a drag-and-drop does with a multi-selection and with a group, are in `04-UX-SPEC §3`.

### 8.3 Tequila Standard Library

Ship an own-content library — never third-party licensed content (§0.2). Requirements:

- **All materials authored as parameter presets over OpenPBR** (a small JSON per material), not as binary blobs. The whole library should be human-readable and diffable, and a user should be able to submit a new material as a pull request.
- **Procedural first**: prefer procedural textures over bundled bitmaps so the library stays small and resolution-independent.
  A preset states its patterns as a short recipe, `"patterns": [...]`, each entry naming the parameter it drives and how: `{"param": "base_color", "procedural": {"kind": "wood_advanced", ...}, "colors": [{"at": 0, "color": [r, g, b]}, ...]}` for a colour through a ramp, `"range": [low, high]` for a number, `"depth": metres` for `bump`, and an optional `"seed"`. The procedural takes the same fields as in a project file. Frequencies are per metre, and thumbnails are shot on a 5 cm ball so a pattern previews at the scale it renders. A preset saved from the application records its graph instead (`"graph"`, as in a project file); patterns and image maps go with it, labels do not. Its maps are copied into `textures/<entry file stem>/` beside the entry and listed as `"textures": [{"id": 1, "path": "textures/leather037/Leather037_Color.jpg", "role": "color"}, ...]`, with paths relative to the entry's folder; the graph's image nodes use those ids, which are renumbered into a project's own when the preset is applied, reusing a picture the project already has. Deleting the entry deletes its copies.
- **The standard library is written as code**: `crates/tequila-format/examples/standard_library/` builds every preset and writes `assets/library/standard/` (`cargo run --release -p tequila-format --example standard_library`). Edit a recipe there rather than its JSON, which the generator rewrites.
  - **What a preset is:** a type's defaults, the physical values that make the material what it is (measured n/k, absorption, scattering), and a node graph of Blender's nodes and Tequila's patterns.
  - **What the graph adds:** the detail that makes a material look made rather than computed:
    - the polish of the mould and the gloss that varies with handling;
    - orange peel under a clear coat or in a powder coat;
    - the stipple of a texture-moulded part;
    - growth rings and drawn-out pores in wood;
    - veins as the contour lines of a deep noise in marble;
    - pebble grain in leather;
    - spangles, knurls, hammer dimples, rust and verdigris on metal.
  - **Checking the set:** `library_contact_sheet` in `app/src/library_tab.rs` renders the whole library onto one sheet for judging it together (`TEQUILA_SHEET=sheet.png cargo test --release -p tequila-app library_contact_sheet -- --ignored --nocapture`).
- Coverage at minimum: 20 metals (with real n/k data), 25 plastics (matte/gloss/textured/transparent), 12 glasses, 15 paints (incl. metallic and pearlescent), 10 rubbers, 15 woods, 12 fabrics/leathers, 10 stones/ceramics, 8 papers/cards, 10 liquids, 6 gems, plus emissive/display presets.
- **HDRIs**: 20+ CC0 studio and outdoor environments, plus a set of procedurally-generated softbox studio setups built from HDRI Editor pins (which cost nothing to ship — they are just pin definitions in a small file).
- `assets/library/CREDITS.md` records the source and licence of every asset. CI fails if an asset lacks an entry.
