//! The `substance` operation's answers that need no window: a Substance
//! file described, the tools found, and a request's values read. The
//! application and the headless server answer from these, so a script reads
//! the same from either.

use std::collections::BTreeMap;
use std::path::Path;
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_import::substance::{bridge, FileKind, Graph, Input, InputKind, SubstanceFile, Value as SubstanceValue};

fn numbers(values: &[f64]) -> Value {
    Value::Array(values.iter().map(|v| Value::Number(*v)).collect())
}

fn kind_name(input: &Input) -> String {
    match input.kind() {
        InputKind::Float(n) => format!("float{n}"),
        InputKind::Integer(n) => format!("integer{n}"),
        InputKind::Toggle => "boolean".into(),
        InputKind::Color { alpha: true } => "color_rgba".into(),
        InputKind::Color { alpha: false } => "color_rgb".into(),
        InputKind::Choice => "choice".into(),
        InputKind::Text => "string".into(),
        InputKind::Font => "font".into(),
        InputKind::Image => "image".into(),
    }
}

fn value(value: &SubstanceValue) -> Value {
    match value {
        SubstanceValue::Numbers(n) if n.len() == 1 => Value::Number(n[0]),
        SubstanceValue::Numbers(n) => numbers(n),
        SubstanceValue::Text(t) => Value::String(t.clone()),
    }
}

/// One input, as a script sees it.
pub fn input(input: &Input) -> Value {
    let mut out = Object::new();
    out.set("identifier", input.identifier.clone())
        .set("label", input.name())
        .set("group", input.group.clone())
        .set("type", kind_name(input))
        .set("widget", input.widget.name())
        .set("default", value(&input.default))
        .set("system", input.is_system());
    if !input.description.is_empty() {
        out.set("description", input.description.clone());
    }
    if let Some(min) = &input.min {
        out.set("min", numbers(min));
    }
    if let Some(max) = &input.max {
        out.set("max", numbers(max));
    }
    if let Some(step) = input.step {
        out.set("step", step);
    }
    if input.clamp {
        out.set("clamp", true);
    }
    if !input.choices.is_empty() {
        out.set(
            "choices",
            Value::Array(
                input
                    .choices
                    .iter()
                    .map(|(v, text)| {
                        let mut choice = Object::new();
                        choice.set("value", *v as f64).set("label", text.clone());
                        choice.build()
                    })
                    .collect(),
            ),
        );
    }
    if !input.labels.is_empty() {
        out.set("labels", Value::Array(input.labels.iter().map(|l| Value::String(l.clone())).collect()));
    }
    if !input.visible_if.is_empty() {
        out.set("visible_if", input.visible_if.clone());
    }
    if !input.usages.is_empty() {
        out.set("usages", Value::Array(input.usages.iter().map(|u| Value::String(u.clone())).collect()));
    }
    out.build()
}

/// One graph, as a script sees it: its inputs, outputs (with the part each
/// plays in a Tequila material) and presets.
pub fn graph(graph: &Graph) -> Value {
    let directx = graph.normal_is_directx(&BTreeMap::new());
    let mut out = Object::new();
    out.set("url", graph.url.clone())
        .set("label", graph.name())
        .set("category", graph.category.clone())
        .set("author", graph.author.clone())
        .set("author_url", graph.author_url.clone())
        .set("description", graph.description.clone())
        .set("keywords", graph.keywords.clone())
        .set("hidden", graph.hidden)
        .set("thumbnail", graph.thumbnail.is_some())
        .set("normal_format", if directx { "directx" } else { "opengl" })
        .set("inputs", Value::Array(graph.inputs.iter().map(input).collect()))
        .set(
            "outputs",
            Value::Array(
                graph
                    .outputs
                    .iter()
                    .map(|o| {
                        let mut output = Object::new();
                        output
                            .set("identifier", o.identifier.clone())
                            .set("label", o.label.clone())
                            .set("usages", Value::Array(o.usages.iter().map(|u| Value::String(u.clone())).collect()));
                        match o.slot(directx) {
                            Ok(slot) => output.set("map", slot.label()),
                            Err(why) => output.set("unused", why),
                        };
                        output.build()
                    })
                    .collect(),
            ),
        )
        .set(
            "presets",
            Value::Array(
                graph
                    .presets
                    .iter()
                    .map(|p| {
                        let mut preset = Object::new();
                        preset.set("label", p.label.clone()).set(
                            "values",
                            Value::Object(p.values_for(graph).into_iter().map(|(k, v)| (k, Value::String(v))).collect()),
                        );
                        preset.build()
                    })
                    .collect(),
            ),
        );
    if let Some(size) = graph.physical_size {
        out.set("physical_size_cm", Value::Array(size.iter().map(|v| Value::Number(*v as f64)).collect()));
    }
    out.build()
}

/// Adobe's tools as found.
pub fn tools(tools: &bridge::Tools) -> Value {
    let mut out = Object::new();
    out.set_some("sbsrender", tools.sbsrender.as_ref().map(|p| p.display().to_string()))
        .set_some("sbscooker", tools.sbscooker.as_ref().map(|p| p.display().to_string()))
        .set("can_render", tools.can_render());
    if !tools.can_render() {
        out.set("install", bridge::INSTALL_HINT);
    }
    out.build()
}

/// `info`: a file, described.
pub fn info(path: &Path, file: &SubstanceFile, found: &bridge::Tools) -> Value {
    let mut out = Object::new();
    out.set("path", path.display().to_string())
        .set("kind", if file.kind == FileKind::Sbsar { "sbsar" } else { "sbs" })
        .set("format_version", file.format_version.clone())
        .set("graphs", Value::Array(file.graphs.iter().map(graph).collect()))
        .set("tools", tools(found))
        .set("can_render", found.can_render_kind(file.kind));
    out.build()
}

/// A request's `values`, as a material keeps them: a number, a list of
/// numbers or a string for each input, typed by `graph`. `Err` naming an
/// input the graph does not have.
pub fn values(request: &Value, graph: &Graph) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let Some(given) = request.get("values") else {
        return Ok(out);
    };
    let Value::Object(map) = given else {
        return Err("`values` must be an object of input identifier to value".into());
    };
    for (id, v) in map {
        let input = graph.input(id).ok_or_else(|| {
            let known: Vec<&str> = graph.inputs.iter().map(|i| i.identifier.as_str()).collect();
            format!("the graph has no input `{id}`; it has {}", known.join(", "))
        })?;
        let parsed = match v {
            Value::Number(n) => SubstanceValue::Numbers(vec![*n]),
            Value::Bool(b) => SubstanceValue::Numbers(vec![if *b { 1.0 } else { 0.0 }]),
            Value::Array(items) => SubstanceValue::Numbers(items.iter().filter_map(|i| i.as_f64()).collect()),
            Value::String(s) if matches!(input.kind(), InputKind::Text | InputKind::Font | InputKind::Image) => SubstanceValue::Text(s.clone()),
            Value::String(s) => SubstanceValue::Numbers(SubstanceValue::parse(s)),
            _ => return Err(format!("`{id}` needs a number, a list of numbers or a string")),
        };
        if let SubstanceValue::Numbers(n) = &parsed {
            let want = input.value_type.components();
            if want > 0 && n.len() != want {
                return Err(format!("`{id}` takes {want} number{}, and was given {}", if want == 1 { "" } else { "s" }, n.len()));
            }
        }
        out.insert(id.clone(), parsed.to_text(input.value_type));
    }
    Ok(out)
}

/// An output size in pixels (`2048`) as `$outputsize`'s power of two.
pub fn output_size(pixels: f64) -> Result<String, String> {
    if !(16.0..=8192.0).contains(&pixels) {
        return Err("`output_size` is in pixels, 16 to 8192".into());
    }
    let power = pixels.log2().round();
    Ok(tequila_import::substance::format_numbers(&[power, power], true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_import::substance::fixture;

    #[test]
    fn a_file_is_described_for_a_script() {
        let file = fixture::file(vec![fixture::bricks()]);
        let out = info(Path::new("C:/m/bricks.sbsar"), &file, &bridge::Tools::default());
        assert_eq!(out.get("can_render").and_then(|v| v.as_bool()), Some(false));
        assert!(out.get("tools").and_then(|t| t.get("install")).is_some(), "it says what to install");
        let graph = out.get("graphs").unwrap().index(0).unwrap();
        assert_eq!(graph.get("url").and_then(|v| v.as_str()), Some("pkg://bricks_tequila"));
        let inputs = graph.get("inputs").unwrap().as_array();
        let colour = inputs.iter().find(|i| i.get("identifier").and_then(|v| v.as_str()) == Some("brick_color")).unwrap();
        assert_eq!(colour.get("type").and_then(|v| v.as_str()), Some("color_rgba"));
        let format = inputs.iter().find(|i| i.get("identifier").and_then(|v| v.as_str()) == Some("normal_format")).unwrap();
        assert_eq!(format.get("choices").unwrap().as_array().len(), 2);
        let outputs = graph.get("outputs").unwrap().as_array();
        assert!(outputs.iter().any(|o| o.get("unused").is_some()), "the specular map is said to be unused");
        assert_eq!(graph.get("normal_format").and_then(|v| v.as_str()), Some("directx"));
    }

    #[test]
    fn values_are_read_by_type() {
        let graph = fixture::bricks();
        let request = tequila_format::json::parse(br#"{"values": {"brick_color": [1, 0, 0, 1], "rows": 12, "worn": false, "stamp": "AB"}}"#).unwrap();
        let read = values(&request, &graph).expect("reads");
        assert_eq!(read.get("brick_color").map(String::as_str), Some("1,0,0,1"));
        assert_eq!(read.get("rows").map(String::as_str), Some("12"));
        assert_eq!(read.get("worn").map(String::as_str), Some("0"));
        assert_eq!(read.get("stamp").map(String::as_str), Some("AB"));
        let wrong = tequila_format::json::parse(br#"{"values": {"brick_color": [1, 0]}}"#).unwrap();
        assert!(values(&wrong, &graph).unwrap_err().contains("takes 4"));
        let unknown = tequila_format::json::parse(br#"{"values": {"nope": 1}}"#).unwrap();
        assert!(values(&unknown, &graph).unwrap_err().contains("no input `nope`"));
        assert_eq!(output_size(2048.0).unwrap(), "11,11");
        assert!(output_size(3.0).is_err());
    }
}
