//! The Lighting pins section of the Environment tab (`docs/01` §1.5).
//!
//! Pins are how a scene gets studio lighting at all in this build. No
//! environment maps ship — §0.2 forbids bundling KeyShot's and CC0
//! replacements are a download — so the alternatives are the built-in
//! procedural studio, a file the user found, or this.
//!
//! ## Softboxes, not lights
//!
//! Every control here is named for what a photographer moves. "Size" is how
//! big the softbox is as seen from the product, which is the thing that
//! decides how soft the shadow edge is. "Softness" is the diffuser, which
//! decides how the highlight falls off across a curved surface. The numbers
//! underneath are angles in radians and a smoothstep parameter, and nobody
//! needs to know that.
//!
//! Position is spherical — a compass bearing and a height — rather than a
//! direction vector. Three numbers that must stay normalized is the wrong
//! control for "move the light round to the left a bit".
//!
//! ## Pictures and gradients
//!
//! Beside softboxes, a pin can hold a picture (a window, a photographed
//! softbox) or a gradient (`TEQUILA_SPEC.md` §1.5). The backdrop gradient is
//! the one people reach for: it sets what is behind the product without
//! touching how the product is lit, which a flat background colour cannot do
//! without taking the reflections with it.

use crate::project::Project;
use eframe::egui;
use tequila_core::spectrum::Spectrum;
use tequila_render_core::pins::{GradientStop, Pin, PinBlend, PinKind, PinShape};
use tequila_scene::Command;

pub use tequila_render_core::pins::{angles_of, direction_from, studio_presets};

pub fn show(ui: &mut egui::Ui, project: &mut Project, aiming: Option<usize>) -> Option<usize> {
    let original = project.document().environment.pins.clone();
    let mut pins = original.clone();
    let mut add: Option<Vec<Pin>> = None;
    let mut aim_request = None;
    // File dialogs block, so they run after the layout pass: `Some(None)`
    // for a new image pin, `Some(Some(i))` to change pin `i`'s picture.
    let mut pick_picture: Option<Option<usize>> = None;
    let mut export = false;
    let missing = project.missing_pin_images();

    crate::field::Fold::new(format!("Lighting pins · {}", pins.len()))
        .default_open(!pins.is_empty())
        .show(ui, |ui| {
            ui.small(
                "Softboxes painted onto the environment. They light the scene, \
                 cast soft shadows and show up in reflections.",
            );

            ui.horizontal_wrapped(|ui| {
                if ui
                    .button("+ Softbox")
                    .on_hover_text("One light, in front and above.")
                    .clicked()
                {
                    add = Some(vec![tequila_render_core::pins::softbox(
                        "Softbox", 25.0, 35.0, 0.8, 0.5, 12.0,
                    )]);
                }
                if ui
                    .button("+ Image…")
                    .on_hover_text("A picture on the environment: a window, a photographed softbox.")
                    .clicked()
                {
                    pick_picture = Some(None);
                }
                if ui
                    .button("+ Backdrop")
                    .on_hover_text(
                        "A gradient behind the product, floor to ceiling. Seen by the \
                         camera; does not change the lighting or the reflections.",
                    )
                    .clicked()
                {
                    let mut with = pins.clone();
                    with.push(Pin::backdrop_gradient(Spectrum::splat(0.02), Spectrum::splat(0.35)));
                    add = Some(with);
                }
                for (name, hint, preset) in studio_presets() {
                    if ui
                        .button(name)
                        .on_hover_text(format!("{hint}\n\nReplaces the pins you have."))
                        .clicked()
                    {
                        add = Some(preset);
                    }
                }
            });

            if pins.is_empty() {
                ui.weak(
                    "No pins. The scene is lit by the environment alone — add a \
                     softbox, or start from one of the setups above.",
                );
                return;
            }

            let mut remove = None;
            let count = pins.len();
            for index in 0..count {
                ui.push_id(index, |ui| {
                    ui.separator();
                    let pin = &mut pins[index];
                    ui.horizontal(|ui| {
                        ui.add(crate::field::Checkbox::new(&mut pin.enabled, "")).on_hover_text("Light on");
                        ui.add(egui::TextEdit::singleline(&mut pin.name).desired_width(110.0));
                        crate::color_picker::edit(ui, &mut pin.color);
                        // An eye, because "show in background" is a sentence
                        // and this row is already full.
                        let seen = pin.show_in_background;
                        if ui
                            .add(egui::SelectableLabel::new(seen, if seen { "👁" } else { "🚫" }))
                            .on_hover_text(if seen {
                                "Seen behind the product. Click to hide it — it \
                                 goes on lighting the scene and showing in \
                                 reflections."
                            } else {
                                "Hidden behind the product, still lighting it \
                                 and still in its reflections. Click to show."
                            })
                            .clicked()
                        {
                            pin.show_in_background = !seen;
                        }
                        let lights = pin.in_lighting;
                        if ui
                            .add(egui::SelectableLabel::new(lights, "💡"))
                            .on_hover_text(if lights {
                                "Lights the scene and shows in reflections. Click \
                                 to make it background only."
                            } else {
                                "Background only: seen behind the product, lights \
                                 nothing. Click to let it light the scene."
                            })
                            .clicked()
                        {
                            pin.in_lighting = !lights;
                        }
                        if ui.small_button("×").on_hover_text("Remove").clicked() {
                            remove = Some(index);
                        }
                        if ui
                            .add(egui::SelectableLabel::new(aiming == Some(index), "◎ Aim"))
                            .on_hover_text(AIM_HINT)
                            .clicked()
                        {
                            aim_request = Some(index);
                        }
                    });
                    if let PinKind::Image { path } = &pin.kind {
                        ui.horizontal(|ui| {
                            let name = std::path::Path::new(path)
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "no picture".into());
                            ui.label(format!("🖼 {name}")).on_hover_text(path.as_str());
                            if ui.small_button("Change…").clicked() {
                                pick_picture = Some(Some(index));
                            }
                        });
                        if missing.contains(path) {
                            ui.label(
                                egui::RichText::new("This picture will not load, so the pin shows nothing.")
                                    .small()
                                    .color(crate::theme::p().busy),
                            );
                        }
                    }
                    controls(ui, pin);
                });
            }
            if let Some(index) = remove {
                project.end_interaction();
                pins.remove(index);
            }
        });

    ui.horizontal(|ui| {
        let id = ui.id().with("export width");
        let mut width: usize = ui.data_mut(|d| *d.get_temp_mut_or(id, 4096usize));
        export = ui
            .button("Export HDRI…")
            .on_hover_text(
                "Write the environment with its pins painted in as an .exr or .hdr \
                 file, for another application or to keep.",
            )
            .clicked();
        egui::ComboBox::from_id_salt(id)
            .selected_text(format!("{}K", width / 1024))
            .width(52.0)
            .show_ui(ui, |ui| {
                for option in [1024usize, 2048, 4096, 8192] {
                    ui.selectable_value(&mut width, option, format!("{}K", option / 1024));
                }
            });
        ui.data_mut(|d| d.insert_temp(id, width));
        if export {
            export_hdri(project, width);
        }
    });

    if let Some(target) = pick_picture {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Picture", &["png", "jpg", "jpeg", "psd", "hdr", "exr"])
            .set_title("Picture for the pin")
            .pick_file()
        {
            match tequila_texture::Image::load(&path) {
                Ok(image) => {
                    let text = path.display().to_string();
                    // The pin takes the picture's own proportions.
                    let aspect = image.height as f32 / image.width.max(1) as f32;
                    project.end_interaction();
                    match target {
                        Some(index) if index < pins.len() => {
                            pins[index].kind = PinKind::Image { path: text };
                            pins[index].angular_size.y = pins[index].angular_size.x * aspect;
                        }
                        _ => {
                            let name = path
                                .file_stem()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| "Image".into());
                            let mut pin = Pin::image(&name, &text, direction_from(0.0, 0.35), 0.8);
                            pin.angular_size.y = pin.angular_size.x * aspect;
                            pins.push(pin);
                        }
                    }
                }
                Err(error) => project.status = format!("{}: {error}", path.display()),
            }
        }
    }
    if let Some(preset) = add {
        project.end_interaction();
        pins = preset;
    }
    if pins != original {
        if let Err(error) = project.apply(Command::SetEnvironmentPins { pins }) {
            project.status = error.to_string();
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        project.end_interaction();
    }
    aim_request
}

/// Said in the panel, because the gesture is not one anybody guesses.
const AIM_HINT: &str = "Click the model where you want this light's highlight and it moves \
                        there. Escape when done.";

/// Ask where to write the environment, and write it.
fn export_hdri(project: &mut Project, width: usize) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("OpenEXR", &["exr"])
        .add_filter("Radiance HDR", &["hdr"])
        .set_title("Export environment")
        .set_file_name("environment.exr")
        .save_file()
    else {
        return;
    };
    project.status = match project.export_environment(&path, width, false) {
        Ok(()) => format!("Environment written to {}", path.display()),
        Err(error) => format!("Could not write the environment: {error}"),
    };
}

/// A gradient's stops, and its shape.
fn gradient_controls(ui: &mut egui::Ui, stops: &mut Vec<GradientStop>, radial: &mut bool) {
    ui.horizontal(|ui| {
        ui.label("Gradient");
        ui.selectable_value(radial, true, "Round")
            .on_hover_text("Out from the pin's centre in every direction. Centred straight down, floor to ceiling.");
        ui.selectable_value(radial, false, "Across")
            .on_hover_text("Bottom to top across the pin.");
    });
    let mut remove = None;
    let count = stops.len();
    for (index, stop) in stops.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            crate::color_picker::edit(ui, &mut stop.color);
            ui.add(crate::field::Slider::new(&mut stop.t, 0.0..=1.0).text("at"));
            if count > 2 && ui.small_button("×").on_hover_text("Remove this colour").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        stops.remove(index);
    }
    if ui.small_button("+ Colour").clicked() {
        let t = 0.5;
        let color = tequila_render_core::pins::gradient_at(stops, t);
        stops.push(GradientStop { t, color });
    }
}

fn controls(ui: &mut egui::Ui, pin: &mut Pin) {
    let radial = matches!(pin.kind, PinKind::Gradient { radial: true, .. });
    if let PinKind::Gradient { stops, radial } = &mut pin.kind {
        gradient_controls(ui, stops, radial);
    }
    let (bearing, height) = angles_of(pin.direction);
    let (mut bearing_deg, mut height_deg) = (bearing.to_degrees(), height.to_degrees());

    let mut moved = false;
    moved |= ui
        .add(
            crate::field::Slider::new(&mut bearing_deg, -180.0..=180.0)
                .text("Around °")
                .suffix("°"),
        )
        .on_hover_text("Swing the light around the product. 0 is behind the camera.")
        .changed();
    moved |= ui
        .add(
            crate::field::Slider::new(&mut height_deg, -89.0..=89.0)
                .text("Height °")
                .suffix("°"),
        )
        .on_hover_text("Raise or lower it. 90 is straight overhead.")
        .changed();
    if moved {
        pin.direction = direction_from(bearing_deg.to_radians(), height_deg.to_radians());
    }

    ui.add(
        crate::field::Slider::new(&mut pin.intensity, 0.0..=80.0)
            .text("Brightness")
            .logarithmic(true),
    );

    // Size in degrees, because that is what a photographer judges — how big
    // the box looks from where the product stands.
    let mut width = pin.angular_size.x.to_degrees();
    let mut height_size = pin.angular_size.y.to_degrees();
    // A round gradient's size is the width of its cap, which can be the whole
    // sphere; everything else stops short of a hemisphere, where a flat
    // footprint stops meaning anything.
    let widest = if radial { 360.0 } else { 170.0 };
    ui.horizontal(|ui| {
        ui.label("Size °");
        if ui
            .add(egui::DragValue::new(&mut width).speed(0.5).range(1.0..=widest))
            .changed()
        {
            pin.angular_size.x = width.to_radians();
        }
        if !radial && ui
            .add(
                egui::DragValue::new(&mut height_size)
                    .speed(0.5)
                    .range(1.0..=170.0),
            )
            .changed()
        {
            pin.angular_size.y = height_size.to_radians();
        }
    })
    .response
    .on_hover_text(
        "How big the light looks from the product. A bigger source gives a \
         softer shadow edge — the same reason a photographer moves a softbox \
         closer.",
    );

    ui.add(crate::field::Slider::new(&mut pin.softness, 0.0..=1.0).text("Softness"))
        .on_hover_text(
            "The diffuser. At 0 the light is flat to its edge, which gives a \
             crisp specular border; higher fades it out from the middle.",
        );

    let mut roll_deg = pin.roll.to_degrees();
    if ui
        .add(crate::field::Slider::new(&mut roll_deg, -90.0..=90.0).text("Turn °"))
        .on_hover_text("Turns the box on its own axis. Only visible when it is not square.")
        .changed()
    {
        pin.roll = roll_deg.to_radians();
    }

    if !radial {
        ui.horizontal(|ui| {
            ui.label("Shape");
            ui.selectable_value(&mut pin.shape, PinShape::Rectangle, "Box");
            ui.selectable_value(&mut pin.shape, PinShape::Ellipse, "Round");
        });
    }
    ui.horizontal(|ui| {
        ui.label("Blend");
        ui.selectable_value(&mut pin.blend, PinBlend::Add, "Add")
            .on_hover_text("Adds light on top of what is there, like a real light.");
        ui.selectable_value(&mut pin.blend, PinBlend::Over, "Cover")
            .on_hover_text("Covers what is under it: the pins above it in the list and the environment.");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The angles and the direction have to be inverses, or dragging a slider
    /// reads back a different number than it wrote and the control jitters as
    /// it is used.
    #[test]
    fn bearing_and_height_round_trip() {
        for bearing in [-179.0f32, -90.0, -1.0, 0.0, 37.0, 90.0, 179.0] {
            for height in [-88.0f32, -45.0, 0.0, 12.0, 88.0] {
                let direction = direction_from(bearing.to_radians(), height.to_radians());
                let (b, h) = angles_of(direction);
                assert!(
                    (b.to_degrees() - bearing).abs() < 1e-2
                        && (h.to_degrees() - height).abs() < 1e-2,
                    "({bearing}, {height}) came back as ({}, {})",
                    b.to_degrees(),
                    h.to_degrees()
                );
            }
        }
    }

    /// Height 90 is straight up, and bearing 0 is down -Z. Stated because
    /// every slider label in the panel promises it.
    #[test]
    fn the_angles_mean_what_the_labels_say() {
        let overhead = direction_from(0.0, std::f32::consts::FRAC_PI_2);
        assert!((overhead.y - 1.0).abs() < 1e-5, "90° was not overhead");

        let ahead = direction_from(0.0, 0.0);
        assert!(
            (ahead.z + 1.0).abs() < 1e-5,
            "bearing 0 was not down -Z: {ahead:?}"
        );

        // Positive bearing goes to +X, so "around" increases to the right.
        let right = direction_from(std::f32::consts::FRAC_PI_2, 0.0);
        assert!((right.x - 1.0).abs() < 1e-5, "positive bearing went the wrong way");
    }

    /// Every shipped preset has to be a set of pins the command bus accepts —
    /// a preset button that produces a refused edit is a button that does
    /// nothing.
    #[test]
    fn every_preset_is_accepted_by_the_command_bus() {
        for (name, _, pins) in studio_presets() {
            let mut document = tequila_scene::document::SceneDocument::new();
            let mut bus = tequila_scene::CommandBus::new();
            bus.apply(&mut document, Command::SetEnvironmentPins { pins: pins.clone() })
                .unwrap_or_else(|error| panic!("preset {name} was refused: {error}"));
            assert_eq!(document.environment.pins.len(), pins.len());
        }
    }

    /// The panel's own additions are accepted too: a backdrop's whole-sphere
    /// size and an image pin both pass the command's checks.
    #[test]
    fn the_panels_new_pins_are_accepted_by_the_command_bus() {
        let pins = vec![
            Pin::backdrop_gradient(Spectrum::splat(0.02), Spectrum::splat(0.35)),
            Pin::image("Window", "window.png", direction_from(0.0, 0.35), 0.8),
        ];
        let mut document = tequila_scene::document::SceneDocument::new();
        let mut bus = tequila_scene::CommandBus::new();
        bus.apply(&mut document, Command::SetEnvironmentPins { pins: pins.clone() })
            .expect("accepted");
        assert_eq!(document.environment.pins, pins);
    }

    /// And every preset actually lights something. A studio setup whose pins
    /// are all pointing below the floor, or are all dark, would pass every
    /// test above and light nothing.
    #[test]
    fn every_preset_puts_light_above_the_horizon() {
        for (name, _, pins) in studio_presets() {
            let lit = pins.iter().any(|pin| {
                pin.enabled && pin.intensity > 0.0 && pin.direction.normalize().y > 0.0
            });
            assert!(lit, "preset {name} has no light above the horizon");
        }
    }
}
