# UV layouts

A part's UV layout is its surface laid out flat, so a picture or a label can be placed on it exactly. A part needs one only for that: the library's presets are procedural and need no UVs, and a texture can be projected `tri_planar` without them (page `textures`). CAD files (STEP, IGES) and STL arrive with no UVs; glTF, FBX and OBJ usually bring their own.

`uv.*` commands go through `tequila_apply`. The `uv_layout` request and the UV editor (`ui`) go through `tequila_request`.

## 1. See what the part has

Open the UV editor on the part, ask what it shows, and take a picture of its window. Nothing is changed and no file is written:

```json
tequila_request {"request": {"op": "ui", "uv": {"part": 12, "zoom": "fit", "heatmap": "area"}}}
tequila_query {"what": "ui"}
tequila_screenshot {"window": "uv"}
```

Under `uv`, the query answers with `island_count` (how many pieces the layout is in: they are numbered from 0 to one less), `seams`, the `set` shown, and a `message` - "The part has no layout to work on: unwrap it first" when there is none. The heatmap (`area`, `angle` or `off`) colours where the layout stretches the surface. `{"op": "ui", "uv_editor": false}` closes the editor.

## 2. Unwrap

```json
{"command": "scene.unwrap_uvs", "node": 12, "method": "conformal", "crease_degrees": 60, "margin": 0.005}
```

- The part is cut along its sharp edges and each piece laid flat. It becomes the part's first UV set; sets it already had move up one, so nothing is lost. One undo step.
- `crease_degrees`: an edge sharper than this is a cut. Lower (30-45) gives more, flatter pieces with less stretch; higher (75-90) fewer, larger ones. `cone_degrees` is how much a curved piece may bend before it is split (66 by default).
- `method`: `conformal` keeps angles (right for labels and print); `least_stretch` goes on from there to even out areas (right for tiling textures).
- `margin` is the gap between pieces, as a fraction of the layout: 0.005 to 0.02.
- `"respect_cuts": true` keeps the pieces the first set already has and lays each out again whole; with `"in_place": 0` too, the set named is laid out again where it is and no set moves.

Look again (step 1). Many small pieces: raise `crease_degrees`. Red in the heatmap: lower it, or use `least_stretch`.

## 3. Tidy the layout

Islands are named by number, from 0 to one less than the `island_count` step 1 answered. `uv.pack` takes **all of them when its list is empty**; the others need the islands listed.

```json
{"command": "uv.pack", "node": 12, "set": 0, "islands": [], "margin": 0.01, "rotate": true, "normalize": true, "orient": true}
{"command": "uv.straighten", "node": 12, "set": 0, "islands": [0, 1, 2, 3, 4], "to": "nearest"}
{"command": "uv.even_density", "node": 12, "set": 0, "islands": [0, 1, 2, 3, 4]}
{"command": "uv.reflatten", "node": 12, "set": 0, "islands": [0, 1, 2, 3, 4], "pins": [], "repack": true}
```

- `uv.pack` fits the islands into the square as large as they go. `normalize` first makes them one density, `orient` turns each to its tightest box, `rotate` allows quarter turns. `"region": [u0, v0, u1, v1]` packs into part of the square instead.
- `uv.straighten` turns islands so their long side is `horizontal`, `vertical` or whichever is `nearest`.
- `uv.even_density` scales islands so a centimetre of surface gets the same amount of picture everywhere; `"match": 1` uses island 1's density.
- `uv.reflatten` lays islands out flat again, keeping their cuts.

Whole islands can also be moved by a matrix - `uv.transform` (the same map for the islands listed, about a `pivot`) and `uv.place_islands` (one map each). `[a, b, tx, c, d, ty]` means u' = a u + b v + tx and v' = c u + d v + ty; `[1, 0, 0.25, 0, 1, 0]` slides a quarter across. A move that would fold the layout is refused.

## 4. UV sets

A part can carry several layouts: one for a tiling texture, one for a label.

```json
{"command": "uv.add_set", "node": 12}
{"command": "uv.copy_set", "node": 12, "from": 0, "to": 1}
{"command": "uv.move_set", "node": 12, "from": 1, "to": 0}
{"command": "uv.delete_set", "node": 12, "set": 1}
```

`uv.add_set` adds a fresh unwrap after the others. Textures and labels on the part follow the sets as they move. Which set a texture reads is its mapping's `uv_set` (page `textures`).

## 5. The layout as a picture, for artwork

This is what a designer draws over to make artwork that fits the part:

```json
tequila_request {"request": {"op": "uv_layout", "node": 12, "path": "C:/work/bottle-uv.svg", "size": 4096, "fill": "none", "line": "#000000", "transparent": true}}
```

- It answers with what it wrote: `islands`, `seams`, the `set`, and a `note` such as "Square: 153 mm on the part · 0.149 mm a pixel". A part with no layout is refused: "has no UV set 0".
- The format is the path's extension: `.svg` (lines that stay sharp, to draw over) or `.png`. The unit square is `size` pixels a side, v running down as a picture's rows do.
- `fill`: `islands` (each piece tinted), `none`, or a stretch heatmap, `area` or `angle`. `edges`, `seams` and the `note` with the real size are drawn unless set `false`; `line` is the lines' colour; `transparent` leaves out the white square.
- It writes a file, so only to a path the person gave or agreed. Tell them the `note`: it is the real size of the square, which the artwork's resolution depends on.

Guides across the layout, which the editor snaps to, are saved with the part:

```json
{"command": "scene.set_uv_guides", "node": 12, "guides": [{"u": 0.5}, {"v": 0.25}]}
```

## Not practical through the tools yet

**Choosing where a seam goes, and moving one particular island.** `uv.cut`, `uv.stitch`, `uv.set_pins` and `uv.set_positions` name edges and vertices by the mesh's own vertex numbers, and islands by number, and no tool tells you which number is where on the part. Do not guess them. What you can do:

- change where the cuts fall with `crease_degrees` and unwrap again;
- open the editor for the person (`{"op": "ui", "uv": {"part": 12}}`) and say what to do there: pick edges and **Cut**, pick islands and **Stitch**, drag islands, **Pack**;
- with a picture of the editor in front of you (`tequila_screenshot {"window": "uv"}`), click a point of the layout to pick the island there (`{"op": "ui", "uv": {"mode": "islands", "click": [0.3, 0.6]}}`, the layout's square running 0 to 1) or name islands (`"islands": [0, 1]`), then act on the selection, one request each: `"straighten": true`, `"even": true`, `"reflatten": true`, `"pack": true`, or `"transform": {"offset": [0.1, 0], "turn": 90, "scale": 0.5}`. Afterwards `tequila_query {"what": "ui"}` says, under `uv`, which `islands` are selected and in `message` what the last tool did ("Straightened 2 islands"). Take a picture again after each.

## Good to know

- A texture that looks stretched or smeared on a part with its own UVs: look at the heatmap before unwrapping again, since unwrapping replaces what artwork was drawn for.
- A label or texture that lands on the wrong piece: the layout has many islands; a larger `crease_degrees` joins them.
- Unwrapping a very dense part (hundreds of thousands of triangles) takes a few seconds.
- UVs are kept with the part in the project, and exported with it (glTF, USD).
