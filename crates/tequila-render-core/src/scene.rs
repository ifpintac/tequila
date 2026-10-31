//! The immutable scene snapshot the renderer reads.
//!
//! `TEQUILA_SPEC.md` §2.2, rule 2: **the renderer never reads the live Scene
//! Document.** It reads an immutable snapshot handed to it by the
//! orchestrator. That is what lets the UI stay responsive while a render is in
//! flight, and it eliminates an entire class of data races by construction.
//!
//! Everything here is therefore `Arc`-shared and never mutated once built.
//! Editing means building a new snapshot; the dirty-state tracker (§9.3)
//! decides how much accumulated work survives.

use crate::environment::{Background, Environment};
use tequila_core::spectrum::Spectrum;
use crate::material::Material;
use crate::light::LightSet;
use crate::shade::TextureTable;
use tequila_materials::graph::CompiledGraph;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{Aabb, Transform, Vec2, Vec3};
use std::sync::Arc;

/// An indexed triangle mesh.
///
/// Positions are in the mesh's own object space; instances place them. Sharing
/// one mesh across many instances is not an optimization — `docs/01` §14 makes
/// the two-level BLAS/TLAS hierarchy "non-negotiable for CAD scenes with
/// thousands of repeated fasteners".
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    /// Vertex normals. Empty means "use geometric normals", which is correct
    /// for faceted geometry and must not be faked with a smooth default.
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    /// Per-vertex tangent frames, `xyz` along u and `w` the handedness in
    /// glTF's convention (`w * cross(n, t)` points up the image). Empty for a
    /// mesh without UVs; a normal map then falls back to the triangle's own
    /// frame.
    pub tangents: Vec<tequila_core::math::Vec4>,
    /// Per-vertex mean curvature in the mesh's own units (see
    /// `tequila_scene::geometry::vertex_curvature`). Empty when not measured.
    pub curvature: Vec<f32>,
    /// Per-vertex occluded fraction at the radius an Occlusion node asked
    /// for (`tequila_scene::occlusion`). Empty when nothing asked.
    pub occlusion: Vec<f32>,
    /// Per-vertex colours, linear RGBA. Empty when the mesh has none.
    pub colors: Vec<tequila_core::math::Vec4>,
    /// Triangle vertex indices, three per triangle.
    pub indices: Vec<u32>,
    /// The model's own edges as segment end points in pairs (a CAD part's
    /// B-rep edges), for the views that draw them. Not rendered.
    pub edge_lines: Vec<Vec3>,
    /// Fibres in place of triangles (§4.4's Fuzz): a mesh that has these has
    /// no triangles, and the renderers trace it as curves. Anything that
    /// reads a mesh only for its triangles - the Solid view, picking a face,
    /// a wireframe - sees an empty mesh, which is what fur is to those.
    pub fibres: Option<Arc<Fibres>>,
    bounds: Aabb,
}

/// Fibres standing out of a part's surface: §4.4's geometric Fuzz, grown by
/// `tequila_scene::fuzz` and rendered as curves.
///
/// Each fibre is a polyline of [`Fibres::POINTS`] points from its root to
/// its tip, with a radius at each, in the mesh's own space. Every fibre has
/// as many points as every other, so segment `s` belongs to fibre
/// `s / SEGMENTS` and no index table is needed.
#[derive(Clone, Debug, Default)]
pub struct Fibres {
    pub points: Vec<Vec3>,
    pub radii: Vec<f32>,
    /// A number from 0 to 1 per fibre, drawn when it was grown: how much of
    /// the material's colour variation darkens this fibre.
    pub random: Vec<f32>,
}

impl Fibres {
    /// Points along a fibre: four segments, enough for its curl to read as
    /// a curve at any distance a fibre is seen from.
    pub const POINTS: usize = 5;
    pub const SEGMENTS: usize = Self::POINTS - 1;

    pub fn count(&self) -> usize {
        self.random.len()
    }

    pub fn segment_count(&self) -> usize {
        self.count() * Self::SEGMENTS
    }

    /// The first point of segment `segment`; the next is its other end.
    #[inline]
    pub fn segment_start(segment: usize) -> usize {
        segment / Self::SEGMENTS * Self::POINTS + segment % Self::SEGMENTS
    }

    /// The box every fibre lies in, its thickness included.
    pub fn bounds(&self) -> Aabb {
        let mut bounds = Aabb::EMPTY;
        for (p, r) in self.points.iter().zip(&self.radii) {
            bounds.extend_point(*p - Vec3::splat(*r));
            bounds.extend_point(*p + Vec3::splat(*r));
        }
        bounds
    }
}

/// Where a ray met a fibre, as the fibre's BSDF reads it (see
/// `tequila_materials::lobes::Fibre`).
#[derive(Clone, Copy, Debug)]
pub struct FibreHit {
    /// Along the fibre, in world space.
    pub tangent: Vec3,
    /// Square to the fibre and toward the viewer: the shading normal.
    pub facing: Vec3,
    /// Where across the fibre the ray met it, -1 to 1 along
    /// `facing × tangent`.
    pub offset: f32,
    /// The fibre's own random number ([`Fibres::random`]).
    pub random: f32,
}

impl Mesh {
    pub fn new(positions: Vec<Vec3>, indices: Vec<u32>) -> Self {
        let mut bounds = Aabb::EMPTY;
        for p in &positions {
            bounds.extend_point(*p);
        }
        Mesh {
            positions,
            normals: Vec::new(),
            uvs: Vec::new(),
            tangents: Vec::new(),
            curvature: Vec::new(),
            occlusion: Vec::new(),
            colors: Vec::new(),
            indices,
            edge_lines: Vec::new(),
            fibres: None,
            bounds,
        }
    }

    /// A mesh of fibres and nothing else.
    pub fn of_fibres(fibres: Fibres) -> Self {
        let bounds = fibres.bounds();
        Mesh { fibres: Some(Arc::new(fibres)), bounds, ..Mesh::default() }
    }

    pub fn with_curvature(mut self, curvature: Vec<f32>) -> Self {
        debug_assert_eq!(curvature.len(), self.positions.len());
        self.curvature = curvature;
        self
    }

    pub fn with_tangents(mut self, tangents: Vec<tequila_core::math::Vec4>) -> Self {
        debug_assert_eq!(tangents.len(), self.positions.len());
        self.tangents = tangents;
        self
    }

    pub fn with_normals(mut self, normals: Vec<Vec3>) -> Self {
        debug_assert_eq!(normals.len(), self.positions.len());
        self.normals = normals;
        self
    }

    pub fn with_uvs(mut self, uvs: Vec<Vec2>) -> Self {
        debug_assert_eq!(uvs.len(), self.positions.len());
        self.uvs = uvs;
        self
    }

    #[inline]
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    #[inline]
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    /// Vertex indices of triangle `i`.
    #[inline]
    pub fn triangle(&self, i: usize) -> [u32; 3] {
        [
            self.indices[i * 3],
            self.indices[i * 3 + 1],
            self.indices[i * 3 + 2],
        ]
    }

    /// Geometric normal of triangle `i`, unnormalized (its length is twice the
    /// triangle's area, which the light sampler wants).
    #[inline]
    pub fn geometric_normal_scaled(&self, i: usize) -> Vec3 {
        let [a, b, c] = self.triangle(i);
        let (p0, p1, p2) = (
            self.positions[a as usize],
            self.positions[b as usize],
            self.positions[c as usize],
        );
        (p1 - p0).cross(p2 - p0)
    }

    /// Recompute smooth vertex normals by area-weighted averaging.
    ///
    /// Area weighting rather than a plain average: an unweighted average lets
    /// a cluster of tiny triangles outvote one large one, which shows up as
    /// shading artifacts exactly where a CAD tessellator puts its small
    /// triangles — along fillets.
    pub fn compute_smooth_normals(&mut self) {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for t in 0..self.triangle_count() {
            let n = self.geometric_normal_scaled(t);
            for &i in &self.triangle(t) {
                normals[i as usize] += n;
            }
        }
        for n in &mut normals {
            *n = n.try_normalize().unwrap_or(Vec3::Y);
        }
        self.normals = normals;
    }
}

/// A surface that exists to *catch* shadow and reflection rather than to be
/// seen (`TEQUILA_SPEC.md` §1.5's Ground).
///
/// A product render nearly always stands on one of these rather than on a
/// modelled floor: it has to take the product's shadow and reflection while
/// leaving the backplate or the environment visible through it, and come out
/// of the renderer with an alpha that composites. No ordinary surface does
/// that, because an ordinary surface is opaque and a shadow on it is a
/// *darkening of its own colour* rather than a coverage value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowCatcher {
    /// Tint of the caught shadow. Black is the usual choice.
    pub shadow_color: Spectrum,
    /// How much of a mirror the floor is, 0 to 1. Only what stands on it is
    /// reflected, never the environment (see `integrator::catch_shadow`).
    pub reflections: f32,
    /// The reflection's roughness, 0 (a mirror) to 1.
    pub reflection_roughness: f32,
    /// The height above the floor, in scene units, by which the reflection
    /// has faded out; `None` for one that never fades.
    pub reflection_fade: Option<f32>,
    /// Weaken the reflection by Fresnel's share, as a glossy floor's is.
    pub reflection_fresnel: bool,
    /// Darken by ambient occlusion rather than by a cast shadow. Useful where
    /// the lighting is soft enough that a drop shadow barely registers but the
    /// product still needs to sit on the floor rather than float.
    pub occlusion_only: bool,
    /// Radius the occlusion is measured over, in scene units.
    pub occlusion_radius: f32,
    /// Whether the floor darkens at all, by a shadow or by occlusion. Off,
    /// it is there only for its reflection - Ground's Catch shadows unticked
    /// with Reflections up - and was a full catcher all the same: the shadow
    /// the person had switched off stayed, under the reflection.
    pub shadows: bool,
    /// The reflection's alpha follows its brightness as displayed rather than
    /// its share of the pixel, so its dark parts are transparent: a picture
    /// on a white page shows the reflection's tint and never a grey band
    /// (V-Ray's matte "alpha contribution", Blender's compositor fix).
    pub reflection_luminance_alpha: bool,
    /// The reflection fades by how much the scene closes in on the floor
    /// point within [`ShadowCatcher::reflection_fade`], as ambient occlusion
    /// measures it, rather than by the height of what it shows.
    pub reflection_fade_by_distance: bool,
    /// What the product sees of the floor and the camera never does: an
    /// emission of this radiance, the soft white sweep of a photo studio
    /// that lifts the faces that mirror the floor. `None` for none.
    pub bounce: Option<Spectrum>,
    /// How far from the product the bounce darkens toward the contact line,
    /// like ambient occlusion over that distance, in scene units; 0 for not
    /// at all.
    pub bounce_falloff: f32,
}

impl Default for ShadowCatcher {
    fn default() -> Self {
        ShadowCatcher {
            shadow_color: Spectrum::ZERO,
            reflections: 0.0,
            reflection_roughness: 0.0,
            reflection_fade: None,
            reflection_fresnel: false,
            occlusion_only: false,
            occlusion_radius: 1.0,
            shadows: true,
            reflection_luminance_alpha: false,
            reflection_fade_by_distance: false,
            bounce: None,
            bounce_falloff: 0.0,
        }
    }
}

/// One renderable placement of a mesh.
#[derive(Clone, Debug)]
pub struct Instance {
    pub id: InstanceId,
    pub mesh: MeshId,
    pub transform: Transform,
    pub material: MaterialId,
    pub visible_to_camera: bool,
    pub casts_shadows: bool,
    /// Seen by the light other surfaces gather, their reflections, and what
    /// shows through their glass; a part none of these see is passed
    /// through by such rays, as one hidden from the camera is by its.
    pub seen_by_diffuse: bool,
    pub seen_by_glossy: bool,
    pub seen_by_transmission: bool,
    /// Cut out of the picture: where the camera sees it, nothing, with no
    /// alpha, for compositing (Cycles' holdout).
    pub holdout: bool,
    /// Set when this instance is a shadow catcher rather than a surface.
    pub shadow_catcher: Option<ShadowCatcher>,
    /// This geometry stands in for an analytic light: it is *seen*, but it does
    /// not light anything.
    ///
    /// §5.4 asks for a light that is visible to the camera, and an analytic
    /// light has no geometry to see. Giving it a proxy is the obvious answer
    /// and the obvious trap: an emissive surface in the scene would light it a
    /// second time, on top of the analytic light already doing so, and a
    /// softbox would come out twice as bright the moment it was made visible.
    /// The flag is what keeps the two apart — the proxy's emission counts only
    /// on a path the camera can still see along.
    pub light_proxy: bool,
    /// The light layer of the light this stands in for (§5.5), so seeing it
    /// directly lands in that light's pass.
    pub light_layer: u8,
}

impl Instance {
    pub fn new(id: InstanceId, mesh: MeshId, material: MaterialId) -> Self {
        Instance {
            id,
            mesh,
            transform: Transform::IDENTITY,
            material,
            visible_to_camera: true,
            casts_shadows: true,
            seen_by_diffuse: true,
            seen_by_glossy: true,
            seen_by_transmission: true,
            holdout: false,
            shadow_catcher: None,
            light_proxy: false,
            light_layer: 0,
        }
    }

    /// Mark this instance as the visible stand-in for an analytic light.
    ///
    /// Shadows are switched off with it: the analytic light is already the
    /// thing casting them, and a proxy that also occluded would shadow the
    /// very surfaces its light was sampled towards.
    pub fn as_light_proxy(mut self) -> Self {
        self.light_proxy = true;
        self.casts_shadows = false;
        self
    }

    pub fn with_light_layer(mut self, layer: u8) -> Self {
        self.light_layer = layer;
        self
    }

    pub fn with_shadow_catcher(mut self, catcher: ShadowCatcher) -> Self {
        self.shadow_catcher = Some(catcher);
        self
    }

    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }
}

/// The immutable snapshot handed to the render backends.
#[derive(Clone, Debug)]
pub struct SceneSnapshot {
    pub meshes: Vec<Arc<Mesh>>,
    pub instances: Vec<Instance>,
    pub materials: Vec<Material>,
    /// Compiled material graphs, parallel to `materials` (`docs/02` §4.3).
    ///
    /// Empty when no material in the scene has a graph, which is the common
    /// case and the one that must stay free — see
    /// [`crate::shade`](crate::shade).
    pub programs: Vec<Option<Arc<CompiledGraph>>>,
    /// How each material bumps its surface, alongside its program.
    pub bumps: Vec<Option<Arc<crate::surface_bump::BumpSource>>>,
    /// A Volume Map's density, fitted to each part whose medium has one, by
    /// instance. Empty in nearly every scene.
    pub densities: Vec<Option<Arc<crate::density::DensityField>>>,
    /// Textures the programs sample.
    pub textures: Arc<TextureTable>,
    /// Analytic lights, prepared for selection (`docs/01` §5.3).
    pub lights: Arc<LightSet>,
    pub environment: Arc<Environment>,
    pub background: Background,
    bounds: Aabb,
    /// Whether any material in the scene is less than fully opaque.
    ///
    /// Cached because it decides, per shadow ray, between an any-hit query and
    /// a walk down the ray shading every surface it crosses. The any-hit query
    /// is "the single most frequent query a path tracer makes"
    /// ([`Intersector::occluded`](crate::intersect::Intersector::occluded)),
    /// so a scene with no cutouts — which is nearly all of them — has to keep
    /// taking exactly the path it took before this existed. One bool, tested
    /// once per shadow ray, is what buys that.
    cutouts: bool,
    /// Cutaway volumes (`docs/02` §3). Empty in nearly every scene, and
    /// then every query takes exactly the path it took before cutaways
    /// existed.
    cutters: Arc<Vec<crate::cutaway::Cutter>>,
    /// The glowing triangles, worked out the first time they are asked for.
    emitters: EmitterCache,
    /// Whether any material in the scene refracts.
    ///
    /// Cached for the same reason [`Self::cutouts`] is, and spending the same
    /// one bool. It decides, per *path*, whether the integrator probes for the
    /// object the camera started inside of — a question only a scene with an
    /// interface to get the wrong way round can answer differently. A scene of
    /// painted metal has hundreds of materials and not one of them transmits,
    /// and walking all of them once a path to find that out is the cost this
    /// exists to remove.
    transmissive: bool,
    /// Whether any material disperses light, which decides whether an
    /// `Auto` render is spectral ([`SceneSnapshot::has_dispersion`]).
    dispersive: DispersionCache,
    /// The clear glass among the materials, by material, which shadow rays
    /// pass ([`Pane`]). Empty when there is none, which is what keeps a
    /// scene without glass on the one any-hit query per shadow ray.
    panes: PaneCache,
}

/// [`SceneSnapshot::pane`]'s table, worked out the first time it is asked
/// for, as [`DispersionCache`] is: the materials are public, and a preview
/// changes one in place after the snapshot is made.
#[derive(Debug, Default)]
struct PaneCache(std::sync::OnceLock<Vec<Option<Pane>>>);

impl Clone for PaneCache {
    fn clone(&self) -> Self {
        PaneCache::default()
    }
}

/// [`SceneSnapshot::emitters`], worked out once. A copy starts without it:
/// copying is how a snapshot is changed, and what glows where may be what
/// changed.
#[derive(Debug, Default)]
struct EmitterCache(std::sync::OnceLock<crate::emitters::Emitters>);

impl Clone for EmitterCache {
    fn clone(&self) -> Self {
        EmitterCache::default()
    }
}

/// [`SceneSnapshot::has_dispersion`], worked out the first time it is asked:
/// once per render, rather than once per path walking every material. A copy
/// starts without it, as [`EmitterCache`] does, since materials are what a
/// copy is made to change.
#[derive(Debug, Default)]
struct DispersionCache(std::sync::OnceLock<bool>);

impl Clone for DispersionCache {
    fn clone(&self) -> Self {
        DispersionCache::default()
    }
}

/// Whether a parameter block refracts with an Abbe number: the materials a
/// path meets as the integrator disperses them.
#[inline]
fn disperses(material: &Material) -> bool {
    material.transmission_weight > 0.0 && material.transmission_dispersion_abbe > 0.0
}

/// The same for a graph: its folded constants, or what it computes, or the
/// surface it is mixed with.
fn program_disperses(program: &CompiledGraph) -> bool {
    use tequila_materials::material_type::Param;
    let driven = |wanted: Param| program.driven.iter().any(|(param, _)| *param == wanted);
    let transmits = program.constants.transmission_weight > 0.0 || driven(Param::TransmissionWeight);
    let abbe = program.constants.transmission_dispersion_abbe > 0.0 || driven(Param::Abbe);
    (transmits && abbe) || program.layer.as_ref().is_some_and(|layer| program_disperses(&layer.shader))
}

/// Whether this parameter block describes a surface light can pass through
/// without being deviated.
///
/// The threshold is not zero: a surface at 0.999 opacity is one the walk would
/// have to shade at every crossing to discover it is very nearly solid, and
/// the difference it makes is below what a render shows.
#[inline]
fn is_cut_out(material: &Material) -> bool {
    material.geometry_opacity < 0.999
}

/// Clear glass as a shadow ray meets it ([`SceneSnapshot::pane`]): how much
/// light it lets by on the way straight through.
///
/// KeyShot's renderer, with its caustics off, lets light through glass to
/// what is behind it, and so does this one: a shadow ray stopped by every
/// pane leaves what is under a display cover or inside a bottle lit only by
/// the paths that happen to refract into a light, which on a product is a
/// screen of white speckle on black. Straight on is exact for a pane or a
/// flat slab, whose two refractions cancel, and close for anything else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pane {
    /// What reaches the transmission lobe: its weight, less what metal and
    /// cutout there are.
    weight: f32,
    /// The opacity a cutout has, the rest passing untouched.
    opacity: f32,
    ior: f32,
    coat: f32,
    coat_ior: f32,
    /// The surface's tint at each crossing, the square root of its colour,
    /// as the transmission lobe has it.
    tint: Spectrum,
    /// The volume behind the surface, for Beer-Lambert between the way in
    /// and the way out: its colour, and the distance that colour is reached
    /// in. `None` for glass with no depth, tinted at the surface alone.
    volume: Option<(Spectrum, f32)>,
    /// An infinitely thin shell, which light leaves straight on.
    thin: bool,
    /// Glass and nothing else: all transmission, solid, no coat, film,
    /// fuzz or flakes, and a plain white reflection. What it does with
    /// light is the interface's Fresnel alone, which [`Pane::split`] can
    /// then do without the surface's BSDF.
    plain: bool,
}

impl Pane {
    /// The glass a parameter block is, if it is glass a shadow ray can pass:
    /// mirror-smooth, scattering nothing inside or under its surface.
    /// Frosted glass blurs what is behind it, so light straight through it
    /// is not what reaches the far side, and it stays solid to shadows.
    pub fn of(material: &Material) -> Option<Pane> {
        let index_matched = material.specular_ior <= 1.0 + 1e-3;
        let smooth = index_matched
            || tequila_materials::microfacet::Ggx::from_roughness(
                material.specular_roughness.clamp(0.0, 1.0),
                material.specular_roughness_anisotropy,
            )
            .is_smooth();
        let weight = material.transmission_weight.clamp(0.0, 1.0) * (1.0 - material.base_metalness.clamp(0.0, 1.0));
        if weight <= 1e-4
            || !smooth
            || material.subsurface_weight > 1e-4
            || !material.transmission_scatter.is_black()
        {
            return None;
        }
        let has_depth = !material.geometry_thin_walled && material.transmission_depth > 0.0;
        let c = if has_depth {
            material.transmission_tint
        } else {
            material.transmission_color * material.transmission_tint
        };
        let plain = weight >= 0.999
            && material.geometry_opacity >= 0.999
            && material.coat_weight <= 1e-3
            && material.thin_film_weight <= 1e-3
            && material.fuzz_weight <= 1e-3
            && material.flake_weight <= 1e-3
            && (material.specular_weight - 1.0).abs() <= 1e-3
            && material.specular_color == Spectrum::ONE
            && material.transmission_dispersion_abbe <= 0.0;
        Some(Pane {
            weight,
            opacity: material.geometry_opacity.clamp(0.0, 1.0),
            ior: material.specular_ior.max(1.0),
            coat: material.coat_weight.clamp(0.0, 1.0),
            coat_ior: material.coat_ior.max(1.0),
            tint: Spectrum { r: c.r.max(0.0).sqrt(), g: c.g.max(0.0).sqrt(), b: c.b.max(0.0).sqrt() },
            volume: has_depth.then(|| (material.transmission_color, material.volume_depth())),
            thin: material.geometry_thin_walled,
            plain,
        })
    }

    /// The two ways light leaves plain glass ([`Self::plain`]) met from
    /// `wo`, in the shading frame with the surface facing it: what it
    /// reflects, and the way through with what that carries, as the
    /// transmission lobe would weigh its sample were it always chosen.
    /// `eta` is the index ahead over the index behind. `None` for glass
    /// that is not plain, and where all of the light is reflected.
    pub fn split(&self, wo: Vec3, eta: f32) -> Option<(f32, tequila_materials::bsdf::BsdfSample)> {
        if !self.plain || wo.z <= 0.0 {
            return None;
        }
        let reflected = tequila_materials::fresnel::dielectric(wo.z, eta);
        if reflected >= 1.0 {
            return None;
        }
        let (wi, scale, crossing) = if self.thin {
            (-wo, 1.0, 1.0)
        } else {
            // Snell, about the normal the ray arrives against.
            let sin2_t = (1.0 - wo.z * wo.z).max(0.0) / (eta * eta);
            if sin2_t >= 1.0 {
                return None;
            }
            let cos_t = (1.0 - sin2_t).sqrt();
            (-wo / eta + Vec3::Z * (wo.z / eta - cos_t), 1.0 / (eta * eta), eta)
        };
        let cos = wi.z.abs();
        if cos <= 0.0 {
            return None;
        }
        Some((
            reflected,
            tequila_materials::bsdf::BsdfSample {
                wi,
                f: self.tint * ((1.0 - reflected) * scale / cos),
                pdf: 1.0,
                is_delta: true,
                is_transmission: true,
                eta: crossing,
                flags: tequila_materials::bsdf::BsdfFlags::TRANSMISSION | tequila_materials::bsdf::BsdfFlags::DELTA,
            },
        ))
    }

    /// The same for a graph: from its folded constants, as long as nothing
    /// that decides what passes is computed per point and no other surface
    /// is mixed in.
    fn of_program(program: &CompiledGraph) -> Option<Pane> {
        use tequila_materials::material_type::Param;
        let varies = program.driven.iter().any(|(param, _)| {
            matches!(
                param,
                Param::BaseMetalness
                    | Param::SpecularRoughness
                    | Param::SpecularIor
                    | Param::Anisotropy
                    | Param::TransmissionWeight
                    | Param::TransmissionColor
                    | Param::TransmissionDepth
                    | Param::TransmissionTint
                    | Param::TransmissionScatter
                    | Param::VolumeDensity
                    | Param::SubsurfaceWeight
                    | Param::CoatWeight
                    | Param::CoatIor
                    | Param::Opacity
            )
        });
        if varies || program.layer.is_some() {
            return None;
        }
        Pane::of(&program.constants)
    }

    /// The light through the surface, met at `cos` to its normal: what the
    /// coat and the interface do not reflect, tinted. The same index from
    /// either side, since straight on a ray inside meets the far face at
    /// the angle it met the near one, where a refracted ray would have
    /// turned back to it.
    pub fn crossing(&self, cos: f32) -> Spectrum {
        let cos = cos.abs().clamp(0.0, 1.0);
        let surface = 1.0 - tequila_materials::fresnel::dielectric(cos, self.ior);
        let coat = 1.0 - self.coat * tequila_materials::fresnel::dielectric(cos, self.coat_ior);
        let glass = self.tint * (self.weight * surface * coat);
        Spectrum::splat(1.0 - self.opacity) + glass * self.opacity
    }

    /// What `distance` of the volume behind the surface lets by.
    pub fn inside(&self, distance: f32) -> Spectrum {
        match self.volume {
            Some((colour, depth)) if depth > 0.0 && distance > 0.0 => {
                let exponent = distance / depth;
                let survive = |c: f32| if c.is_finite() { c.clamp(0.0, 1.0).powf(exponent) } else { 0.0 };
                Spectrum { r: survive(colour.r), g: survive(colour.g), b: survive(colour.b) }
            }
            _ => Spectrum::ONE,
        }
    }

    /// Whether the volume behind the surface absorbs.
    pub fn has_volume(&self) -> bool {
        self.volume.is_some()
    }
}

/// The panes by material, or none at all when no material is one.
fn panes(each: impl Iterator<Item = Option<Pane>>) -> Vec<Option<Pane>> {
    let panes: Vec<Option<Pane>> = each.collect();
    if panes.iter().any(Option::is_some) {
        panes
    } else {
        Vec::new()
    }
}

impl SceneSnapshot {
    pub fn new(
        meshes: Vec<Arc<Mesh>>,
        instances: Vec<Instance>,
        materials: Vec<Material>,
        environment: Arc<Environment>,
    ) -> Self {
        let mut bounds = Aabb::EMPTY;
        for inst in &instances {
            if let Some(mesh) = meshes.get(inst.mesh.index()) {
                bounds.extend(&mesh.bounds().transformed(&inst.transform));
            }
        }
        SceneSnapshot {
            meshes,
            instances,
            programs: Vec::new(),
            bumps: Vec::new(),
            densities: Vec::new(),
            textures: Arc::new(TextureTable::new()),
            lights: Arc::new(LightSet::default()),
            environment,
            background: Background::Environment,
            bounds,
            cutouts: materials.iter().any(is_cut_out),
            cutters: Arc::new(Vec::new()),
            emitters: EmitterCache::default(),
            transmissive: materials.iter().any(Material::is_transmissive),
            dispersive: DispersionCache::default(),
            panes: PaneCache::default(),
            materials,
        }
    }

    /// Cut the scene with these volumes.
    pub fn with_cutters(mut self, cutters: Vec<crate::cutaway::Cutter>) -> Self {
        self.cutters = Arc::new(cutters);
        self.emitters = EmitterCache::default();
        self
    }

    /// The glowing triangles, which next-event estimation samples alongside
    /// the lights.
    pub fn emitters(&self) -> &crate::emitters::Emitters {
        self.emitters.0.get_or_init(|| crate::emitters::Emitters::build(self))
    }

    /// The cutaway volumes, usually none.
    #[inline]
    pub fn cutters(&self) -> &[crate::cutaway::Cutter] {
        &self.cutters
    }

    /// Whether any material is less than fully opaque, so cutouts have to be
    /// accounted for.
    #[inline]
    pub fn has_cutouts(&self) -> bool {
        self.cutouts
    }

    /// Whether any material is clear glass a shadow ray can pass ([`Pane`]).
    #[inline]
    pub fn has_panes(&self) -> bool {
        !self.panes().is_empty()
    }

    /// The glass a material is to a shadow ray, if it is clear glass.
    #[inline]
    pub fn pane(&self, id: MaterialId) -> Option<&Pane> {
        self.panes().get(id.index()).and_then(Option::as_ref)
    }

    fn panes(&self) -> &[Option<Pane>] {
        self.panes.0.get_or_init(|| {
            panes(self.materials.iter().enumerate().map(|(i, material)| match self.programs.get(i).and_then(|p| p.as_ref()) {
                Some(program) => Pane::of_program(program),
                None => Pane::of(material),
            }))
        })
    }

    /// Whether any material refracts, so which side of a surface a path is on
    /// is a question with two possible answers.
    #[inline]
    pub fn has_transmissive(&self) -> bool {
        self.transmissive
    }

    /// Whether any material disperses light (glass or a gem with an Abbe
    /// number), so that an `Auto` render traces wavelengths
    /// ([`crate::settings::SpectralMode`]).
    pub fn has_dispersion(&self) -> bool {
        *self.dispersive.0.get_or_init(|| {
            self.materials.iter().enumerate().any(|(i, material)| match self.programs.get(i).and_then(|p| p.as_ref()) {
                Some(program) => program_disperses(program),
                None => disperses(material),
            })
        })
    }

    /// Whether a surface of this instance stops a shadow ray. One with its
    /// shadow switched off — a light's visible stand-in, say — is passed by
    /// every query that asks what is in the way, however it asks.
    #[inline]
    pub fn casts_shadows(&self, instance: InstanceId) -> bool {
        self.instances
            .get(instance.0 as usize)
            .is_none_or(|i| i.casts_shadows)
    }

    pub fn with_lights(mut self, lights: Arc<LightSet>) -> Self {
        self.lights = lights;
        self
    }

    /// Attach compiled graphs and the textures they sample.
    pub fn with_programs(
        mut self,
        programs: Vec<Option<Arc<CompiledGraph>>>,
        textures: Arc<TextureTable>,
    ) -> Self {
        self.emitters = EmitterCache::default();
        self.dispersive = DispersionCache::default();
        // Recomputed rather than or-ed in: a graph can fold opacity to a
        // constant, and the folded value is the one that shades, so the
        // material block alone is not the whole answer once programs exist.
        self.cutouts = self.materials.iter().enumerate().any(|(i, material)| {
            match programs.get(i).and_then(|p| p.as_ref()) {
                Some(program) => {
                    is_cut_out(&program.constants)
                        || program
                            .driven
                            .iter()
                            .any(|(param, _)| *param == tequila_materials::material_type::Param::Opacity)
                }
                None => is_cut_out(material),
            }
        });
        self.transmissive = self.materials.iter().enumerate().any(|(i, material)| {
            match programs.get(i).and_then(|p| p.as_ref()) {
                Some(program) => {
                    program.constants.is_transmissive()
                        || program.driven.iter().any(|(param, _)| {
                            *param == tequila_materials::material_type::Param::TransmissionWeight
                        })
                }
                None => material.is_transmissive(),
            }
        });
        self.panes = PaneCache::default();
        self.bumps = programs
            .iter()
            .map(|program| {
                program
                    .as_ref()
                    .and_then(|p| crate::surface_bump::BumpSource::of(p, &textures))
                    .map(Arc::new)
            })
            .collect();
        self.programs = programs;
        self.textures = textures;
        self
    }

    /// Give parts the density fields their media vary by, by instance.
    pub fn with_densities(mut self, densities: Vec<Option<Arc<crate::density::DensityField>>>) -> Self {
        self.densities = densities;
        self
    }

    /// The density field inside `instance`, if its medium has one.
    #[inline]
    pub fn density(&self, instance: InstanceId) -> Option<&Arc<crate::density::DensityField>> {
        self.densities.get(instance.0 as usize).and_then(|d| d.as_ref())
    }

    /// How a material bumps its surface, if it does.
    #[inline]
    pub fn bump(&self, id: MaterialId) -> Option<&crate::surface_bump::BumpSource> {
        self.bumps.get(id.index()).and_then(|b| b.as_deref())
    }

    /// The interpolated tangent frame at a hit, in world space: the tangent
    /// and the bitangent a normal map's green channel means. `None` for a
    /// mesh without tangents.
    pub fn tangent_frame(&self, hit: &crate::intersect::Hit) -> Option<(Vec3, f32)> {
        let instance = self.instances.get(hit.instance.0 as usize)?;
        let mesh = self.meshes.get(instance.mesh.index())?;
        if mesh.tangents.len() != mesh.positions.len() {
            return None;
        }
        let ([a, b, c], [u, v, w]) = self.barycentrics(hit)?;
        let (t0, t1, t2) = (mesh.tangents[a], mesh.tangents[b], mesh.tangents[c]);
        let blend = |f: fn(tequila_core::math::Vec4) -> f32| u * f(t0) + v * f(t1) + w * f(t2);
        let tangent = Vec3 {
            x: blend(|t| t.x),
            y: blend(|t| t.y),
            z: blend(|t| t.z),
        };
        let sign = if blend(|t| t.w) < 0.0 { -1.0 } else { 1.0 };
        Some((instance.transform.vector(tangent), sign))
    }

    /// The hit triangle's corners and the hit's barycentric weights on them,
    /// in the mesh's own space.
    fn barycentrics(&self, hit: &crate::intersect::Hit) -> Option<([usize; 3], [f32; 3])> {
        let instance = self.instances.get(hit.instance.0 as usize)?;
        let mesh = self.meshes.get(instance.mesh.index())?;
        if hit.primitive as usize >= mesh.triangle_count() {
            return None;
        }
        let [a, b, c] = mesh.triangle(hit.primitive as usize).map(|i| i as usize);
        let (p0, p1, p2) = (mesh.positions[a], mesh.positions[b], mesh.positions[c]);
        let p = instance.transform.inv_point(hit.position);
        let (e1, e2, d) = (p1 - p0, p2 - p0, p - p0);
        let (d11, d12, d22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
        let (d31, d32) = (d.dot(e1), d.dot(e2));
        let det = d11 * d22 - d12 * d12;
        if det.abs() < 1e-30 {
            return None;
        }
        let v = (d22 * d31 - d12 * d32) / det;
        let w = (d11 * d32 - d12 * d31) / det;
        Some(([a, b, c], [1.0 - v - w, v, w]))
    }

    /// The fibre a hit is on, if it is on one, as seen along `direction`:
    /// its frame, and where across it the ray met it.
    ///
    /// Measured from where the hit is against the fibre's centre line at the
    /// same point along it, so it does not depend on which way the curve
    /// primitive happens to number its width.
    pub fn fibre_at(&self, hit: &crate::intersect::Hit, direction: Vec3) -> Option<FibreHit> {
        let instance = self.instances.get(hit.instance.0 as usize)?;
        let fibres = self.meshes.get(instance.mesh.index())?.fibres.as_ref()?;
        let segment = hit.primitive as usize;
        if segment >= fibres.segment_count() {
            return None;
        }
        let first = Fibres::segment_start(segment);
        let (a, b) = (fibres.points[first], fibres.points[first + 1]);
        let along = hit.uv.x.clamp(0.0, 1.0);
        let transform = &instance.transform;
        let tangent = transform.vector(b - a).try_normalize()?;
        let centre = transform.point(a + (b - a) * along);
        let radius = (fibres.radii[first] * (1.0 - along) + fibres.radii[first + 1] * along) / transform.inv_length_scale();
        let to_viewer = -direction;
        let facing = (to_viewer - tangent * tangent.dot(to_viewer))
            .try_normalize()
            .unwrap_or_else(|| tequila_core::math::Onb::from_normal(tangent).t);
        let across = facing.cross(tangent);
        let offset = if radius > 0.0 { ((hit.position - centre).dot(across) / radius).clamp(-1.0, 1.0) } else { 0.0 };
        Some(FibreHit { tangent, facing, offset, random: fibres.random[segment / Fibres::SEGMENTS] })
    }

    /// The mean curvature at a hit, in inverse scene units, interpolated
    /// across the triangle. Zero for a mesh nothing measured.
    pub fn curvature_at(&self, hit: &crate::intersect::Hit) -> f32 {
        let Some(instance) = self.instances.get(hit.instance.0 as usize) else {
            return 0.0;
        };
        let Some(mesh) = self.meshes.get(instance.mesh.index()) else {
            return 0.0;
        };
        if mesh.curvature.len() != mesh.positions.len() {
            return 0.0;
        }
        let Some(([a, b, c], [u, v, w])) = self.barycentrics(hit) else {
            return 0.0;
        };
        let k = u * mesh.curvature[a] + v * mesh.curvature[b] + w * mesh.curvature[c];
        // Measured in the mesh's own units; a part scaled up is flatter by
        // exactly that factor.
        k * instance.transform.inv_length_scale()
    }

    /// The vertex colour at a hit, interpolated across the triangle, in the
    /// rendering space. White for a mesh with none, so a vertex-colour node
    /// multiplied into a colour leaves it alone.
    pub fn vertex_color_at(&self, hit: &crate::intersect::Hit) -> Spectrum {
        let Some(instance) = self.instances.get(hit.instance.0 as usize) else {
            return Spectrum::ONE;
        };
        let Some(mesh) = self.meshes.get(instance.mesh.index()) else {
            return Spectrum::ONE;
        };
        if mesh.colors.len() != mesh.positions.len() {
            return Spectrum::ONE;
        }
        let Some(([a, b, c], [u, v, w])) = self.barycentrics(hit) else {
            return Spectrum::ONE;
        };
        let mixed = mesh.colors[a] * u + mesh.colors[b] * v + mesh.colors[c] * w;
        Spectrum::from_linear_srgb(tequila_core::math::vec3(mixed.x, mixed.y, mixed.z))
    }

    /// The occluded fraction at a hit, interpolated across the triangle.
    /// Zero for a mesh nothing measured.
    pub fn occlusion_at(&self, hit: &crate::intersect::Hit) -> f32 {
        let Some(instance) = self.instances.get(hit.instance.0 as usize) else {
            return 0.0;
        };
        let Some(mesh) = self.meshes.get(instance.mesh.index()) else {
            return 0.0;
        };
        if mesh.occlusion.len() != mesh.positions.len() {
            return 0.0;
        }
        let Some(([a, b, c], [u, v, w])) = self.barycentrics(hit) else {
            return 0.0;
        };
        u * mesh.occlusion[a] + v * mesh.occlusion[b] + w * mesh.occlusion[c]
    }

    /// How the surface moves per unit of u and of v at a hit, in world space:
    /// the frame a normal map is drawn in and a height map is measured in.
    ///
    /// Per triangle, from its corners' positions and UVs. `None` without UVs,
    /// or where a triangle's UVs are degenerate.
    pub fn uv_derivatives(&self, hit: &crate::intersect::Hit) -> Option<(Vec3, Vec3)> {
        let instance = self.instances.get(hit.instance.0 as usize)?;
        let mesh = self.meshes.get(instance.mesh.index())?;
        if mesh.uvs.len() != mesh.positions.len() || hit.primitive as usize >= mesh.triangle_count() {
            return None;
        }
        let [a, b, c] = mesh.triangle(hit.primitive as usize);
        let (p0, p1, p2) = (
            mesh.positions[a as usize],
            mesh.positions[b as usize],
            mesh.positions[c as usize],
        );
        let (t0, t1, t2) = (mesh.uvs[a as usize], mesh.uvs[b as usize], mesh.uvs[c as usize]);
        let (e1, e2) = (p1 - p0, p2 - p0);
        let (d1, d2) = (t1 - t0, t2 - t0);
        let det = d1.x * d2.y - d1.y * d2.x;
        if det.abs() < 1e-20 || !det.is_finite() {
            return None;
        }
        let dpdu = (e1 * d2.y - e2 * d1.y) / det;
        let dpdv = (e2 * d1.x - e1 * d2.x) / det;
        Some((instance.transform.vector(dpdu), instance.transform.vector(dpdv)))
    }

    /// A material's opacity, when it is the same everywhere on the surface.
    ///
    /// `None` means a graph computes it per point, and the caller has to shade
    /// to find out. That is the expensive answer, and it is rare: opacity is a
    /// number on every material in the catalog, and only becomes a map when
    /// someone wires one in the editor. Distinguishing the two is what lets a
    /// shadow ray walk a cutout scene without shading anything.
    #[inline]
    pub fn opacity_of(&self, id: MaterialId) -> Option<f32> {
        match self.program(id) {
            Some(program) => {
                let driven = program
                    .driven
                    .iter()
                    .any(|(param, _)| *param == tequila_materials::material_type::Param::Opacity);
                (!driven).then_some(program.constants.geometry_opacity)
            }
            None => Some(self.material(id).geometry_opacity),
        }
    }

    /// The compiled graph for a material, if it has one worth running.
    #[inline]
    pub fn program(&self, id: MaterialId) -> Option<&Arc<CompiledGraph>> {
        self.programs.get(id.index()).and_then(|p| p.as_ref())
    }

    /// World-space bounds of every instance. Used to frame the camera and to
    /// size the ground plane and environment dome.
    #[inline]
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }

    #[inline]
    pub fn mesh(&self, id: MeshId) -> &Mesh {
        &self.meshes[id.index()]
    }

    #[inline]
    pub fn material(&self, id: MaterialId) -> &Material {
        &self.materials[id.index()]
    }

    #[inline]
    pub fn instance(&self, id: InstanceId) -> &Instance {
        &self.instances[id.index()]
    }

    /// The shading point for a hit, in the part's **own object space**.
    ///
    /// `docs/02-MATERIAL-SYSTEM.md` §6 puts procedurals in object space, and
    /// §5.3 does the same for the projections that CAD geometry depends on,
    /// for one reason: a pattern belongs to the part. Wood grain that is
    /// anchored to the world slides through a bracket as the bracket is
    /// dragged, and a projected label slides off the box it was placed on.
    ///
    /// The intersector reports hits in world space, so the conversion has to
    /// happen somewhere, and it happens here rather than at each call site.
    /// Both the path tracer and the solid preview shade the same surfaces and
    /// have to agree about where a texture sits on them; two copies of this
    /// would eventually disagree, and the symptom — a pattern that jumps when
    /// the viewport switches modes — would look like a renderer bug rather
    /// than a drift between two conversions.
    pub fn shading_point(
        &self,
        hit: &crate::intersect::Hit,
        shading_normal: Vec3,
    ) -> tequila_texture::projection::SurfacePoint {
        self.shading_point_at(hit, shading_normal, crate::settings::DEFAULT_PIXEL_SPREAD)
    }

    /// As [`SceneSnapshot::shading_point_at`], seen along `direction` - the
    /// ray that found it - for the nodes that read the view.
    pub fn shading_point_along(
        &self,
        hit: &crate::intersect::Hit,
        shading_normal: Vec3,
        pixel_spread: f32,
        direction: Vec3,
    ) -> tequila_texture::projection::SurfacePoint {
        let mut point = self.shading_point_at(hit, shading_normal, pixel_spread);
        point.world.incoming = -direction.normalize();
        // Cycles turns both normals to face the ray, as `facing_forward`
        // does; the shading normal arrives here turned already.
        point.world.backfacing = hit.is_backface(direction);
        point.world.true_normal = hit.facing_forward(direction).0;
        point
    }

    /// 0 to 1, the same over a part and different between parts: Cycles'
    /// object random number, `random_id / 0xFFFFFFFF`, for a random id
    /// hashed from the instance's index - the id the Cycles translation
    /// gives each object too, so Object Info's Random agrees.
    pub fn object_random(instance: usize) -> f32 {
        Self::object_random_id(instance) as f32 * (1.0 / u32::MAX as f32)
    }

    /// The random id behind [`SceneSnapshot::object_random`].
    pub fn object_random_id(instance: usize) -> u32 {
        tequila_texture::cycles::hash::hash_uint(instance as u32 + 1)
    }

    /// Where the hit sits in the part's generated texture space: object
    /// space normalized to the mesh's bounding box, 0 to 1. A flat axis
    /// reads 0.5, as Blender's auto texture space has it.
    fn generated_at(&self, hit: &crate::intersect::Hit, object: Vec3) -> Vec3 {
        let bounds = self.mesh(self.instance(hit.instance).mesh).bounds();
        let center = (bounds.min + bounds.max) * 0.5;
        let half = (bounds.max - bounds.min) * 0.5;
        let size = |h: f32| if h == 0.0 { 1.0 } else { h.max(1e-5) };
        let half = tequila_core::math::vec3(size(half.x), size(half.y), size(half.z));
        (object - center) / half * 0.5 + Vec3::splat(0.5)
    }

    /// As [`SceneSnapshot::shading_point`], for a pixel `pixel_spread`
    /// radians wide — see [`crate::settings::IntegratorConfig::pixel_spread`].
    pub fn shading_point_at(
        &self,
        hit: &crate::intersect::Hit,
        shading_normal: Vec3,
        pixel_spread: f32,
    ) -> tequila_texture::projection::SurfacePoint {
        // How wide the ray's footprint is here, in world units.
        let world = hit.t * pixel_spread;
        let transform = &self.instance(hit.instance).transform;
        let object = transform.inv_point(hit.position);
        tequila_texture::projection::SurfacePoint {
            world: tequila_texture::projection::PointWorld {
                position: hit.position,
                normal: shading_normal,
                true_normal: hit.geometric_normal,
                // Facing the viewer until `shading_point_along`, which knows
                // the ray, says otherwise.
                incoming: shading_normal,
                backfacing: false,
                generated: self.generated_at(hit, object),
                object_location: transform.point(Vec3::ZERO),
                object_random: Self::object_random(hit.instance.index()),
                ray: Default::default(),
            },
            position: object,
            // Renormalized because the inverse transpose does not preserve
            // length under scale, and the projections take a unit normal.
            normal: transform.inv_normal(shading_normal).normalize(),
            uv: hit.uv,
            // The footprint the ray covers on this surface, converted into the
            // same space as the position above. Ray differentials land in M3;
            // until then it widens with distance, which is the dominant term
            // and enough to keep a distant texture from aliasing.
            footprint: world * transform.inv_length_scale(),
            // Scene units per unit of `position`: the instance's own
            // scale, so a pattern counts features per scene unit
            // however the part was scaled on import.
            scale: 1.0 / transform.inv_length_scale(),
            // The same footprint in UV: its length over how far the surface
            // moves per unit of u, and of v.
            uv_footprint: self.uv_derivatives(hit).and_then(|(dpdu, dpdv)| {
                let (lu, lv) = (dpdu.length(), dpdv.length());
                (lu > 1e-12 && lv > 1e-12).then(|| [world / lu, world / lv])
            }),
            curvature: self.curvature_at(hit),
            occlusion: self.occlusion_at(hit),
            vertex_color: self.vertex_color_at(hit),
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.instances
            .iter()
            .map(|i| self.meshes[i.mesh.index()].triangle_count())
            .sum()
    }

    /// Number of distinct meshes actually referenced, which is what governs
    /// BLAS build cost.
    pub fn unique_mesh_count(&self) -> usize {
        let mut seen: Vec<MeshId> = self.instances.iter().map(|i| i.mesh).collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec3, Mat4};
    use tequila_core::spectrum::Spectrum;

    fn quad() -> Mesh {
        Mesh::new(
            vec![
                vec3(0.0, 0.0, 0.0),
                vec3(1.0, 0.0, 0.0),
                vec3(1.0, 1.0, 0.0),
                vec3(0.0, 1.0, 0.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    #[test]
    fn mesh_bounds_cover_every_vertex() {
        let m = quad();
        assert_eq!(m.bounds().min, vec3(0.0, 0.0, 0.0));
        assert_eq!(m.bounds().max, vec3(1.0, 1.0, 0.0));
        assert_eq!(m.triangle_count(), 2);
    }

    #[test]
    fn geometric_normal_points_along_the_winding() {
        let m = quad();
        // Counter-clockwise winding in the XY plane faces +Z.
        let n = m.geometric_normal_scaled(0).normalize();
        assert!((n - vec3(0.0, 0.0, 1.0)).length() < 1e-5, "{n:?}");
        // Its length is twice the triangle area.
        assert!((m.geometric_normal_scaled(0).length() - 1.0).abs() < 1e-5);
    }

    /// Smooth normals must be area-weighted: an unweighted average lets a
    /// cluster of small triangles outvote a large one, which is exactly the
    /// configuration a CAD tessellator produces along a fillet.
    #[test]
    fn smooth_normals_are_area_weighted() {
        // A shared vertex between one large +Z triangle and two tiny +X ones.
        let mut m = Mesh::new(
            vec![
                vec3(0.0, 0.0, 0.0), // shared
                vec3(10.0, 0.0, 0.0),
                vec3(0.0, 10.0, 0.0),
                vec3(0.0, 0.0, 0.01),
                vec3(0.0, 0.01, 0.0),
                vec3(0.0, 0.0, 0.02),
            ],
            vec![0, 1, 2, 0, 3, 4, 0, 4, 5],
        );
        m.compute_smooth_normals();
        let n = m.normals[0];
        assert!(n.z > 0.99, "small triangles outvoted the large one: {n:?}");
    }

    #[test]
    fn scene_bounds_include_instance_transforms() {
        let mesh = Arc::new(quad());
        let material = Material::diffuse(Spectrum::splat(0.5));
        let instances = vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(0), MaterialId(0))
                .with_transform(Transform::translation(vec3(5.0, 0.0, 0.0))),
        ];
        let scene = SceneSnapshot::new(
            vec![mesh],
            instances,
            vec![material],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        assert_eq!(scene.bounds().min.x, 0.0);
        assert_eq!(scene.bounds().max.x, 6.0);
        // Instancing survives: two instances, one mesh, two triangles each.
        assert_eq!(scene.triangle_count(), 4);
        assert_eq!(scene.unique_mesh_count(), 1);
    }

    /// A pattern counts its features per scene unit, however the part was
    /// scaled on the way in. An imported model scaled down fifty times — a
    /// Sketchfab file in centimetres under a 1/100 node, say — shaded its
    /// patterns fifty times finer than asked, far below a pixel, so granite
    /// on a headset rendered as flat grey.
    #[test]
    fn a_pattern_keeps_its_scale_on_a_scaled_part() {
        use crate::shade::{shade, Registers};
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
        use tequila_materials::material_type::{MaterialType, Param};
        use tequila_texture::procedural::Procedural;

        let spots = Procedural::catalog()
            .into_iter()
            .find(|p| p.label() == "Spots")
            .expect("Spots is in the catalog");
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let node = graph.add_node(Node::new(NodeKind::Procedural {
            procedural: Box::new(spots),
            seed: 2,
        }));
        let output = graph.output();
        graph.connect(node, output, Socket::Param(Param::SpecularRoughness));
        let program = graph.compile().expect("compiles");

        let scene_with = |transform: Transform| {
            SceneSnapshot::new(
                vec![Arc::new(quad())],
                vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(transform)],
                vec![Material::diffuse(Spectrum::splat(0.5))],
                Arc::new(Environment::uniform(Spectrum::ONE)),
            )
        };
        let plain = scene_with(Transform::IDENTITY);
        let imported = scene_with(Transform::from_matrix(Mat4::scale(Vec3::splat(0.02))).unwrap());

        let roughness = |scene: &SceneSnapshot, world: Vec3| {
            let hit = crate::intersect::Hit {
                t: 1.0,
                position: world,
                geometric_normal: Vec3::Z,
                shading_normal: Vec3::Z,
                uv: tequila_core::math::vec2(0.0, 0.0),
                instance: InstanceId(0),
                material: MaterialId(0),
                primitive: 0,
            };
            let point = scene.shading_point(&hit, Vec3::Z);
            shade(&program, &scene.textures, &point, &mut Registers::new(), &mut Material::default())
                .specular_roughness
        };
        let mut differing = 0;
        for i in 0..200 {
            let world = vec3(i as f32 * 0.0013, i as f32 * 0.0007, 0.0);
            if (roughness(&plain, world) - roughness(&imported, world)).abs() > 1e-4 {
                differing += 1;
            }
        }
        assert_eq!(differing, 0, "{differing} of 200 world points shaded differently on the scaled part");
    }

    /// A texture seen at about a texel per pixel is sampled sharp, however
    /// the part was scaled on import. The footprint was a length in the
    /// mesh's own units handed to the lookup as a width in UV; on a model
    /// scaled down fifty times that is fifty times too wide, the lookup
    /// averaged some fifty texels, and logos, print and fabric vanished from
    /// the render while the Solid view, which picks its level on the GPU,
    /// still showed them.
    #[test]
    fn a_texture_stays_sharp_on_a_scaled_part() {
        use crate::shade::{shade, Registers, TextureTable};
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, TextureId, Value};
        use tequila_materials::material_type::{MaterialType, Param};
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::Mapping;
        use tequila_texture::texture::{MapRole, Texture};

        // A one-texel checker, 1024 across.
        let size = 1024;
        let mut data = Vec::with_capacity(size * size * 3);
        for y in 0..size {
            for x in 0..size {
                let v = ((x + y) % 2) as f32;
                data.extend_from_slice(&[v, v, v]);
            }
        }
        let checker = Texture::from_linear(
            Image { width: size, height: size, channels: 3, data, color_space: ColorSpace::Raw },
            MapRole::Data,
            Mapping::default(),
        );
        let mut table = TextureTable::new();
        table.insert(TextureId(0), Arc::new(checker));
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let image = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(0),
            fallback: Value::Float(0.5),
        }));
        let output = graph.output();
        graph.connect(image, output, Socket::Param(Param::BaseColor));
        let program = graph.compile().expect("compiles");

        // Fifty units across in its own coordinates, scaled by 0.02 into a
        // one-metre quad — a centimetre model under a 1/100 node, roughly.
        let quad = Mesh::new(
            vec![vec3(0.0, 0.0, 0.0), vec3(50.0, 0.0, 0.0), vec3(50.0, 50.0, 0.0), vec3(0.0, 50.0, 0.0)],
            vec![0, 1, 2, 0, 2, 3],
        )
        .with_uvs(vec![
            tequila_core::math::vec2(0.0, 0.0),
            tequila_core::math::vec2(1.0, 0.0),
            tequila_core::math::vec2(1.0, 1.0),
            tequila_core::math::vec2(0.0, 1.0),
        ]);
        let scene = SceneSnapshot::new(
            vec![Arc::new(quad)],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
                .with_transform(Transform::from_matrix(Mat4::scale(Vec3::splat(0.02))).unwrap())],
            vec![Material::diffuse(Spectrum::splat(0.5))],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        )
        .with_programs(vec![Some(Arc::new(program))], Arc::new(table));

        // Seen from a metre away, where a pixel is about a millimetre: about
        // one texel of a 1024 texture across a metre.
        for i in [100, 301, 512, 777] {
            let (u, v) = ((i as f32 + 0.5) / size as f32, (i as f32 + 0.5) / size as f32);
            let hit = crate::intersect::Hit {
                t: 1.0,
                position: vec3(u, v, 0.0),
                geometric_normal: Vec3::Z,
                shading_normal: Vec3::Z,
                uv: tequila_core::math::vec2(u, v),
                instance: InstanceId(0),
                material: MaterialId(0),
                primitive: 0,
            };
            let point = scene.shading_point(&hit, Vec3::Z);
            let base = shade(&program_of(&scene), &scene.textures, &point, &mut Registers::new(), &mut Material::default())
                .base_color
                .r;
            // A texel at (i, i) is dark; averaged, it would be grey.
            assert!(base < 0.3, "texel {i} shaded {base}: the lookup averaged the checker away");
        }

        fn program_of(scene: &SceneSnapshot) -> CompiledGraph {
            (**scene.program(MaterialId(0)).unwrap()).clone()
        }
    }

    /// The Curvature node reads the surface's curvature through the whole
    /// path, in scene units: a part scaled up twice curves half as much, and
    /// the masks come out as the radius says.
    #[test]
    fn curvature_reaches_the_shader_in_scene_units() {
        use crate::shade::{shade, Registers, TextureTable};
        use tequila_materials::graph::{CurvatureMode, MaterialGraph, Node, NodeKind, Socket};
        use tequila_materials::material_type::{MaterialType, Param};

        let mask = |mode: CurvatureMode, scale: f32, k: f32| {
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            let node = graph.add_node(Node::new(NodeKind::Curvature { mode, radius: 0.5, contrast: 1.0 }));
            let output = graph.output();
            graph.connect(node, output, Socket::Param(Param::SpecularRoughness));
            let program = Arc::new(graph.compile().expect("compiles"));
            let mesh = quad().with_curvature(vec![k; 4]);
            let scene = SceneSnapshot::new(
                vec![Arc::new(mesh)],
                vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
                    .with_transform(Transform::from_matrix(Mat4::scale(Vec3::splat(scale))).unwrap())],
                vec![Material::diffuse(Spectrum::splat(0.5))],
                Arc::new(Environment::uniform(Spectrum::ONE)),
            )
            .with_programs(vec![Some(program.clone())], Arc::new(TextureTable::new()));
            let hit = crate::intersect::Hit {
                t: 1.0,
                position: vec3(0.3, 0.2, 0.0) * scale,
                geometric_normal: Vec3::Z,
                shading_normal: Vec3::Z,
                uv: tequila_core::math::vec2(0.0, 0.0),
                instance: InstanceId(0),
                material: MaterialId(0),
                primitive: 0,
            };
            let point = scene.shading_point(&hit, Vec3::Z);
            shade(&program, &scene.textures, &point, &mut Registers::new(), &mut Material::default()).specular_roughness
        };
        // Curvature 1 per unit at radius 0.5: half in the convex mask.
        assert!((mask(CurvatureMode::Convex, 1.0, 1.0) - 0.5).abs() < 1e-4);
        // Scaled up twice: half the curvature, half the mask.
        assert!((mask(CurvatureMode::Convex, 2.0, 1.0) - 0.25).abs() < 1e-4);
        // Concave reads negative curvature, and ignores positive.
        assert!((mask(CurvatureMode::Concave, 1.0, -1.0) - 0.5).abs() < 1e-4);
        assert!(mask(CurvatureMode::Concave, 1.0, 1.0).abs() < 1e-6);
        // Both: flat is mid-grey.
        assert!((mask(CurvatureMode::Both, 1.0, 0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn rotated_instance_bounds_are_conservative() {
        let mesh = Arc::new(quad());
        let t =
            Transform::from_matrix(Mat4::rotation(Vec3::Z, std::f32::consts::FRAC_PI_4)).unwrap();
        let scene = SceneSnapshot::new(
            vec![mesh],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(t)],
            vec![Material::diffuse(Spectrum::splat(0.5))],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        // A unit square rotated 45 degrees spans sqrt(2) in Y.
        assert!((scene.bounds().max.y - std::f32::consts::SQRT_2).abs() < 1e-4);
    }

    /// A hit on a coloured mesh reads the colour blended across the triangle;
    /// a mesh without colours reads white.
    #[test]
    fn a_hit_reads_the_vertex_colour() {
        use tequila_core::math::{vec3, vec4};
        let mut mesh = Mesh::new(vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
        mesh.colors = vec![vec4(1.0, 0.0, 0.0, 1.0), vec4(0.0, 1.0, 0.0, 1.0), vec4(0.0, 0.0, 1.0, 1.0)];
        let plain = Mesh::new(vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
        let scene = SceneSnapshot::new(
            vec![Arc::new(mesh), Arc::new(plain)],
            vec![
                Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
                Instance::new(InstanceId(1), MeshId(1), MaterialId(0)),
            ],
            vec![Material::diffuse(Spectrum::splat(0.5))],
            Arc::new(Environment::uniform(Spectrum::ONE)),
        );
        let hit = |instance: u32, at: Vec3| crate::intersect::Hit {
            t: 1.0,
            position: at,
            geometric_normal: Vec3::Z,
            shading_normal: Vec3::Z,
            uv: tequila_core::math::vec2(0.0, 0.0),
            instance: InstanceId(instance),
            material: MaterialId(0),
            primitive: 0,
        };
        // In the rendering primaries, as every colour the shader sees is.
        let expect = |r: f32, g: f32, b: f32| Spectrum::from_linear_srgb(vec3(r, g, b));
        let close = |a: Spectrum, b: Spectrum| (a.r - b.r).abs() < 1e-3 && (a.g - b.g).abs() < 1e-3 && (a.b - b.b).abs() < 1e-3;
        let corner = scene.shading_point(&hit(0, Vec3::ZERO), Vec3::Z).vertex_color;
        assert!(close(corner, expect(1.0, 0.0, 0.0)), "{corner:?}");
        let middle = scene.shading_point(&hit(0, vec3(0.5, 0.0, 0.0)), Vec3::Z).vertex_color;
        assert!(close(middle, expect(0.5, 0.5, 0.0)), "not blended: {middle:?}");
        let white = scene.shading_point(&hit(1, vec3(0.2, 0.2, 0.0)), Vec3::Z).vertex_color;
        assert_eq!(white, Spectrum::ONE);
    }
}
