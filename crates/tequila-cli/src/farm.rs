//! Network rendering from the command line: `--worker` makes this machine a
//! render node, and `--farm` sends a run's renders to render nodes instead
//! of rendering them here (`tequila_network`).

use crate::{framing, is_project, output_for, render_one, select, Options};
use tequila_network::worker::{serve, Render, Worker};
use tequila_network::{Farm, Job, JobSpec, JobState, WorkerState};
use tequila_scene::SceneDocument;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The farm's shared secret: `--token`, or `TEQUILA_FARM_TOKEN`.
fn token(options: &Options) -> Result<Vec<u8>, String> {
    let token = options
        .token
        .clone()
        .or_else(|| std::env::var("TEQUILA_FARM_TOKEN").ok())
        .ok_or("a render farm needs a token, the same on every machine: --token, or TEQUILA_FARM_TOKEN")?;
    if token.len() < 8 {
        return Err("the farm's token must be at least 8 characters".into());
    }
    Ok(token.into_bytes())
}

/// Serve as a render node until the process is stopped.
pub fn worker(options: &Options) -> Result<(), String> {
    let token = token(options)?;
    let listen = options.listen.clone().unwrap_or_else(|| format!("0.0.0.0:{}", tequila_network::DEFAULT_PORT));
    let listener = std::net::TcpListener::bind(&listen).map_err(|e| format!("cannot listen on {listen}: {e}"))?;
    let name = options
        .worker_name
        .clone()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "render node".into());
    let scratch = std::env::temp_dir().join("tequila-worker");
    let base = options.clone();
    let render: Arc<Render> = Arc::new(move |spec: &JobSpec, document: SceneDocument, folder: &Path| render_job(&base, spec, document, folder));
    // The address it got, which a port of 0 leaves to the system.
    let bound = listener.local_addr().map(|a| a.to_string()).unwrap_or(listen);
    println!("tequila worker {name}: listening on {bound}");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    serve(listener, Arc::new(Worker::new(token, name, scratch, render)), Arc::new(AtomicBool::new(false))).map_err(|e| e.to_string())
}

/// One job, rendered as `tequila -i` the project would render it.
fn render_job(base: &Options, spec: &JobSpec, mut document: SceneDocument, folder: &Path) -> Result<Vec<u8>, String> {
    let mut options = base.clone();
    options.input = Some(folder.join("job.tequila").to_string_lossy().into_owned());
    options.width = spec.width.or(base.width);
    options.height = spec.height.or(base.height);
    options.samples = spec.samples.or(base.samples);
    options.camera = spec.camera.clone();
    options.studio = spec.studio.clone();
    options.model_set = spec.model_set.clone();
    options.configuration = spec.configuration.clone();
    options.alpha = spec.alpha;
    options.silhouette = spec.silhouette;
    // Cycles where this node has it, on its own --device; else Tequila's
    // own, as the application does without it.
    // A job names its engine; none named is Tequila's.
    let cycles = spec.engine.as_deref() == Some("cycles") && tequila_render_cycles::available().is_ok();
    options.cycles = Some(cycles);
    if spec.engine.as_deref() == Some("cycles") && !cycles {
        eprintln!("tequila worker: Cycles is not installed here; rendering {} with Tequila", spec.name);
    }
    select(&mut document, &options)?;
    let output = folder.join(format!("render.{}", spec.format));
    render_one(&options, &document, &output.to_string_lossy(), None)?;
    std::fs::read(&output).map_err(|e| format!("the render was not written: {e}"))
}

/// Send this run's renders to the farm: the shot as set up, or with
/// `--all-configurations` every valid configuration, each to its file.
pub fn dispatch(options: &Options, document: &SceneDocument) -> Result<(), String> {
    let token = token(options)?;
    let addresses: Vec<&str> = options.farm.as_deref().unwrap_or("").split(',').map(str::trim).filter(|a| !a.is_empty()).collect();
    if addresses.is_empty() {
        return Err("--farm needs the render nodes: host[:port], comma-separated".into());
    }
    let format = Path::new(&options.output).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "png".into());
    let format = if format == "jpeg" { "jpg".to_string() } else { format };
    if !matches!(format.as_str(), "png" | "jpg") {
        return Err(format!("a render node writes .png or .jpg, not .{format}"));
    }
    let mut document = document.clone();
    let mut spec = JobSpec {
        width: options.width,
        height: options.height,
        samples: options.samples,
        format,
        // As here: the flag, or a project's own engine.
        engine: options.cycles.unwrap_or(is_project(options) && document.render.engine.is_cycles()).then(|| "cycles".to_string()),
        alpha: options.alpha,
        silhouette: options.silhouette,
        ..JobSpec::default()
    };
    if !is_project(options) {
        // A model or the demo renders here framed, at the application's
        // defaults; a node renders what it is sent as a project, so it is
        // sent framed and sized.
        spec.width = spec.width.or(Some(960));
        spec.height = spec.height.or(Some(600));
        spec.samples = spec.samples.or(Some(128));
        let aspect = spec.width.unwrap_or(960) as f32 / spec.height.unwrap_or(600) as f32;
        let (camera, target) = framing(options, &document, aspect);
        let index = document.cameras.active_index();
        let name = document.cameras.active().name.clone();
        tequila_scene::CommandBus::new()
            .apply(&mut document, tequila_scene::Command::SetCamera { index, camera: Box::new(tequila_scene::lighting::SavedCamera { name, camera, target }) })
            .map_err(|e| e.to_string())?;
    }
    let base = options.input.as_deref().and_then(|p| Path::new(p).parent()).map(Path::to_path_buf);
    let bundle = tequila_network::bundle::pack(&document, &tequila_format::Preserved::default(), base.as_deref());
    for missing in &bundle.missing {
        eprintln!("tequila: {missing} could not be read, so the render nodes go without it");
    }
    println!("project packed: {:.1} MB", bundle.bytes.len() as f64 / 1e6);

    let farm = Farm::new(token);
    for address in &addresses {
        farm.add_worker(address);
    }
    let submit = |spec: JobSpec, output: String| {
        farm.submit(Job { spec, bundle: Arc::clone(&bundle.bytes), output: PathBuf::from(output) });
    };
    if options.all_configurations {
        let configurator = &document.configurator;
        if configurator.is_empty() {
            return Err("the project has no configurator to render the configurations of".into());
        }
        for choices in configurator.combinations() {
            let words = configurator.describe(&choices).join(" - ");
            let output = output_for(&options.output, &words);
            // By number: a label may hold a comma.
            let configuration = choices.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",");
            submit(JobSpec { name: words, configuration: Some(configuration), ..spec.clone() }, output);
        }
    } else {
        let name = Path::new(&options.output).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "render".into());
        submit(JobSpec { name, ..spec }, options.output.clone());
    }

    let mut last = String::new();
    farm.wait(|jobs, workers| {
        let count = |f: fn(&JobState) -> bool| jobs.iter().filter(|j| f(&j.state)).count();
        let nodes: Vec<String> = workers
            .iter()
            .map(|w| {
                let who = if w.name.is_empty() { w.address.clone() } else { w.name.clone() };
                match &w.state {
                    WorkerState::Connecting => format!("{who} connecting"),
                    WorkerState::Idle => format!("{who} idle"),
                    WorkerState::Rendering { seconds, .. } => format!("{who} rendering {seconds:.0}s"),
                    WorkerState::Offline(why) => format!("{who} offline ({why})"),
                }
            })
            .collect();
        let line = format!(
            "{}/{} done, {} rendering, {} queued, {} failed | {}",
            count(|s| matches!(s, JobState::Done { .. })),
            jobs.len(),
            count(|s| matches!(s, JobState::Rendering { .. })),
            count(|s| matches!(s, JobState::Queued)),
            count(|s| matches!(s, JobState::Failed(_))),
            nodes.join(", ")
        );
        if line != last {
            println!("{line}");
            last = line;
        }
    });
    let jobs = farm.jobs();
    for job in &jobs {
        if let JobState::Done { worker, seconds } = &job.state {
            println!("wrote {} ({worker}, {seconds:.1}s)", job.output.display());
        }
    }
    let failed: Vec<String> = jobs.iter().filter_map(|j| if let JobState::Failed(why) = &j.state { Some(format!("{}: {why}", j.name)) } else { None }).collect();
    if failed.is_empty() { Ok(()) } else { Err(format!("{} of {} renders failed:\n  {}", failed.len(), jobs.len(), failed.join("\n  "))) }
}
