//! A render node: `tequila --worker`.
//!
//! It listens for dispatchers holding the farm's token, takes one job at a
//! time - a render uses every core the machine has - and sends back the
//! picture. Each job is unpacked into a folder of its own under the
//! worker's scratch folder and removed when it is done; nothing a job sends
//! is written anywhere else (`crate::bundle`).
//!
//! While a render runs, the worker says so every few seconds, so a
//! dispatcher can tell a long render from a machine that has gone.

use crate::bundle;
use crate::job::{header, JobSpec};
use crate::wire::{Wire, WireError};
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use tequila_scene::SceneDocument;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How a worker renders: the job, its project (paths pointing into the
/// job's folder), and that folder to write into. The picture's bytes, or
/// why not.
pub type Render = dyn Fn(&JobSpec, SceneDocument, &Path) -> Result<Vec<u8>, String> + Send + Sync;

/// How often a worker says it is still rendering.
pub const HEARTBEAT: Duration = Duration::from_secs(3);

pub struct Worker {
    pub token: Vec<u8>,
    /// Shown in dispatchers' monitors.
    pub name: String,
    /// Where jobs are unpacked, a folder each.
    pub scratch: PathBuf,
    pub render: Arc<Render>,
    /// Held while rendering: one job at a time on this machine. Shared with
    /// the render thread, which can outlive the connection that started it:
    /// a job left rendering after its dispatcher went still keeps the next
    /// one waiting.
    busy: Arc<Mutex<()>>,
    jobs: AtomicU64,
}

impl Worker {
    pub fn new(token: impl Into<Vec<u8>>, name: impl Into<String>, scratch: PathBuf, render: Arc<Render>) -> Worker {
        Worker { token: token.into(), name: name.into(), scratch, render, busy: Arc::new(Mutex::new(())), jobs: AtomicU64::new(0) }
    }
}

/// Serve on `listener` until `stop` is set. A thread per dispatcher.
pub fn serve(listener: TcpListener, worker: Arc<Worker>, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, peer)) => {
                let worker = Arc::clone(&worker);
                std::thread::spawn(move || {
                    if let Err(error) = connection(stream, &worker) {
                        if !matches!(&error, WireError::Io(e) if e.kind() == std::io::ErrorKind::UnexpectedEof) {
                            eprintln!("tequila worker: {peer}: {error}");
                        }
                    }
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn reply(fields: &[(&str, Value)]) -> String {
    let mut out = Object::new();
    for (k, v) in fields {
        out.set(k, v.clone());
    }
    header(&out.build())
}

fn connection(stream: TcpStream, worker: &Worker) -> Result<(), WireError> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    let mut wire = Wire::accept(stream, &worker.token)?;
    // Between jobs a dispatcher may say nothing for as long as it likes.
    wire.stream().set_read_timeout(None)?;
    loop {
        let (text, payload) = wire.receive()?;
        let message = tequila_format::json::parse(text.as_bytes()).map_err(|_| WireError::Protocol("a header was not JSON".into()))?;
        match message.get("op").and_then(Value::as_str).unwrap_or("") {
            "status" => {
                let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
                let busy = worker.busy.try_lock().is_err();
                wire.send(&reply(&[("op", "status".into()), ("name", worker.name.clone().into()), ("cores", (cores as f64).into()), ("busy", busy.into())]), &[])?;
            }
            "render" => {
                let id = message.get("id").cloned().unwrap_or(Value::Null);
                let spec = match message.get("job").map(JobSpec::from_json) {
                    Some(Ok(spec)) => spec,
                    Some(Err(error)) => {
                        wire.send(&reply(&[("op", "failed".into()), ("id", id), ("message", error.into())]), &[])?;
                        continue;
                    }
                    None => {
                        wire.send(&reply(&[("op", "failed".into()), ("id", id), ("message", "the job had no description".into())]), &[])?;
                        continue;
                    }
                };
                let number = worker.jobs.fetch_add(1, Ordering::SeqCst);
                let folder = worker.scratch.join(format!("job-{}-{number}", std::process::id()));
                let start = Instant::now();
                let (sender, receiver) = std::sync::mpsc::channel();
                {
                    let render = Arc::clone(&worker.render);
                    let folder = folder.clone();
                    let spec = spec.clone();
                    let busy = Arc::clone(&worker.busy);
                    std::thread::spawn(move || {
                        let _rendering = busy.lock().unwrap_or_else(|p| p.into_inner());
                        eprintln!("tequila worker: rendering {}", spec.name);
                        // A render that panics fails its job, saying why,
                        // rather than leaving the dispatcher a silence.
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            bundle::unpack(&payload, &folder).and_then(|document| (render)(&spec, document, &folder))
                        }))
                        .unwrap_or_else(|panic| {
                            let why = panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                            Err(format!("the render failed on the worker: {why}"))
                        });
                        let _ = std::fs::remove_dir_all(&folder);
                        let _ = sender.send(result);
                    });
                }
                let result = loop {
                    match receiver.recv_timeout(HEARTBEAT) {
                        Ok(result) => break result,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            wire.send(&reply(&[("op", "working".into()), ("id", id.clone()), ("seconds", start.elapsed().as_secs_f64().into())]), &[])?;
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Err("the render stopped without an answer".to_string()),
                    }
                };
                match result {
                    Ok(picture) => wire.send(
                        &reply(&[("op", "result".into()), ("id", id), ("format", spec.format.clone().into()), ("seconds", start.elapsed().as_secs_f64().into())]),
                        &picture,
                    )?,
                    Err(message) => wire.send(&reply(&[("op", "failed".into()), ("id", id), ("message", message.into())]), &[])?,
                }
            }
            "bye" => return Ok(()),
            other => wire.send(&reply(&[("op", "failed".into()), ("message", format!("`{other}` is not something a worker does").into())]), &[])?,
        }
    }
}

