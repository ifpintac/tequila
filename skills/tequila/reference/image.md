# The look of the picture, after the render

Exposure, contrast, white balance, the tone curve, bloom and a vignette are applied to the finished light, as a photographer develops a picture. Changing them does **not** render again: the viewport and a finished render in the Render window update at once. Reach for these before touching lights or materials when the picture is only too dark, too flat or too cold.

## 1. Read the style, change it, send all of it back

The image style is one block:

```json
tequila_read {"what": "image_style"}
{"command": "image.set_style", "style": {"type": "Photographic", "display_transform": "PBR Neutral", "exposure": 0.5, "contrast": 1.1, "saturation": 1,
  "highlights": -0.3, "shadows": 0.2, "whites": 0, "blacks": 0, "temperature": 6500, "tint": 0,
  "vignette": 0.15, "bloom": 0, "bloom_radius": 0.03, "sharpening": 0, "chromatic_aberration": 0}}
```

Send every field: one left out goes back to its default. Start from the block you read, not from this example.

| Field | Means | Typical |
|---|---|---|
| `exposure` | Brightness in stops: +1 is twice as bright, -1 half | -1 to +1 |
| `contrast` | 1 unchanged; above, deeper darks and brighter lights | 0.9-1.2 |
| `saturation` | 1 unchanged, 0 black and white | 0.9-1.1 |
| `highlights`, `shadows` | Pull the bright end down or lift the dark end, -1 to 1. `highlights` -0.3 brings back a blown white top | -0.5 to 0.5 |
| `whites`, `blacks` | Where pure white and pure black sit | -0.2 to 0.2 |
| `temperature` | White balance in kelvin: 6500 neutral, lower is cooler and bluer, higher warmer | 5000-7500 |
| `tint` | Green to magenta, 0 neutral | -0.1 to 0.1 |
| `vignette` | Darker corners, 0 none | 0.1-0.3 |
| `bloom`, `bloom_radius` | A glow round bright lights and highlights | 0.05-0.2 |
| `sharpening` | Crisper edges; too much rings | 0-0.3 |
| `chromatic_aberration` | Coloured fringes at the edges, as a cheap lens | 0-0.2 |
| `type` | `Basic` or `Photographic`: which set of controls the Image tab shows. Use `Photographic` when setting highlights, shadows, whites or blacks |

## 2. The tone curve: `display_transform`

How the scene's light, which has no upper limit, is brought into a picture:

| Value | Look |
|---|---|
| `PBR Neutral` | True product colour: a brand red stays that red. The default, and right for catalogue and e-commerce |
| `AgX` | Filmic: bright highlights roll off softly and go pale rather than clipping. For dramatic light, glowing parts, hot highlights on metal |
| `ACES` | The film industry's curve: contrasty, saturated, a little darker |
| `sRGB` | No curve: what lights above white there are simply clip. Matches other software's "Standard" view |
| `Raw` | The numbers as they are, for checking |

A bright saturated colour looks paler under `PBR Neutral` than under `sRGB`: that is the curve protecting highlights. When a colour must match a swatch exactly, compare under `sRGB` with `exposure` 0.

## 3. What to change for what was asked

- **"Brighter" / "darker":** `exposure`, a third to two thirds of a stop at a time.
- **"More punch":** `contrast` 1.1-1.2 and `blacks` a little below 0.
- **"Too harsh", blown highlights:** `highlights` -0.3 to -0.5, or `display_transform` `AgX`.
- **"Warmer" / "cooler":** `temperature` 500-1000 kelvin at a time.
- **White background looks grey:** raise `exposure` until it is white rather than brightening materials; if the product then blows out, the background needs to be a `Color` background or brighter pins instead (page `lighting`).
- **"More premium", "moodier":** `vignette` 0.2, `contrast` 1.1, a slightly lower `exposure`; the light does the rest.
- **Glowing parts should glow:** `bloom` 0.1.

Then `tequila_screenshot` (rendered view) and compare. One `image.set_style` is one undo step.

## 4. Colour management with an OCIO config

A studio that works to its own OpenColorIO config chooses a display and a view from it, in place of the built-in curves:

```json
{"command": "image.set_color_management", "color_management": {"config": "C:/studio/config.ocio", "display": "sRGB", "view": "ACES 1.0 - SDR Video"}}
```

`"color_management": null` returns to the built-in transforms. Only with a config file the person gave; the names of its displays and views are the config's own.

## 5. Denoising

Grain is cleaned by the denoiser, part of the render settings rather than the style: `denoise` on or off and `denoise_strength` (page `rendering`). A picture that looks smeared or plasticky has too few samples under a full-strength denoise: more samples, or strength 0.5-0.7.

## Good to know

- The style belongs to the project, and a studio preset brings its own: applying a preset replaces the look (page `variants`).
- The style is baked into PNG, JPEG and TIFF renders. An EXR keeps the scene's light as it is, to be developed elsewhere.
- Do not fix a lighting problem with the style: a shadow that is too deep everywhere is `shadows`; a product lit from the wrong side is the light (page `lighting`).
- The Image tab shows these as the person sees them: `tequila_ui {"tab": "image"}`.
