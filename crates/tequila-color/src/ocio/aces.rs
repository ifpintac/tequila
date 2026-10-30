//! The ACES 1.x output transform maths, written out from the Academy's CTL.
//!
//! Function by function this follows `ACESlib.RRT_Common.ctl`,
//! `ACESlib.Tonescales.ctl`, `ACESlib.SSTS.ctl`, `ACESlib.ODT_Common.ctl` and
//! `LMT.Academy.ReferenceGamutCompress.ctl` of ACES 1.3, with the CTL's own
//! names in the comments so the two can be read side by side. Nothing here is
//! fitted: the tone scales are the CTL's quadratic B-splines evaluated from
//! their published coefficients, and the inverses are the CTL's inverses.
//!
//! OCIO evaluates the same algorithms, though it bakes the combined spline
//! into a half-float-domain 1D LUT; the difference between the two is the
//! LUT's interpolation error, a few parts in ten thousand at most.

use tequila_core::math::{vec3, Vec3};

/// The CTL's `HALF_MIN`, the smallest positive half float, which the splines
/// put in place of zero before taking a logarithm.
const HALF_MIN: f32 = 5.960_464_5e-8;

// The RRT's "sweeteners": a glow that lifts dark saturated colours and a
// modifier that tames saturated reds (ACESlib.RRT_Common.ctl).
const RRT_GLOW_GAIN: f32 = 0.05;
const RRT_GLOW_MID: f32 = 0.08;
const RRT_RED_SCALE: f32 = 0.82;
const RRT_RED_PIVOT: f32 = 0.03;
const RRT_RED_HUE: f32 = 0.0;
const RRT_RED_WIDTH: f32 = 135.0;

/// `rgb_2_saturation`.
fn saturation(c: Vec3) -> f32 {
    let max = c.max_component();
    let min = c.x.min(c.y).min(c.z);
    (max.max(1e-10) - min.max(1e-10)) / max.max(1e-2)
}

/// `rgb_2_yc`: a luminance proxy that a saturated colour reads brighter in,
/// which is what lets the glow tell a dark saturated colour from a dark grey.
fn yc(c: Vec3) -> f32 {
    const YC_RADIUS_WEIGHT: f32 = 1.75;
    // Never negative in exact arithmetic, as it is half the sum of the
    // squared channel differences; rounding can take it just below zero.
    let chroma = (c.z * (c.z - c.y) + c.y * (c.y - c.x) + c.x * (c.x - c.z))
        .max(0.0)
        .sqrt();
    (c.x + c.y + c.z + YC_RADIUS_WEIGHT * chroma) / 3.0
}

/// `sigmoid_shaper`: 0 to 1 across -2 to +2.
fn sigmoid_shaper(x: f32) -> f32 {
    let t = (1.0 - (x / 2.0).abs()).max(0.0);
    let sign = if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    };
    (1.0 + sign * (1.0 - t * t)) / 2.0
}

/// `glow_fwd`.
fn glow_forward(yc_in: f32, gain: f32, mid: f32) -> f32 {
    if yc_in <= 2.0 / 3.0 * mid {
        gain
    } else if yc_in >= 2.0 * mid {
        0.0
    } else {
        gain * (mid / yc_in - 0.5)
    }
}

/// `glow_inv`.
fn glow_inverse(yc_out: f32, gain: f32, mid: f32) -> f32 {
    if yc_out <= (1.0 + gain) * 2.0 / 3.0 * mid {
        -gain / (1.0 + gain)
    } else if yc_out >= 2.0 * mid {
        0.0
    } else {
        gain * (mid / yc_out - 0.5) / (gain / 2.0 - 1.0)
    }
}

/// `rgb_2_hue`, in degrees. `None` for a neutral, whose hue the CTL leaves as
/// NaN so that every comparison against it fails; an `Option` says the same
/// thing without relying on that.
fn hue(c: Vec3) -> Option<f32> {
    if c.x == c.y && c.y == c.z {
        return None;
    }
    let h = (3f32.sqrt() * (c.y - c.z)).atan2(2.0 * c.x - c.y - c.z).to_degrees();
    Some(if h < 0.0 { h + 360.0 } else { h })
}

/// `center_hue`.
fn center_hue(hue: f32, centre: f32) -> f32 {
    let h = hue - centre;
    if h < -180.0 {
        h + 360.0
    } else if h > 180.0 {
        h - 360.0
    } else {
        h
    }
}

/// `cubic_basis_shaper`: a cubic B-spline bump `width` degrees across,
/// peaking at 1 in the middle.
fn cubic_basis_shaper(x: f32, width: f32) -> f32 {
    const M: [[f32; 4]; 4] = [
        [-1.0 / 6.0, 3.0 / 6.0, -3.0 / 6.0, 1.0 / 6.0],
        [3.0 / 6.0, -6.0 / 6.0, 3.0 / 6.0, 0.0],
        [-3.0 / 6.0, 0.0, 3.0 / 6.0, 0.0],
        [1.0 / 6.0, 4.0 / 6.0, 1.0 / 6.0, 0.0],
    ];
    let knots = [-width / 2.0, width / 2.0];
    if !(x > knots[0] && x < knots[1]) {
        return 0.0;
    }
    let knot_coord = (x - knots[0]) * 4.0 / width;
    let j = (knot_coord as usize).min(3);
    let t = knot_coord - j as f32;
    let monomials = [t * t * t, t * t, t, 1.0];
    // Segment j uses column 3 - j of the basis matrix.
    let column = 3 - j;
    let y: f32 = (0..4).map(|row| monomials[row] * M[row][column]).sum();
    y * 1.5
}

/// The red modifier's weight for a colour: how far into the red hues it is.
fn red_weight(c: Vec3) -> (f32, f32) {
    match hue(c) {
        Some(h) => {
            let centred = center_hue(h, RRT_RED_HUE);
            (cubic_basis_shaper(centred, RRT_RED_WIDTH), centred)
        }
        None => (0.0, 0.0),
    }
}

/// The RRT's glow and red modifier, forward (`RRT.ctl`, and
/// `rrt_sweeteners` in 1.1 and later).
///
/// One function rather than two, because the CTL computes the saturation
/// once, before the glow, and uses it for both; split into two passes the red
/// modifier would see the glowed colour's saturation instead, which differs
/// in the darkest values where the saturation's denominator is clamped.
pub fn sweeteners_forward(c: Vec3) -> Vec3 {
    let sat = saturation(c);
    let s = sigmoid_shaper((sat - 0.4) / 0.2);
    let added_glow = 1.0 + glow_forward(yc(c), RRT_GLOW_GAIN * s, RRT_GLOW_MID);
    let mut aces = c * added_glow;

    let (weight, _) = red_weight(aces);
    aces.x += weight * sat * (RRT_RED_PIVOT - aces.x) * (1.0 - RRT_RED_SCALE);
    aces
}

/// The inverse, as `InvRRT.ctl` writes it: the red modifier undone first by
/// solving its quadratic, then the glow.
pub fn sweeteners_inverse(c: Vec3) -> Vec3 {
    let mut aces = c;
    let (weight, centred) = red_weight(aces);
    if weight > 0.0 {
        // In the red hues red is the largest channel, so the forward
        // modifier's saturation is (r - min) / r; which of green and blue is
        // the minimum depends on which side of red the hue is.
        let min_channel = if centred < 0.0 { aces.y } else { aces.z };
        let a = weight * (1.0 - RRT_RED_SCALE) - 1.0;
        let b = aces.x - weight * (RRT_RED_PIVOT + min_channel) * (1.0 - RRT_RED_SCALE);
        let c = weight * RRT_RED_PIVOT * min_channel * (1.0 - RRT_RED_SCALE);
        aces.x = (-b - (b * b - 4.0 * a * c).max(0.0).sqrt()) / (2.0 * a);
    }

    let sat = saturation(aces);
    let s = sigmoid_shaper((sat - 0.4) / 0.2);
    let reduced_glow = 1.0 + glow_inverse(yc(aces), RRT_GLOW_GAIN * s, RRT_GLOW_MID);
    aces * reduced_glow
}

/// A segmented quadratic B-spline in log-log space: the shape of every ACES
/// 1.x tone scale. `SegmentedSplineParams_c5` and `_c9` in the CTL, and the
/// SSTS's `TsParams`, are all this with different knot counts.
#[derive(Clone, Debug, PartialEq)]
pub struct Spline {
    coefs_low: [f32; 10],
    coefs_high: [f32; 10],
    knots_low: usize,
    knots_high: usize,
    /// log10 of (x, y) at the three control points.
    min: [f32; 2],
    mid: [f32; 2],
    max: [f32; 2],
    slope_low: f32,
    slope_high: f32,
}

impl Spline {
    #[allow(clippy::too_many_arguments)]
    fn new(
        low: &[f32],
        high: &[f32],
        min: [f32; 2],
        mid: [f32; 2],
        max: [f32; 2],
        slope_low: f32,
        slope_high: f32,
    ) -> Spline {
        let mut coefs_low = [0.0; 10];
        let mut coefs_high = [0.0; 10];
        coefs_low[..low.len()].copy_from_slice(low);
        coefs_high[..high.len()].copy_from_slice(high);
        Spline {
            coefs_low,
            coefs_high,
            // A spline of N knots carries N + 2 coefficients.
            knots_low: low.len() - 2,
            knots_high: high.len() - 2,
            min: [min[0].log10(), min[1].log10()],
            mid: [mid[0].log10(), mid[1].log10()],
            max: [max[0].log10(), max[1].log10()],
            slope_low,
            slope_high,
        }
    }

    /// `segmented_spline_c5_fwd`, `_c9_fwd` and `ssts`.
    pub fn forward(&self, x: f32) -> f32 {
        // Every spline here has a flat low extension, so where zero is put
        // before the logarithm does not matter as long as it is below the
        // minimum point: C9's 1e-4 and the others' HALF_MIN give the same y.
        let log_x = x.max(HALF_MIN).log10();
        let log_y = if log_x <= self.min[0] {
            log_x * self.slope_low + (self.min[1] - self.slope_low * self.min[0])
        } else if log_x < self.mid[0] {
            let knot = (self.knots_low - 1) as f32 * (log_x - self.min[0]) / (self.mid[0] - self.min[0]);
            segment(&self.coefs_low, self.knots_low, knot)
        } else if log_x < self.max[0] {
            let knot = (self.knots_high - 1) as f32 * (log_x - self.mid[0]) / (self.max[0] - self.mid[0]);
            segment(&self.coefs_high, self.knots_high, knot)
        } else {
            log_x * self.slope_high + (self.max[1] - self.slope_high * self.max[0])
        };
        10f32.powf(log_y)
    }

    /// `segmented_spline_c5_rev`, `_c9_rev` and `inv_ssts`. Like the CTL's,
    /// it clamps to the end points rather than inverting the extensions.
    pub fn inverse(&self, y: f32) -> f32 {
        let log_y = y.max(1e-10).log10();
        let log_x = if log_y <= self.min[1] {
            self.min[0]
        } else if log_y <= self.mid[1] {
            let step = (self.mid[0] - self.min[0]) / (self.knots_low - 1) as f32;
            self.min[0] + inverse_segment(&self.coefs_low, self.knots_low, log_y) * step
        } else if log_y < self.max[1] {
            let step = (self.max[0] - self.mid[0]) / (self.knots_high - 1) as f32;
            self.mid[0] + inverse_segment(&self.coefs_high, self.knots_high, log_y) * step
        } else {
            self.max[0]
        };
        10f32.powf(log_x)
    }
}

/// One quadratic B-spline segment at knot coordinate `knot`. This is the
/// CTL's `monomials . (cf * M)` with M the textbook basis
/// `{{0.5, -1, 0.5}, {-1, 1, 0.5}, {0.5, 0, 0}}`, multiplied out.
fn segment(coefs: &[f32; 10], knots: usize, knot: f32) -> f32 {
    let j = (knot.max(0.0) as usize).min(knots - 2);
    let t = knot - j as f32;
    let (c0, c1, c2) = (coefs[j], coefs[j + 1], coefs[j + 2]);
    0.5 * (c0 - 2.0 * c1 + c2) * t * t + (c1 - c0) * t + 0.5 * (c0 + c1)
}

/// The knot coordinate at which a segment reaches `log_y`, by the CTL's
/// quadratic root `2c / (-sqrt(b^2 - 4ac) - b)` — the form that stays exact
/// when a segment is straight and `a` is zero.
fn inverse_segment(coefs: &[f32; 10], knots: usize, log_y: f32) -> f32 {
    // The value at knot i is the mean of coefficients i and i + 1.
    let mut j = 0;
    for i in 0..knots - 1 {
        let lo = (coefs[i] + coefs[i + 1]) / 2.0;
        let hi = (coefs[i + 1] + coefs[i + 2]) / 2.0;
        if log_y > lo && log_y <= hi {
            j = i;
            break;
        }
    }
    let (c0, c1, c2) = (coefs[j], coefs[j + 1], coefs[j + 2]);
    let a = 0.5 * (c0 - 2.0 * c1 + c2);
    let b = c1 - c0;
    let c = 0.5 * (c0 + c1) - log_y;
    let d = (b * b - 4.0 * a * c).max(0.0).sqrt();
    let t = (2.0 * c) / (-d - b);
    t + j as f32
}

/// The RRT's tone scale, `RRT_PARAMS` in `ACESlib.Tonescales.ctl`: scene
/// exposure to output luminance in nits, 18% grey to 4.8.
pub fn rrt_spline() -> Spline {
    Spline::new(
        &[-4.0, -4.0, -3.157_376_6, -0.485_249_99, 1.847_732_5, 1.847_732_5],
        &[-0.718_548_24, 2.081_030_7, 3.668_124, 4.0, 4.0, 4.0],
        [0.18 * (-15f32).exp2(), 0.0001],
        [0.18, 4.8],
        [0.18 * 18f32.exp2(), 10000.0],
        0.0,
        0.0,
    )
}

/// The 48-nit ODT tone scale, `ODT_48nits`: the RRT's output luminance to the
/// SDR display's, 4.8 nits held at 4.8 and white at 48. Its control points'
/// x are the RRT's outputs for 18% grey and 6.5 stops either side.
pub fn odt_48_nit_spline(rrt: &Spline) -> Spline {
    Spline::new(
        &[
            -1.698_970_0,
            -1.698_970_0,
            -1.4779,
            -1.2291,
            -0.8648,
            -0.448,
            0.005_18,
            0.451_108_03,
            0.911_374_44,
            0.911_374_44,
        ],
        &[
            0.515_438_7,
            0.847_043_8,
            1.1358,
            1.3802,
            1.5197,
            1.5985,
            1.6467,
            1.674_609_1,
            1.687_873_3,
            1.687_873_3,
        ],
        [rrt.forward(0.18 * (-6.5f32).exp2()), 0.02],
        [rrt.forward(0.18), 4.8],
        [rrt.forward(0.18 * 6.5f32.exp2()), 48.0],
        0.0,
        0.04,
    )
}

/// The Single Stage Tone Scale of ACES 1.1 (`ACESlib.SSTS.ctl`): one spline
/// from scene exposure straight to a display's luminance, built from the
/// display's black, mid-grey and peak.
///
/// `init_TsParams`, `init_coefsLow` and `init_coefsHigh` in one: the spline
/// is first built for the black and peak alone, which puts 18% grey at 4.8
/// nits, then shifted along the exposure axis until 18% grey lands on
/// `y_mid` instead — the CTL's `expShift`.
pub fn ssts(y_min: f32, y_mid: f32, y_max: f32) -> Spline {
    let unshifted = ssts_params(y_min, y_max, 0.0);
    let exposure_shift = unshifted.inverse(y_mid).log2() - 0.18f32.log2();
    ssts_params(y_min, y_max, exposure_shift)
}

fn ssts_params(y_min: f32, y_max: f32, exposure_shift: f32) -> Spline {
    const MIN_STOP_SDR: f32 = -6.5;
    const MAX_STOP_SDR: f32 = 6.5;
    const MIN_STOP_RRT: f32 = -15.0;
    const MAX_STOP_RRT: f32 = 18.0;
    const MIN_LUM_SDR: f32 = 0.02;
    const MAX_LUM_SDR: f32 = 48.0;
    const MIN_LUM_RRT: f32 = 0.0001;
    const MAX_LUM_RRT: f32 = 10000.0;
    // The mid point's slope in log-log space.
    const MID_SLOPE: f32 = 1.55;

    let aces_min = 0.18
        * interpolate(
            &[[MIN_LUM_RRT.log10(), MIN_STOP_RRT], [MIN_LUM_SDR.log10(), MIN_STOP_SDR]],
            y_min.log10(),
        )
        .exp2();
    let aces_max = 0.18
        * interpolate(
            &[[MAX_LUM_SDR.log10(), MAX_STOP_SDR], [MAX_LUM_RRT.log10(), MAX_STOP_RRT]],
            y_max.log10(),
        )
        .exp2();

    let (lo_x, lo_y) = (aces_min.log10(), y_min.log10());
    let (mid_x, mid_y) = (0.18f32.log10(), 4.8f32.log10());
    let (hi_x, hi_y) = (aces_max.log10(), y_max.log10());

    // init_coefsLow. Both ends have zero slope, so the two coefficients
    // straddling each is just its log luminance; the mid point's pair
    // straddles it along the 1.55 slope.
    let step_low = (mid_x - lo_x) / 3.0;
    let mut low = [0.0f32; 6];
    low[0] = lo_y;
    low[1] = lo_y;
    low[3] = MID_SLOPE * (mid_x - 0.5 * step_low) + (mid_y - MID_SLOPE * mid_x);
    low[4] = MID_SLOPE * (mid_x + 0.5 * step_low) + (mid_y - MID_SLOPE * mid_x);
    // The middle coefficient sets how sharp the bend is, interpolated
    // between the RRT's and the SDR ODT's.
    let pct_low = interpolate(
        &[[MIN_STOP_RRT, 0.18], [MIN_STOP_SDR, 0.35]],
        (aces_min / 0.18).log2(),
    );
    low[2] = lo_y + pct_low * (mid_y - lo_y);
    low[5] = low[4];

    // init_coefsHigh.
    let step_high = (hi_x - mid_x) / 3.0;
    let mut high = [0.0f32; 6];
    high[0] = MID_SLOPE * (mid_x - 0.5 * step_high) + (mid_y - MID_SLOPE * mid_x);
    high[1] = MID_SLOPE * (mid_x + 0.5 * step_high) + (mid_y - MID_SLOPE * mid_x);
    high[3] = hi_y;
    high[4] = hi_y;
    let pct_high = interpolate(
        &[[MAX_STOP_SDR, 0.89], [MAX_STOP_RRT, 0.90]],
        (aces_max / 0.18).log2(),
    );
    high[2] = mid_y + pct_high * (hi_y - mid_y);
    high[5] = high[4];

    // `shift`: the control points move along the exposure axis; the
    // coefficients, which are in luminance, do not.
    let shift = |x: f32| (x.log2() - exposure_shift).exp2();
    Spline::new(
        &low,
        &high,
        [shift(aces_min), y_min],
        [shift(0.18), 4.8],
        [shift(aces_max), y_max],
        0.0,
        0.0,
    )
}

/// The CTL's `interpolate1D`, for a two-point table.
fn interpolate(table: &[[f32; 2]; 2], p: f32) -> f32 {
    if p <= table[0][0] {
        return table[0][1];
    }
    if p >= table[1][0] {
        return table[1][1];
    }
    let s = (p - table[0][0]) / (table[1][0] - table[0][0]);
    table[0][1] * (1.0 - s) + table[1][1] * s
}

/// The tone scale an output transform applies per channel in AP1, from the
/// RRT's input to the display's linear code value.
#[derive(Clone, Debug, PartialEq)]
pub enum Tonescale {
    /// ACES 1.0 SDR: the RRT spline, then the 48-nit ODT spline, then
    /// `Y_2_linCV` against the 48-nit cinema white and 0.02-nit black. Video
    /// outputs use the same 48-nit curve and are rescaled to 100 nits by the
    /// display; that is how ACES 1.0 defines them.
    Sdr { rrt: Spline, odt: Spline, black: f32 },
    /// ACES 1.1 HDR: the SSTS, then `Y_2_linCV` against the display's own
    /// black and peak.
    Hdr { spline: Spline, y_min: f32, y_max: f32 },
}

pub const CINEMA_WHITE: f32 = 48.0;

impl Tonescale {
    pub fn sdr() -> Tonescale {
        let rrt = rrt_spline();
        let odt = odt_48_nit_spline(&rrt);
        Tonescale::Sdr {
            rrt,
            odt,
            // `CINEMA_BLACK` is written `pow10(log10(0.02))` in the CTL so
            // that it equals what the spline returns for black to the last
            // bit — with a plain 0.02 black could never reach code 0.
            black: 10f32.powf(0.02f32.log10()),
        }
    }

    pub fn hdr(y_min: f32, y_mid: f32, y_max: f32) -> Tonescale {
        Tonescale::Hdr {
            spline: ssts(y_min, y_mid, y_max),
            y_min,
            y_max,
        }
    }

    pub fn forward(&self, x: f32) -> f32 {
        match self {
            Tonescale::Sdr { rrt, odt, black } => {
                (odt.forward(rrt.forward(x)) - black) / (CINEMA_WHITE - black)
            }
            Tonescale::Hdr { spline, y_min, y_max } => {
                (spline.forward(x) - y_min) / (y_max - y_min)
            }
        }
    }

    pub fn inverse(&self, v: f32) -> f32 {
        match self {
            Tonescale::Sdr { rrt, odt, black } => {
                rrt.inverse(odt.inverse(v * (CINEMA_WHITE - black) + black))
            }
            Tonescale::Hdr { spline, y_min, y_max } => {
                spline.inverse(v * (y_max - y_min) + y_min)
            }
        }
    }
}

/// `roll_white_fwd`: a gentle shoulder from `1 - width` up to white, which
/// lands on `new_white` instead of 1. The DCI-white "simulation" outputs use
/// it so a D60 or D65 white fits inside a projector calibrated to a greener
/// one without clipping red first.
pub fn roll_white_forward(v: f32, new_white: f32, width: f32) -> f32 {
    let r = RollWhite::new(new_white, width);
    let t = (-v - r.x0) / (r.x1 - r.x0);
    if t < 0.0 {
        -(t * r.b + r.c)
    } else if t > 1.0 {
        v
    } else {
        -((t * r.a + r.b) * t + r.c)
    }
}

/// `roll_white_rev`.
pub fn roll_white_inverse(v: f32, new_white: f32, width: f32) -> f32 {
    let r = RollWhite::new(new_white, width);
    if -v < r.y0 {
        -r.x0
    } else if -v > r.y1 {
        v
    } else {
        let c = r.c + v;
        let discriminant = (r.b * r.b - 4.0 * r.a * c).max(0.0).sqrt();
        let t = (2.0 * c) / (-discriminant - r.b);
        -(t * (r.x1 - r.x0) + r.x0)
    }
}

/// The CTL's roll-off, worked on negated values: a quadratic from
/// `(x0, y0)` = (-1, -new_white) to `(x1, y1)` on the identity, meeting it
/// with slope 1.
struct RollWhite {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    a: f32,
    b: f32,
    c: f32,
}

impl RollWhite {
    fn new(new_white: f32, width: f32) -> RollWhite {
        let x0 = -1.0;
        let x1 = x0 + width;
        let y0 = -new_white;
        let y1 = x1;
        let m1 = x1 - x0;
        RollWhite {
            x0,
            x1,
            y0,
            y1,
            a: y0 - y1 + m1,
            b: 2.0 * (y1 - y0) - m1,
            c: y0,
        }
    }
}

/// The ACES 1.3 Reference Gamut Compression
/// (`LMT.Academy.ReferenceGamutCompress.ctl`), on linear AP1.
///
/// Each channel's distance from the achromatic axis — `(max - c) / max` — is
/// left alone below a threshold and smoothly compressed above it, so that the
/// distance a common camera's gamut edge reaches lands on AP1's own edge.
/// The limits and thresholds are the ones the Academy's working group chose
/// from those cameras: cyan, magenta and yellow are the directions a red, a
/// green and a blue channel goes negative in.
pub fn gamut_compress(c: Vec3, inverse: bool) -> Vec3 {
    const LIMIT: [f32; 3] = [1.147, 1.264, 1.312];
    const THRESHOLD: [f32; 3] = [0.815, 0.803, 0.880];
    const POWER: f32 = 1.2;

    let achromatic = c.max_component();
    if achromatic == 0.0 {
        return c;
    }
    let scale = achromatic.abs();
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        let distance = (achromatic - c[i]) / scale;
        let compressed = compress_distance(distance, LIMIT[i], THRESHOLD[i], POWER, inverse);
        out[i] = achromatic - compressed * scale;
    }
    vec3(out[0], out[1], out[2])
}

/// The CTL's `compress`: a power curve that is the identity up to
/// `threshold` and reaches 1 (the gamut boundary) at `limit`.
fn compress_distance(distance: f32, limit: f32, threshold: f32, power: f32, inverse: bool) -> f32 {
    if distance < threshold {
        return distance;
    }
    // The scale that makes the curve pass through (limit, 1).
    let scale = (limit - threshold)
        / (((1.0 - threshold) / (limit - threshold)).powf(-power) - 1.0).powf(1.0 / power);
    let normalized = (distance - threshold) / scale;
    let p = normalized.powf(power);
    if !inverse {
        threshold + scale * normalized / (1.0 + p).powf(1.0 / power)
    } else if distance > threshold + scale {
        // Beyond the curve's asymptote there is nothing to uncompress to;
        // the CTL passes the value through rather than produce a NaN.
        distance
    } else {
        threshold + scale * (-(p / (p - 1.0))).powf(1.0 / power)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance * b.abs().max(1e-3)
    }

    /// The RRT spline's control points, which are its definition: 18% grey to
    /// 4.8 nits, 2^-15 of it to 0.0001 and 2^18 of it to 10000. These come
    /// straight from `RRT_PARAMS` — the test checks the coefficients were
    /// typed in to reproduce them.
    #[test]
    fn the_rrt_spline_passes_through_its_control_points() {
        let rrt = rrt_spline();
        assert!(close(rrt.forward(0.18), 4.8, 1e-5), "{}", rrt.forward(0.18));
        assert!(close(rrt.forward(0.18 * (-15f32).exp2()), 0.0001, 1e-4));
        assert!(close(rrt.forward(0.18 * 18f32.exp2()), 10000.0, 1e-4));
        assert!(close(rrt.forward(0.0), 0.0001, 1e-4));
    }

    #[test]
    fn the_48_nit_spline_passes_through_its_control_points() {
        let rrt = rrt_spline();
        let odt = odt_48_nit_spline(&rrt);
        assert!(close(odt.forward(4.8), 4.8, 1e-5), "{}", odt.forward(4.8));
        assert!(close(odt.forward(rrt.forward(0.18 * 6.5f32.exp2())), 48.0, 1e-4));
        assert!(close(odt.forward(0.0001), 0.02, 1e-4));
    }

    /// A check on the SSTS that does not depend on any number derived here:
    /// built for the RRT's own range, 0.0001 to 10000 nits, the SSTS's
    /// construction has to reproduce `RRT_PARAMS` — the coefficients the
    /// Academy published for ACES 1.0 — because that is what ACES 1.1 says
    /// the SSTS generalizes. It does, to the precision they were printed.
    #[test]
    fn the_ssts_at_the_rrt_range_is_the_rrt_spline() {
        let ssts = ssts_params(0.0001, 10000.0, 0.0);
        let rrt = rrt_spline();
        for i in 0..6 {
            assert!(
                (ssts.coefs_low[i] - rrt.coefs_low[i]).abs() < 2e-6,
                "low[{i}]: {} against {}",
                ssts.coefs_low[i],
                rrt.coefs_low[i]
            );
            assert!(
                (ssts.coefs_high[i] - rrt.coefs_high[i]).abs() < 2e-6,
                "high[{i}]: {} against {}",
                ssts.coefs_high[i],
                rrt.coefs_high[i]
            );
        }
    }

    /// The HDR outputs put 18% grey at their `Y_MID` by construction, which
    /// is what the exposure shift is for: 15 nits for the 1000/2000/4000-nit
    /// video outputs and 7.2 for the 108-nit cinema one.
    #[test]
    fn the_ssts_puts_grey_at_the_requested_luminance() {
        for (y_max, y_mid) in [(1000.0, 15.0), (2000.0, 15.0), (4000.0, 15.0), (108.0, 7.2)] {
            let s = ssts(0.0001, y_mid, y_max);
            let grey = s.forward(0.18);
            assert!(close(grey, y_mid, 1e-4), "{y_max} nits: grey at {grey}");
            assert!(close(s.forward(1e6), y_max, 1e-4));
            assert!(close(s.forward(0.0), 0.0001, 1e-4));
        }
    }

    #[test]
    fn every_spline_inverts() {
        let rrt = rrt_spline();
        let splines = [
            rrt.clone(),
            odt_48_nit_spline(&rrt),
            ssts(0.0001, 15.0, 1000.0),
            ssts(0.0001, 7.2, 108.0),
        ];
        for spline in &splines {
            for i in 0..=80 {
                // Inside the control points, where the spline is invertible.
                let t = i as f32 / 80.0;
                let x = 10f32.powf(spline.min[0] + (spline.max[0] - spline.min[0]) * (0.001 + 0.998 * t));
                let back = spline.inverse(spline.forward(x));
                assert!(close(back, x, 2e-4), "{x} came back as {back}");
            }
        }
        let sdr = Tonescale::sdr();
        for x in [0.005f32, 0.05, 0.18, 1.0, 5.0] {
            let back = sdr.inverse(sdr.forward(x));
            assert!(close(back, x, 5e-4), "SDR: {x} came back as {back}");
        }
    }

    #[test]
    fn the_sweeteners_leave_greys_alone_and_invert() {
        for v in [0.001f32, 0.05, 0.18, 2.0] {
            let grey = Vec3::splat(v);
            let out = sweeteners_forward(grey);
            assert!((out - grey).abs().max_component() < 1e-7, "{grey:?} became {out:?}");
        }
        // Saturated colours, dark ones included, which is where the glow
        // acts, and reds, which is where the red modifier does.
        //
        // The inverse is the CTL's, and the CTL's undoes the red modifier
        // with the hue of the colour it is given — the *modified* colour's,
        // which is not quite the hue the forward pass weighted by. So it is
        // close rather than exact: half a percent on a strongly saturated
        // red, and exact wherever the modifier does not act.
        for c in [
            vec3(0.02, 0.005, 0.004),
            vec3(0.3, 0.05, 0.04),
            vec3(0.9, 0.2, 0.3),
            vec3(0.05, 0.08, 0.3),
            vec3(0.1, 0.4, 0.05),
        ] {
            let out = sweeteners_forward(c);
            let back = sweeteners_inverse(out);
            for i in 0..3 {
                assert!(
                    (back[i] - c[i]).abs() < 6e-3 * c.max_component(),
                    "{c:?} -> {out:?} -> {back:?}"
                );
            }
        }
        for c in [vec3(0.05, 0.08, 0.3), vec3(0.1, 0.4, 0.05)] {
            let back = sweeteners_inverse(sweeteners_forward(c));
            assert!((back - c).abs().max_component() < 1e-6, "{c:?} came back as {back:?}");
        }
        // And they do something: a dark saturated red glows and is pulled
        // towards the pivot.
        let red = vec3(0.3, 0.05, 0.04);
        assert!(sweeteners_forward(red).x < red.x);
    }

    /// The Reference Gamut Compression's defining properties: colours inside
    /// the threshold are untouched, a distance at the limit lands on the gamut
    /// boundary (distance 1, a zero channel), and the inverse undoes it.
    #[test]
    fn the_gamut_compression_reaches_the_boundary_at_its_limit() {
        // Untouched up to the rounding of the CTL's own recomputation,
        // `ach - dist * ach`.
        let inside = vec3(0.5, 0.3, 0.2);
        assert!((gamut_compress(inside, false) - inside).abs().max_component() < 1e-7);

        // Red channel at distance 1.147 (the cyan limit) from max 1.
        let at_limit = vec3(1.0 - 1.147, 1.0, 0.5);
        let out = gamut_compress(at_limit, false);
        assert!(out.x.abs() < 1e-5, "{out:?}");

        for c in [vec3(-0.1, 0.8, 0.4), vec3(0.6, -0.05, 0.9), vec3(0.2, 0.4, -0.1)] {
            let back = gamut_compress(gamut_compress(c, false), true);
            assert!((back - c).abs().max_component() < 1e-5, "{c:?} came back as {back:?}");
        }
    }

    #[test]
    fn roll_white_lands_white_on_the_new_white_and_inverts() {
        assert!(close(roll_white_forward(1.0, 0.918, 0.5), 0.918, 1e-6));
        assert_eq!(roll_white_forward(0.3, 0.918, 0.5), 0.3);
        // Continuous where it joins the identity.
        assert!(close(roll_white_forward(0.5, 0.918, 0.5), 0.5, 1e-6));
        for v in [0.2f32, 0.55, 0.7, 0.9, 1.0] {
            let back = roll_white_inverse(roll_white_forward(v, 0.918, 0.5), 0.918, 0.5);
            assert!(close(back, v, 1e-5), "{v} came back as {back}");
        }
    }
}
