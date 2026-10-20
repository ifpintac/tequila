//! The Solid viewport, drawn by the graphics card.
//!
//! Solid was a ray tracer: every frame traced every pixel on the CPU, shaded
//! it, developed it and uploaded it as a texture. At its best that is tens of
//! milliseconds a frame, and it is why orbiting a model felt nothing like
//! Blender's solid view — which is not traced at all. The GPU rasterizes the
//! triangles, and a few hundred parts cost it well under a millisecond.
//!
//! So this draws the same picture the same way everyone else does. The look is
//! [`tequila_render_core::preview`]'s, carried over term for term — the
//! hemisphere, the camera-fixed three-point rig, the highlight, the rim, the
//! backdrop sweep, the transmission see-through — in the same colour space,
//! through the same sRGB display transform, so switching between the two
//! paths changes the speed and not the image. The CPU path remains, as the
//! fallback when there is no usable OpenGL.
//!
//! Textures: a parameter driven by a texture (or by one channel of one, the
//! packed roughness/metalness maps glTF writes) is sampled on the GPU with the
//! same UV transform. Anything more elaborate — procedurals, tri-planar,
//! chains of nodes — falls back to the material's folded constant, which is
//! what a solid view is for anyway.

use eframe::glow::{self, HasContext};
use tequila_core::math::{Mat4, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{Channel, CompiledGraph, Instruction, NodeKind, Register};
use tequila_materials::material_type::Param;
use tequila_render_core::camera::{Camera, Projection};
use tequila_render_core::preview;
use tequila_render_core::scene::{Mesh, SceneSnapshot};
use tequila_texture::mip::Wrap;
use tequila_texture::projection::{Mapping, Projection as UvProjection};
use tequila_texture::texture::{MapRole, Texture};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

// ---- what a material looks like to the GPU --------------------------------------

/// A UV transform in the order [`Mapping`] applies one: scale, rotate about
/// the pivot, offset, flip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform {
    pub scale: [f32; 2],
    pub pivot: [f32; 2],
    pub offset: [f32; 2],
    /// Radians.
    pub rotation: f32,
    pub flip: [bool; 2],
    pub wrap: [Wrap; 2],
}

impl UvTransform {
    fn of(mapping: &Mapping) -> Option<UvTransform> {
        // Only what a vertex attribute can carry: the first UV set, in UV
        // projection. Real-world units scale by the inverse, as `finish` does.
        if mapping.projection != UvProjection::Uv || mapping.uv_set != 0 {
            return None;
        }
        let (su, sv) = if mapping.real_world_units {
            (1.0 / mapping.scale.x.abs().max(1e-6), 1.0 / mapping.scale.y.abs().max(1e-6))
        } else {
            (mapping.scale.x, mapping.scale.y)
        };
        Some(UvTransform {
            scale: [su, sv],
            pivot: [mapping.pivot.x, mapping.pivot.y],
            offset: [mapping.offset.x, mapping.offset.y],
            rotation: mapping.rotation * std::f32::consts::TAU,
            flip: [mapping.flip_u, mapping.flip_v],
            wrap: [mapping.wrap_u, mapping.wrap_v],
        })
    }

    /// The same arithmetic as `Mapping::finish_anisotropic`, for tests.
    #[cfg(test)]
    fn apply(&self, u: f32, v: f32) -> (f32, f32) {
        let (mut u, mut v) = (u * self.scale[0], v * self.scale[1]);
        if self.rotation != 0.0 {
            let (sin, cos) = self.rotation.sin_cos();
            let (pu, pv) = (u - self.pivot[0], v - self.pivot[1]);
            u = pu * cos - pv * sin + self.pivot[0];
            v = pu * sin + pv * cos + self.pivot[1];
        }
        u += self.offset[0];
        v += self.offset[1];
        if self.flip[0] {
            u = -u;
        }
        if self.flip[1] {
            v = -v;
        }
        (u, v)
    }
}

/// Where a parameter's value comes from.
#[derive(Clone, Debug)]
pub enum Source {
    /// The material's folded constant.
    Constant,
    /// A texture lookup, whole (a colour) or one channel of it (a scalar).
    Texture { texture: Arc<Texture>, channel: Option<usize>, uv: UvTransform },
}

// Textures compare by identity: two uploads of one file are one texture.
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Source::Constant, Source::Constant) => true,
            (
                Source::Texture { texture: a, channel: ca, uv: ua },
                Source::Texture { texture: b, channel: cb, uv: ub },
            ) => Arc::ptr_eq(a, b) && ca == cb && ua == ub,
            _ => false,
        }
    }
}

/// A material, reduced to what the Solid shader reads and where each value
/// comes from.
#[derive(Clone, Debug)]
pub struct SolidMaterial {
    /// Rendering space (ACEScg).
    pub base_color: Spectrum,
    pub base_weight: f32,
    pub metalness: f32,
    pub specular_weight: f32,
    pub roughness: f32,
    pub coat_weight: f32,
    pub coat_roughness: f32,
    pub transmission: f32,
    pub transmission_color: Spectrum,
    pub ior: f32,
    /// What a cloudy volume sends back, before lighting.
    pub cloud: Spectrum,
    /// A Volume Map's grid, where one drives the medium's Density: the view
    /// marches it through the part, so a VDB shows its shape here as it
    /// does in the render.
    pub volume: Option<Arc<tequila_texture::volume::VolumeGrid>>,
    /// The medium's extinction per unit length at density 1, as one number,
    /// and the colour its scattering sends back, for the march.
    pub volume_extinction: f32,
    pub volume_albedo: Spectrum,
    pub emission: Spectrum,
    pub base_color_source: Source,
    pub metalness_source: Source,
    pub roughness_source: Source,
    /// A tangent-space normal map wired straight into Bump, as the ray
    /// tracer reads one.
    pub normal_source: Source,
    /// The material's graph written out for the graphics card: its
    /// patterns, and heights through a Bump node, which a shader variant
    /// computes per pixel (`procedural_gpu`).
    pub procedural: Option<Arc<crate::procedural_gpu::Generated>>,
    /// Its labels, bottom to top: the top [`MAX_LABELS`].
    pub labels: Vec<SolidLabel>,
}

/// How many labels a part shows in Solid and Material Preview; the
/// render shows them all.
pub const MAX_LABELS: usize = 4;
/// The texture units the labels' artwork is bound to, from this one up.
const LABEL_UNIT: u32 = 8;
const LABEL_SAMPLERS: [&str; MAX_LABELS] = ["t_label0", "t_label1", "t_label2", "t_label3"];

/// A label as the Solid shader lays one over its part. Labels were drawn
/// only in the render, so a label could be placed, sized and turned only by
/// rendering to see where it went.
#[derive(Clone, Debug)]
pub struct SolidLabel {
    pub texture: Arc<Texture>,
    /// Projection (0 UV, 1 planar, 2 cylindrical, 3 spherical), axis (x, y,
    /// z), sized in scene units, colour-managed artwork.
    pub mode: [i32; 4],
    /// Scale, pivot, offset, rotation and flips, as `transform_uv` takes them.
    pub uv: [f32; 8],
    pub opacity: f32,
    pub roughness: f32,
    pub metallic: bool,
}

impl SolidLabel {
    /// `None` for a label the shader cannot place: tri-planar, or a second
    /// UV set.
    fn of(decal: &tequila_materials::decal::Decal, texture: Arc<Texture>) -> Option<SolidLabel> {
        let m = &decal.mapping;
        let projection = match m.projection {
            UvProjection::Uv if m.uv_set == 0 => 0,
            UvProjection::Planar => 1,
            UvProjection::Cylindrical => 2,
            UvProjection::Spherical => 3,
            _ => return None,
        };
        let axis = match m.axis {
            tequila_texture::projection::Axis::X => 0,
            tequila_texture::projection::Axis::Y => 1,
            tequila_texture::projection::Axis::Z => 2,
        };
        let (su, sv) = if m.real_world_units {
            (1.0 / m.scale.x.abs().max(1e-6), 1.0 / m.scale.y.abs().max(1e-6))
        } else {
            (m.scale.x, m.scale.y)
        };
        let flips = m.flip_u as i32 as f32 + 2.0 * m.flip_v as i32 as f32;
        let colour_managed = texture.role == MapRole::Color && texture.source_color_space != tequila_texture::image::ColorSpace::Raw;
        Some(SolidLabel {
            mode: [projection, axis, (m.real_world_units && projection != 0) as i32, colour_managed as i32],
            uv: [su, sv, m.pivot.x, m.pivot.y, m.offset.x, m.offset.y, m.rotation * std::f32::consts::TAU, flips],
            opacity: decal.opacity.clamp(0.0, 1.0),
            roughness: decal.roughness.clamp(0.0, 1.0),
            metallic: decal.metallic,
            texture,
        })
    }
}

/// A material's labels the shader can draw, the top [`MAX_LABELS`].
fn labels_of(program: &CompiledGraph, scene: &SceneSnapshot) -> Vec<SolidLabel> {
    let mut labels: Vec<SolidLabel> = program
        .decals
        .iter()
        .filter(|d| d.enabled && d.opacity > 0.0)
        .filter_map(|d| SolidLabel::of(d, scene.textures.get(d.texture)?.clone()))
        .collect();
    let extra = labels.len().saturating_sub(MAX_LABELS);
    labels.drain(..extra);
    labels
}

fn source(program: &CompiledGraph, scene: &SceneSnapshot, param: Param, scalar: bool) -> Source {
    let Some((_, register)) = program.driven.iter().find(|(p, _)| *p == param) else {
        return Source::Constant;
    };
    let writer = |register: Register| program.instructions.iter().rev().find(|i| i.destination() == register);
    let (sample, channel) = match writer(*register) {
        Some(sample @ Instruction::Sample { .. }) => (sample, None),
        Some(Instruction::Apply { kind, args, .. }) if args.len() == 1 => match kind.as_ref() {
            NodeKind::Channel { channel } => match writer(args[0]) {
                Some(sample @ Instruction::Sample { .. }) => (
                    sample,
                    Some(match channel {
                        Channel::Red => 0,
                        Channel::Green => 1,
                        Channel::Blue => 2,
                    }),
                ),
                _ => return Source::Constant,
            },
            _ => return Source::Constant,
        },
        _ => return Source::Constant,
    };
    // A scalar driven straight from a colour texture is read by luminance on
    // the CPU; the red channel is the closest single one, and exact for the
    // grey maps that case almost always is.
    let channel = if scalar { Some(channel.unwrap_or(0)) } else { channel };
    let Instruction::Sample { texture, mapping, .. } = sample else { unreachable!() };
    match (scene.textures.get(*texture), UvTransform::of(mapping)) {
        (Some(texture), Some(uv)) => Source::Texture { texture: texture.clone(), channel, uv },
        _ => Source::Constant,
    }
}

/// A medium's extinction per unit length at density 1, grey.
fn volume_extinction(material: &tequila_render_core::material::Material) -> f32 {
    let unit = tequila_render_core::material::Material { volume_density: 1.0, ..material.clone() };
    match unit.volume_scattering() {
        Some(medium) => medium.extinction.luminance(),
        None if unit.transmission_depth > 0.0 => {
            let c = unit.transmission_color.luminance().clamp(1e-4, 1.0);
            -c.ln() / unit.transmission_depth
        }
        None => 0.0,
    }
}

/// The colour a medium's scattering sends back: its single-scattering
/// albedo, which is black for a medium that only absorbs.
fn volume_albedo(material: &tequila_render_core::material::Material) -> Spectrum {
    let unit = tequila_render_core::material::Material { volume_density: 1.0, ..material.clone() };
    unit.volume_scattering().map_or(Spectrum::ZERO, |medium| medium.albedo)
}

impl SolidMaterial {
    pub fn of(scene: &SceneSnapshot, index: usize) -> SolidMaterial {
        let program = scene.programs.get(index).and_then(|p| p.as_ref());
        let constants = match program {
            Some(program) => &program.constants,
            None => &scene.materials[index],
        };
        let sources = |param, scalar| match program {
            Some(program) => source(program, scene, param, scalar),
            None => Source::Constant,
        };
        SolidMaterial {
            // A metal as the colour it is seen to be, not its white tint.
            base_color: constants.solid_color(),
            base_weight: constants.base_weight,
            metalness: constants.base_metalness,
            specular_weight: constants.specular_weight,
            roughness: constants.specular_roughness,
            coat_weight: constants.coat_weight,
            coat_roughness: constants.coat_roughness,
            transmission: constants.transmission_weight,
            transmission_color: constants.see_through_tint(),
            ior: constants.specular_ior,
            cloud: constants.preview_cloud(),
            volume: program
                .and_then(|p| tequila_render_core::density::volume_of(p))
                .and_then(|volume| scene.textures.volume(volume).cloned()),
            volume_extinction: volume_extinction(constants),
            volume_albedo: volume_albedo(constants),
            emission: constants.emission(),
            base_color_source: sources(Param::BaseColor, false),
            metalness_source: sources(Param::BaseMetalness, true),
            roughness_source: sources(Param::SpecularRoughness, true),
            normal_source: match scene.bump(tequila_core::ids::MaterialId(index as u32)) {
                Some(tequila_render_core::surface_bump::BumpSource::NormalMap { .. }) => sources(Param::Bump, false),
                _ => Source::Constant,
            },
            procedural: program.and_then(|p| crate::procedural_gpu::generate(p)).map(Arc::new),
            labels: program.map(|p| labels_of(p, scene)).unwrap_or_default(),
        }
    }
}

// ---- what one frame draws ----------------------------------------------------------

/// Everything a frame needs, gathered on the UI thread and handed to the
/// paint callback.
pub struct Frame {
    pub scene: Arc<SceneSnapshot>,
    pub camera: Camera,
    /// Instance ids to ring, as the CPU path's outline does.
    pub selected: Vec<u32>,
    pub pixels_per_point: f32,
    pub mode: GpuMode,
    /// The document's exposure, as a factor, for Material Preview.
    pub exposure: f32,
    /// Draw the parts' own edges over the shading: a CAD part's B-rep
    /// edges, or the creases and borders of a mesh that has none.
    pub model_edges: bool,
}

/// What the GPU view draws, as Blender's viewport shading offers it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuMode {
    /// Every edge of every mesh, the hidden ones faint: the structure of a
    /// model, seen through itself.
    Wireframe,
    /// Flat studio shading: form and colour, at once.
    #[default]
    Solid,
    /// Materials as they are, lit by the scene's own environment: metals
    /// reflect it, glass lets it through, roughness blurs it - fast, and a
    /// close guess at the render.
    MaterialPreview,
}

// ---- the GL side ------------------------------------------------------------------

struct GpuMesh {
    /// The mesh's own bounding box, which a Volume Map is fitted to.
    bounds: (Vec3, Vec3),
    vao: glow::VertexArray,
    buffers: Vec<glow::Buffer>,
    count: i32,
    indexed: bool,
    /// Every edge once, as line pairs, for the wireframe.
    edge_vao: glow::VertexArray,
    edge_count: i32,
    /// The part's own edges, positions in pairs, for Model edges.
    model_vao: glow::VertexArray,
    model_count: i32,
    /// Held so the pointer the map is keyed by cannot be reused by a new mesh.
    _mesh: Arc<Mesh>,
}

struct GpuTexture {
    texture: glow::Texture,
    _source: Arc<Texture>,
}

struct GpuVolume {
    texture: glow::Texture,
    _source: Arc<tequila_texture::volume::VolumeGrid>,
}

struct Programs {
    surface: glow::Program,
    backdrop: glow::Program,
    backdrop_vao: glow::VertexArray,
    lines: glow::Program,
    /// The ground shadow's three passes: the model from above, a blur, and
    /// the shadow laid on the ground.
    shadow_caster: glow::Program,
    shadow_blur: glow::Program,
    shadow_ground: glow::Program,
}

/// Where Material Preview's ground shadow lies, and how it falls off.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ShadowFootprint {
    /// The square of ground it covers: its centre in x and z, and half its
    /// side.
    centre: (f32, f32),
    half: f32,
    /// The ground's height.
    plane: f32,
    /// How far above the ground a surface stops darkening it: the broad
    /// shadow of the light from overhead, and the tight dark of contact.
    falloff: f32,
    contact_falloff: f32,
    /// How dark each makes the ground at most, 0 to 1.
    strength: f32,
    contact_strength: f32,
    /// How far the broad shadow leans, in x and z, per unit of height: away
    /// from where the environment's light comes from.
    skew: (f32, f32),
}

/// The ground under the model, darkened where the model is close to it: a
/// contact shadow, as the render's floor catches one.
///
/// The model is drawn once from straight above into a small map — each texel
/// keeping how near to the ground the model comes over it — and blurred.
/// That is redone only when the scene changes; each frame just lays the map
/// on the ground. It is not the render's shadow, which depends on where the
/// light comes from, but it puts the model on the floor rather than floating
/// over it, which is most of what a shadow does for a preview.
struct GpuShadow {
    /// The blurred map, and the target the blur bounces through.
    textures: [glow::Texture; 2],
    framebuffers: [glow::Framebuffer; 2],
    footprint: ShadowFootprint,
    /// What it was drawn from: [`shadow_source`].
    source: u64,
}

/// The shadow map's side, in texels.
const SHADOW_SIZE: i32 = 512;

/// The scene's environment as Material Preview lights with it: radiance
/// over the sphere as a mipmapped equirectangular map, blurrier mip by mip
/// for rougher reflections, and its irradiance as nine spherical-harmonic
/// coefficients for the diffuse light.
struct GpuEnvironment {
    texture: glow::Texture,
    levels: f32,
    irradiance: [f32; 27],
    /// What the camera sees behind the model, as the render shows it: the
    /// environment's backdrop over the sphere, or a backplate over the
    /// frame (`background_on_film`).
    background: glow::Texture,
    background_on_film: bool,
    /// The environment and background it was made from.
    source: u64,
}

#[derive(Default)]
struct Gpu {
    programs: Option<Programs>,
    meshes: HashMap<usize, GpuMesh>,
    textures: HashMap<usize, GpuTexture>,
    /// Labels' artwork, premultiplied and clear past its edges: kept apart
    /// from `textures`, which hold the same picture as it is.
    label_textures: HashMap<usize, GpuTexture>,
    volumes: HashMap<usize, GpuVolume>,
    /// The scene the materials were analysed for, and the result.
    materials_for: Option<Arc<SceneSnapshot>>,
    materials: Vec<SolidMaterial>,
    frame: Option<Frame>,
    environment: Option<GpuEnvironment>,
    shadow: Option<GpuShadow>,
    /// Set when the ground shadow could not be drawn on this machine: the
    /// preview goes on without it rather than falling back to the CPU.
    shadow_failed: bool,
    /// The surface shader with a material's program in it, by a hash of
    /// that program's source; `None` where the driver would not compile it,
    /// so the material draws with the plain shader rather than retrying.
    variants: HashMap<u64, Option<glow::Program>>,
    /// Each material's program, worked out once per scene: finding it
    /// generates and hashes the material's whole shader source, which per
    /// material per frame was work for nothing. `None` until worked out.
    material_programs: Option<Vec<Option<glow::Program>>>,
    /// Uniform locations, by program and name. See [`Gpu::uniform`].
    uniforms: std::cell::RefCell<HashMap<(glow::Program, &'static str), Option<glow::UniformLocation>>>,
}

/// The GPU Solid view. Cheap to clone into a paint callback.
#[derive(Clone)]
pub struct SolidView {
    gpu: Arc<Mutex<Gpu>>,
    failed: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
}

impl SolidView {
    pub fn new(has_gl: bool) -> SolidView {
        SolidView {
            gpu: Arc::new(Mutex::new(Gpu::default())),
            failed: Arc::new(AtomicBool::new(!has_gl)),
            error: Arc::new(Mutex::new(if has_gl { None } else { Some("no OpenGL context".into()) })),
        }
    }

    /// Whether to draw Solid on the GPU. False after a failure, and the CPU
    /// path takes over.
    pub fn usable(&self) -> bool {
        !self.failed.load(Ordering::Acquire)
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    /// Queue a frame and return the callback that draws it.
    pub fn paint(&self, rect: egui::Rect, frame: Frame) -> egui::PaintCallback {
        self.gpu.lock().unwrap().frame = Some(frame);
        let view = self.clone();
        egui::PaintCallback {
            rect,
            callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |info, painter| {
                view.draw(painter.gl(), &info);
            })),
        }
    }

    fn draw(&self, gl: &glow::Context, info: &egui::PaintCallbackInfo) {
        if !self.usable() {
            return;
        }
        let mut gpu = self.gpu.lock().unwrap();
        // TEQUILA_GPU_TIMING: how long the graphics card takes over a
        // frame, waited for, to stderr - for weighing a heavy material.
        let timing = std::env::var_os("TEQUILA_GPU_TIMING").is_some();
        if timing {
            unsafe { gl.finish() };
        }
        let started = std::time::Instant::now();
        if let Err(error) = unsafe { gpu.draw(gl, info) } {
            *self.error.lock().unwrap() = Some(error);
            self.failed.store(true, Ordering::Release);
        }
        if timing {
            unsafe { gl.finish() };
            eprintln!("gpu view frame {:.2} ms", started.elapsed().as_secs_f64() * 1000.0);
        }
        unsafe { restore(gl) };
    }
}

unsafe fn restore(gl: &glow::Context) {
    gl.disable(glow::DEPTH_TEST);
    gl.disable(glow::CULL_FACE);
    gl.depth_mask(true);
    gl.bind_vertex_array(None);
    gl.active_texture(glow::TEXTURE0);
    gl.use_program(None);
}

const SURFACE_VERTEX: &str = r#"
layout(location = 0) in vec3 a_position;
layout(location = 1) in vec3 a_normal;
layout(location = 2) in vec2 a_uv;
layout(location = 3) in vec4 a_tangent;
uniform mat4 u_model;
uniform mat3 u_normal_matrix;
uniform mat4 u_view_projection;
uniform float u_outline;
uniform vec2 u_viewport;
out vec3 v_world;
out vec3 v_normal;
out vec2 v_uv;
out vec4 v_tangent;
out vec3 v_object;
out vec3 v_object_normal;
void main() {
    vec4 world = u_model * vec4(a_position, 1.0);
    v_world = world.xyz;
    v_object = a_position;
    v_object_normal = a_normal;
    v_normal = normalize(u_normal_matrix * a_normal);
    v_uv = a_uv;
    // A direction along the surface: it moves with the model matrix, not
    // the normal matrix. Zero when the mesh has no tangents.
    v_tangent = vec4(mat3(u_model) * a_tangent.xyz, a_tangent.w);
    vec4 clip = u_view_projection * world;
    if (u_outline > 0.0) {
        // Pushed out along the normal in screen space, by a fixed number of
        // pixels: an outline that keeps its width at any distance.
        vec4 n = u_view_projection * vec4(v_normal, 0.0);
        vec2 direction = n.xy;
        float length_ = length(direction);
        if (length_ > 1e-6) {
            clip.xy += direction / length_ * u_outline * 2.0 / u_viewport * clip.w;
        }
    }
    gl_Position = clip;
}
"#;

const SURFACE_FRAGMENT: &str = r#"
in vec3 v_world;
in vec3 v_normal;
in vec2 v_uv;
in vec4 v_tangent;
in vec3 v_object;
in vec3 v_object_normal;
out vec4 out_colour;

// What a material's patterns read of the part (`procedural_gpu`): scene
// units per mesh unit, its random number, where it is, and its mesh's
// bounding box for generated coordinates.
uniform float u_object_scale;
uniform float u_object_random;
uniform vec3 u_object_location;
uniform vec3 u_generated_centre;
uniform vec3 u_generated_half;

uniform vec3 u_eye;
uniform vec3 u_forward;
uniform int u_orthographic;
uniform vec3 u_key;
uniform vec3 u_fill;
uniform vec3 u_rim;
uniform vec3 u_sky;
uniform vec3 u_ground;
uniform vec3 u_backdrop_top;
uniform vec3 u_backdrop_bottom;
uniform vec4 u_viewport_px;
uniform mat3 u_srgb_to_ap1;
uniform mat3 u_ap1_to_srgb;

uniform vec3 u_base;
uniform float u_base_weight;
uniform float u_metalness;
uniform float u_specular_weight;
uniform float u_roughness;
uniform float u_coat_weight;
uniform float u_coat_roughness;
uniform float u_transmission;
uniform vec3 u_transmission_colour;
uniform vec3 u_cloud;
uniform vec3 u_emission;

// A Volume Map: the grid as a 3D texture over the part's own bounding box,
// marched from the surface through the box.
uniform int u_volume;
uniform sampler3D t_volume;
uniform mat4 u_model_inverse;
uniform vec3 u_object_low;
uniform vec3 u_object_size;
uniform float u_volume_extinction;
uniform vec3 u_volume_albedo;

uniform sampler2D t_base;
uniform sampler2D t_metal;
uniform sampler2D t_rough;
uniform sampler2D t_normal;
uniform int u_normal_texture;
uniform vec4 u_normal_uv[2];
uniform int u_base_texture;
uniform int u_metal_channel;
uniform int u_rough_channel;
uniform vec4 u_base_uv[2];
uniform vec4 u_metal_uv[2];
uniform vec4 u_rough_uv[2];

uniform int u_outline_pass;
uniform vec3 u_outline_colour;

// Material Preview: the scene's environment, and the display's exposure.
uniform int u_mode;
uniform sampler2D t_environment;
uniform float u_environment_levels;
uniform vec3 u_irradiance[9];
uniform float u_exposure;
uniform float u_ior;

//@MATERIAL@

// Cutaways (`docs/02` §3): up to eight analytic cutters, each as the matrix
// into its own space and its shape. A fragment inside one is not drawn. A
// mesh cutter is the renderer's alone and is not listed here.
const int MAX_CUTTERS = 8;
uniform int u_cutter_count;
uniform mat4 u_cutter_inverse[MAX_CUTTERS];
uniform int u_cutter_shape[MAX_CUTTERS];
// rgb and 1 for a cap colour of its own; w 0 caps in the part's colour.
uniform vec4 u_cutter_cap[MAX_CUTTERS];

bool inside_cutter(int shape, vec3 p) {
    if (shape == 0) return p.z > 0.0;
    if (shape == 1) return all(lessThan(abs(p), vec3(1.0)));
    if (shape == 2) return dot(p, p) < 1.0;
    if (shape == 3) return dot(p.xy, p.xy) < 1.0 && abs(p.z) < 1.0;
    return false;
}

vec2 transform_uv(vec2 uv, vec4 a, vec4 b) {
    // a = (scale.u, scale.v, pivot.u, pivot.v); b = (offset.u, offset.v, rotation, flips)
    vec2 p = uv * a.xy;
    if (b.z != 0.0) {
        vec2 q = p - a.zw;
        float s = sin(b.z);
        float c = cos(b.z);
        p = vec2(q.x * c - q.y * s, q.x * s + q.y * c) + a.zw;
    }
    p += b.xy;
    float flips = b.w;
    if (mod(flips, 2.0) >= 1.0) p.x = -p.x;
    if (flips >= 2.0) p.y = -p.y;
    return p;
}

float channel_of(vec4 texel, int channel) {
    if (channel == 0) return texel.r;
    if (channel == 1) return texel.g;
    return texel.b;
}

vec3 encode_srgb(vec3 linear) {
    vec3 c = clamp(linear, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

vec3 backdrop(float film_y) {
    return mix(u_backdrop_top, u_backdrop_bottom, clamp(film_y, 0.0, 1.0));
}

const float PI = 3.14159265;

// The environment map's own projection: what made it (`environment_map`)
// and what reads it agree, and nothing else needs to.
vec2 equirect(vec3 d) {
    return vec2(atan(d.x, d.z) / (2.0 * PI) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI);
}

vec3 environment(vec3 d, float roughness) {
    float lod = clamp(sqrt(roughness) * (u_environment_levels - 1.0), 0.0, u_environment_levels - 1.0);
    return textureLod(t_environment, equirect(d), lod).rgb;
}

// Irradiance from nine spherical-harmonic coefficients (Ramamoorthi and
// Hanrahan), as cosine-convolved radiance: divide by pi for a Lambertian
// surface's outgoing radiance.
vec3 irradiance(vec3 n) {
    return u_irradiance[0] * 0.886227
        + u_irradiance[1] * (1.023328 * n.y)
        + u_irradiance[2] * (1.023328 * n.z)
        + u_irradiance[3] * (1.023328 * n.x)
        + u_irradiance[4] * (0.858086 * n.x * n.y)
        + u_irradiance[5] * (0.858086 * n.y * n.z)
        + u_irradiance[6] * (0.247708 * (3.0 * n.z * n.z - 1.0))
        + u_irradiance[7] * (0.858086 * n.x * n.z)
        + u_irradiance[8] * (0.429043 * (n.x * n.x - n.y * n.y));
}

// The split-sum's directional albedo, by Karis's analytic fit: how much of
// a GGX lobe's reflection comes back, as scale and bias on F0.
vec2 environment_brdf(float roughness, float n_dot_v) {
    const vec4 c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
    const vec4 c1 = vec4(1.0, 0.0425, 1.04, -0.04);
    vec4 r = roughness * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * n_dot_v)) * r.x + r.y;
    return vec2(-1.04, 1.04) * a004 + r.zw;
}

// Khronos PBR Neutral, the renderer's default display transform, on linear
// sRGB.
vec3 pbr_neutral(vec3 color) {
    const float start = 0.8 - 0.04;
    const float desaturation = 0.15;
    float x = min(color.r, min(color.g, color.b));
    float offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
    color -= offset;
    float peak = max(color.r, max(color.g, color.b));
    if (peak < start) return color;
    const float d = 1.0 - start;
    float new_peak = 1.0 - d * d / (peak + d - start);
    color *= new_peak / peak;
    float g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, vec3(new_peak), g);
}

vec3 display(vec3 rendering) {
    return encode_srgb(pbr_neutral(max(u_ap1_to_srgb * rendering * u_exposure, vec3(0.0))));
}

// Labels (`docs/01` §1.11): up to four, bottom to top, each its artwork
// premultiplied by its coverage and clear past its edges, placed as the
// renderer's `Mapping::project` places it and laid over the surface as its
// `shade` lays it - colour by coverage, foil as metal, its own roughness.
uniform int u_label_count;
uniform sampler2D t_label0;
uniform sampler2D t_label1;
uniform sampler2D t_label2;
uniform sampler2D t_label3;
// projection (0 UV, 1 planar, 2 cylindrical, 3 spherical), axis (x, y, z),
// sized in scene units, colour-managed artwork.
uniform ivec4 u_label_mode[4];
// Two each, as `transform_uv` takes them.
uniform vec4 u_label_uv[8];
// opacity, roughness, foil.
uniform vec4 u_label_finish[4];

vec2 label_frame(int i) {
    ivec4 mode = u_label_mode[i];
    if (mode.x == 0) return v_uv;
    vec3 p = mode.z == 1 ? v_object * u_object_scale : v_object;
    // The two components across the axis, then the one along it.
    vec3 q = mode.y == 0 ? p.yzx : (mode.y == 1 ? p.xzy : p);
    if (mode.x == 1) return q.xy;
    if (mode.x == 2) return vec2(atan(q.y, q.x) / (2.0 * PI) + 0.5, q.z);
    float r = length(p);
    if (r < 1e-6) return vec2(0.0, 0.5);
    vec3 d = p / r;
    return vec2(atan(d.z, d.x) / (2.0 * PI) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI);
}

void lay_label(sampler2D artwork, int i, inout vec3 base, inout float metal, inout float rough, inout float transmission) {
    vec4 texel = texture(artwork, transform_uv(label_frame(i), u_label_uv[2 * i], u_label_uv[2 * i + 1]));
    vec3 colour = u_label_mode[i].w == 1 ? u_srgb_to_ap1 * texel.rgb : texel.rgb;
    float opacity = u_label_finish[i].x;
    float alpha = clamp(texel.a, 0.0, 1.0) * opacity;
    base = base * (1.0 - alpha) + colour * opacity;
    metal = metal * (1.0 - alpha) + (u_label_finish[i].z > 0.5 ? alpha : 0.0);
    rough = rough * (1.0 - alpha) + u_label_finish[i].y * alpha;
    transmission *= 1.0 - alpha;
}

void main() {
    for (int i = 0; i < u_cutter_count; i++) {
        if (inside_cutter(u_cutter_shape[i], (u_cutter_inverse[i] * vec4(v_world, 1.0)).xyz)) discard;
    }
    if (u_outline_pass == 1) {
        out_colour = vec4(u_outline_colour, 1.0);
        return;
    }
    vec3 view = u_orthographic == 1 ? -u_forward : normalize(u_eye - v_world);
    vec3 normal = normalize(v_normal);
    // The inside of a solid, seen where a cutter has opened it. The renderer
    // draws the cap at the cut itself; here it is the far wall, flat-lit in
    // the cap's colour, which reads the same from the front and costs
    // nothing.
    if (u_cutter_count > 0 && dot(normal, view) < 0.0) {
        vec3 cap = u_base;
        for (int i = 0; i < u_cutter_count; i++) {
            if (u_cutter_cap[i].w > 0.5) { cap = u_cutter_cap[i].rgb; break; }
        }
        vec3 lit = cap * (mix(u_ground, u_sky, 0.75) * 0.62 + vec3(0.42 * max(dot(view, u_key), 0.0) + 0.14));
        out_colour = vec4(encode_srgb(u_ap1_to_srgb * lit), 1.0);
        return;
    }
    // A medium whose density a Volume Map varies: march the grid through
    // the part along the view, and let the backdrop through by what it does
    // not stop, lit where it scatters. What the preview does for a uniform
    // cloudy volume, with the density read along the way.
    if (u_volume == 1) {
        vec3 dir = -view;
        vec3 o = (u_model_inverse * vec4(v_world, 1.0)).xyz;
        vec3 d = (u_model_inverse * vec4(dir, 0.0)).xyz;
        vec3 inv = 1.0 / d;
        vec3 ta = (u_object_low - o) * inv;
        vec3 tb = (u_object_low + u_object_size - o) * inv;
        vec3 far_ = max(ta, tb);
        float t_exit = max(min(min(far_.x, far_.y), far_.z), 0.0);
        // Front to back: each step adds what it scatters towards the eye,
        // dimmed by what lies in front of it, and lit by the key through a
        // few taps of the medium towards it — enough self-shadowing that a
        // cloud reads as a volume rather than a cut-out.
        const int STEPS = 64;
        const int TAPS = 4;
        float dt = t_exit / float(STEPS);
        vec3 key = (u_model_inverse * vec4(u_key, 0.0)).xyz;
        float reach = 0.25 * length(u_object_size) / max(length(key), 1e-6);
        float tap = reach / float(TAPS);
        vec3 ambient = mix(u_ground, u_sky, 0.75) * 0.45;
        vec3 gathered = vec3(0.0);
        float kept = 1.0;
        for (int i = 0; i < STEPS; i++) {
            vec3 p = o + d * ((float(i) + 0.5) * dt);
            float density = texture(t_volume, (p - u_object_low) / u_object_size).r;
            if (density <= 0.0) continue;
            float stopped = 1.0 - exp(-density * u_volume_extinction * dt);
            float towards_key = 0.0;
            for (int j = 1; j <= TAPS; j++) {
                towards_key += texture(t_volume, (p + key * (float(j) * tap) - u_object_low) / u_object_size).r;
            }
            float lit_by_key = exp(-towards_key * u_volume_extinction * tap);
            gathered += kept * stopped * u_volume_albedo * (ambient + vec3(0.55 * lit_by_key));
            kept *= 1.0 - stopped;
            if (kept < 0.004) break;
        }
        float film_y = 1.0 - (gl_FragCoord.y - u_viewport_px.y) / u_viewport_px.w;
        vec3 colour = backdrop(film_y) * kept + gathered;
        out_colour = vec4(encode_srgb(u_ap1_to_srgb * colour), 1.0);
        return;
    }
    // Facing the camera, as the ray tracer's `facing_forward` makes it.
    if (dot(normal, view) < 0.0) normal = -normal;

#ifdef FS_MATERIAL
    // The material's own program, written out by `procedural_gpu`.
    vec3 fs_base;
    float fs_metal, fs_rough, fs_height;
    fs_in_object = v_object * u_object_scale;
    fs_in_generated = (v_object - u_generated_centre) / u_generated_half * 0.5 + 0.5;
    fs_in_uv = v_uv;
    fs_in_object_normal = normalize(v_object_normal);
    fs_in_position = v_world;
    fs_in_normal = normal;
    vec3 fs_true = normalize(cross(dFdx(v_world), dFdy(v_world)));
    fs_in_true_normal = dot(fs_true, view) < 0.0 ? -fs_true : fs_true;
    fs_in_incoming = view;
    fs_in_backfacing = gl_FrontFacing ? 0.0 : 1.0;
    fs_in_object_location = u_object_location;
    fs_in_object_random = u_object_random;
    fs_material(fs_base, fs_metal, fs_rough, fs_height);
#ifdef FS_HEIGHT
    // The renderer's N - grad h, with the gradient from how the height and
    // the position change across the pixel (Mikkelsen's surface gradient).
    {
        vec3 dpdx = dFdx(v_world);
        vec3 dpdy = dFdy(v_world);
        float dhdx = dFdx(fs_height);
        float dhdy = dFdy(fs_height);
        vec3 r1 = cross(dpdy, normal);
        vec3 r2 = cross(normal, dpdx);
        float det = dot(dpdx, r1);
        vec3 surface_gradient = sign(det) * (dhdx * r1 + dhdy * r2);
        vec3 bumped = normalize(abs(det) * normal - surface_gradient);
        if (dot(bumped, view) > 1e-4) normal = bumped;
    }
#endif
#endif

    // A normal map, in the frame the UVs run in — the same frame the ray
    // tracer decodes against (`surface_bump.rs`), found here from how the
    // position and the UV change across the pixel rather than from stored
    // tangents: the tangent along u, the bitangent towards decreasing v,
    // because v runs down the image and green means up.
    if (u_normal_texture == 1) {
        vec2 uv = transform_uv(v_uv, u_normal_uv[0], u_normal_uv[1]);
        vec3 dp1 = dFdx(v_world);
        vec3 dp2 = dFdy(v_world);
        vec2 duv1 = dFdx(uv);
        vec2 duv2 = dFdy(uv);
        float det = duv1.x * duv2.y - duv1.y * duv2.x;
        vec3 mesh_t = v_tangent.xyz - normal * dot(normal, v_tangent.xyz);
        if (dot(mesh_t, mesh_t) > 1e-20) {
            // The mesh's own frame, smooth across triangles, as the ray
            // tracer uses it: the bitangent is the handedness times n x t.
            vec3 t = normalize(mesh_t);
            vec3 b = cross(normal, t) * (v_tangent.w < 0.0 ? -1.0 : 1.0);
            vec3 m = texture(t_normal, uv).rgb * 2.0 - 1.0;
            vec3 bumped = normalize(t * m.x + b * m.y + normal * max(m.z, 1e-4));
            if (dot(bumped, view) > 1e-4) normal = bumped;
        } else if (abs(det) > 1e-12) {
            vec3 dpdu = (dp1 * duv2.y - dp2 * duv1.y) / det;
            vec3 dpdv = (dp2 * duv1.x - dp1 * duv2.x) / det;
            vec3 t = dpdu - normal * dot(normal, dpdu);
            vec3 b = dpdv - normal * dot(normal, dpdv);
            if (dot(t, t) > 1e-20 && dot(b, b) > 1e-20) {
                t = normalize(t);
                b = -normalize(b);
                vec3 m = texture(t_normal, uv).rgb * 2.0 - 1.0;
                vec3 bumped = normalize(t * m.x + b * m.y + normal * max(m.z, 1e-4));
                // Never turned away from the eye: that is a surface inside
                // out, not a detail on it.
                if (dot(bumped, view) > 1e-4) normal = bumped;
            }
        }
    }

    vec3 base_colour = u_base;
    if (u_base_texture == 1) {
        vec3 texel = texture(t_base, transform_uv(v_uv, u_base_uv[0], u_base_uv[1])).rgb;
        base_colour = u_srgb_to_ap1 * texel;
    }
    float metalness = u_metalness;
    if (u_metal_channel >= 0) {
        metalness = channel_of(texture(t_metal, transform_uv(v_uv, u_metal_uv[0], u_metal_uv[1])), u_metal_channel);
    }
    float roughness_in = u_roughness;
    if (u_rough_channel >= 0) {
        roughness_in = channel_of(texture(t_rough, transform_uv(v_uv, u_rough_uv[0], u_rough_uv[1])), u_rough_channel);
    }

#ifdef FS_BASE
    base_colour = fs_base;
#endif
#ifdef FS_METAL
    metalness = fs_metal;
#endif
#ifdef FS_ROUGH
    roughness_in = fs_rough;
#endif

    float transmission_in = u_transmission;
    if (u_label_count > 0) lay_label(t_label0, 0, base_colour, metalness, roughness_in, transmission_in);
    if (u_label_count > 1) lay_label(t_label1, 1, base_colour, metalness, roughness_in, transmission_in);
    if (u_label_count > 2) lay_label(t_label2, 2, base_colour, metalness, roughness_in, transmission_in);
    if (u_label_count > 3) lay_label(t_label3, 3, base_colour, metalness, roughness_in, transmission_in);

    vec3 base = base_colour * clamp(u_base_weight, 0.0, 1.0);

    if (u_mode == 2) {
        // Material Preview: the environment's light by the split sum.
        float metal = clamp(metalness, 0.0, 1.0);
        float rough = clamp(roughness_in, 0.02, 1.0);
        float n_dot_v = clamp(dot(normal, view), 1e-4, 1.0);
        vec3 reflected = reflect(-view, normal);
        float f0_dielectric = pow((u_ior - 1.0) / (u_ior + 1.0), 2.0) * clamp(u_specular_weight, 0.0, 1.0);
        vec3 f0 = mix(vec3(f0_dielectric), base, metal);
        vec2 ab = environment_brdf(rough, n_dot_v);
        vec3 specular = environment(reflected, rough) * (f0 * ab.x + ab.y * (metal > 0.5 ? 1.0 : clamp(u_specular_weight, 0.0, 1.0)));
        vec3 kept = 1.0 - (f0 * ab.x + ab.y);
        vec3 diffuse = base * (1.0 - metal) * irradiance(normal) / PI * kept;
        vec3 colour = diffuse + specular;
        float transmission = clamp(transmission_in, 0.0, 1.0) * (1.0 - metal);
        if (transmission > 0.0) {
            // Through the part: the environment along the refracted view,
            // once, tinted - not a path through glass, but what it shows.
            vec3 through = refract(-view, normal, 1.0 / max(u_ior, 1.0001));
            if (dot(through, through) < 1e-6) through = reflected;
            vec3 behind = environment(through, rough) * u_transmission_colour + u_cloud * irradiance(normal) / PI;
            colour = mix(colour, behind * kept + specular, transmission);
        }
        float coat = clamp(u_coat_weight, 0.0, 1.0);
        if (coat > 0.0) {
            float coat_rough = clamp(u_coat_roughness, 0.02, 1.0);
            vec2 cab = environment_brdf(coat_rough, n_dot_v);
            float coat_f = 0.04 * cab.x + cab.y;
            colour = colour * (1.0 - coat * coat_f) + environment(reflected, coat_rough) * (coat * coat_f);
        }
        colour += u_emission;
        out_colour = vec4(display(colour), 1.0);
        return;
    }

    vec3 hemisphere = mix(u_ground, u_sky, 0.5 + 0.5 * normal.y);
    float key = max(dot(normal, u_key), 0.0);
    float fill = max(dot(normal, u_fill), 0.0);
    vec3 lighting = hemisphere * 0.62 + vec3(key * 0.42 + fill * 0.14);
    vec3 colour = base * lighting;

    float roughness = max(clamp(roughness_in, 0.02, 1.0), clamp(u_coat_roughness, 0.0, 1.0) * u_coat_weight);
    vec3 half_ = u_key + view;
    half_ = length(half_) > 0.0 ? normalize(half_) : normal;
    float cosine = dot(normal, half_);
    float weight = clamp(u_specular_weight, 0.0, 1.0) * (1.0 - roughness * 0.8);
    if (cosine > 0.0 && weight > 0.0) {
        float exponent = min(2.0 / max(roughness * roughness, 1e-4), 2048.0);
        vec3 tint = mix(vec3(1.0), base, clamp(metalness, 0.0, 1.0));
        colour += tint * (pow(cosine, exponent) * weight * 0.9);
    }

    float facing = clamp(abs(dot(normal, view)), 0.0, 1.0);
    float fresnel = (1.0 - facing) * (1.0 - facing);
    float rim = max(dot(normal, u_rim), 0.0);
    colour += vec3(fresnel * fresnel * (0.10 + 0.14 * rim));

    float transmission = clamp(transmission_in, 0.0, 1.0);
    if (transmission > 0.0) {
        float film_y = 1.0 - (gl_FragCoord.y - u_viewport_px.y) / u_viewport_px.w;
        vec3 behind = backdrop(film_y) * u_transmission_colour;
        colour = mix(colour, behind + u_cloud * lighting, transmission * 0.8);
    }
    colour += u_emission;

    out_colour = vec4(encode_srgb(u_ap1_to_srgb * colour), 1.0);
}
"#;

const BACKDROP_VERTEX: &str = r#"
out float v_film_y;
void main() {
    // One triangle that covers the viewport.
    vec2 corner = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_film_y = 1.0 - corner.y;
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const BACKDROP_FRAGMENT: &str = r#"
in float v_film_y;
out vec4 out_colour;
uniform vec3 u_backdrop_top;
uniform vec3 u_backdrop_bottom;
uniform mat3 u_ap1_to_srgb;
uniform int u_mode;
uniform mat4 u_inverse_view_projection;
uniform vec4 u_viewport_px;
uniform sampler2D t_environment;
uniform sampler2D t_background;
uniform int u_background_on_film;
uniform float u_environment_levels;
uniform float u_exposure;
const float PI = 3.14159265;
vec3 encode_srgb(vec3 linear) {
    vec3 c = clamp(linear, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}
vec3 pbr_neutral(vec3 color) {
    const float start = 0.8 - 0.04;
    const float desaturation = 0.15;
    float x = min(color.r, min(color.g, color.b));
    float offset = x < 0.08 ? x - 6.25 * x * x : 0.04;
    color -= offset;
    float peak = max(color.r, max(color.g, color.b));
    if (peak < start) return color;
    const float d = 1.0 - start;
    float new_peak = 1.0 - d * d / (peak + d - start);
    color *= new_peak / peak;
    float g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, vec3(new_peak), g);
}
void main() {
    if (u_mode == 2) {
        // The environment behind the model, softened as a lookdev view has
        // it, so the model reads against it.
        vec2 ndc = vec2((gl_FragCoord.x - u_viewport_px.x) / u_viewport_px.z, (gl_FragCoord.y - u_viewport_px.y) / u_viewport_px.w) * 2.0 - 1.0;
        vec4 near_ = u_inverse_view_projection * vec4(ndc, -1.0, 1.0);
        vec4 far_ = u_inverse_view_projection * vec4(ndc, 1.0, 1.0);
        vec3 d = normalize(far_.xyz / far_.w - near_.xyz / near_.w);
        vec2 uv = vec2(atan(d.x, d.z) / (2.0 * PI) + 0.5, acos(clamp(d.y, -1.0, 1.0)) / PI);
        if (u_background_on_film == 1) {
            uv = vec2((gl_FragCoord.x - u_viewport_px.x) / u_viewport_px.z, 1.0 - (gl_FragCoord.y - u_viewport_px.y) / u_viewport_px.w);
        }
        vec3 radiance = textureLod(t_background, uv, 0.0).rgb;
        out_colour = vec4(encode_srgb(pbr_neutral(max(u_ap1_to_srgb * radiance * u_exposure, vec3(0.0)))), 1.0);
        return;
    }
    vec3 colour = mix(u_backdrop_top, u_backdrop_bottom, clamp(v_film_y, 0.0, 1.0));
    out_colour = vec4(encode_srgb(u_ap1_to_srgb * colour), 1.0);
}
"#;

const LINES_VERTEX: &str = r#"
layout(location = 0) in vec3 a_position;
uniform mat4 u_model;
uniform mat4 u_view_projection;
void main() {
    vec4 clip = u_view_projection * u_model * vec4(a_position, 1.0);
    // Pulled a hair towards the eye, so an edge wins the depth test against
    // the faces it borders.
    clip.z -= 2e-5 * clip.w;
    gl_Position = clip;
}
"#;

const LINES_FRAGMENT: &str = r#"
out vec4 out_colour;
uniform vec4 u_colour;
void main() {
    out_colour = u_colour;
}
"#;

const SHADOW_CASTER_VERTEX: &str = r#"
layout(location = 0) in vec3 a_position;
uniform mat4 u_model;
uniform vec3 u_footprint;
uniform float u_plane;
uniform vec2 u_skew;
out float v_height;
void main() {
    vec4 world = u_model * vec4(a_position, 1.0);
    v_height = world.y - u_plane;
    // Onto the floor along the light — straight down when it has no lean —
    // and into the footprint: x across, z up the map.
    vec2 ground = world.xz + u_skew * max(v_height, 0.0);
    gl_Position = vec4((ground.x - u_footprint.x) / u_footprint.z, (ground.y - u_footprint.y) / u_footprint.z, 0.0, 1.0);
}
"#;

const SHADOW_CASTER_FRAGMENT: &str = r#"
in float v_height;
uniform float u_falloff;
uniform float u_contact_falloff;
out vec4 out_colour;
void main() {
    float h = max(v_height, 0.0);
    float overhead = clamp(1.0 - h / u_falloff, 0.0, 1.0);
    float contact = clamp(1.0 - h / u_contact_falloff, 0.0, 1.0);
    out_colour = vec4(overhead, contact * contact, 0.0, 0.0);
}
"#;

const SHADOW_BLUR_VERTEX: &str = r#"
out vec2 v_uv;
void main() {
    vec2 corner = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = corner;
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const SHADOW_BLUR_FRAGMENT: &str = r#"
in vec2 v_uv;
uniform sampler2D t_source;
uniform vec2 u_step;
uniform float u_blur_contact;
out vec4 out_colour;
void main() {
    // Thirteen taps of a Gaussian three taps wide.
    vec2 centre = texture(t_source, v_uv).rg;
    vec2 sum = vec2(0.0);
    float total = 0.0;
    for (int i = -6; i <= 6; i++) {
        float w = exp(-float(i * i) / 18.0);
        sum += texture(t_source, v_uv + u_step * float(i)).rg * w;
        total += w;
    }
    sum /= total;
    out_colour = vec4(sum.r, mix(centre.g, sum.g, u_blur_contact), 0.0, 0.0);
}
"#;

const SHADOW_GROUND_VERTEX: &str = r#"
uniform mat4 u_view_projection;
uniform vec3 u_footprint;
uniform float u_plane;
out vec2 v_uv;
void main() {
    vec2 corners[6] = vec2[](vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    vec2 c = corners[gl_VertexID];
    v_uv = c * 0.5 + 0.5;
    vec3 world = vec3(u_footprint.x + c.x * u_footprint.z, u_plane, u_footprint.y + c.y * u_footprint.z);
    gl_Position = u_view_projection * vec4(world, 1.0);
}
"#;

const SHADOW_GROUND_FRAGMENT: &str = r#"
in vec2 v_uv;
uniform sampler2D t_shadow;
uniform vec2 u_strength;
out vec4 out_colour;
void main() {
    vec2 near = clamp(texture(t_shadow, v_uv).rg, 0.0, 1.0);
    // Each takes its share of the light that is left.
    float shade = 1.0 - (1.0 - near.r * u_strength.x) * (1.0 - near.g * u_strength.y);
    // To nothing at the map's edges, so its border never shows.
    vec2 edge = min(v_uv, 1.0 - v_uv);
    float fade = smoothstep(0.0, 0.08, min(edge.x, edge.y));
    // A tail this soft steps by less than a display level at a time, and
    // the steps show as contours: a dither of about a level breaks them up,
    // as the render's own noise does.
    float dither = (fract(sin(dot(gl_FragCoord.xy, vec2(12.9898, 78.233))) * 43758.5453) - 0.5) * (2.5 / 255.0) * step(0.002, shade);
    out_colour = vec4(0.0, 0.0, 0.0, clamp(shade * fade + dither, 0.0, 1.0));
}
"#;

unsafe fn compile(gl: &glow::Context, vertex: &str, fragment: &str) -> Result<glow::Program, String> {
    let program = gl.create_program()?;
    let mut shaders = Vec::new();
    for (kind, source) in [(glow::VERTEX_SHADER, vertex), (glow::FRAGMENT_SHADER, fragment)] {
        let shader = gl.create_shader(kind)?;
        gl.shader_source(shader, &format!("#version 330 core\n{source}"));
        gl.compile_shader(shader);
        if !gl.get_shader_compile_status(shader) {
            return Err(format!("a Solid shader did not compile: {}", gl.get_shader_info_log(shader)));
        }
        gl.attach_shader(program, shader);
        shaders.push(shader);
    }
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        return Err(format!("the Solid shaders did not link: {}", gl.get_program_info_log(program)));
    }
    for shader in shaders {
        gl.detach_shader(program, shader);
        gl.delete_shader(shader);
    }
    Ok(program)
}

fn column_major(m: &Mat4) -> [f32; 16] {
    let mut out = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            out[column * 4 + row] = m.m[row][column];
        }
    }
    out
}

fn mat3_column_major(columns: [Vec3; 3]) -> [f32; 9] {
    [
        columns[0].x, columns[0].y, columns[0].z,
        columns[1].x, columns[1].y, columns[1].z,
        columns[2].x, columns[2].y, columns[2].z,
    ]
}

/// A colour-space change as a matrix, read off the function that does it.
fn matrix_of(convert: impl Fn(Vec3) -> Vec3) -> [f32; 9] {
    mat3_column_major([convert(Vec3::X), convert(Vec3::Y), convert(Vec3::Z)])
}

fn rgb(s: Spectrum) -> [f32; 3] {
    let v = s.to_rendering_rgb();
    [v.x, v.y, v.z]
}

/// The projection matrix, reproducing `Camera::pick_ray`'s frustum.
fn projection(camera: &Camera, aspect: f32, near: f32, far: f32) -> Option<(Mat4, bool)> {
    let (fov, shift, ortho) = match camera.projection {
        Projection::Perspective { .. } => (camera.vertical_fov(aspect), (0.0, 0.0), None),
        Projection::Shift { shift_x, shift_y, .. } => (camera.vertical_fov(aspect), (shift_x, shift_y), None),
        Projection::Orthographic { scale } => (0.0, (0.0, 0.0), Some(scale)),
        Projection::Equirectangular => return None,
    };
    let rows = |r: [[f32; 4]; 4]| Mat4 { m: r };
    Some(match ortho {
        Some(scale) => {
            let (h, w) = (scale * 0.5, scale * 0.5 * aspect);
            (
                rows([
                    [1.0 / w, 0.0, 0.0, 0.0],
                    [0.0, 1.0 / h, 0.0, 0.0],
                    [0.0, 0.0, -2.0 / (far - near), -(far + near) / (far - near)],
                    [0.0, 0.0, 0.0, 1.0],
                ]),
                true,
            )
        }
        None => {
            let f = 1.0 / (fov * 0.5).tan();
            // A shift slides the frustum across the film: fractions of the
            // sensor, and the sensor spans 2 in normalized device units.
            (
                rows([
                    [f / aspect, 0.0, 2.0 * shift.0, 0.0],
                    [0.0, f, 2.0 * shift.1, 0.0],
                    [0.0, 0.0, (far + near) / (near - far), 2.0 * far * near / (near - far)],
                    [0.0, 0.0, -1.0, 0.0],
                ]),
                false,
            )
        }
    })
}

fn wrap_mode(wrap: Wrap) -> i32 {
    (match wrap {
        Wrap::Repeat => glow::REPEAT,
        Wrap::Clamp => glow::CLAMP_TO_EDGE,
        Wrap::Mirror => glow::MIRRORED_REPEAT,
    }) as i32
}

impl Gpu {
    /// A uniform's location, asked of the driver once per program and name.
    /// Asked per part per frame - about twenty for each part, and glow
    /// allocates each name to ask - it cost tens of milliseconds a frame on
    /// an assembly of thousands of parts. Programs are made once and never
    /// deleted while the view lives, so a location found stays right.
    unsafe fn uniform(&self, gl: &glow::Context, program: glow::Program, name: &'static str) -> Option<glow::UniformLocation> {
        self.uniforms.borrow_mut().entry((program, name)).or_insert_with(|| gl.get_uniform_location(program, name)).clone()
    }

    unsafe fn draw(&mut self, gl: &glow::Context, info: &egui::PaintCallbackInfo) -> Result<(), String> {
        let Some(frame) = self.frame.take() else { return Ok(()) };
        if self.programs.is_none() {
            let surface = compile(gl, SURFACE_VERTEX, SURFACE_FRAGMENT)?;
            let backdrop = compile(gl, BACKDROP_VERTEX, BACKDROP_FRAGMENT)?;
            let backdrop_vao = gl.create_vertex_array()?;
            let lines = compile(gl, LINES_VERTEX, LINES_FRAGMENT)?;
            let shadow_caster = compile(gl, SHADOW_CASTER_VERTEX, SHADOW_CASTER_FRAGMENT)?;
            let shadow_blur = compile(gl, SHADOW_BLUR_VERTEX, SHADOW_BLUR_FRAGMENT)?;
            let shadow_ground = compile(gl, SHADOW_GROUND_VERTEX, SHADOW_GROUND_FRAGMENT)?;
            self.programs = Some(Programs { surface, backdrop, backdrop_vao, lines, shadow_caster, shadow_blur, shadow_ground });
        }
        let viewport = info.viewport_in_pixels();
        let (width, height) = (viewport.width_px.max(1), viewport.height_px.max(1));
        let aspect = width as f32 / height as f32;

        self.sync_scene(gl, &frame.scene)?;
        if frame.mode == GpuMode::MaterialPreview {
            self.sync_environment(gl, &frame.scene)?;
            if !self.shadow_failed {
                if let Err(error) = self.sync_shadow(gl, &frame.scene) {
                    eprintln!("Material Preview goes on without its ground shadow: {error}");
                    self.shadow_failed = true;
                    self.shadow = None;
                }
                // The shadow's passes drew elsewhere; the viewport is the
                // frame's again.
                gl.viewport(viewport.left_px, viewport.from_bottom_px, width, height);
            }
        }

        // The shader variants this frame's materials need, compiled the
        // first time they are seen.
        let material_programs = match self.material_programs.clone() {
            Some(programs) => programs,
            None => {
                let mut material_programs: Vec<Option<glow::Program>> = Vec::with_capacity(self.materials.len());
                for index in 0..self.materials.len() {
                    let Some(generated) = self.materials[index].procedural.clone() else {
                        material_programs.push(None);
                        continue;
                    };
                    let source = generated.source();
                    let key = {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        source.hash(&mut hasher);
                        hasher.finish()
                    };
                    let program = match self.variants.get(&key) {
                        Some(program) => *program,
                        None => {
                            let fragment = SURFACE_FRAGMENT.replace("//@MATERIAL@", &source);
                            let program = match compile(gl, SURFACE_VERTEX, &fragment) {
                                Ok(program) => Some(program),
                                Err(error) => {
                                    eprintln!("a material's pattern could not be drawn on the graphics card: {error}");
                                    None
                                }
                            };
                            self.variants.insert(key, program);
                            program
                        }
                    };
                    material_programs.push(program);
                }
                self.material_programs = Some(material_programs.clone());
                material_programs
            }
        };

        let ap1_to_srgb = matrix_of(|v| Spectrum::from_rendering_rgb(v).to_linear_srgb());
        let srgb_to_ap1 = matrix_of(|v| Spectrum::from_linear_srgb(v).to_rendering_rgb());
        let programs = self.programs.as_ref().expect("compiled above");
        let set3 = |program, name: &str, v: [f32; 3]| {
            gl.uniform_3_f32(gl.get_uniform_location(program, name).as_ref(), v[0], v[1], v[2]);
        };
        let mode_code = match frame.mode {
            GpuMode::Solid => 0,
            GpuMode::Wireframe => 1,
            GpuMode::MaterialPreview => 2,
        };

        // --- the camera --------------------------------------------------------
        let bounds = frame.scene.bounds();
        let camera = &frame.camera;
        let eye = camera.position();
        let extent = (bounds.max - bounds.min).length().max(1e-3);
        let distance = (eye - (bounds.min + bounds.max) * 0.5).length();
        let far = (distance + extent * 2.0).max(1e-2);
        // A near plane a small fraction of the far one: depth precision is
        // spent near the camera, and a product viewport has nothing closer
        // than this worth seeing.
        let near = (far * 1e-4).max(1e-4);
        let Some((projection, orthographic)) = projection(camera, aspect, near, far) else {
            return Err("the Solid GPU view does not draw 360° cameras".into());
        };
        let view = camera.camera_to_world.inverse().ok_or("the camera matrix is singular")?;
        let view_projection = projection * view;

        // --- backdrop ---------------------------------------------------------
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::BLEND);
        gl.use_program(Some(programs.backdrop));
        let (top, bottom) = match frame.mode {
            // Blender's wireframe ground: dark, so the lines carry it.
            GpuMode::Wireframe => ([0.032, 0.032, 0.034], [0.012, 0.012, 0.013]),
            _ => (rgb(preview::BACKDROP_TOP), rgb(preview::BACKDROP_BOTTOM)),
        };
        set3(programs.backdrop, "u_backdrop_top", top);
        set3(programs.backdrop, "u_backdrop_bottom", bottom);
        gl.uniform_matrix_3_f32_slice(gl.get_uniform_location(programs.backdrop, "u_ap1_to_srgb").as_ref(), false, &ap1_to_srgb);
        gl.uniform_1_i32(gl.get_uniform_location(programs.backdrop, "u_mode").as_ref(), mode_code);
        if let (GpuMode::MaterialPreview, Some(environment)) = (frame.mode, &self.environment) {
            let inverse = view_projection.inverse().ok_or("the view is singular")?;
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(programs.backdrop, "u_inverse_view_projection").as_ref(), false, &column_major(&inverse));
            gl.uniform_4_f32(gl.get_uniform_location(programs.backdrop, "u_viewport_px").as_ref(), viewport.left_px as f32, viewport.from_bottom_px as f32, width as f32, height as f32);
            gl.uniform_1_f32(gl.get_uniform_location(programs.backdrop, "u_environment_levels").as_ref(), environment.levels);
            gl.uniform_1_f32(gl.get_uniform_location(programs.backdrop, "u_exposure").as_ref(), frame.exposure);
            gl.uniform_1_i32(gl.get_uniform_location(programs.backdrop, "t_background").as_ref(), 6);
            gl.uniform_1_i32(gl.get_uniform_location(programs.backdrop, "u_background_on_film").as_ref(), environment.background_on_film as i32);
            gl.active_texture(glow::TEXTURE6);
            gl.bind_texture(glow::TEXTURE_2D, Some(environment.background));
            gl.active_texture(glow::TEXTURE0);
        }
        gl.bind_vertex_array(Some(programs.backdrop_vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);

        // --- the ground shadow -------------------------------------------------
        if let (GpuMode::MaterialPreview, Some(shadow)) = (frame.mode, &self.shadow) {
            let f = shadow.footprint;
            // From under the ground there is nothing to lay it on.
            if eye.y > f.plane {
                let program = programs.shadow_ground;
                gl.use_program(Some(program));
                gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(program, "u_view_projection").as_ref(), false, &column_major(&view_projection));
                gl.uniform_3_f32(gl.get_uniform_location(program, "u_footprint").as_ref(), f.centre.0, f.centre.1, f.half);
                gl.uniform_1_f32(gl.get_uniform_location(program, "u_plane").as_ref(), f.plane);
                gl.uniform_2_f32(gl.get_uniform_location(program, "u_strength").as_ref(), f.strength, f.contact_strength);
                gl.uniform_1_i32(gl.get_uniform_location(program, "t_shadow").as_ref(), 7);
                gl.active_texture(glow::TEXTURE7);
                gl.bind_texture(glow::TEXTURE_2D, Some(shadow.textures[0]));
                gl.active_texture(glow::TEXTURE0);
                gl.enable(glow::BLEND);
                gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                gl.draw_arrays(glow::TRIANGLES, 0, 6);
                gl.disable(glow::BLEND);
            }
        }

        // --- the model --------------------------------------------------------
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LESS);
        gl.depth_mask(true);
        gl.clear_depth_f32(1.0);
        gl.clear(glow::DEPTH_BUFFER_BIT);

        let scene = frame.scene.clone();
        // Everything the surface shader needs for the frame, set on one of
        // its programs: the plain one, or a material's variant.
        let configure = |program: glow::Program| {
            gl.use_program(Some(program));
            let location = |name: &str| gl.get_uniform_location(program, name);
            let rig = preview::Rig::for_view(camera.forward(), Vec3::Y);
            gl.uniform_matrix_4_f32_slice(location("u_view_projection").as_ref(), false, &column_major(&view_projection));
            gl.uniform_2_f32(location("u_viewport").as_ref(), width as f32, height as f32);
            gl.uniform_4_f32(location("u_viewport_px").as_ref(), viewport.left_px as f32, viewport.from_bottom_px as f32, width as f32, height as f32);
            gl.uniform_3_f32(location("u_eye").as_ref(), eye.x, eye.y, eye.z);
            let forward = camera.forward();
            gl.uniform_3_f32(location("u_forward").as_ref(), forward.x, forward.y, forward.z);
            gl.uniform_1_i32(location("u_orthographic").as_ref(), orthographic as i32);
            for (name, v) in [("u_key", rig.key), ("u_fill", rig.fill), ("u_rim", rig.rim)] {
                gl.uniform_3_f32(location(name).as_ref(), v.x, v.y, v.z);
            }
            set3(program, "u_sky", rgb(preview::SKY));
            set3(program, "u_ground", rgb(preview::GROUND_BOUNCE));
            set3(program, "u_backdrop_top", rgb(preview::BACKDROP_TOP));
            set3(program, "u_backdrop_bottom", rgb(preview::BACKDROP_BOTTOM));
            gl.uniform_matrix_3_f32_slice(location("u_ap1_to_srgb").as_ref(), false, &ap1_to_srgb);
            gl.uniform_matrix_3_f32_slice(location("u_srgb_to_ap1").as_ref(), false, &srgb_to_ap1);
            gl.uniform_1_i32(location("t_base").as_ref(), 0);
            gl.uniform_1_i32(location("t_metal").as_ref(), 1);
            gl.uniform_1_i32(location("t_rough").as_ref(), 2);
            gl.uniform_1_i32(location("t_normal").as_ref(), 3);
            gl.uniform_1_i32(location("t_volume").as_ref(), 4);
            for (i, name) in LABEL_SAMPLERS.iter().enumerate() {
                gl.uniform_1_i32(location(name).as_ref(), LABEL_UNIT as i32 + i as i32);
            }
            // The ring colour: the interface's accent, as display code.
            let accent = crate::theme::p().accent;
            gl.uniform_3_f32(location("u_outline_colour").as_ref(), accent.r() as f32 / 255.0, accent.g() as f32 / 255.0, accent.b() as f32 / 255.0);
            gl.uniform_1_i32(location("u_mode").as_ref(), mode_code);
            gl.uniform_1_f32(location("u_exposure").as_ref(), frame.exposure);
            gl.uniform_1_i32(location("t_environment").as_ref(), 5);
            if let (GpuMode::MaterialPreview, Some(environment)) = (frame.mode, &self.environment) {
                gl.uniform_1_f32(location("u_environment_levels").as_ref(), environment.levels);
                gl.uniform_3_f32_slice(location("u_irradiance").as_ref(), &environment.irradiance);
                gl.active_texture(glow::TEXTURE5);
                gl.bind_texture(glow::TEXTURE_2D, Some(environment.texture));
                gl.active_texture(glow::TEXTURE0);
            }
            self.set_cutters(gl, program, &scene);
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_outline_pass").as_ref(), 0);
            gl.uniform_1_f32(gl.get_uniform_location(program, "u_outline").as_ref(), 0.0);
        };
        let program = programs.surface;
        configure(program);
        let location = |name: &str| gl.get_uniform_location(program, name);

        let selected: std::collections::HashSet<u32> = frame.selected.iter().copied().collect();
        let drawable = |i: &&tequila_render_core::scene::Instance| i.visible_to_camera && i.shadow_catcher.is_none();

        if frame.mode == GpuMode::Wireframe {
            // The faces into the depth buffer only, so the edges in front of
            // them can be told from the edges behind.
            gl.color_mask(false, false, false, false);
            for instance in scene.instances.iter().filter(drawable) {
                self.draw_instance(gl, program, instance, &scene);
            }
            gl.color_mask(true, true, true, true);
            let lines = programs.lines;
            gl.use_program(Some(lines));
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(lines, "u_view_projection").as_ref(), false, &column_major(&view_projection));
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(false);
            // Hidden edges first and faint, then the visible ones over them.
            for (depth, strength) in [(glow::GREATER, 0.16), (glow::LEQUAL, 0.92)] {
                gl.depth_func(depth);
                for instance in scene.instances.iter().filter(drawable) {
                    let colour = if selected.contains(&instance.id.0) {
                        let a = crate::theme::p().accent;
                        [a.r() as f32 / 255.0, a.g() as f32 / 255.0, a.b() as f32 / 255.0]
                    } else {
                        [0.78, 0.78, 0.8]
                    };
                    gl.uniform_4_f32(self.uniform(gl, lines, "u_colour").as_ref(), colour[0], colour[1], colour[2], strength);
                    self.draw_edges(gl, lines, instance, &scene);
                }
            }
            gl.depth_func(glow::LESS);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
            return Ok(());
        }

        // The outline: selected parts pushed out and drawn back faces only,
        // before the parts themselves cover all but the ring.
        if !selected.is_empty() {
            gl.enable(glow::CULL_FACE);
            gl.cull_face(glow::FRONT);
            gl.uniform_1_i32(location("u_outline_pass").as_ref(), 1);
            gl.uniform_1_f32(location("u_outline").as_ref(), 2.0 * frame.pixels_per_point);
            for instance in scene.instances.iter().filter(drawable).filter(|i| selected.contains(&i.id.0)) {
                self.draw_instance(gl, program, instance, &scene);
            }
            gl.disable(glow::CULL_FACE);
        }
        gl.uniform_1_i32(location("u_outline_pass").as_ref(), 0);
        gl.uniform_1_f32(location("u_outline").as_ref(), 0.0);
        let mut current = program;
        let mut configured = vec![program];
        for instance in scene.instances.iter().filter(drawable) {
            let index = instance.material.index();
            let wanted = material_programs.get(index).copied().flatten().unwrap_or(program);
            if wanted != current {
                if configured.contains(&wanted) {
                    gl.use_program(Some(wanted));
                } else {
                    configure(wanted);
                    configured.push(wanted);
                }
                current = wanted;
            }
            self.set_material(gl, current, index);
            self.set_volume(gl, current, instance, &scene);
            if current != program {
                self.set_object(gl, current, instance, &scene);
            }
            self.draw_instance(gl, current, instance, &scene);
        }
        for program in configured {
            gl.use_program(Some(program));
            gl.uniform_1_i32(gl.get_uniform_location(program, "u_volume").as_ref(), 0);
        }

        // Model edges over the shading, where they are in front of it: the
        // faces are in the depth buffer, so a hidden edge stays hidden.
        if frame.model_edges {
            let lines = programs.lines;
            gl.use_program(Some(lines));
            gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(lines, "u_view_projection").as_ref(), false, &column_major(&view_projection));
            let ink = if frame.mode == GpuMode::MaterialPreview { [0.06, 0.06, 0.07, 0.7] } else { [0.1, 0.1, 0.12, 0.85] };
            gl.uniform_4_f32(gl.get_uniform_location(lines, "u_colour").as_ref(), ink[0], ink[1], ink[2], ink[3]);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
            gl.depth_mask(false);
            gl.depth_func(glow::LEQUAL);
            for instance in scene.instances.iter().filter(drawable) {
                let mesh = &scene.meshes[instance.mesh.index()];
                let Some(gpu) = self.meshes.get(&(Arc::as_ptr(mesh) as usize)) else { continue };
                if gpu.model_count == 0 {
                    continue;
                }
                gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(lines, "u_model").as_ref(), false, &column_major(&instance.transform.matrix));
                gl.bind_vertex_array(Some(gpu.model_vao));
                gl.draw_arrays(glow::LINES, 0, gpu.model_count);
            }
            gl.bind_vertex_array(None);
            gl.depth_func(glow::LESS);
            gl.depth_mask(true);
            gl.disable(glow::BLEND);
        }
        Ok(())
    }

    /// What a material's patterns read of the part: its scale, its random
    /// number, where it is and its mesh's bounding box, as the renderer's
    /// shading point has them (`SceneSnapshot::shading_point`).
    unsafe fn set_object(&self, gl: &glow::Context, program: glow::Program, instance: &tequila_render_core::scene::Instance, scene: &SceneSnapshot) {
        let location = |name: &'static str| self.uniform(gl, program, name);
        gl.uniform_1_f32(location("u_object_scale").as_ref(), 1.0 / instance.transform.inv_length_scale());
        gl.uniform_1_f32(location("u_object_random").as_ref(), SceneSnapshot::object_random(instance.id.index()));
        let at = instance.transform.point(Vec3::ZERO);
        gl.uniform_3_f32(location("u_object_location").as_ref(), at.x, at.y, at.z);
        let bounds = scene.meshes[instance.mesh.index()].bounds();
        let centre = (bounds.min + bounds.max) * 0.5;
        let half = (bounds.max - bounds.min) * 0.5;
        let size = |h: f32| if h == 0.0 { 1.0 } else { h.max(1e-5) };
        gl.uniform_3_f32(location("u_generated_centre").as_ref(), centre.x, centre.y, centre.z);
        gl.uniform_3_f32(location("u_generated_half").as_ref(), size(half.x), size(half.y), size(half.z));
    }

    /// A Volume Map's grid for this part, fitted to its bounding box, or
    /// none.
    unsafe fn set_volume(&self, gl: &glow::Context, program: glow::Program, instance: &tequila_render_core::scene::Instance, scene: &SceneSnapshot) {
        let location = |name: &'static str| self.uniform(gl, program, name);
        let material = self.materials.get(instance.material.index());
        let gpu = material
            .and_then(|m| m.volume.as_ref())
            .and_then(|grid| self.volumes.get(&(Arc::as_ptr(grid) as usize)));
        let mesh = self.meshes.get(&(Arc::as_ptr(&scene.meshes[instance.mesh.index()]) as usize));
        let (Some(material), Some(gpu), Some(mesh)) = (material, gpu, mesh) else {
            gl.uniform_1_i32(location("u_volume").as_ref(), 0);
            return;
        };
        let (low, high) = mesh.bounds;
        let size = high - low;
        gl.uniform_1_i32(location("u_volume").as_ref(), 1);
        gl.active_texture(glow::TEXTURE4);
        gl.bind_texture(glow::TEXTURE_3D, Some(gpu.texture));
        gl.active_texture(glow::TEXTURE0);
        gl.uniform_matrix_4_f32_slice(location("u_model_inverse").as_ref(), false, &column_major(&instance.transform.inverse));
        gl.uniform_3_f32(location("u_object_low").as_ref(), low.x, low.y, low.z);
        gl.uniform_3_f32(location("u_object_size").as_ref(), size.x.max(1e-9), size.y.max(1e-9), size.z.max(1e-9));
        gl.uniform_1_f32(location("u_volume_extinction").as_ref(), material.volume_extinction);
        let albedo = rgb(material.volume_albedo);
        gl.uniform_3_f32(location("u_volume_albedo").as_ref(), albedo[0], albedo[1], albedo[2]);
    }

    /// The analytic cutters, for the fragment shader to clip against.
    unsafe fn set_cutters(&self, gl: &glow::Context, program: glow::Program, scene: &SceneSnapshot) {
        use tequila_render_core::cutaway::CutterShape;
        const MAX: usize = 8;
        let mut inverses = Vec::with_capacity(MAX * 16);
        let mut shapes = Vec::with_capacity(MAX);
        let mut caps = Vec::with_capacity(MAX * 4);
        for cutter in scene.cutters() {
            let shape = match cutter.shape {
                CutterShape::Plane => 0,
                CutterShape::Box => 1,
                CutterShape::Sphere => 2,
                CutterShape::Cylinder => 3,
                CutterShape::Mesh(_) => continue,
            };
            if shapes.len() == MAX {
                break;
            }
            inverses.extend_from_slice(&column_major(&cutter.transform.inverse));
            shapes.push(shape);
            match cutter.cap {
                Some(material) => {
                    let base = scene.material(material).base_color;
                    caps.extend_from_slice(&[base.r, base.g, base.b, 1.0]);
                }
                None => caps.extend_from_slice(&[0.0, 0.0, 0.0, 0.0]),
            }
        }
        let location = |name: &str| gl.get_uniform_location(program, name);
        gl.uniform_1_i32(location("u_cutter_count").as_ref(), shapes.len() as i32);
        if !shapes.is_empty() {
            gl.uniform_matrix_4_f32_slice(location("u_cutter_inverse").as_ref(), false, &inverses);
            gl.uniform_1_i32_slice(location("u_cutter_shape").as_ref(), &shapes);
            gl.uniform_4_f32_slice(location("u_cutter_cap").as_ref(), &caps);
        }
    }

    unsafe fn draw_instance(&self, gl: &glow::Context, program: glow::Program, instance: &tequila_render_core::scene::Instance, scene: &SceneSnapshot) {
        let mesh = &scene.meshes[instance.mesh.index()];
        let Some(gpu) = self.meshes.get(&(Arc::as_ptr(mesh) as usize)) else { return };
        let model = instance.transform.matrix;
        let inverse = instance.transform.inverse;
        // The normal matrix is the inverse transpose; `inverse` is already
        // row-major, so reading it column-major transposes it.
        let normal = [
            inverse.m[0][0], inverse.m[0][1], inverse.m[0][2],
            inverse.m[1][0], inverse.m[1][1], inverse.m[1][2],
            inverse.m[2][0], inverse.m[2][1], inverse.m[2][2],
        ];
        gl.uniform_matrix_4_f32_slice(self.uniform(gl, program, "u_model").as_ref(), false, &column_major(&model));
        gl.uniform_matrix_3_f32_slice(self.uniform(gl, program, "u_normal_matrix").as_ref(), false, &normal);
        gl.bind_vertex_array(Some(gpu.vao));
        if gpu.indexed {
            gl.draw_elements(glow::TRIANGLES, gpu.count, glow::UNSIGNED_INT, 0);
        } else {
            gl.draw_arrays(glow::TRIANGLES, 0, gpu.count);
        }
    }

    unsafe fn draw_edges(&self, gl: &glow::Context, program: glow::Program, instance: &tequila_render_core::scene::Instance, scene: &SceneSnapshot) {
        let mesh = &scene.meshes[instance.mesh.index()];
        let Some(gpu) = self.meshes.get(&(Arc::as_ptr(mesh) as usize)) else { return };
        gl.uniform_matrix_4_f32_slice(self.uniform(gl, program, "u_model").as_ref(), false, &column_major(&instance.transform.matrix));
        gl.bind_vertex_array(Some(gpu.edge_vao));
        gl.draw_elements(glow::LINES, gpu.edge_count, glow::UNSIGNED_INT, 0);
    }

    /// The ground shadow for this scene: drawn from above and blurred when
    /// the geometry or the environment changes, kept while only the camera
    /// moves or a material is edited.
    unsafe fn sync_shadow(&mut self, gl: &glow::Context, scene: &Arc<SceneSnapshot>) -> Result<(), String> {
        let source = shadow_source(scene);
        if self.shadow.as_ref().is_some_and(|s| s.source == source) {
            return Ok(());
        }
        let Some(footprint) = shadow_footprint(scene) else {
            self.drop_shadow(gl);
            return Ok(());
        };
        if self.shadow.is_none() {
            let mut textures = Vec::new();
            let mut framebuffers = Vec::new();
            for _ in 0..2 {
                let texture = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA16F as i32, SHADOW_SIZE, SHADOW_SIZE, 0, glow::RGBA, glow::HALF_FLOAT, None);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                textures.push(texture);
            }
            let previous = gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING);
            for texture in &textures {
                let framebuffer = gl.create_framebuffer()?;
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
                gl.framebuffer_texture_2d(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::TEXTURE_2D, Some(*texture), 0);
                let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
                framebuffers.push(framebuffer);
                if status != glow::FRAMEBUFFER_COMPLETE {
                    gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
                    for framebuffer in framebuffers {
                        gl.delete_framebuffer(framebuffer);
                    }
                    for texture in textures {
                        gl.delete_texture(texture);
                    }
                    return Err(format!("its framebuffer is incomplete ({status:#x})"));
                }
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
            self.shadow = Some(GpuShadow {
                textures: [textures[0], textures[1]],
                framebuffers: [framebuffers[0], framebuffers[1]],
                footprint,
                source: 0,
            });
        }

        let programs = self.programs.as_ref().expect("compiled before the shadow");
        let shadow = self.shadow.as_mut().expect("made above");
        shadow.footprint = footprint;
        shadow.source = source;
        let (textures, framebuffers) = (shadow.textures, shadow.framebuffers);

        let previous = gl.get_parameter_framebuffer(glow::FRAMEBUFFER_BINDING);
        let scissor = gl.is_enabled(glow::SCISSOR_TEST);
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::CULL_FACE);
        gl.viewport(0, 0, SHADOW_SIZE, SHADOW_SIZE);

        // From above: each texel keeps the nearest the model comes to the
        // ground over it.
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffers[0]));
        gl.clear_color(0.0, 0.0, 0.0, 0.0);
        gl.clear(glow::COLOR_BUFFER_BIT);
        gl.enable(glow::BLEND);
        gl.blend_equation(glow::MAX);
        gl.blend_func(glow::ONE, glow::ONE);
        let caster = programs.shadow_caster;
        gl.use_program(Some(caster));
        gl.uniform_3_f32(gl.get_uniform_location(caster, "u_footprint").as_ref(), footprint.centre.0, footprint.centre.1, footprint.half);
        gl.uniform_1_f32(gl.get_uniform_location(caster, "u_plane").as_ref(), footprint.plane);
        gl.uniform_1_f32(gl.get_uniform_location(caster, "u_falloff").as_ref(), footprint.falloff);
        gl.uniform_1_f32(gl.get_uniform_location(caster, "u_contact_falloff").as_ref(), footprint.contact_falloff);
        // The broad shadow into red, cast along the light; the contact into
        // green, straight down, since it is the gap under the part that it
        // darkens wherever the light is.
        for (skew, mask) in [(footprint.skew, (true, false)), ((0.0, 0.0), (false, true))] {
            gl.color_mask(mask.0, mask.1, false, false);
            gl.uniform_2_f32(gl.get_uniform_location(caster, "u_skew").as_ref(), skew.0, skew.1);
            for instance in scene.instances.iter().filter(|i| casts_ground_shadow(i)) {
                let mesh = &scene.meshes[instance.mesh.index()];
                let Some(gpu) = self.meshes.get(&(Arc::as_ptr(mesh) as usize)) else { continue };
                gl.uniform_matrix_4_f32_slice(gl.get_uniform_location(caster, "u_model").as_ref(), false, &column_major(&instance.transform.matrix));
                gl.bind_vertex_array(Some(gpu.vao));
                if gpu.indexed {
                    gl.draw_elements(glow::TRIANGLES, gpu.count, glow::UNSIGNED_INT, 0);
                } else {
                    gl.draw_arrays(glow::TRIANGLES, 0, gpu.count);
                }
            }
        }
        gl.color_mask(true, true, true, true);
        gl.blend_equation(glow::FUNC_ADD);
        gl.disable(glow::BLEND);

        // Blurred across and down, ending back in the first map: the contact
        // a little, once; the overhead shadow wide — a studio's softboxes
        // are big, and the render's penumbra is near half the model's
        // radius — in sixteen more passes of a thirteen-tap kernel, close
        // enough together to stay smooth (spread-out taps ring).
        let blur = programs.shadow_blur;
        gl.use_program(Some(blur));
        gl.bind_vertex_array(Some(programs.backdrop_vao));
        gl.uniform_1_i32(gl.get_uniform_location(blur, "t_source").as_ref(), 0);
        gl.active_texture(glow::TEXTURE0);
        // Taps half a texel off the grid, so linear filtering mixes each
        // texel with its neighbour: at whole-texel spacing a pass only ever
        // mixes texels that far apart, and the floor came out combed.
        let passes = std::iter::once((1.0, true)).chain(std::iter::repeat((2.5, false)).take(16));
        for (reach, contact) in passes {
            let reach = reach / SHADOW_SIZE as f32;
            gl.uniform_1_f32(gl.get_uniform_location(blur, "u_blur_contact").as_ref(), if contact { 1.0 } else { 0.0 });
            for (from, to, step) in [(0usize, 1usize, (reach, 0.0)), (1, 0, (0.0, reach))] {
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffers[to]));
                gl.bind_texture(glow::TEXTURE_2D, Some(textures[from]));
                gl.uniform_2_f32(gl.get_uniform_location(blur, "u_step").as_ref(), step.0, step.1);
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
        }

        gl.bind_framebuffer(glow::FRAMEBUFFER, previous);
        if scissor {
            gl.enable(glow::SCISSOR_TEST);
        }
        Ok(())
    }

    unsafe fn drop_shadow(&mut self, gl: &glow::Context) {
        if let Some(shadow) = self.shadow.take() {
            for framebuffer in shadow.framebuffers {
                gl.delete_framebuffer(framebuffer);
            }
            for texture in shadow.textures {
                gl.delete_texture(texture);
            }
        }
    }

    /// The scene's environment as Material Preview lights with it, made
    /// again when the environment changes.
    unsafe fn sync_environment(&mut self, gl: &glow::Context, scene: &SceneSnapshot) -> Result<(), String> {
        let source = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (Arc::as_ptr(&scene.environment) as usize).hash(&mut h);
            match &scene.background {
                tequila_render_core::environment::Background::Environment => 0u8.hash(&mut h),
                tequila_render_core::environment::Background::Color(c) => (1u8, c.r.to_bits(), c.g.to_bits(), c.b.to_bits()).hash(&mut h),
                tequila_render_core::environment::Background::Backplate { image } => (2u8, Arc::as_ptr(image) as usize).hash(&mut h),
            }
            h.finish()
        };
        if self.environment.as_ref().is_some_and(|e| e.source == source) {
            return Ok(());
        }
        let map = environment_map(&scene.environment);
        let (backdrop, on_film) = background_map(scene);
        let background = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(background));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGB16F as i32,
            backdrop.0 as i32,
            backdrop.1 as i32,
            0,
            glow::RGB,
            glow::FLOAT,
            Some(bytes_of(&backdrop.2)),
        );
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, if on_film { glow::CLAMP_TO_EDGE } else { glow::REPEAT } as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGB16F as i32,
            ENVIRONMENT_WIDTH as i32,
            ENVIRONMENT_HEIGHT as i32,
            0,
            glow::RGB,
            glow::FLOAT,
            Some(bytes_of(&map.radiance)),
        );
        gl.generate_mipmap(glow::TEXTURE_2D);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::REPEAT as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.bind_texture(glow::TEXTURE_2D, None);
        if let Some(old) = self.environment.take() {
            gl.delete_texture(old.texture);
            gl.delete_texture(old.background);
        }
        let levels = (ENVIRONMENT_WIDTH.max(ENVIRONMENT_HEIGHT) as f32).log2().floor() + 1.0;
        self.environment = Some(GpuEnvironment { texture, levels, irradiance: map.irradiance, background, background_on_film: on_film, source });
        Ok(())
    }

    unsafe fn set_material(&self, gl: &glow::Context, program: glow::Program, index: usize) {
        let Some(material) = self.materials.get(index) else { return };
        let location = |name: &'static str| self.uniform(gl, program, name);
        let set3 = |name: &'static str, s: Spectrum| {
            let v = rgb(s);
            gl.uniform_3_f32(location(name).as_ref(), v[0], v[1], v[2]);
        };
        set3("u_base", material.base_color);
        set3("u_transmission_colour", material.transmission_color);
        set3("u_cloud", material.cloud);
        set3("u_emission", material.emission);
        for (name, value) in [
            ("u_base_weight", material.base_weight),
            ("u_metalness", material.metalness),
            ("u_specular_weight", material.specular_weight),
            ("u_roughness", material.roughness),
            ("u_coat_weight", material.coat_weight),
            ("u_coat_roughness", material.coat_roughness),
            ("u_transmission", material.transmission),
            ("u_ior", material.ior),
        ] {
            gl.uniform_1_f32(location(name).as_ref(), value);
        }
        let bind = |unit: u32, source: &Source, flag: &'static str, uv: &'static str, is_channel: bool| {
            match source {
                Source::Texture { texture, channel, uv: transform } => {
                    let Some(gpu) = self.textures.get(&(Arc::as_ptr(texture) as usize)) else {
                        gl.uniform_1_i32(location(flag).as_ref(), -1);
                        return;
                    };
                    gl.active_texture(glow::TEXTURE0 + unit);
                    gl.bind_texture(glow::TEXTURE_2D, Some(gpu.texture));
                    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap_mode(transform.wrap[0]));
                    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap_mode(transform.wrap[1]));
                    let flips = transform.flip[0] as i32 as f32 + 2.0 * transform.flip[1] as i32 as f32;
                    let packed = [
                        transform.scale[0], transform.scale[1], transform.pivot[0], transform.pivot[1],
                        transform.offset[0], transform.offset[1], transform.rotation, flips,
                    ];
                    gl.uniform_4_f32_slice(location(uv).as_ref(), &packed);
                    let value = if is_channel { channel.map(|c| c as i32).unwrap_or(0) } else { 1 };
                    gl.uniform_1_i32(location(flag).as_ref(), value);
                }
                Source::Constant => {
                    gl.uniform_1_i32(location(flag).as_ref(), if is_channel { -1 } else { 0 });
                }
            }
        };
        bind(0, &material.base_color_source, "u_base_texture", "u_base_uv", false);
        bind(1, &material.metalness_source, "u_metal_channel", "u_metal_uv", true);
        bind(2, &material.roughness_source, "u_rough_channel", "u_rough_uv", true);
        bind(3, &material.normal_source, "u_normal_texture", "u_normal_uv", false);

        // The labels, their artwork on units of their own.
        let mut modes = [0i32; 4 * MAX_LABELS];
        let mut uvs = [0f32; 8 * MAX_LABELS];
        let mut finishes = [0f32; 4 * MAX_LABELS];
        let mut count = 0;
        for label in &material.labels {
            let Some(gpu) = self.label_textures.get(&(Arc::as_ptr(&label.texture) as usize)) else { continue };
            gl.active_texture(glow::TEXTURE0 + LABEL_UNIT + count as u32);
            gl.bind_texture(glow::TEXTURE_2D, Some(gpu.texture));
            modes[4 * count..4 * count + 4].copy_from_slice(&label.mode);
            uvs[8 * count..8 * count + 8].copy_from_slice(&label.uv);
            finishes[4 * count..4 * count + 4].copy_from_slice(&[label.opacity, label.roughness, label.metallic as i32 as f32, 0.0]);
            count += 1;
        }
        gl.uniform_1_i32(location("u_label_count").as_ref(), count as i32);
        if count > 0 {
            gl.uniform_4_i32_slice(location("u_label_mode").as_ref(), &modes);
            gl.uniform_4_f32_slice(location("u_label_uv").as_ref(), &uvs);
            gl.uniform_4_f32_slice(location("u_label_finish").as_ref(), &finishes);
        }
        gl.active_texture(glow::TEXTURE0);
    }

    /// Upload what the scene has that the GPU does not, and let go of what it
    /// no longer has.
    unsafe fn sync_scene(&mut self, gl: &glow::Context, scene: &Arc<SceneSnapshot>) -> Result<(), String> {
        if self.materials_for.as_ref().is_some_and(|s| Arc::ptr_eq(s, scene)) {
            return Ok(());
        }
        let mut live_meshes = std::collections::HashSet::new();
        for mesh in &scene.meshes {
            let key = Arc::as_ptr(mesh) as usize;
            live_meshes.insert(key);
            if let std::collections::hash_map::Entry::Vacant(slot) = self.meshes.entry(key) {
                slot.insert(upload_mesh(gl, mesh)?);
            }
        }
        let stale: Vec<usize> = self.meshes.keys().copied().filter(|k| !live_meshes.contains(k)).collect();
        for key in stale {
            if let Some(mesh) = self.meshes.remove(&key) {
                gl.delete_vertex_array(mesh.vao);
                gl.delete_vertex_array(mesh.edge_vao);
                gl.delete_vertex_array(mesh.model_vao);
                for buffer in mesh.buffers {
                    gl.delete_buffer(buffer);
                }
            }
        }

        self.materials = (0..scene.materials.len()).map(|i| SolidMaterial::of(scene, i)).collect();
        self.material_programs = None;
        let mut live_textures = std::collections::HashSet::new();
        for material in &self.materials {
            for source in [
                &material.base_color_source,
                &material.metalness_source,
                &material.roughness_source,
                &material.normal_source,
            ] {
                if let Source::Texture { texture, .. } = source {
                    let key = Arc::as_ptr(texture) as usize;
                    live_textures.insert(key);
                    if let std::collections::hash_map::Entry::Vacant(slot) = self.textures.entry(key) {
                        slot.insert(upload_texture(gl, texture)?);
                    }
                }
            }
        }
        let mut live_labels = std::collections::HashSet::new();
        for label in self.materials.iter().flat_map(|m| &m.labels) {
            let key = Arc::as_ptr(&label.texture) as usize;
            live_labels.insert(key);
            if let std::collections::hash_map::Entry::Vacant(slot) = self.label_textures.entry(key) {
                slot.insert(upload_label(gl, &label.texture)?);
            }
        }
        let stale_labels: Vec<usize> = self.label_textures.keys().copied().filter(|k| !live_labels.contains(k)).collect();
        for key in stale_labels {
            if let Some(texture) = self.label_textures.remove(&key) {
                gl.delete_texture(texture.texture);
            }
        }
        let mut live_volumes = std::collections::HashSet::new();
        for material in &self.materials {
            if let Some(grid) = &material.volume {
                let key = Arc::as_ptr(grid) as usize;
                live_volumes.insert(key);
                if let std::collections::hash_map::Entry::Vacant(slot) = self.volumes.entry(key) {
                    slot.insert(upload_volume(gl, grid)?);
                }
            }
        }
        let stale_volumes: Vec<usize> = self.volumes.keys().copied().filter(|k| !live_volumes.contains(k)).collect();
        for key in stale_volumes {
            if let Some(volume) = self.volumes.remove(&key) {
                gl.delete_texture(volume.texture);
            }
        }
        let stale: Vec<usize> = self.textures.keys().copied().filter(|k| !live_textures.contains(k)).collect();
        for key in stale {
            if let Some(texture) = self.textures.remove(&key) {
                gl.delete_texture(texture.texture);
            }
        }
        self.materials_for = Some(scene.clone());
        Ok(())
    }
}

/// Interleaved position, normal and UV. A mesh without vertex normals is
/// faceted, and is drawn faceted: each triangle gets its own three vertices
/// and its face normal, rather than a smooth normal nobody authored.
/// Floats per vertex: position, normal, UV, and the tangent frame (`xyzw`,
/// all zero for a mesh without one).
const VERTEX_FLOATS: usize = 12;

fn vertex_data(mesh: &Mesh) -> (Vec<f32>, Option<Vec<u32>>) {
    let has_uv = mesh.uvs.len() == mesh.positions.len();
    let uv = |i: usize| if has_uv { [mesh.uvs[i].x, mesh.uvs[i].y] } else { [0.0, 0.0] };
    let has_tangents = mesh.tangents.len() == mesh.positions.len();
    let tangent = |i: usize| {
        if has_tangents {
            let t = mesh.tangents[i];
            [t.x, t.y, t.z, t.w]
        } else {
            [0.0; 4]
        }
    };
    if mesh.normals.len() == mesh.positions.len() {
        let mut data = Vec::with_capacity(mesh.positions.len() * VERTEX_FLOATS);
        for (i, (p, n)) in mesh.positions.iter().zip(&mesh.normals).enumerate() {
            let (t, g) = (uv(i), tangent(i));
            data.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z, t[0], t[1], g[0], g[1], g[2], g[3]]);
        }
        (data, Some(mesh.indices.clone()))
    } else {
        let mut data = Vec::with_capacity(mesh.indices.len() * VERTEX_FLOATS);
        for triangle in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize];
            let n = (mesh.positions[b] - mesh.positions[a])
                .cross(mesh.positions[c] - mesh.positions[a])
                .try_normalize()
                .unwrap_or(Vec3::Y);
            for i in [a, b, c] {
                let (p, t, g) = (mesh.positions[i], uv(i), tangent(i));
                data.extend_from_slice(&[p.x, p.y, p.z, n.x, n.y, n.z, t[0], t[1], g[0], g[1], g[2], g[3]]);
            }
        }
        (data, None)
    }
}

unsafe fn upload_mesh(gl: &glow::Context, mesh: &Arc<Mesh>) -> Result<GpuMesh, String> {
    let (data, indices) = vertex_data(mesh);
    let vao = gl.create_vertex_array()?;
    gl.bind_vertex_array(Some(vao));
    let vbo = gl.create_buffer()?;
    gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes_of(&data), glow::STATIC_DRAW);
    let stride = (VERTEX_FLOATS * 4) as i32;
    for (location, size, offset) in [(0u32, 3, 0), (1, 3, 12), (2, 2, 24), (3, 4, 32)] {
        gl.enable_vertex_attrib_array(location);
        gl.vertex_attrib_pointer_f32(location, size, glow::FLOAT, false, stride, offset);
    }
    let mut buffers = vec![vbo];
    let (count, indexed) = match indices {
        Some(indices) => {
            let ebo = gl.create_buffer()?;
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(ebo));
            gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytes_of(&indices), glow::STATIC_DRAW);
            buffers.push(ebo);
            (indices.len() as i32, true)
        }
        // Each vertex is VERTEX_FLOATS long: dividing by eight drew half as
        // many again as there are, off the end of the buffer.
        None => ((data.len() / VERTEX_FLOATS) as i32, false),
    };
    gl.bind_vertex_array(None);

    // The wireframe's lines: every edge once, over the same vertices.
    let edges = edge_indices(mesh, count as usize, indexed);
    let edge_vao = gl.create_vertex_array()?;
    gl.bind_vertex_array(Some(edge_vao));
    gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, stride, 0);
    let edge_buffer = gl.create_buffer()?;
    gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(edge_buffer));
    gl.buffer_data_u8_slice(glow::ELEMENT_ARRAY_BUFFER, bytes_of(&edges), glow::STATIC_DRAW);
    buffers.push(edge_buffer);
    gl.bind_vertex_array(None);

    // The model's own edges: a CAD part's B-rep edges, or a mesh's creases
    // and borders when it brought none.
    let lines = if mesh.edge_lines.len() >= 2 {
        mesh.edge_lines.clone()
    } else {
        tequila_scene::measure::FeatureEdges::of(mesh).segments(mesh)
    };
    let line_data: Vec<f32> = lines.iter().flat_map(|p| [p.x, p.y, p.z]).collect();
    let model_vao = gl.create_vertex_array()?;
    gl.bind_vertex_array(Some(model_vao));
    let model_buffer = gl.create_buffer()?;
    gl.bind_buffer(glow::ARRAY_BUFFER, Some(model_buffer));
    gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes_of(&line_data), glow::STATIC_DRAW);
    gl.enable_vertex_attrib_array(0);
    gl.vertex_attrib_pointer_f32(0, 3, glow::FLOAT, false, 12, 0);
    buffers.push(model_buffer);
    gl.bind_vertex_array(None);
    gl.bind_buffer(glow::ARRAY_BUFFER, None);
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for p in &mesh.positions {
        low = low.min(*p);
        high = high.max(*p);
    }
    Ok(GpuMesh { bounds: (low, high), vao, buffers, count, indexed, edge_vao, edge_count: edges.len() as i32, model_vao, model_count: (lines.len() / 2 * 2) as i32, _mesh: mesh.clone() })
}

/// Every edge of a mesh once, as pairs of indices into its vertex data: an
/// indexed mesh's from its triangles, shared edges counted once; a faceted
/// mesh's - three vertices a triangle - from each triangle's own three.
fn edge_indices(mesh: &Mesh, vertices: usize, indexed: bool) -> Vec<u32> {
    let mut edges = Vec::new();
    if indexed {
        let mut seen = std::collections::HashSet::with_capacity(mesh.indices.len());
        for t in mesh.indices.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let key = (a.min(b), a.max(b));
                if seen.insert(key) {
                    edges.extend_from_slice(&[key.0, key.1]);
                }
            }
        }
    } else {
        for t in 0..vertices / 3 {
            let i = (t * 3) as u32;
            edges.extend_from_slice(&[i, i + 1, i + 1, i + 2, i + 2, i]);
        }
    }
    edges
}

const ENVIRONMENT_WIDTH: usize = 256;
const ENVIRONMENT_HEIGHT: usize = 128;

/// An environment as Material Preview reads it.
struct EnvironmentMap {
    /// Linear radiance in the rendering space, RGB, row by row from the top.
    radiance: Vec<f32>,
    /// Nine spherical-harmonic coefficients of the radiance, RGB each.
    irradiance: [f32; 27],
}

/// The direction an equirectangular texel looks along, as the shader's
/// `equirect` reads it back.
fn texel_direction(u: f32, v: f32) -> Vec3 {
    let phi = (u - 0.5) * std::f32::consts::TAU;
    let theta = v * std::f32::consts::PI;
    tequila_core::math::vec3(theta.sin() * phi.sin(), theta.cos(), theta.sin() * phi.cos())
}

/// What the camera sees behind the model, as the render's background shows
/// it: (width, height, RGB), and whether it lies on the frame (a backplate)
/// rather than over the sphere.
fn background_map(scene: &SceneSnapshot) -> ((usize, usize, Vec<f32>), bool) {
    use tequila_render_core::environment::Background;
    match &scene.background {
        Background::Backplate { .. } => {
            let (w, h) = (512, 288);
            let mut data = Vec::with_capacity(w * h * 3);
            for y in 0..h {
                for x in 0..w {
                    let film = tequila_core::math::Vec2 { x: (x as f32 + 0.5) / w as f32, y: (y as f32 + 0.5) / h as f32 };
                    let c = scene.background.radiance(&scene.environment, Vec3::ZERO, Vec3::Y, film);
                    data.extend_from_slice(&[c.r, c.g, c.b]);
                }
            }
            ((w, h, data), true)
        }
        _ => {
            let (w, h) = (512, 256);
            let mut data = Vec::with_capacity(w * h * 3);
            for y in 0..h {
                for x in 0..w {
                    let d = texel_direction((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
                    let c = scene.background.radiance(&scene.environment, Vec3::ZERO, d, tequila_core::math::Vec2 { x: 0.5, y: 0.5 });
                    data.extend_from_slice(&[c.r, c.g, c.b]);
                }
            }
            ((w, h, data), false)
        }
    }
}

fn environment_map(environment: &tequila_render_core::environment::Environment) -> EnvironmentMap {
    let (w, h) = (ENVIRONMENT_WIDTH, ENVIRONMENT_HEIGHT);
    let mut radiance = Vec::with_capacity(w * h * 3);
    let mut irradiance = [0.0f32; 27];
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        // Each texel's solid angle.
        let area = (std::f32::consts::TAU / w as f32) * (std::f32::consts::PI / h as f32) * (v * std::f32::consts::PI).sin();
        for x in 0..w {
            let d = texel_direction((x as f32 + 0.5) / w as f32, v);
            let l = environment.radiance(d);
            radiance.extend_from_slice(&[l.r, l.g, l.b]);
            let basis = [
                0.282095,
                0.488603 * d.y,
                0.488603 * d.z,
                0.488603 * d.x,
                1.092548 * d.x * d.y,
                1.092548 * d.y * d.z,
                0.315392 * (3.0 * d.z * d.z - 1.0),
                1.092548 * d.x * d.z,
                0.546274 * (d.x * d.x - d.y * d.y),
            ];
            for (i, b) in basis.iter().enumerate() {
                irradiance[i * 3] += l.r * b * area;
                irradiance[i * 3 + 1] += l.g * b * area;
                irradiance[i * 3 + 2] += l.b * b * area;
            }
        }
    }
    EnvironmentMap { radiance, irradiance }
}

/// What the ground shadow depends on, as one number: where each part is and
/// whether it casts, and the environment it leans away from. Not the
/// materials, so dragging a colour slider does not redraw it.
fn shadow_source(scene: &SceneSnapshot) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (Arc::as_ptr(&scene.environment) as usize).hash(&mut h);
    for instance in &scene.instances {
        (Arc::as_ptr(&scene.meshes[instance.mesh.index()]) as usize).hash(&mut h);
        for row in &instance.transform.matrix.m {
            for v in row {
                v.to_bits().hash(&mut h);
            }
        }
        (casts_ground_shadow(instance), instance.shadow_catcher.as_ref().map(|c| (c.occlusion_only, c.shadows))).hash(&mut h);
    }
    h.finish()
}

/// Whether a part darkens the ground: what the camera sees and what casts
/// shadows in the render, and not the floor itself.
fn casts_ground_shadow(instance: &tequila_render_core::scene::Instance) -> bool {
    instance.visible_to_camera && instance.casts_shadows && instance.shadow_catcher.is_none() && !instance.light_proxy
}

/// Where the ground shadow lies for `scene`: under the parts that cast one,
/// on the floor the render catches shadows on. None without a floor, with a
/// floor that catches none, or with nothing near enough to it to darken it.
fn shadow_footprint(scene: &SceneSnapshot) -> Option<ShadowFootprint> {
    let floor = scene.instances.iter().find(|i| i.shadow_catcher.is_some_and(|c| c.shadows))?;
    let plane = scene.mesh(floor.mesh).bounds().transformed(&floor.transform).max.y;
    let mut model = tequila_core::math::Aabb::EMPTY;
    for instance in scene.instances.iter().filter(|i| casts_ground_shadow(i)) {
        model.extend(&scene.mesh(instance.mesh).bounds().transformed(&instance.transform));
    }
    if model.is_empty() {
        return None;
    }
    let size = model.diagonal();
    let across = size.x.max(size.z).max(1e-6);
    // A studio's light comes mostly from above, so the floor darkens under
    // the whole of the model seen from overhead, a little less where the
    // surface over it is higher: a sphere's shadow is its disc, softening
    // towards the rim. Measured against the model's own height, not its
    // top's height above the floor, so lifting it off the floor does not
    // stretch the shadow out.
    let falloff = (size.y * 2.5).max(across * 0.2);
    // And the floor goes nearly black right where the model meets it, as
    // the render's does: light cannot get into the gap.
    let contact_falloff = (size.y * 0.6).max(across * 0.05);
    if model.min.y - plane >= falloff {
        return None;
    }
    let catcher = floor.shadow_catcher.as_ref().expect("found by it");
    let centre = model.center();
    let skew = light_lean(&scene.environment);
    let lean = (skew.0 * skew.0 + skew.1 * skew.1).sqrt();
    Some(ShadowFootprint {
        centre: (centre.x, centre.z),
        // Room for the blur to spread beyond the silhouette, and for the
        // shadow to lean.
        half: across * 1.5 + lean * size.y,
        plane,
        falloff,
        contact_falloff,
        strength: if catcher.occlusion_only { 0.45 } else { 0.64 },
        contact_strength: 0.93,
        skew,
    })
}

/// Which way the environment's light leans, as the floor receives it: the
/// shift, in x and z per unit of height, of a shadow cast along it.
///
/// The irradiance-weighted mean direction of the sky over the floor. A sky
/// even all round has none, and its shadows fall straight down; a sun or a
/// softbox off to one side moves them away from itself, as far as the
/// light is one-sided.
fn light_lean(environment: &tequila_render_core::environment::Environment) -> (f32, f32) {
    let (w, h) = (48usize, 12usize);
    let (mut sum, mut total) = (Vec3::ZERO, 0.0f32);
    for y in 0..h {
        // The upper hemisphere only: light from below the floor casts no
        // shadow on it.
        let elevation = (y as f32 + 0.5) / h as f32 * std::f32::consts::FRAC_PI_2;
        let (up, around) = (elevation.sin(), elevation.cos());
        for x in 0..w {
            let azimuth = (x as f32 + 0.5) / w as f32 * std::f32::consts::TAU;
            let d = tequila_core::math::vec3(around * azimuth.cos(), up, around * azimuth.sin());
            let l = environment.radiance(d);
            let luminance = 0.2126 * l.r + 0.7152 * l.g + 0.0722 * l.b;
            // Received by the floor as its cosine, over a band that narrows
            // towards the zenith.
            let weight = luminance.max(0.0) * up * around;
            sum = sum + d * weight;
            total += weight;
        }
    }
    if total <= 0.0 {
        return (0.0, 0.0);
    }
    let mean = sum * (1.0 / total);
    // An even sky's mean is straight up at two thirds; anything sideways is
    // the light's lean. Kept within a long evening shadow's length.
    let even = 2.0 / 3.0;
    let tilt = (-mean.x / mean.y.max(even * 0.25), -mean.z / mean.y.max(even * 0.25));
    let length = (tilt.0 * tilt.0 + tilt.1 * tilt.1).sqrt();
    if length > 1.5 {
        (tilt.0 * 1.5 / length, tilt.1 * 1.5 / length)
    } else {
        tilt
    }
}

fn bytes_of<T: Copy>(data: &[T]) -> &[u8] {
    // SAFETY: f32 and u32 have no padding and every bit pattern is a valid
    // byte; the slice covers exactly the vector's initialized elements.
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) }
}

/// The largest texture uploaded. Past this a solid view gains nothing a
/// person can see, and a few 8K maps would otherwise fill the card.
const TEXTURE_LIMIT: usize = 2048;

fn encode_srgb8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

unsafe fn upload_texture(gl: &glow::Context, texture: &Arc<Texture>) -> Result<GpuTexture, String> {
    let pyramid = texture.pyramid();
    let level = (0..pyramid.levels())
        .find(|&l| pyramid.level(l).width.max(pyramid.level(l).height) <= TEXTURE_LIMIT)
        .unwrap_or(pyramid.levels() - 1);
    let image = pyramid.level(level);
    // Colour is stored linear; sent as sRGB-encoded bytes into an sRGB
    // texture, so filtering happens on linear values and eight bits are
    // spent where the eye needs them. Data maps are sent as they are.
    let colour = texture.role == MapRole::Color;
    let channels = image.channels.max(1);
    let mut bytes = Vec::with_capacity(image.width * image.height * 4);
    for texel in image.data.chunks_exact(channels) {
        let get = |c: usize| texel.get(c).copied().unwrap_or(if channels == 1 { texel[0] } else { 0.0 });
        let (r, g, b) = (get(0), if channels >= 3 { get(1) } else { get(0) }, if channels >= 3 { get(2) } else { get(0) });
        let a = if channels == 4 { texel[3] } else { 1.0 };
        let encode = |v: f32| if colour { encode_srgb8(v) } else { (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };
        bytes.extend_from_slice(&[encode(r), encode(g), encode(b), (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
    }
    let handle = gl.create_texture()?;
    gl.bind_texture(glow::TEXTURE_2D, Some(handle));
    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
    gl.tex_image_2d(
        glow::TEXTURE_2D,
        0,
        (if colour { glow::SRGB8_ALPHA8 } else { glow::RGBA8 }) as i32,
        image.width as i32,
        image.height as i32,
        0,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        Some(&bytes),
    );
    gl.generate_mipmap(glow::TEXTURE_2D);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    gl.bind_texture(glow::TEXTURE_2D, None);
    Ok(GpuTexture { texture: handle, _source: texture.clone() })
}

/// A label's artwork as `Texture::sample_decal` reads it: colour multiplied
/// by coverage before filtering, so an edge does not pick up whatever colour
/// the clear pixels happen to hold, and clear past the picture's edges.
unsafe fn upload_label(gl: &glow::Context, texture: &Arc<Texture>) -> Result<GpuTexture, String> {
    let pyramid = texture.pyramid();
    let level = (0..pyramid.levels())
        .find(|&l| pyramid.level(l).width.max(pyramid.level(l).height) <= TEXTURE_LIMIT)
        .unwrap_or(pyramid.levels() - 1);
    let image = pyramid.level(level);
    let colour = texture.role == MapRole::Color;
    let channels = image.channels.max(1);
    let mut bytes = Vec::with_capacity(image.width * image.height * 4);
    for texel in image.data.chunks_exact(channels) {
        let grey = channels < 3;
        let (r, g, b) = if grey { (texel[0], texel[0], texel[0]) } else { (texel[0], texel[1], texel[2]) };
        let a = if channels == 4 { texel[3].clamp(0.0, 1.0) } else { 1.0 };
        let encode = |v: f32| if colour { encode_srgb8(v * a) } else { ((v * a).clamp(0.0, 1.0) * 255.0 + 0.5) as u8 };
        bytes.extend_from_slice(&[encode(r), encode(g), encode(b), (a * 255.0 + 0.5) as u8]);
    }
    let handle = gl.create_texture()?;
    gl.bind_texture(glow::TEXTURE_2D, Some(handle));
    gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
    gl.tex_image_2d(
        glow::TEXTURE_2D,
        0,
        (if colour { glow::SRGB8_ALPHA8 } else { glow::RGBA8 }) as i32,
        image.width as i32,
        image.height as i32,
        0,
        glow::RGBA,
        glow::UNSIGNED_BYTE,
        Some(&bytes),
    );
    gl.generate_mipmap(glow::TEXTURE_2D);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    // Clear past the edges: a label is printed once, never tiled.
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_BORDER as i32);
    gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_BORDER as i32);
    gl.tex_parameter_f32_slice(glow::TEXTURE_2D, glow::TEXTURE_BORDER_COLOR, &[0.0, 0.0, 0.0, 0.0]);
    gl.bind_texture(glow::TEXTURE_2D, None);
    Ok(GpuTexture { texture: handle, _source: texture.clone() })
}

/// A Volume Map's grid as a 3D texture: one float a voxel, filtered
/// trilinearly as the renderer samples it, and zero past its edges.
unsafe fn upload_volume(gl: &glow::Context, grid: &Arc<tequila_texture::volume::VolumeGrid>) -> Result<GpuVolume, String> {
    let handle = gl.create_texture()?;
    gl.bind_texture(glow::TEXTURE_3D, Some(handle));
    let [x, y, z] = grid.resolution;
    gl.tex_image_3d(
        glow::TEXTURE_3D,
        0,
        glow::R32F as i32,
        x as i32,
        y as i32,
        z as i32,
        0,
        glow::RED,
        glow::FLOAT,
        Some(bytes_of(&grid.data)),
    );
    for wrap in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T, glow::TEXTURE_WRAP_R] {
        gl.tex_parameter_i32(glow::TEXTURE_3D, wrap, glow::CLAMP_TO_BORDER as i32);
    }
    gl.tex_parameter_f32_slice(glow::TEXTURE_3D, glow::TEXTURE_BORDER_COLOR, &[0.0, 0.0, 0.0, 0.0]);
    gl.tex_parameter_i32(glow::TEXTURE_3D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
    gl.tex_parameter_i32(glow::TEXTURE_3D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
    gl.bind_texture(glow::TEXTURE_3D, None);
    Ok(GpuVolume { texture: handle, _source: grid.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Material Preview's environment: a uniform sky gives a uniform map,
    /// and irradiance pi times its radiance from every direction, as a
    /// Lambertian surface under it receives.
    #[test]
    fn a_uniform_sky_lights_evenly() {
        let environment = tequila_render_core::environment::Environment::uniform(Spectrum::splat(0.5));
        let map = environment_map(&environment);
        assert!(map.radiance.iter().all(|v| (v - 0.5).abs() < 1e-4));
        // The irradiance as the shader sums it, at a few normals.
        for n in [Vec3::Y, -Vec3::Y, tequila_core::math::vec3(0.6, 0.0, 0.8)] {
            let c = |i: usize| map.irradiance[i * 3];
            let e = c(0) * 0.886227
                + c(1) * 1.023328 * n.y
                + c(2) * 1.023328 * n.z
                + c(3) * 1.023328 * n.x
                + c(4) * 0.858086 * n.x * n.y
                + c(5) * 0.858086 * n.y * n.z
                + c(6) * 0.247708 * (3.0 * n.z * n.z - 1.0)
                + c(7) * 0.858086 * n.x * n.z
                + c(8) * 0.429043 * (n.x * n.x - n.y * n.y);
            assert!((e - std::f32::consts::PI * 0.5).abs() < 0.02, "irradiance {e} at {n:?}");
        }
    }

    /// The texel directions round-trip through the shader's projection.
    #[test]
    fn environment_texels_look_where_the_shader_reads() {
        for (u, v) in [(0.1f32, 0.3f32), (0.5, 0.5), (0.9, 0.8)] {
            let d = texel_direction(u, v);
            let back_u = d.x.atan2(d.z) / std::f32::consts::TAU + 0.5;
            let back_v = d.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
            assert!((back_u - u).abs() < 1e-4 && (back_v - v).abs() < 1e-4, "{u},{v} came back {back_u},{back_v}");
        }
    }

    /// Each edge of an indexed mesh once; a faceted one's from each triangle.
    #[test]
    fn edges_are_counted_once() {
        let quad = Mesh::new(
            vec![Vec3::ZERO, tequila_core::math::vec3(1.0, 0.0, 0.0), tequila_core::math::vec3(1.0, 1.0, 0.0), tequila_core::math::vec3(0.0, 1.0, 0.0)],
            vec![0, 1, 2, 0, 2, 3],
        );
        assert_eq!(edge_indices(&quad, 4, true).len() / 2, 5, "four sides and the diagonal");
        assert_eq!(edge_indices(&quad, 6, false).len() / 2, 6);
    }

    /// A medium whose Density a Volume Map drives reaches the Solid view
    /// with its grid, so the view can march it, and with the extinction and
    /// colour of the medium at density 1.
    #[test]
    fn a_volume_map_reaches_the_solid_view() {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind as Kind, Socket, TextureId};
        use tequila_materials::material_type::MaterialType;
        let grid = Arc::new(tequila_texture::volume::VolumeGrid::new(
            "density".into(),
            vec![1.0; 8],
            [2, 2, 2],
            Vec3::ZERO,
            Vec3::splat(0.5),
        ));
        let mut table = tequila_render_core::shade::TextureTable::new();
        table.insert_volume(TextureId(3), grid.clone());
        let medium = MaterialType::ScatteringMedium.defaults();
        let mut graph = MaterialGraph::from_material(medium.clone());
        let map = graph.add_node(Node::new(Kind::VolumeMap { volume: TextureId(3) }));
        let output = graph.output();
        graph.connect(map, output, Socket::Param(Param::VolumeDensity));
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![medium],
            Arc::new(tequila_render_core::environment::Environment::uniform(tequila_core::spectrum::Spectrum::ONE)),
        )
        .with_programs(vec![Some(Arc::new(graph.compile().unwrap()))], Arc::new(table));
        let solid = SolidMaterial::of(&scene, 0);
        assert!(solid.volume.as_ref().is_some_and(|g| Arc::ptr_eq(g, &grid)), "the grid did not reach the Solid view");
        assert!(solid.volume_extinction > 0.0 && solid.volume_albedo.luminance() > 0.5);
    }

    /// The GPU's UV arithmetic is the CPU mapping's, for every option.
    #[test]
    fn uv_transforms_match_the_cpu_mapping() {
        use tequila_texture::projection::SurfacePoint;
        let mapping = Mapping {
            scale: tequila_core::math::vec3(2.0, 3.0, 1.0),
            offset: tequila_core::math::Vec2 { x: 0.25, y: -0.5 },
            rotation: 0.125,
            pivot: tequila_core::math::Vec2 { x: 0.5, y: 0.5 },
            flip_u: true,
            ..Default::default()
        };
        let transform = UvTransform::of(&mapping).expect("a UV mapping");
        for (u, v) in [(0.1f32, 0.2f32), (0.9, 0.7), (0.5, 0.5)] {
            let point = SurfacePoint {
                position: Vec3::ZERO,
                normal: Vec3::Z,
                uv: tequila_core::math::Vec2 { x: u, y: v },
                footprint: 0.0,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                world: Default::default(),
            };
            let cpu = mapping.project(&point)[0].coords;
            let gpu = transform.apply(u, v);
            assert!((cpu.u - gpu.0).abs() < 1e-5 && (cpu.v - gpu.1).abs() < 1e-5, "({u},{v}): cpu {:?} gpu {gpu:?}", (cpu.u, cpu.v));
        }
    }

    #[test]
    fn only_first_set_uv_mappings_reach_the_gpu() {
        assert!(UvTransform::of(&Mapping { uv_set: 1, ..Default::default() }).is_none());
        assert!(UvTransform::of(&Mapping { projection: UvProjection::TriPlanar, ..Default::default() }).is_none());
    }

    /// A faceted mesh is split per triangle with its face normal; a smooth
    /// one keeps its vertices and indices.
    /// A box of the given size standing on the ground at `x`, with a floor.
    fn standing(size: Vec3, x: f32, lift: f32) -> SceneSnapshot {
        use tequila_core::ids::{InstanceId, MaterialId, MeshId};
        use tequila_render_core::scene::{Instance, ShadowCatcher};
        use tequila_core::math::{vec3, Transform};
        let cube = Mesh::new(
            vec![vec3(-0.5, 0.0, -0.5), vec3(0.5, 0.0, -0.5), vec3(0.5, 1.0, 0.5), vec3(-0.5, 1.0, 0.5)],
            vec![0, 1, 2, 0, 2, 3],
        );
        let floor = Mesh::new(vec![vec3(-50.0, 0.0, -50.0), vec3(50.0, 0.0, -50.0), vec3(50.0, 0.0, 50.0)], vec![0, 1, 2]);
        let place = Transform::from_matrix(Mat4::translation(vec3(x, lift, 0.0)) * Mat4::scale(size)).unwrap();
        SceneSnapshot::new(
            vec![Arc::new(cube), Arc::new(floor)],
            vec![
                Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(place),
                Instance::new(InstanceId(1), MeshId(1), MaterialId(0)).with_shadow_catcher(ShadowCatcher::default()),
            ],
            vec![tequila_render_core::material::Material::diffuse(Spectrum::splat(0.5))],
            Arc::new(tequila_render_core::environment::Environment::uniform(Spectrum::ONE)),
        )
    }

    /// The shadow lies under the model, on the floor's surface, sized by the
    /// model and not by the floor.
    #[test]
    fn the_ground_shadow_lies_under_the_model() {
        let f = shadow_footprint(&standing(Vec3::splat(2.0), 3.0, 0.0)).expect("a model on a floor");
        assert!((f.centre.0 - 3.0).abs() < 1e-5 && f.centre.1.abs() < 1e-5, "under the model: {f:?}");
        assert!(f.plane.abs() < 1e-6, "on the floor's surface");
        assert!(f.half > 1.0 && f.half < 5.0, "the model's size, with room to blur, not the floor's: {}", f.half);
        assert!(f.strength > 0.0 && f.falloff > 0.0);
        assert!(f.contact_falloff < f.falloff && f.contact_strength > f.strength, "contact is tighter and darker");
        assert!(f.skew.0.abs() < 0.02 && f.skew.1.abs() < 0.02, "an even sky casts straight down: {:?}", f.skew);
    }

    /// No floor, no shadow; none on a floor kept only for its reflection
    /// (Catch shadows unticked); and none from a model too far above the
    /// floor to darken it.
    #[test]
    fn no_floor_or_a_high_model_casts_no_ground_shadow() {
        let mut scene = standing(Vec3::splat(1.0), 0.0, 0.0);
        assert!(shadow_footprint(&scene).is_some());
        scene.instances[1].shadow_catcher = Some(tequila_render_core::scene::ShadowCatcher { shadows: false, reflections: 0.5, ..Default::default() });
        assert!(shadow_footprint(&scene).is_none(), "a floor that catches no shadow");
        scene.instances.truncate(1);
        assert!(shadow_footprint(&scene).is_none(), "nothing to catch it");
        assert!(shadow_footprint(&standing(Vec3::splat(1.0), 0.0, 5.0)).is_none(), "five heights up");
    }

    #[test]
    fn meshes_become_vertex_data() {
        let mut mesh = Mesh::new(vec![Vec3::ZERO, Vec3::X, Vec3::Y], vec![0, 1, 2]);
        let (data, indices) = vertex_data(&mesh);
        assert!(indices.is_none());
        assert_eq!(data.len(), 3 * VERTEX_FLOATS);
        assert_eq!(&data[3..6], &[0.0, 0.0, 1.0], "the face normal of a counter-clockwise triangle in XY");
        assert_eq!(&data[8..12], &[0.0; 4], "no tangents, so the shader falls back to derivatives");
        mesh.normals = vec![Vec3::Z; 3];
        let (data, indices) = vertex_data(&mesh);
        assert_eq!(indices.unwrap(), vec![0, 1, 2]);
        assert_eq!(data.len(), 3 * VERTEX_FLOATS);
        let mesh = mesh.with_tangents(vec![tequila_core::math::vec4(1.0, 0.0, 0.0, -1.0); 3]);
        let (data, _) = vertex_data(&mesh);
        assert_eq!(&data[VERTEX_FLOATS + 8..VERTEX_FLOATS + 12], &[1.0, 0.0, 0.0, -1.0], "the second vertex's tangent");
    }

    #[test]
    fn a_perspective_projection_puts_the_target_in_the_middle() {
        let camera = Camera::look_at(tequila_core::math::vec3(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let (projection, orthographic) = projection(&camera, 16.0 / 9.0, 0.1, 100.0).unwrap();
        assert!(!orthographic);
        let view = camera.camera_to_world.inverse().unwrap();
        let clip = projection * view;
        let m = clip.m;
        let (x, y, w) = (m[0][3], m[1][3], m[3][3]);
        assert!((x / w).abs() < 1e-5 && (y / w).abs() < 1e-5, "the origin is at the centre of the frame");
        let z = m[2][3] / w;
        assert!((-1.0..=1.0).contains(&z), "and inside the depth range: {z}");
    }

    /// The Quest 3's materials: base colour from one texture, roughness and
    /// metalness from channels of a packed one — all reach the GPU.
    #[test]
    #[ignore]
    fn the_quest_materials_are_understood() {
        let path = std::env::var("TEQUILA_BENCH_SCENE").expect("set TEQUILA_BENCH_SCENE");
        let loaded = tequila_format::load(std::path::Path::new(&path)).unwrap();
        let project = crate::project::Project::with_document(loaded.document, Arc::new(tequila_scene::demo::studio_environment()));
        let scene = project.scene();
        let textured: Vec<SolidMaterial> = (0..scene.materials.len())
            .map(|i| SolidMaterial::of(&scene, i))
            .filter(|m| m.base_color_source != Source::Constant)
            .collect();
        assert!(!textured.is_empty());
        for material in textured {
            assert!(matches!(material.metalness_source, Source::Texture { channel: Some(2), .. }));
            assert!(matches!(material.roughness_source, Source::Texture { channel: Some(1), .. }));
            assert!(matches!(material.normal_source, Source::Texture { channel: None, .. }), "the normal map was not picked up");
        }
    }

    /// A normal map wired into Bump reaches the Solid view; a height through
    /// a Bump node does not, since drawing one means evaluating the graph
    /// either side of every pixel.
    #[test]
    fn normal_maps_reach_the_solid_view_and_heights_do_not() {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind as Kind, Socket, TextureId, Value};
        use tequila_materials::material_type::MaterialType;
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::Mapping;
        use tequila_texture::texture::{MapRole, Texture};

        let scene_with = |role: MapRole, through_bump_node: bool| {
            let image = Image { width: 2, height: 2, channels: 3, data: [0.5, 0.5, 1.0].repeat(4), color_space: ColorSpace::Raw };
            let mut table = tequila_render_core::shade::TextureTable::new();
            table.insert(TextureId(0), Arc::new(Texture::from_linear(image, role, Mapping::default())));
            let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
            let map = graph.add_node(Node::new(Kind::ImageTexture { texture: TextureId(0), fallback: Value::Float(0.0) }));
            let output = graph.output();
            if through_bump_node {
                let bump = graph.add_node(Node::new(Kind::Bump { depth: 0.001 }));
                graph.connect(map, bump, Socket::Input(0));
                graph.connect(bump, output, Socket::Param(Param::Bump));
            } else {
                graph.connect(map, output, Socket::Param(Param::Bump));
            }
            let program = graph.compile().unwrap();
            SceneSnapshot::new(
                Vec::new(),
                Vec::new(),
                vec![tequila_render_core::material::Material::default()],
                Arc::new(tequila_render_core::environment::Environment::uniform(tequila_core::spectrum::Spectrum::ONE)),
            )
            .with_programs(vec![Some(Arc::new(program))], Arc::new(table))
        };
        let normal = SolidMaterial::of(&scene_with(MapRole::Normal, false), 0);
        assert!(matches!(normal.normal_source, Source::Texture { channel: None, .. }));
        let height = SolidMaterial::of(&scene_with(MapRole::Height, true), 0);
        assert_eq!(height.normal_source, Source::Constant);
    }
}
