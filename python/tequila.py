"""Drive Tequila from Python. Standard-library only; no embedded interpreter.

Two ways in, one API: `open()` starts a headless Tequila of the script's own
(`tequila --serve`), and `connect()` (or `Client()`) drives the application
a person has open. Every edit goes through the same command bus as a click,
so each is one undo step; `commands` holds every command in the catalog
(`docs/scripting-api.md`).

    import tequila
    with tequila.open('kettle.tequila') as fs:
        paint = fs.materials['Paint Base']
        paint.base_color = [0.72, 0.05, 0.06]
        for part in fs.find(metadata={'Material': 'ABS'}):
            part.assign(paint)
        fs.render('red.png', width=1200, height=800, samples=256)
        fs.save('kettle-red.tequila')

    from tequila import Client
    with Client() as fs:
        context = fs.context()
        print(context['nodes'])

        # Many edits as one undo step; if any fails, none of them stay.
        with fs.batch() as b:
            b.apply('material.set_type', material=2, type='velvet')
            b.apply('material.set_parameter', material=2, parameter='base_color', value=[0.1, 0.1, 0.12])
            b.look(view='front')

        fs.screenshot('check.png', viewport=True)   # just the picture
"""
import json
import os
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path


class TequilaError(RuntimeError):
    pass


class Client:
    """A sequential connection to one explicitly discovered app session.

    Mutations are never automatically retried: after a timeout their outcome
    may be unknown. Reconnect, inspect the scene, then decide what to do.
    """
    def __init__(self, discovery=None, *, timeout=30, expected_pid=None):
        path = Path(discovery) if discovery else Path.home() / '.tequila' / 'channel.json'
        info, self._socket = _connect(path, timeout)
        if expected_pid is not None and info['pid'] != expected_pid:
            self._socket.close()
            raise TequilaError('Discovery points to a different Tequila process')
        self.pid = info['pid']
        self._token = info['token']
        self._next_id = 0
        self._stream = self._socket.makefile('rwb')
        try:
            greeting = self.request('ping')
            if greeting.get('protocol') != 1 or greeting.get('pid') != self.pid:
                raise TequilaError('Unsupported protocol or stale discovery file')
        except Exception:
            self.close()
            raise

    def close(self):
        self._stream.close()
        self._socket.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()

    def request(self, op, **fields):
        self._next_id += 1
        request = dict(fields, id=self._next_id, token=self._token, op=op)
        encoded = json.dumps(request, allow_nan=False).encode('utf-8') + b'\n'
        try:
            self._stream.write(encoded)
            self._stream.flush()
            line = self._stream.readline(64 * 1024 * 1024 + 1)
            if not line or len(line) > 64 * 1024 * 1024 or not line.endswith(b'\n'):
                raise TequilaError('Incomplete or oversized reply; outcome may be unknown')
            reply = json.loads(line)
            if reply.get('id') != self._next_id:
                raise TequilaError('Reply ID mismatch; outcome may be unknown')
        except (OSError, ValueError, TequilaError) as error:
            self.close()
            raise TequilaError(f'Connection ended; do not retry edits blindly: {error}') from error
        if not reply.get('ok'):
            raise TequilaError(reply.get('error', 'Request failed'))
        return reply.get('result')

    def query(self, what):
        return self.request('query', what=what)

    def context(self):
        return self.query('context')

    def api(self):
        return self.query('api')

    def apply(self, command, **fields):
        """Apply one journal command; one independently undoable edit."""
        return self.request('apply', command=dict(fields, command=command))

    @property
    def commands(self):
        """Every command in the catalog, by group:
        `fs.commands.scene.rename(node=12, name='Lid')`."""
        from tequila_commands import Commands
        return Commands(self)

    @property
    def materials(self):
        """The scene's materials, by name or id: `fs.materials['Paint Base']`."""
        return Materials(self, self.query('materials')['materials'])

    def nodes(self):
        """Every node: parts, groups and the rest, as `Part`s."""
        return [Part(self, entry) for entry in self.query('nodes')['nodes']]

    def parts(self):
        return [node for node in self.nodes() if node.kind == 'part']

    def find(self, *, name=None, metadata=None, kind='part'):
        """Nodes whose name contains `name` (any case) and whose metadata has
        every field of `metadata`; parts unless `kind` says otherwise (None
        for any)."""
        found = []
        for node in self.nodes():
            if kind is not None and node.kind != kind:
                continue
            if name is not None and name.lower() not in node.name.lower():
                continue
            if metadata and any(node.metadata.get(key) != str(value) for key, value in metadata.items()):
                continue
            found.append(node)
        return found

    def export(self, format, path, **options):
        """Write the scene out: format 'glb', 'usd', 'web' (a folder; live=,
        photos=) or 'spin' (a folder; columns=, rows=, tilt=, transparent=)."""
        return self.request('export', format=format, path=str(Path(path).resolve()), **options)

    def select(self, nodes):
        return self.request('select', nodes=list(nodes))

    def frame(self, *, all=True):
        return self.request('frame', all=all)

    def set_material_parameter(self, material, parameter, value):
        return self.apply('material.set_parameter', material=material, parameter=parameter, value=value)

    def assign_material(self, nodes, material):
        return self.apply('material.assign', nodes=list(nodes), material=material)

    def set_texture(self, material, parameter, path, *, role='color'):
        fields = dict(material=material, param=parameter)
        if path is not None:
            fields.update(path=str(Path(path).resolve()), role=role)
        return self.apply('material.set_texture', **fields)

    def import_model(self, path, *, parent=None):
        fields = {'path': str(Path(path).resolve())}
        if parent is not None:
            fields['parent'] = parent
        return self.request('import', **fields)

    def reimport(self, node=None, path=None):
        """Update an imported model from its file (or from `path`), keeping
        its materials, labels and names: the model `node` belongs to, or the
        selection's. Returns what matched, what was added and what was hidden."""
        fields = {}
        if node is not None:
            fields['node'] = node
        if path is not None:
            fields['path'] = str(Path(path).resolve())
        return self.request('reimport', **fields)

    def texture_set(self, path, nodes=None):
        """Make materials from a PBR texture pack (a folder, a ZIP, or one of
        its maps) and put the first on `nodes`, or on the selection."""
        fields = {'path': str(Path(path).resolve())}
        if nodes is not None:
            fields['nodes'] = list(nodes)
        return self.request('texture_set', **fields)

    def save(self, path=None):
        return self.request('save', **({'path': str(Path(path).resolve())} if path else {}))

    @property
    def recipes(self):
        """Materials of your own, from a short JSON recipe:
        `fs.recipes.create({"type": "plastic", "patterns": [...]}, part)`.
        `fs.recipes.guide()` describes the format, every pattern, and tested
        examples to start from."""
        return Recipes(self)

    @property
    def library(self):
        """The material library, shipped presets and the user's own:
        `fs.library.apply('Aluminium Spun', part)`, `fs.library.save(material,
        'My Blue', category='Plastic')`, `fs.library.import_('shared.fsmat')`."""
        return Library(self)

    @property
    def studios(self):
        """Studio presets - complete studios for a kind of product, by family
        and size: `fs.studios.apply('product-studio-medium')`,
        `fs.studios.save('My studio', family='Bottles', size='Tall')`,
        `fs.studios.render_all()` for every view."""
        return StudioPresets(self)

    def screenshot(self, path, *, viewport=False, window=None, timeout=30):
        """Wait for a new PNG of the window, or of just the viewport, or of
        another window: window='render', 'graph', 'shortcuts' or 'connect'.
        Refuse existing paths to avoid stale-image success."""
        path = Path(path).resolve()
        if path.exists():
            raise FileExistsError(path)
        extra = {'window': window} if window else {}
        result = self.request('screenshot', path=str(path), viewport=viewport, **extra)
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if path.exists():
                try:
                    with path.open('rb') as file:
                        file.seek(-12, 2)
                        if file.read() == b'\x00\x00\x00\x00IEND\xaeB`\x82':
                            return result['path']
                except (OSError, ValueError):
                    pass
            time.sleep(0.05)
        raise TimeoutError('Screenshot has not completed; inspect app activity for errors')

    def render(self, path, *, width=960, height=600, samples=64, timeout=3600):
        """Render to a file and return when it is written.

        Synchronous, and a final-quality frame takes minutes, so the wait is
        this call's own `timeout` (seconds, default an hour) rather than the
        connection's, which is sized for edits and queries. The connection's
        timeout is restored afterwards.
        """
        previous = self._socket.gettimeout()
        self._socket.settimeout(timeout)
        try:
            return self.request('render', path=str(Path(path).resolve()), width=width, height=height, samples=samples)
        finally:
            try:
                self._socket.settimeout(previous)
            except OSError:
                pass

    def undo(self):
        """Take back one step: a single edit, or a whole batch."""
        return self.request('undo')

    def redo(self):
        return self.request('redo')

    def history(self):
        """What undo and redo would do, and whether there is unsaved work."""
        return self.query('history')

    def batch(self, *, atomic=True):
        """Collect operations and send them as one undo step on leaving the block.

        With atomic=True (the default), a failure part-way takes back everything
        the batch did. Nothing is sent if the block itself raises.
        """
        return Batch(self, atomic)

    def look(self, *, view=None, at=None, yaw=None, pitch=None, distance=None, frm=None, focal_length=None):
        """Place the camera: a standard view, orbit angles in degrees around `at`,
        or an exact eye position `frm` looking at `at`. Undoable."""
        return self.request('look', **_look_fields(view, at, yaw, pitch, distance, frm, focal_length))

    def ui(self, **changes):
        """Open or close panels: render_window, console, inspector, library,
        full_viewport, help, frame (the render's shape in the viewport)
        (bools), tab='material', graph=<material> or
        False, viewport='wireframe'|'solid'|'material'|'rendered',
        mode='light'|'dark'|'auto', theme='graphite'|'ocean'|'orchid'|'ember'|
        'meadow'|'rose'."""
        return self.request('ui', **changes)

    def queue(self, **job):
        """Render in the background with the document's settings. Optional:
        name, width, height, samples, folder, filename, denoise, camera, and
        engine ('tequila', 'cycles-cpu' or 'cycles-gpu'). With
        configurations=True, one job per valid combination of the
        configurator, named by its choices ({configuration} in filename).
        studio='Right' (a name or index) renders one view as that Studio
        says, leaving the scene as it was; studios=True renders every
        view, one job each ({view} in filename). Returns at once; poll
        jobs() or use wait_for_job()."""
        return self.request('queue', **job)

    def jobs(self):
        return self.query('jobs')['jobs']

    def wait_for_job(self, job, *, timeout=3600, poll=1.0):
        """Block until a queued job is done or failed; return its entry."""
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for entry in self.jobs():
                if entry['id'] == job and entry['state'] in ('done', 'failed'):
                    if entry['state'] == 'failed':
                        raise TequilaError(entry.get('message', 'The render failed'))
                    return entry
            time.sleep(poll)
        raise TimeoutError(f'Job {job} has not finished')

    def stop(self):
        """Stop the running background render; it still writes its image."""
        return self.request('stop')

    def new_project(self, *, save=False, discard=False):
        """Start a new project. Refused while there is unsaved work unless
        `save` or `discard` says what to do with it."""
        return self.request('new', save=save, discard=discard)

    def open_project(self, path, *, save=False, discard=False):
        """Open the project at `path` in place of this one; unsaved work
        settles as for new_project()."""
        return self.request('open', path=str(Path(path).resolve()), save=save, discard=discard)

    def quit(self, *, save=False, discard=False):
        """Close Tequila. Refused while there is unsaved work unless save or
        discard says what to do with it."""
        result = self.request('quit', save=save, discard=discard)
        self.close()
        return result


def _connect(path, timeout):
    """Connect to the instance the discovery file names.

    If that instance is gone (it crashed, so it could not hand the file on),
    try the other running instances' own records, newest first.
    """
    candidates = []
    if path.exists():
        candidates.append(path)
    others = path.with_name('channels')
    if others.is_dir():
        candidates += sorted(others.glob('*.json'), key=lambda p: p.stat().st_mtime, reverse=True)
    if not candidates:
        raise TequilaError(f'Tequila is not running (no {path})')
    for candidate in candidates:
        try:
            info = json.loads(candidate.read_text(encoding='utf-8'))
            return info, socket.create_connection(('127.0.0.1', info['port']), timeout)
        except (OSError, ValueError, KeyError):
            continue
    raise TequilaError('No running Tequila answered; is it open?')


def _look_fields(view, at, yaw, pitch, distance, frm, focal_length):
    fields = dict(view=view, at=at, yaw=yaw, pitch=pitch, distance=distance, focal_length=focal_length)
    fields['from'] = frm
    return {k: (list(v) if isinstance(v, tuple) else v) for k, v in fields.items() if v is not None}


class Session(Client):
    """A headless Tequila of this script's own: `tequila --serve` in its
    own process, holding a project (or a model to start from, or an empty
    scene). It answers the same protocol as the application, without what
    needs a window. Closing it ends the process, and unsaved changes with
    it: save first, or close(save=True)."""

    def __init__(self, path=None, *, cli=None, timeout=60, args=()):
        executable = _find_cli(cli)
        self._closed = False
        self._quit = False
        self._folder = tempfile.TemporaryDirectory(prefix='tequila-')
        folder = Path(self._folder.name)
        discovery = folder / 'channel.json'
        command = [str(executable), '--serve', '--attached', '--discovery', str(discovery)]
        if path is not None:
            command += ['-i', str(Path(path).resolve())]
        command += [str(a) for a in args]
        self._log_path = folder / 'serve.log'
        self._log = self._log_path.open('wb')
        self._process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=self._log)
        deadline = time.monotonic() + timeout
        while True:
            if self._process.poll() is not None:
                message = self._finish()
                raise TequilaError(f'tequila --serve stopped: {message or "it gave no reason"}')
            try:
                if json.loads(discovery.read_text(encoding='utf-8')).get('port'):
                    break
            except (OSError, ValueError):
                pass
            if time.monotonic() > deadline:
                self._finish()
                raise TimeoutError('tequila --serve did not start')
            time.sleep(0.05)
        try:
            super().__init__(discovery, expected_pid=self._process.pid)
        except Exception:
            self._finish()
            raise

    def _finish(self):
        """End the process however it stands; what it said, for errors."""
        try:
            self._process.stdin.close()
        except OSError:
            pass
        try:
            self._process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self._process.kill()
            self._process.wait()
        self._log.close()
        message = self._log_path.read_text(encoding='utf-8', errors='replace').strip()
        self._folder.cleanup()
        return message

    def quit(self, *, save=False, discard=False):
        result = self.request('quit', save=save, discard=discard)
        self._quit = True
        self.close()
        return result

    def close(self, *, save=False):
        """End the session. Unsaved changes are dropped unless save=True."""
        if self._closed:
            return
        self._closed = True
        try:
            if not self._quit and self._process.poll() is None:
                try:
                    self.request('quit', save=save, discard=not save)
                except TequilaError:
                    pass
        finally:
            super().close()
            self._finish()


def open(path=None, *, cli=None, timeout=60, args=()):
    """A headless session on a .tequila project, a model to import, or an
    empty scene (`path=None`). Use it in a `with` block, or close() it."""
    return Session(path, cli=cli, timeout=timeout, args=args)


def connect(discovery=None, *, timeout=30):
    """The Tequila application a person has open."""
    return Client(discovery, timeout=timeout)


def _find_cli(explicit=None):
    """The command-line renderer: given, in TEQUILA_CLI, beside this folder
    in a distribution (tequila-cli), or in a development build's target."""
    windows = os.name == 'nt'
    distributed = 'tequila-cli.exe' if windows else 'tequila-cli'
    built = 'tequila.exe' if windows else 'tequila'
    here = Path(__file__).resolve().parent
    candidates = []
    if explicit:
        candidates.append(Path(explicit))
    if os.environ.get('TEQUILA_CLI'):
        candidates.append(Path(os.environ['TEQUILA_CLI']))
    candidates.append(here.parent / distributed)
    # A Mac app bundle: python in Contents/Resources, the renderer in
    # Contents/MacOS.
    candidates.append(here.parent.parent / 'MacOS' / distributed)
    builds = [here.parent / 'target' / profile / built for profile in ('release', 'debug')]
    candidates += sorted((b for b in builds if b.is_file()), key=lambda b: b.stat().st_mtime, reverse=True)
    on_path = shutil.which('tequila-cli')
    if on_path:
        candidates.append(Path(on_path))
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    raise TequilaError('The command-line renderer was not found (looked for ' + ', '.join(str(c) for c in candidates or [here.parent / distributed]) + '); set TEQUILA_CLI to its path')


def srgb(colour):
    """A colour as a swatch shows it - '#C23B32', '#c23', or (r, g, b) in
    0-255 or 0-1 - in the space material colours are set in: linear ACEScg,
    Tequila's rendering space. `material.base_color = tequila.srgb('#2D5DA8')`."""
    if isinstance(colour, str):
        text = colour.strip().lstrip('#')
        if len(text) == 3:
            text = ''.join(c * 2 for c in text)
        if len(text) != 6:
            raise ValueError(f'not a colour: {colour!r}')
        encoded = [int(text[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    else:
        values = [float(c) for c in colour]
        if len(values) != 3:
            raise ValueError(f'not a colour: {colour!r}')
        encoded = [c / 255 for c in values] if any(c > 1 for c in values) else values

    def linear(c):
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
    r, g, b = (linear(c) for c in encoded)
    # Rec.709 primaries to ACEScg (AP1), as the application converts a
    # picked colour.
    return [
        round(0.6130974 * r + 0.3395234 * g + 0.0473792 * b, 6),
        round(0.0701936 * r + 0.9163535 * g + 0.0134529 * b, 6),
        round(0.0206158 * r + 0.1095696 * g + 0.8698146 * b, 6),
    ]


def _plain(value):
    return list(value) if isinstance(value, tuple) else value


def _ids(things):
    """Parts, materials or ids, singly or in lists, as ids."""
    out = []
    for thing in things:
        for item in (thing if isinstance(thing, (list, tuple)) else [thing]):
            out.append(item.id if hasattr(item, 'id') else int(item))
    return out


class Library:
    """The material library. Presets are named as the browser names them;
    a name in both libraries means the user's own."""

    def __init__(self, client):
        self._client = client

    def presets(self, *, category=None, mine=False):
        """Every preset: name, category, tags, type, source and file."""
        found = self._client.query('library')['presets']
        if category is not None:
            found = [p for p in found if p['category'].lower() == category.lower()]
        if mine:
            found = [p for p in found if p['source'] == 'user']
        return found

    def folder(self):
        """Where the user's own presets are kept."""
        return self._client.query('library')['user_folder']

    def apply(self, name, *parts, unlink=False):
        """Put the preset `name` on parts (or ids), the selection when none
        are given, as one undo step - as a drop from the browser does."""
        fields = {'action': 'apply', 'name': name, 'unlink': unlink}
        nodes = _ids(parts)
        if nodes:
            fields['nodes'] = nodes
        return self._client.request('library', **fields)

    def save(self, material, name=None, *, category=None, tags=()):
        """A material in the scene into the user's library, pictures and
        all."""
        fields = {'action': 'save', 'material': _ids([material])[0], 'tags': list(tags)}
        if name is not None:
            fields['name'] = name
        if category is not None:
            fields['category'] = category
        return self._client.request('library', **fields)

    def import_(self, path, *, name=None, category=None, tags=(), size=None):
        """A material package (.fsmat), a preset's .json, a KeyShot material
        package (.kmp; the reply's `notes` say what did not come across), or
        a texture pack (folder, ZIP or one of its maps; `size` metres a
        repeat) into the user's library."""
        fields = {'action': 'import', 'path': str(Path(path).resolve()), 'tags': list(tags)}
        for key, value in (('name', name), ('category', category), ('size', size)):
            if value is not None:
                fields[key] = value
        return self._client.request('library', **fields)

    def export(self, name, path):
        """The preset `name` as a material package, to share."""
        return self._client.request('library', action='export', name=name, path=str(Path(path).resolve()))

    def delete(self, name):
        """Remove one of the user's own presets. The shipped ones never."""
        return self._client.request('library', action='delete', name=name)


class StudioPresets:
    """Studio presets (docs/studio-presets.md): lights, cameras and views,
    environment and floor, look and render settings, applied to the product
    in one undo step and framed to its size."""

    def __init__(self, client):
        self._client = client

    def list(self):
        """Every preset installed: id, name, family, size, views, and
        `suggested` on the size in each family nearest the product."""
        return self._client.request('studio_preset', action='list')['presets']

    def suggested(self, family=None):
        """The preset framed nearest the product - in `family`, or in the
        first family that has one; None when there is no product."""
        for preset in self.list():
            if preset['suggested'] and (family is None or preset['family'].lower() == family.lower()):
                return preset
        return None

    def apply(self, id, *, front=None, scale=None, keep_lights=False, keep_cameras=False,
              keep_placement=False, dress=False):
        """Apply the preset `id` as one undo step. `front` is the product's
        side that faces the Main camera ('+z', '-z', '+x', '-x'); `scale` what
        follows its size ('cameras', 'cameras_and_lights', 'none'); `dress`
        puts the preset's materials on parts by their names."""
        fields = {'action': 'apply', 'id': id, 'keep_lights': keep_lights, 'keep_cameras': keep_cameras,
                  'keep_placement': keep_placement, 'dress': dress}
        for key, value in (('front', front), ('scale', scale)):
            if value is not None:
                fields[key] = value
        return self._client.request('studio_preset', **fields)

    def save(self, name, *, family='', size='', description='', materials=False):
        """The scene's studio - everything but the product - as a preset of
        your own; `materials` keeps the project's materials with it."""
        return self._client.request('studio_preset', action='save', name=name, family=family, size=size,
                                    description=description, materials=materials)

    def install(self, path):
        """A studio preset package (.tqstudio), or a preset's folder or
        .studio.json; its materials go into the material library too."""
        return self._client.request('studio_preset', action='install', path=str(Path(path).resolve()))

    def export(self, id, path):
        """The preset `id` as a package, to share."""
        return self._client.request('studio_preset', action='export', id=id, path=str(Path(path).resolve()))

    def delete(self, id):
        """Remove one of your own presets. The shipped ones never."""
        return self._client.request('studio_preset', action='delete', id=id)

    def render_all(self):
        """Queue every view (Studio), one job each, named by the output
        template ({product}_{view}_{date} in a preset's). Returns the job ids;
        wait with wait_for_job()."""
        return self._client.request('studio_preset', action='render_all')['jobs']


class Recipes:
    """Procedural materials from recipes: a type, the parameters it changes,
    and patterns driving parameters through colour ramps and ranges, layered
    with masks. Colours may be hex sRGB strings ("#c0392b")."""

    def __init__(self, client):
        self._client = client

    def guide(self):
        """The recipe format, every pattern with what it looks like and its
        settings, the material types, rules of thumb and tested examples."""
        return self._client.request('material', action='guide')

    def examples(self):
        """The guide's examples: [{'idea': ..., 'recipe': {...}}, ...]."""
        return self.guide()['examples']

    def preview(self, recipe=None, *, material=None, path=None, radius=None, size=None, shape=None):
        """A picture of `recipe` (or the scene's material `material`) on a
        ball, `radius` metres (5 cm by default); the scene is not touched.
        Returns where the PNG is."""
        fields = {'action': 'preview'}
        if recipe is not None:
            fields['recipe'] = recipe
        if material is not None:
            fields['material'] = _ids([material])[0]
        if path is not None:
            fields['path'] = str(Path(path).resolve())
        for key, value in (('radius', radius), ('size', size), ('shape', shape)):
            if value is not None:
                fields[key] = value
        return self._client.request('material', **fields)['path']

    def create(self, recipe, *parts):
        """A new material from `recipe`, on parts (or ids) - the selection
        when none are given - as one undo step. Returns its id."""
        fields = {'action': 'create', 'recipe': recipe}
        nodes = _ids(parts)
        if nodes:
            fields['nodes'] = nodes
        return self._client.request('material', **fields)['material']

    def update(self, material, recipe):
        """Remake the material `material` from `recipe`, where it is."""
        return self._client.request('material', action='update', material=_ids([material])[0], recipe=recipe)

    def read(self, material):
        """Any material as a recipe, to change and update."""
        return self._client.request('material', action='recipe', material=_ids([material])[0])['recipe']

    def save(self, material, name=None, *, category=None, tags=()):
        """Keep the material in the user's library."""
        fields = {'action': 'save', 'material': _ids([material])[0], 'tags': list(tags)}
        if name is not None:
            fields['name'] = name
        if category is not None:
            fields['category'] = category
        return self._client.request('material', **fields)


class Materials:
    """The scene's materials, by name or by id."""

    def __init__(self, client, entries):
        self._items = [Material(client, entry) for entry in entries]

    def __getitem__(self, key):
        for material in self._items:
            if material.name == key or material.id == key:
                return material
        raise KeyError(key)

    def __iter__(self):
        return iter(self._items)

    def __len__(self):
        return len(self._items)

    def __contains__(self, key):
        return any(m.name == key or m.id == key for m in self._items)


class Material:
    """A material. Setting an attribute sets that parameter, as one undo
    step: `paint.base_color = [0.72, 0.05, 0.06]`; reading one gives its
    value as the scene had it when this was read."""

    _OWN = ('id', 'name', 'type', 'definition')

    def __init__(self, client, entry):
        object.__setattr__(self, '_client', client)
        object.__setattr__(self, 'id', entry['id'])
        object.__setattr__(self, 'name', entry['name'])
        object.__setattr__(self, 'type', entry.get('type'))
        object.__setattr__(self, 'definition', entry.get('definition', {}))

    def __getattr__(self, key):
        definition = object.__getattribute__(self, 'definition')
        if key in definition:
            return definition[key]
        raise AttributeError(key)

    def __setattr__(self, key, value):
        if key in self._OWN:
            raise AttributeError(f'{key} is not a parameter; use rename() or set_type()')
        self.set(key, value)

    def __repr__(self):
        return f'<Material {self.id} {self.name!r} ({self.type})>'

    def set(self, parameter, value):
        return self._client.apply('material.set_parameter', material=self.id, parameter=parameter, value=_plain(value))

    def set_type(self, kind):
        return self._client.apply('material.set_type', material=self.id, type=kind)

    def rename(self, name):
        object.__setattr__(self, 'name', name)
        return self._client.apply('material.rename', material=self.id, name=name)

    def assign(self, *parts):
        """Put this material on parts (or their ids), linked."""
        nodes = [p.id if isinstance(p, Part) else int(p) for group in parts for p in (group if isinstance(group, (list, tuple)) else [group])]
        return self._client.apply('material.assign', material=self.id, nodes=nodes)


class Part:
    """A node of the scene, as it stood when read."""

    def __init__(self, client, entry):
        self._client = client
        self.id = entry['id']
        self.name = entry['name']
        self.kind = entry.get('kind')
        self.parent = entry.get('parent')
        self.material = entry.get('material')
        self.metadata = entry.get('metadata', {})
        self.bounds = entry.get('bounds')
        self.visible = entry.get('visible', True)
        # A part's shape: its mesh's key (copies share one) and its size.
        self.mesh = entry.get('mesh')
        self.triangles = entry.get('triangles')

    def __repr__(self):
        return f'<Part {self.id} {self.name!r}>'

    def assign(self, material):
        key = material.id if isinstance(material, Material) else material
        return self._client.apply('material.assign', material=key, nodes=[self.id])

    def rename(self, name):
        self.name = name
        return self._client.apply('scene.rename', node=self.id, name=name)

    def show(self, visible=True):
        self.visible = visible
        return self._client.apply('scene.set_visibility', states=[{'node': self.id, 'value': visible}])

    def hide(self):
        return self.show(False)

    def set_metadata(self, key, value):
        """One metadata field; None removes it."""
        return self._client.apply('scene.set_metadata', node=self.id, key=key, value=None if value is None else str(value))


class Batch:
    """Operations gathered in a `with` block and sent as one request."""

    def __init__(self, client, atomic):
        self._client = client
        self._atomic = atomic
        self.operations = []
        self.result = None

    def add(self, op, **fields):
        self.operations.append(dict(fields, op=op))
        return self

    def apply(self, command, **fields):
        return self.add('apply', command=dict(fields, command=command))

    @property
    def commands(self):
        """The catalog, gathered into this batch rather than sent."""
        from tequila_commands import Commands
        return Commands(self)

    def select(self, nodes):
        return self.add('select', nodes=list(nodes))

    def frame(self, *, all=True):
        return self.add('frame', all=all)

    def look(self, *, view=None, at=None, yaw=None, pitch=None, distance=None, frm=None, focal_length=None):
        return self.add('look', **_look_fields(view, at, yaw, pitch, distance, frm, focal_length))

    def ui(self, **changes):
        return self.add('ui', **changes)

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        if exc_type is None and self.operations:
            self.result = self._client.request('batch', ops=self.operations, atomic=self._atomic)
        return False
