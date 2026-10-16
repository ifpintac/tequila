//! The web configurator export (`docs/03` §9): a static folder that opens in
//! any browser, from a web host or straight off the disk.
//!
//! Two modes, from the same export, either or both:
//!
//! - **Live 3D**: the scene as glTF ([`crate::gltf`]), drawn by
//!   `viewer.js` - Tequila's own small WebGL2 renderer, no library and no
//!   CDN - lit by the scene's environment, prefiltered here: a mip chain
//!   for reflections at each roughness and spherical harmonics for diffuse
//!   light, with a soft ground shadow baked from the parts. The picker
//!   switches materials, model sets and Studios instantly; the camera turns.
//! - **Pre-rendered**: one path-traced image per valid configuration,
//!   rendered by the queue into `images/`, shown as the picker changes. The
//!   real renderer's look, at the cost of a render per combination.
//!
//! Everything the page needs is in `data.js`, loaded by a `<script>` tag
//! rather than fetched: a page opened from the disk may not fetch its own
//! files, and "drop the folder anywhere" includes a desktop.

use crate::gltf::{export, GltfOptions};
use crate::json::Value;
use crate::json_write::Object;
use tequila_core::math::{vec3, Vec3};
use tequila_render_core::environment::Environment;
use tequila_scene::configurator::Drives;
use tequila_scene::document::{BackgroundMode, SceneDocument};
use tequila_scene::tree::NodeKind;
use std::path::{Path, PathBuf};

const VIEWER: &str = include_str!("../web/viewer.js");
const PAGE: &str = include_str!("../web/index.html");

/// The environment's sizes: the sharpest level's width, and how many levels
/// down to the roughest.
const ENV_WIDTH: usize = 512;
const ENV_LEVELS: usize = 6;
/// RGBM's range: radiance up to this many times white survives 8 bits.
const RGBM_RANGE: f32 = 16.0;

#[derive(Clone, Debug)]
pub struct WebOptions {
    /// Write the live 3D view.
    pub live: bool,
    /// Expect pre-rendered images, one per combination, of this extension
    /// (`jpg`, `png`), in this folder under the export.
    pub prerendered: Option<(String, String)>,
    /// Pixels on a side of a baked material atlas.
    pub bake_size: usize,
}

impl Default for WebOptions {
    fn default() -> Self {
        WebOptions { live: true, prerendered: None, bake_size: 1024 }
    }
}

#[derive(Debug, Default)]
pub struct WebReport {
    pub folder: PathBuf,
    /// The page to open.
    pub index: PathBuf,
    pub bytes: usize,
    pub warnings: Vec<String>,
}

/// The folder's `images` folder for pre-rendered images: a new one, never
/// one with renders already in it.
pub fn fresh_images_folder(folder: &Path) -> String {
    for n in 1.. {
        let name = if n == 1 { "images".to_string() } else { format!("images-{n}") };
        let path = folder.join(&name);
        let empty = std::fs::read_dir(&path).map(|mut d| d.next().is_none()).unwrap_or(true);
        if empty {
            return name;
        }
    }
    unreachable!()
}

/// Write the export into `folder`, creating it.
pub fn write(folder: &Path, document: &SceneDocument, environment: &Environment, options: &WebOptions) -> std::io::Result<WebReport> {
    std::fs::create_dir_all(folder)?;
    let mut report = WebReport { folder: folder.to_path_buf(), index: folder.join("index.html"), ..Default::default() };
    let configurator = &document.configurator;
    let mut data = Object::new();
    let title = if configurator.title.is_empty() { document.meta.name.clone() } else { configurator.title.clone() };
    data.set("title", title.clone()).set("subtitle", configurator.subtitle.clone()).set("placement", configurator.placement.key());

    // The live view's scene and its light.
    let exported = options.live.then(|| export(document, &GltfOptions { every_model_set: true, bake_size: options.bake_size }));
    if let Some(exported) = &exported {
        report.warnings.extend(exported.warnings.iter().cloned());
        data.set("glb", base64(&exported.glb));
        data.set("environment", environment_json(document, environment));
        if let Some(shadow) = shadow_json(document, exported) {
            data.set("shadow", shadow);
        }
        data.set("nodes", exported.nodes.len());
    }
    data.set("camera", camera_json(&document.cameras.active().camera, document.cameras.active().target));
    data.set("exposure", document.image_style.exposure);
    // The display transform, so the page develops the light as Tequila
    // does: AgX, which a browser cannot afford per pixel, falls back to
    // PBR Neutral, the closest of the others.
    data.set("display", match document.image_style.display {
        tequila_color::DisplayTransform::Aces => "aces",
        tequila_color::DisplayTransform::Srgb | tequila_color::DisplayTransform::Raw => "srgb",
        _ => "neutral",
    });
    data.set("aspect", document.render.width.max(1) as f32 / document.render.height.max(1) as f32);

    // The configurator: the same options and rules, and what each choice
    // does to the glTF scene.
    let mut options_json = Vec::new();
    for option in &configurator.options {
        let mut o = Object::new();
        o.set("id", option.id).set("name", option.name.clone()).set("style", option.style.key());
        let choices: Vec<Value> = option
            .choices
            .iter()
            .map(|choice| {
                let mut c = Object::new();
                c.set("label", choice.label.clone());
                if let Some(swatch) = swatch_of(document, option, choice) {
                    c.set("swatch", swatch);
                }
                if let Some(exported) = &exported {
                    c.set("effect", effect(document, exported, option.drives, choice.target));
                }
                c.build()
            })
            .collect();
        o.set("drives", match option.drives {
            Drives::Material { .. } => "material",
            Drives::ModelSet => "model_set",
            Drives::Studio => "studio",
        });
        o.set("choices", choices);
        options_json.push(o.build());
    }
    let rules: Vec<Value> = configurator
        .rules
        .iter()
        .map(|r| {
            let mut o = Object::new();
            o.set("option", r.option).set("choice", r.choice).set("other", r.other).set("other_choice", r.other_choice).set(
                "kind",
                match r.kind {
                    tequila_scene::configurator::RuleKind::Excludes => "excludes",
                    tequila_scene::configurator::RuleKind::Requires => "requires",
                },
            );
            o.build()
        })
        .collect();
    let current = configurator.settled().unwrap_or_default();
    data.set("options", options_json).set("rules", rules).set("current", current.iter().map(|&c| Value::from(c)).collect::<Vec<_>>());

    // Pre-rendered: the file each combination's render will be.
    if let Some((extension, images)) = &options.prerendered {
        let mut map = Object::new();
        for choices in configurator.combinations() {
            let key = choices.iter().map(|c| c.to_string()).collect::<Vec<_>>().join("-");
            let fields = crate::output::NameFields { configuration: configurator.describe(&choices).join(" - "), ..Default::default() };
            let stem = crate::output::expand_template("{configuration}", &fields);
            map.set(&key, format!("{images}/{}.{extension}", if stem.is_empty() { "render".into() } else { stem }));
        }
        data.set("images", map.build());
    }

    let data_js = format!("window.TEQUILA = {};\n", crate::json_write::to_string(&data.build()));
    let page = PAGE.replace("{{title}}", &html_escape(&title));
    std::fs::write(folder.join("index.html"), &page)?;
    std::fs::write(folder.join("viewer.js"), VIEWER)?;
    std::fs::write(folder.join("data.js"), &data_js)?;
    report.bytes = page.len() + VIEWER.len() + data_js.len();
    Ok(report)
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A swatch as `#rrggbb`, from the choice or its variant.
fn swatch_of(document: &SceneDocument, option: &tequila_scene::configurator::ConfigOption, choice: &tequila_scene::configurator::Choice) -> Option<String> {
    let rgb = match (choice.swatch, option.drives) {
        (Some(rgb), _) => rgb,
        (None, Drives::Material { multi_material }) => {
            let multi = document.multi_materials.iter().find(|m| m.id == multi_material)?;
            let m = document.materials.get(*multi.variants.get(choice.target)?)?.material();
            let v = (m.base_color * m.base_weight).to_linear_srgb();
            [v.x, v.y, v.z]
        }
        (None, _) => return None,
    };
    let e = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    };
    Some(format!("#{:02x}{:02x}{:02x}", e(rgb[0]), e(rgb[1]), e(rgb[2])))
}

/// What a choice does to the glTF scene: materials assigned to nodes,
/// nodes shown, a camera and light to move to.
fn effect(document: &SceneDocument, exported: &crate::gltf::Exported, drives: Drives, target: usize) -> Value {
    let mut out = Object::new();
    let assign = |out: &mut Object, multi_material: u64, variant: usize| -> Vec<Value> {
        let _ = out;
        let Some(multi) = document.multi_materials.iter().find(|m| m.id == multi_material) else { return Vec::new() };
        let Some(key) = multi.variants.get(variant) else { return Vec::new() };
        multi
            .parts
            .iter()
            .filter_map(|part| Some(Value::from(vec![Value::from(*exported.nodes.get(part)?), Value::from(*exported.materials.get(&(*part, *key))?)])))
            .collect()
    };
    let shown = |set: Option<usize>| -> Vec<Value> {
        let mut sets = document.model_sets.clone();
        sets.active = set;
        exported
            .nodes
            .iter()
            .filter(|(node, _)| sets.allows(&document.tree, **node))
            .map(|(_, &index)| Value::from(index))
            .collect()
    };
    match drives {
        Drives::Material { multi_material } => {
            let list = assign(&mut out, multi_material, target);
            out.set("assign", list);
        }
        Drives::ModelSet => {
            out.set("show", shown(Some(target)));
        }
        Drives::Studio => {
            let Some(studio) = document.studios.get(target) else { return out.build() };
            if let Some(camera) = &studio.camera {
                out.set("camera", camera_json(&camera.camera, camera.target));
            }
            if let Some(environment) = &studio.environment {
                let mut e = Object::new();
                e.set("rotation", environment.rotation - document.environment.rotation)
                    .set("intensity", environment.brightness / document.environment.brightness.max(1e-6));
                out.set("environment", e.build());
            }
            if let Some(set) = studio.model_set {
                out.set("show", shown(set));
            }
            let mut list = Vec::new();
            for (multi, variant) in &studio.variants {
                list.extend(assign(&mut out, *multi, *variant));
            }
            if !list.is_empty() {
                out.set("assign", list);
            }
            if let Some(style) = &studio.image_style {
                out.set("exposure", style.exposure);
            }
        }
    }
    out.build()
}

fn camera_json(camera: &tequila_render_core::camera::Camera, target: Vec3) -> Value {
    use tequila_render_core::camera::Projection;
    let focal = match camera.projection {
        Projection::Perspective { focal_length_mm } | Projection::Shift { focal_length_mm, .. } => focal_length_mm,
        _ => 50.0,
    };
    let p = camera.position();
    let mut c = Object::new();
    c.set("position", vec![p.x, p.y, p.z]).set("target", vec![target.x, target.y, target.z]).set("focal", focal).set("sensor", camera.sensor_width_mm);
    c.build()
}

/// The direction a texel of the equirectangular map looks along, and back.
/// `viewer.js` uses the same two.
fn direction(u: f32, v: f32) -> Vec3 {
    let phi = (u - 0.5) * std::f32::consts::TAU;
    let theta = v * std::f32::consts::PI;
    vec3(theta.sin() * phi.sin(), theta.cos(), -theta.sin() * phi.cos())
}

fn texel_of(d: Vec3) -> (f32, f32) {
    let u = 0.5 + d.x.atan2(-d.z) / std::f32::consts::TAU;
    let v = d.y.clamp(-1.0, 1.0).acos() / std::f32::consts::PI;
    (u, v)
}

/// A float RGB image, bilinearly sampled with the longitude wrapping.
struct Map {
    width: usize,
    height: usize,
    rgb: Vec<[f32; 3]>,
}

impl Map {
    fn sample(&self, u: f32, v: f32) -> [f32; 3] {
        let x = u * self.width as f32 - 0.5;
        let y = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let wrap = |x: f32| (x as i64).rem_euclid(self.width as i64) as usize;
        let (xa, xb) = (wrap(x0), wrap(x0 + 1.0));
        let (ya, yb) = (y0 as usize, (y0 as usize + 1).min(self.height - 1));
        let at = |x: usize, y: usize| self.rgb[y * self.width + x];
        let mut out = [0.0; 3];
        for k in 0..3 {
            let top = at(xa, ya)[k] * (1.0 - fx) + at(xb, ya)[k] * fx;
            let bottom = at(xa, yb)[k] * (1.0 - fx) + at(xb, yb)[k] * fx;
            out[k] = top * (1.0 - fy) + bottom * fy;
        }
        out
    }

    fn half(&self) -> Map {
        let (w, h) = ((self.width / 2).max(1), (self.height / 2).max(1));
        let mut rgb = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut s = [0.0; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = self.rgb[(2 * y + dy).min(self.height - 1) * self.width + (2 * x + dx).min(self.width - 1)];
                    for k in 0..3 {
                        s[k] += p[k] * 0.25;
                    }
                }
                rgb.push(s);
            }
        }
        Map { width: w, height: h, rgb }
    }
}

/// The environment as the viewer lights with it: a mip chain prefiltered
/// for GGX reflections, one roughness a level, RGBM-encoded PNGs; spherical
/// harmonics for diffuse light; and the background.
fn environment_json(document: &SceneDocument, environment: &Environment) -> Value {
    use rayon::prelude::*;
    let (w0, h0) = (ENV_WIDTH, ENV_WIDTH / 2);
    // The sharpest level, 4 samples a texel so a sun does not alias.
    let rgb: Vec<[f32; 3]> = (0..w0 * h0)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w0) as f32, (i / w0) as f32);
            let mut s = [0.0; 3];
            for (dx, dy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                let r = environment.radiance(direction((x + dx) / w0 as f32, (y + dy) / h0 as f32)).to_linear_srgb();
                s[0] += r.x.max(0.0) * 0.25;
                s[1] += r.y.max(0.0) * 0.25;
                s[2] += r.z.max(0.0) * 0.25;
            }
            s
        })
        .collect();
    let base = Map { width: w0, height: h0, rgb };
    let mut pyramid = vec![base];
    while pyramid.last().unwrap().width > 4 {
        let next = pyramid.last().unwrap().half();
        pyramid.push(next);
    }

    // Spherical harmonics, order 2, of the radiance: each texel weighted by
    // its solid angle.
    let mut sh = [[0.0f32; 3]; 9];
    {
        let m = &pyramid[2];
        for y in 0..m.height {
            let theta = (y as f32 + 0.5) / m.height as f32 * std::f32::consts::PI;
            let solid = theta.sin() * (std::f32::consts::PI / m.height as f32) * (std::f32::consts::TAU / m.width as f32);
            for x in 0..m.width {
                let d = direction((x as f32 + 0.5) / m.width as f32, (y as f32 + 0.5) / m.height as f32);
                let basis = [
                    0.282_095,
                    0.488_603 * d.y,
                    0.488_603 * d.z,
                    0.488_603 * d.x,
                    1.092_548 * d.x * d.y,
                    1.092_548 * d.y * d.z,
                    0.315_392 * (3.0 * d.z * d.z - 1.0),
                    1.092_548 * d.x * d.z,
                    0.546_274 * (d.x * d.x - d.y * d.y),
                ];
                let c = m.rgb[y * m.width + x];
                for (i, b) in basis.iter().enumerate() {
                    for k in 0..3 {
                        sh[i][k] += c[k] * b * solid;
                    }
                }
            }
        }
    }

    // GGX-prefiltered levels: importance-sampled about the normal, each
    // sample read from the pyramid level whose texels match its solid angle.
    let mut levels = Vec::with_capacity(ENV_LEVELS);
    for level in 0..ENV_LEVELS {
        let (w, h) = ((w0 >> level).max(1), (h0 >> level).max(1));
        let roughness = level as f32 / (ENV_LEVELS - 1) as f32;
        let pixels: Vec<[f32; 3]> = if level == 0 {
            pyramid[0].rgb.clone()
        } else {
            let a = (roughness * roughness).max(1e-3);
            let samples = 64;
            let texel_solid = 4.0 * std::f32::consts::PI / (w0 * h0) as f32;
            (0..w * h)
                .into_par_iter()
                .map(|i| {
                    let n = direction(((i % w) as f32 + 0.5) / w as f32, ((i / w) as f32 + 0.5) / h as f32);
                    let (t, b) = {
                        let helper = if n.y.abs() < 0.99 { Vec3::Y } else { Vec3::X };
                        let t = helper.cross(n).normalize();
                        (t, n.cross(t))
                    };
                    let mut sum = [0.0f32; 3];
                    let mut weight = 0.0f32;
                    for s in 0..samples {
                        // Hammersley.
                        let e1 = (s as f32 + 0.5) / samples as f32;
                        let e2 = (s as u32).reverse_bits() as f32 / 4_294_967_296.0;
                        let phi = std::f32::consts::TAU * e2;
                        let cos_theta = ((1.0 - e1) / (1.0 + (a * a - 1.0) * e1)).sqrt();
                        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
                        let hv = t * (sin_theta * phi.cos()) + b * (sin_theta * phi.sin()) + n * cos_theta;
                        let l = hv * (2.0 * n.dot(hv)) - n;
                        let nol = n.dot(l);
                        if nol <= 0.0 {
                            continue;
                        }
                        let noh = cos_theta;
                        let d = a * a / (std::f32::consts::PI * ((noh * noh) * (a * a - 1.0) + 1.0).powi(2));
                        let pdf = d / 4.0;
                        let sample_solid = 1.0 / (samples as f32 * pdf + 1e-6);
                        let lod = (0.5 * (sample_solid / texel_solid).log2() + 1.0).clamp(0.0, (pyramid.len() - 1) as f32);
                        let (u, v) = texel_of(l);
                        let c = pyramid[lod.round() as usize].sample(u, v);
                        for k in 0..3 {
                            sum[k] += c[k] * nol;
                        }
                        weight += nol;
                    }
                    let w = weight.max(1e-6);
                    [sum[0] / w, sum[1] / w, sum[2] / w]
                })
                .collect()
        };
        levels.push(Value::from(format!("data:image/png;base64,{}", base64(&png(w, h, &rgbm(&pixels))))));
    }

    // What the camera sees behind the product, which is not always what
    // lights it: the built-in studio's softboxes light the part and stay
    // out of its backdrop.
    let backdrop: Vec<[f32; 3]> = (0..w0 * h0)
        .into_par_iter()
        .map(|i| {
            let d = direction(((i % w0) as f32 + 0.5) / w0 as f32, ((i / w0) as f32 + 0.5) / h0 as f32);
            let r = environment.background_radiance_from(Vec3::ZERO, d).to_linear_srgb();
            [r.x.max(0.0), r.y.max(0.0), r.z.max(0.0)]
        })
        .collect();
    let mut out = Object::new();
    out.set("backdrop", format!("data:image/png;base64,{}", base64(&png(w0, h0, &rgbm(&backdrop)))));
    out.set("levels", levels)
        .set("rgbm", RGBM_RANGE)
        .set("sh", sh.iter().flat_map(|c| c.iter().copied()).collect::<Vec<f32>>());
    let background = match document.environment.background {
        BackgroundMode::Color => {
            let c = document.environment.background_color.to_linear_srgb();
            let mut b = Object::new();
            b.set("mode", "color").set("color", vec![c.x, c.y, c.z]);
            b.build()
        }
        _ => {
            let mut b = Object::new();
            b.set("mode", "environment");
            b.build()
        }
    };
    out.set("background", background);
    out.build()
}

/// Linear radiance as RGBM: colour over a shared multiplier in alpha, so
/// eight bits a channel carry up to [`RGBM_RANGE`] times white.
fn rgbm(pixels: &[[f32; 3]]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(pixels.len() * 4);
    for p in pixels {
        let peak = (p[0].max(p[1]).max(p[2]) / RGBM_RANGE).clamp(1e-6, 1.0);
        let m = (peak * 255.0).ceil() / 255.0;
        for k in 0..3 {
            bytes.push(((p[k] / (m * RGBM_RANGE)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
        bytes.push((m * 255.0 + 0.5) as u8);
    }
    bytes
}

/// A soft shadow under the parts, as an alpha image on the ground: dark
/// where something stands close above, fading with height and distance.
fn shadow_json(document: &SceneDocument, exported: &crate::gltf::Exported) -> Option<Value> {
    let (lo, hi) = exported.bounds?;
    let extent = (hi - lo).max(Vec3::splat(1e-4));
    let span = extent.x.max(extent.z);
    let margin = span * 0.45;
    let (x0, z0) = (lo.x - margin, lo.z - margin);
    let (w_world, d_world) = (extent.x + 2.0 * margin, extent.z + 2.0 * margin);
    let size = 256usize;
    let (w, h) = if w_world >= d_world {
        (size, ((size as f32 * d_world / w_world).round() as usize).max(8))
    } else {
        (((size as f32 * w_world / d_world).round() as usize).max(8), size)
    };
    let ground = lo.y;
    // The lowest geometry over each cell, above the ground.
    let mut height = vec![f32::INFINITY; w * h];
    for (&node, _) in &exported.nodes {
        let Some(n) = document.tree.get(node) else { continue };
        let NodeKind::Part { mesh } = n.kind else { continue };
        let Some(geometry) = document.geometry.get(mesh) else { continue };
        let world = document.tree.world_transform(node);
        let points: Vec<Vec3> = geometry.positions.iter().map(|p| world.point(*p)).collect();
        for t in geometry.indices.chunks_exact(3) {
            let Some(tri) = t.iter().map(|&i| points.get(i as usize).copied()).collect::<Option<Vec<Vec3>>>() else { continue };
            let to_cell = |p: Vec3| ((p.x - x0) / w_world * w as f32, (p.z - z0) / d_world * h as f32);
            let c: Vec<(f32, f32)> = tri.iter().map(|p| to_cell(*p)).collect();
            let (min_x, max_x) = (c.iter().map(|p| p.0).fold(f32::INFINITY, f32::min), c.iter().map(|p| p.0).fold(f32::NEG_INFINITY, f32::max));
            let (min_y, max_y) = (c.iter().map(|p| p.1).fold(f32::INFINITY, f32::min), c.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max));
            let area = (c[1].0 - c[0].0) * (c[2].1 - c[0].1) - (c[2].0 - c[0].0) * (c[1].1 - c[0].1);
            for y in (min_y.floor().max(0.0) as usize)..=(max_y.ceil().min(h as f32 - 1.0).max(0.0) as usize) {
                for x in (min_x.floor().max(0.0) as usize)..=(max_x.ceil().min(w as f32 - 1.0).max(0.0) as usize) {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let e = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0) * (py - a.1) - (px - a.0) * (b.1 - a.1);
                    let (w0, w1, w2) = (e(c[1], c[2]), e(c[2], c[0]), e(c[0], c[1]));
                    let inside = if area >= 0.0 { w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 } else { w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0 };
                    if inside || area.abs() < 1e-9 {
                        let y_here = if area.abs() < 1e-9 {
                            tri.iter().map(|p| p.y).fold(f32::INFINITY, f32::min)
                        } else {
                            (tri[0].y * w0 + tri[1].y * w1 + tri[2].y * w2) / area
                        };
                        let cell = &mut height[y * w + x];
                        *cell = cell.min((y_here - ground).max(0.0));
                    }
                }
            }
        }
    }
    let cell = w_world / w as f32;
    let covered: Vec<f32> = height.iter().map(|h| if h.is_finite() { 1.0 } else { 0.0 }).collect();
    let contact: Vec<f32> = height.iter().map(|h| if h.is_finite() { (-h / (span * 0.04)).exp() } else { 0.0 }).collect();
    let broad = blur(&covered, w, h, (span * 0.08 / cell).max(1.0));
    let near = blur(&contact, w, h, (span * 0.015 / cell).max(0.75));
    let mut rgba = Vec::with_capacity(w * h * 4);
    for i in 0..w * h {
        let a = (0.42 * broad[i] + 0.55 * near[i]).clamp(0.0, 0.85);
        rgba.extend_from_slice(&[0, 0, 0, (a * 255.0 + 0.5) as u8]);
    }
    let mut out = Object::new();
    out.set("image", format!("data:image/png;base64,{}", base64(&png(w, h, &rgba))))
        .set("min", vec![x0, z0])
        .set("size", vec![w_world, d_world])
        .set("y", ground);
    Some(out.build())
}

/// A separable Gaussian blur of a single-channel image.
fn blur(values: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let radius = (sigma * 3.0).ceil() as i64;
    let kernel: Vec<f32> = (-radius..=radius).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let total: f32 = kernel.iter().sum();
    let pass = |src: &[f32], horizontal: bool| -> Vec<f32> {
        let mut out = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0;
                for (k, weight) in kernel.iter().enumerate() {
                    let o = k as i64 - radius;
                    let (sx, sy) = if horizontal { (x as i64 + o, y as i64) } else { (x as i64, y as i64 + o) };
                    if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                        s += src[sy as usize * w + sx as usize] * weight;
                    }
                }
                out[y * w + x] = s / total;
            }
        }
        out
    };
    let once = pass(values, true);
    pass(&once, false)
}

fn png(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("writing to memory");
        writer.write_image_data(rgba).expect("writing to memory");
    }
    out
}

/// Standard base64, as `atob` reads it.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_the_standard_one() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// The equirectangular mapping is its own inverse, as the viewer's
    /// copy of it relies on.
    #[test]
    fn a_direction_finds_its_texel_again() {
        for (u, v) in [(0.1, 0.3), (0.5, 0.5), (0.9, 0.8), (0.25, 0.1)] {
            let (u2, v2) = texel_of(direction(u, v));
            assert!((u2 - u).abs() < 1e-4 && (v2 - v).abs() < 1e-4, "{u},{v} -> {u2},{v2}");
        }
    }

    /// The spherical harmonics of a uniform white environment: only the
    /// constant term, π-weighted irradiance of one.
    #[test]
    fn a_uniform_environment_lights_evenly() {
        let document = SceneDocument::new();
        let environment = Environment::uniform(tequila_core::spectrum::Spectrum::ONE);
        let json = environment_json(&document, &environment);
        let sh = json.get("sh").unwrap().as_f32_array().unwrap();
        // L00 of a constant 1 is 4π * 0.282095.
        assert!((sh[0] - 4.0 * std::f32::consts::PI * 0.282_095).abs() < 0.05, "{}", sh[0]);
        assert!(sh[3..].iter().all(|c| c.abs() < 0.05), "{sh:?}");
        assert_eq!(json.get("levels").unwrap().as_array().len(), ENV_LEVELS);
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use tequila_scene::configurator::{Choice, ConfigOption, Configurator, Style};

    /// The page looks for each combination's photo under the name the
    /// queue gives it: the same `{configuration}` expansion, sanitized the
    /// same way, with the format's extension.
    #[test]
    fn the_page_names_each_photo_as_the_queue_writes_it() {
        let mut document = SceneDocument::new();
        document.configurator = Configurator {
            title: "Kettle".into(),
            options: vec![ConfigOption {
                id: 1,
                name: "Colour".into(),
                drives: Drives::ModelSet,
                choices: vec![
                    Choice { label: "Red/Black".into(), target: 0, swatch: Some([0.5, 0.0, 0.0]) },
                    Choice { label: "Cream".into(), target: 1, swatch: None },
                ],
                style: Style::Buttons,
            }],
            rules: Vec::new(),
            current: vec![0],
            ..Default::default()
        };
        let folder = std::env::temp_dir().join(format!("tequila-web-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let environment = Environment::uniform(tequila_core::spectrum::Spectrum::ONE);
        let options = WebOptions { live: false, prerendered: Some(("jpg".into(), "images".into())), bake_size: 64 };
        let report = write(&folder, &document, &environment, &options).expect("written");
        let data = std::fs::read_to_string(folder.join("data.js")).unwrap();
        let page = std::fs::read_to_string(&report.index).unwrap();
        let _ = std::fs::remove_dir_all(&folder);
        assert!(page.contains("<title>Kettle</title>") && page.contains("data.js") && page.contains("viewer.js"));
        let json = data.trim_start_matches("window.TEQUILA = ").trim_end().trim_end_matches(';');
        let value = crate::json::parse(json.as_bytes()).expect("data.js holds JSON");
        assert!(value.get("glb").is_none(), "no live view was asked for");
        let images = value.get("images").expect("photos");
        for (key, choice) in [("0", "Red/Black"), ("1", "Cream")] {
            let fields = crate::output::NameFields { configuration: choice.into(), ..Default::default() };
            let queued = crate::output::output_path(std::path::Path::new("images"), "{configuration}", &fields, crate::output::ImageFormat::Jpeg(90));
            let expected = queued.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
            assert_eq!(images.get(key).and_then(|v| v.as_str()), Some(expected.as_str()));
        }
        assert_eq!(images.get("0").and_then(|v| v.as_str()), Some("images/Red-Black.jpg"), "a slash is not a folder");
    }
}
