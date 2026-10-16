//! Analytic lights and how to sample them (`docs/01-RENDERER-CORE.md` §5.1).
//!
//! ## Why area lights are sampled by solid angle
//!
//! The obvious way to sample a rectangular light is to pick a uniform point on
//! its surface and convert the area density to a solid-angle one by `d²/cosθ`.
//! It is a few lines, and it is what most renderers start with.
//!
//! It also falls apart in exactly the case product rendering is made of: a large
//! softbox close to the subject. Points near the edge of the rectangle subtend a
//! far smaller solid angle than points near the middle, so a uniform draw over
//! the *area* is a wildly non-uniform draw over the *directions that matter*,
//! and the `d²/cosθ` factor swings over orders of magnitude across a single
//! light. The result is a noisy image that more samples clean up only slowly.
//!
//! Sampling the spherical rectangle directly [Ureña et al. 2013] draws uniformly
//! in solid angle instead, so the density is constant and the variance from the
//! light's geometry disappears entirely. §5.1 calls this out as "why this is
//! worth the complexity", and the difference is visible at any sample count.
//!
//! The method needs the shading point to be off the light's plane and outside
//! its extent to be well-conditioned, so [`AreaLight::sample`] falls back to
//! area sampling when the spherical rectangle degenerates — which §5.1 also asks
//! for.
//!
//! ## Delta lights have no MIS partner
//!
//! A point or spot light occupies no solid angle, so a BSDF sample can never hit
//! one by chance. Its PDF is formally infinite, and the MIS weight on the
//! light-sampling side is 1. Representing that as a large finite number instead
//! would make every MIS weight subtly wrong; [`LightSample::is_delta`] says so
//! explicitly and the integrator branches on it.

use crate::ies::IesProfile;
use tequila_core::ids::LightId;
use tequila_core::math::{Aabb, Onb, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

/// What a light may illuminate (§5.4).
///
/// Not a physical property — a real lamp cannot light a surface without
/// appearing in its reflection — but a control every product renderer has,
/// because a photographer's fill light is often wanted for its light and not
/// for the rectangle it leaves in the lacquer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Visibility {
    /// Seen directly by camera rays.
    pub camera: bool,
    /// Contributes to diffuse shading.
    pub diffuse: bool,
    /// Contributes to specular shading, and so appears in reflections.
    pub specular: bool,
    /// Casts shadows.
    pub shadow: bool,
}

impl Default for Visibility {
    fn default() -> Self {
        Visibility {
            camera: true,
            diffuse: true,
            specular: true,
            shadow: true,
        }
    }
}

impl Visibility {
    /// A light that lights but is never seen: the classic invisible fill.
    pub const HIDDEN: Visibility = Visibility {
        camera: false,
        diffuse: true,
        specular: true,
        shadow: true,
    };

    pub fn lights_anything(&self) -> bool {
        self.diffuse || self.specular
    }
}

/// One sample of a light, taken towards a shading point.
#[derive(Clone, Copy, Debug)]
pub struct LightSample {
    /// Direction from the shading point towards the light, normalized.
    pub direction: Vec3,
    /// Radiance arriving along it.
    pub radiance: Spectrum,
    /// Distance to the light, for the shadow ray's extent. Infinite for a
    /// directional source.
    pub distance: f32,
    /// Solid-angle density of having drawn this direction. Meaningless for a
    /// delta light.
    pub pdf: f32,
    /// Whether the light occupies no solid angle, so BSDF sampling can never
    /// find it and MIS must not try to weight against it.
    pub is_delta: bool,
}

/// A rectangular or disc area light.
#[derive(Clone, Copy, Debug)]
pub struct AreaLight {
    /// Centre of the emitter.
    pub position: Vec3,
    /// Half-extent along the emitter's local X.
    pub half_u: Vec3,
    /// Half-extent along its local Y.
    pub half_v: Vec3,
    /// Emitted radiance, in nits.
    pub radiance: Spectrum,
    /// A disc rather than a rectangle, inscribed in the same extents.
    pub disc: bool,
    /// Emits from both sides.
    pub two_sided: bool,
}

impl AreaLight {
    pub fn normal(&self) -> Vec3 {
        self.half_u.cross(self.half_v).normalize()
    }

    pub fn area(&self) -> f32 {
        let rect = 4.0 * self.half_u.length() * self.half_v.length();
        if self.disc {
            rect * PI / 4.0
        } else {
            rect
        }
    }

    /// Total power leaving the light, for importance-weighted selection (§5.3).
    pub fn power(&self) -> f32 {
        let sides = if self.two_sided { 2.0 } else { 1.0 };
        self.radiance.luminance() * self.area() * PI * sides
    }

    /// Sample a direction towards the light from `from`.
    pub fn sample(&self, from: Vec3, u: Vec2) -> Option<LightSample> {
        let normal = self.normal();
        let to_light = self.position - from;
        let facing = to_light.dot(normal);
        if !self.two_sided && facing > 0.0 {
            // Behind the emitter.
            return None;
        }

        // The spherical rectangle needs the shading point off the plane and the
        // subtended solid angle well conditioned. Close to the plane, or with a
        // degenerate rectangle, it loses precision — so measure first and fall
        // back rather than producing a direction with a bogus density.
        let spherical = (!self.disc)
            .then(|| SphericalRectangle::build(from, self))
            .flatten();

        match spherical {
            Some(rectangle) => {
                // Uniform in solid angle, so the density is simply the
                // reciprocal of the solid angle the light subtends — no
                // `d²/cosθ` at all. That constant density is the entire point
                // of the method; computing an area density here instead would
                // do all the work and throw away the benefit.
                let direction = rectangle.direction(u);
                let cos_light = -direction.dot(normal);
                let cos_light = if self.two_sided { cos_light.abs() } else { cos_light };
                if cos_light <= 1e-6 {
                    return None;
                }
                let distance = self.distance_along(from, direction, normal)?;
                Some(LightSample {
                    direction,
                    radiance: self.radiance,
                    distance,
                    pdf: 1.0 / rectangle.solid_angle,
                    is_delta: false,
                })
            }
            None => {
                // Uniform over the surface, converted to solid angle.
                let (su, sv) = if self.disc {
                    // Concentric mapping keeps the distribution even; a naive
                    // polar draw clusters samples at the centre.
                    let radius = u.x.max(0.0).sqrt();
                    let angle = TAU * u.y;
                    (radius * angle.cos(), radius * angle.sin())
                } else {
                    (2.0 * u.x - 1.0, 2.0 * u.y - 1.0)
                };
                let point = self.position + self.half_u * su + self.half_v * sv;
                self.finish(from, point, normal)
            }
        }
    }

    fn finish(&self, from: Vec3, point: Vec3, normal: Vec3) -> Option<LightSample> {
        let offset = point - from;
        let distance_squared = offset.length_squared();
        if distance_squared < 1e-12 {
            return None;
        }
        let distance = distance_squared.sqrt();
        let direction = offset / distance;

        let cos_light = -direction.dot(normal);
        let cos_light = if self.two_sided {
            cos_light.abs()
        } else {
            cos_light
        };
        if cos_light <= 1e-6 {
            return None;
        }

        // Area density to solid angle. This is the conversion the spherical
        // rectangle exists to avoid, and it is used only on the fallback path.
        let pdf = distance_squared / (cos_light * self.area().max(1e-12));

        Some(LightSample {
            direction,
            radiance: self.radiance,
            distance,
            pdf,
            is_delta: false,
        })
    }

    /// Distance from `from` to where `direction` meets the light's plane.
    fn distance_along(&self, from: Vec3, direction: Vec3, normal: Vec3) -> Option<f32> {
        let denominator = direction.dot(normal);
        if denominator.abs() < 1e-9 {
            return None;
        }
        let t = (self.position - from).dot(normal) / denominator;
        (t > 1e-6).then_some(t)
    }

    /// Density of sampling `direction` from `from`, for MIS.
    ///
    /// Must agree with whichever strategy [`AreaLight::sample`] would have
    /// used from the same point, or every MIS weight is computed against a
    /// density nothing produced.
    pub fn pdf(&self, from: Vec3, direction: Vec3) -> f32 {
        let normal = self.normal();
        let denominator = direction.dot(normal);
        if denominator.abs() < 1e-6 {
            return 0.0;
        }
        let t = (self.position - from).dot(normal) / denominator;
        if t <= 1e-6 {
            return 0.0;
        }
        let point = from + direction * t;
        let offset = point - self.position;
        let (su, sv) = (
            offset.dot(self.half_u) / self.half_u.length_squared().max(1e-12),
            offset.dot(self.half_v) / self.half_v.length_squared().max(1e-12),
        );
        let inside = if self.disc {
            su * su + sv * sv <= 1.0
        } else {
            su.abs() <= 1.0 && sv.abs() <= 1.0
        };
        if !inside {
            return 0.0;
        }

        let cos_light = if self.two_sided {
            denominator.abs()
        } else {
            -denominator
        };
        if cos_light <= 1e-6 {
            return 0.0;
        }

        // The same branch the sampler takes, for the same reason.
        if !self.disc {
            if let Some(rectangle) = SphericalRectangle::build(from, self) {
                return 1.0 / rectangle.solid_angle;
            }
        }
        (t * t) / (cos_light * self.area().max(1e-12))
    }
}

/// The spherical rectangle a planar quad subtends, sampled uniformly in solid
/// angle [Ureña et al. 2013].
#[derive(Clone, Copy, Debug)]
struct SphericalRectangle {
    x: Vec3,
    y: Vec3,
    z: Vec3,
    z0: f32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    b0: f32,
    b1: f32,
    k: f32,
    solid_angle: f32,
}

impl SphericalRectangle {
    fn build(from: Vec3, light: &AreaLight) -> Option<SphericalRectangle> {
        let ex = light.half_u * 2.0;
        let ey = light.half_v * 2.0;
        let origin = light.position - light.half_u - light.half_v;

        let (exl, eyl) = (ex.length(), ey.length());
        if exl < 1e-9 || eyl < 1e-9 {
            return None;
        }
        let x = ex / exl;
        let y = ey / eyl;
        let mut z = x.cross(y);

        let d = origin - from;
        let mut z0 = d.dot(z);
        // Orient the frame so the rectangle is in front.
        if z0 > 0.0 {
            z = -z;
            z0 = -z0;
        }
        // Too close to the plane: the formulation loses all its precision, and
        // a density that is wrong by a factor is worse than a noisier estimator
        // that is right.
        if z0.abs() < 1e-5 {
            return None;
        }

        let x0 = d.dot(x);
        let y0 = d.dot(y);
        let x1 = x0 + exl;
        let y1 = y0 + eyl;

        // The four corner vectors, and the solid angle from their spherical
        // excess.
        let v00 = Vec3 { x: x0, y: y0, z: z0 };
        let v01 = Vec3 { x: x0, y: y1, z: z0 };
        let v10 = Vec3 { x: x1, y: y0, z: z0 };
        let v11 = Vec3 { x: x1, y: y1, z: z0 };
        let n0 = v00.cross(v10).normalize();
        let n1 = v10.cross(v11).normalize();
        let n2 = v11.cross(v01).normalize();
        let n3 = v01.cross(v00).normalize();

        let g0 = (-n0.dot(n1)).clamp(-1.0, 1.0).acos();
        let g1 = (-n1.dot(n2)).clamp(-1.0, 1.0).acos();
        let g2 = (-n2.dot(n3)).clamp(-1.0, 1.0).acos();
        let g3 = (-n3.dot(n0)).clamp(-1.0, 1.0).acos();
        let solid_angle = g0 + g1 + g2 + g3 - TAU;

        if !solid_angle.is_finite() || solid_angle < 1e-7 {
            return None;
        }

        let b0 = n0.z;
        let b1 = n2.z;
        let k = TAU - g2 - g3;

        let _ = origin;
        Some(SphericalRectangle {
            x,
            y,
            z,
            z0,
            x0,
            y0,
            x1,
            y1,
            b0,
            b1,
            k,
            solid_angle,
        })
    }

    /// A direction drawn uniformly over the solid angle the rectangle
    /// subtends.
    fn direction(&self, u: Vec2) -> Vec3 {
        // Compute cu, the azimuthal coordinate, by inverting the spherical
        // excess.
        let au = u.x * self.solid_angle + self.k;
        let fu = (au.cos() * self.b0 - self.b1) / au.sin();
        let cu = (1.0f32).copysign(fu) / (fu * fu + self.b0 * self.b0).sqrt();
        let cu = cu.clamp(-1.0, 1.0);

        let xu = -(cu * self.z0) / (1.0 - cu * cu).max(1e-12).sqrt();
        let xu = xu.clamp(self.x0, self.x1);

        // Then the polar coordinate along that arc.
        let d = (xu * xu + self.z0 * self.z0).sqrt();
        let h0 = self.y0 / (d * d + self.y0 * self.y0).sqrt();
        let h1 = self.y1 / (d * d + self.y1 * self.y1).sqrt();
        let hv = h0 + u.y * (h1 - h0);
        let hv2 = hv * hv;
        let yv = if hv2 < 1.0 - 1e-6 {
            (hv * d) / (1.0 - hv2).sqrt()
        } else {
            self.y1
        };

        // The sampled point, in the frame built around the shading point:
        // `(xu, yv, z0)` are its coordinates there, so the direction is that
        // vector normalized. Reconstructing it in world space and subtracting
        // `from` again would be the same thing with a cancellation in the
        // middle.
        let offset = self.x * xu + self.y * yv + self.z * self.z0;
        offset.try_normalize().unwrap_or(self.z)
    }
}

/// A point light, optionally with a radius that makes it a small sphere.
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    pub position: Vec3,
    /// Luminous intensity in candela, as a colour.
    pub intensity: Spectrum,
    /// 0 is a true delta; above it the light is a sphere and casts soft
    /// shadows.
    pub radius: f32,
}

impl PointLight {
    pub fn power(&self) -> f32 {
        self.intensity.luminance() * 4.0 * PI
    }

    pub fn sample(&self, from: Vec3, u: Vec2) -> Option<LightSample> {
        let offset = if self.radius > 0.0 {
            // A point on the sphere's surface, facing the shading point.
            let towards = (from - self.position).try_normalize()?;
            let frame = Onb::from_normal(towards);
            let z = 1.0 - 2.0 * u.x;
            let r = (1.0 - z * z).max(0.0).sqrt();
            let phi = TAU * u.y;
            let local = Vec3 {
                x: r * phi.cos(),
                y: r * phi.sin(),
                z,
            };
            self.position + frame.to_world(local) * self.radius - from
        } else {
            self.position - from
        };

        let distance_squared = offset.length_squared();
        if distance_squared < 1e-12 {
            return None;
        }
        let distance = distance_squared.sqrt();

        Some(LightSample {
            direction: offset / distance,
            // Intensity to irradiance: the inverse-square law, which is the
            // whole of a point light's falloff.
            radiance: self.intensity / distance_squared,
            distance,
            pdf: 1.0,
            is_delta: self.radius <= 0.0,
        })
    }
}

/// A spotlight: a point light inside a cone, with a soft edge.
#[derive(Clone, Copy, Debug)]
pub struct SpotLight {
    pub position: Vec3,
    /// Where the cone points, normalized.
    pub direction: Vec3,
    pub intensity: Spectrum,
    /// Half-angle of the fully-lit core, in radians.
    pub inner_angle: f32,
    /// Half-angle at which the light reaches zero.
    pub outer_angle: f32,
    /// Falloff exponent applied across the penumbra.
    pub falloff: f32,
}

impl SpotLight {
    pub fn power(&self) -> f32 {
        // The solid angle of the outer cone, treating the penumbra as half lit.
        let cone = TAU * (1.0 - self.outer_angle.cos());
        self.intensity.luminance() * cone
    }

    /// The fraction of full intensity at an angle from the axis.
    ///
    /// Smooth-stepped rather than linear across the penumbra (§5.1): a linear
    /// ramp leaves a visible crease at the inner angle, because the derivative
    /// jumps there and the eye is very good at finding those on a smooth wall.
    pub fn falloff_at(&self, cos_angle: f32) -> f32 {
        let cos_outer = self.outer_angle.cos();
        let cos_inner = self.inner_angle.cos();
        if cos_angle <= cos_outer {
            return 0.0;
        }
        if cos_angle >= cos_inner {
            return 1.0;
        }
        let t = ((cos_angle - cos_outer) / (cos_inner - cos_outer).max(1e-6)).clamp(0.0, 1.0);
        let smooth = t * t * (3.0 - 2.0 * t);
        smooth.powf(self.falloff.max(0.0))
    }

    pub fn sample(&self, from: Vec3, _u: Vec2) -> Option<LightSample> {
        let offset = self.position - from;
        let distance_squared = offset.length_squared();
        if distance_squared < 1e-12 {
            return None;
        }
        let distance = distance_squared.sqrt();
        let direction = offset / distance;

        let cos_angle = (-direction).dot(self.direction);
        let falloff = self.falloff_at(cos_angle);
        if falloff <= 0.0 {
            return None;
        }

        Some(LightSample {
            direction,
            radiance: self.intensity * falloff / distance_squared,
            distance,
            pdf: 1.0,
            is_delta: true,
        })
    }
}

/// A luminaire whose distribution comes from a measurement rather than from a
/// formula (§5.1).
///
/// A point light with a photometric web attached, and modelled that way on
/// purpose: everything about how it is sampled — a delta direction, an inverse
/// square falloff, one shadow ray — is a point light's, and the only difference
/// is where the intensity in a given direction comes from. Making it a separate
/// kind of *sampling* would be inventing a difference that is not there.
#[derive(Clone, Debug, PartialEq)]
pub struct IesLight {
    pub position: Vec3,
    /// The aiming axis: the profile's 0° vertical angle.
    pub direction: Vec3,
    /// The zero of the profile's horizontal angle. Only matters for an
    /// asymmetric luminaire; for a symmetric one any perpendicular will do.
    pub right: Vec3,
    /// Colour and overall scale. The profile supplies the shape of the
    /// distribution; this supplies what the user set.
    pub intensity: Spectrum,
    pub profile: Arc<IesProfile>,
}

impl IesLight {
    /// Candela towards a world-space direction.
    fn candela(&self, world: Vec3) -> f32 {
        let up = self.direction.cross(self.right);
        // Into the luminaire's frame, where the profile expects `-Z` forward.
        let local = Vec3 {
            x: world.dot(self.right),
            y: world.dot(up),
            z: -world.dot(self.direction),
        };
        self.profile.candela_towards(local)
    }

    pub fn power(&self) -> f32 {
        // The equivalent isotropic intensity over the whole sphere — the same
        // quantity `PointLight::power` reports, so selection can weigh the two
        // against each other. The *peak* would have a narrow downlight sampled
        // as if it lit the whole room.
        self.intensity.luminance() * self.profile.mean() * 4.0 * PI
    }

    pub fn sample(&self, from: Vec3, _u: Vec2) -> Option<LightSample> {
        let offset = self.position - from;
        let distance_squared = offset.dot(offset);
        if distance_squared < 1e-12 {
            return None;
        }
        let distance = distance_squared.sqrt();
        let direction = offset / distance;

        // The profile is asked about the direction light *leaves* the
        // luminaire in, which is the reverse of the one the shading point looks
        // along. Getting this backwards turns every downlight into an uplight.
        let candela = self.candela(-direction);
        if candela <= 0.0 {
            return None;
        }
        Some(LightSample {
            direction,
            radiance: self.intensity * (candela / distance_squared),
            distance,
            pdf: 1.0,
            is_delta: true,
        })
    }
}

/// One of the analytic light types.
///
/// No longer `Copy`, because a photometric profile is a measured table shared
/// between every instance of the same fitting — a `Copy` light would mean
/// copying that table on every selection.
#[derive(Clone, Debug)]
pub enum LightKind {
    Area(AreaLight),
    Point(PointLight),
    Spot(SpotLight),
    Ies(IesLight),
}

/// A light in the scene, with the flags every light carries (§5.1).
#[derive(Clone, Debug)]
pub struct Light {
    pub kind: LightKind,
    pub visibility: Visibility,
    /// §5.5's light layers, for rendering a scene's lights as separate passes.
    pub layer: u8,
    pub enabled: bool,
}

impl Light {
    pub fn new(kind: LightKind) -> Light {
        Light {
            kind,
            visibility: Visibility::default(),
            layer: 0,
            enabled: true,
        }
    }

    pub fn with_visibility(mut self, visibility: Visibility) -> Light {
        self.visibility = visibility;
        self
    }

    pub fn with_layer(mut self, layer: u8) -> Light {
        self.layer = layer;
        self
    }

    /// Total power, for the importance-weighted selection of §5.3.
    pub fn power(&self) -> f32 {
        if !self.enabled || !self.visibility.lights_anything() {
            return 0.0;
        }
        match &self.kind {
            LightKind::Area(l) => l.power(),
            LightKind::Point(l) => l.power(),
            LightKind::Spot(l) => l.power(),
            LightKind::Ies(l) => l.power(),
        }
    }

    pub fn sample(&self, from: Vec3, u: Vec2) -> Option<LightSample> {
        if !self.enabled {
            return None;
        }
        match &self.kind {
            LightKind::Area(l) => l.sample(from, u),
            LightKind::Point(l) => l.sample(from, u),
            LightKind::Spot(l) => l.sample(from, u),
            LightKind::Ies(l) => l.sample(from, u),
        }
    }

    /// Density of a direction, for MIS. Zero for delta lights, which BSDF
    /// sampling can never reach.
    pub fn pdf(&self, from: Vec3, direction: Vec3) -> f32 {
        match &self.kind {
            LightKind::Area(l) => l.pdf(from, direction),
            LightKind::Point(_) | LightKind::Spot(_) | LightKind::Ies(_) => 0.0,
        }
    }

    pub fn is_delta(&self) -> bool {
        match &self.kind {
            LightKind::Area(_) => false,
            LightKind::Point(l) => l.radius <= 0.0,
            LightKind::Spot(_) | LightKind::Ies(_) => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Selection (§5.3)
// ---------------------------------------------------------------------------

/// How a light is chosen at a shading point.
///
/// §5.3: "For scenes below ~8 lights, use a simple power-weighted alias table —
/// it's cheaper and the BVH's benefit is nil. Choose the strategy at scene-build
/// time based on light count and log which was chosen."
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionStrategy {
    /// Power-weighted alias table. Constant time, ignores position.
    #[default]
    Alias,
    /// Importance-weighted traversal, which accounts for distance and
    /// orientation as well as power.
    Hierarchy,
}

/// Above this many lights, position starts to matter more than the cost of
/// accounting for it.
pub const HIERARCHY_THRESHOLD: usize = 8;

/// The lights of a scene, prepared for sampling.
#[derive(Clone, Debug, Default)]
pub struct LightSet {
    lights: Vec<Light>,
    /// Alias table: (probability, alternative) per light.
    alias: Vec<(f32, u32)>,
    /// Selection probability of each light, for MIS.
    probabilities: Vec<f32>,
    total_power: f32,
    strategy: SelectionStrategy,
    /// The light hierarchy of §5.3, built only when the strategy calls for it.
    nodes: Vec<LightNode>,
    /// Light indices, reordered by the build so each node owns a contiguous
    /// range of them.
    order: Vec<u32>,
}

impl LightSet {
    pub fn new(lights: Vec<Light>) -> LightSet {
        let count = lights.len();
        let strategy = if count > HIERARCHY_THRESHOLD {
            SelectionStrategy::Hierarchy
        } else {
            SelectionStrategy::Alias
        };
        LightSet::with_strategy(lights, strategy)
    }

    /// Build with a chosen strategy rather than the one the light count
    /// implies.
    ///
    /// §5.3 picks the strategy at build time from the light count, and that is
    /// what [`LightSet::new`] does. This exists because the two strategies are
    /// estimators of the *same* quantity, so the only convincing check on
    /// either is to render a scene both ways and compare — which needs a way
    /// to ask for the one the count would not have chosen.
    pub fn with_strategy(lights: Vec<Light>, strategy: SelectionStrategy) -> LightSet {
        let powers: Vec<f32> = lights.iter().map(|l| l.power()).collect();
        let total_power: f32 = powers.iter().sum();
        let count = lights.len();

        let probabilities: Vec<f32> = if total_power > 0.0 {
            powers.iter().map(|p| p / total_power).collect()
        } else {
            vec![if count == 0 { 0.0 } else { 1.0 / count as f32 }; count]
        };

        let (nodes, order) = if strategy == SelectionStrategy::Hierarchy {
            build_hierarchy(&lights, &powers)
        } else {
            (Vec::new(), Vec::new())
        };

        LightSet {
            alias: build_alias(&probabilities),
            probabilities,
            lights,
            total_power,
            strategy,
            nodes,
            order,
        }
    }

    pub fn len(&self) -> usize {
        self.lights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lights.is_empty()
    }

    pub fn lights(&self) -> &[Light] {
        &self.lights
    }

    pub fn get(&self, id: LightId) -> Option<&Light> {
        self.lights.get(id.0 as usize)
    }

    pub fn strategy(&self) -> SelectionStrategy {
        self.strategy
    }

    pub fn total_power(&self) -> f32 {
        self.total_power
    }

    /// Choose a light, returning it and the probability of that choice.
    ///
    /// The probability is returned rather than assumed, because the estimator
    /// divides by it. A selection scheme that changes — as this one does at
    /// eight lights — must not require the integrator to know which is in use.
    pub fn select(&self, u: f32) -> Option<(LightId, f32)> {
        if self.lights.is_empty() {
            return None;
        }
        let count = self.lights.len();
        let scaled = (u.clamp(0.0, 1.0) * count as f32) as usize;
        let index = scaled.min(count - 1);
        let (probability, alternative) = self.alias[index];
        let fractional = u * count as f32 - index as f32;
        let chosen = if fractional < probability {
            index
        } else {
            alternative as usize
        };
        let p = self.probabilities[chosen];
        if p <= 0.0 {
            return None;
        }
        Some((LightId(chosen as u32), p))
    }

    /// Choose a light *as seen from a shading point*.
    ///
    /// This is the entry point the integrator uses, and the reason the strategy
    /// can change at eight lights without the integrator knowing. Below the
    /// threshold it is [`LightSet::select`] — position adds nothing when there
    /// are three lights and they are all worth sampling. Above it, §5.3's
    /// hierarchy, where a light on the far side of a building is not chosen a
    /// hundredth of the time merely because it is bright.
    pub fn select_at(&self, point: Vec3, normal: Vec3, u: f32) -> Option<(LightId, f32)> {
        if self.strategy == SelectionStrategy::Alias || self.nodes.is_empty() {
            return self.select(u);
        }

        let mut index = 0usize;
        let mut probability = 1.0f32;
        let mut u = u.clamp(0.0, 1.0);

        loop {
            let node = self.nodes[index];
            let Some((left, right)) = node.children else {
                // A leaf. One light, so nothing further to choose.
                let light = *self.order.get(node.first as usize)?;
                if probability <= 0.0 {
                    return None;
                }
                return Some((LightId(light), probability));
            };

            let importance_left = self.nodes[left as usize].importance(point, normal);
            let importance_right = self.nodes[right as usize].importance(point, normal);
            let total = importance_left + importance_right;

            // Neither child can light this point — every light in the subtree is
            // behind the surface or outside its emission cone. Falling back to a
            // coin flip rather than giving up keeps the estimator unbiased in the
            // corner where the importance heuristic underestimates; giving up
            // would lose real light.
            let p_left = if total > 0.0 {
                importance_left / total
            } else {
                0.5
            };

            if u < p_left {
                // Rescaled so the same variate keeps choosing all the way down,
                // rather than each level needing its own dimension — which would
                // make the sampler's dimension count depend on the tree's depth.
                u = if p_left > 0.0 { u / p_left } else { 0.0 };
                probability *= p_left;
                index = left as usize;
            } else {
                let p_right = 1.0 - p_left;
                u = if p_right > 0.0 {
                    (u - p_left) / p_right
                } else {
                    0.0
                };
                probability *= p_right;
                index = right as usize;
            }
            u = u.clamp(0.0, 1.0);
            if probability <= 0.0 {
                return None;
            }
        }
    }

    /// The probability that [`LightSet::select`] would have chosen this light,
    /// for MIS.
    pub fn selection_pdf(&self, id: LightId) -> f32 {
        self.probabilities.get(id.0 as usize).copied().unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------------------
// The light hierarchy (§5.3, after Conty Estevez & Kulla 2018)
// ---------------------------------------------------------------------------

/// One node of the light hierarchy.
///
/// Carries what the importance heuristic needs and nothing else: where the
/// lights in this subtree are, which way they face, how far that facing
/// spreads, and how much power they add up to. The orientation cone is what
/// separates this from a plain power-weighted table — a spot light pointing
/// away from the shading point contributes nothing however bright it is, and a
/// heuristic that only knew power would keep choosing it.
#[derive(Clone, Copy, Debug)]
struct LightNode {
    bounds: Aabb,
    /// Average emission direction of the subtree.
    axis: Vec3,
    /// Half-angle the emission directions spread over. `pi` means every
    /// direction, which is how an isotropic point light is represented — its
    /// axis is then meaningless, and saying so here is what keeps the cone
    /// term from ever cutting it down.
    theta_o: f32,
    /// How far past `theta_o` a light still falls off before reaching zero.
    /// At most `pi/2`, because the term it feeds is a cosine and a wider
    /// angle would make that cosine negative — which turns an importance into
    /// a *negative* weight, makes one child's share exceed one, and stops the
    /// traversal probabilities being a distribution at all.
    theta_e: f32,
    power: f32,
    /// Index into the reordered light list. Meaningful only for a leaf.
    first: u32,
    children: Option<(u32, u32)>,
}

impl LightNode {
    /// How much this subtree is worth sampling from `point`.
    ///
    /// The shape is §5.3's: power over distance squared, cut down by how far
    /// the shading point sits outside the subtree's emission cone and by how
    /// obliquely it sees the subtree. The distance is clamped to the node's own
    /// radius, without which a point *inside* a cluster divides by nearly zero
    /// and the traversal commits to whichever child it happens to stand in.
    fn importance(&self, point: Vec3, normal: Vec3) -> f32 {
        if self.power <= 0.0 {
            return 0.0;
        }
        let centre = self.bounds.center();
        let radius = (self.bounds.diagonal().length() * 0.5).max(1e-4);
        let offset = centre - point;
        let distance = offset.length();
        let direction = if distance > 1e-6 {
            offset / distance
        } else {
            normal
        };
        let distance_squared = (distance * distance).max(radius * radius);

        // Half-angle the node's bounds subtend from here. Everything below is
        // widened by it, so a large cluster is not judged as if it were a point.
        let theta_u = if distance > radius {
            (radius / distance).clamp(-1.0, 1.0).asin()
        } else {
            std::f32::consts::FRAC_PI_2
        };

        // How far the shading point is outside the emission cone.
        let cos_theta = (-direction).dot(self.axis).clamp(-1.0, 1.0);
        let theta = cos_theta.acos();
        let theta_prime = (theta - self.theta_o - theta_u).max(0.0);
        if theta_prime >= self.theta_e {
            return 0.0;
        }

        // And how obliquely the surface sees it. A light below the horizon
        // contributes nothing, which is the single biggest saving the heuristic
        // makes in a scene lit from one side.
        let cos_incident = direction.dot(normal).clamp(-1.0, 1.0);
        let theta_incident = (cos_incident.acos() - theta_u).max(0.0);
        if theta_incident >= std::f32::consts::FRAC_PI_2 {
            return 0.0;
        }

        self.power * theta_prime.cos() * theta_incident.cos() / distance_squared
    }
}

/// Build the hierarchy, returning the nodes and the light order they index.
///
/// A median split on the widest axis rather than a surface-area heuristic: the
/// tree is built once per scene edit over a handful of lights, and the SAH's
/// advantage is a traversal cost this structure does not pay — every descent
/// visits exactly one child.
fn build_hierarchy(lights: &[Light], powers: &[f32]) -> (Vec<LightNode>, Vec<u32>) {
    if lights.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut order: Vec<u32> = (0..lights.len() as u32).collect();
    let mut nodes = Vec::with_capacity(lights.len() * 2);
    split(lights, powers, &mut order, &mut nodes, 0, lights.len());
    (nodes, order)
}

fn split(
    lights: &[Light],
    powers: &[f32],
    order: &mut [u32],
    nodes: &mut Vec<LightNode>,
    first: usize,
    count: usize,
) -> u32 {
    let index = nodes.len() as u32;
    let mut node = cover(lights, powers, &order[first..first + count]);
    node.first = first as u32;
    nodes.push(node);

    if count <= 1 {
        return index;
    }

    // Split on the widest axis of the cluster's bounds, at its median.
    let diagonal = node.bounds.diagonal();
    let axis = if diagonal.x >= diagonal.y && diagonal.x >= diagonal.z {
        0
    } else if diagonal.y >= diagonal.z {
        1
    } else {
        2
    };
    let key = |id: &u32| -> f32 {
        let p = light_position(&lights[*id as usize]);
        match axis {
            0 => p.x,
            1 => p.y,
            _ => p.z,
        }
    };
    let slice = &mut order[first..first + count];
    slice.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));

    let middle = count / 2;
    let left = split(lights, powers, order, nodes, first, middle);
    let right = split(lights, powers, order, nodes, first + middle, count - middle);
    nodes[index as usize].children = Some((left, right));
    index
}

/// The node covering a set of lights: their bounds, their combined emission
/// cone, and their total power.
fn cover(lights: &[Light], powers: &[f32], ids: &[u32]) -> LightNode {
    let mut bounds = Aabb::EMPTY;
    let mut axis_sum = Vec3::ZERO;
    let mut power = 0.0;
    let mut theta_e: f32 = 0.0;
    for id in ids {
        let light = &lights[*id as usize];
        let position = light_position(light);
        bounds.extend_point(position);
        // Weighted by power, so a dim light pointing elsewhere does not drag
        // a bright cluster's axis away from where the light actually goes.
        let weight = powers[*id as usize].max(1e-6);
        axis_sum = axis_sum + light_axis(light) * weight;
        power += powers[*id as usize];
        theta_e = theta_e.max(light_theta_e(light));
    }
    let axis = axis_sum.try_normalize().unwrap_or(Vec3::Y);

    // The spread: the widest angle any member's *own* cone reaches from the
    // average axis. Exact for one light — a leaf's cone is the light's own —
    // and a conservative cover for a cluster, which is what the heuristic
    // needs, since underestimating it would cull lights that do reach.
    let mut theta_o: f32 = 0.0;
    for id in ids {
        let light = &lights[*id as usize];
        let cos = light_axis(light).dot(axis).clamp(-1.0, 1.0);
        theta_o = theta_o.max(cos.acos() + light_spread(light));
    }
    let theta_o = theta_o.min(std::f32::consts::PI);

    LightNode {
        bounds,
        axis,
        theta_o,
        theta_e,
        power,
        first: 0,
        children: None,
    }
}

fn light_position(light: &Light) -> Vec3 {
    match &light.kind {
        LightKind::Area(area) => area.position,
        LightKind::Point(point) => point.position,
        LightKind::Spot(spot) => spot.position,
        LightKind::Ies(ies) => ies.position,
    }
}

/// The direction a light emits along.
fn light_axis(light: &Light) -> Vec3 {
    match &light.kind {
        LightKind::Area(area) => area.normal(),
        // A point light emits in every direction, so its axis is arbitrary and
        // its `theta_e` covers the whole sphere. Any unit vector will do.
        LightKind::Point(_) => Vec3::Y,
        LightKind::Spot(spot) => spot.direction,
        LightKind::Ies(ies) => ies.direction,
    }
}

/// How wide the set of directions a light emits along is, as a half-angle
/// about [`light_axis`].
fn light_spread(light: &Light) -> f32 {
    match &light.kind {
        // A one-sided panel emits about its normal; a two-sided one emits over
        // the whole sphere, which is a half-angle of pi/2 about either face.
        LightKind::Area(area) => {
            if area.two_sided {
                std::f32::consts::FRAC_PI_2
            } else {
                0.0
            }
        }
        // Every direction, so the cone term can never cut it down.
        LightKind::Point(_) => std::f32::consts::PI,
        LightKind::Spot(_) => 0.0,
        // A measured luminaire emits wherever its table says it does, and the
        // table is not a cone. Claiming a spread of zero would let the
        // hierarchy cull a downlight from directly beneath it.
        LightKind::Ies(_) => std::f32::consts::PI,
    }
}

/// How far past its spread a light falls off to nothing.
///
/// Capped at `pi/2` for the reason [`LightNode::theta_e`] gives.
fn light_theta_e(light: &Light) -> f32 {
    match &light.kind {
        LightKind::Area(_) | LightKind::Point(_) | LightKind::Ies(_) => {
            std::f32::consts::FRAC_PI_2
        }
        LightKind::Spot(spot) => spot.outer_angle.min(std::f32::consts::FRAC_PI_2),
    }
}

/// Vose's alias method: O(1) sampling from a discrete distribution.
fn build_alias(probabilities: &[f32]) -> Vec<(f32, u32)> {
    let count = probabilities.len();
    if count == 0 {
        return Vec::new();
    }
    let mut table = vec![(1.0f32, 0u32); count];
    let mut scaled: Vec<f32> = probabilities.iter().map(|p| p * count as f32).collect();

    let mut small: Vec<usize> = Vec::new();
    let mut large: Vec<usize> = Vec::new();
    for (i, p) in scaled.iter().enumerate() {
        if *p < 1.0 {
            small.push(i);
        } else {
            large.push(i);
        }
    }

    while let (Some(l), Some(g)) = (small.pop(), large.pop()) {
        table[l] = (scaled[l], g as u32);
        scaled[g] = (scaled[g] + scaled[l]) - 1.0;
        if scaled[g] < 1.0 {
            small.push(g);
        } else {
            large.push(g);
        }
    }
    for i in large.into_iter().chain(small) {
        table[i] = (1.0, i as u32);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row of point lights, evenly spaced along X.
    fn row(count: usize) -> LightSet {
        LightSet::new(
            (0..count)
                .map(|i| {
                    Light::new(LightKind::Point(PointLight {
                        position: tequila_core::math::vec3(i as f32 * 4.0 - 20.0, 3.0, 0.0),
                        intensity: Spectrum::splat(10.0),
                        radius: 0.0,
                    }))
                })
                .collect(),
        )
    }

    /// §5.3 fixes the crossover at eight lights and asks for it to be chosen at
    /// build time. Both sides of it have to actually take effect, or the
    /// threshold is a comment.
    #[test]
    fn the_strategy_changes_at_the_documented_threshold() {
        assert_eq!(row(HIERARCHY_THRESHOLD).strategy(), SelectionStrategy::Alias);
        assert_eq!(
            row(HIERARCHY_THRESHOLD + 1).strategy(),
            SelectionStrategy::Hierarchy
        );
    }

    /// The property the whole estimator rests on: the probabilities the
    /// traversal reports have to be a distribution. If they sum to less than
    /// one the scene loses light; more, and it gains light out of nothing.
    /// Neither is visible as anything but a scene that is the wrong brightness.
    #[test]
    fn hierarchy_probabilities_sum_to_one() {
        let set = row(16);
        assert_eq!(set.strategy(), SelectionStrategy::Hierarchy);

        for point in [
            tequila_core::math::vec3(0.0, 0.0, 0.0),
            tequila_core::math::vec3(-18.0, 0.0, 2.0),
            tequila_core::math::vec3(30.0, 1.0, -5.0),
        ] {
            let mut total = 0.0;
            let mut seen = vec![false; set.len()];
            // A fine sweep of the variate: every light the traversal can reach
            // appears in some interval of it, and the intervals partition
            // `[0, 1)`, so summing each distinct light's probability once
            // reconstructs the distribution.
            const SWEEP: usize = 20000;
            let mut counts = vec![0usize; set.len()];
            let mut reported = vec![0.0f32; set.len()];
            for i in 0..SWEEP {
                let u = (i as f32 + 0.5) / SWEEP as f32;
                if let Some((id, p)) = set.select_at(point, Vec3::Y, u) {
                    counts[id.0 as usize] += 1;
                    if !seen[id.0 as usize] {
                        seen[id.0 as usize] = true;
                        reported[id.0 as usize] = p;
                        total += p;
                    }
                }
            }
            assert!(
                (total - 1.0).abs() < 1e-3,
                "at {point:?} the probabilities sum to {total}"
            );

            // And the reported probability has to be the frequency the
            // traversal actually produces. Those are two different things, and
            // an implementation can get the arithmetic right while routing the
            // variate wrongly — the estimator divides by the reported value, so
            // a light chosen half as often as it claims comes out twice as
            // bright.
            for (id, count) in counts.iter().enumerate() {
                let frequency = *count as f32 / SWEEP as f32;
                let claimed = reported[id];
                assert!(
                    (frequency - claimed).abs() < 1e-3,
                    "light {id} at {point:?} is chosen {frequency} of the time \
                     but reports {claimed}"
                );
            }
        }
    }

    /// What the hierarchy is *for*: a light near the shading point must be
    /// chosen more often than an identical one far away. A power-weighted table
    /// gives them the same probability, which is precisely the behaviour §5.3
    /// says costs a hundred-light scene a hundred times a one-light scene.
    #[test]
    fn the_hierarchy_prefers_lights_that_are_close() {
        let set = row(16);
        let point = tequila_core::math::vec3(-20.0, 0.0, 0.0);

        let mut near = 0.0f32;
        let mut far = 0.0f32;
        for i in 0..20000 {
            let u = (i as f32 + 0.5) / 20000.0;
            let Some((id, p)) = set.select_at(point, Vec3::Y, u) else {
                continue;
            };
            let x = match &set.get(id).unwrap().kind {
                LightKind::Point(light) => light.position.x,
                _ => unreachable!(),
            };
            if x < -10.0 {
                near = near.max(p);
            }
            if x > 10.0 {
                far = far.max(p);
            }
        }
        assert!(
            near > far * 4.0,
            "the nearest light is chosen with {near} against the farthest {far}"
        );
    }

    /// A spot light pointing away must not be chosen merely for being bright.
    /// The orientation cone is the part of the heuristic that knows this, and
    /// it is the part a power-only table cannot express.
    #[test]
    fn the_hierarchy_discounts_lights_facing_away() {
        let mut lights: Vec<Light> = Vec::new();
        // Nine spots in a line above the origin, all bright. One points down at
        // the shading point; the rest point straight up.
        for i in 0..9 {
            let up = i != 4;
            lights.push(Light::new(LightKind::Spot(SpotLight {
                position: tequila_core::math::vec3(i as f32 - 4.0, 5.0, 0.0),
                direction: if up { Vec3::Y } else { -Vec3::Y },
                intensity: Spectrum::splat(50.0),
                inner_angle: 0.2,
                outer_angle: 0.4,
                falloff: 1.0,
            })));
        }
        let set = LightSet::new(lights);
        assert_eq!(set.strategy(), SelectionStrategy::Hierarchy);

        let mut downward = 0.0f32;
        let mut upward: f32 = 0.0;
        for i in 0..20000 {
            let u = (i as f32 + 0.5) / 20000.0;
            let Some((id, p)) = set.select_at(Vec3::ZERO, Vec3::Y, u) else {
                continue;
            };
            if id.0 == 4 {
                downward = downward.max(p);
            } else {
                upward = upward.max(p);
            }
        }
        assert!(
            downward > upward * 2.0,
            "the one light that can reach the point is chosen with {downward}, \
             against {upward} for one pointing the wrong way"
        );
    }

    /// Even where the heuristic says nothing can contribute, selection must
    /// stay a valid distribution rather than returning nothing. Giving up there
    /// would lose the light the heuristic was wrong about — and it is a
    /// heuristic, not a proof.
    #[test]
    fn selection_still_works_where_the_heuristic_sees_nothing() {
        let set = row(16);
        // Below the lights, facing down: every one of them is behind the
        // surface, so every importance is zero.
        let point = tequila_core::math::vec3(0.0, 10.0, 0.0);
        let mut chosen = 0;
        for i in 0..1000 {
            let u = (i as f32 + 0.5) / 1000.0;
            if let Some((_, p)) = set.select_at(point, -Vec3::Y, u) {
                assert!(p > 0.0);
                chosen += 1;
            }
        }
        assert_eq!(chosen, 1000, "selection gave up where importance was zero");
    }
    use tequila_core::math::{vec2, vec3};

    fn softbox() -> AreaLight {
        AreaLight {
            position: vec3(0.0, 2.0, 0.0),
            half_u: vec3(0.5, 0.0, 0.0),
            half_v: vec3(0.0, 0.0, 0.5),
            radiance: Spectrum::splat(10.0),
            disc: false,
            two_sided: false,
        }
    }

    /// The estimator's correctness condition: sampling the light and dividing
    /// by the density must integrate to the irradiance the light actually
    /// delivers.
    ///
    /// Checked against a reference computed by brute-force area integration,
    /// which shares no code with the sampler — a sampler checked against its own
    /// density agrees with itself whatever it does.
    #[test]
    fn an_area_light_delivers_the_irradiance_it_should() {
        let light = softbox();
        let point = vec3(0.0, 0.0, 0.0);
        let normal = Vec3::Y;

        // Reference: integrate over the surface directly, finely.
        const GRID: usize = 400;
        let mut reference = 0.0f64;
        let area_element = light.area() as f64 / (GRID * GRID) as f64;
        for i in 0..GRID {
            for j in 0..GRID {
                let su = 2.0 * (i as f32 + 0.5) / GRID as f32 - 1.0;
                let sv = 2.0 * (j as f32 + 0.5) / GRID as f32 - 1.0;
                let on_light = light.position + light.half_u * su + light.half_v * sv;
                let offset = on_light - point;
                let distance_squared = offset.length_squared();
                let direction = offset / distance_squared.sqrt();
                let cos_surface = direction.dot(normal).max(0.0);
                let cos_light = (-direction).dot(light.normal()).max(0.0);
                reference += (light.radiance.luminance() * cos_surface * cos_light
                    / distance_squared) as f64
                    * area_element;
            }
        }

        // Estimate by sampling.
        const SAMPLES: usize = 40_000;
        let mut estimate = 0.0f64;
        for i in 0..SAMPLES {
            let u = vec2(
                (i as f32 + 0.5) / SAMPLES as f32,
                ((i * 7919) % SAMPLES) as f32 / SAMPLES as f32,
            );
            if let Some(sample) = light.sample(point, u) {
                if sample.pdf > 0.0 {
                    let cos_surface = sample.direction.dot(normal).max(0.0);
                    estimate +=
                        (sample.radiance.luminance() * cos_surface / sample.pdf) as f64;
                }
            }
        }
        estimate /= SAMPLES as f64;

        let error = (estimate - reference).abs() / reference.max(1e-9);
        assert!(
            error < 0.02,
            "sampling gave {estimate:.5} against a reference of {reference:.5} \
             ({:.1}% off)",
            error * 100.0
        );
    }

    /// A sample's reported density must match what `pdf` says about the same
    /// direction, or every MIS weight in the renderer is computed against the
    /// wrong number.
    #[test]
    fn the_reported_density_matches_the_pdf_function() {
        let light = softbox();
        for from in [vec3(0.0, 0.0, 0.0), vec3(0.7, 0.2, -0.3), vec3(-2.0, 0.5, 1.0)] {
            let mut checked = 0;
            for i in 0..64 {
                let u = vec2((i as f32 + 0.5) / 64.0, ((i * 13) % 64) as f32 / 64.0);
                let Some(sample) = light.sample(from, u) else {
                    continue;
                };
                let independent = light.pdf(from, sample.direction);
                assert!(
                    (sample.pdf - independent).abs() <= 0.02 * independent.max(1e-3),
                    "from {from:?}: the sample says {} and pdf() says {independent}",
                    sample.pdf
                );
                checked += 1;
            }
            assert!(checked > 20, "only {checked} samples were usable from {from:?}");
        }
    }

    /// Nothing behind a one-sided emitter is lit by it.
    #[test]
    fn a_one_sided_light_does_not_illuminate_its_own_back() {
        let light = softbox();
        let behind = vec3(0.0, 4.0, 0.0);
        for i in 0..32 {
            let u = vec2((i as f32 + 0.5) / 32.0, 0.5);
            assert!(
                light.sample(behind, u).is_none(),
                "a point above a downward-facing light was lit by it"
            );
        }

        let two_sided = AreaLight {
            two_sided: true,
            ..light
        };
        assert!(
            two_sided.sample(behind, vec2(0.5, 0.5)).is_some(),
            "a two-sided light should light both ways"
        );
    }

    /// A point light falls off with the square of distance, exactly.
    #[test]
    fn a_point_light_obeys_the_inverse_square_law() {
        let light = PointLight {
            position: Vec3::ZERO,
            intensity: Spectrum::splat(100.0),
            radius: 0.0,
        };
        let near = light.sample(vec3(0.0, 0.0, 1.0), vec2(0.5, 0.5)).unwrap();
        let far = light.sample(vec3(0.0, 0.0, 2.0), vec2(0.5, 0.5)).unwrap();

        assert!(
            (near.radiance.luminance() / far.radiance.luminance() - 4.0).abs() < 1e-3,
            "doubling the distance should quarter the irradiance: {} against {}",
            near.radiance.luminance(),
            far.radiance.luminance()
        );
        assert!(near.is_delta, "a zero-radius point light is a delta");
    }

    /// A point light with a radius is no longer a delta, because a BSDF sample
    /// can hit it.
    #[test]
    fn a_point_light_with_a_radius_is_not_a_delta() {
        let light = PointLight {
            position: Vec3::ZERO,
            intensity: Spectrum::splat(100.0),
            radius: 0.1,
        };
        let sample = light.sample(vec3(0.0, 0.0, 2.0), vec2(0.3, 0.7)).unwrap();
        assert!(!sample.is_delta);
    }

    /// The penumbra is smooth-stepped, not linear: a linear ramp leaves a
    /// visible crease at the inner angle because its derivative jumps there.
    #[test]
    fn a_spotlights_penumbra_has_no_crease() {
        let spot = SpotLight {
            position: Vec3::ZERO,
            direction: -Vec3::Y,
            intensity: Spectrum::splat(100.0),
            inner_angle: 0.3,
            outer_angle: 0.5,
            falloff: 1.0,
        };

        assert_eq!(spot.falloff_at(1.0), 1.0, "on axis");
        assert_eq!(spot.falloff_at(0.0), 0.0, "well outside the cone");
        assert_eq!(spot.falloff_at(spot.inner_angle.cos()), 1.0, "at the inner edge");
        assert_eq!(spot.falloff_at(spot.outer_angle.cos()), 0.0, "at the outer edge");

        // The derivative must go to zero at both ends of the penumbra, which is
        // what "no crease" means numerically.
        let step = 1e-4;
        for edge in [spot.inner_angle, spot.outer_angle] {
            let c = edge.cos();
            let slope = (spot.falloff_at(c + step) - spot.falloff_at(c - step)) / (2.0 * step);
            assert!(
                slope.abs() < 0.5,
                "the falloff has a slope of {slope:.3} at the {edge:.2} rad edge; a \
                 linear ramp would leave a visible line there"
            );
        }

        // And it is monotonic across the penumbra.
        let mut previous = 0.0;
        for i in 0..=50 {
            let angle = spot.outer_angle - (spot.outer_angle - spot.inner_angle) * i as f32 / 50.0;
            let value = spot.falloff_at(angle.cos());
            assert!(value >= previous - 1e-6, "the falloff is not monotonic");
            previous = value;
        }
    }

    /// §5.3's threshold: the strategy is chosen from the light count, and
    /// reported.
    #[test]
    fn the_selection_strategy_follows_the_light_count() {
        let make = |n: usize| {
            LightSet::new(
                (0..n)
                    .map(|_| Light::new(LightKind::Area(softbox())))
                    .collect(),
            )
        };
        assert_eq!(make(1).strategy(), SelectionStrategy::Alias);
        assert_eq!(make(HIERARCHY_THRESHOLD).strategy(), SelectionStrategy::Alias);
        assert_eq!(
            make(HIERARCHY_THRESHOLD + 1).strategy(),
            SelectionStrategy::Hierarchy
        );
    }

    /// Selection is power-weighted, and its reported probability is the one it
    /// actually uses — the estimator divides by it, so a mismatch is a
    /// brightness error rather than a noise one.
    #[test]
    fn selection_is_power_weighted_and_reports_its_own_probability() {
        let bright = AreaLight {
            radiance: Spectrum::splat(90.0),
            ..softbox()
        };
        let dim = AreaLight {
            radiance: Spectrum::splat(10.0),
            ..softbox()
        };
        let set = LightSet::new(vec![
            Light::new(LightKind::Area(bright)),
            Light::new(LightKind::Area(dim)),
        ]);

        let mut counts = [0usize; 2];
        const DRAWS: usize = 20_000;
        for i in 0..DRAWS {
            let u = (i as f32 + 0.5) / DRAWS as f32;
            let (id, p) = set.select(u).expect("a light");
            counts[id.0 as usize] += 1;
            assert!(
                (p - set.selection_pdf(id)).abs() < 1e-6,
                "select reported {p} but selection_pdf says {}",
                set.selection_pdf(id)
            );
        }

        let share = counts[0] as f32 / DRAWS as f32;
        assert!(
            (share - 0.9).abs() < 0.02,
            "the nine-times-brighter light was chosen {:.1}% of the time",
            share * 100.0
        );
    }

    /// Selection probabilities must sum to one, or the estimator is biased by
    /// whatever they sum to instead.
    #[test]
    fn selection_probabilities_sum_to_one() {
        for count in [1usize, 2, 3, 7, 8, 9, 32] {
            let set = LightSet::new(
                (0..count)
                    .map(|i| {
                        Light::new(LightKind::Area(AreaLight {
                            radiance: Spectrum::splat(1.0 + i as f32),
                            ..softbox()
                        }))
                    })
                    .collect(),
            );
            let total: f32 = (0..count).map(|i| set.selection_pdf(LightId(i as u32))).sum();
            assert!(
                (total - 1.0).abs() < 1e-4,
                "{count} lights: probabilities sum to {total}"
            );
        }
    }

    /// A light that lights nothing is never chosen, and does not steal
    /// probability from the ones that do.
    #[test]
    fn a_disabled_light_is_never_selected() {
        let mut dark = Light::new(LightKind::Area(softbox()));
        dark.enabled = false;
        let set = LightSet::new(vec![Light::new(LightKind::Area(softbox())), dark]);

        assert_eq!(set.selection_pdf(LightId(1)), 0.0);
        for i in 0..200 {
            let (id, _) = set.select(i as f32 / 200.0).expect("a light");
            assert_eq!(id.0, 0, "a disabled light was selected");
        }
    }

    #[test]
    fn an_empty_light_set_selects_nothing() {
        let set = LightSet::new(Vec::new());
        assert!(set.is_empty());
        assert!(set.select(0.5).is_none());
    }

    /// §5.4's flags: a light hidden from the camera still lights the scene.
    #[test]
    fn a_hidden_light_still_lights() {
        let light = Light::new(LightKind::Area(softbox())).with_visibility(Visibility::HIDDEN);
        assert!(light.power() > 0.0, "a hidden light should still be chosen");
        assert!(!light.visibility.camera);
        assert!(light.visibility.lights_anything());
    }
}
