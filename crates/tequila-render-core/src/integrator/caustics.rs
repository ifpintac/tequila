//! Caustics: the light a glass, a gem or a mirror focuses onto what is
//! around it (`docs/01` §6.5).
//!
//! A path from the camera finds that light only by chance. From a table under
//! a glass, it would have to bounce off the table *into* the glass along one
//! of the few directions that come out at a light, and light sampling cannot
//! help because the shadow ray stops at the glass. So the light is traced the
//! other way, as §6.5 asks: photons leave the lights, pass through the
//! surfaces that focus light (glass, liquids, gems, metal — smooth or nearly
//! so, with nothing matte about them), and are kept where they first land on
//! anything else. A camera path reads them at the first surface it sees that
//! does not focus light.
//!
//! Only those paths — light, one or more focusing events, a surface — are the
//! photons'. Everything else stays the path tracer's, and the path tracer
//! leaves exactly those out: past the surface where it read the photons, a
//! path that reaches the sky through focusing surfaces alone is not counted,
//! and a focusing surface there does not sample the lights
//! (`trace_path_with_caustics`). Shadow rays already stop at glass, so light
//! sampling from the reading surface never counted them.
//!
//! Each progressive pass traces a fresh set of photons and reads them with a
//! radius that shrinks from pass to pass (Knaus and Zwicker's probabilistic
//! progressive photon mapping, the independent-pass form of §6.5's SPPM):
//! each pass's image is a biased estimate whose bias falls as the radius
//! does, and the framebuffer's running mean of them converges.
//!
//! Photons are aimed at the mirror-smooth objects, not spread over the scene.
//! A photon that meets anything else first is not a caustic photon and would
//! be thrown away, so the ones that miss the smooth objects' bounding sphere
//! were never going to count: aiming at it costs no light and wastes none of
//! the budget.

use super::*;
use tequila_core::math::{Aabb, Vec3};

/// Photons traced per pass.
///
/// §6.5 maps a Caustics Quality of 1 to 50 onto 100 000 photons each; this
/// is quality 1. The radius shrinks every pass, so more passes are what
/// sharpen a caustic, and a viewport pass is not held up.
pub const PHOTONS_PER_PASS: u32 = 100_000;

/// Progressive radius reduction, §6.5's α.
const ALPHA: f32 = 0.7;

/// The first pass's radius, as a fraction of the smooth objects' bounding
/// radius. Around thirty photons fall inside it where a caustic is spread
/// evenly over the smooth objects' footprint.
const FIRST_RADIUS: f32 = 0.03;

/// A photon, where it landed.
#[derive(Clone, Copy, Debug)]
pub struct Photon {
    pub position: Vec3,
    /// The surface's normal on the side the photon arrived from.
    pub normal: Vec3,
    /// Towards where it came from.
    pub from: Vec3,
    pub power: Spectrum,
}

/// Where photons come from, and how likely each source is to be chosen.
#[derive(Clone, Debug)]
pub struct CausticPlan {
    /// The bounding sphere of every object that focuses light.
    pub centre: Vec3,
    pub radius: f32,
    sources: Vec<Source>,
    /// Running sum of the sources' share, ending at 1.
    cdf: Vec<f32>,
    /// How far out an environment photon starts.
    far: f32,
}

#[derive(Clone, Copy, Debug)]
enum Source {
    Environment,
    Light(usize),
}

/// The roughest surface that still focuses light, rather than spreading it.
///
/// Not only the mirror-smooth: glass and liquids from a CAD or OBJ export
/// usually carry some roughness — the reference bottle's is 0.24, OBJ's
/// shininess 32 — and a caustic from glass that is not quite smooth is
/// softer, not absent.
pub const FOCUSING_ROUGHNESS: f32 = 0.3;

/// Whether a surface focuses light: every lobe it has is a reflection or a
/// refraction (glass, liquid, gem, metal), smooth or nearly so. What photons
/// pass through, and what camera paths read past.
pub fn caustic_surface(material: &crate::material::Material, bsdf: &impl Bsdf) -> bool {
    let flags = bsdf.flags();
    if flags.is_pure_delta() {
        return true;
    }
    !flags.intersects(BsdfFlags::DIFFUSE | BsdfFlags::SUBSURFACE)
        && material.specular_roughness <= FOCUSING_ROUGHNESS
}

/// Plan a scene's photons: `None` when there is nothing mirror-smooth to make
/// a caustic, or no light to make one with.
pub fn plan(intersector: &dyn Intersector) -> Option<CausticPlan> {
    let scene = intersector.scene();
    let mut bounds = Aabb::EMPTY;
    for instance in &scene.instances {
        if instance.shadow_catcher.is_some() || instance.light_proxy || scene.mesh(instance.mesh).fibres.is_some() {
            continue;
        }
        let material = scene.material(instance.material);
        if !caustic_surface(material, &material.bsdf(0.0)) {
            continue;
        }
        let local = scene.mesh(instance.mesh).bounds();
        if !local.is_empty() {
            bounds.extend(&local.transformed(&instance.transform));
        }
    }
    if bounds.is_empty() {
        return None;
    }
    let centre = bounds.center();
    let radius = bounds.bounding_radius().max(1e-4);
    let area = std::f32::consts::PI * radius * radius;

    // Each source's share, by the light it sends through the sphere's cross
    // section: a few fixed samples each, which only have to be about right —
    // the share is divided out of every photon it chooses.
    let mut sources = Vec::new();
    let mut shares = Vec::new();
    const PILOT: u32 = 64;
    let pilot = |i: u32| {
        tequila_core::math::vec2(
            (i as f32 + 0.5) / PILOT as f32,
            ((i * 37 + 11) % PILOT) as f32 / PILOT as f32 + 0.5 / PILOT as f32,
        )
    };
    let mut environment = 0.0f32;
    for i in 0..PILOT {
        if let Some(sample) = scene.environment.sample_from(centre, pilot(i)) {
            if sample.pdf > 0.0 {
                environment += sample.radiance.luminance() / sample.pdf;
            }
        }
    }
    let environment = environment / PILOT as f32 * area;
    if environment > 0.0 && environment.is_finite() {
        sources.push(Source::Environment);
        shares.push(environment);
    }
    for (index, light) in scene.lights.lights().iter().enumerate() {
        if !light.enabled || !light.visibility.lights_anything() {
            continue;
        }
        let mut flux = 0.0f32;
        for i in 0..PILOT {
            if let Some(sample) = light.sample(centre, pilot(i)) {
                let density = if sample.is_delta { 1.0 } else { sample.pdf };
                if density > 0.0 {
                    flux += sample.radiance.luminance() / density;
                }
            }
        }
        let flux = flux / PILOT as f32 * area;
        if flux > 0.0 && flux.is_finite() {
            sources.push(Source::Light(index));
            shares.push(flux);
        }
    }
    let total: f32 = shares.iter().sum();
    if sources.is_empty() || total <= 0.0 {
        return None;
    }
    let mut running = 0.0;
    let cdf = shares
        .iter()
        .map(|share| {
            running += share / total;
            running
        })
        .collect();

    let scene_bounds = scene.bounds();
    let far = (scene_bounds.bounding_radius() + (scene_bounds.center() - centre).length() + radius) * 1.5 + 1e-3;
    Some(CausticPlan { centre, radius, sources, cdf, far })
}

impl CausticPlan {
    /// The radius photons are read within on pass `pass` (from 0).
    pub fn radius(&self, pass: u32) -> f32 {
        let first = self.radius * FIRST_RADIUS;
        first * ((pass + 1) as f32).powf((ALPHA - 1.0) * 0.5)
    }

    fn choose(&self, u: f32) -> (Source, f32) {
        let index = self.cdf.iter().position(|&c| u < c).unwrap_or(self.cdf.len() - 1);
        let previous = if index == 0 { 0.0 } else { self.cdf[index - 1] };
        (self.sources[index], (self.cdf[index] - previous).max(1e-9))
    }
}

/// A point on a disc of radius `radius` about `centre`, facing `normal`.
fn on_disc(centre: Vec3, normal: Vec3, radius: f32, u: tequila_core::math::Vec2) -> Vec3 {
    let frame = Onb::from_normal(normal);
    let d = tequila_core::sampling::sample_uniform_disk_concentric(u);
    centre + frame.t * (d.x * radius) + frame.b * (d.y * radius)
}

/// Trace photons `range` of `total` for pass `pass`, adding those that land
/// to `out`. Each photon's random numbers depend only on its index, the pass
/// and the seed, so the photons are the same however the range is split.
pub fn trace_photons(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    plan: &CausticPlan,
    pass: u32,
    seed: u32,
    range: std::ops::Range<u32>,
    total: u32,
    out: &mut Vec<Photon>,
) {
    let scene = intersector.scene();
    let area = |r: f32| std::f32::consts::PI * r * r;
    for index in range {
        // A stream of its own, apart from every pixel's: photons are
        // numbered past any image's rows.
        let mut sampler = Sampler::new(index % 4096, 1_000_000 + index / 4096, pass, seed ^ 0x9e37_79b9);
        let (source, share) = plan.choose(sampler.next_unstructured());
        let emitted = total as f32 * share;
        let (ray, power) = match source {
            Source::Environment => {
                let Some(sample) = scene.environment.sample_from(plan.centre, sampler.next_unstructured2()) else {
                    continue;
                };
                if sample.pdf <= 0.0 || sample.radiance.is_black() {
                    continue;
                }
                let at = on_disc(plan.centre, sample.direction, plan.radius, sampler.next_unstructured2());
                let back = if sample.distance.is_finite() {
                    (sample.distance * 0.999).min(plan.far)
                } else {
                    plan.far
                };
                let power = sample.radiance * (area(plan.radius) / (sample.pdf * emitted));
                (Ray::new(at + sample.direction * back, -sample.direction), power)
            }
            Source::Light(light) => {
                let Some(light) = scene.lights.lights().get(light) else { continue };
                // Aim at the sphere as the light sees it: a disc through its
                // centre, facing a point on the light, wide enough to catch
                // every line from that point that touches the sphere.
                let Some(towards) = light.sample(plan.centre, sampler.next_unstructured2()) else {
                    continue;
                };
                let d = towards.distance;
                let r = plan.radius;
                let wide = if d.is_finite() && d > r * 1.001 {
                    r * d / (d * d - r * r).sqrt()
                } else if d.is_finite() {
                    r * 4.0
                } else {
                    r
                };
                let at = on_disc(plan.centre, towards.direction, wide, sampler.next_unstructured2());
                let Some(sample) = light.sample(at, sampler.next_unstructured2()) else { continue };
                let density = if sample.is_delta { 1.0 } else { sample.pdf };
                if density <= 0.0 || sample.radiance.is_black() {
                    continue;
                }
                let facing = sample.direction.dot(towards.direction).abs();
                let power = sample.radiance * (facing * area(wide) / (density * emitted));
                let origin = if sample.distance.is_finite() {
                    at + sample.direction * (sample.distance * (1.0 - 1e-4))
                } else {
                    at + sample.direction * plan.far
                };
                (Ray::new(origin, -sample.direction), power)
            }
        };
        if power.is_finite() && !power.is_black() {
            trace_photon(intersector, config, ray, power, &mut sampler, out);
        }
    }
}

/// Follow one photon through mirror-smooth surfaces to where it lands.
fn trace_photon(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    mut ray: Ray,
    mut power: Spectrum,
    sampler: &mut Sampler,
    out: &mut Vec<Photon>,
) {
    let scene = intersector.scene();
    let mut media = MediumStack::default();
    let mut smooth_events = 0u32;
    let mut crossings = 0u32;
    let mut wavelength: Option<f32> = None;
    let mut registers = shade::Registers::new();
    let mut scratch = crate::material::Material::default();
    let mut bump_registers = shade::Registers::new();
    let mut bump_scratch = crate::material::Material::default();
    let start = power.max_component();
    let limit = config.max_transmission_bounces.max(4) + 4;

    while smooth_events <= limit {
        let Some(hit) = intersector.intersect(&ray) else { return };
        let Some(instance) = scene.instances.get(hit.instance.0 as usize) else { return };
        let through = |ray: &Ray, crossings: &mut u32| {
            *crossings += 1;
            let (geometric, _) = hit.facing_forward(ray.direction);
            Ray::new(offset_ray_origin(hit.position, -geometric), ray.direction)
        };

        // A light's visible stand-in is not there for light.
        if instance.light_proxy {
            if crossings >= MAX_CUTOUT_CROSSINGS {
                return;
            }
            ray = through(&ray, &mut crossings);
            continue;
        }

        // What absorbs on the way: coloured glass, a liquid.
        if let Some(volume) = media.top(None).and_then(|m| m.volume.as_ref()) {
            power *= volume.transmittance_over(hit.t);
        }
        // A medium that scatters takes the photon out of the smooth chain.
        if media.top(None).is_some_and(|m| m.scattering.is_some()) {
            return;
        }

        let (geometric_normal, shading_normal) = hit.facing_forward(ray.direction);

        // The floor as a shadow catcher: where a caustic on it is read - one
        // that catches shadows; one with its shadows off leaves the picture
        // alone, the light a glass focuses on it too (Cycles does not show
        // such a floor at all). The photon stops there either way.
        if instance.shadow_catcher.is_some() && !config.ground_illumination {
            if smooth_events > 0 && instance.shadow_catcher.is_some_and(|c| c.shadows) {
                out.push(Photon { position: hit.position, normal: geometric_normal, from: -ray.direction, power });
            }
            return;
        }

        // A fibre focuses nothing, whatever it is made of: the photon lands
        // on it, as on any rough surface.
        if scene.fibre_at(&hit, ray.direction).is_some() {
            if smooth_events > 0 {
                out.push(Photon { position: hit.position, normal: geometric_normal, from: -ray.direction, power });
            }
            return;
        }

        let base = scene.material(hit.material);
        let boundary = is_boundary(base);
        if boundary
            && crossings < MAX_CUTOUT_CROSSINGS
            && media.beyond(hit.instance, base.medium_priority) == media.top(None).map(|m| m.instance)
        {
            media.cross_with(hit.instance, base, scene.density(hit.instance));
            ray = through(&ray, &mut crossings);
            continue;
        }

        let shading_normal = if shading_normal.dot(geometric_normal) <= 0.0 {
            geometric_normal
        } else {
            shading_normal
        };
        let point = scene
            .program(hit.material)
            .map(|_| scene.shading_point_along(&hit, shading_normal, config.pixel_spread, ray.direction));
        let material = match (scene.program(hit.material), &point) {
            (Some(program), Some(point)) => shade::shade(program, &scene.textures, point, &mut registers, &mut scratch),
            _ => base,
        };
        let shading_normal = match (scene.bump(hit.material), &point) {
            (Some(bump), Some(point)) => crate::surface_bump::bumped_normal(
                bump,
                scene,
                &hit,
                point,
                shading_normal,
                geometric_normal,
                &registers,
                &mut bump_registers,
                &mut bump_scratch,
            ),
            _ => shading_normal,
        };
        if scene.has_cutouts() && material.geometry_opacity < 0.999 {
            if crossings < MAX_CUTOUT_CROSSINGS && sampler.next_unstructured() >= material.geometry_opacity {
                ray = through(&ray, &mut crossings);
                continue;
            }
        }

        // Dispersion, as the camera path does it: a photon takes one
        // wavelength at its first dispersive surface and keeps it.
        let dispersed;
        let material = if material.transmission_weight > 0.0 && material.transmission_dispersion_abbe > 0.0 {
            let lambda = match wavelength {
                Some(lambda) => lambda,
                None => {
                    let lambda = tequila_materials::dispersion::sample(sampler.next_unstructured());
                    power *= tequila_materials::dispersion::weight(lambda);
                    wavelength = Some(lambda);
                    lambda
                }
            };
            dispersed = crate::material::Material {
                specular_ior: tequila_materials::dispersion::ior_at(
                    material.specular_ior,
                    material.transmission_dispersion_abbe,
                    lambda,
                ),
                ..material.clone()
            };
            &dispersed
        } else {
            material
        };
        let shaded = material;
        let (from_inside, relative_ior) = if boundary {
            let here = media.top(None).map(|m| m.instance);
            let there = media.beyond(hit.instance, base.medium_priority);
            let n_here = media.ior_of(here, None);
            let n_there = media.ior_of(there, Some(shaded));
            if n_there >= n_here {
                (false, n_there / n_here)
            } else {
                (true, n_here / n_there)
            }
        } else {
            (false, shaded.specular_ior)
        };
        let reindexed;
        let material = if (relative_ior - shaded.specular_ior).abs() > 1e-6 {
            reindexed = crate::material::Material { specular_ior: relative_ior, ..shaded.clone() };
            &reindexed
        } else {
            shaded
        };
        let bsdf = if from_inside { material.bsdf_from_inside(0.0) } else { material.bsdf(0.0) };

        // Anything that does not focus light is where the photon lands — if
        // a focusing surface sent it; light that comes straight from the
        // source is the path tracer's.
        if !caustic_surface(material, &bsdf) {
            if smooth_events > 0 {
                out.push(Photon { position: hit.position, normal: geometric_normal, from: -ray.direction, power });
            }
            return;
        }

        let onb = Onb::from_normal(shading_normal);
        let wo = onb.to_local(-ray.direction);
        if wo.z <= 0.0 {
            return;
        }
        let Some(sample) = bsdf.sample(wo, sampler.next_unstructured2(), sampler.next_unstructured()) else {
            return;
        };
        if sample.pdf <= 0.0 || sample.f.is_black() {
            return;
        }
        let wi = onb.to_world(sample.wi);
        if !sample.is_transmission && wi.dot(geometric_normal) <= 0.0 {
            return;
        }
        // Flux, not radiance: a refraction concentrates radiance by the
        // square of the relative index, and a photon's power does not
        // change with the solid angle it is squeezed into.
        let mut weight = sample.throughput_weight();
        if sample.is_transmission {
            weight *= sample.eta * sample.eta;
            if sample.flags.contains(BsdfFlags::SUBSURFACE) {
                return;
            }
            if boundary {
                media.cross_with(hit.instance, shaded, scene.density(hit.instance));
            }
        }
        power *= weight;
        smooth_events += 1;
        if !power.is_finite() || power.is_black() {
            return;
        }
        // Russian roulette once a photon has lost most of what it carried,
        // kept in proportion and scaled up, so the average is unchanged.
        let carried = power.max_component() / start.max(1e-12);
        if carried < 0.1 {
            if sampler.next_unstructured() >= carried {
                return;
            }
            power *= 1.0 / carried;
        }
        ray = Ray {
            origin: offset_ray_origin(hit.position, if sample.is_transmission { -geometric_normal } else { geometric_normal }),
            direction: wi,
            t_min: 0.0,
            t_max: f32::INFINITY,
            time: ray.time,
        };
    }
}

/// One pass's photons, indexed by where they are.
#[derive(Clone, Debug)]
pub struct CausticMap {
    photons: Vec<Photon>,
    radius: f32,
    cell: f32,
    /// A hashed grid of cells `cell` wide: `starts[h]..starts[h + 1]` in
    /// `order` are the photons whose cell hashes to `h`.
    starts: Vec<u32>,
    order: Vec<u32>,
    /// A few directions and weights of the environment, for the light a
    /// shadow catcher would receive with nothing in the way.
    sky: Vec<(Vec3, f32)>,
}

fn cell_of(p: Vec3, cell: f32) -> (i32, i32, i32) {
    ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32, (p.z / cell).floor() as i32)
}

fn hash((x, y, z): (i32, i32, i32), buckets: usize) -> usize {
    let h = (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663) ^ (z as u32).wrapping_mul(83_492_791);
    h as usize & (buckets - 1)
}

impl CausticMap {
    pub fn build(intersector: &dyn Intersector, photons: Vec<Photon>, radius: f32) -> CausticMap {
        let cell = (radius * 2.0).max(1e-6);
        let buckets = (photons.len() * 2).next_power_of_two().max(64);
        let mut counts = vec![0u32; buckets + 1];
        for photon in &photons {
            counts[hash(cell_of(photon.position, cell), buckets) + 1] += 1;
        }
        for i in 1..counts.len() {
            counts[i] += counts[i - 1];
        }
        let starts = counts.clone();
        let mut fill = counts;
        let mut order = vec![0u32; photons.len()];
        for (index, photon) in photons.iter().enumerate() {
            let h = hash(cell_of(photon.position, cell), buckets);
            order[fill[h] as usize] = index as u32;
            fill[h] += 1;
        }
        const SKY: u32 = 256;
        let environment = &intersector.scene().environment;
        let sky = (0..SKY)
            .filter_map(|i| {
                let u = tequila_core::math::vec2((i as f32 + 0.5) / SKY as f32, ((i * 97 + 13) % SKY) as f32 / SKY as f32 + 0.5 / SKY as f32);
                let sample = environment.sample(u)?;
                (sample.pdf > 0.0).then(|| (sample.direction, sample.radiance.luminance() / (sample.pdf * SKY as f32)))
            })
            .collect();
        CausticMap { photons, radius, cell, starts, order, sky }
    }

    pub fn len(&self) -> usize {
        self.photons.len()
    }

    pub fn is_empty(&self) -> bool {
        self.photons.is_empty()
    }

    /// The light the photons bring to `position`, on the side `normal` faces,
    /// each weighed by `response` (the surface's BSDF towards where it came
    /// from): reflected radiance when `response` is the BSDF, irradiance when
    /// it is 1.
    pub fn estimate(&self, position: Vec3, normal: Vec3, mut response: impl FnMut(Vec3) -> Spectrum) -> Spectrum {
        if self.photons.is_empty() {
            return Spectrum::ZERO;
        }
        let buckets = self.starts.len() - 1;
        let r2 = self.radius * self.radius;
        let low = cell_of(position - Vec3::splat(self.radius), self.cell);
        let high = cell_of(position + Vec3::splat(self.radius), self.cell);
        let mut sum = Spectrum::ZERO;
        // At most two cells a side: a cell is a diameter wide.
        let mut seen = [usize::MAX; 8];
        let mut seen_len = 0;
        for x in low.0..=high.0 {
            for y in low.1..=high.1 {
                for z in low.2..=high.2 {
                    let h = hash((x, y, z), buckets);
                    // Two cells can hash alike; a bucket is read once.
                    if seen[..seen_len].contains(&h) {
                        continue;
                    }
                    if seen_len < seen.len() {
                        seen[seen_len] = h;
                        seen_len += 1;
                    }
                    for &i in &self.order[self.starts[h] as usize..self.starts[h + 1] as usize] {
                        let photon = &self.photons[i as usize];
                        if (photon.position - position).length_squared() > r2 || photon.normal.dot(normal) < 0.7 {
                            continue;
                        }
                        sum += response(photon.from) * photon.power;
                    }
                }
            }
        }
        sum * (1.0 / (std::f32::consts::PI * r2))
    }

    /// The irradiance, as luminance, the environment and the lights would
    /// bring to a point facing `normal` with nothing in the way: what a
    /// shadow catcher's caustic is measured against.
    pub fn open_sky(&self, intersector: &dyn Intersector, position: Vec3, normal: Vec3) -> f32 {
        let mut total: f32 = self.sky.iter().map(|(d, w)| d.dot(normal).max(0.0) * w).sum();
        for light in intersector.scene().lights.lights() {
            if let Some(sample) = light.sample(position, tequila_core::math::vec2(0.5, 0.5)) {
                let density = if sample.is_delta { 1.0 } else { sample.pdf };
                if density > 0.0 {
                    total += sample.radiance.luminance() * sample.direction.dot(normal).max(0.0) / density;
                }
            }
        }
        total
    }
}
