# Scripts and raw requests

Everything the tools do is a request on Tequila's scripting channel. For what no tool covers, send the request yourself with `tequila_request {"request": {"op": ..., ...}}` (full connection only). For work the user wants to run again and again outside Tequila's menus - or with no window at all - write them a Python script with Tequila's client. (Inside Tequila's menus, write a plugin: `plugins.md`.)

## Operations

| Request | Does |
|---|---|
| `{"op": "query", "what": ...}` | `api` (material types, parameters, procedurals, examples), `context` (everything), `scene`, `nodes`, `materials`, `selection`, `camera`, `commands`, `ui`, `jobs`, `history`, `library` |
| `{"op": "apply", "command": {...}}` | one command (`commands.md`) |
| `{"op": "batch", "ops": [...], "atomic": true}` | several requests as one undo step |
| `{"op": "undo"}` / `{"op": "redo"}` | |
| `{"op": "select", "nodes": [...]}` | |
| `{"op": "frame", "all": true}`, `{"op": "look", ...}` | the view |
| `{"op": "import", "path": ...}`, `{"op": "reimport", "node": 3, "tessellation": "high"}` | models |
| `{"op": "texture_set", "path": ..., "nodes": [...]}` | a PBR texture pack onto parts |
| `{"op": "library", "action": "apply" / "save" / "import" / "export" / "delete", ...}` | the material library (`materials.md`) |
| `{"op": "material", "action": "guide" / "preview" / "create" / "update" / "recipe" / "save", ...}` | materials from recipes (`recipes.md`) |
| `{"op": "plugin", "action": "guide" / "list" / "write" / "enable" / "run" / "log" / "restart", "plugin": "my.x", ...}` | plugins (`plugins.md`); `plugin` names it, as `id` is every request's own number |
| `{"op": "render", "path": ..., "width": ..., "height": ..., "samples": ...}` | a render now, blocking |
| `{"op": "queue", ...}` | a background render: `name`, `width`, `height`, `samples`, `folder`, `filename`, `denoise`, `camera`, `engine` (`tequila`, `cycles-cpu`, `cycles-gpu`), `configurations: true` (every configurator combination), `network: true` (the render nodes) |
| `{"op": "stop"}` | stop the running background render |
| `{"op": "screenshot", "path": ..., "viewport": true}` | |
| `{"op": "export", "format": "glb" / "usd" / "web" / "spin", "path": ...}` | `scene.md` |
| `{"op": "ui", ...}` | panels, viewport shading, Inspector tab, `graph` (open the Material Graph on a material), `mode` / `theme` |
| `{"op": "save", "path": ...}` | the project |
| `{"op": "open", "path": ..., "discard": true}`, `{"op": "new"}` | open another project, or start a new one - refused over unsaved work unless `save` or `discard` says what to do with it; ask the person first |
| `{"op": "quit", "save": true}` | close Tequila - only when asked |

## The Python client

`tequila.py` ships in the `python` folder beside Tequila, standard library only.

```python
import sys
sys.path.insert(0, r'C:\Users\<you>\AppData\Local\Programs\Tequila\python')   # where Tequila is installed
import tequila

# The window the user has open:
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

- `fs.parts()`, `fs.find(name=..., metadata=...)`, `fs.materials[...]`, `fs.commands.<group>.<verb>(...)`, `fs.apply(name, **fields)`, `fs.query(...)`, `fs.context()`, `fs.request(op, **fields)`.
- `with fs.batch() as b:` - what is sent through `b` is one undo step.
- `fs.library`, `fs.recipes`, `fs.render`, `fs.queue` / `fs.wait_for_job`, `fs.screenshot`, `fs.export`, `fs.save`, `fs.undo`.
- A headless session refuses what needs a window (screenshots, the queue, panels, plugins) by name, and ends unsaved: `fs.save(path)` first.
- `tequila.srgb('#2D5DA8')` converts a swatch colour into the rendering space, though hex strings are accepted directly.

Give the user the script as a file and tell them how to run it (`python my_script.py`), or run it yourself if you have a shell on their computer and they asked you to.
