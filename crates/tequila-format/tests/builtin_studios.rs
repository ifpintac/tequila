//! The studio presets that ship in `assets/studios` read without a problem,
//! carry what the Library shows, and apply to a product in one undo step
//! (`examples/builtin_studios.rs` writes them).

use std::path::Path;
use tequila_format::studio_preset::{list_in, PresetSource};
use tequila_scene::{Command, CommandBus};

#[test]
fn every_built_in_studio_reads_and_applies_in_one_step() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/studios");
    let (presets, problems) = list_in(&[(folder, PresetSource::Builtin)]);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(!presets.is_empty(), "no built-in studios");
    for file in presets {
        let preset = &file.preset;
        assert!(file.thumbnail.is_some(), "{} has no thumbnail", preset.id);
        assert!(!preset.family.is_empty() && !preset.size.is_empty() && !preset.studios.is_empty(), "{}", preset.id);
        let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
        let before = document.clone();
        let mut bus = CommandBus::new();
        bus.apply(&mut document, Command::ApplyStudioPreset { preset: Box::new(preset.clone()), options: Default::default() })
            .unwrap_or_else(|e| panic!("{}: {e}", preset.id));
        assert_eq!(document.studios.len(), preset.studios.len());
        assert_eq!(document.render.filename_template, preset.render.filename_template);
        bus.undo(&mut document).unwrap();
        assert_eq!(document.studios, before.studios, "{}: one undo takes it back", preset.id);
        assert_eq!(document.environment, before.environment);
    }
}

/// Every built-in studio lights and frames a product ten times the size as
/// it does a small one: its lights stand as many of the product's radii
/// off, are as many radii across, and its camera sees it from as many
/// radii away, so a mixer is lit and framed as a pedal is.
#[test]
fn every_built_in_studio_lights_a_large_product_as_a_small_one() {
    use tequila_core::math::{vec3, Mat4, Transform, Vec3};
    use tequila_scene::tree::NodeKind;
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/studios");
    let (presets, _) = list_in(&[(folder, PresetSource::Builtin)]);
    for file in presets {
        let preset = &file.preset;
        let studio = |size: f32| {
            let mut document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
            let mut bus = CommandBus::new();
            let transforms = tequila_scene::studio::product_roots(&document)
                .iter()
                .filter_map(|&id| {
                    let node = document.tree.get(id)?;
                    Transform::from_matrix(Mat4::scale(vec3(size, size, size)) * node.transform.matrix).map(|t| (id, t))
                })
                .collect();
            bus.apply(&mut document, Command::SetTransform { transforms }).unwrap();
            bus.apply(&mut document, Command::ApplyStudioPreset { preset: Box::new(preset.clone()), options: Default::default() })
                .unwrap_or_else(|e| panic!("{}: {e}", preset.id));
            let bounds = tequila_scene::snapshot::visible_bounds(&document);
            let (middle, radius) = (bounds.center(), bounds.bounding_radius());
            let placed = document.placements();
            let mut seen: Vec<f32> = Vec::new();
            for node in document.tree.iter() {
                if let NodeKind::Light { settings } = &node.kind {
                    let world = placed.world(node.id);
                    let off = (world.point(Vec3::ZERO) - middle) * (1.0 / radius);
                    let extent = settings.emitter_extent();
                    let across = world.vector(vec3(extent.x, extent.y, 0.0)).length() / radius;
                    seen.extend([off.x, off.y, off.z, across]);
                }
            }
            let eye = (document.cameras.active().camera.position() - middle) * (1.0 / radius);
            seen.extend([eye.x, eye.y, eye.z]);
            seen
        };
        let (small, large) = (studio(0.3), studio(3.0));
        assert_eq!(small.len(), large.len(), "{}", preset.id);
        for (a, b) in small.iter().zip(&large) {
            assert!((a - b).abs() < 1e-3 * a.abs().max(1.0), "{}: {small:?} against {large:?}", preset.id);
        }
    }
}
