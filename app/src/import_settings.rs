//! The import settings kept between sessions, beside the other settings
//! (`import.txt`): which way up the person's files are drawn
//! (`tequila_import::UpAxis`), and whether a Blender file's own lights are
//! used ([`BlenderLights`]). How they work, not what they are working on,
//! so not in the project.

use std::path::PathBuf;
use std::sync::Mutex;
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

/// Both settings, read from the file the first time either is asked for.
static HELD: Mutex<Option<(UpAxis, BlenderLights)>> = Mutex::new(None);

fn path() -> PathBuf {
    tequila_format::library::settings_directory().join("import.txt")
}

fn from_text(text: &str) -> (UpAxis, BlenderLights) {
    let field = |key: &str| text.lines().find_map(|line| line.strip_prefix(key));
    (field("up=").and_then(UpAxis::from_key).unwrap_or_default(), field("blender_lights=").and_then(BlenderLights::from_key).unwrap_or_default())
}

fn to_text((up, lights): (UpAxis, BlenderLights)) -> String {
    format!("up={}\nblender_lights={}\n", up.key(), lights.key())
}

fn held() -> (UpAxis, BlenderLights) {
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    *held.get_or_insert_with(|| std::fs::read_to_string(path()).map(|t| from_text(&t)).unwrap_or_default())
}

fn keep(settings: (UpAxis, BlenderLights)) -> Result<(), String> {
    *HELD.lock().unwrap_or_else(|e| e.into_inner()) = Some(settings);
    let path = path();
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, to_text(settings)).map_err(|e| e.to_string())
}

/// Which way up imports are taken to be drawn.
pub fn up_axis() -> UpAxis {
    held().0
}

/// Choose it, and keep the choice.
pub fn set_up_axis(up: UpAxis) -> Result<(), String> {
    keep((up, held().1))
}

/// What is done with a Blender file's own lighting.
pub fn blender_lights() -> BlenderLights {
    held().1
}

/// Choose it, and keep the choice.
pub fn set_blender_lights(lights: BlenderLights) -> Result<(), String> {
    keep((held().0, lights))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setting_reads_back_and_an_unknown_one_is_as_the_file_says() {
        for up in UpAxis::ALL {
            for lights in BlenderLights::ALL {
                assert_eq!(from_text(&to_text((up, lights))), (up, lights));
            }
        }
        assert_eq!(from_text("up=sideways\n").0, UpAxis::AsFile);
        assert_eq!(from_text(""), (UpAxis::AsFile, BlenderLights::Ask));
        // A file from before the lights were a setting still reads.
        assert_eq!(from_text("up=z\n"), (UpAxis::Z, BlenderLights::Ask));
    }
}
