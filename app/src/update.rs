//! Which version this is, and Help > Check for updates: whether GitHub has
//! a newer release, and fetching and installing it.
//!
//! A release is named for when its package was built: the package
//! `Tequila-Windows-20261021-010641` is version 2026.10.21.0106, its tag
//! `v2026.10.21.0106`. A package says so in its `BUILD.txt`; one built
//! before it did is dated by when it was built, which is within the same
//! minute. A build run from its checkout has no version.

use eframe::egui;
use std::fmt;

/// A release's version: the date and the minute it was built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    /// Hours and minutes, as 0106 for 01:06.
    pub time: u16,
}

impl Version {
    /// From a release's name or tag: `2026.10.21.0106`, with or without a
    /// leading `v`.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let mut parts = text.split('.');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.parse().ok()?;
        let time: &str = parts.next()?;
        if parts.next().is_some() || time.len() != 4 {
            return None;
        }
        let version = Version { year, month, day, time: time.parse().ok()? };
        version.is_plausible().then_some(version)
    }

    /// From an ISO 8601 time, `2026-10-21T01:06:50.92+08:00`: the minute it
    /// names, where it names it.
    fn from_built(text: &str) -> Option<Version> {
        let (date, clock) = text.trim().split_once('T')?;
        let mut date = date.split('-');
        let year = date.next()?.parse().ok()?;
        let month = date.next()?.parse().ok()?;
        let day = date.next()?.parse().ok()?;
        let hours: u16 = clock.get(..2)?.parse().ok()?;
        let minutes: u16 = clock.get(3..5)?.parse().ok()?;
        let version = Version { year, month, day, time: hours * 100 + minutes };
        version.is_plausible().then_some(version)
    }

    fn is_plausible(&self) -> bool {
        (1..=12).contains(&self.month) && (1..=31).contains(&self.day) && self.time / 100 < 24 && self.time % 100 < 60
    }

    /// What `BUILD.txt` says this package is.
    pub fn from_build_note(text: &str) -> Option<Version> {
        // Windows PowerShell writes it with a byte-order mark.
        let text = text.trim_start_matches('\u{feff}');
        let field = |key: &str| text.lines().find_map(|l| l.strip_prefix(key)).map(str::trim);
        field("Version:").and_then(Version::parse).or_else(|| field("Built:").and_then(Version::from_built))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02}.{:02}.{:04}", self.year, self.month, self.day, self.time)
    }
}

/// This build's version, or `None` for one run from its checkout. Read
/// once: the window asks every frame.
pub fn current() -> Option<Version> {
    static CURRENT: std::sync::OnceLock<Option<Version>> = std::sync::OnceLock::new();
    *CURRENT.get_or_init(|| {
        let note = std::fs::read_to_string(crate::beside_application("BUILD.txt")?).ok()?;
        Version::from_build_note(&note)
    })
}

/// This build, as the start screen and About say it.
pub fn label() -> String {
    match current() {
        Some(version) => format!("Version {version}"),
        None => format!("Development build {}", env!("CARGO_PKG_VERSION")),
    }
}

// ---- the latest release ----------------------------------------------------

/// Where GitHub says what the latest release is.
const LATEST: &str = "https://api.github.com/repos/ifpintac/tequila/releases/latest";

/// A release, as GitHub describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    /// What it brought, in Markdown.
    pub notes: String,
    /// Its page on GitHub.
    pub page: String,
    pub assets: Vec<Asset>,
}

/// One file of a release.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub url: String,
    /// The SHA-256 GitHub computed when it was uploaded, in hex.
    pub sha256: Option<String>,
}

/// A release from GitHub's description of it.
pub fn read_release(json: &[u8]) -> Result<Release, String> {
    use tequila_format::json::Value;
    let value = tequila_format::json::parse(json).map_err(|e| format!("GitHub's answer could not be read: {e}"))?;
    let tag = value.get("tag_name").and_then(Value::as_str).ok_or("GitHub named no release")?;
    let version = Version::parse(tag).ok_or_else(|| format!("{tag} is not a Tequila version"))?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let assets = value
        .get("assets")
        .map(Value::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(|asset| {
            Some(Asset {
                name: asset.get("name")?.as_str()?.to_string(),
                size: asset.get("size")?.as_f64()? as u64,
                url: asset.get("browser_download_url")?.as_str()?.to_string(),
                sha256: asset.get("digest").and_then(Value::as_str).and_then(|d| d.strip_prefix("sha256:")).map(str::to_lowercase),
            })
        })
        .collect();
    Ok(Release { version, notes: text("body"), page: text("html_url"), assets })
}

fn agent(timeout: std::time::Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).build().into()
}

/// GitHub asks every caller to say who it is.
fn user_agent() -> String {
    match current() {
        Some(version) => format!("Tequila/{version}"),
        None => "Tequila/development".into(),
    }
}

/// The latest release, asked of GitHub.
pub fn fetch_latest() -> Result<Release, String> {
    let mut response = agent(std::time::Duration::from_secs(20))
        .get(LATEST)
        .header("User-Agent", &user_agent())
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("GitHub could not be reached ({e})"))?;
    let body = response
        .body_mut()
        .with_config()
        .limit(4 << 20)
        .read_to_vec()
        .map_err(|e| format!("GitHub's answer could not be read ({e})"))?;
    read_release(&body)
}

/// Which computer this is, as the release's files are named for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Mac,
    /// Linux on x86-64, from a .tar.gz.
    Linux,
    /// Nothing is made for it.
    Other,
}

impl Platform {
    pub fn this() -> Platform {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            Platform::Linux
        } else {
            Platform::Other
        }
    }

    /// What installing the downloaded file is, for a button, and what the
    /// person does then.
    pub fn install_label(self) -> (&'static str, &'static str) {
        match self {
            Platform::Mac => ("Open the disk image", "Drag Tequila into Applications, replacing the one there, then open it again."),
            Platform::Linux => (
                "Show the download",
                "Unpack it in place of this Tequila's folder, then open it again. Your materials, plugins and settings stay.",
            ),
            _ => ("Install and restart", "Tequila closes, the update installs, and Tequila opens again. It asks about unsaved work first."),
        }
    }
}

/// The file to fetch to go from `current` to `release`: on Windows the
/// update installer made from exactly this version when there is one, a
/// tenth of the size, and the full installer otherwise; on a Mac the disk
/// image.
pub fn asset_for(release: &Release, current: Option<Version>, platform: Platform) -> Option<&Asset> {
    let named = |name: String| release.assets.iter().find(|a| a.name == name);
    let latest = release.version;
    match platform {
        Platform::Windows => current
            .and_then(|from| named(format!("Tequila-Update-{from}-to-{latest}.exe")))
            .or_else(|| named(format!("Tequila-Setup-{latest}.exe"))),
        Platform::Mac => named(format!("Tequila-{latest}.dmg")),
        Platform::Linux => named(format!("Tequila-{latest}-linux-x86_64.tar.gz")),
        Platform::Other => None,
    }
}

// ---- fetching it -------------------------------------------------------------

/// How far a download has got, read by the window as it goes.
#[derive(Default)]
pub struct Progress {
    pub done: std::sync::atomic::AtomicU64,
    pub cancel: std::sync::atomic::AtomicBool,
}

/// Fetch `url` to `target`: `size` bytes, whose SHA-256 is `sha256` when
/// that is known. Written beside it first and renamed once it checks out,
/// so a file at `target` is always the whole file.
pub fn fetch(url: &str, size: u64, sha256: Option<&str>, target: &std::path::Path, progress: &Progress) -> Result<(), String> {
    use std::io::{Read, Write};
    use std::sync::atomic::Ordering;
    let partial = target.with_extension("part");
    let result = (|| {
        let response = agent(std::time::Duration::from_secs(60 * 60))
            .get(url)
            .header("User-Agent", &user_agent())
            .call()
            .map_err(|e| format!("the download could not start ({e})"))?;
        let mut reader = response.into_body().into_with_config().limit(size + 1).reader();
        let mut file = std::fs::File::create(&partial).map_err(|e| format!("{} could not be written ({e})", partial.display()))?;
        let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
        let mut buffer = vec![0u8; 1 << 16];
        let mut total = 0u64;
        loop {
            if progress.cancel.load(Ordering::Relaxed) {
                return Err("Cancelled".to_string());
            }
            let read = reader.read(&mut buffer).map_err(|e| format!("the download stopped ({e})"))?;
            if read == 0 {
                break;
            }
            file.write_all(&buffer[..read]).map_err(|e| format!("{} could not be written ({e})", partial.display()))?;
            digest.update(&buffer[..read]);
            total += read as u64;
            progress.done.store(total, Ordering::Relaxed);
        }
        file.sync_all().map_err(|e| format!("{} could not be written ({e})", partial.display()))?;
        if total != size {
            return Err(format!("the download was {total} bytes, not the {size} GitHub lists"));
        }
        if let Some(expected) = sha256 {
            let found: String = digest.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect();
            if !found.eq_ignore_ascii_case(expected) {
                return Err("the download is not the file GitHub lists: its SHA-256 differs".to_string());
            }
        }
        drop(file);
        std::fs::rename(&partial, target).map_err(|e| format!("{} could not be written ({e})", target.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// Where downloaded updates are kept until they are installed.
fn downloads() -> std::path::PathBuf {
    std::env::temp_dir().join("Tequila updates")
}

/// Start installing the downloaded `file`: on Windows, the installer runs
/// once Tequila has closed, replacing this installation, and opens Tequila
/// again; on a Mac the disk image opens, for Tequila to be dragged into
/// Applications; on Linux the folder it is in opens, for it to be unpacked
/// in place of this one, which a running Tequila would be unwise to do to
/// itself. True when Tequila should close now.
pub fn install(file: &std::path::Path, platform: Platform) -> Result<bool, String> {
    match platform {
        Platform::Windows => std::process::Command::new(file)
            .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/relaunch=yes"])
            .spawn()
            .map(|_| true)
            .map_err(|e| format!("the installer could not be started ({e})")),
        Platform::Mac => std::process::Command::new("open")
            .arg(file)
            .spawn()
            .map(|_| false)
            .map_err(|e| format!("the disk image could not be opened ({e})")),
        Platform::Linux => std::process::Command::new("xdg-open")
            .arg(file.parent().unwrap_or(file))
            .spawn()
            .map(|_| false)
            .map_err(|e| format!("its folder could not be opened ({e}); it is in {}", file.display())),
        Platform::Other => Err("there is no installer for this computer".into()),
    }
}

// ---- the person's choice -----------------------------------------------------

/// Whether to look for a new version at launch, and when it last did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preferences {
    pub automatic: bool,
    /// Seconds since 1970.
    pub last_check: u64,
}

impl Default for Preferences {
    fn default() -> Preferences {
        Preferences { automatic: true, last_check: 0 }
    }
}

impl Preferences {
    fn from_text(text: &str) -> Preferences {
        let mut out = Preferences::default();
        for line in text.lines() {
            match line.split_once('=').map(|(k, v)| (k.trim(), v.trim())) {
                Some(("check", value)) => out.automatic = value != "off",
                Some(("last", value)) => out.last_check = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        out
    }

    fn to_text(self) -> String {
        format!("check={}\nlast={}\n", if self.automatic { "auto" } else { "off" }, self.last_check)
    }

    fn path() -> std::path::PathBuf {
        crate::library_tab::user_library_path()
            .parent()
            .map(|p| p.join("updates.txt"))
            .unwrap_or_else(|| std::env::temp_dir().join("tequila-updates.txt"))
    }

    pub fn load() -> Preferences {
        std::fs::read_to_string(Preferences::path()).map(|t| Preferences::from_text(&t)).unwrap_or_default()
    }

    pub fn save(self) {
        let path = Preferences::path();
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = std::fs::write(path, self.to_text());
    }

    /// A quiet look at launch is due: at most once a day.
    pub fn due(self, now: u64) -> bool {
        self.automatic && now.saturating_sub(self.last_check) >= 20 * 60 * 60
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---- the window ----------------------------------------------------------------

type Arriving<T> = std::sync::mpsc::Receiver<Result<T, String>>;

enum State {
    Idle,
    Checking { quiet: bool, arriving: Arriving<Release> },
    /// This is the latest, or newer than it.
    Current(Release),
    Newer(Release),
    Downloading { release: Release, asset: Asset, progress: std::sync::Arc<Progress>, arriving: Arriving<std::path::PathBuf> },
    Ready { release: Release, file: std::path::PathBuf },
    Failed(String),
}

/// Help > Check for updates, and the quiet look at launch.
pub struct Updates {
    pub open: bool,
    state: State,
    pub preferences: Preferences,
    /// A newer version the quiet look found, said once.
    pub found: Option<Version>,
}

impl Default for Updates {
    fn default() -> Updates {
        Updates { open: false, state: State::Idle, preferences: Preferences::default(), found: None }
    }
}

/// What the window asks the application to do.
pub enum Act {
    /// Install the downloaded file, once unsaved work is settled.
    Install(std::path::PathBuf),
}

impl Updates {
    pub fn with_preferences(preferences: Preferences) -> Updates {
        Updates { preferences, ..Updates::default() }
    }

    /// Look on GitHub: openly, from the menu, or quietly at launch.
    pub fn check(&mut self, quiet: bool) {
        if matches!(self.state, State::Checking { .. } | State::Downloading { .. }) {
            return;
        }
        let (send, arriving) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = send.send(fetch_latest());
        });
        self.state = State::Checking { quiet, arriving };
        self.preferences.last_check = now();
        self.preferences.save();
    }

    /// The quiet look at launch, when one is due and this is a release.
    pub fn start_up(&mut self) {
        if current().is_some() && self.preferences.due(now()) {
            self.check(true);
        }
    }

    /// Take in what the background threads have found. A newer version a
    /// quiet look found is in `found`, once.
    pub fn poll(&mut self, ctx: &egui::Context) {
        match &self.state {
            State::Checking { quiet, arriving } => match arriving.try_recv() {
                Ok(Ok(release)) => {
                    let newer = current().is_none_or(|have| release.version > have);
                    if *quiet && newer && current().is_some() {
                        self.found = Some(release.version);
                    }
                    self.state = if newer && current().is_some() { State::Newer(release) } else { State::Current(release) };
                }
                Ok(Err(why)) => self.state = if *quiet { State::Idle } else { State::Failed(why) },
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.state = State::Idle,
            },
            State::Downloading { release, arriving, .. } => match arriving.try_recv() {
                Ok(Ok(file)) => self.state = State::Ready { release: release.clone(), file },
                Ok(Err(why)) => self.state = State::Failed(why),
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.state = State::Idle,
            },
            _ => {}
        }
    }

    /// A newer version is known, from either kind of look.
    pub fn newer(&self) -> Option<Version> {
        match &self.state {
            State::Newer(r) | State::Downloading { release: r, .. } | State::Ready { release: r, .. } => Some(r.version),
            _ => None,
        }
    }

    fn download(&mut self, release: Release, asset: Asset) {
        let progress = std::sync::Arc::new(Progress::default());
        let (send, arriving) = std::sync::mpsc::channel();
        let (theirs, from) = (progress.clone(), asset.clone());
        std::thread::spawn(move || {
            let folder = downloads();
            let result = std::fs::create_dir_all(&folder)
                .map_err(|e| format!("{} could not be made ({e})", folder.display()))
                .and_then(|()| {
                    let target = folder.join(&from.name);
                    fetch(&from.url, from.size, from.sha256.as_deref(), &target, &theirs).map(|()| target)
                });
            let _ = send.send(result);
        });
        self.state = State::Downloading { release, asset, progress, arriving };
    }

    /// The window, while it is open.
    pub fn window(&mut self, ctx: &egui::Context) -> Option<Act> {
        self.poll(ctx);
        if !self.open {
            return None;
        }
        let p = crate::theme::p();
        let mut open = true;
        let mut act = None;
        // What a button asked for, done once the window is drawn.
        enum Want {
            Check,
            Download(Release, Asset),
        }
        let mut want = None;
        egui::Window::new("Updates")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.set_width(460.0);
                let have = current();
                let have_text = have.map_or_else(|| "a development build".to_string(), |v| format!("version {v}"));
                match &self.state {
                    State::Idle => {
                        ui.label(format!("You have {have_text}."));
                        if ui.button("Check for updates").clicked() {
                            want = Some(Want::Check);
                        }
                    }
                    State::Checking { .. } => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Asking GitHub for the latest release\u{2026}");
                        });
                    }
                    State::Current(release) => {
                        if have.is_some() {
                            heading(ui, "Tequila is up to date");
                            ui.label(format!("You have {have_text}, the latest."));
                        } else {
                            heading(ui, "A development build");
                            ui.label(format!(
                                "This Tequila was built from its source, which updates it. The latest release is {}.",
                                release.version
                            ));
                        }
                        if ui.link("See every release").clicked() {
                            ctx.open_url(egui::OpenUrl::new_tab(&release.page));
                        }
                    }
                    State::Newer(release) => {
                        heading(ui, &format!("Tequila {} is available", release.version));
                        ui.label(egui::RichText::new(format!("You have {have_text}.")).color(p.text_dim));
                        notes(ui, &release.notes);
                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            match asset_for(release, have, Platform::this()) {
                                Some(asset) => {
                                    let label = format!("Download and install ({:.0} MB)", asset.size as f64 / 1e6);
                                    if ui.add(egui::Button::new(egui::RichText::new(label).color(p.on_accent)).fill(p.accent)).clicked() {
                                        want = Some(Want::Download(release.clone(), asset.clone()));
                                    }
                                }
                                None => {
                                    ui.label(egui::RichText::new("There is no installer for this computer to fetch.").color(p.text_dim));
                                }
                            }
                            if ui.button("See what's new").clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(&release.page));
                            }
                        });
                    }
                    State::Downloading { release, asset, progress, .. } => {
                        heading(ui, &format!("Downloading Tequila {}", release.version));
                        let done = progress.done.load(std::sync::atomic::Ordering::Relaxed);
                        let fraction = if asset.size > 0 { done as f32 / asset.size as f32 } else { 0.0 };
                        ui.add(egui::ProgressBar::new(fraction).text(format!("{:.1} of {:.1} MB", done as f64 / 1e6, asset.size as f64 / 1e6)));
                        if ui.button("Cancel").clicked() {
                            progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                    State::Ready { release, file } => {
                        heading(ui, &format!("Tequila {} is ready to install", release.version));
                        let (label, about) = Platform::this().install_label();
                        ui.label(egui::RichText::new(about).color(p.text_dim));
                        ui.add_space(6.0);
                        if ui.add(egui::Button::new(egui::RichText::new(label).color(p.on_accent)).fill(p.accent)).clicked() {
                            act = Some(Act::Install(file.clone()));
                        }
                    }
                    State::Failed(why) => {
                        heading(ui, "The update did not go through");
                        ui.label(egui::RichText::new(why).color(p.text_dim));
                        ui.horizontal(|ui| {
                            if ui.button("Try again").clicked() {
                                want = Some(Want::Check);
                            }
                            if ui.button("Download from GitHub").clicked() {
                                ctx.open_url(egui::OpenUrl::new_tab(crate::start_screen::RELEASES));
                            }
                        });
                    }
                }
                ui.add_space(10.0);
                ui.separator();
                let mut automatic = self.preferences.automatic;
                if ui
                    .add(crate::field::Checkbox::new(&mut automatic, "Look for updates when Tequila starts"))
                    .on_hover_text("Once a day, Tequila asks GitHub what the latest release is. Nothing about you or your work is sent.")
                    .changed()
                {
                    self.preferences.automatic = automatic;
                    self.preferences.save();
                }
            });
        match want {
            Some(Want::Check) => self.check(false),
            Some(Want::Download(release, asset)) => self.download(release, asset),
            None => {}
        }
        // Closing the window does not stop a download: it carries on, and
        // the window shows it finished when opened again.
        if !open {
            self.open = false;
        }
        act
    }

    /// The menu item, which says when a newer version is known.
    pub fn menu_label(&self) -> String {
        match self.newer() {
            Some(version) => format!("Update to {version}\u{2026}"),
            None => "Check for updates\u{2026}".into(),
        }
    }

    /// Open the window from the menu: looking at once unless it already
    /// knows of a newer version or is busy.
    pub fn open_window(&mut self) {
        self.open = true;
        if matches!(self.state, State::Idle | State::Current(_) | State::Failed(_)) {
            self.check(false);
        }
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(crate::theme::strong(15.0)).color(crate::theme::p().text));
    ui.add_space(4.0);
}

/// A release's notes, plainly: its Markdown's marks taken out.
fn notes(ui: &mut egui::Ui, markdown: &str) {
    let p = crate::theme::p();
    let text = plain(markdown);
    if text.trim().is_empty() {
        return;
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
        ui.label(egui::RichText::new(text).size(12.5).color(p.text));
    });
}

/// Markdown as plain text: headings, emphasis and code marks dropped, list
/// items as bullets, links as their text.
fn plain(markdown: &str) -> String {
    let mut out = String::new();
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        // A rule between sections.
        if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') {
            continue;
        }
        let indent = &line[..line.len() - trimmed.len()];
        let body = trimmed.trim_start_matches('#').trim_start();
        let body = match body.strip_prefix("- ").or_else(|| body.strip_prefix("* ")) {
            Some(item) => format!("{indent}\u{2022} {item}"),
            None => body.to_string(),
        };
        let mut text = String::new();
        let mut rest = body.as_str();
        // [text](link) as text.
        while let Some(open) = rest.find('[') {
            match (rest[open..].find("]("), rest[open..].find(')')) {
                (Some(close), Some(end)) if close < end => {
                    text.push_str(&rest[..open]);
                    text.push_str(&rest[open + 1..open + close]);
                    rest = &rest[open + end + 1..];
                }
                _ => {
                    text.push_str(&rest[..=open]);
                    rest = &rest[open + 1..];
                }
            }
        }
        text.push_str(rest);
        out.push_str(&text.replace("**", "").replace('`', ""));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release's tag, its name and its package's note all give the same
    /// version, which sorts by when it was built.
    #[test]
    fn a_version_is_read_from_a_tag_or_a_build_note() {
        let tagged = Version::parse("v2026.10.21.0106").unwrap();
        assert_eq!(tagged.to_string(), "2026.10.21.0106");
        assert_eq!(Version::parse("2026.10.21.0106"), Some(tagged));
        assert_eq!(Version::from_build_note("\u{feff}Source: 0165\nVersion: 2026.10.21.0106\nBuilt: 2026-10-22T09:00:00+08:00\n"), Some(tagged));
        // A package from before the note said so: dated by its build.
        assert_eq!(Version::from_build_note("Source: 0165\nBuilt: 2026-10-21T01:06:50.9204388+08:00\n"), Some(tagged));
        assert!(Version::parse("v2026.10.21.0930").unwrap() > tagged);
        assert!(Version::parse("v2026.10.20.2359").unwrap() < tagged);
        for nonsense in ["", "v1.2", "2026.13.01.0000", "2026.10.21.2460", "2026.10.21.106", "latest"] {
            assert_eq!(Version::parse(nonsense), None, "{nonsense}");
        }
    }

    /// What GitHub says of a release: an excerpt of a real answer's shape.
    const RELEASE: &str = r#"{
        "tag_name": "v2026.11.02.0930",
        "html_url": "https://github.com/ifpintac/tequila/releases/tag/v2026.11.02.0930",
        "body": "**A start screen.** New, open and recent.\n- Samples\n- [Updates](https://example.com) from Help\n\n---",
        "assets": [
            {"name": "Tequila-2026.11.02.0930.dmg", "size": 109479291, "browser_download_url": "https://example.com/a.dmg", "digest": "sha256:AB12"},
            {"name": "Tequila-Setup-2026.11.02.0930.exe", "size": 132976896, "browser_download_url": "https://example.com/setup.exe", "digest": "sha256:cd34"},
            {"name": "Tequila-Update-2026.10.21.0106-to-2026.11.02.0930.exe", "size": 11388469, "browser_download_url": "https://example.com/update.exe"}
        ]
    }"#;

    /// A release reads into its version, notes, page and files, each file's
    /// digest without its prefix.
    #[test]
    fn a_release_is_read_from_githubs_answer() {
        let release = read_release(RELEASE.as_bytes()).unwrap();
        assert_eq!(release.version.to_string(), "2026.11.02.0930");
        assert!(release.page.ends_with("v2026.11.02.0930"));
        assert_eq!(release.assets.len(), 3);
        assert_eq!(release.assets[0].sha256.as_deref(), Some("ab12"));
        assert_eq!(release.assets[2].sha256, None);
        assert!(read_release(br#"{"tag_name": "nightly"}"#).is_err());
        assert!(read_release(b"not json").is_err());
    }

    /// The file fetched: the update installer from exactly this version,
    /// the full installer from any other or from none, the disk image on a
    /// Mac, and nothing where no installer is made.
    #[test]
    fn the_right_file_is_chosen_for_this_computer() {
        let release = read_release(RELEASE.as_bytes()).unwrap();
        let from = Version::parse("2026.10.21.0106");
        let name = |current, platform| asset_for(&release, current, platform).map(|a| a.name.as_str());
        assert_eq!(name(from, Platform::Windows), Some("Tequila-Update-2026.10.21.0106-to-2026.11.02.0930.exe"));
        assert_eq!(name(Version::parse("2026.10.20.1656"), Platform::Windows), Some("Tequila-Setup-2026.11.02.0930.exe"));
        assert_eq!(name(None, Platform::Windows), Some("Tequila-Setup-2026.11.02.0930.exe"));
        assert_eq!(name(from, Platform::Mac), Some("Tequila-2026.11.02.0930.dmg"));
        assert_eq!(name(from, Platform::Other), None);
        assert_eq!(name(from, Platform::Linux), None, "this release made no Linux archive");
        let mut with_linux = release.clone();
        with_linux.assets.push(Asset { name: "Tequila-2026.11.02.0930-linux-x86_64.tar.gz".into(), size: 1, url: String::new(), sha256: None });
        assert_eq!(asset_for(&with_linux, from, Platform::Linux).map(|a| a.name.as_str()), Some("Tequila-2026.11.02.0930-linux-x86_64.tar.gz"));
    }

    /// Serve `body` once, over plain HTTP on a free port, and say where.
    fn serve(body: Vec<u8>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://{address}/installer.exe")
    }

    fn sha256(bytes: &[u8]) -> String {
        ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A download that is the file GitHub lists is kept; one whose size or
    /// SHA-256 is not leaves nothing behind, not even its part.
    #[test]
    fn a_download_is_kept_only_when_it_checks_out() {
        let folder = std::env::temp_dir().join(format!("tequila-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let target = folder.join("installer.exe");

        let progress = Progress::default();
        fetch(&serve(body.clone()), body.len() as u64, Some(&sha256(&body)), &target, &progress).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), body);
        assert_eq!(progress.done.load(std::sync::atomic::Ordering::Relaxed), body.len() as u64);
        std::fs::remove_file(&target).unwrap();

        let wrong = fetch(&serve(body.clone()), body.len() as u64, Some(&sha256(b"something else")), &target, &Progress::default());
        assert!(wrong.unwrap_err().contains("SHA-256"));
        let short = fetch(&serve(body.clone()), body.len() as u64 + 10, None, &target, &Progress::default());
        assert!(short.is_err());
        assert!(!target.exists() && !target.with_extension("part").exists(), "a failed download left a file");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// The choice survives a round trip, and a look is due once a day.
    #[test]
    fn the_update_preferences_round_trip() {
        let off = Preferences { automatic: false, last_check: 1_700_000_000 };
        assert_eq!(Preferences::from_text(&off.to_text()), off);
        assert_eq!(Preferences::from_text("garbage"), Preferences::default());
        let on = Preferences { automatic: true, last_check: 1_000 };
        assert!(on.due(1_000 + 21 * 3600));
        assert!(!on.due(1_000 + 3600));
        assert!(!off.due(u64::MAX));
    }

    /// Release notes read plainly.
    #[test]
    fn release_notes_lose_their_marks() {
        let release = read_release(RELEASE.as_bytes()).unwrap();
        assert_eq!(plain(&release.notes), "A start screen. New, open and recent.\n\u{2022} Samples\n\u{2022} Updates from Help\n\n");
    }
}
