//! The Environment tab (`TEQUILA_SPEC.md` §1.5).
//!
//! Like the Material tab, every control here is generated rather than written:
//! the panel walks [`EnvParam::ALL`], asks each field what shape it is and what
//! bounds it wants, and builds the row. There is no `match` on a field name in
//! this file, which is what keeps the panel, the command bus, the journal and
//! the file format from drifting apart — they are all reading the same table.
//!
//! The one thing the panel does decide is *grouping*, because a flat list of
//! fourteen controls would bury the two that matter. §1.5's own order is used:
//! the image and its adjustments, then the dome's transform, then the
//! background, then the ground.

use eframe::egui;
use tequila_scene::command::{EnvParam, EnvValue, EnvironmentParameter};
use tequila_scene::document::BackgroundMode;
use tequila_scene::Command;

use crate::project::Project;

#[derive(Default)]
pub struct EnvironmentTab {
    /// A path being typed, held while the field has focus so each keystroke
    /// does not become an undo step and a half-typed path does not become a
    /// failed load.
    editing: Option<(EnvParam, String)>,
    /// Why the last environment file was refused, shown until another is
    /// chosen.
    problem: Option<String>,
    /// A pin whose viewport aiming the user just asked to start or stop.
    /// Drained by the app each frame and handed to the viewport, which is
    /// where the mode lives.
    pub aim_pin_request: Option<usize>,
    /// Which pin is being aimed right now, so its button shows as held down.
    /// Set by the app from the viewport's own state.
    pub aiming_pin: Option<usize>,
    /// Whether the map is kept in view, drawn by the app above the page,
    /// and so not here. Set by the app each frame.
    pub map_pin: crate::pin_panel::MapPin,
    /// The map's pin was clicked here; drained by the app.
    pub map_pin_clicked: bool,
}

/// The groups, in the order §1.5 presents them.
const GROUPS: [(&str, &[EnvParam]); 3] = [
    (
        "Turn and size",
        &[
            EnvParam::Rotation,
            EnvParam::Size,
            EnvParam::Height,
        ],
    ),
    (
        "Background",
        &[
            EnvParam::Background,
            EnvParam::BackgroundColor,
            EnvParam::Backplate,
        ],
    ),
    (
        "Ground",
        &[
            EnvParam::GroundEnabled,
            EnvParam::GroundShadows,
            EnvParam::GroundOcclusion,
            EnvParam::GroundShadowColor,
            EnvParam::GroundReflections,
            EnvParam::GroundReflectionBlur,
            EnvParam::GroundReflectionLength,
            EnvParam::GroundReflectionFadeDistance,
            EnvParam::GroundReflectionFresnel,
            EnvParam::GroundReflectionAlpha,
            EnvParam::GroundReflectionFadeBy,
            EnvParam::GroundBounce,
            EnvParam::GroundBounceStrength,
            EnvParam::GroundBounceColor,
            EnvParam::GroundBounceContactFalloff,
            EnvParam::GroundFlatten,
            EnvParam::GroundSize,
        ],
    ),
];

impl EnvironmentTab {
    pub fn show(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        let settings = project.document().environment.clone();
        let mut edits: Vec<EnvironmentParameter> = Vec::new();

        // The image itself, above the adjustments that act on it.
        //
        // This was a read-only label. §1.5 calls HDRI-first lighting "the
        // second feature that defines the product" and the environment map
        // loader has existed since M0 — with nothing anywhere in the interface
        // able to choose a file, so every scene was lit by the built-in studio
        // and nothing else.
        let mut load = false;
        let mut clear = false;
        // One row: what lights the scene, and the two things to do about
        // it. "HDRI" is the file's kind, said in the tooltip; the row is
        // named for what the image is for.
        crate::theme::section(ui, "Light from");
        crate::field::row(ui, "Image", |ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if settings.image.is_some() {
                    clear = crate::icons::button(ui, crate::icons::Icon::Close, false, 26.0, egui::Sense::click())
                        .on_hover_text("Go back to the built-in studio")
                        .clicked();
                }
                load = ui
                    .button("Choose…")
                    .on_hover_text("Light the scene with a 360° image of a place or a studio: an HDRI, as an .hdr or .exr file")
                    .clicked();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let (shown, whole) = match &settings.image {
                        Some(path) => (
                            std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.clone()),
                            path.clone(),
                        ),
                        None => ("Built-in studio".to_string(), "Tequila's own studio: soft light from above and both sides".to_string()),
                    };
                    ui.add(egui::Label::new(shown).truncate()).on_hover_text(whole);
                });
            });
        });
        if let Some(problem) = &self.problem {
            crate::theme::warning(ui, problem.as_str());
        }
        ui.add_space(2.0);
        // The whole environment flattened, its lights on it, as KeyShot's
        // HDRI editor shows it: where the light comes from, at a glance.
        // Kept in view, it is above the page, drawn by the app.
        if self.map_pin != crate::pin_panel::MapPin::On {
            self.map_pin_clicked |= crate::pin_panel::map_section(ui, project, self.map_pin);
            ui.add_space(2.0);
        }

        // The dialog blocks, so it runs after the panel is laid out rather
        // than in the middle of a layout pass.
        if load {
            self.problem = None;
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("Environment", &["hdr", "exr", "hdri", "pic", "rgbe"])
                .set_title("Lighting environment")
                .pick_file()
            {
                // Checked before it is committed. A file the loader cannot
                // read falls back to the built-in studio, and falling back
                // silently right after somebody chose a file is the worst
                // possible moment for it: the view does not change, and there
                // is nothing to say whether the file was wrong, the format
                // unsupported, or the control broken.
                match tequila_texture::Image::load(&path) {
                    Ok(_) => edits.push(EnvironmentParameter::new(
                        EnvParam::Image,
                        EnvValue::Path(Some(path.display().to_string())),
                    )),
                    Err(error) => {
                        self.problem = Some(format!(
                            "{}: {error}",
                            path.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        ))
                    }
                }
            }
        }
        if clear {
            edits.push(EnvironmentParameter::new(
                EnvParam::Image,
                EnvValue::Path(None),
            ));
        }

        for param in [EnvParam::Brightness, EnvParam::Contrast] {
            self.row(ui, param, &settings, &mut edits);
        }

        // The sections, in the person's own order: by default the order the
        // light is built in - the dome turned and sized, what is seen behind
        // the model, the floor, the sky that replaces the image, and the
        // pins painted on top of it all.
        let mut page = crate::sections::Page::new(ui, "environment");
        for key in page.order() {
            match key {
                "turn" | "background" | "ground" => {
                    let index = ["turn", "background", "ground"].iter().position(|k| *k == key).unwrap_or(0);
                    let (name, params) = GROUPS[index];
                    page.section(ui, key, name, |ui| {
                        crate::field::Fold::new(name).default_open(name == "Background").show(ui, |ui| {
                            for param in params {
                                self.row(ui, *param, &settings, &mut edits);
                            }
                            if name == "Turn and size" && settings.size <= 0.0 {
                                crate::theme::note(
                                    ui,
                                    "Size 0 puts the environment infinitely far away, so lighting \
                                     does not change as a part moves.",
                                );
                            }
                        });
                    });
                }
                "sky" => page.section(ui, key, "Sun & sky", |ui| crate::sky_panel::show(ui, project)),
                "pins" => {
                    let count = project.document().environment.pins.len();
                    let title = if count == 0 { "Lighting pins".to_string() } else { format!("Lighting pins · {count}") };
                    page.section(ui, key, &title, |ui| {
                        if let Some(index) = crate::pin_panel::show(ui, project, self.aiming_pin) {
                            self.aim_pin_request = Some(index);
                        }
                    });
                }
                _ => {}
            }
        }
        page.finish(ui);

        for edit in edits {
            let _ = project.apply(Command::SetEnvironmentParameter { parameter: edit });
        }

        // The whole environment written out, at the page's foot: it is of
        // every section above, not of one.
        crate::pin_panel::export_row(ui, project);
    }

    /// One control, shaped by what the field holds.
    fn row(
        &mut self,
        ui: &mut egui::Ui,
        param: EnvParam,
        settings: &tequila_scene::document::EnvironmentSettings,
        edits: &mut Vec<EnvironmentParameter>,
    ) {
        let ui_hints = param.ui();
        let current = param.read(settings);

        // Controls that would do nothing are disabled rather than hidden:
        // a control that vanishes leaves a user hunting for where it went,
        // while one that is greyed out says what it is waiting for.
        let enabled = match param {
            EnvParam::BackgroundColor => settings.background == BackgroundMode::Color,
            EnvParam::Backplate => settings.background == BackgroundMode::Backplate,
            // Everything below the master switch is greyed out when there is
            // no floor for it to describe.
            EnvParam::GroundShadows | EnvParam::GroundReflections | EnvParam::GroundFlatten | EnvParam::GroundBounce => {
                settings.ground.enabled
            }
            EnvParam::GroundBounceStrength | EnvParam::GroundBounceColor | EnvParam::GroundBounceContactFalloff => {
                settings.ground.enabled && settings.ground.bounce
            }
            // A length set in millimetres takes over from the share of the
            // product's height, so the share is greyed out while it is set.
            EnvParam::GroundReflectionLength => {
                settings.ground.enabled && settings.ground.reflections > 0.0 && settings.ground.reflection_fade_distance <= 0.0
            }
            EnvParam::GroundShadowColor | EnvParam::GroundOcclusion => {
                settings.ground.enabled && settings.ground.shadows
            }
            // The reflection's look, once there is a reflection to look at.
            EnvParam::GroundReflectionBlur
            | EnvParam::GroundReflectionFadeDistance
            | EnvParam::GroundReflectionFresnel
            | EnvParam::GroundReflectionAlpha
            | EnvParam::GroundReflectionFadeBy => settings.ground.enabled && settings.ground.reflections > 0.0,
            EnvParam::GroundSize => {
                settings.ground.enabled
                    && (settings.ground.shadows || settings.ground.reflections > 0.0)
            }
            _ => true,
        };

        match current {
            // Small lengths, stored in metres like every length and shown in
            // millimetres, the size they are set at: 7.5 mm, not 0.0075.
            EnvValue::Scalar(value)
                if matches!(param, EnvParam::GroundReflectionFadeDistance | EnvParam::GroundBounceContactFalloff) =>
            {
                let mut mm = value * 1000.0;
                let response = ui.add_enabled(
                    enabled,
                    crate::field::Slider::new(&mut mm, 0.0..=50.0)
                        .text(ui_hints.label)
                        .suffix(" mm")
                        .unit(crate::numeric::Unit::Millimetres)
                        .clamping(egui::SliderClamping::Never),
                );
                if response.changed() {
                    edits.push(EnvironmentParameter::scalar(param, (mm / 1000.0).max(0.0)));
                }
            }
            // A turn, stored in radians and shown in degrees, the unit a
            // person turns things by: it read 0.00 to 6.28.
            EnvValue::Scalar(value) if param == EnvParam::Rotation => {
                let mut degrees = value.to_degrees();
                let response = ui.add_enabled(
                    enabled,
                    crate::field::Slider::new(&mut degrees, 0.0..=360.0)
                        .text(ui_hints.label)
                        .suffix("°")
                        .fixed_decimals(1)
                        .clamping(egui::SliderClamping::Never),
                );
                if response.changed() {
                    edits.push(EnvironmentParameter::scalar(param, degrees.to_radians()));
                }
            }
            EnvValue::Scalar(value) => {
                let mut v = value;
                let (lo, hi) = ui_hints.range;
                let response = ui.add_enabled(
                    enabled,
                    crate::field::Slider::new(&mut v, lo..=hi)
                        .text(ui_hints.label)
                        .clamping(egui::SliderClamping::Never),
                );
                if response.changed() {
                    edits.push(EnvironmentParameter::scalar(param, v));
                }
                if response.drag_stopped() || response.lost_focus() {
                    // End of a drag: the next edit starts its own undo step,
                    // so two hundred frames of dragging stay one entry and two
                    // deliberate values stay two.
                    ui.ctx().request_repaint();
                }
            }
            EnvValue::Flag(value) => {
                let mut v = value;
                if ui
                    .add_enabled(enabled, crate::field::Checkbox::new(&mut v, ui_hints.label))
                    .changed()
                {
                    edits.push(EnvironmentParameter::flag(param, v));
                }
            }
            EnvValue::Color(value) => {
                let mut picked = value;
                if ui
                    .add_enabled_ui(enabled, |ui| crate::color_picker::field(ui, ui_hints.label, &mut picked))
                    .inner
                    .changed()
                {
                    edits.push(EnvironmentParameter::color(param, picked));
                }
            }
            EnvValue::Mode(mode) => {
                // No name of its own: it is the first thing under the
                // fold called Background, and said it a second time.
                let mut next = mode;
                let options: Vec<(BackgroundMode, &str)> = BackgroundMode::ALL.iter().map(|m| (*m, m.label())).collect();
                if crate::theme::segmented(ui, &mut next, &options) {
                    edits.push(EnvironmentParameter::new(param, EnvValue::Mode(next)));
                }
            }
            EnvValue::Choice(choice) => {
                let mut next = choice;
                let options: Vec<(&'static str, &str)> = param.choices().iter().map(|(key, label)| (*key, *label)).collect();
                let changed = ui.add_enabled_ui(enabled, |ui| crate::field::choice(ui, ui_hints.label, &mut next, &options)).inner;
                if changed {
                    edits.push(EnvironmentParameter::new(param, EnvValue::Choice(next)));
                }
            }
            EnvValue::Path(path) => {
                // A picture is chosen, not typed: the button first, with
                // the path beside it for whoever pastes one.
                let mut browse = false;
                let mut picked: Option<String> = None;
                ui.add_enabled_ui(enabled, |ui| crate::field::row(ui, ui_hints.label, |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    browse = ui.button("Choose…").on_hover_text("A photograph to stand behind the model").clicked();
                    let mut text = match &self.editing {
                        Some((editing, text)) if *editing == param => text.clone(),
                        _ => path.clone().unwrap_or_default(),
                    };
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .desired_width(f32::INFINITY)
                            .hint_text(egui::RichText::new("No picture").color(crate::theme::p().text_faint)),
                    );
                    if response.changed() {
                        self.editing = Some((param, text.clone()));
                    }
                    // Committed on Enter or on losing focus, never per
                    // keystroke: a path is only meaningful once it is whole,
                    // and a load attempted at every character would fail
                    // thirty times on the way to succeeding once.
                    if response.lost_focus() {
                        self.editing = None;
                        let value = (!text.trim().is_empty()).then(|| text.trim().to_string());
                        if value != path {
                            picked = Some(value.unwrap_or_default());
                        }
                    }
                    });
                }));
                if browse {
                    if let Some(file) = rfd::FileDialog::new()
                        .add_filter("Picture", &tequila_texture::image::Image::DECODABLE)
                        .set_title(ui_hints.label)
                        .pick_file()
                    {
                        picked = Some(file.display().to_string());
                    }
                }
                if let Some(text) = picked {
                    edits.push(EnvironmentParameter::new(param, EnvValue::Path((!text.is_empty()).then_some(text))));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel must offer every field the table knows about. A control that
    /// exists in the command bus and nowhere on screen is a feature the user
    /// cannot reach, and the grouping above is exactly the kind of hand-written
    /// list that goes stale.
    #[test]
    fn every_environment_field_appears_in_a_group() {
        // Brightness and Contrast sit above the groups, and the image has a
        // row of its own with a file picker rather than a generic control —
        // it is a path chosen from a dialog, not a number. Listed here so the
        // count below still catches a parameter that reaches no panel at all,
        // which is what this test is for.
        let grouped: Vec<EnvParam> = GROUPS
            .iter()
            .flat_map(|(_, params)| params.iter().copied())
            .chain([EnvParam::Brightness, EnvParam::Contrast, EnvParam::Image])
            .collect();
        for param in EnvParam::ALL {
            assert!(
                grouped.contains(&param),
                "`{}` is in the table but on no panel",
                param.key()
            );
        }
        assert_eq!(
            grouped.len(),
            EnvParam::ALL.len(),
            "a field is shown in more than one place"
        );
    }
}
