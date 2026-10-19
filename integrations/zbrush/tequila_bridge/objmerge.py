# Merge ZBrush's OBJ exports, one per SubTool, into one OBJ for Tequila.
#
# Each SubTool becomes one object named after it (`o <name>`), so Tequila
# makes one part of it and, when the model is sent again, matches the part
# by that name and keeps its material. ZBrush's polypaint - the `#MRGB`
# block, a mask byte and a colour per vertex - is written on the vertex
# lines as display values (`v x y z r g b`), which Tequila reads. Polygroup
# lines (`g`) are dropped: they would split a SubTool into many parts.
#
# Standard library only, and nothing of ZBrush's, so it can be tested
# anywhere: python objmerge.py OUT.obj NAME=IN.obj [NAME=IN.obj ...]
#
# SPDX-License-Identifier: Apache-2.0

import os
import sys


def _read(path):
    """Positions, texture coordinates, normals, faces and polypaint of one
    OBJ file. Faces keep their corners as (v, vt, vn) index triples, 1-based
    within this file, None where absent."""
    positions, uvs, normals, faces, paint = [], [], [], [], []
    with open(path, encoding="utf-8", errors="replace") as f:
        for raw in f:
            line = raw.strip()
            if line.startswith("#MRGB"):
                digits = line[5:].strip()
                for i in range(0, len(digits) - 7, 8):
                    try:
                        value = int(digits[i:i + 8], 16)
                    except ValueError:
                        break
                    paint.append(((value >> 16) & 255, (value >> 8) & 255, value & 255))
                continue
            line = line.split("#", 1)[0].strip()
            if not line:
                continue
            words = line.split()
            key, rest = words[0], words[1:]
            if key == "v":
                positions.append(rest[:6])
            elif key == "vt":
                uvs.append(rest[:2])
            elif key == "vn":
                normals.append(rest[:3])
            elif key == "f":
                corners = []
                for corner in rest:
                    parts = (corner.split("/") + ["", ""])[:3]

                    def index(text, count):
                        if not text:
                            return None
                        i = int(text)
                        return i if i > 0 else count + i + 1

                    corners.append((index(parts[0], len(positions)), index(parts[1], len(uvs)), index(parts[2], len(normals))))
                faces.append(corners)
    return positions, uvs, normals, faces, paint


def merge(out_path, subtools):
    """Write `subtools`, a list of (name, obj path), as one OBJ."""
    lines = ["# Sent from ZBrush by the Tequila bridge"]
    base_v = base_vt = base_vn = 0
    for name, path in subtools:
        positions, uvs, normals, faces, paint = _read(path)
        painted = len(paint) == len(positions) and positions
        lines.append("o " + (name.replace("\n", " ").strip() or "SubTool"))
        for i, p in enumerate(positions):
            xyz = " ".join(p[:3])
            if painted:
                r, g, b = paint[i]
                lines.append(f"v {xyz} {r / 255:.4f} {g / 255:.4f} {b / 255:.4f}")
            elif len(p) >= 6:
                lines.append("v " + " ".join(p[:6]))
            else:
                lines.append("v " + xyz)
        lines += ["vt " + " ".join(t) for t in uvs]
        lines += ["vn " + " ".join(n) for n in normals]
        for face in faces:
            out = []
            for v, vt, vn in face:
                text = str(v + base_v)
                if vt is not None or vn is not None:
                    text += "/" + (str(vt + base_vt) if vt is not None else "")
                if vn is not None:
                    text += "/" + str(vn + base_vn)
                out.append(text)
            lines.append("f " + " ".join(out))
        base_v += len(positions)
        base_vt += len(uvs)
        base_vn += len(normals)
    tmp = out_path + ".part"
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    os.replace(tmp, out_path)


# Run as a script, but never inside ZBrush, which may run every .py it is
# given at startup as __main__.
if __name__ == "__main__" and "zbrush" not in sys.modules:
    if len(sys.argv) < 3:
        sys.exit("usage: python objmerge.py OUT.obj NAME=IN.obj [NAME=IN.obj ...]")
    merge(sys.argv[1], [tuple(a.split("=", 1)) for a in sys.argv[2:]])
