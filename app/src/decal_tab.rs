//! Printed artwork over a material, ordered from bottom to top.
//!
//! ## Placement
//!
//! A label has to be told where on the part it sits, and there are two honest
//! answers. If the mesh was authored with UV coordinates, the artist already
//! decided, and the label follows that layout. If it was not — and an STL never
//! is, nor most exported OBJ meshes — there is nothing to follow and the label
//! is *projected* onto the part instead, like a slide onto a wall.
//!
//! The section used to offer only the first, on every part, with the hint "select
//! a part with UV coordinates". On a part without them the label was added, the
//! list showed it, and the render did not change. The controls below name the
//! choice and default to the one that works for the part in hand.

use crate::project::Project;
use eframe::egui;
use tequila_scene::{decal_fit, textures::TextureSource, tree::MaterialKey, Command};
use tequila_texture::projection::{Axis, Mapping, Projection};
use tequila_texture::{texture::MapRole, Image};

/// Draw the Labels section.
///
/// Returns the index of a label whose viewport placement the user asked to
/// start or stop. Reported upwards rather than acted on here, because the mode
/// lives in the viewport and this panel has no reach into it.
pub fn show(
    ui: &mut egui::Ui,
    project: &mut Project,
    key: MaterialKey,
    placing: Option<(MaterialKey, usize)>,
) -> Option<usize> {
    let mut place_request = None;
    // What the added artwork says it prints at, in metres.
    let mut printed: Option<(f32, f32)> = None;
    let definition = project.document().materials.get(key)?;
    let original = definition.graph.decals.clone();
    let mut decals = original.clone();
    let mut action = None;

    // Read once, before anything borrows the document mutably.
    let part_has_uvs = decal_fit::has_uvs(project.document(), key);
    let fitted = decal_fit::fit_projected(project.document(), key);
    // A video chosen for a label, while its frame is picked.
    let pick_id = egui::Id::new(("video-label", key.0));
    let mut pick: Option<VideoPick> = ui.ctx().data(|d| d.get_temp(pick_id));
    let mut picked = false;

    crate::field::Fold::new(format!("Labels · {}", decals.len()))
        .id_salt(("labels", key.0))
        .default_open(true)
        .show(ui, |ui| {
            ui.small(
                "Printed artwork. Transparency reveals the material below; a file \
                 that says its print resolution arrives at its printed size.",
            );
            if ui.button("+ Add label…").clicked() {
                // Every picture the loader reads but the two HDR formats,
                // which hold light rather than artwork.
                let artwork: Vec<&str> = Image::DECODABLE.into_iter().filter(|e| !matches!(*e, "hdr" | "exr")).collect();
                let both: Vec<&str> = artwork.iter().chain(tequila_texture::video::EXTENSIONS).copied().collect();
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Artwork or video", &both)
                    .add_filter("Artwork", &artwork)
                    .add_filter("Video", tequila_texture::video::EXTENSIONS)
                    .pick_file()
                {
                    if tequila_texture::video::is_video(&path) {
                        // A frame of it, picked below.
                        match tequila_texture::video::duration(&path) {
                            Ok(duration) => {
                                pick = Some(VideoPick { path, duration, seconds: 0.0, preview: None, decoding: None, error: None })
                            }
                            Err(error) => project.status = format!("Cannot use this video: {error}"),
                        }
                    } else {
                        printed = tequila_texture::print_size::print_size(&path);
                        match Image::load(&path) {
                            Ok(_) => {
                                let name = path
                                    .file_stem()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                action = Some(Command::AddDecal {
                                    material: key,
                                    source: TextureSource::new(path, MapRole::Color),
                                    name,
                                });
                            }
                            Err(error) => project.status = format!("Cannot load label: {error}"),
                        }
                    }
                }
            }
            if let Some(chosen) = &mut pick {
                match video_picker(ui, chosen) {
                    Some(true) => {
                        picked = true;
                        match tequila_texture::video::still(&chosen.path, chosen.seconds, video_folder(project)) {
                            Ok(still) => {
                                let stem = chosen.path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                                action = Some(Command::AddDecal {
                                    material: key,
                                    source: TextureSource::new(still, MapRole::Color),
                                    name: format!("{stem} at {:.1} s", chosen.seconds),
                                });
                            }
                            Err(error) => project.status = format!("Cannot take the video's frame: {error}"),
                        }
                    }
                    Some(false) => picked = true,
                    None => {}
                }
            }
            if decals.is_empty() {
                ui.weak(if part_has_uvs {
                    "Add your logo or packaging artwork. It will follow the \
                     part's own UV layout."
                } else {
                    "Add your logo or packaging artwork. This part has no UV \
                     layout, so the label will be projected onto its widest \
                     face — you can change the face below."
                });
            }

            let mut reorder = None;
            let mut remove = None;
            let count = decals.len();
            for index in (0..count).rev() {
                ui.push_id(index, |ui| {
                    let label = &mut decals[index];
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add(crate::field::Checkbox::new(&mut label.enabled, ""))
                            .on_hover_text("Show label");
                        ui.add(egui::TextEdit::singleline(&mut label.name).desired_width(120.0));
                        if ui
                            .add_enabled(index + 1 < count, egui::Button::new("Up"))
                            .on_hover_text("Bring forward")
                            .clicked()
                        {
                            reorder = Some((index, index + 1));
                        }
                        if ui
                            .add_enabled(index > 0, egui::Button::new("Down"))
                            .on_hover_text("Send backward")
                            .clicked()
                        {
                            reorder = Some((index, index - 1));
                        }
                        if ui
                            .small_button("×")
                            .on_hover_text("Remove label")
                            .clicked()
                        {
                            remove = Some(index);
                        }
                        // Not offered for UV placement: there is no frame to
                        // move, so a click on the model could not do anything
                        // and a button that cannot work should not be there.
                        let projected = label.mapping.projection != Projection::Uv;
                        let active = placing == Some((key, index));
                        if ui
                            .add_enabled(
                                projected,
                                egui::SelectableLabel::new(active, "+ Place"),
                            )
                            .on_hover_text(
                                "Click the part in the viewport to put the \
                                 artwork there. Drag to slide it around. Esc \
                                 when done.",
                            )
                            .on_disabled_hover_text(
                                "UV placement follows the mesh's own \
                                 coordinates, so there is nothing to point at. \
                                 Switch to Project flat or Wrap around.",
                            )
                            .clicked()
                        {
                            place_request = Some(index);
                        }
                    });
                    crate::field::Fold::new("Placement & finish")
                        .id_salt("placement")
                        .show(ui, |ui| {
                            placement(ui, &mut label.mapping, part_has_uvs, fitted);
                            ui.separator();
                            ui.add(crate::field::Slider::new(&mut label.opacity, 0.0..=1.0).text("Opacity"));
                            ui.add(
                                crate::field::Slider::new(&mut label.roughness, 0.0..=1.0)
                                    .text("Roughness"),
                            );
                            crate::color_picker::field(ui, "Ink", &mut label.ink)
                                .on_hover_text("The artwork's colour is multiplied by this: white artwork prints in the ink's colour. White leaves it as it is.");
                            ui.add(crate::field::Checkbox::new(&mut label.metallic, "Foil"))
                                .on_hover_text("Printed in metal foil: the artwork reflects like metal, as a hot-stamped logo does.");
                            if label.mapping.projection == tequila_texture::projection::Projection::Planar {
                                use tequila_materials::decal::DecalSide;
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new("Prints on").color(crate::theme::dim()))
                                        .on_hover_text("A flat projection goes straight through the part: by itself, a label on the front also prints, mirrored, on the back. One side prints on faces turned within 60° of the label.");
                                    crate::theme::segmented(
                                        ui,
                                        &mut label.side,
                                        &[(DecalSide::Both, "Both sides"), (DecalSide::Positive, "+ side"), (DecalSide::Negative, "− side")],
                                    );
                                });
                            }
                        });
                });
            }
            if let Some((a, b)) = reorder {
                project.end_interaction();
                decals.swap(a, b);
            }
            if let Some(index) = remove {
                project.end_interaction();
                decals.remove(index);
            }
        });

    ui.ctx().data_mut(|d| match pick.filter(|_| !picked) {
        Some(pick) => d.insert_temp(pick_id, pick),
        None => d.remove::<VideoPick>(pick_id),
    });
    if action.is_none() && decals != original {
        action = Some(Command::SetDecals {
            material: key,
            decals,
        });
    }
    if let Some(command) = action {
        let adding = matches!(command, Command::AddDecal { .. });
        project.begin_group();
        if let Err(error) = project.apply(command) {
            project.status = error.to_string();
        } else if let (true, Some(size)) = (adding, printed) {
            print_at_size(project, key, size);
        }
        project.end_group();
    }
    if ui.input(|i| i.pointer.any_released()) {
        project.end_interaction();
    }
    place_request
}

/// A video chosen as a label's artwork, while the frame to print is picked.
#[derive(Clone)]
struct VideoPick {
    path: std::path::PathBuf,
    duration: f64,
    seconds: f64,
    /// The preview shown, and the time it is of.
    preview: Option<(f64, egui::TextureHandle)>,
    /// A frame being decoded for the preview on another thread, and its time.
    decoding: Option<(f64, std::sync::Arc<std::sync::Mutex<Option<Result<tequila_texture::video::Frame, String>>>>)>,
    error: Option<String>,
}

/// The frame picker: the video at the chosen time, a slider through it, and
/// whether to use that frame (`Some(true)`) or not (`Some(false)`).
fn video_picker(ui: &mut egui::Ui, pick: &mut VideoPick) -> Option<bool> {
    let mut answer = None;
    // A preview decoded since the last frame.
    if let Some((time, slot)) = &pick.decoding {
        let finished = slot.lock().ok().and_then(|mut s| s.take());
        if let Some(result) = finished {
            match result {
                Ok(frame) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied([frame.width, frame.height], &frame.rgba);
                    let texture = ui.ctx().load_texture("video-label-preview", image, egui::TextureOptions::LINEAR);
                    pick.preview = Some((*time, texture));
                    pick.error = None;
                }
                Err(error) => pick.error = Some(error),
            }
            pick.decoding = None;
        }
    }
    // One decode at a time, of the newest time asked for.
    let shown = pick.preview.as_ref().map(|(t, _)| *t);
    if pick.decoding.is_none() && shown != Some(pick.seconds) && pick.error.is_none() {
        let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let (path, seconds, into, ctx) = (pick.path.clone(), pick.seconds, slot.clone(), ui.ctx().clone());
        let started = std::thread::Builder::new().name("tequila-video-frame".into()).spawn(move || {
            let frame = tequila_texture::video::frame(&path, seconds);
            if let Ok(mut into) = into.lock() {
                *into = Some(frame);
            }
            ctx.request_repaint();
        });
        if started.is_ok() {
            pick.decoding = Some((pick.seconds, slot));
        }
    }
    egui::Frame::group(ui.style()).show(ui, |ui| {
        let name = pick.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        ui.label(format!("Which frame of {name}?"));
        match (&pick.preview, &pick.error) {
            (_, Some(error)) => {
                ui.colored_label(crate::theme::p().danger, error.as_str());
            }
            (Some((_, texture)), None) => {
                let size = texture.size_vec2();
                let width = ui.available_width().min(260.0);
                ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(width, width * size.y / size.x.max(1.0))));
            }
            (None, None) => {
                ui.spinner();
            }
        }
        let changed = ui
            .add(egui::Slider::new(&mut pick.seconds, 0.0..=pick.duration.max(0.0)).suffix(" s").max_decimals(2))
            .on_hover_text("Where in the video the label's picture is taken from.")
            .changed();
        if changed {
            pick.error = None;
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(pick.error.is_none(), egui::Button::new("Use this frame")).clicked() {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
        ui.small("The frame is kept as a picture beside the project, and printed like any label.");
    });
    answer
}

/// Where a video label's frame is kept: beside the project, in a folder
/// named after it, so the two travel together; among Tequila's settings for
/// a project not saved yet.
fn video_folder(project: &Project) -> std::path::PathBuf {
    match project.path.as_deref() {
        Some(path) => {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            path.with_file_name(format!("{stem} video frames"))
        }
        None => tequila_format::library::settings_directory().join("Video frames"),
    }
}

/// Size the label just added to what its artwork says it prints at, when it
/// is projected flat — the one placement where a size on the part is a size
/// in the world. Part of the same undo step as the adding.
fn print_at_size(project: &mut Project, key: MaterialKey, (width, height): (f32, f32)) {
    let Some(definition) = project.document().materials.get(key) else { return };
    let mut decals = definition.graph.decals.clone();
    let Some(label) = decals.last_mut() else { return };
    if label.mapping.projection != Projection::Planar {
        return;
    }
    // In metres, as every length in the document is; on a side face the
    // width runs along the projection's second coordinate.
    label.mapping.real_world_units = true;
    let (x, y) = if decal_fit::sideways(&label.mapping) { (height, width) } else { (width, height) };
    label.mapping.scale.x = x;
    label.mapping.scale.y = y;
    let text = format!("{:.1} × {:.1} mm, from the file's print resolution", width * 1000.0, height * 1000.0);
    if project.apply(Command::SetDecals { material: key, decals }).is_ok() {
        project.status = format!("Label sized to {text}");
    }
}

/// How the label finds its way onto the part.
fn placement(ui: &mut egui::Ui, mapping: &mut Mapping, part_has_uvs: bool, fitted: Option<Mapping>) {
    ui.horizontal(|ui| {
        ui.label("Placement");
        // Three named for what they do to artwork, not for the projection
        // they happen to use: "Cylindrical" is the name of the maths, "Wrap
        // around" is the name of the thing a person wants done to a bottle.
        for (projection, name, hint) in [
            (
                Projection::Uv,
                "On UV map",
                "Follow the part's own UV layout, as the artist authored it.",
            ),
            (
                Projection::Planar,
                "Project flat",
                "Shine the artwork onto the part along one axis, like a slide \
                 onto a wall. The right choice for a flat panel or a box face.",
            ),
            (
                Projection::Cylindrical,
                "Wrap around",
                "Wrap the artwork around an axis — a bottle, a can, a tube.",
            ),
        ] {
            let enabled = projection != Projection::Uv || part_has_uvs;
            let chosen = mapping.projection == projection;
            let response = ui.add_enabled(enabled, egui::SelectableLabel::new(chosen, name));
            if response.clicked() && !chosen {
                *mapping = switch_to(mapping, projection, fitted);
            }
            response.on_disabled_hover_text(
                "This part has no UV layout, so there is nothing for UV \
                 placement to follow. Project the artwork on instead.",
            )
            .on_hover_text(hint);
        }
    });

    if mapping.projection == Projection::Uv {
        uv_placement(ui, mapping);
        reset_row(ui, mapping, fitted);
        return;
    }

    ui.horizontal(|ui| {
        ui.label(if mapping.projection == Projection::Cylindrical {
            "Wrap around"
        } else {
            "Facing"
        });
        for candidate in Axis::ALL {
            // Turned to read on the new face as it read on the old one.
            if ui.selectable_label(mapping.axis == candidate, candidate.label()).clicked() && mapping.axis != candidate {
                *mapping = decal_fit::rebase(mapping, mapping.projection, candidate);
            }
        }
        if let Some(fit) = fitted {
            if ui
                .button("Fit to part")
                .on_hover_text(
                    "Centre the artwork on the part and size it to fit, facing \
                     the widest face.",
                )
                .clicked()
            {
                *mapping = decal_fit::rebase(&fit, mapping.projection, fit.axis);
            }
        }
    });

    if mapping.projection == Projection::Planar {
        // A size in metres, so the number on screen is the size the label
        // will actually print at. Under a repeat count the same label is a
        // different size on every part it is applied to.
        mapping.real_world_units = true;
        // On a side face the artwork's width runs along the projection's
        // second coordinate.
        let sideways = decal_fit::sideways(mapping);
        ui.horizontal(|ui| {
            ui.label("Size");
            let scale = &mut mapping.scale;
            let (width, height) = if sideways { (&mut scale.y, &mut scale.x) } else { (&mut scale.x, &mut scale.y) };
            ui.add(
                egui::DragValue::new(width)
                    .speed(0.01)
                    .range(0.001..=1000.0)
                    .prefix("w "),
            );
            ui.add(
                egui::DragValue::new(height)
                    .speed(0.01)
                    .range(0.001..=1000.0)
                    .prefix("h "),
            );
        })
        .response
        .on_hover_text("Across the part, in metres. Larger is bigger.");
    } else {
        ui.horizontal(|ui| {
            ui.label("Wraps / height");
            ui.add(
                egui::DragValue::new(&mut mapping.scale.x)
                    .speed(0.01)
                    .range(0.01..=100.0),
            )
            .on_hover_text("1 wraps the artwork exactly once around.");
            ui.add(
                egui::DragValue::new(&mut mapping.scale.y)
                    .speed(0.01)
                    .range(0.01..=100.0),
            );
        });
    }

    ui.horizontal(|ui| {
        ui.label("Nudge");
        ui.add(
            egui::DragValue::new(&mut mapping.offset.x)
                .speed(0.005)
                .range(-100.0..=100.0),
        );
        ui.add(
            egui::DragValue::new(&mut mapping.offset.y)
                .speed(0.005)
                .range(-100.0..=100.0),
        );
    })
    .response
    .on_hover_text("Slide the artwork across the face. One unit is one artwork width.");

    rotation(ui, mapping);
    mirror(ui, mapping);
    reset_row(ui, mapping, fitted);
}

/// The controls for a label following the mesh's own coordinates.
fn uv_placement(ui: &mut egui::Ui, mapping: &mut Mapping) {
    ui.small("Follows the part's UV layout · artwork does not repeat");
    ui.horizontal(|ui| {
        ui.label("Scale U / V");
        ui.add(
            egui::DragValue::new(&mut mapping.scale.x)
                .speed(0.02)
                .range(0.01..=100.0),
        );
        ui.add(
            egui::DragValue::new(&mut mapping.scale.y)
                .speed(0.02)
                .range(0.01..=100.0),
        );
    })
    .response
    .on_hover_text("Higher values make artwork smaller. Anchored at the UV origin.");
    ui.horizontal(|ui| {
        ui.label("Offset U / V");
        ui.add(
            egui::DragValue::new(&mut mapping.offset.x)
                .speed(0.01)
                .range(-100.0..=100.0),
        );
        ui.add(
            egui::DragValue::new(&mut mapping.offset.y)
                .speed(0.01)
                .range(-100.0..=100.0),
        );
    });
    rotation(ui, mapping);
    mirror(ui, mapping);
}

/// Counted from the right way up, so an upright label reads 0 whichever
/// face it is on.
fn rotation(ui: &mut egui::Ui, mapping: &mut Mapping) {
    let upright = decal_fit::upright_turn(mapping.projection, mapping.axis);
    let mut degrees = ((mapping.rotation - upright) * 360.0 + 180.0).rem_euclid(360.0) - 180.0;
    if ui
        .add(crate::field::Slider::new(&mut degrees, -180.0..=180.0).text("Rotation °"))
        .changed()
    {
        mapping.rotation = upright + degrees / 360.0;
    }
}

/// Turn the artwork over where it sits, for a logo that reads backwards on
/// the face it is projected onto (the far side of a box, the inside of a
/// lid) or artwork that was simply drawn the wrong way round.
fn mirror(ui: &mut egui::Ui, mapping: &mut Mapping) {
    ui.horizontal(|ui| {
        ui.label("Mirror");
        if ui
            .add(egui::SelectableLabel::new(mapping.flip_u, "Horizontal"))
            .on_hover_text("Flip the artwork left to right, where it sits.")
            .clicked()
        {
            *mapping = mapping.mirrored(true);
        }
        if ui
            .add(egui::SelectableLabel::new(mapping.flip_v, "Vertical"))
            .on_hover_text("Flip the artwork top to bottom, where it sits.")
            .clicked()
        {
            *mapping = mapping.mirrored(false);
        }
    });
}

/// Back to where a freshly added label would have started.
///
/// Not to `Mapping::default()`: that is UV placement at one repeat, which on a
/// projected label means a one-scene-unit square — a speck on a car door and
/// larger than the part on a connector pin. Reset means "put it back where it
/// was put for me", so it resets to the same fit.
fn reset_row(ui: &mut egui::Ui, mapping: &mut Mapping, fitted: Option<Mapping>) {
    if ui.small_button("Reset placement").clicked() {
        *mapping = reset_to(mapping, fitted);
    }
}

/// What the placement becomes when the user picks a different mode.
///
/// Pulled out of the widget because it is the part that can be wrong. The
/// scale and offset mean different things on either side of the UV boundary —
/// repeats across a UV square on one side, a size in scene units on the other
/// — so carrying the numbers across unchanged puts the artwork at an arbitrary
/// size somewhere off the part. Switching between the two projected modes does
/// keep them, because there the numbers still mean what they meant.
fn switch_to(mapping: &Mapping, projection: Projection, fitted: Option<Mapping>) -> Mapping {
    let leaving_uv = mapping.projection == Projection::Uv && projection != Projection::Uv;
    match (leaving_uv, fitted) {
        (true, Some(fit)) => decal_fit::rebase(&fit, projection, fit.axis),
        // Between the projected modes, read the same way round on the new one.
        _ if mapping.projection != Projection::Uv && projection != Projection::Uv => {
            decal_fit::rebase(mapping, projection, mapping.axis)
        }
        _ => Mapping {
            projection,
            ..*mapping
        },
    }
}

/// Back to where a freshly added label would have started.
///
/// Not `Mapping::default()` for a projected label: that is one repeat, which
/// under real-world units means a one-scene-unit square — a speck on a car
/// door and larger than the part on a connector pin. Reset means "put it back
/// where it was put for me", so it resets to the same fit.
fn reset_to(mapping: &Mapping, fitted: Option<Mapping>) -> Mapping {
    let projection = mapping.projection;
    match (projection, fitted) {
        (Projection::Uv, _) | (_, None) => Mapping {
            projection,
            ..Default::default()
        },
        (_, Some(fit)) => decal_fit::rebase(&fit, projection, fit.axis),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};

    /// A fit standing in for one measured off a part: a size in scene units,
    /// centred somewhere well away from the origin.
    fn a_fit() -> Mapping {
        Mapping {
            projection: Projection::Planar,
            axis: Axis::Y,
            real_world_units: true,
            scale: vec3(0.8, 0.8, 1.0),
            offset: vec2(0.5 - 12.0 / 0.8, 0.5 - 6.0 / 0.8),
            pivot: vec2(12.0 / 0.8, 6.0 / 0.8),
            ..Default::default()
        }
    }

    /// Leaving UV placement takes the fitted numbers, because the ones it is
    /// leaving behind mean something else.
    ///
    /// A UV mapping's scale is a repeat count across a unit square; a
    /// projected one's is a size in scene units. Carrying "1" across turns a
    /// label covering a whole part into a one-unit square somewhere near the
    /// object's origin — which on an imported part is usually not even on the
    /// part.
    #[test]
    fn leaving_uv_placement_takes_the_fit() {
        let uv = Mapping::default();
        let projected = switch_to(&uv, Projection::Planar, Some(a_fit()));
        assert_eq!(projected.projection, Projection::Planar);
        assert_eq!(projected.scale, a_fit().scale);
        assert_eq!(projected.offset, a_fit().offset);
        assert!(projected.real_world_units);
    }

    /// With no part to measure, the mode still changes. Refusing to switch
    /// would leave the control dead with nothing on screen to say why.
    #[test]
    fn leaving_uv_placement_without_a_part_still_switches() {
        let projected = switch_to(&Mapping::default(), Projection::Planar, None);
        assert_eq!(projected.projection, Projection::Planar);
    }

    /// Between the two projected modes the placement is kept - its axis,
    /// size, pivot and the turn the user gave it, counted from the right way
    /// up - and turned to read the right way round on the new one: the
    /// coordinates round a cylinder run the other way to a flat projection's.
    ///
    /// Re-fitting here would silently discard that work every time someone
    /// tried the other mode to see how it looked.
    #[test]
    fn switching_between_projected_modes_keeps_the_placement() {
        let mine = Mapping {
            rotation: 0.1,
            offset: vec2(3.0, -1.0),
            ..a_fit()
        };
        let wrapped = switch_to(&mine, Projection::Cylindrical, Some(a_fit()));
        assert_eq!(wrapped.projection, Projection::Cylindrical);
        assert_eq!(wrapped.axis, mine.axis);
        assert_eq!((wrapped.scale, wrapped.pivot), (mine.scale, mine.pivot));
        let turn = |m: &Mapping| m.rotation - decal_fit::upright_turn(m.projection, m.axis);
        let apart = (turn(&wrapped) - turn(&mine)).rem_euclid(1.0);
        assert!(apart.min(1.0 - apart) < 1e-5, "the user's turn went from {} to {}", turn(&mine), turn(&wrapped));
        // ...and back again is where it started.
        let back = switch_to(&wrapped, Projection::Planar, Some(a_fit()));
        assert!((back.offset - mine.offset).length() < 1e-4 && (back.rotation - mine.rotation).abs() < 1e-5, "{back:?}");
    }

    /// Reset puts a projected label back on the part, not onto a one-unit
    /// square at the object's origin.
    #[test]
    fn reset_returns_a_projected_label_to_its_fit() {
        let wandered = Mapping {
            rotation: 0.3,
            offset: vec2(90.0, -40.0),
            scale: vec3(0.01, 0.01, 1.0),
            ..a_fit()
        };
        let back = reset_to(&wandered, Some(a_fit()));
        assert_eq!(back.scale, a_fit().scale);
        assert_eq!(back.offset, a_fit().offset);
        assert_eq!(back.pivot, a_fit().pivot);
        assert_eq!(back.rotation, 0.0);
    }

    /// A reset keeps the mode it was reset in. Resetting a wrapped label onto
    /// a flat projection would be answering a question nobody asked.
    #[test]
    fn reset_keeps_the_mode() {
        for projection in [Projection::Planar, Projection::Cylindrical, Projection::Uv] {
            let current = Mapping {
                projection,
                rotation: 0.25,
                ..a_fit()
            };
            assert_eq!(
                reset_to(&current, Some(a_fit())).projection,
                projection,
                "reset changed the placement mode"
            );
        }
        // ...and a UV reset goes back to the plain UV defaults rather than to
        // a fit measured for a projection it is not using.
        let uv = Mapping {
            projection: Projection::Uv,
            ..a_fit()
        };
        assert_eq!(reset_to(&uv, Some(a_fit())), Mapping::default());
    }
}
