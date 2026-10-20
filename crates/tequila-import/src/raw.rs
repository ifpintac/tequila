//! What a format reader produces, before the post-process pipeline.
//!
//! Deliberately not [`SceneDocument`](tequila_scene::SceneDocument): readers
//! know nothing about document IDs, and should not, because a reader that
//! allocated document keys could not be run twice, run in parallel, or run
//! against a document it is not being imported into. The pipeline turns this
//! into a `Command::Import` at the end, once, and that command is the only
//! thing that touches the document.

use tequila_core::math::{Transform, Vec2, Vec3};
use tequila_scene::material::MaterialDefinition;
use std::collections::BTreeMap;

/// A mesh as the file describes it: triangles, and whatever channels the
/// format carried.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawMesh {
    pub positions: Vec<Vec3>,
    /// Empty means the format carried none, or carried unusable ones. §6.1
    /// point 3 turns on this distinction: present normals are never
    /// regenerated.
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    /// Per-vertex colours, linear RGBA. Empty when the file carried none.
    pub colors: Vec<tequila_core::math::Vec4>,
    pub indices: Vec<u32>,
    /// Index into [`RawScene::materials`].
    pub material: Option<usize>,
    /// Name from the file, for diagnostics and for naming split primitives.
    pub name: String,
    /// The model's own edges, segment end points in pairs: a CAD file's B-rep
    /// edges. Empty for a format that has none.
    pub edge_lines: Vec<Vec3>,
}

/// A node as the file describes it, referring to meshes and parents by
/// position in the [`RawScene`].
#[derive(Clone, Debug, PartialEq)]
pub struct RawNode {
    pub name: String,
    pub parent: Option<usize>,
    pub transform: Transform,
    /// Index into [`RawScene::meshes`]; `None` for a pure grouping node.
    pub mesh: Option<usize>,
    pub metadata: BTreeMap<String, String>,
    /// Switched off in the file: a CAD part on a layer that is turned off.
    pub hidden: bool,
}

impl RawNode {
    pub fn part(name: impl Into<String>, mesh: usize) -> RawNode {
        RawNode {
            name: name.into(),
            parent: None,
            transform: Transform::IDENTITY,
            mesh: Some(mesh),
            metadata: BTreeMap::new(),
            hidden: false,
        }
    }

    pub fn group(name: impl Into<String>) -> RawNode {
        RawNode {
            name: name.into(),
            parent: None,
            transform: Transform::IDENTITY,
            mesh: None,
            metadata: BTreeMap::new(),
            hidden: false,
        }
    }
}

/// An image a material asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct RawTexture {
    /// As written in the file, possibly percent-encoded and possibly relative.
    ///
    /// For an [embedded](RawTexture::embedded) image there is no such name in
    /// the file, so this is the bare filename to give it once it is written
    /// out — no directories, which the caller supplies.
    pub uri: String,
    /// What the material wants it for, which decides how it is decoded — a
    /// colour map is sRGB and a normal map is not.
    pub role: tequila_texture::texture::MapRole,
    /// The encoded image, for a model that carries its images inside itself.
    ///
    /// A `.glb` keeps them in its binary chunk and a `.gltf` can keep them in
    /// `data:` URIs, and between them that is most of what anyone downloads.
    /// Neither has a file on disk to point at, and the document stores paths
    /// rather than pixels by design (`docs/file-format.md` §3.1), so the
    /// bytes ride along here and `import` writes them out beside the model.
    ///
    /// Reading still touches no filesystem: this is bytes already in hand
    /// from the container, not a file that was opened.
    pub embedded: Option<std::sync::Arc<Vec<u8>>>,
}

impl RawTexture {
    /// Whether two references are for the same decoded image.
    ///
    /// Name and role, never the bytes. An ORM image arrives as two references
    /// to the same picture and must collapse to one entry, and comparing
    /// several megabytes to discover that would be a strange way to find out.
    pub fn same_image(&self, other: &RawTexture) -> bool {
        self.uri == other.uri && self.role == other.role
    }
}

/// Everything one file contained.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawScene {
    /// Images the materials refer to, in the order their graphs number them.
    ///
    /// A URI as the file wrote it, not a path: reading a model is defined to
    /// touch no filesystem, so resolving these against a directory is the
    /// caller's job and happens in `import`, which is the only part that knows
    /// where the file came from.
    pub textures: Vec<RawTexture>,
    /// Parent-before-child, so a single forward pass can build the tree.
    pub nodes: Vec<RawNode>,
    pub roots: Vec<usize>,
    pub meshes: Vec<RawMesh>,
    pub materials: Vec<MaterialDefinition>,
    /// What the format says its units are, when it says anything. glTF is
    /// metres by specification; OBJ and STL carry nothing, and §3.4 asks for a
    /// guess from the bounding box instead.
    pub source_unit: Option<tequila_core::units::LengthUnit>,
    /// How a CAD file's exact surfaces were divided, recorded on the model's
    /// top nodes so it can be divided again. `None` for a mesh format.
    pub tessellation: Option<String>,
    /// Recoverable problems, surfaced to the user rather than swallowed. A
    /// file that imports with a warning is still an import; §4.2 asks for
    /// "reject-with-explanation" only where the data cannot be trusted.
    pub warnings: Vec<String>,
}

impl RawScene {
    /// Check the reader's own output before the pipeline runs on it. Every
    /// failure here is a bug in a reader, not in the file, and finding it at
    /// the boundary beats finding it as a panic three steps later.
    pub fn validate(&self) -> Result<(), String> {
        for (i, node) in self.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                if parent >= self.nodes.len() {
                    return Err(format!(
                        "node {i} has parent {parent}, which does not exist"
                    ));
                }
                if parent >= i {
                    return Err(format!(
                        "node {i} refers to parent {parent}, which is not listed before it"
                    ));
                }
            }
            if let Some(mesh) = node.mesh {
                if mesh >= self.meshes.len() {
                    return Err(format!(
                        "node {i} refers to mesh {mesh}, which does not exist"
                    ));
                }
            }
        }
        for &root in &self.roots {
            if root >= self.nodes.len() {
                return Err(format!("root {root} does not exist"));
            }
            if self.nodes[root].parent.is_some() {
                return Err(format!("node {root} is listed as a root but has a parent"));
            }
        }
        for (i, mesh) in self.meshes.iter().enumerate() {
            if let Some(material) = mesh.material {
                if material >= self.materials.len() {
                    return Err(format!(
                        "mesh {i} refers to material {material}, which does not exist"
                    ));
                }
            }
            if !mesh.normals.is_empty() && mesh.normals.len() != mesh.positions.len() {
                return Err(format!("mesh {i} has a partial normal channel"));
            }
            if !mesh.uvs.is_empty() && mesh.uvs.len() != mesh.positions.len() {
                return Err(format!("mesh {i} has a partial UV channel"));
            }
        }
        Ok(())
    }
}
