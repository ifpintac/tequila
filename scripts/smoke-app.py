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


def resize_labels(client, box):
    """The tester's crash on a Mac (2026-10-19): insert a label, change its
    size, and Tequila closed. A part, a label on it, the Rendered view, and
    sixty sizes in two seconds."""
    label = box / 'label.png'
    write_label(label)
    # The part a new window opens with.
    part = next(n['id'] for n in client.query('nodes')['nodes'] if n.get('kind') == 'part')
    client.library.apply('Plastic Matte White', part)
    material = next(m['id'] for m in client.query('materials')['materials']
                    if m['used_by'] and m.get('name', '').startswith('Plastic Matte White'))
    client.apply('material.add_decal', material=material, name='Label', path=str(label), role='color')
    client.ui(viewport='rendered', tab='material')
    time.sleep(5)
    for i in range(60):
        size = 0.002 + 0.05 * abs(((i % 20) - 10) / 10.0)
        mapping = {'projection': 'planar', 'axis': 'y', 'real_world_units': True, 'scale': [size, size * 0.5, 1.0],
                   'offset': [0.5, 0.5], 'pivot': [0.5, 0.5], 'rotation': 0, 'uv_set': 0, 'blend_sharpness': 6,
                   'flip_u': False, 'flip_v': False, 'wrap_u': 'clamp', 'wrap_v': 'clamp'}
        client.apply('material.set_decals', material=material, decals=[
            {'enabled': True, 'mapping': mapping, 'metallic': False, 'name': 'Label', 'opacity': 1,
             'roughness': 0.35, 'texture': 1}])
        time.sleep(1 / 30)
    time.sleep(5)
    print('resized a label 60 times in the Rendered view')


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


def opened_from_finder(app_bundle, first, second):
    """Double-clicking a project in Finder sends it to Tequila by an Apple
    Event, not on the command line: launched by one, and sent one while it
    runs, Tequila opens each."""
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
        print('launched by Finder with', first.name)
        subprocess.run(['open', '-a', str(app_bundle), str(second)], check=True)
        wait_for(f'opening {second.name} while running', lambda: showing(client) == second.resolve())
        print('sent', second.name, 'while running, and it opened')
        client.request('quit', discard=True)
        client.close()
    finally:
        wait_for('Tequila to quit', lambda: subprocess.run(['kill', '-0', str(pid)], capture_output=True).returncode != 0, 60)

box = Path(tempfile.mkdtemp(prefix='tequila-smoke-'))
home = box / 'home'
for folder in (home, box / 'Roaming', box / 'Local'):
    folder.mkdir(parents=True)
env = dict(os.environ, HOME=str(home), USERPROFILE=str(home), APPDATA=str(box / 'Roaming'), LOCALAPPDATA=str(box / 'Local'))
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
        print('answering; the scene holds', names)
        time.sleep(3)
        client.request('screenshot', path=str(shot))
        for _ in range(60):
            if shot.is_file() and shot.stat().st_size > 0:
                break
            time.sleep(0.5)
        print('screenshot:', shot, shot.stat().st_size if shot.is_file() else 'not written')
        resize_labels(client, box)
        # Two projects for Finder to open, after.
        projects = [box / 'second.tequila', box / 'first.tequila']
        for project in projects:
            client.save(project)
        labelled = shot.with_name(shot.stem + '-labels' + shot.suffix)
        client.request('screenshot', path=str(labelled))
        for _ in range(60):
            if labelled.is_file() and labelled.stat().st_size > 0:
                break
            time.sleep(0.5)
        client.request('quit', discard=True)
        client.close()
        app.wait(timeout=60)
        print('quit with exit code', app.returncode)
        if app.returncode != 0 or not shot.is_file():
            raise SystemExit(1)
        if target.suffix == '.app' and sys.platform == 'darwin':
            opened_from_finder(target, projects[1], projects[0])
    finally:
        if app.poll() is None:
            app.kill()
        print('--- its output, last lines ---')
        print(''.join(log.read_text(encoding='utf-8', errors='replace').splitlines(True)[-40:]))
        # What a crash left: Tequila's own report, and on a Mac the system's.
        reports = sorted((home / '.tequila' / 'crashes').glob('crash-*.txt'))
        reports += sorted(glob.glob(os.path.expanduser('~/Library/Logs/DiagnosticReports/Tequila*.ips')))
        for report in reports:
            print('--- crash report', report, '---')
            print(Path(report).read_text(encoding='utf-8', errors='replace')[:6000])
