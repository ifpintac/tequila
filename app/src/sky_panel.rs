//! Sun and sky, in the Environment tab.
//!
//! The controls are the two a photographer shooting outdoors actually has: the
//! time of day, which is where the sun is, and the weather, which is how much
//! haze is in the air. Everything else about the sky follows from the physics
//! and is not a knob.
//!
//! Sun position is the same compass bearing and height the light pins use, for
//! the same reason: nobody positions a light by editing a unit vector.

use crate::pin_panel::{angles_of, direction_from};
use crate::project::Project;
use eframe::egui;
use tequila_render_core::sky::Sky;
use tequila_scene::Command;

use tequila_render_core::sky::TIMES_OF_DAY as TIMES;

/// A sky for one of the named times of day, falling back to the default.
///
/// Shared with the buttons rather than duplicated beside them, so a capture or
/// a script gets the same sky the panel does.
pub fn preset(name: &str) -> Sky {
    tequila_render_core::sky::time_of_day(name).unwrap_or_default()
}

pub fn show(ui: &mut egui::Ui, project: &mut Project) {
    let current = project.document().environment.sky;
    let mut next = current;
    let mut changed = false;

    crate::field::Fold::new("Sun & sky")
        .default_open(current.is_some())
        .show(ui, |ui| {
            let mut enabled = next.is_some();
            if ui
                .add(crate::field::Checkbox::new(&mut enabled, "Light the scene with a sky"))
                .on_hover_text(
                    "Replaces the environment image with a physical sky. Light \
                     pins still work on top of it.",
                )
                .changed()
            {
                next = enabled.then(Sky::default);
                changed = true;
            }
            let Some(sky) = next.as_mut() else {
                ui.weak(
                    "Off. The scene is lit by the built-in studio, or by an \
                     image you load above.",
                );
                return;
            };

            ui.horizontal_wrapped(|ui| {
                for (name, bearing, height, hint) in TIMES {
                    let _ = (bearing, height);
                    if ui.button(name).on_hover_text(hint).clicked() {
                        let chosen = preset(name);
                        sky.sun_direction = chosen.sun_direction;
                        sky.turbidity = chosen.turbidity;
                        sky.sun_disc = chosen.sun_disc;
                        changed = true;
                    }
                }
            });

            let (bearing, height) = angles_of(sky.sun_direction);
            let (mut bearing_deg, mut height_deg) = (bearing.to_degrees(), height.to_degrees());
            let mut moved = false;
            moved |= ui
                .add(crate::field::Slider::new(&mut bearing_deg, -180.0..=180.0).text("Sun around °"))
                .on_hover_text("Which way the sun is. 0 is behind the camera.")
                .changed();
            moved |= ui
                .add(crate::field::Slider::new(&mut height_deg, -10.0..=89.0).text("Sun height °"))
                .on_hover_text(
                    "How high. Below zero is dusk: the sun is gone and only the \
                     sky is left.",
                )
                .changed();
            if moved {
                sky.sun_direction = direction_from(bearing_deg.to_radians(), height_deg.to_radians());
                changed = true;
            }

            changed |= ui
                .add(crate::field::Slider::new(&mut sky.turbidity, 1.0..=10.0).text("Haze"))
                .on_hover_text(
                    "Aerosol in the air. Low is a clear day with a deep blue \
                     sky; high is white and even, and softens every shadow.",
                )
                .changed();
            changed |= ui
                .add(
                    crate::field::Slider::new(&mut sky.intensity, 0.05..=8.0)
                        .text("Brightness")
                        .logarithmic(true),
                )
                .changed();
            changed |= ui
                .add(crate::field::Slider::new(&mut sky.ground_albedo, 0.0..=1.0).text("Ground bounce"))
                .on_hover_text(
                    "How much light the ground throws back up. Concrete is \
                     about 0.2; fresh snow is near 0.8.",
                )
                .changed();
            changed |= ui
                .add(crate::field::Checkbox::new(&mut sky.sun_disc, "Sun in the sky"))
                .on_hover_text(
                    "The sun's own disc. Off leaves the skylight without the \
                     hard shadow, which is what an overcast day is.",
                )
                .changed();
        });

    if changed && next != current {
        if let Err(error) = project.apply(Command::SetSky {
            sky: Box::new(next),
        }) {
            project.status = error.to_string();
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        project.end_interaction();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shipped time of day has to put the sun somewhere the sky model
    /// can use, and produce a sky that is not black.
    #[test]
    fn every_time_of_day_lights_something() {
        for (name, bearing, height, _) in TIMES {
            let sky = Sky {
                sun_direction: direction_from(bearing.to_radians(), height.to_radians()),
                turbidity: if name == "Overcast" { 9.0 } else { 2.0 },
                ..Default::default()
            };
            let up = sky.radiance(tequila_core::math::vec3(0.0, 1.0, 0.0));
            assert!(
                up.luminance() > 0.001,
                "{name} gives a black sky overhead ({up:?})"
            );
            assert!(
                sky.sun().y > 0.0,
                "{name} puts the sun below the horizon"
            );
        }
    }

    /// Golden hour has to be redder than midday, or the button is a lie.
    #[test]
    fn golden_hour_is_warmer_than_midday() {
        let at = |name: &str| {
            let (_, bearing, height, _) = TIMES.iter().find(|t| t.0 == name).copied().unwrap();
            Sky {
                sun_direction: direction_from(bearing.to_radians(), height.to_radians()),
                ..Default::default()
            }
        };
        let warmth = |sky: Sky| {
            let c = sky.radiance(sky.sun());
            c.r / (c.b + 1e-9)
        };
        assert!(
            warmth(at("Golden hour")) > warmth(at("Midday")) * 2.0,
            "golden hour is {} against midday's {}",
            warmth(at("Golden hour")),
            warmth(at("Midday"))
        );
    }
}
