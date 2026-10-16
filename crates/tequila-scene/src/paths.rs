//! The files outside a document that rendering it reads, found and renamed.
//!
//! A document holds descriptions, not pixels: texture maps, the
//! environment's image, its backplate and image pins (the scene's own and
//! each Studio's), and lights' IES profiles are paths to files that live
//! wherever the person keeps them. A document sent to another machine to
//! render (`tequila_network`) has to take those files with it, and point at
//! where they land there; this is the one place that knows where every path
//! is kept, so the two cannot drift apart as the document grows.

use crate::document::{EnvironmentSettings, SceneDocument};
use crate::tree::NodeKind;

fn environment(settings: &mut EnvironmentSettings, f: &mut dyn FnMut(&str) -> Option<String>) {
    for path in [&mut settings.image, &mut settings.backplate].into_iter().flatten() {
        if let Some(new) = f(path) {
            *path = new;
        }
    }
    for pin in &mut settings.pins {
        if let tequila_render_core::pins::PinKind::Image { path } = &mut pin.kind {
            if let Some(new) = f(path) {
                *path = new;
            }
        }
    }
}

/// Visit every external path the document holds, replacing each with what
/// `f` returns for it (`None` leaves it). Empty paths are not visited.
pub fn rewrite(document: &mut SceneDocument, f: &mut dyn FnMut(&str) -> Option<String>) {
    let mut visit = |path: &str| if path.is_empty() { None } else { f(path) };
    let textures: Vec<_> = document.textures.iter().map(|(id, _)| id).collect();
    for id in textures {
        if let Some(source) = document.textures.get_mut(id) {
            let current = source.path.to_string_lossy().into_owned();
            if let Some(new) = visit(&current) {
                source.path = new.into();
            }
        }
    }
    environment(&mut document.environment, &mut visit);
    for studio in &mut document.studios {
        if let Some(settings) = &mut studio.environment {
            environment(settings, &mut visit);
        }
    }
    let nodes: Vec<_> = document.tree.iter().map(|n| n.id).collect();
    for id in nodes {
        if let Ok(node) = document.tree.get_mut(id) {
            if let NodeKind::Light { settings } = &mut node.kind {
                if let Some(photometry) = &mut settings.photometry {
                    if let Some(new) = visit(&photometry.path) {
                        photometry.path = new;
                    }
                }
            }
        }
    }
}

/// Every external path the document holds, once each, in the order
/// [`rewrite`] visits them.
pub fn external(document: &SceneDocument) -> Vec<String> {
    // Visited on a copy: the walk is written once, for writing, and a
    // document's copy shares its meshes rather than duplicating them.
    let mut copy = document.clone();
    let mut out: Vec<String> = Vec::new();
    rewrite(&mut copy, &mut |path| {
        if !out.iter().any(|p| p == path) {
            out.push(path.to_string());
        }
        None
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::{LightSettings, Photometry};
    use crate::textures::TextureSource;
    use tequila_texture::texture::MapRole;

    fn document_with_files() -> SceneDocument {
        let mut document = SceneDocument::new();
        document.textures.intern(TextureSource::new("maps/wood.png", MapRole::Color));
        document.environment.image = Some("C:/hdri/studio.exr".into());
        document.environment.backplate = Some("plate.jpg".into());
        document.environment.pins.push(tequila_render_core::pins::Pin {
            kind: tequila_render_core::pins::PinKind::Image { path: "window.png".into() },
            ..tequila_render_core::pins::Pin::default()
        });
        let mut studio = crate::studio::Studio::capture("Outdoor", &SceneDocument::new());
        let mut other = EnvironmentSettings::default();
        other.image = Some("C:/hdri/outdoor.hdr".into());
        studio.environment = Some(other);
        document.studios.push(studio);
        let mut light = LightSettings::default();
        light.photometry = Some(Photometry { path: "lamp.ies".into() });
        let mut bus = crate::CommandBus::new();
        bus.apply(&mut document, crate::Command::AddLight { parent: None, name: "Lamp".into(), settings: Box::new(light) }).unwrap();
        document
    }

    #[test]
    fn every_kind_of_file_a_render_reads_is_found() {
        let found = external(&document_with_files());
        assert_eq!(found, ["maps/wood.png", "C:/hdri/studio.exr", "plate.jpg", "window.png", "C:/hdri/outdoor.hdr", "lamp.ies"]);
    }

    #[test]
    fn rewriting_reaches_every_one() {
        let mut document = document_with_files();
        rewrite(&mut document, &mut |path| Some(format!("job/{}", path.rsplit('/').next().unwrap())));
        let found = external(&document);
        assert!(found.iter().all(|p| p.starts_with("job/")), "{found:?}");
        assert_eq!(found.len(), 6);
    }
}
