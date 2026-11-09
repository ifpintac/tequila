//! The import settings kept between sessions, beside the other settings
//! (`import.txt`): which way up the person's files are drawn
//! (`tequila_import::UpAxis`). How their CAD package writes files is how
//! they work, not what they are working on, so it is not in the project.

use std::path::PathBuf;
use std::sync::Mutex;
use tequila_import::UpAxis;

/// Read from the file the first time it is asked for.
static UP: Mutex<Option<UpAxis>> = Mutex::new(None);

fn path() -> PathBuf {
    tequila_format::library::settings_directory().join("import.txt")
}

fn from_text(text: &str) -> UpAxis {
    text.lines().find_map(|line| line.strip_prefix("up=")).and_then(UpAxis::from_key).unwrap_or_default()
}

fn to_text(up: UpAxis) -> String {
    format!("up={}\n", up.key())
}

/// Which way up imports are taken to be drawn.
pub fn up_axis() -> UpAxis {
    let mut held = UP.lock().unwrap_or_else(|e| e.into_inner());
    *held.get_or_insert_with(|| std::fs::read_to_string(path()).map(|t| from_text(&t)).unwrap_or_default())
}

/// Choose it, and keep the choice.
pub fn set_up_axis(up: UpAxis) -> Result<(), String> {
    *UP.lock().unwrap_or_else(|e| e.into_inner()) = Some(up);
    let path = path();
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, to_text(up)).map_err(|e| e.to_string())
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
            assert_eq!(from_text(&to_text(up)), up);
        }
        assert_eq!(from_text("up=sideways\n"), UpAxis::AsFile);
        assert_eq!(from_text(""), UpAxis::AsFile);
    }
}
