//! CAD metadata: finding parts by it, and the scene tree as a spreadsheet
//! (`docs/03` §6.5).
//!
//! "Filtering/selection by metadata (`select all parts where Material ==
//! "ABS"`) — very useful for bulk material assignment", and "CSV export of
//! the scene tree with selected metadata columns".
//!
//! A search is typed into the Scene tab's search box, so the syntax is what
//! a person would type there:
//!
//! - `ABS` — a name, or any field's value, containing the text;
//! - `Material = ABS` (or `==`) — that field equal to the value, which may use
//!   the Material Templates' wildcards: `Material = AB*`;
//! - `Material != ABS` — that field present and not equal to it;
//! - `Material: AB` — that field containing the text;
//! - `Material =` — that field present, whatever it says.
//!
//! Field names and values match regardless of case, as CAD exporters do not
//! agree on either. Quotes around a value are taken off.

use crate::document::SceneDocument;
use crate::templates::wildcard;
use crate::tree::{NodeId, NodeKind, SceneNode};

/// What a search asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    /// A name or any field's value containing the text.
    Text(String),
    /// A field whose value matches, by wildcard.
    Equals { key: String, value: String },
    /// A field present whose value does not match.
    NotEquals { key: String, value: String },
    /// A field whose value contains the text.
    Contains { key: String, value: String },
    /// A field present.
    Has(String),
}

impl Query {
    /// Read what was typed. `None` for nothing.
    pub fn parse(text: &str) -> Option<Query> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let unquote = |v: &str| -> String {
            let v = v.trim();
            let quoted = v.len() >= 2
                && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\'')));
            if quoted { v[1..v.len() - 1].to_string() } else { v.to_string() }
        };
        // The first operator decides, so a value may hold the others.
        let first = ["!=", "==", "=", ":"]
            .iter()
            .filter_map(|op| text.find(op).map(|at| (at, *op)))
            .min_by_key(|(at, op)| (*at, std::cmp::Reverse(op.len())));
        let Some((at, op)) = first else {
            return Some(Query::Text(text.to_string()));
        };
        let key = text[..at].trim().to_string();
        let value = unquote(&text[at + op.len()..]);
        if key.is_empty() {
            return Some(Query::Text(text.to_string()));
        }
        Some(match op {
            "!=" => Query::NotEquals { key, value },
            ":" if !value.is_empty() => Query::Contains { key, value },
            _ if value.is_empty() => Query::Has(key),
            _ => Query::Equals { key, value },
        })
    }

    /// Whether `node` is one the query asks for.
    pub fn matches(&self, node: &SceneNode) -> bool {
        let field = |key: &str| node.metadata.iter().find(|(k, _)| k.trim().eq_ignore_ascii_case(key)).map(|(_, v)| v);
        match self {
            Query::Text(text) => {
                let text = text.to_lowercase();
                node.name.to_lowercase().contains(&text)
                    || node.metadata.values().any(|v| v.to_lowercase().contains(&text))
            }
            Query::Equals { key, value } => field(key).is_some_and(|v| wildcard(value, v.trim())),
            Query::NotEquals { key, value } => field(key).is_some_and(|v| !wildcard(value, v.trim())),
            Query::Contains { key, value } => {
                field(key).is_some_and(|v| v.to_lowercase().contains(&value.to_lowercase()))
            }
            Query::Has(key) => field(key).is_some(),
        }
    }
}

/// Every node `query` matches, in tree order.
pub fn find(document: &SceneDocument, query: &Query) -> Vec<NodeId> {
    document
        .tree
        .depth_first()
        .into_iter()
        .filter(|&id| document.tree.get(id).is_some_and(|node| query.matches(node)))
        .collect()
}

/// The scene tree as CSV: a row a node, in tree order, with its path, name,
/// type, material and visibility, then one column for each of `columns`.
///
/// RFC 4180, with CRLF line ends and a byte-order mark, which is what makes
/// Excel read UTF-8 names as UTF-8 rather than as mojibake. A value that a
/// spreadsheet would run as a formula (`=`, `+`, `@`, or `-` before anything
/// but a number) is written with a leading apostrophe, as spreadsheets
/// themselves mark text: a CAD file from somewhere else is data, never a
/// formula to evaluate.
pub fn csv(document: &SceneDocument, columns: &[String]) -> String {
    let mut out = String::from("\u{feff}");
    let mut row = |cells: Vec<String>| {
        let line: Vec<String> = cells.iter().map(|c| cell(c)).collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    };
    let mut header: Vec<String> = ["Path", "Name", "Type", "Assigned material", "Visible"].iter().map(|s| s.to_string()).collect();
    header.extend(columns.iter().cloned());
    row(header);
    for id in document.tree.depth_first() {
        let Some(node) = document.tree.get(id) else { continue };
        let kind = match node.kind {
            NodeKind::Group => "Group",
            NodeKind::Part { .. } => "Part",
            NodeKind::Light { .. } => "Light",
            NodeKind::Cutaway { .. } => "Cutaway",
        };
        let material = node
            .material
            .and_then(|key| document.materials.get(key))
            .map(|m| m.name.clone())
            .unwrap_or_default();
        let mut cells = vec![
            document.tree.path_of(id),
            node.name.clone(),
            kind.to_string(),
            material,
            if node.visible { "Yes" } else { "No" }.to_string(),
        ];
        for column in columns {
            cells.push(node.metadata.get(column).cloned().unwrap_or_default());
        }
        row(cells);
    }
    out
}

/// One CSV field, quoted when it has to be and defused when it would be a
/// formula.
fn cell(value: &str) -> String {
    let number = value.parse::<f64>().is_ok();
    let formula = matches!(value.chars().next(), Some('=' | '+' | '@' | '\t' | '\r'))
        || (value.starts_with('-') && !number);
    let value = if formula { format!("'{value}") } else { value.to_string() };
    if value.contains([',', '"', '\n', '\r']) || value.starts_with(' ') || value.ends_with(' ') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, ImportPayload};
    use crate::geometry::Mesh;
    use crate::material::MaterialDefinition;
    use crate::CommandBus;
    use tequila_core::math::{vec3, Vec3};

    /// Assembly
    ///   Housing   Material=ABS, PartNumber=HS-4410
    ///   Screw A   Material=Steel, Finish="Zinc, bright"
    ///   Screw B   Material=steel
    fn assembly() -> (SceneDocument, Vec<NodeId>) {
        let mut document = SceneDocument::new();
        let mut bus = CommandBus::new();
        let mesh = Mesh::new(vec![Vec3::ZERO, vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)], vec![0, 1, 2]);
        let (meshes, materials, ids) = crate::command::reserve_keys(&document, 1, 1, 4);
        let mut root = SceneNode::new(ids[0], "Assembly", NodeKind::Group);
        root.children = ids[1..].to_vec();
        let mut nodes = vec![root];
        for (index, name, fields) in [
            (1, "Housing", vec![("Material", "ABS"), ("PartNumber", "HS-4410")]),
            (2, "Screw A", vec![("Material", "Steel"), ("Finish", "Zinc, bright")]),
            (3, "Screw B", vec![("material ", "steel")]),
        ] {
            let mut node = SceneNode::new(ids[index], name, NodeKind::Part { mesh: meshes[0] });
            node.parent = Some(ids[0]);
            node.material = Some(materials[0]);
            for (k, v) in fields {
                node.metadata.insert(k.into(), v.into());
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
        (document, ids)
    }

    #[test]
    fn a_search_reads_as_it_is_typed() {
        let parse = |s| Query::parse(s).unwrap();
        assert_eq!(parse("bolt"), Query::Text("bolt".into()));
        assert_eq!(parse(r#"Material == "ABS""#), Query::Equals { key: "Material".into(), value: "ABS".into() });
        assert_eq!(parse("Material=AB*"), Query::Equals { key: "Material".into(), value: "AB*".into() });
        assert_eq!(parse("Material != ABS"), Query::NotEquals { key: "Material".into(), value: "ABS".into() });
        assert_eq!(parse("Finish: zinc"), Query::Contains { key: "Finish".into(), value: "zinc".into() });
        assert_eq!(parse("PartNumber ="), Query::Has("PartNumber".into()));
        // The first operator decides; the value keeps the rest.
        assert_eq!(parse("Note: a=b"), Query::Contains { key: "Note".into(), value: "a=b".into() });
        assert_eq!(parse("= x"), Query::Text("= x".into()));
        assert_eq!(Query::parse("   "), None);
    }

    /// `select all parts where Material == "ABS"`, and its relatives, find
    /// what a person means: any case, stray spaces in a CAD key, wildcards.
    #[test]
    fn parts_are_found_by_their_fields() {
        let (document, ids) = assembly();
        let find = |s: &str| find(&document, &Query::parse(s).unwrap());
        assert_eq!(find(r#"Material == "ABS""#), vec![ids[1]]);
        assert_eq!(find("material = STEEL"), vec![ids[2], ids[3]]);
        assert_eq!(find("Material = s*"), vec![ids[2], ids[3]]);
        assert_eq!(find("Material != steel"), vec![ids[1]]);
        assert_eq!(find("Finish: zinc"), vec![ids[2]]);
        assert_eq!(find("PartNumber ="), vec![ids[1]]);
        // Plain text finds names and values alike.
        assert_eq!(find("screw"), vec![ids[2], ids[3]]);
        assert_eq!(find("hs-44"), vec![ids[1]]);
        assert!(find("Colour = red").is_empty());
    }

    #[test]
    fn the_tree_exports_as_a_spreadsheet() {
        let (document, _) = assembly();
        let text = csv(&document, &["Material".into(), "Finish".into(), "PartNumber".into()]);
        assert!(text.starts_with('\u{feff}'), "Excel needs the mark to read UTF-8");
        let lines: Vec<&str> = text.trim_start_matches('\u{feff}').split("\r\n").collect();
        assert_eq!(lines[0], "Path,Name,Type,Assigned material,Visible,Material,Finish,PartNumber");
        assert_eq!(lines[1], "Assembly,Assembly,Group,,Yes,,,");
        assert_eq!(lines[2], "Assembly/Housing,Housing,Part,Shell,Yes,ABS,,HS-4410");
        assert_eq!(lines[3], "Assembly/Screw A,Screw A,Part,Shell,Yes,Steel,\"Zinc, bright\",");
        assert_eq!(lines.len(), 6, "four rows, a header and the final line end");
    }

    #[test]
    fn a_cell_is_quoted_and_never_a_formula() {
        assert_eq!(cell("plain"), "plain");
        assert_eq!(cell("a,b"), "\"a,b\"");
        assert_eq!(cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(cell("two\nlines"), "\"two\nlines\"");
        assert_eq!(cell("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(cell("@SUM(A1)"), "'@SUM(A1)");
        assert_eq!(cell("-12.5"), "-12.5", "a negative number stays a number");
        assert_eq!(cell("-1+cmd"), "'-1+cmd");
    }
}
