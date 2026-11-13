# Rendering

Three ways to get a picture, from quickest to final: a **screenshot** of the rendered viewport (to check your work), a **quick render** (a small true render, returned to you), and a **queued render** (the final picture, written to a file in the background).

Render to a file only when the person asks for a render. Inside Tequila's Assistant the person is asked before any file is written.

## 1. Check your work: the rendered viewport

```json
tequila_ui {"viewport": "rendered"}
tequila_screenshot {"viewport": true, "settle": 12}
```

The viewport's modes are `rendered` (ray traced: real materials and light), `material` and `solid` (fast previews) and `wireframe`. The screenshot waits up to `settle` seconds for the picture and says whether it had finished. "NOT finished" means it was still grainy: take another with a longer `settle` (up to 60) before judging anything.

## 2. A quick render

```json
tequila_render {"width": 640, "height": 400, "samples": 32}
```

Returns the picture to you; nothing is kept. Tequila is busy until it is done, so keep it small: up to about 800 pixels and 64 samples. `"wireframe": true` draws the model's edges instead, in a second.

## 3. The final render

```json
tequila_queue {"name": "Hero", "width": 3000, "height": 2000, "samples": 512}
tequila_jobs {}
```

- It starts in the background with the project's render settings and answers at once with the `job` and the `folder` it writes to. The scene and camera are captured as they are now: you can go on working.
- `tequila_jobs` lists every job with its `state` (`queued`, `rendering`, `done`, `failed`), `progress` from 0 to 1 and, when done, `output`: the file. **Tell the person where the file is.** A big render takes minutes: say it is running rather than waiting on it in silence.
- Left out, `width`, `height` and `samples` are the project's own. `denoise` true or false; `engine` (`cycles-gpu`, `cycles-cpu`, `tequila`) for this render only.
- `folder` and `filename` choose where it goes; left out, the project's render folder. Use them only with a place the person named.
- `tequila_stop` stops the render that is running; it writes what it has. Queued jobs behind it then start, so stop again for each.

**More than one picture:**

```json
tequila_queue {"camera": 1}
tequila_queue {"studio": "Red hero"}
tequila_queue {"studios": true}
tequila_queue {"configurations": true}
```

A saved camera by its number, without switching to it; one Snapshot by name; every Snapshot; every valid combination of the configurator (page `variants`). `tequila_studio_render_all` renders every view of a studio preset. Before a batch, say how many pictures it is.

**How big and how many samples:** web and review 1600-2000 pixels wide; print and hero 3000-4000 and up. Samples 128-256 with denoising for most products; 512-1024 for glass, liquids, gems, depth of field and dark scenes.

## 4. The render settings

Everything else about a render is one block, the project's render settings. Read it, change what was asked, send **all of it** back:

```json
tequila_read {"what": "render"}
{"command": "render.set_settings", "render": {…the block as read, with your changes…}}
```

| Field | Values |
|---|---|
| `resolution` | `[width, height]` in pixels: also the shape of the viewport's frame |
| `max_samples`, `max_seconds` | When to stop; `max_seconds` `null` for no time limit |
| `engine` | `cycles-gpu` (the graphics card, fastest), `cycles-cpu`, `tequila` (Tequila's own renderer) |
| `denoise`, `denoise_strength` | Clean the remaining grain; 1 is full strength |
| `format` | `png`, `jpeg`, `tiff`, `exr`, `psd` |
| `depth` | Bits a channel: `"8"`, `"16"`, `"32"` (32 for EXR) |
| `jpeg_quality` | 1-100 |
| `alpha_mode` | Transparency, below |
| `write_passes` | Render passes written with the picture, below |
| `layers` | Parts rendered as pictures of their own, below |
| `region` | `[x, y, width, height]` to render only a rectangle; `null` for all |
| `dpi` | Written into the file for print |
| `spectral` | `auto` (wavelengths where glass or a gem disperses light), `on`, `off`; Tequila's own renderer only |
| `kind` | `photo`, or `wireframe` for a line drawing of the edges |
| `output_directory`, `filename_template` | Where files go; the template takes `{project}`, `{camera}`, `{date}`. Change only when the person says where |

This command is not offered on a restricted (remote) connection or to an assistant running on an API key or a local model: there, `tequila_queue` sets size, samples, denoising and engine, and format and transparency are the person's to set in the Render window (`tequila_ui {"render_window": true}` opens it).

## 5. A transparent background

In the render settings: `"alpha_mode": "straight"` with `"format": "png"` (or `tiff`, `exr`, `psd`; JPEG has no transparency).

| `alpha_mode` | Gives |
|---|---|
| `none` | No transparency: the picture as on screen |
| `straight` | The product cut out; glass as see-through as it is. The usual choice for PNG |
| `premultiplied` | The same, as compositing software wants it (EXR) |
| `silhouette` | Cut along the product's outline, glass kept solid with what shows through it: a lightbulb is not cut out as a ghost |
| `aliased` | A hard-edged cut-out with no soft edge pixels |

The floor's shadow and reflection stay in the transparency. For a pure white or coloured background instead, set the environment's `background` to `Color` (page `lighting`).

## 6. Layers and passes, for compositing

- **Layers** - some parts as a picture of their own, cut out, written beside the main one. In the render settings: `"layers": [{"name": "Cups", "nodes": [2, 3], "enabled": true}]`. Everything else still casts its shadows and reflections on them; a group in `nodes` brings what is in it. The file is the main one's name with the layer's name added.
- **Passes** - depth, normals, albedo, object masks, and the light split into diffuse, reflection and refraction, for grading: `"write_passes": true` with `"format": "exr"` or `"psd"`. PNG, JPEG and TIFF cannot hold passes, and the setting does nothing there.
- **Relighting** - `"relight": true` keeps a picture per light group so lights can be turned up and down after the render, in the Render window. It needs much more memory; only when asked.

## 7. Rendering on other computers

`tequila_request {"request": {"op": "queue", "network": true}}` sends the render to the render nodes set up in the Render window's Network rendering, of the active camera. It answers with how many jobs were sent and the folder the files land in. It fails by saying no node is set up when none is.

## When a render is wrong

- **Grainy:** more samples, or `denoise` on. Fine sparkles that never clear (fireflies) come from small bright lights seen in glossy or glass parts: enlarge the light or pin.
- **Black, or "standing in for Cycles":** Cycles could not run on the graphics card; Tequila's own renderer is drawing on the processor instead. Suggest updating the graphics driver; `engine` `tequila` or `cycles-cpu` renders meanwhile.
- **Glass or liquid black:** the lighting preset needs more bounces: `Jewelry` (page `lighting`).
- **Different from the viewport:** the render uses the saved camera and the render settings' shape; `tequila_ui {"frame": true}` shows that frame in the viewport. Depth of field and the image style apply to both.
- **Slow:** a reflecting floor, depth of field and high resolution cost most. Halve the size for drafts.
- **The job failed:** its `message` in `tequila_jobs` says why (often the folder cannot be written).
