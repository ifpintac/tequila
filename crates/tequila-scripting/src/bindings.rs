//! The Python module's command half, and the API reference, generated from
//! the command catalog (`docs/04` §8.3: "Python API is generated from this
//! catalog").
//!
//! Each command's fields are read from its example in
//! `tequila_format::examples` - the journal's own round-trip samples, so a
//! binding's keywords are exactly what the decoder accepts - and its
//! description from the doc comment on its variant in `tequila_scene`'s
//! `Command`. `cargo run -p tequila-scripting --example bindings` writes
//! `python/tequila_commands.py` and `docs/scripting-api.md`; a test fails
//! when either falls behind the build, which is `docs/05` §3.8's "every
//! command has a Python binding" as a check rather than a hope.

use tequila_format::json::Value;
use tequila_scene::command::Command;

/// Python's reserved words, which a field cannot be named as a keyword
/// argument: such a field takes a trailing underscore.
const KEYWORDS: &[&str] = &[
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or",
    "pass", "raise", "return", "try", "while", "with", "yield",
];

/// One command, as the bindings describe it.
pub struct Binding {
    pub name: &'static str,
    /// Its fields, in the order the journal writes them.
    pub fields: Vec<String>,
    /// Its example, one line of JSON, as `apply` takes it.
    pub example: String,
    /// The first paragraph of its documentation.
    pub summary: String,
}

/// Every command in the catalog, in catalog order.
pub fn bindings() -> Vec<Binding> {
    let examples = tequila_format::examples::commands();
    let docs = summaries();
    Command::CATALOG
        .iter()
        .map(|&name| {
            let example = examples.iter().find(|c| c.name() == name).map(tequila_format::journal::encode).unwrap_or_default();
            let fields = match tequila_format::json::parse(example.as_bytes()) {
                Ok(Value::Object(entries)) => entries.into_iter().map(|(k, _)| k).filter(|k| k != "command").collect(),
                _ => Vec::new(),
            };
            Binding { name, fields, example: example.trim().to_string(), summary: docs.get(name).cloned().unwrap_or_default() }
        })
        .collect()
}

/// Each command's first paragraph of documentation, from its variant's
/// doc comment, which begins by naming it: `/// \`scene.group\` - collect…`.
fn summaries() -> std::collections::BTreeMap<String, String> {
    let source = include_str!("../../tequila-scene/src/command.rs");
    let body = source.split("pub enum Command {").nth(1).unwrap_or("");
    let mut out = std::collections::BTreeMap::new();
    let mut block: Vec<String> = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if let Some(doc) = line.strip_prefix("///") {
            block.push(doc.trim().to_string());
            continue;
        }
        if line == "}" {
            break;
        }
        if !block.is_empty() {
            let first = block[0].clone();
            if let Some(name) = first.strip_prefix('`').and_then(|r| r.split('`').next()) {
                let after = first.split_once('`').and_then(|(_, r)| r.split_once('`')).map(|(_, r)| r.trim()).unwrap_or("");
                let lead = after.trim_start_matches(['—', '-', ':', ' ']).trim();
                let paragraph: Vec<&str> = if lead.is_empty() {
                    block.iter().skip(1).skip_while(|l| l.is_empty()).take_while(|l| !l.is_empty()).map(String::as_str).collect()
                } else {
                    std::iter::once(lead).chain(block.iter().skip(1).take_while(|l| !l.is_empty()).map(String::as_str)).collect()
                };
                let mut text = paragraph.join(" ");
                if let Some(c) = text.get(..1) {
                    text = c.to_uppercase() + &text[1..];
                }
                out.entry(name.to_string()).or_insert(text);
            }
            block.clear();
        }
    }
    out
}

fn identifier(field: &str) -> String {
    if KEYWORDS.contains(&field) { format!("{field}_") } else { field.to_string() }
}

/// `material` from `material.set_type`.
fn group(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

fn class_name(group: &str) -> String {
    let mut chars = group.chars();
    let first = chars.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
    format!("{first}{}Commands", chars.as_str())
}

fn groups(all: &[Binding]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for b in all {
        let g = group(b.name);
        if !out.contains(&g) {
            out.push(g);
        }
    }
    out
}

fn wrap(text: &str, width: usize, indent: &str) -> String {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(format!("{indent}{line}"));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(format!("{indent}{line}"));
    }
    lines.join("\n")
}

/// `python/tequila_commands.py`.
pub fn python() -> String {
    let all = bindings();
    let mut out = String::new();
    out.push_str(
        "\"\"\"Every command in Tequila's catalog, as a Python call.\n\
         \n\
         Generated from the build's command catalog by\n\
         `cargo run -p tequila-scripting --example bindings`. Do not edit: a test\n\
         fails when this falls behind the build.\n\
         \n\
         Each call applies one command through the bus, as the same click would:\n\
         one undo step, journalled, validated. A field left out is left out of the\n\
         command, and Tequila says so if the command needs it.\n\
         \n\
         \x20   import tequila\n\
         \x20   with tequila.open('kettle.tequila') as fs:\n\
         \x20       fs.commands.material.set_type(material=2, type='velvet')\n\
         \"\"\"\n\n",
    );
    out.push_str("CATALOG = (\n");
    for b in &all {
        out.push_str(&format!("    '{}',\n", b.name));
    }
    out.push_str(")\n\n_UNSET = object()\n\n\n");
    out.push_str(
        "class _Commands:\n\
         \x20   def __init__(self, client):\n\
         \x20       self._client = client\n\
         \n\
         \x20   def _apply(self, command, fields):\n\
         \x20       return self._client.apply(command, **{k: v for k, v in fields.items() if v is not _UNSET})\n\n\n",
    );
    for g in groups(&all) {
        out.push_str(&format!("class {}(_Commands):\n    \"\"\"The `{g}.*` commands.\"\"\"\n", class_name(g)));
        for b in all.iter().filter(|b| group(b.name) == g) {
            let verb = identifier(b.name.split_once('.').map(|(_, v)| v).unwrap_or(b.name));
            let params: Vec<String> = b.fields.iter().map(|f| format!("{}=_UNSET", identifier(f))).collect();
            let signature = if params.is_empty() { "self".to_string() } else { format!("self, *, {}", params.join(", ")) };
            out.push_str(&format!("\n    def {verb}({signature}):\n"));
            out.push_str(&format!("        r\"\"\"`{}`", b.name));
            if !b.summary.is_empty() {
                out.push_str(&format!(": {}", b.summary.trim_end_matches('.')));
                out.push('.');
            }
            out.push_str("\n\n");
            out.push_str(&format!("        {}\n        \"\"\"\n", b.example));
            let fields: Vec<String> = b.fields.iter().map(|f| format!("'{f}': {}", identifier(f))).collect();
            out.push_str(&format!("        return self._apply('{}', {{{}}})\n", b.name, fields.join(", ")));
        }
        out.push_str("\n\n");
    }
    out.push_str("class Commands:\n    \"\"\"The whole catalog, by group: `commands.material.set_type(...)`.\"\"\"\n\n    def __init__(self, client):\n");
    for g in groups(&all) {
        out.push_str(&format!("        self.{g} = {}(client)\n", class_name(g)));
    }
    out
}

/// `docs/scripting-api.md`.
pub fn reference() -> String {
    let all = bindings();
    let mut out = String::new();
    out.push_str(
        "# Scripting API reference\n\n\
         Generated from the command catalog by\n\
         `cargo run -p tequila-scripting --example bindings`; a test fails when it\n\
         falls behind the build. [SCRIPTING.md](SCRIPTING.md) is the guide: the\n\
         console, the protocol, and worked examples. This is the list.\n\n\
         ## Sessions\n\n\
         The `tequila` Python module (the distribution's `python` folder, standard\n\
         library only) drives Tequila through the same command bus the interface\n\
         uses, so anything clickable is scriptable and every call is one undo step.\n\n\
         ```python\n\
         import tequila\n\n\
         # A headless session: `tequila --serve`, no window, its own process.\n\
         with tequila.open('kettle.tequila') as fs:\n\
         \x20   lid = fs.find(name='Lid')[0]\n\
         \x20   red = fs.materials['Paint Base']\n\
         \x20   red.base_color = [0.72, 0.05, 0.06]\n\
         \x20   lid.assign(red)\n\
         \x20   fs.render('red.png', width=1200, height=800, samples=256)\n\
         \x20   fs.save('kettle-red.tequila')\n\n\
         # The application a person has open.\n\
         with tequila.connect() as fs:\n\
         \x20   fs.commands.scene.rename(node=12, name='Lid')\n\
         ```\n\n\
         `tequila.open(path=None)` starts `tequila --serve` with the project (or a\n\
         model to import, or an empty scene) and ends it when the session closes,\n\
         unsaved changes with it: save first. It looks for the command-line\n\
         renderer beside the `python` folder (`tequila-cli`), in a development\n\
         build's `target`, or where `TEQUILA_CLI` says. `tequila.connect()`\n\
         finds the running application through its discovery file.\n\n\
         Both answer the same protocol, one JSON request and reply per line over an\n\
         authenticated loopback socket. A headless session does what needs only\n\
         the document; what needs the window is refused by name.\n\n",
    );
    out.push_str("## Operations\n\n| Operation | Application | Headless |\n|---|---|---|\n");
    let mut every: Vec<&str> = crate::document::APP_OPERATIONS.to_vec();
    for op in crate::document::HEADLESS_OPERATIONS {
        if !every.contains(op) {
            every.push(op);
        }
    }
    for op in every {
        let mark = |list: &[&str]| if list.contains(&op) { "yes" } else { "-" };
        out.push_str(&format!("| `{op}` | {} | {} |\n", mark(crate::document::APP_OPERATIONS), mark(crate::document::HEADLESS_OPERATIONS)));
    }
    out.push_str("\nQueries (`{\"op\":\"query\",\"what\":...}`): ");
    out.push_str(&crate::document::APP_QUERIES.iter().map(|q| format!("`{q}`")).collect::<Vec<_>>().join(", "));
    out.push_str("; a headless session answers all but `ui` and `jobs`.\n\n");
    out.push_str(&format!(
        "## Commands\n\n{} commands, each applied as `{{\"op\":\"apply\",\"command\":{{\"command\":<name>, ...}}}}`, or from Python as\n`fs.commands.<group>.<verb>(field=...)`. Ids and keys in the examples are\nplaceholders: read the real ones from the scene first.\n",
        all.len()
    ));
    for g in groups(&all) {
        out.push_str(&format!("\n### {g}\n"));
        for b in all.iter().filter(|b| group(b.name) == g) {
            let verb = identifier(b.name.split_once('.').map(|(_, v)| v).unwrap_or(b.name));
            out.push_str(&format!("\n#### `{}`\n\n", b.name));
            if !b.summary.is_empty() {
                out.push_str(&wrap(&b.summary, 76, ""));
                out.push_str("\n\n");
            }
            let call: Vec<String> = b.fields.iter().map(|f| format!("{}=...", identifier(f))).collect();
            out.push_str(&format!("`fs.commands.{g}.{verb}({})`\n\n", call.join(", ")));
            out.push_str(&format!("```json\n{}\n```\n", b.example));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_fields_an_example_and_a_summary() {
        let all = bindings();
        assert_eq!(all.len(), Command::CATALOG.len());
        for b in &all {
            assert!(!b.example.is_empty(), "{} has no example", b.name);
            assert!(!b.summary.is_empty(), "{} has no documentation to describe it by", b.name);
        }
    }

    /// No method or keyword argument is a word Python reserves: `scene.import`
    /// is `import_`.
    #[test]
    fn no_generated_name_is_a_python_keyword() {
        let python = python();
        for line in python.lines().map(str::trim).filter(|l| l.starts_with("def ")) {
            let names = line.trim_start_matches("def ").replace(['(', ')', ',', '*', ':', '='], " ");
            for word in names.split_whitespace() {
                assert!(!KEYWORDS.contains(&word), "`{word}` in: {line}");
            }
        }
    }

    /// The generated files in the repository are what this build generates.
    /// When this fails, run `cargo run -p tequila-scripting --example
    /// bindings` and commit what it writes.
    #[test]
    fn the_python_bindings_and_reference_are_current() {
        let normal = |s: &str| s.replace("\r\n", "\n");
        let python_file = normal(include_str!("../../../python/tequila_commands.py"));
        let reference_file = normal(include_str!("../../../docs/scripting-api.md"));
        assert!(python_file == python(), "python/tequila_commands.py is stale: run `cargo run -p tequila-scripting --example bindings`");
        assert!(reference_file == reference(), "docs/scripting-api.md is stale: run `cargo run -p tequila-scripting --example bindings`");
    }
}
