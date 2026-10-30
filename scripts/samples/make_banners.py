"""The start screen's banners (app/assets/start): each sample scene shot
wide, the product a little right of centre and the dark set to its left for
the welcome to sit on. The product is moved across by shifting the lens, not
by turning the camera, so a light behind it in the environment stays behind
it.

    python make_banners.py OUT [samples]

TEQUILA_CLI and TEQUILA_CYCLES as for make_samples.py.
"""
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from make_samples import CLI, REPO  # noqa: E402
from studio import around, session  # noqa: E402

WIDTH, HEIGHT = 2160, 600
SENSOR = 36.0


def shot(name, engine, target, azimuth, elevation, focal, height, across):
    """`height` metres of the set from top to bottom; the product `across`
    of the way along the frame."""
    view = around(azimuth, elevation)
    distance = height * focal / (SENSOR * HEIGHT / WIDTH)
    eye = [target[i] + view[i] * distance for i in range(3)]
    return dict(name=name, engine=engine, at=target, eye=eye, focal=focal, shift=-(across - 0.5))


SHOTS = [
    shot('Headphones', 'cycles', [0.0, 0.094, 0.0], 30, 6, 58, 0.27, 0.68),
    shot('Perfume bottle', 'tequila', [0.0, 0.054, 0.0], 18, 4, 58, 0.155, 0.70),
]


def main():
    out = Path(sys.argv[1]).resolve()
    samples = int(sys.argv[2]) if len(sys.argv) > 2 else 256
    out.mkdir(parents=True, exist_ok=True)
    for s in SHOTS:
        with session(CLI, s['engine']) as c:
            c.open_project(str(REPO / 'assets' / 'samples' / f"{s['name']}.tequila"), discard=True)
            c.look(frm=s['eye'], at=s['at'], focal_length=s['focal'])
            document = c.context()['document']
            index = document.get('active_camera', 0)
            index = index if isinstance(index, int) else 0
            camera = dict(document['cameras'][index])
            camera.update({'type': 'Shift', 'shift_x': s['shift'], 'shift_y': 0.0})
            c.apply('camera.set', index=index, camera=camera)
            render = c.context()['document']['render']
            render.update({'alpha_mode': 'none', 'depth': '8'})
            c.apply('render.set_settings', render=render)
            path = out / f"{s['name'].lower().replace(' ', '-')}.png"
            if path.exists():
                path.unlink()
            result = c.render(str(path), width=WIDTH, height=HEIGHT, samples=samples)
            print(f"{s['name']}: {result['seconds']:.0f} s, {path.stat().st_size / 1e3:.0f} kB", flush=True)


if __name__ == '__main__':
    main()
