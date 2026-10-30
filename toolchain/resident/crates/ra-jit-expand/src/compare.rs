//! The toolchain's proc-macro server held against ra-jit's (docs/no-rustc.md, step J1): with
//! `RA_JIT_PROC_MACRO_COMPARE` naming the server built from `vendor/proc-macro-srv` and
//! `RA_JIT_PROC_MACRO_LIBS` a directory of the libraries ra-jit linked (named as rustc's, or
//! `lib<crate>` and the platform's suffix, with the list of their macros in them), each call the toolchain's server
//! expands with rustc's library of a crate is expanded again by that server with ra-jit's
//! library of the crate, given the same input, and the outputs, spans and all, are compared.

use std::{
    cell::Cell,
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use base_db::{Env, SourceDatabase};
use hir_expand::proc_macro::{ProcMacro, ProcMacroKind};
use paths::{AbsPath, AbsPathBuf};
use proc_macro_api::ProcMacroClient;
use rustc_hash::FxHashMap;
use span::Span;

thread_local! {
    /// Set while ra-jit's server expands a call again, whose expander is wrapped as every
    /// other is: that expansion is neither kept nor compared.
    static COMPARING: Cell<bool> = const { Cell::new(false) };
}

/// Whether this thread is expanding a call again for a comparison.
pub(crate) fn comparing() -> bool {
    COMPARING.with(Cell::get)
}

struct Compare {
    server: ProcMacroClient,
    libs: PathBuf,
    /// The macros of ra-jit's library of each of rustc's, by the path of rustc's, or why
    /// there are none.
    loaded: Mutex<FxHashMap<AbsPathBuf, Result<Vec<ProcMacro>, String>>>,
}

fn compare() -> Option<&'static Compare> {
    static COMPARE: OnceLock<Option<Compare>> = OnceLock::new();
    COMPARE
        .get_or_init(|| {
            let server = std::env::var_os("RA_JIT_PROC_MACRO_COMPARE")?;
            let libs = std::env::var_os("RA_JIT_PROC_MACRO_LIBS")?;
            let server = AbsPathBuf::assert_utf8(std::path::absolute(server).ok()?);
            let none: [(&str, &Option<&str>); 0] = [];
            let server = match ProcMacroClient::spawn(&server, none, None, 1) {
                Ok(it) => it,
                Err(e) => {
                    eprintln!("proc macros: blaze's server {server} did not start: {e}");
                    return None;
                }
            };
            Some(Compare { server, libs: libs.into(), loaded: Mutex::default() })
        })
        .as_ref()
}

/// The crate rustc's library at `path` is of: cargo names it `lib<crate>-<hash>` and the
/// platform's suffix.
fn crate_of(path: &AbsPath) -> Option<&str> {
    let stem = path.file_stem()?;
    let stem = stem.strip_prefix("lib").unwrap_or(stem);
    Some(stem.rsplit_once('-').map_or(stem, |(name, _)| name))
}

impl Compare {
    /// ra-jit's library of rustc's at `dylib`, loaded into ra-jit's server: the one of the
    /// same file name, as ra-jit links it into a target directory where cargo asked rustc for
    /// it, which tells two versions of a crate apart, or else the one of its crate.
    fn macro_of(&self, dylib: &AbsPath, name: &str) -> Result<ProcMacro, String> {
        let mut loaded = self.loaded.lock().unwrap_or_else(|it| it.into_inner());
        let macros = loaded.entry(dylib.to_path_buf()).or_insert_with(|| -> Result<_, String> {
            let krate = crate_of(dylib).ok_or_else(|| format!("{dylib} names no crate"))?;
            let file = format!("lib{krate}{}", std::env::consts::DLL_SUFFIX);
            let same = dylib.file_name().map(|it| self.libs.join(it)).filter(|it| it.exists());
            let path = same.unwrap_or_else(|| self.libs.join(file));
            let path = std::path::absolute(path).map_err(|e| e.to_string())?;
            if !path.exists() {
                return Err(format!("blaze linked no {}", path.display()));
            }
            let path = AbsPathBuf::assert_utf8(path);
            load_cargo::load_proc_macro(&self.server, &path, &[]).map_err(|e| e.to_string())
        });
        let macros = macros.as_ref().map_err(Clone::clone)?;
        let found = macros.iter().find(|it| it.name.as_str() == name);
        found.cloned().ok_or_else(|| format!("blaze's library of {dylib} lists no `{name}`"))
    }
}

/// Expands the call again with ra-jit's server and library when asked to, and counts whether
/// it gave what the toolchain's did.
#[allow(clippy::too_many_arguments)]
pub(crate) fn again(
    dylib: &AbsPath,
    name: &str,
    kind: ProcMacroKind,
    db: &dyn SourceDatabase,
    subtree: &tt::TopSubtree,
    attrs: Option<&tt::TopSubtree>,
    env: &Env,
    def_site: Span,
    call_site: Span,
    mixed_site: Span,
    current_dir: &str,
    toolchain: &Result<tt::TopSubtree, hir_expand::proc_macro::ProcMacroExpansionError>,
) {
    let Some(compare) = compare() else { return };
    let label = format!("{}::{name}", crate_of(dylib).unwrap_or("?"));
    let mac = match compare.macro_of(dylib, name) {
        Ok(it) => it,
        Err(why) => {
            COUNTS.unloaded.fetch_add(1, Ordering::Relaxed);
            noted(&label, NOT_COMPARED, &format!("not compared: {why}"));
            return;
        }
    };
    COMPARING.with(|it| it.set(true));
    let ours = mac.expander.expand(
        db,
        subtree,
        attrs,
        env,
        def_site,
        call_site,
        mixed_site,
        current_dir.to_owned(),
    );
    COMPARING.with(|it| it.set(false));
    COUNTS.compared[crate::kind_index(kind)].fetch_add(1, Ordering::Relaxed);
    let same = match (toolchain, &ours) {
        (Ok(theirs), Ok(ours)) => theirs == ours,
        (Err(theirs), Err(ours)) => {
            COUNTS.failed.fetch_add(1, Ordering::Relaxed);
            format!("{theirs:?}") == format!("{ours:?}")
        }
        _ => false,
    };
    if !same {
        COUNTS.differed.fetch_add(1, Ordering::Relaxed);
        let said = |it: &Result<tt::TopSubtree, _>| match it {
            Ok(tree) => format!("{tree:?}"),
            Err(e) => format!("error: {e:?}"),
        };
        noted(
            &label,
            OTHERWISE,
            &format!(
                "blaze's server and library expanded otherwise than the toolchain's:\n\
                 toolchain's: {}\nblaze's:     {}",
                said(toolchain),
                said(&ours)
            ),
        );
    }
}

const OTHERWISE: usize = 0;
const NOT_COMPARED: usize = 1;

/// The calls of each macro, `crate::name`, expanded otherwise and not compared.
static NAMED: Mutex<BTreeMap<String, [u64; 2]>> = Mutex::new(BTreeMap::new());

/// Counts a call of the macro `label` as `which`, and says `what` of the first.
fn noted(label: &str, which: usize, what: &str) {
    let mut named = NAMED.lock().unwrap_or_else(|it| it.into_inner());
    let calls = named.entry(label.to_owned()).or_default();
    calls[which] += 1;
    if calls[which] == 1 {
        eprintln!("proc macros: `{label}`: {what}");
    }
}

struct Counts {
    compared: [AtomicU64; 3],
    differed: AtomicU64,
    /// Calls both servers failed to expand, the same way or not.
    failed: AtomicU64,
    unloaded: AtomicU64,
}

static COUNTS: Counts = Counts {
    compared: [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)],
    differed: AtomicU64::new(0),
    failed: AtomicU64::new(0),
    unloaded: AtomicU64::new(0),
};

/// What the comparison found so far, for the report: `None` when there is none.
pub(crate) fn report() -> Option<String> {
    compare()?;
    let compared: Vec<u64> = COUNTS.compared.iter().map(|it| it.load(Ordering::Relaxed)).collect();
    let kinds = ["derives", "attributes", "function-like"];
    let kinds: Vec<_> =
        kinds.iter().zip(&compared).map(|(what, calls)| format!("{calls} {what}")).collect();
    let named = NAMED.lock().unwrap_or_else(|it| it.into_inner());
    let of = |which: usize| -> Vec<String> {
        let calls = named.iter().filter(|(_, calls)| calls[which] != 0);
        calls.map(|(label, calls)| format!("{label} {}", calls[which])).collect()
    };
    let mut said = format!(
        "compared with blaze's server: {} calls ({}), {} expanded otherwise, {} failed by both, \
         {} not compared",
        compared.iter().sum::<u64>(),
        kinds.join(", "),
        COUNTS.differed.load(Ordering::Relaxed),
        COUNTS.failed.load(Ordering::Relaxed),
        COUNTS.unloaded.load(Ordering::Relaxed),
    );
    for (which, what) in [(OTHERWISE, "expanded otherwise"), (NOT_COMPARED, "not compared")] {
        let calls = of(which);
        if !calls.is_empty() {
            said += &format!(" ({what}: {})", calls.join(", "));
        }
    }
    Some(said)
}
