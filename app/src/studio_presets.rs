//! Studio presets in the application (`docs/studio-presets.md`): the list,
//! applying one with the options the Library offers, and saving the scene's
//! studio as a preset - shared by the Library's Studios section, the
//! Studios tab and the scripting channel, so all three do the same thing.

use crate::project::Project;
use tequila_format::studio_preset::{self as presets, PresetFile};
use tequila_scene::preset::{ApplyOptions, PresetMaterial};
use tequila_scene::Command;

/// What applying did, to say.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    pub views: usize,
    pub lights: usize,
    pub cameras: usize,
    /// The product's size against the preset's.
    pub scale: f32,
    /// Parts dressed by the preset's rules, when asked.
    pub dressed: usize,
}

/// The preset of `id`, among those installed.
pub fn find(id: &str) -> Result<PresetFile, String> {
    let (all, _) = presets::list();
    all.into_iter().find(|p| p.preset.id == id).ok_or_else(|| format!("no studio preset `{id}` is installed"))
}

/// The product's bounding-sphere radius, metres, as presets are framed by;
/// `None` for a scene with nothing shown.
pub fn product_radius(project: &Project) -> Option<f32> {
    let bounds = tequila_scene::snapshot::visible_bounds(project.document());
    (!bounds.is_empty()).then(|| bounds.bounding_radius()).filter(|r| r.is_finite() && *r > 0.0)
}

/// Apply `file` to the project as one undo step - and, with `dress`, its
/// material rules too, in the same step.
pub fn apply(project: &mut Project, file: &PresetFile, options: ApplyOptions, dress: bool) -> Result<Applied, String> {
    let preset = &file.preset;
    let scale = match product_radius(project) {
        Some(radius) if options.scale.unwrap_or(preset.framing.scale) != tequila_scene::preset::FramingScale::None => {
            radius / preset.framing.reference_radius.max(1e-6)
        }
        _ => 1.0,
    };
    project.begin_group();
    if let Err(error) = project.apply(Command::ApplyStudioPreset { preset: Box::new(preset.clone()), options }) {
        project.abandon_group();
        return Err(error.to_string());
    }
    let mut dressed = 0;
    if dress {
        let name = format!("{} materials", preset.name);
        if let Some(template) = project.document().material_templates.iter().find(|t| t.name == name).cloned() {
            let plan = tequila_scene::templates::plan(project.document(), &template);
            dressed = plan.len();
            for command in tequila_scene::templates::commands(&plan) {
                if let Err(error) = project.apply(command) {
                    project.abandon_group();
                    return Err(error.to_string());
                }
            }
        }
    }
    project.end_group();
    project.last_studio = preset.studios.first().map(|s| s.filename_suffix.clone().unwrap_or_else(|| s.name.clone()));
    project.status = format!("Applied {}", preset.name);
    Ok(Applied { views: preset.studios.len(), lights: if options.keep_lights { 0 } else { preset.lights.len() }, cameras: preset.cameras.len(), scale, dressed })
}

/// A preset's picture from the viewport: the middle 4:3 of it, 320 × 240,
/// as a PNG. `None` before the viewport has drawn anything.
pub fn thumbnail_png(image: &crate::orchestrator::DisplayImage) -> Option<Vec<u8>> {
    const SIZE: (usize, usize) = (320, 240);
    let (w, h) = (image.width, image.height);
    if w < 8 || h < 8 || image.pixels.len() < w * h * 4 {
        return None;
    }
    let (cw, ch) = if w * 3 > h * 4 { (h * 4 / 3, h) } else { (w, w * 3 / 4) };
    let (x0, y0) = ((w - cw) / 2, (h - ch) / 2);
    let mut out = vec![0u8; SIZE.0 * SIZE.1 * 4];
    for y in 0..SIZE.1 {
        let (sy0, sy1) = (y0 + y * ch / SIZE.1, (y0 + (y + 1) * ch / SIZE.1).max(y0 + y * ch / SIZE.1 + 1));
        for x in 0..SIZE.0 {
            let (sx0, sx1) = (x0 + x * cw / SIZE.0, (x0 + (x + 1) * cw / SIZE.0).max(x0 + x * cw / SIZE.0 + 1));
            let mut sum = [0u32; 4];
            for sy in sy0..sy1.min(h) {
                for sx in sx0..sx1.min(w) {
                    let p = &image.pixels[(sy * w + sx) * 4..(sy * w + sx) * 4 + 4];
                    for c in 0..4 {
                        sum[c] += p[c] as u32;
                    }
                }
            }
            let n = ((sy1.min(h) - sy0) * (sx1.min(w) - sx0)).max(1) as u32;
            for c in 0..4 {
                out[(y * SIZE.0 + x) * 4 + c] = (sum[c] / n) as u8;
            }
        }
    }
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, SIZE.0 as u32, SIZE.1 as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().ok()?.write_image_data(&out).ok()?;
    Some(bytes)
}

/// The project's materials as a preset's library: each with the pictures
/// its graph and its labels read.
pub fn project_materials(project: &Project) -> Vec<PresetMaterial> {
    use tequila_materials::graph::NodeKind;
    let document = project.document();
    document
        .materials
        .iter()
        .map(|(_, definition)| {
            let mut ids = std::collections::BTreeSet::new();
            ids.extend(definition.graph.decals.iter().map(|decal| decal.texture));
            for (_, node) in definition.graph.nodes() {
                if let NodeKind::ImageTexture { texture, .. } = &node.kind {
                    ids.insert(*texture);
                }
            }
            PresetMaterial {
                definition: definition.clone(),
                textures: ids.into_iter().filter_map(|id| document.textures.get(id).map(|s| (id, s.clone()))).collect(),
                category: "My studios".into(),
                tags: Vec::new(),
            }
        })
        .collect()
}

/// What to save the scene's studio as.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveAs {
    pub name: String,
    pub family: String,
    pub size: String,
    pub description: String,
    /// Keep the project's materials with it, as its library.
    pub materials: bool,
}

/// Save the scene's studio as one of the person's presets - everything but
/// the product - framed for the product it has now.
pub fn save(project: &Project, what: &SaveAs, thumbnail: Option<&[u8]>) -> Result<PresetFile, String> {
    let name = what.name.trim();
    if name.is_empty() {
        return Err("a studio preset needs a name".into());
    }
    let id: String = presets::slug(name).chars().take(64).collect();
    let mut preset = tequila_scene::preset::capture(project.document(), &id, name, what.family.trim(), what.size.trim());
    preset.description = what.description.clone();
    if what.materials {
        preset.materials = project_materials(project);
    }
    presets::save(&preset, thumbnail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tequila_scene::tree::NodeKind;

    fn demo() -> Project {
        Project::with_document(
            tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.2),
            Arc::new(tequila_scene::demo::studio_environment()),
        )
    }

    fn worn(project: &Project) -> Vec<Option<String>> {
        let document = project.document();
        document
            .tree
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Part { .. }))
            .map(|n| n.material.and_then(|key| document.materials.get(key)).map(|m| m.name.clone()))
            .collect()
    }

    fn lights(project: &Project) -> usize {
        project.document().tree.iter().filter(|n| matches!(n.kind, NodeKind::Light { .. })).count()
    }

    /// Applied with its rules, a preset dresses the product in the same
    /// step as it brings the studio, and one undo takes both back.
    #[test]
    fn applying_and_dressing_is_one_undo_step() {
        let mut project = demo();
        let mut preset = tequila_scene::preset::capture(project.document(), "test", "Test", "Tests", "Small");
        let (_, definition) = project.document().materials.iter().next().expect("the demo has a material");
        let mut definition = definition.clone();
        definition.name = "Studio Red".into();
        preset.materials = vec![PresetMaterial { definition, textures: Vec::new(), category: "Tests".into(), tags: Vec::new() }];
        preset.material_rules = vec![tequila_scene::preset::MaterialRule { pattern: "*".into(), material: "Studio Red".into() }];
        let file = PresetFile {
            preset,
            path: std::path::PathBuf::from("test.studio.json"),
            thumbnail: None,
            source: presets::PresetSource::User,
            unknown: Default::default(),
        };
        let (before, lit) = (worn(&project), lights(&project));
        let applied = apply(&mut project, &file, ApplyOptions::default(), true).expect("applies");
        assert!(applied.dressed > 0, "{applied:?}");
        assert!(worn(&project).iter().all(|m| m.as_deref() == Some("Studio Red")), "{:?}", worn(&project));
        project.undo();
        assert_eq!(worn(&project), before, "one undo takes the dressing back with the studio");
        assert_eq!(lights(&project), lit);
        assert!(!project.can_undo(), "nothing else was left on the history");
    }

    /// The viewport picture, cropped to 4:3 and made small.
    #[test]
    fn a_thumbnail_is_the_middle_of_the_viewport() {
        let image = crate::orchestrator::DisplayImage {
            width: 64,
            height: 32,
            pixels: (0..64 * 32).flat_map(|i| if (i % 64) < 32 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }).collect(),
            ..Default::default()
        };
        let png = thumbnail_png(&image).expect("a picture");
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let reader = decoder.read_info().unwrap();
        assert_eq!((reader.info().width, reader.info().height), (320, 240));
        assert!(thumbnail_png(&crate::orchestrator::DisplayImage::default()).is_none());
    }
}
