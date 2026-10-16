"""Colourways: one render, or one library material, per colour.

An example of a plugin with code: a panel declared in plugin.json, two
buttons answered here, and the selection event keeping the panel's line of
text current. Everything it does goes through the scripting API, so it is
undoable and shows in the Scripting tab's log.
"""
import tequila
import tequila_plugin as plugin


def _colours(text):
    """The colours in the panel's field, as written: hex codes, by commas."""
    return [c.strip() for c in text.replace(';', ',').split(',') if c.strip()]


def _material(fs):
    """The material on the first selected part that has one."""
    selected = set(fs.query('selection')['nodes'])
    for part in fs.parts():
        if part.id in selected and part.material is not None:
            return fs.materials[part.material]
    return None


@plugin.command('render')
def render(fs, values):
    material = _material(fs)
    if material is None:
        plugin.status('Select a part with a material first')
        return
    colours = _colours(values.get('colours', ''))
    original = material.base_color
    queued = 0
    try:
        for colour in colours:
            material.base_color = tequila.srgb(colour)
            # The queue captures the scene as it is now, so each render
            # keeps its own colour.
            fs.queue(name=f'{material.name} {colour.lstrip("#")}')
            queued += 1
    finally:
        material.base_color = original
    plugin.status(f'Queued {queued} render(s) of {material.name}; the Render window shows them')


@plugin.command('keep')
def keep(fs, values):
    material = _material(fs)
    if material is None:
        plugin.status('Select a part with a material first')
        return
    colours = _colours(values.get('colours', ''))
    original = material.base_color
    kept = 0
    try:
        for colour in colours:
            material.base_color = tequila.srgb(colour)
            fs.library.save(material, f'{material.name} {colour.upper()}', category='Colourways', tags=['colourway', colour])
            kept += 1
    finally:
        material.base_color = original
    plugin.status(f'Kept {kept} colourway(s) of {material.name} on your library\'s Mine shelf')


@plugin.on('selection')
def selection(fs, data):
    material = _material(fs) if data.get('nodes') else None
    text = f'Colours for `{material.name}`.' if material else 'Select a part, and list the colours to try.'
    plugin.set_panel('colourways', status=text)
