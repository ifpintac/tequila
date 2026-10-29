//! A render-core scene, rebuilt as a Cycles scene.
//!
//! What maps directly: triangle meshes, instances and their visibility, the
//! OpenPBR parameter block (onto Cycles' Principled BSDF, which is modelled on
//! OpenPBR closely enough that most parameters are a rename), image textures,
//! normal maps, the environment and the camera. What does not is reported in
//! [`Translation::warnings`] rather than dropped silently, so a render that
//! differs from Tequila's own says why.
//!
//! Two conventions change on the way:
//!
//! - **Up.** Tequila is Y-up; Cycles maps environments Z-up. Everything is
//!   turned by [`Y_UP_TO_Z_UP`], the same rotation Blender's glTF importer
//!   applies, so a model and its lighting keep their relationship.
//! - **Colour.** Tequila's rendering space is ACEScg; Cycles here works in
//!   linear Rec.709. Texture texels are already linear Rec.709 (Tequila
//!   converts them as it samples), so they pass straight through; environment
//!   pictures (held in ACEScg) and material constants are converted.
//!   [`to_rendering`] turns the result back.

use std::collections::HashMap;

use tequila_core::math::{vec3, Mat4, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{CompiledGraph, Instruction, NodeKind, Register, TextureId, Value};
use tequila_materials::material_type::Param;
use tequila_materials::nodes::{CyclesSetting, PointInput, ShaderNode};
use tequila_materials::openpbr::Material;
use tequila_render_core::camera::{Aperture, Camera, Projection};
use tequila_render_core::environment::Background;
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::surface_bump::BumpSource;
use tequila_texture::projection::{Mapping, Projection as TextureProjection};

use crate::{ffi, Node, Session, Shader};

/// Tequila world (Y up) to Cycles world (Z up): x stays, y becomes z, and z
/// becomes -y. Row major.
pub const Y_UP_TO_Z_UP: Mat4 = Mat4 {
    m: [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ],
};

/// What a render needs beyond the scene.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub camera: Camera,
    pub width: usize,
    pub height: usize,
    /// Leave the background out of the image, as alpha.
    pub transparent: bool,
    /// The lighting's Ground illumination: the floor lights the product as a
    /// real surface would. Off, as Tequila's renderer treats it, the floor
    /// is there for the camera alone, to show shadows on; reflections,
    /// refraction and bounced light pass through it to the environment.
    pub ground_illumination: bool,
}

/// How hard Cycles works on an image.
#[derive(Clone, Copy, Debug)]
pub struct Quality {
    pub adaptive: bool,
    pub denoise: bool,
    /// Stop after this long, whatever the samples.
    pub seconds: Option<f64>,
    /// The lighting's bounce limits: any bounce, and through glass, which
    /// Tequila counts apart (`IntegratorConfig::max_bounces`,
    /// `max_transmission_bounces`).
    pub bounces: u32,
    pub transmission_bounces: u32,
    /// Tequila's firefly rejection, which is Cycles' clamp on indirect
    /// light at its default of 10; off, no clamp.
    pub reject_fireflies: bool,
    /// A clamp asked for, on direct and indirect light alike, as Blender's
    /// Clamping panel sets them.
    pub clamp: Option<f32>,
    /// Whether the denoiser cleans its guides first: Cycles' "accurate"
    /// prefilter, or none.
    pub prefilter: bool,
    /// The lighting's Light Paths beyond the two limits above
    /// (`IntegratorConfig::cycles`).
    pub transport: tequila_render_core::settings::CyclesTransport,
    /// A clamp on direct light of its own: with it, [`clamp`](Self::clamp)
    /// is the indirect one alone.
    pub clamp_direct: Option<f32>,
    /// The render's pixel filter; `None` is Cycles' Film default.
    pub pixel_filter: Option<tequila_render_core::settings::PixelFilter>,
    /// The render's adaptive-sampling threshold; `None` is Cycles' own.
    pub adaptive_threshold: Option<f32>,
}

impl Quality {
    /// For a render with `config`'s lighting, with Cycles' own clamping and
    /// prefilter: what a render got before these were the settings'.
    pub fn with_lighting(adaptive: bool, denoise: bool, seconds: Option<f64>, config: &tequila_render_core::settings::IntegratorConfig) -> Quality {
        Quality {
            adaptive,
            denoise,
            seconds,
            bounces: config.max_bounces,
            transmission_bounces: config.max_transmission_bounces,
            reject_fireflies: true,
            clamp: None,
            prefilter: true,
            transport: config.cycles,
            clamp_direct: None,
            pixel_filter: None,
            adaptive_threshold: None,
        }
    }

    /// With the render settings' direct clamp, pixel filter and adaptive
    /// threshold, each `None` leaving Cycles as it was.
    pub fn with_render_controls(
        self,
        clamp_direct: Option<f32>,
        pixel_filter: Option<tequila_render_core::settings::PixelFilter>,
        adaptive_threshold: Option<f32>,
    ) -> Quality {
        Quality { clamp_direct, pixel_filter, adaptive_threshold, ..self }
    }

    /// With the render settings' firefly rejection, clamp and prefilter,
    /// which Cycles used to be handed none of.
    pub fn with_clamping(self, reject_fireflies: bool, clamp: Option<f32>, prefilter: bool) -> Quality {
        Quality { reject_fireflies, clamp, prefilter, ..self }
    }
}

/// Cycles' bounce limits for a render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BounceLimits {
    total: i32,
    diffuse: i32,
    glossy: i32,
    transmission: i32,
}

/// Never fewer bounces than Blender's defaults, and more where the lighting
/// asks: the Jewelry preset's twelve. A gem's inner reflections are glossy
/// bounces to Cycles, and at Blender's four a diamond went dark. Tequila
/// counts passes through glass apart; Cycles counts them in the total.
///
/// Diffuse or glossy bounces given on their own (`CyclesTransport`) are
/// taken as they are, and `bounces` is then the total for every kind
/// together, as Blender's Total is.
fn bounce_limits(quality: &Quality) -> BounceLimits {
    let (bounces, through) = (quality.bounces as i32, quality.transmission_bounces as i32);
    let t = &quality.transport;
    let explicit = t.diffuse_bounces.is_some() || t.glossy_bounces.is_some();
    BounceLimits {
        total: if explicit { bounces.max(1) } else { (bounces + through).max(12) },
        diffuse: t.diffuse_bounces.map_or(bounces.max(4), |d| d as i32),
        glossy: t.glossy_bounces.map_or(bounces.max(4), |g| g as i32),
        transmission: through.max(12),
    }
}

/// A setting for one of Cycles' scene nodes.
#[derive(Clone, Debug, PartialEq)]
pub enum Setting {
    Int(i32),
    Float(f32),
    Bool(bool),
    Text(&'static str),
}

/// What [`configure`] sets, node by node: the integrator's and the film's,
/// as `(node, socket, value)`. Apart from the session so it can be checked.
pub fn settings(quality: &Quality) -> Vec<(&'static str, &'static str, Setting)> {
    use Setting::*;
    let limits = bounce_limits(quality);
    let t = &quality.transport;
    let transparent = t.transparent_bounces.map_or(8, |b| b as i32);
    let mut out = vec![
        ("integrator", "max_bounce", Int(limits.total)),
        ("integrator", "max_diffuse_bounce", Int(limits.diffuse)),
        ("integrator", "max_glossy_bounce", Int(limits.glossy)),
        ("integrator", "max_transmission_bounce", Int(limits.transmission)),
        ("integrator", "transparent_max_bounce", Int(transparent)),
        // No Russian roulette through see-through surfaces. A path that
        // reaches a shadow-catching floor through them gives the floor its
        // strength before the roulette's survivors are made up for the paths
        // it ended, so the floor behind half-transparent glass came out
        // darker by the odds of surviving: an eighth of the light, through a
        // sphere half there. (Blender's default, 0, has the same fault.)
        ("integrator", "transparent_min_bounce", Int(transparent)),
        ("integrator", "filter_glossy", Float(t.filter_glossy.unwrap_or(1.0).max(0.0))),
        ("integrator", "use_adaptive_sampling", Bool(quality.adaptive)),
        ("integrator", "use_denoise", Bool(quality.denoise)),
    ];
    if let Some(volume) = t.volume_bounces {
        out.push(("integrator", "max_volume_bounce", Int(volume as i32)));
    }
    if let Some(on) = t.caustics_reflective {
        out.push(("integrator", "caustics_reflective", Bool(on)));
    }
    if let Some(on) = t.caustics_refractive {
        out.push(("integrator", "caustics_refractive", Bool(on)));
    }
    // Cycles reads 0 as no clamp. A direct clamp of its own leaves the other
    // for indirect light alone, firefly rejection's 10 when there is none.
    let rejection = if quality.reject_fireflies { 10.0 } else { 0.0 };
    let (direct, indirect) = match (quality.clamp_direct, quality.clamp) {
        (Some(direct), clamp) => (direct.max(0.0), clamp.map_or(rejection, |c| c.max(0.0))),
        (None, Some(clamp)) => (clamp.max(0.0), clamp.max(0.0)),
        (None, None) => (0.0, rejection),
    };
    out.push(("integrator", "sample_clamp_direct", Float(direct)));
    out.push(("integrator", "sample_clamp_indirect", Float(indirect)));
    // `DenoiserPrefilter` in Cycles' device/denoise.h: 1 none, 3 accurate.
    out.push(("integrator", "denoiser_prefilter", Int(if quality.prefilter { 3 } else { 1 })));
    if let Some(threshold) = quality.adaptive_threshold {
        out.push(("integrator", "adaptive_threshold", Float(threshold.max(1e-6))));
    }
    if let Some(filter) = quality.pixel_filter {
        out.push(("film", "filter_type", Text(filter.kind.key())));
        out.push(("film", "filter_width", Float(if filter.width > 0.0 { filter.width } else { 1.0 })));
    }
    out
}

/// Set the integrator up as Blender does by default, so a scene renders here
/// as it would there, with `quality` on top.
pub fn configure(session: &mut Session, quality: &Quality) -> Result<(), String> {
    for (node, socket, value) in settings(quality) {
        let node = session.scene_node(node)?;
        match value {
            Setting::Int(v) => session.set_int(node, socket, v)?,
            Setting::Float(v) => session.set_float(node, socket, v)?,
            Setting::Bool(v) => session.set_bool(node, socket, v)?,
            Setting::Text(v) => session.set_string(node, socket, v)?,
        }
    }
    session.set_time_limit(quality.seconds);
    Ok(())
}

/// Images already sent to a session, by the Tequila picture they came from.
///
/// Kept across translations by a live view, so a document edit that leaves
/// the textures alone does not send them again: a 4K texture is 64 MB even as
/// bytes, and a material slider would otherwise send every one of them per
/// step. Each entry holds its picture, so the address it is keyed by cannot be
/// reused by another while it is here.
#[derive(Default)]
pub struct Images {
    entries: HashMap<usize, (Kept, i32)>,
    used: std::collections::HashSet<usize>,
}

/// Held, never read: holding the picture is what keeps its address from
/// being reused by another while it keys an entry.
#[allow(dead_code)]
enum Kept {
    Texture(std::sync::Arc<tequila_texture::texture::Texture>),
    Picture(std::sync::Arc<tequila_texture::Image>),
}

impl Images {
    fn get(&mut self, key: usize) -> Option<i32> {
        let id = self.entries.get(&key).map(|(_, id)| *id)?;
        self.used.insert(key);
        Some(id)
    }

    fn insert(&mut self, key: usize, kept: Kept, id: i32) {
        self.entries.insert(key, (kept, id));
        self.used.insert(key);
    }

    /// Forget every image the last translation did not use.
    fn sweep(&mut self, session: &mut Session) {
        let used = std::mem::take(&mut self.used);
        self.entries.retain(|key, (_, id)| {
            let keep = used.contains(key);
            if !keep {
                session.forget_image(*id);
            }
            keep
        });
    }
}

/// A linear value in [0, 1] as an sRGB-encoded byte.
///
/// Through a table: a 4K texture is 67 million values, and a `powf` each made
/// sending one take half a second. 65,536 steps resolve the steepest part of
/// the curve (near black, where one 8-bit code spans 0.0003) several times
/// over.
fn srgb_byte(v: f32) -> u8 {
    static TABLE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        (0..=u16::MAX)
            .map(|i| {
                let v = tequila_core::spectrum::srgb_inverse_eotf(f32::from(i) / f32::from(u16::MAX));
                (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
            })
            .collect()
    });
    table[(v.clamp(0.0, 1.0) * f32::from(u16::MAX) + 0.5) as usize]
}

/// A picture as it goes to Cycles.
#[derive(Debug, PartialEq)]
enum Encoded {
    Bytes(Vec<u8>),
    Floats(Vec<f32>),
}

/// How far over 1 a value may be and still go as a byte, which clamps it:
/// in f32 white comes out of `srgb_to_acescg` a hair over 1, and every
/// texture with a single white texel went as floats, at four times the
/// memory and upload of bytes.
const BYTE_ROUNDING: f32 = 1e-5;

/// A picture as compactly as it allows: 8-bit when every value is in
/// [0, 1], give or take rounding, floats when it holds more than a display
/// range, as an HDRI or a height map in metres can.
///
/// A colour picture (Tequila keeps those linear, in Rec.709 primaries) goes
/// over in ACEScg, the space Cycles works in, and as 8-bit on the sRGB curve,
/// which Cycles takes off again as it reads: the curve spends the bytes where
/// the eye needs them. Rec.709 lies inside ACEScg, so its colours stay in
/// [0, 1]. Data (roughness, normals, masks) goes as it is.
fn encode_picture(image: &tequila_texture::Image, colour: bool) -> Encoded {
    use rayon::prelude::*;
    let channels = image.channels;
    let mut data = image.data.clone();
    if colour && channels >= 3 {
        data.par_chunks_exact_mut(channels).for_each(|pixel| {
            let v = tequila_core::spectrum::srgb_to_acescg(vec3(pixel[0], pixel[1], pixel[2]));
            pixel[0] = v.x;
            pixel[1] = v.y;
            pixel[2] = v.z;
        });
    }
    if !data.par_iter().all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0 + BYTE_ROUNDING) {
        return Encoded::Floats(data);
    }
    Encoded::Bytes(
        data.par_iter()
            .enumerate()
            .map(|(i, &v)| {
                // Alpha is coverage and never had the curve.
                if colour && !(channels == 4 && i % 4 == 3) {
                    srgb_byte(v)
                } else {
                    (v * 255.0 + 0.5) as u8
                }
            })
            .collect(),
    )
}

/// A label's picture as it goes to Cycles, which reads a label - an
/// "unassociated" image (`image_node`) - with its colour multiplied by its
/// coverage, filters it so, and divides the coverage out again as it looks
/// it up, before it takes the sRGB curve off (`svm_image_texture`). So bytes
/// are multiplied on the curve, as Cycles' own loader multiplies an 8-bit
/// PNG's; floats go straight, since the shim multiplies those when Cycles
/// asks (`MemoryLoader::load_pixels`). Multiplied in linear beforehand, a
/// label had its coverage taken twice as floats - a white print at a quarter
/// all but vanished - and divided out on the curve as bytes, which drew a
/// cream print at a quarter brighter than solid ink.
fn encode_label(image: &tequila_texture::Image, colour: bool) -> Encoded {
    use rayon::prelude::*;
    match encode_picture(image, colour) {
        Encoded::Bytes(mut bytes) if image.channels == 4 => {
            bytes.par_chunks_exact_mut(4).for_each(|pixel| {
                let coverage = u32::from(pixel[3]);
                for value in &mut pixel[..3] {
                    *value = ((u32::from(*value) * coverage + 127) / 255) as u8;
                }
            });
            Encoded::Bytes(bytes)
        }
        other => other,
    }
}

/// `image` on the device; `label` for a label's (`encode_label`).
fn send_picture(session: &mut Session, name: &str, image: &tequila_texture::Image, colour: bool, label: bool) -> Result<i32, String> {
    let encoded = if label { encode_label(image, colour) } else { encode_picture(image, colour) };
    match encoded {
        Encoded::Floats(data) => session.image(name, image.width, image.height, image.channels, &data),
        Encoded::Bytes(bytes) => session.image_bytes(name, image.width, image.height, image.channels, &bytes, colour),
    }
}

/// What the translation could not carry over, and the Cycles object made
/// for each instance (none for one that was skipped), so a later move can
/// place them again without translating the scene.
#[derive(Debug, Default)]
pub struct Translation {
    pub warnings: Vec<String>,
    pub objects: Vec<Option<Node>>,
    /// The film was left transparent for the caller to lay Tequila's own
    /// background under the image, with [`background`] and [`lay_under`]:
    /// a shadow-catching floor, a backplate or a colour behind, in an opaque
    /// view. Cycles' own composite of
    /// the floor counts anything see-through in front of it twice, so a
    /// glass bulb on the floor darkened by a fifth wherever the floor was
    /// behind it.
    pub background_under: bool,
    /// The floor reflects, and Cycles has drawn none: the caller lays
    /// Tequila's reflection over the image (`tequila_render_cpu::floor`).
    pub floor_reflection: bool,
    /// Each material's Cycles shader, by material index, for
    /// [`update_materials`] to build again in place.
    pub shaders: Vec<Node>,
}

impl Translation {
    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }
}

fn rows(m: &Mat4) -> [f32; 12] {
    let m = &m.m;
    [
        m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1], m[1][2], m[1][3], m[2][0], m[2][1],
        m[2][2], m[2][3],
    ]
}

/// A colour as Cycles takes it: as it is, since Cycles works in Tequila's
/// rendering space, ACEScg (the shim sets its OpenColorIO config).
fn rgb(s: Spectrum) -> [f32; 3] {
    [s.r, s.g, s.b]
}

/// A Cycles render's pixel in Tequila's rendering space: the same space.
pub fn to_rendering(r: f32, g: f32, b: f32) -> Spectrum {
    Spectrum { r, g, b }
}

/// The turn about Cycles' vertical that makes its environment lookup land
/// where Tequila's does, for an environment rotated by `rotation`.
///
/// Tequila looks the image up at u = atan2(x, -z) / 2pi + 1/2 in its own
/// frame, after turning the direction by the inverse of the environment's
/// rotation. Cycles looks up at u = (pi - atan2(y, x)) / 2pi in its Z-up
/// frame. Both are turns about the vertical: a quarter turn apart, plus the
/// environment's own.
pub fn environment_turn(rotation: &Mat4) -> f32 {
    let turned = rotation.transform_vector(vec3(0.0, 0.0, -1.0));
    let yaw = turned.x.atan2(-turned.z);
    yaw - std::f32::consts::FRAC_PI_2
}

/// Build `scene` in `session`, which should be empty.
pub fn build(session: &mut Session, scene: &SceneSnapshot, view: &View) -> Result<Translation, String> {
    build_with(session, scene, view, &mut Images::default())
}

/// The same, reusing images `images` already sent to this session, and
/// forgetting the ones this scene no longer uses.
pub fn build_with(session: &mut Session, scene: &SceneSnapshot, view: &View, images: &mut Images) -> Result<Translation, String> {
    let mut out = Translation::default();
    // What updates since the last translation used is asked again below.
    images.used.clear();
    let mut builder = Builder { session, scene, out: &mut out, images, nodes: HashMap::new(), points: HashMap::new(), turned: HashMap::new() };

    // One Cycles shader per Tequila material.
    let mut shaders = Vec::with_capacity(scene.materials.len());
    for index in 0..scene.materials.len() {
        shaders.push(builder.material(index, None)?);
    }
    builder.out.shaders = shaders.clone();
    let fallback = builder.fallback_shader()?;
    // The floor's own surface, made when there is one. Tequila's renderer
    // never shades a shadow catcher, so its ground carries whatever material
    // came first in the scene - on the bulb, polished metal - and Cycles,
    // which does shade it, caught a mirror's worth of shadow from that: a
    // faint ghost of the one Blender casts. A plain diffuse floor is what
    // Blender's catcher has by default.
    let mut catcher_shader: Option<Node> = None;
    // The colour a shadow catcher's shadow mixes towards. Cycles has one
    // catcher shadow, black; the session tints it as the image comes out.
    let tint = scene
        .instances
        .iter()
        .filter_map(|i| i.shadow_catcher.as_ref())
        .find(|c| c.shadows && !c.occlusion_only)
        .map_or(Spectrum::ZERO, |c| c.shadow_color);
    builder.session.set_shadow_tint(rgb(tint));

    // Cycles binds shaders to meshes and Tequila binds materials to
    // instances, so a mesh is built once per material it is shown in.
    let mut meshes: HashMap<(usize, usize), Node> = HashMap::new();
    for instance in &scene.instances {
        let Some(mesh) = scene.meshes.get(instance.mesh.index()) else {
            builder.out.objects.push(None);
            continue;
        };
        // A light's stand-in, for the camera to see it by: Cycles' own light,
        // visible to the camera, is that already (`Builder::lights`), and
        // lights nothing twice.
        if instance.light_proxy {
            builder.out.objects.push(None);
            continue;
        }
        let (material, shader) = match &instance.shadow_catcher {
            // A floor with a bounce: what the camera sees of it is as it
            // always was, every other ray sees the glow.
            Some(catcher) if catcher.bounce.is_some() => {
                if catcher.shadows && catcher.shadow_color != tint {
                    builder.out.warn("ground shadows of different colours all take the first one's colour in Cycles");
                }
                let seen = if catcher.shadows && catcher.occlusion_only {
                    FloorLook::Occlusion(catcher.shadow_color, catcher.occlusion_radius)
                } else {
                    FloorLook::Catcher
                };
                let glow = catcher.bounce.unwrap_or(Spectrum::ZERO);
                (usize::MAX - 2, builder.bounce_shader(seen, glow, catcher.bounce_falloff, view.ground_illumination)?)
            }
            Some(catcher) if catcher.shadows && catcher.occlusion_only => {
                (usize::MAX - 1, builder.occlusion_shader(catcher.shadow_color, catcher.occlusion_radius)?)
            }
            Some(catcher) => {
                if catcher.shadows && catcher.shadow_color != tint {
                    builder.out.warn("ground shadows of different colours all take the first one's colour in Cycles");
                }
                let shader = match catcher_shader {
                    Some(shader) => shader,
                    None => {
                        let shader = builder.catcher_shader()?;
                        catcher_shader = Some(shader);
                        shader
                    }
                };
                (usize::MAX, shader)
            }
            None => {
                let material = instance.material.index();
                (material, shaders.get(material).copied().unwrap_or(fallback))
            }
        };
        let key = (instance.mesh.index(), material);
        // An occlusion-only floor's shader is its own, and a bouncing one's.
        let cycles_mesh = match meshes.get(&key).filter(|_| material != usize::MAX - 1 && material != usize::MAX - 2) {
            Some(m) => *m,
            None => {
                let m = builder.mesh(mesh, shader)?;
                meshes.insert(key, m);
                m
            }
        };
        let mut visibility = ffi::VISIBLE_ALL;
        if !instance.visible_to_camera {
            visibility &= !ffi::VISIBLE_CAMERA;
        }
        if !instance.casts_shadows {
            visibility &= !ffi::VISIBLE_SHADOW;
        }
        if !instance.seen_by_diffuse {
            visibility &= !(ffi::VISIBLE_DIFFUSE | ffi::VISIBLE_SCATTER);
        }
        if !instance.seen_by_glossy {
            visibility &= !ffi::VISIBLE_GLOSSY;
        }
        if !instance.seen_by_transmission {
            visibility &= !ffi::VISIBLE_TRANSMISSION;
        }
        // A shadow-catching floor is no surface to anything but the camera,
        // unless the ground is meant to light the product: Cycles' floor
        // would otherwise show in every reflection and through every glass,
        // and the edge of a floor fitted to the product drew a line across
        // a light bulb. It still stops light from below, as Tequila's does.
        if instance.shadow_catcher.is_some() && !view.ground_illumination {
            visibility &= ffi::VISIBLE_CAMERA | ffi::VISIBLE_SHADOW;
        }
        // A floor that catches no shadow, there only for the reflection the
        // caller lays over the image, is no catcher and nothing the camera
        // sees; it still stops light from below, as one that catches does.
        // As a catcher, the shadow switched off showed under the reflection.
        let shadowless = instance.shadow_catcher.is_some_and(|c| !c.shadows);
        if shadowless {
            visibility &= !ffi::VISIBLE_CAMERA;
        }
        // An occlusion-only floor is no catcher to Cycles but a picture laid
        // over the background, and only the camera sees it.
        let occlusion = instance.shadow_catcher.is_some_and(|c| c.shadows && c.occlusion_only);
        if occlusion {
            visibility = ffi::VISIBLE_CAMERA;
        }
        // A bouncing floor is a surface to what the product sends out - its
        // reflections, the light it bounces, what passes through its glass -
        // which see the glow; the camera's view of it is left as above.
        if instance.shadow_catcher.is_some_and(|c| c.bounce.is_some()) {
            visibility |= ffi::VISIBLE_DIFFUSE | ffi::VISIBLE_GLOSSY | ffi::VISIBLE_TRANSMISSION;
        }
        let matrix = Y_UP_TO_Z_UP * instance.transform.matrix;
        let catches = instance.shadow_catcher.is_some_and(|c| c.shadows) && !occlusion;
        let object = builder.session.object(cycles_mesh, &rows(&matrix), visibility, catches)?;
        // One more than the instance's id, so a live frame's object pass
        // names it (0 being nothing).
        let pass_id = instance.id.0.saturating_add(1) as i32;
        builder.session.set_int(object, "pass_id", pass_id)?;
        // Its scale, for a texture of a real-world size (`Builder::position`):
        // scene units per unit of the part's own space.
        let scale = 1.0 / instance.transform.inv_length_scale();
        builder.session.set_float3(object, "color", [scale, scale, scale])?;
        // Object Info's Random, as Tequila's renderer draws it.
        let random_id = SceneSnapshot::object_random_id(instance.id.index());
        builder.session.set_int(object, "random_id", random_id as i32)?;
        if instance.holdout {
            builder.session.set_bool(object, "use_holdout", true)?;
        }
        builder.out.objects.push(Some(object));

        // A reflecting floor reflects nothing here: the caller lays
        // Tequila's reflection over the image. A mirror sheet of Cycles'
        // own reflected the environment as well as the product, so a strong
        // reflection under an HDRI showed the floor as a mirror of the sky;
        // and Cycles' catcher reflects only as a ratio of the backdrop.
        if instance.shadow_catcher.is_some_and(|c| c.reflections > 0.0) {
            builder.out.floor_reflection = true;
        }
    }
    let catcher = scene.instances.iter().any(|i| i.shadow_catcher.is_some_and(|c| c.shadows && !c.occlusion_only));
    if catcher {
        // The catcher's shadow in the image's alpha, as Blender has it
        // whenever the shadow catcher pass itself is not asked for.
        let film = builder.session.scene_node("film")?;
        builder.session.set_bool(film, "use_approximate_shadow_catcher", true)?;
    }
    // Under a catcher of either kind, behind a backplate or a colour, and
    // under a dome of a size - which Cycles' world, knowing a ray's direction
    // but not where it started, cannot draw - Tequila's own background goes
    // beneath the image, which leaves it clear where the floor is and
    // wherever the camera sees past the scene.
    let behind = !scene.background.is_environment();
    let dome = !scene.environment.dome().is_infinite();
    builder.out.background_under =
        (behind || dome || scene.instances.iter().any(|i| i.shadow_catcher.is_some())) && !view.transparent;
    builder.lights()?;

    builder.world(view.transparent || builder.out.background_under)?;
    if behind && builder.out.background_under {
        // Tequila's renderer shows a backplate or a colour along the whole
        // of a camera ray's run through smooth glass, not the environment
        // the glass would refract: Cycles' transparent glass, which counts
        // such a run as seeing through, to what goes under the image.
        let background = builder.session.scene_node("background")?;
        builder.session.set_bool(background, "transparent_glass", true)?;
        builder.session.set_float(background, "transparent_roughness_threshold", 0.02)?;
    }
    builder.camera(view)?;
    builder.images.sweep(builder.session);
    Ok(out)
}

/// What the camera sees of `scene`'s background at each pixel of a
/// `width` x `height` view, rows top to bottom, in the rendering space: the
/// same background Tequila's renderer shows, environment, colour or
/// backplate. Four rays a pixel through its quarters, for an edge as smooth
/// as the image it goes under; through the centre of the lens, so a shallow
/// depth of field leaves it sharp.
pub fn background(scene: &SceneSnapshot, camera: &Camera, width: usize, height: usize) -> Vec<Spectrum> {
    use rayon::prelude::*;
    let aspect = width as f32 / height.max(1) as f32;
    let mut out = vec![Spectrum::ZERO; width * height];
    out.par_chunks_mut(width.max(1)).enumerate().for_each(|(y, row)| {
        for (x, pixel) in row.iter_mut().enumerate() {
            let mut sum = Spectrum::ZERO;
            for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let film = tequila_core::math::vec2((x as f32 + dx) / width as f32, (y as f32 + dy) / height as f32);
                let ray = camera.pick_ray(film, aspect);
                sum += scene.background.radiance(&scene.environment, ray.origin, ray.direction, film);
            }
            *pixel = (sum * 0.25).sanitized();
        }
    });
    out
}

/// A pixel of Cycles' premultiplied film, in the rendering space, with
/// `background` showing through what it leaves uncovered.
pub fn lay_under(rgba: &[f32], background: Spectrum) -> Spectrum {
    let over = to_rendering(rgba[0], rgba[1], rgba[2]).sanitized();
    over + background * (1.0 - rgba[3].clamp(0.0, 1.0))
}

/// A texture read through a mapping: the node outputs of its colour and of
/// its alpha.
#[derive(Clone, Copy)]
struct Sampled {
    colour: (Node, &'static str),
    alpha: Slot,
}

impl Sampled {
    fn colour(&self) -> Slot {
        Slot::Link(self.colour.0, self.colour.1)
    }
}

/// The tri-planar planes, as (u, v) from the point's components: facing X,
/// facing Y, facing Z (`Mapping::tri_planar`).
const TRI_PLANES: [(&str, &str); 3] = [("Z", "Y"), ("X", "Z"), ("X", "Y")];

/// A planar projection's (u, v) from the point's components, and the one it
/// runs along (`Axis::across`).
fn across(axis: tequila_texture::projection::Axis) -> (&'static str, &'static str, &'static str) {
    match axis {
        tequila_texture::projection::Axis::X => ("Y", "Z", "X"),
        tequila_texture::projection::Axis::Y => ("X", "Z", "Y"),
        tequila_texture::projection::Axis::Z => ("X", "Y", "Z"),
    }
}

/// Where a seed moves a pattern's noise to: somewhere up to a hundred cells
/// off, in each axis, so two seeds read two unrelated stretches of it.
fn seed_offset(seed: u32) -> [f32; 3] {
    let mut h = seed.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    let mut next = || {
        h ^= h >> 15;
        h = h.wrapping_mul(0x2c1b_3c6d);
        h ^= h >> 12;
        h = h.wrapping_mul(0x297a_2d39);
        h ^= h >> 15;
        (h >> 8) as f32 / (1u32 << 24) as f32 * 100.0
    };
    [next(), next(), next()]
}

/// A number, as a slot.
fn num(x: f32) -> Slot {
    Slot::Value(Value::Float(x))
}

/// A number for keying random numbers by seed: a different one for every
/// seed and `key`, and never an integer, so it cannot meet a cell's own.
fn salt(seed: u32, key: u32) -> f32 {
    (seed_offset(seed ^ key)[0] * 10.0).floor() + 0.5
}

/// The unit vector along a named component.
fn axis_vector(component: &str) -> Vec3 {
    match component {
        "X" => Vec3::X,
        "Y" => Vec3::Y,
        _ => Vec3::Z,
    }
}

/// A mapping's scale on (u, v): repeats, or for a real-world size the
/// inverse of the size the texture spans (`Mapping::finish_anisotropic`).
fn finish_scale(mapping: &Mapping) -> (f32, f32) {
    if mapping.real_world_units {
        (1.0 / mapping.scale.x.abs().max(1e-6), 1.0 / mapping.scale.y.abs().max(1e-6))
    } else {
        (mapping.scale.x, mapping.scale.y)
    }
}

/// The ways the finished u and v grow through the part's space, for a plane
/// whose raw u and v grow along `u` and `v`: scaled, turned and flipped as
/// the coordinates are. Constant for a planar projection.
fn finished_gradients(u: Vec3, v: Vec3, mapping: &Mapping) -> (Vec3, Vec3) {
    let (su, sv) = finish_scale(mapping);
    let (sin, cos) = (mapping.rotation * std::f32::consts::TAU).sin_cos();
    let (fu, fv) = (if mapping.flip_u { -1.0 } else { 1.0 }, if mapping.flip_v { -1.0 } else { 1.0 });
    (
        (u * (cos * su) - v * (sin * sv)) * fu,
        (u * (sin * su) + v * (cos * sv)) * fv,
    )
}

/// A profile as an IES file (LM-63, photometric type C, absolute candela),
/// for Cycles, which reads profiles from their text: Tequila keeps only the
/// table it parsed.
fn ies_text(profile: &tequila_render_core::ies::IesProfile) -> String {
    use std::fmt::Write;
    let (vertical, horizontal, candela) = profile.table();
    let mut text = String::from("IESNA:LM-63-2002\n[MANUFAC] Tequila\nTILT=NONE\n");
    let _ = writeln!(text, "1 -1 1 {} {} 1 2 0 0 0", vertical.len(), horizontal.len());
    text.push_str("1 1 0\n");
    let line = |values: &[f32]| values.iter().map(|v| format!("{v}")).collect::<Vec<_>>().join(" ");
    let _ = writeln!(text, "{}", line(vertical));
    let _ = writeln!(text, "{}", line(horizontal));
    for row in candela.chunks(vertical.len().max(1)) {
        let _ = writeln!(text, "{}", line(row));
    }
    text
}

/// A conductor's colour seen straight on (F0) and Cycles' edge tint for it:
/// the exact Fresnel at 82 degrees over Schlick's from F0 there, which is
/// what the principled shader's tint scales [Hoffman 2023].
fn conductor_colours(ior: &tequila_materials::fresnel::ComplexIor) -> (Spectrum, Spectrum) {
    use tequila_materials::fresnel::conductor;
    let f0 = conductor(1.0, ior.n, ior.k);
    let edge = 1.0 / 7.0;
    let f82 = conductor(edge, ior.n, ior.k);
    let schlick = |f0: f32| f0 + (1.0 - f0) * (1.0 - edge).powi(5);
    let tint = |f: f32, f0: f32| (f / schlick(f0).max(1e-6)).clamp(0.0, 1.0);
    (f0, Spectrum { r: tint(f82.r, f0.r), g: tint(f82.g, f0.g), b: tint(f82.b, f0.b) })
}

/// Whether `old` and `new` show the same things under the same light, apart
/// from their materials and where their parts are: the same meshes (by
/// pointer, as the snapshot builder shares an unchanged one), lights, world
/// and background. `moves` lets parts be elsewhere; a shadow catcher moved is
/// always a new scene.
fn same_stage(old: &SceneSnapshot, new: &SceneSnapshot, moves: bool) -> bool {
    old.instances.len() == new.instances.len()
        && old.meshes.len() == new.meshes.len()
        && old.meshes.iter().zip(&new.meshes).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
        && old.instances.iter().zip(&new.instances).all(|(a, b)| {
            a.mesh == b.mesh
                && a.material == b.material
                && a.visible_to_camera == b.visible_to_camera
                && a.casts_shadows == b.casts_shadows
                && a.shadow_catcher == b.shadow_catcher
                && ((moves && a.shadow_catcher.is_none()) || a.transform == b.transform)
                && a.light_proxy == b.light_proxy
        })
        // By what they hold: every snapshot builds its light set anew, and a
        // comparison by pointer took every edit for new lighting - a dragged
        // part, and every step of a label being resized, translated the whole
        // scene again, meshes and all.
        && (std::sync::Arc::ptr_eq(&old.lights, &new.lights) || old.lights.lights() == new.lights.lights())
        && std::sync::Arc::ptr_eq(&old.environment, &new.environment)
        && std::mem::discriminant(&old.background) == std::mem::discriminant(&new.background)
        && match (&old.background, &new.background) {
            (Background::Color(a), Background::Color(b)) => a == b,
            (Background::Backplate { image: a }, Background::Backplate { image: b }) => std::sync::Arc::ptr_eq(a, b),
            _ => true,
        }
}

/// Whether `new` differs from `old` only in where its instances are. Such a
/// change - a part dragged with the gizmo - needs only [`move_objects`].
pub fn only_moved(old: &SceneSnapshot, new: &SceneSnapshot) -> bool {
    same_stage(old, new, true) && changed_materials_of(old, new).is_some_and(|changed| changed.is_empty())
}

/// The materials `new` changed from `old`, when nothing else differs - a
/// label resized, a colour picked - for [`update_materials`] to build again
/// in place; `None` when anything else changed too, the parts' places among
/// them.
pub fn changed_materials(old: &SceneSnapshot, new: &SceneSnapshot) -> Option<Vec<usize>> {
    if !same_stage(old, new, false) {
        return None;
    }
    changed_materials_of(old, new)
}

/// The materials that differ, in their parameters, their program, their bump
/// or a picture they read; `None` if there are not as many.
fn changed_materials_of(old: &SceneSnapshot, new: &SceneSnapshot) -> Option<Vec<usize>> {
    if old.materials.len() != new.materials.len() || old.programs.len() != new.programs.len() || old.bumps.len() != new.bumps.len() {
        return None;
    }
    fn same<T>(a: Option<&std::sync::Arc<T>>, b: Option<&std::sync::Arc<T>>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
    let same_textures = std::sync::Arc::ptr_eq(&old.textures, &new.textures) || old.textures.same_as(&new.textures);
    let changed = (0..new.materials.len())
        .filter(|&i| {
            let program = new.programs.get(i).and_then(|p| p.as_ref());
            old.materials[i] != new.materials[i]
                || !same(old.programs.get(i).and_then(|p| p.as_ref()), program)
                || !same(old.bumps.get(i).and_then(|b| b.as_ref()), new.bumps.get(i).and_then(|b| b.as_ref()))
                || (!same_textures
                    && program.is_some_and(|program| {
                        program.requirements.textures.iter().any(|id| !old.textures.same_entry(&new.textures, *id))
                    }))
        })
        .collect();
    Some(changed)
}

/// Build the shaders of materials `changed` again in place, as `scene` has
/// them, from an earlier translation's `shaders`: every mesh keeps its
/// shader, and nothing but the shaders is touched. `Ok(false)` when the
/// Cycles library cannot rebuild a shader in place, and a full translation
/// is needed.
///
/// What a label dragged to a new size costs: one shader, where translating
/// the scene again sent every mesh to the device and built its BVH anew at
/// each step - on a 200 MB model, at the speed of the mouse.
pub fn update_materials(
    session: &mut Session,
    scene: &SceneSnapshot,
    shaders: &[Node],
    changed: &[usize],
    images: &mut Images,
) -> Result<bool, String> {
    if !session.can_reopen() || changed.iter().any(|&i| i >= shaders.len()) {
        return Ok(false);
    }
    let mut out = Translation::default();
    let mut builder = Builder { session, scene, out: &mut out, images, nodes: HashMap::new(), points: HashMap::new(), turned: HashMap::new() };
    for &index in changed {
        builder.material(index, Some(shaders[index]))?;
    }
    for warning in &out.warnings {
        eprintln!("Cycles: {warning}");
    }
    Ok(true)
}

/// Place the objects of an earlier translation where `scene` has them.
pub fn move_objects(session: &mut Session, scene: &SceneSnapshot, objects: &[Option<Node>]) -> Result<(), String> {
    for (instance, object) in scene.instances.iter().zip(objects) {
        if let Some(object) = object {
            let matrix = Y_UP_TO_Z_UP * instance.transform.matrix;
            session.set_transform(*object, "tfm", &rows(&matrix))?;
            // Its scale with it, which textures of a real-world size read: a
            // part scaled with the gizmo otherwise stretched them with it
            // until the next full translation.
            let scale = 1.0 / instance.transform.inv_length_scale();
            session.set_float3(*object, "color", [scale, scale, scale])?;
        }
    }
    Ok(())
}

/// Move the camera alone, for a view whose scene has not changed. Cheap: no
/// geometry or textures are touched.
pub fn set_camera(session: &mut Session, scene: &SceneSnapshot, view: &View) -> Result<Translation, String> {
    let mut out = Translation::default();
    let mut images = Images::default();
    let mut builder = Builder { session, scene, out: &mut out, images: &mut images, nodes: HashMap::new(), points: HashMap::new(), turned: HashMap::new() };
    builder.camera(view)?;
    Ok(out)
}

/// Where a register's value comes from: a literal, or a node's output.
#[derive(Clone, Copy, Debug)]
enum Slot {
    Value(Value),
    Link(Node, &'static str),
}

struct Builder<'a> {
    session: &'a mut Session,
    scene: &'a SceneSnapshot,
    out: &'a mut Translation,
    images: &'a mut Images,
    /// The shader being translated's Blender nodes, by node and inputs, so
    /// a node read at two of its outputs is one Cycles node.
    nodes: HashMap<String, Node>,
    /// What it has read of the shading point.
    points: HashMap<PointInput, Slot>,
    /// A vector turned from Cycles' world into Tequila's, back to what it
    /// was turned from.
    turned: HashMap<Node, (Node, &'static str)>,
}

/// What the camera sees of a floor whose bounce the rest of the scene sees.
#[derive(Clone, Copy)]
enum FloorLook {
    /// The shadow catcher's own surface.
    Catcher,
    /// An occlusion-only floor, of this tint over this radius.
    Occlusion(Spectrum, f32),
}

impl Builder<'_> {
    /// A floor with a bounce (`ShadowCatcher::bounce`): to camera rays the
    /// floor it would be without one; to every other ray an emission of the
    /// bounce's radiance, darkened toward the contact by ambient occlusion
    /// over `falloff` - the white sweep a photo studio puts under a product,
    /// which its reflections see and the picture does not. With ground
    /// illumination on, the lit floor is there to those rays as well.
    fn bounce_shader(&mut self, seen: FloorLook, glow: Spectrum, falloff: f32, lit: bool) -> Result<Node, String> {
        let shader = self.session.shader("bouncing ground")?;
        let camera = match seen {
            FloorLook::Catcher => {
                let bsdf = self.session.node(shader, "diffuse_bsdf")?;
                self.session.set_float3(bsdf, "color", [0.8, 0.8, 0.8])?;
                (bsdf, "BSDF")
            }
            FloorLook::Occlusion(tint, radius) => {
                let ao = self.session.node(shader, "ambient_occlusion")?;
                self.session.set_float(ao, "Distance", radius.max(1e-3))?;
                self.session.set_int(ao, "Samples", 16)?;
                let shade = self.session.node(shader, "emission")?;
                self.session.set_float3(shade, "Color", rgb(tint))?;
                self.session.set_float(shade, "Strength", 1.0)?;
                let clear = self.session.node(shader, "transparent_bsdf")?;
                self.session.set_float3(clear, "Color", [1.0, 1.0, 1.0])?;
                let mix = self.session.node(shader, "mix_closure")?;
                self.session.connect(shader, ao, "AO", mix, "Fac")?;
                self.session.connect(shader, shade, "Emission", mix, "Closure1")?;
                self.session.connect(shader, clear, "BSDF", mix, "Closure2")?;
                (mix, "Closure")
            }
        };
        let emission = self.session.node(shader, "emission")?;
        self.session.set_float3(emission, "Color", rgb(glow))?;
        self.session.set_float(emission, "Strength", 1.0)?;
        if falloff > 0.0 {
            let ao = self.session.node(shader, "ambient_occlusion")?;
            self.session.set_float(ao, "Distance", falloff)?;
            self.session.set_int(ao, "Samples", 16)?;
            self.session.connect(shader, ao, "AO", emission, "Strength")?;
        }
        let mut away = (emission, "Emission");
        if lit {
            let lit_floor = self.session.node(shader, "diffuse_bsdf")?;
            self.session.set_float3(lit_floor, "color", [0.8, 0.8, 0.8])?;
            let add = self.session.node(shader, "add_closure")?;
            self.session.connect(shader, emission, "Emission", add, "Closure1")?;
            self.session.connect(shader, lit_floor, "BSDF", add, "Closure2")?;
            away = (add, "Closure");
        }
        let path = self.session.node(shader, "light_path")?;
        let pick = self.session.node(shader, "mix_closure")?;
        self.session.connect(shader, path, "Is Camera Ray", pick, "Fac")?;
        self.session.connect(shader, away.0, away.1, pick, "Closure1")?;
        self.session.connect(shader, camera.0, camera.1, pick, "Closure2")?;
        let output = self.session.node(shader, "output")?;
        self.session.connect(shader, pick, "Closure", output, "Surface")?;
        self.session.commit(shader)
    }

    /// A shadow catcher's surface: plain white diffuse, as Blender gives a
    /// catcher with no material of its own.
    fn catcher_shader(&mut self) -> Result<Node, String> {
        let shader = self.session.shader("ground")?;
        let bsdf = self.session.node(shader, "diffuse_bsdf")?;
        self.session.set_float3(bsdf, "color", [0.8, 0.8, 0.8])?;
        let output = self.session.node(shader, "output")?;
        self.session.connect(shader, bsdf, "BSDF", output, "Surface")?;
        self.session.commit(shader)
    }

    /// An occlusion-only floor, as Tequila's renderer composites one: what is
    /// behind it, mixed towards the shadow colour by how much of the sky
    /// things near it hide - the share of cosine-weighted rays meeting
    /// something within `radius`, which is one less Cycles' ambient
    /// occlusion. Seen through, it is clear, and the background laid under
    /// the image shows; its darkening is the shadow colour glowing over it.
    fn occlusion_shader(&mut self, tint: Spectrum, radius: f32) -> Result<Node, String> {
        let shader = self.session.shader("occlusion floor")?;
        let ao = self.session.node(shader, "ambient_occlusion")?;
        self.session.set_float(ao, "Distance", radius.max(1e-3))?;
        self.session.set_int(ao, "Samples", 16)?;
        let shade = self.session.node(shader, "emission")?;
        self.session.set_float3(shade, "Color", rgb(tint))?;
        self.session.set_float(shade, "Strength", 1.0)?;
        let clear = self.session.node(shader, "transparent_bsdf")?;
        self.session.set_float3(clear, "Color", [1.0, 1.0, 1.0])?;
        // All the way clear where nothing is near (AO 1), all shadow where
        // everything is (AO 0).
        let mix = self.session.node(shader, "mix_closure")?;
        self.session.connect(shader, ao, "AO", mix, "Fac")?;
        self.session.connect(shader, shade, "Emission", mix, "Closure1")?;
        self.session.connect(shader, clear, "BSDF", mix, "Closure2")?;
        let output = self.session.node(shader, "output")?;
        self.session.connect(shader, mix, "Closure", output, "Surface")?;
        self.session.commit(shader)
    }

    /// What fills a transmissive material with a depth: its colour is what
    /// light keeps after crossing that depth, absorbed along the way, and its
    /// scatter colour is scattered over the same distance; density thickens
    /// both. The coefficients are Tequila's own medium's
    /// (`Material::transmittance_over`, `Material::volume_scattering`), each
    /// as a Cycles volume whose colour and density multiply to it.
    fn volume(&mut self, shader: Shader, output: Node, m: &Material, textured: bool) -> Result<(), String> {
        if textured {
            self.out.warn("a textured colour inside a volume is not translated; its constant colour fills it");
        }
        let depth = m.transmission_depth / m.volume_density.max(1e-6);
        let c = m.transmission_color;
        let absorption = [c.r, c.g, c.b].map(|v| -v.clamp(1e-4, 1.0).ln() / depth);
        let s = m.transmission_scatter;
        let scattering = [s.r, s.g, s.b].map(|v| v.max(0.0) / depth);
        let mut filled: Option<(Node, &str)> = None;
        let strongest = absorption.iter().copied().fold(0.0f32, f32::max);
        if strongest > 0.0 {
            let node = self.session.node(shader, "absorption_volume")?;
            self.session.set_float(node, "Density", strongest)?;
            self.session.set_float3(node, "Color", absorption.map(|a| 1.0 - a / strongest))?;
            filled = Some((node, "Volume"));
        }
        let strongest = scattering.iter().copied().fold(0.0f32, f32::max);
        if strongest > 0.0 {
            let node = self.session.node(shader, "scatter_volume")?;
            self.session.set_float(node, "Density", strongest)?;
            self.session.set_float3(node, "Color", scattering.map(|s| s / strongest))?;
            self.session.set_float(node, "Anisotropy", m.transmission_scatter_anisotropy.clamp(-0.95, 0.95))?;
            filled = Some(match filled {
                Some((absorbing, socket)) => {
                    let both = self.session.node(shader, "add_closure")?;
                    self.session.connect(shader, absorbing, socket, both, "Closure1")?;
                    self.session.connect(shader, node, "Volume", both, "Closure2")?;
                    (both, "Closure")
                }
                None => (node, "Volume"),
            });
        }
        // A glow inside: an emission on the volume output is Cycles' volume
        // emission, radiance per metre, as Blender's Emission there is.
        if m.volume_emission_strength > 0.0 {
            let glow = self.session.node(shader, "emission")?;
            self.session.set_float3(glow, "Color", rgb(m.volume_emission_color))?;
            self.session.set_float(glow, "Strength", m.volume_emission_strength)?;
            filled = Some(match filled {
                Some((medium, socket)) => {
                    let both = self.session.node(shader, "add_closure")?;
                    self.session.connect(shader, medium, socket, both, "Closure1")?;
                    self.session.connect(shader, glow, "Emission", both, "Closure2")?;
                    (both, "Closure")
                }
                None => (glow, "Emission"),
            });
        }
        if let Some((node, socket)) = filled {
            self.session.connect(shader, node, socket, output, "Volume")?;
        }
        Ok(())
    }

    fn fallback_shader(&mut self) -> Result<Node, String> {
        let shader = self.session.shader("fallback")?;
        let bsdf = self.session.node(shader, "principled_bsdf")?;
        let output = self.session.node(shader, "output")?;
        self.session.connect(shader, bsdf, "BSDF", output, "Surface")?;
        self.session.commit(shader)
    }

    fn mesh(&mut self, mesh: &tequila_render_core::scene::Mesh, shader: Node) -> Result<Node, String> {
        let positions: Vec<[f32; 3]> = mesh.positions.iter().map(|p| [p.x, p.y, p.z]).collect();
        let normals: Option<Vec<[f32; 3]>> = (mesh.normals.len() == mesh.positions.len())
            .then(|| mesh.normals.iter().map(|n| [n.x, n.y, n.z]).collect());
        let triangles: Vec<[i32; 3]> = mesh
            .indices
            .chunks_exact(3)
            .map(|t| [t[0] as i32, t[1] as i32, t[2] as i32])
            .collect();
        // Per corner, and with v flipped: Tequila's v runs down the image
        // from its top row, Cycles' up from its bottom one.
        let uvs: Option<Vec<[f32; 2]>> = (mesh.uvs.len() == mesh.positions.len()).then(|| {
            mesh.indices
                .iter()
                .map(|&i| {
                    let uv = mesh.uvs[i as usize];
                    [uv.x, 1.0 - uv.y]
                })
                .collect()
        });
        let node = self.session.mesh(&positions, normals.as_deref(), &triangles, uvs.as_deref(), &[shader])?;
        // Painted vertices, in the rendering space as Tequila's renderer
        // reads them (`SceneSnapshot::vertex_color_at`).
        if mesh.colors.len() == mesh.positions.len() && !mesh.colors.is_empty() && self.session.has_vertex_colours() {
            let rgba: Vec<[f32; 4]> = mesh
                .colors
                .iter()
                .map(|c| {
                    let v = tequila_core::spectrum::Spectrum::from_linear_srgb(tequila_core::math::vec3(c.x, c.y, c.z)).to_rendering_rgb();
                    [v.x, v.y, v.z, 1.0]
                })
                .collect();
            self.session.mesh_colors(node, &rgba)?;
        }
        Ok(node)
    }

    /// `texture` on the device; `label` for a label's, a copy of its own
    /// that Cycles reads with its coverage folded in (`encode_label`).
    fn image(&mut self, texture: TextureId, label: bool) -> Result<Option<i32>, String> {
        let Some(source) = self.scene.textures.get(texture) else {
            self.out.warn(format!("texture {} is missing", texture.0));
            return Ok(None);
        };
        // Pointers are aligned, so the low bit is free to tell the two apart.
        let key = std::sync::Arc::as_ptr(source) as usize | usize::from(label);
        if let Some(id) = self.images.get(key) {
            return Ok(Some(id));
        }
        let base = source.pyramid().base();
        // Colour, in Rec.709 primaries, goes over in ACEScg; data
        // (roughness, normals, masks) as it is.
        let colour = base.color_space == tequila_texture::image::ColorSpace::LinearSrgb;
        let id = if label && base.channels == 4 {
            send_picture(self.session, &format!("label{}", texture.0), base, colour, true)?
        } else {
            send_picture(self.session, &format!("texture{}", texture.0), base, colour, false)?
        };
        self.images.insert(key, Kept::Texture(source.clone()), id);
        Ok(Some(id))
    }

    /// `texture` read through `mapping`.
    fn texture_node(&mut self, shader: Shader, texture: TextureId, mapping: &Mapping) -> Result<Option<Sampled>, String> {
        self.texture_node_with(shader, texture, mapping, false)
    }

    /// `texture` read through `mapping`; `clip` leaves it transparent outside
    /// the picture, as a label is. A tri-planar mapping reads it three times,
    /// along each axis, and blends the three by how much the surface faces
    /// each (`Mapping::tri_planar`).
    fn texture_node_with(&mut self, shader: Shader, texture: TextureId, mapping: &Mapping, clip: bool) -> Result<Option<Sampled>, String> {
        let Some(image) = self.image(texture, clip)? else {
            return Ok(None);
        };
        if mapping.uv_set != 0 {
            self.out.warn("only the first UV set is translated");
        }
        if mapping.projection != TextureProjection::TriPlanar {
            let vector = self.mapped_vector(shader, mapping)?;
            let node = self.image_node(shader, image, mapping, clip, vector)?;
            return Ok(Some(Sampled { colour: (node, "Color"), alpha: Slot::Link(node, "Alpha") }));
        }
        let weights = self.tri_planar_weights(shader, mapping)?;
        let (position, socket) = self.position(shader, mapping)?;
        let split = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, position, socket, split, "Vector")?;
        let mut colour: Option<(Node, &'static str)> = None;
        let mut alpha = Slot::Value(Value::Float(0.0));
        for ((a, b), weight) in TRI_PLANES.iter().map(|plane| (plane.0, plane.1)).zip(weights) {
            let vector = self.finished(shader, Slot::Link(split, a), Slot::Link(split, b), mapping)?;
            let node = self.image_node(shader, image, mapping, clip, Some(vector))?;
            let share = self.vector_op(shader, "scale", (node, "Color"), None, Some(weight))?;
            colour = Some(match colour {
                None => share,
                Some(sum) => self.vector_op(shader, "add", sum, Some(share), None)?,
            });
            alpha = self.float_op3(shader, "multiply_add", Slot::Link(node, "Alpha"), weight, alpha)?;
        }
        Ok(Some(Sampled { colour: colour.expect("three planes"), alpha }))
    }

    /// One image texture node on `image`, looked up at `vector` (the mesh's
    /// UVs when `None`).
    fn image_node(&mut self, shader: Shader, image: i32, mapping: &Mapping, clip: bool, vector: Option<(Node, &'static str)>) -> Result<Node, String> {
        let node = self.session.node(shader, "image_texture")?;
        let extension = match mapping.wrap_u {
            _ if clip => "black",
            tequila_texture::Wrap::Repeat => "periodic",
            tequila_texture::Wrap::Clamp => "clamp",
            tequila_texture::Wrap::Mirror => "mirror",
        };
        self.session.set_string(node, "extension", extension)?;
        // Colour and coverage as stored, nothing dividing one by the other -
        // except a label's, which is filtered premultiplied, as Tequila
        // filters labels (multiplied as it goes over: `encode_label`), and
        // divided out again: between an inked texel and a clear one the ink
        // keeps its colour and only its coverage falls off. Filtered
        // straight, the colour fell off too, and every edge of a label faded
        // twice as fast; and whatever colour the clear texels hold - often
        // white - would bleed into the ink.
        self.session.set_string(node, "alpha_type", if clip { "unassociated" } else { "channel_packed" })?;
        self.session.set_image(node, image)?;
        if let Some((from, socket)) = vector {
            self.session.connect(shader, from, socket, node, "Vector")?;
        }
        Ok(node)
    }

    /// The point a projection measures: in the part's own space - Cycles'
    /// Object coordinates here, the turn to Z up being the object's - and,
    /// for a real-world size, in scene units, the part's own scale applied
    /// (`Mapping::in_frame_units`). The scale is the object's, carried in its
    /// colour (see `build_with`), since one shader serves parts of any size.
    fn position(&mut self, shader: Shader, mapping: &Mapping) -> Result<(Node, &'static str), String> {
        let coords = self.session.node(shader, "texture_coordinate")?;
        if !mapping.real_world_units {
            return Ok((coords, "Object"));
        }
        let info = self.session.node(shader, "object_info")?;
        let scaled = self.session.node(shader, "vector_math")?;
        self.session.set_string(scaled, "math_type", "multiply")?;
        self.session.connect(shader, coords, "Object", scaled, "Vector1")?;
        self.session.connect(shader, info, "Color", scaled, "Vector2")?;
        Ok((scaled, "Vector"))
    }

    /// Where a planar label keeps to one side: one on faces whose object-space
    /// normal looks that way along its axis, else none. `None` for a label
    /// on both sides, which needs nothing.
    fn label_side(&mut self, shader: Shader, decal: &tequila_materials::decal::Decal) -> Result<Option<Slot>, String> {
        use tequila_materials::decal::DecalSide;
        if decal.side == DecalSide::Both || decal.mapping.projection != tequila_texture::projection::Projection::Planar {
            return Ok(None);
        }
        let coords = self.session.node(shader, "texture_coordinate")?;
        let normal = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, coords, "Normal", normal, "Vector")?;
        let axis = match decal.mapping.axis {
            tequila_texture::projection::Axis::X => "X",
            tequila_texture::projection::Axis::Y => "Y",
            tequila_texture::projection::Axis::Z => "Z",
        };
        let op = if decal.side == DecalSide::Positive { "greater_than" } else { "less_than" };
        let threshold = if decal.side == DecalSide::Positive {
            tequila_materials::decal::ONE_SIDED_COSINE
        } else {
            -tequila_materials::decal::ONE_SIDED_COSINE
        };
        Ok(Some(self.float_op(shader, op, Slot::Link(normal, axis), Slot::Value(Value::Float(threshold)))?))
    }

    /// How much each of the tri-planar planes counts at a point: its axis's
    /// share of the normal, raised to the blend sharpness, the three made to
    /// sum to one (`Mapping::tri_planar`).
    fn tri_planar_weights(&mut self, shader: Shader, mapping: &Mapping) -> Result<[Slot; 3], String> {
        let coords = self.session.node(shader, "texture_coordinate")?;
        let normal = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, coords, "Normal", normal, "Vector")?;
        let sharpness = Slot::Value(Value::Float(mapping.blend_sharpness.max(1.0)));
        let mut raw = Vec::new();
        for axis in ["X", "Y", "Z"] {
            let along = self.float_op(shader, "absolute", Slot::Link(normal, axis), Slot::Value(Value::Float(0.0)))?;
            raw.push(self.float_op(shader, "power", along, sharpness)?);
        }
        let total = self.float_op(shader, "add", raw[0], raw[1])?;
        let total = self.float_op(shader, "add", total, raw[2])?;
        Ok([
            self.float_op(shader, "divide", raw[0], total)?,
            self.float_op(shader, "divide", raw[1], total)?,
            self.float_op(shader, "divide", raw[2], total)?,
        ])
    }

    /// Where `mapping` reads a texture, as Cycles looks it up: `None` for the
    /// mesh's own UVs as they stand. Not for tri-planar, which reads three
    /// times (`texture_node_with`).
    ///
    /// Tequila's projections (`Mapping::project`) take the point to a
    /// (u, v): across the axis for Planar, the angle about it and the height
    /// along it for Cylindrical, longitude and latitude for Spherical; then
    /// [`Builder::finished`].
    fn mapped_vector(&mut self, shader: Shader, mapping: &Mapping) -> Result<Option<(Node, &'static str)>, String> {
        use std::f32::consts::{PI, TAU};
        let finish_identity = mapping.scale.x == 1.0
            && mapping.scale.y == 1.0
            && mapping.rotation == 0.0
            && mapping.offset.x == 0.0
            && mapping.offset.y == 0.0
            && !mapping.flip_u
            && !mapping.flip_v;
        if mapping.projection == TextureProjection::Uv && finish_identity {
            return Ok(None);
        }
        let split = self.session.node(shader, "separate_xyz")?;
        let one = Slot::Value(Value::Float(1.0));
        let (u, v): (Slot, Slot) = match mapping.projection {
            TextureProjection::Uv => {
                // The UV attribute holds Tequila's v already turned over.
                let coords = self.session.node(shader, "texture_coordinate")?;
                self.session.connect(shader, coords, "UV", split, "Vector")?;
                let v = self.float_op(shader, "subtract", one, Slot::Link(split, "Y"))?;
                (Slot::Link(split, "X"), v)
            }
            TextureProjection::Planar => {
                let (position, socket) = self.position(shader, mapping)?;
                self.session.connect(shader, position, socket, split, "Vector")?;
                let (a, b, _) = across(mapping.axis);
                (Slot::Link(split, a), Slot::Link(split, b))
            }
            TextureProjection::Cylindrical => {
                let (position, socket) = self.position(shader, mapping)?;
                self.session.connect(shader, position, socket, split, "Vector")?;
                let (a, b, along) = across(mapping.axis);
                let angle = self.float_op(shader, "arctan2", Slot::Link(split, b), Slot::Link(split, a))?;
                let u = self.float_op3(shader, "multiply_add", angle, Slot::Value(Value::Float(1.0 / TAU)), Slot::Value(Value::Float(0.5)))?;
                (u, Slot::Link(split, along))
            }
            TextureProjection::Spherical => {
                let (position, socket) = self.position(shader, mapping)?;
                self.session.connect(shader, position, socket, split, "Vector")?;
                let length = self.session.node(shader, "vector_math")?;
                self.session.set_string(length, "math_type", "length")?;
                self.session.connect(shader, position, socket, length, "Vector1")?;
                let angle = self.float_op(shader, "arctan2", Slot::Link(split, "Z"), Slot::Link(split, "X"))?;
                let u = self.float_op3(shader, "multiply_add", angle, Slot::Value(Value::Float(1.0 / TAU)), Slot::Value(Value::Float(0.5)))?;
                let height = self.float_op(shader, "divide", Slot::Link(split, "Y"), Slot::Link(length, "Value"))?;
                let height = self.float_op(shader, "minimum", height, one)?;
                let height = self.float_op(shader, "maximum", height, Slot::Value(Value::Float(-1.0)))?;
                let latitude = self.float_op(shader, "arccosine", height, Slot::Value(Value::Float(0.0)))?;
                let v = self.float_op(shader, "multiply", latitude, Slot::Value(Value::Float(1.0 / PI)))?;
                (u, v)
            }
            TextureProjection::TriPlanar => unreachable!("read three times by texture_node_with"),
        };
        Ok(Some(self.finished(shader, u, v, mapping)?))
    }

    /// A projected (u, v) finished as every projection is: scale, a turn
    /// about the pivot, the offset, the flips; and Cycles counts v up from
    /// the bottom row where Tequila counts it down from the top.
    fn finished(&mut self, shader: Shader, u: Slot, v: Slot, mapping: &Mapping) -> Result<(Node, &'static str), String> {
        let uv = self.session.node(shader, "combine_xyz")?;
        self.feed(shader, uv, "X", u, false)?;
        self.feed(shader, uv, "Y", v, false)?;
        // Scale, then the turn about the pivot, then the offset - as one
        // point mapping, whose order is scale, rotation, location: a turn R
        // about pivot p and an offset o are R about the origin and a move of
        // p - R p + o.
        let (su, sv) = finish_scale(mapping);
        let angle = mapping.rotation * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        let (px, py) = (mapping.pivot.x, mapping.pivot.y);
        let placed = self.session.node(shader, "mapping")?;
        self.session.set_string(placed, "mapping_type", "point")?;
        self.session.set_float3(placed, "scale", [su, sv, 1.0])?;
        self.session.set_float3(placed, "rotation", [0.0, 0.0, angle])?;
        self.session.set_float3(
            placed,
            "location",
            [px - (px * cos - py * sin) + mapping.offset.x, py - (px * sin + py * cos) + mapping.offset.y, 0.0],
        )?;
        self.session.connect(shader, uv, "Vector", placed, "Vector")?;
        // The flips, and v counted from the bottom.
        let (fu, fv) = (if mapping.flip_u { -1.0 } else { 1.0 }, if mapping.flip_v { -1.0 } else { 1.0 });
        let cycles = self.session.node(shader, "mapping")?;
        self.session.set_string(cycles, "mapping_type", "point")?;
        self.session.set_float3(cycles, "scale", [fu, -fv, 1.0])?;
        self.session.set_float3(cycles, "location", [0.0, 1.0, 0.0])?;
        self.session.connect(shader, placed, "Vector", cycles, "Vector")?;
        Ok((cycles, "Vector"))
    }

    /// A vector math node: `op` on `a` and `b`, or on `a` scaled by `scale`.
    fn vector_op(
        &mut self,
        shader: Shader,
        op: &str,
        a: (Node, &'static str),
        b: Option<(Node, &'static str)>,
        scale: Option<Slot>,
    ) -> Result<(Node, &'static str), String> {
        let node = self.session.node(shader, "vector_math")?;
        self.session.set_string(node, "math_type", op)?;
        self.session.connect(shader, a.0, a.1, node, "Vector1")?;
        if let Some((from, socket)) = b {
            self.session.connect(shader, from, socket, node, "Vector2")?;
        }
        if let Some(scale) = scale {
            self.feed(shader, node, "Scale", scale, false)?;
        }
        Ok((node, if op == "dot_product" || op == "length" { "Value" } else { "Vector" }))
    }

    /// A projected normal map, as Tequila decodes one
    /// (`Texture::sample_normal_with`): each plane's texel in that plane's own
    /// frame - the tangent the way u grows along the surface, the bitangent
    /// against the way v grows, both laid flat on it - the planes blended by
    /// their weights. In world space, for a shader's Normal. `None` for a
    /// projection whose frame bends from point to point, which is not
    /// translated.
    fn projected_normal(&mut self, shader: Shader, texture: TextureId, mapping: &Mapping) -> Result<Option<(Node, &'static str)>, String> {
        let planes: Vec<(&'static str, &'static str, Vec3, Vec3, Slot)> = match mapping.projection {
            TextureProjection::Planar => {
                let (a, b, _) = across(mapping.axis);
                vec![(a, b, axis_vector(a), axis_vector(b), Slot::Value(Value::Float(1.0)))]
            }
            TextureProjection::TriPlanar => {
                let weights = self.tri_planar_weights(shader, mapping)?;
                TRI_PLANES
                    .iter()
                    .zip(weights)
                    .map(|(&(a, b), weight)| (a, b, axis_vector(a), axis_vector(b), weight))
                    .collect()
            }
            _ => {
                self.out.warn(format!("{:?} normal maps are not translated", mapping.projection));
                return Ok(None);
            }
        };
        let Some(image) = self.image(texture, false)? else {
            return Ok(None);
        };
        let coords = self.session.node(shader, "texture_coordinate")?;
        let normal = (coords, "Normal");
        let (position, socket) = self.position(shader, mapping)?;
        let split = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, position, socket, split, "Vector")?;
        let mut total: Option<(Node, &'static str)> = None;
        for (a, b, grow_u, grow_v, weight) in planes {
            let vector = self.finished(shader, Slot::Link(split, a), Slot::Link(split, b), mapping)?;
            let node = self.image_node(shader, image, mapping, false, Some(vector))?;
            let (across_u, down_v) = finished_gradients(grow_u, grow_v, mapping);
            let tangent = self.flat_along(shader, normal, across_u)?;
            let down = self.flat_along(shader, normal, down_v)?;
            // Green is up the image, and v runs down it.
            let texel = self.session.node(shader, "separate_xyz")?;
            self.session.connect(shader, node, "Color", texel, "Vector")?;
            let two = Slot::Value(Value::Float(2.0));
            let minus = Slot::Value(Value::Float(-1.0));
            let x = self.float_op3(shader, "multiply_add", Slot::Link(texel, "X"), two, minus)?;
            let y = self.float_op3(shader, "multiply_add", Slot::Link(texel, "Y"), two, minus)?;
            let z = self.float_op3(shader, "multiply_add", Slot::Link(texel, "Z"), two, minus)?;
            let z = self.float_op(shader, "maximum", z, Slot::Value(Value::Float(1e-4)))?;
            let along_t = self.vector_op(shader, "scale", tangent, None, Some(x))?;
            let along_d = self.vector_op(shader, "scale", down, None, Some(y))?;
            let along_n = self.vector_op(shader, "scale", normal, None, Some(z))?;
            let lean = self.vector_op(shader, "subtract", along_t, Some(along_d), None)?;
            let decoded = self.vector_op(shader, "add", lean, Some(along_n), None)?;
            let decoded = self.vector_op(shader, "normalize", decoded, None, None)?;
            let share = self.vector_op(shader, "scale", decoded, None, Some(weight))?;
            total = Some(match total {
                None => share,
                Some(sum) => self.vector_op(shader, "add", sum, Some(share), None)?,
            });
        }
        let total = self.vector_op(shader, "normalize", total.expect("a plane"), None, None)?;
        let world = self.session.node(shader, "vector_transform")?;
        self.session.set_string(world, "transform_type", "normal")?;
        self.session.set_string(world, "convert_from", "object")?;
        self.session.set_string(world, "convert_to", "world")?;
        self.session.connect(shader, total.0, total.1, world, "Vector")?;
        Ok(Some((world, "Vector")))
    }

    /// Metallic paint's flakes over `body`, as Tequila's renderer lays them
    /// (`tequila_render_core::flakes`, `OpenPbrBsdf`): the surface cut into
    /// cells a flake across, in scene units; a cell holding a flake by the
    /// flake amount; each flake leaning its own way, as far as the spread
    /// allows (sixty degrees at most), and a mirror of aluminium in the flake
    /// colour at the flake roughness. Where a flake is, it and the clear coat
    /// over it stand in place of the paint - the coat as a gloss by its
    /// Fresnel share, the paint's own coat being part of the paint.
    fn flakes(&mut self, shader: Shader, m: &Material, body: (Node, &'static str)) -> Result<(Node, &'static str), String> {
        use std::f32::consts::{FRAC_PI_3, TAU};
        let coords = self.session.node(shader, "texture_coordinate")?;
        let info = self.session.node(shader, "object_info")?;
        let place = self.vector_op(shader, "multiply", (coords, "Object"), Some((info, "Color")), None)?;
        let cells = self.session.node(shader, "voronoi_texture")?;
        self.session.connect(shader, place.0, place.1, cells, "Vector")?;
        // "scale", not "Scale": a texture node's own mapping has a Scale too.
        self.session.set_float(cells, "scale", 1.0 / m.flake_size.max(1e-7))?;
        // Three numbers a cell: whether it holds a flake, how far the flake
        // leans, and which way.
        let random = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, cells, "Color", random, "Vector")?;
        let zero = Slot::Value(Value::Float(0.0));
        let one = Slot::Value(Value::Float(1.0));
        let present = self.float_op(shader, "less_than", Slot::Link(random, "X"), Slot::Value(Value::Float(m.flake_weight)))?;
        let cap = (m.flake_spread.clamp(0.0, 1.0) * FRAC_PI_3).cos();
        let cos_t = self.float_op3(shader, "multiply_add", Slot::Link(random, "Y"), Slot::Value(Value::Float(cap - 1.0)), one)?;
        let square = self.float_op(shader, "multiply", cos_t, cos_t)?;
        let rest = self.float_op(shader, "subtract", one, square)?;
        let rest = self.float_op(shader, "maximum", rest, zero)?;
        let sin_t = self.float_op(shader, "sqrt", rest, zero)?;
        let phi = self.float_op(shader, "multiply", Slot::Link(random, "Z"), Slot::Value(Value::Float(TAU)))?;
        let cos_p = self.float_op(shader, "cosine", phi, zero)?;
        let sin_p = self.float_op(shader, "sine", phi, zero)?;
        let x = self.float_op(shader, "multiply", sin_t, cos_p)?;
        let y = self.float_op(shader, "multiply", sin_t, sin_p)?;
        // The surface's own frame, any way round it: the flakes' turns are
        // uniform, so only the normal matters.
        let geometry = self.session.node(shader, "geometry")?;
        let normal = (geometry, "Normal");
        let split = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, geometry, "Normal", split, "Vector")?;
        let across = self.float_op(shader, "absolute", Slot::Link(split, "X"), zero)?;
        let use_x = self.float_op(shader, "less_than", across, Slot::Value(Value::Float(0.9)))?;
        let helper = self.session.node(shader, "combine_xyz")?;
        self.feed(shader, helper, "X", use_x, false)?;
        let use_y = self.float_op(shader, "subtract", one, use_x)?;
        self.feed(shader, helper, "Y", use_y, false)?;
        let tangent = self.vector_op(shader, "cross_product", normal, Some((helper, "Vector")), None)?;
        let tangent = self.vector_op(shader, "normalize", tangent, None, None)?;
        let bitangent = self.vector_op(shader, "cross_product", normal, Some(tangent), None)?;
        let along_t = self.vector_op(shader, "scale", tangent, None, Some(x))?;
        let along_b = self.vector_op(shader, "scale", bitangent, None, Some(y))?;
        let along_n = self.vector_op(shader, "scale", normal, None, Some(cos_t))?;
        let lean = self.vector_op(shader, "add", along_t, Some(along_b), None)?;
        let lean = self.vector_op(shader, "add", lean, Some(along_n), None)?;
        // The flake: aluminium, in the flake colour.
        let (f0, f82) = conductor_colours(&tequila_materials::fresnel::metals::ALUMINIUM);
        let metal = self.session.node(shader, "metallic_bsdf")?;
        self.session.set_string(metal, "fresnel_type", "f82")?;
        self.session.set_float3(metal, "Base Color", rgb(f0 * m.flake_color))?;
        self.session.set_float3(metal, "Edge Tint", rgb(f82))?;
        self.session.set_float(metal, "Roughness", m.flake_roughness)?;
        self.session.connect(shader, lean.0, lean.1, metal, "Normal")?;
        let mut flake = (metal, "BSDF");
        if m.coat_weight > 1e-4 {
            let gloss = self.session.node(shader, "glossy_bsdf")?;
            self.session.set_string(gloss, "distribution", "ggx")?;
            self.session.set_float3(gloss, "Color", rgb(m.coat_color))?;
            self.session.set_float(gloss, "Roughness", m.coat_roughness)?;
            let fresnel = self.session.node(shader, "fresnel")?;
            self.session.set_float(fresnel, "IOR", m.coat_ior)?;
            let share = self.float_op(shader, "multiply", Slot::Link(fresnel, "Fac"), Slot::Value(Value::Float(m.coat_weight)))?;
            let coated = self.session.node(shader, "mix_closure")?;
            self.feed(shader, coated, "Fac", share, false)?;
            self.session.connect(shader, metal, "BSDF", coated, "Closure1")?;
            self.session.connect(shader, gloss, "BSDF", coated, "Closure2")?;
            flake = (coated, "Closure");
        }
        let laid = self.session.node(shader, "mix_closure")?;
        self.feed(shader, laid, "Fac", present, false)?;
        self.session.connect(shader, body.0, body.1, laid, "Closure1")?;
        self.session.connect(shader, flake.0, flake.1, laid, "Closure2")?;
        Ok((laid, "Closure"))
    }

    /// The way an anisotropic lobe is rougher, as Tequila's renderer draws
    /// it (`anisotropic_frame`). Along the mesh's UVs where it has them,
    /// which Cycles gives only when asked: unwired, Principled's tangent is
    /// radial about the object's Z, and every brushed or woven highlight ran
    /// round a pole instead of along the part. Without UVs, the frame the
    /// normal alone gives (`Onb::from_normal`, in Tequila's world): Cycles'
    /// UV-map tangent is zero there, and Principled made a NaN of it - every
    /// carbon or brushed part on a CAD import rendered black. One shader
    /// serves parts with UVs and parts without, so the choice is made where
    /// it shades. `normal` is the bumped one, if any: Tequila's frame is
    /// drawn about the shading normal. `drawn` is the graph's own direction
    /// (`Param::Tangent`), in the part's space, which comes first wherever
    /// it is not zero.
    fn anisotropy_tangent(&mut self, shader: Shader, normal: Option<(Node, &'static str)>, drawn: Option<Slot>) -> Result<(Node, &'static str), String> {
        let surface = self.surface_tangent(shader, normal)?;
        let Some(drawn) = drawn else {
            return Ok(surface);
        };
        let vector = match drawn {
            Slot::Link(node, socket) => (node, socket),
            Slot::Value(v) => {
                let p = v.as_vector();
                let constant = self.session.node(shader, "combine_xyz")?;
                self.session.set_float(constant, "X", p.x)?;
                self.session.set_float(constant, "Y", p.y)?;
                self.session.set_float(constant, "Z", p.z)?;
                (constant, "Vector")
            }
        };
        // From the part's space to the world - Cycles' object transform
        // carries the turn to Z up as well.
        let world = self.session.node(shader, "vector_transform")?;
        self.session.set_string(world, "transform_type", "vector")?;
        self.session.set_string(world, "convert_from", "object")?;
        self.session.set_string(world, "convert_to", "world")?;
        self.session.connect(shader, vector.0, vector.1, world, "Vector")?;
        let length = self.vector_op(shader, "length", (world, "Vector"), None, None)?;
        let present = self.float_op(shader, "greater_than", Slot::Link(length.0, length.1), Slot::Value(Value::Float(1e-6)))?;
        let absent = self.float_op(shader, "subtract", Slot::Value(Value::Float(1.0)), present)?;
        let kept = self.vector_op(shader, "scale", (world, "Vector"), None, Some(present))?;
        let otherwise = self.vector_op(shader, "scale", surface, None, Some(absent))?;
        self.vector_op(shader, "add", kept, Some(otherwise), None)
    }

    /// The surface's own tangent, for [`Self::anisotropy_tangent`]: along
    /// the UVs, else the frame the normal gives.
    fn surface_tangent(&mut self, shader: Shader, normal: Option<(Node, &'static str)>) -> Result<(Node, &'static str), String> {
        let along_uvs = self.session.node(shader, "tangent")?;
        self.session.set_string(along_uvs, "direction_type", "uv_map")?;
        self.session.set_string(along_uvs, "attribute", "UVMap")?;
        let normal = match normal {
            Some(normal) => normal,
            None => (self.session.node(shader, "geometry")?, "Normal"),
        };
        // Frisvad's frame about Tequila's normal (x, z, -y), turned back
        // into Cycles' world: (1 + s x x a, s x, s x z a), s being the sign
        // of Tequila's z - which is -y here - and a = -1 / (s - y).
        let split = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, normal.0, normal.1, split, "Vector")?;
        let (x, y, z) = (Slot::Link(split, "X"), Slot::Link(split, "Y"), Slot::Link(split, "Z"));
        let one = Slot::Value(Value::Float(1.0));
        let below = self.float_op(shader, "greater_than", y, Slot::Value(Value::Float(0.0)))?;
        let s = self.float_op3(shader, "multiply_add", below, Slot::Value(Value::Float(-2.0)), one)?;
        let across = self.float_op(shader, "subtract", s, y)?;
        let a = self.float_op(shader, "divide", Slot::Value(Value::Float(-1.0)), across)?;
        let sx = self.float_op(shader, "multiply", s, x)?;
        let sxa = self.float_op(shader, "multiply", sx, a)?;
        let fallback = self.session.node(shader, "combine_xyz")?;
        let tx = self.float_op3(shader, "multiply_add", sxa, x, one)?;
        self.feed(shader, fallback, "X", tx, false)?;
        self.feed(shader, fallback, "Y", sx, false)?;
        let tz = self.float_op(shader, "multiply", sxa, z)?;
        self.feed(shader, fallback, "Z", tz, false)?;
        // The UV tangent is unit length, or zero where there are no UVs.
        let length = self.vector_op(shader, "length", (along_uvs, "Tangent"), None, None)?;
        let missing = self.float_op(shader, "less_than", Slot::Link(length.0, length.1), Slot::Value(Value::Float(0.5)))?;
        let filled = self.vector_op(shader, "scale", (fallback, "Vector"), None, Some(missing))?;
        self.vector_op(shader, "add", (along_uvs, "Tangent"), Some(filled), None)
    }

    /// `direction` laid flat on the surface whose normal is `normal`, and
    /// made unit length.
    fn flat_along(&mut self, shader: Shader, normal: (Node, &'static str), direction: Vec3) -> Result<(Node, &'static str), String> {
        let constant = self.session.node(shader, "combine_xyz")?;
        self.session.set_float(constant, "X", direction.x)?;
        self.session.set_float(constant, "Y", direction.y)?;
        self.session.set_float(constant, "Z", direction.z)?;
        let along = (constant, "Vector");
        let dot = self.vector_op(shader, "dot_product", normal, Some(along), None)?;
        let off = self.vector_op(shader, "scale", normal, None, Some(Slot::Link(dot.0, dot.1)))?;
        let flat = self.vector_op(shader, "subtract", along, Some(off), None)?;
        self.vector_op(shader, "normalize", flat, None, None)
    }

    /// `a op b` with Cycles' math node, folded when both are numbers.
    fn float_op(&mut self, shader: Shader, op: &str, a: Slot, b: Slot) -> Result<Slot, String> {
        self.float_op3(shader, op, a, b, Slot::Value(Value::Float(0.0)))
    }

    fn float_op3(&mut self, shader: Shader, op: &str, a: Slot, b: Slot, c: Slot) -> Result<Slot, String> {
        if let (Slot::Value(a), Slot::Value(b), Slot::Value(c)) = (a, b, c) {
            let (a, b, c) = (a.as_float(), b.as_float(), c.as_float());
            let folded = match op {
                "add" => Some(a + b),
                "subtract" => Some(a - b),
                "multiply" => Some(a * b),
                "divide" => Some(if b != 0.0 { a / b } else { 0.0 }),
                "multiply_add" => Some(a * b + c),
                "minimum" => Some(a.min(b)),
                "maximum" => Some(a.max(b)),
                _ => None,
            };
            if let Some(value) = folded {
                return Ok(Slot::Value(Value::Float(value)));
            }
        }
        let node = self.session.node(shader, "math")?;
        self.session.set_string(node, "math_type", op)?;
        self.feed(shader, node, "Value1", a, false)?;
        self.feed(shader, node, "Value2", b, false)?;
        self.feed(shader, node, "Value3", c, false)?;
        Ok(Slot::Link(node, "Value"))
    }

    /// Tequila's anisotropy as Cycles' principled shader takes it. Tequila
    /// stretches the lobe by an aspect of (1 - a^2)^(1/4)
    /// (`Ggx::from_roughness`), Cycles by sqrt(1 - 0.9 a); the same aspect is
    /// a Cycles anisotropy of (1 - sqrt(1 - a^2)) / 0.9. Passed through as it
    /// was, a brushed metal's 0.7 stretched its highlight nearly twice as far
    /// in Cycles as in Tequila's renderer.
    fn cycles_anisotropy(&mut self, shader: Shader, a: Slot) -> Result<Slot, String> {
        let convert = |a: f32| {
            let a = a.clamp(0.0, 0.99);
            (1.0 - (1.0 - a * a).sqrt()) / 0.9
        };
        if let Slot::Value(v) = a {
            return Ok(Slot::Value(Value::Float(convert(v.as_float()))));
        }
        let clamped = self.float_op(shader, "minimum", a, Slot::Value(Value::Float(0.99)))?;
        let clamped = self.float_op(shader, "maximum", clamped, Slot::Value(Value::Float(0.0)))?;
        let square = self.float_op(shader, "multiply", clamped, clamped)?;
        let rest = self.float_op(shader, "subtract", Slot::Value(Value::Float(1.0)), square)?;
        let root = self.float_op(shader, "sqrt", rest, Slot::Value(Value::Float(0.0)))?;
        let short = self.float_op(shader, "subtract", Slot::Value(Value::Float(1.0)), root)?;
        self.float_op(shader, "divide", short, Slot::Value(Value::Float(0.9)))
    }

    /// `a` towards `b` by `t`, for numbers.
    fn float_mix(&mut self, shader: Shader, a: Slot, b: Slot, t: Slot) -> Result<Slot, String> {
        let difference = self.float_op(shader, "subtract", b, a)?;
        self.float_op3(shader, "multiply_add", difference, t, a)
    }

    /// Feed `slot` to a node input of the given kind.
    fn feed(&mut self, shader: Shader, node: Node, socket: &str, slot: Slot, colour: bool) -> Result<(), String> {
        match slot {
            Slot::Link(from, output) => self.session.connect(shader, from, output, node, socket),
            Slot::Value(value) if colour => self.session.set_float3(node, socket, rgb(value.as_color())),
            Slot::Value(value) => self.session.set_float(node, socket, value.as_float()),
        }
    }

    /// Run a material program symbolically: every register becomes a literal
    /// or a Cycles node output.
    fn program(&mut self, shader: Shader, program: &CompiledGraph) -> Result<HashMap<u16, Slot>, String> {
        let mut registers: HashMap<u16, Slot> = HashMap::new();
        self.nodes.clear();
        self.points.clear();
        self.turned.clear();
        let get = |registers: &HashMap<u16, Slot>, r: &Register| {
            registers.get(&r.0).copied().unwrap_or(Slot::Value(Value::Float(0.0)))
        };
        for instruction in &program.instructions {
            let (dst, slot) = match instruction {
                Instruction::Load { dst, value } => (dst, Slot::Value(*value)),
                Instruction::Sample { dst, texture, mapping, fallback } => {
                    let slot = match self.texture_node(shader, *texture, mapping)? {
                        Some(sampled) => sampled.colour(),
                        None => Slot::Value(*fallback),
                    };
                    (dst, slot)
                }
                Instruction::SampleAlpha { dst, texture, mapping, fallback } => {
                    let slot = match self.texture_node(shader, *texture, mapping)? {
                        Some(sampled) => sampled.alpha,
                        None => Slot::Value(*fallback),
                    };
                    (dst, slot)
                }
                Instruction::Apply { dst, kind, args, output } => {
                    let inputs: Vec<Slot> = args.iter().map(|a| get(&registers, a)).collect();
                    (dst, self.apply(shader, kind, &inputs, *output)?)
                }
                Instruction::Input { dst, input } => (dst, self.point_input(shader, *input)?),
                Instruction::VertexColor { dst } => (dst, self.vertex_colour(shader)?),
                Instruction::Evaluate { dst, procedural, seed, mapping } => {
                    let slot = self.procedural(shader, procedural, *seed, mapping.as_deref())?;
                    (dst, slot.unwrap_or(Slot::Value(Value::Float(0.5))))
                }
                other => {
                    self.out.warn(format!(
                        "material node {} is not translated",
                        format!("{other:?}").split([' ', '{', '(']).next().unwrap_or("?")
                    ));
                    (&other.destination(), Slot::Value(Value::Float(0.5)))
                }
            };
            registers.insert(dst.0, slot);
        }
        Ok(registers)
    }

    fn apply(&mut self, shader: Shader, kind: &NodeKind, inputs: &[Slot], output: u8) -> Result<Slot, String> {
        let input = |i: usize| inputs.get(i).copied().unwrap_or(Slot::Value(Value::Float(0.0)));
        // Constant folding happened at compile time, but a fold of all-literal
        // inputs costs nothing to repeat and keeps the Cycles graph small.
        if inputs.iter().all(|s| matches!(s, Slot::Value(_))) {
            let values: Vec<Value> = inputs.iter().map(|s| match s { Slot::Value(v) => *v, _ => unreachable!() }).collect();
            return Ok(Slot::Value(kind.evaluate_output(&values, output)));
        }
        if let NodeKind::Node(node) = kind {
            return self.shader_node(shader, node, inputs, output);
        }
        let mix = |builder: &mut Self, mode: &str, fac: Slot, a: Slot, b: Slot| -> Result<Slot, String> {
            let node = builder.session.node(shader, "mix")?;
            builder.session.set_string(node, "mix_type", mode)?;
            builder.feed(shader, node, "Fac", fac, false)?;
            builder.feed(shader, node, "Color1", a, true)?;
            builder.feed(shader, node, "Color2", b, true)?;
            Ok(Slot::Link(node, "Color"))
        };
        match kind {
            NodeKind::Mix => mix(self, "mix", input(2), input(0), input(1)),
            NodeKind::Multiply => mix(self, "multiply", Slot::Value(Value::Float(1.0)), input(0), input(1)),
            NodeKind::Add => mix(self, "add", Slot::Value(Value::Float(1.0)), input(0), input(1)),
            NodeKind::Gradient { gradient } => self.gradient(shader, gradient, input(0)),
            // A 0-to-1 height made `depth` scene units deep; a colour read as
            // its luminance, as Cycles converts one to a number.
            NodeKind::Bump { depth } => self.float_op(shader, "multiply", input(0), Slot::Value(Value::Float(*depth))),
            NodeKind::Remap { from, to } => {
                // Per channel, to.x + (c - from.x) / span * (to.y - to.x): a
                // point mapping's scale and move, on the colour as a vector.
                let span = (from.y - from.x).abs().max(1e-8) * (from.y - from.x).signum();
                let k = (to.y - to.x) / span;
                let node = self.session.node(shader, "mapping")?;
                self.session.set_string(node, "mapping_type", "point")?;
                self.session.set_float3(node, "scale", [k; 3])?;
                self.session.set_float3(node, "location", [to.x - from.x * k; 3])?;
                self.feed(shader, node, "Vector", input(0), true)?;
                Ok(Slot::Link(node, "Vector"))
            }
            NodeKind::Clamp { low, high } => {
                let below = self.session.node(shader, "vector_math")?;
                self.session.set_string(below, "math_type", "minimum")?;
                self.feed(shader, below, "Vector1", input(0), true)?;
                self.session.set_float3(below, "Vector2", [*high; 3])?;
                let above = self.session.node(shader, "vector_math")?;
                self.session.set_string(above, "math_type", "maximum")?;
                self.session.connect(shader, below, "Vector", above, "Vector1")?;
                self.session.set_float3(above, "Vector2", [*low; 3])?;
                Ok(Slot::Link(above, "Vector"))
            }
            NodeKind::Invert => {
                let node = self.session.node(shader, "invert")?;
                self.feed(shader, node, "Color", input(0), true)?;
                Ok(Slot::Link(node, "Color"))
            }
            NodeKind::Channel { channel } => {
                let node = self.session.node(shader, "separate_color")?;
                self.feed(shader, node, "Color", input(0), true)?;
                let output = match channel {
                    tequila_materials::graph::Channel::Red => "Red",
                    tequila_materials::graph::Channel::Green => "Green",
                    tequila_materials::graph::Channel::Blue => "Blue",
                };
                Ok(Slot::Link(node, output))
            }
            other => {
                self.out.warn(format!("material node {} is not translated", other.label()));
                Ok(input(0))
            }
        }
    }

    /// A Blender node, as the same Cycles node: its settings set, its
    /// inputs fed or linked, read at output `output`.
    fn shader_node(&mut self, shader: Shader, node: &ShaderNode, inputs: &[Slot], output: u8) -> Result<Slot, String> {
        let outputs = node.outputs();
        let Some(out) = outputs.get(output as usize) else {
            return Ok(Slot::Value(Value::Float(0.0)));
        };
        let key = format!("{node:?} {inputs:?}");
        let cycles = match self.nodes.get(&key) {
            Some(cycles) => *cycles,
            None => {
                let cycles = self.session.node(shader, node.cycles_type())?;
                for (socket, setting) in node.cycles_settings() {
                    match setting {
                        CyclesSetting::Enum(v) => self.session.set_string(cycles, socket, v)?,
                        CyclesSetting::Int(v) => self.session.set_int(cycles, socket, v)?,
                        CyclesSetting::Float(v) => self.session.set_float(cycles, socket, v)?,
                        CyclesSetting::Bool(v) => self.session.set_bool(cycles, socket, v)?,
                    }
                }
                for (index, spec) in node.inputs().iter().enumerate() {
                    // What Cycles reads of the surface itself.
                    if spec.hidden {
                        continue;
                    }
                    let slot = inputs.get(index).copied().unwrap_or(Slot::Value(spec.default));
                    match spec.ty {
                        // A normal Cycles reads in its own world: turned back
                        // into it, or left for Cycles' own when it is that.
                        tequila_materials::graph::SocketType::Normal => {
                            if let Some(normal) = self.to_cycles_world(shader, slot)? {
                                self.session.connect(shader, normal.0, normal.1, cycles, spec.cycles)?;
                            }
                        }
                        tequila_materials::graph::SocketType::Float => self.feed(shader, cycles, spec.cycles, slot, false)?,
                        tequila_materials::graph::SocketType::Bool => match slot {
                            Slot::Value(v) => self.session.set_bool(cycles, spec.cycles, v.as_float() > 0.5)?,
                            Slot::Link(from, name) => self.session.connect(shader, from, name, cycles, spec.cycles)?,
                        },
                        tequila_materials::graph::SocketType::Color => self.feed(shader, cycles, spec.cycles, slot, true)?,
                        tequila_materials::graph::SocketType::Vector => match slot {
                            Slot::Value(v) => {
                                let p = v.as_vector();
                                self.session.set_float3(cycles, spec.cycles, [p.x, p.y, p.z])?
                            }
                            Slot::Link(from, name) => self.session.connect(shader, from, name, cycles, spec.cycles)?,
                        },
                    }
                }
                self.nodes.insert(key, cycles);
                cycles
            }
        };
        Ok(Slot::Link(cycles, out.cycles))
    }

    /// The colours painted on the vertices, white where a mesh has none, as
    /// Tequila's renderer reads them. Cycles reads a missing attribute as
    /// transparent black, so its alpha chooses between it and white; a
    /// library built before vertex colours gives white everywhere.
    fn vertex_colour(&mut self, shader: Shader) -> Result<Slot, String> {
        if !self.session.has_vertex_colours() {
            self.out.warn("this Cycles library predates vertex colours; they render white");
            return Ok(Slot::Value(Value::Color(tequila_core::spectrum::Spectrum::ONE)));
        }
        let attribute = self.session.node(shader, "attribute")?;
        self.session.set_string(attribute, "attribute", "tq_color")?;
        let mix = self.session.node(shader, "mix")?;
        self.session.set_string(mix, "mix_type", "mix")?;
        self.session.connect(shader, attribute, "Alpha", mix, "Fac")?;
        self.feed(shader, mix, "Color1", Slot::Value(Value::Color(tequila_core::spectrum::Spectrum::ONE)), true)?;
        self.session.connect(shader, attribute, "Color", mix, "Color2")?;
        Ok(Slot::Link(mix, "Color"))
    }

    /// What the shading point says to a node that reads it, as Tequila's
    /// renderer has it (`shade::point_input`): world vectors turned to
    /// Tequila's Y up, object space in scene units, v up.
    fn point_input(&mut self, shader: Shader, input: PointInput) -> Result<Slot, String> {
        if let Some(slot) = self.points.get(&input) {
            return Ok(*slot);
        }
        let read = |builder: &mut Self, kind: &str, output: &'static str| -> Result<(Node, &'static str), String> {
            Ok((builder.session.node(shader, kind)?, output))
        };
        let slot = match input {
            PointInput::Object => {
                let (coords, _) = read(self, "texture_coordinate", "Object")?;
                let (info, _) = read(self, "object_info", "Color")?;
                let scaled = self.vector_op(shader, "multiply", (coords, "Object"), Some((info, "Color")), None)?;
                Slot::Link(scaled.0, scaled.1)
            }
            PointInput::Generated => {
                let (n, o) = read(self, "texture_coordinate", "Generated")?;
                Slot::Link(n, o)
            }
            PointInput::Uv => {
                let (n, o) = read(self, "texture_coordinate", "UV")?;
                Slot::Link(n, o)
            }
            PointInput::ObjectNormal => {
                let (n, o) = read(self, "texture_coordinate", "Normal")?;
                Slot::Link(n, o)
            }
            PointInput::Reflection => {
                let from = read(self, "texture_coordinate", "Reflection")?;
                self.to_tequila_world(shader, from)?
            }
            PointInput::Position => {
                let from = read(self, "geometry", "Position")?;
                self.to_tequila_world(shader, from)?
            }
            PointInput::Normal => {
                let from = read(self, "geometry", "Normal")?;
                self.to_tequila_world(shader, from)?
            }
            PointInput::TrueNormal => {
                let from = read(self, "geometry", "True Normal")?;
                self.to_tequila_world(shader, from)?
            }
            PointInput::Incoming => {
                let from = read(self, "geometry", "Incoming")?;
                self.to_tequila_world(shader, from)?
            }
            PointInput::Backfacing => {
                let (n, o) = read(self, "geometry", "Backfacing")?;
                Slot::Link(n, o)
            }
            PointInput::ObjectLocation => {
                let from = read(self, "object_info", "Location")?;
                self.to_tequila_world(shader, from)?
            }
            PointInput::ObjectRandom => {
                let (n, o) = read(self, "object_info", "Random")?;
                Slot::Link(n, o)
            }
        };
        self.points.insert(input, slot);
        Ok(slot)
    }

    /// A Cycles world vector in Tequila's world: (x, z, -y), the inverse
    /// of [`Y_UP_TO_Z_UP`], exactly - a swap and a sign.
    fn to_tequila_world(&mut self, shader: Shader, from: (Node, &'static str)) -> Result<Slot, String> {
        let split = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, from.0, from.1, split, "Vector")?;
        let down = self.float_op(shader, "multiply", Slot::Link(split, "Y"), Slot::Value(Value::Float(-1.0)))?;
        let joined = self.session.node(shader, "combine_xyz")?;
        self.session.connect(shader, split, "X", joined, "X")?;
        self.session.connect(shader, split, "Z", joined, "Y")?;
        self.feed(shader, joined, "Z", down, false)?;
        self.turned.insert(joined, from);
        Ok(Slot::Link(joined, "Vector"))
    }

    /// A Tequila world vector in Cycles' world, for a node input that
    /// reads one there; `None` for the shading normal itself, which Cycles
    /// reads of its own when nothing is linked.
    fn to_cycles_world(&mut self, shader: Shader, slot: Slot) -> Result<Option<(Node, &'static str)>, String> {
        if let Slot::Link(node, _) = slot {
            if let Some(from) = self.turned.get(&node).copied() {
                let normal = self.points.get(&PointInput::Normal).copied();
                if matches!(normal, Some(Slot::Link(n, _)) if n == node) {
                    return Ok(None);
                }
                return Ok(Some(from));
            }
        }
        // (x, -z, y), as `Y_UP_TO_Z_UP` turns a direction.
        let joined = self.session.node(shader, "combine_xyz")?;
        match slot {
            Slot::Value(v) => {
                let p = v.as_vector();
                self.session.set_float(joined, "X", p.x)?;
                self.session.set_float(joined, "Y", -p.z)?;
                self.session.set_float(joined, "Z", p.y)?;
            }
            Slot::Link(from, name) => {
                let split = self.session.node(shader, "separate_xyz")?;
                self.session.connect(shader, from, name, split, "Vector")?;
                let back = self.float_op(shader, "multiply", Slot::Link(split, "Z"), Slot::Value(Value::Float(-1.0)))?;
                self.session.connect(shader, split, "X", joined, "X")?;
                self.feed(shader, joined, "Y", back, false)?;
                self.session.connect(shader, split, "Y", joined, "Z")?;
            }
        }
        Ok(Some((joined, "Vector")))
    }

    /// A colour ramp, as `Gradient::evaluate` reads one: from the input's
    /// first channel, linear between stops, held flat outside them, and a
    /// hard edge where two stops share a place. Each stop past the first is a
    /// mix towards it, all the way once the input has passed it.
    fn gradient(&mut self, shader: Shader, gradient: &tequila_texture::gradient::Gradient, input: Slot) -> Result<Slot, String> {
        let stops = gradient.stops();
        match stops {
            [] => return Ok(Slot::Value(Value::Color(Spectrum::splat(0.5)))),
            [only] => return Ok(Slot::Value(Value::Color(only.color))),
            _ => {}
        }
        let t = match input {
            Slot::Link(..) => {
                let split = self.session.node(shader, "separate_color")?;
                self.feed(shader, split, "Color", input, true)?;
                Slot::Link(split, "Red")
            }
            value => value,
        };
        use tequila_texture::gradient::Interpolation;
        let mut colour = Slot::Value(Value::Color(stops[0].color));
        for pair in stops.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let span = b.at - a.at;
            let share = if span <= 0.0 || gradient.interpolation() == Interpolation::Constant {
                self.float_op(shader, "greater_than", t, Slot::Value(Value::Float(b.at)))?
            } else if gradient.interpolation() == Interpolation::Ease {
                // Smoothstep between the two stops, clamped: Map Range's own.
                let fit = self.session.node(shader, "map_range")?;
                self.session.set_string(fit, "range_type", "smoothstep")?;
                self.feed(shader, fit, "value", t, false)?;
                self.session.set_float(fit, "from_min", a.at)?;
                self.session.set_float(fit, "from_max", b.at)?;
                Slot::Link(fit, "Result")
            } else {
                self.clamped(shader, "multiply_add", t, Slot::Value(Value::Float(1.0 / span)), Slot::Value(Value::Float(-a.at / span)))?
            };
            let node = self.session.node(shader, "mix")?;
            self.session.set_string(node, "mix_type", "mix")?;
            self.feed(shader, node, "Fac", share, false)?;
            self.feed(shader, node, "Color1", colour, true)?;
            self.feed(shader, node, "Color2", Slot::Value(Value::Color(b.color)), true)?;
            colour = Slot::Link(node, "Color");
        }
        Ok(colour)
    }

    /// A math node's result clamped to 0 to 1.
    fn clamped(&mut self, shader: Shader, op: &str, a: Slot, b: Slot, c: Slot) -> Result<Slot, String> {
        match self.float_op3(shader, op, a, b, c)? {
            Slot::Value(v) => Ok(Slot::Value(Value::Float(v.as_float().clamp(0.0, 1.0)))),
            Slot::Link(node, socket) => {
                self.session.set_bool(node, "use_clamp", true)?;
                Ok(Slot::Link(node, socket))
            }
        }
    }

    /// A procedural pattern, as Tequila's renderer evaluates it
    /// (`Procedural::evaluate`), or `None` for one not translated.
    ///
    /// Cycles' gradient noise is Tequila's - the same lattice, the same
    /// twelve gradients, the same quintic - scaled to another range, so a
    /// pattern built from it has Tequila's contrast and grain, if not its
    /// very features: the hashes that place them differ, and the seed here
    /// moves the pattern rather than reshuffling it.
    fn procedural(
        &mut self,
        shader: Shader,
        procedural: &tequila_texture::procedural::Procedural,
        seed: u32,
        mapping: Option<&Mapping>,
    ) -> Result<Option<Slot>, String> {
        use tequila_texture::noise::{Fractal, Metric};
        use tequila_texture::procedural::{CellOutput, Procedural};
        let octaves = |n: u32| Fractal { octaves: n, ..Fractal::default() };
        if procedural.is_planar() {
            // On the mapping's (u, v), or flattened onto x-z in object space.
            let position = self.procedural_position(shader, mapping)?;
            let split = self.split(shader, position)?;
            let mapped = mapping.is_some_and(|m| m.projection != TextureProjection::TriPlanar);
            let plane = (Slot::Link(split, "X"), Slot::Link(split, if mapped { "Y" } else { "Z" }));
            return Ok(Some(match *procedural {
                Procedural::Scratches { frequency, width, density, spread } => {
                    self.scratches(shader, plane, seed, frequency, width, density, spread)?
                }
                Procedural::Weave { pattern, frequency, width, variance } => self.weave(shader, plane, seed, pattern, frequency, width, variance)?,
                Procedural::Mesh { shape, frequency, size, softness } => self.perforated(shader, plane, seed, shape, frequency, size, softness)?,
                _ => unreachable!("only flat patterns are planar"),
            }));
        }
        let position = self.procedural_position(shader, mapping)?;
        let zero = Slot::Value(Value::Float(0.0));
        let one_octave = |fractal: Fractal| Fractal { octaves: 1, ..fractal };
        let slot = match *procedural {
            Procedural::Noise { fractal, frequency } => self.noise(shader, position, [frequency.max(1e-4); 3], seed, fractal)?,
            Procedural::NoiseTexture { frequency, contrast, bias, .. } => {
                let n = self.noise(shader, position, [frequency.max(1e-4); 3], seed, one_octave(Fractal::default()))?;
                let c = contrast.max(0.0);
                self.clamped(shader, "multiply_add", n, Slot::Value(Value::Float(c)), Slot::Value(Value::Float(0.5 + bias - 0.5 * c)))?
            }
            Procedural::Brushed { frequency, stretch } => {
                let f = frequency.max(1e-4);
                self.noise(shader, position, [f, f / stretch.max(1e-3), f], seed, one_octave(Fractal::default()))?
            }
            Procedural::Cellular { metric, output, frequency } => {
                let metric = match metric {
                    Metric::Euclidean => "euclidean",
                    Metric::Manhattan => "manhattan",
                    Metric::Chebyshev => "chebychev",
                };
                let placed = self.noise_space(shader, position, [frequency.max(1e-4); 3], seed)?;
                let near = self.voronoi(shader, placed, metric, "f1")?;
                match output {
                    CellOutput::Distance => self.clamped(shader, "add", Slot::Link(near, "Distance"), zero, zero)?,
                    CellOutput::Edges => {
                        let far = self.voronoi(shader, placed, metric, "f2")?;
                        self.clamped(shader, "subtract", Slot::Link(far, "Distance"), Slot::Link(near, "Distance"), zero)?
                    }
                    CellOutput::Id => self.red(shader, (near, "Color"))?,
                }
            }
            Procedural::Ridged { fractal, frequency } => {
                // Per octave (1 - |noise|) squared, each weighted by the one
                // before, which is what keeps ridges on ridges.
                let count = fractal.octaves.max(1);
                let total: f32 = (0..count).map(|i| fractal.gain.powi(i as i32)).sum();
                let (mut sum, mut weight, mut amplitude, mut f) = (num(0.0), num(1.0), 1.0, frequency.max(1e-4));
                for octave in 0..count {
                    let n = self.noise(shader, position, [f; 3], seed.wrapping_add(octave * 0x9e37), one_octave(fractal))?;
                    let signed = self.madd(shader, n, num(2.0), num(-1.0))?;
                    let size = self.unary(shader, "absolute", signed)?;
                    let crest = self.sub(shader, num(1.0), size)?;
                    let ridge = self.mul(shader, crest, crest)?;
                    let ridge = self.mul(shader, ridge, weight)?;
                    weight = self.clamped(shader, "multiply", ridge, num(2.0), zero)?;
                    sum = self.madd(shader, ridge, num(amplitude / total), sum)?;
                    amplitude *= fractal.gain;
                    f *= fractal.lacunarity;
                }
                self.clamped(shader, "add", sum, zero, zero)?
            }
            Procedural::Granite { frequency, speckle, fine } => {
                // Crystals at two sizes, a fine grain, and flecks of white
                // and black.
                let f = frequency.max(1e-4);
                let large = self.cell_id(shader, position, f, seed)?;
                let small = self.cell_id(shader, position, f * 2.7, seed ^ 0x3c6e_f372)?;
                let grain = self.noise(shader, position, [f * 9.0; 3], seed ^ 0x2545_f491, octaves(2))?;
                let fine = fine.clamp(0.0, 1.0);
                let crystals = self.mul(shader, large, num(0.55 * (1.0 - fine)))?;
                let crystals = self.madd(shader, small, num(0.45 * (1.0 - fine)), crystals)?;
                let mut v = self.madd(shader, grain, num(fine), crystals)?;
                let speckle = speckle.clamp(0.0, 1.0);
                if speckle > 0.0 {
                    let placed = self.noise_space(shader, position, [f * 5.3; 3], seed ^ 0x9e37_79b9)?;
                    let fleck = self.voronoi(shader, placed, "euclidean", "f1")?;
                    let id = self.red(shader, (fleck, "Color"))?;
                    let near = self.float_op(shader, "less_than", Slot::Link(fleck, "Distance"), num(0.3))?;
                    let white = self.float_op(shader, "less_than", id, num(speckle * 0.5))?;
                    let white = self.mul(shader, white, near)?;
                    let black = self.float_op(shader, "greater_than", id, num(1.0 - speckle * 0.5))?;
                    let black = self.mul(shader, black, near)?;
                    let headroom = self.sub(shader, num(1.0), v)?;
                    v = self.madd(shader, headroom, white, v)?;
                    let kept = self.sub(shader, num(1.0), black)?;
                    v = self.mul(shader, v, kept)?;
                }
                self.clamped(shader, "add", v, zero, zero)?
            }
            Procedural::Leather { frequency, crack_width, wrinkle } => {
                // Pebbles, cracked apart along the cell edges, and fine
                // wrinkles.
                let f = frequency.max(1e-4);
                let placed = self.noise_space(shader, position, [f; 3], seed)?;
                let near = self.voronoi(shader, placed, "euclidean", "f1")?;
                let pebble = self.smoothstep(shader, num(0.0), num(1.0), Slot::Link(near, "Distance"))?;
                let pebble = self.sub(shader, num(1.0), pebble)?;
                let creases = self.noise(shader, position, [f * 6.0; 3], seed ^ 0x6a09_e667, octaves(3))?;
                let w = wrinkle.clamp(0.0, 1.0);
                let skin = self.mul(shader, pebble, num(1.0 - w))?;
                let skin = self.madd(shader, creases, num(w), skin)?;
                if crack_width > 0.0 {
                    let far = self.voronoi(shader, placed, "euclidean", "f2")?;
                    let gap = self.sub(shader, Slot::Link(far, "Distance"), Slot::Link(near, "Distance"))?;
                    let crack = self.smoothstep(shader, num(0.0), num(crack_width), gap)?;
                    self.clamped(shader, "multiply", skin, crack, zero)?
                } else {
                    self.clamped(shader, "add", skin, zero, zero)?
                }
            }
            Procedural::Spots { frequency, coverage, variance, softness } => {
                // A disc in every cell, some smaller than others.
                let placed = self.noise_space(shader, position, [frequency.max(1e-4); 3], seed)?;
                let near = self.voronoi(shader, placed, "euclidean", "f1")?;
                let id = self.red(shader, (near, "Color"))?;
                let full = coverage.clamp(0.0, 1.0) * 0.8;
                let radius = self.madd(shader, id, num(-full * variance.clamp(0.0, 1.0)), num(full))?;
                let soft = self.madd(shader, radius, num(softness.clamp(0.0, 1.0) * 0.5), num(1e-4))?;
                let from = self.sub(shader, radius, soft)?;
                let to = self.add(shader, radius, soft)?;
                let edge = self.smoothstep(shader, from, to, Slot::Link(near, "Distance"))?;
                self.sub(shader, num(1.0), edge)?
            }
            Procedural::Camouflage { frequency, layers } => {
                // Blotch layers stacked, the topmost showing wherever its
                // noise is over the line.
                let layers = layers.clamp(2, 5);
                let mut top = num(0.0);
                for layer in 0..layers {
                    let scale = frequency.max(1e-4) * (1.0 + layer as f32 * 0.7);
                    let mut at = seed_offset(seed.wrapping_add(layer * 0x2f6b));
                    at[0] += layer as f32 * 17.3;
                    at[2] += layer as f32 * 9.1;
                    let n = self.noise_at(shader, position, [scale; 3], at, octaves(3))?;
                    // fBm over 0.05 is the unit value over 0.525.
                    let over = self.float_op(shader, "greater_than", n, num(0.525))?;
                    let rise = self.sub(shader, num((layer + 1) as f32), top)?;
                    top = self.madd(shader, rise, over, top)?;
                }
                self.mul(shader, top, num(1.0 / layers as f32))?
            }
            Procedural::Contour { axis, frequency, width, softness, distortion } => {
                // Lines across the axis, bent by fBm.
                let split = self.split(shader, position)?;
                let (_, _, along) = across(axis);
                let mut field = self.mul(shader, Slot::Link(split, along), num(frequency))?;
                if distortion != 0.0 {
                    let n = self.noise(shader, position, [frequency.max(1e-4) * 0.3; 3], seed, octaves(3))?;
                    let n = self.madd(shader, n, num(2.0), num(-1.0))?;
                    field = self.madd(shader, n, num(distortion), field)?;
                }
                let line = self.unary(shader, "round", field)?;
                let off = self.sub(shader, field, line)?;
                let off = self.unary(shader, "absolute", off)?;
                let half = width.clamp(0.0, 1.0) * 0.5;
                let soft = softness.clamp(0.0, 1.0) * 0.5 + 1e-4;
                let edge = self.smoothstep(shader, num(half), num(half + soft), off)?;
                self.sub(shader, num(1.0), edge)?
            }
            Procedural::BrushedRadial { axis, frequency, stretch } => {
                // Noise fine across the circles and stretched round them:
                // Tequila's (r f, u/r * r f/s, v/r * r f/s) is (r f, u f/s,
                // v f/s).
                let split = self.split(shader, position)?;
                let (a, b, _) = across(axis);
                let flat = self.combine(shader, Slot::Link(split, a), Slot::Link(split, b), zero)?;
                let radius = self.vector_op(shader, "length", flat, None, None)?;
                let at = self.combine(shader, Slot::Link(radius.0, radius.1), Slot::Link(split, a), Slot::Link(split, b))?;
                let f = frequency.max(1e-4);
                let around = f / stretch.max(1e-3);
                self.noise(shader, at, [f, around, around], seed, one_octave(Fractal::default()))?
            }
            Procedural::WoodAdvanced { .. } => self.wood_advanced(shader, position, seed, procedural)?,
            Procedural::Marble { axis, frequency, distortion, fractal } => {
                // Bands along the axis, bent by turbulence at a third of
                // their frequency.
                let split = self.session.node(shader, "separate_xyz")?;
                self.session.connect(shader, position.0, position.1, split, "Vector")?;
                let (_, _, along) = across(axis);
                let wander = self.turbulence(shader, position, frequency / 3.0, seed, fractal)?;
                let along = self.float_op(shader, "multiply", Slot::Link(split, along), Slot::Value(Value::Float(frequency)))?;
                let banded = self.float_op3(shader, "multiply_add", wander, Slot::Value(Value::Float(distortion)), along)?;
                let wave = self.float_op(shader, "sine", banded, zero)?;
                self.float_op3(shader, "multiply_add", wave, Slot::Value(Value::Float(0.5)), Slot::Value(Value::Float(0.5)))?
            }
            Procedural::Wood { axis, frequency, distortion, grain, fractal } => {
                let split = self.session.node(shader, "separate_xyz")?;
                self.session.connect(shader, position.0, position.1, split, "Vector")?;
                let (a, b, along) = across(axis);
                // Rings about the axis, wandering by fBm at an eighth of
                // their frequency.
                let flat = self.session.node(shader, "combine_xyz")?;
                self.session.connect(shader, split, a, flat, "X")?;
                self.session.connect(shader, split, b, flat, "Y")?;
                let radius = self.vector_op(shader, "length", (flat, "Vector"), None, None)?;
                let radius = self.float_op(shader, "multiply", Slot::Link(radius.0, radius.1), Slot::Value(Value::Float(frequency)))?;
                let wander = self.noise(shader, position, [frequency / 8.0; 3], seed, fractal)?;
                let wander = self.float_op3(shader, "multiply_add", wander, Slot::Value(Value::Float(2.0)), Slot::Value(Value::Float(-1.0)))?;
                let rings = self.float_op3(shader, "multiply_add", wander, Slot::Value(Value::Float(distortion * 8.0)), radius)?;
                // Rust's fract, which keeps the sign: |x - trunc(x)|.
                let whole = self.float_op(shader, "trunc", rings, zero)?;
                let ring = self.float_op(shader, "subtract", rings, whole)?;
                let ring = self.float_op(shader, "absolute", ring, zero)?;
                if grain <= 0.0 {
                    self.clamped(shader, "add", ring, zero, zero)?
                } else {
                    // Streaks, stretched along the grain.
                    let turned = self.session.node(shader, "combine_xyz")?;
                    self.session.connect(shader, split, a, turned, "X")?;
                    self.session.connect(shader, split, along, turned, "Y")?;
                    self.session.connect(shader, split, b, turned, "Z")?;
                    let streak = self.noise(
                        shader,
                        (turned, "Vector"),
                        [frequency * 3.0, frequency * 0.08, frequency * 3.0],
                        seed ^ 0x51a4_0f37,
                        one_octave(Fractal::default()),
                    )?;
                    let streak = self.float_op(shader, "multiply", streak, Slot::Value(Value::Float(grain)))?;
                    self.clamped(shader, "multiply_add", ring, Slot::Value(Value::Float(1.0 - grain)), streak)?
                }
            }
            Procedural::Scratches { .. } | Procedural::Weave { .. } | Procedural::Mesh { .. } => unreachable!("planar, and handled above"),
        };
        Ok(Some(slot))
    }

    /// Tequila's advanced wood: rings - wandering, uneven, with a latewood
    /// band - planks, knots, ray flecks and streaks.
    fn wood_advanced(
        &mut self,
        shader: Shader,
        position: (Node, &'static str),
        seed: u32,
        procedural: &tequila_texture::procedural::Procedural,
    ) -> Result<Slot, String> {
        use tequila_texture::noise::Fractal;
        use tequila_texture::procedural::Procedural;
        use std::f32::consts::TAU;
        let Procedural::WoodAdvanced {
            axis,
            frequency,
            distortion,
            grain,
            fractal,
            ring_variation,
            latewood,
            contrast,
            flecks,
            knots,
            board_width,
        } = *procedural
        else {
            unreachable!("called for advanced wood")
        };
        let single = Fractal { octaves: 1, ..Fractal::default() };
        let f = frequency.max(1e-3);
        let split = self.split(shader, position)?;
        let (a, b, length_axis) = across(axis);
        let (mut u, mut v) = (Slot::Link(split, a), Slot::Link(split, b));
        let along = Slot::Link(split, length_axis);
        // Planks side by side across the grain, each cut from its own log:
        // the pith somewhere else, the grain turned a little. The plank's
        // number keys everything random from here on, as its seed does in
        // Tequila's.
        let mut board = num(0.0);
        if board_width > 1e-4 {
            let index = self.float_op(shader, "divide", u, num(board_width))?;
            let index = self.unary(shader, "floor", index)?;
            board = index;
            let key = self.combine(shader, index, num(salt(seed, 0x2b0a_7d11)), num(0.0))?;
            let random = self.white(shader, key)?;
            let r = self.channels(shader, (random, "Color"))?;
            let centre = self.madd(shader, index, num(board_width), num(0.5 * board_width))?;
            let moved = self.madd(shader, Slot::Link(r, "Red"), num(board_width * 3.0), num(-1.5 * board_width))?;
            let u1 = self.sub(shader, u, centre)?;
            let u1 = self.add(shader, u1, moved)?;
            let lifted = self.madd(shader, Slot::Link(r, "Green"), num(board_width * 4.0), num(-2.0 * board_width))?;
            let v1 = self.add(shader, v, lifted)?;
            let turn = self.madd(shader, Slot::Link(r, "Blue"), num(0.6), num(-0.3))?;
            let (s, c) = (self.unary(shader, "sine", turn)?, self.unary(shader, "cosine", turn)?);
            let (uc, vs) = (self.mul(shader, u1, c)?, self.mul(shader, v1, s)?);
            let (us, vc) = (self.mul(shader, u1, s)?, self.mul(shader, v1, c)?);
            u = self.sub(shader, uc, vs)?;
            v = self.add(shader, us, vc)?;
        }
        let flat = self.combine(shader, u, v, num(0.0))?;
        let length = self.vector_op(shader, "length", flat, None, None)?;
        let mut radius = Slot::Link(length.0, length.1);
        // Knots: a branch through the trunk now and then along it, each a
        // swelling of the rings round a dark core, on the side it grew.
        let mut core = num(0.0);
        if knots > 0.0 {
            let cell_length = 6.0 / f;
            let cell = self.float_op(shader, "divide", along, num(cell_length))?;
            let cell = self.unary(shader, "floor", cell)?;
            for step in -1..=1 {
                let k = self.add(shader, cell, num(step as f32))?;
                let key = self.combine(shader, k, board, num(salt(seed, 0x6f1d_93a5)))?;
                let first = self.white(shader, key)?;
                let r = self.channels(shader, (first, "Color"))?;
                let key = self.combine(shader, k, board, num(salt(seed, 0x6f1d_93a6)))?;
                let second = self.white(shader, key)?;
                let present = self.float_op(shader, "less_than", Slot::Link(r, "Red"), num(knots))?;
                let angle = self.mul(shader, Slot::Link(r, "Green"), num(TAU))?;
                let (du, dv) = (self.unary(shader, "cosine", angle)?, self.unary(shader, "sine", angle)?);
                let ud = self.mul(shader, u, du)?;
                let facing = self.madd(shader, v, dv, ud)?;
                let facing = self.float_op(shader, "greater_than", facing, num(0.0))?;
                let gate = self.mul(shader, present, facing)?;
                let (vdu, udv) = (self.mul(shader, v, du)?, self.mul(shader, u, dv)?);
                let side = self.sub(shader, vdu, udv)?;
                let height = self.add(shader, k, Slot::Link(r, "Blue"))?;
                let height = self.mul(shader, height, num(cell_length))?;
                let rise = self.sub(shader, along, height)?;
                let d2 = self.mul(shader, side, side)?;
                let d2 = self.madd(shader, rise, rise, d2)?;
                let d = self.unary(shader, "sqrt", d2)?;
                let size = self.madd(shader, Slot::Link(second, "Value"), num(0.6 / f), num(0.36 / f))?;
                let size2 = self.mul(shader, size, size)?;
                let falloff = self.float_op(shader, "divide", d2, size2)?;
                let falloff = self.mul(shader, falloff, num(-1.0))?;
                let falloff = self.unary(shader, "exponent", falloff)?;
                let swell = self.mul(shader, falloff, size)?;
                let swell = self.mul(shader, swell, gate)?;
                radius = self.madd(shader, swell, num(2.0), radius)?;
                let inner = self.mul(shader, size, num(0.6))?;
                let reach = self.float_op(shader, "divide", d, inner)?;
                let here = self.clamped(shader, "subtract", num(1.0), reach, num(0.0))?;
                let here = self.mul(shader, here, gate)?;
                core = self.float_op(shader, "maximum", core, here)?;
            }
        }
        // Years, wandering, some wider than others.
        let wander = self.noise(shader, position, [f / 8.0; 3], seed, fractal)?;
        let wander = self.madd(shader, wander, num(2.0), num(-1.0))?;
        let out = self.mul(shader, radius, num(f))?;
        let years = self.madd(shader, wander, num(distortion * 8.0), out)?;
        let mut grown = years;
        if ring_variation != 0.0 {
            let x = self.mul(shader, years, num(0.15))?;
            let by_board = self.mul(shader, board, num(13.7))?;
            let at = self.combine(shader, x, by_board, num(0.0))?;
            let n = self.noise(shader, at, [1.0; 3], seed ^ 0x1234_5678, single)?;
            let n = self.madd(shader, n, num(2.0), num(-1.0))?;
            grown = self.madd(shader, n, num(2.0 * ring_variation), years)?;
        }
        let whole = self.unary(shader, "trunc", grown)?;
        let phase = self.sub(shader, grown, whole)?;
        let phase = self.unary(shader, "absolute", phase)?;
        // The dense latewood at each year's end, eased in.
        let late = latewood.clamp(0.02, 0.98);
        let profile = self.smoothstep(shader, num(1.0 - late), num(1.0), phase)?;
        let c = contrast.clamp(0.0, 1.0);
        let ring = self.mul(shader, phase, num(1.0 - c))?;
        let ring = self.madd(shader, profile, num(c), ring)?;
        // Ray flecks, across the rings.
        let mut fleck = num(0.0);
        if flecks > 0.0 {
            let angle = self.float_op(shader, "arctan2", v, u)?;
            let x = self.mul(shader, angle, num(80.0))?;
            let by_board = self.mul(shader, board, num(17.3))?;
            let x = self.add(shader, x, by_board)?;
            let y = self.mul(shader, radius, num(f * 0.6))?;
            let z = self.mul(shader, along, num(f * 0.8))?;
            let at = self.combine(shader, x, y, z)?;
            let n = self.noise(shader, at, [1.0; 3], seed ^ 0x77aa_1c3d, single)?;
            let edge = 1.0 - flecks.clamp(0.0, 1.0) * 0.35;
            fleck = self.smoothstep(shader, num(edge), num(edge + (1.0 - edge).max(1e-4)), n)?;
        }
        // Streaks along the grain.
        let mut wood = ring;
        if grain > 0.0 {
            let turned = self.combine(shader, u, along, v)?;
            let streak = self.noise(shader, turned, [f * 3.0, f * 0.08, f * 3.0], seed ^ 0x51a4_0f37, single)?;
            let kept = self.mul(shader, ring, num(1.0 - grain))?;
            wood = self.madd(shader, streak, num(grain), kept)?;
        }
        let dimmed = self.madd(shader, fleck, num(-0.5), num(1.0))?;
        let lit = self.mul(shader, wood, dimmed)?;
        let rest = self.sub(shader, num(1.0), wood)?;
        let knot = self.mul(shader, core, rest)?;
        self.clamped(shader, "add", lit, knot, num(0.0))
    }

    /// Tequila's scratches: in each cell of the plane, maybe, a short
    /// straight scratch at its own place, angle and length; the value is the
    /// distance to the nearest, in widths.
    #[allow(clippy::too_many_arguments)]
    fn scratches(&mut self, shader: Shader, (a, b): (Slot, Slot), seed: u32, frequency: f32, width: f32, density: f32, spread: f32) -> Result<Slot, String> {
        const FAR: f32 = 1e6;
        let f = frequency.max(1e-4);
        let (px, py) = (self.mul(shader, a, num(f))?, self.mul(shader, b, num(f))?);
        let (bx, by) = (self.unary(shader, "floor", px)?, self.unary(shader, "floor", py)?);
        let spread = spread.clamp(0.0, 0.5) * std::f32::consts::TAU;
        let mut nearest = num(FAR);
        for dy in -1..=1 {
            for dx in -1..=1 {
                let cx = self.add(shader, bx, num(dx as f32))?;
                let cy = self.add(shader, by, num(dy as f32))?;
                let key = self.combine(shader, cx, cy, num(salt(seed, 0x51ed_2701)))?;
                let first = self.white(shader, key)?;
                let r1 = self.channels(shader, (first, "Color"))?;
                let key = self.combine(shader, cx, cy, num(salt(seed, 0x7c93_1055)))?;
                let second = self.white(shader, key)?;
                let r2 = self.channels(shader, (second, "Color"))?;
                let present = self.float_op(shader, "less_than", Slot::Link(r1, "Red"), num(density.clamp(0.0, 1.0)))?;
                let centre_x = self.add(shader, cx, Slot::Link(r1, "Green"))?;
                let centre_y = self.add(shader, cy, Slot::Link(r1, "Blue"))?;
                let angle = self.madd(shader, Slot::Link(r2, "Red"), num(spread), num(-0.5 * spread))?;
                let (ux, uy) = (self.unary(shader, "cosine", angle)?, self.unary(shader, "sine", angle)?);
                let half = self.madd(shader, Slot::Link(r2, "Green"), num(0.3), num(0.2))?;
                let ox = self.sub(shader, px, centre_x)?;
                let oy = self.sub(shader, py, centre_y)?;
                let along = self.mul(shader, oy, uy)?;
                let along = self.madd(shader, ox, ux, along)?;
                let back = self.mul(shader, half, num(-1.0))?;
                let along = self.float_op(shader, "maximum", along, back)?;
                let along = self.float_op(shader, "minimum", along, half)?;
                let (sx, sy) = (self.mul(shader, ux, along)?, self.mul(shader, uy, along)?);
                let (qx, qy) = (self.sub(shader, ox, sx)?, self.sub(shader, oy, sy)?);
                let d2 = self.mul(shader, qy, qy)?;
                let d2 = self.madd(shader, qx, qx, d2)?;
                let d = self.unary(shader, "sqrt", d2)?;
                // A cell without a scratch is as far as can be.
                let absent = self.madd(shader, present, num(-FAR), num(FAR))?;
                let d = self.add(shader, d, absent)?;
                nearest = self.float_op(shader, "minimum", nearest, d)?;
            }
        }
        self.clamped(shader, "multiply", nearest, num(1.0 / width.max(1e-4)), num(0.0))
    }

    /// Tequila's woven cloth: warp and weft threads, rounded, arching over
    /// and dipping under each other in the pattern's order, some threads
    /// thinner than others.
    #[allow(clippy::too_many_arguments)]
    fn weave(
        &mut self,
        shader: Shader,
        (a, b): (Slot, Slot),
        seed: u32,
        pattern: tequila_texture::procedural::WeavePattern,
        frequency: f32,
        width: f32,
        variance: f32,
    ) -> Result<Slot, String> {
        use tequila_texture::procedural::WeavePattern;
        let f = frequency.max(1e-4);
        let (ox, oy) = tequila_texture::procedural::phase(0.0, 0.0, seed);
        let x = self.madd(shader, a, num(f), num(ox))?;
        let y = self.madd(shader, b, num(f), num(oy))?;
        let (i, j) = (self.unary(shader, "floor", x)?, self.unary(shader, "floor", y)?);
        let (fx, fy) = (self.sub(shader, x, i)?, self.sub(shader, y, j)?);
        let warp_on_top = match pattern {
            WeavePattern::Plain => {
                let s = self.add(shader, i, j)?;
                let m = self.float_op(shader, "floored_modulo", s, num(2.0))?;
                self.float_op(shader, "less_than", m, num(0.5))?
            }
            WeavePattern::Twill => {
                let s = self.sub(shader, i, j)?;
                let m = self.float_op(shader, "floored_modulo", s, num(3.0))?;
                self.float_op(shader, "greater_than", m, num(0.5))?
            }
            WeavePattern::Satin => {
                let s = self.madd(shader, i, num(-2.0), j)?;
                let m = self.float_op(shader, "floored_modulo", s, num(5.0))?;
                self.float_op(shader, "greater_than", m, num(0.5))?
            }
        };
        // A thread's round profile across it, from its middle.
        let half = width.clamp(0.0, 1.0) * 0.5;
        let thread = |builder: &mut Self, across: Slot| -> Result<Slot, String> {
            if half <= 0.0 {
                return Ok(num(0.0));
            }
            let off = builder.sub(shader, across, num(0.5))?;
            let off = builder.unary(shader, "absolute", off)?;
            let t = builder.mul(shader, off, num(1.0 / half))?;
            let t2 = builder.mul(shader, t, t)?;
            let rest = builder.sub(shader, num(1.0), t2)?;
            let rest = builder.float_op(shader, "maximum", rest, num(0.0))?;
            builder.unary(shader, "sqrt", rest)
        };
        let warp = thread(self, fx)?;
        let weft = thread(self, fy)?;
        let variance = variance.clamp(0.0, 1.0);
        let (warp_scale, weft_scale) = if variance > 0.0 {
            let key = self.combine(shader, i, num(salt(seed, 0x7a3d)), num(0.0))?;
            let wi = self.white(shader, key)?;
            let key = self.combine(shader, num(salt(seed, 0x1f5b)), j, num(1.0))?;
            let wj = self.white(shader, key)?;
            (
                self.madd(shader, Slot::Link(wi, "Value"), num(-variance), num(1.0))?,
                self.madd(shader, Slot::Link(wj, "Value"), num(-variance), num(1.0))?,
            )
        } else {
            (num(1.0), num(1.0))
        };
        let pi = std::f32::consts::PI;
        let arch_y = self.mul(shader, fy, num(pi))?;
        let arch_y = self.unary(shader, "sine", arch_y)?;
        let arch_x = self.mul(shader, fx, num(pi))?;
        let arch_x = self.unary(shader, "sine", arch_x)?;
        // The warp's or the weft's, by which is on top.
        let pick = |builder: &mut Self, warp_side: Slot, weft_side: Slot| -> Result<Slot, String> {
            let gap = builder.sub(shader, warp_side, weft_side)?;
            builder.madd(shader, gap, warp_on_top, weft_side)
        };
        let top = pick(self, warp, weft)?;
        let arch = pick(self, arch_y, arch_x)?;
        let top_scale = pick(self, warp_scale, weft_scale)?;
        let under = pick(self, weft, warp)?;
        let under_scale = pick(self, weft_scale, warp_scale)?;
        let raised = self.madd(shader, arch, num(0.4), num(0.6))?;
        let raised = self.mul(shader, raised, top)?;
        let raised = self.mul(shader, raised, top_scale)?;
        let raised = self.madd(shader, raised, num(0.5), num(0.5))?;
        let lowered = self.mul(shader, under, under_scale)?;
        let lowered = self.mul(shader, lowered, num(0.5))?;
        let on = self.float_op(shader, "greater_than", top, num(0.0))?;
        let lift = self.sub(shader, raised, lowered)?;
        self.clamped(shader, "multiply_add", lift, on, lowered)
    }

    /// Tequila's perforated mesh: a hole in every cell of the plane, round,
    /// square or - on a staggered grid - hexagonal.
    #[allow(clippy::too_many_arguments)]
    fn perforated(
        &mut self,
        shader: Shader,
        (a, b): (Slot, Slot),
        seed: u32,
        shape: tequila_texture::procedural::HoleShape,
        frequency: f32,
        size: f32,
        softness: f32,
    ) -> Result<Slot, String> {
        use tequila_texture::procedural::HoleShape;
        let f = frequency.max(1e-4);
        let (ox, oy) = tequila_texture::procedural::phase(0.0, 0.0, seed);
        let x = self.madd(shader, a, num(f), num(ox))?;
        let y = self.madd(shader, b, num(f), num(oy))?;
        let distance = match shape {
            HoleShape::Round | HoleShape::Square => {
                let dx = self.unary(shader, "fraction", x)?;
                let dx = self.sub(shader, dx, num(0.5))?;
                let dy = self.unary(shader, "fraction", y)?;
                let dy = self.sub(shader, dy, num(0.5))?;
                if shape == HoleShape::Round {
                    let d2 = self.mul(shader, dy, dy)?;
                    let d2 = self.madd(shader, dx, dx, d2)?;
                    self.unary(shader, "sqrt", d2)?
                } else {
                    let (ax, ay) = (self.unary(shader, "absolute", dx)?, self.unary(shader, "absolute", dy)?);
                    self.float_op(shader, "maximum", ax, ay)?
                }
            }
            HoleShape::Hex => {
                let h = 3.0f32.sqrt();
                // The nearer of two staggered grids' cell centres.
                let local = |builder: &mut Self, x: Slot, y: Slot| -> Result<(Slot, Slot, Slot), String> {
                    let lx = builder.unary(shader, "fraction", x)?;
                    let lx = builder.sub(shader, lx, num(0.5))?;
                    let ly = builder.float_op(shader, "floored_modulo", y, num(h))?;
                    let ly = builder.sub(shader, ly, num(h * 0.5))?;
                    let d2 = builder.mul(shader, ly, ly)?;
                    let d2 = builder.madd(shader, lx, lx, d2)?;
                    Ok((lx, ly, d2))
                };
                let (ax, ay, ad) = local(self, x, y)?;
                let x2 = self.sub(shader, x, num(0.5))?;
                let y2 = self.sub(shader, y, num(h * 0.5))?;
                let (bx, by, bd) = local(self, x2, y2)?;
                let first = self.float_op(shader, "less_than", ad, bd)?;
                let gap = self.sub(shader, ax, bx)?;
                let dx = self.madd(shader, gap, first, bx)?;
                let gap = self.sub(shader, ay, by)?;
                let dy = self.madd(shader, gap, first, by)?;
                let (adx, ady) = (self.unary(shader, "absolute", dx)?, self.unary(shader, "absolute", dy)?);
                let slant = self.mul(shader, ady, num(0.5))?;
                let slant = self.madd(shader, adx, num(0.866_025), slant)?;
                self.float_op(shader, "maximum", slant, ady)?
            }
        };
        let radius = size.clamp(0.0, 1.0) * 0.5;
        let soft = softness.clamp(0.0, 1.0) * 0.25 + 1e-4;
        self.smoothstep(shader, num(radius - soft), num(radius + soft), distance)
    }

    fn add(&mut self, shader: Shader, a: Slot, b: Slot) -> Result<Slot, String> {
        self.float_op(shader, "add", a, b)
    }

    fn sub(&mut self, shader: Shader, a: Slot, b: Slot) -> Result<Slot, String> {
        self.float_op(shader, "subtract", a, b)
    }

    fn mul(&mut self, shader: Shader, a: Slot, b: Slot) -> Result<Slot, String> {
        self.float_op(shader, "multiply", a, b)
    }

    /// `a * b + c`.
    fn madd(&mut self, shader: Shader, a: Slot, b: Slot, c: Slot) -> Result<Slot, String> {
        self.float_op3(shader, "multiply_add", a, b, c)
    }

    /// A one-input math node.
    fn unary(&mut self, shader: Shader, op: &str, a: Slot) -> Result<Slot, String> {
        self.float_op(shader, op, a, num(0.0))
    }

    /// Tequila's smoothstep of `x` from `from` to `to` - a step where the
    /// two meet - which is Cycles' map range in its smoothstep mode.
    fn smoothstep(&mut self, shader: Shader, from: Slot, to: Slot, x: Slot) -> Result<Slot, String> {
        let node = self.session.node(shader, "map_range")?;
        self.session.set_string(node, "range_type", "smoothstep")?;
        self.feed(shader, node, "Value", x, false)?;
        self.feed(shader, node, "From Min", from, false)?;
        self.feed(shader, node, "From Max", to, false)?;
        Ok(Slot::Link(node, "Result"))
    }

    fn combine(&mut self, shader: Shader, x: Slot, y: Slot, z: Slot) -> Result<(Node, &'static str), String> {
        let node = self.session.node(shader, "combine_xyz")?;
        self.feed(shader, node, "X", x, false)?;
        self.feed(shader, node, "Y", y, false)?;
        self.feed(shader, node, "Z", z, false)?;
        Ok((node, "Vector"))
    }

    fn split(&mut self, shader: Shader, vector: (Node, &'static str)) -> Result<Node, String> {
        let node = self.session.node(shader, "separate_xyz")?;
        self.session.connect(shader, vector.0, vector.1, node, "Vector")?;
        Ok(node)
    }

    /// A colour's channels, as outputs Red, Green and Blue.
    fn channels(&mut self, shader: Shader, colour: (Node, &'static str)) -> Result<Node, String> {
        let node = self.session.node(shader, "separate_color")?;
        self.session.connect(shader, colour.0, colour.1, node, "Color")?;
        Ok(node)
    }

    fn red(&mut self, shader: Shader, colour: (Node, &'static str)) -> Result<Slot, String> {
        Ok(Slot::Link(self.channels(shader, colour)?, "Red"))
    }

    /// Random numbers for a point: a different three, uniform in 0 to 1,
    /// for every distinct `key` (output Color, and Value for a fourth).
    fn white(&mut self, shader: Shader, key: (Node, &'static str)) -> Result<Node, String> {
        let node = self.session.node(shader, "white_noise_texture")?;
        self.session.connect(shader, key.0, key.1, node, "Vector")?;
        Ok(node)
    }

    /// Worley cells over `placed`, which `noise_space` scaled and moved.
    fn voronoi(&mut self, shader: Shader, placed: (Node, &'static str), metric: &str, feature: &str) -> Result<Node, String> {
        let node = self.session.node(shader, "voronoi_texture")?;
        self.session.set_string(node, "metric", metric)?;
        self.session.set_string(node, "feature", feature)?;
        self.session.set_float(node, "scale", 1.0)?;
        self.session.connect(shader, placed.0, placed.1, node, "Vector")?;
        Ok(node)
    }

    /// Tequila's `worley(position * frequency, seed).id`: a random number
    /// per cell.
    fn cell_id(&mut self, shader: Shader, position: (Node, &'static str), frequency: f32, seed: u32) -> Result<Slot, String> {
        let placed = self.noise_space(shader, position, [frequency; 3], seed)?;
        let near = self.voronoi(shader, placed, "euclidean", "f1")?;
        self.red(shader, (near, "Color"))
    }

    /// Where a procedural is read: in object space, in scene units, by
    /// default; through a mapping, at the mapping's (u, v) on the plane z = 0.
    fn procedural_position(&mut self, shader: Shader, mapping: Option<&Mapping>) -> Result<(Node, &'static str), String> {
        match mapping {
            Some(mapping) if mapping.projection != TextureProjection::TriPlanar => {
                let uv = match self.mapped_vector(shader, mapping)? {
                    Some(uv) => uv,
                    None => (self.session.node(shader, "texture_coordinate")?, "UV"),
                };
                // Cycles' v counts up from the bottom; Tequila's, down.
                let turned = self.session.node(shader, "mapping")?;
                self.session.set_string(turned, "mapping_type", "point")?;
                self.session.set_float3(turned, "scale", [1.0, -1.0, 1.0])?;
                self.session.set_float3(turned, "location", [0.0, 1.0, 0.0])?;
                self.session.connect(shader, uv.0, uv.1, turned, "Vector")?;
                return Ok((turned, "Vector"));
            }
            Some(_) => self.out.warn("a procedural through a tri-planar mapping is read in object space"),
            None => {}
        }
        let coords = self.session.node(shader, "texture_coordinate")?;
        let info = self.session.node(shader, "object_info")?;
        self.vector_op(shader, "multiply", (coords, "Object"), Some((info, "Color")), None)
    }

    /// `position` scaled per axis and moved by the seed: the space a
    /// pattern's noise is read in.
    fn noise_space(&mut self, shader: Shader, position: (Node, &'static str), scale: [f32; 3], seed: u32) -> Result<(Node, &'static str), String> {
        self.noise_space_at(shader, position, scale, seed_offset(seed))
    }

    /// `position` scaled per axis, then moved to `location`.
    fn noise_space_at(&mut self, shader: Shader, position: (Node, &'static str), scale: [f32; 3], location: [f32; 3]) -> Result<(Node, &'static str), String> {
        let node = self.session.node(shader, "mapping")?;
        self.session.set_string(node, "mapping_type", "point")?;
        self.session.set_float3(node, "scale", scale)?;
        self.session.set_float3(node, "location", location)?;
        self.session.connect(shader, position.0, position.1, node, "Vector")?;
        Ok((node, "Vector"))
    }

    /// Tequila's `fbm(position * scale, seed, fractal) * 0.5 + 0.5`.
    /// Cycles' normalized fBm is the same sum, its noise scaled by 0.982 to
    /// Tequila's 1.3333 (`noise::perlin`), clamped as Tequila clamps.
    fn noise(&mut self, shader: Shader, position: (Node, &'static str), scale: [f32; 3], seed: u32, fractal: tequila_texture::noise::Fractal) -> Result<Slot, String> {
        self.noise_at(shader, position, scale, seed_offset(seed), fractal)
    }

    /// As `noise`, its space moved to `location` rather than by a seed.
    fn noise_at(&mut self, shader: Shader, position: (Node, &'static str), scale: [f32; 3], location: [f32; 3], fractal: tequila_texture::noise::Fractal) -> Result<Slot, String> {
        const RANGE: f32 = 1.3333 / 0.982;
        let placed = self.noise_space_at(shader, position, scale, location)?;
        let node = self.session.node(shader, "noise_texture")?;
        self.session.set_float(node, "scale", 1.0)?;
        self.session.set_float(node, "detail", (fractal.octaves.max(1) - 1).min(15) as f32)?;
        self.session.set_float(node, "roughness", fractal.gain)?;
        self.session.set_float(node, "lacunarity", fractal.lacunarity)?;
        self.session.connect(shader, placed.0, placed.1, node, "Vector")?;
        self.clamped(shader, "multiply_add", Slot::Link(node, "Fac"), Slot::Value(Value::Float(RANGE)), Slot::Value(Value::Float(0.5 - 0.5 * RANGE)))
    }

    /// Tequila's `turbulence(position * frequency, seed, fractal)`: fBm of
    /// |noise|, an octave at a time.
    fn turbulence(&mut self, shader: Shader, position: (Node, &'static str), frequency: f32, seed: u32, fractal: tequila_texture::noise::Fractal) -> Result<Slot, String> {
        let octaves = fractal.octaves.max(1);
        let total: f32 = (0..octaves).map(|i| fractal.gain.powi(i as i32)).sum();
        let single = tequila_texture::noise::Fractal { octaves: 1, ..fractal };
        let mut sum = Slot::Value(Value::Float(0.0));
        let (mut amplitude, mut f) = (1.0, frequency);
        for octave in 0..octaves {
            let n = self.noise(shader, position, [f; 3], seed.wrapping_add(octave * 0x9e37), single)?;
            let signed = self.float_op3(shader, "multiply_add", n, Slot::Value(Value::Float(2.0)), Slot::Value(Value::Float(-1.0)))?;
            let size = self.float_op(shader, "absolute", signed, Slot::Value(Value::Float(0.0)))?;
            sum = self.float_op3(shader, "multiply_add", size, Slot::Value(Value::Float(amplitude / total)), sum)?;
            amplitude *= fractal.gain;
            f *= fractal.lacunarity;
        }
        Ok(sum)
    }

    /// Material `index`'s shader: a new one, or `reuse` built again.
    fn material(&mut self, index: usize, reuse: Option<Node>) -> Result<Node, String> {
        let material: &Material = &self.scene.materials[index];
        let program = self.scene.programs.get(index).and_then(|p| p.as_deref());
        let shader = match reuse {
            Some(shader) => self.session.reopen(shader).ok_or("this Cycles library cannot rebuild a shader in place")??,
            None => self.session.shader(&format!("material{index}"))?,
        };
        let bsdf = self.session.node(shader, "principled_bsdf")?;
        let output = self.session.node(shader, "output")?;

        // The parameter block the program starts from, and what it drives.
        let constants = program.map(|p| &p.constants).unwrap_or(material);
        let registers = match program {
            Some(p) => self.program(shader, p)?,
            None => HashMap::new(),
        };
        let driven: HashMap<Param, Slot> = program
            .map(|p| {
                p.driven
                    .iter()
                    .filter_map(|(param, r)| registers.get(&r.0).map(|s| (*param, *s)))
                    .collect()
            })
            .unwrap_or_default();
        let value = |param: Param, constant: Value| driven.get(&param).copied().unwrap_or(Slot::Value(constant));

        if std::env::var_os("TEQUILA_CYCLES_DUMP").is_some() {
            eprintln!(
                "material {index}: opacity {} transmission {} thin {} driven {:?} bump {}",
                constants.geometry_opacity,
                constants.transmission_weight,
                constants.geometry_thin_walled,
                driven.keys().collect::<Vec<_>>(),
                self.scene.bumps.get(index).and_then(|b| b.as_deref()).is_some(),
            );
            if let Some(p) = program {
                for i in &p.instructions {
                    eprintln!("    {}", format!("{i:?}").chars().take(160).collect::<String>());
                }
            }
        }
        let m = constants;
        // A thin wall - a bulb's shell, a window pane - passes light straight
        // through rather than bending it, which Cycles' principled shader
        // cannot do: its transmission is always a solid's. Such a material
        // is the principled shader without transmission, blended by the
        // transmission weight with a shell made below.
        let thin = m.geometry_thin_walled && (m.transmission_weight > 0.0 || driven.contains_key(&Param::TransmissionWeight));
        let transmissive = !thin && (m.transmission_weight >= 0.5 || driven.contains_key(&Param::TransmissionWeight));
        // Cycles tints transmission by the base colour, the square root at
        // each crossing; Tequila has a colour of its own for it, and a
        // surface tint. Glass takes its transmission colour - unless it has
        // a depth, when the colour belongs to the volume inside, which
        // absorbs it along the way (below) - and the surface tint either way.
        let volume = !thin && m.transmission_weight > 0.0 && m.transmission_depth > 0.0;
        let surface_tint = value(Param::TransmissionTint, Value::Color(m.transmission_tint));
        let base = if transmissive && volume {
            surface_tint
        } else if transmissive {
            let colour = value(Param::TransmissionColor, Value::Color(m.transmission_color));
            self.apply(shader, &NodeKind::Multiply, &[colour, surface_tint], 0)?
        } else {
            value(Param::BaseColor, Value::Color(m.base_color * m.base_weight))
        };
        if m.base_weight != 1.0 && driven.contains_key(&Param::BaseColor) {
            self.out.warn("a base weight on a textured colour is not translated");
        }
        // Labels, bottom to top, laid over the material as Tequila's shading
        // lays them (`shade::shade`): each its colour where it covers, foil
        // turning metal and ink not, glass made opaque under it, and its own
        // roughness. Read through their own mappings, clear outside the
        // artwork.
        let mut base = base;
        let mut metalness = value(Param::BaseMetalness, Value::Float(m.base_metalness));
        let mut transmission = value(Param::TransmissionWeight, Value::Float(m.transmission_weight));
        let mut roughness = value(Param::SpecularRoughness, Value::Float(m.specular_roughness));
        let decals: &[tequila_materials::decal::Decal] = program.map(|p| p.decals.as_slice()).unwrap_or(&[]);
        for decal in decals.iter().filter(|d| d.enabled && d.opacity > 0.0) {
            let Some(label) = self.texture_node_with(shader, decal.texture, &decal.mapping, true)? else {
                continue;
            };
            let mut alpha = self.float_op(shader, "multiply", label.alpha, Slot::Value(Value::Float(decal.opacity)))?;
            if let Some(side) = self.label_side(shader, decal)? {
                alpha = self.float_op(shader, "multiply", alpha, side)?;
            }
            let ink = if decal.ink == tequila_core::spectrum::Spectrum::ONE {
                label.colour()
            } else {
                self.apply(shader, &NodeKind::Multiply, &[label.colour(), Slot::Value(Value::Color(decal.ink))], 0)?
            };
            base = self.apply(shader, &NodeKind::Mix, &[base, ink, alpha], 0)?;
            let foil = Slot::Value(Value::Float(if decal.metallic { 1.0 } else { 0.0 }));
            metalness = self.float_mix(shader, metalness, foil, alpha)?;
            let bare = self.float_op(shader, "subtract", Slot::Value(Value::Float(1.0)), alpha)?;
            transmission = self.float_op(shader, "multiply", transmission, bare)?;
            roughness = self.float_mix(shader, roughness, Slot::Value(Value::Float(decal.roughness)), alpha)?;
        }
        // A metal's colour is its measured index (n, k), tinted by the base
        // colour, as Tequila's conductor has it. Cycles' principled metal
        // takes a colour seen straight on and a tint at the edge, at 82
        // degrees [Hoffman 2023], and both follow from the index. Blended by
        // metalness, so a part only metal in places stays itself elsewhere.
        let metal = !transmissive
            && (m.base_metalness > 0.0 || driven.contains_key(&Param::BaseMetalness) || decals.iter().any(|d| d.enabled && d.metallic));
        let (base, edge) = if metal {
            let (f0, f82) = conductor_colours(&m.metal_ior);
            let tinted = self.apply(shader, &NodeKind::Multiply, &[base, Slot::Value(Value::Color(f0))], 0)?;
            let base = self.apply(shader, &NodeKind::Mix, &[base, tinted, metalness], 0)?;
            let specular = value(Param::SpecularColor, Value::Color(m.specular_color));
            let edge = self.apply(shader, &NodeKind::Mix, &[specular, Slot::Value(Value::Color(f82)), metalness], 0)?;
            (base, edge)
        } else {
            (base, value(Param::SpecularColor, Value::Color(m.specular_color)))
        };
        self.feed(shader, bsdf, "base_color", base, true)?;
        self.feed(shader, bsdf, "metallic", metalness, false)?;
        self.feed(shader, bsdf, "diffuse_roughness", value(Param::DiffuseRoughness, Value::Float(m.base_diffuse_roughness)), false)?;
        self.feed(shader, bsdf, "roughness", roughness, false)?;
        self.feed(shader, bsdf, "ior", value(Param::SpecularIor, Value::Float(m.specular_ior)), false)?;
        // OpenPBR's specular weight scales F0; Principled's IOR level does the
        // same with 0.5 as "as the IOR says".
        if driven.contains_key(&Param::SpecularWeight) {
            self.out.warn("a textured specular weight is not translated");
        }
        self.session.set_float(bsdf, "specular_ior_level", 0.5 * m.specular_weight)?;
        self.feed(shader, bsdf, "specular_tint", edge, true)?;
        let anisotropy = self.cycles_anisotropy(shader, value(Param::Anisotropy, Value::Float(m.specular_roughness_anisotropy)))?;
        self.feed(shader, bsdf, "anisotropic", anisotropy, false)?;
        // Tequila's rotation runs 0 to 1 over a half turn (an anisotropic
        // lobe turned half way round is the same lobe); Cycles' over a whole.
        let rotation = self.float_op(shader, "multiply", value(Param::AnisotropyRotation, Value::Float(m.specular_rotation)), Slot::Value(Value::Float(0.5)))?;
        self.feed(shader, bsdf, "anisotropic_rotation", rotation, false)?;
        if !thin {
            self.feed(shader, bsdf, "transmission_weight", transmission, false)?;
        }
        self.feed(shader, bsdf, "subsurface_weight", value(Param::SubsurfaceWeight, Value::Float(m.subsurface_weight)), false)?;
        let radius = rgb(m.subsurface_radius);
        self.session.set_float3(bsdf, "subsurface_radius", radius)?;
        self.session.set_float(bsdf, "subsurface_scale", 1.0)?;
        self.feed(shader, bsdf, "coat_weight", value(Param::CoatWeight, Value::Float(m.coat_weight)), false)?;
        self.feed(shader, bsdf, "coat_roughness", value(Param::CoatRoughness, Value::Float(m.coat_roughness)), false)?;
        self.feed(shader, bsdf, "coat_ior", value(Param::CoatIor, Value::Float(m.coat_ior)), false)?;
        self.feed(shader, bsdf, "coat_tint", value(Param::CoatColor, Value::Color(m.coat_color)), true)?;
        self.feed(shader, bsdf, "sheen_weight", value(Param::FuzzWeight, Value::Float(m.fuzz_weight)), false)?;
        self.feed(shader, bsdf, "sheen_tint", value(Param::FuzzColor, Value::Color(m.fuzz_color)), true)?;
        self.feed(shader, bsdf, "sheen_roughness", value(Param::FuzzRoughness, Value::Float(m.fuzz_roughness)), false)?;
        self.feed(shader, bsdf, "emission_color", value(Param::EmissionColor, Value::Color(m.emission_color)), true)?;
        self.feed(shader, bsdf, "emission_strength", value(Param::EmissionLuminance, Value::Float(m.emission_luminance)), false)?;
        // A thin shell's opacity cuts through the whole blend, below.
        if !thin {
            self.feed(shader, bsdf, "alpha", value(Param::Opacity, Value::Float(m.geometry_opacity)), false)?;
        }
        if m.thin_film_weight > 0.0 {
            self.session.set_float(bsdf, "thin_film_thickness", m.thin_film_thickness * m.thin_film_weight)?;
            self.session.set_float(bsdf, "thin_film_ior", m.thin_film_ior)?;
        }
        if volume {
            self.volume(shader, output, m, driven.contains_key(&Param::TransmissionColor))?;
        }

        let mut normal: Option<(Node, &'static str)> = None;
        // Rounded edges: Cycles' Bevel, the normal the bump starts from as
        // Blender chains them, or the surface's own where there is no bump.
        // A normal map has no normal to start from, so none is rounded.
        let bump = self.scene.bumps.get(index).and_then(|b| b.as_deref());
        let bevel = if m.edge_radius > 0.0 {
            if matches!(bump, Some(BumpSource::NormalMap { .. })) {
                self.out.warn("rounded edges are left out under a normal map in Cycles");
                None
            } else {
                let node = self.session.node(shader, "bevel")?;
                self.session.set_float(node, "Radius", m.edge_radius)?;
                self.session.set_int(node, "Samples", m.edge_samples.round().clamp(1.0, 32.0) as i32)?;
                Some(node)
            }
        } else {
            None
        };
        match bump {
            // Laid down by a projection of its own - a texture pack's
            // tri-planar - and decoded in that projection's frames.
            Some(BumpSource::NormalMap { texture, mapping, .. }) if mapping.projection != TextureProjection::Uv => {
                if let Some((node, socket)) = self.projected_normal(shader, *texture, mapping)? {
                    self.session.connect(shader, node, socket, bsdf, "Normal")?;
                    normal = Some((node, socket));
                }
            }
            Some(BumpSource::NormalMap { texture, mapping, .. }) => {
                if let Some(map) = self.texture_node(shader, *texture, mapping)? {
                    let node = self.session.node(shader, "normal_map")?;
                    self.session.set_string(node, "space", "tangent")?;
                    self.session.set_string(node, "attribute", "UVMap")?;
                    self.session.connect(shader, map.colour.0, map.colour.1, node, "Color")?;
                    self.session.connect(shader, node, "Normal", bsdf, "Normal")?;
                    normal = Some((node, "Normal"));
                }
            }
            // A height in scene units, from the material's program cut down
            // to it. Tequila's renderer tilts the normal away from uphill by
            // the height's slope along the surface, in world units; so does
            // Cycles' bump at a distance and strength of one, its filter
            // width cancelling.
            Some(BumpSource::Height { program, register }) => {
                let registers = self.program(shader, program)?;
                if let Some(height @ Slot::Link(..)) = registers.get(&register.0).copied() {
                    let bump = self.session.node(shader, "bump")?;
                    self.feed(shader, bump, "Height", height, false)?;
                    self.session.set_float(bump, "Distance", 1.0)?;
                    self.session.set_float(bump, "Strength", 1.0)?;
                    if let Some(bevel) = bevel {
                        self.session.connect(shader, bevel, "Normal", bump, "Normal")?;
                    }
                    self.session.connect(shader, bump, "Normal", bsdf, "Normal")?;
                    normal = Some((bump, "Normal"));
                } else if let Some(bevel) = bevel {
                    self.session.connect(shader, bevel, "Normal", bsdf, "Normal")?;
                    normal = Some((bevel, "Normal"));
                }
            }
            None => {
                if let Some(bevel) = bevel {
                    self.session.connect(shader, bevel, "Normal", bsdf, "Normal")?;
                    normal = Some((bevel, "Normal"));
                }
            }
        }
        // And the rotation turns the surface's own tangent, or the one the
        // graph draws.
        if m.specular_roughness_anisotropy > 0.0 || driven.contains_key(&Param::Anisotropy) {
            let tangent = self.anisotropy_tangent(shader, normal, driven.get(&Param::Tangent).copied())?;
            self.session.connect(shader, tangent.0, tangent.1, bsdf, "Tangent")?;
        }
        // Tequila's coat lies on the bumped surface too (`SurfaceFrame`);
        // Cycles' keeps to the smooth one unless told.
        if let Some((node, socket)) = normal {
            self.session.connect(shader, node, socket, bsdf, "coat_normal")?;
        }

        if !thin {
            let surface = if m.flake_weight > 1e-4 { self.flakes(shader, m, (bsdf, "BSDF"))? } else { (bsdf, "BSDF") };
            self.session.connect(shader, surface.0, surface.1, output, "Surface")?;
            return self.session.commit(shader);
        }
        // The shell, as Tequila's renderer has a thin wall: a reflection by
        // Fresnel's share at its index, and the rest straight through, tinted
        // by the square root of the colour at each of the two crossings a
        // closed shell has. Straight through is Cycles' transparent shader,
        // which shadows pass too - as they do through a real pane.
        let glossy = self.session.node(shader, "glossy_bsdf")?;
        self.session.set_string(glossy, "distribution", "ggx")?;
        self.feed(shader, glossy, "Color", value(Param::SpecularColor, Value::Color(m.specular_color)), true)?;
        self.feed(shader, glossy, "Roughness", value(Param::SpecularRoughness, Value::Float(m.specular_roughness)), false)?;
        let fresnel = self.session.node(shader, "fresnel")?;
        // A wall reflects alike from either side. Cycles' Fresnel takes the
        // far side of a surface as glass to air, inverting the index, and
        // past the critical angle reflects everything - so light inside a
        // bulb met a mirror at its back wall, and the rim rendered black.
        // The inverse index there cancels the inversion.
        let ior = value(Param::SpecularIor, Value::Float(m.specular_ior));
        let facing = self.session.node(shader, "geometry")?;
        let pick = self.session.node(shader, "math")?;
        self.session.set_string(pick, "math_type", "multiply_add")?;
        self.session.connect(shader, facing, "Backfacing", pick, "Value1")?;
        match ior {
            Slot::Value(v) => {
                let ior = v.as_float().max(1e-5);
                self.session.set_float(pick, "Value2", 1.0 / ior - ior)?;
                self.session.set_float(pick, "Value3", ior)?;
            }
            link => {
                let inverse = self.session.node(shader, "math")?;
                self.session.set_string(inverse, "math_type", "divide")?;
                self.session.set_float(inverse, "Value1", 1.0)?;
                self.feed(shader, inverse, "Value2", link, false)?;
                let difference = self.session.node(shader, "math")?;
                self.session.set_string(difference, "math_type", "subtract")?;
                self.session.connect(shader, inverse, "Value", difference, "Value1")?;
                self.feed(shader, difference, "Value2", link, false)?;
                self.session.connect(shader, difference, "Value", pick, "Value2")?;
                self.feed(shader, pick, "Value3", link, false)?;
            }
        }
        self.session.connect(shader, pick, "Value", fresnel, "IOR")?;
        if let Some((node, socket)) = normal {
            self.session.connect(shader, node, socket, glossy, "Normal")?;
            self.session.connect(shader, node, socket, fresnel, "Normal")?;
        }
        let reflected: Slot = if m.specular_weight == 1.0 {
            Slot::Link(fresnel, "Fac")
        } else {
            let scale = self.session.node(shader, "math")?;
            self.session.set_string(scale, "math_type", "multiply")?;
            self.session.connect(shader, fresnel, "Fac", scale, "Value1")?;
            self.session.set_float(scale, "Value2", m.specular_weight)?;
            Slot::Link(scale, "Value")
        };
        let colour = value(Param::TransmissionColor, Value::Color(m.transmission_color));
        let surface_tint = value(Param::TransmissionTint, Value::Color(m.transmission_tint));
        let tint = match self.apply(shader, &NodeKind::Multiply, &[colour, surface_tint], 0)? {
            Slot::Value(colour) => {
                let c = colour.as_color();
                Slot::Value(Value::Color(Spectrum { r: c.r.max(0.0).sqrt(), g: c.g.max(0.0).sqrt(), b: c.b.max(0.0).sqrt() }))
            }
            link => {
                let root = self.session.node(shader, "gamma")?;
                self.feed(shader, root, "Color", link, true)?;
                self.session.set_float(root, "Gamma", 0.5)?;
                Slot::Link(root, "Color")
            }
        };
        let roughness = value(Param::SpecularRoughness, Value::Float(m.specular_roughness));
        let through = if matches!(roughness, Slot::Value(r) if r.as_float() <= 0.02) {
            let clear = self.session.node(shader, "transparent_bsdf")?;
            self.feed(shader, clear, "Color", tint, true)?;
            (clear, "BSDF")
        } else {
            self.frosted(shader, tint, roughness)?
        };
        let shell = self.session.node(shader, "mix_closure")?;
        self.feed(shader, shell, "Fac", reflected, false)?;
        self.session.connect(shader, through.0, through.1, shell, "Closure1")?;
        self.session.connect(shader, glossy, "BSDF", shell, "Closure2")?;
        let body = self.session.node(shader, "mix_closure")?;
        self.feed(shader, body, "Fac", transmission, false)?;
        self.session.connect(shader, bsdf, "BSDF", body, "Closure1")?;
        self.session.connect(shader, shell, "Closure", body, "Closure2")?;
        // Opacity last, over the lot: a hole is a hole in the shell too.
        let hole = self.session.node(shader, "transparent_bsdf")?;
        let cut = self.session.node(shader, "mix_closure")?;
        self.feed(shader, cut, "Fac", value(Param::Opacity, Value::Float(m.geometry_opacity)), false)?;
        self.session.connect(shader, hole, "BSDF", cut, "Closure1")?;
        // Flakes in it, as anywhere, over the lot but under the holes.
        let body = if m.flake_weight > 1e-4 { self.flakes(shader, m, (body, "Closure"))? } else { (body, "Closure") };
        self.session.connect(shader, body.0, body.1, cut, "Closure2")?;
        self.session.connect(shader, cut, "Closure", output, "Surface")?;
        self.session.commit(shader)
    }

    /// Light through a rough thin wall, as Tequila's renderer passes it: on
    /// through, blurred by the wall's own reflection lobe mirrored to the far
    /// side (`DielectricTransmission`, thin-walled). Cycles has no such lobe;
    /// its rough refraction, facing the viewer, bends nothing on average and
    /// spreads light by its microfacets - at an index of 30, held on both
    /// faces, almost by their whole tilt, where the mirrored reflection
    /// spreads it by twice the tilt. So the roughness is widened to match:
    /// r (1.45 + 0.4 r), fitted to the quartiles of the exit angle at normal
    /// incidence from roughness 0.1 to 0.5 (capped at 1, past which
    /// Tequila's lobe is wider still).
    ///
    /// And where Tequila's wall passes all it does not reflect, Cycles'
    /// lobe loses some to its microfacets' masking and, aimed along the view
    /// rather than the surface, some to leaving on the viewer's side of the
    /// wall: exp(-0.24 r^4 - 0.8 r^1.5 (1 - mu)^2) of it passes, r the
    /// widened roughness and mu the cosine of the view to the surface,
    /// fitted to a simulation of the lobe to within 0.08. The colour is
    /// brightened by as much.
    fn frosted(&mut self, shader: Shader, tint: Slot, roughness: Slot) -> Result<(Node, &'static str), String> {
        const INDEX: f32 = 30.0;
        let wall = self.session.node(shader, "refraction_bsdf")?;
        self.session.set_string(wall, "distribution", "ggx")?;
        let widened = self.madd(shader, roughness, num(0.4), num(1.45))?;
        let widened = self.mul(shader, widened, roughness)?;
        let widened = self.float_op(shader, "minimum", widened, num(1.0))?;
        self.feed(shader, wall, "Roughness", widened, false)?;
        let facing = self.session.node(shader, "geometry")?;
        // What the lobe loses, made up.
        let r2 = self.mul(shader, widened, widened)?;
        let r4 = self.mul(shader, r2, r2)?;
        let r15 = self.float_op(shader, "power", widened, num(1.5))?;
        let view = self.vector_op(shader, "dot_product", (facing, "Incoming"), Some((facing, "Normal")), None)?;
        let mu = self.unary(shader, "absolute", Slot::Link(view.0, view.1))?;
        let away = self.sub(shader, num(1.0), mu)?;
        let away2 = self.mul(shader, away, away)?;
        let lost = self.mul(shader, r15, num(0.8))?;
        let lost = self.mul(shader, lost, away2)?;
        let lost = self.madd(shader, r4, num(0.24), lost)?;
        let gain = self.unary(shader, "exponent", lost)?;
        let colour = self.session.node(shader, "vector_math")?;
        self.session.set_string(colour, "math_type", "scale")?;
        self.feed(shader, colour, "Vector1", tint, true)?;
        self.feed(shader, colour, "Scale", gain, false)?;
        self.session.connect(shader, colour, "Vector", wall, "Color")?;
        // Cycles takes the far side of a surface as leaving the glass and
        // inverts the index there; the inverse cancels that.
        let index = self.madd(shader, Slot::Link(facing, "Backfacing"), num(1.0 / INDEX - INDEX), num(INDEX))?;
        self.feed(shader, wall, "IOR", index, false)?;
        self.session.connect(shader, facing, "Incoming", wall, "Normal")?;
        Ok((wall, "BSDF"))
    }

    /// Tequila's lights as Cycles lights, at the same brightness.
    ///
    /// Tequila states them physically and Cycles in its own "strength", so
    /// each is converted by what Cycles' kernel does with the number: a point
    /// or spot light's intensity is strength / 4 pi, an area light's radiance
    /// strength / (pi x area).
    fn lights(&mut self) -> Result<(), String> {
        use tequila_render_core::light::LightKind;
        let lights = self.scene.lights.lights();
        if lights.is_empty() {
            return Ok(());
        }
        // One white emitter for them all; each light's colour and power is
        // its strength.
        let shader = self.session.shader("light")?;
        let emission = self.session.node(shader, "emission")?;
        self.session.set_float3(emission, "color", [1.0, 1.0, 1.0])?;
        self.session.set_float(emission, "strength", 1.0)?;
        let output = self.session.node(shader, "output")?;
        self.session.connect(shader, emission, "Emission", output, "Surface")?;
        let shader = self.session.commit(shader)?;

        let place = |p: Vec3| Y_UP_TO_Z_UP.transform_point(p);
        let turn = |v: Vec3| Y_UP_TO_Z_UP.transform_vector(v);
        // A frame whose -Z is `facing`, as Cycles aims spots and area lights.
        let aimed = |position: Vec3, facing: Vec3| -> [f32; 12] {
            let z = -turn(facing).normalize();
            let helper = if z.z.abs() < 0.9 { vec3(0.0, 0.0, 1.0) } else { vec3(1.0, 0.0, 0.0) };
            let x = helper.cross(z).normalize();
            let y = z.cross(x);
            let p = place(position);
            [x.x, y.x, z.x, p.x, x.y, y.y, z.y, p.y, x.z, y.z, z.z, p.z]
        };

        let mut made = Vec::new();
        for (index, light) in lights.iter().enumerate() {
            if !light.enabled {
                continue;
            }
            let v = &light.visibility;
            let mut visibility = ffi::VISIBLE_TRANSMISSION;
            if v.camera {
                visibility |= ffi::VISIBLE_CAMERA;
            }
            // Scattering in a volume goes with diffuse light, as Blender's
            // Volume Scatter visibility is on wherever a lamp lights: left
            // out, Cycles' milky liquids, smoke and frosted media took no
            // light from any lamp, only from the environment.
            if v.diffuse {
                visibility |= ffi::VISIBLE_DIFFUSE | ffi::VISIBLE_SCATTER;
            }
            if v.specular {
                visibility |= ffi::VISIBLE_GLOSSY;
            }
            let four_pi = 4.0 * std::f32::consts::PI;
            match &light.kind {
                LightKind::Point(point) => {
                    let p = place(point.position);
                    let matrix = [1.0, 0.0, 0.0, p.x, 0.0, 1.0, 0.0, p.y, 0.0, 0.0, 1.0, p.z];
                    let node = self.session.light("point", shader, &matrix, visibility)?;
                    self.session.set_float(node, "size", point.radius)?;
                    let s = rgb(point.intensity);
                    self.session.set_float3(node, "strength", [s[0] * four_pi, s[1] * four_pi, s[2] * four_pi])?;
                    self.light_flags(node, v.shadow)?;
                    made.push((index, node));
                }
                LightKind::Spot(spot) => {
                    let node = self.session.light("spot", shader, &aimed(spot.position, spot.direction), visibility)?;
                    let outer = spot.outer_angle.max(1e-3);
                    self.session.set_float(node, "spot_angle", 2.0 * outer)?;
                    self.session.set_float(node, "spot_smooth", ((outer - spot.inner_angle) / outer).clamp(0.0, 1.0))?;
                    let s = rgb(spot.intensity);
                    self.session.set_float3(node, "strength", [s[0] * four_pi, s[1] * four_pi, s[2] * four_pi])?;
                    self.light_flags(node, v.shadow)?;
                    made.push((index, node));
                }
                LightKind::Area(area) => {
                    // Tequila's area light faces cross(u, v); Cycles' its -Z.
                    // X along v and Y along u keeps the frame right-handed
                    // with -Z on the lit side.
                    let normal = area.half_u.cross(area.half_v).normalize();
                    let (u, v_axis) = (turn(area.half_v), turn(area.half_u));
                    let (len_u, len_v) = (2.0 * u.length(), 2.0 * v_axis.length());
                    let (x, y) = (u.normalize(), v_axis.normalize());
                    let z = -turn(normal);
                    let p = place(area.position);
                    let matrix = [x.x, y.x, z.x, p.x, x.y, y.y, z.y, p.y, x.z, y.z, z.z, p.z];
                    let sides: &[f32] = if area.two_sided { &[1.0, -1.0] } else { &[1.0] };
                    for &side in sides {
                        let mut m = matrix;
                        if side < 0.0 {
                            // The back face: the same rectangle turned over.
                            for row in 0..3 {
                                m[row * 4] = -m[row * 4];
                                m[row * 4 + 2] = -m[row * 4 + 2];
                            }
                        }
                        let node = self.session.light("area", shader, &m, visibility)?;
                        self.session.set_float(node, "size", 1.0)?;
                        self.session.set_float(node, "sizeu", len_u)?;
                        self.session.set_float(node, "sizev", len_v)?;
                        self.session.set_bool(node, "ellipse", area.disc)?;
                        let mut a = len_u * len_v;
                        if area.disc {
                            a *= std::f32::consts::FRAC_PI_4;
                        }
                        let k = std::f32::consts::PI * a;
                        let s = rgb(area.radiance);
                        self.session.set_float3(node, "strength", [s[0] * k, s[1] * k, s[2] * k])?;
                        self.light_flags(node, v.shadow)?;
                    made.push((index, node));
                    }
                }
                LightKind::Ies(ies) => {
                    // A point light whose shader reads the profile. Cycles
                    // measures a profile's vertical angle from the light's -Z,
                    // as Tequila does from its aim, and its horizontal angle
                    // as atan2(x, y) + pi where Tequila's is atan2(up, right):
                    // the same angle when the light's X is -up and its Y
                    // -right.
                    let up = ies.direction.cross(ies.right);
                    let (x, y, z) = (turn(-up).normalize(), turn(-ies.right).normalize(), turn(-ies.direction).normalize());
                    let p = place(ies.position);
                    let matrix = [x.x, y.x, z.x, p.x, x.y, y.y, z.y, p.y, x.z, y.z, z.z, p.z];
                    let profiled = self.ies_shader(&ies.profile)?;
                    let node = self.session.light("point", profiled, &matrix, visibility)?;
                    self.session.set_float(node, "size", 0.0)?;
                    // Cycles turns a profile's candela into watts per steradian
                    // at 4 pi / 177.83 and a point light's strength into them
                    // at 1 / 4 pi: so the strength that gives the profile's
                    // own numbers, as Tequila reads them, is 177.83.
                    let s = rgb(ies.intensity);
                    self.session.set_float3(node, "strength", [s[0] * 177.83, s[1] * 177.83, s[2] * 177.83])?;
                    self.light_flags(node, v.shadow)?;
                    made.push((index, node));
                }
            }
        }
        self.link_lights(&made)
    }

    /// Light linking (`LightLinks`), as Cycles keeps it: each part receives
    /// from the set made of the linked lights that light it, and each
    /// linked light is a member of the sets it lights. Set 0 is the parts
    /// every linked light lights; Cycles has 64. Shadow linking the same
    /// way, with blocker sets: each part blocks the lights whose shadow it
    /// casts.
    fn link_lights(&mut self, made: &[(usize, Node)]) -> Result<(), String> {
        self.link_sets(made, false)?;
        self.link_sets(made, true)
    }

    /// One of [`Self::link_lights`]' two halves: the lights' `links`, or
    /// with `shadows` their `shadow_links`.
    fn link_sets(&mut self, made: &[(usize, Node)], shadows: bool) -> Result<(), String> {
        let scene = self.scene;
        let lights = scene.lights.lights();
        let links = |light: usize| if shadows { &lights[light].shadow_links } else { &lights[light].links };
        let linked: Vec<usize> = (0..lights.len()).filter(|&i| !links(i).is_all()).take(64).collect();
        if linked.is_empty() {
            return Ok(());
        }
        let (supported, missing) = if shadows {
            (self.session.has_shadow_linking(), "shadow linking needs a newer Cycles library: here every part casts every light's shadow")
        } else {
            (self.session.has_light_linking(), "light linking needs a newer Cycles library: here every light lights everything")
        };
        if !supported {
            self.out.warn(missing);
            return Ok(());
        }
        let every = if linked.len() == 64 { u64::MAX } else { (1u64 << linked.len()) - 1 };
        let mut sets: Vec<u64> = Vec::new();
        let mut members = Vec::new();
        for (instance, object) in self.out.objects.iter().enumerate() {
            let Some(object) = *object else { continue };
            let mut pattern = 0u64;
            for (bit, &light) in linked.iter().enumerate() {
                if links(light).lights(instance as u32) {
                    pattern |= 1 << bit;
                }
            }
            if pattern == every {
                continue;
            }
            let set = match sets.iter().position(|&p| p == pattern) {
                Some(i) => i + 1,
                None if sets.len() < 63 => {
                    sets.push(pattern);
                    sets.len()
                }
                None => continue,
            };
            members.push((object, set as u32));
        }
        if sets.len() == 63 {
            self.out.warn(if shadows {
                "more than 63 different shadow links: some parts cast every light's shadow"
            } else {
                "more than 63 different light links: some parts are lit by every light"
            });
        }
        for (object, set) in members {
            if shadows {
                self.session.shadow_linking(object, u64::MAX, set)?;
            } else {
                self.session.light_linking(object, u64::MAX, set)?;
            }
        }
        for &(index, node) in made {
            let Some(bit) = linked.iter().position(|&l| l == index) else { continue };
            let mut membership = 1u64;
            for (set, &pattern) in sets.iter().enumerate() {
                if pattern & (1 << bit) != 0 {
                    membership |= 1 << (set + 1);
                }
            }
            if shadows {
                self.session.shadow_linking(node, membership, 0)?;
            } else {
                self.session.light_linking(node, membership, 0)?;
            }
        }
        Ok(())
    }

    /// An emitter as bright in each direction as `profile` says.
    fn ies_shader(&mut self, profile: &tequila_render_core::ies::IesProfile) -> Result<Node, String> {
        let shader = self.session.shader("ies light")?;
        let ies = self.session.node(shader, "ies_light")?;
        self.session.set_string(ies, "ies", &ies_text(profile))?;
        let emission = self.session.node(shader, "emission")?;
        self.session.set_float3(emission, "color", [1.0, 1.0, 1.0])?;
        self.session.connect(shader, ies, "Fac", emission, "Strength")?;
        let output = self.session.node(shader, "output")?;
        self.session.connect(shader, emission, "Emission", output, "Surface")?;
        self.session.commit(shader)
    }

    fn light_flags(&mut self, node: Node, shadow: bool) -> Result<(), String> {
        self.session.set_bool(node, "cast_shadow", shadow)?;
        // As Blender sets them: sampled both from the surface and the light.
        self.session.set_bool(node, "use_mis", true)
    }

    fn world(&mut self, transparent: bool) -> Result<(), String> {
        let environment = &self.scene.environment;
        let shader = self.session.shader("world")?;
        let output = self.session.node(shader, "output")?;

        // Tequila looks the image up at u = atan2(x, -z) / 2pi + 1/2 in its
        // own frame, after turning the direction by the inverse of the
        // environment's rotation. Cycles looks up at
        // u = (pi - atan2(y, x)) / 2pi in its Z-up frame. Both are a turn
        // about the vertical; the difference is a quarter turn plus the
        // environment's own.
        let angle = environment_turn(&environment.rotation());

        let lighting = self.environment_node(shader, environment.image(), angle)?;
        let background = self.session.node(shader, "background")?;
        self.session.set_float(background, "strength", environment.intensity())?;

        let camera_sees: Option<Slot> = match &self.scene.background {
            Background::Environment => match environment.camera_image() {
                Some(image) => Some(self.environment_node(shader, image, angle)?),
                None => None,
            },
            Background::Color(colour) => {
                // Shown as is, so undo the strength the lighting is scaled by.
                let scale = 1.0 / environment.intensity().max(1e-6);
                Some(Slot::Value(Value::Color(*colour * scale)))
            }
            // Laid under the image instead (`Translation::background_under`).
            Background::Backplate { .. } => None,
        };
        match camera_sees {
            None => self.feed(shader, background, "Color", lighting, true)?,
            Some(seen) => {
                let path = self.session.node(shader, "light_path")?;
                let mix = self.session.node(shader, "mix")?;
                self.session.connect(shader, path, "Is Camera Ray", mix, "Fac")?;
                self.feed(shader, mix, "Color1", lighting, true)?;
                self.feed(shader, mix, "Color2", seen, true)?;
                self.session.connect(shader, mix, "Color", background, "Color")?;
            }
        }
        self.session.connect(shader, background, "Background", output, "Surface")?;
        let world = self.session.commit(shader)?;
        self.session.set_world(world);
        // Importance sampling for the world, as Blender sets up for every
        // world it lights with: without it a sun in an HDRI is found only
        // by chance, and a slightly rough surface loses its highlight.
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let light = self.session.light("background", world, &identity, ffi::VISIBLE_ALL & !ffi::VISIBLE_CAMERA)?;
        self.session.set_bool(light, "use_mis", true)?;

        let film = self.session.scene_node("background")?;
        self.session.set_bool(film, "transparent", transparent)?;
        if !environment.dome().is_infinite() {
            self.out.warn("a finite environment dome lights the scene, and shows in reflections, as an infinite one");
        }
        Ok(())
    }

    fn environment_node(&mut self, shader: Shader, image: &std::sync::Arc<tequila_texture::Image>, angle: f32) -> Result<Slot, String> {
        let key = std::sync::Arc::as_ptr(image) as usize;
        let id = match self.images.get(key) {
            Some(id) => id,
            None => {
                // Tequila holds environment pictures in its rendering space
                // (`tequila_scene::environment` converts them as they load),
                // which Cycles works in too. Floats: an HDRI is HDR.
                let id = self.session.image("environment", image.width, image.height, image.channels, &image.data)?;
                self.images.insert(key, Kept::Picture(image.clone()), id);
                id
            }
        };
        let node = self.session.node(shader, "environment_texture")?;
        self.session.set_image(node, id)?;
        let coords = self.session.node(shader, "texture_coordinate")?;
        let map = self.session.node(shader, "mapping")?;
        self.session.set_string(map, "mapping_type", "vector")?;
        self.session.set_float3(map, "rotation", [0.0, 0.0, angle])?;
        self.session.connect(shader, coords, "Generated", map, "Vector")?;
        self.session.connect(shader, map, "Vector", node, "Vector")?;
        Ok(Slot::Link(node, "Color"))
    }

    fn camera(&mut self, view: &View) -> Result<(), String> {
        let camera = &view.camera;
        let aspect = view.width as f32 / view.height.max(1) as f32;
        // Tequila's camera looks down -Z; Cycles' down +Z.
        let flip = Mat4 {
            m: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, -1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        };
        let matrix = Y_UP_TO_Z_UP * camera.camera_to_world * flip;
        let vertical = camera.vertical_fov(aspect);
        // Cycles' angle spans the shorter side.
        let fov = if view.width >= view.height {
            vertical
        } else {
            2.0 * ((vertical * 0.5).tan() * aspect).atan()
        };
        match camera.projection {
            Projection::Perspective { .. } => {}
            other => self.out.warn(format!("{other:?} cameras render as perspective")),
        }
        self.session.camera(&rows(&matrix), fov, view.width, view.height);

        // Every value set every time, off included: the live viewport keeps
        // one camera node for its whole session, so depth of field turned
        // off, or a bladed aperture made round, stayed as it had been.
        let node = self.session.scene_node("camera")?;
        let focused = camera.f_number.is_finite();
        self.session.set_float(node, "aperturesize", if focused { camera.aperture_radius(aspect) } else { 0.0 })?;
        if focused {
            self.session.set_float(node, "focaldistance", camera.focus_distance)?;
        }
        let (blades, rotation) = match camera.aperture {
            Aperture::Bladed { blades, rotation } => (blades as i32, rotation),
            _ => (0, 0.0),
        };
        self.session.set_int(node, "blades", blades)?;
        self.session.set_float(node, "bladesrotation", rotation)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lighting's bounce limits reach Cycles, never below Blender's
    /// defaults: the Jewelry preset's twelve glossy bounces, which a diamond's
    /// inner reflections need, where Cycles had Blender's four whatever the
    /// lighting; the quick preview preset's single bounce leaves Blender's.
    #[test]
    fn the_lighting_sets_the_bounce_limits() {
        use tequila_render_core::settings::IntegratorConfig;
        let jewelry = bounce_limits(&Quality::with_lighting(true, false, None, &IntegratorConfig::jewelry()));
        assert_eq!(jewelry, BounceLimits { total: 28, diffuse: 12, glossy: 12, transmission: 16 });
        let quick = bounce_limits(&Quality::with_lighting(true, false, None, &IntegratorConfig::performance()));
        assert_eq!(quick, BounceLimits { total: 12, diffuse: 4, glossy: 4, transmission: 12 });
    }

    /// What Cycles is told with none of the finer controls set is what it
    /// was always told, and each one set reaches its own socket, as a studio
    /// that has to match Blender's Light Paths, clamping and Film sets them:
    /// total 16 with diffuse 4, glossy 8, transmission 12, transparent 16,
    /// volume 0, Filter Glossy 0.5, direct light unclamped and indirect at
    /// 10, Blackman-Harris 1.5 px.
    #[test]
    fn the_finer_controls_reach_cycles_and_change_nothing_unset() {
        use tequila_render_core::settings::{CyclesTransport, IntegratorConfig, PixelFilter, PixelFilterKind};
        let find = |all: &[(&str, &str, Setting)], node: &str, socket: &str| {
            all.iter().find(|(n, s, _)| *n == node && *s == socket).map(|(_, _, v)| v.clone())
        };
        let plain = settings(&Quality::with_lighting(true, true, None, &IntegratorConfig::product()).with_clamping(true, None, true));
        for (socket, value) in [
            ("max_bounce", Setting::Int(14)),
            ("max_diffuse_bounce", Setting::Int(6)),
            ("max_glossy_bounce", Setting::Int(6)),
            ("max_transmission_bounce", Setting::Int(12)),
            ("transparent_max_bounce", Setting::Int(8)),
            ("transparent_min_bounce", Setting::Int(8)),
            ("filter_glossy", Setting::Float(1.0)),
            ("sample_clamp_direct", Setting::Float(0.0)),
            ("sample_clamp_indirect", Setting::Float(10.0)),
        ] {
            assert_eq!(find(&plain, "integrator", socket), Some(value), "{socket} with nothing set");
        }
        for socket in ["max_volume_bounce", "caustics_reflective", "caustics_refractive", "adaptive_threshold"] {
            assert_eq!(find(&plain, "integrator", socket), None, "{socket} sent with nothing set");
        }
        assert!(plain.iter().all(|(node, _, _)| *node != "film"), "the film was touched with no filter set");
        // An old clamp still holds for direct and indirect light alike.
        let clamped = settings(&Quality::with_lighting(true, true, None, &IntegratorConfig::product()).with_clamping(true, Some(4.0), true));
        assert_eq!(find(&clamped, "integrator", "sample_clamp_direct"), Some(Setting::Float(4.0)));

        let mut config = IntegratorConfig { max_bounces: 16, max_transmission_bounces: 12, ..IntegratorConfig::product() };
        config.cycles = CyclesTransport {
            diffuse_bounces: Some(4),
            glossy_bounces: Some(8),
            transparent_bounces: Some(16),
            volume_bounces: Some(0),
            filter_glossy: Some(0.5),
            caustics_reflective: Some(true),
            caustics_refractive: Some(true),
        };
        let studio = settings(
            &Quality::with_lighting(true, true, None, &config).with_clamping(true, None, true).with_render_controls(
                Some(0.0),
                Some(PixelFilter { kind: PixelFilterKind::BlackmanHarris, width: 1.5 }),
                Some(0.01),
            ),
        );
        for (node, socket, value) in [
            ("integrator", "max_bounce", Setting::Int(16)),
            ("integrator", "max_diffuse_bounce", Setting::Int(4)),
            ("integrator", "max_glossy_bounce", Setting::Int(8)),
            ("integrator", "max_transmission_bounce", Setting::Int(12)),
            ("integrator", "transparent_max_bounce", Setting::Int(16)),
            ("integrator", "max_volume_bounce", Setting::Int(0)),
            ("integrator", "filter_glossy", Setting::Float(0.5)),
            ("integrator", "caustics_reflective", Setting::Bool(true)),
            ("integrator", "caustics_refractive", Setting::Bool(true)),
            ("integrator", "sample_clamp_direct", Setting::Float(0.0)),
            ("integrator", "sample_clamp_indirect", Setting::Float(10.0)),
            ("integrator", "adaptive_threshold", Setting::Float(0.01)),
            ("film", "filter_type", Setting::Text("blackman_harris")),
            ("film", "filter_width", Setting::Float(1.5)),
        ] {
            assert_eq!(find(&studio, node, socket), Some(value), "{node}.{socket}");
        }
        // And Cycles takes every one of them: a socket it does not have is an
        // error. Where Cycles is installed.
        match crate::available() {
            Ok(()) => {
                let mut session = crate::Session::new("CPU", 1, None).expect("a session");
                configure(&mut session, &Quality::with_lighting(true, true, None, &config).with_clamping(true, None, true).with_render_controls(
                    Some(0.0),
                    Some(PixelFilter { kind: PixelFilterKind::Gaussian, width: 1.5 }),
                    Some(0.01),
                ))
                .expect("Cycles takes every setting");
            }
            Err(why) => eprintln!("Cycles skipped: {why}"),
        }
    }

    /// The byte encoding loses nothing it can keep: a value from a byte on
    /// the sRGB curve goes back to that byte.
    #[test]
    fn an_8_bit_texture_goes_back_to_its_own_bytes() {
        for code in 0..=255u8 {
            let linear = tequila_core::spectrum::srgb_eotf(f32::from(code) / 255.0);
            assert_eq!(srgb_byte(linear), code, "code {code} came back different");
        }
    }

    /// A colour texture reaches Cycles in ACEScg, the space Cycles works in:
    /// a saturated Rec.709 colour, read back as Cycles reads the bytes, is
    /// that colour's ACEScg value - not its Rec.709 numbers read as ACEScg,
    /// which would saturate it further. Data goes as it is.
    #[test]
    fn a_colour_texture_arrives_in_acescg() {
        let red = vec3(0.8, 0.1, 0.05);
        let image = tequila_texture::Image {
            width: 1,
            height: 1,
            channels: 3,
            data: vec![red.x, red.y, red.z],
            color_space: tequila_texture::image::ColorSpace::LinearSrgb,
        };
        let want = tequila_core::spectrum::srgb_to_acescg(red);
        let Encoded::Bytes(bytes) = encode_picture(&image, true) else { panic!("an in-range colour goes as bytes") };
        let back: Vec<f32> = bytes.iter().map(|&b| tequila_core::spectrum::srgb_eotf(f32::from(b) / 255.0)).collect();
        for (got, want) in back.iter().zip([want.x, want.y, want.z]) {
            assert!((got - want).abs() < 0.004, "{back:?} came back for {want:?}");
        }
        let Encoded::Bytes(data) = encode_picture(&image, false) else { panic!("in-range data goes as bytes") };
        assert_eq!(data, vec![204, 26, 13], "data was changed");
    }

    /// A colour picture holding pure white still goes as bytes: in f32
    /// white comes out of `srgb_to_acescg` a hair over 1, and every texture
    /// with a single white texel went as floats, at four times the memory -
    /// 256 MB for a 4K one. Past rounding it goes as floats, as an HDRI does.
    #[test]
    fn a_white_colour_texture_goes_as_bytes() {
        let image = tequila_texture::Image {
            width: 2,
            height: 1,
            channels: 4,
            data: vec![1.0, 1.0, 1.0, 1.0, 0.8, 0.1, 0.05, 0.5],
            color_space: tequila_texture::image::ColorSpace::LinearSrgb,
        };
        let Encoded::Bytes(bytes) = encode_picture(&image, true) else { panic!("white went as floats") };
        assert_eq!(&bytes[..4], &[255, 255, 255, 255], "white came out as {bytes:?}");
        let hdr = tequila_texture::Image { data: vec![1.001, 1.0, 1.0, 1.0, 0.8, 0.1, 0.05, 0.5], ..image };
        assert!(matches!(encode_picture(&hdr, true), Encoded::Floats(_)), "over white went as bytes");
        let height = tequila_texture::Image { channels: 1, width: 1, data: vec![1.001], ..hdr };
        assert!(matches!(encode_picture(&height, false), Encoded::Floats(_)), "a height over 1 went as bytes");
    }

    /// A label partly covering reads back in Cycles as its own colour, so
    /// the coverage alone fades it: bytes go multiplied by their coverage on
    /// the sRGB curve, which Cycles divides out before it takes the curve
    /// off (`svm_image_texture`); floats go straight, for the shim to
    /// multiply as Cycles asks. Multiplied in linear beforehand, cream at a
    /// quarter came back several times brighter than the ink itself.
    #[test]
    fn a_part_covering_label_reads_back_as_its_own_colour() {
        let cream = vec3(0.8, 0.7, 0.5);
        let want = tequila_core::spectrum::srgb_to_acescg(cream);
        for coverage in [0.1f32, 0.25, 0.5, 0.75, 1.0] {
            let image = tequila_texture::Image {
                width: 1,
                height: 1,
                channels: 4,
                data: vec![cream.x, cream.y, cream.z, coverage],
                color_space: tequila_texture::image::ColorSpace::LinearSrgb,
            };
            let Encoded::Bytes(bytes) = encode_label(&image, true) else { panic!("an in-range label goes as bytes") };
            // As Cycles reads it: the coverage divided out, then the curve.
            let alpha = f32::from(bytes[3]) / 255.0;
            let back: Vec<f32> = bytes[..3].iter().map(|&b| tequila_core::spectrum::srgb_eotf(f32::from(b) / 255.0 / alpha)).collect();
            for (got, want) in back.iter().zip([want.x, want.y, want.z]) {
                // What it adds to the surface, coverage times colour, to
                // about a level.
                assert!((got - want).abs() * coverage < 0.006, "cream at {coverage} came back {back:?}, not {want:?}");
            }
            let bright = tequila_texture::Image { data: vec![1.5, 1.5, 1.5, coverage], ..image };
            let Encoded::Floats(data) = encode_label(&bright, true) else { panic!("an over-white label goes as floats") };
            assert_eq!(data[3], coverage, "the coverage was changed");
            assert!((data[0] - 1.5).abs() < 1e-3, "a float label went over multiplied: {data:?}");
        }
    }
    use tequila_core::math::Vec3;
    use tequila_core::sampling::direction_to_equirect;
    use std::f32::consts::{PI, TAU};

    /// Cycles' equirectangular u for a direction in its own frame, after the
    /// Mapping node's turn about Z (`svm_mapping`, vector type).
    fn cycles_u(direction: Vec3, turn: f32) -> f32 {
        let (s, c) = turn.sin_cos();
        let x = c * direction.x - s * direction.y;
        let y = s * direction.x + c * direction.y;
        ((PI - y.atan2(x)) / TAU).rem_euclid(1.0)
    }

    /// The same direction finds the same column of the image in both
    /// renderers, at any environment rotation. Got wrong, the sun lights the
    /// product from somewhere else in Cycles than in Tequila.
    #[test]
    fn the_environment_is_turned_the_same_way_in_both() {
        for degrees in [0.0f32, 37.0, 90.0, -140.0, 200.0] {
            let rotation = Mat4::rotation(Vec3::Y, degrees.to_radians());
            let inverse = Mat4::rotation(Vec3::Y, -degrees.to_radians());
            let turn = environment_turn(&rotation);
            for direction in [
                vec3(0.0, 0.0, -1.0),
                vec3(1.0, 0.0, 0.0),
                vec3(0.3, 0.5, 0.8).normalize(),
                vec3(-0.7, -0.2, 0.1).normalize(),
            ] {
                let tequila = direction_to_equirect(inverse.transform_vector(direction)).x;
                let cycles = cycles_u(Y_UP_TO_Z_UP.transform_vector(direction), turn);
                let gap = (tequila - cycles).rem_euclid(1.0);
                assert!(gap.min(1.0 - gap) < 1e-4, "{degrees} degrees, {direction:?}: {tequila} vs {cycles}");
            }
        }
    }

    /// A part moved with the gizmo is a move; a changed material, or a mesh
    /// that was rebuilt, is a new scene that has to be translated again.
    #[test]
    fn a_dragged_part_is_only_a_move() {
        use tequila_core::math::Transform;
        let old = tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2);

        let mut moved = old.clone();
        moved.instances[0].transform = Transform::translation(vec3(0.5, 1.0, 0.0));
        assert!(only_moved(&old, &moved), "a moved part was taken for a new scene");

        let mut repainted = moved.clone();
        repainted.materials[0].base_color = Spectrum { r: 0.1, g: 0.9, b: 0.1 };
        assert!(!only_moved(&old, &repainted), "a material change was taken for a move");

        let mut remeshed = old.clone();
        remeshed.meshes[0] = std::sync::Arc::new((*old.meshes[0]).clone());
        assert!(!only_moved(&old, &remeshed), "a rebuilt mesh was taken for a move");

        assert!(only_moved(&old, &old.clone()), "an unchanged scene was taken for a new one");

        // Every snapshot the application makes builds its lights and its
        // texture table anew, around the same lights and pictures.
        let mut rebuilt = moved.clone();
        rebuilt.lights = std::sync::Arc::new((*old.lights).clone());
        rebuilt.textures = std::sync::Arc::new((*old.textures).clone());
        assert!(only_moved(&old, &rebuilt), "a snapshot rebuilt around the same lights was taken for a new scene");
    }

    /// A changed material, and nothing else, is that material to build again;
    /// with a part moved or a mesh rebuilt as well, it is a new scene.
    #[test]
    fn a_repainted_material_is_only_that_material() {
        use tequila_core::math::Transform;
        let old = tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Plastic, 0.2);

        let mut repainted = old.clone();
        repainted.lights = std::sync::Arc::new((*old.lights).clone());
        repainted.textures = std::sync::Arc::new((*old.textures).clone());
        repainted.materials[0].base_color = Spectrum { r: 0.1, g: 0.9, b: 0.1 };
        assert_eq!(changed_materials(&old, &repainted), Some(vec![0]));
        assert_eq!(changed_materials(&old, &old.clone()), Some(vec![]));

        let mut moved = repainted.clone();
        moved.instances[0].transform = Transform::translation(vec3(0.5, 1.0, 0.0));
        assert_eq!(changed_materials(&old, &moved), None, "a move was taken for a repaint");

        let mut remeshed = repainted.clone();
        remeshed.meshes[0] = std::sync::Arc::new((*old.meshes[0]).clone());
        assert_eq!(changed_materials(&old, &remeshed), None, "a rebuilt mesh was taken for a repaint");

        let mut relit = repainted;
        relit.lights = std::sync::Arc::new(tequila_render_core::light::LightSet::default());
        if !old.lights.lights().is_empty() {
            assert_eq!(changed_materials(&old, &relit), None, "new lighting was taken for a repaint");
        }
    }

    /// Up is up: Tequila's +Y is Cycles' +Z, and the frame keeps its
    /// handedness, so a model is not mirrored on the way across.
    #[test]
    fn y_up_becomes_z_up_without_a_mirror() {
        let up = Y_UP_TO_Z_UP.transform_vector(Vec3::Y);
        assert!((up - Vec3::Z).length() < 1e-6);
        let x = Y_UP_TO_Z_UP.transform_vector(Vec3::X);
        let y = Y_UP_TO_Z_UP.transform_vector(Vec3::Y);
        let z = Y_UP_TO_Z_UP.transform_vector(Vec3::Z);
        assert!((x.cross(y) - z).length() < 1e-6, "the rotation mirrors");
    }
}
