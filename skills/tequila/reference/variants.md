# Colourways, Snapshots, configurators and studios

Four things that build on one another: a part's **colourways** (Multi-Materials), which parts are shown (**model sets**), a saved set-up of the whole scene (**Snapshots** - called `studios` in commands), and the choices a customer makes (**the configurator**). Last, **studio presets**: a whole lighting set-up from the library.

Each is a list the project keeps, **sent whole**: read it with `tequila_read`, change it, send all of it back. A list the project does not have yet reads as `null`.

## 1. Colourways: a part with several materials

Make the materials first - library presets put on with `"unlink": true`, recipes, or copies you recoloured (page `materials`) - and note their ids from `tequila_materials`. Then:

```json
tequila_read {"what": "multi_materials"}
{"command": "material.set_multi_materials", "multi_materials": [
  {"id": 1, "name": "Shell colours", "parts": [21, 22], "variants": [2, 14, 16], "active": 0}]}
```

- `parts` wear whichever variant is `active` (its place in `variants`, from 0). `variants` are material ids. `id` is yours to choose, unique among the multi-materials.
- Switching colour is sending the list again with another `active`. Keep every other multi-material in the list as it was.

## 2. Model sets: which parts are shown together

With and without the lid, the long and the short strap:

```json
{"command": "scene.set_model_sets", "model_sets": {"active": 0, "sets": [
  {"name": "With stand", "nodes": [2, 7]},
  {"name": "Without stand", "nodes": [2]}]}}
```

The parts of the `active` set are shown and the parts only other sets name are hidden. `"always_visible": true` on a set keeps it shown whichever is active (the body every version shares). An empty `sets` list removes them.

## 3. Snapshots: a saved set-up

A Snapshot (`studio` in commands) remembers a combination: which colourways are active, which model set, how the product is turned, the environment, the camera. Applying one makes the scene that, in one undo step.

```json
tequila_read {"what": "studios"}
{"command": "studio.set", "studios": [
  {"name": "Red hero", "variants": [{"multi_material": 1, "active": 0}]},
  {"name": "Steel, from the right", "variants": [{"multi_material": 1, "active": 2}], "product_yaw": -0.94, "filename_suffix": "Right"}]}
{"command": "studio.apply", "index": 1}
```

- A Snapshot holds only what it names: one with just `variants` switches colours and leaves light and camera alone. `product_yaw` (radians) turns the product; `model_set` picks a set; `environment` and the lights' on-off states can be kept too - copy those blocks from `tequila_command {"command": "studio.set"}`.
- `studio.set` only edits the list; `studio.apply` shows one, by its place from 0.
- **Turn the product** without a Snapshot: `{"command": "studio.turn_product", "yaw": 0.5}`. The camera and the light stay, so highlights fall on a new side.
- **Render every Snapshot**, one picture each, named for it: `tequila_queue {"studios": true}`; one of them: `tequila_queue {"studio": "Red hero"}` (page `rendering`).

## 4. The configurator

Options a customer chooses, each driving a multi-material, a model set or a Snapshot:

```json
{"command": "configurator.set", "configurator": {"title": "Headphones", "subtitle": "Pick a finish", "placement": "bottom", "options": [
  {"id": 1, "name": "Shell", "drives": "material", "multi_material": 1, "style": "swatches",
   "choices": [{"label": "Red", "target": 0, "swatch": "#c23b32"}, {"label": "Ocean", "target": 1, "swatch": "#2d5da8"}]},
  {"id": 2, "name": "View", "drives": "studio", "style": "buttons",
   "choices": [{"label": "Hero", "target": 0}, {"label": "Right", "target": 1}]}]}}
{"command": "configurator.choose", "choices": [1, 0]}
```

- `drives` is `material` (with `multi_material`; a choice's `target` is the variant's place), `model_set` or `studio` (`target` is the set's or Snapshot's place). `style` is `swatches`, `buttons` or `list`; a `swatch` is the colour of the chip.
- `configurator.choose` shows a combination: one choice per option, in order. Refused when it breaks a rule.
- `rules` tie choices together: `{"kind": "excludes", "option": 1, "choice": 1, "other": 2, "other_choice": 0}` means option 1's second choice cannot go with option 2's first; `"kind": "requires"` means it can only go with it.
- **See it as a customer does:** `tequila_request {"request": {"op": "ui", "present": true}}` puts the configurator over the view, full screen; `false` leaves.
- **Render every valid combination:** `tequila_queue {"configurations": true}`, one job each, named by its choices. Count them first: three options of four choices are 64 renders. Say the number before starting.
- **A web configurator** the person can put on a site: page `files`.

## 5. Studio presets: lighting, cameras and look from the library

A studio preset is not a Snapshot: it is a whole photographic set-up - lights, pins, floor, several named camera views, image look, render settings - applied to any product and sized to it.

```json
tequila_studio_presets {}
tequila_studio_preset_apply {"preset": "tequila-catalogue-studio", "front": "+z", "dress": false}
tequila_studio_render_all {}
```

- The list has each preset's `id`, `name`, `description` (the look and what it renders), and `suggested: true` on the size nearest the product in each family.
- Applying is one undo step, and its answer counts the `lights`, `cameras` and `views` it brought. The views arrive as Snapshots, so `tequila_read {"what": "studios"}` afterwards lists them by name (a hero view, front, side, back, top); the first is shown. The product's materials are never changed, unless `"dress": true` asks for the preset's own materials on parts by name.
- `front` is the product's side that should face the Main view. `keep_lights`, `keep_cameras` and `keep_placement` keep what the scene has; `scale` (`cameras`, `cameras_and_lights`, `none`) says what follows the product's size.
- `tequila_studio_render_all` queues every view, one file each. Only when asked for renders.
- `tequila_studio_preset_save {"name": "My launch studio"}` keeps the scene's current studio as the person's own preset, to use on the next product. Only when asked.

## 6. Material Templates: dress parts by rule

Rules that put materials on parts by name, colour, CAD property or the material they came in with - for dressing every new version of a model the same way:

```json
{"command": "material.set_templates", "templates": [{"name": "Fasteners", "rules": [
  {"match": "name", "pattern": "bolt*", "material": "mat_00000010"},
  {"match": "material", "pattern": "Steel*", "material": "mat_00000010"}]}]}
```

A rule's `material` is the material's name in the file, which `tequila_material {"id": 16}` gives as `definition.id` (`mat_00000010` is material 16, the id in hexadecimal). The person applies a template from the Material tab; setting the list does not dress anything by itself.

## Good to know

- Snapshots, model sets and colourways all live in the window's **Snapshots** tab.
- Changing a material that is a variant changes that colourway everywhere it is used.
- Deleting a material that a multi-material names leaves a hole: take it out of `variants` first.
- A Snapshot's index changes when the list is reordered; read the list again before `studio.apply` if it may have changed.
