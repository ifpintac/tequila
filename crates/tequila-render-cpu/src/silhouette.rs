//! The product's outline, for an image cut out along it.
//!
//! A product photograph for a catalogue is cut out by its silhouette — a
//! clipping path drawn around the object — and everything inside the path
//! is kept, glass included, with whatever the glass shows of the studio
//! behind it. Alpha taken from the render's own coverage does something
//! else: glass a fifth present is a fifth opaque, so a lightbulb cut out
//! that way comes out a ghost of itself.
//!
//! This casts the camera's rays — the same film points, through the same
//! pixel filter, as the render — against the scene's geometry alone, with
//! no shading: a ray that meets anything the camera can see, other than a
//! shadow catcher, is inside the outline. It is cheap beside the render,
//! and it does not care which engine made the picture.

use tequila_core::math::Vec2;
use tequila_core::sampler::Sampler;
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::Camera;
use tequila_core::math::Ray;
use tequila_render_core::intersect::Intersector;
use rayon::prelude::*;

/// Crossings of camera-hidden parts a ray may make before it gives up.
const MAX_HIDDEN_CROSSINGS: usize = 64;

/// How much of each pixel is inside the outline, and what the background
/// shows in the part that is not.
#[derive(Debug)]
pub struct Silhouette {
    pub width: usize,
    pub height: usize,
    /// 0 to 1 per pixel.
    pub coverage: Vec<f32>,
    /// The background the outside of each pixel shows, already weighted by
    /// how much of the pixel is outside: what to take away from a rendered
    /// pixel to leave only the product's share of it.
    pub outside: Vec<Spectrum>,
}

impl Silhouette {
    /// Cast `samples` rays a pixel from `camera` at `width` by `height`.
    pub fn trace(intersector: &dyn Intersector, camera: &Camera, width: usize, height: usize, samples: u32, seed: u32) -> Silhouette {
        let scene = intersector.scene();
        let aspect = width as f32 / height.max(1) as f32;
        let samples = samples.max(1);
        let rows: Vec<(Vec<f32>, Vec<Spectrum>)> = (0..height)
            .into_par_iter()
            .map(|y| {
                let mut coverage = Vec::with_capacity(width);
                let mut outside = Vec::with_capacity(width);
                for x in 0..width {
                    let mut inside = 0u32;
                    let mut background = Spectrum::ZERO;
                    for s in 0..samples {
                        let mut sampler = Sampler::new(x as u32, y as u32, s, seed);
                        let jitter = sampler.pixel_jitter();
                        let film = Vec2 { x: (x as f32 + jitter.x) / width as f32, y: (y as f32 + jitter.y) / height as f32 };
                        let ray = camera.generate_ray(film, aspect, &mut sampler);
                        if meets_product(intersector, ray) {
                            inside += 1;
                        } else {
                            background += scene.background.radiance(&scene.environment, ray.origin, ray.direction, film);
                        }
                    }
                    coverage.push(inside as f32 / samples as f32);
                    outside.push(background * (1.0 / samples as f32));
                }
                (coverage, outside)
            })
            .collect();
        let mut silhouette = Silhouette { width, height, coverage: Vec::with_capacity(width * height), outside: Vec::with_capacity(width * height) };
        for (coverage, outside) in rows {
            silhouette.coverage.extend(coverage);
            silhouette.outside.extend(outside);
        }
        silhouette
    }

    /// `color`, the finished picture with its background, cut out: the
    /// colour premultiplied by the outline's coverage, and that coverage.
    /// Inside the outline nothing changes — the glass keeps what it shows of
    /// the studio — and at its edge the background's share comes out.
    pub fn cut(&self, color: &[Spectrum]) -> (Vec<Spectrum>, Vec<f32>) {
        let premultiplied = color
            .iter()
            .zip(&self.outside)
            .zip(&self.coverage)
            .map(|((c, outside), coverage)| {
                if *coverage <= 0.0 {
                    return Spectrum::ZERO;
                }
                let kept = *c - *outside;
                Spectrum { r: kept.r.max(0.0), g: kept.g.max(0.0), b: kept.b.max(0.0) }
            })
            .collect();
        (premultiplied, self.coverage.clone())
    }
}

/// Whether a camera ray meets something the camera can see, other than a
/// shadow catcher, before it leaves the scene.
fn meets_product(intersector: &dyn Intersector, mut ray: Ray) -> bool {
    let scene = intersector.scene();
    for _ in 0..MAX_HIDDEN_CROSSINGS {
        let Some(hit) = intersector.intersect(&ray) else { return false };
        match scene.instances.get(hit.instance.0 as usize) {
            Some(instance) if !instance.visible_to_camera || instance.shadow_catcher.is_some() => {
                let (geometric, _) = hit.facing_forward(ray.direction);
                ray = Ray::new(tequila_core::math::offset_ray_origin(hit.position, -geometric), ray.direction);
            }
            _ => return true,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Device, EmbreeIntersector};
    use tequila_core::ids::{InstanceId, MaterialId, MeshId};
    use tequila_core::math::{vec3, Vec3};
    use tequila_materials::material_type::MaterialType;
    use tequila_render_core::scene::{Instance, SceneSnapshot};
    use std::sync::Arc;

    /// Glass all but absent is inside the outline all the same: a sphere of
    /// opacity 0.1 covers the middle of the frame fully, and nothing
    /// outside it.
    #[test]
    fn faint_glass_is_inside_the_outline() {
        let mut material = MaterialType::Plastic.defaults();
        material.geometry_opacity = 0.1;
        let scene = SceneSnapshot::new(
            vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 32, 16))],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
            vec![material],
            Arc::new(tequila_scene::demo::studio_environment()),
        );
        let device = Device::shared().expect("an Embree device");
        let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
        let camera = Camera::look_at(vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let silhouette = Silhouette::trace(&intersector, &camera, 40, 40, 8, 0);
        assert_eq!(silhouette.coverage[20 * 40 + 20], 1.0, "the middle is inside");
        assert_eq!(silhouette.coverage[1 * 40 + 1], 0.0, "a corner is outside");
        assert!(silhouette.outside[1 * 40 + 1].luminance() > 0.0, "and shows the background");

        // Cutting keeps the middle as rendered and drops the corner.
        let color = vec![Spectrum::splat(0.5); 40 * 40];
        let (cut, alpha) = silhouette.cut(&color);
        assert_eq!((cut[20 * 40 + 20], alpha[20 * 40 + 20]), (Spectrum::splat(0.5), 1.0));
        assert_eq!(alpha[1 * 40 + 1], 0.0);
    }
}
