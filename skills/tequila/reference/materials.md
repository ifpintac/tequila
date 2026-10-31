# Materials in Tequila

Tequila's materials are OpenPBR. `tequila_query {"what": "api"}` is the authority on every type and parameter (names, ranges, what each type exposes); this page is the working summary.

## Start from the library

309 presets, in these categories (`tequila_query {"what": "library"}` lists every name):

| Category | Some of its presets (exact names) |
|---|---|
| Metal (90) | Aluminium Brushed, Aluminium Bead Blasted, Aluminium Polished, Aluminium Anodised Black, Aluminium Anodised Space Grey, Stainless Brushed, Stainless Polished, Chrome, Brass Satin, Copper Polished, Gold Polished, Titanium Brushed, Steel Knurled, Gunmetal, Cast Iron |
| Plastic (59) | ABS Black, ABS White, ABS Red Gloss, ABS Textured, Piano Black, Plastic Matte White, Plastic Satin Black, Soft Touch Black, Rubber Black, Rubber Grip, Silicone Soft Grey, Polycarbonate Clear, Nylon Black, Plastic VDI 30 Black |
| Paint (21) | Automotive Red, Automotive Blue Metallic, Automotive Pearl White, Paint Matte Black, Paint Satin White, Powder Coat Matte Black, Powder Coat White, Enamel Gloss White |
| Wood (23) | Wood Walnut, Wood European Walnut, Wood Red Oak, Wood White Oak, Wood Smoked Oak, Wood Maple, Wood Cherry, Wood Ash, Wood Teak, Wood Ebony |
| Fabric (21) | Denim Indigo, Canvas Natural, Linen Natural, Felt Grey, Suede Tan, Velvet Charcoal, Herringbone Wool, Upholstery Charcoal, Boucle Cream |
| Composite (20) | Carbon Fibre, Carbon Fibre 12K Twill, Carbon Fibre Matte, Forged Carbon, Kevlar Twill, Fiberglass Gelcoat White |
| Stone (21) | Marble Calacatta, Marble Black, Granite Absolute Black, Concrete Polished, Terrazzo, Slate, Travertine |
| Glass (9) | Glass Clear, Glass Frosted, Glass Reeded, Glass Blue Tint, Acrylic Clear, Acrylic Smoked, Sapphire Crystal |
| FDM 3D Print (11) | FDM PLA White, Black, Grey, Red, Blue, Yellow; FDM PLA Translucent Natural, Red, Orange, Green, Blue. Real 0.2 mm layer lines, seam and top raster, built along the part's own Z (STL, STEP); for a part drawn Y up (glTF, OBJ) set the FDM Layer Lines node's Build Along Z to 0 |
| Others | Gem: Diamond, Ruby, Sapphire Blue. Liquid: Water, Coffee Black, Red Wine, Milk Whole. Ceramic: Porcelain, Ceramic Glazed White, Ceramic Matte. Translucent: Wax Candle, Jade, Soap White. Leather: Leather Black, Leather Tan, Leather Cognac. Light: LED Warm White, LED Cool White, Screen Glow. Other: Cork, Paper Matte |

Apply with `{"op": "library", "action": "apply", "name": "<exact name>", "nodes": [ids]}` through `tequila_request`. Pick by what the part is made of in the real product, then adjust colour or roughness if asked. A preset is far more realistic than a material built from scratch: its grain, brushing, weave or flakes are procedural and sized for real objects.

## Types, by what they are for

Set with `{"command": "material.set_type", "material": <id>, "type": "<key>"}`. Changing type keeps what carries over (the colour, usually).

| Key | For |
|---|---|
| `plastic` | Opaque plastics, glazed ceramics, anything coloured with a clear glossy skin |
| `plastic_transparent` | Clear and tinted plastics |
| `plastic_cloudy` | Frosted, milky, translucent plastics |
| `metal` | Bare metals: aluminium, steel, gold, copper (colour from a measured-metal table, `metal_ior`) |
| `anisotropic` | Brushed and spun metal, where the highlight stretches along the grain |
| `paint` | Solid glossy paint |
| `metallic_paint` | Car paint: flakes under a clear coat |
| `glass` / `glass_solid` | Thin glass (a window, a bulb) / solid glass and thick clear parts |
| `liquid` | Liquids in a container |
| `gem` | Cut gems: dispersion, the fire of a diamond |
| `translucent`, `translucent_medium`, `scattering_medium` | Light passing into the material: wax, soap, skin, milk, jade |
| `velvet`, `real_cloth` | Fabrics |
| `diffuse` | Completely matte surfaces |
| `thin_film`, `multi_layer_optics` | Iridescent coatings, soap-bubble colours |
| `emissive` | Screens, LEDs, glowing parts |
| `generic`, `advanced`, `dielectric` | Every parameter exposed, for building something unusual |
| `toon`, `flat`, `wireframe`, `xray` | Stylised looks |

A parameter can only be set on a type that exposes it: set the type first, then the parameters.

## The parameters that matter

Set with `{"command": "material.set_parameter", "material": <id>, "parameter": "<key>", "value": <v>}`. Colours are hex sRGB strings (`"#C23B32"`), converted for you, or `[r, g, b]` already in linear ACEScg (below); lengths are metres.

| Key | Means | Typical |
|---|---|---|
| `base_color` | The colour | `"#C23B32"` |
| `base_metalness` | 0 a dielectric (plastic, paint, wood), 1 a metal | 0 or 1, rarely between |
| `specular_roughness` | 0 a mirror, 1 completely matte | Polished 0.05-0.15, satin 0.25-0.4, matte 0.5-0.8 |
| `specular_ior` | Strength of the reflection on dielectrics | 1.5 plastic and glass, 1.33 water, 2.4 diamond |
| `specular_roughness_anisotropy`, `specular_rotation` | Brushed-metal stretch and its direction | Anisotropy 0.9-0.97 with roughness about 0.3 for brushed |
| `coat_weight`, `coat_roughness` | A clear lacquer over the surface | Car paint 1 and 0.02-0.05 |
| `transmission_weight`, `transmission_color`, `transmission_depth` | Light through the material; the colour it takes on after `transmission_depth` metres | Glass 1; tinted glass colour over 0.01-0.05 m |
| `transmission_tint` | The surface's own tint on light through it, on top of the volume's colour: what makes a thin edge of dyed plastic coloured | White unless the plastic is dyed; translucent PLA 0.45 + 0.55 x its colour |
| `subsurface_weight`, `subsurface_color`, `subsurface_radius` | Light scattering inside (wax, skin) | Radius a few millimetres: 0.002-0.01 |
| `fuzz_weight`, `fuzz_color`, `fuzz_roughness` | The soft sheen of fabric | Velvet 1 |
| `fuzz_density`, `fuzz_length`, `fuzz_thickness`, `fuzz_lean`, `fuzz_curl`, `fuzz_color_variation` | Real fibres grown out of the surface, in the fuzz colour: felt's halo, a tennis ball's nap, velvet's pile | Density 0 (none); felt 300-1500 a cm2, length 0.002 m, thickness 0.00003-0.00005 m |
| `flake_weight`, `flake_color`, `flake_size` | Metallic flakes in paint | Size 0.00002-0.0001 m |
| `thin_film_weight`, `thin_film_thickness` | Iridescence | |

## Colours: hex, or ACEScg arrays

Tequila's colours are linear ACEScg, the rendering space. **Colour fields take hex sRGB strings** - material parameters, environment colours, lights, gradient stops, recipes - and convert them as Tequila's colour pickers do. Use that whenever you can:

```json
{"command": "material.set_parameter", "material": 3, "parameter": "base_color", "value": "#C23B32"}
{"command": "environment.set_parameter", "parameter": "background_color", "value": "#101014"}
```

`rgb(194, 59, 50)` is `#C23B32`. An array is taken as ACEScg as it is - never send sRGB numbers as an array. If a field refuses a string, send an array converted like this:

1. Each channel to 0-1 (divide 0-255 values by 255).
2. Decode: `c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ^ 2.4`
3. Rec.709 to ACEScg:
   - `R = 0.6130974 r + 0.3395234 g + 0.0473792 b`
   - `G = 0.0701936 r + 0.9163535 g + 0.0134529 b`
   - `B = 0.0206158 r + 0.1095696 g + 0.8698146 b`

Worked example: `#C23B32` → (0.761, 0.231, 0.196) → decoded (0.539, 0.044, 0.032) → **[0.347, 0.078, 0.044]**. White is [1, 1, 1] and mid-grey sRGB 128 is about [0.216, 0.216, 0.216].

Do this arithmetic carefully for every array colour you send.

## Your own procedural materials

When no preset fits, write a recipe: `recipes.md`.

## Textures

Only when the user gives files: `material.set_texture` puts an image on a parameter (see `commands.md`), and `{"op": "texture_set", "path": "<folder or .zip of a PBR texture pack>", "nodes": [ids]}` makes materials from a whole pack (base colour, roughness, normal…) and puts the first on the parts. Images can be PNG, JPEG, TIFF, TGA, BMP, WebP, GIF, JPEG XL, DDS, PSD/PSB, SVG, EXR or HDR; HEIC and AVIF are not read, so ask for another format.
