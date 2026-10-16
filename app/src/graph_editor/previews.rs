//! Each node's picture of what it makes, drawn off the interface's thread.
//!
//! A preview runs the real compiler and shader over a small square of
//! surface, so it is what the renderer would compute and cannot drift from
//! it. That is a few thousand shading points per node; done on the
//! interface's thread for every node on every edit, a graph of twenty
//! textures made every slider drag stutter. So they are made on a worker,
//! the last picture is shown until the next arrives, and a node is redone
//! only when something upstream of it changed - not when anything in the
//! graph did.

use super::view::PREVIEW_PIXELS;
use tequila_materials::graph::{MaterialGraph, NodeId, NodeKind, Socket, SocketType};
use tequila_materials::material_type::Param;
use tequila_render_core::shade::TextureTable;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::mpsc;
use std::sync::Arc;

/// What a node's output depends on: the node itself and everything wired
/// into it, recursively, with the outputs each wire leaves from.
pub fn upstream_hashes(graph: &MaterialGraph) -> HashMap<NodeId, u64> {
    fn visit(graph: &MaterialGraph, id: NodeId, memo: &mut HashMap<NodeId, u64>, visiting: &mut HashSet<NodeId>) -> u64 {
        if let Some(h) = memo.get(&id) {
            return *h;
        }
        // A loop being drawn this frame, refused when it is applied.
        if !visiting.insert(id) {
            return 0;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if let Some(node) = graph.node(id) {
            format!("{:?}", node.kind).hash(&mut hasher);
            format!("{:?}", node.literals).hash(&mut hasher);
            node.muted.hash(&mut hasher);
        }
        let mut incoming: Vec<_> = graph.links().iter().filter(|l| l.to == id).copied().collect();
        incoming.sort();
        for link in incoming {
            link.to_socket.hash(&mut hasher);
            link.from_socket.hash(&mut hasher);
            visit(graph, link.from, memo, visiting).hash(&mut hasher);
        }
        visiting.remove(&id);
        let h = hasher.finish();
        memo.insert(id, h);
        h
    }
    let mut memo = HashMap::new();
    let mut visiting = HashSet::new();
    let ids: Vec<NodeId> = graph.nodes().map(|(id, _)| id).collect();
    for id in ids {
        visit(graph, id, &mut memo, &mut visiting);
    }
    memo
}

/// Run the real compiler and shader for one node's output over a square of
/// surface `extent` scene units across, as sRGB pixels. `None` when the
/// output cannot be shown as a colour or the graph does not compile.
pub fn evaluate(textures: &TextureTable, extent: f32, graph: &MaterialGraph, id: NodeId, output: u8) -> Option<egui::ColorImage> {
    let output_type = graph.node(id)?.kind.output_type_at(output)?;
    if !output_type.converts_to(SocketType::Color) {
        return None;
    }
    let mut preview = graph.clone();
    let surface = preview.output();
    preview.disconnect(surface, Socket::Param(Param::BaseColor));
    if id != surface {
        preview.connect_output(id, output, surface, Socket::Param(Param::BaseColor));
    }
    let program = preview.compile().ok()?;
    // The square is tilted to cut all three axes. Square to one axis, it
    // would miss whatever a pattern does along that axis: a flat pattern
    // (x–z) seen in x–y is one line of itself, and marble banded along the
    // axis it faces is one flat colour.
    use tequila_core::math::vec3;
    let across = vec3(1.0, 0.0, -1.0) * std::f32::consts::FRAC_1_SQRT_2;
    let down = vec3(1.0, 2.0, 1.0) * (1.0 / 6.0f32.sqrt());
    let normal = vec3(1.0, -1.0, 1.0) * (1.0 / 3.0f32.sqrt());
    let mut registers = tequila_render_core::shade::Registers::new();
    let mut scratch = tequila_render_core::material::Material::default();
    let [w, h] = PREVIEW_PIXELS;
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            // Square pixels: the height spans the extent, and the width as
            // much again as the box is wide. Texture coordinates likewise
            // keep 0 to 1 across the middle square and repeat either side.
            let (sx, sy) = ((x as f32 + 0.5) / h as f32, (y as f32 + 0.5) / h as f32);
            let half_width = w as f32 / h as f32 * 0.5;
            let position = across * ((sx - half_width) * extent) + down * ((0.5 - sy) * extent);
            let point = tequila_texture::projection::SurfacePoint {
                position,
                normal,
                uv: tequila_core::math::Vec2 { x: sx - half_width + 0.5, y: sy },
                footprint: extent / h as f32,
                scale: 1.0,
                uv_footprint: None,
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: tequila_core::spectrum::Spectrum::ONE,
                // Seen face on, at the middle of a part of this size.
                world: tequila_texture::projection::PointWorld {
                    position,
                    normal,
                    true_normal: normal,
                    incoming: normal,
                    backfacing: false,
                    generated: position * (1.0 / extent) + tequila_core::math::Vec3::splat(0.5),
                    object_location: tequila_core::math::Vec3::ZERO,
                    object_random: 0.5,
                },
            };
            let material = tequila_render_core::shade::shade(&program, textures, &point, &mut registers, &mut scratch);
            let [r, g, b] = tequila_color::DisplayTransform::Srgb.apply_u8(material.base_color);
            pixels.push(egui::Color32::from_rgb(r, g, b));
        }
    }
    Some(egui::ColorImage { size: [w, h], pixels })
}

struct Job {
    key: (NodeId, u8),
    hash: u64,
    graph: Arc<MaterialGraph>,
    textures: Arc<TextureTable>,
    extent: f32,
}

struct Done {
    key: (NodeId, u8),
    hash: u64,
    image: Option<egui::ColorImage>,
}

/// The previews on show, and the worker making the next ones.
#[derive(Default)]
pub struct Previews {
    jobs: Option<mpsc::Sender<Job>>,
    done: Option<mpsc::Receiver<Done>>,
    shown: HashMap<(NodeId, u8), (u64, Option<egui::TextureHandle>)>,
    asked: HashMap<(NodeId, u8), u64>,
}

impl Previews {
    /// Forget everything: another material is being edited.
    pub fn clear(&mut self) {
        self.shown.clear();
        self.asked.clear();
    }

    fn start(&mut self, ctx: &egui::Context) {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (results, done) = mpsc::channel::<Done>();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new().name("tequila-node-previews".into()).spawn(move || {
            while let Ok(first) = inbox.recv() {
                // Only the newest request for each node is worth making.
                let mut latest: HashMap<(NodeId, u8), Job> = HashMap::new();
                latest.insert(first.key, first);
                while let Ok(job) = inbox.try_recv() {
                    latest.insert(job.key, job);
                }
                for job in latest.into_values() {
                    let image = evaluate(&job.textures, job.extent, &job.graph, job.key.0, job.key.1);
                    if results.send(Done { key: job.key, hash: job.hash, image }).is_err() {
                        return;
                    }
                    repaint.request_repaint();
                }
            }
        });
        if spawned.is_ok() {
            self.jobs = Some(jobs);
            self.done = Some(done);
        }
    }

    /// Take what the worker has finished.
    pub fn collect(&mut self, ctx: &egui::Context) {
        let Some(done) = &self.done else { return };
        while let Ok(result) = done.try_recv() {
            let texture = result.image.map(|image| {
                ctx.load_texture(format!("node-preview-{}-{}", result.key.0 .0, result.key.1), image, egui::TextureOptions::LINEAR)
            });
            self.shown.insert(result.key, (result.hash, texture));
        }
    }

    /// The picture for `key`, asking for a new one if what is shown is out
    /// of date. Shows the old one meanwhile.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        key: (NodeId, u8),
        hash: u64,
        graph: &mut Option<Arc<MaterialGraph>>,
        source: &MaterialGraph,
        textures: &Arc<TextureTable>,
        extent: f32,
    ) -> Option<egui::TextureHandle> {
        let current = self.shown.get(&key).is_some_and(|(h, _)| *h == hash);
        if !current && self.asked.get(&key) != Some(&hash) {
            if self.jobs.is_none() {
                self.start(ctx);
            }
            if let Some(jobs) = &self.jobs {
                // One copy of the graph for every preview asked for this frame.
                let shared = graph.get_or_insert_with(|| Arc::new(source.clone())).clone();
                if jobs.send(Job { key, hash, graph: shared, textures: textures.clone(), extent }).is_ok() {
                    self.asked.insert(key, hash);
                }
            }
        }
        self.shown.get(&key).and_then(|(_, t)| t.clone())
    }

    /// Whether a preview is still being made, for a "working" mark.
    pub fn pending(&self, key: (NodeId, u8), hash: u64) -> bool {
        self.shown.get(&key).map(|(h, _)| *h) != Some(hash)
    }
}

/// The graph's own node kinds whose output is a picture worth previewing,
/// and the output to show: a texture's colour where it has one.
pub fn preview_output(kind: &NodeKind) -> u8 {
    match kind {
        NodeKind::Node(node) => node.outputs().iter().position(|o| o.ty == SocketType::Color).unwrap_or(0) as u8,
        _ => 0,
    }
}
