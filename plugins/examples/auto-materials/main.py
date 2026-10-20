"""Auto Materials: library materials on parts, whatever they are called.

When a model comes in, every part gets a material, decided in turn by:

1. your own rules, words in a name and the material they mean;
2. what it learned from you: the materials you gave parts of the same name
   in projects you saved (or with "Learn from this scene");
3. what the part is called - in English, German, French, Spanish or
   Italian, in CAD shorthand (HSG, BRKT, SCR), or as a fastener standard
   (ISO 4762, DIN 934, M3x10, 1/4-20, a McMaster number) - what the CAD
   file says it is made of ("Stainless Steel 304", "ABS"), its
   description, its layer, and the assemblies it sits in;
4. a part it is a copy of, or named like, that something above matched;
5. and for whatever is left, its shape and colour: a see-through part is
   glass, many identical small parts are hardware, a slender rod is steel,
   and anything else gets a moulded finish in the colour the model gave it.

Plastics and paints keep the colour the model gave the part, so a red
housing stays red with a real moulded finish. Parts that already wear a
library material are left alone unless you say otherwise.

The panel edits the rules (or reads them from a CSV file), previews what
would change and why, and applies the rules to the whole scene at a click.

An example of the `imported` and `panel_opened` events, settings kept with
plugin.save_settings(), a file field, and library presets applied in one
fs.batch().
"""
import csv
import json
import re
import unicodedata
from pathlib import Path

import tequila_plugin as plugin

FIELDS = {f['id']: f for f in json.loads((Path(__file__).parent / 'plugin.json').read_text(encoding='utf-8'))['panels'][0]['fields']}
DEFAULTS = {key: FIELDS[key].get('default', '') for key in
            ('rules', 'rules_file', 'builtin', 'deep', 'guess', 'keep_colours', 'keep_finished', 'on_import', 'learn_on_save')}

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

# Words CAD packages and people shorten, and other languages' words for the
# same things, read as the words the rules know. A name keeps its own words
# as well, so a rule of yours in any of them still matches.
ALIASES = {
    # CAD shorthand.
    'hsg': 'housing', 'hous': 'housing', 'encl': 'enclosure', 'brkt': 'bracket', 'bkt': 'bracket', 'brk': 'bracket',
    'scr': 'screw', 'scrw': 'screw', 'wshr': 'washer', 'wsh': 'washer', 'gskt': 'gasket', 'gsk': 'gasket',
    'brg': 'bearing', 'brng': 'bearing', 'shft': 'shaft', 'sprg': 'spring', 'spg': 'spring', 'btn': 'button',
    'knb': 'knob', 'cvr': 'cover', 'pnl': 'panel', 'frm': 'frame', 'lbl': 'label', 'stdf': 'standoff',
    'stdoff': 'standoff', 'hdl': 'handle', 'mtg': 'mount', 'insrt': 'insert', 'rvt': 'rivet', 'nt': 'nut',
    'lns': 'lens', 'wndw': 'window', 'dspl': 'display', 'disp': 'display', 'gls': 'glass', 'rbr': 'rubber',
    'ss': 'stainless', 'al': 'aluminium', 'alum': 'aluminium', 'cu': 'copper', 'brs': 'brass',
    # German.
    'schraube': 'screw', 'schrauben': 'screw', 'mutter': 'nut', 'muttern': 'nut', 'scheibe': 'washer',
    'unterlegscheibe': 'washer', 'gehause': 'housing', 'gehaeuse': 'housing', 'deckel': 'cover', 'abdeckung': 'cover',
    'glas': 'glass', 'linse': 'lens', 'gummi': 'rubber', 'dichtung': 'seal', 'feder': 'spring', 'welle': 'shaft',
    'lager': 'bearing', 'halter': 'holder', 'halterung': 'bracket', 'griff': 'grip', 'knopf': 'button', 'kabel': 'cable',
    'stahl': 'steel', 'edelstahl': 'stainless', 'holz': 'wood', 'leder': 'leather', 'kunststoff': 'plastic',
    'messing': 'brass', 'kupfer': 'copper', 'bildschirm': 'screen', 'niete': 'rivet', 'stift': 'dowel',
    # French.
    'vis': 'screw', 'ecrou': 'nut', 'rondelle': 'washer', 'boitier': 'housing', 'couvercle': 'cover', 'verre': 'glass',
    'lentille': 'lens', 'caoutchouc': 'rubber', 'ressort': 'spring', 'arbre': 'shaft', 'roulement': 'bearing',
    'bouton': 'button', 'poignee': 'grip', 'acier': 'steel', 'bois': 'wood', 'cuir': 'leather', 'plastique': 'plastic',
    'laiton': 'brass', 'cuivre': 'copper', 'ecran': 'screen', 'rivet': 'rivet', 'support': 'bracket',
    # Spanish.
    'tornillo': 'screw', 'tuerca': 'nut', 'arandela': 'washer', 'carcasa': 'housing', 'tapa': 'cover', 'vidrio': 'glass',
    'cristal': 'glass', 'lente': 'lens', 'goma': 'rubber', 'caucho': 'rubber', 'junta': 'seal', 'resorte': 'spring',
    'muelle': 'spring', 'eje': 'shaft', 'rodamiento': 'bearing', 'boton': 'button', 'acero': 'steel', 'madera': 'wood',
    'cuero': 'leather', 'plastico': 'plastic', 'laton': 'brass', 'cobre': 'copper', 'pantalla': 'screen',
    'remache': 'rivet', 'soporte': 'bracket',
    # Italian.
    'vite': 'screw', 'viti': 'screw', 'dado': 'nut', 'rondella': 'washer', 'alloggiamento': 'housing',
    'coperchio': 'cover', 'vetro': 'glass', 'gomma': 'rubber', 'guarnizione': 'seal', 'molla': 'spring',
    'albero': 'shaft', 'cuscinetto': 'bearing', 'pulsante': 'button', 'acciaio': 'steel', 'legno': 'wood',
    'pelle': 'leather', 'plastica': 'plastic', 'ottone': 'brass', 'rame': 'copper', 'schermo': 'screen',
    'rivetto': 'rivet', 'staffa': 'bracket',
}

# Fastener standards, as CAD libraries name their parts: the kind each is.
STANDARDS = {
    # Socket head cap screws, button heads, countersunk.
    '4762': 'socket head', '912': 'socket head', '7380': 'bhcs', '10642': 'fhcs', '7991': 'fhcs',
    # Hex bolts and screws, set screws.
    '4014': 'bolt', '4017': 'bolt', '931': 'bolt', '933': 'bolt', '4026': 'screw', '4027': 'screw',
    '4028': 'screw', '4029': 'screw', '913': 'screw', '914': 'screw', '915': 'screw', '916': 'screw',
    '7045': 'screw', '7046': 'screw', '7985': 'screw', '965': 'screw', '7981': 'screw', '7982': 'screw',
    # Nuts.
    '4032': 'nut', '4035': 'nut', '934': 'nut', '985': 'nut', '439': 'nut', '1587': 'nut', '10511': 'nut',
    # Washers.
    '7089': 'washer', '7090': 'washer', '7092': 'washer', '7093': 'washer', '125': 'washer', '127': 'washer',
    '433': 'washer', '9021': 'washer', '6798': 'washer',
    # Pins, circlips, rivets.
    '2338': 'dowel', '8734': 'dowel', '6325': 'dowel', '7979': 'dowel', '1481': 'dowel', '471': 'circlip',
    '472': 'circlip', '6799': 'circlip', '15977': 'rivet', '660': 'rivet',
}

# Names that say nothing: what CAD packages call a part nobody named. They
# are not learned from, and a part named one is not "named like" another.
GENERIC = re.compile(r'^(part|body|solid|feature|component|comp|item|mesh|object|obj|shape|unnamed|untitled|default|surface|'
                     r'surfaces|instance|node|group|assembly|asm|subassembly|sub|brep|geometry|import|imported|copy|new)?\s*\d*$')

# What a part nobody could name gets, in its own colour.
MOULDED = 'Plastic Semi-Gloss Grey'
MOULDED_BLACK = 'Plastic Satin Black'


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


def unaccented(text):
    """`Gehäuse` as `Gehause`, `Écrou` as `Ecrou`: names are matched
    without their accents."""
    return ''.join(c for c in unicodedata.normalize('NFKD', text or '') if not unicodedata.combining(c))


def raw_words(name):
    """A name as lower-case words, as written: `FrontLens_02` is front,
    lens, 02; `M3x10-SHCS` is m3x10, shcs."""
    spaced = re.sub(r'([a-z])([A-Z])', r'\1 \2', unaccented(name).replace('ß', 'ss'))
    return [w.lower() for w in re.split(r'[^A-Za-z0-9]+', spaced) if w]


def meanings(name):
    """The words a name means beyond its own: its shorthand and foreign
    words spelled out, and what a fastener standard, a thread or a
    McMaster-Carr number in it says the part is."""
    words = raw_words(name)
    found = [ALIASES[w] for w in words if w in ALIASES]
    text = unaccented(name).lower()
    for code in re.findall(r'(?:iso|din|en|jis|ansi|asme|gb)\s*[-_ ]?\s*(\d{3,5})', text):
        found.append(STANDARDS.get(code, 'screw'))
    # Metric threads with a length (M3x10, M2.5x6), unified ones (#4-40,
    # 1/4-20) and McMaster's fastener numbers (91290A115).
    if re.search(r'(?<![a-z0-9])m\d+(?:[.,]\d+)?\s*x\s*\d+', text) or re.search(r'#\d+\s*-\s*\d+|\b\d+/\d+\s*-\s*\d+\b', text):
        found.append('screw')
    if re.search(r'\b9\d{4}a\d{2,4}\b', text):
        found.append('screw')
    return found


def words_of(name):
    """A name as lower-case words, and the words it means
    (`meanings`): `Gehäuse_SCR` is gehause, scr, housing, screw."""
    return raw_words(name) + meanings(name)


def plain(name):
    """A name for matching phrases: lower case, words apart by one space,
    and what it means after it."""
    return ' ' + ' '.join(raw_words(name)) + ' | ' + ' '.join(meanings(name)) + ' '


def stem(name):
    """A part's name without the counts CAD packages add to copies:
    `Screw <3>`, `Screw_2`, `Screw (2)`, `Screw.001` and `Screw:1` are all
    Screw."""
    name = (name or '').strip()
    while True:
        cut = re.sub(r'\s*(?:<\d+>|\(\d+\)|\[\d+\]|[._:#-]\d+|\s\d+)$', '', name)
        if cut == name or not cut:
            return name
        name = cut


def name_key(name):
    """The name a part is remembered by: its stem, as words."""
    return ' '.join(raw_words(stem(name)))


def says_nothing(name):
    """Whether a name is one nobody chose: `Part12`, `Body3`, `Solid1`."""
    return not raw_words(name) or bool(GENERIC.match(' '.join(raw_words(stem(name)))))


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
    user = []
    if settings.get('rules_file'):
        user += rules_from_file(settings['rules_file'])
    user += parse_rules(settings.get('rules', ''))
    builtin = BUILTIN if settings.get('builtin', True) else []
    learned = {k: v for k, v in (settings.get('learned') or {}).items() if k}
    presets = {p['name'].lower(): p for p in fs.library.presets()}
    nodes = {n.id: n for n in fs.nodes()}
    materials = {m.id: m for m in fs.materials}
    deep = settings.get('deep', True)
    decided, missing, left = {}, set(), []
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
        layers, assemblies = [], []
        if deep:
            own.append((current_name, 'CAD material'))
            for key, value in (part.metadata or {}).items():
                key = key.lower()
                if 'material' in key:
                    own.append((str(value), 'CAD material'))
                elif 'description' in key or 'designation' in key or key in ('part number', 'standard'):
                    own.append((str(value), 'description'))
                elif key == 'layer':
                    layers.append((str(value), 'layer'))
            parent = nodes.get(part.parent)
            # Not the top one: that is usually the file's name.
            while parent is not None and nodes.get(parent.parent) is not None:
                assemblies.append((parent.name, 'assembly'))
                parent = nodes.get(parent.parent)
        material, why = match([own], user)
        if material is not None:
            why = 'your rule'
        if material is None and learned.get(name_key(part.name)) and not says_nothing(part.name):
            material, why = learned[name_key(part.name)], 'learned'
        if material is None:
            material, why = match([own, layers, assemblies], user + builtin)
        if material is None:
            left.append(part)
            continue
        decided[part_id] = (material, why)
    # What is left: a copy of a part that matched, or one named like it,
    # takes its material.
    by_mesh, by_name = {}, {}
    for part_id, (material, _) in decided.items():
        node = nodes[part_id]
        by_mesh.setdefault(getattr(node, 'mesh', None), (material, node.name))
        if not says_nothing(node.name):
            by_name.setdefault(name_key(node.name), (material, node.name))
    copies = {}
    for node in nodes.values():
        if node.kind == 'part' and getattr(node, 'mesh', None) is not None:
            copies[node.mesh] = copies.get(node.mesh, 0) + 1
    model = scene_size(nodes.values())
    unmatched = 0
    for part in left:
        mesh = getattr(part, 'mesh', None)
        if mesh is not None and mesh in by_mesh:
            material, other = by_mesh[mesh]
            decided[part.id] = (material, f'a copy of {other}')
            continue
        if not says_nothing(part.name) and name_key(part.name) in by_name:
            material, other = by_name[name_key(part.name)]
            decided[part.id] = (material, f'named like {other}')
            continue
        guessed = guess(part, materials.get(part.material), copies.get(mesh, 1), model) if settings.get('guess', True) else None
        if guessed is None:
            unmatched += 1
            continue
        decided[part.id] = guessed
    chosen = {}
    for part_id, (material, why) in decided.items():
        preset = presets.get(material.lower())
        if preset is None:
            missing.add(material)
            continue
        current = materials.get(nodes[part_id].material)
        colour = None
        if settings.get('keep_colours', True) and current is not None and preset.get('category', '').lower() in COLOURED:
            colour = model_colour(current)
        chosen.setdefault(preset['name'], []).append((part_id, colour, why))
    return chosen, sorted(missing), unmatched


def size_of(part):
    """A part's box, its sides from shortest to longest, in metres."""
    size = (part.bounds or {}).get('size')
    if not size or len(size) != 3:
        return None
    return sorted(abs(s) for s in size)


def scene_size(nodes):
    """How big the model is: the diagonal of the box around its parts."""
    low, high = [float('inf')] * 3, [float('-inf')] * 3
    for node in nodes:
        bounds = node.bounds if node.kind == 'part' else None
        if not bounds or 'min' not in bounds:
            continue
        for i in range(3):
            low[i] = min(low[i], bounds['min'][i])
            high[i] = max(high[i], bounds['max'][i])
    if low[0] == float('inf'):
        return 0.0
    return sum((high[i] - low[i]) ** 2 for i in range(3)) ** 0.5


def guess(part, material, copies, model):
    """A material for a part nothing named, from what can be seen of it:
    (material, why), or None.

    See-through in the model: glass. Four or more copies of one small shape
    are hardware - screws, nuts, washers, inserts. A slender rod is a pin
    or a shaft, if it is thin and not most of the model. Anything else is
    moulded plastic in the colour the model
    gave it, black if it is black: a finish, where the importer's flat
    colour looked like no material at all."""
    definition = (material.definition or {}) if material is not None else {}
    if definition.get('transmission_weight', 0) >= 0.5 or definition.get('geometry_opacity', 1) < 0.5:
        return 'Glass Clear', 'see-through in the model'
    sides = size_of(part)
    small = sides is not None and sides[2] <= max(0.04, 0.02 * model) and sides[2] <= 0.15 * max(model, 1e-9)
    if sides is not None and copies >= 4 and small:
        return 'Stainless Brushed', f'one of {copies} identical small parts'
    # A rod: round across, at least two and a half times as long, thin (a
    # pen's barrel is slender too, and plastic), and not the model itself.
    if (sides is not None and 0 < sides[0] <= 0.012 and sides[1] <= 1.3 * sides[0] and sides[2] >= 2.5 * sides[1]
            and sides[2] <= 0.6 * max(model, 1e-9)):
        return ('Stainless Brushed' if sides[2] <= 0.03 else 'Steel Turned'), 'a slender rod'
    colour = model_colour(material) if material is not None else None
    if colour is not None and max(colour) < 0.04:
        return MOULDED_BLACK, 'its colour'
    return MOULDED, 'its colour'


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
                batch.apply('material.set_graph', material=material.id, definition=definition)
                # Named for its colour: the graph carries no name, and the copy
                # was "(copy)".
                batch.apply('material.rename', material=material.id, name=f'{preset} ({colour_name(colour)})')
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


def reason(why):
    """What decided, as a heading for the preview's counts."""
    if why.startswith('a copy of') or why.startswith('named like'):
        return 'like another part'
    if why in ('see-through in the model', 'a slender rod', 'its colour') or why.startswith('one of'):
        return 'shape and colour'
    if why in ('your rule', 'learned'):
        return why
    return 'name and CAD data'


def describe(chosen, missing, unmatched, where):
    if not chosen:
        text = f'No part {where} matched a rule.'
    else:
        counts = ', '.join(f'{len(entries)} {name}' for name, entries in sorted(chosen.items(), key=lambda kv: -len(kv[1])))
        total = sum(len(e) for e in chosen.values())
        text = f'{total} part(s) {where}: {counts}.'
        by = {}
        for entries in chosen.values():
            for _, _, why in entries:
                by[reason(why)] = by.get(reason(why), 0) + 1
        order = ['your rule', 'learned', 'name and CAD data', 'like another part', 'shape and colour']
        text += ' By ' + ', '.join(f'{k} {by[k]}' for k in order if k in by) + '.'
    if unmatched:
        text += f' {unmatched} matched nothing.'
    if missing:
        text += ' Not in the library: ' + ', '.join(missing) + '.'
    return text


def learn(fs, settings):
    """What the scene teaches: each part wearing a library material that
    the rules would not have given it, by its name. Returns how many names
    it learned."""
    presets = {p['name'].lower(): p['name'] for p in fs.library.presets()}
    materials = {m.id: m for m in fs.materials}
    rules = current_rules(settings)
    learned = dict(settings.get('learned') or {})
    count = 0
    for part in fs.parts():
        material = materials.get(part.material)
        preset = presets.get(library_name(material.name if material else ''))
        if preset is None or says_nothing(part.name):
            continue
        # What the rules give it already needs no remembering.
        if match([[(part.name, 'name')]], rules)[0] == preset:
            learned.pop(name_key(part.name), None)
            continue
        if learned.get(name_key(part.name)) != preset:
            learned[name_key(part.name)] = preset
            count += 1
    settings['learned'] = learned
    plugin.save_settings(settings)
    return count


def settings_from(values):
    settings = plugin.load_settings(DEFAULTS)
    for key in DEFAULTS:
        if key in values:
            settings[key] = values[key]
    return settings


def remembered(settings):
    count = len(settings.get('learned') or {})
    return f' It knows {count} name(s) from your projects.' if count else ''


@plugin.on('panel_opened')
def panel_opened(fs, data):
    settings = plugin.load_settings(DEFAULTS)
    rules = current_rules(settings)
    plugin.set_panel('auto', **{k: settings[k] for k in DEFAULTS},
                     status=(f'{len(rules)} rule(s). New parts are matched as they are imported.' if settings['on_import']
                             else f'{len(rules)} rule(s). Imports are left alone.') + remembered(settings))


@plugin.command('keep')
def keep(fs, values):
    settings = settings_from(values)
    rules = current_rules(settings)
    plugin.save_settings(settings)
    plugin.set_panel('auto', status=f'Kept {len(rules)} rule(s) for next time.' + remembered(settings))


@plugin.command('learn')
def learn_now(fs, values):
    settings = settings_from(values)
    count = learn(fs, settings)
    text = f'Learned {count} new name(s) from this scene.' if count else 'Nothing new to learn here: the rules already give these parts their materials.'
    plugin.set_panel('auto', status=text + remembered(settings))
    plugin.status(text)


@plugin.command('forget')
def forget(fs, values):
    settings = settings_from(values)
    settings['learned'] = {}
    plugin.save_settings(settings)
    plugin.set_panel('auto', status='Forgot every name it had learned.')


@plugin.on('saved')
def saved(fs, data):
    settings = plugin.load_settings(DEFAULTS)
    if settings.get('learn_on_save', True):
        learn(fs, settings)


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
