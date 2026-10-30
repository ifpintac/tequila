//! Studio presets (`docs/studio-presets.md`): a complete studio - lights,
//! cameras and views, environment and floor, image style, render and
//! lighting settings, and a library of materials to dress a product with -
//! applied to any product in one step, and framed to its size.
//!
//! A preset is the studio only, never a product. Applying one
//! ([`crate::Command::ApplyStudioPreset`]) replaces the scene's studio and
//! leaves the product's geometry and materials as they are: it moves the
//! product's top nodes to stand where the studio expects it, and scales the
//! cameras to its size.
//!
//! Studio presets are not the environment's pin presets
//! (`tequila_render_core::pins::studio_presets`), which paint softboxes on
//! the environment and nothing else.

use crate::document::{EnvironmentSettings, SceneDocument};
use crate::lighting::{LightSettings, SavedCamera};
use crate::material::MaterialDefinition;
use crate::render::RenderSettings;
use crate::studio::Studio;
use crate::textures::TextureSource;
use crate::tree::NodeId;
use tequila_core::math::{vec3, Aabb, Mat4, Transform, Vec3};
use tequila_materials::graph::TextureId;
use tequila_render_core::settings::LightingSettings;

/// A studio, as a preset holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct StudioPreset {
    /// Stable, for files and scripts: `product-studio-small`.
    pub id: String,
    pub name: String,
    /// Presets of one family are shown together, their sizes side by side:
    /// "Product Studio".
    pub family: String,
    /// "Small", "Medium", "Large" - the product size it is framed for.
    pub size: String,
    pub description: String,
    pub tags: Vec<String>,
    /// The preset's own version, free text.
    pub version: String,
    pub environment: EnvironmentSettings,
    pub lighting: LightingSettings,
    /// The preset's render settings. Those that belong to a project on a
    /// machine - its output folder, render layers and region - are the
    /// project's own and are kept when the preset is applied.
    pub render: RenderSettings,
    pub image_style: tequila_color::ImageStyle,
    pub lights: Vec<PresetLight>,
    /// The cameras its views use, named as they name them.
    pub cameras: Vec<SavedCamera>,
    /// Its views: each a Studio with its camera, exposure, light switches
    /// and product turn. The first is the one applied.
    pub studios: Vec<Studio>,
    pub framing: Framing,
    /// Its material library: added to the project when the preset is
    /// applied, to assign, and never assigned by applying it.
    pub materials: Vec<PresetMaterial>,
    /// Rules a Material Template is made of, to dress a product with the
    /// preset's materials when asked: a part-name pattern and a material's
    /// name.
    pub material_rules: Vec<MaterialRule>,
    pub notes: String,
}

/// One of a preset's lights: its settings, and where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetLight {
    pub name: String,
    pub settings: LightSettings,
    /// World space, metres, row-major, as a node's transform.
    pub transform: Mat4,
}

/// One of a preset's materials, as a library preset is: its definition and
/// the pictures its graph reads, under ids of the preset's own.
#[derive(Clone, Debug, PartialEq)]
pub struct PresetMaterial {
    pub definition: MaterialDefinition,
    pub textures: Vec<(TextureId, TextureSource)>,
    /// Where it appears in the library once installed: "Product Studio".
    pub category: String,
    pub tags: Vec<String>,
}

/// A part-name pattern (`*` and `?`) and the material it gets.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialRule {
    pub pattern: String,
    pub material: String,
}

/// How a preset fits a product.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Framing {
    /// The bounding-sphere radius, metres, of the product the cameras were
    /// framed for: half its bounding box's diagonal.
    pub reference_radius: f32,
    /// How far above the floor the product's lowest point stands, metres.
    pub base_offset: f32,
    /// What follows the product's size.
    pub scale: FramingScale,
}

impl Default for Framing {
    fn default() -> Self {
        Framing { reference_radius: 0.1, base_offset: 0.0, scale: FramingScale::Cameras }
    }
}

/// What is scaled by the product's size against the preset's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramingScale {
    /// The cameras move in or out, positions and targets alike, so the
    /// product fills the frame as the preset's did.
    Cameras,
    /// The lights move with them.
    CamerasAndLights,
    /// Nothing: the preset as it was made.
    None,
}

impl FramingScale {
    pub const ALL: [FramingScale; 3] = [FramingScale::Cameras, FramingScale::CamerasAndLights, FramingScale::None];

    pub const fn key(self) -> &'static str {
        match self {
            FramingScale::Cameras => "cameras",
            FramingScale::CamerasAndLights => "cameras_and_lights",
            FramingScale::None => "none",
        }
    }

    pub fn from_key(key: &str) -> Option<FramingScale> {
        FramingScale::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// Which way the product's front faces as it arrives: CAD files come in any
/// orientation, and a studio's front is +Z.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Front {
    #[default]
    PlusZ,
    MinusZ,
    PlusX,
    MinusX,
}

impl Front {
    pub const ALL: [Front; 4] = [Front::PlusZ, Front::MinusZ, Front::PlusX, Front::MinusX];

    pub const fn key(self) -> &'static str {
        match self {
            Front::PlusZ => "+z",
            Front::MinusZ => "-z",
            Front::PlusX => "+x",
            Front::MinusX => "-x",
        }
    }

    pub fn from_key(key: &str) -> Option<Front> {
        let key = key.to_ascii_lowercase();
        Front::ALL.into_iter().find(|f| f.key() == key)
    }

    /// The turn about +Y that brings this front to +Z.
    pub fn yaw(self) -> f32 {
        use std::f32::consts::{FRAC_PI_2, PI};
        match self {
            Front::PlusZ => 0.0,
            Front::MinusZ => PI,
            Front::PlusX => -FRAC_PI_2,
            Front::MinusX => FRAC_PI_2,
        }
    }
}

/// How a preset is applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ApplyOptions {
    /// Which way the product faces as it is now.
    pub front: Front,
    /// What follows the product's size, when not what the preset says.
    pub scale: Option<FramingScale>,
    /// Keep the scene's lights and add none of the preset's.
    pub keep_lights: bool,
    /// Keep the scene's cameras as well as adding the preset's.
    pub keep_cameras: bool,
    /// Leave the product where it is.
    pub keep_placement: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        ApplyOptions { front: Front::PlusZ, scale: None, keep_lights: false, keep_cameras: false, keep_placement: false }
    }
}

/// Where the product goes and how much the cameras move, for `framing`.
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    /// The product's top nodes, placed.
    pub transforms: Vec<(NodeId, Transform)>,
    /// The product's size against the preset's: cameras, and with
    /// [`FramingScale::CamerasAndLights`] lights, move by it.
    pub scale: f32,
    /// The product's bounding-sphere radius, metres.
    pub radius: f32,
}

/// Fit `document`'s product to a studio framed as `framing` says: turned
/// so its front faces +Z, centred on the vertical axis, its lowest point
/// `base_offset` above the floor, and the cameras' scale its size against
/// the reference. Only the product's top nodes' transforms - never its
/// geometry.
pub fn fit(document: &SceneDocument, framing: &Framing, options: &ApplyOptions) -> Fit {
    let bounds = crate::snapshot::visible_bounds(document);
    let roots = crate::studio::product_roots(document);
    if bounds.is_empty() || roots.is_empty() {
        return Fit { transforms: Vec::new(), scale: 1.0, radius: framing.reference_radius };
    }
    let centre = bounds.center();
    let turn = Mat4::translation(centre) * Mat4::rotation(Vec3::Y, options.front.yaw()) * Mat4::translation(-centre);
    // A quarter turn keeps a box a box, so the turned bounds are exact.
    let turned = corners(&bounds).into_iter().fold(Aabb::EMPTY, |b, c| {
        let mut b = b;
        b.extend_point(turn.transform_point(c));
        b
    });
    let middle = turned.center();
    let shift = Mat4::translation(vec3(-middle.x, framing.base_offset - turned.min.y, -middle.z));
    let place = shift * turn;
    let transforms = if options.keep_placement {
        Vec::new()
    } else {
        roots
            .iter()
            .filter_map(|&id| {
                let node = document.tree.get(id)?;
                Transform::from_matrix(place * node.transform.matrix).map(|t| (id, t))
            })
            .collect()
    };
    let radius = turned.bounding_radius();
    let scale = match options.scale.unwrap_or(framing.scale) {
        FramingScale::None => 1.0,
        _ if framing.reference_radius > 0.0 && radius.is_finite() && radius > 0.0 => radius / framing.reference_radius,
        _ => 1.0,
    };
    Fit { transforms, scale, radius }
}

fn corners(b: &Aabb) -> [Vec3; 8] {
    let (lo, hi) = (b.min, b.max);
    [
        vec3(lo.x, lo.y, lo.z),
        vec3(hi.x, lo.y, lo.z),
        vec3(lo.x, hi.y, lo.z),
        vec3(hi.x, hi.y, lo.z),
        vec3(lo.x, lo.y, hi.z),
        vec3(hi.x, lo.y, hi.z),
        vec3(lo.x, hi.y, hi.z),
        vec3(hi.x, hi.y, hi.z),
    ]
}

/// A camera moved in or out by `s` about the studio's centre, as framing
/// for a product `s` times the preset's size does.
pub fn scaled_camera(camera: &SavedCamera, s: f32) -> SavedCamera {
    let mut camera = camera.clone();
    if (s - 1.0).abs() > 1e-7 {
        crate::units::scale_camera(&mut camera, s);
    }
    camera
}

/// The preset's views as the document's Studios, each camera scaled; each
/// as it was made otherwise, so a preset saved and applied again is the
/// same studio.
pub fn views(preset: &StudioPreset, scale: f32) -> Vec<Studio> {
    preset
        .studios
        .iter()
        .map(|studio| {
            let mut studio = studio.clone();
            studio.camera = studio.camera.map(|c| scaled_camera(&c, scale));
            studio
        })
        .collect()
}

/// The preset's render settings for `document`: the project keeps its
/// output folder, its render layers and its region, which name its own
/// parts and its own machine.
pub fn render_settings(preset: &StudioPreset, document: &SceneDocument) -> RenderSettings {
    RenderSettings {
        output_directory: document.render.output_directory.clone(),
        layers: document.render.layers.clone(),
        region: document.render.region,
        ..preset.render.clone()
    }
}

/// A light of the preset placed for a product `s` times its size: moved
/// with the cameras when [`FramingScale::CamerasAndLights`] says so.
pub fn light_transform(light: &PresetLight, s: f32, scale: FramingScale) -> Option<Transform> {
    let mut matrix = light.transform;
    if scale == FramingScale::CamerasAndLights && (s - 1.0).abs() > 1e-7 {
        for row in 0..3 {
            matrix.m[row][3] *= s;
        }
    }
    Transform::from_matrix(matrix)
}

/// The suggestion among `presets` of one family for a product of `radius`:
/// the size whose reference radius is nearest, by ratio. `None` for no
/// presets.
pub fn nearest_size<'a>(presets: impl IntoIterator<Item = &'a StudioPreset>, radius: f32) -> Option<&'a StudioPreset> {
    presets
        .into_iter()
        .filter(|p| p.framing.reference_radius > 0.0)
        .min_by(|a, b| {
            let off = |p: &StudioPreset| (radius / p.framing.reference_radius).ln().abs();
            off(a).total_cmp(&off(b))
        })
}

/// The Material Template a preset's rules become when it is applied.
pub fn template_name(preset: &StudioPreset) -> String {
    format!("{} materials", preset.name)
}

/// Once `preset` is applied: what dresses the product by its rules, through
/// the Material Template they became. Empty for a preset without rules.
pub fn dressing(document: &SceneDocument, preset: &StudioPreset) -> Vec<crate::Command> {
    let name = template_name(preset);
    match document.material_templates.iter().find(|t| t.name == name) {
        Some(template) => crate::templates::commands(&crate::templates::plan(document, template)),
        None => Vec::new(),
    }
}

/// The bounding-sphere radius, metres, presets frame a product by: half the
/// diagonal of what is shown. `None` with nothing shown.
pub fn product_radius(document: &SceneDocument) -> Option<f32> {
    let bounds = crate::snapshot::visible_bounds(document);
    (!bounds.is_empty()).then(|| bounds.bounding_radius()).filter(|r| r.is_finite() && *r > 0.0)
}

/// The document's materials as a preset's library, each with the pictures
/// its graph and its labels read, filed under `category`.
pub fn document_materials(document: &SceneDocument, category: &str) -> Vec<PresetMaterial> {
    use tequila_materials::graph::NodeKind;
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
                category: category.into(),
                tags: Vec::new(),
            }
        })
        .collect()
}

/// A light's linking as a preset can keep it, without the parts of a
/// product it will not be applied to: a light kept to some of the
/// product's parts is kept to the product, and one leaving parts out
/// keeps only the product it leaves out, or lights everything.
fn portable_links(mut settings: LightSettings) -> LightSettings {
    if let Some(links) = &mut settings.links {
        if !links.nodes.is_empty() {
            links.nodes.clear();
            if !links.exclude {
                links.product = true;
            }
        }
        if links.exclude && !links.product {
            settings.links = None;
        }
    }
    settings
}

/// A Studio preset made from `document` as it is: everything but the
/// product, framed for its product. Materials and rules are the caller's
/// to add.
pub fn capture(document: &SceneDocument, id: &str, name: &str, family: &str, size: &str) -> StudioPreset {
    let bounds = crate::snapshot::visible_bounds(document);
    let (radius, base) =
        if bounds.is_empty() { (0.1, 0.0) } else { (bounds.bounding_radius(), bounds.min.y.max(0.0)) };
    let lights = document
        .tree
        .iter()
        .filter_map(|node| match &node.kind {
            crate::tree::NodeKind::Light { settings } => Some(PresetLight {
                name: node.name.clone(),
                settings: portable_links((**settings).clone()),
                transform: document.placements().world(node.id).matrix,
            }),
            _ => None,
        })
        .collect();
    StudioPreset {
        id: id.into(),
        name: name.into(),
        family: family.into(),
        size: size.into(),
        description: String::new(),
        tags: Vec::new(),
        version: "1".into(),
        environment: document.environment.clone(),
        lighting: document.lighting.clone(),
        render: RenderSettings {
            output_directory: String::new(),
            layers: Vec::new(),
            region: None,
            ..document.render.clone()
        },
        image_style: document.image_style,
        lights,
        cameras: document.cameras.iter().map(|(_, c)| c.clone()).collect(),
        studios: document
            .studios
            .iter()
            .map(|s| Studio { model_set: None, variants: Vec::new(), ..s.clone() })
            .collect(),
        framing: Framing { reference_radius: radius, base_offset: base, scale: FramingScale::Cameras },
        materials: Vec::new(),
        material_rules: Vec::new(),
        notes: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A product twice the size gets cameras twice as far, centred on the
    /// vertical axis, its base at the offset; a front of +X is turned to +Z.
    #[test]
    fn framing_centres_the_product_and_scales_by_its_size() {
        let document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.2);
        let bounds = crate::snapshot::visible_bounds(&document);
        let framing = Framing { reference_radius: bounds.bounding_radius() / 2.0, base_offset: 0.0003, scale: FramingScale::Cameras };
        let fit = fit(&document, &framing, &ApplyOptions::default());
        assert!((fit.scale - 2.0).abs() < 1e-4, "{}", fit.scale);
        let mut placed = document.clone();
        for (id, t) in &fit.transforms {
            placed.tree.get_mut(*id).unwrap().transform = *t;
        }
        let after = crate::snapshot::visible_bounds(&placed);
        assert!(after.center().x.abs() < 1e-5 && after.center().z.abs() < 1e-5, "centred: {:?}", after.center());
        assert!((after.min.y - 0.0003).abs() < 1e-5, "its base at the offset: {}", after.min.y);
        let none = super::fit(&document, &framing, &ApplyOptions { scale: Some(FramingScale::None), ..ApplyOptions::default() });
        assert_eq!(none.scale, 1.0);
        assert!((Front::PlusX.yaw() + std::f32::consts::FRAC_PI_2).abs() < 1e-6);
        let turned = Mat4::rotation(Vec3::Y, Front::PlusX.yaw()).transform_vector(Vec3::X);
        assert!((turned - Vec3::Z).length() < 1e-5, "+X faces +Z once turned: {turned:?}");
    }

    /// A preset keeps a light's linking without the parts of the product
    /// it came from: kept to some parts, it is kept to the product; leaving
    /// parts out but not the product, it lights everything.
    #[test]
    fn a_presets_lights_keep_links_a_product_can_take() {
        use crate::lighting::LightLinking;
        let with = |links: LightLinking| portable_links(LightSettings { links: Some(links), ..LightSettings::default() }).links;
        let part = crate::tree::NodeId(7);
        assert_eq!(with(LightLinking { nodes: vec![part], ..LightLinking::default() }), Some(LightLinking { product: true, ..LightLinking::default() }));
        assert_eq!(with(LightLinking { nodes: vec![part], exclude: true, ..LightLinking::default() }), None);
        let product_only = LightLinking { product: true, ..LightLinking::default() };
        assert_eq!(with(product_only.clone()), Some(product_only));
    }

    /// Of three sizes, the one framed nearest a product's size by ratio.
    #[test]
    fn the_nearest_size_is_suggested() {
        let sized = |size: &str, radius: f32| StudioPreset {
            size: size.into(),
            framing: Framing { reference_radius: radius, ..Framing::default() },
            ..capture(&SceneDocument::new(), "x", "x", "Product Studio", size)
        };
        let all = [sized("Small", 0.09), sized("Medium", 0.25), sized("Large", 0.7)];
        assert_eq!(nearest_size(&all, 0.1).map(|p| p.size.as_str()), Some("Small"));
        assert_eq!(nearest_size(&all, 0.3).map(|p| p.size.as_str()), Some("Medium"));
        assert_eq!(nearest_size(&all, 2.0).map(|p| p.size.as_str()), Some("Large"));
    }
}
