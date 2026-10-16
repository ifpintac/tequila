//! IFC: a building model built into a document - standing up, in metres,
//! its window opening cut, its properties searchable.

use tequila_import::{build, read_format, ImportOptions};
use tequila_scene::metadata::{find, Query};
use tequila_scene::tree::NodeKind;
use tequila_scene::{CommandBus, SceneDocument};

/// A storey holding a wall 4 m long, 0.2 m thick and 2.5 m high, with a
/// window opening 1 x 1.2 m through it, written in millimetres along Z up as
/// building models are.
const STOREY: &str = "ISO-10303-21;
HEADER;FILE_DESCRIPTION((''),'2;1');FILE_NAME('storey.ifc','',(''),(''),'','','');FILE_SCHEMA(('IFC2X3'));ENDSEC;
DATA;
#1=IFCPROJECT('3vB2YO$MX4xv5uCqZZG05x',$,'Studio',$,$,$,$,$,#2);
#2=IFCUNITASSIGNMENT((#3));
#3=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);
#10=IFCCARTESIANPOINT((0.,0.,0.));#11=IFCDIRECTION((0.,0.,1.));#12=IFCDIRECTION((1.,0.,0.));
#13=IFCAXIS2PLACEMENT3D(#10,#11,#12);#14=IFCLOCALPLACEMENT($,#13);
#20=IFCBUILDINGSTOREY('2OBrcmyk58NupXoVOHUuXp',$,'Level 1',$,$,#14,$,$,.ELEMENT.,0.);
#21=IFCRELAGGREGATES('1',$,$,$,#1,(#20));
#30=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,4000.,200.);
#31=IFCEXTRUDEDAREASOLID(#30,#13,#11,2500.);
#32=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#31));
#33=IFCPRODUCTDEFINITIONSHAPE($,$,(#32));
#34=IFCWALLSTANDARDCASE('0DWgwt6o1FOx7466fPk$jl',$,'Basic Wall:Exterior',$,'Exterior 200',#14,#33,'318',$);
#35=IFCRELCONTAINEDINSPATIALSTRUCTURE('2',$,$,$,(#34),#20);
#40=IFCCARTESIANPOINT((0.,-200.,1500.));#41=IFCDIRECTION((0.,1.,0.));
#42=IFCAXIS2PLACEMENT3D(#40,#41,#12);
#43=IFCRECTANGLEPROFILEDEF(.AREA.,$,$,1000.,1200.);
#44=IFCEXTRUDEDAREASOLID(#43,#42,#11,400.);
#45=IFCSHAPEREPRESENTATION($,'Body','SweptSolid',(#44));
#46=IFCPRODUCTDEFINITIONSHAPE($,$,(#45));
#47=IFCLOCALPLACEMENT(#14,#13);
#48=IFCOPENINGELEMENT('1hPBEC$Zz5vRV1F1cD4F8a',$,'Window opening',$,$,#47,#46,$);
#49=IFCRELVOIDSELEMENT('3',$,$,$,#34,#48);
#60=IFCPROPERTYSINGLEVALUE('IsExternal',$,IFCBOOLEAN(.T.),$);
#61=IFCPROPERTYSINGLEVALUE('ThermalTransmittance',$,IFCTHERMALTRANSMITTANCEMEASURE(0.24),$);
#62=IFCPROPERTYSET('4',$,'Pset_WallCommon',$,(#60,#61));
#63=IFCRELDEFINESBYPROPERTIES('5',$,$,$,(#34),#62);
#70=IFCCOLOURRGB($,0.8,0.78,0.72);
#71=IFCSURFACESTYLESHADING(#70);
#72=IFCSURFACESTYLE('Render',.BOTH.,(#71));
#73=IFCPRESENTATIONSTYLEASSIGNMENT((#72));
#74=IFCSTYLEDITEM(#31,(#73),$);
ENDSEC;
END-ISO-10303-21;
";

#[test]
fn a_storey_builds_standing_up_with_its_opening_cut() {
    let scene = read_format(STOREY.as_bytes(), "ifc", "storey", &|_| None).expect("reads");
    assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
    // IFC2x3's style assignment, followed to the colour and its name.
    let wall_mesh = scene.meshes.iter().find(|m| m.name == "Basic Wall:Exterior").expect("the wall");
    assert_eq!(scene.materials[wall_mesh.material.unwrap()].name, "Render");

    let mut document = SceneDocument::new();
    let options = ImportOptions { place_on_ground: false, ..Default::default() };
    let (command, _) = build(&document, None, scene, &options, None).unwrap();
    CommandBus::new().apply(&mut document, command).unwrap();

    // Found by its properties.
    let query = Query::parse("Pset_WallCommon.IsExternal = true").expect("a query");
    let found = find(&document, &query);
    assert_eq!(found.len(), 1);
    let wall = document.tree.get(found[0]).unwrap();
    assert_eq!(wall.name, "Basic Wall:Exterior");
    assert_eq!(wall.metadata.get("IFC type").map(String::as_str), Some("IfcWallStandardCase"));
    assert_eq!(wall.metadata.get("ObjectType").map(String::as_str), Some("Exterior 200"));
    assert_eq!(wall.metadata.get("Pset_WallCommon.ThermalTransmittance").map(String::as_str), Some("0.24"));

    // Standing up: 2.5 tall along Y, 4 long along X, 0.2 deep along Z, in
    // metres.
    let NodeKind::Part { mesh } = wall.kind else { panic!("the wall is a part") };
    let world = document.tree.world_transform(found[0]);
    let points: Vec<_> = document.geometry.get(mesh).unwrap().positions.iter().map(|p| world.point(*p)).collect();
    let extent = |axis: fn(&tequila_core::math::Vec3) -> f32| {
        let (lo, hi) = points.iter().map(axis).fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(v), hi.max(v)));
        hi - lo
    };
    assert!((extent(|p| p.y) - 2.5).abs() < 1e-3, "height {}", extent(|p| p.y));
    assert!((extent(|p| p.x) - 4.0).abs() < 1e-3, "length {}", extent(|p| p.x));
    assert!((extent(|p| p.z) - 0.2).abs() < 1e-3, "depth {}", extent(|p| p.z));

    // The opening shows: a ray through the wall where the window is, 1.5 m
    // up its middle, meets nothing; one a metre and a half along meets the
    // wall's two faces.
    let low = |axis: fn(&tequila_core::math::Vec3) -> f32| points.iter().map(axis).fold(f32::MAX, f32::min);
    let (left, bottom, front) = (low(|p| p.x), low(|p| p.y), low(|p| p.z));
    let middle = left + extent(|p| p.x) / 2.0;
    let geometry = document.geometry.get(mesh).unwrap();
    let triangles: Vec<[tequila_core::math::Vec3; 3]> =
        geometry.indices.chunks(3).map(|t| [t[0], t[1], t[2]].map(|i| world.point(geometry.positions[i as usize]))).collect();
    let hits = |x_metres: f32| {
        let origin = tequila_core::math::vec3(middle + x_metres, bottom + 1.5, front - 1.0);
        let dir = tequila_core::math::vec3(0.0, 0.0, 1.0);
        triangles
            .iter()
            .filter(|[a, b, c]| {
                let (e1, e2) = (*b - *a, *c - *a);
                let q = dir.cross(e2);
                let det = e1.dot(q);
                if det.abs() < 1e-12 {
                    return false;
                }
                let s = origin - *a;
                let u = s.dot(q) / det;
                let r = s.cross(e1);
                let v = dir.dot(r) / det;
                u >= 0.0 && v >= 0.0 && u + v <= 1.0 && e2.dot(r) / det > 0.0
            })
            .count()
    };
    assert_eq!(hits(1.5), 2, "the control ray misses the wall");
    assert_eq!(hits(0.0), 0, "the wall still covers its window");
}
