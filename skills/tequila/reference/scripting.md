# Raw requests and the Python client

Everything the tools do is a request on Tequila's scripting channel. Two reasons to go below the tools: a job **no tool covers** (send the request yourself), and work the person wants to **run again, or with no window** (write them a Python script). Inside Tequila's menus, a plugin is the better home: page `plugins`.

## 1. A raw request

`tequila_request {"request": {"op": "<name>", ...}}` sends any request on a full local connection. An assistant running on an API key or a local model has it for the requests that have no tool of their own - `export`, `reimport`, `texture_set`, `substance`, `uv_layout`, `studio_kit`, `library`, `studio_preset`, `queue`, `plugin`, `screenshot`, `render` and `ui` - and is refused `new`, `open` and `quit`, which are the person's to do there. A restricted (remote) connection does not have it, and the jobs below are then the person's to do in the window.

```json
tequila_request {"request": {"op": "export", "format": "glb", "path": "C:/out/kettle.glb"}}
tequila_request {"request": {"op": "batch", "atomic": true, "ops": [
  {"op": "apply", "command": {"command": "scene.rename", "node": 5, "name": "Lid"}},
  {"op": "select", "nodes": [5]},
  {"op": "frame", "all": false}]}}
```

| Request | Does | Page |
|---|---|---|
| `query` with `what` | `scene`, `nodes`, `materials`, `selection`, `camera`, `commands`, `ui`, `jobs`, `history` (short); `context`, `api`, `library` (very long: use `read`, `help` and the library's `search`) | |
| `read` with `what`, `node` or `index` | One part of the project, or one node | |
| `help` with `command` | One command's fields and an example | |
| `guide` with `topic` | One page of this guide | |
| `apply` with `command` | One command | |
| `batch` with `ops`, `atomic` | Several requests as one undo step | |
| `undo`, `redo`, `select`, `frame`, `look` | | `camera` |
| `add_shape` | A simple shape as a new part | `scene` |
| `import`, `reimport`, `save`, `export`, `open`, `new` | Files | `files` |
| `texture_set`, `substance` | PBR packs, Substance materials | `textures` |
| `library` with `action`: `search`, `apply`, `save`, `import`, `export`, `delete` | The material library | `materials` |
| `material` with `action`: `guide`, `preview`, `create`, `update`, `recipe`, `save` | Materials from recipes | `recipes` |
| `uv_layout` | A part's UV layout as a picture | `uv` |
| `studio_preset` with `action`: `list`, `apply`, `save`, `install`, `export`, `delete`, `render_all`; `studio_kit` | Studios | `variants`, `lighting` |
| `render`, `queue`, `stop`, `screenshot` | Pictures | `rendering` |
| `ui` | Panels, windows, the UV and graph editors | `app` |
| `preferences` | Tequila's own settings | `app` |
| `plugin` with `action`: `guide`, `list`, `write`, `enable`, `run`, `log`, `restart` | Plugins; `plugin` names it, since `id` is every request's own number | `plugins` |
| `quit` with `save` or `discard` | Closes Tequila. Only when the person asks | |

A request the application does not understand is refused with what it expected. A batch cannot hold `undo`, `quit` or `uv_layout`.

In a skill folder, `reference/commands.md` lists every command with an example; it is long, so search it for the command's name, or ask `tequila_command`.

## 2. The Python client

`tequila.py` ships in the `python` folder beside Tequila, standard library only. Use it for a script the person runs themselves.

```python
import sys
sys.path.insert(0, r'C:\Users\<you>\AppData\Local\Programs\Tequila\python')   # where Tequila is installed
import tequila

# The window the person has open:
with tequila.connect() as fs:
    for part in fs.find(name='screw'):
        fs.library.apply('Stainless Brushed', part)

# No window - a headless Tequila of its own, for batch jobs:
with tequila.open('kettle.tequila') as fs:
    body = fs.materials['Body']
    for name, colour in [('red', '#C23B32'), ('blue', '#2D5DA8')]:
        body.base_color = colour
        fs.render(f'kettle-{name}.png', width=1600, height=1200, samples=256)
```

- **Finding things:** `fs.parts()`, `fs.find(name=..., metadata=...)`, `fs.materials['Name']` or `fs.materials[3]`.
- **Reading:** `fs.query(what)`, `fs.read('render')['value']` (one part of the project), `fs.read(node=12)` (one node with its transform), `fs.help('scene.set_transform')`, `fs.guide('lighting')`.
- **Changing:** `fs.apply('scene.rename', node=5, name='Lid')` or `fs.commands.scene.rename(node=5, name='Lid')`; `with fs.batch() as b:` makes what is sent through `b` one undo step.
- **Materials:** `fs.library.apply(name, *parts)`, `fs.library.presets()`, `fs.recipes.create(recipe, *parts)`, `.preview(recipe)`, `.update(id, recipe)`.
- **Pictures and files:** `fs.render(path, width=..., height=..., samples=...)`, `fs.queue(...)` with `fs.wait_for_job(id)`, `fs.screenshot(path)`, `fs.import_model(path)`, `fs.export('glb', path)`, `fs.save(path)`, `fs.undo()`.
- `fs.request(op, **fields)` sends anything in the table above.
- `tequila.srgb('#2D5DA8')` converts a swatch colour into the rendering space, though hex strings are accepted wherever a colour goes.

**With no window** (`tequila.open`) a session does what needs only the project: it refuses screenshots, the background queue, panels, plugins and texture packs by name, `render` blocks until the picture is written, and unsaved changes end with it: `fs.save(path)` first.

Give the person the script as a file and say how to run it (`python my_script.py`); run it yourself only if you have a shell on their computer and they asked.

## 3. Which to write

| The person wants | Write |
|---|---|
| Something done now, once | Nothing: do it with the tools |
| The same job on many files overnight, no window | A Python script with `tequila.open` |
| A button or panel inside Tequila, or something on every import | A plugin (page `plugins`) |
| Tequila driven from their own pipeline or another program | The channel itself: one JSON request and reply per line over a local socket; Tequila's `docs/SCRIPTING.md` has the wire protocol |
