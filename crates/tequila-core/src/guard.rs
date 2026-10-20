//! Panics a caller expects and recovers from, told apart from the ones
//! nobody does.
//!
//! Some third-party code panics on input it cannot handle - the STEP
//! mesher on a face it cannot triangulate - and Tequila catches that and
//! goes on without the face. The application's crash reporter reports every
//! other panic. It asks [`expecting`] first, so a model with one bad face
//! does not leave a crash report behind.

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};

thread_local! {
    static EXPECTING: Cell<u32> = const { Cell::new(0) };
}

/// `f`, with a panic in it caught and returned as `Err`, and not reported
/// as a crash.
pub fn catching<T>(f: impl FnOnce() -> T) -> std::thread::Result<T> {
    EXPECTING.with(|e| e.set(e.get() + 1));
    let result = catch_unwind(AssertUnwindSafe(f));
    EXPECTING.with(|e| e.set(e.get() - 1));
    result
}

/// Whether a panic on this thread now would be caught by [`catching`].
pub fn expecting() -> bool {
    EXPECTING.with(|e| e.get() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_caught_panic_is_expected_only_inside() {
        assert!(!expecting());
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let inside = seen.clone();
        let result = catching(move || {
            inside.store(expecting(), std::sync::atomic::Ordering::SeqCst);
            if inside.load(std::sync::atomic::Ordering::SeqCst) {
                std::panic::resume_unwind(Box::new("a bad face"));
            }
        });
        assert!(result.is_err());
        assert!(seen.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!expecting());
    }
}
