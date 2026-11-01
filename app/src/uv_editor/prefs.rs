//! What the editor remembers between sessions: its window's size, what its
//! canvas shows, the unit it reads in and the grid. Kept in `uv-editor.txt`
//! beside the user's library, as the appearance is, one `key = value` a
//! line; a line it does not know is left out, a value it cannot read is its
//! default.

use super::units::Unit;
use tequila_scene::uv::Measure;

/// How the grid's spacing is given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spacing {
    /// A length on the part, in the editor's unit: needs a layout of one
    /// size.
    Length,
    /// A fraction of the layout's square: lines every 1/`divisions`.
    Fraction,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    pub size: egui::Vec2,
    pub snapping: bool,
    pub grid: bool,
    pub guides: bool,
    pub rulers: bool,
    pub artwork: bool,
    pub edges: bool,
    pub unit: Unit,
    pub spacing: Spacing,
    /// The grid's step as a length, in millimetres.
    pub grid_mm: f32,
    /// The grid's step as a fraction of the layout: 1 over this.
    pub divisions: u32,
    /// Lines between the grid's own, each step split in this many.
    pub subdivisions: u32,
    pub guides_locked: bool,
    /// How strongly the labels' artwork is drawn over the layout.
    pub artwork_opacity: f32,
    /// The stretch heatmap over the layout: by area, by angle, or none.
    pub stretch: Option<Measure>,
    /// The 2D cursor, drawn.
    pub show_cursor: bool,
    /// The artwork behind the layout made lighter or darker, in stops.
    pub background_exposure: f32,
    /// The padding round each island, drawn, for a texture of a size and
    /// a padding in its pixels.
    pub padding: bool,
    pub padding_texture: u32,
    pub padding_px: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            size: egui::vec2(1240.0, 820.0),
            snapping: true,
            grid: true,
            guides: true,
            rulers: true,
            artwork: true,
            edges: true,
            unit: Unit::Millimetres,
            spacing: Spacing::Fraction,
            grid_mm: 10.0,
            divisions: 10,
            subdivisions: 4,
            guides_locked: false,
            artwork_opacity: 0.9,
            stretch: None,
            show_cursor: true,
            background_exposure: 0.0,
            padding: false,
            padding_texture: 2048,
            padding_px: 8.0,
        }
    }
}

impl Prefs {
    pub fn to_text(&self) -> String {
        let flag = |b: bool| if b { "on" } else { "off" };
        format!(
            "size = {} {}\nsnapping = {}\ngrid = {}\nguides = {}\nrulers = {}\nartwork = {}\nedges = {}\nunit = {}\nspacing = {}\ngrid_mm = {}\ndivisions = {}\nsubdivisions = {}\nguides_locked = {}\nartwork_opacity = {}\nstretch = {}\nshow_cursor = {}\nbackground_exposure = {}\npadding = {}\npadding_texture = {}\npadding_px = {}\n",
            self.size.x.round(),
            self.size.y.round(),
            flag(self.snapping),
            flag(self.grid),
            flag(self.guides),
            flag(self.rulers),
            flag(self.artwork),
            flag(self.edges),
            self.unit.key(),
            match self.spacing {
                Spacing::Length => "length",
                Spacing::Fraction => "fraction",
            },
            self.grid_mm,
            self.divisions,
            self.subdivisions,
            flag(self.guides_locked),
            self.artwork_opacity,
            match self.stretch {
                None => "off",
                Some(Measure::Area) => "area",
                Some(Measure::Angle) => "angle",
            },
            flag(self.show_cursor),
            self.background_exposure,
            flag(self.padding),
            self.padding_texture,
            self.padding_px,
        )
    }

    pub fn from_text(text: &str) -> Prefs {
        let mut out = Prefs::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let value = value.trim();
            let flag = |fallback: bool| match value {
                "on" => true,
                "off" => false,
                _ => fallback,
            };
            let number = |fallback: f32, low: f32, high: f32| value.parse::<f32>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(low, high)).unwrap_or(fallback);
            match key.trim() {
                "size" => {
                    let parts: Vec<f32> = value.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                    if let [w, h] = parts[..] {
                        if w.is_finite() && h.is_finite() {
                            out.size = egui::vec2(w.clamp(480.0, 8000.0), h.clamp(360.0, 8000.0));
                        }
                    }
                }
                "snapping" => out.snapping = flag(out.snapping),
                "grid" => out.grid = flag(out.grid),
                "guides" => out.guides = flag(out.guides),
                "rulers" => out.rulers = flag(out.rulers),
                "artwork" => out.artwork = flag(out.artwork),
                "edges" => out.edges = flag(out.edges),
                "unit" => out.unit = Unit::from_key(value).unwrap_or(out.unit),
                "spacing" => {
                    out.spacing = match value {
                        "length" => Spacing::Length,
                        _ => Spacing::Fraction,
                    }
                }
                "grid_mm" => out.grid_mm = number(out.grid_mm, 0.01, 10_000.0),
                "divisions" => out.divisions = number(out.divisions as f32, 1.0, 1000.0) as u32,
                "subdivisions" => out.subdivisions = number(out.subdivisions as f32, 1.0, 20.0) as u32,
                "guides_locked" => out.guides_locked = flag(out.guides_locked),
                "artwork_opacity" => out.artwork_opacity = number(out.artwork_opacity, 0.0, 1.0),
                "show_cursor" => out.show_cursor = flag(out.show_cursor),
                "background_exposure" => out.background_exposure = number(out.background_exposure, -4.0, 4.0),
                "padding" => out.padding = flag(out.padding),
                "padding_texture" => out.padding_texture = number(out.padding_texture as f32, 64.0, 16384.0) as u32,
                "padding_px" => out.padding_px = number(out.padding_px, 0.0, 512.0),
                "stretch" => {
                    out.stretch = match value {
                        "area" => Some(Measure::Area),
                        "angle" => Some(Measure::Angle),
                        _ => None,
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn path() -> std::path::PathBuf {
        crate::library_tab::user_library_path()
            .parent()
            .map(|p| p.join("uv-editor.txt"))
            .unwrap_or_else(|| std::env::temp_dir().join("tequila-uv-editor.txt"))
    }

    /// The settings saved last time; the defaults in a test, which must not
    /// read or write the person's own.
    pub fn load() -> Prefs {
        if cfg!(test) {
            return Prefs::default();
        }
        std::fs::read_to_string(Prefs::path()).map(|t| Prefs::from_text(&t)).unwrap_or_default()
    }

    pub fn save(&self) {
        if cfg!(test) {
            return;
        }
        let path = Prefs::path();
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(path, self.to_text());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_read_back_as_written() {
        let prefs = Prefs { size: egui::vec2(1400.0, 900.0), grid: false, unit: Unit::Inches, spacing: Spacing::Length, grid_mm: 2.5, subdivisions: 5, guides_locked: true, artwork_opacity: 0.5, stretch: Some(Measure::Angle), show_cursor: false, background_exposure: -1.5, padding: true, padding_texture: 4096, padding_px: 16.0, ..Prefs::default() };
        assert_eq!(Prefs::from_text(&prefs.to_text()), prefs);
        // Nonsense is the default, a line at a time.
        let read = Prefs::from_text("size = big\nunit = furlongs\nsubdivisions = 900\nwhatever = 3\n");
        assert_eq!(read.size, Prefs::default().size);
        assert_eq!(read.unit, Unit::Millimetres);
        assert_eq!(read.subdivisions, 20);
    }
}
