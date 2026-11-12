//! A chat: what was asked, said and done, as the panel shows it, and kept
//! between sessions so it can be opened again and carried on.
//!
//! Every way of answering - an agent reached over the Agent Client Protocol
//! ([`crate::acp`]), or Tequila's own loop for API keys and models on this
//! computer ([`crate::session`]) - writes into this one list, so the panel
//! draws the same thing whichever is answering: the person's messages, the
//! reply as it streams, the reasoning folded away, each tool call with how it
//! went, the questions the person was asked before something left the scene,
//! and the pictures along the way.
//!
//! A chat is one JSON file in Tequila's `chats` folder, with its pictures as
//! PNG files in a folder beside it: the text stays small enough to list every
//! chat quickly, and a picture is written once however often the chat is
//! saved.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};

/// A picture in a chat: one the person attached, the viewport they sent, or
/// one a tool took.
#[derive(Clone, Debug)]
pub struct Picture {
    /// Unlike any other picture's in this run of the application: what the
    /// panel keeps its drawn copy under.
    pub id: u64,
    pub png: Arc<Vec<u8>>,
}

impl Picture {
    pub fn new(png: Vec<u8>) -> Picture {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Picture { id: NEXT.fetch_add(1, Ordering::Relaxed), png: Arc::new(png) }
    }
}

impl PartialEq for Picture {
    fn eq(&self, other: &Picture) -> bool {
        self.png == other.png
    }
}

/// How a tool call stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Done,
    Failed,
}

impl Status {
    fn key(self) -> &'static str {
        match self {
            Status::Running => "running",
            Status::Done => "done",
            Status::Failed => "failed",
        }
    }

    fn from_key(key: &str) -> Status {
        match key {
            "running" => Status::Running,
            "failed" => Status::Failed,
            _ => Status::Done,
        }
    }
}

/// One tool call: what it is, in words a person reads, and how it went.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// The backend's id for the call, by which later news of it is matched.
    pub id: String,
    pub title: String,
    pub status: Status,
    /// Why it failed, or more about what it did: shown when asked for.
    pub detail: String,
    pub pictures: Vec<Picture>,
}

/// News of a tool call: a new one, or more about one already listed. What
/// is `None` is left as it was.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepNews {
    pub id: String,
    pub title: Option<String>,
    pub status: Option<Status>,
    pub detail: Option<String>,
    pub pictures: Vec<Picture>,
}

/// Something the person was asked before it happened, and what they said.
#[derive(Clone, Debug, PartialEq)]
pub struct Asked {
    /// Which question this is, for the answer to find it.
    pub id: u64,
    pub title: String,
    pub detail: String,
    /// `None` while it waits for the person.
    pub answer: Option<bool>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    You { text: String, pictures: Vec<Picture> },
    Said(String),
    /// What the model reasoned before a reply, and for how long.
    Thought { text: String, seconds: f32 },
    Step(Step),
    Asked(Asked),
    Problem(String),
    Note(String),
}

/// One conversation.
#[derive(Clone, Debug)]
pub struct Chat {
    /// Its file's name, without the extension.
    pub id: String,
    pub title: String,
    /// Which backend answered last - `claude`, or a provider's id - and with
    /// which model.
    pub backend: String,
    pub model: String,
    /// The agent's own name for the conversation, where it keeps one: what
    /// carrying on a reopened chat asks it for.
    pub session: Option<String>,
    /// The folder the agent worked in, which its session belongs to.
    pub folder: Option<PathBuf>,
    /// Seconds since 1970.
    pub created: u64,
    pub updated: u64,
    pub items: Vec<Item>,
    /// When the reasoning now streaming began. Not kept.
    thinking_since: Option<Instant>,
    /// Whether anything has changed since it was last saved. Not kept.
    pub unsaved: bool,
}

/// A chat in the list of past chats.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub backend: String,
    pub model: String,
    pub updated: u64,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A title for a chat from its first message: the first line, cut at a word.
pub fn title_from(text: &str) -> String {
    const LONGEST: usize = 48;
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() <= LONGEST {
        return line.to_string();
    }
    let mut cut: String = line.chars().take(LONGEST).collect();
    if let Some(space) = cut.rfind(' ').filter(|at| *at > LONGEST / 2) {
        cut.truncate(space);
    }
    format!("{}\u{2026}", cut.trim_end_matches([',', '.', ';', ':', ' ']))
}

impl Chat {
    pub fn new() -> Chat {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let id = format!("{millis:x}-{:x}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
        Chat {
            id,
            title: String::new(),
            backend: String::new(),
            model: String::new(),
            session: None,
            folder: None,
            created: now(),
            updated: now(),
            items: Vec::new(),
            thinking_since: None,
            unsaved: false,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn touch(&mut self) {
        self.updated = now();
        self.unsaved = true;
    }

    /// Anything but more reasoning ends the reasoning that was streaming:
    /// how long it took is now known.
    fn settle_thought(&mut self) {
        if let (Some(since), Some(Item::Thought { seconds, .. })) = (self.thinking_since.take(), self.items.last_mut()) {
            *seconds = since.elapsed().as_secs_f32();
        }
    }

    pub fn push(&mut self, item: Item) {
        self.settle_thought();
        if let (Item::You { text, .. }, true) = (&item, self.title.is_empty()) {
            self.title = title_from(text);
        }
        self.items.push(item);
        self.touch();
    }

    /// More of the reply, as it streams: joined to what came just before.
    pub fn say(&mut self, more: &str) {
        if more.is_empty() {
            return;
        }
        self.settle_thought();
        match self.items.last_mut() {
            Some(Item::Said(text)) => text.push_str(more),
            _ => self.items.push(Item::Said(more.to_string())),
        }
        self.touch();
    }

    /// More of the model's reasoning, as it streams.
    pub fn think(&mut self, more: &str) {
        if more.is_empty() {
            return;
        }
        match (self.items.last_mut(), self.thinking_since) {
            (Some(Item::Thought { text, seconds }), Some(since)) => {
                text.push_str(more);
                *seconds = since.elapsed().as_secs_f32();
            }
            _ => {
                self.thinking_since = Some(Instant::now());
                self.items.push(Item::Thought { text: more.to_string(), seconds: 0.0 });
            }
        }
        self.touch();
    }

    /// Whether the last thing in the chat is reasoning still streaming.
    pub fn is_thinking(&self) -> bool {
        self.thinking_since.is_some() && matches!(self.items.last(), Some(Item::Thought { .. }))
    }

    /// A tool call, or more about one: found by its id among the steps of
    /// the request being answered. An id is a backend's own, and the next
    /// conversation - or the same agent started again - uses the same ones
    /// over, so a step of an earlier request is never taken for it.
    pub fn step(&mut self, news: StepNews) {
        self.settle_thought();
        let found = self.items.iter_mut().rev().take_while(|item| !matches!(item, Item::You { .. })).find_map(|item| match item {
            Item::Step(step) if step.id == news.id => Some(step),
            _ => None,
        });
        match found {
            Some(step) => {
                if let Some(title) = news.title.filter(|t| !t.trim().is_empty()) {
                    step.title = title;
                }
                if let Some(status) = news.status {
                    step.status = status;
                }
                if let Some(detail) = news.detail {
                    step.detail = detail;
                }
                step.pictures.extend(news.pictures);
            }
            None => self.items.push(Item::Step(Step {
                id: news.id,
                title: news.title.unwrap_or_default(),
                status: news.status.unwrap_or(Status::Running),
                detail: news.detail.unwrap_or_default(),
                pictures: news.pictures,
            })),
        }
        self.touch();
    }

    /// The person's answer to a question: true when it was waiting for one.
    pub fn answer(&mut self, id: u64, allowed: bool) -> bool {
        for item in self.items.iter_mut().rev() {
            if let Item::Asked(asked) = item {
                if asked.id == id && asked.answer.is_none() {
                    asked.answer = Some(allowed);
                    self.touch();
                    return true;
                }
            }
        }
        false
    }

    /// The request is over, finished or stopped: nothing is left running or
    /// waiting to be answered.
    pub fn settle(&mut self) {
        self.settle_thought();
        let mut changed = false;
        for item in &mut self.items {
            match item {
                Item::Step(step) if step.status == Status::Running => {
                    step.status = Status::Failed;
                    if step.detail.is_empty() {
                        step.detail = "It was stopped before it finished.".into();
                    }
                    changed = true;
                }
                Item::Asked(asked) if asked.answer.is_none() => {
                    asked.answer = Some(false);
                    changed = true;
                }
                _ => {}
            }
        }
        if changed {
            self.touch();
        }
    }

    /// The chat as text, without its pictures: each is named by where it
    /// stands among the chat's pictures.
    fn to_json(&self) -> String {
        let mut next = 0usize;
        let mut names = |pictures: &[Picture]| -> Value {
            Value::Array(
                pictures
                    .iter()
                    .map(|_| {
                        next += 1;
                        Value::String(format!("{next}.png"))
                    })
                    .collect(),
            )
        };
        let mut items = Vec::new();
        for item in &self.items {
            let mut out = Object::new();
            match item {
                Item::You { text, pictures } => {
                    out.set("kind", "you").set("text", text.clone()).set("pictures", names(pictures));
                }
                Item::Said(text) => {
                    out.set("kind", "said").set("text", text.clone());
                }
                Item::Thought { text, seconds } => {
                    out.set("kind", "thought").set("text", text.clone()).set("seconds", *seconds);
                }
                Item::Step(step) => {
                    out.set("kind", "step")
                        .set("id", step.id.clone())
                        .set("title", step.title.clone())
                        .set("status", step.status.key())
                        .set("detail", step.detail.clone())
                        .set("pictures", names(&step.pictures));
                }
                Item::Asked(asked) => {
                    out.set("kind", "asked").set("title", asked.title.clone()).set("detail", asked.detail.clone());
                    out.set_some("answer", asked.answer);
                }
                Item::Problem(text) => {
                    out.set("kind", "problem").set("text", text.clone());
                }
                Item::Note(text) => {
                    out.set("kind", "note").set("text", text.clone());
                }
            }
            items.push(out.build());
        }
        let mut out = Object::new();
        out.set("id", self.id.clone())
            .set("title", self.title.clone())
            .set("backend", self.backend.clone())
            .set("model", self.model.clone())
            .set("created", self.created)
            .set("updated", self.updated)
            .set("items", Value::Array(items));
        out.set_some("session", self.session.clone());
        out.set_some("folder", self.folder.as_ref().map(|f| f.to_string_lossy().into_owned()));
        json_write::to_string(&out.build())
    }

    /// A chat read back. `picture` fetches one of its pictures by name; one
    /// that is gone is left out.
    fn from_json(text: &str, picture: impl Fn(&str) -> Option<Vec<u8>>) -> Result<Chat, String> {
        let value = json::parse(text.as_bytes()).map_err(|e| format!("the chat could not be read: {e}"))?;
        let word = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
        let pictures = |value: &Value| -> Vec<Picture> {
            value
                .get("pictures")
                .map(Value::as_array)
                .unwrap_or(&[])
                .iter()
                .filter_map(|name| name.as_str().and_then(&picture).map(Picture::new))
                .collect()
        };
        let mut items = Vec::new();
        for (index, item) in value.get("items").map(Value::as_array).unwrap_or(&[]).iter().enumerate() {
            let text = word(item, "text");
            items.push(match item.get("kind").and_then(Value::as_str).unwrap_or_default() {
                "you" => Item::You { text, pictures: pictures(item) },
                "said" => Item::Said(text),
                "thought" => Item::Thought { text, seconds: item.get("seconds").and_then(Value::as_f32).unwrap_or(0.0) },
                "step" => Item::Step(Step {
                    id: word(item, "id"),
                    title: word(item, "title"),
                    // Nothing read back from a file is still running.
                    status: match Status::from_key(&word(item, "status")) {
                        Status::Running => Status::Failed,
                        other => other,
                    },
                    detail: word(item, "detail"),
                    pictures: pictures(item),
                }),
                "asked" => Item::Asked(Asked {
                    id: index as u64,
                    title: word(item, "title"),
                    detail: word(item, "detail"),
                    // Nor is a question still waiting for its answer.
                    answer: Some(item.get("answer").and_then(Value::as_bool).unwrap_or(false)),
                }),
                "problem" => Item::Problem(text),
                "note" => Item::Note(text),
                // Something a later version wrote: left out, the rest kept.
                _ => continue,
            });
        }
        let number = |key: &str| value.get(key).and_then(Value::as_f64).unwrap_or(0.0) as u64;
        Ok(Chat {
            id: word(&value, "id"),
            title: word(&value, "title"),
            backend: word(&value, "backend"),
            model: word(&value, "model"),
            session: value.get("session").and_then(Value::as_str).map(str::to_string),
            folder: value.get("folder").and_then(Value::as_str).map(PathBuf::from),
            created: number("created"),
            updated: number("updated"),
            items,
            thinking_since: None,
            unsaved: false,
        })
    }

    fn pictures(&self) -> impl Iterator<Item = &Picture> {
        self.items.iter().flat_map(|item| match item {
            Item::You { pictures, .. } => pictures.as_slice(),
            Item::Step(step) => step.pictures.as_slice(),
            _ => &[],
        })
    }
}

impl Default for Chat {
    fn default() -> Chat {
        Chat::new()
    }
}

/// The folder past chats are kept in.
#[derive(Clone, Debug)]
pub struct Store {
    folder: PathBuf,
}

/// A chat's id is its file's name, and comes back from a file or a script:
/// only what [`Chat::new`] writes is let through, so no id names a file
/// anywhere else.
fn safe(id: &str) -> Result<&str, String> {
    let fine = !id.is_empty() && id.len() <= 80 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    fine.then_some(id).ok_or_else(|| format!("{id:?} is not a chat"))
}

impl Store {
    pub fn new(folder: impl Into<PathBuf>) -> Store {
        Store { folder: folder.into() }
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn file(&self, id: &str) -> PathBuf {
        self.folder.join(format!("{id}.json"))
    }

    /// Every chat kept, the most recent first.
    pub fn list(&self) -> Vec<Summary> {
        let Ok(entries) = std::fs::read_dir(&self.folder) else { return Vec::new() };
        let mut chats: Vec<Summary> = entries
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                if path.extension()? != "json" {
                    return None;
                }
                let chat = Chat::from_json(&std::fs::read_to_string(&path).ok()?, |_| None).ok()?;
                safe(&chat.id).ok()?;
                (path.file_stem()?.to_str()? == chat.id).then_some(Summary {
                    id: chat.id,
                    title: chat.title,
                    backend: chat.backend,
                    model: chat.model,
                    updated: chat.updated,
                })
            })
            .collect();
        chats.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| b.id.cmp(&a.id)));
        chats
    }

    pub fn load(&self, id: &str) -> Result<Chat, String> {
        let id = safe(id)?;
        let text = std::fs::read_to_string(self.file(id)).map_err(|e| format!("the chat could not be opened: {e}"))?;
        let pictures = self.folder.join(id);
        let chat = Chat::from_json(&text, |name| {
            // A picture's name is a number and `.png`, and nothing else.
            let fine = name.strip_suffix(".png").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
            fine.then(|| std::fs::read(pictures.join(name)).ok()).flatten()
        })?;
        if chat.id != id {
            return Err("the chat's file does not hold that chat".into());
        }
        Ok(chat)
    }

    /// Write the chat down. An empty one is not kept.
    pub fn save(&self, chat: &mut Chat) -> Result<(), String> {
        chat.unsaved = false;
        if chat.is_empty() {
            return Ok(());
        }
        let id = safe(&chat.id)?.to_string();
        let problem = |e: std::io::Error| format!("the chat could not be saved: {e}");
        std::fs::create_dir_all(&self.folder).map_err(problem)?;
        let pictures = self.folder.join(&id);
        for (index, picture) in chat.pictures().enumerate() {
            let path = pictures.join(format!("{}.png", index + 1));
            // Pictures are only ever added at the end of a chat, so one
            // already written is the same one.
            if !path.exists() {
                std::fs::create_dir_all(&pictures).map_err(problem)?;
                std::fs::write(&path, picture.png.as_slice()).map_err(problem)?;
            }
        }
        // Written beside and moved into place, so a chat is never half a file.
        let partial = self.folder.join(format!("{id}.json.part"));
        std::fs::write(&partial, chat.to_json()).map_err(problem)?;
        std::fs::rename(&partial, self.file(&id)).map_err(problem)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let id = safe(id)?;
        let _ = std::fs::remove_dir_all(self.folder.join(id));
        std::fs::remove_file(self.file(id)).map_err(|e| format!("the chat could not be deleted: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tequila-chat-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        folder
    }

    #[test]
    fn a_reply_streams_into_one_item_and_reasoning_into_another() {
        let mut chat = Chat::new();
        chat.push(Item::You { text: "Make the strap velvet".into(), pictures: Vec::new() });
        chat.think("The strap ");
        chat.think("is part 4.");
        assert!(chat.is_thinking());
        chat.say("Done: ");
        chat.say("it is velvet now.");
        assert!(!chat.is_thinking());
        chat.step(StepNews { id: "t1".into(), title: Some("Look".into()), ..Default::default() });
        chat.say("And lit.");
        let kinds: Vec<&str> = chat
            .items
            .iter()
            .map(|item| match item {
                Item::You { .. } => "you",
                Item::Said(_) => "said",
                Item::Thought { .. } => "thought",
                Item::Step(_) => "step",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["you", "thought", "said", "step", "said"]);
        assert_eq!(chat.items[1], Item::Thought { text: "The strap is part 4.".into(), seconds: match &chat.items[1] { Item::Thought { seconds, .. } => *seconds, _ => 0.0 } });
        assert_eq!(chat.items[2], Item::Said("Done: it is velvet now.".into()));
        assert_eq!(chat.title, "Make the strap velvet");
    }

    #[test]
    fn news_of_a_tool_call_finds_the_call_it_is_about() {
        let mut chat = Chat::new();
        chat.step(StepNews { id: "a".into(), title: Some("Find the grille".into()), ..Default::default() });
        chat.step(StepNews { id: "b".into(), title: Some("Change its material".into()), ..Default::default() });
        chat.step(StepNews { id: "a".into(), status: Some(Status::Done), ..Default::default() });
        chat.step(StepNews { id: "b".into(), status: Some(Status::Failed), detail: Some("no such part".into()), ..Default::default() });
        // A title that comes back empty does not blank the one it had.
        chat.step(StepNews { id: "a".into(), title: Some(" ".into()), ..Default::default() });
        let steps: Vec<(&str, Status, &str)> = chat
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Step(step) => Some((step.title.as_str(), step.status, step.detail.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(steps, [("Find the grille", Status::Done, ""), ("Change its material", Status::Failed, "no such part")]);
    }

    /// A backend numbers its tool calls from one in every conversation: the
    /// first call of this request is not the first call of the last.
    #[test]
    fn a_step_of_an_earlier_request_is_not_taken_for_a_new_one() {
        let mut chat = Chat::new();
        chat.push(Item::You { text: "Look".into(), pictures: Vec::new() });
        chat.step(StepNews { id: "call-1".into(), title: Some("Looked at the viewport".into()), status: Some(Status::Done), pictures: vec![Picture::new(vec![1])], ..Default::default() });
        chat.push(Item::You { text: "Look again".into(), pictures: Vec::new() });
        chat.step(StepNews { id: "call-1".into(), title: Some("Read the scene".into()), ..Default::default() });
        chat.step(StepNews { id: "call-1".into(), status: Some(Status::Done), ..Default::default() });
        let steps: Vec<(&str, usize)> = chat.items.iter().filter_map(|item| if let Item::Step(step) = item { Some((step.title.as_str(), step.pictures.len())) } else { None }).collect();
        assert_eq!(steps, [("Looked at the viewport", 1), ("Read the scene", 0)]);
    }

    #[test]
    fn stopping_leaves_nothing_running_or_waiting() {
        let mut chat = Chat::new();
        chat.step(StepNews { id: "a".into(), title: Some("Render".into()), ..Default::default() });
        chat.push(Item::Asked(Asked { id: 7, title: "Render to a file?".into(), detail: String::new(), answer: None }));
        chat.settle();
        assert!(matches!(&chat.items[0], Item::Step(step) if step.status == Status::Failed && !step.detail.is_empty()));
        assert!(matches!(&chat.items[1], Item::Asked(asked) if asked.answer == Some(false)));
        // Answered already: a late click changes nothing.
        assert!(!chat.answer(7, true));
    }

    #[test]
    fn a_chat_comes_back_from_its_file_with_its_pictures() {
        let store = Store::new(folder("round-trip"));
        let mut chat = Chat::new();
        chat.backend = "claude".into();
        chat.model = "sonnet".into();
        chat.session = Some("abc-123".into());
        chat.folder = Some(PathBuf::from("/projects/speaker"));
        chat.push(Item::You { text: "Match this \"fabric\"\non the grille".into(), pictures: vec![Picture::new(vec![1, 2, 3])] });
        chat.think("Looking.");
        chat.say("Here it is.");
        chat.step(StepNews {
            id: "t".into(),
            title: Some("Looked at the viewport".into()),
            status: Some(Status::Done),
            pictures: vec![Picture::new(vec![4, 5])],
            ..Default::default()
        });
        chat.push(Item::Asked(Asked { id: 1, title: "Import a file?".into(), detail: "/tmp/a.step".into(), answer: Some(true) }));
        chat.push(Item::Problem("It stopped.".into()));
        chat.push(Item::Note("Ask again.".into()));
        store.save(&mut chat).unwrap();
        assert!(!chat.unsaved);

        let back = store.load(&chat.id).unwrap();
        assert_eq!(back.title, "Match this \"fabric\"");
        assert_eq!((back.backend.as_str(), back.model.as_str(), back.session.as_deref()), ("claude", "sonnet", Some("abc-123")));
        assert_eq!(back.folder, chat.folder);
        assert_eq!(back.items.len(), chat.items.len());
        assert!(matches!(&back.items[0], Item::You { pictures, .. } if *pictures[0].png == vec![1, 2, 3]));
        assert!(matches!(&back.items[3], Item::Step(step) if *step.pictures[0].png == vec![4, 5] && step.status == Status::Done));
        assert!(matches!(&back.items[4], Item::Asked(asked) if asked.answer == Some(true)));

        // Saved again with one more picture: the first two are not rewritten.
        let first = store.folder().join(&chat.id).join("1.png");
        let written = std::fs::metadata(&first).unwrap().modified().unwrap();
        chat.push(Item::You { text: "And this".into(), pictures: vec![Picture::new(vec![6])] });
        store.save(&mut chat).unwrap();
        assert_eq!(std::fs::metadata(&first).unwrap().modified().unwrap(), written);
        assert_eq!(std::fs::read(store.folder().join(&chat.id).join("3.png")).unwrap(), [6]);
        let _ = std::fs::remove_dir_all(store.folder());
    }

    #[test]
    fn past_chats_are_listed_newest_first_and_an_empty_one_is_not_kept() {
        let store = Store::new(folder("list"));
        let mut older = Chat::new();
        older.push(Item::You { text: "Older".into(), pictures: Vec::new() });
        older.updated = 100;
        let mut newer = Chat::new();
        newer.push(Item::You { text: "Newer".into(), pictures: Vec::new() });
        newer.updated = 200;
        let mut empty = Chat::new();
        for chat in [&mut older, &mut newer, &mut empty] {
            store.save(chat).unwrap();
        }
        // Something that is not a chat, in the folder all the same.
        std::fs::write(store.folder().join("notes.json"), "{\"id\": \"../elsewhere\"}").unwrap();
        let titles: Vec<String> = store.list().into_iter().map(|s| s.title).collect();
        assert_eq!(titles, ["Newer", "Older"]);

        store.delete(&newer.id).unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(store.load("../elsewhere").is_err(), "an id is never a path");
        let _ = std::fs::remove_dir_all(store.folder());
    }

    #[test]
    fn a_long_first_message_is_cut_at_a_word_for_the_title() {
        assert_eq!(title_from("  Make it red  "), "Make it red");
        let title = title_from("Make the strap black velvet, the lenses dark glass, and light it like a studio product shot");
        assert_eq!(title, "Make the strap black velvet, the lenses dark\u{2026}");
        assert_eq!(title_from("\n\nSecond line first"), "Second line first");
    }
}
