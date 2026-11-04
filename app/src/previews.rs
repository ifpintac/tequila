//! The library's previews, made in the background (`docs/04` §9).
//!
//! One worker, on one core of its own, renders the previews the shelf
//! asks for: the tiles on screen first, top first, then every other tile
//! with nothing kept, in the shelf's order. It keeps going whether or not
//! the window is drawing - behind another window, or minimised - which a
//! preview started from the UI's frame did not: the shelf rendered a
//! screenful while the window was in front, and nothing while it was not.
//! Each preview it makes is kept on disk at once, and handed back for the
//! shelf to show the next time it draws.

use eframe::egui;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use tequila_materials::graph::MaterialGraph;
use tequila_render_core::environment::Environment;
use tequila_render_cpu::{render_thumbnail_textured, Device, PreviewSettings};

/// One preview to make: a material's graph and pictures, under its key in
/// the disk cache, rendered with `settings`.
pub struct Job {
    pub key: u64,
    pub disk_key: u64,
    pub graph: MaterialGraph,
    pub pictures: Vec<(tequila_materials::graph::TextureId, tequila_scene::textures::TextureSource)>,
    pub settings: PreviewSettings,
    /// The thumbnail the library ships for it, when it has one.
    pub shipped: Option<std::path::PathBuf>,
}

/// What is waiting: the tiles on screen, and the rest of the shelf behind
/// them. Both are taken from the end, so they are kept last-first.
#[derive(Default)]
struct Queue {
    first: Vec<Job>,
    rest: Vec<Job>,
    /// Taken by the worker and not yet back, by key.
    running: Vec<u64>,
    closed: bool,
}

/// A preview made: the material's key, how many pixels across, and the
/// picture.
pub type Made = (u64, usize, egui::ColorImage);

/// The worker, and what it is to do and has done.
pub struct Previews {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    made: mpsc::Receiver<Made>,
    sender: mpsc::Sender<Made>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Default for Previews {
    fn default() -> Self {
        let (sender, made) = mpsc::channel();
        Previews { queue: Arc::new((Mutex::new(Queue::default()), Condvar::new())), made, sender, worker: None }
    }
}

impl Previews {
    /// The tiles on screen, top first: what the worker does next, before
    /// anything else waiting.
    pub fn set_first(&self, jobs: Vec<Job>) {
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        let running = queue.running.clone();
        queue.first = jobs.into_iter().rev().filter(|job| !running.contains(&job.key)).collect();
        wake.notify_all();
    }

    /// The rest of the shelf, in its order: what the worker does when
    /// nothing on screen is waiting.
    pub fn set_rest(&self, jobs: Vec<Job>) {
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        let running = queue.running.clone();
        queue.rest = jobs.into_iter().rev().filter(|job| !running.contains(&job.key)).collect();
        wake.notify_all();
    }

    /// The previews made since the last look.
    pub fn made(&self) -> Vec<Made> {
        self.made.try_iter().collect()
    }

    /// How many are waiting: on screen, and behind them.
    #[cfg(test)]
    pub fn waiting(&self) -> (usize, usize) {
        let queue = self.queue.0.lock().unwrap_or_else(|e| e.into_inner());
        (queue.first.len(), queue.rest.len())
    }

    /// Whether nothing is waiting or being made.
    #[cfg(test)]
    pub fn idle(&self) -> bool {
        let queue = self.queue.0.lock().unwrap_or_else(|e| e.into_inner());
        queue.first.is_empty() && queue.rest.is_empty() && queue.running.is_empty()
    }

    /// Start the worker, once.
    pub fn start(&mut self, device: Arc<Device>, environment: Arc<Environment>, disk: Arc<crate::thumbnail::DiskCache>, ctx: egui::Context) {
        if self.worker.is_some() {
            return;
        }
        let queue = self.queue.clone();
        let sender = self.sender.clone();
        self.worker = std::thread::Builder::new()
            .name("tequila-thumbnail".into())
            .spawn(move || work(&queue, &sender, &device, &environment, &disk, &ctx))
            .ok();
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        wake.notify_all();
    }
}

/// Make previews until told to stop: a kept one read back, any other
/// rendered and kept, each handed back and the shelf woken to show it.
fn work(
    queue: &(Mutex<Queue>, Condvar),
    sender: &mpsc::Sender<Made>,
    device: &Arc<Device>,
    environment: &Arc<Environment>,
    disk: &crate::thumbnail::DiskCache,
    ctx: &egui::Context,
) {
    // One core of its own, at the ordinary priority: the viewport's pool is
    // the viewport's, and in Windows' background mode a preview starved
    // whenever anything kept the processor busy.
    let Some(pool) = rayon::ThreadPoolBuilder::new().num_threads(1).thread_name(|_| "tequila-preview-cpu".into()).build().ok() else {
        return;
    };
    let (lock, wake) = queue;
    loop {
        let job = {
            let mut waiting = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if waiting.closed {
                    return;
                }
                if let Some(job) = waiting.first.pop().or_else(|| waiting.rest.pop()) {
                    waiting.running.push(job.key);
                    break job;
                }
                waiting = wake.wait(waiting).unwrap_or_else(|e| e.into_inner());
            }
        };
        let pixels = job.settings.size;
        // Shipped with the library, scaled to the tile; else kept from an
        // earlier launch; else rendered, and kept.
        let shipped = job.shipped.as_deref().filter(|_| pixels <= crate::thumbnail::SHIPPED_PIXELS).and_then(crate::thumbnail::load_png);
        let image = shipped.map(|image| crate::thumbnail::fit(image, pixels)).or_else(|| disk.load(job.disk_key)).or_else(|| {
            let textures = crate::library_tab::picture_table(&job.pictures);
            let thumbnail = pool.install(|| render_thumbnail_textured(device, &job.graph, textures, environment.clone(), job.settings)).ok()?;
            let rgba = crate::thumbnail::develop_rgba(&thumbnail);
            disk.store(job.disk_key, thumbnail.width, thumbnail.height, &rgba);
            Some(egui::ColorImage::from_rgba_unmultiplied([thumbnail.width, thumbnail.height], &rgba))
        });
        if let Some(image) = image {
            let _ = sender.send((job.key, pixels, image));
            ctx.request_repaint();
        }
        lock.lock().unwrap_or_else(|e| e.into_inner()).running.retain(|&key| key != job.key);
    }
}
