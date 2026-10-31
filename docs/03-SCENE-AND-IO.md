# 03 — Scene Model, File Format, and I/O

The document model, the open `.tequila` file format, the import/export pipeline, LiveLinking, and web/XR output.

---

## 1. Principles

1. **The Scene Document is the single source of truth.** Renderer, UI, scripting, and exporters all read from it; only the command bus writes to it.
2. **The file format is open and documented.** This is a deliberate competitive advantage over closed formats: users can script against it, diff it in git, and build tooling on it. Publish the schema in `docs/file-format.md` and version it from day one.
3. **Nothing is destructive on import.** Exact CAD geometry is retained alongside the tessellation so it can be re-tessellated at any tolerance later.
4. **IDs are stable across sessions.** A part's ID must survive save/load, re-import of an updated CAD file (where possible), and LiveLinking updates — otherwise material assignments break every time the model changes, which is the number-one workflow failure in CAD visualization.

---

## 2. Scene Document

### 2.1 Core structures

```rust
struct SceneDocument {
    meta:        DocumentMeta,        // format version, app version, created/modified, units
    tree:        SceneTree,           // hierarchy of nodes
    geometry:    GeometryStore,       // meshes + retained BREP, deduplicated
    materials:   MaterialStore,       // material instances (graphs), §02
    environment: Environment,         // §02 of master, §01 §5.2
    lights:      Vec<Light>,          // scene lights (non-material lights)
    cameras:     Vec<Camera>,
    model_sets:  Vec<ModelSet>,
    studios:     Vec<Studio>,
    multi_materials: Vec<MultiMaterial>,
    animation:   AnimationDocument,
    configurator: Option<ConfiguratorDocument>,
    render:      RenderSettings,
    image_style: ImageStyleStack,
    units:       SceneUnits,
}

struct SceneNode {
    id:        NodeId,            // stable, u64, never reused within a document
    parent:    Option<NodeId>,
    name:      String,
    kind:      NodeKind,
    transform: Transform,          // local TRS relative to parent
    pivots:    Vec<Pivot>,         // custom pivots (§2.4)
    visible:   bool,
    locked:    bool,
    material:  Option<MaterialRef>,   // or MultiMaterialRef
    labels:    Vec<LabelLayer>,       // §02 §7
    metadata:  HashMap<String, Value>,  // CAD metadata, user attributes
    children:  Vec<NodeId>,
}

enum NodeKind {
    Group,
    Part { geometry: GeometryRef },   // GeometryRef may be shared → instancing
    Light { light: LightId },
    CameraAnchor { camera: CameraId },
    CutawayVolume { shape: CutShape },
}
```

### 2.2 Geometry store and instancing

```rust
struct GeometryStore {
    meshes: SlotMap<MeshId, Mesh>,          // deduplicated by content hash
    breps:  SlotMap<BrepId, BrepHandle>,    // retained exact geometry (OCCT)
    links:  HashMap<MeshId, Option<BrepId>>,
}

struct Mesh {
    positions:  Vec<Vec3>,
    normals:    Vec<Vec3>,
    uvs:        Vec<Vec<Vec2>>,      // multiple UV sets
    tangents:   Option<Vec<Vec4>>,   // MikkTSpace, generated if a normal map needs them
    colors:     Option<Vec<Vec4>>,
    indices:    Vec<u32>,
    groups:     Vec<SubMesh>,        // per-face-group material slots
    tessellation_params: Option<TessellationParams>,  // if derived from BREP
}
```

**Instancing is a first-class concept, not an optimization applied later.** Multiple `Part` nodes referencing one `MeshId` render as BVH instances. A CAD assembly with 4,000 identical M3 screws must load as one mesh and 4,000 transforms; the difference is 3 seconds versus 3 minutes and 200 MB versus 8 GB.

Deduplicate on import by hashing `(positions, indices, normals)` after transform normalization — many CAD exporters write repeated geometry as distinct meshes and rely on the consumer to detect it.

### 2.3 Stable IDs and re-import

`NodeId` is assigned on creation and never reused. Additionally maintain a **persistent path key** per node: the tuple `(parent path, name, occurrence index)` from the source CAD. On re-import of an updated version of the same file:

1. Match new nodes to existing ones by path key.
2. Preserve material assignments, labels, animation tracks, and per-node settings on matched nodes.
3. Report unmatched (added/removed/renamed) nodes to the user in a reconciliation dialog rather than silently discarding work.

This is the single highest-value quality-of-life feature for the target user, whose CAD model changes weekly. Build it in M8 alongside CAD import, not later.

*As built* (`tequila-scene/src/reimport.rs`): a model with one top node keys that node as `(model)` rather than by its name, because CAD packages name it after the document and the document is renamed on every save-as. A node the key does not match is matched by name when the name is unique among the unmatched on both sides, first as written and then without a trailing copy number (`Bottom v3 (1)`). The model's top nodes keep their placement; every other matched node takes the file's. The file is recorded on the top nodes as the `Source file` attribute.

### 2.4 Pivots

```rust
struct Pivot { id, name, transform: Transform, mode: PivotMode }
enum PivotMode { Origin, BoundingBoxCenter, Corner, CylinderAxis, SphereCenter, Custom }
```

Corner / Cylinder / Sphere modes derive the pivot from picked geometry: Corner snaps to a vertex or a box corner of the selection; Cylinder fits a cylinder axis to the picked face (least-squares over face normals); Sphere fits a sphere centre. Multiple pivots per node, each usable by a different animation track.

---

## 3. Scene units — get this right or physics is meaningless

### 3.1 Model

```rust
struct SceneUnits {
    unit: Unit,                 // Millimeter | Centimeter | Meter | Inch | Foot | Unspecified
}
```

Internally, **all geometry is stored in metres** and `unit` is a display/authoring preference: lengths are shown and typed in it, and a scene with no unit shows metres. This avoids the accumulating precision and conversion errors that come from storing in the user's unit, and it is what the renderers, the materials' real sizes and the importer all assume.

Documents written before 2026-10-13 also carried a `meters_per_unit` factor, which the readouts honoured and the renderer never read, so a millimetre scene rendered a thousand times its size. A file whose factor is not 1 is scaled into metres when it is opened, as Scale Scene would scale it.

### 3.2 What units affect

| Affected by scene scale | Why |
|---|---|
| Light intensity | Inverse-square falloff: a light 1 unit away is 1 mm or 1 m away depending on units |
| Volume absorption / `transmission_depth` | Beer-Lambert is per unit length |
| Subsurface radius | Mean free path is a length |
| Texture "real-world size" mapping | mm-based texture scale |
| Depth of field | Focus distance and aperture in physical units |
| Physics | Gravity, mass, collision margins |
| Camera near/far and ray epsilon | Precision scaling |

### 3.3 The two conversion modes

Both are required (KeyShot documents both, and users need both):

- **Keep Scene Size** — the model's physical dimensions stay correct; only the unit they are shown in changes. Nothing in the scene moves, so the render is exactly what it was. Use when the model is the right size.
- **Scale Scene** — the model's numeric dimensions stay, read in the new unit, so the scene is rescaled: geometry, lights, cameras, the ground and the dome together, light power by the square, so the shot keeps its framing and brightness. Materials keep their real sizes, which recalibrates textures and mapping against the model. Use when the model came in at the wrong unit and its textures are the wrong scale on it.

Implement both explicitly, with a preview of what will change, and make it undoable as a single command.

### 3.4 Import unit detection

STEP and IGES carry units in the header — honour them. OBJ and STL carry none; prompt on import with a sensible guess from the bounding box (a 0.05-unit object is probably metres; a 50,000-unit object is probably millimetres) and remember the choice per file extension.

---

## 4. The `.tequila` file format

### 4.1 Container

A ZIP archive (stored, not compressed, for large binary members so memory-mapping works):

```
project.tequila
├── manifest.json          # format version, app version, integrity hashes, asset index
├── scene.json             # SceneDocument minus heavy binary data — human-readable
├── materials/
│   └── <material-id>.json # one material graph per file
├── geometry/
│   ├── <mesh-id>.fsmesh   # binary mesh buffers
│   └── <brep-id>.brep     # OCCT-serialized exact geometry (optional)
├── environments/
│   └── <env-id>.fsenv     # §5
├── textures/              # embedded textures (optional; may be external references)
├── labels/
├── animation.json
├── thumbnails/
└── previews/preview.png   # scene thumbnail for browsers
```

Rationale for splitting materials and geometry out of `scene.json`: a scene with 500 materials stays diffable, and a change to one material produces a one-file diff in git rather than rewriting a monolithic document.

### 4.2 `scene.json` shape (abbreviated schema)

```json
{
  "format_version": "1.0.0",
  "units": { "unit": "Millimeter", "meters_per_unit": 0.001 },
  "tree": [
    { "id": 1, "name": "Assembly", "kind": "Group", "parent": null,
      "transform": { "t": [0,0,0], "r": [0,0,0,1], "s": [1,1,1] },
      "visible": true, "children": [2,3] },
    { "id": 2, "name": "Housing", "kind": "Part", "parent": 1,
      "geometry": { "mesh": "m_a91f", "brep": "b_a91f" },
      "material": "mat_004",
      "labels": [ { "source": {"texture": "labels/logo.png"},
                    "projection": "Planar",
                    "transform": { "position": [12.0, 4.0, 30.0],
                                   "rotation": 15.0, "scale": [20.0, 8.0] },
                    "material_type": "Paint" } ],
      "metadata": { "PartNumber": "HS-4410", "Material": "ABS", "Mass_g": "42.1" },
      "visible": true, "children": [] }
  ],
  "materials_index": ["mat_001", "mat_004"],
  "environment": { "ref": "env_studio_01", "rotation": 0.35, "brightness": 1.2,
                   "contrast": 1.0, "size": 8000.0, "height": 0.0,
                   "background": { "mode": "Backplate", "image": "textures/bp.jpg" },
                   "ground": { "shadows": true, "occlusion_shadows": false,
                               "reflections": 0.4, "flatten": true, "size": 4000.0 } },
  "cameras": [ { "id": "cam_1", "name": "Hero", "type": "Perspective",
                 "position": [400,220,600], "target": [0,40,0], "up": [0,1,0],
                 "focal_length_mm": 85, "sensor_width_mm": 36,
                 "dof": { "enabled": true, "f_number": 2.8,
                          "focus_distance": 720.0, "blades": 7, "keep_focus": true } } ],
  "lighting_preset": "Product",
  "render": { "max_samples": 512, "denoise": true, "resolution": [3840,2160],
              "passes": ["Beauty","Depth","Cryptomatte","Clown"] },
  "image_style": { "type": "Photographic", "exposure": 0.0, "highlights": -0.2,
                   "shadows": 0.1, "temperature": 6500, "vignette": 0.15,
                   "display_transform": "AgX" },
  "model_sets": [...], "studios": [...], "multi_materials": [...]
}
```

Publish the full JSON Schema at `docs/file-format.md` and validate on load. Reject-with-explanation beats silently misparsing.

### 4.3 `.fsmesh` binary layout

```
magic "FSMESH\0\0"  u64
version             u32
vertex_count        u32
index_count         u32
flags               u32   # bit 0 normals, 1 tangents, 2 colors, 3+ uv_set_count
uv_set_count        u32
bounds_min/max      f32[6]
positions           f32[3 × vertex_count]
normals             f32[3 × vertex_count]   (if present)
tangents            f32[4 × vertex_count]   (if present)
uv_sets             f32[2 × vertex_count] × uv_set_count
colors              f32[4 × vertex_count]   (if present)
indices             u32[index_count]
submesh_count       u32
submeshes           { first_index u32, index_count u32, slot u32 }[]
```

Fixed-layout and aligned so it can be memory-mapped and handed to Embree without a copy.

### 4.4 Versioning and migration

`format_version` is semver. Loading rules:
- Same major: load, migrate forward silently, log what was migrated.
- Newer minor: load, warn about ignored unknown fields, **preserve them on save** (round-trip unknown data — this is what lets older builds not destroy newer projects).
- Newer major: refuse with a clear message.

Write a migration function per version bump and a test that loads every historical sample scene in `tests/scenes/`.

### 4.5 Auto-save and crash recovery

Journal every command (§04 §8) to `~/.tequila/journal/<session>.jsonl` as it is applied. On crash, offer to replay the journal onto the last saved state. Auto-save a full snapshot every N minutes (configurable, default 5) to a rotating set of files. This is cheap to build on top of the command bus and it saves users' work — which matters far more than it appears on a feature list.

---

## 5. `.fsenv` — environment container

The HDRI Editor's pins, transforms, and adjustments must travel with the scene (KeyShot embeds them in the project; do the same). `.fsenv` is a ZIP:

```
environment.fsenv
├── environment.json    # base HDRI reference, transform, adjustments, pin stack
├── base.exr            # the source HDRI (optional if referencing an external library asset)
└── pins/               # image-pin source images
```

```json
{ "base": { "file": "base.exr", "rotation": 0.0, "brightness": 1.0, "contrast": 1.0 },
  "sun_sky": { "enabled": true, "elevation": 42.0, "azimuth": 130.0,
               "turbidity": 3.0, "sun_size": 1.0, "ground_color": [0.2,0.2,0.2] },
  "pins": [
    { "type": "Light", "shape": "Rect", "position_uv": [0.25, 0.40],
      "size": [0.30, 0.18], "corner_radius": 0.05, "rotation": 0.0,
      "color": [1,1,1], "intensity": 4.0, "falloff": 1.4, "enabled": true },
    { "type": "Gradient", "stops": [ {"t":0.0,"c":[0.02,0.03,0.05]},
                                     {"t":1.0,"c":[0.4,0.45,0.5]} ],
      "affects": "BackgroundOnly" },
    { "type": "Image", "file": "pins/window.exr", "position_uv": [0.7,0.35],
      "size": [0.2,0.25], "intensity": 2.0, "color_adjust": {"saturation": 0.8} }
  ] }
```

Compositing order is array order. `affects` is one of `LightingAndBackground | LightingOnly | BackgroundOnly` — the separation that makes gradient pins useful.

Export to plain `.hdr`/`.exr` by rasterizing the composite at a chosen resolution.

---

## 6. Import pipeline

### 6.1 Mesh formats (Assimp) — M1

OBJ, STL, PLY, 3DS, FBX, Collada, 3MF, glTF/GLB.

Post-processing pipeline, in order:
1. Triangulate.
2. **Join identical vertices** (Assimp `JoinIdenticalVertices`) — CAD-derived meshes are often fully unwelded.
3. Generate normals **only if absent**, with a smoothing angle (default 30°, user-adjustable per import). Never overwrite authored normals — CAD tessellation carries correct normals and regenerating them makes cylinders faceted.
4. Generate MikkTSpace tangents when UVs exist.
5. Validate: remove degenerate triangles, report NaN/Inf positions rather than importing them.
6. Deduplicate meshes by content hash → instancing (§6.4).
7. Apply unit conversion (§3.4).
8. Import materials: map source material parameters to OpenPBR where possible. **glTF and USD carry OpenPBR/PBR-metallic-roughness parameters directly — map them exactly, do not approximate.** FBX/OBJ carry Phong-family parameters; convert with a documented, tested mapping (specular exponent → roughness via `roughness = sqrt(2/(exponent+2))`).

### 6.2 CAD BREP (OCCT) — M8

STEP, IGES, BREP; optionally ACIS/JT/Parasolid if those kernels become available.

1. Read with `STEPControl_Reader` / `IGESControl_Reader`; read the assembly structure via XDE (`XCAFDoc`) so hierarchy, names, colours, and metadata survive — reading with the basic reader loses all of that and produces one flat blob, which is the difference between a usable import and a useless one.
2. Extract per-shape colour and layer assignments from XDE and use them to seed material assignment (this is what makes "Material Templates by CAD colour" possible, §02).
3. Extract metadata/attributes → `node.metadata`. *Done 2026-10-20* (`native/occt/tequila_occt.cpp`, `crates/tequila-import/src/step.rs`, both readers): the product's part number, description and release; properties of the user's own by their names; the material it names; each solid's layers as `Layer`. A solid whose layers are all switched off arrives hidden. *2026-10-30:* layers have a list of their own on each node (`node.layers`, in the file's order), which the Scene tab's Layers list shows and hides as groups (`scene.set_layer_visibility`); a search and a Material Templates rule still read them as the field `Layer`. IGES levels arrive as layers named `Level 5`, by Tequila's own reader and by `IGESCAFControl_Reader`, which reads an IGES file Tequila's reader cannot read whole.
4. **Tessellate** with `BRepMesh_IncrementalMesh` using:
   - **linear deflection** = `bounding_box_diagonal × relative_tolerance` (default relative 0.0005; expose as Low/Medium/High/Custom),
   - **angular deflection** default 0.35 rad (~20°), tightened to 0.1 for High,
   - relative mode on, parallel meshing on.
5. **Retain the `TopoDS_Shape`** in the geometry store so Re-Tessellate can regenerate at a different tolerance without re-reading the file. Serialize it into the project via `BinTools`/`BRepTools` for `.brep` members.
6. Extract **BREP edges** as polylines for the Wireframe material and for edge-based measurement snapping (§02 §3.3) — tessellated triangle edges are the wrong thing to draw.

Tessellation quality presets, exposed in the import dialog and in the Re-Tessellate tool:

| Preset | Relative linear deflection | Angular deflection |
|---|---|---|
| Draft | 0.005 | 0.6 rad |
| Medium | 0.001 | 0.35 rad |
| High | 0.0005 | 0.2 rad |
| Ultra | 0.0001 | 0.1 rad |

### 6.3 USD — M8

Import and **export**. USD is the interchange format that matters going forward; supporting it well makes Tequila a good pipeline citizen. Map: `UsdGeomMesh`/`UsdGeomXform` → scene tree; `UsdShade` + **MaterialX/OpenPBR** → material graphs (near-lossless in both directions since Tequila's shading model *is* OpenPBR); `UsdLux` → lights; `UsdGeomCamera` → cameras. Support `.usdz` for delivery.

Instancing: honour `UsdGeomPointInstancer` and scene-graph instancing (`instanceable`) natively.

### 6.4 Instancing detection

For formats that do not express instancing, detect it: hash each mesh's `(positions, indices)` after translating to its centroid; identical hashes with differing transforms become instances. Guard against false positives by comparing full buffers on hash match.

### 6.5 CAD metadata and CSV export

Retain all imported attributes per node. Provide:
- A metadata panel in the Scene tab.
- Filtering/selection by metadata (`select all parts where Material == "ABS"`) — very useful for bulk material assignment.
- **CSV export** of the scene tree with selected metadata columns (matching KeyShot 2026.1's CAD data export).

### 6.6 Export

| Format | Purpose |
|---|---|
| USD / USDZ | Full scene interchange (preferred) |
| glTF / GLB | Web delivery, and the Configurator's web export payload |
| OBJ, STL, PLY | Simple mesh handoff |
| Alembic | Baked animation handoff |
| `.fsenv`, `.hdr`, `.exr` | Environments |
| CSV | Scene metadata |

---

## 7. LiveLinking

Push geometry and material-assignment updates from a running CAD/DCC application into a running Tequila session without a manual export/import cycle.

### 7.1 Transport

Local IPC — a Unix domain socket / named pipe at a well-known path, with a simple length-prefixed message protocol (MessagePack or CBOR payloads). Not HTTP: latency matters and the endpoint is local-only by design (do not open a network port; it is a security liability for no benefit).

### 7.2 Protocol

```
→ HELLO      { client: "blender", version, protocol: 1 }
← HELLO_ACK  { session_id, scene_id, capabilities: [...] }

→ SYNC_BEGIN { transaction_id }
→ UPSERT_MESH  { key: "<stable source path>", positions, normals, uvs, indices, transform, units }
→ UPSERT_NODE  { key, parent_key, name, transform, metadata }
→ DELETE_NODE  { key }
→ SYNC_END   { transaction_id }
← SYNC_ACK   { transaction_id, matched: n, added: n, removed: n }

→ CAMERA_SYNC { position, target, focal_length }        # optional live camera mirroring
← PING / PONG heartbeat
```

**Key rule:** updates are matched by the `key` (the source application's stable identifier — Blender object name + library path, CAD occurrence path). Tequila preserves material assignments, labels, and animation on matched nodes (same mechanism as re-import, §2.3). This is the whole value of LiveLinking; a version that reassigns default materials on every sync is worse than useless.

The entire sync must arrive as one transaction and be applied as one command (one undo step, one render restart).

### 7.3 First client: Blender

Blender's Python API is fully open, which makes it the lowest-friction first target. Ship `integrations/blender/` as a Blender add-on that:
- registers a "Send to Tequila" operator and an auto-sync toggle,
- on depsgraph update, sends the changed objects only (not the whole scene),
- maps Blender's Principled BSDF → OpenPBR (a near-direct mapping; document the few parameters that differ),
- mirrors the Blender camera on request.

Subsequent clients (Rhino, Fusion 360, SolidWorks, SketchUp, Cinema 4D) follow the same protocol; the protocol document is `docs/livelink-protocol.md` so third parties can implement their own.

---

## 8. Spin-viewer (XR) export

Render a frame grid and package an embeddable viewer.

**Parameters:** horizontal steps (default 36 = 10° increments), vertical steps (default 1), zoom levels (default 1), rotation axis, direction, output resolution, samples per frame, format (JPEG/PNG/WebP), and whether to render with transparency.

**Rendering:** for each cell, position the camera on the orbit sphere around the camera target, render at the configured sample count. Reuse the render queue (§05); this is just a batch of stills with generated cameras.

**Packaging:** output a folder containing frames plus `index.html`, `viewer.js`, and a `manifest.json` describing the grid. The viewer is a small dependency-free JS module: preload a low-res sprite sheet for instant response, lazy-load full-res frames, support drag/touch/scroll, inertia, auto-rotate, and a "reset view" control. Provide an `<iframe>` embed snippet and an npm-installable web component for people integrating into a storefront.

Also emit a **glTF/GLB alternative** for the same scene when geometry and materials are simple enough to render live in-browser — a real-time viewer beats a frame grid where it works, and Tequila can offer both.

---

## 9. Configurator web export

**Payload:** a glTF/GLB of the scene (or per-model-set GLBs), a JSON configuration describing options (models, materials, studios) and their valid combinations, plus either (a) pre-rendered images for every combination, or (b) live three.js rendering with an IBL environment.

**Two output modes:**
- **Pre-rendered** — highest fidelity, exactly matches Tequila's path-traced output. Combination count grows multiplicatively; warn the user when the batch exceeds a threshold and show the estimated render time before starting.
- **Live 3D** — interactive camera and instant switching; approximate PBR shading in-browser. Because Tequila's materials are OpenPBR, the glTF export maps cleanly onto three.js's physical material, so the live preview is a genuine approximation rather than a guess.

Offer both from the same wizard; users typically want live 3D for the configurator and pre-rendered stills for marketing.

**Output:** a static folder that can be dropped on any host — no server requirement, no build step, no CDN dependency.
