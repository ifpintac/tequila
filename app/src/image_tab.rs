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
    /// The page. Returns whether the render has to restart: Remove noise
    /// is the final render's own settings; everything else develops the
    /// picture already made.
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) -> bool {
        let before = project.image_style();
        let mut style = before;
        let mut restart = false;

        // The page's head: which set of adjustments, and Reset beside it,
        // found before the controls it clears.
        let modes: Vec<(ToneMode, &str)> = ToneMode::ALL.iter().map(|m| (*m, m.label())).collect();
        let mut reset = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                reset = ui
                    .button("Reset")
                    .on_hover_text("Put every adjustment on this page back, keeping the tone map")
                    .clicked();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    crate::theme::segmented(ui, &mut style.mode, &modes);
                });
            });
        });
        crate::theme::note(ui, "Changing these never restarts the render: they work on the picture it has made.");
        ui.add_space(4.0);

        // The sections, in the person's own order. Tones, White balance and
        // Lens effects are Photographic's alone.
        let photographic = style.mode == ToneMode::Photographic;
        let mut page = crate::sections::Page::new(ui, "image");
        for key in page.order() {
            match key {
                "adjust" => page.section(ui, key, "Adjust", |ui| {
                    crate::field::Fold::new("Adjust").id_salt("image-adjust").default_open(true).show(ui, |ui| {
                        ui.add(crate::field::Slider::new(&mut style.exposure, -6.0..=6.0).text("Exposure"))
                            .on_hover_text("Brighter or darker, in stops: +1 is twice the light");
                        ui.add(crate::field::Slider::new(&mut style.contrast, 0.4..=2.5).text("Contrast"));
                        ui.add(crate::field::Slider::new(&mut style.saturation, 0.0..=2.0).text("Saturation"));
                    });
                }),
                "tones" if photographic => page.section(ui, key, "Tones", |ui| {
                    crate::field::Fold::new("Tones").id_salt("image-tones").default_open(true).show(ui, |ui| {
                        ui.add(crate::field::Slider::new(&mut style.highlights, -1.0..=1.0).text("Highlights"));
                        ui.add(crate::field::Slider::new(&mut style.shadows, -1.0..=1.0).text("Shadows"));
                        ui.add(crate::field::Slider::new(&mut style.whites, -1.0..=1.0).text("Whites"));
                        ui.add(crate::field::Slider::new(&mut style.blacks, -1.0..=1.0).text("Blacks"));
                    });
                }),
                "white_balance" if photographic => page.section(ui, key, "White balance", |ui| {
                    crate::field::Fold::new("White balance").id_salt("image-white-balance").default_open(true).show(ui, |ui| {
                        ui.add(
                            crate::field::Slider::new(&mut style.temperature, 1800.0..=15000.0)
                                .text("Temperature")
                                .suffix(" K")
                                .fixed_decimals(0)
                                .logarithmic(true),
                        );
                        ui.add(crate::field::Slider::new(&mut style.tint, -1.0..=1.0).text("Tint"));
                        if ui.button("Neutral").on_hover_text("Daylight, with no tint").clicked() {
                            style.temperature = NEUTRAL_KELVIN;
                            style.tint = 0.0;
                        }
                    });
                }),
                "lens_effects" if photographic => page.section(ui, key, "Lens effects", |ui| {
                    crate::field::Fold::new("Lens effects").id_salt("image-lens-effects").default_open(true).show(ui, |ui| {
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
                    });
                }),
                "output" => page.section(ui, key, "Output", |ui| {
                    crate::field::Fold::new("Output").id_salt("image-output").default_open(true).show(ui, |ui| {
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
                    });
                }),
                "denoise" => page.section(ui, key, "Remove noise", |ui| {
                    crate::field::Fold::new("Remove noise").id_salt("image-denoise").default_open(true).show(ui, |ui| {
                        restart |= denoise(ui, project);
                    });
                }),
                _ => {}
            }
        }
        page.finish(ui);

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

        if style != before {
            let _ = project.apply(Command::SetImageStyle {
                style: Box::new(style),
            });
        }
        restart
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

/// Remove noise: the final render's denoising, edited here because this is
/// where a person notices noise. The Render window shows the same controls;
/// both write to the document. The view's own on and off is on its toolbar.
/// Returns whether anything changed, which restarts the render.
fn denoise(ui: &mut egui::Ui, project: &mut Project) -> bool {
    let mut changed = false;
    let before = project.render_settings().clone();
    let mut settings = before.clone();
    ui.add(crate::field::Checkbox::new(&mut settings.denoise, "Clean up the final render"))
        .on_hover_text("Denoising: smooths away the grain a render has before it has run long. This is the final render's switch; the view has its own, the wand on its toolbar. Strength and how often are for both.");
    // Cycles' denoiser has no strength: it cleans fully or not
    // at all.
    let cycles = settings.engine.is_cycles();
    // Strength and cadence are the viewport's as well as the
    // render's: there to set while either denoises.
    let either = settings.denoise || settings.preview_denoise;
    ui.add_enabled(
        either && !cycles,
        crate::field::Slider::new(&mut settings.denoise_strength, 0.0..=1.0)
            .text("Strength"),
    )
    .on_disabled_hover_text(if cycles {
        "Cycles denoises at full strength; Strength is for Tequila's renderer"
    } else {
        "Denoising is off"
    });
    ui.add_enabled(
        either,
        crate::field::Slider::new(&mut settings.denoise_cadence, 1..=32)
            .text("Every")
            .suffix(" passes"),
    )
    .on_hover_text(
        "1 leaves the cadence to the scheduler, which denoises densely at \
     first and thins out as the image settles. A larger number is a \
     fixed interval, for an image big enough that denoising costs more \
     than the samples it smooths.",
    );
    if settings != before {
        let _ = project.apply(Command::SetRenderSettings {
            settings: Box::new(settings),
        });
        changed = true;
    }
    changed
}
