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
    fn occluded(&self, from: Vec3, from_normal: Vec3, to: Vec3, to_normal: Vec3) -> bool;

    /// The scene this intersector was built for.
    fn scene(&self) -> &SceneSnapshot;
}

/// Offsets and shortens a shadow ray so it neither self-intersects at its
/// origin nor hits the light it is aimed at.
///
/// Both ends need the treatment: offsetting only the origin leaves the ray
/// terminating exactly on the light's surface, where it reliably reports an
/// occlusion against the light itself and turns every light black.
pub fn shadow_ray(from: Vec3, from_normal: Vec3, to: Vec3, to_normal: Vec3) -> Ray {
    let origin = tequila_core::math::offset_ray_origin(from, from_normal);
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
        assert!(ray.origin.y > 0.0, "origin not offset off the surface");
        assert!((ray.direction - Vec3::Y).length() < 1e-4);
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
        // spacing; the offset has to scale (see `offset_ray_origin`).
        let far = vec3(1.0e5, 1.0e5, 1.0e5);
        let ray = shadow_ray(far, Vec3::Y, far + vec3(0.0, 100.0, 0.0), -Vec3::Y);
        assert!(ray.origin.y > far.y, "no offset applied at scale");
        // The offset is a fixed number of ULPs, so its absolute size grows
        // with the coordinate — that is the whole point of the method. What
        // must stay small is its size *relative* to the scene.
        let relative = (ray.origin.y - far.y) / far.y;
        assert!(relative < 1e-4, "offset too coarse at scale: {relative}");
    }
}
