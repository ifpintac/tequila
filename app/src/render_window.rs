//! The Render window: settings, the image, the queue and past renders
//! (`docs/04-UX-SPEC.md` §11, `TEQUILA_SPEC.md` §1.8).
//!
//! # Layout
//!
//! The picture is the point, so it has most of the window. Settings sit in a
//! sidebar on the left, in sections that fold away; the image — live while a
//! render runs, finished afterwards, and a frame of the right shape before
//! either — fills the middle; the queue and the renders already made run
//! along the bottom as a filmstrip; and what is about to be written, with the
//! button that writes it, is pinned to the foot.
//!
//! It used to be four tabs, with the live preview on the third at 320 pixels
//! tall, so the question a person watches a render to answer — "is this
//! going to look right" — was the one the window made hardest to ask.
//!
//! # What this window does not do
//!
//! It does not render. It builds jobs and shows results; the rendering happens
//! on the batch thread. That separation is what lets a queue run while the
//! user carries on working, and it is why nothing in this file blocks.

use eframe::egui;
use tequila_format::output::{expand_template, ImageFormat, NameFields, PrintSize};
use tequila_scene::render::{
    AlphaMode, BitDepth, JobState, OutputFormat, Region, RenderJob, RenderSettings,
};
use tequila_scene::Command;
use std::path::{Path, PathBuf};

use crate::project::Project;

pub fn progress(job: &RenderJob) -> f32 {
    if job.state == JobState::Done { return 1.0; }
    let samples = job.samples as f64 / job.settings.max_samples.max(1) as f64;
    let timed = job.settings.max_seconds.filter(|v| *v > 0).map(|v| job.seconds / v as f64).unwrap_or(0.0);
    samples.max(timed).clamp(0.0, 0.99) as f32
}

pub fn duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    if seconds >= 3600 { format!("{}h {:02}m", seconds/3600, seconds/60%60) }
    else { format!("{}m {:02}s", seconds/60, seconds%60) }
}

/// Where a render is written.
///
/// An empty folder means *beside the project*, which is what the window has
/// always said it meant. The batch thread used to read it as "the directory
/// the application was started from", which is wherever a shortcut happened
/// to point — so renders landed in the source tree, or in System32, and the
/// window described a place they were not. The job and the window both ask
/// here now, so the path shown is the path written.
pub fn output_directory(settings: &RenderSettings, project: Option<&Path>) -> PathBuf {
    let typed = settings.output_directory.trim();
    if !typed.is_empty() {
        return PathBuf::from(typed);
    }
    project.and_then(Path::parent).filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(fallback_directory)
}

/// Where renders go when neither the settings nor a saved project say: the
/// working folder, unless that is the root of a disk - where a Mac app
/// opened from the Finder runs, and cannot write - and then the user's
/// `Pictures/Tequila`.
pub fn fallback_directory() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    choose_fallback(std::env::current_dir().ok(), home)
}

fn choose_fallback(current: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    match (current, home) {
        (Some(dir), _) if dir.parent().is_some() => dir,
        (_, Some(home)) => {
            let pictures = home.join("Pictures").join("Tequila");
            let _ = std::fs::create_dir_all(&pictures);
            pictures
        }
        (Some(dir), None) => dir,
        (None, None) => PathBuf::from("."),
    }
}

/// What `{project}` means in a filename.
///
/// The document's own name, unless it was never given one — then the file it
/// was saved as. A project opened from `quest3.tequila` whose document still
/// says "Untitled" was writing `Untitled_Camera 1.png`, a name that says
/// nothing and collides with every other unnamed project's renders.
pub fn project_name(document_name: &str, path: Option<&Path>) -> String {
    let named = document_name.trim();
    if !named.is_empty() && named != "Untitled" {
        return named.to_string();
    }
    path.and_then(Path::file_stem)
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| named.to_string())
}

/// A finished render, kept for comparison.
pub struct GalleryEntry {
    pub name: String,
    pub path: String,
    pub width: usize,
    pub height: usize,
    pub samples: u32,
    pub seconds: f64,
    /// When it finished, as local `YYYY-MM-DD HH:MM`.
    pub when: String,
    pub texture: Option<egui::TextureHandle>,
}

/// How many past renders are remembered between sessions, newest kept.
const REMEMBERED: usize = 100;

/// The longest side of a preview kept in memory. A 4K render held at full
/// size is 33 MB of texture, and a hundred of them would be the application's
/// whole memory for pictures nobody is looking at; this is still larger than
/// the stage ever shows them.
pub const PREVIEW_SIZE: usize = 1600;

/// Shrink an RGBA8 image so its longest side is at most `limit`, averaging
/// whole blocks so a shrunk render does not shimmer the way skipping pixels
/// would.
pub fn shrink(width: usize, height: usize, rgba: &[u8], limit: usize) -> (usize, usize, Vec<u8>) {
    let step = width.max(height).div_ceil(limit.max(1)).max(1);
    if step == 1 || rgba.len() < width * height * 4 {
        return (width, height, rgba.to_vec());
    }
    let (w, h) = (width.div_ceil(step), height.div_ceil(step));
    let mut out = Vec::with_capacity(w * h * 4);
    for by in 0..h {
        for bx in 0..w {
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for y in by * step..((by + 1) * step).min(height) {
                for x in bx * step..((bx + 1) * step).min(width) {
                    let i = (y * width + x) * 4;
                    for c in 0..4 {
                        sum[c] += rgba[i + c] as u32;
                    }
                    count += 1;
                }
            }
            out.extend(sum.iter().map(|s| (s / count.max(1)) as u8));
        }
    }
    (w, h, out)
}

/// A render's preview, sized for keeping.
pub fn preview_image(width: usize, height: usize, rgba: &[u8]) -> egui::ColorImage {
    let (w, h, pixels) = shrink(width, height, rgba, PREVIEW_SIZE);
    egui::ColorImage::from_rgba_unmultiplied([w, h], &pixels)
}

/// Read a render back from disk for its preview.
///
/// PNG through the `png` crate directly, so the pixels are exactly the ones
/// written. JPEG, TIFF, PSD and EXR through the texture loader, which
/// returns linear light, re-encoded to sRGB — exact for the eight-bit ones,
/// and for an EXR or a 32-bit TIFF a straight clamp that shows the render
/// roughly as the viewport would, not graded.
fn load_preview(path: &str) -> Option<egui::ColorImage> {
    let lower = path.to_ascii_lowercase();
    let (width, height, rgba) = if lower.ends_with(".png") {
        let mut decoder = png::Decoder::new(std::fs::File::open(path).ok()?);
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder.read_info().ok()?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buffer).ok()?;
        let bytes = &buffer[..info.buffer_size()];
        let rgba: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => bytes.to_vec(),
            png::ColorType::Rgb => bytes.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
            png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => return None,
        };
        (info.width as usize, info.height as usize, rgba)
    } else if tequila_texture::image::Image::is_decodable(path) {
        let image = tequila_texture::image::Image::load(path).ok()?;
        let encode = |v: f32| {
            let v = v.clamp(0.0, 1.0);
            let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
            (s * 255.0 + 0.5) as u8
        };
        let channels = image.channels.max(1);
        let mut rgba = Vec::with_capacity(image.width * image.height * 4);
        for texel in image.data.chunks_exact(channels) {
            let (r, g, b) = match channels {
                1 | 2 => (texel[0], texel[0], texel[0]),
                _ => (texel[0], texel[1], texel[2]),
            };
            let a = match channels {
                2 => texel[1],
                4 => texel[3],
                _ => 1.0,
            };
            rgba.extend([encode(r), encode(g), encode(b), (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
        }
        (image.width, image.height, rgba)
    } else {
        return None;
    };
    Some(preview_image(width, height, &rgba))
}

/// Sizes people actually ask for, by the name they ask for them by.
const SIZES: &[(&str, usize, usize)] = &[
    ("HD 1080p", 1920, 1080),
    ("4K UHD", 3840, 2160),
    ("Square", 2048, 2048),
    ("Portrait 4:5", 1600, 2000),
    ("Story 9:16", 1080, 1920),
    ("Web banner 3:1", 2400, 800),
    ("A4 portrait, 300 DPI", 2480, 3508),
    ("A4 landscape, 300 DPI", 3508, 2480),
];

/// Sample budgets by what they are for. The slider is still there for
/// anything between; these are the four a person reaches for without having
/// to know what a sample is.
const QUALITY: &[(&str, u32, &str)] = &[
    ("Draft", 32, "a quick look at framing and lighting"),
    ("Good", 256, "most product shots"),
    ("High", 1024, "glass, liquids, dark interiors"),
    ("Final", 4096, "print, and anything that must be clean"),
];

fn quality_name(samples: u32) -> Option<&'static str> {
    QUALITY.iter().find(|(_, spp, _)| *spp == samples).map(|(name, _, _)| *name)
}

/// The height that keeps a ratio when the width changes.
fn locked_height(new_width: usize, old_width: usize, old_height: usize) -> usize {
    if old_width == 0 {
        return old_height.max(1);
    }
    ((new_width as f64 * old_height as f64 / old_width as f64).round() as usize).max(1)
}

/// `16:9`, `4:5`, `1:1` — or a decimal when the ratio has no short name.
fn ratio_name(width: usize, height: usize) -> String {
    fn gcd(a: usize, b: usize) -> usize {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    let divisor = gcd(width, height).max(1);
    let (w, h) = (width / divisor, height / divisor);
    if w <= 32 && h <= 32 {
        format!("{w}:{h}")
    } else {
        format!("{:.2}:1", width as f64 / height.max(1) as f64)
    }
}

/// The largest rectangle of the given shape that fits, centred.
fn fit(bounds: egui::Rect, width: usize, height: usize) -> egui::Rect {
    if width == 0 || height == 0 || bounds.width() <= 0.0 || bounds.height() <= 0.0 {
        return bounds;
    }
    let aspect = width as f32 / height as f32;
    let size = if bounds.width() / bounds.height() > aspect {
        egui::vec2(bounds.height() * aspect, bounds.height())
    } else {
        egui::vec2(bounds.width(), bounds.width() / aspect)
    };
    egui::Rect::from_center_size(bounds.center(), size)
}

/// Open a file with whatever the system opens that kind of file with.
pub(crate) fn open_file(path: &str) {
    if path.is_empty() {
        return;
    }
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// Behind the picture, around it, and the gallery's cards, from the theme.
fn stage() -> egui::Color32 {
    let p = crate::theme::p();
    if p.dark { egui::Color32::from_gray(20) } else { p.chrome }
}
fn film() -> egui::Color32 {
    crate::theme::p().field
}
fn card() -> egui::Color32 {
    crate::theme::p().card
}
fn failed() -> egui::Color32 {
    crate::theme::p().danger
}

/// What the middle of the window is showing.
enum Showing {
    Compare(usize, usize),
    Live(u64),
    Image(usize),
    Nothing,
}

pub struct RenderWindow {
    pub open: bool,
    /// A render chosen from the filmstrip. While one is chosen the stage stays
    /// on it; otherwise it follows whatever is rendering, then the newest
    /// result. So a finished render appears by itself, but never pulls the
    /// picture away from someone who is looking at an older one.
    selected: Option<usize>,
    /// The other side of an A/B compare.
    compare_with: Option<usize>,
    /// Where the A/B wipe sits, 0 to 1.
    wipe: f32,
    pub gallery: Vec<GalleryEntry>,
    /// A name for the next job, so a queue of five is not five "Render"s.
    job_name: String,
    /// The print-size calculator's own inputs, which are *derived* from the
    /// resolution and so cannot simply be the settings.
    print_inches: Option<(f32, f32)>,
    /// Keep the width-to-height ratio when either is typed.
    lock_ratio: bool,
    /// Whether a job is in progress, so Stop appears only when there is
    /// something to stop.
    pub rendering: bool,
    /// Set by Stop and cleared by whoever acts on it. A flag rather than a
    /// direct call because the renderer is the application's, not this
    /// window's.
    pub cancel_requested: bool,
    /// The animation's frames to render, first and last, asked for this
    /// frame and queued by the application, which turns the product.
    pub frames_requested: Option<(u32, u32)>,
    /// The frame range shown, until the animation changes; `None`, all.
    frames: Option<(u32, u32)>,
    pub preview_job: Option<u64>,
    pub preview: Option<egui::TextureHandle>,
    /// Where past renders are remembered between sessions, once known.
    history_file: Option<PathBuf>,
    /// Previews being read back from disk, by path.
    loading: Vec<(String, std::thread::JoinHandle<Option<egui::ColorImage>>)>,
    /// Paths whose preview could not be read, so they are not tried every
    /// frame.
    unreadable: std::collections::HashSet<String>,
    /// The newest render's light groups, while it can still be relit.
    relight: Option<Relighting>,
    /// Render nodes, and what has been sent to them.
    network: crate::network_panel::NetworkPanel,
}

/// A render being relit: its light groups and what each is turned to.
pub struct Relighting {
    data: Box<crate::batch::Relight>,
    /// Per group: strength, and a tint.
    gains: Vec<(f32, [f32; 3])>,
}

impl Relighting {
    fn gains(&self) -> Vec<tequila_core::spectrum::Spectrum> {
        self.gains
            .iter()
            .map(|(strength, tint)| {
                tequila_core::spectrum::Spectrum {
                    r: tint[0],
                    g: tint[1],
                    b: tint[2],
                } * *strength
            })
            .collect()
    }
}

impl Default for RenderWindow {
    fn default() -> Self {
        RenderWindow {
            open: false,
            selected: None,
            compare_with: None,
            wipe: 0.5,
            gallery: Vec::new(),
            job_name: String::new(),
            print_inches: None,
            lock_ratio: false,
            rendering: false,
            cancel_requested: false,
            frames_requested: None,
            frames: None,
            preview_job: None,
            preview: None,
            history_file: None,
            loading: Vec::new(),
            unreadable: Default::default(),
            relight: None,
            network: Default::default(),
        }
    }
}

// ---- remembering renders between sessions ------------------------------------

fn history_to_json(entries: &[GalleryEntry]) -> String {
    use tequila_format::json::Value;
    use tequila_format::json_write::Object;
    let list = entries
        .iter()
        .map(|entry| {
            let mut out = Object::new();
            out.set("name", entry.name.clone())
                .set("path", entry.path.clone())
                .set("width", entry.width as f64)
                .set("height", entry.height as f64)
                .set("samples", entry.samples as f64)
                .set("seconds", entry.seconds)
                .set("when", entry.when.clone());
            out.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("renders", Value::Array(list));
    tequila_format::json_write::to_string(&out.build())
}

/// Past renders from the file, oldest first, without their pictures — and
/// without any whose file has since been deleted or moved, which would only
/// be a thumbnail of nothing.
fn history_from_json(text: &str) -> Vec<GalleryEntry> {
    let Ok(value) = tequila_format::json::parse(text.as_bytes()) else {
        return Vec::new();
    };
    let Some(list) = value.get("renders") else {
        return Vec::new();
    };
    list.as_array()
        .iter()
        .filter_map(|entry| {
            let text = |key: &str| entry.get(key).and_then(|v| v.as_str()).map(str::to_string);
            let number = |key: &str| entry.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let path = text("path")?;
            Some(GalleryEntry {
                name: text("name").unwrap_or_else(|| "Render".into()),
                width: number("width") as usize,
                height: number("height") as usize,
                samples: number("samples") as u32,
                seconds: number("seconds"),
                when: text("when").unwrap_or_default(),
                texture: None,
                path,
            })
        })
        .collect()
}

impl RenderWindow {
    /// Remember renders in `directory`, and bring back the ones remembered
    /// there before. Renders made this session so far stay, after them.
    pub fn remember_in(&mut self, directory: &Path) {
        let file = directory.join("gallery.json");
        let mut past: Vec<GalleryEntry> = std::fs::read_to_string(&file)
            .map(|text| history_from_json(&text))
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| Path::new(&entry.path).exists())
            .collect();
        past.append(&mut self.gallery);
        self.gallery = past;
        self.history_file = Some(file);
        self.trim_history();
    }

    /// Take in finished queue jobs the history does not have yet — renders
    /// from before it was kept, or from a session that crashed before it
    /// could write it down. Placed before this session's renders, oldest
    /// job first.
    pub fn remember_finished(&mut self, jobs: &[RenderJob]) {
        let mut found: Vec<GalleryEntry> = jobs
            .iter()
            .filter(|job| job.state == JobState::Done)
            .filter_map(|job| {
                let path = job.output.clone()?;
                let known = self.gallery.iter().any(|e| e.path == path);
                (!known && Path::new(&path).exists()).then(|| GalleryEntry {
                    name: job.name.clone(),
                    width: job.settings.width,
                    height: job.settings.height,
                    samples: job.samples,
                    seconds: job.seconds,
                    when: String::new(),
                    texture: None,
                    path,
                })
            })
            .collect();
        if found.is_empty() {
            return;
        }
        found.append(&mut self.gallery);
        self.gallery = found;
        self.trim_history();
        self.save_history();
    }

    /// Add a finished render, and remember it.
    pub fn add(&mut self, entry: GalleryEntry) {
        self.gallery.push(entry);
        self.trim_history();
        self.save_history();
    }

    fn trim_history(&mut self) {
        if self.gallery.len() > REMEMBERED {
            let excess = self.gallery.len() - REMEMBERED;
            self.gallery.drain(..excess);
            self.selected = None;
            self.compare_with = None;
        }
    }

    fn save_history(&self) {
        let Some(file) = &self.history_file else { return };
        // Written beside and renamed over, so a crash mid-write cannot cost
        // the whole list.
        let temporary = file.with_extension("json.tmp");
        if std::fs::write(&temporary, history_to_json(&self.gallery)).is_ok() {
            let _ = std::fs::rename(&temporary, file);
        }
    }

    /// Read back the previews of remembered renders, a couple at a time, on
    /// threads of their own so a folder of 4K renders does not stall the UI.
    fn load_previews(&mut self, ctx: &egui::Context) {
        let mut done = Vec::new();
        let mut still = Vec::new();
        for (path, handle) in std::mem::take(&mut self.loading) {
            if handle.is_finished() {
                done.push((path, handle));
            } else {
                still.push((path, handle));
            }
        }
        self.loading = still;
        for (path, handle) in done {
            match handle.join().ok().flatten() {
                Some(image) => {
                    let texture = ctx.load_texture(format!("render-history-{path}"), image, egui::TextureOptions::LINEAR);
                    for entry in self.gallery.iter_mut().filter(|e| e.path == path) {
                        entry.texture = Some(texture.clone());
                    }
                }
                None => {
                    self.unreadable.insert(path);
                }
            }
        }

        const AT_ONCE: usize = 2;
        let wanted: Vec<String> = self
            .gallery
            .iter()
            .rev()
            .filter(|e| e.texture.is_none())
            .map(|e| e.path.clone())
            .filter(|p| !self.unreadable.contains(p) && !self.loading.iter().any(|(l, _)| l == p))
            .take(AT_ONCE.saturating_sub(self.loading.len()))
            .collect();
        for path in wanted {
            let reading = path.clone();
            if let Ok(handle) = std::thread::Builder::new()
                .name("render-preview".into())
                .spawn(move || load_preview(&reading))
            {
                self.loading.push((path, handle));
            }
        }
        if !self.loading.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

impl RenderWindow {
    /// Open on the render in progress.
    pub fn show_progress(&mut self) {
        self.open = true;
        self.selected = None;
        self.compare_with = None;
    }

    pub fn set_preview(&mut self, ctx: &egui::Context, id: u64, width: usize, height: usize, rgba: &[u8]) {
        self.preview_job = Some(id);
        self.preview = Some(ctx.load_texture(format!("render-preview-{id}"),
            egui::ColorImage::from_rgba_unmultiplied([width, height], rgba), egui::TextureOptions::LINEAR));
    }

    pub fn show(&mut self, ctx: &egui::Context, project: &mut Project) {
        if !self.open {
            return;
        }
        self.load_previews(ctx);
        let mut open = self.open;
        let spec = crate::windows::Spec {
            kind: crate::windows::Kind::Render,
            title: "Render".into(),
            size: egui::vec2(1180.0, 760.0),
            min_size: egui::vec2(760.0, 480.0),
            resizable: true,
            margin: 12.0,
        };
        crate::windows::show(ctx, spec, &mut open, |window| window.default_size([1180.0, 760.0]).min_width(760.0).min_height(480.0).resizable(true), |ui| {
                let before = project.render_settings().clone();
                let mut settings = before.clone();

                // Declared outermost first, so each claims its edge before the
                // stage takes what is left.
                egui::TopBottomPanel::bottom("render-footer")
                    .frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(4.0, 8.0)))
                    .show_inside(ui, |ui| self.footer(ui, project, &settings));

                egui::SidePanel::left("render-settings")
                    .resizable(true)
                    .default_width(330.0)
                    .width_range(280.0..=480.0)
                    .frame(egui::Frame::none().inner_margin(egui::Margin { left: 2.0, right: 10.0, top: 2.0, bottom: 2.0 }))
                    .show_inside(ui, |ui| self.settings_panel(ui, &mut settings, project));

                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(stage()))
                    .show_inside(ui, |ui| {
                        egui::TopBottomPanel::bottom("render-filmstrip")
                            .exact_height(156.0)
                            .frame(egui::Frame::none().fill(film()).inner_margin(8.0))
                            .show_inside(ui, |ui| self.filmstrip(ui, project));
                        let showing = self.showing(project);
                        egui::TopBottomPanel::top("render-stage-bar")
                            .frame(egui::Frame::none().fill(film()).inner_margin(egui::Margin::symmetric(10.0, 7.0)))
                            .show_inside(ui, |ui| self.stage_bar(ui, project, &showing));
                        egui::CentralPanel::default()
                            .frame(egui::Frame::none().fill(stage()).inner_margin(16.0))
                            .show_inside(ui, |ui| self.stage(ui, project, &settings, &showing));
                    });

                if settings != before {
                    let _ = project.apply(Command::SetRenderSettings {
                        settings: Box::new(settings),
                    });
                }
            });
        self.open = open;
    }

    // ---- what is showing ---------------------------------------------------

    fn showing(&self, project: &Project) -> Showing {
        if let (Some(a), Some(b)) = (self.selected, self.compare_with) {
            if a < self.gallery.len() && b < self.gallery.len() {
                return Showing::Compare(a, b);
            }
        }
        if let Some(index) = self.selected.filter(|i| *i < self.gallery.len()) {
            return Showing::Image(index);
        }
        if let Some(job) = project.queue().jobs().iter().find(|j| j.state == JobState::Rendering) {
            return Showing::Live(job.id);
        }
        match self.gallery.len() {
            0 => Showing::Nothing,
            n => Showing::Image(n - 1),
        }
    }

    // ---- the foot: what will be written, and the button that writes it ----

    /// A render dialog's failure mode is that a person presses the button
    /// without knowing what they asked for — the wrong size, the wrong folder,
    /// a region still on from an hour ago. So the foot spells out the full
    /// path of the file this will write, resolved, not the template.
    fn footer(&mut self, ui: &mut egui::Ui, project: &mut Project, settings: &RenderSettings) {
        let path = self.planned_path(project, settings);
        let actions_width = if self.rendering { 470.0 } else { 330.0 };
        ui.horizontal(|ui| {
            let left = (ui.available_width() - actions_width).max(160.0);
            ui.allocate_ui(egui::vec2(left, 40.0), |ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(egui::RichText::new(format!("{} × {}", settings.width, settings.height)).strong());
                        let dot = || egui::RichText::new("·").color(crate::theme::dim());
                        ui.label(dot());
                        ui.label(settings.format.label());
                        ui.label(dot());
                        let quality = match quality_name(settings.max_samples) {
                            Some(name) => format!("{name}, {} spp", settings.max_samples),
                            None => format!("{} spp", settings.max_samples),
                        };
                        ui.label(quality);
                        if let Some(seconds) = settings.max_seconds {
                            ui.label(dot());
                            ui.label(format!("max {}", duration(seconds as f64)));
                        }
                        if settings.denoise {
                            ui.label(dot());
                            ui.label("denoised");
                        }
                        if settings.region.is_some() {
                            // Loud, because a region left on is the single most
                            // common way to get a render that is not what was
                            // wanted.
                            ui.label(dot());
                            ui.colored_label(crate::theme::p().accent, "region only");
                        }
                    });
                    let shown = path.display().to_string();
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!("Saves to  {shown}")).small().monospace().color(crate::theme::dim()),
                        )
                        .truncate(),
                    )
                    .on_hover_text(shown);
                });
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if self.rendering { "Add to queue" } else { "Render" };
                let go = ui
                    .add(
                        egui::Button::new(egui::RichText::new(label).font(crate::theme::strong(14.0)).color(crate::theme::p().on_accent))
                            .fill(crate::theme::p().accent)
                            .min_size(egui::vec2(132.0, 34.0)),
                    )
                    .on_hover_text(if self.rendering {
                        "Renders after the current job. You can keep working meanwhile."
                    } else {
                        "Renders in the background. You can keep working meanwhile."
                    });
                if go.clicked() {
                    let name = if self.job_name.trim().is_empty() {
                        project.camera().name.clone()
                    } else {
                        self.job_name.trim().to_string()
                    };
                    let camera = project.document().cameras.active_index();
                    // Finished jobs live on as thumbnails in the filmstrip; in
                    // the queue they are only clutter, and a button to clear
                    // them was one more thing to understand. Tidied here, where
                    // the queue is next written anyway.
                    project.queue_mut().clear_finished();
                    project.enqueue(name, camera, settings.clone());
                    self.job_name.clear();
                    // Back to following the live render: the person just asked
                    // for it, so it is what they want to see.
                    self.selected = None;
                    self.compare_with = None;
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.job_name)
                        .desired_width(140.0)
                        .hint_text(project.camera().name.clone()),
                )
                .on_hover_text("A name for this render in the queue. Defaults to the camera's name.");
                ui.label(egui::RichText::new("Name").color(crate::theme::dim()));
                // A turntable's frames, numbered by {frame} in the file name;
                // added right to left, so they read "Frames 0 to 35".
                if let Some(animation) = project.document().animation.as_ref() {
                    let last = animation.frames.saturating_sub(1);
                    let (mut first, mut final_frame) = self.frames.unwrap_or((0, last));
                    ui.separator();
                    if ui
                        .button("Render frames")
                        .on_hover_text("Queue these frames of the turntable, one image each, numbered by {frame} in the file name. A frame already rendered is left out, so a stopped turntable is finished by asking again.")
                        .clicked()
                    {
                        self.frames_requested = Some((first.min(last), final_frame.min(last).max(first.min(last))));
                    }
                    ui.add(egui::DragValue::new(&mut final_frame).range(0..=last));
                    ui.label(egui::RichText::new("to").color(crate::theme::dim()));
                    ui.add(egui::DragValue::new(&mut first).range(0..=last));
                    ui.label(egui::RichText::new("Frames").color(crate::theme::dim()));
                    self.frames = Some((first.min(last), final_frame.min(last)));
                }
                if self.rendering {
                    // Stop, not abandon: the job writes whatever it has reached.
                    if ui.button("Stop & save").on_hover_text("Stop the current render and save the image as it is now").clicked() {
                        self.cancel_requested = true;
                    }
                }
            });
        });
    }

    fn planned_path(&self, project: &Project, settings: &RenderSettings) -> PathBuf {
        let name = expand_template(&settings.filename_template, &self.preview_fields(project, settings));
        // Through first_free too, so an existing file shows as the `_2` it
        // will become rather than as a name about to be overwritten.
        tequila_format::output::first_free(
            output_directory(settings, project.path.as_deref())
                .join(format!("{name}.{}", image_format(settings).extension())),
        )
    }

    // ---- the sidebar ---------------------------------------------------------

    /// Send renders to the render nodes, as the Network rendering section's
    /// buttons do: for the scripting channel's `queue` with `network`.
    pub fn send_to_network(&mut self, settings: &RenderSettings, project: &mut Project, every_configuration: bool) -> Result<usize, String> {
        self.network.send(settings, project, every_configuration)
    }

    /// Keep a finished render's light groups, to relight it. Replaces any
    /// earlier one: each holds a full-size image per group.
    pub fn set_relight(&mut self, data: Box<crate::batch::Relight>) {
        let gains = vec![(1.0, [1.0, 1.0, 1.0]); data.groups.len()];
        self.relight = Some(Relighting { data, gains });
    }

    /// Strength and tint per light group for the newest render, the preview
    /// recombined as they move, and a relit copy saved on request.
    fn relight_section(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        let Some(relighting) = &mut self.relight else {
            return;
        };
        ui.label(
            egui::RichText::new(format!(
                "Rebalance {} without rendering again.",
                std::path::Path::new(&relighting.data.path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ))
            .small()
            .color(crate::theme::dim()),
        );
        let mut changed = false;
        for ((name, _), (strength, tint)) in relighting.data.groups.iter().zip(relighting.gains.iter_mut()) {
            let label = match name.strip_prefix("LightLayer") {
                Some(layer) => format!("Light layer {layer}"),
                None => name.clone(),
            };
            ui.horizontal(|ui| {
                changed |= ui.color_edit_button_rgb(tint).changed();
                changed |= ui
                    .add(crate::field::Slider::new(strength, 0.0..=4.0).text(label))
                    .changed();
            });
        }
        ui.horizontal(|ui| {
            if ui.button("Reset").clicked() {
                for gain in &mut relighting.gains {
                    *gain = (1.0, [1.0, 1.0, 1.0]);
                }
                changed = true;
            }
            if ui.button("Save relit image").on_hover_text("Written beside the render as …_relit, full size.").clicked() {
                project.status = match relighting.data.save(&relighting.gains()) {
                    Ok(path) => format!("Saved {}", path.display()),
                    Err(error) => format!("Could not save the relit image: {error}"),
                };
            }
        });
        if changed {
            let output = relighting.data.compose(&relighting.gains(), 1280);
            let image = preview_image(output.width, output.height, &output.to_rgba8());
            let path = relighting.data.path.to_string_lossy().to_string();
            // The render's own gallery entry shows the relit version while it
            // is being adjusted; the file on disk is untouched until saved.
            let texture = ui.ctx().load_texture("relit-preview", image, egui::TextureOptions::LINEAR);
            if let Some(entry) = self.gallery.iter_mut().find(|e| e.path == path) {
                entry.texture = Some(texture.clone());
            }
            self.preview = Some(texture);
        }
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui, settings: &mut RenderSettings, project: &mut Project) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(4.0);
                if self.relight.is_some() {
                    section(ui, "Relight", true, |ui| self.relight_section(ui, project));
                }
                section(ui, "Image size", true, |ui| self.size_section(ui, settings));
                section(ui, "Quality", true, |ui| quality_section(ui, settings));
                section(ui, "Denoise", true, |ui| denoise_section(ui, settings));
                section(ui, "File", true, |ui| file_section(ui, settings, project));
                section(ui, "Region", settings.region.is_some(), |ui| region_section(ui, settings));
                section(ui, "Render layers", !settings.layers.is_empty(), |ui| layers_section(ui, settings, project));
                let nodes = self.network.has_nodes();
                section(ui, "Network rendering", nodes, |ui| self.network.ui(ui, settings, project));
                for entry in self.network.finished() {
                    self.add(entry);
                }
                ui.add_space(8.0);
            });
    }

    fn size_section(&mut self, ui: &mut egui::Ui, settings: &mut RenderSettings) {
        let current = SIZES
            .iter()
            .find(|(_, w, h)| *w == settings.width && *h == settings.height)
            .map(|(name, _, _)| *name)
            .unwrap_or("Custom");
        egui::ComboBox::from_id_salt("render-size-preset")
            .selected_text(current)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for (name, w, h) in SIZES {
                    if ui
                        .selectable_label(current == *name, format!("{name}   {w} × {h}"))
                        .clicked()
                    {
                        settings.width = *w;
                        settings.height = *h;
                        self.print_inches = None;
                    }
                }
            });

        ui.horizontal(|ui| {
            let (old_w, old_h) = (settings.width, settings.height);
            let mut width = settings.width as u32;
            let mut height = settings.height as u32;
            if ui.add(egui::DragValue::new(&mut width).range(16..=16384).prefix("W ").suffix(" px")).changed() {
                settings.width = width as usize;
                if self.lock_ratio {
                    settings.height = locked_height(settings.width, old_w, old_h).clamp(16, 16384);
                }
                self.print_inches = None;
            }
            ui.label("×");
            if ui.add(egui::DragValue::new(&mut height).range(16..=16384).prefix("H ").suffix(" px")).changed() {
                settings.height = height as usize;
                if self.lock_ratio {
                    settings.width = locked_height(settings.height, old_h, old_w).clamp(16, 16384);
                }
                self.print_inches = None;
            }
        });
        ui.horizontal(|ui| {
            ui.toggle_value(&mut self.lock_ratio, "Lock ratio")
                .on_hover_text("Keep the shape when you change the width or the height");
            if ui.button("Swap").on_hover_text("Turn landscape into portrait, and back").clicked() {
                std::mem::swap(&mut settings.width, &mut settings.height);
                self.print_inches = None;
            }
            ui.label(
                egui::RichText::new(format!(
                    "{}  ·  {:.1} MP",
                    ratio_name(settings.width, settings.height),
                    (settings.width * settings.height) as f64 / 1e6
                ))
                .small()
                .color(crate::theme::dim()),
            );
        });
        self.print_calculator(ui, settings);
    }

    /// §1.8's print-size calculator.
    fn print_calculator(&mut self, ui: &mut egui::Ui, settings: &mut RenderSettings) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Print").color(crate::theme::dim()));
            let size = PrintSize::from_pixels(settings.width, settings.height, settings.dpi);
            let (mut inches_w, mut inches_h) = self.print_inches.unwrap_or_else(|| size.inches());

            let mut changed = false;
            changed |= ui
                .add(egui::DragValue::new(&mut inches_w).speed(0.1).range(0.1..=400.0).suffix(" in"))
                .changed();
            ui.label("×");
            changed |= ui
                .add(egui::DragValue::new(&mut inches_h).speed(0.1).range(0.1..=400.0).suffix(" in"))
                .changed();
            let mut dpi = settings.dpi;
            let dpi_changed = ui
                .add(egui::DragValue::new(&mut dpi).range(1.0..=1200.0).suffix(" DPI"))
                .changed();
            settings.dpi = dpi;

            if changed || dpi_changed {
                // Typing a physical size sets the pixels, which is the whole
                // point of the calculator: a print shop asks for inches and a
                // density, and nobody wants to do the multiplication twice.
                self.print_inches = Some((inches_w, inches_h));
                let wanted = PrintSize::from_inches(inches_w, inches_h, settings.dpi);
                settings.width = wanted.pixels.0;
                settings.height = wanted.pixels.1;
            }
        });
        let (mm_w, mm_h) = PrintSize::from_pixels(settings.width, settings.height, settings.dpi).millimetres();
        ui.label(egui::RichText::new(format!("{mm_w:.0} × {mm_h:.0} mm at {:.0} DPI", settings.dpi)).small().color(crate::theme::dim()));
    }

    // ---- the stage ----------------------------------------------------------

    fn stage_bar(&mut self, ui: &mut egui::Ui, project: &Project, showing: &Showing) {
        match *showing {
            Showing::Live(id) => {
                let Some(job) = project.queue().get(id) else { return };
                let stage = if job.message.is_empty() { job.state.label() } else { job.message.as_str() };
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&job.name).strong());
                    ui.label(egui::RichText::new(stage).color(crate::theme::p().accent));
                    ui.label(egui::RichText::new(format!("{} / {} samples", job.samples, job.settings.max_samples)).color(crate::theme::dim()));
                    ui.label(egui::RichText::new(duration(job.seconds)).color(crate::theme::dim()));
                    if job.samples > 0 && stage == "Rendering" {
                        let left = job.seconds / job.samples as f64 * job.settings.max_samples.saturating_sub(job.samples) as f64;
                        let left = job.settings.max_seconds.map(|limit| left.min((limit as f64 - job.seconds).max(0.0))).unwrap_or(left);
                        ui.label(egui::RichText::new(format!("about {} left", duration(left))).color(crate::theme::dim()))
                            .on_hover_text("An estimate. Adaptive sampling and the final denoise can change it.");
                    }
                });
                ui.add(egui::ProgressBar::new(progress(job)).desired_height(6.0).animate(true));
            }
            Showing::Image(index) => {
                let Some(entry) = self.gallery.get(index) else { return };
                let path = entry.path.clone();
                let mut details = format!(
                    "{} × {}  ·  {} spp  ·  {}",
                    entry.width, entry.height, entry.samples, duration(entry.seconds)
                );
                if !entry.when.is_empty() {
                    details.push_str(&format!("  ·  {}", entry.when));
                }
                let name = self.label(index);
                ui.horizontal(|ui| {
                    // The buttons first, from the right; the name and details
                    // in what is left, cut short rather than run under them.
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.gallery.len() > 1 {
                            ui.menu_button("Compare with…", |ui| {
                                for other in (0..self.gallery.len()).rev() {
                                    if other != index && ui.button(self.described(other)).clicked() {
                                        self.selected = Some(index);
                                        self.compare_with = Some(other);
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                        if ui.button("Copy path").clicked() {
                            ui.ctx().copy_text(path.clone());
                        }
                        if ui.button("Show in folder").clicked() {
                            crate::library_tab::reveal(&path);
                        }
                        if ui.button("Open").on_hover_text("Open in your image viewer").clicked() {
                            open_file(&path);
                        }
                        if self.rendering && ui.button("Watch live render").clicked() {
                            self.selected = None;
                        }
                        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&name).strong()).truncate());
                            ui.add(egui::Label::new(egui::RichText::new(&details).color(crate::theme::dim())).truncate())
                                .on_hover_text(&details);
                        });
                    });
                });
            }
            Showing::Compare(a, b) => {
                if a >= self.gallery.len() || b >= self.gallery.len() { return }
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Comparing two renders").strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Done comparing").clicked() {
                            self.compare_with = None;
                        }
                        ui.label(egui::RichText::new("Drag across the image to move the split").small().color(crate::theme::dim()));
                    });
                });
            }
            Showing::Nothing => {
                ui.label(egui::RichText::new("Nothing rendered yet").strong());
            }
        }
    }

    fn stage(&mut self, ui: &mut egui::Ui, project: &Project, settings: &RenderSettings, showing: &Showing) {
        let area = ui.available_rect_before_wrap();
        ui.allocate_rect(area, egui::Sense::hover());
        match *showing {
            Showing::Live(id) => {
                let texture = self.preview.as_ref().filter(|_| self.preview_job == Some(id));
                match texture {
                    Some(texture) => {
                        let size = texture.size();
                        let rect = fit(area, size[0], size[1]);
                        ui.painter().image(texture.id(), rect, full_uv(), egui::Color32::WHITE);
                    }
                    None => {
                        let frame = fit(area, settings.width, settings.height);
                        ui.painter().rect_stroke(frame, 2.0, egui::Stroke::new(1.0_f32, crate::theme::p().line_strong));
                        let spinner = egui::Rect::from_center_size(frame.center() - egui::vec2(0.0, 14.0), egui::vec2(22.0, 22.0));
                        ui.put(spinner, egui::Spinner::new());
                        ui.painter().text(
                            frame.center() + egui::vec2(0.0, 16.0),
                            egui::Align2::CENTER_CENTER,
                            "Preparing the first preview…",
                            egui::FontId::proportional(14.0),
                            crate::theme::dim(),
                        );
                    }
                }
            }
            Showing::Image(index) => {
                if let Some(entry) = self.gallery.get(index) {
                    let rect = fit(area, entry.width, entry.height);
                    match &entry.texture {
                        Some(texture) => {
                            ui.painter().image(texture.id(), rect, full_uv(), egui::Color32::WHITE);
                        }
                        None => {
                            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, &entry.path, egui::FontId::proportional(13.0), crate::theme::dim());
                        }
                    }
                }
            }
            Showing::Compare(a, b) => self.draw_wipe(ui, area, a, b),
            Showing::Nothing => {
                // The frame the render will fill, at its real shape, so the
                // size settings can be judged before anything is spent on them.
                let frame = fit(area, settings.width, settings.height);
                let painter = ui.painter();
                painter.rect_filled(frame, 2.0, crate::theme::p().field);
                painter.rect_stroke(frame, 2.0, egui::Stroke::new(1.0_f32, crate::theme::p().line_strong));
                if let Some(region) = &settings.region {
                    let sx = frame.width() / settings.width.max(1) as f32;
                    let sy = frame.height() / settings.height.max(1) as f32;
                    let r = egui::Rect::from_min_size(
                        frame.min + egui::vec2(region.x as f32 * sx, region.y as f32 * sy),
                        egui::vec2(region.width as f32 * sx, region.height as f32 * sy),
                    );
                    painter.rect_stroke(r, 0.0, egui::Stroke::new(1.5_f32, crate::theme::p().accent));
                    painter.text(r.left_top() + egui::vec2(6.0, 4.0), egui::Align2::LEFT_TOP, "region", egui::FontId::proportional(12.0), crate::theme::p().accent);
                }
                painter.text(
                    frame.center() - egui::vec2(0.0, 12.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{} × {}", settings.width, settings.height),
                    egui::FontId::proportional(22.0),
                    crate::theme::p().text_dim,
                );
                painter.text(
                    frame.center() + egui::vec2(0.0, 16.0),
                    egui::Align2::CENTER_CENTER,
                    format!("{} — press Render to start", project.camera().name),
                    egui::FontId::proportional(13.0),
                    crate::theme::dim(),
                );
            }
        }
    }

    /// The A/B wipe: both images in the same rectangle, split at the wipe.
    ///
    /// Both at the *same* size and position, rather than side by side. A
    /// difference of a few per cent in a highlight is invisible when the two
    /// images are apart and obvious across a seam, and seeing that difference
    /// is the entire reason to keep old renders.
    fn draw_wipe(&mut self, ui: &mut egui::Ui, area: egui::Rect, a: usize, b: usize) {
        let (width, height) = match self.gallery.get(a) {
            Some(entry) => (entry.width, entry.height),
            None => return,
        };
        let rect = fit(area, width, height);
        let response = ui.interact(rect, ui.id().with("wipe"), egui::Sense::click_and_drag());
        if response.dragged() || response.clicked() {
            if let Some(pointer) = response.interact_pointer_pos() {
                self.wipe = ((pointer.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
            }
        }
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        let (Some(first), Some(second)) = (self.gallery.get(a), self.gallery.get(b)) else { return };
        let split = rect.left() + rect.width() * self.wipe;
        let painter = ui.painter_at(rect);
        if let Some(texture) = &first.texture {
            let left = egui::Rect::from_min_max(rect.min, egui::pos2(split, rect.max.y));
            painter.image(texture.id(), left, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(self.wipe, 1.0)), egui::Color32::WHITE);
        }
        if let Some(texture) = &second.texture {
            let right = egui::Rect::from_min_max(egui::pos2(split, rect.min.y), rect.max);
            painter.image(texture.id(), right, egui::Rect::from_min_max(egui::pos2(self.wipe, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        painter.line_segment(
            [egui::pos2(split, rect.top()), egui::pos2(split, rect.bottom())],
            egui::Stroke::new(2.0_f32, egui::Color32::from_white_alpha(200)),
        );
        let tag = |at: egui::Pos2, align, text: &str| {
            let galley = painter.layout_no_wrap(text.to_string(), egui::FontId::proportional(13.0), egui::Color32::WHITE);
            let bg = align_rect(at, galley.size(), align).expand(4.0);
            painter.rect_filled(bg, 3.0, egui::Color32::from_black_alpha(150));
            painter.galley(bg.min + egui::vec2(4.0, 4.0), galley, egui::Color32::WHITE);
        };
        tag(rect.left_top() + egui::vec2(10.0, 10.0), egui::Align2::LEFT_TOP, &format!("A  {}", self.described(a)));
        tag(rect.right_top() + egui::vec2(-10.0, 10.0), egui::Align2::RIGHT_TOP, &format!("B  {}", self.described(b)));
    }

    // ---- the filmstrip --------------------------------------------------------

    /// The queue, then every render made this session, newest first.
    fn filmstrip(&mut self, ui: &mut egui::Ui, project: &mut Project) {
        let (waiting, running, _) = project.queue().counts();
        ui.horizontal(|ui| {
            let mut words = Vec::new();
            if running > 0 { words.push("1 rendering".to_string()); }
            if waiting > 0 { words.push(format!("{waiting} waiting")); }
            words.push(match self.gallery.len() {
                1 => "1 render".to_string(),
                n => format!("{n} renders"),
            });
            ui.label(egui::RichText::new(words.join("  ·  ")).small().color(crate::theme::dim()));
        });
        ui.add_space(4.0);

        let jobs: Vec<RenderJob> = project
            .queue()
            .jobs()
            .iter()
            .filter(|j| j.state != JobState::Done)
            .cloned()
            .collect();
        if jobs.is_empty() && self.gallery.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("Renders you make appear here. Click one to view it, right-click for more.").color(crate::theme::dim()));
            });
            return;
        }

        let mut remove_job = None;
        let mut move_job = None;
        let mut hold_job = None;
        let mut remove_image = None;
        egui::ScrollArea::horizontal()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for job in &jobs {
                        let card = self.job_card(ui, job);
                        if card.clicked() && job.state == JobState::Rendering {
                            self.selected = None;
                            self.compare_with = None;
                        }
                        card.context_menu(|ui| {
                            if job.state != JobState::Rendering
                                && ui.button(if job.state == JobState::Held { "Resume" } else { "Hold" }).clicked()
                            {
                                hold_job = Some(job.id);
                                ui.close_menu();
                            }
                            if ui.button("Move earlier").clicked() {
                                move_job = Some((job.id, -1));
                                ui.close_menu();
                            }
                            if ui.button("Move later").clicked() {
                                move_job = Some((job.id, 1));
                                ui.close_menu();
                            }
                            if job.state != JobState::Rendering && ui.button("Remove from queue").clicked() {
                                remove_job = Some(job.id);
                                ui.close_menu();
                            }
                        });
                    }
                    for index in (0..self.gallery.len()).rev() {
                        let thumb = self.thumbnail(ui, index);
                        if thumb.clicked() {
                            if self.compare_with.is_some() && self.selected != Some(index) {
                                self.compare_with = Some(index);
                            } else {
                                self.selected = Some(index);
                            }
                        }
                        let path = self.gallery[index].path.clone();
                        thumb.on_hover_text(&path).context_menu(|ui| {
                            if ui.button("Open").clicked() {
                                open_file(&path);
                                ui.close_menu();
                            }
                            if ui.button("Show in folder").clicked() {
                                crate::library_tab::reveal(&path);
                                ui.close_menu();
                            }
                            if ui.button("Copy path").clicked() {
                                ui.ctx().copy_text(path.clone());
                                ui.close_menu();
                            }
                            let current = self.selected.unwrap_or(self.gallery.len().saturating_sub(1));
                            if current != index && ui.button("Compare with the one on screen").clicked() {
                                self.selected = Some(current);
                                self.compare_with = Some(index);
                                ui.close_menu();
                            }
                            ui.separator();
                            if ui.button("Remove from this list").on_hover_text("The file on disk is kept").clicked() {
                                remove_image = Some(index);
                                ui.close_menu();
                            }
                        });
                    }
                });
            });

        let mut changed = false;
        if let Some(id) = remove_job {
            project.queue_mut().remove(id);
            changed = true;
        }
        if let Some((id, delta)) = move_job {
            project.queue_mut().move_by(id, delta);
            changed = true;
        }
        if let Some(id) = hold_job {
            if let Some(job) = project.queue_mut().get_mut(id) {
                job.state = if job.state == JobState::Held { JobState::Queued } else { JobState::Held };
            }
            changed = true;
        }
        if changed {
            project.save_queue();
        }
        if let Some(index) = remove_image {
            self.gallery.remove(index);
            self.save_history();
            // Indices after it have shifted; rather than renumber a choice the
            // person made, drop it and go back to following the newest.
            self.selected = None;
            self.compare_with = None;
        }
    }

    fn job_card(&self, ui: &mut egui::Ui, job: &RenderJob) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(150.0, 92.0), egui::Sense::click());
        let painter = ui.painter_at(rect);
        let live = job.state == JobState::Rendering && self.selected.is_none();
        painter.rect_filled(rect, 4.0, card());
        if live || response.hovered() {
            painter.rect_stroke(rect, 4.0, egui::Stroke::new(if live { 2.0_f32 } else { 1.0 }, if live { crate::theme::p().accent } else { crate::theme::p().line_strong }));
        }
        let (state, colour) = match job.state {
            JobState::Rendering => (job.state.label().to_string(), crate::theme::p().accent),
            JobState::Failed => ("Failed".to_string(), failed()),
            JobState::Held => ("Held".to_string(), crate::theme::dim()),
            _ => ("Waiting".to_string(), crate::theme::dim()),
        };
        let inner = rect.shrink(8.0);
        painter.text(inner.left_top(), egui::Align2::LEFT_TOP, clip(&job.name, 18), egui::FontId::proportional(13.0), crate::theme::p().text);
        painter.text(inner.left_top() + egui::vec2(0.0, 20.0), egui::Align2::LEFT_TOP, state, egui::FontId::proportional(12.0), colour);
        painter.text(
            inner.left_top() + egui::vec2(0.0, 38.0),
            egui::Align2::LEFT_TOP,
            format!("{} × {} · {} spp", job.settings.width, job.settings.height, job.settings.max_samples),
            egui::FontId::proportional(11.0),
            crate::theme::dim(),
        );
        if job.state == JobState::Rendering {
            let bar = egui::Rect::from_min_size(egui::pos2(inner.left(), inner.bottom() - 5.0), egui::vec2(inner.width(), 5.0));
            painter.rect_filled(bar, 2.0, crate::theme::p().control);
            let done = egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * progress(job), bar.height()));
            painter.rect_filled(done, 2.0, crate::theme::p().accent);
        }
        if job.state == JobState::Failed {
            return response.on_hover_text(&job.message);
        }
        response
    }

    fn thumbnail(&self, ui: &mut egui::Ui, index: usize) -> egui::Response {
        let entry = &self.gallery[index];
        let (rect, response) = ui.allocate_exact_size(egui::vec2(150.0, 92.0), egui::Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, card());
        let image_area = egui::Rect::from_min_max(rect.min + egui::vec2(4.0, 4.0), egui::pos2(rect.max.x - 4.0, rect.max.y - 22.0));
        if let Some(texture) = &entry.texture {
            painter.image(texture.id(), fit(image_area, entry.width, entry.height), full_uv(), egui::Color32::WHITE);
        }
        painter.text(
            egui::pos2(rect.left() + 6.0, rect.bottom() - 11.0),
            egui::Align2::LEFT_CENTER,
            clip(&self.label(index), 20),
            egui::FontId::proportional(12.0),
            crate::theme::p().text,
        );
        let on_screen = match (self.selected, self.compare_with) {
            (Some(a), Some(b)) => index == a || index == b,
            (Some(a), None) => index == a,
            (None, _) => index + 1 == self.gallery.len() && !self.rendering,
        };
        if on_screen || response.hovered() {
            painter.rect_stroke(rect, 4.0, egui::Stroke::new(if on_screen { 2.0_f32 } else { 1.0 }, if on_screen { crate::theme::p().accent } else { crate::theme::p().line_strong }));
        }
        response
    }

    /// `#2 Camera 1`: a job is named after its camera by default, so five
    /// renders of one camera are five "Camera 1"s without the number.
    fn label(&self, index: usize) -> String {
        match self.gallery.get(index) {
            Some(entry) => format!("#{} {}", index + 1, entry.name),
            None => String::new(),
        }
    }

    /// The label with what makes it different: samples and time.
    fn described(&self, index: usize) -> String {
        match self.gallery.get(index) {
            Some(entry) => format!("{}  ·  {} spp  ·  {}", self.label(index), entry.samples, duration(entry.seconds)),
            None => String::new(),
        }
    }

    fn preview_fields(&self, project: &Project, settings: &RenderSettings) -> NameFields {
        // The real names and today's date, the same fields the job is given,
        // so the filename shown is the one that will be written rather than a
        // sample of what one might look like.
        NameFields {
            project: project_name(&project.document().meta.name, project.path.as_deref()),
            camera: project.camera().name.clone(),
            width: settings.width,
            height: settings.height,
            samples: settings.max_samples,
            date: crate::today(),
            time: crate::now(),
            ..Default::default()
        }
    }
}

// ---- sidebar sections that need nothing but the settings ----------------------

fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    crate::field::Fold::new(egui::RichText::new(title).strong())
        .default_open(open)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            body(ui);
            ui.add_space(4.0);
        });
    ui.add_space(2.0);
}

/// One engine in a list to pick from - here and on the viewport's chip -
/// greyed out with the reason where it cannot run on this computer. The
/// engine when it was clicked.
pub(crate) fn engine_choice(
    ui: &mut egui::Ui,
    current: tequila_scene::render::RenderEngine,
    engine: tequila_scene::render::RenderEngine,
) -> Option<tequila_scene::render::RenderEngine> {
    use tequila_scene::render::RenderEngine;
    let unavailable = match (engine, crate::batch::cycles_here()) {
        (RenderEngine::Tequila, _) => None,
        (_, Err(why)) => Some(why.clone()),
        (RenderEngine::CyclesGpu, Ok(false)) => Some("Cycles found no graphics card it can use".to_string()),
        _ => None,
    };
    let label = egui::SelectableLabel::new(current == engine, engine.label());
    let response = ui.add_enabled(unavailable.is_none(), label);
    let response = match &unavailable {
        Some(why) => response.on_disabled_hover_text(why),
        None => response.on_hover_text(match engine {
            RenderEngine::Tequila => "Tequila's own path tracer, on the processor",
            RenderEngine::CyclesCpu => "Blender's Cycles, on the processor: slower, but it can hold scenes too big for the graphics card",
            RenderEngine::CyclesGpu => "Blender's Cycles, on the graphics card: usually several times faster",
        }),
    };
    response.clicked().then_some(engine)
}

/// Whether Cycles is installed here and has a graphics card to use, asked
/// once: loading it and listing devices wakes the graphics driver, which is
/// not something to do every frame the Render window is open.
fn engine_picker(ui: &mut egui::Ui, settings: &mut RenderSettings) {
    use tequila_scene::render::RenderEngine;
    ui.horizontal(|ui| {
        ui.label("Engine");
        egui::ComboBox::from_id_salt("render-engine")
            .selected_text(settings.engine.label())
            .show_ui(ui, |ui| {
                for engine in RenderEngine::ALL {
                    if let Some(picked) = engine_choice(ui, settings.engine, engine) {
                        settings.engine = picked;
                    }
                }
            });
    });
    // What will render, where it is not what is picked.
    if let (_, Some(why)) = crate::batch::engine_for(settings, !settings.layers.is_empty()) {
        let mut why = why;
        if let Some(first) = why.get(..1) {
            why = first.to_uppercase() + &why[1..];
        }
        ui.label(egui::RichText::new(format!("{why}.")).small().color(crate::theme::dim()));
    }
}

fn quality_section(ui: &mut egui::Ui, settings: &mut RenderSettings) {
    engine_picker(ui, settings);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (name, spp, purpose) in QUALITY {
            if ui
                .selectable_label(settings.max_samples == *spp, *name)
                .on_hover_text(format!("{spp} samples per pixel: {purpose}"))
                .clicked()
            {
                settings.max_samples = *spp;
            }
        }
    });
    let mut samples = settings.max_samples;
    if ui
        .add(crate::field::Slider::new(&mut samples, 1..=8192).logarithmic(true).text("samples"))
        .on_hover_text("More samples, less noise, longer render")
        .changed()
    {
        settings.max_samples = samples;
    }

    let mut limited = settings.max_seconds.is_some();
    ui.horizontal(|ui| {
        if ui.add(crate::field::Checkbox::new(&mut limited, "Stop after")).changed() {
            settings.max_seconds = limited.then_some(300);
        }
        if let Some(seconds) = &mut settings.max_seconds {
            let mut minutes = *seconds as f32 / 60.0;
            if ui.add(egui::DragValue::new(&mut minutes).range(0.1..=1440.0).suffix(" min")).changed() {
                *seconds = (minutes * 60.0).max(1.0) as u32;
            }
            ui.label(egui::RichText::new("or the samples, whichever first").small().color(crate::theme::dim()));
        }
    });

    ui.add(crate::field::Checkbox::new(&mut settings.adaptive, "Adaptive sampling")).on_hover_text(
        "Stop working on pixels that are already clean, and spend the time on the ones that are not.",
    );

    crate::field::Fold::new("Fireflies")
        .id_salt("render-fireflies")
        .default_open(false)
        .show(ui, |ui| {
            ui.add(crate::field::Checkbox::new(&mut settings.reject_fireflies, "Firefly rejection"))
                .on_hover_text("Remove isolated over-bright pixels");
            let mut clamped = settings.clamp_indirect.is_some();
            ui.horizontal(|ui| {
                if ui.add(crate::field::Checkbox::new(&mut clamped, "Clamp samples")).changed() {
                    settings.clamp_indirect = clamped.then_some(10.0);
                }
                if let Some(clamp) = &mut settings.clamp_indirect {
                    ui.add(egui::DragValue::new(clamp).range(0.1..=1000.0).speed(0.1));
                }
            });
            if settings.clamp_indirect.is_some() {
                ui.label(
                    egui::RichText::new(
                        "A clamp removes energy. The Clamped Energy pass records how much, so the bias stays auditable.",
                    )
                    .small()
                    .color(crate::theme::dim()),
                );
            }
        });
    cycles_controls(ui, settings);
}

/// Blender's Clamping, Film filter and noise threshold, for a render that
/// must match one; each unset leaves the render as it always was.
fn cycles_controls(ui: &mut egui::Ui, settings: &mut RenderSettings) {
    use tequila_render_core::settings::{PixelFilter, PixelFilterKind};
    crate::field::Fold::new("Advanced (Cycles)")
        .id_salt("render-cycles-controls")
        .default_open(settings.clamp_direct.is_some() || settings.pixel_filter.is_some() || settings.adaptive_threshold.is_some())
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut apart = settings.clamp_direct.is_some();
                if ui.add(crate::field::Checkbox::new(&mut apart, "Clamp direct light apart")).changed() {
                    settings.clamp_direct = apart.then_some(0.0);
                }
                if let Some(clamp) = &mut settings.clamp_direct {
                    ui.add(egui::DragValue::new(clamp).range(0.0..=1000.0).speed(0.1));
                    ui.label(egui::RichText::new("0 = none").small().color(crate::theme::dim()));
                }
            })
            .response
            .on_hover_text(
                "Blender clamps direct and indirect light apart. Ticked, this is the direct clamp, and Clamp \
                 samples (or firefly rejection's 10) is the indirect one alone, so bright lights keep their highlights.",
            );
            ui.horizontal(|ui| {
                ui.label("Pixel filter");
                let current = settings.pixel_filter;
                let label = current.map_or("Box, 1 px (default)", |f| f.kind.label());
                egui::ComboBox::from_id_salt("render-pixel-filter").selected_text(label).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "Box, 1 px (default)").clicked() {
                        settings.pixel_filter = None;
                    }
                    for kind in PixelFilterKind::ALL {
                        let chosen = current.is_some_and(|f| f.kind == kind);
                        if ui.selectable_label(chosen, kind.label()).clicked() {
                            let width = current.map_or(if kind == PixelFilterKind::Box { 1.0 } else { 1.5 }, |f| f.width);
                            settings.pixel_filter = Some(PixelFilter { kind, width });
                        }
                    }
                });
                if let Some(filter) = &mut settings.pixel_filter {
                    ui.add(egui::DragValue::new(&mut filter.width).range(0.1..=10.0).speed(0.05).suffix(" px"));
                }
            })
            .response
            .on_hover_text("How each pixel's samples are spread. Blender's default is Blackman-Harris, 1.5 px: softer edges, no aliasing.");
            ui.horizontal(|ui| {
                let mut set = settings.adaptive_threshold.is_some();
                if ui.add(crate::field::Checkbox::new(&mut set, "Noise threshold")).changed() {
                    settings.adaptive_threshold = set.then_some(0.01);
                }
                if let Some(threshold) = &mut settings.adaptive_threshold {
                    ui.add(egui::DragValue::new(threshold).range(0.0001..=1.0).speed(0.0005).fixed_decimals(4));
                }
            })
            .response
            .on_hover_text("How clean a pixel must be before adaptive sampling stops working on it. Cycles' own is 0.01.");
        });
}

fn denoise_section(ui: &mut egui::Ui, settings: &mut RenderSettings) {
    ui.add(crate::field::Checkbox::new(&mut settings.denoise, "Denoise the image"))
        .on_hover_text("Clean up the remaining noise with Open Image Denoise");
    ui.add_enabled_ui(settings.denoise, |ui| {
        // Cycles' denoiser has no strength: it cleans fully or not at all.
        let cycles = settings.engine.is_cycles();
        ui.add_enabled(!cycles, crate::field::Slider::new(&mut settings.denoise_strength, 0.0..=1.0).text("strength"))
            .on_disabled_hover_text("Cycles denoises at full strength; Strength is for Tequila's renderer");
        ui.add(crate::field::Slider::new(&mut settings.denoise_cadence, 1..=32).text("preview every N passes"))
            .on_hover_text(
                "Denoising every pass looks smooth and costs real time on a large image. \
                 The final image is always denoised whatever this says.",
            );
        ui.add(crate::field::Checkbox::new(&mut settings.prefilter_guides, "Prefilter guides")).on_hover_text(
            "Filter the albedo and normal passes before using them. Worth it at low sample \
             counts, where the guides are themselves noisy.",
        );
    });
}

/// Placeholders a filename can use, in the order people reach for them.
const TOKENS: &[&str] = &["project", "product", "camera", "view", "date", "time", "resolution", "samples"];

fn file_section(ui: &mut egui::Ui, settings: &mut RenderSettings, project: &Project) {
    ui.label(egui::RichText::new("Folder").color(crate::theme::dim()));
    ui.horizontal(|ui| {
        let default = output_directory(&RenderSettings { output_directory: String::new(), ..Default::default() }, project.path.as_deref());
        ui.add(
            egui::TextEdit::singleline(&mut settings.output_directory)
                .desired_width((ui.available_width() - 76.0).max(80.0))
                .hint_text(if project.path.is_some() { "beside the project".to_string() } else { default.display().to_string() }),
        );
        if ui.button("Browse…").clicked() {
            let mut dialog = rfd::FileDialog::new();
            let start = output_directory(settings, project.path.as_deref());
            if start.is_dir() {
                dialog = dialog.set_directory(start);
            }
            if let Some(folder) = dialog.pick_folder() {
                settings.output_directory = folder.display().to_string();
            }
        }
    });
    if settings.output_directory.trim().is_empty() && project.path.is_none() {
        ui.label(
            egui::RichText::new("Save the project first to keep renders beside it, or pick a folder.")
                .small()
                .color(crate::theme::dim()),
        );
    }

    ui.label(egui::RichText::new("File name").color(crate::theme::dim()));
    ui.add(egui::TextEdit::singleline(&mut settings.filename_template).desired_width(f32::INFINITY));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for token in TOKENS {
            if ui
                .small_button(format!("+ {token}"))
                .on_hover_text(format!("Add {{{token}}} to the name"))
                .clicked()
            {
                let template = &mut settings.filename_template;
                if !template.is_empty() && !template.ends_with(['_', '-', ' ']) {
                    template.push('_');
                }
                template.push_str(&format!("{{{token}}}"));
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Format").color(crate::theme::dim()));
        egui::ComboBox::from_id_salt("render-format")
            .selected_text(settings.format.label())
            .show_ui(ui, |ui| {
                for format in OutputFormat::ALL {
                    ui.selectable_value(&mut settings.format, format, format.label());
                }
            });
        // The depths offered are the ones this format can hold, so a setting
        // that cannot be honoured is never on screen in the first place.
        let depths = settings.format.depths();
        if depths.len() > 1 {
            for depth in depths {
                ui.selectable_value(&mut settings.depth, *depth, depth.label());
            }
        }
    });
    if settings.format == OutputFormat::Jpeg {
        ui.add(crate::field::Slider::new(&mut settings.jpeg_quality, 1..=100).text("JPEG quality"));
        ui.label(
            egui::RichText::new("JPEG has no transparency, so a shadow catcher is matted against black.")
                .small()
                .color(crate::theme::dim()),
        );
    }

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Alpha").color(crate::theme::dim()));
        for mode in AlphaMode::ALL {
            let hint = match mode {
                AlphaMode::None => "No transparency: the picture as it looks here, background and all. What image viewers, web pages and slides show best.",
                AlphaMode::Straight => "Colour kept separate from transparency. What Photoshop and most editors expect.",
                AlphaMode::Premultiplied => "Colour already multiplied by transparency. What compositing tools expect.",
                AlphaMode::Aliased => "Hard edges: every pixel is either fully in or fully out.",
                AlphaMode::Silhouette => "Cut out along the product's outline, like a photographer's clipping path: glass and see-through parts stay solid, showing the studio behind them, and the floor and its shadow are left out. For catalogue shots of glass.",
            };
            ui.selectable_value(&mut settings.alpha_mode, mode, mode.label()).on_hover_text(hint);
        }
    });

    let carries = settings.format.carries_passes();
    ui.add_enabled(carries, crate::field::Checkbox::new(&mut settings.write_passes, "Render passes"))
        .on_hover_text("Depth, normals, albedo and the other passes, as layers in the same file")
        .on_disabled_hover_text("Only EXR and PSD can hold the passes.");
    if !carries {
        ui.label(egui::RichText::new("Passes need EXR or PSD.").small().color(crate::theme::dim()));
    }
    ui.horizontal(|ui| {
        ui.add(crate::field::Checkbox::new(&mut settings.write_xmp, "XMP sidecar"))
            .on_hover_text("Render settings as XMP metadata, readable by Lightroom and Bridge");
        ui.add(crate::field::Checkbox::new(&mut settings.write_text_metadata, "Text sidecar"))
            .on_hover_text("Render settings as a plain text file beside the image");
    });
}

/// Parts rendered as images of their own, to composite (§1.10).
fn layers_section(ui: &mut egui::Ui, settings: &mut RenderSettings, project: &Project) {
    ui.label(
        egui::RichText::new(
            "Each layer is written as its own image beside the main one, holding only \
             its parts. Everything else still casts shadows and shows in reflections, \
             so the layers stack back together.",
        )
        .small()
        .color(crate::theme::dim()),
    );
    let selection: Vec<tequila_scene::tree::NodeId> = project.selection.nodes().collect();
    if ui
        .add_enabled(!selection.is_empty(), egui::Button::new("+ Layer from selection"))
        .on_disabled_hover_text("Select the parts that go on the layer first.")
        .clicked()
    {
        settings.layers.push(tequila_scene::render::RenderLayer {
            name: format!("Layer {}", settings.layers.len() + 1),
            nodes: selection.clone(),
            enabled: true,
        });
    }
    let mut remove = None;
    for (index, layer) in settings.layers.iter_mut().enumerate() {
        ui.push_id(("layer", index), |ui| {
            ui.horizontal(|ui| {
                ui.add(crate::field::Checkbox::new(&mut layer.enabled, "")).on_hover_text("Render this layer");
                ui.add(egui::TextEdit::singleline(&mut layer.name).desired_width(110.0));
                ui.label(egui::RichText::new(format!("{} nodes", layer.nodes.len())).color(crate::theme::dim()));
                if ui
                    .add_enabled(!selection.is_empty(), egui::Button::new("Use selection").small())
                    .clicked()
                {
                    layer.nodes = selection.clone();
                }
                if ui.small_button("×").on_hover_text("Remove the layer").clicked() {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        settings.layers.remove(index);
    }
    ui.separator();
    ui.add(crate::field::Checkbox::new(&mut settings.relight, "Keep light groups for relighting"))
        .on_hover_text(
            "After the render, turn each light layer, the environment and glowing \
             materials up, down or tint them, without rendering again. Uses memory \
             for an image per group, and denoises each.",
        );
    let enabled = settings.layers.iter().filter(|l| l.enabled).count();
    if enabled > 0 {
        ui.label(
            egui::RichText::new(format!("Each layer is a render of its own: {} renders in all.", enabled + 1))
                .small()
                .color(crate::theme::dim()),
        );
    }
}

fn region_section(ui: &mut egui::Ui, settings: &mut RenderSettings) {
    let mut regional = settings.region.is_some();
    if ui.add(crate::field::Checkbox::new(&mut regional, "Render part of the image only")).changed() {
        settings.region = regional.then(|| Region {
            x: settings.width / 4,
            y: settings.height / 4,
            width: settings.width / 2,
            height: settings.height / 2,
        });
    }
    if let Some(region) = &mut settings.region {
        let mut values = [region.x, region.y, region.width, region.height];
        egui::Grid::new("render-region").num_columns(4).show(ui, |ui| {
            for (value, label) in values.iter_mut().zip(["X", "Y", "W", "H"]) {
                let mut v = *value as u32;
                if ui.add(egui::DragValue::new(&mut v).prefix(format!("{label} "))).changed() {
                    *value = v as usize;
                }
            }
        });
        region.x = values[0].min(settings.width.saturating_sub(1));
        region.y = values[1].min(settings.height.saturating_sub(1));
        // Clamped to the image, so a region cannot be dragged off the edge and
        // render nothing.
        region.width = values[2].clamp(1, settings.width - region.x);
        region.height = values[3].clamp(1, settings.height - region.y);
        ui.label(
            egui::RichText::new("Shown in orange on the frame before you render.")
                .small()
                .color(crate::theme::dim()),
        );
    }
}

fn full_uv() -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
}

fn align_rect(at: egui::Pos2, size: egui::Vec2, align: egui::Align2) -> egui::Rect {
    align.anchor_size(at, size)
}

pub(crate) fn clip(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// The writer's format, from the document's.
///
/// One place, so the extension shown in the filename preview and the one the
/// file actually gets cannot differ — which they would, given two translations
/// of the same enum.
pub fn image_format(settings: &RenderSettings) -> ImageFormat {
    let depth = match settings.effective_depth() {
        BitDepth::Eight => tequila_format::psd::Depth::Eight,
        BitDepth::Sixteen => tequila_format::psd::Depth::Sixteen,
        BitDepth::ThirtyTwo => tequila_format::psd::Depth::ThirtyTwo,
    };
    match settings.format {
        OutputFormat::Png => ImageFormat::Png(depth),
        OutputFormat::Jpeg => ImageFormat::Jpeg(settings.jpeg_quality),
        OutputFormat::Tiff => ImageFormat::Tiff(depth),
        OutputFormat::Psd => ImageFormat::Psd(depth),
        OutputFormat::Exr => ImageFormat::Exr(match settings.effective_depth() {
            BitDepth::ThirtyTwo => tequila_format::exr::Precision::Full,
            _ => tequila_format::exr::Precision::Half,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders with nowhere set go to the working folder, unless that is a
    /// disk's root - a Mac app opened from the Finder, which cannot write
    /// there - and then to Pictures/Tequila.
    #[test]
    fn renders_with_nowhere_set_avoid_the_root_of_the_disk() {
        let home = std::env::temp_dir().join(format!("tequila-home-{}", std::process::id()));
        let work = home.join("work");
        assert_eq!(choose_fallback(Some(work.clone()), Some(home.clone())), work);
        let root = if cfg!(windows) { PathBuf::from("C:\\") } else { PathBuf::from("/") };
        let chosen = choose_fallback(Some(root), Some(home.clone()));
        assert_eq!(chosen, home.join("Pictures").join("Tequila"));
        assert!(chosen.is_dir(), "it is made, to be written into");
        std::fs::remove_dir_all(&home).ok();
    }

    /// The extension in the preview and the one the file gets come from one
    /// translation, so they cannot disagree — which is exactly what two
    /// separate matches on the same enum would eventually do.
    #[test]
    fn the_previewed_extension_is_the_one_the_file_gets() {
        for format in OutputFormat::ALL {
            let settings = RenderSettings {
                format,
                ..Default::default()
            };
            let extension = image_format(&settings).extension();
            assert_eq!(extension, expected_extension(format));
        }
    }

    fn expected_extension(format: OutputFormat) -> &'static str {
        match format {
            OutputFormat::Png => "png",
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Tiff => "tif",
            OutputFormat::Exr => "exr",
            OutputFormat::Psd => "psd",
        }
    }

    /// An impossible depth never reaches the writer: the panel offers only what
    /// the format holds, and the translation corrects anything a saved document
    /// carries.
    #[test]
    fn an_impossible_depth_never_reaches_the_writer() {
        let settings = RenderSettings {
            format: OutputFormat::Png,
            depth: BitDepth::ThirtyTwo,
            ..Default::default()
        };
        assert!(matches!(image_format(&settings), ImageFormat::Png(tequila_format::psd::Depth::Eight)));

        let exr = RenderSettings {
            format: OutputFormat::Exr,
            depth: BitDepth::Eight,
            ..Default::default()
        };
        // EXR has no eight-bit form, so it becomes the nearest it does have.
        assert!(matches!(
            image_format(&exr),
            ImageFormat::Exr(tequila_format::exr::Precision::Half)
        ));
    }

    /// An empty folder is beside the project — what the window has always
    /// said — and not the directory the application happened to start in.
    #[test]
    fn an_empty_folder_means_beside_the_project() {
        let settings = RenderSettings::default();
        assert!(settings.output_directory.is_empty());
        let project = Path::new("C:/work/headset/quest3.tequila");
        assert_eq!(output_directory(&settings, Some(project)), PathBuf::from("C:/work/headset"));

        let chosen = RenderSettings { output_directory: "D:/renders".into(), ..Default::default() };
        assert_eq!(output_directory(&chosen, Some(project)), PathBuf::from("D:/renders"));
    }

    /// `{project}` is the file's name when the document was never named.
    #[test]
    fn an_unnamed_project_is_named_after_its_file() {
        let file = Path::new("C:/work/quest3.tequila");
        assert_eq!(project_name("Untitled", Some(file)), "quest3");
        assert_eq!(project_name("Headset", Some(file)), "Headset");
        assert_eq!(project_name("Untitled", None), "Untitled");
    }

    fn entry(name: &str, path: &str) -> GalleryEntry {
        GalleryEntry {
            name: name.into(),
            path: path.into(),
            width: 1920,
            height: 1080,
            samples: 256,
            seconds: 42.5,
            when: "2026-09-24 11:56".into(),
            texture: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("tequila-gallery-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    /// The point of the feature: close the application, open it again, and
    /// the renders are still there to look at and compare.
    #[test]
    fn renders_are_remembered_between_sessions() {
        let directory = scratch("remember");
        let image = directory.join("hero.png");
        std::fs::write(&image, b"not really a png, but it exists").unwrap();
        let image = image.display().to_string();

        let mut first = RenderWindow::default();
        first.remember_in(&directory);
        first.add(entry("Hero", &image));

        let mut second = RenderWindow::default();
        second.remember_in(&directory);
        assert_eq!(second.gallery.len(), 1);
        let back = &second.gallery[0];
        assert_eq!((back.name.as_str(), back.path.as_str()), ("Hero", image.as_str()));
        assert_eq!((back.width, back.height, back.samples), (1920, 1080, 256));
        assert_eq!(back.when, "2026-09-24 11:56");
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A render whose file has been deleted is not brought back as a
    /// thumbnail of nothing.
    #[test]
    fn a_deleted_render_is_forgotten() {
        let directory = scratch("deleted");
        let kept = directory.join("kept.png");
        std::fs::write(&kept, b"x").unwrap();
        let mut window = RenderWindow::default();
        window.remember_in(&directory);
        window.add(entry("Kept", &kept.display().to_string()));
        window.add(entry("Gone", &directory.join("gone.png").display().to_string()));

        let mut later = RenderWindow::default();
        later.remember_in(&directory);
        let names: Vec<_> = later.gallery.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Kept"]);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_history_keeps_the_newest_hundred() {
        let mut window = RenderWindow::default();
        for i in 0..(REMEMBERED + 5) {
            window.add(entry(&format!("R{i}"), "x.png"));
        }
        assert_eq!(window.gallery.len(), REMEMBERED);
        assert_eq!(window.gallery[0].name, "R5", "the oldest go first");
        assert_eq!(window.gallery.last().unwrap().name, format!("R{}", REMEMBERED + 4));
    }

    /// A 4K preview is shrunk to fit, keeps its shape, and averages rather
    /// than skips: a checkerboard becomes grey, not one of its colours.
    #[test]
    fn previews_are_shrunk_by_averaging() {
        let (w, h) = (3840, 2160);
        let mut rgba = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 0 } else { 254 };
                rgba.extend([v, v, v, 255]);
            }
        }
        let (sw, sh, small) = shrink(w, h, &rgba, PREVIEW_SIZE);
        assert!(sw <= PREVIEW_SIZE && sh <= PREVIEW_SIZE, "{sw}x{sh}");
        assert!(((sw as f32 / sh as f32) - 16.0 / 9.0).abs() < 0.01);
        // Odd blocks hold 4 of one square and 5 of the other: 4/9 or 5/9 grey.
        assert!((100..=155).contains(&small[0]), "averaged to grey, got {}", small[0]);
        // Already small enough: untouched.
        assert_eq!(shrink(4, 2, &[7; 32], PREVIEW_SIZE), (4, 2, vec![7; 32]));
    }

    /// A PNG written the way renders are written reads back at the right size.
    #[test]
    fn a_png_render_reads_back_for_its_preview() {
        let directory = scratch("png");
        let path = directory.join("render.png");
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(file, 3, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9, 50, 50, 50, 200, 200, 200]).unwrap();
        let image = load_preview(&path.display().to_string()).expect("decodes");
        assert_eq!(image.size, [3, 2]);
        assert_eq!(image.pixels[0], egui::Color32::from_rgb(255, 0, 0));
        assert!(load_preview(&directory.join("absent.png").display().to_string()).is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_locked_ratio_follows_the_width() {
        assert_eq!(locked_height(3840, 1920, 1080), 2160);
        assert_eq!(locked_height(1000, 1600, 2000), 1250);
        // A degenerate old size does not divide by zero.
        assert_eq!(locked_height(500, 0, 300), 300);
    }

    #[test]
    fn ratios_have_their_short_names() {
        assert_eq!(ratio_name(1920, 1080), "16:9");
        assert_eq!(ratio_name(1600, 2000), "4:5");
        assert_eq!(ratio_name(2048, 2048), "1:1");
        assert_eq!(ratio_name(2480, 3508), "0.71:1");
    }

    #[test]
    fn an_image_is_fitted_without_stretching() {
        let bounds = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 500.0));
        let wide = fit(bounds, 1920, 1080);
        assert!((wide.width() / wide.height() - 1920.0 / 1080.0).abs() < 1e-3);
        assert!(wide.height() <= 500.0 + 1e-3 && wide.width() <= 1000.0 + 1e-3);
        let tall = fit(bounds, 1080, 1920);
        assert!((tall.height() - 500.0).abs() < 1e-3, "a tall image fills the height");
        assert!((tall.center().x - 500.0).abs() < 1e-3, "and is centred");
    }

    #[test]
    fn every_quality_preset_is_named_and_nothing_else_is() {
        for (name, spp, _) in QUALITY {
            assert_eq!(quality_name(*spp), Some(*name));
        }
        assert_eq!(quality_name(300), None);
    }
}
