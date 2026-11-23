//! The intersection interface the integrator is written against.
//!
//! `docs/01-RENDERER-CORE.md` §10.1: the *integrator* may legitimately differ
//! between backends — wavefront on GPU, megakernel on CPU — because the
//! integrator is control flow, not math. This trait is the seam that lets one
//! integrator run over Embree on the CPU (`tequila-render-cpu`) and over
//! hardware ray tracing on the GPU (`tequila-render-gpu`) without duplicating
//! the transport logic.
//!
//! It also makes the analytic tests of `docs/05` §3 possible: a test can
//! supply an intersector with a closed-form answer and check the integrator
//! against it, with no acceleration structure involved at all.

use crate::scene::SceneSnapshot;
use tequila_core::ids::{InstanceId, MaterialId};
use tequila_core::math::{Ray, Vec2, Vec3};

/// Everything the integrator needs about a surface hit.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Distance along the ray.
    pub t: f32,
    /// World-space hit position.
    pub position: Vec3,
    /// How large the numbers were that `position` was worked out from: its
    /// triangle's corners as its part places them and where it places them,
    /// or a ray's origin and length. The point is only as precise as they
    /// are, so a ray leaving it starts that much further off the surface not
    /// to meet it again ([`Hit::offset`]). Zero for a point as precise as its
    /// own coordinates.
    pub magnitude: f32,
    /// **Geometric** normal: the true triangle normal, unit length, oriented
    /// as the winding gives it. Used for ray offsetting and for rejecting
    /// light samples on the wrong side.
    pub geometric_normal: Vec3,
    /// **Shading** normal: interpolated and/or bump-mapped, unit length. Used
    /// for BSDF evaluation.
    ///
    /// Keeping the two distinct is not pedantry. §3.1 requires guarding the
    /// classic shading-normal energy leak — light arriving below the geometric
    /// horizon but above the shading one — and that guard needs both.
    pub shading_normal: Vec3,
    pub uv: Vec2,
    pub instance: InstanceId,
    pub material: MaterialId,
    /// Index of the hit triangle within its mesh.
    pub primitive: u32,
}

impl Hit {
    /// Where a ray leaving the surface on the side `normal` points to starts:
    /// just off it, by as much as the point's precision asks.
    #[inline]
    pub fn offset(&self, normal: Vec3) -> Vec3 {
        tequila_core::math::offset_ray_origin_at(self.position, normal, self.magnitude)
    }

    /// True when the ray struck the back face, judged by the geometric normal.
    #[inline]
    pub fn is_backface(&self, direction: Vec3) -> bool {
        direction.dot(self.geometric_normal) > 0.0
    }

    /// Normals flipped to face the incoming ray, so that shading always works
    /// in the hemisphere the ray came from.
    #[inline]
    pub fn facing_forward(&self, direction: Vec3) -> (Vec3, Vec3) {
        if self.is_backface(direction) {
            (-self.geometric_normal, -self.shading_normal)
        } else {
            (self.geometric_normal, self.shading_normal)
        }
    }
}

/// A ray-tracing backend over one [`SceneSnapshot`].
pub trait Intersector: Send + Sync {
    /// Closest-hit query.
    fn intersect(&self, ray: &Ray) -> Option<Hit>;

    /// Any-hit visibility query between two points.
    ///
    /// Separate from [`Self::intersect`] because it can stop at the first hit
    /// rather than finding the nearest, which is roughly twice as fast and is
    /// the single most frequent query a path tracer makes.
    ///
    /// An instance with `casts_shadows` off does not occlude, though
    /// [`Self::intersect`] still meets it: its shadow is switched off, not the
    /// part.
    ///
    /// `from` is where the ray starts, already stepped off the surface it
    /// leaves on the side `from_normal` points to, as [`shadow_ray`] takes
    /// it; `to` is stepped off its own surface along `to_normal` here.
    fn occluded(&self, from: Vec3, from_normal: Vec3, to: Vec3, to_normal: Vec3) -> bool;

    /// [`Self::occluded`] past the shadow-catching floors the bounces pass
    /// ([`SceneSnapshot::floor_passes`]): the sky's light sample where it
    /// sees past them (`integrator::sky_past_floors`). A backend that can
    /// mask such floors out of an any-hit query overrides this walk.
    fn occluded_past_floors(&self, from: Vec3, from_normal: Vec3, to: Vec3, to_normal: Vec3) -> bool {
        let scene = self.scene();
        let mut ray = shadow_ray(from, from_normal, to, to_normal);
        loop {
            let Some(hit) = self.intersect(&ray) else { return false };
            if scene.casts_shadows(hit.instance) && !scene.floor_passes(hit.instance) {
                return true;
            }
            ray.t_min = crate::cutaway::step_past(&ray, hit.t);
        }
    }

    /// The scene this intersector was built for.
    fn scene(&self) -> &SceneSnapshot;
}

/// A shadow ray from `from` to just short of `to`, so it neither meets the
/// surface it leaves nor the light it is aimed at.
///
/// `from` is taken as it is: the caller steps it off its surface, once, on
/// the side `from_normal` points to - from a hit by [`Hit::offset`], sized to
/// the numbers the hit was worked out from, which this cannot know. It used
/// to be stepped off again here, by the step for its coordinates alone, and
/// the two came to 2.3 µm a hand's length from the world's origin: a shadow
/// ray from paint 2 µm behind a thin lens started past the lens and missed
/// it. `from_normal` is still the way the ray goes should the two ends meet.
///
/// The far end is stepped off its surface along `to_normal` and the ray
/// stops short of it: ending exactly on the light's surface, the ray
/// reliably reports an occlusion against the light itself and turns every
/// light black.
pub fn shadow_ray(from: Vec3, from_normal: Vec3, to: Vec3, to_normal: Vec3) -> Ray {
    let origin = from;
    let target = tequila_core::math::offset_ray_origin(to, to_normal);
    let delta = target - origin;
    let distance = delta.length();
    Ray {
        origin,
        direction: if distance > 0.0 {
            delta / distance
        } else {
            from_normal
        },
        t_min: 0.0,
        // Stop just short of the target.
        t_max: distance * (1.0 - 1e-4),
        time: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    fn hit_facing_up() -> Hit {
        Hit {
            t: 1.0,
            position: Vec3::ZERO,
            magnitude: 0.0,
            geometric_normal: Vec3::Y,
            shading_normal: vec3(0.1, 0.99, 0.0).normalize(),
            uv: Vec2::ZERO,
            instance: InstanceId(0),
            material: MaterialId(0),
            primitive: 0,
        }
    }

    #[test]
    fn backfaces_are_detected_by_the_geometric_normal() {
        let h = hit_facing_up();
        assert!(!h.is_backface(vec3(0.0, -1.0, 0.0)));
        assert!(h.is_backface(vec3(0.0, 1.0, 0.0)));
    }

    #[test]
    fn facing_forward_flips_both_normals_together() {
        let h = hit_facing_up();
        let (ng, ns) = h.facing_forward(vec3(0.0, 1.0, 0.0));
        assert!(ng.y < 0.0 && ns.y < 0.0, "normals must flip as a pair");
        let (ng, ns) = h.facing_forward(vec3(0.0, -1.0, 0.0));
        assert_eq!(ng, h.geometric_normal);
        assert_eq!(ns, h.shading_normal);
    }

    /// The shadow ray must stop short of its target, or it registers a hit
    /// against the very light it is testing.
    #[test]
    fn shadow_rays_stop_short_of_the_target() {
        let ray = shadow_ray(vec3(0.0, 0.0, 0.0), Vec3::Y, vec3(0.0, 10.0, 0.0), -Vec3::Y);
        assert!(ray.t_max < 10.0, "ray reaches the light: {}", ray.t_max);
        assert!(ray.t_max > 9.99, "ray falls unreasonably short");
        assert!((ray.direction - Vec3::Y).length() < 1e-4);
    }

    /// The start is stepped off its surface once, by the caller, and not
    /// again here: a hand's length from the origin the second step took a
    /// shadow ray from paint past a lens 2 µm in front of it.
    #[test]
    fn shadow_rays_start_where_they_are_put() {
        let hit = Hit { position: vec3(0.0, 0.1, 0.3), ..hit_facing_up() };
        let from = hit.offset(Vec3::Z);
        let ray = shadow_ray(from, Vec3::Z, vec3(0.0, 0.1, 1.0), -Vec3::Z);
        assert_eq!(ray.origin, from, "the start was stepped off its surface again");
        assert!(ray.origin.z - 0.3 < 1.5e-6, "one step is {} m", ray.origin.z - 0.3);
    }

    #[test]
    fn shadow_rays_survive_coincident_endpoints() {
        // Degenerate but reachable: a light sample landing on the shading
        // point itself must not produce a NaN direction.
        let ray = shadow_ray(Vec3::ZERO, Vec3::Y, Vec3::ZERO, Vec3::Y);
        assert!(ray.direction.is_finite());
        assert!((ray.direction.length() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn shadow_rays_scale_their_offset_with_the_scene() {
        // At large coordinates a fixed epsilon would vanish into the float
        // spacing; the far end's step has to scale (see `offset_ray_origin`).
        let far = vec3(1.0e5, 1.0e5, 1.0e5);
        let light = far + vec3(0.0, 100.0, 0.0);
        let ray = shadow_ray(far, Vec3::Y, light, -Vec3::Y);
        let end = ray.at(ray.t_max);
        assert!(end.y < light.y, "the ray reaches the light at scale");
        // The step is a fixed number of ULPs, so its absolute size grows
        // with the coordinate — that is the whole point of the method. What
        // must stay small is its size *relative* to the scene.
        let relative = (light.y - end.y) / light.y;
        assert!(relative < 1e-4, "offset too coarse at scale: {relative}");
    }
}
