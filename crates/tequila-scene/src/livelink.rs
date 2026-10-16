//! LiveLinking (`docs/03` §7): a modelling application pushing its scene
//! into a running Tequila as it changes, with no export and import.
//!
//! What comes in is a *sync*: objects created or changed since the last one
//! (`upserts`), each under the source application's stable key for it, and
//! the keys of objects deleted. It becomes one [`Command::Reimport`] -
//! exactly what updating a model from its file is - so a sync is one undo
//! step and one render restart, and an object Tequila has seen before
//! keeps everything done to it here: its material, labels, visibility,
//! name and place in the model sets. Its geometry, placement and attributes
//! come from the source, which is the truth about those.
//!
//! An object the source sends for the first time arrives with a material
//! made from the source's own (Blender's Principled BSDF, mapped onto
//! OpenPBR by [`material`]); after that the material is Tequila's to
//! change. A deleted object is hidden rather than removed, as re-import
//! does, so an object renamed in the source does not take its finish with
//! it.
//!
//! Everything a source sends goes under one group of its own, made on the
//! first sync and found again by its source name (the `LiveLink` field).

use crate::command::{reserve_keys, Command, ImportPayload};
use crate::geometry::Mesh;
use crate::material::MaterialDefinition;
use crate::reimport::{NodeUpdate, ReimportPayload};
use crate::tree::{MaterialKey, MeshKey, NodeId, NodeKind, SceneNode};
use crate::SceneDocument;
use tequila_core::math::{vec3, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use std::collections::{BTreeMap, BTreeSet};

/// The metadata field naming the source a LiveLink group belongs to.
pub const SOURCE: &str = "LiveLink";

/// A source application's material, in the terms Blender's Principled BSDF
/// uses. Colours are linear sRGB.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveMaterial {
    pub name: String,
    pub base_color: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub ior: f32,
    pub transmission: f32,
    pub alpha: f32,
    pub emission: [f32; 3],
    /// Blender's emission strength: 1 is about a display's white.
    pub emission_strength: f32,
    pub coat: f32,
    pub coat_roughness: f32,
    pub sheen: f32,
    pub sheen_tint: [f32; 3],
}

impl Default for LiveMaterial {
    fn default() -> Self {
        LiveMaterial {
            name: "Material".into(),
            base_color: [0.8, 0.8, 0.8],
            metallic: 0.0,
            roughness: 0.5,
            ior: 1.5,
            transmission: 0.0,
            alpha: 1.0,
            emission: [0.0; 3],
            emission_strength: 0.0,
            coat: 0.0,
            coat_roughness: 0.03,
            sheen: 0.0,
            sheen_tint: [1.0; 3],
        }
    }
}

/// One object of a sync.
#[derive(Clone, Debug)]
pub struct LiveObject {
    /// The source's stable name for it.
    pub key: String,
    pub name: String,
    /// The key of the object it hangs under, if the source sent that too;
    /// otherwise it hangs under the source's group.
    pub parent: Option<String>,
    /// Its placement relative to that parent.
    pub transform: Transform,
    /// Its geometry, when that changed (or it is new); `None` keeps what
    /// Tequila has, or makes a group of a new object.
    pub mesh: Option<Mesh>,
    pub material: Option<LiveMaterial>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct LiveSync {
    /// Which source this is, e.g. `blender:C:/models/kettle.blend`.
    pub source: String,
    /// What its group is called when it is first made.
    pub label: String,
    pub upserts: Vec<LiveObject>,
    pub deletes: Vec<String>,
    /// The upserts are everything the source has: what it sent before and
    /// not now is gone. A client that has lost track of what it sent - a
    /// restarted Blender - says so with a full send.
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveReport {
    pub matched: usize,
    pub added: usize,
    pub removed: usize,
}

/// The group a source's objects live under, if it has synced before.
pub fn root_of(document: &SceneDocument, source: &str) -> Option<NodeId> {
    document
        .tree
        .iter()
        .find(|n| n.kind == NodeKind::Group && n.metadata.get(SOURCE).is_some_and(|s| s == source))
        .map(|n| n.id)
}

/// A Principled BSDF as an OpenPBR material: the near-direct mapping
/// `docs/livelink-protocol.md` documents. Glass when it mostly transmits,
/// metal when it is mostly metallic, plastic otherwise; then the shared
/// parameters on top.
pub fn material(live: &LiveMaterial) -> MaterialDefinition {
    let kind = if live.transmission > 0.5 {
        MaterialType::Glass
    } else if live.metallic > 0.5 {
        MaterialType::Metal
    } else {
        MaterialType::Plastic
    };
    let mut definition = MaterialDefinition::preset(live.name.clone(), kind);
    let colour = |c: [f32; 3]| Spectrum::from_linear_srgb(vec3(c[0].max(0.0), c[1].max(0.0), c[2].max(0.0)));
    let m = definition.material_mut();
    m.base_weight = 1.0;
    m.base_color = colour(live.base_color);
    m.base_metalness = live.metallic.clamp(0.0, 1.0);
    m.specular_roughness = live.roughness.clamp(0.0, 1.0);
    m.specular_ior = live.ior.clamp(1.0, 3.0);
    m.transmission_weight = live.transmission.clamp(0.0, 1.0);
    if live.transmission > 0.0 {
        // Blender tints glass by its base colour.
        m.transmission_color = colour(live.base_color);
    }
    m.geometry_opacity = live.alpha.clamp(0.0, 1.0);
    m.coat_weight = live.coat.clamp(0.0, 1.0);
    m.coat_roughness = live.coat_roughness.clamp(0.0, 1.0);
    m.fuzz_weight = live.sheen.clamp(0.0, 1.0);
    m.fuzz_color = colour(live.sheen_tint);
    if live.emission_strength > 0.0 && live.emission.iter().any(|c| *c > 0.0) {
        m.emission_color = colour(live.emission);
        m.emission_luminance = live.emission_strength * 100.0;
    } else {
        m.emission_luminance = 0.0;
    }
    definition
}

/// Plan `sync` against `document`: the command that applies it, and what
/// it matched, added and hid.
pub fn plan(document: &SceneDocument, sync: &LiveSync) -> (Command, LiveReport) {
    let tree = &document.tree;
    let root = root_of(document, &sync.source);

    // What the source has sent before, by key.
    let mut existing: BTreeMap<String, NodeId> = BTreeMap::new();
    if let Some(root) = root {
        for id in tree.descendants_and_self(root) {
            if id == root {
                continue;
            }
            if let Some(key) = tree.get(id).and_then(|n| n.path_key.clone()) {
                existing.entry(key).or_insert(id);
            }
        }
    }
    // The last of each key wins, as the source sent it.
    let mut upserts: BTreeMap<&str, &LiveObject> = BTreeMap::new();
    let mut order: Vec<&str> = Vec::new();
    for object in &sync.upserts {
        if upserts.insert(object.key.as_str(), object).is_none() {
            order.push(object.key.as_str());
        }
    }

    // A mesh whose geometry is what Tequila already has is not a new mesh:
    // every buffer the same, not just the counts a content hash buckets by.
    let unchanged = |object: &LiveObject| -> bool {
        let (Some(mesh), Some(&id)) = (&object.mesh, existing.get(&object.key)) else { return false };
        match tree.get(id).map(|n| &n.kind) {
            Some(NodeKind::Part { mesh: key }) => document.geometry.get(*key).is_some_and(|old| {
                old.positions == mesh.positions && old.indices == mesh.indices && old.normals == mesh.normals && old.uv_sets == mesh.uv_sets
            }),
            _ => false,
        }
    };
    let new_meshes = order.iter().filter(|k| upserts[*k].mesh.is_some() && !unchanged(upserts[*k])).count();
    let new_nodes: Vec<&str> = order.iter().copied().filter(|k| !existing.contains_key(*k)).collect();
    let new_materials = new_nodes.iter().filter(|k| upserts[*k].mesh.is_some()).count();
    let (mesh_keys, material_keys, node_ids) = reserve_keys(document, new_meshes, new_materials, new_nodes.len() + usize::from(root.is_none()));
    let mut mesh_keys = mesh_keys.into_iter();
    let mut material_keys = material_keys.into_iter();
    let mut node_ids = node_ids.into_iter();

    let mut meshes: Vec<(MeshKey, Mesh)> = Vec::new();
    let mut materials: Vec<(MaterialKey, MaterialDefinition)> = Vec::new();
    let mut added: Vec<SceneNode> = Vec::new();
    let mut updates = Vec::new();

    let root_id = match root {
        Some(root) => root,
        None => {
            let id = node_ids.next().expect("reserved");
            let mut group = SceneNode::new(id, sync.label.clone(), NodeKind::Group);
            group.metadata.insert(SOURCE.into(), sync.source.clone());
            added.push(group);
            id
        }
    };
    // Where each key is now, the new ones included, for parents.
    let mut placed: BTreeMap<String, NodeId> = existing.clone();
    let mut ids_of_new: BTreeMap<&str, NodeId> = BTreeMap::new();
    for key in &new_nodes {
        let id = node_ids.next().expect("reserved");
        ids_of_new.insert(key, id);
        placed.insert(key.to_string(), id);
    }

    let mut report = LiveReport::default();
    for key in &order {
        let object = upserts[key];
        let mesh_key = match &object.mesh {
            Some(mesh) if !unchanged(object) => {
                let k = mesh_keys.next().expect("reserved");
                let mut mesh = mesh.clone();
                mesh.recompute_bounds();
                meshes.push((k, mesh));
                Some(k)
            }
            _ => None,
        };
        match existing.get(*key) {
            Some(&id) => {
                let Some(before) = tree.get(id) else { continue };
                let kind = match (mesh_key, &before.kind) {
                    (Some(mesh), _) => NodeKind::Part { mesh },
                    (None, kind) => kind.clone(),
                };
                let mut metadata = before.metadata.clone();
                metadata.extend(object.metadata.clone());
                updates.push(NodeUpdate {
                    node: id,
                    kind,
                    transform: Some(object.transform),
                    // Tequila's, once it has one: that is the point.
                    material: before.material,
                    metadata,
                    path_key: Some(object.key.clone()),
                });
                report.matched += 1;
            }
            None => {
                let id = ids_of_new[key];
                let kind = match mesh_key {
                    Some(mesh) => NodeKind::Part { mesh },
                    None => NodeKind::Group,
                };
                let mut node = SceneNode::new(id, object.name.clone(), kind);
                node.parent = Some(object.parent.as_ref().and_then(|p| placed.get(p).copied()).filter(|&p| p != id).unwrap_or(root_id));
                node.transform = object.transform;
                node.metadata = object.metadata.clone();
                node.path_key = Some(object.key.clone());
                if mesh_key.is_some() {
                    let k = material_keys.next().expect("reserved");
                    materials.push((k, material(&object.material.clone().unwrap_or_default())));
                    node.material = Some(k);
                }
                added.push(node);
                report.added += 1;
            }
        }
    }

    // Children lists for what is added, and the added nodes whose parent
    // already exists, which the command hangs where they belong.
    let added_ids: BTreeSet<NodeId> = added.iter().map(|n| n.id).collect();
    let parents: Vec<(NodeId, Option<NodeId>)> = added.iter().map(|n| (n.id, n.parent)).collect();
    for node in &mut added {
        node.children = parents.iter().filter(|(_, p)| *p == Some(node.id)).map(|(c, _)| *c).collect();
    }
    let roots: Vec<NodeId> = added.iter().filter(|n| !n.parent.is_some_and(|p| added_ids.contains(&p))).map(|n| n.id).collect();
    // Parents before children, as an import inserts them: a source may
    // send a child before the object it hangs under.
    let mut ordered: Vec<SceneNode> = Vec::with_capacity(added.len());
    let mut placed_ids: BTreeSet<NodeId> = BTreeSet::new();
    while !added.is_empty() {
        let before = added.len();
        let mut rest = Vec::with_capacity(added.len());
        for node in added {
            if node.parent.is_some_and(|p| added_ids.contains(&p) && !placed_ids.contains(&p)) {
                rest.push(node);
            } else {
                placed_ids.insert(node.id);
                ordered.push(node);
            }
        }
        added = rest;
        if added.len() == before {
            // A cycle the source made: hang what is left under the group.
            for mut node in added.drain(..) {
                node.parent = Some(root_id);
                ordered.push(node);
            }
        }
    }
    let added = ordered;

    // Deleted in the source: hidden here.
    let mut hidden = Vec::new();
    let mut gone: BTreeSet<&str> = sync.deletes.iter().map(String::as_str).collect();
    if sync.complete {
        gone.extend(existing.keys().map(String::as_str));
    }
    for key in gone {
        if upserts.contains_key(key) {
            continue;
        }
        if let Some(&id) = existing.get(key) {
            if tree.get(id).is_some_and(|n| n.visible) {
                hidden.push(id);
                report.removed += 1;
            }
        }
    }

    let payload = ReimportPayload {
        added: ImportPayload { meshes, textures: Vec::new(), materials, nodes: added, roots },
        updates,
        hidden,
    };
    (Command::Reimport { payload: Box::new(payload) }, report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandBus;
    use tequila_core::math::Vec3;

    fn cube(size: f32) -> Mesh {
        let s = size;
        let positions = vec![vec3(0.0, 0.0, 0.0), vec3(s, 0.0, 0.0), vec3(0.0, s, 0.0), vec3(0.0, 0.0, s)];
        Mesh::new(positions, vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3])
    }

    fn object(key: &str, size: Option<f32>, at: f32) -> LiveObject {
        LiveObject {
            key: key.into(),
            name: key.into(),
            parent: None,
            transform: Transform::translation(vec3(at, 0.0, 0.0)),
            mesh: size.map(cube),
            material: Some(LiveMaterial { name: format!("{key} paint"), base_color: [0.8, 0.1, 0.1], ..Default::default() }),
            metadata: BTreeMap::new(),
        }
    }

    fn sync(upserts: Vec<LiveObject>, deletes: &[&str]) -> LiveSync {
        LiveSync { source: "blender:kettle.blend".into(), label: "Blender".into(), upserts, deletes: deletes.iter().map(|s| s.to_string()).collect(), complete: false }
    }

    fn apply(document: &mut SceneDocument, bus: &mut CommandBus, s: &LiveSync) -> LiveReport {
        let (command, report) = plan(document, s);
        bus.apply(document, command).expect("the sync applies");
        report
    }

    fn by_key(document: &SceneDocument, key: &str) -> NodeId {
        document.tree.iter().find(|n| n.path_key.as_deref() == Some(key)).map(|n| n.id).expect("the object is in the scene")
    }

    /// A first sync makes the source's group, its parts and their
    /// materials, from the source's own.
    #[test]
    fn a_first_sync_brings_the_objects_in() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let report = apply(&mut document, &mut bus, &sync(vec![object("Body", Some(1.0), 0.0), object("Lid", Some(0.5), 2.0)], &[]));
        assert_eq!(report, LiveReport { matched: 0, added: 2, removed: 0 });
        let root = root_of(&document, "blender:kettle.blend").expect("a group for the source");
        let body = by_key(&document, "Body");
        assert_eq!(document.tree.get(body).unwrap().parent, Some(root));
        let key = document.tree.get(body).unwrap().material.expect("a material");
        let colour = document.materials.get(key).unwrap().material().base_color.to_linear_srgb();
        assert!((colour.x - 0.8).abs() < 1e-3 && (colour.y - 0.1).abs() < 1e-3);
        document.validate().unwrap();
    }

    /// The point of it: a part synced again keeps the material it was given
    /// here, takes the source's new geometry and placement, and the sync is
    /// one undo step.
    #[test]
    fn a_synced_part_keeps_its_tequila_material() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        apply(&mut document, &mut bus, &sync(vec![object("Body", Some(1.0), 0.0)], &[]));
        let body = by_key(&document, "Body");
        let chrome = document.materials.insert(MaterialDefinition::preset("Chrome", MaterialType::Metal));
        bus.apply(&mut document, Command::AssignMaterial { nodes: vec![body], material: chrome }).unwrap();

        let report = apply(&mut document, &mut bus, &sync(vec![object("Body", Some(2.0), 5.0)], &[]));
        assert_eq!(report, LiveReport { matched: 1, added: 0, removed: 0 });
        let node = document.tree.get(body).expect("the same node");
        assert_eq!(node.material, Some(chrome), "the sync reassigned the material");
        let NodeKind::Part { mesh } = node.kind else { panic!("a part") };
        assert!((document.geometry.get(mesh).unwrap().bounds().max.x - 2.0).abs() < 1e-5, "the new geometry");
        assert!((document.tree.world_transform(body).point(Vec3::ZERO).x - 5.0).abs() < 1e-5, "the new placement");

        bus.undo(&mut document).unwrap();
        let NodeKind::Part { mesh } = document.tree.get(body).unwrap().kind else { panic!() };
        assert!((document.geometry.get(mesh).unwrap().bounds().max.x - 1.0).abs() < 1e-5, "one undo puts the old geometry back");
    }

    /// Geometry the source sends again unchanged is not a new mesh; a
    /// deleted object is hidden, keeping its material; a child hangs under
    /// its parent.
    #[test]
    fn unchanged_geometry_deletes_and_parents() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let mut handle = object("Handle", Some(0.2), 0.0);
        handle.parent = Some("Body".into());
        apply(&mut document, &mut bus, &sync(vec![handle, object("Body", Some(1.0), 0.0)], &[]));
        let body = by_key(&document, "Body");
        assert_eq!(document.tree.get(by_key(&document, "Handle")).unwrap().parent, Some(body), "the handle hangs under the body");
        let meshes = document.geometry.len();

        apply(&mut document, &mut bus, &sync(vec![object("Body", Some(1.0), 3.0)], &[]));
        assert_eq!(document.geometry.len(), meshes, "unchanged geometry made a new mesh");

        let report = apply(&mut document, &mut bus, &sync(Vec::new(), &["Handle"]));
        assert_eq!(report.removed, 1);
        let handle = document.tree.get(by_key(&document, "Handle")).unwrap();
        assert!(!handle.visible && handle.material.is_some(), "deleted means hidden, finish kept");
        document.validate().unwrap();
    }

    /// A complete send hides what the source sent before and no longer
    /// has, as a delete would.
    #[test]
    fn a_complete_send_hides_what_is_gone() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        apply(&mut document, &mut bus, &sync(vec![object("Body", Some(1.0), 0.0), object("Lid", Some(0.5), 2.0)], &[]));
        let mut full = sync(vec![object("Body", Some(1.0), 0.0)], &[]);
        full.complete = true;
        let report = apply(&mut document, &mut bus, &full);
        assert_eq!(report, LiveReport { matched: 1, added: 0, removed: 1 });
        assert!(!document.tree.get(by_key(&document, "Lid")).unwrap().visible);
        assert!(document.tree.get(by_key(&document, "Body")).unwrap().visible);
    }

    #[test]
    fn principled_maps_onto_openpbr() {
        let glass = material(&LiveMaterial { transmission: 1.0, ior: 1.45, roughness: 0.0, base_color: [0.9, 1.0, 0.9], ..Default::default() });
        assert_eq!(glass.material_type, MaterialType::Glass);
        assert!((glass.material().specular_ior - 1.45).abs() < 1e-6 && glass.material().transmission_weight == 1.0);
        let metal = material(&LiveMaterial { metallic: 1.0, roughness: 0.25, ..Default::default() });
        assert_eq!(metal.material_type, MaterialType::Metal);
        assert_eq!(metal.material().base_metalness, 1.0);
        let lamp = material(&LiveMaterial { emission: [1.0, 0.9, 0.8], emission_strength: 5.0, ..Default::default() });
        assert!((lamp.material().emission_luminance - 500.0).abs() < 1e-3);
    }
}
