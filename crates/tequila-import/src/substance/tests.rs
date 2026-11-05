use super::*;

/// A description as the format documents it, written out by hand rather
/// than by [`fixture`], so the reader is held to the documented names and
/// not only to whatever the fixture writer happens to write.
const HAND_WRITTEN: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<sbsdescription formatversion="1.1" asmuid="9f2e" cookerbuild="x" content="graphs">
 <global><inputs count="0"/></global>
 <graphs count="2">
  <graph pkgurl="pkg://helper" label="Helper" hideInLibrary="on">
   <inputs count="0"/>
   <outputs count="0"/>
  </graph>
  <graph pkgurl="pkg://leather_soft" label="Soft Leather" category="Fabric/Leather" author="Someone" authorurl="https://example.com" description="Grain &amp; wrinkles" keywords="leather soft" physicalSize="50,50,1" icon="leather_soft.png">
   <guigroups count="1"><guigroup identifier="g1" label="Grain"/></guigroups>
   <inputs count="7">
    <input uid="1" identifier="$outputsize" type="8" default="9,9" alteroutputs="11,12">
     <inputgui widget="slider" label="Output Size" group="Global"><guislider min="4,4" max="12,12" step="1" clamp="on"/></inputgui>
    </input>
    <input uid="2" identifier="$randomseed" type="4" default="0">
     <inputgui widget="slider" label="Random Seed"><guislider min="0" max="10" step="1" clamp="off"/></inputgui>
    </input>
    <input uid="3" identifier="tint" type="2" default="0.3,0.2,0.1">
     <inputgui widget="color" label="Tint" group="g1" description="The leather's colour"/>
    </input>
    <input uid="4" identifier="grain_scale" type="1" default="2,3">
     <inputgui widget="slider" label="Grain scale" group="g1"><guislider min="0.5,0.5" max="8,8" step="0.1" clamp="on" label0="U" label1="V"/></inputgui>
    </input>
    <input uid="5" identifier="normal_format" type="4" default="1">
     <inputgui widget="combobox" label="Normal Format" group="Technical"><guicombobox><guicomboboxitem value="0" text="DirectX"/><guicomboboxitem value="1" text="OpenGL"/></guicombobox></inputgui>
    </input>
    <input uid="6" identifier="perforated" type="4" default="0">
     <inputgui widget="togglebutton" label="Perforated" visibleif="input[&quot;grain_scale&quot;].x &gt; 1"><guibutton label0="No" label1="Yes"/></inputgui>
    </input>
    <input uid="7" identifier="pattern" type="5">
     <inputgui widget="image" label="Pattern"><guiimage colortype="grayscale"><channels count="1"><channel names="mask"/></channels></guiimage></inputgui>
    </input>
   </inputs>
   <outputs count="4">
    <output uid="11" identifier="basecolor" format="0" type="2"><outputgui label="Base Color" group="Material"><channels count="1"><channel names="baseColor"/></channels></outputgui></output>
    <output uid="12" identifier="normal" format="0" type="2"><outputgui label="Normal"><channels count="1"><channel names="normal"/></channels></outputgui></output>
    <output uid="13" identifier="orm" format="0" type="2"><outputgui label="ORM"><channels count="3"><channel names="ambientOcclusion"/><channel names="roughness"/><channel names="metallic"/></channels></outputgui></output>
    <output uid="14" identifier="spec" format="0" type="2"><outputgui label="Specular"><channels count="1"><channel names="specularLevel"/></channels></outputgui></output>
   </outputs>
   <sbspresets count="1">
    <sbspreset label="Tan">
     <presetinput identifier="tint" uid="3" type="2" value="0.6,0.4,0.2"/>
     <presetinput identifier="gone" uid="99" type="0" value="1"/>
    </sbspreset>
   </sbspresets>
  </graph>
 </graphs>
</sbsdescription>"#;

#[test]
fn a_hand_written_description_reads() {
    let file = read_description(HAND_WRITTEN).expect("reads");
    assert_eq!(file.format_version, "1.1");
    assert_eq!(file.graphs.len(), 2);
    assert_eq!(file.visible_graphs().len(), 1, "the helper is hidden from libraries");
    let graph = file.graph(None).unwrap();
    assert_eq!(graph.url, "pkg://leather_soft");
    assert_eq!(graph.identifier(), "leather_soft");
    assert_eq!(graph.name(), "Soft Leather");
    assert_eq!(graph.category, "Fabric/Leather");
    assert_eq!(graph.author, "Someone");
    assert_eq!(graph.description, "Grain & wrinkles");
    assert_eq!(graph.physical_size, Some([50.0, 50.0, 1.0]));
    assert_eq!(file.graph(Some("pkg://leather_soft")).map(|g| g.label.as_str()), Some("Soft Leather"));
    assert_eq!(file.graph(Some("leather_soft")).map(|g| g.label.as_str()), Some("Soft Leather"));

    let size = graph.input("$outputsize").unwrap();
    assert_eq!(size.value_type, ValueType::Integer(2));
    assert_eq!(size.default, Value::Numbers(vec![9.0, 9.0]));
    assert!(size.is_system());
    assert_eq!(size.max, Some(vec![12.0, 12.0]));
    assert!(size.clamp);
    assert_eq!(graph.input("$randomseed").unwrap().value_type, ValueType::Integer(1));

    let tint = graph.input("tint").unwrap();
    assert_eq!(tint.kind(), InputKind::Color { alpha: false });
    assert_eq!(tint.group, "Grain", "a group identifier is shown by its label");
    assert_eq!(tint.description, "The leather's colour");

    let scale = graph.input("grain_scale").unwrap();
    assert_eq!(scale.kind(), InputKind::Float(2));
    assert_eq!(scale.min, Some(vec![0.5, 0.5]));
    assert_eq!(scale.step, Some(0.1));
    assert_eq!(scale.labels, vec!["U", "V"]);

    let format = graph.input("normal_format").unwrap();
    assert_eq!(format.kind(), InputKind::Choice);
    assert_eq!(format.choices, vec![(0, "DirectX".to_string()), (1, "OpenGL".to_string())]);
    assert!(!graph.normal_is_directx(&BTreeMap::new()), "this one defaults to OpenGL");
    let directx: BTreeMap<String, String> = [("normal_format".to_string(), "0".to_string())].into_iter().collect();
    assert!(graph.normal_is_directx(&directx));

    let toggle = graph.input("perforated").unwrap();
    assert_eq!(toggle.kind(), InputKind::Toggle);
    assert_eq!(toggle.labels, vec!["No", "Yes"]);
    assert_eq!(toggle.visible_if, r#"input["grain_scale"].x > 1"#);

    let pattern = graph.input("pattern").unwrap();
    assert_eq!(pattern.kind(), InputKind::Image);
    assert_eq!(pattern.usages, vec!["mask"]);

    // Grouped for the panel, the system inputs left out.
    let groups: Vec<(String, Vec<&str>)> = graph.groups().into_iter().map(|(g, i)| (g, i.into_iter().map(|i| i.identifier.as_str()).collect())).collect();
    assert_eq!(
        groups,
        vec![
            ("Grain".to_string(), vec!["tint", "grain_scale"]),
            ("Technical".to_string(), vec!["normal_format"]),
            (String::new(), vec!["perforated", "pattern"]),
        ]
    );

    assert_eq!(graph.outputs.len(), 4);
    assert_eq!(graph.outputs[2].usages, vec!["ambientOcclusion", "roughness", "metallic"]);
    assert_eq!(graph.outputs[2].slot(false), Ok(crate::texture_set::Slot::Packed));
    assert!(graph.outputs[3].slot(false).unwrap_err().contains("specular"));

    assert_eq!(graph.presets.len(), 1);
    let tan = graph.presets[0].values_for(graph);
    assert_eq!(tan.get("tint").map(String::as_str), Some("0.6,0.4,0.2"));
    assert!(!tan.contains_key("gone"), "a preset's value for an input the graph lacks is dropped");
}

/// What the fixture writes, the reader reads back: every kind of input,
/// the presets, the outputs, the thumbnail.
#[test]
fn a_synthetic_sbsar_reads_back() {
    let bricks = fixture::bricks();
    let file = read_sbsar(&fixture::bricks_sbsar()).expect("reads");
    assert_eq!(file.kind, FileKind::Sbsar);
    assert_eq!(file.graphs.len(), 1);
    let graph = &file.graphs[0];
    assert_eq!(graph.inputs, bricks.inputs);
    assert_eq!(graph.outputs, bricks.outputs);
    assert_eq!(graph.presets, bricks.presets);
    assert_eq!(graph.thumbnail, bricks.thumbnail, "the icon file is the graph's thumbnail");
    assert_eq!((graph.label.as_str(), graph.category.as_str(), graph.author.as_str()), (bricks.label.as_str(), bricks.category.as_str(), bricks.author.as_str()));
    assert_eq!(graph.physical_size, bricks.physical_size);
    let kinds: Vec<InputKind> = graph.inputs.iter().map(Input::kind).collect();
    for kind in [InputKind::Color { alpha: true }, InputKind::Color { alpha: false }, InputKind::Integer(1), InputKind::Float(2), InputKind::Float(1), InputKind::Toggle, InputKind::Choice, InputKind::Text, InputKind::Image] {
        assert!(kinds.contains(&kind), "{kind:?} is missing from {kinds:?}");
    }
}

/// Designer's source reads to the same settings, the cooker's system inputs
/// added, and its icon decoded from the base64 it is kept in.
#[test]
fn a_synthetic_sbs_reads_to_the_same_settings() {
    let bricks = fixture::bricks();
    let file = read_sbs(&fixture::bricks_sbs()).expect("reads");
    assert_eq!(file.kind, FileKind::Sbs);
    let graph = &file.graphs[0];
    assert_eq!(graph.url, "pkg://bricks_tequila");
    assert_eq!(graph.label, bricks.label);
    assert_eq!(graph.thumbnail, bricks.thumbnail);
    assert_eq!(graph.input("$outputsize").unwrap().default, Value::Numbers(vec![8.0, 8.0]), "the graph's own output size");
    assert!(graph.input("$randomseed").is_some());
    for want in &bricks.inputs {
        let got = graph.input(&want.identifier).unwrap_or_else(|| panic!("{} is missing", want.identifier));
        assert_eq!(got.kind(), want.kind(), "{}", want.identifier);
        assert_eq!(got.default, want.default, "{}", want.identifier);
        assert_eq!(got.label, want.label, "{}", want.identifier);
        if !want.is_system() {
            assert_eq!(got.group, want.group, "{}", want.identifier);
            assert_eq!(got.min, want.min, "{}", want.identifier);
            assert_eq!(got.choices, want.choices, "{}", want.identifier);
        }
    }
    let usages: Vec<&str> = graph.outputs.iter().flat_map(|o| o.usages.iter().map(String::as_str)).collect();
    assert_eq!(usages, ["baseColor", "normal", "roughness", "metallic", "height", "ambientOcclusion", "specularLevel"]);
    assert_eq!(graph.presets.len(), 2);
    assert_eq!(graph.presets[0].values_for(graph).get("brick_color").map(String::as_str), Some("0.86,0.84,0.8,1"));
}

/// The outputs become a texture set the way a pack's files do: one map per
/// role, unused outputs named with the reason, a DirectX normal marked so.
#[test]
fn outputs_become_a_texture_set() {
    use crate::texture_set::Slot;
    let graph = fixture::bricks();
    let directory = Path::new("C:/renders/k");
    let (set, unused) = texture_set(&graph, true, |o| rendered_file(directory, &o.identifier));
    let slots: Vec<Slot> = set.maps.iter().map(|m| m.slot).collect();
    assert_eq!(
        slots,
        [Slot::BaseColor, Slot::Roughness, Slot::Metalness, Slot::Normal { directx: true }, Slot::Height, Slot::Occlusion]
    );
    assert_eq!(set.name, "Tequila Test Bricks");
    assert_eq!(set.map(Slot::BaseColor).unwrap().path, directory.join("basecolor.png"));
    assert_eq!(unused.len(), 1);
    assert_eq!(unused[0].0, "specularlevel");
    // Wired as a pack is: height is left out beside a normal map.
    assert_eq!(crate::texture_set::texture_count(&set), 5);
}

/// An output without a usage is read from its identifier's words.
#[test]
fn an_output_without_a_usage_is_read_by_its_name() {
    use crate::texture_set::Slot;
    let output = |id: &str| Output { identifier: id.into(), ..Output::default() };
    assert_eq!(output("Rough").slot(false), Ok(Slot::Roughness));
    assert_eq!(output("my_normal").slot(true), Ok(Slot::Normal { directx: true }));
    assert!(output("thing").slot(false).is_err());
}

#[test]
fn values_read_and_write_as_the_tools_take_them() {
    assert_eq!(Value::parse("0.5, 1;2 3"), vec![0.5, 1.0, 2.0, 3.0]);
    assert_eq!(format_numbers(&[0.5, 1.0, 0.123456789, -0.0], false), "0.5,1,0.123457,0");
    assert_eq!(format_numbers(&[10.4, 9.6], true), "10,10");
    let graph = fixture::bricks();
    let input = graph.input("brick_color").unwrap();
    let values: BTreeMap<String, String> = [("brick_color".to_string(), "1,0,0,1".to_string())].into_iter().collect();
    assert_eq!(input.value_in(&values), Value::Numbers(vec![1.0, 0.0, 0.0, 1.0]));
    assert_eq!(input.value_in(&BTreeMap::new()), input.default);
    assert!(graph.normal_is_directx(&BTreeMap::new()), "the bricks default to DirectX, as Substance Source files do");
}

/// What is not a Substance file says so, and nothing truncated panics.
#[test]
fn a_damaged_file_is_refused_with_a_reason() {
    assert!(read_sbsar(b"PK\x03\x04 a zip").unwrap_err().to_string().contains("7-Zip"));
    let no_description = fixture::seven_zip(&[("notes.txt".into(), b"hello".to_vec())]);
    assert!(read_sbsar(&no_description).unwrap_err().to_string().contains("no description"));
    let whole = fixture::bricks_sbsar();
    for cut in (0..whole.len()).step_by(97) {
        let _ = read_sbsar(&whole[..cut]);
    }
    assert!(read_sbs("<notapackage/>").is_err());
    assert!(read_description("<sbsdescription><graphs><graph").is_err());
    assert!(read(Path::new("C:/materials/thing.txt")).is_err());
}

/// The stand-in renderer writes a map per output, named as asked, and
/// follows the Normal format: DirectX's green is OpenGL's flipped.
#[test]
fn the_stand_in_renders_what_it_is_asked_for() {
    let root = std::env::temp_dir().join(format!("tequila-sbs-fake-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let sbsar = root.join("bricks.sbsar");
    std::fs::write(&sbsar, fixture::bricks_sbsar()).unwrap();
    let graph = fixture::bricks();
    let render = |format: &str, out: &str| {
        let values: BTreeMap<String, String> = [("$outputsize".to_string(), "5,5".to_string()), ("normal_format".to_string(), format.to_string())].into_iter().collect();
        let request = bridge::RenderRequest::for_graph(&sbsar, &graph, &values, &BTreeMap::new(), &root.join(out));
        assert_eq!(fake::main(&request.arguments()), 0);
    };
    render("0", "dx");
    render("1", "gl");
    for output in &graph.outputs {
        assert!(rendered_file(&root.join("dx"), &output.identifier).is_file(), "{}", output.identifier);
    }
    let dx = tequila_texture::image::Image::load_raw(root.join("dx").join("normal.png")).unwrap();
    let gl = tequila_texture::image::Image::load_raw(root.join("gl").join("normal.png")).unwrap();
    assert_eq!((dx.width, dx.height), (32, 32), "the output size is a power of two");
    let flipped = dx.data.chunks(dx.channels).zip(gl.data.chunks(gl.channels)).all(|(a, b)| (a[1] + b[1] - 1.0).abs() < 0.01 && (a[0] - b[0]).abs() < 0.01);
    assert!(flipped, "DirectX's green is OpenGL's upside down");
    assert_ne!(fake::main(&["render".into(), "--input".into(), root.join("missing.sbsar").display().to_string()]), 0);
    let _ = std::fs::remove_dir_all(&root);
}
