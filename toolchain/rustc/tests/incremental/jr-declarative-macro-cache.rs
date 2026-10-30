//@ revisions: rpass1 rpass2 rpass3
//@ compile-flags: -Zjr-cache-declarative-macros=yes
//@ edition: 2021

// Literal-only rule selection/transcription is eligible. The output must still
// be parsed and type checked on every invocation, including disk-cache hits.
macro_rules! literal {
    (first) => { 11u32 };
    (second) => { 22u32 };
}

#[cfg(any(rpass1, rpass2))]
macro_rules! definition_changes {
    () => { 33u32 };
}
#[cfg(rpass3)]
macro_rules! definition_changes {
    () => { 44u32 };
}

// These exercise the conservative original-path boundary: no fragment parser
// or repetition matching is skipped by the new query.
macro_rules! fragment {
    ($e:expr) => { $e + 1 };
}
macro_rules! repeated {
    ($($e:expr),*) => { 0 $(+ $e)* };
}

fn main() {
    assert_eq!(literal!(first), 11);
    assert_eq!(literal!(second), 22);
    assert_eq!(fragment!(literal!(first)), 12);
    assert_eq!(repeated!(1, 2, 3), 6);
    #[cfg(any(rpass1, rpass2))]
    assert_eq!(definition_changes!(), 33);
    #[cfg(rpass3)]
    assert_eq!(definition_changes!(), 44);
}
