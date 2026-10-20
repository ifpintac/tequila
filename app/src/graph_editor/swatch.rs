//! The material, rendered large beside the graph as it is edited.
//!
//! Each node shows what it makes; this shows what they make together, with
//! the real path tracer on the library's preview studio. That is the
//! answer to the question every edit is asking - V-Ray's material editor
//! keeps one beside its nodes for the same reason. It comes in two passes:
//! a quick rough one so the change is seen at once, then a clean one if
//! nothing has changed in the meantime.

use tequila_materials::graph::MaterialGraph;
use tequila_render_core::shade::TextureTable;
use tequila_render_cpu::{render_thumbnail_textured, Device, PreviewSettings, PreviewShape, PREVIEW_RADIUS};
use std::sync::{mpsc, Arc, Condvar, Mutex};

struct Job {
    hash: u64,
    graph: MaterialGraph,
    textures: Arc<TextureTable>,
    shape: PreviewShape,
    radius: f32,
}

struct Done {
    hash: u64,
    shape: PreviewShape,
    clean: bool,
    image: egui::ColorImage,
}

type Inbox = Arc<(Mutex<Option<Job>>, Condvar)>;

pub struct Swatch {
    pub shape: PreviewShape,
    inbox: Option<Inbox>,
    done: Option<mpsc::Receiver<Done>>,
    texture: Option<egui::TextureHandle>,
    /// What is shown: the content and shape it was rendered for, and whether
    /// it is the clean pass.
    shown: Option<(u64, PreviewShape, bool)>,
    asked: Option<(u64, PreviewShape)>,
}

impl Default for Swatch {
    fn default() -> Self {
        Swatch { shape: PreviewShape::Sphere, inbox: None, done: None, texture: None, shown: None, asked: None }
    }
}

/// The quick pass and the clean one.
const ROUGH: PreviewSettings = PreviewSettings { size: 112, samples: 6, shape: PreviewShape::Sphere, radius: PREVIEW_RADIUS };
const CLEAN: PreviewSettings = PreviewSettings { size: 256, samples: 64, shape: PreviewShape::Sphere, radius: PREVIEW_RADIUS };

impl Swatch {
    fn start(&mut self, ctx: &egui::Context) {
        let inbox: Inbox = Arc::new((Mutex::new(None), Condvar::new()));
        let (results, done) = mpsc::channel::<Done>();
        let worker_inbox = inbox.clone();
        let repaint = ctx.clone();
        // The clean pass at the screen's own density: the swatch is shown up
        // to 300 points square.
        let clean_pass = PreviewSettings { size: CLEAN.size * crate::thumbnail::density(ctx), ..CLEAN };
        let spawned = std::thread::Builder::new().name("tequila-material-swatch".into()).spawn(move || {
            let Ok(device) = Device::shared() else { return };
            let environment = Arc::new(tequila_scene::demo::preview_environment());
            // Half the machine: quick enough to keep up with a slider, and
            // the viewport keeps the rest.
            let threads = std::thread::available_parallelism().map(|n| (n.get() / 2).max(1)).unwrap_or(2);
            let Ok(pool) = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|_| "tequila-swatch-cpu".into())
                .build()
            else {
                return;
            };
            let take = |wait: bool| -> Option<Job> {
                let (lock, signal) = &*worker_inbox;
                let mut slot = lock.lock().ok()?;
                while wait && slot.is_none() {
                    slot = signal.wait(slot).ok()?;
                }
                slot.take()
            };
            let mut job = take(true);
            while let Some(current) = job {
                for (pass, clean) in [(ROUGH, false), (clean_pass, true)] {
                    let settings = PreviewSettings { shape: current.shape, radius: current.radius, ..pass };
                    let rendered = pool.install(|| render_thumbnail_textured(&device, &current.graph, current.textures.clone(), environment.clone(), settings));
                    if let Ok(thumbnail) = rendered {
                        let rgba = crate::thumbnail::develop_rgba(&thumbnail);
                        let image = egui::ColorImage::from_rgba_unmultiplied([thumbnail.width, thumbnail.height], &rgba);
                        if results.send(Done { hash: current.hash, shape: current.shape, clean, image }).is_err() {
                            return;
                        }
                        repaint.request_repaint();
                    }
                    // Something newer: start on that rather than polishing this.
                    if worker_inbox.0.lock().map(|s| s.is_some()).unwrap_or(false) {
                        break;
                    }
                }
                job = take(true);
            }
        });
        if spawned.is_ok() {
            self.inbox = Some(inbox);
            self.done = Some(done);
        }
    }

    /// Ask for the material as it is now, on a ball `radius` metres round;
    /// take what has been rendered.
    pub fn update(&mut self, ctx: &egui::Context, graph: &MaterialGraph, textures: &Arc<TextureTable>, radius: f32) {
        if let Some(done) = &self.done {
            while let Ok(result) = done.try_recv() {
                // A rough pass never replaces a clean one of the same thing.
                if self.shown.is_some_and(|(h, s, clean)| h == result.hash && s == result.shape && clean && !result.clean) {
                    continue;
                }
                match &mut self.texture {
                    Some(texture) => texture.set(result.image, egui::TextureOptions::LINEAR),
                    None => self.texture = Some(ctx.load_texture("material-swatch", result.image, egui::TextureOptions::LINEAR)),
                }
                self.shown = Some((result.hash, result.shape, result.clean));
            }
        }
        let hash = graph.content_hash() ^ (radius.to_bits() as u64).rotate_left(29);
        if self.asked == Some((hash, self.shape)) {
            return;
        }
        if self.inbox.is_none() {
            self.start(ctx);
        }
        if let Some(inbox) = &self.inbox {
            if let Ok(mut slot) = inbox.0.lock() {
                *slot = Some(Job { hash, graph: graph.clone(), textures: textures.clone(), shape: self.shape, radius });
                inbox.1.notify_one();
                self.asked = Some((hash, self.shape));
            }
        }
    }

    pub fn texture(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    /// Whether what is shown is the material as it is now, cleanly.
    pub fn settled(&self) -> bool {
        matches!((self.shown, self.asked), (Some((h, s, true)), Some((ah, as_))) if h == ah && s == as_)
    }
}
