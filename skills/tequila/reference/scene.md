# The scene: parts, lights, cameras, variants

Every `{"command": ...}` here goes through `tequila_apply`, or several at once as `{"op": "apply", "command": {...}}` entries in `tequila_batch` (one undo step). `{"op": ...}` requests go through `tequila_request`. Exact fields for every command: `commands.md`. Ids are placeholders - read the real ones first.

## Reading it

- `tequila_nodes`: every node - `id`, `name`, `kind` (`group`, `part`, `light`, `cutaway`), `parent`, `material` and `material_name`, `visible`, and for parts their world `size` and `center` in metres.
- `tequila_query {"what": "context"}`: everything at once - `document` holds the whole project as saved: `tree` (each node's `transform`, `metadata`, `children`, and a light's settings), `cameras` and `active_camera`, `environment`, `lighting`, `render`, `image_style`, and - when the project has them - `multi_materials`, `studios`, `configurator`, `model_sets`, `measurements`. A key that is absent means there are none yet.
- CAD files bring metadata (`Source file`, layer names, properties): `metadata` on each node in the context's `tree`.

## Organising parts

```json
{"command": "scene.rename", "node": 5, "name": "Hinge pin"}
{"command": "scene.group", "name": "Fasteners", "nodes": [5, 6, 7]}
{"command": "scene.ungroup", "group": 9}
{"command": "scene.reparent", "node": 5, "parent": 2}
{"command": "scene.delete_nodes", "nodes": [8]}
{"command": "scene.set_metadata", "node": 5, "key": "SKU", "value": "PIN-01"}
{"command": "scene.set_locked", "states": [{"node": 5, "value": true}]}
```

- **Hide and show:** `{"command": "scene.set_visibility", "states": [{"node": 3, "value": true}, {"node": 4, "value": false}]}` - one command for many nodes, so "isolate these" is one undo step: list every part, true for the ones to keep.
- **A model that came in as one lump** (an STL, an OBJ without groups): `{"command": "scene.split_parts", "node": 4}` makes one part per connected piece.
- **Renaming many parts** (`Body.001` → `Body`): read the names, then one `tequila_batch` of `scene.rename`s.
- **Selecting** for the user to see: `tequila_select {"nodes": [3, 4]}`; `tequila_frame {"all": false}` frames them.

## Moving things

`scene.set_transform` takes a 4×4 matrix per node, 16 numbers **row by row**, metres, relative to the node's parent - the translation is at positions 3, 7 and 11 (counting from 0). Read the node's current `transform` from the context's `tree`, change it, send it back:

```json
{"command": "scene.set_transform", "transforms": [{"node": 5, "matrix": [1,0,0,0, 0,1,0,0.01, 0,0,1,0, 0,0,0,1]}]}
```

That puts node 5 1 cm up (+Y is up) from where an identity transform has it. For a rotation, multiply the current matrix by the rotation; keep the scale at 1 unless asked. `scene.set_pivot` sets the point a node rotates about.

**Units:** lengths are always metres. `{"command": "scene.set_units", "unit": "Millimetre", "mode": "KeepSceneSize"}` only changes how lengths are shown. A model imported at the wrong size needs **Units of this model…** (right-click it in the Scene tab) - ask the user.

## Importing and updating models

- `tequila_import {"path": "C:/models/kettle.step"}` (full connection): STEP and IGES (CAD, divided into parts with their colours), glTF/GLB, OBJ, STL, USD (`.usda`), IFC. Not FBX.
- **The file changed:** `{"op": "reimport", "node": 3}` updates the model it belongs to, part by part, keeping materials and edits; the answer says what was added and removed.
- **Smoother or lighter CAD:** `{"op": "reimport", "node": 3, "tessellation": "high"}` - `draft`, `medium`, `high`, `ultra` - divides it again from the CAD data kept with the project.
- **A texture pack** (folder or ZIP of PBR maps) onto parts: `{"op": "texture_set", "path": "C:/packs/Wood049", "nodes": [3]}`.

## Lights

```json
{"command": "scene.add_light", "name": "Key", "light": {"shape": "Area", "width": 0.5, "height": 0.5, "intensity": 1500, "intensity_unit": "lm", "color": "#fff4e6"}}
```

- `shape`: `Area` (a panel; `width`, `height`, `two_sided`), `Disc`, `Point` (`radius`), `Spot` (`inner_degrees`, `outer_degrees`, `falloff`). `intensity` in `lm` (lumens), `cd` (candela) or `nits`; `temperature` in kelvin instead of a colour.
- A new light sits at the origin: find its id in `tequila_nodes` (kind `light`) and place it with `scene.set_transform`. An area or disc light shines along its local **+Z** - the opposite of Blender, so negate the Y and Z columns of a Blender light's matrix. A spot aims along its local -Z, and so does a point light's `.ies` file; an area light's `.ies` file aims where the panel faces. 40 cm up, facing down:

```json
{"command": "scene.set_transform", "transforms": [{"node": 7, "matrix": [1,0,0,0, 0,0,-1,0.4, 0,1,0,0, 0,0,0,1]}]}
```

- `scene.set_light` changes everything but its place. Lights in the scene add to the environment; for most product shots the environment and a lighting preset are enough (`shots.md`).

## Cutaways and measurements

- **A cutaway** shows the inside: `{"command": "scene.add_cutaway", "name": "Section", "cutaway": {"shape": "plane", "enabled": true, "cap": "own"}, "transform": [1,0,0,0.005, 0,1,0,0, 0,0,1,0, 0,0,0,1]}` - `shape` `plane`, `box`, `sphere` or `cylinder`; `cap` `own` (the cut part's material) or `color` with `cap_color`. Place and size it with its transform.
- **Measurements** (dimension lines shown in the viewport): `scene.set_measurements` with the whole list, each `{"a": {"node": 3, "point": [x, y, z]}, "b": {"point": [x, y, z]}, "visible": true}`.

## Cameras

The viewport's view is the active saved camera: `tequila_look` and `tequila_frame` move it, undoably. To keep a shot:

1. Set the view (`tequila_look`, `tequila_frame`).
2. Read `cameras` and `active_camera` from the context's `document`, copy the active camera, give it a `name`, and `{"command": "camera.add", "camera": {...}}` - it is added at the end of the list.
3. `{"command": "camera.select", "index": 1}` makes a saved camera the active one; `camera.rename` and `camera.remove` take its `index`.

Render from a saved camera without switching: `tequila_queue {"camera": 1}`. One render per camera: queue each index (or the user's **Render Every Camera** example plugin).

## Colourways, studios and configurators

These are lists the whole project shares: read the current one from the context's `document`, change it, send the **whole** list back.

**A Multi-Material** - a part with several material variants (colourways), one active:

```json
{"command": "material.set_multi_materials", "multi_materials": [{"id": 1, "name": "Body colours", "parts": [3], "variants": [1, 4, 5], "active": 0}]}
```

`variants` are material ids from `tequila_materials` (make the colourways first: library presets with `"unlink": true`, or recipes). `active` is the index of the one shown. `id` is yours to choose, unique among the multi-materials.

**Studios** - saved set-ups (environment, camera, variants, model set) to switch between: `{"command": "studio.set", "studios": [{"name": "Hero", "variants": [{"multi_material": 1, "active": 1}]}]}`, then `{"command": "studio.apply", "index": 0}`. Copy a full studio from `commands.md` for its environment and camera fields.

**Model sets** - which parts are shown together (with or without the handle): `{"command": "scene.set_model_sets", "model_sets": {"active": 0, "sets": [{"name": "With handle", "nodes": [2, 7]}, {"name": "Without", "nodes": [2]}]}}`.

**A configurator** - the choices a customer makes, each driving a multi-material, a model set or a studio:

```json
{"command": "configurator.set", "configurator": {"title": "Kettle", "options": [
  {"id": 1, "name": "Body", "drives": "material", "multi_material": 1, "style": "swatches",
   "choices": [{"label": "Red", "target": 0, "swatch": "#c23b32"}, {"label": "Ocean", "target": 1, "swatch": "#2d5da8"}]}]}}
{"command": "configurator.choose", "choices": [1]}
```

`target` is the variant's index in the multi-material. `rules` (`"kind": "excludes"`) forbid combinations; `commands.md` has the full shape. Then:

- **Render every combination:** `tequila_request {"request": {"op": "queue", "configurations": true}}` - one job per valid combination, named by its choices.
- **A web configurator:** `{"op": "export", "format": "web", "path": "C:/out/kettle-site", "live": true}` writes a static website (live 3D; `"photos": true` adds a render of every combination).

**Material Templates** - rules that put materials on parts by name, colour, metadata or material: `material.set_templates` (`commands.md`).

## Exporting

`{"op": "export", "format": "glb", "path": "C:/out/kettle.glb"}` (what is shown, as glTF binary), `"format": "usd"` (`.usda`), `"format": "web"` (above), `"format": "spin"` (a 360° spin viewer: `columns` 36, `rows` 1, `tilt` 15, `transparent`). Only when the user asks, to a path they gave or agreed.
