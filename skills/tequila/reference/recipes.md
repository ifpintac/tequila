# Materials of your own: recipes

When nothing in the library fits - terrazzo in the brand's colours, a galaxy resin, worn paint, a camouflage, speckled plastic - make the material from a **recipe**: a short JSON object of a type, the parameters it changes, and patterns. Tequila builds the node graph from it. `tequila_material_guide` is the full description (every pattern, its settings and defaults, the types, and 16 tested examples); read it once per conversation before writing recipes. This page is the working summary.

## The loop

1. **Start from an example** in the guide, or read a library preset already on a part with `tequila_material_recipe {"material": <id>}`, and change it.
2. **Preview** with `tequila_material_preview {"recipe": {...}}`: you get a picture of it on a 5 cm ball, the scene untouched. Set `radius` to the part's real size (from `tequila_nodes`) when the pattern's scale matters. Look, adjust, preview again.
3. **Create** with `tequila_material_create {"recipe": {...}, "nodes": [ids]}`: made and assigned as one undo step; it answers with the material id and a picture.
4. **Look on the product** (`tequila_screenshot`, rendered viewport), then **update** with `tequila_material_update {"material": <id>, "recipe": {...}}` as often as needed - every part using it changes.
5. **Save** it to the user's library with `tequila_material_save` when they like it (full connection only).

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

- `type`: the closest real material type (`plastic`, `metal`, `glass`, `paint`, `velvet`, … - the guide lists them). `metal_preset` picks a measured metal for `metal`: Aluminium, Silver, Gold, Copper, Chromium, Iron, Nickel, Titanium, Tungsten, Platinum.
- Any parameter as a fixed value, by its key. **Colours as hex sRGB** (`"#c0392b"`).
- `patterns`, each driving one parameter (`param`): `base_color`, `specular_roughness`, `base_metalness`, `bump`, `coat_weight`, `transmission_weight`, `thin_film_thickness`, `geometry_opacity`, `emission_luminance`…
  - `procedural`: `{"kind": ..., settings}`; settings left out take their defaults.
  - `size`: the feature size **in metres at real-world scale** (0.01 = a 1 cm pattern). Prefer it to `frequency`.
  - `seed`: a different random layout. The same kind, settings and seed line up exactly - give scratches both a colour and a metalness that way.
  - `colors` (colour parameters): a ramp of stops the pattern's 0-1 value is read through. `range` (number parameters): what 0 and 1 become. `depth` (bump): how high 1 stands, in metres - 0.00005-0.0005.
- **Layers:** a later pattern on the same parameter goes over what is there: `blend` (`mix`, `multiply`, `add`), `amount` 0-1, and `mask` - `{"threshold": 0.6, "softness": 0.03}` shows it where its own pattern is above 0.6; add `procedural`/`size`/`seed` to cut it from another pattern; `"invert": true` flips it.

## Patterns

noise, noise_texture, ridged, cellular, spots, granite, marble, wood, wood_advanced, brushed, brushed_radial, scratches, leather, weave, camouflage, contour, mesh. What each looks like, what it is good for and its settings are in the guide. The traps:

- noise-like patterns and marble run smoothly 0-1; **spots, mesh and contour are 1 on the feature**; **scratches are 1 except in a scratch** (so wear layers invert their mask, and a metalness range runs `[1, 0]`); weave and leather are heights.
- marble's own frequency is radians per metre - give `size` and it is converted for you.
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

The guide has the rest: terrazzo, Carrara and Nero Marquina marble, brushed aluminium, carbon fibre, pebbled leather, concrete, hammered copper, galaxy resin, soft-touch rubber, desert camo, oil-slick iridescence, grey felt and a speaker grille.
