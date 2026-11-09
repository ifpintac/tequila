//! Composition guides: the lines photographers compose by, drawn over the
//! viewport's picture.
//!
//! They are drawn in the render's own frame - the render frame when it is
//! on, or where the render falls in the view when it is off
//! ([`render_region`]) - so a third is a third of the final image, and a
//! line square to a diagonal is square in it. Thin, light and half seen
//! through, on a dark edge, so they read on a white studio and a black one
//! alike. They belong to the viewport, like the grid: never in a Screenshot,
//! a render or Presentation mode.
//!
//! The geometry is pure: polylines in the frame's own 0-1 coordinates,
//! (0, 0) its top left and (1, 1) its bottom right, for the drawing to
//! stretch over the frame and for the tests to check. Which guides are
//! shown, and which way the one-sided ones face, is how the person works
//! rather than what they work on, so it is kept in `guides.txt` beside the
//! other settings, not in the project.

use crate::theme;
use eframe::egui::{self, pos2, vec2, Color32, Pos2, Rect, Stroke, Vec2};
use std::path::{Path, PathBuf};

/// The golden ratio, φ.
pub const PHI: f32 = 1.618_034;

/// How many of the golden spiral's squares are drawn: the twelfth is under
/// a hundredth of the frame, a few pixels.
const SPIRAL_SQUARES: usize = 12;

/// Points along each of the spiral's quarter arcs.
const ARC_STEPS: usize = 24;

/// Lines out from the centre for the radial guide.
const RAYS: usize = 16;

/// A composition guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Guide {
    Thirds,
    GoldenSection,
    GoldenSpiral,
    GoldenTriangle,
    HarmoniousTriangles,
    Diagonals,
    DynamicSymmetry,
    Diagonals45,
    CentreCross,
    Pyramid,
    Radial,
    SafeAreas,
}

impl Guide {
    /// Every guide, in the menu's order.
    pub const ALL: [Guide; 12] = [
        Guide::Thirds,
        Guide::GoldenSection,
        Guide::GoldenSpiral,
        Guide::GoldenTriangle,
        Guide::HarmoniousTriangles,
        Guide::Diagonals,
        Guide::DynamicSymmetry,
        Guide::Diagonals45,
        Guide::CentreCross,
        Guide::Pyramid,
        Guide::Radial,
        Guide::SafeAreas,
    ];

    /// The guide's name for scripts and the settings file.
    pub fn key(self) -> &'static str {
        match self {
            Guide::Thirds => "thirds",
            Guide::GoldenSection => "golden_section",
            Guide::GoldenSpiral => "golden_spiral",
            Guide::GoldenTriangle => "golden_triangle",
            Guide::HarmoniousTriangles => "harmonious_triangles",
            Guide::Diagonals => "diagonals",
            Guide::DynamicSymmetry => "dynamic_symmetry",
            Guide::Diagonals45 => "diagonals_45",
            Guide::CentreCross => "centre_cross",
            Guide::Pyramid => "pyramid",
            Guide::Radial => "radial",
            Guide::SafeAreas => "safe_areas",
        }
    }

    /// A guide by its name, or by another it goes by elsewhere: `rule_of_thirds`,
    /// `phi_grid` or `golden_ratio`, `spiral`, `harmony`, `center`, and so on.
    /// Case, dashes and spaces do not matter.
    pub fn from_key(key: &str) -> Option<Guide> {
        let key = key.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Some(match key.as_str() {
            "thirds" | "rule_of_thirds" => Guide::Thirds,
            "golden_section" | "golden_ratio" | "golden" | "phi_grid" | "phi" => Guide::GoldenSection,
            "golden_spiral" | "spiral" | "fibonacci_spiral" => Guide::GoldenSpiral,
            "golden_triangle" | "golden_triangles" | "triangle" => Guide::GoldenTriangle,
            "harmonious_triangles" | "harmonious_triangle" | "harmony_triangles" | "harmony_triangle" | "harmony" => {
                Guide::HarmoniousTriangles
            }
            "diagonals" | "diagonal" | "centre_diagonal" | "center_diagonal" => Guide::Diagonals,
            "dynamic_symmetry" | "armature" => Guide::DynamicSymmetry,
            "diagonals_45" | "diagonal_45" | "45" | "diagonal_method" => Guide::Diagonals45,
            "centre_cross" | "center_cross" | "centre" | "center" | "cross" => Guide::CentreCross,
            "pyramid" => Guide::Pyramid,
            "radial" => Guide::Radial,
            "safe_areas" | "safe_area" | "safe" => Guide::SafeAreas,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Guide::Thirds => "Rule of thirds",
            Guide::GoldenSection => "Golden section",
            Guide::GoldenSpiral => "Golden spiral",
            Guide::GoldenTriangle => "Golden triangle",
            Guide::HarmoniousTriangles => "Harmonious triangles",
            Guide::Diagonals => "Diagonals",
            Guide::DynamicSymmetry => "Dynamic symmetry",
            Guide::Diagonals45 => "45° diagonals",
            Guide::CentreCross => "Centre cross",
            Guide::Pyramid => "Pyramid",
            Guide::Radial => "Radial",
            Guide::SafeAreas => "Safe areas",
        }
    }

    /// What the guide is, and what it is for.
    pub fn hint(self) -> &'static str {
        match self {
            Guide::Thirds => "The frame in thirds each way: put the product on a line, or where two cross",
            Guide::GoldenSection => "The phi grid: like the thirds but nearer the middle, at 0.382 and 0.618 of the frame",
            Guide::GoldenSpiral => "The frame cut into golden squares, with the spiral through them: put what matters where it winds to",
            Guide::GoldenTriangle => "One diagonal, and a line from each other corner meeting it square: four triangles to fill or lead along",
            Guide::HarmoniousTriangles => {
                "One diagonal, and the lines square to it from the other corners carried on to the frame's edge, as Blender's harmony triangles"
            }
            Guide::Diagonals => "Corner to corner both ways: the baroque diagonal, rising to the right, and the sinister, falling",
            Guide::DynamicSymmetry => "Both diagonals, and from each corner the line square to the diagonal it is not on: the frame's armature",
            Guide::Diagonals45 => "A line at 45° from each corner, as the diagonal method draws them: for leading lines and edges",
            Guide::CentreCross => "The frame halved each way, for a symmetrical shot",
            Guide::Pyramid => "From the bottom corners to the middle of the top: a product built up as a triangle",
            Guide::Radial => "Lines out from the centre, evenly spread, for a picture that draws the eye in",
            Guide::SafeAreas => "Action safe (93%) and title safe (90%): what a crop for video or a page layout keeps",
        }
    }

    fn bit(self) -> u16 {
        1 << self as u16
    }
}

/// The corner of the frame the golden spiral winds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    /// Clockwise from the top left, the order Shift+O turns them in.
    pub const ALL: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomRight, Corner::BottomLeft];

    pub fn key(self) -> &'static str {
        match self {
            Corner::TopLeft => "top_left",
            Corner::TopRight => "top_right",
            Corner::BottomRight => "bottom_right",
            Corner::BottomLeft => "bottom_left",
        }
    }

    pub fn from_key(key: &str) -> Option<Corner> {
        let key = key.trim().to_ascii_lowercase().replace(['-', ' '], "_");
        Corner::ALL.into_iter().find(|c| c.key() == key || c.key().replace('_', "") == key)
    }

    fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "top left",
            Corner::TopRight => "top right",
            Corner::BottomRight => "bottom right",
            Corner::BottomLeft => "bottom left",
        }
    }

    /// Whether the frame is mirrored left to right, and top to bottom, to
    /// take a spiral winding to the bottom right to this corner.
    fn mirrors(self) -> (bool, bool) {
        match self {
            Corner::BottomRight => (false, false),
            Corner::BottomLeft => (true, false),
            Corner::TopRight => (false, true),
            Corner::TopLeft => (true, true),
        }
    }
}

/// Which diagonal a triangle guide is drawn on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slope {
    /// Bottom left to top right: the baroque diagonal, Blender's A.
    Rising,
    /// Top left to bottom right: the sinister diagonal, Blender's B.
    Falling,
}

impl Slope {
    pub fn key(self) -> &'static str {
        match self {
            Slope::Rising => "rising",
            Slope::Falling => "falling",
        }
    }

    pub fn from_key(key: &str) -> Option<Slope> {
        match key.trim().to_ascii_lowercase().as_str() {
            "rising" | "a" | "baroque" => Some(Slope::Rising),
            "falling" | "b" | "sinister" => Some(Slope::Falling),
            _ => None,
        }
    }

    fn other(self) -> Slope {
        match self {
            Slope::Rising => Slope::Falling,
            Slope::Falling => Slope::Rising,
        }
    }
}

/// The guides chosen, and which way the one-sided ones face.
#[derive(Clone, Debug, PartialEq)]
pub struct Guides {
    /// Shown at all (O): hiding them keeps which are chosen.
    pub on: bool,
    /// One bit for each guide chosen ([`Guide::bit`]).
    chosen: u16,
    /// The corner the golden spiral winds to.
    pub spiral: Corner,
    /// The spiral turned the other way: its first, largest square spanning
    /// the frame's long side rather than its short one.
    pub spiral_flip: bool,
    /// The golden triangle's diagonal.
    pub triangle: Slope,
    /// The harmonious triangles' diagonal.
    pub harmony: Slope,
    /// The pyramid upside down: its point in the middle of the bottom.
    pub pyramid_down: bool,
}

impl Default for Guides {
    fn default() -> Guides {
        Guides {
            on: true,
            chosen: 0,
            spiral: Corner::BottomRight,
            spiral_flip: false,
            triangle: Slope::Rising,
            harmony: Slope::Rising,
            pyramid_down: false,
        }
    }
}

impl Guides {
    pub fn is_chosen(&self, guide: Guide) -> bool {
        self.chosen & guide.bit() != 0
    }

    /// Choose a guide, or put it away. Choosing one shows the guides.
    pub fn choose(&mut self, guide: Guide, chosen: bool) {
        if chosen {
            self.chosen |= guide.bit();
            self.on = true;
        } else {
            self.chosen &= !guide.bit();
        }
    }

    /// The guides chosen, in the menu's order.
    pub fn chosen(&self) -> impl Iterator<Item = Guide> + '_ {
        Guide::ALL.into_iter().filter(|g| self.is_chosen(*g))
    }

    /// Just these guides, and shown.
    pub fn choose_only(&mut self, guides: &[Guide]) {
        self.chosen = guides.iter().fold(0, |bits, g| bits | g.bit());
        self.on = true;
    }

    /// Anything to draw.
    pub fn visible(&self) -> bool {
        self.on && self.chosen != 0
    }

    /// O: the guides shown or hidden; with none chosen yet, the rule of
    /// thirds, the one most people want first.
    pub fn toggle(&mut self) {
        if self.chosen == 0 {
            self.choose(Guide::Thirds, true);
        } else {
            self.on = !self.on;
        }
    }

    /// Shift+O: every one-sided guide shown turned to its next way round -
    /// the spiral clockwise to the next corner, and after the fourth to the
    /// other way of turning; the triangles to the other diagonal; the
    /// pyramid over. False, and nothing turned, when none is shown.
    pub fn turn(&mut self) -> bool {
        if !self.on {
            return false;
        }
        let mut turned = false;
        if self.is_chosen(Guide::GoldenSpiral) {
            let at = Corner::ALL.iter().position(|c| *c == self.spiral).unwrap_or(0);
            if at + 1 == Corner::ALL.len() {
                self.spiral_flip = !self.spiral_flip;
            }
            self.spiral = Corner::ALL[(at + 1) % Corner::ALL.len()];
            turned = true;
        }
        if self.is_chosen(Guide::GoldenTriangle) {
            self.triangle = self.triangle.other();
            turned = true;
        }
        if self.is_chosen(Guide::HarmoniousTriangles) {
            self.harmony = self.harmony.other();
            turned = true;
        }
        if self.is_chosen(Guide::Pyramid) {
            self.pyramid_down = !self.pyramid_down;
            turned = true;
        }
        turned
    }

    /// Every chosen guide's lines in a frame `aspect` wide over high.
    pub fn lines(&self, aspect: f32) -> Vec<Vec<Pos2>> {
        self.chosen().flat_map(|g| lines(g, self, aspect)).collect()
    }

    /// Where the choice is kept: `guides.txt`, beside the other settings.
    pub fn file() -> PathBuf {
        tequila_format::library::settings_directory().join("guides.txt")
    }

    fn from_text(text: &str) -> Guides {
        let mut out = Guides::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "on" => out.on = value != "no",
                "show" => {
                    let guides: Vec<Guide> = value.split(',').filter_map(Guide::from_key).collect();
                    out.chosen = guides.iter().fold(0, |bits, g| bits | g.bit());
                }
                "spiral" => out.spiral = Corner::from_key(value).unwrap_or(out.spiral),
                "spiral_flip" => out.spiral_flip = value == "yes",
                "golden_triangle" => out.triangle = Slope::from_key(value).unwrap_or(out.triangle),
                "harmonious_triangles" => out.harmony = Slope::from_key(value).unwrap_or(out.harmony),
                "pyramid" => out.pyramid_down = value == "down",
                _ => {}
            }
        }
        out
    }

    fn to_text(&self) -> String {
        let yes = |b: bool| if b { "yes" } else { "no" };
        let shown: Vec<&str> = self.chosen().map(Guide::key).collect();
        format!(
            "on={}\nshow={}\nspiral={}\nspiral_flip={}\ngolden_triangle={}\nharmonious_triangles={}\npyramid={}\n",
            yes(self.on),
            shown.join(","),
            self.spiral.key(),
            yes(self.spiral_flip),
            self.triangle.key(),
            self.harmony.key(),
            if self.pyramid_down { "down" } else { "up" },
        )
    }

    /// The choice kept in `file`, or none chosen if there is none yet.
    pub fn load(file: &Path) -> Guides {
        std::fs::read_to_string(file).map(|t| Guides::from_text(&t)).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> Result<(), String> {
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(file, self.to_text()).map_err(|e| format!("{}: {e}", file.display()))
    }
}

/// One guide's lines, as polylines in the frame's 0-1 coordinates, for a
/// frame `aspect` wide over high: what is square or at 45° is so in the
/// picture, not in the 0-1 square.
pub fn lines(guide: Guide, guides: &Guides, aspect: f32) -> Vec<Vec<Pos2>> {
    let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    match guide {
        Guide::Thirds => grid(&[1.0 / 3.0, 2.0 / 3.0]),
        Guide::GoldenSection => grid(&[1.0 / (PHI * PHI), 1.0 / PHI]),
        Guide::GoldenSpiral => {
            let spiral = golden_spiral(guides.spiral, guides.spiral_flip, aspect);
            spiral.cuts.into_iter().map(|c| c.to_vec()).chain(spiral.arcs).collect()
        }
        Guide::GoldenTriangle => triangle(guides.triangle, aspect, false),
        Guide::HarmoniousTriangles => triangle(guides.harmony, aspect, true),
        Guide::Diagonals => vec![vec![pos2(0.0, 1.0), pos2(1.0, 0.0)], vec![pos2(0.0, 0.0), pos2(1.0, 1.0)]],
        Guide::DynamicSymmetry => {
            let mut out = triangle(Slope::Rising, aspect, true);
            out.extend(triangle(Slope::Falling, aspect, true));
            out
        }
        Guide::Diagonals45 => {
            let corners = [(pos2(0.0, 0.0), vec2(1.0, 1.0)), (pos2(1.0, 0.0), vec2(-1.0, 1.0)), (pos2(1.0, 1.0), vec2(-1.0, -1.0)), (pos2(0.0, 1.0), vec2(1.0, -1.0))];
            corners.into_iter().map(|(from, towards)| vec![from, to_edge(from, towards, aspect)]).collect()
        }
        Guide::CentreCross => grid(&[0.5]),
        Guide::Pyramid => {
            let (base, point) = if guides.pyramid_down { (0.0, 1.0) } else { (1.0, 0.0) };
            vec![vec![pos2(0.0, base), pos2(0.5, point), pos2(1.0, base)]]
        }
        Guide::Radial => (0..RAYS)
            .map(|k| {
                let angle = k as f32 * std::f32::consts::TAU / RAYS as f32;
                let centre = pos2(0.5, 0.5);
                vec![centre, to_edge(centre, vec2(angle.cos(), angle.sin()), aspect)]
            })
            .collect(),
        Guide::SafeAreas => [0.93_f32, 0.90]
            .into_iter()
            .map(|size| {
                let (a, b) = ((1.0 - size) * 0.5, (1.0 + size) * 0.5);
                vec![pos2(a, a), pos2(b, a), pos2(b, b), pos2(a, b), pos2(a, a)]
            })
            .collect(),
    }
}

/// Lines across and down the frame at each of `at`.
fn grid(at: &[f32]) -> Vec<Vec<Pos2>> {
    at.iter().flat_map(|&t| [vec![pos2(t, 0.0), pos2(t, 1.0)], vec![pos2(0.0, t), pos2(1.0, t)]]).collect()
}

/// A diagonal, and from each of the other two corners the line square to it
/// in the picture: to the diagonal (the golden triangle), or `carried` on
/// to the frame's far edge (the harmonious triangles, and the reciprocals of
/// dynamic symmetry). The diagonal first.
pub fn triangle(slope: Slope, aspect: f32, carried: bool) -> Vec<Vec<Pos2>> {
    let (from, to, others) = match slope {
        Slope::Rising => (pos2(0.0, 1.0), pos2(1.0, 0.0), [pos2(0.0, 0.0), pos2(1.0, 1.0)]),
        Slope::Falling => (pos2(0.0, 0.0), pos2(1.0, 1.0), [pos2(1.0, 0.0), pos2(0.0, 1.0)]),
    };
    let mut out = vec![vec![from, to]];
    for corner in others {
        let foot = foot(corner, from, to, aspect);
        let end = if carried { to_edge(corner, picture(foot, aspect) - picture(corner, aspect), aspect) } else { foot };
        out.push(vec![corner, end]);
    }
    out
}

/// A point of the frame in the picture's own proportions: 0 to `aspect`
/// across, 0 to 1 down.
fn picture(p: Pos2, aspect: f32) -> Pos2 {
    pos2(p.x * aspect, p.y)
}

/// Where the line square to `a`-`b` through `p` meets it, in the picture.
fn foot(p: Pos2, a: Pos2, b: Pos2, aspect: f32) -> Pos2 {
    let (p, a, b) = (picture(p, aspect), picture(a, aspect), picture(b, aspect));
    let along = b - a;
    let t = (p - a).dot(along) / along.length_sq();
    let at = a + along * t;
    pos2(at.x / aspect, at.y)
}

/// Where a line from `from` going `towards` (in the picture's proportions)
/// leaves the frame.
fn to_edge(from: Pos2, towards: Vec2, aspect: f32) -> Pos2 {
    let start = picture(from, aspect);
    let mut t = f32::INFINITY;
    for (at, d, low, high) in [(start.x, towards.x, 0.0, aspect), (start.y, towards.y, 0.0, 1.0)] {
        if d > 1e-6 {
            t = t.min((high - at) / d);
        } else if d < -1e-6 {
            t = t.min((low - at) / d);
        }
    }
    if !t.is_finite() {
        return from;
    }
    let end = start + towards * t;
    pos2((end.x / aspect).clamp(0.0, 1.0), end.y.clamp(0.0, 1.0))
}

/// The golden spiral and the golden rectangle's squares it runs through,
/// stretched over the frame as photo editors draw it.
#[derive(Clone, Debug)]
pub struct Spiral {
    /// The squares, largest first.
    pub squares: Vec<Rect>,
    /// Each square's edge with what is left of the frame: the lines drawn.
    pub cuts: Vec<[Pos2; 2]>,
    /// A quarter arc in each square, from one corner of it to the opposite
    /// one, each starting where the last ended.
    pub arcs: Vec<Vec<Pos2>>,
    /// The point it winds in to.
    pub eye: Pos2,
}

/// The golden spiral winding to `corner` in a frame `aspect` wide over
/// high. Its first, largest square spans the frame's short side - the
/// whole height of a wide frame, the whole width of a tall one - unless
/// `flip`ped to span the long side.
pub fn golden_spiral(corner: Corner, flip: bool, aspect: f32) -> Spiral {
    // Built in a golden rectangle φ wide and 1 high, winding to its bottom
    // right: a square off the left, then the top, the right, the bottom, and
    // round again, each arc centred on the corner of its square nearest the
    // eye. In f64, so the twelfth square is as exact as the first.
    let phi = (1.0 + 5.0_f64.sqrt()) * 0.5;
    let (mut x0, mut y0, mut x1, mut y1) = (0.0_f64, 0.0_f64, phi, 1.0_f64);
    let transpose = flip != (aspect < 1.0);
    let (mirror_x, mirror_y) = corner.mirrors();
    let place = |(x, y): (f64, f64)| {
        let (u, v) = (x / phi, y);
        let (u, v) = if transpose { (v, u) } else { (u, v) };
        pos2((if mirror_x { 1.0 - u } else { u }) as f32, (if mirror_y { 1.0 - v } else { v }) as f32)
    };
    let mut out = Spiral { squares: Vec::new(), cuts: Vec::new(), arcs: Vec::new(), eye: Pos2::ZERO };
    for step in 0..64 {
        let side = (x1 - x0).min(y1 - y0);
        // The square's corners, its cut, and its arc's start, end and centre.
        let (square, cut, start, end, centre) = match step % 4 {
            0 => {
                let out = ([(x0, y0), (x0 + side, y1)], [(x0 + side, y0), (x0 + side, y1)], (x0, y1), (x0 + side, y0), (x0 + side, y1));
                x0 += side;
                out
            }
            1 => {
                let out = ([(x0, y0), (x1, y0 + side)], [(x0, y0 + side), (x1, y0 + side)], (x0, y0), (x1, y0 + side), (x0, y0 + side));
                y0 += side;
                out
            }
            2 => {
                let out = ([(x1 - side, y0), (x1, y1)], [(x1 - side, y0), (x1 - side, y1)], (x1, y0), (x1 - side, y1), (x1 - side, y0));
                x1 -= side;
                out
            }
            _ => {
                let out = ([(x0, y1 - side), (x1, y1)], [(x0, y1 - side), (x1, y1 - side)], (x1, y1), (x0, y1 - side), (x1, y1 - side));
                y1 -= side;
                out
            }
        };
        if step >= SPIRAL_SQUARES {
            continue;
        }
        out.squares.push(Rect::from_two_pos(place(square[0]), place(square[1])));
        out.cuts.push([place(cut[0]), place(cut[1])]);
        let angle = |(x, y): (f64, f64)| (y - centre.1).atan2(x - centre.0);
        let from = angle(start);
        let mut sweep = angle(end) - from;
        if sweep > std::f64::consts::PI {
            sweep -= std::f64::consts::TAU;
        } else if sweep < -std::f64::consts::PI {
            sweep += std::f64::consts::TAU;
        }
        out.arcs.push(
            (0..=ARC_STEPS)
                .map(|i| {
                    let a = from + sweep * i as f64 / ARC_STEPS as f64;
                    place((centre.0 + side * a.cos(), centre.1 + side * a.sin()))
                })
                .collect(),
        );
    }
    // What is left after 64 squares is far under a pixel.
    out.eye = place(((x0 + x1) * 0.5, (y0 + y1) * 0.5));
    out
}

/// What a camera keeps of its picture when the image changes shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Keeps {
    /// Its width: a perspective camera keeps its horizontal field of view
    /// (`Camera::sensor_height_mm`). A shift lens's rise, in fractions of
    /// the film's height, moves what the height takes in.
    Width { shift_y: f32 },
    /// Its height: an orthographic camera's `scale` is its height.
    Height,
    /// All of it, stretched: a 360° picture.
    Whole,
}

impl Keeps {
    pub fn of(projection: &tequila_render_core::camera::Projection) -> Keeps {
        use tequila_render_core::camera::Projection;
        match *projection {
            Projection::Perspective { .. } => Keeps::Width { shift_y: 0.0 },
            Projection::Shift { shift_y, .. } => Keeps::Width { shift_y },
            Projection::Orthographic { .. } => Keeps::Height,
            Projection::Equirectangular => Keeps::Whole,
        }
    }
}

/// Where a render `aspect` wide over high falls in a `view` of the same
/// camera drawn at the view's own shape: the render frame's rectangle when
/// the frame is off. It may reach past the view's top and bottom (or its
/// sides), where the render holds what the view does not show.
pub fn render_region(view: Rect, aspect: f32, keeps: Keeps) -> Rect {
    if !(aspect > 0.0) || view.width() < 1.0 || view.height() < 1.0 {
        return view;
    }
    let view_aspect = view.width() / view.height();
    match keeps {
        Keeps::Whole => view,
        Keeps::Height => Rect::from_center_size(view.center(), vec2(view.height() * aspect, view.height())),
        Keeps::Width { shift_y } => {
            // The render's height, in the view's: the film's half-height is
            // its half-width over its shape, and the half-width is the same
            // in both. A shift lens offsets the film by a share of its
            // height, so it moves the render's middle off the view's.
            let k = view_aspect / aspect;
            let middle = 0.5 - shift_y * (k - 1.0);
            Rect::from_center_size(pos2(view.center().x, view.top() + middle * view.height()), vec2(view.width(), view.height() * k))
        }
    }
}

/// The guides over `frame`, the picture's rectangle in the render's shape,
/// clipped to what `painter` paints in.
pub fn draw(painter: &egui::Painter, frame: Rect, guides: &Guides) {
    if !guides.visible() || frame.width() < 8.0 || frame.height() < 8.0 {
        return;
    }
    let lines = guides.lines(frame.width() / frame.height());
    let place = |p: &Pos2| frame.min + vec2(p.x * frame.width(), p.y * frame.height());
    // Every dark edge first, then every light line over them, so where two
    // guides cross neither cuts the other.
    for stroke in [Stroke::new(3.0_f32, Color32::from_black_alpha(64)), Stroke::new(1.0_f32, Color32::from_white_alpha(150))] {
        for line in &lines {
            painter.add(egui::Shape::line(line.iter().map(place).collect(), stroke));
        }
    }
}

/// The guides' menu, for View › Composition guides and the viewport's
/// toolbar: each guide, and under a chosen one-sided one its ways round,
/// drawn as they will be at the render's `aspect`. True when anything
/// changed.
pub fn menu(ui: &mut egui::Ui, guides: &mut Guides, aspect: f32) -> bool {
    let p = theme::p();
    let mut changed = false;
    let mut on = guides.on;
    if ui
        .add(crate::field::Checkbox::new(&mut on, "Show guides"))
        .on_hover_text(format!("{}: show or hide the guides chosen below", theme::keys("O")))
        .changed()
    {
        guides.on = on;
        changed = true;
    }
    ui.separator();
    for guide in Guide::ALL {
        let mut chosen = guides.is_chosen(guide);
        if ui.add(crate::field::Checkbox::new(&mut chosen, guide.label())).on_hover_text(guide.hint()).changed() {
            guides.choose(guide, chosen);
            changed = true;
        }
        if chosen {
            changed |= ways_round(ui, guides, guide, aspect);
        }
    }
    ui.separator();
    if ui.add_enabled(guides.chosen != 0, egui::Button::new("Clear all")).clicked() {
        guides.chosen = 0;
        changed = true;
    }
    // Short lines, unwrapped: a menu's wrapped text spreads its words.
    for line in [format!("{} turns the one-sided guides", theme::keys("Shift + O")), "Never in screenshots or renders".to_string()] {
        ui.add(egui::Label::new(egui::RichText::new(line).size(11.0).color(p.text_faint)).wrap_mode(egui::TextWrapMode::Extend));
    }
    changed
}

/// The ways round of a one-sided guide, each a small picture of it to
/// click; nothing for the others.
fn ways_round(ui: &mut egui::Ui, guides: &mut Guides, guide: Guide, aspect: f32) -> bool {
    // The pictures at the render's shape, within reason.
    let aspect = aspect.clamp(0.7, 2.0);
    let with = |change: &dyn Fn(&mut Guides)| {
        let mut choice = guides.clone();
        change(&mut choice);
        choice
    };
    let rows: Vec<Vec<(Guides, String)>> = match guide {
        Guide::GoldenSpiral => [false, true]
            .into_iter()
            .map(|flip| {
                Corner::ALL
                    .into_iter()
                    .map(|corner| {
                        let tip = format!("Winds to the {}{}", corner.label(), if flip { ", turned the other way" } else { "" });
                        (with(&|g| (g.spiral, g.spiral_flip) = (corner, flip)), tip)
                    })
                    .collect()
            })
            .collect(),
        Guide::GoldenTriangle | Guide::HarmoniousTriangles => vec![[(Slope::Rising, "On the diagonal rising to the right (A)"), (Slope::Falling, "On the diagonal falling to the right (B)")]
            .into_iter()
            .map(|(slope, tip)| {
                let choice = with(&|g| if guide == Guide::GoldenTriangle { g.triangle = slope } else { g.harmony = slope });
                (choice, tip.to_string())
            })
            .collect()],
        Guide::Pyramid => vec![[(false, "Point up"), (true, "Point down")].into_iter().map(|(down, tip)| (with(&|g| g.pyramid_down = down), tip.to_string())).collect()],
        _ => Vec::new(),
    };
    let mut changed = false;
    for choices in rows {
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.spacing_mut().item_spacing.x = 4.0;
            for (choice, tip) in choices {
                let selected = choice == *guides;
                if thumbnail(ui, selected, &lines(guide, &choice, aspect), aspect).on_hover_text(tip).clicked() && !selected {
                    *guides = choice;
                    changed = true;
                }
            }
        });
    }
    changed
}

/// A small frame with a guide's `lines` in it, lit when `selected`.
fn thumbnail(ui: &mut egui::Ui, selected: bool, lines: &[Vec<Pos2>], aspect: f32) -> egui::Response {
    let p = theme::p();
    let height = 20.0;
    let (rect, response) = ui.allocate_exact_size(vec2(height * aspect + 8.0, height + 8.0), egui::Sense::click());
    let painter = ui.painter();
    let fill = if selected {
        p.accent_fill
    } else if response.hovered() {
        p.hover
    } else {
        Color32::TRANSPARENT
    };
    painter.rect(rect, 4.0, fill, Stroke::new(1.0_f32, if selected { p.accent } else { Color32::TRANSPARENT }));
    let frame = rect.shrink(4.0);
    painter.rect_stroke(frame, 0.0, Stroke::new(1.0_f32, p.line_strong));
    let ink = Stroke::new(1.0_f32, if selected { p.accent_text } else { p.text_dim });
    for line in lines {
        let points = line.iter().map(|q| frame.min + vec2(q.x * frame.width(), q.y * frame.height())).collect();
        painter.add(egui::Shape::line(points, ink));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shapes people render at: wide, the old 4:3, square, a portrait page
    /// and a phone's.
    const ASPECTS: [f32; 6] = [16.0 / 9.0, 4.0 / 3.0, 1.0, 4.0 / 5.0, 9.0 / 16.0, 2.39];

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    fn near(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-4
    }

    /// Square in the picture: the dot product of two directions measured
    /// in its own proportions.
    fn square_in_picture(a: (Pos2, Pos2), b: (Pos2, Pos2), aspect: f32) -> bool {
        let u = (picture(a.1, aspect) - picture(a.0, aspect)).normalized();
        let v = (picture(b.1, aspect) - picture(b.0, aspect)).normalized();
        u.dot(v).abs() < 1e-4
    }

    fn every_way() -> Vec<Guides> {
        let mut out = Vec::new();
        for corner in Corner::ALL {
            for flip in [false, true] {
                for slope in [Slope::Rising, Slope::Falling] {
                    for down in [false, true] {
                        let mut g = Guides { spiral: corner, spiral_flip: flip, triangle: slope, harmony: slope.other(), pyramid_down: down, ..Guides::default() };
                        g.choose_only(&Guide::ALL);
                        out.push(g);
                    }
                }
            }
        }
        out
    }

    #[test]
    fn the_thirds_are_at_a_third_and_two_thirds() {
        let lines = lines(Guide::Thirds, &Guides::default(), 16.0 / 9.0);
        assert_eq!(lines.len(), 4);
        let mut down: Vec<f32> = lines.iter().filter(|l| l[0].x == l[1].x).map(|l| l[0].x).collect();
        let mut across: Vec<f32> = lines.iter().filter(|l| l[0].y == l[1].y).map(|l| l[0].y).collect();
        down.sort_by(f32::total_cmp);
        across.sort_by(f32::total_cmp);
        for at in [down, across] {
            assert!(close(at[0], 1.0 / 3.0) && close(at[1], 2.0 / 3.0), "{at:?}");
        }
        // Each runs the frame's whole length.
        for line in &lines {
            assert!(close((line[1] - line[0]).length(), 1.0), "{line:?}");
        }
    }

    /// The phi grid: 1/φ² from each edge, which is 1/φ from the other.
    #[test]
    fn the_golden_section_is_at_one_over_phi_squared() {
        let lines = lines(Guide::GoldenSection, &Guides::default(), 1.5);
        let mut down: Vec<f32> = lines.iter().filter(|l| l[0].x == l[1].x).map(|l| l[0].x).collect();
        down.sort_by(f32::total_cmp);
        assert_eq!(down.len(), 2);
        assert!(close(down[0], 0.381_966), "{down:?}");
        assert!(close(down[1], 0.618_034), "{down:?}");
        assert!(close(down[0] + down[1], 1.0));
        assert!(close(down[1] / down[0], PHI), "the long part over the short is φ");
        assert!(close(PHI * PHI, PHI + 1.0));
    }

    /// The spiral in a golden frame: squares each 1/φ of the last, a
    /// quarter arc across each from corner to opposite corner, every arc
    /// starting where the last ended and their ends closing in on the eye,
    /// which is where the frame's diagonal crosses the next rectangle's.
    #[test]
    fn the_golden_spirals_squares_and_arcs_close_in_on_its_eye() {
        for aspect in ASPECTS.into_iter().chain([PHI, 1.0 / PHI]) {
            for corner in Corner::ALL {
                for flip in [false, true] {
                    let spiral = golden_spiral(corner, flip, aspect);
                    let transposed = flip != (aspect < 1.0);
                    assert_eq!(spiral.squares.len(), SPIRAL_SQUARES);
                    assert_eq!(spiral.arcs.len(), SPIRAL_SQUARES);
                    for (k, square) in spiral.squares.iter().enumerate() {
                        // Square in the golden rectangle stretched over the
                        // frame: its width over its height is the frame's
                        // stretch, 1/φ one way round and φ the other.
                        let want = if transposed { PHI } else { 1.0 / PHI };
                        let stretch = square.width() / square.height();
                        assert!((stretch / want - 1.0).abs() < 1e-3, "{corner:?} {flip} {aspect}: square {k} is {square:?}");
                        if k > 0 {
                            let last = spiral.squares[k - 1];
                            let shrink = (square.area() / last.area()).sqrt();
                            assert!((shrink * PHI - 1.0).abs() < 1e-3, "square {k} is not 1/φ the size of the last");
                        }
                        let arc = &spiral.arcs[k];
                        let corners = [square.left_top(), square.right_top(), square.right_bottom(), square.left_bottom()];
                        let (start, end) = (arc[0], *arc.last().unwrap());
                        assert!(corners.iter().any(|c| near(*c, start)) && corners.iter().any(|c| near(*c, end)), "arc {k} does not run corner to corner");
                        assert!(close((end - start).x.abs(), square.width()) && close((end - start).y.abs(), square.height()), "arc {k} does not cross its square");
                        assert!(arc.iter().all(|q| square.expand(1e-4).contains(*q)), "arc {k} leaves its square");
                        if k > 0 {
                            assert!(near(*spiral.arcs[k - 1].last().unwrap(), start), "arc {k} does not start where arc {} ended", k - 1);
                        }
                    }
                    // Closing in: each arc's end 1/φ as far from the eye as
                    // the one before, the last within a hundredth.
                    let ends: Vec<f32> = spiral.arcs.iter().map(|a| (*a.last().unwrap() - spiral.eye).length()).collect();
                    for pair in ends.windows(2) {
                        assert!(pair[1] < pair[0], "{ends:?}");
                    }
                    assert!(*ends.last().unwrap() < 0.01, "{ends:?}");
                    // The eye: φ²/(1 + φ²) of the way to its corner, on the
                    // frame's diagonal through that corner.
                    let along = PHI * PHI / (1.0 + PHI * PHI);
                    let (mirror_x, mirror_y) = corner.mirrors();
                    let want = pos2(if mirror_x { 1.0 - along } else { along }, if mirror_y { 1.0 - along } else { along });
                    assert!(near(spiral.eye, want), "{corner:?} {flip} {aspect}: the eye is at {:?}, not {want:?}", spiral.eye);
                }
            }
        }
    }

    /// In a wide frame the first square spans the frame's height; flipped,
    /// its width; a tall frame the other way about.
    #[test]
    fn the_spirals_first_square_spans_the_short_side_unless_flipped() {
        let wide = golden_spiral(Corner::BottomRight, false, 16.0 / 9.0).squares[0];
        assert!(close(wide.height(), 1.0) && close(wide.width(), 1.0 / PHI), "{wide:?}");
        let flipped = golden_spiral(Corner::BottomRight, true, 16.0 / 9.0).squares[0];
        assert!(close(flipped.width(), 1.0) && close(flipped.height(), 1.0 / PHI), "{flipped:?}");
        let tall = golden_spiral(Corner::TopLeft, false, 4.0 / 5.0).squares[0];
        assert!(close(tall.width(), 1.0) && close(tall.height(), 1.0 / PHI), "{tall:?}");
        // Winding to the right, the first square is on the left.
        assert!(close(wide.left(), 0.0));
        assert!(close(golden_spiral(Corner::TopLeft, false, 16.0 / 9.0).squares[0].right(), 1.0));
    }

    /// The golden triangle: from each other corner, a line that meets the
    /// diagonal square to it in the picture - not in the 0-1 square, which
    /// a wide frame stretches.
    #[test]
    fn the_golden_triangles_lines_meet_the_diagonal_square_to_it() {
        for aspect in ASPECTS {
            for slope in [Slope::Rising, Slope::Falling] {
                let lines = triangle(slope, aspect, false);
                assert_eq!(lines.len(), 3);
                let diagonal = (lines[0][0], lines[0][1]);
                for line in &lines[1..] {
                    let (corner, foot) = (line[0], line[1]);
                    assert!(square_in_picture(diagonal, (corner, foot), aspect), "{slope:?} at {aspect}: {line:?}");
                    // On the diagonal: the foot divides it.
                    let a = picture(diagonal.0, aspect);
                    let b = picture(diagonal.1, aspect);
                    let f = picture(foot, aspect);
                    assert!(close((f - a).length() + (b - f).length(), (b - a).length()), "{slope:?} at {aspect}: {foot:?} is off the diagonal");
                    assert!([0.0, 1.0].contains(&corner.x) && [0.0, 1.0].contains(&corner.y), "{corner:?} is not a corner");
                }
            }
        }
        // In a square frame, both meet it in the middle.
        for line in &triangle(Slope::Rising, 1.0, false)[1..] {
            assert!(near(line[1], pos2(0.5, 0.5)));
        }
        // Rising runs bottom left to top right.
        assert_eq!(triangle(Slope::Rising, 1.5, false)[0], vec![pos2(0.0, 1.0), pos2(1.0, 0.0)]);
    }

    /// The harmonious triangles carry the same lines on to the frame's
    /// edge: in a wide frame, the bottom at 1/aspect² from the left, where
    /// Blender's harmony triangle meets it.
    #[test]
    fn the_harmonious_triangles_carry_on_to_the_edge() {
        for aspect in ASPECTS {
            for slope in [Slope::Rising, Slope::Falling] {
                let golden = triangle(slope, aspect, false);
                let carried = triangle(slope, aspect, true);
                for (short, long) in golden[1..].iter().zip(&carried[1..]) {
                    assert_eq!(short[0], long[0], "from the same corner");
                    assert!(square_in_picture((carried[0][0], carried[0][1]), (long[0], long[1]), aspect));
                    let end = long[1];
                    assert!([end.x, end.y].iter().any(|v| close(*v, 0.0) || close(*v, 1.0)), "{end:?} is not on the edge");
                    assert!((long[1] - long[0]).length() >= (short[1] - short[0]).length() - 1e-5, "it stops short of the diagonal");
                }
            }
        }
        let wide = 16.0 / 9.0;
        let from_top_left = &triangle(Slope::Rising, wide, true)[1];
        assert!(near(from_top_left[1], pos2(1.0 / (wide * wide), 1.0)), "{from_top_left:?}");
        // A tall frame's meet the side instead.
        let tall = 4.0 / 5.0;
        let from_top_left = &triangle(Slope::Rising, tall, true)[1];
        assert!(near(from_top_left[1], pos2(1.0, tall * tall)), "{from_top_left:?}");
    }

    /// Dynamic symmetry: both diagonals, each with its two reciprocals.
    #[test]
    fn dynamic_symmetry_is_both_diagonals_and_their_reciprocals() {
        let aspect = 3.0 / 2.0;
        let lines = lines(Guide::DynamicSymmetry, &Guides::default(), aspect);
        assert_eq!(lines.len(), 6);
        let rising = (pos2(0.0, 1.0), pos2(1.0, 0.0));
        let falling = (pos2(0.0, 0.0), pos2(1.0, 1.0));
        let reciprocals: Vec<_> = lines.iter().filter(|l| l.len() == 2 && !(near(l[0], rising.0) && near(l[1], rising.1)) && !(near(l[0], falling.0) && near(l[1], falling.1))).collect();
        assert_eq!(reciprocals.len(), 4);
        for r in reciprocals {
            let line = (r[0], r[1]);
            assert!(square_in_picture(line, rising, aspect) || square_in_picture(line, falling, aspect), "{r:?}");
        }
    }

    /// 45° in the picture, from each corner to the edge across from it.
    #[test]
    fn the_45_degree_lines_are_45_degrees_in_the_picture() {
        for aspect in ASPECTS {
            let lines = lines(Guide::Diagonals45, &Guides::default(), aspect);
            assert_eq!(lines.len(), 4);
            for line in &lines {
                let d = picture(line[1], aspect) - picture(line[0], aspect);
                assert!(close(d.x.abs(), d.y.abs()), "{line:?} at {aspect}");
                assert!(d.length() > 0.5);
            }
        }
    }

    /// Sixteen lines from the centre, evenly spread in the picture, each to
    /// the edge.
    #[test]
    fn the_radial_lines_spread_evenly_from_the_centre() {
        let aspect = 16.0 / 9.0;
        let lines = lines(Guide::Radial, &Guides::default(), aspect);
        assert_eq!(lines.len(), RAYS);
        for (k, line) in lines.iter().enumerate() {
            assert_eq!(line[0], pos2(0.5, 0.5));
            let end = line[1];
            assert!([end.x, end.y].iter().any(|v| close(*v, 0.0) || close(*v, 1.0)), "{end:?} is not on the edge");
            let angle = (picture(end, aspect) - picture(line[0], aspect)).angle().rem_euclid(std::f32::consts::TAU);
            assert!(close(angle, k as f32 * std::f32::consts::TAU / RAYS as f32), "ray {k} at {angle}");
        }
    }

    #[test]
    fn the_pyramid_points_up_or_down_and_the_centre_cross_halves() {
        let up = lines(Guide::Pyramid, &Guides::default(), 1.5);
        assert_eq!(up, vec![vec![pos2(0.0, 1.0), pos2(0.5, 0.0), pos2(1.0, 1.0)]]);
        let down = lines(Guide::Pyramid, &Guides { pyramid_down: true, ..Guides::default() }, 1.5);
        assert_eq!(down[0][1], pos2(0.5, 1.0));
        let cross = lines(Guide::CentreCross, &Guides::default(), 1.5);
        assert_eq!(cross, vec![vec![pos2(0.5, 0.0), pos2(0.5, 1.0)], vec![pos2(0.0, 0.5), pos2(1.0, 0.5)]]);
        let safe = lines(Guide::SafeAreas, &Guides::default(), 1.5);
        assert!(close(safe[0][2].x - safe[0][0].x, 0.93) && close(safe[1][2].y - safe[1][0].y, 0.90));
    }

    /// Whatever the frame's shape and whichever way round, every line is
    /// inside the frame and goes somewhere.
    #[test]
    fn every_guide_stays_in_the_frame() {
        for aspect in ASPECTS {
            for guides in every_way() {
                for guide in Guide::ALL {
                    let lines = lines(guide, &guides, aspect);
                    assert!(!lines.is_empty(), "{guide:?}");
                    for line in lines {
                        assert!(line.len() >= 2, "{guide:?}");
                        assert!(line.iter().all(|q| (-1e-4..=1.0 + 1e-4).contains(&q.x) && (-1e-4..=1.0 + 1e-4).contains(&q.y)), "{guide:?} at {aspect}: {line:?}");
                        let length: f32 = line.windows(2).map(|w| (w[1] - w[0]).length()).sum();
                        assert!(length > 1e-3, "{guide:?} at {aspect}: a line of no length");
                    }
                }
            }
        }
    }

    #[test]
    fn guides_go_by_their_names_and_the_other_names_they_have() {
        for guide in Guide::ALL {
            assert_eq!(Guide::from_key(guide.key()), Some(guide));
        }
        assert_eq!(Guide::from_key("Rule of thirds"), Some(Guide::Thirds));
        assert_eq!(Guide::from_key("phi-grid"), Some(Guide::GoldenSection));
        assert_eq!(Guide::from_key("center"), Some(Guide::CentreCross));
        assert_eq!(Guide::from_key("harmony"), Some(Guide::HarmoniousTriangles));
        assert_eq!(Guide::from_key("nonsense"), None);
        for corner in Corner::ALL {
            assert_eq!(Corner::from_key(corner.key()), Some(corner));
        }
        assert_eq!(Corner::from_key("Top-Left"), Some(Corner::TopLeft));
        assert_eq!(Corner::from_key("bottomright"), Some(Corner::BottomRight));
        assert_eq!(Slope::from_key("B"), Some(Slope::Falling));
    }

    /// O shows the rule of thirds to start with, then hides and shows what
    /// is chosen; Shift+O turns the spiral through all eight ways round.
    #[test]
    fn o_shows_and_hides_and_shift_o_turns() {
        let mut guides = Guides::default();
        assert!(!guides.visible());
        guides.toggle();
        assert!(guides.visible() && guides.is_chosen(Guide::Thirds));
        guides.toggle();
        assert!(!guides.visible() && guides.is_chosen(Guide::Thirds), "hidden, still chosen");
        guides.toggle();
        assert!(!guides.turn(), "nothing one-sided to turn");

        guides.choose(Guide::GoldenSpiral, true);
        let mut seen = Vec::new();
        for _ in 0..8 {
            seen.push((guides.spiral, guides.spiral_flip));
            assert!(guides.turn());
        }
        assert_eq!((guides.spiral, guides.spiral_flip), seen[0], "round again after eight");
        seen.sort_by_key(|(c, f)| (c.key(), *f));
        seen.dedup();
        assert_eq!(seen.len(), 8, "every corner, both ways");

        guides.choose(Guide::GoldenTriangle, true);
        let before = guides.triangle;
        guides.turn();
        assert_eq!(guides.triangle, before.other());
    }

    /// The choice is kept and read back; what is not understood is passed
    /// over.
    #[test]
    fn the_choice_is_kept_and_read_back() {
        let mut guides = Guides { spiral: Corner::TopLeft, spiral_flip: true, triangle: Slope::Falling, harmony: Slope::Falling, pyramid_down: true, ..Guides::default() };
        guides.choose_only(&[Guide::Thirds, Guide::GoldenSpiral, Guide::SafeAreas]);
        guides.on = false;
        assert_eq!(Guides::from_text(&guides.to_text()), guides);
        let file = std::env::temp_dir().join(format!("tequila-guides-{}", std::process::id())).join("guides.txt");
        guides.save(&file).unwrap();
        assert_eq!(Guides::load(&file), guides, "kept in its file, the folder made");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
        assert_eq!(Guides::load(&file), Guides::default(), "no file, none chosen");
        assert_eq!(Guides::from_text(""), Guides::default());
        let odd = Guides::from_text("show=thirds,wobbly,radial\nspiral=sideways\nnonsense\n");
        assert_eq!(odd.chosen().collect::<Vec<_>>(), vec![Guide::Thirds, Guide::Radial]);
        assert_eq!(odd.spiral, Guides::default().spiral);
    }

    /// With the render frame off, the guides go where the render falls in
    /// the view: a point at a place in the render is at that place of the
    /// region, for a camera that keeps its width, one with a shift lens,
    /// and an orthographic one that keeps its height.
    #[test]
    fn the_render_region_is_where_the_render_falls_in_the_view() {
        use tequila_core::math::vec3;
        use tequila_render_core::camera::{Camera, Projection};
        let view = Rect::from_min_size(pos2(40.0, 30.0), vec2(1200.0, 900.0));
        let view_aspect = view.width() / view.height();
        let projections = [
            Projection::Perspective { focal_length_mm: 50.0 },
            Projection::Shift { focal_length_mm: 35.0, shift_x: 0.1, shift_y: 0.2 },
            Projection::Orthographic { scale: 2.0 },
        ];
        let points = [vec3(0.3, 0.2, 0.0), vec3(-0.4, -0.25, 0.1), vec3(0.1, 0.35, -0.2)];
        for projection in projections {
            let camera = Camera { projection, ..Camera::look_at(vec3(0.0, 0.0, 3.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)) };
            for aspect in [16.0 / 9.0, 2.39, 1.0, 4.0 / 5.0] {
                let region = render_region(view, aspect, Keeps::of(&projection));
                assert!(close(region.width() / region.height(), aspect), "{projection:?}: the region is not the render's shape");
                for world in points {
                    let in_render = camera.project(world, aspect).unwrap();
                    let in_view = camera.project(world, view_aspect).unwrap();
                    let on_screen = view.min + vec2(in_view.x * view.width(), in_view.y * view.height());
                    let in_region = region.min + vec2(in_render.x * region.width(), in_render.y * region.height());
                    assert!((on_screen - in_region).length() < 0.05, "{projection:?} at {aspect}: {on_screen:?} in the view, {in_region:?} by the region");
                }
            }
        }
        let whole = render_region(view, 2.0, Keeps::Whole);
        assert_eq!(whole, view);
    }
}
