//! The sections of an inspector page, in an order of the person's own.
//!
//! Each page has a head - what the page is about, the scene's tree or the
//! environment's map - and under it its sections, each a card with a title.
//! A section is moved by dragging its title, or by its grip's menu (Move up,
//! Move down), and the order is kept between sessions in `sections.txt`
//! beside the other settings: it is how a person works, not part of what
//! they work on, so it is not saved with the project.
//!
//! A drag starts anywhere on a section's title and a click there still opens
//! and closes it: the drag area is laid under the title before the card is
//! drawn, and egui gives a click to the clickable thing on top and a drag to
//! the draggable thing beneath it. Laid over the title instead, it would
//! take the click as well.
//!
//! Each frame the application lends the kept order to the context
//! ([`install`]) and takes it back if a drop changed it ([`take_changed`]),
//! so a test's own context starts from the defaults and shares nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::icons::{self, Icon};
use crate::theme;

/// Every page with sections, and its sections in their default order.
pub const PAGES: &[(&str, &[&str])] = &[
    ("scene", &["shape", "pattern", "position", "visible", "values", "details", "layers", "units", "measure"]),
    ("material", &["substance", "surface", "labels", "templates"]),
    ("light", &["shape", "brightness", "colour", "visibility", "light_linking", "shadow_linking", "templates"]),
    ("cutaway", &["templates"]),
    ("environment", &["turn", "background", "ground", "sky", "pins"]),
    ("lighting", &["settings"]),
    ("camera", &["lens", "focus", "exposure", "guides", "match"]),
    ("image", &["adjust", "tones", "white_balance", "lens_effects", "output", "denoise"]),
    ("snapshots", &["turntable", "model_sets", "variants", "configurator"]),
];

/// The Environment page's map, in what is kept in view.
const MAP: &str = "environment.map";

/// The height of a section's title row, the card's top edge included: where
/// a drag starts and where the grip sits.
const TITLE: f32 = 36.0;

/// The space a card leaves under itself (`field::Fold`'s).
const CARD_GAP: f32 = 2.0;

/// A page's sections in their default order, or `None` for no such page.
pub fn defaults(page: &str) -> Option<&'static [&'static str]> {
    PAGES.iter().find(|(name, _)| *name == page).map(|(_, keys)| *keys)
}

/// The kept orders, and a drag a script holds for a picture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Store {
    /// Each page's order where it differs from the default. A section
    /// this version does not have - one a newer Tequila added - is kept in
    /// its place and skipped when drawing, so going back a version and
    /// forward again loses nothing.
    pages: BTreeMap<String, Vec<String>>,
    /// The lines of pages this version does not have, written back as they
    /// were read.
    unknown: Vec<String>,
    /// What is kept in view while its page scrolls: `environment.map`, the
    /// Environment page's map. Kept as a list, written `keep=...`, so a
    /// later version can keep more without this one losing them.
    kept: Vec<String>,
    /// A drag held part-way for a picture: the page, the section and the
    /// slot it would land in (0 above the first section shown, the count
    /// below the last). Nothing moves.
    pub staged: Option<(String, String, usize)>,
}

impl Store {
    /// A page's sections in the order to draw them: the kept order, less
    /// any section the page no longer has, with any it has gained put where
    /// the default puts it, after the section it follows there.
    pub fn order(&self, page: &str) -> Vec<&'static str> {
        let Some(default) = defaults(page) else { return Vec::new() };
        let Some(kept) = self.pages.get(page) else { return default.to_vec() };
        let mut out: Vec<&'static str> = kept.iter().filter_map(|key| default.iter().copied().find(|d| d == key)).collect();
        out.dedup();
        for (index, key) in default.iter().enumerate() {
            if out.contains(key) {
                continue;
            }
            let at = default[..index].iter().rev().find_map(|before| out.iter().position(|k| k == before)).map_or(0, |p| p + 1);
            out.insert(at, key);
        }
        out
    }

    /// Put a page's sections in this order. Every name must be one of the
    /// page's sections; any left out follow them, in their default order.
    pub fn set(&mut self, page: &str, order: &[String]) -> Result<(), String> {
        let Some(default) = defaults(page) else {
            return Err(format!("there is no page `{page}` with sections; the pages are {}", page_names()));
        };
        if let Some(unknown) = order.iter().find(|key| !default.contains(&key.as_str())) {
            return Err(format!("the {page} page has no section `{unknown}`; its sections are {}", default.join(", ")));
        }
        let mut whole: Vec<String> = Vec::new();
        for key in order.iter().map(String::as_str).chain(default.iter().copied()) {
            if !whole.iter().any(|k| k == key) {
                whole.push(key.to_string());
            }
        }
        // Sections a newer version had in the kept order stay after the
        // section they followed there.
        if let Some(kept) = self.pages.get(page) {
            for (index, key) in kept.iter().enumerate() {
                if default.contains(&key.as_str()) || whole.contains(key) {
                    continue;
                }
                let at = kept[..index].iter().rev().find_map(|before| whole.iter().position(|k| k == before)).map_or(0, |p| p + 1);
                whole.insert(at, key.clone());
            }
        }
        if whole.iter().map(String::as_str).eq(default.iter().copied()) {
            self.pages.remove(page);
        } else {
            self.pages.insert(page.to_string(), whole);
        }
        Ok(())
    }

    /// Put a page back in its default order, or every page.
    pub fn reset(&mut self, page: Option<&str>) {
        match page {
            Some(page) => {
                self.pages.remove(page);
            }
            None => self.pages.clear(),
        }
    }

    /// The pages whose order is their own.
    #[cfg(test)]
    pub fn changed_pages(&self) -> impl Iterator<Item = &str> {
        self.pages.keys().map(String::as_str)
    }

    /// Whether the Environment page's map stays at the top while the rest
    /// of the page scrolls.
    pub fn map_in_view(&self) -> bool {
        self.kept.iter().any(|k| k == MAP)
    }

    pub fn set_map_in_view(&mut self, on: bool) {
        self.kept.retain(|k| k != MAP);
        if on {
            self.kept.push(MAP.to_string());
        }
    }

    /// Where the orders are kept: `sections.txt`, beside the other settings.
    pub fn file() -> PathBuf {
        tequila_format::library::settings_directory().join("sections.txt")
    }

    pub fn load(file: &Path) -> Store {
        std::fs::read_to_string(file).map(|text| Store::from_text(&text)).unwrap_or_default()
    }

    pub fn save(&self, file: &Path) -> Result<(), String> {
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        }
        std::fs::write(file, self.to_text()).map_err(|e| e.to_string())
    }

    /// One line a page, `environment=pins,turn,background,ground,sky`,
    /// taken as written: what this version does not know is kept for the
    /// version that does.
    fn from_text(text: &str) -> Store {
        let mut store = Store::default();
        for line in text.lines() {
            let Some((page, keys)) = line.split_once('=') else { continue };
            let page = page.trim();
            if page == "keep" {
                store.kept = keys.split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
                continue;
            }
            if defaults(page).is_none() {
                store.unknown.push(line.trim().to_string());
                continue;
            }
            let mut keys: Vec<String> = keys.split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
            let mut seen = std::collections::BTreeSet::new();
            keys.retain(|k| seen.insert(k.clone()));
            if !keys.is_empty() {
                store.pages.insert(page.to_string(), keys);
            }
        }
        store
    }

    fn to_text(&self) -> String {
        let mut text: String = self.pages.iter().map(|(page, keys)| format!("{page}={}\n", keys.join(","))).collect();
        if !self.kept.is_empty() {
            text.push_str(&format!("keep={}\n", self.kept.join(",")));
        }
        for line in &self.unknown {
            text.push_str(line);
            text.push('\n');
        }
        text
    }
}

/// The names of the pages with sections, for a message.
pub fn page_names() -> String {
    PAGES.iter().map(|(name, _)| *name).collect::<Vec<_>>().join(", ")
}

fn store_id() -> egui::Id {
    egui::Id::new("inspector-sections")
}

fn changed_id() -> egui::Id {
    store_id().with("changed")
}

/// Lend the kept orders to the context for this frame.
pub fn install(ctx: &egui::Context, store: &Store) {
    ctx.data_mut(|d| d.insert_temp(store_id(), store.clone()));
}

/// The orders, if a drop or a menu changed them this frame.
pub fn take_changed(ctx: &egui::Context) -> Option<Store> {
    ctx.data_mut(|d| {
        let changed = d.remove_temp::<bool>(changed_id()).unwrap_or(false);
        changed.then(|| d.get_temp::<Store>(store_id()).unwrap_or_default())
    })
}

fn current(ctx: &egui::Context) -> Store {
    ctx.data(|d| d.get_temp::<Store>(store_id())).unwrap_or_default()
}

fn put(ctx: &egui::Context, store: Store) {
    ctx.data_mut(|d| {
        d.insert_temp(store_id(), store);
        d.insert_temp(changed_id(), true);
    });
}

/// What the grip's menu asked for, carried out once every section has been
/// drawn and their places are known.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ask {
    Up,
    Down,
    Default,
}

/// A page being drawn: its sections in their order, and where each landed.
pub struct Page {
    name: &'static str,
    id: egui::Id,
    order: Vec<&'static str>,
    /// Each section drawn this frame: its key, its title and its card.
    drawn: Vec<(&'static str, String, egui::Rect)>,
    /// How many sections the page showed last frame: a page of one has
    /// nothing to put in order, and shows no grips.
    shown_before: usize,
    ask: Option<(&'static str, Ask)>,
}

impl Page {
    pub fn new(ui: &egui::Ui, name: &'static str) -> Page {
        let id = egui::Id::new(("inspector-page", name));
        Page {
            name,
            id,
            order: current(ui.ctx()).order(name),
            drawn: Vec::new(),
            shown_before: ui.data(|d| d.get_temp::<usize>(id.with("shown"))).unwrap_or(2),
            ask: None,
        }
    }

    /// The page's sections in the order to draw them.
    pub fn order(&self) -> Vec<&'static str> {
        self.order.clone()
    }

    /// A section whose card `add` draws, titled `title` in its drag and
    /// menu. A section `add` draws nothing for is left out of the order's
    /// places.
    pub fn section(&mut self, ui: &mut egui::Ui, key: &'static str, title: &str, add: impl FnOnce(&mut egui::Ui)) {
        let movable = self.shown_before > 1;
        // Under the title, before the card: see the module's note.
        let title_rect = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), TITLE));
        let drag = movable.then(|| ui.interact(title_rect, self.id.with(("title", key)), egui::Sense::drag()));
        // Under an id of the section's own, so what it remembers - open or
        // shut, a field being typed in - goes with it when it moves.
        let block = ui.push_id(("section", key), add).response.rect;
        if block.height() < 2.0 {
            return;
        }
        // The wrapper's own block ends after the card's trailing space, and
        // the page would add its spacing again: the gap between two cards
        // stays what it was without the wrapper. The card is the block less
        // that trailing space.
        let trailing = ui.spacing().item_spacing.y + CARD_GAP;
        ui.add_space(-ui.spacing().item_spacing.y);
        let card = egui::Rect::from_min_max(block.min, egui::pos2(block.max.x, (block.max.y - trailing).max(block.min.y + TITLE)));
        self.drawn.push((key, title.to_string(), card));
        if !movable {
            return;
        }
        // The grip at the title's right end: a drag from it moves the
        // section as a drag from the title does, and a click opens the menu
        // of ways to move it without dragging.
        let at = egui::Rect::from_center_size(egui::pos2(card.right() - 17.0, card.top() + 18.0), egui::vec2(20.0, 24.0));
        let grip = ui.interact(at, self.id.with(("grip", key)), egui::Sense::click_and_drag());
        grip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("Move {title}")));
        let dragging = ui.data(|d| d.get_temp::<&'static str>(self.id.with("drag"))) == Some(key);
        if ui.is_rect_visible(at) {
            let p = theme::p();
            let lit = grip.hovered() || dragging || drag.as_ref().is_some_and(|d| d.hovered());
            icons::paint(ui.painter(), at.shrink2(egui::vec2(2.0, 4.0)), Icon::Grip, if lit { p.text_dim } else { p.text_faint });
        }
        if grip.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if drag.as_ref().is_some_and(|d| d.drag_started()) || grip.drag_started() {
            ui.data_mut(|d| d.insert_temp(self.id.with("drag"), key));
        }
        let menu = self.id.with(("menu", key));
        if grip.clicked() || grip.secondary_clicked() {
            ui.memory_mut(|m| m.toggle_popup(menu));
        }
        let grip = icons::tip(grip, &format!("Move {title}"), "Drag the title up or down the page, or click for Move up and Move down. Each page keeps its own order.", "");
        let mut ask = None;
        egui::popup::popup_below_widget(ui, menu, &grip, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            ui.set_min_width(190.0);
            if ui.button("Move up").clicked() {
                ask = Some(Ask::Up);
            }
            if ui.button("Move down").clicked() {
                ask = Some(Ask::Down);
            }
            ui.separator();
            if ui.button("Put this page back in order").on_hover_text("Every section of this page where it was to begin with").clicked() {
                ask = Some(Ask::Default);
            }
        });
        if let Some(ask) = ask {
            self.ask = Some((key, ask));
        }
    }

    /// After the last section: the drop of a drag, a menu's move, and the
    /// drag drawn - the section lifted, where it would land, and its title
    /// under the pointer.
    pub fn finish(self, ui: &mut egui::Ui) {
        let Page { name, id, order, drawn, ask, .. } = self;
        ui.data_mut(|d| {
            d.insert_temp(id.with("shown"), drawn.len());
            d.insert_temp(id.with("drawn"), drawn.iter().map(|(k, _, r)| (*k, *r)).collect::<Vec<_>>());
        });
        let keys: Vec<&'static str> = drawn.iter().map(|(k, _, _)| *k).collect();
        let rects: Vec<egui::Rect> = drawn.iter().map(|(_, _, r)| *r).collect();

        if let Some((key, ask)) = ask {
            let from = keys.iter().position(|k| *k == key);
            let next = match (ask, from) {
                (Ask::Default, _) => Some(None),
                (Ask::Up, Some(from)) if from > 0 => Some(Some(place(&order, key, &keys, from - 1))),
                (Ask::Down, Some(from)) if from + 1 < keys.len() => Some(Some(place(&order, key, &keys, from + 2))),
                _ => None,
            };
            if let Some(next) = next {
                keep(ui.ctx(), name, next);
            }
        }

        let held: Option<&'static str> = ui.data(|d| d.get_temp(id.with("drag")));
        let staged = current(ui.ctx()).staged.filter(|(page, _, _)| page == name);
        let (key, slot, real) = match (held, staged) {
            (Some(key), _) => {
                let y = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos())).map_or(f32::NAN, |p| p.y);
                (key, drop_slot(&rects, y), true)
            }
            (None, Some((_, key, slot))) => match keys.iter().find(|k| **k == key) {
                Some(key) => (*key, slot.min(keys.len()), false),
                None => return,
            },
            (None, None) => return,
        };
        let Some(from) = keys.iter().position(|k| *k == key) else {
            ui.data_mut(|d| d.remove::<&'static str>(id.with("drag")));
            return;
        };

        if real {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                ui.data_mut(|d| d.remove::<&'static str>(id.with("drag")));
                return;
            }
            if !ui.input(|i| i.pointer.any_down()) {
                ui.data_mut(|d| d.remove::<&'static str>(id.with("drag")));
                if slot != from && slot != from + 1 {
                    keep(ui.ctx(), name, Some(place(&order, key, &keys, slot)));
                }
                return;
            }
            // Near the top or bottom of the page, the page scrolls.
            if let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) {
                let clip = ui.clip_rect();
                let edge = 40.0;
                let speed = if pointer.y < clip.top() + edge {
                    (clip.top() + edge - pointer.y).min(edge) * 0.3
                } else if pointer.y > clip.bottom() - edge {
                    -(pointer.y - (clip.bottom() - edge)).min(edge) * 0.3
                } else {
                    0.0
                };
                if speed != 0.0 {
                    ui.scroll_with_delta(egui::vec2(0.0, speed));
                }
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            ui.ctx().request_repaint();
        }

        // The section lifted where it is.
        let p = theme::p();
        let lifted = rects[from];
        ui.painter().rect(lifted, 10.0, p.accent.gamma_multiply(if p.dark { 0.12 } else { 0.08 }), egui::Stroke::new(1.0_f32, p.accent.gamma_multiply(0.6)));
        // Where it would land: a line across the gap, a dot at each end.
        let y = if slot == 0 {
            rects[0].top() - 3.0
        } else if slot >= rects.len() {
            rects[rects.len() - 1].bottom() + 1.0
        } else {
            (rects[slot - 1].bottom() + rects[slot].top()) * 0.5
        };
        if slot != from && slot != from + 1 {
            let (left, right) = (lifted.left() + 2.0, lifted.right() - 2.0);
            let line = egui::Stroke::new(2.0_f32, p.accent);
            ui.painter().line_segment([egui::pos2(left, y), egui::pos2(right, y)], line);
            ui.painter().circle_filled(egui::pos2(left, y), 3.5, p.accent);
            ui.painter().circle_filled(egui::pos2(right, y), 3.5, p.accent);
        }
        // Its title on a card under the pointer, or for a picture, beside
        // where it would land.
        let title = &drawn[from].1;
        let anchor = if real { ui.input(|i| i.pointer.hover_pos()).unwrap_or(lifted.min) } else { egui::pos2(lifted.left() + 40.0, y + 8.0) };
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("ghost")));
        let galley = painter.layout_no_wrap(title.clone(), theme::strong(theme::BODY), p.text);
        let size = egui::vec2(galley.size().x + 52.0, 34.0);
        let ghost = egui::Rect::from_min_size(anchor + egui::vec2(-20.0, -size.y * 0.5), size);
        painter.add(egui::epaint::Shadow { offset: egui::vec2(0.0, 6.0), blur: 18.0, spread: 0.0, color: p.shadow }.as_shape(ghost, 9.0));
        painter.rect(ghost, 9.0, p.raised, egui::Stroke::new(1.0_f32, p.accent));
        icons::paint(&painter, egui::Rect::from_center_size(egui::pos2(ghost.left() + 17.0, ghost.center().y), egui::vec2(14.0, 16.0)), Icon::Grip, p.text_dim);
        painter.galley(egui::pos2(ghost.left() + 32.0, ghost.center().y - galley.size().y * 0.5), galley, egui::Color32::PLACEHOLDER);
    }
}

/// Where each of a page's sections was drawn last frame, in its order.
#[cfg(test)]
pub fn drawn(ctx: &egui::Context, page: &str) -> Vec<(&'static str, egui::Rect)> {
    ctx.data(|d| d.get_temp(egui::Id::new(("inspector-page", page)).with("drawn"))).unwrap_or_default()
}

/// Keep a page's new order (`None`: its default), for the application to
/// save after the frame.
fn keep(ctx: &egui::Context, page: &str, order: Option<Vec<&'static str>>) {
    let mut store = current(ctx);
    match order {
        Some(order) => {
            let order: Vec<String> = order.iter().map(|k| k.to_string()).collect();
            let _ = store.set(page, &order);
        }
        None => store.reset(Some(page)),
    }
    put(ctx, store);
}

/// Which gap between the cards `y` is nearest: 0 above the first, the
/// count below the last.
fn drop_slot(rects: &[egui::Rect], y: f32) -> usize {
    if !y.is_finite() {
        return 0;
    }
    rects.iter().take_while(|r| y > r.center().y).count()
}

/// The whole order with `key` moved into gap `slot` of the sections shown
/// (`shown`, in their order): before the shown section that will follow it,
/// or after the last shown one. Sections not shown keep their places.
fn place(order: &[&'static str], key: &'static str, shown: &[&'static str], slot: usize) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = order.iter().copied().filter(|k| *k != key).collect();
    match shown.iter().skip(slot).find(|k| **k != key) {
        Some(following) => {
            let at = out.iter().position(|k| k == following).unwrap_or(out.len());
            out.insert(at, key);
        }
        None => {
            let at = shown.iter().rev().find(|k| **k != key).and_then(|last| out.iter().position(|k| k == last)).map_or(out.len(), |p| p + 1);
            out.insert(at, key);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn a_page_without_an_order_of_its_own_has_its_default() {
        let store = Store::default();
        assert_eq!(store.order("environment"), vec!["turn", "background", "ground", "sky", "pins"]);
        assert!(store.order("nonsense").is_empty());
    }

    /// The pins moved under the map, as the person who asked for it would
    /// put them; a section the page no longer has is dropped, and one it has
    /// gained comes in after the section it follows by default.
    #[test]
    fn a_kept_order_survives_sections_coming_and_going() {
        let mut store = Store::default();
        store.set("environment", &strings(&["pins", "turn", "background", "ground", "sky"])).unwrap();
        assert_eq!(store.order("environment"), vec!["pins", "turn", "background", "ground", "sky"]);
        // As if `ground` were new, and an old name were still in the file.
        store.pages.insert("environment".into(), strings(&["pins", "gone", "turn", "background", "sky"]));
        assert_eq!(store.order("environment"), vec!["pins", "turn", "background", "ground", "sky"]);
    }

    #[test]
    fn an_order_names_only_the_pages_own_sections() {
        let mut store = Store::default();
        assert!(store.set("environment", &strings(&["pins", "lens"])).unwrap_err().contains("no section `lens`"));
        assert!(store.set("kitchen", &strings(&["pins"])).unwrap_err().contains("no page `kitchen`"));
        assert_eq!(store, Store::default(), "a refused order changes nothing");
        // Named in part: the rest follow, in their default order.
        store.set("camera", &strings(&["match"])).unwrap();
        assert_eq!(store.order("camera"), vec!["match", "lens", "focus", "exposure", "guides"]);
        // The default order is not kept as the page's own.
        store.set("camera", &strings(&["lens", "focus", "exposure", "guides", "match"])).unwrap();
        assert_eq!(store.changed_pages().count(), 0);
    }

    #[test]
    fn the_orders_are_kept_in_a_file() {
        let file = std::env::temp_dir().join(format!("tequila-sections-{}", std::process::id())).join("sections.txt");
        let mut store = Store::default();
        store.set("environment", &strings(&["pins", "turn"])).unwrap();
        store.set("image", &strings(&["denoise"])).unwrap();
        store.staged = Some(("environment".into(), "pins".into(), 0));
        store.save(&file).unwrap();
        let back = Store::load(&file);
        assert_eq!(back.order("environment"), store.order("environment"));
        assert_eq!(back.order("image"), vec!["denoise", "adjust", "tones", "white_balance", "lens_effects", "output"]);
        assert_eq!(back.staged, None, "a drag held for a picture is not kept");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
        assert_eq!(Store::load(&file), Store::default(), "no file is the defaults");
    }

    /// The file of a newer Tequila - a page this one does not have, and a
    /// section it does not have on a page it does - read, drawn without
    /// them, changed and written back with them where they were; and the
    /// file of an older one, short of a section, read with it in its
    /// default place.
    #[test]
    fn an_order_survives_a_newer_and_an_older_version() {
        let newer = "environment=pins,studio_floor,turn,background,ground,sky\nturntable=speed,frames\n";
        let mut store = Store::from_text(newer);
        assert_eq!(store.order("environment"), vec!["pins", "turn", "background", "ground", "sky"], "an unknown section is skipped");
        store.set("environment", &strings(&["pins", "turn", "ground", "background", "sky"])).unwrap();
        let text = store.to_text();
        assert!(text.contains("environment=pins,studio_floor,turn,ground,background,sky"), "the newer section kept its place: {text}");
        assert!(text.contains("turntable=speed,frames"), "the newer page was kept: {text}");
        // Back in the newer version, put back in this one's default order:
        // the newer section is still kept.
        store.set("environment", &strings(&["turn", "background", "ground", "sky", "pins"])).unwrap();
        assert!(store.to_text().contains("studio_floor"));

        let older = "environment=sky,turn,background,pins\n";
        let store = Store::from_text(older);
        assert_eq!(store.order("environment"), vec!["sky", "turn", "background", "ground", "pins"], "a section the file lacks comes after the one it follows by default");
    }

    /// The map kept in view is kept in the file, beside the orders, and
    /// what a later version keeps in view is not lost.
    #[test]
    fn the_map_kept_in_view_is_kept_in_the_file() {
        let mut store = Store::default();
        assert!(!store.map_in_view(), "it scrolls with the page, as it comes");
        store.set_map_in_view(true);
        let back = Store::from_text(&store.to_text());
        assert!(back.map_in_view());
        let mut later = Store::from_text("keep=environment.map,scene.tree\nenvironment=pins\n");
        assert!(later.map_in_view());
        later.set_map_in_view(false);
        assert_eq!(later.to_text().lines().find(|l| l.starts_with("keep=")), Some("keep=scene.tree"));
        later.reset(None);
        assert!(later.to_text().contains("keep=scene.tree"), "putting the pages back in order keeps what is kept in view");
    }

    /// Moving among the sections shown leaves those not shown where they
    /// were: Tones and the rest are Photographic's alone.
    #[test]
    fn a_section_moves_among_those_shown() {
        let order = vec!["adjust", "tones", "white_balance", "lens_effects", "output", "denoise"];
        let shown = vec!["adjust", "output", "denoise"];
        assert_eq!(place(&order, "denoise", &shown, 0), vec!["denoise", "adjust", "tones", "white_balance", "lens_effects", "output"]);
        assert_eq!(place(&order, "adjust", &shown, 3), vec!["tones", "white_balance", "lens_effects", "output", "denoise", "adjust"]);
        assert_eq!(place(&order, "adjust", &shown, 2), vec!["tones", "white_balance", "lens_effects", "output", "adjust", "denoise"]);
    }

    #[test]
    fn the_gap_is_the_one_nearest_the_pointer() {
        let card = |top: f32| egui::Rect::from_min_size(egui::pos2(0.0, top), egui::vec2(100.0, 40.0));
        let rects = [card(0.0), card(50.0), card(100.0)];
        assert_eq!(drop_slot(&rects, -10.0), 0);
        assert_eq!(drop_slot(&rects, 19.0), 0);
        assert_eq!(drop_slot(&rects, 21.0), 1);
        assert_eq!(drop_slot(&rects, 75.0), 2);
        assert_eq!(drop_slot(&rects, 500.0), 3);
    }

    /// Three folds on a page, drawn as a person sees them, driven as they
    /// would: a click on a title still opens it, and a drag of a title moves
    /// its section and keeps the order.
    #[test]
    fn a_title_clicked_opens_its_section_and_dragged_moves_it() {
        struct Harness {
            ctx: egui::Context,
            time: f64,
            store: Store,
            titles: Vec<(&'static str, egui::Rect)>,
            open: Vec<(&'static str, bool)>,
        }
        impl Harness {
            fn frame(&mut self, events: Vec<egui::Event>) {
                self.time += 1.0 / 60.0;
                install(&self.ctx, &self.store);
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 600.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                };
                let mut titles = Vec::new();
                let mut open = Vec::new();
                let _ = self.ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let mut page = Page::new(ui, "environment");
                        for key in page.order() {
                            if !["turn", "background", "ground"].contains(&key) {
                                continue;
                            }
                            page.section(ui, key, key, |ui| {
                                let fold = crate::field::Fold::new(key).show(ui, |ui| {
                                    ui.label("inside");
                                });
                                titles.push((key, fold.header_response.rect));
                                open.push((key, fold.body_response.is_some()));
                            });
                        }
                        page.finish(ui);
                    });
                });
                if let Some(store) = take_changed(&self.ctx) {
                    self.store = store;
                }
                self.titles = titles;
                self.open = open;
            }
            fn title(&self, key: &str) -> egui::Pos2 {
                self.titles.iter().find(|(k, _)| *k == key).unwrap().1.left_center() + egui::vec2(60.0, 0.0)
            }
            fn pointer(&mut self, at: egui::Pos2, pressed: Option<bool>) {
                let mut events = vec![egui::Event::PointerMoved(at)];
                if let Some(pressed) = pressed {
                    events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
                }
                self.frame(events);
            }
        }
        let mut h = Harness { ctx: egui::Context::default(), time: 0.0, store: Store::default(), titles: Vec::new(), open: Vec::new() };
        h.frame(Vec::new());
        h.frame(Vec::new());
        assert!(h.open.iter().all(|(_, open)| !open), "the folds start shut");

        // A click on Background's title opens it, and nothing moves.
        let at = h.title("background");
        h.pointer(at, Some(true));
        h.pointer(at, Some(false));
        h.frame(Vec::new());
        assert!(h.open.iter().any(|(k, open)| *k == "background" && *open), "a click on the title did not open it");
        assert_eq!(h.store, Store::default(), "a click moved a section");

        // Ground's title dragged above Turn and size's.
        let from = h.title("ground");
        let to = h.title("turn") - egui::vec2(0.0, 14.0);
        h.pointer(from, Some(true));
        for step in 1..=8 {
            let t = step as f32 / 8.0;
            h.pointer(from + (to - from) * t, None);
        }
        h.pointer(to, Some(false));
        h.frame(Vec::new());
        assert_eq!(h.store.order("environment"), vec!["ground", "turn", "background", "sky", "pins"], "the drag did not move Ground to the top");
        let tops: Vec<&str> = {
            let mut titles = h.titles.clone();
            titles.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
            titles.iter().map(|(k, _)| *k).collect()
        };
        assert_eq!(tops, vec!["ground", "turn", "background"], "the page is not drawn in its new order");
    }
}
