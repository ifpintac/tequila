//! Synthetic Substance files, written from the published format.
//!
//! No `.sbsar` or `.sbs` made by Adobe's tools was available where this was
//! written, so the tests, the fake renderer and the pictures in the pull
//! request use files written here: the description in the shape pysbs
//! documents, in a 7-Zip archive beside a stand-in for the compiled graph
//! and an icon. They prove Tequila reads what the format says; they do not
//! prove a real file says it the same way.

use super::{format_numbers, FileKind, Graph, Input, Output, Preset, SubstanceFile, Value, ValueType, Widget};
use std::fmt::Write as _;

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn value_text(value: &Value, value_type: ValueType) -> String {
    match value {
        Value::Text(t) => t.clone(),
        Value::Numbers(n) => format_numbers(n, matches!(value_type, ValueType::Integer(_))),
    }
}

/// The `<sbsdescription>` an `.sbsar` carries for `graphs`.
pub fn description_xml(graphs: &[Graph]) -> String {
    let mut x = String::new();
    let _ = writeln!(x, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(x, r#"<sbsdescription formatversion="1.1" asmuid="00000000000000000000000000000000" cookerbuild="tequila-fixture" content="graphs">"#);
    let _ = writeln!(x, r#" <global><inputs count="0"/></global>"#);
    let _ = writeln!(x, r#" <graphs count="{}">"#, graphs.len());
    for (index, g) in graphs.iter().enumerate() {
        let size = g.physical_size.map(|s| format!(r#" physicalSize="{},{},{}""#, s[0], s[1], s[2])).unwrap_or_default();
        let icon = if g.thumbnail.is_some() { format!(r#" icon="icon_{index}.png""#) } else { String::new() };
        let _ = writeln!(
            x,
            r#"  <graph pkgurl="{}" label="{}" category="{}" author="{}" authorurl="{}" description="{}" keywords="{}" hideInLibrary="{}"{size}{icon}>"#,
            escape(&g.url),
            escape(&g.label),
            escape(&g.category),
            escape(&g.author),
            escape(&g.author_url),
            escape(&g.description),
            escape(&g.keywords),
            if g.hidden { "on" } else { "off" },
        );
        let _ = writeln!(x, r#"   <inputs count="{}">"#, g.inputs.len());
        for input in &g.inputs {
            write_input(&mut x, input);
        }
        let _ = writeln!(x, r#"   </inputs>"#);
        let _ = writeln!(x, r#"   <outputs count="{}">"#, g.outputs.len());
        for output in &g.outputs {
            let _ = writeln!(
                x,
                r#"    <output uid="{}" identifier="{}" format="0" width="0" height="0" mipmaps="0" type="2" dynamicsize="1">"#,
                escape(&output.uid),
                escape(&output.identifier)
            );
            let _ = writeln!(x, r#"     <outputgui label="{}" group="{}">"#, escape(&output.label), escape(&output.group));
            let _ = writeln!(x, r#"      <channels count="{}">"#, output.usages.len());
            for usage in &output.usages {
                let _ = writeln!(x, r#"       <channel names="{}"/>"#, escape(usage));
            }
            let _ = writeln!(x, r#"      </channels>"#);
            let _ = writeln!(x, r#"     </outputgui>"#);
            let _ = writeln!(x, r#"    </output>"#);
        }
        let _ = writeln!(x, r#"   </outputs>"#);
        if !g.presets.is_empty() {
            let _ = writeln!(x, r#"   <sbspresets count="{}">"#, g.presets.len());
            for preset in &g.presets {
                let _ = writeln!(x, r#"    <sbspreset label="{}">"#, escape(&preset.label));
                for (id, value) in &preset.values {
                    let input = g.input(id);
                    let _ = writeln!(
                        x,
                        r#"     <presetinput identifier="{}" uid="{}" type="{}" value="{}"/>"#,
                        escape(id),
                        input.map(|i| escape(&i.uid)).unwrap_or_default(),
                        input.map(|i| i.value_type.sbsar_code()).unwrap_or(0),
                        escape(value)
                    );
                }
                let _ = writeln!(x, r#"    </sbspreset>"#);
            }
            let _ = writeln!(x, r#"   </sbspresets>"#);
        }
        let _ = writeln!(x, r#"  </graph>"#);
    }
    let _ = writeln!(x, r#" </graphs>"#);
    let _ = writeln!(x, r#"</sbsdescription>"#);
    x
}

fn write_input(x: &mut String, input: &Input) {
    let _ = writeln!(
        x,
        r#"    <input uid="{}" identifier="{}" type="{}" default="{}" alteroutputs="">"#,
        escape(&input.uid),
        escape(&input.identifier),
        input.value_type.sbsar_code(),
        escape(&value_text(&input.default, input.value_type))
    );
    let _ = writeln!(
        x,
        r#"     <inputgui widget="{}" label="{}" group="{}" description="{}" visibleif="{}">"#,
        input.widget.name(),
        escape(&input.label),
        escape(&input.group),
        escape(&input.description),
        escape(&input.visible_if)
    );
    let integers = matches!(input.value_type, ValueType::Integer(_));
    let numbers = |v: &Option<Vec<f64>>| v.as_ref().map(|n| format_numbers(n, integers)).unwrap_or_default();
    match input.widget {
        Widget::Slider | Widget::Angle | Widget::Color | Widget::Position => {
            let tag = match input.widget {
                Widget::Angle => "guiangle",
                Widget::Color => "guicolor",
                Widget::Position => "guiposition",
                _ => "guislider",
            };
            let labels: String = input.labels.iter().enumerate().map(|(i, l)| format!(r#" label{i}="{}""#, escape(l))).collect();
            let _ = writeln!(
                x,
                r#"      <{tag} min="{}" max="{}" step="{}" clamp="{}"{labels}/>"#,
                numbers(&input.min),
                numbers(&input.max),
                input.step.map(|s| format_numbers(&[s], integers)).unwrap_or_default(),
                if input.clamp { "on" } else { "off" }
            );
        }
        Widget::Toggle => {
            let label = |i: usize| input.labels.get(i).map(|l| escape(l)).unwrap_or_default();
            let _ = writeln!(x, r#"      <guibutton label0="{}" label1="{}"/>"#, label(0), label(1));
        }
        Widget::ComboBox | Widget::EnumButtons => {
            let tag = if input.widget == Widget::ComboBox { "guicombobox" } else { "guienumbuttons" };
            let _ = writeln!(x, r#"      <{tag}>"#);
            for (value, text) in &input.choices {
                let _ = writeln!(x, r#"       <{tag}item value="{value}" text="{}"/>"#, escape(text));
            }
            let _ = writeln!(x, r#"      </{tag}>"#);
        }
        Widget::Image => {
            let _ = writeln!(x, r#"      <guiimage colortype="color">"#);
            let _ = writeln!(x, r#"       <channels count="{}">"#, input.usages.len());
            for usage in &input.usages {
                let _ = writeln!(x, r#"        <channel names="{}"/>"#, escape(usage));
            }
            let _ = writeln!(x, r#"       </channels>"#);
            let _ = writeln!(x, r#"      </guiimage>"#);
        }
        Widget::Text | Widget::Other => {}
    }
    let _ = writeln!(x, r#"     </inputgui>"#);
    let _ = writeln!(x, r#"    </input>"#);
}

/// An `.sbsar`: the description in a 7-Zip archive, with a stand-in for
/// the compiled graph (`SBAM`, the four bytes a real one starts with) and
/// each graph's thumbnail as an icon file, laid out as a published file is
/// (`assemblies/content/0000/`).
pub fn sbsar(name: &str, graphs: &[Graph]) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = vec![
        (format!("{name}.xml"), description_xml(graphs).into_bytes()),
        (format!("{name}.sbsasm"), b"SBAM tequila fixture: not a compiled graph".to_vec()),
    ];
    for (index, graph) in graphs.iter().enumerate() {
        if let Some(picture) = &graph.thumbnail {
            entries.push((format!("assemblies/content/0000/icon_{index}.png"), picture.clone()));
        }
    }
    seven_zip(&entries)
}

/// A 7-Zip archive of `entries`, LZMA2-compressed as the writer does by
/// default.
pub fn seven_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut writer = sevenz_rust2::SevenZWriter::new(std::io::Cursor::new(Vec::new())).expect("a writer over memory");
    for (name, bytes) in entries {
        writer
            .push_archive_entry(sevenz_rust2::SevenZArchiveEntry::new_file(name), Some(bytes.as_slice()))
            .expect("writing to memory");
    }
    writer.finish().expect("writing to memory").into_inner()
}

/// The `.sbs` Designer would save for `graphs`, as far as Tequila reads one.
pub fn sbs_xml(graphs: &[Graph]) -> String {
    let mut x = String::new();
    let _ = writeln!(x, r#"<?xml version="1.0" encoding="UTF-8"?><package><identifier v="Unsaved Package"/><formatVersion v="1.1.0.202201"/><updaterVersion v="1.1.0.202201"/><fileUID v="{{00000000-0000-0000-0000-000000000000}}"/><versionUID v="0"/><dependencies/><content>"#);
    for g in graphs {
        let _ = write!(x, r#"<graph><identifier v="{}"/><uid v="1"/><attributes>"#, escape(g.identifier()));
        for (tag, value) in [("label", &g.label), ("category", &g.category), ("author", &g.author), ("authorURL", &g.author_url), ("description", &g.description), ("tags", &g.keywords)] {
            if !value.is_empty() {
                let _ = write!(x, r#"<{tag} v="{}"/>"#, escape(value));
            }
        }
        if let Some(picture) = &g.thumbnail {
            use base64::Engine;
            let _ = write!(
                x,
                r#"<icon><datalength v="{}"/><format v="png"/><strdata v="{}"/></icon>"#,
                picture.len(),
                base64::engine::general_purpose::STANDARD.encode(picture)
            );
        }
        let _ = write!(x, "</attributes><paraminputs>");
        for input in g.inputs.iter().filter(|i| !i.is_system()) {
            let (code, tag) = sbs_type(input);
            let default = value_text(&input.default, input.value_type).replace(',', " ");
            let _ = write!(
                x,
                r#"<paraminput><identifier v="{}"/><uid v="{}"/><attributes><label v="{}"/><description v="{}"/></attributes><type v="{code}"/>"#,
                escape(&input.identifier),
                escape(&input.uid),
                escape(&input.label),
                escape(&input.description)
            );
            if let Some(tag) = tag {
                let _ = write!(x, r#"<defaultValue><{tag} v="{}"/></defaultValue>"#, escape(&default));
            }
            let widget = match input.widget {
                Widget::Toggle => "buttons",
                Widget::Image => "",
                other => other.name(),
            };
            let _ = write!(x, r#"<defaultWidget><name v="{widget}"/><options>"#);
            let integers = matches!(input.value_type, ValueType::Integer(_));
            let mut option = |name: &str, value: String| {
                let _ = write!(x, r#"<option><name v="{name}"/><value v="{}"/></option>"#, escape(&value));
            };
            if let Some(min) = &input.min {
                option("min", format_numbers(min, integers).replace(',', ";"));
            }
            if let Some(max) = &input.max {
                option("max", format_numbers(max, integers).replace(',', ";"));
            }
            if let Some(step) = input.step {
                option("step", format_numbers(&[step], integers));
            }
            if input.clamp {
                option("clamp", "1".into());
            }
            for (i, label) in input.labels.iter().enumerate() {
                option(&format!("label{i}"), label.clone());
            }
            if !input.choices.is_empty() {
                let mut fields = vec![input.choices.len().to_string()];
                for (value, text) in &input.choices {
                    fields.push(value.to_string());
                    fields.push(text.clone());
                }
                option("parameters", fields.join(";"));
            }
            let _ = write!(x, "</options></defaultWidget>");
            if !input.group.is_empty() {
                let _ = write!(x, r#"<group v="{}"/>"#, escape(&input.group));
            }
            if !input.visible_if.is_empty() {
                let _ = write!(x, r#"<visibleIf v="{}"/>"#, escape(&input.visible_if));
            }
            let _ = write!(x, "</paraminput>");
        }
        let _ = write!(x, "</paraminputs><graphOutputs>");
        for output in &g.outputs {
            let _ = write!(
                x,
                r#"<graphoutput><identifier v="{}"/><uid v="{}"/><attributes><label v="{}"/></attributes><usages>"#,
                escape(&output.identifier),
                escape(&output.uid),
                escape(&output.label)
            );
            for usage in &output.usages {
                let _ = write!(x, r#"<usage><components v="RGBA"/><name v="{}"/></usage>"#, escape(usage));
            }
            let _ = write!(x, "</usages>");
            if !output.group.is_empty() {
                let _ = write!(x, r#"<group v="{}"/>"#, escape(&output.group));
            }
            let _ = write!(x, "</graphoutput>");
        }
        let _ = write!(x, "</graphOutputs><compNodes/>");
        let size = g.input("$outputsize").map(|i| value_text(&i.default, i.value_type).replace(',', " ")).unwrap_or_else(|| "10 10".into());
        let _ = write!(
            x,
            r#"<baseParameters><parameter><name v="outputsize"/><relativeTo v="0"/><paramValue><constantValueInt2 v="{size}"/></paramValue></parameter></baseParameters>"#
        );
        if !g.presets.is_empty() {
            let _ = write!(x, "<sbspresets>");
            for preset in &g.presets {
                let _ = write!(x, r#"<sbspreset><label v="{}"/><presetinputs>"#, escape(&preset.label));
                for (id, value) in &preset.values {
                    let input = g.input(id);
                    let (code, tag) = input.map(sbs_type).unwrap_or((256, Some("constantValueFloat1")));
                    let _ = write!(
                        x,
                        r#"<presetinput><identifier v="{}"/><uid v="{}"/><type v="{code}"/><value><{} v="{}"/></value></presetinput>"#,
                        escape(id),
                        input.map(|i| escape(&i.uid)).unwrap_or_default(),
                        tag.unwrap_or("constantValueString"),
                        escape(&value.replace(',', " "))
                    );
                }
                let _ = write!(x, "</presetinputs></sbspreset>");
            }
            let _ = write!(x, "</sbspresets>");
        }
        let _ = write!(x, "</graph>");
    }
    let _ = write!(x, "</content></package>");
    x
}

/// An input's `.sbs` type code and constant tag.
fn sbs_type(input: &Input) -> (i64, Option<&'static str>) {
    match (input.value_type, input.widget) {
        (ValueType::Integer(1), Widget::Toggle) => (4, Some("constantValueBool")),
        (ValueType::Integer(1), _) => (16, Some("constantValueInt1")),
        (ValueType::Integer(2), _) => (32, Some("constantValueInt2")),
        (ValueType::Integer(3), _) => (64, Some("constantValueInt3")),
        (ValueType::Integer(_), _) => (128, Some("constantValueInt4")),
        (ValueType::Float(1), _) => (256, Some("constantValueFloat1")),
        (ValueType::Float(2), _) => (512, Some("constantValueFloat2")),
        (ValueType::Float(3), _) => (1024, Some("constantValueFloat3")),
        (ValueType::Float(_), _) => (2048, Some("constantValueFloat4")),
        (ValueType::String, _) => (16384, Some("constantValueString")),
        (ValueType::Image | ValueType::Font, _) => (1, None),
    }
}

fn input(identifier: &str, label: &str, group: &str, value_type: ValueType, widget: Widget, default: &[f64]) -> Input {
    let mut input = Input::new(identifier, value_type);
    input.uid = format!("{}", 1000 + identifier.len() * 7 + label.len());
    input.label = label.into();
    input.group = group.into();
    input.widget = widget;
    input.default = Value::Numbers(default.to_vec());
    input
}

fn output(identifier: &str, label: &str, usage: &str) -> Output {
    Output {
        identifier: identifier.into(),
        uid: format!("{}", 2000 + identifier.len()),
        label: label.into(),
        group: "Material".into(),
        usages: vec![usage.into()],
    }
}

/// A small PNG of one colour with a lighter square in it, for an icon.
pub fn icon_png(rgb: [u8; 3], size: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((size * size * 3) as usize);
    for y in 0..size {
        for x in 0..size {
            let inside = x > size / 4 && x < size * 3 / 4 && y > size / 4 && y < size * 3 / 4;
            for c in rgb {
                data.push(if inside { c.saturating_add(60) } else { c });
            }
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, size, size);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().expect("encoding to memory").write_image_data(&data).expect("encoding to memory");
    }
    out
}

/// The synthetic material the tests and the pictures use: a brick wall,
/// with every kind of input the format has — a colour, sliders of one and
/// two components, a whole number, a switch, a drop-down (the Normal format
/// every Substance Source material has), a string and an image input — the
/// system inputs, a preset, and outputs for every usage Tequila takes.
pub fn bricks() -> Graph {
    let mut color = input("brick_color", "Brick colour", "Bricks", ValueType::Float(4), Widget::Color, &[0.62, 0.25, 0.16, 1.0]);
    color.description = "The colour of the bricks' faces".into();
    let mut mortar = input("mortar_color", "Mortar colour", "Mortar", ValueType::Float(3), Widget::Color, &[0.78, 0.76, 0.72]);
    mortar.description = "Between the bricks".into();
    let mut rows = input("rows", "Rows", "Bricks", ValueType::Integer(1), Widget::Slider, &[8.0]);
    rows.min = Some(vec![1.0]);
    rows.max = Some(vec![32.0]);
    rows.step = Some(1.0);
    rows.clamp = true;
    let mut roughness = input("roughness", "Roughness", "Surface", ValueType::Float(1), Widget::Slider, &[0.7]);
    roughness.min = Some(vec![0.0]);
    roughness.max = Some(vec![1.0]);
    roughness.step = Some(0.01);
    roughness.clamp = true;
    let mut offset = input("offset", "Offset", "Bricks", ValueType::Float(2), Widget::Slider, &[0.5, 0.0]);
    offset.min = Some(vec![0.0, 0.0]);
    offset.max = Some(vec![1.0, 1.0]);
    offset.step = Some(0.01);
    offset.labels = vec!["X".into(), "Y".into()];
    let mut worn = input("worn", "Worn edges", "Surface", ValueType::Integer(1), Widget::Toggle, &[1.0]);
    worn.labels = vec!["Off".into(), "On".into()];
    let mut format = input("normal_format", "Normal format", "Technical", ValueType::Integer(1), Widget::ComboBox, &[0.0]);
    format.choices = vec![(0, "DirectX".into()), (1, "OpenGL".into())];
    let mut stamp = Input::new("stamp", ValueType::String);
    stamp.label = "Stamp".into();
    stamp.group = "Technical".into();
    stamp.widget = Widget::Text;
    stamp.default = Value::Text("TQ".into());
    let mut dirt = Input::new("dirt_mask", ValueType::Image);
    dirt.label = "Dirt mask".into();
    dirt.group = "Surface".into();
    dirt.widget = Widget::Image;
    dirt.usages = vec!["mask".into()];
    let mut size = super::output_size_input(vec![8.0, 8.0]);
    size.uid = "1279484140".into();
    let mut seed = super::random_seed_input();
    seed.uid = "1279484141".into();
    Graph {
        url: "pkg://bricks_tequila".into(),
        label: "Tequila Test Bricks".into(),
        category: "Architecture/Brick".into(),
        author: "Tequila test fixture".into(),
        author_url: "https://github.com/ifpintac/tequila".into(),
        description: "A synthetic Substance graph written by Tequila's tests from the published format. Not made by Adobe's tools.".into(),
        keywords: "brick wall test synthetic".into(),
        hidden: false,
        physical_size: Some([100.0, 100.0, 2.0]),
        inputs: vec![size, seed, color, mortar, rows, offset, roughness, worn, format, stamp, dirt],
        outputs: vec![
            output("basecolor", "Base Color", "baseColor"),
            output("normal", "Normal", "normal"),
            output("roughness", "Roughness", "roughness"),
            output("metallic", "Metallic", "metallic"),
            output("height", "Height", "height"),
            output("ambientocclusion", "Ambient Occlusion", "ambientOcclusion"),
            output("specularlevel", "Specular Level", "specularLevel"),
        ],
        presets: vec![
            Preset {
                label: "Whitewashed".into(),
                values: vec![("brick_color".into(), "0.86,0.84,0.8,1".into()), ("roughness".into(), "0.9".into())],
            },
            Preset {
                label: "Glazed".into(),
                values: vec![("brick_color".into(), "0.1,0.25,0.45,1".into()), ("roughness".into(), "0.15".into()), ("worn".into(), "0".into())],
            },
        ],
        thumbnail: Some(icon_png([150, 64, 40], 64)),
    }
}

/// [`bricks`] as an `.sbsar`'s bytes.
pub fn bricks_sbsar() -> Vec<u8> {
    sbsar("bricks_tequila", &[bricks()])
}

/// [`bricks`] as an `.sbs`.
pub fn bricks_sbs() -> String {
    sbs_xml(&[bricks()])
}

/// A file of these graphs.
pub fn file(graphs: Vec<Graph>) -> SubstanceFile {
    SubstanceFile { kind: FileKind::Sbsar, format_version: "1.1".into(), graphs }
}
