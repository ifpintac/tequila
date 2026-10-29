//! Unidirectional path tracing with next-event estimation and multiple
//! importance sampling — the algorithm of `docs/01-RENDERER-CORE.md` §3.1.
//!
//! This is the workhorse. It handles everything except the caustic paths of
//! §6.5 — light through glass or off a mirror onto a surface — which
//! [`caustics`] traces from the lights when caustics are on.

pub mod caustics;

use crate::intersect::{Hit, Intersector};
use crate::scene::ShadowCatcher;
use crate::shade;
use crate::settings::IntegratorConfig;
use tequila_core::ids::InstanceId;
use tequila_core::math::{offset_ray_origin, Onb, Ray, Vec2, Vec3};
use tequila_core::sampler::Sampler;
use tequila_core::sampling::mis_power;
use tequila_core::spectrum::Spectrum;
use crate::aov::PathPasses;
use crate::lpe::{LpeMatcher, Scatter, Transport};
use tequila_materials::{Bsdf, BsdfFlags};

/// Radiance plus the auxiliary outputs a single path produces.
///
/// Albedo and normal are the denoiser's guide buffers. `docs/01` §13 is blunt
/// about them: "Denoising the beauty pass alone destroys texture detail; with
/// guides it preserves it." They are taken from the first non-specular vertex,
/// not the first hit, so that a mirror contributes the albedo of what it
/// reflects rather than a featureless white.
#[derive(Clone, Copy, Debug, Default)]
pub struct PathResult {
    pub radiance: Spectrum,
    pub albedo: Spectrum,
    pub normal: Vec3,
    /// Distance to the first hit; `f32::INFINITY` if the ray escaped.
    pub depth: f32,
    /// True when the camera ray hit nothing — the pixel is background, and
    /// alpha is zero there.
    pub escaped: bool,
    /// How much of this pixel is *product* rather than background, 0 to 1.
    ///
    /// Binary for ordinary geometry, which either is there or is not. A shadow
    /// catcher is where it stops being binary: a shadow on an invisible floor
    /// has to come out as partial alpha, or compositing the render over
    /// anything else loses the shadow entirely.
    pub coverage: f32,
    /// Energy discarded by firefly rejection, for the auditing AOV of §3.5.
    pub clamped_energy: f32,
    /// World position of the first hit, for §12's Position pass. Compositors
    /// want it and it costs a copy.
    pub position: Vec3,
    /// The instance and material the camera ray landed on, for the ID passes
    /// and for Cryptomatte. `u32::MAX` where the ray hit nothing.
    pub object_id: u32,
    pub material_id: u32,
    /// Direct lighting as it would be with nothing in the way, and as it
    /// actually is. §12's Shadow pass is the ratio, and it is accumulated here
    /// rather than derived because the two differ only inside next-event
    /// estimation, where the shadow ray is traced.
    pub direct_unshadowed: Spectrum,
    pub direct_visible: Spectrum,
    /// Ambient occlusion at the first hit: the fraction of a short hemisphere
    /// of rays that were blocked.
    pub occlusion: f32,
    /// The labels at the first hit, alone, premultiplied (§12's Labels
    /// pass), and how much of the pixel they cover.
    pub labels: Spectrum,
    pub labels_alpha: f32,
    /// The one wavelength the path carried, if it met a dispersive material
    /// (§7.4). Its radiance is then coloured by that wavelength, which firefly
    /// rejection has to see past or it clips some colours more than others.
    pub wavelength: Option<f32>,
    /// The part of `radiance` that is the background the camera saw where
    /// nothing covered it: all of it for a ray that escaped (through a
    /// cutout's holes included), the backdrop's share on a shadow catcher.
    /// An image with alpha leaves it out, since there the background is
    /// transparent; one without keeps it.
    pub background: Spectrum,
}

/// The light-path machinery a path carries while it is traced.
///
/// Passed in rather than returned because it holds buffers: a matcher's state
/// and a path's per-pass sums are both reused across every path a thread
/// traces. An empty one — which is what a viewport render uses — allocates
/// nothing and makes every routing decision a branch on zero.
#[derive(Clone, Debug, Default)]
pub struct PathAovs {
    pub matcher: LpeMatcher,
    pub passes: PathPasses,
}

impl PathAovs {
    pub fn new(expressions: Vec<crate::lpe::Lpe>) -> PathAovs {
        let count = expressions.len();
        PathAovs {
            matcher: LpeMatcher::new(expressions),
            passes: PathPasses::new(count),
        }
    }

    #[inline]
    fn is_off(&self) -> bool {
        self.matcher.is_empty()
    }

    /// Route one contribution to whichever passes accept the path as it
    /// stands, ended at a light of `group` (§5.5's light layers).
    #[inline]
    fn contribute(&mut self, contribution: Spectrum, group: crate::lpe::LightGroup) {
        if self.is_off() || contribution.is_black() {
            return;
        }
        let mask = self.matcher.accepting_at_light(group);
        self.passes.add(mask, contribution);
    }
}

/// How many cutout surfaces one ray may cross before it gives up.
///
/// A bound, not a budget: crossings are free in the sense that matters — they
/// spend no bounce and change no weight — so this exists only to stop a
/// pathological scene from looping for ever. Sixty-four is far past any real
/// stack of foliage or mesh, and a path that reaches it is treated as blocked
/// rather than as unoccluded, which errs towards a shadow rather than towards
/// a light leak.
const MAX_CUTOUT_CROSSINGS: u32 = 64;

/// How many surfaces the seeding probe of [`enclosing_media`] will cross.
///
/// A bound rather than a budget, for the same reason [`MAX_CUTOUT_CROSSINGS`]
/// is one. A probe that runs out has found geometry far too layered for the
/// medium stack to describe anyway, and answering "outside" then is the
/// answer that matches what the camera is nearly always doing.
const MAX_ENCLOSURE_CROSSINGS: u32 = 32;

/// The media a path starts inside, if it starts inside any.
///
/// Decided by counting crossings along the ray rather than by reading a
/// normal: an odd number of surfaces of one instance between the origin and
/// infinity means the origin is within it. That is true of any closed mesh
/// however it is wound, which is the whole point — the winding is the thing
/// that cannot be trusted here.
///
/// Only the boundaries of media are counted. An opaque object the camera has
/// been pushed inside of has no interface to get the wrong way round, and
/// counting it would cost every scene the probe for nothing.
///
/// Costs nothing in the ordinary case: the first hit of a camera ray outside
/// everything is a front face of an opaque surface, and the walk stops at the
/// first surface that is not transmissive.
fn enclosing_media(intersector: &dyn Intersector, ray: &Ray) -> MediumStack {
    let scene = intersector.scene();
    let mut media = MediumStack::default();
    // Nothing in the scene refracts, so nothing can be the wrong way round.
    if !scene.has_transmissive() {
        return media;
    }

    let mut counts: [(InstanceId, tequila_core::ids::MaterialId, u32); MAX_MEDIA] =
        [(InstanceId(u32::MAX), tequila_core::ids::MaterialId(0), 0); MAX_MEDIA];
    let mut used = 0usize;
    let mut probe = *ray;
    for _ in 0..MAX_ENCLOSURE_CROSSINGS {
        let Some(hit) = intersector.intersect(&probe) else {
            break;
        };
        // Only what has an inside: a thin-walled shell is a surface with no
        // medium behind it, whatever its mesh encloses.
        if is_boundary(scene.material(hit.material)) {
            match counts[..used].iter_mut().find(|(i, _, _)| *i == hit.instance) {
                Some((_, _, n)) => *n += 1,
                // More than a handful of media along one ray is past what the
                // stack holds; stop counting rather than grow, and let the
                // extra ones read as outside.
                None if used < MAX_MEDIA => {
                    counts[used] = (hit.instance, hit.material, 1);
                    used += 1;
                }
                None => {}
            }
        }
        // Past the surface just crossed, so the next query cannot find it
        // again. `facing_forward` is what turns the normal to point back
        // towards where the probe came from, which the stored one does not do
        // on its own — offsetting along that instead would push the origin
        // back to the side it started on and find the same surface for ever.
        let (facing, _) = hit.facing_forward(probe.direction);
        probe = Ray::new(offset_ray_origin(hit.position, -facing), probe.direction);
    }

    for (instance, material, n) in &counts[..used] {
        if n % 2 == 1 {
            media.push(Medium::with_field(*instance, scene.material(*material), scene.density(*instance)));
        }
    }
    media
}

/// Whether a surface is the boundary of a medium: transmissive, and not a
/// thin shell standing for a pane with nothing behind it.
#[inline]
fn is_boundary(material: &crate::material::Material) -> bool {
    material.is_transmissive() && !material.geometry_thin_walled
}

/// How many steps a random walk may take before it is taken as absorbed.
///
/// [Chiang et al. 2016] use 256, which suits a part a few mean free paths
/// thick. It does not suit milk: a third of a millimetre between events, a
/// glass-sized volume, and a path that wanders in takes thousands of events
/// to come back — at 256 a non-absorbing milky ball kept 0.83 to 0.86 of a
/// white world's light. The walks that go deep are few (a walk's length
/// has a long, thin tail), so raising the cap costs far less than it
/// sounds: 16384 keeps 0.98 to 1.00 for under three times the time on milk,
/// and nothing on a part that never scatters that deep. Paths that carry
/// little are ended by Russian roulette long before, where they absorb.
const MAX_WALK_STEPS: u32 = 16384;

/// How many null collisions a walk through a Volume Map's medium may make.
/// They are not interactions — the path flies on unchanged — so they do not
/// count against [`MAX_WALK_STEPS`]; this only stops a pathological grid, a
/// single dense voxel in a box of nothing, from walking for ever.
const MAX_NULL_STEPS: u32 = 4096;

/// The surface a random walk leaves by: the subsurface lobe alone, white,
/// with nothing reflecting in front of it.
fn walk_exit() -> crate::material::Material {
    crate::material::Material {
        base_weight: 1.0,
        base_metalness: 0.0,
        specular_weight: 0.0,
        transmission_weight: 0.0,
        subsurface_weight: 1.0,
        subsurface_color: Spectrum::ONE,
        coat_weight: 0.0,
        fuzz_weight: 0.0,
        thin_film_weight: 0.0,
        emission_luminance: 0.0,
        ..crate::material::Material::default()
    }
}

/// How many media deep a path can be. A liquid in a glass in a vitrine is
/// three; past four, the outermost read as air.
const MAX_MEDIA: usize = 4;

/// A medium a path is inside (`docs/01` §6.1).
#[derive(Clone)]
struct Medium {
    instance: InstanceId,
    priority: f32,
    /// The medium's own index, relative to vacuum.
    ior: f32,
    /// The material whose volume absorbs along the path, where there is one:
    /// clear glass with no depth has an index and no volume.
    volume: Option<crate::material::Material>,
    /// A part's subsurface medium, where the path is on a random walk
    /// (§6.4): it scatters as well as absorbs, and the path leaves it through
    /// the part's own surface.
    scattering: Option<tequila_materials::openpbr::SubsurfaceMedium>,
    /// Entered through a subsurface lobe, and so left through one: the
    /// doorway into a random walk rather than a clear part's refracting
    /// surface.
    doorway: bool,
    /// Where a Volume Map varies the medium's density: `scattering` is then
    /// the medium at density 1, and the walk tracks against each majorant
    /// block's bound, keeping a collision with probability `density here /
    /// bound` (null-collision tracking).
    density: Option<std::sync::Arc<crate::density::DensityField>>,
    /// Its surface has the index of what is around it (a Scattering
    /// Medium's, whose surface is only a boundary), so a straight line
    /// leaves through it unbent — which is what lets light be sampled from
    /// inside it.
    index_matched: bool,
}

impl Medium {
    fn of(instance: InstanceId, material: &crate::material::Material) -> Medium {
        Medium {
            instance,
            priority: material.medium_priority,
            ior: material.specular_ior,
            // A volume that scatters absorbs in the walk; one that does not,
            // along the path.
            volume: (material.has_volume() && material.volume_scattering().is_none())
                .then(|| material.clone()),
            scattering: material.volume_scattering(),
            doorway: false,
            density: None,
            index_matched: (material.specular_ior - 1.0).abs() < 1e-3,
        }
    }

    /// The medium inside a part whose density a Volume Map varies. Walked
    /// even where it only absorbs, since absorption that varies along the
    /// path has no closed form: the walk's real collisions are then
    /// absorptions, and its null collisions carry the path on.
    fn varying(
        instance: InstanceId,
        material: &crate::material::Material,
        field: std::sync::Arc<crate::density::DensityField>,
    ) -> Medium {
        // The medium as described, at density 1; the grid scales it.
        let unit = crate::material::Material { volume_density: 1.0, ..material.clone() };
        let coefficients = unit.volume_scattering().or_else(|| {
            (unit.has_volume() && unit.transmission_depth > 0.0).then(|| {
                let absorption = |c: f32| -c.clamp(1e-4, 1.0).ln() / unit.transmission_depth;
                let c = unit.transmission_color;
                tequila_materials::openpbr::SubsurfaceMedium {
                    extinction: Spectrum { r: absorption(c.r), g: absorption(c.g), b: absorption(c.b) },
                    albedo: Spectrum::ZERO,
                    anisotropy: 0.0,
                }
            })
        });
        // At density 1: the walk multiplies in each block's bound as it goes.
        let scattering = coefficients.filter(|_| field.max() > 0.0);
        Medium {
            instance,
            priority: material.medium_priority,
            ior: material.specular_ior,
            volume: None,
            scattering,
            doorway: false,
            density: Some(field),
            index_matched: (material.specular_ior - 1.0).abs() < 1e-3,
        }
    }

    /// [`Medium::of`], or [`Medium::varying`] where the part has a field.
    fn with_field(
        instance: InstanceId,
        material: &crate::material::Material,
        field: Option<&std::sync::Arc<crate::density::DensityField>>,
    ) -> Medium {
        match field {
            Some(field) => Medium::varying(instance, material, field.clone()),
            None => Medium::of(instance, material),
        }
    }

    /// The medium under a subsurface material's surface, which a path enters
    /// through its subsurface lobe.
    fn walk(instance: InstanceId, material: &crate::material::Material) -> Option<Medium> {
        Some(Medium {
            instance,
            priority: material.medium_priority,
            ior: material.specular_ior,
            volume: None,
            scattering: Some(material.subsurface_medium()?),
            doorway: true,
            density: None,
            index_matched: false,
        })
    }
}

/// The media a path is inside, in the order it entered them.
///
/// A stack with priorities rather than one medium, because a filled
/// container is two media that overlap: a liquid is modelled a fraction of a
/// millimetre into its glass, so that no air gap can open between them, and
/// in the overlap the liquid is what is there. So the medium *present* is not
/// the last one entered but the highest-priority one the path is in — ties
/// to the last entered, which leaves a lone glass object exactly as it would
/// be with one medium — and a surface that does not change which medium is
/// present is not an interface at all: the glass's inner wall, inside the
/// liquid, is crossed without scattering.
#[derive(Default)]
struct MediumStack {
    media: [Option<Medium>; MAX_MEDIA],
    len: usize,
}

impl MediumStack {
    fn position(&self, instance: InstanceId) -> Option<usize> {
        self.media[..self.len]
            .iter()
            .position(|m| m.as_ref().is_some_and(|m| m.instance == instance))
    }

    /// The medium present, leaving out the entry at `skip`.
    fn top(&self, skip: Option<usize>) -> Option<&Medium> {
        let mut best: Option<&Medium> = None;
        for (i, medium) in self.media[..self.len].iter().enumerate() {
            let Some(medium) = medium else { continue };
            if Some(i) != skip && best.map_or(true, |b| medium.priority >= b.priority) {
                best = Some(medium);
            }
        }
        best
    }

    /// The medium present on the far side of a surface of `instance`, whose
    /// priority is `priority`: crossing it enters the instance if the path
    /// is outside it and leaves it if the path is in it.
    fn beyond(&self, instance: InstanceId, priority: f32) -> Option<InstanceId> {
        match self.position(instance) {
            Some(i) => self.top(Some(i)).map(|m| m.instance),
            None => match self.top(None) {
                Some(top) if top.priority > priority => Some(top.instance),
                _ => Some(instance),
            },
        }
    }

    fn ior_of(&self, instance: Option<InstanceId>, entering: Option<&crate::material::Material>) -> f32 {
        match instance {
            None => 1.0,
            Some(i) => match self.position(i) {
                Some(at) => self.media[at].as_ref().map_or(1.0, |m| m.ior),
                None => entering.map_or(1.0, |m| m.specular_ior),
            },
        }
    }

    /// Record crossing a surface of `instance`, with no Volume Map.
    #[cfg(test)]
    fn cross(&mut self, instance: InstanceId, material: &crate::material::Material) {
        self.cross_with(instance, material, None)
    }

    /// The same, for a part whose medium a Volume Map may vary.
    fn cross_with(
        &mut self,
        instance: InstanceId,
        material: &crate::material::Material,
        field: Option<&std::sync::Arc<crate::density::DensityField>>,
    ) {
        match self.position(instance) {
            Some(i) => {
                for j in i..self.len - 1 {
                    self.media[j] = self.media[j + 1].take();
                }
                self.len -= 1;
                self.media[self.len] = None;
            }
            None if self.len < MAX_MEDIA => {
                self.media[self.len] = Some(Medium::with_field(instance, material, field));
                self.len += 1;
            }
            None => {}
        }
    }

    /// Whether the path is on a random walk inside `instance`.
    fn walking_in(&self, instance: InstanceId) -> bool {
        self.top(None).is_some_and(|m| m.doorway && m.instance == instance)
    }

    /// Start or end a random walk in `instance`, as a path crosses its
    /// surface through the subsurface lobe.
    fn cross_walk(&mut self, instance: InstanceId, material: &crate::material::Material) {
        if let Some(at) = self.position(instance) {
            for j in at..self.len - 1 {
                self.media[j] = self.media[j + 1].take();
            }
            self.len -= 1;
            self.media[self.len] = None;
        } else if let Some(medium) = Medium::walk(instance, material) {
            self.push(medium);
        }
    }

    fn push(&mut self, medium: Medium) {
        if self.len < MAX_MEDIA {
            self.media[self.len] = Some(medium);
            self.len += 1;
        }
    }
}

/// Trace one camera ray and return its contribution.
pub fn trace_path(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    ray: Ray,
    film: Vec2,
    sampler: &mut Sampler,
) -> PathResult {
    let mut aovs = PathAovs::default();
    trace_path_with_aovs(intersector, config, ray, film, sampler, &mut aovs)
}

/// The same, routing contributions into render passes as it goes.
///
/// `aovs` is cleared here rather than by the caller: forgetting to clear it
/// between paths would add every path's passes to the last one's, which is a
/// failure that looks like an extremely bright render rather than like a bug in
/// the accounting.
pub fn trace_path_with_aovs(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    ray: Ray,
    film: Vec2,
    sampler: &mut Sampler,
    aovs: &mut PathAovs,
) -> PathResult {
    trace_path_with_caustics(intersector, config, ray, film, sampler, aovs, None)
}

/// The same, reading a pass's caustic photons (§6.5) where there are some.
pub fn trace_path_with_caustics(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    mut ray: Ray,
    // Where on the film this path started, in `[0, 1]`. Needed only by a
    // backplate, which is mapped to the frame rather than to a direction.
    film: Vec2,
    sampler: &mut Sampler,
    aovs: &mut PathAovs,
    caustic_map: Option<&caustics::CausticMap>,
) -> PathResult {
    aovs.passes.clear();
    aovs.matcher.begin();
    let scene = intersector.scene();
    let mut out = PathResult {
        depth: f32::INFINITY,
        normal: Vec3::ZERO,
        object_id: u32::MAX,
        material_id: u32::MAX,
        ..Default::default()
    };
    // Reused across every vertex of this path, so shading a graph allocates
    // nothing after the first bounce that needs it.
    let mut shading_registers = shade::Registers::new();
    let mut shading_scratch = crate::material::Material::default();
    // For the height either side of a bumped point, kept apart from the
    // material's own so the main evaluation's registers survive it.
    let mut bump_registers = shade::Registers::new();
    let mut bump_scratch = crate::material::Material::default();

    let mut radiance = Spectrum::ZERO;
    let mut throughput = Spectrum::ONE;
    let mut bounce = 0u32;
    let mut transmission_bounces = 0u32;
    // Crossings of a cutout surface. Not bounces: passing through a hole in a
    // leaf is not a scattering event and must not spend the path's depth
    // budget, or a row of chain-link fences would go black. Counted only so
    // that a scene of a thousand stacked cutouts terminates.
    let mut crossings = 0u32;
    // The pixel sample's coverage, settled at the first cutout the camera ray
    // meets as its expectation ([`camera_coverage`]) rather than by where the
    // path happens to go. `None` until then, and for every ordinary surface.
    let mut settled_coverage: Option<f32> = None;
    // Whether a shadow catcher lies behind those cutouts, whose shadow's
    // coverage is added if the path reaches it.
    let mut catcher_behind = false;
    // Steps of the random walk inside a subsurface medium: a bound, as for
    // cutouts, rather than a budget spent from the bounces — a walk takes
    // tens to hundreds of steps and each is cheap.
    let mut walk_steps = 0u32;
    let mut null_steps = 0u32;
    // The medium whose last scattering event already sampled the lights
    // directly, and the density with which its phase function then chose the
    // way on: until the path meets anything but that medium's own surface,
    // the sky it escapes to is one of two strategies that could have found
    // it, and is weighted as one.
    let mut lit_from_medium: Option<InstanceId> = None;
    let mut lit_phase_pdf = 0.0f32;
    // The one wavelength this path carries, once it has met a dispersive
    // material (§7.4), in nanometres. `None` is an ordinary RGB path.
    let mut wavelength: Option<f32> = None;
    // The media the path is inside (§6.1), which decide which side of an
    // interface it meets, the relative index there, and what absorbs along
    // the way. Empty is the air outside everything.
    //
    // Seeded rather than assumed: a camera can start inside a glass object,
    // and a path that assumes it started in air gets every interface on the
    // way out backwards. `enclosing_media` settles it by counting crossings,
    // which no winding can lie about.
    let mut media = enclosing_media(intersector, &ray);

    // `INFINITY` marks a delta or camera vertex: there is no light-sampling
    // counterpart, so the MIS weight on the next emitter hit is 1.
    let mut prev_pdf_solid_angle = f32::INFINITY;
    let mut specular_chain = true;
    // Whether a shadow catcher met next is seen as the camera would see it
    // (see `catcher_view_through`): through mirrors always, through near
    // mirrors fading out with their roughness.
    let mut catcher_view = !config.in_floor_reflection;
    // What kind of ray this is, for the parts some rays do not see
    // (`Instance::seen_by_diffuse` and the rest).
    let mut seen_as = if config.in_floor_reflection { Seen::FloorMirror } else { Seen::Camera };
    // Accumulated relative IOR squared. §3.1: omitting this from Russian
    // roulette "makes glass unnecessarily noisy".
    let mut eta_scale = 1.0f32;
    let mut roughness_floor = 0.0f32;
    let mut guides_captured = false;
    // Past the surface where this path read the caustic photons, and through
    // mirror-smooth surfaces only since (`smooth_since` of them): the light
    // such a path reaches is the photons', already counted there.
    let mut read_caustics = false;
    let mut smooth_since = 0u32;
    // Every surface so far focuses light (`caustics::caustic_surface`): the
    // next that does not is where the photons are read.
    let mut focused_chain = true;

    loop {
        let hit = intersector.intersect(&ray);

        // The floor's reflection alone: only a camera ray meeting the floor
        // has anything to give (`IntegratorConfig::floor_reflection_only`).
        if config.floor_reflection_only && bounce == 0 {
            let on_floor = hit
                .as_ref()
                .and_then(|h| scene.instances.get(h.instance.0 as usize))
                .is_some_and(|i| i.shadow_catcher.is_some());
            if !on_floor {
                out.escaped = true;
                return out;
            }
        }

        // ---- Escaped: the environment ----------------------------------
        let Some(hit) = hit else {
            // From the ray's origin, not from the world's: with a finite dome
            // the two differ, and using the wrong one would light the scene
            // from a sky that nothing in it can see.
            //
            // A background mode replaces this only on a path the camera can
            // still see along — directly, or through glass and mirrors. Every
            // other path is carrying light *to* a surface, and a solid colour
            // or a backplate must not light anything (§1.5).
            // Two different rules, for two different things.
            //
            // A solid colour or a backplate replaces the view along the whole
            // specular chain, because §1.5 wants a backplate photograph to
            // appear in a chrome surface — without it the product looks
            // pasted onto the picture rather than standing in it.
            //
            // A pin marked not to show in the background is the opposite: it
            // leaves the *primary* ray's view and stays everywhere else, so
            // the backdrop is clean while chrome goes on reflecting the
            // softbox. That is the arrangement a product photographer sets up,
            // and neither background mode can produce it on its own.
            let emitted = if specular_chain && !scene.background.is_environment() {
                scene
                    .background
                    .radiance(&scene.environment, ray.origin, ray.direction, film)
            } else if bounce == 0 {
                scene
                    .environment
                    .background_radiance_from(ray.origin, ray.direction)
            } else {
                scene.environment.radiance_from(ray.origin, ray.direction)
            };
            // Light reached through mirror-smooth surfaces after reading the
            // photons: theirs, not this path's.
            let emitted = if read_caustics && smooth_since > 0 { Spectrum::ZERO } else { emitted };
            if !emitted.is_black() {
                // MIS combines two strategies. If the path arrived here by a
                // delta bounce, or if next-event estimation is switched off
                // entirely, there is no counterpart strategy that could also
                // have found this direction — so BSDF sampling takes the full
                // weight. Applying a MIS weight anyway silently discards the
                // half of the estimator that was never gathered.
                let weight = if lit_from_medium.is_some() {
                    // Straight out of a medium whose last event sampled the
                    // sky too: its phase function and the sky's own
                    // sampling are the two strategies.
                    let pdf_light = scene.environment.pdf_from(ray.origin, ray.direction);
                    mis_power(lit_phase_pdf, pdf_light)
                } else if specular_chain || !config.direct_lighting {
                    1.0
                } else {
                    let pdf_light = scene.environment.pdf_from(ray.origin, ray.direction);
                    mis_power(prev_pdf_solid_angle, pdf_light)
                };
                let contribution = throughput * emitted * weight;
                radiance += contribution;
                if bounce == 0 {
                    out.background += contribution;
                }
                // The environment is a light, so the path ends at one — which
                // is what puts a camera ray that sees the sky into the beauty
                // pass and out of every component pass.
                aovs.contribute(contribution, crate::lpe::ENVIRONMENT);
            }
            if bounce == 0 {
                out.escaped = true;
                out.coverage = settled_coverage.unwrap_or(0.0);
            }
            if !guides_captured {
                // A path that never hit anything: hand the denoiser the
                // environment radiance as albedo and the ray direction as a
                // normal, rather than zeros it would try to sharpen.
                out.albedo = emitted;
                out.normal = -ray.direction;
            }
            break;
        };

        // ---- The random walk (§6.4) --------------------------------------
        //
        // Inside a part's subsurface medium the path may scatter before it
        // reaches the next surface. The distance is sampled in one channel,
        // chosen in proportion to the path's throughput, and weighted over
        // all three — §6.2's spectral MIS, which keeps a medium whose
        // channels differ from turning into colour noise.
        let field = media.top(None).and_then(|m| m.density.clone());
        if let Some(walk) = media.top(None).and_then(|m| m.scattering) {
            walk_steps += 1;
            if walk_steps > MAX_WALK_STEPS {
                break;
            }
            let sigma = [walk.extinction.r, walk.extinction.g, walk.extinction.b];
            let survive = |t: f32| [(-sigma[0] * t).exp(), (-sigma[1] * t).exp(), (-sigma[2] * t).exp()];
            // Flights along this one ray until a real collision or the
            // surface. Null collisions (a Volume Map's medium, tracked at its
            // densest) carry the path on along the same line, so they are
            // taken here rather than by tracing the ray again: the surface
            // ahead is still `hit`, `travelled` short of where it was.
            let mut travelled = 0.0f32;
            let mut collided_at: Option<f32> = None;
            let mut lost = false;
            loop {
                let weights = [throughput.r.max(0.0), throughput.g.max(0.0), throughput.b.max(0.0)];
                let total = weights[0] + weights[1] + weights[2];
                let chance = if total > 0.0 {
                    [weights[0] / total, weights[1] / total, weights[2] / total]
                } else {
                    [1.0 / 3.0; 3]
                };
                let pick = sampler.next_unstructured();
                let channel = if pick < chance[0] {
                    0
                } else if pick < chance[0] + chance[1] {
                    1
                } else {
                    2
                };
                // An optical depth to fly, in density times distance: plain
                // distance in a uniform medium, and through a Volume Map
                // each majorant block's bound times the length crossed.
                let depth = -(1.0 - sampler.next_unstructured()).max(1e-12).ln() / sigma[channel].max(1e-9);
                let (t, bound) = match fly(field.as_deref(), &ray, travelled, hit.t, depth) {
                    Flight::Collided { t, bound } => (t, bound),
                    Flight::Reached { depth } => {
                        let kept = survive(depth);
                        let probability: f32 = (0..3).map(|c| chance[c] * kept[c]).sum();
                        if probability <= 0.0 {
                            lost = true;
                        } else {
                            throughput *= Spectrum { r: kept[0], g: kept[1], b: kept[2] } * (1.0 / probability);
                        }
                        break;
                    }
                };
                let kept = survive(depth);
                let density: f32 = (0..3).map(|c| chance[c] * sigma[c] * kept[c]).sum();
                if density <= 0.0 {
                    lost = true;
                    break;
                }
                travelled = t;
                // A varying medium: the flight was against its densest, so
                // the collision is real with probability density-here over
                // densest, the same in every channel. A null one carries the
                // path on unchanged in direction, weighted as the spectral
                // MIS asks — by exactly 1 where the medium is grey.
                if let Some(field) = &field {
                    let at = ray.origin + ray.direction * travelled;
                    let real = field.at(at) / bound.max(1e-12);
                    if sampler.next_unstructured() >= real {
                        null_steps += 1;
                        throughput *= Spectrum {
                            r: sigma[0] * kept[0],
                            g: sigma[1] * kept[1],
                            b: sigma[2] * kept[2],
                        } * (1.0 / density);
                        if null_steps > MAX_NULL_STEPS || !throughput.is_finite() || throughput.is_black() {
                            lost = true;
                            break;
                        }
                        continue;
                    }
                }
                throughput *= Spectrum {
                    r: walk.albedo.r * sigma[0] * kept[0],
                    g: walk.albedo.g * sigma[1] * kept[1],
                    b: walk.albedo.b * sigma[2] * kept[2],
                } * (1.0 / density);
                // Russian roulette in a long walk: after dozens of events a
                // path's weight is small and its remaining steps cost as much
                // as the first. Kept with the chance of what it carries and
                // scaled up by it, so the average is unchanged.
                let carried = throughput.r.max(throughput.g).max(throughput.b);
                if carried < 0.1 {
                    if sampler.next_unstructured() >= carried {
                        lost = true;
                        break;
                    }
                    throughput *= 1.0 / carried;
                }
                collided_at = Some(travelled);
                break;
            }
            if lost || !throughput.is_finite() || throughput.is_black() {
                break;
            }
            if let Some(distance) = collided_at {
                let at = ray.origin + ray.direction * distance;
                // Light the event directly (§5.3), where a straight line
                // leaves the medium unbent: the sky and the lights, weighted
                // by the phase function and by what of the medium lies
                // between. Without it a small bright light — the sun —
                // reaches smoke only when a scattered path happens to leave
                // straight towards it, which renders as specks.
                lit_from_medium = None;
                let lit = media
                    .top(None)
                    .filter(|m| m.index_matched && !m.doorway && media.len == 1)
                    .map(|m| m.instance);
                if let (Some(instance), true) = (lit, config.direct_lighting) {
                    let (sky, lights, group) = medium_direct_light(
                        intersector,
                        config,
                        at,
                        ray.direction,
                        &walk,
                        field.as_deref(),
                        instance,
                        sampler,
                    );
                    let (sky, lights) = (throughput * sky, throughput * lights);
                    radiance += sky + lights;
                    aovs.contribute(sky, crate::lpe::ENVIRONMENT);
                    aovs.contribute(lights, group);
                    lit_from_medium = Some(instance);
                }
                let next = tequila_core::sampling::sample_henyey_greenstein(
                    ray.direction,
                    walk.anisotropy,
                    sampler.next_unstructured2(),
                );
                // Sampled exactly, so the density is the phase function's
                // value.
                lit_phase_pdf = henyey_greenstein(ray.direction.dot(next), walk.anisotropy);
                ray = Ray::new(at, next);
                // The sky this reaches straight out of a lit medium is
                // weighted against the direct sample above; anything else it
                // meets is found by this path alone.
                prev_pdf_solid_angle = f32::INFINITY;
                specular_chain = false;
                read_caustics = false;
                focused_chain = false;
                continue;
            }
        }

        // Anything but the lit medium's own surface is a new event, which
        // the direct sample did not see past. A light's stand-in, passed
        // below, it did see past: it casts no shadow.
        if lit_from_medium.is_some_and(|m| m != hit.instance)
            && (specular_chain || !is_light_proxy(scene, hit.instance))
        {
            lit_from_medium = None;
        }

        // Hidden from the camera (a render layer's other parts): the camera
        // ray goes straight through, as through a hole, and every other ray
        // still meets it — so what is hidden goes on casting its shadows,
        // lighting its neighbours and showing in their reflections, and the
        // layer composites back over the rest without a seam. A part some
        // other kind of ray does not see is passed through by it the same
        // way: a card the camera cannot see but reflections can, a part
        // that shows in no glass.
        let unseen = |i: &crate::scene::Instance| match seen_as {
            Seen::Camera => bounce == 0 && !i.visible_to_camera,
            // The floor's own reflection: a reflection, and as the camera
            // sees it, so a render layer's hidden parts stay out of it too.
            Seen::FloorMirror => bounce == 0 && (!i.visible_to_camera || !i.seen_by_glossy),
            Seen::Diffuse => !i.seen_by_diffuse,
            Seen::Glossy => !i.seen_by_glossy,
            Seen::Transmission => !i.seen_by_transmission,
        };
        if crossings < MAX_CUTOUT_CROSSINGS && scene.instances.get(hit.instance.0 as usize).is_some_and(unseen) {
            crossings += 1;
            let (geometric, _) = hit.facing_forward(ray.direction);
            ray = Ray::new(
                tequila_core::math::offset_ray_origin(hit.position, -geometric),
                ray.direction,
            );
            continue;
        }

        if bounce == 0 {
            out.depth = hit.t;
            out.coverage = settled_coverage.unwrap_or(1.0);
            out.position = hit.position;
            out.object_id = hit.instance.0;
            out.material_id = hit.material.0;
            // Held out: a hole in the picture where the camera sees it,
            // nothing and no alpha, for what is composited in its place.
            if scene.instances.get(hit.instance.0 as usize).is_some_and(|i| i.holdout) {
                out.coverage = 0.0;
                break;
            }
        }

        // ---- Shadow catcher --------------------------------------------
        //
        // Handled before shading, and only on a path the camera can still see
        // along, because a catcher is not a surface: from any other direction
        // it does not exist at all, which is what keeps it from bouncing light
        // back into the product like a real floor would.
        if let Some(catcher) = scene
            .instances
            .get(hit.instance.0 as usize)
            .and_then(|i| i.shadow_catcher)
        {
            if catcher_view {
                // What the catcher stands in front of, which is what the
                // camera would have seen through it — the background, by the
                // same argument as above.
                let behind =
                    scene
                        .background
                        .radiance(&scene.environment, ray.origin, ray.direction, film);
                let caught = catch_shadow(
                    intersector,
                    config,
                    &hit,
                    &catcher,
                    ray.direction,
                    behind,
                    bounce,
                    sampler,
                );
                // A caustic on the floor: the photons' light against what
                // the floor would receive in the open, brightening what is
                // behind it as that light would a real floor.
                // Not on a floor with its shadows off (`caustics::trace_photon`).
                let caustic = match caustic_map {
                    Some(map) if catcher.shadows => {
                        let facing = if hit.geometric_normal.dot(-ray.direction) > 0.0 {
                            hit.geometric_normal
                        } else {
                            -hit.geometric_normal
                        };
                        let open = map.open_sky(intersector, hit.position, facing);
                        if open > 0.0 {
                            behind * map.estimate(hit.position, facing, |_| Spectrum::ONE) * (1.0 / open)
                        } else {
                            Spectrum::ZERO
                        }
                    }
                    _ => Spectrum::ZERO,
                };
                let contribution = throughput * (caught.radiance + caustic);
                radiance += contribution;
                if bounce == 0 {
                    // What shows of the backdrop where the shadow does not.
                    out.background += throughput * behind * (1.0 - caught.coverage);
                }
                // The backdrop seen through the floor, darkened where
                // shadows fall: the environment's, for the passes. Left out,
                // the Beauty pass had no floor and a relit image lost it.
                aovs.contribute(contribution, crate::lpe::ENVIRONMENT);
                if bounce == 0 {
                    // Through cutouts, whose own share is settled: the
                    // catcher adds its shadow's, on the paths that reach it,
                    // which over all of them is its share of what gets past.
                    out.coverage = match settled_coverage {
                        Some(settled) if catcher_behind => settled + caught.coverage,
                        _ => caught.coverage,
                    };
                    out.escaped = out.coverage <= 0.0;
                    (out.albedo, out.normal) = caught.guide;
                }
                // The camera's view of the catcher is the whole of what it
                // contributes; there is nothing further along this path.
                break;
            }
            // Reached by a bounce rather than by the camera. The catcher is
            // not a surface, so the path carries straight on through it, to
            // what a product sees below the horizon: the environment's own
            // ground, as §1.5's Flatten Ground projects onto the floor. It
            // used to end the path, which made it a black floor to every
            // reflection — a chrome part's lower half rendered black — and
            // blocked the light below it, so turning shadows on changed how
            // the product was lit.
            //
            // Light sampling does not see through it (a shadow ray stops at
            // the floor), so what lies beyond is found by this path alone and
            // takes full weight.
            //
            // A floor with a bounce glows for everything but the camera -
            // the studio's white sweep the product's reflections see - and
            // ends the path there; darker toward the contact over its
            // falloff, as ambient occlusion. In the floor's own reflection
            // too: it shows the product as it is, lit by the bounce. Left
            // out there, a face the bounce lifted was mirrored as dark as it
            // would be without it.
            if let Some(glow) = catcher.bounce.filter(|_| !config.floor_reflection_only) {
                let facing = if hit.geometric_normal.dot(-ray.direction) > 0.0 { hit.geometric_normal } else { -hit.geometric_normal };
                let open = if catcher.bounce_falloff > 0.0 {
                    1.0 - occlusion_at(intersector, &hit, facing, catcher.bounce_falloff, sampler)
                } else {
                    1.0
                };
                let emitted = throughput * glow * open;
                radiance += emitted;
                aovs.contribute(emitted, crate::lpe::ENVIRONMENT);
                break;
            }
            // Unless ground illumination is on, in which case the floor is
            // deliberately a real diffuse surface bouncing light back into the
            // product, and shading falls through to it.
            if !config.ground_illumination {
                if crossings >= MAX_CUTOUT_CROSSINGS {
                    break;
                }
                crossings += 1;
                let (geometric, _) = hit.facing_forward(ray.direction);
                ray = Ray::new(offset_ray_origin(hit.position, -geometric), ray.direction);
                prev_pdf_solid_angle = f32::INFINITY;
                continue;
            }
        }

        // Beer-Lambert across the segment just travelled. Only between two
        // surfaces: a path that escapes while still marked as inside has come
        // out of geometry that does not close, and attenuating it over the
        // distance to the environment would turn every open-sided glass part
        // black rather than admit the mesh is not a solid.
        if let Some((volume, ior)) = media.top(None).and_then(|m| m.volume.as_ref().map(|v| (v, m.ior))) {
            // What the medium itself gives off along the way, a lit light
            // guide's glow, before what it absorbs of the rest. Times the
            // square of its index: refraction here divides light born inside
            // by it on the way out, as light physically is, and Cycles and
            // Blender's refraction do not, so without it a glowing part was
            // twice as dim here as in a render through Cycles.
            let glow = volume.emitted_over(hit.t) * (ior * ior).max(1.0);
            if !glow.is_black() {
                let emitted = throughput * glow;
                radiance += emitted;
                aovs.contribute(emitted, crate::lpe::EMISSIVE);
            }
            throughput *= volume.transmittance_over(hit.t);
            if !throughput.is_finite() || throughput.is_black() {
                break;
            }
        }

        // A light's stand-in is there to be seen, and only on a path the
        // camera can see along. Any other path passes it, as photons and
        // shadow rays do: it does not light the scene — the analytic light it
        // stands for already does — and ending the path on it instead blotted
        // out the sky behind it for every bounce, a shadow by another name.
        // Cycles' lights, which it stands in for there, block nothing either.
        if !specular_chain && is_light_proxy(scene, hit.instance) {
            if crossings >= MAX_CUTOUT_CROSSINGS {
                break;
            }
            crossings += 1;
            ray = carried_past(&hit, &ray);
            continue;
        }

        // A medium's surface inside a medium that outranks it — the glass
        // wall inside the liquid that overlaps it — is not an interface:
        // nothing changes there, so the path carries straight on, as through
        // a cutout, and only the record of what it is inside changes.
        let base = scene.material(hit.material);
        let boundary = is_boundary(base);
        if boundary
            && crossings < MAX_CUTOUT_CROSSINGS
            && media.beyond(hit.instance, base.medium_priority) == media.top(None).map(|m| m.instance)
        {
            crossings += 1;
            media.cross_with(hit.instance, base, scene.density(hit.instance));
            let (geometric, _) = hit.facing_forward(ray.direction);
            ray = Ray::new(offset_ray_origin(hit.position, -geometric), ray.direction);
            continue;
        }

        let wo_world = -ray.direction;
        let (geometric_normal, shading_normal) = hit.facing_forward(ray.direction);
        // `facing_forward` turns both normals by the *geometric* one, so a
        // mesh whose winding contradicts its authored vertex normals arrives
        // here with the shading normal in the opposite hemisphere from the
        // ray. That is not the grazing-angle case the `wo.z` guard below is
        // for — it is a whole surface of contradictory data, and taking the
        // guard at its word terminates every path at its first vertex and
        // renders the object pure black with a pixel-perfect coverage mask.
        //
        // Which of the two is wrong cannot be known here, so the disagreement
        // is resolved towards the geometric normal: ray offsetting and the
        // shading-normal energy-leak guard are both already written against
        // it, and a shading normal that disagrees with those is the one that
        // has nothing consistent to say.
        let shading_normal = if shading_normal.dot(geometric_normal) <= 0.0 {
            geometric_normal
        } else {
            shading_normal
        };

        // A material with a graph is evaluated here; one without is used as it
        // stands. Most materials in most scenes have no graph — fasteners,
        // brackets, the inside of a housing — and making them pay for the
        // general case is how a feature that helps a few surfaces slows down
        // the whole render.
        let point = scene
            .program(hit.material)
            .map(|_| scene.shading_point_along(&hit, shading_normal, config.pixel_spread, ray.direction));
        if bounce == 0 {
            if let (Some(program), Some(point)) = (scene.program(hit.material), &point) {
                if !program.decals.is_empty() {
                    let (labels, alpha) = shade::labels_at(program, &scene.textures, point);
                    out.labels = labels;
                    out.labels_alpha = alpha;
                }
            }
        }
        let material = match (scene.program(hit.material), &point) {
            (Some(program), Some(point)) => shade::shade(
                program,
                &scene.textures,
                point,
                &mut shading_registers,
                &mut shading_scratch,
            ),
            _ => scene.material(hit.material),
        };

        // Metallic paint's flakes (§02 §4.4): which flake this point is on,
        // if any, and how it leans. The shading point is needed for it even
        // where the material has no graph.
        let flaked;
        let material = if material.flake_weight > 1e-4 {
            let at = point.unwrap_or_else(|| scene.shading_point_along(&hit, shading_normal, config.pixel_spread, ray.direction));
            flaked = crate::flakes::apply(material, &at);
            &flaked
        } else {
            material
        };

        // Rounded edges (Cycles' Bevel): near a sharp edge the normal leans
        // toward the next face. On the surface the camera sees, where it
        // shows, and with the material's own count of probes: fewer, and the
        // probes that miss the next face pull the average back toward the
        // sharp edge, a highlight half there. Past that first surface it
        // cost Tequila's renderer 40% on a product all of whose parts are
        // rounded. The bump bends what it leaves, as Bevel feeds Cycles'
        // Bump.
        let shading_normal = if material.edge_radius > 0.0 && bounce == 0 {
            crate::bevel::rounded_normal(intersector, &hit, shading_normal, material.edge_radius, material.edge_samples as u32, sampler)
        } else {
            shading_normal
        };

        // Bump and normal maps (§02 §5.4) bend the normal the BSDF is built
        // around; the geometric one, which offsets rays and guards against
        // leaks, stays as it was.
        let shading_normal = match (scene.bump(hit.material), &point) {
            (Some(bump), Some(point)) => crate::surface_bump::bumped_normal(
                bump,
                scene,
                &hit,
                point,
                shading_normal,
                geometric_normal,
                &shading_registers,
                &mut bump_registers,
                &mut bump_scratch,
            ),
            _ => shading_normal,
        };

        // ---- Cutout ------------------------------------------------------
        //
        // Opacity is a hole, not a filter: the ray either meets the surface or
        // carries straight on undeviated. Deciding stochastically rather than
        // splitting keeps one path one path, and leaves the estimator
        // unbiased with no weight to carry — the probability of continuing is
        // exactly the fraction of light that does.
        //
        // `next_unstructured` rather than a numbered dimension, for the reason
        // its own documentation gives: the number of crossings along a ray is
        // unbounded, so it cannot have a fixed place in the sample sequence.
        if scene.has_cutouts() && material.geometry_opacity < 0.999 {
            // The pixel's coverage, from the first cutout the camera sees:
            // the light through every surface along the ray, averaged
            // rather than sampled, as a shadow ray's is.
            if bounce == 0 && settled_coverage.is_none() {
                let beyond = tequila_core::math::offset_ray_origin(hit.position, -geometric_normal);
                let (coverage, catcher) = camera_coverage(intersector, beyond, ray.direction, material.geometry_opacity);
                settled_coverage = Some(coverage);
                catcher_behind = catcher;
                out.coverage = coverage;
            }
            if crossings < MAX_CUTOUT_CROSSINGS
                && sampler.next_unstructured() >= material.geometry_opacity
            {
                crossings += 1;
                // Offset along the ray, past the surface just crossed. The
                // geometric normal points back towards where the ray came
                // from, so the offset is away from it.
                ray = Ray::new(
                    tequila_core::math::offset_ray_origin(hit.position, -geometric_normal),
                    ray.direction,
                );
                // Everything else is deliberately untouched: no bounce spent,
                // no throughput scaled, and `specular_chain` intact, so a
                // camera ray that passes cleanly through a hole still sees the
                // backplate behind it rather than the environment.
                continue;
            }
        }

        // ---- Emissive surface ------------------------------------------
        //
        // A light proxy emits only towards the camera, which is the only path
        // that reaches here with one (any other passed it above). The analytic
        // light it stands for is already lighting the scene through next-event
        // estimation, and counting the proxy as well would double it.
        if material.is_emissive() && !hit.is_backface(ray.direction) {
            // No emissive-geometry light sampling at M0, so there is no
            // light-sampling counterpart to weight against; M3 adds the light
            // BVH (§5.3) and this gains its MIS partner.
            let contribution = throughput * material.emission();
            radiance += contribution;
            // A light's visible stand-in is that light, and goes in its
            // layer; any other glowing surface is a group of its own.
            let group = match scene.instances.get(hit.instance.0 as usize) {
                Some(instance) if instance.light_proxy => instance.light_layer.min(crate::lpe::LAST_LAYER),
                _ => crate::lpe::EMISSIVE,
            };
            aovs.contribute(contribution, group);
        }

        if bounce >= config.max_bounces {
            break;
        }

        let onb = if material.specular_roughness_anisotropy > 0.0 {
            // The graph's own direction where it draws one, in the part's
            // space: read from the registers the material was just shaded
            // into.
            let drawn = match (scene.program(hit.material), &point) {
                (Some(program), Some(_)) => program
                    .driven
                    .iter()
                    .find(|(param, _)| *param == tequila_materials::material_type::Param::Tangent)
                    .map(|(_, register)| scene.instance(hit.instance).transform.vector(shading_registers.value(*register).as_vector())),
                _ => None,
            };
            anisotropic_frame(scene, &hit, shading_normal, drawn, material.specular_rotation)
        } else {
            Onb::from_normal(shading_normal)
        };
        let wo = onb.to_local(wo_world);
        // A grazing ray can land marginally below the shading hemisphere even
        // though it is above the geometric one; there is nothing to shade.
        if wo.z <= 0.0 {
            break;
        }

        // Which side of the surface the path is on. `facing_forward` above
        // has already turned the shading frame to meet the ray, so the BSDF
        // cannot work this out for itself — and a transmissive one that
        // guesses "outside" every time applies the `1/η²` of entering a
        // denser medium at every crossing and never the `η²` of leaving.
        //
        // Read from where the path has actually been rather than from the
        // geometric normal. The normal answers this question with the
        // triangle winding, and a closed mesh imported inside out — a CAD
        // part, a glTF exported with clockwise faces — reports "inside" at
        // every one of its faces, which inverts the relative IOR at every
        // crossing and renders solid glass as an air bubble. Crossings cannot
        // be inverted that way: the path either went in or it did not.
        //
        // Between two media the interface is between their indices, not the
        // surface's own index and air: water in glass bends light by 1.5 to
        // 1.33, not by 1.33. The BSDF takes one index at least 1 and a side,
        // so going from a denser medium into a lighter one is the same
        // interface seen from inside.
        // Dispersion (§7.4): a path meeting glass with an Abbe number takes a
        // wavelength, is weighted by the colour it stands for, and from here
        // on bends as light of that wavelength does — at this surface and at
        // every dispersive one after it, entering and leaving alike, so the
        // index a medium was entered with is the one it is left by.
        let dispersed;
        let material = if material.transmission_weight > 0.0 && material.transmission_dispersion_abbe > 0.0 {
            let lambda = match wavelength {
                Some(lambda) => lambda,
                None => {
                    let lambda = tequila_materials::dispersion::sample(sampler.next_unstructured());
                    throughput *= tequila_materials::dispersion::weight(lambda);
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
            reindexed = crate::material::Material {
                specular_ior: relative_ior,
                ..shaded.clone()
            };
            &reindexed
        } else {
            shaded
        };
        // A walk that reaches its own part's surface from inside leaves
        // through it, spread over the hemisphere outside: the doorway it came
        // in by, turned round. What the part is made of has already been
        // counted, by the walk.
        let exiting_walk = media.walking_in(hit.instance);
        // Past the photons, glass stays glass: roughened by the floor it
        // would stop being mirror-smooth, and light through it would be
        // counted here as well as by the photons.
        let floor = if read_caustics { 0.0 } else { roughness_floor };
        let bsdf = if exiting_walk {
            walk_exit().bsdf(floor)
        } else if from_inside {
            material.bsdf_from_inside(floor)
        } else {
            material.bsdf(floor)
        };
        let smooth_here = caustic_map.is_some() && caustics::caustic_surface(material, &bsdf);

        if !guides_captured && !bsdf.flags().is_pure_delta() {
            out.albedo = bsdf.albedo(wo);
            out.normal = shading_normal;
            guides_captured = true;
        }

        sampler.begin_bounce(bounce);

        // ---- Next-event estimation -------------------------------------
        //
        // The environment and the analytic lights are sampled separately
        // rather than as one set. They have different structures — one is an
        // importance-sampled image, the others a power-weighted set — and
        // folding them together would mean choosing between them before
        // knowing which the surface can even see. Sampling both and letting
        // MIS weight each against BSDF sampling costs one extra shadow ray on
        // a lit surface and removes an entire class of "the HDRI stopped
        // contributing when I added a lamp".
        // Past the photons, a focusing surface's direct light is theirs too.
        if config.direct_lighting && !bsdf.flags().is_pure_delta() && !(read_caustics && smooth_here) {
            let environment_direct = sample_environment_light(
                intersector,
                config,
                &hit,
                &bsdf,
                &onb,
                wo,
                geometric_normal,
                sampler,
                &mut out,
                throughput,
            );
            let (analytic_direct, layer) = sample_analytic_lights(
                intersector,
                config,
                &hit,
                &bsdf,
                &onb,
                wo,
                geometric_normal,
                sampler,
                &mut out,
                throughput,
            );
            let environment_part = throughput * environment_direct;
            let analytic_part = throughput * analytic_direct;
            let contribution = environment_part + analytic_part;
            radiance += contribution;
            // Next-event estimation ends the path at a light *and then the path
            // keeps going*, so the matcher is rewound afterwards. Without that,
            // every pass beyond the first bounce is routed as if the path had
            // stopped — which still produces a plausible image and a wrong set
            // of passes.
            // The two are routed apart, so each reaches its own light's
            // layer: the environment's and whichever analytic light was
            // sampled.
            let saved = aovs.matcher.save();
            aovs.contribute(environment_part, crate::lpe::ENVIRONMENT);
            aovs.contribute(analytic_part, layer);
            aovs.matcher.restore(&saved);
        }

        // ---- Caustics (§6.5) ---------------------------------------------
        //
        // At the first surface the camera sees that is not mirror-smooth,
        // the light the photons brought: each photon's power, turned towards
        // the camera by this surface's BSDF, over the pass's disc.
        let reading_here = caustic_map.is_some() && focused_chain && !smooth_here;
        focused_chain = focused_chain && smooth_here;
        if let (true, Some(map)) = (reading_here, caustic_map) {
            let transmissive = bsdf.flags().contains(BsdfFlags::TRANSMISSION);
            let caustic = map.estimate(hit.position, geometric_normal, |from| {
                let wi = onb.to_local(from);
                if wi.z <= 0.0 && !transmissive {
                    Spectrum::ZERO
                } else {
                    bsdf.eval(wo, wi)
                }
            });
            let contribution = throughput * caustic;
            radiance += contribution;
            let saved = aovs.matcher.save();
            aovs.contribute(contribution, crate::lpe::ENVIRONMENT);
            aovs.matcher.restore(&saved);
        }

        // ---- BSDF sampling: extend the path ----------------------------
        let Some(sample) = bsdf.sample(wo, sampler.bsdf_direction(), sampler.bsdf_lobe()) else {
            break;
        };
        seen_as = if sample.is_transmission {
            Seen::Transmission
        } else if sample.flags.contains(BsdfFlags::DIFFUSE) {
            Seen::Diffuse
        } else {
            Seen::Glossy
        };
        // What the photons cover from here on. A mirror-smooth lobe of this
        // surface (a clear coat) was not in their estimate, which reads the
        // BSDF, so a path leaving by one is still this path's to count.
        if reading_here {
            read_caustics = !sample.is_delta;
            smooth_since = 0;
        } else if read_caustics {
            if smooth_here {
                smooth_since += 1;
            } else {
                read_caustics = false;
            }
        }
        if sample.pdf <= 0.0 || sample.f.is_black() {
            break;
        }
        let wi_world = onb.to_world(sample.wi);

        // Guard the shading-normal energy leak (§3.1): a reflection lobe must
        // not accept a direction that is above the shading horizon but below
        // the geometric one, or light leaks through solid geometry.
        if !sample.is_transmission && wi_world.dot(geometric_normal) <= 0.0 {
            break;
        }

        throughput *= sample.throughput_weight();
        if sample.is_transmission {
            eta_scale *= sample.eta * sample.eta;
            transmission_bounces += 1;
            if transmission_bounces > config.max_transmission_bounces {
                break;
            }
            // Crossing the interface enters the instance or leaves it. A
            // thin-walled surface stands for a shell with no inside to be in,
            // so passing through one leaves the path where it was — which is
            // also what keeps a pane held inside a glass block from being
            // mistaken for the way out of the block. The medium recorded is
            // the shaded material, with its own index rather than the
            // relative one this interface was given.
            if sample.flags.contains(BsdfFlags::SUBSURFACE) {
                media.cross_walk(hit.instance, shaded);
            } else if boundary {
                media.cross_with(hit.instance, shaded, scene.density(hit.instance));
            }
        }
        if !throughput.is_finite() || throughput.is_black() {
            break;
        }

        prev_pdf_solid_angle = if sample.is_delta {
            f32::INFINITY
        } else {
            sample.pdf
        };
        specular_chain = specular_chain && sample.is_delta;
        catcher_view = catcher_view
            && (sample.is_delta || (!sample.is_transmission && sampler.next_unstructured() < catcher_view_through(shaded)));

        // The event the path just made, for the light path expressions. Taken
        // from the sample rather than from the material, because what an
        // expression is about is what the *path* did: a rough metal hit at a
        // grazing angle scatters glossily whatever its parameters say.
        aovs.matcher.scatter(
            if sample.is_transmission {
                Transport::Transmit
            } else {
                Transport::Reflect
            },
            if sample.is_delta {
                Scatter::Specular
            } else if sample.flags.contains(BsdfFlags::DIFFUSE) {
                Scatter::Diffuse
            } else {
                Scatter::Glossy
            },
        );

        ray = Ray {
            origin: offset_ray_origin(
                hit.position,
                if sample.is_transmission {
                    -geometric_normal
                } else {
                    geometric_normal
                },
            ),
            direction: wi_world,
            t_min: 0.0,
            t_max: f32::INFINITY,
            time: ray.time,
        };
        bounce += 1;

        // ---- Path regularization (§3.5) --------------------------------
        // A bounded, deliberate bias: imposing a roughness floor on deep
        // vertices removes most specular-adjacent fireflies, which persist
        // visually far longer than uniform noise and break denoisers.
        if bounce >= config.regularization_start_bounce {
            roughness_floor = roughness_floor.max(config.regularization_roughness);
        }

        // ---- Russian roulette (§3.4) -----------------------------------
        if bounce >= config.russian_roulette_start_bounce {
            let survival = (throughput.max_component() * eta_scale).clamp(0.0, 0.95);
            if sampler.russian_roulette() >= survival {
                break;
            }
            throughput /= survival;
        }
    }

    out.radiance = radiance.sanitized();
    out.wavelength = wavelength;
    if out.normal == Vec3::ZERO {
        out.normal = -ray.direction;
    }
    out
}

/// The kind of ray a path is, as a part's visibility names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seen {
    Camera,
    /// The first ray of the floor's reflection.
    FloorMirror,
    Diffuse,
    Glossy,
    Transmission,
}

/// The chance that a path reflected off `material` still sees a shadow
/// catcher as the camera would: the backdrop with the floor's shadow on it,
/// rather than a floor lit as a surface.
///
/// A perfect mirror always does - it shows what the camera would see
/// through it. A lobe a hair rougher than the mirror threshold
/// (`microfacet::SMOOTH_ALPHA_THRESHOLD`, a roughness of about 0.032) used
/// never to, so the floor in a polished part changed abruptly across that
/// line, and a roughness pattern on polished metal drew hard-edged patches
/// of a different floor. The view now fades from one to the other as the
/// reflection blurs, by roughness 0.15, so what the floor looks like in a
/// reflection changes as smoothly as the reflection does.
fn catcher_view_through(material: &crate::material::Material) -> f32 {
    let mut roughness = material.specular_roughness;
    if material.coat_weight > 0.0 {
        roughness = roughness.min(material.coat_roughness);
    }
    let t = ((roughness - 0.0316) / (0.15 - 0.0316)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// What a shadow catcher contributes at one hit.
struct CaughtShadow {
    radiance: Spectrum,
    /// How much of the backdrop the floor hides: its shadow, and the share
    /// its reflection takes. The pixel's alpha, so that the floor over the
    /// backdrop it was rendered against is the floor in the beauty.
    coverage: f32,
    /// The denoiser's guides here, albedo and normal: where the floor
    /// reflects, the reflected surface's, as Cycles guides through a mirror.
    /// The floor's own radiance as its albedo carried the reflection's noise
    /// into the guide, so the denoiser kept it as detail - a speckled
    /// reflection.
    guide: (Spectrum, Vec3),
}

/// The ground plane, as a compositing operation rather than a surface.
///
/// The estimator is a ratio, not a lighting calculation: a catcher's job is to
/// report *how much light the product took away*, so what it computes is
/// `1 - shadowed / unshadowed`. Shading it as a diffuse floor and subtracting
/// would tie the result to the floor's albedo — which does not exist — and a
/// shadow would change colour whenever the environment did.
fn catch_shadow(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    hit: &Hit,
    catcher: &ShadowCatcher,
    view: Vec3,
    behind: Spectrum,
    bounce: u32,
    sampler: &mut Sampler,
) -> CaughtShadow {
    let normal = if hit.geometric_normal.dot(-view) > 0.0 {
        hit.geometric_normal
    } else {
        -hit.geometric_normal
    };

    // A floor there only for its reflection darkens by nothing.
    let coverage = if !catcher.shadows {
        0.0
    } else if catcher.occlusion_only {
        occlusion_at(intersector, hit, normal, catcher.occlusion_radius, sampler)
    } else {
        let coverage = shadow_at(intersector, config, hit, normal, bounce, sampler);
        sampler.begin_bounce(bounce);
        coverage
    };
    let mut coverage = coverage.clamp(0.0, 1.0);

    // The shadow is a *darkening of what is behind the floor*, so it mixes
    // towards the shadow colour rather than replacing the picture with it.
    let mut radiance = behind.lerp(catcher.shadow_color, coverage);
    if config.floor_reflection_only {
        // The shadow is Cycles' own, in the image this goes over.
        radiance = Spectrum::ZERO;
        coverage = 0.0;
    }

    let mut guide = (radiance, hit.shading_normal);
    if catcher.reflections > 0.0 {
        if let Some((seen, share)) = floor_reflection(intersector, config, hit, catcher, view, normal, sampler) {
            let reflected_normal = (hit.shading_normal * (1.0 - share) + seen.normal * share).try_normalize().unwrap_or(hit.shading_normal);
            let reflected_albedo = seen.albedo;
            let seen = seen.radiance;
            if config.floor_reflection_only {
                radiance = seen * share;
                coverage = share;
                guide = (reflected_albedo * share, reflected_normal);
            } else {
                // The reflection takes its share from what the floor shows,
                // as a glossy floor's does. Added on top, a floor seen low
                // showed the backdrop and most of the sky again, brighter
                // than the backdrop around it.
                radiance = radiance.lerp(seen, share);
                coverage += share * (1.0 - coverage);
                guide = (guide.0.lerp(reflected_albedo, share), reflected_normal);
            }
        }
    }

    CaughtShadow { radiance, coverage, guide }
}

/// What the floor reflects along one direction, and how much of it shows:
/// `None` where the reflection meets nothing standing on the floor.
///
/// Only what stands on the floor is reflected - the product, other parts,
/// a light - and never the environment, as a matte floor's reflection is in
/// a studio renderer: where the reflection would show the sky, the floor
/// shows what is behind it instead, so the floor itself never shows, only
/// the product mirrored in it. Reflecting the environment as well, a strong
/// reflection under an HDRI made the floor a mirror of the sky, a visible
/// surface rather than an invisible one.
///
/// The strength is the share at every angle, so 1 is a mirror; with
/// `reflection_fresnel` it is weakened by Schlick's share for a dielectric
/// floor, which is how a real glossy floor's grows toward grazing angles -
/// and which at an ordinary camera height left a tenth of the strength.
/// Blur samples a GGX microfacet normal of the floor's roughness.
fn floor_reflection(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    hit: &Hit,
    catcher: &ShadowCatcher,
    view: Vec3,
    normal: Vec3,
    sampler: &mut Sampler,
) -> Option<(PathResult, f32)> {
    let roughness = catcher.reflection_roughness.clamp(0.0, 1.0);
    let facet = if roughness > 1e-3 {
        let frame = Onb::from_normal(normal);
        let wo = frame.to_local(-view);
        let ggx = tequila_materials::microfacet::Ggx::from_roughness(roughness, 0.0);
        frame.to_world(ggx.sample_visible_normal(wo, sampler.bsdf_direction()))
    } else {
        normal
    };
    let reflected = (view - facet * (2.0 * view.dot(facet))).try_normalize()?;
    if reflected.dot(normal) <= 0.0 {
        return None;
    }
    let origin = tequila_core::math::offset_ray_origin(hit.position, normal);
    // A short reflection is gone above its fade, and most of what a floor
    // mirrors is above it: one intersection says so before a whole path is
    // traced and shaded to be thrown away. The first thing the ray meets is
    // never further than what the path would shade, so this skips only what
    // would have faded anyway.
    // By distance, the floor shows its mirror by how much the product closes
    // in on it within the fade, sampled as ambient occlusion is: most of the
    // floor has nothing near and is done with in one short ray.
    let mut near = 1.0;
    match catcher.reflection_fade {
        Some(fade) if catcher.reflection_fade_by_distance => {
            near = occlusion_at(intersector, hit, normal, fade, sampler);
            if near <= 0.0 {
                return None;
            }
        }
        Some(fade) => match intersector.intersect(&Ray::new(origin, reflected)) {
            None => return None,
            Some(first) if first.t * reflected.dot(normal) >= fade => return None,
            Some(_) => {}
        },
        None => {}
    }
    // One bounce, not a recursion: a floor reflecting a floor reflecting a
    // floor is a hall of mirrors nobody asked for, and the second bounce is
    // below the noise floor of a render anyone would ship.
    let mut mirror_config = config.clone();
    mirror_config.max_bounces = config.max_bounces.min(3);
    mirror_config.in_floor_reflection = true;
    mirror_config.floor_reflection_only = false;
    let seen = trace_path(intersector, &mirror_config, Ray::new(origin, reflected), Vec2::ZERO, sampler);
    if seen.escaped {
        return None;
    }
    let mut share = catcher.reflections.clamp(0.0, 1.0) * near;
    // A short reflection: fading with how high above the floor what it shows
    // is, as a photographer's faded reflection does, so it meets the product
    // at the floor and is gone further up.
    if let Some(fade) = catcher.reflection_fade.filter(|_| !catcher.reflection_fade_by_distance) {
        let height = seen.depth * reflected.dot(normal);
        let t = (height / fade.max(1e-6)).clamp(0.0, 1.0);
        share *= 1.0 - t * t * (3.0 - 2.0 * t);
        if share <= 0.0 {
            return None;
        }
    }
    if catcher.reflection_fresnel {
        let cos = (-view).dot(normal).abs().clamp(0.0, 1.0);
        share *= 0.04 + 0.96 * (1.0 - cos).powi(5);
    }
    Some((seen, share))
}

/// How many directions [`shadow_at`] draws before it keeps the one it has.
///
/// The draws are rejection sampling, so how many it takes is the sum of the
/// bounds over the light itself. On a floor facing up the sky and a point
/// lamp are drawn for exactly and the first draw is always kept; a softbox,
/// or a surface facing any other way, turns some away. Past this many the
/// last draw is kept whatever it carries, which tilts that rare sample
/// towards the proposal rather than leaving the floor unshadowed.
const MAX_SHADOW_DRAWS: u32 = 32;

/// Fraction of the light at this point that the scene blocks.
///
/// That is a ratio, blocked light over all of it, and an average of
/// per-sample ratios is not one. The catcher used to divide one sky sample
/// and one lamp sample at a time and average the quotients. Every sky sample
/// below the horizon carried nothing, so with a lamp blocked it reported the
/// floor fully shadowed, and under an HDRI alone it reported it unshadowed: a
/// lamp's shadow came out darker than Cycles' and a sun's lighter.
///
/// So this draws one direction in proportion to the light it would bring with
/// nothing in the way — from the sky and every lamp together, kept or not
/// against each one's bound — and says how much of that one direction is
/// blocked. The average of that is exactly the blocked share, and each sample
/// is a fraction from 0 to 1, as a coverage has to be. The shadow ray goes
/// where the light is, so a small bright sun's shadow is as sharp here as it
/// is on the product.
///
/// `bounce` is the vertex's, whose dimensions the first draw uses. Later
/// draws use the dimensions of the bounces after it, so leave the sampler on
/// some other bounce.
fn shadow_at(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    hit: &Hit,
    normal: Vec3,
    bounce: u32,
    sampler: &mut Sampler,
) -> f32 {
    if !config.shadows {
        return 0.0;
    }
    let scene = intersector.scene();
    let lights = scene.lights.lights();
    let sky = scene.environment.irradiance_bound(hit.position, normal);

    // Each lamp's bound, on the stack for the usual handful of lamps. Every
    // lamp, which a ratio has to include or a scene lit by lamps alone would
    // catch no shadow at all.
    let mut stack = [0.0f32; 16];
    let mut heap = Vec::new();
    let bounds: &mut [f32] = if lights.len() <= stack.len() {
        &mut stack[..lights.len()]
    } else {
        heap.resize(lights.len(), 0.0);
        &mut heap
    };
    let mut total = sky;
    for (bound, light) in bounds.iter_mut().zip(lights) {
        // A light kept off the floor neither lights it nor shadows it.
        *bound = if light.links.lights(hit.instance.0) { light.irradiance_bound(hit.position, normal) } else { 0.0 };
        total += *bound;
    }
    // Negated, so that a NaN bound is refused too.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    if !(total > 0.0) {
        return 0.0;
    }

    for draw in 0..MAX_SHADOW_DRAWS {
        // Each draw on dimensions stratified together: the vertex's own for
        // the first, which is the one that usually counts, and for the rest
        // those of the bounces the path would have made past here — it ends
        // at the catcher, so they are free. Past the sequence's last
        // dimension the sampler falls back to its open stream by itself.
        sampler.begin_bounce(bounce + draw);
        let (pick, u, chance) = (sampler.light_select(), sampler.light_sample(), sampler.bsdf_lobe());
        let last = draw + 1 == MAX_SHADOW_DRAWS;

        let mut along = pick * total;
        if along < sky || bounds.is_empty() {
            let Some(sample) = scene.environment.sample_irradiance(hit.position, normal, u) else {
                continue;
            };
            if chance < sample.keep || (last && sample.keep > 0.0) {
                return blocked_towards(intersector, hit, normal, sample.direction, sample.distance);
            }
            continue;
        }
        along -= sky;
        // The lamp `along` falls in; rounding past the end lands on the last.
        let mut chosen = bounds.len() - 1;
        for (i, bound) in bounds.iter().enumerate() {
            if along < *bound {
                chosen = i;
                break;
            }
            along -= bound;
        }
        let (light, bound) = (&lights[chosen], bounds[chosen]);
        if bound <= 0.0 {
            continue;
        }
        let Some(sample) = light.sample(hit.position, u) else {
            continue;
        };
        let density = if sample.is_delta { 1.0 } else { sample.pdf };
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(density > 0.0) {
            continue;
        }
        let keep = sample.radiance.luminance() * sample.direction.dot(normal).max(0.0) / density / bound;
        if chance < keep || (last && keep > 0.0) {
            return if light.visibility.shadow {
                blocked_towards(intersector, hit, normal, sample.direction, sample.distance)
            } else {
                0.0
            };
        }
    }
    0.0
}

/// Ambient occlusion at a point: the contact darkening of §1.5's Occlusion
/// The frame an anisotropic lobe is drawn in, whose tangent is the direction
/// the surface is rougher in (§3.2's Anisotropic type).
///
/// The direction belongs to the part, as a brushed finish does: the one the
/// material's graph draws (`drawn`, in world space - a brushing pattern's own
/// direction), else the mesh's own tangents where it has them, interpolated
/// so a curve does not show its facets, else the way its UVs run across the
/// triangle. Rotation turns it about the normal, 0 to 1 over a half turn —
/// after which an anisotropic lobe is the same again. Without any of those
/// there is no direction on the surface to follow, and the frame the normal
/// alone gives is the only one there is.
fn anisotropic_frame(scene: &crate::scene::SceneSnapshot, hit: &Hit, normal: Vec3, drawn: Option<Vec3>, rotation: f32) -> Onb {
    let along = drawn
        .filter(|t| t.length_squared() > 1e-12)
        .or_else(|| scene.tangent_frame(hit).map(|(tangent, _)| tangent))
        .or_else(|| scene.uv_derivatives(hit).map(|(dpdu, _)| dpdu));
    let frame = match along {
        Some(tangent) => Onb::from_normal_tangent(normal, tangent),
        None => Onb::from_normal(normal),
    };
    if rotation == 0.0 {
        return frame;
    }
    let (sin, cos) = (rotation * std::f32::consts::PI).sin_cos();
    Onb::from_normal_tangent(normal, frame.t * cos + frame.b * sin)
}

/// Ground Shadows.
fn occlusion_at(
    intersector: &dyn Intersector,
    hit: &Hit,
    normal: Vec3,
    radius: f32,
    sampler: &mut Sampler,
) -> f32 {
    let onb = Onb::from_normal(normal);
    let local = tequila_core::sampling::sample_cosine_hemisphere(sampler.light_sample());
    let direction = onb.to_world(local);
    // Bounded: occlusion is about what is *near*, and letting it reach the
    // whole scene turns a contact shadow into a second, wrong drop shadow.
    blocked_towards(intersector, hit, normal, direction, radius.max(1e-3))
}

/// How much of a shadow ray survives the surfaces between its ends.
///
/// 1 is a clear line, 0 is blocked. Without cutouts those are the only two
/// answers and this is [`Intersector::occluded`] with a different return type;
/// the whole point of the early exit is that a scene with nothing transparent
/// in it takes exactly the path it always did, on the query the tracer makes
/// more than any other.
///
/// # Why a product rather than a coin flip
///
/// A cutout is binary per photon, so the obvious implementation is to pass or
/// block at random with probability equal to the opacity. That is unbiased and
/// noisier than it needs to be. The *expected* fraction getting through a
/// sequence of independent holes is the product of their opacities, and here
/// that expectation can be computed outright rather than sampled — every
/// surface on the segment is visited anyway. Same answer, no variance.
///
/// The camera path cannot do this, because a ray has to end up somewhere
/// specific to keep going; only a shadow ray, whose two ends are already
/// known, can take the average instead of a sample.
fn shadow_transmittance(
    intersector: &dyn Intersector,
    from: Vec3,
    from_normal: Vec3,
    to: Vec3,
    to_normal: Vec3,
) -> f32 {
    if !intersector.scene().has_cutouts() {
        return if intersector.occluded(from, from_normal, to, to_normal) {
            0.0
        } else {
            1.0
        };
    }
    cutout_transmittance(intersector, from, from_normal, to, to_normal)
}

/// [`shadow_transmittance`] in a scene with cutouts: every surface on the
/// segment visited, and the light through their holes multiplied up.
///
/// Out of line so the walk's size never decides whether the common case —
/// one `occluded` — is inlined into the callers that make it per pixel.
#[inline(never)]
fn cutout_transmittance(
    intersector: &dyn Intersector,
    from: Vec3,
    from_normal: Vec3,
    to: Vec3,
    to_normal: Vec3,
) -> f32 {
    let scene = intersector.scene();
    let ray = crate::intersect::shadow_ray(from, from_normal, to, to_normal);
    let mut origin = ray.origin;
    let mut remaining = ray.t_max;
    let mut through = 1.0f32;
    // Shading scratch, made only if some surface on this ray actually needs
    // it. A material whose opacity is a number rather than a map — which is
    // every cutout until someone wires one in the editor — is answered from
    // the parameter block, so the usual walk allocates nothing.
    let mut scratch: Option<(shade::Registers, crate::material::Material)> = None;

    for _ in 0..MAX_CUTOUT_CROSSINGS {
        let mut segment = Ray::new(origin, ray.direction);
        segment.t_max = remaining;
        let Some(hit) = intersector.intersect(&segment) else {
            return through;
        };

        // A part that casts no shadow is open to a shadow ray whatever its
        // opacity, as it is to `occluded` on the way in.
        let opacity = match scene.opacity_of(hit.material) {
            _ if !scene.casts_shadows(hit.instance) => 0.0,
            Some(constant) => constant,
            None => {
                let (registers, material) = scratch.get_or_insert_with(|| {
                    (
                        shade::Registers::new(),
                        crate::material::Material::default(),
                    )
                });
                let (_, shading_normal) = hit.facing_forward(segment.direction);
                let point = scene.shading_point_along(&hit, shading_normal, crate::settings::DEFAULT_PIXEL_SPREAD, segment.direction);
                let program = scene
                    .program(hit.material)
                    .expect("a varying opacity comes from a program");
                shade::shade(program, &scene.textures, &point, registers, material)
                    .geometry_opacity
            }
        };

        // Transmittance is what is *not* solid. `geometry_opacity` is the
        // other way up — 1 is a wall, 0 is a hole — and the two agree at 0.5,
        // which is exactly where a test that only checks a half-open surface
        // cannot tell them apart.
        through *= 1.0 - opacity.clamp(0.0, 1.0);
        // Nothing past here can lighten it, and a shadow this deep is a
        // shadow.
        if through < 1e-3 {
            return 0.0;
        }

        let travelled = hit.t.max(1e-6);
        remaining -= travelled;
        if remaining <= 0.0 {
            return through;
        }
        origin = hit.position + ray.direction * 1e-4;
    }
    // Past the bound. Treated as blocked, which errs towards a shadow rather
    // than towards light leaking through a scene nobody meant to build.
    0.0
}

/// How much of a camera ray's pixel sample is covered, where the first
/// surface it meets is a cutout of `first` opacity and the ray carries on
/// from `start`: one less the light that gets through every surface along
/// it, parts hidden from the camera skipped.
///
/// The path itself decides at random whether to stop at a cutout or pass
/// it, and has to: it needs one place to carry on from. Its colour is
/// unbiased that way. But taking the pixel's alpha from the same coin flip
/// made it 0 or 1 per sample, and a lightbulb's glass, held a fifth present
/// by its opacity map, cut out as a grainy speckle where Cycles — which
/// averages, as this does — cuts it out smooth.
///
/// A shadow catcher along the ray ends the walk, and says so: its coverage
/// is its shadow's, which only the path can find, so the coverage here is
/// what the cutouts in front of it hold, and the path adds the shadow's
/// share where it reaches the catcher. The floor behind a lightbulb's glass
/// did this, and before it the glass there was grainy again.
#[inline(never)]
fn camera_coverage(intersector: &dyn Intersector, start: Vec3, direction: Vec3, first: f32) -> (f32, bool) {
    let scene = intersector.scene();
    let mut through = 1.0 - first.clamp(0.0, 1.0);
    let mut origin = start;
    let mut scratch: Option<(shade::Registers, crate::material::Material)> = None;
    for _ in 0..MAX_CUTOUT_CROSSINGS {
        if through < 1e-4 {
            return (1.0, false);
        }
        let Some(hit) = intersector.intersect(&Ray::new(origin, direction)) else {
            return (1.0 - through, false);
        };
        let instance = scene.instances.get(hit.instance.0 as usize);
        if instance.is_some_and(|i| i.shadow_catcher.is_some()) {
            return (1.0 - through, true);
        }
        let (geometric, shading_normal) = hit.facing_forward(direction);
        let next = tequila_core::math::offset_ray_origin(hit.position, -geometric);
        if instance.is_some_and(|i| !i.visible_to_camera) {
            origin = next;
            continue;
        }
        let opacity = match scene.opacity_of(hit.material) {
            Some(constant) => constant,
            None => {
                let (registers, material) = scratch.get_or_insert_with(|| {
                    (shade::Registers::new(), crate::material::Material::default())
                });
                let point = scene.shading_point_along(&hit, shading_normal, crate::settings::DEFAULT_PIXEL_SPREAD, direction);
                let program = scene.program(hit.material).expect("a varying opacity comes from a program");
                shade::shade(program, &scene.textures, &point, registers, material).geometry_opacity
            }
        };
        through *= 1.0 - opacity.clamp(0.0, 1.0);
        origin = next;
    }
    // Past the bound: covered, as a shadow past it is blocked.
    (1.0, false)
}

/// Whether this instance is a light's visible stand-in.
#[inline]
fn is_light_proxy(scene: &crate::scene::SceneSnapshot, instance: InstanceId) -> bool {
    scene
        .instances
        .get(instance.0 as usize)
        .is_some_and(|i| i.light_proxy)
}

/// The same ray carried on past the surface it just met, undeviated.
///
/// Cold: only a scene with a visible light ever takes it, and `trace_path`
/// is the last place to spend code on what most scenes never do.
#[cold]
#[inline(never)]
fn carried_past(hit: &Hit, ray: &Ray) -> Ray {
    let (geometric, _) = hit.facing_forward(ray.direction);
    Ray::new(offset_ray_origin(hit.position, -geometric), ray.direction)
}

/// How much of the light from `direction` the scene stops before it reaches
/// this point, 0 to 1.
fn blocked_towards(
    intersector: &dyn Intersector,
    hit: &Hit,
    normal: Vec3,
    direction: Vec3,
    distance: f32,
) -> f32 {
    let reach = if distance.is_finite() {
        distance * (1.0 - 1e-3)
    } else {
        intersector
            .scene()
            .bounds()
            .bounding_radius()
            .max(1.0)
            * 1e4
    };
    let target = hit.position + direction * reach;
    // A cutout casts a broken shadow onto the catcher too, so the matte asks
    // the same question the lighting does and takes the same answer: the
    // share that gets through. It used to split at half, so a part more than
    // half open left the floor lit and one less than half open, black.
    1.0 - shadow_transmittance(intersector, hit.position, normal, target, -direction)
}

/// Where a flight through a medium ends.
enum Flight {
    /// A tentative collision at `t` along the ray, in a stretch whose
    /// density is at most `bound` (1 in a uniform medium).
    Collided { t: f32, bound: f32 },
    /// The end of the stretch, having crossed this much optical depth —
    /// density times distance — without colliding.
    Reached { depth: f32 },
}

/// Fly an optical `depth` (density times distance) along `ray` from `from`,
/// stopping at `to`. In a uniform medium depth is distance; through a Volume
/// Map it is each majorant block's bound times the length crossed, so empty
/// blocks are passed for nothing and thin ones cheaply.
fn fly(field: Option<&crate::density::DensityField>, ray: &Ray, from: f32, to: f32, depth: f32) -> Flight {
    let Some(field) = field else {
        return if from + depth < to {
            Flight::Collided { t: from + depth, bound: 1.0 }
        } else {
            Flight::Reached { depth: to - from }
        };
    };
    let mut crossed = 0.0f32;
    let mut out = None;
    field.march(ray.origin, ray.direction, from, to, |start, end, bound| {
        if bound <= 0.0 {
            return true;
        }
        let here = (end - start) * bound;
        if crossed + here > depth {
            out = Some(Flight::Collided { t: start + (depth - crossed) / bound, bound });
            return false;
        }
        crossed += here;
        true
    });
    out.unwrap_or(Flight::Reached { depth: crossed })
}

/// Henyey-Greenstein's phase function, per steradian: `cos_theta` between
/// the direction light was travelling and the one it leaves in.
fn henyey_greenstein(cos_theta: f32, g: f32) -> f32 {
    let g = g.clamp(-0.99, 0.99);
    let d = (1.0 + g * g - 2.0 * g * cos_theta).max(1e-6);
    (1.0 - g * g) / (4.0 * std::f32::consts::PI * d * d.sqrt())
}

/// Direct light at a scattering event inside an index-matched medium: one
/// environment sample and one analytic light, each weighted by the phase
/// function and by the medium's transmittance towards it. Returned before the
/// path's throughput: the sky's share, the lights', and the lights' group.
#[allow(clippy::too_many_arguments)]
fn medium_direct_light(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    at: Vec3,
    forward: Vec3,
    walk: &tequila_materials::openpbr::SubsurfaceMedium,
    field: Option<&crate::density::DensityField>,
    instance: InstanceId,
    sampler: &mut Sampler,
) -> (Spectrum, Spectrum, crate::lpe::LightGroup) {
    let scene = intersector.scene();
    let reach = |distance: f32| {
        if distance.is_finite() {
            distance * (1.0 - 1e-3)
        } else {
            scene.bounds().bounding_radius().max(1.0) * 1e4
        }
    };
    let mut sky = Spectrum::ZERO;
    if let Some(light) = scene.environment.sample_from(at, sampler.light_sample()) {
        if light.pdf > 0.0 && !light.radiance.is_black() {
            let through = if config.shadows {
                medium_transmittance(intersector, at, light.direction, reach(light.distance), instance, walk, field, sampler)
            } else {
                Spectrum::ONE
            };
            let phase = henyey_greenstein(forward.dot(light.direction), walk.anisotropy);
            sky = light.radiance * through * (phase * mis_power(light.pdf, phase) / light.pdf);
        }
    }
    let mut lights = Spectrum::ZERO;
    let mut group = 0;
    if !scene.lights.is_empty() {
        // By power, not by the light tree: a point in a medium has no normal
        // for the tree to cull lights behind.
        let chosen = scene.lights.select(sampler.light_select());
        if let Some((light, p_select)) = chosen.and_then(|(id, p)| Some((scene.lights.get(id)?, p))).filter(|(light, _)| light.links.lights(instance.0)) {
            if let Some(sample) = light.sample(at, sampler.light_sample()) {
                if light.visibility.diffuse && !sample.radiance.is_black() && (sample.is_delta || sample.pdf > 0.0) {
                    let density = if sample.is_delta { p_select } else { sample.pdf * p_select };
                    let through = if config.shadows && light.visibility.shadow {
                        medium_transmittance(intersector, at, sample.direction, reach(sample.distance), instance, walk, field, sampler)
                    } else {
                        Spectrum::ONE
                    };
                    lights = sample.radiance
                        * through
                        * (henyey_greenstein(forward.dot(sample.direction), walk.anisotropy) / density);
                    group = light.layer.min(crate::lpe::LAST_LAYER);
                }
            }
        }
    }
    (sky, lights, group)
}

/// What survives from a point inside `instance`'s medium to `reach` along
/// `direction`: the medium's transmittance over every stretch inside it —
/// exact where it is homogeneous, by ratio tracking where a Volume Map varies
/// it — and nothing at all if anything else is in the way. The medium's own
/// surface is index-matched, so crossing it bends nothing.
#[allow(clippy::too_many_arguments)]
fn medium_transmittance(
    intersector: &dyn Intersector,
    from: Vec3,
    direction: Vec3,
    reach: f32,
    instance: InstanceId,
    walk: &tequila_materials::openpbr::SubsurfaceMedium,
    field: Option<&crate::density::DensityField>,
    sampler: &mut Sampler,
) -> Spectrum {
    let scene = intersector.scene();
    let sigma = walk.extinction;
    let majorant = sigma.r.max(sigma.g).max(sigma.b);
    let mut through = Spectrum::ONE;
    let mut origin = from;
    let mut remaining = reach;
    let mut inside = true;
    for _ in 0..MAX_CUTOUT_CROSSINGS {
        let mut segment = Ray::new(origin, direction);
        segment.t_max = remaining;
        let hit = intersector.intersect(&segment);
        let length = hit.as_ref().map_or(remaining, |h| h.t);
        if inside && majorant > 0.0 {
            match field {
                None => {
                    through = through
                        * Spectrum {
                            r: (-sigma.r * length).exp(),
                            g: (-sigma.g * length).exp(),
                            b: (-sigma.b * length).exp(),
                        };
                }
                Some(field) => {
                    // Ratio tracking through the majorant blocks: each
                    // tentative collision keeps what the medium there would
                    // not have stopped.
                    let segment = Ray::new(origin, direction);
                    let mut t = 0.0f32;
                    for _ in 0..MAX_NULL_STEPS {
                        let depth = -(1.0 - sampler.next_unstructured()).max(1e-12).ln() / majorant;
                        let (at, bound) = match fly(Some(field), &segment, t, length, depth) {
                            Flight::Collided { t, bound } => (t, bound),
                            Flight::Reached { .. } => break,
                        };
                        t = at;
                        let density = field.at(origin + direction * t) / bound.max(1e-12);
                        through = through
                            * Spectrum {
                                r: 1.0 - sigma.r / majorant * density,
                                g: 1.0 - sigma.g / majorant * density,
                                b: 1.0 - sigma.b / majorant * density,
                            };
                        // Russian roulette once little is left: deep in a cloud
                        // most shadow rays carry nothing, and tracking each to
                        // the edge was most of the render. Kept with the
                        // chance of what remains and scaled up by it, so the
                        // average is unchanged.
                        let most = through.r.max(through.g).max(through.b);
                        if most < 0.1 {
                            if sampler.next_unstructured() >= most {
                                return Spectrum::ZERO;
                            }
                            through = through * (1.0 / most);
                        }
                    }
                }
            }
        }
        match hit {
            None => return through,
            // The medium's own surface, and anything that casts no shadow,
            // are passed; anything else is in the way.
            Some(hit) if hit.instance == instance || !scene.casts_shadows(hit.instance) => {
                if hit.instance == instance {
                    inside = !inside;
                }
                remaining -= hit.t;
                if remaining <= 0.0 {
                    return through;
                }
                origin = hit.position + direction * 1e-4;
            }
            Some(_) => return Spectrum::ZERO,
        }
    }
    Spectrum::ZERO
}

/// One next-event-estimation sample against the environment light.
///
/// Returns the contribution *before* the path throughput is applied.
#[allow(clippy::too_many_arguments)]
fn sample_environment_light(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    hit: &Hit,
    bsdf: &impl Bsdf,
    onb: &Onb,
    wo: Vec3,
    geometric_normal: Vec3,
    sampler: &mut Sampler,
    out: &mut PathResult,
    throughput: Spectrum,
) -> Spectrum {
    let scene = intersector.scene();
    let Some(light) = scene
        .environment
        .sample_from(hit.position, sampler.light_sample())
    else {
        return Spectrum::ZERO;
    };
    if light.pdf <= 0.0 || light.radiance.is_black() {
        return Spectrum::ZERO;
    }

    // Which side of the surface the light is on. `geometric_normal` has
    // been turned to face the incoming ray, so "below" means the far side:
    // through the surface rather than in front of it.
    let below = light.direction.dot(geometric_normal) <= 0.0;

    // A surface that only reflects cannot be lit from behind, and refusing
    // those directions here saves the most expensive operation in the loop.
    // A surface that transmits *is* lit from behind, and that is where most
    // of its light comes from, so it is refused only by its own BSDF.
    //
    // Refusing it for everyone is not the conservative choice it looks like.
    // BSDF sampling still finds the far side, and the contribution it finds
    // is still weighted down by the chance that light sampling would have
    // found it too — a chance that never arrives if this returns first. The
    // weight is honest about a strategy that has been silently disabled, and
    // the difference is simply lost: a fifth of the light through a rough
    // glass sphere, a quarter through a rough shell, with nothing in the
    // image to say which part was missing.
    let transmissive = bsdf.flags().contains(BsdfFlags::TRANSMISSION);
    if below && !transmissive {
        return Spectrum::ZERO;
    }

    let wi = onb.to_local(light.direction);
    if wi.z == 0.0 || (wi.z < 0.0 && !transmissive) {
        return Spectrum::ZERO;
    }

    let f = bsdf.eval(wo, wi);
    if f.is_black() {
        return Spectrum::ZERO;
    }

    // MIS against the BSDF-sampling strategy, which could also have found this
    // direction. Both PDFs are in solid-angle measure at the shading point.
    let pdf_bsdf = bsdf.pdf(wo, wi);
    let weight = mis_power(light.pdf, pdf_bsdf);
    let unshadowed = f * light.radiance * (wi.z.abs() * weight / light.pdf);

    // §12's Shadow pass: the ratio of the light that would arrive to the light
    // that does. Accumulated here rather than derived afterwards, because the
    // two quantities differ only across this one shadow ray — anywhere else
    // they are the same number and the ratio is 1.
    out.direct_unshadowed += throughput * unshadowed;

    // How much of the light survives the trip. Without cutouts it is 1 or 0
    // and the shadow test could return early; with them it is a fraction that
    // has to reach every use of the contribution below.
    let mut visibility = 1.0f32;
    if config.shadows {
        // Aim the shadow ray at the dome, or — for a dome that is infinitely
        // far away — far outside the scene. Tracing past a *finite* dome would
        // let geometry behind the sky cast a shadow through it.
        //
        // `occluded` rather than `intersect`: an any-hit query can stop at the
        // first blocker instead of finding the nearest, which is roughly twice
        // as fast on the most frequent query a path tracer makes.
        let far = if light.distance.is_finite() {
            light.distance * (1.0 - 1e-3)
        } else {
            scene.bounds().bounding_radius().max(1.0) * 1e4
        };
        let target = hit.position + light.direction * far;
        // Offset to the side the ray is leaving from. Pushing a shadow ray
        // out along the normal it is heading *away* from starts it inside
        // the surface, where it hits its own geometry at once and reports
        // every transmitted direction as being in shadow.
        let leaving = if below {
            -geometric_normal
        } else {
            geometric_normal
        };
        visibility = shadow_transmittance(
            intersector,
            hit.position,
            leaving,
            target,
            -light.direction,
        );
        if visibility <= 0.0 {
            return Spectrum::ZERO;
        }
    }

    let unshadowed = unshadowed * visibility;
    out.direct_visible += throughput * unshadowed;
    unshadowed
}

/// One next-event-estimation sample against the scene's analytic lights.
///
/// Returns the contribution *before* the path throughput is applied.
#[allow(clippy::too_many_arguments)]
fn sample_analytic_lights(
    intersector: &dyn Intersector,
    config: &IntegratorConfig,
    hit: &Hit,
    bsdf: &impl Bsdf,
    onb: &Onb,
    wo: Vec3,
    geometric_normal: Vec3,
    sampler: &mut Sampler,
    out: &mut PathResult,
    throughput: Spectrum,
) -> (Spectrum, crate::lpe::LightGroup) {
    let scene = intersector.scene();
    if scene.lights.is_empty() {
        return (Spectrum::ZERO, 0);
    }

    // The selection's own probability divides the estimate. Taking it from the
    // set rather than assuming uniform is what lets the strategy change at
    // eight lights without the integrator knowing (§5.3).
    let Some((id, p_select)) =
        scene
            .lights
            .select_at(hit.position, geometric_normal, sampler.light_select())
    else {
        return (Spectrum::ZERO, 0);
    };
    let Some(light) = scene.lights.get(id) else {
        return (Spectrum::ZERO, 0);
    };
    // Light linking: a light kept to other parts lights nothing here.
    if !light.links.lights(hit.instance.0) {
        return (Spectrum::ZERO, 0);
    }

    // A dimension of its own, not the leftovers of the selection. Trying to
    // recover a variate from the coordinate the alias table consumed piles
    // every sample of a given light into the same corner of it — which reads
    // as the light losing energy as it is split into several, because the
    // estimator is averaging one corner's cosine rather than the light's.
    let Some(sample) = light.sample(hit.position, sampler.light_sample()) else {
        return (Spectrum::ZERO, 0);
    };
    if sample.radiance.is_black() {
        return (Spectrum::ZERO, 0);
    }
    if !sample.is_delta && sample.pdf <= 0.0 {
        return (Spectrum::ZERO, 0);
    }

    // The far side of a transmissive surface is lit, and refusing it here
    // is the same loss described in `sample_environment_light`. The light
    // *tree* already handles this correctly on its own: a light behind the
    // surface scores no importance, but the selection falls back to a coin
    // flip rather than giving up, and the probability it returns is divided
    // out — so such a light is chosen rarely and weighted heavily, which is
    // noisy but not dark.
    let below = sample.direction.dot(geometric_normal) <= 0.0;
    let transmissive = bsdf.flags().contains(BsdfFlags::TRANSMISSION);
    if below && !transmissive {
        return (Spectrum::ZERO, 0);
    }
    let wi = onb.to_local(sample.direction);
    if wi.z == 0.0 || (wi.z < 0.0 && !transmissive) {
        return (Spectrum::ZERO, 0);
    }

    // §5.4's visibility flags, which exist for the studio workflows this
    // application is for: a big soft key that appears in reflections but does
    // not wash the product out diffusely, a fill that casts no shadow. They
    // restrict which *lobes* the light reaches, not whether the light is
    // sampled — refusing the whole light would be a cruder feature.
    let f = bsdf.eval_selective(wo, wi, light.visibility.diffuse, light.visibility.specular);
    if f.is_black() {
        return (Spectrum::ZERO, 0);
    }

    // The unshadowed value first, for §12's Shadow pass. Computed before the
    // shadow ray, because it is the light that *would* arrive.
    let weight_preview = if sample.is_delta {
        1.0
    } else {
        mis_power(sample.pdf * p_select, bsdf.pdf(wo, wi))
    };
    let density_preview = if sample.is_delta {
        p_select
    } else {
        sample.pdf * p_select
    };
    let unshadowed = f * sample.radiance * (wi.z.abs() * weight_preview / density_preview);
    out.direct_unshadowed += throughput * unshadowed;

    let mut visibility = 1.0f32;
    if config.shadows && light.visibility.shadow {
        // Stop just short of the light, or the emitter's own surface blocks
        // the ray it was traced towards.
        let target = hit.position + sample.direction * (sample.distance * (1.0 - 1e-3));
        // As above: out of the side the ray actually leaves by.
        let leaving = if below {
            -geometric_normal
        } else {
            geometric_normal
        };
        visibility = shadow_transmittance(
            intersector,
            hit.position,
            leaving,
            target,
            -sample.direction,
        );
        if visibility <= 0.0 {
            return (Spectrum::ZERO, 0);
        }
    }
    out.direct_visible += throughput * unshadowed * visibility;

    // No MIS weight at all, for any analytic light.
    //
    // MIS divides a contribution between two strategies in proportion to how
    // likely each was to find it. That is only correct when both strategies
    // *can*. An analytic light has no geometry in the scene, so a BSDF sample
    // can never land on one — weighting against BSDF sampling would hand it a
    // share of the energy that it then never delivers, and the light comes out
    // dark in proportion to how rough the surface is.
    //
    // It showed up as a light losing a quarter of its output when split into
    // four coincident lights: splitting divides the selection probability, which
    // changes the light-side density, which changes the MIS weight — so the
    // amount silently discarded changed with it.
    //
    // Emissive *geometry* is the case where MIS does apply, because a BSDF ray
    // really can hit it; that path is `sample_environment_light`'s sibling and
    // is weighted there.
    let density = if sample.is_delta {
        p_select
    } else {
        sample.pdf * p_select
    };

    // `visibility`, not a second shadow test: the value returned here is the
    // unweighted one, and it is a *different number* from the `unshadowed`
    // preview above, which carries an MIS weight for the passes. Returning the
    // preview instead is exactly the quarter-of-its-output loss this comment
    // describes, and the test named for it says so within a second.
    (f * sample.radiance * (wi.z.abs() / density) * visibility, light.layer.min(crate::lpe::LAST_LAYER))
}

/// Whether a lobe's flags mark it as one next-event estimation can serve.
#[inline]
pub fn supports_next_event_estimation(flags: BsdfFlags) -> bool {
    !flags.is_pure_delta()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_materials::fresnel::{metals, ComplexIor};
    use crate::environment::Environment;
    use crate::material::Material;
    use crate::scene::SceneSnapshot;
    use crate::test_support::{AnalyticIntersector, Plane, Sphere};
    use tequila_core::math::{vec3, Vec2};
    use std::sync::Arc;

    /// Priority decides which medium is present where two overlap (§6.1),
    /// and a surface that does not change that is not an interface.
    #[test]
    fn the_higher_priority_medium_is_the_one_present() {
        let glass = Material {
            transmission_weight: 1.0,
            specular_ior: 1.5,
            ..Material::default()
        };
        let liquid = |priority: f32| Material {
            specular_ior: 1.33,
            medium_priority: priority,
            ..glass.clone()
        };
        let (bottle, fill) = (InstanceId(0), InstanceId(1));

        // In the glass wall, meeting the liquid's surface: a liquid that
        // outranks the glass is entered there...
        let mut media = MediumStack::default();
        media.cross(bottle, &glass);
        assert_eq!(media.beyond(fill, 1.0), Some(fill));
        media.cross(fill, &liquid(1.0));
        assert_eq!(media.top(None).map(|m| m.instance), Some(fill));
        // ...and inside it, the glass's own inner wall changes nothing: the
        // far side of it is still the liquid.
        assert_eq!(media.beyond(bottle, 0.0), Some(fill));

        // One the glass outranks is not: in the overlap it is the glass that
        // is there, so the liquid's surface inside the wall is no interface.
        let mut media = MediumStack::default();
        media.cross(bottle, &Material { medium_priority: 2.0, ..glass.clone() });
        assert_eq!(media.beyond(fill, 1.0), Some(bottle));

        // And the index across the real one is the ratio of the two.
        let mut media = MediumStack::default();
        media.cross(bottle, &glass);
        media.cross(fill, &liquid(1.0));
        let here = media.top(None).map(|m| m.instance);
        let there = media.beyond(fill, 1.0);
        assert_eq!((media.ior_of(here, None), media.ior_of(there, None)), (1.33, 1.5));
    }

    fn white_furnace_scene(material: Material) -> AnalyticIntersector {
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![material],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        AnalyticIntersector::new(scene).with_sphere(Sphere::new(
            Vec3::ZERO,
            1.0,
            tequila_core::ids::MaterialId(0),
        ))
    }

    /// The path tracer applies a material's bump. For most of this crate's
    /// life it did not: the bump module had its math and its tests, nothing
    /// called it, and every normal map and bump pattern rendered flat. The
    /// first-hit normal is what the BSDF was built around, so it shows
    /// whether the bump reached shading.
    #[test]
    fn a_bump_bends_the_normal_the_path_is_shaded_with() {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
        use tequila_materials::material_type::{MaterialType, Param};
        use tequila_texture::procedural::{Axis, Procedural};

        let normal_at = |bumped: bool, x: f32| {
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            // Soft contour lines across x: a height that rises and falls.
            let lines = graph.add_node(Node::new(NodeKind::Procedural {
                procedural: Box::new(Procedural::Contour {
                    axis: Axis::X,
                    frequency: 10.0,
                    width: 0.4,
                    softness: 1.0,
                    distortion: 0.0,
                }),
                seed: 0,
            }));
            let output = graph.output();
            if bumped {
                let bump = graph.add_node(Node::new(NodeKind::Bump { depth: 0.01 }));
                graph.connect(lines, bump, Socket::Input(0));
                graph.connect(bump, output, Socket::Param(Param::Bump));
            } else {
                graph.connect(lines, output, Socket::Param(Param::SpecularRoughness));
            }
            let program = graph.compile().expect("compiles");
            // One instance for the plane to shade through; the plane itself
            // is analytic.
            let quad = crate::scene::Mesh::new(
                vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
                vec![0, 1, 2],
            );
            let scene = SceneSnapshot::new(
                vec![Arc::new(quad)],
                vec![crate::scene::Instance::new(
                    tequila_core::ids::InstanceId(0),
                    tequila_core::ids::MeshId(0),
                    tequila_core::ids::MaterialId(0),
                )],
                vec![Material::diffuse(Spectrum::splat(0.5))],
                Arc::new(Environment::uniform(Spectrum::ONE)),
            )
            .with_programs(vec![Some(Arc::new(program))], Arc::new(Default::default()));
            let intersector = AnalyticIntersector::new(scene).with_plane(Plane::new(
                Vec3::ZERO,
                Vec3::Z,
                tequila_core::ids::MaterialId(0),
            ));
            let ray = Ray::new(vec3(x, 0.3, 2.0), vec3(0.0, 0.0, -1.0));
            let mut sampler = Sampler::new(0, 0, 0, 3);
            trace_path(&intersector, &IntegratorConfig::default(), ray, Vec2::ZERO, &mut sampler).normal
        };
        // Across a line's shoulder, where the height is changing.
        let mut tilted = 0;
        for i in 0..20 {
            let x = 0.013 + i as f32 * 0.0049;
            let flat = normal_at(false, x);
            assert!((flat - Vec3::Z).length() < 1e-5, "unbumped, the plane's normal moved: {flat:?}");
            let bent = normal_at(true, x);
            assert!(bent.z > 0.0, "a bump turned the surface over: {bent:?}");
            assert!(bent.y.abs() < 1e-4, "lines across x tilted the normal along y: {bent:?}");
            if (bent - Vec3::Z).length() > 0.05 {
                tilted += 1;
            }
        }
        assert!(tilted > 5, "only {tilted} of 20 points on the slopes were bumped");
    }

    fn average_radiance(
        intersector: &AnalyticIntersector,
        config: &IntegratorConfig,
        samples: u32,
    ) -> Spectrum {
        let mut sum = Spectrum::ZERO;
        for i in 0..samples {
            let mut sampler = Sampler::new(0, 0, i, 7);
            // Aim at the sphere from a fixed direction.
            let ray = Ray::new(vec3(0.0, 0.0, 4.0), vec3(0.0, 0.0, -1.0));
            sum += trace_path(intersector, config, ray, Vec2::ZERO, &mut sampler).radiance;
        }
        sum / samples as f32
    }

    /// The integrator's own white furnace: a white object inside a uniform
    /// environment of radiance 1 must be indistinguishable from the
    /// background. Any energy the transport loop loses or invents shows up
    /// here as a visible silhouette.
    ///
    /// This is a stronger statement than the per-lobe furnace tests: it
    /// exercises MIS weighting, Russian roulette compensation, the shading
    /// -normal guard, and the throughput bookkeeping all at once.
    #[test]
    fn white_furnace_holds_through_the_full_transport_loop() {
        let config = IntegratorConfig {
            max_bounces: 40,
            russian_roulette_start_bounce: 3,
            // Regularization is a deliberate bias; it must be off for a test
            // that asserts exact energy conservation.
            regularization_start_bounce: u32::MAX,
            ..IntegratorConfig::product()
        };
        // One per lobe configuration the stack can build, rather than the
        // same diffuse twice: a white furnace only proves what it is pointed
        // at, and layering is where §3.1 says energy is most often lost.
        let cases: [(&str, Material); 15] = [
            ("diffuse", Material::diffuse(Spectrum::ONE)),
            // Transmission was not covered here, and it is the configuration
            // that loses energy most easily: every vertex is a Fresnel split
            // and a refraction, and a sphere sends paths round the inside of
            // itself many times before they leave.
            ("clear glass", Material {
                transmission_weight: 1.0,
                transmission_color: Spectrum::ONE,
                transmission_depth: 0.0,
                geometry_thin_walled: false,
                specular_weight: 1.0,
                specular_roughness: 0.0,
                specular_ior: 1.5,
                base_weight: 1.0,
                base_color: Spectrum::ONE,
                ..Material::default()
            }),
            ("glass at ior 1", Material {
                transmission_weight: 1.0,
                transmission_color: Spectrum::ONE,
                transmission_depth: 0.0,
                geometry_thin_walled: false,
                specular_weight: 1.0,
                specular_roughness: 0.0,
                specular_ior: 1.0,
                base_weight: 1.0,
                base_color: Spectrum::ONE,
                ..Material::default()
            }),
            ("thin glass", Material {
                transmission_weight: 1.0,
                transmission_color: Spectrum::ONE,
                transmission_depth: 0.0,
                geometry_thin_walled: true,
                specular_weight: 1.0,
                specular_roughness: 0.0,
                specular_ior: 1.5,
                base_weight: 1.0,
                base_color: Spectrum::ONE,
                ..Material::default()
            }),
            // Rough enough to be a real test. At 0.3 a broken transmissive
            // path loses a couple of percent and can hide under a tolerance;
            // at 0.6 it loses a fifth of the light and cannot.
            ("rough glass", Material {
                transmission_weight: 1.0,
                transmission_color: Spectrum::ONE,
                transmission_depth: 0.0,
                geometry_thin_walled: false,
                specular_weight: 1.0,
                specular_roughness: 0.6,
                specular_ior: 1.5,
                base_weight: 1.0,
                base_color: Spectrum::ONE,
                ..Material::default()
            }),
            // A rough *shell* is the sharpest case of the lot: its
            // transmitted lobe is spread across the whole far hemisphere, so
            // it is the one most of whose light sits where next-event
            // estimation has to be willing to look.
            ("rough thin glass", Material {
                transmission_weight: 1.0,
                transmission_color: Spectrum::ONE,
                transmission_depth: 0.0,
                geometry_thin_walled: true,
                specular_weight: 1.0,
                specular_roughness: 0.6,
                specular_ior: 1.5,
                base_weight: 1.0,
                base_color: Spectrum::ONE,
                ..Material::default()
            }),
            // Fuzz was not covered here either, and it is the layer that
            // was most recently found inventing light: the sheen lobe
            // reflected up to 2.45 of what fell on it, for as long as the
            // catalog has had a Velvet in it. A furnace case is the check
            // that would have caught it at the transport level — and one at
            // a *tight* roughness is the one that would, because the excess
            // only appears below about 0.1 and no shipped preset goes there.
            ("velvet", Material {
                fuzz_weight: 1.0,
                fuzz_color: Spectrum::ONE,
                fuzz_roughness: 0.3,
                base_weight: 1.0,
                base_color: Spectrum::ONE,
                specular_weight: 0.0,
                ..Material::default()
            }),
            ("tight velvet", Material {
                fuzz_weight: 1.0,
                fuzz_color: Spectrum::ONE,
                fuzz_roughness: 0.05,
                base_weight: 1.0,
                base_color: Spectrum::ONE,
                specular_weight: 0.0,
                ..Material::default()
            }),
            // Anisotropy, which looks up isotropic energy tables with the
            // geometric mean of its two roughnesses. At 0.7 that proxy is
            // good to about a percent; `tequila-materials` carries the
            // measured surface of where it stops being good, which is why
            // the extreme end is pinned there rather than here.
            ("aniso metal", {
                let mut m = Material::metal(metals::ALUMINIUM, 0.3);
                m.metal_ior = ComplexIor {
                    n: Spectrum::splat(1.0),
                    k: Spectrum::splat(1e6),
                };
                m.specular_roughness_anisotropy = 0.7;
                m
            }),
            // Subsurface stands in as tinted diffuse until §6.4's random
            // walk lands. That makes this case a check that the stand-in is
            // at least energy-honest, and a tripwire for the day the real
            // transport replaces it.
            ("subsurface", Material {
                subsurface_weight: 1.0,
                subsurface_color: Spectrum::ONE,
                base_weight: 1.0,
                base_color: Spectrum::ONE,
                specular_weight: 0.0,
                ..Material::default()
            }),
            // A film over a perfect mirror must still be a perfect mirror:
            // interference moves energy between wavelengths and does not
            // make or lose any. Cheap, and it is the only transport-level
            // check the thin film has.
            ("film on metal", {
                let mut m = Material::metal(metals::ALUMINIUM, 0.2);
                m.metal_ior = ComplexIor {
                    n: Spectrum::splat(1.0),
                    k: Spectrum::splat(1e6),
                };
                m.thin_film_weight = 1.0;
                m.thin_film_thickness = 300.0;
                m.thin_film_ior = 2.0;
                m
            }),
            // Clearcoat over metal — car paint, and the most common
            // material in product rendering after plastic. It *invents*
            // three and a half percent, which is the same defect as the
            // velvet allowance below seen from the other side: the coat's
            // trapped-light gain assumes the substrate returns everything,
            // and over a mirror it very nearly does. Written down here
            // rather than left out of the furnace, because a case that is
            // not run is a case nobody is measuring.
            ("coated metal", {
                let mut m = Material::metal(metals::ALUMINIUM, 0.3);
                m.metal_ior = ComplexIor {
                    n: Spectrum::splat(1.0),
                    k: Spectrum::splat(1e6),
                };
                m.coat_weight = 1.0;
                m.coat_color = Spectrum::ONE;
                m
            }),
            ("plastic", Material::plastic(Spectrum::ONE, 0.4)),
            ("white metal", {
                let mut m = Material::metal(metals::ALUMINIUM, 0.3);
                m.metal_ior = ComplexIor {
                    n: Spectrum::splat(1.0),
                    k: Spectrum::splat(1e6),
                };
                m
            }),
            ("coated plastic", Material {
                coat_weight: 1.0,
                coat_color: Spectrum::ONE,
                ..Material::plastic(Spectrum::ONE, 0.4)
            }),
        ];
        let mut bad = Vec::new();
        for (name, material) in cases {
            let intersector = white_furnace_scene(material.clone());
            let result = average_radiance(&intersector, &config, 20_000);
            // Printed rather than only asserted on: when this test fails it
            // is usually one case that moved, and the eight numbers side by
            // side say which part of the stack to look at.
            println!("  {name:18} furnace = {:.4}", result.g);
            // One tolerance for all fifteen. The three cases that used to
            // carry written-down allowances — velvet and tight velvet too
            // dark, coated metal too bright — were all
            // `OpenPbrBsdf::attenuation` composing the layers by multiplying
            // independent per-layer factors, each one assuming everything
            // beneath it returned all of the light. That function now
            // composes the stack recursively and all three land at one, so
            // there is nothing left for an allowance to hold open.
            let tolerance = 0.02;
            if (result.g - 1.0).abs() >= tolerance {
                bad.push(format!("{name} = {:.4}", result.g));
            }
        }
        assert!(
            bad.is_empty(),
            "a white object in a uniform environment of radiance 1 must be \
             invisible; these lost or invented energy: {bad:?}"
        );
    }

    /// Next-event estimation and BSDF sampling must converge to the same
    /// image, and so must their MIS combination (`docs/05` §3.3). Disagreement
    /// localizes a weighting bug immediately.
    #[test]
    fn nee_bsdf_and_mis_agree() {
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![Material::plastic(Spectrum::splat(0.6), 0.35)],
            Arc::new(Environment::uniform(Spectrum::splat(0.7))),
        );
        let intersector = AnalyticIntersector::new(scene).with_plane(Plane::new(
            Vec3::ZERO,
            Vec3::Y,
            tequila_core::ids::MaterialId(0),
        ));

        let mut config = IntegratorConfig::product();
        config.max_bounces = 4;

        // MIS (both strategies).
        let mis = {
            let mut sum = Spectrum::ZERO;
            let n = 40_000;
            for i in 0..n {
                let mut s = Sampler::new(1, 1, i, 11);
                let ray = Ray::new(vec3(0.0, 1.0, 0.0), vec3(0.3, -0.8, 0.2).normalize());
                sum += trace_path(&intersector, &config, ray, Vec2::ZERO, &mut s).radiance;
            }
            sum / n as f32
        };

        // BSDF sampling only: with NEE disabled the emitter is found purely by
        // scattering, and the MIS weight collapses to 1 because there is no
        // counterpart strategy.
        let bsdf_only = {
            let mut c = config.clone();
            c.direct_lighting = false;
            let mut sum = Spectrum::ZERO;
            let n = 40_000;
            for i in 0..n {
                let mut s = Sampler::new(1, 1, i, 11);
                let ray = Ray::new(vec3(0.0, 1.0, 0.0), vec3(0.3, -0.8, 0.2).normalize());
                sum += trace_path(&intersector, &c, ray, Vec2::ZERO, &mut s).radiance;
            }
            sum / n as f32
        };

        let relative = (mis.g - bsdf_only.g).abs() / bsdf_only.g.max(1e-6);
        assert!(
            relative < 0.02,
            "MIS {} disagrees with BSDF-only {}",
            mis.g,
            bsdf_only.g
        );
    }

    /// A shadowing object must darken what is behind it.
    ///
    /// The occluder is **black**, deliberately. A grey one would both block
    /// the sky and bounce light back, so the measurement would confound
    /// shadowing with inter-reflection and could only support a weak
    /// assertion. With a black occluder the entire difference is shadowing,
    /// and the expected magnitude can be predicted: a sphere of radius 1 whose
    /// centre is 1.2 above the shading point subtends a half-angle of
    /// `asin(1/1.2)`, and a cone of half-angle `a` covers `sin^2(a)` of the
    /// cosine-weighted hemisphere — about 69% of the incoming light.
    #[test]
    fn occluders_cast_shadows() {
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![
                Material::diffuse(Spectrum::splat(0.8)),
                Material::diffuse(Spectrum::ZERO),
            ],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        let ground = tequila_core::ids::MaterialId(0);
        let black = tequila_core::ids::MaterialId(1);

        let open = AnalyticIntersector::new(scene.clone()).with_plane(Plane::new(
            Vec3::ZERO,
            Vec3::Y,
            ground,
        ));
        let shadowed = AnalyticIntersector::new(scene)
            .with_plane(Plane::new(Vec3::ZERO, Vec3::Y, ground))
            .with_sphere(Sphere::new(vec3(0.0, 1.2, 0.0), 1.0, black));

        let config = IntegratorConfig::product();
        let measure = |i: &AnalyticIntersector| {
            let mut sum = Spectrum::ZERO;
            let n = 20_000;
            for k in 0..n {
                let mut s = Sampler::new(2, 2, k, 13);
                // Approach the origin almost horizontally, so the view ray
                // passes well clear of the occluder overhead. A 45-degree
                // approach would strike the sphere itself and measure nothing
                // but the occluder. A Lambertian surface looks the same from
                // any angle, so the grazing view costs the test nothing.
                let ray = Ray::new(vec3(10.0, 0.5, 0.0), vec3(-10.0, -0.5, 0.0).normalize());
                sum += trace_path(i, &config, ray, Vec2::ZERO, &mut s).radiance;
            }
            (sum / n as f32).g
        };

        let lit = measure(&open);
        let dark = measure(&shadowed);
        let blocked_fraction = 1.0 - dark / lit;
        let expected = (1.0f32 / 1.2).powi(2); // sin^2 of the half-angle

        assert!(
            (blocked_fraction - expected).abs() < 0.1,
            "occluder blocked {blocked_fraction:.3} of the light, geometry predicts \
             {expected:.3} (lit {lit}, shadowed {dark})"
        );
        assert!(
            dark > 0.0,
            "shadow is totally black; ambient light was lost"
        );
    }

    /// A part with its shadow switched off lets a shadow ray through, by
    /// either road: `occluded` in an ordinary scene, and the cutout walk —
    /// which finds surfaces with `intersect`, and so meets every part — once
    /// any material in the scene is a cutout.
    #[test]
    fn a_part_that_casts_no_shadow_lets_the_light_through() {
        let through = |casts_shadows: bool, cutouts: bool| {
            let mut part = crate::scene::Instance::new(
                tequila_core::ids::InstanceId(0),
                tequila_core::ids::MeshId(0),
                tequila_core::ids::MaterialId(0),
            );
            part.casts_shadows = casts_shadows;
            let mut materials = vec![Material::diffuse(Spectrum::splat(0.5))];
            if cutouts {
                // On nothing; a cutout anywhere is what sends shadow rays
                // down the walk.
                materials.push(Material {
                    geometry_opacity: 0.5,
                    ..Material::default()
                });
            }
            let scene = SceneSnapshot::new(
                Vec::new(),
                vec![part],
                materials,
                Arc::new(Environment::uniform(Spectrum::ONE)),
            );
            assert_eq!(scene.has_cutouts(), cutouts);
            let intersector = AnalyticIntersector::new(scene).with_sphere(Sphere::new(
                Vec3::ZERO,
                1.0,
                tequila_core::ids::MaterialId(0),
            ));
            shadow_transmittance(
                &intersector,
                vec3(0.0, 5.0, 0.0),
                -Vec3::Y,
                vec3(0.0, -5.0, 0.0),
                Vec3::Y,
            )
        };
        for cutouts in [false, true] {
            assert_eq!(through(true, cutouts), 0.0, "an opaque part let light by (cutouts: {cutouts})");
            assert_eq!(
                through(false, cutouts),
                1.0,
                "a part that casts no shadow still blocked the light (cutouts: {cutouts})"
            );
        }
    }

    /// Light into a medium passes what casts no shadow too, or smoke under a
    /// softbox sits in the shadow of the panel the camera sees it as.
    #[test]
    fn light_reaches_into_a_medium_past_a_part_that_casts_no_shadow() {
        let reaches = |casts_shadows: bool| {
            let instance = |i: u32| {
                crate::scene::Instance::new(
                    InstanceId(i),
                    tequila_core::ids::MeshId(0),
                    tequila_core::ids::MaterialId(0),
                )
            };
            let mut panel = instance(1);
            panel.casts_shadows = casts_shadows;
            let scene = SceneSnapshot::new(
                Vec::new(),
                vec![instance(0), panel],
                vec![Material::default()],
                Arc::new(Environment::uniform(Spectrum::ONE)),
            );
            // The medium round the origin, and the part above it.
            let intersector = AnalyticIntersector::new(scene)
                .with_sphere(Sphere::new(Vec3::ZERO, 1.0, tequila_core::ids::MaterialId(0)))
                .with_sphere(Sphere::new(vec3(0.0, 3.0, 0.0), 0.5, tequila_core::ids::MaterialId(0)));
            let clear = tequila_materials::openpbr::SubsurfaceMedium {
                extinction: Spectrum::ZERO,
                albedo: Spectrum::ONE,
                anisotropy: 0.0,
            };
            medium_transmittance(
                &intersector,
                Vec3::ZERO,
                Vec3::Y,
                10.0,
                InstanceId(0),
                &clear,
                None,
                &mut Sampler::new(0, 0, 0, 1),
            )
        };
        assert_eq!(reaches(true), Spectrum::ZERO, "a part that casts shadows let light by");
        assert_eq!(reaches(false), Spectrum::ONE, "a part that casts no shadow still blocked the light");
    }

    /// Radiance must rise monotonically and converge as the bounce limit
    /// rises (`docs/05` §3.3): each extra bounce can only add light.
    #[test]
    fn radiance_converges_as_bounces_increase() {
        // A sphere resting on a plane: the two surfaces see each other, so
        // each extra bounce adds a measurable amount of inter-reflected light.
        // A lone convex object would show no change at all, because there is
        // nothing for it to bounce light off.
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![Material::diffuse(Spectrum::splat(0.85))],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        let material = tequila_core::ids::MaterialId(0);
        let intersector = AnalyticIntersector::new(scene)
            .with_sphere(Sphere::new(vec3(0.0, 1.0, 0.0), 1.0, material))
            .with_plane(Plane::new(Vec3::ZERO, Vec3::Y, material));
        let mut previous = 0.0f32;
        let mut values = Vec::new();
        for max_bounces in [1u32, 2, 4, 8, 16] {
            let config = IntegratorConfig {
                max_bounces,
                regularization_start_bounce: u32::MAX,
                ..IntegratorConfig::product()
            };
            let v = {
                let mut sum = Spectrum::ZERO;
                let n = 8_000;
                for i in 0..n {
                    let mut s = Sampler::new(0, 0, i, 7);
                    // Look into the crevice where the sphere meets the plane,
                    // which is where inter-reflection is strongest.
                    let ray = Ray::new(vec3(0.0, 0.35, 3.0), vec3(0.0, -0.08, -1.0).normalize());
                    sum += trace_path(&intersector, &config, ray, Vec2::ZERO, &mut s).radiance;
                }
                (sum / n as f32).g
            };
            assert!(
                v >= previous - 1e-3,
                "radiance fell from {previous} to {v} at {max_bounces} bounces"
            );
            previous = v;
            values.push(v);
        }
        // Converged: the last step must add far less than the first.
        let first_step = values[1] - values[0];
        let last_step = values[4] - values[3];
        assert!(
            last_step < first_step * 0.25,
            "not converging: steps {first_step} then {last_step}"
        );
    }

    #[test]
    fn escaped_rays_are_flagged_and_carry_the_background() {
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![Material::default()],
            Arc::new(Environment::uniform(Spectrum::splat(2.0))),
        );
        let intersector = AnalyticIntersector::new(scene);
        let mut s = Sampler::new(0, 0, 0, 1);
        let r = trace_path(
            &intersector,
            &IntegratorConfig::product(),
            Ray::new(Vec3::ZERO, Vec3::Y),
            Vec2::ZERO,
            &mut s,
        );
        assert!(r.escaped);
        assert!(r.depth.is_infinite());
        assert!((r.radiance.g - 2.0).abs() < 1e-4);
        // All of it background, which an image with alpha leaves out.
        assert!((r.background.g - 2.0).abs() < 1e-4, "{:?}", r.background);
    }

    /// Denoiser guides must come from the first *non-specular* vertex, so that
    /// a mirror reports what it reflects rather than a flat white.
    #[test]
    fn guides_come_from_the_first_diffuse_vertex() {
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![Material::diffuse(Spectrum {
                r: 0.9,
                g: 0.2,
                b: 0.2,
            })],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        let intersector = AnalyticIntersector::new(scene).with_plane(Plane::new(
            Vec3::ZERO,
            Vec3::Y,
            tequila_core::ids::MaterialId(0),
        ));
        let mut s = Sampler::new(0, 0, 0, 1);
        let r = trace_path(
            &intersector,
            &IntegratorConfig::product(),
            Ray::new(vec3(0.0, 2.0, 0.0), vec3(0.0, -1.0, 0.0)),
            Vec2::ZERO,
            &mut s,
        );
        assert!(
            r.albedo.r > r.albedo.b * 2.0,
            "albedo guide wrong: {:?}",
            r.albedo
        );
        assert!((r.normal - Vec3::Y).length() < 1e-4);
        assert!((r.depth - 2.0).abs() < 1e-3);
    }

    /// Determinism (§4.6): identical inputs must give a bit-identical result.
    #[test]
    fn tracing_is_deterministic() {
        let intersector = white_furnace_scene(Material::plastic(Spectrum::splat(0.5), 0.3));
        let config = IntegratorConfig::product();
        for i in [0u32, 17, 333] {
            let run = || {
                let mut s = Sampler::new(4, 9, i, 21);
                trace_path(
                    &intersector,
                    &config,
                    Ray::new(vec3(0.5, 0.5, 4.0), vec3(0.0, 0.0, -1.0)),
                    Vec2::ZERO,
                    &mut s,
                )
                .radiance
            };
            assert_eq!(run(), run());
        }
    }

    /// Every path must produce finite, non-negative radiance. A single NaN
    /// entering the accumulation buffer poisons that pixel permanently.
    #[test]
    fn radiance_is_always_finite_and_non_negative() {
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![
                Material::metal(tequila_materials::fresnel::metals::GOLD, 0.0),
                Material::plastic(Spectrum::ONE, 0.0),
            ],
            Arc::new(Environment::uniform(Spectrum::splat(5.0))),
        );
        let intersector = AnalyticIntersector::new(scene)
            .with_sphere(Sphere::new(
                Vec3::ZERO,
                1.0,
                tequila_core::ids::MaterialId(0),
            ))
            .with_plane(Plane::new(
                vec3(0.0, -1.0, 0.0),
                Vec3::Y,
                tequila_core::ids::MaterialId(1),
            ));
        let config = IntegratorConfig::jewelry();
        for i in 0..5_000 {
            let mut s = Sampler::new(i % 31, i % 17, i, 5);
            let jitter = Vec2 {
                x: (i % 23) as f32 / 23.0 - 0.5,
                y: (i % 29) as f32 / 29.0 - 0.5,
            };
            let ray = Ray::new(
                vec3(jitter.x * 3.0, jitter.y * 3.0 + 0.5, 5.0),
                vec3(0.0, 0.0, -1.0),
            );
            let r = trace_path(&intersector, &config, ray, Vec2::ZERO, &mut s);
            assert!(r.radiance.is_finite(), "sample {i}: {:?}", r.radiance);
            assert!(r.radiance.min_component() >= 0.0, "sample {i}");
        }
    }

    /// A shadow-catching floor through the origin with a sphere over it: the
    /// sphere is instance 0 and the floor instance 1, as the analytic
    /// intersector numbers them. A triangle that nothing intersects gives the
    /// scene its size, so a shadow ray towards the sky has a finite end.
    fn catcher_scene(
        environment: Environment,
        lights: Vec<crate::light::Light>,
        sphere: Sphere,
        sphere_material: Material,
    ) -> AnalyticIntersector {
        let size = crate::scene::Mesh::new(
            vec![vec3(-4.0, 0.0, -4.0), vec3(4.0, 4.0, 4.0), vec3(4.0, 0.0, -4.0)],
            vec![0, 1, 2],
        );
        let instances = vec![
            crate::scene::Instance::new(InstanceId(0), tequila_core::ids::MeshId(0), tequila_core::ids::MaterialId(0)),
            crate::scene::Instance::new(InstanceId(1), tequila_core::ids::MeshId(1), tequila_core::ids::MaterialId(0))
                .with_shadow_catcher(ShadowCatcher::default()),
        ];
        let scene = SceneSnapshot::new(vec![Arc::new(size)], instances, vec![sphere_material], Arc::new(environment))
            .with_lights(Arc::new(crate::light::LightSet::new(lights)));
        AnalyticIntersector::new(scene)
            .with_sphere(sphere)
            .with_plane(Plane::new(Vec3::ZERO, Vec3::Y, tequila_core::ids::MaterialId(0)))
    }

    /// The share of the light onto the floor at the origin that the scene
    /// blocks, the long way round: a fine grid over the sky, each direction
    /// weighted by the light it brings, and every lamp at its full worth. Of
    /// everything a catcher reports, this is the number it is estimating.
    fn blocked_share(intersector: &AnalyticIntersector) -> f32 {
        let scene = intersector.scene();
        let blocked_towards =
            |direction: Vec3, distance: f32| intersector.occluded(Vec3::ZERO, Vec3::Y, direction * distance, -direction);
        let (mut blocked, mut total) = (0.0f64, 0.0f64);
        let (columns, rows) = (1440, 360);
        for row in 0..rows {
            let theta = (row as f32 + 0.5) / rows as f32 * std::f32::consts::FRAC_PI_2;
            for column in 0..columns {
                let phi = (column as f32 + 0.5) / columns as f32 * std::f32::consts::TAU;
                let direction = vec3(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
                let weight = f64::from(scene.environment.radiance(direction).luminance() * theta.cos() * theta.sin());
                total += weight;
                if blocked_towards(direction, 100.0) {
                    blocked += weight;
                }
            }
        }
        // From the grid's sum to irradiance.
        let cell = f64::from(std::f32::consts::FRAC_PI_2 / rows as f32 * std::f32::consts::TAU / columns as f32);
        let (mut blocked, mut total) = (blocked * cell, total * cell);
        for light in scene.lights.lights() {
            let sample = light.sample(Vec3::ZERO, Vec2 { x: 0.5, y: 0.5 }).expect("the lamp lights the floor");
            assert!(sample.is_delta, "the long way round counts a lamp as a point");
            let weight = f64::from(sample.radiance.luminance() * sample.direction.dot(Vec3::Y).max(0.0));
            total += weight;
            if blocked_towards(sample.direction, sample.distance * 0.999) {
                blocked += weight;
            }
        }
        (blocked / total) as f32
    }

    /// The coverage a catcher reports at the origin, averaged over many
    /// samples, and the most any one of them reported.
    fn caught_at_origin(intersector: &AnalyticIntersector) -> (f32, f32) {
        let config = IntegratorConfig { shadows: true, ..IntegratorConfig::product() };
        let n = 24_000u32;
        let (mut sum, mut most) = (0.0f64, 0.0f32);
        for i in 0..n {
            let mut sampler = Sampler::new(3, 5, i, 29);
            // In low from the side, clear of the sphere overhead.
            let ray = Ray::new(vec3(4.0, 1.0, 0.5), vec3(-4.0, -1.0, -0.5).normalize());
            let result = trace_path(intersector, &config, ray, Vec2::ZERO, &mut sampler);
            assert_eq!(result.object_id, 1, "the camera ray missed the floor");
            sum += f64::from(result.coverage);
            most = most.max(result.coverage);
        }
        ((sum / f64::from(n)) as f32, most)
    }

    /// A lamp's shadow under a sky darkens the floor by the lamp's share of
    /// the light, no more. The catcher used to divide one sky sample and one
    /// lamp sample at a time and average the quotients: half the sky samples
    /// fall below the horizon and count for nothing, and with the lamp blocked
    /// each of those reported the floor fully shadowed. The shadow came out
    /// far darker than Cycles'.
    #[test]
    fn a_catcher_darkens_a_lamps_shadow_by_the_lamps_share_of_the_light() {
        let lamp = crate::light::Light::new(crate::light::LightKind::Point(crate::light::PointLight {
            position: vec3(0.0, 4.0, 0.0),
            intensity: Spectrum::splat(10.0),
            radius: 0.0,
        }));
        // The lamp and the sky about equal, so neither hides the other.
        let intersector = catcher_scene(
            Environment::uniform(Spectrum::splat(0.2)),
            vec![lamp],
            Sphere::new(vec3(0.0, 2.0, 0.0), 0.5, tequila_core::ids::MaterialId(0)),
            Material::diffuse(Spectrum::ZERO),
        );
        let expected = blocked_share(&intersector);
        let (caught, most) = caught_at_origin(&intersector);
        assert!(most <= 1.0, "a sample reported more than all of the light blocked: {most}");
        assert!(
            (caught - expected).abs() < 0.02,
            "the catcher reports {caught:.3} of the light blocked; {expected:.3} of it is"
        );
    }

    /// An HDRI's sun, blocked, leaves the floor as dark as the sun's share of
    /// the light says. With one sky sample per estimate, every sample that
    /// fell below the horizon reported the floor unshadowed, and the sun's
    /// shadow came out light: on the bulb, 67 against Cycles' 52.6.
    #[test]
    fn a_catcher_darkens_an_hdri_suns_shadow_by_the_suns_share_of_the_light() {
        let (width, height) = (128usize, 64usize);
        let sky = 0.3f32;
        let mut data = vec![sky; width * height * 3];
        // A sun three texels across, 45 degrees up, worth about three times
        // the rest of the sky on the floor. The ground half of the image is
        // as bright as the sky half, as it is outdoors.
        let (sun_x, sun_y) = (width * 5 / 8, height / 4);
        for y in sun_y - 1..=sun_y + 1 {
            for x in sun_x - 1..=sun_x + 1 {
                data[(y * width + x) * 3..(y * width + x) * 3 + 3].fill(250.0);
            }
        }
        let image = tequila_texture::Image {
            width,
            height,
            channels: 3,
            data,
            color_space: tequila_texture::ColorSpace::LinearSrgb,
        };
        let sun = tequila_core::sampling::equirect_to_direction(Vec2 {
            x: (sun_x as f32 + 0.5) / width as f32,
            y: (sun_y as f32 + 0.5) / height as f32,
        });
        // A black sphere straight towards the sun, wide enough to cover it
        // and its filtered edge.
        let intersector = catcher_scene(
            Environment::new(Arc::new(image), 1.0),
            Vec::new(),
            Sphere::new(sun * 3.0, 1.2, tequila_core::ids::MaterialId(0)),
            Material::diffuse(Spectrum::ZERO),
        );
        let expected = blocked_share(&intersector);
        assert!(expected > 0.6, "the sun is not most of the light: {expected}");
        let (caught, _) = caught_at_origin(&intersector);
        assert!(
            (caught - expected).abs() < 0.02,
            "the catcher reports {caught:.3} of the light blocked; {expected:.3} of it is"
        );
    }

    /// A see-through part casts the shadow its opacity says. Two walls of a
    /// sphere 40% opaque let 36% of the lamp's light through, so the floor
    /// is 64% shadowed. A catcher that called anything under half open fully
    /// blocked rendered it black.
    #[test]
    fn a_see_through_part_casts_a_partial_shadow_on_a_catcher() {
        let lamp = crate::light::Light::new(crate::light::LightKind::Point(crate::light::PointLight {
            position: vec3(0.0, 4.0, 0.0),
            intensity: Spectrum::splat(10.0),
            radius: 0.0,
        }));
        let intersector = catcher_scene(
            Environment::uniform(Spectrum::ZERO),
            vec![lamp],
            Sphere::new(vec3(0.0, 2.0, 0.0), 0.5, tequila_core::ids::MaterialId(0)),
            Material { geometry_opacity: 0.4, ..Material::diffuse(Spectrum::ZERO) },
        );
        assert!(intersector.scene().has_cutouts());
        let (caught, _) = caught_at_origin(&intersector);
        assert!((caught - 0.64).abs() < 0.01, "the floor is {caught:.3} shadowed under a 40%-opaque sphere");
    }
}
