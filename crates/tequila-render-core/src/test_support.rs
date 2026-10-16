//! Analytic primitives and an intersector built on them.
//!
//! `docs/05-ROADMAP-AND-TESTING.md` §3 asks for correctness tests with a
//! *mathematical* answer, not a golden image. That needs geometry whose
//! intersection is exact — a sphere and a plane have closed forms, so a test
//! scene built from them introduces no tessellation error to be confused with
//! a transport bug.
//!
//! It also decouples the transport tests from the acceleration structure
//! entirely: a furnace or MIS failure here is unambiguously the integrator's,
//! never the BVH's.
//!
//! Compiled into the library rather than hidden behind `#[cfg(test)]` so that
//! the integration tests in `tests/analytic` can use it too.

use crate::intersect::{Hit, Intersector};
use crate::scene::SceneSnapshot;
use tequila_core::ids::{InstanceId, MaterialId};
use tequila_core::math::{solve_quadratic, Ray, Vec2, Vec3};

/// An analytic sphere.
#[derive(Clone, Copy, Debug)]
pub struct Sphere {
    pub center: Vec3,
    pub radius: f32,
    pub material: MaterialId,
}

impl Sphere {
    pub fn new(center: Vec3, radius: f32, material: MaterialId) -> Self {
        Sphere {
            center,
            radius,
            material,
        }
    }

    fn intersect(&self, ray: &Ray) -> Option<f32> {
        let oc = ray.origin - self.center;
        let a = ray.direction.dot(ray.direction);
        let b = 2.0 * oc.dot(ray.direction);
        let c = oc.dot(oc) - self.radius * self.radius;
        let (t0, t1) = solve_quadratic(a, b, c)?;
        // The nearer root first, so a ray starting inside the sphere still
        // reports the far wall rather than nothing.
        [t0, t1]
            .into_iter()
            .find(|&t| t > ray.t_min && t < ray.t_max)
    }
}

/// An infinite analytic plane.
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub point: Vec3,
    pub normal: Vec3,
    pub material: MaterialId,
}

impl Plane {
    pub fn new(point: Vec3, normal: Vec3, material: MaterialId) -> Self {
        Plane {
            point,
            normal: normal.normalize(),
            material,
        }
    }

    fn intersect(&self, ray: &Ray) -> Option<f32> {
        let denom = ray.direction.dot(self.normal);
        if denom.abs() < 1e-8 {
            return None; // parallel
        }
        let t = (self.point - ray.origin).dot(self.normal) / denom;
        if t > ray.t_min && t < ray.t_max {
            Some(t)
        } else {
            None
        }
    }
}

/// A brute-force intersector over analytic primitives.
#[derive(Clone, Debug)]
pub struct AnalyticIntersector {
    scene: SceneSnapshot,
    spheres: Vec<Sphere>,
    planes: Vec<Plane>,
}

impl AnalyticIntersector {
    pub fn new(scene: SceneSnapshot) -> Self {
        AnalyticIntersector {
            scene,
            spheres: Vec::new(),
            planes: Vec::new(),
        }
    }

    pub fn with_sphere(mut self, sphere: Sphere) -> Self {
        self.spheres.push(sphere);
        self
    }

    pub fn with_plane(mut self, plane: Plane) -> Self {
        self.planes.push(plane);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.spheres.is_empty() && self.planes.is_empty()
    }
}

impl Intersector for AnalyticIntersector {
    fn intersect(&self, ray: &Ray) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        let mut closest = ray.t_max;

        for (i, sphere) in self.spheres.iter().enumerate() {
            let mut probe = *ray;
            probe.t_max = closest;
            if let Some(t) = sphere.intersect(&probe) {
                let position = ray.at(t);
                let normal = ((position - sphere.center) / sphere.radius)
                    .try_normalize()
                    .unwrap_or(Vec3::Y);
                closest = t;
                best = Some(Hit {
                    t,
                    position,
                    geometric_normal: normal,
                    shading_normal: normal,
                    uv: Vec2::ZERO,
                    instance: InstanceId(i as u32),
                    material: sphere.material,
                    primitive: 0,
                });
            }
        }

        for (i, plane) in self.planes.iter().enumerate() {
            let mut probe = *ray;
            probe.t_max = closest;
            if let Some(t) = plane.intersect(&probe) {
                closest = t;
                best = Some(Hit {
                    t,
                    position: ray.at(t),
                    geometric_normal: plane.normal,
                    shading_normal: plane.normal,
                    uv: Vec2::ZERO,
                    instance: InstanceId((self.spheres.len() + i) as u32),
                    material: plane.material,
                    primitive: 0,
                });
            }
        }

        best
    }

    fn occluded(&self, from: Vec3, from_normal: Vec3, to: Vec3, to_normal: Vec3) -> bool {
        let mut ray = crate::intersect::shadow_ray(from, from_normal, to, to_normal);
        // Primitive `i` is instance `i` when the scene lists one, and a part
        // whose shadow is switched off is passed rather than met.
        while let Some(hit) = self.intersect(&ray) {
            if self.scene.casts_shadows(hit.instance) {
                return true;
            }
            ray.t_min = hit.t * (1.0 + 1e-5) + 1e-5;
        }
        false
    }

    fn scene(&self) -> &SceneSnapshot {
        &self.scene
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::Environment;
    use crate::material::Material;
    use tequila_core::math::vec3;
    use tequila_core::spectrum::Spectrum;
    use std::sync::Arc;

    fn empty_scene() -> SceneSnapshot {
        SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![Material::default()],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        )
    }

    #[test]
    fn sphere_intersection_is_exact() {
        let i = AnalyticIntersector::new(empty_scene()).with_sphere(Sphere::new(
            Vec3::ZERO,
            2.0,
            MaterialId(0),
        ));
        let hit = i
            .intersect(&Ray::new(vec3(0.0, 0.0, 10.0), vec3(0.0, 0.0, -1.0)))
            .expect("ray must hit the sphere");
        assert!((hit.t - 8.0).abs() < 1e-4, "t = {}", hit.t);
        assert!((hit.position - vec3(0.0, 0.0, 2.0)).length() < 1e-4);
        assert!((hit.geometric_normal - vec3(0.0, 0.0, 1.0)).length() < 1e-4);
    }

    #[test]
    fn rays_starting_inside_a_sphere_exit_through_the_far_side() {
        let i = AnalyticIntersector::new(empty_scene()).with_sphere(Sphere::new(
            Vec3::ZERO,
            2.0,
            MaterialId(0),
        ));
        let hit = i
            .intersect(&Ray::new(Vec3::ZERO, vec3(1.0, 0.0, 0.0)))
            .expect("must hit the far wall");
        assert!((hit.t - 2.0).abs() < 1e-4);
    }

    #[test]
    fn misses_report_nothing() {
        let i = AnalyticIntersector::new(empty_scene()).with_sphere(Sphere::new(
            Vec3::ZERO,
            1.0,
            MaterialId(0),
        ));
        assert!(i
            .intersect(&Ray::new(vec3(0.0, 5.0, 10.0), vec3(0.0, 0.0, -1.0)))
            .is_none());
        // And a ray pointing away from the sphere.
        assert!(i
            .intersect(&Ray::new(vec3(0.0, 0.0, 10.0), vec3(0.0, 0.0, 1.0)))
            .is_none());
    }

    #[test]
    fn planes_reject_parallel_rays() {
        let i = AnalyticIntersector::new(empty_scene()).with_plane(Plane::new(
            Vec3::ZERO,
            Vec3::Y,
            MaterialId(0),
        ));
        assert!(i
            .intersect(&Ray::new(vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0)))
            .is_none());
        let hit = i
            .intersect(&Ray::new(vec3(0.0, 1.0, 0.0), vec3(0.0, -1.0, 0.0)))
            .unwrap();
        assert!((hit.t - 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_nearest_primitive_wins() {
        let i = AnalyticIntersector::new(empty_scene())
            .with_sphere(Sphere::new(vec3(0.0, 0.0, 0.0), 1.0, MaterialId(0)))
            .with_sphere(Sphere::new(vec3(0.0, 0.0, 5.0), 1.0, MaterialId(0)))
            .with_plane(Plane::new(vec3(0.0, 0.0, -20.0), Vec3::Z, MaterialId(0)));
        let hit = i
            .intersect(&Ray::new(vec3(0.0, 0.0, 20.0), vec3(0.0, 0.0, -1.0)))
            .unwrap();
        // The nearer sphere is at z = 6.
        assert!((hit.t - 14.0).abs() < 1e-4, "t = {}", hit.t);
    }

    #[test]
    fn occlusion_matches_intersection() {
        let i = AnalyticIntersector::new(empty_scene()).with_sphere(Sphere::new(
            vec3(0.0, 2.0, 0.0),
            0.5,
            MaterialId(0),
        ));
        assert!(i.occluded(Vec3::ZERO, Vec3::Y, vec3(0.0, 5.0, 0.0), -Vec3::Y));
        assert!(!i.occluded(Vec3::ZERO, Vec3::Y, vec3(5.0, 0.1, 0.0), -Vec3::Y));
    }

    /// A shadow ray aimed at a surface must not report that surface as its own
    /// occluder. This is the offsetting bug that turns every light black.
    #[test]
    fn a_surface_does_not_shadow_itself() {
        let i = AnalyticIntersector::new(empty_scene()).with_plane(Plane::new(
            Vec3::ZERO,
            Vec3::Y,
            MaterialId(0),
        ));
        assert!(!i.occluded(Vec3::ZERO, Vec3::Y, vec3(0.0, 10.0, 0.0), -Vec3::Y));
        // Also at a large scene scale, where a fixed epsilon would fail.
        let far = vec3(1.0e5, 0.0, 1.0e5);
        assert!(!i.occluded(far, Vec3::Y, far + vec3(0.0, 100.0, 0.0), -Vec3::Y));
    }
}
