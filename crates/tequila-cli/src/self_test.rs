//! `tequila --self-test`: a few minutes' check of the computer it runs on,
//! for the machines the people who make Tequila do not have - AMD graphics
//! cards, several cards at once, Intel and Apple-silicon Macs.
//!
//! It renders one small built-in scene (`scene.rs`: glass, gold, a texture,
//! fibres and a lamp) with every engine and device there is here: Tequila's
//! own renderer on the processor, Cycles on the processor and on each kind
//! of graphics card Cycles finds - each card alone, and every card of a kind
//! together where there are several - and puts both denoisers, OIDN on the
//! processor and on the card, through a noisy render of it. Each picture is
//! timed and compared with a reference shipped in the repository
//! (`crates/tequila-cli/self-test/`, made by `--self-test-references`), and
//! passes or fails by how far it is from it. The application adds its
//! viewport (`--viewport`).
//!
//! What comes out is a short report to paste into the Discord, and a folder
//! holding it and the pictures. Nothing is sent anywhere. The report names
//! no one: no user or computer name, no paths, serial numbers or network
//! addresses (`system.rs`, and [`scrub`] for the messages that could carry
//! them).
//!
//! From the application (Help > Test this computer) it runs as a process of
//! its own (`--machine`), so a graphics driver that brings it down takes the
//! test with it and not the application, which starts it again past the
//! check that crashed (`--resume`): the folder keeps what is done
//! (`results.tsv`), and the report says what crashed.

pub mod scene;
pub mod system;

use scene::{Reference, HEIGHT, WIDTH};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use system::Facts;
use tequila_core::math::vec3;
use tequila_core::spectrum::Spectrum;
use tequila_denoise::{DenoiseSettings, Denoiser, GpuDenoiser};
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};

/// Where to share a report: the invite the website and the README give.
pub const DISCORD: &str = "https://discord.gg/Na5T2qgQBX";

/// The references, shipped in the binary so a packaged Tequila needs no
/// file beside it to test itself.
const TEQUILA_REFERENCE: &[u8] = include_bytes!("../self-test/tequila.png");
const CYCLES_REFERENCE: &[u8] = include_bytes!("../self-test/cycles.png");

/// Samples a pixel for the engines' pictures, for the denoisers' noisy one,
/// and for the references.
const RENDER_SAMPLES: u32 = 64;
const NOISY_SAMPLES: u32 = 8;
pub const REFERENCE_SAMPLES: u32 = 4096;

/// The engines' pictures are compared after averaging each square of this
/// many pixels a side, which takes most of the noise of 64 samples out and
/// leaves anything wrong - a missing texture, black glass, a wrong
/// exposure - as large as it was. The denoisers' are compared pixel for
/// pixel: averaging would hide the noise they are there to remove.
const ENGINE_BLOCK: usize = 4;
/// The most an engine's picture may differ from its reference, and a
/// denoised one from the converged picture, as the root-mean-square of the
/// difference in percent of full scale. On the development machine every
/// engine and device came out at 0.24-0.29% and both denoisers at 1.04%,
/// while the two engines' references are 1.42% apart (`docs/05` §3.11): the
/// limits leave room for another processor's rounding and another OIDN's
/// kernels, and none for a device that renders as the other engine does,
/// let alone a missing texture, black glass or a wrong exposure.
const ENGINE_LIMIT: f64 = 1.0;
const DENOISE_LIMIT: f64 = 2.0;
/// And a denoised picture must be this much closer than the noisy one was:
/// one that passed its input through would otherwise pass.
const DENOISE_GAIN: f64 = 0.6;

/// Set to stop: by `cancel` on standard input (the application's Cancel),
/// or when standard input closes (the application has gone).
static CANCEL: AtomicBool = AtomicBool::new(false);

fn cancelled() -> bool {
    CANCEL.load(Ordering::Acquire)
}

/// How a self-test runs.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// Where the report, the pictures and the run's progress go.
    pub folder: PathBuf,
    /// Leave the graphics cards out: what CI can run.
    pub cpu_only: bool,
    /// Describe the computer and render nothing.
    pub facts_only: bool,
    /// Carry on a run in `folder` that stopped, past the check it stopped
    /// in.
    pub resume: bool,
    /// Progress as lines for the application (`@step`, `@progress`,
    /// `@result`, `@done`), and `cancel` heard on standard input.
    pub machine: bool,
    /// The application's viewport check: "ok: what it found" or
    /// "failed: why".
    pub viewport: Option<String>,
    /// With `resume`, what the application saw of the crash: an exit code,
    /// the last thing the test said.
    pub crashed: Option<String>,
}

/// Where a run's folder goes by default: `.tequila/self-test/<when>` in the
/// person's home, beside the crash reports.
pub fn default_folder() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join(".tequila").join("self-test").join(format!("{} {}", tequila_core::clock::date(), tequila_core::clock::time_for_filename()))
}

/// What became of one check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pass,
    Fail,
    Skip,
    /// The test stopped during this check, and was started again past it.
    Crash,
    /// Cancelled before it ran, or while it ran.
    Stopped,
}

impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "fail",
            Status::Skip => "skip",
            Status::Crash => "crash",
            Status::Stopped => "stopped",
        }
    }

    fn from_key(key: &str) -> Option<Status> {
        [Status::Pass, Status::Fail, Status::Skip, Status::Crash, Status::Stopped].into_iter().find(|s| s.key() == key)
    }

    /// As the report shows it: what went wrong stands out.
    fn word(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Fail => "FAIL",
            Status::Skip => "skip",
            Status::Crash => "CRASH",
            Status::Stopped => "stop",
        }
    }
}

/// One check's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub id: String,
    pub label: String,
    pub status: Status,
    /// How long the render or the denoise took.
    pub seconds: Option<f64>,
    /// How long it took to get going before that: opening a Cycles session
    /// on a card and loading its kernels, the denoiser's device.
    pub start: Option<f64>,
    /// How far from its reference, and how far it may be, in percent.
    pub error: Option<f64>,
    pub limit: Option<f64>,
    /// A line more: why it failed or was skipped, what it ran on.
    pub note: String,
}

impl Outcome {
    fn new(id: &str, label: &str, status: Status, note: impl Into<String>) -> Outcome {
        Outcome { id: id.into(), label: label.into(), status, seconds: None, start: None, error: None, limit: None, note: note.into() }
    }

    /// A line of `results.tsv`.
    fn to_line(&self) -> String {
        let number = |n: Option<f64>| n.map_or("-".to_string(), |n| format!("{n}"));
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        format!(
            "outcome\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.id,
            self.status.key(),
            number(self.seconds),
            number(self.start),
            number(self.error),
            number(self.limit),
            clean(&self.label),
            clean(&self.note)
        )
    }

    fn from_line(line: &str) -> Option<Outcome> {
        let fields: Vec<&str> = line.split('\t').collect();
        let ["outcome", id, status, seconds, start, error, limit, label, note] = fields[..] else { return None };
        let number = |s: &str| s.parse::<f64>().ok();
        Some(Outcome {
            id: id.into(),
            label: label.into(),
            status: Status::from_key(status)?,
            seconds: number(seconds),
            start: number(start),
            error: number(error),
            limit: number(limit),
            note: note.into(),
        })
    }
}

/// What a run's folder says was done: the outcomes so far, and the check
/// that was started and never finished, if one was.
pub fn read_progress(text: &str) -> (Vec<Outcome>, Option<String>) {
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut started = None;
    for line in text.lines() {
        if let Some(id) = line.strip_prefix("started\t") {
            started = Some(id.trim().to_string());
        } else if let Some(outcome) = Outcome::from_line(line) {
            if started.as_deref() == Some(outcome.id.as_str()) {
                started = None;
            }
            outcomes.retain(|o| o.id != outcome.id);
            outcomes.push(outcome);
        }
    }
    (outcomes, started)
}

/// What a check does.
#[derive(Clone, Debug, PartialEq)]
enum Work {
    Tequila,
    DenoiseCpu,
    /// Cycles on a device: CPU, a kind of card (every card of it), or one
    /// card's id.
    Cycles(String),
    /// Cycles found no card, though there may be one: said, not run.
    NoCard(String),
    DenoiseGpu,
    Viewport,
}

#[derive(Clone, Debug)]
struct Planned {
    id: String,
    label: String,
    work: Work,
}

/// A kind of card as people know it.
fn kind_label(kind: &str) -> String {
    match kind {
        "OPTIX" => "OptiX".into(),
        "CUDA" => "CUDA".into(),
        "HIP" => "HIP".into(),
        "METAL" => "Metal".into(),
        "ONEAPI" => "oneAPI".into(),
        other => other.into(),
    }
}

/// A card's name, short: the maker is in the facts above.
fn short(name: &str) -> String {
    name.trim().trim_start_matches("NVIDIA ").trim_start_matches("AMD ").to_string()
}

/// The checks, in the order they run: the processor first (Tequila's
/// renderer, its denoiser, Cycles), then each kind of card Cycles found -
/// each card alone and then together where a kind has several - then the
/// card's denoiser and the viewport.
fn plan(facts: &Facts, settings: &Settings) -> Vec<Planned> {
    if settings.facts_only {
        return Vec::new();
    }
    let check = |id: &str, label: &str, work: Work| Planned { id: id.into(), label: label.into(), work };
    let mut out = vec![
        check("tequila-cpu", "Tequila · processor", Work::Tequila),
        check("denoise-cpu", "Denoiser · processor", Work::DenoiseCpu),
        check("cycles-cpu", "Cycles · processor", Work::Cycles("CPU".into())),
    ];
    if !settings.cpu_only {
        if let Ok(devices) = &facts.cycles {
            let mut kinds: Vec<&str> = Vec::new();
            for device in devices.iter().filter(|d| d.kind != "CPU") {
                if !kinds.contains(&device.kind.as_str()) {
                    kinds.push(&device.kind);
                }
            }
            // The best kind first, as a render would choose it.
            kinds.sort_by_key(|k| ["OPTIX", "CUDA", "HIP", "METAL"].iter().position(|w| w == k).unwrap_or(9));
            for kind in &kinds {
                let cards: Vec<_> = devices.iter().filter(|d| d.kind == *kind).collect();
                let lower = kind.to_ascii_lowercase();
                let named = kind_label(kind);
                if let [card] = cards[..] {
                    out.push(check(&format!("cycles-{lower}"), &format!("Cycles · {named} · {}", short(&card.name)), Work::Cycles(kind.to_string())));
                    continue;
                }
                for (i, card) in cards.iter().enumerate() {
                    out.push(check(&format!("cycles-{lower}-{}", i + 1), &format!("Cycles · {named} · card {} · {}", i + 1, short(&card.name)), Work::Cycles(card.id.clone())));
                }
                out.push(check(&format!("cycles-{lower}-all"), &format!("Cycles · {named} · all {} cards", cards.len()), Work::Cycles(kind.to_string())));
            }
            if kinds.is_empty() {
                let cards: Vec<String> = facts.gpus.iter().map(|g| g.name.clone()).collect();
                let note = if cards.is_empty() {
                    "Cycles found no graphics card it can use".to_string()
                } else {
                    format!("Cycles found no graphics card it can use; this computer has {}", cards.join(", "))
                };
                out.push(check("cycles-gpu", "Cycles · graphics card", Work::NoCard(note)));
            }
        }
        out.push(check("denoise-gpu", "Denoiser · graphics card", Work::DenoiseGpu));
    }
    out.push(check("viewport", "Viewport", Work::Viewport));
    out
}

/// Progress, to a person at a terminal or to the application.
struct Events {
    machine: bool,
    shown: f32,
}

impl Events {
    fn say(&self, text: &str) {
        if !self.machine {
            println!("{text}");
        }
    }

    fn total(&self, count: usize) {
        if self.machine {
            println!("@total {count}");
        }
    }

    fn step(&mut self, index: usize, count: usize, label: &str) {
        self.shown = -1.0;
        if self.machine {
            println!("@step {index} {label}");
        } else {
            println!("[{index}/{count}] {label}");
        }
    }

    fn progress(&mut self, fraction: f32) {
        if (fraction - self.shown).abs() < 0.02 && fraction < 1.0 {
            return;
        }
        self.shown = fraction;
        if self.machine {
            println!("@progress {fraction:.3}");
        } else {
            print!("\r  {:>3.0}%   ", fraction * 100.0);
        }
        let _ = std::io::stdout().flush();
    }

    fn result(&self, outcome: &Outcome) {
        let summary = summary(outcome);
        if self.machine {
            println!("@result {} {}\t{summary}", outcome.status.key(), outcome.label);
        } else {
            println!("\r  {:<5}  {summary}", outcome.status.word());
        }
        let _ = std::io::stdout().flush();
    }
}

/// An outcome in a line: time, error, and why.
fn summary(outcome: &Outcome) -> String {
    let mut parts = Vec::new();
    if let Some(seconds) = outcome.seconds {
        parts.push(duration(seconds));
    }
    if let Some(error) = outcome.error {
        parts.push(error_text(error, outcome.limit));
    }
    if let Some(start) = outcome.start.filter(|&s| s >= 1.0) {
        parts.push(format!("first use {}", duration(start)));
    }
    if !outcome.note.is_empty() {
        parts.push(outcome.note.clone());
    }
    parts.join(" · ")
}

fn duration(seconds: f64) -> String {
    if seconds < 1.0 {
        format!("{seconds:.2} s")
    } else if seconds < 10.0 {
        format!("{seconds:.1} s")
    } else if seconds < 120.0 {
        format!("{seconds:.0} s")
    } else {
        format!("{:.0} min {:.0} s", (seconds / 60.0).floor(), seconds % 60.0)
    }
}

fn error_text(error: f64, limit: Option<f64>) -> String {
    match limit {
        Some(limit) => format!("{error:.2}% (max {limit}%)"),
        None => format!("{error:.2}%"),
    }
}

/// Run the checks; `Ok(true)` when none failed or crashed.
pub fn run(settings: &Settings) -> Result<bool, String> {
    let began = Instant::now();
    let folder = &settings.folder;
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let progress_file = folder.join("results.tsv");
    let (mut outcomes, crashed) = if settings.resume {
        read_progress(&std::fs::read_to_string(&progress_file).unwrap_or_default())
    } else {
        let _ = std::fs::remove_file(&progress_file);
        (Vec::new(), None)
    };
    let mut log = std::fs::OpenOptions::new().create(true).append(true).open(&progress_file).map_err(|e| format!("{}: {e}", progress_file.display()))?;
    let mut note_line = |line: String| {
        let _ = writeln!(log, "{line}");
        let _ = log.flush();
    };
    // When the run began, kept in the folder: a resumed run's report times
    // the whole of it.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let began_at = settings.resume.then(|| std::fs::read_to_string(&progress_file).ok()).flatten().and_then(|text| {
        text.lines().find_map(|l| l.strip_prefix("began\t")).and_then(|t| t.trim().parse::<f64>().ok())
    });
    if began_at.is_none() {
        note_line(format!("began\t{now}"));
    }
    let earlier = began_at.map_or(0.0, |at| (now - at).max(0.0));

    if settings.machine {
        // The application's Cancel, or the application gone.
        std::thread::spawn(|| {
            let mut line = String::new();
            loop {
                line.clear();
                match std::io::stdin().read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.trim() == "cancel" => break,
                    Ok(_) => {}
                }
            }
            CANCEL.store(true, Ordering::Release);
        });
    }

    let mut events = Events { machine: settings.machine, shown: -1.0 };
    events.say("Tequila self-test: a few checks of this computer. Nothing is sent anywhere.");
    events.say(&format!("The report and the pictures go to {}\n", folder.display()));

    // Listing Cycles' devices loads Cycles and wakes every graphics driver:
    // if that is what crashed, the run goes on without it.
    note_line("started\tfacts".into());
    let facts = system::gather_with(crashed.as_deref() != Some("facts"));
    note_line("outcome\tfacts\tpass\t-\t-\t-\t-\tfacts\t".into());
    for line in fact_lines(&facts) {
        events.say(&format!("  {line}"));
    }
    events.say("");

    let planned = plan(&facts, settings);
    events.total(planned.len());
    // A check that stopped the last run and is not one of this run's: still
    // reported.
    if let Some(id) = crashed.as_deref().filter(|id| *id != "facts" && !planned.iter().any(|p| p.id == *id) && !outcomes.iter().any(|o| o.id == *id)) {
        let outcome = crash(id, id, settings);
        note_line(outcome.to_line());
        events.result(&outcome);
        outcomes.push(outcome);
    }
    let mut lab = Lab::new();
    for (index, check) in planned.iter().enumerate() {
        if outcomes.iter().any(|o| o.id == check.id) {
            continue;
        }
        events.step(index + 1, planned.len(), &check.label);
        let outcome = if crashed.as_deref() == Some(check.id.as_str()) {
            crash(&check.id, &check.label, settings)
        } else if cancelled() {
            Outcome::new(&check.id, &check.label, Status::Stopped, "")
        } else {
            note_line(format!("started\t{}", check.id));
            let mut outcome = lab.run(check, &facts, settings, folder, &mut |f| events.progress(f));
            outcome.note = scrub(&outcome.note);
            outcome
        };
        note_line(outcome.to_line());
        events.result(&outcome);
        outcomes.push(outcome);
    }

    // In the order planned, whatever order a resumed run found them in.
    let mut ordered: Vec<Outcome> = planned.iter().filter_map(|p| outcomes.iter().find(|o| o.id == p.id).cloned()).collect();
    for outcome in &outcomes {
        if !ordered.iter().any(|o| o.id == outcome.id) && outcome.id != "facts" {
            ordered.push(outcome.clone());
        }
    }
    let text = report(&facts, &ordered, &build_line(), earlier + began.elapsed().as_secs_f64());
    let report_path = folder.join("report.md");
    std::fs::write(&report_path, &text).map_err(|e| format!("{}: {e}", report_path.display()))?;
    let failed = ordered.iter().any(|o| matches!(o.status, Status::Fail | Status::Crash));
    if settings.machine {
        println!("@done {}", if failed { "fail" } else { "pass" });
    } else {
        println!("\n{text}");
        println!("Saved in {}", folder.display());
        println!("To share it, paste the report above into the Tequila Discord: {DISCORD}");
    }
    let _ = std::io::stdout().flush();
    Ok(!failed)
}

/// A check the test stopped during, with what the application saw of it.
fn crash(id: &str, label: &str, settings: &Settings) -> Outcome {
    let why = settings.crashed.as_deref().map(scrub).filter(|s| !s.is_empty());
    Outcome::new(id, label, Status::Crash, why.map_or("the test stopped during this check".into(), |w| format!("the test stopped during this check: {w}")))
}

/// What the checks share: the scene, built once, and the noisy picture the
/// denoisers work on, rendered when first wanted.
struct Lab {
    reference: Reference,
    noisy: Option<Noisy>,
}

/// A few samples' render with its denoising guides, and as it looks.
struct Noisy {
    color: Vec<f32>,
    albedo: Vec<f32>,
    normal: Vec<f32>,
    developed: Vec<u8>,
}

impl Lab {
    fn new() -> Lab {
        Lab { reference: scene::build(), noisy: None }
    }

    fn run(&mut self, check: &Planned, facts: &Facts, settings: &Settings, folder: &Path, progress: &mut dyn FnMut(f32)) -> Outcome {
        let id = check.id.as_str();
        let label = check.label.as_str();
        let result = match &check.work {
            Work::Tequila => self.tequila(id, label, folder, progress),
            Work::DenoiseCpu => self.denoise(id, label, false, facts, folder, progress),
            Work::DenoiseGpu => self.denoise(id, label, true, facts, folder, progress),
            Work::Cycles(device) => self.cycles(id, label, device, facts, folder, progress),
            Work::NoCard(note) => Ok(Outcome::new(id, label, Status::Skip, note.clone())),
            Work::Viewport => Ok(viewport(id, label, settings.viewport.as_deref())),
        };
        result.unwrap_or_else(|error| Outcome::new(id, label, Status::Fail, error))
    }

    fn tequila(&mut self, id: &str, label: &str, folder: &Path, progress: &mut dyn FnMut(f32)) -> Result<Outcome, String> {
        let start = Instant::now();
        tequila_materials::energy::prewarm();
        let intersector = build_intersector(&self.reference)?;
        let start = start.elapsed().as_secs_f64();
        let timer = Instant::now();
        let Some(framebuffer) = render_tequila(&self.reference, &intersector, RENDER_SAMPLES, 0, progress) else {
            return Ok(Outcome::new(id, label, Status::Stopped, ""));
        };
        let seconds = timer.elapsed().as_secs_f64();
        let developed = develop(&means(&framebuffer));
        Ok(judge(id, label, folder, &developed, TEQUILA_REFERENCE, "tequila", ENGINE_BLOCK, ENGINE_LIMIT, Some(seconds), Some(start), String::new()))
    }

    fn cycles(&mut self, id: &str, label: &str, device: &str, facts: &Facts, folder: &Path, progress: &mut dyn FnMut(f32)) -> Result<Outcome, String> {
        if let Err(why) = &facts.cycles {
            return Ok(Outcome::new(id, label, Status::Skip, cycles_reason(why)));
        }
        let Some((colour, start, seconds)) = render_cycles(&self.reference, device, RENDER_SAMPLES, progress)? else {
            return Ok(Outcome::new(id, label, Status::Stopped, ""));
        };
        let note = if tequila_render_cycles::has_curves() { String::new() } else { "this Cycles library predates fibres, so the felt is bare".into() };
        Ok(judge(id, label, folder, &develop(&colour), CYCLES_REFERENCE, "cycles", ENGINE_BLOCK, ENGINE_LIMIT, Some(seconds), Some(start), note))
    }

    fn noisy(&mut self, progress: &mut dyn FnMut(f32)) -> Result<Option<&Noisy>, String> {
        if self.noisy.is_none() {
            let intersector = build_intersector(&self.reference)?;
            let Some(framebuffer) = render_tequila(&self.reference, &intersector, NOISY_SAMPLES, 7, progress) else { return Ok(None) };
            let buffers = framebuffer.to_denoiser_buffers();
            self.noisy = Some(Noisy { color: buffers.color, albedo: buffers.albedo, normal: buffers.normal, developed: develop(&means(&framebuffer)) });
        }
        Ok(self.noisy.as_ref())
    }

    fn denoise(&mut self, id: &str, label: &str, gpu: bool, facts: &Facts, folder: &Path, progress: &mut dyn FnMut(f32)) -> Result<Outcome, String> {
        // The device first: no card to denoise on is a skip, not a render.
        let start = Instant::now();
        let (denoiser, label): (Box<dyn Fn(&Noisy) -> Result<Vec<f32>, String>>, String) = if gpu {
            match GpuDenoiser::shared() {
                Ok(device) => {
                    let label = format!("Denoiser · {}", device.kind());
                    (Box::new(move |n: &Noisy| device.denoise(WIDTH, HEIGHT, &n.color, &n.albedo, &n.normal, settings()).map(|d| d.data)), label)
                }
                Err(why) => {
                    // A card OIDN could use here, and no device on it: that
                    // is worth knowing. Otherwise there is nothing to test.
                    let usable = facts.gpus.iter().any(|g| if cfg!(target_os = "macos") { g.vendor == "Apple" } else { g.vendor == "NVIDIA" || g.vendor == "AMD" });
                    let status = if usable { Status::Fail } else { Status::Skip };
                    return Ok(Outcome::new(id, label, status, why));
                }
            }
        } else {
            let device = Denoiser::shared()?;
            (Box::new(move |n: &Noisy| device.denoise(WIDTH, HEIGHT, &n.color, &n.albedo, &n.normal, settings()).map(|d| d.data)), label.to_string())
        };
        let start = start.elapsed().as_secs_f64();
        let Some(noisy) = self.noisy(progress)? else { return Ok(Outcome::new(id, &label, Status::Stopped, "")) };
        // Once to wake it - OIDN loads its weights and sets its filters up
        // for the size at the first image - and once timed.
        let timer = Instant::now();
        denoiser(noisy)?;
        let start = start + timer.elapsed().as_secs_f64();
        let timer = Instant::now();
        let denoised = denoiser(noisy)?;
        let seconds = timer.elapsed().as_secs_f64();
        let colour: Vec<Spectrum> = denoised.chunks_exact(3).map(|c| Spectrum::from_rendering_rgb(vec3(c[0], c[1], c[2])).sanitized()).collect();
        let developed = develop(&colour);
        let reference = decode(TEQUILA_REFERENCE);
        let before = reference.as_ref().filter(|r| r.0 == WIDTH && r.1 == HEIGHT).map(|r| difference(&noisy.developed, &r.2, WIDTH, HEIGHT, 1));
        let _ = crate::write_png(&folder.join("denoise-input.png").to_string_lossy(), WIDTH, HEIGHT, &noisy.developed, png::BitDepth::Eight);
        let mut outcome = judge(id, &label, folder, &developed, TEQUILA_REFERENCE, "tequila", 1, DENOISE_LIMIT, Some(seconds), Some(start), String::new());
        if let (Some(before), Some(after)) = (before, outcome.error) {
            outcome.note = format!("noisy {before:.2}%");
            if outcome.status == Status::Pass && after > before * DENOISE_GAIN {
                outcome.status = Status::Fail;
                outcome.note = format!("noisy {before:.2}%: hardly any cleaner");
            }
        }
        Ok(outcome)
    }
}

/// How the denoisers are asked to work here: as a final render's.
fn settings() -> DenoiseSettings {
    DenoiseSettings { quality: tequila_denoise::Quality::High, strength: 1.0, prefilter_guides: true }
}

/// Why Cycles' checks are skipped, without the folders it looked in.
fn cycles_reason(why: &str) -> String {
    if why.starts_with("Cycles is not installed") {
        "Cycles is not installed beside this Tequila".into()
    } else {
        scrub(why)
    }
}

fn viewport(id: &str, label: &str, said: Option<&str>) -> Outcome {
    match said {
        None => Outcome::new(id, label, Status::Skip, "checked from the application: Help > Test this computer"),
        Some(text) => match text.split_once(':') {
            Some(("ok", found)) => Outcome::new(id, label, Status::Pass, found.trim()),
            Some(("failed", why)) => Outcome::new(id, label, Status::Fail, why.trim()),
            _ => Outcome::new(id, label, Status::Pass, text.trim()),
        },
    }
}

/// Compare a picture with a reference, keep both and say how it went.
#[allow(clippy::too_many_arguments)]
fn judge(id: &str, label: &str, folder: &Path, developed: &[u8], reference: &[u8], reference_name: &str, block: usize, limit: f64, seconds: Option<f64>, start: Option<f64>, note: String) -> Outcome {
    let _ = crate::write_png(&folder.join(format!("{id}.png")).to_string_lossy(), WIDTH, HEIGHT, developed, png::BitDepth::Eight);
    let reference_file = folder.join(format!("reference-{reference_name}.png"));
    if !reference_file.exists() {
        let _ = std::fs::write(&reference_file, reference);
    }
    let mut outcome = Outcome { id: id.into(), label: label.into(), status: Status::Pass, seconds, start, error: None, limit: Some(limit), note };
    match decode(reference).filter(|r| r.0 == WIDTH && r.1 == HEIGHT) {
        None => {
            outcome.status = Status::Skip;
            outcome.limit = None;
            outcome.note = "no reference picture in this build to compare with".into();
        }
        Some((_, _, expected)) => {
            let error = difference(developed, &expected, WIDTH, HEIGHT, block);
            outcome.error = Some(error);
            if !(error <= limit) {
                outcome.status = Status::Fail;
                let _ = crate::write_png(&folder.join(format!("{id}-difference.png")).to_string_lossy(), WIDTH, HEIGHT, &difference_picture(developed, &expected), png::BitDepth::Eight);
            }
        }
    }
    outcome
}

fn build_intersector(reference: &Reference) -> Result<EmbreeIntersector, String> {
    let device = Device::shared()?;
    EmbreeIntersector::build(&device, reference.scene.clone())
}

/// `samples` of Tequila's renderer, in batches so a cancel is heard; `None`
/// when cancelled.
fn render_tequila(reference: &Reference, intersector: &EmbreeIntersector, samples: u32, seed: u32, progress: &mut dyn FnMut(f32)) -> Option<Framebuffer> {
    let context = RenderContext { camera: reference.camera, config: reference.config.clone(), scene_seed: seed, reject_fireflies: true, ..Default::default() };
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);
    let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT);
    let mut done = 0;
    while done < samples {
        if cancelled() {
            return None;
        }
        let batch = 8.min(samples - done);
        render_pass(intersector, &mut framebuffer, &context, &schedule, batch);
        done += batch;
        progress(done as f32 / samples as f32);
    }
    Some(framebuffer)
}

/// `samples` of Cycles on `device`: the picture in the rendering space, how
/// long the session took to be ready - opened, the scene sent, and one
/// sample rendered, which is when a card loads its kernels - and how long
/// the render took. `None` when cancelled.
fn render_cycles(reference: &Reference, device: &str, samples: u32, progress: &mut dyn FnMut(f32)) -> Result<Option<(Vec<Spectrum>, f64, f64)>, String> {
    use tequila_render_cycles::{translate, Session};
    let start = Instant::now();
    let kernels = std::env::var_os("TEQUILA_CYCLES_KERNELS").map(PathBuf::from);
    let mut session = Session::new(device, 0, kernels.as_deref())?;
    let config = &reference.config;
    let view = translate::View {
        camera: reference.camera,
        width: WIDTH,
        height: HEIGHT,
        transparent: false,
        ground_illumination: config.ground_illumination,
        light_materials: config.light_source_materials,
        light_groups: false,
        shadow_glass: config.shadow_glass(),
        manifold_caustics: config.manifold_caustics(),
    };
    let translation = translate::build(&mut session, &reference.scene, &view)?;
    translate::configure(&mut session, &translate::Quality::with_lighting(false, false, None, config))?;
    session.render_until(WIDTH, HEIGHT, 1, |_, _| {}, Some(&CANCEL))?;
    let start = start.elapsed().as_secs_f64();
    if cancelled() {
        return Ok(None);
    }
    let timer = Instant::now();
    let rgba = session.render_until(WIDTH, HEIGHT, samples, |fraction, _| progress(fraction), Some(&CANCEL))?;
    let seconds = timer.elapsed().as_secs_f64();
    if cancelled() {
        return Ok(None);
    }
    let under = translation.background_under.then(|| translate::background(&reference.scene, &reference.camera, WIDTH, HEIGHT));
    let colour = rgba
        .chunks_exact(4)
        .enumerate()
        .map(|(i, px)| match &under {
            Some(background) => translate::lay_under(px, background[i]),
            None => translate::to_rendering(px[0], px[1], px[2]).sanitized(),
        })
        .collect();
    Ok(Some((colour, start, seconds)))
}

fn means(framebuffer: &Framebuffer) -> Vec<Spectrum> {
    framebuffer.pixels().iter().map(|p| p.mean().sanitized()).collect()
}

/// The picture as it is seen: through the default look, to sRGB bytes.
fn develop(colour: &[Spectrum]) -> Vec<u8> {
    let style = tequila_color::ImageStyle::default();
    style.develop_through(WIDTH, HEIGHT, colour, &tequila_color::DisplayPipeline::Builtin(style.display))
}

/// A PNG's size and RGBA bytes, eight bits a channel.
fn decode(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut reader = png::Decoder::new(bytes).read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.to_vec(),
        png::ColorType::Rgb => pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        _ => return None,
    };
    Some((info.width as usize, info.height as usize, rgba))
}

/// How far two pictures of `width` by `height` are apart: the
/// root-mean-square difference of their colour channels, each first
/// averaged over squares `block` pixels a side, in percent of full scale.
pub fn difference(a: &[u8], b: &[u8], width: usize, height: usize, block: usize) -> f64 {
    let block = block.max(1);
    let (columns, rows) = (width / block, height / block);
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for row in 0..rows {
        for column in 0..columns {
            for channel in 0..3 {
                let (mut x, mut y) = (0.0f64, 0.0f64);
                for dy in 0..block {
                    for dx in 0..block {
                        let i = ((row * block + dy) * width + column * block + dx) * 4 + channel;
                        x += a[i] as f64;
                        y += b[i] as f64;
                    }
                }
                let d = (x - y) / (block * block) as f64 / 255.0;
                sum += d * d;
                count += 1;
            }
        }
    }
    if count == 0 {
        return 0.0;
    }
    (sum / count as f64).sqrt() * 100.0
}

/// Where two pictures differ, four times as bright, for looking at a
/// failure.
fn difference_picture(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.chunks_exact(4)
        .zip(b.chunks_exact(4))
        .flat_map(|(p, q)| {
            let d = |k: usize| ((p[k] as i32 - q[k] as i32).unsigned_abs() * 4).min(255) as u8;
            [d(0), d(1), d(2), 255]
        })
        .collect()
}

/// This build, as a line: its version and source, from the `BUILD.txt` a
/// package carries beside the program (in a Mac bundle, in its Resources).
fn build_line() -> String {
    let note = std::env::current_exe().ok().and_then(|exe| {
        let folder = exe.parent()?.to_path_buf();
        let candidates = [folder.join("BUILD.txt"), folder.parent()?.join("Resources").join("BUILD.txt")];
        candidates.iter().find_map(|p| std::fs::read_to_string(p).ok())
    });
    let field = |text: &str, key: &str| {
        text.trim_start_matches('\u{feff}').lines().find_map(|l| l.strip_prefix(key)).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    };
    match note {
        Some(text) => {
            let version = field(&text, "Version:").unwrap_or_else(|| "packaged build".into());
            match field(&text, "Source:") {
                Some(source) => format!("{version} ({})", source.get(..10).unwrap_or(&source)),
                None => version,
            }
        }
        None => format!("development build {}", env!("CARGO_PKG_VERSION")),
    }
}

/// The facts, a line each, as the report and the terminal show them.
fn fact_lines(facts: &Facts) -> Vec<String> {
    let mut lines = vec![format!("OS       {} · {}", facts.os, facts.arch)];
    let cores = facts.cores.as_ref().map(|c| format!("{c}, ")).unwrap_or_default();
    lines.push(format!("CPU      {} · {cores}{} threads", facts.cpu, facts.threads));
    if let Some(memory) = facts.memory {
        lines.push(format!("Memory   {}", gigabytes(memory)));
    }
    if facts.gpus.is_empty() {
        lines.push("GPU      none found".into());
    }
    for gpu in &facts.gpus {
        let mut parts = vec![gpu.name.clone()];
        if !gpu.name.to_ascii_lowercase().contains(&gpu.vendor.to_ascii_lowercase()) {
            parts.push(gpu.vendor.clone());
        }
        parts.push(gpu.memory.map_or("shared memory".into(), gigabytes));
        if let Some(driver) = &gpu.driver {
            parts.push(format!("driver {driver}"));
        }
        lines.push(format!("GPU      {}", parts.join(" · ")));
    }
    lines.push(format!(
        "Cycles   {}",
        match &facts.cycles {
            Err(why) => cycles_reason(why),
            Ok(devices) => {
                let mut kinds: Vec<String> = Vec::new();
                // The processor first, then the cards as Cycles lists them.
                let processor_first = devices.iter().filter(|d| d.kind == "CPU").chain(devices.iter().filter(|d| d.kind != "CPU"));
                for device in processor_first {
                    let kind = kind_label(&device.kind);
                    if device.kind == "CPU" {
                        if !kinds.contains(&kind) {
                            kinds.push(kind);
                        }
                        continue;
                    }
                    let same: Vec<String> = devices.iter().filter(|d| d.kind == device.kind).map(|d| short(&d.name)).collect();
                    let entry = format!("{kind} ({})", same.join(", "));
                    if !kinds.contains(&entry) {
                        kinds.push(entry);
                    }
                }
                if kinds.is_empty() {
                    "no devices".into()
                } else {
                    kinds.join(", ")
                }
            }
        }
    ));
    lines.into_iter().map(|l| scrub(&l)).collect()
}

fn gigabytes(bytes: u64) -> String {
    let gb = bytes as f64 / (1u64 << 30) as f64;
    if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else {
        format!("{gb:.1} GB")
    }
}

/// The report: a heading, the computer, and a row for each check, in a code
/// block so the columns line up wherever it is pasted. Short enough for one
/// Discord message in the usual case.
pub fn report(facts: &Facts, outcomes: &[Outcome], build: &str, seconds: f64) -> String {
    let count = |status: Status| outcomes.iter().filter(|o| o.status == status).count();
    let mut tally = vec![if outcomes.is_empty() { "the computer only".to_string() } else { format!("{} passed", count(Status::Pass)) }];
    for (status, word) in [(Status::Fail, "failed"), (Status::Crash, "crashed"), (Status::Skip, "skipped"), (Status::Stopped, "stopped")] {
        if count(status) > 0 {
            tally.push(format!("{} {word}", count(status)));
        }
    }
    let mut out = format!("**Tequila self-test** · {} · {} · {}\n```text\n", scrub(build), tally.join(", "), duration(seconds));
    for line in fact_lines(facts) {
        out.push_str(&line);
        out.push('\n');
    }
    if !outcomes.is_empty() {
        let width = outcomes.iter().map(|o| o.label.chars().count()).max().unwrap_or(5).max(5);
        out.push('\n');
        out.push_str(&format!("{:<width$}  {:<6} {:>8}  {}\n", "Check", "Result", "Time", "Error"));
        for outcome in outcomes {
            let time = outcome.seconds.map(duration).unwrap_or_default();
            let error = outcome.error.map(|e| error_text(e, outcome.limit)).unwrap_or_default();
            let row = format!("{:<width$}  {:<6} {:>8}  {}", scrub(&outcome.label), outcome.status.word(), time, error);
            out.push_str(row.trim_end());
            out.push('\n');
            let mut more = Vec::new();
            if let Some(start) = outcome.start.filter(|&s| s >= 1.0) {
                more.push(format!("first use {}", duration(start)));
            }
            if !outcome.note.is_empty() {
                let note: String = scrub(&outcome.note).chars().take(160).collect();
                more.push(note);
            }
            if !more.is_empty() {
                out.push_str(&format!("  {}\n", more.join(" · ")));
            }
        }
    }
    out.push_str("```\n");
    out.push_str(&format!("Tequila's self-test (Help > Test this computer). Share yours on the Tequila Discord: <{DISCORD}>\n"));
    hide_identities(&out, &identities())
}

/// Render the references: Tequila's renderer and Cycles on the processor,
/// converged, into `folder` as `tequila.png` and `cycles.png`, which go into
/// `crates/tequila-cli/self-test/` when the scene or an engine changes what
/// the picture should be.
pub fn make_references(folder: &Path) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let reference = scene::build();
    tequila_materials::energy::prewarm();
    let intersector = build_intersector(&reference)?;
    let timer = Instant::now();
    let mut shown = |f: f32| {
        print!("\r  Tequila {:>3.0}%", f * 100.0);
        let _ = std::io::stdout().flush();
    };
    let framebuffer = render_tequila(&reference, &intersector, REFERENCE_SAMPLES, 0, &mut shown).ok_or("cancelled")?;
    let path = folder.join("tequila.png");
    crate::write_png(&path.to_string_lossy(), WIDTH, HEIGHT, &develop(&means(&framebuffer)), png::BitDepth::Eight)?;
    println!("\nwrote {} in {}", path.display(), duration(timer.elapsed().as_secs_f64()));
    match tequila_render_cycles::available() {
        Err(why) => println!("no Cycles reference: {}", cycles_reason(&why)),
        Ok(()) => {
            let timer = Instant::now();
            let mut shown = |f: f32| {
                print!("\r  Cycles {:>3.0}%", f * 100.0);
                let _ = std::io::stdout().flush();
            };
            let (colour, _, _) = render_cycles(&reference, "CPU", REFERENCE_SAMPLES, &mut shown)?.ok_or("cancelled")?;
            let path = folder.join("cycles.png");
            crate::write_png(&path.to_string_lossy(), WIDTH, HEIGHT, &develop(&colour), png::BitDepth::Eight)?;
            println!("\nwrote {} in {}", path.display(), duration(timer.elapsed().as_secs_f64()));
        }
    }
    Ok(())
}

/// The names that would say whose computer this is: the person's user name,
/// the computer's name and domain, and the home folder.
pub fn identities() -> Vec<String> {
    let mut names = Vec::new();
    for key in ["USERNAME", "USER", "LOGNAME", "COMPUTERNAME", "HOSTNAME", "USERDOMAIN", "USERPROFILE", "HOME"] {
        if let Some(value) = std::env::var_os(key) {
            names.push(value.to_string_lossy().into_owned());
        }
    }
    // The home folder is named for the person, whatever the variables say.
    for key in ["USERPROFILE", "HOME"] {
        if let Some(name) = std::env::var_os(key).and_then(|home| PathBuf::from(home).file_name().map(|n| n.to_string_lossy().into_owned())) {
            names.push(name);
        }
    }
    if let Some(host) = host_name() {
        names.push(host);
    }
    let mut names: Vec<String> = names.into_iter().map(|n| n.trim().to_string()).filter(|n| n.chars().count() >= 3).collect();
    // Longest first, so a home folder goes before the name inside it.
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

#[cfg(unix)]
fn host_name() -> Option<String> {
    extern "C" {
        fn gethostname(name: *mut std::ffi::c_char, length: usize) -> std::ffi::c_int;
    }
    let mut buffer = [0u8; 256];
    // SAFETY: the buffer is as long as it says, and ended by a zero below.
    if unsafe { gethostname(buffer.as_mut_ptr().cast(), buffer.len() - 1) } != 0 {
        return None;
    }
    let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
    let name = String::from_utf8_lossy(&buffer[..end]).into_owned();
    // "name.local": the part before the dot names it too.
    Some(name.split('.').next().unwrap_or(&name).to_string()).filter(|n| !n.is_empty())
}

#[cfg(not(unix))]
fn host_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

/// `text` with every one of `names` in it replaced, whatever its case.
fn hide_identities(text: &str, names: &[String]) -> String {
    let mut out = text.to_string();
    for name in names {
        let wanted = name.to_ascii_lowercase();
        let mut result = String::with_capacity(out.len());
        let mut rest = out.as_str();
        loop {
            match rest.to_ascii_lowercase().find(&wanted) {
                Some(at) => {
                    result.push_str(&rest[..at]);
                    result.push_str("<hidden>");
                    rest = &rest[at + wanted.len()..];
                }
                None => {
                    result.push_str(rest);
                    break;
                }
            }
        }
        out = result;
    }
    out
}

/// A message made fit for the report: the paths in it replaced (a
/// library's own file name kept, which says what was missing), and then
/// any user, computer or home folder name still there.
pub fn scrub(text: &str) -> String {
    hide_identities(&hide_paths(text), &identities())
}

/// `text` with each path in it - `C:\...`, `\\server\...`, `/Users/...`,
/// `~/...` - replaced.
fn hide_paths(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let word_start = |i: usize| i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'"' | b'\'' | b'(' | b'[' | b'=' | b',' | b'`' | b'<');
    while i < bytes.len() {
        let drive = bytes[i].is_ascii_alphabetic() && bytes.get(i + 1) == Some(&b':') && matches!(bytes.get(i + 2), Some(b'\\' | b'/')) && word_start(i);
        let unc = bytes[i] == b'\\' && bytes.get(i + 1) == Some(&b'\\') && word_start(i);
        let home = bytes[i] == b'~' && bytes.get(i + 1) == Some(&b'/') && word_start(i);
        let unix = bytes[i] == b'/' && word_start(i) && bytes.get(i + 1).is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_')) && {
            let end = text[i..].find(|c: char| c.is_whitespace()).map_or(text.len(), |n| i + n);
            text[i + 1..end].contains('/')
        };
        if !(drive || unc || home || unix) {
            let c = text[i..].chars().next().expect("in the text");
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        // To the end of the path: a quote or bracket, a colon or comma and a
        // space after it, or a space whose next word has no separator in it
        // (a folder name may have spaces, a sentence goes on without them).
        let mut end = i;
        let mut chars = text[i..].char_indices().peekable();
        while let Some((offset, c)) = chars.next() {
            let at = i + offset;
            let next = text[at + c.len_utf8()..].chars().next();
            let stop = match c {
                '\n' | '\r' | '\t' | '"' | '\'' | '<' | '>' | '|' | '`' | '*' | '?' | ')' | ']' => true,
                ':' | ',' | ';' => at > i + 1 && next.is_none_or(char::is_whitespace),
                ' ' => {
                    let word: String = text[at + 1..].chars().take_while(|c| !c.is_whitespace()).collect();
                    !(word.contains('\\') || word.contains('/'))
                }
                _ => false,
            };
            if stop {
                break;
            }
            end = at + c.len_utf8();
        }
        let path = text[i..end].trim_end_matches(['.', ',']);
        let end = i + path.len();
        let name = path.rsplit(['\\', '/']).next().unwrap_or("");
        let lower = name.to_ascii_lowercase();
        let library = lower.ends_with(".dll") || lower.ends_with(".dylib") || lower.ends_with(".so") || lower.contains(".so.") || lower.ends_with(".exe");
        if library && name.is_ascii() {
            out.push_str("<path>/");
            out.push_str(name);
        } else {
            out.push_str("<path>");
        }
        i = end.max(i + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_taken_out_of_messages_and_nothing_else_is() {
        let said = r"Cycles is not installed: no tequila_cycles.dll in C:\Users\Someone\AppData\Local\Programs\Tequila\cycles\tequila_cycles.dll, D:\work\build\tequila_cycles.dll";
        assert_eq!(hide_paths(said), "Cycles is not installed: no tequila_cycles.dll in <path>/tequila_cycles.dll, <path>/tequila_cycles.dll");
        assert_eq!(hide_paths("could not open /Users/someone/Library/Logs/x.log: denied"), "could not open <path>: denied");
        assert_eq!(hide_paths(r"kernels in C:\Program Files\Tequila\cycles\lib are missing"), "kernels in <path> are missing");
        assert_eq!(hide_paths(r"\\server\share\folder\file.txt"), "<path>");
        assert_eq!(hide_paths("cache at ~/Library/Caches/Tequila"), "cache at <path>");
        // What is not a path stays: versions, renderer strings, links,
        // sizes and fractions.
        for kept in [
            "NVIDIA GeForce RTX 2060/PCIe/SSE2 · OpenGL 3.3.0 NVIDIA 566.36",
            "driver 566.36 (32.0.15.6636) · 6.0 GB",
            "Share it on <https://discord.gg/Na5T2qgQBX>",
            "pass/fail 1/2 · x86_64 · 4 performance + 4 efficiency cores",
            "hipErrorNoBinaryForGpu: Unable to find code object for all current devices!",
        ] {
            assert_eq!(hide_paths(kept), kept);
        }
    }

    #[test]
    fn names_are_hidden_whatever_their_case() {
        let names = vec!["DESKTOP-AB12CD".to_string(), "Someone".to_string()];
        assert_eq!(hide_identities("on desktop-ab12cd as SOMEONE and someone", &names), "on <hidden> as <hidden> and <hidden>");
    }

    #[test]
    fn the_difference_is_zero_for_the_same_picture_and_grows_with_a_change() {
        let a: Vec<u8> = (0..16 * 8).flat_map(|i| [(i * 2) as u8, 100, 50, 255]).collect();
        assert_eq!(difference(&a, &a, 16, 8, 4), 0.0);
        // Every channel 51 out (20% of full scale): 20% at any block size.
        let b: Vec<u8> = a.chunks_exact(4).flat_map(|p| [p[0].saturating_add(51), p[1] + 51, p[2] + 51, 255]).collect();
        assert!((difference(&a, &b, 16, 8, 1) - 20.0).abs() < 0.01);
        assert!((difference(&a, &b, 16, 8, 4) - 20.0).abs() < 0.01);
        // Noise that averages out within a block nearly vanishes at 4.
        let noisy: Vec<u8> = a.chunks_exact(4).enumerate().flat_map(|(i, p)| {
            let d = if (i + i / 16) % 2 == 0 { 20 } else { -20i32 };
            [p[0], (p[1] as i32 + d) as u8, p[2], 255]
        }).collect();
        assert!(difference(&a, &noisy, 16, 8, 1) > 4.0);
        assert!(difference(&a, &noisy, 16, 8, 4) < 0.01);
    }

    #[test]
    fn a_run_is_picked_up_where_it_stopped() {
        let done = Outcome { seconds: Some(1.25), error: Some(0.75), limit: Some(3.0), ..Outcome::new("tequila-cpu", "Tequila · processor", Status::Pass, "") };
        let text = format!("started\tfacts\noutcome\tfacts\tpass\t-\t-\t-\t-\tfacts\t\nstarted\ttequila-cpu\n{}\nstarted\tcycles-hip\n", done.to_line());
        let (outcomes, crashed) = read_progress(&text);
        assert_eq!(outcomes.iter().find(|o| o.id == "tequila-cpu"), Some(&done));
        assert_eq!(crashed.as_deref(), Some("cycles-hip"));
        let (_, crashed) = read_progress(&format!("started\ttequila-cpu\n{}\n", done.to_line()));
        assert_eq!(crashed, None);
    }

    /// The checks a computer gets: each kind of card once, each card alone
    /// where a kind has several, and those together.
    #[test]
    fn several_cards_are_each_tested_and_then_together() {
        let card = |kind: &str, name: &str, id: &str| system::Device { kind: kind.into(), name: name.into(), id: id.into() };
        let facts = Facts {
            cycles: Ok(vec![
                card("CPU", "AMD Ryzen 9 7950X", "CPU"),
                card("HIP", "AMD Radeon RX 7900 XTX", "HIP_0"),
                card("HIP", "AMD Radeon RX 7900 XTX", "HIP_1"),
                card("CUDA", "NVIDIA GeForce RTX 4090", "CUDA_0"),
                card("OPTIX", "NVIDIA GeForce RTX 4090", "OPTIX_0"),
            ]),
            ..Facts::default()
        };
        let ids: Vec<String> = plan(&facts, &Settings::default()).into_iter().map(|p| p.id).collect();
        assert_eq!(ids, ["tequila-cpu", "denoise-cpu", "cycles-cpu", "cycles-optix", "cycles-cuda", "cycles-hip-1", "cycles-hip-2", "cycles-hip-all", "denoise-gpu", "viewport"]);
        let checks = plan(&facts, &Settings::default());
        assert_eq!(checks[5].work, Work::Cycles("HIP_0".into()));
        assert_eq!(checks[7].work, Work::Cycles("HIP".into()));
        assert_eq!(checks[7].label, "Cycles · HIP · all 2 cards");
        let cpu: Vec<String> = plan(&facts, &Settings { cpu_only: true, ..Settings::default() }).into_iter().map(|p| p.id).collect();
        assert_eq!(cpu, ["tequila-cpu", "denoise-cpu", "cycles-cpu", "viewport"]);
    }

    /// The report says nothing about whose computer it is, even when what
    /// it was given does: a message holding the home folder's path, the
    /// user's and the computer's names. Built from this computer's own
    /// facts, so whatever they hold is checked too.
    #[test]
    fn the_report_names_no_one() {
        let facts = system::gather_with(true);
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| "/home/someone".into());
        let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
        let computer = host_name().unwrap_or_default();
        let separator = std::path::MAIN_SEPARATOR;
        let outcomes = vec![
            Outcome::new("cycles-hip", "Cycles · HIP", Status::Fail, format!("could not load {home}{separator}AppData{separator}tequila_cycles.dll: not found")),
            Outcome::new("viewport", "Viewport", Status::Fail, format!("{user} on {computer} failed")),
        ];
        let exe = std::env::current_exe().unwrap();
        let mut outcomes = outcomes;
        outcomes.push(viewport("viewport-2", "Viewport", Some(&format!("failed: shader log in {}", exe.display()))));
        let text = report(&facts, &outcomes, "test build", 61.0);
        println!("{text}");
        let lower = text.to_lowercase();
        let mut secrets = identities();
        secrets.extend([home.clone(), exe.display().to_string(), std::env::temp_dir().display().to_string(), default_folder().display().to_string()]);
        if let Ok(here) = std::env::current_dir() {
            secrets.push(here.display().to_string());
        }
        for secret in secrets.iter().filter(|s| s.chars().count() >= 3) {
            assert!(!lower.contains(&secret.to_lowercase()), "the report holds {secret:?}");
        }
        assert!(!lower.contains("serial"), "a serial number");
        assert!(!text.contains(":\\") && !text.contains("/Users/") && !text.contains("/home/"), "a path");
        // The address this computer reaches the internet from - found
        // without sending anything, as a socket is only pointed at a
        // documentation address - is not in it.
        if let Some(address) = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect("192.0.2.1:9").and_then(|_| s.local_addr())).ok().map(|a| a.ip().to_string()) {
            if address != "0.0.0.0" {
                assert!(!text.contains(&address), "the report holds {address}");
            }
        }
        assert!(text.contains(DISCORD), "where to post it");
        assert!(text.chars().count() < 1900, "one Discord message: {} characters", text.chars().count());
    }
}
