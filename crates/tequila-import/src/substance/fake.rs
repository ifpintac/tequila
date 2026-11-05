//! A stand-in for Adobe's `sbsrender` and `sbscooker`, for testing the
//! bridge on a computer without them.
//!
//! It takes the same arguments the documentation gives the real tools and
//! writes what they would write — a PNG per output, named the way
//! `--output-name` asks, or an `.sbsar` per cooked `.sbs` — but what it
//! draws is its own: a brick wall that follows the bricks fixture's inputs
//! (colours, rows, offset, roughness, the Normal format, the random seed).
//! It knows nothing of a real graph: it reads the file's description with
//! Tequila's own reader, and every map it makes is a picture of bricks.
//! Anything shown made with it is labelled as such.
//!
//! The test suites run it as a separate program (the bridge test is its
//! own `main`), and `examples/fake_sbsrender.rs` builds it as one for
//! trying the application by hand.

use super::Graph;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Run as the tool would be run: `args` without the program's own name.
/// Returns the exit code.
pub fn main(args: &[String]) -> i32 {
    match run(args) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("[ERROR] {error}");
            1
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("fake sbsrender/sbscooker (a Tequila test stand-in, not Adobe's tool)");
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("render") {
        return render(&args[1..]);
    }
    if args.iter().any(|a| a == "--inputs") {
        return cook(args);
    }
    Err(format!("unrecognised arguments: {}", args.join(" ")))
}

/// Options and their values, each option once or more.
fn options(args: &[String]) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut at = 0;
    while at < args.len() {
        let name = &args[at];
        if !name.starts_with("--") {
            // The input may be given last, without its option.
            out.entry("--input".into()).or_default().push(name.clone());
            at += 1;
            continue;
        }
        let value = args.get(at + 1).ok_or_else(|| format!("{name} needs a value"))?;
        out.entry(name.clone()).or_default().push(value.clone());
        at += 2;
    }
    Ok(out)
}

fn one<'a>(options: &'a BTreeMap<String, Vec<String>>, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|n| options.get(*n).and_then(|v| v.first())).map(String::as_str)
}

fn cook(args: &[String]) -> Result<(), String> {
    let options = options(args)?;
    let input = PathBuf::from(one(&options, &["--inputs"]).ok_or("no --inputs")?);
    let output = PathBuf::from(one(&options, &["--output-path"]).unwrap_or("."));
    let text = std::fs::read_to_string(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let file = super::read_sbs(&text).map_err(|e| e.to_string())?;
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "cooked".into());
    let name = one(&options, &["--output-name"]).map(|n| n.replace("{inputName}", &stem)).unwrap_or(stem.clone());
    let name = if name.ends_with(".sbsar") { name } else { format!("{name}.sbsar") };
    std::fs::create_dir_all(&output).map_err(|e| format!("{}: {e}", output.display()))?;
    let bytes = super::fixture::sbsar(&stem, &file.graphs);
    std::fs::write(output.join(&name), bytes).map_err(|e| format!("{name}: {e}"))
}

fn render(args: &[String]) -> Result<(), String> {
    let options = options(args)?;
    let input = PathBuf::from(one(&options, &["--input", "--inputs"]).ok_or("no --input")?);
    let file = super::read(&input).map_err(|e| format!("{}: {e}", input.display()))?;
    let graph = file.graph(one(&options, &["--input-graph"])).ok_or("no such graph")?;
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    for pair in options.get("--set-value").into_iter().flatten() {
        let (id, value) = pair.split_once('@').ok_or_else(|| format!("--set-value {pair} is not identifier@value"))?;
        if graph.input(id).is_none() {
            return Err(format!("the graph has no input `{id}`"));
        }
        values.insert(id.to_string(), value.to_string());
    }
    let wanted: Vec<&String> = options.get("--input-graph-output").map(|v| v.iter().collect()).unwrap_or_default();
    let directory = PathBuf::from(one(&options, &["--output-path"]).unwrap_or("."));
    let pattern = one(&options, &["--output-name"]).unwrap_or("{inputName}_{inputGraphUrl}_{outputNodeName}");
    if one(&options, &["--output-format"]).is_some_and(|f| !f.eq_ignore_ascii_case("png")) {
        return Err("this stand-in writes PNG only".into());
    }
    std::fs::create_dir_all(&directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    let bricks = Bricks::from(graph, &values);
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    for output in graph.outputs.iter().filter(|o| wanted.is_empty() || wanted.iter().any(|w| **w == o.identifier)) {
        let name = pattern
            .replace("{outputNodeName}", &output.identifier)
            .replace("{inputName}", &stem)
            .replace("{inputGraphUrl}", graph.identifier())
            .replace("{outputLabel}", &output.label)
            .replace("{outputUsages}", &output.usages.join("_"));
        let name = if name.to_ascii_lowercase().ends_with(".png") { name } else { format!("{name}.png") };
        let usage = output.usages.first().map(|u| u.to_ascii_lowercase()).unwrap_or_else(|| output.identifier.to_ascii_lowercase());
        bricks.write(&usage, &directory.join(name))?;
    }
    Ok(())
}

/// The picture every map is drawn from.
struct Bricks {
    size: u32,
    rows: f64,
    offset: f64,
    brick: [f64; 3],
    mortar: [f64; 3],
    roughness: f64,
    worn: bool,
    directx: bool,
    seed: u64,
}

impl Bricks {
    fn from(graph: &Graph, values: &BTreeMap<String, String>) -> Bricks {
        let value = |id: &str| graph.input(id).map(|i| i.value_in(values));
        let numbers = |id: &str| value(id).map(|v| v.numbers().to_vec()).unwrap_or_default();
        // The colours: the inputs named so, or the first two colour inputs.
        let colours: Vec<Vec<f64>> = graph
            .inputs
            .iter()
            .filter(|i| matches!(i.kind(), super::InputKind::Color { .. }))
            .map(|i| i.value_in(values).numbers().to_vec())
            .collect();
        let colour = |id: &str, index: usize, fallback: [f64; 3]| {
            let n = {
                let named = numbers(id);
                if named.len() >= 3 { named } else { colours.get(index).cloned().unwrap_or_default() }
            };
            if n.len() >= 3 { [n[0], n[1], n[2]] } else { fallback }
        };
        let size = numbers("$outputsize").first().copied().unwrap_or(8.0).clamp(4.0, 10.0) as u32;
        Bricks {
            size: 1 << size,
            rows: numbers("rows").first().copied().unwrap_or(8.0).clamp(1.0, 64.0),
            offset: numbers("offset").first().copied().unwrap_or(0.5),
            brick: colour("brick_color", 0, [0.6, 0.25, 0.15]),
            mortar: colour("mortar_color", 1, [0.75, 0.75, 0.72]),
            roughness: numbers("roughness").first().copied().unwrap_or(0.7).clamp(0.0, 1.0),
            worn: numbers("worn").first().copied().unwrap_or(1.0) >= 0.5,
            directx: graph.normal_is_directx(values),
            seed: numbers("$randomseed").first().copied().unwrap_or(0.0) as u64,
        }
    }

    /// A number in [0, 1) for a brick.
    fn random(&self, row: i64, column: i64) -> f64 {
        let mut h = (row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (column as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ self.seed.wrapping_mul(0x1656_67B1_9E37_79F9);
        h ^= h >> 31;
        h = h.wrapping_mul(0xD6E8_FEB8_6659_FD93);
        h ^= h >> 32;
        (h >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Height (1 a brick's face, 0 the mortar) and the brick's random
    /// number, at `(u, v)` in the unit square.
    fn sample(&self, u: f64, v: f64) -> (f64, f64) {
        let rows = self.rows.round().max(1.0);
        let columns = (rows / 2.0).max(1.0);
        let y = v * rows;
        let row = y.floor();
        let x = u * columns + if row as i64 % 2 == 1 { self.offset } else { 0.0 };
        let column = x.floor();
        let (fx, fy) = (x - column, y - row);
        // Distance to the brick's edge, in brick heights.
        let edge = (fx.min(1.0 - fx) * 2.0).min(fy.min(1.0 - fy));
        let mortar = 0.06;
        let bevel = 0.12;
        let mut height = ((edge - mortar) / bevel).clamp(0.0, 1.0);
        height = height * height * (3.0 - 2.0 * height);
        let random = self.random(row as i64, (column as i64).rem_euclid(columns as i64));
        if self.worn && height > 0.0 {
            // Chipped corners: a little lower on some bricks' edges.
            let chip = ((fx * 37.0 + fy * 53.0 + random * 11.0).sin() * 0.5 + 0.5) * (1.0 - height);
            height -= chip * 0.35;
        }
        (height.clamp(0.0, 1.0) * (0.85 + 0.15 * random), random)
    }

    fn write(&self, usage: &str, path: &Path) -> Result<(), String> {
        let n = self.size as usize;
        let mut heights = vec![0.0f64; n * n];
        let mut randoms = vec![0.0f64; n * n];
        for y in 0..n {
            for x in 0..n {
                let (h, r) = self.sample((x as f64 + 0.5) / n as f64, (y as f64 + 0.5) / n as f64);
                heights[y * n + x] = h;
                randoms[y * n + x] = r;
            }
        }
        let at = |x: i64, y: i64| heights[(y.rem_euclid(n as i64) as usize) * n + x.rem_euclid(n as i64) as usize];
        let mut pixels = Vec::with_capacity(n * n * 3);
        let srgb = |linear: f64| {
            let c = linear.clamp(0.0, 1.0);
            if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
        };
        for y in 0..n as i64 {
            for x in 0..n as i64 {
                let h = at(x, y);
                let r = randoms[y as usize * n + x as usize];
                let rgb: [f64; 3] = match usage {
                    "basecolor" | "diffuse" => {
                        let face = h.clamp(0.0, 1.0);
                        let tint = 0.8 + 0.4 * r;
                        let mut c = [0.0; 3];
                        for i in 0..3 {
                            c[i] = srgb(self.mortar[i] * (1.0 - face) + self.brick[i] * tint * face);
                        }
                        c
                    }
                    "normal" => {
                        let strength = n as f64 / 24.0;
                        let dx = (at(x + 1, y) - at(x - 1, y)) * strength;
                        let dy = (at(x, y + 1) - at(x, y - 1)) * strength;
                        let length = (dx * dx + dy * dy + 1.0).sqrt();
                        // Image rows run down; OpenGL's green points up.
                        let (nx, ny, nz) = (-dx / length, dy / length, 1.0 / length);
                        let green = if self.directx { -ny } else { ny };
                        [nx * 0.5 + 0.5, green * 0.5 + 0.5, nz * 0.5 + 0.5]
                    }
                    "roughness" => {
                        let g = (self.roughness * (0.9 + 0.2 * r) * h + 0.95 * (1.0 - h)).clamp(0.0, 1.0);
                        [g; 3]
                    }
                    "glossiness" => [1.0 - self.roughness; 3],
                    "metallic" | "metalness" => [0.0; 3],
                    "height" | "displacement" => [h; 3],
                    "ambientocclusion" | "ao" => {
                        let blur = (at(x - 2, y) + at(x + 2, y) + at(x, y - 2) + at(x, y + 2)) / 4.0;
                        [(0.55 + 0.45 * (1.0 - (blur - h).abs() * 2.0) * (0.4 + 0.6 * h)).clamp(0.0, 1.0); 3]
                    }
                    "opacity" => [1.0; 3],
                    "emissive" | "emission" => [0.0; 3],
                    _ => [0.5; 3],
                };
                for c in rgb {
                    pixels.push((c.clamp(0.0, 1.0) * 255.0).round() as u8);
                }
            }
        }
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), self.size, self.size);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut w| w.write_image_data(&pixels))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The stand-in as a [`super::bridge::Runner`], in this process rather than
/// as a program: for tests that exercise everything but the process.
pub fn runner(_program: &Path, arguments: &[String]) -> Result<(), String> {
    match run(arguments) {
        Ok(()) => Ok(()),
        Err(error) => Err(format!("the stand-in failed: {error}")),
    }
}

