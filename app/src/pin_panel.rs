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

/// The map on its own, at the top of the Environment tab: the whole sky at a
/// glance, and its lights moved by dragging them across it.
pub fn map_section(ui: &mut egui::Ui, project: &mut Project) {
    let original = project.document().environment.pins.clone();
    let mut pins = original.clone();
    if let Some(added) = map(ui, project, &mut pins) {
        project.end_interaction();
        pins = added;
    }
    ui.label(
        egui::RichText::new("Drag a light to move it · scroll over it to size it · double-click the sky to add one")
            .small()
            .color(crate::theme::dim()),
    );
    if pins != original {
        if let Err(error) = project.apply(Command::SetEnvironmentPins { pins }) {
            project.status = error.to_string();
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        project.end_interaction();
    }
}

/// The environment flattened, its pins drawn where they sit: KeyShot's HDRI
/// editor's picture, where a light is moved by dragging it across the sky
/// rather than by typing a bearing and a height. Drag a pin to move it, scroll
/// over one to size it, double-click the sky to put a softbox there. `Some`
/// with every pin and a new one, for a double-click.
fn map(ui: &mut egui::Ui, project: &mut Project, pins: &mut [Pin]) -> Option<Vec<Pin>> {
    use tequila_core::sampling::{direction_to_equirect, equirect_to_direction};
    let width = ui.available_width().clamp(160.0, 560.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, width * 0.5), egui::Sense::click_and_drag());
    let selected_id = ui.id().with("pin-map-selected");
    let mut selected: Option<usize> = ui.data(|d| d.get_temp(selected_id)).filter(|&i: &usize| i < pins.len());

    // The picture, made again when the environment has changed and nothing
    // is being dragged: a few milliseconds, but not every frame of a drag.
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", project.document().environment).hash(&mut h);
        h.finish()
    };
    let picture_id = ui.id().with("pin-map-picture");
    let cached: Option<(u64, egui::TextureHandle)> = ui.data(|d| d.get_temp(picture_id));
    let dragging = ui.input(|i| i.pointer.any_down());
    let texture = match cached {
        Some((k, texture)) if k == key || dragging => texture,
        _ => {
            let image = project.environment_picture(MAP_WIDTH);
            let texture = ui.ctx().load_texture("pin-map", map_picture(&image), egui::TextureOptions::LINEAR);
            ui.data_mut(|d| d.insert_temp(picture_id, (key, texture.clone())));
            texture
        }
    };
    let painter = ui.painter_at(rect);
    painter.image(texture.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    // The horizon and the camera's bearing, to read the sky by.
    let dim = egui::Color32::from_white_alpha(40);
    painter.line_segment([egui::pos2(rect.left(), rect.center().y), egui::pos2(rect.right(), rect.center().y)], egui::Stroke::new(1.0_f32, dim));
    painter.line_segment([egui::pos2(rect.center().x, rect.top()), egui::pos2(rect.center().x, rect.bottom())], egui::Stroke::new(1.0_f32, dim));

    let to_screen = |uv: tequila_core::math::Vec2| egui::pos2(rect.left() + uv.x * rect.width(), rect.top() + uv.y * rect.height());
    let to_uv = |p: egui::Pos2| tequila_core::math::vec2(((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0), ((p.y - rect.top()) / rect.height()).clamp(0.0, 0.9999));
    for (index, pin) in pins.iter().enumerate() {
        if !pin.enabled || matches!(pin.kind, PinKind::Gradient { radial: true, .. }) {
            continue;
        }
        let on = selected == Some(index);
        let stroke = if on {
            egui::Stroke::new(2.0_f32, crate::theme::p().accent)
        } else {
            egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(170))
        };
        for run in outline(pin) {
            let points: Vec<egui::Pos2> = run.into_iter().map(to_screen).collect();
            painter.add(egui::Shape::line(points, stroke));
        }
        let centre = to_screen(direction_to_equirect(pin.direction));
        painter.circle_filled(centre, if on { 3.5 } else { 2.5 }, stroke.color);
        if on {
            painter.text(centre + egui::vec2(6.0, -6.0), egui::Align2::LEFT_BOTTOM, &pin.name, egui::FontId::proportional(11.0), egui::Color32::WHITE);
        }
    }

    // Pick the pin under the pointer: the one whose light covers it, or the
    // nearest centre close by.
    let picked = |p: egui::Pos2| -> Option<usize> {
        let d = equirect_to_direction(to_uv(p));
        // A round backdrop gradient covers the whole sky: never in the way.
        let backdrop = |pin: &Pin| matches!(pin.kind, PinKind::Gradient { radial: true, .. });
        let covering = pins.iter().enumerate().rev().find(|(_, pin)| pin.enabled && !backdrop(pin) && pin.coverage(d) > 0.0).map(|(i, _)| i);
        covering.or_else(|| {
            pins.iter()
                .enumerate()
                .filter(|(_, pin)| pin.enabled)
                .map(|(i, pin)| (i, to_screen(direction_to_equirect(pin.direction)).distance(p)))
                .filter(|(_, distance)| *distance < 12.0)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i)
        })
    };
    if response.drag_started() || response.clicked() {
        selected = response.interact_pointer_pos().and_then(picked);
    }
    if let (Some(index), Some(p)) = (selected, response.interact_pointer_pos()) {
        if response.dragged() {
            pins[index].direction = equirect_to_direction(to_uv(p));
        }
    }
    if let (Some(index), true) = (selected, response.hovered()) {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            let k = (1.0f32 + 0.1).powf(scroll.signum());
            let pin = &mut pins[index];
            pin.angular_size = tequila_core::math::vec2(
                (pin.angular_size.x * k).clamp(0.01, std::f32::consts::PI),
                (pin.angular_size.y * k).clamp(0.01, std::f32::consts::PI),
            );
        }
    }
    let mut added = None;
    if response.double_clicked() && selected.is_none() {
        if let Some(p) = response.interact_pointer_pos() {
            let (bearing, height) = angles_of(equirect_to_direction(to_uv(p)));
            let mut with = pins.to_vec();
            with.push(tequila_render_core::pins::softbox("Softbox", bearing.to_degrees(), height.to_degrees(), 0.8, 0.5, 12.0));
            selected = Some(with.len() - 1);
            added = Some(with);
        }
    }
    let response = response.on_hover_text(
        "The environment, flattened: the middle is straight ahead of the default camera, the line across it the horizon. \
         Drag a light to move it, scroll over it to size it, double-click the sky for a new softbox there.",
    );
    let _ = response;
    match selected {
        Some(index) => ui.data_mut(|d| d.insert_temp(selected_id, index)),
        None => ui.data_mut(|d| d.remove::<usize>(selected_id)),
    }
    added
}

/// How wide the map's picture is made, in texels.
const MAP_WIDTH: usize = 512;

/// The environment as the map shows it: exposed so its average sits mid-grey,
/// lights rolled off instead of clipped, in sRGB.
fn map_picture(image: &tequila_texture::Image) -> egui::ColorImage {
    let channels = image.channels.max(1);
    let luminance: Vec<f32> = image
        .data
        .chunks_exact(channels)
        .map(|p| if channels >= 3 { 0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2] } else { p[0] })
        .collect();
    let mean = luminance.iter().sum::<f32>() / luminance.len().max(1) as f32;
    let exposure = 0.3 / mean.max(1e-4);
    let encode = |v: f32| {
        let v = v * exposure;
        let v = v / (1.0 + v);
        let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (s.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    };
    let pixels = image
        .data
        .chunks_exact(channels)
        .map(|p| {
            let (r, g, b) = if channels >= 3 { (p[0], p[1], p[2]) } else { (p[0], p[0], p[0]) };
            egui::Color32::from_rgb(encode(r), encode(g), encode(b))
        })
        .collect();
    egui::ColorImage { size: [image.width, image.height], pixels }
}

/// The direction `a` radians along a pin's own right and `b` along its up,
/// its roll applied, measured as the renderer measures a pin's footprint:
/// each an angle against the pin's direction on its own.
fn off_centre(pin: &Pin, a: f32, b: f32) -> tequila_core::math::Vec3 {
    use tequila_core::math::Vec3;
    let axis = pin.direction.try_normalize().unwrap_or(Vec3::Y);
    let reference = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let right = reference.cross(axis).normalize();
    let up = axis.cross(right);
    let (sin, cos) = pin.roll.sin_cos();
    let (right, up) = (right * cos + up * sin, up * cos - right * sin);
    // Past a quarter turn the edge is behind the pin, where it covers nothing.
    let limit = std::f32::consts::FRAC_PI_2 - 0.01;
    (axis + right * a.clamp(-limit, limit).tan() + up * b.clamp(-limit, limit).tan()).normalize()
}

/// A pin's edge on the map, as runs of points in the map's coordinates,
/// broken where it crosses the map's left and right edges.
fn outline(pin: &Pin) -> Vec<Vec<tequila_core::math::Vec2>> {
    let (hu, hv) = (pin.angular_size.x * 0.5, pin.angular_size.y * 0.5);
    // Round the edge: an ellipse, or the rectangle's four sides.
    let edge: Vec<(f32, f32)> = match pin.shape {
        PinShape::Ellipse => (0..=64).map(|i| {
            let t = i as f32 / 64.0 * std::f32::consts::TAU;
            (hu * t.cos(), hv * t.sin())
        }).collect(),
        PinShape::Rectangle => {
            let corners = [(-hu, -hv), (hu, -hv), (hu, hv), (-hu, hv), (-hu, -hv)];
            corners.windows(2).flat_map(|w| (0..16).map(move |i| {
                let t = i as f32 / 16.0;
                (w[0].0 + (w[1].0 - w[0].0) * t, w[0].1 + (w[1].1 - w[0].1) * t)
            })).chain(std::iter::once(corners[0])).collect()
        }
    };
    let mut runs = vec![Vec::new()];
    let mut last: Option<tequila_core::math::Vec2> = None;
    for (a, b) in edge {
        let uv = tequila_core::sampling::direction_to_equirect(off_centre(pin, a, b));
        if last.is_some_and(|l| (l.x - uv.x).abs() > 0.5) {
            runs.push(Vec::new());
        }
        runs.last_mut().unwrap().push(uv);
        last = Some(uv);
    }
    runs.retain(|r| r.len() > 1);
    runs
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

    /// The map draws a light where the render puts it: just inside the
    /// outline it draws, the pin lights the sky; just outside, it does not.
    #[test]
    fn the_map_outlines_a_pin_where_it_lights() {
        for (bearing, height, roll, shape) in [
            (25.0f32, 35.0f32, 0.0f32, PinShape::Rectangle),
            (-140.0, 10.0, 0.6, PinShape::Rectangle),
            (90.0, 70.0, 0.0, PinShape::Ellipse),
            (0.0, -20.0, 1.2, PinShape::Ellipse),
        ] {
            let mut pin = tequila_render_core::pins::softbox("Key", bearing, height, 0.9, 0.0, 10.0);
            pin.roll = roll;
            pin.shape = shape;
            pin.angular_size = tequila_core::math::vec2(0.9, 0.5);
            let (hu, hv) = (0.45f32, 0.25f32);
            for i in 0..24 {
                let t = i as f32 / 24.0 * std::f32::consts::TAU;
                let (a, b) = match shape {
                    PinShape::Ellipse => (hu * t.cos(), hv * t.sin()),
                    // Round the rectangle's edge: the larger of the two at its limit.
                    PinShape::Rectangle => {
                        let (c, s) = (t.cos(), t.sin());
                        let k = 1.0 / (c.abs() / hu).max(s.abs() / hv);
                        (k * c, k * s)
                    }
                };
                let inside = off_centre(&pin, a * 0.9, b * 0.9);
                let outside = off_centre(&pin, a * 1.1, b * 1.1);
                assert!(pin.coverage(inside) > 0.0, "{bearing},{height},{roll},{shape:?}: dark just inside at {t}");
                assert_eq!(pin.coverage(outside), 0.0, "{bearing},{height},{roll},{shape:?}: lit just outside at {t}");
            }
            // And its centre is the map point the pin is dragged by.
            let centre = tequila_core::sampling::direction_to_equirect(pin.direction);
            let back = tequila_core::sampling::equirect_to_direction(centre);
            assert!((back - pin.direction.normalize()).length() < 1e-4);
        }
    }

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
