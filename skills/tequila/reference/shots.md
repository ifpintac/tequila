# Shots, renders and the look

Recipes for the shots people ask for most, then the render and image settings. Every `{"command": …}` goes through `tequila_apply`, or several at once as `{"op": "apply", "command": {…}}` entries in `tequila_batch`. Colours are hex sRGB strings (`"#101014"`) or linear ACEScg arrays (`materials.md`); `[0, 0, 0]` is black and `[1, 1, 1]` white either way.

## The clean studio product shot

```json
{"command": "lighting.set_preset", "preset": "Product"}
{"command": "environment.set_parameter", "parameter": "background", "value": "Environment"}
{"command": "environment.set_parameter", "parameter": "ground_enabled", "value": true}
{"command": "environment.set_parameter", "parameter": "ground_shadows", "value": true}
{"command": "environment.set_parameter", "parameter": "ground_reflections", "value": 0.25}
{"command": "environment.set_parameter", "parameter": "ground_reflection_blur", "value": 0.08}
```

Then the camera: `tequila_look {"yaw": 30, "pitch": 15, "focal_length": 85}` for a three-quarter hero view, and `tequila_frame` to fit it. Check with a rendered screenshot.

## The premium black shot

The product reflected on black, as in phone and watch launches:

```json
{"command": "environment.set_parameter", "parameter": "background", "value": "Color"}
{"command": "environment.set_parameter", "parameter": "background_color", "value": [0, 0, 0]}
{"command": "environment.set_parameter", "parameter": "ground_reflections", "value": 0.5}
{"command": "environment.set_parameter", "parameter": "ground_reflection_blur", "value": 0.1}
```

Lower `ground_reflections` toward 0.3 for a subtler floor; `ground_reflection_fresnel: true` fades it toward the camera, as a real glossy surface does. For the short reflection that fades out under the product, add `{"command": "environment.set_parameter", "parameter": "ground_reflection_length", "value": 0.3}` (1 is the whole reflection, 0.5 is gone by the product's top).

## White e-commerce shot

```json
{"command": "environment.set_parameter", "parameter": "background", "value": "Color"}
{"command": "environment.set_parameter", "parameter": "background_color", "value": [1, 1, 1]}
{"command": "environment.set_parameter", "parameter": "ground_shadows", "value": true}
{"command": "environment.set_parameter", "parameter": "ground_reflections", "value": 0}
```

If the product looks grey against the white, raise the image's `exposure` (see the image style below) a third to two thirds of a stop rather than brightening the materials.

## Transparent PNG (cut-out)

Set the render block's `"alpha_mode": "straight"` and `"format": "png"` (see the render settings below). For glass and clear parts, `"alpha_mode": "silhouette"` cuts along the product's outline and keeps the glass solid. The floor's shadow and reflection stay in the alpha.

## Lit by a photograph (HDRI)

Only with a file the user gives: `{"command": "environment.set_parameter", "parameter": "image", "value": "C:/path/to/studio.hdr"}`; `"value": null` returns to the built-in studio. `rotation` (radians, 0 to 6.283) turns the light around the product; `brightness` scales it.

## A whole studio at once (studio presets)

A studio preset brings lights, cameras and views, the floor and the look in one undo step, framed to the product's size. Use one when the person names a studio, or wants the same set of shots across products.
- `tequila_studio_presets` lists them. In each family, the size nearest the product is marked `suggested`.
- `tequila_studio_preset_apply {"preset": "<id>"}` applies one. Options:
  - `"front"`: the side of the product that faces the main camera, `+z`, `-z`, `+x` or `-x`;
  - `"dress": true`: put the preset's materials on parts by name.
- `tequila_studio_render_all` renders every view, one file each. Only when asked for renders.
- `tequila_studio_preset_save {"name": "..."}` keeps the current studio as the person's own preset.

## Jewellery and glass

`{"command": "lighting.set_preset", "preset": "Jewelry"}` gives the deeper light paths gems and glass need; then gem presets (Diamond, Ruby, Sapphire Blue…) or glass ones.

## The camera

- `tequila_look {"view": "front"}`: front, back, left, right, top, bottom, iso.
- `tequila_look {"yaw": 35, "pitch": 20, "distance": 0.6, "at": [0, 0.05, 0]}`: yaw in degrees around the vertical (0 looks from the front), pitch above the horizon, distance and target in metres.
- `"focal_length"` in millimetres: 50 natural; 85-135 for products (less distortion); 24-35 for drama.
- `tequila_frame` fits everything after any of these.

## Rendering

A quick check: `tequila_render {"width": 640, "height": 400, "samples": 32}` returns the image; it blocks Tequila meanwhile, so keep it small.

A final image: `tequila_queue {"name": "Hero", "width": 3000, "height": 2000, "samples": 512}` renders in the background with the project's settings and writes the file; poll `tequila_jobs` for progress and the output path, and tell the user where it is.

### Render settings

`render.set_settings` carries the whole block, so read it first (`tequila_query {"what": "context"}`, the document's `render`), change only what is asked, and send all of it back:

```json
{"command": "render.set_settings", "render": { …everything read, with your changes… }}
```

The fields people ask about:

| Field | Values |
|---|---|
| `engine` | `"cycles-gpu"` (graphics card, fastest), `"cycles-cpu"`, `"tequila"` (Tequila's own renderer) |
| `resolution` | `[width, height]` |
| `max_samples`, `max_seconds` | When to stop; more samples mean less noise |
| `denoise` | `true` cleans the remaining noise |
| `format` | `"png"`, `"jpeg"`, `"tiff"`, `"exr"`, `"psd"` |
| `alpha_mode` | `"none"`, `"straight"`, `"premultiplied"`, `"aliased"`, `"silhouette"` |
| `output_directory`, `filename_template` | Where files go; the template takes `{project}`, `{camera}`, `{date}` |

### The image style

`image.set_style` also carries the whole style: read the document's `image_style`, change, and send all of it back. Useful fields:

| Field | Means |
|---|---|
| `display_transform` | `"PBR Neutral"` (true product colour, the default), `"AgX"` (filmic, handles bright highlights), `"ACES"`, `"sRGB"`, `"Raw"` |
| `exposure` | In stops: +1 twice as bright |
| `contrast`, `saturation` | 1 is unchanged |
| `temperature`, `tint` | White balance, in kelvin (6500 neutral) and green-magenta |
| `vignette`, `bloom` | 0 is none |

Changing the style does not restart the render: the image updates at once.
