# Pictures on materials: maps, texture packs, Substance

Use pictures only when the person gives files: the library's presets are procedural and need none. A logo or a print on a part is a label (page `labels`), not a texture.

Tequila reads PNG, JPEG, TIFF, TGA, BMP, WebP, GIF, JPEG XL, DDS, PSD/PSB, SVG, EXR and HDR. HEIC and AVIF are not read: ask for another format.

## 1. One picture on one parameter

```json
{"command": "material.set_texture", "material": 3, "param": "base_color", "path": "C:/work/maps/panel_color.png", "role": "color", "color_space": "srgb"}
{"command": "material.set_texture", "material": 3, "param": "specular_roughness", "path": "C:/work/maps/panel_rough.png", "role": "data", "color_space": "raw"}
{"command": "material.set_texture", "material": 3, "param": "bump", "path": "C:/work/maps/panel_normal.png", "role": "normal", "color_space": "raw"}
```

- `param` is the parameter the picture drives: `base_color`, `specular_roughness`, `base_metalness`, `bump`, `geometry_opacity`, `emission_color`…
- `role` says what kind of picture it is: `color` (a picture of colours), `data` (numbers: roughness, metalness, opacity), `normal` (a normal map), `height` (a height or displacement map). `color_space` is `srgb` for colours people painted, `raw` for data and normals, `linear_srgb` for linear colour.
- A path that is not absolute is beside the project. Give the full path the person gave.
- One undo step. To stop driving the parameter, `material.set_texture` with `"path": null` and the same `param`: the plain value comes back.

## 2. How it lies on the surface

A new texture uses the part's UVs when it has them. Change how it is laid on with the mapping of that parameter - the whole mapping is sent, so start from the example `tequila_command {"command": "material.set_mapping"}` gives and change what you need:

```json
{"command": "material.set_mapping", "material": 3, "param": "base_color", "projection": "tri_planar", "scale": [0.1, 0.1, 1], "rotation": 0, "offset": [0, 0], "pivot": [0, 0], "axis": "z", "blend_sharpness": 6, "flip_u": false, "flip_v": false, "real_world_units": true, "rigid": false, "uv_set": 0, "wrap_u": "repeat", "wrap_v": "repeat"}
```

| Field | Means |
|---|---|
| `projection` | `uv` (the part's own layout), `planar` (projected flat along `axis`), `cylindrical`, `spherical`, `tri_planar` (from three sides, blended: the one that works on a part with no UVs) |
| `scale` | The size of one repeat. With `real_world_units` true it is metres on the part: `[0.1, 0.1, 1]` is a 10 cm tile |
| `rotation` | In turns: 0.25 is a quarter turn |
| `offset`, `pivot` | Slide the picture; the point it turns and scales about |
| `wrap_u`, `wrap_v` | `repeat`, `clamp` (once, its edge stretched), `mirror` |
| `uv_set` | Which of the part's UV sets, for `uv` |
| `blend_sharpness` | How hard `tri_planar` switches between its three sides |

If a picture shows as one flat colour or as stretched streaks, the part has no usable UVs: use `tri_planar`, or unwrap the part (page `uv`). If it shows far too small or large, check `scale` against the part's `size` from `tequila_nodes`, and the model's units (page `fixing`).

## 3. A whole pack of PBR maps

A folder or ZIP of maps named in the usual way (`Wood049_Color.jpg`, `wood_rough_2k.png`, `T_Wood_Normal.png`; colour, roughness, metalness, normal, height, opacity, ambient occlusion) becomes a finished material in one request. This reads files, so it goes through `tequila_request`, and inside the Assistant the person is asked first:

```json
tequila_request {"request": {"op": "texture_set", "path": "C:/packs/Wood049_2K", "nodes": [12, 13]}}
```

- `path` is the folder, the ZIP, or any one of its maps. `nodes` are the parts to put the first material on; left out, the selection.
- The answer lists the `materials` it made, each with the `maps` it found and which slot each went to, and how many parts it was `assigned_to`. A pack with several materials in it makes them all.
- "no texture maps found" means the file names give no hint of what each map is. Then set them one at a time as in step 1.

To add a pack to the person's **library** instead of the scene, so it is there for every project: `{"op": "library", "action": "import", "path": "C:/packs/Wood049_2K", "size": 0.5, "name": "Oak Planks", "category": "Wood"}`, where `size` is how many metres one repeat of the picture covers.

## 4. Check it

`tequila_ui {"viewport": "rendered"}` and `tequila_screenshot`. `tequila_materials` shows how many `textures` each material has; `tequila_read {"what": "textures"}` lists every picture file the project uses with its id, path and role.

## 5. Substance materials (.sbsar)

A Substance file is a material whose maps are computed from inputs (a colour, an amount of wear). Tequila opens them and changes their inputs; computing new maps needs Adobe's `sbsrender` on the person's computer, which comes with Substance 3D Designer or the Automation Toolkit. All of it goes through `tequila_request`:

```json
tequila_request {"request": {"op": "substance", "action": "tools"}}
tequila_request {"request": {"op": "substance", "action": "info", "path": "C:/work/bricks.sbsar"}}
tequila_request {"request": {"op": "substance", "action": "open", "path": "C:/work/bricks.sbsar", "nodes": [12]}}
tequila_request {"request": {"op": "substance", "action": "set", "material": 4, "values": {"brick_color": "0.6,0.25,0.15,1"}}}
tequila_request {"request": {"op": "substance", "action": "wait", "material": 4}}
```

1. `tools` says whether maps can be computed here (`can_render`) and, if not, what to install. Say so before promising a change.
2. `info` describes a file without opening it: its graphs, and each input with its `identifier`, type, default and range.
3. `open` makes materials from it and puts the first on `nodes`.
4. `set` changes inputs by their `identifier`; values are written as the file writes them (numbers joined by commas for a colour or a size). `"reset": true` puts every input back.
5. `wait` answers when the new maps are in; `status` says where they are without waiting (`rendered`, `rendering`, `no_tools`, `failed` with why).

Without `sbsrender`, a Substance material keeps the maps it came with and its inputs cannot change the look.

## Good to know

- A texture is shared by its file: two materials using one picture load it once.
- Very large maps cost graphics memory in the Rendered view; 4K is plenty for a product part.
- A picture with transparency used as `base_color` does not cut the surface: drive `geometry_opacity` with the same file (role `data`) for cut-outs.
- Video files (MP4, MOV) work for labels, one frame of them (page `labels`).
