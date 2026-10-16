# Making materials for the library

Tequila's library has two halves: the **shipped presets**, read-only and the
same for everybody, and **your own**, in your user folder, which you can make,
change, share and delete. This guide covers every way to add to your own, the
file format a preset is written in, sharing materials, doing all of it from a
script, and how the shipped library itself is built.

## In the application

### New material

Click **+** at the top of the Library panel (or **File → New library
material…**) and pick where the material starts from:

| Start from | What you get |
|---|---|
| **Blank** | A material type's own physics (Plastic, Metal, Glass, Velvet, …) in a colour and roughness you pick. Add patterns and maps afterwards in the Material tab and the node editor. |
| **Selected part** | The material on the selected part exactly as it is now: its node graph, patterns and image maps. The maps are copied into your library, so it keeps working when the folder they came from is tidied away. |
| **Texture pack** | A PBR pack from ambientCG, Poly Haven, Megascans and the like — a folder, a ZIP, or one of its maps. Colour, roughness, metalness, normal and height maps become one material, laid tri-planar so it works on CAD parts without UVs. Set how much one repeat covers. |
| **File** | A material somebody shared: a Tequila material package (`.fsmat`) or a preset's `.json`. |

Give it a name, a category (pick one or type a new one) and tags to find it by,
and it lands on the **Mine** shelf. Tick *Put it on the selected parts* to use
it straight away.

### Changing your own

Right-click a tile:

- **Duplicate to my library** — works on shipped presets too; the copy is
  yours to change, and opens straight into *Edit details*.
- **Edit details…** — name, category, tags.
- **Update from the selected part** — the workflow for refining a preset: put
  it on a part, tune it in the Material tab or node editor, then save the
  result back over the preset, keeping its name.
- **Export…** — a `.fsmat` package to send to someone.
- **Delete…** — asks first; projects that use the material keep their own
  copy.

### Other ways in

- **Drop** a `.fsmat`, a preset `.json`, or a ZIP holding a `material.json`
  onto the window. (A ZIP of maps is a texture pack and goes on the
  selection, as before.)
- **Library ⋯ → Open the folder** — your presets are plain files; anything you
  copy in shows up within a couple of seconds, no restart needed.
- **Material tab → Save to library…** — the same as *Selected part*, from the
  material you are editing.

Your library lives in `%APPDATA%\Tequila\library` on Windows, in
`~/Library/Application Support/Tequila/library` on a Mac, and in
`~/.config/Tequila/library` on Linux (`$XDG_CONFIG_HOME` is honoured where
set), each preset a `.json` beside a `textures` folder holding its copies of
its maps.

## The preset file

A preset is a small JSON file: a material type, only the values that differ
from that type's defaults, and optionally a node graph. Recording only the
differences means a preset follows improvements to its type's defaults, and
stays readable and diffable.

```json
{
  "name": "Housing Blue",
  "type": "plastic",
  "category": "Plastic",
  "tags": ["abs", "satin", "housing"],
  "base_color": [0.045, 0.11, 0.42],
  "specular_roughness": 0.3
}
```

- `type` is a material type's id: `plastic`, `metal`, `anisotropic`, `paint`,
  `metallic_paint`, `glass`, `glass_solid`, `plastic_transparent`,
  `plastic_cloudy`, `translucent`, `velvet`, `generic`, … (`query api` lists
  them all with the parameters each exposes).
- Parameters use their OpenPBR keys (`base_color`, `specular_roughness`,
  `specular_roughness_anisotropy`, `coat_weight`, …; see
  [file-format.md](file-format.md)). Colours are linear, in the rendering
  space.
- Metals name their measured optics rather than giving numbers:
  `"metal_preset": "Aluminium"` (also Silver, Gold, Copper, Chromium, Iron,
  Nickel, Titanium, Tungsten, Platinum).

### Patterns, the short way

For hand-written presets, a `patterns` list drives a parameter with one of
Tequila's procedurals, optionally through a colour ramp, a range or a bump
depth:

```json
{
  "name": "Brushed Panel",
  "type": "anisotropic",
  "category": "Metal",
  "metal_preset": "Aluminium",
  "specular_roughness": 0.3,
  "specular_roughness_anisotropy": 0.9,
  "patterns": [
    { "param": "specular_roughness", "procedural": { "kind": "brushed", "frequency": 1400, "stretch": 80 }, "range": [0.26, 0.34] },
    { "param": "bump", "procedural": { "kind": "brushed", "frequency": 1400, "stretch": 80 }, "depth": 1.2e-6 }
  ]
}
```

### The full node graph

Anything the node editor makes is saved as a `graph`: Blender's shader nodes
by their keys (`noise_texture`, `voronoi_texture`, `wave_texture`, `math`,
`map_range`, `mapping`, `texture_coordinate`, …), Tequila's own procedurals,
colour ramps, bumps and image maps, wired into the surface's parameters with
`param:<key>` links. The easiest way to write one is to build it in the node
editor and save it; the format is in [file-format.md](file-format.md).

Three surface inputs exist only as link targets: `bump` and `displacement`
(heights in metres) and `geometry_tangent` — the direction an anisotropic
finish is brushed, as a vector in the part's own space, which makes brushed,
spun, turned and engraved metals follow their pattern on any part, UVs or
not.

### Image maps

A preset with pictures lists them under `textures`, by the ids its image nodes
use, with paths relative to the preset:

```json
"textures": [
  { "id": 1, "path": "textures/leather037/Leather037_1K-JPG_Color.jpg", "role": "color" },
  { "id": 2, "path": "textures/leather037/Leather037_1K-JPG_NormalGL.jpg", "role": "normal" }
]
```

Roles decide how a map is read: `color` maps are sRGB colour; `data` maps
(roughness, metalness, opacity, masks) are numbers, read as they are;
`normal` and `height` drive the bump.

## Sharing: material packages

A `.fsmat` is a ZIP holding `material.json` and its pictures under
`textures/`. Export one from the tile's menu; open one by dropping it on the
window or with *File → Import material file…*. Paths inside a package never
reach outside it: a package that names `../` or an absolute path is refused.

## From a script

The scripting channel and headless sessions have the same library
([scripting-api.md](scripting-api.md)): the `library` query lists every preset,
and the `library` operation's actions are `apply`, `save`, `import`, `export`
and `delete`. From Python:

```python
import tequila

with tequila.connect() as fs:                      # the open application
    metals = fs.library.presets(category='Metal')
    lid = fs.find(name='lid')[0]
    fs.library.apply('Aluminium Spun', lid)          # one undo step
    fs.library.save(fs.materials['Paint'], 'Signal Orange', category='Paint', tags=['safety'])
    fs.library.import_('C:/Downloads/Leather037_1K-JPG.zip', size=0.2)
    fs.library.export('Signal Orange', 'signal_orange.fsmat')
```

From the console under the viewport: `library`, `library apply Aluminium
Spun`, `library import "C:/shared/blue.fsmat"`.

## How the shipped library is made

The shipped presets are generated by code, never edited as JSON:
`crates/tequila-format/examples/standard_library/`. Each family has its
module (`metal.rs`, `metal_finish.rs`, `metal_pattern.rs`, `plastic.rs`,
`plastic_texture.rs`, `wood.rs`, …), written in a small recipe language over
Blender's nodes (`recipe.rs`), with shared pieces for finishes in `finish.rs`:
tri-planar laying of flat patterns, grooves that point the anisotropy, and the
lattices stampings repeat on.

To add one:

1. Write its recipe in the right module and add it to that module's `all()`.
   Sizes are real ones, in metres; colours are picked in sRGB with `srgb("#…")`.
2. `cargo run --release -p tequila-format --example standard_library`
   rewrites `assets/library/standard`.
3. Judge it: `TEQUILA_LAB_PRESETS="Its Name" TEQUILA_LAB_OUT=board.png cargo
   test -p tequila-app --release reference_board -- --ignored` renders it on
   the library's ball; add `TEQUILA_LAB_PACKS` (folders of a CC0 scan, with a
   `size_m.txt`) to see it beside the real thing.
4. `cargo test -p tequila-format library` checks every preset loads, is sane,
   uses only the parameters its type shows, and names no commercial swatch
   system; `cargo test -p tequila-app no_library_thumbnail_is_speckled` and,
   with Cycles installed, `library_presets_render_alike_in_cycles` check its
   preview and that both renderers agree.

Presets must not copy a commercial swatch system's names or values (Pantone,
RAL, Mold-Tech and the like); `assets/library/CREDITS.md` says where values
come from.
