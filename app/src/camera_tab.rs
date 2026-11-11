//! The Camera tab (`TEQUILA_SPEC.md` §1.7, `docs/01` §8).
//!
//! §8.1 is the rule this panel is built around: "users of this application
//! think in photographic terms and will notice if f/2.8 does not behave like
//! f/2.8". So the controls are focal length in millimetres, f-number, and a
//! focus distance in scene units — never a field of view in radians or an
//! aperture radius, both of which are derived.
//!
//! The camera list and the viewport are the same thing seen twice. The
//! viewport's orbit state *is* the active saved camera: orbiting writes to it,
//! selecting another reads from it. Keeping a separate "viewport camera" that
//! saved views were copied in and out of would mean a user could orbit, switch
//! camera, switch back, and find their view gone.

use eframe::egui;
use tequila_render_core::camera::{Aperture, Projection, FULL_FRAME_SENSOR_WIDTH_MM};
use tequila_scene::lighting::{
    aperture_shapes, lens_types, match_perspective_three_point, match_perspective_two_point,
};
use tequila_scene::Command;

use crate::orbit::OrbitCamera;
use crate::project::Project;

#[derive(Default)]
pub struct CameraTab {
    renaming: Option<(usize, String)>,
    /// The vanishing points of a Match Perspective fit, in film coordinates.
    /// Held here rather than in the document because they are scaffolding for
    /// one action, not part of the scene.
    match_points: Option<[egui::Pos2; 3]>,
    /// Whether the third point is in play. Two is the level-camera case and
    /// three is the general one, and they are separate controls because a user
    /// with a level backplate should not have to find a third vanishing point
    /// that is off at infinity.
    three_point: bool,
    /// What the last fit said, so a refusal is visible rather than silent.
    match_report: String,
    pub show_grid: bool,
}

impl CameraTab {
    /// Returns true when the viewport camera has to be re-read from the
    /// document — a camera was selected, or a lens control was changed.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        project: &mut Project,
        orbit: &mut OrbitCamera,
        aspect: f32,
    ) -> bool {
        let mut adopt = false;
        // The saved cameras are the page's head: the rest is about the one
        // looked through.
        crate::theme::section(ui, "Cameras");
        self.camera_list(ui, project, orbit, &mut adopt);
        ui.add_space(6.0);

        let active = project.document().cameras.active_index();
        let mut saved = project.document().cameras.active().clone();
        let before = saved.clone();

        // The sections, in the person's own order.
        let mut page = crate::sections::Page::new(ui, "camera");
        for key in page.order() {
            match key {
                "lens" => page.section(ui, key, "Lens", |ui| {
                    crate::field::Fold::new("Lens").id_salt("camera-lens-section").aside(lens_label(saved.camera.projection)).default_open(true).show(ui, |ui| {
                    // ---- Lens ------------------------------------------------------
                    let mut lens = saved.camera.projection;
                    crate::field::row(ui, "Kind", |ui| {
                        crate::field::combo(ui, "camera-lens", lens_label(lens), |ui| {
                            for (label, candidate) in lens_types() {
                                let selected = std::mem::discriminant(&lens)
                                    == std::mem::discriminant(&candidate);
                                if ui.selectable_label(selected, label).clicked() && !selected {
                                    lens = candidate;
                                }
                            }
                        });
                    });
                    // Switching *to* orthographic adopts the height the camera was
                    // already framing, rather than the 1.0 the catalog offers as a
                    // placeholder. A scene is rarely one unit tall, so the placeholder
                    // meant the view leapt to a wild close-up on every switch — and then
                    // could not be zoomed back out, because zoom used to move the camera
                    // and an orthographic camera does not care where it is.
                    if !matches!(saved.camera.projection, Projection::Orthographic { .. }) {
                        if let Projection::Orthographic { scale } = &mut lens {
                            *scale = orbit.framed_height(aspect).max(1e-4);
                        }
                    }
                    saved.camera.projection = lens;

                    match &mut saved.camera.projection {
                        Projection::Perspective { focal_length_mm }
                        | Projection::Shift {
                            focal_length_mm, ..
                        } => {
                            // Dragged across the lenses people own, and typed for the
                            // rest: a focal length is a number people know — "an 85" —
                            // rather than a position on a scale, and it is the field
                            // §1.2's expression entry was written for: `= 36/1.5` is a
                            // sensor width over a crop factor, which is how the number is
                            // arrived at. A click types; any unit, `2in`, is taken.
                            ui.add(
                                crate::field::Slider::new(focal_length_mm, 8.0..=400.0)
                                    .text("Focal length")
                                    .suffix(" mm")
                                    .logarithmic(true)
                                    .unit(crate::numeric::Unit::Millimetres)
                                    .clamping(egui::SliderClamping::Never),
                            )
                            .on_hover_text(
                                "Drag, or click and type a length in any unit (`2in`), or an \
                                 expression after `=` (`= 36/1.5`).",
                            );
                            *focal_length_mm = focal_length_mm.clamp(1.0, 2000.0);
                        }
                        Projection::Orthographic { scale } => {
                            ui.add(
                                crate::field::Slider::new(scale, 0.01..=10_000.0)
                                    .text("Height")
                                    .logarithmic(true),
                            );
                        }
                        Projection::Equirectangular => {
                            crate::theme::note(ui, "A 360° camera sees all round, so it has no focal length.");
                        }
                    }
                    if let Projection::Shift {
                        shift_x, shift_y, ..
                    } = &mut saved.camera.projection
                    {
                        ui.add(crate::field::Slider::new(shift_x, -1.0..=1.0).text("Shift X"));
                        ui.add(crate::field::Slider::new(shift_y, -1.0..=1.0).text("Shift Y"));
                    }

                    // The field of view, shown and editable, but derived: it is the focal
                    // length and the sensor that are physical, and editing one has to move
                    // the other rather than the two drifting apart.
                    //
                    // Neither exists for an orthographic camera, whose rays are parallel.
                    // They used to be drawn anyway, reading a meaningless 1 degree beside
                    // a Height that was the only control doing anything.
                    if !matches!(saved.camera.projection, Projection::Orthographic { .. }) {
                        let mut fov = saved.camera.vertical_fov(aspect).to_degrees();
                        if ui
                            .add(crate::field::Slider::new(&mut fov, 1.0..=150.0).text("Field of view").suffix("°"))
                            .changed()
                        {
                            saved.camera.set_vertical_fov(fov.to_radians(), aspect);
                        }
                        ui.horizontal(|ui| {
                            let button = 78.0;
                            crate::field::column(ui, ui.available_width() - button - ui.spacing().item_spacing.x, |ui| {
                                ui.add(
                                    crate::field::Slider::new(&mut saved.camera.sensor_width_mm, 4.0..=80.0)
                                        .text("Sensor width")
                                        .suffix(" mm"),
                                )
                            });
                            if ui
                                .add_sized([button, 26.0], egui::Button::new("Full frame"))
                                .on_hover_text("36 mm: a 35 mm film frame, and most professional cameras")
                                .clicked()
                            {
                                saved.camera.sensor_width_mm = FULL_FRAME_SENSOR_WIDTH_MM;
                            }
                        });
                    }

                    });
                }),
                "focus" => page.section(ui, key, "Focus", |ui| {
                    crate::field::Fold::new("Focus").id_salt("camera-focus").default_open(true).show(ui, |ui| {
                    // ---- Depth of field --------------------------------------------
                    let mut dof = saved.camera.f_number.is_finite();
                    if ui
                        .add(crate::field::Checkbox::new(&mut dof, "Blur what is out of focus"))
                        .on_hover_text("Depth of field: sharp at the focus distance and softer in front of it and behind, as a real lens is. Off, everything is sharp.")
                        .changed()
                    {
                        // Switching off is infinity — a pinhole — rather than a very large
                        // number, so `aperture_radius` is exactly zero and a disabled
                        // control cannot leave a trace of blur behind.
                        saved.camera.f_number = if dof { 2.8 } else { f32::INFINITY };
                    }
                    if dof {
                        let mut f_number = saved.camera.f_number;
                        if ui
                            .add(
                                crate::field::Slider::new(&mut f_number, 0.7..=32.0)
                                    .text("Aperture")
                                    .prefix("f/")
                                    .logarithmic(true),
                            )
                            .on_hover_text("The f-number. A low one (f/1.8) blurs a lot and keeps little in focus; a high one (f/16) keeps nearly everything sharp.")
                            .changed()
                        {
                            saved.camera.f_number = f_number;
                        }
                        ui.add(
                            crate::field::Slider::new(&mut saved.camera.focus_distance, 0.01..=10_000.0)
                                .text("Focus distance")
                                .logarithmic(true),
                        );
                        ui.add(crate::field::Checkbox::new(&mut saved.camera.keep_focus, "Keep focus"))
                            .on_hover_text(
                                "Re-derive the focus distance from the camera's distance to its target \
                                 whenever it moves, so orbiting a subject does not drift it out of focus.",
                            );

                        let mut aperture = saved.camera.aperture;
                        crate::field::row(ui, "Blur shape", |ui| {
                            crate::field::combo(ui, "camera-aperture", aperture_label(aperture), |ui| {
                                for (label, candidate) in aperture_shapes() {
                                    let selected = std::mem::discriminant(&aperture)
                                        == std::mem::discriminant(&candidate);
                                    if ui.selectable_label(selected, label).clicked() && !selected {
                                        aperture = candidate;
                                    }
                                }
                            });
                        })
                        .response
                        .on_hover_text("The shape of the lens opening, which out-of-focus highlights take: round, or the blades of an iris.");
                        if let Aperture::Bladed { blades, rotation } = &mut aperture {
                            ui.add(crate::field::Slider::new(blades, 3..=12).text("Blades"));
                            ui.add(
                                crate::field::Slider::new(rotation, 0.0..=std::f32::consts::TAU).text("Blade angle"),
                            );
                        }
                        saved.camera.aperture = aperture;
                    }

                    });
                }),
                "exposure" => page.section(ui, key, "Exposure", |ui| {
                    crate::field::Fold::new("Exposure").id_salt("camera-exposure").default_open(true).show(ui, |ui| {
                    // ---- Exposure ---------------------------------------------------
                    // ISO and shutter only once they mean something: off, the image's
                    // Exposure is the whole of it.
                    ui.add(crate::field::Checkbox::new(&mut saved.camera.physical_exposure, "Expose as a camera does"))
                        .on_hover_text("Expose by ISO, shutter and f-number as a camera would, on top of the Image tab's Exposure. Doubling the shutter time adds a stop; so does opening up one f-stop.");
                    if saved.camera.physical_exposure {
                        ui.add(
                            crate::field::Slider::new(&mut saved.camera.iso, 25.0..=6400.0)
                                .text("ISO")
                                .logarithmic(true),
                        );
                        let mut shutter = 1.0 / saved.camera.shutter_speed_seconds.max(1e-6);
                        if ui
                            .add(
                                crate::field::Slider::new(&mut shutter, 1.0..=8000.0)
                                    .text("Shutter")
                                    .prefix("1/")
                                    .logarithmic(true),
                            )
                            .changed()
                        {
                            saved.camera.shutter_speed_seconds = 1.0 / shutter.max(1.0);
                        }
                        crate::theme::note(ui, format!("{:+.1} stops on the picture's brightness", saved.camera.exposure_stops()));
                    } else {
                        crate::theme::note(ui, "Off: the picture's brightness is the Image page's Exposure alone.");
                    }

                    });
                }),
                "guides" => page.section(ui, key, "Guides", |ui| {
                    crate::field::Fold::new("Guides").id_salt("camera-guides").default_open(true).show(ui, |ui| {
                    // ---- Overlays ---------------------------------------------------
                    ui.add(crate::field::Checkbox::new(&mut self.show_grid, "Ground grid")).on_hover_text("A measured grid on the floor, to judge sizes and line things up by. Never in a render. Snapping to it is in the tools at the view's left.");

                    });
                }),
                "match" => page.section(ui, key, "Match perspective", |ui| self.match_perspective(ui, &mut saved, aspect)),
                _ => {}
            }
        }
        page.finish(ui);

        if saved != before {
            let _ = project.apply(Command::SetCamera {
                index: active,
                camera: Box::new(saved),
            });
            adopt = true;
        }
        adopt
    }

    fn camera_list(
        &mut self,
        ui: &mut egui::Ui,
        project: &mut Project,
        orbit: &OrbitCamera,
        adopt: &mut bool,
    ) {
        let active = project.document().cameras.active_index();
        let cameras: Vec<(usize, String)> = project
            .document()
            .cameras
            .iter()
            .map(|(i, c)| (i, c.name.clone()))
            .collect();
        let count = cameras.len();

        let mut select = None;
        let mut remove = None;
        let mut rename = None;
        for (index, name) in cameras {
            // A row a camera, as the Scene's tree has a row a part: the one
            // looked through filled in the accent, its tools at the row's
            // end. Chips of different widths read as buttons, not a list.
            if let Some((editing, text)) = self.renaming.as_mut().filter(|(editing, _)| *editing == index) {
                let _ = editing;
                let response = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                if !response.has_focus() && !response.lost_focus() {
                    response.request_focus();
                }
                if response.lost_focus() {
                    rename = Some((index, text.clone()));
                }
                continue;
            }
            let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), egui::Sense::click());
            let p = crate::theme::p();
            let on = index == active;
            if ui.is_rect_visible(rect) {
                if on {
                    ui.painter().rect_filled(rect, 6.0, crate::theme::accent_fill());
                } else if response.hovered() {
                    ui.painter().rect_filled(rect, 6.0, p.hover.gamma_multiply(0.55));
                }
                crate::icons::paint(
                    ui.painter(),
                    egui::Rect::from_center_size(egui::pos2(rect.left() + 15.0, rect.center().y), egui::Vec2::splat(14.0)),
                    crate::icons::Icon::Camera,
                    if on { p.accent_text } else { p.text_dim },
                );
                let mut job = egui::text::LayoutJob::simple_singleline(name.clone(), egui::FontId::proportional(crate::theme::BODY), p.text);
                job.wrap = egui::text::TextWrapping { max_width: (rect.width() - 30.0 - 56.0).max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
                let galley = ui.fonts(|f| f.layout_job(job));
                ui.painter().galley(egui::pos2(rect.left() + 30.0, rect.center().y - galley.size().y * 0.5), galley, egui::Color32::PLACEHOLDER);
            }
            // The tools over the row's end, drawn after it so they take
            // their own clicks.
            let remove_at = egui::Rect::from_center_size(egui::pos2(rect.right() - 15.0, rect.center().y), egui::Vec2::splat(22.0));
            let rename_at = remove_at.translate(egui::vec2(-24.0, 0.0));
            let shown = on || response.hovered() || ui.rect_contains_pointer(rect);
            let pencil = ui.interact(rename_at, response.id.with("rename"), egui::Sense::click());
            if shown {
                crate::icons::paint(ui.painter(), rename_at.shrink(4.0), crate::icons::Icon::Pencil, if pencil.hovered() { p.text } else { p.text_dim });
            }
            if pencil.on_hover_text("Rename").clicked() || response.double_clicked() {
                self.renaming = Some((index, name.clone()));
            } else if response.clicked() && !on {
                select = Some(index);
            }
            // The last camera cannot be removed: the viewport has to be
            // looking through something, and an empty list would make
            // "the active camera" a question with no answer.
            if count > 1 {
                let cross = ui.interact(remove_at, response.id.with("remove"), egui::Sense::click());
                if shown {
                    crate::icons::paint(ui.painter(), remove_at.shrink(4.0), crate::icons::Icon::Close, if cross.hovered() { p.text } else { p.text_dim });
                }
                if cross.on_hover_text("Remove this camera").clicked() {
                    remove = Some(index);
                }
            }
            response.on_hover_text(if on { "The camera the view looks through. Double-click to rename it." } else { "Look through this camera" });
        }

        if ui
            .button("+ Save this view as a camera")
            .on_hover_text("Keep the view as it is now, lens and focus included, to come back to")
            .clicked()
        {
            let mut saved = orbit.to_saved(format!("Camera {}", count + 1));
            saved.name = format!("Camera {}", count + 1);
            let _ = project.apply(Command::AddCamera {
                camera: Box::new(saved),
            });
        }

        if let Some((index, name)) = rename {
            self.renaming = None;
            let _ = project.apply(Command::RenameCamera { index, name });
        }
        if let Some(index) = select {
            let _ = project.apply(Command::SelectCamera { index });
            *adopt = true;
        }
        if let Some(index) = remove {
            let _ = project.apply(Command::RemoveCamera { index });
            *adopt = true;
        }
    }

    /// Match Perspective (§1.7): recover a focal length from two vanishing
    /// points picked on a backplate.
    fn match_perspective(
        &mut self,
        ui: &mut egui::Ui,
        saved: &mut tequila_scene::lighting::SavedCamera,
        aspect: f32,
    ) {
        crate::field::Fold::new("Match perspective")
            .default_open(false)
            .show(ui, |ui| {
                crate::theme::note(
                    ui,
                    "To sit the model in a photograph: enter where the photograph's parallel lines \
                     meet (its vanishing points), as fractions of the frame, and the lens follows.",
                );
                ui.add(crate::field::Checkbox::new(&mut self.three_point, "Three-point"))
                    .on_hover_text(
                        "A third vanishing point recovers the lens shift as well as the \
                         focal length, which is what a tilted or cropped photograph needs. \
                         Two points assume the verticals stay vertical.",
                    );

                let mut points = self.match_points.unwrap_or([
                    egui::pos2(-0.6, 0.5),
                    egui::pos2(1.6, 0.5),
                    egui::pos2(0.5, -2.0),
                ]);
                let count = if self.three_point { 3 } else { 2 };
                for (i, label) in ["First point", "Second point", "Third point"]
                    .into_iter()
                    .take(count)
                    .enumerate()
                {
                    crate::field::row(ui, label, |ui| {
                        let width = ((ui.available_width() - ui.spacing().item_spacing.x) * 0.5).max(40.0);
                        ui.add_sized([width, 26.0], egui::DragValue::new(&mut points[i].x).speed(0.01).prefix("across  "));
                        ui.add_sized([width, 26.0], egui::DragValue::new(&mut points[i].y).speed(0.01).prefix("down  "));
                    });
                }
                self.match_points = Some(points);

                if ui.button("Fit the lens").clicked() {
                    let a = (points[0].x, points[0].y);
                    let b = (points[1].x, points[1].y);
                    let c = (points[2].x, points[2].y);
                    let sensor = saved.camera.sensor_width_mm;
                    self.match_report = if self.three_point {
                        match match_perspective_three_point(a, b, c, sensor, aspect) {
                            Some(fit) => {
                                // Three points fix the lens *and* where its
                                // axis falls on the film, which is what a
                                // tilted or cropped photograph needs — so the
                                // result is a shift lens rather than a plain
                                // perspective one.
                                saved.camera.projection = Projection::Shift {
                                    focal_length_mm: fit.focal_length_mm,
                                    shift_x: fit.shift_x,
                                    shift_y: fit.shift_y,
                                };
                                format!(
                                    "{:.0} mm, shifted ({:+.3}, {:+.3})",
                                    fit.focal_length_mm, fit.shift_x, fit.shift_y
                                )
                            }
                            None => "Those three points do not describe a perspective.".into(),
                        }
                    } else {
                        match match_perspective_two_point(a, b, sensor, aspect) {
                            Some(focal_length_mm) => {
                                // Only the focal length: two points fix the
                                // lens, not where the camera stands. Moving the
                                // camera from them as well would be inventing
                                // the half of the answer they do not contain.
                                saved.camera.projection = match saved.camera.projection {
                                    Projection::Shift {
                                        shift_x, shift_y, ..
                                    } => Projection::Shift {
                                        focal_length_mm,
                                        shift_x,
                                        shift_y,
                                    },
                                    _ => Projection::Perspective { focal_length_mm },
                                };
                                format!("{focal_length_mm:.0} mm")
                            }
                            None => "Those two points do not describe a perspective.".into(),
                        }
                    };
                }
                if !self.match_report.is_empty() {
                    crate::theme::note(ui, self.match_report.clone());
                }
            });
    }
}

fn lens_label(projection: Projection) -> &'static str {
    lens_types()
        .into_iter()
        .find(|(_, p)| std::mem::discriminant(p) == std::mem::discriminant(&projection))
        .map(|(label, _)| label)
        .unwrap_or("Perspective")
}

fn aperture_label(aperture: Aperture) -> &'static str {
    aperture_shapes()
        .into_iter()
        .find(|(_, a)| std::mem::discriminant(a) == std::mem::discriminant(&aperture))
        .map(|(label, _)| label)
        .unwrap_or("Circular")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every lens the model supports has to be offered, or a projection that
    /// exists in the renderer is unreachable from the application.
    #[test]
    fn every_lens_type_has_a_label() {
        for (label, projection) in lens_types() {
            assert_eq!(lens_label(projection), label);
        }
        assert_eq!(lens_types().len(), 4, "§1.7 specifies four lens types");
    }

    #[test]
    fn every_aperture_shape_has_a_label() {
        for (label, aperture) in aperture_shapes() {
            assert_eq!(aperture_label(aperture), label);
        }
    }
}
