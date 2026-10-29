---
name: tequila
description: Work in Tequila, a product-rendering application (like KeyShot), through its tequila_* tools - inspect and organise the open scene, put library materials on parts, invent procedural materials from recipes, light it, set up floor reflections, cameras, studios and configurators, render, and check results with screenshots - and extend Tequila by writing plugins (panels, automations, batch jobs) or Python scripts for it. Use whenever the user mentions Tequila, a product render or shot, materials on a 3D model, a Tequila plugin or script, or asks for anything in an open 3D scene while tequila_* tools are available.
---

# Tequila

Tequila renders products photographically. It is open on the user's computer, and you work in it through the `tequila_*` tools, which drive its scripting channel: every change is exact, undoable, and shows in Tequila's console as you make it.

## The rule

**Work through the `tequila_*` tools. Never click, type into or screenshot the desktop to operate Tequila** - no computer use, even if it is available. The tools do everything the window does, faster and undoably.

If no `tequila_*` tools are available, do not fall back to controlling the screen. Tell the user how to connect, then stop:

> In Tequila, open the **Assistant** (toolbar), click **Connect an AI…**, then **Connect the Claude app**. Quit Claude completely (right-click its icon by the clock → Quit) and open it again. Tequila must be open when you ask me.

If a tool says Tequila does not seem to be open, ask the user to open it. If the plugin or `tequila_request` tools are missing, the connection is restricted (remote); say what you cannot do from there.

## What you can do, and where to look

| The user wants | Use | Read |
|---|---|---|
| To know what is in the scene | `tequila_scene`, `tequila_nodes`, `tequila_materials` | - |
| A real material on parts (aluminium, walnut, ABS...) | the library: `{"op": "library", "action": "apply", ...}` | `reference/materials.md` |
| A material of their own: terrazzo, camo, worn paint, galaxy resin | recipes: `tequila_material_preview` → `create` → `update` | `reference/recipes.md` |
| A colour, finish or roughness changed | `material.set_parameter` (colours as hex) | `reference/materials.md` |
| A shot: studio, black, white e-commerce, cut-out, HDRI, jewellery | lighting, environment, floor, camera, image style | `reference/shots.md` |
| Parts renamed, grouped, hidden, moved, split; units; lights; cutaways; measurements | `scene.*` commands | `reference/scene.md` |
| Colourways, variants, studios, a configurator, renders of every option | multi-materials, studios, configurator, `tequila_queue` | `reference/scene.md` |
| A render | `tequila_render` (quick look) or `tequila_queue` (final) | `reference/shots.md` |
| A new button, panel, automation or batch job inside Tequila | a plugin: `tequila_plugin_*` | `reference/plugins.md` |
| Work without the window, or a repeatable script | the Python client | `reference/scripting.md` |
| Any command's exact fields | `tequila_query {"what": "commands"}` | `reference/commands.md` |

## How to work

1. **Look first.** `tequila_scene` for the overview, then `tequila_nodes` (use `parts_only: true` and `filter` to keep it short) for parts with their ids, materials and sizes, and `tequila_materials` for materials. Ids come only from these answers - never guess one.
2. **Plan, then change in batches.** Several edits that belong together go in one `tequila_batch` (one undo step; with `atomic` true, if one fails none stay).
3. **Look at the result.** `tequila_screenshot` with `viewport: true` shows just the picture. For a true-to-render check, switch the viewport with `tequila_ui {"viewport": "rendered"}`, wait a few seconds for it to settle, and screenshot again. Compare against what was asked, and refine.
4. **Undo freely.** `tequila_undo` takes back the last step - one edit, or one whole batch.
5. **Say what you did**, briefly, in the user's terms: which parts, which materials, what changed.

Do not import files, save, or render to a file unless the user asks.

## Materials

**Prefer the library.** Tequila ships 309 realistic presets - procedural metals, plastics, woods, fabrics, stones, glass, gems, liquids, leathers, paints, composites. List them with `tequila_query {"what": "library"}`, then put one on parts:

```json
tequila_request {"request": {"op": "library", "action": "apply", "name": "Aluminium Brushed", "nodes": [12, 14]}}
```

That is one undo step. The listed parts share one material made from the preset; add `"unlink": true` to give them a separate copy. Leaving out `nodes` uses the selection.

**Nothing in the library fits?** Make one from a recipe - a type, a few parameters and procedural patterns, layered with masks - with the `tequila_material_*` tools. Read `tequila_material_guide` once (it has 16 tested examples to start from), preview on a ball, create it on the parts, look at it on the product, update. `reference/recipes.md` is the short version.

**Parts share materials.** Two parts can wear one material; changing it changes both. Before editing one part's material alone, give it its own: `{"command": "material.unlink", "node": 12}`.

**Editing a material** (after `tequila_materials` for its id):

- `{"command": "material.set_type", "material": 3, "type": "metal"}` - plastic, metal, glass_solid, paint, velvet… (`reference/materials.md`)
- `{"command": "material.set_parameter", "material": 3, "parameter": "base_color", "value": "#c0392b"}`
- `{"command": "material.set_parameter", "material": 3, "parameter": "specular_roughness", "value": 0.25}`
- `{"command": "material.assign", "nodes": [12], "material": 3}` - put an existing material on parts.

**Colours:** give material and environment colours as **hex sRGB strings** (`"#c0392b"`) - Tequila converts them into its rendering space (linear ACEScg). An array `[r, g, b]` is taken as ACEScg as it is, so never send sRGB numbers as an array.

## Lighting, background and floor

- **Lighting preset:** `{"command": "lighting.set_preset", "preset": "Product"}` - Performance, Basic, Product (default for products), Interior, Jewelry (gems and glass).
- **Environment and floor:** `{"command": "environment.set_parameter", "parameter": "<key>", "value": <v>}`. Keys: `rotation` (radians), `brightness`, `contrast`, `background` (`"Environment"`, `"Color"`, `"Backplate"`), `background_color`, `image` (an HDRI path), `ground_enabled`, `ground_shadows`, `ground_shadow_color`, `ground_occlusion`, `ground_flatten`, `ground_size`.
- **Floor reflections, V-Ray style** - the product mirrored on an invisible floor: `ground_reflections` 0-1 (0.3-0.6 typical), `ground_reflection_blur` 0-1 (0.05-0.2 polished), `ground_reflection_length` 0-1 (0.2-0.35 for the short faded launch-shot reflection), `ground_reflection_fresnel` true to fade it from above.

Recipes for the shots people ask for are in `reference/shots.md`.

## Camera

- `tequila_frame` - fit everything (or, with `all: false`, the selection) in view.
- `tequila_look` - a standard view (`front`, `back`, `left`, `right`, `top`, `bottom`, `iso`), or `yaw`/`pitch` in degrees around a target `at`, with `distance`, or an exact `from`; `focal_length` in millimetres (50 natural, 85-100 flatters products). Undoable.
- Saved cameras (`camera.add`, `camera.select`, `camera.rename`) keep shots to come back to and render each: `reference/scene.md`.

Lengths are metres. A part's `size` from `tequila_nodes` tells you how big it is: a phone is about 0.15 m. If a model is absurdly large - a hand tool 120 m long - it came from a file with no units (usually millimetres). Do not rescale it yourself; tell the user: **right-click the model in the Scene tab → Units of this model… → Millimetres**. Materials and recipes are sized for real objects, so this matters.

## Rendering

- **A quick look:** `tequila_render` with small values (`width` 640, `height` 400, `samples` 32) returns the image to you. It blocks Tequila while it runs - keep it small.
- **A final render:** `tequila_queue` (optional `name`, `width`, `height`, `samples`, `denoise`, `camera` - a saved camera's index) renders in the background with the project's settings and writes the file; `tequila_jobs` shows progress and the output path. Tell the user where it went.
- **Every option of a configurator:** `tequila_request {"request": {"op": "queue", "configurations": true}}`.
- **Settings** (engine, size, format, transparency): `render.set_settings` replaces the *whole* block - read it from `tequila_query {"what": "context"}` (the document's `render`), change what you need, send all of it back. Same for `image.set_style`. `reference/shots.md`.

## Plugins: extending Tequila

When the user wants something Tequila does not have - a panel with a button, something that happens on every import, a batch job they will run again - write them a plugin. A plugin is a folder with `plugin.json` and a Python file; Tequila draws its panels and runs its code against the same API you use.

1. `tequila_plugin_guide` once: the reference, this build's field types and events, and a template.
2. `tequila_plugin_write` with `plugin.json` and `main.py` (id like `my.batch-renamer`).
3. `tequila_plugin_enable` - Tequila asks the **user** to turn it on (it runs Python as them). Tell them to click **Turn it on** in Tequila, then check `tequila_plugin_list`.
4. `tequila_plugin_run` a command: you get what it logged, `finished`, or the traceback. Fix, write again (a running plugin restarts with the new code), run again.

`reference/plugins.md` has the manifest, the Python API, the panel fields and events, and patterns for the common plugins. Plugins need Python 3 on the user's computer; `tequila_plugin_list` says whether it was found.

## When something goes wrong

- A refused command comes back as an error message naming the field: read it, fix the request, try again.
- A connection error means the outcome of that request is unknown. Do not repeat edits blindly: check `tequila_query {"what": "history"}` and the scene first.
- A pattern or texture looks like nothing: the model's units are wrong, or the pattern `size` is far off the part's size.
- Rendered view black, or "standing in for Cycles, which could not render": Cycles could not run (usually the graphics card or its driver); Tequila's own renderer draws the view on the processor instead. Suggest updating the graphics driver; carry on meanwhile.
- A plugin "failed": `tequila_plugin_log` has the traceback; `tequila_plugin_list` shows `failure` (no Python, a syntax error, a crash).

## Reference

- `reference/materials.md` - library categories and names, material types, the parameters that matter.
- `reference/recipes.md` - procedural materials from recipes: format, patterns, layering, rules of thumb.
- `reference/shots.md` - the common shots, render settings, the image style.
- `reference/scene.md` - the scene tree, transforms, lights, cutaways, measurements, cameras, colourways, studios, configurators, export.
- `reference/plugins.md` - writing plugins: manifest, API, panels, events, patterns, pitfalls.
- `reference/scripting.md` - the Python client and raw requests.
- `reference/commands.md` - every command and operation with an example (generated from this build).
