# The scene: parts, groups, shapes, units

Every `{"command": ...}` goes through `tequila_apply`, or several at once as `{"op": "apply", "command": {...}}` entries in `tequila_batch` (one undo step). Ids here are placeholders: read the real ones first.

## 1. Read it

- `tequila_scene`: how many parts and materials, the overall size (`bounds`), the file, and any CAD `layers`.
- `tequila_nodes {"parts_only": true, "filter": "screw"}`: nodes by a word in their name or material, each with `id`, `kind` (`group`, `part`, `light`, `cutaway`), `material`, `visible`, and for parts their world `size` and `center` in metres. Without `filter` it lists everything; on a big assembly always filter.
- `tequila_read {"node": 12}`: one node whole - its `transform`, `parent`, `children`, `metadata` (a CAD file's part numbers and properties), `layers`, and a light's or a cutaway's own settings.

A model that came in as `part 1 … part 355` has no names to go by: find parts by `size` and `center` ("the small one at the front is at high +Z"), select what you think it is (`tequila_select {"nodes": [41]}`), and look at a screenshot - the selection is outlined.

## 2. Organise

```json
{"command": "scene.rename", "node": 5, "name": "Hinge pin"}
{"command": "scene.group", "name": "Fasteners", "nodes": [5, 6, 7]}
{"command": "scene.ungroup", "group": 9}
{"command": "scene.reparent", "node": 5, "parent": 2}
{"command": "scene.add_group", "name": "Props"}
{"command": "scene.delete_nodes", "nodes": [8]}
{"command": "scene.set_metadata", "node": 5, "key": "SKU", "value": "PIN-01"}
{"command": "scene.set_locked", "states": [{"node": 5, "value": true}]}
```

Renaming many parts is one `tequila_batch` of `scene.rename`s. Deleting a group deletes everything in it. A locked node, and everything under it, refuses edits until unlocked.

## 3. Hide, show, isolate

```json
{"command": "scene.set_visibility", "states": [{"node": 3, "value": true}, {"node": 4, "value": false}]}
{"command": "scene.set_layer_visibility", "layer": "Construction", "visible": false}
```

Each state needs `value`: true shows, false hides. One command takes many nodes, so "show only these" is one undo step: list every part, true for the ones to keep. Hiding a group hides what is in it. A CAD layer is named exactly as `tequila_scene` lists it.

**Seen by some rays only** - a part that lights or reflects but is not in the picture, or the reverse. Send all six:

```json
{"command": "scene.set_rays", "states": [{"node": 3, "rays": {"camera": false, "diffuse": true, "glossy": true, "transmission": true, "shadow": true, "holdout": false}}]}
```

`camera: false` keeps it out of the picture but in reflections and light (a bounce card); `glossy: false` keeps it out of reflections; `shadow: false` stops its shadow; `holdout: true` cuts a transparent hole where the camera sees it; `"catcher": true` makes it catch shadows as the floor does.

## 4. Move, turn, scale

A node's `transform` is a 4 by 4 matrix, 16 numbers **row by row**, in metres, relative to its parent. Its place is the 4th, 8th and 12th numbers (x, y, z); +Y is up. Read it, change it, send it back:

```json
tequila_read {"node": 5}
{"command": "scene.set_transform", "transforms": [{"node": 5, "matrix": [1,0,0,0.02, 0,1,0,0.01, 0,0,1,0, 0,0,0,1]}]}
```

- **Move:** add to the 4th, 8th or 12th number. The example puts node 5 at 2 cm along x and 1 cm up, from an untouched transform.
- **Turn about the vertical** by angle a: the upper-left 3 by 3 becomes `[cos a, 0, sin a,  0, 1, 0,  -sin a, 0, cos a]`, multiplied onto what was there. **Scale:** multiply the first three numbers of each of the first three rows by the factor.
- One command moves many nodes, each with its own matrix: one undo step.
- A part **on the floor** has its lowest point at y = 0: `center[1] - size[1] / 2` from `tequila_nodes` should be 0. Move it by the difference.
- A group moves everything in it. `scene.set_pivot` sets the point a node turns and scales about in the window: `{"command": "scene.set_pivot", "pivots": [{"node": 5, "at": [0, 0.1, 0]}]}`.

Check with `tequila_nodes` (the part's new `center`) and a screenshot. For the whole product turned to face the camera, turn the product, not its parts: `{"command": "studio.turn_product", "yaw": 0.5}` (radians; page `variants`).

## 5. Add a simple shape

A pedestal, a backdrop, a floor plane, a card to bounce light:

```json
tequila_add_shape {"shape": "cylinder", "width": 0.3, "height": 0.02}
tequila_add_shape {"shape": "backdrop", "width": 2, "depth": 1.2, "height": 1.2, "radius": 0.3}
```

- Shapes: `plane`, `cube`, `cylinder`, `sphere`, `disc`, `backdrop` (a floor curving up into a wall; `radius` is the curve), `wall`. Sizes in metres; `width` is a round shape's diameter. Left out, it is sized to what is in the scene.
- It answers with the new part's `node`. It stands **beside** the scene, on the floor: move it under or behind the product with `scene.set_transform` (step 4), then give it a material (page `materials`).
- Resize it later: `{"command": "scene.set_shape", "node": 27, "shape": {"shape": "cylinder", "width": 0.4, "depth": 0.4, "height": 0.03}}`.

## 6. Copies in a row or a ring

A group becomes a pattern: one of its parts repeated, the copies sharing its mesh and material.

```json
{"command": "scene.set_pattern", "group": 9, "pattern": {"kind": "line", "source": 5, "count": 4, "step": [0.03, 0, 0], "copies": [40, 41, 42]}}
```

`source` is a part inside `group`; `count` includes it; `step` is metres between copies. A ring is `"kind": "circle"` with `centre`, `axis` and `angle` (radians, the whole sweep). `copies` are ids for the new parts: use numbers no node has (one fewer than `count`). Ask `tequila_command {"command": "scene.set_pattern"}` for the ring's exact fields.

## 7. Units and size

Lengths are stored in metres. Two different things go by "units":

- **How lengths are shown** in Tequila's panels: `{"command": "scene.set_units", "unit": "Millimetre", "mode": "KeepSceneSize"}` changes the display and nothing else. Units: `Millimetre`, `Centimetre`, `Metre`, `Inch`, `Foot`.
- **A model that came in at the wrong size** (a phone 150 m long: the file had no units and its numbers were millimetres): page `fixing`.

## 8. A cutaway

A volume that cuts parts open to show the inside:

```json
{"command": "scene.add_cutaway", "name": "Section", "cutaway": {"shape": "plane", "enabled": true, "cap": "own"}, "transform": [1,0,0,0, 0,1,0,0.05, 0,0,1,0, 0,0,0,1]}
```

`shape` is `plane`, `box`, `sphere` or `cylinder`; `cap` is `own` (the cut faces wear the cut part's material) or `color` with `cap_color`. Its transform places and sizes it, as any node's. It appears in `tequila_nodes` as kind `cutaway`; `scene.set_cutaway` changes everything but its place, and `"enabled": false` switches it off without removing it.

## 9. Dimension lines

Measurements are one list, sent whole; read what is there with `tequila_read {"what": "measurements"}`:

```json
{"command": "scene.set_measurements", "measurements": [{"a": {"node": 3, "point": [-0.06, 0.05, 0]}, "b": {"node": 3, "point": [0.06, 0.05, 0]}, "visible": true}]}
```

Each runs from point `a` to point `b`, in metres; with a `node`, the point is in that node's own space and follows it. They show in the viewport, not in renders. An empty list removes them. For a size itself, a part's `size` in `tequila_nodes` is already its width, height and depth.

## Good to know

- **Selecting** shows the person what you mean: `tequila_select {"nodes": [3, 4]}`, and `tequila_frame {"all": false}` points the camera at it.
- A model that arrived as one lump, faceted, lying down or too big: page `fixing`.
- Showing different sets of parts (with and without the lid) as something to switch between: model sets, page `variants`.
- Lights are nodes too: they are added, moved and removed as above, and set up on page `lighting`.
