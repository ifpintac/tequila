# Opening KeyShot scenes

Tequila opens KeyShot's scene files: a `.ksp` package (a ZIP of the scene
and the pictures it uses) and a bare `.bip`. Tequila reads them itself, from
the files; nothing of KeyShot's is needed or used.

## Opening and importing

- **Open** (File > Open, the start screen's Open, dropping the file on the
  window, `tequila -i scene.ksp`) makes a new project of the scene: its model
  where KeyShot had it, its materials, lighting and cameras, looking through
  the view it was saved in. Nothing is saved until you save it as a
  `.tequila` project; the KeyShot file is never written to.
- **Import** (File > Import, the scripting `import`) adds the scene's model
  to the project that is open, with its material variants, its model set and
  its cameras. The open project keeps its own lighting and look, and the
  model is set on the floor as any import is, its cameras moved with it.

What was approximated is listed in the import's warnings: the status line
shows the first, and the console lists them all.

## What comes over

| KeyShot | Tequila |
|---|---|
| Model set, models, layers, objects | Groups in the Scene tab, with their names; the model set becomes a Model Set |
| A hidden model (a design variant) | A hidden group |
| An object's trimmed NURBS faces | One part, or one part per material when its faces wear several, divided at the import quality (Medium; Scene tab > Tessellation divides it again) |
| The faces' boundaries | Model edges (View > Model edges, wireframe renders' Model edges) |
| An object that is only a mesh | That mesh, with its normals and UVs |
| Plastic | Plastic |
| Metallic Paint | Metallic Paint: base, flakes (colour, coverage, roughness, size), clear coat (colour, roughness, index) |
| Metal | Metal: its colour, or its measured preset (Aluminum, Chromium, Copper, Gold, Iron, Nickel, Platinum, Silver, Titanium measured; Brass, Magnesium, Niobium, Zinc fitted), an anodised film as a thin film |
| Glass | Solid glass, coloured by the distance light travels in it |
| Emissive | A glowing material at KeyShot's intensity, its colour from its picture |
| Label on a material | A Tequila label of its picture, placed where KeyShot placed it |
| Emissive label | The glow of the material it is printed on, coloured by its picture where it is placed |
| Bump map, Noise (Fractal) | Bump, from the picture or the noise |
| Image textures | The same pictures, planar, box, cylindrical, spherical or UV as KeyShot had them; a `.ksp`'s pictures are written beside it in `<name>.textures` |
| Multi-material | Material variants (Material tab), the one KeyShot showed active |
| HDRI-editor environment | The environment: its background colour over the sphere and its pins over that, brightness, contrast, rotation, dome size, floor settings |
| Each camera | A saved camera: position, direction, focal length (from KeyShot's horizontal field of view on 36 mm film), orthographic frame, depth of field (f-number, focus distance, blades) |
| The view the scene was saved in | The active camera, "Viewport" |
| Environments and cameras | A Studio per camera, with the environment named after it where there is one; an environment no camera is named after is a Studio of its own |
| Image style (Photographic) | Image style: exposure, contrast, saturation, curve (shadows, highlights, whites), bloom, vignette, chromatic aberration, the background colour |
| Render settings | Render size, DPI, samples, denoising |

## What is approximated

KeyShot does not document its scene format, and some of what a scene stores
has no counterpart here or a meaning that cannot be read from the files with
certainty. The reader takes the best reading and says so:

- **The floor** is where KeyShot's was, at the scene's height zero. A product
  KeyShot showed floating stays floating (KeyShot's own renders of such scenes
  show no shadow under it).
- **Pins**: their position is read as azimuth and elevation in degrees, and
  their size as degrees across; KeyShot's own units for pin sizes are not
  known. Gradient pins come in as a fade along their length. Half pins come in
  whole, rounded corners square. The blend mode is taken as adding light.
- **Roughness** is taken as KeyShot stores it. KeyShot may remap its slider
  before storing; the values as they are agree best with KeyShot's own render
  of the same scene.
- **Colours** are read as sRGB, as KeyShot's colour picker shows them.
- **Glass**: the distance its colour is reached at is read in metres.
- **Bump strength**: a picture's bump is as deep as a thousandth of its
  repeat per unit of height, a noise's as deep as its features are wide.
- **Which way faces look**: a scene keeps no face's orientation. Each face is
  turned to agree with KeyShot's own mesh of its object; where there is none,
  by whether it looks out of its body.
- **Metal presets** are read by their place in KeyShot's list, 0 being its
  colour mode.
- **Image style**: KeyShot's tone mapping, white balance, vibrance, the curve's
  midtone and light points, bloom threshold and front plate have no
  counterpart; Tequila's own display transform is kept.
- **Not read**: rounded edges on meshes, emissive visibility switches,
  flake visibility, clear-coat thickness, studios and configurations (none of
  the scenes this was built from had any), animation, and environments that
  are an HDR picture rather than made in the HDRI editor (the picture is not
  in the scene).

## Rendering a KeyShot scene

Tequila's path tracer is not KeyShot's. A scene renders with the samples
KeyShot was set to, which are not the same amount of work in both; the
command line's `-s` sets them. Light reaches what is under glass only by
passing through it, so a part under a glass cover takes many more samples to
clear than in KeyShot.
