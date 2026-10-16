//! Lights painted onto the environment sphere (`docs/01` §1.5).
//!
//! `TEQUILA_SPEC.md` calls the HDRI Editor a "pin-based lighting compositor
//! over a 360° canvas", and `docs/01` §295 says how it reaches the renderer:
//! pins "composite into the environment as an ordered layer stack evaluated
//! into the same equirectangular texture, after which the CDF is rebuilt".
//!
//! That sentence is the whole design. A pin is not a new kind of light with its
//! own sampling code — it is paint on the environment map, so every pin is
//! importance-sampled by the machinery that already exists for an HDRI, casts
//! the soft shadows an area light casts, and appears in reflections because it
//! is genuinely there in the image the surface reflects.
//!
//! ## Why this matters more than an HDRI loader
//!
//! No environment maps ship with this project and none can: §0.2 forbids
//! bundling KeyShot's, and CC0 replacements are a download rather than
//! something the repository carries. A scene therefore has the built-in studio
//! or whatever the user found themselves.
//!
//! `docs/02` §420 points at the way out — the studio presets to ship are
//! "procedurally-generated softbox studio setups built from HDRI Editor pins
//! (which cost nothing to ship — they are just pin definitions in a small
//! file)". Three numbers and a size, rather than forty megabytes of EXR.
//!
//! ## The shape of a softbox
//!
//! A real softbox is a rectangle of diffuser at a distance. Seen from the
//! product it subtends some angle, and its edge is not sharp — the diffuser
//! wraps, and the corner of the frame is dimmer than the middle. Both matter
//! for the highlight it leaves on a curved surface, which is the thing a
//! product photographer is actually arranging.
//!
//! So a pin is an angular rectangle with a soft edge, not a point. Its
//! falloff runs along each axis independently, which is what gives a long thin
//! strip light the stretched highlight it is chosen for.

use rayon::prelude::*;
use tequila_core::math::{Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_texture::image::{ColorSpace, Image};
use std::collections::HashMap;
use std::sync::Arc;

/// Decoded image-pin sources, by the path the pin names. The compositor does
/// not read files: whoever owns the cache decodes each source once and hands
/// the pixels in, so dragging an image pin re-composites rather than
/// re-decodes.
pub type PinImages = HashMap<String, Arc<Image>>;

/// One colour along a gradient pin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// Where along the gradient, 0 to 1.
    pub t: f32,
    pub color: Spectrum,
}

/// What a pin paints inside its footprint (`TEQUILA_SPEC.md` §1.5's three
/// kinds).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum PinKind {
    /// A flat, soft-edged source: a softbox.
    #[default]
    Light,
    /// A picture stretched over the footprint — a window, a photographed
    /// softbox, a city skyline. Its colours are radiance, scaled by the
    /// pin's intensity and tinted by its colour.
    Image { path: String },
    /// Colours blended between stops. `radial` measures along the angle from
    /// the pin's own direction out to the edge of its footprint — centred on
    /// the nadir with a footprint of the whole sphere, that is a floor-to-
    /// ceiling backdrop gradient. Otherwise it runs bottom to top across the
    /// footprint.
    Gradient { stops: Vec<GradientStop>, radial: bool },
}

/// How a pin combines with what is under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PinBlend {
    /// Added on top: a light is more light, and two overlapping softboxes are
    /// brighter where they cross.
    #[default]
    Add,
    /// Laid over, hiding what is under it in proportion to its coverage — how
    /// a backdrop gradient replaces the environment behind the product rather
    /// than washing over it.
    Over,
}

/// What a pin paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PinShape {
    /// An angular rectangle with a soft edge: a softbox, or a strip light when
    /// one axis is much longer than the other.
    #[default]
    Rectangle,
    /// An angular disc, likewise soft-edged. A beauty dish or a bare bulb.
    Ellipse,
}

/// One light painted onto the environment sphere.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub name: String,
    /// What it paints: a light, an image or a gradient.
    pub kind: PinKind,
    pub shape: PinShape,
    /// Where it sits, as a unit direction in environment space — the same
    /// space the equirectangular image is indexed in, so a pin is unaffected
    /// by the environment's rotation until that rotation is applied to
    /// everything together.
    pub direction: Vec3,
    /// How wide and tall it is *as seen from the centre*, in radians. An
    /// angular size rather than a physical one, because the environment is a
    /// sphere at infinity and there is no distance for a metre to be measured
    /// against.
    pub angular_size: Vec2,
    /// Rotation of the rectangle about its own direction, in radians. What
    /// turns a strip light from horizontal to vertical.
    pub roll: f32,
    /// Edge softness, 0 (hard edge) to 1 (falls off from the very centre).
    pub softness: f32,
    /// Radiance added at the centre, before the colour.
    pub intensity: f32,
    pub color: Spectrum,
    /// Whether the camera sees this light directly, behind the product.
    ///
    /// Off is the studio arrangement: the softbox lights the product and shows
    /// in its reflections, and the backdrop stays clean. The alternative --
    /// turning the whole background to a flat colour -- takes the reflections
    /// off chrome along with the backdrop, because a path that has only ever
    /// bounced off mirrors is still one the camera sees along.
    ///
    /// Defaults to **off**, which is the product-shot arrangement and the
    /// better-looking one: the backdrop stays as the environment made it while
    /// the light does its work. Turning it on puts the softbox itself in the
    /// picture, which is what you want when the light is meant to be seen.
    pub show_in_background: bool,
    /// Whether it lights the scene and shows in reflections. Off with
    /// `show_in_background` on is `docs/03` §5's "BackgroundOnly": a gradient
    /// that changes what is behind the product without touching how it is
    /// lit, which is what makes gradient pins useful.
    pub in_lighting: bool,
    pub blend: PinBlend,
    pub enabled: bool,
}

impl Default for Pin {
    fn default() -> Self {
        Pin {
            name: "Light".into(),
            kind: PinKind::Light,
            shape: PinShape::Rectangle,
            direction: Vec3::Y,
            // About 34 degrees across: a softbox close enough to wrap a small
            // product, which is the usual studio arrangement.
            angular_size: Vec2 { x: 0.6, y: 0.6 },
            roll: 0.0,
            softness: 0.4,
            intensity: 4.0,
            color: Spectrum::splat(1.0),
            show_in_background: false,
            in_lighting: true,
            blend: PinBlend::Add,
            enabled: true,
        }
    }
}

impl Pin {
    /// An image pin showing `path`, `width` radians across, sized to a 4:3
    /// frame until the panel is given the image's own aspect.
    pub fn image(name: &str, path: &str, direction: Vec3, width: f32) -> Pin {
        Pin {
            name: name.into(),
            kind: PinKind::Image { path: path.into() },
            direction,
            angular_size: Vec2 {
                x: width,
                y: width * 0.75,
            },
            // A picture has its own edges.
            softness: 0.0,
            intensity: 1.0,
            show_in_background: true,
            blend: PinBlend::Over,
            ..Pin::default()
        }
    }

    /// A backdrop gradient, `bottom` at the floor to `top` overhead, seen by
    /// the camera and not lighting anything.
    pub fn backdrop_gradient(bottom: Spectrum, top: Spectrum) -> Pin {
        Pin {
            name: "Backdrop".into(),
            kind: PinKind::Gradient {
                stops: vec![
                    GradientStop {
                        t: 0.0,
                        color: bottom,
                    },
                    GradientStop { t: 1.0, color: top },
                ],
                radial: true,
            },
            direction: -Vec3::Y,
            // The whole sphere: the angle from the nadir runs to pi.
            angular_size: Vec2 {
                x: std::f32::consts::TAU,
                y: std::f32::consts::TAU,
            },
            softness: 0.0,
            intensity: 1.0,
            show_in_background: true,
            in_lighting: false,
            blend: PinBlend::Over,
            ..Pin::default()
        }
    }

    fn is_radial(&self) -> bool {
        matches!(self.kind, PinKind::Gradient { radial: true, .. })
    }

    /// Where `direction` falls in the footprint, as offsets `(u, v)` from the
    /// centre in -1..1 along the pin's own right and up, and the soft-edged
    /// coverage there. `None` outside.
    fn footprint(&self, direction: Vec3) -> Option<(f32, f32, f32)> {
        let axis = if self.direction.length() > 1e-6 {
            self.direction.normalize()
        } else {
            return None;
        };
        // Behind the pin there is nothing, and the tangent-plane projection
        // below is meaningless there.
        let along = direction.dot(axis);
        if along <= 0.0 {
            return None;
        }

        let (right, up) = tangents(axis, self.roll);
        // The angular offset along each axis. `atan2` against the component
        // along the pin's own direction rather than `asin` of the tangent
        // component, so the offset keeps growing past 90 degrees instead of
        // folding back.
        let du = direction.dot(right).atan2(along);
        let dv = direction.dot(up).atan2(along);

        let (half_u, half_v) = (
            (self.angular_size.x * 0.5).max(1e-4),
            (self.angular_size.y * 0.5).max(1e-4),
        );
        let (nu, nv) = (du / half_u, dv / half_v);

        let inside = match self.shape {
            PinShape::Rectangle => nu.abs().max(nv.abs()),
            PinShape::Ellipse => (nu * nu + nv * nv).sqrt(),
        };
        let mask = self.edge(inside)?;
        Some((nu, nv, mask))
    }

    /// A radial gradient's position, 0 at the pin's direction to 1 at the
    /// edge of its footprint, and the coverage there. The footprint here is
    /// a cap of angular radius half the width, which may reach all the way
    /// round to the opposite pole.
    fn radial(&self, direction: Vec3) -> Option<(f32, f32)> {
        let axis = self.direction.try_normalize()?;
        let radius = (self.angular_size.x * 0.5).clamp(1e-4, std::f32::consts::PI);
        let angle = direction.dot(axis).clamp(-1.0, 1.0).acos();
        let t = angle / radius;
        // A cap reaching the far pole covers it too, rather than leaving a
        // pinhole where the angle equals the radius.
        if radius >= std::f32::consts::PI - 1e-4 {
            return Some((t.min(1.0), self.edge(t.min(0.999_9))?));
        }
        Some((t, self.edge(t)?))
    }

    /// Soft-edge coverage at `inside`, 0 at the centre to 1 at the edge.
    fn edge(&self, inside: f32) -> Option<f32> {
        if inside >= 1.0 {
            return None;
        }
        // Softness sets where the falloff starts. At 0 the pin is flat to its
        // edge; at 1 it falls from the centre, which is the gaussian-ish blob
        // a bare bulb makes.
        let start = (1.0 - self.softness.clamp(0.0, 1.0)).clamp(0.0, 0.999);
        if inside <= start {
            return Some(1.0);
        }
        let t = (inside - start) / (1.0 - start);
        // Smoothstep, so the edge has no visible crease where it meets the
        // flat middle. A linear ramp shows a ring on a glossy surface.
        let t = t.clamp(0.0, 1.0);
        Some(1.0 - t * t * (3.0 - 2.0 * t))
    }

    /// What this pin paints in `direction`: the radiance, and how much of
    /// what is under it that covers (`0` nothing, `1` all of it). `None`
    /// outside the pin, or for an image pin whose picture is not loaded.
    pub fn paint(&self, direction: Vec3, images: &PinImages) -> Option<(Spectrum, f32)> {
        if !self.enabled || self.intensity <= 0.0 {
            return None;
        }
        let scale = self.color * self.intensity;
        match &self.kind {
            PinKind::Light => {
                let (_, _, mask) = self.footprint(direction)?;
                Some((scale, mask))
            }
            PinKind::Image { path } => {
                let image = images.get(path)?;
                let (nu, nv, mask) = self.footprint(direction)?;
                // Seen from the centre, the pin's `right` runs to the
                // viewer's left, so the picture's u runs against it; its v
                // runs down the picture and against `up`.
                // As stored: pins paint into the environment image, which is
                // linear sRGB and converted when it is read, so converting
                // here would convert twice.
                let [r, g, b, a] = sample_clamped(image, (1.0 - nu) * 0.5, (1.0 - nv) * 0.5);
                let texel = Spectrum { r, g, b };
                Some((texel * scale, mask * a))
            }
            PinKind::Gradient { stops, radial } => {
                let (t, mask) = if *radial {
                    self.radial(direction)?
                } else {
                    let (_, nv, mask) = self.footprint(direction)?;
                    ((nv + 1.0) * 0.5, mask)
                };
                Some((gradient_at(stops, t) * scale, mask))
            }
        }
    }

    /// How much this pin contributes in `direction`, from 0 outside it to 1 at
    /// its centre.
    ///
    /// Zero outside the pin's angular extent — not a tail that goes on
    /// forever. A softbox has edges, and a falloff that never reaches zero
    /// would raise the whole sky by a little, which is exactly the flat
    /// grey wash that makes a studio render look unlit.
    pub fn coverage(&self, direction: Vec3) -> f32 {
        if !self.enabled || self.intensity <= 0.0 {
            return 0.0;
        }
        let covered = if self.is_radial() {
            self.radial(direction).map(|(_, mask)| mask)
        } else {
            self.footprint(direction).map(|(_, _, mask)| mask)
        };
        covered.unwrap_or(0.0)
    }

    /// The cone that holds everything this pin can touch: its axis, and the
    /// cosine of its angular radius. Widened a little, so a texel on the
    /// edge is never clipped.
    fn cone(&self) -> (Vec3, f32) {
        let axis = self.direction.try_normalize().unwrap_or(Vec3::Y);
        let radius = if self.is_radial() {
            self.angular_size.x * 0.5
        } else {
            let (hu, hv) = (self.angular_size.x * 0.5, self.angular_size.y * 0.5);
            (hu * hu + hv * hv).sqrt()
        } * 1.05
            + 1e-3;
        let cos = if radius >= std::f32::consts::PI {
            -2.0
        } else {
            radius.cos()
        };
        (axis, cos)
    }
}

/// The colour a gradient has at `t`, holding its end colours past its ends.
pub fn gradient_at(stops: &[GradientStop], t: f32) -> Spectrum {
    let mut sorted: Vec<GradientStop> = stops.iter().copied().filter(|s| s.t.is_finite()).collect();
    sorted.sort_by(|a, b| a.t.total_cmp(&b.t));
    let (Some(first), Some(last)) = (sorted.first(), sorted.last()) else {
        return Spectrum::ZERO;
    };
    if t <= first.t {
        return first.color;
    }
    if t >= last.t {
        return last.color;
    }
    for pair in sorted.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if t <= b.t {
            let span = b.t - a.t;
            let f = if span > 1e-6 { (t - a.t) / span } else { 1.0 };
            return a.color * (1.0 - f) + b.color * f;
        }
    }
    last.color
}

/// Bilinear sample of a picture, clamped at its edges, as RGBA. Opaque when
/// it has no alpha.
fn sample_clamped(image: &Image, u: f32, v: f32) -> [f32; 4] {
    if image.width == 0 || image.height == 0 || image.channels < 3 {
        return [0.0; 4];
    }
    let fx = u.clamp(0.0, 1.0) * image.width as f32 - 0.5;
    let fy = v.clamp(0.0, 1.0) * image.height as f32 - 0.5;
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |x: f32, y: f32| {
        let x = (x as i64).clamp(0, image.width as i64 - 1) as usize;
        let y = (y as i64).clamp(0, image.height as i64 - 1) as usize;
        let i = (y * image.width + x) * image.channels;
        let alpha = if image.channels > 3 { image.data[i + 3] } else { 1.0 };
        [image.data[i], image.data[i + 1], image.data[i + 2], alpha]
    };
    let (c00, c10, c01, c11) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = c00[k] + (c10[k] - c00[k]) * tx;
        let bottom = c01[k] + (c11[k] - c01[k]) * tx;
        out[k] = top + (bottom - top) * ty;
    }
    out
}

/// Two unit vectors across `axis`, rotated by `roll` about it.
///
/// The reference vector is chosen away from the axis so the cross product
/// cannot collapse, which it would for a pin pointing straight up if Y were
/// used unconditionally — and straight up is where a studio's key light
/// usually is.
fn tangents(axis: Vec3, roll: f32) -> (Vec3, Vec3) {
    let reference = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let right = reference.cross(axis).normalize();
    let up = axis.cross(right);
    if roll == 0.0 {
        return (right, up);
    }
    let (sin, cos) = roll.sin_cos();
    (right * cos + up * sin, up * cos - right * sin)
}

/// The direction an equirectangular texel looks in.
///
/// `tequila_core::sampling`'s mapping, not a second copy of it. A pin
/// composited under one convention and sampled under another lands somewhere
/// else on the sphere, and the symptom — a highlight that is not where the pin
/// is — reads as a renderer bug rather than as a disagreement about
/// coordinates. Calling the same function makes the two agree by construction
/// rather than by inspection.
#[inline]
pub fn equirect_direction(u: f32, v: f32) -> Vec3 {
    tequila_core::sampling::equirect_to_direction(Vec2 { x: u, y: v })
}

/// Where a pin must sit for its highlight to land on a chosen point.
///
/// This is the HDRI Editor's defining gesture: you do not position a light by
/// thinking about where it is, you point at the place on the product where you
/// want the highlight and the light goes wherever it has to. A specular
/// highlight is the light seen mirrored in the surface, so the answer is the
/// view direction reflected about the surface normal.
///
/// `rotation` is the environment's Y rotation, and leaving it out is the trap.
/// Pins live in the environment's own space and the environment is turned
/// under the scene, so a pin placed by a world-space reflection while the
/// environment is rotated puts the highlight somewhere else — off by exactly
/// that rotation, which looks like the click missing rather than like a
/// forgotten transform.
pub fn direction_for_highlight(
    point: Vec3,
    normal: Vec3,
    camera: Vec3,
    rotation: f32,
) -> Option<Vec3> {
    let to_camera = camera - point;
    if to_camera.length() < 1e-6 || normal.length() < 1e-6 {
        return None;
    }
    let v = to_camera.normalize();
    let n = normal.normalize();
    // Mirror the view about the normal. A grazing view of a surface turned
    // away from the camera reflects below the horizon, which is a legitimate
    // answer — a light under the table — so it is not rejected here.
    let world = (n * (2.0 * v.dot(n)) - v).normalize();
    if !world.is_finite() {
        return None;
    }
    // Into the environment's own space, which is the world turned by -rotation.
    let (sin, cos) = (-rotation).sin_cos();
    Some(Vec3 {
        x: world.x * cos + world.z * sin,
        y: world.y,
        z: -world.x * sin + world.z * cos,
    })
}

/// A direction from a compass bearing and a height above the horizon.
///
/// Bearing 0 looks down -Z, which is where the default camera looks from, and
/// increases to the right. Three numbers that must stay normalized are the
/// wrong control for "move the light round to the left a bit", so this is what
/// the panel edits and what the presets below are written in.
pub fn direction_from(bearing: f32, height: f32) -> Vec3 {
    let (sin_b, cos_b) = bearing.sin_cos();
    let (sin_h, cos_h) = height.sin_cos();
    Vec3 {
        x: cos_h * sin_b,
        y: sin_h,
        z: -cos_h * cos_b,
    }
}

/// The bearing and height of a direction — the inverse of [`direction_from`].
pub fn angles_of(direction: Vec3) -> (f32, f32) {
    let d = if direction.length() > 1e-6 {
        direction.normalize()
    } else {
        Vec3::Y
    };
    (d.x.atan2(-d.z), d.y.clamp(-1.0, 1.0).asin())
}

/// One softbox, in the terms a photographer would describe it.
pub fn softbox(
    name: &str,
    bearing_deg: f32,
    height_deg: f32,
    width: f32,
    softness: f32,
    intensity: f32,
) -> Pin {
    Pin {
        name: name.into(),
        shape: PinShape::Rectangle,
        direction: direction_from(bearing_deg.to_radians(), height_deg.to_radians()),
        angular_size: Vec2 {
            x: width,
            y: width * 0.75,
        },
        roll: 0.0,
        softness,
        intensity,
        color: Spectrum::splat(1.0),
        // A studio setup lights the product; it is not meant to be the
        // backdrop.
        show_in_background: false,
        ..Pin::default()
    }
}

/// Ready-made studio setups, as pin definitions.
///
/// `docs/02` §420 asks for exactly this: "procedurally-generated softbox
/// studio setups built from HDRI Editor pins (which cost nothing to ship —
/// they are just pin definitions in a small file)". A few numbers instead of
/// forty megabytes of EXR, and no licence to worry about.
///
/// Here rather than in the panel that draws the buttons, because the headless
/// renderer wants them too. A studio setup is lighting, not interface.
pub fn studio_presets() -> Vec<(&'static str, &'static str, Vec<Pin>)> {
    vec![
        (
            "Three-point",
            "A key at 45 degrees, a softer fill opposite, and a rim behind. \
             The standard arrangement, and the one to start from.",
            vec![
                softbox("Key", 40.0, 35.0, 0.8, 0.5, 14.0),
                softbox("Fill", -60.0, 15.0, 1.1, 0.8, 4.0),
                softbox("Rim", 165.0, 30.0, 0.5, 0.3, 18.0),
            ],
        ),
        (
            "Overhead softbox",
            "One large source straight above. Flattering on curved plastic \
             and the usual packshot setup.",
            vec![softbox("Overhead", 0.0, 85.0, 1.4, 0.7, 10.0)],
        ),
        (
            "Two strips",
            "Tall strip lights either side, for the long vertical highlights \
             that read as glass or polished metal.",
            vec![strip("Left strip", -75.0, 10.0, 22.0), strip("Right strip", 75.0, 10.0, 22.0)],
        ),
    ]
}

/// A tall narrow source, for the stretched highlight it leaves.
pub fn strip(name: &str, bearing_deg: f32, height_deg: f32, intensity: f32) -> Pin {
    Pin {
        angular_size: Vec2 { x: 0.18, y: 1.5 },
        softness: 0.35,
        ..softbox(name, bearing_deg, height_deg, 0.2, 0.35, intensity)
    }
}

/// The studio setup of that name, however it was capitalised.
pub fn studio_preset(name: &str) -> Option<Vec<Pin>> {
    studio_presets()
        .into_iter()
        .find(|(label, _, _)| label.eq_ignore_ascii_case(name))
        .map(|(_, _, pins)| pins)
}

/// Paint the pins over a copy of `base`.
///
/// Each pin combines with what is under it by its [`PinBlend`], in array
/// order. A light adds: it is a light added to the scene, not a patch
/// replacing part of the image, and two overlapping softboxes are brighter
/// where they cross, which is what two real lights do.
///
/// Returns `base` unchanged when nothing is enabled, so a scene with no pins
/// pays nothing and produces byte-identical output to one that never had the
/// feature.
pub fn composite(base: &Image, pins: &[Pin]) -> Image {
    composite_where(base, pins, |_| true)
}

/// The same, painting only the pins `include` accepts.
///
/// What builds the second image the camera sees: everything the renderer
/// bounces off carries every pin, and the backdrop carries only the ones meant
/// to be seen.
pub fn composite_where(base: &Image, pins: &[Pin], include: impl Fn(&Pin) -> bool) -> Image {
    composite_with(base, pins, &PinImages::new(), include)
}

/// The same, with the decoded pictures image pins show.
pub fn composite_with(
    base: &Image,
    pins: &[Pin],
    images: &PinImages,
    include: impl Fn(&Pin) -> bool,
) -> Image {
    let pins: Vec<&Pin> = pins
        .iter()
        .filter(|pin| pin.enabled && pin.intensity > 0.0 && include(pin))
        .collect();
    if pins.is_empty() {
        return base.clone();
    }

    // Each pin's cone: nothing outside it can be touched, and asking a dot
    // product first rules out almost every texel before the per-pin
    // trigonometry.
    let cones: Vec<(Vec3, f32)> = pins.iter().map(|pin| pin.cone()).collect();

    let mut out = base.clone();
    let (width, height, channels) = (out.width, out.height, out.channels);
    out.data
        .par_chunks_mut(width * channels)
        .enumerate()
        .for_each(|(y, row)| {
            let v = (y as f32 + 0.5) / height as f32;
            for x in 0..width {
                let u = (x as f32 + 0.5) / width as f32;
                let direction = equirect_direction(u, v);
                let at = x * channels;
                let mut value = Spectrum {
                    r: row[at],
                    g: if channels > 1 { row[at + 1] } else { row[at] },
                    b: if channels > 2 { row[at + 2] } else { row[at] },
                };
                let mut touched = false;
                for (pin, (axis, cos)) in pins.iter().zip(&cones) {
                    if direction.dot(*axis) < *cos {
                        continue;
                    }
                    let Some((paint, cover)) = pin.paint(direction, images) else {
                        continue;
                    };
                    if cover <= 0.0 {
                        continue;
                    }
                    touched = true;
                    value = match pin.blend {
                        PinBlend::Add => value + paint * cover,
                        PinBlend::Over => value * (1.0 - cover) + paint * cover,
                    };
                }
                if !touched {
                    continue;
                }
                row[at] = value.r;
                if channels > 1 {
                    row[at + 1] = value.g;
                }
                if channels > 2 {
                    row[at + 2] = value.b;
                }
            }
        });
    out.color_space = ColorSpace::Raw;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    fn black(width: usize, height: usize) -> Image {
        Image {
            width,
            height,
            channels: 3,
            data: vec![0.0; width * height * 3],
            color_space: ColorSpace::Raw,
        }
    }

    fn texel(image: &Image, direction: Vec3) -> Spectrum {
        // Search rather than invert, so this reads the image the way the
        // renderer does and cannot agree with `composite` by sharing a bug in
        // the inverse mapping.
        let mut best = (f32::NEG_INFINITY, 0usize);
        for y in 0..image.height {
            let v = (y as f32 + 0.5) / image.height as f32;
            for x in 0..image.width {
                let u = (x as f32 + 0.5) / image.width as f32;
                let dot = equirect_direction(u, v).dot(direction.normalize());
                if dot > best.0 {
                    best = (dot, (y * image.width + x) * image.channels);
                }
            }
        }
        Spectrum {
            r: image.data[best.1],
            g: image.data[best.1 + 1],
            b: image.data[best.1 + 2],
        }
    }

    /// A pin lands where it was aimed, and nowhere else.
    #[test]
    fn a_pin_paints_where_it_points() {
        for direction in [
            Vec3::Y,
            -Vec3::Y,
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 0.0, -1.0),
            vec3(0.4, 0.6, -0.7).normalize(),
        ] {
            let pin = Pin {
                direction,
                intensity: 5.0,
                angular_size: Vec2 { x: 0.5, y: 0.5 },
                softness: 0.2,
                ..Default::default()
            };
            let lit = composite(&black(128, 64), std::slice::from_ref(&pin));

            let at_pin = texel(&lit, direction);
            assert!(
                at_pin.g > 4.0,
                "aimed at {direction:?} the pin reached only {}",
                at_pin.g
            );
            // ...and the opposite side of the sphere is untouched.
            let opposite = texel(&lit, -direction);
            assert_eq!(
                opposite,
                Spectrum::ZERO,
                "the pin lit the far side of the sphere too"
            );
        }
    }

    /// The pin reaches the renderer: it is bright where it points when the
    /// *environment* is asked, and the environment's importance sampling finds
    /// it.
    ///
    /// The tests above ask `coverage` and read the image back through the same
    /// mapping the compositor wrote it with, so a pin painted under one
    /// convention and read under another would agree with itself and be
    /// invisible. This asks `Environment`, which is the thing the integrator
    /// asks, and which indexes the image through its own code.
    ///
    /// The sampling half is the whole point of compositing into the image
    /// rather than adding a light type: §1.5 wants pins importance-sampled by
    /// the machinery that already exists, and a pin the CDF cannot find is a
    /// pin that only shows up as noise.
    #[test]
    fn a_pin_is_bright_and_findable_through_the_environment() {
        use crate::environment::Environment;
        use tequila_core::math::vec2;
        use std::sync::Arc;

        let aim = vec3(0.3, 0.8, -0.5).normalize();
        let pin = Pin {
            direction: aim,
            angular_size: Vec2 { x: 0.5, y: 0.5 },
            softness: 0.3,
            intensity: 30.0,
            ..Default::default()
        };
        // A dim sky under it, so the pin has to be found among something
        // rather than being the only non-zero texel in the image.
        let mut base = black(256, 128);
        base.data.fill(0.02);

        let environment = Environment::new(Arc::new(composite(&base, &[pin.clone()])), 1.0);

        let at_pin = environment.radiance(aim);
        let away = environment.radiance(-aim);
        assert!(
            at_pin.g > 10.0,
            "the environment reads {} where the pin points, so the compositor \
             and the environment disagree about where that is",
            at_pin.g
        );
        assert!(
            away.g < 0.1,
            "the environment reads {} on the far side of the sphere",
            away.g
        );

        // Importance sampling has to concentrate on it. A 0.5-radian pin
        // covers about 0.5% of the sphere; if the CDF had not found it, hits
        // would land there at about that rate.
        let mut inside = 0;
        let total = 4096;
        for i in 0..total {
            let u = vec2(
                (i as f32 + 0.5) / total as f32,
                ((i * 2654435761u64 as usize) % total) as f32 / total as f32,
            );
            if let Some(sample) = environment.sample(u) {
                if pin.coverage(sample.direction) > 0.0 {
                    inside += 1;
                }
            }
        }
        let fraction = inside as f32 / total as f32;
        assert!(
            fraction > 0.5,
            "only {fraction} of environment samples landed on the pin; the CDF \
             is not finding it, so it would render as noise rather than as a \
             light"
        );
    }

    /// The gesture's whole claim: aim at a point, and the light ends up where
    /// its reflection lands there.
    ///
    /// Checked by mirroring back rather than by restating the formula — the
    /// direction is right exactly when a viewer at the camera sees the pin
    /// reflected at the point they clicked, so the test reflects the answer
    /// about the normal again and requires it to point at the camera.
    #[test]
    fn a_pin_placed_for_a_highlight_reflects_back_to_the_camera() {
        let camera = vec3(0.0, 1.0, 4.0);
        for (point, normal) in [
            (vec3(0.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0)),
            (vec3(0.5, 0.4, 0.7), vec3(0.5, 0.4, 0.7)),
            (vec3(-0.6, 0.2, 0.75), vec3(-0.6, 0.2, 0.75)),
            (vec3(0.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)),
        ] {
            let pin = direction_for_highlight(point, normal, camera, 0.0)
                .expect("a point in front of the camera has a reflection");

            let n = normal.normalize();
            let back = (n * (2.0 * pin.dot(n)) - pin).normalize();
            let to_camera = (camera - point).normalize();
            assert!(
                (back - to_camera).length() < 1e-4,
                "a light at {pin:?} reflects to {back:?} at {point:?}, not back \
                 to the camera at {to_camera:?}"
            );
        }
    }

    /// Straight on, the light ends up behind the camera — which is where a
    /// ring flash is, and the right answer.
    #[test]
    fn a_head_on_highlight_puts_the_light_behind_the_camera() {
        let pin = direction_for_highlight(
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 0.0, 1.0),
            vec3(0.0, 0.0, 5.0),
            0.0,
        )
        .expect("reflects");
        assert!(
            (pin - vec3(0.0, 0.0, 1.0)).length() < 1e-5,
            "a head-on highlight put the light at {pin:?}"
        );
    }

    /// The environment's rotation has to be taken out, or the highlight lands
    /// off by exactly that angle.
    ///
    /// Pins live in the environment's space and the environment turns under
    /// the scene. Placing one from a world-space reflection while the
    /// environment is rotated looks like the click missing rather than like a
    /// forgotten transform, which is why it is worth a test of its own.
    #[test]
    fn the_environments_rotation_is_taken_out() {
        let point = vec3(0.3, 0.2, 0.9);
        let camera = vec3(0.0, 0.5, 4.0);
        let unrotated =
            direction_for_highlight(point, point, camera, 0.0).expect("reflects");

        for turn in [0.4f32, -1.2, std::f32::consts::PI] {
            let rotated =
                direction_for_highlight(point, point, camera, turn).expect("reflects");
            // Turning the environment by `turn` must move the pin by -turn in
            // the environment's own space, so that the two land in the same
            // place in the world.
            let (sin, cos) = (-turn).sin_cos();
            let expected = vec3(
                unrotated.x * cos + unrotated.z * sin,
                unrotated.y,
                -unrotated.x * sin + unrotated.z * cos,
            );
            assert!(
                (rotated - expected).length() < 1e-5,
                "with the environment turned {turn} the pin went to {rotated:?} \
                 rather than {expected:?}"
            );
            // ...and it really is a different direction, so this is not
            // passing because the rotation is being ignored on both sides.
            if turn.abs() > 0.1 {
                assert!(
                    (rotated - unrotated).length() > 0.05,
                    "rotating the environment did not move the pin at all"
                );
            }
        }
    }

    /// Degenerate input produces no placement rather than a NaN direction,
    /// which the command would refuse anyway but only after it had travelled
    /// through the UI.
    #[test]
    fn degenerate_placements_are_refused() {
        let at = vec3(1.0, 2.0, 3.0);
        assert!(direction_for_highlight(at, vec3(0.0, 1.0, 0.0), at, 0.0).is_none());
        assert!(
            direction_for_highlight(at, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 9.0), 0.0)
                .is_none()
        );
    }

    /// The pin has edges. Outside its angular size it contributes exactly
    /// nothing — not a small amount.
    ///
    /// A falloff with an infinite tail raises the entire sky by a little,
    /// which is the flat grey wash that makes a studio render look unlit, and
    /// it is invisible in a screenshot until you compare against a render
    /// without it.
    #[test]
    fn a_pin_stops_at_its_own_edge() {
        let pin = Pin {
            direction: Vec3::Y,
            angular_size: Vec2 { x: 0.4, y: 0.4 },
            softness: 1.0,
            intensity: 10.0,
            ..Default::default()
        };
        // Just inside the half-angle of 0.2 radians, and just outside it.
        let inside = vec3(0.19f32.sin(), 0.19f32.cos(), 0.0);
        let outside = vec3(0.21f32.sin(), 0.21f32.cos(), 0.0);
        assert!(pin.coverage(inside) > 0.0);
        assert_eq!(
            pin.coverage(outside),
            0.0,
            "the pin reaches past its own angular size"
        );
        // And far outside it is still exactly zero, not merely small.
        assert_eq!(pin.coverage(vec3(1.0, 0.0, 0.0)), 0.0);
    }

    /// Softness moves where the edge starts, and at zero the pin is flat to
    /// its border — the hard-edged panel a photographer uses for a crisp
    /// specular edge.
    #[test]
    fn softness_controls_the_falloff_and_nothing_else() {
        let hard = Pin {
            direction: Vec3::Y,
            angular_size: Vec2 { x: 1.0, y: 1.0 },
            softness: 0.0,
            ..Default::default()
        };
        let soft = Pin {
            softness: 0.9,
            ..hard.clone()
        };
        // Most of the way out, where the two must differ.
        let edge = vec3(0.45f32.sin(), 0.45f32.cos(), 0.0);
        assert!(
            (hard.coverage(edge) - 1.0).abs() < 1e-6,
            "a hard pin dimmed before its edge: {}",
            hard.coverage(edge)
        );
        assert!(
            soft.coverage(edge) < 0.5,
            "a soft pin did not fall off: {}",
            soft.coverage(edge)
        );
        // Both are still full brightness dead centre, so softness dims the
        // edge rather than the light.
        assert!((hard.coverage(Vec3::Y) - 1.0).abs() < 1e-6);
        assert!((soft.coverage(Vec3::Y) - 1.0).abs() < 1e-6);
    }

    /// A strip light is longer than it is wide, and turning its roll turns the
    /// strip rather than sliding it.
    #[test]
    fn a_strip_light_is_long_in_one_axis_and_rolls() {
        let strip = Pin {
            direction: Vec3::Y,
            angular_size: Vec2 { x: 1.2, y: 0.15 },
            softness: 0.1,
            roll: 0.0,
            ..Default::default()
        };
        // A point offset along the long axis is lit; the same offset along the
        // short axis is not.
        let (right, up) = tangents(Vec3::Y, 0.0);
        let long = (Vec3::Y + right * 0.45).normalize();
        let short = (Vec3::Y + up * 0.45).normalize();
        assert!(strip.coverage(long) > 0.0, "the strip is not long");
        assert_eq!(strip.coverage(short), 0.0, "the strip is not narrow");

        // A quarter turn swaps which of the two is lit.
        let rolled = Pin {
            roll: std::f32::consts::FRAC_PI_2,
            ..strip.clone()
        };
        assert_eq!(rolled.coverage(long), 0.0, "rolling did not turn the strip");
        assert!(rolled.coverage(short) > 0.0);

        // The centre stays put through the roll, or the pin is orbiting
        // rather than turning.
        assert!((rolled.coverage(Vec3::Y) - strip.coverage(Vec3::Y)).abs() < 1e-6);
    }

    /// A pin pointing straight up must not fall over a degenerate cross
    /// product — and straight up is where a key light usually is.
    #[test]
    fn a_pin_overhead_is_not_degenerate() {
        for direction in [Vec3::Y, -Vec3::Y] {
            let pin = Pin {
                direction,
                angular_size: Vec2 { x: 0.3, y: 0.9 },
                ..Default::default()
            };
            assert!(
                (pin.coverage(direction) - 1.0).abs() < 1e-6,
                "an overhead pin did not light its own centre"
            );
            let (right, up) = tangents(direction, 0.0);
            assert!(right.length().is_finite() && up.length().is_finite());
            assert!((right.dot(up)).abs() < 1e-5, "the tangents are not square");
            assert!(right.dot(direction).abs() < 1e-5);
        }
    }

    /// Pins add. Two overlapping softboxes are brighter where they cross,
    /// which is what two real lights do.
    #[test]
    fn overlapping_pins_add() {
        let one = Pin {
            direction: Vec3::Y,
            intensity: 2.0,
            softness: 0.0,
            angular_size: Vec2 { x: 1.0, y: 1.0 },
            ..Default::default()
        };
        let two = Pin {
            intensity: 3.0,
            ..one.clone()
        };
        let lit = composite(&black(64, 32), &[one, two]);
        let centre = texel(&lit, Vec3::Y);
        assert!(
            (centre.g - 5.0).abs() < 1e-4,
            "two pins of 2 and 3 gave {} rather than 5",
            centre.g
        );
    }

    /// Nothing enabled means nothing done: the image comes back as it went in.
    #[test]
    fn no_pins_changes_nothing() {
        let base = {
            let mut image = black(32, 16);
            for (i, value) in image.data.iter_mut().enumerate() {
                *value = (i % 7) as f32 * 0.1;
            }
            image
        };
        assert_eq!(composite(&base, &[]).data, base.data);
        let off = Pin {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(composite(&base, &[off]).data, base.data);
    }

    /// A pin adds to what is already there rather than replacing it.
    #[test]
    fn a_pin_adds_to_the_environment_under_it() {
        let mut base = black(64, 32);
        base.data.fill(0.25);
        let pin = Pin {
            direction: Vec3::Y,
            intensity: 1.0,
            softness: 0.0,
            angular_size: Vec2 { x: 0.8, y: 0.8 },
            ..Default::default()
        };
        let lit = composite(&base, &[pin]);
        let centre = texel(&lit, Vec3::Y);
        assert!(
            (centre.g - 1.25).abs() < 1e-4,
            "a pin over a 0.25 sky gave {} rather than 1.25",
            centre.g
        );
        // Away from the pin the sky is untouched.
        assert!((texel(&lit, vec3(1.0, 0.0, 0.0)).g - 0.25).abs() < 1e-6);
    }

    fn quad() -> Image {
        // Top row red, blue; bottom row green, white — each corner its own.
        Image {
            width: 2,
            height: 2,
            channels: 4,
            data: vec![
                1.0, 0.0, 0.0, 1.0, /**/ 0.0, 0.0, 1.0, 1.0, //
                0.0, 1.0, 0.0, 1.0, /**/ 1.0, 1.0, 1.0, 1.0,
            ],
            color_space: ColorSpace::Raw,
        }
    }

    fn with(path: &str, image: Image) -> PinImages {
        let mut images = PinImages::new();
        images.insert(path.into(), Arc::new(image));
        images
    }

    /// An image pin shows its picture the right way round as seen from the
    /// middle of the sphere: its top-left corner up and to the left.
    ///
    /// The pin's own `right` axis runs to the *viewer's* left when looking
    /// out at it, so the obvious mapping mirrors every picture — text in a
    /// window reads backwards, and nothing else about it looks wrong.
    #[test]
    fn an_image_pin_is_not_mirrored_or_upside_down() {
        let pin = Pin::image("Window", "quad.png", direction_from(0.0, 0.0), 0.6);
        let images = with("quad.png", quad());
        let corner = |bearing: f32, height: f32| {
            let (color, cover) = pin
                .paint(direction_from(bearing, height), &images)
                .expect("inside the pin");
            assert!(cover > 0.99, "an opaque picture covered only {cover}");
            color
        };
        // Bearing grows to the viewer's right; height upward.
        let top_left = corner(-0.2, 0.15);
        let top_right = corner(0.2, 0.15);
        let bottom_left = corner(-0.2, -0.15);
        assert!(top_left.r > 2.0 * top_left.g.max(top_left.b), "top left is {top_left:?}, not red");
        assert!(top_right.b > 2.0 * top_right.r.max(top_right.g), "top right is {top_right:?}, not blue");
        assert!(bottom_left.g > 2.0 * bottom_left.r.max(bottom_left.b), "bottom left is {bottom_left:?}, not green");
    }

    /// A picture's transparent parts leave what is under them alone, and a
    /// picture that did not load paints nothing rather than a black hole.
    #[test]
    fn an_image_pin_honours_alpha_and_absence() {
        let mut clear = quad();
        for texel in clear.data.chunks_mut(4) {
            texel[3] = 0.0;
        }
        let pin = Pin::image("Clear", "clear.png", Vec3::Y, 0.8);
        let base = Image::constant(64, 32, vec3(0.5, 0.5, 0.5));
        let out = composite_with(&base, std::slice::from_ref(&pin), &with("clear.png", clear), |_| true);
        assert_eq!(texel(&out, Vec3::Y), Spectrum::splat(0.5), "a transparent picture covered the sky");

        let out = composite_with(&base, std::slice::from_ref(&pin), &PinImages::new(), |_| true);
        assert_eq!(texel(&out, Vec3::Y), Spectrum::splat(0.5), "a missing picture painted something");

        // ...and an opaque one replaces it.
        let out = composite_with(&base, std::slice::from_ref(&pin), &with("clear.png", quad()), |_| true);
        assert_ne!(texel(&out, Vec3::Y), Spectrum::splat(0.5), "an opaque picture did not show");
    }

    /// A backdrop gradient covers the whole sphere, floor colour straight
    /// down to ceiling colour straight up, and replaces what was there.
    #[test]
    fn a_backdrop_gradient_runs_floor_to_ceiling() {
        let bottom = Spectrum::splat(0.1);
        let top = Spectrum { r: 0.4, g: 0.6, b: 0.8 };
        let pin = Pin::backdrop_gradient(bottom, top);
        let base = Image::constant(128, 64, vec3(5.0, 5.0, 5.0));
        let out = composite(&base, std::slice::from_ref(&pin));

        let close = |a: Spectrum, b: Spectrum| (a - b).max_component().abs() < 0.03 && (b - a).max_component() < 0.03;
        assert!(close(texel(&out, -Vec3::Y), bottom), "the floor is {:?}", texel(&out, -Vec3::Y));
        assert!(close(texel(&out, Vec3::Y), top), "the ceiling is {:?}", texel(&out, Vec3::Y));
        let middle = (bottom + top) * 0.5;
        for direction in [Vec3::X, -Vec3::Z, vec3(-0.7, 0.0, 0.7)] {
            let at = texel(&out, direction);
            assert!(close(at, middle), "at the horizon {direction:?} it is {at:?}, not halfway");
        }
        // Every texel was replaced: nothing of the bright base shows through.
        assert!(
            out.data.iter().all(|v| *v < 1.0),
            "some of the environment showed through the backdrop"
        );
    }

    /// A linear gradient runs bottom to top across its own footprint, and
    /// stops at the footprint's edge.
    #[test]
    fn a_linear_gradient_runs_up_its_footprint() {
        let pin = Pin {
            kind: PinKind::Gradient {
                stops: vec![
                    GradientStop { t: 0.0, color: Spectrum::splat(0.0) },
                    GradientStop { t: 1.0, color: Spectrum::splat(1.0) },
                ],
                radial: false,
            },
            direction: direction_from(0.0, 0.0),
            angular_size: Vec2 { x: 0.8, y: 0.8 },
            softness: 0.0,
            intensity: 1.0,
            ..Default::default()
        };
        let images = PinImages::new();
        let at = |height: f32| pin.paint(direction_from(0.0, height), &images).map(|(c, _)| c.g);
        let (low, mid, high) = (at(-0.35).unwrap(), at(0.0).unwrap(), at(0.35).unwrap());
        assert!(low < 0.1 && (mid - 0.5).abs() < 0.02 && high > 0.9, "{low} {mid} {high}");
        assert!(at(0.5).is_none(), "the gradient ran past its footprint");
    }

    #[test]
    fn gradient_stops_in_any_order_interpolate_and_hold_their_ends() {
        let stops = [
            GradientStop { t: 0.75, color: Spectrum::splat(1.0) },
            GradientStop { t: 0.25, color: Spectrum::splat(0.0) },
        ];
        assert_eq!(gradient_at(&stops, 0.0).g, 0.0);
        assert!((gradient_at(&stops, 0.5).g - 0.5).abs() < 1e-6);
        assert_eq!(gradient_at(&stops, 1.0).g, 1.0);
        assert_eq!(gradient_at(&[], 0.5), Spectrum::ZERO);
    }

    /// Order matters once a pin can cover: a light under a backdrop is
    /// hidden by it, and the same light over it shows.
    #[test]
    fn pins_composite_in_order() {
        let light = Pin {
            direction: Vec3::Y,
            intensity: 10.0,
            softness: 0.0,
            ..Default::default()
        };
        let backdrop = Pin::backdrop_gradient(Spectrum::splat(0.2), Spectrum::splat(0.2));
        let base = black(128, 64);
        let under = composite(&base, &[light.clone(), backdrop.clone()]);
        let over = composite(&base, &[backdrop, light]);
        assert!((texel(&under, Vec3::Y).g - 0.2).abs() < 1e-4, "the backdrop did not cover the light under it");
        assert!((texel(&over, Vec3::Y).g - 10.2).abs() < 1e-3, "the light over the backdrop did not add");
    }
}
