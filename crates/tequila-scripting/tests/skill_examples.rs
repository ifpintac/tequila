//! The Claude skill (`skills/tequila`) teaches by example: every JSON
//! example in it is read here by the parsers Tequila itself uses, so a
//! renamed command, field or pattern fails this test instead of an AI
//! following the skill.

use tequila_format::json::{self, Value};
use std::path::{Path, PathBuf};

fn skill_files() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/tequila");
    let mut files = vec![root.join("SKILL.md")];
    let mut references: Vec<PathBuf> = std::fs::read_dir(root.join("reference"))
        .expect("the skill's references")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    references.sort();
    files.extend(references);
    files
}

/// The ```json blocks of a Markdown file, as text.
fn json_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        match (&mut current, line.trim_start()) {
            (None, l) if l.starts_with("```json") => current = Some(String::new()),
            (Some(block), l) if l.starts_with("```") => {
                blocks.push(std::mem::take(block));
                current = None;
            }
            (Some(block), _) => {
                block.push_str(line);
                block.push('\n');
            }
            _ => {}
        }
    }
    blocks
}

/// The examples in a block: one JSON value after another, each over as
/// many lines as it takes, each perhaps after the tool it is for
/// (`tequila_request {...}`).
fn examples(block: &str) -> Vec<(Option<String>, Value)> {
    let mut found = Vec::new();
    let mut chunk = String::new();
    let (mut depth, mut in_string, mut escaped) = (0i32, false, false);
    for c in block.chars() {
        chunk.push(c);
        match c {
            _ if escaped => escaped = false,
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '{' | '[' if !in_string => depth += 1,
            '}' | ']' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    let text = std::mem::take(&mut chunk);
                    let text = text.trim();
                    let (tool, rest) = match text.split_once(' ') {
                        Some((tool, rest)) if tool.starts_with("tequila_") => (Some(tool.to_string()), rest),
                        _ => (None, text),
                    };
                    match json::parse(rest.as_bytes()) {
                        Ok(value) => found.push((tool, value)),
                        // `…` marks an example cut short on purpose.
                        Err(_) if rest.contains('…') => {}
                        Err(e) => panic!("not JSON: {text}: {e:?}"),
                    }
                }
            }
            _ => {}
        }
    }
    assert!(chunk.trim().is_empty() && depth == 0, "left over: {chunk}");
    found
}

fn check(file: &Path, tool: Option<&str>, value: &Value) -> Result<&'static str, String> {
    let value = match (tool, value.get("request")) {
        (Some("tequila_request"), Some(request)) => request,
        _ => value,
    };
    let text = tequila_format::json_write::to_string(value);
    if value.get("command").is_some() {
        tequila_format::journal::decode(&text).map_err(|e| format!("{e:?}"))?;
        Ok("command")
    } else if value.get("op").is_some() {
        tequila_scripting::protocol::parse_request(&text).map_err(|e| e.to_string())?;
        Ok("request")
    } else if value.get("patterns").is_some() || value.get("type").is_some() {
        tequila_scripting::material::entry_of(value)?;
        Ok("recipe")
    } else if value.get("id").is_some() && value.get("tequila").is_some() {
        // A plugin manifest: read by the application, which this crate is
        // not; its own tests read the bundled ones.
        Ok("manifest")
    } else {
        Err(format!("{}: an example of nothing this test knows: {text}", file.display()))
    }
}

#[test]
fn every_example_in_the_skill_is_read_by_tequila() {
    let mut counts = std::collections::BTreeMap::new();
    let mut failures = Vec::new();
    for file in skill_files() {
        let text = std::fs::read_to_string(&file).unwrap();
        for block in json_blocks(&text) {
            for (tool, value) in examples(&block) {
                match check(&file, tool.as_deref(), &value) {
                    Ok(kind) => *counts.entry(kind).or_insert(0) += 1,
                    Err(why) => failures.push(format!("{}: {why}\n  {}", file.display(), tequila_format::json_write::to_string(&value))),
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // It read what it should have: the skill is example-led.
    assert!(counts.get("command").copied().unwrap_or(0) >= 25, "{counts:?}");
    assert!(counts.get("request").copied().unwrap_or(0) >= 1, "{counts:?}");
    assert!(counts.get("recipe").copied().unwrap_or(0) >= 3, "{counts:?}");
    assert!(counts.contains_key("manifest"), "{counts:?}");
}
