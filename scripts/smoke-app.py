"""Start a packaged Tequila, check that it answers on its scripting channel,
take a screenshot of its window, resize a label on a part over and over with
the viewport rendering, as a drag does, and quit - in folders of its own, so
the settings and work of whoever runs it are never read or touched.

On a Mac it then opens two saved projects the way Finder does, with `open`:
one to launch Tequila, one sent to it while it runs. Launched by macOS, that
Tequila has the runner's own settings, so run it only on a machine that is
there to be tested.

Usage: python3 scripts/smoke-app.py <Tequila.app | package folder> <screenshot.png>
"""
import glob
import os
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

target = Path(sys.argv[1]).resolve()
shot = Path(sys.argv[2]).resolve()
if target.suffix == '.app':
    program = target / 'Contents' / 'MacOS' / 'Tequila'
    module = target / 'Contents' / 'Resources' / 'python'
else:
    program = target / ('Tequila.exe' if os.name == 'nt' else 'Tequila')
    module = target / 'python'
sys.path.insert(0, str(module))
import tequila  # noqa: E402

STARTED = time.time()


def say(*what):
    """A line of progress, timed from the start, out at once: a hang shows
    where it began."""
    print(f'[{time.time() - STARTED:6.1f} s]', *what, flush=True)


def stacks(pid):
    """Where every thread of a process that stopped answering is: macOS's
    `sample`, three seconds of it."""
    if sys.platform != 'darwin':
        return
    sampled = subprocess.run(['sample', str(pid), '3'], capture_output=True, text=True)
    say('--- where Tequila is, by sample ---')
    print('\n'.join((sampled.stdout or sampled.stderr).splitlines()[:400]), flush=True)


def write_label(path, width=64, height=32):
    """A red label with a white band: a PNG written by hand, as the repository
    keeps no picture of its own to use."""
    rows = b''.join(b'\x00' + b''.join(
        (b'\xff\xff\xff\xff' if height // 3 <= y < 2 * height // 3 else b'\xd0\x10\x10\xff') for _ in range(width))
        for y in range(height))

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data) & 0xffffffff)
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
                     + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def drawn(client):
    """Whether the viewport shows a picture: Cycles past preparing the
    graphics card, with samples in it."""
    state = client.ui()
    return state.get('viewport_samples', 0) > 0 and not state.get('viewport_preparing', False)


def first_frame(client, what):
    """Wait for the viewport to show a picture. The first time Cycles runs
    it compiles its graphics-card code, minutes on a Mac, and a label resized
    before it ends was never rendered: the test passed without testing. On a
    machine whose card takes longer than FIRST_FRAME_SECONDS (GitHub's Mac
    gave no picture in twenty minutes), carry on and say so."""
    seconds = float(os.environ.get('FIRST_FRAME_SECONDS', '600'))
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            if drawn(client):
                say(what, 'drawn:', client.ui().get('viewport_samples'), 'samples')
                return True
        except Exception:  # noqa: BLE001 - busy; ask again
            pass
        time.sleep(1)
    say(f'NOTE: no picture {what} after {seconds:.0f} s; carrying on without one')
    return False


def resize_labels(client, box):
    """The tester's crash on a Mac (2026-10-19): insert a label, change its
    size, and Tequila closed. A part, a label on it, the Rendered view, and
    sixty sizes in two seconds, with Cycles rendering."""
    label = box / 'label.png'
    write_label(label)
    # The part a new window opens with. The Rendered view first: a runner
    # with no graphics card draws the Solid view's shaders in software, and a
    # procedural material there took minutes a frame.
    part = next(n['id'] for n in client.query('nodes')['nodes'] if n.get('kind') == 'part')
    client.ui(viewport='rendered', tab='material')
    client.library.apply('Plastic Matte White', part)
    say('applied a library material')
    material = next(m['id'] for m in client.query('materials')['materials']
                    if m['used_by'] and m.get('name', '').startswith('Plastic Matte White'))
    client.apply('material.add_decal', material=material, name='Label', path=str(label), role='color')
    say('added a label')
    rendering = first_frame(client, 'with the label')
    # The label as Tequila fitted it to the part, its size changed as a drag
    # on the size field does: a mapping written here instead put it half a
    # metre off a part 10 cm across, and nothing was ever drawn.
    definition = next(m for m in client.query('materials')['materials'] if m['id'] == material)['definition']
    fitted = (definition.get('graph') or definition)['decals'][0]
    scale = fitted['mapping']['scale']
    for i in range(60):
        factor = 0.25 + 1.5 * abs(((i % 20) - 10) / 10.0)
        decal = dict(fitted, mapping=dict(fitted['mapping'], scale=[scale[0] * factor, scale[1] * factor] + list(scale[2:])))
        client.apply('material.set_decals', material=material, decals=[decal])
        time.sleep(1 / 30)
    client.apply('material.set_decals', material=material, decals=[fitted])
    if rendering:
        first_frame(client, 'after resizing it')
    say('resized a label 60 times in the Rendered view' + ('' if rendering else ', with no picture yet'))


def wait_for(what, check, seconds=120):
    deadline = time.time() + seconds
    while time.time() < deadline:
        try:
            if check():
                return
        except Exception:  # noqa: BLE001 - not there yet
            pass
        time.sleep(1)
    raise SystemExit(f'{what} did not happen in {seconds} s')


def showing(client):
    """The project the window has open, symbolic links and all resolved."""
    path = client.query('history')['path']
    return Path(path).resolve() if path else None


def recent_list():
    """Where a Tequila that macOS launched, with the real settings, keeps
    the projects opened lately."""
    return Path.home() / 'Library' / 'Application Support' / 'Tequila' / 'recent.txt'


def forget_recent(projects, existed):
    """Take this test's projects off the real recent list again, and only
    them: File > Open recent offered two projects in a deleted folder."""
    path = recent_list()
    if not path.is_file():
        return
    ours = {project.resolve() for project in projects}
    lines = path.read_text(encoding='utf-8').splitlines()
    kept = [line for line in lines if not line.strip() or Path(line.strip()).resolve() not in ours]
    if not existed and not any(line.strip() for line in kept):
        path.unlink()
    elif kept != lines:
        path.write_text(''.join(line + '\n' for line in kept), encoding='utf-8')


def opened_from_finder(app_bundle, first, second):
    """Double-clicking a project in Finder sends it to Tequila by an Apple
    Event, not on the command line: launched by one, and sent one while it
    runs, Tequila opens each."""
    existed = recent_list().is_file()
    try:
        finder_opens(app_bundle, first, second)
    finally:
        forget_recent([first, second], existed)


def finder_opens(app_bundle, first, second):
    discovery = Path.home() / '.tequila' / 'channel.json'
    if discovery.exists():
        discovery.unlink()
    subprocess.run(['open', '-a', str(app_bundle), str(first)], check=True)
    client = None

    def answering():
        nonlocal client
        client = tequila.Client(str(discovery), timeout=60)
        return True
    wait_for('Tequila launched by open', answering)
    pid = client.pid
    try:
        wait_for(f'opening {first.name} at launch', lambda: showing(client) == first.resolve())
        say('launched by Finder with', first.name)
        subprocess.run(['open', '-a', str(app_bundle), str(second)], check=True)
        wait_for(f'opening {second.name} while running', lambda: showing(client) == second.resolve())
        say('sent', second.name, 'while running, and it opened')
        client.request('quit', discard=True)
        client.close()
    finally:
        wait_for('Tequila to quit', lambda: subprocess.run(['kill', '-0', str(pid)], capture_output=True).returncode != 0, 60)

box = Path(tempfile.mkdtemp(prefix='tequila-smoke-'))
home = box / 'home'
for folder in (home, box / 'Roaming', box / 'Local'):
    folder.mkdir(parents=True)
env = dict(os.environ, HOME=str(home), USERPROFILE=str(home), APPDATA=str(box / 'Roaming'), LOCALAPPDATA=str(box / 'Local'))
# Cycles' compiled graphics-card code is kept under ~/.cache on a Mac, and is
# neither settings nor work: the real one, so a Mac that has rendered before
# shows a picture in seconds, not after minutes of compiling.
if os.name != 'nt':
    env['XDG_CACHE_HOME'] = os.environ.get('XDG_CACHE_HOME') or str(Path.home() / '.cache')
log = box / 'tequila.log'
with open(log, 'w', encoding='utf-8') as output:
    app = subprocess.Popen([str(program)], env=env, stdout=output, stderr=subprocess.STDOUT)
    client = None
    try:
        deadline = time.time() + 180
        while time.time() < deadline and app.poll() is None and client is None:
            try:
                client = tequila.Client(str(home / '.tequila' / 'channel.json'), expected_pid=app.pid, timeout=60)
            except Exception:  # noqa: BLE001 - not answering yet
                time.sleep(1)
        if client is None:
            raise SystemExit(f'Tequila did not answer (exit code {app.poll()})')
        names = [node['name'] for node in client.query('nodes')['nodes']]
        say('answering; the scene holds', names)
        time.sleep(3)
        client.request('screenshot', path=str(shot))
        for _ in range(60):
            if shot.is_file() and shot.stat().st_size > 0:
                break
            time.sleep(0.5)
        say('screenshot:', shot, shot.stat().st_size if shot.is_file() else 'not written')
        # Two projects for Finder to open, after: of the demo as it opens,
        # whose plain material a runner draws quickly.
        projects = [box / 'second.tequila', box / 'first.tequila']
        for project in projects:
            client.save(project)
        resize_labels(client, box)
        labelled = shot.with_name(shot.stem + '-labels' + shot.suffix)
        client.request('screenshot', path=str(labelled))
        for _ in range(60):
            if labelled.is_file() and labelled.stat().st_size > 0:
                break
            time.sleep(0.5)
        client.request('quit', discard=True)
        client.close()
        app.wait(timeout=60)
        say('quit with exit code', app.returncode)
        if app.returncode != 0 or not shot.is_file():
            raise SystemExit(1)
        if target.suffix == '.app' and sys.platform == 'darwin':
            opened_from_finder(target, projects[1], projects[0])
    except BaseException:
        if app.poll() is None:
            stacks(app.pid)
        raise
    finally:
        if app.poll() is None:
            app.kill()
        print('--- its output, last lines ---')
        print(''.join(log.read_text(encoding='utf-8', errors='replace').splitlines(True)[-40:]))
        # What a crash left: Tequila's own report, and on a Mac the system's.
        reports = sorted((home / '.tequila' / 'crashes').glob('crash-*.txt'))
        # macOS names them for the process, `tequila-…` for the command-line
        # one; only those of this run.
        reports += sorted(report for report in glob.glob(os.path.expanduser('~/Library/Logs/DiagnosticReports/*.ips'))
                          if os.path.basename(report).lower().startswith('tequila')
                          and os.path.getmtime(report) >= STARTED - 1)
        for report in reports:
            print('--- crash report', report, '---')
            print(Path(report).read_text(encoding='utf-8', errors='replace')[:6000])
