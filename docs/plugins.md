# Writing plugins

A Tequila plugin is a folder with a `plugin.json` in it. It can bring
**materials** (a folder of presets — no code at all), **commands** in the
Plugins menu, **panels** of fields that Tequila draws for it, and handlers
for **events**. Code is Python 3, and it works Tequila through the same
[scripting API](scripting-api.md) every script uses: anything a script can
do, a plugin can do, undoably, and everything it does shows in the
Scripting tab's log.

## Quick start

1. **Plugins → Manage plugins…**, type a name, and click **Make it from the
   template**. Tequila writes a working plugin into your plugins folder and
   opens the folder.
2. Tick it on. A plugin with code asks first: it runs Python with your
   permissions.
3. **Plugins → *Your plugin*** now has *Count the selected parts* and a
   panel that renames them. Edit `main.py`, then click **Restart** on the
   plugin to load your change. `print()` goes to the plugin's **Log**.

Python 3.9 or newer must be installed (python.org); Tequila finds `py -3`,
`python3` or `python`, or the program named by `TEQUILA_PYTHON`. Material
packs need no Python.

## Making one with an AI app

Tequila's own Assistant, the Claude app (connected with the Assistant's
**Providers… → Connect the Claude app**), Claude Code, or any MCP client
running `tequila-cli --mcp` can write a plugin for you from a description: *"make me
a plugin with a panel that renames the selected parts with a prefix"*. It
has six tools for it:

| Tool | |
|---|---|
| `tequila_plugin_guide` | This page, the field types and events of this build, and a template. |
| `tequila_plugin_write` | Writes the plugin's files into your plugins folder. A running plugin restarts with them. |
| `tequila_plugin_enable` | Asks **you** to turn it on: Tequila shows its usual question, and only your click starts its code. |
| `tequila_plugin_run` | Runs one of its commands and answers with what it logged: prints, `finished`, or the traceback. |
| `tequila_plugin_log` | Its log, to debug. |
| `tequila_plugin_list` | Every plugin, on or off, running or failed and why. |

So the loop is write → enable (you confirm once) → run → read the log →
write the fix. When the built-in Assistant changes the code of a plugin that
is on, the plugin is turned off and you are asked again. The same is the `plugin` operation of the
[scripting API](scripting-api.md) for scripts. These tools are not offered
to remote (restricted) connections. A plugin the AI writes over one of the
examples becomes your own copy, and is off until you turn it on.

## Where plugins live

| | |
|---|---|
| Yours | `%APPDATA%\Tequila\plugins\<id>\` on Windows, `~/Library/Application Support/Tequila/plugins/<id>/` on a Mac, `~/.config/Tequila/plugins/<id>/` on Linux |
| Examples | the `plugins` folder beside Tequila (`Tequila.app/Contents/Resources/plugins` on a Mac, `plugins/examples` in the source), read-only |
| Which are on | `plugins.json` beside your plugins folder |

**Install from a ZIP…** takes a ZIP with `plugin.json` at its top or one
folder down. To share a plugin, zip its folder.

## plugin.json

```json
{
  "id": "com.example.colourways",
  "name": "Colourways",
  "version": "1.0.0",
  "author": "You",
  "description": "One render per colour.",
  "tequila": 1,
  "main": "main.py",
  "materials": "materials",
  "commands": [
    { "id": "open", "title": "Colourways…", "panel": "colourways" },
    { "id": "count", "title": "Count the selection", "description": "Shown as the menu item's tooltip" }
  ],
  "panels": [
    {
      "id": "colourways",
      "title": "Colourways",
      "fields": [
        { "id": "colours", "type": "text", "label": "Colours", "default": "#C23B32, #2D5DA8" },
        { "id": "render", "type": "button", "label": "Render each", "command": "render" },
        { "id": "status", "type": "label", "text": "Select a part." }
      ]
    }
  ],
  "events": ["selection"]
}
```

| Field | |
|---|---|
| `id` | Unique; letters, digits, `.`, `-`, `_`. Also the folder it installs into. |
| `name`, `version`, `author`, `description` | Shown in the Plugins window. |
| `tequila` | The plugin API it was written for: `1`. A newer one is refused with a reason. |
| `main` | Its Python file. Required if it has commands that run, buttons or events. |
| `materials` | A folder of presets (`.json`) shown in the library while the plugin is on, read-only, findable by the plugin's name. A plugin with only this is a **material pack** and runs no code. |
| `commands` | Menu entries. With `panel`, the entry opens that panel; without, it runs the command. |
| `panels` | Small windows of fields, drawn by Tequila. |
| `events` | Which events to be told about: `selection`, `saved`, `render_finished`, `imported`, `opened`, `panel_opened`. |

Paths in `main` and `materials` must stay inside the plugin's folder.

### Panel fields

| `type` | Extra keys | Value the command gets |
|---|---|---|
| `number` | `min`, `max`, `step`, `default` | a number |
| `text` | `default`, `lines` (over 1: a box that many lines tall) | a string |
| `toggle` | `default` | `true` / `false` |
| `choice` | `options` (list), `default` | the chosen string |
| `colour` | `default` (`[r, g, b]`, sRGB 0–1) | `[r, g, b]`, sRGB 0–1 |
| `file` | `extensions` (`["csv", "png"]`), `save` (`true`: a file to write), `default` | the path, typed or chosen with **Browse…** |
| `folder` | `default` | the folder's path |
| `label` | `text` | — (the plugin can change it) |
| `button` | `command` | runs `command` with every field's value |

## The Python side

```python
import tequila
import tequila_plugin as plugin


@plugin.command('render')
def render(fs, values):
    """fs: a tequila.Client on this window. values: the panel's fields."""
    selected = fs.query('selection')['nodes']
    part = next(p for p in fs.parts() if p.id in selected)
    material = fs.materials[part.material]
    for colour in values['colours'].split(','):
        material.base_color = tequila.srgb(colour)   # swatch colour -> rendering space
        fs.queue(name=f'{material.name} {colour.strip()}')
    plugin.status('Queued the renders')                # a notice for the person


@plugin.on('selection')
def selection(fs, data):                               # data: {'nodes': [...]}
    plugin.set_panel('colourways', status=f"{len(data['nodes'])} selected")
```

| `tequila_plugin` | |
|---|---|
| `@command(id)` | Run on the command `id`: from the menu (values `{}`) or a button (the panel's values). |
| `@on(event)` | Run on an event the manifest lists (below). |
| `status(text)` | A notice in the window. |
| `set_panel(panel, **values)` | Change fields of an open panel: a label's text, a number, a choice, a text box. |
| `log(text)` | A line in the plugin's log only. `print()` goes there too. |
| `load_settings(defaults)` / `save_settings(dict)` | Settings kept between runs, in the plugin's data folder. |
| `data_folder()` | That folder - `plugin-data/<id>` beside your plugins - for anything else it keeps. It is yours: updating or reinstalling the plugin leaves it. |

| Event | `data` |
|---|---|
| `selection` | `{'nodes': [ids]}` — the selection changed |
| `saved` | `{'path': ...}` — the project was saved |
| `render_finished` | `{'path': ...}` — a render was written |
| `imported` | `{'path': ..., 'nodes': [every new node], 'parts': [the new parts]}` — a model came in, from the File menu or a script |
| `opened` | `{'path': ...}` — a project was opened |
| `panel_opened` | `{'panel': id}` — one of its panels opened: fill it in with `set_panel` (saved settings, the scene as it is) |

A panel's field values reach the plugin with a button's command. For an
event, keep what it needs with `save_settings` - a "Keep these settings"
button, say - and read it back with `load_settings`.

`fs` is the whole [scripting API](scripting-api.md) and the
[`tequila` module](../python/tequila.py): `fs.parts()`, `fs.find(...)`,
`fs.materials[...]`, `fs.commands.<group>.<verb>(...)` for every command,
`with fs.batch() as b:` for one undo step (send through `b`: `b.commands...`,
`b.apply(...)`; calls on `fs` inside still go one by one), `fs.library.apply(...)` and the rest of
the [material library](materials-authoring.md#from-a-script), `fs.render(...)`,
`fs.queue(...)`, `fs.screenshot(...)`. Colours set on materials are in the
rendering space; `tequila.srgb('#2D5DA8')` converts a swatch.

## How it runs

Turning a plugin with code on starts `python tequila_plugin.py <folder>` —
a process of its own, with no window. The host loads `main`, connects back to
this window's scripting channel, and waits: Tequila sends commands and
events as JSON lines on its stdin, and the plugin's messages come back on its
stdout. So:

- **A plugin cannot crash Tequila.** If its code fails, the traceback is in
  its log and a notice says so; the plugin keeps running for the next
  command. If its process ends, the Plugins window says so — **Restart**
  starts it again.
- **Work happens while Tequila stays responsive**, and every change it makes
  arrives through the channel, so it is validated, journalled and undoable
  exactly like a click.
- **Commands run one at a time**, in the order they were asked for.
- Turning a plugin off, or closing Tequila, asks it to stop and then ends
  its process.

## Trust

A plugin with code runs as you: it can read and change your files as well as
the scene. Tequila never starts one you have not turned on, and asks before
turning one on. Install plugins only from people you trust, and read
`main.py` — it is a plain Python file. Material packs run nothing.

## Examples

`plugins/examples` (the `plugins` folder beside Tequila). They come with
Tequila and start off: the **Plugins** menu lists them under *Off · click to
turn on*, and one click turns one on. Those with code need Python 3 on the
computer; the menu says so where it is missing.

- **Colourways** — a panel with a list of colours; renders the selected
  part in each, or keeps each as a library material. Uses a panel, two
  buttons, the selection event, `tequila.srgb`, `fs.queue` and
  `fs.library.save`.
- **Anodised Extras** — a material pack: four presets and a manifest, no
  code.
- **Auto Materials** — a library material on every part as a model is
  imported, or on the whole scene at a click, whatever the parts are called.
  In turn:
  1. your rules, typed or from a CSV file;
  2. what it learned from you: the materials you gave parts of the same
     name, in projects you saved or with *Learn from this scene*;
  3. about sixty built-in rules over the part's name (screws → steel,
     lenses → glass, grips → rubber), in English, German, French, Spanish
     or Italian, CAD shorthand (HSG, BRKT, SCR) or a fastener standard
     (ISO 4762, DIN 934, M3x10, 1/4-20, a McMaster number); over the
     material, description and layer the CAD file gave it ("Aluminum
     6061", "ABS"); and over the assemblies it sits in;
  4. a part it is a copy of, or named like (`Screw <3>`, `Bracket_2`);
  5. and for the rest, its shape and colour: see-through parts glass, four or
     more copies of a small shape hardware, a thin rod steel, anything else a
     moulded finish in its own colour.

  Plastics and paints keep the colours the model gave them, and a preview
  says what would change and what decided it. Uses the `imported`, `saved`
  and `panel_opened` events, `load_settings` / `save_settings`, a file
  field, each part's mesh (copies share one) and bounds, and library presets
  applied in one `fs.batch()`.
- **Render Every Camera** — one background render from each saved camera,
  named after it, into a folder of your choice. Uses `fs.context()`, a
  folder field and `fs.queue(camera=...)`.
- **Studio Lighting** — 21 product-photography studios, from e-commerce
  white and soft light tents to glass, jewellery, chrome and low-key sets,
  each set around the camera in one click and then fine-tuned: key, fill and
  rims, softness, warmth, gels, backdrop colour and glow, floor and
  reflections. Three panels with described settings, `load_settings` /
  `save_settings`, and a black environment written once into its data
  folder so the studio's own lights are the only light.
- **Vibes** — one click turns the scene into a mood (Cyberpunk, Golden
  Hour, Keynote, Vaporwave, Noir, Candy, Lava, Deep Space), with Shuffle, a
  Drama slider and Back to my look. Builds lights, backdrops and stars from
  environment pins placed around the camera, so they show in reflections;
  sends everything as one `fs.batch()`, one undo step; and keeps the look
  from before the first vibe, read from `fs.context()`, to put back.

## Testing a plugin without the window

`tequila.open()` starts a headless Tequila, and the host can be pointed at
it — `python/test_plugins.py` does this:

```python
with tequila.open('part.step') as fs:
    env = dict(os.environ, TEQUILA_DISCOVERY=str(Path(fs._folder.name) / 'channel.json'),
               TEQUILA_PID=str(fs.pid), PYTHONPATH='path/to/tequila/python')
    env['TEQUILA_PLUGIN_DATA'] = 'a/temporary/folder'   # not your real settings
    host = subprocess.Popen([sys.executable, '-u', 'tequila_plugin.py', 'my-plugin'],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True)
    host.stdin.write(json.dumps({'type': 'command', 'command': 'rename', 'values': {}}) + '\n')
```

A headless session has no window, so `queue`, `screenshot` and `ui` are
refused there; everything else works.
