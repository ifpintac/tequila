"""Package the Claude skill for working in Tequila (skills/tequila) as a ZIP
to upload to a Claude account: Settings > Capabilities > Skills > Upload.

The same ZIP as package-skill.ps1 makes on Windows, for a Mac (and anywhere
else Python 3 runs): the folder `tequila` with SKILL.md at its root, the
hand-written references, and reference/commands.md - docs/scripting-api.md,
which is generated from the command catalog and tested against the build, so
the skill's list of commands is always this version's.

Usage: python3 scripts/package-skill.py [out.zip]
"""
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

PREFACE = """# Every Tequila command and operation

Generated from Tequila's own command catalog. With the `tequila_*` tools:

- a command `{"command": "<name>", ...}` is sent as `tequila_apply {"command": {...}}`, or inside `tequila_batch` as `{"op": "apply", "command": {...}}`;
- an operation `{"op": "<name>", ...}` (library, texture_set, reimport, export...) goes through `tequila_request {"request": {...}}`;
- queries are `tequila_query {"what": ...}`.

Ids and keys in the examples are placeholders: read the real ones from the scene first. The Python lines (`fs.commands...`) are for Tequila's Python module and can be ignored here.

---
"""


def main():
    out = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'dist' / 'Tequila-Claude-Skill.zip'
    out.parent.mkdir(parents=True, exist_ok=True)
    skill = ROOT / 'skills' / 'tequila'
    reference = (ROOT / 'docs' / 'scripting-api.md').read_text(encoding='utf-8')
    with zipfile.ZipFile(out, 'w', zipfile.ZIP_DEFLATED) as archive:
        archive.write(skill / 'SKILL.md', 'tequila/SKILL.md')
        for file in sorted((skill / 'reference').iterdir()):
            if file.is_file():
                archive.write(file, f'tequila/reference/{file.name}')
        archive.writestr('tequila/reference/commands.md', PREFACE + reference)
    print(f'Skill: {out}')


if __name__ == '__main__':
    main()
