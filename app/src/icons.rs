//! The interface's icons.
//!
//! Most are [Lucide](https://lucide.dev)'s, from its font (ISC licence,
//! `scripts/notices/lucide`): one set, drawn together on a 24-unit grid
//! with round 2-unit strokes, rasterised by the font renderer so they are
//! crisp at any size and display scale, in whatever colour the state needs.
//!
//! A few are Tequila's own, drawn here: the brand mark, and the four
//! viewport shadings, which are spheres as Blender draws them and which no
//! icon set has. Each icon also has a drawn fallback, used where the font
//! is not installed — a headless test's context.

use crate::theme;
use eframe::egui;
use egui::{Color32, Pos2, Rect, Shape, Stroke};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// The brand mark, and the shutter on the Render button.
    Aperture,
    Wireframe,
    Solid,
    MaterialPreview,
    Rendered,
    Move,
    Rotate,
    Scale,
    World,
    Local,
    Magnet,
    FrameAll,
    FrameSelected,
    Pause,
    Play,
    Scene,
    Material,
    Environment,
    Light,
    Camera,
    Image,
    Studios,
    Script,
    Library,
    Console,
    Assistant,
    Import,
    Save,
    Eye,
    EyeClosed,
    Lock,
    Unlock,
    ChevronRight,
    ChevronDown,
    Group,
    Part,
    Cutaway,
    Search,
    Help,
    Close,
    More,
    /// A map driving a parameter: an image or a pattern.
    Texture,
    /// Light mode, dark mode, and the theme picker.
    Sun,
    Moon,
    Palette,
    /// A step done; a choice made.
    Check,
    /// The lighting presets: fastest, sunlit, the product shot, a room,
    /// a gem; and Custom, set by hand.
    Gauge,
    Package,
    House,
    Gem,
    Sliders,
    /// Work kept from a session that did not end cleanly.
    History,
    /// A search that found nothing.
    SearchX,
    /// A favourite; a recent one.
    Star,
    Clock,
    /// Make something new: a material, a plugin.
    Plus,
    /// A folder on disk, opened.
    FolderOpen,
    Copy,
    /// Change a thing's details.
    Pencil,
    Trash,
    /// Out of the application to a file, and in from one.
    Download,
    Upload,
    /// A plugin.
    Puzzle,
    /// Read again from disk.
    Refresh,
}

impl Icon {
    /// Its glyph in the Lucide font, or `None` for the ones drawn here.
    fn glyph(self) -> Option<char> {
        let code = match self {
            Icon::Aperture | Icon::Wireframe | Icon::Solid | Icon::MaterialPreview | Icon::Rendered => return None,
            Icon::Move => 0xe124,
            Icon::Rotate => 0xe2e9,
            Icon::Scale => 0xe116,
            Icon::World => 0xe0eb,
            Icon::Local => 0xe065,
            Icon::Magnet => 0xe2b4,
            Icon::FrameAll => 0xe256,
            Icon::FrameSelected => 0xe29d,
            Icon::Pause => 0xe131,
            Icon::Play => 0xe13f,
            Icon::Scene => 0xe40c,
            Icon::Material => 0xe2e6,
            Icon::Environment => 0xe17c,
            Icon::Light => 0xe1c1,
            Icon::Camera => 0xe068,
            Icon::Image => 0xe299,
            Icon::Studios => 0xe52d,
            Icon::Script => 0xe205,
            Icon::Library => 0xe5a3,
            Icon::Console => 0xe209,
            Icon::Assistant => 0xe416,
            Icon::Import => 0xe22e,
            Icon::Save => 0xe150,
            Icon::Eye => 0xe0be,
            Icon::EyeClosed => 0xe0bf,
            Icon::Lock => 0xe10e,
            Icon::Unlock => 0xe10f,
            Icon::ChevronRight => 0xe073,
            Icon::ChevronDown => 0xe071,
            Icon::Group => 0xe0dc,
            Icon::Part => 0xe065,
            Icon::Cutaway => 0xe151,
            Icon::Search => 0xe154,
            Icon::Help => 0xe082,
            Icon::Close => 0xe1b1,
            Icon::More => 0xe0ba,
            Icon::Texture => 0xe0f9,
            Icon::Sun => 0xe17b,
            Icon::Moon => 0xe121,
            Icon::Palette => 0xe1dc,
            Icon::Check => 0xe070,
            Icon::Gauge => 0xe1be,
            Icon::Package => 0xe12c,
            Icon::House => 0xe0f8,
            Icon::Gem => 0xe241,
            Icon::Sliders => 0xe165,
            Icon::History => 0xe1f4,
            Icon::SearchX => 0xe4b1,
            Icon::Star => 0xe179,
            Icon::Clock => 0xe08b,
            Icon::Plus => 0xe140,
            Icon::FolderOpen => 0xe246,
            Icon::Copy => 0xe0a2,
            Icon::Pencil => 0xe1f8,
            Icon::Trash => 0xe18d,
            Icon::Download => 0xe0b6,
            Icon::Upload => 0xe19d,
            Icon::Puzzle => 0xe29b,
            Icon::Refresh => 0xe148,
        };
        char::from_u32(code)
    }
}

/// How large a Lucide glyph is drawn for a square of a given side: the
/// font's em is a little larger than the icons' 24-unit grid, and this
/// makes the grid fill the square, as Lucide is drawn at its own size.
const GLYPH_SCALE: f32 = 0.9;

/// Draw `icon` centred in `rect`, as large as its shorter side allows.
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    if let Some(glyph) = icon.glyph() {
        if theme::icons_ready(painter.ctx()) {
            let side = rect.width().min(rect.height()) * GLYPH_SCALE;
            let font = egui::FontId::new(side, egui::FontFamily::Name(theme::ICONS.into()));
            let galley = painter.layout_no_wrap(glyph.to_string(), font, color);
            // Centred on its ink, not on the font's line box.
            let at = rect.center() - galley.mesh_bounds.center().to_vec2();
            painter.galley(at, galley, color);
            return;
        }
    }
    drawn(painter, rect, icon, color);
}

/// The icons drawn here: the brand mark and the shadings, and every icon's
/// fallback.
fn drawn(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let s = side / 16.0;
    let origin = rect.center() - egui::vec2(8.0 * s, 8.0 * s);
    let p = |x: f32, y: f32| origin + egui::vec2(x * s, y * s);
    let stroke = Stroke::new((1.4 * s).max(1.0), color);
    let line = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([p(a.0, a.1), p(b.0, b.1)], stroke);
    };
    let poly = |points: &[(f32, f32)]| {
        painter.add(Shape::line(points.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
    };
    let closed = |points: &[(f32, f32)]| {
        painter.add(Shape::closed_line(points.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
    };
    let fill = |points: &[(f32, f32)], colour: Color32| {
        painter.add(Shape::convex_polygon(points.iter().map(|&(x, y)| p(x, y)).collect(), colour, Stroke::NONE));
    };
    let circle = |x: f32, y: f32, r: f32| {
        painter.circle_stroke(p(x, y), r * s, stroke);
    };
    let disc = |x: f32, y: f32, r: f32, colour: Color32| {
        painter.circle_filled(p(x, y), r * s, colour);
    };
    // An elliptical arc from `a0` to `a1` radians, screen-wise (y down, so
    // increasing angles turn clockwise).
    let arc_points = |x: f32, y: f32, rx: f32, ry: f32, a0: f32, a1: f32| -> Vec<(f32, f32)> {
        let steps = (((a1 - a0).abs() / TAU) * 40.0).ceil().max(4.0) as usize;
        (0..=steps)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / steps as f32;
                (x + rx * a.cos(), y + ry * a.sin())
            })
            .collect()
    };
    let arc = |x: f32, y: f32, rx: f32, ry: f32, a0: f32, a1: f32| poly(&arc_points(x, y, rx, ry, a0, a1));
    let rounded = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        painter.rect_stroke(Rect::from_min_max(p(x0, y0), p(x1, y1)), r * s, stroke);
    };
    let rounded_fill = |x0: f32, y0: f32, x1: f32, y1: f32, r: f32| {
        painter.rect_filled(Rect::from_min_max(p(x0, y0), p(x1, y1)), r * s, color);
    };
    // Two strokes back from `tip` along `direction`, as an arrowhead.
    let head = |tip: (f32, f32), direction: (f32, f32), size: f32| {
        let length = (direction.0 * direction.0 + direction.1 * direction.1).sqrt().max(1e-6);
        let (dx, dy) = (direction.0 / length, direction.1 / length);
        let back = |turn: f32| {
            let (c, sn) = (turn.cos(), turn.sin());
            let (rx, ry) = (-dx * c + dy * sn, -dx * sn - dy * c);
            (tip.0 + rx * size, tip.1 + ry * size)
        };
        poly(&[back(0.7), tip, back(-0.7)]);
    };
    let soft = color.gamma_multiply(0.4);

    match icon {
        Icon::Aperture => {
            circle(8.0, 8.0, 6.6);
            // Six blades: from each corner of the central hexagon, along the
            // edge it ends, out to the rim.
            let r = 2.7;
            let corner = |k: i32| {
                let a = PI / 6.0 + k as f32 * PI / 3.0;
                (r * a.cos(), r * a.sin())
            };
            for k in 0..6 {
                let (px, py) = corner(k);
                let (qx, qy) = corner(k - 1);
                let (dx, dy) = (px - qx, py - qy);
                let length = (dx * dx + dy * dy).sqrt();
                let (dx, dy) = (dx / length, dy / length);
                let along = px * dx + py * dy;
                let t = -along + (along * along - (px * px + py * py - 6.6 * 6.6)).sqrt();
                line((8.0 + px, 8.0 + py), (8.0 + px + dx * t, 8.0 + py + dy * t));
            }
        }
        // The four shadings are spheres, drawn the way each shading shows
        // one: its wires; flat and matte; glossy; glossy, and standing in
        // its own shadow — the one thing only the render has.
        Icon::Wireframe => {
            let thin = Stroke::new((1.1 * s).max(0.9), color);
            painter.circle_stroke(p(8.0, 8.0), 6.4 * s, Stroke::new((1.35 * s).max(1.0), color));
            for (rx, ry) in [(6.4, 2.5), (2.6, 6.4)] {
                painter.add(Shape::line(arc_points(8.0, 8.0, rx, ry, 0.0, TAU).iter().map(|&(x, y)| p(x, y)).collect(), thin));
            }
        }
        Icon::Solid => sphere(painter, p(8.0, 8.0), 6.6 * s, color, Finish::Matte),
        Icon::MaterialPreview => sphere(painter, p(8.0, 8.0), 6.6 * s, color, Finish::Gloss),
        Icon::Rendered => {
            painter.add(Shape::ellipse_filled(p(8.0, 14.0), egui::vec2(5.6 * s, 1.3 * s), color.gamma_multiply(0.35)));
            sphere(painter, p(8.0, 7.2), 6.0 * s, color, Finish::Gloss);
        }
        Icon::Move => {
            line((8.0, 1.8), (8.0, 14.2));
            line((1.8, 8.0), (14.2, 8.0));
            head((8.0, 1.8), (0.0, -1.0), 2.6);
            head((8.0, 14.2), (0.0, 1.0), 2.6);
            head((1.8, 8.0), (-1.0, 0.0), 2.6);
            head((14.2, 8.0), (1.0, 0.0), 2.6);
        }
        Icon::Rotate => {
            let (a0, a1) = (0.35 * PI, 1.85 * PI);
            arc(8.0, 8.0, 5.6, 5.6, a0, a1);
            let tip = (8.0 + 5.6 * a1.cos(), 8.0 + 5.6 * a1.sin());
            head(tip, (-a1.sin(), a1.cos()), 3.0);
        }
        Icon::Scale => {
            rounded(2.0, 8.5, 7.5, 14.0, 1.0);
            line((7.5, 8.5), (13.6, 2.4));
            poly(&[(9.2, 2.4), (13.6, 2.4), (13.6, 6.8)]);
        }
        Icon::World => {
            circle(8.0, 8.0, 6.3);
            line((1.7, 8.0), (14.3, 8.0));
            arc(8.0, 8.0, 2.7, 6.3, 0.0, TAU);
        }
        Icon::Local => {
            closed(&[(2.0, 5.5), (10.5, 5.5), (10.5, 14.0), (2.0, 14.0)]);
            poly(&[(2.0, 5.5), (5.5, 2.0), (14.0, 2.0), (14.0, 10.5), (10.5, 14.0)]);
            line((10.5, 5.5), (14.0, 2.0));
        }
        Icon::Magnet => {
            arc(8.0, 8.6, 4.6, 4.6, 0.0, PI);
            line((3.4, 8.6), (3.4, 4.6));
            line((12.6, 8.6), (12.6, 4.6));
            rounded_fill(2.0, 1.8, 4.8, 4.6, 0.6);
            rounded_fill(11.2, 1.8, 14.0, 4.6, 0.6);
        }
        Icon::FrameAll | Icon::FrameSelected => {
            poly(&[(2.0, 6.0), (2.0, 2.0), (6.0, 2.0)]);
            poly(&[(10.0, 2.0), (14.0, 2.0), (14.0, 6.0)]);
            poly(&[(14.0, 10.0), (14.0, 14.0), (10.0, 14.0)]);
            poly(&[(6.0, 14.0), (2.0, 14.0), (2.0, 10.0)]);
            if icon == Icon::FrameAll {
                circle(8.0, 8.0, 2.8);
            } else {
                disc(8.0, 8.0, 2.6, color);
            }
        }
        Icon::Pause => {
            rounded_fill(4.0, 2.8, 6.6, 13.2, 0.8);
            rounded_fill(9.4, 2.8, 12.0, 13.2, 0.8);
        }
        Icon::Play => {
            fill(&[(4.6, 2.6), (13.4, 8.0), (4.6, 13.4)], color);
        }
        Icon::Scene => {
            disc(3.0, 3.6, 1.5, color);
            line((6.2, 3.6), (14.2, 3.6));
            line((3.0, 5.6), (3.0, 12.6));
            line((3.0, 8.4), (5.0, 8.4));
            line((3.0, 12.6), (5.0, 12.6));
            disc(6.8, 8.4, 1.3, color);
            line((9.4, 8.4), (14.2, 8.4));
            disc(6.8, 12.6, 1.3, color);
            line((9.4, 12.6), (14.2, 12.6));
        }
        Icon::Material => {
            fill(&arc_points(8.0, 8.0, 6.3, 6.3, -0.25 * PI, 0.75 * PI), soft);
            circle(8.0, 8.0, 6.3);
            line((3.55, 12.45), (12.45, 3.55));
        }
        Icon::Environment => {
            arc(8.0, 12.0, 6.2, 6.2, PI, TAU);
            line((1.0, 12.0), (15.0, 12.0));
            disc(8.0, 8.4, 1.7, color);
            line((4.0, 14.6), (12.0, 14.6));
        }
        Icon::Light => {
            arc(8.0, 6.4, 4.6, 4.6, 0.62 * PI, 2.38 * PI);
            line((6.1, 10.6), (6.1, 12.2));
            line((9.9, 10.6), (9.9, 12.2));
            line((6.1, 12.2), (9.9, 12.2));
            line((6.8, 14.2), (9.2, 14.2));
        }
        Icon::Camera => {
            rounded(1.4, 4.6, 14.6, 13.4, 2.0);
            poly(&[(5.0, 4.6), (6.2, 2.6), (9.8, 2.6), (11.0, 4.6)]);
            circle(8.0, 9.0, 2.7);
        }
        Icon::Image => {
            for (y, knob) in [(3.6, 5.0), (8.0, 10.6), (12.4, 7.0)] {
                line((1.8, y), (14.2, y));
                disc(knob, y, 1.9, color);
            }
        }
        Icon::Studios => {
            rounded(1.8, 6.4, 10.8, 14.2, 1.4);
            poly(&[(4.0, 6.4), (4.0, 4.2), (13.0, 4.2), (13.0, 12.0), (10.8, 12.0)]);
            poly(&[(6.2, 4.2), (6.2, 2.0), (15.2, 2.0), (15.2, 9.8), (13.0, 9.8)]);
        }
        Icon::Script => {
            poly(&[(5.0, 3.8), (1.6, 8.0), (5.0, 12.2)]);
            poly(&[(11.0, 3.8), (14.4, 8.0), (11.0, 12.2)]);
            line((9.3, 2.8), (6.7, 13.2));
        }
        Icon::Library => {
            disc(4.4, 4.4, 2.9, color);
            circle(11.6, 4.4, 2.8);
            circle(4.4, 11.6, 2.8);
            circle(11.6, 11.6, 2.8);
        }
        Icon::Console => {
            rounded(1.4, 2.4, 14.6, 13.6, 2.0);
            poly(&[(4.4, 5.8), (6.8, 8.0), (4.4, 10.2)]);
            line((8.6, 10.4), (11.8, 10.4));
        }
        Icon::Assistant => {
            let star = |x: f32, y: f32, r: f32, waist: f32| {
                fill(&[(x, y - r), (x + waist, y), (x, y + r), (x - waist, y)], color);
                fill(&[(x - r, y), (x, y - waist), (x + r, y), (x, y + waist)], color);
            };
            star(6.8, 9.2, 5.6, 1.5);
            star(12.6, 3.6, 2.6, 0.8);
        }
        Icon::Import => {
            line((8.0, 1.8), (8.0, 10.0));
            poly(&[(4.6, 6.8), (8.0, 10.2), (11.4, 6.8)]);
            poly(&[(2.0, 10.2), (2.0, 14.0), (14.0, 14.0), (14.0, 10.2)]);
        }
        Icon::Save => {
            closed(&[(2.4, 2.4), (11.2, 2.4), (13.6, 4.8), (13.6, 13.6), (2.4, 13.6)]);
            poly(&[(5.0, 2.4), (5.0, 5.8), (10.4, 5.8), (10.4, 2.4)]);
            circle(8.0, 10.0, 1.9);
        }
        Icon::Eye => {
            let mut outline = arc_points(8.0, 12.6, 7.6, 8.2, 1.19 * PI, 1.81 * PI);
            outline.extend(arc_points(8.0, 3.4, 7.6, 8.2, 0.19 * PI, 0.81 * PI));
            closed(&outline);
            disc(8.0, 8.0, 2.2, color);
        }
        Icon::EyeClosed => {
            arc(8.0, 3.4, 7.6, 8.2, 0.19 * PI, 0.81 * PI);
            line((4.4, 10.0), (3.4, 12.0));
            line((8.0, 11.0), (8.0, 13.4));
            line((11.6, 10.0), (12.6, 12.0));
        }
        Icon::Lock | Icon::Unlock => {
            rounded(3.2, 7.4, 12.8, 14.2, 1.4);
            if icon == Icon::Lock {
                arc(8.0, 5.4, 3.0, 3.0, PI, TAU);
                line((5.0, 5.4), (5.0, 7.4));
                line((11.0, 5.4), (11.0, 7.4));
            } else {
                arc(8.0, 4.4, 3.0, 3.0, PI, 1.95 * PI);
                line((5.0, 4.4), (5.0, 7.4));
            }
            line((8.0, 9.8), (8.0, 11.8));
        }
        Icon::ChevronRight => poly(&[(6.0, 3.6), (10.4, 8.0), (6.0, 12.4)]),
        Icon::ChevronDown => poly(&[(3.6, 6.0), (8.0, 10.4), (12.4, 6.0)]),
        Icon::Group => {
            closed(&[(1.6, 3.6), (6.0, 3.6), (7.6, 5.4), (14.4, 5.4), (14.4, 13.0), (1.6, 13.0)]);
        }
        Icon::Part => {
            closed(&[(8.0, 1.8), (14.0, 5.0), (14.0, 11.0), (8.0, 14.2), (2.0, 11.0), (2.0, 5.0)]);
            poly(&[(2.0, 5.0), (8.0, 8.2), (14.0, 5.0)]);
            line((8.0, 8.2), (8.0, 14.2));
        }
        Icon::Cutaway => {
            // A block with a corner sliced off, and the slice set aside.
            closed(&[(2.0, 2.4), (8.4, 2.4), (13.6, 7.6), (13.6, 14.0), (2.0, 14.0)]);
            line((10.4, 1.4), (14.6, 5.6));
        }
        Icon::Search => {
            circle(6.8, 6.8, 4.6);
            line((10.2, 10.2), (14.2, 14.2));
        }
        Icon::Help => {
            circle(8.0, 8.0, 6.6);
            arc(8.0, 6.2, 2.2, 2.2, PI, 2.3 * PI);
            line((8.0 + 2.2 * (2.3 * PI).cos(), 6.2 + 2.2 * (2.3 * PI).sin()), (8.0, 9.6));
            disc(8.0, 11.9, 0.9, color);
        }
        Icon::Close => {
            line((3.6, 3.6), (12.4, 12.4));
            line((12.4, 3.6), (3.6, 12.4));
        }
        Icon::More => {
            for x in [3.2, 8.0, 12.8] {
                disc(x, 8.0, 1.4, color);
            }
        }
        Icon::Sun => {
            circle(8.0, 8.0, 3.0);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                line((8.0 + 5.0 * a.cos(), 8.0 + 5.0 * a.sin()), (8.0 + 6.8 * a.cos(), 8.0 + 6.8 * a.sin()));
            }
        }
        Icon::Moon => {
            // A crescent: the outer arc, and the inner one it is bitten by.
            let mut outline = arc_points(8.0, 8.0, 6.2, 6.2, 0.036 * PI, 1.516 * PI);
            outline.extend(arc_points(10.6, 5.8, 4.6, 4.6, 1.334 * PI, 0.218 * PI));
            closed(&outline);
        }
        Icon::Palette => {
            let mut outline = arc_points(8.0, 8.0, 6.4, 6.4, 0.18 * PI, 1.95 * PI);
            outline.push((11.2, 10.4));
            outline.push((10.2, 12.0));
            closed(&outline);
            disc(5.0, 6.0, 1.2, color);
            disc(8.0, 4.4, 1.2, color);
            disc(11.0, 6.0, 1.2, color);
            disc(5.2, 9.6, 1.2, color);
        }
        Icon::Check => poly(&[(3.2, 8.4), (6.6, 11.6), (12.8, 4.6)]),
        Icon::Gauge => {
            arc(8.0, 9.6, 6.0, 6.0, PI, TAU);
            line((8.0, 9.6), (11.0, 6.2));
            disc(8.0, 9.6, 1.1, color);
        }
        Icon::Package => {
            rounded(2.6, 4.0, 13.4, 13.4, 1.5);
            line((2.6, 7.2), (13.4, 7.2));
            line((8.0, 4.0), (8.0, 7.2));
        }
        Icon::House => {
            closed(&[(2.6, 7.4), (8.0, 2.8), (13.4, 7.4), (13.4, 13.4), (2.6, 13.4)]);
            poly(&[(6.4, 13.4), (6.4, 9.8), (9.6, 9.8), (9.6, 13.4)]);
        }
        Icon::Gem => {
            closed(&[(4.6, 3.0), (11.4, 3.0), (14.0, 6.6), (8.0, 13.6), (2.0, 6.6)]);
            line((2.0, 6.6), (14.0, 6.6));
            poly(&[(6.2, 3.0), (8.0, 13.6), (9.8, 3.0)]);
        }
        Icon::Sliders => {
            for (x, knob) in [(4.0, 10.0), (8.0, 5.6), (12.0, 9.0)] {
                line((x, 2.6), (x, 13.4));
                disc(x, knob, 1.6, color);
            }
        }
        Icon::History => {
            arc(8.0, 8.0, 5.8, 5.8, 0.85 * PI, 2.8 * PI);
            poly(&[(8.0, 5.0), (8.0, 8.0), (10.2, 9.4)]);
            head((2.6, 6.2), (-0.4, 1.0), 2.0);
        }
        Icon::SearchX => {
            circle(7.0, 7.0, 4.6);
            line((10.4, 10.4), (13.8, 13.8));
            line((5.4, 5.4), (8.6, 8.6));
            line((8.6, 5.4), (5.4, 8.6));
        }
        Icon::Star => {
            let points: Vec<(f32, f32)> = (0..10)
                .map(|k| {
                    let a = -PI / 2.0 + k as f32 * PI / 5.0;
                    let r = if k % 2 == 0 { 6.2 } else { 2.7 };
                    (8.0 + r * a.cos(), 8.4 + r * a.sin())
                })
                .collect();
            closed(&points);
        }
        Icon::Clock => {
            circle(8.0, 8.0, 6.0);
            poly(&[(8.0, 4.6), (8.0, 8.0), (10.4, 9.4)]);
        }
        Icon::Texture => {
            rounded(2.0, 2.0, 14.0, 14.0, 2.0);
            rounded_fill(2.0, 2.0, 8.0, 8.0, 1.0);
            rounded_fill(8.0, 8.0, 14.0, 14.0, 1.0);
        }
        Icon::Plus => {
            line((8.0, 3.0), (8.0, 13.0));
            line((3.0, 8.0), (13.0, 8.0));
        }
        Icon::FolderOpen => {
            poly(&[(2.0, 12.5), (2.0, 3.5), (6.0, 3.5), (7.5, 5.0), (12.5, 5.0), (12.5, 7.0)]);
            closed(&[(2.0, 12.5), (4.0, 7.0), (14.5, 7.0), (12.5, 12.5)]);
        }
        Icon::Copy => {
            rounded(5.5, 5.5, 13.5, 13.5, 1.5);
            poly(&[(3.0, 10.5), (2.5, 10.5), (2.5, 2.5), (10.5, 2.5), (10.5, 3.0)]);
        }
        Icon::Pencil => {
            closed(&[(3.0, 13.0), (3.6, 10.4), (11.2, 2.8), (13.2, 4.8), (5.6, 12.4)]);
        }
        Icon::Trash => {
            line((2.5, 4.0), (13.5, 4.0));
            poly(&[(6.0, 4.0), (6.0, 2.5), (10.0, 2.5), (10.0, 4.0)]);
            poly(&[(4.0, 4.0), (4.8, 13.5), (11.2, 13.5), (12.0, 4.0)]);
        }
        Icon::Download => {
            line((8.0, 2.5), (8.0, 10.0));
            poly(&[(5.0, 7.2), (8.0, 10.2), (11.0, 7.2)]);
            poly(&[(2.5, 10.5), (2.5, 13.5), (13.5, 13.5), (13.5, 10.5)]);
        }
        Icon::Upload => {
            line((8.0, 3.0), (8.0, 10.5));
            poly(&[(5.0, 5.8), (8.0, 2.8), (11.0, 5.8)]);
            poly(&[(2.5, 10.5), (2.5, 13.5), (13.5, 13.5), (13.5, 10.5)]);
        }
        Icon::Puzzle => {
            closed(&[(3.0, 5.0), (6.0, 5.0), (6.0, 3.5), (8.0, 3.5), (8.0, 5.0), (11.0, 5.0), (11.0, 8.0), (12.5, 8.0), (12.5, 10.0), (11.0, 10.0), (11.0, 13.0), (3.0, 13.0)]);
        }
        Icon::Refresh => {
            arc(8.0, 8.0, 5.5, 5.5, -2.6, 2.2);
            poly(&[(12.6, 2.6), (12.6, 5.6), (9.6, 5.6)]);
        }
    }
}

/// The application's icon: the brand mark in the accent on a graphite tile,
/// rasterised here so it needs no asset and cannot drift from the mark the
/// top bar draws. 256 px, sampled 4 x 4 per pixel for its edges.
pub fn app_icon() -> egui::IconData {
    app_icon_sized(256, false)
}

/// The icon `size` pixels across. On `apple_grid` the tile is the size and
/// roundness of a Mac application's (824 of 1024, corners of 185), which
/// leaves room for the shadow the Dock and Finder expect; otherwise it fills
/// the image, as a Windows icon does.
pub fn app_icon_sized(size: usize, apple_grid: bool) -> egui::IconData {
    let scale = size as f32 / 256.0;
    let (inset, radius, mark) = if apple_grid { (100.0 / 4.0, 185.0 / 4.0, 824.0 / 4.0 / 236.0) } else { (10.0, 54.0, 1.0) };
    let unit = 9.6_f32 * scale * mark; // pixels per unit of the 16-unit icon grid
    let centre = size as f32 * 0.5;
    let half_stroke = 0.8 * unit;
    let rim = 6.6;
    // The same six blades as `Icon::Aperture`, in pixels.
    let corner = |k: i32| {
        let a = PI / 6.0 + k as f32 * PI / 3.0;
        (2.7 * a.cos(), 2.7 * a.sin())
    };
    let blades: Vec<((f32, f32), (f32, f32))> = (0..6)
        .map(|k| {
            let ((px, py), (qx, qy)) = (corner(k), corner(k - 1));
            let length = ((px - qx).powi(2) + (py - qy).powi(2)).sqrt();
            let (dx, dy) = ((px - qx) / length, (py - qy) / length);
            let along = px * dx + py * dy;
            let t = -along + (along * along - (px * px + py * py - rim * rim)).sqrt();
            (
                (centre + px * unit, centre + py * unit),
                (centre + (px + dx * t) * unit, centre + (py + dy * t) * unit),
            )
        })
        .collect();
    let to_segment = |x: f32, y: f32, (a, b): ((f32, f32), (f32, f32))| {
        let (vx, vy) = (b.0 - a.0, b.1 - a.1);
        let t = (((x - a.0) * vx + (y - a.1) * vy) / (vx * vx + vy * vy)).clamp(0.0, 1.0);
        ((x - a.0 - vx * t).powi(2) + (y - a.1 - vy * t).powi(2)).sqrt()
    };
    // The tile: a rounded square, a little lighter at the top.
    let (inset, radius) = (inset * scale, radius * scale);
    let in_tile = |x: f32, y: f32| {
        let qx = ((x - centre).abs() - (centre - inset - radius)).max(0.0);
        let qy = ((y - centre).abs() - (centre - inset - radius)).max(0.0);
        (qx * qx + qy * qy).sqrt() <= radius
    };
    let accent = [theme::p().accent.r() as f32, theme::p().accent.g() as f32, theme::p().accent.b() as f32];
    let mut rgba = vec![0u8; size * size * 4];
    for py in 0..size {
        for px in 0..size {
            let (mut colour, mut covered) = ([0.0_f32; 3], 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let (x, y) = (px as f32 + (sx as f32 + 0.5) / 4.0, py as f32 + (sy as f32 + 0.5) / 4.0);
                    if !in_tile(x, y) {
                        continue;
                    }
                    covered += 1;
                    let ring = (((x - centre).powi(2) + (y - centre).powi(2)).sqrt() - rim * unit).abs() <= half_stroke;
                    let mark = ring || blades.iter().any(|&blade| to_segment(x, y, blade) <= half_stroke);
                    let sample = if mark {
                        accent
                    } else {
                        let g = 34.0 - 12.0 * (y / size as f32);
                        [g, g, g]
                    };
                    for c in 0..3 {
                        colour[c] += sample[c];
                    }
                }
            }
            let i = (py * size + px) * 4;
            if covered > 0 {
                for c in 0..3 {
                    rgba[i + c] = (colour[c] / covered as f32).round() as u8;
                }
                rgba[i + 3] = (covered as f32 / 16.0 * 255.0).round() as u8;
            }
        }
    }
    egui::IconData { rgba, width: size as u32, height: size as u32 }
}

/// How a drawn sphere is shaded.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Finish {
    Matte,
    Gloss,
}

/// A small lit sphere in `color`: lit from the upper left, darker towards
/// the lower right, with a highlight when glossy. A mesh of rings, so the
/// shading is smooth at any size, ringed with a hairline so its edge is as
/// soft as a stroke's.
fn sphere(painter: &egui::Painter, centre: Pos2, r: f32, color: Color32, finish: Finish) {
    let light = {
        let (x, y, z) = (-0.5f32, -0.62f32, 0.6f32);
        let l = (x * x + y * y + z * z).sqrt();
        (x / l, y / l, z / l)
    };
    let half = {
        let (x, y, z) = (light.0, light.1, light.2 + 1.0);
        let l = (x * x + y * y + z * z).sqrt();
        (x / l, y / l, z / l)
    };
    let shade = |x: f32, y: f32| -> Color32 {
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        let lambert = (x * light.0 + y * light.1 + z * light.2).max(0.0);
        let (ambient, diffuse) = match finish {
            Finish::Matte => (0.55, 0.5),
            Finish::Gloss => (0.38, 0.72),
        };
        let mut k = ambient + diffuse * lambert;
        let mut spec = 0.0;
        if finish == Finish::Gloss {
            spec = (x * half.0 + y * half.1 + z * half.2).max(0.0).powf(28.0) * 0.85;
            // A horizon in the lower half: the room reflected.
            k += 0.12 * (1.0 - ((y - 0.45) * 5.0).powi(2)).max(0.0);
        }
        let c = |v: u8| ((v as f32 * k.min(1.15)) + 255.0 * spec).clamp(0.0, 255.0) as u8;
        Color32::from_rgba_premultiplied(c(color.r()), c(color.g()), c(color.b()), color.a())
    };
    let (rings, around) = (10usize, 48usize);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, shade(0.0, 0.0));
    for k in 1..=rings {
        let t = k as f32 / rings as f32;
        for i in 0..around {
            let a = i as f32 / around as f32 * TAU;
            let (x, y) = (t * a.cos(), t * a.sin());
            mesh.colored_vertex(centre + egui::vec2(x, y) * r, shade(x, y));
        }
    }
    let at = |k: usize, i: usize| if k == 0 { 0 } else { (1 + (k - 1) * around + i % around) as u32 };
    for i in 0..around {
        mesh.add_triangle(0, at(1, i), at(1, i + 1));
    }
    for k in 1..rings {
        for i in 0..around {
            mesh.add_triangle(at(k, i), at(k + 1, i), at(k + 1, i + 1));
            mesh.add_triangle(at(k, i), at(k + 1, i + 1), at(k, i + 1));
        }
    }
    painter.add(Shape::mesh(mesh));
    painter.circle_stroke(centre, r - 0.25, Stroke::new(1.0_f32, shade(0.3, 0.55)));
}

/// How an icon button looks at rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Quiet until hovered — for tools around the picture.
    Quiet,
    /// Lit: the thing it switches is on, or the tab is the one showing.
    On,
}

/// A square button showing one icon, `size` points across.
///
/// `sense` is the caller's: over the viewport a button has to sense drags as
/// well, or a drag that starts on it is passed through to the camera.
pub fn button(ui: &mut egui::Ui, icon: Icon, on: bool, size: f32, sense: egui::Sense) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), sense);
    paint_button(ui, rect, &response, icon, if on { Tone::On } else { Tone::Quiet }, ui.is_enabled());
    response
}

/// Draw a button's face for `response` in `rect`.
pub fn paint_button(ui: &egui::Ui, rect: Rect, response: &egui::Response, icon: Icon, tone: Tone, enabled: bool) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let p = theme::p();
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    // Eased in and out over a tenth of a second, so a row of tools lights
    // up under a passing pointer rather than flickering.
    let ease = ui.ctx().animate_bool_with_time(response.id.with("ease"), hovered, 0.1);
    let painter = ui.painter();
    let background = match (tone, pressed) {
        (Tone::On, _) => Some(p.accent_fill),
        (_, true) => Some(p.pressed.gamma_multiply(0.85)),
        _ if ease > 0.0 => Some(p.hover.gamma_multiply(0.7 * ease)),
        _ => None,
    };
    if let Some(background) = background {
        painter.rect_filled(rect, 7.0, background);
    }
    if tone == Tone::On && hovered {
        painter.rect_stroke(rect, 7.0, Stroke::new(1.0_f32, p.accent.gamma_multiply(0.5)));
    }
    let colour = if !enabled {
        p.text_faint.gamma_multiply(0.6)
    } else if tone == Tone::On {
        p.accent_text
    } else {
        theme::mix(p.icon, p.icon_hover, ease)
    };
    let glyph = (rect.width().min(rect.height()) * 0.58).clamp(12.0, 20.0);
    paint(painter, Rect::from_center_size(rect.center(), egui::Vec2::splat(glyph)), icon, colour);
}

/// A lighter or darker cut of `c`, for a filled button under the pointer.
fn shade(c: Color32, lighter: bool, t: f32) -> Color32 {
    let target = if lighter { 255.0 } else { 0.0 };
    let l = |v: u8| (v as f32 + (target - v as f32) * t).round() as u8;
    Color32::from_rgb(l(c.r()), l(c.g()), l(c.b()))
}

/// A button with an icon and a word: the top bar's actions. `primary` is
/// the filled one — the Render button, and nothing else.
pub fn labelled(ui: &mut egui::Ui, icon: Icon, label: &str, primary: bool) -> egui::Response {
    let p = theme::p();
    let font = if primary { theme::strong(13.0) } else { egui::FontId::proportional(13.0) };
    let text = if primary { p.on_accent } else { p.text };
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, text);
    let size = egui::vec2(11.0 + 15.0 + 7.0 + galley.size().x + 13.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let pressed = response.is_pointer_button_down_on();
        let ease = ui.ctx().animate_bool_with_time(response.id.with("ease"), response.hovered(), 0.1);
        let fill = match (primary, pressed) {
            (true, true) => shade(p.accent, false, 0.12),
            (true, false) => theme::mix(p.accent, shade(p.accent, true, 0.12), ease),
            (false, true) => p.pressed,
            (false, false) => theme::mix(p.control, p.hover, ease),
        };
        let painter = ui.painter();
        if primary {
            // Lifted a little off the bar: the one button that is meant to
            // be pressed.
            painter.add(egui::epaint::Shadow { offset: egui::vec2(0.0, 2.0), blur: 8.0, spread: 0.0, color: p.accent.gamma_multiply(0.35) }.as_shape(rect, 8.0));
        }
        painter.rect_filled(rect, 8.0, fill);
        paint(painter, Rect::from_center_size(egui::pos2(rect.left() + 18.5, rect.center().y), egui::Vec2::splat(15.0)), icon, text);
        painter.galley(egui::pos2(rect.left() + 33.0, rect.center().y - galley.size().y * 0.5), galley, Color32::PLACEHOLDER);
    }
    response
}

/// A toggle for a panel: an icon and its name, lit while the panel shows.
/// With `named` false, the icon alone — for a window too narrow for words.
pub fn toggle(ui: &mut egui::Ui, icon: Icon, label: &str, on: bool, named: bool) -> egui::Response {
    let p = theme::p();
    let text = if on { p.accent_text } else { p.text_dim };
    let galley = named.then(|| ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), text));
    let width = match &galley {
        Some(g) => 10.0 + 16.0 + 6.0 + g.size().x + 11.0,
        None => 30.0,
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let ease = ui.ctx().animate_bool_with_time(response.id.with("ease"), response.hovered(), 0.1);
        let painter = ui.painter();
        if on {
            painter.rect_filled(rect, 8.0, p.accent_fill);
        } else if ease > 0.0 {
            painter.rect_filled(rect, 8.0, p.hover.gamma_multiply(0.7 * ease));
        }
        let colour = if on { p.accent_text } else { theme::mix(p.icon, p.icon_hover, ease) };
        let at = if galley.is_some() { egui::pos2(rect.left() + 18.0, rect.center().y) } else { rect.center() };
        paint(painter, Rect::from_center_size(at, egui::Vec2::splat(16.0)), icon, colour);
        if let Some(galley) = galley {
            let colour = if on { p.accent_text } else { theme::mix(p.text_dim, p.text, ease) };
            painter.galley(egui::pos2(rect.left() + 32.0, rect.center().y - galley.size().y * 0.5), galley, colour);
        }
    }
    response
}

/// `response` with a tooltip in three parts: what it is, in bold; a line
/// saying more; and the keys that do the same, as key caps. `detail` and
/// `keys` may be empty. Shown whether or not the control is enabled — a
/// greyed-out control is the one most in need of saying why.
pub fn tip(response: egui::Response, title: &str, detail: &str, keys: &str) -> egui::Response {
    response
        .on_hover_ui(|ui| theme::tip_body(ui, title, detail, keys))
        .on_disabled_hover_ui(|ui| theme::tip_body(ui, title, detail, keys))
}

/// A small amber pill for something in progress elsewhere, with a dot.
pub fn busy_pill(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let p = theme::p();
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), p.busy);
    let size = egui::vec2(22.0 + galley.size().x + 10.0, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 12.0, p.busy.gamma_multiply(if response.hovered() { 0.26 } else { 0.16 }));
    painter.circle_filled(egui::pos2(rect.left() + 12.0, rect.center().y), 3.5, p.busy);
    painter.galley(egui::pos2(rect.left() + 21.0, rect.center().y - galley.size().y * 0.5), galley, Color32::PLACEHOLDER);
    response
}

/// A hairline between groups in a horizontal row of controls.
pub fn vertical_divider(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, height), egui::Sense::hover());
    ui.painter().line_segment([rect.center_top(), rect.center_bottom()], Stroke::new(1.0_f32, theme::p().line));
}

/// A floating plate the viewport's tools sit on: translucent enough to keep
/// the picture behind it in mind, lifted by a soft shadow — graphite in
/// dark mode, frosted white in light.
pub fn island(painter: &egui::Painter, rect: Rect) {
    let p = theme::p();
    painter.add(
        egui::epaint::Shadow { offset: egui::vec2(0.0, 4.0), blur: 16.0, spread: 0.0, color: Color32::from_black_alpha(if p.dark { 80 } else { 34 }) }
            .as_shape(rect, 11.0),
    );
    painter.rect(rect, 11.0, p.island, Stroke::new(1.0_f32, p.island_line));
}

/// A thin divider inside an island or a toolbar, across its short side.
pub fn divider(painter: &egui::Painter, a: Pos2, b: Pos2) {
    painter.line_segment([a, b], Stroke::new(1.0_f32, theme::p().line_strong.gamma_multiply(0.8)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The application icon is the mark on a tile: the accent on the ring,
    /// graphite inside it, and nothing outside the tile's rounded corners.
    #[test]
    fn the_app_icon_is_the_mark_on_a_tile() {
        let icon = app_icon();
        let at = |x: usize, y: usize| {
            let i = (y * icon.width as usize + x) * 4;
            [icon.rgba[i], icon.rgba[i + 1], icon.rgba[i + 2], icon.rgba[i + 3]]
        };
        assert_eq!(at(1, 1)[3], 0, "the corner is clear");
        let ring = at(128, 128 - (6.6 * 9.6) as usize);
        assert_eq!(&ring[..3], &[theme::p().accent.r(), theme::p().accent.g(), theme::p().accent.b()], "the ring is the accent");
        let inside = at(128, 128);
        assert!(inside[3] == 255 && inside[0] < 40 && inside[0] == inside[2], "graphite in the middle: {inside:?}");
    }

    /// The application icon as a Windows `.ico`, for the installer, its
    /// shortcuts and `.tequila` files: the same 256 px mark, and smaller
    /// copies averaged down from it for the sizes Windows asks for. Writes
    /// `scripts/installer/tequila.ico`, or `TEQUILA_ICON_OUT`; the file is
    /// kept in the repository, so run this again only when the mark changes.
    #[test]
    #[ignore]
    fn write_the_windows_icon() {
        let icon = app_icon();
        let size = icon.width as usize;
        let mut images = Vec::new();
        for target in [16usize, 24, 32, 48, 64, 128, 256] {
            let rgba = if target == size { icon.rgba.clone() } else { shrink(&icon.rgba, size, target) };
            let mut png_bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut png_bytes, target as u32, target as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
            }
            images.push((target, png_bytes));
        }
        // ICONDIR, then an ICONDIRENTRY per image, then the images: PNG
        // entries, which every Windows since Vista reads at any size.
        let mut ico = vec![0u8, 0, 1, 0];
        ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
        let mut offset = 6 + 16 * images.len();
        for (target, data) in &images {
            let side = if *target >= 256 { 0 } else { *target as u8 };
            ico.extend_from_slice(&[side, side, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
            ico.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += data.len();
        }
        for (_, data) in &images {
            ico.extend_from_slice(data);
        }
        let out = std::env::var_os("TEQUILA_ICON_OUT").map(std::path::PathBuf::from).unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/installer/tequila.ico")
        });
        std::fs::write(&out, &ico).unwrap();
        println!("wrote {} ({} bytes)", out.display(), ico.len());
    }

    /// `rgba`, `from` pixels square, averaged down to `to`: each pixel the
    /// area-weighted mean of what it covers, colour weighted by alpha so a
    /// clear corner does not darken the tile's edge.
    fn shrink(rgba: &[u8], from: usize, to: usize) -> Vec<u8> {
        let scale = from as f32 / to as f32;
        let mut out = vec![0u8; to * to * 4];
        for y in 0..to {
            for x in 0..to {
                let (x0, x1, y0, y1) = (x as f32 * scale, (x + 1) as f32 * scale, y as f32 * scale, (y + 1) as f32 * scale);
                let (mut sum, mut weight) = ([0.0f32; 4], 0.0f32);
                for sy in y0.floor() as usize..(y1.ceil() as usize).min(from) {
                    let wy = (y1.min(sy as f32 + 1.0) - y0.max(sy as f32)).max(0.0);
                    for sx in x0.floor() as usize..(x1.ceil() as usize).min(from) {
                        let w = wy * (x1.min(sx as f32 + 1.0) - x0.max(sx as f32)).max(0.0);
                        let i = (sy * from + sx) * 4;
                        let a = rgba[i + 3] as f32 / 255.0;
                        for c in 0..3 {
                            sum[c] += rgba[i + c] as f32 * a * w;
                        }
                        sum[3] += a * w;
                        weight += w;
                    }
                }
                let i = (y * to + x) * 4;
                if sum[3] > 0.0 {
                    for c in 0..3 {
                        out[i + c] = (sum[c] / sum[3]).round().clamp(0.0, 255.0) as u8;
                    }
                }
                out[i + 3] = (sum[3] / weight * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        out
    }

    /// Every icon, as the interface draws it, on dark and on light, large
    /// and at the sizes it is used: written to a PNG for looking at.
    /// `TEQUILA_ICON_SHEET` is the path; `library_sheet.png` style.
    #[test]
    #[ignore = "writes a picture to look at"]
    fn icon_sheet() {
        let out = std::env::var("TEQUILA_ICON_SHEET").unwrap_or_else(|_| "icon_sheet.png".into());
        let all = every_icon();
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let columns = 8usize;
        let (cell, small) = (96.0f32, 36.0f32);
        let rows = all.len().div_ceil(columns);
        let width = columns as f32 * cell;
        let height = rows as f32 * (cell + small) * 2.0;
        let screen = Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height));
        let input = || egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut atlas: Option<egui::ColorImage> = None;
        let take = |output: &egui::FullOutput, atlas: &mut Option<egui::ColorImage>| {
            for (_, delta) in &output.textures_delta.set {
                let image = match &delta.image {
                    egui::ImageData::Color(c) => (**c).clone(),
                    egui::ImageData::Font(f) => egui::ColorImage { size: f.size, pixels: f.srgba_pixels(None).collect() },
                };
                match (delta.pos, atlas.as_mut()) {
                    (Some([x0, y0]), Some(full)) => {
                        for y in 0..image.size[1] {
                            for x in 0..image.size[0] {
                                full.pixels[(y0 + y) * full.size[0] + x0 + x] = image.pixels[y * image.size[0] + x];
                            }
                        }
                    }
                    _ => *atlas = Some(image),
                }
            }
        };
        let first = ctx.run(input(), |_| {});
        take(&first, &mut atlas);
        let output = ctx.run(input(), |ctx| {
            let painter = egui::Painter::new(ctx.clone(), egui::LayerId::new(egui::Order::Foreground, egui::Id::new("sheet")), screen);
            for (half, (ground, ink)) in [(Color32::from_gray(30), Color32::from_gray(232)), (Color32::from_gray(246), Color32::from_gray(30))].into_iter().enumerate() {
                let top = half as f32 * height * 0.5;
                painter.rect_filled(Rect::from_min_size(egui::pos2(0.0, top), egui::vec2(width, height * 0.5)), 0.0, ground);
                for (i, icon) in all.iter().enumerate() {
                    let (c, r) = (i % columns, i / columns);
                    let origin = egui::pos2(c as f32 * cell, top + r as f32 * (cell + small));
                    painter.rect_stroke(Rect::from_min_size(origin + egui::vec2(16.0, 12.0), egui::Vec2::splat(64.0)), 0.0, Stroke::new(1.0_f32, ink.gamma_multiply(0.12)));
                    paint(&painter, Rect::from_min_size(origin + egui::vec2(16.0, 12.0), egui::Vec2::splat(64.0)), *icon, ink);
                    for (k, size) in [14.0f32, 18.0, 24.0].into_iter().enumerate() {
                        let at = origin + egui::vec2(14.0 + k as f32 * 26.0, cell);
                        paint(&painter, Rect::from_min_size(at, egui::Vec2::splat(size)), *icon, ink);
                    }
                }
            }
        });
        take(&output, &mut atlas);
        let atlas = atlas.expect("the font atlas");
        let primitives = ctx.tessellate(output.shapes, 1.0);
        let (w, h) = (width as usize, height as usize);
        let mut pixels = vec![[0.0f32; 4]; w * h];
        for primitive in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else { continue };
            for tri in mesh.indices.chunks(3) {
                let v = [mesh.vertices[tri[0] as usize], mesh.vertices[tri[1] as usize], mesh.vertices[tri[2] as usize]];
                let (x0, x1) = (v.iter().map(|v| v.pos.x).fold(f32::MAX, f32::min).floor().max(0.0) as usize, v.iter().map(|v| v.pos.x).fold(f32::MIN, f32::max).ceil().min(w as f32 - 1.0) as usize);
                let (y0, y1) = (v.iter().map(|v| v.pos.y).fold(f32::MAX, f32::min).floor().max(0.0) as usize, v.iter().map(|v| v.pos.y).fold(f32::MIN, f32::max).ceil().min(h as f32 - 1.0) as usize);
                let area = (v[1].pos.x - v[0].pos.x) * (v[2].pos.y - v[0].pos.y) - (v[2].pos.x - v[0].pos.x) * (v[1].pos.y - v[0].pos.y);
                if area.abs() < 1e-9 {
                    continue;
                }
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let edge = |a: egui::epaint::Vertex, b: egui::epaint::Vertex| (b.pos.x - a.pos.x) * (py - a.pos.y) - (px - a.pos.x) * (b.pos.y - a.pos.y);
                        let (l0, l1, l2) = (edge(v[1], v[2]) / area, edge(v[2], v[0]) / area, edge(v[0], v[1]) / area);
                        if l0 < 0.0 || l1 < 0.0 || l2 < 0.0 {
                            continue;
                        }
                        let lerp = |f: &dyn Fn(&egui::epaint::Vertex) -> f32| l0 * f(&v[0]) + l1 * f(&v[1]) + l2 * f(&v[2]);
                        let (u, vv) = (lerp(&|v| v.uv.x), lerp(&|v| v.uv.y));
                        let tx = ((u * atlas.size[0] as f32) as usize).min(atlas.size[0] - 1);
                        let ty = ((vv * atlas.size[1] as f32) as usize).min(atlas.size[1] - 1);
                        let texel = atlas.pixels[ty * atlas.size[0] + tx];
                        let c = |k: usize| lerp(&|v| v.color.to_array()[k] as f32 / 255.0);
                        let tex = |k: usize| texel.to_array()[k] as f32 / 255.0;
                        let src = [c(0) * tex(0), c(1) * tex(1), c(2) * tex(2), c(3) * tex(3)];
                        let dst = &mut pixels[y * w + x];
                        for k in 0..4 {
                            dst[k] = src[k] + dst[k] * (1.0 - src[3]);
                        }
                    }
                }
            }
        }
        let file = std::fs::File::create(&out).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| [p[0], p[1], p[2]].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)).collect();
        encoder.write_header().unwrap().write_image_data(&bytes).unwrap();
        println!("wrote {out}");
    }

    fn every_icon() -> Vec<Icon> {
        vec![
            Icon::Aperture, Icon::Wireframe, Icon::Solid, Icon::MaterialPreview, Icon::Rendered, Icon::Move,
            Icon::Rotate, Icon::Scale, Icon::World, Icon::Local, Icon::Magnet, Icon::FrameAll, Icon::FrameSelected,
            Icon::Pause, Icon::Play, Icon::Scene, Icon::Material, Icon::Environment, Icon::Light, Icon::Camera,
            Icon::Image, Icon::Studios, Icon::Script, Icon::Library, Icon::Console, Icon::Assistant, Icon::Import,
            Icon::Save, Icon::Eye, Icon::EyeClosed, Icon::Lock, Icon::Unlock, Icon::ChevronRight,
            Icon::ChevronDown, Icon::Group, Icon::Part, Icon::Cutaway, Icon::Search, Icon::Help, Icon::Close,
            Icon::More, Icon::Texture, Icon::Sun, Icon::Moon, Icon::Palette, Icon::Check, Icon::Gauge,
            Icon::Package, Icon::House, Icon::Gem, Icon::Sliders, Icon::History, Icon::SearchX, Icon::Star,
            Icon::Clock, Icon::Plus, Icon::FolderOpen, Icon::Copy, Icon::Pencil, Icon::Trash, Icon::Download,
            Icon::Upload, Icon::Puzzle, Icon::Refresh,
        ]
    }

    /// Every icon drawn from Lucide is in the font as it ships: a codepoint
    /// that is not would draw as nothing, or as an empty box.
    #[test]
    fn every_lucide_icon_is_in_the_font() {
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        let _ = ctx.run(Default::default(), |_| {});
        let font = egui::FontId::new(16.0, egui::FontFamily::Name(theme::ICONS.into()));
        for icon in every_icon() {
            if let Some(glyph) = icon.glyph() {
                assert!(ctx.fonts(|f| f.has_glyph(&font, glyph)), "{icon:?}'s glyph U+{:04X} is not in the font", glyph as u32);
            }
        }
    }

    /// Every icon draws something, inside the square it was given.
    ///
    /// An icon is a dozen hand-placed coordinates; one typed outside the
    /// grid paints over its neighbour on a rail, and one whose shapes all
    /// degenerate draws a blank button that looks like a missing feature.
    #[test]
    fn every_icon_draws_inside_its_square() {
        let ctx = egui::Context::default();
        let all = every_icon();
        let square = Rect::from_min_size(egui::pos2(100.0, 100.0), egui::Vec2::splat(32.0));
        for icon in all {
            let output = ctx.run(Default::default(), |ctx| {
                let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("probe"));
                paint(&egui::Painter::new(ctx.clone(), layer, Rect::EVERYTHING), square, icon, Color32::WHITE);
            });
            let bounds: Vec<Rect> = output.shapes.iter().map(|c| c.shape.visual_bounding_rect()).collect();
            assert!(!bounds.is_empty(), "{icon:?} drew nothing");
            for b in bounds {
                assert!(b.width() > 0.0 || b.height() > 0.0, "{icon:?} drew a degenerate shape");
                assert!(square.expand(2.5).contains_rect(b), "{icon:?} spills out of its square: {b:?}");
            }
        }
    }
}
