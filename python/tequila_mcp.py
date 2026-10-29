"""An MCP server that lets an AI app drive a running Tequila.

Model Context Protocol clients — Claude Desktop, Claude Code, Cursor and
others — start this program and talk to it over stdin/stdout. It turns their
tool calls into requests on Tequila's local scripting channel, and hands
screenshots and renders back as images, so the assistant can see what it did.

Standard library only. Run it with the Python that runs `tequila.py`:

    claude mcp add tequila -- python C:/path/to/tequila/python/tequila_mcp.py

Tequila must be open; the bridge finds it through ~/.tequila/channel.json
and reconnects if the application is restarted. Everything it does appears in
Tequila's console, in amber, as it happens.

`--restricted` is for anything reached from outside this computer — ChatGPT
through OpenAI's Secure MCP Tunnel (scripts/chatgpt-tunnel.ps1). It keeps the
tools to the open scene: no raw requests, no importing files, no saving or
rendering to a path of the caller's choosing.
"""
import base64
import json
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from tequila import Client, TequilaError  # noqa: E402

PROTOCOL_VERSIONS = ('2025-06-18', '2025-03-26', '2024-11-05')
SERVER_INFO = {'name': 'tequila', 'version': '1.0.0'}

INSTRUCTIONS = (
    'Tequila is a product-rendering application open on this computer, and these tools are the way to work in it: '
    'use them, not screenshots of the desktop, mouse clicks or any computer-use control - they are exact, fast and undoable. '
    'Start with tequila_scene, then tequila_nodes to find parts by name, material or size. '
    'Materials may be shared between parts: unlink a part (material.unlink) before changing it alone. '
    'Use tequila_batch for several edits: they become one undo step, and if one fails none of them stay. '
    'Look at your work with tequila_screenshot (viewport=true for just the picture). '
    'For a material of your own, read tequila_material_guide, try recipes with tequila_material_preview, '
    'then tequila_material_create and refine with tequila_material_update. '
    'To extend Tequila itself - a panel, a batch job, an automation - write a plugin: tequila_plugin_guide, then '
    'tequila_plugin_write, tequila_plugin_enable (the person confirms), tequila_plugin_run and tequila_plugin_log. '
    'Every command is undoable, and the person watching sees each request in Tequila\'s console.'
)

VECTOR = {'type': 'array', 'items': {'type': 'number'}, 'minItems': 3, 'maxItems': 3}


def tool(name, description, properties=None, required=()):
    return {
        'name': name,
        'description': description,
        'inputSchema': {
            'type': 'object',
            'properties': properties or {},
            'required': list(required),
            'additionalProperties': False,
        },
    }


TOOLS = [
    tool('tequila_scene', 'Counts, units, overall bounds and the file of the open scene.'),
    tool('tequila_nodes', 'Parts and groups: id, name, material and world size. Filter by a word in the name or material name to keep the answer short.',
         {'filter': {'type': 'string', 'description': 'Only nodes whose name or material name contains this (case-insensitive).'},
          'parts_only': {'type': 'boolean'}}),
    tool('tequila_materials', 'Every material: id, type, how many parts use it, parameters and texture maps.'),
    tool('tequila_query', 'Any read-only query.',
         {'what': {'type': 'string', 'enum': ['api', 'context', 'scene', 'nodes', 'materials', 'selection', 'camera', 'commands', 'ui', 'jobs', 'history', 'library']}},
         ['what']),
    tool('tequila_apply', 'Apply one command, e.g. {"command":"material.set_type","material":2,"type":"velvet"}. '
         'tequila_query what=commands lists them; what=api describes material parameters and types. One undo step.'
         ' Colours may be hex sRGB strings ("value": "#c0392b"), converted for you; arrays are linear ACEScg.',
         {'command': {'type': 'object', 'description': 'A journal command: {"command": "<name>", ...fields}.'}},
         ['command']),
    tool('tequila_material_guide',
         'How to make procedural materials: the recipe format, every pattern kind with what it looks like and its settings and defaults, the material types, rules of thumb, and 16 tested example recipes (terrazzo, marble, brushed aluminium, walnut, carbon fibre, leather, concrete, hammered copper, galaxy resin, worn paint...). Read it before writing a recipe.',
         {},
         []),
    tool('tequila_material_preview',
         'Show a material recipe (or an existing material by id) on a ball, without changing the scene. Returns an image. The fastest way to try an idea: preview, adjust, preview again, then tequila_material_create. Recipe: {"name": "Speckled Stone", "type": "plastic" (or metal, glass, paint, velvet... see the guide), any parameter as a value ("specular_roughness": 0.3, "base_color": "#c0392b" - hex sRGB is fine), "patterns": [{"param": "base_color", "procedural": {"kind": "noise"}, "size": 0.01, "colors": [{"at": 0, "color": "#d8d4cc"}, {"at": 1, "color": "#a9a59c"}]}, {"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.02, "range": [0.2, 0.4]}, {"param": "bump", "procedural": {"kind": "leather"}, "size": 0.002, "depth": 0.0001}]}. `size` is the feature size in metres at real-world scale. A later pattern on the same param is a layer: "blend" mix/multiply/add, "amount" 0-1, "mask" {"threshold": 0.5, "softness": 0.05}. Pattern kinds: noise, noise_texture, ridged, cellular, spots, granite, marble, wood, wood_advanced, brushed, brushed_radial, scratches, leather, weave, camouflage, contour, mesh. Call tequila_material_guide once for what each looks like, their settings, the rules of thumb and 16 tested example recipes to start from.',
         {"recipe": {"type": "object", "description": "The recipe to show."}, "material": {"type": "integer", "description": "Or an existing material's id."}, "radius": {"type": "number", "description": "The ball's radius in metres (default 0.025). Use the real part's size to judge pattern scale."}, "shape": {"type": "string", "enum": ["sphere", "slab"]}},
         []),
    tool('tequila_material_create',
         'Make a new material from a recipe and put it on parts, as one undo step. Returns its id and a preview image. Recipe: {"name": "Speckled Stone", "type": "plastic" (or metal, glass, paint, velvet... see the guide), any parameter as a value ("specular_roughness": 0.3, "base_color": "#c0392b" - hex sRGB is fine), "patterns": [{"param": "base_color", "procedural": {"kind": "noise"}, "size": 0.01, "colors": [{"at": 0, "color": "#d8d4cc"}, {"at": 1, "color": "#a9a59c"}]}, {"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.02, "range": [0.2, 0.4]}, {"param": "bump", "procedural": {"kind": "leather"}, "size": 0.002, "depth": 0.0001}]}. `size` is the feature size in metres at real-world scale. A later pattern on the same param is a layer: "blend" mix/multiply/add, "amount" 0-1, "mask" {"threshold": 0.5, "softness": 0.05}. Pattern kinds: noise, noise_texture, ridged, cellular, spots, granite, marble, wood, wood_advanced, brushed, brushed_radial, scratches, leather, weave, camouflage, contour, mesh. Call tequila_material_guide once for what each looks like, their settings, the rules of thumb and 16 tested example recipes to start from.',
         {"recipe": {"type": "object", "description": "The recipe."}, "nodes": {"type": "array", "items": {"type": "integer"}, "description": "Part ids to put it on; the selection when left out."}, "preview": {"type": "boolean", "description": "Return a preview image too (default True)."}},
         ['recipe']),
    tool('tequila_material_update',
         'Remake an existing material from a recipe, where it is: every part using it changes. One undo step; returns a preview image. Use it to refine a material after looking at it on the product. Recipe: {"name": "Speckled Stone", "type": "plastic" (or metal, glass, paint, velvet... see the guide), any parameter as a value ("specular_roughness": 0.3, "base_color": "#c0392b" - hex sRGB is fine), "patterns": [{"param": "base_color", "procedural": {"kind": "noise"}, "size": 0.01, "colors": [{"at": 0, "color": "#d8d4cc"}, {"at": 1, "color": "#a9a59c"}]}, {"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.02, "range": [0.2, 0.4]}, {"param": "bump", "procedural": {"kind": "leather"}, "size": 0.002, "depth": 0.0001}]}. `size` is the feature size in metres at real-world scale. A later pattern on the same param is a layer: "blend" mix/multiply/add, "amount" 0-1, "mask" {"threshold": 0.5, "softness": 0.05}. Pattern kinds: noise, noise_texture, ridged, cellular, spots, granite, marble, wood, wood_advanced, brushed, brushed_radial, scratches, leather, weave, camouflage, contour, mesh. Call tequila_material_guide once for what each looks like, their settings, the rules of thumb and 16 tested example recipes to start from.',
         {"material": {"type": "integer", "description": "The material's id."}, "recipe": {"type": "object", "description": "The new recipe. Without a name the material keeps its own."}, "preview": {"type": "boolean", "description": "Return a preview image too (default True)."}},
         ['material', 'recipe']),
    tool('tequila_material_recipe',
         'Read any material - one you made, or a library preset already on a part - back as a recipe, to change it and send it to tequila_material_update or tequila_material_create.',
         {"material": {"type": "integer"}},
         ['material']),
    tool('tequila_material_save',
         "Keep a material in the person's own material library, so it is in the Library tab for every project.",
         {"material": {"type": "integer"}, "name": {"type": "string"}, "category": {"type": "string", "description": "Default: My materials."}, "tags": {"type": "array", "items": {"type": "string"}}},
         ['material']),
    tool('tequila_batch', 'Several operations as ONE undo step. Each is a request object with an "op": '
         'apply, select, frame, look, ui, import, save, screenshot, queue. If one fails the whole batch is taken back, unless atomic is false.',
         {'ops': {'type': 'array', 'items': {'type': 'object'}}, 'atomic': {'type': 'boolean'}},
         ['ops']),
    tool('tequila_undo', 'Take back the last step (a single edit or a whole batch).'),
    tool('tequila_redo', 'Put back the last undone step.'),
    tool('tequila_select', 'Replace the selection with these node ids.',
         {'nodes': {'type': 'array', 'items': {'type': 'integer'}}}, ['nodes']),
    tool('tequila_frame', 'Point the camera at everything, or at the selection.',
         {'all': {'type': 'boolean', 'description': 'true for everything (default), false for the selection.'}}),
    tool('tequila_look', 'Place the camera: a standard view, orbit angles around a target, or an exact eye position. Undoable.',
         {'view': {'type': 'string', 'enum': ['front', 'back', 'left', 'right', 'top', 'bottom', 'iso']},
          'at': {**VECTOR, 'description': 'Target point [x, y, z].'},
          'from': {**VECTOR, 'description': 'Eye position [x, y, z]; not with yaw/pitch/distance.'},
          'yaw': {'type': 'number', 'description': 'Degrees around +Y; 0 looks from the front.'},
          'pitch': {'type': 'number', 'description': 'Degrees above the horizon.'},
          'distance': {'type': 'number'},
          'focal_length': {'type': 'number', 'description': 'Millimetres.'}}),
    tool('tequila_screenshot', 'See the application as it is now. Returns an image.',
         {'viewport': {'type': 'boolean', 'description': 'true for just the 3D view, without panels (default true).'}}),
    tool('tequila_render', 'Render the current view to an image and return it. Blocks Tequila while it runs: keep it small (e.g. 640x400, 32 samples).',
         {'width': {'type': 'integer'}, 'height': {'type': 'integer'}, 'samples': {'type': 'integer'}}),
    tool('tequila_queue', 'Start a background render with the document\'s render settings; returns a job id at once. Poll tequila_jobs. studio renders one view; studios=true renders every view.',
         {'name': {'type': 'string'}, 'width': {'type': 'integer'}, 'height': {'type': 'integer'},
          'samples': {'type': 'integer'}, 'folder': {'type': 'string'}, 'filename': {'type': 'string'},
          'denoise': {'type': 'boolean'}, 'camera': {'type': 'integer'},
          'studio': {'type': 'string', 'description': 'A Studio (a view) by name or index, applied for the render and taken back.'},
          'studios': {'type': 'boolean', 'description': 'Every Studio, one render each, named for its view.'}}),
    tool('tequila_jobs', 'The background render queue: state, progress and output file of each job.'),
    tool('tequila_stop', 'Stop the running background render; it still writes its image.'),
    tool('tequila_ui', 'Open or close panels and switch the viewport shading.',
         {'render_window': {'type': 'boolean'}, 'console': {'type': 'boolean'}, 'inspector': {'type': 'boolean'},
          'library': {'type': 'boolean'}, 'full_viewport': {'type': 'boolean'}, 'help': {'type': 'boolean'},
          'frame': {'type': 'boolean', 'description': "Show the viewport at the render's size and shape (on by default)."},
          'graph': {'type': ['integer', 'boolean'],
                    'description': 'Open the Material Graph editor on this material number; false closes it.'},
          'tab': {'type': 'string', 'enum': ['scene', 'material', 'environment', 'lighting', 'camera', 'image', 'studios', 'scripting']},
          'viewport': {'type': 'string', 'enum': ['wireframe', 'solid', 'material', 'rendered']}}),
    tool('tequila_import', 'Load a model file into the scene: STEP or IGES (CAD, divided into parts with their colours), glTF/GLB, OBJ, STL, USD (.usda) or IFC. Not FBX: ask for glTF, OBJ or USD instead.',
         {'path': {'type': 'string'}}, ['path']),
    tool('tequila_save', 'Save the project, to its own file or to a new path.',
         {'path': {'type': 'string'}}),
    tool('tequila_plugin_guide',
         'How to write a Tequila plugin: the full reference (manifest, Python API, panels and their field types, events, material packs), a working template, and the way to work - write, ask the person to turn it on, run, read the log, fix. Read it once before writing a plugin.',
         {},
         []),
    tool('tequila_plugin_list',
         "Every installed plugin: id, name, version, whether it is on, running or failed (and why), its commands, panels and events, and its folder. Also whether Python was found, and the person's plugins folder.",
         {},
         []),
    tool('tequila_plugin_write',
         "Write a plugin's files into the person's plugins folder: a new plugin, or new versions of an existing one's files (others are kept). Give plugin.json (its id must be `id`) and main.py at least for a new one. Text files only (.py .json .md .txt .csv .toml .ini .cfg), paths inside the plugin's folder. A running plugin restarts with the new code; a new one must be turned on with tequila_plugin_enable.",
         {"id": {"type": "string", "description": "The plugin id and folder name, e.g. \"my.batch-renamer\": letters, digits, . - _"}, "files": {"type": "object", "description": "Path inside the plugin folder -> the file's full text, e.g. {\"plugin.json\": \"...\", \"main.py\": \"...\"}.", "additionalProperties": {"type": "string"}}},
         ['id', 'files']),
    tool('tequila_plugin_enable',
         'Turn a plugin on. A plugin with Python code runs as the person, so Tequila asks them in its Plugins window and only they can say yes: tell them to look at Tequila, then check tequila_plugin_list for "enabled". A material pack is turned on at once.',
         {"id": {"type": "string"}},
         ['id']),
    tool('tequila_plugin_run',
         "Run one of an enabled plugin's commands (or a panel button's command) with `values` as its panel would send them, wait a little, and return what it logged: its prints, status messages, `finished`, or the traceback if it failed. A command with a panel opens the panel instead.",
         {"id": {"type": "string"}, "command": {"type": "string"}, "values": {"type": "object", "description": "Field id -> value, as the panel would send."}, "wait": {"type": "number", "description": "Seconds to wait for it to finish (default 10, at most 120)."}},
         ['id', 'command']),
    tool('tequila_plugin_log',
         "The last lines of a plugin's log: what it printed, its status messages, finished commands and tracebacks. Use it to debug.",
         {"id": {"type": "string"}, "lines": {"type": "integer", "description": "How many (default 40)."}},
         ['id']),
    tool('tequila_request', 'Send any raw request object (with an "op") for anything the other tools do not cover.',
         {'request': {'type': 'object'}}, ['request']),
]


# Tools that reach outside the open scene: gone in restricted mode.
UNRESTRICTED_ONLY = {
    'tequila_request', 'tequila_import', 'tequila_material_save',
    'tequila_plugin_guide', 'tequila_plugin_list', 'tequila_plugin_write',
    'tequila_plugin_enable', 'tequila_plugin_run', 'tequila_plugin_log',
}
# What a batch may hold in restricted mode: nothing that writes a file.
RESTRICTED_BATCH_OPS = {'apply', 'select', 'frame', 'look', 'ui'}


class Bridge:
    """Tool calls in, channel requests out. One connection, re-made when the
    application restarts."""

    def __init__(self, discovery=None, connect=Client, restricted=False):
        self._discovery = discovery
        self._connect = connect
        self._client = None
        self.restricted = restricted

    def tools(self):
        if not self.restricted:
            return TOOLS
        kept = []
        for t in TOOLS:
            if t['name'] in UNRESTRICTED_ONLY:
                continue
            if t['name'] in ('tequila_save', 'tequila_queue'):
                t = json.loads(json.dumps(t))
                for field in ('path', 'folder', 'filename'):
                    t['inputSchema']['properties'].pop(field, None)
            kept.append(t)
        return kept

    def client(self):
        if self._client is None:
            try:
                self._client = self._connect(self._discovery, timeout=600)
            except (OSError, ValueError, TequilaError) as error:
                raise TequilaError(f'Tequila does not seem to be open (or its channel is off): {error}') from error
        return self._client

    def close(self):
        if self._client is not None:
            self._client.close()
            self._client = None

    def request(self, op, **fields):
        try:
            return self.client().request(op, **fields)
        except TequilaError as error:
            # A closed connection means the next call should reconnect; a
            # refusal from the application leaves it open. Either way the
            # error goes back to the assistant, and nothing is retried.
            if getattr(self._client, '_socket', None) is not None and self._client._socket.fileno() == -1:
                self._client = None
            raise error

    # ---- tools ---------------------------------------------------------------

    def call(self, name, arguments):
        handler = getattr(self, 'tool_' + name.removeprefix('tequila_'), None)
        if handler is None or not name.startswith('tequila_') or name not in {t['name'] for t in self.tools()}:
            raise KeyError(name)
        arguments = dict(arguments or {})
        if self.restricted:
            def check_command(command):
                if isinstance(command, dict) and command.get('command') == 'render.set_settings':
                    raise TequilaError('Render output settings are controlled locally; use tequila_queue for resolution, samples and denoising')
            if name == 'tequila_apply':
                check_command(arguments.get('command'))
            for field in ('path', 'folder', 'filename'):
                if name in ('tequila_save', 'tequila_queue') and field in arguments:
                    raise TequilaError(f'{field} is not available remotely; the project and render settings decide where files go')
            if name == 'tequila_batch':
                for i, op in enumerate(arguments.get('ops', [])):
                    if op.get('op') == 'apply':
                        check_command(op.get('command'))
                    if op.get('op') not in RESTRICTED_BATCH_OPS:
                        raise TequilaError(f'operation {i + 1} ({op.get("op")}) is not available remotely; a batch may hold {", ".join(sorted(RESTRICTED_BATCH_OPS))}')
        return handler(**arguments)

    def tool_scene(self):
        return text(self.request('query', what='scene'))

    def tool_nodes(self, filter=None, parts_only=False):
        nodes = self.request('query', what='nodes')['nodes']
        if parts_only:
            nodes = [n for n in nodes if n.get('kind') == 'part']
        if filter:
            word = filter.lower()
            nodes = [n for n in nodes if word in n.get('name', '').lower() or word in n.get('material_name', '').lower()]
        # The shape of the answer is the useful part; drop the corners, keep
        # the size, so a split model's list stays readable.
        slim = []
        for n in nodes:
            entry = {k: n[k] for k in ('id', 'name', 'kind', 'material', 'material_name', 'visible') if k in n}
            if 'bounds' in n:
                entry['size'] = [round(v, 4) for v in n['bounds']['size']]
                entry['center'] = [round((a + b) / 2, 4) for a, b in zip(n['bounds']['min'], n['bounds']['max'])]
            slim.append(entry)
        return text({'count': len(slim), 'nodes': slim})

    def tool_materials(self):
        return text(self.request('query', what='materials'))

    def tool_query(self, what):
        return text(self.request('query', what=what))

    def tool_apply(self, command):
        self.request('apply', command=command)
        return text({'ok': True, 'applied': command.get('command')})

    def tool_material_guide(self):
        return text(self.request('material', action='guide'))

    def tool_material_preview(self, recipe=None, material=None, radius=None, shape=None):
        path = Path(tempfile.gettempdir()) / f'tequila-mcp-material-{time.time_ns()}.png'
        fields = {k: v for k, v in dict(recipe=recipe, material=material, radius=radius, shape=shape).items() if v is not None}
        shown = self.request('material', action='preview', path=str(path), **fields)
        return image(path, json.dumps(shown))

    def _material_and_picture(self, action, preview, **fields):
        made = self.request('material', action=action, **{k: v for k, v in fields.items() if v is not None})
        out = text(made)
        if preview and isinstance(made, dict) and 'material' in made:
            path = Path(tempfile.gettempdir()) / f'tequila-mcp-material-{time.time_ns()}.png'
            try:
                self.request('material', action='preview', material=made['material'], path=str(path))
                out += image(path, 'the material on a 5 cm ball')
            except TequilaError:
                pass
        return out

    def tool_material_create(self, recipe, nodes=None, preview=True):
        return self._material_and_picture('create', preview, recipe=recipe, nodes=nodes)

    def tool_material_update(self, material, recipe, preview=True):
        return self._material_and_picture('update', preview, material=material, recipe=recipe)

    def tool_material_recipe(self, material):
        return text(self.request('material', action='recipe', material=material))

    def tool_material_save(self, material, name=None, category=None, tags=None):
        fields = {k: v for k, v in dict(name=name, category=category, tags=tags).items() if v is not None}
        return text(self.request('material', action='save', material=material, **fields))

    def tool_plugin_guide(self):
        return text(self.request('plugin', action='guide'))

    def tool_plugin_list(self):
        return text(self.request('plugin', action='list'))

    def tool_plugin_write(self, id, files):
        return text(self.request('plugin', action='write', plugin=id, files=files))

    def tool_plugin_enable(self, id):
        return text(self.request('plugin', action='enable', plugin=id))

    def tool_plugin_log(self, id, lines=None):
        fields = {'lines': lines} if lines is not None else {}
        return text(self.request('plugin', action='log', plugin=id, **fields))

    def tool_plugin_run(self, id, command, values=None, wait=10):
        """Run the command, then wait for it to finish, answering with what
        it logged meanwhile: its prints, `finished`, or its traceback."""
        before = self.request('plugin', action='log', plugin=id, lines=1).get('total', 0)
        fields = {'values': values} if values is not None else {}
        sent = self.request('plugin', action='run', plugin=id, command=command, **fields)
        if 'opened_panel' in sent:
            return text(sent)
        finished = f'finished `{command}`'
        deadline = time.monotonic() + max(0.0, min(float(wait), 120.0))
        while True:
            log = self.request('plugin', action='log', plugin=id, lines=400)
            lines = log.get('lines', [])
            new = min(max(0, int(log.get('total', 0)) - int(before)), len(lines))
            said = lines[len(lines) - new:] if new else []
            done = finished in said
            stopped = not log.get('running', False)
            if done or stopped or time.monotonic() >= deadline:
                out = {'finished': done, 'log': said}
                if not done:
                    out['note'] = ('the plugin is not running; see its failure in tequila_plugin_list' if stopped
                                   else 'still running; tequila_plugin_log shows more later')
                return text(out)
            time.sleep(0.15)

    def tool_batch(self, ops, atomic=True):
        return text(self.request('batch', ops=ops, atomic=atomic))

    def tool_undo(self):
        return text(self.request('undo'))

    def tool_redo(self):
        return text(self.request('redo'))

    def tool_select(self, nodes):
        self.request('select', nodes=list(nodes))
        return text({'ok': True, 'selected': list(nodes)})

    def tool_frame(self, all=True):
        return text(self.request('frame', all=all))

    def tool_look(self, **fields):
        return text(self.request('look', **fields))

    def tool_screenshot(self, viewport=True):
        path = Path(tempfile.gettempdir()) / f'tequila-mcp-{time.time_ns()}.png'
        self.client().screenshot(path, viewport=viewport)
        return image(path, f'Tequila {"viewport" if viewport else "window"}')

    def tool_render(self, width=640, height=400, samples=32):
        path = Path(tempfile.gettempdir()) / f'tequila-mcp-render-{time.time_ns()}.png'
        result = self.request('render', path=str(path), width=width, height=height, samples=samples)
        return image(path, json.dumps(result))

    def tool_queue(self, **job):
        return text(self.request('queue', **job))

    def tool_jobs(self):
        return text(self.request('query', what='jobs'))

    def tool_stop(self):
        return text(self.request('stop'))

    def tool_ui(self, **changes):
        return text(self.request('ui', **changes))

    def tool_import(self, path):
        return text(self.request('import', path=str(Path(path).resolve())))

    def tool_save(self, path=None):
        return text(self.request('save', **({'path': str(Path(path).resolve())} if path else {})))

    def tool_request(self, request):
        fields = dict(request)
        op = fields.pop('op', None)
        if not op:
            raise TequilaError('the request needs an "op"')
        fields.pop('id', None)
        fields.pop('token', None)
        return text(self.request(op, **fields))


def text(value):
    return [{'type': 'text', 'text': value if isinstance(value, str) else json.dumps(value, indent=1)}]


def image(path, caption):
    data = base64.b64encode(Path(path).read_bytes()).decode('ascii')
    try:
        Path(path).unlink()
    except OSError:
        pass
    return [{'type': 'image', 'data': data, 'mimeType': 'image/png'}, {'type': 'text', 'text': caption}]


# ---- JSON-RPC over stdio -----------------------------------------------------

def handle(bridge, message):
    """Answer one JSON-RPC message; None for a notification."""
    method = message.get('method')
    ident = message.get('id')
    params = message.get('params') or {}

    def reply(result):
        return {'jsonrpc': '2.0', 'id': ident, 'result': result}

    def error(code, text_):
        return {'jsonrpc': '2.0', 'id': ident, 'error': {'code': code, 'message': text_}}

    if ident is None:
        return None  # notifications: initialized, cancelled — nothing to say
    if method == 'initialize':
        asked = params.get('protocolVersion')
        return reply({
            'protocolVersion': asked if asked in PROTOCOL_VERSIONS else PROTOCOL_VERSIONS[0],
            'capabilities': {'tools': {}},
            'serverInfo': SERVER_INFO,
            'instructions': INSTRUCTIONS,
        })
    if method == 'ping':
        return reply({})
    if method == 'tools/list':
        return reply({'tools': bridge.tools()})
    if method == 'tools/call':
        name = params.get('name', '')
        try:
            content = bridge.call(name, params.get('arguments'))
            return reply({'content': content, 'isError': False})
        except KeyError:
            return error(-32602, f'Unknown tool: {name}')
        except TypeError as problem:
            return reply({'content': text(f'Bad arguments for {name}: {problem}'), 'isError': True})
        except (TequilaError, TimeoutError, FileExistsError, OSError) as problem:
            # An application refusal is a result the assistant should read and
            # act on, not a protocol failure.
            return reply({'content': text(str(problem)), 'isError': True})
    return error(-32601, f'Method not found: {method}')


def serve(bridge=None, stdin=sys.stdin, stdout=sys.stdout):
    bridge = bridge or Bridge()
    try:
        _serve(bridge, stdin, stdout)
    finally:
        bridge.close()


def _serve(bridge, stdin, stdout):
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except ValueError:
            response = {'jsonrpc': '2.0', 'id': None, 'error': {'code': -32700, 'message': 'Parse error'}}
        else:
            response = handle(bridge, message)
        if response is not None:
            stdout.write(json.dumps(response) + '\n')
            stdout.flush()


if __name__ == '__main__':
    serve(Bridge(restricted='--restricted' in sys.argv[1:]))
