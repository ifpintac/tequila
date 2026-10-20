//! Crash reports a person can see, read and pass on.
//!
//! A session can end badly three ways, and each leaves something:
//! - **A panic**: Tequila's own code met something it could not go on from.
//!   The hook writes the whole story before anything else happens - the
//!   message, where, the call stack, and what the person had just done.
//! - **A native crash**: something under Tequila faulted, a graphics driver
//!   or Cycles. On Windows a last-chance handler writes the exception and
//!   the module it was in. On a Mac, macOS writes its own report, and the
//!   next launch reads it.
//! - **Neither**: the window was ended, the computer slept or lost power.
//!   The session's marker is left behind, and so is the activity it noted,
//!   which is often enough.
//!
//! Every session holds a locked marker in the crash folder while it runs, as
//! the journal does. The next launch finds the markers nobody holds, gathers
//! what each left into one report, keeps it, and shows it - with a button
//! that copies a short version for pasting into Discord.

use crate::clock;
use std::collections::VecDeque;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// What a report keeps of the session's activity.
const ACTIVITY_LINES: usize = 40;
/// A Discord message's limit is 2000 characters; a little room is left.
const DISCORD_LIMIT: usize = 1900;
/// Reports kept in the folder, newest first.
const KEPT_REPORTS: usize = 20;

/// One report, ready to show.
#[derive(Clone, Debug)]
pub struct Report {
    pub path: PathBuf,
    /// What happened, in a line.
    pub title: String,
    /// The whole report.
    pub text: String,
}

impl Report {
    /// A short version that fits one Discord message, in a code block so it
    /// keeps its lines: the header, what happened, the top of the call
    /// stack and the last few things done.
    pub fn for_discord(&self) -> String {
        discord_summary(&self.text)
    }
}

struct Session {
    folder: PathBuf,
    id: String,
    /// What every report of this session starts with.
    header: String,
    activity: Mutex<VecDeque<String>>,
    marker: Mutex<Option<File>>,
    /// Panics this session recovered from, waiting to be shown.
    fresh: Mutex<Vec<Report>>,
}

static SESSION: OnceLock<Session> = OnceLock::new();
static GRAPHICS: OnceLock<String> = OnceLock::new();

/// Where reports are kept: `.tequila/crashes` in the person's home.
pub fn folder() -> PathBuf {
    if let Some(folder) = std::env::var_os("TEQUILA_CRASH_DIR") {
        return folder.into();
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".tequila").join("crashes")
}

/// Start reporting for this session: the earlier sessions' reports come
/// back, to be shown.
pub fn install(build: &str) -> Vec<Report> {
    let folder = folder();
    let _ = std::fs::create_dir_all(&folder);
    let earlier = gather(&folder);
    let id = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
    let header = format!("Tequila: {build}\nSystem: {}\nStarted: {}\n", system(), clock::stamp());
    let marker = File::create(folder.join(format!("{id}.running"))).ok().filter(|file| file.try_lock().is_ok());
    if let Some(mut file) = marker.as_ref() {
        use std::io::Write;
        let _ = file.write_all(header.as_bytes());
    }
    let session = Session {
        folder,
        id,
        header,
        activity: Mutex::new(VecDeque::new()),
        marker: Mutex::new(marker),
        fresh: Mutex::new(Vec::new()),
    };
    if SESSION.set(session).is_ok() {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !tequila_core::guard::expecting() {
                on_panic(info);
            }
            default(info);
        }));
        #[cfg(windows)]
        native::install();
    }
    earlier
}

/// The graphics card and driver, once the window has them.
pub fn set_graphics(description: String) {
    if let Some(session) = SESSION.get() {
        if let Ok(marker) = session.marker.lock() {
            if let Some(mut file) = marker.as_ref() {
                use std::io::Write;
                let _ = file.write_all(format!("Graphics: {description}\n").as_bytes());
            }
        }
    }
    let _ = GRAPHICS.set(description);
}

/// Something the person did or saw, kept for a report: an import, a render,
/// a change of view. Cheap enough to call on every status message.
pub fn note(line: &str) {
    let Some(session) = SESSION.get() else { return };
    let line: String = line.lines().next().unwrap_or("").chars().take(160).collect();
    if line.trim().is_empty() {
        return;
    }
    let Ok(mut activity) = session.activity.lock() else { return };
    if activity.back().is_some_and(|last| last.ends_with(&line)) {
        return;
    }
    activity.push_back(format!("{} {line}", clock::time()));
    while activity.len() > ACTIVITY_LINES {
        activity.pop_front();
    }
    // On disk too, for a crash that gives the hook no chance to run.
    let text: String = activity.iter().map(|l| format!("{l}\n")).collect();
    let _ = std::fs::write(session.folder.join(format!("{}.activity", session.id)), text);
}

/// The session ended as it should: nothing to report next time.
pub fn finish() {
    let Some(session) = SESSION.get() else { return };
    if let Ok(mut marker) = session.marker.lock() {
        marker.take();
    }
    for suffix in ["running", "activity", "native", "panic"] {
        let _ = std::fs::remove_file(session.folder.join(format!("{}.{suffix}", session.id)));
    }
}

/// Crash on purpose, as `TEQUILA_CRASH_TEST` asks: `panic`, `worker` (a
/// panic in another thread, which the window survives) or `native` (an
/// access violation). For testing the reports; nothing without it.
pub fn test_crash(ctx: &egui::Context) {
    static STARTED: OnceLock<std::time::Instant> = OnceLock::new();
    static FIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(kind) = std::env::var_os("TEQUILA_CRASH_TEST") else { return };
    // Five seconds in, with the window up and drawing.
    if STARTED.get_or_init(std::time::Instant::now).elapsed().as_secs_f32() < 5.0 {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        return;
    }
    if FIRED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    match kind.to_str() {
        Some("panic") => panic!("a crash asked for by TEQUILA_CRASH_TEST"),
        Some("worker") => {
            let _ = std::thread::Builder::new().name("crash test".into()).spawn(|| panic!("a worker's crash asked for by TEQUILA_CRASH_TEST"));
        }
        Some("native") => {
            note("Crashing on purpose (TEQUILA_CRASH_TEST=native)");
            // SAFETY: not safe at all; that is the point.
            unsafe { std::ptr::write_volatile(std::ptr::null_mut::<u32>().wrapping_add(4), 1) };
        }
        _ => {}
    }
}

/// Panics this session got past - in a worker, not the window - each to be
/// shown once.
pub fn take_fresh() -> Vec<Report> {
    SESSION.get().and_then(|s| s.fresh.lock().ok().map(|mut fresh| std::mem::take(&mut *fresh))).unwrap_or_default()
}

/// Every kept report, newest first.
pub fn kept() -> Vec<Report> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt")))
        .collect();
    files.sort();
    files.reverse();
    files.into_iter().filter_map(|path| read_report(&path)).collect()
}

fn read_report(path: &Path) -> Option<Report> {
    let text = std::fs::read_to_string(path).ok()?;
    let title = text.lines().find_map(|l| l.strip_prefix("What happened: ")).unwrap_or("Tequila stopped").to_string();
    Some(Report { path: path.to_path_buf(), title, text })
}

/// The reports of earlier sessions that ended without finishing, each
/// written as a kept report and its session's files cleared away.
fn gather(folder: &Path) -> Vec<Report> {
    let mut reports = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder) else { return reports };
    let mut ended: Vec<(String, std::time::SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".running")).map(str::to_string) else {
            continue;
        };
        // Held: that Tequila is still open.
        let Ok(file) = std::fs::OpenOptions::new().read(true).write(true).open(&path) else { continue };
        if matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
            continue;
        }
        let started = entry.metadata().and_then(|m| m.created().or_else(|_| m.modified())).unwrap_or(std::time::UNIX_EPOCH);
        drop(file);
        ended.push((id, started));
    }
    ended.sort_by_key(|(_, started)| *started);
    for (id, started) in ended {
        let read = |suffix: &str| std::fs::read_to_string(folder.join(format!("{id}.{suffix}"))).ok();
        let header = read("running").unwrap_or_default();
        let panic = read("panic");
        let native = read("native");
        let mac = mac_report(started);
        let activity = read("activity").unwrap_or_default();
        let title = if let Some(panic) = &panic {
            panic.lines().find_map(|l| l.strip_prefix("What happened: ")).unwrap_or("Tequila stopped on an error").to_string()
        } else if let Some(native) = native.as_deref().or(mac.as_deref()) {
            native.lines().find_map(|l| l.strip_prefix("What happened: ")).unwrap_or("Tequila crashed").to_string()
        } else {
            "Tequila closed without a chance to say why".to_string()
        };
        let mut text = format!("Tequila crash report\nWhat happened: {title}\n{header}");
        if let Some(panic) = &panic {
            text.push_str(&format!("\n{}\n", strip_title(panic)));
        }
        if let Some(native) = &native {
            text.push_str(&format!("\n{}\n", strip_title(native)));
        }
        if let Some(mac) = &mac {
            text.push_str(&format!("\n{}\n", strip_title(mac)));
        }
        if panic.is_none() && native.is_none() && mac.is_none() {
            text.push_str(
                "\nNothing was written as it ended: the window was ended from outside, the computer slept or lost power, \
                 or a graphics driver stopped it before Tequila could say.\n",
            );
        }
        if !activity.trim().is_empty() {
            text.push_str(&format!("\nWhat was happening, oldest first:\n{activity}"));
        }
        let name = format!("crash-{}-{}.txt", clock::date(), clock::time_for_filename());
        let mut path = folder.join(&name);
        let mut n = 1;
        while path.exists() {
            n += 1;
            path = folder.join(name.replace(".txt", &format!("-{n}.txt")));
        }
        if std::fs::write(&path, &text).is_ok() {
            for suffix in ["running", "activity", "native", "panic"] {
                let _ = std::fs::remove_file(folder.join(format!("{id}.{suffix}")));
            }
        }
        reports.push(Report { path, title, text });
    }
    prune(folder);
    reports
}

fn strip_title(text: &str) -> String {
    text.lines().filter(|l| !l.starts_with("What happened: ")).collect::<Vec<_>>().join("\n").trim().to_string()
}

fn prune(folder: &Path) {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt")))
        .collect();
    files.sort();
    let surplus = files.len().saturating_sub(KEPT_REPORTS);
    for old in files.into_iter().take(surplus) {
        let _ = std::fs::remove_file(old);
    }
}

fn on_panic(info: &std::panic::PanicHookInfo) {
    let Some(session) = SESSION.get() else { return };
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".to_string());
    let thread = std::thread::current();
    let thread = thread.name().unwrap_or("unnamed").to_string();
    let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
    let stack = tidy_backtrace(&std::backtrace::Backtrace::force_capture().to_string());
    let title = format!("an error in Tequila's {thread} thread: {}", message.lines().next().unwrap_or(""));
    let text = format!("What happened: {title}\nMessage: {message}\nWhere: {}\nAt: {}\n\nCall stack, innermost first:\n{stack}", short_path(&place), clock::stamp());
    let _ = std::fs::write(session.folder.join(format!("{}.panic", session.id)), &text);
    // A worker's panic leaves the window open: say so now, as well as next
    // time if the session does not survive it. Saved now, in case it does.
    if thread != "main" {
        let activity: String = session.activity.try_lock().map(|a| a.iter().map(|l| format!("{l}\n")).collect()).unwrap_or_default();
        let graphics = GRAPHICS.get().map(|g| format!("Graphics: {g}\n")).unwrap_or_default();
        let full = format!(
            "Tequila crash report\nWhat happened: {title}\n{}{graphics}\n{}\n\nWhat was happening, oldest first:\n{activity}",
            session.header,
            strip_title(&text)
        );
        let path = session.folder.join(format!("crash-{}-{}.txt", clock::date(), clock::time_for_filename()));
        let _ = std::fs::write(&path, &full);
        if let Ok(mut fresh) = session.fresh.try_lock() {
            fresh.push(Report { path, title, text: full });
        }
    }
}

/// The call stack without the frames every panic has - the hook, the
/// standard library's unwinding - and, where the program's symbols are not
/// with it, each address as its module and offset, which the build's
/// symbols turn back into names.
fn tidy_backtrace(raw: &str) -> String {
    let mut out = Vec::new();
    let mut started = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        // Frames start "N: name"; the "at file:line" lines follow theirs.
        let is_frame = trimmed.split_once(':').is_some_and(|(n, _)| n.chars().all(|c| c.is_ascii_digit()));
        if is_frame {
            let name = trimmed.split_once(':').map(|(_, rest)| rest.trim()).unwrap_or("");
            let plumbing = ["std::", "core::", "alloc::", "rust_begin_unwind", "__rust", "tequila_app::crash::on_panic", "tequila_app::crash::install", "<alloc::boxed::Box", "BaseThreadInitThunk", "RtlUserThreadStart"]
                .iter()
                .any(|p| name.starts_with(p));
            if !started && plumbing {
                continue;
            }
            started = true;
            out.push(format!("  {}", relative_address(name)));
        } else if started && trimmed.starts_with("at ") {
            out.push(format!("      at {}", short_path(&trimmed[3..])));
        }
        if out.len() > 80 {
            break;
        }
    }
    if out.is_empty() {
        return raw.to_string();
    }
    out.join("\n")
}

/// A source path as it reads in Tequila's code or a library's, without the
/// folders of the computer that built it.
fn short_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    for marker in ["/freeshot/", "/tequila/"] {
        if let Some(at) = path.find(marker) {
            return path[at + marker.len()..].to_string();
        }
    }
    // A library's: from its crate's name on.
    if let Some(at) = path.find("/registry/src/") {
        let rest = &path[at + "/registry/src/".len()..];
        return rest.split_once('/').map(|(_, crate_path)| crate_path.to_string()).unwrap_or_else(|| rest.to_string());
    }
    if let Some(at) = path.find("/rustc/") {
        return path[at + 1..].splitn(3, '/').nth(2).map(|p| format!("rust/{p}")).unwrap_or(path.clone());
    }
    path
}

/// `0x7ff6...` as `Tequila.exe+0x1234`, where the address is the program's.
fn relative_address(name: &str) -> String {
    let Some(hex) = name.strip_prefix("0x").map(|h| h.split(|c: char| !c.is_ascii_hexdigit()).next().unwrap_or("")) else {
        return name.to_string();
    };
    let Ok(address) = usize::from_str_radix(hex, 16) else { return name.to_string() };
    match module_of(address) {
        Some((module, base)) => format!("{module}+0x{:x}", address - base),
        None => name.to_string(),
    }
}

#[cfg(windows)]
fn module_of(address: usize) -> Option<(String, usize)> {
    native::module_of(address)
}

#[cfg(not(windows))]
fn module_of(_address: usize) -> Option<(String, usize)> {
    None
}

/// The operating system, its version, the processor and how many threads.
fn system() -> String {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    format!("{} · {} · {threads} threads", os_version(), std::env::consts::ARCH)
}

#[cfg(windows)]
fn os_version() -> String {
    native::os_version()
}

#[cfg(target_os = "macos")]
fn os_version() -> String {
    let plist = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").unwrap_or_default();
    let version = plist
        .split("<key>ProductVersion</key>")
        .nth(1)
        .and_then(|rest| rest.split("<string>").nth(1))
        .and_then(|rest| rest.split("</string>").next())
        .unwrap_or("");
    format!("macOS {version}")
}

#[cfg(not(any(windows, target_os = "macos")))]
fn os_version() -> String {
    std::env::consts::OS.to_string()
}

/// What macOS wrote of a crash since `since`: the exception and the
/// crashed thread's stack, from the newest Tequila `.ips` report.
fn mac_report(since: std::time::SystemTime) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let reports = home.join("Library").join("Logs").join("DiagnosticReports");
    let newest = std::fs::read_dir(reports)
        .ok()?
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.starts_with("tequila") && name.ends_with(".ips")
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .filter(|(modified, _)| *modified >= since)
        .max_by_key(|(modified, _)| *modified)?
        .1;
    read_ips(&std::fs::read_to_string(&newest).ok()?, &newest)
}

/// The useful part of a macOS crash report: its first line is a header, the
/// rest one JSON object.
fn read_ips(text: &str, path: &Path) -> Option<String> {
    use tequila_format::json::{parse, Value};
    let body = text.split_once('\n').map(|(_, rest)| rest)?;
    let report = parse(body.as_bytes()).ok()?;
    let field = |v: Option<&Value>, key: &str| v.and_then(|v| v.get(key)).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let exception = report.get("exception");
    let kind = field(exception, "type");
    let signal = field(exception, "signal");
    let subtype = field(exception, "subtype");
    let images = report.get("usedImages").map(|v| v.as_array()).unwrap_or(&[]);
    let faulting = report.get("faultingThread").and_then(|v| v.as_index()).unwrap_or(0);
    let threads = report.get("threads").map(|v| v.as_array()).unwrap_or(&[]);
    let frames = threads.get(faulting).and_then(|t| t.get("frames")).map(|v| v.as_array()).unwrap_or(&[]);
    let mut out = format!("What happened: macOS stopped Tequila: {kind} ({signal}) {subtype}\nmacOS's own report: {}\n\nCrashed thread, innermost first:\n", path.display());
    for frame in frames.iter().take(40) {
        let image = frame
            .get("imageIndex")
            .and_then(|v| v.as_index())
            .and_then(|i| images.get(i))
            .and_then(|i| i.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let symbol = frame.get("symbol").and_then(|v| v.as_str());
        let offset = frame.get("imageOffset").and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
        match symbol {
            Some(symbol) => out.push_str(&format!("  {image}  {symbol}\n")),
            None => out.push_str(&format!("  {image}+0x{offset:x}\n")),
        }
    }
    Some(out)
}

/// The report cut to one Discord message.
fn discord_summary(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let pick = |prefix: &str| lines.iter().find_map(|l| l.strip_prefix(prefix)).unwrap_or("").trim().to_string();
    let mut head = format!("Tequila crash: {}\n{} · {}", pick("What happened: "), pick("Tequila: "), pick("System: "));
    let graphics = pick("Graphics: ");
    if !graphics.is_empty() {
        head.push_str(&format!(" · {graphics}"));
    }
    // The stack's first frames, then the last things done.
    let mut stack = Vec::new();
    let mut in_stack = false;
    for line in &lines {
        if line.starts_with("Call stack") || line.starts_with("Crashed thread") || line.starts_with("Exception") || line.starts_with("At module") {
            in_stack = true;
            if !line.starts_with("Call stack") && !line.starts_with("Crashed thread") {
                stack.push(line.trim().to_string());
            }
            continue;
        }
        if in_stack {
            if line.trim().is_empty() || line.starts_with("What was happening") {
                in_stack = false;
                continue;
            }
            if !line.trim_start().starts_with("at ") {
                stack.push(line.trim().to_string());
            }
        }
    }
    let activity: Vec<&str> = lines.iter().skip_while(|l| !l.starts_with("What was happening")).skip(1).copied().collect();
    let message = pick("Message: ");
    let place = pick("Where: ");
    let build = |frames: usize, done: usize| {
        let mut body = String::new();
        if !message.is_empty() {
            body.push_str(&format!("{message}\n"));
        }
        if !place.is_empty() {
            body.push_str(&format!("at {place}\n"));
        }
        for frame in stack.iter().take(frames) {
            body.push_str(&format!("{frame}\n"));
        }
        if !activity.is_empty() && done > 0 {
            body.push_str("Last done:\n");
            for line in activity.iter().rev().take(done).collect::<Vec<_>>().into_iter().rev() {
                body.push_str(&format!("{line}\n"));
            }
        }
        format!("{head}\n```\n{}```", body)
    };
    for (frames, done) in [(12, 8), (8, 6), (6, 4), (4, 3), (3, 0)] {
        let text = build(frames, done);
        if text.chars().count() <= DISCORD_LIMIT {
            return text;
        }
    }
    let text = build(2, 0);
    let cut: String = text.chars().take(DISCORD_LIMIT - 4).collect();
    format!("{cut}\n```")
}

/// The window that shows reports: after a crash, on the next launch; after
/// a worker's error, at once; and from Help.
#[derive(Default)]
pub struct CrashWindow {
    pub reports: Vec<Report>,
    pub index: usize,
    pub open: bool,
    /// Shown from Help rather than after a crash: the title says so.
    pub browsing: bool,
    copied: Option<std::time::Instant>,
}

impl CrashWindow {
    pub fn show_after_crash(&mut self, reports: Vec<Report>) {
        if reports.is_empty() {
            return;
        }
        self.reports = reports;
        self.index = self.reports.len() - 1;
        self.browsing = false;
        self.open = true;
    }

    pub fn browse(&mut self) {
        self.reports = kept();
        self.index = 0;
        self.browsing = true;
        self.open = true;
    }

    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        let title = if self.browsing { "Crash reports" } else { "Tequila closed unexpectedly" };
        let mut open = self.open;
        egui::Window::new(title)
            .open(&mut open)
            .collapsible(false)
            .default_width(620.0)
            .default_height(460.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let p = crate::theme::p();
                let Some(report) = self.reports.get(self.index).cloned() else {
                    ui.label("No crash reports. When Tequila stops unexpectedly, what happened is kept here.");
                    ui.add_space(6.0);
                    if ui.button("Open the folder").clicked() {
                        let _ = std::fs::create_dir_all(folder());
                        crate::library_tab::open_folder(&folder());
                    }
                    return;
                };
                if !self.browsing {
                    ui.label("Sorry about that. Here is what Tequila saw. Copy it for Discord and post it in the bug-reports channel, and we can fix it.");
                    ui.add_space(4.0);
                }
                if self.reports.len() > 1 {
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.index + 1 < self.reports.len(), egui::Button::new("◀ Older")).clicked() {
                            self.index += 1;
                        }
                        ui.label(egui::RichText::new(format!("{} of {}", self.index + 1, self.reports.len())).color(p.text_dim));
                        if ui.add_enabled(self.index > 0, egui::Button::new("Newer ▶")).clicked() {
                            self.index -= 1;
                        }
                    });
                }
                ui.label(egui::RichText::new(&report.title).strong());
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                    let mut text = report.text.as_str();
                    ui.add(egui::TextEdit::multiline(&mut text).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Copy for Discord").on_hover_text("A short version that fits one message").clicked() {
                        ui.ctx().copy_text(report.for_discord());
                        self.copied = Some(std::time::Instant::now());
                    }
                    if ui.button("Copy the whole report").clicked() {
                        ui.ctx().copy_text(format!("```\n{}\n```", report.text));
                        self.copied = Some(std::time::Instant::now());
                    }
                    if ui.button("Show the file").clicked() {
                        crate::library_tab::open_folder(report.path.parent().unwrap_or(Path::new(".")));
                    }
                    if self.copied.is_some_and(|t| t.elapsed().as_secs_f32() < 3.0) {
                        ui.label(egui::RichText::new("Copied").color(p.accent));
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                    }
                });
            });
        self.open = open;
    }
}

#[cfg(windows)]
mod native {
    //! Windows' last chance: an exception nothing handled, written down with
    //! nothing but the stack - no allocation, no locks - since the process
    //! is in whatever state the fault left it.

    use std::cell::UnsafeCell;
    use std::ffi::c_void;

    struct Fixed<T>(UnsafeCell<T>);
    unsafe impl<T> Sync for Fixed<T> {}

    /// The report's path, UTF-16 and ended by a zero.
    static PATH: Fixed<[u16; 1024]> = Fixed(UnsafeCell::new([0; 1024]));

    #[repr(C)]
    struct ExceptionRecord {
        code: u32,
        flags: u32,
        record: *mut ExceptionRecord,
        address: *mut c_void,
        parameters: u32,
        information: [usize; 15],
    }

    #[repr(C)]
    struct ExceptionPointers {
        record: *mut ExceptionRecord,
        context: *mut c_void,
    }

    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }

    type Filter = unsafe extern "system" fn(*mut ExceptionPointers) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetUnhandledExceptionFilter(filter: Option<Filter>) -> Option<Filter>;
        fn CreateFileW(name: *const u16, access: u32, share: u32, security: *mut c_void, disposition: u32, flags: u32, template: *mut c_void) -> isize;
        fn WriteFile(file: isize, buffer: *const u8, length: u32, written: *mut u32, overlapped: *mut c_void) -> i32;
        fn CloseHandle(handle: isize) -> i32;
        fn GetModuleHandleExW(flags: u32, address: *const u16, module: *mut *mut c_void) -> i32;
        fn GetModuleFileNameW(module: *mut c_void, name: *mut u16, size: u32) -> u32;
    }

    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }

    const FROM_ADDRESS: u32 = 0x4;
    const UNCHANGED_REFCOUNT: u32 = 0x2;

    pub fn install() {
        let Some(session) = super::SESSION.get() else { return };
        let path = session.folder.join(format!("{}.native", session.id));
        let wide: Vec<u16> = path.as_os_str().encode_wide_lossy();
        if wide.len() >= 1023 {
            return;
        }
        // SAFETY: written once, here, before the filter that reads it is
        // installed.
        unsafe {
            std::ptr::copy_nonoverlapping(wide.as_ptr(), PATH.0.get().cast::<u16>(), wide.len());
            SetUnhandledExceptionFilter(Some(filter));
        }
    }

    trait WideLossy {
        fn encode_wide_lossy(&self) -> Vec<u16>;
    }
    impl WideLossy for std::ffi::OsStr {
        fn encode_wide_lossy(&self) -> Vec<u16> {
            use std::os::windows::ffi::OsStrExt;
            self.encode_wide().chain(std::iter::once(0)).collect()
        }
    }

    /// A buffer on the stack, written to as text.
    struct Line {
        bytes: [u8; 2048],
        len: usize,
    }

    impl Line {
        fn text(&mut self, s: &[u8]) {
            for &b in s {
                if self.len < self.bytes.len() {
                    self.bytes[self.len] = b;
                    self.len += 1;
                }
            }
        }

        fn hex(&mut self, mut v: usize) {
            let mut digits = [0u8; 16];
            let mut n = 0;
            loop {
                digits[n] = b"0123456789abcdef"[v & 15];
                n += 1;
                v >>= 4;
                if v == 0 {
                    break;
                }
            }
            self.text(b"0x");
            while n > 0 {
                n -= 1;
                self.text(&digits[n..n + 1]);
            }
        }

        fn wide(&mut self, s: &[u16]) {
            for &c in s {
                if c == 0 {
                    break;
                }
                self.text(&[if c < 128 { c as u8 } else { b'?' }]);
            }
        }
    }

    fn describe(code: u32) -> &'static [u8] {
        match code {
            0xC000_0005 => b"an access violation: memory read or written that was not there",
            0xC000_00FD => b"a stack overflow",
            0xC000_001D => b"an illegal instruction",
            0xC000_0094 => b"an integer divided by zero",
            0xC000_0409 => b"a stack buffer overrun, or a deliberate abort",
            0xC000_0374 => b"a corrupted heap",
            0xC000_0006 => b"a page that could not be read in",
            0xE06D_7363 => b"a C++ exception nothing caught",
            _ => b"an exception",
        }
    }

    unsafe extern "system" fn filter(pointers: *mut ExceptionPointers) -> i32 {
        let mut line = Line { bytes: [0; 2048], len: 0 };
        let record = if pointers.is_null() { std::ptr::null_mut() } else { (*pointers).record };
        let (code, address) = if record.is_null() { (0, 0) } else { ((*record).code, (*record).address as usize) };
        line.text(b"What happened: Tequila crashed: ");
        line.text(describe(code));
        line.text(b"\n");
        line.text(b"Exception: ");
        line.hex(code as usize);
        line.text(b" at ");
        line.hex(address);
        line.text(b"\n");
        let mut module = std::ptr::null_mut();
        if GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, address as *const u16, &mut module) != 0 {
            let mut name = [0u16; 520];
            let length = GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize;
            let file = name[..length].rsplit(|&c| c == b'\\' as u16).next().unwrap_or(&name[..length]);
            line.text(b"At module: ");
            line.wide(file);
            line.text(b"+");
            line.hex(address.wrapping_sub(module as usize));
            line.text(b"\n");
        }
        if code == 0xC000_0005 && !record.is_null() && (*record).parameters >= 2 {
            let what = (*record).information[0];
            line.text(match what {
                0 => b"Reading ".as_slice(),
                1 => b"Writing ".as_slice(),
                _ => b"Running ".as_slice(),
            });
            line.hex((*record).information[1]);
            line.text(b"\n");
        }
        let file = CreateFileW(PATH.0.get().cast::<u16>(), 0x4000_0000, 0, std::ptr::null_mut(), 2, 0x80, std::ptr::null_mut());
        if file != -1 {
            let mut written = 0u32;
            WriteFile(file, line.bytes.as_ptr(), line.len as u32, &mut written, std::ptr::null_mut());
            CloseHandle(file);
        }
        // On to Windows' own handling, which ends the process.
        0
    }

    /// The module holding `address`, and where it starts.
    pub fn module_of(address: usize) -> Option<(String, usize)> {
        let mut module = std::ptr::null_mut();
        // SAFETY: the address is only looked up, not read.
        unsafe {
            if GetModuleHandleExW(FROM_ADDRESS | UNCHANGED_REFCOUNT, address as *const u16, &mut module) == 0 {
                return None;
            }
            let mut name = [0u16; 520];
            let length = GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize;
            let path = String::from_utf16_lossy(&name[..length]);
            let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
            Some((file, module as usize))
        }
    }

    pub fn os_version() -> String {
        let mut info = OsVersionInfo { size: std::mem::size_of::<OsVersionInfo>() as u32, major: 0, minor: 0, build: 0, platform: 0, service_pack: [0; 128] };
        // SAFETY: the structure is the size it says.
        if unsafe { RtlGetVersion(&mut info) } != 0 {
            return "Windows".to_string();
        }
        let name = if info.major == 10 && info.build >= 22000 { "Windows 11" } else if info.major == 10 { "Windows 10" } else { "Windows" };
        format!("{name} ({}.{}.{})", info.major, info.minor, info.build)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANIC: &str = "Tequila crash report
What happened: an error in Tequila's main thread: index out of bounds
Tequila: Built 2026-10-20 07:00 · source abc1234
System: Windows 11 (10.0.26200) · x86_64 · 16 threads
Started: 2026-10-20 07:00

Message: index out of bounds: the len is 3 but the index is 7
Where: app/src/decal_tab.rs:412
Graphics: NVIDIA GeForce RTX 3060 · OpenGL 4.6
At: 2026-10-20 07:31

Call stack, innermost first:
  tequila_app::decal_tab::resize
      at app/src/decal_tab.rs:412
  tequila_app::decal_tab::ui
      at app/src/decal_tab.rs:300

What was happening, oldest first:
07:30:01 Imported 12 nodes
07:30:40 Label resized
";

    #[test]
    fn the_discord_version_fits_a_message_and_says_what_matters() {
        let text = discord_summary(PANIC);
        assert!(text.chars().count() <= DISCORD_LIMIT, "{}", text.len());
        assert!(text.contains("index out of bounds: the len is 3"));
        assert!(text.contains("tequila_app::decal_tab::resize"));
        assert!(text.contains("Label resized"));
        assert!(text.contains("Windows 11"));
        assert!(text.contains("```"));
        // A long one is cut down, not cut off mid-way without its fence.
        let long = PANIC.replace("Call stack, innermost first:\n", &format!("Call stack, innermost first:\n{}", "  some::very::long::frame::name::that::goes::on\n".repeat(200)));
        let text = discord_summary(&long);
        assert!(text.chars().count() <= DISCORD_LIMIT);
        assert!(text.trim_end().ends_with("```"));
    }

    #[test]
    fn an_ended_session_is_gathered_into_one_report() {
        let folder = std::env::temp_dir().join(format!("tequila-crash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("42-1.running"), "Tequila: test\nSystem: test\n").unwrap();
        std::fs::write(folder.join("42-1.activity"), "07:30:01 Imported 12 nodes\n").unwrap();
        std::fs::write(folder.join("42-1.panic"), "What happened: an error in Tequila's main thread: boom\nMessage: boom\n").unwrap();
        let reports = gather(&folder);
        assert_eq!(reports.len(), 1);
        assert!(reports[0].title.contains("boom"));
        assert!(reports[0].text.contains("Imported 12 nodes"));
        assert!(!folder.join("42-1.running").exists());
        assert!(reports[0].path.exists());
        // A session still running is left alone.
        let held = File::create(folder.join("7-1.running")).unwrap();
        held.try_lock().unwrap();
        assert!(gather(&folder).is_empty());
        drop(held);
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_mac_report_reads_as_the_crashed_thread() {
        let ips = r#"{"app_name":"Tequila","bug_type":"309"}
{"exception":{"type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS at 0x0000000000000010"},"faultingThread":1,
 "threads":[{"frames":[]},{"triggered":true,"frames":[{"imageIndex":1,"symbol":"MTLDebugDevice_resize","imageOffset":4096},{"imageIndex":0,"imageOffset":8192}]}],
 "usedImages":[{"name":"Tequila"},{"name":"Metal"}]}"#;
        let text = read_ips(ips, Path::new("Tequila-2026.ips")).expect("reads");
        assert!(text.contains("EXC_BAD_ACCESS (SIGSEGV)"), "{text}");
        assert!(text.contains("Metal  MTLDebugDevice_resize"));
        assert!(text.contains("Tequila+0x2000"));
    }

    #[test]
    fn source_paths_lose_the_build_computers_folders() {
        assert_eq!(short_path(r"C:\Users\someone\Desktop\Claude\freeshot\app\src\main.rs:1451"), "app/src/main.rs:1451");
        assert_eq!(
            short_path(r"C:\Users\someone\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\eframe-0.29.1\src\native\run.rs:283"),
            "eframe-0.29.1/src/native/run.rs:283"
        );
        assert_eq!(short_path("/Users/someone/tequila/app/src/main.rs:9"), "app/src/main.rs:9");
    }

    #[test]
    fn the_hooks_own_frames_are_left_out() {
        let raw = "   0: std::backtrace::Backtrace::force_capture\n   1: tequila_app::crash::on_panic\n   2: std::panicking::rust_panic_with_hook\n   3: tequila_app::decal_tab::resize\n             at app/src/decal_tab.rs:412\n   4: main\n";
        let tidy = tidy_backtrace(raw);
        assert!(tidy.starts_with("  tequila_app::decal_tab::resize"), "{tidy}");
        assert!(tidy.contains("at app/src/decal_tab.rs:412"));
    }
}
