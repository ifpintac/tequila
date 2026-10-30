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
| `show material` / `hide console` | `{"op":"ui","tab":"material","console":false}` | Opens and closes panels: `render_window`, `console`, `inspector`, `library`, `full_viewport`, `help`, `frame` (the viewport at the render's shape, on by default), and Inspector tabs (`configurator` opens the Studios tab at the configurator); `present` enters or leaves Presentation mode; `about` opens Help > About; `start_screen` opens or closes the start screen; `updates` opens Help > Check for updates, which asks GitHub for the latest release; `connect` opens the Assistant's Connect an AI window; `viewport` picks `wireframe`, `solid`, `material` or `rendered`; `mode` (`light`, `dark`, `auto`) and `theme` (`graphite`, `ocean`, `orchid`, `ember`, `meadow`, `rose`) change the look for the session without saving it; `appearance` opens the picker. |
| `viewport rendered` | `{"op":"ui","viewport":"rendered"}` | Switches the viewport between rendered and solid. |
| | `{"op":"query","what":"api"}` → `procedurals` | Every pattern at its defaults, in the exact shape `material.set_procedural` takes as `procedural`. An unknown `kind` is refused rather than read as "detach". |
| | `{"op":"ui","graph":3}` | Opens the Material Graph editor on material 3; `"graph":false` closes it. |
| `ui` | `{"op":"query","what":"ui"}` | Shows what is open, and how far the viewport's picture has got: `viewport_samples`, and `viewport_preparing` while Cycles prepares the graphics card and has shown nothing. |
| `screenshot a.png viewport` | `{"op":"screenshot","path":...,"viewport":true}` | Captures just the viewport rather than the whole window. |
| `screenshot a.png graph` | `{"op":"screenshot","path":...,"window":"graph"}` | Captures one of the other windows: `render`, `graph`, `shortcuts` or `connect`. They are windows of their own, so the main window's screenshot no longer shows them. Refused if that window is not open. |
| `livelink` | `{"op":"livelink","message":"sync",...}` | A modelling application's scene, pushed as it changes: `hello`, `sync` (upserts and deletes as one undo step, matched objects keeping their Tequila materials) and `camera`. See `docs/livelink-protocol.md`. |
| `export web path=site` | `{"op":"export","format":"web","path":"site","live":true,"photos":false}` | Writes the configurator as a static web folder (live 3D, and with `photos` a queued render of every combination into it); `format` `glb` writes what is shown as a glTF binary instead, `usd` as a USD stage (`.usda`); `spin` renders a spin viewer into a new `spin` folder under `path` (`columns` 36, `rows` 1, `tilt` 15 degrees, `transparent`). Returns the size, any warnings, and the photo jobs. |
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
through `~/.tequila/channel.json` and reconnects if Tequila is restarted.
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

Read `~/.tequila/channel.json` locally for `port`, `token`, and `pid`.
Connect to `127.0.0.1:port`, sending one UTF-8 JSON object per newline:

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
