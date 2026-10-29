//! Multi-Materials, Model Sets and Studios through the command bus
//! (`docs/02` §8.1, `TEQUILA_SPEC.md` §1.12).

use tequila_core::spectrum::Spectrum;
use tequila_render_core::environment::Environment;
use tequila_scene::snapshot::{snapshot, MeshCache};
use tequila_scene::studio::{ModelSet, ModelSets, MultiMaterial, Studio};
use tequila_scene::tree::NodeKind;
use tequila_scene::{Command, CommandBus, MaterialKey, NodeId, SceneDocument};
use std::sync::Arc;

fn demo() -> (SceneDocument, CommandBus, NodeId, MaterialKey) {
    let document = tequila_scene::demo::demo_document(tequila_scene::demo::DemoMaterial::Plastic, 0.3);
    let part = document
        .tree
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Part { .. }))
        .map(|n| n.id)
        .unwrap();
    let material = document.tree.get(part).unwrap().material.unwrap();
    (document, CommandBus::new(), part, material)
}

/// A second material: the demo's own, copied by unlinking the part, as the
/// Studios tab's Duplicate does.
fn second_material(document: &mut SceneDocument, bus: &mut CommandBus, part: NodeId) -> MaterialKey {
    let before = document.tree.get(part).unwrap().material;
    bus.apply(document, Command::UnlinkMaterial { node: part }).unwrap();
    let copy = document.tree.get(part).unwrap().material.unwrap();
    assert_ne!(Some(copy), before);
    copy
}

/// Parts in the render: instances other than the ground, which is sized to
/// whatever is shown and goes when nothing is.
fn instances(document: &SceneDocument) -> usize {
    snapshot(document, Arc::new(Environment::uniform(Spectrum::ONE)), &mut MeshCache::default())
        .0
        .instances
        .iter()
        .filter(|i| i.shadow_catcher.is_none())
        .count()
}

#[test]
fn switching_a_variant_reassigns_its_parts_and_undoes() {
    let (mut document, mut bus, part, original) = demo();
    let copy = second_material(&mut document, &mut bus, part);
    let multi = |active| MultiMaterial {
        id: 1,
        name: "Colourways".into(),
        variants: vec![original, copy],
        active,
        parts: vec![part],
    };
    bus.apply(&mut document, Command::SetMultiMaterials { multi_materials: vec![multi(0)] }).unwrap();
    assert_eq!(document.tree.get(part).unwrap().material, Some(original));

    bus.apply(&mut document, Command::SetMultiMaterials { multi_materials: vec![multi(1)] }).unwrap();
    assert_eq!(document.tree.get(part).unwrap().material, Some(copy));

    bus.undo(&mut document).unwrap();
    assert_eq!(document.tree.get(part).unwrap().material, Some(original));
    assert_eq!(document.multi_materials, vec![multi(0)]);

    // A variant that names a material the project does not have is refused.
    let mut broken = multi(0);
    broken.variants.push(MaterialKey(9999));
    assert!(bus.apply(&mut document, Command::SetMultiMaterials { multi_materials: vec![broken] }).is_err());
}

#[test]
fn a_model_set_hides_what_is_not_in_it() {
    let (mut document, mut bus, part, _) = demo();
    let everything = instances(&document);
    let other = |part| ModelSets {
        sets: vec![
            ModelSet {
                name: "The part".into(),
                nodes: vec![part],
                always_visible: false,
            },
            ModelSet {
                name: "Empty".into(),
                nodes: Vec::new(),
                always_visible: false,
            },
        ],
        active: Some(1),
    };
    bus.apply(&mut document, Command::SetModelSets { model_sets: Box::new(other(part)) }).unwrap();
    assert_eq!(instances(&document), everything - 1, "the part stayed with another set shown");

    // Always visible, it shows with every set.
    let mut staged = other(part);
    staged.sets[0].always_visible = true;
    bus.apply(&mut document, Command::SetModelSets { model_sets: Box::new(staged) }).unwrap();
    assert_eq!(instances(&document), everything);

    bus.undo(&mut document).unwrap();
    bus.undo(&mut document).unwrap();
    assert_eq!(document.model_sets, ModelSets::default());
    assert_eq!(instances(&document), everything);
}

/// A Studio puts back the environment, the model set and the variant it
/// was saved with, in one undo step.
#[test]
fn a_studio_recalls_the_shot_in_one_step() {
    let (mut document, mut bus, part, original) = demo();
    let copy = second_material(&mut document, &mut bus, part);
    bus.apply(
        &mut document,
        Command::SetMultiMaterials {
            multi_materials: vec![MultiMaterial {
                id: 4,
                name: "Colourways".into(),
                variants: vec![original, copy],
                active: 0,
                parts: vec![part],
            }],
        },
    )
    .unwrap();
    bus.apply(
        &mut document,
        Command::SetModelSets {
            model_sets: Box::new(ModelSets {
                sets: vec![ModelSet {
                    name: "Set".into(),
                    nodes: vec![part],
                    always_visible: false,
                }],
                active: None,
            }),
        },
    )
    .unwrap();

    // Saved with the second variant, the set shown and a turned environment.
    let mut studio = Studio::capture("Hero", &document);
    studio.variants = vec![(4, 1)];
    studio.model_set = Some(Some(0));
    studio.environment.as_mut().unwrap().rotation = 2.0;
    bus.apply(&mut document, Command::SetStudios { studios: vec![studio] }).unwrap();
    let before = document.clone();

    bus.apply(&mut document, Command::ApplyStudio { index: 0 }).unwrap();
    assert_eq!(document.environment.rotation, 2.0);
    assert_eq!(document.model_sets.active, Some(0));
    assert_eq!(document.multi_materials[0].active, 1);
    assert_eq!(document.tree.get(part).unwrap().material, Some(copy));

    bus.undo(&mut document).unwrap();
    assert!(document.difference_from(&before).is_none(), "{:?}", document.difference_from(&before));

    assert!(bus.apply(&mut document, Command::ApplyStudio { index: 5 }).is_err());
}

/// The graph editor's solo reaches the renderer as the node alone, unlit,
/// on the parts wearing that material — and leaves the document as it was.
#[test]
fn solo_shows_a_node_on_the_model_without_touching_the_document() {
    use tequila_materials::graph::{Node, NodeKind as GraphKind, Socket, Value};
    use tequila_materials::material_type::Param;
    use tequila_scene::snapshot::{snapshot_with, ViewOverrides};

    let (mut document, mut bus, part, material) = demo();
    let mut graph = document.materials.get(material).unwrap().graph.clone();
    let colour = graph.add_node(Node::new(GraphKind::Constant(Value::Color(Spectrum { r: 0.1, g: 0.7, b: 0.3 }))));
    let output = graph.output();
    graph.connect(colour, output, Socket::Param(Param::BaseColor));
    bus.apply(&mut document, Command::SetGraph { material, graph: Box::new(graph) }).unwrap();
    let before = document.clone();

    let build = |view: &ViewOverrides| {
        let (scene, mapping) = snapshot_with(
            &document,
            Arc::new(Environment::uniform(Spectrum::ONE)),
            &mut MeshCache::default(),
            view,
        );
        let instance = mapping.instance_of(part).unwrap();
        scene.material(scene.instances[instance.0 as usize].material).clone()
    };
    let plain = build(&ViewOverrides::default());
    assert!(plain.base_weight > 0.5 && !plain.is_emissive());

    let soloed = build(&ViewOverrides { solo: Some((material, colour, 0)) });
    assert_eq!(soloed.base_weight, 0.0, "the soloed surface still reflects");
    assert!(soloed.is_emissive(), "the soloed surface does not show the node");
    assert!(document.difference_from(&before).is_none(), "soloing changed the document");
}

/// A material with an Occlusion node gets its parts' occlusion measured once,
/// at the node's radius; one without pays nothing for it.
#[test]
fn an_occlusion_node_measures_its_parts_and_nothing_else() {
    use tequila_materials::graph::{Node, NodeKind as GraphKind, Socket};
    use tequila_materials::material_type::Param;

    let (mut document, mut bus, part, material) = demo();
    let mesh_of = |document: &SceneDocument| {
        let (scene, mapping) = snapshot(document, Arc::new(Environment::uniform(Spectrum::ONE)), &mut MeshCache::default());
        let instance = mapping.instance_of(part).unwrap();
        scene.meshes[scene.instances[instance.0 as usize].mesh.index()].clone()
    };
    assert!(mesh_of(&document).occlusion.is_empty(), "occlusion was measured with nothing asking");

    let mut graph = document.materials.get(material).unwrap().graph.clone();
    let occlusion = graph.add_node(Node::new(GraphKind::Occlusion { radius: 0.5, contrast: 1.0 }));
    let output = graph.output();
    graph.connect(occlusion, output, Socket::Param(Param::SpecularRoughness));
    bus.apply(&mut document, Command::SetGraph { material, graph: Box::new(graph) }).unwrap();

    let measured = mesh_of(&document);
    assert_eq!(measured.occlusion.len(), measured.positions.len());
    // A sphere is convex: nothing of it overhangs anything else of it.
    assert!(measured.occlusion.iter().all(|&o| o < 0.05), "a convex sphere occludes itself");
}

/// A Fuzz node grows fibres on its part: one more instance, in the part's
/// place and material, its mesh rooted on the part's surface and standing
/// out of it by about the length asked; and a scene built again reuses the
/// same fibres rather than growing them anew.
#[test]
fn a_fuzz_node_grows_fibres_on_the_part() {
    use tequila_materials::graph::{Node, NodeKind as GraphKind};

    let (mut document, mut bus, part, material) = demo();
    let mut cache = MeshCache::default();
    let (plain, _) = snapshot(&document, Arc::new(Environment::uniform(Spectrum::ONE)), &mut cache);

    let mut graph = document.materials.get(material).unwrap().graph.clone();
    graph.add_node(Node::new(GraphKind::Fuzz { length: 0.05, density: 0.5, thickness: 0.002, lean: 0.0, curl: 0.0, seed: 4 }));
    bus.apply(&mut document, Command::SetGraph { material, graph: Box::new(graph) }).unwrap();

    let (scene, mapping) = snapshot(&document, Arc::new(Environment::uniform(Spectrum::ONE)), &mut cache);
    assert_eq!(scene.instances.len(), plain.instances.len() + 1, "one instance of fibres");
    let part_instance = &scene.instances[mapping.instance_of(part).unwrap().0 as usize];
    let fur_instance = scene
        .instances
        .iter()
        .find(|i| i.shadow_catcher.is_none() && mapping.node_of(i.id).is_none())
        .expect("the fibres' instance");
    assert_eq!(fur_instance.material, part_instance.material);
    assert_eq!(fur_instance.transform.matrix, part_instance.transform.matrix);
    let fur = &scene.meshes[fur_instance.mesh.index()];
    let body = scene.meshes[part_instance.mesh.index()].bounds();
    // The fibres reach past the part by about their length, in its units.
    let reach = (fur.bounds().max - body.max).x * (1.0 / part_instance.transform.inv_length_scale());
    assert!(reach > 0.02 && reach < 0.08, "fibres reach {reach} past the part");

    let (again, _) = snapshot(&document, Arc::new(Environment::uniform(Spectrum::ONE)), &mut cache);
    let fur_again = again.instances.iter().find(|i| i.shadow_catcher.is_none() && mapping.node_of(i.id).is_none()).unwrap();
    assert!(Arc::ptr_eq(&again.meshes[fur_again.mesh.index()], fur), "the fibres were grown again");
}

/// A Bubbles node fills a part with bubbles: wholly inside it, touching
/// none of the others, in the part's own material, and in the same places
/// every time the scene is built.
#[test]
fn a_bubbles_node_fills_the_part() {
    use tequila_materials::graph::{Node, NodeKind as GraphKind};

    let (mut document, mut bus, part, material) = demo();
    let build = |document: &SceneDocument| {
        snapshot(document, Arc::new(Environment::uniform(Spectrum::ONE)), &mut MeshCache::default())
    };
    let (plain, _) = build(&document);

    let mut graph = document.materials.get(material).unwrap().graph.clone();
    graph.add_node(Node::new(GraphKind::Bubbles { count: 40, radius: 0.06, variation: 0.5, seed: 11 }));
    bus.apply(&mut document, Command::SetGraph { material, graph: Box::new(graph) }).unwrap();

    let (scene, mapping) = build(&document);
    let bubbles: Vec<_> = scene.instances[plain.instances.len() - 1..]
        .iter()
        .filter(|i| i.shadow_catcher.is_none() && mapping.node_of(i.id).is_none())
        .collect();
    assert!(bubbles.len() >= 30, "only {} bubbles placed of 40", bubbles.len());

    let part_instance = &scene.instances[mapping.instance_of(part).unwrap().0 as usize];
    // The demo part is a sphere; where it is and how big, in the world.
    let bounds = scene.meshes[part_instance.mesh.index()].bounds().transformed(&part_instance.transform);
    let (centre, radius) = (bounds.center(), bounds.diagonal().x * 0.5);
    let spheres: Vec<(tequila_core::math::Vec3, f32)> = bubbles
        .iter()
        .map(|b| (b.transform.point(tequila_core::math::Vec3::ZERO), 1.0 / b.transform.inv_length_scale()))
        .collect();
    for (i, (c, r)) in spheres.iter().enumerate() {
        assert!((*c - centre).length() + r <= radius * 1.01, "bubble {i} pokes out of the part");
        for (c2, r2) in &spheres[i + 1..] {
            assert!((*c - *c2).length() >= r + r2 - 1e-5, "two bubbles overlap");
        }
    }
    assert!(bubbles.iter().all(|b| b.material == part_instance.material), "a bubble wears another material");

    let (again, _) = build(&document);
    assert_eq!(again.instances.len(), scene.instances.len());
    assert_eq!(again.instances.last().unwrap().transform.matrix, scene.instances.last().unwrap().transform.matrix);
}

/// A configuration switches a variant, a model set and a Studio in one step,
/// is refused when it breaks a rule, and undoes whole.
#[test]
fn a_configuration_is_shown_in_one_step_and_undoes() {
    use tequila_scene::configurator::{Choice, ConfigOption, Configurator, Drives, Rule, RuleKind, Style};
    let (mut document, mut bus, part, original) = demo();
    let copy = second_material(&mut document, &mut bus, part);
    bus.apply(&mut document, Command::SetMultiMaterials {
        multi_materials: vec![MultiMaterial { id: 4, name: "Body".into(), variants: vec![original, copy], active: 0, parts: vec![part] }],
    })
    .unwrap();
    bus.apply(&mut document, Command::SetModelSets {
        model_sets: Box::new(ModelSets {
            sets: vec![
                ModelSet { name: "With".into(), nodes: vec![part], always_visible: false },
                ModelSet { name: "Without".into(), nodes: Vec::new(), always_visible: false },
            ],
            active: Some(0),
        }),
    })
    .unwrap();
    let mut side = Studio::capture("Side", &document);
    side.environment.as_mut().unwrap().rotation = 2.0;
    side.variants.clear();
    side.model_set = None;
    let front = Studio::capture("Front", &document);
    bus.apply(&mut document, Command::SetStudios { studios: vec![front, side] }).unwrap();
    let choice = |label: &str, target| Choice { label: label.into(), target, swatch: None };
    let configurator = Configurator {
        options: vec![
            ConfigOption { id: 1, name: "Body".into(), drives: Drives::Material { multi_material: 4 }, choices: vec![choice("A", 0), choice("B", 1)], style: Style::Swatches },
            ConfigOption { id: 2, name: "Model".into(), drives: Drives::ModelSet, choices: vec![choice("With", 0), choice("Without", 1)], style: Style::Buttons },
            ConfigOption { id: 3, name: "View".into(), drives: Drives::Studio, choices: vec![choice("Front", 0), choice("Side", 1)], style: Style::Buttons },
        ],
        // B is not sold without the part.
        rules: vec![Rule { option: 1, choice: 1, kind: RuleKind::Excludes, other: 2, other_choice: 1 }],
        current: vec![0, 0, 0],
        ..Default::default()
    };
    bus.apply(&mut document, Command::SetConfigurator { configurator: Box::new(configurator) }).unwrap();
    let before = document.clone();

    bus.apply(&mut document, Command::Configure { choices: vec![1, 0, 1] }).unwrap();
    assert_eq!(document.tree.get(part).unwrap().material, Some(copy), "the body variant");
    assert_eq!(document.model_sets.active, Some(0));
    assert_eq!(document.environment.rotation, 2.0, "the side studio");
    assert_eq!(document.configurator.current, vec![1, 0, 1]);

    let refused = bus.apply(&mut document, Command::Configure { choices: vec![1, 1, 0] });
    assert!(refused.is_err(), "B without the part breaks the rule");
    assert_eq!(document.configurator.current, vec![1, 0, 1], "and changes nothing");

    bus.undo(&mut document).unwrap();
    assert_eq!(document.difference_from(&before), None, "undo puts every part of it back");
}

// ---- Views: Studios that switch lights and turn the product --------------

use tequila_scene::studio::{LightOverride, LightTarget};

fn light(document: &mut SceneDocument, bus: &mut CommandBus, name: &str, layer: u8) -> NodeId {
    let mut settings = tequila_scene::lighting::LightSettings::default();
    settings.layer = layer;
    bus.apply(document, Command::AddLight { parent: None, name: name.into(), settings: Box::new(settings) }).unwrap();
    document.tree.iter().find(|n| n.name == name).map(|n| n.id).unwrap()
}

fn on(document: &SceneDocument, id: NodeId) -> bool {
    match &document.tree.get(id).unwrap().kind {
        NodeKind::Light { settings } => settings.enabled,
        _ => panic!("not a light"),
    }
}

fn close(a: tequila_core::math::Transform, b: tequila_core::math::Transform) -> bool {
    (0..4).all(|r| (0..4).all(|c| (a.matrix.m[r][c] - b.matrix.m[r][c]).abs() < 1e-5))
}

/// A view switches the lights it names - by name and by layer - turns the
/// product about its own vertical axis and leaves the lights where they
/// stand, and goes to the saved camera of its camera's name. Switching back
/// to the front view brings everything back, and undo takes each step back
/// whole.
#[test]
fn a_view_switches_lights_turns_the_product_and_takes_its_camera() {
    let (mut document, mut bus, _, _) = demo();
    let key = light(&mut document, &mut bus, "Key", 0);
    let strip = light(&mut document, &mut bus, "Strip L", 1);
    let product = tequila_scene::studio::product_roots(&document);
    assert!(!product.is_empty() && !product.contains(&key) && !product.contains(&strip), "lights are not the product");
    let placed: Vec<_> = product.iter().map(|&id| document.tree.get(id).unwrap().transform).collect();
    let lamp = document.tree.get(key).unwrap().transform;
    let centre = tequila_scene::snapshot::visible_bounds(&document).center();

    let main_camera = document.cameras.active().name.clone();
    let mut rear = document.cameras.active().clone();
    rear.name = "Rear".into();
    rear.target = tequila_core::math::vec3(0.0, 0.1, -0.5);
    bus.apply(&mut document, Command::AddCamera { camera: Box::new(rear.clone()) }).unwrap();
    assert_eq!(document.cameras.active().name, main_camera);

    let views = vec![
        Studio {
            name: "Main".into(),
            lights: vec![LightOverride { target: LightTarget::Name("Strip L".into()), enabled: true }],
            product_yaw: Some(0.0),
            ..Studio::default()
        },
        Studio {
            name: "Rear".into(),
            camera: Some(rear.clone()),
            lights: vec![LightOverride { target: LightTarget::Layer(1), enabled: false }],
            product_yaw: Some(std::f32::consts::PI),
            filename_suffix: Some("Rear".into()),
            ..Studio::default()
        },
    ];
    bus.apply(&mut document, Command::SetStudios { studios: views }).unwrap();

    bus.apply(&mut document, Command::ApplyStudio { index: 1 }).unwrap();
    assert!(!on(&document, strip), "the Rear view switches the strip's layer off");
    assert!(on(&document, key), "a light no view names is left alone");
    assert_eq!(document.cameras.active().name, "Rear", "the view goes to its camera's namesake");
    assert_eq!(document.cameras.len(), 2, "and writes over nothing else");
    for (&id, before) in product.iter().zip(&placed) {
        assert!(!close(document.tree.get(id).unwrap().transform, *before), "the product is turned");
    }
    let turned = tequila_scene::snapshot::visible_bounds(&document).center();
    assert!((turned.x - centre.x).abs() < 1e-4 && (turned.z - centre.z).abs() < 1e-4, "about its own axis: {centre:?} then {turned:?}");
    assert!(close(document.tree.get(key).unwrap().transform, lamp), "the lights stay where they stand");

    bus.apply(&mut document, Command::ApplyStudio { index: 0 }).unwrap();
    assert!(on(&document, strip));
    for (&id, before) in product.iter().zip(&placed) {
        assert!(close(document.tree.get(id).unwrap().transform, *before), "turned back exactly");
    }

    bus.undo(&mut document).unwrap();
    assert!(!on(&document, strip) && document.cameras.active().name == "Rear");
    bus.undo(&mut document).unwrap();
    assert!(on(&document, strip));
    assert_eq!(document.cameras.active().name, main_camera);
    assert_eq!(document.product_turn, tequila_scene::studio::ProductTurn::default());
    for (&id, before) in product.iter().zip(&placed) {
        assert!(close(document.tree.get(id).unwrap().transform, *before));
    }
}

/// A Studio from before views - no lights, no turn - leaves the lights and
/// the product as they are.
#[test]
fn a_studio_without_views_leaves_lights_and_product_alone() {
    let (mut document, mut bus, _, _) = demo();
    let strip = light(&mut document, &mut bus, "Strip L", 1);
    bus.apply(&mut document, Command::SetLight {
        node: strip,
        settings: Box::new(tequila_scene::lighting::LightSettings { enabled: false, layer: 1, ..Default::default() }),
    })
    .unwrap();
    let product = tequila_scene::studio::product_roots(&document);
    let placed: Vec<_> = product.iter().map(|&id| document.tree.get(id).unwrap().transform).collect();
    let mut old = Studio::capture("Old", &document);
    old.lights.clear();
    old.product_yaw = None;
    bus.apply(&mut document, Command::SetStudios { studios: vec![old] }).unwrap();
    bus.apply(&mut document, Command::ApplyStudio { index: 0 }).unwrap();
    assert!(!on(&document, strip));
    for (&id, before) in product.iter().zip(&placed) {
        assert!(close(document.tree.get(id).unwrap().transform, *before));
    }
}
