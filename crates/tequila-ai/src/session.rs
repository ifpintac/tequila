//! One conversation with the assistant, and the loop that drives it.
//!
//! The loop is the usual one — ask; the model answers with words, or asks for
//! tools; the tools' results go back; repeat until it answers with words
//! alone — split so the application stays in charge. Network waits happen on
//! a thread of their own and are polled once a frame; tool calls are handed
//! to the application to carry out on its own thread, because only it can
//! touch the document.

use crate::conversation::{Call, Message, Part, Role};
use crate::provider::Provider;
use crate::tools;
use crate::wire;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

/// How many rounds of tool use one request may take before the loop stops
/// and says so. Enough for real work — look, change twenty materials in a few
/// batches, check, adjust — and a hard ceiling on a model stuck in a loop,
/// which would otherwise spend the person's money until they noticed.
pub const MAX_STEPS: usize = 40;

/// What one reply asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub text: Vec<String>,
    /// Tool calls to carry out, then [`Session::answer`]. None means the
    /// assistant has finished.
    pub calls: Vec<Call>,
}

pub struct Session {
    pub provider: Provider,
    pub messages: Vec<Message>,
    pending: Option<Receiver<Result<wire::Reply, String>>>,
    steps: usize,
}

impl Session {
    pub fn new(provider: Provider) -> Session {
        Session { provider, messages: Vec::new(), pending: None, steps: 0 }
    }

    /// Waiting for the model.
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn steps(&self) -> usize {
        self.steps
    }

    /// Ask something new.
    pub fn ask(&mut self, text: &str) -> Result<(), String> {
        if self.busy() {
            return Err("the assistant is still working on the last request".into());
        }
        if let Some(problem) = self.provider.problem() {
            return Err(format!("{} is not set up: {problem}", self.provider.name));
        }
        self.steps = 0;
        // After a failed request the conversation already ends with the
        // person's words; add to them, since both formats expect the two
        // sides to take turns.
        match self.messages.last_mut() {
            Some(last) if last.role == Role::User => last.parts.push(Part::Text(text.to_string())),
            _ => self.messages.push(Message::user(text)),
        }
        self.send();
        Ok(())
    }

    /// Hand back the results of the calls the last turn asked for.
    pub fn answer(&mut self, results: Vec<Part>) -> Result<(), String> {
        self.messages.push(Message { role: Role::User, parts: results });
        self.steps += 1;
        if self.steps >= MAX_STEPS {
            return Err(format!(
                "stopped after {MAX_STEPS} steps without finishing — ask again to let it continue"
            ));
        }
        self.send();
        Ok(())
    }

    /// The next turn, once it has arrived.
    pub fn poll(&mut self) -> Option<Result<Turn, String>> {
        let pending = self.pending.as_ref()?;
        let result = match pending.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the request ended without an answer".into()),
        };
        self.pending = None;
        Some(result.map(|reply| {
            let mut parts: Vec<Part> = reply.text.iter().cloned().map(Part::Text).collect();
            parts.extend(reply.calls.iter().cloned().map(Part::Call));
            if !parts.is_empty() {
                self.messages.push(Message { role: Role::Assistant, parts });
            }
            Turn { text: reply.text, calls: reply.calls }
        }))
    }

    /// Stop. Whatever the model is composing is dropped when it arrives, and
    /// calls it asked for that were never carried out are answered as
    /// stopped, so the next request is still a valid conversation.
    pub fn cancel(&mut self) {
        self.pending = None;
        if let Some(last) = self.messages.last() {
            if last.role == Role::Assistant {
                let stopped: Vec<Part> = last
                    .parts
                    .iter()
                    .filter_map(|p| match p {
                        Part::Call(call) => Some(Part::Result {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            text: "Stopped by the person before this ran.".into(),
                            image: None,
                            is_error: true,
                        }),
                        _ => None,
                    })
                    .collect();
                if !stopped.is_empty() {
                    self.messages.push(Message { role: Role::User, parts: stopped });
                }
            }
        }
    }

    fn send(&mut self) {
        let (sender, receiver) = channel();
        let provider = self.provider.clone();
        let messages = self.messages.clone();
        let spawned = std::thread::Builder::new().name("assistant-request".into()).spawn(move || {
            let _ = sender.send(exchange(&provider, &messages));
        });
        match spawned {
            Ok(_) => self.pending = Some(receiver),
            Err(error) => {
                let (sender, receiver) = channel();
                let _ = sender.send(Err(format!("the request could not be started: {error}")));
                self.pending = Some(receiver);
            }
        }
    }
}

/// One round trip: the conversation out, the reply back.
fn exchange(provider: &Provider, messages: &[Message]) -> Result<wire::Reply, String> {
    let headers = provider.headers()?;
    let body = wire::request_body(provider, tools::SYSTEM, &tools::all(), messages);
    let response = crate::http::post(&provider.chat_url(), &headers, body)?;
    match wire::parse_reply(provider.kind, &response.body) {
        Ok(reply) if (200..300).contains(&response.status) => Ok(reply),
        Err(message) => Err(explain(response.status, message)),
        Ok(_) => Err(explain(response.status, format!("the service answered {}", response.status))),
    }
}

/// An error, with what usually causes that status.
fn explain(status: u16, message: String) -> String {
    let hint = match status {
        401 | 403 => " — check the API key",
        404 => " — check the address and the model name",
        429 => " — the service is rate-limiting or out of credit; wait, or check the account",
        500..=599 => " — the service had a problem; try again shortly",
        _ => "",
    };
    format!("{message}{hint}")
}

/// Ask a provider which models it offers.
pub fn fetch_models(provider: &Provider) -> Result<Vec<String>, String> {
    let response = crate::http::get(&provider.models_url(), &provider.headers()?)?;
    wire::parse_models(&response.body).map_err(|message| explain(response.status, message))
}

/// Check a provider works, with a one-line request and no tools.
pub fn test(provider: &Provider) -> Result<String, String> {
    if let Some(problem) = provider.problem() {
        return Err(problem);
    }
    let headers = provider.headers()?;
    let messages = [Message::user("Reply with exactly one word: ready")];
    let mut small = provider.clone();
    small.max_tokens = 32;
    let body = wire::request_body(&small, "You are being connected to Tequila.", &[], &messages);
    let response = crate::http::post(&provider.chat_url(), &headers, body)?;
    match wire::parse_reply(provider.kind, &response.body) {
        Ok(reply) if (200..300).contains(&response.status) => Ok(reply.text.join(" ").trim().to_string()),
        Err(message) => Err(explain(response.status, message)),
        Ok(_) => Err(explain(response.status, format!("the service answered {}", response.status))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Key, Kind};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    /// A stand-in model service on a local port, answering each request with
    /// the next canned reply and keeping what it was sent.
    fn fake_service(replies: Vec<&'static str>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for reply in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                log.lock().unwrap().push(String::from_utf8(body).unwrap());
                let mut stream = stream;
                write!(stream, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
        });
        (url, seen)
    }

    fn local(url: String) -> Provider {
        Provider {
            id: "t".into(),
            name: "Test".into(),
            kind: Kind::OpenAi,
            base_url: url,
            model: "test".into(),
            key: Key::None,
            key_header: String::new(),
            vision: true,
            max_tokens: 100,
        }
    }

    fn wait(session: &mut Session) -> Result<Turn, String> {
        for _ in 0..500 {
            if let Some(result) = session.poll() {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("no answer");
    }

    /// The whole loop against a real socket: a tool call, its result sent
    /// back, and a final answer.
    #[test]
    fn a_request_goes_round_the_loop_until_the_model_is_done() {
        let (url, seen) = fake_service(vec![
            r#"{"choices":[{"message":{"content":"Looking.","tool_calls":[{"id":"c1","type":"function","function":{"name":"tequila_scene","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"message":{"content":"The scene has 355 parts."}}]}"#,
        ]);
        let mut session = Session::new(local(url));
        session.ask("How many parts?").unwrap();
        assert!(session.busy());

        let turn = wait(&mut session).unwrap();
        assert_eq!(turn.text, ["Looking."]);
        assert_eq!(turn.calls[0].name, "tequila_scene");

        session
            .answer(vec![Part::Result { id: "c1".into(), name: "tequila_scene".into(), text: "{\"parts\":355}".into(), image: None, is_error: false }])
            .unwrap();
        let turn = wait(&mut session).unwrap();
        assert!(turn.calls.is_empty(), "finished");
        assert_eq!(turn.text, ["The scene has 355 parts."]);

        let sent = seen.lock().unwrap();
        assert!(sent[0].contains("How many parts?") && sent[0].contains("tequila_scene"));
        assert!(sent[1].contains("\"tool_call_id\": \"c1\"") || sent[1].contains("\"tool_call_id\":\"c1\""), "{}", sent[1]);
        assert!(sent[1].contains("355"));
    }

    #[test]
    fn a_stopped_request_leaves_a_conversation_that_can_continue() {
        let (url, _) = fake_service(vec![
            r#"{"choices":[{"message":{"tool_calls":[{"id":"c1","function":{"name":"tequila_scene","arguments":"{}"}}]}}]}"#,
        ]);
        let mut session = Session::new(local(url));
        session.ask("Look").unwrap();
        wait(&mut session).unwrap();
        session.cancel();
        let last = session.messages.last().unwrap();
        assert_eq!(last.role, Role::User);
        assert!(matches!(&last.parts[0], Part::Result { is_error: true, .. }), "the unrun call is answered");
    }

    #[test]
    fn nothing_listening_is_explained() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let mut session = Session::new(local(format!("http://127.0.0.1:{port}/v1")));
        session.ask("hello").unwrap();
        let error = wait(&mut session).unwrap_err();
        assert!(error.contains("127.0.0.1"), "{error}");
    }

    #[test]
    fn a_provider_that_is_not_set_up_is_refused_before_sending() {
        let mut provider = local("http://127.0.0.1:1/v1".into());
        provider.model.clear();
        assert!(Session::new(provider).ask("hi").unwrap_err().contains("model"));
    }
}
