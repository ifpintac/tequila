//! One example of every command, as a script sends it.
//!
//! The same list the journal's round-trip test runs, so every example here
//! is one the decoder is known to accept and to read back as what was meant.
//! The scripting API hands it out: forty-four of the forty-nine commands had
//! no example at all, and a script, or the built-in Assistant, had to guess
//! their fields.

#![allow(unused_imports)]

use crate::json::{self, Value};
use crate::json_write::{self, Object};
use crate::{fsmesh, FormatError};
use tequila_core::math::{Mat4, Transform};
use tequila_core::spectrum::{spectrum, Spectrum};
use tequila_materials::fresnel::ComplexIor;
use tequila_materials::material_type::{MaterialType, Param, ParamValue};
use crate::read_spectrum;
use tequila_render_core::material::Material;
use tequila_scene::command::{
    Command, EnvParam, EnvValue, EnvironmentParameter, ImportPayload, MaterialParameter,
};
use tequila_scene::geometry::Mesh;
use tequila_scene::textures::TextureSource;
use tequila_texture::image::ColorSpace;
use tequila_texture::texture::MapRole;
use tequila_scene::tree::{MaterialKey, MeshKey, NodeId, NodeKind, SceneNode};
use tequila_scene::units::ConversionMode;
use tequila_scene::{CommandBus, SceneDocument};
use std::io::Write;
use std::path::{Path, PathBuf};
use tequila_render_core::settings::LightingPreset;
use tequila_core::math::{vec3, Vec3};
use tequila_core::units::LengthUnit;
use tequila_scene::command::reserve_keys;
use tequila_scene::material::MaterialDefinition;
use crate::journal::{decode, encode};

/// A small document the examples refer to: an assembly of two parts sharing
/// one material.
pub(crate) fn imported() -> (SceneDocument, CommandBus, Vec<NodeId>, MaterialKey) {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let mesh = Mesh::new(
        vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)],
        vec![0, 1, 2],
    );
    let (meshes, materials, ids) = reserve_keys(&document, 1, 1, 3);

    let mut root = SceneNode::new(ids[0], "Assembly", NodeKind::Group);
    root.children = vec![ids[1], ids[2]];
    let mut nodes = vec![root];
    for (i, name) in [(1usize, "Housing"), (2, "Bracket")] {
        let mut node = SceneNode::new(ids[i], name, NodeKind::Part { mesh: meshes[0] });
        node.parent = Some(ids[0]);
        node.material = Some(materials[0]);
        node.metadata.insert("PartNumber".into(), format!("P-{i}"));
        // The bracket is on a CAD layer, so an example that names the layer
        // applies here.
        if i == 2 {
            node.layers.push("Construction".into());
        }
        nodes.push(node);
    }

    bus.apply(
        &mut document,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(meshes[0], mesh)],
                textures: Vec::new(),
                materials: vec![(materials[0], MaterialDefinition::plastic("Shell"))],
                nodes,
                roots: vec![ids[0]],
            }),
        },
    )
    .unwrap();
    (document, bus, ids, materials[0])
}

/// One command of every kind, at least, with values that are not defaults.
pub fn commands() -> Vec<Command> {
    let (document, _, ids, material) = imported();
    let mesh = document.geometry.keys().next().unwrap();
    vec![
        Command::AddDecal { material, name: "Logo".into(), source: TextureSource::new("logo.png", MapRole::Color) },
        Command::SetDecals { material, decals: vec![tequila_materials::decal::Decal::new("Logo", tequila_materials::graph::TextureId(1))] },
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(mesh, document.geometry.get(mesh).unwrap().clone())],
                // A picture, so a journal that drops them is caught.
                textures: vec![(tequila_materials::graph::TextureId(7), TextureSource::new("maps/Leather037_Color.jpg", MapRole::Color))],
                materials: vec![(material, MaterialDefinition::plastic("Shell"))],
                nodes: vec![SceneNode::new(NodeId(50), "X", NodeKind::Group)],
                roots: vec![NodeId(50)],
            }),
        },
        Command::Reimport {
            payload: Box::new(tequila_scene::reimport::ReimportPayload {
                added: ImportPayload {
                    meshes: vec![(mesh, document.geometry.get(mesh).unwrap().clone())],
                    textures: Vec::new(),
                    materials: Vec::new(),
                    nodes: vec![{
                        let mut lid = SceneNode::new(NodeId(51), "Lid", NodeKind::Part { mesh });
                        lid.parent = Some(ids[0]);
                        lid.path_key = Some("Bracket#1/Lid#1".into());
                        lid.layers = vec!["Outer".into()];
                        lid
                    }],
                    roots: vec![NodeId(51)],
                },
                updates: vec![
                    tequila_scene::reimport::NodeUpdate {
                        node: ids[1],
                        kind: NodeKind::Part { mesh },
                        transform: Some(tequila_core::math::Transform::translation(tequila_core::math::vec3(0.0, 0.5, 0.0))),
                        material: Some(material),
                        metadata: [("Revision".to_string(), "B".to_string())].into_iter().collect(),
                        path_key: Some("Bracket#1/Housing#1".into()),
                        layers: vec!["Outer".into(), "Shell, inner".into()],
                    },
                    tequila_scene::reimport::NodeUpdate {
                        node: ids[0],
                        kind: NodeKind::Group,
                        transform: None,
                        material: None,
                        metadata: Default::default(),
                        path_key: Some("Bracket#1".into()),
                        layers: Vec::new(),
                    },
                ],
                hidden: vec![ids[2]],
            }),
        },
        Command::AddGroup {
            parent: Some(ids[0]),
            name: "Sub".into(),
        },
        Command::AddGroup {
            parent: None,
            name: "Top".into(),
        },
        // The six that were missing from the catalog, and so from here.
        // Non-default values wherever a default would round-trip by
        // accident.
        Command::SplitIntoParts { node: ids[1] },
        Command::SetParameterProcedural {
            material,
            param: Param::BaseColor,
            procedural: Some(Box::default()),
            seed: 7,
        },
        Command::SetParameterProcedural {
            material,
            param: Param::SpecularRoughness,
            procedural: None,
            seed: 0,
        },
        Command::SetParameterGradient {
            material,
            param: Param::BaseColor,
            gradient: Box::default(),
        },
        Command::SetTextureMapping {
            material,
            param: Param::BaseColor,
            mapping: Box::new(tequila_texture::projection::Mapping {
                rotation: 0.5,
                flip_u: true,
                uv_set: 1,
                ..Default::default()
            }),
        },
        Command::SetSky {
            sky: Box::new(Some(tequila_render_core::sky::Sky::default())),
        },
        Command::SetSky { sky: Box::new(None) },
        Command::SetEnvironmentPins {
            pins: vec![
                tequila_render_core::pins::strip("Top strip", 30.0, 60.0, 4.0),
                tequila_render_core::pins::Pin::image(
                    "Window",
                    "window.png",
                    tequila_render_core::pins::direction_from(0.5, 0.2),
                    0.6,
                ),
                tequila_render_core::pins::Pin::backdrop_gradient(
                    tequila_core::spectrum::Spectrum::splat(0.1),
                    tequila_core::spectrum::Spectrum::splat(0.7),
                ),
            ],
        },
        Command::SetEnvironmentPins { pins: Vec::new() },
        Command::AddCutaway {
            parent: Some(ids[0]),
            name: "Section".into(),
            settings: Box::new(tequila_scene::cutaway::CutawaySettings {
                shape: tequila_scene::cutaway::CutShape::Cylinder,
                cap: tequila_scene::cutaway::CapStyle::Color(tequila_core::spectrum::Spectrum {
                    r: 0.9,
                    g: 0.2,
                    b: 0.1,
                }),
                enabled: false,
            }),
            transform: Transform::translation(vec3(0.5, -1.0, 2.0)),
        },
        Command::SetCutaway {
            node: ids[1],
            settings: Box::default(),
        },
        Command::SetMultiMaterials {
            multi_materials: vec![tequila_scene::studio::MultiMaterial {
                id: 7,
                name: "Colourways".into(),
                variants: vec![material],
                active: 0,
                parts: vec![ids[1]],
            }],
        },
        Command::SetModelSets {
            model_sets: Box::new(tequila_scene::studio::ModelSets {
                sets: vec![tequila_scene::studio::ModelSet {
                    name: "Chassis A".into(),
                    nodes: vec![ids[0]],
                    always_visible: true,
                }],
                active: Some(0),
            }),
        },
        Command::SetStudios {
            studios: vec![tequila_scene::studio::Studio {
                name: "Hero".into(),
                camera: None,
                environment: Some(tequila_scene::document::EnvironmentSettings {
                    rotation: 1.25,
                    ..Default::default()
                }),
                image_style: None,
                model_set: Some(None),
                variants: vec![(7, 0)],
                lights: vec![
                    tequila_scene::studio::LightOverride {
                        target: tequila_scene::studio::LightTarget::Name("Rim".into()),
                        enabled: false,
                    },
                    tequila_scene::studio::LightOverride { target: tequila_scene::studio::LightTarget::Layer(1), enabled: true },
                ],
                product_yaw: Some(-0.942478),
                filename_suffix: Some("Right".into()),
            }],
        },
        Command::ApplyStudio { index: 0 },
        Command::TurnProduct { yaw: 0.5 },
        Command::SetAnimation { animation: Some(Box::new(tequila_scene::animation::Animation::turntable(36, 24.0, 360.0))) },
        Command::ApplyStudioPreset {
            preset: Box::new({
                let mut preset = tequila_scene::preset::capture(&tequila_scene::SceneDocument::new(), "sample-studio", "Sample Studio", "Samples", "Small");
                preset.lights.push(tequila_scene::preset::PresetLight {
                    name: "Key".into(),
                    settings: tequila_scene::lighting::LightSettings::default(),
                    transform: tequila_core::math::Mat4::translation(tequila_core::math::vec3(0.0, 0.5, 0.6)),
                });
                preset.material_rules.push(tequila_scene::preset::MaterialRule { pattern: "Metal; Screws*".into(), material: "Steel".into() });
                preset
            }),
            options: tequila_scene::preset::ApplyOptions { front: tequila_scene::preset::Front::MinusX, keep_cameras: true, ..Default::default() },
        },
        Command::SetConfigurator {
            configurator: Box::new(tequila_scene::configurator::Configurator {
                title: "Kettle".into(),
                subtitle: "Pick a finish".into(),
                placement: tequila_scene::configurator::Placement::Bottom,
                options: vec![
                    tequila_scene::configurator::ConfigOption {
                        id: 1,
                        name: "Body".into(),
                        drives: tequila_scene::configurator::Drives::Material { multi_material: 7 },
                        choices: vec![
                            tequila_scene::configurator::Choice { label: "Red".into(), target: 0, swatch: Some([0.5, 0.05, 0.04]) },
                            tequila_scene::configurator::Choice { label: "Cream".into(), target: 1, swatch: None },
                        ],
                        style: tequila_scene::configurator::Style::Swatches,
                    },
                    tequila_scene::configurator::ConfigOption {
                        id: 2,
                        name: "View".into(),
                        drives: tequila_scene::configurator::Drives::Studio,
                        choices: vec![tequila_scene::configurator::Choice { label: "Hero".into(), target: 0, swatch: None }],
                        style: tequila_scene::configurator::Style::Buttons,
                    },
                ],
                rules: vec![tequila_scene::configurator::Rule {
                    option: 1,
                    choice: 1,
                    kind: tequila_scene::configurator::RuleKind::Excludes,
                    other: 2,
                    other_choice: 0,
                }],
                current: vec![0, 0],
            }),
        },
        Command::Configure { choices: vec![0, 0] },
        Command::SetMaterialTemplates {
            templates: vec![tequila_scene::templates::MaterialTemplate {
                name: "Fasteners".into(),
                rules: vec![
                    tequila_scene::templates::Rule {
                        condition: tequila_scene::templates::Condition::Name("bolt*".into()),
                        material,
                    },
                    tequila_scene::templates::Rule {
                        condition: tequila_scene::templates::Condition::Colour {
                            color: tequila_core::spectrum::Spectrum::splat(0.25),
                            tolerance: 0.05,
                        },
                        material,
                    },
                    tequila_scene::templates::Rule {
                        condition: tequila_scene::templates::Condition::Metadata {
                            key: "layer".into(),
                            value: "HARDWARE".into(),
                        },
                        material,
                    },
                    tequila_scene::templates::Rule {
                        condition: tequila_scene::templates::Condition::Material("Steel*".into()),
                        material,
                    },
                ],
            }],
        },
        Command::SetMeasurements {
            measurements: vec![tequila_scene::measure::Measurement {
                a: tequila_scene::measure::Anchor {
                    node: Some(ids[1]),
                    point: vec3(0.25, -1.5, 3.0),
                },
                b: tequila_scene::measure::Anchor {
                    node: None,
                    point: vec3(-2.0, 0.5, 0.125),
                },
                visible: false,
            }],
        },
        Command::SetGraph {
            material,
            graph: Box::new({
                use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, Value as GraphValue};
                let mut graph = MaterialGraph::from_material(MaterialDefinition::plastic("P").material().clone());
                let ramp = graph.add_node(Node::new(NodeKind::Constant(GraphValue::Float(0.35))));
                let output = graph.output();
                graph.connect(ramp, output, Socket::Param(Param::SpecularRoughness));
                graph
            }),
        },
        Command::DeleteNodes {
            nodes: vec![ids[1], ids[2]],
        },
        Command::Rename {
            node: ids[1],
            name: "Renamed".into(),
        },
        Command::set_transform(ids[1], Transform::translation(vec3(1.5, -2.25, 0.125))),
        Command::set_visibility(ids[1], false),
        Command::SetLayerVisibility { layer: "Construction".into(), visible: false },
        Command::set_locked(ids[1], true),
        Command::set_rays(ids[1], tequila_scene::tree::Rays { glossy: false, holdout: true, ..Default::default() }),
        Command::Reparent {
            node: ids[1],
            parent: Some(ids[0]),
            index: Some(3),
        },
        Command::Reparent {
            node: ids[1],
            parent: None,
            index: None,
        },
        Command::set_pivot(ids[1], Some(tequila_core::math::vec3(0.5, -1.25, 2.0))),
        // Clearing is a different record from setting: an absent point
        // means "no pivot", not "a pivot at the origin".
        Command::set_pivot(ids[1], None),
        Command::Group {
            nodes: vec![ids[1], ids[2]],
            name: "Fasteners".into(),
        },
        Command::Ungroup { group: ids[0] },
        Command::SetMetadata {
            node: ids[1],
            key: "Mass_g".into(),
            value: Some("42.1".into()),
        },
        Command::SetMetadata {
            node: ids[1],
            key: "Mass_g".into(),
            value: None,
        },
        Command::SetUnits {
            unit: LengthUnit::Inch,
            mode: ConversionMode::ScaleScene,
        },
        Command::AssignMaterial {
            nodes: vec![ids[1]],
            material,
        },
        Command::UnlinkMaterial { node: ids[1] },
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::color(Param::BaseColor, spectrum(0.2, 0.4, 0.8)),
        },
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::SpecularRoughness, 0.35),
        },
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::SpecularIor, 1.45),
        },
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::Anisotropy, -0.5),
        },
        Command::SetMaterialParameter {
            material,
            parameter: MaterialParameter::scalar(Param::DiffuseRoughness, 0.9),
        },
        Command::RenameMaterial {
            material,
            name: "Anodized".into(),
        },
        Command::SetMaterialType {
            material,
            material_type: MaterialType::GlassSolid,
        },
        // Both shapes: attaching a texture, and removing one.
        Command::SetParameterTexture {
            material,
            param: Param::BaseColor,
            source: Some(TextureSource {
                path: std::path::PathBuf::from("maps/panel.png"),
                role: MapRole::Color,
                color_space: Some(ColorSpace::Srgb),
            }),
        },
        Command::SetParameterTexture {
            material,
            param: Param::SpecularRoughness,
            source: None,
        },
        Command::SetLightingPreset {
            preset: LightingPreset::Interior,
        },
        Command::SetEnvironmentParameter {
            parameter: EnvironmentParameter::scalar(EnvParam::Rotation, 0.75),
        },
        Command::SetEnvironmentParameter {
            parameter: EnvironmentParameter::scalar(EnvParam::Brightness, 2.5),
        },
        Command::SetLightingConfig {
            config: Box::new(tequila_render_core::settings::IntegratorConfig::jewelry()),
        },
        // One of each value shape, since the shape is what decides how the
        // record is read back.
        Command::SetEnvironmentParameter {
            parameter: EnvironmentParameter::flag(EnvParam::GroundFlatten, true),
        },
        Command::SetEnvironmentParameter {
            parameter: EnvironmentParameter::color(
                EnvParam::BackgroundColor,
                spectrum(0.9, 0.9, 1.0),
            ),
        },
        Command::SetEnvironmentParameter {
            parameter: EnvironmentParameter::new(
                EnvParam::Background,
                tequila_scene::command::EnvValue::Mode(
                    tequila_scene::document::BackgroundMode::Backplate,
                ),
            ),
        },
        Command::SetEnvironmentParameter {
            parameter: EnvironmentParameter::new(
                EnvParam::Backplate,
                tequila_scene::command::EnvValue::Path(Some("plates/studio.jpg".into())),
            ),
        },
        Command::AddLight {
            parent: None,
            name: "Key".into(),
            settings: Box::new(tequila_scene::lighting::LightSettings::default()),
        },
        Command::SetLight {
            node: ids[1],
            settings: Box::new(tequila_scene::lighting::LightSettings {
                shape: tequila_scene::lighting::LightShape::Spot {
                    inner_degrees: 12.0,
                    outer_degrees: 30.0,
                    falloff: 2.0,
                },
                intensity: tequila_scene::lighting::Intensity::Candela(400.0),
                temperature: Some(3200.0),
                layer: 3,
                ..Default::default()
            }),
        },
        Command::AddCamera {
            camera: Box::new(tequila_scene::lighting::SavedCamera {
                name: "Hero".into(),
                camera: tequila_render_core::camera::Camera::look_at(
                    tequila_core::math::vec3(400.0, 220.0, 600.0),
                    tequila_core::math::vec3(0.0, 40.0, 0.0),
                    tequila_core::math::Vec3::Y,
                ),
                target: tequila_core::math::vec3(0.0, 40.0, 0.0),
            }),
        },
        Command::SetCamera {
            index: 0,
            camera: Box::new(tequila_scene::lighting::SavedCamera {
                name: "Hero".into(),
                camera: tequila_render_core::camera::Camera::look_at(
                    tequila_core::math::vec3(1.0, 2.0, 3.0),
                    tequila_core::math::Vec3::ZERO,
                    tequila_core::math::Vec3::Y,
                ),
                target: tequila_core::math::Vec3::ZERO,
            }),
        },
        Command::RemoveCamera { index: 1 },
        Command::SelectCamera { index: 0 },
        Command::RenameCamera {
            index: 0,
            name: "Three quarter".into(),
        },
        // Both shapes: choosing a config, and going back to the
        // built-in transforms.
        Command::SetColorManagement {
            selection: Some(tequila_scene::document::OcioSelection {
                config: "studio/config.ocio".into(),
                display: "sRGB".into(),
                view: "ACES 1.0 - SDR Video".into(),
            }),
        },
        Command::SetColorManagement { selection: None },
        Command::SetRenderSettings {
            settings: Box::new(tequila_scene::render::RenderSettings {
                width: 3840,
                height: 2160,
                max_samples: 512,
                max_seconds: Some(600),
                clamp_indirect: Some(12.0),
                region: Some(tequila_scene::render::Region {
                    x: 10,
                    y: 20,
                    width: 320,
                    height: 240,
                }),
                format: tequila_scene::render::OutputFormat::Exr,
                depth: tequila_scene::render::BitDepth::ThirtyTwo,
                write_passes: true,
                output_directory: "renders".into(),
                ..Default::default()
            }),
        },
        Command::SetImageStyle {
            style: Box::new(tequila_color::ImageStyle {
                mode: tequila_color::ToneMode::Photographic,
                exposure: 0.75,
                highlights: -0.4,
                temperature: 5200.0,
                vignette: 0.3,
                ..Default::default()
            }),
        },
    ]
}

/// Each command's name and its example, as the JSON a script sends in
/// `{"op":"apply","command":...}`. The first example of each name.
pub fn by_name() -> Vec<(&'static str, crate::json::Value)> {
    let mut out: Vec<(&'static str, crate::json::Value)> = Vec::new();
    for command in commands() {
        let name = command.name();
        if out.iter().any(|(n, _)| *n == name) {
            continue;
        }
        if let Ok(value) = crate::json::parse(encode(&command).as_bytes()) {
            out.push((name, value));
        }
    }
    out
}
