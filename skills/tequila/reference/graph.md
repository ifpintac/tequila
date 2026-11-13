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

## 3. A graph as it is saved

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

- `output` is the surface itself (node 1, not in the list). A link into it names the parameter it drives (`param:base_color`); into any other node, that node's input by its number (`input:0`). A link's `output` picks which output of `from` it leaves by, left out for the first.
- A node's `kind` is what it is; `"kind": "node"` is a general shader node named by `type`, `"kind": "group"` a ready-made group named by `group`. `literals` are the values of inputs nothing is wired into; `decals` are the material's labels (page `labels`).

A preset's graph is 5,000 to 10,000 characters this way. To see what a graph does, or to change it, read it as step 4 does: a few hundred characters.

## 4. Changing a graph, a node at a time

Three tools work on one material's graph the way a person does in the editor: look, look a node up, change one thing. Each change is one undo step, and a change that would leave a graph that does not work is refused in words, with nothing changed.

**Look.** In brief, a line a node; then one node whole:

```json
tequila_graph_read {"material": 3}
tequila_graph_read {"material": 3, "node": 2}
```

The brief reading gives each node's `id`, what it is (`node`: its name in the catalogue) and `to`: where its output goes - `3.Position` is node 3's input Position, `surface.base_color` the material's own parameter. `driven` lists the parameters the graph drives. One node whole gives its `settings`, its `fields`, each input with its `value` or the wire it comes `from`, and each output with where it goes.

**Look a node up before adding it.** The catalogue has every node there is, some ninety: say what you want in words, or ask for one by name.

```json
tequila_graph_nodes {"search": "voronoi"}
tequila_graph_nodes {"family": "group"}
tequila_graph_nodes {"name": "map_range"}
```

A search that leaves three or fewer answers with them whole: what each is for, its inputs (name, type, default, range), outputs, `settings` with their options, and `fields`. More come a line each, a page at a time (`"page": 2`). The families: `input`, `texture`, `pattern` (the patterns of step 1, as `pattern_noise`, `pattern_leather`…), `group` (knurl, hexagons, hammered, cracks, herringbone…), `color`, `converter`, `vector`, `shader`, `simple`, `layout`. Never guess a node's name, an input or a setting: the answer has them exactly.

**Change.** A plain plastic given cells that vary its colour and roughness, step by step (ids are as a first node added to a plain material gets them: read yours from each answer):

```json
tequila_graph_edit {"material": 3, "action": "add", "node": "voronoi_texture", "inputs": {"Scale": 60}}
tequila_graph_edit {"material": 3, "action": "add", "node": "gradient", "fields": {"gradient": [{"at": 0, "color": "#2a2a2e"}, {"at": 1, "color": "#8c8c92"}]}}
tequila_graph_edit {"material": 3, "action": "connect", "from": 2, "output": "Distance", "to": 3, "input": "Position"}
tequila_graph_edit {"material": 3, "action": "connect", "from": 3, "input": "base_color"}
tequila_graph_edit {"material": 3, "action": "add", "node": "map_range", "inputs": {"To Min": 0.25, "To Max": 0.6}}
tequila_graph_edit {"material": 3, "action": "connect", "from": 2, "output": "Distance", "to": 4, "input": "Value"}
tequila_graph_edit {"material": 3, "action": "connect", "from": 4, "input": "specular_roughness"}
tequila_graph_edit {"material": 3, "action": "set", "node": 2, "settings": {"feature": "f2"}, "inputs": {"Scale": 90}}
tequila_graph_edit {"material": 3, "action": "disconnect", "input": "specular_roughness"}
tequila_graph_edit {"material": 3, "action": "remove", "node": 4}
tequila_material_preview {"material": 3}
```

- `add` takes a `node` name from the catalogue, and may set it at once. It answers with the new node's `id` and its sockets. A new node drives nothing until it is connected.
- `set` changes a node that is there: `settings` by key; `fields`, the node's own (`{"depth": 0.0003}` on a bump, `{"procedural": {"frequency": 900}}` on a pattern, `{"value": 0.35}` on a value, a ramp's `gradient`); `inputs` by the input's name - a number, `"#rrggbb"`, `[x, y, z]`, or `null` for its default; `label`; `muted`. What is not named stays as it is.
- `connect` runs a wire `from` a node (its `output` by name when it has several) `to` another node's `input`; leave `to` out and the `input` is a parameter's key on the surface. An input takes one wire: a new one replaces the old, and the answer says what it replaced.
- `disconnect` frees an `input` (of `node`, or of the surface); the parameter goes back to its own value. `remove` takes a `node` out with its wires.
- The surface's own values are not set here: `material.set_parameter` (page `materials`).
- Every part wearing the material changes. `material.unlink` a part first to change it alone.

Look after every few steps: `tequila_material_preview {"material": 3}` shows the material on a ball; the viewport shows it on the product.

The whole graph can still be replaced in one request - `material.set_graph` with the `definition` of step 3, changed - which is right for copying a graph from one material to another, and nothing else.

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

The setups are `blend` (two materials mixed by a mask), `two_sided` (a different material on the back of a surface) and `composite` (layers stacked). They arrive wired; set their values with `tequila_graph_read` and `tequila_graph_edit`.

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
