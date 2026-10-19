//! The Tequila desktop application.
//!
//! At M1 this is the "real-time view" of `TEQUILA_SPEC.md` §1.2 plus the
//! Scene tab: a progressively path-traced viewport over a real
//! [`SceneDocument`](tequila_scene::SceneDocument), with the tree, the
//! transform gizmo, and import/save. The Material, Environment, Camera, Image
//! and Animation tabs arrive with the milestones that give them something to
//! show.
//!
//! The one property worth protecting from the very first version is §1.1: the
//! viewport **is** the renderer. There is no preview mode and no rasterized
//! approximation that later differs from the output — what is on screen is the
//! path tracer, converging.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod assistant;
mod axis_legend;
mod channel;
mod clock;
mod color_picker;
mod connect_claude;
mod console;
mod cutaway_panel;
mod cycles_view;
mod scripting_tab;
mod batch;
mod camera_tab;
mod environment_tab;
mod gizmo_ui;
mod label_gizmo;
mod graph_editor;
mod view_mode;
mod grid;
mod image_tab;
mod library_actions;
mod plugins;
mod plugins_ui;
mod library_dialogs;
mod library_tab;
mod mac;
mod lighting_tab;
mod material_tab;
mod measure_tool;
mod improve_model;
mod model_units;
mod metadata_panel;
mod network_panel;
mod decal_tab;
mod numeric;
mod orbit;
mod pin_gizmo;
mod pin_panel;
mod sky_panel;
mod orchestrator;
mod orient;
mod profile;
mod project;
mod recent;
mod render_frame;
mod render_window;
mod scene_tab;
mod procedural_gpu;
mod solid_gpu;
mod studio_tab;
mod configurator_panel;
mod templates_panel;
mod texture_pack;
mod field;
mod icons;
mod theme;
mod thumbnail;
mod toast;
mod ui_capture;
mod windows;
mod material_lab;

use eframe::egui;
use tequila_color::ImageStyle;
use tequila_core::math::Vec3;
use tequila_scene::gizmo::{GizmoMode, PivotMode};
use tequila_scene::selection::SelectionMode;
use tequila_scene::Command;
use orbit::{OrbitCamera, StandardView};
use orchestrator::{DisplayImage, Orchestrator, RenderRequest};

/// A file named on the command line: a project to open, or a model to import.
///
/// `tequila-app path/to/model.gltf` or `tequila-app scene.tequila`. Small,
/// but it is what lets a file manager, a shell, or another tool hand this
/// application something to look at — and what makes "try this model" a command
/// rather than a set of directions through a dialog.
fn file_from_arguments() -> Option<(Action, std::path::PathBuf)> {
    let path = std::path::PathBuf::from(std::env::args().nth(1)?);
    if !path.is_file() {
        return None;
    }
    let action = match path.extension().and_then(|e| e.to_str()) {
        // Freeshot, as it was called before, saved them as .freeshot.
        Some("tequila" | "freeshot") => Action::Open,
        _ => Action::Import,
    };
    Some((action, path))
}
use library_tab::LibraryTab;
use material_tab::MaterialTab;
use project::Project;
use scene_tab::SceneTab;
use std::sync::Arc;

fn main() -> eframe::Result<()> {
    // `--write-icon <file.png>`: the icon at 1024 pixels on a Mac
    // application's grid, which scripts/package-macos.sh makes the bundle's
    // .icns from - drawn by the same code as the window's icon.
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.get(1).is_some_and(|a| a == "--write-icon") {
        let Some(path) = arguments.get(2).map(std::path::PathBuf::from) else {
            eprintln!("usage: tequila-app --write-icon <file.png>");
            std::process::exit(2);
        };
        let icon = icons::app_icon_sized(1024, true);
        let written = std::fs::File::create(&path).map_err(|e| e.to_string()).and_then(|file| {
            let mut encoder = png::Encoder::new(file, icon.width, icon.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
            writer.write_image_data(&icon.rgba).map_err(|e| e.to_string())
        });
        if let Err(error) = written {
            eprintln!("{}: {error}", path.display());
            std::process::exit(1);
        }
        return Ok(());
    }
    // The application was called Freeshot before: on a computer that had
    // it, its things come across the first time.
    let adopted = tequila_format::library::adopt_older_name();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([
                std::env::var("TEQUILA_UI_WIDTH")
                    .ok()
                    .and_then(|width| width.parse::<f32>().ok())
                    .filter(|width| width.is_finite())
                    .unwrap_or(1440.0)
                    .clamp(640.0, 3840.0),
                900.0,
            ])
            .with_min_inner_size([640.0, 480.0])
            // Models, projects and texture packs can be dropped on the window.
            .with_drag_and_drop(true)
            .with_title("Tequila")
            .with_icon(icons::app_icon()),
        // The GPU Solid view draws depth-tested geometry into the viewport,
        // and smooth edges are most of what makes a solid view look finished.
        depth_buffer: 24,
        multisampling: 4,
        ..Default::default()
    };
    eframe::run_native(
        "Tequila",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            mac::quit_asks_first(cc);
            {
                let mut app = TequilaApp::new();
                if !adopted.is_empty() {
                    app.toasts.info("Brought your materials, plugins and settings across from Freeshot".to_string());
                }
                app.solid = solid_gpu::SolidView::new(cc.gl.is_some());
                // Picked up by the first frame's submit.
                app.submit();
                Ok(Box::new(app))
            }
        }),
    )
}

struct TequilaApp {
    orchestrator: Orchestrator,
    camera: OrbitCamera,
    texture: Option<egui::TextureHandle>,
    image: DisplayImage,
    seen_generation: u64,

    project: Project,
    scene_tab: SceneTab,
    material_tab: MaterialTab,
    library: LibraryTab,
    /// Plugins: what is installed and on, and the code that is running.
    plugins: plugins::Plugins,
    plugins_window: plugins_ui::PluginsWindow,
    /// The selection the plugins were last told about.
    plugin_selection: Vec<u64>,
    /// File → Open recent, newest first, and where it is kept (none in a
    /// test, which must not touch the person's list).
    recent: Vec<std::path::PathBuf>,
    recent_file: Option<std::path::PathBuf>,
    /// The selection and the snapshot's rebuild count the outline was last
    /// worked out for.
    outline_key: Option<(Vec<tequila_scene::tree::NodeId>, u64)>,
    /// Whether the scripting channel has been given a way to wake the UI.
    channel_wakes: bool,
    /// A material picked up from the library and not yet dropped (§04 §3.1).
    dragging: Option<Box<tequila_format::library::LibraryEntry>>,
    show_library: bool,
    /// §2.2's axis legend, bottom-left.
    show_axis_legend: bool,
    /// `Space`: the Project window (§6.2).
    show_project: bool,
    /// `Tab`: everything hidden but the viewport (§6.2).
    full_viewport: bool,
    /// Presentation mode (`TEQUILA_SPEC.md` §1.13), and what it put aside
    /// to come back to.
    presenting: Option<Presentation>,
    /// Draw the parts' own edges in Solid and Material Preview.
    show_model_edges: bool,
    /// The viewport at the render's own shape, so nothing in view is cut
    /// from the render and nothing out of view appears in it.
    show_render_frame: bool,
    /// Full screen asked for, for the next frame to send.
    fullscreen_request: Option<bool>,
    /// Every configuration, waiting on a yes before it is queued.
    batch_plan: Option<BatchPlan>,
    /// The web export's choices, while its window is open.
    web_plan: Option<WebPlan>,
    /// A web export being written on another thread.
    web_export: Option<std::thread::JoinHandle<Result<tequila_format::web::WebReport, String>>>,
    /// The spin viewer's choices, while its window is open.
    spin_plan: Option<SpinPlan>,
    /// Spin viewers whose frames are rendering, finished when they are in.
    spin_pending: Vec<SpinPending>,
    toasts: toast::Toasts,
    /// The node a right-click landed on, or `None` for empty space.
    context_target: Option<tequila_scene::tree::NodeId>,
    /// The parts a material being dragged would land on, outlined live.
    ///
    /// §3.1 step 2 asks for the part under the cursor to be highlighted during
    /// a drag. It reuses the selection outline rather than inventing a second
    /// highlight, so what a person sees before dropping is exactly the shape
    /// they will see selected after.
    drag_highlight: Vec<u32>,
    /// Which page of the Project window is showing.
    project_tab: ProjectTab,
    /// A unit change waiting for the user to say which conversion they mean.
    pending_unit: Option<tequila_core::units::LengthUnit>,
    /// Files being read on other threads, each applied when it arrives. A
    /// STEP assembly takes seconds to mesh, and the window stayed frozen
    /// for all of them while it was read here.
    imports: Vec<PendingImport>,
    /// What the last model update found, shown until dismissed.
    update_report: Option<tequila_scene::reimport::ReimportReport>,
    /// The Model units window, when it is open.
    model_units: Option<model_units::ModelUnits>,
    /// The Improve model window, when it is open.
    improve_model: Option<improve_model::ImproveModel>,
    /// Path traced, or the fast preview. A view setting, not a document one:
    /// it changes nothing that gets saved and nothing that gets rendered when
    /// someone presses Render.
    shading: orchestrator::Shading,
    /// Which of the four shadings the viewport shows; `shading` follows it.
    view_mode: view_mode::ViewMode,
    /// A light, dark or theme change from the scripting channel, for the
    /// next frame to apply.
    pending_look: Option<theme::Appearance>,
    /// The appearance picker opened or closed from the scripting channel.
    open_look: Option<bool>,
    /// The fast view D goes back to from Rendered.
    last_preview_mode: view_mode::ViewMode,
    /// The scripting channel, when one could be opened.
    ///
    /// `docs/04` §8 wants everything clickable to be scriptable; this is the
    /// socket that makes it reachable from outside the process. `None` when
    /// the port could not be bound, which costs scriptability for the session
    /// and nothing else — a renderer that refused to start over it would be
    /// the worse tool.
    channel: Option<tequila_scripting::server::Channel>,
    /// A screenshot the channel asked for, waiting for egui to hand the
    /// framebuffer over on a later frame.
    pending_screenshot: Option<std::path::PathBuf>,
    console: console::Console,
    graph_editor: graph_editor::GraphEditor,
    /// The Solid viewport on the GPU, when OpenGL allows.
    solid: solid_gpu::SolidView,
    /// What `solid` was last told about, so a GPU failure resubmits once.
    gpu_solid_submitted: bool,
    /// Instance ids to ring, shared by both Solid paths.
    outlined: Vec<u32>,
    assistant: assistant::Assistant,
    /// Set by a scripted `quit` that has already settled unsaved work, so the
    /// close goes through without asking a question nobody is there to answer.
    quit_requested: bool,
    /// Whether the person agreed to this close (or there was nothing to ask
    /// about). An application ended without it - a Mac's Quit from the Dock,
    /// logging out - keeps the crash journal, so the work comes back.
    close_approved: bool,
    /// Whether the window has been fitted to its screen yet.
    fitted: bool,
    /// The selection before the viewport's last click, for a double-click
    /// to step up from (`select_enclosing`).
    selected_before_click: Vec<tequila_scene::tree::NodeId>,
    /// Where the viewport is on screen, for a screenshot of just the picture.
    viewport_rect: egui::Rect,
    /// Whether the pending screenshot is of the viewport rather than the window.
    screenshot_viewport: bool,
    /// The window of its own the pending screenshot is of, if not the main one.
    screenshot_window: Option<windows::Kind>,
    /// Whether the windows open as windows of their own, which a platform
    /// may not allow; read from egui each frame.
    native_windows: bool,
    /// The mode the main window's title bar was last set to.
    title_bar_dark: Option<bool>,
    /// Every request the channel has served, for the person watching.
    activity: scripting_tab::Activity,
    /// Whether egui has already been asked for the frame above. Without this
    /// the request would be repeated every frame until the event arrived,
    /// which is a queue of captures for one caller who wanted a picture.
    screenshot_requested: bool,
    gizmo: gizmo_ui::GizmoUi,
    label_gizmo: label_gizmo::LabelGizmo,
    pin_gizmo: pin_gizmo::PinGizmo,
    measure: measure_tool::MeasureTool,
    studio_tab: studio_tab::StudioTab,
    templates_panel: templates_panel::TemplatesPanel,

    environment_tab: environment_tab::EnvironmentTab,
    camera_tab: camera_tab::CameraTab,
    image_tab: image_tab::ImageTab,
    lighting_tab: lighting_tab::LightingTab,
    /// The style last handed to the render thread, so a change is noticed once
    /// rather than every frame. The *document* owns the style; this is only
    /// what the display pipeline has already been told.
    published_style: ImageStyle,

    viewport_size: (usize, usize),
    /// Render pixels per point of `viewport_size` (`render_scale`).
    render_scale: f32,
    /// The scene generation the render thread was last told about, so an edit
    /// restarts the render exactly once.
    submitted_generation: u64,
    interacting: bool,
    show_help: bool,
    show_about: bool,
    show_hud: bool,
    viewport_paused: bool,
    window_title: String,
    /// What the viewport last said about its engine, and the fallback the
    /// person has already been told about, so the notice comes once.
    engine_note: Option<orchestrator::EngineNote>,
    told_fallback: Option<String>,
    ui_capture: ui_capture::UiCapture,
    render_window: render_window::RenderWindow,
    batch: batch::BatchRenderer,
    /// Sessions from a previous run that crashed (§4.5), waiting on the user's
    /// answer. Empty once they have decided.
    recoverable: Vec<tequila_format::journal::Recoverable>,
    /// The document the panels last saw, by its epoch: when another one is
    /// opened, what they hold about the old one is let go.
    seen_epoch: u64,
}

/// The pages of the Project window, in the order `docs/04` §1.1 lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProjectTab {
    Scene,
    Material,
    Environment,
    Lighting,
    Camera,
    Image,
    Studios,
    Scripting,
}

impl ProjectTab {
    const ALL: [ProjectTab; 8] = [
        ProjectTab::Scene,
        ProjectTab::Material,
        ProjectTab::Environment,
        ProjectTab::Lighting,
        ProjectTab::Camera,
        ProjectTab::Image,
        ProjectTab::Studios,
        ProjectTab::Scripting,
    ];

    fn label(self) -> &'static str {
        match self {
            ProjectTab::Scene => "Scene",
            ProjectTab::Material => "Material",
            ProjectTab::Environment => "Environment",
            ProjectTab::Lighting => "Lighting",
            ProjectTab::Camera => "Camera",
            ProjectTab::Image => "Image",
            ProjectTab::Studios => "Studios",
            ProjectTab::Scripting => "Scripting",
        }
    }

    fn icon(self) -> icons::Icon {
        match self {
            ProjectTab::Scene => icons::Icon::Scene,
            ProjectTab::Material => icons::Icon::Material,
            ProjectTab::Environment => icons::Icon::Environment,
            ProjectTab::Lighting => icons::Icon::Light,
            ProjectTab::Camera => icons::Icon::Camera,
            ProjectTab::Image => icons::Icon::Image,
            ProjectTab::Studios => icons::Icon::Studios,
            ProjectTab::Scripting => icons::Icon::Script,
        }
    }

    /// The page's own colour, on its rail icon and its heading.
    fn hue(self) -> theme::Hue {
        match self {
            ProjectTab::Scene => theme::Hue::Blue,
            ProjectTab::Material => theme::Hue::Pink,
            ProjectTab::Environment => theme::Hue::Green,
            ProjectTab::Lighting => theme::Hue::Amber,
            ProjectTab::Camera => theme::Hue::Violet,
            ProjectTab::Image => theme::Hue::Orange,
            ProjectTab::Studios => theme::Hue::Sky,
            ProjectTab::Scripting => theme::Hue::Slate,
        }
    }

    /// What the tab is for, in a line: under its title, and on its rail
    /// icon's tooltip.
    fn hint(self) -> &'static str {
        match self {
            ProjectTab::Scene => "Organize parts and position your model",
            ProjectTab::Material => "Select a part to edit its surface",
            ProjectTab::Environment => "Shape reflections and the background",
            ProjectTab::Lighting => "Control light transport and quality",
            ProjectTab::Camera => "Compose the shot and set focus",
            ProjectTab::Image => "Develop the image without restarting the render",
            ProjectTab::Studios => "Keep versions of the shot and switch between them",
            ProjectTab::Scripting => "Drive the application from a script, and see what has",
        }
    }
}

/// A unit's name, spelled out rather than abbreviated.
///
/// "Millimetres" rather than "mm" in the one place the unit is being *chosen*:
/// the abbreviation is right on a field, where it follows a number and the
/// context is obvious, and ambiguous in a list where the reader is deciding.
fn unit_label(unit: tequila_core::units::LengthUnit) -> &'static str {
    use tequila_core::units::LengthUnit;
    match unit {
        LengthUnit::Unspecified => "Unspecified",
        LengthUnit::Millimetre => "Millimetres",
        LengthUnit::Centimetre => "Centimetres",
        LengthUnit::Metre => "Metres",
        LengthUnit::Inch => "Inches",
        LengthUnit::Foot => "Feet",
    }
}

/// The notice when the viewport could not use Cycles: what happened, what
/// it does instead, and what usually fixes it, with Cycles' own words last.
fn fallback_notice(why: &str) -> String {
    format!(
        "The graphics card could not render with Cycles, so the viewport uses Tequila's own renderer on the \
         processor. Updating the graphics driver usually fixes this; NVIDIA cards older than the GTX 16 and RTX 20 \
         series are not supported. Final renders fall back the same way. (Cycles: {why})"
    )
}

/// The viewport badge while Cycles has shown nothing yet: its own status,
/// shortened, and a word on why the first time is slow.
fn preparing_label(doing: &str) -> String {
    let doing = doing.trim();
    let lower = doing.to_lowercase();
    if lower.contains("kernel") || lower.contains("compil") {
        "preparing the graphics card (first time only, can take a few minutes)".to_string()
    } else if doing.is_empty() {
        "starting Cycles".to_string()
    } else {
        let mut short: String = doing.chars().take(60).collect();
        if doing.chars().count() > 60 {
            short.push('…');
        }
        short.to_lowercase()
    }
}

/// What the path prompt is collecting a path for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Import,
    Open,
    SaveAs,
}

/// Where the journal and auto-save snapshots live (§4.5).
fn journal_directory() -> std::path::PathBuf {
    // Disposable QA sessions must not rewrite the user's persisted queue.
    if let Some(directory) = std::env::var_os("TEQUILA_JOURNAL_DIR") {
        return directory.into();
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".tequila").join("journal")
}

/// Hide the selection, and show everything again. A Mac's Cmd+H hides the
/// application, and never reaches Tequila: there it is Control+H.
const HIDE_KEYS: &str = if cfg!(target_os = "macos") { "Control + H / Alt + H" } else { "Ctrl + H / Alt + H" };

/// The group a double-click on `node` selects: the one it is in, or - when
/// the selection before the click was one of its groups - the group around
/// that, up to the outermost. `None` for a part in no group.
fn enclosing_group(
    document: &tequila_scene::SceneDocument,
    node: tequila_scene::tree::NodeId,
    before: &[tequila_scene::tree::NodeId],
) -> Option<tequila_scene::tree::NodeId> {
    let enclosing = document.tree.ancestors(node);
    let parent = *enclosing.first()?;
    let next = match before {
        [level] => enclosing.iter().position(|n| n == level).map(|at| enclosing[(at + 1).min(enclosing.len() - 1)]),
        _ => None,
    };
    Some(next.unwrap_or(parent))
}

/// The window's size on a screen of `monitor` points, if `inner` does not
/// fit: room is left for the menu bar, title bar and Dock or taskbar.
fn fitted_window(inner: egui::Vec2, monitor: egui::Vec2) -> Option<egui::Vec2> {
    let room = egui::vec2(monitor.x * 0.94, monitor.y * 0.86);
    (inner.x > room.x || inner.y > room.y).then(|| inner.min(room).max(egui::vec2(640.0, 480.0)))
}

/// Render pixels per point of viewport.
///
/// A screen with more than one pixel per point - a Mac's Retina screen, a
/// Windows screen set above 100% - takes egui's pixels per point, the text
/// size's zoom included, so a pixel of the picture is a pixel of the screen.
/// Rendered a pixel per point, it was a half-size picture stretched over a
/// Retina viewport: soft beside the panels around it, drawn at the screen's
/// own resolution.
///
/// A screen of one pixel per point keeps one, whatever the text size: its
/// picture, and what it costs, stay as they always were.
fn render_scale(native_pixels_per_point: Option<f32>, pixels_per_point: f32) -> f32 {
    if native_pixels_per_point.unwrap_or(1.0) > 1.0 { pixels_per_point } else { 1.0 }
}

/// The render's size, in pixels, for a viewport `points` across.
fn render_pixels(points: (usize, usize), scale: f32) -> (usize, usize) {
    let pixels = |n: usize| ((n as f32 * scale).round() as usize).max(1);
    (pixels(points.0), pixels(points.1))
}

/// The guides on the Help menu: title, the file beside a packaged Tequila,
/// its source in `docs`, and what it is about.
const GUIDES: [(&str, &str, &str, &str); 3] = [
    ("Scripting and AI apps", "SCRIPTING.md", "SCRIPTING.md", "Driving Tequila from Python, the console, Claude and other AI apps"),
    ("Writing plugins", "PLUGINS.md", "plugins.md", "Panels, commands and automations of your own, in Python"),
    ("Making materials", "MATERIALS.md", "materials-authoring.md", "Library materials, recipes and material packages"),
];

/// A file shipped beside the application (`packaged`), or - run from a
/// checkout - `source` in the checkout it was built from.
fn shipped_file(packaged: &str, source: &str) -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?;
    shipped_in(folder, packaged).or_else(|| folder.ancestors().map(|a| a.join(source)).find(|p| p.is_file()))
}

/// A file shipped with the application: beside the executable, or in a Mac
/// app bundle - whose executable is in `Contents/MacOS`, where only code may
/// go - in the bundle's `Contents/Resources`.
pub(crate) fn beside_application(name: &str) -> Option<std::path::PathBuf> {
    shipped_in(std::env::current_exe().ok()?.parent()?, name)
}

fn shipped_in(folder: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let beside = folder.join(name);
    if beside.is_file() {
        return Some(beside);
    }
    let resources = folder.parent()?.join("Resources").join(name);
    (folder.file_name()? == "MacOS" && resources.is_file()).then_some(resources)
}

/// What `BUILD.txt` beside a packaged Tequila says: when it was built, and
/// from which revision. `None` for a build run from its checkout.
fn build_note() -> Option<(String, String)> {
    read_build_note(&std::fs::read_to_string(beside_application("BUILD.txt")?).ok()?)
}

fn read_build_note(text: &str) -> Option<(String, String)> {
    // Windows PowerShell writes it with a byte-order mark.
    let text = text.trim_start_matches('\u{feff}');
    let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string());
    let built = field("Built:")?;
    let source = field("Source:").unwrap_or_default();
    Some((built, source))
}

/// The scene every session starts from.
///
/// Opening on an empty document would show a blank viewport, which says
/// nothing about whether the renderer works. The demo scene is a real
/// document — importable, selectable, saveable — so the first frame
/// exercises the same path everything else does. A crashed session that was
/// never saved is recovered onto it too, since its journal starts from it.
fn starter_document() -> tequila_scene::SceneDocument {
    tequila_scene::demo::demo_document_sized(tequila_scene::demo::DemoMaterial::Plastic, 0.18, tequila_scene::demo::STARTER_RADIUS)
}

/// Unique per run, so two open windows cannot interleave into one journal.
fn session_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{}-{now}", std::process::id())
}

impl TequilaApp {
    fn new() -> Self {
        let environment = Arc::new(tequila_scene::demo::studio_environment());
        let mut project = Project::with_document(starter_document(), environment);

        // Skies and environment images are built off the UI thread, so a sky
        // slider can be dragged without stalling every frame.
        project.set_environment_asynchronous(true);

        let directory = journal_directory();
        let id = session_id();
        // Scanned *before* this session's own journal is created, so the list
        // cannot include the session doing the scanning.
        let recoverable = tequila_format::journal::recoverable(&directory, &id);
        project.begin_session(&directory, &id);

        let mut app = TequilaApp::assemble(project, recoverable);
        let recent_file = recent::file();
        app.recent = recent::read(&recent_file);
        app.recent_file = Some(recent_file);
        // Renders from earlier sessions, beside the queue and the journal.
        app.render_window.remember_in(&directory);
        // The AI providers the person has connected, beside the other state.
        if let Some(home) = directory.parent() {
            app.assistant.load(home.join("assistant.json"));
        }
        let finished = app.project.queue().jobs().to_vec();
        app.render_window.remember_finished(&finished);
        app.start_up();
        app
    }

    /// An application around a document, with nothing else: no journal, no
    /// socket, no file from the command line. What the scripting handler's
    /// tests drive, so they exercise the real handler on a real application
    /// without touching the user's home directory or opening a port.
    #[cfg(test)]
    pub(crate) fn for_tests() -> Self {
        let project = Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.18),
            Arc::new(tequila_scene::demo::studio_environment()),
        );
        TequilaApp::assemble(project, Vec::new())
    }

    fn assemble(project: Project, recoverable: Vec<tequila_format::journal::Recoverable>) -> Self {
        let mut app = TequilaApp {
            orchestrator: Orchestrator::new(),
            camera: OrbitCamera::framing(
                Vec3::new_y(tequila_scene::demo::STARTER_RADIUS),
                tequila_scene::demo::STARTER_RADIUS * 1.25,
                16.0 / 9.0,
            ),
            texture: None,
            image: DisplayImage::default(),
            seen_generation: 0,
            project,
            scene_tab: SceneTab::default(),
            material_tab: MaterialTab::default(),
            // Thumbnails have a studio of their own, lit to tell materials
            // apart at ninety pixels rather than to light a product.
            library: LibraryTab::new(std::sync::Arc::new(
                tequila_scene::demo::preview_environment(),
            )),
            // A test application has no plugins of the person's.
            plugins: if cfg!(test) {
                plugins::Plugins::load_from(Vec::new(), std::env::temp_dir().join(format!("tequila-test-plugins-{}.json", std::process::id())))
            } else {
                plugins::Plugins::load()
            },
            plugins_window: Default::default(),
            plugin_selection: Vec::new(),
            recent: Vec::new(),
            recent_file: None,
            outline_key: None,
            channel_wakes: false,
            dragging: None,
            show_library: true,
            show_axis_legend: true,
            show_project: true,
            full_viewport: false,
            presenting: None,
            show_model_edges: false,
            show_render_frame: true,
            fullscreen_request: None,
            batch_plan: None,
            web_plan: None,
            web_export: None,
            spin_plan: None,
            spin_pending: Vec::new(),
            toasts: toast::Toasts::default(),
            context_target: None,
            drag_highlight: Vec::new(),
            project_tab: ProjectTab::Scene,
            pending_unit: None,
            imports: Vec::new(),
            update_report: None,
            model_units: None,
            improve_model: None,
            shading: orchestrator::Shading::default(),
            view_mode: view_mode::ViewMode::default(),
            pending_look: None,
            open_look: None,
            last_preview_mode: view_mode::ViewMode::default(),
            channel: None,
            pending_screenshot: None,
            console: console::Console::default(),
            graph_editor: graph_editor::GraphEditor::default(),
            solid: solid_gpu::SolidView::new(false),
            gpu_solid_submitted: false,
            outlined: Vec::new(),
            assistant: assistant::Assistant::default(),
            quit_requested: false,
            close_approved: false,
            fitted: false,
            selected_before_click: Vec::new(),
            viewport_rect: egui::Rect::NOTHING,
            screenshot_viewport: false,
            screenshot_window: None,
            native_windows: false,
            title_bar_dark: None,
            activity: scripting_tab::Activity::default(),
            screenshot_requested: false,
            gizmo: gizmo_ui::GizmoUi::default(),
            label_gizmo: label_gizmo::LabelGizmo::default(),
            pin_gizmo: pin_gizmo::PinGizmo::default(),
            measure: measure_tool::MeasureTool::default(),
            studio_tab: studio_tab::StudioTab::default(),
            templates_panel: templates_panel::TemplatesPanel::default(),
            environment_tab: Default::default(),
            camera_tab: Default::default(),
            image_tab: Default::default(),
            lighting_tab: Default::default(),
            published_style: ImageStyle::default(),
            viewport_size: (0, 0),
            render_scale: 1.0,
            submitted_generation: 0,
            interacting: false,
            show_help: false,
            show_about: false,
            show_hud: false,
            viewport_paused: false,
            window_title: String::new(),
            engine_note: None,
            told_fallback: None,
            ui_capture: Default::default(),
            render_window: Default::default(),
            batch: batch::BatchRenderer::new(),
            recoverable,
            seen_epoch: 0,
        };

        // A file on the command line replaces the demo scene. Done after the
        // app is otherwise built so it goes through exactly the same path a
        // dialog would, framing and status line included.
        // The opening view is document state like any other camera, so it goes
        // through the bus — and then the document is declared clean, because
        // nobody has edited anything yet.
        app.sync_viewport_camera();
        app.project.mark_clean();
        app
    }

    /// Everything a real launch does beyond building the application: the
    /// file on the command line, the scripting channel, capture hooks.
    fn start_up(&mut self) {
        let app = self;
        if let Some((action, path)) = file_from_arguments() {
            // Start from nothing, not from the demo. Importing alongside it
            // leaves a 40-metre ground plane in the scene, and framing the
            // camera on everything then makes a pair of glasses 140 mm across
            // a speck in the middle of the screen — which reads as "the model
            // did not load".
            app.project.reset();
            app.handle_file(action, path);
        }
        // Last, and never fatal: an application that would not open
        // because a port was busy is a worse tool than one that is simply not
        // scriptable this session. `TEQUILA_NO_CHANNEL` turns it off for
        // anyone who would rather not have a socket at all.
        if std::env::var_os("TEQUILA_NO_CHANNEL").is_none() {
            match tequila_scripting::server::Channel::start() {
                Ok(open) => {
                    app.project.status = format!(
                        "Scripting channel ready on port {} — see ~/.tequila/channel.json",
                        open.port()
                    );
                    app.channel = Some(open);
                }
                Err(error) => app.project.status = error,
            }
        }

        if std::env::var_os("TEQUILA_UI_CAPTURE").is_some() && std::env::var_os("TEQUILA_UI_MATERIAL").is_some() {
            let part = app.project.document().tree.iter().find_map(|node| node.material.map(|_| node.id));
            if let Some(part) = part {
                app.project.selection.select_many([part], tequila_scene::selection::SelectionMode::Replace);
                app.project_tab = ProjectTab::Material;
                app.camera_tab.show_grid = std::env::var_os("TEQUILA_UI_GRID").is_some();
                // TEQUILA_UI_LABEL=<png> adds a label to the selected part
                // and arms its placement, so a capture can show the frame and
                // the mode banner. A label otherwise arrives only through a
                // file dialog, which a screenshot run cannot open.
                if let Some(artwork) = std::env::var_os("TEQUILA_UI_LABEL") {
                    app.add_capture_label(part, std::path::PathBuf::from(artwork));
                }
                // TEQUILA_UI_TAB picks the inspector tab, so a capture can
                // show a panel other than Material.
                if let Some(tab) = std::env::var_os("TEQUILA_UI_TAB") {
                    if tab.to_string_lossy().eq_ignore_ascii_case("scripting") {
                        app.project_tab = ProjectTab::Scripting;
                        app.console.show_and_focus();
                    }
                    if tab.to_string_lossy().eq_ignore_ascii_case("environment") {
                        app.project_tab = ProjectTab::Environment;
                    }
                }
                // TEQUILA_UI_PINS=<preset> applies a lighting preset, so a
                // capture can show the pin controls and what they do to the
                // render. Pins otherwise arrive by clicking a button.
                // Pins light the scene through the environment, which Solid
                // mode does not use — it shades from a fixed rig. A capture
                // meant to show what a pin does has to be in Rendered mode.
                if std::env::var_os("TEQUILA_UI_RENDERED").is_some() {
                    app.view_mode = view_mode::ViewMode::Rendered;
                    app.shading = orchestrator::Shading::Rendered;
                }
                // TEQUILA_UI_SKY=<time of day> turns the sky on, so a
                // capture can show what it does.
                if let Some(when) = std::env::var_os("TEQUILA_UI_SKY") {
                    let sky = sky_panel::preset(&when.to_string_lossy());
                    let _ = app.project.apply(Command::SetSky {
                        sky: Box::new(Some(sky)),
                    });
                }
                if let Some(name) = std::env::var_os("TEQUILA_UI_PINS") {
                    let wanted = name.to_string_lossy().to_ascii_lowercase();
                    if let Some((_, _, pins)) = pin_panel::studio_presets()
                        .into_iter()
                        .find(|(preset, _, _)| preset.to_ascii_lowercase() == wanted)
                    {
                        let _ = app
                            .project
                            .apply(Command::SetEnvironmentPins { pins });
                    }
                }
            }
        }
    }

    /// The viewport's aspect ratio, for every control that is stated against
    /// the frame rather than against the sensor.
    fn aspect(&self) -> f32 {
        let (width, height) = self.viewport_size;
        if height == 0 {
            return 16.0 / 9.0;
        }
        width as f32 / height as f32
    }

    fn submit(&mut self) {
        if self.viewport_size.0 == 0 || self.viewport_size.1 == 0 {
            return;
        }
        let (width, height) = render_pixels(self.viewport_size, self.render_scale);
        let settings = self.project.render_settings().clone();
        self.orchestrator.submit(RenderRequest {
            camera: self.camera.camera,
            width,
            height,
            config: self.project.integrator_config(),
            scene: self.project.scene(),
            scene_generation: self.project.generation(),
            // The viewport and the final render denoise with the same
            // settings, from the document. Two sets of controls for one
            // denoiser would mean a preview that does not look like the render
            // it is previewing, which is the one thing a preview must not do.
            denoise: settings.denoise,
            denoise_strength: settings.denoise_strength,
            denoise_cadence: settings.denoise_cadence,
            prefilter_guides: Some(settings.prefilter_guides),
            reject_fireflies: settings.reject_fireflies,
            clamp_sample: settings.clamp_indirect,
            interacting: self.interacting,
            gpu_solid: self.gpu_solid(),
            shading: self.shading,
            engine: settings.engine,
            pixels_per_point: self.render_scale,
        });
        self.gpu_solid_submitted = self.gpu_solid();
    }

    /// Show `mode` in the viewport.
    fn set_view_mode(&mut self, mode: view_mode::ViewMode) {
        if self.view_mode == mode {
            return;
        }
        self.view_mode = mode;
        self.shading = mode.shading();
        self.submit();
    }

    /// Whether Solid is drawn by the GPU this frame.
    fn gpu_solid(&self) -> bool {
        self.shading == orchestrator::Shading::Fast
            && self.solid.usable()
            && !matches!(self.camera.camera.projection, tequila_render_core::camera::Projection::Equirectangular)
    }
}

impl TequilaApp {
    /// Write the viewport's view into the active saved camera.
    ///
    /// The orbit controls and the Camera tab edit the same camera, so one of
    /// them has to be the writer and the other the reader on any given frame.
    /// Orbiting writes here; the tab reads what it finds and writes back only
    /// when a control actually moved. Without this the tab would show the
    /// camera as it was when it was saved, and the first touch of any slider
    /// would teleport the view back there.
    ///
    /// One command per changed frame, which `Command::can_merge_with` folds
    /// into a single undo entry for the whole drag.
    fn sync_viewport_camera(&mut self) {
        let saved = self.project.camera();
        if saved.camera == self.camera.camera && saved.target == self.camera.target {
            return;
        }
        let index = self.project.document().cameras.active_index();
        let name = saved.name.clone();
        let _ = self.project.apply(Command::SetCamera {
            index,
            camera: Box::new(self.camera.to_saved(name)),
        });
    }

    /// Undo, with the view following the document's camera.
    ///
    /// The viewport writes its own camera into the document every frame
    /// ([`TequilaApp::sync_viewport_camera`]), so an undone orbit, studio
    /// switch or rescale that moved the camera was written straight back, as
    /// a new step that also threw the redo away. The scripting channel has
    /// always followed the camera after an undo; the menu and the keys did
    /// not.
    fn undo(&mut self) {
        let before = self.project.camera().clone();
        self.project.undo();
        self.follow_camera(&before);
    }

    /// Redo, likewise.
    fn redo(&mut self) {
        let before = self.project.camera().clone();
        self.project.redo();
        self.follow_camera(&before);
    }

    fn follow_camera(&mut self, before: &tequila_scene::lighting::SavedCamera) {
        if self.project.camera() != before {
            self.camera.adopt(self.project.camera());
            self.submit();
        }
    }
}

/// What an import's own thread hands over.
enum Arrived {
    /// A model read, for an update to match against what is in the scene.
    Read(tequila_import::ReadModel),
    /// A new model, prepared there up to needing the document: the slow
    /// part of an import, kept off the window's thread.
    Prepared(project::PreparedImport),
}

/// A model file being read on another thread.
struct PendingImport {
    document_epoch: u64,
    /// The file's name, for the status line.
    name: String,
    /// The file, for the plugins told it was imported.
    path: std::path::PathBuf,
    /// The model it updates, by its top nodes; `None` for a new import.
    update: Option<Vec<tequila_scene::tree::NodeId>>,
    receiver: std::sync::mpsc::Receiver<Result<Arrived, tequila_import::ImportError>>,
    /// When it started, for the status line's running time.
    started: std::time::Instant,
}

/// What Presentation mode put aside, to put back.
struct Presentation {
    full_viewport: bool,
    view_mode: view_mode::ViewMode,
    show_hud: bool,
    show_axis_legend: bool,
}

/// The spin viewer's choices.
#[derive(Clone)]
struct SpinPlan {
    options: tequila_format::spin::SpinOptions,
    transparent: bool,
}

/// A spin viewer whose frames are rendering.
struct SpinPending {
    folder: std::path::PathBuf,
    title: String,
    options: tequila_format::spin::SpinOptions,
    width: usize,
    height: usize,
    jobs: Vec<u64>,
}

/// The web export's choices.
#[derive(Clone)]
struct WebPlan {
    live: bool,
    photos: bool,
}

/// A batch of configurations, waiting for a yes.
#[derive(Clone)]
struct BatchPlan {
    combinations: usize,
    capped: bool,
    /// Seconds for one, if anything says.
    each: Option<f64>,
    summary: String,
}

impl eframe::App for TequilaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let frame_started = std::time::Instant::now();
        struct FrameTimer(std::time::Instant);
        impl Drop for FrameTimer {
            fn drop(&mut self) {
                profile::record("ui: frame", self.0.elapsed().as_secs_f64() * 1000.0);
            }
        }
        let _frame = FrameTimer(frame_started);
        // The palette follows egui's theme — the desktop's, in Auto. A look
        // asked for over the scripting channel is applied here, where the
        // context is, and not kept: a script trying themes should not
        // change the person's own choice.
        if let Some(look) = self.pending_look.take() {
            theme::set_appearance(ctx, look, false);
        }
        theme::sync(ctx);
        // The title bar in the interface's mode, not the desktop's.
        let dark = theme::p().dark;
        if self.title_bar_dark != Some(dark) {
            ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(if dark {
                egui::SystemTheme::Dark
            } else {
                egui::SystemTheme::Light
            }));
            self.title_bar_dark = Some(dark);
        }
        self.native_windows = !ctx.embed_viewports();
        // Last frame's navigation, recorded before any panel reads the camera.
        profile::time("ui: sync camera", || self.sync_viewport_camera());

        // An environment being built in the background: pick it up when it
        // lands, and keep frames coming until it does.
        if self.project.poll_environment() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        if self.poll_imports() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        // Another document: material keys start again at one in each, so
        // the graph editor would carry on editing whichever material of the
        // new one had its old key, and the assistant's "Undo request" would
        // reach into a history that is not its own.
        if self.seen_epoch != self.project.document_epoch() {
            self.seen_epoch = self.project.document_epoch();
            self.graph_editor.forget_document();
            self.assistant.forget_turn();
        }

        // Scripted work, before the panels read the document, so a change
        // that arrived over the channel is drawn on the same frame it landed
        // rather than the one after.
        //
        // Taken out of `self` for the duration because the handler needs the
        // whole application and the channel is part of it; it goes straight
        // back afterwards.
        if let Some(open) = self.channel.take() {
            // A request wakes the window, so a script or a plugin is answered
            // at once rather than at the next idle repaint.
            if !self.channel_wakes {
                let waking = ctx.clone();
                open.set_waker(move || waking.request_repaint());
                self.channel_wakes = true;
            }
            open.drain(|request| channel::handle(self, request));
            self.channel = Some(open);
        }
        // Plugins: their code, their messages, their window and panels.
        self.plugins_frame(ctx);
        self.serve_screenshot(ctx);
        self.dropped_files(ctx);
        // The assistant's tool calls, carried out between frames like the
        // scripting channel's.
        let mut assistant = std::mem::take(&mut self.assistant);
        assistant.pump(self, ctx);
        self.assistant = assistant;

        if self.quit_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.quit_requested || self.confirm_close() {
                self.close_approved = true;
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }
        if !self.fitted {
            self.fit_to_screen(ctx);
        }

        let name = self
            .project
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.project.document().meta.name.clone());
        let title = format!(
            "{}{} — Tequila",
            name,
            if self.project.has_unsaved_changes() {
                " *"
            } else {
                ""
            }
        );
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        // Pull the newest finished image, if there is one. Never blocks: if
        // the renderer has nothing new, the previous texture is redrawn
        // (§9.1).
        let upload_started = std::time::Instant::now();
        self.engine_note = self.orchestrator.engine_note();
        if let Some(orchestrator::EngineNote::FellBack { why, .. }) = &self.engine_note {
            if self.told_fallback.as_deref() != Some(why.as_str()) {
                self.told_fallback = Some(why.clone());
                self.toasts.warn(fallback_notice(why));
            }
        }
        if let Some(image) = self.orchestrator.take_image(&mut self.seen_generation) {
            profile::record("ui: images shown", 0.0);
            if image.width > 0 && image.height > 0 {
                let color_image = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width, image.height],
                    &image.pixels,
                );
                match &mut self.texture {
                    Some(texture) => texture.set(color_image, egui::TextureOptions::LINEAR),
                    None => {
                        self.texture = Some(ctx.load_texture(
                            "viewport",
                            color_image,
                            egui::TextureOptions::LINEAR,
                        ))
                    }
                }
                self.image = image;
            }
            profile::record("ui: upload", upload_started.elapsed().as_secs_f64() * 1000.0);
        }

        if let Some(full) = self.fullscreen_request.take() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(full));
        }
        if self.presenting.is_some() {
            // Nothing but the view, whatever a shortcut toggled.
            self.full_viewport = true;
        } else {
            egui::TopBottomPanel::top("menu")
                .exact_height(44.0)
                .show_separator_line(true)
                .frame(egui::Frame::none().fill(theme::p().chrome).inner_margin(egui::Margin::symmetric(12.0, 0.0)))
                .show(ctx, |ui| profile::time("ui: menu", || self.menu_bar(ui)));
        }
        // Across the whole width, under the side panels as well as the
        // viewport: it is the window's frame, not the viewport's.
        if !self.full_viewport {
            egui::TopBottomPanel::bottom("status")
                .exact_height(26.0)
                .frame(egui::Frame::none().fill(theme::p().chrome).inner_margin(egui::Margin::symmetric(12.0, 0.0)))
                .show(ctx, |ui| profile::time("ui: status", || self.status_bar(ui)));
            egui::SidePanel::right("rail")
                .exact_width(RAIL)
                .resizable(false)
                .frame(egui::Frame::none().fill(theme::p().chrome))
                .show(ctx, |ui| profile::time("ui: rail", || self.inspector_rail(ui)));
        }
        // The inspector against its rail, so it opens and closes in one
        // place; the conversation, when it is open, between it and the
        // viewport.
        if !self.full_viewport && self.show_project {
            egui::SidePanel::right("project")
                .default_width(340.0)
                .width_range(320.0..=420.0)
                .frame(egui::Frame::none().fill(theme::p().panel).inner_margin(egui::Margin { left: 14.0, right: 10.0, top: 0.0, bottom: 0.0 }))
                .show(ctx, |ui| profile::time("ui: inspector", || self.project_panel(ui)));
        }

        // The Project window, with the tabs `docs/04` §1.1 lists — the scene
        // tree among them. It was a panel of its own until people had three
        // columns of controls around a viewport that had what was left: the
        // tree and the material editor are never both being read at the same
        // moment, and a tab strip says so where two panels insist otherwise.
        if !self.full_viewport && self.assistant.open {
            let can_undo = self.assistant.can_undo_turn(self.project.bus());
            let mut assistant = std::mem::take(&mut self.assistant);
            let action = egui::SidePanel::right("assistant")
                .resizable(true)
                .default_width(380.0)
                .width_range(300.0..=640.0)
                .show(ctx, |ui| assistant.show(ui, can_undo))
                .inner;
            match action {
                assistant::Action::Ask(text) => assistant.ask(text, self.project.bus().depth()),
                assistant::Action::Stop => assistant.stop(),
                assistant::Action::UndoTurn => assistant.undo_turn(self),
                assistant::Action::None => {}
            }
            self.assistant = assistant;
        }
        {
            let mut assistant = std::mem::take(&mut self.assistant);
            assistant.providers_window(ctx);
            self.assistant = assistant;
        }

        if self.show_library
            && !self.full_viewport
            && (ctx.screen_rect().width() >= 1100.0 || !self.show_project)
        {
            egui::SidePanel::left("library")
                // Exact, not a range.
                //
                // A resizable panel sizes itself to its contents, and a range
                // is only advisory against that — the browser grew past a
                // 340 px ceiling a hundred pixels a frame until the viewport
                // had nothing left. The contents are bounded now as well, but
                // the panel not being able to grow at all is the guarantee
                // that does not depend on getting every child widget right.
                .exact_width(library_tab::PANEL_WIDTH)
                .frame(egui::Frame::none().fill(theme::p().panel).inner_margin(egui::Margin { left: 12.0, right: 12.0, top: 0.0, bottom: 0.0 }))
                .show(ctx, |ui| {
                    panel_title(ui, icons::Icon::Library, theme::hue(theme::Hue::Pink), "Materials", "Drag a finish onto your model", || {
                        self.show_library = false;
                    }, "Hide the library · M");
                    profile::time("ui: library", || self.library_panel(ui));
                });
        }

        // Under the viewport and between the side panels, as a terminal sits
        // under an editor: the scene stays in view while you type at it.
        if self.console.open && !self.full_viewport {
            egui::TopBottomPanel::bottom("console")
                .resizable(true)
                .default_height(240.0)
                .height_range(96.0..=900.0)
                .frame(egui::Frame::none().fill(console::background()))
                .show(ctx, |ui| self.console_panel(ui));
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(theme::p().panel))
            .show(ctx, |ui| profile::time("ui: viewport", || self.viewport(ui)));

        if self.presenting.is_some() {
            self.presentation_overlay(ctx);
        }
        if self.batch_plan.is_some() {
            self.batch_plan_window(ctx);
        }
        if self.web_plan.is_some() {
            self.web_plan_window(ctx);
        }
        if self.spin_plan.is_some() {
            self.spin_plan_window(ctx);
        }
        self.finish_spins();
        if self.web_export.as_ref().is_some_and(|job| job.is_finished()) {
            if let Some(job) = self.web_export.take() {
                self.project.status = match job.join() {
                    Ok(Ok(report)) => {
                        let note = if report.warnings.is_empty() { String::new() } else { format!(" ({})", report.warnings.join("; ")) };
                        format!("Exported the configurator to {}{note}", report.index.display())
                    }
                    Ok(Err(error)) => format!("The web export failed: {error}"),
                    Err(_) => "The web export failed".into(),
                };
            }
        } else if self.web_export.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        if self.show_help {
            self.help_window(ctx);
        }
        if self.show_about {
            self.about_window(ctx);
        }
        self.update_report_window(ctx);
        self.model_units_window(ctx);
        self.improve_model_window(ctx);
        profile::time("ui: queue", || self.pump_queue(ctx));
        let mut window = std::mem::take(&mut self.render_window);
        window.rendering = self.batch.is_running();
        profile::time("ui: render window", || window.show(ctx, &mut self.project));
        if std::mem::take(&mut window.cancel_requested) {
            self.batch.cancel();
        }
        self.render_window = window;
        if !self.recoverable.is_empty() {
            self.recovery_window(ctx);
        }
        {
            let mut editor = std::mem::take(&mut self.graph_editor);
            profile::time("ui: graph", || editor.show(ctx, &mut self.project));
            self.graph_editor = editor;
        }

        profile::time("ui: hotkeys", || self.global_hotkeys(ctx));

        // The document owns the image style; the display pipeline is told
        // about it when it changes. Sending it every frame would be harmless
        // but would lock a mutex the render thread reads, sixty times a second,
        // for nothing.
        let style = self.project.image_style();
        if style != self.published_style {
            self.published_style = style;
            self.orchestrator.set_style(style);
        }
        // The outline follows the selection, and a selection change must not
        // restart the render — so it rides the same display-only channel the
        // image style does. Selecting a group outlines every part under it,
        // because the group is not itself anything the camera can see.
        if self.dragging.is_some() {
            // A drag in progress outlines what the drop would affect instead
            // of what is selected. Showing both at once would put two rings on
            // screen with no way to tell which one the mouse button is about
            // to act on.
            self.outline_key = None;
            self.outlined = self.drag_highlight.clone();
            self.orchestrator.set_selection(self.outlined.clone());
        } else {
            // Worked out when the selection or the scene changes, not every
            // frame: a selected group walked every part under it each time.
            let key = (self.project.selection.nodes().collect::<Vec<_>>(), self.project.rebuilds());
            if self.outline_key.as_ref() != Some(&key) {
                self.outlined = key
                    .0
                    .iter()
                    .flat_map(|&node| self.project.document().tree.descendants_and_self(node))
                    .filter_map(|node| self.project.mapping().instance_of(node))
                    .map(|instance| instance.0)
                    .collect();
                self.orchestrator.set_selection(self.outlined.clone());
                self.outline_key = Some(key);
            }
        }
        // A GPU failure hands Solid back to the CPU: tell the renderer, once.
        if self.gpu_solid_submitted && !self.gpu_solid() {
            if let Some(error) = self.solid.error() {
                self.project.status = format!("Solid is drawing on the CPU: {error}");
            }
            self.submit();
        }

        // And the display pipeline, which the built-in transform is part of —
        // so choosing a different tone map goes through here too rather than
        // being read out of the style on the render thread.
        let display = self.project.display_pipeline();
        self.orchestrator.set_display(display);

        self.project.maybe_autosave();

        // The scene may have changed from the panels this frame.
        if self.project.generation() != self.submitted_generation {
            self.submitted_generation = self.project.generation();
            self.submit();
        }

        // A released pointer ends whatever interaction it was driving, for
        // every slider and field that does not say so itself. At the end of
        // the frame, so this frame's own commands still merge into it.
        if ctx.input(|i| i.pointer.any_released()) {
            self.project.end_interaction();
        }

        // The renderer runs continuously, so the UI must keep repainting to
        // show it converging. This is the one place a UI framework's "only
        // repaint on input" default is wrong for this application.
        //
        // The exception is a finished fast preview. That image cannot change
        // until something is edited, and repainting it sixty times a second
        // burns a core to display a static picture — in the one mode a person
        // chose *because* the machine was busy.
        let suspended = self.viewport_paused
            || (self.batch.is_running() && self.shading != orchestrator::Shading::Fast)
            || ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        self.orchestrator.set_paused(suspended);
        // On the GPU a Solid frame is finished the moment it is drawn; on the
        // CPU, once the renderer has reached its sample cap. Waiting for the
        // cap in GPU mode — which never traces, so never reaches it — kept the
        // whole UI polling at 60 Hz over a picture that could not change.
        let settled = self.shading == orchestrator::Shading::Fast
            && (self.gpu_solid() || self.image.samples >= orchestrator::Shading::FAST_SAMPLE_CAP)
            && !self.interacting;
        if suspended || settled {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        } else {
            // Input still repaints immediately. Polling for render frames
            // at 60 Hz avoids driving the entire UI at the monitor's maximum.
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        self.ui_capture.update(ctx);
    }

    /// A clean shutdown discards the journal (§4.5). Without this every exit
    /// looks like a crash, and the next launch offers to recover work that was
    /// never lost — which trains the user to dismiss the prompt that matters.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.end_session();
        // Before the window is gone, so a client asking in the meantime is
        // pointed at a Tequila that will still be there to answer.
        if let Some(channel) = &mut self.channel {
            channel.retire();
        }
    }
}

impl TequilaApp {
    fn viewport(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_size();
        let (area, response) = ui.allocate_exact_size(available, egui::Sense::click_and_drag());
        // The picture: all of the area, or with the render frame on, the
        // render's own shape inside it. Everything about the picture -
        // drawing, picking, the gizmos - works in `rect`; the controls laid
        // over the view stay where they always are, in `area`. Navigation
        // takes drags anywhere in the area.
        let rect = self.picture_rect(area);
        let size = (
            (rect.width().max(1.0) as usize).max(16),
            (rect.height().max(1.0) as usize).max(16),
        );
        // A window dragged to another screen, or a new text size, changes
        // the pixels behind the same points.
        let scale = render_scale(ui.ctx().native_pixels_per_point(), ui.ctx().pixels_per_point());
        let resized = size != self.viewport_size || scale != self.render_scale;
        self.viewport_size = size;
        self.render_scale = scale;
        self.viewport_rect = rect;

        // ---- Navigation (`TEQUILA_SPEC.md` §1.7) -----------------------
        let mut moved = resized;
        let height = rect.height().max(1.0);
        let aspect = rect.width().max(1.0) / height;
        let modifiers = ui.input(|i| i.modifiers);

        // The gizmo is consulted first, and takes the drag when the cursor is
        // over a handle. Orbiting instead would make the gizmo unusable: every
        // attempt to move a part would spin the camera.
        // Label placement is consulted before the transform gizmo: while it
        // is armed the primary button belongs to it, and two things claiming
        // the same drag is how a gizmo becomes unusable.
        let mut pin_gizmo = std::mem::take(&mut self.pin_gizmo);
        let aiming_pin = pin_gizmo.interact(
            ui,
            rect,
            &self.camera.camera,
            &mut self.project,
            &self.orchestrator,
            &response,
        );
        self.pin_gizmo = pin_gizmo;

        // Measuring takes clicks only, so the view can still be turned
        // between placing one end and the other.
        let mut measure = std::mem::take(&mut self.measure);
        let measuring = measure.interact(
            ui,
            rect,
            &self.camera.camera,
            &mut self.project,
            &self.orchestrator,
            &response,
        );
        self.measure = measure;

        let mut label_gizmo = std::mem::take(&mut self.label_gizmo);
        let placing_label = label_gizmo.interact(
            ui,
            rect,
            &self.camera.camera,
            &mut self.project,
            &self.orchestrator,
            &response,
        );
        self.label_gizmo = label_gizmo;

        let mut gizmo = std::mem::take(&mut self.gizmo);
        let taken = placing_label || aiming_pin || measuring;
        let over_gizmo = taken
            || gizmo.interact(ui, rect, &self.camera.camera, &mut self.project, &response);
        let gizmo_dragging = taken || gizmo.is_dragging();
        self.gizmo = gizmo;

        // ---- A material being dragged in (§04 §3.1) ---------------------
        //
        // Handled before navigation, so a drag that started in the library is
        // never taken for an orbit. It suppresses navigation and picking for
        // this frame, but the frame is still drawn: a viewport that goes blank
        // the moment a material is picked up hides the very thing the drop is
        // being aimed at.
        let dropping = self.dragging.is_some();
        if dropping {
            self.material_drag(ui, rect, &response);
        }

        // ---- The axis legend (§2.2) --------------------------------------
        //
        // Consulted before picking and before navigation, for the same reason
        // the gizmo is: it sits on top of the image, so a click on it is a
        // click on it and not on whatever is behind.
        let over_legend = self.show_axis_legend
            && response
                .hover_pos()
                .is_some_and(|at| axis_legend::contains(area, at));

        // ---- Picking (§4.1) ---------------------------------------------
        let presenting = self.presenting.is_some();
        if !dropping && !over_legend && response.clicked() && !over_gizmo && !presenting {
            // §5 binds pivot placement to Insert. Held down, a click puts the
            // pivot on the surface under the cursor rather than selecting —
            // which is what makes "rotate about *that* corner" a single
            // gesture instead of typing three numbers.
            if ui.input(|i| i.key_down(egui::Key::Insert)) {
                self.set_pivot_at(rect, response.interact_pointer_pos());
            } else {
                if !response.double_clicked() {
                    self.selected_before_click = self.project.selection.nodes().collect();
                }
                self.pick_at(ui, rect, response.interact_pointer_pos());
                if response.double_clicked() && !ui.input(|i| i.modifiers.any()) {
                    self.select_enclosing(rect, response.interact_pointer_pos());
                }
            }
        }

        if !dropping
            && !over_legend
            && !gizmo_dragging
            && response.dragged_by(egui::PointerButton::Primary)
        {
            let d = response.drag_delta();
            self.camera.orbit((d.x, d.y), height);
            moved = true;
        }
        if !dropping && response.dragged_by(egui::PointerButton::Middle) {
            let d = response.drag_delta();
            self.camera.pan((d.x, d.y), height, aspect);
            moved = true;
        }
        if !dropping && response.dragged_by(egui::PointerButton::Secondary) && modifiers.alt {
            self.camera.dolly(response.drag_delta().y);
            moved = true;
        }

        // A trackpad's swipes and pinches (`orbit::Gestures`): a MacBook's
        // only way to pan and zoom.
        let gestures = if !dropping && response.hovered() && !self.label_gizmo.takes_scroll(modifiers) {
            ui.input(|i| orbit::Gestures::read(&i.events))
        } else {
            orbit::Gestures::default()
        };
        if gestures.moves() {
            if gestures.pan != egui::Vec2::ZERO {
                self.camera.pan((gestures.pan.x, gestures.pan.y), height, aspect);
            }
            if gestures.orbit != egui::Vec2::ZERO && !gizmo_dragging {
                self.camera.orbit((gestures.orbit.x, gestures.orbit.y), height);
            }
            if gestures.zoom != 0.0 {
                self.camera.zoom(gestures.zoom);
            }
            if gestures.focal_length != 0.0 {
                self.camera.change_focal_length(gestures.focal_length);
            }
            moved = true;
        }
        if !dropping && response.hovered() && !gestures.swiped {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            // A label being placed takes Ctrl and Shift + scroll for its
            // size and turn.
            if scroll != 0.0 && !self.label_gizmo.takes_scroll(modifiers) {
                // Alt + scroll changes focal length rather than distance —
                // the binding from §1.7.
                if modifiers.alt {
                    self.camera.change_focal_length(scroll / orbit::POINTS_PER_NOTCH);
                } else {
                    self.camera.zoom(scroll / orbit::POINTS_PER_NOTCH);
                }
                moved = true;
            }
        }

        // Standard views on Ctrl+Alt+1..7.
        if modifiers.command && modifiers.alt {
            let digits = [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
            ];
            for (index, key) in digits.iter().enumerate() {
                if ui.input(|i| i.key_pressed(*key)) {
                    if let Some(view) = StandardView::from_digit(index as u32 + 1) {
                        self.camera.set_standard_view(view);
                        moved = true;
                    }
                }
            }
        }

        // §2.3's context menu. A right-click, not a right-drag: the drag is
        // already Alt+dolly, and a menu that appeared at the end of a dolly
        // would be a menu nobody asked for.
        if !dropping && response.secondary_clicked() {
            let at = response.interact_pointer_pos();
            self.context_target = at.and_then(|at| self.node_at(rect, at));
            // Right-clicking a part that is not selected selects it first.
            // Acting on something other than what was clicked is the way a
            // context menu loses people's trust.
            if let Some(node) = self.context_target {
                if !self.project.selection.contains(node) {
                    let mut selection = std::mem::take(&mut self.project.selection);
                    selection.click(
                        self.project.document(),
                        node,
                        tequila_scene::selection::SelectionMode::Replace,
                    );
                    self.project.selection = selection;
                }
            }
        }
        // egui's own menu, not a hand-rolled popup.
        //
        // The hand-rolled one had a bug that made a whole class of entries
        // unreachable. A submenu opens as its own area, so the "the click
        // landed outside the menu, dismiss it" test counted every click
        // *inside* a submenu as a click away and closed the lot before the
        // action could run. Everything under "Standard view" — Isometric
        // included — could be opened, hovered and clicked, and did nothing.
        // Nesting and dismissal are precisely the parts of a menu that have to
        // be right, and egui already has them.
        if !presenting {
            response.context_menu(|ui| {
                ui.set_min_width(190.0);
                if self.context_target.is_some() {
                    self.part_context_items(ui);
                } else {
                    self.background_context_items(ui);
                }
            });
        }

        let dragging = response.dragged() || gestures.moves() || ui.input(|i| i.raw_scroll_delta.y != 0.0);
        if dragging != self.interacting {
            self.interacting = dragging;
            moved = true;
        }
        if moved {
            self.submit();
        }

        // ---- Draw -------------------------------------------------------
        if self.gpu_solid() {
            let frame = solid_gpu::Frame {
                scene: self.project.scene(),
                camera: self.camera.camera,
                selected: self.outlined.clone(),
                pixels_per_point: ui.ctx().pixels_per_point(),
                mode: self.view_mode.gpu().unwrap_or_default(),
                exposure: 2f32.powf(self.project.image_style().exposure),
                model_edges: self.show_model_edges && self.presenting.is_none(),
            };
            ui.painter().add(self.solid.paint(rect, frame));
        } else if let Some(texture) = &self.texture {
            // The render buffer is smaller than the viewport while
            // interacting; stretching it is exactly the "upsampled for
            // display" step of §9.2.
            ui.painter().image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // The grid goes over the image and under the HUD: it is a measure of
        // the scene, so it belongs on the picture, but it must never sit on top
        // of the numbers.
        if self.camera_tab.show_grid {
            grid::draw(
                &ui.painter_at(rect),
                rect,
                &self.camera.camera,
                self.camera.target,
                self.camera.distance,
                self.project.document().units.shown(),
            );
        }

        // Above the image and the grid, below the HUD. Anything painted before
        // the image is painted *under* it: egui draws in call order within a
        // layer, and the image covers the whole viewport.
        // A presentation shows the product and nothing of the tools.
        let presenting = self.presenting.is_some();
        if !presenting {
            cutaway_panel::draw_outlines(ui, rect, &self.camera.camera, &self.project);
            self.measure.draw(ui, rect, &self.camera.camera, &self.project);
            self.gizmo.draw(ui, rect, &self.camera.camera);
            self.label_gizmo.draw(ui, rect, &self.camera.camera, &self.project);
            self.pin_gizmo.draw(ui, rect, &self.project);
        }

        if rect != area {
            render_frame::draw(ui.painter(), area, rect, self.project.render_settings());
        }

        if self.show_axis_legend {
            if let Some(view) = axis_legend::show(ui, area, &self.camera.camera, Vec3::Y) {
                self.camera.set_standard_view(view);
                self.submit();
            }
        }

        // Bare `H` only: `Ctrl+H` hides the selection and `Alt+H` unhides
        // everything, and toggling the HUD as well would make both look like
        // they had done two things.
        // And not while a text field has focus, or typing a name with an "h"
        // in it flips the HUD.
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        if !typing && ui.input(|i| i.key_pressed(egui::Key::H) && !i.modifiers.any()) {
            self.show_hud = !self.show_hud;
        }
        if !presenting {
            self.viewport_overlays(ui, area);
            self.welcome_card(ui, area);
            if self.show_hud {
                self.hud(ui, area);
            }
        }

        let mut toasts = std::mem::take(&mut self.toasts);
        toasts.show(ui.ctx(), area);
        self.toasts = toasts;
    }

    /// Where the picture goes in the viewport's `area`: all of it, or with
    /// the render frame on, the render's shape inside it. Not while
    /// presenting, which is the product full screen and not a render.
    fn picture_rect(&self, area: egui::Rect) -> egui::Rect {
        let settings = self.project.render_settings();
        if !self.show_render_frame || self.presenting.is_some() {
            return area;
        }
        render_frame::fit(area, settings.width, settings.height)
    }

    /// The node under a viewport point, or `None` for empty space.
    fn node_at(&self, rect: egui::Rect, at: egui::Pos2) -> Option<tequila_scene::tree::NodeId> {
        let ray = gizmo_ui::ray_through(&self.camera.camera, rect, at)?;
        let hit = self.orchestrator.pick(&ray)?;
        self.project.mapping().node_of(hit.instance)
    }

    /// The entries for a right-click on geometry.
    fn part_context_items(&mut self, ui: &mut egui::Ui) {
        let selected: Vec<tequila_scene::tree::NodeId> = self.project.selection.nodes().collect();
        let name = self
            .context_target
            .and_then(|n| self.project.document().tree.get(n))
            .map(|n| n.name.clone())
            .unwrap_or_default();
        ui.label(egui::RichText::new(name).strong());
        ui.separator();

        // Split leads, because it is what somebody is hunting for when a
        // model that should be three parts arrived as one, and the viewport is
        // where they noticed.
        let parts = self.split_count(self.context_target);
        if parts > 1 {
            if ui
                .button(format!("Split into {parts} parts"))
                .on_hover_text(
                    "Separate the mesh into its disconnected shells, so each can \
                     take its own material.",
                )
                .clicked()
            {
                if let Some(node) = self.context_target {
                    let name = self
                        .project
                        .document()
                        .tree
                        .get(node)
                        .map(|n| n.name.clone())
                        .unwrap_or_default();
                    match self.project.apply(Command::SplitIntoParts { node }) {
                        Ok(_) => {
                            self.toasts
                                .info(format!("Split `{name}` into {parts} parts"));
                            // Select the new children, so the next thing a
                            // person does — assign a material to one — starts
                            // from the parts rather than from nothing.
                            let children = self
                                .project
                                .document()
                                .tree
                                .get(node)
                                .map(|n| n.children.clone())
                                .unwrap_or_default();
                            let mut selection = std::mem::take(&mut self.project.selection);
                            selection.select_many(
                                children,
                                tequila_scene::selection::SelectionMode::Replace,
                            );
                            self.project.selection = selection;
                        }
                        Err(error) => self.toasts.warn(error.to_string()),
                    }
                }
                ui.close_menu();
            }
            ui.separator();
        }

        // The group it is in: a CAD body that came as a piece per face
        // colour, or a sub-assembly, to take a material or a move as one.
        let document = self.project.document();
        let group = self.context_target.and_then(|n| document.tree.get(n)).and_then(|n| n.parent);
        if let Some((group, group_name)) = group.and_then(|g| document.tree.get(g).map(|n| (g, n.name.clone()))) {
            if ui
                .button(format!("Select `{group_name}`"))
                .on_hover_text("The group this part is in, so a material or a move takes all of it. Double-clicking the part does the same.")
                .clicked()
            {
                self.project.selection.select_many([group], SelectionMode::Replace);
                ui.close_menu();
            }
        }

        if ui.button("Frame selection").clicked() {
            self.frame(false);
            ui.close_menu();
        }
        if ui
            .button("Select similar")
            .on_hover_text("Every part sharing this material")
            .clicked()
        {
            if let Some(node) = self.context_target {
                let material = self
                    .project
                    .document()
                    .tree
                    .get(node)
                    .and_then(|n| n.material);
                if let Some(material) = material {
                    let users =
                        tequila_scene::selection::users_of(self.project.document(), material);
                    let mut selection = std::mem::take(&mut self.project.selection);
                    selection.select_many(users, tequila_scene::selection::SelectionMode::Replace);
                    self.project.selection = selection;
                }
            }
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Hide").on_hover_text(theme::keys(HIDE_KEYS)).clicked() {
            self.hide_selection();
            ui.close_menu();
        }
        if ui.button("Isolate").on_hover_text("/").clicked() {
            self.isolate_selection();
            ui.close_menu();
        }
        if ui.button("Unhide all").on_hover_text("Alt+H").clicked() {
            self.unhide_all();
            ui.close_menu();
        }
        ui.separator();
        ui.menu_button("Standard view", |ui| {
            for view in StandardView::ALL {
                if ui.button(view.label()).clicked() {
                    self.camera.set_standard_view(view);
                    self.submit();
                    ui.close_menu();
                }
            }
        });
        if ui.button("Rename…").clicked() {
            // The tree owns renaming, so this takes the person to it rather
            // than growing a second in-place editor that would drift from the
            // first.
            self.project_tab = ProjectTab::Scene;
            self.show_project = true;
            self.toasts.info("Press F2 in the Scene tree to rename");
            ui.close_menu();
        }
        if ui.button("Unlink material").clicked() {
            if let Some(node) = self.context_target {
                let _ = self.project.apply(Command::UnlinkMaterial { node });
                self.toasts.info("Material unlinked");
            }
            ui.close_menu();
        }
        if ui.button("Set pivot here").clicked() {
            self.toasts
                .info("Hold Insert and click the model to place a pivot");
            ui.close_menu();
        }
        ui.separator();
        let count = selected.len();
        if ui
            .button(if count > 1 {
                format!("Delete {count} parts")
            } else {
                "Delete".to_string()
            })
            .clicked()
        {
            if !selected.is_empty() {
                let _ = self.project.apply(Command::DeleteNodes { nodes: selected });
            }
            ui.close_menu();
        }
    }

    /// And for a right-click on empty space.
    fn background_context_items(&mut self, ui: &mut egui::Ui) {
        if ui.button("Frame all").on_hover_text("Shift+F").clicked() {
            self.frame(true);
            ui.close_menu();
        }
        if ui.button("Deselect all").on_hover_text("Esc").clicked() {
            self.project.selection.clear();
            ui.close_menu();
        }
        ui.separator();
        ui.menu_button("Standard view", |ui| {
            for view in StandardView::ALL {
                if ui.button(view.label()).clicked() {
                    self.camera.set_standard_view(view);
                    self.submit();
                    ui.close_menu();
                }
            }
        });
        ui.separator();
        if ui.button("Environment…").clicked() {
            self.project_tab = ProjectTab::Environment;
            self.show_project = true;
            ui.close_menu();
        }
        if ui.button("Camera…").clicked() {
            self.project_tab = ProjectTab::Camera;
            self.show_project = true;
            ui.close_menu();
        }
        if ui.button("Render…").clicked() {
            self.render_window.open = true;
            ui.close_menu();
        }
    }

    /// How many shells the node's mesh is made of, or 0 when it is not a part.
    fn split_count(&self, node: Option<tequila_scene::tree::NodeId>) -> usize {
        let Some(node) = node else { return 0 };
        let document = self.project.document();
        let Some(node) = document.tree.get(node) else {
            return 0;
        };
        let tequila_scene::tree::NodeKind::Part { mesh } = node.kind else {
            return 0;
        };
        document.geometry.get(mesh).map(scene_tab::pieces).unwrap_or(0)
    }

    /// §3's scene units, and the two ways of changing them.
    ///
    /// Lengths are stored in metres whatever the unit, which is how they are
    /// shown and typed. Changing it asks a question, because the two answers
    /// are not a preference: one leaves the scene alone and the other
    /// rescales it, and picking the wrong one silently ruins either how the
    /// numbers read or the size of the model. So the control asks it in
    /// words, with the size the model comes out at under each answer, rather
    /// than offering the mode names, which mean nothing until you already
    /// know the answer.
    fn units_panel(&mut self, ui: &mut egui::Ui) {
        use tequila_core::units::LengthUnit;
        use tequila_scene::units::{ConversionMode, ConversionPlan};

        let units = self.project.document().units;
        let current = units.unit;
        let mut chosen = current;

        ui.horizontal(|ui| {
            ui.label("Unit");
            egui::ComboBox::from_id_salt("scene-units")
                .selected_text(unit_label(current))
                .show_ui(ui, |ui| {
                    for candidate in [
                        LengthUnit::Millimetre,
                        LengthUnit::Centimetre,
                        LengthUnit::Metre,
                        LengthUnit::Inch,
                        LengthUnit::Foot,
                    ] {
                        ui.selectable_value(&mut chosen, candidate, unit_label(candidate));
                    }
                });
            if current == LengthUnit::Unspecified {
                ui.label(egui::RichText::new("not set").small().color(theme::dim()))
                    .on_hover_text(
                        "Lengths are shown in metres until a unit is chosen. They \
                         are real sizes either way: materials, lights and the \
                         camera's depth of field are physical whatever the unit.",
                    );
            }
        });

        if chosen != current {
            self.pending_unit = Some(chosen);
        }

        let Some(wanted) = self.pending_unit else {
            return;
        };
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!(
                "Changing to {}. Which is true?",
                unit_label(wanted)
            ))
            .strong(),
        );
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);

        // The model's size now, and what reading its numbers in the new unit
        // would make it: the right answer is the one that reads as the right
        // size.
        let bounds = tequila_scene::snapshot::visible_bounds(self.project.document());
        let size = (!bounds.is_empty()).then(|| bounds.max - bounds.min);
        let rescale = ConversionPlan::compute(units, wanted, ConversionMode::ScaleScene).geometry_scale as f32;
        let unit = unit_label(wanted).to_lowercase();
        let outcome = |ui: &mut egui::Ui, text: String| {
            ui.label(egui::RichText::new(text).small().color(theme::dim()));
        };

        // Asked as the two things a person actually knows, rather than as
        // "Keep Scene Size" and "Scale Scene" — mode names that describe the
        // mechanism and tell you nothing about which one you want.
        let mut picked = None;
        if ui
            .button("The model is the right size")
            .on_hover_text(format!(
                "Nothing in the scene changes. Lengths are shown and typed in {unit}."
            ))
            .clicked()
        {
            picked = Some(ConversionMode::KeepSceneSize);
        }
        if let Some(size) = size {
            outcome(ui, format!("Stays {}", model_units::real_size(size)));
        }
        if ui
            .button(format!("The model is the wrong size: its numbers are {unit}"))
            .on_hover_text(
                "It came in at the wrong unit. Its numbers stay as they read and \
                 are taken as the new unit, so the scene is rescaled, with its \
                 lights and cameras, and the shot looks as it did. Materials keep \
                 their real sizes, so their patterns come out right on it.",
            )
            .clicked()
        {
            picked = Some(ConversionMode::ScaleScene);
        }
        if let Some(size) = size {
            outcome(ui, format!("Becomes {}", model_units::real_size(size * rescale)));
        }
        if let Some(mode) = picked {
            if self.project.apply(Command::SetUnits { unit: wanted, mode }).is_ok() {
                if mode == ConversionMode::ScaleScene {
                    // The cameras moved with the scene. The view goes with
                    // them, or the next frame writes the old one back.
                    self.camera.adopt(self.project.camera());
                    self.submit();
                    self.toasts.info(format!("Rescaled the scene, read in {unit}"));
                } else {
                    self.toasts.info(format!("Lengths are shown in {unit}"));
                }
            }
            self.pending_unit = None;
        }
        if ui.small_button("Cancel").clicked() {
            self.pending_unit = None;
        }
    }

    /// §6.1's Frame selection / Frame all.
    fn frame(&mut self, everything: bool) {
        use tequila_core::math::Aabb;
        let document = self.project.document();
        let bounds = if everything || self.project.selection.is_empty() {
            tequila_scene::snapshot::visible_bounds(document)
        } else {
            let mut bounds = Aabb::EMPTY;
            for node in self.project.selection.nodes() {
                for id in document.tree.descendants_and_self(node) {
                    let Some(part) = document.tree.get(id) else {
                        continue;
                    };
                    let tequila_scene::tree::NodeKind::Part { mesh } = part.kind else {
                        continue;
                    };
                    if let Some(source) = document.geometry.get(mesh) {
                        bounds.extend(
                            &source
                                .bounds()
                                .transformed(&document.tree.world_transform(id)),
                        );
                    }
                }
            }
            bounds
        };
        if bounds.is_empty() {
            self.toasts.warn("Nothing to frame");
            return;
        }
        let radius = (bounds.diagonal().length() * 0.5).max(1e-3);
        let aspect = self.aspect();
        let framed = orbit::OrbitCamera::framing(bounds.center(), radius, aspect);
        // Keep the direction the user is looking from and take only the
        // distance and the target. Reframing that also spun the camera back to
        // a default three-quarter view would undo the shot they had set up.
        self.camera.frame_at(framed.target, framed.distance);
        self.submit();
    }

    /// §6.2's `E` / `C` / `B` background bindings.
    fn set_background(&mut self, mode: tequila_scene::document::BackgroundMode) {
        use tequila_scene::command::{EnvParam, EnvValue, EnvironmentParameter};
        if self.project.document().environment.background == mode {
            return;
        }
        let _ = self.project.apply(Command::SetEnvironmentParameter {
            parameter: EnvironmentParameter::new(EnvParam::Background, EnvValue::Mode(mode)),
        });
        self.toasts.info(format!("Background: {}", mode.label()));
    }

    fn hide_selection(&mut self) {
        let states: Vec<_> = self
            .project
            .selection
            .nodes()
            .flat_map(|n| self.project.document().tree.descendants_and_self(n))
            .map(|n| (n, false))
            .collect();
        if states.is_empty() {
            return;
        }
        let count = states.len();
        let _ = self.project.apply(Command::SetVisibility { states });
        self.project.selection.clear();
        self.toasts.info(format!(
            "Hid {count} part{}",
            if count == 1 { "" } else { "s" }
        ));
    }

    /// Show the selection and hide everything else — one command, so one undo
    /// step (which is why `SetVisibility` takes a flag per node).
    fn isolate_selection(&mut self) {
        let keep: std::collections::BTreeSet<_> = self
            .project
            .selection
            .nodes()
            .flat_map(|n| self.project.document().tree.descendants_and_self(n))
            .collect();
        if keep.is_empty() {
            self.toasts.warn("Select something to isolate");
            return;
        }
        // Ancestors of a kept node stay visible too, or isolating a part
        // inside a group hides the group and with it the part.
        let document = self.project.document();
        let states: Vec<_> = document
            .tree
            .iter()
            .map(|node| {
                let visible = keep.contains(&node.id)
                    || document
                        .tree
                        .descendants_and_self(node.id)
                        .iter()
                        .any(|d| keep.contains(d));
                (node.id, visible)
            })
            .collect();
        let hidden = states.iter().filter(|(_, v)| !*v).count();
        let _ = self.project.apply(Command::SetVisibility { states });
        self.toasts.info(format!("Isolated — {hidden} hidden"));
    }

    fn unhide_all(&mut self) {
        let states: Vec<_> = self
            .project
            .document()
            .tree
            .iter()
            .filter(|n| !n.visible)
            .map(|n| (n.id, true))
            .collect();
        if states.is_empty() {
            self.toasts.info("Nothing is hidden");
            return;
        }
        let count = states.len();
        let _ = self.project.apply(Command::SetVisibility { states });
        self.toasts.info(format!("Unhid {count}"));
    }

    /// Select what is under the cursor, through the same BVH the renderer
    /// traced (§4.1's viewport-to-tree synchronization).
    /// The library browser, and what the user does with it.
    fn library_panel(&mut self, ui: &mut egui::Ui) {
        // Which library materials are already in the document, for §9's
        // "used in this project" shelf.
        let in_project: std::collections::BTreeSet<String> = self
            .project
            .document()
            .materials
            .iter()
            .map(|(_, d)| d.name.clone())
            .collect();

        // A preset saved into the folder from outside shows up by itself.
        self.library.watch();
        let selected = self.selected_material().map(|(_, s)| s);
        if let Some(action) = self.library.show(ui, &in_project, selected.as_ref()) {
            self.library_action(action);
        }
    }

    /// Hover preview and drop for a material dragged over the viewport
    /// (§04 §3.1).
    ///
    /// The label is computed from the same planner the drop uses, so it cannot
    /// promise something the drop will not do — two code paths would eventually
    /// disagree, and a label that lies is worse than no label.
    fn material_drag(&mut self, ui: &mut egui::Ui, rect: egui::Rect, response: &egui::Response) {
        use tequila_scene::apply::{plan_drop_in, DropModifiers};

        let Some(entry) = self.dragging.clone() else {
            return;
        };
        let pointer = ui
            .input(|i| i.pointer.hover_pos())
            .or_else(|| response.interact_pointer_pos());
        let keys = ui.input(|i| i.modifiers);
        let modifiers = DropModifiers {
            alt: keys.alt,
            shift: keys.shift,
            ctrl: keys.command,
        };

        let mut plan = None;
        self.drag_highlight.clear();
        if let Some(at) = pointer.filter(|p| rect.contains(*p)) {
            if let Some(ray) = gizmo_ui::ray_through(&self.camera.camera, rect, at) {
                if let Some(hit) = self.orchestrator.pick(&ray) {
                    if let Some(node) = self.project.mapping().node_of(hit.instance) {
                        let selected: Vec<_> = self.project.selection.nodes().collect();
                        plan = plan_drop_in(self.project.document(), node, modifiers, &selected);
                        // Everything the drop would touch, not just the part
                        // under the cursor: with Shift or Ctrl held that is a
                        // whole group or every part sharing a material, and
                        // the label says so in words while this says it in the
                        // picture.
                        if let Some(plan) = &plan {
                            self.drag_highlight = plan
                                .affected
                                .iter()
                                .flat_map(|n| self.project.document().tree.descendants_and_self(*n))
                                .filter_map(|n| self.project.mapping().instance_of(n))
                                .map(|i| i.0)
                                .collect();
                        }
                    }
                }
            }

            // The floating label: what the drop will affect, updating live with
            // the modifiers.
            let text = match &plan {
                Some(plan) => format!("{}\n{}", entry.name, plan.description),
                None => format!("{}\nDrop on a part to apply", entry.name),
            };
            egui::show_tooltip_at(
                ui.ctx(),
                ui.layer_id(),
                egui::Id::new("material_drop"),
                at + egui::vec2(18.0, 18.0),
                |ui| {
                    ui.label(text);
                    ui.label(
                        egui::RichText::new(format!(
                            "Shift: whole group · {}: same material · {}: this part only, unlinked",
                            theme::keys("Ctrl"),
                            theme::keys("Alt"),
                        ))
                        .small()
                        .weak(),
                    );
                },
            );
        }

        // Releasing anywhere ends the drag; releasing over a part applies it.
        if ui.input(|i| i.pointer.any_released()) {
            self.dragging = None;
            if let Some(plan) = plan {
                self.apply_library_material(&entry, &plan.affected, plan.unlink);
            }
        }
    }

    /// Assign a library material, as one undo step (§3.1 step 5).
    fn apply_library_material(
        &mut self,
        entry: &tequila_format::library::LibraryEntry,
        nodes: &[tequila_scene::tree::NodeId],
        unlink: bool,
    ) {
        if nodes.is_empty() {
            return;
        }

        // The recipe every way in shares (`library::apply_commands`): an
        // existing instance of the same library material is reused rather
        // than a second copy added - otherwise dropping "ABS White" on five
        // parts leaves five identical materials in the project, and editing
        // one changes a fifth of what the user expects (§8.2).
        for command in tequila_format::library::apply_commands(entry, self.project.document(), nodes, unlink) {
            if self.project.apply(command).is_err() {
                return;
            }
        }
        self.project.end_interaction();
        self.library.note_used(&entry.name);

        // §3.1 step 4: select what was affected, so the Material tab shows the
        // new material immediately.
        self.project.selection.select_many(
            nodes.iter().copied(),
            tequila_scene::selection::SelectionMode::Replace,
        );
    }


    /// Double-click: the group around what was clicked - a CAD body that
    /// came as a piece per face colour, a sub-assembly - so a material or a
    /// move takes all of it; double-clicked again, the group around that.
    fn select_enclosing(&mut self, rect: egui::Rect, at: Option<egui::Pos2>) {
        let Some(node) = at.and_then(|at| self.node_at(rect, at)) else {
            return;
        };
        if let Some(group) = enclosing_group(self.project.document(), node, &self.selected_before_click) {
            self.project.selection.select_many([group], SelectionMode::Replace);
        }
    }

    fn pick_at(&mut self, ui: &egui::Ui, rect: egui::Rect, at: Option<egui::Pos2>) {
        let Some(at) = at else { return };
        let Some(ray) = gizmo_ui::ray_through(&self.camera.camera, rect, at) else {
            return;
        };
        let modifiers = ui.input(|i| i.modifiers);
        let mode = if modifiers.shift && modifiers.command {
            SelectionMode::Remove
        } else if modifiers.shift {
            SelectionMode::Add
        } else {
            SelectionMode::Replace
        };
        // §4.1's modifier bindings for the *level*, which are separate from
        // the ones that combine with the existing selection.
        let level = if modifiers.alt {
            Some(tequila_scene::selection::SelectionLevel::Object)
        } else if modifiers.command && !modifiers.shift {
            Some(tequila_scene::selection::SelectionLevel::Material)
        } else {
            None
        };
        let restore = self.project.selection.level();
        if let Some(level) = level {
            self.project.selection.set_level(level);
        }

        match self.orchestrator.pick(&ray) {
            Some(hit) => {
                if let Some(node) = self.project.mapping().node_of(hit.instance) {
                    let mut selection = std::mem::take(&mut self.project.selection);
                    if selection.level() == tequila_scene::selection::SelectionLevel::Face {
                        selection.click_face(node, hit.primitive, mode);
                    } else {
                        selection.click(self.project.document(), node, mode);
                    }
                    self.project.selection = selection;
                }
            }
            // Clicking empty space deselects, which is what every DCC does and
            // what makes "click away to finish" work.
            None if mode == SelectionMode::Replace => self.project.selection.clear(),
            None => {}
        }

        if level.is_some() {
            self.project.selection.set_level(restore);
        }
    }

    /// Add a label to a part and arm its placement. Capture runs only.
    ///
    /// Not a feature: the only way a label reaches a scene in normal use is
    /// through the file dialog in the Labels panel, and a screenshot run
    /// cannot open one. Gated on TEQUILA_UI_CAPTURE like the rest of the
    /// capture facility.
    fn add_capture_label(&mut self, part: tequila_scene::tree::NodeId, artwork: std::path::PathBuf) {
        let Some(material) = self
            .project
            .document()
            .tree
            .get(part)
            .and_then(|node| node.material)
        else {
            return;
        };
        if self
            .project
            .apply(Command::AddDecal {
                material,
                source: tequila_scene::textures::TextureSource::new(
                    artwork,
                    tequila_texture::texture::MapRole::Color,
                ),
                name: "Logo".into(),
            })
            .is_err()
        {
            return;
        }
        // Switched to projected placement, which is what the "Project flat"
        // button does: the demo part has UV coordinates, so a label added to
        // it keeps them, and a capture of the placement mode would show no
        // frame because a UV label has none to show.
        if let Some(fit) = tequila_scene::decal_fit::fit_projected(self.project.document(), material)
        {
            let mut decals = self
                .project
                .document()
                .materials
                .get(material)
                .map(|m| m.graph.decals.clone())
                .unwrap_or_default();
            if let Some(decal) = decals.first_mut() {
                decal.mapping = fit;
                let _ = self.project.apply(Command::SetDecals { material, decals });
            }
        }
        self.label_gizmo
            .arm(label_gizmo::Target { material, index: 0 });
    }

    /// Put the active node's pivot on the surface under the cursor (§5).
    ///
    /// Stored in the node's own local space, so it travels with the part; a
    /// world-space pivot would be left behind the first time the part moved.
    fn set_pivot_at(&mut self, rect: egui::Rect, at: Option<egui::Pos2>) {
        let Some(node) = self.project.selection.active() else {
            self.project.status =
                "Select a part first, then Insert-click to place its pivot".into();
            return;
        };
        let Some(at) = at else { return };
        let Some(ray) = gizmo_ui::ray_through(&self.camera.camera, rect, at) else {
            return;
        };
        let Some(hit) = self.orchestrator.pick(&ray) else {
            self.project.status = "Insert-click on the model to place a pivot".into();
            return;
        };

        let local = self
            .project
            .document()
            .tree
            .world_transform(node)
            .inverted()
            .point(hit.position);
        if self
            .project
            .apply(Command::set_pivot(node, Some(local)))
            .is_ok()
        {
            self.gizmo.pivot_mode = PivotMode::Custom;
            self.project.status = "Pivot placed".into();
        }
    }

    /// The real-time view HUD.
    ///
    /// Placement, contents and the `H` toggle follow `docs/04-UX-SPEC.md`
    /// §2.1. Two of its entries are worth calling out:
    ///
    /// - **Percentage converged** is listed there as "a better progress signal
    ///   than sample count and worth showing prominently", so it leads.
    /// - **Whether the view is at reduced interactive resolution** — without
    ///   it, a user who has just stopped dragging sees a soft image and has no
    ///   way to tell whether that is the renderer catching up or the render
    ///   itself.
    fn hud(&self, ui: &egui::Ui, rect: egui::Rect) {
        let image = &self.image;
        let resolution = if image.resolution_divisor > 1 {
            format!(
                "{} x {} (1/{} interactive)",
                image.width, image.height, image.resolution_divisor
            )
        } else {
            format!("{} x {}", image.width, image.height)
        };
        let lines = [
            format!(
                "{:.0}% converged   {} spp",
                image.converged_fraction * 100.0,
                image.samples
            ),
            format!(
                "{:.1} s   {:.1} Mspp/s",
                image.elapsed_seconds,
                image.samples_per_second / 1.0e6
            ),
            resolution,
            format!("{} tris   {} instances", image.triangles, image.instances),
            format!(
                "CPU (Embree){}",
                if image.denoised { " · denoised" } else { "" }
            ),
            format!("first pixels {:.0} ms", image.milliseconds_to_first_pixel),
        ];

        // Top-right, per §2.1, under the shading island. One plate for the
        // lot rather than one per line: light text over a bright render is
        // unreadable, and a render is exactly where the bright parts are.
        let painter = ui.painter();
        let galleys: Vec<_> = lines
            .into_iter()
            .map(|line| painter.layout_no_wrap(line, egui::FontId::monospace(11.5), theme::p().text))
            .collect();
        let width = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
        let line_height = 17.0;
        let plate = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 12.0 - width - 20.0, rect.top() + OVERLAY_INSET + ISLAND + 8.0),
            egui::vec2(width + 20.0, galleys.len() as f32 * line_height + 12.0),
        );
        icons::island(painter, plate);
        let mut y = plate.top() + 6.0;
        for galley in galleys {
            // Right-aligned so the numbers do not jitter horizontally as
            // their width changes.
            painter.galley(egui::pos2(plate.right() - 10.0 - galley.size().x, y), galley, egui::Color32::PLACEHOLDER);
            y += line_height;
        }
    }

    /// The bar across the top of the window: the mark and the menus on the
    /// left, the document's name in the middle, and on the right the panels
    /// it can show and the two things a session is for — bringing a model
    /// in and rendering it out.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let bar = ui.max_rect();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (mark, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
            icons::paint(ui.painter(), mark, icons::Icon::Aperture, theme::p().accent);
            ui.label(egui::RichText::new("Tequila").font(theme::strong(14.0)).color(theme::p().text));
            ui.add_space(12.0);
            egui::menu::bar(ui, |ui| {
                ui.visuals_mut().widgets.inactive.fg_stroke.color = theme::p().text;
                ui.menu_button("File", |ui| {
                    if ui.button("Import model...").clicked() {
                        ui.close_menu();
                        self.ask_for_file(Action::Import);
                    }
                    ui.menu_button("Import texture pack", |ui| {
                        if let Some(folder) = texture_pack::menu(ui) {
                            self.pick_texture_pack(folder);
                        }
                    });
                    ui.separator();
                    if ui.button("New library material...").on_hover_text("Make a material for your library: from a type, the selected part, a texture pack or a file").clicked() {
                        ui.close_menu();
                        let selected = self.selected_material().map(|(_, s)| s);
                        self.library.new_material(selected.as_ref());
                    }
                    if ui.button("Import material file...").on_hover_text("A material package (.fsmat) or preset (.json) somebody shared, into your library").clicked() {
                        ui.close_menu();
                        if let Some(path) = library_dialogs::pick_material_file() {
                            self.import_material_file(&path);
                        }
                    }
                    ui.separator();
                    let selected = self.project.selection.active();
                    if ui
                        .add_enabled(selected.is_some(), egui::Button::new("Reload model from file"))
                        .on_hover_text("Update the selected part's model from its file, keeping materials, labels and names")
                        .on_disabled_hover_text("Select a part of the model to reload")
                        .clicked()
                    {
                        ui.close_menu();
                        if let Some(node) = selected {
                            self.update_model(node, false);
                        }
                    }
                    if ui.button("New project").on_hover_text(theme::keys("Ctrl+N")).clicked() {
                        ui.close_menu();
                        self.new_project();
                    }
                    if ui.button("Open project...").on_hover_text(theme::keys("Ctrl+O")).clicked() {
                        ui.close_menu();
                        self.ask_for_file(Action::Open);
                    }
                    let mut reopen = None;
                    let mut clear = false;
                    ui.add_enabled_ui(!self.recent.is_empty(), |ui| {
                        ui.menu_button("Open recent", |ui| {
                            for path in &self.recent {
                                if ui.button(recent::label(&self.recent, path)).on_hover_text(path.display().to_string()).clicked() {
                                    reopen = Some(path.clone());
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                            if ui.button("Clear the list").clicked() {
                                clear = true;
                                ui.close_menu();
                            }
                        });
                    });
                    if let Some(path) = reopen {
                        self.open_recent(path);
                    }
                    if clear {
                        self.forget_recent(None);
                    }
                    if ui.button("Save").on_hover_text(theme::keys("Ctrl+S")).clicked() {
                        match self.project.path.clone() {
                            Some(path) => self.save_to(&path),
                            None => self.ask_for_file(Action::SaveAs),
                        }
                        ui.close_menu();
                    }
                    if ui.button("Save as...").on_hover_text(theme::keys("Ctrl+Shift+S")).clicked() {
                        ui.close_menu();
                        self.ask_for_file(Action::SaveAs);
                    }
                    ui.separator();
                    if ui.button("Export glTF (.glb)...").on_hover_text("What is shown, for other 3D software and the web: materials as glTF PBR, procedurals and labels baked").clicked() {
                        ui.close_menu();
                        let name = format!("{}.glb", crate::render_window::project_name(&self.project.document().meta.name, self.project.path.as_deref()));
                        if let Some(path) = rfd::FileDialog::new().add_filter("glTF binary", &["glb"]).set_file_name(&name).save_file() {
                            self.project.status = match self.export_glb(&path) {
                                Ok((bytes, warnings)) => format!(
                                    "Exported {} ({:.1} MB){}",
                                    path.display(),
                                    bytes as f64 / 1e6,
                                    if warnings.is_empty() { String::new() } else { format!(" - {}", warnings.join("; ")) }
                                ),
                                Err(error) => error,
                            };
                        }
                    }
                    if ui.button("Export USD (.usda)...").on_hover_text("What is shown as a USD stage: a preview surface for every viewer, and the OpenPBR material whole for MaterialX renderers and Tequila").clicked() {
                        ui.close_menu();
                        let name = format!("{}.usda", crate::render_window::project_name(&self.project.document().meta.name, self.project.path.as_deref()));
                        if let Some(path) = rfd::FileDialog::new().add_filter("USD", &["usda"]).set_file_name(&name).save_file() {
                            self.project.status = match std::fs::write(&path, tequila_format::usd::export(self.project.document())) {
                                Ok(()) => format!("Exported {}", path.display()),
                                Err(error) => format!("Could not write {}: {error}", path.display()),
                            };
                        }
                    }
                    if ui.button("Export for the web...").on_hover_text("The configurator as a web page: live 3D, photos of every combination, or both").clicked() {
                        ui.close_menu();
                        self.web_plan = Some(WebPlan { live: true, photos: false });
                    }
                    if ui.button("Export spin viewer...").on_hover_text("Renders all the way round the product, and a page to turn them with a finger - for a product page").clicked() {
                        ui.close_menu();
                        self.spin_plan = Some(SpinPlan { options: tequila_format::spin::SpinOptions::default(), transparent: false });
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui
                        .add(crate::field::Checkbox::new(&mut self.show_model_edges, "Model edges"))
                        .on_hover_text("Draw the parts' edges in Solid and Material Preview: a CAD part's own B-rep edges, or a mesh's creases and borders")
                        .changed()
                    {
                        ui.close_menu();
                    }
                    if ui
                        .button("Toggle material library")
                        .on_hover_text("M")
                        .clicked()
                    {
                        self.toggle_library(ui.ctx().screen_rect().width());
                    }
                    ui.add(crate::field::Checkbox::new(&mut self.show_project, "Inspector"))
                        .on_hover_text("Space");
                    ui.add(crate::field::Checkbox::new(&mut self.full_viewport, "Full viewport"))
                        .on_hover_text("Tab");
                    ui.separator();
                    ui.add(crate::field::Checkbox::new(&mut self.show_hud, "Render statistics"))
                        .on_hover_text("H");
                    ui.add(crate::field::Checkbox::new(&mut self.show_render_frame, "Render frame"))
                        .on_hover_text("Show the view at the render's size and shape, so what you see is what renders. Set the size in the Render window.");
                    ui.add(crate::field::Checkbox::new(&mut self.show_axis_legend, "Axis legend"));
                    ui.add(crate::field::Checkbox::new(&mut self.camera_tab.show_grid, "Ground grid")).on_hover_text("XZ measurement guide at Y = 0. Does not appear in renders. Grid snapping is a separate transform option.");
                    ui.separator();
                    ui.menu_button("Standard view", |ui| {
                        for view in StandardView::ALL {
                            if ui
                                .button(view.label())
                                .on_hover_text(theme::keys(&format!("Ctrl + Alt + {}", view.digit())))
                                .clicked()
                            {
                                self.camera.set_standard_view(view);
                                self.submit();
                                ui.close_menu();
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("Frame selection").on_hover_text("F").clicked() {
                        self.frame(false);
                        ui.close_menu();
                    }
                    if ui.button("Frame all").on_hover_text("Shift+F").clicked() {
                        self.frame(true);
                        ui.close_menu();
                    }
                });

                ui.menu_button("Edit", |ui| {
                    let undo = self
                        .project
                        .bus()
                        .undo_name()
                        .map(|n| format!("Undo {n}"))
                        .unwrap_or_else(|| "Undo".into());
                    if ui
                        .add_enabled(self.project.can_undo(), egui::Button::new(undo))
                        .clicked()
                    {
                        self.undo();
                        ui.close_menu();
                    }
                    let redo = self
                        .project
                        .bus()
                        .redo_name()
                        .map(|n| format!("Redo {n}"))
                        .unwrap_or_else(|| "Redo".into());
                    if ui
                        .add_enabled(self.project.can_redo(), egui::Button::new(redo))
                        .clicked()
                    {
                        self.redo();
                        ui.close_menu();
                    }
                });

                ui.menu_button("Workspace", |ui| {
                    for (label, library, project, tab) in [
                        ("Studio", true, true, ProjectTab::Scene),
                        ("Materials", true, true, ProjectTab::Material),
                        ("Lighting", false, true, ProjectTab::Environment),
                        ("Camera & composition", false, true, ProjectTab::Camera),
                        ("Presentation", false, false, ProjectTab::Scene),
                    ] {
                        if ui.button(label).clicked() {
                            self.show_library = library;
                            self.show_project = project;
                            self.project_tab = tab;
                            self.full_viewport = !library && !project;
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Configurator").on_hover_text("Set up the product's options, rules and picker").clicked() {
                        self.reveal_configurator();
                        ui.close_menu();
                    }
                    if ui
                        .add(egui::Button::new("Present").shortcut_text("F5"))
                        .on_hover_text("The configurator over the live view, full screen. Esc leaves.")
                        .clicked()
                    {
                        self.start_presentation();
                        ui.close_menu();
                    }
                });
                let mut plugin_action = None;
                ui.menu_button("Plugins", |ui| {
                    plugin_action = plugins_ui::menu(ui, &self.plugins, &mut self.plugins_window);
                });
                if let Some(action) = plugin_action {
                    self.plugin_action(action);
                }
                ui.menu_button("Help", |ui| {
                    if ui.button("Navigation and shortcuts").clicked() {
                        self.show_help = true;
                        ui.close_menu();
                    }
                    if ui.button("Show the welcome card").clicked() {
                        let mut appearance = theme::appearance();
                        appearance.welcome_seen = false;
                        theme::set_appearance(ui.ctx(), appearance, true);
                        ui.close_menu();
                    }
                    ui.menu_button("Guides", |ui| {
                        for (title, packaged, docs, about) in GUIDES {
                            let file = shipped_file(packaged, &format!("docs/{docs}"));
                            if ui.add_enabled(file.is_some(), egui::Button::new(title)).on_hover_text(about).clicked() {
                                if let Some(file) = file {
                                    crate::library_tab::open_folder(&file);
                                }
                                ui.close_menu();
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("About Tequila").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
            let left_end = ui.min_rect().right();

            let right = ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    self.render_button(ui);
                    if icons::tip(icons::labelled(ui, icons::Icon::Import, "Import", false), "Import a model", "STEP, IGES, glTF, OBJ, STL, USD and IFC", "Ctrl + I")
                        .clicked()
                    {
                        self.ask_for_file(Action::Import);
                    }
                    if icons::tip(icons::button(ui, icons::Icon::Save, false, 30.0, egui::Sense::click()), "Save the project", "", "Ctrl + S")
                        .clicked()
                    {
                        match self.project.path.clone() {
                            Some(path) => self.save_to(&path),
                            None => self.ask_for_file(Action::SaveAs),
                        }
                    }
                    icons::vertical_divider(ui, 18.0);
                    // Light or dark, and the theme: the sun or the moon,
                    // whichever the interface is in now.
                    let look = icons::button(
                        ui,
                        if theme::p().dark { icons::Icon::Moon } else { icons::Icon::Sun },
                        false,
                        30.0,
                        egui::Sense::click(),
                    );
                    let popup = ui.id().with("appearance");
                    if look.clicked() {
                        ui.memory_mut(|m| m.toggle_popup(popup));
                    }
                    match self.open_look.take() {
                        Some(true) => ui.memory_mut(|m| m.open_popup(popup)),
                        Some(false) => ui.memory_mut(|m| m.close_popup()),
                        None => {}
                    }
                    let look = icons::tip(look, "Appearance", "Light or dark, the theme, and the size of everything", "");
                    egui::popup::popup_below_widget(ui, popup, &look, egui::PopupCloseBehavior::CloseOnClickOutside, appearance_popover);
                    icons::vertical_divider(ui, 18.0);
                    // The panels, in the order they sit across the window
                    // (laid out right to left), named when there is room.
                    let named = ui.ctx().screen_rect().width() >= 1280.0;
                    if icons::tip(
                        icons::toggle(ui, icons::Icon::Assistant, "Assistant", self.assistant.open, named),
                        "Assistant",
                        "Ask an AI — Claude, OpenAI, a local model or your company's — to set up the scene",
                        "",
                    )
                    .clicked()
                    {
                        self.assistant.toggle();
                    }
                    if icons::tip(
                        icons::toggle(ui, icons::Icon::Console, "Console", self.console.open, named),
                        "Console",
                        "Type commands at the scene, and watch scripts act on it",
                        "`",
                    )
                    .clicked()
                    {
                        self.console.toggle();
                    }
                    let library_visible = self.show_library
                        && (ui.ctx().screen_rect().width() >= 1100.0 || !self.show_project);
                    if icons::tip(
                        icons::toggle(ui, icons::Icon::Library, "Library", library_visible, named),
                        "Material library",
                        "Drag a finish onto any part of the model",
                        "M",
                    )
                    .clicked()
                    {
                        self.toggle_library(ui.ctx().screen_rect().width());
                    }
                    // A light, not a notification: something outside the
                    // application is changing this scene, and the person in
                    // front of it should be able to see that at a glance
                    // rather than by opening a panel.
                    if self.activity.active()
                        && icons::busy_pill(ui, "Scripting")
                            .on_hover_text("Something is driving this session over the scripting channel. Click to watch it.")
                            .clicked()
                    {
                        self.console.show_and_focus();
                    }
                })
                .response
                .rect;

            // The document, centred on the window where there is room for
            // it, and nudged into the gap between the clusters where there
            // is less.
            let name = self
                .project
                .path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.project.document().meta.name.clone());
            let painter = ui.painter();
            let title = painter.layout_no_wrap(name, egui::FontId::proportional(13.0), theme::p().text_dim);
            let edited = self.project.has_unsaved_changes();
            let width = title.size().x + if edited { 14.0 } else { 0.0 };
            let (low, high) = (left_end + 24.0, right.left() - 24.0 - width);
            if high > low {
                let x = (bar.center().x - width * 0.5).clamp(low, high);
                let y = bar.center().y - title.size().y * 0.5;
                painter.galley(egui::pos2(x, y), title.clone(), egui::Color32::PLACEHOLDER);
                if edited {
                    painter.circle_filled(egui::pos2(x + title.size().x + 9.0, bar.center().y), 3.0, theme::p().text_dim);
                }
            }
        });
    }

    /// The one filled button in the window. While a render is running it
    /// says how far along it is, and opens its progress instead.
    fn render_button(&mut self, ui: &mut egui::Ui) {
        let progress = self
            .project
            .queue()
            .jobs()
            .iter()
            .find(|j| j.state == tequila_scene::render::JobState::Rendering)
            .map(render_window::progress);
        let label = match progress {
            Some(done) => format!("Rendering {:.0}%", done * 100.0),
            None => "Render".to_string(),
        };
        let response = icons::labelled(ui, icons::Icon::Aperture, &label, true);
        if let Some(done) = progress {
            let rect = response.rect.shrink2(egui::vec2(8.0, 0.0));
            let track = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 4.0), egui::pos2(rect.right(), rect.bottom() - 2.0));
            ui.painter().rect_filled(track, 1.0, theme::p().on_accent.gamma_multiply(0.25));
            ui.painter().rect_filled(
                egui::Rect::from_min_max(track.min, egui::pos2(track.left() + track.width() * done.clamp(0.0, 1.0), track.bottom())),
                1.0,
                theme::p().on_accent,
            );
        }
        let response = if progress.is_some() {
            icons::tip(response, "Rendering", "Show the render's live preview and progress", "")
        } else {
            icons::tip(response, "Render an image", "Output size, quality and file format", "Ctrl + R")
        };
        if response.clicked() {
            if progress.is_some() {
                self.render_window.show_progress();
            } else {
                self.render_window.open = true;
            }
        }
    }

    /// The tools that float over the picture, as islands rather than a
    /// toolbar across the top: the picture keeps the whole height of its
    /// panel, and each control sits beside what it acts on — shading and
    /// framing at the top right, as Blender has them, the transform tools
    /// down the left.
    ///
    /// Registered after the viewport's own response, so they are on top of
    /// it in egui's hit test, and sensing drags as well as clicks, so a drag
    /// that starts on one does not turn the camera.
    fn viewport_overlays(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        use icons::{Icon, Tone};
        let sense = egui::Sense::click_and_drag();
        let has_selection = self.project.selection.active().is_some();
        let painter = ui.painter().clone();
        let button = |ui: &mut egui::Ui, at: egui::Rect, name: &str, icon: Icon, on: bool, enabled: bool| {
            let response = ui.interact(at, ui.id().with(("viewport-tool", name)), sense);
            icons::paint_button(ui, at, &response, icon, if on { Tone::On } else { Tone::Quiet }, enabled);
            response
        };

        // ---- What the viewport is showing: top left ---------------------
        let image = &self.image;
        let rendered = self.view_mode == view_mode::ViewMode::Rendered;
        let exporting = self.batch.is_running() && rendered;
        let (title, detail) = match self.view_mode {
            view_mode::ViewMode::Wireframe => ("Wireframe", String::new()),
            view_mode::ViewMode::Solid => ("Solid", "studio light".to_string()),
            view_mode::ViewMode::MaterialPreview => (
                "Material Preview",
                if self.gpu_solid() { "environment light, no shadows" } else { "studio light" }.to_string(),
            ),
            view_mode::ViewMode::Rendered => (
                "Rendered",
                if exporting {
                    "paused for the export".to_string()
                } else if self.viewport_paused {
                    "paused".to_string()
                } else if let Some(orchestrator::EngineNote::Preparing(doing)) = &self.engine_note {
                    preparing_label(doing)
                } else if let Some(orchestrator::EngineNote::FellBack { .. }) = &self.engine_note {
                    format!("{} spp · on the processor: the graphics card could not render", image.samples)
                } else if image.converged_fraction >= 0.01 {
                    format!("{} spp · {:.0}% settled", image.samples, image.converged_fraction * 100.0)
                } else {
                    format!("{} spp", image.samples)
                },
            ),
        };
        let converged = image.converged_fraction >= 0.995;
        let title = painter.layout_no_wrap(title.to_string(), theme::strong(12.0), theme::p().text);
        let detail = painter.layout_no_wrap(detail, egui::FontId::proportional(12.0), theme::p().text_dim);
        let chip = egui::Rect::from_min_size(
            rect.min + egui::Vec2::splat(OVERLAY_INSET),
            egui::vec2(28.0 + title.size().x + if detail.is_empty() { 12.0 } else { 8.0 + detail.size().x + 12.0 }, ISLAND - 8.0),
        );
        icons::island(&painter, chip);
        let dot = egui::pos2(chip.left() + 15.0, chip.center().y);
        if rendered {
            let colour = if exporting || self.viewport_paused {
                theme::p().text_faint
            } else if converged {
                theme::p().accent
            } else {
                theme::p().busy
            };
            painter.circle_filled(dot, 3.5, colour);
        } else {
            // Hollow: a preview, not the render — a material is not to be
            // judged by it, and the screen keeps saying so.
            painter.circle_stroke(dot, 3.0, egui::Stroke::new(1.5_f32, theme::p().text_dim));
        }
        let text_y = chip.center().y - title.size().y * 0.5;
        painter.galley(egui::pos2(chip.left() + 26.0, text_y), title.clone(), egui::Color32::PLACEHOLDER);
        if !detail.is_empty() {
            painter.galley(egui::pos2(chip.left() + 26.0 + title.size().x + 8.0, text_y), detail, egui::Color32::PLACEHOLDER);
        }
        if rendered && !converged && !exporting && !self.viewport_paused {
            let track = egui::Rect::from_min_max(
                egui::pos2(chip.left() + 10.0, chip.bottom() - 3.0),
                egui::pos2(chip.right() - 10.0, chip.bottom() - 1.5),
            );
            painter.rect_filled(
                egui::Rect::from_min_max(track.min, egui::pos2(track.left() + track.width() * image.converged_fraction.clamp(0.0, 1.0), track.max.y)),
                1.0,
                theme::p().busy.gamma_multiply(0.8),
            );
        }

        // ---- Framing, shading and pause: top right ----------------------
        let (pad, gap) = (4.0, 9.0);
        let width = pad * 2.0 + TOOL * 7.0 + gap * 2.0;
        let island = egui::Rect::from_min_size(
            egui::pos2(rect.right() - OVERLAY_INSET - width, rect.top() + OVERLAY_INSET),
            egui::vec2(width, ISLAND),
        );
        icons::island(&painter, island);
        let _ = ui.interact(island, ui.id().with("shading-island"), sense);
        let mut x = island.left() + pad;
        let y = island.top() + pad;
        let next = |x: &mut f32| {
            let at = egui::Rect::from_min_size(egui::pos2(*x, y), egui::Vec2::splat(TOOL));
            *x += TOOL;
            at
        };
        if icons::tip(button(ui, next(&mut x), "frame-all", Icon::FrameAll, false, true), "Frame all", "Fit the whole scene in the view", "Shift + F")
            .clicked()
        {
            self.frame(true);
        }
        let response = button(ui, next(&mut x), "frame-selection", Icon::FrameSelected, false, has_selection);
        let response = icons::tip(
            response,
            "Frame the selection",
            if has_selection { "Fit the selected parts in the view" } else { "Select a part first" },
            "F",
        );
        if response.clicked() && has_selection {
            self.frame(false);
        }
        icons::divider(&painter, egui::pos2(x + gap * 0.5, island.top() + 9.0), egui::pos2(x + gap * 0.5, island.bottom() - 9.0));
        x += gap;
        for mode in view_mode::ViewMode::ALL {
            let icon = match mode {
                view_mode::ViewMode::Wireframe => Icon::Wireframe,
                view_mode::ViewMode::Solid => Icon::Solid,
                view_mode::ViewMode::MaterialPreview => Icon::MaterialPreview,
                view_mode::ViewMode::Rendered => Icon::Rendered,
            };
            if icons::tip(button(ui, next(&mut x), mode.key(), icon, self.view_mode == mode, true), mode.label(), mode.hint(), "")
                .clicked()
            {
                self.set_view_mode(mode);
            }
        }
        icons::divider(&painter, egui::pos2(x + gap * 0.5, island.top() + 9.0), egui::pos2(x + gap * 0.5, island.bottom() - 9.0));
        x += gap;
        let paused = self.viewport_paused;
        if icons::tip(
            button(ui, next(&mut x), "pause", if paused { Icon::Play } else { Icon::Pause }, paused, true),
            if paused { "Resume" } else { "Pause" },
            if paused { "Carry on rendering the viewport" } else { "Stop rendering the viewport, keeping the samples so far" },
            "P",
        )
        .clicked()
        {
            self.viewport_paused = !paused;
        }

        // ---- Transform tools: down the left -----------------------------
        let strip = egui::Rect::from_min_size(
            egui::pos2(rect.left() + OVERLAY_INSET, chip.bottom() + 10.0),
            egui::vec2(ISLAND, pad * 2.0 + TOOL * 5.0 + gap),
        );
        icons::island(&painter, strip);
        let _ = ui.interact(strip, ui.id().with("tool-strip"), sense);
        let x = strip.left() + pad;
        let mut y = strip.top() + pad;
        let next = |y: &mut f32| {
            let at = egui::Rect::from_min_size(egui::pos2(x, *y), egui::Vec2::splat(TOOL));
            *y += TOOL;
            at
        };
        // Greyed out with nothing selected, because the handles have
        // nowhere to be: pressing them then did nothing at all, and a button
        // that does nothing when pressed reads as broken rather than as
        // inapplicable.
        for (mode, icon) in [(GizmoMode::Move, Icon::Move), (GizmoMode::Rotate, Icon::Rotate), (GizmoMode::Scale, Icon::Scale)] {
            let response = button(ui, next(&mut y), mode.label(), icon, has_selection && self.gizmo.mode == mode, has_selection);
            let response = icons::tip(
                response,
                mode.label(),
                if has_selection { "Drag a handle in the viewport" } else { "Select a part first" },
                &mode.hotkey().to_string(),
            );
            if response.clicked() && has_selection {
                self.gizmo.mode = mode;
            }
        }
        icons::divider(&painter, egui::pos2(strip.left() + 9.0, y + gap * 0.5), egui::pos2(strip.right() - 9.0, y + gap * 0.5));
        y += gap;
        let local = self.gizmo.local_space;
        if icons::tip(
            button(ui, next(&mut y), "space", if local { Icon::Local } else { Icon::World }, false, true),
            if local { "Local axes" } else { "World axes" },
            if local {
                "The handles follow the part's own axes. Click for the world's."
            } else {
                "The handles follow the world's axes. Click for the part's own."
            },
            "",
        )
        .clicked()
        {
            self.gizmo.local_space = !local;
        }
        let snapping = self.gizmo.snapping.grid.is_some()
            || self.gizmo.snapping.angle.is_some()
            || self.gizmo.snapping.vertex_within.is_some();
        let snap = button(ui, next(&mut y), "snapping", Icon::Magnet, snapping, true);
        let popup = ui.id().with("snapping-popup");
        if snap.clicked() {
            ui.memory_mut(|m| m.toggle_popup(popup));
        }
        let snap = icons::tip(snap, "Snapping", "To the grid, to angles, and to other parts' corners", "");
        egui::popup::popup_above_or_below_widget(
            ui,
            popup,
            &snap,
            egui::AboveOrBelow::Below,
            egui::PopupCloseBehavior::CloseOnClickOutside,
            |ui| {
                ui.set_min_width(190.0);
                theme::section(ui, "Snapping");
                self.snapping_controls(ui);
            },
        );
    }

    /// Three steps to a first picture, over the bottom of the viewport until
    /// they are put away: bring in a model, dress it, render it.
    fn welcome_card(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        if theme::appearance().welcome_seen || rect.width() < 520.0 || rect.height() < 380.0 {
            return;
        }
        let p = theme::p();
        let size = egui::vec2(560.0_f32.min(rect.width() - 48.0), 178.0);
        let card = egui::Rect::from_center_size(egui::pos2(rect.center().x, rect.bottom() - 28.0 - size.y * 0.5), size);
        let painter = ui.painter().clone();
        painter.add(egui::epaint::Shadow { offset: egui::vec2(0.0, 10.0), blur: 34.0, spread: 0.0, color: egui::Color32::from_black_alpha(if p.dark { 120 } else { 50 }) }.as_shape(card, 16.0));
        painter.rect(card, 16.0, p.raised, egui::Stroke::new(1.0_f32, p.line));
        // The card's own clicks and drags, so none of them turns the camera.
        let _ = ui.interact(card, ui.id().with("welcome"), egui::Sense::click_and_drag());

        let mut dismiss = false;
        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(card.shrink2(egui::vec2(20.0, 16.0))), |ui| {
            ui.horizontal(|ui| {
                let (mark, _) = ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::hover());
                icons::paint(ui.painter(), mark, icons::Icon::Aperture, p.accent);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(egui::RichText::new("Welcome to Tequila").font(theme::strong(16.0)).color(p.text));
                    ui.label(egui::RichText::new("Three steps to your first picture").size(12.0).color(p.text_dim));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if icons::button(ui, icons::Icon::Close, false, 26.0, egui::Sense::click()).on_hover_text("Put this away").clicked() {
                        dismiss = true;
                    }
                });
            });
            ui.add_space(12.0);
            let gap = 12.0;
            let each = (ui.available_width() - gap * 2.0) / 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let steps: [(theme::Hue, &str, &str, icons::Icon, &str); 3] = [
                    (theme::Hue::Blue, "Bring in a model", "OBJ, STL, glTF, STEP and more", icons::Icon::Import, "Import"),
                    (theme::Hue::Pink, "Dress it", "Drag a material onto any part", icons::Icon::Library, "Library"),
                    (theme::Hue::Amber, "Take the shot", "Size, quality and file", icons::Icon::Aperture, "Render"),
                ];
                for (number, (hue, title, detail, icon, action)) in steps.into_iter().enumerate() {
                    ui.allocate_ui_with_layout(egui::vec2(each, 96.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(each);
                        let colour = theme::hue(hue);
                        ui.horizontal(|ui| {
                            let (dot, _) = ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 11.0, colour.gamma_multiply(if p.dark { 0.22 } else { 0.16 }));
                            ui.painter().text(dot.center(), egui::Align2::CENTER_CENTER, format!("{}", number + 1), theme::strong(12.0), colour);
                            ui.label(egui::RichText::new(title).font(theme::strong(13.0)).color(p.text));
                        });
                        ui.label(egui::RichText::new(detail).size(11.5).color(p.text_dim));
                        ui.add_space(2.0);
                        if icons::labelled(ui, icon, action, number == 2).clicked() {
                            match number {
                                0 => self.ask_for_file(Action::Import),
                                1 => {
                                    self.show_library = true;
                                    if ui.ctx().screen_rect().width() < 1100.0 {
                                        self.show_project = false;
                                    }
                                }
                                _ => self.render_window.open = true,
                            }
                        }
                    });
                }
            });
        });
        if dismiss {
            let mut appearance = theme::appearance();
            appearance.welcome_seen = true;
            theme::set_appearance(ui.ctx(), appearance, true);
        }
    }

    /// Grid, angle and vertex snapping for the transform gizmo.
    fn snapping_controls(&mut self, ui: &mut egui::Ui) {
        let mut snap = self.gizmo.snapping.grid.is_some();
        if ui.add(crate::field::Checkbox::new(&mut snap, "Grid snap")).changed() {
            self.gizmo.snapping.grid = snap.then_some(1.0);
        }
        if let Some(step) = &mut self.gizmo.snapping.grid {
            ui.add(egui::DragValue::new(step).speed(0.1).range(0.001..=1000.0));
        }
        let mut angle = self.gizmo.snapping.angle.is_some();
        if ui.add(crate::field::Checkbox::new(&mut angle, "Angle snap")).changed() {
            self.gizmo.snapping.angle = angle.then_some(15f32.to_radians());
        }

        // §5's vertex snapping. The gizmo has implemented it since M1 —
        // resolved against the grabbed node and applied as one rigid shift
        // to the whole selection, so the parts do not come apart — and the
        // field was only ever `None`, so no drag ever snapped to anything.
        let mut vertex = self.gizmo.snapping.vertex_within.is_some();
        if ui
            .add(crate::field::Checkbox::new(&mut vertex, "Vertex snap"))
            .on_hover_text(
                "While dragging, land on the nearest vertex within the \
                 radius. The whole selection shifts together, so parts \
                 stay in their arrangement.",
            )
            .changed()
        {
            // A radius, not a toggle: "nearest vertex" with no bound would
            // teleport a part across the scene to whatever happened to be
            // closest. Scaled to the scene so it means the same thing on a
            // watch and on a car.
            let extent = tequila_scene::snapshot::visible_bounds(self.project.document())
                .diagonal()
                .length();
            let radius = if extent > 0.0 { extent * 0.02 } else { 0.1 };
            self.gizmo.snapping.vertex_within = vertex.then_some(radius);
        }
        if let Some(radius) = &mut self.gizmo.snapping.vertex_within {
            ui.add(
                egui::DragValue::new(radius)
                    .speed(0.01)
                    .range(1e-4..=1e6)
                    .max_decimals(4),
            )
            .on_hover_text("Snap radius, in metres.");
        }
    }

    /// §5's numeric entry, in absolute world coordinates.
    fn transform_panel(&mut self, ui: &mut egui::Ui) {
        let Some(node) = self.project.selection.active() else {
            ui.label(egui::RichText::new("Nothing selected").weak());
            return;
        };
        let Some(current) = self.project.document().tree.get(node).map(|n| n.transform) else {
            return;
        };
        let name = self
            .project
            .document()
            .tree
            .get(node)
            .map(|n| n.name.clone())
            .unwrap_or_default();

        crate::field::Fold::new(format!("Transform — {name}"))
            .default_open(true)
            .show(ui, |ui| {
                // The fields take §1.2's units and expressions: a position
                // typed as `2.5in` from a drawing, or `= 120/2` when somebody
                // is halving a dimension. They read in the unit the scene
                // shows lengths in, so a suffix converts into that, and the
                // position itself is in metres, like every length.
                let shown = self.project.document().units.shown();
                let per_unit = shown.to_metres() as f32;
                let unit = numeric::Unit::from_scene(shown);
                let mut position = current.point(Vec3::ZERO) / per_unit;
                let before = position;
                let mut committed = false;
                ui.horizontal(|ui| {
                    ui.label("Position");
                    for value in [&mut position.x, &mut position.y, &mut position.z] {
                        let field = ui.add(numeric::drag_value(value, unit).speed(0.01));
                        // Dragging the field is one interaction; releasing it,
                        // or committing a typed number, ends it. Without this
                        // two values typed in turn become one undo step and
                        // the first is silently lost.
                        committed |= field.drag_stopped() || field.lost_focus();
                    }
                });
                if position != before {
                    // From what changed, so an axis nobody touched does not
                    // move by the rounding of a round trip through the unit.
                    let offset = (position - before) * per_unit;
                    if let Some(transform) = tequila_core::math::Transform::from_matrix(
                        tequila_core::math::Mat4::translation(offset) * current.matrix,
                    ) {
                        let _ = self.project.apply(Command::set_transform(node, transform));
                    }
                }
                if committed {
                    self.project.end_interaction();
                }

                ui.horizontal(|ui| {
                    if ui
                        .button("Drop to ground")
                        .on_hover_text("Snap to ground (§5)")
                        .clicked()
                    {
                        if let Some(transform) = tequila_scene::gizmo::snap_to_ground(
                            self.project.document(),
                            node,
                            0.0,
                        ) {
                            let _ = self.project.apply(Command::set_transform(node, transform));
                        }
                    }
                    if ui.button("Reset").clicked() {
                        let _ = self.project.apply(Command::set_transform(
                            node,
                            tequila_core::math::Transform::IDENTITY,
                        ));
                    }
                });

                ui.horizontal_wrapped(|ui| {
                    ui.label("Pivot");
                    for mode in [
                        PivotMode::BoundingBoxCenter,
                        PivotMode::Origin,
                        PivotMode::Corner,
                        PivotMode::Custom,
                    ] {
                        if ui
                            .selectable_label(self.gizmo.pivot_mode == mode, mode.label())
                            .clicked()
                        {
                            self.gizmo.pivot_mode = mode;
                        }
                    }
                });
                let has_custom = self
                    .project
                    .document()
                    .tree
                    .get(node)
                    .is_some_and(|n| n.pivot.is_some());
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(if has_custom {
                            "Insert-click the model to move the pivot"
                        } else {
                            "Insert-click the model to place a pivot"
                        })
                        .small()
                        .weak(),
                    );
                    if ui
                        .add_enabled(has_custom, egui::Button::new("Clear"))
                        .clicked()
                    {
                        let _ = self.project.apply(Command::set_pivot(node, None));
                    }
                });
            });
    }

    /// The window's bottom edge: what is selected and what just happened on
    /// the left, the viewport's engine and size on the right. Small and dim:
    /// it is read when looked for, and the viewport's own chip carries the
    /// render's state where the eye already is.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let small = |text: String, colour: egui::Color32| egui::RichText::new(text).size(11.5).color(colour);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let image = &self.image;
            let engine = if image.shading != orchestrator::Shading::Fast {
                "Path traced"
            } else if self.gpu_solid() {
                "Graphics card"
            } else {
                "CPU preview"
            };
            // The graphics card draws straight to the viewport, leaving no
            // image behind to measure: its size is the viewport's, in pixels.
            let (width, height) = if image.width > 0 {
                (image.width as usize, image.height as usize)
            } else {
                let ppp = ui.ctx().pixels_per_point();
                ((self.viewport_size.0 as f32 * ppp).round() as usize, (self.viewport_size.1 as f32 * ppp).round() as usize)
            };
            icons::tip(
                ui.label(small(format!("{engine} · {width} × {height}"), theme::p().text_faint)),
                "Rendering statistics",
                "Samples, timings and memory, over the viewport",
                "H",
            );
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                if let Some(job) = self.project.queue().jobs().iter().find(|j| j.state == tequila_scene::render::JobState::Rendering) {
                    let label = format!("Export {:.0}% · {}", render_window::progress(job) * 100.0, job.message);
                    if ui
                        .add(egui::Label::new(small(label, theme::p().busy)).sense(egui::Sense::click()))
                        .on_hover_text("Show the render's live preview and progress")
                        .clicked()
                    {
                        self.render_window.show_progress();
                    }
                }
                // §4.1 requires the selection level to be shown.
                let selection = self.project.selection.summary(self.project.document());
                ui.label(small(selection, theme::p().text_dim));
                if !self.project.status.is_empty() {
                    let status = self.project.status.clone();
                    ui.add(egui::Label::new(small(status.clone(), theme::p().text_dim)).truncate())
                        .on_hover_text(status);
                } else {
                    // Nothing to report: one of the things worth knowing,
                    // a new one every little while.
                    let tick = (ui.input(|i| i.time) / 40.0) as usize;
                    let tip = TIPS[tick % TIPS.len()];
                    ui.add(egui::Label::new(small(format!("Tip: {tip}"), theme::p().text_faint)).truncate());
                }
            });
        });
    }

    /// §4.5: "On crash, offer to replay the journal onto the last saved
    /// state." Offered, not done automatically — replaying onto the wrong
    /// document would make a mess the user did not ask for.
    fn recovery_window(&mut self, ctx: &egui::Context) {
        let sessions = self.recoverable.clone();
        let mut dismiss = false;

        let p = theme::p();
        egui::Window::new("Recover unsaved work")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .frame(egui::Frame::window(&ctx.style()).inner_margin(egui::Margin::same(18.0)))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.horizontal(|ui| {
                    let (badge, _) = ui.allocate_exact_size(egui::Vec2::splat(36.0), egui::Sense::hover());
                    theme::badge(ui.painter(), badge, icons::Icon::History, p.busy);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(egui::RichText::new("Recover unsaved work").font(theme::strong(15.0)).color(p.text));
                        ui.label(
                            egui::RichText::new("Tequila did not shut down cleanly, and kept what you had not saved.")
                                .size(12.0)
                                .color(p.text_dim),
                        );
                    });
                });
                ui.add_space(12.0);
                for session in &sessions {
                    // The person's project by name; the file recovery
                    // continues from can be an auto-save snapshot whose
                    // name means nothing to them.
                    let describes = match session.project.as_ref().or(session.document.as_ref()) {
                        Some(path) if session.project.is_some() => path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| path.display().to_string()),
                        _ => "an unsaved project".to_string(),
                    };
                    let later = match session.commands {
                        0 => String::new(),
                        1 => ", plus 1 later change".to_string(),
                        n => format!(", plus {n} later changes"),
                    };
                    let title = {
                        let mut chars = describes.chars();
                        chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
                    };
                    let detail = match session.snapshot_time {
                        Some(when) => format!("Auto-saved at {}{later}", crate::clock::clock_of(when)),
                        None => format!(
                            "{} unsaved change{}",
                            session.commands,
                            if session.commands == 1 { "" } else { "s" }
                        ),
                    };
                    egui::Frame::none()
                        .fill(p.card)
                        .stroke(egui::Stroke::new(1.0_f32, p.line))
                        .rounding(10.0)
                        .inner_margin(egui::Margin::symmetric(14.0, 12.0))
                        .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.label(egui::RichText::new(title).font(theme::strong(13.0)).color(p.text));
                                ui.label(egui::RichText::new(detail).size(12.0).color(p.text_dim));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                if icons::labelled(ui, icons::Icon::History, "Recover", true).clicked() {
                                    dismiss = self.recover_session(session);
                                }
                                if icons::tip(
                                    icons::labelled(ui, icons::Icon::Close, "Discard", false),
                                    "Discard",
                                    "Forget these changes. Your saved project file is not touched.",
                                    "",
                                )
                                .clicked()
                                {
                                    // The journal and the auto-save snapshot only.
                                    // The user's own project file is never touched:
                                    // "discard the unsaved work" must not mean
                                    // "delete what was saved".
                                    if let Some(directory) = session.journal.parent() {
                                        tequila_format::journal::forget(directory, &session.id);
                                    }
                                    dismiss = true;
                                }
                            });
                        });
                    });
                    ui.add_space(6.0);
                }
                ui.add_space(2.0);
                // A row, not the rest of the window: a right-to-left layout
                // given the window's space takes all of its height.
                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 24.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(egui::RichText::new("Ignore for now").color(p.text_dim)).frame(false))
                        .on_hover_text("Ask again the next time Tequila starts")
                        .clicked()
                    {
                        dismiss = true;
                    }
                });
            });

        if dismiss {
            self.recoverable.clear();
        }
    }

    /// Bring back a crashed session's work: open what its journal continues
    /// from, replay the journal onto that, and only then forget the crashed
    /// session's files. False, leaving the prompt up, when it could not be
    /// done; nothing is deleted then.
    fn recover_session(&mut self, session: &tequila_format::journal::Recoverable) -> bool {
        // Recovery replaces the open document: the prompt is not modal, and
        // a project opened since launch may have edits of its own.
        if !self.confirm_unsaved() {
            return false;
        }
        // The journal holds the commands since the last save or auto-save,
        // so they are replayed onto that same file. Onto anything else they
        // build a scene that never existed.
        let base = session.document.clone().or(session.snapshot.clone());
        match &base {
            Some(base) => {
                if let Err(error) = self.project.open(base) {
                    self.project.status = format!("Nothing was recovered: {} would not open ({error})", base.display());
                    self.toasts.warn(self.project.status.clone());
                    return false;
                }
            }
            // Never saved and never auto-saved: the session started from the
            // starter scene, as every session does - not from whatever this
            // one has open now.
            None => self.project.reset_to(starter_document()),
        }
        if let Err(error) = self.project.recover(&session.journal) {
            self.project.status = format!("Nothing was recovered: {error}");
            self.toasts.warn(self.project.status.clone());
            return false;
        }
        // Recovered into this session, which has snapshotted it: the crashed
        // session's files have done their job. Left in place, the same
        // session was offered again on the next launch.
        if let Some(directory) = session.journal.parent() {
            tequila_format::journal::forget(directory, &session.id);
        }
        if base.is_none() || base == session.snapshot {
            // An auto-save snapshot is not the file the person was editing:
            // Save asks where to put it rather than overwriting the snapshot,
            // and closing asks before throwing it away, however few commands
            // the journal added to it.
            self.project.path = None;
            self.project.mark_unsaved();
        }
        // The recovered document's camera, as opening a project adopts it;
        // the viewport would otherwise write the old view over it next frame.
        self.camera.adopt(self.project.camera());
        self.submit();
        true
    }

    /// §6's bindings that are not tied to the viewport.
    fn global_hotkeys(&mut self, ctx: &egui::Context) {
        // Undo, redo and save pressed in a window of its own: they mean the
        // same there as here, and only here is there a project to save.
        for key in windows::take_keys(ctx) {
            match key {
                windows::Shared::Undo => self.undo(),
                windows::Shared::Redo => self.redo(),
                windows::Shared::Save => match self.project.path.clone() {
                    Some(path) => self.save_to(&path),
                    None => self.ask_for_file(Action::SaveAs),
                },
            }
        }
        // Not while a text field has focus, or typing "w" in a rename box
        // switches the gizmo mode.
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let modifiers = ctx.input(|i| i.modifiers);
        let pressed = |key| ctx.input(|i| i.key_pressed(key));
        // F5 presents, as it does a slide show.
        if pressed(egui::Key::F5) && !modifiers.any() && self.presenting.is_none() {
            self.start_presentation();
        }
        // Over the graph editor, its own keys mean a node, not a part: only
        // undo, redo and save reach through, which mean the same everywhere.
        // Undo stopped working altogether over the editor before this.
        if self.graph_editor.hovered {
            if modifiers.command && pressed(egui::Key::Z) {
                if modifiers.shift {
                    self.redo();
                } else {
                    self.undo();
                }
            }
            if modifiers.command && pressed(egui::Key::Y) {
                self.redo();
            }
            if modifiers.command && pressed(egui::Key::S) {
                match self.project.path.clone() {
                    Some(path) => self.save_to(&path),
                    None => self.ask_for_file(Action::SaveAs),
                }
            }
            return;
        }

        if modifiers.is_none() && pressed(egui::Key::P) {
            self.viewport_paused = !self.viewport_paused;
        }

        // The key every game and most editors open their console with.
        if modifiers.is_none() && pressed(egui::Key::Backtick) {
            self.console.toggle();
        }

        if modifiers.command && pressed(egui::Key::Z) {
            if modifiers.shift {
                self.redo();
            } else {
                self.undo();
            }
        }
        if modifiers.command && pressed(egui::Key::Y) {
            self.redo();
        }
        if modifiers.command && pressed(egui::Key::A) {
            let mut selection = std::mem::take(&mut self.project.selection);
            selection.select_all(self.project.document());
            self.project.selection = selection;
        }
        if modifiers.command && pressed(egui::Key::G) {
            let nodes: Vec<_> = self.project.selection.nodes().collect();
            if nodes.len() >= 2 {
                let _ = self.project.apply(Command::Group {
                    nodes,
                    name: "Group".into(),
                });
            }
        }
        if modifiers.command && pressed(egui::Key::S) {
            match (modifiers.shift, self.project.path.clone()) {
                (false, Some(path)) => self.save_to(&path),
                _ => self.ask_for_file(Action::SaveAs),
            }
        }
        if modifiers.command && !modifiers.shift && pressed(egui::Key::N) {
            self.new_project();
        }
        if modifiers.command && !modifiers.shift && pressed(egui::Key::O) {
            self.ask_for_file(Action::Open);
        }
        if pressed(egui::Key::Escape) {
            self.project.selection.clear();
        }
        if pressed(egui::Key::Delete) || pressed(egui::Key::Backspace) {
            let nodes: Vec<_> = self.project.selection.nodes().collect();
            if !nodes.is_empty() {
                let _ = self.project.apply(Command::DeleteNodes { nodes });
            }
        }
        // §6.2's editing bindings that take a modifier. On a Mac, Cmd+H is
        // the system's: it hides the application before Tequila sees it.
        if pressed(egui::Key::H) && (modifiers.command || (cfg!(target_os = "macos") && modifiers.ctrl)) {
            self.hide_selection();
        }
        if modifiers.alt && pressed(egui::Key::H) {
            self.unhide_all();
        }
        if modifiers.command && pressed(egui::Key::R) {
            self.render_window.open = true;
        }
        if modifiers.command && pressed(egui::Key::I) {
            self.ask_for_file(Action::Import);
        }
        if modifiers.shift && pressed(egui::Key::F) {
            self.frame(true);
        }

        if !modifiers.any() {
            if pressed(egui::Key::W) {
                self.gizmo.mode = GizmoMode::Move;
            }
            // §6.2 records this conflict and its resolution: `E` is both
            // Rotate and "background = Environment". The gizmo wins when there
            // is something to rotate, because that is what the key was pressed
            // for in every case where both are possible.
            if pressed(egui::Key::E) {
                if self.project.selection.active().is_some() {
                    self.gizmo.mode = GizmoMode::Rotate;
                } else {
                    self.set_background(tequila_scene::document::BackgroundMode::Environment);
                }
            }
            if pressed(egui::Key::R) {
                self.gizmo.mode = GizmoMode::Scale;
            }
            if pressed(egui::Key::C) {
                self.set_background(tequila_scene::document::BackgroundMode::Color);
            }
            if pressed(egui::Key::B) {
                self.set_background(tequila_scene::document::BackgroundMode::Backplate);
            }
            if pressed(egui::Key::F) {
                self.frame(false);
            }
            if pressed(egui::Key::Slash) {
                self.isolate_selection();
            }
            if pressed(egui::Key::Space) {
                self.show_project = !self.show_project;
            }
            if pressed(egui::Key::Tab) {
                self.full_viewport = !self.full_viewport;
            }
            if pressed(egui::Key::M) {
                self.toggle_library(ctx.screen_rect().width());
            }
            // §6's binding for Studios.
            if pressed(egui::Key::U) {
                self.project_tab = ProjectTab::Studios;
                self.show_project = true;
            }
            // D for display mode. Toggling rather than cycling: there are two,
            // and one key that goes back where it came from is what makes it
            // usable as a glance — hold the scene still, tap D, see the real
            // thing, tap D again.
            if pressed(egui::Key::D) {
                let back = self.last_preview_mode;
                if self.view_mode == view_mode::ViewMode::Rendered {
                    self.set_view_mode(back);
                } else {
                    self.last_preview_mode = self.view_mode;
                    self.set_view_mode(view_mode::ViewMode::Rendered);
                }
            }
            // Z steps through the four views, as Blender's pie offers them.
            if pressed(egui::Key::Z) {
                let next = self.view_mode.next();
                self.set_view_mode(next);
            }
        }
    }

    fn save_to(&mut self, path: &std::path::Path) {
        if let Err(error) = self.project.save(path) {
            self.project.status = error.to_string();
            rfd::MessageDialog::new()
                .set_title("Could not save project")
                .set_description(&self.project.status)
                .set_level(rfd::MessageLevel::Error)
                .show();
            return;
        }
        self.remember_recent(path);
        self.plugin_event("saved", path);
    }

    /// Put `path` first in File → Open recent.
    fn remember_recent(&mut self, path: &std::path::Path) {
        let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.recent = recent::with(&self.recent, &path);
        if let Some(file) = &self.recent_file {
            recent::write(file, &self.recent);
        }
    }

    /// Take `path` out of File → Open recent: it is gone, or asked to go.
    fn forget_recent(&mut self, path: Option<&std::path::Path>) {
        self.recent = match path {
            Some(path) => recent::without(&self.recent, path),
            None => Vec::new(),
        };
        if let Some(file) = &self.recent_file {
            recent::write(file, &self.recent);
        }
    }

    /// File → New project: the starter scene, as a launch has it, after
    /// asking about unsaved work.
    fn new_project(&mut self) {
        if !self.confirm_unsaved() {
            return;
        }
        self.start_new_project();
    }

    /// The starter scene, whatever there was: for File → New project once
    /// it has asked, and a script's `new` once it has settled unsaved work.
    pub(crate) fn start_new_project(&mut self) {
        self.imports.clear();
        self.project.start_new(starter_document());
        self.camera = OrbitCamera::framing(
            Vec3::new_y(tequila_scene::demo::STARTER_RADIUS),
            tequila_scene::demo::STARTER_RADIUS * 1.25,
            self.aspect(),
        );
        self.sync_viewport_camera();
        self.project.mark_clean();
        self.submit();
        self.project.status = "New project".into();
    }

    /// Open the project at `path`, whatever is open now: for File → Open
    /// once it has asked about unsaved work, and a script's `open`.
    pub(crate) fn open_project(&mut self, path: &std::path::Path) -> Result<(), String> {
        self.project.open(path).map_err(|e| e.to_string())?;
        // Opening a project restores its composition. Framing all geometry
        // here overwrote the saved active camera, especially badly for large
        // studio floors.
        self.camera.adopt(self.project.camera());
        self.submit();
        self.remember_recent(path);
        self.plugin_event("opened", path);
        Ok(())
    }

    /// A recent project, opened - or, gone, taken off the list.
    fn open_recent(&mut self, path: std::path::PathBuf) {
        if !path.is_file() {
            self.toasts.warn(format!("{} is no longer there", path.display()));
            self.forget_recent(Some(&path));
            return;
        }
        self.handle_file(Action::Open, path);
    }

    /// A cancelled dialog or failed write must keep the current project open.
    /// The crash journal at the end: discarded when the close was agreed to
    /// or nothing was unsaved, kept otherwise. A Mac ends an application
    /// from the Dock's Quit, or on logging out, without its window ever
    /// being asked to close; the "Save your changes?" question never came,
    /// and discarding the journal then lost the work for good.
    fn end_session(&mut self) {
        if self.project.has_unsaved_changes() && !(self.close_approved || self.quit_requested) {
            self.project.keep_for_recovery();
        } else {
            self.project.close();
        }
    }

    /// Fit the window to its screen, once the screen's size is known: the
    /// first size suits a desktop monitor and runs off a 13-inch laptop's,
    /// title bar and Dock or taskbar included.
    fn fit_to_screen(&mut self, ctx: &egui::Context) {
        let (monitor, inner) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().inner_rect));
        let (Some(monitor), Some(inner)) = (monitor, inner) else {
            return;
        };
        self.fitted = true;
        if let Some(size) = fitted_window(inner.size(), monitor) {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        }
    }

    fn confirm_unsaved(&mut self) -> bool {
        if !self.project.has_unsaved_changes() {
            return true;
        }
        match rfd::MessageDialog::new()
            .set_title("Save your changes?")
            .set_description("Save changes to the current project?\n\nYes: save your work.\nNo: discard unsaved changes.\nCancel: keep working.")
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .set_level(rfd::MessageLevel::Warning)
            .show() {
                rfd::MessageDialogResult::Yes => {
                    match self.project.path.clone() {
                        Some(path) => self.save_to(&path),
                        None => self.ask_for_file(Action::SaveAs),
                    }
                    !self.project.has_unsaved_changes()
                }
                rfd::MessageDialogResult::No => true,
                _ => false,
            }
    }

    fn confirm_close(&mut self) -> bool {
        if self.batch.is_running()
            && rfd::MessageDialog::new()
                .set_title("A render is still running")
                .set_description("Closing Tequila will stop the current render. Close anyway?")
                .set_buttons(rfd::MessageButtons::YesNo)
                .set_level(rfd::MessageLevel::Warning)
                .show()
                != rfd::MessageDialogResult::Yes
        {
            return false;
        }
        self.confirm_unsaved()
    }

    /// Ask for a path with the platform's own file dialog, then act on it.
    ///
    /// Modal, and deliberately so: it blocks this frame while the dialog is
    /// open. A non-blocking dialog would need the result to arrive on a later
    /// frame through a channel, which is more machinery than an operation the
    /// user is already waiting on deserves. The renderer keeps running on its
    /// own thread throughout, so nothing stops converging.
    fn ask_for_file(&mut self, action: Action) {
        let dialog = rfd::FileDialog::new();
        let chosen = match action {
            Action::Import => dialog
                .set_title("Import model")
                .add_filter("Models", &["obj", "stl", "gltf", "glb", "step", "stp", "igs", "iges", "usda", "usd", "ifc"])
                .add_filter("All files", &["*"])
                .pick_file(),
            Action::Open => dialog
                .set_title("Open project")
                .add_filter("Tequila project", &["tequila", "freeshot"])
                .pick_file(),
            Action::SaveAs => dialog
                .set_title("Save project as")
                .add_filter("Tequila project", &["tequila"])
                .set_file_name("Untitled.tequila")
                .save_file(),
        };
        let Some(path) = chosen else {
            return;
        };

        self.handle_file(action, path);
    }

    /// Ask for a texture pack and load it onto the selected parts.
    fn pick_texture_pack(&mut self, folder: bool) {
        if let Some(source) = texture_pack::pick(folder) {
            self.load_texture_pack(&source);
        }
    }

    /// A texture pack, as materials: the first on the selected parts, or
    /// only added to the project when nothing is selected.
    fn load_texture_pack(&mut self, source: &std::path::Path) {
        let targets = tequila_scene::apply::parts_of(self.project.document(), self.project.selection.nodes());
        match texture_pack::load(&mut self.project, source, targets) {
            Ok(loaded) => {
                let status = texture_pack::status(&loaded);
                self.toasts.info(status.clone());
                self.project.status = status;
                // The Material tab shows what was just made.
                if !loaded.assigned.is_empty() {
                    self.show_project = true;
                    self.project_tab = ProjectTab::Material;
                }
            }
            Err(error) => {
                self.toasts.warn(error.clone());
                self.project.status = error;
            }
        }
    }

    /// Files dropped on the window: a model is imported, a project opened
    /// and a texture pack loaded onto the selection. While they hover, a
    /// sign says which.
    fn dropped_files(&mut self, ctx: &egui::Context) {
        let hovering: Vec<std::path::PathBuf> =
            ctx.input(|i| i.raw.hovered_files.iter().filter_map(|f| f.path.clone()).collect());
        if let Some(path) = hovering.first() {
            let parts = self.project.selection.nodes().count();
            let text = match texture_pack::classify_drop(path) {
                texture_pack::Dropped::Model => "Drop to import the model".to_string(),
                texture_pack::Dropped::Project => "Drop to open the project".to_string(),
                texture_pack::Dropped::TexturePack if parts == 0 => {
                    "Drop to add the texture pack to the project\n(select parts first to put it on them)".to_string()
                }
                texture_pack::Dropped::TexturePack => format!(
                    "Drop to put the texture pack on the {parts} selected part{}",
                    if parts == 1 { "" } else { "s" }
                ),
                texture_pack::Dropped::Material => "Drop to add the material to your library".to_string(),
                texture_pack::Dropped::Unknown => "Tequila does not open this kind of file".to_string(),
            };
            let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("file-drop")));
            let screen = ctx.screen_rect();
            painter.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(150));
            painter.text(
                screen.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(20.0),
                egui::Color32::WHITE,
            );
        }

        let dropped: Vec<std::path::PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        for path in dropped {
            match texture_pack::classify_drop(&path) {
                texture_pack::Dropped::Model => self.handle_file(Action::Import, path),
                texture_pack::Dropped::Project => self.handle_file(Action::Open, path),
                texture_pack::Dropped::TexturePack => self.load_texture_pack(&path),
                texture_pack::Dropped::Material => self.import_material_file(&path),
                texture_pack::Dropped::Unknown => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.toasts.warn(format!("Tequila does not open {name}"));
                }
            }
        }
    }

    /// Act on a chosen file, however it was chosen.
    ///
    /// Split out from the dialog so that a path on the command line and a path
    /// picked in a dialog take exactly the same route — including the framing
    /// and the status line, which is the part that would otherwise quietly
    /// differ between the two.
    fn handle_file(&mut self, action: Action, path: std::path::PathBuf) {
        match action {
            Action::Import => {
                // Read on another thread; `poll_imports` applies it.
                let (sender, receiver) = std::sync::mpsc::channel();
                let reading = path.clone();
                std::thread::spawn(move || {
                    let prepared = tequila_import::read_file(&reading).and_then(project::PreparedImport::prepare);
                    let _ = sender.send(prepared.map(Arrived::Prepared));
                });
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.project.status = format!("Importing {name}\u{2026}");
                self.imports.push(PendingImport {
                    name,
                    path,
                    receiver,
                    document_epoch: self.project.document_epoch(),
                    update: None,
                    started: std::time::Instant::now(),
                });
            }
            Action::Open => {
                if self.confirm_unsaved() {
                    if let Err(error) = self.open_project(&path) {
                        self.project.status = error;
                    }
                }
            }
            // A save dialog does not always append the extension, and a
            // project saved as `Untitled` will not reopen through the filter
            // that only shows `.tequila`.
            Action::SaveAs => {
                let path = if path.extension().is_none() {
                    path.with_extension("tequila")
                } else {
                    path
                };
                self.save_to(&path);
            }
        }
    }

    /// Apply any import that has finished reading. True while some are
    /// still being read, so the frames keep coming until they land.
    fn poll_imports(&mut self) -> bool {
        // A CAD file's surfaces divide for a while: say how far it has got,
        // so a big assembly reads as working rather than stuck.
        // The running time too: OpenCASCADE's percentage can sit still for
        // half a minute on one big part.
        if let (Some(first), Some(progress)) = (self.imports.first(), tequila_import::step::progress()) {
            let seconds = first.started.elapsed().as_secs();
            self.project.status = if progress.total == 0 && progress.read == 0 {
                format!("Reading {}\u{2026} {seconds} s", first.name)
            } else if progress.total == 0 {
                format!("Reading {}: {}% \u{00b7} {seconds} s", first.name, progress.read)
            } else {
                format!(
                    "Importing {}: dividing surfaces, {} of {} faces ({}%) \u{00b7} {seconds} s",
                    first.name,
                    progress.done,
                    progress.total,
                    progress.done * 100 / progress.total.max(1)
                )
            };
        }
        let mut index = 0;
        while index < self.imports.len() {
            if self.imports[index].document_epoch != self.project.document_epoch() {
                let pending = self.imports.remove(index);
                self.project.status = format!("Discarded pending import {} because the project changed", pending.name);
                continue;
            }
            let result = match self.imports[index].receiver.try_recv() {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    index += 1;
                    continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err("the import stopped before it finished".to_string())
                }
            };
            let pending = self.imports.remove(index);
            if let Some(roots) = &pending.update {
                let updated = result.and_then(|arrived| match arrived {
                    Arrived::Read(model) => self.project.update_read(model, roots),
                    Arrived::Prepared(_) => Err("an update arrived as a new import".to_string()),
                });
                match updated {
                    Ok(report) => {
                        self.project.status = format!(
                            "Updated {}: {} parts kept their work, {} added, {} no longer in the file",
                            pending.name,
                            report.matched,
                            report.added.len(),
                            report.removed.len()
                        );
                        self.toasts.info(self.project.status.clone());
                        if !report.added.is_empty() || !report.removed.is_empty() {
                            self.update_report = Some(report);
                        }
                    }
                    Err(error) => {
                        self.project.status = format!("{}: {error}", pending.name);
                        self.toasts.warn(self.project.status.clone());
                    }
                }
                continue;
            }
            // The starter sphere is there to show that rendering works: the
            // person's own model takes its place rather than sitting beside
            // it - in the same undo step, and it comes back if the import
            // fails. The lighting and look they set up stay.
            let starter = self.untouched_starter();
            if let Some(demo) = &starter {
                self.project.begin_group();
                let _ = self.project.apply(Command::DeleteNodes { nodes: demo.clone() });
            }
            // What the import adds, for the plugins listening for it.
            let before = self.node_ids();
            let imported = result.and_then(|arrived| match arrived {
                Arrived::Prepared(ready) => self.project.import_prepared(ready),
                Arrived::Read(model) => self.project.import_read(model),
            });
            if starter.is_some() {
                if imported.is_ok() {
                    self.project.end_group();
                } else {
                    self.project.abandon_group();
                }
            }
            match imported {
                Ok((report, units)) => {
                    self.plugin_imported(&pending.path, &before);
                    self.project.status = format!(
                        "Imported {} nodes, {} triangles, {} instanced",
                        report.nodes, report.triangles, report.instances_detected
                    );
                    for warning in report.warnings.iter().take(1) {
                        self.project.status = format!("{}. {warning}", self.project.status);
                    }
                    // CAD files do not say which way up they were drawn, and
                    // one drawn Z up arrives lying on its back.
                    let cad = std::path::Path::new(&pending.name)
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| ["step", "stp", "iges", "igs"].iter().any(|c| e.eq_ignore_ascii_case(c)));
                    if cad && report.warnings.is_empty() {
                        self.project.status.push_str(". Lying on its back? Right-click it in the Scene tab: Turn, Upright");
                    }
                    if units.is_some() {
                        self.model_units = units;
                    }
                    self.frame_selection();
                }
                Err(error) => self.project.status = format!("{}: {error}", pending.name),
            }
        }
        !self.imports.is_empty()
    }

    /// Help > About Tequila: which build this is, and its licences.
    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        egui::Window::new("About Tequila")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let p = theme::p();
                ui.horizontal(|ui| {
                    let (mark, _) = ui.allocate_exact_size(egui::Vec2::splat(28.0), egui::Sense::hover());
                    icons::paint(ui.painter(), mark, icons::Icon::Aperture, p.accent);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Tequila").font(theme::strong(18.0)).color(p.text));
                        ui.label(egui::RichText::new("Product rendering").size(12.0).color(p.text_dim));
                    });
                });
                ui.add_space(8.0);
                match build_note() {
                    Some((built, source)) => {
                        let date = built.get(..16).unwrap_or(&built).replace('T', " ");
                        ui.label(format!("Built {date}"));
                        if !source.is_empty() {
                            ui.label(egui::RichText::new(format!("Source {}", source.get(..10).unwrap_or(&source))).small().color(p.text_dim));
                        }
                    }
                    None => {
                        ui.label(format!("Development build {}", env!("CARGO_PKG_VERSION")));
                    }
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(licences) = shipped_file("THIRD_PARTY_LICENSES.md", "THIRD_PARTY_LICENSES.md") {
                        if ui.button("Licences").on_hover_text("Tequila's licence and the open-source software it is built with").clicked() {
                            crate::library_tab::open_folder(&licences);
                        }
                    }
                    if let Some(folder) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) {
                        if ui.button("Open the program folder").clicked() {
                            crate::library_tab::open_folder(&folder);
                        }
                    }
                });
            });
        self.show_about = open;
    }

    /// The starter scene's demo group, while the scene is that and nothing
    /// else in a project never saved: what an import replaces.
    pub(crate) fn untouched_starter(&self) -> Option<Vec<tequila_scene::tree::NodeId>> {
        let document = self.project.document();
        if self.project.path.is_some() || document.tree.len() != 2 {
            return None;
        }
        let root = document.tree.iter().find(|n| n.parent.is_none())?;
        let sphere = document.tree.iter().find(|n| n.parent == Some(root.id))?;
        let starter = root.name == "Demo" && sphere.name == "Sphere" && matches!(root.kind, tequila_scene::tree::NodeKind::Group);
        starter.then(|| vec![root.id])
    }

    /// Update the model `node` belongs to from its file, or from one the
    /// user picks: read on another thread, applied by `poll_imports`.
    fn update_model(&mut self, node: tequila_scene::tree::NodeId, pick: bool) {
        let roots = tequila_scene::reimport::import_roots(self.project.document(), node);
        let recorded = tequila_scene::reimport::source_file(self.project.document(), &roots)
            .map(std::path::PathBuf::from);
        let path = match (pick, recorded) {
            (false, Some(path)) if path.is_file() => path,
            (false, Some(path)) => {
                self.toasts.warn(format!(
                    "{} is no longer there. Use Update from another file to choose where it went.",
                    path.display()
                ));
                return;
            }
            (_, recorded) => {
                let mut dialog = rfd::FileDialog::new()
                    .set_title("Update model from")
                    .add_filter("Models", &["obj", "stl", "gltf", "glb", "step", "stp", "igs", "iges", "usda", "usd", "ifc"]);
                if let Some(folder) = recorded.as_ref().and_then(|p| p.parent()) {
                    dialog = dialog.set_directory(folder);
                }
                let Some(path) = dialog.pick_file() else { return };
                path
            }
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let reading = path.clone();
        std::thread::spawn(move || {
            let _ = sender.send(tequila_import::read_file(&reading).map(Arrived::Read));
        });
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.project.status = format!("Updating from {name}\u{2026}");
        self.imports.push(PendingImport {
            name,
            path,
            receiver,
            document_epoch: self.project.document_epoch(),
            update: Some(roots),
            started: std::time::Instant::now(),
        });
    }

    /// Divide a CAD model's exact surfaces again at `quality`, from the
    /// source kept with the project or else its file, as an update: every
    /// part keeps its material, labels and settings.
    pub(crate) fn retessellate(&mut self, node: tequila_scene::tree::NodeId, quality: tequila_import::step::Tessellation) {
        let document = self.project.document();
        let roots = tequila_scene::reimport::import_roots(document, node);
        let top = roots.first().and_then(|r| document.tree.get(*r));
        let path = top.and_then(|n| n.metadata.get(tequila_scene::reimport::SOURCE_FILE)).map(std::path::PathBuf::from);
        let kept = top.and_then(|n| n.metadata.get(tequila_scene::document::BREP)).and_then(|id| document.sources.get(id)).cloned();
        let name = path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| top.map(|n| n.name.clone()))
            .unwrap_or_else(|| "Model".into());
        let bytes = match (kept, &path) {
            (Some(bytes), _) => bytes,
            (None, Some(path)) if path.is_file() => match std::fs::read(path) {
                Ok(bytes) => std::sync::Arc::new(bytes),
                Err(error) => {
                    self.toasts.warn(format!("{} could not be read: {error}", path.display()));
                    return;
                }
            },
            _ => {
                self.toasts.warn("The CAD data was not kept with the project and its file is gone, so the model cannot be divided again.".to_string());
                return;
            }
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let reading = name.clone();
        let source = path.clone().unwrap_or_default();
        std::thread::spawn(move || {
            let _ = sender.send(tequila_import::read_cad(&bytes, &reading, path, quality).map(Arrived::Read));
        });
        self.project.status = format!("Dividing {name} at {} quality\u{2026}", quality.label());
        self.imports.push(PendingImport {
            name: format!("{name} ({})", quality.label()),
            path: source,
            receiver,
            document_epoch: self.project.document_epoch(),
            update: Some(roots),
            started: std::time::Instant::now(),
        });
    }

    /// What an update added and what it hid, with the choice of deleting
    /// the hidden parts: the reconciliation `docs/03` §2.3 asks for rather
    /// than discarding work silently.
    fn update_report_window(&mut self, ctx: &egui::Context) {
        let Some(report) = self.update_report.clone() else { return };
        let mut open = true;
        let mut delete = false;
        egui::Window::new("Model updated")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label(format!("{} parts kept their materials, labels and settings.", report.matched));
                if !report.added.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(format!("New in the file ({})", report.added.len())).strong());
                    egui::ScrollArea::vertical().id_salt("added").max_height(120.0).show(ui, |ui| {
                        for name in &report.added {
                            ui.label(name);
                        }
                    });
                }
                if !report.removed.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(format!("No longer in the file ({}), hidden", report.removed.len())).strong());
                    ui.label(
                        egui::RichText::new("A part renamed in CAD shows up here and above. Give the new one the old one's material, then delete the old.")
                            .small()
                            .color(theme::dim()),
                    );
                    egui::ScrollArea::vertical().id_salt("removed").max_height(120.0).show(ui, |ui| {
                        for (_, name) in &report.removed {
                            ui.label(name);
                        }
                    });
                    if ui.button(format!("Delete the {} hidden parts", report.removed.len())).clicked() {
                        delete = true;
                    }
                }
            });
        if delete {
            let nodes: Vec<tequila_scene::tree::NodeId> = report
                .removed
                .iter()
                .map(|(id, _)| *id)
                .filter(|&id| self.project.document().tree.contains(id))
                .collect();
            let _ = self.project.apply(Command::DeleteNodes { nodes });
            self.project.end_interaction();
            open = false;
        }
        if !open {
            self.update_report = None;
        }
    }

    /// The Improve model window: smoother curves, subdivision, shading.
    fn improve_model_window(&mut self, ctx: &egui::Context) {
        let Some(mut window) = self.improve_model.take() else { return };
        let (action, open) = window.show(ctx, self.project.document());
        match action {
            Some(improve_model::Improve::Retessellate(node, quality)) => self.retessellate(node, quality),
            Some(improve_model::Improve::Apply(command, said)) => match self.project.apply(command) {
                Ok(_) => {
                    self.project.end_interaction();
                    self.toasts.info(said);
                }
                Err(error) => self.toasts.warn(format!("The model could not be changed: {error}")),
            },
            None => {}
        }
        if open {
            self.improve_model = Some(window);
        }
    }

    /// The Model units window: what a model's numbers are in.
    fn model_units_window(&mut self, ctx: &egui::Context) {
        let Some(mut units) = self.model_units.take() else { return };
        let (rescale, open) = units.show(ctx, self.project.document());
        if let Some(rescale) = rescale {
            let _ = self.project.apply(rescale);
            self.project.end_interaction();
            // Ten or a thousand times the size it was: frame it again, or
            // the camera is inside it or looking at a speck.
            self.frame_selection();
        }
        if open {
            self.model_units = Some(units);
        }
    }

    /// Frame the camera on everything visible, so an import is not off screen.
    fn frame_selection(&mut self) {
        let bounds = tequila_scene::snapshot::visible_bounds(self.project.document());
        if bounds.is_empty() {
            return;
        }
        let radius = (bounds.diagonal().length() * 0.5).max(1e-3);
        self.camera = OrbitCamera::framing(bounds.center(), radius, self.aspect());
        self.sync_viewport_camera();
        self.submit();
    }

    fn toggle_library(&mut self, window_width: f32) {
        let visible = self.show_library && (window_width >= 1100.0 || !self.show_project);
        self.show_library = !visible;
        if self.show_library && window_width < 1100.0 {
            self.show_project = false;
        }
    }

    /// The inspector's tabs as a rail of icons down the window's right edge.
    ///
    /// Always there, so the inspector is one click from closed as well as
    /// from open: a tab shows its page, and the tab already showing puts the
    /// panel away and gives its width to the viewport.
    fn inspector_rail(&mut self, ui: &mut egui::Ui) {
        let rail = ui.max_rect();
        let p = theme::p();
        let mut y = rail.top() + 8.0;
        for tab in ProjectTab::ALL {
            // The scene and its surfaces; the light, lens and film; the
            // shot's versions and the script.
            if matches!(tab, ProjectTab::Environment | ProjectTab::Studios) {
                y += 3.0;
                ui.painter().line_segment(
                    [egui::pos2(rail.left() + 18.0, y), egui::pos2(rail.right() - 18.0, y)],
                    egui::Stroke::new(1.0_f32, p.line),
                );
                y += 6.0;
            }
            let item = egui::Rect::from_min_size(egui::pos2(rail.left() + 6.0, y), egui::vec2(rail.width() - 12.0, 50.0));
            y += 53.0;
            let on = self.show_project && self.project_tab == tab;
            let response = icons::tip(
                ui.interact(item, ui.id().with(("rail", tab.label())), egui::Sense::click()),
                tab.label(),
                tab.hint(),
                "",
            );
            let ease = ui.ctx().animate_bool_with_time(response.id.with("ease"), response.hovered(), 0.1);
            let hue = theme::hue(tab.hue());
            let painter = ui.painter();
            if on {
                painter.rect_filled(item, 10.0, hue.gamma_multiply(if p.dark { 0.18 } else { 0.13 }));
                painter.rect_filled(
                    egui::Rect::from_min_max(egui::pos2(rail.left(), item.top() + 12.0), egui::pos2(rail.left() + 3.0, item.bottom() - 12.0)),
                    1.5,
                    hue,
                );
            } else if ease > 0.0 {
                painter.rect_filled(item, 10.0, p.hover.gamma_multiply(0.6 * ease));
            }
            let glyph = egui::Rect::from_center_size(egui::pos2(item.center().x, item.top() + 18.0), egui::Vec2::splat(19.0));
            // Each page in its own colour, so it is found by colour as much
            // as by shape; dimmer when another is showing.
            icons::paint(painter, glyph, tab.icon(), if on || response.hovered() { hue } else { hue.gamma_multiply(if p.dark { 0.8 } else { 0.85 }) });
            painter.text(
                egui::pos2(item.center().x, item.bottom() - 11.0),
                egui::Align2::CENTER_CENTER,
                tab.label(),
                if on { theme::strong(10.5) } else { egui::FontId::proportional(10.5) },
                if on { p.text } else if response.hovered() { p.text } else { p.text_dim },
            );
            if response.clicked() {
                if on {
                    self.show_project = false;
                } else {
                    self.show_project = true;
                    self.project_tab = tab;
                }
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            if icons::tip(icons::button(ui, icons::Icon::Help, self.show_help, 36.0, egui::Sense::click()), "Help", "Navigation and shortcuts", "")
                .clicked()
            {
                self.show_help = !self.show_help;
            }
        });
    }

    fn project_panel(&mut self, ui: &mut egui::Ui) {
        let tab = self.project_tab;
        panel_title(ui, tab.icon(), theme::hue(tab.hue()), tab.label(), tab.hint(), || self.show_project = false, "Hide the inspector · Space");

        let mut resubmit = false;
        egui::ScrollArea::vertical()
            .id_salt(("inspector-content", self.project_tab.label()))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                resubmit = self.project_tab_body(ui);
                ui.add_space(12.0);
            });

        if resubmit {
            self.submit();
        }
    }

    /// One tab's contents. Returns whether the render has to restart.
    fn scripting_panel(&mut self, ui: &mut egui::Ui) {
        let status = scripting_tab::ChannelStatus {
            open: self.channel.is_some(),
            port: self.channel.as_ref().map(|c| c.port()).unwrap_or(0),
            discovery: tequila_scripting::server::discovery_path(),
        };
        match scripting_tab::show(ui, &status, &self.activity) {
            scripting_tab::Action::OpenConsole => self.console.show_and_focus(),
            scripting_tab::Action::ClearLog => self.activity.clear(),
            scripting_tab::Action::None => {}
        }
    }

    fn console_panel(&mut self, ui: &mut egui::Ui) {
        let status = console::Status {
            listening: self.channel.as_ref().map(|c| c.port()),
            requests: self.activity.total(),
            active: self.activity.active(),
        };
        // Run after the panel has let go of the console, so the handler has
        // the whole application to work with: the console can do anything
        // the socket can, including a render.
        match self.console.show(ui, &status) {
            console::Action::Run(line) => {
                console::run(self, &line);
            }
            console::Action::RunFile => {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("Tequila script", &["txt", "fsh", "json"])
                    .pick_file()
                {
                    console::run(self, &format!("run \"{}\"", path.display()));
                }
            }
            console::Action::None => {}
        }
    }

    fn project_tab_body(&mut self, ui: &mut egui::Ui) -> bool {
        let mut resubmit = false;
        match self.project_tab {
            ProjectTab::Scripting => self.scripting_panel(ui),
            ProjectTab::Studios => {
                let mut tab = std::mem::take(&mut self.studio_tab);
                if tab.show(ui, &mut self.project) {
                    self.camera.adopt(self.project.camera());
                    resubmit = true;
                }
                let action = tab.action.take();
                self.studio_tab = tab;
                match action {
                    Some(configurator_panel::Action::Present) => self.start_presentation(),
                    Some(configurator_panel::Action::RenderAll) => self.plan_configuration_batch(),
                    Some(configurator_panel::Action::ExportWeb) => {
                        self.web_plan = Some(WebPlan { live: true, photos: false });
                    }
                    None => {}
                }
            }
            ProjectTab::Scene => {
                let mut tab = std::mem::take(&mut self.scene_tab);
                tab.show(ui, &mut self.project);
                ui.add_space(4.0);
                ui.separator();
                theme::section(ui, "Metadata");
                tab.metadata_panel(ui, &mut self.project);
                let update = tab.update_request.take();
                let retessellate = tab.retessellate_request.take();
                let units = tab.units_request.take();
                let improve = tab.improve_request.take();
                self.scene_tab = tab;
                if let Some(node) = improve {
                    self.improve_model = improve_model::ImproveModel::new(self.project.document(), node);
                    if self.improve_model.is_none() {
                        self.toasts.info("That model has no parts to improve.");
                    }
                }
                if let Some(node) = units {
                    let document = self.project.document();
                    let roots = tequila_scene::reimport::import_roots(document, node);
                    let name = roots
                        .first()
                        .and_then(|&root| document.tree.get(root))
                        .map(|root| root.name.clone())
                        .unwrap_or_else(|| "The model".into());
                    self.model_units = model_units::ModelUnits::new(document, roots, name);
                    if self.model_units.is_none() {
                        self.toasts.info("That model has nothing to measure yet.");
                    }
                }
                if let Some((node, pick)) = update {
                    self.update_model(node, pick);
                }
                if let Some((node, quality)) = retessellate {
                    self.retessellate(node, quality);
                }
                ui.add_space(4.0);
                ui.separator();
                theme::section(ui, "Transform");
                self.transform_panel(ui);
                ui.add_space(6.0);
                ui.separator();
                theme::section(ui, "Scene units");
                self.units_panel(ui);
                ui.add_space(6.0);
                ui.separator();
                theme::section(ui, "Measurements");
                measure_tool::panel(ui, &mut self.project, &mut self.measure);
            }
            ProjectTab::Material => {
                // A light and a part are both nodes, and the panel that follows
                // the selection has to follow it into either. Showing both at
                // once would mean showing a material slot for a light.
                let is_light = lighting_tab::light_panel(ui, &mut self.project)
                    || cutaway_panel::cutaway_panel(ui, &mut self.project);
                if !is_light {
                    let mut tab = std::mem::take(&mut self.material_tab);
                    tab.library_categories = tequila_format::library::categories(self.library.entries());
                    // So the Place button can show as held down while the
                    // viewport is in that mode. Read before the panel draws,
                    // written back after it asks for a change.
                    tab.placing_label = self
                        .label_gizmo
                        .armed()
                        .map(|target| (target.material, target.index));
                    tab.show(ui, &mut self.project);
                    if let Some(material) = tab.open_graph.take() {
                        self.graph_editor.open_for(material);
                    }
                    if let Some((material, index)) = tab.place_label_request.take() {
                        self.label_gizmo
                            .toggle(label_gizmo::Target { material, index });
                    }
                    if let Some(message) = tab.take_saved() {
                        // The browser re-reads so the new entry is on the
                        // shelf immediately. A library that needed a restart
                        // to show what was just saved would teach people that
                        // saving does not work.
                        self.library.reload();
                        self.toasts.info(message);
                    }
                    self.material_tab = tab;
                }
                // Whatever is selected: templates work across the assembly.
                ui.add_space(6.0);
                ui.separator();
                self.templates_panel.show(ui, &mut self.project);
            }
            ProjectTab::Environment => {
                let mut tab = std::mem::take(&mut self.environment_tab);
                tab.aiming_pin = self.pin_gizmo.armed();
                tab.show(ui, &mut self.project);
                if let Some(index) = tab.aim_pin_request.take() {
                    self.pin_gizmo.toggle(index);
                }
                self.environment_tab = tab;
            }
            ProjectTab::Lighting => {
                let mut tab = std::mem::take(&mut self.lighting_tab);
                tab.show(ui, &mut self.project);
                self.lighting_tab = tab;
            }
            ProjectTab::Camera => {
                let aspect = self.aspect();
                let mut tab = std::mem::take(&mut self.camera_tab);
                if tab.show(ui, &mut self.project, &mut self.camera, aspect) {
                    self.camera.adopt(self.project.camera());
                    resubmit = true;
                }
                self.camera_tab = tab;
            }
            ProjectTab::Image => {
                // The Image tab re-develops the accumulated buffer. `docs/01`
                // §9.3 classifies every control on it as needing no re-render
                // at all, so none of them touches `resubmit`.
                let mut tab = std::mem::take(&mut self.image_tab);
                tab.show(ui, &mut self.project);
                self.image_tab = tab;

                ui.separator();
                theme::section(ui, "Denoise");
                {
                    // The same settings the final render uses, edited here because
                    // this is where a person notices noise. The Render window shows
                    // the identical controls; both write to the document.
                    let before = self.project.render_settings().clone();
                    let mut settings = before.clone();
                    ui.add(crate::field::Checkbox::new(&mut settings.denoise, "Enabled"));
                    // Cycles' denoiser has no strength: it cleans fully or not
                    // at all.
                    let cycles = settings.engine.is_cycles();
                    ui.add_enabled(
                        settings.denoise && !cycles,
                        crate::field::Slider::new(&mut settings.denoise_strength, 0.0..=1.0)
                            .text("Strength"),
                    )
                    .on_disabled_hover_text(if cycles {
                        "Cycles denoises at full strength; Strength is for Tequila's renderer"
                    } else {
                        "Denoising is off"
                    });
                    ui.add_enabled(
                        settings.denoise,
                        crate::field::Slider::new(&mut settings.denoise_cadence, 1..=32)
                            .text("Every N passes"),
                    )
                    .on_hover_text(
                        "1 leaves the cadence to the scheduler, which denoises densely at \
                     first and thins out as the image settles. A larger number is a \
                     fixed interval, for an image big enough that denoising costs more \
                     than the samples it smooths.",
                    );
                    if settings != before {
                        let _ = self.project.apply(Command::SetRenderSettings {
                            settings: Box::new(settings),
                        });
                        resubmit = true;
                    }
                }
            }
        }
        resubmit
    }

    /// Move the render queue along: collect what the batch thread has said, and
    /// start the next job when it is idle.
    ///
    /// Called every frame and does nothing almost every time. Polling rather
    /// than a callback because the queue's state lives in the document's
    /// project and egui owns the frame — a callback would need a lock around
    /// everything the UI touches to deliver a message the next frame reads
    /// anyway.
    fn pump_queue(&mut self, ctx: &egui::Context) {
        use tequila_scene::render::JobState;

        let mut changed = false;
        for event in self.batch.drain() {
            match event {
                batch::BatchEvent::Stage { id, label } => {
                    if let Some(job) = self.project.queue_mut().get_mut(id) { job.message = label.into(); }
                }
                batch::BatchEvent::Preview { id, width, height, rgba } => {
                    self.render_window.set_preview(ctx, id, width, height, &rgba);
                }
                batch::BatchEvent::Started { id } => {
                    if let Some(job) = self.project.queue_mut().get_mut(id) {
                        job.state = JobState::Rendering;
                    }
                    changed = true;
                }
                batch::BatchEvent::Progress {
                    id,
                    samples,
                    seconds,
                } => {
                    if let Some(job) = self.project.queue_mut().get_mut(id) {
                        job.samples = samples;
                        job.seconds = seconds;
                    }
                    // Deliberately not saved: progress changes sixty times a
                    // render and the queue file is about surviving a crash, not
                    // about being a log.
                }
                batch::BatchEvent::Finished {
                    id,
                    path,
                    width,
                    height,
                    samples,
                    seconds,
                    preview,
                    halt,
                    stopped,
                    relight,
                } => {
                    if let Some(relight) = relight {
                        self.render_window.set_relight(relight);
                    }
                    self.plugin_event("render_finished", std::path::Path::new(&path));
                    let name = self
                        .project
                        .queue()
                        .get(id)
                        .map(|j| j.name.clone())
                        .unwrap_or_else(|| "Render".into());
                    if let Some(job) = self.project.queue_mut().get_mut(id) {
                        job.state = JobState::Done;
                        job.output = Some(path.clone());
                        job.samples = samples;
                        job.seconds = seconds;
                        job.message = if stopped { "Stopped · partial image saved".into() } else { match halt {
                            tequila_scene::render::Halt::Samples => String::new(),
                            tequila_scene::render::Halt::Time => "stopped on time".into(),
                            tequila_scene::render::Halt::Converged => "converged".into(),
                        }};
                    }
                    let texture = (width > 0 && height > 0).then(|| {
                        ctx.load_texture(
                            format!("gallery-{id}"),
                            render_window::preview_image(width, height, &preview),
                            egui::TextureOptions::LINEAR,
                        )
                    });
                    self.render_window.preview_job = Some(id);
                    self.render_window.preview = texture.clone();
                    self.render_window.add(render_window::GalleryEntry {
                        name,
                        path,
                        width,
                        height,
                        samples,
                        seconds,
                        when: clock::stamp(),
                        texture,
                    });
                    changed = true;
                }
                batch::BatchEvent::Failed { id, message } => {
                    if let Some(job) = self.project.queue_mut().get_mut(id) {
                        job.state = JobState::Failed;
                        job.message = message.clone();
                    }
                    self.project.status = format!("A render failed: {message}");
                    changed = true;
                }
            }
        }
        if changed {
            self.project.save_queue();
        }

        if self.batch.is_running() {
            // A queue that is running has to keep the frames coming, or the
            // progress it reports is only seen when the user moves the mouse.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            return;
        }
        let Some(id) = self.project.queue().next_queued() else {
            return;
        };
        if let Some(job) = self.build_job(id) {
            if let Some(queued) = self.project.queue_mut().get_mut(id) { queued.state = JobState::Rendering; queued.message = "Preparing scene".into(); }
            self.batch.submit(job);
        }
    }

    /// Into Presentation mode: full screen, nothing but the view and the
    /// configurator's picker over it, rendered rather than drawn.
    pub(crate) fn start_presentation(&mut self) {
        if self.presenting.is_some() {
            return;
        }
        self.presenting = Some(Presentation {
            full_viewport: self.full_viewport,
            view_mode: self.view_mode,
            show_hud: self.show_hud,
            show_axis_legend: self.show_axis_legend,
        });
        self.full_viewport = true;
        self.show_hud = false;
        self.show_axis_legend = false;
        // No outline and no gizmo on the product.
        self.project.selection.clear();
        self.set_view_mode(view_mode::ViewMode::Rendered);
        self.fullscreen_request = Some(true);
    }

    /// The Studios tab, scrolled to the configurator.
    pub(crate) fn reveal_configurator(&mut self) {
        self.show_project = true;
        self.full_viewport = false;
        self.project_tab = ProjectTab::Studios;
        self.studio_tab.reveal = true;
    }

    /// Back to work, everything as it was.
    pub(crate) fn end_presentation(&mut self) {
        let Some(before) = self.presenting.take() else {
            return;
        };
        self.full_viewport = before.full_viewport;
        self.show_hud = before.show_hud;
        self.show_axis_legend = before.show_axis_legend;
        self.set_view_mode(before.view_mode);
        self.fullscreen_request = Some(false);
    }

    /// The picker floating over the presented view, where the layout puts
    /// it, and the way out.
    fn presentation_overlay(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.end_presentation();
            return;
        }
        use tequila_scene::configurator::Placement;
        let p = theme::p();
        let configurator = self.project.document().configurator.clone();
        let screen = ctx.screen_rect();
        let (anchor, offset, width) = match configurator.placement {
            Placement::Left => (egui::Align2::LEFT_CENTER, egui::vec2(28.0, 0.0), 340.0),
            Placement::Right => (egui::Align2::RIGHT_CENTER, egui::vec2(-28.0, 0.0), 340.0),
            Placement::Bottom => (egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -28.0), (screen.width() - 56.0).min(1180.0)),
        };
        let mut picked = None;
        egui::Area::new(egui::Id::new("presentation-picker"))
            .anchor(anchor, offset)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(p.raised.gamma_multiply(0.93))
                    .stroke(egui::Stroke::new(1.0_f32, p.line))
                    .rounding(18.0)
                    .inner_margin(egui::Margin::same(22.0))
                    .shadow(egui::epaint::Shadow { offset: egui::vec2(0.0, 8.0), blur: 28.0, spread: 0.0, color: p.shadow })
                    .show(ui, |ui| {
                        ui.set_width(width);
                        if !configurator.title.is_empty() {
                            ui.label(egui::RichText::new(&configurator.title).size(24.0).strong().color(p.text));
                        }
                        if !configurator.subtitle.is_empty() {
                            ui.label(egui::RichText::new(&configurator.subtitle).size(14.0).color(p.text_dim));
                        }
                        if !configurator.title.is_empty() || !configurator.subtitle.is_empty() {
                            ui.add_space(14.0);
                        }
                        if configurator.is_empty() {
                            ui.label("There is no configurator yet. Build one in the Studios tab, then present it.");
                        } else {
                            let columns = configurator.placement == Placement::Bottom;
                            picked = configurator_panel::picker_laid_out(ui, self.project.document(), &configurator, true, columns);
                        }
                    });
            });
        egui::Area::new(egui::Id::new("presentation-exit"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-18.0, 18.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if ui
                    .add(egui::Button::new(egui::RichText::new("Exit presentation   Esc").size(13.0)).rounding(10.0).min_size(egui::vec2(0.0, 32.0)))
                    .clicked()
                {
                    self.end_presentation();
                }
            });
        if let Some(choices) = picked {
            let moves_camera = configurator.options.iter().any(|o| o.drives == tequila_scene::configurator::Drives::Studio);
            match self.project.apply(tequila_scene::Command::Configure { choices }) {
                Ok(()) if moves_camera => {
                    self.camera.adopt(self.project.camera());
                    self.submit();
                }
                Ok(()) => {}
                Err(error) => self.project.status = error.to_string(),
            }
        }
    }

    /// Ask before queuing every configuration: how many, at what size, and
    /// about how long it will take.
    fn plan_configuration_batch(&mut self) {
        let configurator = &self.project.document().configurator;
        let combinations = configurator.combinations().len();
        if combinations == 0 {
            self.project.status = "The configurator has no valid combination to render".into();
            return;
        }
        let settings = self.project.render_settings();
        let finished: Vec<&tequila_scene::render::RenderJob> =
            self.project.queue().jobs().iter().filter(|j| j.state == tequila_scene::render::JobState::Done).collect();
        let each = tequila_scene::render::estimate_seconds(settings, &finished);
        self.batch_plan = Some(BatchPlan {
            combinations,
            capped: combinations >= tequila_scene::configurator::MOST_COMBINATIONS,
            each,
            summary: format!("{} × {} px, {} samples", settings.width, settings.height, settings.max_samples),
        });
    }

    fn batch_plan_window(&mut self, ctx: &egui::Context) {
        let Some(plan) = self.batch_plan.clone() else { return };
        let mut open = true;
        let mut queue = false;
        let mut cancel = false;
        egui::Window::new("Render every configuration")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(format!(
                    "{}{} renders, one for each valid combination, at {}.",
                    plan.combinations,
                    if plan.capped { "+" } else { "" },
                    plan.summary
                ));
                ui.label(match plan.each {
                    Some(each) => format!(
                        "About {} in all ({} each), judged from your last renders.",
                        render_window::duration(each * plan.combinations as f64),
                        render_window::duration(each)
                    ),
                    None => "How long is unknown until the first one finishes: set a time limit in Render settings to bound it.".into(),
                });
                ui.label(
                    egui::RichText::new("Files are named by their choices: {configuration} is added to the name template.")
                        .small()
                        .color(theme::p().text_dim),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(format!("Queue {} renders", plan.combinations)).clicked() {
                        queue = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if queue {
            self.batch_plan = None;
            self.queue_configurations(None);
        } else if cancel || !open {
            self.batch_plan = None;
        }
    }

    /// One queued render per valid combination, each snapshotted as that
    /// combination, the scene put back as it was afterwards. `settings`
    /// default to the document's. Returns the jobs.
    pub(crate) fn queue_configurations(&mut self, settings: Option<tequila_scene::render::RenderSettings>) -> Vec<u64> {
        let configurator = self.project.document().configurator.clone();
        let all = configurator.combinations();
        let mut settings = settings.unwrap_or_else(|| self.project.render_settings().clone());
        if !settings.filename_template.contains("{configuration}") {
            settings.filename_template.push_str(" {configuration}");
        }
        let title = if configurator.title.is_empty() { self.project.document().meta.name.clone() } else { configurator.title.clone() };
        // Each combination is applied, captured, and the lot taken back as
        // one: the queue holds the scenes, the document is untouched.
        self.project.begin_group();
        let mut queued = Vec::with_capacity(all.len());
        for choices in &all {
            if self.project.apply(tequila_scene::Command::Configure { choices: choices.clone() }).is_err() {
                continue;
            }
            let words = configurator.describe(choices);
            let camera = self.project.document().cameras.active_index();
            queued.push(self.project.enqueue_configuration(format!("{title} · {}", words.join(", ")), camera, settings.clone(), words.join(" - ")));
        }
        self.project.abandon_group();
        self.camera.adopt(self.project.camera());
        self.submit();
        self.render_window.open = true;
        self.project.status = format!("Queued {} configuration renders", queued.len());
        queued
    }

    /// What is shown, as a `.glb`. Returns its size and anything that did
    /// not export exactly.
    pub(crate) fn export_glb(&mut self, path: &std::path::Path) -> Result<(usize, Vec<String>), String> {
        let options = tequila_format::gltf::GltfOptions { every_model_set: false, ..Default::default() };
        let exported = tequila_format::gltf::export(self.project.document(), &options);
        std::fs::write(path, &exported.glb).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        Ok((exported.glb.len(), exported.warnings))
    }

    /// The configurator as a web folder, now: the live view and, with
    /// `photos`, a render queued for every combination into the folder.
    pub(crate) fn export_web(&mut self, folder: &std::path::Path, live: bool, photos: bool) -> Result<(tequila_format::web::WebReport, Vec<u64>), String> {
        let (options, jobs) = self.plan_web_export(folder, live, photos)?;
        let environment = self.project.scene().environment.clone();
        let report = tequila_format::web::write(folder, self.project.document(), &environment, &options).map_err(|e| format!("Could not write {}: {e}", folder.display()))?;
        Ok((report, jobs))
    }

    /// Queue the photos, if asked for, and say what the export will expect.
    fn plan_web_export(&mut self, folder: &std::path::Path, live: bool, photos: bool) -> Result<(tequila_format::web::WebOptions, Vec<u64>), String> {
        if !live && !photos {
            return Err("Choose live 3D, photos, or both".into());
        }
        if photos && self.project.document().configurator.is_empty() {
            return Err("Photos need a configurator: there is nothing to photograph each combination of".into());
        }
        std::fs::create_dir_all(folder).map_err(|e| format!("Could not make {}: {e}", folder.display()))?;
        let mut jobs = Vec::new();
        let images = photos.then(|| tequila_format::web::fresh_images_folder(folder));
        if let Some(images) = &images {
            let mut settings = self.project.render_settings().clone();
            settings.output_directory = folder.join(images).display().to_string();
            settings.filename_template = "{configuration}".into();
            settings.format = tequila_scene::render::OutputFormat::Jpeg;
            settings.jpeg_quality = 90;
            jobs = self.queue_configurations(Some(settings));
        }
        Ok((tequila_format::web::WebOptions { live, prerendered: images.map(|i| ("jpg".to_string(), i)), ..Default::default() }, jobs))
    }

    fn web_plan_window(&mut self, ctx: &egui::Context) {
        let Some(mut plan) = self.web_plan.clone() else { return };
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        let combinations = self.project.document().configurator.combinations().len();
        let has_configurator = !self.project.document().configurator.is_empty();
        egui::Window::new("Export for the web")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label("A folder that opens in any browser - put it on any web host, or open index.html from the disk. Nothing is downloaded from anywhere.");
                ui.add_space(6.0);
                ui.add(crate::field::Checkbox::new(&mut plan.live, "Live 3D"))
                    .on_hover_text("Turn the product and switch choices instantly, drawn in the browser - an approximation of the render");
                ui.add_enabled(has_configurator, crate::field::Checkbox::new(&mut plan.photos, format!("Photos of every combination ({combinations} renders)")))
                    .on_hover_text("Each combination rendered by the queue into the folder: the real look, a render apiece");
                if !has_configurator {
                    ui.weak("Without a configurator the page shows the scene as it is.");
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(plan.live || plan.photos, egui::Button::new("Choose a folder and export…")).clicked() {
                        go = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        self.web_plan = Some(plan.clone());
        if cancel || !open {
            self.web_plan = None;
        }
        if go {
            self.web_plan = None;
            let Some(folder) = rfd::FileDialog::new().set_title("Folder for the web export").pick_folder() else { return };
            if self.web_export.is_some() {
                self.project.status = "A web export is already being written".into();
                return;
            }
            match self.plan_web_export(&folder, plan.live, plan.photos) {
                Ok((options, jobs)) => {
                    // Baking and prefiltering take seconds: on a thread of
                    // its own, with copies of what it reads.
                    let document = self.project.document().clone();
                    let environment = self.project.scene().environment.clone();
                    self.web_export = Some(std::thread::spawn(move || {
                        tequila_format::web::write(&folder, &document, &environment, &options).map_err(|e| e.to_string())
                    }));
                    self.project.status = if jobs.is_empty() {
                        "Writing the web export…".into()
                    } else {
                        format!("Writing the web export; {} photos queued", jobs.len())
                    };
                }
                Err(message) => self.project.status = message,
            }
        }
    }

    /// A spin viewer: a new folder under `parent`, its page written now and a
    /// render queued for every frame into it. Returns the folder and jobs.
    pub(crate) fn export_spin(
        &mut self,
        parent: &std::path::Path,
        options: &tequila_format::spin::SpinOptions,
        transparent: bool,
    ) -> Result<(std::path::PathBuf, Vec<u64>), String> {
        // A folder of its own, never one with frames already in it: a render
        // never overwrites, so old frames would push the new ones to `_2`.
        let folder = (1..)
            .map(|n| parent.join(if n == 1 { "spin".to_string() } else { format!("spin-{n}") }))
            .find(|f| std::fs::read_dir(f).map(|mut d| d.next().is_none()).unwrap_or(true))
            .expect("some name is free");
        let title = crate::render_window::project_name(&self.project.document().meta.name, self.project.path.as_deref());
        let mut settings = self.project.render_settings().clone();
        settings.output_directory = folder.join("frames").display().to_string();
        settings.region = None;
        if transparent {
            settings.format = tequila_scene::render::OutputFormat::Png;
            settings.alpha_mode = tequila_scene::render::AlphaMode::Silhouette;
        } else {
            settings.format = tequila_scene::render::OutputFormat::Jpeg;
            settings.jpeg_quality = 90;
        }
        let options = tequila_format::spin::SpinOptions { extension: if transparent { "png".into() } else { "jpg".into() }, ..options.clone() };
        tequila_format::spin::write_package(&folder, &title, &options, settings.width, settings.height)
            .map_err(|e| format!("Could not write {}: {e}", folder.display()))?;
        let saved = self.project.document().cameras.active().clone();
        let frames = tequila_format::spin::frames(&saved.camera, saved.target, &options);
        let count = frames.len();
        let mut jobs = Vec::with_capacity(count);
        for (i, frame) in frames.into_iter().enumerate() {
            let mut job = settings.clone();
            job.filename_template = frame.name.clone();
            let camera = tequila_scene::lighting::SavedCamera { name: frame.name, camera: frame.camera, target: saved.target };
            jobs.push(self.project.enqueue_camera(format!("{title} · spin {}/{count}", i + 1), camera, job));
        }
        self.spin_pending.push(SpinPending { folder: folder.clone(), title, options, width: settings.width, height: settings.height, jobs: jobs.clone() });
        self.render_window.open = true;
        self.project.status = format!("Rendering {count} spin frames into {}", folder.display());
        Ok((folder, jobs))
    }

    /// Spin viewers whose frames have all finished: the sprite sheet, and
    /// a word that the folder is ready.
    fn finish_spins(&mut self) {
        if self.spin_pending.is_empty() {
            return;
        }
        let queue = self.project.queue();
        let done = |job: &u64| queue.get(*job).is_none_or(|j| j.state.is_finished());
        let ready: Vec<usize> = self.spin_pending.iter().enumerate().filter(|(_, s)| s.jobs.iter().all(done)).map(|(i, _)| i).collect();
        for i in ready.into_iter().rev() {
            let spin = self.spin_pending.remove(i);
            self.project.status = match tequila_format::spin::finish(&spin.folder, &spin.title, &spin.options, spin.width, spin.height) {
                Ok(true) => format!("The spin viewer is ready: {}", spin.folder.join("index.html").display()),
                Ok(false) => format!("Some spin frames did not render; {} turns without them", spin.folder.display()),
                Err(error) => format!("The spin viewer's sprite sheet failed: {error}"),
            };
        }
    }

    fn spin_plan_window(&mut self, ctx: &egui::Context) {
        let Some(mut plan) = self.spin_plan.clone() else { return };
        let mut open = true;
        let mut go = false;
        let mut cancel = false;
        let settings = self.project.render_settings().clone();
        let finished: Vec<&tequila_scene::render::RenderJob> =
            self.project.queue().jobs().iter().filter(|j| j.state == tequila_scene::render::JobState::Done).collect();
        let each = tequila_scene::render::estimate_seconds(&settings, &finished);
        egui::Window::new("Export spin viewer")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label("Renders all the way round the product from the current camera, and a page that turns them under a finger or a mouse.");
                ui.add_space(6.0);
                egui::Grid::new("spin-plan").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Frames round");
                    ui.add(egui::DragValue::new(&mut plan.options.columns).range(4..=360));
                    ui.end_row();
                    ui.label("Rows of tilt");
                    ui.add(egui::DragValue::new(&mut plan.options.rows).range(1..=19));
                    ui.end_row();
                    if plan.options.rows > 1 {
                        ui.label("Degrees between rows");
                        ui.add(egui::DragValue::new(&mut plan.options.tilt).range(1.0..=45.0).suffix("°"));
                        ui.end_row();
                    }
                    ui.label("Turns");
                    let mut clockwise = plan.options.clockwise;
                    crate::theme::segmented(ui, &mut clockwise, &[(true, "Clockwise"), (false, "Anticlockwise")]);
                    plan.options.clockwise = clockwise;
                    ui.end_row();
                });
                ui.add(crate::field::Checkbox::new(&mut plan.transparent, "Transparent background (PNG, cut out)"));
                let frames = plan.options.columns * plan.options.rows;
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!(
                    "{frames} renders at {} × {} px, {} samples{}.",
                    settings.width,
                    settings.height,
                    settings.max_samples,
                    match each {
                        Some(each) => format!(", about {} in all", crate::render_window::duration(each * frames as f64)),
                        None => String::new(),
                    }
                )).color(theme::p().text_dim));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Choose a folder and render…").clicked() {
                        go = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        self.spin_plan = Some(plan.clone());
        if cancel || !open {
            self.spin_plan = None;
        }
        if go {
            self.spin_plan = None;
            if let Some(parent) = rfd::FileDialog::new().set_title("Folder for the spin viewer").pick_folder() {
                if let Err(error) = self.export_spin(&parent, &plan.options, plan.transparent) {
                    self.project.status = error;
                }
            }
        }
    }

    /// Turn a queued job into something the batch thread can render.
    fn build_job(&mut self, id: u64) -> Option<batch::BatchJob> {
        if let Some(job) = self.project.take_queued_snapshot(id) { return Some(job); }
        if let Some(job) = self.project.queue_mut().get_mut(id) {
            job.state = tequila_scene::render::JobState::Failed;
            job.message = "Scene snapshot unavailable (for example after restarting). Open the source project and queue this render again.".into();
        }
        self.project.save_queue();
        None
    }

    /// Every binding, grouped by what it is for, with the keys drawn as keys.
    fn help_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_help;
        let spec = windows::Spec {
            kind: windows::Kind::Shortcuts,
            title: "Shortcuts".into(),
            size: egui::vec2(780.0, 520.0),
            min_size: egui::vec2(520.0, 320.0),
            resizable: true,
            margin: 18.0,
        };
        windows::show(ctx, spec, &mut open, |window| window.resizable(false), |ui| {
            egui::ScrollArea::vertical().auto_shrink([true, true]).show(ui, |ui| {
                // A Mac's trackpad first (`orbit::Gestures`): a MacBook has
                // no middle button and no wheel.
                let navigate: &[(&str, &str)] = if cfg!(target_os = "macos") {
                    &[
                        ("Orbit", "Left drag / Shift + Swipe"),
                        ("Pan", "Two-finger swipe / Middle drag"),
                        ("Zoom", "Pinch / Ctrl + Swipe / Scroll"),
                        ("Focal length", "Alt + Swipe / Alt + Scroll"),
                        ("Dolly", "Alt + Right drag"),
                        ("Standard views", "Ctrl + Alt + 1-7"),
                        ("Frame selection / all", "F / Shift + F"),
                    ]
                } else {
                    &[
                        ("Orbit", "Left drag"),
                        ("Pan", "Middle drag"),
                        ("Zoom", "Scroll"),
                        ("Focal length", "Alt + Scroll"),
                        ("Dolly", "Alt + Right drag"),
                        ("Standard views", "Ctrl + Alt + 1-7"),
                        ("Frame selection / all", "F / Shift + F"),
                    ]
                };
                let groups: [(&str, &[(&str, &str)]); 4] = [
                    ("Navigate", navigate),
                    (
                        "View",
                        &[
                            ("Step through the shadings", "Z"),
                            ("Rendered, and back", "D"),
                            ("Pause the viewport", "P"),
                            ("Render statistics", "H"),
                            ("Full viewport", "Tab"),
                        ],
                    ),
                    (
                        "Edit",
                        &[
                            ("Move / rotate / scale", "W / E / R"),
                            ("Select the part's group", "Double-click"),
                            ("Group", "Ctrl + G"),
                            ("Hide / show everything", HIDE_KEYS),
                            ("Isolate", "/"),
                            ("Delete", "Delete"),
                            ("Undo / redo", "Ctrl + Z / Ctrl + Y"),
                        ],
                    ),
                    (
                        "Panels and files",
                        &[
                            ("Inspector", "Space"),
                            ("Material library", "M"),
                            ("Studios", "U"),
                            ("Console", "`"),
                            ("Render", "Ctrl + R"),
                            ("Import / save", "Ctrl + I / Ctrl + S"),
                            ("New / open project", "Ctrl + N / Ctrl + O"),
                            ("Save as", "Ctrl + Shift + S"),
                        ],
                    ),
                ];
                ui.columns(2, |columns| {
                    for (index, (title, rows)) in groups.iter().enumerate() {
                        let ui = &mut columns[index / 2];
                        theme::section(ui, title);
                        egui::Grid::new(("bindings", index)).num_columns(2).spacing([18.0, 6.0]).show(ui, |ui| {
                            for (action, binding) in rows.iter() {
                                ui.label(egui::RichText::new(*action).color(theme::p().text));
                                theme::key_caps(ui, binding);
                                ui.end_row();
                            }
                        });
                        ui.add_space(6.0);
                    }
                });
            });
        });
        self.show_help = open;
    }

    /// Whether window `kind` is open, wherever it is drawn.
    pub(crate) fn window_open(&self, kind: windows::Kind) -> bool {
        match kind {
            windows::Kind::Render => self.render_window.open,
            windows::Kind::Graph => self.graph_editor.open,
            windows::Kind::Shortcuts => self.show_help,
            windows::Kind::Connect => self.assistant.providers_open(),
        }
    }
}

/// How far the viewport's islands sit in from its edges.
const OVERLAY_INSET: f32 = 12.0;
/// An island's thickness: a tool and its padding.
const ISLAND: f32 = TOOL + 8.0;
/// A tool button over the viewport.
const TOOL: f32 = 28.0;

/// The rail of Inspector pages: wide enough for each page's name under its
/// icon.
const RAIL: f32 = 76.0;

/// A side panel's heading: its icon on a badge in its colour, its name,
/// what it is for, and a button that puts it away.
fn panel_title(ui: &mut egui::Ui, icon: icons::Icon, hue: egui::Color32, title: &str, hint: &str, close: impl FnOnce(), close_hint: &str) {
    let p = theme::p();
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let (badge, _) = ui.allocate_exact_size(egui::Vec2::splat(34.0), egui::Sense::hover());
        theme::badge(ui.painter(), badge, icon, hue);
        ui.add_space(4.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(egui::RichText::new(title).font(theme::strong(15.5)).color(p.text));
            ui.label(egui::RichText::new(hint).size(11.5).color(p.text_dim));
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if icons::button(ui, icons::Icon::Close, false, 26.0, egui::Sense::click())
                .on_hover_text(close_hint)
                .clicked()
            {
                close();
            }
        });
    });
    ui.add_space(10.0);
    let y = ui.cursor().top();
    let full = ui.clip_rect();
    ui.painter().line_segment(
        [egui::pos2(full.left(), y), egui::pos2(full.right(), y)],
        egui::Stroke::new(1.0_f32, p.line),
    );
    ui.add_space(12.0);
}

/// What the status line says when it has nothing else to.
const TIPS: [&str; 10] = [
    "drag a material from the library straight onto your model",
    "press Z to step through Wireframe, Solid, Material Preview and Rendered",
    "press D to flip between the render and the last fast view",
    "double-click a library tile to put it on the selected parts",
    "press F to frame the selection, Shift+F to frame everything",
    "the sun or moon at the top right changes the theme",
    "number fields take units and maths: try 2.5in or =120/2",
    "press Space to show or hide the Inspector, M for the library",
    "Edit graph on the Material page builds a material from nodes",
    "press H for the render statistics",
];

/// Light or dark, the theme, and the size: changed as they are clicked,
/// and kept for next time.
fn appearance_popover(ui: &mut egui::Ui) {
    let p = theme::p();
    let mut look = theme::appearance();
    let before = look;
    ui.set_width(312.0);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let (badge, _) = ui.allocate_exact_size(egui::Vec2::splat(28.0), egui::Sense::hover());
        theme::badge(ui.painter(), badge, icons::Icon::Palette, p.accent_text);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(egui::RichText::new("Appearance").font(theme::strong(14.5)).color(p.text));
            ui.label(egui::RichText::new("Kept for next time").size(11.5).color(p.text_dim));
        });
    });
    theme::section(ui, "Mode");
    theme::segmented(ui, &mut look.mode, &theme::Mode::ALL.map(|m| (m, m.label())));
    theme::section(ui, "Theme");
    let gap = 8.0;
    let each = (ui.available_width() - gap * 2.0) / 3.0;
    for row in theme::Flavor::ALL.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for flavor in row {
                let (rect, response) = ui.allocate_exact_size(egui::vec2(each, 74.0), egui::Sense::click());
                let chosen = look.flavor == *flavor;
                let sample = theme::palette(*flavor, p.dark);
                let painter = ui.painter();
                // A little window in the theme: its frame, a panel, a card
                // with a control lit in its accent.
                let window = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 52.0));
                painter.rect(window, 8.0, sample.panel, egui::Stroke::new(if chosen { 2.0_f32 } else { 1.0 }, if chosen { sample.accent } else if response.hovered() { p.line_strong } else { p.line }));
                let bar = egui::Rect::from_min_size(window.min + egui::vec2(1.0, 1.0), egui::vec2(window.width() - 2.0, 11.0));
                painter.rect_filled(bar, egui::Rounding { nw: 7.0, ne: 7.0, sw: 0.0, se: 0.0 }, sample.chrome);
                painter.circle_filled(bar.left_center() + egui::vec2(7.0, 0.0), 2.4, sample.accent);
                let card = egui::Rect::from_min_size(window.min + egui::vec2(7.0, 17.0), egui::vec2(window.width() - 14.0, 28.0));
                painter.rect_filled(card, 5.0, sample.card);
                painter.rect_filled(egui::Rect::from_min_size(card.min + egui::vec2(6.0, 7.0), egui::vec2(card.width() * 0.45, 4.0)), 2.0, sample.text_faint);
                painter.rect_filled(egui::Rect::from_min_size(card.min + egui::vec2(6.0, 16.0), egui::vec2(card.width() * 0.3, 5.0)), 2.5, sample.accent);
                if chosen {
                    let tick = egui::pos2(card.right() - 9.0, card.center().y);
                    painter.circle_filled(tick, 7.0, sample.accent);
                    icons::paint(painter, egui::Rect::from_center_size(tick, egui::Vec2::splat(9.0)), icons::Icon::Check, sample.on_accent);
                }
                painter.text(
                    egui::pos2(rect.center().x, rect.bottom() - 9.0),
                    egui::Align2::CENTER_CENTER,
                    flavor.label(),
                    if chosen { theme::strong(12.0) } else { egui::FontId::proportional(12.0) },
                    if chosen { p.text } else { p.text_dim },
                );
                if response.on_hover_text(flavor.blurb()).clicked() {
                    look.flavor = *flavor;
                }
            }
        });
    }
    theme::section(ui, "Size");
    theme::segmented(ui, &mut look.size, &theme::Size::ALL.map(|s| (s, s.label())));
    ui.add_space(4.0);
    ui.label(egui::RichText::new(look.flavor.blurb()).size(11.5).color(p.text_faint));
    if look != before {
        theme::set_appearance(ui.ctx(), look, true);
    }
}

/// Today, as `YYYY-MM-DD`, in local time. For `{date}` in filenames.
fn today() -> String {
    clock::date()
}

/// The time of day as `HH-MM-SS`, in local time. For `{time}` in filenames.
fn now() -> String {
    clock::time_for_filename()
}

/// Small convenience so the framing call reads clearly.
trait VecY {
    fn new_y(y: f32) -> Vec3;
}

impl VecY for Vec3 {
    fn new_y(y: f32) -> Vec3 {
        tequila_core::math::vec3(0.0, y, 0.0)
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;
    use tequila_core::units::LengthUnit;
    use tequila_scene::units::ConversionMode;

    /// Undoing a step that moved the camera takes the view back with it, and
    /// redo brings it forward again. The viewport writes its camera into the
    /// document every frame, so a view left where it was wrote the undone
    /// camera straight back - as a new step, which also lost the redo.
    #[test]
    fn undo_and_redo_take_the_view_with_the_camera() {
        let mut app = TequilaApp::for_tests();
        app.sync_viewport_camera();
        app.project.end_interaction();
        let before = app.project.camera().clone();

        // What the Scene tab does for "the model is the wrong size".
        app.project
            .apply(Command::SetUnits { unit: LengthUnit::Millimetre, mode: ConversionMode::ScaleScene })
            .unwrap();
        app.camera.adopt(app.project.camera());
        let after = app.project.camera().clone();
        assert_ne!(after, before, "the rescale should have moved the camera");

        app.undo();
        app.sync_viewport_camera();
        assert_eq!(*app.project.camera(), before);
        assert!(app.project.can_redo(), "the next frame wrote over the undone camera");

        app.redo();
        app.sync_viewport_camera();
        assert_eq!(*app.project.camera(), after);
    }

    /// Help > About lays out, with or without a BUILD.txt beside the program.
    #[test]
    fn the_about_window_lays_out() {
        let mut app = TequilaApp::for_tests();
        app.show_about = true;
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
        for _ in 0..3 {
            let _ = ctx.run(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |ctx| app.about_window(ctx));
        }
        assert!(app.show_about, "it closed itself");
        let note = read_build_note("\u{feff}Source: 5198cc44c7dc\nBuilt: 2026-10-15T14:06:02+08:00\nIncludes current working-tree changes.\n");
        assert_eq!(note, Some(("2026-10-15T14:06:02+08:00".to_string(), "5198cc44c7dc".to_string())), "the byte-order mark hid the source");
        assert_eq!(GUIDES.iter().filter(|(_, packaged, docs, _)| shipped_file(packaged, &format!("docs/{docs}")).is_some()).count(), GUIDES.len(), "a guide is missing from docs");
    }

    /// In a Mac app bundle the guides, BUILD.txt and the skill sit in
    /// `Contents/Resources`, not beside the executable in `Contents/MacOS`.
    #[test]
    fn shipped_files_are_found_in_a_mac_bundle() {
        let root = std::env::temp_dir().join(format!("tequila-bundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let contents = root.join("Tequila.app").join("Contents");
        std::fs::create_dir_all(contents.join("MacOS")).unwrap();
        std::fs::create_dir_all(contents.join("Resources")).unwrap();
        std::fs::write(contents.join("Resources").join("BUILD.txt"), "Built: now\n").unwrap();
        assert_eq!(shipped_in(&contents.join("MacOS"), "BUILD.txt"), Some(contents.join("Resources").join("BUILD.txt")));
        assert_eq!(shipped_in(&contents.join("MacOS"), "PLUGINS.md"), None);
        // Beside the executable first, as on Windows; and a folder that is
        // not a bundle's is not looked past.
        std::fs::write(contents.join("MacOS").join("BUILD.txt"), "Built: beside\n").unwrap();
        assert_eq!(shipped_in(&contents.join("MacOS"), "BUILD.txt"), Some(contents.join("MacOS").join("BUILD.txt")));
        std::fs::create_dir_all(root.join("Tequila")).unwrap();
        std::fs::create_dir_all(root.join("Resources")).unwrap();
        std::fs::write(root.join("Resources").join("SCRIPTING.md"), "").unwrap();
        assert_eq!(shipped_in(&root.join("Tequila"), "SCRIPTING.md"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A close nobody was asked about - a Mac's Quit from the Dock, logging
    /// out - leaves unsaved work recoverable; an agreed close, or one with
    /// nothing unsaved, leaves nothing behind.
    #[test]
    fn an_unasked_close_keeps_unsaved_work_for_recovery() {
        let directory = std::env::temp_dir().join(format!("tequila-unasked-close-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let session = |app: &mut TequilaApp, id: &str, edit: bool| {
            app.project.begin_session(&directory, id);
            if edit {
                app.project.apply(Command::AddGroup { parent: None, name: "Unsaved".into() }).unwrap();
            }
        };
        let left = || tequila_format::journal::recoverable(&directory, "").into_iter().map(|r| r.id).collect::<Vec<_>>();

        let mut unasked = TequilaApp::for_tests();
        session(&mut unasked, "unasked", true);
        unasked.end_session();
        assert_eq!(left(), vec!["unasked".to_string()], "the unsaved work was thrown away");
        tequila_format::journal::forget(&directory, "unasked");

        let mut agreed = TequilaApp::for_tests();
        session(&mut agreed, "agreed", true);
        agreed.close_approved = true;
        agreed.end_session();
        let mut scripted = TequilaApp::for_tests();
        session(&mut scripted, "scripted", true);
        scripted.quit_requested = true;
        scripted.end_session();
        let mut clean = TequilaApp::for_tests();
        session(&mut clean, "clean", false);
        clean.end_session();
        assert_eq!(left(), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Apply to selection (a library tile's double-click or menu) with a
    /// group selected puts the material on the group's parts: on the group
    /// itself it was never drawn.
    #[test]
    fn a_material_applied_to_a_selected_group_reaches_its_parts() {
        let mut app = TequilaApp::for_tests();
        let (entries, _) = tequila_format::library::read_directory_from(&library_tab::standard_library_path(), tequila_format::library::Source::Standard);
        let entry = entries.iter().find(|e| e.name == "Aluminium Spun").expect("ships").clone();
        let document = app.project.document();
        let group = document.tree.iter().find(|n| matches!(n.kind, tequila_scene::tree::NodeKind::Group)).map(|n| n.id).unwrap();
        let parts = tequila_scene::apply::parts_of(document, [group]);
        app.project.selection.select_many([group], SelectionMode::Replace);
        app.library_action(library_tab::LibraryAction::ApplyToSelection(Box::new(entry)));
        let document = app.project.document();
        for part in parts {
            let name = document.tree.get(part).and_then(|n| n.material).and_then(|k| document.materials.get(k)).map(|m| m.name.clone());
            assert_eq!(name.as_deref(), Some("Aluminium Spun"), "the part kept its old material");
        }
        // One undo takes it back.
        assert_eq!(app.project.undo_step().map(|(name, _)| name), Some("material.assign"));
    }

    /// Double-click selects the group around the part, and again the group
    /// around that, up to the outermost; a part in no group stays itself.
    #[test]
    fn a_double_click_steps_up_through_the_groups() {
        use tequila_scene::tree::{NodeId, NodeKind, SceneNode};
        let mut document = tequila_scene::SceneDocument::new();
        let ids: Vec<NodeId> = (1..=4).map(NodeId).collect();
        let mut nodes = vec![
            SceneNode::new(ids[0], "Mixer", NodeKind::Group),
            SceneNode::new(ids[1], "Housing", NodeKind::Group),
            SceneNode::new(ids[2], "Housing (Red)", NodeKind::Part { mesh: tequila_scene::tree::MeshKey(1) }),
            SceneNode::new(ids[3], "Loose", NodeKind::Part { mesh: tequila_scene::tree::MeshKey(1) }),
        ];
        nodes[1].parent = Some(ids[0]);
        nodes[2].parent = Some(ids[1]);
        let mesh = tequila_scene::geometry::Mesh::new(vec![Vec3::ZERO, tequila_core::math::vec3(1.0, 0.0, 0.0), tequila_core::math::vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
        tequila_scene::command::CommandBus::new()
            .apply(&mut document, Command::Import {
                payload: Box::new(tequila_scene::command::ImportPayload {
                    meshes: vec![(tequila_scene::tree::MeshKey(1), mesh)],
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes,
                    roots: vec![ids[0], ids[3]],
                }),
            })
            .unwrap();
        // A piece of a body: first its body, then the assembly, then it stays there.
        assert_eq!(enclosing_group(&document, ids[2], &[]), Some(ids[1]));
        assert_eq!(enclosing_group(&document, ids[2], &[ids[1]]), Some(ids[0]));
        assert_eq!(enclosing_group(&document, ids[2], &[ids[0]]), Some(ids[0]));
        // From another selection, its own group again.
        assert_eq!(enclosing_group(&document, ids[2], &[ids[3]]), Some(ids[1]));
        assert_eq!(enclosing_group(&document, ids[3], &[]), None);
    }

    #[test]
    fn the_window_fits_a_laptop_screen() {
        let first = egui::vec2(1440.0, 900.0);
        // A 13-inch MacBook Air: the height goes to the menu bar and Dock.
        let air = fitted_window(first, egui::vec2(1470.0, 956.0)).expect("too tall for it");
        assert!(air.y <= 956.0 * 0.86 && air.x <= 1470.0, "{air:?}");
        // A desktop monitor fits it already; nothing shrinks past the minimum.
        assert_eq!(fitted_window(first, egui::vec2(1920.0, 1080.0)), None);
        assert_eq!(fitted_window(first, egui::vec2(600.0, 400.0)), Some(egui::vec2(640.0, 480.0)));
    }

    /// On a Retina screen the Rendered viewport asks for the screen's own
    /// pixels, which Cycles renders. It asked for a pixel per point, which
    /// there is a picture half the viewport's size, stretched over it. A
    /// screen of one pixel per point still asks for a pixel per point,
    /// whatever the text size; and Tequila's own renderer, on the
    /// processor, traces a pixel per point on either.
    #[test]
    fn a_retina_viewport_asks_for_the_screens_own_pixels() {
        // The screen's pixels per point, the text size's zoom, and the
        // picture's pixels per point of viewport: 1.84 is a MacBook's at
        // the Compact size.
        for (native, zoom, scale) in [(2.0, 0.92, 1.84f32), (2.0, 1.0, 2.0), (1.0, 0.92, 1.0), (1.0, 1.0, 1.0)] {
            let mut app = TequilaApp::for_tests();
            // The whole viewport: this is about pixels per point, not about
            // the render frame's shape.
            app.show_render_frame = false;
            // Tequila's renderer, which any test machine has: Cycles on a
            // graphics card renders the size asked for.
            let settings = tequila_scene::render::RenderSettings {
                engine: tequila_scene::render::RenderEngine::Tequila,
                ..app.project.render_settings().clone()
            };
            app.project.apply(Command::SetRenderSettings { settings: Box::new(settings) }).unwrap();
            app.set_view_mode(view_mode::ViewMode::Rendered);
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            ctx.options_mut(|o| o.zoom_factor = zoom);
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 150.0))),
                ..Default::default()
            };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| app.viewport(ui));
            });
            assert_eq!(app.viewport_size, (200, 150));
            let request = app.orchestrator.request().expect("the viewport asked for nothing");
            let expected = ((200.0 * scale).round() as usize, (150.0 * scale).round() as usize);
            assert_eq!((request.width, request.height), expected, "at {native} pixels a point and a zoom of {zoom}");
            assert!((request.pixels_per_point - scale).abs() < 1e-5, "{} at {native} and {zoom}", request.pixels_per_point);
            // Tequila's own renderer still traces the points.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let mut seen = 0;
            loop {
                if let Some(image) = app.orchestrator.take_image(&mut seen) {
                    if image.resolution_divisor == 1 {
                        assert_eq!((image.width, image.height), (200, 150), "traced at {native} pixels a point");
                        break;
                    }
                }
                assert!(std::time::Instant::now() < deadline, "no full-size picture at {native} pixels a point");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    /// With the render frame on (the default), the viewport renders at the
    /// render's shape: a 16:9 render in a 4:3 viewport asks for a 16:9
    /// picture across its width, so nothing on screen is cut from the
    /// render. Turned off, the picture fills the viewport again.
    #[test]
    fn the_viewport_takes_the_renders_shape() {
        let mut app = TequilaApp::for_tests();
        assert!(app.show_render_frame, "the render frame is on by default");
        let settings = tequila_scene::render::RenderSettings {
            engine: tequila_scene::render::RenderEngine::Tequila,
            width: 1920,
            height: 1080,
            ..app.project.render_settings().clone()
        };
        app.project.apply(Command::SetRenderSettings { settings: Box::new(settings) }).unwrap();
        app.set_view_mode(view_mode::ViewMode::Rendered);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let run = |app: &mut TequilaApp| {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))),
                ..Default::default()
            };
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(1.0);
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| app.viewport(ui));
            });
        };
        run(&mut app);
        assert_eq!(app.viewport_size, (400, 225), "the picture is not the render's shape");
        let request = app.orchestrator.request().expect("the viewport asked for nothing");
        assert_eq!((request.width, request.height), (400, 225));
        assert!(app.viewport_rect.center().distance(egui::pos2(200.0, 150.0)) <= 0.5, "the picture is not centred");

        app.show_render_frame = false;
        run(&mut app);
        assert_eq!(app.viewport_size, (400, 300), "with the frame off the picture fills the viewport");
    }

    /// Import a file through the same path the File menu and a drop take.
    fn import_through_the_menu(app: &mut TequilaApp, path: std::path::PathBuf) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = sender.send(tequila_import::read_file(&path).and_then(project::PreparedImport::prepare).map(Arrived::Prepared));
        app.imports.push(PendingImport {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path,
            receiver,
            document_epoch: app.project.document_epoch(),
            update: None,
            started: std::time::Instant::now(),
        });
        app.poll_imports();
    }

    fn names(app: &TequilaApp) -> Vec<String> {
        app.project.document().tree.iter().map(|n| n.name.clone()).collect()
    }

    /// A first model replaces the starter sphere, as one undo step, keeping
    /// the look; a scene that is more than the starter keeps everything,
    /// and a failed import leaves the starter where it was.
    #[test]
    fn a_first_import_replaces_the_starter_sphere() {
        let folder = std::env::temp_dir().join(format!("tequila-starter-import-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let plate = folder.join("plate.obj");
        std::fs::write(&plate, "v 0 0 0\nv 0.1 0 0\nv 0.1 0.1 0\nf 1 2 3\n").unwrap();

        let mut app = TequilaApp::for_tests();
        app.project
            .apply(Command::SetEnvironmentParameter {
                parameter: tequila_scene::command::EnvironmentParameter::new(tequila_scene::command::EnvParam::Brightness, tequila_scene::command::EnvValue::Scalar(1.7)),
            })
            .unwrap();
        import_through_the_menu(&mut app, plate.clone());
        let after = names(&app);
        assert!(!after.iter().any(|n| n == "Demo" || n == "Sphere"), "the starter stayed: {after:?}");
        assert!(after.iter().any(|n| n.starts_with("plate")), "{after:?}");
        assert_eq!(app.project.document().environment.brightness, 1.7, "the look was lost");
        // The framing that follows an import is a step of its own, as ever;
        // under it, the starter's removal and the import are one.
        assert_eq!(app.project.undo_step().map(|(name, _)| name), Some("camera.set"));
        app.undo();
        assert_eq!(app.project.undo_step(), Some(("scene.import", 2)));
        app.undo();
        assert_eq!(names(&app), vec!["Demo".to_string(), "Sphere".to_string()], "not one undo step");

        // More than the starter: a second model sits beside the first.
        import_through_the_menu(&mut app, plate.clone());
        import_through_the_menu(&mut app, plate.clone());
        assert_eq!(names(&app).iter().filter(|n| n.starts_with("plate")).count(), 2, "{:?}", names(&app));

        // A file that does not read leaves the starter.
        let mut app = TequilaApp::for_tests();
        let broken = folder.join("broken.obj");
        std::fs::write(&broken, "this is not a model").unwrap();
        import_through_the_menu(&mut app, broken);
        assert_eq!(names(&app), vec!["Demo".to_string(), "Sphere".to_string()]);
        let _ = std::fs::remove_dir_all(&folder);
    }
}
