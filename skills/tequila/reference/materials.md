# Materials on parts

Tequila's materials are OpenPBR. The order of work: find the part, search the library, put a preset on, then change it if it is not quite right. Make a material from nothing only when the library has nothing close (page `recipes`).

## 1. Find the parts and what they wear

`tequila_nodes {"parts_only": true, "filter": "strap"}` lists parts by a word in their name or their material's name, each with its `id`, `material` (an id) and `material_name`. `tequila_materials` lists every material briefly, with `used_by` (how many parts wear it).

## 2. Search the library

Several hundred presets, each procedural and sized for real objects, plus the person's own.

```json
tequila_library {}
tequila_library {"search": "black leather"}
tequila_library {"category": "Wood", "limit": 40}
```

With nothing it lists the categories and how many presets each has. Every word of `search` must be in the preset's name, category, type, tags or description: if nothing comes back, use fewer or other words (the colour, the finish, what it is made of). Do not read the whole library with `tequila_query`: it is a third of a million characters.

What is there, to choose words by: **Metal** (aluminium brushed, bead blasted, polished and anodised in colours; stainless, chrome, brass, copper, gold, titanium, knurled steel, cast iron), **Plastic** (ABS, piano black, matte, satin, soft touch, rubber, silicone, polycarbonate, nylon, VDI textures), **Paint** (automotive solid, metallic and pearl; matte, satin, powder coat, enamel), **Wood**, **Fabric** (denim, canvas, linen, felt, velvet, wool, upholstery, nylon, webbing, laces), **Leather & Suede**, **Composite** (carbon fibre, forged carbon, kevlar, fiberglass), **Stone** (marble, granite, concrete, terrazzo, slate), **Glass** (clear, frosted, reeded, tinted, acrylic, sapphire), **Gem**, **Liquid**, **Ceramic**, **Translucent** (wax, jade, soap), **Light** (LEDs, a glowing screen), **FDM 3D Print**.

## 3. Put a preset on parts

```json
tequila_library_apply {"name": "Leather Black", "nodes": [7, 8]}
tequila_library_apply {"name": "Aluminium Brushed", "nodes": [21], "unlink": true}
```

The name is exactly as the search gave it. The listed parts share one new material; `unlink` gives them a copy that is theirs alone. One undo step. Pick by what the part is made of in the real product.

## 4. Change it

**Parts share materials.** Changing a material changes every part that wears it. To change one part alone, give it its own copy first, then read its new material id from `tequila_nodes`:

```json
{"command": "material.unlink", "node": 12}
```

**A colour, a finish, a number** - one parameter by its key:

```json
{"command": "material.set_parameter", "material": 3, "parameter": "base_color", "value": "#c0392b"}
{"command": "material.set_parameter", "material": 3, "parameter": "specular_roughness", "value": 0.25}
```

**The colour did not change?** In many presets the colour comes from a pattern, not from `base_color` (leather's grain, wood's rings, a weave). `tequila_material {"id": 3}` reads the one material whole: if its `definition.graph.links` has a link whose `socket` is `param:base_color`, the colour is in the graph. Then recolour it by its recipe:

1. `tequila_material_recipe {"material": 3}` answers with the material as a recipe.
2. Change the `colors` of the patterns that drive `base_color` (keep their darks and lights the same distance apart, or the grain goes flat), or the colours inside its `graph`.
3. `tequila_material_update {"material": 3, "recipe": {...}}` puts it back, and shows it on a ball.

**Another kind of material** - the type decides which parameters exist. Set the type first, then its parameters:

```json
{"command": "material.set_type", "material": 3, "type": "metal"}
```

| Type | For |
|---|---|
| `plastic` | Opaque plastics, glazed ceramics: colour under a glossy skin |
| `plastic_transparent`, `plastic_cloudy` | Clear and tinted plastics; frosted and milky ones |
| `metal` | Bare metals, their colour from measured data |
| `anisotropic` | Brushed and spun metal, the highlight stretched along the grain |
| `paint`, `metallic_paint` | Solid glossy paint; car paint with flakes under a clear coat |
| `glass`, `glass_solid` | Thin glass (a bulb, a window); solid glass and thick clear parts |
| `liquid`, `gem` | A liquid in a container; cut gems with fire |
| `translucent`, `translucent_medium`, `scattering_medium` | Light passing into it: wax, soap, skin, milk, jade |
| `velvet`, `real_cloth` | Fabrics |
| `diffuse`, `flat` | Completely matte; unlit colour |
| `thin_film`, `multi_layer_optics` | Iridescent coatings |
| `emissive` | Screens, LEDs, glowing parts |
| `generic`, `advanced`, `dielectric` | Every parameter exposed, for something unusual |
| `toon`, `wireframe`, `xray` | Stylised looks |

**The parameters that matter:**

| Key | Means | Typical |
|---|---|---|
| `base_color` | The colour | `"#C23B32"` |
| `base_metalness` | 0 plastic, paint, wood; 1 metal | 0 or 1 |
| `specular_roughness` | 0 a mirror, 1 matte | Polished 0.05-0.15, satin 0.25-0.4, matte 0.5-0.8 |
| `specular_ior` | Strength of reflection on non-metals | 1.5 plastic and glass, 1.33 water, 2.4 diamond |
| `specular_roughness_anisotropy`, `specular_rotation` | Brushed stretch and its direction | 0.9-0.97 with roughness about 0.3 |
| `coat_weight`, `coat_roughness` | A clear lacquer over the surface | Car paint 1 and 0.02-0.05 |
| `transmission_weight`, `transmission_color`, `transmission_depth` | Light through it; the colour it reaches after that many metres | Glass 1; tint over 0.01-0.05 m |
| `subsurface_weight`, `subsurface_color`, `subsurface_radius` | Light scattering inside | Radius 0.002-0.01 |
| `fuzz_weight`, `fuzz_color`, `fuzz_roughness` | The soft sheen of cloth | Velvet 1 |
| `flake_weight`, `flake_color`, `flake_size` | Metallic flakes | Size 0.00002-0.0001 |
| `thin_film_weight`, `thin_film_thickness` | Iridescence | Thickness 300-700 (nm) |
| `emission_luminance`, `emission_color` | Glow | 1 is a picture's white; an indicator 1-1.5 |
| `geometry_opacity` | See-through cut-outs | 1 solid |
| `edge_radius` | Rounds sharp edges in the shading, metres | 0.0002-0.001 |

A parameter a type does not show is refused by name. `tequila_command {"command": "material.set_parameter"}` shows the command; the full list of keys with ranges is in `tequila_query {"what": "api"}` under `material_parameters`, which is long: ask it only for a parameter this table does not name.

## 5. Put an existing material on more parts, rename it

```json
{"command": "material.assign", "material": 3, "nodes": [12, 13]}
{"command": "material.rename", "material": 3, "name": "Body red"}
```

`material.assign` links: the parts then share the material.

## 6. Look at it

`tequila_ui {"viewport": "rendered"}` then `tequila_screenshot`. Solid and material previews do not show real reflections, glass or cloth sheen. `tequila_material_preview {"material": 3}` shows one material alone on a 5 cm ball without touching the scene; `"radius"` in metres makes the ball the part's size, which matters for patterns.

## 7. Keep it

`tequila_material_save {"material": 3, "name": "Brand Red Gloss", "category": "Plastic"}` adds it to the person's own library (they are asked first inside the Assistant). Only when they ask.

## Colours

Give hex sRGB strings everywhere a colour goes: material parameters, environment colours, lights, gradient stops, recipes. `rgb(194, 59, 50)` is `"#C23B32"`. An array is taken as linear ACEScg, the rendering space, as it is. If a field refuses a string, convert: divide each channel by 255; decode with `c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ^ 2.4`; then

- `R = 0.6130974 r + 0.3395234 g + 0.0473792 b`
- `G = 0.0701936 r + 0.9163535 g + 0.0134529 b`
- `B = 0.0206158 r + 0.1095696 g + 0.8698146 b`

`#C23B32` comes to `[0.347, 0.078, 0.044]`; white is `[1, 1, 1]`.

## Good to know

- A liquid in a glass is modelled a fraction of a millimetre into the glass wall; the `liquid` type's higher priority makes it the medium there.
- Glass reaches its colour over `transmission_depth` metres and patterns are sized in metres, so check the part's `size` before tuning either.
- FDM presets build their layer lines along the part's own Z, as STL and STEP have it; a part drawn Y up needs the FDM Layer Lines node's Build Along Z set to 0 (page `graph`).
- Several materials for one part to switch between are colourways: page `variants`.
- Rules that dress parts by their names, for many models alike: Material Templates, page `variants`.
