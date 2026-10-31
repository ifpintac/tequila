//! The ACES studio config, end to end: read, every view built, and images
//! developed through it.
//!
//! The fixture is a hand-written subset of `studio-config-v1.0.0_aces-v1.3_
//! ocio-v2.1` (see its header for what is and is not in it). Its views are
//! built from OCIO v2 `BuiltinTransform`s throughout — the ACES output
//! transforms, the display encodings, the ACES and camera log curves — which
//! is the part of a real ACES config that used to be reported unsupported.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tequila_color::ocio::curves::Curve;
use tequila_color::ocio::Config;
use tequila_color::{DisplayPipeline, DisplayTransform, ImageStyle};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/aces-studio-config/studio-config-v1.0.0_aces-v1.3_ocio-v2.1.ocio")
}

fn config() -> Config {
    Config::load(fixture()).expect("the studio config fixture reads")
}

/// The roles, and the displays and views in the order the config's
/// `active_displays` and `active_views` give them — which is the order an
/// application is meant to offer them in, and why the first view of the first
/// display is ACES 1.0 SDR Video on sRGB rather than Raw, which the file
/// lists first.
#[test]
fn the_config_offers_its_active_displays_and_views_in_order() {
    let config = config();
    assert_eq!(config.scene_linear(), Some("ACEScg"));
    assert_eq!(config.role("aces_interchange"), Some("ACES2065-1"));

    let displays: Vec<&str> = config.displays.iter().map(|(d, _)| d.as_str()).collect();
    assert_eq!(displays.len(), 10, "{displays:?}");
    assert_eq!(displays[0], "sRGB - Display");
    assert_eq!(displays[2], "Rec.1886 Rec.709 - Display");

    let (_, views) = &config.displays[0];
    let names: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "ACES 1.0 - SDR Video",
            "ACES 1.0 - SDR Video (D60 sim on D65)",
            "Un-tone-mapped",
            "Raw"
        ]
    );
    // A shared view's `<USE_DISPLAY_NAME>` is the display it is used on.
    assert_eq!(views[0].colorspace, "sRGB - Display");
    assert_eq!(views[0].view_transform.as_deref(), Some("ACES 1.0 - SDR Video"));
    let (_, pq_views) = config
        .displays
        .iter()
        .find(|(d, _)| d == "Rec.2100-PQ - Display")
        .unwrap();
    assert_eq!(pq_views[0].colorspace, "Rec.2100-PQ - Display");

    // What the application opens the config on.
    assert_eq!(
        config.first_buildable_view(),
        Some(("sRGB - Display".to_string(), "ACES 1.0 - SDR Video".to_string()))
    );
}

/// Every view of every display builds a complete pipeline. This is the
/// sentence the roadmap could not say before `BuiltinTransform`: each of
/// these used to come back with the output transform named as unsupported.
#[test]
fn every_view_of_every_display_builds() {
    let config = config();
    let mut count = 0;
    for (display, views) in &config.displays {
        for view in views {
            let processor = config.view_pipeline(display, &view.name);
            assert!(
                processor.is_complete(),
                "{display} / {}: {:?}",
                view.name,
                processor.unsupported
            );
            count += 1;
        }
    }
    assert_eq!(count, 47);
}

/// 18% grey, black and white through the default view. The expected grey,
/// 0.35595 in sRGB, is derived by hand from the CTL constants (see
/// `ocio::builtin::tests::sdr_video_puts_grey_black_and_white_where_the_ctl_does`
/// for the derivation); black is exact by construction.
#[test]
fn aces_sdr_video_on_srgb_puts_grey_black_and_white_where_the_ctl_does() {
    let config = config();
    let processor = config.view_pipeline("sRGB - Display", "ACES 1.0 - SDR Video");
    let grey = processor.apply(Vec3::splat(0.18));
    assert!((grey - Vec3::splat(0.355_95)).abs().max_component() < 5e-4, "{grey:?}");
    assert!(processor.apply(Vec3::ZERO).abs().max_component() < 1e-5);
    let white = processor.apply(Vec3::splat(30.0));
    assert!(white.x.min(white.y).min(white.z) >= 0.9999, "{white:?}");

    let rec709 = config.view_pipeline("Rec.1886 Rec.709 - Display", "ACES 1.0 - SDR Video");
    let grey = rec709.apply(Vec3::splat(0.18));
    assert!((grey - Vec3::splat(0.389_53)).abs().max_component() < 5e-4, "{grey:?}");
}

/// The HDR views put grey at 15 nits — the transform's definition — which
/// through PQ is code 0.33260.
#[test]
fn aces_hdr_video_puts_grey_at_fifteen_nits() {
    let config = config();
    for view in [
        "ACES 1.1 - HDR Video (1000 nits & Rec.2020 lim)",
        "ACES 1.1 - HDR Video (2000 nits & Rec.2020 lim)",
        "ACES 1.1 - HDR Video (4000 nits & P3 lim)",
    ] {
        let processor = config.view_pipeline("Rec.2100-PQ - Display", view);
        let grey = processor.apply(Vec3::splat(0.18));
        assert!((grey - Vec3::splat(0.332_60)).abs().max_component() < 5e-4, "{view}: {grey:?}");
        let nits = Curve::Pq.to_linear(grey.y) * 100.0;
        assert!((nits - 15.0).abs() < 0.02, "{view}: grey at {nits} nits");
    }
}

/// Un-tone-mapped is ACES2065-1 to XYZ and XYZ to sRGB, with no curve
/// between, so it has to agree with the analytic sRGB transform — the same
/// check `docs/05` §1 note 3 asks of the OCIO path, now through the display
/// connection space and two builtins.
#[test]
fn un_tone_mapped_matches_the_analytic_srgb_transform() {
    let config = config();
    let processor = config.view_pipeline("sRGB - Display", "Un-tone-mapped");
    assert!(processor.is_complete(), "{:?}", processor.unsupported);
    for probe in [vec3(0.18, 0.18, 0.18), vec3(0.5, 0.2, 0.05), vec3(0.02, 0.3, 0.6)] {
        let scene = Spectrum::from_linear_srgb(probe);
        let ocio = processor.apply(scene.to_rendering_rgb());
        let analytic = DisplayTransform::Srgb.apply(scene);
        assert!(
            (ocio - analytic).abs().max_component() < 2e-3,
            "{probe:?}: OCIO {ocio:?}, analytic {analytic:?}"
        );
    }
}

/// Raw is a data space, and OCIO never converts data: the view shows the
/// rendering space's values as they are.
#[test]
fn the_raw_view_passes_values_through() {
    let config = config();
    let processor = config.view_pipeline("sRGB - Display", "Raw");
    assert!(processor.is_complete() && processor.is_empty());
}

/// The ACES log spaces through the config, against S-2016-001 and
/// S-2014-003: 18% grey in ACEScg is 0.4135884 in both ACEScct and ACEScc,
/// and both round trip.
#[test]
fn acescct_and_acescc_through_the_config() {
    let config = config();
    for space in ["ACEScct", "ACEScc"] {
        let encode = config.processor("ACEScg", space);
        let decode = config.processor(space, "ACEScg");
        assert!(encode.is_complete() && decode.is_complete());
        let grey = encode.apply(Vec3::splat(0.18));
        assert!((grey - Vec3::splat(0.413_588_4)).abs().max_component() < 2e-6, "{space}: {grey:?}");
        for probe in [vec3(0.001, 0.02, 0.18), vec3(0.5, 1.0, 4.0), vec3(12.0, 0.3, 0.07)] {
            let back = decode.apply(encode.apply(probe));
            assert!(
                (back - probe).abs().max_component() < 2e-5 * probe.max_component().max(1.0),
                "{space}: {probe:?} came back as {back:?}"
            );
        }
    }
    // A space can be named by its alias, with any case.
    assert!(config.processor("acescct_ap1", "lin_ap1").is_complete());
    assert!(config.processor("ACES - ACEScct", "aces2065_1").is_complete());
}

/// The camera spaces through the config: each vendor's 18% grey code value
/// (S-Log3 code 420, LogC3 EI 800 0.391007, LogC4 0.2784, Log3G10 a third,
/// V-Log code 433 — all published) comes out as 18% grey in ACEScg. Grey
/// stays grey because every camera matrix maps its own white to the ACES
/// white. LogC4 goes through the config's `LogCameraTransform`, the others
/// through builtins, so this checks both routes.
#[test]
fn camera_grey_lands_on_aces_grey() {
    let config = config();
    for (space, code) in [
        ("S-Log3 S-Gamut3", 420.0 / 1023.0),
        ("S-Log3 S-Gamut3.Cine", 420.0 / 1023.0),
        ("ARRI LogC3 (EI800)", 0.391_007),
        ("ARRI LogC4", 0.278_4),
        ("Log3G10 REDWideGamutRGB", 1.0 / 3.0),
        ("V-Log V-Gamut", 433.0 / 1023.0),
    ] {
        let processor = config.processor(space, "ACEScg");
        assert!(processor.is_complete(), "{space}: {:?}", processor.unsupported);
        let grey = processor.apply(Vec3::splat(code));
        assert!(
            (grey - Vec3::splat(0.18)).abs().max_component() < 2e-3,
            "{space}: code {code} became {grey:?}"
        );
    }
}

/// A display space converts back to the scene through the default view
/// transform, Un-tone-mapped, as OCIO bridges the two references.
#[test]
fn a_display_space_converts_back_to_the_scene() {
    let config = config();
    let out = config.processor("ACEScg", "sRGB - Display");
    let back = config.processor("sRGB - Display", "ACEScg");
    assert!(out.is_complete() && back.is_complete(), "{:?} {:?}", out.unsupported, back.unsupported);
    for probe in [vec3(0.18, 0.18, 0.18), vec3(0.4, 0.2, 0.1)] {
        let round = back.apply(out.apply(probe));
        assert!((round - probe).abs().max_component() < 1e-4, "{probe:?} came back as {round:?}");
    }
}

/// An image developed through the default view, the way the application
/// develops one: `ImageStyle::develop_through` with an OCIO pipeline. A
/// horizontal ramp of eighteen stops must come out monotonic, black on the
/// left, white on the right, with 18% grey at code 91 (0.35595 of 255,
/// give or take the dither's one code).
#[test]
fn an_image_develops_through_the_studio_config() {
    let config = config();
    let processor = config.view_pipeline("sRGB - Display", "ACES 1.0 - SDR Video");
    let pipeline = DisplayPipeline::Ocio {
        processor: Arc::new(processor),
        label: "sRGB - Display / ACES 1.0 - SDR Video".into(),
    };

    let (width, height) = (256usize, 4usize);
    // Column 0 is black; the rest span 2^-12 to 2^6 of ACEScg, with
    // 0.18 at the column where log2(0.18) falls.
    let exposure = |x: usize| -> f32 {
        if x == 0 {
            0.0
        } else {
            (-12.0 + 18.0 * (x as f32 - 1.0) / (width as f32 - 2.0)).exp2()
        }
    };
    let pixels: Vec<Spectrum> = (0..height)
        .flat_map(|_| (0..width).map(move |x| Spectrum::from_rendering_rgb(Vec3::splat(exposure(x)))))
        .collect();
    let image = ImageStyle::default().develop_through(width, height, &pixels, &pipeline);
    assert_eq!(image.len(), width * height * 4);

    let code = |x: usize, y: usize| image[(y * width + x) * 4 + 1];
    assert_eq!(code(0, 0), 0, "black");
    assert_eq!(code(width - 1, 0), 255, "white");
    // Monotonic along each row, allowing for the ordered dither's one code.
    for y in 0..height {
        for x in 1..width {
            assert!(
                code(x, y) as i32 >= code(x - 1, y) as i32 - 1,
                "row {y} decreases at column {x}"
            );
        }
    }
    let grey_column = (1.0 + (0.18f32.log2() + 12.0) / 18.0 * (width as f32 - 2.0)).round() as usize;
    let grey_exposure = exposure(grey_column);
    let expected = pipeline.apply(Spectrum::from_rendering_rgb(Vec3::splat(grey_exposure))).y * 255.0;
    for y in 0..height {
        assert!(
            (code(grey_column, y) as f32 - expected).abs() <= 1.0,
            "grey at {grey_exposure}: code {} against {expected}",
            code(grey_column, y)
        );
    }
    assert!((expected - 0.355_95 * 255.0).abs() < 1.5, "{expected}");
}

/// A view with a look: the Reference Gamut Compression brings a colour a
/// camera can record but AP1 cannot hold back inside it, and leaves a
/// colour well inside alone. The studio config defines the look but no view
/// uses it, so the view here is the studio config's plus the look.
#[test]
fn the_gamut_compression_look_applies_in_a_view() {
    let text = std::fs::read_to_string(fixture()).unwrap();
    // As checked out: on Windows with CRLF, which the edits below would miss.
    let text = text.replace("\r\n", "\n");
    let text = text.replace(
        "  - !<View> {name: Un-tone-mapped, view_transform: Un-tone-mapped, display_colorspace: <USE_DISPLAY_NAME>}",
        "  - !<View> {name: Un-tone-mapped, view_transform: Un-tone-mapped, display_colorspace: <USE_DISPLAY_NAME>}\n  - !<View> {name: Un-tone-mapped (RGC), view_transform: Un-tone-mapped, display_colorspace: <USE_DISPLAY_NAME>, looks: ACES 1.3 Reference Gamut Compression}",
    );
    let text = text.replace(
        "    - !<Views> [ACES 1.0 - SDR Video, ACES 1.0 - SDR Video (D60 sim on D65), Un-tone-mapped]\n  Display P3",
        "    - !<Views> [ACES 1.0 - SDR Video, ACES 1.0 - SDR Video (D60 sim on D65), Un-tone-mapped, Un-tone-mapped (RGC)]\n  Display P3",
    );
    let config = Config::parse(&text).unwrap();
    let plain = config.view_pipeline("sRGB - Display", "Un-tone-mapped");
    let compressed = config.view_pipeline("sRGB - Display", "Un-tone-mapped (RGC)");
    assert!(compressed.is_complete(), "{:?}", compressed.unsupported);

    let inside = vec3(0.3, 0.2, 0.1);
    assert!((plain.apply(inside) - compressed.apply(inside)).abs().max_component() < 1e-5);

    // Negative in ACEScg, as a camera's saturated blue-green is: a third of
    // the way past AP1's red edge.
    let outside = vec3(-0.05, 0.4, 0.3);
    let to_acescg = config.processor("sRGB - Display", "ACEScg");
    let back_plain = to_acescg.apply(plain.apply(outside));
    let back_compressed = to_acescg.apply(compressed.apply(outside));
    assert!(back_plain.x < -0.04, "{back_plain:?}");
    assert!(back_compressed.x > back_plain.x + 0.02, "{back_compressed:?}");
}
