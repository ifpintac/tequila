//! What the canvas shows besides the layout: a texture of the material
//! behind it, made lighter or darker so the layout's lines read over dark
//! or busy art; and the padding round each island - the band a texture of a
//! given size bleeds into round it - with the islands whose bands meet
//! marked, as too close for that texture.
//!
//! The padding is the space islands need between them: each island's band
//! is half of it, so two bands meet where two islands are closer than the
//! padding - as Pack's margin is the gap between islands, half of it at the
//! square's edges, and Use for Pack sets one to the other.

use super::layout::Layout;
use super::view::Xform;
use super::{Part, UvEditor};
use crate::project::Project;
use std::collections::HashMap;
use std::sync::Arc;
use tequila_core::math::Vec2;
use tequila_materials::graph::TextureId;
use tequila_materials::material_type::Param;
use tequila_scene::uv::{EdgeKind, UvLayout};
use tequila_scene::MeshKey;

/// The padding a texture `size` pixels a side wants between its islands:
/// 4 pixels at 1K, 8 at 2K, 16 at 4K, in proportion to the size.
pub fn suggested_padding(size: u32) -> f32 {
    (size as f32 / 256.0).max(1.0)
}

#[derive(Default)]
pub struct Display {
    /// The material's texture shown behind the layout in every mode, by
    /// the parameter it drives.
    pub behind: Option<Param>,
    /// The checker shown behind the layout instead, as the part wears it
    /// through the set shown.
    pub checker_behind: bool,
    /// The checker's picture, for the canvas, and what it was drawn for.
    checker_picture: Option<(u64, egui::TextureHandle)>,
    /// Pictures as read, to be lightened or darkened.
    sources: HashMap<TextureId, Option<Arc<egui::ColorImage>>>,
    /// Pictures lightened by some stops, by the stops in quarters.
    exposed: HashMap<TextureId, (i32, egui::TextureHandle)>,
    /// The islands too close for the padding, for a mesh, set and padding.
    close: Option<((MeshKey, usize, u32), Arc<Vec<bool>>)>,
}

impl Display {
    /// How many islands were last found too close, where they were.
    pub fn too_close_known(&self) -> Option<usize> {
        self.close.as_ref().map(|(_, close)| close.iter().filter(|&&c| c).count())
    }
}

/// The islands closer to another than `gap` on the layout, or overlapping
/// one: their border edges, by a grid of cells as wide as the gap, each
/// checked against the other islands' in the cells round it; and an island
/// with a corner inside another.
pub fn too_close(layout: &UvLayout, gap: f32) -> Vec<bool> {
    let mut out = vec![false; layout.islands.len()];
    if gap <= 0.0 || layout.islands.len() < 2 {
        return out;
    }
    let segments: Vec<(u32, Vec2, Vec2)> = layout
        .edges
        .iter()
        .filter(|e| e.kind != EdgeKind::Inner)
        .map(|e| (e.island, layout.uv_vertices[e.ends[0] as usize].uv, layout.uv_vertices[e.ends[1] as usize].uv))
        .collect();
    let cell = gap.max(1e-6);
    let key = |p: Vec2| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
    for (k, (_, a, b)) in segments.iter().enumerate() {
        let (lo, hi) = (key(a.min(*b)), key(a.max(*b)));
        // A long edge over very many cells is put in a few hundred at most.
        if (hi.0 - lo.0 + 1) * (hi.1 - lo.1 + 1) > 400 {
            for t in 0..=20 {
                let p = *a + (*b - *a) * (t as f32 / 20.0);
                grid.entry(key(p)).or_default().push(k as u32);
            }
            continue;
        }
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                grid.entry((x, y)).or_default().push(k as u32);
            }
        }
    }
    let mut checked: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    for (&(x, y), list) in &grid {
        let mut near: Vec<u32> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(l) = grid.get(&(x + dx, y + dy)) {
                    near.extend_from_slice(l);
                }
            }
        }
        for &s in list {
            let (island, a, b) = segments[s as usize];
            if out[island as usize] && near.iter().all(|&t| out[segments[t as usize].0 as usize]) {
                continue;
            }
            for &t in &near {
                let (other, c, d) = segments[t as usize];
                if other == island || !checked.insert((s.min(t), s.max(t))) {
                    continue;
                }
                if segment_distance(a, b, c, d) < gap {
                    out[island as usize] = true;
                    out[other as usize] = true;
                }
            }
        }
    }
    // One inside another, borders apart.
    for (i, island) in layout.islands.iter().enumerate() {
        let Some(&first) = island.boundary.first().and_then(|l| l.first()) else { continue };
        if let Some(hit) = layout.triangle_at(layout.uv_vertices[first as usize].uv) {
            if hit.island != i as u32 {
                out[i] = true;
                out[hit.island as usize] = true;
            }
        }
    }
    out
}

fn segment_distance(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> f32 {
    let cross = |o: Vec2, p: Vec2, q: Vec2| (p.x - o.x) * (q.y - o.y) - (p.y - o.y) * (q.x - o.x);
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return 0.0;
    }
    let to = |p: Vec2, a: Vec2, b: Vec2| {
        let ab = b - a;
        let l = ab.length_squared();
        let t = if l > 0.0 { ((p - a).dot(ab) / l).clamp(0.0, 1.0) } else { 0.0 };
        (a + ab * t - p).length()
    };
    to(a, c, d).min(to(b, c, d)).min(to(c, a, b)).min(to(d, a, b))
}

impl UvEditor {
    /// The padding in the layout's units.
    pub(super) fn padding_uv(&self) -> f32 {
        self.prefs.padding_px / self.prefs.padding_texture.max(1) as f32
    }

    /// The islands too close for the padding, worked out again when the
    /// layout or the padding is not the one they were for.
    pub(super) fn too_close_now(&mut self, part: &Part, layout: &Layout) -> Arc<Vec<bool>> {
        let key = (part.mesh, layout.uv.set, self.padding_uv().to_bits());
        if let Some((k, close)) = &self.display.close {
            if *k == key {
                return close.clone();
            }
        }
        let close = Arc::new(too_close(&layout.uv, self.padding_uv()));
        self.display.close = Some((key, close.clone()));
        close
    }

    /// Each island's band, drawn soft along its border with round
    /// corners, so a ragged edge does not throw spikes; the islands too
    /// close in the colour of a warning. In colours already laid on the
    /// layout's own, so where bands cross they do not darken.
    pub(super) fn draw_padding(&mut self, painter: &egui::Painter, xform: &Xform, part: &Part, layout: &Layout) {
        let p = crate::theme::p();
        let close = self.too_close_now(part, layout);
        let width = (self.padding_uv() * xform.scale).max(1.0);
        let (soft, warn) = (crate::theme::mix(p.canvas, p.accent, 0.22), crate::theme::mix(p.canvas, p.danger, 0.4));
        for (i, island) in layout.uv.islands.iter().enumerate() {
            let colour = if close.get(i).copied().unwrap_or(false) { warn } else { soft };
            let stroke = egui::Stroke::new(width, colour);
            for ring in &island.boundary {
                let points: Vec<egui::Pos2> = ring.iter().map(|&v| xform.to_screen(layout.uv.uv_vertices[v as usize].uv)).collect();
                for k in 0..points.len() {
                    let (a, b) = (points[k], points[(k + 1) % points.len()]);
                    painter.line_segment([a, b], stroke);
                    if width > 2.0 {
                        painter.circle_filled(a, width * 0.5, colour);
                    }
                }
            }
        }
    }

    /// How many islands are too close for the padding.
    pub(super) fn too_close_count(&mut self, part: &Part, layout: &Layout) -> usize {
        self.too_close_now(part, layout).iter().filter(|&&c| c).count()
    }

    /// The tint artwork is drawn with for the background's exposure where
    /// it darkens; where it lightens, the picture itself is lightened.
    pub(super) fn exposure_tint(&self) -> f32 {
        2f32.powf(self.prefs.background_exposure.min(0.0))
    }

    /// A picture as the background's exposure shows it: the picture as it
    /// is where the exposure darkens it or leaves it, a lightened copy
    /// where it lightens it.
    pub(super) fn exposed_artwork(&mut self, project: &Project, id: TextureId, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        let stops = self.prefs.background_exposure;
        if stops <= 0.0 {
            return super::artwork(&mut self.artwork, project, id, ctx);
        }
        let quarters = (stops * 4.0).round() as i32;
        if let Some((q, handle)) = self.display.exposed.get(&id) {
            if *q == quarters {
                return Some(handle.clone());
            }
        }
        let source = self.display.sources.entry(id).or_insert_with(|| source_image(project, id)).clone()?;
        let gain = 2f32.powf(quarters as f32 / 4.0);
        let pixels = source
            .pixels
            .iter()
            .map(|c| {
                // On the values as shown, not as light: what the eye needs
                // from it is the dark parts brought up to where lines read.
                let lift = |v: u8| (v as f32 * gain).min(255.0);
                egui::Color32::from_rgba_unmultiplied(lift(c.r()) as u8, lift(c.g()) as u8, lift(c.b()) as u8, c.a())
            })
            .collect();
        let image = egui::ColorImage { size: source.size, pixels };
        let handle = ctx.load_texture(format!("uv-editor-exposed-{}", id.0), image, egui::TextureOptions::LINEAR);
        self.display.exposed.insert(id, (quarters, handle.clone()));
        Some(handle)
    }

    /// View options' part for what the canvas shows besides the layout,
    /// and the checker on the part.
    pub(super) fn display_options(&mut self, ui: &mut egui::Ui, part: &Part, layout: Option<&Layout>) {
        let p = crate::theme::p();
        crate::theme::section(ui, "Behind the layout");
        let textures: Vec<(Param, String)> = part.textures.iter().map(|t| (t.param, format!("{} · {}", t.param.label(), t.name))).collect();
        let shown = match self.display.behind.and_then(|b| textures.iter().find(|(p, _)| *p == b)) {
            Some((_, name)) => name.clone(),
            None if self.display.checker_behind => "The checker".to_string(),
            None => "Nothing".to_string(),
        };
        egui::ComboBox::from_id_salt("uv-behind")
            .width(240.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui.selectable_label(self.display.behind.is_none() && !self.display.checker_behind, "Nothing").clicked() {
                    (self.display.behind, self.display.checker_behind) = (None, false);
                }
                if ui.selectable_label(self.display.behind.is_none() && self.display.checker_behind, "The checker").on_hover_text("The checker the part wears in the viewport, as it lies on this layout").clicked() {
                    (self.display.behind, self.display.checker_behind) = (None, true);
                }
                for (param, name) in &textures {
                    if ui.selectable_label(self.display.behind == Some(*param), name.as_str()).clicked() {
                        (self.display.behind, self.display.checker_behind) = (Some(*param), false);
                    }
                }
            })
            .response
            .on_hover_text("The checker, or one of the material's textures laid on by the layout, shown behind it in every mode");
        let mut stops = self.prefs.background_exposure;
        if ui
            .add(crate::field::Slider::new(&mut stops, -4.0..=4.0).text("Exposure").suffix(" stops").fixed_decimals(1))
            .on_hover_text("Lighten or darken the artwork behind the layout - the textures and the labels' artwork - so the layout's lines read over dark or busy art. Only how it is shown here")
            .changed()
        {
            self.prefs.background_exposure = stops;
        }
        crate::theme::section(ui, "Padding");
        ui.add(crate::field::Checkbox::new(&mut self.prefs.padding, "Show the padding round each island"))
            .on_hover_text("The band a texture bleeds into round each island: half the padding each side, so where two bands meet the islands are closer than the padding");
        let mut size = self.prefs.padding_texture;
        if crate::theme::segmented(ui, &mut size, &super::density::SIZES) {
            self.prefs.padding_texture = size;
        }
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.prefs.padding_px).range(0.0..=512.0).speed(0.1).max_decimals(1).suffix(" px")).on_hover_text("The space islands need between them, in pixels of the texture");
            if ui.button("Suggest").on_hover_text("4 pixels at 1K, 8 at 2K, 16 at 4K: in proportion to the size").clicked() {
                self.prefs.padding_px = suggested_padding(self.prefs.padding_texture);
            }
            if ui.button("Use for Pack").on_hover_text("Set Pack's margin to this padding, in pixels of this texture").clicked() {
                self.pack.margin = self.prefs.padding_px;
                self.pack.unit = super::actions::MarginUnit::Pixels;
                self.pack.texture = self.prefs.padding_texture;
                let (px, size) = (self.prefs.padding_px, self.prefs.padding_texture);
                self.say(&format!("Pack's margin is now {px:.1} px of a {} texture", super::density::size_name(size)), false);
            }
        });
        if let (true, Some(layout)) = (self.prefs.padding, layout) {
            let close = self.too_close_count(part, layout);
            let text = match close {
                0 => "No islands too close".to_string(),
                1 => "1 island too close".to_string(),
                n => format!("{n} islands too close"),
            };
            ui.label(egui::RichText::new(text).size(12.0).color(if close > 0 { p.danger } else { p.text_dim }));
        }
        crate::theme::section(ui, "2D cursor");
        ui.add(crate::field::Checkbox::new(&mut self.prefs.show_cursor, "Show the cursor")).on_hover_text("Shift and a right click put it down; the right-click menu moves it to the selection or the layout's middle");
        crate::theme::section(ui, "Checker on the part");
        ui.add(crate::field::Checkbox::new(&mut self.checker.on, "Show a checker on the part"))
            .on_hover_text("The part in the viewport drawn with a checker through a UV set, in place of its colour, to see how the layout lies on it. In Wireframe, Solid and Material Preview, on the graphics card or the processor; never in Rendered or a render, and nothing is saved or changed");
        ui.horizontal(|ui| {
            ui.add(egui::DragValue::new(&mut self.checker.squares).range(1..=64).prefix("Squares ")).on_hover_text("How many squares across the layout's square");
        });
        crate::theme::segmented(ui, &mut self.checker.pattern, &[(super::checker::Pattern::Plain, "Plain"), (super::checker::Pattern::Letters, "Letters"), (super::checker::Pattern::Numbers, "Numbers")]);
        // The UV set it is drawn through: the one shown here, which it
        // follows, or one of the part's.
        let set_name = |set: usize| format!("UV set {}", set + 1);
        let following = format!("The one shown here ({})", set_name(self.shown_set));
        let chosen = match self.checker.set {
            None => following.clone(),
            Some(set) => set_name(set.min(part.sets.saturating_sub(1))),
        };
        ui.add_enabled_ui(part.sets > 1, |ui| {
            egui::ComboBox::from_id_salt("uv-checker-set").width(240.0).selected_text(chosen).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.checker.set, None, following);
                for set in 0..part.sets {
                    ui.selectable_value(&mut self.checker.set, Some(set), set_name(set));
                }
            });
        })
        .response
        .on_hover_text("The UV set the checker is drawn through: the one the editor shows, following it as you pick another, or one of the part's")
        .on_disabled_hover_text("The part has one UV set");
    }

    /// The checker's picture for the canvas, sent again when its squares or
    /// pattern change.
    pub(super) fn checker_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        let key = self.checker.squares as u64 * 4 + self.checker.pattern as u64;
        if let Some((k, texture)) = &self.display.checker_picture {
            if *k == key {
                return texture.clone();
            }
        }
        let image = self.checker_image();
        let (side, bytes) = (image.0, &image.1);
        let pixels = bytes.chunks_exact(4).map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
        let texture = ctx.load_texture("uv-editor-checker", egui::ColorImage { size: [side, side], pixels }, egui::TextureOptions::LINEAR);
        self.display.checker_picture = Some((key, texture.clone()));
        texture
    }

    /// Let go of the pictures, as the document goes.
    pub(super) fn forget_display(&mut self) {
        self.display = Display::default();
    }
}

/// A picture read for lightening, at most a thousand pixels across, as the
/// editor draws artwork.
fn source_image(project: &Project, id: TextureId) -> Option<Arc<egui::ColorImage>> {
    let source = project.document().textures.get(id)?;
    let image = tequila_texture::image::Image::load_raw(&source.path).ok()?;
    let step = (image.width.max(image.height) as f32 / 1024.0).ceil().max(1.0) as usize;
    let (width, height) = (image.width.div_ceil(step), image.height.div_ceil(step));
    let mut pixels = Vec::with_capacity(width * height);
    let channel = |x: usize, y: usize, c: usize| image.data[(y * image.width + x) * image.channels + c.min(image.channels - 1)];
    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = ((x * step).min(image.width - 1), (y * step).min(image.height - 1));
            let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            let (r, g, b) = if image.channels >= 3 { (channel(sx, sy, 0), channel(sx, sy, 1), channel(sx, sy, 2)) } else { let l = channel(sx, sy, 0); (l, l, l) };
            let a = if image.channels == 4 || image.channels == 2 { channel(sx, sy, image.channels - 1) } else { 1.0 };
            pixels.push(egui::Color32::from_rgba_unmultiplied(byte(r), byte(g), byte(b), byte(a)));
        }
    }
    Some(Arc::new(egui::ColorImage { size: [width, height], pixels }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};

    /// Two squares of a part laid out `apart` from each other.
    fn two(apart: f32) -> UvLayout {
        let positions = vec![
            vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0),
            vec3(2.0, 0.0, 0.0), vec3(3.0, 0.0, 0.0), vec3(3.0, 1.0, 0.0), vec3(2.0, 1.0, 0.0),
        ];
        let square = |x: f32| [vec2(x, 0.1), vec2(x + 0.3, 0.1), vec2(x + 0.3, 0.4), vec2(x, 0.4)];
        let uv = square(0.1).into_iter().chain(square(0.4 + apart)).collect();
        let mesh = tequila_scene::Mesh::new(positions, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]).with_uvs(uv);
        UvLayout::new(&mesh, 0).unwrap()
    }

    #[test]
    fn islands_closer_than_the_padding_are_marked() {
        let gap = 8.0 / 2048.0;
        assert_eq!(too_close(&two(0.01), gap), vec![false, false], "far enough apart");
        assert_eq!(too_close(&two(0.002), gap), vec![true, true], "their bands meet");
        assert_eq!(too_close(&two(-0.05), gap), vec![true, true], "overlapping");
        assert_eq!(suggested_padding(1024), 4.0);
        assert_eq!(suggested_padding(2048), 8.0);
        assert_eq!(suggested_padding(4096), 16.0);
    }
}
