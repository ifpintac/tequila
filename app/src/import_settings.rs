//! The import settings kept between sessions, beside the other settings
//! (`import.txt`): which way up the person's files are drawn
//! (`tequila_import::UpAxis`), whether a Blender file's own lights are
//! used ([`BlenderLights`]), and the rest of what the import dialog last
//! chose ([`Choices`]). How they work, not what they are working on, so
//! not in the project.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tequila_core::units::LengthUnit;
use tequila_import::step::Tessellation;
use tequila_import::UpAxis;

/// What a Blender file's own lighting - its lights, world and exposure -
/// is done with once the model is in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlenderLights {
    /// Ask, each time a file has some.
    #[default]
    Ask,
    /// Light the model as Blender does.
    Blender,
    /// Keep the lighting the scene has.
    Tequila,
}

impl BlenderLights {
    pub const ALL: [BlenderLights; 3] = [BlenderLights::Ask, BlenderLights::Blender, BlenderLights::Tequila];

    pub fn key(self) -> &'static str {
        match self {
            BlenderLights::Ask => "ask",
            BlenderLights::Blender => "blender",
            BlenderLights::Tequila => "tequila",
        }
    }

    pub fn from_key(key: &str) -> Option<BlenderLights> {
        BlenderLights::ALL.into_iter().find(|b| b.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            BlenderLights::Ask => "Ask each time",
            BlenderLights::Blender => "Use Blender's lighting",
            BlenderLights::Tequila => "Keep Tequila's lighting",
        }
    }
}

/// Where an imported model goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// Into the scene that is open, beside what is there; in place of the
    /// starter ball when that is all there is.
    #[default]
    Add,
    /// Into a new scene, as File › New starts one, after asking about
    /// unsaved changes.
    New,
}

impl Target {
    pub fn key(self) -> &'static str {
        match self {
            Target::Add => "add",
            Target::New => "new",
        }
    }

    pub fn from_key(key: &str) -> Option<Target> {
        [Target::Add, Target::New].into_iter().find(|t| t.key() == key)
    }
}

/// What the import dialog last chose, which the next import starts from.
#[derive(Clone, Debug, PartialEq)]
pub struct Choices {
    /// Show the dialog for each model. Off ("Don't ask again"), a model
    /// goes straight in with these, the progress window still showing;
    /// Preferences › Import, or File › Show import options, turns it back
    /// on.
    pub ask: bool,
    pub target: Target,
    pub tessellation: Tessellation,
    /// Keep a CAD file's exact surfaces with the project, to divide them
    /// again later at another quality without the file.
    pub keep_cad: bool,
    /// Point the camera at the model once it is in.
    pub frame_camera: bool,
    /// Fit the dome and the ground to the scene with the model in it
    /// (`import_dialog::fit_environment`).
    pub fit_environment: bool,
    /// Stand the model on the floor (`ImportOptions::place_on_ground`).
    pub on_floor: bool,
    /// The unit each kind of file is in, by its extension; one not listed
    /// is as the file says, or guessed from its size. Per extension because
    /// that is how a unit goes wrong (`docs/03` §3.4): every STL from one
    /// CAD package is in millimetres, while a glTF says it is in metres.
    pub units: BTreeMap<String, LengthUnit>,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            ask: true,
            target: Target::Add,
            tessellation: Tessellation::default(),
            keep_cad: true,
            frame_camera: true,
            fit_environment: true,
            on_floor: true,
            units: BTreeMap::new(),
        }
    }
}

impl Choices {
    /// The unit files with this extension were last said to be in; `None`
    /// is as the file says.
    pub fn unit_for(&self, extension: &str) -> Option<LengthUnit> {
        self.units.get(&extension.to_ascii_lowercase()).copied()
    }

    pub fn set_unit_for(&mut self, extension: &str, unit: Option<LengthUnit>) {
        let extension = extension.to_ascii_lowercase();
        match unit.filter(|u| *u != LengthUnit::Unspecified) {
            Some(unit) => self.units.insert(extension, unit),
            None => self.units.remove(&extension),
        };
    }
}

/// The units a file can be said to be in, smallest first.
pub const UNITS: [LengthUnit; 5] = [LengthUnit::Millimetre, LengthUnit::Centimetre, LengthUnit::Metre, LengthUnit::Inch, LengthUnit::Foot];

fn unit_from_symbol(symbol: &str) -> Option<LengthUnit> {
    UNITS.into_iter().find(|u| u.symbol() == symbol)
}

/// Everything in the file, read the first time any of it is asked for.
#[derive(Clone, Debug, Default, PartialEq)]
struct Kept {
    up: UpAxis,
    lights: BlenderLights,
    choices: Choices,
}

static HELD: Mutex<Option<Kept>> = Mutex::new(None);

fn path() -> PathBuf {
    tequila_format::library::settings_directory().join("import.txt")
}

fn from_text(text: &str) -> Kept {
    let field = |key: &str| text.lines().find_map(|line| line.strip_prefix(key)).map(str::trim);
    let flag = |key: &str, default: bool| match field(key) {
        Some("on") => true,
        Some("off") => false,
        _ => default,
    };
    let defaults = Choices::default();
    let choices = Choices {
        ask: flag("ask=", defaults.ask),
        target: field("target=").and_then(Target::from_key).unwrap_or_default(),
        tessellation: field("tessellation=").and_then(Tessellation::from_label).unwrap_or_default(),
        keep_cad: flag("keep_cad=", defaults.keep_cad),
        frame_camera: flag("frame_camera=", defaults.frame_camera),
        fit_environment: flag("fit_environment=", defaults.fit_environment),
        on_floor: flag("on_floor=", defaults.on_floor),
        units: text
            .lines()
            .filter_map(|line| line.strip_prefix("unit."))
            .filter_map(|rest| rest.split_once('='))
            .filter_map(|(extension, symbol)| Some((extension.trim().to_ascii_lowercase(), unit_from_symbol(symbol.trim())?)))
            .collect(),
    };
    Kept {
        up: field("up=").and_then(UpAxis::from_key).unwrap_or_default(),
        lights: field("blender_lights=").and_then(BlenderLights::from_key).unwrap_or_default(),
        choices,
    }
}

fn to_text(kept: &Kept) -> String {
    let on = |b: bool| if b { "on" } else { "off" };
    let c = &kept.choices;
    let mut text = format!(
        "up={}\nblender_lights={}\nask={}\ntarget={}\ntessellation={}\nkeep_cad={}\nframe_camera={}\nfit_environment={}\non_floor={}\n",
        kept.up.key(),
        kept.lights.key(),
        on(c.ask),
        c.target.key(),
        c.tessellation.name(),
        on(c.keep_cad),
        on(c.frame_camera),
        on(c.fit_environment),
        on(c.on_floor),
    );
    for (extension, unit) in &c.units {
        text.push_str(&format!("unit.{extension}={}\n", unit.symbol()));
    }
    text
}

fn held() -> Kept {
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    held.get_or_insert_with(|| std::fs::read_to_string(path()).map(|t| from_text(&t)).unwrap_or_default()).clone()
}

fn keep(kept: Kept) -> Result<(), String> {
    let text = to_text(&kept);
    *HELD.lock().unwrap_or_else(|e| e.into_inner()) = Some(kept);
    let path = path();
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// Which way up imports are taken to be drawn.
pub fn up_axis() -> UpAxis {
    held().up
}

/// Choose it, and keep the choice.
pub fn set_up_axis(up: UpAxis) -> Result<(), String> {
    keep(Kept { up, ..held() })
}

/// What is done with a Blender file's own lighting.
pub fn blender_lights() -> BlenderLights {
    held().lights
}

/// Choose it, and keep the choice.
pub fn set_blender_lights(lights: BlenderLights) -> Result<(), String> {
    keep(Kept { lights, ..held() })
}

/// The import dialog's last choices.
pub fn choices() -> Choices {
    held().choices
}

/// Keep them for the next import, and the next session.
pub fn set_choices(choices: Choices) -> Result<(), String> {
    keep(Kept { choices, ..held() })
}

/// File › Lights in Blender files: one choice each, the current one ticked.
pub fn blender_lights_menu(ui: &mut eframe::egui::Ui) -> Option<String> {
    let current = blender_lights();
    let mut said = None;
    for choice in BlenderLights::ALL {
        if ui.radio(current == choice, choice.label()).clicked() {
            said = Some(match set_blender_lights(choice) {
                Ok(()) => format!("Blender files' lighting: {}", choice.label().to_lowercase()),
                Err(error) => format!("Could not keep the setting: {error}"),
            });
            ui.close_menu();
        }
    }
    said
}

/// File › Up axis of imports: one choice each, the current one ticked.
pub fn menu(ui: &mut eframe::egui::Ui) -> Option<String> {
    let current = up_axis();
    let mut said = None;
    for up in UpAxis::ALL {
        if ui.radio(current == up, up.label()).clicked() {
            said = Some(match set_up_axis(up) {
                Ok(()) => format!("Models are imported {}", if up == UpAxis::AsFile { "as their files say".to_string() } else { up.label().to_string() }),
                Err(error) => format!("Could not keep the setting: {error}"),
            });
            ui.close_menu();
        }
    }
    said
}

/// File › Show import options: whether the import dialog comes up for
/// each model, the way back from its "Don't ask again".
pub fn ask_menu(ui: &mut eframe::egui::Ui) -> Option<String> {
    let mut choices = choices();
    let mut ask = choices.ask;
    if ui
        .checkbox(&mut ask, "Show import options")
        .on_hover_text("Ask how to bring each model in: into this scene or a new one, which way up, its unit, how finely to divide a CAD file. Off, models come in as you last chose.")
        .clicked()
    {
        choices.ask = ask;
        ui.close_menu();
        return Some(match set_choices(choices) {
            Ok(()) if ask => "Each import asks how to bring the model in".to_string(),
            Ok(()) => "Models are imported as you last chose, without asking".to_string(),
            Err(error) => format!("Could not keep the setting: {error}"),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_reads_back_and_an_unknown_one_is_as_the_file_says() {
        for up in UpAxis::ALL {
            for lights in BlenderLights::ALL {
                let kept = Kept { up, lights, ..Kept::default() };
                assert_eq!(from_text(&to_text(&kept)), kept);
            }
        }
        assert_eq!(from_text("up=sideways\n").up, UpAxis::AsFile);
        assert_eq!(from_text(""), Kept::default());
        // A file from before the lights were a setting still reads.
        let old = from_text("up=z\n");
        assert_eq!((old.up, old.lights), (UpAxis::Z, BlenderLights::Ask));
    }

    /// Everything the import dialog chose comes back as it was chosen, a
    /// custom tessellation's tolerances and each extension's unit with it,
    /// and a file from before the dialog reads as its defaults.
    #[test]
    fn the_import_dialogs_choices_round_trip() {
        let mut choices = Choices {
            ask: false,
            target: Target::New,
            tessellation: Tessellation::custom(2.5e-4, 15f64.to_radians()),
            keep_cad: false,
            frame_camera: false,
            fit_environment: false,
            on_floor: false,
            units: BTreeMap::new(),
        };
        choices.set_unit_for("STL", Some(LengthUnit::Millimetre));
        choices.set_unit_for("obj", Some(LengthUnit::Inch));
        choices.set_unit_for("glb", None);
        let kept = Kept { up: UpAxis::Z, lights: BlenderLights::Blender, choices };
        let back = from_text(&to_text(&kept));
        assert_eq!((back.up, back.lights), (UpAxis::Z, BlenderLights::Blender));
        let c = &back.choices;
        assert_eq!((c.ask, c.target, c.keep_cad, c.frame_camera, c.fit_environment, c.on_floor), (false, Target::New, false, false, false, false));
        assert!((c.tessellation.chord() - 2.5e-4).abs() < 1e-12 && (c.tessellation.angle() - 15f64.to_radians()).abs() < 1e-9, "{:?}", c.tessellation);
        assert_eq!(c.unit_for("stl"), Some(LengthUnit::Millimetre));
        assert_eq!(c.unit_for("OBJ"), Some(LengthUnit::Inch));
        assert_eq!(c.unit_for("glb"), None);
        // Before the dialog: only the up axis and the lights.
        let old = from_text("up=y\nblender_lights=tequila\n");
        assert_eq!(old.choices, Choices::default());
        assert!(old.choices.ask, "an old setting file must not silence the dialog");
        // A unit that is not one is as the file says.
        assert_eq!(from_text("unit.stl=furlong\n").choices.unit_for("stl"), None);
    }
}
