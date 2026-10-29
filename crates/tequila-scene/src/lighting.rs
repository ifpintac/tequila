//! Lights and cameras the document owns (`docs/04-UX-SPEC.md` §4, §8).
//!
//! A light is a node in the scene tree, not a thing off to one side. That is
//! not a filing decision — it is what makes a light selectable, movable with the
//! gizmo, groupable, hideable, lockable, undoable and saveable without any of
//! those features being written a second time for lights. A renderer that keeps
//! its lights in a separate list ends up reimplementing half the scene tree
//! around them, badly.
//!
//! What the renderer needs is different: a flat, prepared
//! [`LightSet`](tequila_render_core::light::LightSet) with selection
//! probabilities already computed. The snapshot builds that from the tree, the
//! same way it builds instances from parts.
//!
//! ## Cameras are a list, not tree nodes
//!
//! Cameras go the other way, and for a reason worth stating since it is the
//! opposite choice. A camera is not *in* the scene — it is a way of looking at
//! it. Parenting one to a part would be useful for a mounted shot, and that is
//! M5's job with animation; until then, putting cameras in the tree would mean
//! they show up in every selection, every bounding box, and every "select all",
//! which is noise for something the user thinks of as a saved viewpoint.

use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
pub use tequila_color::blackbody_rgb;
use tequila_render_core::camera::{Aperture, Camera, Projection};
use tequila_render_core::ies::IesProfile;
use tequila_render_core::light::{
    AreaLight, DistantLight, IesLight, Light, LightKind, PointLight, SpotLight, Visibility,
};
use std::sync::Arc;

/// What kind of light a light node is.
///
/// Parameterised the way the UI presents them rather than the way the sampler
/// wants them: a size in scene units and a power in lumens, not half-extent
/// vectors and radiance. The conversion happens once, where the snapshot is
/// built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightShape {
    /// A rectangular or elliptical panel.
    Area {
        width: f32,
        height: f32,
        disc: bool,
        two_sided: bool,
    },
    /// A point, or a small sphere when `radius` is above zero.
    Point { radius: f32 },
    /// A cone, with a soft edge between the two angles.
    Spot {
        inner_degrees: f32,
        outer_degrees: f32,
        falloff: f32,
    },
    /// The sun: light from far away along the node's -Z, every ray parallel
    /// but for the disc's own angle across, which softens its shadows - the
    /// real sun's is about half a degree. Where it is does not matter, only
    /// which way it points.
    Sun { angle_degrees: f32 },
}

/// A light shape, with the photometric file an IES light needs.
///
/// Separate from [`LightShape`] because a `.ies` path is a `String` and
/// [`LightShape`] is `Copy` — every light in the document carries one, and the
/// shapes that do not need a path should not pay for one.
#[derive(Clone, Debug, PartialEq)]
pub struct Photometry {
    /// Path to a `.ies` file. Empty means the light uses its analytic shape.
    pub path: String,
}

impl LightShape {
    pub fn label(&self) -> &'static str {
        match self {
            LightShape::Area { disc: false, .. } => "Area",
            LightShape::Area { disc: true, .. } => "Disc",
            LightShape::Point { .. } => "Point",
            LightShape::Spot { .. } => "Spot",
            LightShape::Sun { .. } => "Sun",
        }
    }
}

/// How a light's brightness is specified.
///
/// §7.5 asks for physical units, and the reason a choice exists at all is that
/// photographers and engineers reach for different ones: a softbox is bought by
/// its power in watts or lumens, while a screen or a lightbox is specified by
/// its luminance in nits. Converting between them needs the emitter's area,
/// which the shape already knows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intensity {
    /// Total luminous flux, in lumens.
    Lumens(f32),
    /// Luminance, in nits (candela per square metre).
    Nits(f32),
    /// Luminous intensity, in candela. The natural unit for a point source.
    Candela(f32),
    /// Illuminance, in lux: what falls on a surface facing the light. The
    /// natural unit for the sun, which has no size or distance to speak of.
    Lux(f32),
}

impl Intensity {
    pub fn value(&self) -> f32 {
        match self {
            Intensity::Lumens(v) | Intensity::Nits(v) | Intensity::Candela(v) | Intensity::Lux(v) => *v,
        }
    }

    pub fn with_value(&self, value: f32) -> Intensity {
        match self {
            Intensity::Lumens(_) => Intensity::Lumens(value),
            Intensity::Nits(_) => Intensity::Nits(value),
            Intensity::Candela(_) => Intensity::Candela(value),
            Intensity::Lux(_) => Intensity::Lux(value),
        }
    }

    pub fn unit(&self) -> &'static str {
        match self {
            Intensity::Lumens(_) => "lm",
            Intensity::Nits(_) => "nits",
            Intensity::Candela(_) => "cd",
            Intensity::Lux(_) => "lx",
        }
    }

    /// Radiance leaving an emitter of the given area, in nits.
    ///
    /// A Lambertian emitter of area `A` and radiance `L` emits `L · A · π`
    /// lumens — the π is the integral of the cosine over the hemisphere, and
    /// leaving it out is the classic way to get a light that is π times too
    /// bright and to compensate by tuning every other number in the scene.
    pub fn radiance_for_area(&self, area: f32) -> f32 {
        let area = area.max(1e-6);
        match self {
            Intensity::Nits(v) => *v,
            Intensity::Lumens(v) => v / (area * std::f32::consts::PI),
            // Candela is intensity per steradian; over a Lambertian emitter's
            // projected area that is radiance directly.
            Intensity::Candela(v) => v / area,
            // Light leaving a Lambertian surface, its exitance: π times its
            // radiance.
            Intensity::Lux(v) => v / std::f32::consts::PI,
        }
    }

    /// A multiplier on a distribution that is already in physical units.
    ///
    /// What an IES light needs: the file supplies candela, so the user's number
    /// is a scale. A value in lumens or nits has no meaning against a measured
    /// web, so both are read as "times one" at their nominal value — which
    /// keeps switching a light to a photometric file from changing its
    /// brightness by four orders of magnitude.
    pub fn scale(&self) -> f32 {
        match self {
            Intensity::Candela(v) => *v,
            Intensity::Lumens(v) => v / 1000.0,
            Intensity::Nits(v) => v / 1000.0,
            Intensity::Lux(v) => v / 1000.0,
        }
    }

    /// Luminous intensity in candela, for a point source.
    pub fn candela(&self) -> f32 {
        match self {
            Intensity::Candela(v) => *v,
            // Over the whole sphere.
            Intensity::Lumens(v) => v / (4.0 * std::f32::consts::PI),
            // A bare number with no area to scale it; treated as candela so
            // the light is not silently black. Lux the same.
            Intensity::Nits(v) | Intensity::Lux(v) => *v,
        }
    }

    /// Illuminance in lux, for the sun. Any other unit, which a sun has no
    /// size or distance to turn into lux, at its nominal value.
    pub fn lux(&self) -> f32 {
        self.value()
    }
}

/// A light, as the document stores it.
///
/// Its *position* is the node's transform — a light is moved with the same
/// gizmo as everything else.
#[derive(Clone, Debug, PartialEq)]
pub struct LightSettings {
    pub shape: LightShape,
    /// A measured distribution to use instead of the shape's analytic one
    /// (§5.1). `None` is the analytic shape.
    ///
    /// The *path*, not the profile: a document holds descriptions, and a
    /// photometric file is a manufacturer's, shared between projects and
    /// updated independently of them.
    pub photometry: Option<Photometry>,
    pub color: Spectrum,
    /// Colour temperature in kelvin, when the colour is specified that way.
    /// `None` means the colour above is used directly.
    pub temperature: Option<f32>,
    pub intensity: Intensity,
    pub visibility: Visibility,
    pub layer: u8,
    pub enabled: bool,
    /// The parts it is kept to (light linking); `None` lights everything.
    pub links: Option<LightLinking>,
    /// The parts that cast its shadow (shadow linking), in the same shape:
    /// those named, or all but those. `None`: every part it lights.
    pub shadow_links: Option<LightLinking>,
    /// An area or disc light's spread, in degrees: 180 is a plain panel,
    /// less a softbox behind a grid, its light gathered into a beam that
    /// wide. Its power stays what it was.
    pub spread_degrees: f32,
}

/// What a light is kept to (light linking): the parts it lights, or the
/// parts it leaves out. A group stands for everything under it, and what
/// a light does not light it does not shadow either. The same shape says
/// which parts cast a light's shadow (shadow linking).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightLinking {
    /// Leave these out, rather than light only these.
    pub exclude: bool,
    /// Parts and groups, by node.
    pub nodes: Vec<crate::tree::NodeId>,
    /// The product - everything in the scene but its lights - counts among
    /// them. What a studio preset can say, since it cannot name a
    /// product's parts.
    pub product: bool,
}

impl Default for LightSettings {
    fn default() -> Self {
        LightSettings {
            shape: LightShape::Area {
                width: 1.0,
                height: 1.0,
                disc: false,
                two_sided: false,
            },
            photometry: None,
            color: Spectrum::ONE,
            temperature: None,
            intensity: Intensity::Lumens(1500.0),
            visibility: Visibility::default(),
            layer: 0,
            enabled: true,
            links: None,
            shadow_links: None,
            spread_degrees: 180.0,
        }
    }
}

impl LightSettings {
    /// The emitter's area in scene units, for the unit conversions.
    pub fn area(&self) -> f32 {
        match self.shape {
            LightShape::Area {
                width,
                height,
                disc,
                ..
            } => {
                let rect = width * height;
                if disc {
                    rect * std::f32::consts::PI / 4.0
                } else {
                    rect
                }
            }
            // A point has no area; the conversions treat it as a point source.
            LightShape::Point { .. } | LightShape::Spot { .. } | LightShape::Sun { .. } => 1.0,
        }
    }

    /// Half-extents of the emitter in its own local space, for framing and
    /// for the viewport's wireframe.
    pub fn emitter_extent(&self) -> Vec3 {
        match self.shape {
            LightShape::Area { width, height, .. } => {
                vec3(width * 0.5, height * 0.5, 0.0)
            }
            LightShape::Point { radius } => Vec3::splat(radius.max(1e-3)),
            // A cone has no size of its own, nor a sun; a small marker is
            // what there is to frame.
            LightShape::Spot { .. } | LightShape::Sun { .. } => Vec3::splat(1e-2),
        }
    }

    /// The renderer's form of this light, placed by `world`.
    ///
    /// The whole unit conversion — lumens to radiance, degrees to radians,
    /// size in scene units to half-extent vectors — happens here and only
    /// here, so the sampler never sees a unit and the UI never sees a
    /// half-extent.
    ///
    /// An area light's panel spans local X and Y and emits along local
    /// **+Z**, the cross of its width and its height, so a light with no
    /// transform shines along world +Z, towards the Front view's camera.
    /// That is the opposite of Blender's area lights: a matrix taken from
    /// Blender needs turning half a turn about its X axis. A spot aims along
    /// local **-Z**, as does a photometric file on a point or a spot; on an
    /// area light the file aims where the panel faces, +Z.
    pub fn to_render_light(&self, world: &Transform) -> Option<Light> {
        self.to_render_light_with(world, None)
    }

    /// The same, with a photometric profile already loaded.
    ///
    /// The profile is passed in rather than read here because reading it is a
    /// file access and this runs on every snapshot — which is every frame of a
    /// gizmo drag. The snapshot's cache owns the loading.
    pub fn to_render_light_with(
        &self,
        world: &Transform,
        profile: Option<Arc<IesProfile>>,
    ) -> Option<Light> {
        if !self.enabled {
            return None;
        }
        let position = world.point(Vec3::ZERO);
        let color = self.emitted_color();

        // A measured distribution replaces the shape's own, keeping the
        // placement and the intensity the user set. It is deliberately not a
        // fifth shape: a luminaire with a photometric file is still a point in
        // space with an aiming axis, and the file only says how brightly it
        // emits in each direction. A sun is not a fitting, and keeps its own.
        if let Some(profile) = profile.filter(|_| !matches!(self.shape, LightShape::Sun { .. })) {
            // Aimed where the shape points, as the Lighting tab promises: a
            // panel faces +Z, so a file on one aims there - the luminaire
            // turned over about its width, keeping `right`, so an asymmetric
            // file is turned rather than mirrored. Aiming every file along -Z
            // turned a panel round the moment it was given one.
            let aim = match self.shape {
                LightShape::Area { .. } => Vec3::Z,
                LightShape::Point { .. } | LightShape::Spot { .. } | LightShape::Sun { .. } => -Vec3::Z,
            };
            let direction = world.vector(aim).try_normalize().unwrap_or(aim);
            let right = world.vector(Vec3::X).try_normalize().unwrap_or(Vec3::X);
            return Some(
                Light::new(LightKind::Ies(IesLight {
                    position,
                    direction,
                    right,
                    // The profile is already in candela, so the user's
                    // intensity is a *scale* on it rather than a replacement —
                    // a manufacturer's file at 1.0 is the fitting as measured.
                    intensity: color * self.intensity.scale(),
                    profile,
                }))
                .with_visibility(self.visibility)
                .with_layer(self.layer),
            );
        }

        let kind = match self.shape {
            LightShape::Area {
                width,
                height,
                disc,
                two_sided,
            } => {
                // The half-extents carry the transform's scale, so scaling a
                // light node in the gizmo resizes the emitter — which is what
                // a user dragging a scale handle on a softbox means.
                let half_u = world.vector(vec3(width * 0.5, 0.0, 0.0));
                let half_v = world.vector(vec3(0.0, height * 0.5, 0.0));
                // Radiance from the *transformed* area, not the nominal one:
                // a light scaled to twice its size emits the same lumens over
                // four times the area, which is what "1500 lm" has to keep
                // meaning after a scale.
                let mut light = AreaLight {
                    position,
                    half_u,
                    half_v,
                    radiance: Spectrum::ZERO,
                    disc,
                    two_sided,
                    spread: self.spread_degrees.clamp(1.0, 180.0).to_radians(),
                };
                light.radiance = color * self.intensity.radiance_for_area(light.area());
                LightKind::Area(light)
            }
            LightShape::Point { radius } => LightKind::Point(PointLight {
                position,
                intensity: color * self.intensity.candela(),
                radius: radius * world.vector(Vec3::X).length(),
            }),
            LightShape::Spot {
                inner_degrees,
                outer_degrees,
                falloff,
            } => {
                let direction = world
                    .vector(-Vec3::Z)
                    .try_normalize()
                    .unwrap_or(-Vec3::Z);
                let outer = outer_degrees.to_radians();
                LightKind::Spot(SpotLight {
                    position,
                    direction,
                    intensity: color * self.intensity.candela(),
                    // Clamped rather than swapped: a user dragging the inner
                    // angle past the outer means "fill the cone", and swapping
                    // the two would make the outer angle jump instead.
                    inner_angle: inner_degrees.to_radians().min(outer),
                    outer_angle: outer,
                    falloff,
                })
            }
            LightShape::Sun { angle_degrees } => LightKind::Distant(DistantLight {
                direction: world.vector(-Vec3::Z).try_normalize().unwrap_or(-Vec3::Z),
                irradiance: color * self.intensity.lux(),
                half_angle: (angle_degrees.clamp(0.0, 90.0) * 0.5).to_radians(),
            }),
        };
        Some(
            Light::new(kind)
                .with_visibility(self.visibility)
                .with_layer(self.layer),
        )
    }

    /// The emitted colour, after any colour-temperature conversion.
    pub fn emitted_color(&self) -> Spectrum {
        match self.temperature {
            Some(kelvin) => self.color * blackbody_rgb(kelvin),
            None => self.color,
        }
    }
}


/// A saved viewpoint (`docs/04` §8).
#[derive(Clone, Debug, PartialEq)]
pub struct SavedCamera {
    pub name: String,
    pub camera: Camera,
    /// What the camera orbits and focuses on, kept so a restored view can be
    /// orbited rather than only looked through.
    pub target: Vec3,
}

/// The document's cameras.
#[derive(Clone, Debug, PartialEq)]
pub struct CameraStore {
    cameras: Vec<SavedCamera>,
    active: usize,
}

impl Default for CameraStore {
    fn default() -> Self {
        CameraStore::new()
    }
}

impl CameraStore {
    pub fn new() -> CameraStore {
        CameraStore {
            cameras: vec![SavedCamera {
                name: "Camera 1".into(),
                camera: Camera::default(),
                target: Vec3::ZERO,
            }],
            active: 0,
        }
    }

    /// Rebuild a store from a saved list.
    ///
    /// An empty list gives the default store rather than an empty one: the
    /// invariant that there is always an active camera is what lets `active()`
    /// return a reference instead of an option, and a file with no cameras in
    /// it must not be able to break that.
    pub fn from_cameras(cameras: Vec<SavedCamera>) -> CameraStore {
        if cameras.is_empty() {
            return CameraStore::new();
        }
        CameraStore { cameras, active: 0 }
    }

    pub fn len(&self) -> usize {
        self.cameras.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cameras.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (usize, &SavedCamera)> {
        self.cameras.iter().enumerate()
    }

    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> &SavedCamera {
        &self.cameras[self.active.min(self.cameras.len() - 1)]
    }

    pub fn active_mut(&mut self) -> &mut SavedCamera {
        let index = self.active.min(self.cameras.len() - 1);
        &mut self.cameras[index]
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut SavedCamera> {
        self.cameras.get_mut(index)
    }

    pub fn get(&self, index: usize) -> Option<&SavedCamera> {
        self.cameras.get(index)
    }

    /// The first saved camera of this name.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.iter().find(|(_, c)| c.name == name).map(|(i, _)| i)
    }

    pub fn select(&mut self, index: usize) {
        if index < self.cameras.len() {
            self.active = index;
        }
    }

    /// Add a camera, named so it does not collide with an existing one.
    pub fn add(&mut self, camera: Camera, target: Vec3) -> usize {
        let name = self.next_name();
        self.cameras.push(SavedCamera {
            name,
            camera,
            target,
        });
        self.cameras.len() - 1
    }

    pub fn insert(&mut self, index: usize, saved: SavedCamera) {
        let index = index.min(self.cameras.len());
        self.cameras.insert(index, saved);
        if self.active >= index {
            self.active = (self.active + 1).min(self.cameras.len() - 1);
        }
    }

    /// Remove a camera. The last one cannot be removed: a scene with no camera
    /// has nothing to render from, and the UI would have to grow a special
    /// empty state for a situation nobody wants to be in.
    pub fn remove(&mut self, index: usize) -> Option<SavedCamera> {
        if self.cameras.len() <= 1 || index >= self.cameras.len() {
            return None;
        }
        let removed = self.cameras.remove(index);
        // A camera above the active one moves it up a place; it is still the
        // active one, as `insert` keeps it.
        if index < self.active {
            self.active -= 1;
        } else if self.active >= self.cameras.len() {
            self.active = self.cameras.len() - 1;
        }
        Some(removed)
    }

    fn next_name(&self) -> String {
        for n in 1.. {
            let candidate = format!("Camera {n}");
            if !self.cameras.iter().any(|c| c.name == candidate) {
                return candidate;
            }
        }
        unreachable!("the loop is unbounded")
    }
}

/// What a three-point fit recovers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchedPerspective {
    pub focal_length_mm: f32,
    /// The principal point, as a film-plane shift in fractions of the sensor.
    /// Zero is a camera whose axis passes through the centre of the frame.
    pub shift_x: f32,
    pub shift_y: f32,
}

/// Match Perspective from **three** vanishing points (§1.7).
///
/// Three points are more than the two-point case needs, and what the extra one
/// buys is the *principal point*: a photograph taken with a tilted camera, or
/// cropped off-centre, has its optical axis somewhere other than the middle of
/// the frame, and a render matched with the axis forced to the centre lines up
/// in the middle and diverges at the edges — which is exactly where a
/// composite is looked at.
///
/// The geometry is a classical result and worth stating because it is the whole
/// function. Three vanishing points of mutually perpendicular directions form a
/// triangle whose **orthocentre is the principal point**, and the focal length
/// is then `sqrt(-(v1 - p) · (v2 - p))` for any pair. Both fall out of the same
/// fact the two-point case uses — that perpendicular directions have vanishing
/// points whose rays from the pinhole are perpendicular.
pub fn match_perspective_three_point(
    a: (f32, f32),
    b: (f32, f32),
    c: (f32, f32),
    sensor_width_mm: f32,
    aspect: f32,
) -> Option<MatchedPerspective> {
    // Into a square coordinate system about the frame's centre, where
    // perpendicularity is perpendicularity.
    let to_square = |p: (f32, f32)| (p.0 - 0.5, (p.1 - 0.5) / aspect.max(1e-6));
    let (ax, ay) = to_square(a);
    let (bx, by) = to_square(b);
    let (cx, cy) = to_square(c);

    // The orthocentre, from two altitudes. An altitude through `a` is
    // perpendicular to `bc`, so its direction is `bc` and the point satisfies
    // `(p - a) · (c - b) = 0`.
    let (ux, uy) = (cx - bx, cy - by);
    let (vx, vy) = (cx - ax, cy - ay);
    let determinant = ux * vy - uy * vx;
    if determinant.abs() < 1e-9 {
        // The three points are collinear, which no real camera produces: it
        // would mean three perpendicular directions sharing a horizon.
        return None;
    }
    let ra = ax * ux + ay * uy;
    let rb = bx * vx + by * vy;
    let px = (ra * vy - rb * uy) / determinant;
    let py = (rb * ux - ra * vx) / determinant;

    // The focal length, in the same square units. Averaged over the three
    // pairs rather than taken from one: they agree exactly only for points that
    // are exactly right, and a user placing them by eye deserves the fit rather
    // than whichever pair happened to be listed first.
    let pair = |p: (f32, f32), q: (f32, f32)| -> f32 {
        -((p.0 - px) * (q.0 - px) + (p.1 - py) * (q.1 - py))
    };
    let squared = (pair((ax, ay), (bx, by)) + pair((bx, by), (cx, cy)) + pair((cx, cy), (ax, ay)))
        / 3.0;
    if !(squared > 0.0) {
        // A negative mean says the points do not describe perpendicular
        // directions seen through a pinhole at all.
        return None;
    }
    let focal_mm = squared.sqrt() * sensor_width_mm;
    if !(focal_mm.is_finite() && focal_mm > 1.0 && focal_mm < 2000.0) {
        return None;
    }

    Some(MatchedPerspective {
        focal_length_mm: focal_mm,
        // Back out of the square system, in the sense `Projection::Shift`
        // uses: the ray generator places the optical axis at
        // `(0.5 - shift_x, 0.5 + shift_y)` on the film, so the shift is the
        // principal point's offset from centre, negated in x and not in y.
        // The two signs differing is not a slip — the film's y runs downwards
        // and the camera's up-axis does not.
        shift_x: -px,
        shift_y: py * aspect,
    })
}

/// §8.2's Match Perspective: recover a camera from vanishing points.
///
/// Given two vanishing points of a rectilinear subject in the image, the focal
/// length and orientation that would produce them are determined — which is what
/// lets a render be dropped onto a photographed backplate and line up.
///
/// The two-point case assumes the vertical axis stays vertical in the image,
/// which is what a level camera does and what the control is for.
pub fn match_perspective_two_point(
    vanishing_a: (f32, f32),
    vanishing_b: (f32, f32),
    sensor_width_mm: f32,
    aspect: f32,
) -> Option<f32> {
    // In normalized image coordinates about the centre, two vanishing points of
    // perpendicular directions satisfy `va · vb + f² = 0` once both are
    // expressed as rays through the principal point — so the focal length is
    // the geometric mean of the two offsets.
    let ax = vanishing_a.0;
    let ay = vanishing_a.1 / aspect;
    let bx = vanishing_b.0;
    let by = vanishing_b.1 / aspect;

    let dot = ax * bx + ay * by;
    if dot >= -1e-6 {
        // The points are on the same side of the centre, so no real camera
        // produced them. Refusing is better than returning a focal length that
        // looks plausible and is not.
        return None;
    }
    let focal_normalized = (-dot).sqrt();
    let focal_mm = focal_normalized * sensor_width_mm;
    (focal_mm.is_finite() && focal_mm > 1.0 && focal_mm < 2000.0).then_some(focal_mm)
}

/// The lens types §8.1 lists, for a dropdown.
pub fn lens_types() -> [(&'static str, Projection); 4] {
    [
        (
            "Perspective",
            Projection::Perspective {
                focal_length_mm: 50.0,
            },
        ),
        ("Orthographic", Projection::Orthographic { scale: 1.0 }),
        (
            "Shift",
            Projection::Shift {
                focal_length_mm: 50.0,
                shift_x: 0.0,
                shift_y: 0.0,
            },
        ),
        ("Spherical", Projection::Equirectangular),
    ]
}

/// The aperture shapes §8.1 offers.
pub fn aperture_shapes() -> [(&'static str, Aperture); 2] {
    [("Circular", Aperture::Circular), ("Bladed", Aperture::Bladed { blades: 6, rotation: 0.0 })]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §7.5's physical units: a light specified in lumens must emit that many
    /// lumens, whatever its size.
    ///
    /// The π is the thing this pins. A Lambertian emitter of radiance `L` and
    /// area `A` puts out `L·A·π` lumens; dropping the π gives a light π times
    /// too bright, which people then compensate for by detuning every other
    /// number in the scene.
    #[test]
    fn a_light_in_lumens_emits_that_many_lumens() {
        for area in [0.25f32, 1.0, 4.0] {
            let intensity = Intensity::Lumens(1000.0);
            let radiance = intensity.radiance_for_area(area);
            let emitted = radiance * area * std::f32::consts::PI;
            assert!(
                (emitted - 1000.0).abs() < 0.1,
                "an area of {area} emitted {emitted:.1} lumens rather than 1000"
            );
        }
    }

    /// Doubling a light's area at fixed lumens halves its radiance: the same
    /// light spread over more surface, which is what a bigger softbox is.
    #[test]
    fn spreading_a_fixed_output_over_more_area_dims_it() {
        let intensity = Intensity::Lumens(1000.0);
        let small = intensity.radiance_for_area(1.0);
        let large = intensity.radiance_for_area(2.0);
        assert!((small / large - 2.0).abs() < 1e-3);
    }

    /// Nits are radiance directly, so they do not change with area — that is
    /// the difference between the two units and the reason both exist.
    #[test]
    fn nits_do_not_depend_on_area() {
        let intensity = Intensity::Nits(500.0);
        assert_eq!(intensity.radiance_for_area(1.0), 500.0);
        assert_eq!(intensity.radiance_for_area(9.0), 500.0);
    }

    /// Colour temperature changes the colour and not the exposure.
    ///
    /// Without the normalization, warming a light also dims it, and the two
    /// controls become impossible to separate by eye — every adjustment to
    /// warmth needs a compensating adjustment to brightness.
    #[test]
    fn colour_temperature_does_not_change_brightness() {
        for kelvin in [2000.0f32, 3200.0, 5600.0, 6500.0, 10000.0] {
            let colour = blackbody_rgb(kelvin);
            assert!(
                (colour.luminance() - 1.0).abs() < 1e-3,
                "{kelvin} K has luminance {}",
                colour.luminance()
            );
        }
    }

    /// And it changes the colour in the direction everyone expects: low is
    /// warm, high is cool.
    #[test]
    fn low_temperatures_are_warm_and_high_ones_cool() {
        let tungsten = blackbody_rgb(2700.0);
        let daylight = blackbody_rgb(6500.0);
        let overcast = blackbody_rgb(9000.0);

        assert!(
            tungsten.r > tungsten.b * 2.0,
            "2700 K should be markedly warm: {tungsten:?}"
        );
        assert!(
            overcast.b > overcast.r,
            "9000 K should be cool: {overcast:?}"
        );
        assert!(
            tungsten.r / tungsten.b > daylight.r / daylight.b,
            "warmth should fall as temperature rises"
        );
    }

    /// A scene always has a camera to render from.
    #[test]
    fn the_last_camera_cannot_be_removed() {
        let mut store = CameraStore::new();
        assert_eq!(store.len(), 1);
        assert!(store.remove(0).is_none(), "the only camera was removed");

        store.add(Camera::default(), Vec3::ZERO);
        assert_eq!(store.len(), 2);
        assert!(store.remove(0).is_some());
        assert_eq!(store.len(), 1);
    }

    /// Removing the active camera leaves a valid one active rather than an
    /// index past the end.
    #[test]
    fn removing_the_active_camera_keeps_one_active() {
        let mut store = CameraStore::new();
        store.add(Camera::default(), Vec3::ZERO);
        store.add(Camera::default(), Vec3::ZERO);
        store.select(2);

        store.remove(2);
        assert!(store.active_index() < store.len());
        let _ = store.active();
    }

    /// Removing a camera listed above the active one leaves the same camera
    /// active, not the one after it.
    #[test]
    fn removing_a_camera_above_the_active_one_keeps_it_active() {
        let mut store = CameraStore::new();
        store.add(Camera::default(), Vec3::ZERO);
        store.add(Camera::default(), Vec3::ZERO);
        store.select(1);
        let active = store.active().name.clone();
        store.remove(0);
        assert_eq!(store.active().name, active);
    }

    /// Round trip: project a known camera's three axis directions to their
    /// vanishing points, fit from those, and recover the camera.
    ///
    /// A synthetic round trip is worth far more here than a hand-checked
    /// example, because the failure this guards against is a sign or an axis
    /// convention — and those produce numbers that look entirely reasonable.
    #[test]
    fn a_three_point_fit_recovers_the_camera_that_made_it() {
        let aspect = 3.0 / 2.0;
        for (focal, shift_x, shift_y) in [
            (35.0f32, 0.0f32, 0.0f32),
            (85.0, 0.12, -0.08),
            (24.0, -0.2, 0.15),
        ] {
            // A camera looking down a direction that sees all three axes.
            let forward = vec3(-0.5, -0.35, -0.79).try_normalize().unwrap();
            let right = forward.cross(Vec3::Y).try_normalize().unwrap();
            let up = right.cross(forward);

            // The vanishing point of a world direction is the film point whose
            // ray is parallel to it. Derived from `Camera::generate_ray` rather
            // than from a remembered formula: the whole value of this test is
            // that it agrees with the code that actually makes the picture.
            let vanishing = |direction: Vec3| -> Option<(f32, f32)> {
                let z = -direction.dot(forward);
                if z.abs() < 1e-6 {
                    return None;
                }
                let scale = focal / 36.0;
                let x = direction.dot(right) / z;
                let y = direction.dot(up) / z;
                Some((
                    0.5 - x * scale - shift_x,
                    0.5 + y * scale * aspect + shift_y,
                ))
            };

            let a = vanishing(Vec3::X).expect("X has a vanishing point");
            let b = vanishing(Vec3::Y).expect("Y has a vanishing point");
            let c = vanishing(Vec3::Z).expect("Z has a vanishing point");

            let fitted = match_perspective_three_point(a, b, c, 36.0, aspect)
                .expect("three perpendicular directions describe a camera");
            assert!(
                (fitted.focal_length_mm - focal).abs() < focal * 0.02,
                "focal length came back as {} rather than {focal}",
                fitted.focal_length_mm
            );
            assert!(
                (fitted.shift_x - shift_x).abs() < 0.02
                    && (fitted.shift_y - shift_y).abs() < 0.02,
                "the shift came back as ({}, {}) rather than ({shift_x}, {shift_y})",
                fitted.shift_x,
                fitted.shift_y
            );
        }
    }

    /// Three points on a line cannot come from three perpendicular directions,
    /// and a fit that returned something anyway would be worse than one that
    /// refuses: the number would look like a lens.
    #[test]
    fn three_collinear_points_are_refused() {
        assert!(
            match_perspective_three_point((0.0, 0.5), (0.5, 0.5), (1.0, 0.5), 36.0, 1.5).is_none()
        );
    }

    /// Three vanishing points of perpendicular directions always form an
    /// *acute* triangle — that is what having a real focal length means. An
    /// obtuse one puts the orthocentre outside it and the focal length becomes
    /// imaginary, and returning a number anyway would hand the user a lens that
    /// cannot exist.
    #[test]
    fn an_impossible_triangle_is_refused() {
        assert!(
            match_perspective_three_point((0.02, 0.5), (0.98, 0.5), (0.5, 0.52), 36.0, 1.0)
                .is_none()
        );
    }

    #[test]
    fn camera_names_do_not_collide() {
        let mut store = CameraStore::new();
        for _ in 0..5 {
            store.add(Camera::default(), Vec3::ZERO);
        }
        let names: std::collections::BTreeSet<&str> =
            store.iter().map(|(_, c)| c.name.as_str()).collect();
        assert_eq!(names.len(), store.len(), "two cameras share a name");
    }

    /// §8.2: two vanishing points determine the focal length that produced
    /// them.
    #[test]
    fn match_perspective_recovers_a_focal_length() {
        // A 50 mm lens on a 36 mm sensor has a normalized focal length of
        // 50/36; two perpendicular vanishing points symmetric about the centre
        // then sit at ±that.
        let f = 50.0 / 36.0;
        let recovered = match_perspective_two_point((-f, 0.0), (f, 0.0), 36.0, 1.0)
            .expect("a real camera produced these");
        assert!(
            (recovered - 50.0).abs() < 1.0,
            "recovered {recovered:.1} mm rather than 50"
        );
    }

    /// Vanishing points that no camera could have produced are refused rather
    /// than answered with a plausible-looking number.
    #[test]
    fn match_perspective_refuses_impossible_input() {
        // Both on the same side of centre: no perpendicular pair does that.
        assert!(match_perspective_two_point((1.0, 0.0), (2.0, 0.0), 36.0, 1.0).is_none());
        // Coincident.
        assert!(match_perspective_two_point((0.0, 0.0), (0.0, 0.0), 36.0, 1.0).is_none());
    }

    /// All four of §8.1's lens types are offered, and they are distinct.
    #[test]
    fn every_lens_type_is_offered() {
        let types = lens_types();
        assert_eq!(types.len(), 4);
        let names: std::collections::BTreeSet<&str> = types.iter().map(|(n, _)| *n).collect();
        assert_eq!(names.len(), 4, "two lens types share a name");
        assert!(types
            .iter()
            .any(|(_, p)| matches!(p, Projection::Orthographic { .. })));
        assert!(types.iter().any(|(_, p)| matches!(p, Projection::Shift { .. })));
        assert!(types
            .iter()
            .any(|(_, p)| matches!(p, Projection::Equirectangular)));
    }
}
