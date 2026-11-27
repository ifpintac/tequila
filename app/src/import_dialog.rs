//! The import dialog (`docs/04-UX-SPEC.md` §11.1, `docs/03` §6): how a
//! model comes in, then the import itself with a progress bar and a Cancel.
//!
//! Every model the person imports - File › Import model, the Import button,
//! a file dropped on the window or named on the command line - comes here.
//! The dialog asks:
//! - **where:** into this scene, or a new one (asking about unsaved work as
//!   File › New does);
//! - **which way up:** as the file says, Y up, Z up, or another axis;
//! - **what unit** its numbers are in, remembered for each kind of file,
//!   with the size the model comes out at, so a wrong unit or the wrong way
//!   up is obvious before it is in;
//! - for a CAD file, **how finely** its exact surfaces are divided (Tequila's
//!   four presets, or a chord and angle of the person's own), and whether
//!   they are **kept** with the project to divide again later (the Scene
//!   tab's Tessellation, and Improve model);
//! - whether to **point the camera** at the model and **fit the
//!   environment** to it ([`fit_environment`]), and whether to stand it on
//!   the floor;
//! - for a Blender file, what to do with its own lighting.
//!
//! The choices are kept for the next import (`import_settings::Choices`);
//! "Don't ask again" imports with them without asking, and File › Show
//! import options or Preferences › Import bring the question back.
//!
//! To size the model before it is in, the file is read as soon as the
//! dialog opens, on another thread; the import then carries on from that
//! read rather than reading again, unless a CAD file is to be divided at
//! another quality. A large CAD file is not read ahead: reading one can
//! take minutes and gigabytes, and a second read at the quality finally
//! chosen would have to wait for the first.
//!
//! The import runs off the window's thread, under a
//! [`Watch`](tequila_import::watch::Watch): the bar shows the stage, and a
//! share where the importer can say how far it is - OpenCASCADE's reading,
//! the faces divided, the meshes prepared - and moves without one where it
//! cannot. Cancel stops it (OpenCASCADE's helper at once) and leaves the
//! scene as it was: nothing is in the document until the import is done.

use crate::import_settings::{self, BlenderLights, Choices, Target, UNITS};
use crate::project::PreparedImport;
use crate::{theme, Arrived, PendingImport, TequilaApp};
use eframe::egui;
use std::collections::{BTreeSet, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;
use tequila_core::units::LengthUnit;
use tequila_import::step::Tessellation;
use tequila_import::watch::{Seen, Stage, Watch};
use tequila_import::{ImportError, ImportOptions, Preview, ReadModel, UpAxis};
use tequila_scene::command::{EnvParam, EnvValue, EnvironmentParameter};
use tequila_scene::tree::NodeId;
use tequila_scene::{Command, SceneDocument};

/// A CAD file larger than this is not read until Import is pressed.
const READ_AHEAD_CAD: u64 = 40 * 1024 * 1024;

/// What happens once the model is in, carried with the import to where it
/// lands (`TequilaApp::poll_imports`, [`TequilaApp::after_import`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    pub frame_camera: bool,
    pub fit_environment: bool,
}

/// The dialog, and the files waiting their turn: one field on the
/// application.
pub struct Dialogs {
    current: Option<ImportDialog>,
    /// Files chosen or dropped while one was being imported, in order.
    waiting: VecDeque<PathBuf>,
    /// "Use these choices for the other files": the rest go in without
    /// asking, until none is waiting.
    same_for_rest: bool,
    /// Keep what is chosen in the settings. Off in tests, which must not
    /// change the person's own.
    pub remember: bool,
}

impl Default for Dialogs {
    fn default() -> Self {
        Dialogs { current: None, waiting: VecDeque::new(), same_for_rest: false, remember: true }
    }
}

impl Dialogs {
    /// Whether an import is being asked about or is under way.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }
}

/// What arrives from an import's thread.
enum Message {
    Read(Result<Box<ReadModel>, ImportError>),
    Prepared(Result<PreparedImport, ImportError>),
}

/// Work on another thread: its watch, and where its answer comes.
struct Job {
    watch: Watch,
    receiver: mpsc::Receiver<Message>,
    /// The quality a read divides at.
    quality: Tessellation,
}

impl Job {
    fn read(path: PathBuf, quality: Tessellation) -> Job {
        let watch = Watch::new();
        let (sender, receiver) = mpsc::channel();
        let on_thread = watch.clone();
        std::thread::spawn(move || {
            let read = on_thread.during(|| tequila_import::read_file_with(&path, quality));
            let _ = sender.send(Message::Read(read.map(Box::new)));
        });
        Job { watch, receiver, quality }
    }

    fn prepare(model: ReadModel, options: ImportOptions, watch: Watch, quality: Tessellation) -> Job {
        let (sender, receiver) = mpsc::channel();
        let on_thread = watch.clone();
        std::thread::spawn(move || {
            let prepared = on_thread.during(|| PreparedImport::prepare_with(model, Some(options)));
            let _ = sender.send(Message::Prepared(prepared));
        });
        Job { watch, receiver, quality }
    }
}

enum Phase {
    /// The options are up; the file may be being read ahead to size it.
    Choosing,
    /// Reading, dividing, preparing.
    Importing(Going),
    /// Ready: put in on the frame after the one that says so, since
    /// building a big model holds the window for a moment.
    Building(Going, Box<PreparedImport>, bool),
}

/// An import under way.
struct Going {
    started: Instant,
    plan: Plan,
    /// Into a new scene: the history's count of changes when the person
    /// agreed to leave the scene, to ask again if they worked on meanwhile.
    new_scene: Option<u64>,
    /// The project it goes into, for one added to the scene.
    epoch: u64,
}

/// What the person did in the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Import,
    /// Close or Cancel: no import, or the one under way stopped.
    Cancel,
}

/// What a file's format is, in the dialog's words.
fn format_name(extension: &str) -> &'static str {
    match extension {
        "step" | "stp" => "STEP",
        "igs" | "iges" => "IGES",
        "3dm" => "Rhino",
        "obj" => "OBJ",
        "stl" => "STL",
        "ply" => "PLY",
        "3mf" => "3MF",
        "fbx" => "FBX",
        "gltf" | "glb" => "glTF",
        "usd" | "usda" | "usdc" | "usdz" => "USD",
        "ifc" => "IFC",
        "blend" => "Blender",
        "ksp" | "bip" => "KeyShot",
        _ => "model",
    }
}

/// The custom tolerances a dialog starts with: a custom pair kept from
/// last time, or the preset's own.
fn custom_of(quality: Tessellation) -> (f64, f64) {
    (quality.chord(), quality.angle())
}

/// One model being asked about, then imported.
pub struct ImportDialog {
    path: PathBuf,
    /// Its file name, extension and all, as the window says it.
    name: String,
    extension: String,
    bytes: u64,
    choices: Choices,
    /// The unit its numbers are read in; `None` as the file says.
    unit: Option<LengthUnit>,
    up: UpAxis,
    lights: BlenderLights,
    /// Whether the options were put to the person, rather than taken as
    /// last chosen.
    asked: bool,
    dont_ask: bool,
    /// The custom chord and angle as last edited, kept while a preset is
    /// picked so going back to Custom finds them.
    custom: (f64, f64),
    job: Option<Job>,
    /// The model read ahead, and the quality it was divided at.
    model: Option<(Box<ReadModel>, Tessellation)>,
    preview: Option<Preview>,
    /// Why reading it ahead failed, to say.
    problem: Option<String>,
    phase: Phase,
}

impl ImportDialog {
    fn new(path: PathBuf, choices: Choices, up: UpAxis, lights: BlenderLights, asked: bool) -> ImportDialog {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let extension = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let unit = choices.unit_for(&extension);
        let custom = custom_of(choices.tessellation);
        let mut dialog = ImportDialog {
            path,
            name,
            extension,
            bytes,
            choices,
            unit,
            up,
            lights,
            asked,
            dont_ask: false,
            custom,
            job: None,
            model: None,
            preview: None,
            problem: None,
            phase: Phase::Choosing,
        };
        if asked && dialog.reads_ahead() {
            dialog.job = Some(Job::read(dialog.path.clone(), dialog.quality()));
        }
        dialog
    }

    fn divides(&self) -> bool {
        tequila_import::divides_surfaces(&self.extension)
    }

    /// Whether the file is read as the dialog opens, to size it.
    fn reads_ahead(&self) -> bool {
        !self.divides() || self.bytes <= READ_AHEAD_CAD
    }

    /// The quality chosen, its custom tolerances held to their ranges.
    fn quality(&self) -> Tessellation {
        match self.choices.tessellation {
            Tessellation::Custom { .. } => Tessellation::custom(self.custom.0, self.custom.1),
            preset => preset,
        }
    }

    /// The importer's options: the unit chosen - or, the person having
    /// seen the guess for a file that states none, the guess, so it is not
    /// asked about again once the model is in - the up axis and the floor.
    fn options(&self) -> ImportOptions {
        let seen_guess = self.asked.then_some(self.preview.as_ref()).flatten().filter(|p| p.stated_unit.is_none()).map(|p| p.guessed_unit).filter(|u| *u != LengthUnit::Unspecified);
        ImportOptions { source_unit: self.unit.or(seen_guess), place_on_ground: self.choices.on_floor, up_axis: self.up, ..ImportOptions::default() }
    }

    /// The choices to keep for next time.
    fn kept(&self) -> Choices {
        let mut choices = self.choices.clone();
        choices.tessellation = self.quality();
        choices.ask = !self.dont_ask && choices.ask;
        choices.set_unit_for(&self.extension, self.unit);
        choices
    }

    fn plan(&self) -> Plan {
        Plan { frame_camera: self.choices.frame_camera, fit_environment: self.choices.fit_environment }
    }

    /// Answers from the threads, taken in turn. An error to say, when the
    /// import under way failed.
    fn poll(&mut self) -> Option<String> {
        let message = {
            let job = self.job.as_ref()?;
            match job.receiver.try_recv() {
                Ok(message) => message,
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                    return matches!(self.phase, Phase::Importing(_)).then(|| "the import stopped before it finished".to_string());
                }
            }
        };
        let job = self.job.take().expect("a job answered");
        let importing = matches!(self.phase, Phase::Importing(_));
        match (message, importing) {
            (Message::Read(Ok(model)), false) => {
                self.preview = Some(model.preview());
                self.model = Some((model, job.quality));
            }
            (Message::Read(Err(ImportError::Cancelled)), _) => {}
            (Message::Read(Err(error)), false) => self.problem = Some(error.to_string()),
            (Message::Read(Ok(model)), true) => {
                self.preview.get_or_insert_with(|| model.preview());
                self.job = Some(self.prepare(*model, job.watch));
            }
            (Message::Prepared(Ok(ready)), true) => {
                if let Phase::Importing(going) = std::mem::replace(&mut self.phase, Phase::Choosing) {
                    self.phase = Phase::Building(going, Box::new(ready), false);
                }
            }
            (Message::Read(Err(error)) | Message::Prepared(Err(error)), true) => return Some(error.to_string()),
            // An answer for a stage left behind.
            _ => {}
        }
        None
    }

    /// The model read, prepared with the options, on another thread.
    fn prepare(&self, mut model: ReadModel, watch: Watch) -> Job {
        if !self.choices.keep_cad {
            model.brep = None;
        }
        Job::prepare(model, self.options(), watch, self.quality())
    }

    /// Start importing: from the model read ahead where it was read as
    /// wanted, from the read under way where that is, or from a new read.
    fn begin(&mut self, going: Going) {
        let (quality, divides) = (self.quality(), self.divides());
        let fits = move |read_at: Tessellation| !divides || read_at == quality;
        self.phase = Phase::Importing(going);
        if let Some((model, read_at)) = self.model.take() {
            if fits(read_at) {
                self.job = Some(self.prepare(*model, Watch::new()));
                return;
            }
        }
        match self.job.take() {
            Some(job) if fits(job.quality) => self.job = Some(job),
            other => {
                if let Some(job) = other {
                    job.watch.cancel();
                }
                self.job = Some(Job::read(self.path.clone(), quality));
            }
        }
    }

    /// Stop whatever is being done for it.
    fn cancel(&mut self) {
        if let Some(job) = self.job.take() {
            job.watch.cancel();
        }
    }

    fn seen(&self) -> Seen {
        match (&self.phase, &self.job) {
            (Phase::Building(..), _) => Seen { stage: Stage::Building, done: 0, total: 0, percent: None },
            (_, Some(job)) => job.watch.seen(),
            _ => Seen { stage: Stage::Reading, done: 0, total: 0, percent: None },
        }
    }

    fn importing(&self) -> bool {
        !matches!(self.phase, Phase::Choosing)
    }

    /// The size line: what the model comes in at, read as chosen.
    fn size_line(&self) -> Option<String> {
        let preview = self.preview.as_ref()?;
        let metres = preview.metres(self.unit, self.up);
        (metres.length() > 0.0).then(|| crate::model_units::real_size(metres))
    }

    /// The window. What the person did, if anything.
    fn show(&mut self, ctx: &egui::Context, waiting: usize, same_for_rest: &mut bool, replaces_starter: bool) -> Option<Act> {
        let mut open = true;
        let mut act = None;
        let title = if self.importing() { "Importing" } else { "Import model" };
        egui::Window::new(title)
            .id(egui::Id::new("import-dialog"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(430.0)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.set_width(430.0);
                if self.importing() {
                    act = self.progress_ui(ui);
                } else {
                    act = self.options_ui(ui, waiting, same_for_rest, replaces_starter);
                }
            });
        if !open {
            act = Some(Act::Cancel);
        }
        if let Phase::Building(_, _, shown) = &mut self.phase {
            *shown = true;
        }
        act
    }

    /// The file and what it comes in as, on a card at the top.
    fn header_ui(&self, ui: &mut egui::Ui) {
        let p = theme::p();
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(34.0), egui::Sense::hover());
                theme::badge(ui.painter(), rect, crate::icons::Icon::Import, p.accent);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(&self.name).font(theme::strong(14.0)).color(p.text));
                    let kind = if self.divides() { "exact CAD surfaces, divided into triangles as it is read" } else { "triangles already" };
                    ui.label(egui::RichText::new(format!("{} file, {}: {kind}", format_name(&self.extension), size_of_file(self.bytes))).size(11.5).color(p.text_dim));
                });
            });
            ui.add_space(6.0);
            match (&self.preview, &self.problem) {
                (Some(preview), _) => {
                    if let Some(size) = self.size_line() {
                        ui.label(egui::RichText::new(format!("Comes in {size}")).font(theme::strong(15.0)).color(p.text));
                    }
                    let mut about = format!("{} {}, {} triangles", grouped(preview.parts), if preview.parts == 1 { "part" } else { "parts" }, grouped(preview.triangles));
                    if self.divides() && preview.divided {
                        let read_at = match self.model.as_ref().map(|(_, quality)| *quality) {
                            Some(Tessellation::Custom { .. }) => "divided at your own tolerances".to_string(),
                            Some(preset) => format!("divided at {}", preset.label()),
                            None => "divided at the quality chosen".to_string(),
                        };
                        about.push_str(&format!(", {read_at}"));
                    }
                    theme::note(ui, about);
                    if preview.has_setup && self.up != UpAxis::AsFile {
                        theme::warning(ui, "The file brings its own cameras and lighting, so it is not turned: they would no longer match.");
                    }
                }
                (None, Some(problem)) => {
                    theme::warning(ui, format!("It could not be read to measure: {problem}"));
                }
                (None, None) if self.job.is_some() => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(egui::RichText::new("Measuring it\u{2026}").color(p.text_dim));
                    });
                }
                (None, None) => {
                    theme::note(ui, "Its size shows once it is read: a CAD file this large takes a while, so it is read when you press Import.");
                }
            }
        });
    }

    fn options_ui(&mut self, ui: &mut egui::Ui, waiting: usize, same_for_rest: &mut bool, replaces_starter: bool) -> Option<Act> {
        let mut act = None;
        self.header_ui(ui);
        // The choices scroll where the screen is too short for them all -
        // a 13-inch laptop's - with the file above and the buttons below
        // always in sight.
        let room = (ui.ctx().screen_rect().height() - 360.0).max(200.0);
        egui::ScrollArea::vertical().id_salt("import-choices").max_height(room).auto_shrink([false, true]).show(ui, |ui| {
            self.choices_ui(ui, replaces_starter);
        });

        theme::rule(ui);
        ui.add_space(4.0);
        ui.checkbox(&mut self.dont_ask, "Don't ask again").on_hover_text(
            "Models come in with these choices without this window. Turn it back on in File › Show import options, or Preferences › Import.",
        );
        if waiting > 0 {
            ui.checkbox(same_for_rest, format!("Use these choices for the other {waiting} {}", if waiting == 1 { "file" } else { "files" }));
        }
        ui.add_space(6.0);
        buttons(ui, |ui| {
            let p = theme::p();
            let primary = egui::Button::new(egui::RichText::new("Import").color(p.on_accent)).fill(p.accent).min_size(egui::vec2(88.0, 0.0));
            if ui.add(primary).clicked() {
                act = Some(Act::Import);
            }
            if ui.button("Cancel").clicked() {
                act = Some(Act::Cancel);
            }
        });
        act
    }

    /// The choices themselves, between the file and the buttons.
    fn choices_ui(&mut self, ui: &mut egui::Ui, replaces_starter: bool) {
        theme::section(ui, "Where");
        theme::segmented(ui, &mut self.choices.target, &[(Target::Add, "Add to this scene"), (Target::New, "New scene")]);
        match self.choices.target {
            Target::Add if replaces_starter => theme::note(ui, "It takes the starter ball's place; your lighting and look stay."),
            Target::Add => theme::note(ui, "Beside what is in the scene, which keeps its lighting and look."),
            Target::New => theme::note(ui, "A fresh scene, as File › New starts one. Unsaved changes are asked about first."),
        };

        theme::section(ui, "Up axis");
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 104.0).max(160.0);
            ui.allocate_ui(egui::vec2(width, 28.0), |ui| {
                theme::segmented(ui, &mut self.up, &[(UpAxis::AsFile, "Automatic"), (UpAxis::Y, "Y up"), (UpAxis::Z, "Z up")]);
            });
            let others = [UpAxis::X, UpAxis::NegativeX, UpAxis::NegativeY, UpAxis::NegativeZ];
            let shown = if others.contains(&self.up) { self.up.label() } else { "Other" };
            egui::ComboBox::from_id_salt("import-up-other").width(92.0).selected_text(shown).show_ui(ui, |ui| {
                for up in others {
                    ui.selectable_value(&mut self.up, up, up.label());
                }
            });
        });
        theme::note(
            ui,
            match self.up {
                UpAxis::AsFile => "As the file says: glTF, USD, FBX and 3MF say which way is up; the rest are taken as Y up.",
                UpAxis::Z => "For a model drawn Z up, as most CAD packages draw: otherwise it comes in lying on its back.",
                UpAxis::Y => "Tequila is Y up, as SolidWorks and most modelling programs are.",
                _ => "Turned so this axis points up.",
            },
        );

        theme::section(ui, "Unit");
        let automatic = match &self.preview {
            Some(Preview { stated_unit: Some(unit), .. }) => format!("Automatic: {}, as the file says", crate::unit_label(*unit).to_lowercase()),
            Some(Preview { guessed_unit, .. }) if *guessed_unit != LengthUnit::Unspecified => {
                format!("Automatic: {}, a guess from its size", crate::unit_label(*guessed_unit).to_lowercase())
            }
            _ => "Automatic: as the file says".to_string(),
        };
        let size_as = |unit: Option<LengthUnit>| self.preview.as_ref().map(|p| crate::model_units::real_size(p.metres(unit, self.up)));
        let selected = match self.unit {
            Some(unit) => crate::unit_label(unit).to_string(),
            None => automatic.clone(),
        };
        let mut unit = self.unit;
        egui::ComboBox::from_id_salt("import-unit").width(ui.available_width() - 8.0).selected_text(selected).show_ui(ui, |ui| {
            let row = |ui: &mut egui::Ui, unit: &mut Option<LengthUnit>, value: Option<LengthUnit>, text: String| {
                let label = match size_as(value) {
                    Some(size) => format!("{text}   \u{2192} {size}"),
                    None => text,
                };
                ui.selectable_value(unit, value, label);
            };
            row(ui, &mut unit, None, automatic.clone());
            for each in UNITS {
                row(ui, &mut unit, Some(each), crate::unit_label(each).to_string());
            }
        });
        self.unit = unit;
        theme::note(ui, format!("Kept for {} files. Materials are made to real sizes, so at the wrong size their patterns come out far too fine or too coarse.", format_name(&self.extension)));

        theme::section(ui, "Surfaces");
        let divides = self.divides();
        let mesh_reason = format!("{} files are triangles already: there are no exact surfaces to divide.", format_name(&self.extension));
        ui.add_enabled_ui(divides, |ui| {
            let mut index = match self.choices.tessellation {
                Tessellation::Draft => 0,
                Tessellation::Medium => 1,
                Tessellation::High => 2,
                Tessellation::Ultra => 3,
                Tessellation::Custom { .. } => 4,
            };
            let response = ui.scope(|ui| theme::segmented(ui, &mut index, &[(0, "Draft"), (1, "Medium"), (2, "High"), (3, "Ultra"), (4, "Custom")])).response;
            response.on_disabled_hover_text(mesh_reason.as_str());
            let picked = match index {
                0 => Tessellation::Draft,
                1 => Tessellation::Medium,
                2 => Tessellation::High,
                3 => Tessellation::Ultra,
                _ => Tessellation::custom(self.custom.0, self.custom.1),
            };
            if picked != self.choices.tessellation {
                // Custom starts from the preset it was on.
                if matches!(picked, Tessellation::Custom { .. }) && !matches!(self.choices.tessellation, Tessellation::Custom { .. }) {
                    self.custom = custom_of(self.choices.tessellation);
                }
                self.choices.tessellation = match picked {
                    Tessellation::Custom { .. } => Tessellation::custom(self.custom.0, self.custom.1),
                    preset => preset,
                };
            }
            if let Tessellation::Custom { .. } = self.choices.tessellation {
                ui.horizontal(|ui| {
                    let mut percent = self.custom.0 * 100.0;
                    ui.label("Deviation");
                    ui.add(
                        egui::DragValue::new(&mut percent)
                            .range(Tessellation::CHORDS.start() * 100.0..=Tessellation::CHORDS.end() * 100.0)
                            .speed(0.001)
                            .fixed_decimals(3)
                            .suffix(" %"),
                    )
                    .on_hover_text("How far a triangle may stray from the true surface, as a share of each part's size. Smaller is smoother, and more triangles.");
                    ui.add_space(12.0);
                    let mut degrees = self.custom.1.to_degrees();
                    ui.label("Angle");
                    ui.add(egui::DragValue::new(&mut degrees).range(3.0..=60.0).speed(0.2).fixed_decimals(1).suffix("°"))
                        .on_hover_text("How far a curve may turn from one triangle's edge to the next: smaller keeps small fillets round.");
                    self.custom = (percent / 100.0, degrees.to_radians());
                    self.choices.tessellation = Tessellation::custom(self.custom.0, self.custom.1);
                });
            }
        });
        if divides {
            theme::note(
                ui,
                match self.choices.tessellation {
                    Tessellation::Draft => "Fewest triangles: a big assembly seen whole.",
                    Tessellation::Medium => "Smooth at a normal distance: how a CAD file comes in.",
                    Tessellation::High => "For close-ups of curved edges.",
                    Tessellation::Ultra => "For extreme close-ups; many triangles.",
                    Tessellation::Custom { .. } => "Your own tolerances, as OpenCASCADE takes them.",
                },
            );
        }
        let (keepable, why_not) = self.keepable();
        let mut keep = self.choices.keep_cad && keepable;
        let response = ui.add_enabled(keepable, egui::Checkbox::new(&mut keep, "Keep the CAD data with the project"));
        let response = response.on_hover_text("Its exact surfaces go in the project, so the model can be divided again at another quality without its file: right-click it in the Scene tab, Tessellation.");
        if let Some(why_not) = why_not {
            response.on_disabled_hover_text(why_not);
        }
        if keepable {
            self.choices.keep_cad = keep;
        }

        if self.extension == "blend" {
            theme::section(ui, "Lights in Blender files");
            for choice in BlenderLights::ALL {
                ui.radio_value(&mut self.lights, choice, choice.label());
            }
        }

        theme::section(ui, "Once it is in");
        ui.checkbox(&mut self.choices.frame_camera, "Point the camera at the model")
            .on_hover_text("Frame the view on the model, so it is not off screen or a speck.");
        ui.checkbox(&mut self.choices.fit_environment, "Fit the environment to the model").on_hover_text(
            "A ground set to a size of its own fits the scene again, and a dome of a set size is sized to it, its height in proportion. \
             An endless dome and a ground that fits already need nothing. Undo takes it back.",
        );
        ui.checkbox(&mut self.choices.on_floor, "Stand it on the floor")
            .on_hover_text("Move it up or down so its lowest point is on the ground. Off, it lands where it was modelled: one part of an assembly beside the rest.");
    }

    /// Whether its CAD data can be kept, and why not where it cannot.
    fn keepable(&self) -> (bool, Option<String>) {
        let format = format_name(&self.extension);
        if !self.divides() {
            return (false, Some(format!("{format} files are triangles already: there are no exact surfaces to keep.")));
        }
        if !tequila_import::keeps_source(&self.extension) {
            return (false, Some("A KeyShot scene is divided again from the scene file itself, where it is.".to_string()));
        }
        if self.bytes > tequila_import::MOST_KEPT as u64 {
            return (false, Some(format!("Files over {} MB are not kept in the project; dividing again reads the file from where it is.", tequila_import::MOST_KEPT / (1024 * 1024))));
        }
        if self.preview.as_ref().is_some_and(|p| !p.divided) {
            return (false, Some("This file holds meshes only: there are no exact surfaces to keep.".to_string()));
        }
        (true, None)
    }

    fn progress_ui(&mut self, ui: &mut egui::Ui) -> Option<Act> {
        let p = theme::p();
        let seen = self.seen();
        let mut act = None;
        ui.label(egui::RichText::new(&self.name).font(theme::strong(14.0)).color(p.text));
        ui.add_space(4.0);
        let fraction = seen.fraction();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(stage_text(&seen)).color(p.text));
            if let Some(fraction) = fraction {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{:.0}%", fraction * 100.0)).color(p.text_dim));
                });
            }
        });
        bar(ui, fraction);
        ui.add_space(4.0);
        let started = match &self.phase {
            Phase::Importing(going) | Phase::Building(going, ..) => Some(going),
            Phase::Choosing => None,
        };
        if let Some(going) = started {
            let mut note = format!("{} s", going.started.elapsed().as_secs());
            if going.new_scene.is_some() {
                note.push_str(". A new scene takes this one's place once the model is ready.");
            } else {
                note.push_str(". The scene is not touched until the model is ready, and Cancel leaves it as it is.");
            }
            theme::note(ui, note);
        }
        ui.add_space(6.0);
        let building = matches!(self.phase, Phase::Building(..));
        buttons(ui, |ui| {
            if ui.add_enabled(!building, egui::Button::new("Cancel")).clicked() {
                act = Some(Act::Cancel);
            }
        });
        act
    }
}

/// A row of buttons at the window's right: one row high, where a
/// right-to-left layout on its own takes all the height there is and
/// stretches the window to the screen.
fn buttons(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), add);
}

/// What the import is doing, in words.
fn stage_text(seen: &Seen) -> String {
    match seen.stage {
        Stage::Reading => "Reading the file".to_string(),
        Stage::Dividing => format!("Dividing surfaces: {} of {} faces", grouped(seen.done), grouped(seen.total)),
        Stage::Preparing if seen.total > 1 => format!("Preparing meshes: {} of {}", grouped(seen.done), grouped(seen.total)),
        Stage::Preparing => "Preparing the meshes".to_string(),
        Stage::Building => "Building the scene".to_string(),
    }
}

/// A progress bar: filled to `fraction`, or, where how far is not known, a
/// band sweeping across to say it is working.
fn bar(ui: &mut egui::Ui, fraction: Option<f32>) {
    let p = theme::p();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, p.field);
    match fraction {
        Some(fraction) => {
            let filled = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()));
            if filled.width() > 0.5 {
                painter.rect_filled(filled, 4.0, p.accent);
            }
        }
        None => {
            let time = ui.input(|i| i.time) as f32;
            let band = rect.width() * 0.3;
            let travel = rect.width() + band;
            let left = rect.left() - band + (time * 0.6).fract() * travel;
            let shown = egui::Rect::from_x_y_ranges(left.max(rect.left())..=(left + band).min(rect.right()), rect.y_range());
            if shown.width() > 0.5 {
                painter.rect_filled(shown, 4.0, p.accent);
            }
            ui.ctx().request_repaint();
        }
    }
}

/// A count with its thousands apart: 12,400.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A file's size as a person reads it.
fn size_of_file(bytes: u64) -> String {
    match bytes {
        b if b >= 1024 * 1024 * 1024 => format!("{:.1} GB", b as f64 / (1024.0 * 1024.0 * 1024.0)),
        b if b >= 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b if b >= 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{b} bytes"),
    }
}

/// What fitting the environment to the model changes, as commands: the
/// scene with the model in it, as the renderer measures it, held by the
/// ground and the dome the way Tequila fits them elsewhere.
///
/// - A ground of a size of its own goes back to fitting the scene (size 0),
///   which is what a new scene's ground does: three times the scene's
///   diagonal each way, kept fitted as the scene changes.
/// - A dome of a set size (a finite environment, whose ground and
///   reflections are projected from a point) is sized so its radius
///   reaches as far as the fitted ground does, its height above the
///   ground in the same proportion as before. An endless dome, the
///   default, is left: it fits anything.
///
/// The lights are left where they are: they are things in the scene the
/// person placed, and a Studio preset sizes its own to the product when
/// it is applied. Nothing at all for a scene with nothing shown.
pub fn fit_environment(document: &SceneDocument) -> Vec<Command> {
    let bounds = tequila_scene::snapshot::visible_bounds(document);
    if bounds.is_empty() || !bounds.diagonal().length().is_finite() {
        return Vec::new();
    }
    let reach = (bounds.diagonal().length() * 3.0).max(1e-3);
    let environment = &document.environment;
    let set = |param: EnvParam, value: f32| Command::SetEnvironmentParameter { parameter: EnvironmentParameter::new(param, EnvValue::Scalar(value)) };
    let mut out = Vec::new();
    if environment.ground.size > 0.0 {
        out.push(set(EnvParam::GroundSize, 0.0));
    }
    if environment.size > 0.0 {
        let size = reach * 2.0;
        if (size - environment.size).abs() > environment.size * 0.01 {
            out.push(set(EnvParam::Size, size));
            if environment.height != 0.0 {
                out.push(set(EnvParam::Height, environment.height * size / environment.size));
            }
        }
    }
    out
}

impl TequilaApp {
    /// A model to import, however it was chosen: the dialog for it, or -
    /// asked not to ask, or to treat the rest of a batch alike - straight in
    /// with the last choices, the progress window showing. One at a time;
    /// the others wait their turn.
    pub(crate) fn begin_import(&mut self, path: PathBuf) {
        let choices = if self.import_dialogs.remember { import_settings::choices() } else { self.import_dialogs.current.as_ref().map(|d| d.choices.clone()).unwrap_or_default() };
        self.begin_import_with(path, choices);
    }

    /// [`begin_import`](Self::begin_import) with these choices to start
    /// from.
    pub(crate) fn begin_import_with(&mut self, path: PathBuf, choices: Choices) {
        if self.import_dialogs.current.is_some() {
            self.import_dialogs.waiting.push_back(path);
            return;
        }
        let asked = choices.ask && !self.import_dialogs.same_for_rest;
        let (up, lights) = if self.import_dialogs.remember { (import_settings::up_axis(), import_settings::blender_lights()) } else { (UpAxis::AsFile, BlenderLights::Ask) };
        self.import_dialogs.current = Some(ImportDialog::new(path, choices, up, lights, asked));
        if !asked {
            self.confirm_import();
        }
    }

    /// Import with the dialog's choices: what its Import button does. Into
    /// a new scene, unsaved work is asked about first; a "no" keeps the
    /// dialog up, or, never shown, drops the import.
    pub(crate) fn confirm_import(&mut self) {
        let Some(mut dialog) = self.import_dialogs.current.take() else { return };
        let new_scene = dialog.choices.target == Target::New;
        if new_scene && !self.confirm_unsaved() {
            if dialog.asked {
                self.import_dialogs.current = Some(dialog);
            } else {
                dialog.cancel();
                self.project.status = format!("{} was not imported: the scene you have stays", dialog.name);
                self.next_import();
            }
            return;
        }
        if self.import_dialogs.remember && dialog.asked {
            let kept = [
                import_settings::set_choices(dialog.kept()),
                import_settings::set_up_axis(dialog.up),
                if dialog.extension == "blend" { import_settings::set_blender_lights(dialog.lights) } else { Ok(()) },
            ];
            if let Some(Err(error)) = kept.into_iter().find(Result::is_err) {
                self.toasts.warn(format!("The import choices could not be kept: {error}"));
            }
        }
        let going = Going { started: Instant::now(), plan: dialog.plan(), new_scene: new_scene.then(|| self.project.changes()), epoch: self.project.document_epoch() };
        dialog.begin(going);
        self.project.status = format!("Importing {}\u{2026}", dialog.name);
        self.import_dialogs.current = Some(dialog);
    }

    /// The dialog's threads answered, and an import that is ready put in.
    /// True while there is anything to wait for.
    pub(crate) fn pump_import_dialog(&mut self) -> bool {
        let Some(mut dialog) = self.import_dialogs.current.take() else { return false };
        if let Some(error) = dialog.poll() {
            self.project.status = format!("{}: {error}", dialog.name);
            self.toasts.warn(self.project.status.clone());
            self.next_import();
            return true;
        }
        if matches!(dialog.phase, Phase::Building(_, _, true)) {
            self.land_import(dialog);
            return true;
        }
        if let Phase::Importing(going) = &dialog.phase {
            self.project.status = format!("Importing {}: {} \u{00b7} {} s", dialog.name, stage_text(&dialog.seen()).to_lowercase(), going.started.elapsed().as_secs());
        }
        self.import_dialogs.current = Some(dialog);
        true
    }

    /// The model, ready, into the scene: through `poll_imports`, as every
    /// import lands, after starting the new scene it was asked into.
    fn land_import(&mut self, dialog: ImportDialog) {
        let ImportDialog { name, path, phase, .. } = dialog;
        let Phase::Building(going, ready, _) = phase else { return };
        match going.new_scene {
            Some(changes) => {
                // Worked on since agreeing to leave it: asked again.
                if self.project.has_unsaved_changes() && self.project.changes() != changes && !self.confirm_unsaved() {
                    self.project.status = format!("{name} was not imported: the scene you have stays");
                    self.next_import();
                    return;
                }
                self.start_new_project();
            }
            None if self.project.document_epoch() != going.epoch => {
                self.project.status = format!("Discarded the import of {name} because the project changed");
                self.next_import();
                return;
            }
            None => {}
        }
        let (sender, receiver) = mpsc::channel();
        let _ = sender.send(Ok(Arrived::Prepared(*ready)));
        self.imports.push(PendingImport {
            document_epoch: self.project.document_epoch(),
            name,
            path,
            update: None,
            receiver,
            started: going.started,
            plan: Some(going.plan),
        });
        self.poll_imports();
        self.next_import();
    }

    /// The next file waiting, if any.
    fn next_import(&mut self) {
        self.import_dialogs.current = None;
        match self.import_dialogs.waiting.pop_front() {
            Some(path) => self.begin_import(path),
            None => self.import_dialogs.same_for_rest = false,
        }
    }

    /// Stop the import being asked about or under way; the scene is as it
    /// was.
    pub(crate) fn cancel_import(&mut self) {
        let Some(mut dialog) = self.import_dialogs.current.take() else { return };
        let was_going = dialog.importing();
        dialog.cancel();
        if was_going {
            self.project.status = format!("Import of {} cancelled; the scene is as it was", dialog.name);
        }
        self.next_import();
    }

    /// The dialog, each frame.
    pub(crate) fn import_dialog_window(&mut self, ctx: &egui::Context) {
        if self.pump_import_dialog() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        let replaces_starter = self.untouched_starter().is_some();
        let waiting = self.import_dialogs.waiting.len();
        let mut same = self.import_dialogs.same_for_rest;
        let Some(dialog) = self.import_dialogs.current.as_mut() else { return };
        let act = dialog.show(ctx, waiting, &mut same, replaces_starter);
        self.import_dialogs.same_for_rest = same;
        match act {
            Some(Act::Import) => self.confirm_import(),
            Some(Act::Cancel) => self.cancel_import(),
            None => {}
        }
    }

    /// What follows an import landing: for one from the dialog, the
    /// environment fitted and the camera pointed at the model as it asked;
    /// for any other, the camera framed on the scene, as ever.
    pub(crate) fn after_import(&mut self, plan: Option<Plan>, before: &BTreeSet<NodeId>) {
        let Some(plan) = plan else {
            self.frame_selection();
            return;
        };
        if plan.fit_environment {
            let commands = fit_environment(self.project.document());
            if !commands.is_empty() {
                self.project.begin_group();
                for command in commands {
                    let _ = self.project.apply(command);
                }
                self.project.end_group();
                self.submit();
            }
        }
        if plan.frame_camera {
            let document = self.project.document();
            let roots: Vec<NodeId> = document.tree.iter().filter(|n| !before.contains(&n.id) && n.parent.is_none_or(|p| before.contains(&p))).map(|n| n.id).collect();
            let mut bounds = tequila_scene::snapshot::visible_bounds_under(document, &roots);
            if bounds.is_empty() {
                bounds = tequila_scene::snapshot::visible_bounds(document);
            }
            if !bounds.is_empty() {
                let radius = (bounds.diagonal().length() * 0.5).max(1e-3);
                self.camera = crate::OrbitCamera::framing(bounds.center(), radius, self.aspect());
                self.sync_viewport_camera();
                self.submit();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// A folder of its own for each test, with a model in it.
    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tequila-import-dialog-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A box 120 x 60 x 30 in its numbers, with no unit: a part a CAD
    /// package wrote out in millimetres.
    fn bracket(folder: &std::path::Path) -> PathBuf {
        let path = folder.join("bracket.obj");
        let mut text = String::new();
        for i in 0..8 {
            text.push_str(&format!("v {} {} {}\n", if i & 1 == 0 { 0 } else { 120 }, if i & 2 == 0 { 0 } else { 60 }, if i & 4 == 0 { 0 } else { 30 }));
        }
        text.push_str("f 1 2 4 3\nf 5 7 8 6\nf 1 5 6 2\nf 3 4 8 7\nf 1 3 7 5\nf 2 6 8 4\n");
        std::fs::write(&path, text).unwrap();
        path
    }

    fn app() -> TequilaApp {
        let mut app = TequilaApp::for_tests();
        app.import_dialogs.remember = false;
        app
    }

    /// Wait for the dialog's import to land, as the frames would, the
    /// "Building the scene" frame taken as shown.
    fn finish(app: &mut TequilaApp) {
        let started = Instant::now();
        while app.import_dialogs.is_open() {
            if let Some(ImportDialog { phase: Phase::Building(_, _, shown), .. }) = app.import_dialogs.current.as_mut() {
                *shown = true;
            }
            app.pump_import_dialog();
            assert!(started.elapsed().as_secs() < 60, "the import never landed");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Wait for the file read ahead to be measured.
    fn measured(app: &mut TequilaApp) -> Preview {
        let started = Instant::now();
        loop {
            app.pump_import_dialog();
            if let Some(preview) = app.import_dialogs.current.as_ref().and_then(|d| d.preview) {
                return preview;
            }
            assert!(started.elapsed().as_secs() < 60, "it was never measured");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn model_size(app: &TequilaApp) -> tequila_core::math::Vec3 {
        let roots: Vec<NodeId> = app.project.document().tree.iter().filter(|n| n.name.starts_with("bracket") && n.parent.is_none()).map(|n| n.id).collect();
        tequila_scene::snapshot::visible_bounds_under(app.project.document(), &roots).diagonal()
    }

    /// The dialog's unit and up axis reach the importer: read as inches and
    /// stood Z up, the bracket comes in 120 x 30 x 60 inches, and nothing
    /// asks about its unit afterwards.
    #[test]
    fn the_unit_and_up_axis_chosen_reach_the_importer() {
        let folder = folder("options");
        let path = bracket(&folder);
        let mut app = app();
        app.begin_import_with(path, Choices::default());
        let preview = measured(&mut app);
        assert_eq!((preview.stated_unit, preview.guessed_unit), (None, LengthUnit::Millimetre));
        let dialog = app.import_dialogs.current.as_mut().unwrap();
        assert_eq!(dialog.size_line().as_deref(), Some("12 × 6 × 3 cm"), "the guess, before choosing");
        dialog.unit = Some(LengthUnit::Inch);
        dialog.up = UpAxis::Z;
        assert_eq!(dialog.size_line().as_deref(), Some("3.05 × 0.76 × 1.52 m"));
        app.confirm_import();
        finish(&mut app);
        let size = model_size(&app);
        let inch = 0.0254;
        assert!((size - vec3(120.0 * inch, 30.0 * inch, 60.0 * inch)).length() < 1e-3, "{size:?}");
        assert!(app.model_units.is_none(), "a unit chosen in the dialog was asked about again");
        let lowest = tequila_scene::snapshot::visible_bounds(app.project.document()).min.y;
        assert!(lowest.abs() < 1e-5, "not on the floor: {lowest}");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The guess the dialog showed is the size the model comes in at, and
    /// is not put to the person a second time; without the dialog, it is,
    /// as before.
    #[test]
    fn a_guess_seen_in_the_dialog_is_taken() {
        let folder = folder("guess");
        let path = bracket(&folder);
        let mut app = app();
        app.begin_import_with(path.clone(), Choices::default());
        measured(&mut app);
        app.confirm_import();
        finish(&mut app);
        assert!((model_size(&app) - vec3(0.12, 0.06, 0.03)).length() < 1e-5, "{:?}", model_size(&app));
        assert!(app.model_units.is_none());

        let mut app = self::app();
        app.begin_import_with(path, Choices { ask: false, ..Choices::default() });
        finish(&mut app);
        assert!((model_size(&app) - vec3(0.12, 0.06, 0.03)).length() < 1e-5);
        assert!(app.model_units.is_some(), "unseen, the guess is said");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Added to the scene, the model sits beside what is there; into a new
    /// scene, it is all there is, the old work gone and the lighting new.
    #[test]
    fn add_to_the_scene_and_a_new_scene() {
        let folder = folder("target");
        let path = bracket(&folder);
        let mut app = app();
        // More than the starter, so the first import keeps it; saved, so a
        // new scene asks nothing.
        let first = folder.join("first.obj");
        std::fs::write(&first, "v 0 0 0\nv 0.1 0 0\nv 0 0.1 0\nf 1 2 3\n").unwrap();
        app.begin_import_with(first, Choices { ask: false, ..Choices::default() });
        finish(&mut app);
        app.project
            .apply(Command::SetEnvironmentParameter { parameter: EnvironmentParameter::new(EnvParam::Brightness, EnvValue::Scalar(1.7)) })
            .unwrap();
        let names = |app: &TequilaApp| app.project.document().tree.iter().filter(|n| n.parent.is_none()).map(|n| n.name.clone()).collect::<Vec<_>>();
        let before = names(&app);
        assert!(before.iter().any(|n| n.starts_with("first")), "{before:?}");

        app.begin_import_with(path.clone(), Choices { ask: false, target: Target::Add, ..Choices::default() });
        finish(&mut app);
        let added = names(&app);
        assert!(added.iter().any(|n| n.starts_with("first")) && added.iter().any(|n| n.starts_with("bracket")), "{added:?}");
        assert_eq!(app.project.document().environment.brightness, 1.7);

        app.project.mark_clean();
        let epoch = app.project.document_epoch();
        app.begin_import_with(path, Choices { ask: false, target: Target::New, ..Choices::default() });
        finish(&mut app);
        assert_ne!(app.project.document_epoch(), epoch, "not a new scene");
        let fresh = names(&app);
        assert_eq!(fresh.len(), 1, "a new scene holds the model alone: {fresh:?}");
        assert!(fresh[0].starts_with("bracket"));
        assert_ne!(app.project.document().environment.brightness, 1.7, "the old scene's look came along");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Cancel, while it is read or once it is ready to build, leaves the
    /// document exactly as it was, and its history with it.
    #[test]
    fn cancel_leaves_the_document_unchanged() {
        let folder = folder("cancel");
        let path = bracket(&folder);
        let mut app = app();
        let before = app.project.document().clone();
        let changes = app.project.changes();

        // Under way.
        app.begin_import_with(path.clone(), Choices { ask: false, ..Choices::default() });
        assert!(app.import_dialogs.is_open());
        app.cancel_import();
        assert!(!app.import_dialogs.is_open());
        std::thread::sleep(std::time::Duration::from_millis(200));
        app.pump_import_dialog();
        app.poll_imports();
        assert!(app.project.document() == &before, "a cancelled import changed the document");
        assert_eq!(app.project.changes(), changes);

        // Read, prepared, about to go in.
        app.begin_import_with(path, Choices { ask: false, ..Choices::default() });
        let started = Instant::now();
        while !matches!(app.import_dialogs.current.as_ref().map(|d| &d.phase), Some(Phase::Building(..))) {
            app.pump_import_dialog();
            assert!(started.elapsed().as_secs() < 60);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.cancel_import();
        app.poll_imports();
        assert!(app.project.document() == &before, "a cancel once it was ready changed the document");
        assert_eq!(app.project.changes(), changes);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A CAD file's quality reaches the reader, and a model read ahead at
    /// another quality is read again at the one chosen rather than used.
    #[test]
    fn the_tessellation_chosen_reaches_the_reader() {
        let folder = folder("quality");
        let path = folder.join("two_bodies.step");
        std::fs::write(&path, include_bytes!("../../crates/tequila-import/tests/data/two_bodies.step")).unwrap();
        let triangles = |app: &TequilaApp| app.project.document().tree.iter().filter_map(|n| n.mesh()).filter_map(|m| app.project.document().geometry.get(m)).map(|m| m.indices.len() / 3).sum::<usize>();
        let recorded = |app: &TequilaApp| {
            let document = app.project.document();
            document.tree.iter().find(|n| n.parent.is_none() && n.metadata.contains_key(tequila_import::step::TESSELLATION)).and_then(|n| n.metadata.get(tequila_import::step::TESSELLATION).cloned())
        };

        let mut app = app();
        app.begin_import_with(path.clone(), Choices { tessellation: Tessellation::Draft, ..Choices::default() });
        measured(&mut app);
        app.confirm_import();
        finish(&mut app);
        let draft = triangles(&app);
        assert_eq!(recorded(&app).as_deref(), Some("Draft"));

        // Read ahead at Draft, imported at a custom chord as fine as High's.
        let mut app = self::app();
        app.begin_import_with(path, Choices { tessellation: Tessellation::Draft, ..Choices::default() });
        measured(&mut app);
        let dialog = app.import_dialogs.current.as_mut().unwrap();
        dialog.custom = (Tessellation::High.chord(), Tessellation::High.angle());
        dialog.choices.tessellation = Tessellation::custom(dialog.custom.0, dialog.custom.1);
        app.confirm_import();
        finish(&mut app);
        assert!(triangles(&app) > draft * 2, "{} triangles against Draft's {draft}", triangles(&app));
        assert!(recorded(&app).is_some_and(|r| r.starts_with("Custom 0.0005")), "{:?}", recorded(&app));
        // Kept, by default, to divide again.
        let kept = app.project.document().tree.iter().find_map(|n| n.metadata.get(tequila_scene::document::BREP).cloned());
        assert!(kept.is_some_and(|id| app.project.document().sources.get(&id).is_some()), "the CAD data was not kept");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Not kept when the person says so: the project carries no CAD data,
    /// and the model does not claim it.
    #[test]
    fn the_cad_data_is_not_kept_when_asked_not_to() {
        let folder = folder("keep");
        let path = folder.join("two_bodies.step");
        std::fs::write(&path, include_bytes!("../../crates/tequila-import/tests/data/two_bodies.step")).unwrap();
        let mut app = app();
        app.begin_import_with(path, Choices { ask: false, keep_cad: false, ..Choices::default() });
        finish(&mut app);
        let document = app.project.document();
        assert!(document.tree.iter().any(|n| n.mesh().is_some()), "nothing came in");
        assert!(document.sources.is_empty(), "the CAD data was kept");
        assert!(!document.tree.iter().any(|n| n.metadata.contains_key(tequila_scene::document::BREP)));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Fitting the environment: a ground of a set size fits the scene
    /// again, a dome of a set size takes the model's measure with its
    /// height in proportion, and an endless dome is left as it is.
    #[test]
    fn the_environment_is_fitted_to_the_model() {
        let mut app = app();
        assert!(fit_environment(app.project.document()).is_empty(), "the starter's environment fits already");
        let set = |param: EnvParam, value: f32| Command::SetEnvironmentParameter { parameter: EnvironmentParameter::new(param, EnvValue::Scalar(value)) };
        app.project.apply(set(EnvParam::GroundSize, 0.5)).unwrap();
        app.project.apply(set(EnvParam::Size, 40.0)).unwrap();
        app.project.apply(set(EnvParam::Height, 4.0)).unwrap();
        let reach = tequila_scene::snapshot::visible_bounds(app.project.document()).diagonal().length() * 3.0;
        for command in fit_environment(app.project.document()) {
            app.project.apply(command).unwrap();
        }
        let environment = &app.project.document().environment;
        assert_eq!(environment.ground.size, 0.0);
        assert!((environment.size - reach * 2.0).abs() < 1e-4, "{} against {}", environment.size, reach * 2.0);
        assert!((environment.height - 4.0 * environment.size / 40.0).abs() < 1e-4, "{}", environment.height);
        assert!(fit_environment(app.project.document()).is_empty(), "fitted twice");
    }

    /// Through the dialog: the fit and the camera follow the model in, the
    /// fit one step to undo; asked for neither, the camera stays put.
    #[test]
    fn the_camera_and_environment_follow_the_plan() {
        let folder = folder("plan");
        let path = bracket(&folder);
        let mut app = app();
        app.project
            .apply(Command::SetEnvironmentParameter { parameter: EnvironmentParameter::new(EnvParam::GroundSize, EnvValue::Scalar(0.05)) })
            .unwrap();
        let camera = app.camera;
        // Read as metres: 120 m across, nowhere near where the camera is.
        let mut metres = Choices { ask: false, ..Choices::default() };
        metres.set_unit_for("obj", Some(LengthUnit::Metre));
        app.begin_import_with(path.clone(), Choices { frame_camera: false, fit_environment: false, ..metres.clone() });
        finish(&mut app);
        assert_eq!((app.camera.target, app.camera.distance), (camera.target, camera.distance), "the camera moved");
        assert_eq!(app.project.document().environment.ground.size, 0.05, "the ground was fitted");

        app.begin_import_with(path, metres);
        finish(&mut app);
        let count = |app: &TequilaApp| app.project.document().tree.iter().filter(|n| n.parent.is_none() && n.name.starts_with("bracket")).count();
        assert_eq!(count(&app), 2);
        assert_eq!(app.project.document().environment.ground.size, 0.0, "the ground was not fitted");
        assert!(app.camera.distance > camera.distance * 10.0, "the camera did not move out to the model: {} from {}", app.camera.distance, camera.distance);
        // The fit is a step of its own, under the camera's: undone, the
        // model stays.
        for _ in 0..2 {
            if app.project.document().environment.ground.size != 0.05 {
                app.undo();
            }
        }
        assert_eq!(app.project.document().environment.ground.size, 0.05, "the fit did not undo");
        assert_eq!(count(&app), 2, "undoing the fit took the model");
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn counts_and_sizes_read_as_people_say_them() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(12_400), "12,400");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(size_of_file(2_400_000), "2.3 MB");
        assert_eq!(stage_text(&Seen { stage: Stage::Dividing, done: 1204, total: 5320, percent: None }), "Dividing surfaces: 1,204 of 5,320 faces");
    }
}
