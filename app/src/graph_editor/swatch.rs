//! The material, rendered large beside the graph as it is edited.
//!
//! Each node shows what it makes; this shows what they make together, with
//! the real path tracer on the library's preview studio. That is the
//! answer to the question every edit is asking - V-Ray's material editor
//! keeps one beside its nodes for the same reason. It comes as it is made:
//! a quick small one so the change is seen at once, then the full-size
//! picture after two samples a pixel, eight, twenty-four and all sixty-four,
//! for as long as nothing has changed in the meantime.
//!
//! It was the small one and then the whole clean one, which on an M3 Max
//! took three and a half seconds for plain plastic and fifteen for felt:
//! the small one, blown up to a blur, was what a person saw for all of that,
//! and since every edit starts it over, for nearly all of the time they
//! spent on a material.

use tequila_materials::graph::MaterialGraph;
use tequila_render_core::shade::TextureTable;
use tequila_render_cpu::{render_thumbnail_in_steps, render_thumbnail_textured, Device, PreviewSettings, PreviewShape, Thumbnail, PREVIEW_RADIUS};
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
    stage: Stage,
    image: egui::ColorImage,
}

/// How far along a picture is: 0 the quick small one, then the full-size
/// one after each of [`STEPS`].
type Stage = u8;

/// The stage that is the finished picture.
const LAST: Stage = STEPS.len() as Stage;

/// Whether a picture that has arrived is shown over the one that is: one
/// further back never replaces one further on of the same thing.
fn replaces(shown: Option<(u64, PreviewShape, Stage)>, hash: u64, shape: PreviewShape, stage: Stage) -> bool {
    !matches!(shown, Some((h, s, at)) if h == hash && s == shape && at > stage)
}

type Inbox = Arc<(Mutex<Option<Job>>, Condvar)>;

pub struct Swatch {
    pub shape: PreviewShape,
    inbox: Option<Inbox>,
    done: Option<mpsc::Receiver<Done>>,
    texture: Option<egui::TextureHandle>,
    /// What is shown: the content and shape it was rendered for, and how
    /// far along it is.
    shown: Option<(u64, PreviewShape, Stage)>,
    asked: Option<(u64, PreviewShape)>,
}

impl Default for Swatch {
    fn default() -> Self {
        Swatch { shape: PreviewShape::Sphere, inbox: None, done: None, texture: None, shown: None, asked: None }
    }
}

/// The quick small picture, and the full-size one.
const ROUGH: PreviewSettings = PreviewSettings { size: 112, samples: 6, shape: PreviewShape::Sphere, radius: PREVIEW_RADIUS, uv_unit: None };
const CLEAN: PreviewSettings = PreviewSettings { size: 256, samples: 64, shape: PreviewShape::Sphere, radius: PREVIEW_RADIUS, uv_unit: None };

/// The samples a pixel the full-size picture is shown after, each added to
/// the ones before: 2, 8, 24 and [`CLEAN`]'s 64.
const STEPS: [u32; 4] = [2, 6, 16, 40];

/// The most samples a pixel rendered before looking for a newer material:
/// a step is not stopped part-way, and the last one whole was ten seconds
/// of felt during which a change to the material showed nothing.
const CHUNK: u32 = 4;

/// [`STEPS`] in pieces of at most [`CHUNK`]: each piece's samples, and
/// whether a step ends with it and the picture is shown.
fn pieces() -> Vec<(u32, bool)> {
    let mut out = Vec::new();
    for step in STEPS {
        let mut left = step;
        while left > 0 {
            let piece = left.min(CHUNK);
            left -= piece;
            out.push((piece, left == 0));
        }
    }
    out
}

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
            let newer = || worker_inbox.0.lock().map(|s| s.is_some()).unwrap_or(false);
            // `TEQUILA_SWATCH_TRACE=1` prints when each picture is handed
            // over, in milliseconds since its material was taken up.
            let trace = std::env::var_os("TEQUILA_SWATCH_TRACE").is_some();
            // False when the editor that asked is gone.
            let hand_over = |job: &Job, since: std::time::Instant, stage: Stage, thumbnail: &Thumbnail| -> bool {
                let rgba = crate::thumbnail::develop_rgba(thumbnail);
                let image = egui::ColorImage::from_rgba_unmultiplied([thumbnail.width, thumbnail.height], &rgba);
                if trace {
                    eprintln!("swatch: stage {stage} of {LAST}, {} px, {:.0} ms", thumbnail.width, since.elapsed().as_secs_f64() * 1000.0);
                }
                let sent = results.send(Done { hash: job.hash, shape: job.shape, stage, image }).is_ok();
                repaint.request_repaint();
                sent
            };
            let mut job = take(true);
            while let Some(current) = job {
                let since = std::time::Instant::now();
                let rough = PreviewSettings { shape: current.shape, radius: current.radius, ..ROUGH };
                if let Ok(thumbnail) = pool.install(|| render_thumbnail_textured(&device, &current.graph, current.textures.clone(), environment.clone(), rough)) {
                    if !hand_over(&current, since, 0, &thumbnail) {
                        return;
                    }
                }
                // Something newer: start on that rather than polishing this.
                if !newer() {
                    let full = PreviewSettings { shape: current.shape, radius: current.radius, ..clean_pass };
                    let pieces = pieces();
                    let samples: Vec<u32> = pieces.iter().map(|(samples, _)| *samples).collect();
                    let mut piece = 0;
                    let mut stage: Stage = 0;
                    let mut gone = false;
                    let _ = pool.install(|| {
                        render_thumbnail_in_steps(&device, &current.graph, current.textures.clone(), environment.clone(), full, &samples, |thumbnail, _| {
                            let shown = pieces[piece].1;
                            piece += 1;
                            if shown {
                                stage += 1;
                                gone = !hand_over(&current, since, stage, &thumbnail);
                            }
                            !gone && !newer()
                        })
                    });
                    if gone {
                        return;
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
                if !replaces(self.shown, result.hash, result.shape, result.stage) {
                    continue;
                }
                match &mut self.texture {
                    Some(texture) => texture.set(result.image, egui::TextureOptions::LINEAR),
                    None => self.texture = Some(ctx.load_texture("material-swatch", result.image, egui::TextureOptions::LINEAR)),
                }
                self.shown = Some((result.hash, result.shape, result.stage));
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

    /// Whether what is shown is the material as it is now, finished.
    pub fn settled(&self) -> bool {
        matches!((self.shown, self.asked), (Some((h, s, LAST)), Some((ah, as_))) if h == ah && s == as_)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pictures of one material arrive in order, but the editor may have
    /// moved on and back: a picture further back never covers one further
    /// on of the same material and shape, and anything covers another's.
    #[test]
    fn a_rougher_picture_never_covers_a_finer_one_of_the_same_thing() {
        let (sphere, slab) = (PreviewShape::Sphere, PreviewShape::Slab);
        assert!(replaces(None, 7, sphere, 0));
        for stage in 1..=LAST {
            assert!(replaces(Some((7, sphere, stage - 1)), 7, sphere, stage), "stage {stage} did not replace the one before");
            assert!(!replaces(Some((7, sphere, stage)), 7, sphere, stage - 1), "stage {} covered stage {stage}", stage - 1);
        }
        assert!(replaces(Some((7, sphere, LAST)), 8, sphere, 0), "another material's first picture was held back");
        assert!(replaces(Some((7, sphere, LAST)), 7, slab, 0), "another shape's first picture was held back");
        // The steps end on the clean picture's samples.
        assert_eq!(STEPS.iter().sum::<u32>(), CLEAN.samples);
    }

    /// A change to the material is looked for every few samples, not once a
    /// step: the pieces are small, add up to the steps, and show a picture
    /// exactly where each step ends.
    #[test]
    fn the_steps_are_rendered_in_small_pieces() {
        let pieces = pieces();
        assert!(pieces.iter().all(|(samples, _)| (1..=CHUNK).contains(samples)), "{pieces:?}");
        assert_eq!(pieces.iter().map(|(samples, _)| samples).sum::<u32>(), CLEAN.samples);
        assert_eq!(pieces.iter().filter(|(_, shown)| *shown).count(), STEPS.len());
        let mut behind = 0;
        let shown_at: Vec<u32> = pieces
            .iter()
            .filter_map(|(samples, shown)| {
                behind += samples;
                shown.then_some(behind)
            })
            .collect();
        assert_eq!(shown_at, [2, 8, 24, 64]);
    }
}
