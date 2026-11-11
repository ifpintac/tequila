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

*As built, November 5:* the first embedded textures are a Substance material's rendered maps (`substance/<key>/<output>.png`, `docs/file-format.md` §1 and §4.2), written by `tequila_format::archive` from the textures a material's `substance.maps` names and put back into the render cache on load (`read_into`, `load_into`), so a project with Substance materials opens and renders the same on a computer without Adobe's tools. Every other texture is still a path.

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

*As built, November 6:* the folder is `tequila_format::library::home_directory()/journal`, which is `~/.tequila/journal`, or `data/journal` beside the program in a portable copy (a `portable.txt` beside it; `docs/05-ROADMAP-AND-TESTING.md` §5). `TEQUILA_JOURNAL_DIR` still comes first. The crash reports, the render history and queue, and the scripting channel's discovery file follow the same folder.

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

*Which way up* (as built, November 9): Tequila is Y up, and a STEP, IGES, STL or OBJ file does not say which way up it was drawn; one drawn Z up arrived lying on its back, to be turned in the Scene tab (Turn, Upright) or exported again turned. **File › Up axis of imports** keeps the choice (`import.txt`, beside the other settings): as the file says (the default: glTF, USD and FBX say, the rest are taken as Y up), Y up, Z up, X up, or upside down on each axis. An import turns the model about the origin so that axis points up (`UpAxis::turn` in `tequila-import`; Z up with its front, −Y, toward +Z, where the default camera looks), before it is set on the floor, and the status line says it was stood up. An update of a model turns it the same way. A file with its own cameras and lighting is not turned, and a note says so: they would no longer match. A script's `import` takes `up` for that import alone (`file`, `y`, `z`, `x`, `-y`, `-z`, `-x`), as the AI tools' `tequila_import` does.

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

*As built, November 5: PLY, 3MF and FBX* (`crates/tequila-import/src/ply.rs`, `three_mf.rs`, `fbx.rs`; `tequila_import::MODEL_EXTENSIONS`, which the Import and Update dialogs' Models filter, dropping a file on the window, the CLI's `--input` and the scripting `import` all go by). Each reader hands the pipeline above a `RawScene`; welding, normals only where absent, tangents, instancing and units are the pipeline's, as for OBJ.
- **PLY** (`.ply`): ASCII, binary little- and big-endian, any of the eight number types under either name, the header followed element by element so unknown elements and properties are read past. Positions; normals (`nx ny nz`); texture coordinates (`s t`, `u v`, `texture_u texture_v`, flipped to the top-left origin) and MeshLab's per-corner `texcoord`, which splits a vertex where its corners differ; colours (`red green blue alpha`, bytes or floats, `diffuse_` spellings) as display values decoded to linear and shown through the vertex-colour multiply every format gets, dropped when white throughout. Faces of any size: fanned when convex, ears clipped in their own plane when not (`ply::triangulate_polygon`). MeshLab's `comment TextureFile` becomes the base colour map, found beside the model. No unit and no up axis: the unit is guessed (§3.4) and the model comes in as written. A file of points only is refused by name. Not read: triangle strips (said), per-face colours.
- **3MF** (`.3mf`): the package's start part through `_rels/.rels`, read through the crate's own ZIP reader and roxmltree (already in the tree through usvg). The model's unit (micron, read as millimetres a thousandth the size; mm, cm, inch, foot, metre); mesh objects and component objects to any depth, a cycle refused; build items and components with their 3x4 transforms; the production extension's `p:path`, by which Bambu Studio, Orca and PrusaSlicer keep objects in parts of their own. Base materials (name, `displaycolor` with alpha as opacity) and colour groups become plastic materials, roughness 0.45; a triangle whose corners take different colours gets vertex colours; texture groups become a material whose base colour is the package's picture, written out beside the model as a glTF's embedded images are, with the triangles' coordinates. An object of several looks is a group of a part per material, named after it; an object placed several times is one mesh. Z up turned Y up under a group named after the file (each part keeps its own Z as its build direction, as the FDM layer lines expect); the title and other `<metadata>` on that group, a build item's part number on its part. Support objects are left out and counted. Not read: the thumbnail, composite materials and multi-properties (their triangles come in plain, said), beam lattices, slices.
- **FBX** (`.fbx`), through ufbx 0.23 (the `ufbx` crate 0.11, MIT OR Unlicense; `docs/05` M1 note 2 records why): binary and ASCII. ufbx converts any axis system to Y up (`AdjustTransforms`), bakes geometric transforms into meshes and compensates Maya's segment scale; `UnitScaleFactor` is the source unit (mm, cm, m, inch, foot; any other is applied into metres by the reader). The node hierarchy with each node's transform, under a group named after the file when there are several top nodes; nodes with no mesh at or below them (cameras, lights, bones, empties) are left out. A mesh placed by several nodes is one mesh; a mesh of several materials is a group of a part per material, a node's own materials winning over its mesh's. Normals, the first UV set (flipped), vertex colours (decoded from display values, dropped when white). Materials: Lambert and Phong by the mapping of point 8 — base colour `DiffuseColor × DiffuseFactor`, `roughness = sqrt(2/(ShininessExponent + 2))`, a Lambert diffuse alone (type Diffuse), no highlight where `SpecularColor × SpecularFactor` is black, emission `EmissiveColor` as bright as `EmissiveFactor`, opacity from `Opacity` or one less `TransparentColor × TransparencyFactor`; maps for diffuse colour, normal, bump (a normal map when its name says so), specular colour and emission, others named in a warning. The physically based shaders ufbx reads — Autodesk Standard Surface, Arnold, 3ds Max Physical and PBR, Stingray PBS, glTF and OpenPBR materials, and Blender's export, whose Phong ufbx reads back as the Principled BSDF it was — map one to one: base weight and colour, metalness, roughness (glossiness inverted, by node for a map), specular weight, colour and IOR, anisotropy, transmission, coat, sheen as fuzz, thin film, subsurface weight and colour, emission, opacity, thin-walled; maps for base colour, roughness, metalness, normal, emission and opacity. Textures: packed ones written beside the model under their own file name; a path looked for relative to the model, then at the absolute path the file also names (`RawTexture::elsewhere`), then by name beside the model. Users' properties become node attributes. Left out, and said in one warning: cameras, lights, NURBS surfaces, animation, skinning (skinned meshes come in at their bind pose), shape keys.
- Held by `crates/tequila-import/tests/more_formats.rs`: a PLY box in all three encodings lands 40 x 20 x 10 mm as 0.04 x 0.02 x 0.01 m with its colours shown; a 3MF print plate lands Y up in metres with each object named, its build placements, a two-colour object as two parts, the turned component, three placements of one object as one mesh, the texture written out and the support left out; Blender's export of a synthetic scene (`tests/data/make_fbx_fixtures.py`: a plinth, a tilted column, a ball on it and a half-size linked copy, three materials, a checker texture, a camera and a light) as binary FBX in millimetres Y up with the picture packed, and as ASCII FBX in centimetres Z up with the picture beside it, both land every part's centre within 2 µm of where Blender put it (the ball through two levels of parents), the plinth 80 x 12 x 60 mm, the camera and light left out and said, one mesh for the two balls, Blender's roughness 0.35 and metal recovered from the binary and the ASCII's Phong mapped by the formula (0.213); a texture path that does not resolve falls back to the absolute one, then to beside the model. Unit tests in each module cover the encodings, broken files (errors, never panics), the transform convention and the polygon division.

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

*As built, November 5* (`tequila-import/src/rhino/`): **Rhino `.3dm`**, Rhino 5 to 8, read by Tequila's own pure-Rust reader of the openNURBS archive, written from McNeel's open-source openNURBS (no openNURBS linked, no new dependency; `cadmpeg-codec-rhino` on crates.io was looked at and not taken, being a hundred thousand lines decoding into a CAD model of its own). Read: the start and version (4-byte chunk lengths before archive 50, 8-byte from it), the chunks, the settings' unit (every Rhino unit; those Tequila has no name for are scaled into metres) and absolute tolerance, the layer table (each layer's full `Parent::Child` name, colour, on/off inherited down the tree, render material), the material table (the basic `ON_Material` and Rhino 7's PBR user data, mapped to OpenPBR as `rhino/material.rs` documents, with colour, bump, transparency and PBR maps found by path, then beside the model), the block definitions, and the object table: meshes (plain and compressed arrays, normals, texture coordinates, vertex colours, double precision vertices), B-reps (the render mesh Rhino saved on each face, when it did; else each face on its NURBS, plane, revolved or sum surface, its loops of NURBS, line, polyline, arc and polycurve trims, divided by the IGES divider through a new `iges::divide_parametric`, in parallel, at the import presets with the model's absolute tolerance as the floor), extrusions (their saved mesh cache, or sides and caps divided the same way), SubD (`rhino/subd.rs`: the control cage, Catmull-Clark with crease rules and limit positions, two rounds at Draft, three at Medium, four at High and Ultra), lone surfaces, and blocks (each definition meshed once and placed by every reference, nested ones through both). Each object keeps its name, its Rhino ID as an attribute, its layer as its CAD layer, and wears its render material or else a plastic of its display colour, resolved by layer, object, material or parent as Rhino does (an untouched black Default layer means the default material); hidden objects and objects on layers that are off arrive hidden. The tree is the layer tree, objects under their layer. Rhino's Z up is turned to Y up on the model's node. B-rep edges come from their 3D curves, seams left out. A file with anything divided is kept with the project for Re-Tessellate. Not read, and named in the import's warnings: curves, points, annotation, hatches, lights, linked blocks, embedded bitmaps, mitered extrusion ends (drawn square), RDK content beyond its simulated material, per-face colours, texture mappings other than the mesh's own coordinates, offset surfaces and curve proxies in a B-rep. Tests: `tequila-import/tests/rhino.rs` (14, and `real_files` by hand on a folder named by `TEQUILA_3DM_SAMPLES`) against fixtures from a test writer, so they prove the reader against its reading of openNURBS, not against Rhino: all five archive versions, a box of planes divided to a volume of 1000.0000 of 1000, a NURBS face with a hole to an area of 87.470 of 87.434, a revolved sphere to 112.69 of 113.10, a capped extrusion to 62.70 of 62.83, blocks placed to the millimetre, a SubD rounded inside its cage; and the unit tests in `rhino/` (8). Read by hand: a 10 x 20 x 30 mm box written by McNeel's rhino3dm (archive 70), read as that box in millimetres; and a sphere mesh written in archives 50 to 80 by `cadmpeg-codec-rhino`'s independent writer, read whole (528 triangles, its box to the 0.1 mm). That crate's reader, run on the test writer's files, agrees on their meshes, layers, materials, blocks, SubD and extrusions. *Later on November 5*, on McNeel's own files - the openNURBS `example_files` for Rhino 4 to 8 and rhino3dm's test models, 72 files, 58 of them with geometry, many saved by Rhino itself - every file reads into a valid document, nothing the reader should read left out (2.59 million triangles in all); they found one bug, now fixed: Rhino saves its B-reps, NURBS and revolved and sum surfaces under classes of its own (`TL_Brep` and the rest), which now read as openNURBS's, as openNURBS's own table of them says. Two of rhino3dm's files, a mesh Rhino 8 saved and a SubD with creases, are fixtures in `tests/data/rhino3dm` (MIT, licence beside them), read by `mcneel_files_read`; openNURBS's example files carry no licence that allows passing them on, so none is in the repository.

*Measured, November 11: where a large STEP import's time goes.* A 236 MB file from Siemens NX (204 solids, 101,454 faces, 225,552 edges; 1.07 million triangles at Medium) takes about 92 s on an M3 Max, 73-80 s of it in the helper (`tequila-occt`) at a peak of 12-13 GB: reading the file 5 s, **turning its entities into shapes 55 s**, meshing 11 s (in parallel). Sampled, all of those 55 s are OpenCASCADE's shape healing (`XSAlgo_ShapeProcessor::ProcessShape`, `ShapeFix_Shape`), about 70% of it `ShapeFix_Edge::FixAddPCurve` projecting each edge's 3D curve onto its faces' B-spline surfaces (`ShapeAnalysis_Surface::ValueOfUV`, `Extrema_GenExtPS`): the file has no `PCURVE` or `SURFACE_CURVE` at all, as NX writes it, and the mesher needs a curve on the surface for every edge of every face. One part alone takes 38 s of it. It runs on one core, and is not divided by part here: the healing happens inside the reader's transfer, before colours, names and layers are attached to the shapes it leaves, and faces of one shell share the edges the curves are written to. Reading sub-shape names (`ReadSubshapeNames`) costs nothing measurable (54.8 s with, 54.7 s without). A faster import of such files would be healing deferred and run per solid across cores, with the document's sub-shape labels mapped through each fix - a change to the helper that needs the three platforms' builds to test - or OpenCASCADE made faster at it; neither is done.

### 6.3 USD — M8

Import and **export**. USD is the interchange format that matters going forward; supporting it well makes Tequila a good pipeline citizen. Map: `UsdGeomMesh`/`UsdGeomXform` → scene tree; `UsdShade` + **MaterialX/OpenPBR** → material graphs (near-lossless in both directions since Tequila's shading model *is* OpenPBR); `UsdLux` → lights; `UsdGeomCamera` → cameras. Support `.usdz` for delivery.

Instancing: honour `UsdGeomPointInstancer` and scene-graph instancing (`instanceable`) natively.

*As built, November 5* - **USDZ, for AR on Apple's devices** (`tequila-format/src/usdz.rs`, with `usdc.rs` and `bake.rs`): File > Export USDZ for AR, `tequila --export-usdz PATH`, scripting `export format=usdz`. What is shown, as the package AR Quick Look on an iPhone or iPad and Vision Pro open: a ZIP of stored members, each one's data on a 64-byte boundary (the local header padded with an extra field, as Pixar's writer pads it), the stage first and its PNGs after it.
- The stage is binary USD, crate version 0.8.0, written by Tequila's own crate writer (`usdc.rs`; no dependency): what Apple's own tools write and the form AR Quick Look was first made to read. A text stage would also open on a current device - Pixar dropped `usdchecker --arkit`'s root-layer rule in 2023 (PR 2863, "no longer valid for RealityKit") and three.js writes text - but binary has no exceptions and is a fraction of the size of a dense mesh as text. Structural sections are compressed as 0.4.0 and later require, the integers in USD's two-bit delta coding and the whole in LZ4 blocks of literals only (valid LZ4, which any decoder reads); values are stored plain.
- Real size and place: `metersPerUnit = 1`, `upAxis = "Y"`, a default prim named for the file, of kind `component`, translated so the product's box stands on the ground with its middle over the origin.
- Geometry: a `Mesh` per part shown, named as the part, with its normals, its UVs (or the bake's) and its extent, in `Xform`s for its groups; a mesh several shown parts share is written once under the class prim `/Prototypes`, and each part is an instanceable `Xform` that references it and binds its own material (a binding inside the prototype could not reach the materials). Lights, cameras, hidden parts and other model sets are left out.
- Materials are `UsdPreviewSurface` and nothing else - all AR Quick Look and RealityKit draw: a metal's colour from its measured reflectance (`Material::solid_color`; the `.usda` and glTF exports write the white base a gold has), glass as transparency, since a preview surface can cover what is behind it but not tint it (clear glass a faint dark film that carries its reflections - white, it went milky - and a tinted one covering more, in its colour, the deeper its tint), the IOR, coat, emission. A material whose graph varies is baked by the bake glTF export uses, moved to `bake.rs` and grown: colour and opacity, roughness and metalness (and baked occlusion) packed as glTF packs them, emission, coat weight and roughness, and a tangent-space normal map from the bump (height bumps measured as the renderer measures them, across half a texel; normal maps decoded and re-encoded in the bake's frame, green up). Each image is written only where it varies over the part - a uniform one becomes the plain value - and read through a `UsdUVTexture` (sRGB for colours, raw for data; the normal map scaled 2 and biased -1 as the checker asks) and a `UsdPrimvarReader_float2` of `st`. A mesh with no UVs of its own and over 40,000 triangles takes its colours at its vertices, through a `UsdPrimvarReader_float3` of `displayColor`. A material without images carries its whole Tequila definition, as in the `.usda`.
- Read back (`tequila-import/src/usd.rs`): `.usdc` and `.usdz` now import, through `usdc::read` (real LZ4, both compressed-array codings, crate 0.4.0 and later) into the same prims the text reader makes; a package's pictures come in from inside it, a preview surface's `UsdUVTexture` inputs (whole, or one channel, or the alpha) become image nodes, a `UsdTransform2d`'s scale and offset its mapping; references within the file are followed and `class` prims not drawn.
- Held by `tequila-format/tests/usdz.rs`: the package is stored, aligned and starts with the stage; the stage is metres, Y up, standing on the ground at the origin at the document's own size (the perfume bottle); plain materials' inputs; a procedural baked where it varies and nowhere else; a bump as a normal map; a shared mesh as one prototype and two instances; Tequila reading its own package back (geometry on the ground at its size, the baked wood as its picture, a plain material whole); the Headphones and palma P8 samples; and a layer Pixar's library wrote (`tests/data/make_pixar_written.py`) reading field for field. `usdc.rs`'s own tests round-trip a layer and refuse damage.
- Checked with Pixar's USD as Blender 5.2 bundles it: `UsdUtils.ComplianceChecker(arkit=True)` - `usdchecker --arkit` - on the exports of the perfume bottle (2.4 MB, 2.0 s from the command line), the headphones (26.6 MB with 32 baked images, 18.6 s) and the palma P8 (11.9 MB, 8 images, 20 instances of 10 prototypes, 5.8 s): no errors, no failed checks, no warnings. Pixar's bounding box of each is its real size standing at the origin (11.3, 19.7 and 14.1 cm tall, from y = 0), and each opens in Blender (Eevee) where the project's camera frames it as Tequila does.
- The `.usda` export, checked the same way, had two faults now mended: a node's CAD attributes were written as a dictionary-valued attribute, which USD does not have, so Pixar's reader - and Blender - refused every `.usda` of an imported model (they are the prim's `customData` now); and its camera had no clipping range, so USD's default of a metre to a million cut away a product nearer than a metre (`clippingRange` is a millimetre to 100 km now).
- Not done: Multi-Material variants as variant sets (the one shown is written); a product's animation; meshes over 40,000 triangles without UVs keep only vertex colours, and only their first material's; an instance's baked material is baked once, from the first instance's place (a pattern in world space shows the same on each); RealityKit's own materials (`RealityKit*` schemas) and physics.

### 6.3.1 Adobe Substance materials

*As built, November 5* (`tequila-import/src/substance/`, `docs/02-MATERIAL-SYSTEM.md` §5.6). An `.sbsar` (a 7-Zip archive around an XML description) or an `.sbs` (Designer's XML) is read for its graphs' settings, presets, outputs and pictures in pure Rust, without Adobe's software; it becomes one material per graph, wired from the outputs by the texture-pack code, whose maps Adobe's `sbsrender` renders when it is installed (`bridge.rs`). The tests read synthetic files written from the published format; no file made by Adobe's tools was available.

### 6.3.2 Blender files

*As built, November 12* (`tequila-import/src/blend/`). A `.blend` is read by the Blender installed on the computer, not by a reader of Tequila's own: the file is Blender's memory written out, in a layout that changes with every version, and most of what makes its geometry - modifiers, curves, text, geometry nodes, instances - only Blender can evaluate. Tequila runs it in the background (`--background --factory-startup --disable-autoexec`, so nothing in the file runs and the person's add-ons change nothing) with `blend/tequila_export.py`, which writes down what the saved scene holds: its objects with modifiers applied, under their collections and parents, hidden where Blender's render leaves them out; instances placed under what made them, sharing a mesh; every material's node tree node for node; its images, packed ones written out; its cameras, render size and view transform. Nothing is saved back. Tequila links nothing of Blender (GPL); the script is Tequila's own, Apache-2.0, and Blender is a program it runs, as it runs OpenCASCADE's reader. Blender is found where its installers put it (the newest first), on the `PATH`, or where `TEQUILA_BLENDER` says; without it the import says Blender 4.2 or later is needed and where to get it.

The node trees are translated, not baked (`blend/material.rs`): Tequila has Blender's textures, maths, colour and vector nodes as nodes of its own, evaluated with Cycles' algorithms (`tequila_materials::nodes`, now with `ShaderNode::from_blender`), so a powder coat made of a Voronoi and a noise is that Voronoi and that noise, at the same scale, still editable. Groups, reroutes and muted nodes are seen through first. The Principled BSDF maps onto OpenPBR parameter by parameter (Specular IOR Level times two is the specular weight; Blender's anisotropy rotation turns a whole turn where Tequila's turns half; transmission and subsurface take the base colour, as Blender's do); the other BSDFs become the OpenPBR material that renders as they do. Mix and Add Shader become Tequila's own, rearranged into the nesting Tequila mixes in; a Transparent BSDF mixed in becomes the other surface's opacity, an Emission added becomes its emission. Image textures read through the mesh's UVs where Blender does and otherwise at the coordinates the tree works out; Clip, Extend and Mirror are built from Blender's own maths nodes around the lookup, so both renderers do them, and a colour is premultiplied where Blender's is. A world-space position or direction is turned from Tequila's Y up into Blender's Z up where it is used as a number, and left where it is used as a normal. Bump, Displacement and Normal Map become the surface's Bump (bumps on bumps add their heights), Bevel its edge radius; Color Ramp and RGB and Float Curves become ramps. A colour attribute reaches a part only when its material reads it. What has no counterpart is named once in the warnings, and what it fed keeps the value typed into that socket.

Not read: lights (a model is lit by Tequila's studios), the world, the file's other scenes, volumes, OSL, and Blender's Light Path outputs beyond the five Tequila has. Tests: `tequila-import/tests/blend.rs` (11) on the answer Blender gave for `tests/data/make_blend_fixtures.py`'s scene - made of Blender's primitives, with a part of two materials, a child with a bevel modifier, hidden and excluded parts, a collection instanced twice, text, a curve, two cameras, and materials using a packed label clipped onto the top face, a node group, a muted node, a reroute, a ramp, curves, a bump, a normal map, mixed shaders, a cut-out and a glow - and, where Blender is installed, on that scene read by Blender itself, which must read the same; and a file Blender cannot open is refused, not read as Blender's start-up scene. Read by hand: a guitar pedal of nine parts and thirteen materials, 266,000 triangles, in 3.4 s, its procedural powder coat, its silkscreen projected from above and clipped to the top face, its spun knobs and their radial anisotropy, and its nine cameras, rendering as Blender renders it.

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
| USD / USDZ | Full scene interchange (preferred); USDZ for AR on Apple's devices (§6.3) |
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
