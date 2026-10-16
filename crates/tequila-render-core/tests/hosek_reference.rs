//! The Hosek-Wilkie port against the authors' own code.
//!
//! `data/hosek_reference.txt` was printed by the reference implementation
//! (version 1.4a, compiled unmodified with MSVC) over six turbidities, three
//! albedos, five solar elevations and six view directions for the RGB sky,
//! and eleven bands of the spectral sky and sun. A port that reads one table
//! entry off by one, or interpolates a corner the wrong way round, is a sky
//! that looks plausible and is wrong; these hold it to the reference.

use tequila_render_core::hosek::{xyz_to_linear_srgb, HosekSky};

fn lines(kind: &str) -> Vec<Vec<f64>> {
    include_str!("data/hosek_reference.txt")
        .lines()
        .filter(|line| line.starts_with(kind))
        .map(|line| line.split_whitespace().skip(1).map(|v| v.parse().unwrap()).collect())
        .collect()
}

fn close(got: f64, want: f64, tolerance: f64) -> bool {
    (got - want).abs() <= tolerance * want.abs().max(1e-12)
}

#[test]
fn the_sky_matches_the_reference_code() {
    let rows = lines("rgb ");
    assert_eq!(rows.len(), 6 * 3 * 5 * 6);
    for row in rows {
        let (t, a, e, theta, gamma) = (row[0], row[1], row[2], row[3], row[4]);
        let got = HosekSky::new(t, a, e).radiance(theta, gamma);
        for c in 0..3 {
            assert!(
                close(got[c], row[5 + c], 1e-9),
                "turbidity {t}, albedo {a}, elevation {e}, theta {theta}, gamma {gamma}: \
                 channel {c} is {} against {}",
                got[c],
                row[5 + c]
            );
        }
    }
}

#[test]
fn the_sun_matches_the_reference_code() {
    let rows = lines("spec ");
    assert_eq!(rows.len(), 6 * 4 * 11);
    for row in rows {
        let (t, e, wavelength, want) = (row[0], row[2], row[5], row[7]);
        let got = HosekSky::new(t, 0.2, e).solar_radiance(wavelength, 0.0);
        assert!(
            close(got, want, 1e-9),
            "turbidity {t}, elevation {e}, {wavelength} nm: the sun is {got} against {want}"
        );
    }
}

/// The sun is only published per wavelength, and it is taken to RGB here the
/// way the authors say their tristimulus sky was made: through the CIE
/// observer. That is only right if their RGB sky is in the same units, so the
/// same conversion applied to their *spectral* sky has to land on their RGB
/// sky. It does not land exactly — the RGB data is a separate fit — but a
/// units mismatch would miss by a factor, not by percent. Measured: within
/// 1.5% in 21 of the 24 cases, 5% in two more, and 0.82 for the clearest air
/// with the sun nearly overhead, the corner of the fit.
#[test]
fn the_spectral_sky_through_the_observer_is_the_rgb_sky() {
    let spectral = lines("spec ");
    let rgb = lines("rgb ");
    for group in spectral.chunks(11) {
        let (t, a, e, theta, gamma) = (group[0][0], group[0][1], group[0][2], group[0][3], group[0][4]);
        // The reference's own linear interpolation between bands.
        let at = |wavelength: f64| {
            let x = (wavelength - 320.0) / 40.0;
            let low = (x as usize).min(9);
            let f = x - low as f64;
            (1.0 - f) * group[low][6] + f * group[low + 1][6]
        };
        let mut xyz = [0.0f64; 3];
        for step in 0..340 {
            let wavelength = 380.5 + step as f64;
            let cmf = tequila_materials::dispersion::cie_xyz(wavelength as f32);
            for c in 0..3 {
                xyz[c] += at(wavelength) * cmf[c] as f64;
            }
        }
        let converted = xyz_to_linear_srgb(xyz);
        let reference = rgb
            .iter()
            .find(|r| r[0] == t && r[1] == a && r[2] == e && r[3] == theta && r[4] == gamma)
            .expect("the RGB reference covers the spectral one");
        let luminance = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let ratio = luminance(converted) / luminance([reference[5], reference[6], reference[7]]);
        assert!(
            (0.78..1.1).contains(&ratio),
            "turbidity {t}, elevation {e}: the spectral sky converts to {ratio:.3} of the RGB sky"
        );
    }
}
