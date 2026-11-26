//! How far an import has got, and a way to stop it: what the import
//! dialog's progress bar reads and its Cancel button sets.
//!
//! An import runs on a thread of its own, and the readers it calls know
//! nothing about windows. So the watch is put on that thread for as long as
//! the import lasts ([`Watch::during`]), and the parts of the import that
//! can say how far they are, or can stop part way, find it there
//! ([`current`]): the STEP and IGES readers count faces divided, as they did
//! for the status line, OpenCASCADE's helper says how far through the file
//! it is and is stopped at once on Cancel, Tequila's own STEP reader stops
//! between bodies, and the pipeline counts the meshes it has prepared. A
//! reader that can say neither - an OBJ, a glTF - is shown as working,
//! without a share, and a Cancel during it is taken at the next step.
//!
//! A cancelled import ends in [`ImportError::Cancelled`](crate::ImportError)
//! and changes nothing: the document is only touched once the import is
//! done, by its command.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::Arc;

/// Faces divided out of faces to divide, and how far through reading the
/// file in percent: what [`crate::step::progress`] sums for the status line,
/// and a watch's own counts.
#[derive(Default)]
pub(crate) struct Counts {
    pub done: AtomicUsize,
    pub total: AtomicUsize,
    pub read: AtomicUsize,
}

impl Counts {
    pub fn clear(&self) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.read.store(0, Ordering::Relaxed);
    }
}

/// What an import is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Reading the file: opening it, and for a CAD file turning its
    /// entities into shapes.
    Reading,
    /// Dividing a CAD file's exact surfaces into triangles.
    Dividing,
    /// Welding, giving normals, finding repeated shapes: the pipeline
    /// (`docs/03` §6.1).
    Preparing,
    /// Putting it in the scene, on the window's thread.
    Building,
}

impl Stage {
    fn from_code(code: u8) -> Stage {
        match code {
            1 => Stage::Dividing,
            2 => Stage::Preparing,
            3 => Stage::Building,
            _ => Stage::Reading,
        }
    }

    fn code(self) -> u8 {
        match self {
            Stage::Reading => 0,
            Stage::Dividing => 1,
            Stage::Preparing => 2,
            Stage::Building => 3,
        }
    }
}

/// How far an import has got, as [`Watch::seen`] last saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Seen {
    pub stage: Stage,
    /// Of `total`: faces divided, or meshes prepared. Both zero where the
    /// stage does not count.
    pub done: usize,
    pub total: usize,
    /// How far through reading the file, where the reader says.
    pub percent: Option<usize>,
}

impl Seen {
    /// The share done, 0 to 1, where it is known: a bar fills to it, and
    /// shows only that something is happening where it is not.
    pub fn fraction(&self) -> Option<f32> {
        match (self.total, self.percent) {
            (total, _) if total > 0 => Some((self.done as f32 / total as f32).clamp(0.0, 1.0)),
            (_, Some(percent)) => Some((percent as f32 / 100.0).clamp(0.0, 1.0)),
            _ => None,
        }
    }
}

#[derive(Default)]
struct Shared {
    counts: Arc<Counts>,
    stage: AtomicU8,
    cancelled: AtomicBool,
}

/// One import's progress and its Cancel, shared between the thread doing it
/// and the window showing it.
#[derive(Clone, Default)]
pub struct Watch(Arc<Shared>);

impl Watch {
    pub fn new() -> Watch {
        Watch::default()
    }

    /// Ask the import to stop. It stops at the next point it looks, and
    /// ends in [`ImportError::Cancelled`](crate::ImportError::Cancelled).
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Relaxed)
    }

    /// Move on to `stage`, its counts starting again.
    pub fn set_stage(&self, stage: Stage) {
        self.0.counts.clear();
        self.0.stage.store(stage.code(), Ordering::Relaxed);
    }

    /// `done` of `total` in the stage under way.
    pub fn count(&self, done: usize, total: usize) {
        self.0.counts.total.store(total, Ordering::Relaxed);
        self.0.counts.done.store(done, Ordering::Relaxed);
    }

    /// How far it has got. A read whose faces are being counted is
    /// dividing them, whichever reader set the count.
    pub fn seen(&self) -> Seen {
        let counts = &self.0.counts;
        let (done, total, read) = (counts.done.load(Ordering::Relaxed), counts.total.load(Ordering::Relaxed), counts.read.load(Ordering::Relaxed));
        let mut stage = Stage::from_code(self.0.stage.load(Ordering::Relaxed));
        if stage == Stage::Reading && total > 0 {
            stage = Stage::Dividing;
        }
        Seen { stage, done: done.min(total), total, percent: (stage == Stage::Reading && read > 0).then_some(read) }
    }

    /// Run `f` with this watch on the thread, for the readers to find.
    pub fn during<R>(&self, f: impl FnOnce() -> R) -> R {
        struct Restore(Option<Watch>);
        impl Drop for Restore {
            fn drop(&mut self) {
                let before = self.0.take();
                CURRENT.with(|current| *current.borrow_mut() = before);
            }
        }
        let _restore = Restore(CURRENT.with(|current| current.replace(Some(self.clone()))));
        f()
    }

    pub(crate) fn counts(&self) -> Arc<Counts> {
        self.0.counts.clone()
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Watch>> = const { RefCell::new(None) };
}

/// The watch the import on this thread is under, if it is under one.
pub fn current() -> Option<Watch> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Whether the import on this thread was cancelled.
pub(crate) fn stopped() -> bool {
    current().is_some_and(|watch| watch.is_cancelled())
}

/// Move the import on this thread, if it is watched, on to `stage`.
pub(crate) fn stage(stage: Stage) {
    if let Some(watch) = current() {
        watch.set_stage(stage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_watch_is_found_only_on_its_own_thread_and_only_while_it_lasts() {
        let watch = Watch::new();
        assert!(current().is_none());
        watch.during(|| {
            assert!(current().is_some());
            std::thread::spawn(|| assert!(current().is_none(), "another thread saw it")).join().unwrap();
            let inner = Watch::new();
            inner.during(|| inner.cancel());
            assert!(!stopped(), "the inner watch's cancel reached the outer one");
        });
        assert!(current().is_none(), "it outlasted its import");
    }

    #[test]
    fn the_stage_and_its_share_read_as_the_bar_shows_them() {
        let watch = Watch::new();
        assert_eq!(watch.seen().stage, Stage::Reading);
        assert_eq!(watch.seen().fraction(), None, "an OBJ being read says nothing of how far");
        watch.counts().read.store(40, Ordering::Relaxed);
        assert_eq!(watch.seen().fraction(), Some(0.4));
        // Faces counted: dividing them.
        watch.count(25, 100);
        assert_eq!(watch.seen().stage, Stage::Dividing);
        assert_eq!(watch.seen().fraction(), Some(0.25));
        watch.set_stage(Stage::Preparing);
        assert_eq!((watch.seen().stage, watch.seen().fraction()), (Stage::Preparing, None), "the counts start again");
        watch.count(3, 4);
        assert_eq!(watch.seen().fraction(), Some(0.75));
    }
}
