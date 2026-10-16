# Library credits and provenance

`TEQUILA_SPEC.md` §0.2 sets the rules this directory follows. They are worth
restating here, because this is the directory where they are easiest to break by
accident.

## Materials (`standard/`)

The Tequila Standard Library is **original work**. Every parameter value was
chosen from published physical data — measured complex indices of refraction for
the metals, ordinary refractive indices for the glasses and plastics — or from
plain observation of the material being described.

Nothing here is derived from a commercial swatch system. §0.2 rule 3 forbids
bundling Pantone, RAL, Mold-Tech, Sorensen, Coloro, INTERPON or Axalta
libraries, so the presets are named for **what a material is** — "Aluminium
Brushed", "ABS White" — rather than for anybody's catalogue entry. A preset
named after a swatch code would be a redistribution of that catalogue even if
the numbers behind it were arrived at independently.

A test in `tequila-format` fails the build if a preset name, category or tag
mentions one of those systems. It is a crude check and deliberately so: the way
this rule gets broken is somebody matching a swatch by eye and naming the file
after it, which a crude check catches.

## Metals

The complex indices of refraction in `tequila-materials::fresnel::metals` are
measured optical constants, which are physical facts rather than anyone's
creative work. The presets here select from that table by name.

## HDRIs and images

None are bundled yet. When they are, §0.2 rule 4 applies: CC0 sources only, with
each file's origin and licence recorded in this file. KeyShot's own HDRIs and
material files are never to be included, in any form.

## Contributing a preset

- Describe the material, do not name a swatch.
- Set only what your chosen catalog type exposes. If a material needs controls
  the type hides, its type is `generic` — a preset that sets a hidden parameter
  is more often a typo than an intention, and a test says so.
- Values are scene-linear. The generator converts from 8-bit sRGB where that is
  the natural way to describe a colour.
