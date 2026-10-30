//! Render settings and the render queue (`TEQUILA_SPEC.md` §1.8, `docs/04`
//! §11).
//!
//! What a person sets before pressing Render, and what happens to the jobs
//! afterwards. Both live in the document rather than in the application: a
//! render setup is part of a project — the resolution a client asked for, the
//! passes a compositor needs, the folder everything lands in — and losing it
//! when the window closes would mean setting it up again every session.
//!
//! # Halting
//!
//! §1.8 asks for a sample limit *and* a time limit, and they are not
//! alternatives. A sample count is what makes two renders comparable; a time
//! limit is what makes a queue of forty of them finish before a deadline.
//! Whichever comes first stops the render, and [`RenderSettings::stop_after`]
//! is the one place that decision is made so the viewport, the queue and the
//! command line cannot disagree about it.

use std::time::Duration;

/// Precision of a written file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BitDepth {
    #[default]
    Eight,
    Sixteen,
    ThirtyTwo,
}

impl BitDepth {
    pub const ALL: [BitDepth; 3] = [BitDepth::Eight, BitDepth::Sixteen, BitDepth::ThirtyTwo];

    pub fn label(self) -> &'static str {
        match self {
            BitDepth::Eight => "8-bit",
            BitDepth::Sixteen => "16-bit",
            BitDepth::ThirtyTwo => "32-bit",
        }
    }
}

/// A file format, as the document names it.
///
/// Kept apart from `tequila-format`'s own enum on purpose: this one is what a
/// document stores and a panel offers, and it must be serializable and stable
/// across versions. The format crate's carries a quality number and a precision
/// and is what the writer takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Png,
    Jpeg,
    Tiff,
    Exr,
    Psd,
}

impl OutputFormat {
    pub const ALL: [OutputFormat; 5] = [
        OutputFormat::Png,
        OutputFormat::Jpeg,
        OutputFormat::Tiff,
        OutputFormat::Exr,
        OutputFormat::Psd,
    ];

    pub fn label(self) -> &'static str {
        match self {
            OutputFormat::Png => "PNG",
            OutputFormat::Jpeg => "JPEG",
            OutputFormat::Tiff => "TIFF",
            OutputFormat::Exr => "EXR",
            OutputFormat::Psd => "PSD",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            OutputFormat::Png => "png",
            OutputFormat::Jpeg => "jpeg",
            OutputFormat::Tiff => "tiff",
            OutputFormat::Exr => "exr",
            OutputFormat::Psd => "psd",
        }
    }

    pub fn from_key(key: &str) -> Option<OutputFormat> {
        OutputFormat::ALL.into_iter().find(|f| f.key() == key)
    }

    /// Which depths make sense. Offering 32-bit PNG would be offering something
    /// that cannot be written.
    pub fn depths(self) -> &'static [BitDepth] {
        match self {
            OutputFormat::Png => &[BitDepth::Eight, BitDepth::Sixteen],
            OutputFormat::Jpeg => &[BitDepth::Eight],
            OutputFormat::Exr => &[BitDepth::Sixteen, BitDepth::ThirtyTwo],
            OutputFormat::Tiff | OutputFormat::Psd => &BitDepth::ALL,
        }
    }

    /// Whether the format can hold the render passes.
    pub fn carries_passes(self) -> bool {
        matches!(self, OutputFormat::Exr | OutputFormat::Psd)
    }
}

/// How the coverage is folded in, as the document names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AlphaMode {
    /// No transparency: the picture as it looks on screen. The default, as
    /// "include alpha" is off by default in every renderer a product
    /// photographer uses, because most renders go to a viewer, not a
    /// compositor.
    #[default]
    None,
    Straight,
    Premultiplied,
    Aliased,
    /// Cut out along the product's outline, as a photographer's clipping
    /// path: everything inside it kept, glass included with what it shows of
    /// the studio behind. The other modes make glass as transparent as it
    /// is, which cuts a lightbulb out as a ghost.
    Silhouette,
}

impl AlphaMode {
    pub const ALL: [AlphaMode; 5] = [
        AlphaMode::None,
        AlphaMode::Straight,
        AlphaMode::Premultiplied,
        AlphaMode::Aliased,
        AlphaMode::Silhouette,
    ];

    /// Whether the renderer leaves the background out of the picture. Not
    /// for None, which keeps it, nor for Silhouette, which keeps what shows
    /// through the product and cuts along its outline afterwards.
    pub fn transparent_background(self) -> bool {
        !matches!(self, AlphaMode::None | AlphaMode::Silhouette)
    }

    pub fn label(self) -> &'static str {
        match self {
            AlphaMode::None => "None",
            AlphaMode::Straight => "Straight",
            AlphaMode::Premultiplied => "Premultiplied",
            AlphaMode::Aliased => "Aliased",
            AlphaMode::Silhouette => "Silhouette",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            AlphaMode::None => "none",
            AlphaMode::Straight => "straight",
            AlphaMode::Premultiplied => "premultiplied",
            AlphaMode::Aliased => "aliased",
            AlphaMode::Silhouette => "silhouette",
        }
    }

    pub fn from_key(key: &str) -> Option<AlphaMode> {
        AlphaMode::ALL.into_iter().find(|m| m.key() == key)
    }
}

/// A rectangle of the image, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

/// Which path tracer renders the final image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderEngine {
    /// Tequila's own.
    Tequila,
    /// Cycles, Blender's path tracer, on the processor.
    CyclesCpu,
    /// Cycles on the graphics card: the default. Where there is no
    /// card it can use it renders on the processor, and where Cycles is not
    /// installed, or the render needs what only Tequila's own renderer
    /// makes, Tequila's renders instead (the app's `batch::engine_for`).
    #[default]
    CyclesGpu,
}

impl RenderEngine {
    pub const ALL: [RenderEngine; 3] = [RenderEngine::Tequila, RenderEngine::CyclesCpu, RenderEngine::CyclesGpu];

    /// How it is written in a project file.
    pub fn key(self) -> &'static str {
        match self {
            RenderEngine::Tequila => "tequila",
            RenderEngine::CyclesCpu => "cycles-cpu",
            RenderEngine::CyclesGpu => "cycles-gpu",
        }
    }

    pub fn from_key(key: &str) -> Option<RenderEngine> {
        RenderEngine::ALL.into_iter().find(|engine| engine.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            RenderEngine::Tequila => "Tequila",
            RenderEngine::CyclesCpu => "Cycles (processor)",
            RenderEngine::CyclesGpu => "Cycles (graphics card)",
        }
    }

    pub fn is_cycles(self) -> bool {
        self != RenderEngine::Tequila
    }

    /// The Cycles device it renders on. On the graphics card: Metal on a Mac;
    /// elsewhere OptiX - an NVIDIA card's ray-tracing cores - which
    /// `tequila_render_cycles` falls back from to plain CUDA where OptiX
    /// will not start.
    pub fn cycles_device(self) -> &'static str {
        match self {
            RenderEngine::CyclesGpu if cfg!(target_os = "macos") => "METAL",
            RenderEngine::CyclesGpu => "OPTIX",
            _ => "CPU",
        }
    }
}

/// What a render makes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderKind {
    /// The picture: path traced by the engine.
    #[default]
    Photo,
    /// A line drawing of the model's edges, drawn rather than traced: the
    /// viewport's Wireframe at the render's size, in a second or two. A job
    /// asks for it ("Render wireframe"); the document's own settings stay
    /// on Photo unless a script says otherwise, so one wireframe does not
    /// turn every render after it into one.
    Wireframe,
}

impl RenderKind {
    pub const ALL: [RenderKind; 2] = [RenderKind::Photo, RenderKind::Wireframe];

    pub fn key(self) -> &'static str {
        match self {
            RenderKind::Photo => "photo",
            RenderKind::Wireframe => "wireframe",
        }
    }

    pub fn from_key(key: &str) -> Option<RenderKind> {
        RenderKind::ALL.into_iter().find(|kind| kind.key() == key)
    }
}

/// Which edges a wireframe draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WireEdges {
    /// Every edge of every triangle, as the viewport's Wireframe shows them.
    #[default]
    Mesh,
    /// The part's own edges - a CAD part's B-rep edges, or a mesh's creases
    /// and borders - and its outline as the camera sees it: a line drawing
    /// rather than the tessellation.
    Model,
}

impl WireEdges {
    pub const ALL: [WireEdges; 2] = [WireEdges::Mesh, WireEdges::Model];

    pub fn key(self) -> &'static str {
        match self {
            WireEdges::Mesh => "mesh",
            WireEdges::Model => "model",
        }
    }

    pub fn from_key(key: &str) -> Option<WireEdges> {
        WireEdges::ALL.into_iter().find(|edges| edges.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            WireEdges::Mesh => "Every triangle",
            WireEdges::Model => "Model edges and outline",
        }
    }
}

/// What a wireframe does with the edges behind the model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HiddenLines {
    /// Drawn faint, so the structure reads through itself, as the viewport
    /// draws them.
    #[default]
    Faint,
    /// Left out: a hidden-line drawing.
    Hidden,
    /// Drawn as strongly as the ones in front.
    Shown,
}

impl HiddenLines {
    pub const ALL: [HiddenLines; 3] = [HiddenLines::Faint, HiddenLines::Hidden, HiddenLines::Shown];

    pub fn key(self) -> &'static str {
        match self {
            HiddenLines::Faint => "faint",
            HiddenLines::Hidden => "hidden",
            HiddenLines::Shown => "shown",
        }
    }

    pub fn from_key(key: &str) -> Option<HiddenLines> {
        HiddenLines::ALL.into_iter().find(|hidden| hidden.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            HiddenLines::Faint => "Faint",
            HiddenLines::Hidden => "Hidden",
            HiddenLines::Shown => "Shown",
        }
    }

    /// How strongly a hidden edge is drawn, against one in front at 1.
    pub fn strength(self) -> f32 {
        match self {
            // The viewport's, a touch firmer: a print loses what a screen
            // shows of a faint line.
            HiddenLines::Faint => 0.2,
            HiddenLines::Hidden => 0.0,
            HiddenLines::Shown => 1.0,
        }
    }
}

/// How a wireframe render looks. Colours are as they appear in the file,
/// sRGB from 0 to 1: a drawing is not lit, so nothing develops them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WireframeSettings {
    pub edges: WireEdges,
    pub hidden: HiddenLines,
    /// In pixels of the image, so a 4K drawing's lines are as thick as a
    /// 1080p one's unless asked otherwise.
    pub line_width: f32,
    pub line_colour: [f32; 3],
    /// The background, from the top of the image to the bottom; a file with
    /// alpha (the render's alpha mode) leaves it out.
    pub background_top: [f32; 3],
    pub background_bottom: [f32; 3],
}

impl Default for WireframeSettings {
    fn default() -> Self {
        WireframeSettings::DARK
    }
}

impl WireframeSettings {
    /// The viewport's: light lines over a dark ground.
    pub const DARK: WireframeSettings = WireframeSettings {
        edges: WireEdges::Mesh,
        hidden: HiddenLines::Faint,
        line_width: 1.5,
        line_colour: [0.82, 0.82, 0.84],
        background_top: [0.2, 0.2, 0.21],
        background_bottom: [0.11, 0.11, 0.115],
    };

    /// A drawing on paper: dark lines on white.
    pub const LIGHT: WireframeSettings = WireframeSettings {
        edges: WireEdges::Mesh,
        hidden: HiddenLines::Faint,
        line_width: 1.5,
        line_colour: [0.1, 0.1, 0.11],
        background_top: [1.0, 1.0, 1.0],
        background_bottom: [1.0, 1.0, 1.0],
    };
}

/// Everything a render is told before it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderSettings {
    /// What renders the image: Tequila's own path tracer, or Cycles.
    pub engine: RenderEngine,
    /// The picture, or a wireframe drawing of the model.
    pub kind: RenderKind,
    /// How a wireframe looks, kept for the next one.
    pub wireframe: WireframeSettings,
    /// Whether Tequila's renderer traces wavelengths or RGB (`docs/01`
    /// §7.4): by default spectral only where something disperses light.
    /// Cycles renders RGB whatever this says.
    pub spectral: tequila_render_core::settings::SpectralMode,
    pub width: usize,
    pub height: usize,
    /// Dots per inch, for the print-size calculator. Carried even though the
    /// renderer never reads it: a print size is a property of the *job*, and a
    /// user who set 300 DPI for a client should find it there tomorrow.
    pub dpi: f32,

    pub max_samples: u32,
    /// A wall-clock limit, or none. Whichever limit is reached first stops the
    /// render.
    pub max_seconds: Option<u32>,
    pub adaptive: bool,

    pub denoise: bool,
    pub denoise_strength: f32,
    /// How many passes between denoises while a render is in progress. One
    /// means every pass, which looks smooth and costs real time on a large
    /// image.
    pub denoise_cadence: u32,
    /// Pre-filter the albedo and normal guides before denoising. Worth it at
    /// low sample counts, where the guides are themselves noisy, and wasted
    /// once they are clean.
    pub prefilter_guides: bool,

    /// Outlier rejection (§3.5).
    pub reject_fireflies: bool,
    /// A hard ceiling on a single sample's luminance, or none.
    ///
    /// Separate from rejection because it is a different trade: rejection is
    /// adaptive and keeps the estimator nearly unbiased, while a clamp is blunt
    /// and removes energy the Clamped Energy pass then has to account for. §3.5
    /// allows it only as an explicit choice, which is what `Option` says here.
    pub clamp_indirect: Option<f32>,
    /// A ceiling on direct light of its own (Cycles), 0 for none. Unset, the
    /// clamp above holds for direct light too, as it always did; set, the
    /// two are apart, as Blender's are: a studio that clamps bounced light at
    /// 10 and leaves direct light alone keeps its brightest highlights.
    pub clamp_direct: Option<f32>,
    /// How each pixel's samples are spread. Unset is one pixel's box, what
    /// both renderers always used; Blender's is Blackman-Harris, 1.5 px.
    pub pixel_filter: Option<tequila_render_core::settings::PixelFilter>,
    /// The noise below which a pixel stops taking samples, for Cycles.
    /// Unset is Cycles' own, 0.01.
    pub adaptive_threshold: Option<f32>,

    /// Render only this rectangle (§9.5).
    pub region: Option<Region>,

    pub format: OutputFormat,
    pub depth: BitDepth,
    pub alpha_mode: AlphaMode,
    pub jpeg_quality: u8,
    /// Write the render passes alongside the beauty image.
    pub write_passes: bool,

    pub output_directory: String,
    pub filename_template: String,
    pub write_xmp: bool,
    pub write_text_metadata: bool,
    /// Render layers (`TEQUILA_SPEC.md` §1.10): each enabled one is
    /// written as an image of its own beside the main one.
    pub layers: Vec<RenderLayer>,
    /// Keep an image per light group (§5.5) so the render can be relit
    /// afterwards — each light layer, the environment and glowing materials
    /// turned up, down or tinted without rendering again. Off by default:
    /// it holds a full-size image per group in memory and denoises each.
    pub relight: bool,
}

/// Parts rendered as an image of their own, to composite.
///
/// The layer's image holds only its members. Everything else stays in the
/// scene for every ray but the camera's — casting shadows, bouncing light,
/// showing in reflections — so the layers go back together without holes
/// where one stood in front of another. The ground and its shadows stay in
/// the main image.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderLayer {
    pub name: String,
    /// Its members; a member's descendants belong with it.
    pub nodes: Vec<crate::tree::NodeId>,
    pub enabled: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        RenderSettings {
            engine: RenderEngine::default(),
            kind: RenderKind::Photo,
            wireframe: WireframeSettings::default(),
            spectral: tequila_render_core::settings::SpectralMode::Auto,
            width: 1920,
            height: 1080,
            dpi: 300.0,
            max_samples: 256,
            max_seconds: None,
            adaptive: true,
            denoise: true,
            denoise_strength: 1.0,
            denoise_cadence: 4,
            prefilter_guides: true,
            reject_fireflies: true,
            clamp_indirect: None,
            clamp_direct: None,
            pixel_filter: None,
            adaptive_threshold: None,
            region: None,
            format: OutputFormat::Png,
            depth: BitDepth::Eight,
            alpha_mode: AlphaMode::None,
            jpeg_quality: 92,
            write_passes: false,
            output_directory: String::new(),
            filename_template: "{project}_{camera}_{date}".into(),
            write_xmp: false,
            write_text_metadata: false,
            layers: Vec::new(),
            relight: false,
        }
    }
}

/// Why a render stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    Samples,
    Time,
    /// Every pixel retired under the adaptive threshold.
    Converged,
}

impl RenderSettings {
    /// Whether a render should stop, and why.
    ///
    /// One place, so the viewport, the queue and the command line cannot
    /// disagree about when a job is done — which they would, given three
    /// separate comparisons of a sample count against a limit.
    pub fn stop_after(&self, samples: u32, elapsed: Duration, converged: bool) -> Option<Halt> {
        if samples >= self.max_samples {
            return Some(Halt::Samples);
        }
        if let Some(limit) = self.max_seconds {
            if elapsed.as_secs_f64() >= limit as f64 {
                return Some(Halt::Time);
            }
        }
        // Only when adaptive sampling is what is doing the converging. With it
        // off, "every pixel is done" is a claim nothing measured.
        if converged && self.adaptive {
            return Some(Halt::Converged);
        }
        None
    }

    /// The depth this format will actually be written at.
    ///
    /// A setting that cannot be honoured is corrected rather than obeyed: a
    /// document that asks for a 32-bit PNG was saved by a version that offered
    /// it or edited by hand, and writing eight bits is better than refusing.
    pub fn effective_depth(&self) -> BitDepth {
        let allowed = self.format.depths();
        if allowed.contains(&self.depth) {
            self.depth
        } else {
            allowed[0]
        }
    }

    /// Whether the passes will actually be written.
    pub fn effective_write_passes(&self) -> bool {
        self.write_passes && self.format.carries_passes()
    }

    /// Pixels in the region, or in the whole image.
    pub fn rendered_pixels(&self) -> usize {
        match self.region {
            Some(region) => region.width * region.height,
            None => self.width * self.height,
        }
    }
}

// ---------------------------------------------------------------------------
// The queue (§04 §11)
// ---------------------------------------------------------------------------

/// Where a job has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JobState {
    #[default]
    Queued,
    Rendering,
    Done,
    Failed,
    /// Taken out of the running without being deleted, so a user can park a job
    /// rather than lose its settings.
    Held,
}

impl JobState {
    pub fn label(self) -> &'static str {
        match self {
            JobState::Queued => "Queued",
            JobState::Rendering => "Rendering",
            JobState::Done => "Done",
            JobState::Failed => "Failed",
            JobState::Held => "Held",
        }
    }

    pub fn is_finished(self) -> bool {
        matches!(self, JobState::Done | JobState::Failed)
    }
}

/// One queued render.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderJob {
    /// Stable for the life of the queue, so a job that is reordered keeps its
    /// identity and its finished image.
    pub id: u64,
    pub name: String,
    /// Which saved camera to render from.
    pub camera: usize,
    pub settings: RenderSettings,
    pub state: JobState,
    /// Where the finished image went, once it has.
    pub output: Option<String>,
    /// What went wrong, for a job that failed.
    pub message: String,
    pub seconds: f64,
    pub samples: u32,
}

/// The queue, which persists (§04 §11).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderQueue {
    jobs: Vec<RenderJob>,
    next_id: u64,
}

impl RenderQueue {
    pub fn new() -> RenderQueue {
        RenderQueue {
            jobs: Vec::new(),
            next_id: 1,
        }
    }

    pub fn jobs(&self) -> &[RenderJob] {
        &self.jobs
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Restore a queue that was saved, keeping the identifiers it had.
    pub fn from_jobs(jobs: Vec<RenderJob>, next_id: u64) -> RenderQueue {
        let highest = jobs.iter().map(|j| j.id).max().unwrap_or(0);
        RenderQueue {
            jobs,
            // Never below what is already in use: a restored queue that reissued
            // an identifier would make two jobs the same job.
            next_id: next_id.max(highest + 1).max(1),
        }
    }

    pub fn push(&mut self, name: impl Into<String>, camera: usize, settings: RenderSettings) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.push(RenderJob {
            id,
            name: name.into(),
            camera,
            settings,
            state: JobState::Queued,
            output: None,
            message: String::new(),
            seconds: 0.0,
            samples: 0,
        });
        id
    }

    pub fn get(&self, id: u64) -> Option<&RenderJob> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut RenderJob> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    pub fn remove(&mut self, id: u64) {
        self.jobs.retain(|j| j.id != id);
    }

    pub fn clear_finished(&mut self) {
        self.jobs.retain(|j| !j.state.is_finished());
    }

    /// Move a job by one place, for reordering by hand.
    pub fn move_by(&mut self, id: u64, delta: i32) {
        let Some(from) = self.jobs.iter().position(|j| j.id == id) else {
            return;
        };
        let to = (from as i32 + delta).clamp(0, self.jobs.len() as i32 - 1) as usize;
        if to == from {
            return;
        }
        let job = self.jobs.remove(from);
        self.jobs.insert(to, job);
    }

    /// The next job to run: the first that is queued.
    ///
    /// In list order rather than by identifier, which is what makes dragging a
    /// job up the list mean what it looks like it means.
    pub fn next_queued(&self) -> Option<u64> {
        self.jobs
            .iter()
            .find(|j| j.state == JobState::Queued)
            .map(|j| j.id)
    }

    /// How many are waiting, running and finished.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut waiting = 0;
        let mut running = 0;
        let mut finished = 0;
        for job in &self.jobs {
            match job.state {
                JobState::Queued => waiting += 1,
                JobState::Rendering => running += 1,
                JobState::Done | JobState::Failed => finished += 1,
                JobState::Held => {}
            }
        }
        (waiting, running, finished)
    }
}

/// How long a render with `settings` should take, in seconds, judged from
/// renders that have finished: their seconds per sample per pixel, times
/// this one's samples and pixels, held to its time limit. With nothing
/// finished to go by, its time limit alone, or `None`.
pub fn estimate_seconds(settings: &RenderSettings, finished: &[&RenderJob]) -> Option<f64> {
    let work = |s: &RenderSettings, samples: u32| samples as f64 * (s.width * s.height) as f64;
    let (seconds, done) = finished
        .iter()
        .filter(|j| j.state == JobState::Done && j.samples > 0 && j.seconds > 0.0)
        .fold((0.0, 0.0), |(t, w), j| (t + j.seconds, w + work(&j.settings, j.samples)));
    let limit = settings.max_seconds.map(f64::from);
    if done > 0.0 {
        let guess = seconds / done * work(settings, settings.max_samples);
        return Some(limit.map_or(guess, |l| guess.min(l)));
    }
    limit
}

impl RenderQueue {
    /// Seconds until everything queued is done: each waiting job by
    /// [`estimate_seconds`], the one rendering by its own progress. `None`
    /// while there is nothing to judge a waiting job by.
    pub fn remaining_seconds(&self) -> Option<f64> {
        let finished: Vec<&RenderJob> = self.jobs.iter().filter(|j| j.state == JobState::Done).collect();
        let mut total = 0.0;
        for job in &self.jobs {
            match job.state {
                JobState::Queued => total += estimate_seconds(&job.settings, &finished)?,
                JobState::Rendering if job.samples > 0 => {
                    let left = job.seconds / job.samples as f64 * job.settings.max_samples.saturating_sub(job.samples) as f64;
                    total += job.settings.max_seconds.map_or(left, |l| left.min((l as f64 - job.seconds).max(0.0)));
                }
                JobState::Rendering => total += estimate_seconds(&job.settings, &finished)?,
                _ => {}
            }
        }
        Some(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A finished 100-sample render of a megapixel took 20 s: a 400-sample
    /// one of two megapixels should take 8 times as long, unless its time
    /// limit is shorter. Nothing finished: only a limit says anything.
    #[test]
    fn a_render_is_estimated_from_the_ones_before() {
        let settings = |w, h, samples, limit| RenderSettings { width: w, height: h, max_samples: samples, max_seconds: limit, ..RenderSettings::default() };
        let mut queue = RenderQueue::new();
        let done = queue.push("done", 0, settings(1000, 1000, 100, None));
        let job = queue.get_mut(done).unwrap();
        job.state = JobState::Done;
        job.seconds = 20.0;
        job.samples = 100;
        let finished: Vec<&RenderJob> = queue.jobs().iter().collect();
        assert_eq!(estimate_seconds(&settings(2000, 1000, 400, None), &finished), Some(160.0));
        assert_eq!(estimate_seconds(&settings(2000, 1000, 400, Some(60)), &finished), Some(60.0));
        assert_eq!(estimate_seconds(&settings(2000, 1000, 400, Some(60)), &[]), Some(60.0));
        assert_eq!(estimate_seconds(&settings(2000, 1000, 400, None), &[]), None);

        queue.push("next", 0, settings(1000, 1000, 100, None));
        queue.push("after", 0, settings(1000, 1000, 50, None));
        assert_eq!(queue.remaining_seconds(), Some(30.0));
    }

    /// A sample limit and a time limit are not alternatives, and whichever is
    /// reached first has to stop the render. One place decides, so the
    /// viewport, the queue and the command line cannot disagree.
    #[test]
    fn whichever_limit_comes_first_stops_the_render() {
        let settings = RenderSettings {
            max_samples: 100,
            max_seconds: Some(10),
            ..Default::default()
        };
        assert_eq!(settings.stop_after(50, Duration::from_secs(1), false), None);
        assert_eq!(
            settings.stop_after(100, Duration::from_secs(1), false),
            Some(Halt::Samples)
        );
        assert_eq!(
            settings.stop_after(50, Duration::from_secs(10), false),
            Some(Halt::Time)
        );
        assert_eq!(
            settings.stop_after(50, Duration::from_secs(2), true),
            Some(Halt::Converged)
        );
    }

    /// "Every pixel is converged" is a claim the adaptive sampler makes. With
    /// it switched off nothing measured that, so nothing should act on it.
    #[test]
    fn convergence_only_halts_when_something_measured_it() {
        let settings = RenderSettings {
            adaptive: false,
            max_samples: 1000,
            ..Default::default()
        };
        assert_eq!(settings.stop_after(10, Duration::from_secs(1), true), None);
    }

    /// A format that cannot hold what was asked for gets the nearest thing it
    /// can, rather than refusing. A document asking for a 32-bit PNG was hand
    /// edited or saved by a version that offered it.
    #[test]
    fn an_impossible_depth_is_corrected_rather_than_obeyed() {
        let png = RenderSettings {
            format: OutputFormat::Png,
            depth: BitDepth::ThirtyTwo,
            ..Default::default()
        };
        assert_eq!(png.effective_depth(), BitDepth::Eight);

        let exr = RenderSettings {
            format: OutputFormat::Exr,
            depth: BitDepth::Eight,
            ..Default::default()
        };
        assert_eq!(exr.effective_depth(), BitDepth::Sixteen);

        // And a depth that *is* offered survives untouched.
        let tiff = RenderSettings {
            format: OutputFormat::Tiff,
            depth: BitDepth::ThirtyTwo,
            ..Default::default()
        };
        assert_eq!(tiff.effective_depth(), BitDepth::ThirtyTwo);
    }

    /// Asking for passes in a format that cannot hold them is a setting that
    /// would otherwise fail silently at write time.
    /// A new render writes the picture as it looks, with no transparency:
    /// most renders go to a viewer, which shows a transparent background as
    /// its own white or black rather than the studio. And every mode round
    /// trips through its key.
    #[test]
    fn a_render_is_opaque_unless_asked_otherwise() {
        assert_eq!(RenderSettings::default().alpha_mode, AlphaMode::None);
        for mode in AlphaMode::ALL {
            assert_eq!(AlphaMode::from_key(mode.key()), Some(mode));
        }
    }

    #[test]
    fn passes_are_only_written_where_they_fit() {
        let png = RenderSettings {
            format: OutputFormat::Png,
            write_passes: true,
            ..Default::default()
        };
        assert!(!png.effective_write_passes());
        let exr = RenderSettings {
            format: OutputFormat::Exr,
            write_passes: true,
            ..Default::default()
        };
        assert!(exr.effective_write_passes());
    }

    #[test]
    fn a_region_decides_how_much_is_rendered() {
        let whole = RenderSettings::default();
        assert_eq!(whole.rendered_pixels(), 1920 * 1080);
        let part = RenderSettings {
            region: Some(Region {
                x: 100,
                y: 100,
                width: 320,
                height: 240,
            }),
            ..Default::default()
        };
        assert_eq!(part.rendered_pixels(), 320 * 240);
    }

    /// The queue runs in *list* order, which is what makes dragging a job up
    /// the list mean what it looks like it means. Running by identifier would
    /// make reordering a no-op that appears to work.
    #[test]
    fn the_queue_runs_in_list_order() {
        let mut queue = RenderQueue::new();
        let first = queue.push("First", 0, RenderSettings::default());
        let second = queue.push("Second", 0, RenderSettings::default());
        assert_eq!(queue.next_queued(), Some(first));

        queue.move_by(second, -1);
        assert_eq!(queue.next_queued(), Some(second));
        assert_eq!(queue.jobs()[0].name, "Second");

        // And moving past the end does nothing rather than panicking.
        queue.move_by(second, -5);
        assert_eq!(queue.jobs()[0].id, second);
        queue.move_by(second, 99);
        assert_eq!(queue.jobs()[1].id, second);
    }

    /// A restored queue must never reissue an identifier. Two jobs with the
    /// same one would share a finished image, which is the sort of thing that
    /// looks like the renderer overwrote a file.
    #[test]
    fn a_restored_queue_does_not_reissue_identifiers() {
        let mut queue = RenderQueue::new();
        queue.push("A", 0, RenderSettings::default());
        let second = queue.push("B", 0, RenderSettings::default());
        let jobs = queue.jobs().to_vec();

        // Restored with a watermark that is too low, as a hand-edited file
        // might carry.
        let mut restored = RenderQueue::from_jobs(jobs, 1);
        let third = restored.push("C", 0, RenderSettings::default());
        assert!(third > second, "{third} should be past {second}");
        assert_eq!(restored.len(), 3);
    }

    #[test]
    fn finished_jobs_can_be_cleared_and_held_ones_are_kept() {
        let mut queue = RenderQueue::new();
        let a = queue.push("A", 0, RenderSettings::default());
        let b = queue.push("B", 0, RenderSettings::default());
        let c = queue.push("C", 0, RenderSettings::default());
        queue.get_mut(a).unwrap().state = JobState::Done;
        queue.get_mut(b).unwrap().state = JobState::Held;
        queue.get_mut(c).unwrap().state = JobState::Failed;

        assert_eq!(queue.counts(), (0, 0, 2));
        queue.clear_finished();
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.jobs()[0].state, JobState::Held);
        // A held job is not queued, so nothing runs it by accident.
        assert_eq!(queue.next_queued(), None);
    }
}
