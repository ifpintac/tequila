# UV layouts

A part's UV layout is its surface laid out flat, so a picture or a label can be placed on it exactly. A part needs one only for that: the library's presets are procedural and need no UVs, and a texture can be projected `tri_planar` without them (page `textures`). CAD files (STEP, IGES) and STL arrive with no UVs; glTF, FBX and OBJ usually bring their own.

`uv.*` commands go through `tequila_apply`; `tequila_uv_islands`, `tequila_uv_cut` and `tequila_uv_stitch` are tools of their own. The `uv_layout` request and the UV editor (`ui`) go through `tequila_request`.

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

Islands are named by number, from 0: step 6 reads which is which. `uv.pack` takes **all of them when its list is empty**; the others need the islands listed.

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

## 6. Which island is which, and seams where you mean them

Read the layout island by island before naming one:

```json
tequila_uv_islands {"node": 12}
```

It lists the islands largest first, each with:

- `island`: its number, what `uv.pack`, `uv.transform`, `uv.place_islands`, `uv.straighten` and the rest take;
- `box`: `[u0, v0, u1, v1]`, where it sits on the layout's square;
- `centre`, `size` and `faces`: where it lies **on the part**, in the world's metres, and the way it faces (`+y (up)`, `-z`…, or `round` for one that wraps the part, as a bottle's wall does). This is how you tell the front panel from the base: compare with the part's own centre and size from `tequila_nodes`;
- `of_part` and `area_cm2`: how much of the part it is; `stretch` (1 is none, 1.2 shows on a label) and `density` (1 is as fine as the rest).

A layout of hundreds of islands comes a page at a time (`"page": 2`). So "move the front panel's island to the top left, a third of the square":

```json
{"command": "uv.pack", "node": 12, "set": 0, "islands": [4], "margin": 0.01, "rotate": false, "normalize": false, "orient": false, "region": [0, 0.66, 0.34, 1]}
```

**Cut a seam** by saying where it runs on the part: a `path` of two or more points on its surface, in the world's metres. The cut follows the part's own edges the shortest way from each point to the next.

```json
tequila_uv_cut {"node": 12, "path": [[0.0, 0.21, -0.035], [0.0, 0.02, -0.035]], "pack": true}
tequila_uv_cut {"node": 12, "sharper_than": 40, "islands": [0]}
```

- The first cuts down the back of a bottle 7 cm across, from near its top to near its foot: work the points out from the part's centre and size.
- A seam runs along the mesh's own edges, so each point is taken to mean the nearest point of the mesh. A round wall has edges all the way down it. The middle of a flat face of a CAD model has none - the face is a few large triangles - and a path across it is refused, saying so: cut along the face's edges instead (`sharper_than`), or give the mesh more edges first with `tequila_improve {"node": 12, "subdivide": 1}` (page `fixing`) and cut again.
- To divide an island the cut runs from one edge of it to another: a path that stops just short of the edge is carried on to it. `"across": false` cuts between the points exactly, for a slit that lets a piece lie flatter without dividing it.
- `sharper_than` cuts every edge inside an island that is sharper than that many degrees: a box's edges, a cylinder's rims. `islands` narrows it to some.
- The pieces are laid flat again where they are; `"pack": true` packs the whole layout after. One undo step.
- It answers with `islands_before` and `islands_after`. **The islands are numbered anew by a cut: read them again before naming one.**

**Stitch** islands back together along the seams between them:

```json
tequila_uv_stitch {"node": 12, "islands": [2, 5]}
```

The smaller is turned and moved onto the larger. Islands that do not touch on the part are refused; follow with `uv.reflatten` if the joined island shows stretch. Take a picture of the layout after either (`tequila_screenshot {"window": "uv"}` with the editor open, step 1).

Still for the person, in the editor: pinning vertices and dragging single points of a layout (`uv.set_pins`, `uv.set_positions` name vertices of the mesh, which no tool lists). Open the editor on the part and say what to do there.

## Good to know

- A texture that looks stretched or smeared on a part with its own UVs: look at the heatmap before unwrapping again, since unwrapping replaces what artwork was drawn for.
- A label or texture that lands on the wrong piece: the layout has many islands; a larger `crease_degrees` joins them.
- Unwrapping a very dense part (hundreds of thousands of triangles) takes a few seconds.
- UVs are kept with the part in the project, and exported with it (glTF, USD).
