//! The transport kernel's variants (`docs/07` R3, G1.6): the kernel built
//! with only the parts a scene uses.
//!
//! The transport is one megakernel, and the driver's cold compile of it
//! grows with every part ported into it. Most scenes use few of those
//! parts. `features.wgsl` names each part as a pipeline-overridable
//! constant, true unless the pipeline says otherwise, and guards the part's
//! code with it at the place the kernel already turns the part away when
//! the scene has none of it. naga writes an override as a plain constant
//! (`process_overrides`), and the driver drops a branch on a constant
//! false before its expensive work, so a variant compiles as if the part
//! had never been written.
//!
//! A variant leaves out only what the whole kernel would not have run for
//! the scene: [`KernelFeatures::of`] reads the same records the kernel
//! reads - the frame's flags, the materials', the programs' headers and
//! words, the instances', the meshes' - and keeps a part wherever any of
//! them could reach it. So a scene's variant draws the same samples in the
//! same order, and its picture is the whole kernel's to the bit: every
//! scene of the parity test renders with its own variant and is held to the
//! whole kernel's picture bit for bit (`tests/parity.rs`).
//!
//! A part ported later joins as one more: its constant in `features.wgsl`,
//! true by default; its gate, `FEATURE_X && ...`, at the place the kernel
//! already turns it away; a bit and a name here, and in
//! [`KernelFeatures::of`] the records that reach it. A scene that has the
//! part keeps it; one that has not leaves it out, and the parity test says
//! whether that was the whole kernel's picture.
//!
//! The same logic is not quite the same picture: where leaving code out lets
//! the driver see through what is left, it may fuse or reorder the
//! arithmetic of what the scene does run, and round otherwise. Two places
//! did: the anisotropic frame (`frame_of` fused into what reads the frame
//! once the other branch was gone), which is not gated as it saved no
//! compile time; and the glowing triangles' light sample skipped outright
//! (the sky's shadow ray through clear glass and cutouts rounded
//! otherwise), which is left out behind a stand-in the driver cannot see
//! through (`emitters.wgsl`). `tests/variants.rs` finds which part, and
//! which place of it, moves a picture.
//!
//! Some parts are a frame's, not a scene's: the camera's lens, its blades
//! and its projections, and the film's pixel filter.
//! The records cannot say which camera will look at a scene, so
//! [`KernelFeatures::of`] keeps none of them, and a renderer widens its
//! kernel to the ones a frame reaches ([`KernelFeatures::of_frame`]) at the
//! first frame that asks for one it left out, building the wider kernel
//! once and keeping it for the frames after.

use crate::lights::{FLAG_IES, FLAG_SHADOW_LINKS_ALL_BUT, FLAG_SHADOW_LINKS_ONLY};
use crate::material::GpuMaterial;
use crate::program::ProgramTable;
use crate::scene::{GpuInstance, GpuMesh, INSTANCE_CASTS_SHADOWS, INSTANCE_STAND_IN, INSTANCE_UNSEEN, NO_CATCHER, NO_DATA, NO_LIGHT};

/// The parts of the transport kernel a pipeline keeps: the parts
/// `features.wgsl` names, one bit each, and which of the interpreter's
/// instructions it keeps an arm for, a bit each by opcode (its
/// `FEATURE_OPS`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct KernelFeatures {
    pub parts: u32,
    pub ops: u32,
}

const fn part(bit: u32) -> KernelFeatures {
    KernelFeatures { parts: 1 << bit, ops: 0 }
}

impl KernelFeatures {
    /// Glass and media: the transmission lobe, the media a path is in, the
    /// split at the first plain glass, absorption.
    pub const GLASS: KernelFeatures = part(0);
    /// Clear glass a shadow ray passes, and glass a seeing bounce sees.
    pub const PANES: KernelFeatures = part(1);
    /// Cutouts.
    pub const CUTOUTS: KernelFeatures = part(2);
    /// Cutouts whose opacity a program drives.
    pub const CUT_PROGRAMS: KernelFeatures = part(3);
    /// The glowing triangles as lights.
    pub const EMITTERS: KernelFeatures = part(4);
    /// A glowing surface met at a hit.
    pub const GLOW: KernelFeatures = part(5);
    /// A glowing volume.
    pub const VOLUME_GLOW: KernelFeatures = part(6);
    /// The analytic lamps.
    pub const LAMPS: KernelFeatures = part(7);
    /// The shadow catcher's floor.
    pub const FLOOR: KernelFeatures = part(8);
    /// The material programs: the interpreter, its arms by [`Self::ops`].
    pub const PROGRAMS: KernelFeatures = part(9);
    /// The normal maps of pictures (the picture's arm is `OP_TEXTURE`'s).
    pub const TEXTURES: KernelFeatures = part(10);
    /// The measures at the mesh's corners at a hit (the arm is
    /// `OP_ATTRIBUTE`'s).
    pub const ATTRIBUTES: KernelFeatures = part(11);
    /// Rounded edges: the bevel's probes at the surface a path meets first.
    pub const BEVEL: KernelFeatures = part(12);
    /// A background colour or backplate past the parts, along a specular
    /// chain (`past_the_scene`).
    pub const BACKGROUND: KernelFeatures = part(13);
    /// The backplate's picture, read at the film point.
    pub const BACKPLATE: KernelFeatures = part(14);
    /// The thin lens (a frame's part: [`KernelFeatures::of_frame`]).
    pub const THIN_LENS: KernelFeatures = part(15);
    /// An opening of blades (a frame's part).
    pub const BLADES: KernelFeatures = part(16);
    /// A shifted lens's film (a frame's part).
    pub const SHIFT: KernelFeatures = part(17);
    /// Parallel rays from the film plane (a frame's part).
    pub const ORTHOGRAPHIC: KernelFeatures = part(18);
    /// Every direction round the camera (a frame's part).
    pub const EQUIRECTANGULAR: KernelFeatures = part(19);
    /// A pixel filter other than one pixel's box (a frame's part).
    pub const FILTER: KernelFeatures = part(20);
    /// A dome of a size: the sky from where a path is.
    pub const DOME: KernelFeatures = part(21);
    /// A part some kind of ray does not see, or one held out.
    pub const UNSEEN: KernelFeatures = part(22);
    /// A lamp's shadow links.
    pub const SHADOW_LINKS: KernelFeatures = part(23);
    /// The light past a part its shadow rays pass.
    pub const PASSES_PARTS: KernelFeatures = part(24);
    /// An IES lamp's profile.
    pub const IES: KernelFeatures = part(25);
    /// Metallic paint's flakes: the flake field at a point and the metal
    /// lobe under the coat in the flake's frame.
    pub const FLAKES: KernelFeatures = part(26);
    /// A thin film over the metal and the dielectric interface.
    pub const FILM: KernelFeatures = part(27);
    /// Glass that disperses light: a wavelength drawn at it, its index at
    /// that wavelength.
    pub const DISPERSION: KernelFeatures = part(28);

    /// Each part, with its constant's name in `features.wgsl`.
    pub const NAMED: [(KernelFeatures, &'static str); 29] = [
        (Self::GLASS, "FEATURE_GLASS"),
        (Self::PANES, "FEATURE_PANES"),
        (Self::CUTOUTS, "FEATURE_CUTOUTS"),
        (Self::CUT_PROGRAMS, "FEATURE_CUT_PROGRAMS"),
        (Self::EMITTERS, "FEATURE_EMITTERS"),
        (Self::GLOW, "FEATURE_GLOW"),
        (Self::VOLUME_GLOW, "FEATURE_VOLUME_GLOW"),
        (Self::LAMPS, "FEATURE_LAMPS"),
        (Self::FLOOR, "FEATURE_FLOOR"),
        (Self::PROGRAMS, "FEATURE_PROGRAMS"),
        (Self::TEXTURES, "FEATURE_TEXTURES"),
        (Self::ATTRIBUTES, "FEATURE_ATTRIBUTES"),
        (Self::BEVEL, "FEATURE_BEVEL"),
        (Self::BACKGROUND, "FEATURE_BACKGROUND"),
        (Self::BACKPLATE, "FEATURE_BACKPLATE"),
        (Self::THIN_LENS, "FEATURE_THIN_LENS"),
        (Self::BLADES, "FEATURE_BLADES"),
        (Self::SHIFT, "FEATURE_SHIFT"),
        (Self::ORTHOGRAPHIC, "FEATURE_ORTHOGRAPHIC"),
        (Self::EQUIRECTANGULAR, "FEATURE_EQUIRECTANGULAR"),
        (Self::FILTER, "FEATURE_FILTER"),
        (Self::DOME, "FEATURE_DOME"),
        (Self::UNSEEN, "FEATURE_UNSEEN"),
        (Self::SHADOW_LINKS, "FEATURE_SHADOW_LINKS"),
        (Self::PASSES_PARTS, "FEATURE_PASSES_PARTS"),
        (Self::IES, "FEATURE_IES"),
        (Self::FLAKES, "FEATURE_FLAKES"),
        (Self::FILM, "FEATURE_FILM"),
        (Self::DISPERSION, "FEATURE_DISPERSION"),
    ];

    /// The instructions' constant in `features.wgsl`.
    pub const OPS: &'static str = "FEATURE_OPS";

    /// None of them: a grey product under the sky.
    pub const NONE: KernelFeatures = KernelFeatures { parts: 0, ops: 0 };
    /// All of them: the whole kernel, which renders every scene.
    pub const ALL: KernelFeatures = KernelFeatures { parts: (1 << Self::NAMED.len()) - 1, ops: u32::MAX };

    /// The instruction `op`'s arm.
    pub const fn op(op: u32) -> KernelFeatures {
        KernelFeatures { parts: 0, ops: 1 << op }
    }

    /// Whether every part and instruction `other` keeps, this keeps.
    pub fn contains(self, other: KernelFeatures) -> bool {
        self.parts & other.parts == other.parts && self.ops & other.ops == other.ops
    }

    pub fn is_empty(self) -> bool {
        self == Self::NONE
    }

    /// These without what `other` keeps.
    pub fn without(self, other: KernelFeatures) -> KernelFeatures {
        KernelFeatures { parts: self.parts & !other.parts, ops: self.ops & !other.ops }
    }

    /// The pipeline's constants: every part, kept (1) or left out (0), and
    /// the instructions' bits.
    pub fn constants(self) -> Vec<(&'static str, f64)> {
        let mut constants: Vec<(&'static str, f64)> = Self::NAMED.iter().map(|(f, name)| (*name, if self.contains(*f) { 1.0 } else { 0.0 })).collect();
        constants.push((Self::OPS, self.ops as f64));
        constants
    }

    /// What a variant is called in a cache's file names.
    pub fn key(self) -> String {
        format!("{:04x}-{:08x}", self.parts, self.ops)
    }

    /// The parts a scene's records reach (see the module's notes): each
    /// kept wherever a record the kernel reads could send a path into it.
    pub fn of(r: &SceneRecords) -> KernelFeatures {
        use crate::material::{GLASS_BOUNDARY, GLASS_PANE};
        use crate::program::{PROGRAM_AT_HIT, PROGRAM_CUTS, PROGRAM_NORMAL_PROJECTED, PROGRAM_NORMAL_UV, PROGRAM_RUNS};
        use crate::transport::{SCENE_BACKGROUND_COLOUR, SCENE_BACKPLATE, SCENE_CUTOUTS, SCENE_CUT_PROGRAMS, SCENE_FINITE_DOME, SCENE_PANES, SCENE_TRANSMITS};
        let mut f = KernelFeatures::NONE;
        let mut keep = |on: bool, part: KernelFeatures| {
            if on {
                f = f | part;
            }
        };
        let any_material = |test: &dyn Fn(&GpuMaterial) -> bool| r.materials.iter().any(test);
        let headers = &r.programs.headers;
        let any_header = |flags: u32| headers.iter().any(|h| h.flags & flags != 0);
        let ops = ProgramOps::of(r.programs);

        // transport.wgsl's enclosing_media (the frame's SCENE_TRANSMITS), a
        // medium's boundary (shade_vertex, the null crossing), the
        // transmission lobe (openpbr_build, its weight), the split and the
        // transmissions after it (a pane, below).
        let panes = r.glass_flags & SCENE_PANES != 0 || any_material(&|m| m.glass & GLASS_PANE != 0);
        keep(
            r.glass_flags & SCENE_TRANSMITS != 0 || panes || any_material(&|m| m.glass & GLASS_BOUNDARY != 0 || !(m.transmission_color[3] <= 0.0)),
            Self::GLASS,
        );
        // shadow_through (SCENE_PANES), a pane at a vertex and the split.
        keep(panes, Self::PANES);
        // A cutout at a hit, shadow_through and blocked_towards: the frame's
        // SCENE_CUTOUTS, which `GlassScene` sets from the scene; and a
        // material record's opacity, or a program's, for good measure.
        keep(
            r.glass_flags & SCENE_CUTOUTS != 0 || any_material(&|m| !(m.geometry_opacity >= 0.999)) || any_header(PROGRAM_CUTS),
            Self::CUTOUTS,
        );
        // crossing_opacity (a header's PROGRAM_CUTS), the replays (the
        // frame's SCENE_CUT_PROGRAMS).
        keep(r.glass_flags & SCENE_CUT_PROGRAMS != 0 || any_header(PROGRAM_CUTS), Self::CUT_PROGRAMS);
        // The frame's SCENE_EMITTERS, which is the tree's having leaves
        // (and a configuration's light_source_materials).
        keep(r.emitters, Self::EMITTERS);
        // material_is_emissive of the material at a hit: its record's, or
        // what a program that drives the glow writes over it.
        keep(
            any_material(&|m| !(m.emission_color[3] <= 0.0) && m.emission_color[..3].iter().any(|c| *c != 0.0)) || any_header(PROGRAM_AT_HIT),
            Self::GLOW,
        );
        keep(any_material(&|m| m.volume_glow[..3].iter().any(|c| *c != 0.0)), Self::VOLUME_GLOW);
        // The frame's light_count, and a stand-in's lamp at a glow.
        keep(r.lamps > 0 || r.instances.iter().any(|i| i.light != NO_LIGHT), Self::LAMPS);
        keep(r.instances.iter().any(|i| i.catcher != NO_CATCHER), Self::FLOOR);
        // A header's PROGRAM_RUNS, which every program on the card has; and
        // each instruction's arm, by the instructions the words hold.
        keep(any_header(PROGRAM_RUNS) || ops.seen != 0, Self::PROGRAMS);
        // An opcode past the constant's 32 bits keeps every arm.
        keep(true, KernelFeatures { parts: 0, ops: if ops.seen >> 32 != 0 { u32::MAX } else { ops.seen as u32 } });
        // prog_shade's and material_at's normal maps (a header's flags).
        keep(any_header(PROGRAM_NORMAL_UV | PROGRAM_NORMAL_PROJECTED), Self::TEXTURES);
        // mesh_measures (a mesh's attribute_base).
        keep(r.meshes.iter().any(|m| m.attribute_base != NO_DATA), Self::ATTRIBUTES);
        // bevel_normal (the frame's SCENE_BEVEL, which a material record's
        // edge radius sets).
        keep(any_material(&|m| m.edge_radius > 0.0), Self::BEVEL);
        // past_the_scene (the frame's SCENE_BACKGROUND_COLOUR and
        // SCENE_BACKPLATE, which the scene's background sets).
        keep(r.glass_flags & (SCENE_BACKGROUND_COLOUR | SCENE_BACKPLATE) != 0, Self::BACKGROUND);
        keep(r.glass_flags & SCENE_BACKPLATE != 0, Self::BACKPLATE);
        // The dome's lookups (the frame's SCENE_FINITE_DOME).
        keep(r.glass_flags & SCENE_FINITE_DOME != 0, Self::DOME);
        // A part some ray passes, or held out (an instance's flags).
        keep(r.instances.iter().any(|i| i.flags & INSTANCE_UNSEEN != 0), Self::UNSEEN);
        // shadow_through's walk for a lamp's shadow links (a lamp's flags).
        keep(r.lamp_flags & (FLAG_SHADOW_LINKS_ONLY | FLAG_SHADOW_LINKS_ALL_BUT) != 0, Self::SHADOW_LINKS);
        // light_past_part and nearness_at (the frame's LAMPS_PASSES_PARTS
        // and LAMPS_SHADOWLESS): a part that casts no shadow and is no
        // lamp's stand-in (an instance's flags), or a lamp a ray can meet
        // that casts no shadow or has shadow links (a lamp's).
        keep(
            r.instances.iter().any(|i| i.flags & (INSTANCE_CASTS_SHADOWS | INSTANCE_STAND_IN) == 0) || r.lamps_met_past,
            Self::PASSES_PARTS,
        );
        // lamp_sample's and light_irradiance_bound's IES lamp (a lamp's
        // flags).
        keep(r.lamp_flags & FLAG_IES != 0, Self::IES);
        // The flake field and the flake's lobe (shade_vertex, openpbr_build:
        // a material record's flake weight, which no program drives).
        keep(any_material(&|m| m.flake_color[3] > 1e-4), Self::FLAKES);
        // The film over the metal and the interface (openpbr_build: a
        // material record's film weight, which no program drives).
        keep(any_material(&|m| m.film[0] > 1e-4), Self::FILM);
        // A dispersive surface (shade_vertex: a material record's Abbe
        // number, which no program drives, whatever its transmission, which
        // a program may).
        keep(any_material(&|m| m.film[3] > 0.0), Self::DISPERSION);
        // A floor under a dome of a size keeps the programs' interpreter
        // too, whether a program runs or not: without it the floor's
        // shadow under the dome came out otherwise (finite_dome: 7,269 of
        // 57,600 pixels, a hundredth darker), whichever way the spread
        // bound was written, where the whole kernel gives the processor's
        // picture. The interpreter never runs there; the driver compiles
        // the shadow's loop otherwise without it (tests/variants.rs's
        // which_part_left_out_moves_a_picture).
        if f.contains(Self::FLOOR | Self::DOME) {
            f = f | Self::PROGRAMS;
        }
        f
    }

    /// The parts a frame reaches through its camera and film, from the
    /// frame's `camera_flags` as the kernel reads them: the thin lens, its
    /// blades, a shifted film, the orthographic and 360-degree projections,
    /// a pixel filter other than one pixel's box. A scene's records cannot say which camera will look at it,
    /// so [`KernelFeatures::of`] keeps none of these, and a renderer widens
    /// its kernel to them at the first frame that asks for one it left out
    /// (`GpuRenderer::render_pass`).
    pub fn of_frame(camera_flags: u32) -> KernelFeatures {
        use crate::transport::{CAMERA_BLADED, CAMERA_EQUIRECTANGULAR, CAMERA_FILTER_BOX, CAMERA_FILTER_TABLE, CAMERA_ORTHOGRAPHIC, CAMERA_SHIFT, CAMERA_THIN_LENS};
        [
            (CAMERA_THIN_LENS, Self::THIN_LENS),
            (CAMERA_BLADED, Self::BLADES),
            (CAMERA_SHIFT, Self::SHIFT),
            (CAMERA_ORTHOGRAPHIC, Self::ORTHOGRAPHIC),
            (CAMERA_EQUIRECTANGULAR, Self::EQUIRECTANGULAR),
            (CAMERA_FILTER_BOX | CAMERA_FILTER_TABLE, Self::FILTER),
        ]
        .into_iter()
        .filter(|(flag, _)| camera_flags & flag != 0)
        .fold(KernelFeatures::NONE, |f, (_, part)| f | part)
    }

    /// Each part's name, without its prefix, and the instructions:
    /// "lamps, programs; instructions 2, 24".
    pub fn names(self) -> String {
        let names: Vec<String> = Self::NAMED
            .iter()
            .filter(|(f, _)| self.contains(*f))
            .map(|(_, name)| name.trim_start_matches("FEATURE_").to_ascii_lowercase().replace('_', " "))
            .collect();
        let mut out = if names.is_empty() { "none".to_string() } else { names.join(", ") };
        if self.ops == u32::MAX {
            out.push_str("; every instruction");
        } else if self.ops != 0 {
            let ops: Vec<String> = (0..32).filter(|b| self.ops & (1 << b) != 0).map(|b| b.to_string()).collect();
            out.push_str(&format!("; instructions {}", ops.join(", ")));
        }
        out
    }
}

impl std::fmt::Debug for KernelFeatures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KernelFeatures({})", self.names())
    }
}

impl std::ops::BitOr for KernelFeatures {
    type Output = KernelFeatures;
    fn bitor(self, other: KernelFeatures) -> KernelFeatures {
        KernelFeatures { parts: self.parts | other.parts, ops: self.ops | other.ops }
    }
}

/// What [`KernelFeatures::of`] reads: the records as the renderer uploads
/// them.
pub struct SceneRecords<'a> {
    pub materials: &'a [GpuMaterial],
    pub programs: &'a ProgramTable,
    pub instances: &'a [GpuInstance],
    pub meshes: &'a [GpuMesh],
    /// How many lamps (`FrameParams::light_count`).
    pub lamps: u32,
    /// Every lamp record's flags, or'd (`crate::lights::GpuLight::flags`).
    pub lamp_flags: u32,
    /// Whether some lamp a ray can meet casts no shadow or has shadow links.
    pub lamps_met_past: bool,
    /// Whether the glowing triangles' tree has leaves.
    pub emitters: bool,
    /// `FrameParams::glass_flags` as the scene sets them, whatever the
    /// configuration: `SCENE_TRANSMITS`, `SCENE_PANES`, `SCENE_CUTOUTS`,
    /// `SCENE_CUT_PROGRAMS`, `SCENE_BACKGROUND_COLOUR`, `SCENE_BACKPLATE`,
    /// `SCENE_FINITE_DOME`.
    pub glass_flags: u32,
}

/// Which instructions the programs hold, a bit each by opcode, walked as
/// `prog_run` walks them.
struct ProgramOps {
    seen: u64,
}

impl ProgramOps {
    fn of(table: &ProgramTable) -> ProgramOps {
        let mut ops = ProgramOps { seen: 0 };
        for h in &table.headers {
            for (start, end) in [(h.main_start, h.main_end), (h.bump_start, h.bump_end)] {
                let mut pc = start as usize;
                while pc < end as usize && pc < table.words.len() {
                    let head = table.words[pc];
                    let op = head & 0xFF;
                    ops.seen |= if op < 64 { 1u64 << op } else { 1u64 << 63 };
                    pc += 2 + (head >> 24) as usize;
                }
            }
        }
        ops
    }
}

/// What this machine has built of the transport kernel, kept between runs
/// (`docs/07` G1.6): one file per variant built, named for the card, its
/// driver, the kernel's source and the variant, holding the driver's
/// pipeline cache where the device keeps one (Vulkan's, through
/// `wgpu::PipelineCache`), else empty (Metal keeps its own). A file says the
/// variant was built here, by this driver, from this source, so building it
/// again is the driver's cache answering rather than a cold compile; a new
/// driver or a new kernel names other files.
///
/// `GpuRenderer::with_cache` uses it so that a first picture waits on a
/// cold compile only on a machine that has never built the kernel: a
/// scene's own variant where that is kept; else the whole kernel where that
/// is, while the variant is built aside and taken up once it is ready (the
/// same picture, so at any pass); else, the first time, the variant, the
/// smallest cold compile there is, with the whole kernel built aside for
/// the scenes after.
#[derive(Clone, Debug)]
pub struct KernelCache {
    dir: std::path::PathBuf,
    /// The card, its driver and the kernel's source, as a name.
    prefix: String,
}

impl KernelCache {
    /// The cache in `dir` (made when first written) for the kernels the
    /// device `info` describes builds from this crate's source.
    pub fn new(dir: impl Into<std::path::PathBuf>, info: &wgpu::AdapterInfo) -> KernelCache {
        let card = format!("{} / {} / {} / {:?} {} {}", info.name, info.driver, info.driver_info, info.backend, info.vendor, info.device);
        KernelCache::for_card(dir, &card)
    }

    /// The cache for a card and driver named `card`.
    pub fn for_card(dir: impl Into<std::path::PathBuf>, card: &str) -> KernelCache {
        let mut hash = Fnv::default();
        hash.write(card.as_bytes());
        let card = hash.0;
        let mut hash = Fnv::default();
        hash.write(crate::shader::source("transport").as_bytes());
        KernelCache { dir: dir.into(), prefix: format!("transport-{card:016x}-{:016x}", hash.0) }
    }

    fn path(&self, kernel: KernelFeatures) -> std::path::PathBuf {
        self.dir.join(format!("{}-{}.bin", self.prefix, kernel.key()))
    }

    /// Whether this machine has built `kernel`.
    pub fn knows(&self, kernel: KernelFeatures) -> bool {
        self.path(kernel).is_file()
    }

    /// The pipeline cache to build `kernel` with on `device`: what was kept
    /// of it, or an empty one; none where the device keeps none.
    pub fn pipeline_cache(&self, device: &wgpu::Device, kernel: KernelFeatures) -> Option<wgpu::PipelineCache> {
        if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
            return None;
        }
        let data = std::fs::read(self.path(kernel)).ok().filter(|d| !d.is_empty());
        // SAFETY: the data is what `PipelineCache::get_data` gave on this
        // card with this driver for this kernel (the file's name says so),
        // or none. wgpu checks its header, and `fallback` starts an empty
        // cache where it does not match.
        Some(unsafe { device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor { label: Some("transport"), data: data.as_deref(), fallback: true }) })
    }

    /// Note that `kernel` has been built, keeping what `cache` holds.
    pub fn keep(&self, kernel: KernelFeatures, cache: Option<&wgpu::PipelineCache>) {
        let data = cache.and_then(wgpu::PipelineCache::get_data).unwrap_or_default();
        let path = self.path(kernel);
        if std::fs::create_dir_all(&self.dir).is_err() {
            return;
        }
        // Written aside and renamed, so no reader finds half a file.
        let aside = path.with_extension(format!("{}.part", std::process::id()));
        if std::fs::write(&aside, &data).is_ok() && std::fs::rename(&aside, &path).is_err() {
            let _ = std::fs::remove_file(&aside);
        }
    }
}

/// FNV-1a, 64 bits: a name that stays the same from one build of Tequila to
/// the next, as std's hashers do not promise.
#[derive(Clone, Copy)]
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        // Each part ends, so "ab","c" is not "a","bc".
        self.0 = (self.0 ^ 0xff).wrapping_mul(0x0000_0100_0000_01b3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::{GLASS_BOUNDARY, GLASS_PANE};
    use crate::program::{CardProgram, ProgramTable};
    use crate::scene::{GpuInstance, GpuMesh};

    fn grey() -> GpuMaterial {
        let mut m = GpuMaterial::from_material(&tequila_materials::openpbr::Material::default());
        m.base_color = [0.5, 0.5, 0.5, 0.0];
        m
    }

    /// A part as the scene uploads one: seen by every ray, casting shadows.
    fn instance() -> GpuInstance {
        GpuInstance { catcher: NO_CATCHER, light: NO_LIGHT, emitter: crate::emitters::EMITTER_NONE, flags: INSTANCE_CASTS_SHADOWS, ..GpuInstance::default() }
    }

    fn mesh() -> GpuMesh {
        GpuMesh { uv_base: NO_DATA, tangent_base: NO_DATA, attribute_base: NO_DATA, ..GpuMesh::default() }
    }

    fn features(materials: &[GpuMaterial], programs: &ProgramTable, instances: &[GpuInstance], meshes: &[GpuMesh], lamps: u32, emitters: bool, glass_flags: u32) -> KernelFeatures {
        KernelFeatures::of(&SceneRecords { materials, programs, instances, meshes, lamps, lamp_flags: 0, lamps_met_past: false, emitters, glass_flags })
    }

    /// A program of one instruction `op` with `operands` operands, writing
    /// `writes` back.
    fn program(op: u32, writes: Vec<u32>) -> CardProgram {
        CardProgram { main: vec![op | (0xFF << 16) | (1 << 24), 0, 0], bump: Vec::new(), constants: vec![[0.0; 4]], writes, bump_here: u32::MAX, bump_height: u32::MAX, tangent: u32::MAX, registers: 1, normal_map: 0 }
    }

    #[test]
    fn a_grey_product_under_the_sky_needs_none_of_them() {
        let empty = ProgramTable::new(&[None]);
        let f = features(&[grey()], &empty, &[instance()], &[mesh()], 0, false, 0);
        assert_eq!(f, KernelFeatures::NONE, "{f:?}");
    }

    #[test]
    fn each_part_is_kept_by_what_reaches_it() {
        let empty = ProgramTable::new(&[None]);
        let one = |m: GpuMaterial| features(&[grey(), m], &empty, &[instance()], &[mesh()], 0, false, 0);
        let mut glass = grey();
        glass.transmission_color[3] = 1.0;
        assert_eq!(one(glass), KernelFeatures::GLASS);
        let mut boundary = grey();
        boundary.glass = GLASS_BOUNDARY;
        assert_eq!(one(boundary), KernelFeatures::GLASS);
        let mut pane = grey();
        pane.glass = GLASS_PANE;
        assert_eq!(one(pane), KernelFeatures::GLASS | KernelFeatures::PANES);
        let mut cut = grey();
        cut.geometry_opacity = 0.5;
        assert_eq!(one(cut), KernelFeatures::CUTOUTS);
        let mut glowing = grey();
        glowing.emission_color = [1.0, 0.5, 0.0, 2.0];
        assert_eq!(one(glowing), KernelFeatures::GLOW);
        let mut dark = grey();
        dark.emission_color = [0.0, 0.0, 0.0, 2.0];
        assert_eq!(one(dark), KernelFeatures::NONE, "a glow of no colour is none");
        let mut volume = grey();
        volume.volume_glow = [0.0, 1.0, 0.0, 0.5];
        assert_eq!(one(volume), KernelFeatures::VOLUME_GLOW);

        // The frame's flags, the lamps, the tree, the instances, the meshes.
        use crate::transport::{SCENE_CUTOUTS, SCENE_TRANSMITS};
        assert_eq!(features(&[grey()], &empty, &[instance()], &[mesh()], 0, false, SCENE_TRANSMITS), KernelFeatures::GLASS);
        assert_eq!(features(&[grey()], &empty, &[instance()], &[mesh()], 0, false, SCENE_CUTOUTS), KernelFeatures::CUTOUTS);
        assert_eq!(features(&[grey()], &empty, &[instance()], &[mesh()], 3, false, 0), KernelFeatures::LAMPS);
        assert_eq!(features(&[grey()], &empty, &[instance()], &[mesh()], 0, true, 0), KernelFeatures::EMITTERS);
        let floor = GpuInstance { catcher: 0, ..instance() };
        assert_eq!(features(&[grey()], &empty, &[instance(), floor], &[mesh()], 0, false, 0), KernelFeatures::FLOOR);
        let stand_in = GpuInstance { light: 0, ..instance() };
        assert_eq!(features(&[grey()], &empty, &[stand_in], &[mesh()], 0, false, 0), KernelFeatures::LAMPS);
        let measured = GpuMesh { attribute_base: 0, ..mesh() };
        assert_eq!(features(&[grey()], &empty, &[instance()], &[measured], 0, false, 0), KernelFeatures::ATTRIBUTES);
    }

    #[test]
    fn a_programs_instructions_keep_their_families() {
        use crate::program::*;
        let with = |p: CardProgram| {
            let table = ProgramTable::new(&[None, Some(p)]);
            features(&[grey(), grey()], &table, &[instance()], &[mesh()], 0, false, 0)
        };
        // Each instruction keeps its own arm and no other.
        for op in [OP_INPUT, OP_MATH, OP_MIX, OP_NOISE, OP_VORONOI, OP_TEXTURE, OP_PROCEDURAL, OP_WAVE, OP_ATTRIBUTE, OP_GABOR] {
            assert_eq!(with(program(op, Vec::new())), KernelFeatures::PROGRAMS | KernelFeatures::op(op), "instruction {op}");
        }
        let two = ProgramTable::new(&[Some(program(OP_MATH, Vec::new())), Some(program(OP_TEXTURE, Vec::new()))]);
        assert_eq!(features(&[grey(), grey()], &two, &[instance()], &[mesh()], 0, false, 0), KernelFeatures::PROGRAMS | KernelFeatures::op(OP_MATH) | KernelFeatures::op(OP_TEXTURE));
        let base = KernelFeatures::PROGRAMS | KernelFeatures::op(OP_MATH);
        // What it writes: the glow, the opacity.
        let at = |field: usize| (field / 4) as u32;
        let glow = at(std::mem::offset_of!(GpuMaterial, emission_color));
        assert_eq!(with(program(OP_MATH, vec![glow | 0x100, 0])), base | KernelFeatures::GLOW);
        let opacity = at(std::mem::offset_of!(GpuMaterial, geometry_opacity));
        assert_eq!(with(program(OP_MATH, vec![opacity, 0])), base | KernelFeatures::GLOW | KernelFeatures::CUTOUTS | KernelFeatures::CUT_PROGRAMS);
        // A colour written over the opacity's float and the two after it.
        assert_eq!(with(program(OP_MATH, vec![opacity | 0x100, 0])), base | KernelFeatures::GLOW | KernelFeatures::CUTOUTS | KernelFeatures::CUT_PROGRAMS);
        // A normal map.
        let mut normal = program(OP_MATH, Vec::new());
        normal.normal_map = PROGRAM_NORMAL_UV;
        assert_eq!(with(normal), base | KernelFeatures::TEXTURES);
        // An instruction past the constant's bits keeps every arm.
        assert_eq!(with(program(40, Vec::new())).ops, u32::MAX);
    }

    #[test]
    fn a_kept_kernel_is_known_by_its_card_driver_source_and_parts() {
        let dir = std::env::temp_dir().join(format!("tequila-kernel-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let card = "A card / driver 610.88";
        let cache = KernelCache::for_card(&dir, card);
        assert!(!cache.knows(KernelFeatures::ALL) && !cache.knows(KernelFeatures::NONE));
        cache.keep(KernelFeatures::NONE, None);
        assert!(cache.knows(KernelFeatures::NONE) && !cache.knows(KernelFeatures::ALL));
        // Another driver knows nothing of it.
        let updated = KernelCache::for_card(&dir, "A card / driver 615.00");
        assert!(!updated.knows(KernelFeatures::NONE));
        assert!(KernelCache::for_card(&dir, card).knows(KernelFeatures::NONE));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_constants_name_every_part_once() {
        let all = KernelFeatures::ALL.constants();
        assert_eq!(all.len(), KernelFeatures::NAMED.len() + 1);
        assert!(all.iter().all(|(name, v)| *v == if *name == KernelFeatures::OPS { u32::MAX as f64 } else { 1.0 }));
        assert!(KernelFeatures::NONE.constants().iter().all(|(_, v)| *v == 0.0));
        let features = crate::shader::module("features").unwrap();
        let mut bits = 0;
        for (f, name) in KernelFeatures::NAMED {
            assert_eq!((f.parts.count_ones(), f.ops), (1, 0), "{name}");
            assert_eq!(bits & f.parts, 0, "{name} shares a bit");
            bits |= f.parts;
            assert!(features.contains(&format!("override {name}: bool = true;")), "features.wgsl has no {name}");
        }
        assert_eq!(bits, KernelFeatures::ALL.parts);
        assert!(features.contains("override FEATURE_OPS: u32 = 0xFFFFFFFFu;"));
        // And every arm of the interpreter is the instruction's.
        let program = crate::shader::module("program").unwrap();
        for op in 1..=28 {
            assert!(program.contains(&format!("case {op}u: {{ if (prog_holds({op}u)) {{")), "arm {op} is not kept by its instruction");
        }
    }
}
