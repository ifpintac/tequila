"""The look Cycles gives Tequila's pictures, kept, and compared against
(docs/07 R0.2).

Before Cycles leaves Tequila, the pictures it makes of what Tequila ships
are rendered once and kept: after it has gone, they are the only record of
the look existing projects were made in, and Tequila's own renderer is held
to them (R0.4, R6.2).

    python3 scripts/look-refs/looks.py make    --out DIR [--device GPU]
    python3 scripts/look-refs/looks.py compare --refs DIR_OR_ZIP --out DIR
                                               [--engine tequila]

`make` renders the set with Cycles at 1,024 samples into DIR, writes
DIR/manifest.json, and zips it as DIR.zip, printing its SHA-256. `compare`
renders the same set with another engine (Tequila's renderer by default,
at 256 samples: enough for a mean and a correlation) and writes
DIR/report.md, worst first, and DIR/sheet-N.png, the worst pictures side
by side with their references.

The set:
  samples/   each project in assets/samples, as it is saved, 960x540
  demo/      the demonstration scene in plastic, metal and diffuse
  studios/   every Studio applied to the Headphones, 960x540
  library/   every preset in the material library on a ball 5 cm across,
             as the library's own previews are, 256x256

Each picture is rendered as the application renders a final picture:
through the command line's render, which is what the render queue's
pictures are made by, with the project's own look and denoising.

Needs the command-line renderer (`cargo build --release -p tequila-cli`, or
--tequila) and Cycles where it finds it, or TEQUILA_CYCLES.

The archive made on 2026-11-17, look-refs-1.zip, is kept on a draft release
of the repository (drafts are seen only by its writers, and never start a
release build or the Discord announcement: it must never be published):

    gh release download look-refs-1 --repo ifpintac/tequila -p look-refs-1.zip

`compare` checks a zip against the SHA-256 kept beside this script under
its name, look-refs-1.sha256.
"""
import argparse
import base64
import hashlib
import io
import json
import math
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'python'))
import tequila  # noqa: E402

SAMPLES_DIR = ROOT / 'assets' / 'samples'
KINDS = ('samples', 'demo', 'studios', 'library')


def default_tequila():
    exe = 'tequila.exe' if sys.platform == 'win32' else 'tequila'
    for folder in ('target/release', 'target/debug'):
        candidate = ROOT / folder / exe
        if candidate.is_file():
            return candidate
    return None


def engine_args(engine, device):
    if engine == 'tequila':
        return ['--engine', 'tequila']
    return ['--engine', 'cycles', '--device', device]


def ball_gltf(path, radius=0.025, segments=96, rings=48):
    """A UV sphere `radius` metres across the middle, as a glTF file:
    glTF is in metres, so nothing guesses its size on import."""
    positions, normals, uvs, indices = [], [], [], []
    for ring in range(rings + 1):
        v = ring / rings
        theta = v * math.pi
        for segment in range(segments + 1):
            u = segment / segments
            phi = u * 2 * math.pi
            n = (math.sin(theta) * math.cos(phi), math.cos(theta), math.sin(theta) * math.sin(phi))
            positions.append(tuple(c * radius for c in n))
            normals.append(n)
            uvs.append((u, v))
    for ring in range(rings):
        for segment in range(segments):
            a = ring * (segments + 1) + segment
            b = a + segments + 1
            indices += [a, a + 1, b, a + 1, b + 1, b]
    blob = io.BytesIO()
    for p in positions:
        blob.write(struct.pack('<3f', *p))
    for n in normals:
        blob.write(struct.pack('<3f', *n))
    for t in uvs:
        blob.write(struct.pack('<2f', *t))
    for i in indices:
        blob.write(struct.pack('<I', i))
    data = blob.getvalue()
    count = len(positions)
    pos_len, nrm_len, uv_len = count * 12, count * 12, count * 8
    gltf = {
        'asset': {'version': '2.0'},
        'scene': 0,
        'scenes': [{'nodes': [0]}],
        'nodes': [{'mesh': 0, 'name': 'Ball', 'translation': [0, radius, 0]}],
        'meshes': [{'name': 'Ball', 'primitives': [{'attributes': {'POSITION': 0, 'NORMAL': 1, 'TEXCOORD_0': 2}, 'indices': 3}]}],
        'buffers': [{'byteLength': len(data), 'uri': 'data:application/octet-stream;base64,' + base64.b64encode(data).decode()}],
        'bufferViews': [
            {'buffer': 0, 'byteOffset': 0, 'byteLength': pos_len},
            {'buffer': 0, 'byteOffset': pos_len, 'byteLength': nrm_len},
            {'buffer': 0, 'byteOffset': pos_len + nrm_len, 'byteLength': uv_len},
            {'buffer': 0, 'byteOffset': pos_len + nrm_len + uv_len, 'byteLength': len(indices) * 4},
        ],
        'accessors': [
            {'bufferView': 0, 'componentType': 5126, 'count': count, 'type': 'VEC3',
             'min': [-radius, -radius, -radius], 'max': [radius, radius, radius]},
            {'bufferView': 1, 'componentType': 5126, 'count': count, 'type': 'VEC3'},
            {'bufferView': 2, 'componentType': 5126, 'count': count, 'type': 'VEC2'},
            {'bufferView': 3, 'componentType': 5125, 'count': len(indices), 'type': 'SCALAR'},
        ],
    }
    Path(path).write_text(json.dumps(gltf))


def safe(name):
    return ''.join(c if c.isalnum() or c in '-_' else '_' for c in name).strip('_') or 'unnamed'


def render_set(cli, engine, device, samples, out, kinds, limit, log):
    """Render the set into `out`; return the manifest, one entry a picture."""
    out.mkdir(parents=True, exist_ok=True)
    manifest = {}
    args = engine_args(engine, device)

    def record(rel, kind, name, started, extra=None):
        entry = {'kind': kind, 'name': name, 'seconds': round(time.time() - started, 2)}
        entry.update(extra or {})
        manifest[rel] = entry
        log(f'{rel}  {entry["seconds"]:.1f} s')

    def cli_render(rel, kind, name, extra_args):
        path = out / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        started = time.time()
        run = subprocess.run([str(cli), *extra_args, *args, '-s', str(samples), '-o', str(path)],
                             capture_output=True, text=True)
        if run.returncode != 0 or not path.is_file():
            why = (run.stderr or run.stdout).strip().splitlines()
            log(f'{rel}  FAILED: {why[-1] if why else run.returncode}')
            return
        record(rel, kind, name, started)

    if 'samples' in kinds:
        for project in sorted(SAMPLES_DIR.glob('*.tequila'))[:limit]:
            cli_render(f'samples/{safe(project.stem)}.png', 'samples', project.stem,
                       ['-i', str(project), '-w', '960', '-h', '540'])
    if 'demo' in kinds:
        for material in ('plastic', 'metal', 'diffuse')[:limit]:
            cli_render(f'demo/{material}.png', 'demo', material, ['-m', material, '-w', '960', '-h', '540'])
    if 'studios' in kinds:
        headphones = SAMPLES_DIR / 'Headphones.tequila'
        with tequila.open(headphones, cli=cli, args=args) as session:
            studios = session.studios().list()
            studios = studios.get('studios', studios) if isinstance(studios, dict) else studios
            for studio in studios[:limit]:
                sid = studio.get('id') if isinstance(studio, dict) else studio
                rel = f'studios/{safe(str(sid))}.png'
                started = time.time()
                try:
                    session.open_project(headphones, discard=True)
                    session.studios().apply(sid)
                    session.render(out / rel, width=960, height=540, samples=samples)
                    record(rel, 'studios', str(sid), started)
                except tequila.TequilaError as error:
                    log(f'{rel}  FAILED: {error}')
    if 'library' in kinds:
        with tempfile.TemporaryDirectory(prefix='look-refs-') as scratch:
            ball = Path(scratch) / 'ball.gltf'
            ball_gltf(ball)
            with tequila.open(ball, cli=cli, args=args) as session:
                parts = [n for n in session.query('nodes')['nodes'] if n.get('kind') == 'part']
                if not parts:
                    raise SystemExit('the ball did not import as a part')
                part = parts[0]['id']
                # The library previews' own view of their ball (`preview_camera`
                # in tequila-render-cpu/src/preview.rs), the ball here standing
                # on the floor rather than sunk into it.
                r = 0.025
                session.frame(all=True)
                session.look(frm=(2.1 * r, 2.5 * r, 3.0 * r), at=(0.0, 0.95 * r, 0.0))
                presets = session.library.presets()
                presets = presets.get('presets', presets) if isinstance(presets, dict) else presets
                presets = [p for p in presets if p.get('source', 'shipped') != 'mine']
                for preset in presets[:limit]:
                    name = preset['name']
                    rel = f'library/{safe(name)}.png'
                    started = time.time()
                    try:
                        session.library.apply(name, part)
                        session.render(out / rel, width=256, height=256, samples=samples)
                        record(rel, 'library', name, started, {'category': preset.get('category')})
                    except tequila.TequilaError as error:
                        log(f'{rel}  FAILED: {error}')
    return manifest


def git_commit():
    try:
        return subprocess.run(['git', 'rev-parse', 'HEAD'], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    except OSError:
        return None


def make(args):
    out = args.out
    if out.exists() and any(out.iterdir()) and not args.resume:
        sys.exit(f'{out} is not empty')
    started = time.time()
    manifest = render_set(args.tequila, 'cycles', args.device, args.samples, out, args.kinds, args.limit, print)
    meta = {'engine': 'cycles', 'device': args.device, 'samples': args.samples, 'commit': git_commit(),
            'made': time.strftime('%Y-%m-%d %H:%M'), 'seconds': round(time.time() - started), 'pictures': manifest}
    (out / 'manifest.json').write_text(json.dumps(meta, indent=1))
    archive = out.with_suffix('.zip')
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as z:
        for path in sorted(out.rglob('*')):
            if path.is_file():
                z.write(path, path.relative_to(out))
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    print(f'\n{len(manifest)} pictures in {meta["seconds"]} s\nwrote {archive}\nsha256 {digest}')


def load_refs(refs):
    """A folder, or a zip unpacked to a temporary folder."""
    refs = Path(refs)
    if refs.is_dir():
        return refs, None
    kept = Path(__file__).with_name(refs.stem + '.sha256')
    if kept.is_file():
        want = kept.read_text().split()[0]
        got = hashlib.sha256(refs.read_bytes()).hexdigest()
        if got != want:
            sys.exit(f'{refs} is not the archive {kept.name} names: sha256 {got}, not {want}')
    scratch = tempfile.TemporaryDirectory(prefix='look-refs-')
    with zipfile.ZipFile(refs) as z:
        z.extractall(scratch.name)
    return Path(scratch.name), scratch


def picture(path):
    from PIL import Image
    return Image.open(path).convert('RGB')


def linear(img):
    import numpy as np
    a = np.asarray(img, dtype=np.float64) / 255.0
    return np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4)


def compare_pair(ref_path, new_path):
    """Mean absolute difference and correlation in display-linear values,
    and how far apart the two means are, as a share of the reference's."""
    import numpy as np
    a, b = linear(picture(ref_path)), linear(picture(new_path))
    if a.shape != b.shape:
        return None
    mad = float(np.abs(a - b).mean())
    corr = float(np.corrcoef(a.ravel(), b.ravel())[0, 1]) if a.std() > 0 and b.std() > 0 else 1.0
    mean_shift = float((b.mean() - a.mean()) / max(a.mean(), 1e-6))
    return {'mad': mad, 'correlation': corr, 'mean_shift': mean_shift}


def sheet(rows, refs, new, path, label):
    from PIL import Image, ImageDraw, ImageFont
    try:
        font = ImageFont.truetype('segoeui.ttf' if os.name == 'nt' else 'Helvetica.ttc', 16)
    except OSError:
        font = ImageFont.load_default()
    cell, pad, text = 320, 8, 44
    im = Image.new('RGB', (pad + 2 * (cell + pad), pad + len(rows) * (cell * 9 // 16 + text + pad)), (14, 15, 17))
    d = ImageDraw.Draw(im)
    y = pad
    for rel, r in rows:
        h = cell * 9 // 16
        for i, folder in enumerate((refs, new)):
            img = picture(folder / rel)
            img.thumbnail((cell, h))
            im.paste(img, (pad + i * (cell + pad), y + text))
        d.text((pad, y + 2), f'{rel}', font=font, fill=(236, 236, 238))
        d.text((pad, y + 22), f'correlation {r["correlation"]:.3f}   mean {r["mean_shift"]:+.0%}   (left: Cycles, right: {label})',
               font=font, fill=(150, 152, 158))
        y += h + text + pad
    im.save(path)


def compare(args):
    refs, scratch = load_refs(args.refs)
    meta = json.loads((refs / 'manifest.json').read_text())
    kinds = sorted({e['kind'] for e in meta['pictures'].values()} & set(args.kinds))
    new = args.out / 'renders'
    manifest = render_set(args.tequila, args.engine, args.device, args.samples, new, kinds, args.limit, print)
    rows = []
    for rel in sorted(manifest):
        if rel in meta['pictures'] and (refs / rel).is_file():
            r = compare_pair(refs / rel, new / rel)
            if r:
                rows.append((rel, r))
    rows.sort(key=lambda row: row[1]['correlation'] - abs(row[1]['mean_shift']))
    flagged = [row for row in rows if row[1]['correlation'] < args.correlation or abs(row[1]['mean_shift']) > args.mean]
    label = args.engine if args.engine == 'tequila' else f'{args.engine} {args.device}'
    lines = [f'# {label} against the Cycles look ({meta.get("made")}, {meta.get("samples")} samples)', '',
             f'{len(rows)} pictures compared; {len(flagged)} with correlation under {args.correlation} '
             f'or a mean more than {args.mean:.0%} off.', '',
             '| Picture | Correlation | Mean | Mean absolute difference |', '|---|---|---|---|']
    for rel, r in rows:
        mark = ' **!**' if (rel, r) in flagged else ''
        lines.append(f'| {rel}{mark} | {r["correlation"]:.3f} | {r["mean_shift"]:+.1%} | {r["mad"]:.4f} |')
    (args.out / 'report.md').write_text('\n'.join(lines) + '\n')
    (args.out / 'report.json').write_text(json.dumps({rel: r for rel, r in rows}, indent=1))
    for n in range(0, min(len(flagged), args.sheet), 8):
        sheet(flagged[n:n + 8], refs, new, args.out / f'sheet-{n // 8 + 1}.png', label)
    print(f'\n{len(rows)} compared, {len(flagged)} flagged; wrote {args.out / "report.md"}')
    if scratch:
        scratch.cleanup()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('make', 'compare'):
        p = sub.add_parser(name)
        p.add_argument('--tequila', type=Path, default=default_tequila())
        p.add_argument('--out', type=Path, required=True)
        p.add_argument('--device', default='GPU', help='the Cycles device: GPU (the best card) or CPU')
        p.add_argument('--kinds', default=','.join(KINDS))
        p.add_argument('--limit', type=int, default=None, help='at most this many of each kind (for a quick try)')
        if name == 'make':
            p.add_argument('--samples', type=int, default=1024)
            p.add_argument('--resume', action='store_true')
        else:
            p.add_argument('--refs', type=Path, required=True)
            p.add_argument('--engine', default='tequila', choices=('tequila', 'cycles'))
            p.add_argument('--samples', type=int, default=256)
            p.add_argument('--correlation', type=float, default=0.98)
            p.add_argument('--mean', type=float, default=0.05)
            p.add_argument('--sheet', type=int, default=48, help='how many flagged pictures go on sheets')
    args = parser.parse_args()
    if not args.tequila or not Path(args.tequila).is_file():
        sys.exit('no tequila command-line build found: build it or give --tequila')
    args.kinds = [k for k in args.kinds.split(',') if k]
    for kind in args.kinds:
        if kind not in KINDS:
            sys.exit(f'unknown kind {kind!r}: one of {", ".join(KINDS)}')
    (make if args.command == 'make' else compare)(args)


if __name__ == '__main__':
    main()
