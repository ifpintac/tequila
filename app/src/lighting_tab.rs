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
use tequila_render_core::settings::{CausticsMethod, LightingPreset};
use tequila_scene::lighting::{Intensity, LightSettings, LightShape};
use tequila_scene::tree::{NodeId, NodeKind};
use tequila_scene::Command;

use crate::project::Project;

#[derive(Default)]
pub struct LightingTab {}

/// A count Cycles takes that may be left to it: ticked, the number given;
/// unticked, `automatic`, shown greyed out as what it will be.
fn optional_count(ui: &mut egui::Ui, label: &str, value: &mut Option<u32>, automatic: u32) {
    ui.horizontal(|ui| {
        let mut set = value.is_some();
        if ui.add(crate::field::Checkbox::new(&mut set, label)).changed() {
            *value = set.then_some(automatic);
        }
        match value {
            Some(count) => {
                ui.add(egui::DragValue::new(count).range(0..=128));
            }
            None => {
                ui.label(egui::RichText::new(automatic.to_string()).color(crate::theme::dim()));
            }
        }
    });
}

/// A switch Cycles takes that may be left to it (on, there).
fn optional_flag(ui: &mut egui::Ui, label: &str, value: &mut Option<bool>) {
    let mut on = value.unwrap_or(true);
    if ui.add(crate::field::Checkbox::new(&mut on, label)).changed() {
        *value = Some(on);
    }
}

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
                // Off, or one of the two ways of finding caustics: the
                // setting is the switch and the method together.
                let mut caustics = config.caustics.then_some(config.caustics_method);
                egui::ComboBox::from_label("Caustics")
                    .selected_text(caustics.map_or("Off", CausticsMethod::label))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut caustics, None, "Off").on_hover_text(
                            "Light passes clear glass straight on, tinted, onto what is behind it, \
                             and is focused nowhere.",
                        );
                        ui.selectable_value(&mut caustics, Some(CausticsMethod::Photons), CausticsMethod::Photons.label())
                            .on_hover_text(
                                "Photons from every light, the environment's included, through glass, \
                                 liquids and gems and off mirrors: soft at first, sharper the longer \
                                 it renders. Jewelry uses these.",
                            );
                        ui.selectable_value(&mut caustics, Some(CausticsMethod::Manifold), CausticsMethod::Manifold.label())
                            .on_hover_text(
                                "The path from each lamp through smooth glass, solved for at every \
                                 surface: caustics sharp from the first samples, each sample slower. \
                                 Lamps only, through glass that bends light; the environment's light \
                                 through glass is found by bounces, and mirrors focus nothing.",
                            );
                    })
                    .response
                    .on_hover_text(
                        "The bright patterns glass, liquids and gems focus under and beside \
                         themselves. Needs solid glass (Glass Solid, Liquid, Gem); a thin pane \
                         passes light straight through.",
                    );
                config.caustics = caustics.is_some();
                if let Some(method) = caustics {
                    config.caustics_method = method;
                }
                ui.add(crate::field::Checkbox::new(&mut config.interior_mode, "Interior mode"));
                ui.add(
                    crate::field::Slider::new(&mut config.regularization_roughness, 0.0..=0.4)
                        .text("Roughness floor"),
                );

                let unset = tequila_render_core::settings::CyclesTransport::NONE;
                crate::field::Fold::new("Advanced (Cycles)")
                    .id_salt("lighting-cycles-light-paths")
                    .default_open(config.cycles != unset)
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Blender's Light Paths, for a studio that must match one. Unticked, Cycles takes \
                                 what it always has; with diffuse or glossy set, Ray bounces is the total.",
                            )
                            .small()
                            .color(crate::theme::dim()),
                        );
                        let automatic = config.max_bounces.max(4);
                        optional_count(ui, "Diffuse bounces", &mut config.cycles.diffuse_bounces, automatic);
                        optional_count(ui, "Glossy bounces", &mut config.cycles.glossy_bounces, automatic);
                        optional_count(ui, "Transparent bounces", &mut config.cycles.transparent_bounces, 8);
                        optional_count(ui, "Volume bounces", &mut config.cycles.volume_bounces, 7);
                        ui.horizontal(|ui| {
                            let mut set = config.cycles.filter_glossy.is_some();
                            if ui.add(crate::field::Checkbox::new(&mut set, "Filter glossy")).changed() {
                                config.cycles.filter_glossy = set.then_some(1.0);
                            }
                            if let Some(blur) = &mut config.cycles.filter_glossy {
                                ui.add(egui::DragValue::new(blur).range(0.0..=10.0).speed(0.05));
                            }
                        })
                        .response
                        .on_hover_text("Blur glossy reflections seen after a diffuse bounce, which cuts their noise. Blender's is 0.5; unticked, 1.");
                        optional_flag(ui, "Reflective caustics", &mut config.cycles.caustics_reflective);
                        optional_flag(ui, "Refractive caustics", &mut config.cycles.caustics_refractive);
                    });

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
        // The real sun's half a degree.
        ("Sun", LightShape::Sun { angle_degrees: 0.53 }),
    ] {
        if ui.button(name).clicked() {
            // A sun is given the light falling on things, in lux; the rest
            // the light they give, in lumens.
            let intensity = match shape {
                LightShape::Sun { .. } => Intensity::Lux(500.0),
                _ => LightSettings::default().intensity,
            };
            chosen = Some(Command::AddLight {
                parent,
                name: shape.label().to_string(),
                settings: Box::new(LightSettings {
                    shape,
                    intensity,
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
            ui.add(crate::field::Slider::new(&mut light.spread_degrees, 1.0..=180.0).text("Spread"))
                .on_hover_text("How wide the light leaves the panel: 180 is a plain panel, less a softbox behind a grid, its light gathered into a narrower beam of the same power.");
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
        LightShape::Sun { angle_degrees } => {
            ui.add(crate::field::Slider::new(angle_degrees, 0.0..=30.0).text("Angle"))
                .on_hover_text("How wide the sun's disc is. Zero casts perfectly hard shadows; the real sun is half a degree. Where the light is does not matter, only which way it points.");
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
            Intensity::Lux(amount),
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
    // Not for a sun, which is not a fitting.
    ui.add_space(4.0);
    let mut photometric = light.photometry.is_some();
    if !matches!(light.shape, LightShape::Sun { .. }) && ui
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

    // ---- Light and shadow linking -------------------------------------------
    linking_fold(
        ui,
        project,
        ("Light linking", "light-linking"),
        &mut light.links,
        "Lights every part, and shadows them all.",
        "What it does not light it does not shadow: a light kept to the product casts no shadow on the floor.",
    );
    linking_fold(
        ui,
        project,
        ("Shadow linking", "shadow-linking"),
        &mut light.shadow_links,
        "Every part casts its shadow.",
        "The parts that cast its shadow. The rest let its light through, so a prop can stand in its beam without shadowing the product.",
    );

    if light != before {
        let _ = project.apply(Command::SetLight {
            node,
            settings: Box::new(light),
        });
    }
    true
}

/// A light's links, for its light or for its shadow: everything, only
/// some parts, or all but some. `title` is the fold's name and id.
fn linking_fold(
    ui: &mut egui::Ui,
    project: &Project,
    (title, salt): (&str, &str),
    links: &mut Option<tequila_scene::lighting::LightLinking>,
    everything: &str,
    note: &str,
) {
    crate::field::Fold::new(title)
        .id_salt(salt)
        .default_open(links.is_some())
        .show(ui, |ui| {
            use tequila_scene::lighting::LightLinking;
            let mut mode = match &*links {
                None => 0u8,
                Some(links) if links.exclude => 2,
                Some(_) => 1,
            };
            if crate::theme::segmented(ui, &mut mode, &[(0, "Everything"), (1, "Only"), (2, "All but")]) {
                *links = match mode {
                    0 => None,
                    _ => Some(LightLinking {
                        exclude: mode == 2,
                        ..links.clone().unwrap_or(LightLinking { product: true, ..LightLinking::default() })
                    }),
                };
            }
            let Some(links) = links else {
                ui.label(egui::RichText::new(everything).small().color(crate::theme::dim()));
                return;
            };
            ui.add(crate::field::Checkbox::new(&mut links.product, "The product"))
                .on_hover_text("Everything in the scene but its lights. What a studio preset keeps, since it cannot name a product's parts.");
            let document = project.document();
            let roots = tequila_scene::studio::product_roots(document);
            if !links.product {
                for &root in &roots {
                    let name = document.tree.get(root).map(|n| n.name.clone()).unwrap_or_default();
                    let mut on = links.nodes.contains(&root);
                    if ui.add(crate::field::Checkbox::new(&mut on, name)).changed() {
                        if on {
                            links.nodes.push(root);
                        } else {
                            links.nodes.retain(|&n| n != root);
                        }
                    }
                }
            }
            let deeper = links.nodes.iter().filter(|n| !roots.contains(n)).count();
            if deeper > 0 {
                ui.label(egui::RichText::new(format!("and {deeper} parts set by a script")).small().color(crate::theme::dim()));
            }
            ui.label(egui::RichText::new(note).small().color(crate::theme::dim()));
        });
}
