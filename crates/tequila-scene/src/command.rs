//! The command bus, undo/redo, and the journal.
//!
//! `TEQUILA_SPEC.md` §2.2 rule 1: "Nothing mutates the Scene Document except
//! the Command Bus. The UI and the Python API are peers, both issuing
//! commands. This is what makes undo, scripting, and automated testing work
//! without duplicated logic."
//!
//! # Why an enum rather than the trait of `docs/04` §8.1
//!
//! §8.1 sketches `trait Command` with `apply`/`revert`/`merge_with`. This
//! implements the same contract as an **enum**, for three reasons the trait
//! shape makes awkward:
//!
//! - **The journal has to serialize commands.** §8.4 requires journalling to
//!   disk for crash recovery, and a trait object cannot be deserialized back
//!   into itself without a hand-written registry — which is the enum, spelled
//!   less directly.
//! - **CI has to enumerate the catalog.** `docs/05` §3.8 asserts every command
//!   is reachable from both Python and the UI. [`Command::CATALOG`] is that
//!   list, checked by the compiler to be exhaustive rather than maintained by
//!   hand.
//! - **The Python binding needs one dispatch point**, not a registry of
//!   constructors.
//!
//! Undo data is kept beside the command in the history rather than inside it,
//! so a `Command` stays a plain description of intent — which is exactly what
//! is worth writing to a journal or receiving from a script.

use crate::document::{BackgroundMode, EnvironmentSettings, OcioSelection, SceneDocument};
use crate::geometry::Mesh;
use crate::lighting::{LightSettings, SavedCamera};
use crate::material::MaterialDefinition;
use crate::render::RenderSettings;
use crate::textures::{TextureSource, TextureStore};
use crate::tree::{MaterialKey, MeshKey, NodeId, NodeKind, SceneNode, TreeError};
use crate::units::{ConversionMode, ConversionPlan, SceneUnits};
use tequila_color::ImageStyle;
use tequila_core::math::{Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_materials::material_type::{MaterialType, Param, ParamValue};
use tequila_render_core::scheduler::{Dirty, SceneChange};
use std::collections::BTreeMap;

/// Why a command was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum CommandError {
    Tree(TreeError),
    NoSuchMaterial(MaterialKey),
    NoSuchMesh(MeshKey),
    /// The parameter does not apply to a material — either the value was the
    /// wrong shape for it, or it is a texture slot rather than a value.
    NoSuchParameter(Param),
    /// The node, or one of its ancestors, is locked. Locking exists to refuse
    /// edits, so this is a normal outcome rather than a fault.
    Locked(NodeId),
    /// A light command addressed a node that is not a light.
    NotALight(NodeId),
    /// A cutaway command addressed a node that is not a cutaway.
    NotACutaway(NodeId),
    /// The command's recorded undo data does not match the document. Only
    /// reachable if history and document have been driven out of step.
    InconsistentHistory(&'static str),
    /// A graph that would not compile.
    InvalidGraph(String),
    /// A value the renderer cannot use, such as a colour that is not a
    /// number.
    InvalidValue(&'static str),
}

impl From<TreeError> for CommandError {
    fn from(error: TreeError) -> Self {
        CommandError::Tree(error)
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandError::Tree(e) => write!(f, "{e}"),
            CommandError::NoSuchMaterial(k) => write!(f, "no such material: {}", k.0),
            CommandError::NoSuchMesh(k) => write!(f, "no such mesh: {}", k.0),
            CommandError::NoSuchParameter(p) => {
                write!(f, "`{}` cannot be set on a material", p.key())
            }
            CommandError::Locked(id) => write!(f, "{id} is locked"),
            CommandError::NotALight(id) => write!(f, "{id} is not a light"),
            CommandError::NotACutaway(id) => write!(f, "{id} is not a cutaway"),
            CommandError::InvalidValue(what) => write!(f, "{what}"),
            CommandError::InconsistentHistory(what) => {
                write!(f, "undo history is inconsistent with the document: {what}")
            }
            CommandError::InvalidGraph(why) => write!(f, "the material graph is not valid: {why}"),
        }
    }
}

impl std::error::Error for CommandError {}

/// Geometry and materials an import brings with it.
///
/// Carried by value in the command so that undoing an import removes exactly
/// what it added, and redoing it restores the same keys.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportPayload {
    pub meshes: Vec<(MeshKey, Mesh)>,
    /// The image files the materials refer to, with the ids their graphs were
    /// built against.
    ///
    /// Carried in the payload rather than interned by the importer, because
    /// the importer never touches the document — that is what makes an import
    /// undoable and scriptable. The ids come from `reserve_texture_ids`, like
    /// every other key here.
    pub textures: Vec<(tequila_materials::graph::TextureId, TextureSource)>,
    pub materials: Vec<(MaterialKey, MaterialDefinition)>,
    /// Nodes in parent-before-child order, with parents referring to nodes
    /// already listed or to an existing node.
    pub nodes: Vec<SceneNode>,
    pub roots: Vec<NodeId>,
}

/// Every mutation the document supports.
///
/// This enum **is** the command catalog of `docs/04` §8.2. Adding a variant
/// adds an entry to [`Command::CATALOG`], a Python binding, and a line the
/// API-parity test checks.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `scene.import` — a model read from a file, its nodes, meshes and
    /// materials added as one step.
    Import { payload: Box<ImportPayload> },
    /// `scene.reimport`: an updated version of an imported model, matched
    /// to it part by part (`crate::reimport`).
    Reimport { payload: Box<crate::reimport::ReimportPayload> },
    /// `scene.add_group` — an empty group, under `parent` or at the top.
    AddGroup {
        parent: Option<NodeId>,
        name: String,
    },
    /// `scene.delete_nodes` — nodes and everything under them, removed.
    DeleteNodes { nodes: Vec<NodeId> },
    /// `scene.rename` — a node's name.
    Rename { node: NodeId, name: String },
    /// `scene.set_transform`
    ///
    /// Takes a transform per node rather than one node, because moving a
    /// selection of six parts is one action and has to be one undo step. A
    /// shared transform would be meaningless — each part has its own place —
    /// so the pairs are explicit.
    SetTransform {
        transforms: Vec<(NodeId, Transform)>,
    },
    /// `scene.split_parts`
    ///
    /// Turn one part into a group of parts, one per connected shell of its
    /// mesh. For the models that arrive as a single lump because the file had
    /// no way to say otherwise — an STL, an OBJ with no groups — which is the
    /// only way to get at the pieces afterwards.
    SplitIntoParts { node: NodeId },
    /// `scene.set_visibility`
    ///
    /// A flag per node rather than one flag for all, so that "Isolate" — show
    /// these, hide everything else — is a single command and therefore a
    /// single undo step. Two commands would leave the user pressing Ctrl+Z
    /// twice for one menu item, with the first press appearing to do nothing.
    SetVisibility { states: Vec<(NodeId, bool)> },
    /// `scene.set_locked` — a lock per node: a locked node, and everything
    /// under it, refuses edits until unlocked.
    SetLocked { states: Vec<(NodeId, bool)> },
    /// `scene.set_pivot` — the point a node rotates and scales about.
    ///
    /// `None` clears it, returning the node to the gizmo's derived pivot
    /// modes. Points are in each node's own local space, so a pivot travels
    /// with the part.
    SetPivot { pivots: Vec<(NodeId, Option<Vec3>)> },
    /// `scene.reparent` — a node moved under another parent (or to the
    /// top), at `index` among its siblings or last.
    Reparent {
        node: NodeId,
        parent: Option<NodeId>,
        index: Option<usize>,
    },
    /// `scene.group` — collect nodes under a new group.
    Group { nodes: Vec<NodeId>, name: String },
    /// `scene.ungroup` — dissolve a group, promoting its children.
    Ungroup { group: NodeId },
    /// `scene.set_metadata` — one metadata field on a node, set, or removed
    /// with no value.
    SetMetadata {
        node: NodeId,
        key: String,
        value: Option<String>,
    },
    /// `scene.set_units` — the unit lengths are shown in. They are stored in
    /// metres either way; `mode` says whether the scene keeps its size
    /// (`KeepSceneSize`) or keeps its numbers and is rescaled to them
    /// (`ScaleScene`), with its lights and cameras.
    SetUnits {
        unit: LengthUnit,
        mode: ConversionMode,
    },
    /// `material.assign` — one material on these nodes, linked: editing it
    /// changes every part it is on.
    AssignMaterial {
        nodes: Vec<NodeId>,
        material: MaterialKey,
    },
    /// `material.unlink` — a node given its own copy of its material, so it
    /// can be edited alone.
    UnlinkMaterial { node: NodeId },
    /// `material.set_parameter` — one parameter of a material, by its key
    /// (`base_color`, `specular_roughness`, ...).
    SetMaterialParameter {
        material: MaterialKey,
        parameter: MaterialParameter,
    },
    /// `material.add_decal` — a label, an image projected onto a material.
    /// PNG, JPEG or Photoshop artwork; or a video (MP4, MOV and the rest the
    /// system plays), sent with `time` in seconds, whose frame then is the
    /// picture (kept under "Video frames" among Tequila's settings).
    AddDecal {
        material: MaterialKey,
        source: TextureSource,
        name: String,
    },
    /// `material.set_decals` — a material's whole stack of labels at once.
    SetDecals {
        material: MaterialKey,
        decals: Vec<tequila_materials::decal::Decal>,
    },
    /// `material.rename` — a material's name.
    RenameMaterial { material: MaterialKey, name: String },
    /// `material.set_graph` — the material's whole node graph, replaced.
    ///
    /// What the Material Graph editor issues for every edit (`docs/04` §7:
    /// "graph edits are commands like anything else"). The whole graph rather
    /// than one command per kind of edit: a graph edit — add a node, wire
    /// it, set three literals — is one thing to the person, and replacing the
    /// graph makes it one step to undo whatever it was, with nothing to
    /// renumber on the way back. Refused if the graph would not compile, so a
    /// half-wired graph cannot reach the renderer.
    SetGraph {
        material: MaterialKey,
        graph: Box<tequila_materials::graph::MaterialGraph>,
    },
    /// `material.set_type` — a material's type (plastic, metal, glass,
    /// velvet, ...), keeping what carries over.
    SetMaterialType {
        material: MaterialKey,
        material_type: MaterialType,
    },
    /// `material.set_texture`
    ///
    /// Drive a parameter from a texture, or stop driving it. This is §4.2's
    /// "enabling the texture toggle next to a parameter inserts a Texture Map
    /// node connected to that input" — one command, because from the user's
    /// side it is one action and has to undo as one.
    SetParameterTexture {
        material: MaterialKey,
        param: Param,
        /// `None` removes whatever drives the parameter, leaving its literal.
        source: Option<TextureSource>,
    },
    /// `material.set_procedural`
    ///
    /// Drive a parameter from a procedural pattern rather than from a file.
    /// `None` detaches, leaving the literal, exactly as a texture does.
    SetParameterProcedural {
        material: MaterialKey,
        param: Param,
        procedural: Option<Box<tequila_texture::procedural::Procedural>>,
        seed: u32,
    },
    /// `material.set_gradient`
    ///
    /// The colour ramp a parameter's procedural is read through.
    SetParameterGradient {
        material: MaterialKey,
        param: Param,
        gradient: Box<tequila_texture::gradient::Gradient>,
    },
    /// `material.set_mapping`
    ///
    /// How a parameter's texture is laid onto the surface: the projection, the
    /// scale, the rotation and the rest of §5.3's shared controls.
    ///
    /// Separate from [`Command::SetParameterTexture`] because the coordinates
    /// are a separate concern from the lookup — that is why they are two nodes
    /// in the graph — and because these are the controls a person adjusts
    /// repeatedly after choosing a file once.
    SetTextureMapping {
        material: MaterialKey,
        param: Param,
        mapping: Box<tequila_texture::projection::Mapping>,
    },
    /// `lighting.set_preset` — the lighting preset: performance, basic,
    /// product, interior or jewelry.
    SetLightingPreset {
        preset: tequila_render_core::settings::LightingPreset,
    },
    /// `lighting.set_custom` — the Custom panel's integrator configuration.
    ///
    /// Boxed because it is by far the largest thing a command carries and an
    /// unboxed copy would set the size of *every* variant, including the
    /// thousands a gizmo drag leaves in the history.
    SetLightingConfig {
        config: Box<tequila_render_core::settings::IntegratorConfig>,
    },
    /// `environment.set_parameter` — one parameter of the environment:
    /// brightness, rotation, background, ground and the like.
    SetEnvironmentParameter { parameter: EnvironmentParameter },
    /// `environment.set_sky` — a physical sky, or `None` for an image.
    SetSky {
        sky: Box<Option<tequila_render_core::sky::Sky>>,
    },
    /// `environment.set_pins` — the whole stack at once.
    ///
    /// The list rather than one pin, for the same reason `SetDecals` takes the
    /// list: adding, removing and reordering are edits to the stack itself,
    /// and a per-pin command would need an identity for a pin that nothing
    /// else in the document refers to.
    SetEnvironmentPins {
        pins: Vec<tequila_render_core::pins::Pin>,
    },
    /// `scene.add_light` — a light, under `parent` or at the top.
    AddLight {
        parent: Option<NodeId>,
        name: String,
        settings: Box<LightSettings>,
    },
    /// `scene.set_light` — every field of a light but its placement, which is
    /// the node's transform and goes through `scene.set_transform` like any
    /// other node's.
    SetLight {
        node: NodeId,
        settings: Box<LightSettings>,
    },
    /// `scene.add_cutaway` — a cutaway volume, placed. The transform comes
    /// with it, because a cutter is sized to what it is meant to cut and a
    /// separate move would be a second undo step for one action.
    AddCutaway {
        parent: Option<NodeId>,
        name: String,
        settings: Box<crate::cutaway::CutawaySettings>,
        transform: Transform,
    },
    /// `scene.set_cutaway` — every field of a cutaway but its placement.
    SetCutaway {
        node: NodeId,
        settings: Box<crate::cutaway::CutawaySettings>,
    },
    /// `scene.set_measurements` — the whole list, as pins and decals are
    /// edited: adding, removing and hiding are all edits to the list.
    SetMeasurements {
        measurements: Vec<crate::measure::Measurement>,
    },
    /// `material.set_multi_materials` — the whole list, and the active
    /// variant of each assigned to its parts.
    SetMultiMaterials {
        multi_materials: Vec<crate::studio::MultiMaterial>,
    },
    /// `scene.set_model_sets` — the sets, and which is shown.
    SetModelSets {
        model_sets: Box<crate::studio::ModelSets>,
    },
    /// `studio.set` — the list of Studios. Editing a Studio does not apply
    /// it.
    SetStudios { studios: Vec<crate::studio::Studio> },
    /// `studio.apply` — make the scene what a Studio says, in one step.
    ApplyStudio { index: usize },
    /// `configurator.set` — the configurator: its options, rules and
    /// layout. Setting it does not apply a configuration.
    SetConfigurator { configurator: Box<crate::configurator::Configurator> },
    /// `configurator.choose` — show a configuration: every option's choice
    /// applied, in one step. Refused when it breaks a rule.
    Configure { choices: Vec<usize> },
    /// `material.set_templates` — the list of Material Templates. Applying
    /// one is ordinary assignments, grouped.
    SetMaterialTemplates {
        templates: Vec<crate::templates::MaterialTemplate>,
    },
    /// `camera.add` — save the current view as a new camera.
    AddCamera { camera: Box<SavedCamera> },
    /// `camera.remove` — a saved camera, removed.
    RemoveCamera { index: usize },
    /// `camera.select` — which saved camera is the active one.
    SelectCamera { index: usize },
    /// `camera.set` — replace a saved camera, which is what every lens control
    /// and every orbit of the viewport amounts to.
    SetCamera {
        index: usize,
        camera: Box<SavedCamera>,
    },
    /// `camera.rename` — a saved camera's name.
    RenameCamera { index: usize, name: String },
    /// `image.set_color_management` — choose an OCIO config's display and
    /// view, or `None` to go back to the built-in transforms.
    SetColorManagement { selection: Option<OcioSelection> },
    /// `render.set_settings`
    ///
    /// One command for the whole block rather than one per control, for the
    /// same reason the image style is one: none of it reaches the renderer
    /// until Render is pressed, so there is nothing to be gained by telling the
    /// settings apart.
    SetRenderSettings { settings: Box<RenderSettings> },
    /// `image.set_style`
    ///
    /// One command for the whole style rather than one per control: §9.3
    /// classifies every one of them as `Display`, so there is nothing to be
    /// gained by telling them apart, and a style is small enough to carry
    /// whole.
    SetImageStyle { style: Box<ImageStyle> },
}

/// A single material parameter edit: which parameter, and its new value.
///
/// Split out so that dragging a slider produces one coalesced undo entry
/// rather than two hundred (§8.1's merge policy) — consecutive edits merge
/// when they address the same `param`.
///
/// This was a five-variant enum at M1, one variant per editable parameter.
/// With §2.2's full block there are thirty-seven, and enumerating them here
/// would mean enumerating them again in the journal, again in the file format
/// and again in the panel. [`Param`] carries that knowledge once instead, so a
/// new parameter becomes a row in one table rather than four matching arms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialParameter {
    pub param: Param,
    pub value: ParamValue,
}

impl MaterialParameter {
    pub fn new(param: Param, value: ParamValue) -> Self {
        MaterialParameter { param, value }
    }

    pub fn scalar(param: Param, value: f32) -> Self {
        MaterialParameter::new(param, ParamValue::Scalar(value))
    }

    pub fn color(param: Param, value: tequila_core::spectrum::Spectrum) -> Self {
        MaterialParameter::new(param, ParamValue::Color(value))
    }

    pub fn flag(param: Param, value: bool) -> Self {
        MaterialParameter::new(param, ParamValue::Flag(value))
    }
}

/// One editable field of the environment (§1.5).
///
/// A table rather than one command variant per control, for the same reason
/// [`Param`] is a table: fourteen controls would otherwise be enumerated in
/// the command, again in the undo record, again in the journal, again in the
/// file format and again in the panel, and the five lists would drift. Here a
/// new control is a row, and `every_environment_parameter_round_trips` checks
/// that the row is complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EnvParam {
    Rotation,
    Brightness,
    Contrast,
    Size,
    Height,
    Background,
    BackgroundColor,
    Backplate,
    /// The lighting environment's own image. `None` is the built-in studio.
    ///
    /// A parameter rather than a field the panel writes directly, so that
    /// loading an HDRI is undoable, journalled and serialized by the same
    /// machinery as every other environment control — and so the panel cannot
    /// change it without the renderer being told.
    Image,
    GroundEnabled,
    GroundShadows,
    GroundShadowColor,
    GroundOcclusion,
    GroundReflections,
    GroundReflectionBlur,
    GroundReflectionLength,
    GroundReflectionFadeDistance,
    GroundReflectionFresnel,
    GroundReflectionAlpha,
    GroundBounce,
    GroundBounceStrength,
    GroundBounceColor,
    GroundBounceContactFalloff,
    GroundFlatten,
    GroundSize,
}

/// A value one of those fields can hold.
#[derive(Clone, Debug, PartialEq)]
pub enum EnvValue {
    Scalar(f32),
    Flag(bool),
    Color(Spectrum),
    Mode(BackgroundMode),
    /// A file path, or `None` for "not set".
    Path(Option<String>),
    /// One of a fixed set of named options, [`EnvParam::choices`].
    Choice(&'static str),
}

impl EnvValue {
    pub fn as_scalar(&self) -> Option<f32> {
        match self {
            EnvValue::Scalar(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_flag(&self) -> Option<bool> {
        match self {
            EnvValue::Flag(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_color(&self) -> Option<Spectrum> {
        match self {
            EnvValue::Color(v) => Some(*v),
            _ => None,
        }
    }
}

/// How a control should be presented, and what values it accepts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvUi {
    pub label: &'static str,
    /// Inclusive range for a scalar, ignored by the other shapes. The upper
    /// bound is a *soft* one for sizes, which depend on the scene's scale.
    pub range: (f32, f32),
}

impl EnvParam {
    pub const ALL: [EnvParam; 25] = [
        EnvParam::Rotation,
        EnvParam::Brightness,
        EnvParam::Contrast,
        EnvParam::Size,
        EnvParam::Height,
        EnvParam::Background,
        EnvParam::BackgroundColor,
        EnvParam::Backplate,
        EnvParam::Image,
        EnvParam::GroundEnabled,
        EnvParam::GroundShadows,
        EnvParam::GroundShadowColor,
        EnvParam::GroundOcclusion,
        EnvParam::GroundReflections,
        EnvParam::GroundReflectionBlur,
        EnvParam::GroundReflectionLength,
        EnvParam::GroundReflectionFadeDistance,
        EnvParam::GroundReflectionFresnel,
        EnvParam::GroundReflectionAlpha,
        EnvParam::GroundBounce,
        EnvParam::GroundBounceStrength,
        EnvParam::GroundBounceColor,
        EnvParam::GroundBounceContactFalloff,
        EnvParam::GroundFlatten,
        EnvParam::GroundSize,
    ];

    /// The options a [`EnvValue::Choice`] field offers: its keys, as
    /// written, and their labels. Empty for every other field.
    pub const fn choices(self) -> &'static [(&'static str, &'static str)] {
        match self {
            EnvParam::GroundReflectionAlpha => &[("coverage", "By coverage"), ("luminance", "By brightness")],
            _ => &[],
        }
    }

    /// The name used in the file format, the journal and the scripting API.
    pub const fn key(self) -> &'static str {
        match self {
            EnvParam::Rotation => "rotation",
            EnvParam::Brightness => "brightness",
            EnvParam::Contrast => "contrast",
            EnvParam::Size => "size",
            EnvParam::Height => "height",
            EnvParam::Background => "background",
            EnvParam::BackgroundColor => "background_color",
            EnvParam::Backplate => "backplate",
            EnvParam::Image => "image",
            EnvParam::GroundEnabled => "ground_enabled",
            EnvParam::GroundShadows => "ground_shadows",
            EnvParam::GroundShadowColor => "ground_shadow_color",
            EnvParam::GroundOcclusion => "ground_occlusion",
            EnvParam::GroundReflections => "ground_reflections",
            EnvParam::GroundReflectionBlur => "ground_reflection_blur",
            EnvParam::GroundReflectionLength => "ground_reflection_length",
            EnvParam::GroundReflectionFadeDistance => "ground_reflection_fade_distance",
            EnvParam::GroundReflectionFresnel => "ground_reflection_fresnel",
            EnvParam::GroundReflectionAlpha => "ground_reflection_alpha",
            EnvParam::GroundBounce => "ground_bounce",
            EnvParam::GroundBounceStrength => "ground_bounce_strength",
            EnvParam::GroundBounceColor => "ground_bounce_color",
            EnvParam::GroundBounceContactFalloff => "ground_bounce_contact_falloff",
            EnvParam::GroundFlatten => "ground_flatten",
            EnvParam::GroundSize => "ground_size",
        }
    }

    pub fn from_key(key: &str) -> Option<EnvParam> {
        EnvParam::ALL.into_iter().find(|p| p.key() == key)
    }

    /// Where the field sits in a saved document: `(section, field)`, with an
    /// empty section meaning the top level of the environment object.
    ///
    /// The file format groups these the way `docs/03` §4.1 shows them, while
    /// [`Self::key`] is flat because the journal and the scripting API address
    /// one field at a time. Both come from this one table, so the two naming
    /// schemes cannot drift apart.
    pub const fn json_path(self) -> (&'static str, &'static str) {
        match self {
            EnvParam::Rotation => ("", "rotation"),
            EnvParam::Brightness => ("", "brightness"),
            EnvParam::Contrast => ("", "contrast"),
            EnvParam::Size => ("", "size"),
            EnvParam::Height => ("", "height"),
            EnvParam::Background => ("background", "mode"),
            EnvParam::BackgroundColor => ("background", "color"),
            EnvParam::Backplate => ("background", "image"),
            // At the top level rather than in a section: it is the thing the
            // rest of the environment is *about*, and it was already written
            // there before it became a parameter.
            EnvParam::Image => ("", "image"),
            EnvParam::GroundEnabled => ("ground", "enabled"),
            EnvParam::GroundShadows => ("ground", "shadows"),
            EnvParam::GroundShadowColor => ("ground", "shadow_color"),
            EnvParam::GroundOcclusion => ("ground", "occlusion_shadows"),
            EnvParam::GroundReflections => ("ground", "reflections"),
            EnvParam::GroundReflectionBlur => ("ground", "reflection_blur"),
            EnvParam::GroundReflectionLength => ("ground", "reflection_length"),
            EnvParam::GroundReflectionFadeDistance => ("ground", "reflection_fade_distance"),
            EnvParam::GroundReflectionFresnel => ("ground", "reflection_fresnel"),
            EnvParam::GroundReflectionAlpha => ("ground", "reflection_alpha"),
            EnvParam::GroundBounce => ("ground", "bounce"),
            EnvParam::GroundBounceStrength => ("ground", "bounce_strength"),
            EnvParam::GroundBounceColor => ("ground", "bounce_color"),
            EnvParam::GroundBounceContactFalloff => ("ground", "bounce_contact_falloff"),
            EnvParam::GroundFlatten => ("ground", "flatten"),
            EnvParam::GroundSize => ("ground", "size"),
        }
    }

    pub const fn ui(self) -> EnvUi {
        match self {
            EnvParam::Rotation => EnvUi {
                label: "Rotation",
                range: (0.0, std::f32::consts::TAU),
            },
            EnvParam::Brightness => EnvUi {
                label: "Brightness",
                range: (0.0, 10.0),
            },
            EnvParam::Contrast => EnvUi {
                label: "Contrast",
                range: (0.1, 4.0),
            },
            EnvParam::Size => EnvUi {
                label: "Size",
                range: (0.0, 100_000.0),
            },
            EnvParam::Height => EnvUi {
                label: "Height",
                range: (-10_000.0, 10_000.0),
            },
            EnvParam::Background => EnvUi {
                label: "Background",
                range: (0.0, 0.0),
            },
            EnvParam::BackgroundColor => EnvUi {
                label: "Background colour",
                range: (0.0, 1.0),
            },
            EnvParam::Backplate => EnvUi {
                label: "Backplate",
                range: (0.0, 0.0),
            },
            EnvParam::Image => EnvUi {
                label: "HDRI",
                range: (0.0, 0.0),
            },
            EnvParam::GroundEnabled => EnvUi {
                label: "Ground plane",
                range: (0.0, 0.0),
            },
            EnvParam::GroundShadows => EnvUi {
                label: "Catch shadows",
                range: (0.0, 0.0),
            },
            EnvParam::GroundShadowColor => EnvUi {
                label: "Shadow colour",
                range: (0.0, 1.0),
            },
            EnvParam::GroundOcclusion => EnvUi {
                label: "Occlusion shadows",
                range: (0.0, 0.0),
            },
            EnvParam::GroundReflections => EnvUi {
                label: "Ground reflections",
                range: (0.0, 1.0),
            },
            EnvParam::GroundReflectionBlur => EnvUi {
                label: "Reflection blur",
                range: (0.0, 1.0),
            },
            EnvParam::GroundReflectionLength => EnvUi {
                label: "Reflection length",
                range: (0.0, 1.0),
            },
            EnvParam::GroundReflectionFadeDistance => EnvUi {
                label: "Fade distance",
                range: (0.0, 1.0),
            },
            EnvParam::GroundReflectionFresnel => EnvUi {
                label: "Fainter seen from above",
                range: (0.0, 0.0),
            },
            EnvParam::GroundReflectionAlpha => EnvUi {
                label: "Reflection alpha",
                range: (0.0, 0.0),
            },
            EnvParam::GroundBounce => EnvUi {
                label: "Floor bounce",
                range: (0.0, 0.0),
            },
            EnvParam::GroundBounceStrength => EnvUi {
                label: "Bounce brightness",
                range: (0.0, 20.0),
            },
            EnvParam::GroundBounceColor => EnvUi {
                label: "Bounce colour",
                range: (0.0, 1.0),
            },
            EnvParam::GroundBounceContactFalloff => EnvUi {
                label: "Contact falloff",
                range: (0.0, 1.0),
            },
            EnvParam::GroundFlatten => EnvUi {
                label: "Flatten ground",
                range: (0.0, 0.0),
            },
            EnvParam::GroundSize => EnvUi {
                label: "Ground size",
                range: (0.0, 100_000.0),
            },
        }
    }

    /// The current value of this field.
    pub fn read(self, settings: &EnvironmentSettings) -> EnvValue {
        match self {
            EnvParam::Rotation => EnvValue::Scalar(settings.rotation),
            EnvParam::Brightness => EnvValue::Scalar(settings.brightness),
            EnvParam::Contrast => EnvValue::Scalar(settings.contrast),
            EnvParam::Size => EnvValue::Scalar(settings.size),
            EnvParam::Height => EnvValue::Scalar(settings.height),
            EnvParam::Background => EnvValue::Mode(settings.background),
            EnvParam::BackgroundColor => EnvValue::Color(settings.background_color),
            EnvParam::Backplate => EnvValue::Path(settings.backplate.clone()),
            EnvParam::Image => EnvValue::Path(settings.image.clone()),
            EnvParam::GroundEnabled => EnvValue::Flag(settings.ground.enabled),
            EnvParam::GroundShadows => EnvValue::Flag(settings.ground.shadows),
            EnvParam::GroundShadowColor => EnvValue::Color(settings.ground.shadow_color),
            EnvParam::GroundOcclusion => EnvValue::Flag(settings.ground.occlusion_shadows),
            EnvParam::GroundReflections => EnvValue::Scalar(settings.ground.reflections),
            EnvParam::GroundReflectionBlur => EnvValue::Scalar(settings.ground.reflection_blur),
            EnvParam::GroundReflectionLength => EnvValue::Scalar(settings.ground.reflection_length),
            EnvParam::GroundReflectionFadeDistance => EnvValue::Scalar(settings.ground.reflection_fade_distance),
            EnvParam::GroundReflectionFresnel => EnvValue::Flag(settings.ground.reflection_fresnel),
            EnvParam::GroundReflectionAlpha => EnvValue::Choice(settings.ground.reflection_alpha.key()),
            EnvParam::GroundBounce => EnvValue::Flag(settings.ground.bounce),
            EnvParam::GroundBounceStrength => EnvValue::Scalar(settings.ground.bounce_strength),
            EnvParam::GroundBounceColor => EnvValue::Color(settings.ground.bounce_color),
            EnvParam::GroundBounceContactFalloff => EnvValue::Scalar(settings.ground.bounce_contact_falloff),
            EnvParam::GroundFlatten => EnvValue::Flag(settings.ground.flatten),
            EnvParam::GroundSize => EnvValue::Scalar(settings.ground.size),
        }
    }

    /// Write a value, ignoring one of the wrong shape.
    ///
    /// Ignoring rather than refusing, because the only way to reach this with
    /// a mismatched value is a hand-edited file or a scripting typo, and the
    /// useful outcome there is that the rest of the document still loads.
    pub fn write(self, settings: &mut EnvironmentSettings, value: &EnvValue) {
        let (lo, hi) = self.ui().range;
        let scalar = |v: &EnvValue| v.as_scalar().map(|v| v.clamp(lo, hi));
        match (self, value) {
            (EnvParam::Rotation, v) => {
                if let Some(v) = scalar(v) {
                    settings.rotation = v;
                }
            }
            (EnvParam::Brightness, v) => {
                if let Some(v) = scalar(v) {
                    settings.brightness = v;
                }
            }
            (EnvParam::Contrast, v) => {
                if let Some(v) = scalar(v) {
                    settings.contrast = v;
                }
            }
            (EnvParam::Size, v) => {
                if let Some(v) = scalar(v) {
                    settings.size = v;
                }
            }
            (EnvParam::Height, v) => {
                if let Some(v) = scalar(v) {
                    settings.height = v;
                }
            }
            (EnvParam::Background, EnvValue::Mode(m)) => settings.background = *m,
            (EnvParam::BackgroundColor, EnvValue::Color(c)) => settings.background_color = *c,
            (EnvParam::Backplate, EnvValue::Path(p)) => settings.backplate = p.clone(),
            (EnvParam::Image, EnvValue::Path(p)) => settings.image = p.clone(),
            (EnvParam::GroundEnabled, EnvValue::Flag(f)) => settings.ground.enabled = *f,
            (EnvParam::GroundShadows, EnvValue::Flag(f)) => settings.ground.shadows = *f,
            (EnvParam::GroundShadowColor, EnvValue::Color(c)) => settings.ground.shadow_color = *c,
            (EnvParam::GroundOcclusion, EnvValue::Flag(f)) => settings.ground.occlusion_shadows = *f,
            (EnvParam::GroundReflections, v) => {
                if let Some(v) = scalar(v) {
                    settings.ground.reflections = v;
                }
            }
            (EnvParam::GroundReflectionBlur, v) => {
                if let Some(v) = scalar(v) {
                    settings.ground.reflection_blur = v;
                }
            }
            (EnvParam::GroundReflectionLength, v) => {
                if let Some(v) = scalar(v) {
                    settings.ground.reflection_length = v.clamp(0.0, 1.0);
                }
            }
            (EnvParam::GroundReflectionFadeDistance, v) => {
                if let Some(v) = v.as_scalar() {
                    // A length, so only the floor of the range binds: a big
                    // product's fade may be longer than the slider shows.
                    settings.ground.reflection_fade_distance = v.max(0.0);
                }
            }
            (EnvParam::GroundReflectionFresnel, EnvValue::Flag(f)) => settings.ground.reflection_fresnel = *f,
            (EnvParam::GroundReflectionAlpha, EnvValue::Choice(c)) => {
                if let Some(alpha) = crate::document::ReflectionAlpha::from_key(c) {
                    settings.ground.reflection_alpha = alpha;
                }
            }
            (EnvParam::GroundBounce, EnvValue::Flag(f)) => settings.ground.bounce = *f,
            (EnvParam::GroundBounceStrength, v) => {
                if let Some(v) = v.as_scalar() {
                    settings.ground.bounce_strength = v.max(0.0);
                }
            }
            (EnvParam::GroundBounceColor, EnvValue::Color(c)) => settings.ground.bounce_color = *c,
            (EnvParam::GroundBounceContactFalloff, v) => {
                if let Some(v) = v.as_scalar() {
                    settings.ground.bounce_contact_falloff = v.max(0.0);
                }
            }
            (EnvParam::GroundFlatten, EnvValue::Flag(f)) => settings.ground.flatten = *f,
            (EnvParam::GroundSize, v) => {
                if let Some(v) = scalar(v) {
                    settings.ground.size = v;
                }
            }
            _ => {}
        }
    }

    /// How much accumulated work this field invalidates.
    ///
    /// Rotation and brightness have their own entries in §9.3 because they are
    /// dragged; everything else here is rare enough that a full restart costs
    /// nothing anyone notices.
    pub const fn scene_change(self) -> SceneChange {
        match self {
            EnvParam::Rotation => SceneChange::EnvironmentRotation,
            EnvParam::Brightness => SceneChange::EnvironmentBrightness,
            // A different image is a different importance distribution, so the
            // CDF is rebuilt and everything accumulated so far is of a
            // different scene.
            EnvParam::Image => SceneChange::GeometryEdit,
            _ => SceneChange::GeometryEdit,
        }
    }
}

/// A single environment edit: which field, and its new value.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvironmentParameter {
    pub param: EnvParam,
    pub value: EnvValue,
}

impl EnvironmentParameter {
    pub fn new(param: EnvParam, value: EnvValue) -> EnvironmentParameter {
        EnvironmentParameter { param, value }
    }

    pub fn scalar(param: EnvParam, value: f32) -> EnvironmentParameter {
        EnvironmentParameter::new(param, EnvValue::Scalar(value))
    }

    pub fn flag(param: EnvParam, value: bool) -> EnvironmentParameter {
        EnvironmentParameter::new(param, EnvValue::Flag(value))
    }

    pub fn color(param: EnvParam, value: Spectrum) -> EnvironmentParameter {
        EnvironmentParameter::new(param, EnvValue::Color(value))
    }
}

impl Command {
    /// The dotted name, as it appears in the catalog and in the Python API.
    /// One node's transform. The list form is the general case; this is the
    /// one almost every call site wants and keeps them readable.
    pub fn set_transform(node: NodeId, transform: Transform) -> Command {
        Command::SetTransform {
            transforms: vec![(node, transform)],
        }
    }

    pub fn set_visibility(node: NodeId, visible: bool) -> Command {
        Command::SetVisibility {
            states: vec![(node, visible)],
        }
    }

    /// The same flag for many nodes, which is what "Hide others" and every
    /// per-row eye amount to.
    pub fn set_visibility_of(nodes: impl IntoIterator<Item = NodeId>, visible: bool) -> Command {
        Command::SetVisibility {
            states: nodes.into_iter().map(|node| (node, visible)).collect(),
        }
    }

    pub fn set_pivot(node: NodeId, pivot: Option<Vec3>) -> Command {
        Command::SetPivot {
            pivots: vec![(node, pivot)],
        }
    }

    pub fn set_locked(node: NodeId, locked: bool) -> Command {
        Command::SetLocked {
            states: vec![(node, locked)],
        }
    }

    pub fn set_locked_of(nodes: impl IntoIterator<Item = NodeId>, locked: bool) -> Command {
        Command::SetLocked {
            states: nodes.into_iter().map(|node| (node, locked)).collect(),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Command::Import { .. } => "scene.import",
            Command::Reimport { .. } => "scene.reimport",
            Command::AddGroup { .. } => "scene.add_group",
            Command::DeleteNodes { .. } => "scene.delete_nodes",
            Command::SplitIntoParts { .. } => "scene.split_parts",
            Command::Rename { .. } => "scene.rename",
            Command::SetTransform { .. } => "scene.set_transform",
            Command::SetVisibility { .. } => "scene.set_visibility",
            Command::SetLocked { .. } => "scene.set_locked",
            Command::SetPivot { .. } => "scene.set_pivot",
            Command::Reparent { .. } => "scene.reparent",
            Command::Group { .. } => "scene.group",
            Command::Ungroup { .. } => "scene.ungroup",
            Command::SetMetadata { .. } => "scene.set_metadata",
            Command::SetUnits { .. } => "scene.set_units",
            Command::AssignMaterial { .. } => "material.assign",
            Command::UnlinkMaterial { .. } => "material.unlink",
            Command::SetMaterialParameter { .. } => "material.set_parameter",
            Command::AddDecal { .. } => "material.add_decal",
            Command::SetDecals { .. } => "material.set_decals",
            Command::SetMaterialType { .. } => "material.set_type",
            Command::SetParameterTexture { .. } => "material.set_texture",
            Command::SetParameterProcedural { .. } => "material.set_procedural",
            Command::SetParameterGradient { .. } => "material.set_gradient",
            Command::SetTextureMapping { .. } => "material.set_mapping",
            Command::RenameMaterial { .. } => "material.rename",
            Command::SetGraph { .. } => "material.set_graph",
            Command::SetLightingPreset { .. } => "lighting.set_preset",
            Command::SetLightingConfig { .. } => "lighting.set_custom",
            Command::SetEnvironmentParameter { .. } => "environment.set_parameter",
            Command::SetEnvironmentPins { .. } => "environment.set_pins",
            Command::SetSky { .. } => "environment.set_sky",
            Command::AddLight { .. } => "scene.add_light",
            Command::SetLight { .. } => "scene.set_light",
            Command::AddCutaway { .. } => "scene.add_cutaway",
            Command::SetCutaway { .. } => "scene.set_cutaway",
            Command::SetMeasurements { .. } => "scene.set_measurements",
            Command::SetMultiMaterials { .. } => "material.set_multi_materials",
            Command::SetModelSets { .. } => "scene.set_model_sets",
            Command::SetStudios { .. } => "studio.set",
            Command::ApplyStudio { .. } => "studio.apply",
            Command::SetConfigurator { .. } => "configurator.set",
            Command::Configure { .. } => "configurator.choose",
            Command::SetMaterialTemplates { .. } => "material.set_templates",
            Command::AddCamera { .. } => "camera.add",
            Command::RemoveCamera { .. } => "camera.remove",
            Command::SelectCamera { .. } => "camera.select",
            Command::SetCamera { .. } => "camera.set",
            Command::RenameCamera { .. } => "camera.rename",
            Command::SetColorManagement { .. } => "image.set_color_management",
            Command::SetRenderSettings { .. } => "render.set_settings",
            Command::SetImageStyle { .. } => "image.set_style",
        }
    }

    /// Every command name. `docs/05` §3.8 asserts each is reachable from both
    /// the UI and the scripting API.
    pub const CATALOG: [&'static str; 52] = [
        "material.add_decal",
        "material.set_decals",
        "scene.import",
        "scene.reimport",
        "scene.add_group",
        "scene.delete_nodes",
        "scene.split_parts",
        "scene.rename",
        "scene.set_transform",
        "scene.set_visibility",
        "scene.set_locked",
        "scene.set_pivot",
        "scene.reparent",
        "scene.group",
        "scene.ungroup",
        "scene.set_metadata",
        "scene.set_units",
        "material.assign",
        "material.unlink",
        "material.set_parameter",
        "material.rename",
        "material.set_graph",
        "material.set_type",
        "material.set_texture",
        "material.set_procedural",
        "material.set_gradient",
        "material.set_mapping",
        "lighting.set_preset",
        "lighting.set_custom",
        "environment.set_parameter",
        "environment.set_pins",
        "environment.set_sky",
        "scene.add_light",
        "scene.set_light",
        "scene.add_cutaway",
        "scene.set_cutaway",
        "scene.set_measurements",
        "material.set_multi_materials",
        "scene.set_model_sets",
        "studio.set",
        "studio.apply",
        "configurator.set",
        "configurator.choose",
        "material.set_templates",
        "camera.add",
        "camera.remove",
        "camera.select",
        "camera.set",
        "camera.rename",
        "image.set_style",
        "image.set_color_management",
        "render.set_settings",
    ];

    /// How much accumulated render work this command invalidates (§01 §9.3).
    pub fn scene_change(&self) -> SceneChange {
        match self {
            Command::Import { .. }
            | Command::Reimport { .. }
            | Command::DeleteNodes { .. }
            | Command::Group { .. }
            | Command::SplitIntoParts { .. }
            | Command::Ungroup { .. }
            | Command::Reparent { .. }
            | Command::SetUnits { .. } => SceneChange::GeometryEdit,
            Command::SetTransform { .. } => SceneChange::InstanceTransform,
            Command::SetVisibility { .. } => SceneChange::VisibilityToggle,
            Command::AssignMaterial { .. } | Command::UnlinkMaterial { .. } => {
                SceneChange::MaterialAssignment
            }
            Command::SetMaterialParameter { .. } => SceneChange::MaterialParameter,
            Command::AddDecal { .. } | Command::SetDecals { .. } => SceneChange::MaterialAssignment,
            Command::SetMaterialType { .. } => SceneChange::MaterialParameter,
            // A texture changes what the material *is*, not just a number in
            // it: the graph recompiles and the texture has to be loaded.
            Command::SetParameterTexture { .. } => SceneChange::MaterialAssignment,
            Command::SetParameterProcedural { .. } => SceneChange::MaterialAssignment,
            Command::SetParameterGradient { .. } => SceneChange::MaterialAssignment,
            Command::SetTextureMapping { .. } => SceneChange::MaterialAssignment,
            // The program recompiles and textures may be new.
            Command::SetGraph { .. } => SceneChange::MaterialAssignment,
            Command::SetLightingPreset { .. } | Command::SetLightingConfig { .. } => {
                SceneChange::GeometryEdit
            }
            Command::SetEnvironmentParameter { parameter } => parameter.param.scene_change(),
            // A pin edit rebuilds the environment's CDF, which is the studio
            // changing — every surface in the scene is lit differently and
            // nothing accumulated so far survives.
            Command::SetEnvironmentPins { .. } => SceneChange::ModelSetOrStudioSwitch,
            Command::SetSky { .. } => SceneChange::ModelSetOrStudioSwitch,
            Command::AddLight { .. } => SceneChange::LightAddedOrRemoved,
            // A light that moved or changed colour is a light-parameter edit,
            // not a geometry one: §9.3 keeps accumulation outside the region
            // it affects.
            Command::SetLight { .. } => SceneChange::LightParameter,
            // What is seen changes everywhere the cutter reaches, and every
            // shadow the removed part cast goes with it.
            Command::AddCutaway { .. } | Command::SetCutaway { .. } => SceneChange::GeometryEdit,
            Command::SelectCamera { .. } | Command::SetCamera { .. } => {
                SceneChange::CameraTransform
            }
            Command::SetImageStyle { .. } | Command::SetColorManagement { .. } => {
                SceneChange::ImageStyle
            }
            // Nothing here reaches the viewport at all: a render's resolution,
            // its output folder and its sample limit are read when Render is
            // pressed and not before.
            Command::SetRenderSettings { .. } => SceneChange::SelectionChange,
            // Renames, locks and metadata change nothing the renderer sees —
            // and neither does saving or discarding a camera that is not the
            // one being looked through.
            Command::AddGroup { .. }
            | Command::Rename { .. }
            | Command::SetLocked { .. }
            | Command::SetPivot { .. }
            | Command::SetMetadata { .. }
            | Command::AddCamera { .. }
            | Command::RemoveCamera { .. }
            | Command::RenameCamera { .. }
            | Command::RenameMaterial { .. }
            // An annotation over the image, never in it.
            | Command::SetMeasurements { .. }
            | Command::SetStudios { .. }
            | Command::SetMaterialTemplates { .. }
            | Command::SetConfigurator { .. } => SceneChange::SelectionChange,
            // A variant switch is an assignment (`docs/02` §8.1).
            Command::SetMultiMaterials { .. } => SceneChange::MaterialAssignment,
            Command::SetModelSets { .. } | Command::ApplyStudio { .. } | Command::Configure { .. } => {
                SceneChange::ModelSetOrStudioSwitch
            }
        }
    }

    pub fn dirty(&self) -> Dirty {
        self.scene_change().dirty()
    }

    /// Whether this command *could* absorb `previous`, if the two are part of
    /// one continuous interaction.
    ///
    /// This answers only "are these the same kind of edit to the same target".
    /// Whether they are actually one interaction is not something a command
    /// can know — two identical edits a minute apart are two edits — so the
    /// bus decides, using the barrier that [`CommandBus::end_interaction`]
    /// raises. §8.1's example is the honest statement of the intent: "dragging
    /// a slider produces one undo entry, not two hundred". Two deliberate
    /// values typed into the same field are two entries.
    pub fn can_merge_with(&self, previous: &Command) -> bool {
        match (self, previous) {
            (Command::SetDecals { material: a, .. }, Command::SetDecals { material: b, .. }) => a == b,
            // Dragging a node, or a literal's slider, issues one per frame;
            // the interaction barrier ends the run, so two separate edits
            // still undo separately.
            (Command::SetGraph { material: a, .. }, Command::SetGraph { material: b, .. }) => a == b,
            (
                Command::SetMaterialParameter {
                    material: a,
                    parameter: pa,
                },
                Command::SetMaterialParameter {
                    material: b,
                    parameter: pb,
                },
            ) => a == b && pa.param == pb.param,
            (
                Command::SetEnvironmentParameter { parameter: pa },
                Command::SetEnvironmentParameter { parameter: pb },
            ) => pa.param == pb.param,
            // Dragging any image-style slider, and orbiting the viewport, are
            // both continuous interactions that must leave one undo entry.
            (Command::SetImageStyle { .. }, Command::SetImageStyle { .. }) => true,
            (Command::SetRenderSettings { .. }, Command::SetRenderSettings { .. }) => true,
            (Command::SetCamera { index: a, .. }, Command::SetCamera { index: b, .. }) => a == b,
            (Command::SetLight { node: a, .. }, Command::SetLight { node: b, .. }) => a == b,
            (Command::SetCutaway { node: a, .. }, Command::SetCutaway { node: b, .. }) => a == b,
            // A drag issues one of these per frame; they merge into a single
            // undo step only while the *same* parts are being moved. Merging
            // across different selections would fold two deliberate edits into
            // one the user cannot separate.
            (Command::SetTransform { transforms: a }, Command::SetTransform { transforms: b }) => {
                a.len() == b.len() && a.iter().zip(b).all(|((a, _), (b, _))| a == b)
            }
            _ => false,
        }
    }
}

/// What a command has to remember in order to undo itself.
#[derive(Clone, Debug, PartialEq)]
enum Undo {
    Import {
        nodes: Vec<NodeId>,
        meshes: Vec<MeshKey>,
        materials: Vec<MaterialKey>,
        textures: Vec<tequila_materials::graph::TextureId>,
    },
    AddedNode(NodeId),
    /// An update: what it added, and each touched node as it was before.
    Reimported {
        added: Box<Undo>,
        previous: Vec<(NodeId, NodeKind, Transform, Option<MaterialKey>, std::collections::BTreeMap<String, String>, Option<String>)>,
        hidden: Vec<NodeId>,
    },
    /// A part that became a group of parts. The node keeps its identity, so
    /// undoing is a matter of putting its mesh back and taking the children
    /// away — which is also why a material assigned to the part before the
    /// split is still assigned to it after undoing one.
    Split {
        node: NodeId,
        mesh: MeshKey,
        children: Vec<NodeId>,
        meshes: Vec<MeshKey>,
    },
    /// Removed nodes, deepest first, each with the sibling index it occupied.
    RemovedNodes(Vec<(SceneNode, usize)>),
    Name(NodeId, String),
    Transforms(Vec<(NodeId, Transform)>),
    Visibility(Vec<(NodeId, bool)>),
    Locked(Vec<(NodeId, bool)>),
    Pivots(Vec<(NodeId, Option<Vec3>)>),
    Parent {
        node: NodeId,
        parent: Option<NodeId>,
        index: usize,
    },
    Grouped {
        group: NodeId,
        members: Vec<(NodeId, Option<NodeId>, usize)>,
    },
    Ungrouped {
        /// Boxed because a `SceneNode` is by far the largest thing any undo
        /// record holds, and an unboxed one would make *every* entry in the
        /// history that size — including the thousands left by a drag.
        group: Box<SceneNode>,
        index: usize,
        members: Vec<(NodeId, usize)>,
    },
    Metadata(NodeId, String, Option<String>),
    /// Boxed for the same reason as `Ungrouped`: it holds the cameras, the
    /// environment and every top node's transform.
    Units(SceneUnits, Box<crate::units::Rescaled>),
    Materials(Vec<(NodeId, Option<MaterialKey>)>),
    /// Unlink also created a material, which undo must remove again.
    Unlinked {
        node: NodeId,
        previous: Option<MaterialKey>,
        created: MaterialKey,
    },
    MaterialParameter(MaterialKey, MaterialParameter),
    /// The whole definition, because switching type rewrites the parameter
    /// block and there is no smaller thing to remember.
    MaterialType(MaterialKey, Box<MaterialDefinition>),
    /// The definition and the texture store, because attaching a texture adds
    /// nodes, links, and possibly a texture reference — and detaching removes
    /// them. Restoring the pieces one at a time would leave the graph briefly
    /// invalid, and the id has to come back as it was or the graph points at
    /// nothing.
    ParameterTexture {
        material: MaterialKey,
        definition: Box<MaterialDefinition>,
        textures: Box<TextureStore>,
    },
    MaterialName(MaterialKey, String),
    Lighting(Box<tequila_render_core::settings::LightingSettings>),
    EnvironmentParameter(EnvironmentParameter),
    EnvironmentPins(Vec<tequila_render_core::pins::Pin>),
    Sky(Box<Option<tequila_render_core::sky::Sky>>),
    Light(NodeId, Box<LightSettings>),
    Cutaway(NodeId, Box<crate::cutaway::CutawaySettings>),
    Measurements(Vec<crate::measure::Measurement>),
    MultiMaterials(Vec<crate::studio::MultiMaterial>, Vec<(NodeId, Option<MaterialKey>)>),
    ModelSets(Box<crate::studio::ModelSets>),
    Studios(Vec<crate::studio::Studio>),
    MaterialTemplates(Vec<crate::templates::MaterialTemplate>),
    /// Everything a Studio can change, as it was.
    StudioApplied(Box<StudioUndo>),
    Configurator(Box<crate::configurator::Configurator>),
    /// Everything a configuration can change, as it was, and the choices.
    Configured(Box<StudioUndo>, Vec<usize>),
    /// The whole camera list. Small, and removing a camera can change which
    /// one is active — restoring the pieces separately would leave a moment
    /// with the wrong camera selected.
    Cameras(Box<crate::lighting::CameraStore>),
    ImageStyle(Box<ImageStyle>),
    ColorManagement(Box<Option<OcioSelection>>),
    RenderSettings(Box<RenderSettings>),
}

/// A command that has been applied, with what it needs to undo itself.
#[derive(Clone, Debug)]
struct Applied {
    command: Command,
    undo: Undo,
    /// The group this was applied in, if any. Consecutive entries sharing a
    /// group are one step to undo and redo. See [`CommandBus::begin_group`].
    group: Option<u64>,
}

/// The command bus: applies commands, and owns the undo history.
#[derive(Debug, Default)]
pub struct CommandBus {
    history: Vec<Applied>,
    /// Number of entries in `history` that are currently applied. Redo lives
    /// above it.
    depth: usize,
    /// Every command ever applied, in order, for crash recovery (§8.4).
    journal: Vec<Command>,
    /// Commands applied since the last save, used to decide whether the
    /// document is dirty on disk.
    unsaved: usize,
    /// Every change to the history ever made - each apply, undo, redo and
    /// abandoned step - never reset. Two moments with the same count have
    /// the same history; two with the same depth need not.
    changes: u64,
    /// Set when the next command must start its own undo step rather than
    /// merging into the previous one. See [`CommandBus::end_interaction`].
    barrier: bool,
    /// The group commands are currently being applied into, and, for each
    /// `begin_group` waiting for its `end_group`, where in the history it
    /// began: a nested batch that fails takes back its own steps, not the
    /// ones the batch around it had already made.
    open_group: Option<u64>,
    group_starts: Vec<usize>,
    next_group: u64,
}

impl CommandBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a command, pushing it onto the undo history.
    pub fn apply(
        &mut self,
        document: &mut SceneDocument,
        command: Command,
    ) -> Result<Dirty, CommandError> {
        let dirty = command.dirty();
        let undo = execute(document, &command, Reissue::FRESH)?;

        // Redo is invalidated by a new edit: the branch it pointed at no
        // longer follows from the current state.
        self.history.truncate(self.depth);

        // A barrier means the previous entry belongs to a finished
        // interaction, so this command starts its own step even if it is the
        // same kind of edit to the same target. Nor does anything merge
        // across a group's edge: an entry inside a group swallowing one from
        // outside it would make undoing the group take the outsider too.
        let merged = !self.barrier
            && self
                .history
                .last()
                .map(|prev| prev.group == self.open_group && command.can_merge_with(&prev.command))
                .unwrap_or(false);
        self.barrier = false;

        if merged {
            // Keep the *earlier* undo record — it restores the state from
            // before the whole drag — and the later command, which describes
            // where the drag ended.
            let entry = self.history.last_mut().expect("merged implies non-empty");
            entry.command = command.clone();
        } else {
            self.history.push(Applied {
                command: command.clone(),
                undo,
                group: self.open_group,
            });
            self.depth = self.history.len();
        }
        self.journal.push(command);
        self.unsaved += 1;
        self.changes += 1;
        Ok(dirty)
    }

    pub fn can_undo(&self) -> bool {
        self.depth > 0
    }

    /// How many history entries are applied — the position undo and redo
    /// move. Crash recovery compares it against where its journal starts.
    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn can_redo(&self) -> bool {
        self.depth < self.history.len()
    }

    /// Name of the step undo would reverse, for the history panel (§8.4).
    pub fn undo_name(&self) -> Option<&'static str> {
        self.history
            .get(self.depth.checked_sub(1)?)
            .map(|e| e.command.name())
    }

    pub fn redo_name(&self) -> Option<&'static str> {
        self.history.get(self.depth).map(|e| e.command.name())
    }

    /// The step undo would take back: its last command's name, and how many
    /// commands it holds (more than one for a group).
    pub fn undo_step(&self) -> Option<(&'static str, usize)> {
        let top = self.depth.checked_sub(1)?;
        let entry = &self.history[top];
        let count = match entry.group {
            None => 1,
            Some(group) => self.history[..=top]
                .iter()
                .rev()
                .take_while(|e| e.group == Some(group))
                .count(),
        };
        Some((entry.command.name(), count))
    }

    /// The step redo would put back, likewise.
    pub fn redo_step(&self) -> Option<(&'static str, usize)> {
        let entry = self.history.get(self.depth)?;
        let count = match entry.group {
            None => 1,
            Some(group) => self.history[self.depth..]
                .iter()
                .take_while(|e| e.group == Some(group))
                .count(),
        };
        Some((entry.command.name(), count))
    }

    /// Names of every step, oldest first, for the history panel.
    pub fn history_names(&self) -> Vec<&'static str> {
        self.history.iter().map(|e| e.command.name()).collect()
    }

    /// Start applying commands as one undo step.
    ///
    /// Everything applied until the matching [`CommandBus::end_group`] is
    /// undone and redone together. It exists for scripts: a script that sets
    /// up thirty materials is one thing a person did, and making them press
    /// Ctrl+Z thirty times to take it back — watching the scene come apart a
    /// part at a time — is how scripting earns a reputation for making messes.
    ///
    /// Groups nest by counting: an inner begin/end pair is absorbed into the
    /// outer group, so a script that runs a script is still one step.
    pub fn begin_group(&mut self) {
        if self.group_starts.is_empty() {
            self.open_group = Some(self.next_group);
            self.next_group += 1;
            self.barrier = true;
        }
        self.group_starts.push(self.depth);
    }

    /// Close the group opened by the matching [`CommandBus::begin_group`].
    /// Unbalanced calls are ignored rather than trusted.
    pub fn end_group(&mut self) {
        if self.group_starts.pop().is_some() && self.group_starts.is_empty() {
            self.open_group = None;
            self.barrier = true;
        }
    }

    /// Whether a group is open.
    pub fn in_group(&self) -> bool {
        !self.group_starts.is_empty()
    }

    /// Take back everything applied in the open group, and close it.
    ///
    /// For a batch that failed part-way: the commands before the failure were
    /// written against a plan that did not finish, and leaving them in place
    /// leaves the scene in a state nobody chose. The reverted steps are
    /// dropped from redo as well — redoing half of a failed batch is not an
    /// undo anyone wants to be offered. Returns how many commands were
    /// reverted.
    pub fn abandon_group(&mut self, document: &mut SceneDocument) -> Result<usize, CommandError> {
        let Some(group) = self.open_group else {
            return Ok(0);
        };
        let start = self.group_starts.pop().unwrap_or(0);
        let mut reverted = 0;
        while let Some(index) = self.depth.checked_sub(1) {
            if index < start || self.history[index].group != Some(group) {
                break;
            }
            let undo = self.history[index].undo.clone();
            revert(document, &undo)?;
            self.depth = index;
            reverted += 1;
        }
        self.history.truncate(self.depth);
        self.unsaved += reverted;
        self.changes += reverted as u64;
        // Only this level closes; a batch around it carries on.
        if self.group_starts.is_empty() {
            self.open_group = None;
        }
        self.barrier = true;
        Ok(reverted)
    }

    /// End the current continuous interaction, so the next command starts a
    /// new undo step.
    ///
    /// Called when a drag is released, a text field is committed, or focus
    /// leaves a control. Without it the merge policy cannot tell a drag from
    /// two deliberate edits of the same value, and silently swallows the
    /// first — which looks like undo skipping a step.
    ///
    /// Idempotent, so a caller that is unsure may call it freely; the cost of
    /// an extra barrier is one more undo step, and the cost of a missing one
    /// is a lost edit.
    pub fn end_interaction(&mut self) {
        self.barrier = true;
    }

    pub fn undo(&mut self, document: &mut SceneDocument) -> Result<Option<Dirty>, CommandError> {
        // Whatever came before an undo is finished by definition; a command
        // applied after it must not merge backwards into it. That includes a
        // group left open: undoing into the middle of one would otherwise let
        // the next command join a group whose start is gone.
        self.barrier = true;
        self.group_starts.clear();
        self.open_group = None;
        let Some(first) = self.depth.checked_sub(1) else {
            return Ok(None);
        };
        let group = self.history[first].group;
        let mut dirty = Dirty::None;
        // One entry, or every consecutive entry of its group.
        while let Some(index) = self.depth.checked_sub(1) {
            let entry = &self.history[index];
            if index != first && (group.is_none() || entry.group != group) {
                break;
            }
            dirty = dirty.max(entry.command.dirty());
            let undo = entry.undo.clone();
            revert(document, &undo)?;
            self.depth = index;
            self.unsaved += 1;
            self.changes += 1;
        }
        Ok(Some(dirty))
    }

    pub fn redo(&mut self, document: &mut SceneDocument) -> Result<Option<Dirty>, CommandError> {
        self.barrier = true;
        self.group_starts.clear();
        self.open_group = None;
        if self.depth >= self.history.len() {
            return Ok(None);
        }
        let group = self.history[self.depth].group;
        let first = self.depth;
        let mut dirty = Dirty::None;
        while self.depth < self.history.len() {
            let entry = &self.history[self.depth];
            if self.depth != first && (group.is_none() || entry.group != group) {
                break;
            }
            let command = entry.command.clone();
            dirty = dirty.max(command.dirty());
            // Replay under the identifiers it issued the first time, so redo
            // restores the scene rather than renumbering it.
            let reissue = Reissue::from_undo(&entry.undo);
            let undo = execute(document, &command, reissue)?;
            self.history[self.depth].undo = undo;
            self.depth += 1;
            self.unsaved += 1;
            self.changes += 1;
        }
        Ok(Some(dirty))
    }

    /// The journal, for writing to disk alongside the document.
    pub fn journal(&self) -> &[Command] {
        &self.journal
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.unsaved > 0
    }

    /// See the field: a count that tells one history from another where the
    /// depth alone cannot, as after an undo and a fresh edit by hand.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    pub fn mark_saved(&mut self) {
        self.unsaved = 0;
    }

    /// Unsaved although no command says so: work recovered from an
    /// auto-save snapshot is in no file the person chose.
    pub fn mark_unsaved(&mut self) {
        self.unsaved = self.unsaved.max(1);
    }

    /// Forget the history, keeping the document as it stands. Used after a
    /// load, where the previous document's history is meaningless.
    pub fn reset(&mut self) {
        self.changes += 1;
        self.history.clear();
        self.journal.clear();
        self.depth = 0;
        self.unsaved = 0;
        self.barrier = false;
        self.group_starts.clear();
        self.open_group = None;
    }
}

/// Create a group node, under a reissued ID when redoing.
fn add_group(
    document: &mut SceneDocument,
    reissue: Option<NodeId>,
    parent: Option<NodeId>,
    name: &str,
) -> Result<NodeId, CommandError> {
    match reissue {
        Some(id) => {
            if document.tree.contains(id) {
                return Err(CommandError::InconsistentHistory(
                    "redo would reissue a node ID that is already in use",
                ));
            }
            Ok(document.add_node_with_id(id, parent, name, NodeKind::Group)?)
        }
        None => Ok(document.add_node(parent, name, NodeKind::Group)?),
    }
}

/// The identifiers a command handed out last time it ran.
///
/// A command that creates something — a group, an unlinked material — normally
/// allocates a fresh ID. On *redo* it must not: the node it re-creates has to
/// carry the ID it had before, or every reference recorded against it while it
/// existed (material assignment, selection, animation track) is left dangling,
/// and undo-then-redo silently renumbers the scene.
///
/// Redo therefore replays the command with the IDs recovered from its own undo
/// record. `apply` passes [`Reissue::FRESH`], because a newly issued command
/// genuinely should get new IDs — §2.3 forbids reusing a freed one.
#[derive(Clone, Debug, Default)]
struct Reissue {
    node: Option<NodeId>,
    material: Option<MaterialKey>,
    /// The child nodes and meshes a previous run of a split created.
    ///
    /// Redo has to reuse them rather than allocate again. Fresh identifiers
    /// would leave the redone parts unreachable from anything that referred to
    /// them — a selection, a material assignment, a saved camera — and would
    /// grow the id space every time somebody pressed undo and redo.
    split: Option<(Vec<NodeId>, Vec<MeshKey>)>,
}

/// What applying a Studio replaced.
#[derive(Clone, Debug, PartialEq)]
pub struct StudioUndo {
    /// All of them, and which was active: a Studio's camera goes to the
    /// saved camera of its name, which need not be the active one.
    cameras: crate::lighting::CameraStore,
    environment: crate::document::EnvironmentSettings,
    image_style: tequila_color::ImageStyle,
    model_set: Option<usize>,
    multi_materials: Vec<crate::studio::MultiMaterial>,
    materials: Vec<(NodeId, Option<MaterialKey>)>,
    /// Every light's on-or-off.
    lights: Vec<(NodeId, bool)>,
    /// The product's roots' transforms, and its turn.
    transforms: Vec<(NodeId, Transform)>,
    turn: crate::studio::ProductTurn,
}

impl StudioUndo {
    /// Everything a Studio can change, as it is now; the part materials
    /// are filled in once the variants are applied.
    fn of(document: &SceneDocument) -> StudioUndo {
        StudioUndo {
            cameras: document.cameras.clone(),
            environment: document.environment.clone(),
            image_style: document.image_style,
            model_set: document.model_sets.active,
            multi_materials: document.multi_materials.clone(),
            materials: Vec::new(),
            lights: document
                .tree
                .iter()
                .filter_map(|n| match &n.kind {
                    NodeKind::Light { settings } => Some((n.id, settings.enabled)),
                    _ => None,
                })
                .collect(),
            transforms: crate::studio::product_roots(document)
                .into_iter()
                .filter_map(|id| document.tree.get(id).map(|n| (id, n.transform)))
                .collect(),
            turn: document.product_turn,
        }
    }

    fn restore(&self, document: &mut SceneDocument) {
        crate::studio::restore_materials(document, &self.materials);
        document.cameras = self.cameras.clone();
        document.environment = self.environment.clone();
        document.image_style = self.image_style;
        document.model_sets.active = self.model_set;
        document.multi_materials = self.multi_materials.clone();
        for &(id, enabled) in &self.lights {
            if let Ok(node) = document.tree.get_mut(id) {
                if let NodeKind::Light { settings } = &mut node.kind {
                    settings.enabled = enabled;
                }
            }
        }
        for &(id, transform) in &self.transforms {
            if let Ok(node) = document.tree.get_mut(id) {
                node.transform = transform;
            }
        }
        document.product_turn = self.turn;
    }
}

/// Make the scene what `studio` says, leaving its variants for
/// [`crate::studio::apply_variants`] to assign.
fn put_studio(document: &mut SceneDocument, studio: crate::studio::Studio) {
    // To the saved camera of its name, made active, so a set of views
    // shares the cameras it names rather than writing each over whichever
    // camera was active; one without a namesake takes the active one's
    // place, as it always did.
    if let Some(camera) = studio.camera {
        if let Some(index) = document.cameras.find(&camera.name) {
            document.cameras.select(index);
        }
        *document.cameras.active_mut() = camera;
    }
    crate::studio::switch_lights(document, &studio.lights);
    if let Some(yaw) = studio.product_yaw {
        crate::studio::turn_product(document, yaw);
    }
    if let Some(environment) = studio.environment {
        document.environment = environment;
    }
    if let Some(style) = studio.image_style {
        document.image_style = style;
    }
    if let Some(set) = studio.model_set {
        // A set deleted since the Studio was saved shows everything
        // rather than refusing the whole Studio.
        document.model_sets.active = set.filter(|&i| i < document.model_sets.sets.len());
    }
    for (id, active) in studio.variants {
        if let Some(multi) = document.multi_materials.iter_mut().find(|m| m.id == id) {
            if active < multi.variants.len() {
                multi.active = active;
            }
        }
    }
}

impl Reissue {
    /// Allocate new identifiers, as a command applied for the first time does.
    const FRESH: Reissue = Reissue {
        node: None,
        material: None,
        split: None,
    };

    /// The identifiers recorded by a previous run of the same command.
    fn from_undo(undo: &Undo) -> Reissue {
        match undo {
            Undo::AddedNode(node) => Reissue {
                node: Some(*node),
                material: None,
                split: None,
            },
            Undo::Split {
                children, meshes, ..
            } => Reissue {
                node: None,
                material: None,
                split: Some((children.clone(), meshes.clone())),
            },
            Undo::Grouped { group, .. } => Reissue {
                node: Some(*group),
                material: None,
                split: None,
            },
            Undo::Unlinked { created, .. } => Reissue {
                node: None,
                material: Some(*created),
                split: None,
            },
            _ => Reissue::FRESH,
        }
    }
}

/// Apply a command and produce its undo record.
fn execute(
    document: &mut SceneDocument,
    command: &Command,
    reissue: Reissue,
) -> Result<Undo, CommandError> {
    match command {
        Command::Import { payload } => {
            // Every parent checked before anything is written, so a refusal
            // leaves the document as it was. A scripted import naming a
            // parent that is not there left an orphan node behind, with its
            // meshes and materials and no step to undo, and a later Delete
            // of everything stopped part way on it.
            let arriving: std::collections::HashSet<NodeId> = payload.nodes.iter().map(|n| n.id).collect();
            for node in &payload.nodes {
                if let Some(parent) = node.parent.filter(|p| !arriving.contains(p)) {
                    document.tree.try_get(parent)?;
                }
            }
            for (key, mesh) in &payload.meshes {
                document.geometry.insert_with_key(*key, mesh.clone());
            }
            // Before the materials, whose graphs refer to them.
            for (id, source) in &payload.textures {
                document.textures.insert_with_id(*id, source.clone());
            }
            for (key, material) in &payload.materials {
                document.materials.insert_with_key(*key, material.clone());
            }
            // Nodes arrive parent-before-child, each appended after its
            // existing siblings. `usize::MAX` means "append", which `reinsert`
            // clamps — that is right for both a root and a child, and avoids
            // having to special-case the two.
            for node in &payload.nodes {
                document.tree.reinsert(node.clone(), usize::MAX)?;
            }
            Ok(Undo::Import {
                nodes: payload.roots.clone(),
                meshes: payload.meshes.iter().map(|(k, _)| *k).collect(),
                materials: payload.materials.iter().map(|(k, _)| *k).collect(),
                textures: payload.textures.iter().map(|(id, _)| *id).collect(),
            })
        }

        Command::Reimport { payload } => {
            // Everything checked before anything is written, so a refusal
            // leaves the document as it was.
            for update in &payload.updates {
                document.tree.try_get(update.node)?;
            }
            for &node in &payload.hidden {
                document.tree.try_get(node)?;
            }
            let added = execute(document, &Command::Import { payload: Box::new(payload.added.clone()) }, Reissue::FRESH)?;
            let mut previous = Vec::with_capacity(payload.updates.len());
            for update in &payload.updates {
                let node = document.tree.get_mut(update.node)?;
                previous.push((
                    update.node,
                    node.kind.clone(),
                    node.transform,
                    node.material,
                    node.metadata.clone(),
                    node.path_key.clone(),
                ));
                node.kind = update.kind.clone();
                if let Some(transform) = update.transform {
                    node.transform = transform;
                }
                node.material = update.material;
                node.metadata = update.metadata.clone();
                node.path_key = update.path_key.clone();
            }
            for &node in &payload.hidden {
                document.tree.get_mut(node)?.visible = false;
            }
            Ok(Undo::Reimported { added: Box::new(added), previous, hidden: payload.hidden.clone() })
        }

        Command::AddGroup { parent, name } => {
            let id = add_group(document, reissue.node, *parent, name)?;
            Ok(Undo::AddedNode(id))
        }

        Command::DeleteNodes { nodes } => {
            let mut removed = Vec::new();
            for &node in nodes {
                check_unlocked(document, node)?;
            }
            for &node in nodes {
                // Skip anything already taken as part of an earlier subtree.
                if !document.tree.contains(node) {
                    continue;
                }
                let index = document.tree.sibling_index(node).unwrap_or(0);
                let subtree = document.tree.remove_subtree(node)?;
                // Only the root of the subtree needs its index remembered; the
                // rest are restored relative to their parents. The root comes
                // last, the subtree being deepest first: remembered on the
                // first node, the index went to a leaf, and an undone group
                // came back at the end of its siblings.
                for n in subtree {
                    let at = if n.id == node { index } else { usize::MAX };
                    removed.push((n, at));
                }
            }
            Ok(Undo::RemovedNodes(removed))
        }

        Command::SplitIntoParts { node } => {
            check_unlocked(document, *node)?;
            let source = document
                .tree
                .get(*node)
                .ok_or(CommandError::InconsistentHistory("no such node"))?;
            let NodeKind::Part { mesh } = source.kind else {
                return Err(CommandError::InconsistentHistory(
                    "only a part can be split",
                ));
            };
            if !source.children.is_empty() {
                return Err(CommandError::InconsistentHistory(
                    "a part with children cannot be split",
                ));
            }
            let name = source.name.clone();
            let material = source.material;

            let geometry = document
                .geometry
                .get(mesh)
                .ok_or(CommandError::InconsistentHistory("the part has no mesh"))?;
            let parts = crate::split::into_parts(geometry).ok_or(
                CommandError::InconsistentHistory("the mesh is a single connected shell"),
            )?;

            // Reuse the identifiers a previous run allocated, so redo produces
            // the same nodes and meshes rather than a second set.
            let (child_ids, mesh_keys) = match &reissue.split {
                Some((nodes, meshes)) if nodes.len() == parts.len() => {
                    (nodes.clone(), meshes.clone())
                }
                _ => {
                    let (meshes, _, nodes) =
                        reserve_keys(document, parts.len(), 0, parts.len());
                    (nodes, meshes)
                }
            };

            let names = crate::split::part_names(&name, parts.len());
            for (index, part) in parts.into_iter().enumerate() {
                document
                    .geometry
                    .insert_with_key(mesh_keys[index], part);
                let mut child = SceneNode::new(
                    child_ids[index],
                    names[index].clone(),
                    NodeKind::Part {
                        mesh: mesh_keys[index],
                    },
                );
                child.parent = Some(*node);
                // The children inherit the material, so a split changes what
                // the model is made of and not how it looks. They sit at the
                // parent's origin because the split kept world positions in
                // the geometry rather than moving them into transforms.
                child.material = material;
                document.tree.reinsert(child, usize::MAX)?;
            }

            // The node itself becomes the group, keeping its id, name,
            // transform and place among its siblings — so every reference to
            // it still resolves and the model does not jump.
            let parent = document.tree.get_mut(*node)?;
            parent.kind = NodeKind::Group;
            parent.material = None;

            Ok(Undo::Split {
                node: *node,
                mesh,
                children: child_ids,
                meshes: mesh_keys,
            })
        }

        Command::Rename { node, name } => {
            check_unlocked(document, *node)?;
            let previous = document.tree.get_mut(*node)?.name.clone();
            document.tree.get_mut(*node)?.name = name.clone();
            Ok(Undo::Name(*node, previous))
        }

        Command::SetTransform { transforms } => {
            // Every node is checked before any is written. A command that
            // moved four parts and then refused on the fifth would leave the
            // document in a state the user never asked for and cannot undo,
            // since the refusal produces no undo record.
            for (node, _) in transforms {
                check_unlocked(document, *node)?;
                document.tree.try_get(*node)?;
            }
            let mut previous = Vec::with_capacity(transforms.len());
            for (node, transform) in transforms {
                previous.push((*node, document.tree.get_mut(*node)?.transform));
                document.tree.get_mut(*node)?.transform = *transform;
            }
            Ok(Undo::Transforms(previous))
        }

        Command::SetVisibility { states } => {
            for (node, _) in states {
                document.tree.try_get(*node)?;
            }
            let mut previous = Vec::with_capacity(states.len());
            for (node, visible) in states {
                previous.push((*node, document.tree.get_mut(*node)?.visible));
                document.tree.get_mut(*node)?.visible = *visible;
            }
            Ok(Undo::Visibility(previous))
        }

        Command::SetPivot { pivots } => {
            for (node, _) in pivots {
                check_unlocked(document, *node)?;
                document.tree.try_get(*node)?;
            }
            let mut previous = Vec::with_capacity(pivots.len());
            for (node, pivot) in pivots {
                previous.push((*node, document.tree.get_mut(*node)?.pivot));
                document.tree.get_mut(*node)?.pivot = *pivot;
            }
            Ok(Undo::Pivots(previous))
        }

        Command::SetLocked { states } => {
            for (node, _) in states {
                document.tree.try_get(*node)?;
            }
            let mut previous = Vec::with_capacity(states.len());
            for (node, locked) in states {
                previous.push((*node, document.tree.get_mut(*node)?.locked));
                document.tree.get_mut(*node)?.locked = *locked;
            }
            Ok(Undo::Locked(previous))
        }

        Command::Reparent {
            node,
            parent,
            index,
        } => {
            check_unlocked(document, *node)?;
            let previous_parent = document.tree.try_get(*node)?.parent;
            let previous_index = document.tree.sibling_index(*node).unwrap_or(0);
            document.tree.reparent(*node, *parent, *index)?;
            Ok(Undo::Parent {
                node: *node,
                parent: previous_parent,
                index: previous_index,
            })
        }

        Command::Group { nodes, name } => {
            if nodes.is_empty() {
                return Err(CommandError::InconsistentHistory("grouping nothing"));
            }
            for &node in nodes {
                check_unlocked(document, node)?;
            }
            // The group is created beside the first member, so the grouped
            // content stays where it was in the tree rather than jumping to
            // the root.
            let anchor = nodes[0];
            let parent = document.tree.try_get(anchor)?.parent;
            let at = document.tree.sibling_index(anchor).unwrap_or(0);

            let members: Vec<(NodeId, Option<NodeId>, usize)> = nodes
                .iter()
                .map(|&n| {
                    (
                        n,
                        document.tree.get(n).and_then(|node| node.parent),
                        document.tree.sibling_index(n).unwrap_or(0),
                    )
                })
                .collect();

            let group = add_group(document, reissue.node, parent, name)?;
            document.tree.reparent(group, parent, Some(at))?;
            for &node in nodes {
                document.tree.reparent(node, Some(group), None)?;
            }
            Ok(Undo::Grouped { group, members })
        }

        Command::Ungroup { group } => {
            check_unlocked(document, *group)?;
            let node = document.tree.try_get(*group)?.clone();
            let parent = node.parent;
            let at = document.tree.sibling_index(*group).unwrap_or(0);
            let members: Vec<(NodeId, usize)> = node
                .children
                .iter()
                .enumerate()
                .map(|(i, &child)| (child, i))
                .collect();

            // Promote children into the group's slot, in order.
            for (offset, &(child, _)) in members.iter().enumerate() {
                document.tree.reparent(child, parent, Some(at + offset))?;
            }
            let removed = document.tree.remove_subtree(*group)?;
            let group_node = removed
                .into_iter()
                .next()
                .ok_or(CommandError::InconsistentHistory("group vanished"))?;
            Ok(Undo::Ungrouped {
                group: Box::new(group_node),
                index: at + members.len(),
                members,
            })
        }

        Command::SetMetadata { node, key, value } => {
            let previous = document.tree.get_mut(*node)?.metadata.get(key).cloned();
            match value {
                Some(value) => {
                    document
                        .tree
                        .get_mut(*node)?
                        .metadata
                        .insert(key.clone(), value.clone());
                }
                None => {
                    document.tree.get_mut(*node)?.metadata.remove(key);
                }
            }
            Ok(Undo::Metadata(*node, key.clone(), previous))
        }

        Command::SetUnits { unit, mode } => {
            let previous = document.units;
            let plan = ConversionPlan::compute(previous, *unit, *mode);
            let rescaled = crate::units::rescale(document, plan.geometry_scale);
            document.units = plan.to;
            Ok(Undo::Units(previous, Box::new(rescaled)))
        }

        Command::AssignMaterial { nodes, material } => {
            if document.materials.get(*material).is_none() {
                return Err(CommandError::NoSuchMaterial(*material));
            }
            let mut previous = Vec::with_capacity(nodes.len());
            for &node in nodes {
                check_unlocked(document, node)?;
                previous.push((node, document.tree.try_get(node)?.material));
            }
            for &node in nodes {
                document.tree.get_mut(node)?.material = Some(*material);
            }
            Ok(Undo::Materials(previous))
        }

        Command::UnlinkMaterial { node } => {
            check_unlocked(document, *node)?;
            let current =
                document
                    .tree
                    .try_get(*node)?
                    .material
                    .ok_or(CommandError::InconsistentHistory(
                        "node has no material to unlink",
                    ))?;
            let created = match reissue.material {
                Some(key) => {
                    if document.materials.get(key).is_some() {
                        return Err(CommandError::InconsistentHistory(
                            "redo would reissue a material key that is already in use",
                        ));
                    }
                    document.materials.unlink_with_key(current, key)
                }
                None => document.materials.unlink(current),
            }
            .ok_or(CommandError::NoSuchMaterial(current))?;
            document.tree.get_mut(*node)?.material = Some(created);
            Ok(Undo::Unlinked {
                node: *node,
                previous: Some(current),
                created,
            })
        }

        Command::AddDecal { material, source, name } => {
            let previous = document.materials.get(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?.clone();
            let textures = Box::new(document.textures.clone());
            // Fitted before the texture is interned, because the fit reads the
            // document and interning borrows it mutably.
            let fitted = crate::decal_fit::fit(document, *material);
            let texture = document.textures.intern(source.clone());
            let mut decal = tequila_materials::decal::Decal::new(name, texture);
            // A part with no UV coordinates gets a projected placement instead
            // of the UV one, which on that part addresses nothing. Without
            // this the label is added, the list shows it, and the render does
            // not change — and STL carries no UVs at all, so that is the
            // common case rather than the exotic one.
            if let Some(mapping) = fitted {
                decal.mapping = mapping;
            }
            document.materials.get_mut(*material).unwrap().graph.decals.push(decal);
            Ok(Undo::ParameterTexture { material: *material, definition: Box::new(previous), textures })
        }
        Command::SetDecals { material, decals } => {
            if decals.iter().any(|d| !d.is_valid() || document.textures.get(d.texture).is_none()) {
                return Err(CommandError::InconsistentHistory("invalid decal settings or missing texture"));
            }
            let definition = document.materials.get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            let previous = Box::new(definition.clone());
            definition.graph.decals = decals.clone();
            Ok(Undo::MaterialType(*material, previous))
        }
        Command::SetMaterialParameter {
            material,
            parameter,
        } => {
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            let previous = read_parameter(definition.material(), parameter.param)
                .ok_or(CommandError::NoSuchParameter(parameter.param))?;
            if !parameter.param.write(definition.material_mut(), parameter.value) {
                return Err(CommandError::NoSuchParameter(parameter.param));
            }
            Ok(Undo::MaterialParameter(*material, previous))
        }

        Command::SetMaterialType {
            material,
            material_type,
        } => {
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            let previous = definition.clone();
            definition.retype(*material_type);
            Ok(Undo::MaterialType(*material, Box::new(previous)))
        }

        Command::SetParameterTexture {
            material,
            param,
            source,
        } => {
            if document.materials.get(*material).is_none() {
                return Err(CommandError::NoSuchMaterial(*material));
            }
            let previous = document
                .materials
                .get(*material)
                .expect("checked just above")
                .clone();
            let previous_textures = document.textures.clone();

            match source {
                Some(source) => {
                    let texture = document.textures.intern(source.clone());
                    let definition = document
                        .materials
                        .get_mut(*material)
                        .expect("checked just above");
                    if source.role == tequila_texture::texture::MapRole::Volume {
                        attach_volume(&mut definition.graph, *param, texture);
                    } else {
                        attach_texture(&mut definition.graph, *param, texture);
                        // A height map on Bump or Displacement needs a depth; a
                        // normal map on Bump does not, and is read as one only
                        // when wired straight in.
                        if source.role != tequila_texture::texture::MapRole::Normal || *param == Param::Displacement {
                            through_bump_node(&mut definition.graph, *param);
                        }
                    }
                }
                None => {
                    let definition = document
                        .materials
                        .get_mut(*material)
                        .expect("checked just above");
                    detach_texture(&mut definition.graph, *param);
                    // The texture reference stays in the store; a later
                    // collection pass drops it if nothing else wants it. Doing
                    // it here would renumber ids that other materials still
                    // hold.
                }
            }

            Ok(Undo::ParameterTexture {
                material: *material,
                definition: Box::new(previous),
                textures: Box::new(previous_textures),
            })
        }

        Command::SetParameterProcedural {
            material,
            param,
            procedural,
            seed,
        } => {
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            let previous = definition.clone();
            match procedural {
                Some(procedural) => {
                    retune_procedural(&mut definition.graph, *param, (**procedural).clone(), *seed)
                }
                None => detach_texture(&mut definition.graph, *param),
            }
            Ok(Undo::MaterialType(*material, Box::new(previous)))
        }

        Command::SetParameterGradient {
            material,
            param,
            gradient,
        } => {
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            let previous = definition.clone();
            if !set_gradient(&mut definition.graph, *param, (**gradient).clone()) {
                return Err(CommandError::InconsistentHistory(
                    "that parameter has no colour ramp",
                ));
            }
            Ok(Undo::MaterialType(*material, Box::new(previous)))
        }

        Command::SetTextureMapping {
            material,
            param,
            mapping,
        } => {
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            let previous = definition.clone();
            if !set_mapping(&mut definition.graph, *param, **mapping) {
                return Err(CommandError::InconsistentHistory(
                    "that parameter is not driven by a texture",
                ));
            }
            Ok(Undo::MaterialType(*material, Box::new(previous)))
        }

        Command::SetGraph { material, graph } => {
            // Checked before anything changes: a graph the compiler refuses
            // would leave the material rendering as nothing at all.
            graph.compile().map_err(|e| CommandError::InvalidGraph(format!("{e:?}")))?;
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            let previous = definition.clone();
            definition.graph = (**graph).clone();
            Ok(Undo::MaterialType(*material, Box::new(previous)))
        }

        Command::RenameMaterial { material, name } => {
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            let previous = std::mem::replace(&mut definition.name, name.clone());
            Ok(Undo::MaterialName(*material, previous))
        }

        Command::SetLightingPreset { preset } => {
            let previous = document.lighting.clone();
            document.lighting.select(*preset);
            Ok(Undo::Lighting(Box::new(previous)))
        }

        Command::SetLightingConfig { config } => {
            let previous = document.lighting.clone();
            // Selecting a configuration by hand *is* leaving the preset: the
            // scene no longer matches it, and continuing to display its name
            // would be a lie the user cannot correct.
            document.lighting.preset = None;
            document.lighting.custom = (**config).clone();
            Ok(Undo::Lighting(Box::new(previous)))
        }

        Command::SetEnvironmentParameter { parameter } => {
            let previous = EnvironmentParameter::new(
                parameter.param,
                parameter.param.read(&document.environment),
            );
            parameter
                .param
                .write(&mut document.environment, &parameter.value);
            Ok(Undo::EnvironmentParameter(previous))
        }

        Command::SetSky { sky } => {
            if let Some(sky) = sky.as_ref() {
                if !sky_is_valid(sky) {
                    return Err(CommandError::InconsistentHistory(
                        "a sky has a value the model cannot use",
                    ));
                }
            }
            let previous = std::mem::replace(&mut document.environment.sky, **sky);
            Ok(Undo::Sky(Box::new(previous)))
        }

        Command::SetEnvironmentPins { pins } => {
            if pins.iter().any(|pin| !pin_is_valid(pin)) {
                return Err(CommandError::InconsistentHistory(
                    "a pin has a value the compositor cannot use",
                ));
            }
            let previous = std::mem::replace(&mut document.environment.pins, pins.clone());
            Ok(Undo::EnvironmentPins(previous))
        }

        Command::AddLight {
            parent,
            name,
            settings,
        } => {
            let kind = NodeKind::Light {
                settings: settings.clone(),
            };
            let id = match reissue.node {
                Some(id) => {
                    if document.tree.contains(id) {
                        return Err(CommandError::InconsistentHistory(
                            "redo would reissue a node ID that is already in use",
                        ));
                    }
                    document.add_node_with_id(id, *parent, name, kind)?
                }
                None => document.add_node(*parent, name, kind)?,
            };
            Ok(Undo::AddedNode(id))
        }

        Command::SetLight { node, settings } => {
            check_unlocked(document, *node)?;
            let target = document.tree.get_mut(*node)?;
            let NodeKind::Light {
                settings: current, ..
            } = &mut target.kind
            else {
                return Err(CommandError::NotALight(*node));
            };
            let previous = std::mem::replace(current, settings.clone());
            Ok(Undo::Light(*node, previous))
        }

        Command::AddCutaway {
            parent,
            name,
            settings,
            transform,
        } => {
            if !settings.is_valid() {
                return Err(CommandError::InvalidValue("a cutaway's cap colour must be finite"));
            }
            let kind = NodeKind::Cutaway {
                settings: settings.clone(),
            };
            let id = match reissue.node {
                Some(id) => {
                    if document.tree.contains(id) {
                        return Err(CommandError::InconsistentHistory(
                            "redo would reissue a node ID that is already in use",
                        ));
                    }
                    document.add_node_with_id(id, *parent, name, kind)?
                }
                None => document.add_node(*parent, name, kind)?,
            };
            document.tree.get_mut(id)?.transform = *transform;
            Ok(Undo::AddedNode(id))
        }

        Command::SetCutaway { node, settings } => {
            check_unlocked(document, *node)?;
            if !settings.is_valid() {
                return Err(CommandError::InvalidValue("a cutaway's cap colour must be finite"));
            }
            let target = document.tree.get_mut(*node)?;
            let NodeKind::Cutaway { settings: current } = &mut target.kind else {
                return Err(CommandError::NotACutaway(*node));
            };
            let previous = std::mem::replace(current, settings.clone());
            Ok(Undo::Cutaway(*node, previous))
        }

        Command::SetMultiMaterials { multi_materials } => {
            for multi in multi_materials {
                if multi.variants.iter().any(|&key| document.materials.get(key).is_none()) {
                    return Err(CommandError::InvalidValue("a multi-material names a material that does not exist"));
                }
                if !multi.variants.is_empty() && multi.active >= multi.variants.len() {
                    return Err(CommandError::InvalidValue("a multi-material's active variant is past its last"));
                }
            }
            let previous = std::mem::replace(&mut document.multi_materials, multi_materials.clone());
            let materials = crate::studio::apply_variants(document);
            Ok(Undo::MultiMaterials(previous, materials))
        }

        Command::SetModelSets { model_sets } => {
            if model_sets.active.is_some_and(|i| i >= model_sets.sets.len()) {
                return Err(CommandError::InvalidValue("the active model set is past the last"));
            }
            let previous = std::mem::replace(&mut document.model_sets, (**model_sets).clone());
            Ok(Undo::ModelSets(Box::new(previous)))
        }

        Command::SetMaterialTemplates { templates } => {
            let previous = std::mem::replace(&mut document.material_templates, templates.clone());
            Ok(Undo::MaterialTemplates(previous))
        }

        Command::SetStudios { studios } => {
            let previous = std::mem::replace(&mut document.studios, studios.clone());
            Ok(Undo::Studios(previous))
        }

        Command::ApplyStudio { index } => {
            let Some(studio) = document.studios.get(*index).cloned() else {
                return Err(CommandError::InvalidValue("no studio at that index"));
            };
            let undo = StudioUndo::of(document);
            put_studio(document, studio);
            let materials = crate::studio::apply_variants(document);
            Ok(Undo::StudioApplied(Box::new(StudioUndo { materials, ..undo })))
        }

        Command::SetConfigurator { configurator } => {
            let previous = std::mem::replace(&mut document.configurator, (**configurator).clone());
            Ok(Undo::Configurator(Box::new(previous)))
        }

        Command::Configure { choices } => {
            let configurator = document.configurator.clone();
            if !configurator.is_valid(choices) {
                return Err(CommandError::InvalidValue("that configuration is not one the configurator allows"));
            }
            let undo = StudioUndo::of(document);
            let previous = std::mem::replace(&mut document.configurator.current, choices.clone());
            use crate::configurator::Drives;
            // A Studio first, since it can set a model set or a variant
            // that a later option then overrides.
            for (option, &choice) in configurator.options.iter().zip(choices) {
                if option.drives == Drives::Studio {
                    if let Some(studio) = document.studios.get(option.choices[choice].target).cloned() {
                        put_studio(document, studio);
                    }
                }
            }
            for (option, &choice) in configurator.options.iter().zip(choices) {
                let target = option.choices[choice].target;
                match option.drives {
                    Drives::Studio => {}
                    Drives::ModelSet => {
                        if target < document.model_sets.sets.len() {
                            document.model_sets.active = Some(target);
                        }
                    }
                    Drives::Material { multi_material } => {
                        if let Some(multi) = document.multi_materials.iter_mut().find(|m| m.id == multi_material) {
                            if target < multi.variants.len() {
                                multi.active = target;
                            }
                        }
                    }
                }
            }
            let materials = crate::studio::apply_variants(document);
            Ok(Undo::Configured(Box::new(StudioUndo { materials, ..undo }), previous))
        }

        Command::SetMeasurements { measurements } => {
            if !measurements.iter().all(crate::measure::Measurement::is_valid) {
                return Err(CommandError::InvalidValue("a measurement's ends must be finite"));
            }
            let previous = std::mem::replace(&mut document.measurements, measurements.clone());
            Ok(Undo::Measurements(previous))
        }

        Command::AddCamera { camera } => {
            let previous = document.cameras.clone();
            document.cameras.insert(document.cameras.len(), (**camera).clone());
            Ok(Undo::Cameras(Box::new(previous)))
        }

        Command::RemoveCamera { index } => {
            let previous = document.cameras.clone();
            document.cameras.remove(*index);
            Ok(Undo::Cameras(Box::new(previous)))
        }

        Command::SelectCamera { index } => {
            let previous = document.cameras.clone();
            document.cameras.select(*index);
            Ok(Undo::Cameras(Box::new(previous)))
        }

        Command::SetCamera { index, camera } => {
            let previous = document.cameras.clone();
            if let Some(slot) = document.cameras.get_mut(*index) {
                *slot = (**camera).clone();
            }
            Ok(Undo::Cameras(Box::new(previous)))
        }

        Command::RenameCamera { index, name } => {
            let previous = document.cameras.clone();
            if let Some(slot) = document.cameras.get_mut(*index) {
                slot.name = name.clone();
            }
            Ok(Undo::Cameras(Box::new(previous)))
        }

        Command::SetImageStyle { style } => {
            let previous = document.image_style;
            document.image_style = **style;
            Ok(Undo::ImageStyle(Box::new(previous)))
        }

        Command::SetRenderSettings { settings } => {
            let previous = document.render.clone();
            document.render = (**settings).clone();
            Ok(Undo::RenderSettings(Box::new(previous)))
        }

        Command::SetColorManagement { selection } => {
            let previous = document.ocio.clone();
            document.ocio = selection.clone();
            Ok(Undo::ColorManagement(Box::new(previous)))
        }
    }
}

/// Reverse an applied command.
fn revert(document: &mut SceneDocument, undo: &Undo) -> Result<(), CommandError> {
    match undo {
        Undo::Import {
            nodes,
            meshes,
            materials,
            textures,
        } => {
            for &id in textures {
                document.textures.remove(id);
            }
            for &node in nodes {
                if document.tree.contains(node) {
                    document.tree.remove_subtree(node)?;
                }
            }
            let dropped_meshes: std::collections::BTreeSet<_> = meshes.iter().copied().collect();
            document
                .geometry
                .retain_keys(&|key| !dropped_meshes.contains(&key));
            let dropped_materials: std::collections::BTreeSet<_> =
                materials.iter().copied().collect();
            document
                .materials
                .retain_keys(&|key| !dropped_materials.contains(&key));
            Ok(())
        }

        Undo::AddedNode(id) => {
            document.tree.remove_subtree(*id)?;
            Ok(())
        }

        Undo::Reimported { added, previous, hidden } => {
            for &node in hidden {
                document.tree.get_mut(node)?.visible = true;
            }
            for (node, kind, transform, material, metadata, path_key) in previous {
                let restored = document.tree.get_mut(*node)?;
                restored.kind = kind.clone();
                restored.transform = *transform;
                restored.material = *material;
                restored.metadata = metadata.clone();
                restored.path_key = path_key.clone();
            }
            // Last: the new meshes are what the updated parts pointed at.
            revert(document, added)
        }

        Undo::Split {
            node,
            mesh,
            children,
            meshes,
        } => {
            for &child in children {
                if document.tree.contains(child) {
                    document.tree.remove_subtree(child)?;
                }
            }
            let dropped: std::collections::BTreeSet<_> = meshes.iter().copied().collect();
            document.geometry.retain_keys(&|key| !dropped.contains(&key));
            // The original mesh was never removed, so putting the part back is
            // a matter of pointing at it again.
            let restored = document.tree.get_mut(*node)?;
            restored.kind = NodeKind::Part { mesh: *mesh };
            Ok(())
        }

        Undo::RemovedNodes(removed) => {
            // Removal produced deepest-first order, so restoring in reverse
            // always finds each node's parent already present.
            for (node, index) in removed.iter().rev() {
                let at = if *index == usize::MAX {
                    usize::MAX
                } else {
                    *index
                };
                document.tree.reinsert(node.clone(), at)?;
            }
            Ok(())
        }

        Undo::Name(node, name) => {
            document.tree.get_mut(*node)?.name = name.clone();
            Ok(())
        }
        Undo::Transforms(previous) => {
            for (node, transform) in previous {
                document.tree.get_mut(*node)?.transform = *transform;
            }
            Ok(())
        }
        Undo::Visibility(previous) => {
            for (node, visible) in previous {
                document.tree.get_mut(*node)?.visible = *visible;
            }
            Ok(())
        }
        Undo::Pivots(previous) => {
            for (node, pivot) in previous {
                document.tree.get_mut(*node)?.pivot = *pivot;
            }
            Ok(())
        }
        Undo::Locked(previous) => {
            for (node, locked) in previous {
                document.tree.get_mut(*node)?.locked = *locked;
            }
            Ok(())
        }
        Undo::Parent {
            node,
            parent,
            index,
        } => {
            document.tree.reparent(*node, *parent, Some(*index))?;
            Ok(())
        }

        Undo::Grouped { group, members } => {
            // Put each member back where it was, in its original order, then
            // discard the now-empty group.
            let mut ordered = members.clone();
            ordered.sort_by_key(|&(_, _, index)| index);
            for (node, parent, index) in ordered {
                document.tree.reparent(node, parent, Some(index))?;
            }
            document.tree.remove_subtree(*group)?;
            Ok(())
        }

        Undo::Ungrouped {
            group,
            index,
            members,
        } => {
            document.tree.reinsert((**group).clone(), *index)?;
            let mut ordered = members.clone();
            ordered.sort_by_key(|&(_, index)| index);
            for (node, index) in ordered {
                document.tree.reparent(node, Some(group.id), Some(index))?;
            }
            Ok(())
        }

        Undo::Metadata(node, key, value) => {
            match value {
                Some(value) => {
                    document
                        .tree
                        .get_mut(*node)?
                        .metadata
                        .insert(key.clone(), value.clone());
                }
                None => {
                    document.tree.get_mut(*node)?.metadata.remove(key);
                }
            }
            Ok(())
        }

        Undo::Units(previous, rescaled) => {
            // Put back what was there rather than scaling by the inverse, so a
            // conversion undoes to precisely where it was.
            rescaled.restore(document);
            document.units = *previous;
            Ok(())
        }

        Undo::Materials(previous) => {
            for &(node, material) in previous {
                document.tree.get_mut(node)?.material = material;
            }
            Ok(())
        }

        Undo::Unlinked {
            node,
            previous,
            created,
        } => {
            document.tree.get_mut(*node)?.material = *previous;
            document.materials.remove(*created);
            Ok(())
        }

        Undo::MaterialParameter(material, parameter) => {
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            parameter.param.write(definition.material_mut(), parameter.value);
            Ok(())
        }

        Undo::MaterialType(material, previous) => {
            let definition = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            *definition = (**previous).clone();
            Ok(())
        }

        Undo::ParameterTexture {
            material,
            definition,
            textures,
        } => {
            let slot = document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?;
            *slot = (**definition).clone();
            document.textures = (**textures).clone();
            Ok(())
        }

        Undo::MaterialName(material, name) => {
            document
                .materials
                .get_mut(*material)
                .ok_or(CommandError::NoSuchMaterial(*material))?
                .name = name.clone();
            Ok(())
        }

        Undo::Lighting(settings) => {
            document.lighting = (**settings).clone();
            Ok(())
        }

        Undo::EnvironmentParameter(parameter) => {
            parameter
                .param
                .write(&mut document.environment, &parameter.value);
            Ok(())
        }

        Undo::EnvironmentPins(pins) => {
            document.environment.pins = pins.clone();
            Ok(())
        }

        Undo::Sky(sky) => {
            document.environment.sky = **sky;
            Ok(())
        }

        Undo::Light(node, settings) => {
            let target = document.tree.get_mut(*node)?;
            let NodeKind::Light {
                settings: current, ..
            } = &mut target.kind
            else {
                return Err(CommandError::InconsistentHistory(
                    "undo expected a light and found something else",
                ));
            };
            *current = settings.clone();
            Ok(())
        }

        Undo::MultiMaterials(list, materials) => {
            crate::studio::restore_materials(document, materials);
            document.multi_materials = list.clone();
            Ok(())
        }

        Undo::ModelSets(sets) => {
            document.model_sets = (**sets).clone();
            Ok(())
        }

        Undo::MaterialTemplates(templates) => {
            document.material_templates = templates.clone();
            Ok(())
        }

        Undo::Studios(studios) => {
            document.studios = studios.clone();
            Ok(())
        }

        Undo::StudioApplied(undo) => {
            undo.restore(document);
            Ok(())
        }

        Undo::Configurator(configurator) => {
            document.configurator = (**configurator).clone();
            Ok(())
        }

        Undo::Configured(undo, choices) => {
            undo.restore(document);
            document.configurator.current = choices.clone();
            Ok(())
        }

        Undo::Measurements(measurements) => {
            document.measurements = measurements.clone();
            Ok(())
        }

        Undo::Cutaway(node, settings) => {
            let target = document.tree.get_mut(*node)?;
            let NodeKind::Cutaway { settings: current } = &mut target.kind else {
                return Err(CommandError::InconsistentHistory(
                    "undo expected a cutaway and found something else",
                ));
            };
            *current = settings.clone();
            Ok(())
        }

        Undo::Cameras(cameras) => {
            document.cameras = (**cameras).clone();
            Ok(())
        }

        Undo::ImageStyle(style) => {
            document.image_style = **style;
            Ok(())
        }

        Undo::RenderSettings(settings) => {
            document.render = (**settings).clone();
            Ok(())
        }

        Undo::ColorManagement(selection) => {
            document.ocio = (**selection).clone();
            Ok(())
        }
    }
}

fn check_unlocked(document: &SceneDocument, node: NodeId) -> Result<(), CommandError> {
    document.tree.try_get(node)?;
    if document.tree.is_effectively_locked(node) {
        return Err(CommandError::Locked(node));
    }
    Ok(())
}

/// Insert the mapping-and-texture pair §4.2 describes, replacing whatever
/// drove the parameter before.
///
/// Two nodes rather than one: the coordinates are a separate concern from the
/// lookup, and keeping them separate is what lets a person switch a texture
/// from UV to tri-planar without touching the texture, or share one mapping
/// across several maps on the same material.
fn attach_texture(
    graph: &mut tequila_materials::graph::MaterialGraph,
    param: Param,
    texture: tequila_materials::graph::TextureId,
) {
    use tequila_materials::graph::{Node, NodeKind, Socket, Value};

    detach_texture(graph, param);

    // The fallback is the parameter's current literal, so a texture that fails
    // to load leaves the material looking as it did rather than turning black.
    let fallback = param
        .read(graph.material())
        .map(|value| match value {
            ParamValue::Color(c) => Value::Color(c),
            ParamValue::Scalar(v) => Value::Float(v),
            ParamValue::Flag(b) => Value::Bool(b),
            ParamValue::Metal(_) => Value::Float(0.5),
        })
        .unwrap_or(Value::Float(0.5));

    let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
        mapping: tequila_texture::projection::Mapping::default(),
    }));
    let node = graph.add_node(Node::new(NodeKind::ImageTexture { texture, fallback }));
    let output = graph.output();
    graph.connect(mapping, node, Socket::Input(0));
    graph.connect(node, output, Socket::Param(param));
}

/// Drive `param` from a Volume Map of `volume`: an OpenVDB grid, fitted to
/// the part, wired straight in. It has no mapping — the part's bounding box
/// is where it goes — and no fallback: a grid that fails to load leaves the
/// medium as described.
fn attach_volume(
    graph: &mut tequila_materials::graph::MaterialGraph,
    param: Param,
    volume: tequila_materials::graph::TextureId,
) {
    use tequila_materials::graph::{Node, NodeKind, Socket};
    detach_texture(graph, param);
    let node = graph.add_node(Node::new(NodeKind::VolumeMap { volume }));
    let output = graph.output();
    graph.connect(node, output, Socket::Param(param));
}

/// Rewrite the mapping node feeding a parameter's texture.
///
/// Walks back from the parameter rather than remembering which node it was:
/// the graph is what holds the truth, a mapping node can be shared between
/// several maps, and an index stored elsewhere would be one refactor from
/// pointing at the wrong node.
///
/// Returns false when nothing there takes a mapping, so the caller can refuse
/// rather than silently doing nothing.
fn set_mapping(
    graph: &mut tequila_materials::graph::MaterialGraph,
    param: Param,
    mapping: tequila_texture::projection::Mapping,
) -> bool {
    use tequila_materials::graph::NodeKind;

    let Some(driver) = graph.driver_of(param) else {
        return false;
    };
    // Breadth-first back from the driver. A mapping node is usually one hop
    // away, but a texture may sit behind a chain of utility nodes and the
    // coordinates still come from the same place.
    // Found first, then written. Walking with `&mut` would borrow the graph
    // for the whole traversal and the link list is read on every step.
    let mut to_visit = vec![driver];
    let mut seen = std::collections::BTreeSet::new();
    let mut found = None;
    while let Some(node) = to_visit.pop() {
        if !seen.insert(node) {
            continue;
        }
        match graph.node(node).map(|n| &n.kind) {
            Some(NodeKind::Mapping2d { .. }) => {
                found = Some(node);
                break;
            }
            Some(_) => {
                to_visit.extend(graph.links().iter().filter(|l| l.to == node).map(|l| l.from))
            }
            None => {}
        }
    }
    let Some(node) = found else { return false };
    if let Some(NodeKind::Mapping2d { mapping: current }) = graph.node_mut(node).map(|n| &mut n.kind)
    {
        *current = mapping;
        return true;
    }
    false
}

/// The procedural driving a parameter, if one does.
pub fn procedural_of(
    graph: &tequila_materials::graph::MaterialGraph,
    param: Param,
) -> Option<(tequila_texture::procedural::Procedural, u32)> {
    use tequila_materials::graph::NodeKind;
    let driver = graph.driver_of(param)?;
    // Through the ramp, where a colour parameter has one: the procedural is
    // what the user chose and the ramp is machinery placed on their behalf, so
    // asking "what drives this" has to answer with the pattern either way.
    let mut to_visit = vec![driver];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(node) = to_visit.pop() {
        if !seen.insert(node) {
            continue;
        }
        match graph.node(node).map(|n| &n.kind) {
            Some(NodeKind::Procedural { procedural, seed }) => return Some((**procedural, *seed)),
            Some(_) => {
                to_visit.extend(graph.links().iter().filter(|l| l.to == node).map(|l| l.from))
            }
            None => {}
        }
    }
    None
}

/// How deep the Bump node driving `param` makes its pattern or map, in
/// metres, where one does: what the Material tab's Depth control shows.
pub fn bump_depth_of(graph: &tequila_materials::graph::MaterialGraph, param: Param) -> Option<f32> {
    use tequila_materials::graph::NodeKind;
    match graph.node(graph.driver_of(param)?)?.kind {
        NodeKind::Bump { depth } => Some(depth),
        _ => None,
    }
}

/// Set the depth of the Bump node driving `param`. False where there is
/// none, so the caller can refuse rather than silently doing nothing.
pub fn set_bump_depth(graph: &mut tequila_materials::graph::MaterialGraph, param: Param, depth: f32) -> bool {
    use tequila_materials::graph::NodeKind;
    let Some(driver) = graph.driver_of(param) else {
        return false;
    };
    match graph.node_mut(driver).map(|n| &mut n.kind) {
        Some(NodeKind::Bump { depth: current }) if depth.is_finite() && depth >= 0.0 => {
            *current = depth;
            true
        }
        _ => false,
    }
}

/// Read the mapping feeding a parameter's texture, if there is one.
pub fn mapping_of(
    graph: &tequila_materials::graph::MaterialGraph,
    param: Param,
) -> Option<tequila_texture::projection::Mapping> {
    use tequila_materials::graph::NodeKind;

    let driver = graph.driver_of(param)?;
    let mut to_visit = vec![driver];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(node) = to_visit.pop() {
        if !seen.insert(node) {
            continue;
        }
        match graph.node(node).map(|n| &n.kind) {
            Some(NodeKind::Mapping2d { mapping }) => return Some(*mapping),
            Some(_) => {
                to_visit.extend(graph.links().iter().filter(|l| l.to == node).map(|l| l.from))
            }
            None => {}
        }
    }
    None
}

/// Whether a sky can be evaluated without producing a non-finite texel.
///
/// Same reasoning as the pins: the sky becomes the environment image, and one
/// NaN in that image poisons the CDF built from it.
fn sky_is_valid(sky: &tequila_render_core::sky::Sky) -> bool {
    sky.sun_direction.is_finite()
        && sky.sun_direction.length() > 1e-6
        && sky.intensity.is_finite()
        && sky.intensity >= 0.0
        && sky.turbidity.is_finite()
        && sky.turbidity >= 1.0
        && sky.ground_albedo.is_finite()
        && (0.0..=1.0).contains(&sky.ground_albedo)
        && sky.sun_intensity.is_finite()
        && sky.sun_intensity >= 0.0
}

/// Whether a pin can be composited without producing a non-finite texel.
///
/// Checked at the command rather than clamped in the compositor: a NaN that
/// reaches the environment image poisons the CDF, and a CDF with a NaN in it
/// fails as a light that is sampled always or never rather than as anything
/// that looks like bad input.
fn pin_is_valid(pin: &tequila_render_core::pins::Pin) -> bool {
    pin.direction.is_finite()
        && pin.direction.length() > 1e-6
        && pin.angular_size.x.is_finite()
        && pin.angular_size.x > 0.0
        && pin.angular_size.y.is_finite()
        && pin.angular_size.y > 0.0
        && pin.roll.is_finite()
        && pin.softness.is_finite()
        && (0.0..=1.0).contains(&pin.softness)
        && pin.intensity.is_finite()
        && pin.intensity >= 0.0
        && pin.color.r.is_finite()
        && pin.color.g.is_finite()
        && pin.color.b.is_finite()
        && match &pin.kind {
            tequila_render_core::pins::PinKind::Gradient { stops, .. } => stops.iter().all(|stop| {
                stop.t.is_finite() && stop.color.r.is_finite() && stop.color.g.is_finite() && stop.color.b.is_finite()
            }),
            _ => true,
        }
}

/// Drive a parameter from a procedural, replacing whatever drove it before.
///
/// One node, not two: a procedural is a function of the shading point and needs
/// no coordinates fed to it. A mapping node can be added later to move it out
/// of object space, and its absence is what §6's "object space by default"
/// means in the graph.
/// Change the pattern driving `param` and nothing around it: the colour ramp
/// it is read through, the range it is mapped to and the depth of its bump
/// all stay. Where nothing drives `param` yet, it is attached afresh.
///
/// What a pattern's controls and the tex menu both go through. They went
/// through [`attach_procedural`], which tears the chain down and builds a
/// new one, so every nudge of a frequency slider reset the colours to grey.
pub fn retune_procedural(
    graph: &mut tequila_materials::graph::MaterialGraph,
    param: Param,
    procedural: tequila_texture::procedural::Procedural,
    seed: u32,
) {
    use tequila_materials::graph::NodeKind;
    let existing = graph.driver_of(param).and_then(|driver| {
        let mut to_visit = vec![driver];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(node) = to_visit.pop() {
            if !seen.insert(node) {
                continue;
            }
            match graph.node(node).map(|n| &n.kind) {
                Some(NodeKind::Procedural { .. }) => return Some(node),
                Some(_) => to_visit.extend(graph.links().iter().filter(|l| l.to == node).map(|l| l.from)),
                None => {}
            }
        }
        None
    });
    match existing.and_then(|node| graph.node_mut(node)) {
        Some(node) => {
            node.kind = NodeKind::Procedural { procedural: Box::new(procedural), seed };
        }
        None => attach_procedural(graph, param, procedural, seed),
    }
}

/// Drive `param` with a procedural, with a colour ramp in between for a
/// colour and a Bump node for a bump: what `material.set_procedural` does,
/// and what a library preset's patterns are built with.
pub fn attach_procedural(
    graph: &mut tequila_materials::graph::MaterialGraph,
    param: Param,
    procedural: tequila_texture::procedural::Procedural,
    seed: u32,
) {
    use tequila_materials::graph::{Node, NodeKind, Socket};
    use tequila_materials::material_type::ParamUi;

    detach_texture(graph, param);
    let node = graph.add_node(Node::new(NodeKind::Procedural {
        procedural: Box::new(procedural),
        seed,
    }));
    let output = graph.output();

    // A procedural produces one number. That is the whole answer for a
    // roughness and none of it for a colour, so a colour parameter gets a ramp
    // between the two — which is also what makes wood wood rather than a grey
    // field, and the reason the ramp node exists.
    //
    // Inserted here rather than left to the user: attaching a pattern to a
    // colour and getting greyscale would look like the pattern was broken,
    // and the fix — "add a node you have not been told about" — is not
    // discoverable from that.
    if param.ui() == ParamUi::Color {
        let ramp = graph.add_node(Node::new(NodeKind::Gradient {
            gradient: Box::new(tequila_texture::gradient::Gradient::default()),
        }));
        graph.connect(node, ramp, Socket::Input(0));
        graph.connect(ramp, output, Socket::Param(param));
        return;
    }
    graph.connect(node, output, Socket::Param(param));
    through_bump_node(graph, param);
}

/// On the Bump parameter, put a Bump node between the socket and whatever
/// drives it, so a 0-to-1 pattern or height map becomes a bump of a sensible
/// depth rather than one a whole scene unit deep. Nothing on any other
/// parameter, or when the driver already is one.
///
/// Inserted for the same reason a colour parameter gets a ramp: attaching a
/// pattern to Bump and getting a surface torn to shreds would look like a
/// broken pattern, and the fix is a node nobody has been told about.
fn through_bump_node(graph: &mut tequila_materials::graph::MaterialGraph, param: Param) {
    use tequila_materials::graph::{Node, NodeKind, Socket, DEFAULT_BUMP_DEPTH};

    if param != Param::Bump && param != Param::Displacement {
        return;
    }
    let Some(driver) = graph.driver_of(param) else {
        return;
    };
    if matches!(graph.node(driver).map(|n| &n.kind), Some(NodeKind::Bump { .. })) {
        return;
    }
    let bump = graph.add_node(Node::new(NodeKind::Bump {
        depth: DEFAULT_BUMP_DEPTH,
    }));
    let output = graph.output();
    graph.connect(driver, bump, Socket::Input(0));
    graph.connect(bump, output, Socket::Param(param));
}

/// The ramp a parameter's procedural feeds, if there is one.
pub fn gradient_of(
    graph: &tequila_materials::graph::MaterialGraph,
    param: Param,
) -> Option<tequila_texture::gradient::Gradient> {
    use tequila_materials::graph::NodeKind;
    let driver = graph.driver_of(param)?;
    match graph.node(driver).map(|n| &n.kind) {
        Some(NodeKind::Gradient { gradient }) => Some((**gradient).clone()),
        _ => None,
    }
}

/// Replace the ramp a parameter's procedural feeds.
pub fn set_gradient(
    graph: &mut tequila_materials::graph::MaterialGraph,
    param: Param,
    gradient: tequila_texture::gradient::Gradient,
) -> bool {
    use tequila_materials::graph::NodeKind;
    let Some(driver) = graph.driver_of(param) else {
        return false;
    };
    if let Some(NodeKind::Gradient { gradient: current }) =
        graph.node_mut(driver).map(|n| &mut n.kind)
    {
        *current = Box::new(gradient);
        return true;
    }
    false
}

/// Remove whatever drives a parameter, and any nodes left feeding nothing.
fn detach_texture(graph: &mut tequila_materials::graph::MaterialGraph, param: Param) {
    use tequila_materials::graph::Socket;

    let Some(driver) = graph.driver_of(param) else {
        return;
    };
    let output = graph.output();
    graph.disconnect(output, Socket::Param(param));

    // Sweep up what the removal orphaned. A mapping node left behind with
    // nothing downstream would show in the graph editor as a node the user did
    // not add and cannot explain.
    let mut doomed = vec![driver];
    while let Some(id) = doomed.pop() {
        let still_used = graph.links().iter().any(|l| l.from == id);
        if still_used {
            continue;
        }
        let feeders: Vec<_> = graph
            .links()
            .iter()
            .filter(|l| l.to == id)
            .map(|l| l.from)
            .collect();
        graph.remove_node(id);
        doomed.extend(feeders);
    }
}

/// The material's current value for a parameter, as the edit that would
/// restore it. This is what makes undo of a slider drag exact rather than
/// approximately-back-to-where-it-was.
fn read_parameter(
    material: &tequila_render_core::material::Material,
    param: Param,
) -> Option<MaterialParameter> {
    Some(MaterialParameter::new(param, param.read(material)?))
}

/// Mesh and material keys an import needs, allocated against a document.
///
/// Importers build their payload against these so the command carries concrete
/// keys, which is what makes an import undoable and redoable to the same IDs.
/// Reserve texture ids for an import, alongside [`reserve_keys`].
///
/// Separate because most importers produce none, and threading a fourth count
/// through every call site to say zero would be noise.
pub fn reserve_texture_ids(
    document: &SceneDocument,
    count: usize,
) -> Vec<tequila_materials::graph::TextureId> {
    let base = document.textures.next_id();
    (0..count)
        .map(|i| tequila_materials::graph::TextureId(base + i as u32))
        .collect()
}

pub fn reserve_keys(
    document: &SceneDocument,
    meshes: usize,
    materials: usize,
    nodes: usize,
) -> (Vec<MeshKey>, Vec<MaterialKey>, Vec<NodeId>) {
    let mesh_base = document.geometry.next_key();
    let material_base = document.materials.next_key();
    let node_base = document.tree.next_id();
    (
        (0..meshes).map(|i| MeshKey(mesh_base + i as u64)).collect(),
        (0..materials)
            .map(|i| MaterialKey(material_base + i as u64))
            .collect(),
        (0..nodes).map(|i| NodeId(node_base + i as u64)).collect(),
    )
}

/// Metadata keys used across a document, for the metadata panel's column list.
pub fn metadata_keys(document: &SceneDocument) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for node in document.tree.iter() {
        for key in node.metadata.keys() {
            *counts.entry(key.clone()).or_insert(0) += 1;
        }
    }
    counts
}

/// §5.3's mapping, which decides how a texture lands on a surface.
#[cfg(test)]
mod retune_tests {
    use super::*;
    use tequila_materials::graph::{MaterialGraph, NodeKind};
    use tequila_materials::material_type::MaterialType;
    use tequila_texture::gradient::{Gradient, Stop};
    use tequila_texture::procedural::Procedural;

    fn wood(frequency: f32) -> Procedural {
        match Procedural::catalog().into_iter().find(|p| p.key() == "wood").expect("wood") {
            Procedural::Wood { axis, distortion, grain, fractal, .. } => {
                Procedural::Wood { axis, frequency, distortion, grain, fractal }
            }
            _ => unreachable!(),
        }
    }

    /// Changing a pattern's own controls — its frequency, say, from the
    /// Material tab — changes the pattern and nothing else. It went through
    /// the same path as attaching a new pattern, which tears down the chain
    /// and builds a fresh one: every nudge of a slider threw away the
    /// colours the pattern was read through, the depth of its bump and the
    /// range it was mapped to.
    #[test]
    fn retuning_a_pattern_keeps_its_colours_depth_and_range() {
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        attach_procedural(&mut graph, Param::BaseColor, wood(80.0), 3);
        let colours = Gradient::new(vec![
            Stop { at: 0.0, color: tequila_core::spectrum::spectrum(0.6, 0.4, 0.2) },
            Stop { at: 1.0, color: tequila_core::spectrum::spectrum(0.2, 0.1, 0.05) },
        ]);
        assert!(set_gradient(&mut graph, Param::BaseColor, colours.clone()));
        attach_procedural(&mut graph, Param::Bump, wood(80.0), 3);
        let bump = graph.driver_of(Param::Bump).unwrap();
        if let Some(NodeKind::Bump { depth }) = graph.node_mut(bump).map(|n| &mut n.kind) {
            *depth = 0.0001;
        }

        retune_procedural(&mut graph, Param::BaseColor, wood(120.0), 3);
        retune_procedural(&mut graph, Param::Bump, wood(120.0), 3);

        assert_eq!(procedural_of(&graph, Param::BaseColor).map(|(p, _)| p), Some(wood(120.0)));
        assert_eq!(gradient_of(&graph, Param::BaseColor), Some(colours), "the colours were reset");
        let bump = graph.driver_of(Param::Bump).unwrap();
        assert!(
            matches!(graph.node(bump).map(|n| &n.kind), Some(NodeKind::Bump { depth }) if (*depth - 0.0001).abs() < 1e-9),
            "the bump depth was reset"
        );
        graph.compile().expect("still compiles");
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::*;
    use tequila_texture::mip::Wrap;
    use tequila_texture::projection::{Mapping, Projection};

    fn material_with_a_texture() -> (SceneDocument, CommandBus, MaterialKey) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let key = document
            .materials
            .insert(crate::material::MaterialDefinition::new(
                "Painted",
                tequila_materials::material_type::MaterialType::Plastic,
                Default::default(),
            ));
        bus.apply(
            &mut document,
            Command::SetParameterTexture {
                material: key,
                param: Param::BaseColor,
                source: Some(crate::textures::TextureSource::new(
                    "colour.png",
                    tequila_texture::texture::MapRole::Color,
                )),
            },
        )
        .expect("attach");
        (document, bus, key)
    }

    /// Attaching a texture creates the mapping node, and it starts at the
    /// defaults — so there is always something to edit rather than a first
    /// edit having to create it.
    #[test]
    fn attaching_a_texture_creates_a_mapping() {
        let (document, _, key) = material_with_a_texture();
        let graph = &document.materials.get(key).unwrap().graph;
        let mapping = mapping_of(graph, Param::BaseColor).expect("a mapping exists");
        assert_eq!(mapping, Mapping::default());
    }

    /// And it can be changed. This is the part that did not exist: the node
    /// was created, evaluated on every texture lookup, and unreachable — so a
    /// texture could be attached and then never scaled, rotated, tiled, or
    /// projected any way but UV.
    #[test]
    fn the_mapping_can_be_changed() {
        let (mut document, mut bus, key) = material_with_a_texture();
        let wanted = Mapping {
            projection: Projection::TriPlanar,
            scale: tequila_core::math::vec3(4.0, 4.0, 4.0),
            rotation: 0.25,
            wrap_u: Wrap::Mirror,
            wrap_v: Wrap::Mirror,
            real_world_units: true,
            ..Mapping::default()
        };
        bus.apply(
            &mut document,
            Command::SetTextureMapping {
                material: key,
                param: Param::BaseColor,
                mapping: Box::new(wanted),
            },
        )
        .expect("the mapping applies");

        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(mapping_of(graph, Param::BaseColor), Some(wanted));
    }

    /// Undo puts it back, so a projection tried and rejected costs one Ctrl+Z.
    #[test]
    fn the_mapping_undoes() {
        let (mut document, mut bus, key) = material_with_a_texture();
        bus.apply(
            &mut document,
            Command::SetTextureMapping {
                material: key,
                param: Param::BaseColor,
                mapping: Box::new(Mapping {
                    projection: Projection::Spherical,
                    ..Mapping::default()
                }),
            },
        )
        .expect("apply");
        bus.undo(&mut document).expect("undo");

        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(
            mapping_of(graph, Param::BaseColor).map(|m| m.projection),
            Some(Projection::Uv)
        );
    }

    /// A parameter with no texture has no mapping, and asking to set one is
    /// refused rather than quietly doing nothing — a command that reports
    /// success without acting is how a UI comes to show a control that does
    /// not work.
    #[test]
    fn a_parameter_without_a_texture_refuses() {
        let (mut document, mut bus, key) = material_with_a_texture();
        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(mapping_of(graph, Param::SpecularRoughness), None);

        assert!(bus
            .apply(
                &mut document,
                Command::SetTextureMapping {
                    material: key,
                    param: Param::SpecularRoughness,
                    mapping: Box::new(Mapping::default()),
                },
            )
            .is_err());
    }

    /// Detaching the texture takes the mapping with it, rather than leaving a
    /// node feeding nothing for the next attach to inherit.
    #[test]
    fn detaching_removes_the_mapping() {
        let (mut document, mut bus, key) = material_with_a_texture();
        bus.apply(
            &mut document,
            Command::SetParameterTexture {
                material: key,
                param: Param::BaseColor,
                source: None,
            },
        )
        .expect("detach");
        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(mapping_of(graph, Param::BaseColor), None);
    }
}


/// §3's two conversion modes, which answer opposite questions.
///
/// Worth pinning precisely because the two are destructive in opposite
/// directions: pick the wrong one and either the model is the wrong size or
/// it is shown in the wrong unit, and neither is obvious until much later.
/// Lengths are stored in metres either way, because that is what the
/// renderer reads them as.
#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::lighting::{Intensity, LightShape};
    use crate::measure::{Anchor, Measurement};
    use crate::units::ConversionMode;
    use tequila_core::math::vec3;
    use tequila_core::units::LengthUnit;
    use tequila_render_core::camera::Camera;

    /// A part 121 across, as an STL with no unit comes in, in a scene with
    /// no unit: a light over it, a camera on it, a fixed-size floor and a
    /// dimension from the part to a point in the world.
    fn hangboard() -> (SceneDocument, CommandBus) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 1);
        let mesh = crate::geometry::Mesh::new(
            vec![vec3(0.0, 0.0, 0.0), vec3(121.0, 0.0, 0.0), vec3(121.0, 25.0, 110.0)],
            vec![0, 1, 2],
        );
        let mut node = SceneNode::new(ids[0], "Hangboard", NodeKind::Part { mesh: meshes[0] });
        node.material = Some(materials[0]);
        let mut definition = MaterialDefinition::plastic("Cork");
        // A label printed 50 units wide, and an image laid on at a real size
        // too - the first placed on the part, the second its material.
        let mut label = tequila_materials::decal::Decal::new("Logo", tequila_materials::graph::TextureId(0));
        label.mapping.real_world_units = true;
        label.mapping.scale = vec3(50.0, 20.0, 1.0);
        definition.graph.decals.push(label);
        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: vec![(materials[0], definition)],
                    nodes: vec![node],
                    roots: vec![ids[0]],
                }),
            },
        )
        .expect("import");
        bus.apply(
            &mut document,
            Command::AddLight {
                parent: None,
                name: "Key".into(),
                settings: Box::new(LightSettings {
                    shape: LightShape::Area { width: 40.0, height: 30.0, disc: false, two_sided: false },
                    intensity: Intensity::Lumens(1500.0),
                    ..Default::default()
                }),
            },
        )
        .expect("light");
        let light = light_node(&document);
        bus.apply(&mut document, Command::set_transform(light, Transform::translation(vec3(60.0, 200.0, 55.0))))
            .expect("place the light");
        let mut camera = Camera::look_at(vec3(300.0, 200.0, 300.0), vec3(60.0, 12.0, 55.0), Vec3::Y);
        camera.projection = tequila_render_core::camera::Projection::Orthographic { scale: 150.0 };
        bus.apply(
            &mut document,
            Command::SetCamera {
                index: 0,
                camera: Box::new(SavedCamera { name: "Camera 1".into(), camera, target: vec3(60.0, 12.0, 55.0) }),
            },
        )
        .expect("camera");
        bus.apply(
            &mut document,
            Command::SetEnvironmentParameter {
                parameter: EnvironmentParameter::new(EnvParam::GroundSize, EnvValue::Scalar(400.0)),
            },
        )
        .expect("ground");
        bus.apply(
            &mut document,
            Command::SetMeasurements {
                measurements: vec![Measurement::new(
                    Anchor { node: Some(ids[0]), point: vec3(121.0, 0.0, 0.0) },
                    Anchor { node: None, point: vec3(121.0, 0.0, 200.0) },
                )],
            },
        )
        .expect("dimension");
        (document, bus)
    }

    fn light_node(document: &SceneDocument) -> NodeId {
        document.tree.iter().find(|n| n.kind.light().is_some()).expect("a light").id
    }

    fn width(document: &SceneDocument) -> f32 {
        crate::snapshot::visible_bounds(document).diagonal().x
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() <= 1e-5 * a.abs().max(b.abs()).max(1e-3)
    }

    /// "The model is the right size; show it in millimetres." Nothing in
    /// the scene moves: lengths are metres either way, and only the unit
    /// they are shown in changes.
    #[test]
    fn keeping_the_scene_size_changes_only_the_unit() {
        let (mut document, mut bus) = hangboard();
        let before = document.clone();
        bus.apply(
            &mut document,
            Command::SetUnits { unit: LengthUnit::Millimetre, mode: ConversionMode::KeepSceneSize },
        )
        .expect("convert");
        assert_eq!(document.units.unit, LengthUnit::Millimetre);
        let mut relabelled = before;
        relabelled.units.unit = LengthUnit::Millimetre;
        assert_eq!(document.difference_from(&relabelled), None);
        assert!(document == relabelled);
    }

    /// "The model is the wrong size: its numbers were millimetres." It
    /// comes out 121 mm across, and still reads 121 - and the shot comes
    /// with it: the light, its size and power, the camera, the floor, the
    /// dimension's end in the world and the label on the part. The
    /// material does not, because cork's grain is the same size on any
    /// part.
    #[test]
    fn scaling_the_scene_reads_the_numbers_in_the_new_unit() {
        let (mut document, mut bus) = hangboard();
        let before = document.clone();
        bus.apply(
            &mut document,
            Command::SetUnits { unit: LengthUnit::Millimetre, mode: ConversionMode::ScaleScene },
        )
        .expect("convert");

        assert_eq!(document.units.unit, LengthUnit::Millimetre);
        assert!(close(width(&document), 0.121), "the part came out {} m across", width(&document));

        let light = light_node(&document);
        let world = document.tree.world_transform(light);
        assert!((world.point(Vec3::ZERO) - vec3(0.06, 0.2, 0.055)).length() < 1e-6);
        let render = document.tree.get(light).unwrap().kind.light().unwrap().to_render_light(&world).unwrap();
        let tequila_render_core::light::LightKind::Area(panel) = render.kind else { panic!("an area light") };
        assert!(close(panel.area(), 0.04 * 0.03), "the panel is {} m² now", panel.area());
        // A thousandth the distance and a millionth the area: a millionth
        // the lumens leaves the part lit as it was.
        let Intensity::Lumens(lumens) = document.tree.get(light).unwrap().kind.light().unwrap().intensity else {
            panic!("still in lumens")
        };
        assert!(close(lumens, 1500.0e-6), "{lumens} lm");

        let saved = document.cameras.active();
        assert!((saved.camera.position() - vec3(0.3, 0.2, 0.3)).length() < 1e-6);
        assert!((saved.target - vec3(0.06, 0.012, 0.055)).length() < 1e-6);
        let distance = (vec3(0.3, 0.2, 0.3) - vec3(0.06, 0.012, 0.055)).length();
        assert!(close(saved.camera.focus_distance, distance), "focus at {}", saved.camera.focus_distance);
        let tequila_render_core::camera::Projection::Orthographic { scale } = saved.camera.projection else {
            panic!("still orthographic")
        };
        assert!(close(scale, 0.15));
        // The camera still looks the way it did, unscaled.
        assert!((saved.camera.forward() - before.cameras.active().camera.forward()).length() < 1e-6);

        assert!(close(document.environment.ground.size, 0.4));
        let measured = document.measurements[0].length(&document).unwrap();
        assert!(close(measured, 0.2), "the dimension reads {measured} m");

        let (_, material) = document.materials.iter().next().unwrap();
        assert!((material.graph.decals[0].mapping.scale - vec3(0.05, 0.02, 0.001)).abs().max_component() < 1e-7);
        let (_, was) = before.materials.iter().next().unwrap();
        assert_eq!(material.material(), was.material(), "the material itself must keep its real sizes");
    }

    /// A panel stated in nits emits per area, and grows with the scene: it
    /// already lights the scaled scene as it lit the original, and scaling
    /// its number as well would darken it a millionfold.
    #[test]
    fn a_panel_in_nits_keeps_its_luminance() {
        let (mut document, mut bus) = hangboard();
        let light = light_node(&document);
        let mut settings = document.tree.get(light).unwrap().kind.light().unwrap().clone();
        settings.intensity = Intensity::Nits(800.0);
        bus.apply(&mut document, Command::SetLight { node: light, settings: Box::new(settings) }).expect("nits");
        bus.apply(
            &mut document,
            Command::SetUnits { unit: LengthUnit::Millimetre, mode: ConversionMode::ScaleScene },
        )
        .expect("convert");
        let intensity = document.tree.get(light).unwrap().kind.light().unwrap().intensity;
        assert_eq!(intensity, Intensity::Nits(800.0));
    }

    /// The two are genuinely different, which is the whole reason the user is
    /// asked.
    #[test]
    fn the_two_modes_disagree() {
        let (mut a, mut bus_a) = hangboard();
        let (mut b, mut bus_b) = hangboard();
        for (document, bus, mode) in [
            (&mut a, &mut bus_a, ConversionMode::KeepSceneSize),
            (&mut b, &mut bus_b, ConversionMode::ScaleScene),
        ] {
            bus.apply(document, Command::SetUnits { unit: LengthUnit::Millimetre, mode }).expect("convert");
        }
        assert!(close(width(&a), 121.0) && close(width(&b), 0.121), "{} and {}", width(&a), width(&b));
    }

    /// And either is undoable to exactly where it was, since choosing the
    /// wrong one is the mistake the question exists to prevent and the
    /// person who makes it needs a way back. Redo makes it again.
    #[test]
    fn a_unit_change_undoes_exactly() {
        for mode in ConversionMode::ALL {
            let (mut document, mut bus) = hangboard();
            let before = document.clone();
            bus.apply(&mut document, Command::SetUnits { unit: LengthUnit::Millimetre, mode }).expect("convert");
            let after = document.clone();
            bus.undo(&mut document).expect("undo");
            assert_eq!(document.difference_from(&before), None, "{mode:?}");
            assert!(document == before, "{mode:?}");
            bus.redo(&mut document).expect("redo");
            assert_eq!(document.difference_from(&after), None, "{mode:?}");
            assert!(document == after, "{mode:?}");
        }
    }

    /// Scaled one way and back the other, the scene lands where it was, to
    /// rounding: repeated unit changes do not drift it.
    #[test]
    fn scaling_there_and_back_comes_home() {
        let (mut document, mut bus) = hangboard();
        let before = width(&document);
        for unit in [LengthUnit::Millimetre, LengthUnit::Inch, LengthUnit::Metre] {
            bus.apply(&mut document, Command::SetUnits { unit, mode: ConversionMode::ScaleScene }).expect("convert");
        }
        assert!(close(width(&document), before), "{} after, {before} before", width(&document));
    }
}

/// Marble and Wood through the command bus: attach, retune, undo.
#[cfg(test)]
mod marble_and_wood_tests {
    use super::*;
    use tequila_texture::noise::Fractal;
    use tequila_texture::procedural::{Axis, Procedural};

    fn plastic() -> (SceneDocument, CommandBus, MaterialKey) {
        let mut document = SceneDocument::new();
        let bus = CommandBus::new();
        let key = document
            .materials
            .insert(crate::material::MaterialDefinition::new(
                "Panel",
                tequila_materials::material_type::MaterialType::Plastic,
                Default::default(),
            ));
        (document, bus, key)
    }

    fn wood(frequency: f32) -> Procedural {
        Procedural::Wood {
            axis: Axis::Z,
            frequency,
            distortion: 0.4,
            grain: 0.3,
            fractal: Fractal::default(),
        }
    }

    /// Wood on base colour gets the ramp inserted for it, like any other
    /// procedural — the path it shares with the rest is the path it has to
    /// keep working on.
    #[test]
    fn wood_on_a_colour_gets_a_ramp() {
        let (mut document, mut bus, key) = plastic();
        bus.apply(
            &mut document,
            Command::SetParameterProcedural {
                material: key,
                param: Param::BaseColor,
                procedural: Some(Box::new(wood(9.0))),
                seed: 5,
            },
        )
        .expect("attach");

        let graph = &document.materials.get(key).unwrap().graph;
        assert!(
            gradient_of(graph, Param::BaseColor).is_some(),
            "wood on a colour got no ramp, so it would render as a grey field"
        );
        assert_eq!(
            procedural_of(graph, Param::BaseColor).map(|(p, _)| p),
            Some(wood(9.0)),
            "the ramp hid the wood behind it"
        );
    }

    /// Retuning is one undo step, and undo restores the previous settings
    /// rather than detaching the node — a slider tried and rejected must cost
    /// one Ctrl+Z, not the whole material.
    #[test]
    fn retuning_marble_undoes_to_the_previous_settings() {
        let (mut document, mut bus, key) = plastic();
        let coarse = Procedural::Marble {
            axis: Axis::X,
            frequency: 2.0,
            distortion: 1.0,
            fractal: Fractal::default(),
        };
        let fine = Procedural::Marble {
            axis: Axis::X,
            frequency: 18.0,
            distortion: 6.0,
            fractal: Fractal::default(),
        };

        for procedural in [coarse, fine] {
            bus.apply(
                &mut document,
                Command::SetParameterProcedural {
                    material: key,
                    param: Param::SpecularRoughness,
                    procedural: Some(Box::new(procedural)),
                    seed: 5,
                },
            )
            .expect("apply");
        }

        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(
            procedural_of(graph, Param::SpecularRoughness).map(|(p, _)| p),
            Some(fine)
        );

        bus.undo(&mut document).expect("undo");
        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(
            procedural_of(graph, Param::SpecularRoughness).map(|(p, _)| p),
            Some(coarse),
            "undo did not return the earlier settings"
        );
    }

    /// And detaching leaves the literal, with no ramp or procedural left
    /// feeding nothing.
    #[test]
    fn detaching_wood_clears_the_ramp_with_it() {
        let (mut document, mut bus, key) = plastic();
        bus.apply(
            &mut document,
            Command::SetParameterProcedural {
                material: key,
                param: Param::BaseColor,
                procedural: Some(Box::new(wood(9.0))),
                seed: 5,
            },
        )
        .expect("attach");
        bus.apply(
            &mut document,
            Command::SetParameterProcedural {
                material: key,
                param: Param::BaseColor,
                procedural: None,
                seed: 5,
            },
        )
        .expect("detach");

        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(procedural_of(graph, Param::BaseColor), None);
        assert_eq!(
            gradient_of(graph, Param::BaseColor),
            None,
            "the ramp outlived the procedural it was reading"
        );
    }
}

#[cfg(test)]
mod label_placement_tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};
    use tequila_texture::projection::Projection;

    /// A part and a material, with UV coordinates or without them.
    ///
    /// A plate four units across and a tenth thick, so its widest face is
    /// unambiguous and the fit has an obvious right answer.
    fn plate(with_uvs: bool) -> (SceneDocument, CommandBus, MaterialKey) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let (meshes, _, ids) = reserve_keys(&document, 1, 0, 1);

        let positions = vec![
            vec3(-2.0, 0.05, -2.0),
            vec3(2.0, 0.05, -2.0),
            vec3(2.0, 0.05, 2.0),
            vec3(-2.0, 0.05, 2.0),
        ];
        let mut mesh = crate::geometry::Mesh::new(positions, vec![0, 1, 2, 0, 2, 3]);
        if with_uvs {
            mesh.uv_sets = vec![vec![
                vec2(0.0, 0.0),
                vec2(1.0, 0.0),
                vec2(1.0, 1.0),
                vec2(0.0, 1.0),
            ]];
        }

        let material = document
            .materials
            .insert(crate::material::MaterialDefinition::new(
                "Shell",
                tequila_materials::material_type::MaterialType::Plastic,
                Default::default(),
            ));
        let mut node = SceneNode::new(ids[0], "Plate", NodeKind::Part { mesh: meshes[0] });
        node.material = Some(material);

        bus.apply(
            &mut document,
            Command::Import {
                payload: Box::new(ImportPayload {
                    meshes: vec![(meshes[0], mesh)],
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes: vec![node],
                    roots: vec![ids[0]],
                }),
            },
        )
        .expect("import");
        (document, bus, material)
    }

    fn add_label(document: &mut SceneDocument, bus: &mut CommandBus, material: MaterialKey) {
        bus.apply(
            document,
            Command::AddDecal {
                material,
                source: crate::textures::TextureSource::new(
                    "logo.png",
                    tequila_texture::texture::MapRole::Color,
                ),
                name: "Logo".into(),
            },
        )
        .expect("the label is added");
    }

    fn only_label(document: &SceneDocument, material: MaterialKey) -> tequila_materials::decal::Decal {
        let decals = &document.materials.get(material).unwrap().graph.decals;
        assert_eq!(decals.len(), 1);
        decals[0].clone()
    }

    /// The case the Labels section could not handle: a part with no UV
    /// coordinates.
    ///
    /// UV placement on a mesh with no UV set addresses nothing — every point
    /// reads the same coordinate — so the label would be added, listed, and
    /// invisible. STL stores no UVs at all and most exported OBJ meshes have
    /// none either, which makes this the ordinary case for the formats this
    /// project imports rather than an edge one.
    #[test]
    fn a_label_on_a_part_without_uvs_is_projected_onto_it() {
        let (mut document, mut bus, material) = plate(false);
        add_label(&mut document, &mut bus, material);

        let label = only_label(&document, material);
        assert_eq!(
            label.mapping.projection,
            Projection::Planar,
            "the label was left on UV placement, which this part cannot use"
        );
        // Down the thin axis, onto the face.
        assert_eq!(label.mapping.axis, tequila_texture::projection::Axis::Y);
        assert!(label.is_valid(), "the fitted mapping is not a valid decal");

        // And it lands on the part: the centre of the plate is the centre of
        // the artwork.
        let point = tequila_texture::projection::SurfacePoint {
            position: vec3(0.0, 0.05, 0.0),
            normal: vec3(0.0, 1.0, 0.0),
            uv: vec2(0.0, 0.0),
            footprint: 0.0,
            scale: 1.0,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: tequila_core::spectrum::Spectrum::ONE,
            world: Default::default(),
        };
        let coords = label.mapping.project(&point)[0].coords;
        assert!(
            (coords.u - 0.5).abs() < 1e-4 && (coords.v - 0.5).abs() < 1e-4,
            "the middle of the part projected to ({}, {}), so the artwork is \
             not on it",
            coords.u,
            coords.v
        );
    }

    /// A part that *does* have UV coordinates keeps them.
    ///
    /// The artist authored that layout and a projection would override it;
    /// this is also what keeps every scene made before the fit existed opening
    /// exactly as it did.
    #[test]
    fn a_label_on_a_uv_mapped_part_keeps_its_uv_placement() {
        let (mut document, mut bus, material) = plate(true);
        add_label(&mut document, &mut bus, material);
        assert_eq!(
            only_label(&document, material).mapping,
            tequila_texture::projection::Mapping::default(),
            "a UV-mapped part had its authored layout overridden"
        );
    }

    /// Undo removes the label, fit and all.
    #[test]
    fn undoing_a_fitted_label_removes_it() {
        let (mut document, mut bus, material) = plate(false);
        add_label(&mut document, &mut bus, material);
        bus.undo(&mut document).expect("undo");
        assert!(
            document
                .materials
                .get(material)
                .unwrap()
                .graph
                .decals
                .is_empty(),
            "the label survived undo"
        );
        bus.redo(&mut document).expect("redo");
        assert_eq!(
            only_label(&document, material).mapping.projection,
            Projection::Planar,
            "redo brought the label back without its placement"
        );
    }
}

#[cfg(test)]
mod pin_tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};
    use tequila_render_core::pins::{Pin, PinShape};

    fn key_light() -> Pin {
        Pin {
            name: "Key".into(),
            shape: PinShape::Rectangle,
            direction: vec3(0.4, 0.8, -0.45),
            angular_size: vec2(0.7, 0.35),
            roll: 0.3,
            softness: 0.6,
            intensity: 12.0,
            color: tequila_core::spectrum::Spectrum {
                r: 1.0,
                g: 0.95,
                b: 0.88,
            },
            // Not the default, so a field dropped on the way through a
            // command or a file cannot pass by coming back as itself.
            show_in_background: true,
            blend: tequila_render_core::pins::PinBlend::Over,
            ..Default::default()
        }
    }

    /// A pin edit is one undo step and puts the stack back exactly.
    #[test]
    fn setting_pins_undoes() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        assert!(document.environment.pins.is_empty());

        bus.apply(
            &mut document,
            Command::SetEnvironmentPins {
                pins: vec![key_light()],
            },
        )
        .expect("pins are set");
        assert_eq!(document.environment.pins, vec![key_light()]);

        bus.undo(&mut document).expect("undo");
        assert!(
            document.environment.pins.is_empty(),
            "undo left the pin behind"
        );
        bus.redo(&mut document).expect("redo");
        assert_eq!(document.environment.pins, vec![key_light()]);
    }

    /// A pin the compositor cannot use is refused rather than written.
    ///
    /// A NaN reaching the environment image poisons the CDF, and a poisoned
    /// CDF fails as a light that is sampled always or never — which looks like
    /// a renderer bug rather than like bad input.
    #[test]
    fn a_pin_that_would_poison_the_cdf_is_refused() {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();

        for broken in [
            Pin {
                direction: vec3(0.0, 0.0, 0.0),
                ..key_light()
            },
            Pin {
                intensity: f32::NAN,
                ..key_light()
            },
            Pin {
                angular_size: vec2(0.0, 0.5),
                ..key_light()
            },
            Pin {
                softness: 4.0,
                ..key_light()
            },
            Pin {
                direction: vec3(f32::INFINITY, 0.0, 0.0),
                ..key_light()
            },
        ] {
            assert!(
                bus.apply(
                    &mut document,
                    Command::SetEnvironmentPins {
                        pins: vec![broken.clone()]
                    }
                )
                .is_err(),
                "a pin with {broken:?} was accepted"
            );
        }
        assert!(
            document.environment.pins.is_empty(),
            "a refused pin was written anyway"
        );
    }

    /// The pin reaches the environment the renderer is handed, and the cache
    /// notices when it changes.
    ///
    /// The cache is the part worth testing here. It keys on what the CDF was
    /// built from, and pins change the image the CDF is built from — a key
    /// that ignored them would keep serving an environment that does not know
    /// a light is there, so the pin would appear in reflections and be
    /// invisible to the sampler.
    #[test]
    fn a_pin_reaches_the_built_environment_and_invalidates_the_cache() {
        let mut settings = crate::document::EnvironmentSettings::default();
        let mut cache = crate::environment::EnvironmentCache::new();

        let aim = key_light().direction.normalize();
        let before = cache.environment(&settings).radiance(aim);

        settings.pins = vec![Pin {
            intensity: 40.0,
            ..key_light()
        }];
        let after = cache.environment(&settings).radiance(aim);
        assert!(
            after.g > before.g + 10.0,
            "the built environment reads {} where the pin points against {} \
             without it; either the pin is not composited or the cache served \
             a stale build",
            after.g,
            before.g
        );

        // ...and turning it off gets the original back, so the cache is
        // keyed on the pins rather than merely dirtied once.
        settings.pins[0].enabled = false;
        let off = cache.environment(&settings).radiance(aim);
        assert!(
            (off.g - before.g).abs() < 1e-5,
            "disabling the pin left {} rather than the original {}",
            off.g,
            before.g
        );
    }
}

#[cfg(test)]
mod bump_attach_tests {
    use super::*;
    use tequila_materials::graph::NodeKind;
    use tequila_texture::texture::MapRole;

    fn plastic() -> (SceneDocument, CommandBus, MaterialKey) {
        let mut document = SceneDocument::new();
        let bus = CommandBus::new();
        let key = document.materials.insert(crate::material::MaterialDefinition::new(
            "Panel",
            tequila_materials::material_type::MaterialType::Plastic,
            Default::default(),
        ));
        (document, bus, key)
    }

    fn bump_driver(document: &SceneDocument, key: MaterialKey) -> NodeKind {
        let graph = &document.materials.get(key).unwrap().graph;
        let driver = graph.driver_of(Param::Bump).expect("bump is driven");
        graph.node(driver).unwrap().kind.clone()
    }

    fn texture(document: &mut SceneDocument, bus: &mut CommandBus, key: MaterialKey, role: MapRole) {
        bus.apply(
            document,
            Command::SetParameterTexture {
                material: key,
                param: Param::Bump,
                source: Some(crate::textures::TextureSource::new("map.png", role)),
            },
        )
        .expect("attach");
    }

    /// A pattern on Bump goes through a Bump node whose depth the Material
    /// tab can read and change, as one undoable step, without the graph
    /// editor. It had no control for it: a leather grain came in at the
    /// default depth, and changing that meant opening the graph.
    #[test]
    fn a_bump_patterns_depth_can_be_read_and_changed() {
        let (mut document, mut bus, key) = plastic();
        let pattern = tequila_texture::procedural::Procedural::catalog()[9];
        bus.apply(
            &mut document,
            Command::SetParameterProcedural { material: key, param: Param::Bump, procedural: Some(Box::new(pattern)), seed: 1 },
        )
        .expect("attach");
        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(bump_depth_of(graph, Param::Bump), Some(tequila_materials::graph::DEFAULT_BUMP_DEPTH));
        assert_eq!(bump_depth_of(graph, Param::SpecularRoughness), None);

        let mut deeper = graph.clone();
        assert!(set_bump_depth(&mut deeper, Param::Bump, 0.002));
        bus.apply(&mut document, Command::SetGraph { material: key, graph: Box::new(deeper) }).expect("set");
        assert_eq!(bump_depth_of(&document.materials.get(key).unwrap().graph, Param::Bump), Some(0.002));
        // The pattern is still there behind it, and an undo puts the depth back.
        assert_eq!(procedural_of(&document.materials.get(key).unwrap().graph, Param::Bump).map(|(p, _)| p), Some(pattern));
        bus.undo(&mut document).expect("undo");
        assert_eq!(
            bump_depth_of(&document.materials.get(key).unwrap().graph, Param::Bump),
            Some(tequila_materials::graph::DEFAULT_BUMP_DEPTH)
        );
        assert!(!set_bump_depth(&mut document.materials.get_mut(key).unwrap().graph.clone(), Param::SpecularRoughness, 0.1));
    }

    /// A VDB file on Density becomes a Volume Map wired to it — which is
    /// what the snapshot looks for to vary the medium — and comes off again
    /// with an undo, leaving the medium as it was.
    #[test]
    fn a_vdb_on_density_is_a_volume_map() {
        let (mut document, mut bus, key) = plastic();
        let before = document.materials.get(key).unwrap().clone();
        bus.apply(
            &mut document,
            Command::SetParameterTexture {
                material: key,
                param: Param::VolumeDensity,
                source: Some(crate::textures::TextureSource::new("smoke.vdb", MapRole::Volume)),
            },
        )
        .expect("attach");
        let graph = &document.materials.get(key).unwrap().graph;
        let driver = graph.driver_of(Param::VolumeDensity).expect("driven");
        let NodeKind::VolumeMap { volume } = graph.node(driver).unwrap().kind else {
            panic!("Density is driven by {:?}", graph.node(driver).unwrap().kind);
        };
        let program = graph.compile().expect("compiles");
        assert_eq!(tequila_render_core::density::volume_of(&program), Some(volume));
        assert_eq!(document.textures.get(volume).map(|s| s.role), Some(MapRole::Volume));

        bus.undo(&mut document).expect("undo");
        assert_eq!(document.materials.get(key).unwrap(), &before);
    }

    /// A pattern or a height map on Bump goes through a Bump node, which
    /// gives its 0-to-1 value a depth; a normal map goes straight in, where
    /// it is read as one. Either way the pattern is still found behind it.
    #[test]
    fn heights_get_a_bump_node_and_normal_maps_do_not() {
        let (mut document, mut bus, key) = plastic();
        let pattern = tequila_texture::procedural::Procedural::catalog()[9];
        bus.apply(
            &mut document,
            Command::SetParameterProcedural {
                material: key,
                param: Param::Bump,
                procedural: Some(Box::new(pattern)),
                seed: 1,
            },
        )
        .expect("attach");
        assert!(matches!(bump_driver(&document, key), NodeKind::Bump { .. }));
        let graph = &document.materials.get(key).unwrap().graph;
        assert_eq!(procedural_of(graph, Param::Bump).map(|(p, _)| p), Some(pattern));
        graph.compile().expect("compiles");

        texture(&mut document, &mut bus, key, MapRole::Height);
        assert!(matches!(bump_driver(&document, key), NodeKind::Bump { .. }));

        texture(&mut document, &mut bus, key, MapRole::Normal);
        assert!(matches!(bump_driver(&document, key), NodeKind::ImageTexture { .. }));

        // Other parameters are untouched by any of this.
        texture(&mut document, &mut bus, key, MapRole::Height);
        bus.apply(
            &mut document,
            Command::SetParameterProcedural {
                material: key,
                param: Param::SpecularRoughness,
                procedural: Some(Box::new(pattern)),
                seed: 1,
            },
        )
        .expect("attach");
        let graph = &document.materials.get(key).unwrap().graph;
        let driver = graph.driver_of(Param::SpecularRoughness).unwrap();
        assert!(matches!(graph.node(driver).unwrap().kind, NodeKind::Procedural { .. }));
    }
}
