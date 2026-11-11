//! The studio presets that ship with Tequila, written as code:
//! `cargo run -p tequila-format --example builtin_studios` writes them to
//! `assets/studios` (`docs/studio-presets.md`).
//!
//! Generic and made here, so there are no pictures to license, and framed
//! for a product about 20 cm across. Each is lit as a photographer lights a
//! table: softboxes, strips and cards standing a hand's breadth or two from
//! the product, each brighter in its middle than at its edges as a real
//! softbox's front is (`edge_falloff`), in a room dark enough that gloss
//! paint and chrome have black in them as well as white. The lights are in
//! nits and grow and move with the product (`FramingScale::CamerasAndLights`),
//! so a mixer is lit as a pedal is.
//!
//! A thumbnail already in a preset's folder is kept; one is rendered with
//! `tequila --studio-preset assets/studios/<id> --studio <first view> -w 400 -h 300 -o assets/studios/<id>/thumbnail.png`.

use std::f32::consts::{FRAC_PI_2, PI};
use std::path::Path;
use tequila_core::math::{vec3, Mat4, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::{Camera, Projection};
use tequila_render_core::pins::{GradientStop, Pin, PinBlend, PinKind, PinShape};
use tequila_scene::document::{BackgroundMode, EnvironmentSettings, GroundSettings};
use tequila_scene::lighting::{Intensity, LightSettings, LightShape, SavedCamera};
use tequila_scene::preset::{Framing, FramingScale, PresetLight, StudioPreset};
use tequila_scene::render::{AlphaMode, BitDepth, OutputFormat, RenderSettings};
use tequila_scene::studio::Studio;

/// The bounding-sphere radius the cameras are framed for.
const RADIUS: f32 = 0.1;

/// Where the lights aim: the middle of a product of [`RADIUS`] standing on
/// the floor.
const MIDDLE: Vec3 = Vec3 { x: 0.0, y: 0.04, z: 0.0 };

/// A camera looking at the product's middle from `direction`, far enough
/// that a product of [`RADIUS`] fills the 4:3 frame with a margin.
fn camera(name: &str, direction: Vec3, up: Vec3, focal_length_mm: f32) -> SavedCamera {
    framed(name, direction, up, focal_length_mm, 1.15)
}

/// As [`camera`], the product's sphere `margin` times its radius from the
/// middle of the frame to its top: 1 fills it.
fn framed(name: &str, direction: Vec3, up: Vec3, focal_length_mm: f32, margin: f32) -> SavedCamera {
    let target = vec3(0.0, 0.45 * RADIUS, 0.0);
    let half_height = (13.5 / focal_length_mm).atan();
    let distance = RADIUS * margin / half_height.sin();
    let mut camera = Camera::look_at(target + direction.normalize() * distance, target, up);
    camera.projection = Projection::Perspective { focal_length_mm };
    SavedCamera { name: name.into(), camera, target }
}

fn view(name: &str, camera: &SavedCamera, yaw: f32) -> Studio {
    Studio { name: name.into(), camera: Some(camera.clone()), product_yaw: Some(yaw), ..Studio::default() }
}

/// A softbox `width` by `height` metres standing at `position` and turned
/// to face `target`, `nits` bright. In nits, so the studio lights a large
/// product as it lights a small one when its lights are scaled with it.
/// Not seen by the camera: it lights, and shows in what reflects it. Its
/// height runs as upright as its aim allows, so a fade runs top to bottom.
fn softbox_at(name: &str, position: Vec3, target: Vec3, width: f32, height: f32, nits: f32) -> PresetLight {
    // A panel shines along its own +Z.
    let z = (target - position).normalize();
    let x = if z.y.abs() > 0.999 { Vec3::X } else { Vec3::Y.cross(z).normalize() };
    let y = z.cross(x);
    let mut settings = LightSettings {
        shape: LightShape::Area { width, height, disc: false, two_sided: false },
        intensity: Intensity::Nits(nits),
        ..LightSettings::default()
    };
    settings.visibility.camera = false;
    PresetLight { name: name.into(), settings, transform: Mat4::translation(position) * Mat4::from_columns(x, y, z) }
}

/// What a light's front does: how much its edges fall off (a softbox's
/// diffuser), a fade along it (a graduated scrim) and a grid's spread.
fn front(mut light: PresetLight, falloff: f32, fade: f32, spread: f32) -> PresetLight {
    light.settings.edge_falloff = falloff;
    light.settings.fade = fade;
    light.settings.spread_degrees = spread;
    light
}

/// A softbox's front: brighter in the middle, falling to about half at its
/// edges, as a double-diffused box's is.
fn softbox(light: PresetLight) -> PresetLight {
    front(light, 0.55, 0.0, 180.0)
}

/// A strip light's front: a narrower diffuser, dimmer towards its ends.
fn strip(light: PresetLight) -> PresetLight {
    front(light, 0.45, 0.0, 180.0)
}

/// A white card bouncing light back: dim, and lit almost evenly.
fn card(light: PresetLight) -> PresetLight {
    front(light, 0.2, 0.0, 180.0)
}

/// The light through a coloured gel.
fn gel(mut light: PresetLight, colour: Spectrum) -> PresetLight {
    light.settings.color = colour;
    light
}

fn grey(v: f32) -> Spectrum {
    Spectrum::splat(v)
}

fn rgb(r: f32, g: f32, b: f32) -> Spectrum {
    tequila_core::spectrum::spectrum(r, g, b)
}

/// The whole sphere one tone, laid over the built-in room: what the room
/// is to the lighting and the reflections, not to the eye. Dark, so the
/// softboxes are the light and what is between them in a mirror is black,
/// which is what makes gloss and metal read as gloss and metal.
fn room(name: &str, colour: Spectrum) -> Pin {
    Pin {
        name: name.into(),
        kind: PinKind::Gradient { stops: vec![GradientStop { t: 0.0, color: colour }, GradientStop { t: 1.0, color: colour }], radial: true },
        shape: PinShape::Ellipse,
        direction: Vec3::Y,
        angular_size: Vec2 { x: 2.0 * PI, y: 2.0 * PI },
        softness: 0.0,
        intensity: 1.0,
        show_in_background: false,
        in_lighting: true,
        blend: PinBlend::Over,
        ..Pin::default()
    }
}

/// The backdrop: paper swept up behind the product, from `stops` straight
/// down (t 0) to the horizon (t 0.5) and over the top (t 1). Seen, and
/// lighting nothing, so every view, high or low, has a slope of it behind
/// the product.
fn backdrop(stops: &[(f32, Spectrum)]) -> Pin {
    Pin {
        name: "Backdrop".into(),
        kind: PinKind::Gradient { stops: stops.iter().map(|&(t, color)| GradientStop { t, color }).collect(), radial: true },
        shape: PinShape::Ellipse,
        direction: -Vec3::Y,
        angular_size: Vec2 { x: 2.0 * PI, y: 2.0 * PI },
        softness: 0.0,
        intensity: 1.0,
        show_in_background: true,
        in_lighting: false,
        blend: PinBlend::Over,
        ..Pin::default()
    }
}

/// A pool of coloured light thrown on the backdrop behind the product, as
/// a gelled background light does: brightest at `direction`, gone by the
/// edge of a disc `size` radians across. Seen, lighting nothing.
fn wash(name: &str, direction: Vec3, size: f32, colour: Spectrum) -> Pin {
    Pin {
        name: name.into(),
        kind: PinKind::Gradient {
            stops: vec![
                GradientStop { t: 0.0, color: colour },
                GradientStop { t: 0.35, color: colour * 0.55 },
                GradientStop { t: 0.7, color: colour * 0.15 },
                GradientStop { t: 1.0, color: Spectrum::ZERO },
            ],
            radial: true,
        },
        shape: PinShape::Ellipse,
        direction: direction.normalize(),
        angular_size: Vec2 { x: size, y: size },
        softness: 0.0,
        intensity: 1.0,
        show_in_background: true,
        in_lighting: false,
        blend: PinBlend::Add,
        ..Pin::default()
    }
}

/// What every preset here renders at, `alpha` saying whether it is cut
/// out or keeps its backdrop.
fn render(alpha: AlphaMode) -> RenderSettings {
    RenderSettings {
        width: 3000,
        height: 2250,
        max_samples: 512,
        format: OutputFormat::Png,
        depth: BitDepth::Sixteen,
        alpha_mode: alpha,
        filename_template: "{product}_{view}_{date}".into(),
        ..RenderSettings::default()
    }
}

#[allow(clippy::too_many_arguments)]
fn preset(
    id: &str,
    name: &str,
    description: &str,
    tags: &[&str],
    environment: EnvironmentSettings,
    lights: Vec<PresetLight>,
    cameras: Vec<SavedCamera>,
    studios: Vec<Studio>,
    alpha: AlphaMode,
) -> StudioPreset {
    StudioPreset {
        id: id.into(),
        name: name.into(),
        family: "Tequila Studio".into(),
        size: "Medium".into(),
        description: description.into(),
        tags: tags.iter().map(|t| t.to_string()).chain(["built-in".to_string()]).collect(),
        version: "2".into(),
        environment,
        lighting: Default::default(),
        render: render(alpha),
        image_style: Default::default(),
        lights,
        cameras,
        studios,
        // Framed tight, as a product photographer frames: the product's box
        // fills four fifths of every view, whatever its shape.
        framing: Framing { reference_radius: RADIUS, base_offset: 0.0, scale: FramingScale::CamerasAndLights, fill: 0.8 },
        materials: Vec::new(),
        material_rules: Vec::new(),
        notes: "Generated by crates/tequila-format/examples/builtin_studios.rs.".into(),
    }
}

/// The everyday studio: bright and soft, cut out with its shadow and a
/// short reflection for any background.
///
/// High-key without being flat: a large overhead box and a key box to the
/// left give every face its own tone and a gradient across it, a strip
/// either side behind draws the edges, and a card on the right fills the
/// shadow side. The room is a mid grey rather than white, so a chrome part
/// still mirrors dark between the boxes.
fn soft_studio() -> StudioPreset {
    let hero = camera("Hero", vec3(0.62, 0.42, 0.66), Vec3::Y, 85.0);
    let front_camera = camera("Front", vec3(0.0, 0.12, 1.0), Vec3::Y, 85.0);
    let top = camera("Top", vec3(0.0, 1.0, 0.001), -Vec3::Z, 85.0);
    let lights = vec![
        softbox(softbox_at("Overhead", vec3(0.0, 0.42, 0.06), MIDDLE, 0.7, 0.5, 4.5)),
        softbox(softbox_at("Key", vec3(-0.36, 0.22, 0.26), MIDDLE, 0.4, 0.5, 6.0)),
        strip(softbox_at("Left strip", vec3(-0.3, 0.18, -0.26), MIDDLE, 0.09, 0.45, 9.0)),
        strip(softbox_at("Right strip", vec3(0.3, 0.18, -0.26), MIDDLE, 0.09, 0.45, 9.0)),
        card(softbox_at("Fill card", vec3(0.4, 0.12, 0.3), MIDDLE, 0.45, 0.45, 1.8)),
    ];
    let environment = EnvironmentSettings {
        image: None,
        pins: vec![room("Room", grey(0.06))],
        ground: GroundSettings {
            enabled: true,
            shadows: true,
            reflections: 0.15,
            reflection_blur: 0.05,
            reflection_length: 0.45,
            reflection_fresnel: true,
            ..GroundSettings::default()
        },
        ..EnvironmentSettings::default()
    };
    preset(
        "tequila-soft-studio",
        "Soft Studio",
        "Bright and soft: an overhead softbox, a key box, two edge strips and a fill card close to the product, cut \
         out with a soft shadow and a short reflection. Hero, front, side, back and top views, as transparent 16-bit \
         PNGs at 3000 by 2250.",
        &["product", "softbox", "transparent"],
        environment,
        lights,
        vec![hero.clone(), front_camera.clone(), top.clone()],
        vec![
            view("Hero", &hero, 0.0),
            view("Front", &front_camera, 0.0),
            view("Side", &front_camera, -FRAC_PI_2),
            view("Back", &front_camera, PI),
            view("Top", &top, 0.0),
        ],
        AlphaMode::Straight,
    )
}

/// A catalogue studio: the product cut out on white with a short
/// reflection under it, as a web shop or a data sheet shows it.
///
/// Lit so that a flat top stays its own colour. Seen from high in front,
/// a top face mirrors whatever is high behind the product, so nothing
/// bright is put there: the key is to one side in front, the large soft
/// box overhead is forward of the product, and what is behind is a low,
/// narrow strip that only draws the far edges.
fn catalogue_studio() -> StudioPreset {
    let main = camera("Main", vec3(0.0, 0.82, 0.57), Vec3::Y, 85.0);
    let side = camera("Side", vec3(0.0, 0.2, 1.0), Vec3::Y, 85.0);
    let top = camera("Top", vec3(0.0, 1.0, 0.001), -Vec3::Z, 85.0);
    let lights = vec![
        softbox(softbox_at("Key", vec3(-0.3, 0.26, 0.26), MIDDLE, 0.36, 0.3, 4.4)),
        softbox(softbox_at("Overhead", vec3(0.0, 0.38, 0.2), MIDDLE, 0.55, 0.42, 2.4)),
        softbox(softbox_at("Fill", vec3(0.36, 0.14, 0.22), MIDDLE, 0.4, 0.34, 1.4)),
        softbox(softbox_at("Front", vec3(0.0, 0.08, 0.42), MIDDLE, 0.55, 0.2, 1.0)),
        strip(softbox_at("Back strip", vec3(0.0, 0.12, -0.36), MIDDLE, 0.6, 0.06, 2.4)),
        // What a metal top has to reflect: large and dim, so a painted top
        // is barely touched by it.
        card(softbox_at("High back", vec3(0.0, 0.32, -0.24), MIDDLE, 0.6, 0.3, 0.32)),
    ];
    let environment = EnvironmentSettings {
        image: None,
        pins: vec![room("Room", grey(0.05))],
        background: BackgroundMode::Color,
        background_color: grey(1.0),
        ground: GroundSettings {
            enabled: true,
            shadows: false,
            // The white floor lights what stands on it from below, as a
            // sweep of white paper does: without it every upright face of
            // a metal mirrors a black room.
            bounce: true,
            bounce_strength: 1.0,
            reflections: 0.3,
            reflection_blur: 0.02,
            reflection_length: 0.5,
            ..GroundSettings::default()
        },
        ..EnvironmentSettings::default()
    };
    let mut preset = preset(
        "tequila-catalogue-studio",
        "Catalogue Studio",
        "The product cut out on white with a short reflection, lit so a flat top keeps its colour: a main view from \
         high in front, left, right, rear and top, rendered as transparent 16-bit PNGs at 3000 by 2250.",
        &["product", "catalogue", "packshot", "white", "transparent"],
        environment,
        lights,
        vec![main.clone(), side.clone(), top.clone()],
        vec![
            view("Main", &main, 0.0),
            view("Left", &side, FRAC_PI_2),
            view("Right", &side, -FRAC_PI_2),
            view("Rear", &side, PI),
            view("Top", &top, 0.0),
        ],
        AlphaMode::Straight,
    );
    // A bright, saturated colour stays that colour, as a catalogue wants
    // it: the plain sRGB curve, a little under, where the default eases
    // highlights towards white.
    preset.image_style.display = tequila_color::DisplayTransform::Srgb;
    preset.image_style.exposure = 0.3;
    preset
}

/// A photographer's table: real softboxes standing close to the product
/// in a dark room, on a graduated backdrop.
///
/// A softbox a hand's breadth from a flat face is seen in it from one edge
/// to the other at different angles, so the face carries a gradient and
/// not one flat tone; and between the softboxes the room is dark, so a
/// polished metal has black in it as well as white, which is what makes
/// it read as metal.
fn photo_studio() -> StudioPreset {
    let hero = camera("Hero", vec3(0.62, 0.42, 0.66), Vec3::Y, 85.0);
    let front_camera = camera("Front", vec3(0.0, 0.16, 1.0), Vec3::Y, 85.0);
    let high = camera("High", vec3(0.0, 0.82, 0.57), Vec3::Y, 85.0);
    let top = camera("Top", vec3(0.0, 1.0, 0.001), -Vec3::Z, 85.0);
    let lights = vec![
        // The main light: a wide box up behind the product, tipped down at
        // it. A top face mirrors it, and since it is close, behind a grid and
        // brighter in its middle, what the face mirrors is a sheen that falls
        // away across it and not a white rectangle.
        front(softbox_at("Back sweep", vec3(0.0, 0.3, -0.32), vec3(0.0, 0.02, 0.06), 0.9, 0.5, 3.0), 0.6, 0.0, 90.0),
        // A smaller box high in front, off to the key's side: the light on
        // the faces the camera sees. Not overhead, where a view from above
        // would see it mirrored whole in every flat top.
        softbox(softbox_at("Front top", vec3(-0.1, 0.3, 0.18), MIDDLE, 0.3, 0.24, 7.0)),
        // A tall strip to one side in front: the long highlight down a knob
        // and along a rounded edge.
        strip(softbox_at("Key strip", vec3(-0.34, 0.17, 0.2), MIDDLE, 0.1, 0.42, 16.0)),
        // A strip behind on the other side: a rim that lifts the far edges
        // off the backdrop.
        strip(softbox_at("Rim strip", vec3(0.3, 0.2, -0.28), MIDDLE, 0.08, 0.42, 14.0)),
        // A white card where the key is not, to keep the shadow side from
        // going to black.
        card(softbox_at("Fill card", vec3(0.4, 0.14, 0.3), MIDDLE, 0.4, 0.4, 1.6)),
    ];
    let environment = EnvironmentSettings {
        image: None,
        pins: vec![
            room("Dark room", grey(0.012)),
            // Lit where it lies flat under the product, falling away to dark
            // up the wall.
            backdrop(&[(0.0, grey(0.11)), (0.2, grey(0.095)), (0.3, grey(0.075)), (0.4, grey(0.045)), (0.5, grey(0.02)), (1.0, grey(0.012))]),
        ],
        ground: GroundSettings {
            enabled: true,
            shadows: true,
            reflections: 0.1,
            reflection_blur: 0.08,
            reflection_length: 0.45,
            ..GroundSettings::default()
        },
        ..EnvironmentSettings::default()
    };
    preset(
        "tequila-photo-studio",
        "Photo Studio",
        "A photographer's table: an overhead softbox, a key strip, a rim strip and a fill card standing close to the \
         product in a dark room, on a graduated grey backdrop. Flat faces carry gradients and metals have black in \
         them. Hero, front, high, side, back and top views, as 16-bit PNGs at 3000 by 2250.",
        &["product", "photography", "softbox", "gradient", "backdrop"],
        environment,
        lights,
        vec![hero.clone(), front_camera.clone(), high.clone(), top.clone()],
        vec![
            view("Hero", &hero, 0.0),
            view("Front", &front_camera, 0.0),
            view("High", &high, 0.0),
            view("Side", &front_camera, -FRAC_PI_2),
            view("Back", &front_camera, PI),
            view("Top", &top, 0.0),
        ],
        AlphaMode::None,
    )
}

/// Low key: the product coming out of black, drawn by its edges.
///
/// Two strips behind it, one either side, lay a bright line down every
/// edge and along every rounded corner; a gridded box above and behind
/// puts a sheen across the top that falls away towards the camera; a
/// small gridded box in front gives the faces just enough to read. The
/// floor is black acrylic, mirroring the product, and a faint pool of light
/// on the backdrop behind keeps a dark product from vanishing into it.
fn dark_studio() -> StudioPreset {
    let hero = camera("Hero", vec3(0.66, 0.26, 0.7), Vec3::Y, 100.0);
    let front_camera = camera("Front", vec3(0.0, 0.1, 1.0), Vec3::Y, 100.0);
    let high = camera("High", vec3(0.5, 0.7, 0.5), Vec3::Y, 100.0);
    let lights = vec![
        front(softbox_at("Left rim", vec3(-0.035, 0.13, -0.3), vec3(-0.035, 0.05, 0.1), 0.06, 0.42, 45.0), 0.45, 0.0, 50.0),
        front(softbox_at("Right rim", vec3(0.035, 0.13, -0.3), vec3(0.035, 0.05, 0.1), 0.06, 0.42, 45.0), 0.45, 0.0, 50.0),
        front(softbox_at("Top sheen", vec3(0.0, 0.36, -0.16), MIDDLE, 0.5, 0.3, 5.0), 0.65, 0.0, 70.0),
        front(softbox_at("Front kicker", vec3(-0.28, 0.12, 0.32), MIDDLE, 0.25, 0.25, 1.2), 0.5, 0.0, 60.0),
    ];
    let environment = EnvironmentSettings {
        image: None,
        pins: vec![
            room("Black room", grey(0.002)),
            backdrop(&[(0.0, grey(0.006)), (0.5, grey(0.002)), (1.0, grey(0.001))]),
            wash("Background light", vec3(-0.33, -0.23, -0.91), 0.9, grey(0.05)),
        ],
        ground: GroundSettings {
            enabled: true,
            shadows: true,
            reflections: 0.25,
            reflection_blur: 0.02,
            reflection_length: 0.5,
            reflection_fresnel: true,
            ..GroundSettings::default()
        },
        ..EnvironmentSettings::default()
    };
    let mut preset = preset(
        "tequila-dark-studio",
        "Dark Studio",
        "Low key: the product out of black, its edges drawn by two rim strips behind it, a gridded sheen across \
         its top and a little light on its front, on black acrylic that mirrors it. Hero (low), front and high \
         views, as 16-bit PNGs at 3000 by 2250.",
        &["product", "photography", "low key", "dark", "black", "rim light", "hero"],
        environment,
        lights,
        vec![hero.clone(), front_camera.clone(), high.clone()],
        vec![view("Hero", &hero, 0.0), view("Front", &front_camera, 0.0), view("High", &high, 0.0)],
        AlphaMode::None,
    );
    preset.image_style.contrast = 1.1;
    preset
}

/// White seamless: the web shop's picture, the product on pure white with
/// its soft shadow under it and true colours.
///
/// The backdrop is the brightest thing in the picture and stays white, the
/// product a stop or so under it: a large box overhead, a box either side
/// in front, the right one dimmer so the faces still turn, and a strip
/// behind for the far edges. The white floor lights the product from below
/// as a paper sweep does.
fn white_studio() -> StudioPreset {
    let hero = framed("Hero", vec3(0.62, 0.42, 0.66), Vec3::Y, 85.0, 1.05);
    let front_camera = framed("Front", vec3(0.0, 0.12, 1.0), Vec3::Y, 85.0, 1.05);
    let top = framed("Top", vec3(0.0, 1.0, 0.001), -Vec3::Z, 85.0, 1.05);
    let lights = vec![
        softbox(softbox_at("Overhead", vec3(0.0, 0.42, 0.08), MIDDLE, 0.8, 0.6, 4.0)),
        softbox(softbox_at("Left box", vec3(-0.38, 0.2, 0.26), MIDDLE, 0.5, 0.6, 4.5)),
        softbox(softbox_at("Right box", vec3(0.38, 0.18, 0.26), MIDDLE, 0.5, 0.6, 2.3)),
        strip(softbox_at("Back strip", vec3(0.0, 0.16, -0.34), MIDDLE, 0.6, 0.08, 5.0)),
    ];
    let environment = EnvironmentSettings {
        image: None,
        pins: vec![room("Room", grey(0.12))],
        background: BackgroundMode::Color,
        background_color: grey(1.0),
        ground: GroundSettings {
            enabled: true,
            shadows: true,
            bounce: true,
            bounce_strength: 1.0,
            reflections: 0.0,
            ..GroundSettings::default()
        },
        ..EnvironmentSettings::default()
    };
    let mut preset = preset(
        "tequila-white-studio",
        "White Seamless",
        "The web shop's picture: the product on pure white with a soft shadow under it and true colours, lit by an \
         overhead softbox, a box either side in front and a strip behind. Hero, front, side, back and top views, as \
         16-bit PNGs at 3000 by 2250 on white.",
        &["product", "e-commerce", "white", "seamless", "packshot", "shop", "amazon"],
        environment,
        lights,
        vec![hero.clone(), front_camera.clone(), top.clone()],
        vec![
            view("Hero", &hero, 0.0),
            view("Front", &front_camera, 0.0),
            view("Side", &front_camera, -FRAC_PI_2),
            view("Back", &front_camera, PI),
            view("Top", &top, 0.0),
        ],
        AlphaMode::None,
    );
    preset.image_style.display = tequila_color::DisplayTransform::Srgb;
    // A shop's picture fills its square.
    preset.framing.fill = 0.86;
    preset
}

/// Coloured gels: a neutral key for the product's own colours, and colour
/// at its edges and behind it.
///
/// A teal pool on a charcoal backdrop, an amber rim strip behind on the
/// right and a teal one on the left, so every edge is drawn in colour while
/// the faces the camera sees keep their own.
fn gel_studio() -> StudioPreset {
    let hero = camera("Hero", vec3(0.62, 0.3, 0.7), Vec3::Y, 85.0);
    let front_camera = camera("Front", vec3(0.0, 0.14, 1.0), Vec3::Y, 85.0);
    let teal = rgb(0.1, 0.62, 1.0);
    let amber = rgb(1.0, 0.42, 0.1);
    let lights = vec![
        softbox(softbox_at("Key", vec3(-0.3, 0.26, 0.28), MIDDLE, 0.34, 0.42, 5.0)),
        gel(front(softbox_at("Amber rim", vec3(0.035, 0.13, -0.3), vec3(0.035, 0.05, 0.1), 0.06, 0.42, 40.0), 0.45, 0.0, 50.0), amber),
        gel(front(softbox_at("Teal rim", vec3(-0.035, 0.13, -0.3), vec3(-0.035, 0.05, 0.1), 0.06, 0.42, 32.0), 0.45, 0.0, 50.0), teal),
        front(softbox_at("Top", vec3(0.0, 0.38, -0.05), MIDDLE, 0.45, 0.3, 2.0), 0.6, 0.0, 90.0),
    ];
    let environment = EnvironmentSettings {
        image: None,
        pins: vec![
            room("Dark room", grey(0.006)),
            backdrop(&[(0.0, grey(0.03)), (0.3, grey(0.018)), (0.5, grey(0.01)), (1.0, grey(0.006))]),
            wash("Teal wash", vec3(-0.33, -0.23, -0.91), 0.9, teal * 0.09),
        ],
        ground: GroundSettings {
            enabled: true,
            shadows: true,
            reflections: 0.12,
            reflection_blur: 0.05,
            reflection_length: 0.5,
            reflection_fresnel: true,
            ..GroundSettings::default()
        },
        ..EnvironmentSettings::default()
    };
    preset(
        "tequila-gel-studio",
        "Gel Studio",
        "Colour at the edges: a neutral key softbox for the product's own colours, an amber rim strip and a teal \
         one behind it and a teal pool of light on a charcoal backdrop. Hero and front views, as 16-bit PNGs at 3000 \
         by 2250.",
        &["product", "photography", "colour", "gel", "rim light", "accent", "dramatic"],
        environment,
        lights,
        vec![hero.clone(), front_camera.clone()],
        vec![view("Hero", &hero, 0.0), view("Front", &front_camera, 0.0)],
        AlphaMode::None,
    )
}

/// Flat lay: the product seen from straight above, lying on pale paper by
/// a window.
///
/// One large soft source low on the left, as a window is, throws a soft
/// shadow off to the right and a gradient across the top; a white card on
/// the right lifts the shadow side. Nothing is overhead, where a flat top
/// would mirror it whole.
fn flat_studio() -> StudioPreset {
    let top = framed("Top", vec3(0.0, 1.0, 0.001), -Vec3::Z, 70.0, 1.05);
    let high = camera("High", vec3(0.0, 0.85, 0.5), Vec3::Y, 70.0);
    let lights = vec![
        front(softbox_at("Window", vec3(-0.55, 0.3, -0.05), MIDDLE, 0.9, 0.7, 7.0), 0.4, 0.0, 180.0),
        card(softbox_at("Card", vec3(0.45, 0.14, 0.02), MIDDLE, 0.5, 0.4, 1.0)),
    ];
    let paper = rgb(0.74, 0.72, 0.69);
    let environment = EnvironmentSettings {
        image: None,
        pins: vec![room("Room", grey(0.1))],
        background: BackgroundMode::Color,
        background_color: paper,
        ground: GroundSettings {
            enabled: true,
            shadows: true,
            bounce: true,
            bounce_strength: 0.6,
            bounce_color: paper,
            reflections: 0.0,
            ..GroundSettings::default()
        },
        ..EnvironmentSettings::default()
    };
    let mut preset = preset(
        "tequila-flat-studio",
        "Flat Lay",
        "From straight above: the product lying on pale paper by a window, a soft shadow off to one side and a card \
         lifting the other. Top and high views, as 16-bit PNGs at 3000 by 2250.",
        &["product", "flat lay", "top down", "overhead", "window light", "paper", "lifestyle"],
        environment,
        lights,
        vec![top.clone(), high.clone()],
        vec![view("Top", &top, 0.0), view("High", &high, 0.0)],
        AlphaMode::None,
    );
    preset.image_style.display = tequila_color::DisplayTransform::Srgb;
    preset
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/studios");
    for preset in [soft_studio(), catalogue_studio(), photo_studio(), dark_studio(), white_studio(), gel_studio(), flat_studio()] {
        let folder = root.join(&preset.id);
        let file = tequila_format::studio_preset::write_folder(&preset, &folder, None).expect("writes");
        println!("wrote {}", file.display());
    }
}
