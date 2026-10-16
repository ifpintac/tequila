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
#[derive(Clone, Debug, PartialEq)]
pub struct Reply {
    pub text: Vec<String>,
    pub calls: Vec<Call>,
}

pub fn request_body(provider: &Provider, system: &str, tools: &[Tool], messages: &[Message]) -> String {
    let messages = with_recent_images(messages, if provider.vision { IMAGES_KEPT } else { 0 });
    let body = match provider.kind {
        Kind::Anthropic => anthropic_body(provider, system, tools, &messages),
        Kind::OpenAi => openai_body(provider, system, tools, &messages),
    };
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
    let mut reply = Reply { text: Vec::new(), calls: Vec::new() };
    for block in content.as_array() {
        match block.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                    reply.text.push(text.to_string());
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
    let mut reply = Reply { text: Vec::new(), calls: Vec::new() };
    if let Some(text) = message.get("content").and_then(|c| c.as_str()) {
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
                    Value::String(text) if text.trim().is_empty() => Ok(object(vec![])),
                    Value::String(text) => json::parse(text.as_bytes()).map_err(|_| format!("the arguments were not valid JSON: {}", excerpt(text))),
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
}
