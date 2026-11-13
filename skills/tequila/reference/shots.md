# Whole shots, step by step

The shots people ask for most, each as the few steps that make it. Every `{"command": …}` goes through `tequila_apply`, or several at once as `{"op": "apply", "command": {…}}` entries in one `tequila_batch`. Each step has its own page for more: `lighting`, `camera`, `rendering`, `image`.

Whatever the shot: materials first (page `materials`), then light, then camera, then look at a rendered screenshot, then render only if asked.

## The quickest good result: a studio preset

```json
tequila_studio_presets {}
tequila_studio_preset_apply {"preset": "tequila-soft-studio"}
```

Lights, floor, several camera views, image look and render settings in one undo step, sized to the product (page `variants`). Catalogue and White for e-commerce, Soft and Photo for hero shots, Dark for drama. Start here unless the person described the lighting themselves, then adjust.

## The clean studio product shot

```json
{"command": "lighting.set_preset", "preset": "Product"}
{"command": "environment.set_parameter", "parameter": "background", "value": "Environment"}
{"command": "environment.set_parameter", "parameter": "ground_enabled", "value": true}
{"command": "environment.set_parameter", "parameter": "ground_shadows", "value": true}
{"command": "environment.set_parameter", "parameter": "ground_reflections", "value": 0.25}
{"command": "environment.set_parameter", "parameter": "ground_reflection_blur", "value": 0.08}
```

Then the camera, a three-quarter view through a product lens, and fit it:

```json
tequila_look {"yaw": 30, "pitch": 15, "focal_length": 85}
tequila_frame {"all": true}
```

Turn the environment (`rotation`) until the highlights run along the product's edges.

## The premium black shot

The product reflected on black, as in phone and watch launches:

```json
{"command": "environment.set_parameter", "parameter": "background", "value": "Color"}
{"command": "environment.set_parameter", "parameter": "background_color", "value": "#000000"}
{"command": "environment.set_parameter", "parameter": "ground_reflections", "value": 0.5}
{"command": "environment.set_parameter", "parameter": "ground_reflection_blur", "value": 0.1}
{"command": "environment.set_parameter", "parameter": "ground_reflection_length", "value": 0.3}
{"command": "environment.set_parameter", "parameter": "ground_reflection_fresnel", "value": true}
```

The background being black does not darken the light: the studio still lights the product. For real darkness with rim light, use the Dark studio preset or add strip pins (page `lighting`). Dark products on black need a bright edge to separate them: a strip behind and to the side.

## White e-commerce

```json
{"command": "environment.set_parameter", "parameter": "background", "value": "Color"}
{"command": "environment.set_parameter", "parameter": "background_color", "value": "#ffffff"}
{"command": "environment.set_parameter", "parameter": "ground_shadows", "value": true}
{"command": "environment.set_parameter", "parameter": "ground_reflections", "value": 0}
```

A soft shadow and no reflection. If the product looks grey against the white, raise the image's `exposure` a third to two thirds of a stop (page `image`) rather than brightening its materials. Marketplaces want the product filling most of a square frame: render settings `resolution` `[2000, 2000]`, then `tequila_frame`.

## Cut-out on a transparent background

In the render settings (page `rendering`): `"alpha_mode": "straight"` and `"format": "png"`. For glass and clear parts, `"alpha_mode": "silhouette"` cuts along the outline and keeps the glass solid. The floor's shadow and reflection stay in the transparency; switch `ground_shadows` off and `ground_reflections` to 0 for the product alone.

## Lit by a photograph (HDRI)

Only with a file the person gives:

```json
{"command": "environment.set_parameter", "parameter": "image", "value": "C:/hdri/studio_small_08.hdr"}
{"command": "environment.set_parameter", "parameter": "rotation", "value": 2.1}
{"command": "environment.set_parameter", "parameter": "background", "value": "Color"}
```

`rotation` turns the light round the product; `brightness` scales it. Keep the photograph out of the picture with a `Color` background unless the product is meant to stand in that place.

## Jewellery, glass and liquids

```json
{"command": "lighting.set_preset", "preset": "Jewelry"}
```

gives the many bounces these need; without it they render dark. Then gem and glass presets from the library ("diamond", "sapphire", "glass clear"). Gems want small bright lights to sparkle (a few small pins), glass wants big soft ones to show its shape, and both want a dark or graduated background to stand against. Render with 512 samples or more.

## A hero close-up with a soft background

After framing, a long lens and a wide aperture (page `camera`):

```json
tequila_look {"yaw": 35, "pitch": 12, "focal_length": 100}
tequila_frame {"all": true}
```

then read the active camera and send it back with `"dof": {"enabled": true, "f_number": 2.8, …}` and `focus_distance` set to the camera's distance to the part that must be sharp.

## The same product from every side

Apply a studio preset: its views (a hero view, front, side, back, top) become Snapshots. Then, when asked for the renders:

```json
tequila_studio_render_all {}
```

One file per view, named for it.

## Every colour

Colourways as a multi-material, a Snapshot per colour, then `tequila_queue {"studios": true}` (page `variants`).

## Checking a shot before calling it done

Take a rendered screenshot and look for: the product whole inside the frame with air round it; its front readable, not lost in shadow; highlights that describe its shape; a shadow or reflection that grounds it; nothing blown out to white or sunk to black unless meant; the background the colour asked for. Fix the largest fault first, and look again.
