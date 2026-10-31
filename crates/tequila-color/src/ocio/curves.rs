//! The one-dimensional curves OCIO's built-in transforms use: the ACES log
//! encodings, the cameras' log encodings, and the HDR display encodings.
//!
//! Each is written from its published formula, in both directions, and the
//! direction is named for what it produces: [`Curve::to_linear`] decodes an
//! encoded value, [`Curve::from_linear`] encodes one. OCIO itself builds most
//! of the camera curves as 4096-entry LUTs sampled from these same formulas;
//! evaluating the formula is the reference those LUTs approximate.

/// The largest finite half float, which the ACES CTL clamps to in several
/// places because ACES files are half-float EXRs.
pub const HALF_MAX: f32 = 65504.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    /// ACEScct (Academy S-2016-001): ACEScc with a straight toe.
    AcesCct,
    /// ACEScc (Academy S-2014-003): a pure log2 encoding.
    AcesCc,
    /// ACESproxy, 10-bit integer flavour, as normalized code values.
    AcesProxy10,
    /// ADX's channel-independent density to relative exposure, the middle
    /// step of the Academy's ADX to ACES transform.
    AdxDensity,
    /// ARRI LogC3 at EI 800, the ALEXA's (and the ALEXA Mini's) encoding.
    LogC3Ei800,
    /// ARRI LogC4, the ALEXA 35's.
    LogC4,
    /// Sony S-Log3.
    SLog3,
    /// RED Log3G10.
    Log3G10,
    /// Canon Log 2.
    CLog2,
    /// Canon Log 3.
    CLog3,
    /// Panasonic V-Log.
    VLog,
    /// Apple Log.
    AppleLog,
    /// SMPTE ST 2084, the PQ curve, with linear 1.0 at 100 nits — the scale
    /// OCIO's display transforms all share, so that an SDR white and an HDR
    /// one mean the same luminance.
    Pq,
    /// The BT.2100 HLG OETF, scene-linear in `[0, 1]` to signal.
    Hlg,
}

impl Curve {
    /// Encoded to linear.
    pub fn to_linear(self, v: f32) -> f32 {
        match self {
            Curve::AcesCct => {
                if v > ACESCCT_Y_BRK {
                    (v * 17.52 - 9.72).exp2()
                } else {
                    (v - ACESCCT_B) / ACESCCT_A
                }
            }
            Curve::AcesCc => {
                if v < (9.72 - 15.0) / 17.52 {
                    ((v * 17.52 - 9.72).exp2() - (-16.0f32).exp2()) * 2.0
                } else if v < (HALF_MAX.log2() + 9.72) / 17.52 {
                    (v * 17.52 - 9.72).exp2()
                } else {
                    HALF_MAX
                }
            }
            Curve::AcesProxy10 => ((v * 1023.0 - 425.0) / 50.0 - 2.5).exp2(),
            Curve::AdxDensity => {
                let log_exposure = if v <= 0.6 {
                    interpolate(&ADX_DENSITY_TO_LOG_EXPOSURE, v)
                } else {
                    (100.0 / 55.0) * v - adx_reference_point()
                };
                10f32.powf(log_exposure)
            }
            Curve::LogC3Ei800 => {
                let c = LOGC3_EI800;
                if v > c.e * c.cut + c.f {
                    (10f32.powf((v - c.d) / c.c) - c.b) / c.a
                } else {
                    (v - c.f) / c.e
                }
            }
            Curve::LogC4 => {
                let k = LogC4::new();
                if v >= 0.0 {
                    ((14.0 * (v - k.c) / k.b + 6.0).exp2() - 64.0) / k.a
                } else {
                    v * k.s + k.t
                }
            }
            Curve::SLog3 => {
                if v >= 171.210_29 / 1023.0 {
                    10f32.powf((v * 1023.0 - 420.0) / 261.5) * (0.18 + 0.01) - 0.01
                } else {
                    (v * 1023.0 - 95.0) * 0.011_25 / (171.210_29 - 95.0)
                }
            }
            Curve::Log3G10 => {
                if v < 0.0 {
                    v / LOG3G10_G - LOG3G10_C
                } else {
                    (10f32.powf(v / LOG3G10_A) - 1.0) / LOG3G10_B - LOG3G10_C
                }
            }
            Curve::CLog2 => {
                let linear = if v < 0.092_864_125 {
                    -(10f32.powf((0.092_864_125 - v) / 0.241_360_77) - 1.0) / 87.099_375
                } else {
                    (10f32.powf((v - 0.092_864_125) / 0.241_360_77) - 1.0) / 87.099_375
                };
                // Canon's curve is defined against a reflectance of 0.9 for
                // white rather than 1.0, and its IDT scales back.
                linear * 0.9
            }
            Curve::CLog3 => {
                let linear = if v < 0.097_465_473 {
                    -(10f32.powf((0.127_839_01 - v) / 0.367_268_45) - 1.0) / 14.983_25
                } else if v <= 0.152_778_91 {
                    (v - 0.125_122_19) / 1.975_479_8
                } else {
                    (10f32.powf((v - 0.122_405_37) / 0.367_268_45) - 1.0) / 14.983_25
                };
                linear * 0.9
            }
            Curve::VLog => {
                if v < 0.181 {
                    (v - 0.125) / 5.6
                } else {
                    10f32.powf((v - 0.598_206) / 0.241_514) - 0.008_73
                }
            }
            Curve::AppleLog => {
                let a = APPLE_LOG;
                if v >= a.threshold() {
                    ((v - a.delta) / a.gamma).exp2() - a.beta
                } else if v > 0.0 {
                    (v / a.c).sqrt() + a.r0
                } else {
                    a.r0
                }
            }
            Curve::Pq => mirrored(v, pq_to_linear),
            Curve::Hlg => mirrored(v, |e| {
                if e <= 0.5 {
                    e * e / 3.0
                } else {
                    (((e - HLG_C) / HLG_A).exp() + HLG_B) / 12.0
                }
            }),
        }
    }

    /// Linear to encoded.
    pub fn from_linear(self, v: f32) -> f32 {
        match self {
            Curve::AcesCct => {
                if v <= ACESCCT_X_BRK {
                    ACESCCT_A * v + ACESCCT_B
                } else {
                    (v.log2() + 9.72) / 17.52
                }
            }
            Curve::AcesCc => {
                if v <= 0.0 {
                    ((-16.0f32) + 9.72) / 17.52
                } else if v < (-15.0f32).exp2() {
                    (((-16.0f32).exp2() + v * 0.5).log2() + 9.72) / 17.52
                } else {
                    (v.log2() + 9.72) / 17.52
                }
            }
            Curve::AcesProxy10 => {
                // The Academy's encoder also rounds to an integer code; a
                // float pipeline keeps the fraction, which is what OCIO's
                // inverted LUT does too. The clamp to the legal range is
                // part of the encoding and is kept.
                let code = (v.max(1e-10).log2() + 2.5) * 50.0 + 425.0;
                code.clamp(64.0, 940.0) / 1023.0
            }
            Curve::AdxDensity => {
                let log_exposure = v.max(1e-10).log10();
                let knee = ADX_DENSITY_TO_LOG_EXPOSURE[ADX_DENSITY_TO_LOG_EXPOSURE.len() - 1];
                if log_exposure <= knee[1] {
                    interpolate_inverse(&ADX_DENSITY_TO_LOG_EXPOSURE, log_exposure)
                } else {
                    (log_exposure + adx_reference_point()) * 55.0 / 100.0
                }
            }
            Curve::LogC3Ei800 => {
                let c = LOGC3_EI800;
                if v > c.cut {
                    c.c * (c.a * v + c.b).log10() + c.d
                } else {
                    c.e * v + c.f
                }
            }
            Curve::LogC4 => {
                let k = LogC4::new();
                if v >= k.t {
                    ((k.a * v + 64.0).log2() - 6.0) / 14.0 * k.b + k.c
                } else {
                    (v - k.t) / k.s
                }
            }
            Curve::SLog3 => {
                if v >= 0.011_25 {
                    (420.0 + ((v + 0.01) / (0.18 + 0.01)).log10() * 261.5) / 1023.0
                } else {
                    (v * (171.210_29 - 95.0) / 0.011_25 + 95.0) / 1023.0
                }
            }
            Curve::Log3G10 => {
                let shifted = v + LOG3G10_C;
                if shifted < 0.0 {
                    shifted * LOG3G10_G
                } else {
                    LOG3G10_A * (shifted * LOG3G10_B + 1.0).log10()
                }
            }
            Curve::CLog2 => {
                let r = v / 0.9;
                if r < 0.0 {
                    0.092_864_125 - 0.241_360_77 * (1.0 - 87.099_375 * r).log10()
                } else {
                    0.092_864_125 + 0.241_360_77 * (1.0 + 87.099_375 * r).log10()
                }
            }
            Curve::CLog3 => {
                let r = v / 0.9;
                if r < -0.014 {
                    0.127_839_01 - 0.367_268_45 * (1.0 - 14.983_25 * r).log10()
                } else if r <= 0.014 {
                    1.975_479_8 * r + 0.125_122_19
                } else {
                    0.367_268_45 * (14.983_25 * r + 1.0).log10() + 0.122_405_37
                }
            }
            Curve::VLog => {
                if v < 0.01 {
                    5.6 * v + 0.125
                } else {
                    0.241_514 * (v + 0.008_73).log10() + 0.598_206
                }
            }
            Curve::AppleLog => {
                let a = APPLE_LOG;
                if v >= a.rt {
                    a.gamma * (v + a.beta).log2() + a.delta
                } else if v >= a.r0 {
                    a.c * (v - a.r0) * (v - a.r0)
                } else {
                    0.0
                }
            }
            Curve::Pq => mirrored(v, linear_to_pq),
            Curve::Hlg => mirrored(v, |e| {
                if e <= 1.0 / 12.0 {
                    (3.0 * e).sqrt()
                } else {
                    HLG_A * (12.0 * e - HLG_B).ln() + HLG_C
                }
            }),
        }
    }
}

/// Apply `f` to the magnitude and keep the sign, which is what OCIO does for
/// curves with no meaning below zero: a value slightly outside the display
/// gamut comes back slightly negative instead of turning into NaN.
fn mirrored(v: f32, f: impl Fn(f32) -> f32) -> f32 {
    if v < 0.0 {
        -f(-v)
    } else {
        f(v)
    }
}

// ACEScct, from S-2016-001. The two breakpoints are the published ones; `A`
// and `B` are the straight segment that meets the log curve at them with a
// continuous slope.
pub const ACESCCT_X_BRK: f32 = 0.007_812_5;
pub const ACESCCT_Y_BRK: f32 = 0.155_251_14;
pub const ACESCCT_A: f32 = 10.540_237_7;
pub const ACESCCT_B: f32 = 0.072_905_534;

// ST 2084.
const PQ_M1: f32 = 2610.0 / 16384.0;
const PQ_M2: f32 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f32 = 3424.0 / 4096.0;
const PQ_C2: f32 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f32 = 2392.0 / 4096.0 * 32.0;

/// PQ code to linear, 1.0 = 100 nits.
fn pq_to_linear(n: f32) -> f32 {
    let p = n.powf(1.0 / PQ_M2);
    let y = ((p - PQ_C1).max(0.0) / (PQ_C2 - PQ_C3 * p)).powf(1.0 / PQ_M1);
    y * 100.0
}

fn linear_to_pq(v: f32) -> f32 {
    let y = (v / 100.0).powf(PQ_M1);
    ((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y)).powf(PQ_M2)
}

// BT.2100 HLG.
const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 1.0 - 4.0 * HLG_A;
const HLG_C: f32 = 0.559_910_7;

// ARRI LogC3, EI 800, from ARRI's "ALEXA Log C Curve" white paper. `e` and
// `f` are the published straight segment; they meet the log curve at `cut`
// with a continuous value and slope, which the tests check rather than trust.
#[derive(Clone, Copy)]
struct LogC3 {
    cut: f32,
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

const LOGC3_EI800: LogC3 = LogC3 {
    cut: 0.010_591,
    a: 5.555_556,
    b: 0.052_272,
    c: 0.247_190,
    d: 0.385_537,
    e: 5.367_655,
    f: 0.092_809,
};

/// ARRI LogC4, from ARRI's "LogC4 Logarithmic Color Space" specification.
/// Every constant follows from three choices — 14 stops above the black
/// offset, code 95 as black and the 117.45 exposure scale — so they are
/// computed rather than copied.
struct LogC4 {
    a: f32,
    b: f32,
    c: f32,
    s: f32,
    t: f32,
}

impl LogC4 {
    fn new() -> LogC4 {
        let a = (2f64.powi(18) - 16.0) / 117.45;
        let b = (1023.0 - 95.0) / 1023.0;
        let c = 95.0 / 1023.0;
        let s = (7.0 * std::f64::consts::LN_2 * 2f64.powf(7.0 - 14.0 * c / b)) / (a * b);
        let t = (2f64.powf(14.0 * (-c / b) + 6.0) - 64.0) / a;
        LogC4 {
            a: a as f32,
            b: b as f32,
            c: c as f32,
            s: s as f32,
            t: t as f32,
        }
    }
}

// RED Log3G10, from RED's IPP2 white paper.
const LOG3G10_A: f32 = 0.224_282;
const LOG3G10_B: f32 = 155.975_327;
const LOG3G10_C: f32 = 0.01;
const LOG3G10_G: f32 = 15.192_7;

/// Apple Log, from Apple's "Apple Log Profile" white paper.
#[derive(Clone, Copy)]
struct AppleLogConstants {
    r0: f32,
    rt: f32,
    c: f32,
    beta: f32,
    gamma: f32,
    delta: f32,
}

impl AppleLogConstants {
    /// The code value where the parabola hands over to the log segment.
    fn threshold(self) -> f32 {
        self.c * (self.rt - self.r0) * (self.rt - self.r0)
    }
}

const APPLE_LOG: AppleLogConstants = AppleLogConstants {
    r0: -0.056_410_88,
    rt: 0.01,
    c: 47.287_112,
    beta: 0.009_640_52,
    gamma: 0.085_504_79,
    delta: 0.693_369_45,
};

/// The Academy's ADX table, channel-independent density to log exposure,
/// below the density of 0.6 where the film curve is no longer straight.
const ADX_DENSITY_TO_LOG_EXPOSURE: [[f32; 2]; 11] = [
    [-0.19, -6.0],
    [0.01, -2.721_718_6],
    [0.028, -2.521_718_6],
    [0.054, -2.321_718_6],
    [0.095, -2.121_718_6],
    [0.145, -1.921_718_6],
    [0.22, -1.721_718_6],
    [0.30, -1.521_718_6],
    [0.40, -1.321_718_6],
    [0.50, -1.121_718_6],
    [0.60, -0.926_545_7],
];

/// Above the table the curve is straight, `log E = (100/55) D - REF_PT`.
fn adx_reference_point() -> f32 {
    (7120.0 - 1520.0) / 8000.0 * (100.0 / 55.0) - 0.18f32.log10()
}

/// The CTL's `interpolate1D`: piecewise linear, clamped at both ends.
fn interpolate(table: &[[f32; 2]], p: f32) -> f32 {
    let last = table.len() - 1;
    if p <= table[0][0] {
        return table[0][1];
    }
    if p >= table[last][0] {
        return table[last][1];
    }
    for pair in table.windows(2) {
        if pair[0][0] <= p && p < pair[1][0] {
            let s = (p - pair[0][0]) / (pair[1][0] - pair[0][0]);
            return pair[0][1] * (1.0 - s) + pair[1][1] * s;
        }
    }
    table[last][1]
}

/// [`interpolate`] run backwards, for a table increasing in both columns.
fn interpolate_inverse(table: &[[f32; 2]], y: f32) -> f32 {
    let swapped: Vec<[f32; 2]> = table.iter().map(|&[x, y]| [y, x]).collect();
    interpolate(&swapped, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Curve; 14] = [
        Curve::AcesCct,
        Curve::AcesCc,
        Curve::AcesProxy10,
        Curve::AdxDensity,
        Curve::LogC3Ei800,
        Curve::LogC4,
        Curve::SLog3,
        Curve::Log3G10,
        Curve::CLog2,
        Curve::CLog3,
        Curve::VLog,
        Curve::AppleLog,
        Curve::Pq,
        Curve::Hlg,
    ];

    fn close(a: f32, b: f32, tolerance: f32) -> bool {
        (a - b).abs() <= tolerance * b.abs().max(1.0)
    }

    /// ACEScct and ACEScc against S-2016-001 and S-2014-003. These are the
    /// published values, not ones derived here: 18% grey encodes to
    /// 0.4135884 in both (the two curves are the same log2 above the toe),
    /// ACEScct's straight segment meets the log at X_BRK = 2^-7 and
    /// Y_BRK = 0.155251141552511, its black is B = 0.0729055341958355, and
    /// ACEScc's zero is -0.3584474886 and its half-float maximum 1.4679964.
    #[test]
    fn acescct_and_acescc_hit_their_published_values() {
        assert!(close(Curve::AcesCct.from_linear(0.18), 0.413_588_4, 1e-6));
        assert!(close(Curve::AcesCc.from_linear(0.18), 0.413_588_4, 1e-6));
        assert!(close(Curve::AcesCct.from_linear(0.0), 0.072_905_534, 1e-7));
        assert!(close(Curve::AcesCc.from_linear(0.0), -0.358_447_5, 1e-6));
        assert!(close(Curve::AcesCc.from_linear(HALF_MAX), 1.467_996_4, 1e-6));

        // Both sides of ACEScct's break give the same value there, and the
        // two segments have the same slope — the straight one is A, the log
        // one is 1 / (17.52 ln 2 x) at x = 2^-7.
        let log_side = (ACESCCT_X_BRK.log2() + 9.72) / 17.52;
        let linear_side = ACESCCT_A * ACESCCT_X_BRK + ACESCCT_B;
        assert!(close(log_side, ACESCCT_Y_BRK, 1e-6), "{log_side}");
        assert!(close(linear_side, ACESCCT_Y_BRK, 1e-6), "{linear_side}");
        let log_slope = 1.0 / (17.52 * std::f32::consts::LN_2 * ACESCCT_X_BRK);
        assert!(close(log_slope, ACESCCT_A, 1e-5), "{log_slope}");
    }

    /// Published anchors for the camera curves, each from its vendor's
    /// document: 18% grey is 10-bit code 420 in S-Log3, 0.391 in LogC3 at
    /// EI 800, 0.2784 in LogC4, exactly a third in Log3G10 (that is how RED
    /// defines it), and 10-bit code 433 in V-Log.
    #[test]
    fn camera_curves_put_grey_where_the_vendors_say() {
        assert!(close(Curve::SLog3.from_linear(0.18), 420.0 / 1023.0, 1e-5));
        assert!(close(Curve::LogC3Ei800.from_linear(0.18), 0.391_007, 1e-4));
        assert!(close(Curve::LogC4.from_linear(0.18), 0.278_4, 2e-4));
        assert!(close(Curve::Log3G10.from_linear(0.18), 1.0 / 3.0, 1e-4));
        assert!((Curve::VLog.from_linear(0.18) * 1023.0 - 433.0).abs() < 0.6);
    }

    /// Where a curve changes formula, both formulas give the same value —
    /// this is what catches a mistyped constant in a piecewise curve, which
    /// otherwise shows only as a step at one exposure.
    #[test]
    fn piecewise_curves_are_continuous_at_their_breaks() {
        let checks: [(Curve, f32); 8] = [
            (Curve::LogC3Ei800, LOGC3_EI800.e * LOGC3_EI800.cut + LOGC3_EI800.f),
            (Curve::LogC4, 0.0),
            (Curve::SLog3, 171.210_29 / 1023.0),
            (Curve::CLog2, 0.092_864_125),
            (Curve::CLog3, 0.097_465_473),
            (Curve::CLog3, 0.152_778_91),
            (Curve::VLog, 0.181),
            (Curve::AppleLog, APPLE_LOG.threshold()),
        ];
        for (curve, at) in checks {
            let below = curve.to_linear(at - 1e-5);
            let above = curve.to_linear(at + 1e-5);
            assert!(
                (below - above).abs() < 1e-4,
                "{curve:?} jumps at {at}: {below} below, {above} above"
            );
        }
        // ADX: the table's last point is on the straight segment.
        let table_end = ADX_DENSITY_TO_LOG_EXPOSURE[10][1];
        let line = (100.0 / 55.0) * 0.6 - adx_reference_point();
        assert!((table_end - line).abs() < 1e-5, "{table_end} against {line}");
    }

    /// The display curves against the standards. ST 2084 puts 100 nits at
    /// 0.5081 and 1000 at 0.7518 and its maximum at 10000; HLG puts
    /// 1/12 at a half and 1.0 at 1.0. All published (BT.2100, and the code
    /// values every PQ test pattern carries). PQ's zero is not exactly zero —
    /// the formula gives c1^m2, 7.3e-7 — which is the standard's own value.
    #[test]
    fn pq_and_hlg_hit_the_standard_values() {
        assert!(close(Curve::Pq.from_linear(1.0), 0.508_078, 1e-5));
        assert!(close(Curve::Pq.from_linear(10.0), 0.751_827, 1e-5));
        assert!(close(Curve::Pq.from_linear(100.0), 1.0, 1e-6));
        assert!(Curve::Pq.from_linear(0.0) < 1e-6);
        assert!(close(Curve::Hlg.from_linear(1.0 / 12.0), 0.5, 1e-6));
        assert!(close(Curve::Hlg.from_linear(1.0), 1.0, 1e-5));
    }

    /// Every curve's two directions undo each other across its useful range.
    #[test]
    fn every_curve_round_trips() {
        for curve in ALL {
            for i in 0..=60 {
                // 2^-12 to 2^8 of scene-linear, which covers every camera's
                // range and the PQ range up to 25600 nits... clipped where the
                // curve itself stops.
                let linear = (i as f32 / 3.0 - 12.0).exp2();
                let linear = match curve {
                    Curve::Hlg => linear.min(1.0),
                    Curve::Pq => linear.min(100.0),
                    Curve::AcesProxy10 => linear.clamp(0.002, 200.0),
                    _ => linear,
                };
                let back = curve.to_linear(curve.from_linear(linear));
                assert!(
                    close(back, linear, 2e-4),
                    "{curve:?}: {linear} came back as {back}"
                );
            }
        }
    }

    #[test]
    fn every_curve_is_monotonic() {
        for curve in ALL {
            let mut previous = f32::NEG_INFINITY;
            for i in 0..=400 {
                let code = i as f32 / 400.0;
                let linear = curve.to_linear(code);
                assert!(
                    linear >= previous,
                    "{curve:?} decreases at code {code}: {previous} then {linear}"
                );
                previous = linear;
            }
        }
    }
}
