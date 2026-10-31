//! OCIO v2's `BuiltinTransform` registry: the transforms a config names by
//! `style` instead of carrying them in a file.
//!
//! The ACES configs lean on these for everything that is not a plain matrix:
//! the ACES log encodings, the camera encodings and, above all, the ACES
//! output transforms, which OCIO v2 moved out of LUT files and into the
//! library. Each entry here expands a style name into the same chain of
//! operations OCIO builds for it — written from the ACES CTL and the vendors'
//! published formulas, with every matrix derived from chromaticities in
//! [`crate::ocio::primaries`] — so that a config which says
//! `!<BuiltinTransform> {style: ACES-OUTPUT - ...}` develops the picture the
//! rest of a pipeline reading that config will see.
//!
//! # The display connection space
//!
//! OCIO splits every ACES output transform in two at CIE XYZ with a D65
//! white: an `ACES-OUTPUT` builtin takes ACES2065-1 to that XYZ, and a
//! `DISPLAY` builtin takes the XYZ to one display's primaries and encoding.
//! Linear 1.0 in that XYZ is the SDR white, and 100 nits wherever a display
//! is absolute (PQ); an HDR output's 1000-nit peak is therefore Y = 10.
//!
//! The ACES 1.0 SDR outputs are defined per display, each clipping to its own
//! primaries, and the split leaves no display to clip to. They are left
//! unclipped here and the display encoding's quantization clips, which gives
//! the same code values for every display the 1.0 ODTs were written for. The
//! "lim" outputs of ACES 1.1 clip to their limiting primaries explicitly, as
//! the CTL does, and the "sim" outputs work in the display's own primaries
//! and leave the white where the simulation puts it.
//!
//! # Not here
//!
//! The ACES 2.0 output transforms (OCIO 2.4's `..._2.0` styles) are reported
//! by name rather than approximated: their tone scale, chroma compression and
//! gamut compression are a body of work on the scale of this whole module,
//! and an approximation would be exactly the quietly-wrong picture OCIO
//! exists to prevent.

use crate::ocio::aces::Tonescale;
use crate::ocio::curves::{Curve, HALF_MAX};
use crate::ocio::primaries::{self as p, Adaptation, Matrix, Primaries};
use crate::ocio::Op;
use std::sync::Arc;
use tequila_core::math::Vec3;

/// The ops a style expands to, or why it cannot be built.
pub fn ops(style: &str) -> Result<Vec<Op>, String> {
    if let Some((_, build)) = REGISTRY.iter().find(|(name, _)| *name == style) {
        let mut ops = Vec::new();
        build(&mut ops);
        return Ok(ops);
    }
    if style.starts_with("ACES-OUTPUT") && style.ends_with("_2.0") {
        return Err(format!(
            "BuiltinTransform `{style}`: the ACES 2.0 output transforms are not implemented"
        ));
    }
    Err(format!("BuiltinTransform `{style}` is not one Tequila implements"))
}

/// Every style [`ops`] can build, in registry order.
pub fn styles() -> impl Iterator<Item = &'static str> {
    REGISTRY.iter().map(|(name, _)| *name)
}

type Builder = fn(&mut Vec<Op>);

static REGISTRY: &[(&str, Builder)] = &[
    ("IDENTITY", |_| {}),
    // --- Utilities ---------------------------------------------------------
    ("UTILITY - ACES-AP0_to_CIE-XYZ-D65_BFD", |ops| {
        ops.push(matrix(&p::to_xyz_d65(&p::AP0, Adaptation::Bradford)))
    }),
    ("UTILITY - ACES-AP1_to_CIE-XYZ-D65_BFD", |ops| {
        ops.push(matrix(&p::to_xyz_d65(&p::AP1, Adaptation::Bradford)))
    }),
    ("UTILITY - ACES-AP1_to_LINEAR-REC709_BFD", |ops| {
        ops.push(matrix(&p::conversion(&p::AP1, &p::REC709, Adaptation::Bradford)))
    }),
    // --- ACES colour spaces ------------------------------------------------
    ("ACEScg_to_ACES2065-1", |ops| ops.push(ap1_to_ap0())),
    ("ACEScct_to_ACES2065-1", |ops| {
        ops.push(curve(Curve::AcesCct));
        ops.push(ap1_to_ap0());
    }),
    ("ACEScc_to_ACES2065-1", |ops| {
        ops.push(curve(Curve::AcesCc));
        ops.push(ap1_to_ap0());
    }),
    ("ACESproxy10i_to_ACES2065-1", |ops| {
        ops.push(curve(Curve::AcesProxy10));
        ops.push(ap1_to_ap0());
    }),
    ("ADX10_to_ACES2065-1", |ops| adx(ops, 1023.0, 95.0, 500.0)),
    ("ADX16_to_ACES2065-1", |ops| adx(ops, 65535.0, 1520.0, 8000.0)),
    ("CURVE - ACEScct-LOG_to_LINEAR", |ops| ops.push(curve(Curve::AcesCct))),
    // --- ACES looks --------------------------------------------------------
    ("ACES-LMT - BLUE_LIGHT_ARTIFACT_FIX", |ops| {
        // LMT.Academy.BlueLightArtifactFix.ctl, in ACES2065-1. The CTL
        // writes it for row vectors; this is its transpose. Every row sums
        // to one, so greys are untouched.
        ops.push(matrix(&[
            0.940_437_268_3, -0.018_306_878_7, 0.077_869_610_4, //
            0.008_378_696_9, 0.828_659_993_9, 0.162_961_309_2, //
            0.000_547_126_1, -0.000_883_374_6, 1.000_336_248_6,
        ]))
    }),
    ("ACES-LMT - ACES 1.3 Reference Gamut Compression", |ops| {
        ops.push(matrix(&p::conversion(&p::AP0, &p::AP1, Adaptation::None)));
        ops.push(Op::AcesGamutCompress { inverse: false });
        ops.push(ap1_to_ap0());
    }),
    // --- ACES 1.0 and 1.1 SDR output transforms ----------------------------
    // ODT.Academy.RGBmonitor_100nits_dim, Rec709_100nits_dim and
    // Rec2020_100nits_dim, which differ only in the display they clip to.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO_1.0", |ops| {
        sdr(ops, Surround::Dim, WhiteFit::None, SdrOutput::Adapted)
    }),
    // ODT.Academy.Rec2020_Rec709limited_100nits_dim.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO-REC709lim_1.1", |ops| {
        sdr(ops, Surround::Dim, WhiteFit::None, SdrOutput::Limited(p::REC709))
    }),
    // ODT.Academy.Rec2020_P3D65limited_100nits_dim.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO-P3lim_1.1", |ops| {
        sdr(ops, Surround::Dim, WhiteFit::None, SdrOutput::Limited(p::P3_D65))
    }),
    // ODT.Academy.RGBmonitor_D60sim_100nits_dim and Rec709_D60sim_100nits_dim.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO-D60sim-D65_1.0", |ops| {
        sdr(
            ops,
            Surround::Dim,
            WhiteFit::Scale(0.955),
            SdrOutput::Simulated {
                white: p::ACES_WHITE,
                display: p::REC709,
                connection: Adaptation::None,
            },
        )
    }),
    // ODT.Academy.P3D60_48nits and P3D65_48nits: the cinema outputs that
    // adapt the white, clipped by the display.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-CINEMA_1.0", |ops| {
        sdr(ops, Surround::Dark, WhiteFit::None, SdrOutput::Adapted)
    }),
    // ODT.Academy.P3D65_Rec709limited_48nits.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-CINEMA-REC709lim_1.1", |ops| {
        sdr(ops, Surround::Dark, WhiteFit::None, SdrOutput::Limited(p::REC709))
    }),
    // ODT.Academy.P3D65_D60sim_48nits.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-CINEMA-D60sim-D65_1.1", |ops| {
        sdr(
            ops,
            Surround::Dark,
            WhiteFit::Scale(0.964),
            SdrOutput::Simulated {
                white: p::ACES_WHITE,
                display: p::P3_D65,
                connection: Adaptation::None,
            },
        )
    }),
    // ODT.Academy.P3DCI_48nits: D60 on a DCI-white projector.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-CINEMA-D60sim-DCI_1.0", |ops| {
        sdr(
            ops,
            Surround::Dark,
            WhiteFit::Roll {
                new_white: 0.918,
                width: 0.5,
                scale: 0.96,
            },
            SdrOutput::Simulated {
                white: p::ACES_WHITE,
                display: p::P3_DCI,
                connection: Adaptation::Bradford,
            },
        )
    }),
    // ODT.Academy.P3DCI_D65sim_48nits: D65 on a DCI-white projector.
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-CINEMA-D65sim-DCI_1.1", |ops| {
        sdr(
            ops,
            Surround::Dark,
            WhiteFit::Roll {
                new_white: 0.908,
                width: 0.5,
                scale: 0.9575,
            },
            SdrOutput::Simulated {
                white: p::D65,
                display: p::P3_DCI,
                connection: Adaptation::Bradford,
            },
        )
    }),
    // --- ACES 1.1 HDR output transforms (the RRTODT.Academy.* CTL) ---------
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-1000nit-15nit-REC2020lim_1.1", |ops| {
        hdr(ops, 1000.0, 15.0, p::REC2020)
    }),
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-1000nit-15nit-P3lim_1.1", |ops| {
        hdr(ops, 1000.0, 15.0, p::P3_D65)
    }),
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-2000nit-15nit-REC2020lim_1.1", |ops| {
        hdr(ops, 2000.0, 15.0, p::REC2020)
    }),
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-2000nit-15nit-P3lim_1.1", |ops| {
        hdr(ops, 2000.0, 15.0, p::P3_D65)
    }),
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-4000nit-15nit-REC2020lim_1.1", |ops| {
        hdr(ops, 4000.0, 15.0, p::REC2020)
    }),
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-4000nit-15nit-P3lim_1.1", |ops| {
        hdr(ops, 4000.0, 15.0, p::P3_D65)
    }),
    ("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-CINEMA-108nit-7.2nit-P3lim_1.1", |ops| {
        hdr(ops, 108.0, 7.2, p::P3_D65)
    }),
    // --- Display encodings, from CIE XYZ D65 -------------------------------
    ("DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709", |ops| {
        display_power(ops, &p::REC709, Adaptation::None, 2.4)
    }),
    ("DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.2020", |ops| {
        display_power(ops, &p::REC2020, Adaptation::None, 2.4)
    }),
    ("DISPLAY - CIE-XYZ-D65_to_G2.2-REC.709", |ops| {
        display_power(ops, &p::REC709, Adaptation::None, 2.2)
    }),
    ("DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D65", |ops| {
        display_power(ops, &p::P3_D65, Adaptation::None, 2.6)
    }),
    ("DISPLAY - CIE-XYZ-D65_to_G2.6-P3-DCI-BFD", |ops| {
        display_power(ops, &p::P3_DCI, Adaptation::Bradford, 2.6)
    }),
    ("DISPLAY - CIE-XYZ-D65_to_G2.6-P3-D60-BFD", |ops| {
        display_power(ops, &p::P3_D60, Adaptation::Bradford, 2.6)
    }),
    ("DISPLAY - CIE-XYZ-D65_to_sRGB", |ops| display_srgb(ops, &p::REC709)),
    ("DISPLAY - CIE-XYZ-D65_to_DisplayP3", |ops| display_srgb(ops, &p::P3_D65)),
    ("DISPLAY - CIE-XYZ-D65_to_REC.2100-PQ", |ops| {
        ops.push(matrix(&p::from_xyz_d65(&p::REC2020, Adaptation::None)));
        ops.push(encode(Curve::Pq));
    }),
    ("DISPLAY - CIE-XYZ-D65_to_ST2084-P3-D65", |ops| {
        ops.push(matrix(&p::from_xyz_d65(&p::P3_D65, Adaptation::None)));
        ops.push(encode(Curve::Pq));
    }),
    ("DISPLAY - CIE-XYZ-D65_to_REC.2100-HLG-1000nit", |ops| {
        ops.push(matrix(&p::from_xyz_d65(&p::REC2020, Adaptation::None)));
        // 1000 nits is 10 in the connection space; HLG's display light is
        // normalized to its peak.
        ops.push(matrix(&p::scale(0.1)));
        // The inverse of BT.2100's OOTF, gamma 1.2 at a 1000-nit peak:
        // display light back to scene light, by the luminance.
        ops.push(Op::LumaPower {
            weights: [0.2627, 0.6780, 0.0593],
            gamma: 1.0 / 1.2,
            floor: 1e-4,
        });
        ops.push(encode(Curve::Hlg));
    }),
    // DCDM: X'Y'Z' for a cinema projector, gamma 2.6 against the 52.37-nit
    // reference of SMPTE ST 428-1 with white at 48, and its PQ counterpart.
    ("DISPLAY - CIE-XYZ-D65_to_DCDM-D65", |ops| {
        ops.push(matrix(&p::scale(48.0 / 52.37)));
        ops.push(Op::Exponent {
            value: [1.0 / 2.6; 3],
        });
    }),
    ("DISPLAY - CIE-XYZ-D65_to_ST2084-DCDM-D65", |ops| ops.push(encode(Curve::Pq))),
    ("CURVE - ST-2084_to_LINEAR", |ops| ops.push(curve(Curve::Pq))),
    ("CURVE - LINEAR_to_ST-2084", |ops| ops.push(encode(Curve::Pq))),
    // --- Cameras, each its vendor's log curve then its gamut to AP0 --------
    ("ARRI_ALEXA-LOGC-EI800-AWG_to_ACES2065-1", |ops| {
        camera(ops, Curve::LogC3Ei800, &p::ARRI_WIDE_GAMUT_3, Adaptation::Cat02)
    }),
    ("ARRI_LOGC4_to_ACES2065-1", |ops| {
        camera(ops, Curve::LogC4, &p::ARRI_WIDE_GAMUT_4, Adaptation::Cat02)
    }),
    ("CURVE - ARRI_LOGC4_to_LINEAR", |ops| ops.push(curve(Curve::LogC4))),
    ("SONY_SLOG3-SGAMUT3_to_ACES2065-1", |ops| {
        camera(ops, Curve::SLog3, &p::S_GAMUT3, Adaptation::Cat02)
    }),
    ("SONY_SLOG3-SGAMUT3.CINE_to_ACES2065-1", |ops| {
        camera(ops, Curve::SLog3, &p::S_GAMUT3_CINE, Adaptation::Cat02)
    }),
    ("CURVE - SONY_SLOG3_to_LINEAR", |ops| ops.push(curve(Curve::SLog3))),
    // RED's own IPP2 matrices use Bradford, unlike the others.
    ("RED_LOG3G10-RWG_to_ACES2065-1", |ops| {
        camera(ops, Curve::Log3G10, &p::RED_WIDE_GAMUT, Adaptation::Bradford)
    }),
    ("CURVE - RED_LOG3G10_to_LINEAR", |ops| ops.push(curve(Curve::Log3G10))),
    ("CANON_CLOG2-CGAMUT_to_ACES2065-1", |ops| {
        camera(ops, Curve::CLog2, &p::CANON_CINEMA_GAMUT, Adaptation::Cat02)
    }),
    ("CURVE - CANON_CLOG2_to_LINEAR", |ops| ops.push(curve(Curve::CLog2))),
    ("CANON_CLOG3-CGAMUT_to_ACES2065-1", |ops| {
        camera(ops, Curve::CLog3, &p::CANON_CINEMA_GAMUT, Adaptation::Cat02)
    }),
    ("CURVE - CANON_CLOG3_to_LINEAR", |ops| ops.push(curve(Curve::CLog3))),
    ("PANASONIC_VLOG-VGAMUT_to_ACES2065-1", |ops| {
        camera(ops, Curve::VLog, &p::V_GAMUT, Adaptation::Cat02)
    }),
    ("CURVE - PANASONIC_VLOG_to_LINEAR", |ops| ops.push(curve(Curve::VLog))),
    // Apple Log is defined on Rec.2020 primaries.
    ("APPLE_LOG_to_ACES2065-1", |ops| {
        camera(ops, Curve::AppleLog, &p::REC2020, Adaptation::Cat02)
    }),
    ("CURVE - APPLE_LOG_to_LINEAR", |ops| ops.push(curve(Curve::AppleLog))),
];

fn matrix(m: &Matrix) -> Op {
    Op::Matrix {
        matrix: p::to_f32(m),
        offset: Vec3::ZERO,
    }
}

fn ap1_to_ap0() -> Op {
    matrix(&p::conversion(&p::AP1, &p::AP0, Adaptation::None))
}

/// A curve decoding to linear.
fn curve(curve: Curve) -> Op {
    Op::Curve {
        curve,
        to_linear: true,
    }
}

/// A curve encoding from linear.
fn encode(curve: Curve) -> Op {
    Op::Curve {
        curve,
        to_linear: false,
    }
}

fn clamp(lo: f32, hi: f32) -> Op {
    Op::Clamp { lo, hi }
}

fn camera(ops: &mut Vec<Op>, log: Curve, gamut: &Primaries, adaptation: Adaptation) {
    ops.push(curve(log));
    ops.push(matrix(&p::conversion(gamut, &p::AP0, adaptation)));
}

fn display_power(ops: &mut Vec<Op>, display: &Primaries, adaptation: Adaptation, gamma: f32) {
    ops.push(matrix(&p::from_xyz_d65(display, adaptation)));
    ops.push(Op::Exponent {
        value: [1.0 / gamma; 3],
    });
}

fn display_srgb(ops: &mut Vec<Op>, display: &Primaries) {
    ops.push(matrix(&p::from_xyz_d65(display, Adaptation::None)));
    ops.push(Op::MonCurve {
        gamma: [2.4; 3],
        offset: [0.055; 3],
        inverse: true,
    });
}

/// The Academy's ADX to ACES (`ACEScsc.Academy.ADX10_to_ACES.ctl` and its
/// 16-bit twin): code values to channel-dependent density, to
/// channel-independent density, to relative exposure, to ACES.
fn adx(ops: &mut Vec<Op>, code_scale: f32, black_code: f32, codes_per_density: f32) {
    ops.push(Op::Matrix {
        matrix: [
            code_scale / codes_per_density,
            0.0,
            0.0,
            0.0,
            code_scale / codes_per_density,
            0.0,
            0.0,
            0.0,
            code_scale / codes_per_density,
        ],
        offset: Vec3::splat(-black_code / codes_per_density),
    });
    // CDD_TO_CID and EXP_TO_ACES, transposed from the CTL's row-vector
    // convention. Each row sums to one, so a neutral density stays neutral.
    ops.push(matrix(&[
        0.75573, 0.22197, 0.02230, //
        0.05901, 0.96928, -0.02829, //
        0.16134, 0.07406, 0.76460,
    ]));
    ops.push(curve(Curve::AdxDensity));
    ops.push(matrix(&[
        0.72286, 0.12630, 0.15084, //
        0.11923, 0.76418, 0.11659, //
        0.01427, 0.08213, 0.90359,
    ]));
}

/// The RRT up to its tone scale (`rrt_sweeteners`): glow and red modifier
/// in AP0, then into AP1 and the 0.96 global desaturation.
fn rrt_preamble(ops: &mut Vec<Op>) {
    ops.push(Op::AcesSweeteners { inverse: false });
    ops.push(clamp(0.0, f32::INFINITY));
    ops.push(matrix(&p::conversion(&p::AP0, &p::AP1, Adaptation::None)));
    ops.push(clamp(0.0, HALF_MAX));
    ops.push(matrix(&p::saturation(0.96, p::luminance(&p::AP1))));
}

/// The viewing environment an SDR output is graded for.
#[derive(Clone, Copy)]
enum Surround {
    /// A cinema: no compensation.
    Dark,
    /// A video display: the 0.9811 surround gamma and the 0.93 desaturation
    /// of `ODT_Common`, because a dim room makes the same picture look
    /// lighter and more colourful than a dark one.
    Dim,
}

/// How a white simulated on a display with another white is kept from
/// clipping one channel first.
#[derive(Clone, Copy)]
enum WhiteFit {
    None,
    /// `min(linearCV, 1) * scale`.
    Scale(f32),
    /// `roll_white_fwd`, then `min(linearCV, new_white) * scale`.
    Roll { new_white: f32, width: f32, scale: f32 },
}

/// Where an SDR output leaves the picture in the connection space.
#[derive(Clone, Copy)]
enum SdrOutput {
    /// The ACES white adapted to D65 with Bradford, as the 1.0 ODTs do,
    /// clipped by the display.
    Adapted,
    /// Adapted, then clipped to these (D65-white) primaries, as the 1.1
    /// "limited" ODTs do.
    Limited(Primaries),
    /// The white moved only as far as `white`, in the display's own
    /// primaries, and clipped there. `connection` is the adaptation the
    /// display's builtin will undo on its way back from XYZ D65, so that the
    /// display receives exactly the RGB the CTL computes.
    Simulated {
        white: [f64; 2],
        display: Primaries,
        connection: Adaptation,
    },
}

/// An ACES 1.0 SDR output transform: RRT and the 48-nit ODT tone scale,
/// `ODT.Academy.*_48nits` and `*_100nits_dim`.
fn sdr(ops: &mut Vec<Op>, surround: Surround, fit: WhiteFit, output: SdrOutput) {
    rrt_preamble(ops);
    ops.push(Op::AcesTonescale {
        tonescale: Arc::new(Tonescale::sdr()),
        inverse: false,
    });

    match fit {
        WhiteFit::None => {}
        WhiteFit::Scale(scale) => {
            ops.push(clamp(f32::NEG_INFINITY, 1.0));
            ops.push(matrix(&p::scale(scale as f64)));
        }
        WhiteFit::Roll {
            new_white,
            width,
            scale,
        } => {
            ops.push(Op::RollWhite {
                new_white,
                width,
                inverse: false,
            });
            ops.push(clamp(f32::NEG_INFINITY, new_white));
            ops.push(matrix(&p::scale(scale as f64)));
        }
    }

    if let Surround::Dim = surround {
        // `darkSurround_to_dimSurround`: Y to the power 0.9811 at constant
        // chromaticity, which is a scale of the whole colour by Y^-0.0189.
        let luma = p::luminance(&p::AP1);
        ops.push(Op::LumaPower {
            weights: [luma[0] as f32, luma[1] as f32, luma[2] as f32],
            gamma: 0.9811,
            floor: 1e-10,
        });
        ops.push(matrix(&p::saturation(0.93, luma)));
    }

    match output {
        SdrOutput::Adapted => ops.push(matrix(&p::to_xyz_d65(&p::AP1, Adaptation::Bradford))),
        SdrOutput::Limited(limit) => {
            ops.push(matrix(&p::conversion(&p::AP1, &limit, Adaptation::Bradford)));
            ops.push(clamp(0.0, 1.0));
            ops.push(matrix(&p::to_xyz_d65(&limit, Adaptation::None)));
        }
        SdrOutput::Simulated {
            white,
            display,
            connection,
        } => {
            let to_display = p::mul(
                &p::xyz_to_rgb(&display),
                &p::mul(
                    &p::adaptation(p::ACES_WHITE, white, Adaptation::Bradford),
                    &p::rgb_to_xyz(&p::AP1),
                ),
            );
            ops.push(matrix(&to_display));
            ops.push(clamp(0.0, 1.0));
            ops.push(matrix(&p::to_xyz_d65(&display, connection)));
        }
    }
}

/// An ACES 1.1 HDR output transform: `outputTransform` of
/// `ACESlib.OutputTransforms.ctl` with a 0.0001-nit black, the given
/// mid-grey and peak, a dark surround and black stretched to zero, as every
/// `RRTODT.Academy.*_ST2084` sets it.
fn hdr(ops: &mut Vec<Op>, y_max: f32, y_mid: f32, limit: Primaries) {
    const Y_MIN: f32 = 0.0001;
    rrt_preamble(ops);
    ops.push(Op::AcesTonescale {
        tonescale: Arc::new(Tonescale::hdr(Y_MIN, y_mid, y_max)),
        inverse: false,
    });
    // `limit_to_primaries`, which the CTL applies to the XYZ of the ACES
    // white *before* adapting it: into the limiting primaries with no
    // adaptation, clipped to [0, 1], and back.
    ops.push(matrix(&p::conversion(&p::AP1, &limit, Adaptation::None)));
    ops.push(clamp(0.0, 1.0));
    // Then the CAT to D65, and from linear code values to the connection
    // space's 100-nit unit. `STRETCH_BLACK` makes the code value's zero
    // zero nits, so it is a plain scale by the peak.
    let to_xyz = p::mul(
        &p::adaptation(p::ACES_WHITE, p::D65, Adaptation::Bradford),
        &p::rgb_to_xyz(&limit),
    );
    ops.push(matrix(&p::mul(&p::scale(y_max as f64 / 100.0), &to_xyz)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocio::Processor;
    use tequila_core::math::vec3;

    fn processor(styles: &[&str]) -> Processor {
        let mut processor = Processor::default();
        for style in styles {
            for op in ops(style).unwrap() {
                processor.push(op);
            }
        }
        processor
    }

    fn inverse_processor(styles: &[&str]) -> Processor {
        let mut processor = Processor::default();
        for style in styles.iter().rev() {
            for op in ops(style).unwrap().into_iter().rev() {
                processor.push(op.inverse().expect("every builtin op inverts"));
            }
        }
        processor
    }

    /// ACEScg in, the way the renderer holds it, to the display's code.
    fn develop(output: &str, display: &str, acescg: Vec3) -> Vec3 {
        processor(&["ACEScg_to_ACES2065-1", output, display]).apply(acescg)
    }

    const SDR_VIDEO: &str = "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-VIDEO_1.0";
    const HDR_1000: &str = "ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - HDR-VIDEO-1000nit-15nit-REC2020lim_1.1";

    #[test]
    fn every_registered_style_builds_and_inverts() {
        let mut count = 0;
        for style in styles() {
            let built = ops(style).unwrap();
            for op in &built {
                assert!(op.inverse().is_some(), "{style}: {op:?} has no inverse");
            }
            count += 1;
        }
        assert!(count >= 60, "{count} styles");
    }

    #[test]
    fn aces_2_and_unknown_styles_are_reported_by_name() {
        let error = ops("ACES-OUTPUT - ACES2065-1_to_CIE-XYZ-D65 - SDR-100nit-REC709_2.0").unwrap_err();
        assert!(error.contains("ACES 2.0") && error.contains("SDR-100nit-REC709_2.0"), "{error}");
        let error = ops("NOT_A_STYLE").unwrap_err();
        assert!(error.contains("NOT_A_STYLE"), "{error}");
    }

    /// 18% grey, black and white through ACES 1.0 SDR Video to Rec.1886 and
    /// sRGB.
    ///
    /// The grey values are **derived here by hand from the CTL constants**,
    /// not read from a published table, and the derivation is short enough
    /// to state: the RRT spline puts 0.18 at its mid point, 4.8 nits; the
    /// 48-nit ODT spline holds 4.8 at 4.8; `Y_2_linCV` makes that
    /// (4.8 - 0.02) / (48 - 0.02) = 0.099625; the dim-surround gamma takes it
    /// to 0.099625^0.9811 = 0.104071; a grey is untouched by every saturation
    /// step and lands on the display's white axis. Rec.1886's 1/2.4 power
    /// then gives 0.38953, and the sRGB curve 0.35595. Black and white are
    /// exact by construction: ACES 0 is the spline's 0.02-nit floor, which
    /// `CINEMA_BLACK` maps to code 0, and anything above the 48-nit knee maps
    /// to linear 1 and code 1.
    #[test]
    fn sdr_video_puts_grey_black_and_white_where_the_ctl_does() {
        let rec1886 = "DISPLAY - CIE-XYZ-D65_to_REC.1886-REC.709";
        let srgb = "DISPLAY - CIE-XYZ-D65_to_sRGB";
        let grey = develop(SDR_VIDEO, rec1886, Vec3::splat(0.18));
        for c in [grey.x, grey.y, grey.z] {
            assert!((c - 0.389_53).abs() < 5e-4, "Rec.1886 grey {grey:?}");
        }
        let grey = develop(SDR_VIDEO, srgb, Vec3::splat(0.18));
        for c in [grey.x, grey.y, grey.z] {
            assert!((c - 0.355_95).abs() < 5e-4, "sRGB grey {grey:?}");
        }
        let black = develop(SDR_VIDEO, srgb, Vec3::ZERO);
        assert!(black.abs().max_component() < 1e-5, "black {black:?}");
        let white = develop(SDR_VIDEO, srgb, Vec3::splat(20.0));
        for c in [white.x, white.y, white.z] {
            assert!(c >= 0.9999, "white {white:?}");
        }
    }

    /// The same three for the 1000-nit HDR output through PQ.
    ///
    /// Grey at 15 nits is the output's *definition* (its `Y_MID`), so that
    /// part is certain; the code value 0.33260 is that luminance through the
    /// ST 2084 formula, computed here. Black is 0 nits by `STRETCH_BLACK`.
    /// The peak is derived here and is not 1000 nits exactly: the CTL limits
    /// to Rec.2020 *before* adapting the ACES white to D65, and in unadapted
    /// Rec.2020 that white's red is 1.024, which the limit clips — so peak
    /// white comes out at Y = 0.99367 of the peak, 993.7 nits.
    #[test]
    fn hdr_1000_puts_grey_black_and_peak_where_the_ctl_does() {
        let to_xyz = processor(&["ACEScg_to_ACES2065-1", HDR_1000]);
        let grey = to_xyz.apply(Vec3::splat(0.18));
        assert!((grey.y * 100.0 - 15.0).abs() < 0.01, "grey at {} nits", grey.y * 100.0);
        let pq = develop(HDR_1000, "DISPLAY - CIE-XYZ-D65_to_REC.2100-PQ", Vec3::splat(0.18));
        for c in [pq.x, pq.y, pq.z] {
            assert!((c - 0.332_60).abs() < 5e-4, "PQ grey {pq:?}");
        }
        assert!(to_xyz.apply(Vec3::ZERO).abs().max_component() < 1e-7);
        let peak = to_xyz.apply(Vec3::splat(1e4));
        assert!((peak.y * 100.0 - 993.67).abs() < 0.5, "peak at {} nits", peak.y * 100.0);
    }

    /// Every output transform is monotonic along the grey axis from black
    /// to far past its peak, in every channel of every display it feeds.
    #[test]
    fn every_output_transform_is_monotonic_in_grey() {
        for style in styles().filter(|s| s.starts_with("ACES-OUTPUT")) {
            let to_xyz = processor(&["ACEScg_to_ACES2065-1", style]);
            let mut previous = Vec3::splat(f32::NEG_INFINITY);
            for i in 0..=400 {
                let exposure = (i as f32 / 400.0 * 30.0 - 16.0).exp2();
                let out = to_xyz.apply(Vec3::splat(exposure));
                for c in 0..3 {
                    assert!(
                        out[c] >= previous[c] - 1e-6,
                        "{style} decreases at {exposure}: {previous:?} then {out:?}"
                    );
                }
                previous = out;
            }
        }
    }

    /// Forward then inverse comes back, for every builtin, on values inside
    /// the range each one keeps (an output transform clips what is past its
    /// peak and the gamut it limits to, and nothing brings that back).
    #[test]
    fn every_builtin_round_trips_inside_its_range() {
        let probes = [
            vec3(0.18, 0.18, 0.18),
            vec3(0.05, 0.04, 0.03),
            vec3(0.3, 0.25, 0.2),
            vec3(0.6, 0.5, 0.55),
        ];
        for style in styles() {
            let forward = processor(&[style]);
            let inverse = inverse_processor(&[style]);
            // Feed each style something in its own input domain.
            let input = |c: Vec3| -> Vec3 {
                if style.starts_with("DISPLAY") || style.starts_with("CURVE - LINEAR") {
                    // Display-linear XYZ, below the SDR white.
                    c
                } else if style.starts_with("CURVE") || style.contains("LOG") || style.contains("ACEScc")
                    || style.contains("proxy") || style.contains("ADX")
                {
                    // Encoded values, mid-range.
                    c + Vec3::splat(0.25)
                } else {
                    c
                }
            };
            for probe in probes {
                let x = input(probe);
                let back = inverse.apply(forward.apply(x));
                // The output transforms' inverses are the CTL's, which undo
                // the red modifier with the modified colour's hue and so only
                // approximately (see `aces::sweeteners_inverse`); half a
                // percent is inside one eight-bit code value.
                let tolerance = if style.starts_with("ACES-OUTPUT") { 5e-3 } else { 2e-4 };
                for c in 0..3 {
                    assert!(
                        (back[c] - x[c]).abs() <= tolerance * x[c].abs().max(0.1),
                        "{style}: {x:?} came back as {back:?}"
                    );
                }
            }
        }
    }

    /// The Narkowicz fit the analytic `DisplayTransform::Aces` uses is a fit
    /// to this transform, so the two should agree in shape: both start at
    /// black, rise through the same S-curve and roll off into white.
    /// Not in value — the fit is known to sit brighter in the mid-tones, and
    /// the exact transform's 18% grey is 0.356 in sRGB to the fit's 0.410 —
    /// so the tolerance is loose: within a tenth of the code range
    /// everywhere, and both curves monotonic and saturating.
    #[test]
    fn the_exact_transform_has_the_shape_of_the_narkowicz_fit() {
        use crate::DisplayTransform;
        use tequila_core::spectrum::Spectrum;
        let srgb = "DISPLAY - CIE-XYZ-D65_to_sRGB";
        let mut worst = 0.0f32;
        for i in 0..=60 {
            let exposure = (i as f32 / 60.0 * 14.0 - 9.0).exp2();
            let scene = Spectrum::from_linear_srgb(Vec3::splat(exposure));
            let exact = develop(SDR_VIDEO, srgb, scene.to_rendering_rgb()).y.clamp(0.0, 1.0);
            let fit = DisplayTransform::Aces.apply(scene).y;
            worst = worst.max((exact - fit).abs());
            assert!((exact - fit).abs() < 0.1, "at {exposure}: exact {exact}, fit {fit}");
        }
        assert!(worst > 0.01, "suspiciously identical: {worst}");
    }
}
