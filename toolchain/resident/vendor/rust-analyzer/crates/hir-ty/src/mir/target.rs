//! What the target is, where MIR and the evaluator depend on it.

use std::sync::atomic::{AtomicUsize, Ordering};

static WORD: AtomicUsize = AtomicUsize::new(0);

/// Says how many bytes an address of the target takes, before anything is lowered: the code
/// generator does when the target is selected. Without it the host's is taken.
pub fn select_word(word: usize) {
    WORD.store(word, Ordering::Relaxed);
}

/// How many bytes an address of the target takes.
pub fn word() -> usize {
    match WORD.load(Ordering::Relaxed) {
        0 => size_of::<usize>(),
        word => word,
    }
}
