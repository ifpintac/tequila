//! A downloaded PBR texture pack, from a folder or a ZIP, becomes a material.

use tequila_import::texture_set::{import_sets, SetOptions, Slot};
use tequila_materials::material_type::Param;
use tequila_scene::{CommandBus, SceneDocument};
use std::path::{Path, PathBuf};

fn write_png(path: &Path, size: u32, rgb: [u8; 3]) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), size, size);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = (0..size * size).flat_map(|_| rgb).collect();
    encoder.write_header().unwrap().write_image_data(&data).unwrap();
}

/// A pack as ambientCG ships one: two resolutions of the colour, and a
/// normal map in DirectX's convention only.
fn pack(directory: &Path) -> Vec<PathBuf> {
    std::fs::create_dir_all(directory).unwrap();
    let files = [
        ("Tiles012_1K-PNG_Color.png", 4, [200, 40, 40]),
        ("Tiles012_2K-PNG_Color.png", 8, [220, 30, 30]),
        ("Tiles012_2K-PNG_Roughness.png", 8, [128, 128, 128]),
        // Green at a quarter: pointing down in DirectX, so up in OpenGL.
        ("Tiles012_2K-PNG_NormalDX.png", 8, [128, 64, 255]),
        ("Tiles012.png", 8, [0, 0, 0]),
    ];
    files
        .iter()
        .map(|(name, size, rgb)| {
            let path = directory.join(name);
            write_png(&path, *size, *rgb);
            path
        })
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("tequila-texture-set-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    directory
}

fn check(document: &SceneDocument, report: &tequila_import::texture_set::SetReport) {
    assert_eq!(report.materials.len(), 1);
    let (key, name) = &report.materials[0];
    assert_eq!(name, "Tiles012");
    let definition = document.materials.get(*key).expect("the material is in the document");
    for param in [Param::BaseColor, Param::SpecularRoughness, Param::Bump] {
        assert!(definition.graph.driver_of(param).is_some(), "{param:?} is not driven");
    }
    let maps: Vec<(Slot, String)> = report.maps[0].clone();
    assert!(maps.contains(&(Slot::BaseColor, "Tiles012_2K-PNG_Color.png".into())), "{maps:?}");
    // The converted normal map is the one in use, and it points the right way.
    let (_, normal) = maps.iter().find(|(s, _)| matches!(s, Slot::Normal { .. })).expect("a normal map");
    assert!(normal.ends_with("_opengl.png"), "{normal}");
    let source = document
        .textures
        .iter()
        .find(|(_, t)| t.path.to_string_lossy().ends_with("_opengl.png"))
        .map(|(_, t)| t.path.clone())
        .expect("the document refers to the converted map");
    let image = tequila_texture::image::Image::load_raw(&source).expect("the converted map decodes");
    let green = image.data[1];
    let red = image.data[0];
    assert!((red - 128.0 / 255.0).abs() < 0.01, "red was {red}, not kept");
    assert!((green - (1.0 - 64.0 / 255.0)).abs() < 0.01, "green was {green}, not flipped");
    assert!(report.notes.iter().any(|n| n.contains("DirectX")), "{:?}", report.notes);
    assert!(report.notes.iter().any(|n| n.contains("Tiles012.png")), "the stray file is not reported: {:?}", report.notes);
    document.validate().expect("the document holds together");
}

#[test]
fn a_folder_of_maps_becomes_one_material() {
    let directory = scratch("folder");
    pack(&directory);
    let mut document = SceneDocument::new();
    let (command, report) = import_sets(&document, &directory, SetOptions::default()).expect("imports");
    CommandBus::new().apply(&mut document, command).expect("applies");
    check(&document, &report);
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_zip_is_unpacked_beside_itself_and_read_the_same() {
    let directory = scratch("zip");
    let source = directory.join("source");
    let files = pack(&source);
    let mut zip = tequila_format::zip::ZipWriter::new();
    for path in &files {
        zip.add(format!("Tiles012/{}", path.file_name().unwrap().to_string_lossy()), std::fs::read(path).unwrap());
    }
    let archive = directory.join("Tiles012_2K-PNG.zip");
    std::fs::write(&archive, zip.finish()).unwrap();

    let mut document = SceneDocument::new();
    let (command, report) = import_sets(&document, &archive, SetOptions::default()).expect("imports");
    CommandBus::new().apply(&mut document, command).expect("applies");
    check(&document, &report);
    assert!(directory.join("Tiles012_2K-PNG").join("Tiles012_2K-PNG_Color.png").exists(), "unpacked beside the ZIP");
    let _ = std::fs::remove_dir_all(&directory);
}

#[test]
fn a_folder_with_no_maps_says_what_it_looks_for() {
    let directory = scratch("empty");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("readme.txt"), "hello").unwrap();
    let error = import_sets(&SceneDocument::new(), &directory, SetOptions::default()).unwrap_err();
    assert!(error.to_string().contains("no texture maps"), "{error}");
    let _ = std::fs::remove_dir_all(&directory);
}
