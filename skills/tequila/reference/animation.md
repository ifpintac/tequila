# Turntables and 360-degree spins

Tequila animates one thing: the product turning about its vertical axis, with the camera and the light standing still, as a product on a turntable is photographed. Two things come out of it: a **spin viewer** (a folder of pictures a web page turns with the mouse) and a **turntable** (numbered frames for a video).

## 1. Set up the shot first

Everything else is as for a still: materials, lighting, camera (pages `materials`, `lighting`, `camera`). Check a few angles before rendering dozens of pictures:

```json
{"command": "studio.turn_product", "yaw": 1.5708}
```

turns the product to an angle, in radians (1.5708 is a quarter turn, 3.1416 a half), and nothing else moves. Take a `tequila_screenshot` at 0, a quarter and a half turn: is the product inside the frame at every angle, and does the light work on its back? A long product swings wider than it looks from the front: frame it at its widest angle. `"yaw": 0` puts it back.

## 2. A spin viewer

A request, sent with `tequila_request`; it writes files, so only when the person asks, to a folder they named:

```json
tequila_request {"request": {"op": "export", "format": "spin", "path": "C:/out/headphones", "columns": 36, "rows": 1, "transparent": false}}
```

- A new folder `spin` is made inside `path`, holding the frames and a small web page that turns them.
- `columns` is how many pictures go round: 24 is acceptable, 36 smooth, 72 very smooth. `rows` adds views from higher up, `tilt` degrees apart (15 by default): 3 rows of 36 is 108 renders.
- `transparent` true cuts the product out (PNG); false writes JPEGs on the scene's background.
- It answers with the folder and the `jobs` it queued, one render each, at the project's render size and samples. They run in the background: `tequila_jobs` shows progress. **Say how many renders it is and roughly how long before starting**; a quick render of one frame (page `rendering`) gives the time for one.
- The frames use the project's render settings: for a spin on a web page, 1000-1600 pixels and 128-256 samples are plenty. Set those first (`render.set_settings`, page `rendering`).
- `tequila_stop {"all": true}` ends the whole spin early: the frame that is rendering writes what it has, and the ones waiting are taken out of the queue.

## 3. A turntable for video

The animation is the product's angle at key frames:

```json
{"command": "animation.set", "animation": {"frames": 72, "fps": 24, "interpolation": "linear",
  "keys": [{"frame": 0, "product_yaw": 0}, {"frame": 72, "product_yaw": 6.2832}]}}
```

- `frames` at `fps` a second: 72 at 24 is a three-second turn. A full turn is 6.2832 radians. The last key is one frame past the end, where the first began, so the loop closes without a repeated picture.
- `interpolation`: `linear` for a steady turn that loops; `ease` starts and stops gently, for a turn shown once.
- More keys make a turn that pauses or swings back: `{"frame": 36, "product_yaw": 1.57}` and so on.
- `"animation": null` removes it. `tequila_read {"what": "animation"}` shows what is set.

Setting it turns nothing until a frame is shown.

**Render the frames** with the queue, when the person asks for them:

```json
tequila_queue {"frames": true, "folder": "C:/out/turntable", "filename": "headphones"}
tequila_queue {"frames": true, "first": 0, "last": 11}
tequila_jobs {}
tequila_stop {"all": true}
```

- One job a frame, the product turned to each frame's angle, from the active camera, at the project's render size and samples (or `width`, `height`, `samples` given here). It answers with the `jobs`, their `count` and the `folder`. The files are numbered: `headphones_0000.png`, `headphones_0001.png`…
- **Say how many renders it is and roughly how long before starting**: 72 frames at two minutes each is two and a half hours. `first` and `last` render some of them - the first dozen to check the turn before the rest.
- A frame whose file is already in the folder is left out (`already_rendered` counts them), so a turntable that was stopped carries on where it left off when queued again.
- `tequila_jobs` shows progress; `tequila_stop {"all": true}` ends it early. The scene is left as it was: the product is not left turned.
- The frames are pictures. Joining them into a video file is done outside Tequila.

If the person only needs the product seen from all round on a web page, the spin viewer above is the shorter way.

## Good to know

- The product turns about the vertical through its middle; the floor's shadow and reflection turn with it, the lighting does not.
- A turning product shows every side: finish the back and the underside's materials before rendering.
- Depth of field with a turning product: focus on the axis the product turns about, or the near edge swings in and out of focus.
- For a camera that moves round a still product, or parts that move, there is no animation in Tequila.
