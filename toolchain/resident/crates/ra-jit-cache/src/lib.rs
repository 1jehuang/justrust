//! `#[ra_jit::cache]`: a pure function's results kept across runs of the program, and dropped
//! when the code they came from changes.
//!
//! ```toml
//! [dependencies]
//! ra-jit-cache = "0.1"
//! serde = { version = "1", features = ["derive"] }
//! ```
//!
//! ```rust,ignore
//! #[ra_jit::cache]
//! fn simulate(params: &Params) -> Report { /* expensive and pure */ }
//! ```
//!
//! A call looks for what the same arguments gave before, in this run and in the runs before
//! it, and calls the function only when nothing is found. What it is looked up by is the
//! arguments, written with `serde`, and what ra-jit names the code by: the machine code of the
//! function and of everything it can reach, as ra-jit compiled it. A comment, a change of
//! formatting, lines moved or an edit to a function it does not reach leave that the same,
//! and the results stay; an edit to anything it reaches gives other code, and the next call
//! computes again. Live edits do the same while the program runs.
//!
//! ra-jit checks that the function is pure before it caches it: a function that reaches a
//! `static` that can change, a thread-local, a foreign function, inline assembly or a raw
//! pointer of its own crate is run every time, and the first call says why on stderr.
//!
//! Built without ra-jit, by `cargo build`, the function is called every time as written:
//! only ra-jit can say what the code is.
//!
//! The arguments have to implement `serde::Serialize` and the result `Serialize` and
//! `DeserializeOwned`. Results are kept under the target directory the program was built in,
//! `target/ra-jit/fn-cache`, up to 512 MiB, the least recently used going first, and for no
//! more than a function's last three codes. The environment of the program can change that
//! (each setting is also read by its old name, `RA_JIT_` for `BLAZE_`):
//!
//! - `BLAZE_FN_CACHE=off` calls every function every time; `memory` keeps results for the
//!   run alone.
//! - `BLAZE_FN_CACHE_DIR` is the directory to keep them in.
//! - `BLAZE_FN_CACHE_SIZE` is how much to keep there, in bytes, or with `K`, `M` or `G`.
//! - `BLAZE_FN_CACHE_LOG=1` says on stderr what became of each call.
//!
//! `cargo blaze cache` lists what each function kept, and `cargo blaze cache clear` empties
//! the directory of a project (`--stale`: of all but what each function's code computed).

pub use ra_jit_cache_macros::cache;

mod key;
mod store;

#[doc(hidden)]
pub mod __private {
    use serde::{Serialize, de::DeserializeOwned};

    pub use crate::store::{Held, Key, call, clear_stale, holdings, key};

    /// What ra-jit names the code of a cached function by, with the function's name, or
    /// without a name for the code why the function is not cached. All zeros when ra-jit did
    /// not compile the caller.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Code {
        pub(crate) hash: [u64; 2],
        text: *const u8,
        len: usize,
    }

    impl Code {
        const UNKNOWN: Code = Code { hash: [0, 0], text: std::ptr::null(), len: 0 };

        pub(crate) fn text(&self) -> &'static str {
            if self.len == 0 {
                return "";
            }
            // SAFETY: ra-jit writes the address and length of a `&'static str` here.
            unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.text, self.len))
            }
        }

        pub(crate) fn known(&self) -> bool {
            self.hash != [0, 0]
        }
    }

    /// The code of `body`. ra-jit compiles a call of this into the answer; rustc's copy has
    /// no answer to give.
    #[inline(never)]
    pub fn __ra_jit_cache_code<F>(body: &F) -> Code {
        let _ = body;
        Code::UNKNOWN
    }

    /// Names what writes `A` and reads and writes `R`, for the code of a function that takes
    /// `A` and returns `R` to cover them: a result read back by other code is another result.
    pub fn __ra_jit_codec<A: Serialize, R: Serialize + DeserializeOwned>() {
        let _ = crate::key::encode::<A> as fn(&A) -> Option<Vec<u8>>;
        let _ = encode::<R> as fn(&R) -> Option<Vec<u8>>;
        let _ = decode::<R> as fn(&[u8]) -> Option<R>;
    }

    pub(crate) fn encode<T: Serialize>(value: &T) -> Option<Vec<u8>> {
        postcard::to_allocvec(value).ok()
    }

    pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Option<T> {
        postcard::from_bytes(bytes).ok()
    }
}
