//! Material Templates on an assembly (`docs/04` §3.3).

use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_render_core::material::Material;
use tequila_scene::command::{reserve_keys, ImportPayload};
use tequila_scene::geometry::Mesh;
use tequila_scene::material::MaterialDefinition;
use tequila_scene::templates::{commands, plan, Condition, MaterialTemplate, Rule};
use tequila_scene::tree::{NodeKind, SceneNode};
use tequila_scene::{Command, CommandBus, MaterialKey, NodeId, SceneDocument};

/// Five parts: two bolts, a housing, a part on the HARDWARE layer, and a
/// badge — wearing an imported grey, a steel, and a red.
fn assembly() -> (SceneDocument, CommandBus, Vec<NodeId>, Vec<MaterialKey>) {
    let mut document = SceneDocument::new();
    let mut bus = CommandBus::new();
    let (meshes, materials, ids) = reserve_keys(&document, 1, 5, 5);
    let mesh = Mesh::new(vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
    let grey = Material::plastic(Spectrum::splat(0.25), 0.4);
    let red = Material::plastic(Spectrum { r: 0.8, g: 0.05, b: 0.05 }, 0.4);
    let named = |name: &str, material: Material| MaterialDefinition::new(name, tequila_materials::material_type::MaterialType::Plastic, material);
    let defs = vec![
        (materials[0], named("CAD grey", grey.clone())),
        (materials[1], named("Steel_brushed", grey)),
        (materials[2], named("CAD red", red)),
        (materials[3], named("Chrome", Material::plastic(Spectrum::splat(0.9), 0.05))),
        (materials[4], named("Black ABS", Material::plastic(Spectrum::splat(0.03), 0.3))),
    ];
    let parts = [
        ("Bolt M6x20", 1),
        ("bolt M6x40", 1),
        ("Housing (left)", 0),
        ("Bracket", 0),
        ("Badge", 2),
    ];
    let mut nodes = Vec::new();
    for (i, (name, material)) in parts.iter().enumerate() {
        let mut node = SceneNode::new(ids[i], *name, NodeKind::Part { mesh: meshes[0] });
        node.material = Some(materials[*material]);
        if *name == "Bracket" {
            node.metadata.insert("Layer".into(), "HARDWARE".into());
        }
        nodes.push(node);
    }
    bus.apply(
        &mut document,
        Command::Import {
            payload: Box::new(ImportPayload {
                meshes: vec![(meshes[0], mesh)],
                textures: Vec::new(),
                materials: defs,
                roots: ids.clone(),
                nodes,
            }),
        },
    )
    .unwrap();
    (document, bus, ids, materials)
}

#[test]
fn the_first_matching_rule_decides_and_applying_is_one_undo() {
    let (mut document, mut bus, ids, materials) = assembly();
    let (chrome, black) = (materials[3], materials[4]);
    let template = MaterialTemplate {
        name: "Product".into(),
        rules: vec![
            // Fasteners by name, whatever colour the CAD gave them.
            Rule { condition: Condition::Name("bolt*".into()), material: chrome },
            // The HARDWARE layer.
            Rule {
                condition: Condition::Metadata { key: "layer".into(), value: "hard*".into() },
                material: chrome,
            },
            // Everything else that came in CAD grey.
            Rule {
                condition: Condition::Colour { color: Spectrum::splat(0.25), tolerance: 0.02 },
                material: black,
            },
        ],
    };
    let planned = plan(&document, &template);
    let decided: Vec<(NodeId, MaterialKey, usize)> = planned.iter().map(|p| (p.part, p.to, p.rule)).collect();
    assert_eq!(
        decided,
        vec![(ids[0], chrome, 0), (ids[1], chrome, 0), (ids[2], black, 2), (ids[3], chrome, 1)],
        "the bolts go by name even though they are grey, the bracket by its layer, the housing by colour, and the red badge is left alone"
    );

    bus.begin_group();
    for command in commands(&planned) {
        bus.apply(&mut document, command).unwrap();
    }
    bus.end_group();
    assert_eq!(document.tree.get(ids[0]).unwrap().material, Some(chrome));
    assert_eq!(document.tree.get(ids[2]).unwrap().material, Some(black));
    assert_eq!(document.tree.get(ids[4]).unwrap().material, Some(materials[2]));

    // Applied again, there is nothing left to do.
    assert!(plan(&document, &template).is_empty());

    bus.undo(&mut document).unwrap();
    assert_eq!(document.tree.get(ids[0]).unwrap().material, Some(materials[1]));
    assert_eq!(document.tree.get(ids[2]).unwrap().material, Some(materials[0]));
}

#[test]
fn a_material_name_rule_maps_what_the_cad_called_it() {
    let (document, _, ids, materials) = assembly();
    let template = MaterialTemplate {
        name: "Rename".into(),
        rules: vec![Rule {
            condition: Condition::Material("steel*".into()),
            material: materials[3],
        }],
    };
    let parts: Vec<NodeId> = plan(&document, &template).iter().map(|p| p.part).collect();
    assert_eq!(parts, vec![ids[0], ids[1]]);
}
