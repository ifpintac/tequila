# Tequila scripting for people and AI

Tequila exposes the running application's command bus over authenticated
loopback TCP. Scripts edit the same document as the UI, with journaling and
undo. This build adds an external Python client; it does not embed Python or
Blender's `bpy`, and the in-app console takes short commands or JSON, not Python code.

## Start here

### The console

Press **`** (backtick), or click **Console** on the toolbar, to open a terminal
under the viewport. Type a command and press Enter. **Tab** completes the first
word, **Up**/**Down** step through what you've typed, and **Esc** leaves the
prompt so the viewport shortcuts work again.

```text
› context                 everything a script needs: scene, parts, materials, camera
› parts strap             a table of parts whose name or material contains "strap"
› materials               every material, its type, how many parts use it, its maps
› select 360 361
› frame sel
› material.set_type material=2 type=velvet
› material.set_parameter material=2 parameter=specular_roughness value=0.3
› undo
› render out.png 1200x800 64
› run setup.txt           one command per line, # for comments
› help
```

Every line becomes exactly the request an external client would send, and goes
through the same handler. So anything that works here works from a script, and
the other way round. `{...}` on its own sends a raw request, and `apply {...}`
sends any command as a journal line. `examples` lists the worked examples and
`example 3` puts one at the prompt to read before running. Values in
`key=value` are typed by what they look like: numbers, `true`/`false`,
`[0.8, 0.8, 0.8]` for colours and vectors, anything else as text. Quote a value
to force text (`name="123"`) or to keep spaces in a path.

Requests from outside the application (a script, or an AI assistant) appear in
the same scrollback **in amber**, with where they came from, as they happen.
Failed requests appear in red with the reason. While one is acting, the toolbar
shows **⚡ Scripting**.

The **Scripting** tab in the Inspector holds the connection details, the full
activity log, and **Copy instructions for an AI assistant**. That button gives
an agent the connection and workflow instructions without copying the session
token to the clipboard.

Use a local Python 3.10+ installation and the distribution's `python` folder:

```powershell
cd python
python inspect_scene.py > scene-context.json
```

No pip dependencies are required. From another script, put this folder on
Python's module search path, then import `Client` from `tequila`.

```python
from tequila import Client

with Client(timeout=60) as fs:
    api = fs.api()          # supported operations, examples, parameters/ranges
    context = fs.context() # one-frame snapshot of the open scene
    parts = context['nodes']['nodes']
    candidates = [p for p in parts if p['name'] == 'Housing']
    if len(candidates) != 1:
        raise ValueError('Resolve the intended part before editing')
    part = candidates[0]
    fs.select([part['id']])
    fs.frame(all=False)
    # Make this part independent before changing a shared material.
    fs.apply('material.unlink', node=part['id'])
    updated = next(p for p in fs.query('nodes')['nodes'] if p['id'] == part['id'])
    fs.set_material_parameter(updated['material'], 'specular_roughness', 0.3)
    fs.screenshot('housing-check.png')  # choose a new filename
    # Save only to the project/output path intended for this work.
    fs.save('housing-study.tequila')
```

### Without the window: `tequila.open()`

`tequila.open(path)` starts a Tequila of the script's own - `tequila
--serve`, the command-line renderer holding a project, with no window - and
ends it when the session closes. It answers the same protocol with what
needs only the document: queries, `apply`, batches, undo, import and
reimport, `frame` and `look`, `render`, `export` and `save`. Screenshots,
panels and the background queue need the window and are refused by name.
It never touches the application's discovery file, so a script's session
does not take clients from the Tequila a person has open.

```python
import tequila

with tequila.open('kettle.tequila') as fs:        # or a model, or None
    lid = fs.find(name='Lid')[0]
    paint = fs.materials['Paint Base']
    paint.base_color = [0.72, 0.05, 0.06]           # one undo step
    lid.assign(paint)
    fs.commands.scene.set_metadata(node=lid.id, key='Finish', value='Gloss')
    fs.render('red.png', width=1200, height=800, samples=256)
    fs.save('kettle-red.tequila')                   # unsaved work ends with the session
```

`fs.commands` holds every command in the catalog as a call,
`fs.commands.<group>.<verb>(field=...)`, generated from the build
(`python/tequila_commands.py`) and listed with an example each in
[scripting-api.md](scripting-api.md). It works on the open application too
(`tequila.connect()`), and in a batch: `b.commands.scene.rename(...)` is
gathered into the batch rather than sent. `fs.materials`, `fs.parts()` and
`fs.find(name=..., metadata={...})` give the scene as objects whose methods
are those commands.

The renderer is found beside the `python` folder in a distribution
(`tequila-cli`), in a development build's `target`, or where `TEQUILA_CLI`
points.

## What an agent can understand

`query api` reports supported operations, command names, material types and
parameter kinds/ranges generated from the app's catalogs, plus examples tested
against the actual request parser. `command_examples` holds an example of
every command, exactly as `apply` takes it — the journal's own round-trip
samples, so each is known to parse. Their ids and keys are placeholders: read
the real ones from the scene, copy the example, and change what you need. It
is a discovery guide, not a JSON Schema.

`query context` combines selection, node IDs/names/parents/world bounds,
material users, full material definitions/graphs, and the saved scene metadata:
units, local transforms, lights, environment, textures, cameras and render
settings. Mesh vertex/index buffers are omitted; a part's `mesh` key says which
mesh it uses (copies of one shape share it) and `triangles` how big it is.
`document.tree` contains local transforms; node-summary bounds are world-space. Material definitions follow
the project file schema. Parameter keys follow the API catalog. Colors passed
to parameter commands are linear RGB, not 0–255 display colors, in the
rendering space (ACEScg primaries). Greys and gentle tints are the same as
linear sRGB; for a saturated sRGB colour, convert linear sRGB with the matrix
rows (0.6131, 0.3395, 0.0474), (0.0702, 0.9164, 0.0135),
(0.0206, 0.1096, 0.8698), or it renders more saturated than intended.

### CAD metadata and layers

Every node in `nodes` (and in `query nodes`) carries `metadata`, the
attributes its CAD file gave it - a part number, a description, the
material, properties of the user's own, by their names - together with
fields added since (`scene.set_metadata`), and `layers`, the names of the CAD
layers the file put it on, in the file's order (an IGES file's layers are its
levels, `"Level 5"`). `scene.layers` lists the scene's layers in the order
the tree meets them, each with the `nodes` on it and whether they are
`shown`: `"all"`, `"none"` (a layer the file had switched off arrives so), or
`"some"` where a part was shown or hidden by hand.

```json
{"scene": {"layers": [{"name": "Outer", "nodes": [4], "shown": "all"},
                      {"name": "Construction", "nodes": [6], "shown": "none"}]},
 "nodes": {"nodes": [{"id": 4, "name": "Box", "layers": ["Outer"],
                      "metadata": {"Material": "ABS"}}]}}
```

Show or hide a layer by its name, every node on it as one undo step:
`{"op":"apply","command":{"command":"scene.set_layer_visibility","layer":"Construction","visible":true}}`
(`fs.commands.scene.set_layer_visibility(layer=..., visible=...)`). A layer
no node is on is refused by name. A search, a Material Templates rule and the
CSV export read layers as the field `Layer`: `Layer = Outer` is every part on
Outer, whatever else it is on.

Use screenshots or preview renders to check spatial and aesthetic decisions.
Metadata cannot reliably identify semantic parts with anonymous names. Select
and frame candidates, examine a picture, then verify IDs and values again.
This API provides images; interpreting them requires a vision-capable client.

## Control

These are the operations beyond reading the scene and applying commands. Each
is available from the console, the wire protocol and the Python client.

| Console | Request | What it does |
|---|---|---|
| `a; b; c` | `{"op":"batch","ops":[...]}` | Runs several operations as **one undo step**. If one fails, the whole batch is taken back (`"atomic":false` keeps what worked). A console `run` file is a batch too. |
| `look front` | `{"op":"look","view":"front"}` | Uses a standard view: front, back, left, right, top, bottom or iso. |
| `look yaw=35 pitch=20 distance=1.5` | `{"op":"look","yaw":35,...}` | Orbits around `at`, or around the current target if `at` is not given. |
| `look from=[x,y,z] at=[x,y,z] focal=85` | `{"op":"look","from":[...],"at":[...]}` | Puts the eye at `from`, pointing at `at`. Every camera move is undoable. |
| `show material` / `hide console` | `{"op":"ui","tab":"material","console":false}` | Opens and closes panels: `render_window`, `console`, `inspector`, `library`, `full_viewport`, `help`, `frame` (the viewport at the render's shape, on by default), and Inspector tabs (`configurator` opens the Studios tab at the configurator); `present` enters or leaves Presentation mode; `about` opens Help > About; `menu` (`file`) opens that menu of the menu bar, for a screenshot of it; `start_screen` opens or closes the start screen; `updates` opens Help > Check for updates, which asks GitHub for the latest release; `connect` opens the Assistant's Connect an AI window; `viewport` picks `wireframe`, `solid`, `material` or `rendered`; `mode` (`light`, `dark`, `auto`) and `theme` (`graphite`, `ocean`, `orchid`, `ember`, `meadow`, `rose`) change the look for the session without saving it; `appearance` opens the picker. |
| `viewport rendered` | `{"op":"ui","viewport":"rendered"}` | Switches the viewport between rendered and solid. |
| | `{"op":"query","what":"api"}` → `procedurals` | Every pattern at its defaults, in the exact shape `material.set_procedural` takes as `procedural`. An unknown `kind` is refused rather than read as "detach". |
| | `{"op":"ui","graph":3}` | Opens the Material Graph editor on material 3; `"graph":false` closes it. |
| | `{"op":"ui","uv_editor":true}` | Opens the UV editor on the selected part; `false` closes it. |
| | `{"op":"ui","library_section":"studios","screenshot_folder":"C:/work/shots","save_screenshot":true,"window_size":[1280,800]}` | `library_section` shows the Library at `materials` or `studios` (the studio presets). The Project panel's Snapshots tab is `studios` to the channel (`tab`, `studio.set`, `studio.apply`, `queue studio=`): the tab was called Studios until November 8, and the names a script uses stayed. `screenshot_folder` is where P's Screenshots go, as View > Screenshot folder... chooses it, kept in the application's settings ("" for the default, Pictures/Tequila Screenshots). `save_screenshot` takes one, as P does: the viewport's picture without gizmos, outline or statistics, at the preview's own size, into that folder, with its notice. `window_size` sets the window's inner size in points. `query ui` reports `library_section`, `screenshot` (`folder`, the `last` one saved, and whether one is `waiting` for the graphics card's next frame) and `preview_denoise`, the viewport's own denoise switch (the render settings' `preview_denoise`). |
| | `{"op":"ui","studio_drag":[0,3]}` | For a picture of a Studio being dragged to a new place in the Studios tab: holds Studio 0 as though dragged to slot 3 (0 is above the first Studio, the count below the last), with the drop line and the Studio under the pointer drawn, and opens the tab. Nothing moves; `"studio_drag":false` lets it go. `query ui` reports `studio_pictures`: how many Studios show a picture of the scene as it is (`current`), of how many (`studios`), and whether one is `rendering`; the tab renders them while it is shown. |
| | `{"op":"ui","preferences":true,"spacemouse":{"reports":[[1,0,0,0,0,0,0,0,0,0,0,94,1]],"seconds":1}}` | `preferences` opens or closes Edit > Preferences (the 3D mouse's settings). `spacemouse` is made-up 3D mouse input, for a script or a test without a device: HID input reports as a device sends them, each its report id and then its bytes (report 1: six signed 16-bit little-endian numbers for translation, or twelve for all six axes; report 2: rotation; report 3: a button mask, bit 0 the left button and bit 1 the right), read exactly as a device's are. The motion they end on is held for `seconds`, then let go. The example twists the cap clockwise at full deflection for a second. `query ui` reports `spacemouse`: `enabled`, the `device` read (or null) and the six `axes` the view takes now, smoothed (pan x, pan y, zoom, tilt, spin, roll; 1 is full deflection). |
| | `{"op":"ui","uv":{"label":[3,0],"tool":"measure","zoom":2}}` | Drives the UV editor: `part` opens it on a part by id; `label` on a label, `[material, index]` (a part without a layout is unwrapped first, as the Labels tab's button does); `tool` is `select`, `move`, `rotate`, `scale` or `measure`; `zoom` is a number (1 is the layout's square filling the view) or `"fit"`; `select` selects labels by their place in the material's list, `texture` a texture on the layout by its parameter (`base_color`), `island` an island by its number; `align` aligns the selected labels as the Align panel does (`left`, `centre`, `right`, `top`, `middle`, `bottom`); `help` shows the shortcuts; `click` clicks a point `[u, v]` of the layout as a person would, picking a label, a guide or an island; `frame` frames the view on a box of the layout, `[[u, v], [u, v]]`. For a picture of a drag in progress, `pointer` puts the pointer at `[u, v]` of the layout, and `drag` holds a drag with the tool from `[u, v]` to `[u, v]`, snapping as a person's would; `release` lets it go as they would (the edit is made), and `"drag":false` lets it go without one. `query ui` reports the editor under `uv`: its `mode`, the `islands`, `edges` and `vertices` selected, the `pins`, the `set` shown, the `heatmap`, its last `message`, and what is `under` the pointer (island, face, edge, vertex). |
| | `{"op":"ui","uv":{"mode":"edges","edges":[[12,40]],"cut":true}}` | The layout's own, as the island tools do it. `mode` says what a click picks: `labels`, `islands`, `edges` or `vertices`. `islands` selects islands by number, `edges` edges by the mesh vertices at their ends, `vertices` vertices by mesh vertex (each switching to its mode), `select_all` everything the mode picks. Then: `cut` cuts along the selected edges; `stitch` stitches the selected seams, or the seams between the selected islands; `pin` `true` pins the selected vertices and `false` unpins them, each one step to undo (`uv.set_pins`), saved with the part; `reflatten` lays the selected islands (or all) out flat again with their pins, `repack` packing after; `pack` (`true`, or `{"margin":8,"unit":"px","texture":2048,"rotate":true,"into":"guides"}`, `unit` `mm` or `px`, `into` `square` or `guides`, the box between the outermost guides) packs the selected islands or all; `straighten`, `even` (even size) and `unwrap` (unwrap again, keeping the cuts) work on the selected islands or all. `transform` moves, turns and scales the selection about the Transform panel's chosen point: `{"offset":[du,dv],"turn":15,"scale":[1,0.5],"flip":"horizontal","quarter":1}` (`turn` in degrees anticlockwise, `quarter` clockwise as shown). Each is one step to undo, and a label on an island that moves goes with it; a refusal comes back as the editor's `message`. `pack_options` opens the Pack popover. |
| | `{"op":"ui","uv":{"heatmap":"angle","set":1,"export":{"path":"C:/work/shell.png","size":2048}}}` | `heatmap` shows the stretch heatmap, `area`, `angle` or `off`. `set` shows a UV set by number; `set_action` is `add`, `copy`, `delete`, `up` or `down` on the set shown, and `set_menu` opens the set picker. `export` writes the layout as the Export dialog does (`path` `.png` or `.svg`, `size`, `fill` `islands`, `none`, `area` or `angle`, `line` `"#rrggbb"`, `transparent`); `export_dialog` opens the dialog. The link to the part: `faces` picks faces of the part as the viewport does at Face level, `from_faces` selects their islands, `cut_faces` cuts them out of the layout as an island of their own, and `hover_face` holds a face under the viewport's pointer, so the editor lights it (`false` gives the pointer back); the editor's `pointer`, in turn, lights what is under it on the part in the viewport. `place` puts the selected labels on the island last picked: `layout` (printed by the layout rather than projected), `fit`, `centre`, `along` or `square`. |
| | `{"op":"ui","uv":{"cursor":[0.5,0.5],"align_to":"cursor","islands":[2,5],"align":"top_left"}}` | The arranging tools. `cursor` puts the 2D cursor at `[u, v]`, on the selection's middle (`"selection"`), the layout's (`"centre"`), or takes it away (`false`); `show_cursor` draws it; `pivot` `"cursor"` makes turns and scales go about it (`"frame"` gives them back). `align_to` is `selection`, `island`, `layout` or `cursor`, and `align` also takes `top_left`, `top_right`, `bottom_left` and `bottom_right`; in the Vertices and Edges modes it lines up vertices, or each picked edge path. `distribute` is `across`, `down` or `along` (a picked path), `spread` `gaps`, `centres`, `even` or `part`. Then, each one step on the selected islands or all: `square_edges` (`true` or a tolerance in degrees), `upright`, `sort` (`true` or `{"by":"x","flow":"row","align":"centre","spacing":8,"unit":"px","texture":2048,"wrap":"width","group":"shape","reverse":false}`, `by` `x`, `y`, `z`, `around_x`, `around_y`, `around_z`, `grid` (with `rows` and `columns` axes) or `layout`, `wrap` a count, `"width"` or `false`, `group` `none`, `size`, `shape` or `closeness`), `match_size` (`{"to":"largest","dimension":"width","keep_proportions":true}`, `to` `active`, `largest` or `average`), `fit` (`{"each":true}`), `stack` (`{"mirrored":true,"offset":[0.05,0],"tolerance":5}`), `copy_shape`, `paste_shape` (`{"in_place":true}`), `shuffle` (two swap) and `as_on_part` (`{"view":"front","turn":true,"true_size":true}`: `view` `front`, `side`, `top` or `auto`; `turn` turns each island to read as seen, `true_size` scales each to the layout's own scale). `density` reads and sets texel density: `{"texture":2048,"get":true}` reads it into the field, `{"apply":10.24}` (pixels per centimetre, or `true` for the field's) scales the selected islands to it. The Select menu: `select_similar` (`true` or a tolerance in per cent), `select_borders`, `select_square` (`across` or `down`), `select_no_area` and `select_flipped` (faces, picked on the part); `select_menu` opens it, `sort_options` Sort's options, `view_options` View options, and `panel` (`arrange` or `density`) scrolls the side panel to that panel. What it shows: `behind` (one of the material's UV textures behind the layout, by the parameter it drives, `checker` for the checker, or `none`), `exposure` (stops), `padding` (`true`, `false` or `{"texture":2048,"px":8,"suggest":true,"use_for_pack":true}`) and `checker` (`true`, `false` or `{"squares":8,"pattern":"letters","set":1}`, `plain`, `letters` or `numbers`, `set` a UV set's number from 0 or `"shown"` to follow the editor's; drawn in the viewport's Wireframe, Solid and Material Preview, on the graphics card or the processor, never in Rendered, nothing saved). `query ui` reports under `uv` the `cursor`, `pivot`, `align_to`, `exposure`, the shape `copied`, the `checker`, the `padding` (with how many islands are `too_close`) and the `density` (`px_per_cm`, `coverage` and `overlap` of the square). |
| | `{"op":"ui","uv":{"islands":[2],"unwrap_options":{"direction":"across","method":"least_stretch","keep":{"place":true,"size":true,"turn":false}},"unwrap":true}}` | The shape tools, as the Unwrap, Shape and Symmetry panels do them, each on the selection in its mode (every island when nothing is picked) and one step to undo; a refusal comes back as the editor's `message`. `unwrap_options` sets Unwrap again's `direction` (`both`, `across`, `down`), `method` (`conformal`, `least_stretch`) and `keep` (`true`, `false`, or `place`, `size`, `turn` each), which `unwrap` then uses. Each tool key is `true`, or an object of its options: `straight` (`keep_pins`), `grid` (`even`, `fit_square`), `rectangle` (`spacing` `surface` or `even`, `inside` `reflatten` or `smooth`, `sides` `average` or `longest`, `pin`), `project` (`separately`, `whole_island`, `scale`), `relax` (`rounds`), `circle` (`even`, `whole_region`), `grow` (a distance, negative to shrink, or `{"by":-4,"unit":"px","texture":2048}`), `weld` (a distance, or `{"within":2,"unit":"mm"}`), `match_seams` (`line_up` `height`, `across` or `place`; `move` `both` or `other`), `cut_angle` (an angle, or `{"angle":30,"unwrap_after":true}`). `shape_options` opens a tool's popover by name (`unwrap`, `grid`, `rectangle`, `project`, `match_seams`, `cut_angle`; `false` shuts them) and `shape_panel` scrolls `unwrap`, `shape` or `symmetry` into view. The `ui` query's `uv.shape` reports the settings, the stretch the last Unwrap again left, the part's mirror plane and share paired, the edges Cut by angle would cut while its options are open, and the selection's box. |
| | `{"op":"ui","uv":{"mode":"vertices","select_side":"+x"}}` | Symmetry: `symmetrize` and `snap_mirror` (`true`, or `plane` `auto`, `x`, `y`, `z`; `line` `vertical` or `horizontal`; `at` `layout`, `selection`, `fitted` (the line the layout is nearest symmetric about, through an island's places on the plane) or `cursor` (through the 2D cursor); `source` `auto`, `plus`, `minus`; `both_ways`); `select_side` selects what is on one side of the part's middle, `+x`, `-x`, `+y`, `-y`, `+z` or `-z`, in the mode there is; `select_mirror` (`true`, or `{"add":true}`) selects the mirror image of the selection. |
| | `{"op":"uv_layout","node":12,"path":"C:/work/shell.svg","line":"#204080","transparent":true}` | A part's layout written as a picture, the application's as the headless session's; see `docs/scripting-api.md`. |
| `ui` | `{"op":"query","what":"ui"}` | Shows what is open, and how far the viewport's picture has got: `viewport_samples`, and `viewport_preparing` while Cycles prepares the graphics card and has shown nothing. |
| `screenshot a.png viewport` | `{"op":"screenshot","path":...,"viewport":true}` | Captures just the viewport rather than the whole window. |
| `screenshot a.png graph` | `{"op":"screenshot","path":...,"window":"graph"}` | Captures one of the other windows: `render`, `graph`, `shortcuts`, `connect` or `uv` (the UV editor). They are windows of their own, so the main window's screenshot no longer shows them. Refused if that window is not open. |
| `livelink` | `{"op":"livelink","message":"sync",...}` | A modelling application's scene, pushed as it changes: `hello`, `sync` (upserts and deletes as one undo step, matched objects keeping their Tequila materials) and `camera`. See `docs/livelink-protocol.md`. |
| `export web path=site` | `{"op":"export","format":"web","path":"site","live":true,"photos":false}` | Writes the configurator as a static web folder (live 3D, and with `photos` a queued render of every combination into it); `format` `glb` writes what is shown as a glTF binary instead, `usd` as a USD stage (`.usda`), `usdz` as a USDZ package for AR on an iPhone, iPad or Vision Pro (real size, standing on the ground, materials baked to preview surfaces); `spin` renders a spin viewer into a new `spin` folder under `path` (`columns` 36, `rows` 1, `tilt` 15 degrees, `transparent`). Returns the size, any warnings, and the photo jobs. |
| `queue 1920x1080 256 name=Hero` | `{"op":"queue",...}` | Starts a background render with the document's render settings; it does not block the app. `engine=cycles-gpu` (or `cycles-cpu`, `tequila`) picks the engine for this job. `configurations=true` queues one job per valid combination of the configurator instead, each captured as that combination, named by `{configuration}` in the filename. `studio=Right` (a name or an index) renders one Studio - one view - as it says, and leaves the scene as it was; `studios=true` queues every Studio, one job each, named by `{view}` (the Studio's file-name suffix or name; `{studio}` is the same, and `{product}` is the product's top group's name). `network=true` sends the render (or with `configurations`, every combination) to the render nodes set up in the Render window's Network rendering section instead; the files land in the render folder as they finish, and appear in the Render window. |
| `jobs` / `stop` | `{"op":"query","what":"jobs"}` / `{"op":"stop"}` | Shows queue progress and output files, or stops the current job. A stopped job still saves its image. |
| `changes` | `{"op":"query","what":"history"}` | Shows what undo and redo would do, and whether there is unsaved work. |
| | `{"op":"material","action":"guide"}` | Materials of your own from a short JSON recipe: `guide` (the format, every pattern, tested examples), `preview` (a picture of a recipe on a ball, the scene untouched), `create`, `update`, `recipe` (any material back as one) and `save`. Python: `fs.recipes`. See `docs/materials-authoring.md`. |
| | `{"op":"plugin","action":"write","plugin":"my.x","files":{...}}` | Plugins made and driven by a script or an AI app (`plugin` names it): `guide`, `list`, `write` (files into the person's plugins folder; a running one restarts), `enable` (asks the person, who confirms in the Plugins window), `run` (a command, with panel `values`), `log` (`lines`, and `total` for how many it has logged) and `restart`. Window only. See `docs/plugins.md`. |
| | `{"op":"open","path":"C:/work/kettle.tequila","discard":true}` / `{"op":"new"}` | Opens a project in place of this one (and lists it under File > Open recent), or starts a new one - the starter scene in the window, an empty one headless. Like `quit`, refused over unsaved work unless `save` or `discard` says what to do with it. Python: `fs.open_project(path)`, `fs.new_project()`. |
| `quit save` / `quit discard` | `{"op":"quit","save":true}` | Closes Tequila cleanly. It refuses while there is unsaved work unless you say which option to take. |

Undo, redo and quit cannot be part of a batch. A scripted camera move is its
own undo step. It never merges into an orbit you just made with the mouse.

In Python:

```python
with fs.batch() as b:                      # one undo step; nothing stays if one fails
    b.apply('material.set_type', material=2, type='velvet')
    b.look(view='front')
job = fs.queue(name='Hero', width=1920, height=1080, samples=256)['job']
print(fs.wait_for_job(job)['output'])
fs.quit(save=True)
```

Queue entries capture their scene, camera, materials and destination when
submitted. Later edits or switching projects do not change those renders.
Snapshots are currently held in memory: after restarting, unfinished entries
without a snapshot report a failure and must be queued again from their source
project. Tequila will not silently render the newly opened scene for an old job.

Restricted MCP access and the built-in Assistant cannot apply
`render.set_settings`, including inside a batch, because it can redirect output
files. Use `queue` to adjust resolution, samples and denoising within the locally
configured output location.

## The built-in Assistant

To ask for changes in plain language inside Tequila, using OpenAI, Claude,
Gemini, OpenRouter, Mistral, DeepSeek, xAI, Groq, a model on this computer
or your company's AI, see `docs/ASSISTANT.md`. It uses the same
scripting requests described here.

## Plugins

A script that should live in Tequila's own menus - with a panel of fields,
told when the selection changes - is a plugin: a folder with a
`plugin.json` and a Python file, run by Tequila against this same API. See
`docs/plugins.md` (`PLUGINS.md` beside a packaged Tequila). The material
library is reachable from scripts too: `docs/materials-authoring.md`.

## Connecting an AI app (MCP)

Tequila's command-line program is a Model Context Protocol server:
`tequila-cli --mcp`. It lets an AI app use Tequila directly, and needs
nothing else installed - no Python. MCP apps that launch local servers can
start it: the Claude app, Claude Code, Cursor, VS Code with GitHub Copilot,
OpenAI's Codex, Gemini CLI, Windsurf, LM Studio and others. Its tools
cover reading the scene, applying commands and batches, undo, camera, panels,
background renders, screenshots, procedural materials from recipes
(`tequila_material_*`) and writing plugins (`tequila_plugin_*`).
**Screenshots, renders and material previews come back as images**, so the
assistant sees the result of what it did. Everything it
sends also appears in Tequila's console as it happens.

**Connecting an app:** in Tequila, open the Assistant, click **Connect an
AI…**. Under *AI apps on this computer*, each app Tequila finds has a
**Connect** button that adds Tequila to that app's settings, keeping
everything else there. The original is kept beside it as
`<file>.before-tequila`. A settings file that is not plain JSON, one with
comments say, is left alone and reported, for Tequila to be added by hand.

| App | Settings Tequila is added to | Then |
|---|---|---|
| Claude (desktop) | `claude_desktop_config.json`, every copy the app keeps | Quit it fully and open it again |
| Cursor | `~/.cursor/mcp.json` | Settings → MCP: turn Tequila on if asked |
| VS Code (GitHub Copilot) | `mcp.json` in VS Code's user folder, under `servers` | Copilot Chat, agent mode: Tequila's tools are in the tools list |
| Codex (OpenAI) | `~/.codex/config.toml`, a `[mcp_servers.tequila]` table | New sessions have the tools; `/mcp` lists them |
| Gemini CLI | `~/.gemini/settings.json` | New sessions have the tools; `/mcp` lists them |
| Windsurf | `~/.codeium/windsurf/mcp_config.json` | Refresh its MCP servers, or restart it |
| LM Studio | `~/.lmstudio/mcp.json` | Turn the tequila tools on in a chat, with a model that calls tools |
| Claude Code | none: **Copy the command** | Paste it into a terminal |

Claude Code's command is:

```powershell
claude mcp add tequila -- "C:/Users/you/AppData/Local/Programs/Tequila/tequila-cli.exe" --mcp
```

**Any other MCP app** (Zed, Cline, Continue, Jan, AnythingLLM and others):
**Copy the settings** gives the entry most of them read:

```json
{
  "mcpServers": {
    "tequila": {
      "command": "C:/Users/you/AppData/Local/Programs/Tequila/tequila-cli.exe",
      "args": ["--mcp"]
    }
  }
}
```

**ChatGPT** reaches MCP servers only over the internet. OpenAI's Secure MCP
Tunnel connects it to Tequila without opening anything on this computer to
the internet: see `docs/ASSISTANT.md`. Or connect an OpenAI key to Tequila's
own Assistant.

`python/tequila_mcp.py` is the same bridge in Python, standard library only,
for a checkout without a built `tequila-cli`.

**The Tequila skill** teaches the AI how to work in Tequila - the shots
people ask for, materials, recipes, scene work, configurators, plugins and
scripts. It is an Agent Skill (a folder with a `SKILL.md`), which the Claude
app, Claude Code, Codex and other apps read, and it ships as
`Tequila-Skill.zip` beside Tequila. In the Claude app: **Settings →
Capabilities → Skills → Upload skill** (the **Connect an AI…** window has
**Show the skill file**). For Claude Code and Codex, the window's **Add the
skill** unpacks it into `~/.claude/skills/tequila` or
`~/.codex/skills/tequila`.

`--restricted` (for anything reached from outside this computer) leaves out
the tools that reach beyond the open scene - raw requests, imports, saving
to the library, and every plugin tool - and refuses them if called by name.

Tequila has to be open, with its scripting channel on. The bridge finds it
through `~/.tequila/channel.json` and reconnects if Tequila is restarted. A
portable Tequila (a `portable.txt` beside the program) writes the file to
`data/channel.json` beside it instead, where its own `tequila-cli --mcp`
and the `python` folder inside it look.
The ChatGPT desktop app connects only to *remote* MCP servers over HTTP, which
this local bridge is not. For ChatGPT, use an agent that can run commands on
this computer, such as Codex, together with the Python client.

## Sending a model from another application

`Tequila --send <model file>` is what a modelling application's "Send to
Tequila" button runs (`integrations/` has ones for SolidWorks and ZBrush).
If Tequila is open, the model sent from that file before is updated from it,
keeping its materials, labels and names; a file not sent before is imported
and framed. If Tequila is not open, it starts one with the file, on its own,
and returns. It prints one line (`updated`, `imported` or `started Tequila
with`, and the file) and exits 0, or 1 with the reason. The
model is found again by the file it records, so a bridge writes each
document to the same path every time. On a Mac the program is
`Tequila.app/Contents/MacOS/Tequila`.

## Wire protocol

Read `~/.tequila/channel.json` locally for `port`, `token`, and `pid`
(`data/channel.json` beside a portable Tequila's program; `tequila.discovery_file()`
says which). Connect to `127.0.0.1:port`, sending one UTF-8 JSON object per newline:

```json
{"id":1,"token":"<read locally>","op":"query","what":"context"}
```

Replies contain the same `id`, `ok`, and either `result` or `error`.
The Python client checks protocol/PID and matches replies. Pass `expected_pid`
when multiple app instances may be open; discovery points to the last instance
started. Keep the token local. This is a local control API, not a remote service.

For separate instances, launch the app with `TEQUILA_CHANNEL_FILE` set to a
distinct absolute discovery-file path, then use `Client(discovery=that_path)`.
This avoids replacing another session's default discovery file.

Operations:

| Operation | Fields | Result/behavior |
|---|---|---|
| `ping` | none | App, protocol, PID |
| `query` | `what` | api, context, scene, nodes, materials, selection, camera, commands |
| `apply` | `command` object | Journal command, one separate undo step |
| `select` | `nodes`: integer IDs | Replace selection; rejects invalid IDs |
| `frame` | `all`: boolean, default true | Frame all or selection |
| `import` | `path`, optional `parent` | Import a model |
| `reimport` | optional `node` (a node of the model; the selection otherwise), optional `path` (the file it records otherwise) | Update an imported model from a newer version of its file, keeping materials, labels and names; returns `matched`, `added` and `removed` (hidden) |
| `texture_set` | `path` (a folder, a ZIP, or one of its maps), optional `nodes` | Make materials from a PBR texture pack and put the first on `nodes`, or on the selection |
| `substance` | `action`: `info` (`path`), `open` (`path`, optional `nodes`, `graph`), `set` (optional `material`, `values` {identifier: number, list or string}, `output_size` in pixels, `seed`, `preset`, `images` {identifier: path or null}, `reset`), `status` (optional `material`), `wait` (optional `timeout`, seconds), `tools` (optional `path`: the folder of Adobe's tools, or null to forget it) | Adobe Substance materials (`docs/02-MATERIAL-SYSTEM.md` §5.6). `info` describes a file's graphs, inputs (type, widget, default, range, choices), outputs (the map each becomes, or why it is unused) and presets, and whether Adobe's `sbsrender` was found; `open` makes it a material on the parts, one undo step; `set` is one `material.set_substance` step; `status` gives each Substance material's state (`rendered`, `waiting`, `rendering`, `failed`, `no_tools` with what to install, `missing`, `unreadable`) and its maps; `wait` blocks the window until renders finish. Headless (`tequila --serve`) answers `info` and `tools`; `tequila --substance-info FILE` prints `info` on the command line. Python: `fs.substance('open', path=..., nodes=[...])`. |
| `save` | optional `path` | Save the project |
| `screenshot` | `path`, optional `viewport` or `window` | Deferred window PNG; one pending capture at a time |
| `render` | `path`, `width`, `height`, `samples`, `wireframe` | Synchronous PNG render. `"wireframe": true` draws the model's edges instead, with the project's Wireframe settings (`render.set_settings` → `wireframe: {edges: mesh\|model, hidden: faint\|hidden\|shown, line_width, line_colour, background_top, background_bottom}`): no ray tracing, a second or two at any size. The console's `render out.png 1920x1080 wireframe` and Python's `render(path, wireframe=True)` say the same. |
| `undo`, `redo` | none | Existing document history |

Paths refer to the app's computer. Python helpers resolve them absolutely.
Scripted PNG exports honor the document's denoising, guide prefilter, denoise
strength, firefly rejection and sample clamp settings. The reply includes
`denoised` so clients can verify filtering actually ran. A requested denoiser
failure is reported as an error rather than silently exporting a noisy image.
Camera commands, undo and redo update the viewport and export camera together;
adopting a saved camera preserves its exact orientation, including roll.
Textures remain external assets. Imported and shared materials should be
inspected before changing them. After creation/import/unlink, query fresh IDs.
Failed commands are logged in the workspace. A script is not atomic: if its
fifth operation fails, its first four successful edits remain. Do not blindly
undo several steps if a person or another client edited in between.

## Current limits and next steps

- Python runs externally. There is no embedded editor or runtime, and no
  debugging or autocomplete for Python. The console has Tab completion for its
  own commands.
- `render` blocks the UI until it finishes. Use `queue` for anything larger than
  a preview: it renders in the background and reports progress through `jobs`.
- Screenshot replies only confirm that the capture is scheduled. The Python
  helper waits for a complete new PNG and refuses an existing output path, so it
  never returns a stale image.
- `render()` waits for the file with its own timeout (an hour by default,
  `timeout=` to change it), not the connection's, which is sized for edits.
- Requests are not retried automatically. A timeout can mean an edit completed
  but its reply never arrived. Reconnect and check the scene before retrying.
- There is no lock between clients and no revision-conditional write. Two
  scripts editing at once are applied in the order they arrive.
- A ChatGPT cloud conversation cannot reach this local API. Local MCP apps
  can reach it through `tequila_mcp.py`, and command-line agents through the
  Python client. Neither handles cloud credentials or messages.
