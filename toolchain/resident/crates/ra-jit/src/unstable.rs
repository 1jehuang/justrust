//! rustc's unstable flags, `-Z name[=value]`, and what ra-jit does with each: the one table
//! every place that reads rustc's flags asks (the daemon's plan, the plan of a program linked
//! against rustc's libraries, the build scripts and proc macros ra-jit compiles, ra-jit's own
//! answers to rustc's questions), so that they all agree. rustc still gets every flag cargo
//! gives it, for what it compiles; the table says what ra-jit does in its own code.
//!
//! - **Ignored**: the flag changes how fast rustc is, what it prints, the files it writes
//!   beside its output or what rustc's own backend does inside a function, and nothing a
//!   program does or how ra-jit's code links against rustc's.
//! - **Modelled**: the flag changes what a program does, and ra-jit's code already does what
//!   it asks.
//! - **Refused**: the flag changes the code, its ABI, what rustc's libraries export or what a
//!   program does, and ra-jit does not do the same; a crate built with it is not ra-jit's,
//!   with a reason that names the flag. A flag the table does not have is refused too.
//!
//! The names are those of `rustc -Z help` of the toolchain in `harness/toolchain`.

/// What ra-jit does with one `-Z` flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Treatment {
    Ignored(&'static str),
    Modelled(&'static str),
    Refused(&'static str),
}

/// A row of the table: how a flag is treated, by its value where that decides it.
#[derive(Clone, Copy)]
enum Row {
    Ignored(&'static str),
    /// Modelled when the flag turns the thing off, which is what ra-jit's code does; refused
    /// when it turns it on.
    OffOnly(&'static str),
    Refused(&'static str),
}

use Row::{Ignored, OffOnly, Refused};

const SPEED: &str = "how fast rustc compiles";
const PRINTS: &str = "what rustc prints or writes beside its output";
const CHECKS: &str = "what rustc checks of itself while it compiles";
const ACCEPTS: &str = "which programs rustc's type checker accepts, not the code of one it accepts";
const RUSTC_BACKEND: &str =
    "how rustc's backend optimizes or lays out its own code; the program does the same";
const DEBUG: &str = "what rustc's own objects tell a debugger; blaze writes its own debug info";
const CODE: &str = "it changes the code rustc generates, which blaze's code generator does not do";
const ABI: &str = "it changes the ABI or symbols rustc's code has, which blaze's code must match";
const EXPORTS: &str =
    "it changes which functions rustc's libraries export, which blaze links against";
const SEMANTICS: &str = "it changes what the program does, which blaze does not model";
const LINKS: &str = "it changes how rustc links, which blaze's link does not do";
const NOT_BUILT: &str = "rustc does not build the crate's code with it";
const INSTRUMENTS: &str = "it instruments the code, which blaze's code generator does not do";

/// Every flag of `rustc -Z help`, sorted by name.
const TABLE: &[(&str, Row)] = &[
    ("allow-features", Refused("blaze does not check which features a crate enables")),
    ("allow-partial-mitigations", Refused(ABI)),
    ("always-encode-mir", Refused(EXPORTS)),
    ("annotate-moves", Ignored(DEBUG)),
    ("assert-incr-state", Ignored(CHECKS)),
    ("assume-incomplete-release", Refused("it changes what `cfg(version)` says")),
    ("assumptions-on-binders", Ignored(ACCEPTS)),
    ("async-panic", Refused(SEMANTICS)),
    ("autodiff", Refused(CODE)),
    ("autodiff-post-passes", Refused(CODE)),
    ("binary-dep-depinfo", Ignored(PRINTS)),
    ("box-noalias", Ignored(RUSTC_BACKEND)),
    ("branch-protection", Refused(CODE)),
    ("build-sdylib-interface", Refused(NOT_BUILT)),
    ("cache-proc-macros", Ignored(SPEED)),
    ("cf-protection", Refused(CODE)),
    ("check-cfg-all-expected", Ignored(PRINTS)),
    ("checksum-hash-algorithm", Ignored(PRINTS)),
    (
        "codegen-backend",
        Refused("blaze links against what rustc's LLVM backend builds, not another backend's"),
    ),
    ("codegen-emit-retag", Refused(INSTRUMENTS)),
    ("codegen-source-order", Ignored(RUSTC_BACKEND)),
    ("contract-checks", OffOnly("blaze compiles `contract_checks()` as false")),
    ("coverage-options", Refused(INSTRUMENTS)),
    ("crate-attr", Refused("blaze's front end does not see the attribute it adds")),
    ("cross-crate-inline-threshold", Refused(EXPORTS)),
    ("debug-info-type-line-numbers", Ignored(DEBUG)),
    ("debuginfo-compression", Ignored(DEBUG)),
    ("debuginfo-for-profiling", Ignored(DEBUG)),
    ("deduplicate-diagnostics", Ignored(PRINTS)),
    ("default-visibility", Refused(ABI)),
    ("deny-partial-mitigations", Refused(ABI)),
    ("dep-info-omit-d-target", Ignored(PRINTS)),
    ("direct-access-external-data", Refused(ABI)),
    ("disable-fast-paths", Ignored(SPEED)),
    ("disable-incr-comp-backend-caching", Ignored(SPEED)),
    ("disable-param-env-normalization-hack", Ignored(ACCEPTS)),
    ("dual-proc-macros", Refused(NOT_BUILT)),
    ("dump-dep-graph", Ignored(PRINTS)),
    ("dump-mir", Ignored(PRINTS)),
    ("dump-mir-dataflow", Ignored(PRINTS)),
    ("dump-mir-dir", Ignored(PRINTS)),
    ("dump-mir-exclude-alloc-bytes", Ignored(PRINTS)),
    ("dump-mir-exclude-pass-number", Ignored(PRINTS)),
    ("dump-mir-graphviz", Ignored(PRINTS)),
    ("dump-mono-stats", Ignored(PRINTS)),
    ("dump-mono-stats-format", Ignored(PRINTS)),
    ("dwarf-version", Ignored(DEBUG)),
    ("dylib-lto", Refused(EXPORTS)),
    ("eagerly-emit-delayed-bugs", Ignored(CHECKS)),
    ("ehcont-guard", Refused(CODE)),
    // Cargo on a nightly toolchain passes `-Z embed-metadata=no` to every crate by itself: the
    // metadata goes to the `.rmeta` file and not into the rlib, whose objects are the same.
    ("embed-metadata", Ignored("where rustc puts a library's metadata; the objects are the same")),
    ("embed-source", Ignored(DEBUG)),
    ("emit-stack-sizes", Refused(CODE)),
    ("enforce-type-length-limit", Ignored(ACCEPTS)),
    ("experimental-default-bounds", Refused(SEMANTICS)),
    ("export-executable-symbols", Refused(LINKS)),
    ("external-clangrt", Refused(LINKS)),
    ("extra-const-ub-checks", Ignored(CHECKS)),
    ("fewer-names", Ignored(RUSTC_BACKEND)),
    ("fixed-x18", Refused(ABI)),
    ("flatten-format-args", Refused(CODE)),
    ("fmt-debug", Refused("it changes what `#[derive(Debug)]` prints")),
    ("force-intrinsic-fallback", Refused(CODE)),
    ("force-unstable-if-unmarked", Refused(SEMANTICS)),
    ("function-return", Refused(CODE)),
    ("function-sections", Ignored(RUSTC_BACKEND)),
    ("future-incompat-test", Ignored(PRINTS)),
    ("graphviz-dark-mode", Ignored(PRINTS)),
    ("graphviz-font", Ignored(PRINTS)),
    ("has-thread-local", Refused("it changes what `cfg(target_thread_local)` says")),
    ("help", Refused(NOT_BUILT)),
    ("higher-ranked-assumptions", Ignored(ACCEPTS)),
    ("hint-mostly-unused", Refused(EXPORTS)),
    ("hint-msrv", Ignored(PRINTS)),
    ("human-readable-cgu-names", Ignored(RUSTC_BACKEND)),
    ("identify-regions", Ignored(PRINTS)),
    ("ignore-directory-in-diagnostics-source-blocks", Ignored(PRINTS)),
    ("implicit-sysroot-deps", Refused(LINKS)),
    ("incremental-ignore-spans", Ignored(SPEED)),
    ("incremental-info", Ignored(PRINTS)),
    ("incremental-verify-ich", Ignored(CHECKS)),
    ("indirect-branch-cs-prefix", Refused(CODE)),
    ("inline-llvm", Ignored(RUSTC_BACKEND)),
    ("inline-mir", Refused(EXPORTS)),
    ("inline-mir-forwarder-threshold", Refused(EXPORTS)),
    ("inline-mir-hint-threshold", Refused(EXPORTS)),
    ("inline-mir-preserve-debug", Ignored(DEBUG)),
    ("inline-mir-threshold", Refused(EXPORTS)),
    ("input-stats", Ignored(PRINTS)),
    ("instrument-mcount", Refused(INSTRUMENTS)),
    ("instrument-xray", Refused(INSTRUMENTS)),
    ("internal-testing-features", Refused(SEMANTICS)),
    ("large-data-threshold", Refused(CODE)),
    ("layout-seed", Refused("it changes the layout of types")),
    ("link-directives", Refused(LINKS)),
    ("link-native-libraries", Refused(LINKS)),
    ("link-only", Refused(LINKS)),
    ("lint-llvm-ir", Ignored(CHECKS)),
    ("lint-mir", Ignored(CHECKS)),
    ("llvm-module-flag", Refused(CODE)),
    ("llvm-plugins", Refused(CODE)),
    ("llvm-target-feature", Refused(ABI)),
    ("llvm-time-trace", Ignored(PRINTS)),
    ("llvm-writable", Ignored(RUSTC_BACKEND)),
    ("location-detail", Refused("it changes what `Location::caller()` says")),
    ("ls", Refused(NOT_BUILT)),
    ("macro-backtrace", Ignored(PRINTS)),
    ("macro-stats", Ignored(PRINTS)),
    ("maximal-hir-to-mir-coverage", Refused(INSTRUMENTS)),
    ("merge-functions", Ignored(RUSTC_BACKEND)),
    ("meta-stats", Ignored(PRINTS)),
    ("metadata-crate-hash", Refused(ABI)),
    ("metrics-dir", Ignored(PRINTS)),
    ("min-function-alignment", Refused(CODE)),
    ("min-recursion-limit", Ignored(ACCEPTS)),
    ("mir-enable-passes", Refused(CODE)),
    ("mir-include-spans", Ignored(PRINTS)),
    ("mir-move-elimination", Refused(CODE)),
    ("mir-opt-bisect-limit", Refused(CODE)),
    ("mir-opt-level", Refused(EXPORTS)),
    ("mir-preserve-ub", Refused(CODE)),
    ("mir-strip-debuginfo", Ignored(DEBUG)),
    ("move-size-limit", Ignored(PRINTS)),
    ("namespaced-crates", Refused(SEMANTICS)),
    ("next-solver", Ignored(ACCEPTS)),
    ("nll-facts", Ignored(PRINTS)),
    ("nll-facts-dir", Ignored(PRINTS)),
    ("no-analysis", Refused(NOT_BUILT)),
    ("no-codegen", Refused(NOT_BUILT)),
    ("no-generate-arange-section", Ignored(DEBUG)),
    ("no-implied-bounds-compat", Ignored(ACCEPTS)),
    ("no-leak-check", Refused(SEMANTICS)),
    ("no-link", Refused(LINKS)),
    ("no-parallel-backend", Ignored(SPEED)),
    ("no-profiler-runtime", Refused(LINKS)),
    ("no-steal-thir", Ignored(SPEED)),
    ("no-trait-vptr", Refused("it changes the layout of vtables")),
    ("no-unique-section-names", Ignored(RUSTC_BACKEND)),
    ("normalize-docs", Ignored("what rustdoc documents")),
    ("offload", Refused(CODE)),
    ("on-broken-pipe", Refused(SEMANTICS)),
    ("osx-rpath-install-name", Refused(LINKS)),
    ("packed-bundled-libs", Refused(LINKS)),
    ("packed-stack", Refused(ABI)),
    (
        "panic-abort-tests",
        Refused("blaze's test harness does not run tests built with panic=abort"),
    ),
    ("panic-in-drop", Refused(SEMANTICS)),
    ("parse-crate-root-only", Refused(NOT_BUILT)),
    ("patchable-function-entry", Refused(CODE)),
    ("plt", Ignored(RUSTC_BACKEND)),
    ("pointer-authentication", Refused(ABI)),
    ("polonius", Ignored(ACCEPTS)),
    ("pre-link-arg", Refused(LINKS)),
    ("pre-link-args", Refused(LINKS)),
    ("precise-enum-drop-elaboration", Refused(CODE)),
    ("print-codegen-stats", Ignored(PRINTS)),
    ("print-codegen-stats-json", Ignored(PRINTS)),
    ("print-llvm-passes", Ignored(PRINTS)),
    ("print-mono-items", Ignored(PRINTS)),
    ("print-type-sizes", Ignored(PRINTS)),
    ("proc-macro-backtrace", Ignored(PRINTS)),
    ("proc-macro-execution-strategy", Refused("blaze runs proc macros its own way")),
    ("profile-closures", Ignored(PRINTS)),
    ("profiler-runtime", Refused(LINKS)),
    ("query-dep-graph", Ignored(CHECKS)),
    ("randomize-layout", Refused("it changes the layout of types")),
    ("reg-struct-return", Refused(ABI)),
    ("regparm", Refused(ABI)),
    ("relax-elf-relocations", Ignored(RUSTC_BACKEND)),
    ("remap-cwd-prefix", Refused("it changes the paths `file!()` and panics name")),
    ("remark-dir", Ignored(PRINTS)),
    ("renormalize-rigid-aliases", Ignored(ACCEPTS)),
    ("retpoline", Refused(CODE)),
    ("retpoline-external-thunk", Refused(CODE)),
    ("sanitizer", Refused(INSTRUMENTS)),
    ("sanitizer-cfi-canonical-jump-tables", Refused(INSTRUMENTS)),
    ("sanitizer-cfi-diag", Refused(INSTRUMENTS)),
    ("sanitizer-cfi-generalize-pointers", Refused(INSTRUMENTS)),
    ("sanitizer-cfi-minimal-runtime", Refused(INSTRUMENTS)),
    ("sanitizer-cfi-normalize-integers", Refused(INSTRUMENTS)),
    ("sanitizer-cfi-recover", Refused(INSTRUMENTS)),
    ("sanitizer-dataflow-abilist", Refused(INSTRUMENTS)),
    ("sanitizer-ignorelist", Refused(INSTRUMENTS)),
    ("sanitizer-kcfi-arity", Refused(INSTRUMENTS)),
    ("sanitizer-memory-track-origins", Refused(INSTRUMENTS)),
    ("sanitizer-recover", Refused(INSTRUMENTS)),
    ("self-profile", Ignored(PRINTS)),
    ("self-profile-counter", Ignored(PRINTS)),
    ("self-profile-events", Ignored(PRINTS)),
    ("share-generics", Refused(EXPORTS)),
    ("shell-argfiles", Ignored("how rustc reads its own argument files")),
    ("simulate-remapped-rust-src-base", Refused("it changes the paths panics name")),
    ("small-data-threshold", Refused(CODE)),
    ("span-debug", Refused(SEMANTICS)),
    ("span-free-formats", Ignored(PRINTS)),
    ("split-dwarf-inlining", Ignored(DEBUG)),
    ("split-dwarf-kind", Ignored(DEBUG)),
    ("split-dwarf-out-dir", Ignored(DEBUG)),
    ("split-lto-unit", Refused(EXPORTS)),
    ("src-hash-algorithm", Ignored(DEBUG)),
    ("stack-protector", Refused(CODE)),
    ("staticlib-allow-rdylib-deps", Refused(LINKS)),
    ("staticlib-hide-internal-symbols", Refused(LINKS)),
    ("staticlib-prefer-dynamic", Refused(LINKS)),
    ("staticlib-rename-internal-symbols", Refused(LINKS)),
    ("strict-init-checks", Refused(SEMANTICS)),
    ("teach", Ignored(PRINTS)),
    ("temps-dir", Ignored(PRINTS)),
    ("terminal-urls", Ignored(PRINTS)),
    ("thinlto", Refused(EXPORTS)),
    ("threads", Ignored(SPEED)),
    ("time-llvm-passes", Ignored(PRINTS)),
    ("time-passes", Ignored(PRINTS)),
    ("time-passes-format", Ignored(PRINTS)),
    ("tiny-const-eval-limit", Refused(SEMANTICS)),
    ("tls-model", Refused(ABI)),
    ("trace-macros", Ignored(PRINTS)),
    ("track-diagnostics", Ignored(PRINTS)),
    ("translate-remapped-path-to-local-path", Refused("it changes the paths panics name")),
    ("trap-unreachable", Ignored(RUSTC_BACKEND)),
    ("treat-err-as-bug", Ignored(CHECKS)),
    ("trim-diagnostic-paths", Ignored(PRINTS)),
    ("tune-cpu", Ignored(RUSTC_BACKEND)),
    ("ub-checks", OffOnly("blaze compiles `ub_checks()` as false")),
    ("ui-testing", Ignored(PRINTS)),
    ("uninit-const-chunk-threshold", Refused(CODE)),
    ("unleash-the-miri-inside-of-you", Refused(SEMANTICS)),
    ("unpretty", Refused(NOT_BUILT)),
    ("unsound-mir-opts", Refused(CODE)),
    ("unstable-options", Ignored("it lets other unstable options be given, and changes nothing")),
    ("use-ctors-section", Refused(LINKS)),
    ("use-sync-unwind", Refused(CODE)),
    ("validate-mir", Ignored(CHECKS)),
    ("verbose-asm", Ignored(PRINTS)),
    ("verbose-internals", Ignored(PRINTS)),
    ("verify-llvm-ir", Ignored(CHECKS)),
    ("virtual-function-elimination", Refused(EXPORTS)),
    ("wasi-exec-model", Refused(LINKS)),
    ("wasm-c-abi", Refused(ABI)),
    ("wasm-proc-macros", Refused(NOT_BUILT)),
    ("write-long-types-to-disk", Ignored(PRINTS)),
];

/// What a flag the table does not have gets.
const UNKNOWN: &str = "blaze does not know the flag";

/// One `-Z` flag as given: its name with `_` spelled `-`, as rustc reads both, and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Flag<'a> {
    pub(crate) name: String,
    pub(crate) value: Option<&'a str>,
    /// As it was given, after the `-Z`.
    pub(crate) given: &'a str,
}

impl Flag<'_> {
    /// What ra-jit does with it.
    pub(crate) fn treatment(&self) -> Treatment {
        let row = TABLE.binary_search_by(|(name, _)| name.cmp(&self.name.as_str()));
        match row.map(|at| TABLE[at].1) {
            Err(_) => Treatment::Refused(UNKNOWN),
            Ok(Ignored(why)) => Treatment::Ignored(why),
            Ok(Refused(why)) => Treatment::Refused(why),
            Ok(OffOnly(why)) => match off(self.value) {
                Some(true) => Treatment::Modelled(why),
                Some(false) => Treatment::Refused(why),
                None => Treatment::Refused("blaze does not know the value"),
            },
        }
    }

    /// Why ra-jit does not build code with it, naming it, when it does not.
    pub(crate) fn refusal(&self) -> Option<String> {
        match self.treatment() {
            Treatment::Refused(why) => Some(format!("-Z {}: {why}", self.given)),
            Treatment::Ignored(_) | Treatment::Modelled(_) => None,
        }
    }
}

/// Whether a boolean flag's value turns it off, as rustc reads one: no value is on.
fn off(value: Option<&str>) -> Option<bool> {
    match value {
        None | Some("y" | "yes" | "on" | "true") => Some(false),
        Some("n" | "no" | "off" | "false") => Some(true),
        Some(_) => None,
    }
}

/// The `-Z` flags among the words of a rustc command line, spelled `-Z name=value`,
/// `-Zname=value`, `-Z name` or `-Zname`.
pub(crate) fn flags<'a, S: AsRef<str> + 'a>(
    words: impl IntoIterator<Item = &'a S>,
) -> Vec<Flag<'a>> {
    let mut words = words.into_iter().map(AsRef::as_ref);
    let mut found = Vec::new();
    while let Some(word) = words.next() {
        let given = match word.strip_prefix("-Z") {
            Some("") => match words.next() {
                Some(next) => next,
                None => break,
            },
            Some(joined) => joined,
            None => continue,
        };
        found.push(flag(given));
    }
    found
}

/// The flag `given` is, after its `-Z`.
pub(crate) fn flag(given: &str) -> Flag<'_> {
    let (name, value) = match given.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (given, None),
    };
    Flag { name: name.replace('_', "-"), value, given }
}

/// Why ra-jit does not build code with the flags of a rustc command line, naming the first
/// flag it refuses, when it does not.
pub(crate) fn refusal<'a, S: AsRef<str> + 'a>(
    words: impl IntoIterator<Item = &'a S>,
) -> Option<String> {
    flags(words).iter().find_map(Flag::refusal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_sorted_and_has_each_name_once() {
        assert!(TABLE.windows(2).all(|it| it[0].0 < it[1].0), "TABLE must be sorted by name");
    }

    #[test]
    fn spellings() {
        let words =
            ["-Zthreads=8", "-Z", "time-passes", "--cfg", "x", "-Z", "ub_checks=no", "-Zfoo"];
        let found = flags(&words);
        let names: Vec<_> = found.iter().map(|it| (it.name.as_str(), it.value)).collect();
        assert_eq!(
            names,
            [
                ("threads", Some("8")),
                ("time-passes", None),
                ("ub-checks", Some("no")),
                ("foo", None)
            ]
        );
        assert_eq!(found[2].given, "ub_checks=no");
        // A `-Z` at the end has nothing to name.
        assert!(flags(&["-Z"]).is_empty());
    }

    #[test]
    fn treatments() {
        let of = |given| flag(given).treatment();
        assert!(matches!(of("threads=8"), Treatment::Ignored(_)));
        assert!(matches!(of("embed-metadata=no"), Treatment::Ignored(_)));
        assert!(matches!(of("ub-checks=no"), Treatment::Modelled(_)));
        assert!(matches!(of("ub-checks"), Treatment::Refused(_)));
        assert!(matches!(of("ub-checks=yes"), Treatment::Refused(_)));
        assert!(matches!(of("ub-checks=maybe"), Treatment::Refused(_)));
        assert!(matches!(of("share-generics=y"), Treatment::Refused(_)));
        assert_eq!(of("no-such-flag"), Treatment::Refused(UNKNOWN));
    }

    #[test]
    fn a_refusal_names_the_flag() {
        assert_eq!(refusal(&["-Zthreads=8", "-Z", "embed-metadata=no"]), None);
        let why = refusal(&["-Zthreads=8", "-Z", "no_such_flag=3"]).unwrap();
        assert_eq!(why, format!("-Z no_such_flag=3: {UNKNOWN}"));
    }
}
