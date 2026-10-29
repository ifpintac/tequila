# The `.tequila` file format

The reference for reading and writing Tequila projects, as required by
`03-SCENE-AND-IO.md` §4.2 ("Publish the full JSON Schema at
`docs/file-format.md` and validate on load").

This describes **format 1.0.0**, which is what the current build writes.

> **This document is checked against the code.** `documented_fields_match_the_writer`
> in `crates/tequila-format/tests/documentation.rs` reads the field tables
> below and compares them to what the writer actually emits. A field added to
> one and not the other fails the build. If you are reading this after changing
> the format and the test is red, this file is what needs updating.

---

## 1. Container

A `.tequila` file is a **ZIP archive**, with every member **stored rather than
deflated**. Storing costs disk space and buys two things: a `.fsmesh` member can
be memory-mapped and handed to the ray tracer without a copy, and the writer
needs no compressor.

```
project.tequila
├── manifest.json              what is inside, and what wrote it
├── scene.json                 the scene: hierarchy, units, environment
├── materials/
│   └── mat_00000001.json      one file per material
├── geometry/
│   └── m_00000001.fsmesh      one file per mesh, binary
└── sources/
    └── <id>.step              a CAD file kept with the project (.iges for IGES)
```

A `sources/` member is a CAD file's own bytes (`.step` or `.iges`, as the file
was), kept so a model can be divided
into triangles again at another quality, or its edges drawn, without the file
(`docs/03` §6.2's B-rep retention). Its `<id>` is the FNV-1a hash of its
content and its length (`<hash>-<length>`, hexadecimal), named by the
`B-rep` metadata field of the model's top nodes, which also record the
`Tessellation` (`Draft`, `Medium`, `High` or `Ultra`) it was divided at. Only
sources some node still names are written. Files over 64 MB are not kept; their
models divide again from the file they came from.

Members not listed here are **preserved unchanged** through a load and save (see
§5). That is how a file written by a newer Tequila survives being opened in an
older one.

### 1.1 Determinism

Saving the same project twice produces **byte-identical** files. This is a
deliberate property, not a coincidence, and it is what keeps a project in
version control from churning on every save. It requires:

- a fixed timestamp on every ZIP entry, never the wall clock;
- members written in a fixed order;
- JSON object keys sorted;
- numbers written at their own precision — an `f32` widened to `f64` and printed
  in full is exactly correct and completely unreadable.

A reader may rely on none of this. A writer must provide all of it.

---

## 2. `manifest.json`

| Field | Type | Meaning |
|---|---|---|
| `format_version` | string | Semver for the format. See §5. |
| `application_version` | string | The Tequila build that wrote the file. Informational. |
| `geometry` | string[] | Every mesh id present, matching a `geometry/<id>.fsmesh` member. |
| `materials` | string[] | Every material id present, matching a `materials/<id>.json` member. |

```json
{
  "application_version": "0.1.0",
  "format_version": "1.0.0",
  "geometry": ["m_00000001", "m_00000002"],
  "materials": ["mat_00000001"]
}
```

---

## 3. `scene.json`

The document itself, minus the heavy binary data. Human-readable and diffable on
purpose: a change to one material produces a one-file diff rather than rewriting
the whole project.

### 3.1 Top level

| Field | Type | Meaning |
|---|---|---|
| `format_version` | string | Semver, matching the manifest. |
| `name` | string | The project's name. Not a file path — a project that has never been saved still has one. |
| `units` | object | §3.2. |
| `environment` | object | §3.3. |
| `lighting` | object | §3.3.1. |
| `cameras` | object[] | §3.3.2. Always at least one. |
| `active_camera` | number | Index into `cameras` of the one the viewport looks through. |
| `image_style` | object | §3.3.3. |
| `render` | object | §3.3.4. |
| `color_management` | object? | `{config, display, view}`, naming an OpenColorIO config to take the display transform from instead of the built-in one. Null — the default — means the built-in transforms. A path and two names, never the config's contents: a config is a studio's file, shared across projects and updated independently, and a project that embedded a copy would keep developing through last year's version of it. |
| `measurements` | object[]? | Dimensions annotated on the model, `[{a, b, visible}]`. Each end is `{node?, point}`: a point in that node's own space, so the dimension follows the part when it moves, or in the world when `node` is absent. An end whose node has been deleted leaves the measurement kept but not drawn. Omitted when there are none. |
| `multi_materials` | object[]? | Material variants (`docs/02` §8.1): `[{id, name, variants, active, parts}]`. `variants` are material ids, `active` an index into them, `parts` node ids; the active variant is also each part's own `material`, which is what every reader uses. Omitted when there are none. |
| `model_sets` | object? | `{sets, active?}`: `sets` is `[{name, nodes, always_visible}]`, `active` the index of the set shown, absent to show every set. A node in some set is shown only when one of its sets is active or always visible; a node in none is always shown. Omitted when there are no sets. |
| `material_templates` | object[]? | Rule-based assignment (`docs/04` §3.3): `[{name, rules}]`, each rule `{match, material, …}`. `match` is `name` or `material` with a `pattern` (a case-insensitive wildcard: `*` any run, `?` any one character), `colour` with `color` and `tolerance` (the current material's base colour, which is where a CAD colour lands), or `metadata` with a `key` and a value `pattern` — a CAD layer is a metadata field. The first rule to match a part decides it. Omitted when there are none. |
| `studios` | object[]? | Saved shots (§1.12): `[{name, camera?, environment?, image_style?, model_set?, variants}]`. Each part present is applied; an absent one is left alone. `camera` is a `cameras` entry, `environment` and `image_style` are as at the top level, `model_set` is an index or null for "show every set", and `variants` is `[{multi_material, active}]`. Omitted when there are none. |
| `configurator` | object? | The product configurator (§1.13): `{title, subtitle, placement, options, rules, current}`. `placement` is `left`, `right` or `bottom`, where the picker sits in Presentation mode. Each option is `{id, name, drives, style, choices}`: `drives` is `material` (with `multi_material`, an id; a choice's `target` is a variant index), `model_set` (`target` is a set index) or `studio` (`target` is a Studio index); `style` is `swatches`, `buttons` or `list`; each choice is `{label, target, swatch?}` with `swatch` a linear RGB triple, absent to take the variant's colour. Each rule is `{option, choice, kind, other, other_choice}` (options by id, choices by index) with `kind` `excludes` or `requires`. `current` is the choice shown for each option, in order. Omitted when there is no configurator. |
| `tree` | object[] | Every node, in depth-first order. §3.4. |
| `roots` | number[] | Node ids with no parent, in order. |
| `next_node_id` | number | The next node id to hand out. §3.6. |
| `next_mesh_key` | number | The next mesh key to hand out. |
| `next_material_key` | number | The next material key to hand out. |
| `next_texture_id` | number | The next texture id to hand out. |
| `textures` | object[] | Texture references: `{id, path, role, color_space?}`. See below. |

### 3.2 `units`

| Field | Type | Meaning |
|---|---|---|
| `unit` | string | `Unspecified`, `Millimetre`, `Centimetre`, `Metre`, `Inch` or `Foot`. |
| `metres_per_unit` | number | Always written as 1: every length in the file is in metres, and `unit` is only how the application shows them. Files written before 2026-10-13 may hold another factor; a reader scales such a file into metres when it opens it. Absent means 1. |

A reader also accepts the American spellings (`Millimeter`, `Centimeter`,
`Meter`), since §4.2's own example uses them. A writer emits only the forms
above, so the round trip stays exact.

### 3.3 `environment`

| Field | Type | Meaning |
|---|---|---|
| `image` | string? | Path to an HDR or EXR. Absent means the built-in studio lighting. |
| `rotation` | number | Radians about the vertical axis. |
| `brightness` | number | Multiplier. |
| `contrast` | number | Power about the image's mean luminance. 1 leaves it alone. |
| `size` | number | Diameter of the environment dome, in scene units. 0 means infinitely distant, which is the usual assumption and the one that makes lighting independent of where a part stands. |
| `height` | number | Vertical offset of the dome's centre relative to the ground. |
| `background` | object | `{mode, color, image}`. `mode` is `Environment`, `Color` or `Backplate`; `color` is used by the first, `image` — a path, or null — by the last. |
| `ground` | object | `{enabled, shadows, shadow_color, occlusion_shadows, reflections, reflection_blur, reflection_length, reflection_fade_distance, reflection_fresnel, reflection_alpha, bounce, bounce_strength, bounce_color, bounce_contact_falloff, flatten, size}`. The shadow catcher of §1.5. `enabled` is the master switch — false means there is no floor in the scene at all, which is how a product is rendered in mid-air; the other fields describe a floor that exists. `reflections` is 0 to 1, `size` is in scene units with 0 meaning "fit the scene". The reflection fades out by `reflection_fade_distance` metres above the floor when that is above 0 (V-Ray's Dim distance, the same under any product), and otherwise by `reflection_length`, a share of the product's height (1 = no fade). `reflection_alpha` is `coverage` (the default: the reflection's alpha is its share of the pixel) or `luminance` (its alpha follows its brightness as displayed, so a dark reflection is transparent and never lays a grey band on the page; Cycles renders). `bounce` gives the floor a glow only the product sees — its reflections and the light it bounces — never the camera: `bounce_strength` in nits, `bounce_color`, and `bounce_contact_falloff`, metres over which it darkens toward the contact line (0 = none). |
| `sky` | object? | A physical sky, which replaces `image` when present. Absent means none. |
| `pins` | array? | The HDRI Editor's layer stack, painted over the base in array order (§3.3.5). Absent when empty, so a project from before pins and one without any are the same bytes. |

A reader also accepts a top-level `lighting_preset` string, which is what
documents written before the Lighting tab existed carry instead of `lighting`.
A writer never emits it.

Every field above is written and read through one table in the command bus
(`EnvParam`), so a new control on the Environment tab appears here without the
reader and the writer being edited separately.

### 3.3.1 `lighting`

| Field | Type | Meaning |
|---|---|---|
| `preset` | string? | `Performance`, `Basic`, `Product`, `Interior` or `Jewelry`. **Null means Custom** — Custom is modelled as the absence of a preset rather than as a sixth one, so a document can say both which preset was chosen and that the scene no longer matches it. |
| `custom` | object | The integrator configuration Custom uses. Kept even while a preset is selected, so switching away and back does not discard a tuned setup. Field names match `IntegratorConfig`. |
| `custom.cycles` | object? | Blender's Light Paths for Cycles, written only when one is set: `{diffuse_bounces, glossy_bounces, transparent_bounces, volume_bounces, filter_glossy, caustics_reflective, caustics_refractive}`, each optional. Unset, Cycles gets what it always did (diffuse and glossy = `max_bounces`, at least 4; transparent 8; Filter Glossy 1; its own volume bounces and caustics). With diffuse or glossy set, `max_bounces` is the total, as Blender's Total is. |

### 3.3.2 `cameras` entries

| Field | Type | Meaning |
|---|---|---|
| `name` | string | Shown in the camera list. |
| `position` | number[3] | The eye, in world space. |
| `target` | number[3] | What the camera orbits and focuses on. Kept so a restored view can be orbited, not only looked through. |
| `up` | number[3] | The camera's up axis. |
| `basis` | number[9] | The right, up and forward axes, in that order. Redundant with `position`/`target`/`up`, and present because those three are a *lossy* encoding of an orthonormal basis: rebuilding one costs a cross product and a normalize, which lands within a few ULPs rather than exactly, and §2 requires a reopened document to save byte-identically. A reader uses `basis` when it is present and falls back to the readable triple when it is not. |
| `type` | string | `Perspective`, `Orthographic`, `Shift` or `Spherical`. |
| `focal_length_mm` | number? | Present for `Perspective` and `Shift`. |
| `scale` | number? | Vertical extent in scene units. Present for `Orthographic`. |
| `shift_x`, `shift_y` | number? | Film-plane offset in fractions of the sensor. Present for `Shift`. |
| `sensor_width_mm` | number | What the focal length is quoted against. 36 is full frame. |
| `iso`, `shutter_speed_seconds` | number | The physical exposure of §01 §8.3. |
| `dof` | object | `{enabled, f_number, focus_distance, keep_focus, aperture, blades?, rotation?}`. `enabled` is false for a pinhole; `f_number` still carries the value the control returns to, because JSON has no infinity. `aperture` is `Circular` or `Bladed`. |

### 3.3.3 `image_style`

| Field | Type | Meaning |
|---|---|---|
| `type` | string | `Basic` or `Photographic`. Basic ignores the photographic fields below without discarding them, so switching between the two is reversible. |
| `exposure` | number | Stops. |
| `contrast`, `saturation` | number | 1 is neutral. |
| `highlights`, `shadows`, `whites`, `blacks` | number | -1 to 1, neutral at 0. |
| `temperature` | number | Kelvin. The image is corrected *as if* lit at this temperature, so a lower number makes it cooler. 6500 is neutral. |
| `tint` | number | -1 to 1, green against magenta. |
| `vignette`, `bloom`, `sharpening`, `chromatic_aberration` | number | 0 to 1, off at 0. |
| `bloom_radius` | number | Fraction of the image's smaller side. |
| `display_transform` | string | `AgX`, `ACES`, `PBR Neutral`, `sRGB` or `Raw`. |

An image style is never part of the render: §01 §9.3 classifies every field
here as re-developing the existing samples, so changing one costs a pass over
the accumulation buffer and no rays at all. The same is true of
`color_management`.

### 3.3.4 `render`

What a render is told before it starts (§1.8). None of it reaches the viewport:
these are read when Render is pressed and not before.

| Field | Type | Meaning |
|---|---|---|
| `resolution` | number[2] | Width and height in pixels. |
| `dpi` | number | For the print-size calculator. The renderer never reads it; a print size is a property of the job, and a user who set 300 DPI for a client should find it there tomorrow. |
| `max_samples` | number | Samples per pixel to stop at. |
| `max_seconds` | number? | A wall-clock limit, or null. **Whichever limit is reached first stops the render** — they are not alternatives: a sample count makes two renders comparable, a time limit makes a queue of forty finish before a deadline. |
| `adaptive` | boolean | Per-pixel variance halting (§01 §4.4). |
| `denoise` | boolean | |
| `denoise_strength` | number | 0 to 1. |
| `denoise_cadence` | number | Passes between denoises while rendering. 1 is every pass, which looks smooth and costs real time on a large image. |
| `prefilter_guides` | boolean | Filter the albedo and normal guides first. Worth it at low sample counts, where the guides are themselves noisy. |
| `reject_fireflies` | boolean | Outlier rejection (§01 §3.5). |
| `clamp_indirect` | number? | A hard ceiling on one sample's luminance, or null. Separate from rejection because it is a different trade: rejection is adaptive and keeps the estimator nearly unbiased, a clamp is blunt and removes energy the Clamped Energy pass then has to account for. §3.5 allows it only as an explicit choice. |
| `clamp_direct` | number? | A ceiling on direct light of its own, 0 for none, or null. Null, `clamp_indirect` holds for direct light too, as it always did; set, the two are apart, as Blender's are, and `clamp_indirect` (or firefly rejection's 10) is the indirect one alone. |
| `pixel_filter` | object? | `{type, width_px}`: `box`, `gaussian` or `blackman_harris`, in pixels as Cycles' Film takes them, or null for one pixel's box (both renderers' default; Blender's is Blackman-Harris 1.5). |
| `adaptive_threshold` | number? | The noise below which a pixel stops taking samples, or null for the renderer's own (Cycles 0.01). |
| `region` | number[4]? | `[x, y, width, height]`, or null for the whole image (§01 §9.5). |
| `format` | string | `png`, `jpeg`, `tiff`, `exr` or `psd`. |
| `depth` | string | `8`, `16` or `32`. A depth the format cannot hold is corrected on load rather than obeyed. |
| `alpha_mode` | string | `none`, `straight`, `premultiplied`, `aliased` or `silhouette` (§01 §12). `silhouette` cuts the picture out along the product's outline, like a clipping path. Glass keeps what it shows of the studio, and the floor and its shadow are left out. `none` writes every pixel opaque, the background part of the picture; it is the default, and what a project written before it existed and without the field reads as. |
| `jpeg_quality` | number | 1 to 100. |
| `write_passes` | boolean | Write the render passes beside the beauty image. Only EXR and PSD can hold them. |
| `output_directory` | string | Empty means "ask". |
| `filename_template` | string | `{project}`, `{camera}`, `{configuration}` (a configurator combination's choices, "Red - Leather"; empty otherwise), `{model_set}`, `{studio}`, `{frame}`, `{width}`, `{height}`, `{resolution}`, `{samples}`, `{date}`, `{time}`. An unknown field is left as written rather than dropped, so a typo is visible in the filename instead of producing two renders that overwrite each other. |
| `write_xmp` | boolean | Write an XMP sidecar beside the image. |
| `write_text_metadata` | boolean | Write a plain `name: value` sidecar. |
| `relight` | boolean | Keep an image per light group — each light layer, the environment, glowing materials — so the finished render can be relit in the Render window without rendering again. |
| `layers` | object[] | Render layers, `[{name, nodes, enabled}]`. Each enabled layer is rendered as an image of its own, `<image>_<name>`, beside the main one: its member nodes and their descendants seen by the camera, everything else still in the scene for shadows, light and reflections but not seen. The ground stays in the main image. |

The three optional fields are written as `null` rather than omitted, so "no
time limit" stays distinguishable from "a field this build does not know" —
without which §4.4's preservation rule would resurrect a limit the user
cleared.

Tequila reads a config's `FileTransform` (`.cube`, `.spi1d`, `.spimtx`),
`MatrixTransform`, `ExponentTransform`, `ExponentWithLinearTransform`,
`RangeTransform`, `LogTransform`, `CDLTransform`, `GroupTransform` and
`ColorSpaceTransform`. It does **not** implement OCIO v2's `BuiltinTransform`
registry, which is defined by the library's own C++ rather than by a file; a
config that uses one is reported as unsupported and the built-in transforms
stay available, rather than the image being quietly developed through an
incomplete pipeline.

### 3.3.5 `pins` entries

Each is a light, a picture or a gradient painted onto the environment sphere.

| Field | Type | Meaning |
|---|---|---|
| `name` | string | Shown in the panel. |
| `kind` | string | `light`, `image` or `gradient`. Absent, or a kind this version does not know, reads as `light`. |
| `image` | string | An `image` pin's picture, as a path. A picture that will not load paints nothing and is reported in the panel. |
| `stops` | array | A `gradient` pin's colours, `[{t, color}]`, `t` from 0 to 1 in any order. |
| `radial` | bool | A `gradient` pin's shape: out from its direction to the edge of a cap `angular_size[0]` wide (which may be the whole sphere), or, when false, bottom to top across its footprint. |
| `shape` | string | `rectangle` or `ellipse`. |
| `direction` | [x, y, z] | Where it points, in the environment's own space. |
| `angular_size` | [w, h] | Its size as seen from the centre, in radians. |
| `roll` | number | Rotation about its own direction, in radians. |
| `softness` | number | 0 is flat to the edge; 1 falls off from the centre. |
| `intensity` | number | Multiplies the colour, the picture or the gradient. |
| `color` | [r, g, b] | Linear tint. |
| `blend` | string | `add` (light on top) or `over` (covers what is under it, in proportion to its coverage and a picture's alpha). |
| `lighting` | bool | Lights the scene and shows in reflections. Absent reads as true. |
| `show_in_background` | bool | Seen by the camera behind the product. `lighting` false with this true is a backdrop. |
| `enabled` | bool | |

### 3.4 `tree` entries

| Field | Type | Meaning |
|---|---|---|
| `id` | number | Unique within the document, and never reused. §3.6. |
| `name` | string | What the Scene tab shows. Not unique. |
| `kind` | string | `Group`, `Part`, `Cutaway`, or one of the light shapes — `Area`, `Disc`, `Point`, `Spot`. A light's shape is its kind so the Scene tab can label the row without reading the light. |
| `parent` | number? | Absent for a root. |
| `transform` | number[16] | §3.5. |
| `visible` | boolean | This node's own flag. A node is drawn only if it *and* every ancestor are visible. |
| `locked` | boolean | Locked nodes refuse edits. Inherited the same way. |
| `children` | number[] | Child ids, in display order. |
| `mesh` | string? | Required when `kind` is `Part`; names a `geometry/<id>.fsmesh` member. |
| `light` | object? | Required when `kind` is a light shape. `{shape, color, intensity, intensity_unit, layer, enabled, visible_to_camera, affects_diffuse, affects_specular, casts_shadows, temperature?}` plus the shape's own fields: `width`/`height`/`two_sided` for `Area` and `Disc`, `radius` for `Point`, `inner_degrees`/`outer_degrees`/`falloff` for `Spot`. `intensity_unit` is `lm`, `nits` or `cd`. An optional `photometry` names a `.ies` file whose measured distribution replaces the shape's own falloff (§01 §5.1) — the path, never the file's contents, since a photometric file is a manufacturer's and is shared between projects. Written in the units the user set, never in the radiance the sampler wants: a file holding converted values would change meaning the day a conversion is corrected. A light's *placement* is the node's `transform` like any other node's — an `Area` or `Disc` panel spans local X and Y and emits along local +Z, the opposite of Blender's area lights; a `Spot` aims along local -Z. A `photometry` file aims where its shape does: +Z on a panel, -Z on a `Point` or `Spot`. |
| `cutaway` | object? | Required when `kind` is `Cutaway`. `{shape, enabled, cap, cap_color?}`: `shape` is `plane`, `box`, `sphere` or `cylinder`, in the node's own space — the half-space in front of `z = 0` along +Z, the cube from -1 to 1, the unit sphere, or radius 1 about Z from z = -1 to 1 — so the `transform` sizes and places it. `cap` is `own` (a cut face in the material of the part it cuts) or `color`, with `cap_color` linear RGB. Everything inside the volume is removed from the render and cut solids are capped (`docs/02` §3). A part whose material is of type Cutaway cuts with its own mesh instead, and has no node kind of its own. |
| `material` | string? | Names a `materials/<id>.json` member. Absent means unassigned. |
| `metadata` | object? | String-to-string. CAD attributes and user annotations, kept verbatim. |
| `path_key` | string? | The source file's stable identifier for this node, so a re-import can match it and keep its material. |
| `pivot` | number[3]? | A custom pivot in this node's local space, if one has been set. Absent means the gizmo uses a derived pivot. Stored because §04 §5 requires pivots to persist — they are what an animation track rotates about. |

Parent and child links must agree: if A lists B as a child, B must name A as its
parent. A loader that finds otherwise rejects the file rather than guessing.

### 3.5 Transforms

A node's transform is **sixteen numbers, row-major**, relative to its parent.

`03-SCENE-AND-IO.md` §4.2 sketches a `{t, r, s}` triple instead. That is not
used, and the reason is not stylistic: a translate/rotate/scale triple cannot
represent a shear, and nested non-uniform scales in an imported CAD assembly
produce one. Decomposing would be lossy — and a lossy transform does not merely
break the byte-identical round trip, it *moves the user's geometry*.

The inverse is not stored. It is derived on load, so a hand-edited matrix cannot
disagree with its own inverse.

A transform that cannot be inverted is rejected: it would collapse the part to
nothing and leave the renderer without the inverse it needs for surface normals.

### 3.6 Identifiers, and why the counters are in the file

`next_node_id`, `next_mesh_key` and `next_material_key` are not derivable from
the content, and they matter.

§2.3 requires that an id, once issued, is **never reused**. A recycled id lets a
stale material assignment or animation track silently re-attach to an unrelated
part. If the highest-numbered node in a project has been deleted, recomputing
the counter from what remains would hand that id straight back out on the next
edit. So the watermark travels with the file.

This is also why undo does not rewind the counters, and why "the document is
back to how it started" is a statement about the scene rather than about every
byte.

---

### 3.1 The `textures` list

A texture reference is where a file is and what it is used for — not its pixels,
which live in the renderer's bounded cache and are never written into the save
file (`docs/02-MATERIAL-SYSTEM.md` §5.1). References are small enough that one
file each would be archive noise without making anything more diffable, so they
sit here.

| Field | Type | Meaning |
|---|---|---|
| `id` | number | Referred to by `image_texture` nodes in a material's graph. |
| `path` | string | Relative to the project, or absolute. |
| `role` | string | `color`, `data`, `normal` or `height`. |
| `color_space?` | string | `srgb`, `linear_srgb` or `raw`, overriding the role's default. |

**The role is stored, not inferred.** §5.2 assigns colour space by map role
rather than by file format, and the role cannot be recovered from the file:
`roughness.png` and `colour.png` are the same kind of file and want opposite
treatment. `color_space` is an override for the rare case where someone knows
better than the role — a normal map mistakenly saved with a transfer curve —
which is why it is optional rather than the primary record.

The same file used in two roles gets two ids, because it decodes differently for
each; it is still one file to load.

## 4. `materials/<id>.json`

One file per material. Splitting them out is what keeps a scene with 500
materials diffable.

| Field | Type | Meaning |
|---|---|---|
| `id` | string | `mat_` followed by eight lowercase hex digits. |
| `name` | string | Shown in the library. The single source of the material's name. |
| `type` | string | Catalog entry, lowercase with underscores: `plastic`, `metal`, `glass_solid`, `area_light` and so on. See below. |
| `graph?` | object | The material graph, present only when there is more than the surface node. See §4.1. |

Then the OpenPBR parameter block of `docs/02-MATERIAL-SYSTEM.md` §2.2. Each key
is spelled exactly as the parameter is named there.

**Base**

| Field | Type | Meaning |
|---|---|---|
| `base_weight` | number | Overall strength of the base layer. |
| `base_color` | number[3] | Linear RGB. Diffuse albedo, or the conductor's reflectance when metallic. |
| `base_metalness` | number | 0 dielectric, 1 conductor. Interpolates. |
| `base_diffuse_roughness` | number | Oren-Nayar roughness. |

**Specular**

| Field | Type | Meaning |
|---|---|---|
| `specular_weight` | number | Strength of the dielectric interface. |
| `specular_color` | number[3] | Tint on the reflection. |
| `specular_roughness` | number | 0 to 1. |
| `specular_ior` | number | Index of refraction of the interface. |
| `specular_roughness_anisotropy` | number | -1 to 1. |
| `specular_rotation` | number | Anisotropy rotation, in turns. |
| `metal_ior` | object | `n` and `k`, each `number[3]`: the conductor's complex index of refraction. Written for every material, and ignored where `base_metalness` is 0. |

**Transmission**

| Field | Type | Meaning |
|---|---|---|
| `transmission_weight` | number | How much of the base transmits rather than scatters. |
| `transmission_color` | number[3] | Tint of what passes through. |
| `transmission_depth` | number | Attenuation distance in scene units. 0 means no volume. |
| `transmission_tint` | number[3] | The surface's own tint on light through it, in and out again: the square root at each crossing, as Cycles' principled glass takes its base colour. With a depth, on top of what the volume absorbs: a dyed plastic is coloured at its skin as well as through it. White by default. |
| `transmission_scatter` | number[3] | In-scattering albedo of the interior. |
| `transmission_scatter_anisotropy` | number | Henyey-Greenstein `g`, -1 to 1. |
| `volume_density` | number | How thick the medium is, as a multiple of what its colour, depth and scatter describe; scales absorption and scattering together. 1 by default; 0 is clear. A Volume Map node drives it to make the medium heterogeneous. Not an OpenPBR parameter. |
| `transmission_dispersion_abbe` | number | Abbe number. 0 disables dispersion. |
| `medium_priority` | number | Which medium is present where two overlap: the higher wins (a liquid over its glass). Whole numbers, 0 to 10. |

**Subsurface**

| Field | Type | Meaning |
|---|---|---|
| `subsurface_weight` | number | Translucency. |
| `subsurface_color` | number[3] | Linear RGB. |
| `subsurface_radius` | number[3] | Mean free path per channel, in scene units. |
| `subsurface_scatter_anisotropy` | number | -1 to 1. |

**Coat, fuzz and thin film**

| Field | Type | Meaning |
|---|---|---|
| `coat_weight` | number | Strength of the clear coat. |
| `coat_color` | number[3] | Tint of the coat. |
| `coat_roughness` | number | 0 to 1. |
| `coat_ior` | number | Index of refraction of the coat. |
| `coat_darkening` | number | 1 darkens the base physically; 0 leaves it alone. |
| `fuzz_weight` | number | Sheen strength. |
| `fuzz_color` | number[3] | Sheen colour. |
| `fuzz_roughness` | number | 0 to 1. |
| `thin_film_weight` | number | Strength of the interference film. |
| `thin_film_thickness` | number | Nanometres. |
| `thin_film_ior` | number | Index of refraction of the film. |
| `flake_weight` | number | Share of the base covered by metal flakes, 0 to 1. 0 is none. |
| `flake_color` | number[3] | Flake reflectance tint. |
| `flake_size` | number | Flake diameter in scene units. |
| `flake_spread` | number | How far flakes may lean, 0 (flat) to 1 (sixty degrees). |
| `flake_roughness` | number | Roughness of each flake's surface, 0 to 1. |
| `displacement_detail` | number | Edge length, in scene units, the surface is subdivided to before the graph's Displacement height moves it. 0 is bump-only. |

**Emission and geometry**

| Field | Type | Meaning |
|---|---|---|
| `emission_luminance` | number | Nits. |
| `emission_color` | number[3] | Linear RGB. |
| `geometry_opacity` | number | 0 to 1. |
| `geometry_thin_walled` | bool | A shell with no interior volume. |
| `geometry_occlusion` | number | 0 to 1; 1 is unoccluded. Baked ambient occlusion, usually driven by a map rather than set here. Not an OpenPBR parameter. |

Colours are **scene-linear**, never gamma-encoded, in the rendering space:
**ACEScg** primaries, not sRGB's. Neutral values are the same in both; a
saturated sRGB colour has to be converted (linear sRGB to AP1) to render as
itself, which is what the application's colour pickers and the glTF and OBJ
importers do. A value above 1 is legitimate for emission. The same holds for
library presets' colours and their pattern ramps.

### Why every parameter is written

A file records all of them, not only those differing from its `type`'s defaults.
Writing the differences would be smaller, but it would make a saved file's
meaning depend on the preset table of whichever version opens it: adjust the
Plastic preset in a later release and every Plastic material in every existing
file would quietly change. A document has to mean the same thing forever, so it
carries its own values.

`type` is therefore not needed to reconstruct the shading. It records which
catalog entry the material is an instance of, which is what the Material tab
uses to decide which parameters to show (`docs/04-UX-SPEC.md` §3). Reopening a
file gives back the panel the user was working in rather than all thirty-seven
controls.

### Reading a file written by an older version

A reader starts from the `type`'s defaults and overwrites with whatever the file
provides. A parameter that did not exist when the file was written gets the
value its type would have given it, rather than a global default that may make
no sense for that type. A parameter present but malformed — a colour where a
number belongs, a NaN — falls back the same way instead of failing the open.

### 4.1 The `graph` object

A material with no textures *is* a graph — one surface node whose literals are
the parameter block above (`docs/02-MATERIAL-SYSTEM.md` §4.2) — so a plain
material writes no `graph` key at all and its file is byte-for-byte what it was
before graphs existed.

When there is more, `graph` holds the rest:

| Field | Type | Meaning |
|---|---|---|
| `output` | number | Id of the surface node, so links naming it resolve on load. |
| `nodes` | object[] | Every node **except** the surface node, which is the flat block above. |
| `links` | object[] | Connections. |

A node:

| Field | Type | Meaning |
|---|---|---|
| `id` | number | File-local. Reassigned on load, and links are remapped with it. |
| `kind` | string | `constant`, `mapping_2d`, `image_texture`, `mix`, `multiply`, `add`, `invert`, `remap`, `clamp`, `node` (a Blender shader node), `group`, and the rest of `NodeKind`. |
| `position` | number[2] | Editor layout only; never affects rendering. |
| `label?` | string | User-assigned name. |
| `literals?` | object[] | Values for unconnected inputs: `{socket, value}`. |

Kind-specific fields: `value` on a constant, `mapping` on a mapping node,
`texture` and `fallback` on an image texture, `from`/`to` on a remap,
`low`/`high` on a clamp, `group` on a node group. A node group is one of
Tequila's built-in groups, named - `fdm_layer_lines`, a filament print's
surface - with its inputs as literals or links like any node's. What is
inside it is not written: it is the group as the Tequila opening the file
builds it, and one that does not know the name refuses the file.

A link is `{from, to, socket}`, where `socket` is `param:<parameter key>` for an
input on the surface node — the keys are the ones tabled above — or `input:<n>`
for any other node.

Three surface inputs exist only as link targets, with no field in the
parameter block: `bump` and `displacement` (heights in scene units) and
`geometry_tangent` (OpenPBR's: the direction an anisotropic lobe is rougher
in, before `specular_rotation` turns it, as a vector in the part's own space —
the space the Object texture coordinate is in). Unlinked, or zero where it is
linked, the surface's own tangent serves: along its UVs, else the frame its
normal gives.

A value is a single-key object naming its type: `{"float": 0.5}`,
`{"color": [r, g, b]}`, `{"vector": [x, y, z]}` or `{"bool": true}`.

**The parameter block is never written twice.** The surface node's literals live
in the flat block and nowhere else. Recording them inside `graph` as well would
be two sources of truth for the same values, and they would be discovered to
disagree as "the material changed when I reopened it".

A graph that does not validate — a link to a node that is not there, an unknown
node kind, a cycle — is refused rather than half-loaded. Opening a file into a
state the editor cannot display is worse than declining to open it.

---

## 5. `geometry/<id>.fsmesh`

Binary, little-endian, fixed layout, so it can be mapped and handed to the ray
tracer without parsing. The 52-byte header is a multiple of 4, which keeps every
array that follows 4-byte aligned without padding.

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | Magic, `FSMESH\0\0` |
| 8 | 4 | Version (currently 1) |
| 12 | 4 | Vertex count |
| 16 | 4 | Index count |
| 20 | 4 | Flags: bit 0 normals, bit 1 tangents, bit 2 colours, bit 3 edges |
| 24 | 4 | UV set count |
| 28 | 24 | Bounds: min xyz, max xyz |

Then, in order, each present only if its flag is set:

| Array | Size |
|---|---|
| Positions | `f32[3 × vertex_count]` |
| Normals | `f32[3 × vertex_count]` |
| Tangents | `f32[4 × vertex_count]` — `xyz` and a bitangent sign in `w` |
| UV sets | `f32[2 × vertex_count]`, repeated `uv_set_count` times |
| Colours | `f32[4 × vertex_count]` |
| Indices | `u32[index_count]` |
| Submesh count | `u32` |
| Submeshes | `{ first_index: u32, index_count: u32, slot: u32 }`, repeated |
| Edge segment count | `u32` (bit 3) |
| Edges | `f32[6 × segment_count]`: each segment's two end points, in the mesh's space (bit 3) |

The edges are the part's own - a CAD B-rep's, seams left out - which the Solid
view draws as Model edges. Last in the member, so a reader that does not know
bit 3 reads everything before them unchanged.

An **empty normal array means the mesh has no authored normals**, which is not
the same as having flat ones. The distinction is load-bearing: §6.1 forbids
regenerating normals that a CAD tessellator supplied, because regenerating them
makes cylinders faceted.

The stored bounds are **recomputed on load rather than trusted**. They are
derivable from the positions, and a stored value that disagreed would make the
renderer cull geometry that is really there.

A reader validates the declared sizes against the bytes actually present
*before* allocating for them, so a header claiming four billion vertices fails
cleanly instead of exhausting memory.

---

## 6. Versioning

`format_version` is semver, and a loader follows §4.4:

| The file's version | What happens |
|---|---|
| Same major, same or older minor | Loads. Migrations run silently and are logged. |
| Same major, **newer minor** | Loads. Fields this build does not understand are reported to the user and **preserved unchanged on save**. |
| **Newer major** | Refused, with a message naming both versions. |

The middle row is the one that matters. Without it, opening a project in an
older Tequila and pressing Save would silently delete whatever the newer
version had added. Preservation covers unknown top-level fields in `scene.json`,
unknown fields on individual nodes, and whole archive members this build does
not write.

### 6.1 What counts as which bump

- **Patch** — no format change; a bug fix in the reader or writer.
- **Minor** — new optional fields, new archive members, new enum values that
  have a sensible fallback. An older build must still be able to open the file
  and preserve what it does not understand.
- **Major** — anything an older build would misread rather than merely fail to
  understand: a field whose meaning changes, a required field, a different
  binary layout.

---

## 7. Rejecting a file

§4.2: *"Reject-with-explanation beats silently misparsing."*

A file is refused, with a message saying what was wrong, when:

- the archive is not a ZIP, or a member's checksum does not match its bytes;
- a member is compressed rather than stored;
- a member's name would escape its own directory;
- `scene.json` is absent or is not valid JSON;
- a node names a mesh or material that is not in the archive;
- parent and child links disagree, or the hierarchy contains a cycle;
- a transform has other than sixteen values, or cannot be inverted;
- a `.fsmesh` member's declared sizes do not fit the bytes present, or an index
  points past the vertex buffer;
- the format's major version is newer than this build's.

None of these produce a partially loaded document. A file either opens
completely or does not open.
