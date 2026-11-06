//! The two wire formats: writing a conversation as a request body, and
//! reading the model's reply.
//!
//! Hand-written against the documented shapes rather than through a client
//! library for each service: two formats, a few hundred lines, and no
//! dependency that moves under us when a vendor ships a new SDK.

use crate::conversation::{with_recent_images, Call, Message, Part, Role, IMAGES_KEPT};
use crate::provider::{Kind, Provider};
use crate::tools::Tool;
use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};

/// What came back from one request.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reply {
    pub text: Vec<String>,
    pub calls: Vec<Call>,
    /// What the model reasoned before answering, where the service sends
    /// it. Shown to the person; not sent back, since neither format needs
    /// it again.
    pub reasoning: String,
}

/// How a request is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Ask for the reply as a stream of events, as it is written.
    pub stream: bool,
    /// How many of the conversation's images go with it: the newest.
    pub images: usize,
}

impl Options {
    /// The whole reply at once, with as many pictures as the model can use.
    pub fn plain(provider: &Provider) -> Options {
        Options { stream: false, images: if provider.vision { IMAGES_KEPT } else { 0 } }
    }
}

pub fn request_body(provider: &Provider, system: &str, tools: &[Tool], messages: &[Message]) -> String {
    request_body_with(provider, system, tools, messages, Options::plain(provider))
}

pub fn request_body_with(provider: &Provider, system: &str, tools: &[Tool], messages: &[Message], options: Options) -> String {
    let messages = with_recent_images(messages, options.images);
    let mut body = match provider.kind {
        Kind::Anthropic => anthropic_body(provider, system, tools, &messages),
        Kind::OpenAi => openai_body(provider, system, tools, &messages),
    };
    if options.stream {
        if let Value::Object(fields) = &mut body {
            fields.insert("stream".into(), Value::Bool(true));
        }
    }
    json_write::to_string(&body)
}

pub fn parse_reply(kind: Kind, body: &str) -> Result<Reply, String> {
    let value = json::parse(body.as_bytes())
        .map_err(|_| format!("the reply was not JSON: {}", excerpt(body)))?;
    if let Some(message) = error_message(&value) {
        return Err(message);
    }
    match kind {
        Kind::Anthropic => anthropic_reply(&value),
        Kind::OpenAi => openai_reply(&value),
    }
}

/// The message out of an error body, whichever format it is in.
pub fn error_message(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    Some(match error {
        Value::String(text) => text.clone(),
        other => other
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| json_write::to_string(other)),
    })
}

fn excerpt(text: &str) -> String {
    let trimmed: String = text.trim().chars().take(300).collect();
    if trimmed.is_empty() { "(nothing)".into() } else { trimmed }
}

fn object(pairs: Vec<(&str, Value)>) -> Value {
    let mut out = Object::new();
    for (key, value) in pairs {
        out.set(key, value);
    }
    out.build()
}

fn string(text: &str) -> Value {
    Value::String(text.to_string())
}

// ---- Anthropic ------------------------------------------------------------------

fn anthropic_body(provider: &Provider, system: &str, tools: &[Tool], messages: &[Message]) -> Value {
    let tools = tools
        .iter()
        .map(|t| object(vec![("name", string(&t.name)), ("description", string(&t.description)), ("input_schema", t.schema.clone())]))
        .collect();
    let messages = messages
        .iter()
        .map(|m| {
            let role = match m.role { Role::User => "user", Role::Assistant => "assistant" };
            let content = m.parts.iter().filter_map(anthropic_block).collect();
            object(vec![("role", string(role)), ("content", Value::Array(content))])
        })
        .collect();
    object(vec![
        ("model", string(&provider.model)),
        ("max_tokens", Value::Number(provider.max_tokens as f64)),
        ("system", string(system)),
        ("tools", Value::Array(tools)),
        ("messages", Value::Array(messages)),
    ])
}

fn anthropic_image(png: &str) -> Value {
    object(vec![
        ("type", string("image")),
        ("source", object(vec![("type", string("base64")), ("media_type", string("image/png")), ("data", string(png))])),
    ])
}

fn anthropic_block(part: &Part) -> Option<Value> {
    Some(match part {
        Part::Text(text) if text.is_empty() => return None,
        Part::Text(text) => object(vec![("type", string("text")), ("text", string(text))]),
        Part::Image(png) => anthropic_image(png),
        Part::Call(call) => object(vec![
            ("type", string("tool_use")),
            ("id", string(&call.id)),
            ("name", string(&call.name)),
            ("input", call.arguments.clone().unwrap_or_else(|_| object(vec![]))),
        ]),
        Part::Result { id, text, image, is_error, .. } => {
            let mut content = vec![object(vec![("type", string("text")), ("text", string(text))])];
            if let Some(png) = image {
                content.push(anthropic_image(png));
            }
            object(vec![
                ("type", string("tool_result")),
                ("tool_use_id", string(id)),
                ("content", Value::Array(content)),
                ("is_error", Value::Bool(*is_error)),
            ])
        }
    })
}

fn anthropic_reply(value: &Value) -> Result<Reply, String> {
    let content = value
        .get("content")
        .ok_or_else(|| format!("the reply had no content: {}", excerpt(&json_write::to_string(value))))?;
    let mut reply = Reply::default();
    for block in content.as_array() {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    reply.text.push(text.to_string());
                }
            }
            Some("thinking") => {
                if let Some(text) = block.get("thinking").and_then(|t| t.as_str()) {
                    reply.reasoning.push_str(text);
                }
            }
            Some("tool_use") => reply.calls.push(Call {
                id: block.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                name: block.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                arguments: Ok(block.get("input").cloned().unwrap_or_else(|| object(vec![]))),
            }),
            _ => {}
        }
    }
    Ok(reply)
}

// ---- OpenAI-compatible -----------------------------------------------------------

fn data_url(png: &str) -> String {
    format!("data:image/png;base64,{png}")
}

fn openai_body(provider: &Provider, system: &str, tools: &[Tool], messages: &[Message]) -> Value {
    let tools = tools
        .iter()
        .map(|t| {
            object(vec![
                ("type", string("function")),
                ("function", object(vec![("name", string(&t.name)), ("description", string(&t.description)), ("parameters", t.schema.clone())])),
            ])
        })
        .collect();
    let mut out = vec![object(vec![("role", string("system")), ("content", string(system))])];
    for message in messages {
        match message.role {
            Role::Assistant => {
                let text: Vec<&str> = message.parts.iter().filter_map(|p| match p { Part::Text(t) => Some(t.as_str()), _ => None }).collect();
                let calls: Vec<Value> = message
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Call(call) => Some(object(vec![
                            ("id", string(&call.id)),
                            ("type", string("function")),
                            ("function", object(vec![
                                ("name", string(&call.name)),
                                ("arguments", string(&call.arguments.as_ref().map(one_line).unwrap_or_else(|_| "{}".into()))),
                            ])),
                        ])),
                        _ => None,
                    })
                    .collect();
                let mut fields = vec![
                    ("role", string("assistant")),
                    ("content", if text.is_empty() { Value::Null } else { string(&text.join("\n")) }),
                ];
                if !calls.is_empty() {
                    fields.push(("tool_calls", Value::Array(calls)));
                }
                out.push(object(fields));
            }
            Role::User => {
                // Tool results are their own messages in this format, and may
                // hold only text; a screenshot follows them as a user message,
                // which is where images are allowed.
                let mut images = Vec::new();
                let mut content = Vec::new();
                for part in &message.parts {
                    match part {
                        Part::Result { id, name, text, image, is_error } => {
                            let mut text = if *is_error { format!("Error: {text}") } else { text.clone() };
                            if image.is_some() {
                                text.push_str(" (the image follows)");
                            }
                            out.push(object(vec![("role", string("tool")), ("tool_call_id", string(id)), ("content", string(&text))]));
                            if let Some(png) = image {
                                images.push((name.clone(), png.clone()));
                            }
                        }
                        Part::Text(text) if !text.is_empty() => content.push(object(vec![("type", string("text")), ("text", string(text))])),
                        Part::Image(png) => content.push(object(vec![
                            ("type", string("image_url")),
                            ("image_url", object(vec![("url", string(&data_url(png)))])),
                        ])),
                        _ => {}
                    }
                }
                for (name, png) in images {
                    content.push(object(vec![("type", string("text")), ("text", string(&format!("Image from {name}:")))]));
                    content.push(object(vec![
                        ("type", string("image_url")),
                        ("image_url", object(vec![("url", string(&data_url(&png)))])),
                    ]));
                }
                if !content.is_empty() {
                    // Plain text as a string, which every compatible server
                    // accepts; the array form only when there is a picture.
                    let only_text = content.iter().all(|c| c.get("type").and_then(|t| t.as_str()) == Some("text"));
                    let value = if only_text {
                        string(&content.iter().filter_map(|c| c.get("text").and_then(|t| t.as_str())).collect::<Vec<_>>().join("\n"))
                    } else {
                        Value::Array(content)
                    };
                    out.push(object(vec![("role", string("user")), ("content", value)]));
                }
            }
        }
    }
    object(vec![
        ("model", string(&provider.model)),
        ("max_tokens", Value::Number(provider.max_tokens as f64)),
        ("tools", Value::Array(tools)),
        ("messages", Value::Array(out)),
    ])
}

fn one_line(value: &Value) -> String {
    json_write::to_string(value).split('\n').map(str::trim).collect::<Vec<_>>().join(" ")
}

fn openai_reply(value: &Value) -> Result<Reply, String> {
    let message = value
        .get("choices")
        .and_then(|c| c.as_array().first())
        .and_then(|c| c.get("message"))
        .ok_or_else(|| format!("the reply had no message: {}", excerpt(&json_write::to_string(value))))?;
    let mut reply = Reply { reasoning: reasoning_of(message).unwrap_or_default().to_string(), ..Reply::default() };
    if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
        let (thought, text) = split_thinking(text);
        reply.reasoning.push_str(thought.unwrap_or_default());
        if !text.trim().is_empty() {
            reply.text.push(text.to_string());
        }
    }
    if let Some(calls) = message.get("tool_calls") {
        for (i, call) in calls.as_array().iter().enumerate() {
            let function = call.get("function");
            let arguments = function
                .and_then(|f| f.get("arguments"))
                .map(|a| match a {
                    // Normally a JSON string; some servers send the object.
                    Value::String(text) => parse_arguments(text),
                    other => Ok(other.clone()),
                })
                .unwrap_or_else(|| Ok(object(vec![])));
            reply.calls.push(Call {
                id: call
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    // Some local servers omit ids; the tool message still
                    // needs one to answer to.
                    .unwrap_or_else(|| format!("call_{i}")),
                name: function.and_then(|f| f.get("name")).and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                arguments,
            });
        }
    }
    Ok(reply)
}

/// The reasoning an OpenAI-compatible server puts beside the answer. The
/// field is not part of OpenAI's own format, and servers name it
/// differently: Ollama and LM Studio `reasoning`, vLLM and DeepSeek
/// `reasoning_content`.
fn reasoning_of(message: &Value) -> Option<&str> {
    ["reasoning", "reasoning_content"].iter().filter_map(|key| message.get(key).and_then(|r| r.as_str())).find(|r| !r.is_empty())
}

/// Reasoning written into the answer itself, as some local servers send
/// it: `<think>…</think>` before the words. The reasoning (all of it while
/// the closing tag has not come yet), then the rest.
pub fn split_thinking(text: &str) -> (Option<&str>, &str) {
    let Some(after) = text.trim_start().strip_prefix("<think>") else {
        return (None, text);
    };
    match after.find("</think>") {
        Some(end) => (Some(&after[..end]), after[end + "</think>".len()..].trim_start()),
        None => (Some(after), ""),
    }
}

fn parse_arguments(text: &str) -> Result<Value, String> {
    if text.trim().is_empty() {
        return Ok(object(vec![]));
    }
    json::parse(text.as_bytes()).map_err(|_| format!("the arguments were not valid JSON: {}", excerpt(text)))
}

// ---- Streams ---------------------------------------------------------------------

/// What a streaming reply is doing at the moment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    /// Nothing has come yet: the server is reading the request.
    #[default]
    Waiting,
    Thinking,
    Writing,
    /// Writing a call to this tool.
    Calling(String),
}

/// A tool call as it arrives, a piece at a time.
#[derive(Default)]
struct PartialCall {
    /// The OpenAI format numbers calls, so their pieces can interleave.
    index: Option<i64>,
    id: String,
    name: String,
    arguments: String,
    /// An Anthropic call's input as its start gave it, for when no pieces
    /// follow.
    input: Option<Value>,
}

/// What an Anthropic content block's pieces add to.
enum Block {
    Text(usize),
    Call(usize),
    Thinking,
    Other,
}

/// A reply put together from its stream of events, in either format.
pub struct Assembler {
    kind: Kind,
    texts: Vec<String>,
    reasoning: String,
    calls: Vec<PartialCall>,
    blocks: Vec<(i64, Block)>,
    activity: Activity,
    /// The end was announced: `[DONE]`, a finish reason, `message_stop`.
    finished: bool,
    events: usize,
}

impl Assembler {
    pub fn new(kind: Kind) -> Assembler {
        Assembler {
            kind,
            texts: Vec::new(),
            reasoning: String::new(),
            calls: Vec::new(),
            blocks: Vec::new(),
            activity: Activity::Waiting,
            finished: false,
            events: 0,
        }
    }

    /// One event of the stream. An error the service reports inside it
    /// ends the reply with that error.
    pub fn event(&mut self, event: &crate::sse::Event) -> Result<(), String> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(());
        }
        self.events += 1;
        if data == "[DONE]" {
            self.finished = true;
            return Ok(());
        }
        let value = json::parse(data.as_bytes()).map_err(|_| format!("the stream held something that was not JSON: {}", excerpt(data)))?;
        if let Some(message) = error_message(&value) {
            return Err(message);
        }
        match self.kind {
            Kind::OpenAi => self.openai_event(&value),
            Kind::Anthropic => self.anthropic_event(&value),
        }
        Ok(())
    }

    fn openai_event(&mut self, value: &Value) {
        // A chunk with no choices carries only the token counts.
        let Some(choice) = value.get("choices").and_then(|c| c.as_array().first()) else { return };
        if choice.get("finish_reason").and_then(|r| r.as_str()).is_some() {
            self.finished = true;
        }
        let Some(delta) = choice.get("delta").or_else(|| choice.get("message")) else { return };
        if let Some(piece) = reasoning_of(delta) {
            self.reasoning.push_str(piece);
            self.activity = Activity::Thinking;
        }
        if let Some(piece) = delta.get("content").and_then(|c| c.as_str()).filter(|p| !p.is_empty()) {
            if self.texts.is_empty() {
                self.texts.push(String::new());
            }
            self.texts[0].push_str(piece);
            let (thought, rest) = split_thinking(&self.texts[0]);
            self.activity = if thought.is_some() && rest.is_empty() { Activity::Thinking } else { Activity::Writing };
        }
        for call in delta.get("tool_calls").map(|c| c.as_array()).unwrap_or(&[]) {
            let index = call.get("index").and_then(|i| i.as_f64()).map(|i| i as i64);
            let id = call.get("id").and_then(|i| i.as_str()).filter(|i| !i.is_empty());
            let found = match (index, id) {
                (Some(index), _) => self.calls.iter().position(|c| c.index == Some(index)),
                (None, Some(id)) => self.calls.iter().position(|c| c.id == id),
                // Neither: more of the call being written.
                (None, None) => self.calls.len().checked_sub(1),
            };
            let slot = found.unwrap_or_else(|| {
                self.calls.push(PartialCall { index, ..PartialCall::default() });
                self.calls.len() - 1
            });
            let partial = &mut self.calls[slot];
            if let (Some(id), true) = (id, partial.id.is_empty()) {
                partial.id = id.to_string();
            }
            let function = call.get("function");
            if let Some(name) = function.and_then(|f| f.get("name")).and_then(|n| n.as_str()).filter(|n| !n.is_empty()) {
                // Sent once, normally; some servers repeat it with each piece.
                if partial.name != name {
                    partial.name.push_str(name);
                }
            }
            match function.and_then(|f| f.get("arguments")) {
                Some(Value::String(piece)) => partial.arguments.push_str(piece),
                Some(Value::Null) | None => {}
                Some(whole) => partial.arguments = json_write::to_string(whole),
            }
            self.activity = Activity::Calling(partial.name.clone());
        }
    }

    fn anthropic_event(&mut self, value: &Value) {
        let index = value.get("index").and_then(|i| i.as_f64()).map(|i| i as i64).unwrap_or(-1);
        match value.get("type").and_then(|t| t.as_str()).unwrap_or_default() {
            "content_block_start" => {
                let block = value.get("content_block");
                let text_of = |key: &str| block.and_then(|b| b.get(key)).and_then(|t| t.as_str()).unwrap_or_default().to_string();
                let slot = match block.and_then(|b| b.get("type")).and_then(|t| t.as_str()) {
                    Some("text") => {
                        self.texts.push(text_of("text"));
                        self.activity = Activity::Writing;
                        Block::Text(self.texts.len() - 1)
                    }
                    Some("tool_use") => {
                        let name = text_of("name");
                        self.activity = Activity::Calling(name.clone());
                        self.calls.push(PartialCall {
                            id: text_of("id"),
                            name,
                            input: block.and_then(|b| b.get("input")).cloned(),
                            ..PartialCall::default()
                        });
                        Block::Call(self.calls.len() - 1)
                    }
                    Some("thinking") => {
                        self.reasoning.push_str(&text_of("thinking"));
                        self.activity = Activity::Thinking;
                        Block::Thinking
                    }
                    _ => Block::Other,
                };
                self.blocks.push((index, slot));
            }
            "content_block_delta" => {
                let Some(delta) = value.get("delta") else { return };
                let piece = |key: &str| delta.get(key).and_then(|t| t.as_str()).unwrap_or_default();
                match self.blocks.iter().rev().find(|(i, _)| *i == index).map(|(_, b)| b) {
                    Some(Block::Text(slot)) => {
                        self.texts[*slot].push_str(piece("text"));
                        self.activity = Activity::Writing;
                    }
                    Some(Block::Call(slot)) => {
                        let call = &mut self.calls[*slot];
                        call.arguments.push_str(piece("partial_json"));
                        self.activity = Activity::Calling(call.name.clone());
                    }
                    Some(Block::Thinking) => {
                        self.reasoning.push_str(piece("thinking"));
                        self.activity = Activity::Thinking;
                    }
                    Some(Block::Other) | None => {}
                }
            }
            "message_delta" => {
                if value.get("delta").and_then(|d| d.get("stop_reason")).and_then(|r| r.as_str()).is_some() {
                    self.finished = true;
                }
            }
            "message_stop" => self.finished = true,
            _ => {}
        }
    }

    pub fn activity(&self) -> &Activity {
        &self.activity
    }

    /// The reasoning so far, including any written into the answer.
    pub fn reasoning(&self) -> String {
        let written = match self.kind {
            Kind::OpenAi => self.texts.first().and_then(|t| split_thinking(t).0),
            Kind::Anthropic => None,
        };
        match written {
            Some(thought) if self.reasoning.is_empty() => thought.to_string(),
            Some(thought) => format!("{}\n{thought}", self.reasoning),
            None => self.reasoning.clone(),
        }
    }

    /// The answer's words so far.
    pub fn text(&self) -> String {
        self.texts
            .iter()
            .enumerate()
            .map(|(i, t)| if i == 0 && self.kind == Kind::OpenAi { split_thinking(t).1 } else { t.as_str() })
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// How much has been written, in characters: words, reasoning and the
    /// tools' arguments.
    pub fn written(&self) -> usize {
        self.texts.iter().map(|t| t.chars().count()).sum::<usize>()
            + self.reasoning.chars().count()
            + self.calls.iter().map(|c| c.arguments.chars().count() + c.name.len()).sum::<usize>()
    }

    /// The whole reply, once the stream has ended.
    pub fn finish(self) -> Result<Reply, String> {
        if self.events == 0 {
            return Err("the service ended the stream without replying".into());
        }
        // The OpenAI format's end is announced by convention only, and a
        // few servers just close the connection after the last piece; an
        // Anthropic stream always ends with `message_stop`.
        let unfinished_call = self.calls.iter().any(|c| parse_arguments(&c.arguments).is_err());
        if !self.finished && (self.kind == Kind::Anthropic || unfinished_call) {
            return Err("the reply broke off part-way, before the service said it had finished".into());
        }
        let mut reply = Reply { reasoning: self.reasoning(), ..Reply::default() };
        for (i, text) in self.texts.iter().enumerate() {
            let text = if i == 0 && self.kind == Kind::OpenAi { split_thinking(text).1 } else { text.as_str() };
            if !text.trim().is_empty() {
                reply.text.push(text.to_string());
            }
        }
        for (i, call) in self.calls.into_iter().enumerate() {
            let arguments = match (call.arguments.trim().is_empty(), call.input) {
                (true, Some(input)) => Ok(input),
                _ => parse_arguments(&call.arguments),
            };
            reply.calls.push(Call {
                // Some local servers send no ids; the result still needs one
                // to answer to.
                id: if call.id.is_empty() { format!("call_{i}") } else { call.id },
                name: call.name,
                arguments,
            });
        }
        Ok(reply)
    }
}

/// The model ids a `/models` endpoint lists, in either format.
pub fn parse_models(body: &str) -> Result<Vec<String>, String> {
    let value = json::parse(body.as_bytes()).map_err(|_| format!("the reply was not JSON: {}", excerpt(body)))?;
    if let Some(message) = error_message(&value) {
        return Err(message);
    }
    let mut ids: Vec<String> = value
        .get("data")
        .or_else(|| value.get("models"))
        .map(|list| list.as_array().iter().filter_map(|m| m.get("id").or_else(|| m.get("name")).and_then(|i| i.as_str()).map(str::to_string)).collect())
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::Provider;
    use crate::tools;

    fn provider(kind: Kind) -> Provider {
        Provider { kind, model: "test-model".into(), ..Provider::presets().remove(0) }
    }

    fn conversation() -> Vec<Message> {
        vec![
            Message::user("Make the strap velvet"),
            Message {
                role: Role::Assistant,
                parts: vec![
                    Part::Text("Looking first.".into()),
                    Part::Call(Call { id: "c1".into(), name: "tequila_screenshot".into(), arguments: Ok(json::parse(b"{\"viewport\":true}").unwrap()) }),
                ],
            },
            Message {
                role: Role::User,
                parts: vec![Part::Result { id: "c1".into(), name: "tequila_screenshot".into(), text: "captured".into(), image: Some("UE5H".into()), is_error: false }],
            },
        ]
    }

    fn body(kind: Kind) -> Value {
        let text = request_body(&provider(kind), "You drive Tequila.", &tools::all(), &conversation());
        json::parse(text.as_bytes()).expect("the body is JSON")
    }

    #[test]
    fn an_anthropic_body_has_the_documented_shape() {
        let body = body(Kind::Anthropic);
        assert_eq!(body.get("model").and_then(|m| m.as_str()), Some("test-model"));
        assert_eq!(body.get("system").and_then(|s| s.as_str()), Some("You drive Tequila."));
        assert!(body.get("max_tokens").is_some());
        let tool = &body.get("tools").unwrap().as_array()[0];
        assert!(tool.get("input_schema").is_some());
        let messages = body.get("messages").unwrap().as_array();
        let call = &messages[1].get("content").unwrap().as_array()[1];
        assert_eq!(call.get("type").and_then(|t| t.as_str()), Some("tool_use"));
        let result = &messages[2].get("content").unwrap().as_array()[0];
        assert_eq!(result.get("type").and_then(|t| t.as_str()), Some("tool_result"));
        assert_eq!(result.get("tool_use_id").and_then(|t| t.as_str()), Some("c1"));
        let inner = result.get("content").unwrap().as_array();
        assert_eq!(inner[1].get("type").and_then(|t| t.as_str()), Some("image"), "the screenshot travels inside the result");
    }

    /// Tool results in the OpenAI format are text-only messages, so the
    /// screenshot has to follow as a user message — or the model never sees it.
    #[test]
    fn an_openai_body_carries_the_screenshot_after_the_tool_message() {
        let body = body(Kind::OpenAi);
        let messages = body.get("messages").unwrap().as_array();
        let roles: Vec<&str> = messages.iter().map(|m| m.get("role").and_then(|r| r.as_str()).unwrap()).collect();
        assert_eq!(roles, ["system", "user", "assistant", "tool", "user"]);
        let call = &messages[2].get("tool_calls").unwrap().as_array()[0];
        let arguments = call.get("function").unwrap().get("arguments").unwrap().as_str().unwrap();
        assert!(json::parse(arguments.as_bytes()).is_ok(), "arguments are a JSON string: {arguments}");
        assert_eq!(messages[3].get("tool_call_id").and_then(|t| t.as_str()), Some("c1"));
        let picture = &messages[4].get("content").unwrap().as_array()[1];
        assert!(picture.get("image_url").unwrap().get("url").unwrap().as_str().unwrap().starts_with("data:image/png;base64,"));
        let tool = &body.get("tools").unwrap().as_array()[0];
        assert!(tool.get("function").unwrap().get("parameters").is_some());
    }

    /// A model that cannot see is not sent pictures it would refuse.
    #[test]
    fn a_blind_model_gets_no_images() {
        let mut blind = provider(Kind::OpenAi);
        blind.vision = false;
        let text = request_body(&blind, "s", &tools::all(), &conversation());
        assert!(!text.contains("image_url"), "{text}");
    }

    #[test]
    fn replies_are_read_in_both_formats() {
        let anthropic = r#"{"content":[{"type":"text","text":"On it."},{"type":"tool_use","id":"toolu_1","name":"tequila_nodes","input":{"filter":"strap"}}],"stop_reason":"tool_use"}"#;
        let reply = parse_reply(Kind::Anthropic, anthropic).unwrap();
        assert_eq!(reply.text, ["On it."]);
        assert_eq!(reply.calls[0].name, "tequila_nodes");
        assert_eq!(reply.calls[0].arguments.as_ref().unwrap().get("filter").and_then(|f| f.as_str()), Some("strap"));

        let openai = r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"call_9","type":"function","function":{"name":"tequila_look","arguments":"{\"view\":\"front\"}"}}]},"finish_reason":"tool_calls"}]}"#;
        let reply = parse_reply(Kind::OpenAi, openai).unwrap();
        assert!(reply.text.is_empty());
        assert_eq!(reply.calls[0].id, "call_9");
        assert_eq!(reply.calls[0].arguments.as_ref().unwrap().get("view").and_then(|f| f.as_str()), Some("front"));
    }

    #[test]
    fn errors_say_what_the_service_said() {
        let error = parse_reply(Kind::Anthropic, r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#).unwrap_err();
        assert_eq!(error, "invalid x-api-key");
        let error = parse_reply(Kind::OpenAi, r#"{"error":{"message":"The model does not exist"}}"#).unwrap_err();
        assert!(error.contains("does not exist"));
        assert!(parse_reply(Kind::OpenAi, "<html>Bad gateway</html>").unwrap_err().contains("Bad gateway"));
    }

    #[test]
    fn bad_arguments_are_reported_not_guessed() {
        let openai = r#"{"choices":[{"message":{"tool_calls":[{"id":"c","function":{"name":"tequila_apply","arguments":"{not json"}}]}}]}"#;
        let reply = parse_reply(Kind::OpenAi, openai).unwrap();
        assert!(reply.calls[0].arguments.is_err());
    }

    #[test]
    fn model_lists_are_read_in_both_formats() {
        assert_eq!(parse_models(r#"{"data":[{"id":"b"},{"id":"a"}]}"#).unwrap(), ["a", "b"]);
        assert_eq!(parse_models(r#"{"models":[{"name":"llama3"}]}"#).unwrap(), ["llama3"]);
    }

    #[test]
    fn a_streamed_request_says_so_in_both_formats() {
        for kind in Kind::ALL {
            let options = Options { stream: true, images: 1 };
            let text = request_body_with(&provider(kind), "s", &tools::all(), &conversation(), options);
            let body = json::parse(text.as_bytes()).unwrap();
            assert_eq!(body.get("stream").and_then(|s| s.as_bool()), Some(true), "{kind:?}");
            assert!(!request_body(&provider(kind), "s", &[], &conversation()).contains("\"stream\""));
        }
    }

    /// Reasoning written into the answer, whole or still coming, is taken
    /// out of it; an answer without any is left alone.
    #[test]
    fn thinking_written_into_the_answer_is_taken_out() {
        assert_eq!(split_thinking("<think>a plan</think>\n\nThe answer."), (Some("a plan"), "The answer."));
        assert_eq!(split_thinking("  <think>still going"), (Some("still going"), ""));
        assert_eq!(split_thinking("Plain words."), (None, "Plain words."));
        let reply = parse_reply(Kind::OpenAi, r#"{"choices":[{"message":{"content":"<think>hm</think>Done.","reasoning_content":"first "}}]}"#).unwrap();
        assert_eq!(reply.text, ["Done."]);
        assert_eq!(reply.reasoning, "first hm");
    }

    /// The live view of a stream: what it is doing, its words and its
    /// reasoning so far, and a count that grows.
    #[test]
    fn a_stream_shows_what_it_is_doing_as_it_goes() {
        let event = |data: &str| crate::sse::Event { name: String::new(), data: data.to_string() };
        let mut stream = Assembler::new(Kind::OpenAi);
        assert_eq!(stream.activity(), &Activity::Waiting);
        stream.event(&event(r#"{"choices":[{"delta":{"reasoning":"Look first."}}]}"#)).unwrap();
        assert_eq!(stream.activity(), &Activity::Thinking);
        stream.event(&event(r#"{"choices":[{"delta":{"content":"Checking."}}]}"#)).unwrap();
        assert_eq!((stream.activity(), stream.text().as_str()), (&Activity::Writing, "Checking."));
        stream.event(&event(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"tequila_scene","arguments":"{}"}}]}}]}"#)).unwrap();
        assert_eq!(stream.activity(), &Activity::Calling("tequila_scene".into()));
        assert!(stream.written() > 20);
        // Ended with no word that it had finished, a call half-written: cut off.
        let mut cut = Assembler::new(Kind::OpenAi);
        cut.event(&event(r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"tequila_apply","arguments":"{\"comm"}}]}}]}"#)).unwrap();
        assert!(cut.finish().unwrap_err().contains("broke off"));
        // An Anthropic stream that never said it was done is cut off too.
        let mut anthropic = Assembler::new(Kind::Anthropic);
        anthropic.event(&event(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Hal"}}"#)).unwrap();
        assert!(anthropic.finish().is_err());
    }
}
