//! The dispatcher: a queue of renders and the workers that take them.
//!
//! Each worker has a thread that keeps it connected, takes the next job
//! when it is free and writes the picture where the job says. A worker that
//! drops mid-job gives the job back to the queue for another (three tries,
//! then it fails); one that cannot be reached is tried again, less and less
//! often, until it can. A render that fails on a worker - a project it
//! cannot read - fails the job, since another worker would fail it too.
//!
//! What the monitor shows is [`Farm::workers`] and [`Farm::jobs`]: plain
//! snapshots, safe to take every frame.

use crate::job::{header, JobSpec};
use crate::wire::{Wire, WireError};
use tequila_format::json::Value;
use tequila_format::json_write::Object;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The port a worker listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 7740;
/// Tries a job gets on workers that drop it.
const ATTEMPTS: u32 = 3;

/// A render to do: what, from which project bundle, and where its picture
/// goes on this machine.
#[derive(Clone)]
pub struct Job {
    pub spec: JobSpec,
    pub bundle: Arc<Vec<u8>>,
    pub output: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkerState {
    Connecting,
    Idle,
    Rendering { job: u64, seconds: f64 },
    Offline(String),
}

#[derive(Clone, Debug)]
pub struct WorkerStatus {
    pub address: String,
    /// What the worker calls itself, once it has said.
    pub name: String,
    pub cores: usize,
    pub state: WorkerState,
    pub done: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Queued,
    Rendering { worker: String, seconds: f64 },
    Done { worker: String, seconds: f64 },
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct JobStatus {
    pub id: u64,
    pub name: String,
    pub output: PathBuf,
    pub state: JobState,
    pub attempts: u32,
}

struct Slot {
    address: String,
    status: WorkerStatus,
    removed: bool,
}

struct Entry {
    id: u64,
    job: Job,
    state: JobState,
    attempts: u32,
}

#[derive(Default)]
struct Shared {
    workers: Vec<Slot>,
    jobs: Vec<Entry>,
    next: u64,
    closing: bool,
}

pub struct Farm {
    shared: Arc<Mutex<Shared>>,
    token: Arc<Vec<u8>>,
}

/// `host` or `host:port`, with the default port when none is given.
pub fn address(text: &str) -> String {
    let text = text.trim();
    if text.rsplit_once(':').is_some_and(|(_, port)| port.parse::<u16>().is_ok()) && !text.ends_with(']') {
        text.to_string()
    } else {
        format!("{text}:{DEFAULT_PORT}")
    }
}

impl Farm {
    pub fn new(token: impl Into<Vec<u8>>) -> Farm {
        Farm { shared: Arc::new(Mutex::new(Shared::default())), token: Arc::new(token.into()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Start using a worker at `host[:port]`. False if it is already in.
    pub fn add_worker(&self, address_text: &str) -> bool {
        let address = address(address_text);
        {
            let mut shared = self.lock();
            if shared.workers.iter().any(|w| w.address == address && !w.removed) {
                return false;
            }
            shared.workers.retain(|w| w.address != address);
            shared.workers.push(Slot {
                address: address.clone(),
                status: WorkerStatus { address: address.clone(), name: String::new(), cores: 0, state: WorkerState::Connecting, done: 0 },
                removed: false,
            });
        }
        let shared = Arc::clone(&self.shared);
        let token = Arc::clone(&self.token);
        std::thread::Builder::new()
            .name(format!("farm {address}"))
            .spawn(move || run(shared, token, address))
            .expect("a thread for the worker");
        true
    }

    /// Stop using a worker. A job it is rendering finishes first.
    pub fn remove_worker(&self, address_text: &str) {
        let address = address(address_text);
        let mut shared = self.lock();
        for slot in shared.workers.iter_mut().filter(|w| w.address == address) {
            slot.removed = true;
        }
    }

    pub fn submit(&self, job: Job) -> u64 {
        let mut shared = self.lock();
        shared.next += 1;
        let id = shared.next;
        shared.jobs.push(Entry { id, job, state: JobState::Queued, attempts: 0 });
        id
    }

    /// A queued job taken off the queue; one rendering finishes.
    pub fn cancel(&self, id: u64) {
        let mut shared = self.lock();
        if let Some(entry) = shared.jobs.iter_mut().find(|e| e.id == id && e.state == JobState::Queued) {
            entry.state = JobState::Failed("cancelled".into());
        }
    }

    /// A failed job put back on the queue.
    pub fn retry(&self, id: u64) {
        let mut shared = self.lock();
        if let Some(entry) = shared.jobs.iter_mut().find(|e| e.id == id && matches!(e.state, JobState::Failed(_))) {
            entry.state = JobState::Queued;
            entry.attempts = 0;
        }
    }

    /// Done and failed jobs, forgotten.
    pub fn clear_finished(&self) {
        self.lock().jobs.retain(|e| matches!(e.state, JobState::Queued | JobState::Rendering { .. }));
    }

    pub fn workers(&self) -> Vec<WorkerStatus> {
        self.lock().workers.iter().filter(|w| !w.removed).map(|w| w.status.clone()).collect()
    }

    pub fn jobs(&self) -> Vec<JobStatus> {
        self.lock()
            .jobs
            .iter()
            .map(|e| JobStatus { id: e.id, name: e.job.spec.name.clone(), output: e.job.output.clone(), state: e.state.clone(), attempts: e.attempts })
            .collect()
    }

    /// Whether every job is done or has failed.
    pub fn finished(&self) -> bool {
        self.lock().jobs.iter().all(|e| matches!(e.state, JobState::Done { .. } | JobState::Failed(_)))
    }

    /// Block until [`Farm::finished`], calling `each` with the jobs about
    /// once a second, for progress.
    pub fn wait(&self, mut each: impl FnMut(&[JobStatus], &[WorkerStatus])) {
        loop {
            let (jobs, workers) = (self.jobs(), self.workers());
            each(&jobs, &workers);
            if self.finished() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
}

impl Drop for Farm {
    fn drop(&mut self) {
        self.lock().closing = true;
    }
}

fn update(shared: &Mutex<Shared>, f: impl FnOnce(&mut Shared)) {
    f(&mut shared.lock().unwrap_or_else(|p| p.into_inner()));
}

fn set_worker(shared: &Mutex<Shared>, address: &str, f: impl FnOnce(&mut WorkerStatus)) {
    update(shared, |s| {
        if let Some(slot) = s.workers.iter_mut().find(|w| w.address == address && !w.removed) {
            f(&mut slot.status);
        }
    });
}

/// What a worker is called in a job's state: its own name once it has
/// said it, else its address.
fn label(shared: &Mutex<Shared>, address: &str) -> String {
    let s = shared.lock().unwrap_or_else(|p| p.into_inner());
    s.workers.iter().find(|w| w.address == address && !w.removed).map(|w| w.status.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| address.to_string())
}

fn leaving(shared: &Mutex<Shared>, address: &str) -> bool {
    let s = shared.lock().unwrap_or_else(|p| p.into_inner());
    s.closing || !s.workers.iter().any(|w| w.address == address && !w.removed)
}

/// Why a node could not be reached, in words a person can act on.
fn unreachable(error: &std::io::Error) -> String {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::ConnectionRefused => "nothing listens there: is tequila-cli --worker running on it?".into(),
        ErrorKind::TimedOut | ErrorKind::WouldBlock => "no answer: is the machine on, and its port open?".into(),
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => "the network cannot reach it".into(),
        _ => error.to_string(),
    }
}

fn connect(address: &str, token: &[u8]) -> Result<Wire, String> {
    let target = address
        .to_socket_addrs()
        .map_err(|_| "that name is not known on this network".to_string())?
        .next()
        .ok_or_else(|| format!("{address} is not an address"))?;
    let stream = TcpStream::connect_timeout(&target, Duration::from_secs(5)).map_err(|e| unreachable(&e))?;
    stream.set_read_timeout(Some(Duration::from_secs(20))).map_err(|e| e.to_string())?;
    stream.set_nodelay(true).ok();
    Wire::connect(stream, token).map_err(|e| e.to_string())
}

fn message(fields: &[(&str, Value)]) -> String {
    let mut out = Object::new();
    for (k, v) in fields {
        out.set(k, v.clone());
    }
    header(&out.build())
}

/// A worker's thread: connect, work, and when the connection goes, again.
fn run(shared: Arc<Mutex<Shared>>, token: Arc<Vec<u8>>, address: String) {
    let mut wait = Duration::from_secs(1);
    while !leaving(&shared, &address) {
        // A node that could not be reached stays shown as such, with why,
        // while it is tried again.
        set_worker(&shared, &address, |w| {
            if !matches!(w.state, WorkerState::Offline(_)) {
                w.state = WorkerState::Connecting;
            }
        });
        let outcome = connect(&address, &token).and_then(|mut wire| {
            wait = Duration::from_secs(1);
            wire.send(&message(&[("op", "status".into())]), &[]).map_err(|e| e.to_string())?;
            let (status, _) = wire.receive().map_err(|e| e.to_string())?;
            let status = tequila_format::json::parse(status.as_bytes()).map_err(|_| "the worker's status was not JSON".to_string())?;
            set_worker(&shared, &address, |w| {
                w.name = status.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                w.cores = status.get("cores").and_then(Value::as_f64).unwrap_or(0.0) as usize;
                w.state = WorkerState::Idle;
            });
            work(&shared, &mut wire, &address)
        });
        if let Err(error) = outcome {
            set_worker(&shared, &address, |w| w.state = WorkerState::Offline(error));
            let until = Instant::now() + wait;
            while Instant::now() < until && !leaving(&shared, &address) {
                std::thread::sleep(Duration::from_millis(100));
            }
            wait = (wait * 2).min(Duration::from_secs(30));
        }
    }
}

/// Give a job back after its worker dropped it: to the queue, or failed
/// when it has had its tries.
fn give_back(shared: &Mutex<Shared>, id: u64, why: &str) {
    update(shared, |s| {
        if let Some(entry) = s.jobs.iter_mut().find(|e| e.id == id) {
            entry.state = if entry.attempts >= ATTEMPTS { JobState::Failed(format!("{ATTEMPTS} workers dropped it; the last: {why}")) } else { JobState::Queued };
        }
    });
}

/// Take jobs until the connection goes (an error) or the worker leaves the
/// farm (Ok).
fn work(shared: &Mutex<Shared>, wire: &mut Wire, address: &str) -> Result<(), String> {
    let mut last_word = Instant::now();
    let who = label(shared, address);
    loop {
        if leaving(shared, address) {
            let _ = wire.send(&message(&[("op", "bye".into())]), &[]);
            return Ok(());
        }
        let next = {
            let mut s = shared.lock().unwrap_or_else(|p| p.into_inner());
            match s.jobs.iter_mut().find(|e| e.state == JobState::Queued) {
                Some(entry) => {
                    entry.state = JobState::Rendering { worker: who.clone(), seconds: 0.0 };
                    entry.attempts += 1;
                    Some((entry.id, entry.job.clone()))
                }
                None => None,
            }
        };
        let Some((id, job)) = next else {
            // Idle: make sure it is still there now and then.
            if last_word.elapsed() > Duration::from_secs(10) {
                wire.send(&message(&[("op", "status".into())]), &[]).map_err(|e| e.to_string())?;
                wire.receive().map_err(|e| e.to_string())?;
                last_word = Instant::now();
            }
            std::thread::sleep(Duration::from_millis(150));
            continue;
        };
        set_worker(shared, address, |w| w.state = WorkerState::Rendering { job: id, seconds: 0.0 });
        let start = Instant::now();
        let request = message(&[("op", "render".into()), ("id", (id as f64).into()), ("job", job.spec.to_json())]);
        // A long render is said to be going on every few seconds; a minute
        // of silence is a worker that has gone.
        wire.stream().set_read_timeout(Some(Duration::from_secs(60))).ok();
        if let Err(error) = wire.send(&request, &job.bundle) {
            give_back(shared, id, &error.to_string());
            return Err(error.to_string());
        }
        loop {
            let (text, payload) = match wire.receive() {
                Ok(frame) => frame,
                Err(error) => {
                    let why = match &error {
                        WireError::Io(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => "the worker went silent".to_string(),
                        other => other.to_string(),
                    };
                    give_back(shared, id, &why);
                    return Err(why);
                }
            };
            let reply = tequila_format::json::parse(text.as_bytes()).unwrap_or(Value::Null);
            let seconds = start.elapsed().as_secs_f64();
            match reply.get("op").and_then(Value::as_str).unwrap_or("") {
                "working" => {
                    update(shared, |s| {
                        if let Some(entry) = s.jobs.iter_mut().find(|e| e.id == id) {
                            entry.state = JobState::Rendering { worker: who.clone(), seconds };
                        }
                    });
                    set_worker(shared, address, |w| w.state = WorkerState::Rendering { job: id, seconds });
                }
                "result" => {
                    let written = job.output.parent().map_or(Ok(()), |p| if p.as_os_str().is_empty() { Ok(()) } else { std::fs::create_dir_all(p) }).and_then(|_| std::fs::write(&job.output, &payload));
                    update(shared, |s| {
                        if let Some(entry) = s.jobs.iter_mut().find(|e| e.id == id) {
                            entry.state = match &written {
                                Ok(()) => JobState::Done { worker: who.clone(), seconds },
                                Err(e) => JobState::Failed(format!("rendered, but {} could not be written: {e}", job.output.display())),
                            };
                        }
                    });
                    set_worker(shared, address, |w| {
                        w.done += 1;
                        w.state = WorkerState::Idle;
                    });
                    break;
                }
                "failed" => {
                    let why = reply.get("message").and_then(Value::as_str).unwrap_or("the render failed").to_string();
                    update(shared, |s| {
                        if let Some(entry) = s.jobs.iter_mut().find(|e| e.id == id) {
                            entry.state = JobState::Failed(why);
                        }
                    });
                    set_worker(shared, address, |w| w.state = WorkerState::Idle);
                    break;
                }
                _ => {}
            }
        }
        last_word = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::{serve, Worker};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A worker on this machine whose "render" is the job's name, after a
    /// pause, or a failure for a job named "bad".
    fn worker(token: &'static str, pause: Duration) -> (String, Arc<AtomicBool>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let render: Arc<crate::worker::Render> = Arc::new(move |spec: &JobSpec, _document, _folder: &std::path::Path| {
            std::thread::sleep(pause);
            if spec.name == "bad" { Err("this project cannot be rendered".to_string()) } else { Ok(spec.name.clone().into_bytes()) }
        });
        let scratch = std::env::temp_dir().join(format!("tequila-farm-test-{}", std::process::id()));
        let worker = Arc::new(Worker::new(token, "test worker", scratch, render));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        std::thread::spawn(move || serve(listener, worker, flag));
        (address, stop)
    }

    fn bundle() -> Arc<Vec<u8>> {
        crate::bundle::pack(&tequila_scene::SceneDocument::new(), &tequila_format::Preserved::default(), None).bytes
    }

    fn job(name: &str, output: PathBuf) -> Job {
        Job { spec: JobSpec { name: name.into(), format: "png".into(), ..JobSpec::default() }, bundle: bundle(), output }
    }

    #[test]
    fn jobs_are_shared_out_and_their_pictures_written() {
        let folder = std::env::temp_dir().join(format!("tequila-farm-out-{}", std::process::id()));
        let (a, stop_a) = worker("farm token", Duration::from_millis(150));
        let (b, stop_b) = worker("farm token", Duration::from_millis(150));
        let farm = Farm::new("farm token");
        farm.add_worker(&a);
        farm.add_worker(&b);
        for n in 0..6 {
            farm.submit(job(&format!("shot {n}"), folder.join(format!("shot {n}.png"))));
        }
        farm.submit(job("bad", folder.join("bad.png")));
        farm.wait(|_, _| {});
        let jobs = farm.jobs();
        for n in 0..6 {
            assert_eq!(std::fs::read(folder.join(format!("shot {n}.png"))).unwrap(), format!("shot {n}").into_bytes());
        }
        let failed: Vec<_> = jobs.iter().filter(|j| matches!(j.state, JobState::Failed(_))).map(|j| j.name.clone()).collect();
        assert_eq!(failed, ["bad"], "only the bad job fails, and it is not retried elsewhere");
        let workers = farm.workers();
        assert!(workers.iter().all(|w| w.done > 0), "both workers took jobs: {workers:?}");
        assert!(workers.iter().all(|w| w.name == "test worker"));
        stop_a.store(true, Ordering::SeqCst);
        stop_b.store(true, Ordering::SeqCst);
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn a_worker_with_another_token_is_offline_and_takes_nothing() {
        let (a, stop) = worker("their token", Duration::ZERO);
        let farm = Farm::new("our token");
        farm.add_worker(&a);
        farm.submit(job("shot", std::env::temp_dir().join("never.png")));
        std::thread::sleep(Duration::from_millis(800));
        let workers = farm.workers();
        assert!(matches!(&workers[0].state, WorkerState::Offline(why) if why.contains("token")), "{workers:?}");
        assert_eq!(farm.jobs()[0].state, JobState::Queued);
        stop.store(true, Ordering::SeqCst);
    }

    #[test]
    fn a_job_dropped_by_its_worker_goes_to_another() {
        let folder = std::env::temp_dir().join(format!("tequila-farm-drop-{}", std::process::id()));
        // A "worker" that accepts the job and then hangs up.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let dropping = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                if let Ok(mut wire) = Wire::accept(stream, b"farm token") {
                    loop {
                        let Ok((text, _)) = wire.receive() else { break };
                        if text.contains("\"render\"") {
                            break;
                        }
                        let _ = wire.send(&message(&[("op", "status".into()), ("name", "flaky".into())]), &[]);
                    }
                }
            }
        });
        let farm = Farm::new("farm token");
        farm.add_worker(&dropping);
        farm.submit(job("shot", folder.join("shot.png")));
        // Let the flaky one take it and drop it, then bring a good one.
        let deadline = Instant::now() + Duration::from_secs(5);
        while farm.jobs()[0].attempts == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let (good, stop) = worker("farm token", Duration::ZERO);
        farm.remove_worker(&dropping);
        farm.add_worker(&good);
        farm.wait(|_, _| {});
        assert!(matches!(farm.jobs()[0].state, JobState::Done { .. }), "{:?}", farm.jobs());
        assert_eq!(std::fs::read(folder.join("shot.png")).unwrap(), b"shot");
        stop.store(true, Ordering::SeqCst);
        std::fs::remove_dir_all(&folder).ok();
    }

    #[test]
    fn a_node_nobody_runs_says_so_in_words() {
        // A port taken and let go: nothing listens there now.
        let address = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().to_string();
        let farm = Farm::new("farm token");
        farm.add_worker(&address);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !matches!(farm.workers()[0].state, WorkerState::Offline(_)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let state = farm.workers()[0].state.clone();
        assert!(matches!(&state, WorkerState::Offline(why) if why.contains("nothing listens")), "{state:?}");
        // And it stays offline while it is tried again.
        let until = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < until {
            assert!(matches!(farm.workers()[0].state, WorkerState::Offline(_)), "it flickered back to connecting");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn an_address_without_a_port_gets_the_farm_port() {
        assert_eq!(address("render-07"), format!("render-07:{DEFAULT_PORT}"));
        assert_eq!(address("10.0.0.5:9000"), "10.0.0.5:9000");
    }
}
