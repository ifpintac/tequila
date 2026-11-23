//! The scene's analytic lamps on the card (`docs/07` R3): each lamp as a
//! record `lights.wgsl` reads, and the way the processor chooses one for a
//! light sample - its alias table, or its light hierarchy - as
//! `LightSet` built it, uploaded unchanged (`lamps.wgsl`).
//!
//! Whatever a lamp's sampling works out from its own parameters alone - an
//! area lamp's normal (an f64 cross product), its area, its spherical
//! rectangle's frame, a grid's tangent and normalization, a spot's cosines,
//! a sun's disc - is worked out here, by the processor's own functions, so
//! the card starts from the processor's numbers. What it works out from
//! where the shading point is, the card works out, as the processor does.
//!
//! What of the lamps the card renders otherwise, [`lamp_limits`] says.

use bytemuck::{Pod, Zeroable};
use tequila_core::math::Vec3;
use tequila_core::spectrum::Spectrum;
use tequila_render_core::light::{Light, LightKind, LightLinks, LightSet, SelectionStrategy};
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::settings::{IntegratorConfig, ShadowGlass};
use wgpu::util::DeviceExt;

/// `lights.wgsl`'s `GpuLight`, member for member.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuLight {
    /// The lamp's place; `[3]` a point lamp's radius.
    pub position: [f32; 4],
    /// An area lamp's half extents; `[3]` each one's squared length, at
    /// least 1e-12, as `AreaLight::face_at` divides by it.
    pub half_u: [f32; 4],
    pub half_v: [f32; 4],
    /// An area lamp's normal (`AreaLight::normal`), a spot's axis, or the
    /// way a sun's light travels; `[3]` an area lamp's area.
    pub normal: [f32; 4],
    /// Radiance (nits), intensity (cd) or illuminance (lux); `[3]` a sun's
    /// `1 / (pi sin² half_angle)`.
    pub radiance: [f32; 4],
    /// The spherical rectangle's frame (`SphericalRectangle::build`): its
    /// x and y with the edge lengths in `[3]`, its z, and the corner.
    pub rect_x: [f32; 4],
    pub rect_y: [f32; 4],
    pub rect_z: [f32; 4],
    pub rect_origin: [f32; 4],
    pub kind: u32,
    pub flags: u32,
    pub tan_half: f32,
    pub spread_norm: f32,
    pub falloff: f32,
    pub fade: f32,
    /// A spot's `cos(outer)`, a sun's `cos(half_angle)`.
    pub cos_a: f32,
    /// A spot's `cos(inner)`, a sun's `sin(half_angle)`.
    pub cos_b: f32,
    /// A spot's falloff exponent, a sun's disc's density.
    pub exponent: f32,
    pub probability: f32,
    pub alias_p: f32,
    pub alias_alt: u32,
    /// The hierarchy's leaf that holds it.
    pub leaf: u32,
    /// Where its lists start in the links buffer: the instances it lights
    /// (its light links, `link_count` of them), then the instances that cast
    /// its shadow or do not (its shadow links, `shadow_link_count`).
    pub links: u32,
    pub link_count: u32,
    pub shadow_link_count: u32,
}

/// `lights.wgsl`'s `GpuLightNode`: a node as `LightSet::hierarchy` shows it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuLightNode {
    /// The bounds' middle; `[3]` half their diagonal.
    pub centre: [f32; 4],
    /// The emission axis; `[3]` the power.
    pub axis: [f32; 4],
    pub cos_o: f32,
    pub sin_o: f32,
    pub cos_e: f32,
    pub distant: u32,
    /// The children, [`LEAF`] for a leaf.
    pub left: u32,
    pub right: u32,
    /// The node above, [`LEAF`] at the root.
    pub parent: u32,
    /// A leaf's light.
    pub light: u32,
}

/// [`GpuLightNode::left`] of a leaf, and [`GpuLightNode::parent`] of the root.
pub const LEAF: u32 = u32::MAX;

pub const KIND_AREA: u32 = 0;
pub const KIND_POINT: u32 = 1;
pub const KIND_SPOT: u32 = 2;
pub const KIND_IES: u32 = 3;
pub const KIND_DISTANT: u32 = 4;

pub const FLAG_DISC: u32 = 1;
pub const FLAG_TWO_SIDED: u32 = 2;
pub const FLAG_ENABLED: u32 = 4;
pub const FLAG_CAMERA: u32 = 8;
pub const FLAG_DIFFUSE: u32 = 16;
pub const FLAG_SPECULAR: u32 = 32;
pub const FLAG_SHADOW: u32 = 64;
pub const FLAG_CAN_BE_MET: u32 = 128;
pub const FLAG_DELTA: u32 = 256;
pub const FLAG_EVEN: u32 = 512;
pub const FLAG_SPREAD_FULL: u32 = 1024;
pub const FLAG_RECTANGLE: u32 = 2048;
pub const FLAG_LINKS_ONLY: u32 = 4096;
pub const FLAG_LINKS_ALL_BUT: u32 = 8192;
/// Its shadow links name the only instances that cast its shadow
/// (`LightLinks::Only`), or the ones that do not (`LightLinks::AllBut`).
pub const FLAG_SHADOW_LINKS_ONLY: u32 = 16384;
pub const FLAG_SHADOW_LINKS_ALL_BUT: u32 = 32768;
/// An IES lamp, whose profile follows its link lists in the links buffer
/// (`lamps.wgsl`'s `ies_candela`).
pub const FLAG_IES: u32 = 65536;

fn v4(v: Vec3, w: f32) -> [f32; 4] {
    [v.x, v.y, v.z, w]
}

fn s4(s: Spectrum, w: f32) -> [f32; 4] {
    [s.r, s.g, s.b, w]
}

/// A lamp's record, but for where it sits in the selection and its links.
pub fn light_record(light: &Light) -> GpuLight {
    let mut flags = 0;
    for (on, flag) in [
        (light.enabled, FLAG_ENABLED),
        (light.visibility.camera, FLAG_CAMERA),
        (light.visibility.diffuse, FLAG_DIFFUSE),
        (light.visibility.specular, FLAG_SPECULAR),
        (light.visibility.shadow, FLAG_SHADOW),
        (light.can_be_met(), FLAG_CAN_BE_MET),
        (light.is_delta(), FLAG_DELTA),
    ] {
        if on {
            flags |= flag;
        }
    }
    let mut r = GpuLight { leaf: LEAF, ..GpuLight::default() };
    match &light.kind {
        LightKind::Area(area) => {
            r.kind = KIND_AREA;
            if area.disc {
                flags |= FLAG_DISC;
            }
            if area.two_sided {
                flags |= FLAG_TWO_SIDED;
            }
            if area.is_even() {
                flags |= FLAG_EVEN;
            }
            r.position = v4(area.position, 0.0);
            r.half_u = v4(area.half_u, area.half_u.length_squared().max(1e-12));
            r.half_v = v4(area.half_v, area.half_v.length_squared().max(1e-12));
            r.normal = v4(area.normal(), area.area());
            r.radiance = s4(area.radiance, 0.0);
            r.falloff = area.falloff;
            r.fade = area.fade;
            // AreaLight::spread_attenuation's constants.
            if area.spread >= std::f32::consts::PI - 1e-4 {
                flags |= FLAG_SPREAD_FULL;
            } else {
                let half = 0.5 * area.spread.max(1e-3);
                r.tan_half = half.tan();
                r.spread_norm = if half > 0.05 { 1.0 / (r.tan_half - half) } else { 3.0 / (half * half * half) };
            }
            // SphericalRectangle::build's, up to where the point comes in.
            let ex = area.half_u * 2.0;
            let ey = area.half_v * 2.0;
            let origin = area.position - area.half_u - area.half_v;
            let (exl, eyl) = (ex.length(), ey.length());
            if !(exl < 1e-9 || eyl < 1e-9) {
                flags |= FLAG_RECTANGLE;
                let x = ex / exl;
                let y = ey / eyl;
                r.rect_x = v4(x, exl);
                r.rect_y = v4(y, eyl);
                r.rect_z = v4(x.cross(y), 0.0);
                r.rect_origin = v4(origin, 0.0);
            }
            // Light::irradiance_bound's from the lamp alone: the reach of
            // its sphere, and its brightest radiance's luminance.
            let (u, v) = (area.half_u.length(), area.half_v.length());
            r.cos_a = if area.disc { u.max(v) } else { (u * u + v * v).sqrt() };
            r.cos_b = area.radiance.luminance().max(0.0) * area.spread_attenuation(1.0) * face_peak(area);
        }
        LightKind::Point(point) => {
            r.kind = KIND_POINT;
            r.position = v4(point.position, point.radius);
            r.radiance = s4(point.intensity, 0.0);
        }
        LightKind::Spot(spot) => {
            r.kind = KIND_SPOT;
            r.position = v4(spot.position, 0.0);
            r.normal = v4(spot.direction, 0.0);
            r.radiance = s4(spot.intensity, 0.0);
            r.cos_a = spot.outer_angle.cos();
            r.cos_b = spot.inner_angle.cos();
            r.exponent = spot.falloff;
        }
        LightKind::Ies(ies) => {
            // The luminaire's frame (IesLight::candela): its right, its up
            // and its aim; the profile is the links buffer's.
            r.kind = KIND_IES;
            flags |= FLAG_IES;
            r.position = v4(ies.position, 0.0);
            r.normal = v4(ies.direction, 0.0);
            r.half_u = v4(ies.right, 0.0);
            r.half_v = v4(ies.direction.cross(ies.right), 0.0);
            r.radiance = s4(ies.intensity, 0.0);
        }
        LightKind::Distant(sun) => {
            r.kind = KIND_DISTANT;
            r.normal = v4(sun.direction, 0.0);
            let (cos_max, sin_max) = (sun.half_angle.cos(), sun.half_angle.sin());
            r.cos_a = cos_max;
            r.cos_b = sin_max;
            r.radiance = s4(sun.irradiance, 1.0 / (std::f32::consts::PI * sin_max * sin_max));
            r.exponent = 1.0 / (std::f32::consts::TAU * (1.0 - cos_max));
        }
    }
    r.flags = flags;
    r
}

/// `AreaLight::face_peak`: the brightest its face can be, one over the
/// average its falloff and fade are divided by.
fn face_peak(area: &tequila_render_core::light::AreaLight) -> f32 {
    if area.is_even() {
        return 1.0;
    }
    let falloff = area.falloff.clamp(0.0, 1.0);
    let fade = area.fade.clamp(-1.0, 1.0).abs();
    let edge_mean = if area.disc { 1.0 - 0.5 * falloff } else { (1.0 - falloff / 3.0).powi(2) };
    1.0 / (edge_mean * (1.0 - 0.5 * fade)).max(1e-3)
}

/// The lamps of a scene as the card reads them, and the selection
/// structure over them.
#[derive(Clone, Debug, Default)]
pub struct LightRecords {
    pub lights: Vec<GpuLight>,
    pub nodes: Vec<GpuLightNode>,
    /// Every lamp's linked instances, end to end, and an IES lamp's profile
    /// after them: the counts of its vertical and horizontal angles, the
    /// span its horizontal angles fold about, the angles and the candela
    /// values (horizontal-major), as f32 bits.
    pub links: Vec<u32>,
    /// Whether [`Self::nodes`] is what chooses (`SelectionStrategy::Hierarchy`).
    pub hierarchy: bool,
}

impl LightRecords {
    pub fn of(scene: &SceneSnapshot) -> LightRecords {
        LightRecords::of_set(&scene.lights)
    }

    pub fn of_set(set: &LightSet) -> LightRecords {
        let alias = set.alias_table();
        let probabilities = set.probabilities();
        let leaves = set.leaves();
        let mut links = Vec::new();
        let lights = set
            .lights()
            .iter()
            .enumerate()
            .map(|(i, light)| {
                let mut r = light_record(light);
                r.probability = probabilities[i];
                (r.alias_p, r.alias_alt) = alias[i];
                r.leaf = leaves.get(i).copied().unwrap_or(LEAF);
                // Its light links, then its shadow links, end to end.
                let start = links.len() as u32;
                let lists = |links: &LightLinks, only: u32, all_but: u32| match links {
                    LightLinks::All => None,
                    LightLinks::Only(these) => Some((only, these.clone())),
                    LightLinks::AllBut(these) => Some((all_but, these.clone())),
                };
                if let Some((flag, list)) = lists(&light.links, FLAG_LINKS_ONLY, FLAG_LINKS_ALL_BUT) {
                    r.flags |= flag;
                    r.links = start;
                    r.link_count = list.len() as u32;
                    links.extend_from_slice(&list);
                }
                if let Some((flag, list)) = lists(&light.shadow_links, FLAG_SHADOW_LINKS_ONLY, FLAG_SHADOW_LINKS_ALL_BUT) {
                    r.flags |= flag;
                    r.links = start;
                    r.shadow_link_count = list.len() as u32;
                    links.extend_from_slice(&list);
                }
                if let LightKind::Ies(ies) = &light.kind {
                    r.links = start;
                    let (vertical, horizontal, candela) = ies.profile.table();
                    let span = horizontal.last().copied().unwrap_or(0.0).max(1e-6);
                    links.extend_from_slice(&[vertical.len() as u32, horizontal.len() as u32, span.to_bits()]);
                    links.extend(vertical.iter().chain(horizontal).chain(candela).map(|v| v.to_bits()));
                }
                r
            })
            .collect();
        let order = set.order();
        let nodes: Vec<GpuLightNode> = set
            .hierarchy()
            .map(|node| {
                let (left, right) = node.children.unwrap_or((LEAF, LEAF));
                GpuLightNode {
                    centre: v4(node.centre, node.half),
                    axis: v4(node.axis, node.power),
                    cos_o: node.cos_o,
                    sin_o: node.sin_o,
                    cos_e: node.cos_e,
                    distant: node.distant as u32,
                    left,
                    right,
                    parent: node.parent,
                    light: if node.children.is_none() { order.get(node.first as usize).copied().unwrap_or(LEAF) } else { LEAF },
                }
            })
            .collect();
        let hierarchy = set.strategy() == SelectionStrategy::Hierarchy && !nodes.is_empty();
        LightRecords { lights, nodes, links, hierarchy }
    }
}

/// The lamps uploaded: `lamps.wgsl`'s bindings 8, 9 and 10.
pub struct GpuLights {
    pub lights: wgpu::Buffer,
    pub nodes: wgpu::Buffer,
    pub links: wgpu::Buffer,
    pub count: u32,
    pub hierarchy: bool,
}

impl GpuLights {
    pub fn upload(device: &wgpu::Device, records: &LightRecords) -> GpuLights {
        // A buffer may not be empty.
        let lights = if records.lights.is_empty() { vec![GpuLight::default()] } else { records.lights.clone() };
        let nodes = if records.nodes.is_empty() { vec![GpuLightNode::default()] } else { records.nodes.clone() };
        let links = if records.links.is_empty() { vec![0u32] } else { records.links.clone() };
        let init = |label: &str, contents: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE })
        };
        GpuLights {
            lights: init("lights", bytemuck::cast_slice(&lights)),
            nodes: init("light nodes", bytemuck::cast_slice(&nodes)),
            links: init("light links", bytemuck::cast_slice(&links)),
            count: records.lights.len() as u32,
            hierarchy: records.hierarchy,
        }
    }
}

/// `FrameParams::lamp_flags`: a bounce off a sharp enough reflection has a
/// density to see by (integrator.rs's `seen`: `meets_glass || meets_lamps`).
pub const LAMPS_SEEN: u32 = 1;
/// There is a lamp a seeing bounce meets (`meets_lamps`).
pub const LAMPS_MET: u32 = 2;
/// A lamp shows in mirrors and the camera's view through glass, rather
/// than by its stand-in (`lamps_in_mirrors`).
pub const LAMPS_IN_MIRRORS: u32 = 4;
/// One of the lamps a bounce meets lights nothing diffusely
/// (`lamps_for_reflections`).
pub const LAMPS_FOR_REFLECTIONS: u32 = 8;
/// Shadow rays pass all clear glass and there is some (`meets_glass`).
pub const LAMPS_MEETS_GLASS: u32 = 16;
/// Some light passes a part its shadow rays pass, which a path stopping at
/// the part meets there (`passes_parts`: `lamps_past`, `light_past`).
pub const LAMPS_PASSES_PARTS: u32 = 32;
/// Some part casts no shadow (`SceneSnapshot::has_shadowless_parts`): a
/// floor's nearness is measured past it (`nearness_at`).
pub const LAMPS_SHADOWLESS: u32 = 64;
/// The lights are not sampled directly (`IntegratorConfig::direct_lighting`
/// off): no light sample anywhere, and every bounce that meets a light takes
/// it whole.
pub const LAMPS_UNSAMPLED: u32 = 128;
/// The sky's light sample sees past the shadow-catching floors the bounces
/// pass, and a bounce past one keeps the sky's weight
/// (`integrator::sky_past_floors`).
pub const LAMPS_SKY_PAST_FLOORS: u32 = 256;

/// What [`lamp_flags`] and [`lamp_limits`] ask of a scene, read when it is
/// uploaded, so either can be said once a configuration is known.
#[derive(Clone, Copy, Debug, Default)]
pub struct LampScene {
    panes: bool,
    /// A lamp a ray can meet that shows in reflections, one of those that
    /// lights nothing diffusely, and one a ray can meet that shows in
    /// reflections or to the camera.
    met_specular: bool,
    met_specular_only: bool,
    met_specular_or_camera: bool,
    shadowless_parts: bool,
    met_past_parts: bool,
    /// The sky is infinitely far away (no dome of a size).
    infinite_dome: bool,
}

impl LampScene {
    pub fn of(scene: &SceneSnapshot) -> LampScene {
        let lights = scene.lights.lights();
        let met = |l: &&Light| l.can_be_met();
        LampScene {
            panes: scene.has_panes(),
            met_specular: lights.iter().filter(met).any(|l| l.visibility.specular),
            met_specular_only: lights.iter().filter(met).any(|l| l.visibility.specular && !l.visibility.diffuse),
            met_specular_or_camera: lights.iter().filter(met).any(|l| l.visibility.specular || l.visibility.camera),
            shadowless_parts: scene.has_shadowless_parts(),
            met_past_parts: lights.iter().filter(met).any(|l| !l.visibility.shadow || !l.shadow_links.is_all()),
            infinite_dome: scene.environment.dome().is_infinite(),
        }
    }

    /// The integrator's flags about the lamps (ic:917-932) under `config`.
    pub fn flags(&self, config: &IntegratorConfig) -> u32 {
        let meets_glass = config.shadow_glass() == ShadowGlass::Clear && self.panes;
        let meets_lamps = config.direct_lighting && config.shadows && self.met_specular;
        let for_reflections = meets_lamps && self.met_specular_only;
        let in_mirrors = config.direct_lighting && self.met_specular_or_camera;
        let passes_parts = config.shadows && (self.shadowless_parts || self.met_past_parts);
        let sky_past_floors = tequila_render_core::integrator::sky_past_floors_of(config, passes_parts, self.panes, self.infinite_dome);
        let mut flags = 0;
        for (on, flag) in [
            (meets_glass || meets_lamps, LAMPS_SEEN),
            (meets_lamps, LAMPS_MET),
            (in_mirrors, LAMPS_IN_MIRRORS),
            (for_reflections, LAMPS_FOR_REFLECTIONS),
            (meets_glass, LAMPS_MEETS_GLASS),
            (passes_parts, LAMPS_PASSES_PARTS),
            (self.shadowless_parts, LAMPS_SHADOWLESS),
            (!config.direct_lighting, LAMPS_UNSAMPLED),
            (sky_past_floors, LAMPS_SKY_PAST_FLOORS),
        ] {
            if on {
                flags |= flag;
            }
        }
        flags
    }

    /// [`lamp_limits`] of the scene this was read from: nothing now - IES
    /// lamps, shadow links, the light past a part its shadow rays pass and
    /// direct lighting off are the processor's on the card too. Kept for the
    /// renderer and the command line, which say each of the `*_limits`.
    pub fn limits(&self, _config: &IntegratorConfig) -> Vec<&'static str> {
        Vec::new()
    }
}

/// The integrator's flags about the lamps (ic:917-932) under `config`.
pub fn lamp_flags(scene: &SceneSnapshot, config: &IntegratorConfig) -> u32 {
    LampScene::of(scene).flags(config)
}

/// What of the lamps and the light past parts in `scene` the card renders
/// otherwise than the processor does under `config`, one sentence each;
/// empty where they are the processor's. The renderer says each at its
/// first pass, with [`crate::transport::floor_limits`].
pub fn lamp_limits(scene: &SceneSnapshot, config: &IntegratorConfig) -> Vec<&'static str> {
    LampScene::of(scene).limits(config)
}
