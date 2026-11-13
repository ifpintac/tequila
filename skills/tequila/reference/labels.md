# Labels: logos, prints and stickers

A label is a picture printed on a material, over whatever the material is: a logo on a housing, a panel's legends, a sticker. It belongs to the **material**, so every part wearing that material gets it. Give the part its own material first if it should be alone.

Artwork can be PNG, JPEG, TIFF, TGA, BMP, WebP, GIF, JPEG XL, DDS, PSD/PSB or SVG (drawn sharp, up to 8192 pixels), and a video (MP4, MOV) for one frame of it. Transparent artwork stays transparent.

## 1. Give the part its own material

```json
{"command": "material.unlink", "node": 21}
```

Then read the part's new material id with `tequila_nodes {"filter": "<its name>"}`. Skip this when every part wearing the material should carry the label.

## 2. Put the label on

```json
{"command": "material.add_decal", "material": 20, "name": "Logo", "path": "C:/work/logo.png", "role": "color"}
```

- `path` is the artwork, a file the person gave. For a video add `"time"`, seconds into it.
- It lands **flat on the part's widest face, centred, the right way up, covering a little under half of that face**. On a part that already has a UV layout it can sit on the layout instead (step 4).
- One undo step. The label is added on top of the material's other labels.

## 3. Look

`tequila_ui {"viewport": "rendered"}`, then `tequila_screenshot`. If the face is out of sight, read the view, point the camera at the face with `tequila_look`, look, and put the view back (page `camera`). Check three things: is it on the face that was meant, is it the right size, is it the right way round.

## 4. Move it, size it, turn it

Do this with the UV editor's own controls, which do the arithmetic: open the editor on the label, select it, and transform it. These are `ui` requests, sent with `tequila_request`. A label is named by its material and its place in that material's list of labels, counted from 0.

```json
tequila_request {"request": {"op": "ui", "uv": {"label": [20, 0]}}}
tequila_request {"request": {"op": "ui", "uv": {"select": [0], "transform": {"scale": 0.5}}}}
tequila_request {"request": {"op": "ui", "uv": {"select": [0], "transform": {"offset": [0.01, 0]}}}}
tequila_request {"request": {"op": "ui", "uv": {"select": [0], "transform": {"turn": 90}}}}
tequila_request {"request": {"op": "ui", "uv_editor": false}}
```

- `scale` is a factor about the label's own middle: 0.5 halves it, 2 doubles it; `[across, down]` stretches it.
- `offset` slides it across and down. For a label projected flat (the usual case) the units are metres on the part: `[0.01, 0]` is 1 cm across.
- `turn` is degrees, anticlockwise; `"quarter": 1` is a quarter turn clockwise; `"flip": "horizontal"` or `"vertical"` mirrors it.
- Each is one undo step. Look after each change rather than making several blind: `tequila_screenshot {"window": "uv"}` shows the editor with the label on the layout, an ordinary `tequila_screenshot` the part.
- Several labels lined up: select them (`"select": [0, 1, 2]`) with `"align": "left"`, `centre`, `right`, `top`, `middle` or `bottom`.

A part with a UV layout shows its islands in the editor, and a label can be put on one: click the island (`"click": [u, v]`, a point of the layout from 0 to 1), then `"place": "fit"` (fill the island), `"centre"`, `"along"` or `"square"`.

## 5. Its finish

Everything else about the labels is one list, sent whole. Read it from `tequila_material {"id": 20}` under `definition.graph.decals`, change what was asked, and send all of it back:

```json
{"command": "material.set_decals", "material": 20, "decals": [
  {"name": "Logo", "texture": 1, "enabled": true, "opacity": 1, "roughness": 0.2, "metallic": false,
   "mapping": {"projection": "planar", "axis": "x", "scale": [0.03, 0.03, 1], "offset": [-0.99, -0.5], "pivot": [1.49, 0], "rotation": 0.25,
               "real_world_units": true, "rigid": true, "flip_u": false, "flip_v": true, "wrap_u": "clamp", "wrap_v": "clamp", "uv_set": 0, "blend_sharpness": 6}}]}
```

| Field | Means |
|---|---|
| `opacity` | 1 solid ink, lower lets the material show through |
| `roughness` | The ink's own finish: 0.1 glossy print, 0.35 the default, 0.7 matte |
| `metallic` | `true` for foil and hot-stamped metal |
| `enabled` | `false` hides the label without removing it |
| `side` | `both` (the default), `positive` or `negative`: only faces looking along the projection, or against it, so a label projected through a thin part prints on one side |
| `texture` | The artwork's id; keep it as read |
| `mapping` | Where it sits. Keep it as read, and change it with step 4 |

- **Remove a label:** send the list without it.
- **Order:** later in the list prints on top.
- **Copy a label to another material:** add its entry to that material's list.
- `mapping.rotation` is in turns (0.25 is a quarter); `scale` is the label's width and height. Its `offset` and `pivot` depend on each other, which is why step 4 is the way to move it.

## 6. Artwork that must line up with the part

For artwork drawn to fit - a control panel's legends, a full wrap, a shoe's side panel - the label goes on the part's UV layout, and the artwork is drawn over a picture of that layout:

1. Unwrap the part if it has no layout, and write the layout as a picture (page `uv`): the person, or their designer, draws over it.
2. Add the finished artwork as a label, open the editor on it, and `"place": "fit"` it on its island.

## Good to know

- A label that shows on the wrong face: the part's widest face is not the one wanted. Move it in the editor, or unwrap the part and place it on the right island.
- A label that shows on both the front and the back of a thin part: set `"side"` to `positive` or `negative`.
- A label mirrored: `"flip": "horizontal"` in step 4.
- Nothing shows at all: is the label `enabled`, is `opacity` above 0, does the artwork have anything opaque in it, and is the camera on the face it landed on?
- A label on one of several parts that share a material shows on all of them: step 1.
- Labels are printed flat by default. For a print that follows a strongly curved part without stretching, the part needs a UV layout and the label placed on it.
