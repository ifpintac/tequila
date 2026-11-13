//! What Tequila's own loop makes of a model's replies, kept as files: for
//! each, the calls it would carry out and the words it would show.
//!
//! ```text
//! cargo run -p tequila-ai --example replies -- replies/*.json replies/*.sse
//! ```
//!
//! A `.json` file is a whole reply in the OpenAI format, as
//! `/v1/chat/completions` answers; anything else is read as the stream of
//! events the same address sends when asked to stream. It is how to count,
//! for a model that writes its tool calls into its words, how many of its
//! replies end as a call and how many as markup shown to the person.

use tequila_ai::provider::Kind;
use tequila_ai::{sse, tools, wire};

fn read(path: &str, tools: &[tools::Tool]) -> Result<(wire::Reply, wire::Reply), String> {
    let body = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let reply = if path.ends_with(".json") {
        wire::parse_reply(Kind::OpenAi, &body)?
    } else {
        let mut parser = sse::Parser::new();
        let mut events = Vec::new();
        parser.feed(body.as_bytes(), &mut events);
        parser.finish(&mut events);
        let mut stream = wire::Assembler::new(Kind::OpenAi);
        for event in &events {
            stream.event(event)?;
        }
        stream.finish()?
    };
    Ok((reply.clone(), wire::with_written_calls(reply, tools)))
}

fn main() {
    let tools = tools::all();
    let (mut calls, mut written, mut markup, mut words) = (0, 0, 0, 0);
    for path in std::env::args().skip(1) {
        let (as_sent, as_read) = match read(&path, &tools) {
            Ok(both) => both,
            Err(error) => {
                println!("{path}: {error}");
                continue;
            }
        };
        let shown = as_read.text.join(" ");
        let kind = if !as_sent.calls.is_empty() {
            calls += 1;
            "call"
        } else if !as_read.calls.is_empty() {
            written += 1;
            "written call, read"
        } else if shown.contains("<function=") || shown.contains("tool_call>") {
            markup += 1;
            "MARKUP SHOWN"
        } else {
            words += 1;
            "words"
        };
        let named: Vec<&str> = as_read.calls.iter().map(|call| call.name.as_str()).collect();
        let shown: String = shown.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(70).collect();
        println!("{path}: {kind} [{}] {shown}", named.join(", "));
    }
    println!("{calls} sent as calls, {written} written into the words and read as calls, {markup} left as markup, {words} only words");
}
