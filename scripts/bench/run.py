"""Measure every engine on this machine (docs/07 R0.1).

Runs `tequila --bench` once per scene and engine, each in a process of its
own so its peak memory is its own, and writes:

  OUT/results.jsonl   one measurement a line, as `tequila --json` writes it,
                      with the graphics card's peak memory added where
                      nvidia-smi can read it
  OUT/results.md      the table docs/07 §2.1 and docs/05 §4 take
  OUT/machine.json    what was measured on

The number that matters is `seconds_to_target`: how long each engine takes
to a picture as clean as the target noise (see crates/tequila-cli/src/
noise_bench.rs for how noise is measured).

Usage:
  python3 scripts/bench/run.py [--tequila PATH] [--out DIR] [--width W]
      [--height H] [--scenes NAME,...] [--engines tequila,cycles-cpu,cycles-gpu]

Scenes are the generated ones by name (simple_product, cad_assembly,
fibre_product, subsurface_product, lamp_lit) and the
sample projects in assets/samples by file name. Cycles needs its library
where the app finds it, or TEQUILA_CYCLES set; without it the Cycles rows
say why they are missing and the run carries on.
"""
import argparse
import json
import platform
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SAMPLES = ROOT / 'assets' / 'samples'
GENERATED = ['simple_product', 'cad_assembly', 'fibre_product', 'subsurface_product', 'lamp_lit']
ENGINES = {
    'tequila': ['--engine', 'tequila'],
    'cycles-cpu': ['--engine', 'cycles', '--device', 'CPU'],
    'cycles-gpu': ['--engine', 'cycles', '--device', 'GPU'],
}


def default_tequila():
    exe = 'tequila.exe' if sys.platform == 'win32' else 'tequila'
    for folder in ('target/release', 'target/debug'):
        candidate = ROOT / folder / exe
        if candidate.is_file():
            return candidate
    return None


def scenes_wanted(names):
    found = {p.stem: p for p in sorted(SAMPLES.glob('*.tequila'))}
    every = GENERATED + list(found)
    chosen = names.split(',') if names else every
    out = []
    for name in chosen:
        if name in GENERATED:
            out.append((name, name))
        elif name in found:
            out.append((name, str(found[name])))
        else:
            sys.exit(f'unknown scene {name!r}: one of {", ".join(every)}')
    return out


class CardMemory:
    """The graphics card's memory in use, read every quarter second by
    nvidia-smi while a run lasts; its peak above what was in use before."""

    def __init__(self):
        self.tool = shutil.which('nvidia-smi')
        self.peak = None
        self.base = None
        self._stop = threading.Event()
        self._thread = None

    def read(self):
        try:
            out = subprocess.run([self.tool, '--query-gpu=memory.used', '--format=csv,noheader,nounits'],
                                 capture_output=True, text=True, timeout=5).stdout
            return sum(int(line) for line in out.split() if line.strip().isdigit()) * 1024 * 1024
        except (OSError, ValueError, subprocess.TimeoutExpired):
            return None

    def __enter__(self):
        if self.tool:
            self.base = self.read()
            self.peak = self.base
            self._thread = threading.Thread(target=self._watch, daemon=True)
            self._thread.start()
        return self

    def _watch(self):
        while not self._stop.wait(0.25):
            now = self.read()
            if now is not None and (self.peak is None or now > self.peak):
                self.peak = now

    def __exit__(self, *_):
        self._stop.set()
        if self._thread:
            self._thread.join()

    def used(self):
        if self.peak is None or self.base is None:
            return None
        return max(0, self.peak - self.base)


def measure(tequila, scene, engine, args, jsonl):
    before = jsonl.read_text().count('\n') if jsonl.exists() else 0
    command = [str(tequila), '--bench', scene, *ENGINES[engine], '-w', str(args.width), '-h', str(args.height),
               '--json', str(jsonl)]
    started = time.time()
    with CardMemory() as card:
        run = subprocess.run(command, capture_output=True, text=True)
    lines = jsonl.read_text().splitlines() if jsonl.exists() else []
    if run.returncode != 0 or len(lines) <= before:
        why = (run.stderr or run.stdout).strip().splitlines()
        return None, why[-1] if why else f'exit code {run.returncode}'
    row = json.loads(lines[-1])
    row['card_memory'] = card.used() if engine == 'cycles-gpu' else None
    row['wall_seconds'] = round(time.time() - started, 1)
    lines[-1] = json.dumps(row)
    jsonl.write_text('\n'.join(lines) + '\n')
    return row, None


def table(rows):
    def cell(row, key, fmt):
        return fmt(row[key]) if row and row.get(key) is not None else '-'

    out = ['| Scene | Engine | Samples | Noise | Seconds to clean | Msamples/s | Memory | Card memory |',
           '|---|---|---|---|---|---|---|---|']
    for scene, engine, row, why in rows:
        if row is None:
            out.append(f'| {scene} | {engine} | not measured: {why} | | | | | |')
            continue
        out.append('| {} | {} | {} | {} | {} | {} | {} | {} |'.format(
            scene, engine if engine != 'cycles-gpu' else f'cycles-gpu ({row["device"]})',
            cell(row, 'samples', str),
            cell(row, 'noise', lambda v: f'{v:.4f}'),
            cell(row, 'seconds_to_target', lambda v: f'{v:.1f}'),
            cell(row, 'samples_per_second', lambda v: f'{v / 1e6:.1f}'),
            cell(row, 'peak_memory', lambda v: f'{v / 1e9:.2f} GB'),
            cell(row, 'card_memory', lambda v: f'{v / 1e9:.2f} GB'),
        ))
    return '\n'.join(out) + '\n'


def machine():
    facts = {'system': platform.platform(), 'processor': platform.processor() or platform.machine(),
             'python': platform.python_version(), 'when': time.strftime('%Y-%m-%d %H:%M')}
    if shutil.which('nvidia-smi'):
        out = subprocess.run(['nvidia-smi', '--query-gpu=name,driver_version', '--format=csv,noheader'],
                             capture_output=True, text=True).stdout.strip()
        facts['cards'] = out.splitlines()
    return facts


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--tequila', type=Path, default=default_tequila())
    parser.add_argument('--out', type=Path, default=ROOT / 'target' / 'bench')
    parser.add_argument('--width', type=int, default=960)
    parser.add_argument('--height', type=int, default=540)
    parser.add_argument('--scenes', default='')
    parser.add_argument('--engines', default=','.join(ENGINES))
    args = parser.parse_args()
    if not args.tequila or not args.tequila.is_file():
        sys.exit('no tequila command-line build found: build it (cargo build --release -p tequila-cli) or give --tequila')
    engines = args.engines.split(',')
    for engine in engines:
        if engine not in ENGINES:
            sys.exit(f'unknown engine {engine!r}: one of {", ".join(ENGINES)}')
    args.out.mkdir(parents=True, exist_ok=True)
    jsonl = args.out / 'results.jsonl'
    jsonl.unlink(missing_ok=True)
    (args.out / 'machine.json').write_text(json.dumps(machine(), indent=1))

    rows = []
    for name, scene in scenes_wanted(args.scenes):
        for engine in engines:
            row, why = measure(args.tequila, scene, engine, args, jsonl)
            rows.append((name, engine, row, why))
            print(f'{name:16s} {engine:11s} ' + (f'{row["seconds_to_target"]:8.1f} s to clean' if row else f'not measured: {why}'), flush=True)
            (args.out / 'results.md').write_text(table(rows))
    print(f'\nwrote {args.out / "results.md"}')


if __name__ == '__main__':
    main()
