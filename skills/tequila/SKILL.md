---
name: tequila
description: Work in Tequila, a product-rendering application (like KeyShot), through its tequila_* tools - inspect and organise the open scene, put library materials on parts, make procedural materials, edit material graphs, labels and UVs, light it, set up the floor, cameras, colourways, Snapshots and configurators, render, tune the image, make turntables, fix models that came in wrong, import and export - and extend Tequila with plugins or Python scripts. Use whenever the user mentions Tequila, a product render or shot, materials on a 3D model, a Tequila plugin or script, or asks for anything in an open 3D scene while tequila_* tools are available.
---

# Tequila

Tequila renders products photographically. It is open on the person's computer, and you work in it through the `tequila_*` tools: every change is exact, shows in Tequila as you make it, and can be undone. This page is how to work; the rest of the guide is one page per area, opened with `tequila_guide` (in a skill folder, the same page is `reference/<page>.md`).

## How to work

1. **Open the guide's page first.** Your first call for any job is `tequila_guide {"topic": "<page>"}`, with the page from the table below. Do not work from what you know of other software: Tequila has its own ways (the background and the floor are settings of the environment, not a plane you add; a material comes from the library, not from parameters you make up).
2. **Use the tools, never the screen.** No clicking or screenshotting the desktop to operate Tequila, even if you can. With no `tequila_*` tools, say how to connect and stop: in Tequila, **Assistant → Connect an AI…**, then quit the AI app completely and open it again.
3. **Look before you change.** `tequila_scene` for the overview; `tequila_nodes` (with `parts_only: true` and a `filter` word) for parts with their ids, materials, sizes and centres; `tequila_materials` for materials. Ids come only from these answers: never guess one.
4. **Never guess a field.** Before a command you have not used here, ask `tequila_command {"command": "<name>"}`: it answers with the exact fields and an example to copy. A field that is not in the example is not read.
5. **Read a block before you send it back.** Many commands replace a whole block (the render settings, the image style, a camera, a light). `tequila_read {"what": "render"}` reads that one block, `tequila_read {"node": 12}` one node with its transform; change what was asked and send all of it back. Never read the whole project (`tequila_query` `context`, `api`, `library`): tens of thousands of characters.
6. **Change only what was asked, in batches.** Edits that belong together go in one `tequila_batch`: one undo step, and if one fails none stay. Leave the product's own materials, parts and camera alone unless the job is about them.
7. **Check the picture.** After changing how something looks: `tequila_ui {"viewport": "rendered"}`, then `tequila_screenshot`. It says whether the picture had finished; if not, ask again with a longer `settle` rather than judging the grain. Compare with what was asked, and fix what is off.
8. **Undo freely.** `tequila_undo` takes back the last step, one edit or one whole batch.
9. **Stay in the scene unless asked.** Import, save, render to a file, export, change a preference or write a plugin only when the person asks.
10. **Say what you did**, briefly: which parts, which materials, what changed.

## Materials: the library first

For any material, **search the library before anything else**: several hundred finished presets, plus the person's own.

```json
tequila_library {"search": "brushed aluminium"}
tequila_library_apply {"name": "Aluminium Brushed", "nodes": [12, 14]}
```

Every word of `search` must match: try fewer if nothing comes back. Wrong colour? `tequila_material_tint {"material": 7, "color": "#1f4d2b"}`, or `color` on `tequila_library_apply` - not `base_color`, which many presets do not use. Otherwise close? Put it on and change it (page `materials`). Make one from nothing only when the library has nothing close (page `recipes`). Parts can share a material: `material.unlink` a part before changing its material alone.

## Which page for which job

| The person wants | Page |
|---|---|
| A real material on parts; the same one in another colour; a finish or roughness changed | `materials` |
| A material of their own: terrazzo, camo, worn paint, a pattern | `recipes` |
| One parameter driven by a pattern or a ramp; nodes in the material graph | `graph` |
| A picture as a texture; a pack of PBR maps; a Substance file | `textures` |
| A logo, print or sticker on a part | `labels` |
| A part unwrapped, its UVs cut, packed or exported for artwork | `uv` |
| Parts renamed, grouped, hidden, moved, copied in a row; a pedestal or backdrop; a cutaway; a dimension | `scene` |
| Lighting, an HDRI, a softbox, the background, the floor's shadow and reflection | `lighting` |
| A view, a lens, depth of field, a saved camera, composition guides | `camera` |
| Colourways, saved set-ups (Snapshots), a configurator, a whole studio at once | `variants` |
| A render: quick or final, size, format, transparent background, layers, every view | `rendering` |
| Brighter, more contrast, warmer, a vignette, a filmic look | `image` |
| A turntable or a 360-degree spin | `animation` |
| A model that came in too big, as one lump, faceted, lying down or with parts missing | `fixing` |
| A file brought in, updated from its file, saved, or exported (glTF, USD, AR, web) | `files` |
| A button, panel, batch job or automation inside Tequila | `plugins` |
| A panel opened, the viewport mode, Tequila's own settings and shortcuts | `app` |
| Something no tool covers; a script to run again; work with no window | `scripting` |
| A whole shot from a description: clean studio, black, white, cut-out, jewellery | `shots` |

## What holds everywhere

- **Lengths are metres and +Y is up.** A phone is about 0.15; a hand tool 120 m long came from a file with no units (page `fixing`).
- **Colours are hex sRGB strings** (`"#c0392b"`), converted for you into the rendering space. An array `[r, g, b]` is taken as linear ACEScg as it is: never send sRGB numbers as an array.
- **A command** is `{"command": "<group>.<verb>", ...}` sent with `tequila_apply`, or as `{"op": "apply", "command": {...}}` inside `tequila_batch`. **A request** is `{"op": "<name>", ...}` sent with `tequila_request` where no tool of its own exists. Angles are radians in commands, degrees in `tequila_look`.
- **The view is the person's as well.** They turn and zoom it while you work, and no material edit moves the camera, so a view unlike your last look is theirs: do not frame it or move it back. Move it only when asked or to see something out of sight, then put it back (page `camera`).
- **Snapshots** in the window are `studios` in commands; a **studio preset** is a whole lighting set-up from the library.
- Inside Tequila's Assistant, **Tequila asks the person itself** before a file is read or written: call the tool and wait. If they say no, do not try another way.

## When something goes wrong

- A refused command says which field is wrong: fix that and send it again. Two refusals running mean you are guessing: ask `tequila_command`, or read the page.
- A connection error means the outcome is unknown: check `tequila_query {"what": "history"}` and the scene before repeating an edit.
- A change does not show: is the part hidden (`visible` in `tequila_nodes`), is it where you think (`center`, `size`), had the picture finished?
- A page says a job is not possible through the tools yet: say so plainly and tell the person where it is in the window, rather than trying another way round.
