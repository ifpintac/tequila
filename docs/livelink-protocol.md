# LiveLink protocol

LiveLink pushes a modelling application's scene into a running Tequila as it
changes (`docs/03-SCENE-AND-IO.md` §7). This document is for anyone writing a
client; Tequila's own is the Blender add-on in
`integrations/blender/tequila_livelink`.

## The one rule

**An object Tequila has seen before keeps everything done to it in
Tequila.** Its material, labels, visibility, name and model sets stay; its
geometry, placement and attributes come from the client, which is the truth
about those. A sync that reassigned materials would be worse than no
LiveLink. A client's material is used once, when an object first arrives.

## Transport

LiveLink is part of Tequila's scripting channel, so a client needs nothing
else to be running:

1. Read `~/.tequila/channel.json`. It holds `port`, `token` and `pid` while
   Tequila is open. If it is missing, Tequila is not running.
2. Connect a TCP socket to `127.0.0.1:<port>`. The channel listens on the
   loopback address only; nothing outside this computer can reach it.
3. Send one JSON object per line (UTF-8, ending `\n`), each with an `id` you
   choose, the `token`, and `"op": "livelink"`. Read one line back per
   request: `{"id", "ok": true, "result": {...}}`, or `"ok": false` with an
   `error` saying why.

Requests on one connection are answered in order. Keep the connection open
between syncs; reconnect (re-reading the file) if it closes, since Tequila
may have been restarted on a new port.

`docs/03` §7.1 asked for a named pipe instead of a port. The channel was
already a token-guarded loopback socket, which is no wider than a pipe in
practice, and a second listener would be a second thing to secure.

## Messages

Every request carries `"message"` naming which one it is.

### hello

```json
{"op": "livelink", "message": "hello", "client": "blender", "version": "5.0.0", "protocol": 1, "id": 1, "token": "…"}
```

Answered with `{"protocol": 1, "session": <pid>, "scene": "<document name>",
"capabilities": ["sync", "camera", "materials", "deletes"]}`. A client should
send it first and stop if `protocol` is not one it speaks.

### sync

One transaction: applied as **one undo step** and one render restart.

```json
{
  "op": "livelink", "message": "sync", "id": 2, "token": "…",
  "source": "blender:C:/models/kettle.blend",
  "label": "Blender · kettle",
  "transaction": 7,
  "complete": false,
  "upserts": [
    {
      "key": "Body",
      "name": "Body",
      "parent": null,
      "matrix": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0.1,0, 1],
      "mesh": {
        "positions": "<base64>",
        "normals": "<base64>",
        "uvs": "<base64>",
        "indices": "<base64>"
      },
      "material": {"name": "Red plastic", "base_color": [0.8, 0.05, 0.03], "roughness": 0.3},
      "metadata": {"Blender object": "Body"}
    }
  ],
  "deletes": ["Old handle"]
}
```

- `source` names the client and its document. Everything it sends goes under
  one group of its own, made on the first sync and found again by this name
  (stored as the group's `LiveLink` metadata). Two documents are two groups.
- `label` is what that group is called when it is made.
- `key` is the client's **stable** name for the object - Blender's object
  name, with its library path when linked; a CAD occurrence path. Matching
  is by key alone. Renaming an object in the client is a delete and an add.
- `parent` is another object's key, to hang under it; absent or `null`, the
  object hangs under the source's group. A parent sent in the same sync, in
  any order, is fine.
- `matrix` is the object's placement relative to its parent (or the group):
  sixteen numbers, **column-major** as glTF writes them, in **Tequila's
  axes: Y up, metres**. A Z-up client converts (Blender:
  `(x, y, z) → (x, z, -y)`, times the scene's unit scale).
- `mesh` is the geometry in the object's own space: little-endian `f32`
  positions (xyz) and normals (xyz), `f32` UVs (uv) with **v running down the
  image** as glTF's does, and `u32` indices, each base64-encoded. Indices may
  be left out to mean one vertex per corner, in order. Sending one vertex per
  triangle corner is the simple way to carry split normals and UV seams;
  Tequila shares identical corners again. Leave `mesh` out to move an object
  without sending its geometry again. Geometry identical to what Tequila has
  is not stored twice.
- `material` is used only when the object is new. Its fields are the
  Principled BSDF's: `base_color` (linear RGB), `metallic`, `roughness`,
  `ior`, `transmission`, `alpha`, `emission` (linear RGB) and
  `emission_strength`, `coat`, `coat_roughness`, `sheen`, `sheen_tint`. All
  optional.
- `metadata` is string fields, shown in Tequila's Metadata panel, searchable
  and exportable.
- `deletes` lists keys the client no longer has. Tequila **hides** them
  rather than removing them, so a renamed object does not take its finish
  with it; the user deletes what is really gone.
- `complete: true` says the upserts are everything the client has, so any key
  it sent before and not now is treated as deleted. A client that does not
  remember what it sent - one just restarted - should make its full sends
  complete.

Answered with `{"transaction", "matched", "added", "removed"}`.

### camera

```json
{"op": "livelink", "message": "camera", "position": [7, 5, 7], "target": [0, 0.5, 0], "focal_length": 50, "id": 3, "token": "…"}
```

Moves Tequila's view, in Tequila's axes and metres, focal length in
millimetres on a 36 mm sensor. Mirroring a camera is a stream of these; send
one only when the camera has moved.

## Principled BSDF → OpenPBR

| Principled | OpenPBR in Tequila |
|---|---|
| Base Color | `base_color` (weight 1) |
| Metallic | `base_metalness`; above 0.5 the material is made as Metal |
| Roughness | `specular_roughness` |
| IOR | `specular_ior` |
| Transmission Weight | `transmission_weight`, tinted by Base Color as Blender does; above 0.5 the material is made as Glass |
| Alpha | `geometry_opacity` |
| Coat Weight, Coat Roughness | `coat_weight`, `coat_roughness` |
| Sheen Weight, Sheen Tint | `fuzz_weight`, `fuzz_color` |
| Emission Color × Strength | `emission_color`, `emission_luminance` = strength × 100 nits |

What differs: Blender's emission strength is relative, OpenPBR's luminance
absolute, so strength 1 is taken as a display's white, 100 nits. Texture
inputs are not followed: a material arrives with its socket values, and a
textured finish is the kind of thing that is then done in Tequila.

## The Blender add-on

Install `integrations/blender/tequila_livelink` (zip the folder; it is a
Blender 4.2+ extension and a legacy add-on). The 3D Viewport's sidebar gets a
**Tequila** tab:

- **Send to Tequila** sends the visible meshes, modifiers applied, as a
  complete sync; **Send selected** only the selection.
- **Auto sync** sends each change a quarter second after the last one: moved
  objects without their geometry, edited ones with it, removed ones as
  deletes.
- **Mirror camera** moves Tequila's view with the scene camera.
