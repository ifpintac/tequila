//! P's Screenshot: the viewport's picture as it is, saved as a PNG.
//!
//! The picture alone - developed and graded as it shows, at the preview's
//! own resolution, still refining or not - and none of what is drawn over
//! it: the statistics, the gizmo, the selection's outline, the grid. Into
//! a folder of the person's choosing, kept in the application's settings
//! rather than the project: by default `Tequila Screenshots` in their
//! Pictures folder, or in a portable copy's data folder. Named for the
//! project and the moment, and never over another file.
//!
//! What is shown comes from one of two places (`main.rs`): the renderer's
//! image, which the Rendered view and a processor's Solid show, or the
//! graphics card's Solid, read back as it is drawn (`solid_gpu`).

use std::path::{Path, PathBuf};

/// The folder screenshots go into unless the person chose another.
pub const FOLDER_NAME: &str = "Tequila Screenshots";

/// Where screenshots go when the person has not chosen: `Tequila
/// Screenshots` in their Pictures folder; in a portable copy, in its data
/// folder, so the copy takes them with it.
pub fn default_folder() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from);
    default_folder_for(tequila_format::library::portable_directory().as_deref(), home)
}

fn default_folder_for(portable: Option<&Path>, home: Option<PathBuf>) -> PathBuf {
    match (portable, home) {
        (Some(data), _) => data.join(FOLDER_NAME),
        (None, Some(home)) => home.join("Pictures").join(FOLDER_NAME),
        (None, None) => std::env::temp_dir().join(FOLDER_NAME),
    }
}

/// The person's choice of folder, kept beside their other settings
/// (`screenshots.txt`), not in the project: where screenshots go is how
/// they work, not what they are working on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preferences {
    pub folder: Option<PathBuf>,
}

impl Preferences {
    fn path() -> PathBuf {
        tequila_format::library::settings_directory().join("screenshots.txt")
    }

    fn from_text(text: &str) -> Preferences {
        let mut out = Preferences::default();
        for line in text.lines() {
            if let Some(path) = line.strip_prefix("folder=") {
                let path = path.trim();
                if !path.is_empty() {
                    out.folder = Some(PathBuf::from(path));
                }
            }
        }
        out
    }

    fn to_text(&self) -> String {
        match &self.folder {
            Some(path) => format!("folder={}\n", path.display()),
            None => String::new(),
        }
    }

    pub fn load() -> Preferences {
        std::fs::read_to_string(Preferences::path()).map(|t| Preferences::from_text(&t)).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Preferences::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, self.to_text()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The folder to save into: the one chosen, or the default.
    pub fn folder(&self) -> PathBuf {
        self.folder.clone().unwrap_or_else(default_folder)
    }
}

/// A screenshot's name, without the `.png`: the project's name, or
/// "Untitled", then the date and the time - `Speaker 2026-11-08 14-03-27`.
/// What a file name may not hold, on any of the three systems, becomes a
/// dash.
pub fn file_stem(project: &str, date: &str, time: &str) -> String {
    let name: String = project
        .trim()
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '-' } else { c })
        .collect();
    // Windows drops a name's trailing dots and spaces.
    let name = name.trim_end_matches(['.', ' ']).trim();
    let name = if name.is_empty() { "Untitled" } else { name };
    format!("{name} {date} {time}")
}

/// Write `width` by `height` RGBA pixels into `folder` as `stem.png`, or
/// `stem (2).png` and on if that is taken: never over a file already
/// there, even one that turned up between looking and writing. The folder
/// is made if it is not there. Returns the file written.
pub fn save(folder: &Path, stem: &str, width: usize, height: usize, rgba: &[u8]) -> Result<PathBuf, String> {
    if width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return Err("there is no picture in the viewport yet".into());
    }
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    for n in 1..10_000 {
        let path = if n == 1 { folder.join(format!("{stem}.png")) } else { folder.join(format!("{stem} ({n}).png")) };
        let file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("{}: {error}", path.display())),
        };
        let written = (|| {
            let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&rgba[..width * height * 4])?;
            writer.finish()
        })();
        return match written {
            Ok(()) => Ok(path),
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                Err(format!("{}: {error}", path.display()))
            }
        };
    }
    Err(format!("{}: every name for {stem} is taken", folder.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The project's name, or Untitled, then the day and the time; nothing
    /// a file system refuses.
    #[test]
    fn a_screenshot_is_named_for_the_project_and_the_moment() {
        assert_eq!(file_stem("Speaker", "2026-11-08", "14-03-27"), "Speaker 2026-11-08 14-03-27");
        assert_eq!(file_stem("", "2026-11-08", "14-03-27"), "Untitled 2026-11-08 14-03-27");
        assert_eq!(file_stem("  ", "2026-11-08", "09-00-00"), "Untitled 2026-11-08 09-00-00");
        assert_eq!(file_stem("A/B: \"v2\"?", "2026-11-08", "14-03-27"), "A-B- -v2-- 2026-11-08 14-03-27");
        assert_eq!(file_stem("Lamp. ", "2026-11-08", "14-03-27"), "Lamp 2026-11-08 14-03-27");
    }

    /// Unchosen, screenshots go to Pictures/Tequila Screenshots, or into a
    /// portable copy's data folder; the choice is kept and read back.
    #[test]
    fn the_folder_is_pictures_or_the_portable_data_folder_or_the_choice() {
        let home = PathBuf::from("home");
        assert_eq!(default_folder_for(None, Some(home.clone())), home.join("Pictures").join("Tequila Screenshots"));
        let data = PathBuf::from("Tequila").join("data");
        assert_eq!(default_folder_for(Some(&data), Some(home)), data.join("Tequila Screenshots"));
        assert_eq!(default_folder_for(None, None), std::env::temp_dir().join("Tequila Screenshots"));

        let chosen = Preferences { folder: Some(PathBuf::from("D:/Shots for the client")) };
        assert_eq!(Preferences::from_text(&chosen.to_text()), chosen);
        assert_eq!(chosen.folder(), PathBuf::from("D:/Shots for the client"));
        assert_eq!(Preferences::from_text(""), Preferences::default());
        assert_eq!(Preferences::from_text("folder=\nnonsense\n"), Preferences::default());
    }

    /// Two screenshots in the same second are two files: the second never
    /// writes over the first, or over anything else there.
    #[test]
    fn a_screenshot_never_writes_over_a_file() {
        let folder = std::env::temp_dir().join(format!("tequila-screenshots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let pixels: Vec<u8> = (0..4 * 3).flat_map(|i| [i as u8 * 20, 0, 255, 255]).collect();
        let first = save(&folder, "Untitled 2026-11-08 14-03-27", 4, 3, &pixels).unwrap();
        let second = save(&folder, "Untitled 2026-11-08 14-03-27", 4, 3, &pixels).unwrap();
        assert_eq!(first, folder.join("Untitled 2026-11-08 14-03-27.png"), "the folder was made and the name kept");
        assert_eq!(second, folder.join("Untitled 2026-11-08 14-03-27 (2).png"));
        let decoder = png::Decoder::new(std::fs::File::open(&first).unwrap());
        let reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (4, 3));
        assert!(save(&folder, "Empty", 0, 0, &[]).is_err(), "nothing to save, saved");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
