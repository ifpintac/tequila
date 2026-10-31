//! Chromaticities, and the matrices built from them.
//!
//! Every matrix the built-in transforms use is derived here from the primaries
//! and white point that define a space, the way both the ACES CTL and
//! OpenColorIO derive them, rather than typed in as twelve-digit constants. A
//! constant can be wrong in its ninth digit and nothing would notice; a matrix
//! derived from four chromaticities is checked against the published ones in
//! the tests below, which is how the derivation itself is known to be right.
//!
//! Worked in `f64` and handed to the ops as `f32` at the end, so that the
//! products of three or four matrices a transform chains together do not
//! accumulate single-precision rounding before the pixel maths even starts.

/// A 3x3 matrix, row-major, applied to column vectors: `out = M * in`.
pub type Matrix = [f64; 9];

pub const IDENTITY: Matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// The CIE xy chromaticities of a space's three primaries and its white.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    pub red: [f64; 2],
    pub green: [f64; 2],
    pub blue: [f64; 2],
    pub white: [f64; 2],
}

pub const D65: [f64; 2] = [0.3127, 0.3290];
/// The ACES white, "approximately D60" in SMPTE ST 2065-1's words — not the
/// CIE D60 daylight locus point, which is why it is written out.
pub const ACES_WHITE: [f64; 2] = [0.32168, 0.33767];
pub const DCI_WHITE: [f64; 2] = [0.314, 0.351];

/// ACES2065-1's primaries (SMPTE ST 2065-1).
pub const AP0: Primaries = Primaries {
    red: [0.7347, 0.2653],
    green: [0.0, 1.0],
    blue: [0.0001, -0.0770],
    white: ACES_WHITE,
};

/// ACEScg's, ACEScc's and ACEScct's primaries (Academy S-2014-004).
pub const AP1: Primaries = Primaries {
    red: [0.713, 0.293],
    green: [0.165, 0.830],
    blue: [0.128, 0.044],
    white: ACES_WHITE,
};

pub const REC709: Primaries = Primaries {
    red: [0.64, 0.33],
    green: [0.30, 0.60],
    blue: [0.15, 0.06],
    white: D65,
};

pub const REC2020: Primaries = Primaries {
    red: [0.708, 0.292],
    green: [0.170, 0.797],
    blue: [0.131, 0.046],
    white: D65,
};

const P3_RED: [f64; 2] = [0.680, 0.320];
const P3_GREEN: [f64; 2] = [0.265, 0.690];
const P3_BLUE: [f64; 2] = [0.150, 0.060];

pub const P3_D65: Primaries = Primaries {
    red: P3_RED,
    green: P3_GREEN,
    blue: P3_BLUE,
    white: D65,
};

pub const P3_DCI: Primaries = Primaries {
    red: P3_RED,
    green: P3_GREEN,
    blue: P3_BLUE,
    white: DCI_WHITE,
};

pub const P3_D60: Primaries = Primaries {
    red: P3_RED,
    green: P3_GREEN,
    blue: P3_BLUE,
    white: ACES_WHITE,
};

/// ARRI Wide Gamut 3, the ALEXA's LogC3 gamut.
pub const ARRI_WIDE_GAMUT_3: Primaries = Primaries {
    red: [0.6840, 0.3130],
    green: [0.2210, 0.8480],
    blue: [0.0861, -0.1020],
    white: D65,
};

/// ARRI Wide Gamut 4, the ALEXA 35's LogC4 gamut.
pub const ARRI_WIDE_GAMUT_4: Primaries = Primaries {
    red: [0.7347, 0.2653],
    green: [0.1424, 0.8576],
    blue: [0.0991, -0.0308],
    white: D65,
};

pub const S_GAMUT3: Primaries = Primaries {
    red: [0.730, 0.280],
    green: [0.140, 0.855],
    blue: [0.100, -0.050],
    white: D65,
};

pub const S_GAMUT3_CINE: Primaries = Primaries {
    red: [0.766, 0.275],
    green: [0.225, 0.800],
    blue: [0.089, -0.087],
    white: D65,
};

pub const RED_WIDE_GAMUT: Primaries = Primaries {
    red: [0.780308, 0.304253],
    green: [0.121595, 1.493994],
    blue: [0.095612, -0.084589],
    white: D65,
};

pub const CANON_CINEMA_GAMUT: Primaries = Primaries {
    red: [0.740, 0.270],
    green: [0.170, 1.140],
    blue: [0.080, -0.100],
    white: D65,
};

pub const V_GAMUT: Primaries = Primaries {
    red: [0.730, 0.280],
    green: [0.165, 0.840],
    blue: [0.100, -0.030],
    white: D65,
};

/// How a conversion between two whites is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adaptation {
    /// None at all: the source white keeps its chromaticity. This is what a
    /// "D60 simulation" output wants, and what a clamp in a display's own
    /// primaries needs before the white has been moved.
    None,
    /// Bradford, which the ACES CTL and OCIO's ACES transforms use.
    Bradford,
    /// CAT02, which ARRI, Sony, Canon and Panasonic use for their own
    /// camera-to-ACES matrices.
    Cat02,
}

const BRADFORD: Matrix = [
    0.8951, 0.2664, -0.1614, //
    -0.7502, 1.7135, 0.0367, //
    0.0389, -0.0685, 1.0296,
];

const CAT02: Matrix = [
    0.7328, 0.4296, -0.1624, //
    -0.7036, 1.6975, 0.0061, //
    0.0030, 0.0136, 0.9834,
];

/// XYZ of a chromaticity at unit luminance.
pub fn xyz_of(xy: [f64; 2]) -> [f64; 3] {
    [xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1]]
}

/// RGB to XYZ for a set of primaries, with the white at unit luminance.
///
/// The primaries' XYZ fix the matrix up to one scale per column, and the white
/// fixes those scales: (1, 1, 1) has to land on the white. That is the whole
/// derivation, and the same one SMPTE RP 177 gives.
pub fn rgb_to_xyz(p: &Primaries) -> Matrix {
    let r = xyz_of(p.red);
    let g = xyz_of(p.green);
    let b = xyz_of(p.blue);
    let m = [r[0], g[0], b[0], r[1], g[1], b[1], r[2], g[2], b[2]];
    let s = apply(&invert(&m), xyz_of(p.white));
    [
        m[0] * s[0],
        m[1] * s[1],
        m[2] * s[2],
        m[3] * s[0],
        m[4] * s[1],
        m[5] * s[2],
        m[6] * s[0],
        m[7] * s[1],
        m[8] * s[2],
    ]
}

pub fn xyz_to_rgb(p: &Primaries) -> Matrix {
    invert(&rgb_to_xyz(p))
}

/// The von Kries adaptation from one white to another, in the cone space of
/// `method`.
pub fn adaptation(from: [f64; 2], to: [f64; 2], method: Adaptation) -> Matrix {
    let cone = match method {
        Adaptation::None => return IDENTITY,
        Adaptation::Bradford => BRADFORD,
        Adaptation::Cat02 => CAT02,
    };
    if from == to {
        return IDENTITY;
    }
    let src = apply(&cone, xyz_of(from));
    let dst = apply(&cone, xyz_of(to));
    let scale = [
        dst[0] / src[0],
        0.0,
        0.0,
        0.0,
        dst[1] / src[1],
        0.0,
        0.0,
        0.0,
        dst[2] / src[2],
    ];
    mul(&invert(&cone), &mul(&scale, &cone))
}

/// RGB in `src` to RGB in `dst`, adapting the white with `method`.
pub fn conversion(src: &Primaries, dst: &Primaries, method: Adaptation) -> Matrix {
    mul(
        &xyz_to_rgb(dst),
        &mul(&adaptation(src.white, dst.white, method), &rgb_to_xyz(src)),
    )
}

/// RGB in `src` to CIE XYZ with a D65 white, which is the display connection
/// space of OCIO's display transforms.
pub fn to_xyz_d65(src: &Primaries, method: Adaptation) -> Matrix {
    mul(&adaptation(src.white, D65, method), &rgb_to_xyz(src))
}

/// CIE XYZ with a D65 white to RGB in `dst`.
pub fn from_xyz_d65(dst: &Primaries, method: Adaptation) -> Matrix {
    mul(&xyz_to_rgb(dst), &adaptation(D65, dst.white, method))
}

/// The saturation matrix of the ACES CTL's `calc_sat_adjust_matrix`: a mix of
/// each colour with its own luminance, `sat` of the colour and the rest grey.
pub fn saturation(sat: f64, luma: [f64; 3]) -> Matrix {
    let mut m = [0.0; 9];
    for row in 0..3 {
        for col in 0..3 {
            m[row * 3 + col] = (1.0 - sat) * luma[col] + if row == col { sat } else { 0.0 };
        }
    }
    m
}

/// A uniform scale.
pub fn scale(s: f64) -> Matrix {
    [s, 0.0, 0.0, 0.0, s, 0.0, 0.0, 0.0, s]
}

/// The luminance weights of a space: the Y row of its RGB to XYZ matrix.
pub fn luminance(p: &Primaries) -> [f64; 3] {
    let m = rgb_to_xyz(p);
    [m[3], m[4], m[5]]
}

pub fn mul(a: &Matrix, b: &Matrix) -> Matrix {
    let mut out = [0.0; 9];
    for row in 0..3 {
        for col in 0..3 {
            out[row * 3 + col] = (0..3).map(|k| a[row * 3 + k] * b[k * 3 + col]).sum();
        }
    }
    out
}

pub fn apply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    [
        m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
        m[3] * v[0] + m[4] * v[1] + m[5] * v[2],
        m[6] * v[0] + m[7] * v[1] + m[8] * v[2],
    ]
}

/// The inverse. Every matrix here comes from a set of real primaries, which
/// are never collinear, so the determinant is never near zero; a singular one
/// would be a typo in a constant above, and is better caught loudly.
pub fn invert(m: &Matrix) -> Matrix {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6]);
    assert!(det.abs() > 1e-12, "a singular colour matrix: {m:?}");
    let inv = 1.0 / det;
    [
        (m[4] * m[8] - m[5] * m[7]) * inv,
        (m[2] * m[7] - m[1] * m[8]) * inv,
        (m[1] * m[5] - m[2] * m[4]) * inv,
        (m[5] * m[6] - m[3] * m[8]) * inv,
        (m[0] * m[8] - m[2] * m[6]) * inv,
        (m[2] * m[3] - m[0] * m[5]) * inv,
        (m[3] * m[7] - m[4] * m[6]) * inv,
        (m[1] * m[6] - m[0] * m[7]) * inv,
        (m[0] * m[4] - m[1] * m[3]) * inv,
    ]
}

pub fn to_f32(m: &Matrix) -> [f32; 9] {
    m.map(|v| v as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_matrix(name: &str, got: &Matrix, expected: &Matrix, tolerance: f64) {
        for (i, (g, e)) in got.iter().zip(expected).enumerate() {
            assert!(
                (g - e).abs() <= tolerance,
                "{name}[{i}]: derived {g}, published {e}\nderived {got:?}"
            );
        }
    }

    /// The anchors these matrices are checked against are published, not
    /// derived here: AP0 to XYZ is in SMPTE ST 2065-1 and the Academy's
    /// S-2008-001, and AP1 to XYZ and the AP0/AP1 pair are in S-2014-004
    /// (ACEScg), all to ten decimal places. Agreement to 1e-9 says the
    /// derivation is the one the Academy used, and the primaries above are
    /// typed correctly.
    #[test]
    fn the_ap0_and_ap1_matrices_are_the_published_ones() {
        assert_matrix(
            "AP0 to XYZ",
            &rgb_to_xyz(&AP0),
            &[
                0.9525523959, 0.0000000000, 0.0000936786, //
                0.3439664498, 0.7281660966, -0.0721325464, //
                0.0000000000, 0.0000000000, 1.0088251844,
            ],
            1e-9,
        );
        assert_matrix(
            "AP1 to XYZ",
            &rgb_to_xyz(&AP1),
            &[
                0.6624541811, 0.1340042065, 0.1561876870, //
                0.2722287168, 0.6740817658, 0.0536895174, //
                -0.0055746495, 0.0040607335, 1.0103391003,
            ],
            1e-9,
        );
        assert_matrix(
            "AP0 to AP1",
            &conversion(&AP0, &AP1, Adaptation::Bradford),
            &[
                1.4514393161, -0.2365107469, -0.2149285693, //
                -0.0765537734, 1.1762296998, -0.0996759264, //
                0.0083161484, -0.0060324498, 0.9977163014,
            ],
            1e-9,
        );
        assert_matrix(
            "AP1 to AP0",
            &conversion(&AP1, &AP0, Adaptation::Bradford),
            &[
                0.6954522414, 0.1406786965, 0.1638690622, //
                0.0447945634, 0.8596711185, 0.0955343182, //
                -0.0055258826, 0.0040252103, 1.0015006723,
            ],
            1e-9,
        );
    }

    /// ACEScg and ACES2065-1 to linear Rec.709 with a Bradford D60 to D65
    /// adaptation: the matrices every ACES config carries for its "Linear
    /// Rec.709" spaces, the first of them the one the test config in
    /// `ocio::tests` is written with. Checked to the five decimals they are
    /// usually quoted to, which is as far as they are relied on here.
    #[test]
    fn acescg_to_rec709_is_the_one_configs_carry() {
        assert_matrix(
            "AP1 to Rec.709 (Bradford)",
            &conversion(&AP1, &REC709, Adaptation::Bradford),
            &[
                1.70505, -0.62179, -0.08326, //
                -0.13026, 1.14080, -0.01055, //
                -0.02400, -0.12897, 1.15297,
            ],
            1e-5,
        );
        assert_matrix(
            "AP0 to Rec.709 (Bradford)",
            &conversion(&AP0, &REC709, Adaptation::Bradford),
            &[
                2.52169, -1.13413, -0.38756, //
                -0.27648, 1.37272, -0.09624, //
                -0.01538, -0.15298, 1.16835,
            ],
            1e-5,
        );
    }

    /// The camera matrices are checked against the vendors' own: ARRI's
    /// ALEXA IDT and Sony's S-Gamut3 and S-Gamut3.Cine IDTs publish their
    /// camera-to-ACES matrices, and both are reproduced from the primaries
    /// with CAT02. Six decimals is what ARRI prints; Sony prints ten.
    #[test]
    fn camera_matrices_match_the_vendors_published_ones() {
        assert_matrix(
            "AWG3 to AP0 (CAT02)",
            &conversion(&ARRI_WIDE_GAMUT_3, &AP0, Adaptation::Cat02),
            &[
                0.680206, 0.236137, 0.083658, //
                0.085415, 1.017471, -0.102886, //
                0.002057, -0.062563, 1.060506,
            ],
            2e-6,
        );
        assert_matrix(
            "S-Gamut3 to AP0 (CAT02)",
            &conversion(&S_GAMUT3, &AP0, Adaptation::Cat02),
            &[
                0.7529825954, 0.1433702162, 0.1036471884, //
                0.0217076974, 1.0153188355, -0.0370265329, //
                -0.0094160528, 0.0033704179, 1.0060456349,
            ],
            1e-6,
        );
        assert_matrix(
            "S-Gamut3.Cine to AP0 (CAT02)",
            &conversion(&S_GAMUT3_CINE, &AP0, Adaptation::Cat02),
            &[
                0.6387886672, 0.2723514337, 0.0888599292, //
                -0.0039159060, 1.0880732309, -0.0841573249, //
                -0.0299072021, -0.0264325799, 1.0563397820,
            ],
            1e-6,
        );
    }

    /// Two more cameras whose matrices are published, each pinning down which
    /// adaptation its vendor used. RED's REDWideGamutRGB to ACES matrix is
    /// reproduced with Bradford to 2e-4 — its printed third row sums to
    /// 1.00013, so it is itself rounded — where CAT02 misses it by up to
    /// 6e-3. The "Linear CinemaGamut D55" matrix the ACES configs carry for
    /// Canon is reproduced with CAT02 to all nine of its printed decimals.
    #[test]
    fn red_and_canon_matrices_match_their_published_ones() {
        assert_matrix(
            "RWG to AP0 (Bradford)",
            &conversion(&RED_WIDE_GAMUT, &AP0, Adaptation::Bradford),
            &[
                0.785043, 0.083844, 0.131118, //
                0.023172, 1.087892, -0.111055, //
                -0.073769, -0.314639, 1.388537,
            ],
            2e-4,
        );
        assert_matrix(
            "Cinema Gamut to AP0 (CAT02)",
            &conversion(&CANON_CINEMA_GAMUT, &AP0, Adaptation::Cat02),
            &[
                0.763064455, 0.149021161, 0.087914384, //
                0.003657457, 1.10696038, -0.110617837, //
                -0.009407794, -0.218383305, 1.227791099,
            ],
            1e-8,
        );
    }

    /// Any conversion maps the source white to the destination white once
    /// adapted, and leaves it where it was when not. That is the property
    /// the D60-simulation outputs rely on.
    #[test]
    fn white_goes_to_white_only_when_adapted() {
        let adapted = apply(&conversion(&AP1, &REC709, Adaptation::Bradford), [1.0, 1.0, 1.0]);
        for c in adapted {
            assert!((c - 1.0).abs() < 1e-9, "{adapted:?}");
        }
        let simulated = apply(&conversion(&AP1, &REC709, Adaptation::None), [1.0, 1.0, 1.0]);
        assert!(
            (simulated[0] - simulated[2]).abs() > 0.05,
            "the ACES white should stay warm on a D65 display: {simulated:?}"
        );
    }
}
