# Fixing a model that came in wrong

A model from a CAD system, a scanner or another renderer often needs a few repairs before it can be dressed and lit. Find out what is wrong first: `tequila_scene` (the overall `bounds` and how many parts), `tequila_nodes` (names, sizes), and a screenshot.

## 1. Far too big or too small

Lengths are metres. A hand tool 120 m long, or a chair 2 mm tall, came from a file that did not say its units; nearly always its numbers were millimetres. Materials, patterns and studio presets are sized for real objects, so fix this first.

Ask what size the model would be in each unit, then say which its numbers are in:

```json
tequila_model_units {"node": 1}
tequila_model_units {"node": 1, "unit": "mm"}
```

- The first changes nothing: it answers with the `size` now ("121 × 110 × 25 m") and, under `if_its_numbers_are`, the size in millimetres, centimetres, inches, feet and metres. Pick the one that reads as the real thing: a pedal is 12 cm, not 12 m.
- The second scales the whole model the node belongs to (`node` is any node of it) about the middle of its base, so it stands where it stood. One undo step. This is the window's **Units of this model…**.
- `"read_as": "mm"` says what the numbers are read as now, when not metres: a model read as millimetres that was drawn in inches is `{"unit": "in", "read_as": "mm"}`.
- Then `tequila_frame`, and `tequila_scene`: are the `bounds` the size of the real thing? Lights and cameras set up round the old size stay where they were: apply the studio again, or frame again.

Do not use `scene.set_units` for this: with `KeepSceneSize` it only changes how lengths are shown, and with `ScaleScene` it rescales the whole scene, lights and cameras too.

## 2. Lying on its back

CAD draws Z up; Tequila is Y up. A model lying down was read the wrong way up.

- **Best, when you are importing it:** say so at import: `tequila_import {"path": "C:/models/bracket.stl", "up": "z"}` (page `files`).
- **Already in the scene:** turn its top node a quarter turn about X. For a top node at the identity, Z up to Y up is:

```json
{"command": "scene.set_transform", "transforms": [{"node": 1, "matrix": [1,0,0,0, 0,0,1,0, 0,-1,0,0, 0,0,0,1]}]}
```

  (`[1,0,0,0, 0,0,-1,0, 0,1,0,0, 0,0,0,1]` turns it the other way, if it ends up face down.) Then check it stands on the floor: its lowest point, `center[1] - size[1] / 2` in `tequila_nodes`, should be 0; add the difference to the 8th number.
- The window has **Turn** in the same right-click menu.

## 3. One lump instead of parts

An STL, or an OBJ without groups, arrives as a single part, so nothing can be dressed alone.

```json
{"command": "scene.split_parts", "node": 4}
```

makes one part for each separate piece of the mesh, in a group. Pieces that touch or share a surface stay together: the split follows what is connected, not what is a different material. The new parts are named by number; find them by `size` and `center`, rename the ones that matter (page `scene`). One undo step. A part that is already one connected piece is refused ("a single connected shell"): then there is nothing to split, and different materials on one piece need the model divided where it was made.

## 4. Faceted curves

Round surfaces showing flat facets, two different causes:

- **A CAD model (STEP, IGES) divided too coarsely.** Its exact surfaces are kept with the project and can be divided again, finer:

```json
tequila_request {"request": {"op": "reimport", "node": 3, "tessellation": "high"}}
```

  `draft`, `medium` (the default), `high`, `ultra`. `node` is any node of the model. Materials, names and edits are kept. `ultra` on a large assembly makes millions of triangles and slows the viewport: use `high` unless a close-up needs more.
- **A mesh (STL, OBJ) that looks faceted.** This is the window's **Improve model…**, on a part or on a group and everything under it:

```json
tequila_improve {"node": 3}
tequila_improve {"node": 3, "shade": 30}
tequila_improve {"node": 3, "subdivide": 1}
```

  The first changes nothing and says how many triangles there are and how many a subdivision would make. `shade` works the shading out again, smooth across edges gentler than that many degrees and sharp across steeper ones: the fix for a file that had no normals, and the one to try first. `subdivide` (1 to 3) makes four triangles of every one each time, the new points following the curve of the surface, for curves that still show their facets; flat faces and sharp edges stay as they are. Each is one undo step. Look in the rendered view after: more than one subdivision is rarely needed, and each multiplies the triangles by four.

A round part that looks faceted only in the Solid view may be fine: look in the rendered view first.

## 5. Parts missing, or things that should not be there

- **Hidden parts:** `tequila_nodes` shows `visible: false`. Show them with `scene.set_visibility`. CAD files bring construction geometry on layers of their own: `tequila_scene` lists `layers`, and `{"command": "scene.set_layer_visibility", "layer": "Construction", "visible": false}` hides one.
- **A part far away from the rest** makes the whole product tiny when framed: its `center` is far from the others'. Delete it or move it.
- **Duplicates on top of each other** (two shells in the same place flicker or look dark): two parts with the same `size` and `center`. Hide one and look.
- **Inside parts nobody will see** (screws, boards) cost memory: hide the group; do not delete what a cutaway may want later.

## 6. Dark, inside-out or see-through faces

A part that renders black, or shows its inside, has faces pointing the wrong way. There is no command to flip them. For a mesh, *Shade again* in **Improve model…** often cures shading that is only wrong, not flipped; a truly inside-out part needs fixing where it was modelled and updating (step 8). Thin single-skin surfaces (a sheet with no thickness) render from both sides, but glass needs a real thickness to refract: say so rather than fighting the material.

## 7. Textures or labels that will not sit right

The part has no UV layout, or a poor one: unwrap it (page `uv`), or project the texture `tri_planar` (page `textures`).

## 8. The model changed in CAD

Do not import it again: update it, and every material, name and edit stays on the parts that are still there.

```json
tequila_request {"request": {"op": "reimport", "node": 3}}
tequila_request {"request": {"op": "reimport", "node": 3, "path": "C:/models/kettle_rev_B.step"}}
```

The first reads the file it came from again; the second takes a newer file saved under another name. The answer says how many parts matched and which were added and removed (page `files`).

## What cannot be fixed here

Tequila is not a modeller: holes in a mesh, missing faces, gaps between parts, a wrong shape. Say plainly that it needs fixing in the CAD or modelling program, then update the model from the corrected file.
