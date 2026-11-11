//! Preferences (`docs/04` §12): every choice Tequila makes for the person,
//! in one window.
//!
//! Each choice is a [`Setting`] in one table, [`SETTINGS`]: its page, its
//! words, what kind of value it takes, its default, and how the application
//! reads and takes it. The window, its search, the reset marks, the file
//! that is exported and the scripting channel's `preferences` operation
//! are all drawn from that table, so a setting added there is at once in
//! every one of them.
//!
//! Tequila's newer choices are kept here, in `preferences.txt` beside the
//! person's other settings ([`Prefs`]). The older ones - the look, the up
//! axis of imports, the screenshot folder, updates, the 3D mouse - are
//! still kept where they were, by the code that owns them; the table reads
//! and sets them through it, so they are in the window, the export and
//! scripts all the same.

use crate::shortcuts::{Action, Chord, Shortcuts};
use crate::view_mode::ViewMode;
use crate::TequilaApp;
use eframe::egui;
use std::path::PathBuf;
use tequila_scene::gizmo::GizmoMode;

/// A setting's value: on or off, a number, or a word - a choice's key, or
/// a folder.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
}

impl Value {
    /// As a line of the settings file holds it.
    pub fn to_text(&self) -> String {
        match self {
            Value::Bool(on) => if *on { "on" } else { "off" }.to_string(),
            Value::Number(n) => format!("{n}"),
            Value::Text(text) => text.clone(),
        }
    }

    pub fn to_json(&self) -> tequila_format::json::Value {
        match self {
            Value::Bool(on) => tequila_format::json::Value::Bool(*on),
            Value::Number(n) => tequila_format::json::Value::Number(*n),
            Value::Text(text) => tequila_format::json::Value::String(text.clone()),
        }
    }

    fn same(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Number(a), Value::Number(b)) => (a - b).abs() < 1e-6,
            _ => self == other,
        }
    }
}

/// What a setting takes.
pub enum Kind {
    Toggle,
    Number { min: f64, max: f64, decimals: usize, suffix: &'static str, logarithmic: bool },
    /// One of these: each a key, for the file and a script, and its words.
    Choice(&'static [(&'static str, &'static str)]),
    Folder,
}

/// The window's pages, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Interface,
    Viewport,
    Navigation,
    Keyboard,
    Import,
    Folders,
    Updates,
    Advanced,
}

impl Page {
    pub const ALL: [Page; 9] =
        [Page::General, Page::Interface, Page::Viewport, Page::Navigation, Page::Keyboard, Page::Import, Page::Folders, Page::Updates, Page::Advanced];

    pub fn label(self) -> &'static str {
        match self {
            Page::General => "General",
            Page::Interface => "Interface",
            Page::Viewport => "Viewport",
            Page::Navigation => "Navigation",
            Page::Keyboard => "Keyboard",
            Page::Import => "Import",
            Page::Folders => "Folders",
            Page::Updates => "Updates",
            Page::Advanced => "Advanced",
        }
    }

    /// Its name in a script.
    pub fn key(self) -> &'static str {
        match self {
            Page::General => "general",
            Page::Interface => "interface",
            Page::Viewport => "viewport",
            Page::Navigation => "navigation",
            Page::Keyboard => "keyboard",
            Page::Import => "import",
            Page::Folders => "folders",
            Page::Updates => "updates",
            Page::Advanced => "advanced",
        }
    }

    pub fn from_key(key: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.key().eq_ignore_ascii_case(key.trim()))
    }

    fn blurb(self) -> &'static str {
        match self {
            Page::General => "How Tequila starts, and what it keeps as you work.",
            Page::Interface => "How Tequila looks.",
            Page::Viewport => "What the view shows, and how the Rendered view draws.",
            Page::Navigation => "How the mouse, a trackpad and a 3D mouse move the view.",
            Page::Keyboard => "Which keys do what. Click a key, then press the one you want.",
            Page::Import => "How model files are read.",
            Page::Folders => "Where Tequila keeps things.",
            Page::Updates => "New versions of Tequila.",
            Page::Advanced => "All the settings at once: saved to a file, read from one, or put back.",
        }
    }

    fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Page::General => Icon::House,
            Page::Interface => Icon::Palette,
            Page::Viewport => Icon::Eye,
            Page::Navigation => Icon::Camera,
            Page::Keyboard => Icon::Keyboard,
            Page::Import => Icon::Import,
            Page::Folders => Icon::FolderOpen,
            Page::Updates => Icon::Download,
            Page::Advanced => Icon::Sliders,
        }
    }
}

/// One choice the person can make.
pub struct Setting {
    /// Its name in the file and a script: `viewport.gizmo_size`.
    pub key: &'static str,
    pub page: Page,
    /// The heading it is under on its page.
    pub group: &'static str,
    pub label: &'static str,
    /// What it does and what it costs, for its tooltip and the search.
    pub help: &'static str,
    pub kind: Kind,
    /// Kept in `preferences.txt` ([`Prefs`]) rather than by other code.
    pub own: bool,
    pub default: fn() -> Value,
    pub get: fn(&TequilaApp) -> Value,
    /// Take a value already checked ([`Setting::checked`]), acting on it
    /// at once.
    pub set: fn(&mut TequilaApp, Value) -> Result<(), String>,
}

impl Setting {
    /// `value` as this setting takes it: a number held to its range and
    /// rounded to its decimals, a choice that is one of its own. An error
    /// in words otherwise.
    pub fn checked(&self, value: Value) -> Result<Value, String> {
        match (&self.kind, value) {
            (Kind::Toggle, Value::Bool(on)) => Ok(Value::Bool(on)),
            (Kind::Toggle, other) => Err(format!("`{}` is on or off (true or false), not {}", self.key, other.to_text())),
            (Kind::Number { min, max, decimals, .. }, Value::Number(n)) if n.is_finite() => {
                let scale = 10f64.powi(*decimals as i32);
                Ok(Value::Number((n.clamp(*min, *max) * scale).round() / scale))
            }
            (Kind::Number { min, max, .. }, other) => Err(format!("`{}` is a number from {min} to {max}, not {}", self.key, other.to_text())),
            (Kind::Choice(options), Value::Text(text)) => match options.iter().find(|(key, _)| key.eq_ignore_ascii_case(text.trim())) {
                Some((key, _)) => Ok(Value::Text((*key).to_string())),
                None => Err(format!("`{}` is one of {}, not `{text}`", self.key, options.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", "))),
            },
            (Kind::Choice(options), other) => {
                Err(format!("`{}` is one of {}, not {}", self.key, options.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", "), other.to_text()))
            }
            (Kind::Folder, Value::Text(text)) => Ok(Value::Text(text.trim().to_string())),
            (Kind::Folder, other) => Err(format!("`{}` is a folder's path, not {}", self.key, other.to_text())),
        }
    }

    /// A line of the settings file, read as this setting's value.
    pub fn parse(&self, text: &str) -> Result<Value, String> {
        let text = text.trim();
        let value = match self.kind {
            Kind::Toggle => match text.to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" | "1" => Value::Bool(true),
                "off" | "false" | "no" | "0" => Value::Bool(false),
                _ => return Err(format!("`{}` is on or off, not `{text}`", self.key)),
            },
            Kind::Number { .. } => Value::Number(text.parse::<f64>().map_err(|_| format!("`{}` is a number, not `{text}`", self.key))?),
            Kind::Choice(_) | Kind::Folder => Value::Text(text.to_string()),
        };
        self.checked(value)
    }

    /// A script's value, read as this setting's.
    pub fn from_json(&self, value: &tequila_format::json::Value) -> Result<Value, String> {
        use tequila_format::json::Value as Json;
        let value = match value {
            Json::Bool(on) => Value::Bool(*on),
            Json::Number(n) => Value::Number(*n),
            Json::String(text) => match self.kind {
                // "on", "5": a script that sends words is read as the file is.
                Kind::Toggle | Kind::Number { .. } => return self.parse(text),
                _ => Value::Text(text.clone()),
            },
            _ => return Err(format!("`{}` takes true or false, a number or a word", self.key)),
        };
        self.checked(value)
    }

    /// The value in the words the window shows: a choice's label, a number
    /// with its unit.
    pub fn shown(&self, value: &Value) -> String {
        match (&self.kind, value) {
            (Kind::Toggle, Value::Bool(on)) => if *on { "on" } else { "off" }.to_string(),
            (Kind::Number { decimals, suffix, .. }, Value::Number(n)) => format!("{n:.*}{suffix}", *decimals),
            (Kind::Choice(options), Value::Text(key)) => options.iter().find(|(k, _)| k == key).map_or(key.clone(), |(_, label)| (*label).to_string()),
            (_, value) => value.to_text(),
        }
    }

    /// Whether every word of `query` is somewhere in its words.
    fn matches(&self, words: &[String]) -> bool {
        let mut hay = format!("{} {} {} {} {}", self.label, self.help, self.group, self.page.label(), self.key).to_lowercase();
        if let Kind::Choice(options) = self.kind {
            for (_, label) in options {
                hay.push(' ');
                hay.push_str(&label.to_lowercase());
            }
        }
        words.iter().all(|word| hay.contains(word.as_str()))
    }
}

/// How the Rendered view trades its picture's size for speed while the
/// view moves: the time a moving picture is given (`cycles_view`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MovingPicture {
    Smooth,
    #[default]
    Balanced,
    Sharp,
}

impl MovingPicture {
    const ALL: [MovingPicture; 3] = [MovingPicture::Smooth, MovingPicture::Balanced, MovingPicture::Sharp];

    fn key(self) -> &'static str {
        match self {
            MovingPicture::Smooth => "smooth",
            MovingPicture::Balanced => "balanced",
            MovingPicture::Sharp => "sharp",
        }
    }

    /// Milliseconds a moving picture may take: the view sizes it to this.
    pub fn target_ms(self) -> u32 {
        match self {
            MovingPicture::Smooth => 22,
            MovingPicture::Balanced => 33,
            MovingPicture::Sharp => 50,
        }
    }
}

/// Tequila's own choices, kept in `preferences.txt`. Each is a [`Setting`]
/// marked `own`, which reads and writes its field here.
#[derive(Clone, Debug, PartialEq)]
pub struct Prefs {
    pub autosave: bool,
    pub autosave_minutes: f32,
    pub recent_projects: usize,
    pub tooltips: bool,
    pub tooltip_delay: f32,
    /// How the material library shows its shelf.
    pub library_view: crate::library_tab::View,
    /// The Material Graph editor's browser, down its left side.
    pub graph_browser: bool,
    /// What a session starts with: its shading and what the view shows.
    pub shading: ViewMode,
    pub show_grid: bool,
    pub show_axis_legend: bool,
    pub show_render_frame: bool,
    pub show_statistics: bool,
    pub show_model_edges: bool,
    pub show_outline: bool,
    pub show_gizmo: bool,
    /// The gizmo's size on screen, in percent of its usual.
    pub gizmo_size: f32,
    pub gizmo_mode: GizmoMode,
    pub moving_picture: MovingPicture,
    /// Samples the Cycles view refines to.
    pub view_samples: u32,
    pub orbit_speed: f32,
    pub pan_speed: f32,
    pub zoom_speed: f32,
    pub invert_zoom: bool,
    /// A trackpad's two-finger swipe orbits and Shift pans, rather than
    /// the other way about.
    pub trackpad_orbits: bool,
    /// Whose way the mouse moves the view, and the person's own three
    /// drags, used when that is Custom (`navigation`).
    pub nav_preset: crate::navigation::Preset,
    pub nav_custom: crate::navigation::Scheme,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs {
            autosave: true,
            autosave_minutes: 5.0,
            recent_projects: 10,
            tooltips: true,
            tooltip_delay: 0.5,
            library_view: crate::library_tab::View::default(),
            graph_browser: true,
            shading: ViewMode::default(),
            show_grid: false,
            show_axis_legend: true,
            show_render_frame: true,
            show_statistics: false,
            show_model_edges: false,
            show_outline: true,
            show_gizmo: true,
            gizmo_size: 100.0,
            gizmo_mode: GizmoMode::Universal,
            moving_picture: MovingPicture::Balanced,
            view_samples: 1024,
            orbit_speed: 1.0,
            pan_speed: 1.0,
            zoom_speed: 1.0,
            invert_zoom: false,
            trackpad_orbits: false,
            nav_preset: crate::navigation::Preset::default(),
            nav_custom: crate::navigation::Preset::default().scheme().expect("Tequila's own has drags"),
        }
    }
}

impl Prefs {
    /// How often the recovery copy is made, or never.
    pub fn autosave_every(&self) -> Option<std::time::Duration> {
        self.autosave.then(|| std::time::Duration::from_secs_f32(self.autosave_minutes.clamp(1.0, 60.0) * 60.0))
    }

    /// Which drags orbit, pan and zoom: the preset's, or the person's own.
    pub fn navigation(&self) -> crate::navigation::Scheme {
        self.nav_preset.scheme().unwrap_or(self.nav_custom)
    }

    /// What a wheel notch or a pinch is multiplied by.
    pub fn zoom_factor(&self) -> f32 {
        if self.invert_zoom {
            -self.zoom_speed
        } else {
            self.zoom_speed
        }
    }
}

/// Where Tequila's own choices are kept.
pub fn file() -> PathBuf {
    tequila_format::library::settings_directory().join("preferences.txt")
}

const MODES: &[(&str, &str)] = &[("light", "Light"), ("dark", "Dark"), ("auto", "Auto")];
const THEMES: &[(&str, &str)] =
    &[("graphite", "Graphite"), ("ocean", "Ocean"), ("orchid", "Orchid"), ("ember", "Ember"), ("meadow", "Meadow"), ("rose", "Rose")];
const SIZES: &[(&str, &str)] = &[("compact", "Compact"), ("regular", "Regular"), ("large", "Large")];
const LIBRARY_VIEWS: &[(&str, &str)] = &[("thumbnails", "Thumbnails"), ("icons", "Icons"), ("list", "List"), ("cards", "Cards")];
const SHADINGS: &[(&str, &str)] = &[("wireframe", "Wireframe"), ("solid", "Solid"), ("material", "Material Preview"), ("rendered", "Rendered")];
const GIZMO_MODES: &[(&str, &str)] = &[("all", "All at once"), ("move", "Move"), ("rotate", "Rotate"), ("scale", "Scale")];
const MOVING: &[(&str, &str)] = &[("smooth", "Smooth"), ("balanced", "Balanced"), ("sharp", "Sharp")];
const PRESETS: &[(&str, &str)] = &[
    ("keyshot", "Tequila (as KeyShot)"),
    ("blender", "Blender"),
    ("maya", "Maya"),
    ("cinema4d", "Cinema 4D"),
    ("3dsmax", "3ds Max"),
    ("solidworks", "SolidWorks"),
    ("rhino", "Rhino"),
    ("fusion360", "Fusion 360"),
    ("custom", "Custom"),
];
const TRACKPAD: &[(&str, &str)] = &[("pan", "Pans the view"), ("orbit", "Orbits the view")];
const BLENDER_LIGHTS: &[(&str, &str)] = &[("ask", "Ask each time"), ("blender", "Use Blender's lighting"), ("tequila", "Keep Tequila's lighting")];

const UP_AXES: &[(&str, &str)] = &[
    ("file", "As the file says"),
    ("y", "Y up"),
    ("z", "Z up"),
    ("x", "X up"),
    ("-y", "−Y up (upside down)"),
    ("-z", "−Z up"),
    ("-x", "−X up"),
];

/// The look as it will be: a change waiting for the next frame, or the
/// one in use.
fn look(app: &TequilaApp) -> crate::theme::Appearance {
    app.pending_look.unwrap_or_else(crate::theme::appearance)
}

/// Change the look and keep the change, at the next frame, where the
/// context is.
fn set_look(app: &mut TequilaApp, change: impl FnOnce(&mut crate::theme::Appearance)) {
    let mut next = look(app);
    change(&mut next);
    app.pending_look = Some(next);
    app.keep_pending_look = true;
}

fn text(value: Value) -> String {
    match value {
        Value::Text(text) => text,
        other => other.to_text(),
    }
}

/// The drag that makes one of the view's moves, as a setting: read from
/// the preset in use, and set into a scheme of the person's own.
macro_rules! own_drag {
    ($key:literal, $label:literal, $help:literal, $what:ident) => {
        Setting {
            key: $key,
            page: Page::Navigation,
            group: BUTTONS,
            label: $label,
            help: $help,
            kind: Kind::Choice(crate::navigation::DRAGS),
            own: true,
            default: || Value::Text(Prefs::default().navigation().drag(crate::navigation::Move::$what).key()),
            get: |app| Value::Text(app.prefs.navigation().drag(crate::navigation::Move::$what).key()),
            set: |app, value| {
                let what = crate::navigation::Move::$what;
                let mut scheme = app.prefs.navigation();
                // The drag it has already - the preset's own, read back
                // from the file - changes nothing, and is not Custom.
                if let Some(drag) = crate::navigation::Drag::from_key(&text(value)).filter(|drag| *drag != scheme.drag(what)) {
                    scheme.set(what, drag);
                    app.prefs.nav_custom = scheme;
                    app.prefs.nav_preset = crate::navigation::Preset::Custom;
                }
                Ok(())
            },
        }
    };
}

const BUTTONS: &str = "Mouse buttons";

/// One of Tequila's own on-or-off choices: a field of [`Prefs`], and what
/// changing it does to the session.
macro_rules! own_toggle {
    ($key:literal, $page:ident, $group:expr, $label:literal, $help:literal, $field:ident, $apply:expr) => {
        Setting {
            key: $key,
            page: Page::$page,
            group: $group,
            label: $label,
            help: $help,
            kind: Kind::Toggle,
            own: true,
            default: || Value::Bool(Prefs::default().$field),
            get: |app| Value::Bool(app.prefs.$field),
            set: |app, value| {
                if let Value::Bool(on) = value {
                    app.prefs.$field = on;
                    let apply: fn(&mut TequilaApp, bool) = $apply;
                    apply(app, on);
                }
                Ok(())
            },
        }
    };
}

/// One of Tequila's own numbers: a field of [`Prefs`], read where it is used.
macro_rules! own_number {
    ($key:literal, $page:ident, $group:expr, $label:literal, $help:literal, $field:ident as $ty:ty, $min:literal ..= $max:literal, $decimals:literal, $suffix:literal, $log:literal) => {
        Setting {
            key: $key,
            page: Page::$page,
            group: $group,
            label: $label,
            help: $help,
            kind: Kind::Number { min: $min, max: $max, decimals: $decimals, suffix: $suffix, logarithmic: $log },
            own: true,
            default: || Value::Number(Prefs::default().$field as f64),
            get: |app| Value::Number(app.prefs.$field as f64),
            set: |app, value| {
                if let Value::Number(n) = value {
                    app.prefs.$field = n as $ty;
                }
                Ok(())
            },
        }
    };
}

const STARTS: &str = "When Tequila starts";

/// Every setting, in the order the window shows them.
pub static SETTINGS: &[Setting] = &[
    // ---- General ---------------------------------------------------------
    Setting {
        key: "general.start_screen",
        page: Page::General,
        group: STARTS,
        label: "Show the start screen",
        help: "Recent projects, the sample scenes and a new project, each time Tequila starts. Help › Start screen opens it at any time.",
        kind: Kind::Toggle,
        own: false,
        default: || Value::Bool(true),
        get: |app| Value::Bool(!look(app).start_screen_hidden),
        set: |app, value| {
            if let Value::Bool(on) = value {
                set_look(app, |look| look.start_screen_hidden = !on);
            }
            Ok(())
        },
    },
    own_toggle!(
        "general.autosave",
        General,
        "Recovery",
        "Keep a recovery copy as you work",
        "A copy of unsaved work, made in the background, that Tequila offers back after a crash. It never writes over your project file. Off, each change is still written down, and recovering a long session takes longer.",
        autosave,
        |_, _| {}
    ),
    own_number!(
        "general.autosave_minutes",
        General,
        "Recovery",
        "Every",
        "How often the recovery copy is made while there are unsaved changes.",
        autosave_minutes as f32,
        1.0..=60.0,
        0,
        " min",
        false
    ),
    Setting {
        key: "general.recent_projects",
        page: Page::General,
        group: "Recent projects",
        label: "Projects listed",
        help: "How many File › Open recent and the start screen list. 0 lists none.",
        kind: Kind::Number { min: 0.0, max: 30.0, decimals: 0, suffix: "", logarithmic: false },
        own: true,
        default: || Value::Number(Prefs::default().recent_projects as f64),
        get: |app| Value::Number(app.prefs.recent_projects as f64),
        set: |app, value| {
            if let Value::Number(n) = value {
                app.prefs.recent_projects = n as usize;
                if app.recent.len() > app.prefs.recent_projects {
                    app.recent.truncate(app.prefs.recent_projects);
                    if let Some(file) = &app.recent_file {
                        crate::recent::write(file, &app.recent);
                    }
                }
            }
            Ok(())
        },
    },
    // ---- Interface -------------------------------------------------------
    Setting {
        key: "interface.mode",
        page: Page::Interface,
        group: "Look",
        label: "Light or dark",
        help: "Auto follows the desktop's own setting.",
        kind: Kind::Choice(MODES),
        own: false,
        default: || Value::Text(crate::theme::Mode::default().key().to_string()),
        get: |app| Value::Text(look(app).mode.key().to_string()),
        set: |app, value| {
            let key = text(value);
            if let Some(mode) = crate::theme::Mode::ALL.into_iter().find(|m| m.key() == key) {
                set_look(app, |look| look.mode = mode);
            }
            Ok(())
        },
    },
    Setting {
        key: "interface.theme",
        page: Page::Interface,
        group: "Look",
        label: "Colour theme",
        help: "The accent colour, and the faintest tint of it in the greys. Graphite is neutral: the truest surround for judging colour.",
        kind: Kind::Choice(THEMES),
        own: false,
        default: || Value::Text(crate::theme::Flavor::default().key().to_string()),
        get: |app| Value::Text(look(app).flavor.key().to_string()),
        set: |app, value| {
            let key = text(value);
            if let Some(flavor) = crate::theme::Flavor::ALL.into_iter().find(|f| f.key() == key) {
                set_look(app, |look| look.flavor = flavor);
            }
            Ok(())
        },
    },
    Setting {
        key: "interface.size",
        page: Page::Interface,
        group: "Look",
        label: "Interface size",
        help: "How large the whole interface is drawn: text, buttons and panels together.",
        kind: Kind::Choice(SIZES),
        own: false,
        default: || Value::Text(crate::theme::Size::default().key().to_string()),
        get: |app| Value::Text(look(app).size.key().to_string()),
        set: |app, value| {
            let key = text(value);
            if let Some(size) = crate::theme::Size::ALL.into_iter().find(|s| s.key() == key) {
                set_look(app, |look| look.size = size);
            }
            Ok(())
        },
    },
    own_toggle!(
        "interface.tooltips",
        Interface,
        "Tooltips",
        "Show tooltips",
        "The explanations that appear when the pointer rests on a control. They are where each setting and tool says what it does.",
        tooltips,
        |_, _| {}
    ),
    own_number!(
        "interface.tooltip_delay",
        Interface,
        "Tooltips",
        "Appear after",
        "How long the pointer rests on a control before its tooltip appears.",
        tooltip_delay as f32,
        0.0..=3.0,
        1,
        " s",
        false
    ),
    Setting {
        key: "interface.library_view",
        page: Page::Interface,
        group: "Material library",
        label: "Show materials as",
        help: "Thumbnails are pictures with their names; Icons are small pictures, many across; List is a line each, the most names on screen; Cards say what each material is and whose. The same choice as the four small buttons above the library's shelf.",
        kind: Kind::Choice(LIBRARY_VIEWS),
        own: true,
        default: || Value::Text(crate::library_tab::View::default().key().to_string()),
        get: |app| Value::Text(app.prefs.library_view.key().to_string()),
        set: |app, value| {
            if let Some(view) = crate::library_tab::View::from_key(&text(value)) {
                app.prefs.library_view = view;
                app.library.view = view;
            }
            Ok(())
        },
    },
    own_toggle!(
        "interface.graph_browser",
        Interface,
        "Material Graph editor",
        "Show the browser",
        "The list of materials, maps and the scene's materials down the left of the Material Graph editor, as 3ds Max's Slate editor has: a row is dragged into the graph, or double-clicked. The editor's Browser button and T show and hide it too.",
        graph_browser,
        |app, on| app.graph_editor.show_browser = on
    ),
    // ---- Viewport --------------------------------------------------------
    Setting {
        key: "viewport.shading",
        page: Page::Viewport,
        group: STARTS,
        label: "Shading",
        help: "How the view draws when Tequila starts. Solid is the quickest; Rendered is the final picture, and takes the graphics card. Z steps through them at any time.",
        kind: Kind::Choice(SHADINGS),
        own: true,
        default: || Value::Text(Prefs::default().shading.key().to_string()),
        get: |app| Value::Text(app.prefs.shading.key().to_string()),
        set: |app, value| {
            if let Some(mode) = ViewMode::from_key(&text(value)) {
                app.prefs.shading = mode;
                app.set_view_mode(mode);
            }
            Ok(())
        },
    },
    own_toggle!("viewport.grid", Viewport, STARTS, "Ground grid", "A measuring grid on the floor. It never appears in renders.", show_grid, |app, on| app
        .camera_tab
        .show_grid = on),
    own_toggle!("viewport.axis_legend", Viewport, STARTS, "Axis legend", "The small X, Y and Z arrows in the view's corner.", show_axis_legend, |app, on| app
        .show_axis_legend = on),
    own_toggle!(
        "viewport.render_frame",
        Viewport,
        STARTS,
        "Render frame",
        "The view at the render's size and shape, so what you see is what renders.",
        show_render_frame,
        |app, on| app.show_render_frame = on
    ),
    own_toggle!(
        "viewport.statistics",
        Viewport,
        STARTS,
        "Render statistics",
        "Samples, engine, frame rate and memory, over the view. H shows and hides them at any time.",
        show_statistics,
        |app, on| app.show_hud = on
    ),
    own_toggle!(
        "viewport.model_edges",
        Viewport,
        STARTS,
        "Model edges",
        "The parts' edges drawn in Solid and Material Preview.",
        show_model_edges,
        |app, on| app.show_model_edges = on
    ),
    own_toggle!(
        "viewport.selection_outline",
        Viewport,
        "Selection",
        "Outline what is selected",
        "The coloured outline round selected parts. Shift+H hides and shows it for the session; this is how each session starts.",
        show_outline,
        |app, on| app.set_selection_outline(on)
    ),
    own_toggle!(
        "viewport.gizmo",
        Viewport,
        "Selection",
        "Show the gizmo on the selection",
        "The move, turn and scale handles. Shift+G hides and shows them for the session; this is how each session starts.",
        show_gizmo,
        |app, on| app.show_gizmo = on
    ),
    own_number!(
        "viewport.gizmo_size",
        Viewport,
        "Selection",
        "Gizmo size",
        "How large the gizmo's handles are on screen. They keep that size however far away the part is.",
        gizmo_size as f32,
        50.0..=250.0,
        0,
        "%",
        false
    ),
    Setting {
        key: "viewport.gizmo_mode",
        page: Page::Viewport,
        group: "Selection",
        label: "Gizmo starts as",
        help: "All at once has arrows, rings and scale handles together (Q). W, E and R pick move, rotate or scale alone at any time.",
        kind: Kind::Choice(GIZMO_MODES),
        own: true,
        default: || Value::Text(gizmo_key(Prefs::default().gizmo_mode).to_string()),
        get: |app| Value::Text(gizmo_key(app.prefs.gizmo_mode).to_string()),
        set: |app, value| {
            let key = text(value);
            if let Some(mode) = GizmoMode::ALL.into_iter().find(|m| gizmo_key(*m) == key) {
                app.prefs.gizmo_mode = mode;
                app.gizmo.mode = mode;
            }
            Ok(())
        },
    },
    Setting {
        key: "viewport.moving_picture",
        page: Page::Viewport,
        group: "Rendered view",
        label: "While the view moves",
        help: "The Rendered view draws a smaller picture while you orbit, and its full size once you stop. Smooth gives more pictures a second and a coarser picture; Sharp a finer picture and fewer of them.",
        kind: Kind::Choice(MOVING),
        own: true,
        default: || Value::Text(MovingPicture::default().key().to_string()),
        get: |app| Value::Text(app.prefs.moving_picture.key().to_string()),
        set: |app, value| {
            let key = text(value);
            if let Some(choice) = MovingPicture::ALL.into_iter().find(|m| m.key() == key) {
                app.prefs.moving_picture = choice;
            }
            Ok(())
        },
    },
    own_number!(
        "viewport.samples",
        Viewport,
        "Rendered view",
        "Samples at most",
        "How far a still Rendered view on Cycles refines before it rests. More is cleaner and keeps the graphics card busy longer; most views are clean well before the limit.",
        view_samples as u32,
        16.0..=16384.0,
        0,
        "",
        true
    ),
    // ---- Navigation ------------------------------------------------------
    Setting {
        key: "navigation.preset",
        page: Page::Navigation,
        group: BUTTONS,
        label: "Move the view as in",
        help: "Which mouse buttons orbit, pan and zoom: Tequila's own, which are KeyShot's, or those of the program your hands know. Changing a drag below makes it Custom. The wheel zooms in all of them.",
        kind: Kind::Choice(PRESETS),
        own: true,
        default: || Value::Text(crate::navigation::Preset::default().key().to_string()),
        get: |app| Value::Text(app.prefs.nav_preset.key().to_string()),
        set: |app, value| {
            if let Some(preset) = crate::navigation::Preset::from_key(&text(value)) {
                // Custom starts from the drags in use.
                if preset == crate::navigation::Preset::Custom && app.prefs.nav_preset != preset {
                    app.prefs.nav_custom = app.prefs.navigation();
                }
                app.prefs.nav_preset = preset;
            }
            Ok(())
        },
    },
    own_drag!("navigation.orbit_drag", "Orbit", "The drag that turns the view about what it looks at.", Orbit),
    own_drag!("navigation.pan_drag", "Pan", "The drag that slides the view.", Pan),
    own_drag!("navigation.zoom_drag", "Zoom", "The drag that moves the view closer and further, up and down. The wheel always zooms too.", Zoom),
    own_number!(
        "navigation.orbit_speed",
        Navigation,
        "Mouse and trackpad",
        "Orbit speed",
        "How far the view turns for a drag. At 1, a drag the height of the view turns it half way round.",
        orbit_speed as f32,
        0.25..=4.0,
        2,
        "×",
        true
    ),
    own_number!(
        "navigation.pan_speed",
        Navigation,
        "Mouse and trackpad",
        "Pan speed",
        "How far the view slides for a drag. At 1, what is under the pointer stays under it.",
        pan_speed as f32,
        0.25..=4.0,
        2,
        "×",
        true
    ),
    own_number!(
        "navigation.zoom_speed",
        Navigation,
        "Mouse and trackpad",
        "Zoom speed",
        "How much a notch of the wheel, a pinch or a dolly changes the distance.",
        zoom_speed as f32,
        0.25..=4.0,
        2,
        "×",
        true
    ),
    own_toggle!(
        "navigation.invert_zoom",
        Navigation,
        "Mouse and trackpad",
        "Reverse the zoom direction",
        "Rolling the wheel away from you zooms out instead of in, as some CAD programs have it.",
        invert_zoom,
        |_, _| {}
    ),
    Setting {
        key: "navigation.trackpad",
        page: Page::Navigation,
        group: "Mouse and trackpad",
        label: "A two-finger swipe",
        help: "On a Mac's trackpad or a Magic Mouse. Whichever it does, Shift and a swipe does the other: pan or orbit.",
        kind: Kind::Choice(TRACKPAD),
        own: true,
        default: || Value::Text("pan".to_string()),
        get: |app| Value::Text(if app.prefs.trackpad_orbits { "orbit" } else { "pan" }.to_string()),
        set: |app, value| {
            app.prefs.trackpad_orbits = text(value) == "orbit";
            Ok(())
        },
    },
    // ---- Import ----------------------------------------------------------
    Setting {
        key: "import.up_axis",
        page: Page::Import,
        group: "Models",
        label: "Up axis of imports",
        help: "Which way up your files are drawn. Z up is for a CAD model drawn Z up, which otherwise comes in lying on its back.",
        kind: Kind::Choice(UP_AXES),
        own: false,
        default: || Value::Text(tequila_import::UpAxis::default().key().to_string()),
        get: |_| Value::Text(crate::import_settings::up_axis().key().to_string()),
        set: |_, value| match tequila_import::UpAxis::from_key(&text(value)) {
            Some(up) => crate::import_settings::set_up_axis(up),
            None => Ok(()),
        },
    },
    Setting {
        key: "import.blender_lights",
        page: Page::Import,
        group: "Models",
        label: "Lights in Blender files",
        help: "What Tequila does with a Blender file's own lights, world and exposure: ask each time, light the model as Blender does, or keep the lighting you have.",
        kind: Kind::Choice(BLENDER_LIGHTS),
        own: false,
        default: || Value::Text("ask".to_string()),
        get: |_| Value::Text(crate::import_settings::blender_lights().key().to_string()),
        set: |_, value| match crate::import_settings::BlenderLights::from_key(&text(value)) {
            Some(choice) => crate::import_settings::set_blender_lights(choice),
            None => Ok(()),
        },
    },
    // ---- Folders ---------------------------------------------------------
    Setting {
        key: "folders.screenshots",
        page: Page::Folders,
        group: "Where things go",
        label: "Screenshots",
        help: "Where P's Screenshot of the viewport is saved.",
        kind: Kind::Folder,
        own: false,
        default: || Value::Text(crate::screenshot::default_folder().display().to_string()),
        get: |app| Value::Text(app.screenshots.folder().display().to_string()),
        set: |app, value| {
            let folder = text(value);
            let default = crate::screenshot::default_folder();
            app.screenshots.folder = (!folder.is_empty() && PathBuf::from(&folder) != default).then(|| PathBuf::from(folder));
            app.screenshots.save()
        },
    },
    // ---- Updates ---------------------------------------------------------
    Setting {
        key: "updates.automatic",
        page: Page::Updates,
        group: "New versions",
        label: "Look for a new version when Tequila starts",
        help: "A quiet look at GitHub's releases, at most once a day. Nothing is downloaded or installed without you; Help › Check for updates looks at any time.",
        kind: Kind::Toggle,
        own: false,
        default: || Value::Bool(true),
        get: |app| Value::Bool(app.updates.preferences.automatic),
        set: |app, value| {
            if let Value::Bool(on) = value {
                app.updates.preferences.automatic = on;
                app.updates.preferences.save();
            }
            Ok(())
        },
    },
];

fn gizmo_key(mode: GizmoMode) -> &'static str {
    match mode {
        GizmoMode::Universal => "all",
        GizmoMode::Move => "move",
        GizmoMode::Rotate => "rotate",
        GizmoMode::Scale => "scale",
    }
}

/// The setting called `key`.
pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.key.eq_ignore_ascii_case(key.trim()))
}

/// A part of a page that is not a row of the table: drawn by hand, and
/// found by the search through its words.
struct Extra {
    page: Page,
    title: &'static str,
    words: &'static str,
}

const MOUSE_3D: &str = "3D mouse";
const OWN_FOLDERS: &str = "Tequila's folders";
const EVERYTHING: &str = "All settings";
const RECENT: &str = "Recent projects";
const KEYS: &str = "Keyboard shortcuts";
/// The prefix of a shortcut's line in the settings file and a script.
pub const SHORTCUT_PREFIX: &str = "shortcut.";

const EXTRAS: &[Extra] = &[
    Extra { page: Page::Keyboard, title: KEYS, words: "keyboard shortcuts keys hotkeys bindings" },
    Extra { page: Page::General, title: RECENT, words: "recent projects clear list forget open recent" },
    Extra { page: Page::Navigation, title: MOUSE_3D, words: "3d mouse spacemouse space mouse 3dconnexion speed dead zone axes invert buttons navigation" },
    Extra { page: Page::Folders, title: OWN_FOLDERS, words: "tequila's folders settings library materials open folder storage" },
    Extra { page: Page::Advanced, title: EVERYTHING, words: "all settings export import file backup reset everything defaults another computer team" },
];

/// The prefix of the 3D mouse's lines in an exported file.
const MOUSE_PREFIX: &str = "3d_mouse.";

/// The window.
pub struct Window {
    pub open: bool,
    pub page: Page,
    pub search: String,
    /// The search field takes the keys when the window opens.
    focus: bool,
    /// The shortcut whose key is being chosen: the action and which of
    /// its places. The next key pressed is it.
    choosing: Option<(Action, usize)>,
    /// A key chosen that another action has, waiting to be told which
    /// keeps it.
    clash: Option<Clash>,
    /// Frames for which keys passed on from the window are dropped: the
    /// press that chose Ctrl+Z is not also an undo.
    swallow: u8,
}

/// A key chosen for one action that another has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clash {
    action: Action,
    slot: usize,
    chord: Chord,
    other: Action,
}

impl Default for Window {
    fn default() -> Window {
        Window { open: false, page: Page::General, search: String::new(), focus: false, choosing: None, clash: None, swallow: 0 }
    }
}

impl Window {
    pub fn show(&mut self) {
        if !self.open {
            self.focus = true;
        }
        self.open = true;
    }

    /// Whether a key is being chosen in Preferences › Keyboard: a key
    /// pressed now is the choice, not a command.
    pub fn choosing_key(&self) -> bool {
        self.choosing.is_some()
    }

    /// Whether keys passed on from the window this frame are dropped,
    /// counting the frame.
    pub fn swallows_keys(&mut self) -> bool {
        if self.choosing.is_some() {
            return true;
        }
        let swallowing = self.swallow > 0;
        self.swallow = self.swallow.saturating_sub(1);
        swallowing
    }
}

/// What a click in the window asks for, done once the window is drawn.
enum Act {
    /// Start choosing a shortcut's key, or stop.
    Choose(Option<(Action, usize)>),
    /// A key for a shortcut's place, or none.
    Key(Action, usize, Option<Chord>),
    /// The clash settled: the key goes to the action that asked.
    TakeKey,
    LeaveKey,
    ResetKey(Action),
    Set(usize, Value),
    Reset(usize),
    ResetPage(Page),
    ResetAll,
    ChooseFolder(usize),
    Open(PathBuf),
    ClearRecent,
    Export,
    Import,
}

impl TequilaApp {
    /// Set the setting called `key`, checked, acted on at once and kept.
    pub(crate) fn set_preference(&mut self, key: &str, value: Value) -> Result<(), String> {
        let setting = setting(key).ok_or_else(|| format!("there is no setting `{key}`"))?;
        let value = setting.checked(value)?;
        (setting.set)(self, value)?;
        if setting.own {
            self.keep_preferences();
        }
        Ok(())
    }

    /// Put settings back to their defaults: a page's, or all of them and
    /// the 3D mouse's.
    pub(crate) fn reset_preferences(&mut self, page: Option<Page>) -> Result<(), String> {
        for setting in SETTINGS.iter().filter(|s| page.is_none_or(|p| s.page == p)) {
            if !(setting.get)(self).same(&(setting.default)()) {
                (setting.set)(self, (setting.default)())?;
            }
        }
        if page.is_none_or(|p| p == Page::Keyboard) {
            self.shortcuts = Shortcuts::default();
            self.preferences.clash = None;
        }
        if page.is_none_or(|p| p == Page::Navigation) {
            let keep = self.prefs_file.is_some();
            self.spacemouse.set_settings(crate::spacemouse::navigate::Settings::default(), keep);
        }
        self.keep_preferences();
        Ok(())
    }

    /// Tequila's own choices as the file holds them: a line each.
    fn own_preferences_text(&self) -> String {
        let mut out: String = SETTINGS.iter().filter(|s| s.own).map(|s| format!("{}={}\n", s.key, (s.get)(self).to_text())).collect();
        // The shortcuts that are not their defaults.
        for (name, keys) in self.shortcuts.changed_lines() {
            out.push_str(&format!("{SHORTCUT_PREFIX}{name}={keys}\n"));
        }
        out
    }

    /// Write Tequila's own choices to their file, where there is one: an
    /// application made for a test has none, and keeps nothing.
    fn keep_preferences(&mut self) {
        let Some(file) = self.prefs_file.clone() else { return };
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        if let Err(error) = std::fs::write(&file, self.own_preferences_text()) {
            self.toasts.warn(format!("The settings could not be kept: {error}"));
        }
    }

    /// Read Tequila's own choices from `file` and act on them: the session
    /// starts as they say. A line that is not understood keeps its default.
    pub(crate) fn load_preferences(&mut self, file: PathBuf) {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        self.read_preferences(&text, true);
        // A file that held nothing still starts the session as the
        // defaults say: the gizmo's mode, what the view shows.
        for setting in SETTINGS.iter().filter(|s| s.own) {
            let _ = (setting.set)(self, (setting.get)(self));
        }
        self.prefs_file = Some(file);
    }

    /// Take every line of `text` that names a setting: `own_only` for
    /// Tequila's own file, where another's line has no business. How many
    /// were taken, and what was wrong with the rest.
    fn read_preferences(&mut self, text: &str, own_only: bool) -> (usize, Vec<String>) {
        let (mut taken, mut wrong) = (0, Vec::new());
        let mut mouse = String::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let Some((key, value)) = line.split_once('=') else {
                wrong.push(format!("`{line}` is not a setting"));
                continue;
            };
            if let Some(rest) = key.trim().strip_prefix(MOUSE_PREFIX) {
                mouse.push_str(&format!("{rest}={value}\n"));
                continue;
            }
            if let Some(name) = key.trim().strip_prefix(SHORTCUT_PREFIX) {
                match self.shortcuts.read_line(name, value) {
                    Ok(()) => taken += 1,
                    Err(error) => wrong.push(error),
                }
                continue;
            }
            match setting(key).filter(|s| s.own || !own_only) {
                Some(setting) => match setting.parse(value).and_then(|value| (setting.set)(self, value)) {
                    Ok(()) => taken += 1,
                    Err(error) => wrong.push(error),
                },
                None => wrong.push(format!("there is no setting `{}`", key.trim())),
            }
        }
        if !mouse.is_empty() && !own_only {
            let keep = self.prefs_file.is_some();
            self.spacemouse.set_settings(crate::spacemouse::settings_from_text(&mouse), keep);
            taken += 1;
        }
        (taken, wrong)
    }

    /// Every setting as one file: to move to another computer, or to give
    /// a team one standard.
    pub(crate) fn exported_preferences(&self) -> String {
        let mut out = String::from("# Tequila's settings. Edit › Preferences › Advanced reads this file back.\n");
        for setting in SETTINGS {
            out.push_str(&format!("{}={}\n", setting.key, (setting.get)(self).to_text()));
        }
        for line in crate::spacemouse::settings_to_text(&self.spacemouse.settings).lines() {
            out.push_str(&format!("{MOUSE_PREFIX}{line}\n"));
        }
        for (name, keys) in self.shortcuts.all_lines() {
            out.push_str(&format!("{SHORTCUT_PREFIX}{name}={keys}\n"));
        }
        out
    }

    /// Take every setting in an exported file. How many were taken, and
    /// what was wrong with the rest.
    pub(crate) fn import_preferences(&mut self, text: &str) -> (usize, Vec<String>) {
        let result = self.read_preferences(text, false);
        self.keep_preferences();
        result
    }

    /// What the settings ask of the session at every frame: cheap, and
    /// right whichever way a setting was changed.
    pub(crate) fn apply_preferences(&mut self, ctx: &egui::Context) {
        // The library's own buttons choose its view too: kept as chosen.
        if self.library.view != self.prefs.library_view {
            self.prefs.library_view = self.library.view;
            self.keep_preferences();
        }
        // So do the graph editor's Browser button and T.
        if self.graph_editor.show_browser != self.prefs.graph_browser {
            self.prefs.graph_browser = self.graph_editor.show_browser;
            self.keep_preferences();
        }
        self.gizmo.scale = (self.prefs.gizmo_size / 100.0).clamp(0.25, 4.0);
        self.project.set_autosave_every(self.prefs.autosave_every());
        crate::cycles_view::set_tuning(self.prefs.moving_picture.target_ms(), self.prefs.view_samples);
        // No tooltips is a wait no pointer outlasts.
        let delay = if self.prefs.tooltips { self.prefs.tooltip_delay.clamp(0.0, 10.0) } else { 1.0e9 };
        if ctx.style().interaction.tooltip_delay != delay {
            ctx.style_mut(|style| style.interaction.tooltip_delay = delay);
        }
    }

    /// What a setting's row is marked against: its default, but for the
    /// three drags under a preset, which are the preset's and not a change
    /// of the person's. Marked, Blender's drags each offered to go "back"
    /// to Tequila's own, one at a time.
    fn shown_default(&self, setting: &Setting) -> Value {
        let preset = self.prefs.nav_preset != crate::navigation::Preset::Custom;
        if preset && setting.key.ends_with("_drag") && setting.page == Page::Navigation {
            (setting.get)(self)
        } else {
            (setting.default)()
        }
    }

    /// Edit › Preferences: the window.
    pub(crate) fn preferences_window(&mut self, ctx: &egui::Context) {
        if !self.preferences.open {
            return;
        }
        let mut open = true;
        let values: Vec<Value> = SETTINGS.iter().map(|s| (s.get)(self)).collect();
        let defaults: Vec<Value> = SETTINGS.iter().map(|s| self.shown_default(s)).collect();
        let recent = self.recent.len();
        let own_folders = [("Settings", tequila_format::library::settings_directory()), ("Material library", crate::library_tab::user_library_path())];
        let mut mouse = std::mem::take(&mut self.spacemouse);
        let mut page = self.preferences.page;
        let mut search = std::mem::take(&mut self.preferences.search);
        let focus = std::mem::take(&mut self.preferences.focus);
        let shortcuts = self.shortcuts.clone();
        let choosing = self.preferences.choosing;
        let clash = self.preferences.clash;
        let mut acts: Vec<Act> = Vec::new();

        let spec = crate::windows::Spec {
            kind: crate::windows::Kind::Preferences,
            title: "Preferences".into(),
            size: egui::vec2(760.0, 600.0),
            min_size: egui::vec2(620.0, 380.0),
            resizable: true,
            margin: 0.0,
        };
        crate::windows::show(ctx, spec, &mut open, |window| window.resizable(false), |ui| {
            let p = crate::theme::p();
            // A key being chosen: the next one pressed in this window, with
            // what was held. Nothing else has the keyboard meanwhile.
            if let Some((action, slot)) = choosing {
                ui.memory_mut(|memory| {
                    if let Some(focused) = memory.focused() {
                        memory.surrender_focus(focused);
                    }
                });
                let pressed = ui.input(|input| {
                    input.events.iter().find_map(|event| match event {
                        egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => Some(Chord::pressed(*key, modifiers)),
                        _ => None,
                    })
                });
                if let Some(chord) = pressed {
                    acts.push(Act::Key(action, slot, Some(chord)));
                }
            }
            egui::SidePanel::left("preferences-pages")
                .resizable(false)
                .exact_width(190.0)
                .frame(egui::Frame::none().fill(p.card).inner_margin(egui::Margin::same(12.0)))
                .show_inside(ui, |ui| {
                    let field = crate::theme::search_field(ui, &mut search, "Search settings");
                    if focus {
                        field.request_focus();
                    }
                    ui.add_space(8.0);
                    for candidate in Page::ALL {
                        let chosen = search.trim().is_empty() && candidate == page;
                        if page_button(ui, candidate, chosen).clicked() {
                            page = candidate;
                            search.clear();
                        }
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        ui.add(egui::Label::new(egui::RichText::new("Changes apply at once.").small().color(crate::theme::dim())).wrap());
                    });
                });
            egui::CentralPanel::default().frame(egui::Frame::none().fill(p.panel).inner_margin(egui::Margin::same(18.0))).show_inside(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let words: Vec<String> = search.to_lowercase().split_whitespace().map(str::to_string).collect();
                    let mut blocks = Blocks { mouse: &mut mouse, ctx, recent, own_folders: &own_folders, shortcuts: &shortcuts, choosing, clash, words: &words, acts: &mut acts };
                    if words.is_empty() {
                        heading(ui, page.label(), page.blurb());
                        let rows: Vec<usize> = (0..SETTINGS.len()).filter(|&i| SETTINGS[i].page == page).collect();
                        let extras: Vec<&Extra> = EXTRAS.iter().filter(|e| e.page == page).collect();
                        page_body(ui, &rows, &extras, &values, &defaults, &mut blocks);
                        if rows.iter().any(|&i| !values[i].same(&defaults[i])) {
                            ui.add_space(14.0);
                            if ui.button("Restore this page's defaults").clicked() {
                                blocks.acts.push(Act::ResetPage(page));
                            }
                        }
                    } else {
                        let mut found = false;
                        for candidate in Page::ALL {
                            let rows: Vec<usize> = (0..SETTINGS.len()).filter(|&i| SETTINGS[i].page == candidate && SETTINGS[i].matches(&words)).collect();
                            let extras: Vec<&Extra> = EXTRAS
                                .iter()
                                .filter(|e| {
                                    e.page == candidate
                                        && (words.iter().all(|w| e.words.contains(w.as_str()) || e.title.to_lowercase().contains(w.as_str()))
                                            || (e.title == KEYS && crate::shortcuts::ACTIONS.iter().any(|info| shortcut_matches(info, &shortcuts, &words))))
                                })
                                .collect();
                            if rows.is_empty() && extras.is_empty() {
                                continue;
                            }
                            if found {
                                ui.add_space(14.0);
                            }
                            found = true;
                            heading(ui, candidate.label(), "");
                            page_body(ui, &rows, &extras, &values, &defaults, &mut blocks);
                        }
                        if !found {
                            crate::theme::empty_state(ui, crate::icons::Icon::SearchX, crate::theme::dim(), "No setting found", "Try another word, or look through the pages.");
                        }
                    }
                });
            });
        });

        self.spacemouse = mouse;
        self.preferences.page = page;
        self.preferences.search = search;
        self.preferences.open = open;
        for act in acts {
            self.preferences_act(act);
        }
    }

    fn preferences_act(&mut self, act: Act) {
        let said = match act {
            Act::Choose(which) => {
                self.preferences.choosing = which;
                self.preferences.clash = None;
                None
            }
            Act::Key(action, slot, chord) => {
                self.preferences.choosing = None;
                self.preferences.swallow = 3;
                match chord.and_then(|chord| self.shortcuts.owner(chord, action).map(|other| (chord, other))) {
                    // Another action's key: asked about, not taken.
                    Some((chord, other)) => self.preferences.clash = Some(Clash { action, slot, chord, other }),
                    None => {
                        self.shortcuts.set(action, slot, chord);
                        self.keep_preferences();
                    }
                }
                None
            }
            Act::TakeKey => {
                if let Some(clash) = self.preferences.clash.take() {
                    self.shortcuts.set(clash.action, clash.slot, Some(clash.chord));
                    self.keep_preferences();
                }
                None
            }
            Act::LeaveKey => {
                self.preferences.clash = None;
                None
            }
            Act::ResetKey(action) => {
                self.shortcuts.reset(action);
                self.preferences.clash = None;
                self.keep_preferences();
                None
            }
            Act::Set(index, value) => self.set_preference(SETTINGS[index].key, value).err(),
            Act::Reset(index) => self.set_preference(SETTINGS[index].key, (SETTINGS[index].default)()).err(),
            Act::ResetPage(page) => self.reset_preferences(Some(page)).err(),
            Act::ResetAll => {
                let sure = rfd::MessageDialog::new()
                    .set_title("Reset every setting?")
                    .set_description("Every setting in Preferences goes back to its default. Your projects, materials and plugins are not touched.")
                    .set_buttons(rfd::MessageButtons::OkCancel)
                    .show();
                match sure {
                    rfd::MessageDialogResult::Ok => match self.reset_preferences(None) {
                        Ok(()) => {
                            self.toasts.info("Every setting is back to its default");
                            None
                        }
                        Err(error) => Some(error),
                    },
                    _ => None,
                }
            }
            Act::ChooseFolder(index) => {
                let setting = &SETTINGS[index];
                let start = PathBuf::from(text((setting.get)(self)));
                let mut dialog = rfd::FileDialog::new().set_title(setting.label);
                if start.is_dir() {
                    dialog = dialog.set_directory(&start);
                }
                match dialog.pick_folder() {
                    Some(folder) => self.set_preference(setting.key, Value::Text(folder.display().to_string())).err(),
                    None => None,
                }
            }
            Act::Open(folder) => {
                crate::library_tab::open_folder(&folder);
                None
            }
            Act::ClearRecent => {
                self.recent.clear();
                if let Some(file) = &self.recent_file {
                    crate::recent::write(file, &self.recent);
                }
                self.toasts.info("The list of recent projects is empty");
                None
            }
            Act::Export => {
                match rfd::FileDialog::new().set_title("Export settings").set_file_name("Tequila settings.txt").add_filter("Tequila settings", &["txt"]).save_file() {
                    Some(path) => match std::fs::write(&path, self.exported_preferences()) {
                        Ok(()) => {
                            self.toasts.info(format!("Settings exported to {}", path.display()));
                            None
                        }
                        Err(error) => Some(format!("The settings could not be exported: {error}")),
                    },
                    None => None,
                }
            }
            Act::Import => match rfd::FileDialog::new().set_title("Import settings").add_filter("Tequila settings", &["txt"]).pick_file() {
                Some(path) => match std::fs::read_to_string(&path) {
                    Ok(text) => {
                        let (taken, wrong) = self.import_preferences(&text);
                        if wrong.is_empty() {
                            self.toasts.info(format!("{taken} settings imported"));
                            None
                        } else {
                            Some(format!("{taken} settings imported; {} were not understood: {}", wrong.len(), wrong.join("; ")))
                        }
                    }
                    Err(error) => Some(format!("The file could not be read: {error}")),
                },
                None => None,
            },
        };
        if let Some(problem) = said {
            self.toasts.warn(problem);
        }
    }
}

/// What the hand-drawn parts of a page need.
struct Blocks<'a> {
    mouse: &'a mut crate::spacemouse::SpaceMouse,
    ctx: &'a egui::Context,
    recent: usize,
    own_folders: &'a [(&'static str, PathBuf)],
    shortcuts: &'a Shortcuts,
    choosing: Option<(Action, usize)>,
    clash: Option<Clash>,
    /// What is searched for, each word in lower case; none on a page.
    words: &'a [String],
    acts: &'a mut Vec<Act>,
}

/// A page in the list down the side.
fn page_button(ui: &mut egui::Ui, page: Page, chosen: bool) -> egui::Response {
    let p = crate::theme::p();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    if chosen || response.hovered() {
        ui.painter().rect_filled(rect, 6.0, if chosen { p.pressed } else { p.hover });
    }
    let colour = if chosen { p.accent_text } else { p.text };
    let glyph = egui::Rect::from_center_size(egui::pos2(rect.left() + 17.0, rect.center().y), egui::Vec2::splat(15.0));
    crate::icons::paint(ui.painter(), glyph, page.icon(), colour);
    ui.painter().text(egui::pos2(rect.left() + 34.0, rect.center().y), egui::Align2::LEFT_CENTER, page.label(), egui::FontId::proportional(13.0), colour);
    response
}

fn heading(ui: &mut egui::Ui, title: &str, blurb: &str) {
    ui.label(egui::RichText::new(title).font(crate::theme::strong(17.0)));
    if !blurb.is_empty() {
        ui.label(egui::RichText::new(blurb).small().color(crate::theme::dim()));
    }
}

/// A page's rows under their headings, then its hand-drawn parts.
fn page_body(ui: &mut egui::Ui, rows: &[usize], extras: &[&Extra], values: &[Value], defaults: &[Value], blocks: &mut Blocks) {
    let mut group = "";
    for &index in rows {
        let setting = &SETTINGS[index];
        if setting.group != group {
            group = setting.group;
            crate::theme::section(ui, group);
        }
        row(ui, index, &values[index], &defaults[index], blocks.acts);
    }
    for extra in extras {
        // The recent projects' button goes under their own heading, which
        // the row above has already drawn.
        if extra.title != group && extra.title != KEYS {
            crate::theme::section(ui, extra.title);
        }
        match extra.title {
            RECENT => {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(GUTTER);
                    let label = if blocks.recent == 0 { "The list is empty".to_string() } else { format!("Clear the list ({})", blocks.recent) };
                    if ui.add_enabled(blocks.recent > 0, egui::Button::new(label)).on_hover_text("Forget the projects File › Open recent and the start screen list. The projects themselves are not touched.").clicked() {
                        blocks.acts.push(Act::ClearRecent);
                    }
                });
            }
            MOUSE_3D => inset(ui, |ui| blocks.mouse.settings_ui(ui, blocks.ctx)),
            KEYS => shortcut_rows(ui, blocks),
            OWN_FOLDERS => {
                for (name, folder) in blocks.own_folders {
                    ui.horizontal(|ui| {
                        ui.add_space(GUTTER);
                        label(ui, name, "");
                        if ui.button("Open").on_hover_text(folder.display().to_string()).clicked() {
                            blocks.acts.push(Act::Open(folder.clone()));
                        }
                        path_label(ui, &folder.display().to_string());
                    });
                }
            }
            EVERYTHING => inset(ui, |ui| {
                ui.add(egui::Label::new(egui::RichText::new("One file holds every setting here, the 3D mouse's included: to move them to another computer, or to give a team one standard.").small().color(crate::theme::dim())).wrap());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Export…").on_hover_text("Save every setting to a file").clicked() {
                        blocks.acts.push(Act::Export);
                    }
                    if ui.button("Import…").on_hover_text("Take every setting in a file exported from here. Settings the file does not name stay as they are.").clicked() {
                        blocks.acts.push(Act::Import);
                    }
                });
                ui.add_space(10.0);
                if ui.button("Reset every setting…").on_hover_text("Every setting back to its default. It asks first.").clicked() {
                    blocks.acts.push(Act::ResetAll);
                }
            }),
            _ => {}
        }
    }
}

/// Whether every word searched for is in a shortcut's words or its keys.
fn shortcut_matches(info: &crate::shortcuts::Info, shortcuts: &Shortcuts, words: &[String]) -> bool {
    let hay = format!("{} {} {} {} shortcut key", info.label, info.group.label(), info.key, crate::theme::keys(&shortcuts.shown(info.action))).to_lowercase();
    words.iter().all(|word| hay.contains(word.as_str()))
}

/// Preferences › Keyboard: every action under its heading with its keys,
/// each a button that takes the next key pressed; the ones searched for,
/// when something is.
fn shortcut_rows(ui: &mut egui::Ui, blocks: &mut Blocks) {
    let p = crate::theme::p();
    let mut any_changed = false;
    for group in crate::shortcuts::Group::ALL {
        let rows: Vec<&crate::shortcuts::Info> = crate::shortcuts::ACTIONS
            .iter()
            .filter(|info| info.group == group && (blocks.words.is_empty() || shortcut_matches(info, blocks.shortcuts, blocks.words)))
            .collect();
        if rows.is_empty() {
            continue;
        }
        crate::theme::section(ui, group.label());
        for info in rows {
            let action = info.action;
            let keys = blocks.shortcuts.of(action);
            let changed = blocks.shortcuts.changed(action);
            any_changed |= changed;
            ui.horizontal(|ui| {
                let (mark, _) = ui.allocate_exact_size(egui::vec2(GUTTER - 4.0, 22.0), egui::Sense::hover());
                if changed {
                    let response = ui.interact(mark, ui.id().with(("reset-key", info.key)), egui::Sense::click());
                    crate::icons::paint(ui.painter(), mark.shrink2(egui::vec2(3.0, 4.0)), crate::icons::Icon::History, if response.hovered() { p.text } else { p.accent_text });
                    let default = Shortcuts::default().shown(action);
                    let default = if default.is_empty() { "no key".to_string() } else { crate::theme::keys(&default) };
                    if response.on_hover_text(format!("Back to the default: {default}")).clicked() {
                        blocks.acts.push(Act::ResetKey(action));
                    }
                }
                ui.allocate_ui_with_layout(egui::vec2(KEY_LABELS, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(KEY_LABELS);
                    ui.label(info.label);
                });
                for slot in 0..crate::shortcuts::SLOTS {
                    // The second place shows once the first has a key.
                    if slot > keys.len() {
                        break;
                    }
                    let choosing = blocks.choosing == Some((action, slot));
                    let text = match (choosing, keys.get(slot)) {
                        (true, _) => egui::RichText::new("Press a key…").color(p.accent_text),
                        (false, Some(chord)) => egui::RichText::new(crate::theme::keys(&chord.shown())),
                        (false, None) if slot == 0 => egui::RichText::new("No key").color(crate::theme::dim()),
                        (false, None) => egui::RichText::new("Add another").color(crate::theme::dim()),
                    };
                    let button = ui.add(egui::Button::new(text).min_size(egui::vec2(132.0, 24.0)).selected(choosing));
                    let button = button.on_hover_text(if choosing { "The next key pressed, with Ctrl, Shift or Alt held, is this shortcut's. Click again to leave it as it was." } else { "Click, then press the key for this" });
                    if button.clicked() {
                        blocks.acts.push(Act::Choose((!choosing).then_some((action, slot))));
                    }
                    if choosing && keys.get(slot).is_some() && ui.small_button("Remove").on_hover_text("Leave this place without a key").clicked() {
                        blocks.acts.push(Act::Key(action, slot, None));
                    }
                }
            });
            // The key chosen is another action's: which keeps it.
            if let Some(clash) = blocks.clash.filter(|clash| clash.action == action) {
                ui.horizontal(|ui| {
                    ui.add_space(GUTTER);
                    let key = crate::theme::keys(&clash.chord.shown());
                    ui.label(egui::RichText::new(format!("{key} is already {}.", clash.other.info().label)).color(p.busy));
                    if ui.button(format!("Use it for {}", info.label)).on_hover_text(format!("{} is left without that key", clash.other.info().label)).clicked() {
                        blocks.acts.push(Act::TakeKey);
                    }
                    if ui.button("Cancel").clicked() {
                        blocks.acts.push(Act::LeaveKey);
                    }
                });
            }
        }
    }
    if blocks.words.is_empty() {
        ui.add_space(8.0);
        inset(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new("Rotate and the environment background share a key by design: it turns the selection when there is one, and sets the background when there is not. The Material Graph editor's and the UV editor's own keys are not here.").small().color(crate::theme::dim())).wrap());
        });
        if any_changed {
            ui.add_space(10.0);
            if ui.button("Restore the default keys").clicked() {
                blocks.acts.push(Act::ResetPage(Page::Keyboard));
            }
        }
    }
}

/// The width the actions' names share on the Keyboard page.
const KEY_LABELS: f32 = 230.0;

/// Room at the left of every row for the mark that puts a changed setting
/// back.
const GUTTER: f32 = 24.0;
/// The width the labels of a page's numbers and choices share.
const LABELS: f32 = 170.0;

/// One setting: its reset mark when it is not at its default, its label
/// and its control.
fn row(ui: &mut egui::Ui, index: usize, value: &Value, default: &Value, acts: &mut Vec<Act>) {
    let setting = &SETTINGS[index];
    ui.horizontal(|ui| {
        let (mark, _) = ui.allocate_exact_size(egui::vec2(GUTTER - 4.0, 22.0), egui::Sense::hover());
        if !value.same(default) {
            let response = ui.interact(mark, ui.id().with(("reset", setting.key)), egui::Sense::click());
            let p = crate::theme::p();
            crate::icons::paint(ui.painter(), mark.shrink2(egui::vec2(3.0, 4.0)), crate::icons::Icon::History, if response.hovered() { p.text } else { p.accent_text });
            if response.on_hover_text(format!("Back to the default: {}", setting.shown(default))).clicked() {
                acts.push(Act::Reset(index));
            }
        }
        match (&setting.kind, value) {
            (Kind::Toggle, Value::Bool(on)) => {
                let mut on = *on;
                if ui.add(crate::field::Checkbox::new(&mut on, setting.label)).on_hover_text(setting.help).changed() {
                    acts.push(Act::Set(index, Value::Bool(on)));
                }
            }
            (Kind::Number { min, max, decimals, suffix, logarithmic }, Value::Number(n)) => {
                label(ui, setting.label, setting.help);
                let mut n = *n;
                let slider = crate::field::Slider::new(&mut n, *min..=*max).fixed_decimals(*decimals).suffix(*suffix).logarithmic(*logarithmic);
                if ui.add(slider).on_hover_text(setting.help).changed() {
                    acts.push(Act::Set(index, Value::Number(n)));
                }
            }
            (Kind::Choice(options), Value::Text(key)) => {
                label(ui, setting.label, setting.help);
                let mut chosen = options.iter().position(|(k, _)| k == key).unwrap_or(0);
                let before = chosen;
                if options.len() <= 4 {
                    let cells: Vec<(usize, &str)> = options.iter().enumerate().map(|(i, (_, label))| (i, *label)).collect();
                    ui.push_id(setting.key, |ui| crate::theme::segmented(ui, &mut chosen, &cells)).response.on_hover_text(setting.help);
                } else {
                    egui::ComboBox::from_id_salt(setting.key).selected_text(options[chosen].1).width(200.0).show_ui(ui, |ui| {
                        for (i, (_, label)) in options.iter().enumerate() {
                            ui.selectable_value(&mut chosen, i, *label);
                        }
                    }).response.on_hover_text(setting.help);
                }
                if chosen != before {
                    acts.push(Act::Set(index, Value::Text(options[chosen].0.to_string())));
                }
            }
            (Kind::Folder, Value::Text(folder)) => {
                label(ui, setting.label, setting.help);
                if ui.button("Choose…").on_hover_text(setting.help).clicked() {
                    acts.push(Act::ChooseFolder(index));
                }
                if ui.button("Open").on_hover_text("Show the folder").clicked() {
                    acts.push(Act::Open(PathBuf::from(folder)));
                }
                path_label(ui, folder);
            }
            _ => {}
        }
    });
}

/// A row's label, in the column the page's labels share, level with the
/// control beside it.
fn label(ui: &mut egui::Ui, text: &str, help: &str) {
    ui.allocate_ui_with_layout(egui::vec2(LABELS, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(LABELS);
        let response = ui.label(text);
        if !help.is_empty() {
            response.on_hover_text(help);
        }
    });
}

/// A hand-drawn part of a page, set in as far as the rows are.
fn inset(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(GUTTER);
        ui.vertical(add);
    });
}

/// A folder's path, small, cut short in the middle of a long one.
fn path_label(ui: &mut egui::Ui, path: &str) {
    ui.add(egui::Label::new(egui::RichText::new(path).small().color(crate::theme::dim())).truncate()).on_hover_text(path);
}

/// Answer the scripting channel's `preferences` operation: with nothing
/// asked, every setting as it is; `set` an object of settings by name,
/// all checked before any is taken; `reset` `"all"`, a page's name, or
/// a list of settings. The reply is every setting afterwards.
pub fn script(app: &mut TequilaApp, request: &tequila_format::json::Value) -> Result<tequila_format::json::Value, String> {
    use tequila_format::json::Value as Json;
    use tequila_format::json_write::Object;
    if let Some(set) = request.get("set") {
        let Json::Object(asked) = set else {
            return Err("`set` is an object of settings by name: {\"viewport.gizmo_size\": 150}".into());
        };
        let mut checked = Vec::new();
        let mut keys = Vec::new();
        for (key, value) in asked {
            // A shortcut: its keys as one written out, a list of them, or
            // "" for none.
            if let Some(name) = key.strip_prefix(SHORTCUT_PREFIX) {
                let action = Action::named(name).ok_or_else(|| format!("there is no shortcut `{key}`; ask for `preferences` with nothing else to list them"))?;
                let chords = match value {
                    Json::String(one) if one.trim().is_empty() => Vec::new(),
                    Json::String(one) => vec![Chord::parse(one)?],
                    Json::Array(several) => several.iter().map(|k| k.as_str().ok_or_else(|| format!("`{key}` lists keys as words")).and_then(Chord::parse)).collect::<Result<_, _>>()?,
                    _ => return Err(format!("`{key}` takes a key written out (\"Ctrl + G\"), a list of them, or \"\" for none")),
                };
                if chords.len() > crate::shortcuts::SLOTS {
                    return Err(format!("`{key}` has room for {} keys", crate::shortcuts::SLOTS));
                }
                keys.push((action, chords));
                continue;
            }
            let setting = setting(key).ok_or_else(|| format!("there is no setting `{key}`; ask for `preferences` with nothing else to list them"))?;
            checked.push((setting, setting.from_json(value)?));
        }
        for (setting, value) in checked {
            app.set_preference(setting.key, value)?;
        }
        if !keys.is_empty() {
            // A key given to one action is taken from the one that had it,
            // as in the window once it has asked.
            for (action, chords) in keys {
                app.shortcuts.bind(action, &chords);
            }
            app.keep_preferences();
        }
    }
    match request.get("reset") {
        None | Some(Json::Null) => {}
        Some(Json::String(what)) if what.eq_ignore_ascii_case("all") => app.reset_preferences(None)?,
        Some(Json::String(what)) => match (Page::from_key(what), setting(what), what.strip_prefix(SHORTCUT_PREFIX).and_then(Action::named)) {
            (Some(page), _, _) => app.reset_preferences(Some(page))?,
            (None, Some(setting), _) => app.set_preference(setting.key, (setting.default)())?,
            (None, None, Some(action)) => {
                app.shortcuts.reset(action);
                app.keep_preferences();
            }
            (None, None, None) => return Err(format!("`{what}` is not \"all\", a page, a setting or a shortcut")),
        },
        Some(Json::Array(keys)) => {
            let mut settings = Vec::new();
            for key in keys {
                let name = key.as_str().ok_or("`reset` lists settings by name")?;
                settings.push(setting(name).ok_or_else(|| format!("there is no setting `{name}`"))?);
            }
            for setting in settings {
                app.set_preference(setting.key, (setting.default)())?;
            }
        }
        Some(_) => return Err("`reset` is \"all\", a page's name, a setting's name or a list of settings".into()),
    }
    let settings: Vec<Json> = SETTINGS
        .iter()
        .map(|setting| {
            let mut out = Object::new();
            out.set("key", setting.key).set("page", setting.page.key()).set("group", setting.group).set("label", setting.label).set("help", setting.help);
            out.set("value", (setting.get)(app).to_json()).set("default", (setting.default)().to_json());
            match &setting.kind {
                Kind::Toggle => {
                    out.set("kind", "toggle");
                }
                Kind::Number { min, max, .. } => {
                    out.set("kind", "number").set("min", *min).set("max", *max);
                }
                Kind::Choice(options) => {
                    out.set("kind", "choice").set("options", options.iter().map(|(key, _)| Json::String((*key).to_string())).collect::<Vec<_>>());
                }
                Kind::Folder => {
                    out.set("kind", "folder");
                }
            }
            out.build()
        })
        .collect();
    let shortcuts: Vec<Json> = crate::shortcuts::ACTIONS
        .iter()
        .map(|info| {
            let written = |chords: &[Chord]| chords.iter().map(|chord| Json::String(chord.shown())).collect::<Vec<_>>();
            let mut out = Object::new();
            out.set("key", format!("{SHORTCUT_PREFIX}{}", info.key)).set("group", info.group.label()).set("label", info.label);
            out.set("keys", written(app.shortcuts.of(info.action))).set("default", written(Shortcuts::default().of(info.action)));
            out.build()
        })
        .collect();
    let mut out = Object::new();
    out.set("settings", settings);
    out.set("shortcuts", shortcuts);
    out.set_some("file", app.prefs_file.as_ref().map(|f| f.display().to_string()));
    Ok(out.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every setting has a key of its own on its page's name, words to
    /// find it by, and a default it takes as it is.
    #[test]
    fn every_setting_is_named_explained_and_takes_its_default() {
        let app = TequilaApp::for_tests();
        for (i, setting) in SETTINGS.iter().enumerate() {
            assert!(setting.key.starts_with(&format!("{}.", setting.page.key())), "{} is not named for its page", setting.key);
            assert!(SETTINGS[..i].iter().all(|other| other.key != setting.key), "{} is there twice", setting.key);
            assert!(!setting.label.is_empty() && setting.help.len() > 20, "{} does not explain itself", setting.key);
            let default = (setting.default)();
            assert_eq!(setting.checked(default.clone()).as_ref(), Ok(&default), "{} does not take its own default", setting.key);
            assert_eq!(setting.parse(&default.to_text()).as_ref(), Ok(&default), "{}'s default does not read back from the file", setting.key);
            // An application as it starts has Tequila's own at their
            // defaults; the others are whatever this computer's files say.
            if setting.own {
                assert!((setting.get)(&app).same(&default), "{} starts at {:?}, not its default {default:?}", setting.key, (setting.get)(&app));
            }
        }
        // The choices offered are the ones the application has.
        assert_eq!(UP_AXES.iter().map(|(k, l)| (*k, *l)).collect::<Vec<_>>(), tequila_import::UpAxis::ALL.iter().map(|a| (a.key(), a.label())).collect::<Vec<_>>());
        assert_eq!(MODES.iter().map(|(k, l)| (*k, *l)).collect::<Vec<_>>(), crate::theme::Mode::ALL.iter().map(|m| (m.key(), m.label())).collect::<Vec<_>>());
        assert_eq!(THEMES.iter().map(|(k, l)| (*k, *l)).collect::<Vec<_>>(), crate::theme::Flavor::ALL.iter().map(|f| (f.key(), f.label())).collect::<Vec<_>>());
        assert_eq!(SIZES.iter().map(|(k, l)| (*k, *l)).collect::<Vec<_>>(), crate::theme::Size::ALL.iter().map(|s| (s.key(), s.label())).collect::<Vec<_>>());
        assert_eq!(SHADINGS.iter().map(|(k, l)| (*k, *l)).collect::<Vec<_>>(), ViewMode::ALL.iter().map(|m| (m.key(), m.label())).collect::<Vec<_>>());
        assert_eq!(MOVING.iter().map(|(k, _)| *k).collect::<Vec<_>>(), MovingPicture::ALL.iter().map(|m| m.key()).collect::<Vec<_>>());
        assert_eq!(LIBRARY_VIEWS.iter().map(|(k, l)| (*k, *l)).collect::<Vec<_>>(), crate::library_tab::View::ALL.iter().map(|v| (v.key(), v.label())).collect::<Vec<_>>());
        assert_eq!(PRESETS.iter().map(|(k, _)| *k).collect::<Vec<_>>(), crate::navigation::Preset::ALL.iter().map(|p| p.key()).collect::<Vec<_>>());
        assert_eq!(GIZMO_MODES.iter().map(|(k, _)| *k).collect::<Vec<_>>(), GizmoMode::ALL.iter().map(|m| gizmo_key(*m)).collect::<Vec<_>>());
    }

    /// A value is held to what its setting takes: a number to its range
    /// and decimals, a choice to its own, each said plainly when it is not.
    #[test]
    fn a_value_is_checked_against_its_setting() {
        let size = setting("viewport.gizmo_size").unwrap();
        assert_eq!(size.checked(Value::Number(1000.0)), Ok(Value::Number(250.0)));
        assert_eq!(size.checked(Value::Number(120.4)), Ok(Value::Number(120.0)));
        assert!(size.checked(Value::Bool(true)).unwrap_err().contains("a number from 50 to 250"));
        assert!(size.checked(Value::Number(f64::NAN)).is_err());
        let shading = setting("viewport.shading").unwrap();
        assert_eq!(shading.checked(Value::Text("Rendered".into())), Ok(Value::Text("rendered".into())));
        assert!(shading.checked(Value::Text("shiny".into())).unwrap_err().contains("wireframe, solid, material, rendered"));
        let grid = setting("viewport.grid").unwrap();
        assert_eq!(grid.parse("on"), Ok(Value::Bool(true)));
        assert_eq!(grid.parse(" false "), Ok(Value::Bool(false)));
        assert!(grid.parse("maybe").is_err());
        assert_eq!(size.shown(&Value::Number(150.0)), "150%");
        assert_eq!(shading.shown(&Value::Text("material".into())), "Material Preview");
    }

    /// Setting a preference acts on the session at once and is kept in the
    /// file; a session started from that file starts as it says.
    #[test]
    fn a_setting_acts_at_once_is_kept_and_starts_the_next_session() {
        let folder = std::env::temp_dir().join(format!("tequila-preferences-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let file = folder.join("preferences.txt");
        let mut app = TequilaApp::for_tests();
        app.load_preferences(file.clone());
        assert!(app.show_gizmo && !app.show_hud && app.gizmo.mode == GizmoMode::Universal);

        app.set_preference("viewport.gizmo", Value::Bool(false)).unwrap();
        app.set_preference("viewport.statistics", Value::Bool(true)).unwrap();
        app.set_preference("viewport.gizmo_mode", Value::Text("rotate".into())).unwrap();
        app.set_preference("viewport.gizmo_size", Value::Number(180.0)).unwrap();
        app.set_preference("navigation.zoom_speed", Value::Number(2.0)).unwrap();
        app.set_preference("navigation.invert_zoom", Value::Bool(true)).unwrap();
        app.set_preference("general.autosave_minutes", Value::Number(12.0)).unwrap();
        assert!(!app.show_gizmo && app.show_hud, "the session did not follow");
        assert_eq!(app.gizmo.mode, GizmoMode::Rotate);
        assert_eq!(app.prefs.zoom_factor(), -2.0);
        assert_eq!(app.prefs.autosave_every(), Some(std::time::Duration::from_secs(12 * 60)));
        assert!(app.set_preference("viewport.gizmo_mode", Value::Text("sideways".into())).is_err());
        assert!(app.set_preference("viewport.nothing", Value::Bool(true)).unwrap_err().contains("no setting"));

        let kept = std::fs::read_to_string(&file).expect("the settings were not kept");
        assert!(kept.contains("viewport.gizmo=off\n") && kept.contains("viewport.gizmo_size=180\n") && kept.contains("viewport.gizmo_mode=rotate\n"), "{kept}");
        assert!(!kept.contains("interface.mode"), "the look is kept by its own file: {kept}");

        // The next session.
        let mut next = TequilaApp::for_tests();
        next.load_preferences(file.clone());
        assert!(!next.show_gizmo && next.show_hud);
        assert_eq!(next.gizmo.mode, GizmoMode::Rotate);
        assert_eq!(next.prefs, app.prefs);

        // A page put back, and no other.
        next.reset_preferences(Some(Page::Viewport)).unwrap();
        assert!(next.show_gizmo && !next.show_hud && next.gizmo.mode == GizmoMode::Universal);
        assert_eq!(next.prefs.zoom_speed, 2.0, "Navigation was reset with Viewport");
        assert!(next.prefs.autosave_minutes == 12.0);
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A preset is its program's three drags; a drag changed by hand makes
    /// it Custom, swapping with the move that had that drag; both are kept
    /// and start the next session as they were.
    #[test]
    fn a_navigation_preset_and_a_custom_drag_are_kept() {
        use crate::navigation::{Drag, Preset};
        let folder = std::env::temp_dir().join(format!("tequila-navigation-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let file = folder.join("preferences.txt");
        let mut app = TequilaApp::for_tests();
        app.load_preferences(file.clone());
        assert_eq!(app.prefs.navigation(), Preset::KeyShot.scheme().unwrap());

        app.set_preference("navigation.preset", Value::Text("blender".into())).unwrap();
        assert_eq!(app.prefs.navigation(), Preset::Blender.scheme().unwrap());
        let value = |app: &TequilaApp, key: &str| (setting(key).unwrap().get)(app);
        assert_eq!(value(&app, "navigation.orbit_drag"), Value::Text("middle".into()));
        assert_eq!(value(&app, "navigation.pan_drag"), Value::Text("shift+middle".into()));
        // The next session is Blender's still, not a Custom that looks like it.
        let mut next = TequilaApp::for_tests();
        next.load_preferences(file.clone());
        assert_eq!(next.prefs.nav_preset, Preset::Blender);

        // Orbit given pan's drag: Custom, and pan has orbit's.
        app.set_preference("navigation.orbit_drag", Value::Text("shift+middle".into())).unwrap();
        assert_eq!(app.prefs.nav_preset, Preset::Custom);
        assert_eq!(value(&app, "navigation.pan_drag"), Value::Text("middle".into()));
        app.set_preference("navigation.zoom_drag", Value::Text("alt+left".into())).unwrap();
        let custom = app.prefs.navigation();
        assert_eq!((custom.orbit, custom.pan, custom.zoom), (Drag::from_key("shift+middle").unwrap(), Drag::from_key("middle").unwrap(), Drag::from_key("alt+left").unwrap()));
        let mut next = TequilaApp::for_tests();
        next.load_preferences(file.clone());
        assert_eq!(next.prefs.nav_preset, Preset::Custom);
        assert_eq!(next.prefs.navigation(), custom);
        assert!(app.set_preference("navigation.orbit_drag", Value::Text("fourth button".into())).is_err());

        // Under a preset the drags are not marked as changed; a drag of
        // the person's own is.
        let orbit = setting("navigation.orbit_drag").unwrap();
        assert!(!(orbit.get)(&next).same(&next.shown_default(orbit)), "a custom drag is marked");
        // Another preset leaves the custom drags aside; Custom brings back
        // the drags in use, to change from.
        next.set_preference("navigation.preset", Value::Text("rhino".into())).unwrap();
        assert_eq!(next.prefs.navigation(), Preset::Rhino.scheme().unwrap());
        assert!((orbit.get)(&next).same(&next.shown_default(orbit)), "Rhino's own drag was marked as a change");
        next.set_preference("navigation.preset", Value::Text("custom".into())).unwrap();
        assert_eq!(next.prefs.navigation(), Preset::Rhino.scheme().unwrap());
        next.reset_preferences(Some(Page::Navigation)).unwrap();
        assert_eq!(next.prefs.nav_preset, Preset::KeyShot);
        assert_eq!(next.prefs.navigation(), Preset::KeyShot.scheme().unwrap());
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// A file that is half nonsense still gives what it can, and a line of
    /// another setting's has no place in Tequila's own file.
    #[test]
    fn a_damaged_file_gives_what_it_can() {
        let mut app = TequilaApp::for_tests();
        let (taken, wrong) = app.read_preferences("# a note\nviewport.gizmo_size=140\nviewport.gizmo_size\nviewport.samples=lots\nnothing.here=1\n\nnavigation.pan_speed=9\n", true);
        assert_eq!(taken, 2);
        assert_eq!(wrong.len(), 3, "{wrong:?}");
        assert_eq!(app.prefs.gizmo_size, 140.0);
        assert_eq!(app.prefs.pan_speed, 4.0, "held to its range");
        assert_eq!(app.prefs.view_samples, 1024);
    }

    /// The exported file holds every setting and the 3D mouse's, and read
    /// back gives the same settings.
    #[test]
    fn an_exported_file_reads_back() {
        let mut app = TequilaApp::for_tests();
        app.set_preference("viewport.samples", Value::Number(256.0)).unwrap();
        app.set_preference("navigation.trackpad", Value::Text("orbit".into())).unwrap();
        app.spacemouse.settings.sensitivity = 2.5;
        let exported = app.exported_preferences();
        for setting in SETTINGS {
            assert!(exported.contains(&format!("\n{}=", setting.key)), "{} is not in the export", setting.key);
        }
        assert!(exported.contains("3d_mouse.speed=2.5"), "{exported}");
        assert!(exported.contains("\nshortcut.edit.undo=Ctrl + Z\n") && exported.contains("\nshortcut.panels.preferences=Ctrl + Comma\n"), "{exported}");
        // Only Tequila's own and the 3D mouse's are read back here: the
        // others write to the person's settings folder as they are set.
        let own: String = exported.lines().filter(|l| l.starts_with('#') || l.starts_with(MOUSE_PREFIX) || setting(l.split('=').next().unwrap()).is_some_and(|s| s.own)).map(|l| format!("{l}\n")).collect();
        let mut other = TequilaApp::for_tests();
        let (taken, wrong) = other.read_preferences(&own, false);
        assert!(wrong.is_empty(), "{wrong:?}");
        assert_eq!(taken, SETTINGS.iter().filter(|s| s.own).count() + 1);
        assert_eq!(other.prefs, app.prefs);
        assert_eq!(other.spacemouse.settings.sensitivity, 2.5);
    }

    /// The search finds a setting by any of its words, and the hand-drawn
    /// parts by theirs.
    #[test]
    fn the_search_finds_settings_by_their_words() {
        let find = |query: &str| -> Vec<&str> {
            let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
            SETTINGS.iter().filter(|s| s.matches(&words)).map(|s| s.key).collect()
        };
        assert_eq!(find("gizmo size"), vec!["viewport.gizmo_size"]);
        assert!(find("gizmo").len() >= 3);
        assert!(find("zoom").contains(&"navigation.zoom_speed") && find("zoom").contains(&"navigation.invert_zoom"));
        assert_eq!(find("upside down"), vec!["import.up_axis"], "a choice is found by its options");
        assert_eq!(find("blender"), vec!["navigation.preset", "import.blender_lights"]);
        assert!(find("xyzzy").is_empty());
        assert!(EXTRAS.iter().any(|e| e.words.contains("spacemouse")));
    }

    fn press(app: &mut TequilaApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        let input = egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
        let _ = ctx.run(input, |ctx| app.global_hotkeys(ctx));
    }

    /// A shortcut given another key answers to it and no longer to the old
    /// one; the change is kept, starts the next session, and is put back
    /// with the page.
    #[test]
    fn a_shortcut_given_another_key_answers_to_it_and_is_kept() {
        let folder = std::env::temp_dir().join(format!("tequila-shortcuts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let file = folder.join("preferences.txt");
        let ctx = egui::Context::default();
        let mut app = TequilaApp::for_tests();
        app.load_preferences(file.clone());
        press(&mut app, &ctx, egui::Key::H, egui::Modifiers::NONE);
        assert!(app.show_hud, "H shows the statistics by default");

        // Statistics on F9, as the window sets it.
        app.preferences_act(Act::Choose(Some((Action::Statistics, 0))));
        assert!(app.preferences.choosing_key());
        // While a key is being chosen, keys are not commands.
        press(&mut app, &ctx, egui::Key::H, egui::Modifiers::NONE);
        assert!(app.show_hud, "a key pressed while choosing was taken as a command");
        app.preferences_act(Act::Key(Action::Statistics, 0, Some(Chord::plain(egui::Key::F9))));
        assert!(!app.preferences.choosing_key());
        assert_eq!(app.key(Action::Statistics), "F9");
        press(&mut app, &ctx, egui::Key::H, egui::Modifiers::NONE);
        assert!(app.show_hud, "the old key still answers");
        press(&mut app, &ctx, egui::Key::F9, egui::Modifiers::NONE);
        assert!(!app.show_hud, "the new key does not answer");

        let kept = std::fs::read_to_string(&file).unwrap();
        assert!(kept.contains("shortcut.view.statistics=F9\n"), "{kept}");
        assert!(!kept.contains("shortcut.edit.undo"), "an unchanged shortcut was kept: {kept}");
        let mut next = TequilaApp::for_tests();
        next.load_preferences(file.clone());
        assert_eq!(next.shortcuts, app.shortcuts);

        next.reset_preferences(Some(Page::Keyboard)).unwrap();
        assert_eq!(next.shortcuts, Shortcuts::default());
        assert!(!std::fs::read_to_string(&file).unwrap().contains("shortcut."));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// In the window itself: a key clicked to change takes the next key
    /// pressed there, with what was held.
    #[test]
    fn the_window_takes_the_next_key_pressed() {
        let mut app = TequilaApp::for_tests();
        let ctx = egui::Context::default();
        app.preferences.open = true;
        app.preferences.page = Page::Keyboard;
        app.preferences_act(Act::Choose(Some((Action::Screenshot, 0))));
        let modifiers = egui::Modifiers::SHIFT;
        let input = egui::RawInput { modifiers, events: vec![egui::Event::Key { key: egui::Key::F8, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
        let _ = ctx.run(input, |ctx| app.preferences_window(ctx));
        assert_eq!(app.key(Action::Screenshot), "Shift + F8");
        assert!(!app.preferences.choosing_key());
        // And nothing more is taken once it has one.
        let input = egui::RawInput { events: vec![egui::Event::Key { key: egui::Key::F7, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }], ..Default::default() };
        let _ = ctx.run(input, |ctx| app.preferences_window(ctx));
        assert_eq!(app.key(Action::Screenshot), "Shift + F8");
    }

    /// A key another action has is asked about before it is taken, and
    /// either answer is honoured; a key no action has is taken at once.
    #[test]
    fn a_key_in_use_is_asked_about_before_it_moves() {
        let mut app = TequilaApp::for_tests();
        let g = Chord::parse("Ctrl + G").unwrap();
        app.preferences_act(Act::Key(Action::Duplicate, 1, Some(g)));
        assert_eq!(app.preferences.clash, Some(Clash { action: Action::Duplicate, slot: 1, chord: g, other: Action::Group }));
        assert_eq!(app.shortcuts, Shortcuts::default(), "the key moved before the answer");
        app.preferences_act(Act::LeaveKey);
        assert!(app.preferences.clash.is_none() && app.shortcuts == Shortcuts::default());

        app.preferences_act(Act::Key(Action::Duplicate, 1, Some(g)));
        app.preferences_act(Act::TakeKey);
        assert_eq!(app.key(Action::Duplicate), "Ctrl + D / Ctrl + G");
        assert!(app.key(Action::Group).is_empty());
        // Rotate's E is also the environment's, by design: no question.
        app.preferences_act(Act::Key(Action::GizmoRotate, 1, Some(Chord::plain(egui::Key::E))));
        assert!(app.preferences.clash.is_none());
        // One action's key put back takes it back.
        app.preferences_act(Act::ResetKey(Action::Group));
        assert_eq!(app.key(Action::Group), "Ctrl + G");
        assert_eq!(app.key(Action::Duplicate), "Ctrl + D");
        // A place emptied.
        app.preferences_act(Act::Key(Action::Console, 0, None));
        assert!(app.key(Action::Console).is_empty());
    }

    /// The press that chooses a key in the window is not also the undo or
    /// save that window passes on.
    #[test]
    fn the_press_that_chooses_a_key_is_not_passed_on() {
        let mut app = TequilaApp::for_tests();
        app.preferences_act(Act::Choose(Some((Action::Statistics, 0))));
        assert!(app.preferences.swallows_keys());
        app.preferences_act(Act::Key(Action::Statistics, 0, Some(Chord::parse("Ctrl + Z").unwrap())));
        assert!(app.preferences.clash.is_some(), "Ctrl+Z is Undo's");
        assert!(app.preferences.swallows_keys(), "the frame after choosing");
        for _ in 0..3 {
            app.preferences.swallows_keys();
        }
        assert!(!app.preferences.swallows_keys(), "keys are passed on again");
    }

    /// A script lists the shortcuts, gives one other keys, and puts it
    /// back; a key that is not one is refused.
    #[test]
    fn a_script_reads_and_changes_shortcuts() {
        use tequila_format::json::Value as Json;
        let ask = |app: &mut TequilaApp, text: &str| script(app, &tequila_format::json::parse(text.as_bytes()).unwrap());
        let keys_of = |reply: &Json, key: &str| -> Vec<String> {
            let all = reply.get("shortcuts").unwrap().as_array();
            let one = all.iter().find(|s| s.get("key").and_then(Json::as_str) == Some(key)).unwrap();
            one.get("keys").unwrap().as_array().iter().map(|k| k.as_str().unwrap().to_string()).collect()
        };
        let mut app = TequilaApp::for_tests();
        let listed = ask(&mut app, "{}").unwrap();
        assert_eq!(listed.get("shortcuts").unwrap().as_array().len(), crate::shortcuts::ACTIONS.len());
        assert_eq!(keys_of(&listed, "shortcut.edit.redo"), vec!["Ctrl + Y", "Ctrl + Shift + Z"]);

        let reply = ask(&mut app, r#"{"set":{"shortcut.view.screenshot":["F12","Ctrl + Shift + ,"],"shortcut.panels.console":""}}"#).unwrap();
        assert_eq!(keys_of(&reply, "shortcut.view.screenshot"), vec!["F12", "Ctrl + Shift + ,"]);
        assert!(keys_of(&reply, "shortcut.panels.console").is_empty());
        assert!(ask(&mut app, r#"{"set":{"shortcut.view.screenshot":"Ctrl + Banana"}}"#).unwrap_err().contains("not a key"));
        assert!(ask(&mut app, r#"{"set":{"shortcut.view.nothing":"A"}}"#).unwrap_err().contains("no shortcut"));
        assert!(ask(&mut app, r#"{"set":{"shortcut.edit.redo":["A","B","C"]}}"#).unwrap_err().contains("room for 2"));
        assert_eq!(app.key(Action::Screenshot), "F12 / Ctrl + Shift + ,");

        ask(&mut app, r#"{"reset":"shortcut.view.screenshot"}"#).unwrap();
        assert_eq!(app.key(Action::Screenshot), "P");
        assert!(app.key(Action::Console).is_empty());
        ask(&mut app, r#"{"reset":"keyboard"}"#).unwrap();
        assert_eq!(app.shortcuts, Shortcuts::default());
    }

    /// The search finds a shortcut by what it does and by its key.
    #[test]
    fn the_search_finds_shortcuts() {
        let keys = Shortcuts::default();
        let find = |query: &str| -> Vec<&str> {
            let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
            crate::shortcuts::ACTIONS.iter().filter(|info| shortcut_matches(info, &keys, &words)).map(|info| info.key).collect()
        };
        assert_eq!(find("screenshot"), vec!["view.screenshot"]);
        assert!(find("shift").contains(&"view.gizmo") && find("shift").contains(&"camera.frame_all"));
        assert_eq!(find("isometric"), vec!["camera.isometric"]);
    }

    /// A script lists the settings, sets several at once or none when one
    /// is wrong, and puts them back.
    #[test]
    fn a_script_reads_sets_and_resets_settings() {
        use tequila_format::json::Value as Json;
        let ask = |app: &mut TequilaApp, text: &str| script(app, &tequila_format::json::parse(text.as_bytes()).unwrap());
        let value_of = |reply: &Json, key: &str| -> Json {
            reply.get("settings").unwrap().as_array().iter().find(|s| s.get("key").and_then(Json::as_str) == Some(key)).unwrap().get("value").unwrap().clone()
        };
        let mut app = TequilaApp::for_tests();
        let listed = ask(&mut app, "{}").unwrap();
        assert_eq!(listed.get("settings").unwrap().as_array().len(), SETTINGS.len());
        assert_eq!(value_of(&listed, "viewport.gizmo_size"), Json::Number(100.0));

        let reply = ask(&mut app, r#"{"set":{"viewport.gizmo_size":150,"viewport.grid":true,"viewport.moving_picture":"sharp"}}"#).unwrap();
        assert_eq!(value_of(&reply, "viewport.gizmo_size"), Json::Number(150.0));
        assert!(app.camera_tab.show_grid && app.prefs.moving_picture == MovingPicture::Sharp);

        // One wrong value, and nothing of the request is taken.
        let error = ask(&mut app, r#"{"set":{"viewport.gizmo_size":200,"viewport.moving_picture":"blurry"}}"#).unwrap_err();
        assert!(error.contains("smooth, balanced, sharp"), "{error}");
        assert_eq!(app.prefs.gizmo_size, 150.0);

        ask(&mut app, r#"{"reset":["viewport.gizmo_size"]}"#).unwrap();
        assert_eq!(app.prefs.gizmo_size, 100.0);
        assert!(app.camera_tab.show_grid);
        ask(&mut app, r#"{"reset":"viewport"}"#).unwrap();
        assert!(!app.camera_tab.show_grid && app.prefs.moving_picture == MovingPicture::Balanced);
        assert!(ask(&mut app, r#"{"reset":"nowhere"}"#).is_err());
    }
}
