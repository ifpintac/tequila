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
            // What the bars say, in words, for whoever wonders.
            let response = response.on_hover_text(match speed {
                5 => "The bars are speed: this is the fastest.",
                4 => "The bars are speed: fast.",
                3 => "The bars are speed: in the middle.",
                2 => "The bars are speed: slow.",
                _ => "The bars are speed: this is the slowest.",
            });
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
            "Every setting below, set by hand.",
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

        // Folded under the presets, which set all of it; open on Custom,
        // where it is what there is to set. A section like every page's,
        // though it is this page's only one.
        let mut page = crate::sections::Page::new(ui, "lighting");
        for section in page.order() {
            match section {
                "studio" => page.section(ui, section, "Add to the studio", |ui| studio_kit(ui, project)),
                "settings" => self.settings(&mut page, ui, project, &lighting, custom),
                _ => {}
            }
        }
        page.finish(ui);
    }

    /// The Settings section: everything the presets set.
    fn settings(
        &mut self,
        page: &mut crate::sections::Page,
        ui: &mut egui::Ui,
        project: &mut Project,
        lighting: &tequila_render_core::settings::LightingSettings,
        custom: bool,
    ) {
        page.section(ui, "settings", "Settings", |ui| {
        crate::field::Fold::new("Settings")
            .id_salt("lighting-transport")
            .default_open(custom)
            .show(ui, |ui| {
                let before = lighting.config();
                let mut config = before.clone();
                let mut bounces = config.max_bounces;
                if !custom {
                    crate::theme::note(ui, "What the chosen preset sets. Changing one makes the lighting Custom.");
                }
                ui.add(crate::field::Slider::new(&mut bounces, 1..=64).text("Ray bounces"))
                    .on_hover_text("How many times light may bounce between surfaces before it is given up. More reaches deeper into corners and through more layers of glass, and takes longer.");
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

                ui.add(crate::field::Checkbox::new(&mut config.global_illumination, "Global illumination"))
                    .on_hover_text("Light bounced from one surface onto another: the soft fill in corners and the colour a red part throws on a white one.");
                ui.add(crate::field::Checkbox::new(&mut config.direct_lighting, "Direct lighting"))
                    .on_hover_text("Light that reaches a surface straight from a light or the environment.");
                ui.add(crate::field::Checkbox::new(&mut config.shadows, "Shadows"));
                ui.add(crate::field::Checkbox::new(&mut config.light_source_materials, "Light source materials"))
                    .on_hover_text("Parts with a glowing material light the parts around them, as a lamp would.");
                ui.add(crate::field::Checkbox::new(&mut config.ground_illumination, "Ground illumination"))
                    .on_hover_text("Let the ground plane bounce light back into the scene.");
                // Off, or one of the two ways of finding caustics: the
                // setting is the switch and the method together.
                let mut caustics = config.caustics.then_some(config.caustics_method);
                crate::field::row(ui, "Caustics", |ui| {
                    crate::field::combo(ui, "lighting-caustics", caustics.map_or("Off", CausticsMethod::label), |ui| {
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
                                 surface: sharp where it finds the path, but slower and noisier than \
                                 photons, and short of light where several paths through the glass \
                                 meet, as in a bottle's caustic. Lamps only, through glass that bends \
                                 light; the environment's light through glass is found by bounces, and \
                                 mirrors focus nothing.",
                            );
                    });
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
                ui.add(crate::field::Checkbox::new(&mut config.interior_mode, "Interior mode"))
                    .on_hover_text("For a room lit through its windows: looks harder for the light that finds its way in. Slower, much cleaner.");
                ui.add(
                    crate::field::Slider::new(&mut config.regularization_roughness, 0.0..=0.4)
                        .text("Roughness floor"),
                )
                .on_hover_text("Treats very shiny surfaces as a little rougher after the first bounce, which takes the sparkle of noise out of their reflections. 0 is exact.");

                let unset = tequila_render_core::settings::CyclesTransport::NONE;
                crate::field::Fold::new("Advanced (Cycles)")
                    .id_salt("lighting-cycles-light-paths")
                    .default_open(config.cycles != unset)
                    .show(ui, |ui| {
                        crate::theme::note(
                            ui,
                            "Blender's Light Paths, for a studio that must match one. Unticked, Cycles takes \
                             Tequila's own; with diffuse or glossy set, Ray bounces is the total.",
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
                        .on_hover_text("Blur glossy reflections seen after a diffuse bounce, which cuts their noise but darkens what is seen through glass. Blender's is 0.5; unticked, none.");
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
        });
    }
}

/// The photographer's kit, a piece a click (`tequila_scene::studio_kit`):
/// sized to the product, placed for the camera, one step to undo, and
/// selected so it can be moved or aimed at once.
fn studio_kit(ui: &mut egui::Ui, project: &mut Project) {
    use tequila_scene::studio_kit::Piece;
    crate::field::Fold::new("Add to the studio").id_salt("lighting-kit").default_open(true).show(ui, |ui| {
        crate::theme::note(
            ui,
            "Real lights and cards beside the product, sized to it and placed for the camera. Move one, or aim it at \
             a highlight from its own page.",
        );
        ui.horizontal_wrapped(|ui| {
            for piece in Piece::ALL {
                if ui.button(piece.label()).on_hover_text(piece.description()).clicked() {
                    // What went wrong is in the status line.
                    let _ = add_to_studio(project, piece);
                }
            }
        });
    });
}

/// Add `piece` to the studio as one step, and select it. The node it
/// added, or what went wrong.
pub fn add_to_studio(project: &mut Project, piece: tequila_scene::studio_kit::Piece) -> Result<NodeId, String> {
    let eye = project.document().cameras.active().camera.position();
    let (commands, node) = tequila_scene::studio_kit::add(project.document(), piece, eye);
    project.begin_group();
    let mut failed = None;
    for command in commands {
        if let Err(error) = project.apply(command) {
            failed = Some(error);
            break;
        }
    }
    match failed {
        None => {
            project.end_group();
            project.selection.select_many([node], tequila_scene::selection::SelectionMode::Replace);
            project.status = format!("Added a {}", piece.label().to_lowercase());
            Ok(node)
        }
        Some(error) => {
            project.abandon_group();
            project.status = format!("Could not add the {}: {error}", piece.label().to_lowercase());
            Err(project.status.clone())
        }
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
pub fn light_panel(
    ui: &mut egui::Ui,
    project: &mut Project,
    aim: &mut crate::pin_gizmo::PinGizmo,
    templates: &mut dyn FnMut(&mut egui::Ui, &mut Project),
) -> bool {
    let Some(node) = project.selection.active() else {
        return false;
    };
    let Some(NodeKind::Light { settings }) = project.document().tree.get(node).map(|n| &n.kind)
    else {
        return false;
    };
    let before = (**settings).clone();
    let mut light = before.clone();

    crate::theme::section(ui, "Light");
    ui.add(crate::field::Checkbox::new(&mut light.enabled, "On"));
    hide_in_final_render(ui, &mut light);
    // Where a photographer starts: the highlight, not the numbers.
    let aiming = aim.armed_light() == Some(node);
    if ui
        .add(egui::Button::new(if aiming { "Aiming: click the model" } else { "Aim at a highlight" }).selected(aiming))
        .on_hover_text(
            "Click the model where you want this light's highlight and the light moves there, keeping its \
             distance and turning to face the spot. Escape when done.",
        )
        .clicked()
    {
        aim.toggle_light(node);
    }
    ui.add_space(2.0);

    // The sections, in the person's own order.
    let mut page = crate::sections::Page::new(ui, "light");
    for section in page.order() {
        match section {
            "shape" => page.section(ui, section, "Shape", |ui| {
                crate::field::Fold::new("Shape").id_salt("light-shape").aside(light.shape.label()).default_open(true).show(ui, |ui| {
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
                            ui.add(crate::field::Slider::new(&mut light.edge_falloff, 0.0..=1.0).text("Edge falloff"))
                                .on_hover_text("How much darker its edges are than its middle, as a softbox's front is: none is a bare panel, an even white shape with a hard edge in gloss paint and chrome; about half is a softbox, a sheen that fades towards its edges. The light gives the same, its middle brighter.");
                            ui.add(crate::field::Slider::new(&mut light.fade, -1.0..=1.0).text("Fade"))
                                .on_hover_text("Fades it from its top edge to its bottom, as a graduated scrim does: 1 leaves the bottom edge dark, -1 the top. What gives a bottle or a curved face a highlight that melts away at one end. The light gives the same.");
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

                });
            }),
            "brightness" => page.section(ui, section, "Brightness", |ui| {
                crate::field::Fold::new("Brightness").id_salt("light-brightness").default_open(true).show(ui, |ui| {
                    let mut amount = light.intensity.value();
                    crate::field::row(ui, "Amount", |ui| {
                        let speed = amount.max(1.0) * 0.02;
                        ui.add_sized(
                            [ui.available_width().min(120.0), 26.0],
                            egui::DragValue::new(&mut amount)
                                .speed(speed)
                                .range(0.0..=f32::INFINITY)
                                .suffix(format!(" {}", light.intensity.unit())),
                        );
                    });
                    // The unit is part of the value, not a display option: 1500 lumens and
                    // 1500 nits are different lights, and switching the unit must keep the
                    // number the user typed rather than silently converting it into
                    // something they did not ask for.
                    let units = [
                        Intensity::Lumens(amount),
                        Intensity::Nits(amount),
                        Intensity::Candela(amount),
                        Intensity::Lux(amount),
                    ];
                    let mut unit = units.iter().position(|c| std::mem::discriminant(c) == std::mem::discriminant(&light.intensity)).unwrap_or(0);
                    let names: Vec<(usize, &str)> = units.iter().enumerate().map(|(i, c)| (i, c.unit())).collect();
                    if crate::field::choice(ui, "Measured in", &mut unit, &names) {
                        light.intensity = units[unit];
                    }
                    light.intensity = light.intensity.with_value(amount);

                    // ---- Photometry (§5.1) ----------------------------------------------
                    // Not for a sun, which is not a fitting.
                    let mut photometric = light.photometry.is_some();
                    if !matches!(light.shape, LightShape::Sun { .. }) && ui
                        .add(crate::field::Checkbox::new(&mut photometric, "Use a maker's light file (.ies)"))
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
                        let mut browse = false;
                        crate::field::row(ui, "File", |ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                browse = ui.button("Choose…").clicked();
                                ui.add(
                                    egui::TextEdit::singleline(&mut photometry.path)
                                        .desired_width(f32::INFINITY)
                                        .hint_text(egui::RichText::new("No file").color(crate::theme::p().text_faint)),
                                );
                            });
                        });
                        if browse {
                            if let Some(file) = rfd::FileDialog::new().add_filter("Photometric file", &["ies"]).set_title("A light's measured spread").pick_file() {
                                photometry.path = file.display().to_string();
                            }
                        }
                    }

                });
            }),
            "colour" => page.section(ui, section, "Colour", |ui| {
                crate::field::Fold::new("Colour").id_salt("light-colour").default_open(true).show(ui, |ui| {
                    let mut by_temperature = light.temperature.is_some();
                    if crate::field::choice(ui, "Set by", &mut by_temperature, &[(false, "Colour"), (true, "Temperature")]) {
                        light.temperature = by_temperature.then_some(5500.0);
                    }
                    match &mut light.temperature {
                        Some(kelvin) => {
                            ui.add(
                                crate::field::Slider::new(kelvin, 1500.0..=15000.0)
                                    .text("Temperature")
                                    .suffix(" K")
                                    .fixed_decimals(0)
                                    .logarithmic(true),
                            )
                            .on_hover_text("Warm candlelight is about 1900 K, a household bulb 2700 K, daylight 5500 K, an overcast sky 7000 K.");
                        }
                        None => {
                            crate::color_picker::field(ui, "Colour", &mut light.color);
                        }
                    }

                });
            }),
            "visibility" => page.section(ui, section, "Visibility", |ui| {
                crate::field::Fold::new("Visibility")
                    .default_open(false)
                    .show(ui, |ui| {
                        // Whether the camera sees it is *Hide in final render*,
                        // at the top of the page.
                        ui.add(crate::field::Checkbox::new(&mut light.visibility.diffuse, "Affects diffuse"));
                        ui.add(crate::field::Checkbox::new(&mut light.visibility.specular, "Seen in reflections"))
                            .on_hover_text(SEEN_IN_REFLECTIONS);
                        ui.add(crate::field::Checkbox::new(&mut light.visibility.shadow, "Casts shadows"));
                        let mut layer = light.layer;
                        ui.add(crate::field::Slider::new(&mut layer, 0..=7).text("Light layer"))
                            .on_hover_text(
                                "Layers isolate a light's contribution so it can be rescaled after the \
                                 render rather than re-rendered.",
                            );
                        light.layer = layer;
                    });

            }),
            "light_linking" => page.section(ui, section, "Light linking", |ui| {
                linking_fold(
                    ui,
                    project,
                    ("Light linking", "light-linking"),
                    &mut light.links,
                    "Lights every part, and shadows them all.",
                    "What it does not light it does not shadow: a light kept to the product casts no shadow on the floor.",
                );
            }),
            "shadow_linking" => page.section(ui, section, "Shadow linking", |ui| {
                linking_fold(
                    ui,
                    project,
                    ("Shadow linking", "shadow-linking"),
                    &mut light.shadow_links,
                    "Every part casts its shadow.",
                    "The parts that cast its shadow. The rest let its light through, so a prop can stand in its beam without shadowing the product.",
                );
            }),
            "templates" => page.section(ui, section, "Material templates", |ui| templates(ui, project)),
            _ => {}
        }
    }
    page.finish(ui);

    if light != before {
        let _ = project.apply(Command::SetLight {
            node,
            settings: Box::new(light),
        });
    }
    true
}

/// What *Seen in reflections* says when pointed at.
const SEEN_IN_REFLECTIONS: &str =
    "Its reflection in shiny surfaces, and with it the highlight it gives them: a light's highlight is its reflection. Off, it lights by diffuse light alone.";

/// *Hide in final render*: the light still lights the scene and gives the
/// product its highlights, and its shape is not seen by the camera - the community's
/// request beside the viewport's light outlines, which still show where it
/// is. It is the light's camera visibility (`Visibility::camera`, the file's
/// `visible_to_camera`), which every renderer already honours.
///
/// Camera only, not reflections too: a softbox stands beside a product for
/// its reflection, the highlight down a bottle or along a car's flank, and a
/// light hidden from reflections gives no highlight at all. What is wanted
/// out of the picture is the panel standing in the frame. Reflections are
/// the Visibility section's *Seen in reflections*, as KeyShot keeps the two
/// apart.
///
/// Only a panel has a shape the camera sees (`docs/01-RENDERER-CORE.md`
/// §5.4: "an area lamp; the camera sees no sun"): on a point, a bulb, a
/// spot and the sun the box is greyed out, as a box that changes nothing
/// reads as broken.
fn hide_in_final_render(ui: &mut egui::Ui, light: &mut LightSettings) {
    let has_shape = matches!(light.shape, LightShape::Area { .. });
    let mut hidden = !light.visibility.camera;
    let response = ui
        .add_enabled(has_shape, crate::field::Checkbox::new(&mut hidden, "Hide in final render"))
        .on_hover_text(HIDE_IN_FINAL_RENDER)
        .on_disabled_hover_text(
            "Only a panel or a disc is seen by the camera: a point, a bulb, a spot and the sun light the scene with no shape in the picture.",
        );
    if response.changed() {
        light.visibility.camera = !hidden;
    }
}

/// What *Hide in final render* says when pointed at.
const HIDE_IN_FINAL_RENDER: &str = "The camera does not see the light itself, in the Rendered view or the final render. It still lights the scene, and the highlights it gives the product stay. Its outline in the viewport still shows where it is.";

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
                crate::theme::note(ui, everything);
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
                crate::theme::note(ui, format!("and {deeper} parts set by a script"));
            }
            crate::theme::note(ui, note);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_scene::studio_kit::Piece;

    /// Each piece of the kit comes in as one step to undo, selected, placed
    /// off the product facing it: a light for the softbox, the strip and
    /// the card, a part the camera does not see for the flag.
    #[test]
    fn each_piece_of_the_kit_is_one_step_selected_and_facing_the_product() {
        for piece in Piece::ALL {
            let mut project = Project::with_document(
                tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3),
                std::sync::Arc::new(tequila_scene::demo::studio_environment()),
            );
            let nodes = project.document().tree.iter().count();
            add_to_studio(&mut project, piece).expect("added");
            let node = project.selection.active().unwrap_or_else(|| panic!("{piece:?} was not selected"));
            let document = project.document();
            let added = document.tree.get(node).expect("the node");
            match piece {
                Piece::Flag => {
                    assert!(matches!(added.kind, NodeKind::Part { .. }), "the flag is a part");
                    assert!(!added.rays.camera && added.rays.shadow && added.rays.glossy, "the flag: {:?}", added.rays);
                }
                _ => {
                    let NodeKind::Light { settings } = &added.kind else { panic!("{piece:?} is not a light") };
                    assert!(settings.edge_falloff > 0.0 && !settings.visibility.camera, "{piece:?}: {settings:?}");
                }
            }
            let middle = tequila_scene::snapshot::visible_bounds(document).center();
            let world = document.placements().world(node);
            let at = world.point(tequila_core::math::Vec3::ZERO);
            assert!((at - middle).length() > 0.01, "{piece:?} sits on the product");
            project.undo();
            assert_eq!(project.document().tree.iter().count(), nodes, "{piece:?}: one undo takes it all back");
        }
    }

    /// Ticked, *Hide in final render* is the light's camera visibility off,
    /// and it stays so through a save and a load, the light beside it still
    /// seen.
    #[test]
    fn a_light_hidden_in_the_final_render_stays_hidden_after_a_save_and_a_load() {
        let folder = std::env::temp_dir().join(format!("tequila-hidden-light-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("hidden.tequila");
        let mut project = Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3),
            std::sync::Arc::new(tequila_scene::demo::studio_environment()),
        );
        for name in ["Hidden", "Seen"] {
            project
                .apply(Command::AddLight { parent: None, name: name.into(), settings: Box::new(LightSettings::default()) })
                .expect("added");
        }
        let light = |project: &Project, name: &str| {
            let node = project.document().tree.iter().find(|n| n.name == name).expect("the light");
            match &node.kind {
                NodeKind::Light { settings } => (node.id, (**settings).clone()),
                _ => panic!("{name} is not a light"),
            }
        };
        let (node, before) = light(&project, "Hidden");
        assert!(before.visibility.camera, "a new light is seen, as today");
        let hidden = LightSettings { visibility: tequila_render_core::light::Visibility { camera: false, ..before.visibility }, ..before };
        project.apply(Command::SetLight { node, settings: Box::new(hidden.clone()) }).expect("hidden");
        project.save(&file).expect("saved");

        let mut opened = Project::with_document(tequila_scene::SceneDocument::new(), std::sync::Arc::new(tequila_scene::demo::studio_environment()));
        opened.open(&file).expect("opened");
        assert_eq!(light(&opened, "Hidden").1, hidden, "the light came back changed");
        let seen = light(&opened, "Seen").1;
        assert!(seen.visibility.camera && seen.visibility.specular, "the other light was hidden too: {:?}", seen.visibility);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A panel in the picture, before a ball it lights, rendered by
    /// Tequila's renderer and by Cycles: hidden in the final render, where
    /// the panel was shows what is behind it, as with the light off, and the
    /// ball is lit as it was. Cycles is skipped where it is not installed.
    #[test]
    fn a_light_hidden_in_the_final_render_still_lights_the_scene_in_both_renderers() {
        use tequila_core::math::{vec3, Vec3};
        use tequila_scene::render::RenderEngine;
        let mut engines = vec![RenderEngine::Tequila];
        match tequila_render_cycles::available() {
            Ok(()) => engines.push(RenderEngine::CyclesCpu),
            Err(why) => eprintln!("Cycles skipped: {why}"),
        }
        let (width, height) = (160usize, 100usize);
        let camera = tequila_render_core::camera::Camera::look_at(vec3(0.0, 1.2, 7.0), vec3(0.0, 1.0, 0.0), Vec3::Y);
        let panel = vec3(-1.6, 2.1, 0.5);
        let middle = vec3(0.0, 1.0, 0.0);
        // The ball's face towards the panel, which the camera sees.
        let lit = middle + (panel - middle).normalize();
        let mean_round = |image: &[u8], point: Vec3| {
            let film = camera.project(point, width as f32 / height as f32).expect("in view");
            let (x, y) = ((film.x * width as f32) as usize, (film.y * height as f32) as usize);
            let mut sum = 0.0;
            for py in y - 2..=y + 2 {
                for px in x - 2..=x + 2 {
                    let i = (py * width + px) * 4;
                    sum += 0.2126 * f64::from(image[i]) + 0.7152 * f64::from(image[i + 1]) + 0.0722 * f64::from(image[i + 2]);
                }
            }
            sum / 25.0
        };
        for engine in engines {
            let [seen, hidden, off] = [(true, true), (true, false), (false, true)].map(|(on, camera_sees)| {
                let mut project = panel_before_a_ball(panel, on, camera_sees);
                render(&mut project, engine, camera, (width, height), &format!("{}-{on}-{camera_sees}", engine.key()))
            });
            let at_panel = [&seen, &hidden, &off].map(|image| mean_round(image, panel));
            let on_ball = [&seen, &hidden, &off].map(|image| mean_round(image, lit));
            println!("{}: where the panel is {at_panel:.1?}, the ball's lit face {on_ball:.1?} (seen, hidden, off)", engine.label());
            assert!(at_panel[0] > at_panel[1] + 60.0, "{}: the panel was not seen, or not hidden: {at_panel:.1?}", engine.label());
            assert!((at_panel[1] - at_panel[2]).abs() < 8.0, "{}: hidden, the panel's place is not what is behind it: {at_panel:.1?}", engine.label());
            assert!((on_ball[0] - on_ball[1]).abs() < (on_ball[0] * 0.06).max(6.0), "{}: hiding the panel changed its light: {on_ball:.1?}", engine.label());
            assert!(on_ball[1] > on_ball[2] + 20.0, "{}: hidden, the panel lights nothing: {on_ball:.1?}", engine.label());
        }
    }

    /// The demo's ball, a metre round, in a dim even light, and a 0.8 m
    /// panel at `at`, `on` or off, `seen` by the camera or not, facing a
    /// point between the ball and the camera: the camera sees the side that
    /// shines, and so does the ball.
    fn panel_before_a_ball(at: tequila_core::math::Vec3, on: bool, seen: bool) -> Project {
        use tequila_core::math::{vec3, Mat4, Transform, Vec3};
        let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Diffuse, 1.0);
        let environment = tequila_render_core::environment::Environment::uniform(tequila_core::spectrum::Spectrum::splat(0.03));
        let mut project = Project::with_document(document, std::sync::Arc::new(environment));
        let settings = LightSettings {
            shape: LightShape::Area { width: 0.8, height: 0.8, disc: false, two_sided: false },
            // Bright beside the room's own light, so what it gives the ball
            // stands well clear of the noise.
            intensity: Intensity::Nits(20.0),
            enabled: on,
            visibility: tequila_render_core::light::Visibility { camera: seen, ..Default::default() },
            ..LightSettings::default()
        };
        project.apply(Command::AddLight { parent: None, name: "Panel".into(), settings: Box::new(settings) }).expect("added");
        let node = project.document().tree.iter().find(|n| n.name == "Panel").expect("the panel").id;
        let towards = vec3(0.0, 1.0, 1.5);
        let facing = Mat4::look_at(at, at + (at - towards), Vec3::Y);
        project.apply(Command::set_transform(node, Transform::from_matrix(facing).expect("invertible"))).expect("placed");
        project
    }

    /// `project` rendered by `engine` from `camera`, as the Render window
    /// renders it: the developed picture.
    fn render(
        project: &mut Project,
        engine: tequila_scene::render::RenderEngine,
        camera: tequila_render_core::camera::Camera,
        (width, height): (usize, usize),
        name: &str,
    ) -> Vec<u8> {
        use std::time::{Duration, Instant};
        let folder = std::env::temp_dir().join(format!("tequila-hide-in-final-render-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let settings = tequila_scene::render::RenderSettings {
            engine,
            width,
            height,
            max_samples: 32,
            max_seconds: None,
            denoise: false,
            adaptive: false,
            filename_template: name.into(),
            output_directory: folder.display().to_string(),
            ..project.render_settings().clone()
        };
        let saved = tequila_scene::lighting::SavedCamera { name: "Test".into(), camera, target: tequila_core::math::vec3(0.0, 1.0, 0.0) };
        let id = project.enqueue_camera(name, saved, settings);
        let job = project.take_queued_snapshot(id).expect("the render's job");
        let renderer = crate::batch::BatchRenderer::new();
        renderer.submit(job);
        let deadline = Instant::now() + Duration::from_secs(900);
        loop {
            for event in renderer.drain() {
                match event {
                    crate::batch::BatchEvent::Finished { preview, path, .. } => {
                        println!("{name}: {path}");
                        return preview;
                    }
                    crate::batch::BatchEvent::Failed { message, .. } => panic!("{name}: {message}"),
                    _ => {}
                }
            }
            assert!(Instant::now() < deadline, "{name} never finished");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
