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

        ui.horizontal(|ui| {
            for mode in ToneMode::ALL {
                ui.selectable_value(&mut style.mode, mode, mode.label());
            }
        });
        ui.add_space(2.0);

        ui.add(crate::field::Slider::new(&mut style.exposure, -6.0..=6.0).text("Exposure"));
        ui.add(crate::field::Slider::new(&mut style.contrast, 0.4..=2.5).text("Contrast"));
        ui.add(crate::field::Slider::new(&mut style.saturation, 0.0..=2.0).text("Saturation"));

        if style.mode == ToneMode::Photographic {
            ui.add_space(4.0);
            ui.add(crate::field::Slider::new(&mut style.highlights, -1.0..=1.0).text("Highlights"));
            ui.add(crate::field::Slider::new(&mut style.shadows, -1.0..=1.0).text("Shadows"));
            ui.add(crate::field::Slider::new(&mut style.whites, -1.0..=1.0).text("Whites"));
            ui.add(crate::field::Slider::new(&mut style.blacks, -1.0..=1.0).text("Blacks"));

            ui.add_space(4.0);
            ui.add(
                crate::field::Slider::new(&mut style.temperature, 1800.0..=15000.0)
                    .text("Temperature")
                    .suffix(" K")
                    .logarithmic(true),
            );
            ui.add(crate::field::Slider::new(&mut style.tint, -1.0..=1.0).text("Tint"));
            if ui.small_button("Neutral").clicked() {
                style.temperature = NEUTRAL_KELVIN;
                style.tint = 0.0;
            }

            ui.add_space(4.0);
            ui.add(crate::field::Slider::new(&mut style.vignette, 0.0..=1.0).text("Vignette"));
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

        ui.add_space(4.0);
        egui::ComboBox::from_label("Tone map")
            .selected_text(style.display.label())
            .show_ui(ui, |ui| {
                for transform in DisplayTransform::ALL {
                    ui.selectable_value(&mut style.display, transform, transform.label());
                }
            });

        self.color_management(ui, project);

        if ui.small_button("Reset").clicked() {
            // The display transform survives a reset: it is a choice about the
            // *output device and pipeline*, not part of the look, and having it
            // snap back to AgX every time the grade is cleared would be an
            // unrelated change the user did not ask for.
            style = ImageStyle {
                display: style.display,
                ..ImageStyle::default()
            };
        }

        ui.label(
            egui::RichText::new("These re-grade the accumulated image; they never restart it.")
                .small()
                .weak(),
        );

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

                ui.label(
                    egui::RichText::new(
                        "Point at an OpenColorIO config to develop through the same \
                         transforms as the rest of your pipeline.",
                    )
                    .small()
                    .weak(),
                );
                let response = ui.add(
                    egui::TextEdit::singleline(&mut path)
                        .desired_width(f32::INFINITY)
                        .hint_text("path to config.ocio"),
                );
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
                egui::ComboBox::from_label("Display")
                    .selected_text(&chosen.display)
                    .show_ui(ui, |ui| {
                        for (display, _) in &displays {
                            ui.selectable_value(&mut chosen.display, display.clone(), display);
                        }
                    });
                let views = displays
                    .iter()
                    .find(|(name, _)| *name == chosen.display)
                    .map(|(_, views)| views.clone())
                    .unwrap_or_default();
                egui::ComboBox::from_label("View")
                    .selected_text(&chosen.view)
                    .show_ui(ui, |ui| {
                        for view in &views {
                            ui.selectable_value(&mut chosen.view, view.clone(), view);
                        }
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
