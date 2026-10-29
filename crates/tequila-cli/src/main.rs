//! The headless Tequila renderer.
//!
//! `TEQUILA_SPEC.md` §1.15 maps network rendering onto "a headless CLI
//! renderer plus a job dispatcher", and §1.16 wants everything scriptable.
//! The full command surface arrives at M7 with the Python API over the same
//! command bus.
//!
//! It renders a `.tequila` project, an imported model, or the demonstration
//! scene, to a PNG or a JPEG. Which matters more than it sounds: it makes
//! the whole pipeline — import, document, snapshot, BVH, transport, denoise,
//! tone map, encode — verifiable without a display. That is what CI needs, and
//! it is what makes a rendering regression reproducible from a bug report.
//!
//! And everything M7 asks of a machine with no one at it: a saved camera,
//! Studio, model set or configurator choice to render from; every valid
//! configuration in one run, each named by its choices; the glTF, web
//! configurator and spin viewer exports, with their photos and frames
//! rendered here; the Silhouette cut-out. A scripted job of colourways and
//! cameras runs unattended and leaves a working web configurator behind.
//!
//! With `--serve` it renders nothing by itself: it holds a document and
//! answers the scripting channel's protocol, which is what the Python
//! module's `tequila.open()` runs (`serve.rs`). With `--worker` it is a
//! render node, and with `--farm` it sends its renders to render nodes
//! rather than rendering them here (`farm.rs`).

use tequila_color::DisplayTransform;
use tequila_core::math::{vec3, Vec3};
use tequila_denoise::{DenoiseSettings, Denoiser};
use tequila_render_core::camera::Camera;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::LightingPreset;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_scene::bench::BenchScene;
use tequila_scene::demo::DemoMaterial;
use tequila_scene::snapshot::{snapshot, visible_bounds, MeshCache};
use std::sync::Arc;
use std::time::Instant;

mod farm;
mod mcp;
mod serve;

#[derive(Clone)]
struct Options {
    /// Unset means the project's own setting, or the demo's default.
    width: Option<usize>,
    height: Option<usize>,
    samples: Option<u32>,
    output: String,
    /// A `.tequila` project or a model to import. `None` renders the demo
    /// scene.
    input: Option<String>,
    /// Measure a reference benchmark scene rather than rendering an image.
    bench: Option<BenchScene>,
    material: DemoMaterial,
    roughness: f32,
    preset: Option<LightingPreset>,
    /// A studio setup of light pins, by name.
    pins: Option<String>,
    /// A time of day to build a sky for, by name.
    sky: Option<String>,
    /// An HDR or EXR to light with, and its turn about the vertical.
    hdri: Option<String>,
    hdri_rotation: Option<f32>,
    /// Leave the floor out, whatever the project says.
    no_ground: bool,
    denoise: bool,
    exposure: Option<f32>,
    display: Option<DisplayTransform>,
    /// Keep the background transparent, whatever the project says.
    alpha: bool,
    /// Caustics on or off, whatever the lighting says.
    caustics: Option<bool>,
    /// Render with Cycles (true) or Tequila's own path tracer (false);
    /// unsaid, a project renders with its own engine.
    cycles: Option<bool>,
    /// The Cycles device: CPU, OPTIX, CUDA, or an id; unsaid, the engine's.
    device: Option<String>,
    /// Whether `-o` was given, rather than defaulted: an export-only run
    /// renders nothing else.
    output_set: bool,
    /// Cut out along the product's outline, glass solid.
    silhouette: bool,
    /// A saved camera, Studio or model set, by name or number.
    camera: Option<String>,
    studio: Option<String>,
    model_set: Option<String>,
    /// The configurator's choices, by label or number, comma-separated.
    configuration: Option<String>,
    /// Every valid configuration, each to its own file.
    all_configurations: bool,
    /// Every Studio - every view - each to its own file.
    all_studios: bool,
    /// A studio preset to apply first: an installed one's id, or its folder
    /// or `.studio.json`; how the product faces it, and whether it dresses
    /// the product by its rules.
    studio_preset: Option<String>,
    front: Option<String>,
    dress: bool,
    export_glb: Option<String>,
    export_usd: Option<String>,
    export_web: Option<String>,
    /// Render a photo of every configuration into the web export.
    photos: bool,
    export_spin: Option<String>,
    columns: usize,
    rows: usize,
    tilt: f32,
    /// Answer the scripting protocol instead of rendering.
    serve: bool,
    /// Where `--serve` announces its port and token.
    discovery: Option<String>,
    /// End `--serve` when standard input closes: when the script that
    /// started it has gone.
    attached: bool,
    /// Be the MCP bridge for an AI app, over standard input and output.
    mcp: bool,
    /// The bridge for something reached from outside this computer.
    restricted: bool,
    /// Be a render node.
    worker: bool,
    /// Where a render node listens.
    listen: Option<String>,
    /// What a render node calls itself.
    worker_name: Option<String>,
    /// The farm's shared secret.
    token: Option<String>,
    /// Render on these nodes instead of here: host[:port], comma-separated.
    farm: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            width: None,
            height: None,
            samples: None,
            output: "tequila.png".into(),
            input: None,
            bench: None,
            material: DemoMaterial::Plastic,
            pins: None,
            sky: None,
            hdri: None,
            hdri_rotation: None,
            no_ground: false,
            roughness: 0.18,
            preset: None,
            denoise: true,
            exposure: None,
            display: None,
            alpha: false,
            caustics: None,
            cycles: None,
            device: None,
            output_set: false,
            silhouette: false,
            camera: None,
            studio: None,
            model_set: None,
            configuration: None,
            all_configurations: false,
            all_studios: false,
            studio_preset: None,
            front: None,
            dress: false,
            export_glb: None,
            export_usd: None,
            export_web: None,
            photos: false,
            export_spin: None,
            columns: 36,
            rows: 1,
            tilt: 15.0,
            serve: false,
            discovery: None,
            attached: false,
            worker: false,
            mcp: false,
            restricted: false,
            listen: None,
            worker_name: None,
            token: None,
            farm: None,
        }
    }
}

const USAGE: &str = "\
tequila - headless renderer

usage: tequila [options]

      --bench NAME      measure a reference scene instead of rendering:
                        simple_product | cad_assembly
  -i, --input PATH      a .tequila project, or a model to import
                        (.obj .stl .gltf .glb); omit for the demo scene
  -o, --output PATH     output image, .png or .jpg (default tequila.png);
                        {configuration} in it stands for the choices
  -w, --width N         image width (default: the project's, else 960)
  -h, --height N        image height (default: the project's, else 600)
  -s, --samples N       samples per pixel (default: the project's, else 128)
  -m, --material NAME   plastic | metal | diffuse (default plastic)
  -r, --roughness F     0 to 1 (default 0.18)
  -l, --lighting NAME   performance | basic | product | interior | jewelry
      --pins NAME       studio lighting: three-point | overhead softbox |
                        two strips
      --sky NAME        light with a physical sky: midday | afternoon |
                        golden hour | overcast
  -e, --exposure F      exposure in stops (default 0)
  -t, --tonemap NAME    agx | aces | neutral | srgb | raw (default: the
                        project's look, else the application's)
      --no-denoise      skip the denoiser
      --alpha           keep the background transparent
      --engine NAME     tequila or cycles: render with Blender's Cycles,
                        when tequila_cycles.dll is installed (default: a
                        project's own engine, else tequila)
      --device NAME     the Cycles device: CPU, OPTIX, CUDA (default: the
                        engine's - a project's graphics-card engine uses the
                        card, else the processor)
      --hdri PATH       light with an HDR or EXR environment
      --hdri-rotation D turn the environment by D degrees
      --no-ground       no floor: the model floats
      --caustics        trace light through glass and off mirrors onto
                        what is around them (--no-caustics to turn off)
      --silhouette      cut out along the product's outline: glass stays
                        solid, the floor and its shadow go (implies alpha)

From a project's own setup:
      --camera NAME     a saved camera, by name or number from 0
      --studio NAME     apply a Studio first, by name or number
      --model-set NAME  show a model set, by name or number
      --configuration C the configurator's choices, by label or number,
                        comma-separated in option order: 'Red,Leather'
      --all-configurations
                        render every valid configuration, each to the
                        output name with {configuration} in it
      --all-studios     render every Studio (every view), each to the
                        output name with {view} in it - or, without -o,
                        to the project's own output folder and name
      --studio-preset P apply a studio preset first: an installed one's id,
                        or its folder or .studio.json; the render then
                        takes the preset's settings
      --front F         with --studio-preset, the product's side that faces
                        the Main camera: +z (default), -z, +x or -x
      --dress           with --studio-preset, put its materials on parts
                        by their names

Exports (with no -o, only these run):
      --export-glb PATH what is shown, as a glTF binary
      --export-usd PATH what is shown, as a USD stage (.usda)
      --export-web DIR  the configurator as a static web folder
      --photos          with --export-web: render every configuration into
                        it as photos (DIR/images)
      --export-spin DIR a spin viewer: frames rendered all the way round
      --columns N       spin frames round (default 36)
      --rows N          spin rows of tilt (default 1)
      --tilt D          degrees between rows (default 15)

Scripting:
      --serve           render nothing; hold the -i project (or model, or an
                        empty scene) and answer the scripting protocol, as
                        the application does, for tequila.open() in Python
      --discovery FILE  where --serve writes its port and token
      --attached        end --serve when standard input closes
      --mcp             be the MCP server an AI app (the Claude app, Claude
                        Code, Cursor) starts, to work in the Tequila that is
                        open, over standard input and output
      --restricted      with --mcp: the open scene only - no importing, no
                        raw requests, no saving or rendering to chosen paths

Network rendering:
      --worker          be a render node: take renders from dispatchers
                        holding the farm's token, until stopped
      --listen ADDR     where a node listens (default 0.0.0.0:7740)
      --name NAME       what a node calls itself (default: the computer's)
      --farm NODES      render on these nodes instead of here: host[:port],
                        comma-separated; with --all-configurations, the
                        configurations are shared out among them
      --token SECRET    the farm's shared secret, at least 8 characters
                        (or TEQUILA_FARM_TOKEN); the same on every machine.
                        Projects and pictures travel unencrypted: use a
                        network you trust, or a VPN

A .tequila project renders as the application renders it: its camera,
lighting, look, resolution and samples. The flags override those.
      --help            show this message
";

fn parse_args() -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-o" | "--output" => {
                options.output = value()?;
                options.output_set = true;
            }
            "--silhouette" => options.silhouette = true,
            "--camera" => options.camera = Some(value()?),
            "--studio" => options.studio = Some(value()?),
            "--model-set" => options.model_set = Some(value()?),
            "--configuration" => options.configuration = Some(value()?),
            "--all-configurations" => options.all_configurations = true,
            "--all-studios" => options.all_studios = true,
            "--studio-preset" => options.studio_preset = Some(value()?),
            "--front" => options.front = Some(value()?),
            "--dress" => options.dress = true,
            "--export-glb" => options.export_glb = Some(value()?),
            "--export-usd" => options.export_usd = Some(value()?),
            "--export-web" => options.export_web = Some(value()?),
            "--photos" => options.photos = true,
            "--export-spin" => options.export_spin = Some(value()?),
            "--serve" => options.serve = true,
            "--discovery" => options.discovery = Some(value()?),
            "--attached" => options.attached = true,
            "--worker" => options.worker = true,
            "--mcp" => options.mcp = true,
            "--restricted" => options.restricted = true,
            "--listen" => options.listen = Some(value()?),
            "--name" => options.worker_name = Some(value()?),
            "--token" => options.token = Some(value()?),
            "--farm" => options.farm = Some(value()?),
            "--columns" => options.columns = value()?.parse().map_err(|_| "columns must be a number")?,
            "--rows" => options.rows = value()?.parse().map_err(|_| "rows must be a number")?,
            "--tilt" => options.tilt = value()?.parse().map_err(|_| "tilt must be a number")?,
            "-i" | "--input" => options.input = Some(value()?),
            "--bench" => {
                let name = value()?;
                options.bench = Some(
                    BenchScene::from_name(&name)
                        .ok_or_else(|| format!("unknown benchmark scene {name:?}"))?,
                );
            }
            "-w" | "--width" => {
                options.width = Some(value()?.parse().map_err(|_| "width must be a number")?)
            }
            "-h" | "--height" => {
                options.height = Some(value()?.parse().map_err(|_| "height must be a number")?)
            }
            "-s" | "--samples" => {
                options.samples = Some(value()?.parse().map_err(|_| "samples must be a number")?)
            }
            "-r" | "--roughness" => {
                options.roughness = value()?.parse().map_err(|_| "roughness must be a number")?
            }
            "-e" | "--exposure" => {
                options.exposure = Some(value()?.parse().map_err(|_| "exposure must be a number")?)
            }
            "-m" | "--material" => {
                let name = value()?;
                options.material = match name.to_ascii_lowercase().as_str() {
                    "plastic" => DemoMaterial::Plastic,
                    "metal" => DemoMaterial::Metal,
                    "diffuse" => DemoMaterial::Diffuse,
                    _ => return Err(format!("unknown material {name:?}")),
                };
            }
            "--pins" => {
                let name = value()?;
                if tequila_render_core::pins::studio_preset(&name).is_none() {
                    return Err(format!("unknown studio setup {name:?}"));
                }
                options.pins = Some(name);
            }
            "--sky" => {
                let name = value()?;
                if tequila_render_core::sky::time_of_day(&name).is_none() {
                    return Err(format!("unknown time of day {name:?}"));
                }
                options.sky = Some(name);
            }
            "-l" | "--lighting" => {
                let name = value()?;
                options.preset = Some(
                    LightingPreset::ALL
                        .into_iter()
                        .find(|p| p.label().eq_ignore_ascii_case(&name))
                        .ok_or_else(|| format!("unknown lighting preset {name:?}"))?,
                );
            }
            "-t" | "--tonemap" => {
                let name = value()?;
                options.display = Some(match name.to_ascii_lowercase().as_str() {
                    "agx" => DisplayTransform::AgX,
                    "aces" => DisplayTransform::Aces,
                    "neutral" => DisplayTransform::PbrNeutral,
                    "srgb" => DisplayTransform::Srgb,
                    "raw" => DisplayTransform::Raw,
                    _ => return Err(format!("unknown tone map {name:?}")),
                });
            }
            "--no-denoise" => options.denoise = false,
            "--alpha" => options.alpha = true,
            "--caustics" => options.caustics = Some(true),
            "--no-caustics" => options.caustics = Some(false),
            "--engine" => match value()?.as_str() {
                "cycles" => options.cycles = Some(true),
                "tequila" => options.cycles = Some(false),
                other => return Err(format!("unknown engine {other:?}: tequila or cycles")),
            },
            "--device" => options.device = Some(value()?),
            "--hdri" => options.hdri = Some(value()?),
            "--no-ground" => options.no_ground = true,
            "--hdri-rotation" => {
                options.hdri_rotation =
                    Some(value()?.parse().map_err(|_| "--hdri-rotation must be a number of degrees")?)
            }
            other => return Err(format!("unknown option {other:?}")),
        }
    }

    if options.width == Some(0) || options.height == Some(0) {
        return Err("width and height must be non-zero".into());
    }
    if options.samples == Some(0) {
        return Err("samples must be at least 1".into());
    }
    Ok(options)
}

fn main() {
    let options = match parse_args() {
        Ok(options) => options,
        Err(error) => {
            eprintln!("tequila: {error}\n\n{USAGE}");
            std::process::exit(2);
        }
    };

    let outcome = match options.bench {
        Some(scene) => benchmark(scene),
        None if options.mcp => mcp::run(options.discovery.as_deref(), options.restricted),
        None if options.serve => serve::run(&options),
        None if options.worker => farm::worker(&options),
        None => run(&options),
    };
    if let Err(error) = outcome {
        eprintln!("tequila: {error}");
        std::process::exit(1);
    }
}

/// What to render: a saved project, an imported model, or the demo scene.
///
/// Import goes through the same `Command::Import` the application uses, so a
/// file that renders here renders there — and a file that fails here fails
/// with the same message.
fn load_document(options: &Options) -> Result<tequila_scene::SceneDocument, String> {
    let Some(input) = &options.input else {
        return Ok(tequila_scene::demo::demo_document_sized(
            options.material,
            options.roughness,
            tequila_scene::demo::STARTER_RADIUS,
        ));
    };
    let path = std::path::Path::new(input);

    if path.extension().and_then(|e| e.to_str()) == Some("tequila") {
        let loaded = tequila_format::load(path).map_err(|e| e.to_string())?;
        for note in &loaded.notes {
            eprintln!("tequila: {note}");
        }
        return Ok(loaded.document);
    }

    let mut document = tequila_scene::SceneDocument::new();
    let mut bus = tequila_scene::CommandBus::new();
    let (command, report) = tequila_import::import(
        &document,
        None,
        path,
        &tequila_import::ImportOptions::default(),
    )
    .map_err(|e| e.to_string())?;
    for warning in &report.warnings {
        eprintln!("tequila: {warning}");
    }
    eprintln!(
        "imported {} nodes, {} meshes ({} instanced), {} triangles, as {:?}{}",
        report.nodes,
        report.meshes_kept,
        report.instances_detected,
        report.triangles,
        report.source_unit,
        if report.unit_was_guessed {
            " (guessed from the bounding box)"
        } else {
            ""
        }
    );
    bus.apply(&mut document, command)
        .map_err(|e| e.to_string())?;
    Ok(document)
}

/// Measure a reference scene against the targets of `docs/01` §15.
///
/// Reported rather than asserted. This machine is not the fixed CI hardware
/// §4.2 calls for, and a target failed on a laptop under thermal throttling is
/// not evidence of a regression. What the numbers are for is comparison over
/// time on the *same* machine — §4.2's "graph of samples/sec over the
/// project's life is the most useful single artifact for catching slow
/// regressions".
fn benchmark(scene: BenchScene) -> Result<(), String> {
    // 1920x1080, which is the resolution the targets are stated at.
    const WIDTH: usize = 1920;
    const HEIGHT: usize = 1080;
    const SAMPLES: u32 = 64;

    let start = Instant::now();
    tequila_materials::energy::prewarm();
    let prewarm = start.elapsed();
    let device = Device::shared()?;

    let start = Instant::now();
    let document = scene.build();
    let build = start.elapsed();

    let environment = Arc::new(tequila_scene::demo::studio_environment());
    let start = Instant::now();
    let (snapshot_scene, _) = snapshot(&document, environment, &mut MeshCache::new());
    let flatten = start.elapsed();
    let triangles = snapshot_scene.triangle_count();
    let instances = snapshot_scene.instances.len();

    let start = Instant::now();
    let mut intersector = EmbreeIntersector::build(&device, snapshot_scene.clone())?;
    let bvh = start.elapsed();

    // §15's refit target: what a transform change costs once the BVH exists.
    let mut moved = snapshot_scene.clone();
    for instance in &mut moved.instances {
        instance.transform = tequila_core::math::Transform::translation(vec3(0.0, 0.01, 0.0));
    }
    let start = Instant::now();
    let refitted = intersector.update_transforms(&moved);
    let refit = start.elapsed();

    let bounds = visible_bounds(&document);
    let centre = bounds.center();
    let radius = (bounds.diagonal().length() * 0.5).max(1e-4);
    let camera = Camera::look_at(
        centre + vec3(0.75, 0.45, 1.0).normalize() * radius * 2.6,
        centre,
        Vec3::Y,
    );

    let context = RenderContext {
        camera,
        config: LightingPreset::Product.config(),
        scene_seed: 0,
        reject_fireflies: true,
        region: None,
        ..Default::default()
    };

    // §15's "time to first displayed pixels after a camera move" is measured
    // in the regime a camera move puts the renderer into, not at rest. The
    // schedule drops to a reduced resolution while the user is still moving
    // and climbs back afterwards — measuring the full-resolution pass instead
    // would report a number the user never waits for.
    let mut interactive = ProgressiveSchedule::default();
    interactive.interrupt();
    let (iw, ih) = interactive.render_resolution(WIDTH, HEIGHT);
    let mut first_frame = Framebuffer::new(iw, ih);
    let start = Instant::now();
    render_pass(&intersector, &mut first_frame, &context, &interactive, 1);
    let first_pixels = start.elapsed();

    // And the same at rest, which is what the throughput below is measured at.
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);
    let mut framebuffer = Framebuffer::new(WIDTH, HEIGHT);
    let start = Instant::now();
    render_pass(&intersector, &mut framebuffer, &context, &schedule, 1);
    let full_pass = start.elapsed();

    let start = Instant::now();
    render_pass(&intersector, &mut framebuffer, &context, &schedule, SAMPLES);
    let elapsed = start.elapsed();
    let samples_per_second = (WIDTH * HEIGHT) as f64 * SAMPLES as f64 / elapsed.as_secs_f64();
    // Interactive frame rate: the reduced-resolution pass repeated, which is
    // what navigating actually costs.
    let interactive_fps = 1.0 / first_pixels.as_secs_f64();

    println!("scene            {}", scene.name());
    println!("resolution       {WIDTH}x{HEIGHT}");
    println!("triangles        {triangles}");
    println!("instances        {instances}");
    println!("meshes stored    {}", document.geometry.len());
    println!();
    println!("scene build      {:>8.1} ms", build.as_secs_f64() * 1000.0);
    println!(
        "flatten          {:>8.1} ms",
        flatten.as_secs_f64() * 1000.0
    );
    // The initial build is part of scene load, whose target is 15 s. The
    // 200 ms figure in §15 is for a *rebuild after a transform change*, and a
    // transform change no longer rebuilds — it refits, on the line below.
    println!(
        "bvh build        {:>8.1} ms   (part of scene load, target <= 15 s)",
        bvh.as_secs_f64() * 1000.0
    );
    if refitted {
        println!(
            "bvh refit        {:>8.1} ms   (transform change, target <= 20 ms at 5M tris)",
            refit.as_secs_f64() * 1000.0
        );
    } else {
        println!("bvh refit             n/a   (the scene needed a full rebuild)");
    }
    println!(
        "first pixels     {:>8.1} ms   (target <= 50 ms, at {iw}x{ih} interactive)",
        first_pixels.as_secs_f64() * 1000.0
    );
    println!(
        "interactive fps  {:>8.1}      (target >= 30)",
        interactive_fps
    );
    println!(
        "one full pass    {:>8.1} ms   at {WIDTH}x{HEIGHT}",
        full_pass.as_secs_f64() * 1000.0
    );
    println!(
        "energy tables    {:>8.1} ms",
        prewarm.as_secs_f64() * 1000.0
    );
    println!();
    println!(
        "throughput       {:>8.2} Mspp/s (target >= 8 on 16 cores, simple_product)",
        samples_per_second / 1.0e6
    );
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    println!("threads          {threads:>8}");
    if threads != 16 {
        // The targets are stated for sixteen cores. Comparing against them on
        // a different machine is not a measurement, it is a coincidence.
        println!();
        println!("note: the targets in §15 are stated for 16 cores; this machine has {threads}.");
    }

    Ok(())
}

/// Everything the flags ask for, in order: the selections, the exports,
/// then the render - one, or one per configuration.
fn run(options: &Options) -> Result<(), String> {
    let mut document = prepare(options)?;
    select(&mut document, options)?;
    let exporting = options.export_glb.is_some() || options.export_usd.is_some() || options.export_web.is_some() || options.export_spin.is_some();
    if let Some(path) = &options.export_usd {
        std::fs::write(path, tequila_format::usd::export(&document)).map_err(|e| format!("{path}: {e}"))?;
        println!("wrote {path}");
    }

    if let Some(path) = &options.export_glb {
        let exported = tequila_format::gltf::export(&document, &tequila_format::gltf::GltfOptions { every_model_set: false, ..Default::default() });
        std::fs::write(path, &exported.glb).map_err(|e| format!("{path}: {e}"))?;
        for warning in &exported.warnings {
            eprintln!("tequila: {warning}");
        }
        println!("wrote {path} ({:.1} MB)", exported.glb.len() as f64 / 1e6);
    }

    if let Some(dir) = &options.export_web {
        let folder = std::path::Path::new(dir);
        let images = options.photos.then(|| tequila_format::web::fresh_images_folder(folder));
        if images.is_some() && document.configurator.is_empty() {
            return Err("--photos needs a configurator in the project".into());
        }
        let environment = tequila_scene::environment::EnvironmentCache::new().environment(&document.environment);
        let web = tequila_format::web::WebOptions { live: true, prerendered: images.clone().map(|i| ("jpg".to_string(), i)), ..Default::default() };
        let report = tequila_format::web::write(folder, &document, &environment, &web).map_err(|e| format!("{dir}: {e}"))?;
        for warning in &report.warnings {
            eprintln!("tequila: {warning}");
        }
        println!("wrote {}", report.index.display());
        if let Some(images) = images {
            let template = folder.join(images).join("{configuration}.jpg");
            each_configuration(options, &document, &template.to_string_lossy())?;
        }
    }

    if let Some(dir) = &options.export_spin {
        let folder = std::path::Path::new(dir);
        let project = is_project(options);
        let width = options.width.unwrap_or(if project { document.render.width } else { 960 });
        let height = options.height.unwrap_or(if project { document.render.height } else { 600 });
        // Round from the camera a plain render would use.
        let (base, target) = if project || options.camera.is_some() || options.studio.is_some() {
            let saved = document.cameras.active();
            (saved.camera, saved.target)
        } else {
            framing(options, &document, width as f32 / height as f32)
        };
        let spin = tequila_format::spin::SpinOptions {
            columns: options.columns.clamp(1, 360),
            rows: options.rows.clamp(1, 19),
            tilt: options.tilt,
            extension: if options.alpha || options.silhouette { "png".into() } else { "jpg".into() },
            ..Default::default()
        };
        let title = if document.meta.name.is_empty() { "Tequila".to_string() } else { document.meta.name.clone() };
        tequila_format::spin::write_package(folder, &title, &spin, width, height).map_err(|e| format!("{dir}: {e}"))?;
        let frames = tequila_format::spin::frames(&base, target, &spin);
        let count = frames.len();
        for (i, frame) in frames.into_iter().enumerate() {
            println!("spin frame {}/{count}", i + 1);
            let path = folder.join("frames").join(format!("{}.{}", frame.name, spin.extension));
            render_one(options, &document, &path.to_string_lossy(), Some(frame.camera))?;
        }
        tequila_format::spin::finish(folder, &title, &spin, width, height)?;
        println!("wrote {}", folder.join("index.html").display());
    }

    // An export-only run renders nothing more, unless it asked for every
    // configuration.
    let rendering = options.all_configurations || options.all_studios || !exporting || options.output_set;
    if !rendering {
        Ok(())
    } else if options.farm.is_some() {
        farm::dispatch(options, &document)
    } else if options.all_configurations {
        each_configuration(options, &document, &options.output)
    } else if options.all_studios {
        each_studio(options, &document, options.output_set.then_some(options.output.as_str())).map(|_| ())
    } else {
        render_one(options, &document, &options.output, None)
    }
}

/// Render every valid configuration of the configurator to `template`, the
/// choices standing in for `{configuration}` (added before the extension
/// when it is missing).
fn each_configuration(options: &Options, document: &tequila_scene::SceneDocument, template: &str) -> Result<(), String> {
    let configurator = &document.configurator;
    if configurator.is_empty() {
        return Err("the project has no configurator to render the configurations of".into());
    }
    let all = configurator.combinations();
    for (n, choices) in all.iter().enumerate() {
        let mut copy = document.clone();
        tequila_scene::CommandBus::new()
            .apply(&mut copy, tequila_scene::Command::Configure { choices: choices.clone() })
            .map_err(|e| e.to_string())?;
        let words = configurator.describe(choices).join(" - ");
        let output = output_for(template, &words);
        println!("configuration {}/{}: {words}", n + 1, all.len());
        render_one(options, &copy, &output, None)?;
    }
    Ok(())
}

/// Render every Studio - every view - one file each, as the application's
/// Render all views does: to `template` (`-o`), with `{view}` added when it
/// has none, or to the project's own output folder and file-name template.
/// The files written.
pub(crate) fn each_studio(options: &Options, document: &tequila_scene::SceneDocument, template: Option<&str>) -> Result<Vec<String>, String> {
    use tequila_format::output::{expand_template, first_free, output_path, ImageFormat, NameFields};
    if document.studios.is_empty() {
        return Err("the project has no Studios to render; apply a studio preset or save some views first".into());
    }
    let (date, time) = (tequila_core::clock::date(), tequila_core::clock::time_for_filename());
    let mut written = Vec::new();
    for (index, studio) in document.studios.iter().enumerate() {
        let mut copy = document.clone();
        tequila_scene::CommandBus::new()
            .apply(&mut copy, tequila_scene::Command::ApplyStudio { index })
            .map_err(|e| format!("Studio {}: {e}", studio.name))?;
        let settings = &copy.render;
        let fields = NameFields {
            project: if copy.meta.name.is_empty() { "Tequila".into() } else { copy.meta.name.clone() },
            camera: copy.cameras.active().name.clone(),
            studio: studio.filename_suffix.clone().unwrap_or_else(|| studio.name.clone()),
            product: tequila_scene::studio::product_name(&copy),
            width: options.width.unwrap_or(settings.width),
            height: options.height.unwrap_or(settings.height),
            samples: options.samples.unwrap_or(settings.max_samples),
            date: date.clone(),
            time: time.clone(),
            ..Default::default()
        };
        let path = match template {
            Some(template) => {
                let path = std::path::Path::new(template);
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let stem = if stem.contains("{view}") || stem.contains("{studio}") { stem } else { format!("{stem}_{{view}}") };
                let extension = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
                path.with_file_name(format!("{}.{extension}", expand_template(&stem, &fields)))
            }
            None => {
                let directory = if settings.output_directory.is_empty() { std::path::PathBuf::from(".") } else { std::path::PathBuf::from(&settings.output_directory) };
                first_free(output_path(&directory, &settings.filename_template, &fields, ImageFormat::Png(tequila_format::psd::Depth::Eight)))
            }
        };
        let path = path.to_string_lossy().into_owned();
        println!("view {}/{}: {}", index + 1, document.studios.len(), studio.name);
        render_one(options, &copy, &path, None)?;
        written.push(path);
    }
    Ok(written)
}

/// Apply the studio preset the flags name: an installed one's id, or its
/// folder or `.studio.json`.
fn apply_studio_preset(document: &mut tequila_scene::SceneDocument, options: &Options) -> Result<(), String> {
    use tequila_format::studio_preset as presets;
    let Some(wanted) = &options.studio_preset else { return Ok(()) };
    let path = std::path::Path::new(wanted);
    let file = if path.is_dir() {
        let found = presets::file_in(path).ok_or_else(|| format!("{wanted} holds no {} file", presets::EXTENSION))?;
        presets::read(&found, presets::PresetSource::User)?
    } else if path.is_file() {
        presets::read(path, presets::PresetSource::User)?
    } else {
        presets::find(wanted)?
    };
    let request = {
        let mut fields = tequila_format::json_write::Object::new();
        fields.set_some("front", options.front.clone());
        fields.build()
    };
    let choices = tequila_scripting::studio_presets::options(&request)?;
    let mut bus = tequila_scene::CommandBus::new();
    bus.apply(document, tequila_scene::Command::ApplyStudioPreset { preset: Box::new(file.preset.clone()), options: choices })
        .map_err(|e| e.to_string())?;
    if options.dress {
        for command in tequila_scene::preset::dressing(document, &file.preset) {
            bus.apply(document, command).map_err(|e| e.to_string())?;
        }
    }
    println!("applied the studio preset {} ({} views)", file.preset.name, file.preset.studios.len());
    Ok(())
}

/// `template` with `{configuration}` expanded the way the render queue
/// expands it, so a file named here matches the web export's photo names.
fn output_for(template: &str, configuration: &str) -> String {
    let path = std::path::Path::new(template);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = if stem.contains("{configuration}") { stem } else { format!("{stem} {{configuration}}") };
    let fields = tequila_format::output::NameFields { configuration: configuration.into(), ..Default::default() };
    let name = tequila_format::output::expand_template(&stem, &fields);
    let extension = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into());
    path.with_file_name(format!("{name}.{extension}")).to_string_lossy().into_owned()
}

/// A name or a number among `names`.
fn pick(what: &str, wanted: &str, names: &[String]) -> Result<usize, String> {
    if let Some(i) = names.iter().position(|n| n.eq_ignore_ascii_case(wanted.trim())) {
        return Ok(i);
    }
    match wanted.trim().parse::<usize>() {
        Ok(i) if i < names.len() => Ok(i),
        _ => Err(format!("there is no {what} {wanted:?}; the project has {}", if names.is_empty() { "none".into() } else { names.join(", ") })),
    }
}

/// The saved camera, Studio, model set and configuration the flags name.
fn select(document: &mut tequila_scene::SceneDocument, options: &Options) -> Result<(), String> {
    let mut bus = tequila_scene::CommandBus::new();
    if let Some(name) = &options.studio {
        let names: Vec<String> = document.studios.iter().map(|s| s.name.clone()).collect();
        let index = pick("Studio", name, &names)?;
        bus.apply(document, tequila_scene::Command::ApplyStudio { index }).map_err(|e| e.to_string())?;
    }
    if let Some(name) = &options.camera {
        let names: Vec<String> = document.cameras.iter().map(|(_, c)| c.name.clone()).collect();
        let index = pick("camera", name, &names)?;
        document.cameras.select(index);
    }
    if let Some(name) = &options.model_set {
        let names: Vec<String> = document.model_sets.sets.iter().map(|s| s.name.clone()).collect();
        document.model_sets.active = Some(pick("model set", name, &names)?);
    }
    if let Some(wanted) = &options.configuration {
        let configurator = document.configurator.clone();
        let parts: Vec<&str> = wanted.split(',').collect();
        if parts.len() != configurator.options.len() {
            return Err(format!("the configurator has {} options; give a choice for each, comma-separated", configurator.options.len()));
        }
        let mut choices = Vec::with_capacity(parts.len());
        for (option, part) in configurator.options.iter().zip(parts) {
            let labels: Vec<String> = option.choices.iter().map(|c| c.label.clone()).collect();
            choices.push(pick(&format!("choice of {}", option.name), part, &labels)?);
        }
        bus.apply(document, tequila_scene::Command::Configure { choices }).map_err(|e| format!("that configuration cannot be had: {e}"))?;
    }
    Ok(())
}

/// The document as the flags leave it: loaded, its lighting overridden.
fn prepare(options: &Options) -> Result<tequila_scene::SceneDocument, String> {
    let mut document = load_document(options)?;
    apply_studio_preset(&mut document, options)?;

    // The flags, over whatever the project said.
    if let Some(name) = &options.pins {
        document.environment.pins =
            tequila_render_core::pins::studio_preset(name).unwrap_or_default();
    }
    if let Some(name) = &options.sky {
        document.environment.sky = tequila_render_core::sky::time_of_day(name);
    }
    if let Some(path) = &options.hdri {
        document.environment.image = Some(path.clone());
        document.environment.sky = None;
    }
    if let Some(degrees) = options.hdri_rotation {
        document.environment.rotation = degrees.to_radians();
    }
    if options.no_ground {
        document.environment.ground.enabled = false;
    }
    Ok(document)
}

/// The camera a render without a saved one is taken from, and what it looks
/// at.
///
/// Framed on what was loaded — but only for a loaded file. An imported part
/// may be 3 mm or 3 km across and nowhere near the origin, so it needs
/// framing; the demo scene has a fixed camera composed for it, and framing
/// that on the bounds instead would fit the 80-unit ground plane and leave
/// the subject a speck in the middle of it.
fn framing(options: &Options, document: &tequila_scene::SceneDocument, aspect: f32) -> (Camera, Vec3) {
    let bounds = if options.input.is_some() {
        visible_bounds(document)
    } else {
        tequila_core::math::Aabb::EMPTY
    };
    let (eye, target) = if bounds.is_empty() {
        let r = tequila_scene::demo::STARTER_RADIUS;
        (vec3(3.2, 2.1, 4.4) * r, vec3(0.0, 0.95, 0.0) * r)
    } else {
        let centre = bounds.center();
        let radius = (bounds.diagonal().length() * 0.5).max(1e-4);
        (centre + vec3(0.85, 0.55, 1.15).normalize() * radius * 3.0, centre)
    };
    let mut camera = Camera::look_at(eye, target, Vec3::Y);
    camera.set_vertical_fov(0.62, aspect);
    camera.keep_focus = true;
    (camera, target)
}

/// Whether the input is a saved project, which renders as it was set up -
/// or has a studio preset applied, whose settings it then renders with.
fn is_project(options: &Options) -> bool {
    if options.studio_preset.is_some() {
        return true;
    }
    // Freeshot, as it was called before, saved them as .freeshot.
    options.input.as_deref().is_some_and(|p| {
        let p = p.to_ascii_lowercase();
        p.ends_with(".tequila") || p.ends_with(".freeshot")
    })
}

/// One render of `document` to `output`, from `camera` when given, else
/// the project's own or one framed on the scene.
fn render_one(options: &Options, document: &tequila_scene::SceneDocument, output: &str, camera_override: Option<Camera>) -> Result<(), String> {
    let start = Instant::now();
    tequila_materials::energy::prewarm();
    let prewarm = start.elapsed();
    let device = Device::shared()?;

    // Through the cache, because that is what knows how to turn a document's
    // environment into one — a sky, a loaded HDRI or the built-in studio, with
    // the pins painted on and the CDF rebuilt.
    //
    // This was `studio_environment()` unconditionally, which meant a project
    // rendered headlessly was lit by the built-in studio however it had been
    // set up in the application. Nothing reported it; the image simply came
    // out lit the wrong way.
    let mut environments = tequila_scene::environment::EnvironmentCache::new();
    let environment = environments.environment(&document.environment);

    // A saved project renders the way the application renders it: its
    // resolution, samples, camera, lighting, look, denoising and alpha. The
    // flags override; the demo and an imported model get the application's
    // defaults, which a new document carries. This rendered every project
    // from an angle of its own, under product lighting and AgX, with a
    // transparent background — a different picture from the one saved.
    let project = is_project(options);
    let settings = document.render.clone();
    let width = options.width.unwrap_or(if project { settings.width } else { 960 });
    let height = options.height.unwrap_or(if project { settings.height } else { 600 });
    let samples = options.samples.unwrap_or(if project { settings.max_samples } else { 128 });
    let mut config = options.preset.map(|p| p.config()).unwrap_or_else(|| document.lighting.config());
    if let Some(caustics) = options.caustics {
        config.caustics = caustics;
    }
    // A project's adaptive sampling switched off, as the Render window
    // renders it: zero retires nothing.
    if project && !settings.adaptive {
        config.adaptive_sampling_threshold = 0.0;
    }
    let mut style = document.image_style.clone();
    if let Some(exposure) = options.exposure {
        style.exposure = exposure;
    }
    if let Some(display) = options.display {
        style.display = display;
    }
    let denoise = options.denoise && (!project || settings.denoise);
    let denoise_strength = if project { settings.denoise_strength } else { 1.0 };
    // The Silhouette cut renders the picture whole, then cuts it out.
    let silhouette = options.silhouette || (project && settings.alpha_mode == tequila_scene::render::AlphaMode::Silhouette);
    let keep_alpha = !silhouette && (options.alpha || (project && settings.alpha_mode.transparent_background()));
    let saved_camera = camera_override.or_else(|| project.then(|| document.cameras.active().camera));
    // A one-shot render, so nothing is reused and the cache is a local.
    let (mut scene, _) = snapshot(document, environment, &mut MeshCache::new());
    // And behind it the project's background - a colour, a backplate - as
    // the application shows it. Left out, every project rendered here showed
    // its environment behind the product whatever had been chosen.
    scene.background = environments.background(&document.environment);
    let triangles = scene.triangle_count();
    let (cycles, cycles_device, why) = engine_choice(options.cycles, options.device.as_deref(), project.then_some(&settings), &tequila_render_cycles::devices);
    if let Some(why) = why {
        println!("  {why}");
    }
    let cycles_scene = cycles.then(|| scene.clone());

    let build_start = Instant::now();
    let intersector = EmbreeIntersector::build(&device, scene)?;
    let build = build_start.elapsed();

    let aspect = width as f32 / height as f32;
    let camera = match saved_camera {
        Some(camera) => camera,
        None => framing(options, document, aspect).0,
    };

    let context = RenderContext {
        camera,
        config,
        scene_seed: 0,
        // The project's own, as the Render window renders with.
        reject_fireflies: if project { settings.reject_fireflies } else { true },
        clamp_sample: if project { settings.clamp_indirect } else { None },
        region: None,
        ..Default::default()
    };

    // A final render, so the progressive schedule is at rest: full resolution,
    // adaptive sampling on.
    let mut schedule = ProgressiveSchedule::default();
    schedule.update(60_000);

    if let Some(scene) = cycles_scene {
        let cut = silhouette.then(|| tequila_render_cpu::Silhouette::trace(&intersector, &camera, width, height, SILHOUETTE_SAMPLES, 0));
        return render_with_cycles(&cycles_device, output, &scene, camera, width, height, samples, keep_alpha, &context.config, denoise, style, cut, project.then_some(&settings));
    }

    let mut framebuffer = Framebuffer::new(width, height);
    let render_start = Instant::now();
    println!(
        "rendering {}x{} at {} spp, {} triangles",
        width, height, samples, triangles
    );

    // In batches, so progress is visible on a long render rather than the
    // process appearing to hang.
    const BATCH: u32 = 8;
    let mut done = 0;
    while done < samples {
        let batch = BATCH.min(samples - done);
        render_pass(&intersector, &mut framebuffer, &context, &schedule, batch);
        done += batch;
        print!(
            "\r  {done}/{} spp   {:.0}% converged",
            samples,
            framebuffer.converged_fraction() * 100.0
        );
        use std::io::Write;
        let _ = std::io::stdout().flush();
    }
    println!();
    let render_time = render_start.elapsed();

    let denoised = if denoise {
        let denoise_start = Instant::now();
        let result = Denoiser::shared().ok().and_then(|denoiser| {
            // With alpha, the foreground alone: see `Pixel::mean_foreground`.
            let buffers = framebuffer.to_denoiser_buffers_with(keep_alpha);
            denoiser
                .denoise(
                    width,
                    height,
                    &buffers.color,
                    &buffers.albedo,
                    &buffers.normal,
                    DenoiseSettings {
                        quality: tequila_denoise::Quality::High,
                        strength: denoise_strength,
                        // A project's own choice; otherwise, at a final
                        // render's sample count the guides are already
                        // clean, and prefiltering them would only blur
                        // detail out of the result.
                        prefilter_guides: if project { settings.prefilter_guides } else { samples < 64 },
                    },
                )
                .ok()
        });
        if result.is_some() {
            println!(
                "  denoised in {:.2} s",
                denoise_start.elapsed().as_secs_f32()
            );
        }
        result
    } else {
        None
    };


    // Gathered first and developed as one image, through the same call the
    // application and the render queue use. Developing pixel by pixel here was
    // a second code path, and it had already drifted: it quantized without the
    // dither, so the same scene rendered from the command line came out with
    // banding the application did not have.
    let mut scene_linear = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let color = match &denoised {
                Some(image) => {
                    let i = (y * width + x) * 3;
                    tequila_core::spectrum::Spectrum::from_rendering_rgb(vec3(
                        image.data[i],
                        image.data[i + 1],
                        image.data[i + 2],
                    ))
                }
                None if keep_alpha => framebuffer.pixel(x, y).mean_foreground(),
                None => framebuffer.pixel(x, y).mean(),
            };
            scene_linear.push(color.sanitized());
        }
    }
    // Cut out along the outline: the same camera rays cast again, keeping
    // what they hit of the product, as the render queue does.
    let mut cut_alpha = None;
    if silhouette {
        let cut = tequila_render_cpu::Silhouette::trace(&intersector, &camera, width, height, SILHOUETTE_SAMPLES, 0);
        let (color, alpha) = cut.cut(&scene_linear);
        scene_linear = straight(width, height, color, alpha.clone(), style);
        cut_alpha = Some(alpha);
    }
    // A PNG's alpha is straight: the colour is the foreground's own, the
    // coverage beside it. The foreground above is premultiplied, so it is
    // divided back out — and carried past the edge — as the application's
    // writer does, or glass that covers a fifth of its pixels comes out a
    // fifth as bright.
    // The coverage, denoised as the colour was (`smooth_alpha`).
    let coverage: Option<Vec<f32>> = keep_alpha.then(|| {
        let alpha = (0..height).flat_map(|y| (0..width).map(move |x| (x, y))).map(|(x, y)| framebuffer.pixel(x, y).alpha()).collect();
        smooth_alpha(width, height, alpha, denoise)
    });
    if let Some(alpha) = &coverage {
        scene_linear = tequila_format::output::RenderOutput {
            width,
            height,
            color: scene_linear,
            alpha: alpha.clone(),
            layers: Vec::new(),
            style,
            display: tequila_color::DisplayPipeline::Builtin(style.display),
            alpha_mode: tequila_format::output::AlphaMode::Straight,
        }
        .unmatted();
    }
    // Opaque unless alpha was asked for: the picture as it looks, background
    // and all, is what a viewer shows well. With alpha, coverage, so the
    // image composites over a backplate (`docs/01` §12).
    let alpha = cut_alpha.as_deref().or(coverage.as_deref());
    let wide = project && settings.effective_depth() != tequila_scene::render::BitDepth::Eight;
    finish_image(output, width, height, &scene_linear, alpha, &style, wide)?;

    println!(
        "wrote {} ({}x{})",
        output, width, height
    );
    println!(
        "  tables {:.0} ms · bvh {:.0} ms · render {:.2} s · {:.2} Mspp/s",
        prewarm.as_secs_f32() * 1000.0,
        build.as_secs_f32() * 1000.0,
        render_time.as_secs_f32(),
        (samples as f64 * (width * height) as f64)
            / render_time.as_secs_f64()
            / 1.0e6
    );
    Ok(())
}

/// An image's alpha, denoised when the render is: denoising the colour
/// leaves it as sampled, so a transparent floor's shadow and reflection were
/// grainy (`Denoiser::denoise_alpha`). As it was without the denoiser.
fn smooth_alpha(width: usize, height: usize, alpha: Vec<f32>, denoise: bool) -> Vec<f32> {
    if !denoise {
        return alpha;
    }
    match Denoiser::shared().and_then(|d| d.denoise_alpha(width, height, &alpha)) {
        Ok(smooth) => smooth,
        Err(_) => alpha,
    }
}

/// Which engine renders here, the Cycles device, and why when it is not
/// the one named: the flags when given; otherwise a project's own engine,
/// with the application's fallbacks - Tequila's renderer for passes,
/// layers and regions, or without Cycles; Cycles on the processor without
/// a graphics card it can use - and Tequila's renderer for a model.
fn engine_choice(
    flag: Option<bool>,
    device: Option<&str>,
    project: Option<&tequila_scene::render::RenderSettings>,
    devices: &dyn Fn() -> Result<Vec<(String, String, String)>, String>,
) -> (bool, String, Option<String>) {
    use tequila_scene::render::RenderEngine;
    let named = |fallback: &str| device.unwrap_or(fallback).to_string();
    if let Some(cycles) = flag {
        return (cycles, named("CPU"), None);
    }
    let Some(settings) = project.filter(|s| s.engine.is_cycles()) else {
        return (false, named("CPU"), None);
    };
    if settings.effective_write_passes() || settings.relight || !settings.layers.is_empty() || settings.region.is_some() {
        return (false, named("CPU"), Some("passes, light groups, layers and regions are rendered by Tequila's own renderer".into()));
    }
    let found = match devices() {
        Ok(found) => found,
        Err(why) => return (false, named("CPU"), Some(format!("Cycles is not available ({why}), so Tequila's own renderer renders"))),
    };
    if device.is_some() || settings.engine != RenderEngine::CyclesGpu {
        return (true, named(settings.engine.cycles_device()), None);
    }
    let has = |kind: &str| found.iter().any(|(k, _, _)| k == kind);
    let wanted = settings.engine.cycles_device();
    let card = if has(wanted) { Some(wanted) } else if !cfg!(target_os = "macos") && has("CUDA") { Some("CUDA") } else { None };
    match card {
        Some(card) => (true, card.to_string(), None),
        None => (true, "CPU".into(), Some("Cycles found no graphics card it can use, so it renders on the processor".into())),
    }
}

/// The same image through Cycles: the scene translated, rendered, and then
/// developed exactly as Tequila's own render is.
#[allow(clippy::too_many_arguments)]
fn render_with_cycles(
    device: &str,
    output: &str,
    scene: &tequila_render_core::scene::SceneSnapshot,
    camera: Camera,
    width: usize,
    height: usize,
    samples: u32,
    keep_alpha: bool,
    lighting: &tequila_render_core::settings::IntegratorConfig,
    denoise: bool,
    style: tequila_color::ImageStyle,
    cut: Option<tequila_render_cpu::Silhouette>,
    project: Option<&tequila_scene::render::RenderSettings>,
) -> Result<(), String> {
    use tequila_render_cycles::{translate, Session};
    use std::io::Write;

    let start = Instant::now();
    let mut session = Session::new(device, 0, std::env::var_os("TEQUILA_CYCLES_KERNELS").map(std::path::PathBuf::from).as_deref())?;
    let view = translate::View { camera, width, height, transparent: keep_alpha, ground_illumination: lighting.ground_illumination, light_materials: lighting.light_source_materials };
    let translation = translate::build(&mut session, scene, &view)?;
    for warning in &translation.warnings {
        println!("  cycles: {warning}");
    }
    let under = translation.background_under.then(|| translate::background(scene, &camera, width, height));
    // A project's adaptive sampling, time limit, clamping and prefilter, as
    // the Render window renders it; otherwise Cycles' defaults.
    let quality = match project {
        Some(settings) => translate::Quality::with_lighting(settings.adaptive, denoise, settings.max_seconds.map(f64::from), lighting)
            .with_clamping(settings.reject_fireflies, settings.clamp_indirect, settings.prefilter_guides)
            .with_render_controls(settings.clamp_direct, settings.pixel_filter, settings.adaptive_threshold),
        None => translate::Quality::with_lighting(true, denoise, None, lighting),
    };
    translate::configure(&mut session, &quality)?;
    let built = start.elapsed();

    println!(
        "rendering {}x{} at {} spp with Cycles on {}",
        width, height, samples, device
    );
    let start = Instant::now();
    let rgba = session.render(width, height, samples, |fraction, _| {
        print!("\r  {:.0}%", fraction * 100.0);
        let _ = std::io::stdout().flush();
    })?;
    println!();
    let render_time = start.elapsed();

    // Cycles' film is premultiplied; a PNG's alpha is straight.
    let mut scene_linear = Vec::with_capacity(width * height);
    let mut alpha = Vec::with_capacity(width * height);
    for (i, px) in rgba.chunks_exact(4).enumerate() {
        let a = if keep_alpha { px[3].clamp(0.0, 1.0) } else { 1.0 };
        let unmatte = if keep_alpha && a > 1e-6 { 1.0 / a } else { 1.0 };
        scene_linear.push(match &under {
            Some(background) => translate::lay_under(px, background[i]),
            None => translate::to_rendering(px[0] * unmatte, px[1] * unmatte, px[2] * unmatte).sanitized(),
        });
        alpha.push(a);
    }
    // The floor's reflection, which Cycles was left without: Tequila's,
    // through the same camera, laid over (`tequila_render_cpu::floor`).
    if translation.floor_reflection {
        let device = Device::shared().map_err(|e| e.to_string())?;
        let intersector = EmbreeIntersector::build(&device, scene.clone()).map_err(|e| e.to_string())?;
        let mut layer = Framebuffer::new(width, height);
        tequila_render_cpu::floor::render_floor_reflection(&intersector, &mut layer, camera, lighting, (samples / 2).clamp(16, 512));
        let guides = layer.to_denoiser_buffers();
        let denoised = denoise
            .then(|| Denoiser::shared().ok())
            .flatten()
            .and_then(|d| d.denoise(width, height, &guides.color, &guides.albedo, &guides.normal, DenoiseSettings::default()).ok());
        let reflected: &[f32] = denoised.as_ref().map_or(&guides.color, |image| &image.data);
        // Each pixel's share, denoised as the colour is: as sampled, a
        // blurred reflection's fade put specks on a light backdrop.
        let shares = smooth_alpha(width, height, layer.pixels().iter().map(|p| p.alpha()).collect(), denoise);
        let bright = tequila_render_cpu::floor::alpha_by_brightness(&scene);
        for (i, &share) in shares.iter().enumerate() {
            if share > 0.0 {
                let reflection = tequila_core::spectrum::Spectrum::from_rendering_rgb(vec3(
                    reflected[3 * i].max(0.0),
                    reflected[3 * i + 1].max(0.0),
                    reflected[3 * i + 2].max(0.0),
                ));
                // Premultiplied to lay over, straight again for the file.
                let before = alpha[i];
                let premultiplied = if keep_alpha { scene_linear[i] * before } else { scene_linear[i] };
                let (reflection, share) =
                    if bright { tequila_render_cpu::floor::by_brightness(reflection, share, style.exposure) } else { (reflection, share) };
                let (colour, after) = tequila_render_cpu::floor::lay_reflection(premultiplied, before, reflection, share);
                alpha[i] = after;
                scene_linear[i] = if keep_alpha && after > 1e-6 { colour * (1.0 / after) } else { colour };
            }
        }
    }
    // Cycles leaves its alpha as sampled: a transparent shadow was grainy.
    if keep_alpha {
        alpha = smooth_alpha(width, height, alpha, denoise);
    }
    let keep_alpha = keep_alpha || cut.is_some();
    if let Some(cut) = cut {
        let (color, cut_alpha) = cut.cut(&scene_linear);
        scene_linear = straight(width, height, color, cut_alpha.clone(), style);
        alpha = cut_alpha;
    }
    let wide = project.is_some_and(|s| s.effective_depth() != tequila_scene::render::BitDepth::Eight);
    finish_image(output, width, height, &scene_linear, keep_alpha.then_some(&alpha[..]), &style, wide)?;
    println!("wrote {} ({}x{})", output, width, height);
    println!(
        "  scene {:.0} ms · render {:.2} s",
        built.as_secs_f32() * 1000.0,
        render_time.as_secs_f32()
    );
    Ok(())
}

/// Rays a pixel for the Silhouette outline, as the render queue casts.
const SILHOUETTE_SAMPLES: u32 = 16;

/// Premultiplied colour and coverage, as straight colour for an image file.
fn straight(width: usize, height: usize, color: Vec<tequila_core::spectrum::Spectrum>, alpha: Vec<f32>, style: tequila_color::ImageStyle) -> Vec<tequila_core::spectrum::Spectrum> {
    tequila_format::output::RenderOutput {
        width,
        height,
        color,
        alpha,
        layers: Vec::new(),
        style,
        display: tequila_color::DisplayPipeline::Builtin(style.display),
        alpha_mode: tequila_format::output::AlphaMode::Straight,
    }
    .unmatted()
}

/// A PNG, or a JPEG for `.jpg`/`.jpeg` (opaque, on white where the image
/// is see-through).
/// Develop the picture and write it: straight colour, and its coverage or
/// `None` for opaque. Sixteen bits a sample when `wide` and it is a PNG, as
/// the project's 16-bit setting asks.
fn finish_image(
    path: &str,
    width: usize,
    height: usize,
    scene_linear: &[tequila_core::spectrum::Spectrum],
    alpha: Option<&[f32]>,
    style: &tequila_color::ImageStyle,
    wide: bool,
) -> Result<(), String> {
    let display = tequila_color::DisplayPipeline::Builtin(style.display);
    let lower = path.to_ascii_lowercase();
    if wide && !(lower.ends_with(".jpg") || lower.ends_with(".jpeg")) {
        let rgb = style.develop_through_wide(width, height, scene_linear, &display);
        let mut samples = Vec::with_capacity(width * height * 8);
        for (i, pixel) in rgb.chunks_exact(3).enumerate() {
            let a = alpha.map_or(65535, |alpha| (alpha[i].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16);
            for value in [pixel[0], pixel[1], pixel[2], a] {
                samples.extend_from_slice(&value.to_be_bytes());
            }
        }
        make_parent(path)?;
        return write_png(path, width, height, &samples, png::BitDepth::Sixteen);
    }
    let mut pixels = style.develop_through(width, height, scene_linear, &display);
    for (i, a) in alpha.iter().flat_map(|a| a.iter()).enumerate() {
        pixels[i * 4 + 3] = (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    }
    write_image(path, width, height, &pixels)
}

fn make_parent(path: &str) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    Ok(())
}

fn write_image(path: &str, width: usize, height: usize, rgba: &[u8]) -> Result<(), String> {
    make_parent(path)?;
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        let rgb: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as u32;
                [0, 1, 2].map(|k| ((p[k] as u32 * a + 255 * (255 - a)) / 255) as u8)
            })
            .collect();
        return jpeg_encoder::Encoder::new_file(path, 92)
            .map_err(|e| format!("{path}: {e}"))?
            .encode(&rgb, width as u16, height as u16, jpeg_encoder::ColorType::Rgb)
            .map_err(|e| format!("{path}: {e}"));
    }
    write_png(path, width, height, rgba, png::BitDepth::Eight)
}

/// RGBA samples as a PNG: bytes at eight bits, big-endian pairs at sixteen.
fn write_png(path: &str, width: usize, height: usize, rgba: &[u8], depth: png::BitDepth) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("{path}: {e}"))?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(depth);
    // The output is sRGB-encoded by the display transform, so say so; without
    // the chunk, viewers guess, and some guess wrong.
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|e| format!("{path}: {e}"))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| format!("{path}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod engine_tests {
    use super::engine_choice;
    use tequila_scene::render::{Region, RenderEngine, RenderSettings};

    type Devices = Result<Vec<(String, String, String)>, String>;

    /// A project renders with the engine it is set to, as the Render window
    /// renders it, with the same fallbacks; the flags always win, and a
    /// model renders with Tequila's renderer as it always has.
    #[test]
    fn a_project_renders_with_its_own_engine() {
        let with = |engine| RenderSettings { engine, ..Default::default() };
        let gpu = with(RenderEngine::CyclesGpu);
        let card = RenderEngine::CyclesGpu.cycles_device().to_string();
        let with_card = || -> Devices { Ok(vec![(card.clone(), "a graphics card".into(), "0".into()), ("CPU".into(), "the processor".into(), "1".into())]) };
        let processor_only = || -> Devices { Ok(vec![("CPU".into(), "the processor".into(), "0".into())]) };
        let missing = || -> Devices { Err("tequila_cycles.dll is not installed".into()) };

        assert_eq!(engine_choice(Some(false), None, Some(&gpu), &with_card), (false, "CPU".into(), None), "the flag wins");
        assert_eq!(engine_choice(Some(true), Some("CUDA"), None, &with_card).1, "CUDA", "the device flag wins");
        assert_eq!(engine_choice(None, None, Some(&gpu), &with_card), (true, card.clone(), None));
        let (cycles, device, why) = engine_choice(None, None, Some(&gpu), &processor_only);
        assert!(cycles && device == "CPU" && why.is_some_and(|w| w.contains("no graphics card")));
        let (cycles, _, why) = engine_choice(None, None, Some(&gpu), &missing);
        assert!(!cycles && why.is_some_and(|w| w.contains("not available")));
        assert_eq!(engine_choice(None, None, Some(&with(RenderEngine::CyclesCpu)), &with_card), (true, "CPU".into(), None));
        assert_eq!(engine_choice(None, None, Some(&with(RenderEngine::Tequila)), &with_card), (false, "CPU".into(), None));
        let region = RenderSettings { region: Some(Region { x: 0, y: 0, width: 10, height: 10 }), ..gpu.clone() };
        let (cycles, _, why) = engine_choice(None, None, Some(&region), &with_card);
        assert!(!cycles && why.is_some(), "a region is Tequila's renderer's");
        assert_eq!(engine_choice(None, None, None, &with_card), (false, "CPU".into(), None), "a model");
    }
}
