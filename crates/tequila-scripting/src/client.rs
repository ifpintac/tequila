//! The other end of the channel: a connection to the Tequila that is open,
//! as the MCP bridge (`tequila --mcp`) and `Tequila --send` use it.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tequila_format::json::{self, Value};
use tequila_format::json_write::{self, Object};

/// One connection to Tequila's scripting channel.
pub struct Connection {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    token: String,
    next: u64,
}

impl Connection {
    /// The instance the discovery file names, or, if that one has gone (it
    /// crashed and could not hand the file on), the newest other instance
    /// still answering.
    pub fn connect(discovery: &Path) -> Result<Connection, String> {
        let mut candidates = Vec::new();
        if discovery.exists() {
            candidates.push(discovery.to_path_buf());
        }
        let others = crate::server::instances_dir(discovery);
        if let Ok(entries) = std::fs::read_dir(&others) {
            let mut records: Vec<(std::time::SystemTime, PathBuf)> = entries
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect();
            records.sort_by(|a, b| b.0.cmp(&a.0));
            candidates.extend(records.into_iter().map(|(_, p)| p));
        }
        if candidates.is_empty() {
            return Err(format!("Tequila is not running (no {})", discovery.display()));
        }
        for candidate in candidates {
            let Ok(bytes) = std::fs::read(&candidate) else { continue };
            let Ok(info) = json::parse(&bytes) else { continue };
            let (Some(port), Some(token)) = (info.get("port").and_then(Value::as_f64), info.get("token").and_then(Value::as_str)) else {
                continue;
            };
            let address = std::net::SocketAddr::from(([127, 0, 0, 1], port as u16));
            let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(5)) else { continue };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(600)));
            let Ok(writer) = stream.try_clone() else { continue };
            let mut channel = Connection { reader: BufReader::new(stream), writer, token: token.to_string(), next: 0 };
            if channel.request("ping", vec![]).is_ok() {
                return Ok(channel);
            }
        }
        Err("no running Tequila answered; is it open?".into())
    }

    pub fn request(&mut self, op: &str, fields: Vec<(&str, Value)>) -> Result<Value, String> {
        self.next += 1;
        let mut request = Object::new();
        for (key, value) in fields {
            request.set(key, value);
        }
        request.set("id", self.next as f64).set("token", self.token.as_str()).set("op", op);
        let line = one_line(&request.build());
        let ended = |e: &dyn std::fmt::Display| format!("Connection ended; do not retry edits blindly: {e}");
        writeln!(self.writer, "{line}").map_err(|e| ended(&e))?;
        self.writer.flush().map_err(|e| ended(&e))?;
        let mut answer = String::new();
        match self.reader.read_line(&mut answer) {
            Ok(0) => return Err(ended(&"Tequila closed the connection")),
            Ok(_) => {}
            Err(e) => return Err(ended(&e)),
        }
        let reply = json::parse(answer.as_bytes()).map_err(|e| ended(&e))?;
        if reply.get("id").and_then(Value::as_f64) != Some(self.next as f64) {
            return Err(ended(&"the reply was for another request"));
        }
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            return Err(reply.get("error").and_then(Value::as_str).unwrap_or("Request failed").to_string());
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }
}

/// The shared writer indents; this protocol is a line per message. Its
/// strings escape their own newlines, so the only ones left are its own.
pub fn one_line(value: &Value) -> String {
    json_write::to_string(value).split('\n').map(str::trim).collect::<Vec<_>>().join(" ").trim().to_string()
}

/// A canonical path as people write it: on Windows without the `\\?\` that
/// canonicalizing adds, which would otherwise be what the model records as
/// its file and shows in the Metadata panel.
fn plain(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

/// What [`send`] did with a file.
#[derive(Debug)]
pub enum Sent {
    /// A model sent from this file before was updated from it: its
    /// materials, labels and names kept. `node` is one of its nodes.
    Updated { node: u64, result: Value },
    /// The file came in as a new model.
    Imported(Value),
}

/// Why [`send`] could not hand a file over.
#[derive(Debug)]
pub enum SendError {
    /// No Tequila answered: start one with the file instead.
    NotRunning(String),
    /// One did, and could not take the file; this says why.
    Failed(String),
}

/// Hand `file` to the Tequila that is open, as a modelling application's
/// "send to Tequila" does: the model sent from the same file before is
/// updated from it - found by the file every import records, and kept with
/// its materials, labels and names - and otherwise the file is imported.
///
/// [`SendError::NotRunning`] when no Tequila answers, so the caller can
/// start one with the file.
pub fn send(discovery: &Path, file: &Path) -> Result<Sent, SendError> {
    let file = plain(std::fs::canonicalize(file).map_err(|e| SendError::Failed(format!("{}: {e}", file.display())))?);
    let mut channel = Connection::connect(discovery).map_err(SendError::NotRunning)?;
    let nodes = channel.request("query", vec![("what", Value::String("nodes".into()))]).map_err(SendError::Failed)?;
    let sent_before = nodes.get("nodes").map(Value::as_array).unwrap_or(&[]).iter().find_map(|node| {
        let recorded = node.get("metadata")?.get(tequila_scene::reimport::SOURCE_FILE)?.as_str()?;
        (plain(std::fs::canonicalize(recorded).ok()?) == file).then(|| node.get("id")?.as_f64())?
    });
    let path = Value::String(file.display().to_string());
    match sent_before {
        Some(node) => {
            let result = channel.request("reimport", vec![("node", Value::Number(node)), ("path", path)]).map_err(SendError::Failed)?;
            Ok(Sent::Updated { node: node as u64, result })
        }
        None => {
            let result = channel.request("import", vec![("path", path)]).map_err(SendError::Failed)?;
            // In view, as the File menu's import leaves it.
            let _ = channel.request("frame", vec![("all", Value::Bool(true))]);
            Ok(Sent::Imported(result))
        }
    }
}
