//! The guide (`skills/tequila`: the core every assistant is given, and a
//! page for each area of the application) teaches by example: every JSON
//! example in it is read here by the parsers Tequila itself uses, and every
//! tool call by the tool's own list of what it takes, so a renamed command,
//! field, pattern or argument fails this test instead of an AI following
//! the guide. It is also held to its size here - a page too long to read is
//! not read - and to itself: the pages there are, the pages the core sends
//! an assistant to, and the pages one page sends it on to are the same.

use tequila_format::json::{self, Value};
use tequila_scripting::guide;
use std::path::{Path, PathBuf};

fn skill() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/tequila")
}

fn skill_files() -> Vec<PathBuf> {
    let mut files = vec![skill().join("SKILL.md")];
    let mut references: Vec<PathBuf> = std::fs::read_dir(skill().join("reference"))
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

/// The tools an AI app is offered, by name: what each takes.
fn tools() -> Vec<Value> {
    let text = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tequila-cli/src/mcp_tools.json")).expect("the bridge's tools");
    json::parse(&text).expect("the bridge's tools are JSON").as_array().to_vec()
}

/// A tool call's arguments against the tool's own schema: every argument
/// one it takes, and every one it needs there.
fn check_arguments(tools: &[Value], tool: &str, arguments: &Value) -> Result<(), String> {
    let Some(schema) = tools.iter().find(|t| t.get("name").and_then(Value::as_str) == Some(tool)).and_then(|t| t.get("inputSchema")) else {
        return Err(format!("there is no tool `{tool}`"));
    };
    let Value::Object(given) = arguments else { return Err(format!("`{tool}` takes an object")) };
    let takes = schema.get("properties");
    for key in given.keys() {
        if takes.and_then(|p| p.get(key)).is_none() {
            return Err(format!("`{tool}` takes no `{key}`"));
        }
    }
    for needed in schema.get("required").map(Value::as_array).unwrap_or(&[]) {
        let needed = needed.as_str().unwrap_or_default();
        if !given.contains_key(needed) {
            return Err(format!("`{tool}` needs `{needed}`"));
        }
    }
    Ok(())
}

/// A tool call as the request its tool sends, where the request is worth
/// reading: `None` for a tool that only looks.
fn as_request(tool: &str, arguments: &Value) -> Result<Option<Value>, String> {
    let with = |op: &str, extra: &[(&str, &str)]| {
        let mut request = arguments.clone();
        if let Value::Object(fields) = &mut request {
            fields.insert("op".into(), Value::String(op.into()));
            for (key, value) in extra {
                fields.insert((*key).into(), Value::String((*value).into()));
            }
        }
        Some(request)
    };
    // A tool that names what it works on `id` sends it as `plugin`.
    let plugin = |action: &str| {
        let mut request = with("plugin", &[("action", action)]);
        if let Some(Value::Object(fields)) = &mut request {
            if let Some(id) = fields.remove("id") {
                fields.insert("plugin".into(), id);
            }
        }
        request
    };
    Ok(match tool {
        "tequila_request" => Some(arguments.get("request").cloned().ok_or("`tequila_request` needs `request`")?),
        "tequila_apply" => with("apply", &[]),
        "tequila_batch" => with("batch", &[]),
        "tequila_library" => with("library", &[("action", "search")]),
        "tequila_library_apply" => with("library", &[("action", "apply")]),
        "tequila_look" => with("look", &[]),
        "tequila_frame" => with("frame", &[]),
        "tequila_select" => with("select", &[]),
        "tequila_ui" => with("ui", &[]),
        "tequila_queue" => with("queue", &[]),
        "tequila_add_shape" => with("add_shape", &[]),
        "tequila_import" => with("import", &[]),
        "tequila_save" => with("save", &[]),
        "tequila_preferences" => with("preferences", &[]),
        "tequila_query" => with("query", &[]),
        "tequila_read" => with("read", &[]),
        "tequila_render" => with("render", &[("path", "/tmp/render.png")]),
        "tequila_screenshot" => with("screenshot", &[("path", "/tmp/screenshot.png")]),
        "tequila_material_preview" => with("material", &[("action", "preview")]),
        "tequila_material_create" => with("material", &[("action", "create")]),
        "tequila_material_update" => with("material", &[("action", "update")]),
        "tequila_material_recipe" => with("material", &[("action", "recipe")]),
        "tequila_material_save" => with("material", &[("action", "save")]),
        "tequila_studio_presets" => with("studio_preset", &[("action", "list")]),
        "tequila_studio_preset_apply" => with("studio_preset", &[("action", "apply")]),
        "tequila_studio_preset_save" => with("studio_preset", &[("action", "save")]),
        "tequila_studio_render_all" => with("studio_preset", &[("action", "render_all")]),
        "tequila_plugin_write" => plugin("write"),
        "tequila_plugin_enable" => plugin("enable"),
        "tequila_plugin_run" => plugin("run"),
        "tequila_plugin_log" => plugin("log"),
        // What a page is asked for by has to be a page, and a command a
        // command.
        "tequila_guide" => {
            if let Some(topic) = arguments.get("topic").and_then(Value::as_str) {
                guide::page(Some(topic))?;
            }
            None
        }
        "tequila_command" => {
            tequila_scripting::document::help(arguments.get("command").and_then(Value::as_str))?;
            None
        }
        _ => None,
    })
}

fn check(file: &Path, tools: &[Value], tool: Option<&str>, value: &Value) -> Result<&'static str, String> {
    let sent;
    let value = match tool {
        Some(tool) => {
            check_arguments(tools, tool, value)?;
            match as_request(tool, value)? {
                Some(request) => {
                    sent = request;
                    &sent
                }
                None => return Ok("tool"),
            }
        }
        None => value,
    };
    let text = tequila_format::json_write::to_string(value);
    if value.get("command").is_some() {
        tequila_format::journal::decode(&text).map_err(|e| format!("{e:?}"))?;
        Ok("command")
    } else if value.get("op").and_then(Value::as_str) == Some(tequila_scripting::uv_layout::OPERATION) {
        // The application answers this one whole, outside the protocol's
        // own operations: what it reads is read here as it reads it.
        let Value::Object(fields) = value else { unreachable!("it has an op") };
        for key in fields.keys() {
            if !["op", "node", "path", "set", "size", "fill", "edges", "seams", "note", "line", "transparent", "format"].contains(&key.as_str()) {
                return Err(format!("`uv_layout` reads no `{key}`"));
            }
        }
        if value.get("node").and_then(Value::as_index).is_none() || value.get("path").and_then(Value::as_str).is_none() {
            return Err("`uv_layout` needs a `node` and a `path`".into());
        }
        if let Some(fill) = value.get("fill").and_then(Value::as_str) {
            if !["islands", "none", "area", "angle"].contains(&fill) {
                return Err(format!("`{fill}` is not a fill"));
            }
        }
        if let Some(line) = value.get("line").and_then(Value::as_str) {
            tequila_scripting::uv_layout::colour(line).ok_or_else(|| format!("`{line}` is not a colour"))?;
        }
        Ok("request")
    } else if value.get("op").is_some() {
        let request = tequila_scripting::protocol::parse_request(&text).map_err(|e| e.to_string())?;
        // What is read is a part of the project that there is.
        if let tequila_scripting::protocol::Operation::Read { what: Some(what), .. } = &request.operation {
            if !tequila_scripting::document::SECTIONS.iter().any(|(name, _)| name == what) {
                return Err(format!("`{what}` is no part of a project"));
            }
        }
        // A recipe sent to be made is read as one.
        if let tequila_scripting::protocol::Operation::Material(material) = &request.operation {
            if let Some(recipe) = &material.recipe {
                tequila_scripting::material::entry_of(recipe)?;
            }
        }
        Ok("request")
    } else if value.get("nodes").is_some() && value.get("links").is_some() {
        // A material's graph, as a material is read: sent back inside a
        // definition, it is taken.
        let (_, example) = tequila_format::examples::by_name().into_iter().find(|(name, _)| *name == "material.set_graph").expect("the catalog has it");
        let mut command = example;
        if let Some(Value::Object(definition)) = command.get("definition").cloned() {
            let mut definition = definition;
            definition.insert("graph".into(), value.clone());
            if let Value::Object(fields) = &mut command {
                fields.insert("definition".into(), Value::Object(definition));
            }
        }
        tequila_format::journal::decode(&tequila_format::json_write::to_string(&command)).map_err(|e| format!("{e:?}"))?;
        Ok("graph")
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
    let tools = tools();
    let mut counts = std::collections::BTreeMap::new();
    let mut failures = Vec::new();
    let mut bare = Vec::new();
    for file in skill_files() {
        let text = std::fs::read_to_string(&file).unwrap();
        let mut here = 0;
        for block in json_blocks(&text) {
            for (tool, value) in examples(&block) {
                here += 1;
                match check(&file, &tools, tool.as_deref(), &value) {
                    Ok(kind) => *counts.entry(kind).or_insert(0) += 1,
                    Err(why) => failures.push(format!("{}: {why}\n  {}", file.display(), tequila_format::json_write::to_string(&value))),
                }
            }
        }
        if here == 0 {
            bare.push(file.display().to_string());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Every page teaches by example, and between them they are of every
    // kind.
    assert!(bare.is_empty(), "no tested example in {bare:?}");
    assert!(counts.get("command").copied().unwrap_or(0) >= 80, "{counts:?}");
    assert!(counts.get("request").copied().unwrap_or(0) >= 60, "{counts:?}");
    assert!(counts.contains_key("tool"), "{counts:?}");
    assert!(counts.get("recipe").copied().unwrap_or(0) >= 3, "{counts:?}");
    assert!(counts.contains_key("graph"), "{counts:?}");
    assert!(counts.contains_key("manifest"), "{counts:?}");
}

/// The core is short enough to be given whole to every assistant with
/// every conversation, and a page short enough to be read whole: an answer
/// of tens of thousands of characters is skipped, and the assistant guesses.
///
/// The lengths are the files' as the repository keeps them. `.gitattributes`
/// keeps them so in every checkout; one made before that, on Windows, has a
/// carriage return on every line, which is not counted.
#[test]
fn the_core_and_every_page_stay_short_enough_to_read() {
    let count = |text: &str| text.chars().filter(|c| *c != '\r').count();
    let core = count(guide::core());
    assert!(core <= 7_000, "the core is {core} characters; it is to stay well under {}", guide::CORE_LIMIT);
    assert!(guide::CORE_LIMIT <= 8_000 && guide::PAGE_LIMIT <= 12_000);
    for page in guide::PAGES {
        let length = count(page.text);
        assert!(length <= guide::PAGE_LIMIT, "`{}` is {length} characters, over {}", page.topic, guide::PAGE_LIMIT);
        assert!(length >= 1_500, "`{}` is {length} characters: too little to work from", page.topic);
        assert!(page.text.starts_with("# "), "`{}` starts with its title", page.topic);
        assert!(!page.about.is_empty() && page.about.len() <= 140, "`{}` is listed in a line", page.topic);
    }
    // The list of pages is itself a short answer.
    assert!(tequila_format::json_write::to_string(&guide::list()).len() < 4_000);
}

/// The pages the guide serves are the files of the skill, no more and no
/// fewer: a page written and not listed would never be read, and one
/// listed and not written would not build.
#[test]
fn the_pages_served_are_the_skills_files() {
    let mut files: Vec<String> = skill_files().iter().skip(1).map(|file| file.file_stem().unwrap().to_string_lossy().into_owned()).collect();
    let mut pages: Vec<String> = guide::PAGES.iter().map(|page| page.topic.to_string()).collect();
    files.sort();
    pages.sort();
    assert_eq!(pages, files);
    for page in guide::PAGES {
        assert_eq!(guide::find(page.topic), Some(page.topic));
        assert_eq!(std::fs::read_to_string(skill().join("reference").join(format!("{}.md", page.topic))).unwrap(), page.text, "`{}` is built from its file", page.topic);
    }
    assert!(std::fs::read_to_string(skill().join("SKILL.md")).unwrap().ends_with(guide::core()));
}

/// The pages a text sends its reader to: the words in backticks after
/// "page" or "pages", and the last cell of a table's rows where the table
/// is of pages.
fn pages_named(text: &str) -> Vec<String> {
    let mut named = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("page") {
        rest = &rest[at + 4..];
        let mut after = rest.strip_prefix('s').unwrap_or(rest);
        // `page `a``, `pages `a`, `b` and `c``.
        loop {
            let trimmed = after.trim_start_matches([' ', ',']).trim_start_matches("and ").trim_start_matches("or ");
            let Some(inner) = trimmed.strip_prefix('`') else { break };
            let Some((name, tail)) = inner.split_once('`') else { break };
            if name.chars().all(|c| c.is_ascii_lowercase() || c == '_') && !name.is_empty() {
                named.push(name.to_string());
            }
            after = tail;
        }
    }
    named
}

/// Every page the core names is one that exists, every page that exists is
/// named in the core - an assistant can only open what it has been told is
/// there - and every page a page sends its reader on to exists too.
#[test]
fn every_page_the_guide_names_exists() {
    let core = guide::core();
    // The core's table: a job, and the page for it.
    let mut in_table: Vec<String> = Vec::new();
    for line in core.lines().filter(|line| line.starts_with('|')) {
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if let Some(name) = cells.last().and_then(|cell| cell.strip_prefix('`')).and_then(|cell| cell.strip_suffix('`')) {
            in_table.push(name.to_string());
        }
    }
    for page in guide::PAGES {
        assert!(in_table.iter().any(|name| name == page.topic), "the core's table does not send anyone to `{}`", page.topic);
    }
    for name in &in_table {
        assert!(guide::PAGES.iter().any(|page| page.topic == name), "the core's table names `{name}`, which is no page");
    }
    for (from, text) in std::iter::once(("the core", core)).chain(guide::PAGES.iter().map(|page| (page.topic, page.text))) {
        for name in pages_named(text) {
            assert!(guide::PAGES.iter().any(|page| page.topic == name), "{from} sends its reader to page `{name}`, which there is not");
        }
    }
    // And the core says how a page is opened.
    assert!(core.contains("tequila_guide {\"topic\""));
}

/// Every tool the guide tells an assistant to use is one the bridge offers.
#[test]
fn every_tool_the_guide_names_is_offered() {
    let tools = tools();
    let offered: Vec<&str> = tools.iter().filter_map(|tool| tool.get("name").and_then(Value::as_str)).collect();
    for file in skill_files() {
        let text = std::fs::read_to_string(&file).unwrap();
        let mut named: Vec<&str> = text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|word| word.starts_with("tequila_") && word.len() > 8).collect();
        named.sort_unstable();
        named.dedup();
        for tool in named {
            // `tequila_*` and `tequila_plugin_*` name families; the Python
            // module's own names are not tools.
            if tool.ends_with('_') || matches!(tool, "tequila_plugin" | "tequila_mcp") {
                continue;
            }
            assert!(offered.contains(&tool), "{} names `{tool}`, which the bridge does not offer", file.display());
        }
    }
}
