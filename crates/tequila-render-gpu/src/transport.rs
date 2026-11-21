//! The transport on the card (`docs/07` R2.5, R3): camera rays, triangles,
//! the scene's OpenPBR materials, the environment sampled by its own tables
//! with next-event estimation and MIS, the analytic lamps - their light
//! samples, the lamps a sharp enough bounce meets and the stand-ins the
//! camera sees - glass - the media a path is in, absorption, shadow rays
//! through clear glass, the glass a light sample passed and the glass a
//! seeing bounce sees, the split at the first plain glass the camera sees -
//! Russian roulette and the bounce limit, and the shadow catcher's floor -
//! its shadow, its reflection, the backdrop behind it - as the processor's
//! integrator has them (`transport.wgsl`). What of a floor, of the lamps or
//! of glass and the other materials the card renders otherwise, it says
//! ([`floor_limits`], [`crate::lights::lamp_limits`], [`glass_limits`]).
//!
//! The card hands each sample back, radiance and the background its camera
//! ray saw, and the processor's [`Framebuffer`] accumulates them in sample
//! order, firefly rejection and all (`docs/07` D8), so the two renderers'
//! pictures differ only where their paths do.

use bytemuck::{Pod, Zeroable};
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::{Camera, Projection};
use tequila_render_core::environment::Background;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::integrator::PathResult;
use tequila_render_core::scene::SceneSnapshot;
use tequila_render_core::settings::{IntegratorConfig, ShadowGlass};

use crate::device::GpuDevice;
use crate::environment::GpuEnvironment;
use crate::lights::{GpuLights, LampScene, LightRecords};
use crate::scene::GpuScene;
use crate::tables::Tables;
use crate::textures::{CardPictures, Pictures};
use crate::variants::{KernelCache, KernelFeatures, SceneRecords};

/// Every material's program lowered for the card (`crate::program::lower`),
/// by material: `Ok(None)` for a material that is its constants, `Err` with
/// the reason for one the card cannot run. Its pictures as the scene's
/// (`CardPictures::of`), on a card that binds them all.
pub fn lower_programs(scene: &SceneSnapshot) -> Vec<Result<Option<crate::program::CardProgram>, crate::program::Refusal>> {
    let pictures = CardPictures::of(scene);
    lower_programs_with(scene, Pictures::On(&pictures))
}

/// [`lower_programs`] with the scene's pictures as `pictures`.
pub fn lower_programs_with(scene: &SceneSnapshot, pictures: Pictures) -> Vec<Result<Option<crate::program::CardProgram>, crate::program::Refusal>> {
    (0..scene.materials.len())
        .map(|i| match scene.program(tequila_core::ids::MaterialId(i as u32)) {
            Some(program) => crate::program::lower_with(program, pictures),
            None => Ok(None),
        })
        .collect()
}

/// Whether `pictures` fit the buffers `device` binds: `Pictures::On`, or
/// how far over they are.
pub fn pictures_on<'a>(pictures: &'a CardPictures, limits: &wgpu::Limits) -> Pictures<'a> {
    let limit = limits.max_storage_buffer_binding_size.min(limits.max_buffer_size);
    let bytes = pictures.texel_bytes().max(pictures.header_bytes());
    if bytes > limit {
        Pictures::TooLarge { bytes, limit }
    } else {
        Pictures::On(pictures)
    }
}

/// What a dispatch of `transport.wgsl` is told; `frame.wgsl` is its twin.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FrameParams {
    pub camera_to_world: [[f32; 4]; 3],
    pub half_height: f32,
    pub half_width: f32,
    pub width: u32,
    pub height: u32,
    pub sample_base: u32,
    pub samples: u32,
    pub scene_seed: u32,
    pub max_bounces: u32,
    pub rr_start: u32,
    pub clamp_direct: f32,
    pub clamp_indirect: f32,
    pub shadow_far: f32,
    pub env_rotation: [[f32; 4]; 3],
    pub env_inverse_rotation: [[f32; 4]; 3],
    pub env_width: u32,
    pub env_height: u32,
    pub env_tiles_x: u32,
    pub env_tiles_y: u32,
    pub env_intensity: f32,
    pub env_weighted_sum: f32,
    pub env_infinite: u32,
    /// Bit 0: add the samples up on the card; bit 1: firefly rejection.
    pub mode: u32,
    pub off_luminance: u32,
    pub off_cdf: u32,
    pub off_power: u32,
    pub off_rows: u32,
    pub off_choice: u32,
    pub off_tile_column: u32,
    pub off_tile_row: u32,
    pub off_row_start: u32,
    pub regularization_start: u32,
    pub regularization_roughness: f32,
    pub off_marginal: u32,
    pub off_upward: u32,
    pub env_upward_sum: f32,
    /// `IntegratorConfig::shadows`, 1 for on.
    pub shadows: u32,
    /// `IntegratorConfig::ground_illumination`, 1 for on.
    pub ground_illumination: u32,
    /// How many lamps the scene has.
    pub light_count: u32,
    /// 1 where the light hierarchy chooses among them, 0 for the alias table.
    pub light_hierarchy: u32,
    /// [`crate::lights::lamp_flags`].
    pub lamp_flags: u32,
    /// `IntegratorConfig::pixel_spread`: the radians a pixel spans, which a
    /// hit's footprint is its distance times (a material's program, its bump).
    pub pixel_spread: f32,
    /// `IntegratorConfig::max_transmission_bounces`.
    pub max_transmission_bounces: u32,
    /// `IntegratorConfig::shadow_glass`: [`GLASS_SOLID`], [`GLASS_CLEAR`] or
    /// [`GLASS_UNBENT`].
    pub shadow_glass: u32,
    /// [`SCENE_TRANSMITS`], [`SCENE_PANES`], [`SCENE_CUTOUTS`],
    /// [`SCENE_EMITTERS`], [`SCENE_LIGHT_MATERIALS`],
    /// [`SCENE_CUT_PROGRAMS`], [`SCENE_BEVEL`], [`SCENE_BACKGROUND_COLOUR`]
    /// and [`SCENE_BACKPLATE`].
    pub glass_flags: u32,
    /// The rows a dispatch draws: `row_base` and every `row_stride`-th row
    /// after it (`render_pass`); 0 and 1 for the whole frame.
    pub row_base: u32,
    pub row_stride: u32,
    /// `Background::Color`'s colour, under [`SCENE_BACKGROUND_COLOUR`].
    pub background: [f32; 3],
    /// A backplate's picture under [`SCENE_BACKPLATE`]: where it starts in
    /// the camera's image buffer, and its size (`crate::environment`).
    pub plate_offset: u32,
    pub plate_width: u32,
    pub plate_height: u32,
    /// [`CAMERA_THIN_LENS`], [`CAMERA_BLADED`], [`CAMERA_SHIFT`],
    /// [`CAMERA_ORTHOGRAPHIC`], [`CAMERA_EQUIRECTANGULAR`],
    /// [`CAMERA_FILTER_BOX`] and [`CAMERA_FILTER_TABLE`].
    pub camera_flags: u32,
    /// The thin lens (`Camera::generate_ray`): the opening's radius in
    /// scene units (`Camera::aperture_radius`), the focus distance, and a
    /// bladed opening's blades (3 or more) and turn.
    pub lens_radius: f32,
    pub focus_distance: f32,
    pub aperture_blades: u32,
    pub aperture_rotation: f32,
    /// A shifted lens's film offset, in fractions of the sensor
    /// (`Projection::Shift`), under [`CAMERA_SHIFT`].
    pub shift_x: f32,
    pub shift_y: f32,
    /// The pixel filter (`FilterSampler::place`): a box's width in pixels
    /// under [`CAMERA_FILTER_BOX`]; under [`CAMERA_FILTER_TABLE`] the
    /// inverse of its distribution over one half, `filter_table`'s 1024
    /// values four to a vector and `filter_last` the 1025th.
    pub filter_width: f32,
    pub filter_last: f32,
    pub _pad5: u32,
    /// A dome of a size under [`SCENE_FINITE_DOME`] (`Dome`): its radius in
    /// scene units, its centre's height above the floor, and 1 where its
    /// lower cap is the ground plane.
    pub dome_radius: f32,
    pub dome_height: f32,
    pub dome_flatten: u32,
    pub _pad6: u32,
    pub filter_table: [[f32; 4]; 256],
}

impl Default for FrameParams {
    fn default() -> Self {
        Zeroable::zeroed()
    }
}

/// [`FrameParams::shadow_glass`]: no glass lets a shadow ray by.
pub const GLASS_SOLID: u32 = 0;
/// All clear glass does.
pub const GLASS_CLEAR: u32 = 1;
/// Only clear glass that does not bend light does.
pub const GLASS_UNBENT: u32 = 2;
/// [`FrameParams::glass_flags`]: some material transmits
/// (`SceneSnapshot::has_transmissive`).
pub const SCENE_TRANSMITS: u32 = 1;
/// Some material is clear glass a shadow ray passes (`SceneSnapshot::has_panes`).
pub const SCENE_PANES: u32 = 2;
/// Some material is cut out (`SceneSnapshot::has_cutouts`): a path crosses
/// its holes, and shadow rays walk through them.
pub const SCENE_CUTOUTS: u32 = 4;
/// The glowing triangles are lights a light sample chooses among
/// (`SceneSnapshot::emitters` is not empty and
/// `IntegratorConfig::light_source_materials` is on).
pub const SCENE_EMITTERS: u32 = 8;
/// `IntegratorConfig::light_source_materials`: a glow a diffuse bounce meets
/// lights what the bounce left.
pub const SCENE_LIGHT_MATERIALS: u32 = 16;
/// A program on the card drives some material's opacity
/// (`crate::program::PROGRAM_CUTS`): a walk crossing it asks the program.
pub const SCENE_CUT_PROGRAMS: u32 = 32;
/// Some material rounds its edges (`edge_radius`, Cycles' Bevel): the
/// surface a path meets first has its normal rounded by probes
/// (`transport.wgsl`'s `bevel_normal`, `bevel.rs`).
pub const SCENE_BEVEL: u32 = 64;
/// The camera, and the mirrors and glass it sees through, see a colour past
/// the parts rather than the environment (`Background::Color`,
/// `integrator.rs`'s `past_the_scene`).
pub const SCENE_BACKGROUND_COLOUR: u32 = 128;
/// They see a backplate's picture, at the film point the path set out from
/// (`Background::Backplate`).
pub const SCENE_BACKPLATE: u32 = 256;
/// The environment is a dome of a size (`Dome::radius`): the sky is looked
/// up, sampled and weighed from where a path is, and a shadow ray towards
/// it stops at it.
pub const SCENE_FINITE_DOME: u32 = 512;

/// [`FrameParams::camera_flags`]: the camera is a thin lens with an opening
/// (`Camera::aperture_radius` over nought): each ray starts on the opening,
/// drawn on the lens dimensions, and passes through its pinhole ray's point
/// on the focus plane.
pub const CAMERA_THIN_LENS: u32 = 1;
/// The opening has blades (`Aperture::Bladed`) rather than being round.
pub const CAMERA_BLADED: u32 = 2;
/// The film is offset (`Projection::Shift`): verticals kept parallel.
pub const CAMERA_SHIFT: u32 = 4;
/// Parallel rays from the film plane (`Projection::Orthographic`): the
/// frame's half-height and half-width are in scene units.
pub const CAMERA_ORTHOGRAPHIC: u32 = 8;
/// Every direction, the film's width round the horizon and its height
/// from the zenith down (`Projection::Equirectangular`).
pub const CAMERA_EQUIRECTANGULAR: u32 = 16;
/// The pixel filter is a box other than one pixel wide: a sample's jitter
/// is spread about the pixel's centre by its width.
pub const CAMERA_FILTER_BOX: u32 = 32;
/// The pixel filter is a Gaussian or a Blackman-Harris: a sample is placed
/// by the inverse of its distribution, read from a table.
pub const CAMERA_FILTER_TABLE: u32 = 64;

/// A pixel's sums on the card (`frame.wgsl`'s `PixelSums`): what of
/// [`Framebuffer`]'s `Pixel` this transport fills.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct PixelSums {
    /// Radiance, and in `[3]` the energy firefly rejection took off.
    pub sum: [f32; 4],
    /// Background, and in `[3]` the coverage.
    pub background: [f32; 4],
    /// Squared luminance: the sum and Kahan's compensation, which the sum is
    /// over by.
    pub squares_high: f32,
    pub squares_low: f32,
    pub samples: u32,
    pub _pad: u32,
}

const MODE_ON_CARD: u32 = 1;
const MODE_REJECT: u32 = 2;

/// What a render asks, as the processor's `RenderContext` has it.
#[derive(Clone, Debug)]
pub struct FrameSettings {
    pub camera: Camera,
    pub config: IntegratorConfig,
    pub scene_seed: u32,
    pub reject_fireflies: bool,
    pub clamp_direct: Option<f32>,
    pub clamp_indirect: Option<f32>,
}

/// Firefly rejection's ceiling on indirect light, the mean of a
/// contribution's red, green and blue (`tequila-render-cpu`'s
/// `FIREFLY_CLAMP`).
pub const FIREFLY_CLAMP: f32 = 10.0;

impl FrameSettings {
    /// The processor's `renderer::clamps`: (direct, indirect).
    pub fn clamps(&self) -> (Option<f32>, Option<f32>) {
        let rejection = self.reject_fireflies.then_some(FIREFLY_CLAMP);
        match (self.clamp_direct, self.clamp_indirect) {
            (Some(direct), clamp) => (Some(direct), clamp.or(rejection)),
            (None, Some(clamp)) => (Some(clamp), Some(clamp)),
            (None, None) => (None, rejection),
        }
    }
}

/// What of a shadow catcher in `scene` the card renders otherwise than the
/// processor does under `config`, one sentence each; empty where the floor
/// is the processor's (`transport.wgsl`'s catch_shadow). The renderer says
/// each at its first pass. A floor's reflection alone
/// (`IntegratorConfig::floor_reflection_only`) the card does not render at
/// all: [`GpuRenderer::render_pass`] refuses it.
pub fn floor_limits(scene: &SceneSnapshot, config: &IntegratorConfig) -> Vec<&'static str> {
    FloorScene::of(scene).limits(config)
}

/// What of glass, media and the other materials in `scene` the card renders
/// otherwise than the processor does under `config`, one sentence each;
/// empty where they are the processor's (`transport.wgsl`'s path through
/// glass). The renderer says each at its first pass, with [`floor_limits`]
/// and [`crate::lights::lamp_limits`].
pub fn glass_limits(scene: &SceneSnapshot, config: &IntegratorConfig) -> Vec<&'static str> {
    GlassScene::of(scene).limits(config)
}

/// What [`glass_limits`] and the glass flags ask of a scene, read when it is
/// uploaded.
#[derive(Clone, Copy, Debug, Default)]
struct GlassScene {
    transmits: bool,
    panes: bool,
    dispersion: bool,
    subsurface: bool,
    film: bool,
    flakes: bool,
    scattering: bool,
    volume_maps: bool,
    unseen_parts: bool,
    folded_glass: bool,
    cutouts: bool,
}

impl GlassScene {
    fn of(scene: &SceneSnapshot) -> GlassScene {
        // Each material as the card shades it: a program's constants, or
        // the scene's own material.
        let shaded: Vec<&tequila_materials::openpbr::Material> = (0..scene.materials.len())
            .map(|i| match scene.program(tequila_core::ids::MaterialId(i as u32)) {
                Some(program) => &program.constants,
                None => &scene.materials[i],
            })
            .collect();
        let any = |test: &dyn Fn(&tequila_materials::openpbr::Material) -> bool| shaded.iter().any(|m| test(m));
        // The integrator reads a medium's boundary, priority and volume off
        // the scene's own material, the card off one record for both.
        let folded = shaded.iter().zip(&scene.materials).any(|(m, own)| {
            m.transmission_weight != own.transmission_weight
                || m.transmission_color != own.transmission_color
                || m.transmission_depth != own.transmission_depth
                || m.volume_density != own.volume_density
                || m.medium_priority != own.medium_priority
                || m.geometry_thin_walled != own.geometry_thin_walled
                || m.volume_emission_color != own.volume_emission_color
                || m.volume_emission_strength != own.volume_emission_strength
        });
        GlassScene {
            transmits: scene.has_transmissive(),
            panes: scene.has_panes(),
            dispersion: scene.has_dispersion(),
            subsurface: any(&|m| m.subsurface_weight != 0.0),
            film: any(&|m| m.thin_film_weight > 1e-4),
            // A material's coverage is always nought: it is the point's,
            // which the processor fills in wherever the weight is over 1e-4
            // (flakes.rs's apply).
            flakes: any(&|m| m.flake_weight > 1e-4),
            scattering: any(&|m| m.transmission_weight > 0.0 && m.volume_scattering().is_some()),
            volume_maps: (0..scene.instances.len()).any(|i| scene.density(tequila_core::ids::InstanceId(i as u32)).is_some()),
            unseen_parts: scene
                .instances
                .iter()
                .any(|i| !i.visible_to_camera || !i.seen_by_diffuse || !i.seen_by_glossy || !i.seen_by_transmission || i.holdout),
            folded_glass: folded,
            cutouts: scene.has_cutouts(),
        }
    }

    /// [`FrameParams::glass_flags`].
    fn flags(&self) -> u32 {
        (if self.transmits { SCENE_TRANSMITS } else { 0 }) | if self.panes { SCENE_PANES } else { 0 } | if self.cutouts { SCENE_CUTOUTS } else { 0 }
    }

    fn limits(&self, config: &IntegratorConfig) -> Vec<&'static str> {
        use tequila_render_core::settings::{CausticsMethod, SpectralMode};
        let mut limits = Vec::new();
        let spectral = match config.spectral {
            SpectralMode::Auto => self.dispersion,
            SpectralMode::On => true,
            SpectralMode::Off => false,
        };
        if spectral {
            limits.push("the picture is traced in red, green and blue rather than by wavelength");
        }
        if self.dispersion {
            limits.push("glass that disperses light bends every colour alike");
        }
        if config.caustics {
            limits.push(match config.caustics_method {
                CausticsMethod::Photons => "no caustic photons: the light glass and mirrors focus is left out",
                CausticsMethod::Manifold => "no manifold caustics: a lamp's light through glass that bends it is left out",
            });
        }
        if self.subsurface {
            limits.push("subsurface scattering is left out: no random walk under a surface");
        }
        if self.scattering {
            limits.push("a volume that scatters light only absorbs it");
        }
        if self.volume_maps {
            limits.push("a Volume Map is left out: its part is empty inside");
        }
        if self.film {
            limits.push("a thin film is left out");
        }
        if self.flakes {
            limits.push("metallic flakes are left out");
        }
        if self.unseen_parts {
            limits.push("a part hidden from some rays, or held out, is seen by every ray");
        }
        if self.folded_glass {
            limits.push("glass whose graph sets its transmission is a medium as its graph's constants have it");
        }
        limits
    }
}

/// What of the parts and what lies past them in `scene` the card renders
/// otherwise than the processor does, one sentence each; empty where they
/// are the processor's. The renderer says each at its first pass, with the
/// other `*_limits`; nothing here depends on `config`.
pub fn part_limits(scene: &SceneSnapshot, _config: &IntegratorConfig) -> Vec<&'static str> {
    let mut limits = Vec::new();
    // The cutaway volumes (`cutaway.rs`): every ray the processor traces
    // steps past what they cut; the card's acceleration structure has the
    // parts whole.
    if !scene.cutters().is_empty() {
        limits.push("a cutaway is left out: the parts it cuts are whole");
    }
    // The fibres a material grows (`fuzz_density`, the Fuzz node) are curves
    // the card's acceleration structure does not hold.
    if scene.instances.iter().any(|i| scene.meshes.get(i.mesh.index()).is_some_and(|m| m.fibres.is_some())) {
        limits.push("fibres are left out: the parts that grow them are bare");
    }
    limits
}

/// What of `camera`, and of the film under `config`, the card renders
/// otherwise than the processor does, one sentence each: nothing now - its
/// projections, lens and pixel filter are the processor's
/// (`transport.wgsl`'s `camera_ray` and `filter_place`). Kept for the
/// renderer and the command line, which say each of the `*_limits` at the
/// first pass.
pub fn camera_limits(_camera: &Camera, _config: &IntegratorConfig) -> Vec<&'static str> {
    Vec::new()
}

/// The most samples a dispatch draws: a bounded buffer.
const SAMPLES_PER_DISPATCH: u32 = 16;

/// How long a dispatch should keep the card from the display, in seconds.
/// Windows resets a card whose one piece of work runs past two seconds
/// (its TDR), and on any system the display waits on it meanwhile.
const DISPATCH_SECONDS: f64 = 0.1;

/// A first dispatch's work in pixel samples, before the card's pace on the
/// scene is known: small enough for the heaviest scene to finish in well
/// under two seconds.
const FIRST_DISPATCH_WORK: u64 = 1 << 18;

/// A scene on one card, ready to render.
pub struct GpuRenderer {
    pub gpu: GpuDevice,
    /// Add the samples up on the card, firefly rejection and all, and bring
    /// back each pixel's sums once a pass: the fast way, and the default.
    /// Off, each sample comes back and the processor's `Framebuffer` adds it
    /// up, exactly as it adds up its own (the parity test).
    pub accumulate_on_card: bool,
    tables: Tables,
    /// The scene's buffers and acceleration structures, which the bind
    /// groups read; kept for as long as the renderer is.
    pub scene: GpuScene,
    environment: GpuEnvironment,
    shadow_far: f32,
    /// The scene's materials, which the scene's bind group reads.
    pub materials: wgpu::Buffer,
    /// The materials' programs: their words, their constants and where
    /// each material's are (`crate::program`).
    pub programs: [wgpu::Buffer; 3],
    /// The pictures they sample: the texels and the header
    /// (`crate::textures`).
    pub pictures: [wgpu::Buffer; 2],
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::PipelineLayout,
    scene_group: wgpu::BindGroup,
    environment_group: wgpu::BindGroup,
    frame_layout: wgpu::BindGroupLayout,
    /// What [`floor_limits`] reads of the scene, kept to say it once a
    /// configuration is known (the first pass), and whether it has been.
    floor_scene: FloorScene,
    said_floor_limits: std::sync::atomic::AtomicBool,
    /// How much work, in pixel samples, a dispatch of this scene can do in
    /// [`DISPATCH_SECONDS`], as the dispatches so far have timed it; and the
    /// seconds the last timed one took, as `f64` bits.
    dispatch_work: std::sync::atomic::AtomicU64,
    dispatch_seconds: std::sync::atomic::AtomicU64,
    /// The scene's lamps, which the scene's bind group reads, and what
    /// [`crate::lights::lamp_limits`] and the lamp flags read of the scene.
    pub lights: GpuLights,
    lamp_scene: LampScene,
    /// What [`glass_limits`] and the glass flags read of the scene.
    glass_scene: GlassScene,
    /// What [`crate::textures::texture_limits`] says of the scene.
    texture_limits: Vec<&'static str>,
    /// The glowing triangles' tree and leaves (`crate::emitters`), which the
    /// scene's bind group reads, and whether there are any.
    pub emitters: [wgpu::Buffer; 2],
    has_emitters: bool,
    /// Whether a program on the card drives some opacity.
    cut_programs: bool,
    /// Whether some material rounds its edges, as the card shades it.
    bevels: bool,
    /// What [`part_limits`] says of the scene.
    part_limits: Vec<&'static str>,
    /// What the camera sees past the parts: [`SCENE_BACKGROUND_COLOUR`] or
    /// [`SCENE_BACKPLATE`] (none for the environment), with
    /// [`SCENE_FINITE_DOME`] for a dome of a size; and the colour.
    background_flags: u32,
    background: [f32; 3],
    /// The environment's dome.
    dome: tequila_render_core::environment::Dome,
    /// The kernel widened to the frame's parts a frame asked for that it
    /// left out ([`KernelFeatures::of_frame`]), each built once.
    widened: std::sync::Mutex<Vec<(KernelFeatures, wgpu::ComputePipeline)>>,
    /// The parts of the kernel the scene reaches ([`KernelFeatures::of`]),
    /// and the parts of the one `pipeline` was built with, which holds
    /// them all.
    pub features: KernelFeatures,
    pub kernel: KernelFeatures,
    /// How long building `pipeline` took, in seconds: the first picture's
    /// wait for the kernel.
    pub kernel_seconds: f64,
    /// A kernel being built aside ([`GpuRenderer::with_cache`]).
    aside: Option<Aside>,
}

/// A kernel being built on a thread of its own while the renderer goes on
/// with the one it has: a scene's own variant, which it renders with once
/// ready (the same picture, so from any pass), or the whole kernel, built
/// only for the scenes after.
struct Aside {
    kernel: KernelFeatures,
    take_up: bool,
    ready: std::sync::Arc<std::sync::OnceLock<wgpu::ComputePipeline>>,
    thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Aside {
    fn spawn(device: &wgpu::Device, layout: &wgpu::PipelineLayout, cache: &KernelCache, kernel: KernelFeatures, take_up: bool) -> Aside {
        let ready = std::sync::Arc::new(std::sync::OnceLock::new());
        let (device, layout, cache, slot) = (device.clone(), layout.clone(), cache.clone(), ready.clone());
        let thread = std::thread::Builder::new()
            .name("transport kernel".into())
            .spawn(move || {
                let pipeline = kept_pipeline(&device, &layout, &cache, kernel);
                let _ = slot.set(pipeline);
            })
            .ok();
        Aside { kernel, take_up, ready, thread: std::sync::Mutex::new(thread) }
    }
}

/// How a renderer's kernel is chosen.
enum KernelChoice<'a> {
    /// Built for the parts it keeps, the scene's own where none.
    Parts(Option<KernelFeatures>),
    /// Through what this machine has kept ([`GpuRenderer::with_cache`]).
    Kept(&'a KernelCache),
}

/// Build `kernel` through what `cache` keeps of it, and keep what the
/// build leaves.
fn kept_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, cache: &KernelCache, kernel: KernelFeatures) -> wgpu::ComputePipeline {
    let pipeline_cache = cache.pipeline_cache(device, kernel);
    let pipeline = build_pipeline(device, layout, None, crate::shader::source("transport"), kernel, pipeline_cache.as_ref());
    cache.keep(kernel, pipeline_cache.as_ref());
    pipeline
}

/// The kernel a scene reaching `features` starts with, through `cache`
/// (see [`KernelCache`]): its own variant where this machine has built it,
/// or where it has built nothing yet (then building the whole kernel aside,
/// for the scenes after); else the whole kernel, which it has built, while
/// its variant is built aside to take over.
fn kept_kernel(device: &wgpu::Device, layout: &wgpu::PipelineLayout, features: KernelFeatures, cache: &KernelCache) -> (KernelFeatures, wgpu::ComputePipeline, Option<Aside>) {
    if cache.knows(features) || !cache.knows(KernelFeatures::ALL) {
        let pipeline = kept_pipeline(device, layout, cache, features);
        let aside = (!cache.knows(KernelFeatures::ALL)).then(|| Aside::spawn(device, layout, cache, KernelFeatures::ALL, false));
        (features, pipeline, aside)
    } else {
        let pipeline = kept_pipeline(device, layout, cache, KernelFeatures::ALL);
        (KernelFeatures::ALL, pipeline, Some(Aside::spawn(device, layout, cache, features, true)))
    }
}

/// What [`floor_limits`] asks of a scene, read when it is uploaded.
#[derive(Clone, Copy, Debug, Default)]
struct FloorScene {
    catcher: bool,
    catcher_shadows: bool,
    lamps: bool,
    shadowless_parts: bool,
}

impl FloorScene {
    fn of(scene: &SceneSnapshot) -> FloorScene {
        FloorScene {
            catcher: scene.instances.iter().any(|i| i.shadow_catcher.is_some()),
            catcher_shadows: scene.instances.iter().any(|i| i.shadow_catcher.is_some_and(|c| c.shadows)),
            lamps: !scene.lights.lights().is_empty(),
            shadowless_parts: scene.has_shadowless_parts(),
        }
    }

    fn limits(&self, config: &IntegratorConfig) -> Vec<&'static str> {
        let mut limits = Vec::new();
        if !self.catcher {
            return limits;
        }
        let shadows = config.shadows && self.catcher_shadows;
        if shadows && self.lamps {
            limits.push("the floor's shadows are the sky's alone: the lamps cast none on it");
        }
        if self.shadowless_parts {
            limits.push("a part that casts no shadow shadows the floor all the same");
        }
        if config.caustics && shadows {
            limits.push("no caustics on the floor");
        }
        limits
    }
}

/// The transport kernel's pipeline, with the parts `kernel` keeps
/// (`crate::variants`). `salt`, when given, makes the kernel a different one
/// to the driver without changing what it does, so its shader cache cannot
/// answer for it: how long a cold compile takes is measured so.
fn transport_pipeline(device: &wgpu::Device, layout: &wgpu::PipelineLayout, salt: Option<u32>, kernel: KernelFeatures) -> wgpu::ComputePipeline {
    transport_pipeline_of(device, layout, salt, crate::shader::source("transport"), kernel)
}

fn transport_pipeline_of(device: &wgpu::Device, layout: &wgpu::PipelineLayout, salt: Option<u32>, source: String, kernel: KernelFeatures) -> wgpu::ComputePipeline {
    build_pipeline(device, layout, salt, source, kernel, None)
}

fn build_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    salt: Option<u32>,
    mut source: String,
    kernel: KernelFeatures,
    cache: Option<&wgpu::PipelineCache>,
) -> wgpu::ComputePipeline {
    if let Some(salt) = salt {
        // The entry point's first test, which the salt is added to. It must
        // be there: when dispatches were cut by rows it read `id.y` no more,
        // the salt went nowhere, and the driver's cache answered.
        const FIRST: &str = "if (id.x >= frame.width || y >= frame.height) {";
        assert!(source.contains(FIRST), "the transport's entry point no longer starts with {FIRST:?}: the salt has nowhere to go");
        source = source.replace(FIRST, &format!("if (id.x >= frame.width || y >= frame.height || frame.width == {}u) {{", salt | 0x8000_0000));
    }
    // The constants the source declares: one edited to measure what a part
    // costs may have taken some out.
    let constants: Vec<(&str, f64)> = kernel.constants().into_iter().filter(|(name, _)| source.contains(&format!("override {name}:"))).collect();
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("transport"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("transport"),
        layout: Some(layout),
        module: &module,
        entry_point: Some("transport_main"),
        compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, ..Default::default() },
        cache,
    })
}

/// How long the transport kernel takes to compile on the renderer's card
/// with nothing cached: its source salted (see `transport_pipeline`), built
/// against the renderer's own layout. For keeping the kernel's size in check.
pub fn cold_compile_time(renderer: &GpuRenderer, salt: u32) -> std::time::Duration {
    cold_compile_time_of(renderer, salt, KernelFeatures::ALL)
}

/// [`cold_compile_time`] of the kernel with the parts `kernel` keeps.
pub fn cold_compile_time_of(renderer: &GpuRenderer, salt: u32, kernel: KernelFeatures) -> std::time::Duration {
    let started = std::time::Instant::now();
    let _pipeline = transport_pipeline(&renderer.gpu.device, &renderer.layout, Some(salt), kernel);
    renderer.gpu.wait();
    started.elapsed()
}

/// How long the kernel with the parts `kernel` keeps takes to build, its
/// source unsalted: from the driver's cache where it has compiled it
/// before.
pub fn compile_time_of(renderer: &GpuRenderer, kernel: KernelFeatures) -> std::time::Duration {
    let started = std::time::Instant::now();
    let _pipeline = transport_pipeline(&renderer.gpu.device, &renderer.layout, None, kernel);
    renderer.gpu.wait();
    started.elapsed()
}

/// [`cold_compile_time`] of the kernel's source as `edit` leaves it: what a
/// part of the kernel costs to compile, measured by leaving it out.
pub fn cold_compile_time_edited(renderer: &GpuRenderer, salt: u32, edit: &dyn Fn(String) -> String) -> std::time::Duration {
    cold_compile_time_edited_of(renderer, salt, edit, KernelFeatures::ALL)
}

/// [`cold_compile_time_edited`] of the kernel with the parts `kernel` keeps.
pub fn cold_compile_time_edited_of(renderer: &GpuRenderer, salt: u32, edit: &dyn Fn(String) -> String, kernel: KernelFeatures) -> std::time::Duration {
    let source = edit(crate::shader::source("transport"));
    let started = std::time::Instant::now();
    let _pipeline = transport_pipeline_of(&renderer.gpu.device, &renderer.layout, Some(salt), source, kernel);
    renderer.gpu.wait();
    started.elapsed()
}

fn entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }
}

fn storage(read_only: bool) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only },
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

impl GpuRenderer {
    /// Upload `scene` to `gpu`, which must have ray queries, with the
    /// kernel built for the parts the scene reaches (`crate::variants`);
    /// `TEQUILA_GPU_KERNEL=full` builds the whole kernel instead.
    pub fn new(gpu: GpuDevice, scene: &SceneSnapshot) -> GpuRenderer {
        let full = std::env::var("TEQUILA_GPU_KERNEL").is_ok_and(|v| v.eq_ignore_ascii_case("full"));
        GpuRenderer::with_kernel(gpu, scene, full.then_some(KernelFeatures::ALL))
    }

    /// [`GpuRenderer::new`] with the kernel built for the parts `kernel`
    /// keeps, the scene's own where `None`. A kernel that leaves out a part
    /// the scene reaches would not give its picture, and is widened to it.
    pub fn with_kernel(gpu: GpuDevice, scene: &SceneSnapshot, kernel: Option<KernelFeatures>) -> GpuRenderer {
        GpuRenderer::upload(gpu, scene, KernelChoice::Parts(kernel))
    }

    /// [`GpuRenderer::new`] with its kernel built through what this machine
    /// keeps in `cache` (see [`KernelCache`]): the first picture waits on a
    /// cold compile only where the machine has never built the kernel, and
    /// then on the scene's own variant, the smallest there is.
    pub fn with_cache(gpu: GpuDevice, scene: &SceneSnapshot, cache: &KernelCache) -> GpuRenderer {
        GpuRenderer::upload(gpu, scene, KernelChoice::Kept(cache))
    }

    fn upload(gpu: GpuDevice, scene: &SceneSnapshot, choice: KernelChoice) -> GpuRenderer {
        let device = &gpu.device;
        let tables = Tables::new(device);
        let uploaded = GpuScene::upload(device, &gpu.queue, scene);
        let environment = GpuEnvironment::upload(device, &scene.environment, &scene.background);
        let shadow_far = scene.bounds().bounding_radius().max(1.0) * 1e4;
        // The materials as data: a material with a program has the constants
        // its graph folded to (`CompiledGraph::constants`), which the
        // program's driven parameters are written over at each point. One
        // the card's stack cannot render yet (glass, skin, film, flakes)
        // renders as its opaque part, and says so; one whose program the
        // card cannot run renders as its constants, and says why.
        let mut pictures = CardPictures::of(scene);
        let on_card = pictures_on(&pictures, &device.limits());
        let too_large = matches!(on_card, Pictures::TooLarge { .. });
        let lowered = lower_programs_with(scene, on_card);
        if too_large {
            // Nothing of them is uploaded: every material reading one is
            // refused, and says so.
            pictures = CardPictures::default();
            pictures.finish();
        }
        for (index, refusal) in lowered.iter().enumerate().filter_map(|(i, l)| l.as_ref().err().map(|r| (i, r))) {
            eprintln!("  material {index} renders as its constants on the graphics card for now: {refusal} (docs/07 R3)");
        }
        let programs = crate::program::ProgramTable::new(&lowered.iter().map(|l| l.clone().ok().flatten()).collect::<Vec<_>>());
        let bases: Vec<&tequila_materials::openpbr::Material> = (0..scene.materials.len())
            .map(|i| match scene.program(tequila_core::ids::MaterialId(i as u32)) {
                Some(program) => &program.constants,
                None => &scene.materials[i],
            })
            .collect();
        let records: Vec<crate::material::GpuMaterial> = bases
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let id = tequila_core::ids::MaterialId(i as u32);
                crate::material::GpuMaterial::with_glass(m, scene.material(id), scene.pane(id))
            })
            .collect();
        let unsupported = bases.iter().filter(|m| !crate::material::GpuMaterial::supports(m)).count();
        if unsupported > 0 {
            eprintln!(
                "  {unsupported} material(s) render on the graphics card for now without their subsurface, thin film, flakes, scattering,                  or dispersion (docs/07 R3)"
            );
        }
        let records = if records.is_empty() { vec![crate::material::GpuMaterial::default()] } else { records };
        let lights = GpuLights::upload(device, &LightRecords::of(scene));
        let materials = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("materials"),
                contents: bytemuck::cast_slice(&records),
                usage: wgpu::BufferUsages::STORAGE,
            },
        );
        let init = |label: &str, contents: &[u8]| {
            wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage: wgpu::BufferUsages::STORAGE })
        };
        let program_words = init("program words", bytemuck::cast_slice(&programs.words));
        let program_constants = init("program constants", bytemuck::cast_slice(&programs.constants));
        let program_headers = init("programs", bytemuck::cast_slice(&programs.headers));
        // The glowing triangles, their parts numbered as the card's
        // instances are.
        let mut slots = vec![crate::emitters::EMITTER_NONE; scene.instances.len()];
        for (slot, record) in uploaded.instance_records.iter().enumerate() {
            if let Some(at) = slots.get_mut(record.instance as usize) {
                *at = slot as u32;
            }
        }
        let emitter_records = crate::emitters::EmitterRecords::of(scene, &|n| slots.get(n as usize).copied().unwrap_or(crate::emitters::EMITTER_NONE));
        let has_emitters = !emitter_records.is_empty();
        let emitter_nodes = if has_emitters { emitter_records.nodes.clone() } else { vec![crate::emitters::GpuEmitterNode::default()] };
        let emitter_leaves = if emitter_records.leaves.is_empty() { vec![crate::emitters::EMITTER_NONE] } else { emitter_records.leaves.clone() };
        let emitter_nodes = init("emitter nodes", bytemuck::cast_slice(&emitter_nodes));
        let emitter_leaves = init("emitter leaves", bytemuck::cast_slice(&emitter_leaves));
        let texture_texels = init("texture texels", bytemuck::cast_slice(&pictures.texels));
        let texture_header = init("texture header", bytemuck::cast_slice(&pictures.header));

        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene"),
            entries: &[
                entry(0, wgpu::BindingType::AccelerationStructure { vertex_return: false }),
                entry(1, storage(true)),
                entry(2, storage(true)),
                entry(3, storage(true)),
                entry(4, storage(true)),
                entry(5, storage(true)),
                entry(6, storage(true)),
                entry(7, storage(true)),
                entry(8, storage(true)),
                entry(9, storage(true)),
                entry(10, storage(true)),
                entry(11, storage(true)),
                entry(12, storage(true)),
                entry(13, storage(true)),
                entry(14, storage(true)),
                entry(15, storage(true)),
                entry(16, storage(true)),
                entry(17, storage(true)),
                entry(18, storage(true)),
                entry(19, storage(true)),
                entry(20, storage(true)),
            ],
        });
        let environment_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("environment"),
            entries: &[entry(0, storage(true)), entry(1, storage(true)), entry(2, storage(true)), entry(3, storage(true))],
        });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame"),
            entries: &[
                entry(
                    0,
                    wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                ),
                entry(1, storage(false)),
                entry(2, storage(false)),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transport"),
            bind_group_layouts: &[Some(&scene_layout), Some(&tables.layout), Some(&environment_layout), Some(&frame_layout)],
            immediate_size: 0,
        });
        let glass_scene = GlassScene::of(scene);
        let cut_programs = programs.headers.iter().any(|h| h.flags & crate::program::PROGRAM_CUTS != 0);
        let background_flags = match &scene.background {
            Background::Environment => 0,
            Background::Color(_) => SCENE_BACKGROUND_COLOUR,
            Background::Backplate { .. } => SCENE_BACKPLATE,
        } | if scene.environment.dome().is_infinite() { 0 } else { SCENE_FINITE_DOME };
        let features = KernelFeatures::of(&SceneRecords {
            materials: &records,
            programs: &programs,
            instances: &uploaded.instance_records,
            meshes: &uploaded.mesh_records,
            lamps: lights.count,
            emitters: has_emitters,
            glass_flags: glass_scene.flags() | if cut_programs { SCENE_CUT_PROGRAMS } else { 0 } | background_flags,
        });
        let started = std::time::Instant::now();
        let (kernel, pipeline, aside) = match choice {
            KernelChoice::Parts(kernel) => {
                let kernel = kernel.map_or(features, |k| k | features);
                (kernel, transport_pipeline(device, &layout, None, kernel), None)
            }
            KernelChoice::Kept(cache) => kept_kernel(device, &layout, features, cache),
        };
        let kernel_seconds = started.elapsed().as_secs_f64();
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene"),
            layout: &scene_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::AccelerationStructure(&uploaded.tlas) },
                wgpu::BindGroupEntry { binding: 1, resource: uploaded.positions.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: uploaded.normals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: uploaded.indices.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: uploaded.meshes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: uploaded.instances.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 6, resource: materials.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: uploaded.catchers.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: lights.lights.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 9, resource: lights.nodes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 10, resource: lights.links.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 11, resource: program_words.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 12, resource: program_constants.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 13, resource: program_headers.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 14, resource: uploaded.uvs.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 15, resource: uploaded.tangents.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 16, resource: texture_texels.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 17, resource: texture_header.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 18, resource: emitter_nodes.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 19, resource: emitter_leaves.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 20, resource: uploaded.attributes.as_entire_binding() },
            ],
        });
        let environment_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("environment"),
            layout: &environment_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: environment.image.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: environment.camera_image.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: environment.floats.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: environment.ints.as_entire_binding() },
            ],
        });
        GpuRenderer {
            gpu,
            accumulate_on_card: true,
            tables,
            scene: uploaded,
            environment,
            shadow_far,
            materials,
            programs: [program_words, program_constants, program_headers],
            pictures: [texture_texels, texture_header],
            pipeline,
            layout,
            scene_group,
            environment_group,
            frame_layout,
            floor_scene: FloorScene::of(scene),
            said_floor_limits: std::sync::atomic::AtomicBool::new(false),
            dispatch_work: std::sync::atomic::AtomicU64::new(FIRST_DISPATCH_WORK),
            dispatch_seconds: std::sync::atomic::AtomicU64::new(f64::INFINITY.to_bits()),
            lights,
            lamp_scene: LampScene::of(scene),
            glass_scene,
            texture_limits: crate::textures::texture_limits(scene, &IntegratorConfig::default()),
            emitters: [emitter_nodes, emitter_leaves],
            has_emitters,
            cut_programs,
            bevels: records.iter().any(|r| r.edge_radius > 0.0),
            part_limits: part_limits(scene, &IntegratorConfig::default()),
            features,
            kernel,
            kernel_seconds,
            aside,
            background_flags,
            dome: scene.environment.dome(),
            widened: std::sync::Mutex::new(Vec::new()),
            background: match &scene.background {
                Background::Color(c) => [c.r, c.g, c.b],
                _ => [0.0; 3],
            },
        }
    }

    /// The parts of the kernel the next pass renders with: the variant built
    /// aside once it is ready, else the one built first.
    pub fn kernel_now(&self) -> KernelFeatures {
        match &self.aside {
            Some(aside) if aside.take_up && aside.ready.get().is_some() => aside.kernel,
            _ => self.kernel,
        }
    }

    /// Wait for a kernel being built aside, if one is.
    pub fn wait_for_aside(&self) {
        if let Some(aside) = &self.aside {
            let thread = aside.thread.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
            if let Some(thread) = thread {
                let _ = thread.join();
            }
        }
    }

    /// The parts of the kernel a frame of `settings` at `width` by `height`
    /// reaches through its camera and film ([`KernelFeatures::of_frame`]).
    pub fn frame_features(&self, settings: &FrameSettings, width: usize, height: usize) -> KernelFeatures {
        KernelFeatures::of_frame(self.params(settings, width, height, 0, 1).camera_flags)
    }

    /// The pipeline a pass that reaches the frame's parts `frame` renders
    /// with: the one it has, where that keeps them; else that kernel widened
    /// to them, built the first time a frame asks and kept.
    fn pipeline_for(&self, frame: KernelFeatures) -> wgpu::ComputePipeline {
        let kernel = self.kernel_now();
        if kernel.contains(frame) {
            return self.pipeline_now().clone();
        }
        let wide = kernel | frame;
        let mut widened = self.widened.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, pipeline)) = widened.iter().find(|(k, _)| *k == wide) {
            return pipeline.clone();
        }
        let pipeline = transport_pipeline(&self.gpu.device, &self.layout, None, wide);
        widened.push((wide, pipeline.clone()));
        pipeline
    }

    /// The pipeline the next pass renders with.
    fn pipeline_now(&self) -> &wgpu::ComputePipeline {
        match &self.aside {
            Some(aside) if aside.take_up => aside.ready.get().unwrap_or(&self.pipeline),
            _ => &self.pipeline,
        }
    }

    /// Build the kernel again with the parts `kernel` keeps, widened to the
    /// parts the scene reaches, and render with it from the next pass: the
    /// same picture whichever it is.
    pub fn set_kernel(&mut self, kernel: KernelFeatures) {
        let kernel = kernel | self.features;
        self.wait_for_aside();
        self.aside = None;
        if kernel != self.kernel {
            self.pipeline = transport_pipeline(&self.gpu.device, &self.layout, None, kernel);
            self.kernel = kernel;
        }
    }

    /// [`GpuRenderer::set_kernel`] with the kernel's source as `edit`
    /// leaves it: for finding what of a variant moves a picture.
    #[doc(hidden)]
    pub fn set_kernel_edited(&mut self, kernel: KernelFeatures, edit: &dyn Fn(String) -> String) {
        self.wait_for_aside();
        self.aside = None;
        self.kernel = kernel | self.features;
        self.pipeline = transport_pipeline_of(&self.gpu.device, &self.layout, None, edit(crate::shader::source("transport")), self.kernel);
    }

    /// [`floor_limits`] of the scene this renders, under `config`.
    pub fn floor_limits(&self, config: &IntegratorConfig) -> Vec<&'static str> {
        self.floor_scene.limits(config)
    }

    /// [`crate::lights::lamp_limits`] of the scene this renders, under `config`.
    pub fn lamp_limits(&self, config: &IntegratorConfig) -> Vec<&'static str> {
        self.lamp_scene.limits(config)
    }

    /// [`glass_limits`] of the scene this renders, under `config`.
    pub fn glass_limits(&self, config: &IntegratorConfig) -> Vec<&'static str> {
        self.glass_scene.limits(config)
    }

    /// [`crate::textures::texture_limits`] of the scene this renders.
    pub fn texture_limits(&self, _config: &IntegratorConfig) -> Vec<&'static str> {
        self.texture_limits.clone()
    }

    /// [`part_limits`] of the scene this renders.
    pub fn part_limits(&self, _config: &IntegratorConfig) -> Vec<&'static str> {
        self.part_limits.clone()
    }

    fn params(&self, settings: &FrameSettings, width: usize, height: usize, sample_base: u32, samples: u32) -> FrameParams {
        let aspect = width as f32 / height as f32;
        // The frame's half-height and half-width as Camera::generate_ray
        // works them out: tan(fov / 2) for a lens, half the scale in scene
        // units for a parallel projection, unread for a 360-degree one.
        let (half_height, projection_flags, shift) = match settings.camera.projection {
            Projection::Orthographic { scale } => (scale * 0.5, CAMERA_ORTHOGRAPHIC, (0.0, 0.0)),
            Projection::Equirectangular => (0.0, CAMERA_EQUIRECTANGULAR, (0.0, 0.0)),
            Projection::Shift { shift_x, shift_y, .. } if shift_x != 0.0 || shift_y != 0.0 => {
                ((settings.camera.vertical_fov(aspect) * 0.5).tan(), CAMERA_SHIFT, (shift_x, shift_y))
            }
            Projection::Perspective { .. } | Projection::Shift { .. } => ((settings.camera.vertical_fov(aspect) * 0.5).tan(), 0, (0.0, 0.0)),
        };
        let m = settings.camera.camera_to_world.m;
        let (direct, indirect) = settings.clamps();
        let e = &self.environment.layout;
        // The thin lens, as Camera::generate_ray has it: an opening of this
        // radius (none for a pinhole, or a projection with no lens), round
        // or of `blades` (at least 3) turned by `rotation`.
        let lens_radius = settings.camera.aperture_radius(aspect);
        // Where in or around its pixel a sample goes (FilterSampler::place):
        // one pixel's box where its jitter falls, another box spread by its
        // width, a Gaussian or Blackman-Harris by its table.
        let filter = settings.config.pixel_filter.sampler();
        let mut filter_table = [[0.0f32; 4]; 256];
        let mut filter_last = 0.0;
        let filter_flags = match filter.table() {
            Some(table) => {
                assert_eq!(table.len(), 1025, "the filter's table has 1025 steps, as the card reads it");
                for (i, v) in table[..1024].iter().enumerate() {
                    filter_table[i / 4][i % 4] = *v;
                }
                filter_last = table[1024];
                CAMERA_FILTER_TABLE
            }
            None if filter.width() == 1.0 => 0,
            None => CAMERA_FILTER_BOX,
        };
        let (blades, rotation) = match settings.camera.aperture {
            tequila_render_core::camera::Aperture::Bladed { blades, rotation } => (blades.max(3), rotation),
            tequila_render_core::camera::Aperture::Circular => (0, 0.0),
        };
        FrameParams {
            camera_to_world: [m[0], m[1], m[2]],
            half_height,
            half_width: half_height * aspect,
            width: width as u32,
            height: height as u32,
            sample_base,
            samples,
            scene_seed: settings.scene_seed,
            max_bounces: settings.config.max_bounces,
            rr_start: settings.config.russian_roulette_start_bounce,
            clamp_direct: direct.unwrap_or(-1.0),
            clamp_indirect: indirect.unwrap_or(-1.0),
            shadow_far: self.shadow_far,
            env_rotation: e.rotation,
            env_inverse_rotation: e.inverse_rotation,
            env_width: e.width,
            env_height: e.height,
            env_tiles_x: e.tiles_x,
            env_tiles_y: e.tiles_y,
            env_intensity: e.intensity,
            env_weighted_sum: e.weighted_sum,
            env_infinite: e.infinite as u32,
            mode: if self.accumulate_on_card { MODE_ON_CARD } else { 0 } | if settings.reject_fireflies { MODE_REJECT } else { 0 },
            off_luminance: e.off_luminance,
            off_cdf: e.off_cdf,
            off_power: e.off_power,
            off_rows: e.off_rows,
            off_choice: e.off_choice,
            off_tile_column: e.off_tile_column,
            off_tile_row: e.off_tile_row,
            off_row_start: e.off_row_start,
            regularization_start: settings.config.regularization_start_bounce,
            regularization_roughness: settings.config.regularization_roughness,
            off_marginal: e.off_marginal,
            off_upward: e.off_upward,
            env_upward_sum: e.upward_sum,
            shadows: settings.config.shadows as u32,
            ground_illumination: settings.config.ground_illumination as u32,
            light_count: self.lights.count,
            light_hierarchy: self.lights.hierarchy as u32,
            lamp_flags: self.lamp_scene.flags(&settings.config),
            // As tequila-render-cpu's render_pass sets it: the pixel's angle
            // for this camera at this resolution, the default for a camera
            // with none.
            pixel_spread: if settings.camera.vertical_fov(aspect) > 0.0 {
                settings.camera.vertical_fov(aspect) / height as f32
            } else {
                tequila_render_core::settings::DEFAULT_PIXEL_SPREAD
            },
            max_transmission_bounces: settings.config.max_transmission_bounces,
            shadow_glass: match settings.config.shadow_glass() {
                ShadowGlass::Solid => GLASS_SOLID,
                ShadowGlass::Clear => GLASS_CLEAR,
                ShadowGlass::Unbent => GLASS_UNBENT,
            },
            glass_flags: self.glass_scene.flags()
                | if settings.config.light_source_materials { SCENE_LIGHT_MATERIALS } else { 0 }
                | if settings.config.light_source_materials && self.has_emitters { SCENE_EMITTERS } else { 0 }
                | if self.cut_programs { SCENE_CUT_PROGRAMS } else { 0 }
                | if self.bevels { SCENE_BEVEL } else { 0 }
                | self.background_flags,
            row_base: 0,
            row_stride: 1,
            background: self.background,
            plate_offset: e.plate_offset,
            plate_width: e.plate_width,
            plate_height: e.plate_height,
            camera_flags: projection_flags | filter_flags | if lens_radius > 0.0 { CAMERA_THIN_LENS | if blades > 0 { CAMERA_BLADED } else { 0 } } else { 0 },
            lens_radius,
            focus_distance: settings.camera.focus_distance,
            aperture_blades: blades,
            aperture_rotation: rotation,
            shift_x: shift.0,
            shift_y: shift.1,
            filter_width: filter.width(),
            filter_last,
            _pad5: 0,
            filter_table,
            dome_radius: self.dome.radius.unwrap_or(0.0),
            dome_height: self.dome.height,
            dome_flatten: self.dome.flatten as u32,
            _pad6: 0,
        }
    }

    /// `samples` more samples into every pixel of `framebuffer`, as the
    /// processor's `render_pass` adds them: each pixel's next sample indices,
    /// accumulated in order. The framebuffer's pixels must all hold the same
    /// number of samples (a fresh one, or one only this has filled).
    pub fn render_pass(&self, framebuffer: &mut Framebuffer, settings: &FrameSettings, samples: u32) {
        assert!(
            !settings.config.floor_reflection_only,
            "the floor's reflection alone is the processor's (tequila-render-cpu's floor.rs); the card renders the whole picture"
        );
        if !self.said_floor_limits.swap(true, std::sync::atomic::Ordering::Relaxed) {
            for limit in self
                .floor_limits(&settings.config)
                .iter()
                .chain(&self.lamp_limits(&settings.config))
                .chain(&self.glass_limits(&settings.config))
                .chain(&self.texture_limits(&settings.config))
                .chain(&self.part_limits(&settings.config))
                .chain(&camera_limits(&settings.camera, &settings.config))
            {
                eprintln!("  on the graphics card for now, {limit} (docs/07 R3)");
            }
        }
        let (width, height) = (framebuffer.width, framebuffer.height);
        let pixels = width * height;
        let device = &self.gpu.device;
        let start = framebuffer.pixels().first().map_or(0, |p| p.samples);
        let batch = SAMPLES_PER_DISPATCH.min(samples.max(1));
        let on_card = self.accumulate_on_card;
        // Each sample back, when the processor adds them up; else a word.
        let sample_bytes = if on_card { 16 } else { (pixels * batch as usize * 2 * 16) as u64 };
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("samples"),
            size: sample_bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        // The pixels' sums so far, when the card adds them up; else a pixel.
        let sums: Vec<PixelSums> = if on_card {
            framebuffer
                .pixels()
                .iter()
                .map(|p| {
                    let high = p.sum_squared_luminance as f32;
                    PixelSums {
                        sum: [p.sum.r, p.sum.g, p.sum.b, p.clamped_energy],
                        background: [p.background_sum.r, p.background_sum.g, p.background_sum.b, p.coverage_sum],
                        squares_high: high,
                        squares_low: (high as f64 - p.sum_squared_luminance) as f32,
                        samples: p.samples,
                        _pad: 0,
                    }
                })
                .collect()
        } else {
            vec![PixelSums::default()]
        };
        let pixel_bytes = (sums.len() * std::mem::size_of::<PixelSums>()) as u64;
        let pixel_buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("pixels"),
                contents: bytemuck::cast_slice(&sums),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            },
        );
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read back"),
            size: sample_bytes.max(pixel_bytes),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame"),
            size: std::mem::size_of::<FrameParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame"),
            layout: &self.frame_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: pixel_buffer.as_entire_binding() },
            ],
        });
        // Taken up at the pass's start, so one pass is one kernel's; widened
        // to the parts this frame's camera reaches that it left out.
        let pipeline = &self.pipeline_for(self.frame_features(settings, width, height));
        // The most a dispatch may do: every pixel, `batch` samples each.
        let most = pixels as u64 * batch as u64;
        let mut done = 0u32;
        while done < samples {
            // As many samples and rows as the card does in DISPATCH_SECONDS:
            // up to `batch` samples of the whole frame, else one sample of
            // every `stride`-th row at a time. Each pixel still takes its
            // samples in order, so the picture is the same however it is cut.
            let work = self.dispatch_work.load(std::sync::atomic::Ordering::Relaxed).clamp(width as u64, most);
            let count = ((work / pixels as u64) as u32).clamp(1, batch.min(samples - done));
            let rows = if count > 1 { height } else { ((work / width as u64) as usize).clamp(1, height) };
            let stride = height.div_ceil(rows);
            for base in 0..stride {
                let drawn = (height - base).div_ceil(stride);
                let mut params = self.params(settings, width, height, start + done, count);
                params.row_base = base as u32;
                params.row_stride = stride as u32;
                self.gpu.queue.write_buffer(&uniform, 0, bytemuck::bytes_of(&params));
                let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("transport") });
                {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("transport"), timestamp_writes: None });
                    pass.set_pipeline(pipeline);
                    pass.set_bind_group(0, &self.scene_group, &[]);
                    pass.set_bind_group(1, &self.tables.bind_group, &[]);
                    pass.set_bind_group(2, &self.environment_group, &[]);
                    pass.set_bind_group(3, &frame_group, &[]);
                    pass.dispatch_workgroups((width as u32).div_ceil(8), (drawn as u32).div_ceil(8), 1);
                }
                let begun = std::time::Instant::now();
                self.gpu.queue.submit(Some(encoder.finish()));
                // Timed while the pace is being found, and wherever a
                // dispatch came near its time; at full size and well inside
                // it, dispatches queue up without a wait between them.
                let last = f64::from_bits(self.dispatch_seconds.load(std::sync::atomic::Ordering::Relaxed));
                if !on_card || work < most || last > DISPATCH_SECONDS / 4.0 {
                    self.gpu.wait();
                    self.check_device();
                    self.timed(drawn as u64 * width as u64 * count as u64, begun.elapsed().as_secs_f64(), most);
                }
            }
            if on_card {
                done += count;
                continue;
            }
            let used = (pixels * count as usize * 2 * 16) as u64;
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("read back") });
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, used);
            self.gpu.queue.submit(Some(encoder.finish()));
            readback.slice(..used).map_async(wgpu::MapMode::Read, |r| {
                if let Err(why) = r {
                    eprintln!("  the card's samples did not come back: {why}");
                }
            });
            self.gpu.wait();
            self.check_device();
            {
                let view = readback.slice(..used).get_mapped_range().expect("the samples are mapped");
                let values: &[[f32; 4]] = bytemuck::cast_slice(&view[..]);
                let reject = settings.reject_fireflies;
                for (index, pixel) in framebuffer.pixels_mut().iter_mut().enumerate() {
                    for i in 0..count as usize {
                        let at = (i * pixels + index) * 2;
                        let (r, b) = (values[at], values[at + 1]);
                        let result = PathResult {
                            radiance: Spectrum { r: r[0], g: r[1], b: r[2] },
                            background: Spectrum { r: b[0], g: b[1], b: b[2] },
                            coverage: r[3],
                            escaped: r[3] == 0.0,
                            ..Default::default()
                        };
                        pixel.accumulate_with(&result, reject, None);
                    }
                }
            }
            readback.unmap();
            done += count;
        }
        if on_card {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("read back") });
            encoder.copy_buffer_to_buffer(&pixel_buffer, 0, &readback, 0, pixel_bytes);
            self.gpu.queue.submit(Some(encoder.finish()));
            readback.slice(..pixel_bytes).map_async(wgpu::MapMode::Read, |r| {
                if let Err(why) = r {
                    eprintln!("  the card's pixels did not come back: {why}");
                }
            });
            self.gpu.wait();
            self.check_device();
            {
                let view = readback.slice(..pixel_bytes).get_mapped_range().expect("the pixels are mapped");
                let back: &[PixelSums] = bytemuck::cast_slice(&view[..]);
                for (pixel, s) in framebuffer.pixels_mut().iter_mut().zip(back) {
                    pixel.sum = Spectrum { r: s.sum[0], g: s.sum[1], b: s.sum[2] };
                    pixel.clamped_energy = s.sum[3];
                    pixel.background_sum = Spectrum { r: s.background[0], g: s.background[1], b: s.background[2] };
                    pixel.coverage_sum = s.background[3];
                    pixel.sum_squared_luminance = s.squares_high as f64 - s.squares_low as f64;
                    pixel.samples = s.samples;
                }
            }
            readback.unmap();
        }
    }

    /// A lost card hands back nothing to trust (a black picture, once, with
    /// no other sign): say so rather than pass it on.
    fn check_device(&self) {
        if self.gpu.is_lost() {
            panic!("the graphics card was lost during the render: the system reset it, or its driver failed (see above)");
        }
    }

    /// What a dispatch of `work` pixel samples that took `seconds` says of
    /// the card's pace: the next dispatches' work, aimed at
    /// [`DISPATCH_SECONDS`], at most twice the last and at most `most`.
    fn timed(&self, work: u64, seconds: f64, most: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        let pace = work as f64 / seconds.max(1e-6);
        let last = self.dispatch_work.load(Relaxed);
        let next = ((pace * DISPATCH_SECONDS) as u64).min(last.saturating_mul(2)).clamp(1, most.max(1));
        self.dispatch_work.store(next, Relaxed);
        self.dispatch_seconds.store(seconds.to_bits(), Relaxed);
    }

    /// Sets the work, in pixel samples, the next dispatch starts from, as
    /// [`GpuRenderer::render_pass`] would have learnt it: for tests that cut
    /// a frame finely, and for a caller that knows the card's pace already.
    pub fn set_dispatch_work(&self, work: u64) {
        self.dispatch_work.store(work.max(1), std::sync::atomic::Ordering::Relaxed);
        self.dispatch_seconds.store(f64::INFINITY.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// For differential tests of what reads the scene (`docs/07` R2.4): the
    /// transport's source with `kernel` after it, run from its entry point
    /// `entry` over `groups` workgroups against this renderer's scene,
    /// tables and environment, the frame as `settings` has it at one pixel.
    /// `words` are in `frame_samples` as it starts, read as four words a
    /// vector, and what it has there when it ends is handed back. Only what
    /// `entry` reaches is compiled: a module with two entry points is
    /// pared down to the one a pipeline takes.
    pub fn run_on_scene(&self, settings: &FrameSettings, kernel: &str, entry: &str, words: &[u32], groups: u32) -> Vec<u32> {
        self.run_on_scene_at(settings, (1, 1), kernel, entry, words, groups)
    }

    /// [`GpuRenderer::run_on_scene`] with the frame `width` by `height`, for
    /// what reads the frame's size: the camera's half-widths, its aspect.
    pub fn run_on_scene_at(&self, settings: &FrameSettings, (width, height): (usize, usize), kernel: &str, entry: &str, words: &[u32], groups: u32) -> Vec<u32> {
        let device = &self.gpu.device;
        let source = format!("{}\n{kernel}\n", crate::shader::source("transport"));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(entry), source: wgpu::ShaderSource::Wgsl(source.into()) });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(entry),
            layout: Some(&self.layout),
            module: &module,
            entry_point: Some(entry),
            compilation_options: Default::default(),
            cache: None,
        });
        let mut padded = words.to_vec();
        padded.resize(words.len().div_ceil(4).max(1) * 4, 0);
        let bytes = (padded.len() * 4) as u64;
        let init = |label: &str, contents: &[u8], usage| wgpu::util::DeviceExt::create_buffer_init(device, &wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage });
        let io = init("scene test", bytemuck::cast_slice(&padded), wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC);
        let pixel = init("scene test pixel", bytemuck::bytes_of(&PixelSums::default()), wgpu::BufferUsages::STORAGE);
        let params = self.params(settings, width, height, 0, 1);
        let uniform = init("scene test frame", bytemuck::bytes_of(&params), wgpu::BufferUsages::UNIFORM);
        let readback = device.create_buffer(&wgpu::BufferDescriptor { label: Some("scene test back"), size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene test frame"),
            layout: &self.frame_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: io.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: pixel.as_entire_binding() },
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(entry) });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(entry), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &self.scene_group, &[]);
            pass.set_bind_group(1, &self.tables.bind_group, &[]);
            pass.set_bind_group(2, &self.environment_group, &[]);
            pass.set_bind_group(3, &frame_group, &[]);
            pass.dispatch_workgroups(groups.max(1), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&io, 0, &readback, 0, bytes);
        self.gpu.queue.submit(Some(encoder.finish()));
        readback.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("the scene test's words map"));
        self.gpu.wait();
        let view = readback.slice(..).get_mapped_range().expect("the scene test's words are mapped");
        let mut out: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&view[..]).to_vec();
        out.truncate(words.len());
        out
    }
}

