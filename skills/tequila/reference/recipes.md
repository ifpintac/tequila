# Materials of your own: recipes

When nothing in the library fits - terrazzo in the brand's colours, a galaxy resin, worn paint, a camouflage, speckled plastic - make the material from a **recipe**: a short JSON object of a type, the parameters it changes, and patterns. Tequila builds the node graph from it. Search the library first (page `materials`): a preset changed a little nearly always beats a material from nothing.

This page is enough to write recipes. `tequila_material_guide` has the rest, a part at a time: `{"topic": "leather"}` one pattern's settings and defaults, `{"topic": "Terrazzo"}` one of 16 tested recipes, `{"topic": "examples"}` what they are.

For a library preset in another colour you do not need a recipe at all: `tequila_material_tint` (page `materials`) recolours it in one step and keeps its pattern.

## The loop

1. **Start from an example** below, or read a library preset already on a part with `tequila_material_recipe {"material": <id>}`, and change it.
2. **Preview** with `tequila_material_preview {"recipe": {...}}`: you get a picture of it on a 5 cm ball, the scene untouched. Set `radius` to the part's real size (from `tequila_nodes`) when the pattern's scale matters. Look, adjust, preview again.
3. **Create** with `tequila_material_create {"recipe": {...}, "nodes": [ids]}`: made and assigned as one undo step; it answers with the material id and a picture.
4. **Look on the product** (`tequila_screenshot`, rendered viewport), then **update** with `tequila_material_update {"material": <id>, "recipe": {...}}` as often as needed - every part using it changes.
5. **Save** it to the person's library with `tequila_material_save` when they ask.

## A recipe

```json
{
  "name": "Speckled Stone",
  "type": "plastic",
  "specular_roughness": 0.3,
  "base_color": "#d8d4cc",
  "patterns": [
    {"param": "base_color", "procedural": {"kind": "noise"}, "size": 0.01,
     "colors": [{"at": 0, "color": "#d8d4cc"}, {"at": 1, "color": "#a9a59c"}]},
    {"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.02, "range": [0.25, 0.4]},
    {"param": "bump", "procedural": {"kind": "leather"}, "size": 0.002, "depth": 0.0001}
  ]
}
```

- `type`: the closest real material type (`plastic`, `metal`, `glass`, `paint`, `velvet`… - page `materials` lists them). `metal_preset` picks a measured metal for `metal`: Aluminium, Silver, Gold, Copper, Chromium, Iron, Nickel, Titanium, Tungsten, Platinum.
- Any parameter as a fixed value, by its key. **Colours as hex sRGB** (`"#c0392b"`).
- `patterns`, each driving one parameter (`param`): `base_color`, `specular_roughness`, `base_metalness`, `bump`, `coat_weight`, `transmission_weight`, `thin_film_thickness`, `geometry_opacity`, `emission_luminance`…
  - `procedural`: `{"kind": ..., settings}`; settings left out take their defaults.
  - `size`: the feature size **in metres at real-world scale** (0.01 = a 1 cm pattern). Prefer it to `frequency`.
  - `seed`: a different random layout. The same kind, settings and seed line up exactly - give scratches both a colour and a metalness that way.
  - `colors` (colour parameters): a ramp of stops the pattern's 0-1 value is read through. `range` (number parameters): what 0 and 1 become. `depth` (bump): how high 1 stands, in metres - 0.00005-0.0005.
- **Layers:** a later pattern on the same parameter goes over what is there: `blend` (`mix`, `multiply`, `add`), `amount` 0-1, and `mask` - `{"threshold": 0.6, "softness": 0.03}` shows it where its own pattern is above 0.6; add `procedural`/`size`/`seed` to cut it from another pattern; `"invert": true` flips it.

## Patterns

Each is `{"kind": "<name>", ...settings}`; settings left out take sensible defaults, and `size` beside it (metres) replaces `frequency`.

| Kind | Looks like | Settings worth changing |
|---|---|---|
| `noise` | Soft fractal clouds: mottling, variation, dirt | `octaves` 1-8, `gain` |
| `noise_texture` | Blotches, hard-edged with contrast | `contrast` 1 soft to 4+, `bias` |
| `ridged` | Sharp crests and creases | `octaves`, `gain` |
| `cellular` | Voronoi cells | `output`: `distance` (dimples), `edges` (cracks), `id` (a random value per cell) |
| `spots` | A round spot per cell, 1 in a spot | `coverage`, `variance`, `softness`, each 0-1 |
| `granite` | Fine grain and flecks | `speckle`, `fine`, each 0-1 |
| `marble` | Bands bent into veins | `distortion` 0 stripes, 2-4 marble; `axis` |
| `wood` | Growth rings with grain | `axis` (the grain's direction), `distortion`, `grain` 0-1 |
| `wood_advanced` | Real boards: ring widths, knots, planks | `latewood`, `contrast`, `flecks`, `knots`, `board_width` (metres) |
| `brushed`, `brushed_radial` | Parallel streaks; streaks in circles about an `axis` | `stretch` 10-60 |
| `scratches` | Thin scratches, 0 in a scratch | `width`, `density` 0-1, `spread` 0 parallel to 0.25 random |
| `leather` | Pebbled grain with cracks (a height) | `crack_width`, `wrinkle`, each 0-1 |
| `weave` | Threads over and under (a height) | `pattern`: `plain`, `twill`, `satin`; `width` 0-1 |
| `camouflage` | Stacked blotches in steps | `layers` 2-5 |
| `contour` | Lines at even heights | `width`, `softness`, `distortion` |
| `mesh` | A perforated sheet, 0 in a hole | `shape`: `round`, `square`, `hex`; `size` of the hole |

The traps:

- noise-like patterns and marble run smoothly 0-1; **spots, mesh and contour are 1 on the feature**; **scratches are 1 except in a scratch** (so wear layers invert their mask, and a metalness range runs `[1, 0]`); weave and leather are heights.
- weave and mesh are flat: they lie in the part's own x-z plane.
- Chips of different colours (terrazzo, confetti): colour a layer with `cellular` `"output": "id"` through a stepped ramp, masked by `spots` of the **same size and seed** - they share their cells.

## Rules of thumb

- **Real-world scale is everything**: wood rings 2-5 mm apart (`size` 0.003), carbon weave 2 mm, terrazzo chips 3-15 mm, brushing 0.1 mm across. Check the part's size first; preview with `radius` near it.
- Realism comes from **roughness variation and a little bump**, more than from colour.
- Natural materials: keep colour ramps close (a few shades of one hue); strong contrast reads as a print.
- Layer, do not replace: base, then flecks or veins under a mask, then wear.
- A clear coat (`coat_weight` 0.5-1, `coat_roughness` 0.02-0.1) makes wood, carbon and paint read as finished.
- A uniform preview means the pattern is far too small or too large for the ball: change `size`.
- An error names the field that is wrong: fix that field and send it again.

## Two tested examples

Walnut under a satin clear coat:

```json
{"name": "Walnut", "type": "plastic", "specular_roughness": 0.45, "coat_weight": 0.6, "coat_roughness": 0.12, "patterns": [
  {"param": "base_color", "procedural": {"kind": "wood", "axis": "x", "distortion": 0.1, "grain": 0.5}, "size": 0.004,
   "colors": [{"at": 0, "color": "#80593a"}, {"at": 0.55, "color": "#5a3a23"}, {"at": 1, "color": "#36220f"}]},
  {"param": "bump", "procedural": {"kind": "wood", "axis": "x", "distortion": 0.1, "grain": 0.5}, "size": 0.004, "depth": 2e-05}]}
```

Red paint worn to bare steel (the same seed puts colour and metalness on the same scratches):

```json
{"name": "Worn Red Paint", "type": "plastic", "base_color": "#a3201b", "specular_roughness": 0.35, "patterns": [
  {"param": "base_color", "procedural": {"kind": "scratches", "width": 0.08, "density": 0.7, "spread": 0.25}, "size": 0.008, "seed": 5,
   "colors": [{"at": 0, "color": "#b9bdc2"}, {"at": 1, "color": "#9a9ea3"}], "mask": {"threshold": 0.6, "softness": 0.1, "invert": true}},
  {"param": "base_metalness", "procedural": {"kind": "scratches", "width": 0.08, "density": 0.7, "spread": 0.25}, "size": 0.008, "seed": 5, "range": [1, 0]},
  {"param": "specular_roughness", "procedural": {"kind": "noise_texture", "contrast": 2}, "size": 0.02, "range": [0.3, 0.5]}]}
```

`tequila_material_guide {"topic": "Walnut"}` has each of the rest, by its name: terrazzo, Carrara and Nero Marquina marble, brushed aluminium, carbon fibre, pebbled leather, concrete, hammered copper, galaxy resin, soft-touch rubber, desert camo, oil-slick iridescence, grey felt and a speaker grille.
