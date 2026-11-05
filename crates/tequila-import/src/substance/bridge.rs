//! Rendering a Substance material's maps with Adobe's own tools.
//!
//! The only route Tequila takes to a Substance material's pictures is the
//! one Adobe documents and supports: its command-line tools, run as
//! programs, when the person has them. They ship with Substance 3D
//! Designer and with the Substance 3D Automation Toolkit:
//!
//! - `sbsrender render` renders an `.sbsar`'s outputs to image files, with
//!   `--input-graph` for the graph, `--input-graph-output` per output,
//!   `--set-value <identifier>@<value>` per input (`$outputsize@10,10` is
//!   1024 pixels square), `--set-entry <identifier>@<file>` per image input,
//!   and `--output-path`, `--output-name` (`{outputNodeName}` is the
//!   output's identifier) and `--output-format`.
//! - `sbscooker --inputs <file.sbs> --output-path <folder>` cooks Designer's
//!   source into an `.sbsar` named after it.
//!
//! Nothing here loads, links or calls into Adobe's libraries: a tool is a
//! program Tequila starts and waits for, as a person would from a terminal.
//! None of it has been run against the real tools; the tests run a stand-in
//! that takes the same arguments ([`super::fake`]).

use super::{FileKind, Graph};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where Adobe's tools are on this computer, as far as Tequila found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tools {
    pub sbsrender: Option<PathBuf>,
    pub sbscooker: Option<PathBuf>,
}

impl Tools {
    /// Whether an `.sbsar` can be rendered.
    pub fn can_render(&self) -> bool {
        self.sbsrender.is_some()
    }

    /// Whether a file of `kind` can be rendered: an `.sbs` is cooked first.
    pub fn can_render_kind(&self, kind: FileKind) -> bool {
        match kind {
            FileKind::Sbsar => self.sbsrender.is_some(),
            FileKind::Sbs => self.sbsrender.is_some() && self.sbscooker.is_some(),
        }
    }
}

/// What to install, said once for every place that has to say it.
pub const INSTALL_HINT: &str = "Rendering a Substance material's maps needs Adobe's sbsrender, which comes with \
     Adobe Substance 3D Designer and with the Substance 3D Automation Toolkit. Install either, or set the folder \
     that holds sbsrender in the Substance settings. Until then Tequila shows the material's settings and \
     picture, and any maps saved with the project.";

fn executable(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// The folders Adobe's installers put the tools in by default, newest
/// first. From Adobe's install documentation and the folders its installers
/// are known to use; not checked on a machine with them installed.
pub fn default_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    if cfg!(windows) {
        for base in [std::env::var_os("ProgramFiles"), Some("C:\\Program Files".into())].into_iter().flatten() {
            let base = PathBuf::from(base);
            folders.push(base.join("Adobe").join("Adobe Substance 3D Designer"));
            folders.push(base.join("Adobe").join("Adobe Substance 3D Automation Toolkit"));
            folders.push(base.join("Allegorithmic").join("Substance Automation Toolkit"));
            folders.push(base.join("Allegorithmic").join("Substance Designer"));
        }
    } else if cfg!(target_os = "macos") {
        folders.push("/Applications/Adobe Substance 3D Designer/Adobe Substance 3D Designer.app/Contents/MacOS".into());
        folders.push("/Applications/Adobe Substance 3D Designer.app/Contents/MacOS".into());
        folders.push("/Applications/Substance Automation Toolkit".into());
        folders.push("/Applications/Adobe Substance 3D Automation Toolkit".into());
        folders.push("/Applications/Substance Designer.app/Contents/MacOS".into());
    } else {
        folders.push("/opt/Adobe/Adobe_Substance_3D_Designer".into());
        folders.push("/opt/Adobe/Adobe_Substance_3D_Automation_Toolkit".into());
        folders.push("/opt/Allegorithmic/Substance_Automation_Toolkit".into());
        folders.push("/opt/Allegorithmic/Substance_Designer".into());
    }
    folders.dedup();
    folders
}

/// Find the tools: in `configured` first (a folder, or the path of
/// `sbsrender` itself), then in the default install folders, then on the
/// `PATH`.
pub fn find(configured: Option<&Path>) -> Tools {
    find_in(configured, &default_folders(), std::env::var_os("PATH"))
}

fn find_in(configured: Option<&Path>, defaults: &[PathBuf], path: Option<std::ffi::OsString>) -> Tools {
    let mut folders: Vec<PathBuf> = Vec::new();
    let mut tools = Tools::default();
    if let Some(configured) = configured.filter(|p| !p.as_os_str().is_empty()) {
        if configured.is_file() {
            tools.sbsrender = Some(configured.to_path_buf());
            if let Some(parent) = configured.parent() {
                folders.push(parent.to_path_buf());
            }
        } else {
            folders.push(configured.to_path_buf());
        }
    }
    folders.extend(defaults.iter().cloned());
    if let Some(path) = path {
        folders.extend(std::env::split_paths(&path));
    }
    for folder in &folders {
        if tools.sbsrender.is_none() {
            let candidate = folder.join(executable("sbsrender"));
            if candidate.is_file() {
                tools.sbsrender = Some(candidate);
            }
        }
        if tools.sbscooker.is_none() {
            let candidate = folder.join(executable("sbscooker"));
            if candidate.is_file() {
                tools.sbscooker = Some(candidate);
            }
        }
        if tools.sbsrender.is_some() && tools.sbscooker.is_some() {
            break;
        }
    }
    tools
}

/// One render: which graph of which `.sbsar`, with which values, into
/// which folder.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderRequest {
    pub input: PathBuf,
    pub graph: String,
    /// The outputs to render, by identifier; all of the graph's when empty.
    pub outputs: Vec<String>,
    /// `--set-value`s, by input identifier.
    pub values: BTreeMap<String, String>,
    /// `--set-entry`s: image inputs, by identifier.
    pub images: BTreeMap<String, PathBuf>,
    pub output_dir: PathBuf,
}

impl RenderRequest {
    /// The request for `graph` with a material's `values` and `images`,
    /// keeping only the inputs the graph has: a value for an input the
    /// file no longer has (it was re-exported) would make the tool refuse
    /// the whole render.
    pub fn for_graph(input: &Path, graph: &Graph, values: &BTreeMap<String, String>, images: &BTreeMap<String, PathBuf>, output_dir: &Path) -> RenderRequest {
        RenderRequest {
            input: input.to_path_buf(),
            graph: graph.url.clone(),
            outputs: graph.outputs.iter().map(|o| o.identifier.clone()).collect(),
            values: values
                .iter()
                .filter(|(id, _)| graph.input(id).is_some_and(|i| i.value_type != super::ValueType::Image))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            images: images.iter().filter(|(id, path)| graph.input(id).is_some() && path.is_file()).map(|(k, v)| (k.clone(), v.clone())).collect(),
            output_dir: output_dir.to_path_buf(),
        }
    }

    /// `sbsrender`'s arguments, as the documentation spells them. Each is
    /// one argument to the program, so nothing needs quoting or escaping —
    /// the `$` of `$outputsize` included, which only a shell would read.
    pub fn arguments(&self) -> Vec<String> {
        let mut args = vec!["render".to_string(), "--input".into(), self.input.display().to_string()];
        if !self.graph.is_empty() {
            args.push("--input-graph".into());
            args.push(self.graph.clone());
        }
        for output in &self.outputs {
            args.push("--input-graph-output".into());
            args.push(output.clone());
        }
        for (id, value) in &self.values {
            args.push("--set-value".into());
            args.push(format!("{id}@{value}"));
        }
        for (id, path) in &self.images {
            args.push("--set-entry".into());
            args.push(format!("{id}@{}", path.display()));
        }
        args.extend([
            "--output-path".into(),
            self.output_dir.display().to_string(),
            "--output-name".into(),
            "{outputNodeName}".into(),
            "--output-format".into(),
            "png".into(),
        ]);
        args
    }
}

/// `sbscooker`'s arguments for cooking `sbs` into `directory`.
pub fn cook_arguments(sbs: &Path, directory: &Path) -> Vec<String> {
    vec!["--inputs".into(), sbs.display().to_string(), "--output-path".into(), directory.display().to_string()]
}

/// How long one run of a tool may take before it is given up on.
pub const TIMEOUT: Duration = Duration::from_secs(600);

/// Run `program` with `arguments` and wait for it, at most `timeout`.
/// `Err` with what it said on failure.
pub fn run(program: &Path, arguments: &[String], timeout: Duration) -> Result<(), String> {
    use std::io::Read;
    let mut command = std::process::Command::new(program);
    command
        .args(arguments)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        // No console window flashing up behind the application.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let name = program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut child = command.spawn().map_err(|e| format!("{name} could not be started: {e}"))?;
    // Read both pipes on their own threads, so a chatty tool cannot fill one
    // and stall waiting for us.
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut text);
            }
            String::from_utf8_lossy(&text).into_owned()
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name} took longer than {} seconds and was stopped", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("{name} could not be waited for: {e}")),
        }
    };
    let out = stdout.join().unwrap_or_default();
    let err = stderr.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let said = if err.trim().is_empty() { out } else { err };
    let said: String = said.trim().lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ");
    Err(format!(
        "{name} failed ({}){}",
        status.code().map(|c| format!("exit code {c}")).unwrap_or_else(|| "stopped".into()),
        if said.is_empty() { String::new() } else { format!(": {said}") }
    ))
}

/// What renders a request: Adobe's tools as programs normally; the tests
/// put a stand-in here.
pub type Runner = fn(program: &Path, arguments: &[String]) -> Result<(), String>;

/// The normal [`Runner`]: the program itself, with [`TIMEOUT`].
pub fn run_program(program: &Path, arguments: &[String]) -> Result<(), String> {
    run(program, arguments, TIMEOUT)
}

/// What a render made.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    /// Each output's map, by identifier, as the tool wrote it.
    pub maps: BTreeMap<String, PathBuf>,
    /// The `.sbsar` that was rendered: the file itself, or the one an `.sbs`
    /// was cooked into.
    pub sbsar: PathBuf,
}

/// Render `graph` of `file` with `values` and `images` into `directory`:
/// an `.sbs` is cooked first (into `cooked`), then `sbsrender` writes one
/// PNG per output. A DirectX normal map is converted to OpenGL's
/// convention beside itself, as a texture pack's is
/// ([`super::converted_file`]). The folder is written to completion before
/// a `done` marker goes in, so a half-rendered folder is never taken for a
/// finished one.
#[allow(clippy::too_many_arguments)]
pub fn render(
    tools: &Tools,
    runner: Runner,
    file: &Path,
    graph: &Graph,
    values: &BTreeMap<String, String>,
    images: &BTreeMap<String, PathBuf>,
    directory: &Path,
    cooked: &Path,
) -> Result<Rendered, String> {
    let kind = FileKind::of(file).ok_or_else(|| format!("{} is not an .sbsar or .sbs file", file.display()))?;
    if !file.is_file() {
        return Err(format!("{} is not there any more", file.display()));
    }
    let sbsrender = tools.sbsrender.as_deref().ok_or_else(|| INSTALL_HINT.to_string())?;
    let sbsar = match kind {
        FileKind::Sbsar => file.to_path_buf(),
        FileKind::Sbs => {
            let sbscooker = tools.sbscooker.as_deref().ok_or_else(|| {
                "An .sbs has to be cooked into an .sbsar before it renders, which needs Adobe's sbscooker; it was not found beside sbsrender.".to_string()
            })?;
            std::fs::create_dir_all(cooked).map_err(|e| format!("{}: {e}", cooked.display()))?;
            runner(sbscooker, &cook_arguments(file, cooked))?;
            let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let made = cooked.join(format!("{stem}.sbsar"));
            if !made.is_file() {
                return Err(format!("sbscooker finished without writing {}", made.display()));
            }
            made
        }
    };
    // The graph by the cooked file's own URL, which for an `.sbs` in a
    // folder of its package may differ from the one guessed from the source.
    let graph_url = match kind {
        FileKind::Sbsar => graph.url.clone(),
        FileKind::Sbs => super::read(&sbsar)
            .ok()
            .and_then(|cooked| cooked.graph(Some(&graph.url)).map(|g| g.url.clone()))
            .unwrap_or_else(|| graph.url.clone()),
    };
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let marker = directory.join(DONE);
    let _ = std::fs::remove_file(&marker);
    let mut request = RenderRequest::for_graph(&sbsar, graph, values, images, directory);
    request.graph = graph_url;
    runner(sbsrender, &request.arguments())?;
    let mut maps = BTreeMap::new();
    let mut missing = Vec::new();
    for output in &graph.outputs {
        let path = super::rendered_file(directory, &output.identifier);
        // The tool names files after the identifier as it is; the file
        // Tequila reads is the safe spelling of it.
        let written = directory.join(format!("{}.png", output.identifier));
        if !path.is_file() && written.is_file() && written != path {
            let _ = std::fs::rename(&written, &path);
        }
        if path.is_file() {
            maps.insert(output.identifier.clone(), path);
        } else {
            missing.push(output.identifier.clone());
        }
    }
    if maps.is_empty() {
        return Err(format!("sbsrender finished without writing any of the maps ({})", missing.join(", ")));
    }
    if graph.normal_is_directx(values) {
        for output in graph.outputs.iter().filter(|o| matches!(o.slot(true), Ok(crate::texture_set::Slot::Normal { .. }))) {
            if let Some(path) = maps.get(&output.identifier) {
                crate::texture_set::flip_green(path, &super::converted_file(directory, &output.identifier)).map_err(|e| e.to_string())?;
            }
        }
    }
    std::fs::write(&marker, format!("{}\n", graph.url)).map_err(|e| format!("{}: {e}", marker.display()))?;
    Ok(Rendered { maps, sbsar })
}

/// The marker [`render`] leaves in a finished folder.
pub const DONE: &str = "done.txt";

/// Whether `directory` holds a finished render.
pub fn is_rendered(directory: &Path) -> bool {
    directory.join(DONE).is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arguments are the documented ones, each its own argument, and
    /// values for inputs the graph does not have are left out.
    #[test]
    fn a_request_says_what_the_documentation_says() {
        let graph = super::super::fixture::bricks();
        let values: BTreeMap<String, String> = [
            ("$outputsize".to_string(), "10,10".to_string()),
            ("brick_color".to_string(), "1,0,0,1".to_string()),
            ("gone".to_string(), "1".to_string()),
        ]
        .into_iter()
        .collect();
        let request = RenderRequest::for_graph(Path::new("C:/m/bricks.sbsar"), &graph, &values, &BTreeMap::new(), Path::new("C:/out"));
        let args = request.arguments();
        assert_eq!(&args[..5], ["render", "--input", "C:/m/bricks.sbsar", "--input-graph", "pkg://bricks_tequila"]);
        let pairs: Vec<(&str, &str)> = args.windows(2).map(|w| (w[0].as_str(), w[1].as_str())).collect();
        assert!(pairs.contains(&("--set-value", "$outputsize@10,10")));
        assert!(pairs.contains(&("--set-value", "brick_color@1,0,0,1")));
        assert!(!args.iter().any(|a| a.starts_with("gone@")), "{args:?}");
        assert!(pairs.contains(&("--input-graph-output", "basecolor")));
        assert!(pairs.contains(&("--output-name", "{outputNodeName}")));
        assert!(pairs.contains(&("--output-format", "png")));
        assert!(pairs.contains(&("--output-path", "C:/out")));
    }

    /// A configured folder is looked in first, then the default folders,
    /// then the PATH; nothing found is nothing found.
    #[test]
    fn the_tools_are_found_where_they_are() {
        let root = std::env::temp_dir().join(format!("tequila-sbs-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (mine, default, on_path) = (root.join("mine"), root.join("default"), root.join("path"));
        for folder in [&mine, &default, &on_path] {
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::write(default.join(executable("sbsrender")), b"").unwrap();
        std::fs::write(default.join(executable("sbscooker")), b"").unwrap();
        std::fs::write(on_path.join(executable("sbsrender")), b"").unwrap();
        let path = std::env::join_paths([&on_path]).unwrap();

        let found = find_in(None, std::slice::from_ref(&default), Some(path.clone()));
        assert_eq!(found.sbsrender.as_deref(), Some(default.join(executable("sbsrender")).as_path()));
        assert!(found.can_render_kind(FileKind::Sbs));

        std::fs::write(mine.join(executable("sbsrender")), b"").unwrap();
        let found = find_in(Some(&mine), std::slice::from_ref(&default), Some(path.clone()));
        assert_eq!(found.sbsrender.as_deref(), Some(mine.join(executable("sbsrender")).as_path()), "the configured folder comes first");
        assert_eq!(found.sbscooker.as_deref(), Some(default.join(executable("sbscooker")).as_path()));

        let found = find_in(None, &[], Some(path));
        assert_eq!(found.sbsrender.as_deref(), Some(on_path.join(executable("sbsrender")).as_path()));
        assert!(!found.can_render_kind(FileKind::Sbs), "an .sbs needs the cooker as well");

        assert_eq!(find_in(None, &[], None), Tools::default());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tool that fails says why; one that runs too long is stopped.
    #[test]
    fn a_failing_tool_says_why() {
        let missing = Path::new("C:/no/such/sbsrender.exe");
        let error = run(missing, &[], Duration::from_secs(5)).unwrap_err();
        assert!(error.contains("could not be started"), "{error}");
    }
}
