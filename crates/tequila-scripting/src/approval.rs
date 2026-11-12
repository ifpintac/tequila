//! What the built-in Assistant asks the person before doing.
//!
//! An agent working in Tequila's chat panel may change the scene freely:
//! every change is one step to undo, and "Undo request" takes back all of
//! them. What leaves the scene cannot be taken back that way - a file
//! brought in or written, a render saved, a preference changed, a plugin's
//! code run - so those wait for the person to say yes.
//!
//! The decision is made here, by Tequila, on the request itself, and not by
//! the agent that sent it: agents differ in what they ask about, and their
//! own settings can tell them not to ask at all. [`question`] is put to
//! every request that carries the Assistant's mark
//! ([`crate::protocol::Request::agent`]); a request it has a question for is
//! held until the person answers in the chat.

use crate::protocol::{Operation, ProjectAction};
use std::path::Path;

/// What to ask the person, and what it is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    /// The question, a few words: "Import a file?"
    pub title: String,
    /// What exactly: the file, the size, the setting.
    pub detail: String,
}

fn ask(title: &str, detail: impl Into<String>) -> Option<Question> {
    Some(Question { title: title.to_string(), detail: detail.into() })
}

/// Whether a picture's path is one of the scratch files the Assistant's
/// tools write to look at their own work: in the temporary folder, where
/// nothing of the person's is.
fn scratch(path: &str, temporary: &Path) -> bool {
    let path = Path::new(path);
    path.is_absolute() && path.starts_with(temporary) && !path.components().any(|c| matches!(c, std::path::Component::ParentDir))
}

/// The question to ask before `operation`, or `None` when it may go ahead.
/// `temporary` is the system's folder for scratch files.
pub fn question(operation: &Operation, temporary: &Path) -> Option<Question> {
    match operation {
        Operation::Import { path, .. } => ask("Import a file?", path.clone()),
        Operation::TextureSet { path, .. } => ask("Bring in a texture pack?", path.clone()),
        Operation::Reimport { path: Some(path), .. } => ask("Update the model from another file?", path.clone()),
        Operation::Save { path: Some(path) } => ask("Save the project to a new place?", path.clone()),
        // A look at the scene, written where the tool reads it back from,
        // is not a file of the person's; one written anywhere else is.
        Operation::Screenshot { path, .. } if !scratch(path, temporary) => ask("Save a screenshot to a file?", path.clone()),
        Operation::Render { path, width, height, .. } if !scratch(path, temporary) => ask("Render to a file?", format!("{path}, {width} \u{d7} {height}")),
        Operation::Queue(job) => {
            let mut detail = Vec::new();
            match (&job.folder, &job.filename) {
                (Some(folder), Some(file)) => detail.push(format!("{folder}/{file}")),
                (Some(folder), None) => detail.push(folder.clone()),
                (None, Some(file)) => detail.push(file.clone()),
                (None, None) => detail.push("In the project's render folder".to_string()),
            }
            if let (Some(width), Some(height)) = (job.width, job.height) {
                detail.push(format!("{width} \u{d7} {height}"));
            }
            if job.studios == Some(true) {
                detail.push("every Studio".to_string());
            }
            if job.configurations == Some(true) {
                detail.push("every configuration".to_string());
            }
            ask("Render to a file?", detail.join(", "))
        }
        Operation::Export { format, path, .. } => ask("Export the scene to a file?", format!("{path} ({format})")),
        Operation::StudioPreset(request) => match request.get("action").and_then(|a| a.as_str()).unwrap_or_default() {
            "render_all" => ask("Render every Studio to files?", "In the project's render folder"),
            "save" => ask("Save a Studio to your library?", request.get("name").and_then(|n| n.as_str()).unwrap_or_default()),
            "delete" => ask("Delete a Studio from your library?", request.get("preset").or(request.get("name")).and_then(|n| n.as_str()).unwrap_or_default()),
            "install" | "export" => ask("Read or write a Studio file?", request.get("path").and_then(|n| n.as_str()).unwrap_or_default()),
            _ => None,
        },
        Operation::Preferences(request) => {
            let changes: Vec<String> = match request.get("set") {
                Some(tequila_format::json::Value::Object(set)) => set.keys().cloned().collect(),
                _ => Vec::new(),
            };
            match (changes.is_empty(), request.get("reset")) {
                (false, _) => ask("Change Tequila's preferences?", changes.join(", ")),
                (true, Some(what)) => ask("Reset Tequila's preferences?", what.as_str().unwrap_or("all")),
                // Only reading them.
                (true, None) => None,
            }
        }
        Operation::Library(request) => match request.action.as_str() {
            "save" => ask("Save a material to your library?", request.name.clone().unwrap_or_default()),
            "import" => ask("Bring a file into your library?", request.path.clone().unwrap_or_default()),
            "export" => ask("Export a material to a file?", request.path.clone().unwrap_or_default()),
            "delete" => ask("Delete a material from your library?", request.name.clone().unwrap_or_default()),
            _ => None,
        },
        Operation::Material(request) => match request.action.as_str() {
            "save" => ask("Save a material to your library?", request.name.clone().unwrap_or_default()),
            "preview" => request.path.as_deref().filter(|path| !scratch(path, temporary)).and_then(|path| ask("Save a material's picture to a file?", path)),
            _ => None,
        },
        Operation::Plugin(request) => {
            let plugin = request.plugin.clone().unwrap_or_default();
            match request.action.as_str() {
                "write" => {
                    let files: Vec<&str> = request.files.iter().map(|(path, _)| path.as_str()).collect();
                    ask("Write a plugin's code?", format!("{plugin}: {}", files.join(", ")))
                }
                "run" => ask("Run a plugin?", format!("{plugin}: {}", request.command.clone().unwrap_or_default())),
                "restart" => ask("Restart a plugin?", plugin),
                // Turning one on has a question of its own, in the Plugins
                // window; the rest only read.
                _ => None,
            }
        }
        Operation::Project { action, .. } => match action {
            ProjectAction::New => ask("Start a new project?", "The open one is closed"),
            ProjectAction::Open(path) => ask("Open another project?", path.clone()),
        },
        Operation::Quit { .. } => ask("Quit Tequila?", ""),
        // A Substance file opened is a file brought in; its settings and
        // state are the scene's.
        Operation::Substance(request) => match request.get("action").and_then(|a| a.as_str()).unwrap_or_default() {
            "open" => ask("Open a Substance file?", request.get("path").and_then(|p| p.as_str()).unwrap_or_default()),
            _ => None,
        },
        // A modelling application's link is that application's to send.
        Operation::LiveLink(_) => ask("Send a LiveLink message?", ""),
        // One question for the whole batch, naming each thing in it that
        // would have asked.
        Operation::Batch { operations, .. } => {
            let inside: Vec<Question> = operations.iter().filter_map(|operation| question(operation, temporary)).collect();
            match inside.as_slice() {
                [] => None,
                [one] => Some(one.clone()),
                several => ask(
                    "Let these steps go ahead?",
                    several.iter().map(|q| format!("{} {}", q.title.trim_end_matches('?'), q.detail).trim().to_string()).collect::<Vec<_>>().join("; "),
                ),
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::parse_request;

    /// The question for a request. `SCRATCH` in it stands for this
    /// computer's folder for scratch files, written as JSON writes a path.
    fn asked(json: &str) -> Option<Question> {
        let temporary = std::env::temp_dir();
        let json = json.replace("SCRATCH", &temporary.join("x").to_string_lossy().trim_end_matches('x').replace('\\', "\\\\"));
        let request = parse_request(&json).unwrap_or_else(|e| panic!("{json}: {e:?}"));
        question(&request.operation, &temporary)
    }

    #[test]
    fn scene_edits_and_looking_go_ahead_unasked() {
        for request in [
            r#"{"op":"query","what":"scene"}"#,
            r#"{"op":"apply","command":{"command":"scene.rename","node":1,"name":"A"}}"#,
            r#"{"op":"undo"}"#,
            r#"{"op":"select","nodes":[1]}"#,
            r#"{"op":"frame","all":true}"#,
            r#"{"op":"save"}"#,
            r#"{"op":"screenshot","path":"SCRATCHtequila-mcp-1.png","viewport":true}"#,
            r#"{"op":"render","path":"SCRATCHtequila-mcp-render-1.png","width":640,"height":400,"samples":32}"#,
            r#"{"op":"material","action":"guide"}"#,
            r#"{"op":"material","action":"preview","material":1,"path":"SCRATCHball.png"}"#,
            r#"{"op":"preferences"}"#,
            r#"{"op":"plugin","action":"list"}"#,
            r#"{"op":"plugin","action":"enable","plugin":"my.hello"}"#,
            r#"{"op":"studio_preset","action":"list"}"#,
            r#"{"op":"library","action":"apply","name":"Brushed Aluminium","nodes":[1]}"#,
            r#"{"op":"studio_kit","piece":"softbox"}"#,
            r#"{"op":"batch","ops":[{"op":"frame","all":true},{"op":"select","nodes":[1]}]}"#,
        ] {
            assert_eq!(asked(request), None, "{request}");
        }
    }

    #[test]
    fn what_leaves_the_scene_asks_first_and_says_what() {
        let cases = [
            (r#"{"op":"import","path":"/models/speaker.step"}"#, "Import a file?", "/models/speaker.step"),
            (r#"{"op":"save","path":"/projects/copy.tequila"}"#, "Save the project to a new place?", "/projects/copy.tequila"),
            (r#"{"op":"render","path":"/projects/shot.png","width":3840,"height":2160,"samples":64}"#, "Render to a file?", "/projects/shot.png, 3840 \u{d7} 2160"),
            (r#"{"op":"screenshot","path":"/projects/look.png"}"#, "Save a screenshot to a file?", "/projects/look.png"),
            (r#"{"op":"queue","width":1920,"height":1080}"#, "Render to a file?", "In the project's render folder, 1920 \u{d7} 1080"),
            (r#"{"op":"queue","folder":"/renders","filename":"front.png"}"#, "Render to a file?", "/renders/front.png"),
            (r#"{"op":"preferences","set":{"viewport.gizmo_size":150}}"#, "Change Tequila's preferences?", "viewport.gizmo_size"),
            (r#"{"op":"plugin","action":"run","plugin":"my.hello","command":"go"}"#, "Run a plugin?", "my.hello: go"),
            (r#"{"op":"plugin","action":"write","plugin":"my.hello","files":{"main.py":"print(1)"}}"#, "Write a plugin's code?", "my.hello: main.py"),
            (r#"{"op":"material","action":"save","material":1,"name":"Blue terrazzo"}"#, "Save a material to your library?", "Blue terrazzo"),
            (r#"{"op":"studio_preset","action":"render_all"}"#, "Render every Studio to files?", "In the project's render folder"),
            (r#"{"op":"export","format":"glb","path":"/out/speaker.glb"}"#, "Export the scene to a file?", "/out/speaker.glb (glb)"),
            (r#"{"op":"library","action":"import","path":"/packs/oak"}"#, "Bring a file into your library?", "/packs/oak"),
            (r#"{"op":"quit","discard":true}"#, "Quit Tequila?", ""),
            (r#"{"op":"new","discard":true}"#, "Start a new project?", "The open one is closed"),
        ];
        for (request, title, detail) in cases {
            assert_eq!(asked(request), Some(Question { title: title.into(), detail: detail.into() }), "{request}");
        }
    }

    /// A path that only starts in the scratch folder is not a scratch file.
    #[test]
    fn a_picture_that_climbs_out_of_the_scratch_folder_asks() {
        assert!(asked(r#"{"op":"screenshot","path":"SCRATCH../../Users/me/Desktop/a.png"}"#).is_some());
        assert!(asked(r#"{"op":"screenshot","path":"scratch.png"}"#).is_some());
    }

    #[test]
    fn a_batch_asks_once_for_everything_in_it() {
        assert_eq!(
            asked(r#"{"op":"batch","ops":[{"op":"frame","all":true},{"op":"save","path":"/p/copy.tequila"}]}"#).map(|q| q.title),
            Some("Save the project to a new place?".into())
        );
        let both = asked(r#"{"op":"batch","ops":[{"op":"save","path":"/p/copy.tequila"},{"op":"queue","folder":"/renders"}]}"#).unwrap();
        assert_eq!(both.title, "Let these steps go ahead?");
        assert_eq!(both.detail, "Save the project to a new place /p/copy.tequila; Render to a file /renders");
    }
}
