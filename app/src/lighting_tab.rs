//! The Lighting tab (`TEQUILA_SPEC.md` §1.6) and the light-node panel (§1.5).
//!
//! §1.6 is explicit that this tab "is not cosmetic — it is the global
//! quality/technique controller that determines which light-transport features
//! are enabled". So the presets are not a hint to the renderer; each one *is* a
//! complete [`IntegratorConfig`], and Custom is the absence of a preset rather
//! than a sixth one. That distinction is what lets the panel say both which
//! preset was chosen and that the scene no longer matches it — a tab showing
//! "Product" over a scene with shadows switched off would be lying.
//!
//! The light panel below it is the other half: a light is a node, so its
//! placement is the transform gizmo's job and everything here is what a
//! transform cannot express.

use eframe::egui;
use tequila_render_core::settings::LightingPreset;
use tequila_scene::lighting::{Intensity, LightSettings, LightShape};
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::Command;

use crate::project::Project;

#[derive(Default)]
pub struct LightingTab {}

impl LightingTab {
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        let lighting = project.document().lighting.clone();

        // Each preset as a card saying what it is for, with how fast it
        // renders at a glance: the choice is a trade, and the trade is what
        // has to be seen to choose.
        ui.spacing_mut().item_spacing.y = 6.0;
        for preset in LightingPreset::ALL {
            let (icon, hue, speed) = look(preset);
            let response = crate::theme::choice(
                ui,
                ui.id().with(("preset", preset.label())),
                icon,
                crate::theme::hue(hue),
                preset.label(),
                preset.description(),
                lighting.preset == Some(preset),
            );
            speed_pips(ui, response.rect, speed);
            if response.clicked() && lighting.preset != Some(preset) {
                let _ = project.apply(Command::SetLightingPreset { preset });
            }
        }
        let custom = lighting.preset.is_none();
        if crate::theme::choice(
            ui,
            ui.id().with("preset-custom"),
            crate::icons::Icon::Sliders,
            crate::theme::hue(crate::theme::Hue::Slate),
            "Custom",
            "Every transport setting, set by hand below.",
            custom,
        )
        .clicked()
            && !custom
        {
            // Starting from the configuration on screen rather than from a
            // default, so the first edit changes the one thing it edits.
            let _ = project.apply(Command::SetLightingConfig {
                config: Box::new(lighting.config()),
            });
        }

        crate::field::Fold::new("Transport")
            .default_open(custom)
            .show(ui, |ui| {
                let before = lighting.config();
                let mut config = before.clone();
                let mut bounces = config.max_bounces;
                ui.add(crate::field::Slider::new(&mut bounces, 1..=64).text("Ray bounces"));
                config.max_bounces = bounces;
                let mut transmission = config.max_transmission_bounces;
                ui.add(
                    crate::field::Slider::new(&mut transmission, 1..=64).text("Transmission bounces"),
                )
                .on_hover_text(
                    "Tracked apart from the bounce limit: glass and gems need many \
                     transmission events but few diffuse ones, and conflating them either \
                     makes glass black or makes everything slow.",
                );
                config.max_transmission_bounces = transmission;

                ui.add(crate::field::Checkbox::new(&mut config.global_illumination, "Global illumination"));
                ui.add(crate::field::Checkbox::new(&mut config.direct_lighting, "Direct lighting"));
                ui.add(crate::field::Checkbox::new(&mut config.shadows, "Shadows"));
                ui.add(crate::field::Checkbox::new(&mut config.light_source_materials, "Light source materials"));
                ui.add(crate::field::Checkbox::new(&mut config.ground_illumination, "Ground illumination"))
                    .on_hover_text("Let the ground plane bounce light back into the scene.");
                ui.add(crate::field::Checkbox::new(&mut config.caustics, "Caustics")).on_hover_text(
                    "Trace light through glass, liquids and gems, and off mirrors, onto \
                     what is around them: the bright patterns under a glass or beside a \
                     bottle. Needs solid glass (Glass Solid, Liquid, Gem); a thin pane \
                     passes light straight through. Slower; Jewelry turns it on.",
                );
                ui.add(crate::field::Checkbox::new(&mut config.interior_mode, "Interior mode"));
                ui.add(
                    crate::field::Slider::new(&mut config.regularization_roughness, 0.0..=0.4)
                        .text("Roughness floor"),
                );

                if config != before {
                    // Touching any of these leaves the preset behind — the
                    // scene no longer matches it, and continuing to show its
                    // name would be a claim the user cannot correct.
                    let _ = project.apply(Command::SetLightingConfig {
                        config: Box::new(config),
                    });
                }
            });
    }
}

/// A preset's icon, colour, and speed out of five.
fn look(preset: LightingPreset) -> (crate::icons::Icon, crate::theme::Hue, usize) {
    use crate::icons::Icon;
    use crate::theme::Hue;
    match preset {
        LightingPreset::Performance => (Icon::Gauge, Hue::Green, 5),
        LightingPreset::Basic => (Icon::Sun, Hue::Amber, 4),
        LightingPreset::Product => (Icon::Package, Hue::Blue, 3),
        LightingPreset::Interior => (Icon::House, Hue::Orange, 2),
        LightingPreset::Jewelry => (Icon::Gem, Hue::Violet, 1),
    }
}

/// How fast a preset renders, as five rising bars in a card's lower right
/// corner: lit up to `speed`.
fn speed_pips(ui: &egui::Ui, card: egui::Rect, speed: usize) {
    let p = crate::theme::p();
    let painter = ui.painter();
    let lit = crate::theme::hue(crate::theme::Hue::Green);
    for k in 0..5 {
        let height = 4.0 + k as f32 * 2.0;
        let x = card.right() - 13.0 - (4 - k) as f32 * 5.0;
        let bar = egui::Rect::from_min_max(egui::pos2(x - 3.0, card.bottom() - 10.0 - height), egui::pos2(x, card.bottom() - 10.0));
        painter.rect_filled(bar, 1.0, if k < speed { lit } else { p.line_strong });
    }
}

/// Add-light entries for a menu. Returns the command to apply, if one was
/// chosen.
pub fn add_light_menu(ui: &mut egui::Ui, parent: Option<NodeId>) -> Option<Command> {
    let mut chosen = None;
    for (name, shape) in [
        (
            "Area light",
            LightShape::Area {
                width: 1.0,
                height: 1.0,
                disc: false,
                two_sided: false,
            },
        ),
        (
            "Disc light",
            LightShape::Area {
                width: 1.0,
                height: 1.0,
                disc: true,
                two_sided: false,
            },
        ),
        ("Point light", LightShape::Point { radius: 0.0 }),
        (
            "Spot light",
            LightShape::Spot {
                inner_degrees: 20.0,
                outer_degrees: 35.0,
                falloff: 1.0,
            },
        ),
    ] {
        if ui.button(name).clicked() {
            chosen = Some(Command::AddLight {
                parent,
                name: shape.label().to_string(),
                settings: Box::new(LightSettings {
                    shape,
                    ..Default::default()
                }),
            });
            ui.close_menu();
        }
    }
    chosen
}

/// The properties of the selected light, if the selection is one.
pub fn light_panel(ui: &mut egui::Ui, project: &mut Project) -> bool {
    let Some(node) = project.selection.active() else {
        return false;
    };
    let Some(NodeKind::Light { settings }) = project.document().tree.get(node).map(|n| &n.kind)
    else {
        return false;
    };
    let before = (**settings).clone();
    let mut light = before.clone();

    ui.add(crate::field::Checkbox::new(&mut light.enabled, "Enabled"));

    // ---- Shape ---------------------------------------------------------
    match &mut light.shape {
        LightShape::Area {
            width,
            height,
            disc,
            two_sided,
        } => {
            ui.add(
                crate::field::Slider::new(width, 0.001..=1000.0)
                    .text("Width")
                    .logarithmic(true),
            );
            ui.add(
                crate::field::Slider::new(height, 0.001..=1000.0)
                    .text("Height")
                    .logarithmic(true),
            );
            ui.add(crate::field::Checkbox::new(disc, "Elliptical"));
            ui.add(crate::field::Checkbox::new(two_sided, "Two sided"));
        }
        LightShape::Point { radius } => {
            ui.add(
                crate::field::Slider::new(radius, 0.0..=100.0)
                    .text("Radius")
                    .logarithmic(true),
            )
            .on_hover_text("Zero is a true point, which casts a perfectly hard shadow.");
        }
        LightShape::Spot {
            inner_degrees,
            outer_degrees,
            falloff,
        } => {
            ui.add(crate::field::Slider::new(outer_degrees, 1.0..=89.0).text("Cone angle"));
            ui.add(
                crate::field::Slider::new(inner_degrees, 0.0..=*outer_degrees).text("Inner angle"),
            );
            ui.add(crate::field::Slider::new(falloff, 0.1..=8.0).text("Falloff"));
        }
    }

    // ---- Intensity ------------------------------------------------------
    ui.add_space(4.0);
    let mut amount = light.intensity.value();
    ui.horizontal(|ui| {
        let speed = amount.max(1.0) * 0.02;
        ui.add(
            egui::DragValue::new(&mut amount)
                .speed(speed)
                .range(0.0..=f32::INFINITY),
        );
        // The unit is part of the value, not a display option: 1500 lumens and
        // 1500 nits are different lights, and switching the unit must keep the
        // number the user typed rather than silently converting it into
        // something they did not ask for.
        for candidate in [
            Intensity::Lumens(amount),
            Intensity::Nits(amount),
            Intensity::Candela(amount),
        ] {
            let selected =
                std::mem::discriminant(&light.intensity) == std::mem::discriminant(&candidate);
            if ui.selectable_label(selected, candidate.unit()).clicked() {
                light.intensity = candidate;
            }
        }
    });
    light.intensity = light.intensity.with_value(amount);

    // ---- Colour ---------------------------------------------------------
    ui.add_space(4.0);
    let mut by_temperature = light.temperature.is_some();
    if ui.add(crate::field::Checkbox::new(&mut by_temperature, "Colour temperature")).changed() {
        light.temperature = by_temperature.then_some(5500.0);
    }
    match &mut light.temperature {
        Some(kelvin) => {
            ui.add(
                crate::field::Slider::new(kelvin, 1500.0..=15000.0)
                    .text("Kelvin")
                    .logarithmic(true),
            );
        }
        None => {
            crate::color_picker::field(ui, "Colour", &mut light.color);
        }
    }

    // ---- Photometry (§5.1) ----------------------------------------------
    ui.add_space(4.0);
    let mut photometric = light.photometry.is_some();
    if ui
        .add(crate::field::Checkbox::new(&mut photometric, "Photometric file"))
        .on_hover_text(
            "Use a manufacturer's measured distribution — an .ies file — instead of \
             the shape's own falloff. The shape still says where the light is and \
             which way it points.",
        )
        .changed()
    {
        light.photometry = photometric.then(|| tequila_scene::lighting::Photometry {
            path: String::new(),
        });
    }
    if let Some(photometry) = &mut light.photometry {
        ui.add(
            egui::TextEdit::singleline(&mut photometry.path)
                .desired_width(f32::INFINITY)
                .hint_text("path to a .ies file"),
        );
    }

    // ---- Visibility and layers -----------------------------------------
    crate::field::Fold::new("Visibility")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(crate::field::Checkbox::new(&mut light.visibility.camera, "Visible to camera"));
            ui.add(crate::field::Checkbox::new(&mut light.visibility.diffuse, "Affects diffuse"));
            ui.add(crate::field::Checkbox::new(&mut light.visibility.specular, "Affects specular"));
            ui.add(crate::field::Checkbox::new(&mut light.visibility.shadow, "Casts shadows"));
            let mut layer = light.layer;
            ui.add(crate::field::Slider::new(&mut layer, 0..=7).text("Light layer"))
                .on_hover_text(
                    "Layers isolate a light's contribution so it can be rescaled after the \
                     render rather than re-rendered.",
                );
            light.layer = layer;
        });

    if light != before {
        let _ = project.apply(Command::SetLight {
            node,
            settings: Box::new(light),
        });
    }
    true
}
