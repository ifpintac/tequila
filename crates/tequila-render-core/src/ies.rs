//! IES photometric profiles (`docs/01-RENDERER-CORE.md` §5.1).
//!
//! An IES file is a measurement: a real luminaire on a goniophotometer, its
//! intensity recorded in candela over a grid of angles. That is why §5.1 asks
//! for them rather than for another cone shape — an architectural downlight's
//! beam has a soft shoulder and a faint spill that no analytic falloff
//! reproduces, and a render that uses the manufacturer's own file is showing
//! the fitting the client is actually going to buy.
//!
//! # The format
//!
//! IESNA LM-63, in the 1986, 1991, 1995 and 2002 revisions, which differ in
//! their headers and not in the numbers. After `TILT=` come two lines of
//! metadata and then three lists: the vertical angles, the horizontal angles,
//! and the candela values, horizontal-major. The numbers are whitespace
//! separated and wrap across lines wherever the writer felt like it, so this
//! reads a *stream* of numbers rather than lines — which is the one structural
//! thing about the format worth knowing, and the reason a line-oriented parser
//! of it fails on half the files in the wild.
//!
//! # What the angles mean
//!
//! Type C photometry, which is what almost every architectural file uses: the
//! vertical angle runs from 0° straight down the luminaire's aiming axis to
//! 180° straight back up it, and the horizontal angle runs around that axis. A
//! profile covering only 0–90° is a downlight that emits nothing upwards; one
//! covering a single horizontal angle is rotationally symmetric. Both are
//! handled by clamping rather than by special cases — a lookup outside the
//! measured range returns the nearest measured value, which for the symmetric
//! case means every azimuth finds the one column that exists.

use tequila_core::math::Vec3;

/// A photometric distribution, in candela.
#[derive(Clone, Debug, PartialEq)]
pub struct IesProfile {
    /// Measured vertical angles, in degrees, ascending. 0 is the aiming axis.
    vertical: Vec<f32>,
    /// Measured horizontal angles, in degrees, ascending.
    horizontal: Vec<f32>,
    /// `horizontal.len() * vertical.len()` values, horizontal-major.
    candela: Vec<f32>,
    /// The brightest measured value, for light selection and normalization.
    peak: f32,
    /// Mean over the sphere, weighted by solid angle — the value an equivalent
    /// isotropic point light would have.
    mean: f32,
}

impl IesProfile {
    /// Read a `.ies` file's contents.
    pub fn parse(text: &str) -> Result<IesProfile, String> {
        // Everything before `TILT=` is a header of free-form keywords. The
        // tilt line itself may be followed by its own block of numbers, which
        // describes how the luminaire's output changes as it is tilted — a
        // correction this does not apply, so the block is skipped rather than
        // misread as photometry.
        let (_, rest) = text
            .split_once("TILT=")
            .ok_or_else(|| "not an IES file: no TILT= line".to_string())?;
        let mut line_end = rest.splitn(2, '\n');
        let tilt = line_end.next().unwrap_or("").trim().to_string();
        let body = line_end.next().unwrap_or("");

        let mut numbers = Numbers::new(body);
        if !tilt.eq_ignore_ascii_case("NONE") {
            // `<lamp-to-luminaire geometry> <count>` then `count` angle/factor
            // pairs.
            let _geometry = numbers.next()?;
            let count = numbers.next()? as usize;
            for _ in 0..count * 2 {
                numbers.next()?;
            }
        }

        let _lamp_count = numbers.next()?;
        let lumens_per_lamp = numbers.next()?;
        let multiplier = numbers.next()?;
        let vertical_count = numbers.next()? as usize;
        let horizontal_count = numbers.next()? as usize;
        let _photometric_type = numbers.next()?;
        let _units_type = numbers.next()?;
        let _width = numbers.next()?;
        let _length = numbers.next()?;
        let _height = numbers.next()?;
        let ballast_factor = numbers.next()?;
        // LM-63-1995 called this the ballast-lamp photometric factor and
        // expected it applied; LM-63-2002 reserved it and expects 1. Applying
        // it when it is positive is right under both readings, and a file
        // carrying a 0 there is not asking for a luminaire that emits nothing.
        let photometric_factor = numbers.next()?;
        let _input_watts = numbers.next()?;

        if vertical_count == 0 || horizontal_count == 0 {
            return Err("an IES file with no angles".into());
        }
        if vertical_count > 4096 || horizontal_count > 4096 {
            return Err("an IES file claiming an implausible number of angles".into());
        }

        let mut vertical = Vec::with_capacity(vertical_count);
        for _ in 0..vertical_count {
            vertical.push(numbers.next()?);
        }
        let mut horizontal = Vec::with_capacity(horizontal_count);
        for _ in 0..horizontal_count {
            horizontal.push(numbers.next()?);
        }

        // The candela values are absolute once scaled; the lumens field is
        // informational, and a -1 there is the file's way of saying the lamp's
        // output is not separately known. Scaling by it — which is tempting,
        // since it is right there — is how an IES light ends up a thousand
        // times too bright.
        let _ = lumens_per_lamp;
        let scale = multiplier.max(0.0)
            * if ballast_factor > 0.0 { ballast_factor } else { 1.0 }
            * if photometric_factor > 0.0 {
                photometric_factor
            } else {
                1.0
            };

        let mut candela = Vec::with_capacity(vertical_count * horizontal_count);
        for _ in 0..vertical_count * horizontal_count {
            candela.push(numbers.next()? * scale);
        }

        let peak = candela.iter().copied().fold(0.0f32, f32::max);
        if !(peak > 0.0) {
            return Err("an IES file whose every measurement is zero".into());
        }

        let mut profile = IesProfile {
            vertical,
            horizontal,
            candela,
            peak,
            mean: 0.0,
        };
        profile.mean = profile.solid_angle_mean();
        Ok(profile)
    }

    pub fn load(path: impl AsRef<std::path::Path>) -> Result<IesProfile, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("{} could not be read: {e}", path.display()))?;
        IesProfile::parse(&text)
    }

    pub fn peak(&self) -> f32 {
        self.peak
    }

    /// The intensity an isotropic light of the same total output would have.
    ///
    /// What light selection needs: a single number for "how bright is this",
    /// comparable with a point light's candela. The peak would overstate a
    /// narrow beam by orders of magnitude and have every such light sampled far
    /// more than it is worth.
    pub fn mean(&self) -> f32 {
        self.mean
    }

    /// Candela at a vertical and horizontal angle, both in degrees.
    pub fn candela_at(&self, vertical: f32, horizontal: f32) -> f32 {
        let (v0, v1, fv) = bracket(&self.vertical, vertical);
        // The horizontal axis wraps, and a file covering a quadrant or a half
        // means the rest is its mirror. Folding before the lookup is what makes
        // a 0–90° file light all four quadrants instead of three of them black.
        let span = self
            .horizontal
            .last()
            .copied()
            .unwrap_or(0.0)
            .max(1e-6);
        let folded = fold(horizontal, span);
        let (h0, h1, fh) = bracket(&self.horizontal, folded);

        let at = |h: usize, v: usize| self.candela[h * self.vertical.len() + v];
        let a = at(h0, v0) * (1.0 - fv) + at(h0, v1) * fv;
        let b = at(h1, v0) * (1.0 - fv) + at(h1, v1) * fv;
        a * (1.0 - fh) + b * fh
    }

    /// The measured table: vertical angles and horizontal angles in degrees,
    /// and candela, horizontal-major (`horizontal.len() * vertical.len()`).
    pub fn table(&self) -> (&[f32], &[f32], &[f32]) {
        (&self.vertical, &self.horizontal, &self.candela)
    }

    /// Candela along a direction in the luminaire's own frame, where `-Z` is
    /// the aiming axis and `+X` is the zero of the horizontal angle.
    pub fn candela_towards(&self, local: Vec3) -> f32 {
        let direction = local.try_normalize().unwrap_or(-Vec3::Z);
        // 0° is straight down the aiming axis.
        let vertical = (-direction.z).clamp(-1.0, 1.0).acos().to_degrees();
        let horizontal = direction.y.atan2(direction.x).to_degrees();
        let horizontal = if horizontal < 0.0 {
            horizontal + 360.0
        } else {
            horizontal
        };
        self.candela_at(vertical, horizontal)
    }

    /// Mean over the sphere, weighted by `sin(theta)` so each measurement
    /// counts for the solid angle it covers.
    fn solid_angle_mean(&self) -> f32 {
        let mut total = 0.0f64;
        let mut weight = 0.0f64;
        const STEPS: usize = 64;
        for i in 0..STEPS {
            let theta = (i as f32 + 0.5) / STEPS as f32 * 180.0;
            let sin = theta.to_radians().sin() as f64;
            for j in 0..STEPS {
                let phi = (j as f32 + 0.5) / STEPS as f32 * 360.0;
                total += self.candela_at(theta, phi) as f64 * sin;
                weight += sin;
            }
        }
        if weight > 0.0 {
            (total / weight) as f32
        } else {
            self.peak
        }
    }
}

/// Where `value` falls in an ascending table: the two indices and the fraction
/// between them, clamped at both ends.
fn bracket(table: &[f32], value: f32) -> (usize, usize, f32) {
    if table.len() < 2 {
        return (0, 0, 0.0);
    }
    if value <= table[0] {
        return (0, 0, 0.0);
    }
    let last = table.len() - 1;
    if value >= table[last] {
        return (last, last, 0.0);
    }
    let upper = table.partition_point(|entry| *entry <= value);
    let lower = upper - 1;
    let span = (table[upper] - table[lower]).max(1e-9);
    (lower, upper, (value - table[lower]) / span)
}

/// Fold a horizontal angle into the measured range.
///
/// A file measuring 0–90° describes a luminaire with two mirror planes; one
/// measuring 0–180° has one. Mirroring rather than clamping is what the format
/// means by a partial sweep, and clamping instead leaves three quadrants lit by
/// whatever the last measured column happened to be.
fn fold(angle: f32, span: f32) -> f32 {
    let mut a = angle.rem_euclid(360.0);
    if span <= 0.0 {
        return 0.0;
    }
    if span < 359.0 && a > span {
        // Reflect about the span, repeatedly, until inside it.
        let period = if span <= 90.5 { 180.0 } else { 360.0 };
        a = a.rem_euclid(period);
        if a > span {
            a = (period - a).abs();
        }
        if a > span {
            a = span;
        }
    }
    a
}

/// A stream of whitespace-separated numbers.
///
/// The format wraps its lists across lines wherever the writer chose to, so
/// there is nothing to be gained by tracking lines and a great deal to be lost.
struct Numbers<'a> {
    parts: std::str::SplitAsciiWhitespace<'a>,
}

impl<'a> Numbers<'a> {
    fn new(text: &'a str) -> Numbers<'a> {
        Numbers {
            parts: text.split_ascii_whitespace(),
        }
    }

    fn next(&mut self) -> Result<f32, String> {
        let part = self
            .parts
            .next()
            .ok_or_else(|| "the IES file ends in the middle of its data".to_string())?;
        // Some writers separate with commas as well as spaces.
        part.trim_end_matches(',')
            .parse::<f32>()
            .map_err(|_| format!("`{part}` is not a number"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A downlight: 0–90° vertical, rotationally symmetric, cosine-ish falloff.
    fn downlight() -> String {
        let mut text = String::from("IESNA:LM-63-2002\n[TEST] synthetic\nTILT=NONE\n");
        // 1 lamp, -1 lumens (absolute), multiplier 1, 10 vertical, 1 horizontal,
        // type C, units, sizes; then ballast, future, watts.
        text.push_str("1 -1 1 10 1 1 2 0 0 0\n1 1 100\n");
        for i in 0..10 {
            text.push_str(&format!("{} ", i as f32 * 10.0));
        }
        text.push('\n');
        text.push_str("0\n");
        for i in 0..10 {
            let theta = (i as f32 * 10.0).to_radians();
            text.push_str(&format!("{} ", 1000.0 * theta.cos().max(0.0)));
        }
        text.push('\n');
        text
    }

    #[test]
    fn a_profile_reads_its_angles_and_values() {
        let profile = IesProfile::parse(&downlight()).unwrap();
        assert_eq!(profile.vertical.len(), 10);
        assert_eq!(profile.horizontal.len(), 1);
        assert!((profile.peak() - 1000.0).abs() < 1.0);
        // Straight down the axis is the peak; ninety degrees off it is nothing.
        assert!((profile.candela_at(0.0, 0.0) - 1000.0).abs() < 1.0);
        assert!(profile.candela_at(90.0, 0.0) < 1.0);
        // And in between it interpolates rather than stepping.
        let five = profile.candela_at(5.0, 0.0);
        assert!(
            five < 1000.0 && five > profile.candela_at(10.0, 0.0),
            "5° gave {five}, which is not between its neighbours"
        );
    }

    /// A rotationally symmetric file has one horizontal column, and every
    /// azimuth has to find it. Clamping the horizontal lookup to a one-entry
    /// table is easy to get wrong in a way that leaves the light lit in one
    /// direction only.
    #[test]
    fn a_symmetric_profile_lights_every_azimuth() {
        let profile = IesProfile::parse(&downlight()).unwrap();
        let reference = profile.candela_at(30.0, 0.0);
        assert!(reference > 0.0);
        for azimuth in [0.0, 45.0, 90.0, 180.0, 270.0, 359.0] {
            let value = profile.candela_at(30.0, azimuth);
            assert!(
                (value - reference).abs() < 1e-3,
                "at {azimuth}° the profile gives {value} rather than {reference}"
            );
        }
    }

    /// The aiming axis is local `-Z`, matching every other light in the scene.
    /// A profile pointing the wrong way is a downlight that lights the ceiling.
    #[test]
    fn the_aiming_axis_is_negative_z() {
        let profile = IesProfile::parse(&downlight()).unwrap();
        assert!((profile.candela_towards(-Vec3::Z) - 1000.0).abs() < 1.0);
        assert!(profile.candela_towards(Vec3::Z) < 1.0);
        assert!(profile.candela_towards(Vec3::X) < 1.0);
    }

    /// Selection needs one number for how bright a luminaire is, and it has to
    /// be the *equivalent isotropic* intensity. Using the peak would have a
    /// narrow spot sampled as if it lit the whole room.
    #[test]
    fn the_mean_is_far_below_the_peak_for_a_narrow_beam() {
        let profile = IesProfile::parse(&downlight()).unwrap();
        assert!(profile.mean() > 0.0);
        assert!(
            profile.mean() < profile.peak() * 0.3,
            "a downlight's mean {} is too close to its peak {}",
            profile.mean(),
            profile.peak()
        );
    }

    /// The numbers wrap across lines wherever the writer chose. A parser that
    /// expected one list per line would reject most real files.
    #[test]
    fn the_data_may_wrap_across_lines_anywhere() {
        let compact = downlight();
        let wrapped = compact.replace(' ', "\n");
        let a = IesProfile::parse(&compact).unwrap();
        let b = IesProfile::parse(&wrapped).unwrap();
        assert_eq!(a, b);
    }

    /// A file that is truncated, or not an IES file at all, is an error rather
    /// than a light that is silently black.
    #[test]
    fn a_malformed_file_is_reported() {
        assert!(IesProfile::parse("hello").is_err());
        assert!(IesProfile::parse("TILT=NONE\n1 -1 1 10 1\n").is_err());
        // A file whose every measurement is zero is a luminaire that emits
        // nothing, which is a broken file rather than a very dim light.
        let zeroed = "TILT=NONE
1 -1 1 2 1 1 2 0 0 0
1 1 100
0 90
0
0 0
";
        assert!(IesProfile::parse(zeroed).is_err());
    }

    /// A tilt block is data, and skipping it by the wrong number of values
    /// shifts everything after it — which reads as a profile with plausible
    /// angles and nonsense intensities.
    #[test]
    fn a_tilt_block_is_skipped_by_exactly_its_length() {
        let plain = IesProfile::parse(&downlight()).unwrap();
        let tilted = downlight().replace(
            "TILT=NONE\n",
            "TILT=INCLUDE\n1 3\n0 45 90\n1.0 0.9 0.8\n",
        );
        let parsed = IesProfile::parse(&tilted).unwrap();
        assert_eq!(parsed, plain);
    }
}
