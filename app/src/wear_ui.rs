//! The wear patterns' controls (`tequila_texture::wear`): scratches, dust,
//! fingerprints and smudges, scuffs, and edge wear, as the Material tab and
//! the node editor's sidebar show them.
//!
//! The sliders are named for what they do to the marks, not for the
//! arithmetic behind them, and each one that is not obvious says on hover
//! what it changes. Sizes are as every other pattern has them: a Scale in
//! features per metre, the rest shares of a feature.

use tequila_texture::procedural::Procedural;
use tequila_texture::wear::{Dust, EdgeWear, ScratchDirection, Scratches, Scuffs, Fingerprints};

/// What a wear pattern is for, in one line, for the pattern menu.
pub(crate) fn hint(p: &Procedural) -> &'static str {
    match p {
        Procedural::WearScratches(_) => {
            "Scratches that follow the surface, no UVs needed: length, width, taper, \
             bend, wobble, a direction or swirl marks, and fine hairlines among them. \
             Put it on roughness, colour or bump."
        }
        Procedural::Dust(_) => {
            "A soft dust film with fine specks, gathering on what faces up. Put it on \
             colour and roughness, or on a coat's roughness."
        }
        Procedural::Fingerprints(_) => {
            "Fingerprints with their ridges, and wiped smears: for screens, glass, \
             gloss plastics and polished metal. Put it on roughness."
        }
        Procedural::Scuffs(_) => {
            "Soft worn patches full of fine rub lines, where something has rubbed. \
             Dulls the gloss and lightens the colour."
        }
        Procedural::EdgeWear(_) => {
            "Wear on a part's edges, worn through where they are sharpest and ragged \
             where it stops: paint worn to the metal, a moulding's pale edges."
        }
        _ => "",
    }
}

fn scale(ui: &mut egui::Ui, frequency: &mut f32, range: std::ops::RangeInclusive<f32>, hover: &str) {
    ui.add(crate::field::Slider::new(frequency, range).text("Scale").logarithmic(true)).on_hover_text(hover);
}

fn share(ui: &mut egui::Ui, value: &mut f32, label: &str, hover: &str) {
    ui.add(crate::field::Slider::new(value, 0.0..=1.0).text(label)).on_hover_text(hover);
}

pub(crate) fn scratches(ui: &mut egui::Ui, s: &mut Scratches) {
    scale(ui, &mut s.frequency, 1.0..=2000.0, "Scratches per metre, roughly: higher is smaller and more of them.");
    share(ui, &mut s.density, "Density", "How many places hold a scratch.");
    share(ui, &mut s.length, "Length", "How long the scratches are, for their scale.");
    share(ui, &mut s.length_variation, "Length variation", "How much shorter some are than others.");
    ui.add(crate::field::Slider::new(&mut s.width, 0.0005..=0.05).text("Width").logarithmic(true))
        .on_hover_text("How wide a scratch is at its widest, for its scale. Deeper ones are wider.");
    share(ui, &mut s.taper, "Taper", "How much a scratch narrows towards its ends. At 1 it runs to a point.");
    share(ui, &mut s.fade, "Fade", "How softly the ends fade out, rather than stopping short.");
    share(ui, &mut s.bend, "Bend", "Straight at 0; curling into hooks at 1.");
    share(ui, &mut s.wobble, "Wobble", "How hand-made a scratch is: its line wanders and its width comes and goes.");
    ui.horizontal(|ui| {
        ui.label("Direction");
        egui::ComboBox::from_id_salt(ui.id().with("wear_scratch_direction"))
            .selected_text(s.direction.label())
            .show_ui(ui, |ui| {
                for direction in ScratchDirection::ALL {
                    ui.selectable_value(&mut s.direction, direction, direction.label());
                }
            })
            .response
            .on_hover_text(
                "Any way, as handling leaves them; along an axis of the part, as a slide or \
                 a sander leaves them; or swirls, the rings a polishing pad leaves round a \
                 highlight.",
            );
    });
    if s.direction != ScratchDirection::Random {
        share(
            ui,
            &mut s.spread,
            "Spread",
            if s.direction == ScratchDirection::Swirl {
                "How far the swirls turn off their circles."
            } else {
                "How far scratches turn from the axis. At 0 they are all parallel."
            },
        );
    }
    share(ui, &mut s.depth_variation, "Depth variation", "How much shallower some scratches are than others.");
    ui.add(crate::field::Slider::new(&mut s.layers, 1..=3).text("Layers"))
        .on_hover_text("The scratches, then one or two layers of ever finer, fainter hairlines among them.");
}

pub(crate) fn dust(ui: &mut egui::Ui, d: &mut Dust) {
    scale(ui, &mut d.frequency, 1.0..=400.0, "Patches of film per metre.");
    share(ui, &mut d.amount, "Amount", "How much of the film there is, from none to a coat.");
    share(ui, &mut d.patchiness, "Patchiness", "Even at 0; in drifts with clean ground between at 1.");
    share(ui, &mut d.specks, "Specks", "How many grains of dust lie in it.");
    ui.add(crate::field::Slider::new(&mut d.speck_frequency, 50.0..=20000.0).text("Speck size").logarithmic(true))
        .on_hover_text("Specks per metre, about: higher is finer.");
    share(ui, &mut d.up_bias, "Settles on top", "How much the dust keeps to faces looking up. At 0 it is everywhere alike.");
}

pub(crate) fn fingerprints(ui: &mut egui::Ui, s: &mut Fingerprints) {
    scale(ui, &mut s.frequency, 1.0..=400.0, "Prints per metre, about: higher is smaller prints. At 25 a print is about 18 mm long.");
    share(ui, &mut s.prints, "Prints", "How many fingerprints.");
    share(ui, &mut s.smears, "Smears", "How much wiped, oily film lies between them.");
    share(ui, &mut s.ridges, "Ridges", "How clearly a print's ridges show. At 0 a print is a plain oily patch.");
    share(ui, &mut s.softness, "Softness", "How soft a print's edge is.");
}

pub(crate) fn scuffs(ui: &mut egui::Ui, s: &mut Scuffs) {
    scale(ui, &mut s.frequency, 0.5..=400.0, "Scuffs per metre, about: higher is smaller and more of them.");
    share(ui, &mut s.coverage, "Coverage", "How many places are scuffed.");
    share(ui, &mut s.stretch, "Stretch", "Round at 0; a long streak the way it was rubbed at 1.");
    share(ui, &mut s.streaks, "Rub lines", "How much the fine lines of the rubbing show inside a scuff.");
    share(ui, &mut s.softness, "Softness", "How soft a scuff's edge is.");
}

pub(crate) fn edge_wear(ui: &mut egui::Ui, e: &mut EdgeWear) {
    // Shown in millimetres, as an edge radius is thought of; kept in metres.
    let mut millimetres = e.radius * 1000.0;
    if ui
        .add(crate::field::Slider::new(&mut millimetres, 0.1..=50.0).text("Edge radius (mm)").logarithmic(true))
        .on_hover_text(
            "Edges rounded this tightly or tighter can wear through; gentler ones less, \
             flat faces never. Read from the model's own shape, as the Curvature node is.",
        )
        .changed()
    {
        e.radius = millimetres / 1000.0;
    }
    share(ui, &mut e.amount, "Amount", "How far in from the edge the wear reaches.");
    share(ui, &mut e.breakup, "Breakup", "How ragged its border is: smooth at 0, in chips at 1.");
    ui.add(crate::field::Slider::new(&mut e.frequency, 10.0..=5000.0).text("Breakup scale").logarithmic(true))
        .on_hover_text("Chips per metre, about: higher is finer.");
    share(ui, &mut e.softness, "Softness", "How soft its border is.");
}

/// How much surface, in scene units, a wear pattern's node previews over at
/// most: a few of its own marks, where a whole model's width would put
/// them under a pixel. `None` for anything else.
pub(crate) fn preview_extent(kind: &tequila_materials::graph::NodeKind) -> Option<f32> {
    let tequila_materials::graph::NodeKind::Procedural { procedural, .. } = kind else { return None };
    match **procedural {
        Procedural::WearScratches(s) => Some(3.0 / s.frequency.max(1e-3)),
        Procedural::Dust(d) => Some(2.0 / d.frequency.max(1e-3)),
        Procedural::Fingerprints(f) => Some(3.0 / f.frequency.max(1e-3)),
        Procedural::Scuffs(s) => Some(4.0 / s.frequency.max(1e-3)),
        Procedural::EdgeWear(e) => Some(40.0 / e.frequency.max(1e-3)),
        _ => None,
    }
}

/// The edge radius of the first Edge wear in `graph`, for a preview to give
/// its square a curvature: a flat square has no edge to wear.
pub(crate) fn preview_edge(graph: &tequila_materials::graph::MaterialGraph) -> Option<f32> {
    graph.nodes().find_map(|(_, node)| match &node.kind {
        tequila_materials::graph::NodeKind::Procedural { procedural, .. } => match **procedural {
            Procedural::EdgeWear(e) => Some(e.radius.max(1e-6)),
            _ => None,
        },
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every wear pattern says what it is for in the pattern menu.
    #[test]
    fn every_wear_pattern_has_a_hint() {
        for p in Procedural::catalog().iter().filter(|p| p.is_wear()) {
            assert!(!hint(p).is_empty(), "{} has no hint", p.label());
        }
    }

    /// The controls lay out for every wear pattern without editing
    /// anything that was not touched.
    #[test]
    fn the_controls_draw_and_change_nothing_untouched() {
        let ctx = egui::Context::default();
        for p in Procedural::catalog().into_iter().filter(|p| p.is_wear()) {
            let mut edited = p;
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| match &mut edited {
                    Procedural::WearScratches(s) => scratches(ui, s),
                    Procedural::Dust(d) => dust(ui, d),
                    Procedural::Fingerprints(s) => fingerprints(ui, s),
                    Procedural::Scuffs(s) => scuffs(ui, s),
                    Procedural::EdgeWear(e) => edge_wear(ui, e),
                    _ => {}
                });
            });
            assert_eq!(edited, p, "{} changed without being touched", p.label());
        }
    }
}
