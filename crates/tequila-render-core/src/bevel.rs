//! Rounded edges in Tequila's renderer, as Cycles' Bevel node rounds them:
//! the shading normal blended with those of the faces within the edge
//! radius, so a sharp CAD edge catches light as a machined one does, with
//! the geometry left as it is (`edge_radius`, `docs/02`).
//!
//! Each sample takes a point on a disc of that radius around the shading
//! point, across one of its three axes - the normal half the time, either
//! tangent a quarter - and casts a short ray through the disc from each
//! side. What it meets of the same part, near enough, lends its normal,
//! weighted down with its distance. A flat face meets only itself and keeps
//! its normal; near an edge the neighbouring face leans it over.

use crate::intersect::{Hit, Intersector};
use tequila_core::math::{Onb, Ray, Vec3};
use tequila_core::sampler::Sampler;

/// The normal at `hit` rounded over `radius`, from `samples` samples.
pub fn rounded_normal(intersector: &dyn Intersector, hit: &Hit, normal: Vec3, radius: f32, samples: u32, sampler: &mut Sampler) -> Vec3 {
    let frame = Onb::from_normal(normal);
    let sigma = radius * 0.5;
    let mut sum = normal * 1e-3;
    for _ in 0..samples.max(1) {
        let pick = sampler.next_unstructured();
        let (axis, across, other) = if pick < 0.5 {
            (frame.n, frame.t, frame.b)
        } else if pick < 0.75 {
            (frame.t, frame.b, frame.n)
        } else {
            (frame.b, frame.n, frame.t)
        };
        let r = radius * sampler.next_unstructured().sqrt();
        let phi = std::f32::consts::TAU * sampler.next_unstructured();
        let height = (radius * radius - r * r).max(0.0).sqrt().max(radius * 1e-3);
        let offset = (across * phi.cos() + other * phi.sin()) * r;
        for side in [1.0f32, -1.0] {
            let start = hit.position + offset + axis * (height * side);
            let mut ray = Ray::new(start, axis * -side);
            ray.t_max = 2.0 * height;
            let Some(near) = intersector.intersect(&ray) else { continue };
            if near.instance != hit.instance {
                continue;
            }
            // The far side of a thin part leans the other way: not an edge.
            let seen = near.shading_normal;
            if seen.dot(normal) < -0.1 {
                continue;
            }
            let distance = (near.position - hit.position).length();
            let weight = (-(distance * distance) / (2.0 * sigma * sigma)).exp();
            sum += seen * weight;
        }
    }
    sum.try_normalize().unwrap_or(normal)
}
