//! The shape tools: the Unwrap, Shape and Symmetry panels below the Islands
//! panel, and what their buttons, the right-click menu, the Alt keys and a
//! script's `ui.uv` keys do.
//!
//! Each tool works in the selection mode there is: on the picked vertices,
//! edges or islands, the faces picked on the part where it takes faces, or
//! every island when nothing is picked; and says plainly when what is
//! picked is not what it works on. The engine works out where the layout's
//! vertices go (`tequila_scene::uv`), and the editor sends that as one
//! `uv.set_positions` at the end of the frame - with a pack after, or a cut
//! before, where a tool needs one - all one step to undo, the labels on the
//! islands carried along as for any edit of the layout.

use super::actions::MarginUnit;
use super::editing;
use super::layout::Layout;
use super::selection::{Item, Mode};
use super::{Edits, LayoutEdit, Part, UvEditor};
use crate::icons::{self, Icon};
use crate::project::Project;
use tequila_core::math::{vec2, Vec2};
use tequila_format::json::Value;
use tequila_scene::uv::circle::CircleOptions;
use tequila_scene::uv::grid::GridOptions;
use tequila_scene::uv::project::ProjectOptions;
use tequila_scene::uv::rectangle::{Inside, RectangleOptions, Sides, Spacing};
use tequila_scene::uv::relax::RelaxOptions;
use tequila_scene::uv::seam_sides::{LineUp, Mover};
use tequila_scene::uv::symmetry::{Line, Plane, Source};
use tequila_scene::uv::{self, AgainOptions, Direction, Method, Pick};
use tequila_scene::{Command, MeshKey};

/// What the shape tools do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Unwrap,
    Straight,
    Grid,
    Rectangle,
    Project,
    Relax,
    Circle,
    Grow,
    Shrink,
    Weld,
    MatchSeams,
    CutAngle,
    Symmetrize,
    SnapMirror,
    SelectSide,
    SelectMirror,
}

impl Tool {
    pub fn name(self) -> &'static str {
        match self {
            Tool::Unwrap => "Unwrap again",
            Tool::Straight => "Straight line",
            Tool::Grid => "Grid",
            Tool::Rectangle => "Rectangle",
            Tool::Project => "Project",
            Tool::Relax => "Relax",
            Tool::Circle => "Circle",
            Tool::Grow => "Grow",
            Tool::Shrink => "Shrink",
            Tool::Weld => "Weld nearby",
            Tool::MatchSeams => "Match seam sides",
            Tool::CutAngle => "Cut by angle",
            Tool::Symmetrize => "Symmetrize",
            Tool::SnapMirror => "Snap to mirror",
            Tool::SelectSide => "Select one side",
            Tool::SelectMirror => "Select mirror",
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Tool::Unwrap => Icon::Unwrap,
            Tool::Straight => Icon::StraightLine,
            Tool::Grid => Icon::QuadGrid,
            Tool::Rectangle => Icon::Rectangle,
            Tool::Project => Icon::ProjectFaces,
            Tool::Relax => Icon::Relax,
            Tool::Circle => Icon::Circle,
            Tool::Grow => Icon::Grow,
            Tool::Shrink => Icon::Shrink,
            Tool::Weld => Icon::Weld,
            Tool::MatchSeams => Icon::MatchSeams,
            Tool::CutAngle => Icon::CutAngle,
            Tool::Symmetrize => Icon::Symmetrize,
            Tool::SnapMirror => Icon::Magnet,
            Tool::SelectSide => Icon::OneSide,
            Tool::SelectMirror => Icon::MirrorPick,
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Tool::Unwrap => "Lay the selected islands, or all, out flat again as the options say: which way, how, and what each keeps of where it was",
            Tool::Straight => "Lay the picked edges or vertices, one path, on a straight line across or down, as long as it is on the part; the rest of the island follows",
            Tool::Grid => "Lay four-sided faces out in straight rows and columns: the selected islands, or the faces picked on the part",
            Tool::Rectangle => "Make an island's border, or the border of the faces picked on the part, a rectangle; four picked vertices are its corners",
            Tool::Project => "Lay the faces picked on the part, or the selected islands, flat as they are seen straight on",
            Tool::Relax => "Smooth the picked vertices, or the inside of the selected islands, toward less stretch; the border and pins hold",
            Tool::Circle => "Make a picked loop of edges or vertices, or an island's border, a circle; the rest of the island follows",
            Tool::Grow => "Move the border of the selected islands, or all, out by the distance; the inside follows",
            Tool::Shrink => "Move the border of the selected islands, or all, in by the distance, to make room between them",
            Tool::Weld => "Bring picked vertices within the distance of each other together; copies of one vertex join",
            Tool::MatchSeams => "Line the picked seam vertices, or the seams of the selected islands, up with their other side",
            Tool::CutAngle => "Cut along the edges where faces meet more sharply than the angle, on the selected islands or the whole part",
            Tool::Symmetrize => "Make one side of the layout the mirror image of the other, as the part is symmetric",
            Tool::SnapMirror => "Snap the picked vertices to the mirror image of their pair on the other side of the part",
            Tool::SelectSide => "Select what lies on one side of the part's middle",
            Tool::SelectMirror => "Select the mirror image of what is selected, on the other side of the part",
        }
    }

    pub fn keys(self) -> &'static str {
        match self {
            Tool::Unwrap => "Alt + U",
            Tool::Straight => "Alt + L",
            Tool::Grid => "Alt + G",
            Tool::Rectangle => "Alt + B",
            Tool::Project => "Alt + P",
            Tool::Relax => "Alt + R",
            Tool::Circle => "Alt + O",
            Tool::Weld => "Alt + W",
            Tool::MatchSeams => "Alt + M",
            Tool::Symmetrize => "Alt + Y",
            _ => "",
        }
    }

    /// What it is called in a sentence saying it could not be done.
    fn verb(self) -> &'static str {
        match self {
            Tool::Unwrap => "unwrap again",
            Tool::Straight => "lay it straight",
            Tool::Grid => "lay out as a grid",
            Tool::Rectangle => "lay out as a rectangle",
            Tool::Project => "project it",
            Tool::Relax => "relax",
            Tool::Circle => "lay it round",
            Tool::Grow => "grow the border",
            Tool::Shrink => "shrink the border",
            Tool::Weld => "weld",
            Tool::MatchSeams => "line up the seam sides",
            Tool::CutAngle => "cut by angle",
            Tool::Symmetrize => "symmetrize",
            Tool::SnapMirror => "snap to the mirror",
            Tool::SelectSide => "select one side",
            Tool::SelectMirror => "select the mirror",
        }
    }
}

/// The tools' popovers of options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Options {
    Unwrap,
    Grid,
    Rectangle,
    Project,
    Match,
    CutAngle,
}

impl Options {
    fn from_key(key: &str) -> Option<Options> {
        Some(match key {
            "unwrap" => Options::Unwrap,
            "grid" => Options::Grid,
            "rectangle" => Options::Rectangle,
            "project" => Options::Project,
            "match_seams" => Options::Match,
            "cut_angle" => Options::CutAngle,
            _ => return None,
        })
    }
}

/// Where a mirror line of the layout is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineAt {
    /// Through the middle of the layout's square.
    Layout,
    /// Through the middle of the selection.
    Selection,
    /// Where the selection's layout is nearest symmetric about already:
    /// for an island across the plane whose middle is not there.
    Fitted,
}

/// What the shape tools are set to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub again: AgainOptions,
    pub keep_pins: bool,
    pub grid: GridOptions,
    pub rectangle: RectangleOptions,
    /// The rectangle's border pinned after.
    pub pin_border: bool,
    pub project: ProjectOptions,
    pub relax: RelaxOptions,
    pub circle: CircleOptions,
    /// How far a border grows or shrinks, and how near vertices weld, in
    /// `unit`; pixels of a texture `texture` a side.
    pub border: f32,
    pub weld: f32,
    pub unit: MarginUnit,
    pub texture: u32,
    pub line_up: LineUp,
    pub mover: Mover,
    /// Cut by angle's angle, in degrees, and whether it unwraps after.
    pub angle: f32,
    pub unwrap_after: bool,
    pub plane: Plane,
    /// Mirrored across a vertical line of the layout, else a horizontal one.
    pub vertical: bool,
    pub at: LineAt,
    pub source: Source,
    pub both_ways: bool,
    /// The side Select one side picks: an axis of the part, plus or minus.
    pub side: (usize, bool),
    /// Select mirror adds to the selection rather than replacing it.
    pub mirror_add: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            again: AgainOptions::default(),
            keep_pins: true,
            grid: GridOptions::default(),
            rectangle: RectangleOptions::default(),
            pin_border: false,
            project: ProjectOptions::default(),
            relax: RelaxOptions::default(),
            circle: CircleOptions::default(),
            border: 4.0,
            weld: 2.0,
            unit: MarginUnit::Pixels,
            texture: 2048,
            line_up: LineUp::Height,
            mover: Mover::Both,
            angle: 30.0,
            unwrap_after: true,
            plane: Plane::Auto,
            vertical: true,
            at: LineAt::Layout,
            source: Source::Auto,
            both_ways: false,
            side: (0, true),
            mirror_add: false,
        }
    }
}

/// The shape tools' part of the editor.
#[derive(Default)]
pub struct State {
    pub settings: Settings,
    /// Tools asked for this frame - by a key, the menu, a script - done at
    /// its end, where the part's mesh is at hand.
    pub pending: Vec<Tool>,
    /// A popover a script asked open, or shut.
    asked: Option<(Options, bool)>,
    /// Whether Cut by angle's popover is open: the edges it would cut are
    /// lit on the layout while it is.
    cut_open: bool,
    /// Those edges, on the layout, for the mesh, set, angle and islands they
    /// were found for.
    cut_preview: Option<((MeshKey, usize, u32, Vec<u32>), Vec<[Vec2; 2]>)>,
    /// The part's mirror for a mesh and plane: its axis and the share of
    /// its vertices paired.
    mirror: Option<((MeshKey, Plane), Option<(usize, f32)>)>,
    /// The stretch the last Unwrap again left: on what part and set, from
    /// what mesh, onto what mesh once it is applied.
    stretch: Option<(tequila_scene::NodeId, usize, MeshKey, Option<MeshKey>, f32)>,
    /// Every popover shut, as a script asked.
    close_all: bool,
    /// Asked from the faces picked on the part, which then come first.
    faces_first: bool,
    /// A panel a script asked to see: scrolled into view, 0 to 2.
    show_panel: Option<u8>,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

impl UvEditor {
    /// What a tool works on, by the selection mode: the islands, edges or
    /// vertices picked; with `faces`, the faces picked on the part when
    /// nothing is picked here.
    fn shape_pick(&self, part: &Part, layout: &Layout, faces: bool) -> Pick {
        let mut pick = Pick::default();
        if faces && self.shape.faces_first && !part.faces.is_empty() {
            pick.faces = part.faces.clone();
            return pick;
        }
        match self.mode {
            Mode::Islands => pick.islands = editing::islands(layout, &self.selection),
            Mode::Edges => pick.edges = self.selection.edges(),
            Mode::Vertices => pick.vertices = self.selection.vertices(),
            Mode::Labels => {}
        }
        if faces && pick.is_empty() {
            pick.faces = part.faces.clone();
        }
        pick
    }

    /// A distance of the shape tools in the layout's units, or why not.
    fn shape_distance(&self, layout: &Layout, value: f32) -> Result<f32, &'static str> {
        let s = &self.shape.settings;
        match s.unit {
            MarginUnit::Pixels => Ok(value / s.texture.max(1) as f32),
            MarginUnit::Millimetres => match layout.density() {
                Some(density) if density > 0.0 => Ok(value / 1000.0 / density),
                _ => Err("This layout is not one size all over, so a distance in millimetres means nothing: give it in pixels"),
            },
        }
    }

    fn shape_distance_words(&self, value: f32) -> String {
        match self.shape.settings.unit {
            MarginUnit::Pixels => format!("{value} px"),
            MarginUnit::Millimetres => format!("{value} mm"),
        }
    }

    /// Where a mirror line goes: the middle of the layout's square, or of
    /// the selection (of every island when nothing is selected).
    fn mirror_centre(&self, part: &Part, layout: &Layout, mesh: &tequila_scene::Mesh) -> Vec2 {
        let s = &self.shape.settings;
        match s.at {
            LineAt::Layout => Vec2::splat(0.5),
            LineAt::Selection => self
                .layout_frame(layout)
                .map(|f| f.centre)
                .or_else(|| layout.extent().map(|(low, high)| (low + high) * 0.5))
                .unwrap_or(Vec2::splat(0.5)),
            LineAt::Fitted => {
                let pick = self.shape_pick(part, layout, false);
                let at = |vertical: bool| uv::symmetry::fitted_line(mesh, layout.uv.set, s.plane, &pick, vertical).ok().flatten().unwrap_or(0.5);
                vec2(at(true), at(false))
            }
        }
    }

    /// Whether a tool can be done now, or why not.
    pub(super) fn can_shape(&self, tool: Tool, layout: Option<&Layout>) -> Result<(), &'static str> {
        if layout.is_none() {
            return Err("Unwrap the part first");
        }
        let picked = !self.selection.edges().is_empty() || !self.selection.vertices().is_empty();
        match tool {
            Tool::Straight if !picked => Err("Pick the edges (3) or vertices (4) of a path first"),
            Tool::Weld if !self.selection.is_layout() => Err("Pick the vertices to weld (4), or islands (2)"),
            Tool::SnapMirror if !self.selection.is_layout() => Err("Pick the vertices to snap to their mirror (4)"),
            Tool::SelectMirror if !self.selection.is_layout() => Err("Select something to pick its mirror"),
            _ => Ok(()),
        }
    }

    /// Do the tools asked for this frame.
    pub(super) fn run_shape_tools(&mut self, part: &Part, layout: Option<&Layout>, project: &Project, edits: &mut Edits) {
        let pending = std::mem::take(&mut self.shape.pending);
        if pending.is_empty() {
            return;
        }
        let Some(layout) = layout else {
            self.say("The part has no layout to work on: unwrap it first", true);
            return;
        };
        let Some(mesh) = project.document().geometry.get(part.mesh) else { return };
        for tool in pending {
            if let Err(why) = self.can_shape(tool, Some(layout)) {
                self.say(why, true);
                continue;
            }
            self.shape_tool(tool, part, layout, mesh, edits);
        }
        self.shape.faces_first = false;
    }

    /// Project, in the Islands panel's section on the faces picked on the
    /// part: those faces, whatever is selected here.
    pub(super) fn project_faces_button(&mut self, ui: &mut egui::Ui, part: &Part, layout: &Layout) {
        let ready = if part.faces.is_empty() { Err("Pick faces on the part in the viewport at Face level first") } else { self.can_shape(Tool::Project, Some(layout)) };
        ui.add_enabled_ui(ready.is_ok(), |ui| {
            let response = icons::tip(icons::labelled(ui, Tool::Project.icon(), "Project them", false), "Project them", match ready {
                Ok(()) => "Lay the faces picked on the part flat as they are seen straight on, as the Unwrap panel's Project options say",
                Err(why) => why,
            }, "");
            if response.clicked() {
                self.shape.faces_first = true;
                self.shape.pending.push(Tool::Project);
            }
        });
    }

    fn shape_tool(&mut self, tool: Tool, part: &Part, layout: &Layout, mesh: &tequila_scene::Mesh, edits: &mut Edits) {
        let (node, set) = (part.node, layout.uv.set);
        let pins: Vec<u32> = self.pins.get(&(node, set)).map(|p| p.iter().copied().collect()).unwrap_or_default();
        let s = self.shape.settings;
        let refuse = |editor: &mut UvEditor, why: String| editor.say(&format!("Could not {}: {why}", tool.verb()), true);
        let (moves, done) = match tool {
            Tool::Unwrap => {
                let islands = editing::islands_touched(layout, &self.selection);
                let method = if s.again.method == Method::LeastStretch { " with least stretch" } else { "" };
                // Not kept in place, the whole layout both ways is unwrapped
                // again as it always was: cut where it will not lie flat,
                // and packed.
                if islands.is_empty() && !s.again.keep.place && s.again.direction == Direction::Both {
                    let options = uv::UnwrapOptions { cone_degrees: self.cone, respect_cuts: true, in_place: Some(set), method: s.again.method, ..Default::default() };
                    let done = format!("Unwrapped the layout again{method}, its cuts kept, and packed");
                    edits.layout.push(LayoutEdit { command: Command::UnwrapUvs { node, options }, verb: tool.verb(), done, carry: true });
                    return;
                }
                match uv::unwrap_again(mesh, set, &islands, &s.again) {
                    Ok((moves, report)) => {
                        self.shape.stretch = Some((node, set, part.mesh, None, report.stretch));
                        let way = match s.again.direction {
                            Direction::Both => "",
                            Direction::Across => " across only",
                            Direction::Down => " down only",
                        };
                        let fell = if report.conformal > 0 { format!(" ({} conformally, as least stretch would fold)", plural(report.conformal, "island", "islands")) } else { String::new() };
                        let done = format!("Laid {} out again{way}{method}{fell}, {:.1}% stretch left", plural(report.islands, "island", "islands"), report.stretch * 100.0);
                        if !self.reshape(part, layout, moves, tool, done, edits) {
                            return;
                        }
                        if !s.again.keep.place {
                            let Some(margin) = self.pack.margin_uv(layout.density()) else { return };
                            let options = uv::PackOptions { margin, rotate: self.pack.rotate, normalize: false, orient: false, region: uv::UvRect::UNIT };
                            edits.layout.push(LayoutEdit { command: Command::PackUvs { node, set, islands: Vec::new(), options }, verb: "pack", done: "packed".into(), carry: true });
                        }
                        return;
                    }
                    Err(error) => return refuse(self, error.to_string()),
                }
            }
            Tool::Straight => match uv::straight::straight_line(mesh, set, &self.shape_pick(part, layout, false), if s.keep_pins { &pins } else { &[] }) {
                Ok((moves, report)) => (moves, format!("Laid {} on a straight line {}, the island laid out again round it", plural(report.vertices, "vertex", "vertices"), if report.across { "across" } else { "down" })),
                Err(error) => return refuse(self, error.to_string()),
            },
            Tool::Grid => match uv::grid::grid(mesh, set, &self.shape_pick(part, layout, true), &s.grid) {
                Ok((moves, report)) => {
                    let fit = if s.grid.fit_square { ", fitted to the square" } else { "" };
                    let mut done = if report.grids == 1 {
                        format!("Laid {} out as a grid, {} by {}{fit}", plural(report.columns * report.rows, "face", "faces"), report.columns, report.rows)
                    } else {
                        format!("Laid out {} grids{fit}", report.grids)
                    };
                    if report.odd > 0 {
                        done.push_str(&format!("; {} that {} four-sided laid out round it", plural(report.odd, "face", "faces"), if report.odd == 1 { "isn't" } else { "aren't" }));
                    }
                    (moves, done)
                }
                Err(error) => return refuse(self, error.to_string()),
            },
            Tool::Rectangle => match uv::rectangle::rectangle(mesh, set, &self.shape_pick(part, layout, true), &s.rectangle) {
                Ok((moves, report)) => {
                    let mut done = if report.rectangles == 1 { "Laid the border out as a rectangle".to_string() } else { format!("Laid {} borders out as rectangles", report.rectangles) };
                    if s.pin_border {
                        self.pins.entry((node, set)).or_default().extend(report.border.iter().copied());
                        done.push_str(&format!(", {} pinned", plural(report.border.len(), "vertex", "vertices")));
                    }
                    (moves, done)
                }
                Err(error) => return refuse(self, error.to_string()),
            },
            Tool::Project => match uv::project::project_faces(mesh, set, &self.shape_pick(part, layout, true), &s.project) {
                Ok((moves, report)) => (moves, format!("Laid {} flat as seen straight on, {}", plural(report.faces, "face", "faces"), plural(report.pieces, "piece", "pieces"))),
                Err(error) => return refuse(self, error.to_string()),
            },
            Tool::Relax => match uv::relax::relax_layout(mesh, set, &self.shape_pick(part, layout, false), &pins, &s.relax) {
                Ok((moves, report)) => {
                    let stopped = if report.rounds < s.relax.rounds { ", stopped before a fold" } else { "" };
                    (moves, format!("Relaxed {} over {}{stopped}", plural(report.vertices, "vertex", "vertices"), plural(report.rounds as usize, "round", "rounds")))
                }
                Err(error) => return refuse(self, error.to_string()),
            },
            Tool::Circle => match uv::circle::circle(mesh, set, &self.shape_pick(part, layout, true), &pins, &s.circle) {
                Ok((moves, report)) => (moves, format!("Laid {} on {}", plural(report.vertices, "vertex", "vertices"), if report.circles == 1 { "a circle".to_string() } else { format!("{} circles", report.circles) })),
                Err(error) => return refuse(self, error.to_string()),
            },
            Tool::Grow | Tool::Shrink => {
                let by = match self.shape_distance(layout, s.border) {
                    Ok(by) => by,
                    Err(why) => return self.say(why, true),
                };
                let by = if tool == Tool::Grow { by } else { -by };
                match uv::offset::grow_border(mesh, set, &self.shape_pick(part, layout, false), by) {
                    Ok((moves, report)) => {
                        let how = if tool == Tool::Grow { "Grew" } else { "Shrank" };
                        let mut done = format!("{how} the border of {} by {}", plural(report.islands, "island", "islands"), self.shape_distance_words(s.border));
                        if report.eased > 0 {
                            done.push_str(&format!(", less at {} where an island is thinner", plural(report.eased, "vertex", "vertices")));
                        }
                        if report.left > 0 {
                            done.push_str(&format!("; {} too small for it left as {}", plural(report.left, "island", "islands"), if report.left == 1 { "it was" } else { "they were" }));
                        }
                        (moves, done)
                    }
                    Err(error) => return refuse(self, error.to_string()),
                }
            }
            Tool::Weld => {
                let within = match self.shape_distance(layout, s.weld) {
                    Ok(within) => within,
                    Err(why) => return self.say(why, true),
                };
                match uv::weld::weld_nearby(mesh, set, &self.shape_pick(part, layout, false), within) {
                    Ok((moves, report)) => {
                        let mut done = format!("Welded {} in {}", plural(report.welded, "vertex", "vertices"), plural(report.places, "place", "places"));
                        if report.joined > 0 {
                            done.push_str(&format!(", {} joined across seams", plural(report.joined, "vertex", "vertices")));
                        }
                        if report.skipped > 0 {
                            done.push_str(&format!("; {} left apart, as welding would squash a face", plural(report.skipped, "pair", "pairs")));
                        }
                        (moves, done)
                    }
                    Err(error) => return refuse(self, error.to_string()),
                }
            }
            Tool::MatchSeams => match uv::seam_sides::match_seams(mesh, set, &self.shape_pick(part, layout, false), s.line_up, s.mover, &pins) {
                Ok((moves, report)) => {
                    let how = match s.line_up {
                        LineUp::Height => "at one height",
                        LineUp::Across => "one above the other",
                        LineUp::Place => "in one place, a hair apart",
                    };
                    (moves, format!("Lined up the seam sides of {} {how}", plural(report.vertices, "vertex", "vertices")))
                }
                Err(error) => return refuse(self, error.to_string()),
            },
            Tool::CutAngle => {
                let islands = editing::islands(layout, &self.selection);
                let edges = match uv::angle_cut::sharp_edges(mesh, set, &islands, s.angle) {
                    Ok(edges) => edges,
                    Err(error) => return refuse(self, error.to_string()),
                };
                if edges.is_empty() {
                    return self.say(&format!("No edges inside {} are sharper than {:.0}°", if islands.is_empty() { "the islands" } else { "the selected islands" }, s.angle), true);
                }
                let mut done = format!("Cut along {} sharper than {:.0}°", plural(edges.len(), "edge", "edges"), s.angle);
                edits.layout.push(LayoutEdit { command: Command::CutUvs { node, set, edges, reflatten: true }, verb: tool.verb(), done: done.clone(), carry: true });
                if s.unwrap_after {
                    let options = uv::UnwrapOptions { cone_degrees: self.cone, respect_cuts: true, in_place: Some(set), method: s.again.method, ..Default::default() };
                    done = "unwrapped again keeping the cuts".into();
                    edits.layout.push(LayoutEdit { command: Command::UnwrapUvs { node, options }, verb: "unwrap again", done, carry: true });
                }
                return;
            }
            Tool::Symmetrize => {
                let centre = self.mirror_centre(part, layout, mesh);
                let line = if s.vertical { Line::Vertical(centre.x) } else { Line::Horizontal(centre.y) };
                match uv::symmetry::symmetrize(mesh, set, s.plane, &self.shape_pick(part, layout, false), line, s.source) {
                    Ok((moves, report)) => {
                        let axis = ["X", "Y", "Z"][report.axis];
                        let (from, to) = if report.from_plus { ('+', '−') } else { ('−', '+') };
                        let mut done = format!("Made the {to}{axis} side the mirror image of the {from}{axis} side");
                        if report.unpaired > 0 {
                            let have = if report.unpaired == 1 { "has" } else { "have" };
                            done.push_str(&format!(" · {} {have} no mirror on the part: its two sides differ there", plural(report.unpaired, "vertex", "vertices")));
                        }
                        if report.elsewhere > 0 {
                            done.push_str(&format!(" · {} mirrored on another island: the sides are cut differently there", plural(report.elsewhere, "vertex", "vertices")));
                        }
                        if report.eased > 0 {
                            done.push_str(&format!(" · {} carried along, as the sides are split into triangles differently there", plural(report.eased, "vertex", "vertices")));
                        }
                        (moves, done)
                    }
                    Err(error) => return refuse(self, error.to_string()),
                }
            }
            Tool::SnapMirror => {
                let centre = self.mirror_centre(part, layout, mesh);
                match uv::symmetry::snap_to_mirror(mesh, set, s.plane, &self.shape_pick(part, layout, false), centre, s.vertical, s.both_ways) {
                    Ok((moves, report)) => {
                        let mut done = format!("Snapped {} to their mirror{}", plural(report.snapped, "vertex", "vertices"), if s.both_ways { " both ways" } else { "" });
                        if report.unpaired > 0 {
                            done.push_str(&format!(" · {} without one", plural(report.unpaired, "vertex", "vertices")));
                        }
                        (moves, done)
                    }
                    Err(error) => return refuse(self, error.to_string()),
                }
            }
            Tool::SelectSide => return self.select_side(layout, mesh),
            Tool::SelectMirror => return self.select_mirror(layout, mesh),
        };
        self.reshape(part, layout, moves, tool, done, edits);
    }

    /// A tool's moves sent as one `uv.set_positions`; false where nothing
    /// moves, which it says.
    fn reshape(&mut self, part: &Part, layout: &Layout, moves: uv::Moves, tool: Tool, done: String, edits: &mut Edits) -> bool {
        let mut vertices = Vec::with_capacity(moves.len());
        let mut places = Vec::with_capacity(moves.len());
        for (v, p) in moves {
            if layout.uv.uv_vertices.get(v as usize).is_some_and(|u| u.uv != p) {
                vertices.push(v);
                places.push([p.x, p.y]);
            }
        }
        if vertices.is_empty() {
            self.say(&format!("Nothing to {}: the layout is that way already", tool.verb()), false);
            return false;
        }
        let command = Command::SetUvPositions { node: part.node, set: layout.uv.set, vertices, uv: places };
        edits.layout.push(LayoutEdit { command, verb: tool.verb(), done, carry: true });
        true
    }

    /// Select what lies on one side of the part, in the mode there is; in
    /// the Labels mode, its islands.
    fn select_side(&mut self, layout: &Layout, mesh: &tequila_scene::Mesh) {
        let (axis, positive) = self.shape.settings.side;
        let side = match uv::symmetry::one_side(mesh, layout.uv.set, axis, positive) {
            Ok(side) => side,
            Err(error) => return self.say(&format!("Could not select one side: {error}"), true),
        };
        if self.mode == Mode::Labels {
            self.set_mode(Mode::Islands);
        }
        let name = format!("{}{}", if positive { '+' } else { '−' }, ["X", "Y", "Z"][axis.min(2)]);
        let (items, what) = side_items(layout, self.mode, &side);
        let count = items.len();
        self.selection.set_all(items);
        self.say(&format!("Selected {} on the {name} side", plural(count, what.0, what.1)), false);
    }

    /// Select the mirror image of the selection, adding to it or in its
    /// place.
    fn select_mirror(&mut self, layout: &Layout, mesh: &tequila_scene::Mesh) {
        let places: Vec<u32> = self.selection.vertices().iter().filter_map(|&v| layout.vertex_uv.get(v as usize).copied().filter(|&g| g != u32::MAX)).collect();
        let edges: Vec<u32> = self.selection.edges().iter().filter_map(|&e| layout.edge_named(e)).collect();
        let islands = editing::islands(layout, &self.selection);
        let images = match uv::symmetry::mirrored(mesh, layout.uv.set, self.shape.settings.plane, &places, &edges, &islands) {
            Ok(images) => images,
            Err(error) => return self.say(&format!("Could not select the mirror: {error}"), true),
        };
        let (items, what) = side_items(layout, self.mode, &images);
        if items.is_empty() {
            return self.say("Nothing selected has a mirror image on the part", true);
        }
        let count = items.len();
        if self.shape.settings.mirror_add {
            self.selection.add_all(items);
        } else {
            self.selection.set_all(items);
        }
        self.say(&format!("Selected the mirror image: {}", plural(count, what.0, what.1)), false);
    }

    // ---- the panels -------------------------------------------------------

    /// The Unwrap, Shape and Symmetry panels.
    pub(super) fn shape_panels(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&Layout>, project: &Project) {
        self.shape.cut_open = false;
        let shown = [
            crate::field::Fold::new("Unwrap").id_salt("uv-shape-unwrap").default_open(true).show(ui, |ui| self.unwrap_panel(ui, part, layout)).header_response,
            crate::field::Fold::new("Shape").id_salt("uv-shape-shape").default_open(true).show(ui, |ui| self.shape_panel(ui, part, layout, project)).header_response,
            crate::field::Fold::new("Symmetry").id_salt("uv-shape-symmetry").default_open(true).show(ui, |ui| self.symmetry_panel(ui, part, layout, project)).header_response,
        ];
        if let Some(which) = self.shape.show_panel.take() {
            shown[which.min(2) as usize].scroll_to_me(Some(egui::Align::TOP));
        }
        self.shape.asked = None;
        self.shape.close_all = false;
    }

    /// A tool's button, greyed with the reason where it cannot be done.
    fn shape_button(&mut self, ui: &mut egui::Ui, tool: Tool, layout: Option<&Layout>) {
        let ready = self.can_shape(tool, layout);
        ui.add_enabled_ui(ready.is_ok(), |ui| {
            let response = icons::labelled(ui, tool.icon(), tool.name(), false);
            let response = icons::tip(response, tool.name(), match ready {
                Ok(()) => tool.detail(),
                Err(why) => why,
            }, tool.keys());
            if response.clicked() {
                self.shape.pending.push(tool);
            }
        });
    }

    /// A tool's options, in a popover off a sliders button; open or shut as
    /// a script asks. Whether it is open.
    fn options_button(&mut self, ui: &mut egui::Ui, which: Options, title: &str, detail: &str, add: impl FnOnce(&mut UvEditor, &mut egui::Ui)) -> bool {
        let id = ui.id().with(("uv-shape-options", which as u8));
        let showing = ui.memory(|m| m.is_popup_open(id));
        let button = icons::tip(icons::button(ui, Icon::Sliders, showing, 30.0, egui::Sense::click()), title, detail, "");
        match (button.clicked(), self.shape.asked) {
            (true, _) => ui.memory_mut(|m| m.toggle_popup(id)),
            _ if self.shape.close_all && showing => ui.memory_mut(|m| m.close_popup()),
            (false, Some((asked, true))) if asked == which => ui.memory_mut(|m| m.open_popup(id)),
            (false, Some((asked, false))) if asked == which && showing => ui.memory_mut(|m| m.close_popup()),
            _ => {}
        }
        let mut open = false;
        egui::popup::popup_below_widget(ui, id, &button, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_min_width(260.0);
            open = true;
            add(self, ui);
        });
        open
    }

    fn unwrap_panel(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&Layout>) {
        // The stretch the last Unwrap again left, while the layout it made
        // is the one shown.
        if let Some((node, set, before, after, _)) = &mut self.shape.stretch {
            if *node == part.node && layout.is_some_and(|l| l.uv.set == *set) && after.is_none() && part.mesh != *before {
                *after = Some(part.mesh);
            }
        }
        let stretch = self.shape.stretch.filter(|(node, _, _, after, _)| *node == part.node && *after == Some(part.mesh)).map(|s| s.4);
        if layout.is_none() {
            ui.weak("Unwrap the part to shape its layout.");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::Unwrap, layout);
            self.options_button(ui, Options::Unwrap, "Unwrap options", "Which way, by which method, and what each island keeps of where it was", |editor, ui| editor.unwrap_options(ui));
            if let Some(stretch) = stretch {
                ui.label(egui::RichText::new(format!("{:.1}% stretch left", stretch * 100.0)).size(12.0).color(crate::theme::p().text_dim));
            }
        });
        crate::theme::section(ui, "Lay out");
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::Straight, layout);
            ui.add(crate::field::Checkbox::new(&mut self.shape.settings.keep_pins, "Keep pins")).on_hover_text("Hold the pins too while the island is laid out again round the line");
        });
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::Grid, layout);
            self.options_button(ui, Options::Grid, "Grid options", "How the rows and columns are spaced, and whether the grid fills the square", |editor, ui| {
                let s = &mut editor.shape.settings.grid;
                ui.add(crate::field::Checkbox::new(&mut s.even, "Even spacing")).on_hover_text("Every column as wide and every row as high as the others, rather than as long as their edges on the part");
                ui.add(crate::field::Checkbox::new(&mut s.fit_square, "Fit to square")).on_hover_text("Scale each grid to fill the layout's square, across and down");
            });
            self.shape_button(ui, Tool::Rectangle, layout);
            self.options_button(ui, Options::Rectangle, "Rectangle options", "How the sides are spaced and sized, what the inside does, and pins on the border after", |editor, ui| editor.rectangle_options(ui));
        });
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::Project, layout);
            self.options_button(ui, Options::Project, "Project options", "Each piece on its own, the whole of each island, and how large", |editor, ui| editor.project_options(ui));
        });
    }

    fn unwrap_options(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.shape.settings.again;
        crate::theme::section(ui, "Direction");
        crate::theme::segmented(ui, &mut s.direction, &[(Direction::Both, "Both"), (Direction::Across, "Across only"), (Direction::Down, "Down only")]);
        ui.small("One way only solves that coordinate again and leaves the other exactly as it is.");
        crate::theme::section(ui, "Method");
        crate::theme::segmented(ui, &mut s.method, &[(Method::Conformal, "Conformal"), (Method::LeastStretch, "Least stretch")]);
        ui.small("Least stretch keeps the surface's lengths, starting from the conformal layout. The Unwrap button in the toolbar uses it too.");
        crate::theme::section(ui, "Each island keeps its");
        ui.horizontal(|ui| {
            ui.add(crate::field::Checkbox::new(&mut s.keep.place, "Place")).on_hover_text("Centred where it was; without it, the layout is packed after");
            ui.add(crate::field::Checkbox::new(&mut s.keep.size, "Size")).on_hover_text("As large as it was; without it, at the layout's own density");
            ui.add(crate::field::Checkbox::new(&mut s.keep.turn, "Turn")).on_hover_text("Turned as it lay; without it, laid square to the layout");
        });
        ui.add_space(6.0);
        if ui.button("Unwrap again").clicked() {
            self.shape.pending.push(Tool::Unwrap);
            ui.close_menu();
        }
    }

    fn rectangle_options(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.shape.settings;
        crate::theme::section(ui, "Along the sides");
        crate::theme::segmented(ui, &mut s.rectangle.spacing, &[(Spacing::Surface, "As on the part"), (Spacing::Even, "Even")]);
        crate::theme::section(ui, "Size");
        crate::theme::segmented(ui, &mut s.rectangle.sides, &[(Sides::Average, "Average of opposite sides"), (Sides::Longest, "Longest")]);
        crate::theme::section(ui, "Inside");
        crate::theme::segmented(ui, &mut s.rectangle.inside, &[(Inside::Reflatten, "Re-flatten"), (Inside::Smooth, "Smooth")]);
        ui.small("Re-flatten lays the inside out true to the surface; Smooth spreads it evenly, which cannot fold.");
        ui.add(crate::field::Checkbox::new(&mut s.pin_border, "Pin the border")).on_hover_text("Leave pins on the border's vertices after");
    }

    fn project_options(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.shape.settings.project;
        ui.add(crate::field::Checkbox::new(&mut s.separately, "Each piece on its own")).on_hover_text("Each connected piece seen down its own direction, rather than all down one");
        ui.add(crate::field::Checkbox::new(&mut s.whole_island, "The whole island")).on_hover_text("Take in every face of the islands the picked faces are on");
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Scale").color(crate::theme::dim()));
            ui.add(egui::DragValue::new(&mut s.scale).range(0.01..=100.0).speed(0.01).max_decimals(3).suffix("×")).on_hover_text("How large, times the layout's density");
        });
    }

    fn shape_panel(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&Layout>, project: &Project) {
        if layout.is_none() {
            ui.weak("Unwrap the part to shape its layout.");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::Relax, layout);
            let rounds = &mut self.shape.settings.relax.rounds;
            ui.add(crate::field::Slider::new(rounds, 1..=200).text("Rounds")).on_hover_text("How many rounds of smoothing: more is smoother");
        });
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::Circle, layout);
            let s = &mut self.shape.settings.circle;
            ui.add(crate::field::Checkbox::new(&mut s.even, "Even")).on_hover_text("Space the vertices evenly round, rather than as their edges are long on the part");
            ui.add(crate::field::Checkbox::new(&mut s.whole_region, "Whole region")).on_hover_text("Make the border of the selected islands, or of the faces picked on the part, the circle");
        });
        crate::theme::section(ui, "Border and weld");
        ui.horizontal_wrapped(|ui| {
            let s = &mut self.shape.settings;
            let suffix = if s.unit == MarginUnit::Millimetres { " mm" } else { " px" };
            ui.add(egui::DragValue::new(&mut s.border).range(0.0..=500.0).speed(0.1).max_decimals(2).suffix(suffix)).on_hover_text("How far Grow and Shrink move the border");
            self.shape_button(ui, Tool::Grow, layout);
            self.shape_button(ui, Tool::Shrink, layout);
        });
        ui.horizontal_wrapped(|ui| {
            let s = &mut self.shape.settings;
            let suffix = if s.unit == MarginUnit::Millimetres { " mm" } else { " px" };
            ui.add(egui::DragValue::new(&mut s.weld).range(0.0..=500.0).speed(0.1).max_decimals(2).suffix(suffix)).on_hover_text("How near vertices have to be to weld");
            self.shape_button(ui, Tool::Weld, layout);
        });
        ui.horizontal(|ui| {
            let density = layout.and_then(|l| l.density());
            let s = &mut self.shape.settings;
            let mut unit = if density.is_none() { MarginUnit::Pixels } else { s.unit };
            ui.add_enabled_ui(density.is_some(), |ui| {
                ui.set_width(100.0);
                if crate::theme::segmented(ui, &mut unit, &[(MarginUnit::Millimetres, "mm"), (MarginUnit::Pixels, "px")]) {
                    s.unit = unit;
                }
            })
            .response
            .on_disabled_hover_text("This layout is not one size all over, so distances are in pixels");
            if s.unit == MarginUnit::Pixels || density.is_none() {
                ui.label(egui::RichText::new("of").color(crate::theme::dim()));
                ui.add(egui::DragValue::new(&mut s.texture).range(64..=16384).speed(16.0).suffix(" px")).on_hover_text("The side of the texture the distances are pixels of");
            }
        });
        crate::theme::section(ui, "Seams");
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::MatchSeams, layout);
            self.options_button(ui, Options::Match, "Match options", "How the sides line up, and which side moves", |editor, ui| {
                let s = &mut editor.shape.settings;
                crate::theme::section(ui, "Line up");
                crate::theme::segmented(ui, &mut s.line_up, &[(LineUp::Height, "Same height"), (LineUp::Across, "Same across"), (LineUp::Place, "Same place")]);
                ui.small("At the same place the sides stay a hair apart, so the seam stays a seam; Stitch joins them.");
                crate::theme::section(ui, "Which moves");
                crate::theme::segmented(ui, &mut s.mover, &[(Mover::Both, "Both, to the middle"), (Mover::Other, "The other side")]);
            });
        });
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::CutAngle, layout);
            let open = self.options_button(ui, Options::CutAngle, "Cut by angle options", "How sharp an edge is cut, and whether the layout is unwrapped again after; the edges it would cut are lit", |editor, ui| {
                let s = &mut editor.shape.settings;
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Sharper than").color(crate::theme::dim()));
                    ui.add(egui::DragValue::new(&mut s.angle).range(1.0..=179.0).speed(0.5).suffix("°").max_decimals(1));
                });
                ui.add(crate::field::Checkbox::new(&mut s.unwrap_after, "Unwrap after")).on_hover_text("Unwrap the layout again keeping the cuts, and pack it");
                ui.small("The edges that would be cut are lit on the layout.");
                ui.add_space(4.0);
                if ui.button("Cut").clicked() {
                    editor.shape.pending.push(Tool::CutAngle);
                    ui.close_menu();
                }
            });
            if open {
                self.shape.cut_open = true;
                if let Some(layout) = layout {
                    self.keep_cut_preview(part, layout, project);
                }
            }
        });
    }

    /// The edges Cut by angle would cut, found again when what they are for
    /// changes.
    fn keep_cut_preview(&mut self, part: &Part, layout: &Layout, project: &Project) {
        let islands = editing::islands(layout, &self.selection);
        let key = (part.mesh, layout.uv.set, self.shape.settings.angle.to_bits(), islands.clone());
        if self.shape.cut_preview.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let Some(mesh) = project.document().geometry.get(part.mesh) else { return };
        let edges = uv::angle_cut::sharp_edges(mesh, layout.uv.set, &islands, self.shape.settings.angle).unwrap_or_default();
        let segments = edges.iter().filter_map(|&e| layout.edge_named(e)).map(|e| layout.ends(e)).collect();
        self.shape.cut_preview = Some((key, segments));
    }

    /// What the shape tools draw over the layout: the edges Cut by angle
    /// would cut, while its options are open.
    pub(super) fn draw_shape_overlay(&self, painter: &egui::Painter, xform: &super::view::Xform) {
        if !self.shape.cut_open {
            return;
        }
        let Some((_, segments)) = &self.shape.cut_preview else { return };
        let stroke = egui::Stroke::new(3.0_f32, crate::theme::hue(crate::theme::Hue::Pink));
        for [a, b] in segments {
            painter.line_segment([xform.to_screen(*a), xform.to_screen(*b)], stroke);
        }
    }

    fn symmetry_panel(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&Layout>, project: &Project) {
        let p = crate::theme::p();
        if layout.is_none() {
            ui.weak("Unwrap the part to make its layout symmetric.");
            return;
        }
        // The mirror, found again when the mesh or the plane changes.
        let key = (part.mesh, self.shape.settings.plane);
        if self.shape.mirror.as_ref().is_none_or(|(k, _)| *k != key) {
            let found = project.document().geometry.get(part.mesh).and_then(|mesh| uv::symmetry::find_mirror(mesh, key.1)).map(|m| (m.axis, m.share()));
            self.shape.mirror = Some((key, found));
        }
        crate::theme::section(ui, "Mirror plane");
        ui.horizontal(|ui| {
            crate::theme::segmented(ui, &mut self.shape.settings.plane, &[(Plane::Auto, "Auto"), (Plane::X, "X"), (Plane::Y, "Y"), (Plane::Z, "Z")]);
        });
        if let Some((_, Some((axis, share)))) = &self.shape.mirror {
            let colour = if *share >= 0.9 { p.text_dim } else { p.danger };
            ui.label(egui::RichText::new(format!("The part's {} plane: {:.0}% of its vertices have a mirror", ["X", "Y", "Z"][*axis], share * 100.0)).size(12.0).color(colour));
        }
        crate::theme::section(ui, "Mirror line on the layout");
        ui.horizontal(|ui| {
            let s = &mut self.shape.settings;
            crate::theme::segmented(ui, &mut s.vertical, &[(true, "Vertical"), (false, "Horizontal")]);
        });
        ui.horizontal(|ui| {
            let s = &mut self.shape.settings;
            crate::theme::segmented(ui, &mut s.at, &[(LineAt::Layout, "Layout's middle"), (LineAt::Selection, "Selection's middle"), (LineAt::Fitted, "Best fit")]);
        });
        crate::theme::section(ui, "Copy from");
        ui.horizontal(|ui| {
            let s = &mut self.shape.settings;
            crate::theme::segmented(ui, &mut s.source, &[(Source::Auto, "Auto"), (Source::Plus, "Plus side"), (Source::Minus, "Minus side")]);
        });
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::Symmetrize, layout);
            self.shape_button(ui, Tool::SnapMirror, layout);
            ui.add(crate::field::Checkbox::new(&mut self.shape.settings.both_ways, "Both ways")).on_hover_text("Snap across a vertical and a horizontal line, for a part symmetric in two planes");
        });
        crate::theme::section(ui, "Select");
        ui.horizontal_wrapped(|ui| {
            let s = &mut self.shape.settings;
            let sides = [((0, true), "+X"), ((0, false), "−X"), ((1, true), "+Y"), ((1, false), "−Y"), ((2, true), "+Z"), ((2, false), "−Z")];
            crate::theme::segmented(ui, &mut s.side, &sides);
        });
        ui.horizontal_wrapped(|ui| {
            self.shape_button(ui, Tool::SelectSide, layout);
            self.shape_button(ui, Tool::SelectMirror, layout);
            ui.add(crate::field::Checkbox::new(&mut self.shape.settings.mirror_add, "Add")).on_hover_text("Add the mirror image to the selection rather than selecting it instead");
        });
    }

    // ---- scripting --------------------------------------------------------

    /// A script's shape tool keys: options set, tools asked for, each as
    /// the panels do it. Checked first, so one bad key changes nothing.
    pub(super) fn shape_script(&mut self, keys: &[(String, Value)]) -> Result<(), String> {
        let mut s = self.shape.settings;
        let mut tools = Vec::new();
        let mut asked = None;
        let mut show = None;
        let mut close = false;
        for (key, value) in keys {
            let off = matches!(value, Value::Bool(false) | Value::Null);
            match key.as_str() {
                "shape_panel" => {
                    show = Some(match value.as_str() {
                        Some("unwrap") => 0,
                        Some("shape") => 1,
                        Some("symmetry") => 2,
                        _ => return Err("`shape_panel` is the panel to show: unwrap, shape or symmetry".into()),
                    });
                }
                "unwrap_options" => {
                    if let Some(direction) = text(value, "direction")? {
                        s.again.direction = Direction::from_key(direction).ok_or_else(|| format!("`{direction}` is not a direction: both, across or down"))?;
                    }
                    if let Some(method) = text(value, "method")? {
                        s.again.method = Method::from_key(method).ok_or_else(|| format!("`{method}` is not a method: conformal or least_stretch"))?;
                    }
                    match value.get("keep") {
                        None => {}
                        Some(Value::Bool(all)) => s.again.keep = uv::Keep { place: *all, size: *all, turn: *all },
                        Some(keep) => {
                            s.again.keep.place = flag(keep, "place")?.unwrap_or(s.again.keep.place);
                            s.again.keep.size = flag(keep, "size")?.unwrap_or(s.again.keep.size);
                            s.again.keep.turn = flag(keep, "turn")?.unwrap_or(s.again.keep.turn);
                        }
                    }
                }
                "shape_options" => {
                    asked = Some(match value {
                        Value::String(which) => (Options::from_key(which).ok_or_else(|| format!("`{which}` has no options: unwrap, grid, rectangle, project, match_seams or cut_angle"))?, true),
                        Value::Bool(false) | Value::Null => {
                            close = true;
                            continue;
                        }
                        _ => return Err("`shape_options` is the name of a tool's options to open, or false".into()),
                    });
                }
                _ if off => {}
                "straight" => {
                    s.keep_pins = flag(value, "keep_pins")?.unwrap_or(s.keep_pins);
                    tools.push(Tool::Straight);
                }
                "grid" => {
                    s.grid.even = flag(value, "even")?.unwrap_or(s.grid.even);
                    s.grid.fit_square = flag(value, "fit_square")?.unwrap_or(s.grid.fit_square);
                    tools.push(Tool::Grid);
                }
                "rectangle" => {
                    if let Some(spacing) = text(value, "spacing")? {
                        s.rectangle.spacing = match spacing {
                            "surface" => Spacing::Surface,
                            "even" => Spacing::Even,
                            other => return Err(format!("`{other}` is not a spacing: surface or even")),
                        };
                    }
                    if let Some(inside) = text(value, "inside")? {
                        s.rectangle.inside = match inside {
                            "reflatten" => Inside::Reflatten,
                            "smooth" => Inside::Smooth,
                            other => return Err(format!("`{other}` is not an inside: reflatten or smooth")),
                        };
                    }
                    if let Some(sides) = text(value, "sides")? {
                        s.rectangle.sides = match sides {
                            "average" => Sides::Average,
                            "longest" => Sides::Longest,
                            other => return Err(format!("`{other}` is not a size: average or longest")),
                        };
                    }
                    s.pin_border = flag(value, "pin")?.unwrap_or(s.pin_border);
                    tools.push(Tool::Rectangle);
                }
                "project" => {
                    s.project.separately = flag(value, "separately")?.unwrap_or(s.project.separately);
                    s.project.whole_island = flag(value, "whole_island")?.unwrap_or(s.project.whole_island);
                    if let Some(scale) = number(value, "scale")? {
                        s.project.scale = scale.clamp(0.01, 100.0);
                    }
                    tools.push(Tool::Project);
                }
                "relax" => {
                    if let Some(rounds) = number(value, "rounds")? {
                        s.relax.rounds = rounds.clamp(1.0, 200.0) as u32;
                    }
                    tools.push(Tool::Relax);
                }
                "circle" => {
                    s.circle.even = flag(value, "even")?.unwrap_or(s.circle.even);
                    s.circle.whole_region = flag(value, "whole_region")?.unwrap_or(s.circle.whole_region);
                    tools.push(Tool::Circle);
                }
                "grow" => {
                    let by = match value {
                        Value::Number(n) => Some(*n as f32),
                        _ => number(value, "by")?,
                    };
                    distance_unit(value, &mut s)?;
                    let by = by.unwrap_or(s.border);
                    if !by.is_finite() || by == 0.0 {
                        return Err("`grow` is a distance, negative to shrink".into());
                    }
                    s.border = by.abs().min(500.0);
                    tools.push(if by > 0.0 { Tool::Grow } else { Tool::Shrink });
                }
                "weld" => {
                    let within = match value {
                        Value::Number(n) => Some(*n as f32),
                        _ => number(value, "within")?,
                    };
                    distance_unit(value, &mut s)?;
                    if let Some(within) = within {
                        s.weld = within.clamp(0.0, 500.0);
                    }
                    tools.push(Tool::Weld);
                }
                "match_seams" => {
                    if let Some(line_up) = text(value, "line_up")? {
                        s.line_up = match line_up {
                            "height" => LineUp::Height,
                            "across" => LineUp::Across,
                            "place" => LineUp::Place,
                            other => return Err(format!("`{other}` is not a way to line up: height, across or place")),
                        };
                    }
                    if let Some(mover) = text(value, "move")? {
                        s.mover = match mover {
                            "both" => Mover::Both,
                            "other" => Mover::Other,
                            other => return Err(format!("`{other}` is not which side moves: both or other")),
                        };
                    }
                    tools.push(Tool::MatchSeams);
                }
                "cut_angle" => {
                    let angle = match value {
                        Value::Number(n) => Some(*n as f32),
                        _ => number(value, "angle")?,
                    };
                    if let Some(angle) = angle {
                        s.angle = angle.clamp(1.0, 179.0);
                    }
                    s.unwrap_after = flag(value, "unwrap_after")?.unwrap_or(s.unwrap_after);
                    tools.push(Tool::CutAngle);
                }
                "symmetrize" | "snap_mirror" => {
                    if let Some(plane) = text(value, "plane")? {
                        s.plane = Plane::from_key(plane).ok_or_else(|| format!("`{plane}` is not a plane: auto, x, y or z"))?;
                    }
                    if let Some(line) = text(value, "line")? {
                        s.vertical = match line {
                            "vertical" => true,
                            "horizontal" => false,
                            other => return Err(format!("`{other}` is not a mirror line: vertical or horizontal")),
                        };
                    }
                    if let Some(at) = text(value, "at")? {
                        s.at = match at {
                            "layout" => LineAt::Layout,
                            "selection" => LineAt::Selection,
                            "fitted" | "best_fit" => LineAt::Fitted,
                            other => return Err(format!("`{other}` is not where the line is: layout, selection or fitted")),
                        };
                    }
                    if let Some(source) = text(value, "source")? {
                        s.source = match source {
                            "auto" => Source::Auto,
                            "plus" => Source::Plus,
                            "minus" => Source::Minus,
                            other => return Err(format!("`{other}` is not a side to copy from: auto, plus or minus")),
                        };
                    }
                    s.both_ways = flag(value, "both_ways")?.unwrap_or(s.both_ways);
                    tools.push(if key == "symmetrize" { Tool::Symmetrize } else { Tool::SnapMirror });
                }
                "select_side" => {
                    let side = value.as_str().ok_or("`select_side` is a side of the part: +x, -x, +y, -y, +z or -z")?;
                    let positive = !side.starts_with('-');
                    let axis = match side.trim_start_matches(['+', '-']) {
                        "x" | "X" => 0,
                        "y" | "Y" => 1,
                        "z" | "Z" => 2,
                        _ => return Err(format!("`{side}` is not a side of the part: +x, -x, +y, -y, +z or -z")),
                    };
                    s.side = (axis, positive);
                    tools.push(Tool::SelectSide);
                }
                "select_mirror" => {
                    if let Some(plane) = text(value, "plane")? {
                        s.plane = Plane::from_key(plane).ok_or_else(|| format!("`{plane}` is not a plane: auto, x, y or z"))?;
                    }
                    s.mirror_add = flag(value, "add")?.unwrap_or(s.mirror_add);
                    tools.push(Tool::SelectMirror);
                }
                other => return Err(format!("`{other}` is not a shape tool")),
            }
        }
        self.shape.settings = s;
        self.shape.pending.extend(tools);
        if asked.is_some() {
            self.shape.asked = asked;
        }
        if show.is_some() {
            self.shape.show_panel = show;
        }
        self.shape.close_all |= close;
        Ok(())
    }

    /// The shape tools' settings, for the scripting channel's `ui` query.
    pub(super) fn shape_state(&self) -> Value {
        let s = &self.shape.settings;
        let mut out = tequila_format::json_write::Object::new();
        out.set("direction", s.again.direction.key())
            .set("method", s.again.method.key())
            .set("plane", s.plane.key())
            .set("stretch", self.shape.stretch.map(|s| Value::Number(s.4 as f64)).unwrap_or(Value::Null))
            .set("cut_edges", self.shape.cut_preview.as_ref().filter(|_| self.shape.cut_open).map(|(_, s)| Value::Number(s.len() as f64)).unwrap_or(Value::Null));
        if let Some((_, Some((axis, share)))) = &self.shape.mirror {
            out.set("mirror_axis", ["x", "y", "z"][*axis]).set("mirror_paired", *share);
        }
        // The box round the selection, for a script to frame.
        if let Some(frame) = self.layout.as_ref().and_then(|(_, _, layout)| self.layout_frame(layout)) {
            let (low, high) = (frame.centre - frame.size * 0.5, frame.centre + frame.size * 0.5);
            out.set("selection_box", Value::Array(vec![Value::from(vec![low.x, low.y]), Value::from(vec![high.x, high.y])]));
        }
        out.build()
    }
}

/// What a side, or a mirror image, of the layout selects in a mode: its
/// places, edges or islands, and what they are called.
fn side_items(layout: &Layout, mode: Mode, side: &uv::symmetry::Side) -> (Vec<Item>, (&'static str, &'static str)) {
    match mode {
        Mode::Edges => (side.edges.iter().map(|&e| Item::Edge(layout.uv.edges[e as usize].vertices)).collect(), ("edge", "edges")),
        Mode::Vertices => (side.places.iter().map(|&g| Item::Vertex(layout.uv.uv_vertices[g as usize].vertex)).collect(), ("vertex", "vertices")),
        _ => (side.islands.iter().filter_map(|&i| editing::island_item(layout, i)).collect(), ("island", "islands")),
    }
}

/// A shape tool's options, as a script gives them: a string, a yes or no,
/// a number, each where a key names it.
fn text<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("`{key}` is a word")),
    }
}

fn flag(value: &Value, key: &str) -> Result<Option<bool>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` is true or false")),
    }
}

fn number(value: &Value, key: &str) -> Result<Option<f32>, String> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) if n.is_finite() => Ok(Some(*n as f32)),
        Some(_) => Err(format!("`{key}` is a number")),
    }
}

/// A distance's unit and texture, where a script gives them.
fn distance_unit(value: &Value, s: &mut Settings) -> Result<(), String> {
    if let Some(unit) = text(value, "unit")? {
        s.unit = match unit {
            "mm" => MarginUnit::Millimetres,
            "px" => MarginUnit::Pixels,
            other => return Err(format!("`{other}` is not a unit: mm or px")),
        };
    }
    if let Some(texture) = number(value, "texture")? {
        s.texture = texture.clamp(64.0, 16384.0) as u32;
    }
    Ok(())
}

/// The right-click menu's shape tools, in two submenus.
pub(super) fn context_menu(ui: &mut egui::Ui, editor: &UvEditor, layout: &Layout) -> Option<Tool> {
    let mut out = None;
    let mut item = |ui: &mut egui::Ui, tool: Tool| {
        let enabled = editor.can_shape(tool, Some(layout)).is_ok();
        let button = egui::Button::new(tool.name()).shortcut_text(crate::theme::keys(&tool.keys().replace(" + ", "+")));
        if ui.add_enabled(enabled, button).clicked() {
            out = Some(tool);
            ui.close_menu();
        }
    };
    ui.menu_button("Shape", |ui| {
        for tool in [Tool::Unwrap, Tool::Straight, Tool::Grid, Tool::Rectangle, Tool::Project] {
            item(ui, tool);
        }
        ui.separator();
        for tool in [Tool::Relax, Tool::Circle, Tool::Grow, Tool::Shrink, Tool::Weld, Tool::MatchSeams, Tool::CutAngle] {
            item(ui, tool);
        }
    });
    ui.menu_button("Symmetry", |ui| {
        for tool in [Tool::Symmetrize, Tool::SnapMirror, Tool::SelectSide, Tool::SelectMirror] {
            item(ui, tool);
        }
    });
    out
}

/// The shape tools as a person or a script uses them: frame after frame
/// without a window, each one step to undo, a refusal saying why.
#[cfg(test)]
mod tests {
    use super::super::*;
    use std::sync::Arc;
    use tequila_core::math::{vec2, vec3};
    use tequila_scene::command::{reserve_keys, ImportPayload};
    use tequila_scene::{SceneDocument, SceneNode};
    use tequila_format::json::Value;
    use tequila_scripting::protocol::UvEditorChange;

    fn part_with(mesh: tequila_scene::Mesh) -> (Project, NodeId) {
        let document = SceneDocument::new();
        let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 1);
        let mut node = SceneNode::new(ids[0], "Panel", NodeKind::Part { mesh: meshes[0] });
        node.material = Some(materials[0]);
        let payload = ImportPayload {
            meshes: vec![(meshes[0], mesh)],
            textures: Vec::new(),
            materials: vec![(materials[0], tequila_scene::MaterialDefinition::plastic("Shell"))],
            nodes: vec![node],
            roots: vec![ids[0]],
        };
        let mut project = Project::with_document(document, Arc::new(tequila_scene::demo::studio_environment()));
        project.apply(Command::Import { payload: Box::new(payload) }).unwrap();
        project.end_interaction();
        (project, ids[0])
    }

    /// A flat sheet `columns` by `rows` quads, a twentieth of a metre a
    /// side each, laid out as it lies but for its first row bowed.
    fn sheet(rows: usize, columns: usize) -> tequila_scene::Mesh {
        let mut positions = Vec::new();
        for r in 0..=rows {
            for c in 0..=columns {
                positions.push(vec3(c as f32 * 0.05, 0.0, r as f32 * 0.05));
            }
        }
        let at = |r: usize, c: usize| (r * (columns + 1) + c) as u32;
        let mut indices = Vec::new();
        for r in 0..rows {
            for c in 0..columns {
                indices.extend_from_slice(&[at(r, c), at(r + 1, c), at(r + 1, c + 1), at(r, c), at(r + 1, c + 1), at(r, c + 1)]);
            }
        }
        let uv = positions
            .iter()
            .map(|p| {
                let bow = if p.z == 0.0 { (p.x * 20.0).sin() * 0.01 } else { 0.0 };
                vec2(0.1 + p.x * 2.0, 0.1 + p.z * 2.0 + bow)
            })
            .collect();
        tequila_scene::Mesh::new(positions, indices).with_uvs(uv)
    }

    fn mesh_of(project: &Project, node: NodeId) -> tequila_scene::Mesh {
        let key = project.document().tree.get(node).and_then(|n| n.mesh()).unwrap();
        project.document().geometry.get(key).unwrap().clone()
    }

    fn frames(editor: &mut UvEditor, project: &mut Project, ctx: &egui::Context, count: usize) {
        for _ in 0..count {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))), ..Default::default() };
            let _ = ctx.run(input, |ctx| show(ctx, editor, project));
        }
    }

    fn editor_on(project: &mut Project, node: NodeId, ctx: &egui::Context) -> UvEditor {
        let mut editor = UvEditor::default();
        editor.open_on(node);
        frames(&mut editor, project, ctx, 3);
        editor
    }

    /// A script's change, the shape keys given as JSON.
    fn ask(editor: &mut UvEditor, project: &mut Project, ctx: &egui::Context, mut change: UvEditorChange, shape: &str) -> Result<(), String> {
        if !shape.is_empty() {
            let Value::Object(keys) = tequila_format::json::parse(shape.as_bytes()).unwrap() else { panic!("an object") };
            change.shape = tequila_scripting::protocol::SHAPE_KEYS.iter().filter_map(|k| keys.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
        }
        editor.script(&change, project)?;
        frames(editor, project, ctx, 2);
        Ok(())
    }

    fn message(editor: &UvEditor) -> (String, bool) {
        editor.message.as_ref().map(|(text, refused, _)| (text.clone(), *refused)).unwrap_or_default()
    }

    /// The bowed first row, picked and laid straight: on one line, in one
    /// step, which undo takes back.
    #[test]
    fn a_picked_row_is_laid_straight_in_one_step() {
        let (mut project, node) = part_with(sheet(2, 6));
        let ctx = egui::Context::default();
        let mut editor = editor_on(&mut project, node, &ctx);
        let before = mesh_of(&project, node);
        let row: Vec<usize> = (0..7).collect();
        ask(&mut editor, &mut project, &ctx, UvEditorChange { vertices: Some(row.clone()), ..Default::default() }, r#"{"straight": true}"#).unwrap();
        let (text, refused) = message(&editor);
        assert!(!refused && text.starts_with("Laid 7 vertices on a straight line across"), "{text}");
        let after = mesh_of(&project, node);
        let level = after.uv_sets[0][0].y;
        assert!(row.iter().all(|&v| (after.uv_sets[0][v].y - level).abs() < 1e-5), "on one line");
        assert!(tequila_scene::uv::UvLayout::new(&after, 0).unwrap().folded_triangles().is_empty());
        project.undo();
        assert_eq!(mesh_of(&project, node), before, "one step to undo");
    }

    /// A tool that does not apply says why in the status bar and changes
    /// nothing; a bad key changes nothing either, and says what it takes.
    #[test]
    fn a_refusal_says_why_and_a_bad_key_changes_nothing() {
        let (mut project, node) = part_with(sheet(2, 6));
        let ctx = egui::Context::default();
        let mut editor = editor_on(&mut project, node, &ctx);
        let before = mesh_of(&project, node);
        let steps = project.undo_step();
        // Two vertices that share no edge are no path.
        ask(&mut editor, &mut project, &ctx, UvEditorChange { vertices: Some(vec![0, 20]), ..Default::default() }, r#"{"straight": true}"#).unwrap();
        let (text, refused) = message(&editor);
        assert!(refused && text.starts_with("Could not lay it straight: "), "{text}");
        assert_eq!(mesh_of(&project, node), before);
        assert_eq!(project.undo_step(), steps, "nothing to undo");
        let settings = editor.shape.settings;
        let bad = ask(&mut editor, &mut project, &ctx, UvEditorChange::default(), r#"{"relax": {"rounds": 9}, "match_seams": {"line_up": "sideways"}}"#);
        assert!(bad.unwrap_err().contains("sideways"));
        assert_eq!(editor.shape.settings, settings, "nothing set");
        assert!(editor.shape.pending.is_empty(), "nothing asked");
    }

    /// The sheet as a grid, its rows square; relaxed, it has nothing to
    /// change, and says so; grown and unwrapped again with least stretch,
    /// each its own step.
    #[test]
    fn grid_relax_grow_and_unwrap_again_are_each_one_step() {
        let (mut project, node) = part_with(sheet(3, 5));
        let ctx = egui::Context::default();
        let mut editor = editor_on(&mut project, node, &ctx);
        let start = project.undo_step();
        ask(&mut editor, &mut project, &ctx, UvEditorChange::default(), r#"{"grid": {"even": true}}"#).unwrap();
        let (text, refused) = message(&editor);
        assert!(!refused && text.starts_with("Laid 15 faces out as a grid, 5 by 3"), "{text}");
        let grid = mesh_of(&project, node);
        let ys: Vec<f32> = (0..6).map(|c| grid.uv_sets[0][c].y).collect();
        assert!(ys.iter().all(|y| (y - ys[0]).abs() < 1e-5), "the first row straight: {ys:?}");
        for (keys, starts) in [
            (r#"{"relax": {"rounds": 10}}"#, "Nothing to relax"),
            (r#"{"grow": {"by": 4, "unit": "px", "texture": 1024}}"#, "Grew the border of 1 island by 4 px"),
        ] {
            ask(&mut editor, &mut project, &ctx, UvEditorChange::default(), keys).unwrap();
            let (text, refused) = message(&editor);
            assert!(!refused && text.starts_with(starts), "{keys}: {text}");
        }
        ask(&mut editor, &mut project, &ctx, UvEditorChange { unwrap: true, ..Default::default() }, r#"{"unwrap_options": {"method": "least_stretch", "keep": true}}"#).unwrap();
        let (text, refused) = message(&editor);
        assert!(!refused && text.starts_with("Laid 1 island out again with least stretch"), "{text}");
        assert_ne!(project.undo_step(), start);
        for _ in 0..3 {
            project.undo();
        }
        assert_eq!(project.undo_step(), start, "three steps, one each");
    }

    /// A part symmetric across X: one side selected, its mirror selected,
    /// and the layout made symmetric, a dent on the minus side gone.
    #[test]
    fn one_side_its_mirror_and_symmetrize() {
        // A sheet from x = -0.15 to 0.15, laid out as it lies, its minus
        // half dented.
        let mut mesh = sheet(2, 6);
        for p in &mut mesh.positions {
            p.x -= 0.15;
        }
        let dented: Vec<Vec2> = mesh.positions.iter().map(|p| vec2(0.5 + p.x * 2.0, 0.3 + p.z * 2.0 + if p.x < -0.01 && p.z > 0.01 { 0.01 } else { 0.0 })).collect();
        mesh.uv_sets[0] = dented;
        let (mut project, node) = part_with(mesh);
        let ctx = egui::Context::default();
        let mut editor = editor_on(&mut project, node, &ctx);
        ask(&mut editor, &mut project, &ctx, UvEditorChange { mode: Some("vertices".into()), ..Default::default() }, r#"{"select_side": "+x"}"#).unwrap();
        let plus = editor.selection.vertices();
        assert_eq!(plus.len(), 9, "three columns of three");
        ask(&mut editor, &mut project, &ctx, UvEditorChange::default(), r#"{"select_mirror": {"add": false}}"#).unwrap();
        let minus = editor.selection.vertices();
        assert_eq!(minus.len(), 9);
        assert!(minus.iter().all(|v| !plus.contains(v)), "the other side");
        ask(&mut editor, &mut project, &ctx, UvEditorChange { vertices: Some(Vec::new()), ..Default::default() }, r#"{"symmetrize": {"plane": "x", "line": "vertical", "at": "layout", "source": "plus"}}"#).unwrap();
        let (text, refused) = message(&editor);
        assert!(!refused && text.starts_with("Made the −X side the mirror image of the +X side"), "{text}");
        let out = mesh_of(&project, node);
        for (v, p) in out.positions.iter().enumerate() {
            assert!((out.uv_sets[0][v].y - (0.3 + p.z * 2.0)).abs() < 1e-5, "the dent is gone at {v}");
        }
    }

    /// Cut by angle on a folded sheet cuts along its crease and unwraps
    /// again, one step; the edges it would cut are counted while its
    /// options are open.
    #[test]
    fn cut_by_angle_cuts_the_crease_and_unwraps_in_one_step() {
        // Two flat halves at a right angle, laid out as one.
        let mut mesh = sheet(2, 4);
        for p in &mut mesh.positions {
            if p.x > 0.1 {
                *p = vec3(0.1, p.x - 0.1, p.z);
            }
        }
        let (mut project, node) = part_with(mesh);
        let ctx = egui::Context::default();
        let mut editor = editor_on(&mut project, node, &ctx);
        let steps = project.undo_step();
        ask(&mut editor, &mut project, &ctx, UvEditorChange { mode: Some("islands".into()), ..Default::default() }, r#"{"shape_panel": "shape", "shape_options": "cut_angle"}"#).unwrap();
        frames(&mut editor, &mut project, &ctx, 2);
        let state = editor.shape_state();
        assert_eq!(state.get("cut_edges").and_then(|v| v.as_index()), Some(2), "the crease's two edges lit");
        ask(&mut editor, &mut project, &ctx, UvEditorChange::default(), r#"{"cut_angle": {"angle": 30, "unwrap_after": true}}"#).unwrap();
        let (text, refused) = message(&editor);
        assert!(!refused && text.starts_with("Cut along 2 edges sharper than 30°"), "{text}");
        assert_eq!(tequila_scene::uv::UvLayout::new(&mesh_of(&project, node), 0).unwrap().islands.len(), 2);
        project.undo();
        assert_eq!(project.undo_step(), steps, "the cut and the unwrap, one step");
    }
}
