//! A part's material from its right-click menu - Edit material, Copy
//! material, Paste material and Paste linked - in the viewport and in the
//! Scene tree alike, and from the keyboard: Alt+C copies the selected
//! part's material and Alt+V pastes it onto the selection (`shortcuts`).
//!
//! **What Copy keeps.** A material in Tequila is a shared instance
//! (`tequila_scene::material`): a part names one by its key, and the
//! material holds all of itself - its type, its graph with every texture
//! and the mapping that lays each one on, its labels, a Substance file's
//! settings. Copy keeps the whole of it as it is at that moment, with the
//! pictures it reads: what is pasted is what was copied, an edit to the
//! original afterwards is not, and it can be pasted in another project.
//!
//! **What Paste makes.** A copy of its own on the parts it is pasted onto
//! (`material.paste`), named as an unlinked copy is (`Paint 2`): editing it
//! later changes none of the parts the original is on, which is what
//! somebody coming from KeyShot expects of paste. The parts pasted onto
//! together share that one copy, as parts a library material is dropped on
//! together do; Unlink parts them. **Paste linked** puts the original
//! itself on them, shared, as picking it from the In-Project library does.
//!
//! **What stays with the part.** Its colour and number (read by Object
//! Info), the rays that see it, and its UV layout with the guides and pins
//! on it are the part's own, and do not travel. Texture mapping is the
//! material's, one per texture, laid on in each part's own coordinates, so
//! it travels with the textures: a projection sized for a part much larger
//! or smaller than the one pasted onto may want placing again.

use crate::project::Project;
use eframe::egui;
use std::collections::{BTreeMap, BTreeSet};
use tequila_materials::graph::TextureId;
use tequila_scene::command::{Command, ImportPayload};
use tequila_scene::material::MaterialDefinition;
use tequila_scene::selection::{Selection, SelectionMode};
use tequila_scene::textures::TextureSource;
use tequila_scene::tree::{MaterialKey, NodeId};
use tequila_scene::SceneDocument;

/// What Copy material kept, until the next copy.
#[derive(Debug, Default)]
pub(crate) struct Clipboard {
    copied: Option<Copied>,
}

#[derive(Clone, Debug)]
struct Copied {
    /// The project it was copied in (`Project::document_epoch`) and the
    /// material's key there: Paste linked needs the original itself, and a
    /// paste in the same project names it rather than carry it.
    epoch: u64,
    key: MaterialKey,
    /// The part it was copied from, for the menu to name.
    part: String,
    /// The material as it was, whole.
    definition: MaterialDefinition,
    /// The pictures it reads, by the ids its graph names them by.
    textures: Vec<(TextureId, TextureSource)>,
}

/// What a menu needs to know of the clipboard and the keys: made by the
/// application, so the Scene tree, which has only the project, can draw
/// the same entries.
#[derive(Clone, Debug, Default)]
pub(crate) struct Offer {
    /// The copied material's name and the part it was copied from.
    copied: Option<(String, String)>,
    /// Why Paste linked cannot be offered, when it cannot.
    unlinkable: Option<String>,
    /// Copy's and Paste's keys, as the menus show them.
    copy_keys: String,
    paste_keys: String,
}

/// What a material entry asked for, for the application to carry out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Select the node and open its material in the Material tab.
    Edit(NodeId),
    Copy(NodeId),
    /// Onto the node right-clicked, or onto the selection it is in.
    Paste { at: NodeId, linked: bool },
    Unlink(NodeId),
}

/// The parts a paste goes on.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Targets {
    /// What was pasted onto, selected afterwards: the node clicked, or the
    /// selection it was in.
    pub nodes: Vec<NodeId>,
    /// The parts at or under them that take the material: a group's own
    /// material is never drawn. Locked ones are left out.
    pub parts: Vec<NodeId>,
    /// How many locked parts were left out.
    pub locked: usize,
}

impl Targets {
    /// For a right-click on `at`: onto it alone, or onto everything
    /// selected when it is selected itself or is in a selected group - as
    /// a material dropped from the library goes on the selection when it
    /// is dropped on a part of it.
    pub(crate) fn at(document: &SceneDocument, selection: &Selection, at: NodeId) -> Targets {
        let in_selection = selection.contains(at) || document.tree.ancestors(at).iter().any(|&a| selection.contains(a));
        let nodes = if in_selection { selection.nodes().collect() } else { vec![at] };
        Targets::of(document, nodes)
    }

    /// Onto these nodes.
    pub(crate) fn of(document: &SceneDocument, nodes: Vec<NodeId>) -> Targets {
        let (parts, locked): (Vec<NodeId>, Vec<NodeId>) =
            tequila_scene::apply::parts_of(document, nodes.iter().copied()).into_iter().partition(|&part| !document.tree.is_effectively_locked(part));
        Targets { nodes, parts, locked: locked.len() }
    }

    /// The parts, in words: "`Lid`", "the 4 parts in `Wheels`", "the 3
    /// selected parts".
    fn words(&self, document: &SceneDocument) -> String {
        match (self.nodes.as_slice(), self.parts.as_slice()) {
            (_, [part]) => format!("`{}`", name(document, *part)),
            ([node], parts) => format!("the {} parts in `{}`", parts.len(), name(document, *node)),
            (_, parts) => format!("the {} selected parts", parts.len()),
        }
    }
}

fn name(document: &SceneDocument, node: NodeId) -> String {
    document.tree.get(node).map(|n| n.name.clone()).unwrap_or_default()
}

/// The material a node shows in the Material tab: its own, or a group's
/// first part's.
pub(crate) fn material_of(document: &SceneDocument, node: NodeId) -> Option<MaterialKey> {
    let own = |id: NodeId| document.tree.get(id).and_then(|n| n.material);
    own(node).or_else(|| tequila_scene::apply::parts_of(document, [node]).into_iter().find_map(own))
}

/// Every picture a material reads: its image and volume nodes', its
/// labels', and a Substance material's maps.
fn pictures(definition: &MaterialDefinition) -> BTreeSet<TextureId> {
    // Read through the graph's own renumbering, so a kind of node that
    // reads a picture is found here the day it is added there.
    let found = std::cell::RefCell::new(BTreeSet::new());
    definition.graph.clone().remap_textures(&|id| {
        found.borrow_mut().insert(id);
        id
    });
    let mut found = found.into_inner();
    if let Some(link) = &definition.substance {
        found.extend(link.maps.values().copied());
    }
    found
}

impl Copied {
    /// The material as `document` would hold it, and the pictures it must
    /// add first: a picture the document has already - the same file, for
    /// the same use - is read where it is, and the rest are given ids of
    /// their own.
    fn for_document(&self, document: &SceneDocument) -> (MaterialDefinition, Vec<(TextureId, TextureSource)>) {
        let mut next = document.textures.next_id();
        let mut renumbered = BTreeMap::new();
        let mut added = Vec::new();
        for (id, source) in &self.textures {
            let target = document.textures.iter().find(|(_, s)| *s == source).map(|(id, _)| id).unwrap_or_else(|| {
                let target = TextureId(next);
                next += 1;
                added.push((target, source.clone()));
                target
            });
            renumbered.insert(*id, target);
        }
        let mut definition = self.definition.clone();
        let renumber = |id: TextureId| renumbered.get(&id).copied().unwrap_or(id);
        definition.graph.remap_textures(&renumber);
        if let Some(link) = &mut definition.substance {
            for id in link.maps.values_mut() {
                *id = renumber(*id);
            }
        }
        (definition, added)
    }
}

impl Clipboard {
    /// Keep the material `node` shows, whole. Answers with its name.
    pub(crate) fn copy(&mut self, project: &Project, node: NodeId) -> Result<String, String> {
        let document = project.document();
        let key = material_of(document, node).ok_or("This part has no material to copy.")?;
        let definition = document.materials.get(key).ok_or("This part's material is missing.")?.clone();
        let textures = pictures(&definition).into_iter().filter_map(|id| document.textures.get(id).map(|source| (id, source.clone()))).collect();
        let material = definition.name.clone();
        self.copied = Some(Copied { epoch: project.document_epoch(), key, part: name(document, node), definition, textures });
        Ok(material)
    }

    /// The menus' view of it, with the keys they show.
    pub(crate) fn offer(&self, project: &Project, copy_keys: String, paste_keys: String) -> Offer {
        let copied = self.copied.as_ref().map(|c| (c.definition.name.clone(), c.part.clone()));
        let unlinkable = self.copied.as_ref().and_then(|c| unlinkable(c, project.document_epoch(), project.document()));
        Offer { copied, unlinkable, copy_keys, paste_keys }
    }

    /// The commands a paste onto `parts` is, in order, or why there are
    /// none.
    fn commands(&self, epoch: u64, document: &SceneDocument, parts: &[NodeId], linked: bool) -> Result<Vec<Command>, String> {
        let copied = self.copied.as_ref().ok_or_else(|| NOTHING_COPIED.to_string())?;
        if parts.is_empty() {
            return Err("There is no part there to paste onto.".into());
        }
        let nodes = parts.to_vec();
        if linked {
            if let Some(reason) = unlinkable(copied, epoch, document) {
                return Err(reason);
            }
            return Ok(vec![Command::AssignMaterial { nodes, material: copied.key }]);
        }
        // The original, as it was copied: a copy of it, by name.
        if copied.epoch == epoch && document.materials.get(copied.key) == Some(&copied.definition) {
            return Ok(vec![Command::PasteMaterial { material: copied.key, nodes }]);
        }
        // Changed since, taken back by an undo, or copied in another
        // project: the copy kept is brought in, with its pictures, and put
        // on the parts, as a library material is.
        let (mut definition, textures) = copied.for_document(document);
        if document.materials.iter().any(|(_, d)| d.name == definition.name) {
            definition.name = document.materials.next_available_name(&definition.name);
        }
        let key = MaterialKey(document.materials.next_key());
        Ok(vec![
            Command::Import {
                payload: Box::new(ImportPayload { meshes: Vec::new(), textures, materials: vec![(key, definition)], nodes: Vec::new(), roots: Vec::new() }),
            },
            Command::AssignMaterial { nodes, material: key },
        ])
    }

    /// Paste onto `parts`, as one step to undo. Answers with the name of
    /// the material they now wear.
    pub(crate) fn paste(&self, project: &mut Project, parts: &[NodeId], linked: bool) -> Result<String, String> {
        let commands = self.commands(project.document_epoch(), project.document(), parts, linked)?;
        // Inside a group already open, that group is the one step.
        let own_step = commands.len() > 1 && !project.in_group();
        if own_step {
            project.begin_group();
        }
        for command in commands {
            if let Err(error) = project.apply(command) {
                if own_step {
                    project.abandon_group();
                }
                return Err(error.to_string());
            }
        }
        if own_step {
            project.end_group();
        }
        project.end_interaction();
        let document = project.document();
        Ok(parts.first().and_then(|p| material_of(document, *p)).and_then(|k| document.materials.get(k)).map(|d| d.name.clone()).unwrap_or_default())
    }
}

const NOTHING_COPIED: &str = "Nothing copied yet: right-click a part and choose Copy material.";

/// Why the original itself cannot go on another part, when it cannot.
fn unlinkable(copied: &Copied, epoch: u64, document: &SceneDocument) -> Option<String> {
    if copied.epoch != epoch {
        Some(format!("`{}` was copied in another project: Paste material brings a copy of it here.", copied.definition.name))
    } else if document.materials.get(copied.key).is_none() {
        Some(format!("`{}` is no longer in this project: Paste material brings back a copy of it.", copied.definition.name))
    } else {
        None
    }
}

/// One entry of the menu, worked out before it is drawn, so the words can
/// be checked without a window.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    pub label: String,
    pub enabled: bool,
    pub hover: String,
}

impl Entry {
    fn on(label: impl Into<String>, hover: impl Into<String>) -> Entry {
        Entry { label: label.into(), enabled: true, hover: hover.into() }
    }

    fn off(label: impl Into<String>, hover: impl Into<String>) -> Entry {
        Entry { label: label.into(), enabled: false, hover: hover.into() }
    }

    /// Drawn as a menu button, with its keys at the right when it has any.
    fn show(&self, ui: &mut egui::Ui, keys: &str) -> egui::Response {
        let mut button = egui::Button::new(&self.label);
        if !keys.is_empty() {
            button = button.shortcut_text(keys);
        }
        let response = ui.add_enabled(self.enabled, button);
        if self.enabled {
            response.on_hover_text(&self.hover)
        } else {
            response.on_disabled_hover_text(&self.hover)
        }
    }
}

pub(crate) fn edit_entry(document: &SceneDocument, at: NodeId) -> Entry {
    match material_of(document, at).and_then(|k| document.materials.get(k)) {
        Some(material) => Entry::on("Edit material", format!("Select `{}` and open `{}` in the Material tab", name(document, at), material.name)),
        None => Entry::off("Edit material", "This part has no material yet: drag one onto it from the Library"),
    }
}

pub(crate) fn copy_entry(document: &SceneDocument, at: NodeId) -> Entry {
    match material_of(document, at).and_then(|k| document.materials.get(k)) {
        Some(material) => Entry::on("Copy material", format!("Keep `{}` to paste onto other parts: all of it, with its textures, their mapping and its labels", material.name)),
        None => Entry::off("Copy material", "This part has no material to copy"),
    }
}

/// Paste, or Paste linked: what would be pasted, and onto what.
pub(crate) fn paste_entry(document: &SceneDocument, offer: &Offer, targets: &Targets, linked: bool) -> Entry {
    let verb = if linked { "Paste linked" } else { "Paste material" };
    let label = match targets.parts.len() {
        0 | 1 => verb.to_string(),
        n => format!("{verb} onto {n} parts"),
    };
    let Some((material, from)) = &offer.copied else {
        let keys = if offer.copy_keys.is_empty() { String::new() } else { format!(" ({})", offer.copy_keys) };
        return Entry::off(label, format!("{}{keys}", NOTHING_COPIED.trim_end_matches('.')));
    };
    if targets.parts.is_empty() {
        return Entry::off(
            label,
            match targets.locked {
                0 => "There is no part here to paste onto".to_string(),
                1 => "That part is locked: unlock it in the Scene tree first".to_string(),
                _ => "Those parts are locked: unlock them in the Scene tree first".to_string(),
            },
        );
    }
    if linked {
        if let Some(reason) = &offer.unlinkable {
            return Entry::off(label, reason.trim_end_matches('.'));
        }
    }
    let onto = targets.words(document);
    let mut hover = if linked {
        format!("`{material}` itself on {onto}, shared with the parts that wear it: changing it changes them all")
    } else {
        format!("A copy of `{material}`, copied from `{from}`, on {onto} as a material of its own: changing it later changes nothing else")
    };
    match targets.locked {
        0 => {}
        1 => hover.push_str(". 1 locked part keeps its own"),
        n => hover.push_str(&format!(". {n} locked parts keep theirs")),
    }
    Entry::on(label, hover)
}

/// The material entries of a right-click on `at`, a part or a group: Edit,
/// Copy, Paste and Paste linked, and Unlink when it wears a material of its
/// own. Answers with the one chosen; the caller closes the menu.
pub(crate) fn menu_items(ui: &mut egui::Ui, document: &SceneDocument, selection: &Selection, offer: &Offer, at: NodeId) -> Option<Action> {
    let targets = Targets::at(document, selection, at);
    let mut chosen = None;
    if edit_entry(document, at).show(ui, "").clicked() {
        chosen = Some(Action::Edit(at));
    }
    if copy_entry(document, at).show(ui, &offer.copy_keys).clicked() {
        chosen = Some(Action::Copy(at));
    }
    if paste_entry(document, offer, &targets, false).show(ui, &offer.paste_keys).clicked() {
        chosen = Some(Action::Paste { at, linked: false });
    }
    if paste_entry(document, offer, &targets, true).show(ui, "").clicked() {
        chosen = Some(Action::Paste { at, linked: true });
    }
    if let Some(material) = document.tree.get(at).and_then(|n| n.material).and_then(|k| document.materials.get(k)) {
        if ui.button("Unlink material").on_hover_text(format!("Give `{}` a copy of `{}` of its own, to change it alone", name(document, at), material.name)).clicked() {
            chosen = Some(Action::Unlink(at));
        }
    }
    chosen
}

impl crate::TequilaApp {
    /// What the menus show of the clipboard, with the keys as they are now.
    pub(crate) fn material_offer(&self) -> Offer {
        self.material_clipboard.offer(&self.project, self.key_text(crate::shortcuts::Action::CopyMaterial), self.key_text(crate::shortcuts::Action::PasteMaterial))
    }

    /// Carry out what a material entry asked for.
    pub(crate) fn part_material(&mut self, action: Action) {
        match action {
            Action::Edit(node) => {
                self.project.selection.select_many([node], SelectionMode::Replace);
                self.project_tab = crate::ProjectTab::Material;
                self.show_project = true;
            }
            Action::Copy(node) => self.copy_material(node),
            Action::Paste { at, linked } => {
                let targets = Targets::at(self.project.document(), &self.project.selection, at);
                self.paste_material(targets, linked);
            }
            Action::Unlink(node) => match self.project.apply(Command::UnlinkMaterial { node }) {
                Ok(()) => self.toasts.info("Material unlinked"),
                Err(error) => self.toasts.warn(error.to_string()),
            },
        }
    }

    fn copy_material(&mut self, node: NodeId) {
        match self.material_clipboard.copy(&self.project, node) {
            Ok(material) => {
                let keys = self.key_text(crate::shortcuts::Action::PasteMaterial);
                self.toasts.info(if keys.is_empty() {
                    format!("Copied `{material}`: right-click a part to paste it")
                } else {
                    format!("Copied `{material}`: {keys} pastes it onto the selected parts")
                });
            }
            Err(error) => self.toasts.warn(error),
        }
    }

    fn paste_material(&mut self, targets: Targets, linked: bool) {
        if targets.parts.is_empty() && targets.locked > 0 {
            self.toasts.warn(if targets.locked == 1 { "That part is locked: unlock it in the Scene tree first." } else { "Those parts are locked: unlock them in the Scene tree first." });
            return;
        }
        let onto = if targets.parts.is_empty() { String::new() } else { targets.words(self.project.document()) };
        match self.material_clipboard.paste(&mut self.project, &targets.parts, linked) {
            Ok(material) => {
                // Selected, so the Material tab shows what they wear now.
                self.project.selection.select_many(targets.nodes.iter().copied(), SelectionMode::Replace);
                let mut note = if linked { format!("`{material}` is on {onto} too, linked") } else { format!("Pasted `{material}` onto {onto}") };
                match targets.locked {
                    0 => {}
                    1 => note.push_str("; 1 locked part kept its own"),
                    n => note.push_str(&format!("; {n} locked parts kept theirs")),
                }
                self.toasts.info(note);
            }
            Err(error) => self.toasts.warn(error),
        }
    }

    /// Alt+C: the selected part's material.
    pub(crate) fn copy_selected_material(&mut self) {
        match self.project.selection.active() {
            Some(node) => self.copy_material(node),
            None => self.toasts.info("Select a part first, to copy its material."),
        }
    }

    /// Alt+V: onto the selection.
    pub(crate) fn paste_onto_selection(&mut self) {
        let nodes: Vec<NodeId> = self.project.selection.nodes().collect();
        if nodes.is_empty() {
            self.toasts.info("Select the parts to paste the material onto first.");
            return;
        }
        let targets = Targets::of(self.project.document(), nodes);
        self.paste_material(targets, false);
    }

    /// Edit › Copy material and Paste material, for the selection.
    pub(crate) fn material_menu_items(&mut self, ui: &mut egui::Ui) {
        let offer = self.material_offer();
        let document = self.project.document();
        let active = self.project.selection.active();
        let copy = match active {
            Some(node) => copy_entry(document, node),
            None => Entry::off("Copy material", "Select a part first, to copy its material"),
        };
        let targets = Targets::of(document, self.project.selection.nodes().collect());
        let paste = if targets.nodes.is_empty() {
            Entry::off("Paste material", "Select the parts to paste onto first")
        } else {
            paste_entry(document, &offer, &targets, false)
        };
        if copy.show(ui, &offer.copy_keys).clicked() {
            self.copy_selected_material();
            ui.close_menu();
        }
        if paste.show(ui, &offer.paste_keys).clicked() {
            self.paste_material(targets, false);
            ui.close_menu();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TequilaApp;
    use tequila_materials::material_type::{MaterialType, Param};
    use tequila_scene::command::MaterialParameter;
    use tequila_scene::shapes::Shape;
    use tequila_texture::texture::MapRole;

    /// A picture from the repository's own samples, which every machine
    /// that runs the tests has.
    fn picture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/samples/palma P8.textures/palma-p8 screen.png")
    }

    /// A shape beside the demo's ball, with a plastic of its own named for
    /// it.
    fn shape(app: &mut TequilaApp, shape: Shape) -> NodeId {
        let (payload, id) = tequila_scene::shapes::add(app.project.document(), shape, None);
        app.project.apply(Command::Import { payload: Box::new(payload) }).unwrap();
        app.project.end_interaction();
        id
    }

    /// Three of them.
    fn shapes(app: &mut TequilaApp) -> Vec<NodeId> {
        [Shape::Cube, Shape::Cylinder, Shape::Disc].into_iter().map(|s| shape(app, s)).collect()
    }

    fn material(app: &TequilaApp, node: NodeId) -> MaterialKey {
        app.project.document().tree.get(node).unwrap().material.unwrap()
    }

    /// A material with something in every part of it: another type, a
    /// texture laid on by a mapping of its own, a label, a Substance file's
    /// settings and a parameter moved.
    fn dress(app: &mut TequilaApp, key: MaterialKey) {
        let commands = [
            Command::SetMaterialType { material: key, material_type: MaterialType::Metal },
            Command::SetParameterTexture { material: key, param: Param::BaseColor, source: Some(TextureSource::new(picture(), MapRole::Color)) },
            Command::AddDecal { material: key, source: TextureSource::new(picture(), MapRole::Color), name: "Logo".into() },
            Command::SetMaterialParameter { material: key, parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.37) },
            Command::SetSubstance {
                material: key,
                substance: Some(Box::new(tequila_scene::material::SubstanceLink { file: "bricks.sbsar".into(), graph: "pkg://bricks".into(), ..Default::default() })),
            },
        ];
        for command in commands {
            app.project.apply(command).unwrap();
            app.project.end_interaction();
        }
        let mut mapping = tequila_texture::projection::Mapping::default();
        mapping.scale = tequila_core::math::vec3(3.0, 2.0, 1.0);
        mapping.rotation = 0.125;
        app.project.apply(Command::SetTextureMapping { material: key, param: Param::BaseColor, mapping: Box::new(mapping) }).unwrap();
        app.project.end_interaction();
    }

    /// Copy then paste carries the whole material - type, graph, texture,
    /// its mapping, the label, the Substance settings - onto the part as a
    /// copy of its own, named as one; editing the copy leaves the original
    /// alone, and one undo puts the part's own material back.
    #[test]
    fn a_pasted_material_is_all_of_the_copied_one_and_its_own() {
        let mut app = TequilaApp::for_tests();
        let parts = shapes(&mut app);
        let source = material(&app, parts[0]);
        dress(&mut app, source);
        let before = material(&app, parts[1]);
        let original = app.project.document().materials.get(source).unwrap().clone();

        app.part_material(Action::Copy(parts[0]));
        app.part_material(Action::Paste { at: parts[1], linked: false });

        let pasted = material(&app, parts[1]);
        assert_ne!(pasted, source, "paste linked the original rather than copying it");
        let copy = app.project.document().materials.get(pasted).unwrap().clone();
        assert_eq!(MaterialDefinition { name: original.name.clone(), ..copy.clone() }, original, "a field of the material did not travel");
        assert_eq!(copy.name, app.project.document().materials.get(source).unwrap().name.clone() + " 2");
        assert_eq!(copy.graph.decals.len(), 1);
        assert_eq!(app.project.selection.nodes().collect::<Vec<_>>(), vec![parts[1]], "the part pasted onto is selected, so the Material tab shows it");

        app.project.apply(Command::SetMaterialParameter { material: pasted, parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.9) }).unwrap();
        app.project.end_interaction();
        assert_eq!(app.project.document().materials.get(source), Some(&original), "editing the pasted copy changed the original");

        app.undo();
        app.undo();
        assert_eq!(material(&app, parts[1]), before, "undo did not put the part's own material back");
        assert!(app.project.document().materials.get(pasted).is_none(), "undo left the copy behind");
    }

    /// Pasted on a part of the selection, it goes on every selected part,
    /// one copy between them, in one step that undo takes back whole; a
    /// part outside the selection gets it alone.
    #[test]
    fn a_paste_on_a_selected_part_reaches_the_whole_selection_in_one_step() {
        let mut app = TequilaApp::for_tests();
        let parts = shapes(&mut app);
        let ball = app.project.document().tree.iter().find(|n| n.name == "Sphere").map(|n| n.id).unwrap();
        let own: Vec<MaterialKey> = [parts[1], parts[2], ball].iter().map(|&p| material(&app, p)).collect();
        let source = material(&app, parts[0]);
        app.part_material(Action::Copy(parts[0]));

        app.project.selection.select_many([parts[1], parts[2]], SelectionMode::Replace);
        let targets = Targets::at(app.project.document(), &app.project.selection, parts[2]);
        assert_eq!(targets.parts, vec![parts[1], parts[2]]);
        let depth = app.project.bus().depth();
        app.part_material(Action::Paste { at: parts[2], linked: false });
        assert_eq!(app.project.bus().depth(), depth + 1, "the paste was more than one step");
        let pasted = material(&app, parts[1]);
        assert_ne!(pasted, source);
        assert_eq!(material(&app, parts[2]), pasted, "the selected parts pasted onto together share one copy");
        assert_eq!(material(&app, ball), own[2], "a part not selected took the paste");

        app.undo();
        assert_eq!(material(&app, parts[1]), own[0]);
        assert_eq!(material(&app, parts[2]), own[1]);

        // Not selected: onto that part alone.
        let alone = Targets::at(app.project.document(), &app.project.selection, ball);
        assert_eq!(alone.parts, vec![ball]);
        assert_eq!(alone.nodes, vec![ball]);
    }

    /// Copied in one project, it pastes into another, its pictures with
    /// it; and in the same project after the original changed, it pastes
    /// what was copied, not the change. Paste linked is the original
    /// itself, and only where it is.
    #[test]
    fn what_is_pasted_is_what_was_copied_even_in_another_project() {
        let mut app = TequilaApp::for_tests();
        let parts = shapes(&mut app);
        let source = material(&app, parts[0]);
        dress(&mut app, source);
        let original = app.project.document().materials.get(source).unwrap().clone();
        app.part_material(Action::Copy(parts[0]));

        // Changed after the copy: the paste is the material as copied.
        app.project.apply(Command::SetMaterialParameter { material: source, parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.8) }).unwrap();
        app.project.end_interaction();
        let own = material(&app, parts[1]);
        app.part_material(Action::Paste { at: parts[1], linked: false });
        let brought = material(&app, parts[1]);
        let pasted = app.project.document().materials.get(brought).unwrap().clone();
        assert_eq!(MaterialDefinition { name: original.name.clone(), ..pasted.clone() }, original, "the paste took the change made after the copy");
        assert_eq!(pasted.name, original.name.clone() + " 2", "the copy brought in took the original's name");
        app.undo();
        assert_eq!(material(&app, parts[1]), own, "one undo did not take the whole paste back");
        assert!(app.project.document().materials.get(brought).is_none(), "undo left the copy brought in behind");

        // Linked: the original itself, shared.
        app.part_material(Action::Paste { at: parts[2], linked: true });
        assert_eq!(material(&app, parts[2]), source);

        // Another project: a copy, with the picture and the label it reads.
        app.project.start_new(tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.18));
        let plane = shape(&mut app, Shape::Plane);
        let own = material(&app, plane);
        let offer = app.material_offer();
        let targets = Targets::of(app.project.document(), vec![plane]);
        assert!(!paste_entry(app.project.document(), &offer, &targets, true).enabled, "Paste linked was offered in another project");
        assert!(paste_entry(app.project.document(), &offer, &targets, false).enabled);
        app.part_material(Action::Paste { at: plane, linked: false });
        let document = app.project.document();
        let brought = document.materials.get(material(&app, plane)).unwrap();
        assert_eq!(brought.name, original.name, "a name nobody uses here was changed");
        assert_eq!(brought.material(), original.material());
        assert_eq!(brought.substance, original.substance);
        assert_eq!(brought.graph.decals.len(), 1);
        let label = brought.graph.decals[0].texture;
        assert_eq!(document.textures.get(label).map(|s| s.path.clone()), Some(picture()), "the label's picture did not come with it");
        for id in pictures(brought) {
            assert!(document.textures.get(id).is_some(), "the copy reads a picture this project does not have");
        }
        app.undo();
        assert_eq!(material(&app, plane), own, "undo did not put the part's own material back");
        assert!(app.project.document().textures.get(label).is_none(), "undo left the picture brought in behind");
    }

    /// The menu's words: Paste greyed with the reason when nothing is
    /// copied, and naming what would go where when something is; Paste
    /// linked greyed in another project; locked parts left out and said.
    #[test]
    fn the_menu_says_what_would_be_pasted_and_onto_what() {
        let mut app = TequilaApp::for_tests();
        let parts = shapes(&mut app);
        let document = app.project.document();
        let one = Targets::of(document, vec![parts[1]]);
        let nothing = paste_entry(document, &app.material_offer(), &one, false);
        assert!(!nothing.enabled && nothing.hover.starts_with("Nothing copied yet"), "{nothing:?}");
        assert!(nothing.hover.contains(&crate::theme::keys("Alt + C")), "the disabled entry does not say the key: {nothing:?}");
        assert!(edit_entry(document, parts[1]).enabled && copy_entry(document, parts[1]).enabled);

        app.part_material(Action::Copy(parts[0]));
        let name = app.project.document().materials.get(material(&app, parts[0])).unwrap().name.clone();
        let offer = app.material_offer();
        assert_eq!(offer.paste_keys, crate::theme::keys("Alt + V"));
        let document = app.project.document();
        let paste = paste_entry(document, &offer, &one, false);
        assert!(paste.enabled);
        assert_eq!(paste.label, "Paste material");
        let part = document.tree.get(parts[1]).unwrap().name.clone();
        let source = document.tree.get(parts[0]).unwrap().name.clone();
        assert!(paste.hover.contains(&format!("`{name}`")) && paste.hover.contains(&format!("`{part}`")) && paste.hover.contains(&format!("`{source}`")), "{paste:?}");
        let several = Targets::of(document, vec![parts[1], parts[2]]);
        let paste = paste_entry(document, &offer, &several, false);
        assert_eq!(paste.label, "Paste material onto 2 parts");
        assert!(paste.hover.contains("the 2 selected parts"), "{paste:?}");
        assert!(paste_entry(document, &offer, &several, true).enabled);

        app.project.apply(Command::set_locked(parts[2], true)).unwrap();
        let document = app.project.document();
        let several = Targets::of(document, vec![parts[1], parts[2]]);
        assert_eq!((several.parts.clone(), several.locked), (vec![parts[1]], 1));
        assert!(paste_entry(document, &offer, &several, false).hover.contains("1 locked part keeps its own"));
        let locked = Targets::of(document, vec![parts[2]]);
        let refused = paste_entry(document, &offer, &locked, false);
        assert!(!refused.enabled && refused.hover.contains("locked"), "{refused:?}");
    }

    /// Edit material selects the part alone and opens the Material tab on
    /// it; the keys copy the selected part's material and paste it onto
    /// the selection.
    #[test]
    fn edit_material_opens_the_material_tab_and_the_keys_copy_and_paste() {
        let mut app = TequilaApp::for_tests();
        let parts = shapes(&mut app);
        app.project.selection.select_many([parts[0], parts[1]], SelectionMode::Replace);
        app.project_tab = crate::ProjectTab::Scene;
        app.show_project = false;
        app.part_material(Action::Edit(parts[1]));
        assert_eq!(app.project.selection.nodes().collect::<Vec<_>>(), vec![parts[1]]);
        assert_eq!(app.project_tab, crate::ProjectTab::Material);
        assert!(app.show_project);

        let ctx = egui::Context::default();
        let press = |app: &mut TequilaApp, key: egui::Key| {
            let modifiers = egui::Modifiers::ALT;
            let input = egui::RawInput { modifiers, events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }], ..Default::default() };
            let _ = ctx.run(input, |ctx| app.global_hotkeys(ctx));
        };
        app.project.selection.select_many([parts[0]], SelectionMode::Replace);
        press(&mut app, egui::Key::C);
        app.project.selection.select_many([parts[2]], SelectionMode::Replace);
        let before = material(&app, parts[2]);
        press(&mut app, egui::Key::V);
        let pasted = material(&app, parts[2]);
        assert_ne!(pasted, before, "Alt+V pasted nothing");
        assert_ne!(pasted, material(&app, parts[0]), "Alt+V linked rather than copied");
        let names = |key| app.project.document().materials.get(key).map(|d| d.material().clone());
        assert_eq!(names(pasted), names(material(&app, parts[0])));
    }
}
