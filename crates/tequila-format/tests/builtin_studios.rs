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
