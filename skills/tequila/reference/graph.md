# The material graph

Every material has a graph: nodes that feed its parameters. A plain material has none - each parameter is a number or a colour. A library preset usually has many: patterns, ramps, bumps, mixes. Work at the highest level that does the job, in this order.

## 1. One parameter from a pattern

The quickest change, and one undo step: a procedural pattern drives one parameter.

```json
{"command": "material.set_procedural", "material": 3, "param": "specular_roughness", "procedural": {"kind": "noise", "frequency": 200}, "seed": 3}
{"command": "material.set_procedural", "material": 3, "param": "bump", "procedural": {"kind": "leather", "frequency": 800}, "seed": 2}
{"command": "material.set_procedural", "material": 3, "param": "base_color", "procedural": {"kind": "noise", "frequency": 80}, "seed": 1}
```

- `param` is a parameter key (`base_color`, `specular_roughness`, `base_metalness`, `bump`, `coat_weight`, `geometry_opacity`…). `frequency` is repeats per metre: 100 is a 1 cm feature.
- The kinds: `noise`, `noise_texture`, `ridged`, `cellular`, `spots`, `granite`, `marble`, `wood`, `wood_advanced`, `brushed`, `brushed_radial`, `scratches`, `leather`, `weave`, `camouflage`, `contour`, `mesh` (their settings are on page `recipes`).
- `"procedural": null` detaches it again and leaves the plain value.
- On a **colour** parameter the pattern is read through a colour ramp, grey to start with. Set the ramp next - it only exists once the parameter has a pattern:

```json
{"command": "material.set_gradient", "material": 3, "param": "base_color", "gradient": [{"at": 0, "color": "#2b1d14"}, {"at": 0.6, "color": "#5a3a23"}, {"at": 1, "color": "#80593a"}]}
```

A picture instead of a pattern is `material.set_texture` (page `textures`).

## 2. Several layers: a recipe

Patterns over one another with masks, a range for a number, a depth for a bump, all in one request: write a recipe and `tequila_material_update` the material with it (page `recipes`). `tequila_material_recipe {"material": 3}` reads any material back as a recipe to start from.

## 3. Reading a graph

`tequila_material {"id": 3}` reads one material whole. Its `definition` holds every parameter and, when it has nodes, `graph`:

```json
{"output": 1,
 "nodes": [
  {"id": 2, "kind": "procedural", "procedural": {"kind": "noise", "frequency": 80, "octaves": 4, "gain": 0.5, "lacunarity": 2, "basis": "perlin"}, "seed": 1, "position": [0, 0]},
  {"id": 3, "kind": "gradient", "gradient": [{"at": 0, "color": [0.05, 0.05, 0.05]}, {"at": 1, "color": [0.8, 0.8, 0.8]}], "position": [0, 0]},
  {"id": 4, "kind": "procedural", "procedural": {"kind": "leather", "frequency": 800}, "seed": 2, "position": [0, 0]},
  {"id": 5, "kind": "bump", "depth": 0.0005, "position": [0, 0]}],
 "links": [
  {"from": 2, "to": 3, "socket": "input:0"},
  {"from": 3, "to": 1, "socket": "param:base_color"},
  {"from": 4, "to": 5, "socket": "input:0"},
  {"from": 5, "to": 1, "socket": "param:bump"}],
 "decals": []}
```

- `output` is the surface itself (node 1, not in the list). A link into it names the parameter it drives: `param:base_color`. A link into any other node names that node's input by its number: `input:0`. A link's `output` (a number, left out for the first) picks which output of `from` it leaves by.
- What the kinds are: `procedural` (a pattern and its `seed`), `gradient` (a colour ramp), `bump` (a height made a bump, `depth` in metres), `constant` (`value`: `{"float": 0.35}`, `{"color": [r, g, b]}`), `remap` (`from`, `to` ranges), `clamp`, `invert`, `mix`, `multiply`, `add`, `channel`, `image_texture` (a picture, by its texture id), `mapping_2d` (how a picture is laid on), `curvature`, `occlusion`, `distance`, `vertex_color`, `reroute`.
- `"kind": "node"` is one of the general shader nodes, named by `type`: `noise_texture`, `voronoi_texture`, `wave_texture`, `brick_texture`, `checker_texture`, `gradient_texture`, `white_noise_texture`, `mapping`, `texture_coordinate`, `geometry`, `object_info`, `fresnel`, `layer_weight`, `mix`, `math`, `vector_math`, `map_range`, `clamp`, `hue_saturation`, `brightness_contrast`, `gamma`, `invert`, `blackbody`, `separate_xyz`, `combine_xyz`, `separate_color`, `combine_color` and a few more. Its fixed inputs are `literals` (`{"socket": "input:2", "value": {"float": 20}}`), its options `settings`.
- `"kind": "group"` is a ready-made group (`fdm_layer_lines`, `blend_stack`, `herringbone`, `basketweave`, `stochastic_tiling`, knurls, dots, stripes…); `shader`, `mix_shader` and `add_shader` layer whole surfaces.
- `decals` are the material's labels (page `labels`).

A preset's graph can be 5,000 to 10,000 characters. Read it when you need to know where a look comes from; do not read every material.

## 4. Changing nodes

There is no command for one node. `material.set_graph` replaces the material's **whole definition**, so: read the material, change the `definition` you got, send all of it back.

```json
{"command": "material.set_graph", "material": 3, "definition": {…the definition as read, with your changes…}}
```

What works well this way:

- **A value in a node**: a pattern's `frequency` or settings, a ramp's stops (colours may be hex strings), a bump's `depth`, a `constant`'s value, a general node's `literals` and `settings`.
- **A link moved**: change its `to` and `socket`, or remove it from `links`.
- **A node removed**: take it out of `nodes` along with every link that touches it.
- **A node copied** from elsewhere in the same graph or from another material you read: give it an `id` no other node has, and link it.

Keep every field you did not mean to change, and keep `id`s unique. Then look: `tequila_material_preview {"material": 3}` shows it on a ball.

**Not possible through the tools yet:** adding a node of a new type from nothing. No tool lists what inputs, outputs and settings each node type has, so a graph written from memory is refused or wrong. Do not invent node types or socket numbers. Reach the same look with a pattern (step 1), a recipe (step 2) or by copying a node that already exists; or open the editor for the person (step 5) and say which node to add.

## 5. The editor, for the person to see

```json
tequila_ui {"graph": 3}
```

opens the Material Graph editor on material 3 (`false` closes it). With `tequila_request` you can also bring a whole library material or a ready-made setup into the open graph, as dragging it from the editor's browser does, and search the browser:

```json
tequila_request {"request": {"op": "ui", "graph_bring": {"library": "Aluminium Brushed"}}}
tequila_request {"request": {"op": "ui", "graph_bring": {"setup": "blend"}}}
tequila_request {"request": {"op": "ui", "graph_search": "voronoi"}}
```

The setups are `blend` (two materials mixed by a mask), `two_sided` (a different material on the back of a surface) and `composite` (layers stacked). They arrive wired; the person, or a `material.set_graph` after reading the material again, sets their values.

To see the editor yourself: `tequila_screenshot {"window": "graph"}` returns a picture of that window while it is open.

## Per part: one material, different on each part

A graph can read a colour and a number each part says of itself (the Object Info node's Color and Number), so one material gives many parts their own tint:

```json
{"command": "scene.set_values", "states": [{"node": 12, "colour": "#c23b32", "number": 2}]}
```

This only shows on a material whose graph reads them; the library's random-colour setups do.

## Good to know

- Patterns are sized in metres on the real object. If a pattern looks like nothing, the model's size is wrong (page `fixing`) or the frequency is far off: a 5 cm part with `frequency` 2 shows a tenth of one repeat.
- Realism comes from roughness variation and a little bump more than from colour.
- A bump's `depth` of 0.00005 to 0.0005 is typical; more reads as plastic wrap.
- Two patterns with the same kind, settings and seed line up exactly: that is how a scratch gets both a colour and a metalness.
