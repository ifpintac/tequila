# Writing Tequila plugins

A plugin adds to Tequila itself: menu commands, small panels of fields that Tequila draws, and code that runs when something happens (an import, a save, a render). It is a folder with a `plugin.json` and, for code, a Python 3 file. Its code works Tequila through the same scripting API you use - undoable, shown in the console.

**Write a plugin when** the user wants something to stay: a button they will press again, an automation on every import, a batch job with settings. For a one-off change, just do it with the tools.

`tequila_plugin_guide` returns the full reference for this build, its field types and events, and a template. Read it once before writing one.

## The loop

1. `tequila_plugin_write {"id": "my.batch-renamer", "files": {"plugin.json": "...", "main.py": "..."}}` - the files' full text. Later writes may send only the files that changed. A plugin with that id that is running restarts with the new code.
2. `tequila_plugin_enable {"id": "my.batch-renamer"}` - a plugin with code runs Python as the user, so Tequila asks them; only their click turns it on. Tell them: *"Tequila is asking to turn on Batch Renamer - click **Turn it on**."* Then `tequila_plugin_list` until `enabled` and `running` are true. A material pack (no code) is turned on at once.
3. `tequila_plugin_run {"id": "my.batch-renamer", "command": "rename", "values": {"prefix": "KT-"}}` - runs a command with the values a panel would send; you get what it logged: prints, status messages, `finished \`rename\``, or the traceback. A command with a `panel` opens the panel for the user instead.
4. Fix and write again; run again. `tequila_plugin_log` shows the log any time; `tequila_plugin_list` shows `failure` when it could not start (no Python, a syntax error).
5. Tell the user where it lives: **Plugins** menu → its commands; **Plugins → Manage plugins…** to turn it off, restart it, or open its folder. To share it: zip the folder; others use **Install from a ZIP…**.

Needs Python 3.9+ on the user's computer (`tequila_plugin_list` → `python`). Use only the standard library unless the user says they have a package installed.

## plugin.json

```json
{
  "id": "my.batch-renamer",
  "name": "Batch Renamer",
  "version": "1.0.0",
  "author": "The user's name",
  "description": "Renames the selected parts with a prefix and a number.",
  "tequila": 1,
  "main": "main.py",
  "commands": [
    {"id": "open", "title": "Batch rename…", "panel": "main", "description": "Tooltip in the menu"}
  ],
  "panels": [
    {"id": "main", "title": "Batch Renamer", "fields": [
      {"id": "prefix", "type": "text", "label": "Prefix", "default": "Part-"},
      {"id": "start", "type": "number", "label": "Start at", "min": 0, "max": 9999, "step": 1, "default": 1},
      {"id": "go", "type": "button", "label": "Rename the selection", "command": "rename"},
      {"id": "status", "type": "label", "text": "Select parts."}
    ]}
  ],
  "events": ["selection"]
}
```

- `id`: letters, digits, `.`, `-`, `_` - `my.something` for the user's own. It is also the folder's name.
- `commands`: menu entries. With `panel` the entry opens that panel; without, it runs the command in the code (values `{}`).
- `main` is required when there are code commands, buttons or events. `materials` (a folder of library presets) makes it a material pack too.

### Panel fields

| `type` | Keys | The command gets |
|---|---|---|
| `number` | `min`, `max`, `step`, `default` | a number |
| `text` | `default`, `lines` (over 1: a box that tall) | a string |
| `toggle` | `default` | true / false |
| `choice` | `options` (list), `default` | the chosen string |
| `colour` | `default` `[r, g, b]` sRGB 0-1 | `[r, g, b]` sRGB 0-1 (use `tequila.srgb()` before setting a material) |
| `file` | `extensions` (`["csv"]`), `save` (true: a file to write), `default` | the path |
| `folder` | `default` | the path |
| `label` | `text` | - (the code changes it with `set_panel`) |
| `button` | `command` | runs that command with **every field's value** |

### Events

| Event | `data` |
|---|---|
| `selection` | `{"nodes": [ids]}` |
| `saved` | `{"path": ...}` |
| `render_finished` | `{"path": ...}` - the image written |
| `imported` | `{"path": ..., "nodes": [new nodes], "parts": [new parts]}` |
| `opened` | `{"path": ...}` - a project opened |
| `panel_opened` | `{"panel": id}` - fill the panel in with `set_panel` |

## main.py

```python
import tequila                     # the client: tequila.srgb(), and fs below is a tequila.Client
import tequila_plugin as plugin


@plugin.command('rename')
def rename(fs, values):
    selected = set(fs.query('selection')['nodes'])
    parts = [p for p in fs.parts() if p.id in selected]
    if not parts:
        plugin.status('Select some parts first')
        return
    number = int(values.get('start', 1))
    with fs.batch() as batch:        # one undo step - for what is sent through `batch`
        for part in parts:
            batch.commands.scene.rename(node=part.id, name=f"{values.get('prefix', '')}{number:03}")
            number += 1
    plugin.set_panel('main', status=f'Renamed {len(parts)} part(s)')
    plugin.status(f'Renamed {len(parts)} part(s)')


@plugin.on('selection')
def selection(fs, data):
    plugin.set_panel('main', status=f"{len(data['nodes'])} selected")
```

| `tequila_plugin` | |
|---|---|
| `@command(id)` | Runs on command `id` with `(fs, values)`. |
| `@on(event)` | Runs on an event the manifest lists, with `(fs, data)`. |
| `status(text)` | A notice in Tequila's window. |
| `set_panel(panel, **values)` | Changes fields of an open panel. |
| `log(text)`, `print()` | Lines in the plugin's log (what `tequila_plugin_run` returns). |
| `load_settings(defaults)`, `save_settings(dict)` | Settings kept between runs, in its own data folder. |
| `data_folder()` | That folder (a `Path`), for anything else it keeps. |

`fs` (a `tequila.Client` on the window) - the useful part:

| | |
|---|---|
| `fs.parts()`, `fs.nodes()`, `fs.find(name='screw', metadata={...}, kind='part')` | `Part`s: `id`, `name`, `kind`, `parent`, `material`, `metadata`, `bounds`, `visible`; `.rename()`, `.assign(material)`, `.show()`, `.hide()`, `.set_metadata()` |
| `fs.materials['Paint']` / `fs.materials[3]` | a `Material`: read parameters as attributes; `m.base_color = '#c0392b'` sets one; `.set_type()`, `.rename()`, `.assign(*parts)` |
| `fs.query(what)`, `fs.context()` | any query; the whole project (`fs.context()['document']`) |
| `fs.apply('scene.rename', node=5, name='Lid')`, `fs.commands.<group>.<verb>(...)` | any command |
| `with fs.batch() as b:` | what is sent **through `b`** - `b.apply(...)`, `b.commands.<group>.<verb>(...)`, `b.add('library', action='apply', ...)`, `b.select()`, `b.look()` - is one undo step, all or nothing, sent when the block ends. Calls on `fs` inside the block still go one by one. |
| `fs.library.apply('Chrome', *parts)`, `fs.library.presets()`, `fs.library.save(material, name)` | the material library |
| `fs.recipes.create(recipe, *parts)`, `.preview(recipe)`, `.update(id, recipe)` | procedural materials from recipes (`recipes.md`) |
| `fs.select(ids)`, `fs.frame()`, `fs.look(view='front')`, `fs.ui(viewport='rendered')` | the view |
| `fs.queue(name=..., camera=i, samples=..., folder=...)`, `fs.jobs()`, `fs.render(path, ...)`, `fs.screenshot(path, viewport=True)` | rendering |
| `fs.import_model(path)`, `fs.export('glb', path)`, `fs.save()` | files |
| `fs.request(op, **fields)` | any raw request, e.g. `fs.request('plugin', action='list')` |

## Patterns

- **Do something on every import:** `"events": ["imported"]`; use `data['parts']`; keep the user's choices with `save_settings` (a "Keep these settings" button), since a panel's values arrive only with its buttons. The bundled **Auto Materials** example does exactly this.
- **Fill a panel when it opens:** `"panel_opened"`; `plugin.set_panel(...)` with saved settings or what the scene holds.
- **A batch of renders:** `fs.queue(...)` returns at once, so queue them all in one command; `render_finished` tells you as each lands. The bundled **Render Every Camera** example.
- **Colourways / variants:** make materials with `fs.library.apply(..., unlink=True)` or recipes, then `fs.apply('material.set_multi_materials', ...)` (`scene.md`).
- **Read a spreadsheet:** a `file` field with `"extensions": ["csv"]`, then `csv.reader`.
- **Long work:** commands run one at a time; a slow one holds the next. Keep each command quick; report progress with `set_panel` as it goes.

## Pitfalls

- **Never read `sys.stdin` or write to `sys.__stdout__`**: they are how Tequila talks to the plugin. `print()` is safe (it goes to the log).
- An exception is caught: its traceback goes to the log, the user sees a notice, the plugin keeps running.
- Colours set on materials: hex strings (`'#2D5DA8'`) or `tequila.srgb('#2D5DA8')`; a `colour` field's value is sRGB 0-1 - pass it through `tequila.srgb()`.
- Lengths are metres; +Y is up.
- Files a plugin ships with: `Path(__file__).parent`. State it keeps: `plugin.data_folder()` - never the plugin's own folder (the bundled examples' is read-only).
- Changing `plugin.json`'s commands or panels needs the plugin reloaded: a write does that.
- Test the logic you can without Tequila (pure functions at the top of `main.py`), then run it.
