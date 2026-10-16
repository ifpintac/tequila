//! Flattening a [`SceneDocument`] into what the renderer traces.
//!
//! `TEQUILA_SPEC.md` §1.1: the viewport **is** the renderer. There is no
//! preview representation that can drift from the output, so there is exactly
//! one path from the document to an image, and this is its first step.
//!
//! The document is a hierarchy the user edits; a
//! [`SceneSnapshot`](tequila_render_core::scene::SceneSnapshot) is a flat
//! list of instances the BVH can build over. Flattening happens here rather
//! than in the renderer because it needs to know about groups, inherited
//! visibility, and material assignment — none of which the renderer should
//! have an opinion about.
//!
//! # Instancing survives the flattening
//!
//! §2.2's rule — "4,000 identical M3 screws must load as one mesh and 4,000
//! transforms" — would be undone by a flattener that copied a mesh per node.
//! Meshes are uploaded once each and referenced by index, so a document with
//! one mesh and 4,000 parts produces one mesh and 4,000 instances.

use crate::document::SceneDocument;
use crate::tree::{MaterialKey, MeshKey, NodeId, NodeKind};
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_render_core::environment::Environment;
use tequila_render_core::light::LightSet;
use tequila_render_core::material::Material;
use tequila_render_core::scene::{Instance, Mesh as RenderMesh, SceneSnapshot, ShadowCatcher};
use tequila_materials::graph::{CompiledGraph, TextureId};
use tequila_render_core::shade::TextureTable;
use tequila_texture::texture::Texture;
use std::collections::BTreeMap;
use std::sync::Arc;

/// The correspondence between a snapshot and the document it came from.
///
/// Picking in the viewport gives an [`InstanceId`]; the Scene tab needs the
/// [`NodeId`]. Without this the two panels cannot stay synchronized, which
/// §04 §4.1 requires without exception.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapshotMapping {
    nodes: Vec<NodeId>,
    instances: BTreeMap<NodeId, InstanceId>,
    /// The node each light came from, indexed by `LightId`. Lights are not
    /// instances — they have no geometry to pick — so they need their own
    /// correspondence for a panel to say which row is which light.
    lights: Vec<NodeId>,
}

impl SnapshotMapping {
    /// The node an instance came from.
    pub fn node_of(&self, instance: InstanceId) -> Option<NodeId> {
        self.nodes.get(instance.0 as usize).copied()
    }

    /// The instance a node produced, if it produced one — a hidden part or a
    /// group has none.
    pub fn instance_of(&self, node: NodeId) -> Option<InstanceId> {
        self.instances.get(&node).copied()
    }

    /// The node a light came from.
    pub fn light_node(&self, light: tequila_core::ids::LightId) -> Option<NodeId> {
        self.lights.get(light.0 as usize).copied()
    }

    pub fn light_count(&self) -> usize {
        self.lights.len()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Converted meshes, kept across snapshots so that a scene which only moved
/// hands the renderer the *same* mesh allocations it had before.
///
/// That identity is what lets the CPU backend update instance transforms in
/// place instead of rebuilding every per-mesh BVH: it compares `Arc` pointers
/// to decide whether a mesh is unchanged. Without a cache every snapshot
/// allocates fresh meshes, every comparison fails, and a gizmo drag rebuilds
/// the whole acceleration structure once a frame — the slow path, reached
/// silently, with nothing failing.
///
/// The cache is the caller's, and the caller must clear it whenever the
/// geometry itself could have changed. `Command::scene_change` already says
/// when that is.
#[derive(Debug, Default)]
pub struct MeshCache {
    converted: BTreeMap<MeshKey, Arc<RenderMesh>>,
    /// Compiled graphs, with the content hash they were compiled from. A
    /// snapshot is rebuilt on every edit, and recompiling every material each
    /// time would put a compile on the interactive path for materials nobody
    /// touched.
    programs: BTreeMap<MaterialKey, (u64, Arc<CompiledGraph>)>,
    /// Decoded textures, so a file used by twelve materials is read once.
    textures: BTreeMap<TextureId, Option<Arc<Texture>>>,
    /// Volume Maps' density grids, read once each, failures remembered as
    /// textures' are.
    volumes: BTreeMap<TextureId, Option<Arc<tequila_texture::volume::VolumeGrid>>>,
    /// Photometric profiles, by path.
    ///
    /// A failure is remembered as a failure, for the same reason a texture's
    /// is: a missing `.ies` file would otherwise be opened once a frame for as
    /// long as the light exists.
    photometry: BTreeMap<String, Option<Arc<tequila_render_core::ies::IesProfile>>>,
    /// The visible stand-in for each area light, and the corners it was built
    /// from. Cached like every other mesh, so a scene whose lights only moved
    /// hands the backend the same allocations and keeps the in-place path.
    proxies: BTreeMap<NodeId, ([f32; 9], Arc<RenderMesh>)>,
    /// Displaced meshes, by the mesh, the height field, the detail and the
    /// part's scale they were built for. Only what the latest snapshot used
    /// is kept, so a displacement being tuned does not leave every earlier
    /// version of a million-triangle mesh behind it.
    displaced: BTreeMap<(MeshKey, u64), Arc<RenderMesh>>,
    /// Meshes with their occlusion measured, by the mesh they came from and
    /// the radius in its own units. The source is held with the result, so
    /// its address — the key — cannot be reused by another mesh while the
    /// entry lives.
    occluded: BTreeMap<(usize, u32), (Arc<RenderMesh>, Arc<RenderMesh>)>,
    /// Where each part's bubbles went, by the part's mesh, placement and
    /// request, holding the mesh as `occluded` does.
    bubbles: BTreeMap<u64, (Arc<RenderMesh>, Vec<(tequila_core::math::Vec3, f32)>)>,
    /// The one inside-out sphere every bubble is an instance of. Kept, so
    /// the renderer sees the same mesh from one build to the next and a
    /// move stays a transform update.
    bubble_mesh: Option<Arc<RenderMesh>>,
    /// Each part's fibres, by its mesh, scale and Fuzz request, holding the
    /// mesh as `occluded` does: fur is expensive to grow, so a part that
    /// only moved keeps its own.
    fuzz: BTreeMap<u64, (Arc<RenderMesh>, Option<Arc<RenderMesh>>)>,
    /// The ground quad and the extent it was built for.
    ///
    /// Cached for the same reason every other mesh is: the backend decides
    /// whether it can update transforms in place by comparing `Arc` pointers,
    /// and a ground plane reallocated on every snapshot would fail that
    /// comparison and rebuild every BVH in the scene once a frame — the slow
    /// path, reached silently, with nothing failing.
    ground: Option<(u32, Arc<RenderMesh>)>,
}

impl MeshCache {
    pub fn new() -> MeshCache {
        MeshCache::default()
    }

    /// Decode a texture, or hand back what was decoded before.
    ///
    /// A failure is remembered as a failure, so a missing file is attempted
    /// once rather than on every snapshot — which at sixty snapshots a second
    /// would be sixty failed opens a second while the user drags a slider.
    pub(crate) fn texture(
        &mut self,
        id: TextureId,
        source: &crate::textures::TextureSource,
    ) -> Option<Arc<Texture>> {
        if let Some(cached) = self.textures.get(&id) {
            return cached.clone();
        }
        let loaded = load_texture(source).map(Arc::new);
        self.textures.insert(id, loaded.clone());
        loaded
    }

    /// A Volume Map's grid, read once.
    pub(crate) fn volume(
        &mut self,
        id: TextureId,
        source: &crate::textures::TextureSource,
    ) -> Option<Arc<tequila_texture::volume::VolumeGrid>> {
        if let Some(cached) = self.volumes.get(&id) {
            return cached.clone();
        }
        let loaded = std::fs::read(&source.path)
            .ok()
            .and_then(|bytes| tequila_texture::volume::read_vdb(&bytes).ok())
            .map(Arc::new);
        self.volumes.insert(id, loaded.clone());
        loaded
    }

    /// A photometric profile, read once.
    fn photometry(
        &mut self,
        path: &str,
    ) -> Option<Arc<tequila_render_core::ies::IesProfile>> {
        if path.is_empty() {
            return None;
        }
        if let Some(cached) = self.photometry.get(path) {
            return cached.clone();
        }
        let loaded = tequila_render_core::ies::IesProfile::load(path)
            .ok()
            .map(Arc::new);
        self.photometry.insert(path.to_string(), loaded.clone());
        loaded
    }

    /// The ground quad.
    ///
    /// `stated` is the size the user asked for, and zero means "fit the
    /// scene", in which case `measure` supplies one. The two are cached
    /// differently on purpose:
    ///
    /// - A stated size is keyed on its bits, so typing a new one resizes the
    ///   floor at once.
    /// - A fitted size is computed once and then held until the cache is
    ///   cleared — which [`Command::scene_change`] arranges for exactly the
    ///   edits that can change the scene's extent. Re-measuring on every
    ///   snapshot would make the floor breathe while a part is dragged, and
    ///   would reallocate the mesh each frame, which costs far more than the
    ///   two triangles suggest: the backend compares `Arc` pointers to decide
    ///   whether it can update transforms in place, and one mismatch sends the
    ///   whole scene down the rebuild path.
    fn ground(&mut self, stated: f32, measure: impl FnOnce() -> f32) -> Option<Arc<RenderMesh>> {
        const FITTED: u32 = u32::MAX;
        let key = if stated > 0.0 { stated.to_bits() } else { FITTED };
        if let Some((cached, mesh)) = &self.ground {
            if *cached == key {
                return Some(mesh.clone());
            }
        }
        let extent = if stated > 0.0 { stated * 0.5 } else { measure() };
        if !(extent > 0.0) {
            return None;
        }
        let mesh = Arc::new(ground_mesh(extent));
        self.ground = Some((key, mesh.clone()));
        Some(mesh)
    }

    /// A quad standing where an area light is.
    fn light_proxy(
        &mut self,
        node: NodeId,
        light: &tequila_render_core::light::AreaLight,
    ) -> Option<Arc<RenderMesh>> {
        let key = [
            light.position.x,
            light.position.y,
            light.position.z,
            light.half_u.x,
            light.half_u.y,
            light.half_u.z,
            light.half_v.x,
            light.half_v.y,
            light.half_v.z,
        ];
        if let Some((cached, mesh)) = self.proxies.get(&node) {
            if *cached == key {
                return Some(mesh.clone());
            }
        }
        let normal = light.normal();
        let corner = |u: f32, v: f32| light.position + light.half_u * u + light.half_v * v;
        let mut mesh = RenderMesh::new(
            vec![
                corner(-1.0, -1.0),
                corner(1.0, -1.0),
                corner(1.0, 1.0),
                corner(-1.0, 1.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        );
        mesh.normals = vec![normal; 4];
        let mesh = Arc::new(mesh);
        self.proxies.insert(node, (key, mesh.clone()));
        Some(mesh)
    }

    /// Forget everything. Call when a command may have rewritten mesh data.
    pub fn clear(&mut self) {
        self.converted.clear();
        self.refit();
    }

    /// Take a mesh converted elsewhere - on an import's own thread - as the
    /// conversion of `key`, so the snapshot does not convert it again on the
    /// window's thread.
    pub fn adopt(&mut self, key: MeshKey, converted: Arc<RenderMesh>) {
        self.converted.insert(key, converted);
    }

    /// Forget what is built from the scene as a whole - the ground fitted
    /// to the model, the lights' stand-ins, their profiles - and keep the
    /// converted meshes. For an edit that changes what is in the scene or
    /// around it without rewriting any mesh: a light, the ground, a
    /// cutaway, a group, a delete.
    pub fn refit(&mut self) {
        self.ground = None;
        self.proxies.clear();
        self.photometry.clear();
    }

    pub fn len(&self) -> usize {
        self.converted.len()
    }

    pub fn is_empty(&self) -> bool {
        self.converted.is_empty()
    }
}

/// Build a snapshot, and the mapping back to the document.
///
/// Hidden parts are omitted rather than marked invisible: a hidden part should
/// cost nothing, and an instance excluded from the BVH costs nothing at all.
/// (`visible_to_camera` exists for the different case of §02's shadow-catcher
/// materials, which are invisible but still light the scene.)
pub fn snapshot(
    document: &SceneDocument,
    environment: Arc<Environment>,
    cache: &mut MeshCache,
) -> (SceneSnapshot, SnapshotMapping) {
    snapshot_with(document, environment, cache, &ViewOverrides::default())
}

/// How the view differs from the document, without the document changing:
/// nothing here is saved or undone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewOverrides {
    /// The graph editor's solo (`docs/04` §7): one node of one material
    /// shown on every part wearing it, unlit, in place of the material.
    /// A graph node's output shown in place of its material: the
    /// material, the node, and which of its outputs.
    pub solo: Option<(MaterialKey, tequila_materials::graph::NodeId, u8)>,
}

/// [`snapshot`], with the view's overrides applied.
pub fn snapshot_with(
    document: &SceneDocument,
    environment: Arc<Environment>,
    cache: &mut MeshCache,
    view: &ViewOverrides,
) -> (SceneSnapshot, SnapshotMapping) {
    let mut meshes: Vec<Arc<RenderMesh>> = Vec::new();
    let mut mesh_ids: BTreeMap<MeshKey, MeshId> = BTreeMap::new();
    let mut materials: Vec<Material> = Vec::new();
    let mut programs: Vec<Option<Arc<CompiledGraph>>> = Vec::new();
    let mut material_ids: BTreeMap<MaterialKey, MaterialId> = BTreeMap::new();
    let mut instances = Vec::new();
    // The document mesh behind each instance, for the displacement pass;
    // `None` for a light's stand-in.
    let mut instance_meshes: Vec<Option<MeshKey>> = Vec::new();
    let mut mapping = SnapshotMapping::default();

    // A part with no material assigned still has to render as something. Slot
    // 0 is a neutral grey — the "unassigned" look every DCC uses, and one a
    // user will recognize as "I have not set this yet" rather than mistake for
    // a material choice.
    materials.push(Material::plastic(
        tequila_core::spectrum::Spectrum::splat(0.5),
        0.3,
    ));
    programs.push(None);

    let mut lights = Vec::new();
    let mut light_nodes = Vec::new();
    let mut cutters = Vec::new();
    let placed = document.placements();

    for node in document.tree.iter() {
        if let NodeKind::Light { settings } = &node.kind {
            // Hiding a light turns it off. That is the one interpretation a
            // user can act on: an eye icon that dimmed the Scene tab row but
            // left the scene lit would read as a bug every time.
            if !placed.shown(node.id) {
                continue;
            }
            let world = placed.world(node.id);
            let profile = settings
                .photometry
                .as_ref()
                .and_then(|photometry| cache.photometry(&photometry.path));
            if let Some(light) = settings.to_render_light_with(&world, profile) {
                // A light the camera can see needs something to see. Only an
                // area light gets one: a point has no extent to draw, and a
                // spot's cone is a helper rather than a surface.
                if settings.visibility.camera {
                    if let tequila_render_core::light::LightKind::Area(area) = &light.kind {
                        if let Some(mesh) = cache.light_proxy(node.id, area) {
                            let mesh_id = MeshId(meshes.len() as u32);
                            meshes.push(mesh);
                            let material_id = MaterialId(materials.len() as u32);
                            materials.push(Material::emissive(
                                area.radiance,
                                area.radiance.luminance().max(0.0),
                            ));
                            programs.push(None);
                            instances.push(
                                Instance::new(
                                    InstanceId(instances.len() as u32),
                                    mesh_id,
                                    material_id,
                                )
                                .as_light_proxy()
                                .with_light_layer(settings.layer),
                            );
                            instance_meshes.push(None);
                        }
                    }
                }
                light_nodes.push(node.id);
                lights.push(light);
            }
            continue;
        }
        if let NodeKind::Cutaway { settings } = &node.kind {
            // Hidden or switched off, a cutter cuts nothing.
            if settings.enabled && placed.shown(node.id) {
                let mut cutter = tequila_render_core::cutaway::Cutter::new(
                    settings.render_shape(),
                    placed.world(node.id),
                );
                if let crate::cutaway::CapStyle::Color(color) = settings.cap {
                    cutter.cap = Some(MaterialId(materials.len() as u32));
                    // Matte, so the section reads as a flat colour from any
                    // angle rather than picking up the studio.
                    materials.push(Material::plastic(color, 0.6));
                    programs.push(None);
                }
                cutters.push(cutter);
            }
            continue;
        }
        let NodeKind::Part { mesh } = node.kind else {
            continue;
        };
        if !placed.shown(node.id) {
            continue;
        }
        let Some(source) = document.geometry.get(mesh) else {
            // A dangling mesh reference is a broken document, which `validate`
            // catches on load. Skipping keeps a live editing session from
            // panicking on the frame between two commands.
            continue;
        };

        // A part wearing the Cutaway material is not a surface but a cutter
        // of its own shape (`docs/02` §3): it leaves the scene as geometry
        // and joins it as a volume.
        let cuts = node
            .material
            .and_then(|key| document.materials.get(key))
            .is_some_and(|definition| {
                definition.material_type == tequila_materials::material_type::MaterialType::Cutaway
            });
        if cuts {
            let world = placed.world(node.id);
            let triangles: Vec<[tequila_core::math::Vec3; 3]> = source
                .indices
                .chunks_exact(3)
                .filter(|t| t.iter().all(|&i| (i as usize) < source.positions.len()))
                .map(|t| {
                    let at = |k: usize| world.point(source.positions[t[k] as usize]);
                    [at(0), at(1), at(2)]
                })
                .collect();
            let cutter = tequila_render_core::cutaway::CutMesh::new(triangles);
            if !cutter.is_empty() {
                cutters.push(tequila_render_core::cutaway::Cutter::new(
                    tequila_render_core::cutaway::CutterShape::Mesh(Arc::new(cutter)),
                    tequila_core::math::Transform::IDENTITY,
                ));
            }
            continue;
        }

        let mesh_id = *mesh_ids.entry(mesh).or_insert_with(|| {
            let id = MeshId(meshes.len() as u32);
            let converted = cache
                .converted
                .entry(mesh)
                .or_insert_with(|| Arc::new(to_render_mesh(source)));
            meshes.push(converted.clone());
            id
        });
        let material_id = match node.material {
            Some(key) => *material_ids.entry(key).or_insert_with(|| {
                let id = MaterialId(materials.len() as u32);
                let definition = document.materials.get(key);
                // Soloed: the material as the node alone, shown unlit.
                let soloed = view
                    .solo
                    .filter(|(material, _, _)| *material == key)
                    .and_then(|(_, node, output)| {
                        let original = definition?;
                        let graph = original.graph.soloed_at(node, output)?;
                        Some(crate::material::MaterialDefinition::from_graph(
                            &original.name,
                            original.material_type,
                            graph,
                        ))
                    });
                let definition = soloed.as_ref().or(definition);
                materials.push(
                    definition
                        .map(|d| d.material().clone())
                        .unwrap_or_default(),
                );
                // Compile only where there is a graph to compile. A material
                // that is just a parameter block gets `None`, and the
                // integrator's fast path then skips the machinery entirely.
                programs.push(definition.and_then(|d| compile_for(cache, key, d)));
                id
            }),
            None => MaterialId(0),
        };

        let instance_id = InstanceId(instances.len() as u32);
        instances.push(
            Instance::new(instance_id, mesh_id, material_id)
                .with_transform(placed.world(node.id)),
        );
        instance_meshes.push(Some(mesh));
        mapping.nodes.push(node.id);
        mapping.instances.insert(node.id, instance_id);
    }

    // Load whatever the compiled programs ask for. Failures are silent here
    // by design: the program carries a fallback for every texture, so a file
    // that is missing or unreadable leaves the material looking as it did
    // rather than turning the part black, and the browser is where a person is
    // told about it.
    let mut textures = TextureTable::new();
    for program in programs.iter().flatten() {
        for id in &program.requirements.textures {
            if textures.get(*id).is_some() {
                continue;
            }
            let Some(source) = document.textures.get(*id) else {
                continue;
            };
            if let Some(loaded) = cache.texture(*id, source) {
                textures.insert(*id, loaded);
            }
        }
    }

    // Volume Maps' grids, into the same table: the renderer fits them to
    // parts, and the Material tab's sphere to itself.
    for program in programs.iter().flatten() {
        for id in &program.requirements.volumes {
            if textures.volume(*id).is_some() {
                continue;
            }
            let Some(source) = document.textures.get(*id) else {
                continue;
            };
            if let Some(grid) = cache.volume(*id, source) {
                textures.insert_volume(*id, grid);
            }
        }
    }

    displace_instances(&mut meshes, &mut instances, &instance_meshes, &programs, &textures, cache);
    occlude_instances(&mut meshes, &mut instances, &programs, cache);
    let densities = density_fields(&meshes, &instances, &programs, &textures);
    add_bubbles(&mut meshes, &mut instances, &programs, cache);
    add_fuzz(&mut meshes, &mut instances, &programs, cache);

    // The ground goes in last, so its instance id is past every node's and
    // the mapping from instance to node stays a plain index. It has no node —
    // it is a property of the environment, not a thing in the scene tree —
    // and picking it must therefore select nothing rather than the last part.
    if let Some(mut catcher) = ground_catcher(&document.environment.ground) {
        catcher.reflection_fade = reflection_fade(document.environment.ground.reflection_length, || visible_bounds(document).max.y);
        if let Some(ground) = cache.ground(document.environment.ground.size, || {
            fitted_ground_extent(document)
        }) {
            let mesh_id = MeshId(meshes.len() as u32);
            meshes.push(ground);
            instances.push(
                Instance::new(InstanceId(instances.len() as u32), mesh_id, MaterialId(0))
                    .with_shadow_catcher(catcher),
            );
        }
    }

    // Conversions of meshes no part uses any more - a deleted model's - are
    // let go. Hidden parts keep theirs, so showing them again is free.
    if cache.converted.len() > mesh_ids.len() {
        let used: std::collections::BTreeSet<MeshKey> = document.tree.iter().filter_map(|node| node.mesh()).collect();
        cache.converted.retain(|key, _| used.contains(key));
    }

    mapping.lights = light_nodes;
    let scene = SceneSnapshot::new(meshes, instances, materials, environment)
        .with_programs(programs, Arc::new(textures))
        .with_densities(densities)
        .with_lights(Arc::new(LightSet::new(lights)))
        .with_cutters(cutters);
    (scene, mapping)
}

/// §6's Volume Map: for each part whose medium's Density is a Volume Map,
/// the grid fitted to the part's bounding box. After displacement, so the
/// box is the surface that bounds the medium; before bubbles, whose
/// instances are not parts.
fn density_fields(
    meshes: &[Arc<RenderMesh>],
    instances: &[Instance],
    programs: &[Option<Arc<CompiledGraph>>],
    textures: &TextureTable,
) -> Vec<Option<Arc<tequila_render_core::density::DensityField>>> {
    if !programs.iter().flatten().any(|p| !p.requirements.volumes.is_empty()) {
        return Vec::new();
    }
    instances
        .iter()
        .map(|instance| {
            let program = programs.get(instance.material.index())?.as_ref()?;
            let volume = tequila_render_core::density::volume_of(program)?;
            let grid = textures.volume(volume)?.clone();
            let mesh = meshes.get(instance.mesh.index())?;
            let mut low = tequila_core::math::Vec3::splat(f32::INFINITY);
            let mut high = tequila_core::math::Vec3::splat(f32::NEG_INFINITY);
            for p in &mesh.positions {
                low = low.min(*p);
                high = high.max(*p);
            }
            tequila_render_core::density::DensityField::fitted(grid, (low, high), instance.transform).map(Arc::new)
        })
        .collect()
}

/// The snapshot a render layer is rendered from: its own parts seen by the
/// camera, and everything else — the ground and the lights' stand-ins
/// included — hidden from the camera but still in the scene.
pub fn for_layer(
    scene: &SceneSnapshot,
    mapping: &SnapshotMapping,
    tree: &crate::tree::SceneTree,
    layer: &crate::render::RenderLayer,
) -> SceneSnapshot {
    let mut out = scene.clone();
    for (index, instance) in out.instances.iter_mut().enumerate() {
        let member = mapping.node_of(InstanceId(index as u32)).is_some_and(|node| {
            let mut chain = vec![node];
            chain.extend(tree.ancestors(node));
            layer.nodes.iter().any(|n| chain.contains(n))
        });
        if !member {
            instance.visible_to_camera = false;
        }
    }
    out
}

/// §6.2's Occlusion: give each part whose material has an Occlusion node a
/// copy of its mesh with the occlusion measured at the node's radius — in the
/// part's own units, so a part scaled up is measured over the same stretch
/// of scene. After displacement, so a displaced surface is occluded by its
/// own relief.
fn occlude_instances(
    meshes: &mut Vec<Arc<RenderMesh>>,
    instances: &mut [Instance],
    programs: &[Option<Arc<CompiledGraph>>],
    cache: &mut MeshCache,
) {
    let mut kept: BTreeMap<(usize, u32), (Arc<RenderMesh>, Arc<RenderMesh>)> = BTreeMap::new();
    let mut used: BTreeMap<(usize, u32), MeshId> = BTreeMap::new();
    for instance in instances.iter_mut() {
        let Some(program) = programs.get(instance.material.index()).and_then(|p| p.as_ref()) else {
            continue;
        };
        let Some(radius) = program.requirements.occlusion_radius() else {
            continue;
        };
        let local = radius * instance.transform.inv_length_scale();
        let base = meshes[instance.mesh.index()].clone();
        // To three significant figures, as displacement keys its scale.
        let key = (Arc::as_ptr(&base) as usize, format!("{local:.3e}").parse::<f32>().unwrap_or(local).to_bits());
        if let Some(id) = used.get(&key) {
            instance.mesh = *id;
            continue;
        }
        let measured = match cache.occluded.get(&key) {
            Some((source, mesh)) if Arc::ptr_eq(source, &base) => mesh.clone(),
            _ => {
                let mut copy = (*base).clone();
                copy.occlusion = crate::occlusion::vertex_occlusion(&base, local);
                Arc::new(copy)
            }
        };
        let id = MeshId(meshes.len() as u32);
        meshes.push(measured.clone());
        kept.insert(key, (base, measured));
        used.insert(key, id);
        instance.mesh = id;
    }
    cache.occluded = kept;
}

/// §4.4's Bubbles: for each part whose material has a Bubbles node, an
/// inside-out sphere instance per bubble, in the part's own material (see
/// [`crate::bubbles`]). Appended after the parts, so they belong to no node:
/// picking one picks nothing, as picking the ground does.
fn add_bubbles(
    meshes: &mut Vec<Arc<RenderMesh>>,
    instances: &mut Vec<Instance>,
    programs: &[Option<Arc<CompiledGraph>>],
    cache: &mut MeshCache,
) {
    use std::hash::{Hash, Hasher};
    let mut kept = BTreeMap::new();
    let mut sphere_id: Option<MeshId> = None;
    let parts = instances.len();
    for index in 0..parts {
        let (mesh_id, material, transform) = {
            let instance = &instances[index];
            (instance.mesh, instance.material, instance.transform)
        };
        let Some(request) = programs
            .get(material.index())
            .and_then(|p| p.as_ref())
            .and_then(|p| p.requirements.bubbles)
        else {
            continue;
        };
        let mesh = meshes[mesh_id.index()].clone();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (Arc::as_ptr(&mesh) as usize).hash(&mut hasher);
        for row in transform.matrix.m {
            for v in row {
                v.to_bits().hash(&mut hasher);
            }
        }
        request.hash(&mut hasher);
        let key = hasher.finish();
        let placed = match cache.bubbles.get(&key) {
            Some((source, placed)) if Arc::ptr_eq(source, &mesh) => placed.clone(),
            _ => {
                let triangles: Vec<[tequila_core::math::Vec3; 3]> = mesh
                    .indices
                    .chunks_exact(3)
                    .filter(|t| t.iter().all(|&i| (i as usize) < mesh.positions.len()))
                    .map(|t| {
                        let at = |k: usize| transform.point(mesh.positions[t[k] as usize]);
                        [at(0), at(1), at(2)]
                    })
                    .collect();
                let bounds = mesh.bounds().transformed(&transform);
                crate::bubbles::place(triangles, bounds, &request)
            }
        };
        if placed.is_empty() {
            kept.insert(key, (mesh, placed));
            continue;
        }
        let sphere = *sphere_id.get_or_insert_with(|| {
            let shared = cache
                .bubble_mesh
                .get_or_insert_with(|| Arc::new(crate::bubbles::bubble_mesh()))
                .clone();
            meshes.push(shared);
            MeshId(meshes.len() as u32 - 1)
        });
        for (centre, radius) in &placed {
            let place = tequila_core::math::Mat4::translation(*centre)
                * tequila_core::math::Mat4::scale(tequila_core::math::Vec3::splat(*radius));
            let Some(place) = tequila_core::math::Transform::from_matrix(place) else { continue };
            instances.push(
                Instance::new(InstanceId(instances.len() as u32), sphere, material).with_transform(place),
            );
        }
        kept.insert(key, (mesh, placed));
    }
    cache.bubbles = kept;
}

/// §4.4's Fuzz: for each part whose material has a Fuzz node, its fibres as
/// one more instance, in the part's place and its material (see
/// [`crate::fuzz`]). Appended after the parts, as bubbles are, so picking a
/// fibre picks nothing.
fn add_fuzz(
    meshes: &mut Vec<Arc<RenderMesh>>,
    instances: &mut Vec<Instance>,
    programs: &[Option<Arc<CompiledGraph>>],
    cache: &mut MeshCache,
) {
    use std::hash::{Hash, Hasher};
    let mut kept = BTreeMap::new();
    let parts = instances.len();
    for index in 0..parts {
        let (mesh_id, material, transform) = {
            let instance = &instances[index];
            (instance.mesh, instance.material, instance.transform)
        };
        let Some(request) = programs
            .get(material.index())
            .and_then(|p| p.as_ref())
            .and_then(|p| p.requirements.fuzz)
        else {
            continue;
        };
        let mesh = meshes[mesh_id.index()].clone();
        let scale = 1.0 / transform.inv_length_scale();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (Arc::as_ptr(&mesh) as usize).hash(&mut hasher);
        scale.to_bits().hash(&mut hasher);
        request.hash(&mut hasher);
        let key = hasher.finish();
        let grown = match cache.fuzz.get(&key) {
            Some((source, grown)) if Arc::ptr_eq(source, &mesh) => grown.clone(),
            _ => crate::fuzz::grow(&mesh, scale, &request).map(Arc::new),
        };
        if let Some(fur) = &grown {
            meshes.push(fur.clone());
            let id = MeshId(meshes.len() as u32 - 1);
            instances.push(Instance::new(InstanceId(instances.len() as u32), id, material).with_transform(transform));
        }
        kept.insert(key, (mesh, grown));
    }
    cache.fuzz = kept;
}

/// §4.4's Displace: give each part whose material drives Displacement a
/// subdivided, displaced copy of its mesh (see [`crate::displacement`]).
///
/// After the textures are loaded, because the height may read them. Keyed by
/// the mesh, the height field's fingerprint, the detail and the part's
/// scale, so two parts sharing all four share one displaced mesh, and an edit
/// that changes none of them reuses it.
fn displace_instances(
    meshes: &mut Vec<Arc<RenderMesh>>,
    instances: &mut [Instance],
    instance_meshes: &[Option<MeshKey>],
    programs: &[Option<Arc<CompiledGraph>>],
    textures: &TextureTable,
    cache: &mut MeshCache,
) {
    use std::hash::{Hash, Hasher};
    let mut used: BTreeMap<(MeshKey, u64), MeshId> = BTreeMap::new();
    let mut kept: BTreeMap<(MeshKey, u64), Arc<RenderMesh>> = BTreeMap::new();
    for (instance, source) in instances.iter_mut().zip(instance_meshes) {
        let Some(mesh_key) = source else { continue };
        let Some(program) = programs.get(instance.material.index()).and_then(|p| p.as_ref()) else {
            continue;
        };
        let detail = program.constants.displacement_detail;
        if detail <= 0.0 {
            // The bump-only approximation: nothing to build.
            continue;
        }
        let Some(displacer) = crate::displacement::Displacer::of(program) else {
            continue;
        };
        let scale = 1.0 / instance.transform.inv_length_scale();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        displacer.fingerprint().hash(&mut hasher);
        detail.to_bits().hash(&mut hasher);
        // To three significant figures: a part nudged in size by a rounding
        // error keeps its mesh.
        format!("{scale:.3e}").hash(&mut hasher);
        let key = (*mesh_key, hasher.finish());

        if let Some(id) = used.get(&key) {
            instance.mesh = *id;
            continue;
        }
        let displaced = match cache.displaced.get(&key) {
            Some(mesh) => mesh.clone(),
            None => {
                let base = &meshes[instance.mesh.index()];
                Arc::new(crate::displacement::displace(base, &displacer, textures, scale, detail))
            }
        };
        let id = MeshId(meshes.len() as u32);
        meshes.push(displaced.clone());
        kept.insert(key, displaced);
        used.insert(key, id);
        instance.mesh = id;
    }
    cache.displaced = kept;
}

/// Compile a material's graph, reusing the last result when its topology and
/// values are unchanged.
///
/// A snapshot is rebuilt on every edit, and recompiling every graph each time
/// would put a compile on the interactive path for materials nobody touched.
fn compile_for(
    cache: &mut MeshCache,
    key: MaterialKey,
    definition: &crate::material::MaterialDefinition,
) -> Option<Arc<CompiledGraph>> {
    // A single-node graph has nothing to run; saying so here is what keeps the
    // integrator's fast path on for the overwhelming majority of materials.
    if definition.graph.len() <= 1 && definition.graph.decals.is_empty() {
        return None;
    }
    let content = definition.graph.content_hash();
    if let Some((cached_hash, program)) = cache.programs.get(&key) {
        if *cached_hash == content {
            return Some(program.clone());
        }
    }
    let compiled = Arc::new(definition.graph.compile().ok()?);
    cache.programs.insert(key, (content, compiled.clone()));
    Some(compiled)
}

/// The document's mesh in the renderer's layout.
///
/// The two types differ deliberately: the document's carries several UV sets,
/// tangents and submeshes because a user edits those, and the renderer's
/// carries the flat arrays its inner loop reads. Only UV set 0 crosses over,
/// which is all the M1 shading model uses.
/// A document mesh in the renderer's form: what a snapshot converts each
/// mesh to, for converting one ahead of it, off the window's thread.
pub fn render_mesh_of(mesh: &crate::geometry::Mesh) -> RenderMesh {
    to_render_mesh(mesh)
}

fn to_render_mesh(mesh: &crate::geometry::Mesh) -> RenderMesh {
    let mut out = RenderMesh::new(mesh.positions.clone(), mesh.indices.clone());
    out.normals = mesh.normals.clone();
    if mesh.colors.len() == mesh.positions.len() {
        out.colors = mesh.colors.clone();
    }
    out.edge_lines = mesh.edge_lines.clone();
    out.curvature = crate::geometry::vertex_curvature(&mesh.positions, &mesh.normals, &mesh.indices);
    if let Some(uvs) = mesh.uv_sets.first() {
        out.uvs = uvs.clone();
        // Regenerated rather than copied from the document: tangents saved
        // by an earlier version used the opposite handedness.
        if let Some(tangents) =
            crate::geometry::tangent_frames(&mesh.positions, &mesh.normals, uvs, &mesh.indices)
        {
            out.tangents = tangents;
        }
    }
    out
}

/// The renderer's form of the ground settings, or `None` when the ground is
/// switched off entirely.
fn ground_catcher(ground: &crate::document::GroundSettings) -> Option<ShadowCatcher> {
    // The master switch, checked first. A floor that is switched off has no
    // instance in the snapshot at all, which is what lets a product be
    // rendered in mid-air with the background showing through underneath it.
    if !ground.enabled {
        return None;
    }
    if !ground.shadows && ground.reflections <= 0.0 {
        return None;
    }
    Some(ShadowCatcher {
        shadow_color: ground.shadow_color,
        reflections: ground.reflections,
        reflection_roughness: ground.reflection_blur,
        reflection_fade: None,
        reflection_fresnel: ground.reflection_fresnel,
        // Occlusion instead of a cast shadow, not as well as: the two are
        // alternative ways of saying the product sits on the floor, and
        // showing both puts two different shadows under one object.
        occlusion_only: ground.occlusion_shadows,
        occlusion_radius: 1.0,
    })
}

/// The height a reflection fades out by, for a length of 0 to 1 of the
/// product's height (`top`, the floor being at zero): `None` at 1, where it
/// does not fade at all. `length / (1 - length)` of the height, so the
/// slider runs smoothly into "all of it" rather than jumping there - half
/// is gone by the top of the product, a quarter by a third of the way up.
pub fn reflection_fade(length: f32, top: impl FnOnce() -> f32) -> Option<f32> {
    if !(length < 1.0) {
        return None;
    }
    let height = top();
    let height = if height.is_finite() && height > 0.0 { height } else { 1.0 };
    let length = length.max(0.0);
    Some((height * length / (1.0 - length)).max(height * 1e-4))
}

/// Half-width of a ground quad that fits the scene.
///
/// Fitting is the default because a fixed size is either smaller than the
/// product or a mirror stretching to the horizon, and neither is ever what was
/// meant. The multiple is generous because a shadow falls *outside* the object
/// that casts it.
fn fitted_ground_extent(document: &SceneDocument) -> f32 {
    let bounds = visible_bounds(document);
    if bounds.is_empty() {
        return 0.0;
    }
    (bounds.diagonal().length() * 3.0).max(1e-3)
}

/// A quad in the ground plane, wound so its normal points up.
fn ground_mesh(extent: f32) -> RenderMesh {
    let mut mesh = RenderMesh::new(
        vec![
            tequila_core::math::vec3(-extent, 0.0, -extent),
            tequila_core::math::vec3(extent, 0.0, -extent),
            tequila_core::math::vec3(extent, 0.0, extent),
            tequila_core::math::vec3(-extent, 0.0, extent),
        ],
        vec![0, 2, 1, 0, 3, 2],
    );
    mesh.normals = vec![tequila_core::math::Vec3::Y; 4];
    mesh
}

/// World-space bounds of everything a snapshot would contain, for framing the
/// camera. Empty when nothing is visible.
pub fn visible_bounds(document: &SceneDocument) -> tequila_core::math::Aabb {
    let mut bounds = tequila_core::math::Aabb::EMPTY;
    let placed = document.placements();
    for node in document.tree.iter() {
        let NodeKind::Part { mesh } = node.kind else {
            continue;
        };
        if !placed.shown(node.id) {
            continue;
        }
        if let Some(source) = document.geometry.get(mesh) {
            bounds.extend(&source.bounds().transformed(&placed.world(node.id)));
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{reserve_keys, Command, ImportPayload};
    use crate::geometry::Mesh;
    use crate::material::MaterialDefinition;
    use crate::tree::SceneNode;
    use crate::CommandBus;

    /// Reflection length as the height it fades out by: never at full
    /// length, the product's height at half, a third of it at a quarter,
    /// and above zero even at nothing - and a sensible height for an empty
    /// scene's infinite bounds.
    #[test]
    fn a_reflection_length_is_a_fade_height() {
        assert_eq!(reflection_fade(1.0, || 2.0), None);
        assert_eq!(reflection_fade(0.5, || 2.0), Some(2.0));
        assert!((reflection_fade(0.25, || 3.0).unwrap() - 1.0).abs() < 1e-6);
        assert!(reflection_fade(0.0, || 2.0).unwrap() > 0.0);
        assert_eq!(reflection_fade(0.5, || f32::NEG_INFINITY), Some(1.0));
    }

    /// The setting reaches the floor the renderer gets: a fade at the height
    /// the length says, and none at full length.
    #[test]
    fn a_short_reflection_reaches_the_floor() {
        let mut document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.2);
        document.environment.ground.reflections = 0.6;
        let floor = |document: &SceneDocument| {
            let (scene, _) = snapshot(document, crate::environment::EnvironmentCache::new().environment(&document.environment), &mut MeshCache::new());
            scene.instances.iter().find_map(|i| i.shadow_catcher).expect("a floor")
        };
        assert_eq!(floor(&document).reflection_fade, None);
        document.environment.ground.reflection_length = 0.5;
        let top = visible_bounds(&document).max.y;
        assert_eq!(floor(&document).reflection_fade, Some(top));
    }
    use tequila_core::math::{vec3, Transform, Vec3};

    fn environment() -> Arc<Environment> {
        Arc::new(crate::demo::studio_environment())
    }

    /// One mesh, `count` parts placed a metre apart, under one group.
    fn assembly(count: usize) -> (SceneDocument, CommandBus, Vec<NodeId>) {
        let mut document = SceneDocument::new();
        // The ground is on by default, and these tests are about parts: an
        // extra instance in every count would obscure what each one is
        // actually asserting. `the_ground_is_one_extra_instance` covers it
        // directly instead.
        document.environment.ground.shadows = false;
        let mut bus = CommandBus::new();
        let mesh = Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        let (meshes, materials, ids) = reserve_keys(&document, 1, 1, count + 1);

        let mut root = SceneNode::new(ids[0], "Assembly", NodeKind::Group);
        root.children = ids[1..].to_vec();
        let mut nodes = vec![root];
        for (i, id) in ids[1..].iter().enumerate() {
            let mut node =
                SceneNode::new(*id, format!("Part {i}"), NodeKind::Part { mesh: meshes[0] });
            node.parent = Some(ids[0]);
            node.transform = Transform::translation(vec3(i as f32, 0.0, 0.0));
            node.material = Some(materials[0]);
            nodes.push(node);
        }

        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: vec![(materials[0], MaterialDefinition::plastic("Shell"))],
                    nodes,
                    roots: vec![ids[0]],
                }),
            },
        )
        .unwrap();
        (document, bus, ids)
    }

    /// §2.2's rule, at the flattening step: many parts, one mesh.
    #[test]
    fn identical_parts_upload_one_mesh_and_many_instances() {
        let (document, _, _) = assembly(400);
        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());

        assert_eq!(scene.meshes.len(), 1, "the mesh was copied per part");
        assert_eq!(scene.instances.len(), 400);
        assert_eq!(mapping.len(), 400);
    }

    #[test]
    fn instances_carry_their_world_transform() {
        let (document, _, ids) = assembly(3);
        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());

        let instance = mapping.instance_of(ids[2]).unwrap();
        let transform = scene.instances[instance.0 as usize].transform;
        assert!(
            (transform.point(Vec3::ZERO) - vec3(1.0, 0.0, 0.0)).length() < 1e-5,
            "at {:?}",
            transform.point(Vec3::ZERO)
        );
    }

    /// A transform on a group must reach its children, or moving an assembly
    /// moves nothing.
    #[test]
    fn a_group_transform_reaches_its_children() {
        let (mut document, mut bus, ids) = assembly(2);
        bus.apply(
            &mut document,
            Command::set_transform(ids[0], Transform::translation(vec3(0.0, 10.0, 0.0))),
        )
        .unwrap();

        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());
        let instance = mapping.instance_of(ids[1]).unwrap();
        let at = scene.instances[instance.0 as usize]
            .transform
            .point(Vec3::ZERO);
        assert!(
            (at.y - 10.0).abs() < 1e-5,
            "the group transform was lost: {at:?}"
        );
    }

    #[test]
    fn hidden_parts_are_left_out_entirely() {
        let (mut document, mut bus, ids) = assembly(3);
        bus.apply(&mut document, Command::set_visibility(ids[1], false))
            .unwrap();

        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());
        assert_eq!(scene.instances.len(), 2);
        assert_eq!(mapping.instance_of(ids[1]), None);
        assert!(mapping.instance_of(ids[2]).is_some());
    }

    /// Hiding a group hides everything under it.
    #[test]
    fn hiding_a_group_hides_its_children() {
        let (mut document, mut bus, ids) = assembly(3);
        bus.apply(&mut document, Command::set_visibility(ids[0], false))
            .unwrap();

        let (scene, _) = snapshot(&document, environment(), &mut MeshCache::new());
        assert!(scene.instances.is_empty());
    }

    /// The mapping is what keeps the viewport and the Scene tab in step
    /// (§04 §4.1), so it must round-trip in both directions.
    #[test]
    fn the_mapping_round_trips_between_nodes_and_instances() {
        let (document, _, ids) = assembly(5);
        let (_, mapping) = snapshot(&document, environment(), &mut MeshCache::new());

        for &id in &ids[1..] {
            let instance = mapping.instance_of(id).expect("every part has an instance");
            assert_eq!(mapping.node_of(instance), Some(id));
        }
        // Groups produce no instance.
        assert_eq!(mapping.instance_of(ids[0]), None);
        assert_eq!(mapping.node_of(InstanceId(999)), None);
    }

    #[test]
    fn parts_with_no_material_get_the_neutral_slot() {
        let (mut document, mut bus, ids) = assembly(2);
        // Clearing an assignment is done by assigning nothing; the command set
        // has no "unassign", so this reaches for the closest equivalent: a
        // fresh group has no material, and adding a part under it exercises
        // the same path.
        bus.apply(
            &mut document,
            Command::AddGroup {
                parent: None,
                name: "Bare".into(),
            },
        )
        .unwrap();

        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());
        // Slot 0 is the neutral material; the assembly's parts use slot 1.
        let instance = mapping.instance_of(ids[1]).unwrap();
        assert_eq!(scene.instances[instance.0 as usize].material, MaterialId(1));
        assert!(scene.materials.len() >= 2);
    }

    /// Two materials must not collapse into one slot, and one material used by
    /// many parts must not be uploaded twice.
    #[test]
    fn materials_are_deduplicated_but_not_merged() {
        let (mut document, mut bus, ids) = assembly(4);
        let existing = document.materials.keys().next().unwrap();
        bus.apply(&mut document, Command::UnlinkMaterial { node: ids[1] })
            .unwrap();
        let unlinked = document.tree.get(ids[1]).unwrap().material.unwrap();
        assert_ne!(unlinked, existing);

        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());
        // Neutral + the original + the unlinked copy.
        assert_eq!(scene.materials.len(), 3);

        let slot =
            |id: NodeId| scene.instances[mapping.instance_of(id).unwrap().0 as usize].material;
        assert_ne!(slot(ids[1]), slot(ids[2]));
        assert_eq!(slot(ids[2]), slot(ids[3]), "one material, one slot");
    }

    /// The reason the cache exists: a scene that only moved must hand the
    /// renderer the same mesh allocations, so the CPU backend can update
    /// instance transforms in place instead of rebuilding every BVH.
    ///
    /// This is asserted by pointer identity because that is exactly what the
    /// backend compares. Without it the slow path is taken silently — nothing
    /// fails, the render is just an order of magnitude more expensive.
    #[test]
    fn a_transform_only_change_reuses_the_converted_meshes() {
        let (mut document, mut bus, ids) = assembly(3);
        let mut cache = MeshCache::new();

        let (first, _) = snapshot(&document, environment(), &mut cache);
        bus.apply(
            &mut document,
            Command::set_transform(ids[1], Transform::translation(vec3(0.0, 5.0, 0.0))),
        )
        .unwrap();
        let (second, _) = snapshot(&document, environment(), &mut cache);

        assert_eq!(first.meshes.len(), 1);
        assert!(
            Arc::ptr_eq(&first.meshes[0], &second.meshes[0]),
            "the mesh was reconverted for a move"
        );
        // And the instance really did move, so this is not reuse by accident.
        assert_ne!(first.instances[0].transform, second.instances[0].transform);
    }

    /// The shadow catcher is one instance with no node behind it: it belongs
    /// to the environment, not to the scene tree, so picking it must select
    /// nothing rather than whichever part happens to share its index.
    /// Switching the ground off leaves no floor in the snapshot at all.
    ///
    /// Not a hidden instance and not a transparent one: nothing. A product
    /// rendered in mid-air needs the background visible underneath it and the
    /// alpha channel empty there, and both follow from the instance simply not
    /// existing.
    #[test]
    fn a_disabled_ground_produces_no_instance() {
        let (mut document, _, _) = assembly(3);
        document.environment.ground.enabled = true;
        document.environment.ground.shadows = true;

        let (with, _) = snapshot(&document, environment(), &mut MeshCache::new());
        assert_eq!(
            with.instances
                .iter()
                .filter(|i| i.shadow_catcher.is_some())
                .count(),
            1,
            "this scene should have a ground to switch off"
        );

        document.environment.ground.enabled = false;
        let (without, _) = snapshot(&document, environment(), &mut MeshCache::new());
        assert!(
            without.instances.iter().all(|i| i.shadow_catcher.is_none()),
            "a disabled ground still put an instance in the scene"
        );
        assert_eq!(
            without.instances.len(),
            with.instances.len() - 1,
            "switching the ground off changed something other than the ground"
        );
    }

    /// And the shadow flag is no longer the master switch. Shadows off with
    /// reflections on still leaves a floor, because a mirror floor with no cast
    /// shadow is a real thing somebody might ask for — and because a control
    /// named for one thing must not silently be the switch for another.
    #[test]
    fn shadows_off_is_not_the_same_as_no_ground() {
        let (mut document, _, _) = assembly(3);
        document.environment.ground.enabled = true;
        document.environment.ground.shadows = false;
        document.environment.ground.reflections = 0.5;

        let (scene, _) = snapshot(&document, environment(), &mut MeshCache::new());
        assert_eq!(
            scene
                .instances
                .iter()
                .filter(|i| i.shadow_catcher.is_some())
                .count(),
            1,
            "turning shadows off removed the floor"
        );
    }

    #[test]
    fn the_ground_is_one_extra_instance_that_maps_to_no_node() {
        let (mut document, _, _) = assembly(3);
        document.environment.ground.shadows = true;

        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());
        assert_eq!(scene.instances.len(), 4);
        assert_eq!(mapping.len(), 3);

        let ground = scene.instances.last().expect("the ground");
        assert!(ground.shadow_catcher.is_some());
        assert_eq!(mapping.node_of(ground.id), None);

        // And switching it off takes it away again, rather than leaving an
        // invisible surface the product can still be occluded by.
        document.environment.ground.shadows = false;
        let (without, _) = snapshot(&document, environment(), &mut MeshCache::new());
        assert_eq!(without.instances.len(), 3);
    }

    /// Clearing is the caller's job, and it must actually take effect —
    /// otherwise an edited mesh renders as its previous version.
    #[test]
    fn clearing_the_cache_reconverts() {
        let (document, _, _) = assembly(2);
        let mut cache = MeshCache::new();

        let (first, _) = snapshot(&document, environment(), &mut cache);
        assert_eq!(cache.len(), 1);

        cache.clear();
        assert!(cache.is_empty());
        let (second, _) = snapshot(&document, environment(), &mut cache);
        assert!(
            !Arc::ptr_eq(&first.meshes[0], &second.meshes[0]),
            "a cleared cache still returned the old conversion"
        );
    }

    #[test]
    fn an_empty_document_makes_an_empty_snapshot() {
        let document = SceneDocument::new();
        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());
        assert!(scene.instances.is_empty());
        assert!(mapping.is_empty());
    }

    /// A part whose material drives Displacement renders a subdivided,
    /// displaced mesh; an edit that does not touch the height reuses it; and
    /// a detail of zero builds no geometry and shades the height as a bump.
    #[test]
    fn displacement_builds_geometry_and_detail_zero_is_bump() {
        use crate::command::{Command, MaterialParameter};
        use tequila_materials::material_type::Param;
        let mut document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.2);
        let mut bus = crate::CommandBus::new();
        let (scene, mapping) = snapshot(&document, environment(), &mut MeshCache::new());
        let sphere_node = mapping.nodes[0];
        let material = document.tree.get(sphere_node).unwrap().material.expect("the sphere has a material");
        let plain = scene.meshes[scene.instances[0].mesh.index()].triangle_count();

        let noise = tequila_texture::procedural::Procedural::catalog()[0];
        bus.apply(
            &mut document,
            Command::SetParameterProcedural { material, param: Param::Displacement, procedural: Some(Box::new(noise)), seed: 3 },
        )
        .unwrap();
        bus.apply(
            &mut document,
            Command::SetMaterialParameter { material, parameter: MaterialParameter::scalar(Param::DisplacementDetail, 0.02) },
        )
        .unwrap();
        let mut cache = MeshCache::new();
        let (scene, _) = snapshot(&document, environment(), &mut cache);
        let displaced = &scene.meshes[scene.instances[0].mesh.index()];
        assert!(displaced.triangle_count() > plain * 4, "{} triangles against {plain}", displaced.triangle_count());
        let before = Arc::as_ptr(displaced);

        // A colour change leaves the height field alone, and the mesh with it.
        bus.apply(
            &mut document,
            Command::SetMaterialParameter { material, parameter: MaterialParameter::color(Param::BaseColor, tequila_core::spectrum::Spectrum::splat(0.3)) },
        )
        .unwrap();
        let (scene, _) = snapshot(&document, environment(), &mut cache);
        assert_eq!(Arc::as_ptr(&scene.meshes[scene.instances[0].mesh.index()]), before, "an unrelated edit rebuilt the displacement");

        // Detail zero: the plain mesh, and the height becomes the bump.
        bus.apply(
            &mut document,
            Command::SetMaterialParameter { material, parameter: MaterialParameter::scalar(Param::DisplacementDetail, 0.0) },
        )
        .unwrap();
        let (scene, _) = snapshot(&document, environment(), &mut cache);
        assert_eq!(scene.meshes[scene.instances[0].mesh.index()].triangle_count(), plain);
        assert!(
            matches!(scene.bump(scene.instances[0].material), Some(tequila_render_core::surface_bump::BumpSource::Height { .. })),
            "the bump-only approximation has no bump"
        );
    }

    /// The renderer's meshes carry their curvature: the demo sphere reads
    /// about one over its radius, which is what the Curvature node scales.
    #[test]
    fn render_meshes_carry_their_curvature() {
        let document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.2);
        let (scene, _) = snapshot(&document, environment(), &mut MeshCache::new());
        let sphere = &scene.meshes[scene.instances[0].mesh.index()];
        assert_eq!(sphere.curvature.len(), sphere.positions.len());
        let bounds = sphere.bounds();
        let radius = (bounds.max.x - bounds.min.x) * 0.5;
        let mut sorted = sphere.curvature.clone();
        sorted.sort_by(f32::total_cmp);
        let median = sorted[sorted.len() / 2];
        assert!((median * radius - 1.0).abs() < 0.1, "median curvature {median} on a sphere of radius {radius}");
    }

    /// A normal map or a data map in an 8-bit file is read as the numbers
    /// in it. It was read through the sRGB curve and then relabelled Raw, so
    /// a flat normal's 128 came in as 0.22 instead of 0.5 and every normal
    /// map leaned the whole surface one way, and a glTF roughness map read
    /// every roughness far too low.
    #[test]
    fn data_and_normal_maps_keep_their_numbers() {
        use tequila_texture::texture::MapRole;
        let dir = std::env::temp_dir().join(format!("tequila-raw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("flat_normal.png");
        {
            let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), 2, 2);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[128, 128, 255].repeat(4)).unwrap();
        }
        let texel = |role: MapRole| {
            let texture = load_texture(&crate::textures::TextureSource::new(path.clone(), role)).expect("loads");
            let base = texture.pyramid().level(0);
            (base.data[0], base.data[2])
        };
        for role in [MapRole::Normal, MapRole::Data, MapRole::Height] {
            let (r, b) = texel(role);
            assert!((r - 128.0 / 255.0).abs() < 1e-4 && (b - 1.0).abs() < 1e-4, "{role:?} read 128 as {r}");
        }
        // Colour still has the curve removed: 128 in sRGB is about 0.216 linear.
        let (r, _) = texel(MapRole::Color);
        assert!((r - 0.2158).abs() < 2e-3, "colour read 128 as {r}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn visible_bounds_cover_the_visible_parts_only() {
        let (mut document, mut bus, ids) = assembly(3);
        let full = visible_bounds(&document);
        assert!((full.max.x - 3.0).abs() < 1e-4, "bounds {full:?}");

        bus.apply(&mut document, Command::set_visibility(ids[3], false))
            .unwrap();
        let reduced = visible_bounds(&document);
        assert!(
            reduced.max.x < full.max.x,
            "hiding the far part did not shrink the bounds"
        );
    }
}

/// Read a texture file and build its pyramid, honouring the map role (§5.2).
///
/// Public for the library's thumbnails, which read a preset's pictures
/// without a document to hold them.
pub fn load_texture(source: &crate::textures::TextureSource) -> Option<Texture> {
    use tequila_texture::image::ColorSpace;

    // Numbers are read as numbers. `Image::load` removes the sRGB curve from
    // 8-bit files, and relabelling its result Raw left that decode in place:
    // every normal map leaned and every roughness map read low.
    let image = if source.effective_color_space() == ColorSpace::Raw {
        tequila_texture::image::Image::load_raw(&source.path).ok()?
    } else {
        tequila_texture::image::Image::load(&source.path).ok()?
    };
    Some(Texture::from_linear(
        image,
        source.role,
        tequila_texture::projection::Mapping::default(),
    ))
}
