//! What a model's numbers are in (`docs/03-SCENE-AND-IO.md` §3.4).
//!
//! STL, OBJ and plenty of other exports carry no unit, and a document's
//! lengths are metres. So a part a CAD package wrote out as 121 ×
//! 110 × 25 millimetres came in 121 metres across, and every material sized
//! for real objects — cork's grains, wood's rings, a weave's threads, how far
//! light gets into milk — was a thousandth of the size it should have been
//! on it: too fine to see. The importer guesses a unit from the size and
//! says it guessed, and §3.4 asks for the guess to be put to the user;
//! nothing asked, and the guess was dropped.
//!
//! Now the guess is applied, and a window says what was assumed and offers
//! the other units, each with the size the model comes out at, so the right
//! one is the one that reads as the right size. The same window, from a
//! part's menu, corrects a model already in a scene.
//!
//! Only the model's size changes, as one step to undo: its top nodes are
//! scaled about the middle of its base, so it stands where it stood - or,
//! just imported, about the origin, as a file that states its unit is. The
//! scene's unit, which is only how lengths are shown, is left alone.

use eframe::egui;
use tequila_core::math::{vec3, Aabb, Mat4, Transform, Vec3};
use tequila_core::units::LengthUnit;
use tequila_scene::tree::NodeId;
use tequila_scene::{Command, SceneDocument};

/// The units offered, smallest first.
const UNITS: [LengthUnit; 5] = [
    LengthUnit::Millimetre,
    LengthUnit::Centimetre,
    LengthUnit::Inch,
    LengthUnit::Foot,
    LengthUnit::Metre,
];

/// One model, and what its numbers are being read as.
#[derive(Clone, Debug)]
pub struct ModelUnits {
    roots: Vec<NodeId>,
    name: String,
    /// Its size in the numbers it had when this was made.
    numbers: Vec3,
    /// The middle of its base, which scaling leaves where it is.
    pivot: Vec3,
    /// Metres per one of those numbers, as it is being read now.
    metres_per_number: f64,
    /// Whether the reading is the importer's guess, applied on import.
    guessed: bool,
}

impl ModelUnits {
    /// The model under `roots` as it stands, its numbers read as the metres
    /// they are stored in: `None` when it has no geometry to measure.
    pub fn new(document: &SceneDocument, roots: Vec<NodeId>, name: String) -> Option<ModelUnits> {
        let bounds = bounds_of(document, &roots)?;
        let centre = bounds.center();
        Some(ModelUnits {
            roots,
            name,
            numbers: bounds.max - bounds.min,
            pivot: vec3(centre.x, bounds.min.y, centre.z),
            metres_per_number: 1.0,
            guessed: false,
        })
    }

    /// Scaled about the scene's origin, at its floor, rather than where it
    /// stands: for a model just imported, which then lands where the same
    /// file comes in when it states its unit, rather than a few centimetres
    /// across and as far from the origin as its numbers were.
    pub fn about_origin(mut self) -> ModelUnits {
        self.pivot = vec3(0.0, self.pivot.y, 0.0);
        self
    }

    /// Read the numbers as `unit`: the command that rescales the model from
    /// its current reading, or `None` when it is read that way already.
    pub fn read_as(&mut self, unit: LengthUnit, document: &SceneDocument) -> Option<Command> {
        let ratio = unit.to_metres() / self.metres_per_number;
        if (ratio - 1.0).abs() < 1e-9 {
            return None;
        }
        self.metres_per_number = unit.to_metres();
        self.guessed = false;
        Some(rescale(document, &self.roots, self.pivot, ratio as f32))
    }

    /// The same, as the importer's guess: the window says it was assumed.
    pub fn guess(&mut self, unit: LengthUnit, document: &SceneDocument) -> Option<Command> {
        let command = self.read_as(unit, document);
        self.guessed = true;
        command
    }

    /// The model's top nodes.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn roots(&self) -> &[NodeId] {
        &self.roots
    }

    /// The unit the numbers are read in now, when it is one of the units.
    pub fn reading(&self) -> Option<LengthUnit> {
        UNITS.into_iter().find(|unit| (unit.to_metres() / self.metres_per_number - 1.0).abs() < 1e-6)
    }

    /// Its real size read in `metres_per_number`, for a person to recognise.
    fn size_at(&self, metres_per_number: f64) -> String {
        real_size(self.numbers * metres_per_number as f32)
    }

    /// The window. Returns a rescale to apply, and whether it is still open.
    pub fn show(&mut self, ctx: &egui::Context, document: &SceneDocument) -> (Option<Command>, bool) {
        let mut open = true;
        let mut done = false;
        let mut command = None;
        egui::Window::new("Model units")
            .id(egui::Id::new("model-units"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(340.0)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                let now = self.size_at(self.metres_per_number);
                match (self.guessed, self.reading()) {
                    (true, Some(unit)) => ui.label(format!(
                        "{} doesn't say what unit it is in. It was read as {}: {now}.",
                        self.name,
                        crate::unit_label(unit).to_lowercase()
                    )),
                    _ => ui.label(format!("What unit are {}'s numbers in? It is {now} now.", self.name)),
                };
                ui.add_space(6.0);
                egui::Grid::new("model-units-choices").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                    for unit in UNITS {
                        let chosen = self.reading() == Some(unit);
                        if ui.selectable_label(chosen, crate::unit_label(unit)).clicked() && !chosen {
                            command = self.read_as(unit, document);
                        }
                        ui.label(egui::RichText::new(self.size_at(unit.to_metres())).color(crate::theme::dim()));
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Materials are made to real sizes, so on a model at the wrong size their \
                         patterns come out far too fine or too coarse. Only the model's size \
                         changes, and Undo takes it back.",
                    )
                    .small()
                    .color(crate::theme::dim()),
                );
                ui.add_space(4.0);
                if ui.button("Done").clicked() {
                    done = true;
                }
            });
        (command, open && !done)
    }
}

/// The world bounds of the parts under `roots`.
fn bounds_of(document: &SceneDocument, roots: &[NodeId]) -> Option<Aabb> {
    let mut bounds = Aabb::EMPTY;
    for &root in roots {
        bounds.extend(&document.tree.bounds(root, &|mesh| document.geometry.get(mesh).map(|m| m.bounds())));
    }
    (!bounds.is_empty() && (bounds.max - bounds.min).length() > 0.0).then_some(bounds)
}

/// Scale the model under `roots` by `ratio` about `pivot`, in the world: one
/// command for all of them, so it is one step to undo.
fn rescale(document: &SceneDocument, roots: &[NodeId], pivot: Vec3, ratio: f32) -> Command {
    let about = Mat4::translation(pivot) * Mat4::scale(Vec3::splat(ratio)) * Mat4::translation(-pivot);
    let transforms = roots
        .iter()
        .filter_map(|&root| {
            let local = document.tree.get(root)?.transform;
            let world = document.tree.world_transform(root);
            // world = parent · local, so the new local is parent⁻¹ · about ·
            // world, and parent⁻¹ is local · world⁻¹.
            Transform::from_matrix(local.matrix * world.inverse * about * world.matrix).map(|t| (root, t))
        })
        .collect();
    Command::SetTransform { transforms }
}

/// A size as a person would say it: in millimetres, centimetres, metres or
/// kilometres by how big it is, to three figures.
pub fn real_size(metres: Vec3) -> String {
    let largest = metres.x.max(metres.y).max(metres.z);
    let (per, symbol) = match largest {
        l if l < 0.1 => (0.001, "mm"),
        l if l < 2.0 => (0.01, "cm"),
        l if l < 2000.0 => (1.0, "m"),
        _ => (1000.0, "km"),
    };
    let figure = |metres: f32| {
        let value = metres / per;
        let decimals = if value >= 100.0 { 0 } else if value >= 10.0 { 1 } else { 2 };
        let text = format!("{value:.decimals$}");
        if text.contains('.') { text.trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
    };
    format!("{} × {} × {} {symbol}", figure(metres.x), figure(metres.y), figure(metres.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_as_people_say_them() {
        assert_eq!(real_size(vec3(0.121, 0.110, 0.025)), "12.1 × 11 × 2.5 cm");
        assert_eq!(real_size(vec3(121.0, 110.0, 25.0)), "121 × 110 × 25 m");
        assert_eq!(real_size(vec3(0.004, 0.0025, 0.001)), "4 × 2.5 × 1 mm");
        assert_eq!(real_size(vec3(3.07, 2.79, 0.64)), "3.07 × 2.79 × 0.64 m");
    }
}
