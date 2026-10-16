//! The Hosek-Wilkie sky and sun (Hosek & Wilkie 2012, *An Analytic Model for
//! Full Spectral Sky-Dome Radiance*; 2013, *Adding a Solar Radiance Function
//! to the Hosek Skylight Model*).
//!
//! A port of the authors' reference implementation, version 1.4a, with their
//! fitted data in [`crate::hosek_data`]. The sky dome is the model's RGB
//! variant — linear sRGB, fitted to a brute-force path-traced atmosphere with
//! multiple scattering and ground bounce. The sun is the spectral solar
//! radiance function, which the authors publish only per wavelength band,
//! taken to linear sRGB through the CIE observer the way they state their
//! tristimulus data was made, so the sun and the sky share units and their
//! ratio is the model's, not a guess.
//!
//! `tests/hosek_reference.rs` holds the port to the reference code's own
//! output, to twelve digits.

use crate::hosek_data::{LIMB_DARKENING, SKY_RGB, SKY_RGB_RADIANCE, SOLAR};
use std::f64::consts::PI;

/// The sun's angular radius the model was fitted with: 0.51 degrees across.
pub const SOLAR_RADIUS: f64 = 0.51 * PI / 180.0 / 2.0;

/// The sky and sun for one turbidity, ground albedo and solar elevation.
#[derive(Clone, Copy, Debug)]
pub struct HosekSky {
    configs: [[f64; 9]; 3],
    radiances: [f64; 3],
    turbidity: f64,
    elevation: f64,
}

/// The quintic Bézier the model interpolates over solar elevation with, on
/// the cube root of the elevation's share of a right angle.
fn over_elevation(control: impl Fn(usize) -> f64, s: f64) -> f64 {
    let t = 1.0 - s;
    t.powi(5) * control(0)
        + 5.0 * t.powi(4) * s * control(1)
        + 10.0 * t.powi(3) * s.powi(2) * control(2)
        + 10.0 * t.powi(2) * s.powi(3) * control(3)
        + 5.0 * t * s.powi(4) * control(4)
        + s.powi(5) * control(5)
}

/// Blend a quantity over the four corners of albedo (0, 1) and turbidity
/// (its integer below and above), as the reference does. `at(albedo, t)`
/// gives the corner for albedo index 0 or 1 and zero-based turbidity `t`.
fn over_corners(turbidity: f64, albedo: f64, at: impl Fn(usize, usize) -> f64) -> f64 {
    let whole = turbidity as usize;
    let rest = turbidity - whole as f64;
    let mut value = (1.0 - albedo) * (1.0 - rest) * at(0, whole - 1) + albedo * (1.0 - rest) * at(1, whole - 1);
    if whole == 10 {
        return value;
    }
    value += (1.0 - albedo) * rest * at(0, whole) + albedo * rest * at(1, whole);
    value
}

impl HosekSky {
    /// `turbidity` from 1 (the clearest air) to 10 (thick haze), `albedo` of
    /// the ground from 0 to 1, `elevation` of the sun in radians above the
    /// horizon. Each is held to the range the model was fitted over.
    pub fn new(turbidity: f64, albedo: f64, elevation: f64) -> HosekSky {
        let turbidity = if turbidity.is_finite() { turbidity.clamp(1.0, 10.0) } else { 2.0 };
        let albedo = if albedo.is_finite() { albedo.clamp(0.0, 1.0) } else { 0.0 };
        let elevation = if elevation.is_finite() { elevation.clamp(0.0, PI / 2.0) } else { 0.0 };
        let s = (elevation / (PI / 2.0)).powf(1.0 / 3.0);
        let mut configs = [[0.0; 9]; 3];
        let mut radiances = [0.0; 3];
        for channel in 0..3 {
            let data = &SKY_RGB[channel];
            for (i, config) in configs[channel].iter_mut().enumerate() {
                *config = over_corners(turbidity, albedo, |a, t| {
                    let base = 9 * 6 * 10 * a + 9 * 6 * t;
                    over_elevation(|k| data[base + i + 9 * k], s)
                });
            }
            let data = &SKY_RGB_RADIANCE[channel];
            radiances[channel] = over_corners(turbidity, albedo, |a, t| {
                let base = 6 * 10 * a + 6 * t;
                over_elevation(|k| data[base + k], s)
            });
        }
        HosekSky { configs, radiances, turbidity, elevation }
    }

    /// Sky radiance in linear sRGB, `theta` from the zenith to the view and
    /// `gamma` from the sun to the view, both in radians. The horizon is the
    /// model's edge: below it, it is held at the horizon's value.
    pub fn radiance(&self, theta: f64, gamma: f64) -> [f64; 3] {
        let cos_theta = theta.cos().max(0.0);
        let cos_gamma = gamma.cos();
        let mut out = [0.0; 3];
        for (channel, value) in out.iter_mut().enumerate() {
            let c = &self.configs[channel];
            let exp_m = (c[4] * gamma).exp();
            let ray_m = cos_gamma * cos_gamma;
            let mie_m = (1.0 + cos_gamma * cos_gamma) / (1.0 + c[8] * c[8] - 2.0 * c[8] * cos_gamma).powf(1.5);
            let zenith = cos_theta.sqrt();
            let shape = (1.0 + c[0] * (c[1] / (cos_theta + 0.01)).exp())
                * (c[2] + c[3] * exp_m + c[5] * ray_m + c[6] * mie_m + c[7] * zenith);
            *value = shape * self.radiances[channel];
        }
        out
    }

    /// The sun's direct radiance in W/(m² sr nm) at `wavelength` (320 to 720
    /// nm), `gamma` radians from the centre of its disc, before the sky in
    /// front of it. Zero off the disc.
    pub fn solar_radiance(&self, wavelength: f64, gamma: f64) -> f64 {
        if gamma > SOLAR_RADIUS {
            return 0.0;
        }
        let wavelength = wavelength.clamp(320.0, 720.0);
        let mut turb_low = self.turbidity as usize - 1;
        let mut turb_frac = self.turbidity - (turb_low + 1) as f64;
        if turb_low == 9 {
            turb_low = 8;
            turb_frac = 1.0;
        }
        let (mut wl_low, mut wl_frac) = (((wavelength - 320.0) / 40.0) as usize, (wavelength % 40.0) / 40.0);
        if wl_low == 10 {
            wl_low = 9;
            wl_frac = 1.0;
        }
        let band = |t: usize, w: usize| solar_band(t, w, self.elevation);
        let direct = (1.0 - turb_frac)
            * ((1.0 - wl_frac) * band(turb_low, wl_low) + wl_frac * band(turb_low, wl_low + 1))
            + turb_frac * ((1.0 - wl_frac) * band(turb_low + 1, wl_low) + wl_frac * band(turb_low + 1, wl_low + 1));

        let limb: [f64; 6] =
            std::array::from_fn(|i| (1.0 - wl_frac) * LIMB_DARKENING[wl_low][i] + wl_frac * LIMB_DARKENING[wl_low + 1][i]);
        let ar2 = 1.0 / SOLAR_RADIUS.sin().powi(2);
        let sample_cosine = (1.0 - ar2 * gamma.sin().powi(2)).max(0.0).sqrt();
        let darkening: f64 = limb.iter().enumerate().map(|(i, c)| c * sample_cosine.powi(i as i32)).sum();
        direct * darkening
    }

    /// The sun's irradiance at the ground, facing it, in the sky's linear
    /// sRGB units: its spectral radiance integrated over the disc (limb
    /// darkening and all) and through the CIE 1931 observer.
    pub fn solar_irradiance_rgb(&self) -> [f64; 3] {
        // Rings across the disc, each weighted by the solid angle it covers.
        // Five-nanometre steps and eight rings: the spectrum is the model's
        // forty-nanometre bands, linearly joined, and the limb darkening a
        // smooth polynomial, so finer changes nothing but the cost.
        const RINGS: usize = 8;
        const STEP: f64 = 5.0;
        let mut xyz = [0.0f64; 3];
        for step in 0..((720.0 - 380.0) / STEP) as usize {
            let wavelength = 380.0 + (step as f64 + 0.5) * STEP;
            let mut over_disc = 0.0;
            for ring in 0..RINGS {
                let inner = SOLAR_RADIUS * ring as f64 / RINGS as f64;
                let outer = SOLAR_RADIUS * (ring + 1) as f64 / RINGS as f64;
                let solid_angle = 2.0 * PI * (inner.cos() - outer.cos());
                over_disc += self.solar_radiance(wavelength, 0.5 * (inner + outer)) * solid_angle;
            }
            let cmf = tequila_materials::dispersion::cie_xyz(wavelength as f32);
            for c in 0..3 {
                xyz[c] += over_disc * cmf[c] as f64 * STEP;
            }
        }
        xyz_to_linear_srgb(xyz)
    }
}

/// One band's direct solar radiance at the disc's centre: a cubic in the
/// elevation, on one of forty-five pieces spaced on its cube root.
fn solar_band(turbidity: usize, band: usize, elevation: f64) -> f64 {
    const PIECES: usize = 45;
    const ORDER: usize = 4;
    let pos = (((2.0 * elevation / PI).powf(1.0 / 3.0) * PIECES as f64) as usize).min(PIECES - 1);
    let break_x = (pos as f64 / PIECES as f64).powi(3) * (PI * 0.5);
    let top = ORDER * PIECES * turbidity + ORDER * (pos + 1) - 1;
    let x = elevation - break_x;
    (0..ORDER).map(|i| SOLAR[band][top - i] * x.powi(i as i32)).sum()
}

/// CIE XYZ to linear sRGB (D65).
pub fn xyz_to_linear_srgb(xyz: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = xyz;
    [
        3.2404542 * x - 1.5371385 * y - 0.4985314 * z,
        -0.9692660 * x + 1.8760108 * y + 0.0415560 * z,
        0.0556434 * x - 0.2040259 * y + 1.0572252 * z,
    ]
}
