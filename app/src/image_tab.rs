//! The Image tab (`TEQUILA_SPEC.md` §1.9).
//!
//! Everything here is free. `docs/01` §9.3 classifies image style, tone map and
//! exposure as needing "no re-render at all" — the accumulation buffer is
//! re-developed and the samples are kept. That is why these controls live on
//! their own tab rather than beside the render settings: dragging exposure on a
//! render that has been converging for ten minutes must not throw those ten
//! minutes away, and the only way to be sure of that is for the control never
//! to reach the render request at all.

use eframe::egui;
use tequila_color::{DisplayTransform, ImageStyle, ToneMode, NEUTRAL_KELVIN};
use tequila_scene::Command;

use crate::project::Project;

#[derive(Default)]
pub struct ImageTab {
    /// A config path being typed. Held while the field has focus, so a
    /// half-typed path is never handed to the loader.
    typing: Option<String>,
    /// The displays and views of the config that was last read, so the two
    /// selectors can offer what the config actually contains rather than
    /// asking the user to type names.
    known: Option<(String, Vec<(String, Vec<String>)>)>,
    /// The config path last read into `known`, successfully or not, so a
    /// config that will not read is not re-read on every frame.
    read: Option<String>,
}

impl ImageTab {
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        let before = project.image_style();
        let mut style = before;

        // Reset at the heading's right, where it is found before the
        // controls it clears and is not one more row among them.
        let reset = crate::theme::heading_with_first(ui, "Adjust", |ui| {
            ui.add(egui::Button::new("Reset").small())
                .on_hover_text("Put every adjustment on this page back, keeping the tone map")
                .clicked()
        });
        let modes: Vec<(ToneMode, &str)> = ToneMode::ALL.iter().map(|m| (*m, m.label())).collect();
        crate::theme::segmented(ui, &mut style.mode, &modes);
        ui.add_space(2.0);

        ui.add(crate::field::Slider::new(&mut style.exposure, -6.0..=6.0).text("Exposure"))
            .on_hover_text("Brighter or darker, in stops: +1 is twice the light");
        ui.add(crate::field::Slider::new(&mut style.contrast, 0.4..=2.5).text("Contrast"));
        ui.add(crate::field::Slider::new(&mut style.saturation, 0.0..=2.0).text("Saturation"));

        if style.mode == ToneMode::Photographic {
            crate::theme::heading(ui, "Tones");
            ui.add(crate::field::Slider::new(&mut style.highlights, -1.0..=1.0).text("Highlights"));
            ui.add(crate::field::Slider::new(&mut style.shadows, -1.0..=1.0).text("Shadows"));
            ui.add(crate::field::Slider::new(&mut style.whites, -1.0..=1.0).text("Whites"));
            ui.add(crate::field::Slider::new(&mut style.blacks, -1.0..=1.0).text("Blacks"));

            let neutral = crate::theme::heading_with(ui, "White balance", |ui| {
                ui.add(egui::Button::new("Neutral").small())
                    .on_hover_text("Daylight, with no tint")
                    .clicked()
            });
            ui.add(
                crate::field::Slider::new(&mut style.temperature, 1800.0..=15000.0)
                    .text("Temperature")
                    .suffix(" K")
                    .fixed_decimals(0)
                    .logarithmic(true),
            );
            ui.add(crate::field::Slider::new(&mut style.tint, -1.0..=1.0).text("Tint"));
            if neutral {
                style.temperature = NEUTRAL_KELVIN;
                style.tint = 0.0;
            }

            crate::theme::heading(ui, "Lens effects");
            ui.add(crate::field::Slider::new(&mut style.vignette, 0.0..=1.0).text("Vignette"))
                .on_hover_text("Darkens the picture's corners, as a lens does");
            ui.add(crate::field::Slider::new(&mut style.bloom, 0.0..=1.0).text("Bloom"));
            if style.bloom > 0.0 {
                ui.add(
                    crate::field::Slider::new(&mut style.bloom_radius, 0.002..=0.2)
                        .text("Bloom radius")
                        .logarithmic(true),
                );
            }
            ui.add(crate::field::Slider::new(&mut style.sharpening, 0.0..=1.0).text("Sharpening"));
            ui.add(
                crate::field::Slider::new(&mut style.chromatic_aberration, 0.0..=1.0)
                    .text("Chromatic aberration"),
            );
        }

        crate::theme::heading(ui, "Output");
        crate::field::row(ui, "Tone map", |ui| {
            crate::field::combo(ui, "image-tone-map", style.display.label(), |ui| {
                for transform in DisplayTransform::ALL {
                    ui.selectable_value(&mut style.display, transform, transform.label());
                }
            });
        })
        .response
        .on_hover_text("How the render's light is fitted to a screen. PBR Neutral keeps colours as the materials set them; AgX and Filmic roll bright areas off like film; sRGB is the plain curve other programs' Standard view uses.");

        self.color_management(ui, project);

        if reset {
            // The display transform survives a reset: it is a choice about the
            // *output device and pipeline*, not part of the look, and having it
            // snap back to AgX every time the grade is cleared would be an
            // unrelated change the user did not ask for.
            style = ImageStyle {
                display: style.display,
                ..ImageStyle::default()
            };
        }

        crate::theme::note(ui, "Changing these never restarts the render: they work on the picture it has made.");

        if style != before {
            let _ = project.apply(Command::SetImageStyle {
                style: Box::new(style),
            });
        }
    }

    /// The OCIO selector (§01 §7).
    ///
    /// Off by default and folded away, because the built-in transforms are
    /// what most scenes want and a config is something a studio brings. When
    /// one is loaded the display and view are *chosen from what it contains*
    /// rather than typed — a mistyped view name would silently develop through
    /// the wrong look, and the config already knows its own names.
    fn color_management(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        crate::field::Fold::new("Colour management")
            .default_open(false)
            .show(ui, |ui| {
                let current = project.document().ocio.clone();
                let mut path = match (&self.typing, &current) {
                    (Some(typing), _) => typing.clone(),
                    (None, Some(selection)) => selection.config.clone(),
                    (None, None) => String::new(),
                };

                crate::theme::note(
                    ui,
                    "For a studio with its own colour pipeline: point at its OpenColorIO config \
                     and the picture is developed through the same transforms.",
                );
                let mut browse = false;
                let response = crate::field::row(ui, "Config", |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        browse = ui.button("Choose…").clicked();
                        ui.add(
                            egui::TextEdit::singleline(&mut path)
                                .desired_width(f32::INFINITY)
                                .hint_text(egui::RichText::new("No config").color(crate::theme::p().text_faint)),
                        )
                    })
                    .inner
                })
                .inner;
                if browse {
                    if let Some(file) = rfd::FileDialog::new().add_filter("OpenColorIO config", &["ocio"]).set_title("Colour config").pick_file() {
                        self.typing = None;
                        self.load(&file.display().to_string(), project);
                        return;
                    }
                }
                if response.changed() {
                    self.typing = Some(path.clone());
                }
                if response.lost_focus() {
                    self.typing = None;
                    self.load(path.trim(), project);
                }

                // A selection the document arrived with — a project saved with
                // a config — has not been through `load`, so its lists are
                // read here, once per path, without changing the selection.
                if let Some(selection) = &current {
                    if self.typing.is_none() && self.read.as_deref() != Some(selection.config.as_str()) {
                        self.read = Some(selection.config.clone());
                        self.known = offered(&selection.config)
                            .ok()
                            .map(|displays| (selection.config.clone(), displays));
                    }
                }

                // What the config offers.
                let Some((loaded, displays)) = self.known.clone() else {
                    if current.is_some() && ui.button("Use the built-in transform").clicked() {
                        let _ = project.apply(Command::SetColorManagement { selection: None });
                    }
                    return;
                };
                let Some(selection) = current else {
                    return;
                };
                if selection.config != loaded {
                    return;
                }

                let mut chosen = selection.clone();
                crate::field::row(ui, "Display", |ui| {
                    crate::field::combo(ui, "ocio-display", chosen.display.clone(), |ui| {
                        for (display, _) in &displays {
                            ui.selectable_value(&mut chosen.display, display.clone(), display);
                        }
                    });
                });
                let views = displays
                    .iter()
                    .find(|(name, _)| *name == chosen.display)
                    .map(|(_, views)| views.clone())
                    .unwrap_or_default();
                crate::field::row(ui, "View", |ui| {
                    crate::field::combo(ui, "ocio-view", chosen.view.clone(), |ui| {
                        for view in &views {
                            ui.selectable_value(&mut chosen.view, view.clone(), view);
                        }
                    });
                });
                // Changing the display can leave a view that the new one does
                // not have, which would develop through nothing at all.
                if !views.is_empty() && !views.contains(&chosen.view) {
                    chosen.view = views[0].clone();
                }

                if chosen != selection {
                    let _ = project.apply(Command::SetColorManagement {
                        selection: Some(chosen),
                    });
                }
                if ui.button("Use the built-in transform").clicked() {
                    let _ = project.apply(Command::SetColorManagement { selection: None });
                }
            });
    }

    /// Read a config's displays and views, and select the first view it can
    /// develop through.
    fn load(&mut self, path: &str, project: &mut Project) {
        self.read = Some(path.to_string());
        if path.is_empty() {
            self.known = None;
            if project.document().ocio.is_some() {
                let _ = project.apply(Command::SetColorManagement { selection: None });
            }
            return;
        }
        match tequila_color::ocio::Config::load(path) {
            Ok(config) => {
                let displays = lists(&config);
                let first = config.first_buildable_view();
                self.known = Some((path.to_string(), displays));
                if let Some((display, view)) = first {
                    let _ = project.apply(Command::SetColorManagement {
                        selection: Some(tequila_scene::document::OcioSelection {
                            config: path.to_string(),
                            display,
                            view,
                        }),
                    });
                }
            }
            Err(error) => {
                self.known = None;
                project.status = format!("The colour config could not be read: {error}");
            }
        }
    }
}

/// A config's displays and their views, in the order the config offers them.
fn offered(path: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    tequila_color::ocio::Config::load(path).map(|config| lists(&config))
}

fn lists(config: &tequila_color::ocio::Config) -> Vec<(String, Vec<String>)> {
    config
        .displays
        .iter()
        .map(|(name, views)| (name.clone(), views.iter().map(|v| v.name.clone()).collect()))
        .collect()
}
