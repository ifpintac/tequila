# Lighting, the environment and the floor

A product in Tequila is lit by its **environment** (a studio all round it, built in, or a photograph) plus any **lights** you add, and stands on an invisible **floor** that catches its shadow and reflection. Work from the largest lever to the smallest: a whole studio, then the environment, then single lights, then the floor.

Judge light only in the rendered view: `tequila_ui {"viewport": "rendered"}`, then `tequila_screenshot`.

## 1. A whole studio at once

When the person names a kind of shot, or just wants it to look good, start from a studio preset: lights, cameras and views, floor, image look and render settings, sized to the product, in one undo step.

```json
tequila_studio_presets {}
tequila_studio_preset_apply {"preset": "tequila-soft-studio", "front": "+z"}
```

The list gives each preset's `id`, a description of the look, and marks the size nearest the product `suggested`. Built in: Catalogue (cut out on white, short reflection), White, Soft, Photo (real softboxes near the product, dark surround), Dark, Flat, Gel. `front` is the side of the product that should face the main camera (`+z`, `-z`, `+x`, `-x`); `keep_lights`, `keep_cameras` and `keep_placement` leave those as they are; `"dress": true` also puts the preset's materials on parts by name. The product's own materials are never touched. More on presets and their views: page `variants`.

## 2. The lighting preset: how light is traced

```json
{"command": "lighting.set_preset", "preset": "Product"}
```

`Performance` (fast, direct light), `Basic`, `Product` (the default: soft bounced light), `Interior` (light that has to find its way in), `Jewelry` (the many bounces gems and glass need). Glass, liquids and gems that look black or dull want `Jewelry`.

## 3. The environment

One setting at a time, by its key:

```json
{"command": "environment.set_parameter", "parameter": "rotation", "value": 1.57}
{"command": "environment.set_parameter", "parameter": "brightness", "value": 1.2}
{"command": "environment.set_parameter", "parameter": "background", "value": "Color"}
{"command": "environment.set_parameter", "parameter": "background_color", "value": "#101014"}
```

| Key | Means |
|---|---|
| `rotation` | Turns the light round the product, radians (0 to 6.283). The first thing to try when highlights sit in the wrong place |
| `brightness`, `contrast` | Scale the light; 1 is as it is. Brightness dims pins too |
| `image` | An HDRI file to light by; `null` returns to the built-in studio. Only a file the person gave |
| `background` | What the camera sees behind the product: `Environment` (the lighting itself), `Color` (with `background_color`), `Backplate` (a picture, `backplate`, that lights nothing) |
| `height`, `size` | Where the environment's horizon sits; how big its dome is |

The background does not change the light: a black `Color` background still has the studio lighting the product. `tequila_read {"what": "environment"}` shows everything as it is.

**A physical sky**, for products outdoors - a sun and the sky's own light:

```json
{"command": "environment.set_sky", "sky": {"sun": [0.45, 0.574, -0.685], "turbidity": 2, "intensity": 1, "sun_intensity": 4, "sun_disc": true, "ground_albedo": 0.2}}
```

`sun` is the direction to the sun (y is height: low y is a low, warm sun); `turbidity` 2 clear to 10 hazy. `"sky": null` goes back to the image.

## 4. Light pins: softboxes painted on the environment

A pin is a light, a picture or a gradient placed on the environment's sphere: a strip overhead, a soft key from the left, a dark backdrop. Pins are how most product lighting is shaped. They are one list, sent whole: read it (`tequila_read {"what": "environment"}`, under `pins`), add or change entries, send all of it back.

```json
{"command": "environment.set_pins", "pins": [
  {"name": "Top strip", "kind": "light", "shape": "rectangle", "direction": [0, 1, 0], "angular_size": [0.6, 0.2], "roll": 0,
   "intensity": 4, "color": "#ffffff", "softness": 0.4, "blend": "add", "enabled": true, "lighting": true, "show_in_background": false}]}
```

- `direction` points from the product to the pin: `[0, 1, 0]` overhead, `[-1, 0.5, 0.5]` upper left front. `angular_size` is its width and height in radians (0.6 is a large softbox, 0.1 a small hard light); `roll` turns it.
- `softness` 0 a hard-edged panel, 1 fading right to its middle. `intensity` 1 to 10 for a softbox.
- `blend`: `add` (light on top of the environment) or `over` (covers what is behind it: a black `over` pin is a flag that takes light away).
- `lighting: false` makes a pin seen but not lighting (a backdrop); `show_in_background: false` lights but is not seen behind the product.
- `kind` is `light`, `image` (with `image`, a file) or `gradient` (with `stops` and `radial`). Copy a gradient's fields from `tequila_command {"command": "environment.set_pins"}`.

Keep the pins that are there unless asked to replace the lighting: a studio preset's look is its pins.

## 5. Real lights in the scene

A light with a place and a size, for light that must fall off with distance or come from inside the scene:

```json
{"command": "scene.add_light", "name": "Key", "light": {"shape": "Area", "width": 0.5, "height": 0.5, "intensity": 1500, "intensity_unit": "lm", "color": "#fff4e6"}}
```

- `shape`: `Area` (a panel; `width`, `height`, `two_sided`), `Disc`, `Point` (`radius`), `Spot` (`inner_degrees`, `outer_degrees`, `falloff`), `Sun` (`angle_degrees`: 0.53 is the real sun, 0 gives hard shadows).
- `intensity` with `intensity_unit`: `lm` (lumens: 800 to 3000 for a softbox near a small product), `cd`, `nits`, or `lx` for a sun. `temperature` in kelvin can stand in for `color` (3200 warm, 5600 daylight).
- A new light sits at the origin. Find its id in `tequila_nodes` (kind `light`) and place it with `scene.set_transform` (page `scene`). An area or disc light shines along its own **+Z**; a spot along its own -Z; a sun from far away along its own -Z, so only its turn matters. 40 cm above the origin, facing down:

```json
{"command": "scene.set_transform", "transforms": [{"node": 29, "matrix": [1,0,0,0, 0,0,-1,0.4, 0,1,0,0, 0,0,0,1]}]}
```

- **Change a light:** `tequila_read {"node": 29}` gives its `light` block; send it back changed with `{"command": "scene.set_light", "node": 29, "light": {...}}`. `"enabled": false` switches it off.
- **A softbox's look** on an `Area` or `Disc`: `edge_falloff` 0 to 1 (0.5 is a softbox: gloss paint shows a sheen fading at its edges, not a hard white shape); `fade` -1 to 1 dims it from top to bottom as a graduated scrim; `spread_degrees` below 180 puts it behind a grid.
- **Light linking:** `"links": {"exclude": false, "nodes": [3], "product": false}` lights only those parts (`"exclude": true`: all but those; `"product": true`: the whole product and nothing else). `shadow_links` does the same for its shadow. A big softbox linked to leave out polished metal stops the metal blowing out.
- `visible_to_camera: false` keeps the lamp itself out of the picture.

**The studio kit** adds a photographer's piece already sized to the product and placed for the current camera - quicker than placing a light by matrix:

```json
tequila_request {"request": {"op": "studio_kit", "piece": "softbox"}}
```

`softbox`, `strip` (a long narrow light for an edge highlight), `card` (a white card that bounces light into shadows) or `flag` (a black card that takes light away). It answers with the node it added. One undo step.

## 6. The floor

The floor is part of the environment and set the same way:

```json
{"command": "environment.set_parameter", "parameter": "ground_shadows", "value": true}
{"command": "environment.set_parameter", "parameter": "ground_reflections", "value": 0.3}
{"command": "environment.set_parameter", "parameter": "ground_reflection_blur", "value": 0.1}
{"command": "environment.set_parameter", "parameter": "ground_reflection_length", "value": 0.3}
```

| Key | Means |
|---|---|
| `ground_enabled` | The floor at all |
| `ground_shadows`, `ground_shadow_color` | The product's shadow on it |
| `ground_occlusion` | A soft contact shadow where the product touches |
| `ground_reflections` | The product mirrored in the floor, 0 to 1: 0.3-0.6 typical |
| `ground_reflection_blur` | 0 a mirror; 0.05-0.2 polished |
| `ground_reflection_length` | 1 the whole reflection; 0.2-0.35 the short one that fades out under the product |
| `ground_reflection_fresnel` | `true` fades it when seen from above, as real gloss does |
| `ground_bounce`, `ground_bounce_strength`, `ground_bounce_color` | Light bounced up from the floor onto the product's underside |
| `ground_flatten`, `ground_size` | Flatten the environment's lower half onto the floor; how far it reaches |

`tequila_command {"command": "environment.set_parameter"}` lists every key. For a floor with a material of its own (wood, marble), add a `plane` or `backdrop` shape under the product instead (page `scene`).

## What to reach for

- **Flat and dull:** turn the environment (`rotation`) until highlights run along the product's edges; add a strip pin for a rim.
- **Too contrasty:** a `card` opposite the key, or raise `ground_bounce`.
- **Blown-out white tops:** lower `brightness`, or the image's `exposure` (page `image`); do not darken the materials.
- **Dark and dramatic:** background `Color` near black, a dark `over` pin behind, one or two strips.
- **Chrome and polished metal** show what is around them: they need big soft pins to reflect, not more brightness.
