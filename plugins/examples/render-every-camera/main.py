"""Render Every Camera: one background render per saved camera.

Each render is named after its camera and captures the scene as it is when
queued, so they can all be set going and the work carried on. An example of
reading the document (fs.context()), a folder field, fs.queue() with a
camera, and the panel_opened event keeping the panel's line current.
"""
import tequila_plugin as plugin


def cameras(fs):
    """The saved cameras' names, by index - the number fs.queue() takes."""
    return [camera.get('name') or f'Camera {i + 1}' for i, camera in enumerate(fs.context()['document'].get('cameras', []))]


def chosen(names, only):
    only = (only or '').strip().lower()
    return [(i, name) for i, name in enumerate(names) if not only or only in name.lower()]


def describe(names):
    if not names:
        return 'No saved cameras: save some in the Camera tab first.'
    shown = ', '.join(names[:6]) + ('…' if len(names) > 6 else '')
    return f'{len(names)} saved camera(s): {shown}'


@plugin.on('panel_opened')
def panel_opened(fs, data):
    plugin.set_panel('cameras', status=describe(cameras(fs)))


@plugin.on('opened')
def opened(fs, data):
    # Another project, other cameras.
    plugin.set_panel('cameras', status=describe(cameras(fs)))


@plugin.command('queue')
def queue(fs, values):
    picked = chosen(cameras(fs), values.get('only'))
    if not picked:
        plugin.set_panel('cameras', status='No saved camera has that in its name.')
        return
    job = {}
    if values.get('samples'):
        job['samples'] = int(values['samples'])
    if values.get('folder'):
        job['folder'] = values['folder']
    for index, name in picked:
        fs.queue(name=name, camera=index, **job)
    text = f'Queued {len(picked)} render(s); the Render window shows them as they go.'
    plugin.set_panel('cameras', status=text)
    plugin.status(text)
