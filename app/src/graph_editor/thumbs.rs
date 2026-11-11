//! The small pictures beside the browser's rows.
//!
//! A material's is a ball of it, as the library's shelf shows one; a map's
//! is a square of what it makes, as its node shows it. Somebody choosing
//! between Granite and Marble, or between two scene materials named by an
//! importer, chooses by eye, and a list of names made them read.
//!
//! Made for the rows in view only, on one worker thread, and kept by what
//! they are a picture of - a material's graph, a node as it is set - so a
//! material edited is pictured again and one scrolled past twice is not.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{mpsc, Arc};
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, TextureId};
use tequila_render_core::shade::TextureTable;
use tequila_render_cpu::{render_thumbnail_textured, Device, PreviewSettings, PreviewShape, PREVIEW_RADIUS};
use tequila_scene::textures::TextureSource;

/// A row's picture, on screen, in points.
pub const SIDE: f32 = 18.0;

/// What to make a picture of.
pub enum Make {
    /// A material, on the library's ball: with the pictures it reads, as
    /// files to load (a library material's) or already loaded (the
    /// document's).
    Ball { graph: MaterialGraph, files: Vec<(TextureId, TextureSource)>, loaded: Option<Arc<TextureTable>>, uv_unit: Option<f32> },
    /// A texture or pattern node as it is first set, over a square of
    /// surface of the size that shows it best ([`EXTENTS`]).
    Map { kind: NodeKind, textures: Arc<TextureTable> },
}

/// The sizes of surface a map is tried over, in metres across. A map's
/// picture is of what the map is, the same in every project, and no one
/// size shows them all: a metre of a weave is grey and a centimetre of a
/// marble is one colour. (Its node on the canvas shows it at the model's
/// size.)
const EXTENTS: [f32; 6] = [2.0, 0.5, 0.12, 0.03, 0.008, 0.002];

/// How well a picture shows what it is of: its contrast, less how much of
/// that is from one pixel to the next. Features a few pixels across score
/// well; speckle scores below nothing and one flat colour scores nothing.
fn clarity(image: &egui::ColorImage) -> f32 {
    let [w, h] = image.size;
    if w < 2 || h < 2 {
        return 0.0;
    }
    let light = |x: usize, y: usize| {
        let p = image.pixels[y * w + x];
        (p.r() as f32 + p.g() as f32 + p.b() as f32) / 3.0
    };
    let count = (w * h) as f32;
    let mean = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| light(x, y)).sum::<f32>() / count;
    let spread = ((0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| (light(x, y) - mean).powi(2)).sum::<f32>() / count).sqrt();
    let fine = (0..h - 1).flat_map(|y| (0..w - 1).map(move |x| (x, y))).map(|(x, y)| (light(x, y) - light(x + 1, y)).abs() + (light(x, y) - light(x, y + 1)).abs()).sum::<f32>()
        / (2.0 * ((w - 1) * (h - 1)) as f32);
    spread - 1.5 * fine
}

/// A key for anything that prints itself: a node as it is set.
pub fn key_of(what: &impl std::fmt::Debug, salt: u64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    salt.hash(&mut hasher);
    format!("{what:?}").hash(&mut hasher);
    hasher.finish()
}

#[derive(Default)]
pub struct Thumbs {
    made: HashMap<u64, egui::TextureHandle>,
    /// Asked of the worker, or found to have no picture: not asked again.
    asked: HashSet<u64>,
    /// The rows in view this frame with no picture yet.
    wanted: Vec<u64>,
    jobs: Option<mpsc::Sender<(u64, Make)>>,
    done: Option<mpsc::Receiver<(u64, Option<egui::ColorImage>)>>,
}

impl Thumbs {
    /// The picture kept under `key`, if it is made; noted as wanted if not.
    pub fn get(&mut self, key: u64) -> Option<egui::TextureId> {
        match self.made.get(&key) {
            Some(texture) => Some(texture.id()),
            None => {
                if !self.asked.contains(&key) && !self.wanted.contains(&key) {
                    self.wanted.push(key);
                }
                None
            }
        }
    }

    /// The rows that asked this frame and have nothing coming.
    pub fn take_wanted(&mut self) -> Vec<u64> {
        std::mem::take(&mut self.wanted)
    }

    /// A picture that was to hand: the library's own, read from its files.
    pub fn put(&mut self, ctx: &egui::Context, key: u64, image: egui::ColorImage) {
        self.asked.insert(key);
        self.made.insert(key, ctx.load_texture(format!("browser_{key:x}"), image, egui::TextureOptions::LINEAR));
    }

    /// Nothing can be pictured under `key`: stop asking.
    pub fn none(&mut self, key: u64) {
        self.asked.insert(key);
    }

    /// Have the worker make `key`'s picture.
    pub fn make(&mut self, ctx: &egui::Context, key: u64, what: Make) {
        if !self.asked.insert(key) {
            return;
        }
        if self.jobs.is_none() {
            self.start(ctx);
        }
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send((key, what));
        }
    }

    /// Take what the worker has made since the last frame.
    pub fn collect(&mut self, ctx: &egui::Context) {
        // A long session over a large library: start over rather than keep
        // every picture there ever was.
        if self.made.len() > 2000 {
            self.made.clear();
            self.asked.clear();
        }
        let Some(done) = &self.done else { return };
        for (key, image) in done.try_iter() {
            if let Some(image) = image {
                self.made.insert(key, ctx.load_texture(format!("browser_{key:x}"), image, egui::TextureOptions::LINEAR));
            }
        }
    }

    fn start(&mut self, ctx: &egui::Context) {
        let (jobs, work) = mpsc::channel::<(u64, Make)>();
        let (results, done) = mpsc::channel();
        let repaint = ctx.clone();
        // Twice the points on a Retina screen, and no more: these are small.
        let pixels = (SIDE as usize + 2) * crate::thumbnail::density(ctx).clamp(1, 2);
        let spawned = std::thread::Builder::new().name("tequila-browser-thumbs".into()).spawn(move || {
            let Ok(device) = Device::shared() else { return };
            let environment = Arc::new(tequila_scene::demo::preview_environment());
            // One thread: the viewport and the swatch have the rest.
            let Ok(pool) = rayon::ThreadPoolBuilder::new().num_threads(1).thread_name(|_| "tequila-browser-thumbs-cpu".into()).build() else {
                return;
            };
            while let Ok((key, what)) = work.recv() {
                let image = pool.install(|| picture(&device, &environment, what, pixels));
                if results.send((key, image)).is_err() {
                    return;
                }
                repaint.request_repaint();
            }
        });
        if spawned.is_ok() {
            self.jobs = Some(jobs);
            self.done = Some(done);
        }
    }
}

/// Make one picture, `pixels` across.
fn picture(device: &Arc<Device>, environment: &Arc<tequila_render_core::environment::Environment>, what: Make, pixels: usize) -> Option<egui::ColorImage> {
    match what {
        Make::Ball { graph, files, loaded, uv_unit } => {
            let textures = loaded.unwrap_or_else(|| crate::library_tab::picture_table(&files));
            // Twice the size asked of the renderer, drawn down: a ball this
            // small is mostly edge.
            let settings = PreviewSettings { size: pixels * 2, samples: 24, shape: PreviewShape::Sphere, radius: PREVIEW_RADIUS, uv_unit };
            let thumbnail = render_thumbnail_textured(device, &graph, textures, environment.clone(), settings).ok()?;
            Some(crate::thumbnail::fit(crate::thumbnail::develop(&thumbnail), pixels))
        }
        Make::Map { kind, textures } => {
            let mut graph = MaterialGraph::from_material(Default::default());
            let output = super::previews::preview_output(&kind);
            let id = graph.add_node(Node::new(kind));
            // The widest of the sizes that is clear: at least half as clear
            // as the clearest, which is always the closest, one brick or
            // one cell filling the square. The first, when none shows
            // anything.
            let mut tried = Vec::new();
            for extent in EXTENTS {
                let image = super::previews::evaluate(&textures, extent, &graph, id, output)?;
                tried.push((clarity(&image), image));
            }
            let clearest = tried.iter().map(|(score, _)| *score).fold(0.0f32, f32::max);
            let at = tried.iter().position(|(score, _)| clearest > 0.0 && *score >= clearest * 0.5).unwrap_or(0);
            let image = tried.swap_remove(at).1;
            // The middle square of the node's wider picture.
            let [w, h] = image.size;
            let left = (w.saturating_sub(h)) / 2;
            let pixels: Vec<egui::Color32> = (0..h).flat_map(|y| (0..h.min(w)).map(move |x| (x, y))).map(|(x, y)| image.pixels[y * w + left + x]).collect();
            Some(egui::ColorImage { size: [h.min(w), h], pixels })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Arc<Device> {
        Device::shared().expect("an Embree device")
    }

    /// A material's picture is a small ball of it, and a map's a square of
    /// what it makes: neither blank, and two that differ pictured apart.
    #[test]
    fn a_rows_picture_is_of_what_the_row_is() {
        let environment = Arc::new(tequila_scene::demo::preview_environment());
        let ball = |kind: tequila_materials::material_type::MaterialType| {
            let graph = MaterialGraph::from_material(kind.defaults());
            picture(&device(), &environment, Make::Ball { graph, files: Vec::new(), loaded: None, uv_unit: None }, 36).expect("a ball")
        };
        let (metal, plastic) = (ball(tequila_materials::material_type::MaterialType::Metal), ball(tequila_materials::material_type::MaterialType::Plastic));
        assert_eq!(metal.size, [36, 36]);
        assert_ne!(metal.pixels, plastic.pixels, "metal and plastic are pictured alike");
        let spread = |image: &egui::ColorImage| {
            let lights: Vec<u32> = image.pixels.iter().map(|p| p.r() as u32 + p.g() as u32 + p.b() as u32).collect();
            lights.iter().max().unwrap() - lights.iter().min().unwrap()
        };
        assert!(spread(&metal) > 60, "the ball is not a flat square");

        let map = |name: &str| {
            let node = tequila_materials::nodes::ShaderNode::from_key(name).expect("a node");
            picture(&device(), &environment, Make::Map { kind: NodeKind::Node(node), textures: Arc::new(TextureTable::new()) }, 36).expect("a square")
        };
        let (checker, noise) = (map("checker_texture"), map("noise_texture"));
        assert_eq!(checker.size[0], checker.size[1], "a square of it");
        assert!(spread(&checker) > 60, "a checker has two colours");
        assert_ne!(checker.pixels, noise.pixels);

        // Of the sizes tried, one that shows it: a checker's squares some
        // pixels across, not a grey of them.
        assert!(clarity(&checker) > 0.0, "the checker's picture is speckle or flat: {}", clarity(&checker));

        // Speckle is worse than nothing, and large shapes are best.
        let made = |f: &dyn Fn(usize, usize) -> u8| egui::ColorImage { size: [32, 32], pixels: (0..32 * 32).map(|i| egui::Color32::from_gray(f(i % 32, i / 32))).collect() };
        let flat = made(&|_, _| 128);
        let speckle = made(&|x, y| if (x * 7 + y * 13) % 2 == 0 { 0 } else { 255 });
        let blocks = made(&|x, y| if (x / 8 + y / 8) % 2 == 0 { 0 } else { 255 });
        assert!(clarity(&blocks) > clarity(&flat) && clarity(&flat) > clarity(&speckle), "{} {} {}", clarity(&blocks), clarity(&flat), clarity(&speckle));

        // A node that makes no colour has no picture, and is not asked twice.
        let value = picture(&device(), &environment, Make::Map { kind: NodeKind::Reroute { ty: tequila_materials::graph::SocketType::Shader }, textures: Arc::new(TextureTable::new()) }, 36);
        assert!(value.is_none());
    }

    /// A key is what the row is a picture of: the same node the same key,
    /// a setting changed another, and the groups kept apart.
    #[test]
    fn a_picture_is_kept_by_what_it_is_of() {
        let bump = |depth: f32| NodeKind::Bump { depth };
        assert_eq!(key_of(&bump(0.5), 1), key_of(&bump(0.5), 1));
        assert_ne!(key_of(&bump(0.5), 1), key_of(&bump(0.9), 1));
        assert_ne!(key_of(&bump(0.5), 1), key_of(&bump(0.5), 2));

        let mut thumbs = Thumbs::default();
        assert_eq!(thumbs.get(7), None);
        assert_eq!(thumbs.get(7), None);
        assert_eq!(thumbs.take_wanted(), [7], "asked once, however many rows wait on it");
        thumbs.none(7);
        assert_eq!(thumbs.get(7), None);
        assert!(thumbs.take_wanted().is_empty(), "one with no picture is not asked for again");
    }
}
