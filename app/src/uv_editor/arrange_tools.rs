//! The arranging tools: Align and Distribute on whatever the mode picks -
//! islands whole, vertices, each picked edge path on its own - and the
//! Arrange panel's: square a strip's edges, turn islands upright, sort them,
//! size them alike, fit them to the square, stack the same shapes, copy a
//! shape and paste it, swap and shuffle, and lay them out as on the part.
//!
//! The engine works each out (`tequila_scene::uv::{arrange, sort, similar,
//! vertices}`) and the editor sends it as one step: islands as
//! `uv.place_islands`, vertices as `uv.set_positions`. What needs the part's
//! own mesh - its shape on the part, where it is - waits for the end of the
//! frame's panels, where the document is at hand (`Request`), so a menu, a
//! key and a script ask for it the same way the panel does.

use super::align::{self, Align, Target};
use super::editing;
use super::layout::Layout;
use super::selection::Mode;
use super::{Edits, LayoutEdit, Part, UvEditor};
use crate::icons::{self, Icon};
use crate::project::Project;
use tequila_core::math::{vec2, Vec2, Vec3};
use tequila_scene::uv::arrange::{self, Plan, SizeTarget, Spread};
use tequila_scene::uv::similar::{self, IslandShape};
use tequila_scene::uv::sort::{self, Grouping, PartAxes, SortBy, SortOptions, View, Wrap};
use tequila_scene::uv::vertices::{self, LineUp, Places};
use tequila_scene::uv::UvRect;
use tequila_scene::Command;

/// The Arrange panel's tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    SquareEdges,
    Upright,
    Sort,
    MatchSize,
    Fit,
    Stack,
    CopyShape,
    PasteShape,
    Shuffle,
    AsOnPart,
}

impl Tool {
    pub fn icon(self) -> Icon {
        match self {
            Tool::SquareEdges => Icon::SquareEdges,
            Tool::Upright => Icon::Upright,
            Tool::Sort => Icon::Sort,
            Tool::MatchSize => Icon::MatchSize,
            Tool::Fit => Icon::FitSquare,
            Tool::Stack => Icon::Stack,
            Tool::CopyShape => Icon::Copy,
            Tool::PasteShape => Icon::Paste,
            Tool::Shuffle => Icon::Shuffle,
            Tool::AsOnPart => Icon::AsOnPart,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::SquareEdges => "Line up edges",
            Tool::Upright => "Upright",
            Tool::Sort => "Sort",
            Tool::MatchSize => "Match size",
            Tool::Fit => "Fit to square",
            Tool::Stack => "Stack similar",
            Tool::CopyShape => "Copy shape",
            Tool::PasteShape => "Paste shape",
            Tool::Shuffle => "Swap",
            Tool::AsOnPart => "As on the part",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Tool::SquareEdges => "Make the edges that run nearly across or down the layout run exactly so, moving only their vertices: for tidying strips. On the picked edges, or the selected islands, or all",
            Tool::Upright => "Turn each selected island, or all, so the part's up runs up the layout",
            Tool::Sort => "Lay the selected islands, or all, out in a row, a column or a grid, in order of where they are on the part or on the layout",
            Tool::MatchSize => "Scale the selected islands to the same width or height as the island last clicked, the largest or their average",
            Tool::Fit => "Scale and move the selected islands, or all, to fill the layout's square or the box between the guides, keeping their proportions",
            Tool::Stack => "Lay the islands of the same shape as the island last clicked exactly over it, corner for corner",
            Tool::CopyShape => "Keep the shape of the island last clicked, to paste onto islands of the same piece",
            Tool::PasteShape => "Give the selected islands the shape copied, each where it is, or over the one copied",
            Tool::Shuffle => "Swap two selected islands' places; more go round into each other's places",
            Tool::AsOnPart => "Lay the selected islands, or all, out as they sit on the part, seen from its front, none overlapping",
        }
    }

    /// Its shortcut, where it has one.
    pub fn keys(self) -> &'static str {
        match self {
            Tool::Upright => "Shift + U",
            Tool::Sort => "Shift + O",
            Tool::Stack => "Shift + S",
            Tool::Fit => "Shift + F",
            Tool::Shuffle => "Shift + X",
            _ => "",
        }
    }

    /// What it is called in a sentence saying it could not be done.
    fn verb(self) -> &'static str {
        match self {
            Tool::SquareEdges => "line up the edges",
            Tool::Upright => "turn them upright",
            Tool::Sort => "sort",
            Tool::MatchSize => "match the sizes",
            Tool::Fit => "fit them",
            Tool::Stack => "stack",
            Tool::CopyShape => "copy the shape",
            Tool::PasteShape => "paste the shape",
            Tool::Shuffle => "swap",
            Tool::AsOnPart => "lay them out as on the part",
        }
    }
}

/// What is asked of the tools that need the part's own mesh, done once the
/// panels are drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Request {
    Tool(Tool),
    /// Spread across, down, or along the picked path.
    Distribute(Way),
    Select(super::select_tools::Pick),
    DensityGet,
    DensityApply(f32),
    CursorToSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Way {
    Across,
    Down,
    Along,
}

/// What Sort orders by, as the panel offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Axis(usize),
    Around(usize),
    Grid,
    Layout,
}

impl Order {
    pub fn key(self) -> &'static str {
        match self {
            Order::Axis(0) => "x",
            Order::Axis(1) => "y",
            Order::Axis(_) => "z",
            Order::Around(0) => "around_x",
            Order::Around(1) => "around_y",
            Order::Around(_) => "around_z",
            Order::Grid => "grid",
            Order::Layout => "layout",
        }
    }

    pub fn from_key(key: &str) -> Option<Order> {
        [Order::Axis(0), Order::Axis(1), Order::Axis(2), Order::Around(0), Order::Around(1), Order::Around(2), Order::Grid, Order::Layout].into_iter().find(|o| o.key() == key)
    }
}

/// How wide a gap is given: millimetres of the part, or pixels of a texture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gap {
    pub amount: f32,
    pub unit: super::actions::MarginUnit,
    pub texture: u32,
}

impl Gap {
    /// In the layout's units, on a layout of `density` metres a unit; a gap
    /// in millimetres needs one.
    pub fn uv(&self, density: Option<f32>) -> Option<f32> {
        super::actions::PackSettings { margin: self.amount, unit: self.unit, texture: self.texture, ..Default::default() }.margin_uv(density)
    }
}

/// The tools' settings, the shape copied, and what waits for the end of
/// the frame.
#[derive(Clone, Debug)]
pub struct Arrange {
    pub spread: Spread,
    /// Along a path: in proportion to its edges on the part, not evenly.
    pub by_part: bool,
    /// How far from square an edge may be to be squared, in degrees.
    pub tolerance: f32,
    pub order: Order,
    /// The 3D grid's rows and columns, by axis.
    pub rows: usize,
    pub columns: usize,
    pub across: bool,
    pub line: i8,
    pub gap: Gap,
    pub wrap: Wrap,
    pub group: Grouping,
    pub reverse: bool,
    pub size_to: SizeTarget,
    pub size_across: bool,
    pub keep_proportions: bool,
    pub fit_each: bool,
    /// Stacking and pasting take mirrored pieces too.
    pub mirrored: bool,
    /// Each stacked copy a step from the last, in the layout's units.
    pub offset: Option<Vec2>,
    /// How alike two islands' shapes must be, as a share.
    pub similar: f32,
    pub in_place: bool,
    pub view: View,
    /// As on the part turns each island to read as seen, and scales it to
    /// the layout's own scale.
    pub turn_as_seen: bool,
    pub true_size: bool,
    /// Bumped at each shuffle: the next order drawn.
    pub seed: u64,
    pub copied: Option<(String, IslandShape)>,
    pub pending: Vec<Request>,
    /// Sort's options, asked open or shut by a script.
    pub sort_open: Option<bool>,
    /// The side panel scrolled to one of these panels, as a script asks.
    pub scroll_to: Option<String>,
    /// View options, asked open or shut by a script.
    pub view_open: Option<bool>,
}

impl Default for Arrange {
    fn default() -> Self {
        Arrange {
            spread: Spread::Gaps,
            by_part: false,
            tolerance: 15.0,
            order: Order::Axis(0),
            rows: 1,
            columns: 0,
            across: true,
            line: 0,
            gap: Gap { amount: 8.0, unit: super::actions::MarginUnit::Pixels, texture: 2048 },
            wrap: Wrap::Width,
            group: Grouping::None,
            reverse: false,
            size_to: SizeTarget::Largest,
            size_across: true,
            keep_proportions: true,
            fit_each: false,
            mirrored: true,
            offset: None,
            similar: 0.05,
            in_place: false,
            view: View::Front,
            turn_as_seen: true,
            true_size: true,
            seed: 1,
            copied: None,
            pending: Vec::new(),
            sort_open: None,
            scroll_to: None,
            view_open: None,
        }
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// A word to add where a plan leaves islands outside the layout's square:
/// where to go next.
fn beyond(layout: &Layout, plan: &Plan) -> &'static str {
    let outside = plan.islands.iter().zip(&plan.maps).any(|(&i, map)| {
        let island = &layout.uv.islands[i as usize];
        [island.min, island.max, vec2(island.min.x, island.max.y), vec2(island.max.x, island.min.y)].iter().map(|c| map.apply(*c)).any(|p| p.x < -1e-4 || p.y < -1e-4 || p.x > 1.0001 || p.y > 1.0001)
    });
    if outside { "; some are outside the square: Fit to square brings them in" } else { "" }
}

/// The world's axes in a part's own space, as it stands in the scene.
pub fn axes_of(project: &Project, part: &Part) -> PartAxes {
    let world = project.document().tree.world_transform(part.node);
    let local = |v: Vec3| world.inv_vector(v).try_normalize().unwrap_or(v);
    PartAxes([local(Vec3::X), local(Vec3::Y), local(Vec3::Z)])
}

impl UvEditor {
    // ---- what the tools work on ------------------------------------------------

    /// The island the tools take as the one to match: the island last
    /// clicked where it is among the chosen or nothing is chosen, else the
    /// last one selected.
    pub(super) fn active_island(&self, layout: &Layout) -> Option<u32> {
        let chosen = self.chosen_islands(layout);
        let clicked = self.island.map(|i| i as u32).filter(|&i| (i as usize) < layout.uv.islands.len());
        match clicked {
            Some(i) if chosen.is_empty() || chosen.contains(&i) => Some(i),
            _ => self.selection.island_triangles().last().and_then(|&t| layout.uv.island_of.get(t as usize).copied()).or(chosen.last().copied()),
        }
    }

    /// The picked vertices, as the layout's UV vertices.
    fn picked_vertices(&self, layout: &Layout) -> Vec<u32> {
        layout.uv.uv_vertices_of(&self.selection.vertices())
    }

    /// The picked edges, as the layout's.
    fn picked_edges(&self, layout: &Layout) -> Vec<u32> {
        self.selection.edges().into_iter().filter_map(|e| layout.edge_named(e)).collect()
    }

    fn place(&self, edits: &mut Edits, part: &Part, layout: &Layout, plan: Plan, verb: &'static str, done: String) {
        let command = Command::PlaceUvIslands { node: part.node, set: layout.uv.set, islands: plan.islands, maps: plan.maps };
        edits.layout.push(LayoutEdit { command, verb, done, carry: true });
    }

    fn put(&self, edits: &mut Edits, part: &Part, layout: &Layout, places: Places, verb: &'static str, done: String) {
        let command = Command::SetUvPositions { node: part.node, set: layout.uv.set, vertices: places.vertices, uv: places.uv };
        edits.layout.push(LayoutEdit { command, verb, done, carry: true });
    }

    /// What Align lines up against, as a box: none where each lines up
    /// against its own, or a word saying why there is nothing to.
    fn align_reference(&self, part: &Part, layout: &Layout) -> Result<Option<align::Box2>, &'static str> {
        match self.align_to {
            Target::Selection => Ok(None),
            Target::Island => self.island.and_then(|i| layout.boxes.get(i).copied()).map(Some).ok_or("Could not align: click an island to align to it"),
            Target::Layout => Ok(Some((Vec2::ZERO, Vec2::splat(1.0)))),
            Target::Cursor => self.cursor_of(part).map(|c| Some((c, c))).ok_or("Could not align: put the 2D cursor down first, with Shift and a right click"),
        }
    }

    // ---- Align and Distribute on the layout's own ------------------------------

    /// The layout's selection lined up as the Align panel says: islands each
    /// moved whole; vertices to one line, or onto the line between the two
    /// outermost for the middles; each picked edge path on its own.
    pub(super) fn align_layout(&mut self, part: &Part, layout: &Layout, edits: &mut Edits, how: Align) {
        let reference = match self.align_reference(part, layout) {
            Ok(r) => r,
            Err(why) => {
                self.say(why, true);
                return;
            }
        };
        // "by their bottom right corners, on the 2D cursor"
        let by = how.name().trim_start_matches("Align ");
        let against = match self.align_to {
            Target::Selection if self.mode == Mode::Edges => "each against its own",
            Target::Selection => "against the selection",
            Target::Island => "against the island",
            Target::Layout => "against the layout's square",
            Target::Cursor => "on the 2D cursor",
        };
        let done = format!("by their {by}, {against}");
        match self.mode {
            Mode::Vertices | Mode::Edges => {
                let groups: Vec<Vec<u32>> = if self.mode == Mode::Vertices {
                    vec![self.picked_vertices(layout)]
                } else {
                    vertices::edge_chains(&layout.uv, &self.picked_edges(layout)).into_iter().map(|c| c.vertices).collect()
                };
                let count: usize = groups.iter().map(|g| g.len()).sum();
                let needed = if reference.is_none() { 2 } else { 1 };
                if count < needed {
                    self.say(if self.mode == Mode::Vertices { "Could not align: select two vertices or more" } else { "Could not align: select edges to line up" }, true);
                    return;
                }
                let (across, down) = how.sides();
                let middles = matches!(how, Align::Centre | Align::Middle);
                let places = if self.mode == Mode::Vertices && middles {
                    if count < 3 {
                        self.say("Could not align: put three vertices or more on the line between the outermost", true);
                        return;
                    }
                    vertices::onto_line(&layout.uv, &groups)
                } else {
                    let span = |a: bool| reference.map(|(low, high)| if a { (low.x, high.x) } else { (low.y, high.y) });
                    let mut merged = Places::default();
                    for (side, is_across) in [(across, true), (down, false)] {
                        let Some(side) = side else { continue };
                        let lined = vertices::line_up(&layout.uv, &groups, LineUp { across: is_across, side, against: span(is_across) });
                        for (v, [u, w]) in lined.vertices.into_iter().zip(lined.uv) {
                            let now = merged.vertices.iter().position(|&m| m == v).map(|i| merged.uv[i]);
                            let at = match (now, is_across) {
                                (Some([_, w0]), true) => vec2(u, w0),
                                (Some([u0, _]), false) => vec2(u0, w),
                                (None, _) => vec2(u, w),
                            };
                            merged.push(v, at);
                        }
                    }
                    merged
                };
                if places.is_empty() {
                    self.say("They are lined up already", false);
                    return;
                }
                let what = if self.mode == Mode::Vertices { plural(places.vertices.len(), "vertex", "vertices") } else { plural(groups.len(), "edge path", "edge paths") };
                let done = if self.mode == Mode::Vertices && middles { format!("Put {what} on the line between the outermost") } else { format!("Lined up {what} {done}") };
                self.put(edits, part, layout, places, "line them up", done);
            }
            _ => {
                let islands = editing::islands(layout, &self.selection);
                if islands.is_empty() {
                    self.say("Could not align: select islands to line them up", true);
                    return;
                }
                let boxes: Vec<align::Box2> = islands.iter().map(|&i| layout.boxes[i as usize]).collect();
                let target = reference.unwrap_or_else(|| if boxes.len() > 1 { align::union(&boxes).expect("some") } else { (Vec2::ZERO, Vec2::splat(1.0)) });
                let mut plan = Plan::default();
                for (&island, delta) in islands.iter().zip(align::align(&boxes, target, how)) {
                    plan.push(island, tequila_scene::uv::UvAffine::translation(delta));
                }
                let plan = plan.moving();
                if plan.is_empty() {
                    self.say("The islands are lined up already", false);
                    return;
                }
                self.place(edits, part, layout, plan, "line them up", format!("Lined up {} {done}", plural(islands.len(), "island", "islands")));
            }
        }
    }

    /// Islands spread with equal gaps or middles; a picked path's vertices
    /// spread along it.
    fn distribute_layout(&mut self, part: &Part, layout: &Layout, mesh: &tequila_scene::Mesh, edits: &mut Edits, way: Way) {
        if self.mode == Mode::Islands || way != Way::Along && !matches!(self.mode, Mode::Vertices | Mode::Edges) {
            let islands = editing::islands(layout, &self.selection);
            if islands.len() < 3 {
                self.say("Could not spread them: select three islands or more", true);
                return;
            }
            let plan = arrange::distribute(&layout.uv, &islands, way != Way::Down, self.arrange.spread);
            if plan.is_empty() {
                self.say("The islands are spread evenly already", false);
                return;
            }
            let how = if self.arrange.spread == Spread::Gaps { "equal gaps" } else { "their middles evenly" };
            self.place(edits, part, layout, plan, "spread them", format!("Spread {} {} with {how}", plural(islands.len(), "island", "islands"), if way == Way::Down { "down" } else { "across" }));
            return;
        }
        let chains = if self.mode == Mode::Vertices { vertices::vertex_chains(&layout.uv, &self.picked_vertices(layout)) } else { vertices::edge_chains(&layout.uv, &self.picked_edges(layout)) };
        let paths: Vec<Vec<u32>> = chains.into_iter().filter(|c| c.path && c.vertices.len() >= 3).map(|c| c.vertices).collect();
        if paths.is_empty() {
            self.say("Could not spread them: pick a path of three vertices or more, joined by edges, with two ends", true);
            return;
        }
        let mut places = Places::default();
        for path in &paths {
            let spread = vertices::spread(&layout.uv, mesh, path, self.arrange.by_part);
            for (v, [u, w]) in spread.vertices.into_iter().zip(spread.uv) {
                places.push(v, vec2(u, w));
            }
        }
        if places.is_empty() {
            self.say("The path's vertices are spread evenly already", false);
            return;
        }
        let how = if self.arrange.by_part { "as the part has them" } else { "evenly" };
        self.put(edits, part, layout, places, "spread them", format!("Spread the vertices of {} {how}", plural(paths.len(), "path", "paths")));
    }

    // ---- the Arrange panel's tools ---------------------------------------------

    /// Whether a tool can be used now, or why not.
    pub(super) fn can_arrange(&self, tool: Tool, layout: Option<&Layout>) -> Result<(), &'static str> {
        let Some(layout) = layout else { return Err("Unwrap the part first") };
        match tool {
            Tool::Stack | Tool::CopyShape if self.active_island(layout).is_none() => Err("Click an island first"),
            Tool::PasteShape if self.arrange.copied.is_none() => Err("Copy an island's shape first"),
            Tool::PasteShape if self.chosen_islands(layout).is_empty() => Err("Select the islands to paste the shape onto"),
            Tool::MatchSize if self.arrange.size_to == SizeTarget::Island(0) && self.active_island(layout).is_none() => Err("Click the island to match first"),
            Tool::Shuffle if self.chosen_islands(layout).len() == 1 => Err("Select two islands or more to swap them"),
            _ => Ok(()),
        }
    }

    /// Do one of the Arrange panel's tools on the selection, or on every
    /// island where nothing is selected.
    fn arrange_tool(&mut self, tool: Tool, part: &Part, layout: &Layout, project: &Project, mesh: &tequila_scene::Mesh, edits: &mut Edits) {
        if let Err(why) = self.can_arrange(tool, Some(layout)) {
            self.say(&format!("Could not {}: {}", tool.verb(), why.to_lowercase()), true);
            return;
        }
        let refuse = |editor: &mut UvEditor, why: &str| editor.say(&format!("Could not {}: {why}", tool.verb()), true);
        let islands = self.islands_or_all(layout);
        let uv = &layout.uv;
        match tool {
            Tool::SquareEdges => {
                let picked = self.picked_edges(layout);
                let edges: Vec<u32> = if !picked.is_empty() && self.mode == Mode::Edges {
                    picked
                } else {
                    islands.iter().flat_map(|&i| uv.islands[i as usize].edges.clone()).map(|e| e as u32).collect()
                };
                let (places, report) = vertices::square_edges_reported(uv, mesh, &edges, self.arrange.tolerance.to_radians());
                if places.is_empty() {
                    self.say(&format!("No edges within {:.0}° of square to line up", self.arrange.tolerance), false);
                    return;
                }
                let found = report.across.len() + report.down.len();
                let mut done = format!("Lined up {} within {:.0}° of square, moving {}", plural(found - report.left.len(), "edge", "edges"), self.arrange.tolerance, plural(places.vertices.len() - report.carried, "vertex", "vertices"));
                if report.carried > 0 {
                    done.push_str(&format!(" and carrying {} along", plural(report.carried, "vertex", "vertices")));
                }
                if !report.left.is_empty() {
                    let clashing = report.left.iter().filter(|e| report.clashing.contains(e)).count();
                    let folding = report.left.len() - clashing;
                    let mut why = Vec::new();
                    if clashing > 0 {
                        why.push(format!("{clashing} would put two vertices on one place or flatten a face"));
                    }
                    if folding > 0 {
                        why.push(format!("{folding} would fold the layout"));
                    }
                    done.push_str(&format!("; {} left as {}: {}", plural(report.left.len(), "edge", "edges"), if report.left.len() == 1 { "it was" } else { "they were" }, why.join(", ")));
                }
                self.put(edits, part, layout, places, tool.verb(), done);
            }
            Tool::Upright => {
                let up = axes_of(project, part).0[1];
                let (plan, flat) = arrange::upright(uv, mesh, &islands, up);
                if plan.is_empty() && flat.is_empty() {
                    self.say("The islands are upright already", false);
                    return;
                }
                if plan.is_empty() {
                    refuse(self, &format!("{} face straight up or down, so have no up to turn to", plural(flat.len(), "island", "islands")));
                    return;
                }
                let left = if flat.is_empty() { String::new() } else { format!("; {} facing up or down left as they were", plural(flat.len(), "island", "islands")) };
                let done = format!("Turned {} upright{left}", plural(plan.islands.len(), "island", "islands"));
                self.place(edits, part, layout, plan, tool.verb(), done);
            }
            Tool::Sort => {
                let Some(spacing) = self.arrange.gap.uv(layout.density()) else {
                    refuse(self, "this layout is not one size all over, so a gap in millimetres means nothing: give it in pixels");
                    return;
                };
                let by = match self.arrange.order {
                    Order::Axis(a) => SortBy::Axis(a),
                    Order::Around(a) => SortBy::Around(a),
                    Order::Grid => SortBy::Grid { rows: self.arrange.rows, columns: self.arrange.columns },
                    Order::Layout => SortBy::Layout,
                };
                let options = SortOptions { by, across: self.arrange.across, align: self.arrange.line, spacing, wrap: self.arrange.wrap, group: self.arrange.group, group_spacing: spacing * 4.0, reverse: self.arrange.reverse };
                let plan = sort::sort(uv, mesh, &islands, &options, &axes_of(project, part));
                if plan.is_empty() {
                    self.say("The islands are in that order already", false);
                    return;
                }
                let done = format!("Sorted {} into {} by {}{}", plural(islands.len(), "island", "islands"), if self.arrange.across { "rows" } else { "columns" }, order_words(self.arrange.order), beyond(layout, &plan));
                self.place(edits, part, layout, plan, tool.verb(), done);
            }
            Tool::MatchSize => {
                let target = match self.arrange.size_to {
                    SizeTarget::Island(_) => SizeTarget::Island(self.active_island(layout).expect("checked")),
                    other => other,
                };
                let plan = match arrange::match_size(uv, &islands, target, self.arrange.size_across, self.arrange.keep_proportions) {
                    Ok(plan) => plan,
                    Err(error) => return refuse(self, &error.to_string()),
                };
                if plan.is_empty() {
                    self.say("The islands are that size already", false);
                    return;
                }
                let to = match target {
                    SizeTarget::Island(i) => format!("island {}", i + 1),
                    SizeTarget::Largest => "the largest".into(),
                    SizeTarget::Average => "their average".into(),
                };
                let done = format!("Matched {} to the {} of {to}", plural(plan.islands.len(), "island", "islands"), if self.arrange.size_across { "width" } else { "height" });
                self.place(edits, part, layout, plan, tool.verb(), done);
            }
            Tool::Fit => {
                let region = match self.pack.into {
                    super::actions::PackInto::Square => UvRect::UNIT,
                    super::actions::PackInto::Guides => match super::actions::between_guides(&part.guides) {
                        Some(region) => region,
                        None => return refuse(self, "drag out two guides across or down the layout to fit between them, or fit into the square in Pack's options"),
                    },
                };
                let margin = self.pack.margin_uv(layout.density()).unwrap_or(0.0) * 0.5;
                let plan = match arrange::fit(uv, &islands, region, margin, self.arrange.fit_each) {
                    Ok(plan) => plan,
                    Err(error) => return refuse(self, &error.to_string()),
                };
                if plan.is_empty() {
                    self.say("The islands fill it already", false);
                    return;
                }
                let into = if self.pack.into == super::actions::PackInto::Guides { "the box between the guides" } else { "the square" };
                let done = format!("Fitted {} to {into}{}", plural(islands.len(), "island", "islands"), if self.arrange.fit_each { ", each on its own" } else { "" });
                self.place(edits, part, layout, plan, tool.verb(), done);
            }
            Tool::Stack => {
                let active = self.active_island(layout).expect("checked");
                let selected = self.chosen_islands(layout);
                let first = IslandShape::of(uv, mesh, active);
                // The selected others, or where only the one is chosen, every
                // island of its shape.
                let candidates: Vec<u32> = if selected.len() > 1 { selected.into_iter().filter(|&i| i != active).collect() } else { (0..uv.islands.len() as u32).filter(|&i| i != active).collect() };
                let mut places = Places::default();
                let mut stacked = 0usize;
                for &other in &candidates {
                    let theirs = IslandShape::of(uv, mesh, other);
                    let Some(m) = similar::similar(&first, &theirs, self.arrange.similar, self.arrange.mirrored) else { continue };
                    stacked += 1;
                    let offset = self.arrange.offset.unwrap_or(Vec2::ZERO) * stacked as f32;
                    let (v, p) = similar::laid_over(&first, &theirs, &m, offset);
                    for (v, [u, w]) in v.into_iter().zip(p) {
                        places.push(v, vec2(u, w));
                    }
                }
                if stacked == 0 {
                    return refuse(self, &format!("no other island is the same piece and shape as island {}{}", active + 1, if self.arrange.mirrored { "" } else { " (mirrored pieces are left out)" }));
                }
                let places = places.moving(uv);
                let done = format!("Stacked {} on island {}{}", plural(stacked, "island", "islands"), active + 1, if self.arrange.offset.is_some() { ", each a step from the last" } else { "" });
                self.put(edits, part, layout, places, tool.verb(), done);
            }
            Tool::CopyShape => {
                let active = self.active_island(layout).expect("checked");
                let shape = IslandShape::of(uv, mesh, active);
                let faces = shape.triangles.len();
                self.arrange.copied = Some((format!("island {} of {}", active + 1, part.name), shape));
                self.say(&format!("Copied the shape of island {} ({})", active + 1, plural(faces, "face", "faces")), false);
            }
            Tool::PasteShape => {
                let (name, copied) = self.arrange.copied.clone().expect("checked");
                let targets = self.chosen_islands(layout);
                let mut places = Places::default();
                let (mut pasted, mut other) = (0usize, 0usize);
                for &island in &targets {
                    let theirs = IslandShape::of(uv, mesh, island);
                    let Some(m) = similar::correspond(&copied, &theirs, self.arrange.mirrored) else {
                        other += 1;
                        continue;
                    };
                    let offset = if self.arrange.in_place { Vec2::ZERO } else { theirs.centre() - copied.centre() };
                    let (v, p) = similar::laid_over(&copied, &theirs, &m, offset);
                    for (v, [u, w]) in v.into_iter().zip(p) {
                        places.push(v, vec2(u, w));
                    }
                    pasted += 1;
                }
                if pasted == 0 {
                    return refuse(self, &format!("{} not the same piece as {name}", if targets.len() == 1 { "the island is" } else { "none of the islands is" }));
                }
                let places = places.moving(uv);
                if places.is_empty() {
                    self.say("They have that shape already", false);
                    return;
                }
                let left = if other > 0 { format!("; {} of another piece left", plural(other, "island", "islands")) } else { String::new() };
                let done = format!("Pasted the shape of {name} onto {}{}{left}", plural(pasted, "island", "islands"), if self.arrange.in_place { ", over the one copied" } else { "" });
                self.put(edits, part, layout, places, tool.verb(), done);
            }
            Tool::Shuffle => {
                if islands.len() < 2 {
                    return refuse(self, "there is only one island");
                }
                self.arrange.seed = self.arrange.seed.wrapping_add(1);
                let plan = arrange::shuffle(uv, &islands, self.arrange.seed);
                let done = if islands.len() == 2 { "Swapped 2 islands".to_string() } else { format!("Shuffled {} into each other's places", plural(islands.len(), "island", "islands")) };
                self.place(edits, part, layout, plan, tool.verb(), done);
            }
            Tool::AsOnPart => {
                let Some(density) = arrange::linear_density(uv, &islands) else {
                    return refuse(self, "the islands have no area");
                };
                let gap = self.pack.margin_uv(layout.density()).unwrap_or(0.004);
                let options = sort::AsOnPart { view: self.arrange.view, turn: self.arrange.turn_as_seen, true_size: self.arrange.true_size, gap };
                let plan = sort::as_on_part(uv, mesh, &islands, &axes_of(project, part), density, &options);
                if plan.is_empty() {
                    self.say("The islands are laid out as on the part already", false);
                    return;
                }
                let seen = match self.arrange.view {
                    View::Front => "from the front",
                    View::Side => "from the side",
                    View::Top => "from above",
                    View::Auto => "from where they spread most",
                };
                let done = format!("Laid {} out as on the part, seen {seen}{}", plural(islands.len(), "island", "islands"), beyond(layout, &plan));
                self.place(edits, part, layout, plan, tool.verb(), done);
            }
        }
    }

    /// What was asked of the tools that need the part's mesh, done now.
    pub(super) fn arrange_pending(&mut self, part: &Part, layout: Option<&Layout>, project: &Project, edits: &mut Edits) {
        let pending = std::mem::take(&mut self.arrange.pending);
        if pending.is_empty() {
            return;
        }
        let (Some(layout), Some(mesh)) = (layout, project.document().geometry.get(part.mesh)) else {
            self.say("The part has no layout to work on: unwrap it first", true);
            return;
        };
        for request in pending {
            match request {
                Request::Tool(tool) => self.arrange_tool(tool, part, layout, project, mesh, edits),
                Request::Distribute(way) => self.distribute_layout(part, layout, mesh, edits, way),
                Request::Select(pick) => self.select_pick(pick, part, layout, mesh, edits),
                Request::DensityGet => self.density_get(part, layout, mesh),
                Request::DensityApply(pixels) => self.density_apply(pixels, part, layout, edits),
                Request::CursorToSelection => {
                    let labels = super::canvas::labels_of(edits, part).to_vec();
                    self.cursor_to_selection(part, layout, &labels);
                }
            }
        }
    }

    // ---- the panel ---------------------------------------------------------------

    pub(super) fn arrange_panel(&mut self, ui: &mut egui::Ui, layout: Option<&Layout>) {
        let p = crate::theme::p();
        if layout.is_none() {
            ui.weak("Unwrap the part to arrange its islands.");
            return;
        }
        let what = match (self.mode, self.selection.is_layout()) {
            (Mode::Labels, _) | (_, false) => "On every island; select some to work on those".to_string(),
            _ => format!("On the {} selected", plural(self.chosen_islands(layout.expect("checked")).len(), "island", "islands")),
        };
        ui.label(egui::RichText::new(what).size(12.0).color(p.text_dim));
        let mut asked: Option<Tool> = None;
        let mut tool_button = |ui: &mut egui::Ui, editor: &mut UvEditor, tool: Tool| {
            let ready = editor.can_arrange(tool, layout);
            ui.add_enabled_ui(ready.is_ok(), |ui| {
                let response = icons::labelled(ui, tool.icon(), tool.name(), false);
                let detail = match ready {
                    Ok(()) => tool.detail(),
                    Err(why) => why,
                };
                if icons::tip(response, tool.name(), detail, tool.keys()).clicked() {
                    asked = Some(tool);
                }
            });
        };
        let options = |ui: &mut egui::Ui, id: &str, title: &str, asked: Option<bool>, add: &mut dyn FnMut(&mut egui::Ui)| {
            let id = ui.id().with(id);
            let showing = ui.memory(|m| m.is_popup_open(id));
            let response = icons::tip(icons::button(ui, Icon::Sliders, showing, 30.0, egui::Sense::click()), title, "", "");
            match (response.clicked(), asked) {
                (true, _) => ui.memory_mut(|m| m.toggle_popup(id)),
                (false, Some(true)) => ui.memory_mut(|m| m.open_popup(id)),
                (false, Some(false)) if showing => ui.memory_mut(|m| m.close_popup()),
                _ => {}
            }
            egui::popup::popup_below_widget(ui, id, &response, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                ui.set_min_width(270.0);
                add(ui);
            });
        };

        crate::theme::section(ui, "Square and upright");
        ui.horizontal_wrapped(|ui| {
            tool_button(ui, self, Tool::SquareEdges);
            ui.add(egui::DragValue::new(&mut self.arrange.tolerance).range(1.0..=44.0).speed(0.2).suffix("°").max_decimals(1))
                .on_hover_text("How far from across or down an edge may run and still be lined up");
            tool_button(ui, self, Tool::Upright);
        });

        crate::theme::section(ui, "Order");
        ui.horizontal_wrapped(|ui| {
            tool_button(ui, self, Tool::Sort);
            let scale = self.scale;
            let asked = self.arrange.sort_open.take();
            let arrange = &mut self.arrange;
            options(ui, "uv-sort-options", "Sort options", asked, &mut |ui| sort_options(ui, arrange, scale));
            ui.label(egui::RichText::new(format!("by {}", order_words(self.arrange.order))).size(12.0).color(p.text_dim));
        });
        ui.horizontal_wrapped(|ui| {
            tool_button(ui, self, Tool::Shuffle);
            tool_button(ui, self, Tool::AsOnPart);
            let mut view = self.arrange.view;
            egui::ComboBox::from_id_salt("uv-as-on-part-view").width(70.0).selected_text(view_word(view)).show_ui(ui, |ui| {
                for v in [View::Front, View::Side, View::Top, View::Auto] {
                    ui.selectable_value(&mut view, v, view_word(v));
                }
            });
            self.arrange.view = view;
            let arrange = &mut self.arrange;
            options(ui, "uv-as-on-part-options", "As on the part options", None, &mut |ui| {
                ui.add(crate::field::Checkbox::new(&mut arrange.turn_as_seen, "Turn each to read as seen"))
                    .on_hover_text("Each island turned so what runs across the view runs across the layout and what runs down it down; one seen from behind reads as seen through the part");
                ui.add(crate::field::Checkbox::new(&mut arrange.true_size, "At the layout's scale"))
                    .on_hover_text("Each island scaled to the layout's own scale, so it is as large beside the others as it is on the part");
            });
        });

        crate::theme::section(ui, "Size");
        ui.horizontal_wrapped(|ui| {
            tool_button(ui, self, Tool::MatchSize);
            let arrange = &mut self.arrange;
            options(ui, "uv-match-options", "Match size options", None, &mut |ui| {
                crate::theme::section(ui, "To");
                let mut to = match arrange.size_to {
                    SizeTarget::Island(_) => 0,
                    SizeTarget::Largest => 1,
                    SizeTarget::Average => 2,
                };
                if crate::theme::segmented(ui, &mut to, &[(0, "Island clicked"), (1, "Largest"), (2, "Average")]) {
                    arrange.size_to = [SizeTarget::Island(0), SizeTarget::Largest, SizeTarget::Average][to];
                }
                crate::theme::section(ui, "By");
                crate::theme::segmented(ui, &mut arrange.size_across, &[(true, "Width"), (false, "Height")]);
                ui.add(crate::field::Checkbox::new(&mut arrange.keep_proportions, "Keep proportions")).on_hover_text("Scale both ways; off, only the width or the height is stretched");
            });
            tool_button(ui, self, Tool::Fit);
            ui.add(crate::field::Checkbox::new(&mut self.arrange.fit_each, "Each on its own")).on_hover_text("Fit each island to the square by itself, rather than all of them together");
        });

        crate::theme::section(ui, "Same shape");
        ui.horizontal_wrapped(|ui| {
            tool_button(ui, self, Tool::Stack);
            let scale = self.scale;
            let arrange = &mut self.arrange;
            options(ui, "uv-stack-options", "Stack options", None, &mut |ui| {
                ui.add(crate::field::Checkbox::new(&mut arrange.mirrored, "Mirrored pieces too")).on_hover_text("A piece that is the other's mirror image - a left and a right - is stacked too, and prints mirrored");
                let mut offset = arrange.offset.is_some();
                if ui.add(crate::field::Checkbox::new(&mut offset, "Offset")).on_hover_text("Each copy a step from the last instead of exactly on top").changed() {
                    arrange.offset = offset.then_some(vec2(0.02, 0.0));
                }
                if let Some(step) = &mut arrange.offset {
                    ui.horizontal(|ui| {
                        let mut across = scale.shown(step.x);
                        let mut down = scale.shown(step.y);
                        let a = ui.add(egui::DragValue::new(&mut across).speed(0.1).prefix("Across ").suffix(format!(" {}", scale.unit.suffix())).max_decimals(3)).changed();
                        let d = ui.add(egui::DragValue::new(&mut down).speed(0.1).prefix("Down ").suffix(format!(" {}", scale.unit.suffix())).max_decimals(3)).changed();
                        if a || d {
                            *step = vec2(scale.layout(across), scale.layout(down));
                        }
                    });
                }
                let mut percent = arrange.similar * 100.0;
                if ui.add(crate::field::Slider::new(&mut percent, 0.5..=50.0).text("Shape tolerance").suffix("%").fixed_decimals(1)).on_hover_text("How far two islands of the same piece may differ in shape and still count as alike").changed() {
                    arrange.similar = percent / 100.0;
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            tool_button(ui, self, Tool::CopyShape);
            tool_button(ui, self, Tool::PasteShape);
            ui.add(crate::field::Checkbox::new(&mut self.arrange.in_place, "In place")).on_hover_text("Paste exactly over the island copied, rather than where each island is");
        });
        if let Some((name, shape)) = &self.arrange.copied {
            ui.label(egui::RichText::new(format!("Copied: {name}, {}", plural(shape.triangles.len(), "face", "faces"))).size(12.0).color(p.text_dim));
        }
        if let Some(tool) = asked {
            self.arrange.pending.push(Request::Tool(tool));
        }
    }

    /// The side panel scrolled to show the panel named, once, where a
    /// script asked for it.
    pub(super) fn scroll_to_panel(&mut self, ui: &mut egui::Ui, name: &str, header: &egui::Response) {
        if self.arrange.scroll_to.as_deref() == Some(name) {
            ui.scroll_to_rect(header.rect, Some(egui::Align::TOP));
            self.arrange.scroll_to = None;
        }
    }

    /// The Align panel for the layout's own: what it lines up against, the
    /// sides, middles and corners, and the spreads.
    pub(super) fn align_panel(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&Layout>, edits: &mut Edits) {
        let targets = Target::ALL.map(|t| (t, t.label()));
        crate::theme::segmented(ui, &mut self.align_to, &targets);
        ui.add_space(4.0);
        let layout_mode = self.mode.is_layout();
        let (count, things) = match (layout_mode, layout) {
            (true, Some(layout)) => match self.mode {
                Mode::Vertices => (self.selection.vertices().len(), "vertices"),
                Mode::Edges => (self.selection.edges().len(), "edges"),
                _ => (editing::islands(layout, &self.selection).len(), "islands"),
            },
            _ => (self.selected_labels(super::canvas::labels_of(edits, part)).len(), "labels"),
        };
        let cursor_missing = self.align_to == Target::Cursor && self.cursor_of(part).is_none();
        let can_align = count >= 1 && !cursor_missing;
        let detail = |how: Align| -> String {
            if cursor_missing {
                return "Put the 2D cursor down first, with Shift and a right click".into();
            }
            if count == 0 {
                return format!("Select {things} to line them up");
            }
            if self.mode == Mode::Vertices && matches!(how, Align::Centre | Align::Middle) {
                return "Put the selected vertices on the straight line between the two farthest apart".into();
            }
            let against = match self.align_to {
                Target::Selection if self.mode == Mode::Edges => "each edge path's own box",
                Target::Selection => "the box round the selection; one alone, the layout's square",
                Target::Island => "the island last clicked",
                Target::Layout => "the layout's square",
                Target::Cursor => "the 2D cursor",
            };
            format!("To {against}")
        };
        let details: Vec<(Align, String)> = Align::ALL.into_iter().chain(Align::CORNERS).map(|how| (how, detail(how))).collect();
        let detail = |how: Align| details.iter().find(|(h, _)| *h == how).map(|(_, d)| d.clone()).unwrap_or_default();
        let mut clicked = None;
        for row in [&Align::ALL[..], &Align::CORNERS[..]] {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for &how in row {
                    ui.add_enabled_ui(can_align, |ui| {
                        let response = icons::button(ui, how.icon(), false, 28.0, egui::Sense::click());
                        if icons::tip(response, how.name(), &detail(how), "").clicked() {
                            clicked = Some(how);
                        }
                    });
                }
                if row.len() == 4 {
                    icons::vertical_divider(ui, 22.0);
                    let path = layout_mode && matches!(self.mode, Mode::Vertices | Mode::Edges);
                    if path {
                        ui.add_enabled_ui(count >= 2, |ui| {
                            let response = icons::button(ui, Icon::SpreadAlong, false, 28.0, egui::Sense::click());
                            if icons::tip(response, "Spread along the path", "Spread the vertices of the picked path along it, its ends staying put: evenly, or as the part's edges run", "").clicked() {
                                self.arrange.pending.push(Request::Distribute(Way::Along));
                            }
                        });
                    } else {
                        for (way, icon, name) in [(Way::Across, Icon::DistributeAcross, "Spread across"), (Way::Down, Icon::DistributeDown, "Spread down")] {
                            ui.add_enabled_ui(count >= 3, |ui| {
                                let response = icons::button(ui, icon, false, 28.0, egui::Sense::click());
                                let detail = format!("Equal gaps between three or more {things}, the first and last staying put");
                                if icons::tip(response, name, &detail, "").clicked() {
                                    if layout_mode {
                                        self.arrange.pending.push(Request::Distribute(way));
                                    } else if let Some(layout) = layout {
                                        self.distribute(part, layout, edits, way == Way::Across);
                                    }
                                }
                            });
                        }
                    }
                }
            });
        }
        if layout_mode {
            match self.mode {
                Mode::Vertices | Mode::Edges => {
                    crate::theme::segmented(ui, &mut self.arrange.by_part, &[(false, "Evenly"), (true, "As on the part")]);
                }
                _ => {
                    crate::theme::segmented(ui, &mut self.arrange.spread, &[(Spread::Gaps, "Equal gaps"), (Spread::Centres, "Equal middles")]);
                }
            }
        }
        if let (Some(how), Some(layout)) = (clicked, layout) {
            self.align(part, edits, layout, how);
        }
    }
}

/// Where Sort's order comes from, in a few words.
pub fn order_words(order: Order) -> &'static str {
    match order {
        Order::Axis(0) => "X on the part",
        Order::Axis(1) => "height on the part",
        Order::Axis(_) => "Z on the part",
        Order::Around(0) => "the angle round X",
        Order::Around(1) => "the angle round Y",
        Order::Around(_) => "the angle round Z",
        Order::Grid => "rows and columns on the part",
        Order::Layout => "where they are on the layout",
    }
}

fn view_word(view: View) -> &'static str {
    match view {
        View::Front => "Front",
        View::Side => "Side",
        View::Top => "Top",
        View::Auto => "Auto",
    }
}

/// The Sort popover.
fn sort_options(ui: &mut egui::Ui, arrange: &mut Arrange, scale: super::units::Scale) {
    crate::theme::section(ui, "Order by");
    let orders = [
        (Order::Axis(0), "X on the part"),
        (Order::Axis(1), "Y, the top first"),
        (Order::Axis(2), "Z on the part"),
        (Order::Around(0), "Round X"),
        (Order::Around(1), "Round Y"),
        (Order::Around(2), "Round Z"),
        (Order::Grid, "A grid on the part"),
        (Order::Layout, "Where they are now"),
    ];
    egui::ComboBox::from_id_salt("uv-sort-by").width(200.0).selected_text(orders.iter().find(|(o, _)| *o == arrange.order).map(|(_, t)| *t).unwrap_or("")).show_ui(ui, |ui| {
        for (order, text) in orders {
            ui.selectable_value(&mut arrange.order, order, text);
        }
    });
    if arrange.order == Order::Grid {
        ui.horizontal(|ui| {
            let axes = [(0usize, "X"), (1, "Y"), (2, "Z")];
            ui.label("Rows by");
            egui::ComboBox::from_id_salt("uv-sort-rows").width(40.0).selected_text(axes[arrange.rows.min(2)].1).show_ui(ui, |ui| {
                for (a, t) in axes {
                    ui.selectable_value(&mut arrange.rows, a, t);
                }
            });
            ui.label("columns by");
            egui::ComboBox::from_id_salt("uv-sort-columns").width(40.0).selected_text(axes[arrange.columns.min(2)].1).show_ui(ui, |ui| {
                for (a, t) in axes {
                    ui.selectable_value(&mut arrange.columns, a, t);
                }
            });
        });
    }
    crate::theme::section(ui, "Lay out");
    crate::theme::segmented(ui, &mut arrange.across, &[(true, "In rows"), (false, "In columns")]);
    crate::theme::segmented(ui, &mut arrange.line, &[(-1, "Start"), (0, "Centre"), (1, "End")]);
    crate::theme::section(ui, "Gap");
    crate::theme::segmented(ui, &mut arrange.gap.unit, &[(super::actions::MarginUnit::Millimetres, "mm"), (super::actions::MarginUnit::Pixels, "px")]);
    ui.horizontal(|ui| {
        let suffix = if arrange.gap.unit == super::actions::MarginUnit::Millimetres { " mm" } else { " px" };
        ui.add(egui::DragValue::new(&mut arrange.gap.amount).range(0.0..=500.0).speed(0.1).max_decimals(2).suffix(suffix)).on_hover_text("The space between islands");
        if arrange.gap.unit == super::actions::MarginUnit::Pixels {
            ui.add(egui::DragValue::new(&mut arrange.gap.texture).range(64..=16384).speed(16.0).prefix("of a texture ").suffix(" px")).on_hover_text("The side of the texture the gap is pixels of");
        }
    });
    let mut wrap = match arrange.wrap {
        Wrap::Never => 0,
        Wrap::After(_) => 1,
        Wrap::Width => 2,
    };
    crate::theme::section(ui, "Wrap");
    if crate::theme::segmented(ui, &mut wrap, &[(0, "Never"), (1, "After"), (2, "At the edge")]) {
        arrange.wrap = match wrap {
            0 => Wrap::Never,
            1 => Wrap::After(4),
            _ => Wrap::Width,
        };
    }
    if let Wrap::After(n) = &mut arrange.wrap {
        ui.add(egui::DragValue::new(n).range(1..=1000).prefix("After ").suffix(" islands"));
    }
    crate::theme::section(ui, "Group by");
    crate::theme::segmented(ui, &mut arrange.group, &[(Grouping::None, "None"), (Grouping::Size, "Size"), (Grouping::Shape, "Shape"), (Grouping::Closeness, "Nearness")]);
    ui.add(crate::field::Checkbox::new(&mut arrange.reverse, "Reverse the order"));
    let _ = scale;
}

/// A script's sort settings, from `{"by": "x", "flow": "row", ...}`.
pub fn sort_from(arrange: &mut Arrange, value: &tequila_format::json::Value) -> Result<(), String> {
    let text = |key: &str| value.get(key).and_then(|v| v.as_str());
    let axis = |key: &str| -> Result<Option<usize>, String> {
        match text(key) {
            None => Ok(None),
            Some("x") => Ok(Some(0)),
            Some("y") => Ok(Some(1)),
            Some("z") => Ok(Some(2)),
            Some(other) => Err(format!("`{other}` is not an axis: x, y or z")),
        }
    };
    if let Some(by) = text("by") {
        arrange.order = Order::from_key(by).ok_or_else(|| format!("`{by}` is not an order: x, y, z, around_x, around_y, around_z, grid or layout"))?;
    }
    if let Some(rows) = axis("rows")? {
        arrange.rows = rows;
    }
    if let Some(columns) = axis("columns")? {
        arrange.columns = columns;
    }
    match text("flow") {
        None => {}
        Some("row") | Some("rows") => arrange.across = true,
        Some("column") | Some("columns") => arrange.across = false,
        Some(other) => return Err(format!("`{other}` is not a flow: row or column")),
    }
    match text("align") {
        None => {}
        Some("start") => arrange.line = -1,
        Some("centre") | Some("center") => arrange.line = 0,
        Some("end") => arrange.line = 1,
        Some(other) => return Err(format!("`{other}` is not an alignment: start, centre or end")),
    }
    if let Some(gap) = value.get("spacing").and_then(|v| v.as_f32()) {
        arrange.gap.amount = gap.clamp(0.0, 500.0);
    }
    match text("unit") {
        None => {}
        Some("mm") => arrange.gap.unit = super::actions::MarginUnit::Millimetres,
        Some("px") => arrange.gap.unit = super::actions::MarginUnit::Pixels,
        Some(other) => return Err(format!("`{other}` is not a unit: mm or px")),
    }
    if let Some(texture) = value.get("texture").and_then(|v| v.as_index()) {
        arrange.gap.texture = (texture as u32).clamp(64, 16384);
    }
    match value.get("wrap") {
        None => {}
        Some(tequila_format::json::Value::Bool(false)) => arrange.wrap = Wrap::Never,
        Some(tequila_format::json::Value::String(s)) if s == "width" => arrange.wrap = Wrap::Width,
        Some(v) => match v.as_index() {
            Some(n) if n > 0 => arrange.wrap = Wrap::After(n),
            _ => return Err("`wrap` is a number of islands, \"width\" or false".into()),
        },
    }
    match text("group") {
        None => {}
        Some("none") => arrange.group = Grouping::None,
        Some("size") => arrange.group = Grouping::Size,
        Some("shape") => arrange.group = Grouping::Shape,
        Some("closeness") | Some("nearness") => arrange.group = Grouping::Closeness,
        Some(other) => return Err(format!("`{other}` is not a grouping: none, size, shape or closeness")),
    }
    if let Some(reverse) = value.get("reverse").and_then(|v| v.as_bool()) {
        arrange.reverse = reverse;
    }
    Ok(())
}

// ---- the scripting channel ------------------------------------------------------

impl UvEditor {
    /// A script's arranging keys (`tequila_scripting::protocol::UvArrange`),
    /// carried out as the panels, the menu and the keys do them. Checked
    /// first, so a request with one bad field changes nothing.
    pub(super) fn script_arrange(&mut self, change: &tequila_scripting::protocol::UvArrange) -> Result<(), String> {
        use tequila_format::json::Value;
        use tequila_scripting::protocol::UvCursor;
        let align_to = match &change.align_to {
            Some(key) => Some(Target::from_key(key).ok_or_else(|| format!("`{key}` is not what to align to: selection, island, layout or cursor"))?),
            None => None,
        };
        let pivot = match change.pivot.as_deref() {
            None => None,
            Some("cursor") => Some(true),
            Some("frame") | Some("selection") => Some(false),
            Some(other) => return Err(format!("`{other}` is not a pivot: cursor or frame")),
        };
        let distribute = match change.distribute.as_deref() {
            None => None,
            Some("across") => Some(Way::Across),
            Some("down") => Some(Way::Down),
            Some("along") => Some(Way::Along),
            Some(other) => return Err(format!("`{other}` is not a way to spread: across, down or along")),
        };
        let spread = match change.spread.as_deref() {
            None => None,
            Some("gaps") => Some((Some(Spread::Gaps), None)),
            Some("centres") | Some("centers") | Some("middles") => Some((Some(Spread::Centres), None)),
            Some("even") | Some("evenly") => Some((None, Some(false))),
            Some("part") => Some((None, Some(true))),
            Some(other) => return Err(format!("`{other}` is not a spread: gaps, centres, even or part")),
        };
        let square = match change.select_square.as_deref() {
            None => None,
            Some("across") | Some("horizontal") => Some(super::select_tools::Pick::Across),
            Some("down") | Some("vertical") => Some(super::select_tools::Pick::Down),
            Some(other) => return Err(format!("`{other}` is not a way edges run: across or down")),
        };
        let text = |value: &Value, key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_string);
        let flag = |value: &Value, key: &str| value.get(key).and_then(|v| v.as_bool());
        let number = |value: &Value, key: &str| value.get(key).and_then(|v| v.as_f32()).filter(|v| v.is_finite());
        // Settings first, each checked; then the tools.
        let mut settings = self.arrange.clone();
        if let Some(sort) = &change.sort {
            sort_from(&mut settings, sort)?;
        }
        if let Some(options) = &change.match_size {
            match text(options, "to").as_deref() {
                None => {}
                Some("active") | Some("island") => settings.size_to = SizeTarget::Island(0),
                Some("largest") => settings.size_to = SizeTarget::Largest,
                Some("average") => settings.size_to = SizeTarget::Average,
                Some(other) => return Err(format!("`{other}` is not a size to match: active, largest or average")),
            }
            match text(options, "dimension").as_deref() {
                None => {}
                Some("width") => settings.size_across = true,
                Some("height") => settings.size_across = false,
                Some(other) => return Err(format!("`{other}` is not a dimension: width or height")),
            }
            if let Some(keep) = flag(options, "keep_proportions") {
                settings.keep_proportions = keep;
            }
        }
        if let Some(each) = change.fit.as_ref().and_then(|o| flag(o, "each")) {
            settings.fit_each = each;
        }
        if let Some(options) = &change.stack {
            if let Some(mirrored) = flag(options, "mirrored") {
                settings.mirrored = mirrored;
            }
            match options.get("offset") {
                None => {}
                Some(Value::Bool(false)) | Some(Value::Null) => settings.offset = None,
                Some(v) => match v.as_f32_array().as_deref() {
                    Some(&[u, w]) if u.is_finite() && w.is_finite() => settings.offset = Some(vec2(u, w)),
                    _ => return Err("`offset` is a step across and down the layout, [du, dv], or false".into()),
                },
            }
            if let Some(tolerance) = number(options, "tolerance") {
                settings.similar = (tolerance / 100.0).clamp(0.005, 0.5);
            }
        }
        if let Some(in_place) = change.paste_shape.as_ref().and_then(|o| flag(o, "in_place")) {
            settings.in_place = in_place;
        }
        if let Some(options) = &change.as_on_part {
            match text(options, "view").as_deref() {
                None => {}
                Some("front") => settings.view = View::Front,
                Some("side") => settings.view = View::Side,
                Some("top") => settings.view = View::Top,
                Some("auto") => settings.view = View::Auto,
                Some(other) => return Err(format!("`{other}` is not a view: front, side, top or auto")),
            }
            if let Some(turn) = flag(options, "turn") {
                settings.turn_as_seen = turn;
            }
            if let Some(true_size) = flag(options, "true_size") {
                settings.true_size = true_size;
            }
        }
        if let Some(Some(degrees)) = change.square_edges {
            settings.tolerance = degrees.clamp(1.0, 44.0);
        }
        if let Some(Some(percent)) = change.select_similar {
            settings.similar = (percent / 100.0).clamp(0.005, 0.5);
        }
        if let Some((spread, by_part)) = spread {
            if let Some(spread) = spread {
                settings.spread = spread;
            }
            if let Some(by_part) = by_part {
                settings.by_part = by_part;
            }
        }
        let mut checker = self.checker;
        match &change.checker {
            None => {}
            Some(Value::Bool(on)) => checker.on = *on,
            Some(options) => {
                checker.on = flag(options, "on").unwrap_or(true);
                if let Some(squares) = options.get("squares").and_then(|v| v.as_index()) {
                    checker.squares = (squares as u32).clamp(1, 64);
                }
                if let Some(pattern) = text(options, "pattern") {
                    checker.pattern = super::checker::Pattern::from_key(&pattern).ok_or_else(|| format!("`{pattern}` is not a pattern: plain, letters or numbers"))?;
                }
                // The UV set it is drawn through: a set's number, from 0, or
                // "shown" to follow the editor's.
                match options.get("set") {
                    None => {}
                    Some(Value::String(s)) if s == "shown" => checker.set = None,
                    Some(v) => checker.set = Some(v.as_index().ok_or("the checker's `set` is a UV set's number, from 0, or \"shown\"")?),
                }
            }
        }
        let mut prefs = self.prefs;
        let mut use_for_pack = false;
        match &change.padding {
            None => {}
            Some(Value::Bool(on)) => prefs.padding = *on,
            Some(options) => {
                prefs.padding = flag(options, "on").unwrap_or(true);
                if let Some(texture) = options.get("texture").and_then(|v| v.as_index()) {
                    prefs.padding_texture = (texture as u32).clamp(64, 16384);
                }
                if let Some(px) = number(options, "px") {
                    prefs.padding_px = px.clamp(0.0, 512.0);
                }
                if flag(options, "suggest") == Some(true) {
                    prefs.padding_px = super::display::suggested_padding(prefs.padding_texture);
                }
                use_for_pack = flag(options, "use_for_pack") == Some(true);
            }
        }
        if let Some(stops) = change.exposure {
            prefs.background_exposure = stops.clamp(-4.0, 4.0);
        }
        if let Some(show) = change.show_cursor {
            prefs.show_cursor = show;
        }
        let mut texel = self.texel.clone();
        let density = match &change.density {
            Some(options) => {
                if let Some(texture) = options.get("texture").and_then(|v| v.as_index()) {
                    let texture = texture as u32;
                    if !super::density::SIZES.iter().any(|(s, _)| *s == texture) {
                        return Err(format!("{texture} is not a texture size: 512, 1024, 2048, 4096 or 8192"));
                    }
                    texel.texture = texture;
                }
                if let Some(field) = number(options, "field") {
                    texel.field = field.max(0.01);
                }
                let apply = match options.get("apply") {
                    None | Some(Value::Null) | Some(Value::Bool(false)) => None,
                    Some(Value::Bool(true)) => Some(texel.field),
                    Some(v) => Some(v.as_f32().filter(|p| p.is_finite() && *p > 0.0).ok_or("`apply` is a density in pixels per centimetre, or true for the field's")?),
                };
                (flag(options, "get") == Some(true), apply)
            }
            None => (false, None),
        };
        let behind = match change.behind.as_deref() {
            None => None,
            Some("none") => Some((None, false)),
            Some("checker") => Some((None, true)),
            Some(key) => Some((Some(tequila_materials::material_type::Param::ALL.iter().copied().find(|p| p.key() == key).ok_or_else(|| format!("`{key}` is not a parameter"))?), false)),
        };
        let node = self.shown_part();
        if change.cursor.is_some() && node.is_none() {
            return Err("the editor shows no part to put the cursor on".into());
        }
        // All read: now as the panels do it.
        self.arrange = settings;
        self.checker = checker;
        self.prefs = prefs;
        self.texel = texel;
        if use_for_pack {
            self.pack.margin = self.prefs.padding_px;
            self.pack.unit = super::actions::MarginUnit::Pixels;
            self.pack.texture = self.prefs.padding_texture;
        }
        if let Some(target) = align_to {
            self.align_to = target;
        }
        if let Some((behind, checker)) = behind {
            (self.display.behind, self.display.checker_behind) = (behind, checker);
        }
        if let Some(pivot) = pivot {
            self.cursors.pivot = pivot;
        }
        if let Some(open) = change.sort_options {
            self.arrange.sort_open = Some(open);
        }
        if let Some(open) = change.view_options {
            self.arrange.view_open = Some(open);
        }
        if let Some(panel) = &change.panel {
            if !["arrange", "density"].contains(&panel.as_str()) {
                return Err(format!("`{panel}` is not a panel to show: arrange or density"));
            }
            self.arrange.scroll_to = Some(panel.clone());
        }
        if let (Some(cursor), Some(node)) = (change.cursor, node) {
            match cursor {
                UvCursor::At([u, w]) => {
                    self.cursors.set(node, vec2(u, w));
                    self.prefs.show_cursor = true;
                }
                UvCursor::Centre => {
                    self.cursors.set(node, vec2(0.5, 0.5));
                    self.prefs.show_cursor = true;
                }
                UvCursor::Clear => self.cursors.clear(node),
                UvCursor::Selection => self.arrange.pending.push(Request::CursorToSelection),
            }
        }
        let pending = &mut self.arrange.pending;
        if let Some(way) = distribute {
            pending.push(Request::Distribute(way));
        }
        for (wanted, tool) in [
            (change.square_edges.is_some(), Tool::SquareEdges),
            (change.upright, Tool::Upright),
            (change.sort.is_some(), Tool::Sort),
            (change.match_size.is_some(), Tool::MatchSize),
            (change.fit.is_some(), Tool::Fit),
            (change.stack.is_some(), Tool::Stack),
            (change.copy_shape, Tool::CopyShape),
            (change.paste_shape.is_some(), Tool::PasteShape),
            (change.shuffle, Tool::Shuffle),
            (change.as_on_part.is_some(), Tool::AsOnPart),
        ] {
            if wanted {
                pending.push(Request::Tool(tool));
            }
        }
        use super::select_tools::Pick;
        for (wanted, pick) in [
            (change.select_similar.is_some(), Pick::Similar),
            (change.select_borders, Pick::Borders),
            (square == Some(Pick::Across), Pick::Across),
            (square == Some(Pick::Down), Pick::Down),
            (change.select_no_area, Pick::NoArea),
            (change.select_flipped, Pick::Flipped),
        ] {
            if wanted {
                pending.push(Request::Select(pick));
            }
        }
        if density.0 {
            pending.push(Request::DensityGet);
        }
        if let Some(pixels) = density.1 {
            pending.push(Request::DensityApply(pixels));
        }
        Ok(())
    }

    /// The arranging tools' part of the editor's state, for the `ui` query.
    pub(super) fn arrange_state(&self, out: &mut tequila_format::json_write::Object) {
        use tequila_format::json::Value;
        let pair = |p: Vec2| Value::Array(vec![Value::Number(p.x as f64), Value::Number(p.y as f64)]);
        let cursor = self.shown_part().and_then(|node| self.cursors.of(node));
        out.set("cursor", cursor.map(pair).unwrap_or(Value::Null))
            .set("show_cursor", self.prefs.show_cursor)
            .set("pivot", if self.cursors.pivot { "cursor" } else { "frame" })
            .set("align_to", self.align_to.key())
            .set("exposure", self.prefs.background_exposure)
            .set("behind", match self.display.behind {
                Some(p) => Value::String(p.key().to_string()),
                None if self.display.checker_behind => Value::String("checker".into()),
                None => Value::Null,
            })
            .set("copied", self.arrange.copied.as_ref().map(|(name, _)| Value::String(name.clone())).unwrap_or(Value::Null));
        let mut checker = tequila_format::json_write::Object::new();
        checker
            .set("on", self.checker.on)
            .set("squares", self.checker.squares as usize)
            .set("pattern", self.checker.pattern.key())
            .set("set", match self.checker.set {
                Some(set) => Value::Number(set as f64),
                None => Value::String("shown".into()),
            });
        out.set("checker", checker.build());
        let mut padding = tequila_format::json_write::Object::new();
        padding.set("on", self.prefs.padding).set("texture", self.prefs.padding_texture as usize).set("px", self.prefs.padding_px);
        if let Some(count) = self.display.too_close_known() {
            padding.set("too_close", count);
        }
        out.set("padding", padding.build());
        let mut density = tequila_format::json_write::Object::new();
        density.set("texture", self.texel.texture as usize).set("field", self.texel.field);
        if let Some((_, _, layout)) = self.layout.as_ref() {
            if let Some(d) = arrange::linear_density(&layout.uv, &self.islands_or_all(layout)) {
                density.set("px_per_cm", tequila_scene::uv::density::pixels_per_cm(d, self.texel.texture));
            }
        }
        if let Some(coverage) = self.texel.known_coverage() {
            density.set("coverage", coverage.covered).set("overlap", coverage.overlap);
        }
        out.set("density", density.build());
    }
}
