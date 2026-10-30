//! Auto-save and crash recovery (§4.5).
//!
//! > Journal every command to `~/.tequila/journal/<session>.jsonl` as it is
//! > applied. On crash, offer to replay the journal onto the last saved
//! > state. […] This is cheap to build on top of the command bus and it saves
//! > users' work — which matters far more than it appears on a feature list.
//!
//! Cheap because the command bus already holds the one thing recovery needs:
//! an ordered list of every mutation, with nothing else able to change the
//! document behind its back. Recovery is replaying it.
//!
//! # Why commands serialize as their catalog names
//!
//! A journal line is `{"command": "scene.rename", …}`, using the same name
//! `Command::name()` returns and `Command::CATALOG` lists — the name the
//! scripting API will bind. One vocabulary for the command bus, the journal
//! and the Python API means a command cannot be added to one and forgotten in
//! the others, and the catalog test already checks the set is complete.

use crate::json::{self, Value};
use crate::json_write::{self, Object};
use crate::{fsmesh, FormatError};
use tequila_core::math::{Mat4, Transform};
use tequila_core::spectrum::Spectrum;
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

/// The most snapshots a session will rotate through, and so the most the
/// recovery scan has to look for. A cap rather than a setting because the two
/// numbers must agree: a scan that stopped short of what a session wrote would
/// silently offer an older snapshot than the one available.
pub const MAX_SNAPSHOTS: usize = 8;

/// One journalled command, as a single line of JSONL.
pub fn encode(command: &Command) -> String {
    let mut out = Object::new();
    out.set("command", command.name());

    match command {
        Command::Import { payload } => {
            out.set("payload", import_payload_to_json(payload));
        }
        Command::Reimport { payload } => {
            out.set("payload", reimport_payload_to_json(payload));
        }
        Command::AddGroup { parent, name } => {
            out.set_some("parent", parent.map(|p| p.0))
                .set("name", name.clone());
        }
        Command::DeleteNodes { nodes } => {
            out.set("nodes", ids(nodes));
        }
        Command::SplitIntoParts { node } => {
            out.set("node", node.0);
        }
        Command::UnwrapUvs { node, options } => {
            out.set("node", node.0)
                .set("cone_degrees", options.cone_degrees)
                .set("crease_degrees", options.crease_degrees)
                .set("margin", options.margin);
        }
        Command::Rename { node, name } => {
            out.set("node", node.0).set("name", name.clone());
        }
        Command::SetTransform { transforms } => {
            // A node and its matrix per entry, so a multi-part move survives
            // as the one command it was rather than as several.
            out.set(
                "transforms",
                Value::Array(
                    transforms
                        .iter()
                        .map(|(node, transform)| {
                            let mut entry = Object::new();
                            entry.set("node", node.0).set("matrix", matrix(*transform));
                            entry.build()
                        })
                        .collect(),
                ),
            );
        }
        Command::SetVisibility { states } => {
            out.set("states", flags(states));
        }
        Command::SetLayerVisibility { layer, visible } => {
            out.set("layer", layer.clone()).set("visible", *visible);
        }
        Command::SetLocked { states } => {
            out.set("states", flags(states));
        }
        Command::SetRays { states } => {
            out.set(
                "states",
                Value::Array(
                    states
                        .iter()
                        .map(|(node, rays)| {
                            let mut entry = Object::new();
                            entry.set("node", node.0).set("rays", crate::rays_to_json(rays));
                            entry.build()
                        })
                        .collect(),
                ),
            );
        }
        Command::SetPivot { pivots } => {
            out.set(
                "pivots",
                Value::Array(
                    pivots
                        .iter()
                        .map(|(node, pivot)| {
                            let mut entry = Object::new();
                            entry.set("node", node.0);
                            // Absent means "cleared", which is a different
                            // thing from a pivot at the origin.
                            if let Some(p) = pivot {
                                entry.set("at", Value::from(vec![p.x, p.y, p.z]));
                            }
                            entry.build()
                        })
                        .collect(),
                ),
            );
        }
        Command::Reparent {
            node,
            parent,
            index,
        } => {
            out.set("node", node.0)
                .set_some("parent", parent.map(|p| p.0))
                .set_some("index", *index);
        }
        Command::Group { nodes, name } => {
            out.set("nodes", ids(nodes)).set("name", name.clone());
        }
        Command::Ungroup { group } => {
            out.set("group", group.0);
        }
        Command::SetMetadata { node, key, value } => {
            out.set("node", node.0)
                .set("key", key.clone())
                .set_some("value", value.clone());
        }
        Command::SetUnits { unit, mode } => {
            out.set("unit", crate::unit_name(*unit)).set(
                "mode",
                match mode {
                    ConversionMode::KeepSceneSize => "KeepSceneSize",
                    ConversionMode::ScaleScene => "ScaleScene",
                },
            );
        }
        Command::AssignMaterial { nodes, material } => {
            out.set("nodes", ids(nodes)).set("material", material.0);
        }
        Command::UnlinkMaterial { node } => {
            out.set("node", node.0);
        }
        Command::SetMaterialParameter {
            material,
            parameter,
        } => {
            out.set("material", material.0);
            let value = match parameter.value {
                ParamValue::Scalar(v) => Value::from(v),
                ParamValue::Color(c) => color(c),
                ParamValue::Flag(v) => Value::from(v),
                ParamValue::Metal(ior) => {
                    let mut object = Object::new();
                    object.set("n", color(ior.n)).set("k", color(ior.k));
                    object.build()
                }
            };
            out.set("parameter", parameter.param.key()).set("value", value);
        }
        Command::RenameMaterial { material, name } => {
            out.set("material", material.0).set("name", name.clone());
        }
        Command::SetGraph { material, graph } => {
            // In the project file's own material format, which already
            // carries a graph — surface literals, nodes, links, decals — and
            // is tested for fidelity. The name and type are placeholders the
            // decoder discards; only the graph is the command's.
            let definition = tequila_scene::material::MaterialDefinition::from_graph(
                "graph",
                MaterialType::ALL[0],
                (**graph).clone(),
            );
            out.set("material", material.0)
                .set("definition", crate::material_to_json(*material, &definition));
        }
        Command::SetMaterialType {
            material,
            material_type,
        } => {
            out.set("material", material.0)
                .set("type", material_type.id());
        }
        Command::AddDecal { material, source, name } => {
            out.set("material", material.0).set("name", name.clone())
                .set("path", source.path.to_string_lossy().to_string()).set("role", role_name(source.role));
            if let Some(space) = source.color_space { out.set("color_space", color_space_name(space)); }
        }
        Command::SetDecals { material, decals } => {
            out.set("material", material.0).set("decals", crate::graph::decals_to_json(decals));
        }
        Command::SetParameterTexture {
            material,
            param,
            source,
        } => {
            out.set("material", material.0).set("param", param.key());
            if let Some(source) = source {
                out.set("path", source.path.to_string_lossy().to_string())
                    .set("role", role_name(source.role));
                if let Some(space) = source.color_space {
                    out.set("color_space", color_space_name(space));
                }
            }
        }
        Command::SetParameterGradient {
            material,
            param,
            gradient,
        } => {
            out.set("material", material.0)
                .set("param", param.key())
                .set("gradient", crate::graph::gradient_to_json(gradient));
        }
        Command::SetParameterProcedural {
            material,
            param,
            procedural,
            seed,
        } => {
            out.set("material", material.0)
                .set("param", param.key())
                .set("seed", *seed as f32);
            if let Some(procedural) = procedural {
                out.set("procedural", crate::graph::procedural_to_json(procedural));
            }
        }
        Command::SetTextureMapping {
            material,
            param,
            mapping,
        } => {
            out.set("material", material.0)
                .set("param", param.key())
                .set("projection", projection_name(mapping.projection))
                .set("axis", axis_name(mapping.axis))
                .set("uv_set", mapping.uv_set as f32)
                .set(
                    "scale",
                    vec![mapping.scale.x, mapping.scale.y, mapping.scale.z],
                )
                .set("offset", vec![mapping.offset.x, mapping.offset.y])
                .set("rotation", mapping.rotation)
                .set("pivot", vec![mapping.pivot.x, mapping.pivot.y])
                .set("wrap_u", wrap_name(mapping.wrap_u))
                .set("wrap_v", wrap_name(mapping.wrap_v))
                .set("flip_u", mapping.flip_u)
                .set("flip_v", mapping.flip_v)
                .set("blend_sharpness", mapping.blend_sharpness)
                .set("real_world_units", mapping.real_world_units);
        }
        Command::SetSky { sky } => {
            // `null` rather than an absent key: the difference between "no
            // sky" and "a build that did not know about skies" has to survive
            // a replay, or undoing back past a sky would resurrect it.
            match sky.as_ref() {
                Some(sky) => out.set("sky", crate::sky_to_json(sky)),
                None => out.set("sky", crate::json::Value::Null),
            };
        }
        Command::SetEnvironmentPins { pins } => {
            out.set(
                "pins",
                crate::json::Value::from(
                    pins.iter().map(crate::pin_to_json).collect::<Vec<_>>(),
                ),
            );
        }
        Command::SetLightingPreset { preset } => {
            out.set("preset", crate::preset_name(*preset));
        }
        Command::SetLightingConfig { config } => {
            out.set("config", crate::integrator_to_json(config));
        }
        Command::SetEnvironmentParameter { parameter } => {
            let value = match &parameter.value {
                EnvValue::Scalar(v) => Value::from(*v),
                EnvValue::Flag(v) => Value::from(*v),
                EnvValue::Color(c) => color(*c),
                EnvValue::Mode(m) => Value::from(m.label()),
                EnvValue::Path(Some(p)) => Value::from(p.clone()),
                EnvValue::Path(None) => Value::Null,
                EnvValue::Choice(c) => Value::from(*c),
            };
            out.set("parameter", parameter.param.key()).set("value", value);
        }
        Command::AddLight {
            parent,
            name,
            settings,
        } => {
            out.set_some("parent", parent.map(|p| p.0))
                .set("name", name.clone())
                .set("light", crate::light_to_json(settings));
        }
        Command::SetLight { node, settings } => {
            out.set("node", node.0)
                .set("light", crate::light_to_json(settings));
        }
        Command::AddCutaway {
            parent,
            name,
            settings,
            transform,
        } => {
            out.set_some("parent", parent.map(|p| p.0))
                .set("name", name.clone())
                .set("cutaway", crate::cutaway_to_json(settings))
                .set("transform", matrix(*transform));
        }
        Command::SetCutaway { node, settings } => {
            out.set("node", node.0)
                .set("cutaway", crate::cutaway_to_json(settings));
        }
        Command::SetMultiMaterials { multi_materials } => {
            out.set(
                "multi_materials",
                multi_materials.iter().map(crate::multi_material_to_json).collect::<Vec<_>>(),
            );
        }
        Command::SetModelSets { model_sets } => {
            out.set("model_sets", crate::model_sets_to_json(model_sets));
        }
        Command::SetStudios { studios } => {
            out.set("studios", studios.iter().map(crate::studio_to_json).collect::<Vec<_>>());
        }
        Command::ApplyStudio { index } => {
            out.set("index", *index as f64);
        }
        Command::TurnProduct { yaw } => {
            out.set("yaw", *yaw);
        }
        Command::SetAnimation { animation } => {
            out.set("animation", animation.as_deref().map_or(Value::Null, crate::animation_to_json));
        }
        Command::ApplyStudioPreset { preset, options } => {
            out.set("preset", crate::studio_preset::to_inline(preset));
            let mut o = Object::new();
            o.set("front", options.front.key())
                .set_some("scale", options.scale.map(|s| s.key()))
                .set("keep_lights", options.keep_lights)
                .set("keep_cameras", options.keep_cameras)
                .set("keep_placement", options.keep_placement);
            out.set("options", o.build());
        }
        Command::SetConfigurator { configurator } => {
            out.set("configurator", crate::configurator_to_json(configurator));
        }
        Command::Configure { choices } => {
            out.set("choices", choices.iter().map(|&c| Value::from(c)).collect::<Vec<_>>());
        }
        Command::SetMaterialTemplates { templates } => {
            out.set("templates", templates.iter().map(crate::template_to_json).collect::<Vec<_>>());
        }
        Command::SetMeasurements { measurements } => {
            out.set(
                "measurements",
                measurements.iter().map(crate::measurement_to_json).collect::<Vec<_>>(),
            );
        }
        Command::AddCamera { camera } => {
            out.set("camera", crate::camera_to_json(camera));
        }
        Command::SetCamera { index, camera } => {
            out.set("index", *index as f64)
                .set("camera", crate::camera_to_json(camera));
        }
        Command::RemoveCamera { index } => {
            out.set("index", *index as f64);
        }
        Command::SelectCamera { index } => {
            out.set("index", *index as f64);
        }
        Command::RenameCamera { index, name } => {
            out.set("index", *index as f64).set("name", name.clone());
        }
        Command::SetImageStyle { style } => {
            out.set("style", crate::image_style_to_json(style));
        }
        Command::SetRenderSettings { settings } => {
            out.set("render", crate::render_settings_to_json(settings));
        }
        Command::SetColorManagement { selection } => {
            out.set(
                "color_management",
                crate::color_management_to_json(selection.as_ref()),
            );
        }
    }

    // One line: a journal is appended to under a crash, and a record split
    // across lines cannot be told from a truncated one.
    let text = json_write::to_string(&out.build());
    let compact: String = text
        .split('\n')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    compact
}

/// Read one journal line back.
pub fn decode(line: &str) -> Result<Command, FormatError> {
    let value = json::parse(line.as_bytes())
        .map_err(|e| FormatError::Invalid(format!("journal line: {e}")))?;
    let name = value
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| FormatError::Invalid("a journal line names no command".into()))?;

    let node = |key: &str| -> Result<NodeId, FormatError> {
        value
            .get(key)
            .and_then(|v| v.as_index())
            .map(|v| NodeId(v as u64))
            .ok_or_else(|| FormatError::Invalid(format!("`{name}` has no `{key}`")))
    };
    let optional_node = |key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_index())
            .map(|v| NodeId(v as u64))
    };
    let text = |key: &str| -> Result<String, FormatError> {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| FormatError::Invalid(format!("`{name}` has no `{key}`")))
    };
    let index = |key: &str| -> Result<usize, FormatError> {
        value
            .get(key)
            .and_then(|v| v.as_index())
            .ok_or_else(|| FormatError::Invalid(format!("`{name}` has no `{key}`")))
    };
    let scalar = |key: &str| -> Result<f32, FormatError> {
        value
            .get(key)
            .and_then(|v| v.as_f32())
            .filter(|v| v.is_finite())
            .ok_or_else(|| FormatError::Invalid(format!("`{name}` has no finite `{key}`")))
    };
    let node_list = |key: &str| -> Vec<NodeId> {
        value
            .get(key)
            .map(|v| v.as_array())
            .unwrap_or(&[])
            .iter()
            .filter_map(|v| v.as_index())
            .map(|v| NodeId(v as u64))
            .collect()
    };
    let material = |key: &str| -> Result<MaterialKey, FormatError> {
        value
            .get(key)
            .and_then(|v| v.as_index())
            .map(|v| MaterialKey(v as u64))
            .ok_or_else(|| FormatError::Invalid(format!("`{name}` has no `{key}`")))
    };

    Ok(match name {
        "scene.import" => Command::Import {
            payload: Box::new(import_payload_from_json(value.get("payload").ok_or_else(
                || FormatError::Invalid("the import has no payload".into()),
            )?)?),
        },
        "scene.reimport" => Command::Reimport {
            payload: Box::new(reimport_payload_from_json(value.get("payload").ok_or_else(
                || FormatError::Invalid("the update has no payload".into()),
            )?)?),
        },
        "scene.add_group" => Command::AddGroup {
            parent: optional_node("parent"),
            name: text("name")?,
        },
        "scene.delete_nodes" => Command::DeleteNodes {
            nodes: node_list("nodes"),
        },
        "scene.split_parts" => Command::SplitIntoParts { node: node("node")? },
        // Each setting the default's when left out, and kept to what lays a
        // surface out: a cone past a right angle would fold charts over.
        "scene.unwrap_uvs" => {
            let defaults = tequila_scene::uv::UnwrapOptions::default();
            let number = |key: &str, fallback: f32, low: f32, high: f32| {
                value.get(key).and_then(|v| v.as_f32()).filter(|v| v.is_finite()).map(|v| v.clamp(low, high)).unwrap_or(fallback)
            };
            Command::UnwrapUvs {
                node: node("node")?,
                options: tequila_scene::uv::UnwrapOptions {
                    cone_degrees: number("cone_degrees", defaults.cone_degrees, 5.0, 89.0),
                    crease_degrees: number("crease_degrees", defaults.crease_degrees, 1.0, 180.0),
                    margin: number("margin", defaults.margin, 0.0, 0.1),
                },
            }
        }
        "scene.rename" => Command::Rename {
            node: node("node")?,
            name: text("name")?,
        },
        "scene.set_transform" => {
            let mut transforms = Vec::new();
            for entry in value.get("transforms").map(|t| t.as_array()).unwrap_or(&[]) {
                let id = entry
                    .get("node")
                    .and_then(|v| v.as_index())
                    .ok_or_else(|| FormatError::Invalid("a transform names no node".into()))?;
                transforms.push((NodeId(id as u64), read_named_matrix(entry, "matrix")?));
            }
            if transforms.is_empty() {
                return Err(FormatError::Invalid(
                    "`scene.set_transform` has no transforms".into(),
                ));
            }
            Command::SetTransform { transforms }
        }
        "scene.set_visibility" => Command::SetVisibility {
            states: read_flags(&value)?,
        },
        // Whether to show or hide is the whole point, so it is never
        // assumed.
        "scene.set_layer_visibility" => Command::SetLayerVisibility {
            layer: text("layer")?,
            visible: value
                .get("visible")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| FormatError::Invalid(format!("`{name}` has no `visible`")))?,
        },
        "scene.set_locked" => Command::SetLocked {
            states: read_flags(&value)?,
        },
        "scene.set_rays" => {
            let mut states = Vec::new();
            for entry in value.get("states").map(|s| s.as_array()).unwrap_or(&[]) {
                let id = entry
                    .get("node")
                    .and_then(|v| v.as_index())
                    .ok_or_else(|| FormatError::Invalid("a state names no node".into()))?;
                states.push((NodeId(id as u64), crate::read_rays(entry.get("rays"))));
            }
            Command::SetRays { states }
        }
        "scene.set_pivot" => {
            let mut pivots = Vec::new();
            for entry in value.get("pivots").map(|p| p.as_array()).unwrap_or(&[]) {
                let id = entry
                    .get("node")
                    .and_then(|v| v.as_index())
                    .ok_or_else(|| FormatError::Invalid("a pivot names no node".into()))?;
                let at = entry
                    .get("at")
                    .and_then(|v| v.as_f32_array())
                    .filter(|v| v.len() == 3)
                    .map(|v| tequila_core::math::vec3(v[0], v[1], v[2]));
                pivots.push((NodeId(id as u64), at));
            }
            if pivots.is_empty() {
                return Err(FormatError::Invalid(
                    "`scene.set_pivot` has no pivots".into(),
                ));
            }
            Command::SetPivot { pivots }
        }
        "scene.reparent" => Command::Reparent {
            node: node("node")?,
            parent: optional_node("parent"),
            index: value.get("index").and_then(|v| v.as_index()),
        },
        "scene.group" => Command::Group {
            nodes: node_list("nodes"),
            name: text("name")?,
        },
        "scene.ungroup" => Command::Ungroup {
            group: node("group")?,
        },
        "scene.set_metadata" => Command::SetMetadata {
            node: node("node")?,
            key: text("key")?,
            value: value
                .get("value")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        "scene.set_units" => Command::SetUnits {
            unit: crate::unit_from_name(&text("unit")?)
                .ok_or_else(|| FormatError::Invalid("the journal names an unknown unit".into()))?,
            mode: match value.get("mode").and_then(|v| v.as_str()) {
                Some("ScaleScene") => ConversionMode::ScaleScene,
                _ => ConversionMode::KeepSceneSize,
            },
        },
        "material.assign" => Command::AssignMaterial {
            nodes: node_list("nodes"),
            material: material("material")?,
        },
        "material.unlink" => Command::UnlinkMaterial {
            node: node("node")?,
        },
        "material.set_parameter" => {
            let named = value.get("parameter").and_then(|v| v.as_str());
            let param = named.and_then(Param::from_key).ok_or_else(|| {
                FormatError::Invalid(format!("unknown material parameter {named:?}"))
            })?;
            // The shape of the recorded value has to match the parameter, and
            // the parameter itself says which shape that is. Reading the shape
            // from the JSON instead would let a colour written as three
            // numbers be replayed into a scalar.
            let shape = param.read(&Material::default()).ok_or_else(|| {
                FormatError::Invalid(format!(
                    "`{}` is a texture slot and carries no value",
                    param.key()
                ))
            })?;
            let parsed = match shape {
                ParamValue::Scalar(_) => ParamValue::Scalar(scalar("value")?),
                ParamValue::Color(_) => ParamValue::Color(read_color(&value)?),
                ParamValue::Flag(_) => ParamValue::Flag(
                    value
                        .get("value")
                        .and_then(|v| v.as_bool())
                        .ok_or_else(|| FormatError::Invalid("value is not a flag".into()))?,
                ),
                ParamValue::Metal(fallback) => {
                    let ior = value.get("value");
                    ParamValue::Metal(ComplexIor {
                        n: ior
                            .and_then(|v| v.get("n"))
                            .and_then(read_spectrum)
                            .unwrap_or(fallback.n),
                        k: ior
                            .and_then(|v| v.get("k"))
                            .and_then(read_spectrum)
                            .unwrap_or(fallback.k),
                    })
                }
            };
            let parameter = MaterialParameter::new(param, parsed);
            Command::SetMaterialParameter {
                material: material("material")?,
                parameter,
            }
        }
        "material.set_gradient" => {
            let named = text("param")?;
            let param = Param::from_key(&named).ok_or_else(|| {
                FormatError::Invalid(format!("`{named}` is not a material parameter"))
            })?;
            Command::SetParameterGradient {
                material: material("material")?,
                param,
                gradient: Box::new(
                    value
                        .get("gradient")
                        .and_then(crate::graph::gradient_from_json)
                        .unwrap_or_default(),
                ),
            }
        }
        "material.set_procedural" => {
            let named = text("param")?;
            let param = Param::from_key(&named).ok_or_else(|| {
                FormatError::Invalid(format!("`{named}` is not a material parameter"))
            })?;
            // Absent means "detach", the same as a texture command with no
            // path. Present but unreadable is a mistake, and is refused: read
            // as absent, a misspelt kind stripped the pattern off.
            let procedural = match value.get("procedural") {
                None | Some(Value::Null) => None,
                Some(given) => Some(Box::new(crate::graph::procedural_from_json(given).ok_or_else(|| {
                    let kinds: Vec<&str> = tequila_texture::procedural::Procedural::catalog()
                        .iter()
                        .map(|p| p.key())
                        .collect();
                    FormatError::Invalid(format!(
                        "material.set_procedural: not a procedural this version knows; the kinds are {}",
                        kinds.join(", ")
                    ))
                })?)),
            };
            Command::SetParameterProcedural {
                material: material("material")?,
                param,
                procedural,
                seed: value.get("seed").and_then(|v| v.as_f32()).unwrap_or(0.0) as u32,
            }
        }
        "material.set_mapping" => {
            use tequila_texture::projection::Mapping;
            let named = text("param")?;
            let param = Param::from_key(&named).ok_or_else(|| {
                FormatError::Invalid(format!("`{named}` is not a material parameter"))
            })?;
            let default = Mapping::default();
            let scale = floats(&value, "scale");
            let offset = floats(&value, "offset");
            let pivot = floats(&value, "pivot");
            let projection = text("projection")
                .ok()
                .and_then(|t| projection_from_name(&t))
                .unwrap_or(default.projection);
            let mapping = Mapping {
                projection,
                // Entries written before the axis existed keep their meaning:
                // planar looked down Z, cylindrical wrapped about Y.
                axis: match text("axis").ok().and_then(|t| axis_from_name(&t)) {
                    Some(axis) => axis,
                    None if projection
                        == tequila_texture::projection::Projection::Cylindrical =>
                    {
                        tequila_texture::projection::Axis::Y
                    }
                    None => tequila_texture::projection::Axis::Z,
                },
                uv_set: index("uv_set").unwrap_or(0),
                scale: tequila_core::math::vec3(
                    *scale.first().unwrap_or(&default.scale.x),
                    *scale.get(1).unwrap_or(&default.scale.y),
                    *scale.get(2).unwrap_or(&default.scale.z),
                ),
                offset: tequila_core::math::vec2(
                    *offset.first().unwrap_or(&0.0),
                    *offset.get(1).unwrap_or(&0.0),
                ),
                rotation: scalar("rotation").unwrap_or(0.0),
                pivot: tequila_core::math::vec2(
                    *pivot.first().unwrap_or(&default.pivot.x),
                    *pivot.get(1).unwrap_or(&default.pivot.y),
                ),
                wrap_u: text("wrap_u")
                    .ok()
                    .and_then(|t| wrap_from_name(&t))
                    .unwrap_or(default.wrap_u),
                wrap_v: text("wrap_v")
                    .ok()
                    .and_then(|t| wrap_from_name(&t))
                    .unwrap_or(default.wrap_v),
                flip_u: boolean(&value, "flip_u"),
                flip_v: boolean(&value, "flip_v"),
                blend_sharpness: scalar("blend_sharpness").unwrap_or(default.blend_sharpness),
                real_world_units: boolean(&value, "real_world_units"),
            };
            Command::SetTextureMapping {
                material: material("material")?,
                param,
                mapping: Box::new(mapping),
            }
        }
        "material.rename" => Command::RenameMaterial {
            material: material("material")?,
            name: text("name")?,
        },
        "material.set_graph" => {
            let definition = value
                .get("definition")
                .ok_or_else(|| FormatError::Invalid("material.set_graph has no definition".into()))?;
            // A material with no graph is a plain one, which a project file
            // may hold; this command's whole content is the graph, and one
            // that is missing or not an object would otherwise be read as a
            // blank graph and silently replace the material's.
            if !matches!(definition.get("graph"), Some(Value::Object(_))) {
                return Err(FormatError::Invalid(
                    "material.set_graph: definition.graph must be a graph object (nodes, links, output)".into(),
                ));
            }
            Command::SetGraph {
                material: material("material")?,
                graph: Box::new(crate::read_material(definition, "material.set_graph")?.graph),
            }
        }
        "material.add_decal" => Command::AddDecal {
            material: material("material")?, name: text("name")?,
            source: TextureSource {
                path: text("path")?.into(),
                role: value.get("role").and_then(Value::as_str).and_then(role_from_name).unwrap_or(MapRole::Color),
                color_space: value.get("color_space").and_then(Value::as_str).and_then(color_space_from_name),
            },
        },
        "material.set_decals" => Command::SetDecals {
            material: material("material")?, decals: crate::graph::decals_from_json(value.get("decals"))?,
        },
        "material.set_texture" => {
            let named = text("param")?;
            let param = Param::from_key(&named).ok_or_else(|| {
                FormatError::Invalid(format!("`{named}` is not a material parameter"))
            })?;
            // No path means the command removed the texture, which is a
            // meaningful thing to record rather than an omission.
            let source = match value.get("path").and_then(|v| v.as_str()) {
                Some(path) => {
                    let role = value
                        .get("role")
                        .and_then(|v| v.as_str())
                        .and_then(role_from_name)
                        .unwrap_or(MapRole::Color);
                    Some(TextureSource {
                        path: std::path::PathBuf::from(path),
                        role,
                        color_space: value
                            .get("color_space")
                            .and_then(|v| v.as_str())
                            .and_then(color_space_from_name),
                    })
                }
                None => None,
            };
            Command::SetParameterTexture {
                material: material("material")?,
                param,
                source,
            }
        }
        "material.set_type" => {
            let named = text("type")?;
            let material_type = MaterialType::from_id(&named).ok_or_else(|| {
                FormatError::Invalid(format!("`{named}` is not a material type"))
            })?;
            Command::SetMaterialType {
                material: material("material")?,
                material_type,
            }
        }
        "lighting.set_preset" => Command::SetLightingPreset {
            preset: {
                let named = text("preset")?;
                crate::preset_from_name(&named).ok_or_else(|| {
                    FormatError::Invalid(format!(
                        "`{named}` is not a lighting preset; they are Performance, Basic, Product, Interior and Jewelry"
                    ))
                })?
            },
        },
        "lighting.set_custom" => {
            let config = value
                .get("config")
                .ok_or_else(|| FormatError::Invalid("lighting.set_custom has no config".into()))?;
            Command::SetLightingConfig {
                config: Box::new(crate::read_integrator(
                    config,
                    tequila_render_core::settings::IntegratorConfig::product(),
                )),
            }
        }
        "environment.set_sky" => Command::SetSky {
            sky: Box::new(
                value
                    .get("sky")
                    .filter(|v| !matches!(v, crate::json::Value::Null))
                    .map(crate::sky_from_json),
            ),
        },
        "environment.set_pins" => Command::SetEnvironmentPins {
            pins: value
                .get("pins")
                .map(|v| v.as_array())
                .map(|entries| entries.iter().map(crate::pin_from_json).collect())
                .unwrap_or_default(),
        },
        "environment.set_parameter" => {
            let named = text("parameter")?;
            let param = EnvParam::from_key(&named).ok_or_else(|| {
                FormatError::Invalid(format!("unknown environment parameter `{named}`"))
            })?;
            let raw = value
                .get("value")
                .ok_or_else(|| FormatError::Invalid("an environment edit has no value".into()))?;
            // The parameter's own shape decides how the value is read, so a
            // journal cannot smuggle a number into a flag and have it applied.
            let default = crate::EnvironmentSettings::default();
            let shaped = match param.read(&default) {
                EnvValue::Scalar(_) => raw.as_f32().map(EnvValue::Scalar),
                EnvValue::Flag(_) => raw.as_bool().map(EnvValue::Flag),
                EnvValue::Color(_) => crate::read_spectrum(raw).map(EnvValue::Color),
                EnvValue::Mode(_) => raw
                    .as_str()
                    .and_then(|s| crate::BackgroundMode::ALL.into_iter().find(|m| m.label() == s))
                    .map(EnvValue::Mode),
                EnvValue::Path(_) => Some(EnvValue::Path(raw.as_str().map(|s| s.to_string()))),
                EnvValue::Choice(_) => raw
                    .as_str()
                    .and_then(|s| param.choices().iter().find(|(key, _)| *key == s))
                    .map(|(key, _)| EnvValue::Choice(key)),
            };
            let shaped = shaped.ok_or_else(|| {
                FormatError::Invalid(format!("`{named}` was given a value of the wrong kind"))
            })?;
            Command::SetEnvironmentParameter {
                parameter: EnvironmentParameter::new(param, shaped),
            }
        }
        "scene.add_light" => {
            let light = value
                .get("light")
                .ok_or_else(|| FormatError::Invalid("scene.add_light has no light".into()))?;
            Command::AddLight {
                parent: value
                    .get("parent")
                    .and_then(|v| v.as_index())
                    .map(|p| NodeId(p as u64)),
                name: text("name").unwrap_or_else(|_| "Light".into()),
                settings: Box::new(crate::read_light(light)),
            }
        }
        "scene.set_light" => {
            let light = value
                .get("light")
                .ok_or_else(|| FormatError::Invalid("scene.set_light has no light".into()))?;
            Command::SetLight {
                node: node("node")?,
                settings: Box::new(crate::read_light(light)),
            }
        }
        "scene.add_cutaway" => {
            let cutaway = value
                .get("cutaway")
                .ok_or_else(|| FormatError::Invalid("scene.add_cutaway has no cutaway".into()))?;
            Command::AddCutaway {
                parent: value
                    .get("parent")
                    .and_then(|v| v.as_index())
                    .map(|p| NodeId(p as u64)),
                name: text("name").unwrap_or_else(|_| "Cutaway".into()),
                settings: Box::new(crate::read_cutaway(cutaway)),
                transform: read_matrix(&value)?,
            }
        }
        "material.set_multi_materials" => Command::SetMultiMaterials {
            multi_materials: match value.get("multi_materials") {
                Some(list) => list
                    .as_array()
                    .iter()
                    .map(|entry| {
                        crate::read_multi_material(entry)
                            .ok_or_else(|| FormatError::Invalid("a multi-material is malformed".into()))
                    })
                    .collect::<Result<_, _>>()?,
                None => Vec::new(),
            },
        },
        "scene.set_model_sets" => Command::SetModelSets {
            model_sets: Box::new(value.get("model_sets").map(crate::read_model_sets).unwrap_or_default()),
        },
        "studio.set" => Command::SetStudios {
            studios: value
                .get("studios")
                .map(|v| v.as_array().iter().map(crate::read_studio).collect())
                .unwrap_or_default(),
        },
        "configurator.set" => Command::SetConfigurator {
            configurator: Box::new(value.get("configurator").map(crate::read_configurator).unwrap_or_default()),
        },
        "configurator.choose" => Command::Configure {
            choices: match value.get("choices") {
                Some(list) => list
                    .as_array()
                    .iter()
                    .map(|c| c.as_index().ok_or_else(|| FormatError::Invalid("a choice is not an index".into())))
                    .collect::<Result<_, _>>()?,
                None => return Err(FormatError::Invalid("configurator.choose needs its choices".into())),
            },
        },
        "material.set_templates" => Command::SetMaterialTemplates {
            templates: value
                .get("templates")
                .map(|v| v.as_array().iter().map(crate::read_template).collect())
                .unwrap_or_default(),
        },
        "studio_preset.apply" => {
            let preset = value.get("preset").ok_or_else(|| {
                FormatError::Invalid("studio_preset.apply carries no preset: give it inline, or name an installed one by \"id\" through the app".into())
            })?;
            let preset = crate::studio_preset::from_inline(preset).map_err(FormatError::Invalid)?;
            let options = value.get("options");
            let flag = |key: &str| options.and_then(|o| o.get(key)).and_then(|v| v.as_bool()).unwrap_or(false);
            let front = match options.and_then(|o| o.get("front")).and_then(|v| v.as_str()) {
                None => tequila_scene::preset::Front::PlusZ,
                Some(f) => tequila_scene::preset::Front::from_key(f)
                    .ok_or_else(|| FormatError::Invalid(format!("`{f}` is no front: +z, -z, +x or -x")))?,
            };
            let scale = match options.and_then(|o| o.get("scale")).and_then(|v| v.as_str()) {
                None => None,
                Some(s) => Some(tequila_scene::preset::FramingScale::from_key(s).ok_or_else(|| {
                    FormatError::Invalid(format!("`{s}` is no framing scale: cameras, cameras_and_lights or none"))
                })?),
            };
            Command::ApplyStudioPreset {
                preset: Box::new(preset),
                options: tequila_scene::preset::ApplyOptions {
                    front,
                    scale,
                    keep_lights: flag("keep_lights"),
                    keep_cameras: flag("keep_cameras"),
                    keep_placement: flag("keep_placement"),
                },
            }
        }
        "studio.turn_product" => Command::TurnProduct {
            yaw: value
                .get("yaw")
                .and_then(|v| v.as_f32())
                .ok_or_else(|| FormatError::Invalid("studio.turn_product names no yaw".into()))?,
        },
        "animation.set" => Command::SetAnimation {
            animation: match value.get("animation") {
                None | Some(Value::Null) => None,
                Some(animation) => Some(Box::new(crate::read_animation(animation))),
            },
        },
        "studio.apply" => Command::ApplyStudio {
            index: value
                .get("index")
                .and_then(|v| v.as_index())
                .ok_or_else(|| FormatError::Invalid("studio.apply names no studio".into()))?,
        },
        "scene.set_measurements" => Command::SetMeasurements {
            // Every entry or none: a list that lost one on the way in would
            // be a different edit from the one recorded.
            measurements: match value.get("measurements") {
                Some(list) => list
                    .as_array()
                    .iter()
                    .map(|entry| {
                        crate::read_measurement(entry)
                            .ok_or_else(|| FormatError::Invalid("a measurement is malformed".into()))
                    })
                    .collect::<Result<_, _>>()?,
                None => Vec::new(),
            },
        },
        "scene.set_cutaway" => {
            let cutaway = value
                .get("cutaway")
                .ok_or_else(|| FormatError::Invalid("scene.set_cutaway has no cutaway".into()))?;
            Command::SetCutaway {
                node: node("node")?,
                settings: Box::new(crate::read_cutaway(cutaway)),
            }
        }
        "camera.add" => {
            let camera = value
                .get("camera")
                .ok_or_else(|| FormatError::Invalid("camera.add has no camera".into()))?;
            Command::AddCamera {
                camera: Box::new(crate::read_camera(camera)),
            }
        }
        "camera.set" => {
            let camera = value
                .get("camera")
                .ok_or_else(|| FormatError::Invalid("camera.set has no camera".into()))?;
            Command::SetCamera {
                index: index("index")?,
                camera: Box::new(crate::read_camera(camera)),
            }
        }
        "camera.remove" => Command::RemoveCamera {
            index: index("index")?,
        },
        "camera.select" => Command::SelectCamera {
            index: index("index")?,
        },
        "camera.rename" => Command::RenameCamera {
            index: index("index")?,
            name: text("name")?,
        },
        "render.set_settings" => {
            let settings = value
                .get("render")
                .ok_or_else(|| FormatError::Invalid("render.set_settings has no settings".into()))?;
            Command::SetRenderSettings {
                settings: Box::new(crate::read_render_settings_value(settings)),
            }
        }
        "image.set_color_management" => Command::SetColorManagement {
            selection: value
                .get("color_management")
                .and_then(crate::read_color_management_value),
        },
        "image.set_style" => {
            let style = value
                .get("style")
                .ok_or_else(|| FormatError::Invalid("image.set_style has no style".into()))?;
            Command::SetImageStyle {
                style: Box::new(crate::read_image_style_value(style)),
            }
        }
        other => {
            return Err(FormatError::Invalid(format!(
                "the journal names an unknown command `{other}`"
            )))
        }
    })
}

/// A node and a flag per entry, for the commands that set one per node.
fn flags(states: &[(NodeId, bool)]) -> Value {
    Value::Array(
        states
            .iter()
            .map(|(node, flag)| {
                let mut entry = Object::new();
                entry.set("node", node.0).set("value", *flag);
                entry.build()
            })
            .collect(),
    )
}

fn read_flags(value: &Value) -> Result<Vec<(NodeId, bool)>, FormatError> {
    let mut out = Vec::new();
    for entry in value.get("states").map(|s| s.as_array()).unwrap_or(&[]) {
        let id = entry
            .get("node")
            .and_then(|v| v.as_index())
            .ok_or_else(|| FormatError::Invalid("a state names no node".into()))?;
        out.push((
            NodeId(id as u64),
            entry
                .get("value")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        ));
    }
    if out.is_empty() {
        return Err(FormatError::Invalid("a state command has no states".into()));
    }
    Ok(out)
}

fn ids(nodes: &[NodeId]) -> Value {
    Value::Array(nodes.iter().map(|n| Value::from(n.0)).collect())
}

fn color(s: Spectrum) -> Value {
    Value::from(vec![s.r, s.g, s.b])
}

/// A colour value: three components in the rendering space, or a hex sRGB
/// string (`"#c0392b"`) as a person or an AI app writes one, converted as
/// every picker converts it. What is recorded is the converted colour.
fn read_color(value: &Value) -> Result<Spectrum, FormatError> {
    value
        .get("value")
        .and_then(crate::read_spectrum)
        .ok_or_else(|| FormatError::Invalid("a colour needs three components, or a hex sRGB string like \"#c0392b\"".into()))
}

fn matrix(transform: Transform) -> Value {
    let mut values = Vec::with_capacity(16);
    for row in transform.matrix.m {
        values.extend_from_slice(&row);
    }
    Value::from(values)
}

fn read_matrix(value: &Value) -> Result<Transform, FormatError> {
    read_named_matrix(value, "transform")
}

fn read_named_matrix(value: &Value, key: &str) -> Result<Transform, FormatError> {
    let values = value
        .get(key)
        .and_then(|v| v.as_f32_array())
        .filter(|v| v.len() == 16)
        .ok_or_else(|| FormatError::Invalid("a transform needs sixteen values".into()))?;
    let mut m = Mat4::IDENTITY;
    for row in 0..4 {
        for column in 0..4 {
            m.m[row][column] = values[row * 4 + column];
        }
    }
    Transform::from_matrix(m)
        .ok_or_else(|| FormatError::Invalid("a journalled transform is not invertible".into()))
}

/// An import carries its geometry, and the journal has to carry it too.
///
/// A journal entry that named a file instead would recover nothing once that
/// file moved — and the case recovery exists for is the user who imported,
/// worked for an hour, and crashed. Meshes go in as base64 `.fsmesh`, which is
/// the same encoding the archive uses, so there is one mesh serializer rather
/// than two that can disagree.
fn import_payload_to_json(payload: &ImportPayload) -> Value {
    let mut out = Object::new();
    out.set(
        "meshes",
        Value::Array(
            payload
                .meshes
                .iter()
                .map(|(key, mesh)| {
                    let mut entry = Object::new();
                    entry
                        .set("key", key.0)
                        .set("data", base64_encode(&fsmesh::write(mesh)));
                    entry.build()
                })
                .collect(),
        ),
    );
    out.set(
        "materials",
        Value::Array(
            payload
                .materials
                .iter()
                .map(|(key, definition)| crate::material_to_json(*key, definition))
                .collect(),
        ),
    );
    out.set(
        "nodes",
        Value::Array(payload.nodes.iter().map(node_to_json).collect()),
    );
    out.set("roots", ids(&payload.roots));
    // The pictures the materials read. Left out, a replayed import of a
    // textured model or a texture pack came back with every map missing.
    if !payload.textures.is_empty() {
        out.set(
            "textures",
            Value::Array(
                payload
                    .textures
                    .iter()
                    .map(|(id, source)| {
                        let mut entry = Object::new();
                        entry
                            .set("id", id.0 as f64)
                            .set("path", source.path.to_string_lossy().to_string())
                            .set("role", role_name(source.role));
                        if let Some(space) = source.color_space {
                            entry.set("color_space", color_space_name(space));
                        }
                        entry.build()
                    })
                    .collect(),
            ),
        );
    }
    out.build()
}

fn reimport_payload_to_json(payload: &tequila_scene::reimport::ReimportPayload) -> Value {
    let mut out = Object::new();
    out.set("added", import_payload_to_json(&payload.added));
    out.set(
        "updates",
        Value::Array(
            payload
                .updates
                .iter()
                .map(|update| {
                    let mut entry = Object::new();
                    entry
                        .set("node", update.node.0)
                        .set_some("material", update.material.map(|m| m.0))
                        .set_some("path_key", update.path_key.clone())
                        .set(
                            "metadata",
                            Value::Object(
                                update
                                    .metadata
                                    .iter()
                                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                                    .collect(),
                            ),
                        );
                    if let NodeKind::Part { mesh } = update.kind {
                        entry.set("mesh", mesh.0);
                    }
                    if let Some(transform) = update.transform {
                        entry.set("transform", matrix(transform));
                    }
                    if !update.layers.is_empty() {
                        entry.set("layers", crate::layers_to_json(&update.layers));
                    }
                    entry.build()
                })
                .collect(),
        ),
    );
    out.set("hidden", ids(&payload.hidden));
    out.build()
}

fn reimport_payload_from_json(value: &Value) -> Result<tequila_scene::reimport::ReimportPayload, FormatError> {
    let added = import_payload_from_json(
        value.get("added").ok_or_else(|| FormatError::Invalid("the update adds nothing".into()))?,
    )?;
    let mut updates = Vec::new();
    for entry in value.get("updates").map(|u| u.as_array()).unwrap_or(&[]) {
        let node = entry
            .get("node")
            .and_then(|v| v.as_index())
            .ok_or_else(|| FormatError::Invalid("an updated node has no id".into()))?;
        let mut metadata = std::collections::BTreeMap::new();
        if let Some(Value::Object(map)) = entry.get("metadata") {
            for (key, item) in map {
                if let Some(text) = item.as_str() {
                    metadata.insert(key.clone(), text.to_string());
                }
            }
        }
        updates.push(tequila_scene::reimport::NodeUpdate {
            node: NodeId(node as u64),
            kind: match entry.get("mesh").and_then(|v| v.as_index()) {
                Some(mesh) => NodeKind::Part { mesh: MeshKey(mesh as u64) },
                None => NodeKind::Group,
            },
            transform: if entry.get("transform").is_some() { Some(read_matrix(entry)?) } else { None },
            material: entry.get("material").and_then(|v| v.as_index()).map(|m| MaterialKey(m as u64)),
            metadata,
            path_key: entry.get("path_key").and_then(|v| v.as_str()).map(String::from),
            layers: crate::read_layers(entry.get("layers")),
        });
    }
    Ok(tequila_scene::reimport::ReimportPayload {
        added,
        updates,
        hidden: value
            .get("hidden")
            .map(|r| r.as_array())
            .unwrap_or(&[])
            .iter()
            .filter_map(|v| v.as_index())
            .map(|v| NodeId(v as u64))
            .collect(),
    })
}

fn node_to_json(node: &SceneNode) -> Value {
    let mut out = Object::new();
    out.set("id", node.id.0)
        .set("name", node.name.clone())
        .set("kind", node.kind.label())
        .set_some("parent", node.parent.map(|p| p.0))
        .set("transform", matrix(node.transform))
        .set("visible", node.visible)
        .set("locked", node.locked)
        .set("children", ids(&node.children))
        .set_some("material", node.material.map(|m| m.0))
        .set_some("path_key", node.path_key.clone())
        .set_some(
            "pivot",
            node.pivot.map(|p| Value::from(vec![p.x, p.y, p.z])),
        );
    if let NodeKind::Part { mesh } = node.kind {
        out.set("mesh", mesh.0);
    }
    if !node.metadata.is_empty() {
        out.set(
            "metadata",
            Value::Object(
                node.metadata
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                    .collect(),
            ),
        );
    }
    if !node.layers.is_empty() {
        out.set("layers", crate::layers_to_json(&node.layers));
    }
    out.build()
}

fn import_payload_from_json(value: &Value) -> Result<ImportPayload, FormatError> {
    let mut meshes = Vec::new();
    for entry in value.get("meshes").map(|m| m.as_array()).unwrap_or(&[]) {
        let key = entry
            .get("key")
            .and_then(|v| v.as_index())
            .ok_or_else(|| FormatError::Invalid("a journalled mesh has no key".into()))?;
        let data = entry
            .get("data")
            .and_then(|v| v.as_str())
            .and_then(base64_decode)
            .ok_or_else(|| FormatError::Invalid("a journalled mesh has no data".into()))?;
        let mesh: Mesh =
            fsmesh::read(&data).map_err(|e| FormatError::Mesh(format!("journalled mesh: {e}")))?;
        meshes.push((MeshKey(key as u64), mesh));
    }

    let mut materials = Vec::new();
    for entry in value.get("materials").map(|m| m.as_array()).unwrap_or(&[]) {
        let key = entry
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(|s| crate::parse_id("mat_", s))
            .ok_or_else(|| FormatError::Invalid("a journalled material has no id".into()))?;
        materials.push((
            MaterialKey(key),
            crate::read_material(entry, "journalled material")?,
        ));
    }

    let mut nodes = Vec::new();
    for entry in value.get("nodes").map(|n| n.as_array()).unwrap_or(&[]) {
        nodes.push(node_from_json(entry)?);
    }

    let mut textures = Vec::new();
    for entry in value.get("textures").map(|t| t.as_array()).unwrap_or(&[]) {
        let (Some(id), Some(path)) = (
            entry.get("id").and_then(|v| v.as_index()),
            entry.get("path").and_then(|v| v.as_str()),
        ) else {
            return Err(FormatError::Invalid("a journalled texture has no id or path".into()));
        };
        textures.push((
            tequila_materials::graph::TextureId(id as u32),
            TextureSource {
                path: PathBuf::from(path),
                role: entry
                    .get("role")
                    .and_then(|v| v.as_str())
                    .and_then(role_from_name)
                    .unwrap_or(MapRole::Color),
                color_space: entry
                    .get("color_space")
                    .and_then(|v| v.as_str())
                    .and_then(color_space_from_name),
            },
        ));
    }

    Ok(ImportPayload {
        meshes,
        textures,
        materials,
        nodes,
        roots: value
            .get("roots")
            .map(|r| r.as_array())
            .unwrap_or(&[])
            .iter()
            .filter_map(|v| v.as_index())
            .map(|v| NodeId(v as u64))
            .collect(),
    })
}

fn node_from_json(value: &Value) -> Result<SceneNode, FormatError> {
    let id = value
        .get("id")
        .and_then(|v| v.as_index())
        .ok_or_else(|| FormatError::Invalid("a journalled node has no id".into()))?
        as u64;
    let kind = match value.get("kind").and_then(|v| v.as_str()) {
        Some("Part") => NodeKind::Part {
            mesh: MeshKey(
                value
                    .get("mesh")
                    .and_then(|v| v.as_index())
                    .ok_or_else(|| FormatError::Invalid("a Part names no mesh".into()))?
                    as u64,
            ),
        },
        _ => NodeKind::Group,
    };
    let mut node = SceneNode::new(
        NodeId(id),
        value.get("name").and_then(|v| v.as_str()).unwrap_or("Node"),
        kind,
    );
    node.parent = value
        .get("parent")
        .and_then(|v| v.as_index())
        .map(|p| NodeId(p as u64));
    if value.get("transform").is_some() {
        node.transform = read_matrix(value)?;
    }
    node.visible = value
        .get("visible")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    node.locked = value
        .get("locked")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    node.material = value
        .get("material")
        .and_then(|v| v.as_index())
        .map(|m| MaterialKey(m as u64));
    node.children = value
        .get("children")
        .map(|c| c.as_array())
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| v.as_index())
        .map(|c| NodeId(c as u64))
        .collect();
    node.path_key = value
        .get("path_key")
        .and_then(|v| v.as_str())
        .map(String::from);
    node.pivot = value
        .get("pivot")
        .and_then(|v| v.as_f32_array())
        .filter(|v| v.len() == 3)
        .map(|v| tequila_core::math::vec3(v[0], v[1], v[2]));
    if let Some(Value::Object(map)) = value.get("metadata") {
        for (key, item) in map {
            if let Some(text) = item.as_str() {
                node.metadata.insert(key.clone(), text.to_string());
            }
        }
    }
    node.layers = crate::read_layers(value.get("layers"));
    Ok(node)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - i * 6)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [0xffu8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut accumulator, mut bits) = (0u32, 0u32);
    for byte in text.bytes() {
        if byte == b'=' {
            continue;
        }
        let value = lookup[byte as usize];
        if value == 0xff {
            return None;
        }
        accumulator = (accumulator << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((accumulator >> bits) as u8);
        }
    }
    Some(out)
}

// --- history markers ----------------------------------------------------

/// Something the journal records that is not a command: what the undo
/// history did.
///
/// Without these the journal was a list of everything ever applied, and
/// recovery replayed all of it — so an edit undone before a crash came back
/// after it, and a batch that failed and was taken back reappeared in full.
/// Replaying the history's moves through the same bus reproduces the document
/// the person was actually looking at, and the undo history they had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Undo,
    Redo,
    BeginGroup,
    EndGroup,
    AbandonGroup,
}

impl Marker {
    const ALL: [Marker; 5] = [
        Marker::Undo,
        Marker::Redo,
        Marker::BeginGroup,
        Marker::EndGroup,
        Marker::AbandonGroup,
    ];

    fn name(self) -> &'static str {
        match self {
            Marker::Undo => "undo",
            Marker::Redo => "redo",
            Marker::BeginGroup => "begin_group",
            Marker::EndGroup => "end_group",
            Marker::AbandonGroup => "abandon_group",
        }
    }
}

/// A marker as a journal line. A different key from a command's, so an older
/// build reading a newer journal refuses these lines rather than
/// misreading them.
pub fn encode_marker(marker: Marker) -> String {
    format!("{{\"history\":\"{}\"}}", marker.name())
}

/// The marker on a line, if the line is one.
pub fn decode_marker(line: &str) -> Option<Marker> {
    if !line.trim_start().starts_with("{\"history\"") {
        return None;
    }
    let value = crate::json::parse(line.as_bytes()).ok()?;
    let name = value.get("history")?.as_str()?;
    Marker::ALL.into_iter().find(|m| m.name() == name)
}

/// A cut in the journal: where a snapshot written on another thread copied
/// the document. Commands after it are the ones that snapshot lacks.
pub fn encode_cut(cut: u64) -> String {
    format!("{{\"cut\":{cut}}}")
}

/// The cut on a line, if the line is one.
pub fn decode_cut(line: &str) -> Option<u64> {
    let rest = line.trim().strip_prefix("{\"cut\":")?;
    rest.strip_suffix('}')?.trim().parse().ok()
}

/// What `{id}.origin` says: the file the journal continues from, and the
/// cut after which its lines apply - absent when all of them do.
fn read_origin(path: &Path) -> (Option<PathBuf>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (None, None);
    };
    let mut lines = text.lines();
    let document = lines.next().map(|l| PathBuf::from(l.trim())).filter(|p| !p.as_os_str().is_empty());
    let cut = lines.find_map(|l| l.trim().strip_prefix("cut ").and_then(|n| n.trim().parse().ok()));
    (document, cut)
}

/// The origin marker beside a journal: `{id}.origin` for `{id}.jsonl`.
fn origin_beside(journal: &Path) -> PathBuf {
    journal.with_extension("origin")
}

// --- the session --------------------------------------------------------

/// A crash-recovery session: a journal file that is appended to as commands
/// are applied, and periodic full snapshots.
#[derive(Debug)]
pub struct Session {
    directory: PathBuf,
    id: String,
    journal: PathBuf,
    /// Snapshots are rotated so a corrupt one does not cost everything.
    snapshot_index: usize,
    snapshots: usize,
    /// `{id}.lock`, locked for as long as the session runs. The operating
    /// system lets go of it when the process ends, however it ends, so a
    /// lock another window cannot take is a session still running there:
    /// not a crash to offer, and not files to clean up. `None` where the
    /// file system will not lock, which is as recovery was before.
    lock: Option<std::fs::File>,
    /// The journal, kept open for appending: opened and closed for every
    /// command, it cost a file open and a virus scan per orbit frame, since
    /// every frame of an orbit is a camera step. `None` if it would not
    /// open, when each append opens it as before.
    file: Option<std::fs::File>,
    /// The last cut made in the journal, numbering them.
    cuts: u64,
    /// The snapshot recovery would start from now, if one: never written
    /// over by the next one.
    origin: Option<PathBuf>,
    /// A snapshot being written on another thread: its slot is taken.
    in_flight: Option<PathBuf>,
}

/// A snapshot to be written on another thread, from a copy of the document
/// made when this was begun. See [`Session::begin_snapshot`].
#[derive(Clone, Debug, PartialEq)]
pub struct PendingSnapshot {
    path: PathBuf,
    cut: u64,
}

impl PendingSnapshot {
    /// Where to write it.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// What a recoverable session looks like from the outside, for the dialog that
/// offers to restore it.
#[derive(Clone, Debug, PartialEq)]
pub struct Recoverable {
    pub id: String,
    pub journal: PathBuf,
    /// The newest snapshot, if one was written.
    pub snapshot: Option<PathBuf>,
    /// The state the journal continues from: the last saved project, or the
    /// last auto-save snapshot, whichever came later.
    ///
    /// Recovery needs this because the journal holds only the commands *since*
    /// that point. Replaying them onto anything else reproduces a scene that
    /// never existed — and replaying them onto a stale save silently discards
    /// everything the snapshot in between had already captured.
    pub document: Option<PathBuf>,
    pub commands: usize,
    /// The project the person was working on, when it had been saved or
    /// opened — the name to show them. `document` can be an auto-save
    /// snapshot with an internal name nobody recognises; this is the file
    /// they know.
    pub project: Option<PathBuf>,
    /// When the snapshot the journal continues from was written, if it
    /// continues from one. Work captured in a snapshot is recovered from it,
    /// not from the journal, so `commands` alone undercounts it — to zero,
    /// right after an auto-save.
    pub snapshot_time: Option<std::time::SystemTime>,
}

impl Session {
    /// Open a session under `directory` (`~/.tequila/journal` in the app).
    ///
    /// The session ID must be unique per run, or two Tequila windows would
    /// interleave their commands into one journal and recovery would replay a
    /// scene that never existed.
    pub fn open(directory: &Path, id: &str, snapshots: usize) -> Result<Session, FormatError> {
        std::fs::create_dir_all(directory).map_err(|e| {
            FormatError::Io(format!("{} could not be created: {e}", directory.display()))
        })?;
        let journal = directory.join(format!("{id}.jsonl"));
        // Created eagerly, so a crash before the first command still leaves a
        // session the recovery scan can see and clean up.
        std::fs::File::create(&journal).map_err(|e| {
            FormatError::Io(format!("{} could not be opened: {e}", journal.display()))
        })?;
        let lock = std::fs::File::create(directory.join(format!("{id}.lock")))
            .ok()
            .filter(|file| file.try_lock().is_ok());
        let file = std::fs::OpenOptions::new().append(true).open(&journal).ok();
        Ok(Session {
            directory: directory.to_path_buf(),
            id: id.to_string(),
            journal,
            snapshot_index: 0,
            // Clamped to what [`recoverable`] will look for, so a session
            // cannot write a snapshot the recovery scan never finds.
            snapshots: snapshots.clamp(1, MAX_SNAPSHOTS),
            lock,
            file,
            cuts: 0,
            origin: None,
            in_flight: None,
        })
    }

    pub fn journal_path(&self) -> &Path {
        &self.journal
    }

    /// Let go of the lock as a process that crashed does, keeping the files.
    #[cfg(test)]
    fn crash(&self) {
        if let Some(lock) = &self.lock {
            let _ = lock.unlock();
        }
    }

    /// Append one command, and flush.
    ///
    /// Flushed on every command rather than buffered, because the whole point
    /// is the commands applied in the seconds before a crash — which is
    /// exactly what a buffer still holds.
    pub fn record(&self, command: &Command) -> Result<(), FormatError> {
        self.append(&encode(command))
    }

    /// Append a command already encoded with [`encode`].
    pub fn record_line(&self, line: &str) -> Result<(), FormatError> {
        self.append(line)
    }

    /// Append what the undo history did. See [`Marker`].
    pub fn record_marker(&self, marker: Marker) -> Result<(), FormatError> {
        self.append(&encode_marker(marker))
    }

    fn append(&self, line: &str) -> Result<(), FormatError> {
        // One write per line, so a crash leaves whole lines and at most one
        // torn one at the end. Appending: a rebase that empties the file
        // under the open handle is written after, not over.
        let mut text = String::with_capacity(line.len() + 1);
        text.push_str(line);
        text.push('\n');
        if let Some(mut file) = self.file.as_ref() {
            if file.write_all(text.as_bytes()).and_then(|_| file.flush()).is_ok() {
                return Ok(());
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.journal)
            .map_err(|e| FormatError::Io(format!("the journal could not be opened: {e}")))?;
        file.write_all(text.as_bytes())
            .and_then(|_| file.flush())
            .map_err(|e| FormatError::Io(format!("the journal could not be written: {e}")))
    }

    /// Record the state the journal now continues from, and start it over.
    ///
    /// Called after a save, an open, or an auto-save snapshot. In each case
    /// some file on disk now holds everything the journal described, so
    /// replaying it as well would apply those commands a second time — and
    /// leaving the marker pointing at an older file would replay them onto a
    /// state that is missing the work in between.
    pub fn rebase(&self, document: Option<&Path>) -> Result<(), FormatError> {
        let marker = self.directory.join(format!("{}.origin", self.id));
        let project = self.directory.join(format!("{}.project", self.id));
        let remove = |path: &Path| match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
        match document {
            Some(path) => write_whole(&marker, path.to_string_lossy().as_bytes()).and_then(|_| {
                // The person's own file, remembered apart from the origin: an
                // auto-save snapshot moves the origin into this directory,
                // and the prompt should still name the project.
                if path.parent() == Some(self.directory.as_path()) {
                    Ok(())
                } else {
                    std::fs::write(&project, path.to_string_lossy().as_bytes())
                }
            }),
            // An unsaved document has no origin; a stale marker would send
            // recovery to a file this session is no longer editing.
            None => remove(&marker).and_then(|_| remove(&project)),
        }
        .map_err(|e| FormatError::Io(format!("the session marker could not be written: {e}")))?;

        std::fs::write(&self.journal, b"")
            .map_err(|e| FormatError::Io(format!("the journal could not be reset: {e}")))
    }

    /// Begin a snapshot to be written on another thread from a copy of the
    /// document made now: a cut goes into the journal, and the snapshot's
    /// slot is chosen - never the one recovery starts from, nor one being
    /// written. Until [`Session::adopt_snapshot`], recovery still starts from
    /// the old origin and replays every line, so a crash while it is being
    /// written loses nothing.
    pub fn begin_snapshot(&mut self) -> Result<PendingSnapshot, FormatError> {
        if self.in_flight.is_some() {
            return Err(FormatError::Io("a snapshot is being written already".into()));
        }
        let path = self.next_slot();
        let cut = self.cuts + 1;
        self.append(&encode_cut(cut))?;
        self.cuts = cut;
        self.in_flight = Some(path.clone());
        Ok(PendingSnapshot { path, cut })
    }

    /// The snapshot `pending` is written whole: recovery starts from it, and
    /// replays only what the journal has after its cut. One write of the
    /// marker, whole or not at all, so a crash on either side of it recovers
    /// exactly once. Only for a snapshot whose cut is still in the journal:
    /// nothing may have rebased it since [`Session::begin_snapshot`].
    pub fn adopt_snapshot(&mut self, pending: &PendingSnapshot) -> Result<(), FormatError> {
        self.in_flight = None;
        let marker = self.directory.join(format!("{}.origin", self.id));
        write_whole(&marker, format!("{}\ncut {}\n", pending.path.display(), pending.cut).as_bytes())
            .map_err(|e| FormatError::Io(format!("the session marker could not be written: {e}")))?;
        self.origin = Some(pending.path.clone());
        Ok(())
    }

    /// Give up a snapshot begun with [`Session::begin_snapshot`]: it failed,
    /// or the journal moved on while it was written. Recovery never pointed
    /// at it; its file goes.
    pub fn abandon_snapshot(&mut self, pending: &PendingSnapshot) {
        self.in_flight = None;
        if self.origin.as_deref() != Some(pending.path.as_path()) {
            let _ = std::fs::remove_file(&pending.path);
        }
    }

    /// The next snapshot slot, passing over the one recovery starts from and
    /// one being written.
    fn next_slot(&mut self) -> PathBuf {
        for _ in 0..self.snapshots {
            let path = self.directory.join(format!("{}.{}.tequila", self.id, self.snapshot_index));
            self.snapshot_index = (self.snapshot_index + 1) % self.snapshots;
            if Some(&path) != self.origin.as_ref() && Some(&path) != self.in_flight.as_ref() {
                return path;
            }
        }
        // Too few slots to pass over both: the rotation as it always was.
        let path = self.directory.join(format!("{}.{}.tequila", self.id, self.snapshot_index));
        self.snapshot_index = (self.snapshot_index + 1) % self.snapshots;
        path
    }

    /// Write a full snapshot, rotating through `snapshots` files.
    ///
    /// Rotating rather than overwriting: a crash *during* the snapshot write
    /// would otherwise destroy the only copy, which is the one moment the
    /// user most needs it. `save` writes to a temporary and renames, so the
    /// window is small — but small is not none.
    pub fn snapshot(&mut self, document: &SceneDocument) -> Result<PathBuf, FormatError> {
        let path = self.next_slot();
        crate::save(&path, document, &crate::Preserved::default())?;
        // The snapshot is now the state the journal continues from — not the
        // project file, which may be an hour behind it. Pointing recovery at
        // the older one would throw away everything the snapshot captured.
        self.rebase(Some(&path))?;
        self.origin = Some(path.clone());
        Ok(path)
    }

    /// Delete this session's files. Called on a clean exit: a journal left
    /// behind is a crash-recovery prompt the user did not need.
    pub fn discard(mut self) {
        drop(self.file.take());
        drop(self.lock.take());
        let _ = std::fs::remove_file(self.directory.join(format!("{}.writing", self.id)));
        let _ = std::fs::remove_file(self.directory.join(format!("{}.lock", self.id)));
        let _ = std::fs::remove_file(&self.journal);
        let _ = std::fs::remove_file(self.directory.join(format!("{}.origin", self.id)));
        let _ = std::fs::remove_file(self.directory.join(format!("{}.project", self.id)));
        for i in 0..self.snapshots {
            let _ = std::fs::remove_file(self.directory.join(format!("{}.{i}.tequila", self.id)));
        }
    }
}

/// Sessions in `directory` that were not discarded — that is, that crashed.
///
/// Sessions with nothing in them are cleaned up rather than offered: a journal
/// with no commands recovers nothing, and prompting about it teaches the user
/// to dismiss the prompt.
pub fn recoverable(directory: &Path, exclude: &str) -> Vec<Recoverable> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // This session, or one running in another window: its journal is
        // live, and deleting or offering it would lose that window's work.
        if id == exclude || running(directory, id) {
            continue;
        }
        let session = describe(directory, id);
        if session.commands == 0 && session.snapshot.is_none() {
            forget(directory, id);
            continue;
        }
        out.push(session);
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// What the session `id` in `directory` left, running or not.
pub fn describe(directory: &Path, id: &str) -> Recoverable {
    let journal = directory.join(format!("{id}.jsonl"));
    // Counted in bytes: a crash part way through a multi-byte character
    // leaves a journal that is not UTF-8, and it still holds every command
    // before that line.
    let (origin, cut) = read_origin(&directory.join(format!("{id}.origin")));
    let commands = std::fs::read(&journal)
        .map(|bytes| lines_after(&String::from_utf8_lossy(&bytes), cut).filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    let snapshot = (0..MAX_SNAPSHOTS)
        .map(|i| directory.join(format!("{id}.{i}.tequila")))
        .filter(|p| p.exists())
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let document = origin.filter(|path| path.exists());
    let project = std::fs::read_to_string(directory.join(format!("{id}.project")))
        .ok()
        .map(|text| PathBuf::from(text.trim()))
        .filter(|path| !path.as_os_str().is_empty());
    let snapshot_time = document
        .as_ref()
        .filter(|document| Some(*document) == snapshot.as_ref())
        .and_then(|snapshot| std::fs::metadata(snapshot).and_then(|m| m.modified()).ok());
    Recoverable { id: id.to_string(), journal, snapshot, document, commands, project, snapshot_time }
}

/// Whether the session `id` is still running, in this process or another:
/// its lock is held. A session from before locks, or one whose lock file
/// could not be made, reads as not running, as every session did then.
fn running(directory: &Path, id: &str) -> bool {
    let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(directory.join(format!("{id}.lock"))) else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// Delete every file a crashed session left: its journal, its origin marker,
/// and *all* of its rotated auto-save snapshots.
///
/// All of them, not the newest. The recovery prompt's Discard used to remove
/// only the snapshot it had offered, and a session rotates through up to
/// three — so every discard left one or two full copies of the scene behind,
/// with no journal to make them recoverable and nothing to ever delete them.
/// A month of that was twenty orphaned files and a hundred megabytes.
pub fn forget(directory: &Path, id: &str) {
    let _ = std::fs::remove_file(directory.join(format!("{id}.lock")));
    let _ = std::fs::remove_file(directory.join(format!("{id}.jsonl")));
    let _ = std::fs::remove_file(directory.join(format!("{id}.origin")));
    let _ = std::fs::remove_file(directory.join(format!("{id}.project")));
    let _ = std::fs::remove_file(directory.join(format!("{id}.writing")));
    for i in 0..MAX_SNAPSHOTS {
        let _ = std::fs::remove_file(directory.join(format!("{id}.{i}.tequila")));
    }
}

/// A journal's lines that apply after the cut `cut` (all of them without
/// one), cut lines left out.
fn lines_after(text: &str, cut: Option<u64>) -> impl Iterator<Item = &str> {
    let skip = cut.map_or(0, |cut| text.lines().position(|l| decode_cut(l) == Some(cut)).map_or(usize::MAX, |p| p + 1));
    text.lines().skip(skip).filter(|l| decode_cut(l).is_none())
}

/// Write `bytes` to `path` whole or not at all: to a file beside it, then
/// moved over it, so a crash part way leaves the old one.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("writing");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

/// What a recovery produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recovery {
    pub replayed: usize,
    /// Lines that could not be applied, with why. A journal can end mid-line
    /// if the crash landed during a write, and a later command can refuse if
    /// an earlier one was lost — neither is a reason to abandon the rest.
    pub skipped: Vec<String>,
}

/// Replay a journal onto a document, applying each command through the bus so
/// the recovered session has a working undo history.
pub fn replay(
    document: &mut SceneDocument,
    bus: &mut CommandBus,
    journal: &Path,
) -> Result<Recovery, FormatError> {
    // Lossy: a line torn part way through a multi-byte character fails to
    // decode as a command and is skipped, rather than costing every line.
    let bytes = std::fs::read(journal)
        .map_err(|e| FormatError::Io(format!("{} could not be read: {e}", journal.display())))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut recovery = Recovery::default();
    // Continuing from a snapshot written on another thread: only what came
    // after its cut is missing from it.
    let (_, cut) = read_origin(&origin_beside(journal));
    if let Some(cut) = cut {
        if !text.lines().any(|l| decode_cut(l) == Some(cut)) {
            recovery.skipped.push(format!("the journal's cut {cut} is gone, so nothing after the snapshot was replayed"));
            return Ok(recovery);
        }
    }
    let skip = cut.map_or(0, |cut| text.lines().position(|l| decode_cut(l) == Some(cut)).map_or(0, |p| p + 1));

    for (number, line) in text.lines().enumerate().skip(skip) {
        if line.trim().is_empty() || decode_cut(line).is_some() {
            continue;
        }
        if let Some(marker) = decode_marker(line) {
            let moved = match marker {
                Marker::Undo => bus.undo(document).map(|_| ()),
                Marker::Redo => bus.redo(document).map(|_| ()),
                Marker::BeginGroup => {
                    bus.begin_group();
                    Ok(())
                }
                Marker::EndGroup => {
                    bus.end_group();
                    Ok(())
                }
                Marker::AbandonGroup => bus.abandon_group(document).map(|_| ()),
            };
            if let Err(error) = moved {
                recovery
                    .skipped
                    .push(format!("line {}: {} could not be replayed: {error}", number + 1, marker.name()));
            }
            continue;
        }
        match decode(line) {
            Ok(command) => match bus.apply(document, command) {
                Ok(_) => recovery.replayed += 1,
                Err(error) => recovery
                    .skipped
                    .push(format!("line {}: {error}", number + 1)),
            },
            Err(error) => recovery
                .skipped
                .push(format!("line {}: {error}", number + 1)),
        }
    }
    Ok(recovery)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_render_core::settings::LightingPreset;
    use tequila_core::units::LengthUnit;

    /// A preset is named however it is capitalised, and a name that is no
    /// preset is refused. Read as Product, a script asking for "jewelry" got
    /// product lighting and no word of it: no caustics, no ground bounce and
    /// half the transmission depth, on exactly the scene that needed them.
    #[test]
    fn a_lighting_preset_is_named_in_any_case_and_a_wrong_name_is_refused() {
        match decode(r#"{"command":"lighting.set_preset","preset":"jewelry"}"#) {
            Ok(Command::SetLightingPreset { preset }) => assert_eq!(preset, LightingPreset::Jewelry),
            other => panic!("{other:?}"),
        }
        assert!(decode(r#"{"command":"lighting.set_preset","preset":"sparkly"}"#).is_err());
    }

    /// A cut reads back as itself and nothing else does; an origin marker
    /// from before cuts - one line, the file - still reads, with no cut.
    #[test]
    fn cuts_and_origins_read_back() {
        assert_eq!(decode_cut(&encode_cut(12)), Some(12));
        assert_eq!(decode_cut(r#"{"history":"undo"}"#), None);
        assert_eq!(decode_cut(r#"{"command":"scene.rename","node":1,"name":"{\"cut\":3}"}"#), None);
        let directory = std::env::temp_dir().join(format!("tequila-origin-read-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let marker = directory.join("s.origin");
        std::fs::write(&marker, "C:/work/kettle.tequila").unwrap();
        assert_eq!(read_origin(&marker), (Some(PathBuf::from("C:/work/kettle.tequila")), None));
        write_whole(&marker, b"C:/j/s.1.tequila\ncut 4\n").unwrap();
        assert_eq!(read_origin(&marker), (Some(PathBuf::from("C:/j/s.1.tequila")), Some(4)));
        assert!(!directory.join("s.writing").exists(), "the whole-write left its temporary");
        let text = "a\n{\"cut\":1}\nb\n{\"cut\":2}\nc\n";
        assert_eq!(lines_after(text, None).collect::<Vec<_>>(), ["a", "b", "c"]);
        assert_eq!(lines_after(text, Some(1)).collect::<Vec<_>>(), ["b", "c"]);
        assert_eq!(lines_after(text, Some(9)).count(), 0, "a cut that is not there counts nothing");
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A material colour may be sent as hex sRGB, as a person or an AI app
    /// picks one: converted as a picker converts it, and recorded converted,
    /// so it reads back the same.
    #[test]
    fn a_material_colour_may_be_hex_srgb() {
        let command = decode(r##"{"command":"material.set_parameter","material":1,"parameter":"base_color","value":"#c0392b"}"##).expect("decodes");
        let Command::SetMaterialParameter { parameter, .. } = &command else { panic!("{command:?}") };
        assert_eq!(parameter.value, ParamValue::Color(crate::hex_colour("#c0392b").unwrap()));
        assert_eq!(decode(&encode(&command)).expect("reads back"), command);
        assert!(decode(r##"{"command":"material.set_parameter","material":1,"parameter":"base_color","value":"crimson"}"##).is_err());
    }

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("tequila-journal-test-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn imported() -> (SceneDocument, CommandBus, Vec<NodeId>, MaterialKey) {
        crate::examples::imported()
    }

    #[test]
    fn labels_survive_undo_redo_and_project_reopening() {
        let (mut document, mut bus, _, material) = imported();
        let directory = scratch("labels");
        let artwork = directory.join("artwork.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&artwork).unwrap(), 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[255,0,0,255, 0,0,255,0]).unwrap();
        writer.finish().unwrap();
        let add = Command::AddDecal { material, name: "Logo".into(),
            source: TextureSource::new(&artwork, MapRole::Color) };
        bus.apply(&mut document, decode(&encode(&add)).unwrap()).unwrap();
        bus.end_interaction();
        let mut decals = document.materials.get(material).unwrap().graph.decals.clone();
        decals[0].opacity = 0.625;
        decals[0].mapping.offset.y = -0.25;
        bus.apply(&mut document, Command::SetDecals { material, decals: decals.clone() }).unwrap();
        bus.end_interaction();
        bus.undo(&mut document).unwrap();
        assert_eq!(document.materials.get(material).unwrap().graph.decals[0].opacity, 1.0);
        bus.undo(&mut document).unwrap();
        assert!(document.materials.get(material).unwrap().graph.decals.is_empty());
        bus.redo(&mut document).unwrap();
        bus.redo(&mut document).unwrap();
        document.collect_garbage();
        let path = directory.join("labels.tequila");
        crate::save(&path, &document, &crate::Preserved::default()).unwrap();
        let loaded = crate::load(&path).unwrap();
        assert_eq!(loaded.document.materials.get(material).unwrap().graph.decals, decals);
        assert!(loaded.document.textures.get(decals[0].texture).is_some());
        let source = loaded.document.textures.get(decals[0].texture).unwrap();
        let image = tequila_texture::Image::load(&source.path).unwrap();
        assert_eq!(image.channels, 4);
        assert_eq!(image.data[3], 1.0);
        assert_eq!(image.data[7], 0.0);
    }

    /// Every command in the catalog has to survive the journal, or recovery
    /// silently drops whichever one was forgotten.
    #[test]
    fn every_command_round_trips_through_the_journal() {

        let commands = crate::examples::commands();

        // Every catalog entry must appear above.
        let covered: std::collections::BTreeSet<&str> = commands.iter().map(|c| c.name()).collect();
        for name in Command::CATALOG {
            assert!(covered.contains(name), "`{name}` is not journal-tested");
        }

        for command in commands {
            let line = encode(&command);
            assert!(
                !line.contains('\n'),
                "a journal record must be one line: {line}"
            );
            let back = decode(&line).unwrap_or_else(|e| panic!("{}: {e}", command.name()));
            assert_eq!(back, command, "`{}` did not survive", command.name());
        }
    }

    /// The end-to-end claim of §4.5: crash, replay, same document.
    #[test]
    fn replaying_a_journal_reproduces_the_lost_work() {
        let directory = scratch("replay");
        let session = Session::open(&directory, "session-a", 2).unwrap();

        let (mut document, mut bus, ids, material) = imported();
        let saved = document.clone();

        // An hour's work, journalled as it happens.
        for command in [
            Command::Group {
                nodes: vec![ids[1], ids[2]],
                name: "Fasteners".into(),
            },
            Command::Rename {
                node: ids[1],
                name: "Housing, upper".into(),
            },
            Command::set_visibility(ids[2], false),
            Command::AssignMaterial {
                nodes: vec![ids[1]],
                material,
            },
            Command::SetUnits {
                unit: LengthUnit::Millimetre,
                mode: ConversionMode::KeepSceneSize,
            },
        ] {
            bus.apply(&mut document, command.clone()).unwrap();
            session.record(&command).unwrap();
        }

        // Crash. Reopen the last saved state and replay.
        let mut recovered = saved;
        let mut recovered_bus = CommandBus::new();
        let recovery = replay(&mut recovered, &mut recovered_bus, session.journal_path()).unwrap();

        assert_eq!(recovery.replayed, 5);
        assert!(recovery.skipped.is_empty(), "{:?}", recovery.skipped);
        if let Some(difference) = recovered.difference_from(&document) {
            panic!("recovery did not reproduce the work: {difference}");
        }

        // And the recovered session can undo, because replay went through the
        // bus rather than around it.
        assert!(recovered_bus.can_undo());
        assert_eq!(recovered_bus.history_names().len(), 5);
    }

    /// Apply a command to the live document and journal it, as the
    /// application does.
    fn live(session: &Session, document: &mut SceneDocument, bus: &mut CommandBus, command: Command) {
        bus.apply(document, command.clone()).unwrap();
        session.record(&command).unwrap();
    }

    /// Crash now: replay the journal onto the state it started from, and
    /// demand the document the person was looking at.
    #[track_caller]
    fn crash_and_recover(session: &Session, start: &SceneDocument, expected: &SceneDocument) -> CommandBus {
        let mut recovered = start.clone();
        let mut bus = CommandBus::new();
        let recovery = replay(&mut recovered, &mut bus, session.journal_path()).unwrap();
        assert!(recovery.skipped.is_empty(), "{:?}", recovery.skipped);
        if let Some(difference) = recovered.difference_from(expected) {
            panic!("recovery did not reproduce the document as it was: {difference}");
        }
        bus
    }

    /// The bug this exists for: undo was never written down, so an edit
    /// undone before a crash came back after it.
    #[test]
    fn an_undone_edit_stays_undone_after_a_crash() {
        let directory = scratch("undo-marker");
        let session = Session::open(&directory, "undo", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();
        let start = document.clone();

        live(&session, &mut document, &mut bus, Command::Rename { node: ids[1], name: "Kept".into() });
        live(&session, &mut document, &mut bus, Command::Rename { node: ids[2], name: "Regretted".into() });
        bus.undo(&mut document).unwrap();
        session.record_marker(Marker::Undo).unwrap();

        let recovered = crash_and_recover(&session, &start, &document);
        assert!(recovered.can_redo(), "and the undone edit is still there to redo");
    }

    /// A batch is one step, and undoing it after a crash must undo all of it —
    /// which needs the group's edges in the journal, not just its commands.
    #[test]
    fn a_batch_undone_before_a_crash_stays_undone() {
        let directory = scratch("group-marker");
        let session = Session::open(&directory, "group", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();
        let start = document.clone();

        bus.begin_group();
        session.record_marker(Marker::BeginGroup).unwrap();
        live(&session, &mut document, &mut bus, Command::Rename { node: ids[1], name: "One".into() });
        live(&session, &mut document, &mut bus, Command::Rename { node: ids[2], name: "Two".into() });
        bus.end_group();
        session.record_marker(Marker::EndGroup).unwrap();
        bus.undo(&mut document).unwrap();
        session.record_marker(Marker::Undo).unwrap();

        crash_and_recover(&session, &start, &document);
    }

    /// A batch that failed and was taken back must not reappear.
    #[test]
    fn an_abandoned_batch_does_not_come_back() {
        let directory = scratch("abandon-marker");
        let session = Session::open(&directory, "abandon", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();
        let start = document.clone();

        live(&session, &mut document, &mut bus, Command::Rename { node: ids[1], name: "Before".into() });
        bus.begin_group();
        session.record_marker(Marker::BeginGroup).unwrap();
        live(&session, &mut document, &mut bus, Command::Rename { node: ids[2], name: "Half".into() });
        bus.abandon_group(&mut document).unwrap();
        session.record_marker(Marker::AbandonGroup).unwrap();

        crash_and_recover(&session, &start, &document);
        assert_eq!(document.tree.get(ids[1]).unwrap().name, "Before");
    }

    #[test]
    fn a_redo_is_replayed_too() {
        let directory = scratch("redo-marker");
        let session = Session::open(&directory, "redo", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();
        let start = document.clone();

        live(&session, &mut document, &mut bus, Command::Rename { node: ids[1], name: "Back".into() });
        bus.undo(&mut document).unwrap();
        session.record_marker(Marker::Undo).unwrap();
        bus.redo(&mut document).unwrap();
        session.record_marker(Marker::Redo).unwrap();

        crash_and_recover(&session, &start, &document);
    }

    #[test]
    fn markers_round_trip_and_are_not_commands() {
        for marker in Marker::ALL {
            let line = encode_marker(marker);
            assert_eq!(decode_marker(&line), Some(marker), "{line}");
            assert!(decode(&line).is_err(), "a marker must never read as a command: {line}");
        }
        assert_eq!(decode_marker(&encode(&Command::Ungroup { group: NodeId(3) })), None);
    }

    /// Discarding a crashed session removes all of it, not just the newest
    /// snapshot — the leak that left twenty orphaned auto-saves behind.
    #[test]
    fn forgetting_a_session_removes_every_snapshot() {
        let directory = scratch("forget");
        let (document, ..) = imported();
        let mut session = Session::open(&directory, "crashed", 3).unwrap();
        for _ in 0..3 {
            session.snapshot(&document).unwrap();
        }
        session.record(&Command::Ungroup { group: NodeId(1) }).unwrap();
        // Crash: the session is never discarded. Another one is left alone.
        std::mem::forget(session);
        let bystander = Session::open(&directory, "bystander", 3).unwrap();
        bystander.record(&Command::Ungroup { group: NodeId(2) }).unwrap();

        forget(&directory, "crashed");

        let left: Vec<String> = std::fs::read_dir(&directory)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.iter().all(|name| !name.starts_with("crashed")), "left behind: {left:?}");
        assert!(left.iter().any(|name| name.starts_with("bystander")), "another session was touched: {left:?}");
    }

    /// A crash during a write leaves a half-written last line. It must cost
    /// that one command, not the whole session.
    #[test]
    fn a_truncated_final_line_costs_only_that_command() {
        let directory = scratch("truncated");
        let session = Session::open(&directory, "session-b", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();
        let saved = document.clone();

        for command in [
            Command::Rename {
                node: ids[1],
                name: "First".into(),
            },
            Command::Rename {
                node: ids[2],
                name: "Second".into(),
            },
        ] {
            bus.apply(&mut document, command.clone()).unwrap();
            session.record(&command).unwrap();
        }
        // Chop the last line in half, as a crash mid-write would.
        let text = std::fs::read_to_string(session.journal_path()).unwrap();
        let cut = text.len() - 12;
        std::fs::write(session.journal_path(), &text[..cut]).unwrap();

        let mut recovered = saved;
        let recovery = replay(
            &mut recovered,
            &mut CommandBus::new(),
            session.journal_path(),
        )
        .unwrap();

        assert_eq!(recovery.replayed, 1, "the first command must survive");
        assert_eq!(recovery.skipped.len(), 1);
        assert_eq!(recovered.tree.get(ids[1]).unwrap().name, "First");
    }

    /// The same, cut inside a character that takes several bytes: the file
    /// is then not UTF-8 at all, and that must still cost one line.
    #[test]
    fn a_line_torn_inside_a_character_costs_only_that_command() {
        let directory = scratch("torn");
        let session = Session::open(&directory, "torn", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();
        let saved = document.clone();
        for command in [
            Command::Rename { node: ids[1], name: "Première".into() },
            Command::Rename { node: ids[2], name: "日本語の部品".into() },
        ] {
            bus.apply(&mut document, command.clone()).unwrap();
            session.record(&command).unwrap();
        }
        let bytes = std::fs::read(session.journal_path()).unwrap();
        let last = bytes.windows(3).rposition(|w| w == "語".as_bytes()).unwrap();
        std::fs::write(session.journal_path(), &bytes[..last + 1]).unwrap();
        session.crash();

        assert_eq!(recoverable(&directory, "elsewhere")[0].commands, 2, "the session reads as empty");
        let mut recovered = saved;
        let recovery = replay(&mut recovered, &mut CommandBus::new(), session.journal_path()).unwrap();
        assert_eq!((recovery.replayed, recovery.skipped.len()), (1, 1));
        assert_eq!(recovered.tree.get(ids[1]).unwrap().name, "Première");
    }

    /// A snapshot already contains everything before it, so the journal is
    /// truncated — replaying both would apply those commands twice.
    #[test]
    fn a_snapshot_clears_the_journal_it_supersedes() {
        let directory = scratch("snapshot");
        let mut session = Session::open(&directory, "session-c", 3).unwrap();
        let (mut document, mut bus, ids, _) = imported();

        let command = Command::Rename {
            node: ids[1],
            name: "Before".into(),
        };
        bus.apply(&mut document, command.clone()).unwrap();
        session.record(&command).unwrap();
        assert_eq!(count_lines(session.journal_path()), 1);

        let snapshot = session.snapshot(&document).unwrap();
        assert_eq!(count_lines(session.journal_path()), 0);

        // The snapshot is a real, loadable document holding that work.
        let loaded = crate::load(&snapshot).unwrap().document;
        assert_eq!(loaded.tree.get(ids[1]).unwrap().name, "Before");

        // And snapshots rotate rather than overwriting, so a crash during one
        // does not destroy the only copy.
        session.snapshot(&document).unwrap();
        let second = directory.join("session-c.1.tequila");
        assert!(second.exists(), "the second snapshot went to the same file");
    }

    #[test]
    fn a_crashed_session_is_offered_and_a_clean_one_is_not() {
        let directory = scratch("recoverable");
        let (mut document, mut bus, ids, _) = imported();

        let crashed = Session::open(&directory, "crashed", 2).unwrap();
        let command = Command::Rename {
            node: ids[1],
            name: "Lost work".into(),
        };
        bus.apply(&mut document, command.clone()).unwrap();
        crashed.record(&command).unwrap();
        // No `discard`: this is what a crash looks like.

        let clean = Session::open(&directory, "clean", 2).unwrap();
        clean.record(&command).unwrap();
        clean.discard();

        crashed.crash();
        let found = recoverable(&directory, "current-session");
        assert_eq!(found.len(), 1, "found {found:?}");
        assert_eq!(found[0].id, "crashed");
        assert_eq!(found[0].commands, 1);
    }

    /// An empty journal recovers nothing, and prompting about it teaches the
    /// user to dismiss the prompt.
    #[test]
    fn an_empty_session_is_cleaned_up_rather_than_offered() {
        let directory = scratch("empty");
        let session = Session::open(&directory, "did-nothing", 2).unwrap();
        let journal = session.journal_path().to_path_buf();
        assert!(journal.exists());

        session.crash();
        assert!(recoverable(&directory, "other").is_empty());
        assert!(!journal.exists(), "the empty journal was left behind");
    }

    /// The running session must never offer to recover itself.
    #[test]
    fn the_current_session_is_never_offered_for_recovery() {
        let directory = scratch("self");
        let session = Session::open(&directory, "mine", 2).unwrap();
        session
            .record(&Command::AddGroup {
                parent: None,
                name: "A".into(),
            })
            .unwrap();

        assert!(recoverable(&directory, "mine").is_empty());
        // Nor is one running in another window: its journal is live.
        assert!(recoverable(&directory, "someone-else").is_empty(), "a running session was offered");
        assert!(session.journal_path().exists(), "a running session's journal was deleted");
        session.crash();
        assert_eq!(recoverable(&directory, "someone-else").len(), 1);
    }

    /// A journal referring to nodes that are not in the saved state must skip
    /// those commands and keep the rest, rather than abandoning the recovery.
    #[test]
    fn commands_that_no_longer_apply_are_skipped_not_fatal() {
        let directory = scratch("stale");
        let session = Session::open(&directory, "stale", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();

        for command in [
            Command::Rename {
                node: NodeId(9999),
                name: "Gone".into(),
            },
            Command::Rename {
                node: ids[1],
                name: "Fine".into(),
            },
        ] {
            let _ = bus.apply(&mut document, command.clone());
            session.record(&command).unwrap();
        }

        let (mut recovered, _, _, _) = imported();
        let recovery = replay(
            &mut recovered,
            &mut CommandBus::new(),
            session.journal_path(),
        )
        .unwrap();

        assert_eq!(recovery.replayed, 1);
        assert_eq!(recovery.skipped.len(), 1);
        assert!(
            recovery.skipped[0].contains("line 1"),
            "{:?}",
            recovery.skipped
        );
        assert_eq!(recovered.tree.get(ids[1]).unwrap().name, "Fine");
    }

    /// The journal holds commands *since* the last save. After a save or an
    /// open, replaying it as well would apply those commands twice — so it
    /// starts over, and records which file it starts over from.
    #[test]
    fn saving_rebases_the_journal_onto_the_saved_file() {
        let directory = scratch("rebase");
        let session = Session::open(&directory, "rebased", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();

        let before_save = Command::Rename {
            node: ids[1],
            name: "Before saving".into(),
        };
        bus.apply(&mut document, before_save.clone()).unwrap();
        session.record(&before_save).unwrap();

        // Save, which is what rebase follows.
        let project = directory.join("work.tequila");
        crate::save(&project, &document, &crate::Preserved::default()).unwrap();
        session.rebase(Some(&project)).unwrap();
        assert_eq!(count_lines(session.journal_path()), 0);

        let after_save = Command::Rename {
            node: ids[2],
            name: "After saving".into(),
        };
        bus.apply(&mut document, after_save.clone()).unwrap();
        session.record(&after_save).unwrap();

        // Crash. Recovery finds the file, and the one command that came after.
        session.crash();
        let found = recoverable(&directory, "someone-else");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].document.as_deref(), Some(project.as_path()));
        assert_eq!(found[0].commands, 1);

        // Reopening and replaying reproduces the work exactly once.
        let mut recovered = crate::load(&project).unwrap().document;
        replay(&mut recovered, &mut CommandBus::new(), &found[0].journal).unwrap();
        if let Some(difference) = recovered.difference_from(&document) {
            panic!("recovery did not reproduce the session: {difference}");
        }
    }

    /// Opening a different project must not leave the previous one's commands
    /// in the journal, or recovery replays them onto a scene they never
    /// applied to.
    #[test]
    fn opening_another_project_clears_the_journal() {
        let directory = scratch("reopen");
        let session = Session::open(&directory, "reopened", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();

        let command = Command::Rename {
            node: ids[1],
            name: "From the first project".into(),
        };
        bus.apply(&mut document, command.clone()).unwrap();
        session.record(&command).unwrap();
        assert_eq!(count_lines(session.journal_path()), 1);

        let other = directory.join("other.tequila");
        crate::save(&other, &SceneDocument::new(), &crate::Preserved::default()).unwrap();
        session.rebase(Some(&other)).unwrap();

        assert_eq!(
            count_lines(session.journal_path()),
            0,
            "the previous project's commands survived the open"
        );

        // Work in the new project, and recovery points at *that* file.
        let second = Command::Rename {
            node: ids[2],
            name: "From the second project".into(),
        };
        bus.apply(&mut document, second.clone()).unwrap();
        session.record(&second).unwrap();

        session.crash();
        let found = recoverable(&directory, "elsewhere");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].document.as_deref(), Some(other.as_path()));
        assert_eq!(found[0].commands, 1, "only the new project's work");
    }

    /// A document that has never been saved has no origin, and a marker left
    /// from an earlier save would send recovery to the wrong file.
    #[test]
    fn a_new_unsaved_document_clears_the_origin_marker() {
        let directory = scratch("origin");
        let session = Session::open(&directory, "fresh", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();

        let saved = directory.join("saved.tequila");
        crate::save(&saved, &document, &crate::Preserved::default()).unwrap();
        session.rebase(Some(&saved)).unwrap();
        session.rebase(None).unwrap();

        let command = Command::Rename {
            node: ids[1],
            name: "Untitled work".into(),
        };
        bus.apply(&mut document, command.clone()).unwrap();
        session.record(&command).unwrap();

        session.crash();
        let found = recoverable(&directory, "elsewhere");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].document, None, "a stale origin survived");
    }

    /// Right after an auto-save the journal is empty and the work is in the
    /// snapshot. The prompt must still say which project and that there is
    /// work to recover: it named the snapshot's internal file and "0 unsaved
    /// commands", which reads as nothing to lose and invites Discard.
    #[test]
    fn a_snapshot_keeps_the_project_name_and_says_there_is_work() {
        let directory = scratch("project-name");
        let mut session = Session::open(&directory, "named", 2).unwrap();
        let (mut document, mut bus, ids, _) = imported();
        let saved = directory.parent().unwrap().join("my-product.tequila");
        crate::save(&saved, &document, &crate::Preserved::default()).unwrap();
        session.rebase(Some(&saved)).unwrap();

        bus.apply(&mut document, Command::Rename { node: ids[1], name: "Unsaved".into() }).unwrap();
        let snapshot = session.snapshot(&document).unwrap();

        session.crash();
        let found = recoverable(&directory, "elsewhere");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].commands, 0, "the snapshot emptied the journal");
        assert_eq!(found[0].document.as_ref(), Some(&snapshot));
        assert_eq!(found[0].project.as_ref(), Some(&saved), "the project's own name was lost");
        assert!(found[0].snapshot_time.is_some(), "nothing says the snapshot holds work");

        forget(&directory, "named");
        assert!(!directory.join("named.project").exists());
        let _ = std::fs::remove_file(&saved);
    }

    /// The snapshot is newer than the last save, so recovery has to continue
    /// from *it*. Replaying the journal onto the saved file instead would
    /// silently discard everything between the save and the snapshot.
    #[test]
    fn recovery_continues_from_the_snapshot_when_it_is_newer_than_the_save() {
        let directory = scratch("newer-snapshot");
        let mut session = Session::open(&directory, "autosaved", 3).unwrap();
        let (mut document, mut bus, ids, _) = imported();

        // Save, then work, then auto-save, then work again, then crash.
        let project = directory.join("work.tequila");
        crate::save(&project, &document, &crate::Preserved::default()).unwrap();
        session.rebase(Some(&project)).unwrap();

        let long_stretch = Command::Rename {
            node: ids[1],
            name: "An hour of work".into(),
        };
        bus.apply(&mut document, long_stretch.clone()).unwrap();
        session.record(&long_stretch).unwrap();

        let snapshot = session.snapshot(&document).unwrap();

        let after = Command::Rename {
            node: ids[2],
            name: "And a little more".into(),
        };
        bus.apply(&mut document, after.clone()).unwrap();
        session.record(&after).unwrap();

        session.crash();
        let found = recoverable(&directory, "elsewhere");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].document.as_deref(),
            Some(snapshot.as_path()),
            "recovery would have continued from the stale save"
        );
        assert_eq!(found[0].commands, 1);

        let base = found[0].document.clone().unwrap();
        let mut recovered = crate::load(&base).unwrap().document;
        replay(&mut recovered, &mut CommandBus::new(), &found[0].journal).unwrap();
        if let Some(difference) = recovered.difference_from(&document) {
            panic!("the hour of work was lost: {difference}");
        }
    }

    /// A graph edit with no usable graph is refused, not read as a blank
    /// graph that would replace the material's own.
    #[test]
    fn a_graph_edit_without_a_graph_is_refused() {
        for definition in [r#"{}"#, r#"{"graph":"nonsense"}"#, r#"{"graph":null}"#] {
            let line = format!(r#"{{"command":"material.set_graph","material":2,"definition":{definition}}}"#);
            assert!(decode(&line).is_err(), "accepted {definition}");
        }
    }

    /// A pattern that cannot be read is refused. Read as absent, it meant
    /// "detach": a misspelt kind stripped the pattern off the material.
    #[test]
    fn an_unreadable_procedural_is_refused_not_detached() {
        let line = r#"{"command":"material.set_procedural","material":1,"param":"base_color","seed":0,"procedural":{"kind":"granit"}}"#;
        assert!(decode(line).is_err(), "a misspelt kind was accepted");
        // Absent still detaches.
        let detach = r#"{"command":"material.set_procedural","material":1,"param":"base_color","seed":0}"#;
        assert!(matches!(decode(detach), Ok(Command::SetParameterProcedural { procedural: None, .. })));
    }

    #[test]
    fn base64_round_trips_at_every_length() {
        for length in 0..40usize {
            let bytes: Vec<u8> = (0..length).map(|i| (i * 37 % 251) as u8).collect();
            let encoded = base64_encode(&bytes);
            assert_eq!(base64_decode(&encoded), Some(bytes), "length {length}");
        }
        assert_eq!(base64_decode("!!!!"), None);
    }

    fn count_lines(path: &Path) -> usize {
        std::fs::read_to_string(path)
            .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0)
    }
}

/// Map roles, as short stable names (`docs/02` §5.2).
/// A list of finite numbers at `key`, or empty.
fn floats(value: &crate::json::Value, key: &str) -> Vec<f32> {
    value
        .get(key)
        .map(|v| v.as_array())
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| v.as_f32())
        .filter(|v| v.is_finite())
        .collect()
}

/// A flag at `key`, defaulting to false — an absent flag is an unset flag.
fn boolean(value: &crate::json::Value, key: &str) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn projection_name(projection: tequila_texture::projection::Projection) -> &'static str {
    use tequila_texture::projection::Projection;
    match projection {
        Projection::Uv => "uv",
        Projection::Planar => "planar",
        Projection::Cylindrical => "cylindrical",
        Projection::Spherical => "spherical",
        Projection::TriPlanar => "triplanar",
    }
}

fn projection_from_name(name: &str) -> Option<tequila_texture::projection::Projection> {
    use tequila_texture::projection::Projection;
    Some(match name {
        "uv" => Projection::Uv,
        "planar" => Projection::Planar,
        "cylindrical" => Projection::Cylindrical,
        "spherical" => Projection::Spherical,
        "triplanar" => Projection::TriPlanar,
        _ => return None,
    })
}

fn axis_name(axis: tequila_texture::projection::Axis) -> &'static str {
    use tequila_texture::projection::Axis;
    match axis {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

fn axis_from_name(name: &str) -> Option<tequila_texture::projection::Axis> {
    use tequila_texture::projection::Axis;
    Some(match name {
        "x" => Axis::X,
        "y" => Axis::Y,
        "z" => Axis::Z,
        _ => return None,
    })
}

fn wrap_name(wrap: tequila_texture::mip::Wrap) -> &'static str {
    use tequila_texture::mip::Wrap;
    match wrap {
        Wrap::Repeat => "repeat",
        Wrap::Clamp => "clamp",
        Wrap::Mirror => "mirror",
    }
}

fn wrap_from_name(name: &str) -> Option<tequila_texture::mip::Wrap> {
    use tequila_texture::mip::Wrap;
    Some(match name {
        "repeat" => Wrap::Repeat,
        "clamp" => Wrap::Clamp,
        "mirror" => Wrap::Mirror,
        _ => return None,
    })
}

pub(crate) fn role_name(role: MapRole) -> &'static str {
    match role {
        MapRole::Color => "color",
        MapRole::Data => "data",
        MapRole::Normal => "normal",
        MapRole::Height => "height",
        MapRole::Volume => "volume",
    }
}

pub(crate) fn role_from_name(text: &str) -> Option<MapRole> {
    Some(match text {
        "color" => MapRole::Color,
        "data" => MapRole::Data,
        "normal" => MapRole::Normal,
        "height" => MapRole::Height,
        "volume" => MapRole::Volume,
        _ => return None,
    })
}

pub(crate) fn color_space_name(space: ColorSpace) -> &'static str {
    match space {
        ColorSpace::LinearSrgb => "linear_srgb",
        ColorSpace::Srgb => "srgb",
        ColorSpace::Raw => "raw",
    }
}

pub(crate) fn color_space_from_name(text: &str) -> Option<ColorSpace> {
    Some(match text {
        "linear_srgb" => ColorSpace::LinearSrgb,
        "srgb" => ColorSpace::Srgb,
        "raw" => ColorSpace::Raw,
        _ => return None,
    })
}
