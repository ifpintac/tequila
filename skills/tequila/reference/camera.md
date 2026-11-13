# The camera: views, lenses, saved cameras, composition

The viewport looks through the **active camera**. Moving the view moves that camera, undoably. A project keeps a list of saved cameras, each a shot to come back to and render.

## 1. Point it

```json
tequila_look {"view": "front"}
tequila_look {"yaw": 30, "pitch": 15, "focal_length": 85}
tequila_frame {"all": true}
```

- `view`: `front`, `back`, `left`, `right`, `top`, `bottom`, `iso`.
- `yaw` is degrees round the vertical: 0 looks from the front (the camera on +Z), 90 from +X (the right-hand side as you face the front), 180 from behind; `pitch` is degrees above the horizon. They orbit about the target `at` (a point `[x, y, z]` in metres) at `distance` metres. Left out, each stays as it is.
- `from` is an exact eye position instead of yaw, pitch and distance.
- `tequila_frame` fits everything in the picture after any of these; `{"all": false}` fits the selection (select the part first with `tequila_select`).
- Each answers with where the camera now is: `position`, `target`, `yaw`, `pitch`, `distance`, `focal_length`. `tequila_query {"what": "camera"}` reads the same without moving.

The usual order: choose the angle (`yaw`, `pitch`), choose the lens, then `tequila_frame`, then look at a screenshot.

**Angles that work for products:** a three-quarter hero view is `yaw` 25-40 with `pitch` 10-20. Straight on (`pitch` 0) reads as technical; `pitch` 30-50 looks down, as at something on a table; top (`pitch` 90) is a flat lay.

## 2. The lens

`focal_length` in millimetres, on a 36 mm sensor:

| Lens | Look |
|---|---|
| 24-35 | Wide: dramatic, the near corner looms. Small products look distorted |
| 50 | Natural |
| 85-135 | The product lens: flatter, truer proportions. The default choice |
| 200+ | Nearly flat, as in catalogue packshots |

Changing the lens changes how big the product is in frame: `tequila_frame` again afterwards.

## 3. Keep a shot: saved cameras

```json
tequila_read {"what": "cameras"}
{"command": "camera.select", "index": 1}
{"command": "camera.rename", "index": 1, "name": "Three quarter"}
{"command": "camera.remove", "index": 2}
```

`tequila_read {"what": "cameras"}` lists the cameras, each whole, and `active_camera`, the number (from 0) of the one in use. To save the current view as a new camera: set the view, read the active camera (`tequila_read {"what": "cameras", "index": 0}`), give that block a new `name`, and add it:

```json
{"command": "camera.add", "camera": {"name": "Hero", "type": "Perspective", "position": [0.35, 0.17, 0.6], "target": [0, 0.09, 0], "up": [-0.05, 0.99, -0.09],
  "basis": [0.866, 0, -0.5, -0.052, 0.995, -0.091, 0.497, 0.105, 0.861], "focal_length_mm": 58, "sensor_width_mm": 36, "iso": 100, "shutter_speed_seconds": 0.01,
  "dof": {"enabled": false, "aperture": "Circular", "f_number": 8, "focus_distance": 0.7, "keep_focus": false}}}
```

Do not write a camera's numbers by hand: `basis`, `up` and `position` must agree, so copy the block you read. The new camera goes at the end of the list; `camera.select` switches to one, and from then on `tequila_look` and `tequila_frame` move that one.

Render from a saved camera without switching to it: `tequila_queue {"camera": 1}` (page `rendering`).

## 4. Depth of field

Blur in front of and behind the point of focus is part of the camera block. Read the camera, change `dof`, send the whole camera back with its index:

```json
{"command": "camera.set", "index": 0, "camera": {…the camera as read, with "dof": {"enabled": true, "f_number": 2.8, "focus_distance": 0.6, "aperture": "Circular", "keep_focus": false}…}}
```

- `f_number`: 1.4-2.8 strong blur, 5.6-8 a little, 11-16 nearly none. Small products close up blur far more than large ones at the same number.
- `focus_distance` is metres from the camera to what must be sharp. The camera's `distance` from `tequila_look` is its distance to the target: a good first value.
- `"keep_focus": true` holds the focus on the target as the camera moves.
- Depth of field shows only in the rendered view and in renders.

`focal_length_mm` and `sensor_width_mm` live in the same block; `tequila_look`'s `focal_length` is the quick way to set the first.

## 5. Shape of the picture

The picture's shape is the render's `resolution` (page `rendering`): 1920 by 1080 is wide, 2000 by 2000 square, 1600 by 2000 an upright 4:5. The viewport shows that shape as a frame, so what is inside the frame is what the render holds; `tequila_ui {"frame": true}` shows it.

## 6. Composition guides

Lines over the viewport to place the product by, for the person and for your own screenshots:

```json
tequila_request {"request": {"op": "ui", "guides": ["thirds"]}}
tequila_request {"request": {"op": "ui", "guides": false}}
```

A name or a list of names shows just those; `true` and `false` show and hide them keeping the choice. The guides: `thirds`, `golden_section`, `golden_spiral`, `golden_triangle`, `harmonious_triangles`, `diagonals`, `diagonals_45`, `dynamic_symmetry`, `centre_cross`, `pyramid`, `radial`, `safe_areas`. `tequila_query {"what": "ui"}` says which are on, under `guides`. Guides are never in a render.

**Placing the product by them:** the product's middle on a third rather than dead centre gives a calmer picture when there is copy space to leave; dead centre suits a symmetrical packshot. Move the target, not the product: `tequila_look {"at": [0.05, 0.09, 0]}` shifts what the camera aims at, which slides the product across the frame.

## 7. Check

`tequila_screenshot` after every camera change: the answer's numbers do not tell you whether a part is cut off at the frame's edge or hidden behind another. Look for: the whole product inside the frame with a little air round it; no part touching the edge; the face that matters towards the camera; the floor's horizon not cutting through the product.

## Good to know

- `tequila_frame` moves the camera back or forward until everything visible fits, hidden parts not counted. A stray part far away (a forgotten shape, a light's stand) makes the product tiny: hide or delete it, then frame again.
- Each Snapshot keeps a camera, and a studio preset brings several named views (page `variants`).
- An orthographic view, with no perspective at all, is the camera block's `"type": "Orthographic"`.
- Turning the product itself, rather than the camera, keeps the lighting where it is: `studio.turn_product` (page `variants`).
- Camera moves are undo steps: `tequila_undo` goes back to the last view.
