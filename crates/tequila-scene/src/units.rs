//! Scene units and the two ways of changing them.
//!
//! `docs/03-SCENE-AND-IO.md` §3 titles itself "get this right or physics is
//! meaningless", and `docs/05` §6 lists mishandled units as a risk whose
//! consequence is "physically wrong lighting that users cannot diagnose" —
//! the image looks plausible and is wrong.
//!
//! Two decisions from §3 carry the weight:
//!
//! - **Lengths are stored in metres**, always. The unit is how lengths are
//!   shown and typed, not a storage format. Everything that measures works
//!   in metres — both render engines, every material's real sizes (cork's
//!   grain, how far light gets into milk), light falloff, a camera's depth of
//!   field, the importer — so this is the one arrangement in which they all
//!   agree. Documents used to carry a `metres_per_unit` factor as well, which
//!   the readouts honoured and the renderer never read: a millimetre scene's
//!   numbers rendered as metres, a thousand times too big. Files saved that
//!   way are scaled into metres when they are opened.
//! - **Both conversion modes exist**, because they answer different questions,
//!   and a user who wanted one and got the other has silently corrupted their
//!   scene.

use crate::document::{EnvironmentSettings, SceneDocument};
use crate::lighting::{CameraStore, Intensity, LightSettings, LightShape, SavedCamera};
use crate::measure::Measurement;
use crate::studio::Studio;
use crate::tree::{MaterialKey, NodeId, NodeKind};
use tequila_core::math::{Mat4, Transform, Vec3};
use tequila_core::units::{LengthUnit, UnitDependent};
use tequila_render_core::camera::Projection;

/// The document's unit setting: what lengths are shown and typed in. They
/// are stored in metres whatever it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SceneUnits {
    pub unit: LengthUnit,
}

impl SceneUnits {
    pub fn new(unit: LengthUnit) -> Self {
        SceneUnits { unit }
    }

    /// Whether the document has an established unit. A new scene does not,
    /// and shows its lengths in metres, which is what they are.
    pub fn is_established(&self) -> bool {
        self.unit != LengthUnit::Unspecified
    }

    /// The unit lengths are shown in: the document's, or metres when it has
    /// none.
    pub fn shown(&self) -> LengthUnit {
        match self.unit {
            LengthUnit::Unspecified => LengthUnit::Metre,
            unit => unit,
        }
    }
}

/// Which question the user is answering when they change units (§3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionMode {
    /// "The model is the right size; show its lengths in another unit."
    ///
    /// The model keeps its physical size, so nothing in the scene changes and
    /// the render is exactly what it was. Lengths are shown and typed in the
    /// new unit.
    KeepSceneSize,
    /// "The model is the wrong size: its numbers were meant in the new unit."
    ///
    /// The numbers stay as they read, so the scene is scaled by the ratio of
    /// the units — geometry, lights and cameras together, and light power by
    /// its square so the shot keeps its framing and its brightness. Materials
    /// keep their real sizes, which is the point: on a model that came in a
    /// thousand times too big, cork's grain and wood's rings were a thousandth
    /// of the size they should have been.
    ScaleScene,
}

impl ConversionMode {
    pub const ALL: [ConversionMode; 2] =
        [ConversionMode::KeepSceneSize, ConversionMode::ScaleScene];

    pub const fn label(self) -> &'static str {
        match self {
            ConversionMode::KeepSceneSize => "Keep Scene Size",
            ConversionMode::ScaleScene => "Scale Scene",
        }
    }

    /// Independently written guidance for the conversion dialog, per
    /// `TEQUILA_SPEC.md` §0.2 rule 2.
    pub const fn description(self) -> &'static str {
        match self {
            ConversionMode::KeepSceneSize => {
                "Keeps the model's real size. Nothing in the scene changes; lengths \
                 are shown and typed in the new unit. Use this when the model is \
                 the right size."
            }
            ConversionMode::ScaleScene => {
                "Keeps the numbers and reads them in the new unit, so the scene is \
                 rescaled, with its lights and cameras. Materials keep their real \
                 sizes. Use this when the model came in at the wrong size."
            }
        }
    }
}

/// What a unit change will do, computed before it is applied.
///
/// §3.3 asks for "a preview of what will change". Producing the plan
/// separately from applying it is what makes that possible, and it also makes
/// the whole conversion one undoable command.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversionPlan {
    pub mode: ConversionMode,
    pub from: SceneUnits,
    pub to: SceneUnits,
    /// Factor applied to lengths in the scene. Exactly 1 for
    /// `KeepSceneSize`, which moves nothing.
    pub geometry_scale: f64,
    /// Factor applied to each unit-dependent quantity.
    pub quantity_scales: Vec<(UnitDependent, f64)>,
}

impl ConversionPlan {
    /// Work out what changing to `target` would do.
    pub fn compute(current: SceneUnits, target: LengthUnit, mode: ConversionMode) -> Self {
        let geometry_scale = match mode {
            ConversionMode::KeepSceneSize => 1.0,
            // A part that reads 121 in metres, read as millimetres, has to
            // become 0.121 m long. A scene with no unit reads in metres.
            ConversionMode::ScaleScene => target.to_metres() / current.unit.to_metres(),
        };
        let quantity_scales = UnitDependent::ALL
            .into_iter()
            .map(|quantity| {
                let scale = match quantity {
                    // What makes up the shot moves with the geometry, each by
                    // its own power. Light intensity is the one that goes as
                    // the square, and forgetting it is the documented failure
                    // mode of this conversion.
                    UnitDependent::Geometry
                    | UnitDependent::CameraDistance
                    | UnitDependent::LightIntensity
                    | UnitDependent::EnvironmentSize => quantity.factor(geometry_scale),
                    // Materials are made to real sizes, in metres, and keep
                    // them: that is what puts their patterns, depths and
                    // bumps at the right size on a model that was not.
                    UnitDependent::MediumDistance
                    | UnitDependent::TextureMappingScale
                    | UnitDependent::DisplacementHeight => 1.0,
                };
                (quantity, scale)
            })
            .collect();

        ConversionPlan {
            mode,
            from: current,
            to: SceneUnits::new(target),
            geometry_scale,
            quantity_scales,
        }
    }

    /// The factor for one quantity.
    pub fn scale_for(&self, quantity: UnitDependent) -> f64 {
        self.quantity_scales
            .iter()
            .find(|(q, _)| *q == quantity)
            .map(|(_, s)| *s)
            .unwrap_or(1.0)
    }

    /// Whether applying this plan would change anything at all.
    pub fn is_noop(&self) -> bool {
        self.from == self.to
            && (self.geometry_scale - 1.0).abs() < f64::EPSILON
            && self
                .quantity_scales
                .iter()
                .all(|(_, s)| (s - 1.0).abs() < f64::EPSILON)
    }

    /// A human-readable summary for the confirmation dialog.
    pub fn summary(&self) -> String {
        let from = if self.from.unit == LengthUnit::Unspecified {
            "unspecified units".to_string()
        } else {
            self.from.unit.abbreviation().to_string()
        };
        let to = self.to.unit.abbreviation();
        match self.mode {
            ConversionMode::KeepSceneSize => format!(
                "Show lengths in {to} instead of {from}: nothing in the scene changes."
            ),
            ConversionMode::ScaleScene => format!(
                "Read the numbers in {from} as {to}: the scene is scaled x{:.6} with its \
                 lights and cameras, light intensity x{:.6}. Materials keep their real sizes.",
                self.geometry_scale,
                self.scale_for(UnitDependent::LightIntensity)
            ),
        }
    }
}

/// Scale the whole scene by `factor` about the origin, so the shot keeps its
/// framing and its brightness: what Scale Scene does, and what opening a file
/// saved in another unit does to bring it into metres.
///
/// Moves with the scene: the top nodes' transforms (parts, lights and
/// cutters, and with them the lights' sizes), the cameras with their targets,
/// focus and orthographic extent, the studios' cameras and environments, the
/// dome and the ground, dimensions fixed in the world, and labels sized in
/// real units, which are placed on the parts rather than being what the
/// parts are made of. Light power follows the square. Materials keep their
/// real sizes.
///
/// Returns what it changed, as it was.
pub fn rescale(document: &mut SceneDocument, factor: f64) -> Rescaled {
    let before = Rescaled::capture(document);
    if !(factor.is_finite() && factor > 0.0) || (factor - 1.0).abs() < 1e-12 {
        return before;
    }
    let s = factor as f32;

    // Only the top nodes: a child's transform is relative to its parent, so
    // scaling it too would compound the factor once per level.
    let about = Mat4::scale(Vec3::splat(s));
    for root in document.tree.roots().to_vec() {
        if let Ok(node) = document.tree.get_mut(root) {
            if let Some(transform) = Transform::from_matrix(about * node.transform.matrix) {
                node.transform = transform;
            }
        }
    }

    let power = UnitDependent::LightIntensity.factor(factor) as f32;
    let lights: Vec<NodeId> = document
        .tree
        .iter()
        .filter(|n| n.kind.light().is_some())
        .map(|n| n.id)
        .collect();
    for id in lights {
        if let Ok(node) = document.tree.get_mut(id) {
            if let NodeKind::Light { settings } = &mut node.kind {
                let gain = power_gain(settings, power);
                settings.intensity = settings.intensity.with_value(settings.intensity.value() * gain);
            }
        }
    }

    for index in 0..document.cameras.len() {
        if let Some(saved) = document.cameras.get_mut(index) {
            scale_camera(saved, s);
        }
    }
    for studio in &mut document.studios {
        if let Some(saved) = &mut studio.camera {
            scale_camera(saved, s);
        }
        if let Some(environment) = &mut studio.environment {
            scale_environment(environment, s);
        }
    }
    scale_environment(&mut document.environment, s);

    // An end on a part is in the part's own space and moves with it.
    for measurement in &mut document.measurements {
        for anchor in [&mut measurement.a, &mut measurement.b] {
            if anchor.node.is_none() {
                anchor.point *= s;
            }
        }
    }

    let materials: Vec<MaterialKey> = document.materials.keys().collect();
    for key in materials {
        if let Some(definition) = document.materials.get_mut(key) {
            for decal in &mut definition.graph.decals {
                if decal.mapping.real_world_units {
                    decal.mapping.scale *= s;
                }
            }
        }
    }
    before
}

/// How much a light's stated power has to grow for it to light a scene
/// `power` times the area as it lit the original.
fn power_gain(settings: &LightSettings, power: f32) -> f32 {
    let measured = settings.photometry.as_ref().is_some_and(|p| !p.path.is_empty());
    match (settings.intensity, settings.shape) {
        // Luminance is per area, and the panel grew with the scene: it
        // already emits that much more, and lights the larger scene as it
        // lit the smaller one. Against a measured profile it is a plain
        // multiplier, like the others.
        (Intensity::Nits(_), LightShape::Area { .. }) if !measured => 1.0,
        _ => power,
    }
}

/// Move a camera with the scene. Its orientation stays as it is: a scale in
/// a camera's matrix would distort its rays rather than move it.
pub(crate) fn scale_camera(saved: &mut SavedCamera, s: f32) {
    let camera = &mut saved.camera;
    for row in 0..3 {
        camera.camera_to_world.m[row][3] *= s;
    }
    camera.focus_distance *= s;
    if let Projection::Orthographic { scale } = &mut camera.projection {
        *scale *= s;
    }
    saved.target *= s;
}

fn scale_environment(environment: &mut EnvironmentSettings, s: f32) {
    environment.size *= s;
    environment.height *= s;
    environment.ground.size *= s;
    environment.ground.reflection_fade_distance *= s;
    environment.ground.bounce_contact_falloff *= s;
}

/// Everything [`rescale`] changes, as it was before: undo puts it back
/// exactly, rather than scaling by the inverse and landing a rounding error
/// away from where it started.
#[derive(Clone, Debug, PartialEq)]
pub struct Rescaled {
    transforms: Vec<(NodeId, Transform)>,
    lights: Vec<(NodeId, Intensity)>,
    cameras: CameraStore,
    studios: Vec<Studio>,
    environment: EnvironmentSettings,
    measurements: Vec<Measurement>,
    decals: Vec<(MaterialKey, Vec<tequila_materials::decal::Decal>)>,
}

impl Rescaled {
    fn capture(document: &SceneDocument) -> Rescaled {
        Rescaled {
            transforms: document
                .tree
                .roots()
                .iter()
                .filter_map(|&id| Some((id, document.tree.get(id)?.transform)))
                .collect(),
            lights: document
                .tree
                .iter()
                .filter_map(|n| Some((n.id, n.kind.light()?.intensity)))
                .collect(),
            cameras: document.cameras.clone(),
            studios: document.studios.clone(),
            environment: document.environment.clone(),
            measurements: document.measurements.clone(),
            decals: document
                .materials
                .iter()
                .filter(|(_, d)| d.graph.decals.iter().any(|decal| decal.mapping.real_world_units))
                .map(|(key, d)| (key, d.graph.decals.clone()))
                .collect(),
        }
    }

    /// Put it all back.
    pub fn restore(&self, document: &mut SceneDocument) {
        for &(id, transform) in &self.transforms {
            if let Ok(node) = document.tree.get_mut(id) {
                node.transform = transform;
            }
        }
        for &(id, intensity) in &self.lights {
            if let Ok(node) = document.tree.get_mut(id) {
                if let NodeKind::Light { settings } = &mut node.kind {
                    settings.intensity = intensity;
                }
            }
        }
        document.cameras = self.cameras.clone();
        document.studios = self.studios.clone();
        document.environment = self.environment.clone();
        document.measurements = self.measurements.clone();
        for (key, decals) in &self.decals {
            if let Some(definition) = document.materials.get_mut(*key) {
                definition.graph.decals = decals.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_scene_has_no_established_unit_and_shows_metres() {
        let units = SceneUnits::default();
        assert!(!units.is_established());
        assert_eq!(units.shown(), LengthUnit::Metre);
        assert_eq!(SceneUnits::new(LengthUnit::Inch).shown(), LengthUnit::Inch);
    }

    /// Keep Scene Size changes how lengths read and nothing else. Moving
    /// anything here would be the exact opposite of what the user asked for.
    #[test]
    fn keep_scene_size_moves_nothing() {
        let plan = ConversionPlan::compute(
            SceneUnits::new(LengthUnit::Millimetre),
            LengthUnit::Metre,
            ConversionMode::KeepSceneSize,
        );
        assert_eq!(plan.geometry_scale, 1.0);
        for quantity in UnitDependent::ALL {
            assert_eq!(plan.scale_for(quantity), 1.0, "{quantity:?}");
        }
        assert_eq!(plan.to.unit, LengthUnit::Metre);
    }

    /// The headline behaviour of Scale Scene: a model whose numbers read 121
    /// in metres, read as millimetres, comes out 0.121 m long — and still
    /// reads 121 in the new unit.
    #[test]
    fn scale_scene_keeps_the_numbers_as_they_read() {
        let current = SceneUnits::default();
        let plan =
            ConversionPlan::compute(current, LengthUnit::Millimetre, ConversionMode::ScaleScene);
        assert!((plan.geometry_scale - 0.001).abs() < 1e-12, "{}", plan.geometry_scale);
        let read_before = 121.0 / current.shown().to_metres();
        let read_after = 121.0 * plan.geometry_scale / plan.to.shown().to_metres();
        assert!((read_before - read_after).abs() < 1e-9, "{read_before} -> {read_after}");
    }

    /// Light intensity scales with the **square** of the length change. This
    /// is the specific mistake §3.3 and `docs/05` §6 both warn about, and the
    /// one that produces wrong lighting a user cannot diagnose.
    #[test]
    fn scale_scene_scales_light_intensity_quadratically() {
        let plan = ConversionPlan::compute(
            SceneUnits::new(LengthUnit::Millimetre),
            LengthUnit::Metre,
            ConversionMode::ScaleScene,
        );
        assert!((plan.geometry_scale - 1000.0).abs() < 1e-9);
        assert!(
            (plan.scale_for(UnitDependent::LightIntensity) - 1.0e6).abs() < 1.0,
            "light intensity scaled by {}",
            plan.scale_for(UnitDependent::LightIntensity)
        );
        for quantity in [
            UnitDependent::Geometry,
            UnitDependent::CameraDistance,
            UnitDependent::EnvironmentSize,
        ] {
            assert!(
                (plan.scale_for(quantity) - 1000.0).abs() < 1e-6,
                "{quantity:?} scaled by {}",
                plan.scale_for(quantity)
            );
        }
    }

    /// What materials are made of keeps its real size: that is the whole
    /// reason to scale a model that came in at the wrong size.
    #[test]
    fn scale_scene_leaves_materials_at_their_real_sizes() {
        let plan = ConversionPlan::compute(
            SceneUnits::default(),
            LengthUnit::Millimetre,
            ConversionMode::ScaleScene,
        );
        for quantity in [
            UnitDependent::MediumDistance,
            UnitDependent::TextureMappingScale,
            UnitDependent::DisplacementHeight,
        ] {
            assert_eq!(plan.scale_for(quantity), 1.0, "{quantity:?}");
        }
    }

    /// The two modes must genuinely differ. A build where they agreed would
    /// silently corrupt every scene converted with the wrong one.
    #[test]
    fn the_two_modes_do_different_things() {
        let current = SceneUnits::new(LengthUnit::Millimetre);
        let keep =
            ConversionPlan::compute(current, LengthUnit::Inch, ConversionMode::KeepSceneSize);
        let scale = ConversionPlan::compute(current, LengthUnit::Inch, ConversionMode::ScaleScene);
        assert_ne!(keep.geometry_scale, scale.geometry_scale);
        assert_ne!(
            keep.scale_for(UnitDependent::LightIntensity),
            scale.scale_for(UnitDependent::LightIntensity)
        );
        assert_eq!(keep.to.unit, scale.to.unit);
    }

    #[test]
    fn converting_to_the_same_unit_changes_nothing() {
        let current = SceneUnits::new(LengthUnit::Centimetre);
        for mode in ConversionMode::ALL {
            let plan = ConversionPlan::compute(current, LengthUnit::Centimetre, mode);
            assert!(plan.is_noop(), "{mode:?} was not a no-op: {plan:?}");
        }
    }

    /// Converting there and back must land exactly where it started, or
    /// repeated unit changes drift the model.
    #[test]
    fn scale_scene_round_trips_exactly() {
        let start = SceneUnits::new(LengthUnit::Millimetre);
        let out = ConversionPlan::compute(start, LengthUnit::Foot, ConversionMode::ScaleScene);
        let back =
            ConversionPlan::compute(out.to, LengthUnit::Millimetre, ConversionMode::ScaleScene);
        let combined = out.geometry_scale * back.geometry_scale;
        assert!(
            (combined - 1.0).abs() < 1e-12,
            "round trip scaled by {combined}"
        );
        assert_eq!(back.to, start);
    }

    #[test]
    fn every_mode_has_a_label_and_guidance() {
        for mode in ConversionMode::ALL {
            assert!(!mode.label().is_empty());
            assert!(mode.description().len() > 40, "{mode:?}");
        }
    }

    #[test]
    fn summaries_name_the_units_and_the_factors() {
        let plan = ConversionPlan::compute(
            SceneUnits::new(LengthUnit::Millimetre),
            LengthUnit::Metre,
            ConversionMode::ScaleScene,
        );
        let summary = plan.summary();
        assert!(summary.contains("mm") && summary.contains('m'), "{summary}");
        assert!(summary.contains("light intensity"), "{summary}");
        let keep = ConversionPlan::compute(
            SceneUnits::new(LengthUnit::Millimetre),
            LengthUnit::Metre,
            ConversionMode::KeepSceneSize,
        );
        assert!(keep.summary().contains("nothing"), "{}", keep.summary());
    }

    /// Converting from an unspecified unit is the common case on import, and
    /// must not divide by zero or produce a nonsensical factor.
    #[test]
    fn converting_from_unspecified_units_is_well_defined() {
        let plan = ConversionPlan::compute(
            SceneUnits::default(),
            LengthUnit::Millimetre,
            ConversionMode::ScaleScene,
        );
        assert!(plan.geometry_scale.is_finite() && plan.geometry_scale > 0.0);
        assert!((plan.geometry_scale - 0.001).abs() < 1e-12);
        assert!(plan.summary().contains("unspecified"));
    }
}
