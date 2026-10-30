//! The Scene Document, command bus, undo/redo, and dirty-state tracking.
//!
//! `TEQUILA_SPEC.md` §2.2 states the rule this crate enforces: **nothing
//! mutates the Scene Document except the Command Bus.** The UI and the Python
//! API are peers, both issuing commands, which is what makes undo, scripting
//! and automated testing work without duplicated logic.
//!
//! That is enforced structurally rather than by convention: the document's
//! mutating methods are `pub(crate)`, so the only public path to a change is
//! [`command::Command`].

#![warn(missing_debug_implementations)]

pub mod animation;
pub mod apply;
pub mod bench;
pub mod bubbles;
pub mod fuzz;
pub mod command;
pub mod configurator;
pub mod cutaway;
pub mod decal_fit;
pub mod demo;
pub mod displacement;
pub mod document;
pub mod environment;
pub mod geometry;
pub mod gizmo;
pub mod lighting;
pub mod livelink;
pub mod material;
pub mod measure;
pub mod metadata;
pub mod occlusion;
pub mod orbit;
pub mod paths;
pub mod preset;
pub mod refine;
pub mod reimport;
pub mod render;
pub mod selection;
pub mod snapshot;
pub mod split;
pub mod studio;
pub mod templates;
pub mod textures;
pub mod tree;
pub mod units;
pub mod uv;
pub mod wireframe;

pub use command::{Command, CommandBus, CommandError};
pub use document::SceneDocument;
pub use geometry::{GeometryStore, Mesh};
pub use material::{MaterialDefinition, MaterialStore};
pub use tree::{MaterialKey, MeshKey, NodeId, NodeKind, SceneNode, SceneTree};
pub use units::{ConversionMode, ConversionPlan, SceneUnits};
