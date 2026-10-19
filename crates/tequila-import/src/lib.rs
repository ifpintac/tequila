//! Geometry import.
//!
//! `docs/03-SCENE-AND-IO.md` §6.1 defines an eight-step post-process pipeline,
//! and [`process`] runs it in that order. The steps are not independent —
//! welding must precede normal generation so there is something to smooth
//! across, normals must precede tangents which are built against them, and
//! instancing must come last so it compares finished meshes — so the order is
//! part of the specification rather than an implementation detail.
//!
//! # Deviation from §6.1: no Assimp, and no FBX
//!
//! §6.1 names Assimp and lists eight formats. OBJ, STL and glTF/GLB are
//! implemented directly, for the same reason the HDR and EXR readers in
//! `tequila-texture` are written out: the library crates carry no
//! third-party Rust dependencies, and adding a C++ build dependency to reach
//! three formats that are each a few hundred lines is a poor trade.
//!
//! **FBX is out of scope.** It is Autodesk's proprietary format: undocumented,
//! with no published specification, and reachable only through their own SDK
//! or through a reverse-engineered reader. §0.2's ground rules already put
//! the equivalent CAD formats out of scope for exactly this reason, and FBX
//! belongs with them. Every tool that writes FBX also writes glTF, OBJ or USD,
//! so the cost to a user is one export menu rather than a lost model — and the
//! importer says so by name rather than failing as though the file were
//! corrupt.
//!
//! PLY, 3DS, Collada and 3MF are M8's business, alongside the CAD kernels.

pub mod gltf;
pub mod ifc;
pub mod iges;
pub mod keyshot;
pub mod obj;
mod occt;
mod part21;
mod planar;
pub mod raw;
pub mod step;
pub mod usd;
pub mod stl;
pub mod texture_set;

use tequila_core::math::{Mat4, Transform, Vec3};
use tequila_core::units::LengthUnit;
use tequila_scene::command::{reserve_keys, reserve_texture_ids, Command, ImportPayload};
use tequila_scene::geometry::Mesh;
use tequila_scene::tree::{MaterialKey, NodeId, NodeKind, SceneNode};
use tequila_scene::SceneDocument;
use raw::RawScene;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    /// The file could not be read from disk.
    Io(String),
    /// The extension is not one this crate reads.
    UnknownFormat(String),
    /// The format is recognized but this file uses a part of it that is not
    /// implemented. Distinct from `Malformed` because the file is fine and the
    /// reader is not, which is a different thing to tell the user.
    Unsupported(String),
    /// The file is recognized but its contents do not hold together. §4.2 asks
    /// for "reject-with-explanation" over silent misparsing, so these carry
    /// what was wrong and where.
    Malformed(String),
    /// A file the import depends on — an external buffer, say — is absent.
    Missing(String),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::Io(m) => write!(f, "{m}"),
            ImportError::UnknownFormat(m) => write!(f, "unsupported file type: {m}"),
            ImportError::Unsupported(m) => write!(f, "not implemented: {m}"),
            ImportError::Malformed(m) => write!(f, "the file is malformed: {m}"),
            ImportError::Missing(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Import settings, as the import dialog will present them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImportOptions {
    /// §6.1 point 3: default 30°, user-adjustable per import. Only ever
    /// applied to meshes that arrive *without* normals.
    pub smoothing_angle: f32,
    /// §6.1 point 2. Off only for diagnosing a suspect file.
    pub join_identical_vertices: bool,
    /// §6.1 point 4.
    pub generate_tangents: bool,
    /// §6.1 point 6 / §6.4.
    pub detect_instances: bool,
    /// What the file's units are. `None` asks the format, and falls back to
    /// the bounding-box guess of §3.4 when the format says nothing.
    pub source_unit: Option<LengthUnit>,
    /// Set the model on the floor: move it up or down so its lowest point is
    /// at height zero, where the ground is.
    ///
    /// On by default because most files are not authored standing on the
    /// floor — a glTF from a scanning app or a store is centred on its own
    /// origin — and a model half below the ground renders cut in half, with
    /// nothing in the Solid view to say why, since the Solid view does not
    /// draw the ground. Off for a file that has to land where it was
    /// modelled: one part of an assembly imported beside the rest.
    pub place_on_ground: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        ImportOptions {
            smoothing_angle: 30f32.to_radians(),
            join_identical_vertices: true,
            generate_tangents: true,
            detect_instances: true,
            source_unit: None,
            place_on_ground: true,
        }
    }
}

/// What an import did, for the import summary and for tests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportReport {
    pub nodes: usize,
    pub meshes_read: usize,
    /// After instancing. The difference from `meshes_read` is the win.
    pub meshes_kept: usize,
    pub instances_detected: usize,
    pub vertices_welded: usize,
    pub triangles_dropped: usize,
    pub meshes_given_normals: usize,
    pub meshes_given_tangents: usize,
    pub triangles: usize,
    /// Image maps carried in from the file.
    pub textures: usize,
    /// The unit the geometry was interpreted as.
    pub source_unit: LengthUnit,
    /// True when that unit came from a bounding-box guess rather than from the
    /// file, so the UI knows to ask (§3.4).
    pub unit_was_guessed: bool,
    pub warnings: Vec<String>,
    /// How far the model was moved up to stand on the ground, in its own
    /// units; negative for down. Zero when it already stood there or
    /// [`ImportOptions::place_on_ground`] was off.
    pub lifted: f32,
}

/// Read a file and build the `Command::Import` that adds it to `document`.
///
/// The document is **not** modified: the importer produces a command, and the
/// command bus applies it. That is what makes an import undoable, journalled
/// and scriptable without the importer knowing about any of those.
/// Where a model was read from, for the companion files that belong with it.
///
/// A bare directory would do for resolving a texture URI, which is all this
/// used to be. Extracting an embedded image needs the model's name as well, so
/// that two models sharing a folder do not write over each other's images.
#[derive(Clone, Copy, Debug)]
pub struct Origin<'a> {
    /// The folder the model itself is in.
    pub directory: &'a std::path::Path,
    /// Its filename without the extension.
    pub stem: &'a str,
}

impl Origin<'_> {
    /// The folder extracted images go into: `<model>.textures`, beside the
    /// model.
    ///
    /// Beside the model rather than in a hidden cache, because these are real
    /// files the saved project will point at. A cache that got cleared would
    /// take the textures out of a project that had been working, silently,
    /// which is the failure this codebase keeps finding and removing.
    pub fn extraction_directory(&self) -> std::path::PathBuf {
        self.directory.join(format!("{}.textures", self.stem))
    }
}

pub fn import(
    document: &SceneDocument,
    parent: Option<NodeId>,
    path: &Path,
    options: &ImportOptions,
) -> Result<(Command, ImportReport), ImportError> {
    build_read(document, parent, read_file(path)?, options)
}

/// A model file read and turned into a [`RawScene`], not yet built into a
/// document: the half of [`import`] that needs no document, and so the half
/// that can run on another thread. A large STEP file spends seconds here.
pub struct ReadModel {
    pub scene: RawScene,
    /// The file's name without its extension.
    pub name: String,
    /// Where the file is, which is what its textures are relative to.
    pub directory: Option<std::path::PathBuf>,
    /// The file itself, recorded on the model's top nodes so it can be
    /// reloaded when it changes (`tequila_scene::reimport`).
    pub path: Option<std::path::PathBuf>,
    /// A CAD file's own bytes and their id, to keep with the project
    /// (`tequila_scene::document::Sources`) and name on the model's top
    /// nodes.
    pub brep: Option<(String, std::sync::Arc<Vec<u8>>)>,
}

/// The largest CAD file kept with a project. A bigger one is still read;
/// re-dividing it then needs the file where it was.
pub const MOST_KEPT: usize = 64 * 1024 * 1024;

/// Read a STEP file's bytes at `quality`: what re-dividing a model from its
/// kept source does. `path` is where it came from, if known.
pub fn read_cad(bytes: &[u8], name: &str, path: Option<std::path::PathBuf>, quality: step::Tessellation) -> Result<ReadModel, ImportError> {
    let scene = if iges::looks_like(bytes) { iges::read_with(bytes, name, quality)? } else { step::read_with(bytes, name, quality)? };
    Ok(ReadModel { scene, name: name.to_string(), directory: path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())), path, brep: None })
}

/// Read a model file: the first half of [`import`].
pub fn read_file(path: &Path) -> Result<ReadModel, ImportError> {
    read_file_with(path, step::Tessellation::default())
}

/// [`read_file`], dividing a CAD file's surfaces at `quality`.
pub fn read_file_with(path: &Path, quality: step::Tessellation) -> Result<ReadModel, ImportError> {
    let bytes = std::fs::read(path)
        .map_err(|e| ImportError::Io(format!("{} could not be read: {e}", path.display())))?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Imported".to_string());

    // Sibling files — an OBJ's .mtl, a glTF's .bin — resolve relative to the
    // model. Restricted to the model's own directory: a file from a stranger
    // must not be able to name `../../.ssh/id_rsa` and have the importer read
    // it.
    let directory = path.parent().map(|p| p.to_path_buf());
    let sibling = move |relative: &str| -> Option<Vec<u8>> {
        let directory = directory.as_ref()?;
        let candidate = directory.join(relative);
        let canonical = candidate.canonicalize().ok()?;
        let root = directory.canonicalize().ok()?;
        if !canonical.starts_with(&root) {
            return None;
        }
        std::fs::read(canonical).ok()
    };

    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let cad = matches!(extension.as_str(), "step" | "stp" | "igs" | "iges");
    let scene = match extension.as_str() {
        "step" | "stp" => step::read_with(&bytes, &name, quality)?,
        "igs" | "iges" => iges::read_with(&bytes, &name, quality)?,
        _ => read_format(&bytes, &extension, &name, &sibling)?,
    };
    let file = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let brep = (cad && bytes.len() <= MOST_KEPT).then(|| (tequila_scene::document::Sources::id_of(&bytes), std::sync::Arc::new(bytes)));
    Ok(ReadModel { scene, name, directory: path.parent().map(|p| p.to_path_buf()), path: Some(file), brep })
}

/// Build a read model into an import command: the second half of
/// [`import`], against the document it will be applied to.
pub fn build_read(
    document: &SceneDocument,
    parent: Option<NodeId>,
    model: ReadModel,
    options: &ImportOptions,
) -> Result<(Command, ImportReport), ImportError> {
    model.prepare(options)?.finish(document, parent, options)
}

/// A model read and processed - welded, its normals made, its repeated
/// shapes found, brought into metres - and not yet given the document's
/// keys: all of an import that does not depend on the document, so it can be
/// done off the window's thread. On a five-million-triangle model that was
/// three and a half seconds of a frozen window. [`Prepared::finish`] makes
/// it the import command.
pub struct Prepared {
    scene: RawScene,
    processed: Processed,
    report: ImportReport,
    directory: Option<std::path::PathBuf>,
    name: String,
    /// The CAD file's own bytes and their id, as [`ReadModel::brep`].
    pub brep: Option<(String, std::sync::Arc<Vec<u8>>)>,
}

impl Prepared {
    /// The meshes the import will add, in the order its command lists them:
    /// for converting them for the renderer off the window's thread too.
    pub fn meshes(&self) -> &[Mesh] {
        &self.processed.0
    }

    /// The file's name without its extension.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The import command, against the document it will be applied to.
    pub fn finish(self, document: &SceneDocument, parent: Option<NodeId>, options: &ImportOptions) -> Result<(Command, ImportReport), ImportError> {
        let Prepared { scene, processed, report, directory, name, .. } = self;
        let origin = directory.as_deref().map(|directory| Origin { directory, stem: &name });
        finish_build(document, parent, scene, processed, report, options, origin)
    }
}

impl ReadModel {
    /// The first half of building an import, which needs no document: see
    /// [`Prepared`].
    pub fn prepare(self, options: &ImportOptions) -> Result<Prepared, ImportError> {
        let model = self;
        let mut scene = model.scene;
        if let Some(file) = &model.path {
            for &root in &scene.roots {
                if let Some(node) = scene.nodes.get_mut(root) {
                    node.metadata.insert(
                        tequila_scene::reimport::SOURCE_FILE.to_string(),
                        file.display().to_string(),
                    );
                }
            }
        }
        // How a CAD model was divided, and the source it was divided from, on
        // its top nodes: what dividing it again reads.
        let tessellation = scene.tessellation.clone();
        for &root in &scene.roots.clone() {
            if let Some(node) = scene.nodes.get_mut(root) {
                if let Some(quality) = &tessellation {
                    node.metadata.insert(step::TESSELLATION.to_string(), quality.clone());
                }
                if let Some((id, _)) = &model.brep {
                    node.metadata.insert(tequila_scene::document::BREP.to_string(), id.clone());
                }
            }
        }
        let (scene, processed, report) = prepare_scene(scene, options)?;
        Ok(Prepared { scene, processed, report, directory: model.directory, name: model.name, brep: model.brep })
    }
}

/// Write an image the model carried inside itself out beside the model.
///
/// `docs/file-format.md` §3.1: the document stores where a texture came from
/// and never its pixels. An embedded image has no "where" until one is made
/// for it, so this makes one. The alternative — teaching the document to hold
/// pixels — would put a scene's worth of decoded 4K images into the save file,
/// which §3.1 rules out in as many words.
///
/// The name is already reduced to safe characters by the reader, so it cannot
/// climb out of the folder; the same rule the `sibling` resolver applies to a
/// model naming its own companion files.
///
/// Returns the path to record whether or not the write succeeded: a path that
/// does not resolve renders as the fallback, which is the same as any other
/// missing map, and the warning says what happened.
fn extract(
    name: &str,
    bytes: &[u8],
    origin: Option<Origin<'_>>,
    warnings: &mut Vec<String>,
) -> std::path::PathBuf {
    let Some(origin) = origin else {
        warnings.push(format!(
            "{name} is stored inside the model and there is nowhere to put it, \
             because the model was read from memory rather than from a file; \
             that map will not appear"
        ));
        return std::path::PathBuf::from(name);
    };

    // Beside the model first. A model opened from somewhere read-only — a
    // mounted image, a network share — still deserves its textures, so the
    // temporary directory is the fallback rather than the failure.
    let beside = origin.extraction_directory();
    let mut directory = beside.clone();
    if std::fs::create_dir_all(&directory).is_err() {
        directory = std::env::temp_dir().join(format!("tequila-{}.textures", origin.stem));
        if std::fs::create_dir_all(&directory).is_err() {
            warnings.push(format!(
                "{name} is stored inside the model and could not be written \
                 out, to {} or to a temporary folder; that map will not appear",
                beside.display()
            ));
            return beside.join(name);
        }
        warnings.push(format!(
            "{} is not writable, so the model's own images were put in {} \
             instead; moving the project will leave them behind",
            beside.display(),
            directory.display()
        ));
    }

    let path = directory.join(name);
    if let Err(e) = std::fs::write(&path, bytes) {
        warnings.push(format!(
            "{name} is stored inside the model and could not be written to {}: \
             {e}; that map will not appear",
            path.display()
        ));
    }
    path
}

/// Read bytes whose format is named by `extension`, without touching the
/// filesystem. `sibling` resolves companion files.
pub fn read_format(
    bytes: &[u8],
    extension: &str,
    name: &str,
    sibling: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<RawScene, ImportError> {
    match extension {
        "obj" => obj::read(bytes, name, sibling),
        "stl" => stl::read(bytes, name),
        "gltf" | "glb" => gltf::read(bytes, name, sibling),
        "usda" | "usd" => usd::read(bytes, name),
        "step" | "stp" => step::read(bytes, name),
        "igs" | "iges" => iges::read(bytes, name),
        "ifc" => ifc::read(bytes, name),
        // Named explicitly so the message can say what to do. Falling
        // through to `UnknownFormat` would suggest a typo rather than a
        // deliberate decision.
        "fbx" => Err(ImportError::Unsupported(
            "FBX is Autodesk's proprietary format and Tequila does not read it. \
             Export the same model as glTF, OBJ or USD instead — every tool that \
             writes FBX writes at least one of those."
                .into(),
        )),
        other => Err(ImportError::UnknownFormat(format!(".{other}"))),
    }
}

/// Run the post-process pipeline and turn the result into a command.
pub fn build(
    document: &SceneDocument,
    parent: Option<NodeId>,
    scene: RawScene,
    options: &ImportOptions,
    // Where the file came from, which is what a texture URI is relative to.
    // `None` when the caller has only bytes, in which case a texture is found
    // only if its URI is already absolute.
    origin: Option<Origin<'_>>,
) -> Result<(Command, ImportReport), ImportError> {
    let (scene, processed, report) = prepare_scene(scene, options)?;
    finish_build(document, parent, scene, processed, report, options, origin)
}

/// The half of [`build`] that needs no document.
fn prepare_scene(mut scene: RawScene, options: &ImportOptions) -> Result<(RawScene, Processed, ImportReport), ImportError> {
    scene.validate().map_err(ImportError::Malformed)?;
    show_vertex_colours(&mut scene);
    let mut report = ImportReport {
        meshes_read: scene.meshes.len(),
        warnings: std::mem::take(&mut scene.warnings),
        ..Default::default()
    };

    // §3.4: a unit the file states, or the user chose, is honoured by
    // converting the geometry into metres, which is what a document stores
    // whatever unit it shows. Presets' depths and radii, the camera's focal
    // length and the Material tab's millimetres all assume it, so a part a
    // CAD package exported in millimetres, kept as its numbers, rendered a
    // thousand times its size: milk as solid as plastic, and a bump of half
    // a millimetre invisible. A guessed unit is not applied here; the
    // application puts the guess to the user.
    if let Some(unit) = options.source_unit.or(scene.source_unit) {
        if unit != LengthUnit::Unspecified {
            rescale(&mut scene, unit.to_metres());
        }
    }
    let processed = process(&scene, options, &mut report)?;
    resolve_units(&scene, &processed.0, options, &mut report);
    Ok((scene, processed, report))
}

/// Colours painted on the vertices, shown: the base colour of every material
/// a coloured mesh wears is multiplied by them, as glTF's COLOR_0 asks. A
/// mesh without colours reads white there, so a material it shares is
/// unchanged on it. A coloured mesh with no material is given one.
///
/// Without this the colours were imported and nothing showed them: a ZBrush
/// polypaint, a scan's colours and a glTF's painted vertices all rendered as
/// a plain finish, and only a Vertex Color node added by hand brought them
/// back.
fn show_vertex_colours(scene: &mut RawScene) {
    use tequila_materials::graph::{Node, NodeKind, Socket, Value};
    use tequila_materials::material_type::{MaterialType, Param};

    let mut own: Option<usize> = None;
    let mut shown = std::collections::BTreeSet::new();
    for i in 0..scene.meshes.len() {
        let mesh = &scene.meshes[i];
        if mesh.colors.is_empty() || mesh.colors.len() != mesh.positions.len() {
            continue;
        }
        let material = match mesh.material.filter(|&m| m < scene.materials.len()) {
            Some(material) => material,
            None => *own.get_or_insert_with(|| {
                scene.materials.push(tequila_scene::material::MaterialDefinition::new(
                    "Vertex colours",
                    MaterialType::Plastic,
                    tequila_materials::openpbr::Material::plastic(tequila_core::spectrum::Spectrum::ONE, 0.45),
                ));
                scene.materials.len() - 1
            }),
        };
        scene.meshes[i].material = Some(material);
        if !shown.insert(material) {
            continue;
        }
        let graph = &mut scene.materials[material].graph;
        let output = graph.output();
        let colours = graph.add_node(Node::new(NodeKind::VertexColor));
        // Whatever gave the base colour before - a picture, or the colour
        // itself - times the vertices'.
        let driver = graph
            .links()
            .iter()
            .find(|l| l.to == output && l.to_socket == Socket::Param(Param::BaseColor))
            .map(|l| (l.from, l.from_socket));
        let base = graph.material().base_color;
        let by = match driver {
            Some(driver) => Some(driver),
            None if (base.max_component() - base.min_component()).abs() < 1e-4 && base.max_component() > 0.999 => None,
            None => Some((graph.add_node(Node::new(NodeKind::Constant(Value::Color(base)))), 0)),
        };
        match by {
            None => graph.connect(colours, output, Socket::Param(Param::BaseColor)),
            Some((from, socket)) => {
                let product = graph.add_node(Node::new(NodeKind::Multiply));
                graph.connect(colours, product, Socket::Input(0));
                graph.connect_output(from, socket, product, Socket::Input(1));
                graph.connect(product, output, Socket::Param(Param::BaseColor));
            }
        }
    }
}

/// The half of [`build`] that gives the import the document's keys.
fn finish_build(
    document: &SceneDocument,
    parent: Option<NodeId>,
    scene: RawScene,
    (meshes, remap, offsets): Processed,
    mut report: ImportReport,
    options: &ImportOptions,
    origin: Option<Origin<'_>>,
) -> Result<(Command, ImportReport), ImportError> {

    // Reserve the keys the command will use. Everything below refers to nodes
    // by their eventual `NodeId`, which is why this happens before the nodes
    // are built rather than after.
    let extra_nodes = 0;
    let (mesh_keys, material_keys, node_ids) = reserve_keys(
        document,
        meshes.len(),
        scene.materials.len(),
        scene.nodes.len() + extra_nodes,
    );

    // Each node's place in the file, which is what an update of the model
    // matches it by (`docs/03` §2.3).
    let path_keys = tequila_scene::reimport::keys_of(
        &scene.nodes.iter().map(|n| (n.name.clone(), n.parent)).collect::<Vec<_>>(),
    );

    let mut nodes = Vec::with_capacity(scene.nodes.len());
    for (i, source) in scene.nodes.iter().enumerate() {
        let id = node_ids[i];
        let mut node = SceneNode::new(id, source.name.clone(), NodeKind::Group);
        node.path_key = Some(path_keys[i].clone());
        node.parent = source.parent.map(|p| node_ids[p]).or(parent);
        node.transform = source.transform;
        node.metadata = source.metadata.clone();

        if let Some(raw_mesh) = source.mesh {
            let kept = remap[raw_mesh];
            node.kind = NodeKind::Part {
                mesh: mesh_keys[kept],
            };
            // An instance of an already-imported shape sits at a different
            // place in its own local space, and the difference moves into the
            // node's transform (§6.4).
            let offset = offsets[raw_mesh];
            if offset != Vec3::ZERO {
                let shifted = source.transform.matrix * Mat4::translation(offset);
                node.transform = Transform::from_matrix(shifted).unwrap_or(source.transform);
            }
            node.material = scene.meshes[raw_mesh]
                .material
                .map(|m| material_keys[m])
                .or(node.material);
        }
        nodes.push(node);
    }

    // Children must be listed on their parents, since `reinsert` restores a
    // subtree by its stored links.
    let mut children: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for (i, source) in scene.nodes.iter().enumerate() {
        if let Some(p) = source.parent {
            children.entry(node_ids[p]).or_default().push(node_ids[i]);
        }
    }
    for node in &mut nodes {
        if let Some(list) = children.remove(&node.id) {
            node.children = list;
        }
    }

    // On the floor. Only for a model imported at the top of the tree: under
    // a parent, it is being placed relative to something else on purpose.
    if options.place_on_ground && parent.is_none() {
        let index_of: BTreeMap<NodeId, usize> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let world = |mut i: usize| {
            let mut transform = nodes[i].transform;
            while let Some(p) = nodes[i].parent.and_then(|p| index_of.get(&p).copied()) {
                transform = nodes[p].transform * transform;
                i = p;
            }
            transform
        };
        let kept_at: std::collections::HashMap<_, usize> = mesh_keys.iter().enumerate().map(|(i, &k)| (k, i)).collect();
        let mut lowest = f32::INFINITY;
        for (i, node) in nodes.iter().enumerate() {
            let NodeKind::Part { mesh } = node.kind else { continue };
            let Some(&kept) = kept_at.get(&mesh) else { continue };
            let world = world(i);
            // The box's lowest corner is the part's lowest point only when
            // the part is square to the floor. Turned any other way the box
            // of the box sits lower than anything in it - a ball under a 45°
            // turn by a seventh of its size - and the model floated above the
            // floor, its shadow apart from it. The lowest vertex, then.
            let row = world.matrix.m[1];
            let square = [row[0], row[1], row[2]].iter().filter(|v| v.abs() > 1e-6).count() <= 1;
            let low = if square {
                let bounds = meshes[kept].bounds().transformed(&world);
                if bounds.is_empty() { f32::INFINITY } else { bounds.min.y }
            } else {
                meshes[kept].positions.iter().map(|p| row[0] * p.x + row[1] * p.y + row[2] * p.z + row[3]).fold(f32::INFINITY, f32::min)
            };
            lowest = lowest.min(low);
        }
        if lowest.is_finite() && lowest != 0.0 {
            let lift = Transform::translation(Vec3 {
                x: 0.0,
                y: -lowest,
                z: 0.0,
            });
            for &root in &scene.roots {
                nodes[root].transform = lift * nodes[root].transform;
            }
            report.lifted = -lowest;
        }
    }

    report.nodes = nodes.len();
    report.meshes_kept = meshes.len();
    report.triangles = meshes.iter().map(|m| m.triangle_count()).sum();

    // Texture ids, and the materials renumbered onto them.
    //
    // The reader numbered its images from zero because it is not allowed to
    // look at the document; the document's own ids are reserved here and the
    // graphs rewritten to match.
    let texture_ids = reserve_texture_ids(document, scene.textures.len());
    let mut materials = scene.materials.clone();
    if !texture_ids.is_empty() {
        for material in &mut materials {
            let ids = &texture_ids;
            material.graph.remap_textures(&|local| {
                ids.get(local.0 as usize).copied().unwrap_or(local)
            });
        }
    }

    let textures: Vec<_> = texture_ids
        .iter()
        .copied()
        .zip(scene.textures.iter())
        .map(|(id, raw)| {
            let relative = crate::gltf::percent_decode(&raw.uri);
            if let Some(bytes) = &raw.embedded {
                let path = extract(&relative, bytes, origin, &mut report.warnings);
                return (
                    id,
                    tequila_scene::textures::TextureSource::new(path, raw.role),
                );
            }
            let mut path = match origin {
                Some(origin) => origin.directory.join(&relative),
                None => std::path::PathBuf::from(&relative),
            };
            // A map named by where it was on the author's machine
            // (`C:\Users\artist\tex\wood.jpg`), or in a folder that did not
            // come with the model, is usually sent beside the model.
            if !path.exists() {
                let file = relative.rsplit(['/', '\\']).next().unwrap_or(&relative);
                if let Some(beside) = origin.map(|origin| origin.directory.join(file)).filter(|p| p.exists()) {
                    path = beside;
                }
            }
            if !path.exists() {
                report.warnings.push(format!(
                    "{relative} is referenced by the model and not beside it; \
                     that map will not appear"
                ));
            } else if !tequila_texture::image::Image::is_decodable(&path) {
                // Existing but undecodable was the quieter of the two
                // failures, and the worse one. The map is listed in the
                // panel, the graph refers to it, the file is right there
                // — and the render comes out flat, because the decode
                // fails later and the node falls back to the parameter's
                // own colour. Nothing along that chain has anywhere to
                // put a complaint, so the complaint belongs here.
                let formats = tequila_texture::image::Image::DECODABLE.join(", ");
                report.warnings.push(format!(
                    "{relative} is in a format this renderer cannot read \
                     ({formats} only); that map will not appear"
                ));
            }
            (id, tequila_scene::textures::TextureSource::new(path, raw.role))
        })
        .collect();
    report.textures = textures.len();

    let payload = ImportPayload {
        meshes: mesh_keys.iter().copied().zip(meshes).collect(),
        textures,
        materials: material_keys
            .iter()
            .copied()
            .zip(materials)
            .collect::<Vec<(MaterialKey, _)>>(),
        roots: scene.roots.iter().map(|&r| node_ids[r]).collect(),
        nodes,
    };
    Ok((
        Command::Import {
            payload: Box::new(payload),
        },
        report,
    ))
}

/// The meshes the import keeps, a map from each raw mesh to the kept mesh that
/// stands in for it, and the translation each raw mesh's node must absorb to
/// use that stand-in (§6.4).
type Processed = (Vec<Mesh>, Vec<usize>, Vec<Vec3>);

/// The §6.1 pipeline, steps 1-6.
///
/// Step 1, triangulation, happens in the readers: OBJ fans its polygons and
/// the other two formats are triangles already, so nothing arrives here
/// untriangulated. Step 7, unit conversion, is [`resolve_units`]; step 8,
/// material mapping, belongs to each reader because it is format-specific.
fn process(
    scene: &RawScene,
    options: &ImportOptions,
    report: &mut ImportReport,
) -> Result<Processed, ImportError> {
    let mut processed = Vec::with_capacity(scene.meshes.len());

    for (i, raw) in scene.meshes.iter().enumerate() {
        let mut mesh = Mesh::new(raw.positions.clone(), raw.indices.clone());
        mesh.normals = raw.normals.clone();
        if !raw.uvs.is_empty() {
            mesh.uv_sets = vec![raw.uvs.clone()];
        }
        if raw.colors.len() == raw.positions.len() {
            mesh.colors = raw.colors.clone();
        }
        mesh.edge_lines = raw.edge_lines.clone();

        // Step 5 runs early as well as late: welding and normal generation
        // both index into the buffers, and a bad index there is a panic rather
        // than an error.
        let dropped = mesh
            .validate_and_clean()
            .map_err(|e| ImportError::Malformed(format!("mesh {i} (`{}`): {e}", raw.name)))?;
        report.triangles_dropped += dropped;

        // Step 2.
        if options.join_identical_vertices {
            report.vertices_welded += mesh.join_identical_vertices();
        }

        // Step 3 — only when absent, which §6.1 is emphatic about.
        if mesh.normals.is_empty() {
            mesh.generate_normals(options.smoothing_angle);
            report.meshes_given_normals += 1;
        }

        // Step 4.
        if options.generate_tangents && mesh.generate_tangents() {
            report.meshes_given_tangents += 1;
        }

        mesh.recompute_bounds();
        processed.push(mesh);
    }

    // Step 6: instancing.
    let mut kept: Vec<Mesh> = Vec::new();
    let mut remap = vec![0usize; processed.len()];
    let mut offsets = vec![Vec3::ZERO; processed.len()];
    let mut by_hash: BTreeMap<u64, Vec<usize>> = BTreeMap::new();

    for (i, mesh) in processed.into_iter().enumerate() {
        if !options.detect_instances {
            remap[i] = kept.len();
            kept.push(mesh);
            continue;
        }
        let hash = mesh.content_hash();
        let candidates = by_hash.entry(hash).or_default();
        // §6.4: "Guard against false positives by comparing full buffers on
        // hash match." The hash only groups; this decides.
        match candidates
            .iter()
            .find(|&&candidate| kept[candidate].is_same_shape_as(&mesh))
        {
            Some(&candidate) => {
                remap[i] = candidate;
                offsets[i] = centroid(&mesh) - centroid(&kept[candidate]);
                report.instances_detected += 1;
            }
            None => {
                let index = kept.len();
                candidates.push(index);
                remap[i] = index;
                kept.push(mesh);
            }
        }
    }

    Ok((kept, remap, offsets))
}

/// Centroid in `f64`, matching what `is_same_shape_as` compares. Summing
/// thousands of large coordinates in `f32` loses the low bits that distinguish
/// one placement from another.
fn centroid(mesh: &Mesh) -> Vec3 {
    if mesh.positions.is_empty() {
        return Vec3::ZERO;
    }
    let (mut x, mut y, mut z) = (0.0f64, 0.0f64, 0.0f64);
    for p in &mesh.positions {
        x += p.x as f64;
        y += p.y as f64;
        z += p.z as f64;
    }
    let n = mesh.positions.len() as f64;
    tequila_core::math::vec3((x / n) as f32, (y / n) as f32, (z / n) as f32)
}

/// Scale everything the file placed by `ratio`: the meshes' vertices, and
/// each node's translation. Conjugating every local transform by the same
/// uniform scale scales the world transforms the same way and leaves their
/// rotations alone.
fn rescale(scene: &mut RawScene, ratio: f64) {
    if !ratio.is_finite() || ratio <= 0.0 || (ratio - 1.0).abs() < 1e-9 {
        return;
    }
    let k = ratio as f32;
    for mesh in &mut scene.meshes {
        for p in &mut mesh.positions {
            *p = *p * k;
        }
        for p in &mut mesh.edge_lines {
            *p = *p * k;
        }
    }
    let Some(scale) = Transform::from_matrix(Mat4::scale(Vec3::splat(k))) else {
        return;
    };
    for node in &mut scene.nodes {
        node.transform = scale * node.transform * scale.inverted();
    }
}

/// Step 7. Decides *what unit the file is in*. The unit the document shows
/// lengths in is `Command::SetUnits`, which is the user's decision, not the
/// importer's.
fn resolve_units(
    scene: &RawScene,
    meshes: &[Mesh],
    options: &ImportOptions,
    report: &mut ImportReport,
) {
    if let Some(unit) = options.source_unit {
        report.source_unit = unit;
        return;
    }
    if let Some(unit) = scene.source_unit {
        report.source_unit = unit;
        return;
    }
    report.source_unit = guess_unit(meshes);
    report.unit_was_guessed = true;
}

/// §3.4's bounding-box heuristic, for formats that carry no units: "a
/// 0.05-unit object is probably metres; a 50,000-unit object is probably
/// millimetres".
///
/// A guess, offered to the user rather than applied silently — `unit_was_guessed`
/// is what tells the import dialog to ask.
pub fn guess_unit(meshes: &[Mesh]) -> LengthUnit {
    let extent = meshes
        .iter()
        .map(|m| m.bounds().diagonal().length())
        .fold(0.0f32, f32::max);
    if extent <= 0.0 || !extent.is_finite() {
        return LengthUnit::Unspecified;
    }
    // Bands chosen so a product-sized object lands on the unit a designer of
    // that product would have modelled it in.
    match extent {
        e if e < 0.5 => LengthUnit::Metre,
        e if e < 50.0 => LengthUnit::Centimetre,
        _ => LengthUnit::Millimetre,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;
    use tequila_scene::CommandBus;

    fn cube_obj(offset: [f32; 3]) -> String {
        let [dx, dy, dz] = offset;
        let mut text = String::new();
        for (x, y, z) in [
            (0.0, 0.0, 0.0),
            (1.0, 0.0, 0.0),
            (1.0, 1.0, 0.0),
            (0.0, 1.0, 0.0),
        ] {
            text.push_str(&format!("v {} {} {}\n", x + dx, y + dy, z + dz));
        }
        text.push_str("f 1 2 3 4\n");
        text
    }

    fn import_text(
        text: &str,
        extension: &str,
        options: &ImportOptions,
    ) -> (SceneDocument, ImportReport) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let scene = read_format(text.as_bytes(), extension, "test", &|_| None).unwrap();
        let (command, report) = build(&document, None, scene, options, None).unwrap();
        bus.apply(&mut document, command).unwrap();
        document.validate().unwrap();
        (document, report)
    }

    /// A model centred on its origin is set on the floor, and one floating
    /// above it is brought down; switching the option off leaves both where
    /// the file put them.
    #[test]
    fn a_model_is_set_on_the_ground() {
        let lowest = |document: &SceneDocument| document.bounds().min.y;
        let centred = cube_obj([0.0, -0.5, 0.0]);
        let (document, report) = import_text(&centred, "obj", &Default::default());
        assert!(lowest(&document).abs() < 1e-6, "the model's lowest point is at {}", lowest(&document));
        assert!((report.lifted - 0.5).abs() < 1e-6);

        let floating = cube_obj([0.0, 3.0, 0.0]);
        let (document, report) = import_text(&floating, "obj", &Default::default());
        assert!(lowest(&document).abs() < 1e-6, "a floating model stayed up at {}", lowest(&document));
        assert!((report.lifted + 3.0).abs() < 1e-6);

        let kept = ImportOptions {
            place_on_ground: false,
            ..Default::default()
        };
        let (document, report) = import_text(&centred, "obj", &kept);
        assert!((lowest(&document) + 0.5).abs() < 1e-6);
        assert_eq!(report.lifted, 0.0);
    }

    /// A part turned at an angle lands with its lowest point on the floor,
    /// not lifted by the corner of a box around its turned box.
    #[test]
    fn a_turned_model_is_set_on_the_ground_by_its_lowest_point() {
        // A diamond in the YZ plane, its points on the axes, turned 45°
        // about X: the lowest point is then at -0.71, where the box of its
        // turned box says -1.41.
        let mut mesh = raw::RawMesh::default();
        mesh.positions = vec![vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, -1.0, 0.0), vec3(0.0, 0.0, -1.0)];
        mesh.indices = vec![0, 1, 2, 0, 2, 3];
        let mut part = raw::RawNode::part("Diamond", 0);
        part.transform = Transform::rotation(Vec3::X, std::f32::consts::FRAC_PI_4);
        let scene = raw::RawScene { meshes: vec![mesh], nodes: vec![part], roots: vec![0], ..Default::default() };
        let document = SceneDocument::new();
        let (command, report) = build(&document, None, scene, &ImportOptions::default(), None).unwrap();
        let tequila_scene::Command::Import { payload } = command else { panic!("an import") };
        let node = &payload.nodes[0];
        let (_, mesh) = &payload.meshes[0];
        let lowest = mesh.positions.iter().map(|&p| node.transform.point(p).y).fold(f32::INFINITY, f32::min);
        assert!(lowest.abs() < 1e-4, "the lowest point is at {lowest}, lifted {}", report.lifted);
    }

    /// An OBJ whose vertices carry colours brings them in, decoded from the
    /// display values the extension writes; two parts of the same shape
    /// painted differently stay two meshes.
    #[test]
    fn obj_vertex_colours_come_in_and_keep_parts_apart() {
        let text = "o red\nv 0 0 0 1 0 0\nv 1 0 0 1 0 0\nv 1 1 0 1 0 0\nv 0 1 0 1 0 0\nf 1 2 3 4\n\
                    o grey\nv 5 0 0 0.5 0.5 0.5\nv 6 0 0 0.5 0.5 0.5\nv 6 1 0 0.5 0.5 0.5\nv 5 1 0 0.5 0.5 0.5\nf 5 6 7 8\n";
        let (document, report) = import_text(text, "obj", &Default::default());
        assert_eq!(report.instances_detected, 0, "differently coloured parts were taken for one");
        let meshes: Vec<&Mesh> = document.geometry.iter().map(|(_, m)| m).collect();
        assert_eq!(meshes.len(), 2);
        let red = meshes.iter().find(|m| m.colors.first().is_some_and(|c| c.x > 0.9)).expect("the red part");
        assert_eq!(red.colors.len(), red.positions.len());
        let grey = meshes.iter().find(|m| m.colors.first().is_some_and(|c| c.x < 0.9)).expect("the grey part");
        // 0.5 on screen is about 0.214 linear.
        assert!((grey.colors[0].x - 0.214).abs() < 0.01, "not decoded: {:?}", grey.colors[0]);
    }

    /// Painted vertices are shown: the parts that carry colours wear a
    /// material whose base colour is the vertices' - a material of their own
    /// when they had none, the one they had multiplied when they did - and a
    /// part without colours keeps its material as it was.
    #[test]
    fn vertex_colours_are_shown_on_the_parts_that_carry_them() {
        use tequila_materials::graph::{NodeKind, Socket};
        use tequila_materials::material_type::Param;
        // Whether a part's material reads the vertices' colours into its base
        // colour, through whatever lies between.
        let reads_colours = |document: &SceneDocument, coloured: bool| -> Vec<bool> {
            document
                .tree
                .iter()
                .filter_map(|n| match n.kind {
                    tequila_scene::tree::NodeKind::Part { mesh } => {
                        let has = document.geometry.get(mesh).is_some_and(|g| !g.colors.is_empty());
                        (has == coloured).then_some(n.material)
                    }
                    _ => None,
                })
                .map(|material| {
                    let Some(graph) = material.and_then(|m| document.materials.get(m)).map(|d| &d.graph) else {
                        return false;
                    };
                    let output = graph.output();
                    let mut stack: Vec<_> = graph
                        .links()
                        .iter()
                        .filter(|l| l.to == output && l.to_socket == Socket::Param(Param::BaseColor))
                        .map(|l| l.from)
                        .collect();
                    while let Some(id) = stack.pop() {
                        if matches!(graph.node(id).map(|n| &n.kind), Some(NodeKind::VertexColor)) {
                            return true;
                        }
                        stack.extend(graph.links().iter().filter(|l| l.to == id).map(|l| l.from));
                    }
                    false
                })
                .collect()
        };

        // No material at all: one is made, for the painted part only.
        let text = "o painted
v 0 0 0 1 0 0
v 1 0 0 0 1 0
v 1 1 0 0 0 1
f 1 2 3
o plain
v 5 0 0
v 6 0 0
v 6 1 0
f 4 5 6
";
        let (document, _) = import_text(text, "obj", &Default::default());
        assert_eq!(reads_colours(&document, true), vec![true], "the painted part's material does not read its colours");
        assert_eq!(reads_colours(&document, false), vec![false], "the plain part was given the painted part's material");

        // A coloured glTF-style mesh with its own tinted material: multiplied.
        let mut scene = RawScene::default();
        scene.materials.push(tequila_scene::material::MaterialDefinition::new(
            "Tinted",
            tequila_materials::material_type::MaterialType::Plastic,
            tequila_materials::openpbr::Material::plastic(tequila_core::spectrum::spectrum(0.8, 0.2, 0.2), 0.4),
        ));
        scene.meshes.push(crate::raw::RawMesh {
            positions: vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            colors: vec![tequila_core::math::vec4(0.5, 0.5, 0.5, 1.0); 3],
            indices: vec![0, 1, 2],
            material: Some(0),
            name: "tinted".into(),
            ..Default::default()
        });
        show_vertex_colours(&mut scene);
        let graph = &scene.materials[0].graph;
        let output = graph.output();
        let driver = graph.links().iter().find(|l| l.to == output && l.to_socket == Socket::Param(Param::BaseColor)).expect("driven");
        assert!(matches!(graph.node(driver.from).map(|n| &n.kind), Some(NodeKind::Multiply)), "the tint was replaced rather than multiplied");
        assert_eq!(scene.materials.len(), 1, "a material was made for a mesh that had one");
    }

    #[test]
    fn an_obj_becomes_a_valid_document() {
        let (document, report) = import_text(&cube_obj([0.0; 3]), "obj", &Default::default());

        assert_eq!(report.nodes, 1);
        assert_eq!(report.triangles, 2);
        assert_eq!(document.tree.len(), 1);
        assert_eq!(document.geometry.len(), 1);
    }

    /// The point of §6.4. Two identical parts at different places must share
    /// one mesh, with the difference moved into the node transforms.
    #[test]
    fn identical_parts_at_different_places_become_instances() {
        let mut text = String::new();
        text.push_str("g a\n");
        text.push_str(&cube_obj([0.0, 0.0, 0.0]));
        text.push_str("g b\n");
        // Faces below use absolute indices, so the second part's vertices are
        // 5..8. Written out rather than reusing `cube_obj`.
        for (x, y, z) in [
            (100.0, 0.0, 0.0),
            (101.0, 0.0, 0.0),
            (101.0, 1.0, 0.0),
            (100.0, 1.0, 0.0),
        ] {
            text.push_str(&format!("v {x} {y} {z}\n"));
        }
        text.push_str("f 5 6 7 8\n");

        let (document, report) = import_text(&text, "obj", &Default::default());

        assert_eq!(report.meshes_read, 2);
        assert_eq!(report.meshes_kept, 1, "the two parts are the same shape");
        assert_eq!(report.instances_detected, 1);
        assert_eq!(document.geometry.len(), 1);

        // And they must still be 100 units apart in the world.
        let parts: Vec<_> = document
            .tree
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Part { .. }))
            .collect();
        assert_eq!(parts.len(), 2);
        let world = |n: &SceneNode| document.tree.world_transform(n.id).point(Vec3::ZERO);
        let separation = (world(parts[1]) - world(parts[0])).length();
        assert!(
            (separation - 100.0).abs() < 1e-3,
            "the instance moved: separation {separation}"
        );
    }

    #[test]
    fn instancing_can_be_turned_off() {
        let mut text = cube_obj([0.0; 3]);
        text.push_str("g b\n");
        for (x, y, z) in [
            (0.0, 0.0, 0.0),
            (1.0, 0.0, 0.0),
            (1.0, 1.0, 0.0),
            (0.0, 1.0, 0.0),
        ] {
            text.push_str(&format!("v {x} {y} {z}\n"));
        }
        text.push_str("f 5 6 7 8\n");

        let options = ImportOptions {
            detect_instances: false,
            ..Default::default()
        };
        let (document, report) = import_text(&text, "obj", &options);
        assert_eq!(report.meshes_kept, 2);
        assert_eq!(document.geometry.len(), 2);
    }

    /// §6.1 point 3, the rule that keeps cylinders round.
    #[test]
    fn authored_normals_are_never_regenerated() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nvn 0 1 0\nvn 0 1 0\nvn 0 1 0\nf 1//1 2//2 3//3\n";
        let (document, report) = import_text(obj, "obj", &Default::default());

        assert_eq!(report.meshes_given_normals, 0);
        let mesh = document.geometry.iter().next().unwrap().1;
        for n in &mesh.normals {
            // The authored normal is +Y, deliberately at odds with the +Z the
            // geometry would give.
            assert!((n.y - 1.0).abs() < 1e-5, "normal {n:?} was regenerated");
        }
    }

    #[test]
    fn a_mesh_without_normals_gets_them() {
        let (document, report) = import_text(&cube_obj([0.0; 3]), "obj", &Default::default());
        assert_eq!(report.meshes_given_normals, 1);
        let mesh = document.geometry.iter().next().unwrap().1;
        assert_eq!(mesh.normals.len(), mesh.vertex_count());
    }

    #[test]
    fn the_unit_guess_follows_the_bounding_box() {
        let small = Mesh::new(vec![Vec3::ZERO, vec3(0.05, 0.0, 0.0)], vec![]);
        let large = Mesh::new(vec![Vec3::ZERO, vec3(50_000.0, 0.0, 0.0)], vec![]);
        assert_eq!(guess_unit(&[small]), LengthUnit::Metre);
        assert_eq!(guess_unit(&[large]), LengthUnit::Millimetre);
        assert_eq!(guess_unit(&[]), LengthUnit::Unspecified);
    }

    /// glTF states its unit, so nothing is guessed; OBJ does not, so the guess
    /// is flagged for the dialog to ask about.
    #[test]
    fn a_stated_unit_is_honoured_and_a_guessed_one_is_flagged() {
        let (_, report) = import_text(&cube_obj([0.0; 3]), "obj", &Default::default());
        assert!(report.unit_was_guessed);

        let options = ImportOptions {
            source_unit: Some(LengthUnit::Inch),
            ..Default::default()
        };
        let (_, report) = import_text(&cube_obj([0.0; 3]), "obj", &options);
        assert_eq!(report.source_unit, LengthUnit::Inch);
        assert!(!report.unit_was_guessed);
    }

    #[test]
    fn an_unknown_extension_is_refused_by_name() {
        let error = read_format(b"", "sldprt", "x", &|_| None).unwrap_err();
        assert!(matches!(error, ImportError::UnknownFormat(_)));
        assert!(error.to_string().contains("sldprt"));

        // FBX is recognized but not implemented, which is a different message.
        // FBX is recognized and deliberately refused, which is a different
        // thing to tell the user than "unknown file type" — and the message
        // has to say what to export instead, or the refusal is just a wall.
        let error = read_format(b"", "fbx", "x", &|_| None).unwrap_err();
        assert!(matches!(error, ImportError::Unsupported(_)));
        let message = error.to_string();
        assert!(message.contains("proprietary"), "{message}");
        for alternative in ["glTF", "OBJ", "USD"] {
            assert!(
                message.contains(alternative),
                "the refusal does not mention {alternative}: {message}"
            );
        }
    }

    /// An import is one command, so one undo removes all of it.
    #[test]
    fn an_import_undoes_in_a_single_step() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let scene = read_format(cube_obj([0.0; 3]).as_bytes(), "obj", "test", &|_| None).unwrap();
        let (command, _) = build(&document, None, scene, &Default::default(), None).unwrap();

        bus.apply(&mut document, command).unwrap();
        assert_eq!(document.tree.len(), 1);

        bus.undo(&mut document).unwrap();
        assert_eq!(document.tree.len(), 0);
        assert_eq!(document.geometry.len(), 0, "the mesh must go with the node");
        document.validate().unwrap();
    }
}
