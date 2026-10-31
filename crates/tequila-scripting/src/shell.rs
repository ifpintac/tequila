//! The console's language: what a person types, turned into what a client
//! sends.
//!
//! The wire protocol is JSON because programs write it. People should not
//! have to: `{"op":"select","nodes":[360]}` is a sentence nobody types twice.
//! So the console accepts short commands — `select 360`, `nodes strap`,
//! `frame sel` — and rewrites each one into exactly the request a client would
//! have sent. It is a front end, not a second API: every line still becomes a
//! [`crate::protocol::Request`] and goes through the same parser and the same
//! handler as the socket, so nothing works here that a script cannot also do.
//!
//! Three escape hatches keep it from being a cage:
//!
//! - a line starting with `{` is a raw request, passed through untouched;
//! - `apply {...}` sends any journal line as a command;
//! - any catalog name works as a verb, with `key=value` arguments:
//!   `material.set_type material=2 type=velvet`.

use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};
use std::collections::BTreeMap;

/// How the answer to a request should be laid out for a person.
///
/// The request is the same either way; this is only presentation. A node list
/// shown as raw JSON is several thousand lines for a split model, which is
/// data rather than an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum View {
    /// Indented JSON.
    Json,
    /// A table of nodes, optionally narrowed by a word, optionally parts only.
    Nodes { filter: Option<String>, parts_only: bool },
    Materials,
    Commands,
    Jobs,
    History,
}

/// Things the console does itself, without asking the application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Local {
    Help(Option<String>),
    Clear,
    History,
    Copy,
    Examples,
    /// Put example `n` in the input line, for reading before running.
    Example(usize),
    /// Run every line of a file.
    Run(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Empty,
    Request { json: String, view: View },
    Local(Local),
}

/// One usage line and what it does, for `help`.
pub const HELP: &[(&str, &str)] = &[
    ("help [verb]", "this list, or the detail for one verb"),
    ("ping", "is the application answering"),
    ("context", "everything a script needs to start: scene, parts, materials, camera"),
    ("api", "the protocol, material parameters and types, as JSON"),
    ("scene", "counts, units and overall bounds"),
    ("nodes [word]", "every node as a table; a word narrows it by name, id or material"),
    ("parts [word]", "the same, parts only"),
    ("materials", "every material, its type, how many parts use it, and its maps"),
    ("selection", "what is selected"),
    ("camera", "where the camera is and what it looks at"),
    ("commands", "every command this build accepts"),
    ("select <id> [id ...]", "replace the selection"),
    ("frame [all|sel]", "point the camera at everything, or at the selection"),
    ("undo / redo", "one step back or forward, exactly as Ctrl+Z"),
    ("save [path]", "write the project; without a path, where it came from"),
    ("import <path>", "load a model into the scene"),
    ("texture_set <path>", "make a material from a PBR texture pack (folder or ZIP) and put it on the selection"),
    ("reimport [path]", "update the selected part's model from its file, or from path, keeping materials and labels"),
    ("screenshot <path> [viewport|render|graph|shortcuts|connect]", "capture the window, just the viewport, or another window, to a PNG (arrives a frame later)"),
    ("render <path> [w h spp]", "render to a PNG; also accepts 1200x800"),
    ("<command> key=value ...", "any catalog command, e.g. material.set_type material=2 type=velvet"),
    ("apply {json}", "any command as a journal line"),
    ("{json}", "a raw request, exactly as a client would send it"),
    ("a; b; c", "several commands as one undo step; if one fails, all are taken back"),
    ("look <view>", "front, back, left, right, top, bottom or iso"),
    ("look key=value ...", "yaw= pitch= (degrees) distance= at=[x,y,z] from=[x,y,z] focal= (mm)"),
    ("show <panel|tab>", "render, console, inspector, library, or an Inspector tab like material"),
    ("hide <panel>", "render, console, inspector or library"),
    ("viewport rendered|solid", "switch the viewport's shading"),
    ("ui", "which panels are open"),
    ("queue [WxH] [spp] [key=value]", "render in the background; name= folder= filename= denoise= camera="),
    ("jobs", "the background queue: what is waiting, rendering and done"),
    ("library", "every material preset, shipped and your own"),
    ("library apply <name>", "put a library preset on the selection, as one undo step"),
    ("library import <path>", "a material package (.fsmat), preset (.json), KeyShot package (.kmp) or texture pack into your library"),
    ("stop", "stop the background render; it still saves what it has"),
    ("changes", "what undo and redo would do, and whether there is unsaved work"),
    ("quit [save|discard]", "close Tequila; refuses unsaved work unless told which"),
    ("run <file>", "run a script as one undo step, taken back if a line fails; # for comments"),
    ("examples / example <n>", "list the worked examples, or load one into the prompt"),
    ("history", "what you have typed this session"),
    ("copy", "copy the last answer to the clipboard"),
    ("clear", "clear the console"),
];

/// Every word that can start a line, for completion and for `help <verb>`.
pub const VERBS: &[&str] = &[
    "api", "apply", "camera", "changes", "clear", "commands", "context", "copy", "example",
    "examples", "frame", "help", "hide", "history", "import", "jobs", "library", "look", "materials",
    "nodes", "parts", "ping", "queue", "quit", "redo", "render", "run", "save", "scene",
    "reimport", "screenshot", "select", "selection", "show", "stop", "texture_set", "ui", "undo", "viewport",
];

/// A word from the line, and whether any of it was quoted.
///
/// Quoting matters beyond spaces: `name="123"` is the string `123` and
/// `name=123` is the number, and only the quotes say which was meant.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Token {
    text: String,
    quoted: bool,
}

/// Split a line into words.
///
/// Whitespace separates words, except inside double quotes and inside JSON
/// brackets, so `value=[0.8, 0.8, 0.8]` and `import "C:/My Models/a.gltf"` each
/// stay one word. There are no backslash escapes, on purpose: this runs on
/// Windows, where a backslash is a path separator far more often than it is
/// anything else, and a path pasted from Explorer has to survive intact.
fn tokens(line: &str) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut quoted = false;
    let mut in_quotes = false;
    let mut depth = 0i32;
    let mut started = false;

    for c in line.chars() {
        if depth > 0 {
            // Inside brackets the text is JSON and is kept verbatim, quotes
            // and all, for the JSON parser to read later. Quotes still have to
            // be tracked, or a `]` inside a string would close the bracket.
            text.push(c);
            match c {
                '"' => in_quotes = !in_quotes,
                '[' | '{' if !in_quotes => depth += 1,
                ']' | '}' if !in_quotes => depth -= 1,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => {
                in_quotes = !in_quotes;
                quoted = true;
                started = true;
            }
            '[' | '{' if !in_quotes => {
                depth = 1;
                text.push(c);
                started = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if started {
                    out.push(Token { text: std::mem::take(&mut text), quoted });
                    quoted = false;
                    started = false;
                }
            }
            c => {
                text.push(c);
                started = true;
            }
        }
    }
    if in_quotes {
        return Err("a quote was opened and never closed".into());
    }
    if depth != 0 {
        return Err("a bracket was opened and never closed".into());
    }
    if started {
        out.push(Token { text, quoted });
    }
    Ok(out)
}

/// A value from `key=value`, typed by what it looks like.
fn value_of(token: &Token, raw: &str) -> Result<Value, String> {
    if token.quoted {
        return Ok(Value::String(raw.to_string()));
    }
    if raw.starts_with('[') || raw.starts_with('{') {
        return json::parse(raw.as_bytes()).map_err(|e| format!("`{raw}` is not JSON: {e}"));
    }
    match raw {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        "null" => return Ok(Value::Null),
        _ => {}
    }
    // Numbers only when the whole word is one. `inf` and `nan` parse as
    // floats and are also perfectly good names, so they stay text.
    if let Ok(number) = raw.parse::<f64>() {
        if number.is_finite() && raw.chars().any(|c| c.is_ascii_digit()) {
            return Ok(Value::Number(number));
        }
    }
    Ok(Value::String(raw.to_string()))
}

fn request(object: Object) -> String {
    json_write::to_string(&object.build())
}

fn query(what: &str, view: View) -> Parsed {
    let mut out = Object::new();
    out.set("op", "query").set("what", what);
    Parsed::Request { json: request(out), view }
}

fn id_list(args: &[Token], verb: &str) -> Result<Vec<Value>, String> {
    if args.is_empty() {
        return Err(format!("{verb} needs at least one id — `nodes` lists them"));
    }
    args.iter()
        .map(|t| {
            t.text
                .parse::<u64>()
                .map(|n| Value::Number(n as f64))
                .map_err(|_| format!("`{}` is not a node id", t.text))
        })
        .collect()
}

/// `key=value` words, typed as a catalog command's are.
fn key_values(args: &[Token]) -> Result<Vec<(String, Value)>, String> {
    args.iter()
        .map(|token| {
            let (key, raw) = token
                .text
                .split_once('=')
                .ok_or_else(|| format!("`{}` should be key=value", token.text))?;
            if key.is_empty() {
                return Err(format!("`{}` has no key before the =", token.text));
            }
            Ok((key.to_string(), value_of(token, raw)?))
        })
        .collect()
}

/// Split at `separator` where it is not inside quotes or brackets.
fn split_top_level(line: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quoted, mut start) = (0i32, false, 0);
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '{' if !quoted => depth += 1,
            ']' | '}' if !quoted => depth -= 1,
            c if c == separator && !quoted && depth == 0 => {
                parts.push(&line[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&line[start..]);
    parts
}

/// The panels `show` and `hide` know, as the `ui` field each sets.
fn panel_field(word: &str) -> Option<&'static str> {
    Some(match word {
        "render" | "render_window" | "renderer" => "render_window",
        "console" => "console",
        "inspector" => "inspector",
        "library" | "materials" => "library",
        _ => return None,
    })
}

fn one_path(args: &[Token], verb: &str) -> Result<String, String> {
    match args {
        [path] => Ok(path.text.clone()),
        [] => Err(format!("{verb} needs a path")),
        _ => Err(format!(
            "{verb} takes one path; quote it if it has spaces in it"
        )),
    }
}

/// Read one line.
pub fn parse(line: &str) -> Result<Parsed, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(Parsed::Empty);
    }
    if line.starts_with('{') {
        json::parse(line.as_bytes()).map_err(|e| format!("that is not valid JSON: {e}"))?;
        return Ok(Parsed::Request {
            json: line.to_string(),
            view: View::Json,
        });
    }

    let parts: Vec<&str> = split_top_level(line, ';')
        .into_iter()
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() > 1 {
        let mut operations = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            match parse(part).map_err(|e| format!("part {} (`{part}`): {e}", i + 1))? {
                Parsed::Request { json, .. } => operations.push(
                    json::parse(json.as_bytes()).map_err(|e| format!("part {}: {e}", i + 1))?,
                ),
                Parsed::Empty => {}
                Parsed::Local(_) => {
                    return Err(format!(
                        "part {} (`{part}`) is a console command, which cannot be part of a batch",
                        i + 1
                    ))
                }
            }
        }
        let mut out = Object::new();
        out.set("op", "batch").set("ops", Value::Array(operations));
        return Ok(Parsed::Request { json: request(out), view: View::Json });
    }

    let words = tokens(line)?;
    let (verb, args) = words.split_first().expect("a non-empty line has a word");
    let verb_text = verb.text.to_ascii_lowercase();

    let parsed = match verb_text.as_str() {
        "help" | "?" => Parsed::Local(Local::Help(args.first().map(|t| t.text.clone()))),
        "clear" | "cls" => Parsed::Local(Local::Clear),
        "history" => Parsed::Local(Local::History),
        "copy" => Parsed::Local(Local::Copy),
        "examples" => Parsed::Local(Local::Examples),
        "example" => {
            let n = args
                .first()
                .and_then(|t| t.text.parse::<usize>().ok())
                .ok_or("example needs a number — `examples` lists them")?;
            Parsed::Local(Local::Example(n))
        }
        "run" => Parsed::Local(Local::Run(one_path(args, "run")?)),

        "ping" | "undo" | "redo" => {
            let mut out = Object::new();
            out.set("op", verb_text.as_str());
            Parsed::Request { json: request(out), view: View::Json }
        }
        "api" | "context" | "scene" | "selection" | "camera" => query(&verb_text, View::Json),
        "commands" => query("commands", View::Commands),
        "materials" => query("materials", View::Materials),
        "nodes" | "parts" => query(
            "nodes",
            View::Nodes {
                filter: (!args.is_empty())
                    .then(|| args.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ")),
                parts_only: verb_text == "parts",
            },
        ),

        "select" => {
            let mut out = Object::new();
            out.set("op", "select").set("nodes", Value::Array(id_list(args, "select")?));
            Parsed::Request { json: request(out), view: View::Json }
        }
        "frame" => {
            let all = match args.first().map(|t| t.text.to_ascii_lowercase()) {
                None => true,
                Some(word) if word == "all" || word == "everything" => true,
                Some(word) if word.starts_with("sel") => false,
                Some(word) => return Err(format!("frame takes `all` or `sel`, not `{word}`")),
            };
            let mut out = Object::new();
            out.set("op", "frame").set("all", all);
            Parsed::Request { json: request(out), view: View::Json }
        }
        "save" => {
            let mut out = Object::new();
            out.set("op", "save");
            if let Some(path) = args.first() {
                out.set("path", path.text.clone());
            }
            Parsed::Request { json: request(out), view: View::Json }
        }
        "import" => {
            let mut out = Object::new();
            out.set("op", "import").set("path", one_path(args, "import")?);
            Parsed::Request { json: request(out), view: View::Json }
        }
        "reimport" => {
            let mut out = Object::new();
            out.set("op", "reimport");
            if !args.is_empty() {
                out.set("path", one_path(args, "reimport")?);
            }
            Parsed::Request { json: request(out), view: View::Json }
        }
        "texture_set" => {
            let mut out = Object::new();
            out.set("op", "texture_set").set("path", one_path(args, "texture_set")?);
            Parsed::Request { json: request(out), view: View::Json }
        }
        "screenshot" => {
            const WINDOWS: [&str; 4] = ["render", "graph", "shortcuts", "connect"];
            let (path, viewport, window) = match args {
                [path] => (path, false, None),
                [path, flag] if flag.text.eq_ignore_ascii_case("viewport") => (path, true, None),
                [path, name] if WINDOWS.iter().any(|w| w.eq_ignore_ascii_case(&name.text)) => {
                    (path, false, Some(name.text.to_ascii_lowercase()))
                }
                [] => return Err("screenshot needs a path".into()),
                _ => {
                    return Err(
                        "screenshot takes a path, and optionally viewport or a window (render, graph, shortcuts, connect); quote a path with spaces"
                            .into(),
                    )
                }
            };
            let mut out = Object::new();
            out.set("op", "screenshot").set("path", path.text.clone()).set("viewport", viewport);
            if let Some(window) = window {
                out.set("window", window);
            }
            Parsed::Request { json: request(out), view: View::Json }
        }
        "quit" | "exit" => {
            let mut out = Object::new();
            out.set("op", "quit");
            match args.first().map(|t| t.text.to_ascii_lowercase()).as_deref() {
                None => {}
                Some("save") => {
                    out.set("save", true);
                }
                Some("discard") => {
                    out.set("discard", true);
                }
                Some(other) => return Err(format!("quit takes save or discard, not `{other}`")),
            }
            Parsed::Request { json: request(out), view: View::Json }
        }
        "show" | "hide" => {
            let showing = verb_text == "show";
            let word = args
                .first()
                .map(|t| t.text.to_ascii_lowercase())
                .ok_or_else(|| format!("{verb_text} what? render, console, inspector or library"))?;
            let mut out = Object::new();
            out.set("op", "ui");
            match panel_field(&word) {
                Some(field) => {
                    out.set(field, showing);
                }
                // Anything else is an Inspector tab, which only makes sense
                // to show; the application says so if the name is wrong.
                None if showing => {
                    out.set("tab", word);
                }
                None => return Err(format!("hide takes render, console, inspector or library, not `{word}`")),
            }
            Parsed::Request { json: request(out), view: View::Json }
        }
        "viewport" => {
            let mode = args
                .first()
                .map(|t| t.text.to_ascii_lowercase())
                .ok_or("viewport rendered, or viewport solid")?;
            let mut out = Object::new();
            out.set("op", "ui").set("viewport", mode);
            Parsed::Request { json: request(out), view: View::Json }
        }
        "ui" => query("ui", View::Json),
        "jobs" => query("jobs", View::Jobs),
        "library" => match args.first().map(|t| t.text.to_ascii_lowercase()) {
            None => query("library", View::Json),
            Some(action) if action == "apply" => {
                let name = args[1..].iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ");
                if name.is_empty() {
                    return Err("library apply needs a preset's name".into());
                }
                let mut out = Object::new();
                out.set("op", "library").set("action", "apply").set("name", name);
                Parsed::Request { json: request(out), view: View::Json }
            }
            Some(action) if action == "import" => {
                let mut out = Object::new();
                out.set("op", "library").set("action", "import").set("path", one_path(&args[1..], "library import")?);
                Parsed::Request { json: request(out), view: View::Json }
            }
            Some(other) => return Err(format!("library {other}: try `library`, `library apply <name>` or `library import <path>`")),
        },
        "changes" => query("history", View::History),
        "stop" => {
            let mut out = Object::new();
            out.set("op", "stop");
            Parsed::Request { json: request(out), view: View::Json }
        }
        "look" => {
            let mut out = Object::new();
            out.set("op", "look");
            let mut rest = args;
            if let Some((first, tail)) = args.split_first() {
                if !first.text.contains('=') {
                    out.set("view", first.text.to_ascii_lowercase());
                    rest = tail;
                }
            }
            for (key, value) in key_values(rest)? {
                let key = match key.as_str() {
                    "focal" | "lens" => "focal_length".to_string(),
                    "target" => "at".to_string(),
                    "eye" => "from".to_string(),
                    _ => key,
                };
                out.set(&key, value);
            }
            Parsed::Request { json: request(out), view: View::Json }
        }
        "queue" => {
            let mut out = Object::new();
            out.set("op", "queue");
            for token in args {
                let word = &token.text;
                if let Some((w, h)) = word.split_once(['x', 'X']).filter(|_| !word.contains('=')) {
                    match (w.parse::<u32>(), h.parse::<u32>()) {
                        (Ok(w), Ok(h)) => {
                            out.set("width", w as f64).set("height", h as f64);
                            continue;
                        }
                        _ => return Err(format!("`{word}` is not a size like 1920x1080")),
                    }
                }
                if let Ok(samples) = word.parse::<u32>() {
                    out.set("samples", samples as f64);
                    continue;
                }
                for (key, value) in key_values(std::slice::from_ref(token))? {
                    let value = match (key.as_str(), &value) {
                        // on/off reads better than true/false at a prompt.
                        ("denoise", Value::String(v)) if v == "on" => Value::Bool(true),
                        ("denoise", Value::String(v)) if v == "off" => Value::Bool(false),
                        _ => value,
                    };
                    out.set(&key, value);
                }
            }
            Parsed::Request { json: request(out), view: View::Json }
        }
        "render" => {
            let (path, rest) = args
                .split_first()
                .ok_or("render needs a path, and optionally a size and a sample count")?;
            let mut numbers = Vec::new();
            let mut wireframe = false;
            for token in rest {
                if token.text.eq_ignore_ascii_case("wireframe") {
                    wireframe = true;
                    continue;
                }
                // `1200x800` as well as `1200 800`: it is how sizes are
                // written everywhere else, and refusing it would be pedantry.
                for part in token.text.split(['x', 'X']) {
                    numbers.push(
                        part.parse::<u64>()
                            .map_err(|_| format!("`{}` is not a size or a sample count", token.text))?,
                    );
                }
            }
            let mut out = Object::new();
            out.set("op", "render").set("path", path.text.clone());
            let names = ["width", "height", "samples"];
            if numbers.len() > names.len() {
                return Err("render takes a width, a height and a sample count, and no more".into());
            }
            for (name, number) in names.iter().zip(&numbers) {
                out.set(name, *number as f64);
            }
            if wireframe {
                out.set("wireframe", true);
            }
            Parsed::Request { json: request(out), view: View::Json }
        }
        "apply" => {
            let body = line["apply".len()..].trim();
            if !body.starts_with('{') {
                return Err("apply takes a JSON object: apply {\"command\":\"...\", ...}".into());
            }
            let command =
                json::parse(body.as_bytes()).map_err(|e| format!("that command is not JSON: {e}"))?;
            let mut out = Object::new();
            out.set("op", "apply").set("command", command);
            Parsed::Request { json: request(out), view: View::Json }
        }

        // A catalog command by name. Not checked against the catalog here:
        // the journal's decoder is the authority on what is a command, and a
        // second list in the console would be one more thing to drift.
        name if name.contains('.') => {
            let mut fields: BTreeMap<String, Value> = BTreeMap::new();
            fields.insert("command".into(), Value::String(verb.text.clone()));
            for token in args {
                let (key, raw) = token.text.split_once('=').ok_or_else(|| {
                    format!("`{}` should be key=value", token.text)
                })?;
                if key.is_empty() {
                    return Err(format!("`{}` has no key before the =", token.text));
                }
                fields.insert(key.to_string(), value_of(token, raw)?);
            }
            let mut out = Object::new();
            out.set("op", "apply").set("command", Value::Object(fields));
            Parsed::Request { json: request(out), view: View::Json }
        }

        other => {
            let near: Vec<&str> = VERBS
                .iter()
                .copied()
                .filter(|v| v.starts_with(&other[..other.len().min(2)]))
                .collect();
            return Err(if near.is_empty() {
                format!("`{other}` is not a command — type help")
            } else {
                format!("`{other}` is not a command — did you mean {}?", near.join(", "))
            });
        }
    };
    Ok(parsed)
}

/// What Tab does to a line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// The line with its first word extended, when there was anything to add.
    pub line: Option<String>,
    /// Every word that still fits, when more than one does.
    pub candidates: Vec<String>,
}

/// Complete the first word against the verbs and the command catalog.
///
/// Only the first word, deliberately: that is where the vocabulary is, and
/// completing arguments would mean guessing at ids the console has not been
/// told about.
pub fn complete(line: &str, catalog: &[&str]) -> Completion {
    let none = Completion { line: None, candidates: Vec::new() };
    if line.contains(char::is_whitespace) || line.starts_with('{') {
        return none;
    }
    let mut matches: Vec<&str> = VERBS
        .iter()
        .chain(catalog.iter())
        .copied()
        .filter(|word| word.starts_with(line))
        .collect();
    matches.sort_unstable();
    matches.dedup();

    match matches.as_slice() {
        [] => none,
        [only] => Completion {
            line: Some(format!("{only} ")),
            candidates: Vec::new(),
        },
        several => {
            let shared = several.iter().skip(1).fold(several[0].to_string(), |acc, word| {
                acc.chars()
                    .zip(word.chars())
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a)
                    .collect()
            });
            Completion {
                line: (shared.len() > line.len()).then_some(shared),
                candidates: several.iter().map(|s| s.to_string()).collect(),
            }
        }
    }
}

/// The worked examples that ship with this crate, as `(title, request)`.
///
/// The same file the `api` query serves, so an example a person loads here is
/// one an assistant was shown too.
pub fn examples() -> Vec<(String, String)> {
    let Ok(parsed) = json::parse(include_bytes!("examples.json")) else {
        return Vec::new();
    };
    parsed
        .as_array()
        .iter()
        .filter_map(|entry| {
            let title = entry.get("title")?.as_str()?.to_string();
            let request = entry.get("request")?;
            let one_line = json_write::to_string(request)
                .split('\n')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(" ");
            Some((title, one_line.trim().to_string()))
        })
        .collect()
}

/// A script file, as the lines to run.
///
/// Either a JSON request file — one object, or an array of them, which is what
/// the previous console saved — or a text script with one console line per
/// line and `#` for comments.
pub fn script_lines(text: &str) -> Result<Vec<String>, String> {
    let trimmed = text.trim_start_matches('\u{feff}').trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let value = json::parse(trimmed.as_bytes()).map_err(|e| format!("not valid JSON: {e}"))?;
        let flatten = |v: &Value| {
            json_write::to_string(v)
                .split('\n')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string()
        };
        return Ok(match &value {
            Value::Array(items) => items.iter().map(flatten).collect(),
            other => vec![flatten(other)],
        });
    }
    Ok(trimmed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect())
}

/// Lay an answer out for a person.
///
/// Pure, so the tables can be tested without an application: the value in is
/// exactly what a client would have received.
pub fn render(view: &View, value: &Value) -> String {
    match view {
        View::Json => json_write::to_string(value).trim_end().to_string(),
        View::Nodes { filter, parts_only } => nodes_table(value, filter.as_deref(), *parts_only),
        View::Materials => materials_table(value),
        View::Commands => commands_list(value),
        View::Jobs => jobs_table(value),
        View::History => history_line(value),
    }
}

fn jobs_table(value: &Value) -> String {
    let Some(all) = value.get("jobs").map(Value::as_array) else {
        return json_write::to_string(value).trim_end().to_string();
    };
    if all.is_empty() {
        return "nothing queued — `queue` adds a background render".into();
    }
    let mut out = format!(
        "{:>4}  {:<9}  {:>5}  {:<22}  {:>11}  {:>11}  {}\n",
        "id", "state", "done", "name", "size", "samples", "file"
    );
    for job in all {
        out.push_str(&format!(
            "{:>4}  {:<9}  {:>4.0}%  {:<22}  {:>11}  {:>11}  {}\n",
            number(job, "id").map(|i| i as u64).unwrap_or(0),
            text(job, "state"),
            number(job, "progress").unwrap_or(0.0) * 100.0,
            clip(text(job, "name"), 22),
            format!(
                "{}x{}",
                number(job, "width").unwrap_or(0.0) as u64,
                number(job, "height").unwrap_or(0.0) as u64
            ),
            format!(
                "{}/{}",
                number(job, "samples").unwrap_or(0.0) as u64,
                number(job, "max_samples").unwrap_or(0.0) as u64
            ),
            match text(job, "output") {
                "" => text(job, "message"),
                path => path,
            }
        ));
    }
    out.trim_end().to_string()
}

fn history_line(value: &Value) -> String {
    let step = |key: &str| match value.get(key) {
        Some(step) if !matches!(step, Value::Null) => {
            let commands = number(step, "commands").unwrap_or(1.0) as u64;
            if commands > 1 {
                format!("{} ({commands} commands)", text(step, "command"))
            } else {
                text(step, "command").to_string()
            }
        }
        _ => "nothing".to_string(),
    };
    let unsaved = value.get("unsaved").and_then(Value::as_bool).unwrap_or(false);
    format!(
        "undo: {}\nredo: {}\n{}",
        step("undo"),
        step("redo"),
        if unsaved { "unsaved changes" } else { "saved" }
    )
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

/// Clip to a column, marking the cut so a truncated name is not mistaken for
/// the whole of it.
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// A size as three short numbers. Metres at the scale products are modelled
/// at, so three decimals is a millimetre and anything finer is noise.
fn size_of(node: &Value) -> String {
    let size = node.get("bounds").and_then(|b| b.get("size"));
    match size.map(Value::as_array) {
        Some([x, y, z]) => format!(
            "{:.3} x {:.3} x {:.3}",
            x.as_f64().unwrap_or(0.0),
            y.as_f64().unwrap_or(0.0),
            z.as_f64().unwrap_or(0.0)
        ),
        _ => String::new(),
    }
}

fn nodes_table(value: &Value, filter: Option<&str>, parts_only: bool) -> String {
    let all = value.get("nodes").map(Value::as_array).unwrap_or(&[]);
    let needle = filter.map(str::to_lowercase);
    let eligible = |n: &&Value| !parts_only || text(n, "kind") == "part";
    let shown: Vec<&Value> = all
        .iter()
        .filter(eligible)
        .filter(|n| match &needle {
            None => true,
            Some(needle) => {
                let id = number(n, "id").map(|i| (i as u64).to_string()).unwrap_or_default();
                id == *needle
                    || text(n, "name").to_lowercase().contains(needle.as_str())
                    || text(n, "material_name").to_lowercase().contains(needle.as_str())
            }
        })
        .collect();

    let mut out = format!(
        "{:>6}  {:<5}  {:<28}  {:<24}  {:>8}  {}\n",
        "id", "kind", "name", "material", "tris", "size (m)"
    );
    for node in &shown {
        let id = number(node, "id").map(|i| i as u64).unwrap_or(0);
        let hidden = node.get("visible").and_then(Value::as_bool) == Some(false);
        let name = if hidden {
            format!("{} (hidden)", text(node, "name"))
        } else {
            text(node, "name").to_string()
        };
        let material = match (number(node, "material"), text(node, "material_name")) {
            (Some(key), name) if !name.is_empty() => format!("#{} {}", key as u64, name),
            (Some(key), _) => format!("#{}", key as u64),
            (None, _) => String::new(),
        };
        let tris = number(node, "triangles")
            .map(|t| (t as u64).to_string())
            .unwrap_or_default();
        out.push_str(&format!(
            "{:>6}  {:<5}  {:<28}  {:<24}  {:>8}  {}\n",
            id,
            text(node, "kind"),
            clip(&name, 28),
            clip(&material, 24),
            tris,
            size_of(node)
        ));
    }
    let noun = if parts_only { "parts" } else { "nodes" };
    match filter {
        Some(word) => out.push_str(&format!(
            "{} of {} {noun} match \"{word}\"",
            shown.len(),
            all.iter().filter(eligible).count()
        )),
        None => out.push_str(&format!("{} {noun}", shown.len())),
    }
    out
}

fn materials_table(value: &Value) -> String {
    let all = value.get("materials").map(Value::as_array).unwrap_or(&[]);
    let mut out = format!(
        "{:>4}  {:<16}  {:>5}  {:<30}  {}\n",
        "id", "type", "parts", "name", "maps"
    );
    for material in all {
        let maps: Vec<&str> = material
            .get("textures")
            .map(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .map(|m| text(m, "role"))
            .collect();
        out.push_str(&format!(
            "{:>4}  {:<16}  {:>5}  {:<30}  {}\n",
            number(material, "id").map(|i| i as u64).unwrap_or(0),
            clip(text(material, "type"), 16),
            number(material, "used_by").map(|u| u as u64).unwrap_or(0),
            clip(text(material, "name"), 30),
            maps.join(", ")
        ));
    }
    out.push_str(&format!(
        "{} materials. Every parameter: {{\"op\":\"query\",\"what\":\"materials\"}}",
        all.len()
    ));
    out
}

fn commands_list(value: &Value) -> String {
    let names: Vec<&str> = value
        .get("commands")
        .map(Value::as_array)
        .unwrap_or(&[])
        .iter()
        .filter_map(Value::as_str)
        .collect();
    // Grouped by the part before the dot, one line per group: the catalog is
    // organised that way and reads far faster like this than one per line.
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for name in &names {
        let (group, rest) = name.split_once('.').unwrap_or(("", name));
        groups.entry(group).or_default().push(rest);
    }
    let mut out = String::new();
    for (group, members) in groups {
        out.push_str(&format!("{group:<10} {}\n", members.join("  ")));
    }
    out.push_str(&format!(
        "{} commands. Run one as  group.name key=value ...  e.g. material.set_type material=2 type=velvet",
        names.len()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{parse_request, Operation, Query};
    use tequila_format::json_write::Object;

    /// Every console line has to become a request the real parser accepts,
    /// or the console is a second API that only looks like the first.
    fn operation(line: &str) -> Operation {
        match parse(line).expect("parses") {
            Parsed::Request { json, .. } => {
                parse_request(&json).expect("the protocol accepts it").operation
            }
            other => panic!("`{line}` was not a request: {other:?}"),
        }
    }

    #[test]
    fn short_commands_become_the_requests_a_client_would_send() {
        assert!(matches!(operation("ping"), Operation::Ping));
        assert!(matches!(operation("undo"), Operation::Undo));
        assert!(matches!(operation("scene"), Operation::Query(Query::Scene)));
        assert!(matches!(operation("context"), Operation::Query(Query::Context)));
        assert!(matches!(operation("nodes strap"), Operation::Query(Query::Nodes)));
        assert!(matches!(operation("frame"), Operation::Frame { all: true }));
        assert!(matches!(operation("frame sel"), Operation::Frame { all: false }));
        match operation("select 360 12") {
            Operation::Select(ids) => assert_eq!(ids, vec![360, 12]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_catalog_command_takes_typed_arguments() {
        match operation("material.set_type material=2 type=velvet") {
            Operation::Apply(command) => {
                assert_eq!(command.name(), "material.set_type");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Values are typed by their look, and quotes override the look.
    #[test]
    fn values_are_numbers_booleans_json_or_text() {
        let json = match parse(r#"x.y a=2 b=0.5 c=true d=[1, 2, 3] e="7" f=velvet g=nan"#).unwrap() {
            Parsed::Request { json, .. } => json,
            other => panic!("{other:?}"),
        };
        let command = json::parse(json.as_bytes()).unwrap();
        let command = command.get("command").unwrap();
        assert_eq!(command.get("a"), Some(&Value::Number(2.0)));
        assert_eq!(command.get("b"), Some(&Value::Number(0.5)));
        assert_eq!(command.get("c"), Some(&Value::Bool(true)));
        assert_eq!(command.get("d").map(|v| v.as_array().len()), Some(3));
        // Quoted, so a string — the whole reason quotedness is tracked.
        assert_eq!(command.get("e"), Some(&Value::String("7".into())));
        assert_eq!(command.get("f"), Some(&Value::String("velvet".into())));
        // Parses as a float, reads as a word; the word wins.
        assert_eq!(command.get("g"), Some(&Value::String("nan".into())));
    }

    /// A Windows path pasted from Explorer, spaces and backslashes included,
    /// has to arrive exactly as it was pasted.
    #[test]
    fn a_windows_path_survives_intact() {
        match operation(r#"import "C:\Users\someone\My Models\q3\scene.gltf""#) {
            Operation::Import { path, .. } => {
                assert_eq!(path, r"C:\Users\someone\My Models\q3\scene.gltf")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn render_draws_a_wireframe_when_asked() {
        match operation("render lines.png 1920x1080 wireframe") {
            Operation::Render { width, height, wireframe, .. } => assert_eq!((width, height, wireframe), (1920, 1080, true)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn render_takes_a_size_either_way() {
        for line in ["render out.png 1200 800 64", "render out.png 1200x800 64"] {
            match operation(line) {
                Operation::Render { width, height, samples, .. } => {
                    assert_eq!((width, height, samples), (1200, 800, 64), "{line}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn raw_json_and_apply_pass_through() {
        assert!(matches!(
            operation(r#"{"op":"query","what":"camera"}"#),
            Operation::Query(Query::Camera)
        ));
        assert!(matches!(
            operation(r#"apply {"command":"scene.add_group","name":"Rig"}"#),
            Operation::Apply(_)
        ));
    }

    #[test]
    fn mistakes_are_explained_rather_than_guessed_at() {
        assert!(parse("selct 1").unwrap_err().contains("select"), "a near miss is suggested");
        assert!(parse("select strap").unwrap_err().contains("not a node id"));
        assert!(parse("material.set_type material").unwrap_err().contains("key=value"));
        assert!(parse(r#"import "unclosed"#).unwrap_err().contains("quote"));
        assert!(parse("x.y v=[1, 2").unwrap_err().contains("bracket"));
    }

    #[test]
    fn comments_and_blank_lines_are_nothing() {
        assert_eq!(parse("   "), Ok(Parsed::Empty));
        assert_eq!(parse("# a note"), Ok(Parsed::Empty));
    }

    #[test]
    fn tab_completes_the_first_word() {
        let catalog = ["material.set_type", "material.set_texture", "material.unlink"];
        // One match: the whole word and a space, ready for arguments.
        assert_eq!(complete("scr", &catalog).line.as_deref(), Some("screenshot "));
        // Several: as far as they agree, and the list.
        let several = complete("material.set_t", &catalog);
        // Nothing to add: the two already agree as far as was typed.
        assert_eq!(several.line, None);
        assert_eq!(several.candidates.len(), 2);
        let shared = complete("mat", &catalog);
        assert_eq!(shared.line.as_deref(), Some("material"));
        // Nothing after the first word: arguments are ids it cannot know.
        assert_eq!(complete("select 3", &catalog), Completion { line: None, candidates: vec![] });
    }

    #[test]
    fn every_shipped_example_is_a_request_the_protocol_accepts() {
        let examples = examples();
        assert!(!examples.is_empty(), "the examples file did not load");
        for (title, request) in examples {
            parse_request(&request).unwrap_or_else(|e| panic!("`{title}`: {e}"));
        }
    }

    #[test]
    fn a_script_is_lines_or_a_json_file() {
        let text = "# set up\nscene\n\nframe\n";
        assert_eq!(script_lines(text).unwrap(), vec!["scene", "frame"]);
        let json = r#"[{"op":"ping"}, {"op":"query","what":"scene"}]"#;
        assert_eq!(script_lines(json).unwrap().len(), 2);
        // A single pretty-printed request, the shape the old Save… wrote.
        let single = "{\n  \"op\": \"ping\"\n}\n";
        let lines = script_lines(single).unwrap();
        assert_eq!(lines.len(), 1);
        parse_request(&lines[0]).expect("still a request");
    }


    fn node(id: u64, name: &str, kind: &str, material: Option<(u64, &str)>) -> Value {
        let mut n = Object::new();
        n.set("id", id as f64).set("name", name).set("kind", kind).set("visible", true);
        if let Some((key, label)) = material {
            n.set("material", key as f64).set("material_name", label);
        }
        n.build()
    }

    #[test]
    fn the_node_table_filters_by_name_id_and_material() {
        let mut v = Object::new();
        v.set(
            "nodes",
            Value::Array(vec![
                node(1, "Quest 3", "group", None),
                node(360, "part 12", "part", Some((2, "Strap Felt"))),
                node(361, "Lens", "part", Some((3, "Glass"))),
            ]),
        );
        let v = v.build();
        let view = |filter: Option<&str>, parts_only| {
            render(&View::Nodes { filter: filter.map(str::to_string), parts_only }, &v)
        };
        assert!(view(None, false).ends_with("3 nodes"));
        assert!(view(None, true).ends_with("2 parts"));
        // By material name, case-insensitively: the question people actually
        // ask of a split model is "which parts are the strap".
        let strap = view(Some("strap"), false);
        assert!(strap.contains("part 12") && !strap.contains("Lens"), "{strap}");
        assert!(strap.ends_with("1 of 3 nodes match \"strap\""), "{strap}");
        let by_id = view(Some("361"), false);
        assert!(by_id.contains("Lens") && !by_id.contains("part 12"), "{by_id}");
    }

    #[test]
    fn a_long_name_is_clipped_visibly() {
        assert_eq!(clip("short", 8), "short");
        assert_eq!(clip("a very long name", 8), "a very \u{2026}");
    }

    #[test]
    fn the_control_verbs_become_requests() {
        use crate::protocol::Look;
        assert!(matches!(operation("quit"), Operation::Quit { save: false, discard: false }));
        assert!(matches!(operation("quit save"), Operation::Quit { save: true, .. }));
        assert!(matches!(operation("quit discard"), Operation::Quit { discard: true, .. }));
        assert!(matches!(operation("stop"), Operation::Stop));
        assert!(matches!(operation("jobs"), Operation::Query(Query::Jobs)));
        assert!(matches!(operation("changes"), Operation::Query(Query::History)));
        assert!(matches!(operation("ui"), Operation::Query(Query::Ui)));

        match operation("show material") {
            Operation::Ui(change) => assert_eq!(change.tab.as_deref(), Some("material")),
            other => panic!("{other:?}"),
        }
        match operation("hide console") {
            Operation::Ui(change) => assert_eq!(change.console, Some(false)),
            other => panic!("{other:?}"),
        }
        match operation("show render") {
            Operation::Ui(change) => assert_eq!(change.render_window, Some(true)),
            other => panic!("{other:?}"),
        }
        match operation("viewport rendered") {
            Operation::Ui(change) => assert_eq!(change.viewport.as_deref(), Some("rendered")),
            other => panic!("{other:?}"),
        }
        match operation("screenshot shot.png viewport") {
            Operation::Screenshot { viewport, .. } => assert!(viewport),
            other => panic!("{other:?}"),
        }
        match operation("screenshot shot.png Graph") {
            Operation::Screenshot { viewport, window, .. } => {
                assert!(!viewport);
                assert_eq!(window.as_deref(), Some("graph"));
            }
            other => panic!("{other:?}"),
        }
        match operation("look front") {
            Operation::Look(look) => assert_eq!(look, Look { view: Some("front".into()), ..Default::default() }),
            other => panic!("{other:?}"),
        }
        match operation("look yaw=30 pitch=15 distance=2.5 focal=85") {
            Operation::Look(look) => {
                assert_eq!((look.yaw, look.pitch, look.distance), (Some(30.0), Some(15.0), Some(2.5)));
                assert_eq!(look.focal_length, Some(85.0));
            }
            other => panic!("{other:?}"),
        }
        match operation("look from=[1, 1, 2] at=[0, 0.5, 0]") {
            Operation::Look(look) => assert_eq!((look.from, look.at), (Some([1.0, 1.0, 2.0]), Some([0.0, 0.5, 0.0]))),
            other => panic!("{other:?}"),
        }
        match operation("queue 1920x1080 256 name=Hero denoise=off") {
            Operation::Queue(job) => {
                assert_eq!((job.width, job.height, job.samples), (Some(1920), Some(1080), Some(256)));
                assert_eq!(job.name.as_deref(), Some("Hero"));
                assert_eq!(job.denoise, Some(false));
            }
            other => panic!("{other:?}"),
        }
    }

    /// Semicolons make a batch, and a batch is one undo step.
    #[test]
    fn texture_set_takes_a_pasted_path() {
        match operation(r#"texture_set "C:\Users\someone\Downloads\Wood049_2K-JPG.zip""#) {
            Operation::TextureSet { path, nodes } => {
                assert_eq!(path, r"C:\Users\someone\Downloads\Wood049_2K-JPG.zip");
                assert!(nodes.is_empty());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn semicolons_make_one_batch() {
        match operation("select 360; frame sel; material.set_type material=2 type=velvet") {
            Operation::Batch { operations, atomic } => {
                assert!(atomic);
                assert_eq!(operations.len(), 3);
                assert!(matches!(operations[1], Operation::Frame { all: false }));
            }
            other => panic!("{other:?}"),
        }
        // A semicolon inside quotes or brackets is not a separator.
        match operation(r#"import "C:/odd;name/model.glb""#) {
            Operation::Import { path, .. } => assert_eq!(path, "C:/odd;name/model.glb"),
            other => panic!("{other:?}"),
        }
        let error = parse("select 1; help").unwrap_err();
        assert!(error.contains("part 2"), "{error}");
    }

    #[test]
    fn help_covers_every_verb() {
        for verb in VERBS {
            assert!(
                // `_` belongs to a word: `texture_set` is one verb.
                HELP.iter().any(|(usage, _)| usage.split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|w| w == *verb)),
                "`{verb}` has no line in help"
            );
        }
    }

    /// `library` lists, `library apply` puts a preset on the selection by
    /// its name - spaces and all, unquoted - and `library import` brings a
    /// file in; anything else is refused with what would work.
    #[test]
    fn the_library_verb_lists_applies_and_imports() {
        let field = |line: &str, key: &str| -> String {
            let Ok(Parsed::Request { json, .. }) = parse(line) else { panic!("{line}: no request") };
            let value = tequila_format::json::parse(json.as_bytes()).expect("json");
            value.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string()
        };
        assert_eq!(field("library", "what"), "library");
        assert_eq!(field("library apply Aluminium Spun", "action"), "apply");
        assert_eq!(field("library apply Aluminium Spun", "name"), "Aluminium Spun");
        assert_eq!(field("library import \"C:/shared/My Blue.fsmat\"", "path"), "C:/shared/My Blue.fsmat");
        assert!(parse("library apply").is_err());
        assert!(parse("library melt it").is_err());
    }
}
