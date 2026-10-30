# Scripting API reference

Generated from the command catalog by
`cargo run -p tequila-scripting --example bindings`; a test fails when it
falls behind the build. [SCRIPTING.md](SCRIPTING.md) is the guide: the
console, the protocol, and worked examples. This is the list.

## Sessions

The `tequila` Python module (the distribution's `python` folder, standard
library only) drives Tequila through the same command bus the interface
uses, so anything clickable is scriptable and every call is one undo step.

```python
import tequila

# A headless session: `tequila --serve`, no window, its own process.
with tequila.open('kettle.tequila') as fs:
    lid = fs.find(name='Lid')[0]
    red = fs.materials['Paint Base']
    red.base_color = [0.72, 0.05, 0.06]
    lid.assign(red)
    fs.render('red.png', width=1200, height=800, samples=256)
    fs.save('kettle-red.tequila')

# The application a person has open.
with tequila.connect() as fs:
    fs.commands.scene.rename(node=12, name='Lid')
```

`tequila.open(path=None)` starts `tequila --serve` with the project (or a
model to import, or an empty scene) and ends it when the session closes,
unsaved changes with it: save first. It looks for the command-line
renderer beside the `python` folder (`tequila-cli`), in a development
build's `target`, or where `TEQUILA_CLI` says. `tequila.connect()`
finds the running application through its discovery file.

Both answer the same protocol, one JSON request and reply per line over an
authenticated loopback socket. A headless session does what needs only
the document; what needs the window is refused by name.

## Operations

| Operation | Application | Headless |
|---|---|---|
| `ping` | yes | yes |
| `query` | yes | yes |
| `apply` | yes | yes |
| `batch` | yes | yes |
| `undo` | yes | yes |
| `redo` | yes | yes |
| `select` | yes | yes |
| `import` | yes | yes |
| `reimport` | yes | yes |
| `texture_set` | yes | - |
| `save` | yes | yes |
| `frame` | yes | yes |
| `look` | yes | yes |
| `screenshot` | yes | - |
| `render` | yes | yes |
| `queue` | yes | - |
| `export` | yes | yes |
| `livelink` | yes | - |
| `library` | yes | yes |
| `material` | yes | yes |
| `plugin` | yes | - |
| `studio_preset` | yes | yes |
| `stop` | yes | - |
| `ui` | yes | - |
| `new` | yes | yes |
| `open` | yes | yes |
| `quit` | yes | yes |

Queries (`{"op":"query","what":...}`): `api`, `context`, `scene`, `nodes`, `materials`, `selection`, `camera`, `commands`, `ui`, `jobs`, `history`, `library`; a headless session answers all but `ui` and `jobs`.

## Commands

57 commands, each applied as `{"op":"apply","command":{"command":<name>, ...}}`, or from Python as
`fs.commands.<group>.<verb>(field=...)`. Ids and keys in the examples are
placeholders: read the real ones from the scene first.

### material

#### `material.add_decal`

A label, an image projected onto a material. PNG, JPEG or Photoshop artwork;
or a video (MP4, MOV and the rest the system plays), sent with `time` in
seconds, whose frame then is the picture (kept under "Video frames" among
Tequila's settings).

`fs.commands.material.add_decal(material=..., name=..., path=..., role=...)`

```json
{ "command": "material.add_decal", "material": 1, "name": "Logo", "path": "logo.png", "role": "color" }
```

#### `material.set_decals`

A material's whole stack of labels at once.

`fs.commands.material.set_decals(decals=..., material=...)`

```json
{ "command": "material.set_decals", "decals": [ { "enabled": true, "mapping": { "axis": "z", "blend_sharpness": 6, "flip_u": false, "flip_v": false, "offset": [0, 0], "pivot": [0, 0], "projection": "uv", "real_world_units": false, "rotation": 0, "scale": [1, 1, 1], "uv_set": 0, "wrap_u": "repeat", "wrap_v": "repeat" }, "metallic": false, "name": "Logo", "opacity": 1, "roughness": 0.35, "texture": 1 } ], "material": 1 }
```

#### `material.assign`

One material on these nodes, linked: editing it changes every part it is on.

`fs.commands.material.assign(material=..., nodes=...)`

```json
{ "command": "material.assign", "material": 1, "nodes": [2] }
```

#### `material.unlink`

A node given its own copy of its material, so it can be edited alone.

`fs.commands.material.unlink(node=...)`

```json
{ "command": "material.unlink", "node": 2 }
```

#### `material.set_parameter`

One parameter of a material, by its key (`base_color`, `specular_roughness`,
...).

`fs.commands.material.set_parameter(material=..., parameter=..., value=...)`

```json
{ "command": "material.set_parameter", "material": 1, "parameter": "base_color", "value": [0.2, 0.4, 0.8] }
```

#### `material.rename`

A material's name.

`fs.commands.material.rename(material=..., name=...)`

```json
{ "command": "material.rename", "material": 1, "name": "Anodized" }
```

#### `material.set_graph`

The material's whole node graph, replaced.

`fs.commands.material.set_graph(definition=..., material=...)`

```json
{ "command": "material.set_graph", "definition": { "base_color": [0.5, 0.5, 0.5], "base_diffuse_roughness": 0, "base_metalness": 0, "base_weight": 1, "coat_color": [1, 1, 1], "coat_darkening": 1, "coat_ior": 1.5, "coat_roughness": 0.02, "coat_weight": 0, "displacement_detail": 0.0005, "edge_radius": 0, "edge_samples": 8, "emission_color": [1, 1, 1], "emission_luminance": 0, "flake_color": [0.9, 0.9, 0.9], "flake_roughness": 0.3, "flake_size": 0.00005, "flake_spread": 0.3, "flake_weight": 0, "fuzz_color": [1, 1, 1], "fuzz_roughness": 0.3, "fuzz_weight": 0, "geometry_occlusion": 1, "geometry_opacity": 1, "geometry_thin_walled": false, "graph": { "decals": [], "links": [ { "from": 2, "socket": "param:specular_roughness", "to": 1 } ], "nodes": [ { "id": 2, "kind": "constant", "position": [0, 0], "value": { "float": 0.35 } } ], "output": 1 }, "id": "mat_00000001", "medium_priority": 0, "metal_ior": { "k": [7.475, 6.4, 5.303], "n": [1.345, 0.965, 0.617] }, "name": "graph", "specular_color": [1, 1, 1], "specular_ior": 1.5, "specular_rotation": 0, "specular_roughness": 0.2, "specular_roughness_anisotropy": 0, "specular_weight": 1, "subsurface_color": [0.8, 0.8, 0.8], "subsurface_radius": [1, 1, 1], "subsurface_scatter_anisotropy": 0, "subsurface_weight": 0, "thin_film_ior": 1.4, "thin_film_thickness": 500, "thin_film_weight": 0, "transmission_color": [1, 1, 1], "transmission_depth": 0, "transmission_dispersion_abbe": 0, "transmission_scatter": [0, 0, 0], "transmission_scatter_anisotropy": 0, "transmission_tint": [1, 1, 1], "transmission_weight": 0, "type": "diffuse", "volume_density": 1, "volume_emission_color": [1, 1, 1], "volume_emission_strength": 0 }, "material": 1 }
```

#### `material.set_type`

A material's type (plastic, metal, glass, velvet, ...), keeping what carries
over.

`fs.commands.material.set_type(material=..., type=...)`

```json
{ "command": "material.set_type", "material": 1, "type": "glass_solid" }
```

#### `material.set_texture`

Drive a parameter from a texture, or stop driving it. This is §4.2's
"enabling the texture toggle next to a parameter inserts a Texture Map node
connected to that input" — one command, because from the user's side it is
one action and has to undo as one.

`fs.commands.material.set_texture(color_space=..., material=..., param=..., path=..., role=...)`

```json
{ "color_space": "srgb", "command": "material.set_texture", "material": 1, "param": "base_color", "path": "maps/panel.png", "role": "color" }
```

#### `material.set_procedural`

Drive a parameter from a procedural pattern rather than from a file. `None`
detaches, leaving the literal, exactly as a texture does.

`fs.commands.material.set_procedural(material=..., param=..., procedural=..., seed=...)`

```json
{ "command": "material.set_procedural", "material": 1, "param": "base_color", "procedural": { "basis": "perlin", "frequency": 4, "gain": 0.5, "kind": "noise", "lacunarity": 2, "octaves": 4 }, "seed": 7 }
```

#### `material.set_gradient`

The colour ramp a parameter's procedural is read through.

`fs.commands.material.set_gradient(gradient=..., material=..., param=...)`

```json
{ "command": "material.set_gradient", "gradient": [ { "at": 0, "color": [0.05, 0.05, 0.05] }, { "at": 1, "color": [0.8, 0.8, 0.8] } ], "material": 1, "param": "base_color" }
```

#### `material.set_mapping`

How a parameter's texture is laid onto the surface: the projection, the
scale, the rotation and the rest of §5.3's shared controls.

`fs.commands.material.set_mapping(axis=..., blend_sharpness=..., flip_u=..., flip_v=..., material=..., offset=..., param=..., pivot=..., projection=..., real_world_units=..., rotation=..., scale=..., uv_set=..., wrap_u=..., wrap_v=...)`

```json
{ "axis": "z", "blend_sharpness": 6, "command": "material.set_mapping", "flip_u": true, "flip_v": false, "material": 1, "offset": [0, 0], "param": "base_color", "pivot": [0, 0], "projection": "uv", "real_world_units": false, "rotation": 0.5, "scale": [1, 1, 1], "uv_set": 1, "wrap_u": "repeat", "wrap_v": "repeat" }
```

#### `material.set_multi_materials`

The whole list, and the active variant of each assigned to its parts.

`fs.commands.material.set_multi_materials(multi_materials=...)`

```json
{ "command": "material.set_multi_materials", "multi_materials": [ { "active": 0, "id": 7, "name": "Colourways", "parts": [2], "variants": [ "mat_00000001" ] } ] }
```

#### `material.set_templates`

The list of Material Templates. Applying one is ordinary assignments,
grouped.

`fs.commands.material.set_templates(templates=...)`

```json
{ "command": "material.set_templates", "templates": [ { "name": "Fasteners", "rules": [ { "match": "name", "material": "mat_00000001", "pattern": "bolt*" }, { "color": [0.25, 0.25, 0.25], "match": "colour", "material": "mat_00000001", "tolerance": 0.05 }, { "key": "layer", "match": "metadata", "material": "mat_00000001", "pattern": "HARDWARE" }, { "match": "material", "material": "mat_00000001", "pattern": "Steel*" } ] } ] }
```

### scene

#### `scene.import`

A model read from a file, its nodes, meshes and materials added as one step.

`fs.commands.scene.import_(payload=...)`

```json
{ "command": "scene.import", "payload": { "materials": [ { "base_color": [0.5, 0.5, 0.5], "base_diffuse_roughness": 0, "base_metalness": 0, "base_weight": 1, "coat_color": [1, 1, 1], "coat_darkening": 1, "coat_ior": 1.5, "coat_roughness": 0.02, "coat_weight": 0, "displacement_detail": 0.0005, "edge_radius": 0, "edge_samples": 8, "emission_color": [1, 1, 1], "emission_luminance": 0, "flake_color": [0.9, 0.9, 0.9], "flake_roughness": 0.3, "flake_size": 0.00005, "flake_spread": 0.3, "flake_weight": 0, "fuzz_color": [1, 1, 1], "fuzz_roughness": 0.3, "fuzz_weight": 0, "geometry_occlusion": 1, "geometry_opacity": 1, "geometry_thin_walled": false, "id": "mat_00000001", "medium_priority": 0, "metal_ior": { "k": [7.475, 6.4, 5.303], "n": [1.345, 0.965, 0.617] }, "name": "Shell", "specular_color": [1, 1, 1], "specular_ior": 1.5, "specular_rotation": 0, "specular_roughness": 0.2, "specular_roughness_anisotropy": 0, "specular_weight": 1, "subsurface_color": [0.8, 0.8, 0.8], "subsurface_radius": [1, 1, 1], "subsurface_scatter_anisotropy": 0, "subsurface_weight": 0, "thin_film_ior": 1.4, "thin_film_thickness": 500, "thin_film_weight": 0, "transmission_color": [1, 1, 1], "transmission_depth": 0, "transmission_dispersion_abbe": 0, "transmission_scatter": [0, 0, 0], "transmission_scatter_anisotropy": 0, "transmission_tint": [1, 1, 1], "transmission_weight": 0, "type": "plastic", "volume_density": 1, "volume_emission_color": [1, 1, 1], "volume_emission_strength": 0 } ], "meshes": [ { "data": "RlNNRVNIAAABAAAAAwAAAAMAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgD8AAIA/AAAAAAAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAABAAAAAgAAAAAAAAA=", "key": 1 } ], "nodes": [ { "children": [], "id": 50, "kind": "Group", "locked": false, "name": "X", "transform": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1], "visible": true } ], "roots": [50], "textures": [ { "id": 7, "path": "maps/Leather037_Color.jpg", "role": "color" } ] } }
```

#### `scene.reimport`

An updated version of an imported model, matched to it part by part
(`crate::reimport`).

`fs.commands.scene.reimport(payload=...)`

```json
{ "command": "scene.reimport", "payload": { "added": { "materials": [], "meshes": [ { "data": "RlNNRVNIAAABAAAAAwAAAAMAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAgD8AAIA/AAAAAAAAAAAAAAAAAAAAAAAAgD8AAAAAAAAAAAAAAAAAAIA/AAAAAAAAAAABAAAAAgAAAAAAAAA=", "key": 1 } ], "nodes": [ { "children": [], "id": 51, "kind": "Part", "layers": [ "Outer" ], "locked": false, "mesh": 1, "name": "Lid", "parent": 1, "path_key": "Bracket#1/Lid#1", "transform": [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1], "visible": true } ], "roots": [51] }, "hidden": [3], "updates": [ { "layers": [ "Outer", "Shell, inner" ], "material": 1, "mesh": 1, "metadata": { "Revision": "B" }, "node": 2, "path_key": "Bracket#1/Housing#1", "transform": [1, 0, 0, 0, 0, 1, 0, 0.5, 0, 0, 1, 0, 0, 0, 0, 1] }, { "metadata": {}, "node": 1, "path_key": "Bracket#1" } ] } }
```

#### `scene.add_group`

An empty group, under `parent` or at the top.

`fs.commands.scene.add_group(name=..., parent=...)`

```json
{ "command": "scene.add_group", "name": "Sub", "parent": 1 }
```

#### `scene.delete_nodes`

Nodes and everything under them, removed.

`fs.commands.scene.delete_nodes(nodes=...)`

```json
{ "command": "scene.delete_nodes", "nodes": [2, 3] }
```

#### `scene.split_parts`

Turn one part into a group of parts, one per connected shell of its mesh.
For the models that arrive as a single lump because the file had no way to
say otherwise — an STL, an OBJ with no groups — which is the only way to
get at the pieces afterwards.

`fs.commands.scene.split_parts(node=...)`

```json
{ "command": "scene.split_parts", "node": 2 }
```

#### `scene.rename`

A node's name.

`fs.commands.scene.rename(name=..., node=...)`

```json
{ "command": "scene.rename", "name": "Renamed", "node": 2 }
```

#### `scene.set_transform`

Takes a transform per node rather than one node, because moving a selection
of six parts is one action and has to be one undo step. A shared transform
would be meaningless — each part has its own place — so the pairs are
explicit.

`fs.commands.scene.set_transform(transforms=...)`

```json
{ "command": "scene.set_transform", "transforms": [ { "matrix": [1, 0, 0, 1.5, 0, 1, 0, -2.25, 0, 0, 1, 0.125, 0, 0, 0, 1], "node": 2 } ] }
```

#### `scene.set_visibility`

A flag per node rather than one flag for all, so that "Isolate" — show
these, hide everything else — is a single command and therefore a single
undo step. Two commands would leave the user pressing Ctrl+Z twice for one
menu item, with the first press appearing to do nothing.

`fs.commands.scene.set_visibility(states=...)`

```json
{ "command": "scene.set_visibility", "states": [ { "node": 2, "value": false } ] }
```

#### `scene.set_layer_visibility`

Every node on a CAD layer, as the file named it, shown or hidden as one
step.

`fs.commands.scene.set_layer_visibility(layer=..., visible=...)`

```json
{ "command": "scene.set_layer_visibility", "layer": "Construction", "visible": false }
```

#### `scene.set_locked`

A lock per node: a locked node, and everything under it, refuses edits until
unlocked.

`fs.commands.scene.set_locked(states=...)`

```json
{ "command": "scene.set_locked", "states": [ { "node": 2, "value": true } ] }
```

#### `scene.set_rays`

Which rays see each node (`Rays`): camera, diffuse, glossy, transmission and
shadow, and holdout.

`fs.commands.scene.set_rays(states=...)`

```json
{ "command": "scene.set_rays", "states": [ { "node": 2, "rays": { "camera": true, "diffuse": true, "glossy": false, "holdout": true, "shadow": true, "transmission": true } } ] }
```

#### `scene.set_pivot`

The point a node rotates and scales about.

`fs.commands.scene.set_pivot(pivots=...)`

```json
{ "command": "scene.set_pivot", "pivots": [ { "at": [0.5, -1.25, 2], "node": 2 } ] }
```

#### `scene.reparent`

A node moved under another parent (or to the top), at `index` among its
siblings or last.

`fs.commands.scene.reparent(index=..., node=..., parent=...)`

```json
{ "command": "scene.reparent", "index": 3, "node": 2, "parent": 1 }
```

#### `scene.group`

Collect nodes under a new group.

`fs.commands.scene.group(name=..., nodes=...)`

```json
{ "command": "scene.group", "name": "Fasteners", "nodes": [2, 3] }
```

#### `scene.ungroup`

Dissolve a group, promoting its children.

`fs.commands.scene.ungroup(group=...)`

```json
{ "command": "scene.ungroup", "group": 1 }
```

#### `scene.set_metadata`

One metadata field on a node, set, or removed with no value.

`fs.commands.scene.set_metadata(key=..., node=..., value=...)`

```json
{ "command": "scene.set_metadata", "key": "Mass_g", "node": 2, "value": "42.1" }
```

#### `scene.set_units`

The unit lengths are shown in. They are stored in metres either way; `mode`
says whether the scene keeps its size (`KeepSceneSize`) or keeps its numbers
and is rescaled to them (`ScaleScene`), with its lights and cameras.

`fs.commands.scene.set_units(mode=..., unit=...)`

```json
{ "command": "scene.set_units", "mode": "ScaleScene", "unit": "Inch" }
```

#### `scene.add_light`

A light, under `parent` or at the top.

`fs.commands.scene.add_light(light=..., name=...)`

```json
{ "command": "scene.add_light", "light": { "affects_diffuse": true, "affects_specular": true, "casts_shadows": true, "color": [1, 1, 1], "enabled": true, "height": 1, "intensity": 1500, "intensity_unit": "lm", "layer": 0, "shape": "Area", "two_sided": false, "visible_to_camera": true, "width": 1 }, "name": "Key" }
```

#### `scene.set_light`

Every field of a light but its placement, which is the node's transform and
goes through `scene.set_transform` like any other node's.

`fs.commands.scene.set_light(light=..., node=...)`

```json
{ "command": "scene.set_light", "light": { "affects_diffuse": true, "affects_specular": true, "casts_shadows": true, "color": [1, 1, 1], "enabled": true, "falloff": 2, "inner_degrees": 12, "intensity": 400, "intensity_unit": "cd", "layer": 3, "outer_degrees": 30, "shape": "Spot", "temperature": 3200, "visible_to_camera": true }, "node": 2 }
```

#### `scene.add_cutaway`

A cutaway volume, placed. The transform comes with it, because a cutter is
sized to what it is meant to cut and a separate move would be a second undo
step for one action.

`fs.commands.scene.add_cutaway(cutaway=..., name=..., parent=..., transform=...)`

```json
{ "command": "scene.add_cutaway", "cutaway": { "cap": "color", "cap_color": [0.9, 0.2, 0.1], "enabled": false, "shape": "cylinder" }, "name": "Section", "parent": 1, "transform": [1, 0, 0, 0.5, 0, 1, 0, -1, 0, 0, 1, 2, 0, 0, 0, 1] }
```

#### `scene.set_cutaway`

Every field of a cutaway but its placement.

`fs.commands.scene.set_cutaway(cutaway=..., node=...)`

```json
{ "command": "scene.set_cutaway", "cutaway": { "cap": "own", "enabled": true, "shape": "plane" }, "node": 2 }
```

#### `scene.set_measurements`

The whole list, as pins and decals are edited: adding, removing and hiding
are all edits to the list.

`fs.commands.scene.set_measurements(measurements=...)`

```json
{ "command": "scene.set_measurements", "measurements": [ { "a": { "node": 2, "point": [0.25, -1.5, 3] }, "b": { "point": [-2, 0.5, 0.125] }, "visible": false } ] }
```

#### `scene.set_model_sets`

The sets, and which is shown.

`fs.commands.scene.set_model_sets(model_sets=...)`

```json
{ "command": "scene.set_model_sets", "model_sets": { "active": 0, "sets": [ { "always_visible": true, "name": "Chassis A", "nodes": [1] } ] } }
```

### lighting

#### `lighting.set_preset`

The lighting preset: performance, basic, product, interior or jewelry.

`fs.commands.lighting.set_preset(preset=...)`

```json
{ "command": "lighting.set_preset", "preset": "Interior" }
```

#### `lighting.set_custom`

The Custom panel's integrator configuration.

`fs.commands.lighting.set_custom(config=...)`

```json
{ "command": "lighting.set_custom", "config": { "adaptive_sampling_threshold": 0.005, "caustics": true, "direct_lighting": true, "global_illumination": true, "ground_illumination": true, "interior_mode": true, "light_source_materials": true, "max_bounces": 12, "max_transmission_bounces": 16, "regularization_roughness": 0.09, "regularization_start_bounce": 3, "russian_roulette_start_bounce": 3, "shadows": true } }
```

### environment

#### `environment.set_parameter`

One parameter of the environment: brightness, rotation, background, ground
and the like.

`fs.commands.environment.set_parameter(parameter=..., value=...)`

```json
{ "command": "environment.set_parameter", "parameter": "rotation", "value": 0.75 }
```

#### `environment.set_pins`

The whole stack at once.

`fs.commands.environment.set_pins(pins=...)`

```json
{ "command": "environment.set_pins", "pins": [ { "angular_size": [0.18, 1.5], "blend": "add", "color": [1, 1, 1], "direction": [0.24999999, 0.86602545, -0.43301266], "enabled": true, "intensity": 4, "kind": "light", "lighting": true, "name": "Top strip", "roll": 0, "shape": "rectangle", "show_in_background": false, "softness": 0.35 }, { "angular_size": [0.6, 0.45000002], "blend": "over", "color": [1, 1, 1], "direction": [0.46986896, 0.19866933, -0.86008936], "enabled": true, "image": "window.png", "intensity": 1, "kind": "image", "lighting": true, "name": "Window", "roll": 0, "shape": "rectangle", "show_in_background": true, "softness": 0 }, { "angular_size": [6.2831855, 6.2831855], "blend": "over", "color": [1, 1, 1], "direction": [0, -1, 0], "enabled": true, "intensity": 1, "kind": "gradient", "lighting": false, "name": "Backdrop", "radial": true, "roll": 0, "shape": "rectangle", "show_in_background": true, "softness": 0, "stops": [ { "color": [0.1, 0.1, 0.1], "t": 0 }, { "color": [0.7, 0.7, 0.7], "t": 1 } ] } ] }
```

#### `environment.set_sky`

A physical sky, or `None` for an image.

`fs.commands.environment.set_sky(sky=...)`

```json
{ "command": "environment.set_sky", "sky": { "ground_albedo": 0.2, "intensity": 1, "sun": [0.45, 0.574, -0.685], "sun_disc": true, "sun_intensity": 4, "turbidity": 2 } }
```

### studio

#### `studio.set`

The list of Studios. Editing a Studio does not apply it.

`fs.commands.studio.set(studios=...)`

```json
{ "command": "studio.set", "studios": [ { "environment": { "background": { "color": [0.05, 0.05, 0.05], "image": null, "mode": "Environment" }, "brightness": 1, "contrast": 1, "ground": { "bounce": false, "bounce_color": [1, 1, 1], "bounce_contact_falloff": 0, "bounce_strength": 1, "enabled": true, "flatten": false, "occlusion_shadows": false, "reflection_alpha": "coverage", "reflection_blur": 0, "reflection_fade_by": "height", "reflection_fade_distance": 0, "reflection_fresnel": false, "reflection_length": 1, "reflections": 0, "shadow_color": [0, 0, 0], "shadows": true, "size": 0 }, "height": 0, "image": null, "rotation": 1.25, "size": 0 }, "filename_suffix": "Right", "lights": [ { "enabled": false, "name": "Rim" }, { "enabled": true, "layer": 1 } ], "model_set": null, "name": "Hero", "product_yaw": -0.942478, "variants": [ { "active": 0, "multi_material": 7 } ] } ] }
```

#### `studio.apply`

Make the scene what a Studio says, in one step.

`fs.commands.studio.apply(index=...)`

```json
{ "command": "studio.apply", "index": 0 }
```

#### `studio.turn_product`

Turn the product to `yaw` radians about its vertical axis, as a Studio's
view or a frame of a turntable does; nothing else moves.

`fs.commands.studio.turn_product(yaw=...)`

```json
{ "command": "studio.turn_product", "yaw": 0.5 }
```

### animation

#### `animation.set`

The product turning over frames, or `None` for a still. Setting it turns
nothing until a frame is shown.

`fs.commands.animation.set(animation=...)`

```json
{ "animation": { "fps": 24, "frames": 36, "interpolation": "linear", "keys": [ { "frame": 0, "product_yaw": 0 }, { "frame": 36, "product_yaw": 6.2831855 } ] }, "command": "animation.set" }
```

### configurator

#### `configurator.set`

The configurator: its options, rules and layout. Setting it does not apply a
configuration.

`fs.commands.configurator.set(configurator=...)`

```json
{ "command": "configurator.set", "configurator": { "current": [0, 0], "options": [ { "choices": [ { "label": "Red", "swatch": [0.5, 0.05, 0.04], "target": 0 }, { "label": "Cream", "target": 1 } ], "drives": "material", "id": 1, "multi_material": 7, "name": "Body", "style": "swatches" }, { "choices": [ { "label": "Hero", "target": 0 } ], "drives": "studio", "id": 2, "name": "View", "style": "buttons" } ], "placement": "bottom", "rules": [ { "choice": 1, "kind": "excludes", "option": 1, "other": 2, "other_choice": 0 } ], "subtitle": "Pick a finish", "title": "Kettle" } }
```

#### `configurator.choose`

Show a configuration: every option's choice applied, in one step. Refused
when it breaks a rule.

`fs.commands.configurator.choose(choices=...)`

```json
{ "choices": [0, 0], "command": "configurator.choose" }
```

### camera

#### `camera.add`

Save the current view as a new camera.

`fs.commands.camera.add(camera=...)`

```json
{ "camera": { "basis": [0.83205026, 0, -0.5547002, -0.13433959, 0.9702303, -0.20150937, 0.53818697, 0.24218413, 0.8072804], "dof": { "aperture": "Circular", "enabled": false, "f_number": 8, "focus_distance": 743.23615, "keep_focus": false }, "focal_length_mm": 50, "iso": 100, "name": "Hero", "position": [400, 220, 600], "sensor_width_mm": 36, "shutter_speed_seconds": 0.01, "target": [0, 40, 0], "type": "Perspective", "up": [-0.13433959, 0.9702303, -0.20150937] }, "command": "camera.add" }
```

#### `camera.remove`

A saved camera, removed.

`fs.commands.camera.remove(index=...)`

```json
{ "command": "camera.remove", "index": 1 }
```

#### `camera.select`

Which saved camera is the active one.

`fs.commands.camera.select(index=...)`

```json
{ "command": "camera.select", "index": 0 }
```

#### `camera.set`

Replace a saved camera, which is what every lens control and every orbit of
the viewport amounts to.

`fs.commands.camera.set(camera=..., index=...)`

```json
{ "camera": { "basis": [0.9486834, 0, -0.3162278, -0.16903086, 0.8451543, -0.5070926, 0.26726124, 0.5345225, 0.8017837], "dof": { "aperture": "Circular", "enabled": false, "f_number": 8, "focus_distance": 3.7416575, "keep_focus": false }, "focal_length_mm": 50, "iso": 100, "name": "Hero", "position": [1, 2, 3], "sensor_width_mm": 36, "shutter_speed_seconds": 0.01, "target": [0, 0, 0], "type": "Perspective", "up": [-0.16903086, 0.8451543, -0.5070926] }, "command": "camera.set", "index": 0 }
```

#### `camera.rename`

A saved camera's name.

`fs.commands.camera.rename(index=..., name=...)`

```json
{ "command": "camera.rename", "index": 0, "name": "Three quarter" }
```

### image

#### `image.set_style`

One command for the whole style rather than one per control: §9.3
classifies every one of them as `Display`, so there is nothing to be gained
by telling them apart, and a style is small enough to carry whole.

`fs.commands.image.set_style(style=...)`

```json
{ "command": "image.set_style", "style": { "blacks": 0, "bloom": 0, "bloom_radius": 0.03, "chromatic_aberration": 0, "contrast": 1, "display_transform": "PBR Neutral", "exposure": 0.75, "highlights": -0.4, "saturation": 1, "shadows": 0, "sharpening": 0, "temperature": 5200, "tint": 0, "type": "Photographic", "vignette": 0.3, "whites": 0 } }
```

#### `image.set_color_management`

Choose an OCIO config's display and view, or `None` to go back to the
built-in transforms.

`fs.commands.image.set_color_management(color_management=...)`

```json
{ "color_management": { "config": "studio/config.ocio", "display": "sRGB", "view": "ACES 1.0 - SDR Video" }, "command": "image.set_color_management" }
```

### render

#### `render.set_settings`

One command for the whole block rather than one per control, for the same
reason the image style is one: none of it reaches the renderer until Render
is pressed, so there is nothing to be gained by telling the settings apart.

`fs.commands.render.set_settings(render=...)`

```json
{ "command": "render.set_settings", "render": { "adaptive": true, "adaptive_threshold": null, "alpha_mode": "none", "clamp_direct": null, "clamp_indirect": 12, "denoise": true, "denoise_cadence": 4, "denoise_strength": 1, "depth": "32", "dpi": 300, "engine": "cycles-gpu", "filename_template": "{project}_{camera}_{date}", "format": "exr", "jpeg_quality": 92, "kind": "photo", "layers": [], "max_samples": 512, "max_seconds": 600, "output_directory": "renders", "pixel_filter": null, "prefilter_guides": true, "region": [10, 20, 320, 240], "reject_fireflies": true, "relight": false, "resolution": [3840, 2160], "spectral": "auto", "wireframe": { "background_bottom": [0.11, 0.11, 0.115], "background_top": [0.2, 0.2, 0.21], "edges": "mesh", "hidden": "faint", "line_colour": [0.82, 0.82, 0.84], "line_width": 1.5 }, "write_passes": true, "write_text_metadata": false, "write_xmp": false } }
```

### studio_preset

#### `studio_preset.apply`

A whole studio from a preset, framed to the product: environment and floor,
lights, cameras and views, image style, render and lighting settings, and
the preset's materials added to assign. One step to undo. The product's
geometry and its materials are never touched; its top nodes are moved to
stand where the studio expects them (`crate::preset::fit`).

`fs.commands.studio_preset.apply(options=..., preset=...)`

```json
{ "command": "studio_preset.apply", "options": { "front": "-x", "keep_cameras": true, "keep_lights": false, "keep_placement": false }, "preset": { "cameras": [ { "basis": [1, 0, 0, 0, 1, 0, 0, 0, 1], "dof": { "aperture": "Circular", "enabled": false, "f_number": 8, "focus_distance": 1, "keep_focus": false }, "focal_length_mm": 50, "iso": 100, "name": "Camera 1", "position": [0, 0, 0], "sensor_width_mm": 36, "shutter_speed_seconds": 0.01, "target": [0, 0, 0], "type": "Perspective", "up": [0, 1, 0] } ], "description": "", "environment": { "background": { "color": [0.05, 0.05, 0.05], "image": null, "mode": "Environment" }, "brightness": 1, "contrast": 1, "ground": { "bounce": false, "bounce_color": [1, 1, 1], "bounce_contact_falloff": 0, "bounce_strength": 1, "enabled": true, "flatten": false, "occlusion_shadows": false, "reflection_alpha": "coverage", "reflection_blur": 0, "reflection_fade_by": "height", "reflection_fade_distance": 0, "reflection_fresnel": false, "reflection_length": 1, "reflections": 0, "shadow_color": [0, 0, 0], "shadows": true, "size": 0 }, "height": 0, "image": null, "rotation": 0, "size": 0 }, "family": "Samples", "format": "tequila.studio-preset", "framing": { "base_offset_m": 0, "reference_radius_m": 0.1, "scale": "cameras" }, "id": "sample-studio", "image_style": { "blacks": 0, "bloom": 0, "bloom_radius": 0.03, "chromatic_aberration": 0, "contrast": 1, "display_transform": "PBR Neutral", "exposure": 0, "highlights": 0, "saturation": 1, "shadows": 0, "sharpening": 0, "temperature": 6500, "tint": 0, "type": "Basic", "vignette": 0, "whites": 0 }, "lighting": { "custom": { "adaptive_sampling_threshold": 0.01, "caustics": false, "direct_lighting": true, "global_illumination": true, "ground_illumination": false, "interior_mode": false, "light_source_materials": true, "max_bounces": 6, "max_transmission_bounces": 8, "regularization_roughness": 0.09, "regularization_start_bounce": 2, "russian_roulette_start_bounce": 3, "shadows": true }, "preset": "Product" }, "lights": [ { "light": { "affects_diffuse": true, "affects_specular": true, "casts_shadows": true, "color": [1, 1, 1], "enabled": true, "height": 1, "intensity": 1500, "intensity_unit": "lm", "layer": 0, "shape": "Area", "two_sided": false, "visible_to_camera": true, "width": 1 }, "name": "Key", "transform": [1, 0, 0, 0, 0, 1, 0, 0.5, 0, 0, 1, 0.6, 0, 0, 0, 1] } ], "material_rules": [ { "material": "Steel", "pattern": "Metal; Screws*" } ], "materials": [], "name": "Sample Studio", "notes": "", "preset_version": "1", "render": { "adaptive": true, "adaptive_threshold": null, "alpha_mode": "none", "clamp_direct": null, "clamp_indirect": null, "denoise": true, "denoise_cadence": 4, "denoise_strength": 1, "depth": "8", "dpi": 300, "engine": "cycles-gpu", "filename_template": "{project}_{camera}_{date}", "format": "png", "jpeg_quality": 92, "kind": "photo", "layers": [], "max_samples": 256, "max_seconds": null, "output_directory": "", "pixel_filter": null, "prefilter_guides": true, "region": null, "reject_fireflies": true, "relight": false, "resolution": [1920, 1080], "spectral": "auto", "wireframe": { "background_bottom": [0.11, 0.11, 0.115], "background_top": [0.2, 0.2, 0.21], "edges": "mesh", "hidden": "faint", "line_colour": [0.82, 0.82, 0.84], "line_width": 1.5 }, "write_passes": false, "write_text_metadata": false, "write_xmp": false }, "size": "Small", "studios": [], "tags": [], "version": 1 } }
```
