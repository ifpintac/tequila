# Files: importing, updating, saving, exporting

Everything here reads or writes files on the person's computer, so do it **only when asked**, with paths the person gave. Inside Tequila's Assistant the person is asked to confirm each one; call the tool and wait. Paths are full paths on the person's computer (`C:/Users/…` or `/Users/…`); forward slashes work on Windows.

## 1. Bring a model in

```json
tequila_import {"path": "C:/models/kettle.step"}
tequila_import {"path": "C:/models/bracket.stl", "up": "z"}
```

| Kind | Files |
|---|---|
| CAD, divided into parts with their names, colours and layers | STEP (`.step`, `.stp`), IGES (`.igs`, `.iges`), Rhino (`.3dm`) |
| Meshes | glTF (`.gltf`, `.glb`), FBX, OBJ, STL, 3MF, PLY |
| Scenes from other software | USD (`.usd`, `.usda`, `.usdc`, `.usdz`), Blender (`.blend`), IFC |

- The model is added to the scene, standing on the floor, as one undo step. The answer has how many `nodes`, `meshes` and `triangles` came in, the `unit` the file declared, `unit_was_guessed` when it declared none, and `warnings`.
- A big STEP assembly takes a minute or more to read; the tool answers when it is in.
- `up`: which way up the file was drawn - `z` for a CAD model that would otherwise lie on its back, `y`, or `file` to trust what the file says. Left out, the person's setting decides.
- `lighting`, for a Blender file with lights of its own: `blender` (light the model with them), `tequila` (keep the scene's lighting), `ask`.
- **After importing, look:** `tequila_scene` for the size (a wrong size means missing units: page `fixing`), `tequila_frame`, and a screenshot.
- KeyShot scenes (`.ksp`, `.bip`) open as projects from the window's File menu, not through import.

## 2. The model changed: update it

When the design moves on in CAD, update the model in place. Parts are matched to the new file one by one; materials, names, labels and edits stay on the parts that are still there.

```json
tequila_request {"request": {"op": "reimport", "node": 3}}
tequila_request {"request": {"op": "reimport", "node": 3, "path": "C:/models/kettle_rev_B.step"}}
tequila_request {"request": {"op": "reimport", "node": 3, "tessellation": "high"}}
```

- `node` is any node of the model. Without `path` it reads the file the model came from again; with it, a newer version saved under another name.
- `tessellation` (`draft`, `medium`, `high`, `ultra`), with no `path`, divides a CAD model's curves again, finer or coarser, from the CAD data kept in the project: the file is not needed.
- The answer says how many parts `matched` and lists the ones `added` and `removed`. New parts arrive with the file's own colours and need dressing; removed parts are hidden rather than deleted, so nothing is lost if the new file was wrong.
- One undo step.

## 3. Save

```json
tequila_save {}
tequila_save {"path": "C:/work/kettle-red.tequila"}
```

With nothing, the project is saved to its own file (refused, saying so, when it has never been saved: then a path is needed). With a path it is saved **as** that file and the project now lives there, as Save As does. `tequila_query {"what": "history"}` says whether there is `unsaved` work and the project's `path`.

Never save as a side effect of other work, and never over a file the person did not name.

## 4. Export for other software

A request, sent with `tequila_request`:

```json
tequila_request {"request": {"op": "export", "format": "glb", "path": "C:/out/kettle.glb"}}
tequila_request {"request": {"op": "export", "format": "usdz", "path": "C:/out/kettle.usdz"}}
tequila_request {"request": {"op": "export", "format": "web", "path": "C:/out/kettle-site", "live": true}}
```

| `format` | Writes | For |
|---|---|---|
| `glb` | One glTF binary of what is shown, with its materials | Web viewers, game engines, other 3D software |
| `usd` | A `.usda` text file | Pipelines built on USD |
| `usdz` | One file at the product's real size | AR on iPhone, iPad and Vision Pro |
| `web` | A folder that is a static website: the configurator with a live 3D view (`"live": true`); `"photos": true` also queues a render of every combination | Putting a configurator on a site |
| `spin` | A folder of pictures and a page that turns them (page `animation`) | A 360-degree view on a page |

- The answer has the `path` written, its size in `bytes`, any `warnings` (a material that could not be carried over exactly: procedural patterns are baked or simplified), and the `jobs` queued when renders are part of it.
- Only what is visible is exported: hide what should stay behind first.
- **There is no export to STEP, OBJ, STL or FBX.** Say so; `glb` is the format other 3D software reads.
- For pictures, not models: page `rendering`.

## 5. Materials and studios as files

- A material from the person's library to share: `{"op": "library", "action": "export", "name": "Brand Red Gloss", "path": "C:/out/brand-red.fsmat"}`, and into the library from such a file: `"action": "import"` with its `path`.
- A studio preset to share: `{"op": "studio_preset", "action": "export", "preset": "<its id>", "path": "<where to write the package>"}`; `"action": "install"` with the `path` of a package brings one in.

Both go through `tequila_request`.

## 6. Another project

Opening another project or starting a new one closes the one that is open, so ask the person first, and say what happens to unsaved work:

```json
tequila_request {"request": {"op": "open", "path": "C:/work/lamp.tequila", "save": true}}
tequila_request {"request": {"op": "new", "discard": true}}
```

Each is refused over unsaved work unless it says `save` or `discard`. Never `discard` without the person saying so. An assistant running on an API key or a local model is refused both requests: there, opening and starting a project are the person's to do, in the File menu.

## Good to know

- A path that does not exist is refused by name: ask the person for the right one rather than guessing at folders.
- A model's textures are found beside its file; a glTF or FBX moved without its texture folder comes in grey.
- Imported CAD keeps its part numbers and properties as `metadata` on each node (`tequila_read {"node": 12}`), and its layers.
- A live link from a modelling program sends the model across without files; it is set up from that program's side with Tequila's add-on for it, not through these tools.
