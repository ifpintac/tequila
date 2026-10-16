"""Tequila plugins: the API a plugin's code uses, and the host that runs it.

A plugin's `main` file registers what it answers:

    import tequila_plugin as plugin

    @plugin.command('rename')
    def rename(fs, values):            # values: the panel's fields
        ...
        plugin.status('Renamed 4 parts')

    @plugin.on('selection')
    def selection(fs, data):           # data: {'nodes': [...]}
        plugin.set_panel('main', selected=f"{len(data['nodes'])} selected")

`fs` is a `tequila.Client` connected to the window that started the
plugin - the whole scripting API (docs/scripting-api.md). print() goes to
the plugin's log in the Plugins window.

Tequila starts this file as `python tequila_plugin.py <plugin folder>`,
with the channel to connect back to in TEQUILA_DISCOVERY and the window's
process in TEQUILA_PID. It sends commands and events as JSON lines on
stdin; what the plugin says goes back as JSON lines on stdout, which is
why print() is taken over.
"""
import importlib.util
import json
import os
import sys
import threading
import traceback
from pathlib import Path

_commands = {}
_events = {}
_lock = threading.Lock()
_out = sys.stdout


def command(name):
    """Register a function to run when the command `name` is chosen - from
    the Plugins menu or a panel's button. It gets (fs, values)."""
    def register(function):
        _commands[name] = function
        return function
    return register


def on(event):
    """Register a function for an event the manifest asks for. It gets
    (fs, data):

    - `selection`: the selection changed; data {'nodes': [ids]}.
    - `saved`: the project was saved; data {'path': ...}.
    - `render_finished`: a render was written; data {'path': ...}.
    - `imported`: a model came in; data {'path': ..., 'nodes': [every new
      node], 'parts': [the new parts with geometry]}.
    - `opened`: a project was opened; data {'path': ...}.
    - `panel_opened`: one of this plugin's panels opened; data {'panel':
      id}. Fill it in with set_panel()."""
    def register(function):
        _events.setdefault(event, []).append(function)
        return function
    return register


def status(text):
    """A message for the person, shown as a notice."""
    _send({'type': 'status', 'text': str(text)})


def set_panel(panel, **values):
    """Change fields of one of the plugin's open panels: a label's text, a
    number, a choice."""
    _send({'type': 'panel', 'panel': panel, 'values': values})


def log(text):
    """A line for the plugin's log only."""
    _send({'type': 'log', 'text': str(text)})


_id = None


def _settings_folder():
    # Tequila's folder of the person's settings, as the application has
    # it (tequila_format::library::settings_directory).
    if sys.platform == 'darwin':
        return Path.home() / 'Library' / 'Application Support' / 'Tequila'
    base = os.environ.get('APPDATA') or os.environ.get('XDG_CONFIG_HOME') or (Path.home() / '.config')
    return Path(base) / 'Tequila'


def data_folder():
    """A folder of this plugin's own for settings and caches - the person's,
    kept when the plugin is updated or reinstalled. Made on first use."""
    folder = os.environ.get('TEQUILA_PLUGIN_DATA')
    if folder:
        folder = Path(folder)
    else:
        folder = _settings_folder() / 'plugin-data' / (_id or 'plugin')
    folder.mkdir(parents=True, exist_ok=True)
    return folder


def load_settings(defaults=None):
    """What save_settings() kept, over `defaults`: a dict."""
    settings = dict(defaults or {})
    try:
        settings.update(json.loads((data_folder() / 'settings.json').read_text(encoding='utf-8')))
    except (OSError, ValueError):
        pass
    return settings


def save_settings(settings):
    """Keep a dict of settings (anything JSON can hold) for next time."""
    path = data_folder() / 'settings.json'
    temporary = path.with_suffix('.tmp')
    temporary.write_text(json.dumps(settings, indent=2), encoding='utf-8')
    os.replace(temporary, path)


def _send(message):
    line = json.dumps(message, allow_nan=False)
    with _lock:
        _out.write(line + '\n')
        _out.flush()


class _ToLog:
    """print() inside a plugin: to its log, not into the conversation."""

    def __init__(self):
        self._pending = ''

    def write(self, text):
        self._pending += text
        while '\n' in self._pending:
            line, self._pending = self._pending.split('\n', 1)
            if line:
                log(line)
        return len(text)

    def flush(self):
        if self._pending:
            log(self._pending)
            self._pending = ''


def _connect():
    discovery = os.environ.get('TEQUILA_DISCOVERY')
    if not discovery:
        return None
    import tequila
    pid = os.environ.get('TEQUILA_PID')
    return tequila.Client(discovery, expected_pid=int(pid) if pid else None)


def _load(folder):
    global _id
    manifest = json.loads((folder / 'plugin.json').read_text(encoding='utf-8'))
    _id = manifest.get('id')
    main = folder / manifest['main']
    sys.path.insert(0, str(folder))
    spec = importlib.util.spec_from_file_location('tequila_plugin_main', main)
    module = importlib.util.module_from_spec(spec)
    sys.modules['tequila_plugin_main'] = module
    spec.loader.exec_module(module)
    return manifest


def _serve(folder):
    sys.stdout = _ToLog()
    try:
        _load(folder)
        fs = _connect()
    except Exception:
        _send({'type': 'error', 'text': traceback.format_exc()})
        return 1
    _send({'type': 'ready', 'commands': sorted(_commands), 'events': sorted(_events)})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except ValueError:
            log(f'not understood: {line[:200]}')
            continue
        kind = message.get('type')
        if kind == 'shutdown':
            break
        if kind == 'command':
            handlers = [_commands.get(message.get('command'))]
            if handlers[0] is None:
                _send({'type': 'error', 'text': f"no command {message.get('command')!r} in this plugin"})
                continue
            argument = message.get('values') or {}
        elif kind == 'event':
            handlers = _events.get(message.get('event'), [])
            argument = message.get('data') or {}
        else:
            continue
        for handler in handlers:
            try:
                handler(fs, argument)
            except Exception:
                _send({'type': 'error', 'text': traceback.format_exc()})
        sys.stdout.flush()
        if kind == 'command':
            _send({'type': 'done', 'command': message.get('command')})
        else:
            _send({'type': 'done'})
    if fs is not None:
        try:
            fs.close()
        except Exception:
            pass
    return 0


if __name__ == '__main__':
    # Run as a script this module is `__main__`; the plugin's own
    # `import tequila_plugin` must find this one, with its registrations,
    # not load a second copy whose commands nobody reads.
    sys.modules['tequila_plugin'] = sys.modules['__main__']
    sys.exit(_serve(Path(sys.argv[1])))
