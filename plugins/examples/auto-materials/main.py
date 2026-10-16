"""Auto Materials: library materials on parts by their names.

When a model is imported, every new part whose name holds one of a rule's
words gets that rule's library material - screws steel, lenses glass,
grips rubber - as one undo step. The panel edits the rules (or reads them
from a CSV file), turns the import behaviour on or off, and applies them to
the whole scene at a click.

An example of the `imported` and `panel_opened` events, settings kept with
plugin.save_settings(), a file field, and library presets applied in one
fs.batch().
"""
import csv
import json
import re
from pathlib import Path

import tequila_plugin as plugin

# The same as plugin.json's, for when the panel has not been opened.
DEFAULT_RULES = next(f['default'] for f in json.loads((Path(__file__).parent / 'plugin.json').read_text(encoding='utf-8'))['panels'][0]['fields']
                     if f['id'] == 'rules')
DEFAULTS = {'rules': DEFAULT_RULES, 'rules_file': '', 'on_import': True}


def parse_rules(text):
    """Lines (or `;`-separated rules) of `words, words: Material` as
    [(words, material)], in order: the first rule a part's name matches is
    the one used."""
    rules = []
    for chunk in text.replace('\n', ';').split(';'):
        if ':' not in chunk:
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
                words = [w.strip().lower() for w in re.split(r'[|\s]+', row[0]) if w.strip()]
                rules.append((words, row[1].strip()))
    return rules


def words_of(name):
    """A part's name as lower-case words: `FrontLens_02` is front, lens, 02."""
    return [w.lower() for w in re.findall(r'[A-Z]?[a-z]+|[A-Z]+(?![a-z])|\d+', name)]


def match(name, rules):
    """The material for a part called `name`, or None. A word matches a
    word of the name that starts with it (`screw` matches `Screws`); a
    rule word with a hyphen or space matches anywhere in the name."""
    words = words_of(name)
    lowered = name.lower()
    for keys, material in rules:
        for key in keys:
            if (' ' in key or '-' in key) and key in lowered:
                return material
            if any(word.startswith(key) for word in words):
                return material
    return None


def current_rules(settings):
    if settings.get('rules_file'):
        return rules_from_file(settings['rules_file'])
    return parse_rules(settings.get('rules') or DEFAULT_RULES)


def apply(fs, part_ids, rules):
    """Each part in `part_ids` its rule's material, as one undo step.
    Returns (how many parts changed, the materials that are not in the
    library)."""
    library = {p['name'].lower(): p['name'] for p in fs.library.presets()}
    names = {p.id: p.name for p in fs.parts()}
    chosen = {}
    missing = set()
    for part in part_ids:
        material = match(names.get(part, ''), rules)
        if material is None:
            continue
        if material.lower() not in library:
            missing.add(material)
            continue
        chosen.setdefault(library[material.lower()], []).append(part)
    if chosen:
        with fs.batch() as batch:
            for material, parts in chosen.items():
                # One material per rule: every screw the same steel, to
                # change in one place.
                batch.add('library', action='apply', name=material, nodes=parts)
    return sum(len(p) for p in chosen.values()), sorted(missing)


def report(changed, missing, where):
    text = f'{changed} part(s) {where} got a material.' if changed else f'No part {where} matched a rule.'
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
    plugin.set_panel('auto', rules=settings['rules'], rules_file=settings['rules_file'], on_import=settings['on_import'],
                     status=f'{len(rules)} rule(s). New parts are matched as they are imported.' if settings['on_import']
                     else f'{len(rules)} rule(s). Imports are left alone.')


@plugin.command('keep')
def keep(fs, values):
    settings = settings_from(values)
    rules = current_rules(settings)
    plugin.save_settings(settings)
    plugin.set_panel('auto', status=f'Kept {len(rules)} rule(s) for next time.')


@plugin.command('apply')
def apply_all(fs, values):
    settings = settings_from(values)
    parts = [p.id for p in fs.parts()]
    changed, missing = apply(fs, parts, current_rules(settings))
    text = report(changed, missing, 'in the scene')
    plugin.set_panel('auto', status=text)
    plugin.status(text)


@plugin.on('imported')
def imported(fs, data):
    settings = plugin.load_settings(DEFAULTS)
    if not settings['on_import'] or not data.get('parts'):
        return
    changed, missing = apply(fs, data['parts'], current_rules(settings))
    if changed or missing:
        plugin.status(report(changed, missing, 'of the new model'))
