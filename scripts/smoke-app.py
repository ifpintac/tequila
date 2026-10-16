"""Start a packaged Tequila, check that it answers on its scripting channel,
take a screenshot of its window, and quit - in folders of its own, so the
settings and work of whoever runs it are never read or touched.

Usage: python3 scripts/smoke-app.py <Tequila.app | package folder> <screenshot.png>
"""
import os
import subprocess
import sys
import tempfile
import time
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
        client.request('quit', discard=True)
        client.close()
        app.wait(timeout=60)
        print('quit with exit code', app.returncode)
        if app.returncode != 0 or not shot.is_file():
            raise SystemExit(1)
    finally:
        if app.poll() is None:
            app.kill()
        print('--- its output, last lines ---')
        print(''.join(log.read_text(encoding='utf-8', errors='replace').splitlines(True)[-40:]))
