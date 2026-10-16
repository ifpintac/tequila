//! A physical sky, for the scenes that are not in a studio (`docs/01` §1.5).
//!
//! ## The model
//!
//! Hosek and Wilkie's (SIGGRAPH 2012, IEEE CG&A 2013), ported in
//! [`crate::hosek`] from the authors' reference code with their fitted data,
//! and held to that code's own output by `tests/hosek_reference.rs`. It is a
//! fit to a brute-force path-traced atmosphere, so it has what an analytic
//! single-scattering sky does not: light scattered more than once, which
//! brightens a hazy day and the sky opposite the sun, and light the ground
//! throws back up. The sun is the model's solar radiance function, limb
//! darkening and all, taken to RGB through the CIE observer so that it and
//! the sky are in the same units and their ratio is the model's.
//!
//! The model's domain is a sun at or above the horizon and turbidity from 1
//! to 10. Below the horizon the sky is held at its sunset pattern and faded,
//! a stop and a half per degree, which is a stand-in for twilight rather
//! than a model of it; the time-of-day presets never go there.
//!
//! ## Where it lands
//!
//! A sky produces the equirectangular image the environment is built from, so
//! everything downstream is unchanged: it is importance-sampled by the same
//! CDF, and light pins composite on top of it exactly as they do over a studio
//! or a loaded HDRI.

use crate::hosek::HosekSky;
use tequila_core::math::Vec3;
use tequila_core::spectrum::Spectrum;
use tequila_texture::image::{ColorSpace, Image};

/// The sun's angular radius, the one the model was fitted with: about a
/// quarter of a degree.
const SUN_ANGULAR_RADIUS: f32 = crate::hosek::SOLAR_RADIUS as f32;

/// The solid angle that disc covers, in steradians -- about 62 millionths.
///
/// Radiance is power per unit solid angle, so the same power crammed into this
/// tiny cone is what makes looking at the sun different from looking at the
/// sky beside it.
const SUN_SOLID_ANGLE: f32 = std::f32::consts::PI * SUN_ANGULAR_RADIUS * SUN_ANGULAR_RADIUS;

/// The model's units to the scene's, at a `sun_intensity` of 1.
///
/// The model is in W/(m² sr nm), and a clear midday zenith comes out near
/// 10. The application is composed around a studio whose mean radiance is
/// about a quarter, so this is chosen, not derived: a default sky opens at
/// about the studio's exposure, as the single-scattering sky before it did,
/// and `a_sky_exposes_roughly_like_the_studio` holds it there. It scales sun
/// and sky together, so nothing about their balance is chosen.
const UNITS: f32 = 0.0047;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sky {
    /// Where the sun is, as a unit direction with `+Y` up.
    pub sun_direction: Vec3,
    /// Overall multiplier, after the physics.
    pub intensity: f32,
    /// Haze, as the model's turbidity: 2 is a clear day, 3 a normal one, 6 a
    /// hazy one and 10 the thickest the model knows. Held to 1 to 10.
    pub turbidity: f32,
    /// How much light the ground throws back up. The model's own input: it
    /// lifts the lower sky, and it lights the ground below the horizon.
    pub ground_albedo: f32,
    /// Whether the sun's own disc is drawn.
    pub sun_disc: bool,
    /// Brightness of the whole sky, sun included, as a multiple of
    /// [`UNITS`]. Kept apart from `intensity` because files written before
    /// the model changed carry it; 4 is the default.
    pub sun_intensity: f32,
}

impl Default for Sky {
    fn default() -> Self {
        Sky {
            // Mid-afternoon, about 35 degrees up.
            sun_direction: Vec3 {
                x: 0.45,
                y: 0.574,
                z: -0.685,
            },
            intensity: 1.0,
            turbidity: 2.0,
            ground_albedo: 0.2,
            sun_disc: true,
            sun_intensity: 4.0,
        }
    }
}

impl Sky {
    /// The sun, as a unit vector, however badly it was set.
    pub fn sun(&self) -> Vec3 {
        if self.sun_direction.length() > 1e-6 {
            self.sun_direction.normalize()
        } else {
            Vec3::Y
        }
    }

    /// How high the sun is above the horizon, in radians. Negative at night.
    pub fn sun_elevation(&self) -> f32 {
        self.sun().y.clamp(-1.0, 1.0).asin()
    }

    /// The model for this sky's sun, haze and ground.
    fn model(&self) -> HosekSky {
        HosekSky::new(
            self.turbidity as f64,
            self.ground_albedo as f64,
            self.sun_elevation().max(0.0) as f64,
        )
    }

    /// The scene units the model's numbers are multiplied into, with the fade
    /// below the horizon: a stop and a half per degree of depression.
    fn scale(&self) -> f32 {
        let elevation = self.sun_elevation().to_degrees();
        let fade = if elevation < 0.0 { (elevation * 1.04).exp() } else { 1.0 };
        UNITS * self.sun_intensity.max(0.0) * self.intensity * fade
    }

    /// Radiance arriving from `direction`, looking out from the ground.
    pub fn radiance(&self, direction: Vec3) -> Spectrum {
        let view = if direction.length() > 1e-6 {
            direction.normalize()
        } else {
            return Spectrum::ZERO;
        };
        // One direction needs only what it sees: the ground below the
        // horizon, the sun's power on its disc. Both are integrals, and a
        // sweep of the sky asks this thousands of times.
        let on_disc = self.sun_disc && view.y >= 0.0 && view.dot(self.sun()) > SUN_ANGULAR_RADIUS.cos();
        let model = self.model();
        let sun = if on_disc || view.y < 0.0 { self.sun_power(&model) } else { Spectrum::ZERO };
        let ground = if view.y < 0.0 {
            self.skylight_on_the_ground(&model, sun) * self.ground_albedo
        } else {
            Spectrum::ZERO
        };
        let prepared = Prepared { model, sun, ground };
        let mut total = self.radiance_with(view, &prepared);
        if on_disc {
            total = total + prepared.sun * (1.0 / SUN_SOLID_ANGLE);
        }
        total
    }

    /// Everything about the sky that does not depend on where one looks,
    /// worked out once. The sun's power alone is an integral over three
    /// hundred wavelengths and the disc; worked out per texel of the ground,
    /// as it briefly was, it took a 512-wide sky from 11 ms to six seconds.
    fn prepare(&self) -> Prepared {
        let model = self.model();
        let sun = self.sun_power(&model);
        let ground = self.skylight_on_the_ground(&model, sun) * self.ground_albedo;
        Prepared { model, sun, ground }
    }

    /// The sun's power per unit area, arriving at the ground: the model's
    /// solar radiance over its disc, in scene units. Divided by a solid angle
    /// it becomes a radiance: by the sun's own for the analytic disc, by a
    /// texel's for the image. Nothing once the sun has set.
    fn sun_power(&self, model: &HosekSky) -> Spectrum {
        if self.sun_elevation() <= 0.0 {
            return Spectrum::ZERO;
        }
        let rgb = model.solar_irradiance_rgb();
        positive(Spectrum::from_linear_srgb(tequila_core::math::vec3(rgb[0] as f32, rgb[1] as f32, rgb[2] as f32)))
            * self.scale()
    }

    /// The sky's own radiance, without the sun's disc.
    fn dome(&self, view: Vec3, model: &HosekSky) -> Spectrum {
        let sun = self.sun();
        // Below the horizon the model's pattern is the sunset's, so the angle
        // to the sun is measured to where it set.
        let sun = if sun.y < 0.0 {
            Vec3 { x: sun.x, y: 0.0, z: sun.z }.try_normalize().unwrap_or(Vec3::X)
        } else {
            sun
        };
        let theta = view.y.clamp(-1.0, 1.0).acos() as f64;
        let gamma = view.dot(sun).clamp(-1.0, 1.0).acos() as f64;
        let rgb = model.radiance(theta, gamma);
        positive(Spectrum::from_linear_srgb(tequila_core::math::vec3(rgb[0] as f32, rgb[1] as f32, rgb[2] as f32)))
            * self.scale()
    }

    /// Sky above the horizon, ground below it.
    fn radiance_with(&self, view: Vec3, prepared: &Prepared) -> Spectrum {
        let model = &prepared.model;
        // The sky term, evaluated along the horizon for anything at or below
        // it so that the two sides meet at the same value.
        let sky_view = if view.y > 0.0 {
            view
        } else {
            Vec3 { x: view.x, y: 0.0, z: view.z }.try_normalize().unwrap_or(Vec3::X)
        };
        let sky = self.dome(sky_view, model);
        if view.y >= 0.0 {
            return sky;
        }

        // Below the horizon there is ground, which the model does not draw.
        // The far distance is all haze — ground at a grazing angle is seen
        // through so much air that it *is* the horizon sky — so the ground
        // starts from exactly the sky beside it and darkens towards
        // albedo-times-skylight as the view turns downward. Continuous at the
        // horizon by construction: a ground that began at its own value drew
        // a hard line straight across the render.
        let down = (-view.y).clamp(0.0, 1.0);
        let t = down * down * (3.0 - 2.0 * down);
        sky * (1.0 - t) + prepared.ground * t
    }

    /// What lights the ground, as the radiance a white ground would send
    /// back: the sky, averaged over four directions 45 degrees up (a uniform
    /// sky of radiance L puts πL on the ground, which a Lambertian ground
    /// returns as L), and the sun's irradiance on level ground, over π.
    fn skylight_on_the_ground(&self, model: &HosekSky, sun_power: Spectrum) -> Spectrum {
        let mut total = Spectrum::ZERO;
        for i in 0..4 {
            let angle = i as f32 / 4.0 * std::f32::consts::TAU;
            let direction = Vec3 { x: angle.cos() * 0.7, y: 0.7, z: angle.sin() * 0.7 };
            total = total + self.dome(direction.normalize(), model);
        }
        // The sun is most of it: three quarters of the light on level ground
        // on a clear afternoon. Left out, the ground was a fifth as bright
        // as it is outdoors, and blue — which products standing on a catcher
        // reflect and are lit by from below.
        let sun = sun_power * (self.sun().y.max(0.0) / std::f32::consts::PI);
        total * 0.25 + sun
    }

    /// The sky as an equirectangular image, ready for the environment.
    ///
    /// The sun is put in by hand rather than sampled, and the difference is
    /// energy. Its disc is a quarter of a degree across; a texel of a
    /// 256-wide equirectangular image is about one and a half degrees. Asking
    /// [`Sky::radiance`] at texel centres therefore misses the sun entirely
    /// almost always, and lands a radiance meant for a tiny cone on a texel
    /// hundreds of times larger on the rare occasion it hits -- either way the
    /// total power is wrong, which is how a sky ends up with no sun in it or
    /// with a sun that lights the scene like a floodlight.
    ///
    /// So the disc's *power* is spread over exactly the texel that contains
    /// it: radiance divided by that texel's solid angle rather than by the
    /// sun's. The flux is right, and the shadow it casts is as sharp as the
    /// resolution allows -- softer than reality at 256 wide, sharper as the
    /// image grows.
    pub fn to_image(&self, width: usize, height: usize) -> Image {
        use rayon::prelude::*;

        let prepared = self.prepare();
        let data: Vec<f32> = (0..height)
            .into_par_iter()
            .flat_map_iter(|y| {
                let v = (y as f32 + 0.5) / height as f32;
                let mut row = Vec::with_capacity(width * 3);
                for x in 0..width {
                    let u = (x as f32 + 0.5) / width as f32;
                    let direction = tequila_core::sampling::equirect_to_direction(
                        tequila_core::math::vec2(u, v),
                    );
                    let radiance = self.radiance_with(direction, &prepared);
                    row.extend_from_slice(&[radiance.r, radiance.g, radiance.b]);
                }
                row
            })
            .collect();

        let mut image = Image {
            width,
            height,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        };
        if self.sun_disc {
            let sun = self.sun();
            let uv = tequila_core::sampling::direction_to_equirect(sun);
            let x = ((uv.x * width as f32) as usize).min(width - 1);
            let y = ((uv.y * height as f32) as usize).min(height - 1);
            // The solid angle this texel covers: the equirectangular
            // Jacobian, which shrinks towards the poles.
            let theta = (y as f32 + 0.5) / height as f32 * std::f32::consts::PI;
            let solid_angle = (std::f32::consts::TAU / width as f32)
                * (std::f32::consts::PI / height as f32)
                * theta.sin().max(1e-4);
            let added = prepared.sun * (1.0 / solid_angle);
            let at = (y * width + x) * 3;
            image.data[at] += added.r.max(0.0);
            image.data[at + 1] += added.g.max(0.0);
            image.data[at + 2] += added.b.max(0.0);
        }
        image
    }
}

/// A sky's direction-independent parts; see [`Sky::prepare`].
struct Prepared {
    model: HosekSky,
    /// The sun's power at the ground, in scene units.
    sun: Spectrum,
    /// The ground's radiance: its albedo times the sun and sky on it.
    ground: Spectrum,
}

/// Clamp at zero: the conversion into the rendering space can take a deep
/// sunset red a hair below it in one channel.
fn positive(c: Spectrum) -> Spectrum {
    Spectrum { r: c.r.max(0.0), g: c.g.max(0.0), b: c.b.max(0.0) }
}

/// Times of day worth one click, as a bearing and a height for the sun.
///
/// Named rather than numbered because "golden hour" is a thing people ask for
/// and six degrees above the horizon is not. Here rather than in the panel
/// that draws the buttons, so the headless renderer can be told the same
/// things.
pub const TIMES_OF_DAY: [(&str, f32, f32, &str); 4] = [
    ("Midday", 0.0, 78.0, "High and hard. Short shadows, strong contrast."),
    (
        "Afternoon",
        35.0,
        40.0,
        "Enough angle to shape a product without long shadows.",
    ),
    (
        "Golden hour",
        -60.0,
        8.0,
        "Low, warm and long-shadowed - the light the atmosphere reddens on \
         its way through.",
    ),
    (
        "Overcast",
        20.0,
        55.0,
        "Heavy haze. A soft even wash with almost no shadow, like a huge \
         softbox overhead.",
    ),
];

/// A sky for one of [`TIMES_OF_DAY`], or `None` if the name is not one.
pub fn time_of_day(name: &str) -> Option<Sky> {
    let (label, bearing, height, _) = TIMES_OF_DAY
        .iter()
        .find(|(label, _, _, _)| label.eq_ignore_ascii_case(name))
        .copied()?;
    Some(Sky {
        sun_direction: crate::pins::direction_from(bearing.to_radians(), height.to_radians()),
        turbidity: if label == "Overcast" { 9.0 } else { 2.0 },
        sun_disc: label != "Overcast",
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// Sun well up but not overhead, so "look at the zenith" is not the same
    /// as "look at the sun". Aimed at the zenith, every test below about the
    /// colour of the sky would instead be measuring the sun's own aureole,
    /// which is white and enormous — as the first version of these tests
    /// discovered.
    fn noon() -> Sky {
        Sky {
            sun_direction: vec3(0.7, 0.71, 0.0),
            sun_disc: false,
            ..Default::default()
        }
    }

    /// How blue a colour is relative to its own brightness, so the comparison
    /// survives one direction being far brighter than another.
    fn blueness(c: Spectrum) -> f32 {
        let total = c.r + c.g + c.b;
        if total <= 0.0 {
            0.0
        } else {
            c.b / total
        }
    }

    /// The sky is Hosek and Wilkie's: above the horizon, in every direction
    /// and every channel, it is their model times one exposure scale — the
    /// multiply scattered, ground-lit sky their path-traced atmosphere gives,
    /// not a single-scattering approximation of it.
    #[test]
    fn the_sky_is_the_hosek_wilkie_model() {
        for (sun, turbidity, albedo) in [(vec3(0.7, 0.71, 0.0), 2.0f32, 0.2f32), (vec3(0.2, 0.1, -0.97), 6.5, 0.6)] {
            let sky = Sky {
                sun_direction: sun,
                turbidity,
                ground_albedo: albedo,
                sun_disc: false,
                ..Default::default()
            };
            let sun = sky.sun();
            let model = crate::hosek::HosekSky::new(turbidity as f64, albedo as f64, sun.y.asin() as f64);
            let mut ratios = Vec::new();
            for direction in [vec3(0.0, 1.0, 0.0), vec3(0.6, 0.45, 0.2), vec3(-0.9, 0.1, 0.3), vec3(0.1, 0.3, -0.9)] {
                let view = direction.normalize();
                let expected = model.radiance(
                    view.y.clamp(-1.0, 1.0).acos() as f64,
                    view.dot(sun).clamp(-1.0, 1.0).acos() as f64,
                );
                let got = sky.radiance(view).to_linear_srgb();
                for c in 0..3 {
                    ratios.push(got[c] as f64 / expected[c]);
                }
            }
            let first = ratios[0];
            assert!(first > 0.0, "the sky is black");
            for ratio in &ratios {
                assert!(
                    (ratio / first - 1.0).abs() < 2e-3,
                    "the sky is not a scaled Hosek-Wilkie sky: ratios {ratios:?}"
                );
            }
        }
    }

    /// The sky is blue, and it is blue because of the 1/λ⁴ in the scattering
    /// coefficients rather than because anything says so.
    ///
    /// This is the first thing to fail if the Rayleigh constants are entered in
    /// the wrong order, which is the mistake a table-driven model would hide.
    #[test]
    fn the_sky_is_blue_overhead() {
        let sky = noon();
        let zenith = sky.radiance(vec3(0.0, 1.0, 0.0));
        assert!(
            zenith.b > zenith.g && zenith.g > zenith.r,
            "the zenith came out {zenith:?}, which is not blue"
        );
        assert!(
            blueness(zenith) > 0.45,
            "the zenith is only {} blue",
            blueness(zenith)
        );
    }

    /// And paler at the horizon, where the long path through the air has
    /// scattered so much that every channel arrives.
    #[test]
    fn the_horizon_is_paler_than_the_zenith() {
        let sky = noon();
        let zenith = sky.radiance(vec3(0.0, 1.0, 0.0));
        let horizon = sky.radiance(vec3(1.0, 0.03, 0.0).normalize());
        assert!(
            blueness(horizon) < blueness(zenith) - 0.02,
            "the horizon ({}) is no paler than the zenith ({})",
            blueness(horizon),
            blueness(zenith)
        );
    }

    /// A low sun reddens, which is the same 1/λ⁴ working the other way: the
    /// long slanting path scatters the blue out of the direct beam.
    #[test]
    fn a_setting_sun_reddens() {
        let high = Sky {
            sun_direction: vec3(0.0, 1.0, 0.0),
            sun_disc: true,
            ..Default::default()
        };
        let low = Sky {
            sun_direction: vec3(1.0, 0.03, 0.0),
            sun_disc: true,
            ..Default::default()
        };

        let midday = high.radiance(high.sun());
        let sunset = low.radiance(low.sun());
        assert!(
            midday.b > 0.0 && sunset.b >= 0.0,
            "no light from the sun at all: {midday:?} and {sunset:?}"
        );

        let redness = |c: Spectrum| c.r / (c.b + 1e-9);
        assert!(
            redness(sunset) > redness(midday) * 3.0,
            "the setting sun is {} red against {} at midday; it should be far \
             redder",
            redness(sunset),
            redness(midday)
        );
    }

    /// There is a glow around the sun, because Mie scattering is forward.
    ///
    /// Without it the sky is a smooth dome with a dot in it, which is what an
    /// isotropic phase function gives and what an aerosol layer does not.
    #[test]
    fn there_is_a_glow_around_the_sun() {
        let sky = Sky {
            sun_direction: vec3(0.3, 0.5, -0.81),
            sun_disc: false,
            ..Default::default()
        };
        let sun = sky.sun();
        // Ten degrees off the sun, against ninety degrees off it, both well
        // above the horizon so the comparison is not really about altitude.
        let near = (sun + vec3(0.17, 0.0, 0.0)).normalize();
        let far = vec3(-sun.z, sun.y, sun.x).normalize();

        let near_brightness = sky.radiance(near).luminance();
        let far_brightness = sky.radiance(far).luminance();
        assert!(
            near_brightness > far_brightness * 1.3,
            "just off the sun reads {near_brightness} against {far_brightness} \
             away from it; there is no forward scattering"
        );
    }

    /// Haze makes the sky paler, not bluer. Mie scattering is achromatic, so
    /// adding it washes the colour out.
    #[test]
    fn haze_desaturates_the_sky() {
        let clear = Sky {
            turbidity: 1.0,
            ..noon()
        };
        let hazy = Sky {
            turbidity: 8.0,
            ..noon()
        };
        let direction = vec3(0.6, 0.45, 0.0).normalize();
        assert!(
            blueness(hazy.radiance(direction)) < blueness(clear.radiance(direction)),
            "turbidity 8 is no paler than turbidity 1"
        );
    }

    /// The sun's disc is vastly brighter than the sky beside it — which is the
    /// reason the environment's importance sampling exists at all.
    #[test]
    fn the_suns_disc_is_far_brighter_than_the_sky() {
        let sky = Sky {
            sun_direction: vec3(0.0, 0.8, -0.6),
            sun_disc: true,
            ..Default::default()
        };
        let on_sun = sky.radiance(sky.sun()).luminance();
        // Two degrees off, which is well outside the quarter-degree disc.
        let beside = sky
            .radiance((sky.sun() + vec3(0.035, 0.0, 0.0)).normalize())
            .luminance();
        assert!(
            on_sun > beside * 50.0,
            "the disc reads {on_sun} against {beside} beside it; either it is \
             not being drawn or it is not a sun"
        );
    }

    /// Below the horizon there is ground, lit by the sun as well as the sky
    /// and scaled by its albedo. The sun is most of it — three quarters of
    /// the light on level ground on a clear afternoon, by the model — so
    /// sunlit ground at a fifth albedo is about as bright as the sky
    /// overhead, as it is outdoors, and grey rather than sky-blue. Lit by the
    /// sky alone it was a fifth as bright and blue, which a chrome part on a
    /// catcher showed at once: it reflects this ground.
    #[test]
    fn the_ground_is_sunlit_as_bright_as_the_sky_and_not_black() {
        let sky = time_of_day("midday").expect("a listed time");
        let below = sky.radiance(vec3(0.0, -1.0, 0.0));
        let above = sky.radiance(vec3(0.0, 1.0, 0.0)).luminance();
        let ratio = below.luminance() / above;
        assert!((0.5..3.0).contains(&ratio), "the ground is {ratio:.2} of the sky overhead");
        let rgb = below.to_linear_srgb();
        assert!(rgb.z < rgb.x * 1.1, "sunlit ground came out sky-coloured: {rgb:?}");

        let sky = Sky { ground_albedo: 0.3, ..noon() };
        let below = sky.radiance(vec3(0.2, -0.9, 0.1).normalize()).luminance();
        assert!(below > 0.0, "the ground is black");

        // And the albedo controls it.
        let darker = Sky {
            ground_albedo: 0.05,
            ..sky
        };
        assert!(
            darker.radiance(vec3(0.2, -0.9, 0.1).normalize()).luminance() < below,
            "the ground albedo does nothing"
        );
    }

    /// The horizon is a horizon, not a seam.
    ///
    /// The ground and the sky are different calculations and they meet at
    /// elevation zero. Switched at that line rather than blended, the meeting
    /// draws a hard edge straight across the render — which a golden-hour
    /// screenshot showed at once, and which no test then in place could see.
    #[test]
    fn nothing_jumps_across_the_horizon() {
        for sun_height in [0.1f32, 0.5, 0.95] {
            let sky = Sky {
                sun_direction: vec3((1.0f32 - sun_height * sun_height).sqrt(), sun_height, 0.0),
                sun_disc: false,
                ..Default::default()
            };
            // Walk down through the horizon and require each step to be small
            // next to the values either side of it.
            let mut previous: Option<Spectrum> = None;
            let mut worst = 0.0f32;
            for i in 0..=80 {
                let y = 0.2 - i as f32 * 0.005;
                let here = sky.radiance(vec3(0.3, y, -0.9).normalize());
                if let Some(before) = previous {
                    let scale = (before.luminance() + here.luminance()).max(1e-6);
                    worst = worst.max((before.luminance() - here.luminance()).abs() / scale);
                }
                previous = Some(here);
            }
            assert!(
                worst < 0.2,
                "sun at {sun_height}: one step across the horizon changed the sky by {worst} of itself, which draws a line across the image"
            );
        }
    }

    /// The sky is smooth everywhere, not just across the horizon.
    ///
    /// Not "nothing changes fast" — near a low sun the sky genuinely changes
    /// by a tenth of itself per degree, from extinction along a long slanting
    /// path, and a fixed tolerance cannot tell that from a step. What tells
    /// them apart is *refinement*: halve the sampling and a gradient's biggest
    /// jump halves with it, while a discontinuity does not move at all.
    ///
    /// So this measures the worst step at one degree, a quarter of one and a
    /// sixteenth, and requires each refinement to shrink it. Not by four: the
    /// Hosek-Wilkie sky has a √cos θ term whose slope is infinite at the
    /// horizon, so there a step halves when the sampling quarters (measured:
    /// 0.31, 0.18, 0.10, 0.05 down to a sixty-fourth of a degree, with the
    /// sun three degrees up). Continuous all the same, which is the claim; a
    /// seam would not shrink at all. It is what caught the
    /// horizon being under-sampled — twenty-four evenly spaced steps along a
    /// ray that crosses six hundred kilometres of air step straight over the
    /// dense part, and the sky quadrupled in brightness over two degrees.
    #[test]
    fn the_sky_has_no_steps_in_it() {
        for sun_height in [0.05f32, 0.3, 0.9] {
            let sky = Sky {
                sun_direction: vec3((1.0f32 - sun_height * sun_height).sqrt(), sun_height, 0.0),
                sun_disc: false,
                turbidity: 3.0,
                ..Default::default()
            };

            let worst_step = |samples: usize| {
                let mut worst = 0.0f32;
                for azimuth_step in 0..12 {
                    let azimuth = azimuth_step as f32 / 12.0 * std::f32::consts::TAU;
                    let mut previous: Option<f32> = None;
                    for i in 0..=samples {
                        let elevation = i as f32 / samples as f32
                            * std::f32::consts::FRAC_PI_2
                            * 0.98;
                        let direction = vec3(
                            elevation.cos() * azimuth.cos(),
                            elevation.sin(),
                            elevation.cos() * azimuth.sin(),
                        );
                        // Away from the sun itself: the forward-scattering
                        // lobe is sharp enough that the aureole would dominate
                        // this measurement, and it is physics either way.
                        if direction.normalize().dot(sky.sun()) > 0.2f32.cos() {
                            previous = None;
                            continue;
                        }
                        let here = sky.radiance(direction).luminance();
                        if let Some(before) = previous {
                            let scale = (before + here).max(1e-9);
                            worst = worst.max((before - here).abs() / scale);
                        }
                        previous = Some(here);
                    }
                }
                worst
            };

            let coarse = worst_step(90);
            let fine = worst_step(360);
            let finer = worst_step(1440);
            assert!(coarse > 0.0, "the sky does not vary at all");
            assert!(
                fine < coarse * 0.7 && finer < fine * 0.7,
                "sun at {sun_height}: sweeping four times as finely only took the worst step from {coarse} to {fine}, and sixteen times to {finer}. A slope shrinks as it is sampled more finely; this is a step."
            );
        }
    }

    /// A sky exposes roughly like the studio it replaces.
    ///
    /// Not a physical claim — daylight is far brighter than a softbox — but a
    /// usability one. Turning the sky on should not need three stops of
    /// exposure compensation before the render can be judged, and at the first
    /// value chosen here it needed exactly that: a default sky was ten times
    /// the studio's mean, midday was thirty-six times, and a third of a
    /// golden-hour render clipped to white.
    ///
    /// The bounds are wide because the spread between times of day is real and
    /// worth keeping. They are there to catch the scale being wrong by an
    /// order of magnitude, which is what happened.
    #[test]
    fn a_sky_exposes_roughly_like_the_studio() {
        // The built-in studio's mean radiance, which the rest of the
        // application is composed around.
        const STUDIO: f32 = 0.2576;

        let mean = |sky: Sky| {
            let image = sky.to_image(128, 64);
            image.data.iter().copied().sum::<f32>() / image.data.len() as f32
        };

        let default = mean(Sky::default()) / STUDIO;
        assert!(
            (0.4..=2.5).contains(&default),
            "a default sky is {default} times the studio's brightness; it should open at about the same exposure"
        );

        for (name, _, _, _) in TIMES_OF_DAY {
            let sky = time_of_day(name).expect("a listed time");
            let ratio = mean(sky) / STUDIO;
            assert!(
                (0.15..=6.0).contains(&ratio),
                "{name} is {ratio} times the studio's brightness, which is more than a few stops away from anything else in the application"
            );
        }
    }

    /// Nothing anywhere may be negative or non-finite: one NaN texel poisons
    /// the environment's CDF, and a poisoned CDF fails as a light that is
    /// sampled always or never.
    #[test]
    fn every_direction_is_finite_and_positive() {
        for elevation in [-1.5f32, -0.2, 0.0, 0.01, 0.6, 1.57] {
            let sky = Sky {
                sun_direction: vec3(elevation.cos(), elevation.sin(), 0.0),
                turbidity: 6.0,
                ..Default::default()
            };
            for i in 0..64 {
                let a = i as f32 / 64.0 * std::f32::consts::TAU;
                for y in [-1.0f32, -0.5, 0.0, 0.5, 1.0] {
                    let direction = vec3(a.cos(), y, a.sin());
                    let c = sky.radiance(direction);
                    assert!(
                        c.r.is_finite() && c.g.is_finite() && c.b.is_finite(),
                        "sun at {elevation}, looking {direction:?}: {c:?}"
                    );
                    assert!(c.r >= 0.0 && c.g >= 0.0 && c.b >= 0.0, "negative: {c:?}");
                }
            }
        }
        // A zero direction must not divide by its own length.
        assert_eq!(noon().radiance(Vec3::ZERO), Spectrum::ZERO);
    }

    /// The sun carries the same power whatever the image's resolution.
    ///
    /// This is the claim `to_image` makes when it divides by the texel's solid
    /// angle rather than the sun's. Divide by the sun's and the flux scales
    /// with the texel — the sun gets four times brighter every time the image
    /// doubles, which shows up as an outdoor scene that is lit differently
    /// depending on a setting nobody associates with brightness.
    #[test]
    fn the_suns_power_does_not_depend_on_the_resolution() {
        let sky = Sky {
            sun_direction: vec3(0.4, 0.6, -0.69),
            sun_disc: true,
            ..Default::default()
        };

        let flux_at = |width: usize, height: usize| {
            let image = sky.to_image(width, height);
            let uv = tequila_core::sampling::direction_to_equirect(sky.sun());
            let x = ((uv.x * width as f32) as usize).min(width - 1);
            let y = ((uv.y * height as f32) as usize).min(height - 1);
            let at = (y * width + x) * 3;
            let theta = (y as f32 + 0.5) / height as f32 * std::f32::consts::PI;
            let solid_angle = (std::f32::consts::TAU / width as f32)
                * (std::f32::consts::PI / height as f32)
                * theta.sin().max(1e-4);
            // Power = radiance x solid angle. The sky under the sun is
            // included and is thousands of times smaller, so it does not move
            // this.
            (image.data[at] + image.data[at + 1] + image.data[at + 2]) * solid_angle
        };

        let coarse = flux_at(128, 64);
        let fine = flux_at(512, 256);
        assert!(coarse > 0.0, "no sun in the image at all");
        assert!(
            (fine - coarse).abs() / coarse < 0.05,
            "the sun carries {coarse} at 128 wide and {fine} at 512; its power depends on the resolution"
        );
    }

    /// The image is the same sky, read through the same equirectangular
    /// mapping the environment uses — so the sun is in the picture where the
    /// sun direction says it is.
    #[test]
    fn the_image_puts_the_sun_where_the_sun_is() {
        let sky = Sky {
            sun_direction: vec3(0.5, 0.6, -0.62),
            sun_disc: true,
            ..Default::default()
        };
        let image = sky.to_image(128, 64);
        let uv = tequila_core::sampling::direction_to_equirect(sky.sun());
        let x = ((uv.x * 128.0) as usize).min(127);
        let y = ((uv.y * 64.0) as usize).min(63);
        let at = (y * 128 + x) * 3;
        let there = image.data[at] + image.data[at + 1] + image.data[at + 2];

        // The opposite side of the sky, which must be far darker.
        let opposite = tequila_core::sampling::direction_to_equirect(-sky.sun());
        let ox = ((opposite.x * 128.0) as usize).min(127);
        let oy = ((opposite.y * 64.0) as usize).min(63);
        let oat = (oy * 128 + ox) * 3;
        let elsewhere = image.data[oat] + image.data[oat + 1] + image.data[oat + 2];

        assert!(
            there > elsewhere * 10.0,
            "the image reads {there} where the sun should be and {elsewhere} \
             opposite it; the sky and the mapping disagree about where that is"
        );
    }
}
