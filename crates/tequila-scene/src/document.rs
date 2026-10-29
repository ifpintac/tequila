//! The Scene Document.
//!
//! `docs/03-SCENE-AND-IO.md` §1: "The Scene Document is the single source of
//! truth. Renderer, UI, scripting, and exporters all read from it; only the
//! command bus writes to it."
//!
//! That last clause is enforced structurally. The mutating entry points are
//! `pub(crate)`, so nothing outside this crate can reach them, and the only
//! public path to a mutation is [`crate::command::Command`]. Undo, scripting
//! and the command journal then work for free rather than needing every call
//! site to remember them.

use crate::geometry::GeometryStore;
use crate::material::MaterialStore;
use crate::textures::TextureStore;
use crate::tree::{MaterialKey, MeshKey, NodeId, NodeKind, SceneTree, TreeError};
use crate::units::SceneUnits;
use tequila_core::math::Aabb;
use tequila_core::spectrum::Spectrum;
use tequila_color::ImageStyle;
use tequila_render_core::settings::LightingSettings;
use crate::lighting::CameraStore;
use crate::render::RenderSettings;

/// The current `.tequila` format version.
///
/// Semver on the format, independent of the application's own version
/// (`docs/05` §5). Recorded in every file so a loader can migrate.
pub const FORMAT_VERSION: &str = "1.0.0";

/// Document-level metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentMeta {
    pub format_version: String,
    pub application_version: String,
    /// Free-form, set by the user. Not a file path: a document that has never
    /// been saved still has a name.
    pub name: String,
}

impl Default for DocumentMeta {
    fn default() -> Self {
        DocumentMeta {
            format_version: FORMAT_VERSION.into(),
            application_version: env!("CARGO_PKG_VERSION").into(),
            name: "Untitled".into(),
        }
    }
}

/// Environment settings, as stored in the document.
///
/// The renderer's [`Environment`](tequila_render_core::environment::Environment)
/// is a *built* object with a CDF; this is the description it is built from,
/// which is what belongs in a file.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentSettings {
    /// Path to an HDR or EXR file, or `None` for the built-in studio setup.
    pub image: Option<String>,
    pub rotation: f32,
    pub brightness: f32,
    /// Separation of dark and bright regions, as a power about the image's
    /// mean luminance (§1.5). 1.0 leaves the image alone.
    pub contrast: f32,
    /// Diameter of the environment dome in scene units. Zero means the dome is
    /// infinitely far away, which is the usual assumption and the one that
    /// makes lighting independent of where a part stands.
    pub size: f32,
    /// Vertical offset of the dome's centre relative to the ground.
    pub height: f32,
    /// A physical sky to light the scene with, instead of an image.
    ///
    /// Takes precedence over [`EnvironmentSettings::image`] when set, because
    /// a sky *is* the environment rather than an adjustment to one. Pins
    /// composite over it exactly as they do over a studio or a loaded HDRI.
    pub sky: Option<tequila_render_core::sky::Sky>,
    /// Lights painted onto the environment, bottom to top (`docs/01` §1.5).
    ///
    /// Part of the environment rather than of the scene tree, because that is
    /// what they are: paint on the sphere, composited into the image before
    /// its CDF is built. A pin is not a node and does not move with anything.
    pub pins: Vec<tequila_render_core::pins::Pin>,
    pub background: BackgroundMode,
    /// Used when `background` is [`BackgroundMode::Color`].
    pub background_color: Spectrum,
    /// Used when `background` is [`BackgroundMode::Backplate`].
    pub backplate: Option<String>,
    pub ground: GroundSettings,
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        EnvironmentSettings {
            image: None,
            rotation: 0.0,
            brightness: 1.0,
            contrast: 1.0,
            size: 0.0,
            height: 0.0,
            sky: None,
            pins: Vec::new(),
            background: BackgroundMode::Environment,
            background_color: Spectrum::splat(0.05),
            backplate: None,
            ground: GroundSettings::default(),
        }
    }
}

/// What the camera sees where no geometry is (§1.5).
///
/// Stored separately from the renderer's
/// [`Background`](tequila_render_core::environment::Background) because that
/// one carries a decoded backplate image and this one carries a path: a
/// document holds descriptions, not pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundMode {
    /// The lighting environment itself (hotkey `E`).
    Environment,
    /// A solid colour (`C`).
    Color,
    /// A backplate image, which lights nothing (`B`).
    Backplate,
}

impl BackgroundMode {
    pub const ALL: [BackgroundMode; 3] = [
        BackgroundMode::Environment,
        BackgroundMode::Color,
        BackgroundMode::Backplate,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BackgroundMode::Environment => "Environment",
            BackgroundMode::Color => "Color",
            BackgroundMode::Backplate => "Backplate",
        }
    }
}

/// The ground plane (§1.5).
///
/// A product render almost always stands on something, and the something is
/// nearly always a shadow catcher rather than a modelled floor: it has to take
/// shadow and reflection from the product while leaving the backplate or the
/// environment visible through it, which no ordinary surface does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundSettings {
    /// Whether there is a floor at all.
    ///
    /// Separate from [`shadows`](Self::shadows), and it did not used to be:
    /// the shadow flag doubled as the master switch, so the only way to render
    /// a product in mid-air was to find a checkbox labelled "Ground shadows"
    /// and reason that turning shadows off also removes the floor. Nobody
    /// reasons that. A control named for one thing must not silently be the
    /// switch for another.
    pub enabled: bool,
    /// Whether the floor catches a shadow.
    pub shadows: bool,
    /// Tint of the caught shadow.
    pub shadow_color: Spectrum,
    /// Contact/ambient-occlusion darkening instead of a cast shadow. Useful
    /// where the lighting is so soft that a drop shadow barely registers but
    /// the product still needs to sit on the floor rather than float.
    pub occlusion_shadows: bool,
    /// How much of a mirror the floor is, 0 to 1. Only what stands on it is
    /// reflected - the product, other parts, lights - never the environment,
    /// so the floor itself never shows: one at full strength is the product
    /// mirrored on an invisible floor.
    pub reflections: f32,
    /// How blurred the reflection is, 0 (a mirror) to 1: the floor's
    /// roughness.
    pub reflection_blur: f32,
    /// How far the reflection reaches before it fades out, 0 to 1 of the
    /// product's height: 1 is all of it, 0.5 is gone by the top of the
    /// product, lower is a short reflection that only meets the floor. The
    /// studio photographer's faded reflection, under the product.
    pub reflection_length: f32,
    /// How far up the reflection fades out, in metres above the floor; 0
    /// leaves it to [`reflection_length`](Self::reflection_length). V-Ray's
    /// "Dim distance": a fade of 7.5 mm stays 7.5 mm whatever the product's
    /// size, where the length is a share of its height and so fades
    /// differently under every product.
    pub reflection_fade_distance: f32,
    /// Weaken the reflection where the floor is seen from above, as a real
    /// glossy floor's is (Fresnel), rather than the same everywhere.
    pub reflection_fresnel: bool,
    /// What the reflection's alpha follows. Its share of the pixel by
    /// default; its brightness for pictures placed on any background, where
    /// the dark parts of a reflection otherwise come out as a grey band.
    pub reflection_alpha: ReflectionAlpha,
    /// A floor only the product sees: its reflections and the light it
    /// bounces take this glow, while the camera sees the floor exactly as
    /// before. A photo studio's white sweep, invisible in the picture; it
    /// lifts metal and anodised faces that mirror the floor and would
    /// otherwise mirror black.
    pub bounce: bool,
    /// The bounce's brightness, in nits.
    pub bounce_strength: f32,
    /// The bounce's colour.
    pub bounce_color: Spectrum,
    /// How far from the product the bounce darkens toward the contact line,
    /// in metres, like ambient occlusion over that distance; 0 for not at
    /// all. It keeps the bottom of each face a little darker, as a real
    /// floor's light is.
    pub bounce_contact_falloff: f32,
    /// Project the below-horizon part of the environment flat onto the ground
    /// (`G`), so the floor does not visibly curve away.
    pub flatten: bool,
    /// Extent of the catcher in scene units. Zero means "fit the scene",
    /// which is what a user wants almost every time and what avoids a floor
    /// that is either smaller than the product or a mirror stretching to the
    /// horizon.
    pub size: f32,
}

impl Default for GroundSettings {
    fn default() -> Self {
        GroundSettings {
            enabled: true,
            shadows: true,
            shadow_color: Spectrum::ZERO,
            occlusion_shadows: false,
            reflections: 0.0,
            reflection_blur: 0.0,
            reflection_length: 1.0,
            reflection_fade_distance: 0.0,
            reflection_fresnel: false,
            reflection_alpha: ReflectionAlpha::Coverage,
            bounce: false,
            bounce_strength: 1.0,
            bounce_color: Spectrum::splat(1.0),
            bounce_contact_falloff: 0.0,
            flatten: false,
            size: 0.0,
        }
    }
}

/// What the ground reflection's alpha follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ReflectionAlpha {
    /// Its share of the pixel, whatever its brightness: what it always was.
    #[default]
    Coverage,
    /// Its brightness as displayed: a dark reflection is transparent, a
    /// bright one keeps its colour, and over black it is the same picture.
    Luminance,
}

impl ReflectionAlpha {
    pub const ALL: [ReflectionAlpha; 2] = [ReflectionAlpha::Coverage, ReflectionAlpha::Luminance];

    /// The name in the file format and the scripting API.
    pub const fn key(self) -> &'static str {
        match self {
            ReflectionAlpha::Coverage => "coverage",
            ReflectionAlpha::Luminance => "luminance",
        }
    }

    pub fn from_key(key: &str) -> Option<ReflectionAlpha> {
        ReflectionAlpha::ALL.into_iter().find(|a| a.key() == key)
    }
}

/// The document.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneDocument {
    pub meta: DocumentMeta,
    pub tree: SceneTree,
    pub geometry: GeometryStore,
    pub materials: MaterialStore,
    /// Where the scene's textures come from. The pixels live in the renderer's
    /// bounded cache, not here (`docs/02` §5.1).
    pub textures: TextureStore,
    pub units: SceneUnits,
    pub environment: EnvironmentSettings,
    pub lighting: LightingSettings,
    /// Saved cameras. The active one is the viewport's (§4, §8).
    pub cameras: CameraStore,
    /// The tone mapping and grade applied to the accumulated image. Not a
    /// render setting: changing it re-develops the existing samples rather
    /// than starting over (§01 §9.3).
    pub image_style: ImageStyle,
    /// An OpenColorIO config to take the display transform from, instead of
    /// the built-in one named by `image_style.display` (§01 §7).
    ///
    /// A path and two names rather than a loaded config, because that is what
    /// belongs in a document: the config is a studio's file, shared across
    /// projects and updated independently, and a project that embedded a copy
    /// would keep showing last year's version of it.
    pub ocio: Option<OcioSelection>,
    /// What a render is told before it starts (§1.8).
    ///
    /// In the document because a render setup *is* part of a project — the
    /// resolution a client asked for, the passes a compositor needs, the folder
    /// everything lands in — and rebuilding it every session is the sort of
    /// friction that makes people stop using a feature.
    pub render: RenderSettings,
    /// Dimensions annotated on the model (`TEQUILA_SPEC.md` §1.11). Saved,
    /// because a dimension somebody placed is part of what they made.
    pub measurements: Vec<crate::measure::Measurement>,
    /// Material variants on parts (`docs/02` §8.1).
    pub multi_materials: Vec<crate::studio::MultiMaterial>,
    /// Named selections of the tree, one shown at a time (§1.12).
    pub model_sets: crate::studio::ModelSets,
    /// Saved shots (§1.12).
    pub studios: Vec<crate::studio::Studio>,
    /// How Studios have turned the product (`Studio::product_yaw`).
    pub product_turn: crate::studio::ProductTurn,
    /// Rule-based bulk assignment (`docs/04` §3.3).
    pub material_templates: Vec<crate::templates::MaterialTemplate>,
    /// The product configurator built over the Studios, Model Sets and
    /// Multi-Materials (§1.13). Empty when there is none.
    pub configurator: crate::configurator::Configurator,
    /// CAD files' exact geometry, kept with the project (`docs/03` §6.2's
    /// B-rep retention).
    pub sources: Sources,
}

/// The metadata field on a CAD model's top nodes naming its source in
/// [`Sources`].
pub const BREP: &str = "B-rep";

/// CAD files kept with the project, by content: what a part's triangles
/// were made from, so they can be made again at another quality - or the
/// edges drawn again - when the file itself is long gone or on another
/// computer.
///
/// A cache rather than an edit: filled when a model is imported, written
/// when the project is saved (only what some model still refers to), and
/// outside undo, since taking a source away never changes the scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sources(std::collections::BTreeMap<String, std::sync::Arc<Vec<u8>>>);

impl Sources {
    /// A source's id: its length and a hash of its content, so the same
    /// file imported twice is kept once.
    pub fn id_of(bytes: &[u8]) -> String {
        // FNV-1a: stable across builds and platforms, unlike std's hasher.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}-{:x}", bytes.len())
    }

    pub fn insert(&mut self, id: String, bytes: std::sync::Arc<Vec<u8>>) {
        self.0.insert(id, bytes);
    }

    pub fn get(&self, id: &str) -> Option<&std::sync::Arc<Vec<u8>>> {
        self.0.get(id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &std::sync::Arc<Vec<u8>>)> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Which display and view of which config to develop through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OcioSelection {
    pub config: String,
    pub display: String,
    pub view: String,
}

impl Default for SceneDocument {
    fn default() -> Self {
        SceneDocument::new()
    }
}

/// Each node's world transform and whether it is shown, from
/// [`SceneDocument::placements`].
#[derive(Clone, Debug)]
pub struct Placements {
    /// By node id.
    placed: Vec<Option<(tequila_core::math::Transform, bool)>>,
}

impl Placements {
    /// The node's world transform; identity for a node that is not in the
    /// tree.
    pub fn world(&self, id: NodeId) -> tequila_core::math::Transform {
        self.placed.get(id.0 as usize).copied().flatten().map_or(tequila_core::math::Transform::IDENTITY, |p| p.0)
    }

    /// Whether the node is in the picture; a node not in the tree is not.
    pub fn shown(&self, id: NodeId) -> bool {
        self.placed.get(id.0 as usize).copied().flatten().is_some_and(|p| p.1)
    }
}

impl SceneDocument {
    pub fn new() -> Self {
        SceneDocument {
            meta: DocumentMeta::default(),
            tree: SceneTree::new(),
            geometry: GeometryStore::new(),
            materials: MaterialStore::new(),
            textures: TextureStore::new(),
            units: SceneUnits::default(),
            environment: EnvironmentSettings::default(),
            lighting: LightingSettings::default(),
            cameras: CameraStore::new(),
            image_style: ImageStyle::default(),
            ocio: None,
            render: RenderSettings::default(),
            measurements: Vec::new(),
            multi_materials: Vec::new(),
            model_sets: crate::studio::ModelSets::default(),
            studios: Vec::new(),
            product_turn: crate::studio::ProductTurn::default(),
            material_templates: Vec::new(),
            configurator: Default::default(),
            sources: Sources::default(),
        }
    }

    /// Whether a node is in the picture: visible itself and through its
    /// ancestors, and allowed by the active model set.
    pub fn is_shown(&self, id: NodeId) -> bool {
        self.tree.is_effectively_visible(id) && self.model_sets.allows(&self.tree, id)
    }

    /// Every node's world transform and whether it is shown, in one walk
    /// down the tree. What a snapshot asks of every node: asked node by
    /// node, each walked its ancestors three times over, on every frame of
    /// a gizmo drag - a third of a frame on a 5,000-part assembly.
    pub fn placements(&self) -> Placements {
        // Sets matter only while one is active.
        let active = self.model_sets.active;
        let mut allowed = std::collections::HashSet::new();
        let mut in_a_set = std::collections::HashSet::new();
        if active.is_some() {
            for (index, set) in self.model_sets.sets.iter().enumerate() {
                in_a_set.extend(set.nodes.iter().copied());
                if Some(index) == active || set.always_visible {
                    allowed.extend(set.nodes.iter().copied());
                }
            }
        }
        // Ids are handed out in order and never reused, so they index an
        // array as well as they key a map - and far faster.
        let mut placed = vec![None; self.tree.next_id() as usize];
        // (node, its parent's world, the parent visible, the chain so far in
        // an allowed set, the chain so far in any set)
        let mut stack: Vec<(NodeId, tequila_core::math::Transform, bool, bool, bool)> = self
            .tree
            .roots()
            .iter()
            .rev()
            .map(|&root| (root, tequila_core::math::Transform::IDENTITY, true, false, false))
            .collect();
        while let Some((id, parent, visible, in_allowed, in_any)) = stack.pop() {
            let Some(node) = self.tree.get(id) else { continue };
            let Some(slot) = placed.get_mut(id.0 as usize) else { continue };
            if slot.is_some() {
                continue;
            }
            let world = parent * node.transform;
            let visible = visible && node.visible;
            let (in_allowed, in_any) = match active {
                Some(_) => (in_allowed || allowed.contains(&id), in_any || in_a_set.contains(&id)),
                None => (false, false),
            };
            let shown = visible && (active.is_none() || in_allowed || !in_any);
            *slot = Some((world, shown));
            for &child in node.children.iter().rev() {
                stack.push((child, world, visible, in_allowed, in_any));
            }
        }
        Placements { placed }
    }

    /// World-space bounds of everything visible.
    pub fn bounds(&self) -> Aabb {
        let mut bounds = Aabb::EMPTY;
        for &root in self.tree.roots() {
            bounds.extend(
                &self
                    .tree
                    .bounds(root, &|mesh| self.geometry.get(mesh).map(|m| m.bounds())),
            );
        }
        bounds
    }

    pub fn triangle_count(&self) -> usize {
        self.tree
            .depth_first()
            .into_iter()
            .filter_map(|id| self.tree.get(id).and_then(|n| n.mesh()))
            .filter_map(|mesh| self.geometry.get(mesh))
            .map(|m| m.triangle_count())
            .sum()
    }

    /// Meshes actually referenced by a node, as distinct from meshes stored.
    /// The gap between the two is what [`Self::collect_garbage`] reclaims.
    pub fn referenced_meshes(&self) -> std::collections::BTreeSet<MeshKey> {
        self.tree.iter().filter_map(|node| node.mesh()).collect()
    }

    pub fn referenced_materials(&self) -> std::collections::BTreeSet<MaterialKey> {
        self.tree.iter().filter_map(|node| node.material).collect()
    }

    /// Every texture any material's graph refers to.
    pub fn referenced_textures(&self) -> std::collections::BTreeSet<tequila_materials::graph::TextureId> {
        use tequila_materials::graph::NodeKind;
        let mut used = std::collections::BTreeSet::new();
        for (_, definition) in self.materials.iter() {
            used.extend(definition.graph.decals.iter().map(|decal| decal.texture));
            for (_, node) in definition.graph.nodes() {
                if let NodeKind::ImageTexture { texture, .. } = &node.kind {
                    used.insert(*texture);
                }
            }
        }
        used
    }

    /// Drop unreferenced meshes and materials.
    ///
    /// Deliberately **not** automatic after every deletion: undoing a delete
    /// must restore the parts intact, and collecting their geometry eagerly
    /// would make that impossible. Collection is an explicit act, run before
    /// saving or on request.
    pub fn collect_garbage(&mut self) -> (usize, usize) {
        let meshes = self.referenced_meshes();
        let materials = self.referenced_materials();
        let dropped_meshes = self.geometry.retain_keys(&|key| meshes.contains(&key));
        let dropped_materials = self.materials.retain_keys(&|key| materials.contains(&key));
        // Textures are collected *after* materials, because dropping a material
        // is what makes its textures unreferenced.
        let textures = self.referenced_textures();
        self.textures.retain_ids(&|id| textures.contains(&id));
        (dropped_meshes, dropped_materials)
    }

    /// Whether two documents describe the same scene.
    ///
    /// Deliberately *not* `==`. A document also carries its ID allocators —
    /// the next node, mesh and material key to hand out — and those are
    /// monotonic by design: §2.3 forbids reusing an ID, because a recycled one
    /// lets a stale material assignment or animation track silently re-attach
    /// to an unrelated part. So undoing a command that created a node does not
    /// rewind the counter, and must not: redoing and then editing would then
    /// collide with the very IDs it had handed back.
    ///
    /// Two documents differing only in how many IDs have been spent describe
    /// the same scene, and that is what "identical to the start" means for
    /// undo. Strict `==` remains the right comparison for a save/reopen round
    /// trip, where the counters are written to the file and must survive it
    /// exactly.
    pub fn describes_the_same_scene_as(&self, other: &SceneDocument) -> bool {
        self.difference_from(other).is_none()
    }

    /// The first way in which this document's scene differs from another's, as
    /// a short description, or `None` if they agree.
    ///
    /// Exists because `assert_eq!` on two documents prints every node of both
    /// and says nothing about which field diverged.
    pub fn difference_from(&self, other: &SceneDocument) -> Option<String> {
        if self.meta != other.meta {
            return Some(format!("metadata: {:?} vs {:?}", self.meta, other.meta));
        }
        if self.units != other.units {
            return Some(format!("units: {:?} vs {:?}", self.units, other.units));
        }
        if self.environment != other.environment {
            return Some(format!(
                "environment: {:?} vs {:?}",
                self.environment, other.environment
            ));
        }
        if self.lighting != other.lighting {
            return Some(format!(
                "lighting: {:?} vs {:?}",
                self.lighting, other.lighting
            ));
        }
        if self.multi_materials != other.multi_materials {
            return Some(format!(
                "multi-materials: {:?} vs {:?}",
                self.multi_materials, other.multi_materials
            ));
        }
        if self.model_sets != other.model_sets {
            return Some(format!("model sets: {:?} vs {:?}", self.model_sets, other.model_sets));
        }
        if self.material_templates != other.material_templates {
            return Some(format!(
                "material templates: {:?} vs {:?}",
                self.material_templates, other.material_templates
            ));
        }
        if self.studios != other.studios {
            return Some(format!("studios: {:?} vs {:?}", self.studios, other.studios));
        }
        if self.configurator != other.configurator {
            return Some(format!("configurator: {:?} vs {:?}", self.configurator, other.configurator));
        }
        if self.measurements != other.measurements {
            return Some(format!(
                "measurements: {:?} vs {:?}",
                self.measurements, other.measurements
            ));
        }

        // Depth-first order, so a reparent or a reorder is a difference even
        // when the set of nodes is unchanged.
        let (mine, theirs) = (self.tree.depth_first(), other.tree.depth_first());
        if mine != theirs {
            return Some(format!(
                "hierarchy: {:?} vs {:?}",
                &mine[..mine.len().min(12)],
                &theirs[..theirs.len().min(12)]
            ));
        }
        for id in mine {
            let (a, b) = match (self.tree.get(id), other.tree.get(id)) {
                (Some(a), Some(b)) => (a, b),
                _ => return Some(format!("{id} exists in only one document")),
            };
            if a == b {
                continue;
            }
            return Some(if a.name != b.name {
                format!("{id} name: {:?} vs {:?}", a.name, b.name)
            } else if a.kind != b.kind {
                format!("{id} kind: {:?} vs {:?}", a.kind, b.kind)
            } else if a.material != b.material {
                format!("{id} material: {:?} vs {:?}", a.material, b.material)
            } else if a.transform != b.transform {
                format!("{id} ({:?}) has a different transform", a.name)
            } else if a.visible != b.visible {
                format!("{id} visible: {} vs {}", a.visible, b.visible)
            } else if a.locked != b.locked {
                format!("{id} locked: {} vs {}", a.locked, b.locked)
            } else if a.metadata != b.metadata {
                format!("{id} metadata: {:?} vs {:?}", a.metadata, b.metadata)
            } else if a.children != b.children {
                format!("{id} children: {:?} vs {:?}", a.children, b.children)
            } else if a.parent != b.parent {
                format!("{id} parent: {:?} vs {:?}", a.parent, b.parent)
            } else {
                format!("{id} differs")
            });
        }

        let meshes: Vec<_> = self.geometry.keys().collect();
        let other_meshes: Vec<_> = other.geometry.keys().collect();
        if meshes != other_meshes {
            return Some(format!(
                "meshes: {:?} vs {:?}",
                meshes.iter().map(|k| k.0).collect::<Vec<_>>(),
                other_meshes.iter().map(|k| k.0).collect::<Vec<_>>()
            ));
        }
        for key in meshes {
            if self.geometry.get(key) != other.geometry.get(key) {
                return Some(format!("mesh {} differs", key.0));
            }
        }

        let materials: Vec<_> = self.materials.keys().collect();
        let other_materials: Vec<_> = other.materials.keys().collect();
        if materials != other_materials {
            return Some(format!(
                "materials: {:?} vs {:?}",
                materials.iter().map(|k| k.0).collect::<Vec<_>>(),
                other_materials.iter().map(|k| k.0).collect::<Vec<_>>()
            ));
        }
        for key in materials {
            if self.materials.get(key) != other.materials.get(key) {
                return Some(format!("material {} differs", key.0));
            }
        }

        let textures: Vec<_> = self.textures.ids().collect();
        let other_textures: Vec<_> = other.textures.ids().collect();
        if textures != other_textures {
            return Some(format!(
                "textures: {:?} vs {:?}",
                textures.iter().map(|k| k.0).collect::<Vec<_>>(),
                other_textures.iter().map(|k| k.0).collect::<Vec<_>>()
            ));
        }
        for id in textures {
            if self.textures.get(id) != other.textures.get(id) {
                return Some(format!("texture {} differs", id.0));
            }
        }
        None
    }

    /// Check every cross-reference. A loaded document is validated before it is
    /// accepted: §4.2 asks for "reject-with-explanation" over silent
    /// misparsing.
    pub fn validate(&self) -> Result<(), String> {
        self.tree.validate()?;
        for node in self.tree.iter() {
            if let NodeKind::Part { mesh } = node.kind {
                if self.geometry.get(mesh).is_none() {
                    return Err(format!("{} references missing mesh {}", node.id, mesh.0));
                }
            }
            if let Some(material) = node.material {
                if self.materials.get(material).is_none() {
                    return Err(format!(
                        "{} references missing material {}",
                        node.id, material.0
                    ));
                }
            }
        }
        Ok(())
    }

    // --- mutations, reachable only through the command bus -----------------

    pub(crate) fn add_node(
        &mut self,
        parent: Option<NodeId>,
        name: impl Into<String>,
        kind: NodeKind,
    ) -> Result<NodeId, TreeError> {
        self.tree.insert(parent, name, kind)
    }

    /// Add a node under an ID chosen by the caller, for redo.
    pub(crate) fn add_node_with_id(
        &mut self,
        id: NodeId,
        parent: Option<NodeId>,
        name: impl Into<String>,
        kind: NodeKind,
    ) -> Result<NodeId, TreeError> {
        self.tree.insert_with_id(id, parent, name, kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Mesh;
    use crate::material::MaterialDefinition;
    use tequila_core::math::{vec3, Vec3};

    fn triangle() -> Mesh {
        Mesh::new(
            vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        )
    }

    /// The one walk down the tree answers as asking node by node does:
    /// transforms through moved groups, a node moved under one made after
    /// it, hidden groups, and model sets - active, always visible, and none.
    #[test]
    fn placements_agree_with_asking_node_by_node() {
        use crate::command::Command;
        use tequila_core::math::Transform;
        let mut document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.3);
        let mut bus = crate::CommandBus::new();
        let demo = document.tree.roots()[0];
        let sphere = document.tree.get(demo).unwrap().children[0];
        bus.apply(&mut document, Command::AddGroup { parent: None, name: "Outer".into() }).unwrap();
        let outer = *document.tree.roots().last().unwrap();
        bus.apply(&mut document, Command::AddGroup { parent: Some(outer), name: "Inner".into() }).unwrap();
        let inner = *document.tree.get(outer).unwrap().children.last().unwrap();
        // The demo, made first, moved under groups made after it.
        bus.apply(&mut document, Command::Reparent { node: demo, parent: Some(inner), index: None }).unwrap();
        bus.apply(&mut document, Command::AddGroup { parent: None, name: "Beside".into() }).unwrap();
        let beside = *document.tree.roots().last().unwrap();
        bus.apply(
            &mut document,
            Command::SetTransform {
                transforms: vec![
                    (outer, Transform::translation(vec3(1.0, 2.0, 3.0))),
                    (inner, Transform::translation(vec3(-0.5, 0.0, 0.25)) * Transform::rotation(vec3(0.0, 1.0, 0.0), 0.7)),
                ],
            },
        )
        .unwrap();
        let agree = |document: &SceneDocument, when: &str| {
            let placed = document.placements();
            for node in document.tree.iter() {
                assert_eq!(placed.world(node.id), document.tree.world_transform(node.id), "{when}: world of {}", node.name);
                assert_eq!(placed.shown(node.id), document.is_shown(node.id), "{when}: shown {}", node.name);
            }
        };
        agree(&document, "moved groups");
        bus.apply(&mut document, Command::SetVisibility { states: vec![(inner, false)] }).unwrap();
        agree(&document, "a hidden group");
        assert!(!document.placements().shown(sphere));
        bus.apply(&mut document, Command::SetVisibility { states: vec![(inner, true)] }).unwrap();
        let sets = |active, always| crate::studio::ModelSets {
            sets: vec![
                crate::studio::ModelSet { name: "Demo".into(), nodes: vec![demo], always_visible: false },
                crate::studio::ModelSet { name: "Beside".into(), nodes: vec![beside], always_visible: always },
            ],
            active,
        };
        for (active, always) in [(Some(0), false), (Some(1), false), (Some(1), true), (None, false)] {
            bus.apply(&mut document, Command::SetModelSets { model_sets: Box::new(sets(active, always)) }).unwrap();
            agree(&document, &format!("model set {active:?}, always {always}"));
        }
        assert!(!document.placements().shown(crate::tree::NodeId(9999)), "a node not in the tree");
    }

    fn document_with_a_part() -> (SceneDocument, NodeId, MeshKey, MaterialKey) {
        let mut doc = SceneDocument::new();
        let (mesh, _) = doc.geometry.insert_deduplicated(triangle());
        let material = doc.materials.insert(MaterialDefinition::plastic("Red"));
        let node = doc.add_node(None, "Part", NodeKind::Part { mesh }).unwrap();
        doc.tree.get_mut(node).unwrap().material = Some(material);
        (doc, node, mesh, material)
    }

    #[test]
    fn a_fresh_document_is_valid_and_empty() {
        let doc = SceneDocument::new();
        doc.validate().unwrap();
        assert!(doc.tree.is_empty());
        assert_eq!(doc.triangle_count(), 0);
        assert!(doc.bounds().is_empty());
        assert_eq!(doc.meta.format_version, FORMAT_VERSION);
    }

    #[test]
    fn a_document_with_a_part_validates() {
        let (doc, _, _, _) = document_with_a_part();
        doc.validate().unwrap();
        assert_eq!(doc.triangle_count(), 1);
        assert!(!doc.bounds().is_empty());
    }

    /// Validation must catch a reference to geometry that is not there, which
    /// is what a truncated or hand-edited file looks like.
    #[test]
    fn dangling_references_are_reported() {
        let (mut doc, node, _, _) = document_with_a_part();
        doc.tree.get_mut(node).unwrap().kind = NodeKind::Part { mesh: MeshKey(999) };
        let error = doc.validate().unwrap_err();
        assert!(error.contains("missing mesh"), "{error}");

        let (mut doc, node, mesh, _) = document_with_a_part();
        doc.tree.get_mut(node).unwrap().kind = NodeKind::Part { mesh };
        doc.tree.get_mut(node).unwrap().material = Some(MaterialKey(999));
        assert!(doc.validate().unwrap_err().contains("missing material"));
    }

    #[test]
    fn triangle_counts_include_every_instance() {
        let mut doc = SceneDocument::new();
        let (mesh, _) = doc.geometry.insert_deduplicated(triangle());
        for i in 0..10 {
            doc.add_node(None, format!("copy{i}"), NodeKind::Part { mesh })
                .unwrap();
        }
        // One mesh, ten instances, ten triangles rendered.
        assert_eq!(doc.geometry.len(), 1);
        assert_eq!(doc.triangle_count(), 10);
    }

    /// Garbage collection must not run eagerly: undoing a deletion has to
    /// bring the geometry back with the nodes.
    #[test]
    fn garbage_collection_is_explicit_and_reclaims_only_the_unreferenced() {
        let (mut doc, node, mesh, material) = document_with_a_part();
        doc.tree.remove_subtree(node).unwrap();

        // Still present, so an undo can restore the part intact.
        assert!(doc.geometry.get(mesh).is_some());
        assert!(doc.materials.get(material).is_some());

        let (meshes, materials) = doc.collect_garbage();
        assert_eq!((meshes, materials), (1, 1));
        assert!(doc.geometry.get(mesh).is_none());
        doc.validate().unwrap();
    }

    #[test]
    fn collection_keeps_anything_still_in_use() {
        let (mut doc, _, mesh, material) = document_with_a_part();
        let (kept, _) = doc.collect_garbage();
        assert_eq!(kept, 0);
        assert!(doc.geometry.get(mesh).is_some());
        assert!(doc.materials.get(material).is_some());
    }
}
