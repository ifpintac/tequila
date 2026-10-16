//! Every material type in the catalog renders something a person would accept.
//!
//! The catalog offers thirty-four presets and the Material tab lists all of
//! them. Each is a claim: pick this one and you get glass, or brushed metal, or
//! velvet. Nothing checked that any of them produced an image at all.
//!
//! This is the same audit that found eleven parameters exposed and unread, run
//! against the presets rather than the controls. It cannot say a material looks
//! *right* — that needs an eye, or the golden images `docs/05` asks for — but
//! it can say a material is not black where it should not be, not featureless
//! where it should not be, and not identical to another entry in the list.
//!
//! Every exemption below is a fact about the renderer, written down. Two of
//! them were guesses of mine that the test disproved, which is the argument for
//! writing the check before writing the list.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{Category, MaterialType};
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::pins::{composite, studio_preset};
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

/// A sphere in a three-point studio, which is what a person judging a finish
/// would set up.
fn render(material_type: MaterialType) -> Framebuffer {
    render_material(material_type.defaults())
}

/// The same studio, for a material that is a variation on a preset rather
/// than a preset itself.
fn render_material(material: tequila_materials::openpbr::Material) -> Framebuffer {
    let studio = tequila_scene::demo::studio_environment();
    let lit = composite(
        studio.image(),
        &studio_preset("three-point").expect("a shipped preset"),
    );

    let scene = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 64, 32))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::IDENTITY)],
        vec![material],
        Arc::new(Environment::new(Arc::new(lit), 1.0)),
    );

    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(64, 64);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(
            vec3(0.0, 0.4, 3.4),
            Vec3::ZERO,
            Vec3::Y,
        ),
        ..RenderContext::default()
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        48,
    );
    framebuffer
}

/// The pixels the sphere covers, as linear RGB.
///
/// A box well inside the silhouette, so the backdrop cannot carry the
/// measurement for a material that renders as nothing.
fn subject(image: &Framebuffer) -> Vec<Spectrum> {
    let mut out = Vec::new();
    for y in 20..44 {
        for x in 20..44 {
            out.push(image.pixel(x, y).mean());
        }
    }
    out
}

fn mean_of(pixels: &[Spectrum]) -> f32 {
    pixels.iter().map(|p| p.luminance()).sum::<f32>() / pixels.len() as f32
}

/// Whether this preset is a light standing in for a light.
///
/// Its defaults carry an emission of 1500, so it is supposed to be far beyond
/// the white point. Judging it against the same brightness bound as a plastic
/// would be asking a lamp to look like a teacup.
fn is_an_emitter(material_type: MaterialType) -> bool {
    material_type.defaults().emission_luminance > 0.0
        || material_type.category() == Category::Light
}

/// The Thin Film preset has to be *coloured*, and its thickness has to be
/// what decides the colour.
///
/// This is a render rather than a unit test because of how the last version
/// failed. There was a unit test asserting that the interference function
/// returned different values at different thicknesses, and it passed — while
/// the function it tested was `pub`, called by nothing, and reached by no
/// material in the renderer. Every Thin Film sphere in the catalog came out
/// as plain grey metal and the test was happy.
///
/// So each claim here is made against an image of the preset a person
/// actually picks. Changing the thickness is the sharpest of them: a film
/// whose colour does not move when its thickness does is not interfering
/// with anything, whatever else it may be doing.
#[test]
fn thin_film_renders_iridescent_and_its_thickness_sets_the_colour() {
    let saturation = |p: &Spectrum| {
        let high = p.r.max(p.g).max(p.b);
        let low = p.r.min(p.g).min(p.b);
        if high <= 1.0e-6 {
            0.0
        } else {
            (high - low) / high
        }
    };
    let mean_saturation = |image: &Framebuffer| {
        let pixels = subject(image);
        pixels.iter().map(saturation).sum::<f32>() / pixels.len() as f32
    };
    // Hue as a pair of ratios, which needs no colour space and does not move
    // when the surface merely gets brighter or darker.
    let chroma = |image: &Framebuffer| {
        let pixels = subject(image);
        let mut sum = Spectrum::ZERO;
        for p in &pixels {
            sum += *p;
        }
        let mean = sum * (1.0 / pixels.len() as f32);
        let total = (mean.r + mean.g + mean.b).max(1.0e-6);
        (mean.r / total, mean.b / total)
    };

    let preset = MaterialType::ThinFilm.defaults();
    let film = render_material(preset.clone());
    assert!(
        mean_saturation(&film) > 0.2,
        "the thin film rendered grey: mean saturation {:.4}",
        mean_saturation(&film)
    );

    // Thicker film, same everything else.
    let thicker = render_material(tequila_materials::openpbr::Material {
        thin_film_thickness: 300.0,
        ..preset.clone()
    });
    let (r0, b0) = chroma(&film);
    let (r1, b1) = chroma(&thicker);
    let moved = (r0 - r1).abs() + (b0 - b1).abs();
    assert!(
        moved > 0.05,
        "the colour did not follow the thickness: 120 nm gave (r {r0:.3}, \
         b {b0:.3}) and 300 nm gave (r {r1:.3}, b {b1:.3})"
    );

    // And the weight really is an off switch. Bare titanium is very slightly
    // warm, so this is not a claim of zero saturation — it is a claim that
    // almost all of the colour was the film.
    let bare = render_material(tequila_materials::openpbr::Material {
        thin_film_weight: 0.0,
        ..preset
    });
    assert!(
        mean_saturation(&bare) < 0.5 * mean_saturation(&film),
        "turning the film off left {:.4} of the {:.4} saturation behind",
        mean_saturation(&bare),
        mean_saturation(&film)
    );
}

/// Types that render black, with the reason each is allowed to.
///
/// Empty. It held one entry — `Cutaway`, which set base weight, specular
/// weight *and* opacity to zero and so went black where it should have
/// vanished, because opacity was unimplemented. The note said "when that lands
/// this entry stops being true and the check below says so", and that is
/// exactly how it came off: the staleness check failed the moment cutouts
/// started working.
const LEGITIMATELY_BLACK: &[(MaterialType, &str)] = &[];

/// Pairs that render identically, with the reason it is not a defect.
const LEGITIMATELY_ALIKE: &[(MaterialType, MaterialType, &str)] = &[
    (
        MaterialType::Advanced,
        MaterialType::Generic,
        "both are the whole catalog with nothing hidden; they differ in which \
         parameters the panel shows, not in how they shade",
    ),
    // Scattering Medium and Cutaway were a pair here — two ways of being
    // invisible — until the volume learned to scatter (September 26). As
    // this list predicted, that day the pair stopped matching.
];

/// The four light presets share one set of defaults.
///
/// What separates a spotlight from a point light is the light node's own
/// settings, not the material standing in for it, so four catalog entries
/// mapping to one material is the design rather than an oversight.
fn both_are_lights(a: MaterialType, b: MaterialType) -> bool {
    a.category() == Category::Light && b.category() == Category::Light
}

#[test]
fn every_material_type_renders_something() {
    let mut black = Vec::new();
    let mut featureless = Vec::new();
    let mut nonfinite = Vec::new();
    let mut stale = Vec::new();

    for material_type in MaterialType::ALL {
        let pixels = subject(&render(material_type));

        if pixels
            .iter()
            .any(|p| !p.r.is_finite() || !p.g.is_finite() || !p.b.is_finite())
        {
            nonfinite.push(format!("{material_type:?}"));
            continue;
        }

        let mean = mean_of(&pixels);
        let allowed_black = LEGITIMATELY_BLACK.iter().any(|(t, _)| *t == material_type);
        if mean < 0.002 && !allowed_black {
            black.push(format!("{material_type:?} ({mean:.5})"));
        }
        // The other direction: an exemption that has stopped being true hides
        // whatever breaks next.
        if mean >= 0.002 && allowed_black {
            stale.push(format!("{material_type:?} is no longer black"));
        }

        // Nothing but a light should be pinned beyond the white point across
        // the whole subject, which is a surface with no shading left in it.
        if !is_an_emitter(material_type) {
            let clipped = pixels.iter().filter(|p| p.luminance() > 12.0).count();
            if clipped > pixels.len() / 2 {
                featureless.push(format!("{material_type:?}"));
            }
        }
    }

    assert!(
        nonfinite.is_empty(),
        "these material types render NaN or infinity, which poisons the \
         accumulation buffer:\n  {}",
        nonfinite.join("\n  ")
    );
    assert!(
        black.is_empty(),
        "these material types render black under a three-point studio, so \
         choosing one looks like the renderer failing:\n  {}",
        black.join("\n  ")
    );
    assert!(
        featureless.is_empty(),
        "these material types render with no shading left in them:\n  {}",
        featureless.join("\n  ")
    );
    assert!(
        stale.is_empty(),
        "these are excused in LEGITIMATELY_BLACK and no longer need to be; \
         take them off the list:\n  {}",
        stale.join("\n  ")
    );
}

/// The catalog is presets, not decoration: two entries that render identically
/// are one entry and a confusing extra.
///
/// §1 says a material type is "a parameter preset over one shading model", so
/// two of them *can* legitimately be close. What they must not be is
/// indistinguishable, because then the list is teaching the user a distinction
/// the renderer does not make.
#[test]
fn material_types_are_not_duplicates_of_each_other() {
    let rendered: Vec<(MaterialType, Vec<Spectrum>)> = MaterialType::ALL
        .into_iter()
        .map(|t| (t, subject(&render(t))))
        .collect();

    let mut identical = Vec::new();
    for (i, (a, left)) in rendered.iter().enumerate() {
        for (b, right) in rendered.iter().skip(i + 1) {
            let difference: f32 = left
                .iter()
                .zip(right)
                .map(|(l, r)| (l.r - r.r).abs() + (l.g - r.g).abs() + (l.b - r.b).abs())
                .sum::<f32>()
                / left.len() as f32;
            let excused = both_are_lights(*a, *b)
                || LEGITIMATELY_ALIKE
                    .iter()
                    .any(|(x, y, _)| (x == a && y == b) || (x == b && y == a));
            if difference < 1e-4 && !excused {
                identical.push(format!("{a:?} and {b:?}"));
            }
        }
    }

    assert!(
        identical.is_empty(),
        "these material types render identically, so the catalog offers a \
         choice that is not one:\n  {}",
        identical.join("\n  ")
    );

    // And the other direction, which `LEGITIMATELY_BLACK` has had all along
    // and this list had not. An excuse that has stopped being true hides
    // whatever breaks next, and one of these pairs is excused only because a
    // feature is missing — when volumetric transport lands, this is what says
    // to take it off.
    let mut stale = Vec::new();
    for (a, b, reason) in LEGITIMATELY_ALIKE {
        let left = rendered.iter().find(|(t, _)| t == a).map(|(_, p)| p);
        let right = rendered.iter().find(|(t, _)| t == b).map(|(_, p)| p);
        let (Some(left), Some(right)) = (left, right) else {
            continue;
        };
        let difference: f32 = left
            .iter()
            .zip(right)
            .map(|(l, r)| (l.r - r.r).abs() + (l.g - r.g).abs() + (l.b - r.b).abs())
            .sum::<f32>()
            / left.len() as f32;
        if difference >= 1e-4 {
            stale.push(format!("{a:?} and {b:?} ({reason})"));
        }
    }
    assert!(
        stale.is_empty(),
        "these are excused in LEGITIMATELY_ALIKE and now render differently; \
         take them off the list:\n  {}",
        stale.join("\n  ")
    );
}
