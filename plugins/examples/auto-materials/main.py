"""Auto Materials: library materials on parts by what they are called.

When a model comes in, every part is matched against the rules - your own
first, then Tequila's built-in ones - by its name, by the name of the
material the CAD file gave it ("Stainless Steel 304", "ABS"), and by the
assemblies it sits in ("Fasteners", "Front Bezel"). Screws get steel,
lenses glass, grips rubber, a part whose CAD material is aluminium gets
aluminium. Plastics and paints keep the colour the model gave the part, so
a red housing stays red with a real moulded finish. Parts that already wear
a library material are left alone unless you say otherwise.

The panel edits the rules (or reads them from a CSV file), previews what
would change, and applies the rules to the whole scene at a click.

An example of the `imported` and `panel_opened` events, settings kept with
plugin.save_settings(), a file field, and library presets applied in one
fs.batch().
"""
import csv
import json
import re
from pathlib import Path

import tequila_plugin as plugin

FIELDS = {f['id']: f for f in json.loads((Path(__file__).parent / 'plugin.json').read_text(encoding='utf-8'))['panels'][0]['fields']}
DEFAULTS = {key: FIELDS[key].get('default', '') for key in ('rules', 'rules_file', 'builtin', 'deep', 'keep_colours', 'keep_finished', 'on_import')}

# Tequila's own rules, after yours: the names parts, CAD materials and
# assemblies are commonly given, and the library preset each wants. The
# more particular come first: the first rule that matches is the one used.
BUILTIN = [
    # Lights and screens.
    (['led', 'leds', 'indicator light'], 'LED Cool White'),
    (['screen', 'display', 'lcd', 'oled', 'touchscreen'], 'Screen Glow'),
    # Clear parts.
    (['light pipe', 'lightpipe', 'light guide', 'lightguide', 'diffuser'], 'Acrylic Frosted'),
    (['sapphire'], 'Sapphire Crystal'),
    (['lens', 'lenses', 'window', 'glass', 'glazing', 'cover glass', 'visor'], 'Glass Clear'),
    (['acrylic', 'pmma', 'plexiglass', 'perspex'], 'Acrylic Clear'),
    (['polycarbonate clear', 'clear pc', 'transparent'], 'Polycarbonate Clear'),
    # 3D prints.
    (['petg'], 'PETG 3D Print Black'),
    (['3d print', '3d printed', '3dp', 'fdm', 'fff', 'pla'], 'FDM PLA White'),
    (['sla', 'resin print'], 'Resin SLA Grey'),
    # Fasteners and hardware.
    (['shcs', 'bhcs', 'fhcs', 'socket head', 'cap screw'], 'Steel Black Oxide'),
    (['screw', 'bolt', 'nut', 'washer', 'rivet', 'standoff', 'stand-off', 'spacer', 'threaded insert', 'heat set insert',
      'hex key', 'circlip', 'retaining ring', 'dowel'], 'Stainless Brushed'),
    (['spring'], 'Stainless Polished'),
    (['bearing', 'shaft', 'axle', 'bushing', 'linear rod'], 'Steel Turned'),
    (['magnet', 'neodymium'], 'Nickel Satin'),
    (['heatsink', 'heat sink', 'cooler', 'fin'], 'Aluminium Anodised Black'),
    (['grille', 'grill', 'speaker mesh', 'perforated'], 'Steel Perforated Round'),
    (['contact', 'pogo', 'terminal'], 'Gold Polished'),
    (['chrome', 'chromed', 'chrome plated'], 'Chrome'),
    # Soft parts.
    (['o-ring', 'oring', 'gasket', 'seal', 'grommet', 'bumper', 'damper', 'foot', 'feet', 'tyre', 'tire', 'rubber', 'epdm',
      'nbr', 'neoprene', 'tpu', 'tpe', 'grip'], 'Rubber Black'),
    (['silicone'], 'Silicone Soft Grey'),
    (['foam', 'cushion', 'eva'], 'EVA Foam Black'),
    (['strap', 'leather'], 'Leather Black'),
    (['fabric', 'textile', 'cloth', 'upholstery'], 'Upholstery Charcoal'),
    (['cable', 'cord', 'wire', 'harness'], 'Plastic Satin Black'),
    (['label', 'sticker', 'decal'], 'Paper Matte'),
    # Metals, by what they are made of.
    (['stainless', 'inox', '304', '316', 'a2', 'a4'], 'Stainless Brushed'),
    (['anodised', 'anodized'], 'Aluminium Anodised Silver'),
    (['aluminium', 'aluminum', 'alu', '6061', '6063', '7075', '5052', 'alsi'], 'Aluminium Bead Blasted'),
    (['titanium', 'ti6al4v', 'grade 5'], 'Titanium Satin'),
    (['brass'], 'Brass Satin'),
    (['bronze'], 'Bronze Satin'),
    (['copper'], 'Copper Satin'),
    (['zinc', 'zamak', 'galvanised', 'galvanized'], 'Steel Zinc Plated Clear'),
    (['cast iron'], 'Cast Iron'),
    (['gold'], 'Gold Polished'),
    (['silver'], 'Silver Polished'),
    (['steel', 'carbon steel', 's235', 's355', '1018', '1045', '4140'], 'Steel Turned'),
    # Plastics, by what they are made of.
    (['carbon fibre', 'carbon fiber', 'cfrp', 'carbon composite'], 'Carbon Fibre'),
    (['fibreglass', 'fiberglass', 'gfrp', 'g10', 'fr4'], 'Fiberglass Gelcoat White'),
    (['pom', 'acetal', 'delrin'], 'POM White'),
    (['nylon', 'polyamide', 'pa6', 'pa12', 'pa66'], 'Nylon Black'),
    (['hdpe', 'polyethylene'], 'HDPE Milk White'),
    (['polypropylene', 'pp'], 'Polypropylene Natural'),
    (['pvc'], 'PVC Grey'),
    (['abs', 'pc abs', 'pc-abs', 'polycarbonate', 'plastic', 'polymer', 'injection moulded', 'injection molded'], 'ABS Textured'),
    # Plastic parts, by what they are.
    (['button', 'key', 'keycap', 'knob', 'dial', 'switch', 'trigger', 'slider'], 'Plastic Satin Black'),
    (['housing', 'enclosure', 'case', 'casing', 'cover', 'shell', 'body', 'lid', 'bezel', 'cap', 'panel', 'frame', 'bracket',
      'mount', 'holder', 'clip', 'base', 'stand'], 'Plastic VDI 24 Grey'),
    # Everything else people make things of.
    (['wood', 'timber', 'plywood'], 'Wood Beech'),
    (['oak'], 'Wood White Oak'),
    (['walnut'], 'Wood Walnut'),
    (['maple'], 'Wood Maple'),
    (['cork'], 'Cork'),
    (['ceramic', 'porcelain'], 'Ceramic Glazed White'),
    (['concrete', 'cement'], 'Concrete'),
    (['marble'], 'Marble White'),
    (['granite'], 'Granite Polished'),
    (['paper', 'cardboard', 'carton'], 'Paper Matte'),
]

# Library categories whose look a part's own colour should carry.
COLOURED = {'plastic', 'paint', 'fdm 3d print', 'leather', 'fabric', 'composite'}


def parse_rules(text):
    """Lines (or `;`-separated rules) of `words, words: Material` as
    [(words, material)], in order."""
    rules = []
    for chunk in (text or '').replace('\n', ';').split(';'):
        if ':' not in chunk or chunk.lstrip().startswith('#'):
            continue
        words, material = chunk.rsplit(':', 1)
        words = [w.strip().lower() for w in words.split(',') if w.strip()]
        if words and material.strip():
            rules.append((words, material.strip()))
    return rules


def rules_from_file(path):
    """A CSV file of rows `words, material` - the words may be separated by
    spaces or `|` - read as rules."""
    rules = []
    with open(path, newline='', encoding='utf-8-sig') as file:
        for row in csv.reader(file):
            if len(row) >= 2 and row[0].strip() and not row[0].lstrip().startswith('#'):
                words = [w.strip().lower() for w in re.split(r'[|]+', row[0]) if w.strip()]
                if len(words) == 1 and ' ' in words[0] and len(words[0].split()) > 2:
                    words = words[0].split()
                rules.append((words, row[1].strip()))
    return rules


def words_of(name):
    """A name as lower-case words: `FrontLens_02` is front, lens, 02;
    `M3x10-SHCS` is m3x10, shcs."""
    spaced = re.sub(r'([a-z])([A-Z])', r'\1 \2', name or '')
    return [w.lower() for w in re.split(r'[^A-Za-z0-9]+', spaced) if w]


def plain(name):
    """A name for matching phrases: lower case, words apart by one space."""
    return ' ' + ' '.join(words_of(name)) + ' '


def word_matches(key, words):
    """A rule word against a name's words: the word itself or its plural,
    or for a word of five letters or more, any word starting with it
    (`enclos` would not, `screw` matches `screwdriver`... and `screws`)."""
    for word in words:
        if word == key or word in (key + 's', key + 'es'):
            return True
        if len(key) >= 5 and word.startswith(key):
            return True
    return False


def match_text(text, rules):
    """The first rule's material that `text` matches, or None. A rule word
    with a space or a hyphen is a phrase, found anywhere in the name."""
    words = words_of(text)
    phrase = plain(text)
    for keys, material in rules:
        for key in keys:
            if ' ' in key or '-' in key:
                if plain(key) in phrase or key.replace('-', '') in words:
                    return material
            elif word_matches(key, words):
                return material
    return None


def match(tiers, rules):
    """The material for a part, and what matched: each rule in turn against
    what the part itself is called - its name, its CAD material's - so a
    housing whose CAD material is aluminium is aluminium; then, only if
    nothing did, against the assemblies it sits in."""
    for tier in tiers:
        for keys, material in rules:
            for text, source in tier:
                if text and match_text(text, [(keys, material)]):
                    return material, source
    return None, None


def library_name(name):
    """A material's library preset, from its name in the scene: `ABS
    Textured (red)` and `ABS Textured (copy)` are ABS Textured."""
    return re.sub(r'\s*\([^)]*\)$', '', name or '').lower()


def current_rules(settings):
    rules = []
    if settings.get('rules_file'):
        rules += rules_from_file(settings['rules_file'])
    rules += parse_rules(settings.get('rules', ''))
    if settings.get('builtin', True):
        rules += BUILTIN
    return rules


def plan(fs, part_ids, settings):
    """What the rules would do: {material: [(part, colour or None, why)]},
    the materials not in the library, and how many parts nothing matched."""
    rules = current_rules(settings)
    presets = {p['name'].lower(): p for p in fs.library.presets()}
    nodes = {n.id: n for n in fs.nodes()}
    materials = {m.id: m for m in fs.materials}
    deep = settings.get('deep', True)
    chosen, missing, unmatched = {}, set(), 0
    for part_id in part_ids:
        part = nodes.get(part_id)
        if part is None or part.kind != 'part':
            continue
        current = materials.get(part.material)
        current_name = current.name if current else ''
        # A library material, put there by hand or by these rules: leave it.
        if settings.get('keep_finished', True) and library_name(current_name) in presets:
            continue
        own = [(part.name, 'name')]
        assemblies = []
        if deep:
            own.append((current_name, 'CAD material'))
            own += [(str(v), 'CAD material') for k, v in (part.metadata or {}).items() if 'material' in k.lower()]
            parent = nodes.get(part.parent)
            # Not the top one: that is usually the file's name.
            while parent is not None and nodes.get(parent.parent) is not None:
                assemblies.append((parent.name, 'assembly'))
                parent = nodes.get(parent.parent)
        material, why = match([own, assemblies], rules)
        if material is None:
            unmatched += 1
            continue
        preset = presets.get(material.lower())
        if preset is None:
            missing.add(material)
            continue
        colour = None
        if settings.get('keep_colours', True) and current is not None and preset.get('category', '').lower() in COLOURED:
            colour = model_colour(current)
        chosen.setdefault(preset['name'], []).append((part_id, colour, why))
    return chosen, sorted(missing), unmatched


def model_colour(material):
    """The colour the model gave a part, if it gave one: not the plain
    grey or white an importer falls back to."""
    colour = (material.definition or {}).get('base_color')
    if not colour or len(colour) != 3:
        return None
    spread = max(colour) - min(colour)
    level = sum(colour) / 3
    # Neutral greys and whites are what a file without colours gets.
    if spread < 0.03 and 0.15 < level < 0.85:
        return None
    return [round(c, 4) for c in colour]


def apply(fs, part_ids, settings):
    """The rules on `part_ids`: one undo step for the materials, and one
    more when parts keep their own colours. Returns (parts changed, the
    materials not in the library, parts nothing matched)."""
    chosen, missing, unmatched = plan(fs, part_ids, settings)
    recolour = []
    if chosen:
        with fs.batch() as batch:
            for material, entries in chosen.items():
                shared = [part for part, colour, _ in entries if colour is None]
                if shared:
                    # One material per rule: every screw the same steel,
                    # to change in one place.
                    batch.add('library', action='apply', name=material, nodes=shared)
                by_colour = {}
                for part, colour, _ in entries:
                    if colour is not None:
                        by_colour.setdefault(tuple(colour), []).append(part)
                for colour, parts in by_colour.items():
                    # A copy for each colour the model used.
                    batch.add('library', action='apply', name=material, nodes=parts, unlink=True)
                    recolour.append((parts[0], list(colour), material))
    if recolour:
        materials = {m.id: m for m in fs.materials}
        nodes = {n.id: n for n in fs.nodes()}
        with fs.batch() as batch:
            for part, colour, preset in recolour:
                material = materials.get(nodes[part].material) if part in nodes else None
                if material is None:
                    continue
                definition = recoloured(material.definition, colour)
                definition['name'] = f'{preset} ({colour_name(colour)})'
                batch.apply('material.set_graph', material=material.id, definition=definition)
    return sum(len(e) for e in chosen.values()), missing, unmatched


def recoloured(definition, colour):
    """The material in another colour: its base colour, and where its graph
    paints the colour itself (a print's filament under its creases), the
    node that holds the old one."""
    definition = json.loads(json.dumps(definition))
    old = definition.get('base_color')
    definition['base_color'] = colour
    for node in (definition.get('graph') or {}).get('nodes', []):
        for literal in node.get('literals', []):
            value = literal.get('value', {})
            if 'color' in value and old and all(abs(a - b) < 1e-3 for a, b in zip(value['color'], old)):
                value['color'] = colour
    return definition


def colour_name(colour):
    """A colour as a person would call it, roughly, for the material's name."""
    r, g, b = colour
    top, low = max(colour), min(colour)
    if top - low < 0.05:
        return 'white' if top > 0.6 else 'black' if top < 0.05 else 'grey'
    hue = 0.0
    if top == r:
        hue = ((g - b) / (top - low)) % 6
    elif top == g:
        hue = (b - r) / (top - low) + 2
    else:
        hue = (r - g) / (top - low) + 4
    hue *= 60
    for limit, name in ((15, 'red'), (45, 'orange'), (70, 'yellow'), (165, 'green'), (200, 'cyan'), (255, 'blue'), (290, 'purple'),
                        (335, 'pink'), (360, 'red')):
        if hue <= limit:
            return name
    return 'colour'


def describe(chosen, missing, unmatched, where):
    if not chosen:
        text = f'No part {where} matched a rule.'
    else:
        counts = ', '.join(f'{len(entries)} {name}' for name, entries in sorted(chosen.items(), key=lambda kv: -len(kv[1])))
        total = sum(len(e) for e in chosen.values())
        text = f'{total} part(s) {where}: {counts}.'
    if unmatched:
        text += f' {unmatched} matched nothing.'
    if missing:
        text += ' Not in the library: ' + ', '.join(missing) + '.'
    return text


def settings_from(values):
    settings = plugin.load_settings(DEFAULTS)
    for key in DEFAULTS:
        if key in values:
            settings[key] = values[key]
    return settings


@plugin.on('panel_opened')
def panel_opened(fs, data):
    settings = plugin.load_settings(DEFAULTS)
    rules = current_rules(settings)
    plugin.set_panel('auto', **{k: settings[k] for k in DEFAULTS},
                     status=f'{len(rules)} rule(s). New parts are matched as they are imported.' if settings['on_import']
                     else f'{len(rules)} rule(s). Imports are left alone.')


@plugin.command('keep')
def keep(fs, values):
    settings = settings_from(values)
    rules = current_rules(settings)
    plugin.save_settings(settings)
    plugin.set_panel('auto', status=f'Kept {len(rules)} rule(s) for next time.')


@plugin.command('preview')
def preview(fs, values):
    settings = settings_from(values)
    chosen, missing, unmatched = plan(fs, [p.id for p in fs.parts()], settings)
    plugin.set_panel('auto', status='Would give ' + describe(chosen, missing, unmatched, 'in the scene'))


@plugin.command('apply')
def apply_all(fs, values):
    settings = settings_from(values)
    changed, missing, unmatched = apply(fs, [p.id for p in fs.parts()], settings)
    text = f'{changed} part(s) in the scene got a material.' if changed else 'No part in the scene matched a rule.'
    if missing:
        text += ' Not in the library: ' + ', '.join(missing) + '.'
    plugin.set_panel('auto', status=text)
    plugin.status(text)


@plugin.on('imported')
def imported(fs, data):
    settings = plugin.load_settings(DEFAULTS)
    if not settings['on_import'] or not data.get('parts'):
        return
    changed, missing, _ = apply(fs, data['parts'], settings)
    if changed or missing:
        text = f'Auto Materials: {changed} part(s) of the new model got a material.'
        if missing:
            text += ' Not in the library: ' + ', '.join(missing) + '.'
        plugin.status(text)
