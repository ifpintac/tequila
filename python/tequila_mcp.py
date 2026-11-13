"""An MCP server that lets an AI app drive a running Tequila.

Model Context Protocol clients — the Claude app, Cursor, VS Code with GitHub
Copilot, Codex, Gemini CLI, Windsurf, LM Studio and others — start this program and talk to it over stdin/stdout. It turns their
tool calls into requests on Tequila's local scripting channel, and hands
screenshots and renders back as images, so the assistant can see what it did.

Standard library only. Run it with the Python that runs `tequila.py`:

    claude mcp add tequila -- python C:/path/to/tequila/python/tequila_mcp.py

Tequila must be open; the bridge finds it through ~/.tequila/channel.json
(data/channel.json in a portable copy, when this is the bridge inside it)
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
    'Before working in an area for the first time, read its page of the guide with tequila_guide: materials, recipes, graph, textures, labels, uv, '
    'scene, lighting, camera, variants, rendering, image, animation, fixing, files, plugins, app, scripting, shots - and start, the short guide to '
    'working here, if you have no Tequila skill. Each page is short, and tested against this build. '
    'Start with tequila_scene, then tequila_nodes to find parts by name, material or size. '
    'For any material, look in the library first: tequila_library searches several hundred finished presets '
    '(leather, fabric, metal, plastic, wood, glass...) and tequila_library_apply puts one on parts. '
    'Make a material yourself only when the library has nothing close - tequila_material_guide, tequila_material_preview, '
    'then tequila_material_create and tequila_material_update - or change a preset once it is on a part. '
    'Materials may be shared between parts: unlink a part (material.unlink) before changing it alone. '
    'Before sending a command you have not used, ask tequila_command for it: it answers with the exact fields and an example. '
    'Never guess a field name; a field that is not in the example is not read. '
    'A command that takes a whole block back (render.set_settings, image.set_style, camera.set, studio.set...) starts from tequila_read, '
    'which reads one part of the project or one node; never read the whole project to find one setting. '
    'Use tequila_batch for several edits: they become one undo step, and if one fails none of them stay. '
    'A new plane, cube, cylinder, sphere, disc, backdrop or wall is tequila_add_shape. '
    'Look at your work with tequila_screenshot: it waits for the picture to finish and says whether it had. '
    'If something looks wrong, first check the simple causes with tequila_nodes: is the part hidden, is it where you think. '
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


# The pages of the guide (`crates/tequila-scripting/src/guide.rs`, which a
# test holds this list to) and the parts of a project `read` answers with.
GUIDE_TOPICS = ['start', 'materials', 'recipes', 'graph', 'textures', 'labels', 'uv', 'scene', 'lighting', 'camera', 'variants',
                'rendering', 'image', 'animation', 'fixing', 'files', 'plugins', 'app', 'scripting', 'shots']
PROJECT_PARTS = ['render', 'image_style', 'color_management', 'environment', 'lighting', 'cameras', 'units', 'multi_materials',
                 'model_sets', 'studios', 'configurator', 'animation', 'product_turn', 'material_templates', 'measurements', 'textures']

TOOLS = [
    tool('tequila_guide', 'How to do a job in Tequila: one page of the guide, short enough to read whole, with tested examples. '
         'READ THE PAGE for an area before working in it for the first time. Topics: start (how to work here: look, change, check the picture, undo), '
         'materials (the library, changing a preset, types, colours), recipes (a material of your own), graph (the material graph and its nodes), '
         'textures (picture maps, texture packs, Substance), labels (logos and decals), uv (UV layouts), scene (parts, groups, moving, shapes, units, cutaways), '
         'lighting (environment, lights, floor), camera (views, lens, depth of field, composition), variants (colourways, Snapshots, configurator, studio presets), '
         'rendering (quick look, final render, queue, format, transparency, layers), image (the look after the render), animation (turntable, spin), '
         'fixing (a model that came in wrong), files (import, update, save, export), plugins, app (panels and preferences), scripting (raw requests, Python), '
         'shots (whole shots step by step). With no topic, the list of pages.',
         {'topic': {'type': 'string', 'enum': GUIDE_TOPICS}}),
    tool('tequila_read', 'Read one part of the project, small enough to read: what is render (the render settings), image_style, color_management, environment '
         '(with its light pins and the floor), lighting, cameras, units, multi_materials (colourways), model_sets, studios (Snapshots), configurator, animation, '
         'product_turn, material_templates, measurements or textures - each as the command that sets it takes it back. index takes one entry of a list (one camera, one Snapshot). '
         'node reads one node of the scene by its id: its transform (place, turn and size), and a light\'s or a cutaway\'s settings. '
         'With nothing, what there is to read. Use it before any command that takes a whole block (render.set_settings, image.set_style, camera.set, studio.set, scene.set_light): '
         'read, change what was asked, send all of it back.',
         {'what': {'type': 'string', 'enum': PROJECT_PARTS},
          'index': {'type': 'integer', 'description': 'One entry of a part that is a list, numbered from 0.'},
          'node': {'type': 'integer', 'description': 'A node\'s id, from tequila_nodes: read that node instead of a part.'}}),
    tool('tequila_scene', 'Counts, units, overall bounds and the file of the open scene.'),
    tool('tequila_nodes', 'Parts and groups: id, name, material and world size. Filter by a word in the name or material name to keep the answer short.',
         {'filter': {'type': 'string', 'description': 'Only nodes whose name or material name contains this (case-insensitive).'},
          'parts_only': {'type': 'boolean'}}),
    tool('tequila_materials', 'Every material in the scene, briefly: id, name, type, how many parts use it and how many texture maps it has. tequila_material gives one in full.'),
    tool('tequila_material', 'One material of the scene in full: every parameter, pattern and texture map.',
         {'id': {'type': 'integer', 'description': 'The material\'s id, from tequila_materials or tequila_nodes.'}},
         ['id']),
    tool('tequila_library', 'Search the material library: several hundred finished presets. LOOK HERE FIRST for any material, before making one. '
         'With no arguments it lists the categories and how many presets each has. search takes words that must all match the name, category, type, tags or description '
         '("black leather", "brushed aluminium", "translucent ripstop"); category narrows to one. Answers with name, category, type and a line about each: the name is what tequila_library_apply takes.',
         {'search': {'type': 'string', 'description': 'Words to look for: a colour, a finish, what it is made of.'},
          'category': {'type': 'string', 'description': 'One category, as the list with no arguments names them.'},
          'limit': {'type': 'integer', 'description': 'How many presets to answer with at most (default 20).'}}),
    tool('tequila_library_apply', 'Put a library preset on parts, by its exact name from tequila_library. One undo step. '
         'unlink=true gives the parts a copy of their own, to change without changing the other parts that use the preset. '
         'To change its colour afterwards, read it with tequila_material: a preset that sets its colour by a pattern needs the pattern\'s colours changed (tequila_material_recipe, then tequila_material_update), not base_color.',
         {'name': {'type': 'string'},
          'nodes': {'type': 'array', 'items': {'type': 'integer'}, 'description': 'Part ids to put it on; the selection when left out.'},
          'unlink': {'type': 'boolean'}},
         ['name']),
    tool('tequila_command', 'How to write one command for tequila_apply: its exact fields and a working example, and for a setting-by-name command the names it takes. '
         'With no command, the name of every command. Ask before using a command for the first time, rather than guessing its fields.',
         {'command': {'type': 'string', 'description': 'A command\'s name, e.g. "scene.set_visibility" or "environment.set_parameter".'}}),
    tool('tequila_add_shape', 'Add a simple shape as a new part: plane, cube, cylinder, sphere, disc, backdrop or wall. It stands beside what is in the scene, on the floor, half its size unless sizes are given. '
         'Not for the floor or the background, which the scene already has: those are settings of the environment (tequila_guide lighting). '
         'Sizes are metres: width (a cylinder\'s, sphere\'s and disc\'s diameter), depth, height, radius (half a round shape\'s width; a backdrop\'s curve). One undo step. Answers with the new part\'s id and its sizes.',
         {'shape': {'type': 'string', 'enum': ['plane', 'cube', 'cylinder', 'sphere', 'disc', 'backdrop', 'wall']},
          'width': {'type': 'number'}, 'depth': {'type': 'number'}, 'height': {'type': 'number'}, 'radius': {'type': 'number'}},
         ['shape']),
    tool('tequila_query', 'A read-only query. Short answers: scene, selection, camera, commands, ui, jobs, history. Long ones, tens of thousands of characters '
         'on a real scene, which the tool named does a page at a time: context (tequila_read), api (tequila_command; the materials page of tequila_guide), '
         'materials (tequila_materials, tequila_material), nodes (tequila_nodes), library (tequila_library).',
         {'what': {'type': 'string', 'enum': ['api', 'context', 'scene', 'nodes', 'materials', 'selection', 'camera', 'commands', 'ui', 'jobs', 'history', 'library']}},
         ['what']),
    tool('tequila_apply', 'Apply one command, as the guide\'s page for the job shows it (tequila_guide first), e.g. {"command":"material.set_type","material":2,"type":"velvet"}. '
         'tequila_command gives any command\'s exact fields and an example: ask it first. tequila_query what=api describes material parameters and types. One undo step.'
         ' Colours may be hex sRGB strings ("value": "#c0392b"), converted for you; arrays are linear ACEScg.',
         {'command': {'type': 'object', 'description': 'A journal command: {"command": "<name>", ...fields}.'}},
         ['command']),
    tool('tequila_material_guide',
         'How to make procedural materials, a part at a time. With no topic: how to work, and the patterns and examples there are. '
         'topic recipe: the recipe format, the rules of thumb and the parameters that matter - read it before writing a first recipe. types: every material type; a type\'s id (plastic, metal, velvet...): the parameters it shows. '
         'patterns: every pattern in a line; a pattern\'s kind (leather, weave, wood_advanced...): its settings and defaults. '
         'examples: the 16 tested recipes in a line each; an example\'s name (Terrazzo, Walnut, Carbon Fibre...): its recipe, to change and preview.',
         {'topic': {'type': 'string', 'description': 'contents, recipe, types, patterns, examples, a type\'s id, a pattern\'s kind or an example\'s name.'}},
         []),
    tool('tequila_material_preview',
         'Show a material recipe (or an existing material by id) on a ball, without changing the scene. Returns an image. The fastest way to try an idea: preview, adjust, preview again, then tequila_material_create. Recipe: {"name": "Speckled Stone", "type": "plastic" (or metal, glass, paint, velvet... see the guide), any parameter as a value ("specular_roughness": 0.3, "base_color": "#c0392b" - hex sRGB is fine), "patterns": [{"param": "base_color", "procedural": {"kind": "noise"}, "size": 0.01, "colors": [{"at": 0, "color": "#d8d4cc"}, {"at": 1, "color": "#a9a59c"}]}, {"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.02, "range": [0.2, 0.4]}, {"param": "bump", "procedural": {"kind": "leather"}, "size": 0.002, "depth": 0.0001}]}. `size` is the feature size in metres at real-world scale. A later pattern on the same param is a layer: "blend" mix/multiply/add, "amount" 0-1, "mask" {"threshold": 0.5, "softness": 0.05}. Pattern kinds: noise, noise_texture, ridged, cellular, spots, granite, marble, wood, wood_advanced, brushed, brushed_radial, scratches, leather, weave, camouflage, contour, mesh. Call tequila_material_guide once for what each looks like, their settings, the rules of thumb and 16 tested example recipes to start from.',
         {"recipe": {"type": "object", "description": "The recipe to show."}, "material": {"type": "integer", "description": "Or an existing material's id."}, "radius": {"type": "number", "description": "The ball's radius in metres (default 0.025). Use the real part's size to judge pattern scale."}, "shape": {"type": "string", "enum": ["sphere", "slab"]}},
         []),
    tool('tequila_material_create',
         'Make a new material from a recipe and put it on parts, as one undo step. Only when tequila_library has nothing close: search it first. Returns its id and a preview image. Recipe: {"name": "Speckled Stone", "type": "plastic" (or metal, glass, paint, velvet... see the guide), any parameter as a value ("specular_roughness": 0.3, "base_color": "#c0392b" - hex sRGB is fine), "patterns": [{"param": "base_color", "procedural": {"kind": "noise"}, "size": 0.01, "colors": [{"at": 0, "color": "#d8d4cc"}, {"at": 1, "color": "#a9a59c"}]}, {"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.02, "range": [0.2, 0.4]}, {"param": "bump", "procedural": {"kind": "leather"}, "size": 0.002, "depth": 0.0001}]}. `size` is the feature size in metres at real-world scale. A later pattern on the same param is a layer: "blend" mix/multiply/add, "amount" 0-1, "mask" {"threshold": 0.5, "softness": 0.05}. Pattern kinds: noise, noise_texture, ridged, cellular, spots, granite, marble, wood, wood_advanced, brushed, brushed_radial, scratches, leather, weave, camouflage, contour, mesh. Call tequila_material_guide once for what each looks like, their settings, the rules of thumb and 16 tested example recipes to start from.',
         {"recipe": {"type": "object", "description": "The recipe."}, "nodes": {"type": "array", "items": {"type": "integer"}, "description": "Part ids to put it on; the selection when left out."}, "preview": {"type": "boolean", "description": "Return a preview image too (default true)."}},
         ['recipe']),
    tool('tequila_material_update',
         'Remake an existing material from a recipe, where it is: every part using it changes. One undo step; returns a preview image. Use it to refine a material after looking at it on the product. Recipe: {"name": "Speckled Stone", "type": "plastic" (or metal, glass, paint, velvet... see the guide), any parameter as a value ("specular_roughness": 0.3, "base_color": "#c0392b" - hex sRGB is fine), "patterns": [{"param": "base_color", "procedural": {"kind": "noise"}, "size": 0.01, "colors": [{"at": 0, "color": "#d8d4cc"}, {"at": 1, "color": "#a9a59c"}]}, {"param": "specular_roughness", "procedural": {"kind": "noise"}, "size": 0.02, "range": [0.2, 0.4]}, {"param": "bump", "procedural": {"kind": "leather"}, "size": 0.002, "depth": 0.0001}]}. `size` is the feature size in metres at real-world scale. A later pattern on the same param is a layer: "blend" mix/multiply/add, "amount" 0-1, "mask" {"threshold": 0.5, "softness": 0.05}. Pattern kinds: noise, noise_texture, ridged, cellular, spots, granite, marble, wood, wood_advanced, brushed, brushed_radial, scratches, leather, weave, camouflage, contour, mesh. Call tequila_material_guide once for what each looks like, their settings, the rules of thumb and 16 tested example recipes to start from.',
         {"material": {"type": "integer", "description": "The material's id."}, "recipe": {"type": "object", "description": "The new recipe. Without a name the material keeps its own."}, "preview": {"type": "boolean", "description": "Return a preview image too (default true)."}},
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
    tool('tequila_screenshot', 'See the application as it is now. Returns an image. After a change the rendered view is dark and grainy for a few seconds while it redraws: '
         'this waits for the picture to finish, up to settle seconds, and says with the image whether it had. Do not judge a look from one it calls unfinished: look again.',
         {'viewport': {'type': 'boolean', 'description': 'true for just the 3D view, without panels (default true).'},
          'settle': {'type': 'number', 'description': 'Seconds to give the picture to finish (default 12; 0 takes it at once).'},
          'window': {'type': 'string', 'enum': ['uv', 'graph', 'render', 'preferences'],
                     'description': 'One of Tequila\'s other windows instead, which must be open: uv (the UV editor), graph (the Material Graph editor), render (the Render window), preferences.'}}),
    tool('tequila_render', "Render the current view to an image and return it. Blocks Tequila while it runs: keep it small (e.g. 640x400, 32 samples). wireframe=true draws the model's edges instead, as the project's Wireframe settings say - no ray tracing, a second or two at any size.",
         {'width': {'type': 'integer'}, 'height': {'type': 'integer'}, 'samples': {'type': 'integer'},
          'wireframe': {'type': 'boolean'}}),
    tool('tequila_queue', 'Start a background render with the document\'s render settings; returns a job id at once. Poll tequila_jobs. studio renders one view; studios=true renders every view; '
         'configurations=true every valid combination of the configurator; frames=true the frames of the turntable (animation.set), one render each, leaving out those already rendered.',
         {'name': {'type': 'string'}, 'width': {'type': 'integer'}, 'height': {'type': 'integer'},
          'samples': {'type': 'integer'}, 'folder': {'type': 'string'}, 'filename': {'type': 'string'},
          'denoise': {'type': 'boolean'}, 'camera': {'type': 'integer'},
          'studio': {'type': 'string', 'description': 'A Studio (a view) by name or index, applied for the render and taken back.'},
          'studios': {'type': 'boolean', 'description': 'Every Studio, one render each, named for its view.'},
          'engine': {'type': 'string', 'enum': ['tequila', 'cycles-cpu', 'cycles-gpu'], 'description': 'The engine for this render; the document\'s when left out.'},
          'configurations': {'type': 'boolean', 'description': 'One render for every valid combination of the configurator, named by its choices.'},
          'frames': {'type': 'boolean', 'description': 'One render for every frame of the turntable, the product turned to each frame\'s angle; the file name takes the frame\'s number.'},
          'first': {'type': 'integer', 'description': 'With frames: the first frame to render (default 0).'},
          'last': {'type': 'integer', 'description': 'With frames: the last frame to render (default the last there is).'}}),
    tool('tequila_jobs', 'The background render queue: state, progress and output file of each job.'),
    tool('tequila_studio_presets',
         'The studio presets installed: complete studios - lights, cameras and views, floor, look - for a kind of product, '
         'by family and size. Each lists its views and the product radius it was framed for; "suggested" marks the size '
         'in each family nearest the product in the scene.'),
    tool('tequila_studio_preset_apply',
         "Apply an installed studio preset as ONE undo step: its lights, cameras and views (Studios), environment and floor, "
         "look and render settings, framed to the product's size and the product stood on its floor. The first view is "
         "applied; tequila_studio_render_all renders them all.",
         {'preset': {'type': 'string', 'description': "The preset's id, from tequila_studio_presets."},
          'front': {'type': 'string', 'enum': ['+z', '-z', '+x', '-x'],
                    'description': "The product's side that should face the Main camera (default +z)."},
          'scale': {'type': 'string', 'enum': ['cameras', 'cameras_and_lights', 'none'],
                    'description': "What follows the product's size (default: the preset's own choice)."},
          'keep_lights': {'type': 'boolean'},
          'keep_cameras': {'type': 'boolean'},
          'keep_placement': {'type': 'boolean', 'description': 'Leave the product where it is.'},
          'dress': {'type': 'boolean', 'description': "Also put the preset's materials on parts by their names (its material rules)."}},
         ['preset']),
    tool('tequila_studio_preset_save',
         "Keep the scene's studio - everything but the product - as a studio preset of the person's own, framed for the product it has now.",
         {'name': {'type': 'string'}, 'family': {'type': 'string'}, 'size': {'type': 'string'},
          'description': {'type': 'string'},
          'materials': {'type': 'boolean', 'description': "Keep the project's materials with it."}},
         ['name']),
    tool('tequila_studio_render_all',
         'Render every view (Studio) in the background, one image each, named for its view. Only when asked for renders.'),
    tool('tequila_stop', 'Stop the running background render; it still writes its image, and the jobs waiting behind it start. job stops one job by its id from tequila_jobs: '
         'the one rendering is stopped, one still waiting is taken out of the queue and writes nothing. all=true stops the one rendering and takes out every job waiting: '
         'how to end a spin, a turntable or a batch of views early.',
         {'job': {'type': 'integer'}, 'all': {'type': 'boolean'}}),
    tool('tequila_ui', 'Open or close panels and switch the viewport shading.',
         {'render_window': {'type': 'boolean'}, 'console': {'type': 'boolean'}, 'inspector': {'type': 'boolean'},
          'library': {'type': 'boolean'}, 'full_viewport': {'type': 'boolean'}, 'statistics': {'type': 'boolean'}, 'help': {'type': 'boolean'},
          'frame': {'type': 'boolean', 'description': "Show the viewport at the render's size and shape (on by default)."},
          'selection_outline': {'type': 'boolean', 'description': "The selection's outline in the viewport (Shift+H); hiding it keeps the selection."},
          'gizmo': {'type': 'boolean', 'description': 'The move, turn and scale gizmo on the selection (Shift+G).'},
          'start_screen': {'type': 'boolean', 'description': 'The start screen, with recent projects and the sample scenes.'},
          'updates': {'type': 'boolean', 'description': 'Help > Check for updates. Opening it asks GitHub for the latest release.'},
          'graph': {'type': ['integer', 'boolean'],
                    'description': 'Open the Material Graph editor on this material number; false closes it.'},
          'tab': {'type': 'string', 'enum': ['scene', 'material', 'environment', 'lighting', 'camera', 'image', 'studios', 'scripting']},
          'viewport': {'type': 'string', 'enum': ['wireframe', 'solid', 'material', 'rendered']}}),
    tool('tequila_preferences',
         "Tequila's own settings, as Edit > Preferences shows them, a page at a time. With no arguments, the pages there are. page is one of general, interface, viewport, navigation, "
         "keyboard (the shortcuts), import, folders, updates: each of its settings with its key, value, default and what it takes. search finds settings by words in their name or help. "
         "set and reset answer with what changed. They are the person's own and kept between sessions: change one only when asked to.",
         {'page': {'type': 'string', 'enum': ['general', 'interface', 'viewport', 'navigation', 'keyboard', 'import', 'folders', 'updates']},
          'search': {'type': 'string', 'description': 'Words that must all be in a setting\'s key, label or help: "zoom", "autosave".'},
          'set': {'type': 'object', 'description': 'Settings to change, by name: {"viewport.gizmo_size": 150, "navigation.invert_zoom": true}. All are checked before any is taken. A keyboard shortcut is set the same way: {"shortcut.view.screenshot": "F12"}, a list of two keys, or "" for none.'},
          'reset': {'type': ['string', 'array'], 'items': {'type': 'string'},
                    'description': '"all", a page (general, interface, viewport, navigation, keyboard, import, folders, updates), a setting, a shortcut, or a list of settings, put back to their defaults.'}}),
    tool('tequila_import', 'Load a model file into the scene: STEP or IGES (CAD, divided into parts with their colours), Rhino .3dm (layers, materials, blocks), Blender .blend (read by the Blender installed here, materials node for node), glTF/GLB, FBX, OBJ, STL, 3MF, PLY, USD (.usda, .usdc or .usdz) or IFC.',
         {'path': {'type': 'string'},
          'up': {'type': 'string', 'enum': ['file', 'y', 'z', 'x', '-y', '-z', '-x'],
                 'description': 'Which way up the file was drawn: z for a CAD model drawn Z up, which otherwise comes in lying on its back. Without it, the setting File > Up axis of imports decides.'},
          'lighting': {'type': 'string', 'enum': ['blender', 'tequila', 'ask'],
                       'description': "For a Blender file with lights of its own: blender lights the model with them, its world and its exposure, as it looks in Blender; tequila keeps the scene's lighting; ask asks the person in Tequila's window. Without it, the setting File > Lights in Blender files decides."}},
         ['path']),
    tool('tequila_save', 'Save the project, to its own file or to a new path.',
         {'path': {'type': 'string'}}),
    tool('tequila_graph_nodes',
         'The catalogue of nodes a material graph can hold: what each is for, with its inputs, outputs, settings and fields. '
         'With nothing, the first page of them all, a line each, and the families. search takes words that must all match '
         '("voronoi", "edge wear", "brushed"); family narrows to one (input, texture, pattern, group, color, converter, vector, shader, simple, layout). '
         'Three or fewer answers come whole; name asks for one whole. LOOK A NODE UP HERE before adding it: never guess a node\'s name, an input or a setting.',
         {'search': {'type': 'string'},
          'family': {'type': 'string', 'enum': ['input', 'texture', 'pattern', 'group', 'color', 'converter', 'vector', 'shader', 'simple', 'layout']},
          'name': {'type': 'string', 'description': "One node whole, by its name in the catalogue: voronoi_texture, gradient, pattern_leather, knurl."},
          'page': {'type': 'integer', 'description': 'Which page of a long list, from 1.'}}),
    tool('tequila_graph_read',
         "A material's node graph in brief: each node on a line with its id, what it is and where its output goes, and which of the material's parameters the graph drives. "
         'With node, that one node whole: its settings and fields, each input with its value or the wire into it, each output with where it goes. '
         'A few hundred characters where the whole material (tequila_material) is thousands.',
         {'material': {'type': 'integer', 'description': "The material's id, from tequila_materials or tequila_nodes."},
          'node': {'type': ['integer', 'string'], 'description': 'A node\'s id from the brief reading, or "surface".'},
          'page': {'type': 'integer', 'description': 'Which page of a graph with many nodes, from 1.'}},
         ['material']),
    tool('tequila_graph_edit',
         "Change a material's node graph one step at a time, each one undo step; a graph that would not work is refused and nothing changes. action is: "
         'add (node: a name from tequila_graph_nodes; settings, fields, inputs and label may come with it) - answers with the new node\'s id and its sockets; '
         'set (node: an id; settings {key: value}, fields {the node\'s own fields, such as a bump\'s depth or a ramp\'s gradient}, inputs {an input\'s name: a number, "#rrggbb", [x, y, z] or null for its default}, label, muted); '
         'connect (from: a node id, output: its name when the node has several, to: a node id or left out for the surface, input: the input\'s name, or on the surface a parameter\'s key such as base_color, specular_roughness, bump); '
         'disconnect (node: an id or left out for the surface, input); remove (node: an id). '
         'Every part that wears the material changes: material.unlink a part first to change it alone. Look with tequila_material_preview.',
         {'material': {'type': 'integer'},
          'action': {'type': 'string', 'enum': ['add', 'set', 'connect', 'disconnect', 'remove']},
          'node': {'type': ['integer', 'string'], 'description': 'add: the name of the node to add. set, remove, disconnect: the id of a node in the graph, or "surface".'},
          'settings': {'type': 'object', 'description': 'A general node\'s settings by key: {"feature": "f2"}.'},
          'fields': {'type': 'object', 'description': 'The node\'s own fields, as tequila_graph_read shows them: {"depth": 0.0003}, {"gradient": [{"at": 0, "color": "#101010"}, {"at": 1, "color": "#d0d0d0"}]}, {"procedural": {"frequency": 900}}, {"value": 0.35}.'},
          'inputs': {'type': 'object', 'description': 'Values for inputs nothing is wired into, by the input\'s name: {"Scale": 40, "Color1": "#202020"}.'},
          'label': {'type': 'string'},
          'muted': {'type': 'boolean'},
          'from': {'type': 'integer', 'description': 'connect: the node the wire leaves.'},
          'output': {'type': ['string', 'integer'], 'description': 'connect: which output of it, by name; the first when left out.'},
          'to': {'type': ['integer', 'string'], 'description': 'connect: the node the wire goes into; the surface when left out.'},
          'input': {'type': 'string', 'description': 'connect, disconnect: the input by its name, or a parameter\'s key on the surface.'}},
         ['material', 'action']),
    tool('tequila_uv_islands',
         "A part's UV layout, island by island, largest first: each island's number (what the uv.* commands take), its box on the layout, how much of the part it is, "
         'its area, how stretched it is, and where on the part it lies - centre and size in the world\'s metres and the way it faces. '
         'How to tell which island is the front panel before moving, packing, cutting or stitching one. Numbers change after a cut or an unwrap: read again.',
         {'node': {'type': 'integer', 'description': "The part's id, from tequila_nodes."},
          'set': {'type': 'integer', 'description': 'Which UV set, from 0 (default 0).'},
          'page': {'type': 'integer', 'description': 'Which page of a layout with many islands, from 1.'}},
         ['node']),
    tool('tequila_uv_cut',
         "Cut a seam in a part's UV layout where you mean it, as one undo step. path is two or more points ON THE PART, each [x, y, z] in the world's metres "
         '(tequila_nodes gives the part\'s centre and size; tequila_uv_islands each island\'s): the cut follows the part\'s own edges the shortest way from each point to the next - '
         'down the back of a bottle is [[0, top, -r], [0, bottom, -r]]. A seam follows the mesh\'s edges: across a flat CAD face that has none it is refused, and says the way round. Or sharper_than: every edge inside an island that is sharper than that many degrees (islands narrows it to some). '
         'The pieces are laid flat again in place; pack=true also packs the whole layout after. Answers with how many islands there were and are.',
         {'node': {'type': 'integer'},
          'set': {'type': 'integer'},
          'path': {'type': 'array', 'items': VECTOR, 'minItems': 2, 'description': 'Points on the part, in the world\'s metres.'},
          'sharper_than': {'type': 'number', 'description': 'Degrees: cut every edge inside an island that is sharper than this.'},
          'islands': {'type': 'array', 'items': {'type': 'integer'}, 'description': 'With sharper_than: only inside these islands.'},
          'pack': {'type': 'boolean', 'description': 'Pack the whole layout into the square afterwards.'},
          'reflatten': {'type': 'boolean', 'description': 'Lay the pieces the cut touches flat again (default true).'},
          'across': {'type': 'boolean', 'description': 'With path: carry a cut that stops just short of the island\'s edge on to it, so it divides the island (default true). false cuts between the points exactly, for a slit.'}},
         ['node']),
    tool('tequila_uv_stitch',
         "Join islands of a part's UV layout along the seams between them, as one undo step: two or more islands that meet on the part, by their numbers from tequila_uv_islands, "
         'or one island to shut a seam inside it. The smaller is moved onto the larger. Islands that do not touch on the part are refused.',
         {'node': {'type': 'integer'},
          'set': {'type': 'integer'},
          'islands': {'type': 'array', 'items': {'type': 'integer'}, 'minItems': 1}},
         ['node', 'islands']),
    tool('tequila_improve',
         'Improve model, for a model that came in coarse: the parts at and under a node made finer or shaded again, as one undo step. '
         'subdivide (1 to 3) makes four triangles of every one each time, the new points following the curve of the surface, for curves that show their facets; '
         'shade (degrees, 30 is usual) works the shading out again, smooth across edges gentler than that, for an STL or OBJ that looks faceted all over. '
         'With neither, how many triangles there are and how many each would make. A CAD model (STEP, IGES) is better divided again from its exact surfaces: page fixing of tequila_guide.',
         {'node': {'type': 'integer', 'description': 'A part, or a group: everything under it.'},
          'subdivide': {'type': 'integer', 'minimum': 1, 'maximum': 3},
          'shade': {'type': 'number', 'description': 'Degrees, 5 to 90: edges sharper than this stay sharp.'},
          'crease': {'type': 'number', 'description': 'With subdivide, for a mesh that had no shading: edges sharper than this stay sharp (default 30).'}},
         ['node']),
    tool('tequila_model_units',
         'Units of this model: put right a model that came in the wrong size because its file named no unit (an STL in millimetres is read as metres, a thousand times too large). '
         'unit is what its numbers really are: mm, cm, in, ft or m. The whole model the node belongs to is scaled about the middle of its base, so it stands where it stood; one undo step. '
         'With no unit, its size now and the size it would be in each unit: pick the one that reads as the real thing. read_as says what its numbers are read as now, when not metres.',
         {'node': {'type': 'integer', 'description': 'Any node of the model.'},
          'unit': {'type': 'string', 'enum': ['mm', 'cm', 'in', 'ft', 'm']},
          'read_as': {'type': 'string', 'enum': ['mm', 'cm', 'in', 'ft', 'm']}},
         ['node']),
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
    'tequila_request', 'tequila_import', 'tequila_preferences', 'tequila_material_save', 'tequila_studio_preset_save',
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
        # Briefly: every parameter of every material is hundreds of
        # thousands of characters on a real scene.
        materials = self.request('query', what='materials').get('materials', [])
        slim = [{'id': m.get('id'), 'name': m.get('name'), 'type': m.get('type'), 'used_by': m.get('used_by'),
                 'textures': len(m.get('textures') or [])} for m in materials]
        return text({'count': len(slim), 'materials': slim})

    def tool_material(self, id):
        for material in self.request('query', what='materials').get('materials', []):
            if material.get('id') == id:
                return text(material)
        raise TequilaError(f'there is no material {id}; tequila_materials lists them')

    def tool_library(self, search=None, category=None, limit=None):
        fields = {k: v for k, v in dict(search=search, category=category, limit=limit).items() if v is not None}
        return text(self.request('library', action='search', **fields))

    def tool_library_apply(self, name, nodes=None, unlink=None):
        fields = {k: v for k, v in dict(nodes=nodes, unlink=unlink).items() if v is not None}
        return text(self.request('library', action='apply', name=name, **fields))

    def tool_guide(self, topic=None):
        return text(self.request('guide', **({'topic': topic} if topic else {})))

    def tool_read(self, what=None, node=None, index=None):
        fields = {k: v for k, v in dict(what=what, node=node, index=index).items() if v is not None}
        return text(self.request('read', **fields))

    def tool_command(self, command=None):
        return text(self.request('help', **({'command': command} if command else {})))

    def tool_add_shape(self, shape, width=None, depth=None, height=None, radius=None, diameter=None):
        fields = {k: v for k, v in dict(width=width, diameter=diameter, depth=depth, height=height, radius=radius).items() if v is not None}
        return text(self.request('add_shape', shape=shape, **fields))

    def tool_query(self, what):
        return text(self.request('query', what=what))

    def tool_apply(self, command):
        self.request('apply', command=command)
        return text({'ok': True, 'applied': command.get('command')})

    def tool_material_guide(self, topic=None):
        # A part at a time: the whole guide is thirty thousand characters.
        return text(self.request('material', action='guide', topic=topic or 'contents'))

    def tool_graph_nodes(self, search=None, family=None, name=None, page=None):
        fields = {k: v for k, v in dict(search=search, family=family, name=name, page=page).items() if v is not None}
        return text(self.request('graph', action='catalogue', **fields))

    def tool_graph_read(self, material, node=None, page=None):
        fields = {k: v for k, v in dict(node=node, page=page).items() if v is not None}
        return text(self.request('graph', action='read', material=material, **fields))

    def tool_graph_edit(self, material, action, **fields):
        if action not in ('add', 'set', 'connect', 'disconnect', 'remove'):
            raise TequilaError(f'`{action}` is not an edit: add, set, connect, disconnect or remove')
        return text(self.request('graph', action=action, material=material, **{k: v for k, v in fields.items() if v is not None}))

    def tool_uv_islands(self, node, set=None, page=None):
        fields = {k: v for k, v in dict(set=set, page=page).items() if v is not None}
        return text(self.request('uv', action='islands', node=node, **fields))

    def tool_uv_cut(self, node, set=None, path=None, sharper_than=None, islands=None, pack=None, reflatten=None, across=None):
        fields = {k: v for k, v in dict(set=set, path=path, sharper_than=sharper_than, islands=islands, pack=pack, reflatten=reflatten, across=across).items() if v is not None}
        return text(self.request('uv', action='cut', node=node, **fields))

    def tool_uv_stitch(self, node, islands, set=None):
        fields = {'set': set} if set is not None else {}
        return text(self.request('uv', action='stitch', node=node, islands=islands, **fields))

    def tool_improve(self, node, subdivide=None, shade=None, crease=None):
        fields = {k: v for k, v in dict(subdivide=subdivide, shade=shade, crease=crease).items() if v is not None}
        return text(self.request('improve', node=node, **fields))

    def tool_model_units(self, node, unit=None, read_as=None):
        fields = {k: v for k, v in dict(unit=unit, read_as=read_as).items() if v is not None}
        return text(self.request('model_units', node=node, **fields))

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

    def tool_screenshot(self, viewport=True, settle=12, window=None):
        path = Path(tempfile.gettempdir()) / f'tequila-mcp-{time.time_ns()}.png'
        seconds = max(0.0, min(float(settle), 60.0))
        if window:
            # One of the windows of its own: the UV editor, the graph.
            self.request('screenshot', path=str(path), viewport=False, window=window)
            wait_for_png(path, 30)
            return image(path, f'Tequila\'s {window} window')
        self.request('screenshot', path=str(path), viewport=viewport, settle=seconds)
        wait_for_png(path, 30 + seconds)
        return image(path, picture_caption(viewport, self.request('query', what='ui')))

    def tool_render(self, width=640, height=400, samples=32, wireframe=False):
        path = Path(tempfile.gettempdir()) / f'tequila-mcp-render-{time.time_ns()}.png'
        fields = dict(path=str(path), width=width, height=height, samples=samples)
        if wireframe:
            fields['wireframe'] = True
        result = self.request('render', **fields)
        return image(path, json.dumps(result))

    def tool_queue(self, **job):
        return text(self.request('queue', **job))

    def tool_jobs(self):
        return text(self.request('query', what='jobs'))

    def tool_studio_presets(self):
        return text(self.request('studio_preset', action='list'))

    def tool_studio_preset_apply(self, preset, **options):
        return text(self.request('studio_preset', action='apply', preset=preset, **options))

    def tool_studio_preset_save(self, name, **details):
        return text(self.request('studio_preset', action='save', name=name, **details))

    def tool_studio_render_all(self):
        return text(self.request('studio_preset', action='render_all'))

    def tool_stop(self, job=None, all=None):
        fields = {k: v for k, v in dict(job=job, all=all).items() if v is not None}
        return text(self.request('stop', **fields))

    def tool_ui(self, **changes):
        return text(self.request('ui', **changes))

    def tool_preferences(self, set=None, reset=None, page=None, search=None):
        # Briefly: every setting at once is nineteen thousand characters,
        # and was the answer to changing one.
        fields = {k: v for k, v in dict(set=set, reset=reset, page=page, search=search).items() if v is not None}
        return text(self.request('preferences', brief=True, **fields))

    def tool_import(self, path, up=None, lighting=None):
        fields = {k: v for k, v in (('up', up), ('lighting', lighting)) if v}
        return text(self.request('import', path=str(Path(path).resolve()), **fields))

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

def wait_for_png(path, timeout):
    """Wait until a PNG the application is writing has its end chunk."""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            if path.read_bytes().endswith(b'IEND\xaeB`\x82'):
                return
        except OSError:
            pass
        time.sleep(0.05)
    raise TequilaError('the screenshot did not arrive')


def picture_caption(viewport, ui):
    """What a screenshot shows and whether its picture had finished, for
    whoever is about to judge a look by it."""
    what = 'Tequila viewport' if viewport else 'Tequila window'
    mode = ui.get('viewport', '')
    if mode != 'rendered':
        return f'{what}, {mode} view (not ray traced: switch the viewport to rendered to judge materials and light).' if mode else what
    samples = int(ui.get('picture_samples', 0))
    if ui.get('picture_settled'):
        return f'{what}, rendered, {samples} samples: finished enough to judge.'
    return (f'{what}, rendered, only {samples} samples: NOT finished - it is still dark and grainy from redrawing. '
            'Do not judge the look from this; take another screenshot.')


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
