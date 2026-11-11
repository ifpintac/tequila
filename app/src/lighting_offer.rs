//! The question a Blender file with lighting of its own asks once its model
//! is in: light it as Blender does, or keep the lighting the scene has.
//!
//! Asked after the model is in rather than before, so the import is never
//! held up and the model is there to look at while the person decides; the
//! lighting is a step of its own, which Undo takes back alone. What the
//! file has comes as `tequila_import::SceneLighting`, and choosing it is
//! its `commands`: the lights in a group under the model, the world as the
//! environment, the file's exposure and view transform on the look. The
//! answer can be kept for every Blender file from then on
//! (`import_settings::BlenderLights`, File › Lights in Blender files).

use eframe::egui;

/// A question waiting for its answer.
pub struct LightingOffer {
    /// The file's name, as the question says it.
    pub file: String,
    pub lighting: Box<tequila_import::SceneLighting>,
    /// The project it was asked about: a project closed meanwhile takes the
    /// question with it.
    pub document_epoch: u64,
    remember: bool,
}

/// What the person chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Light the model as the file does.
    File { remember: bool },
    /// Keep the scene's lighting.
    Keep { remember: bool },
}

impl LightingOffer {
    pub fn new(file: String, lighting: Box<tequila_import::SceneLighting>, document_epoch: u64) -> LightingOffer {
        LightingOffer { file, lighting, document_epoch, remember: false }
    }

    /// The window. `None` while it waits; closing it keeps the lighting as
    /// it is, this once.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Answer> {
        let mut open = true;
        let mut answer = None;
        egui::Window::new("Lighting from Blender")
            .id(egui::Id::new("blender-lighting-offer"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 64.0))
            .show(ctx, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.label(format!(
                    "{} has {}. Light the model with them, so it looks as it does in Blender, or keep the lighting you have?",
                    self.file,
                    self.lighting.summary()
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    // The main choice, as the update prompt marks its own.
                    let p = crate::theme::p();
                    let primary = egui::Button::new(egui::RichText::new("Use Blender's lighting").color(p.on_accent)).fill(p.accent);
                    if ui.add(primary).clicked() {
                        answer = Some(Answer::File { remember: self.remember });
                    }
                    if ui.button("Keep Tequila's").clicked() {
                        answer = Some(Answer::Keep { remember: self.remember });
                    }
                });
                ui.add_space(6.0);
                ui.checkbox(&mut self.remember, "Do the same for every Blender file");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Blender's lights go in a group of their own under the model, its world becomes \
                         the environment (without Tequila's ground, since the scene has its own floor), \
                         and its exposure and view transform come with it. Undo takes it back. Change \
                         this later in File › Lights in Blender files.",
                    )
                    .small()
                    .color(crate::theme::dim()),
                );
            });
        if !open && answer.is_none() {
            answer = Some(Answer::Keep { remember: false });
        }
        answer
    }
}
