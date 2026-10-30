//! `RA_JIT_TESTS=affected`: `cargo ra-jit test` runs, of each program of tests the daemon
//! built, the tests that have not passed with what they are a function of as it is now, and
//! says how many passed before and where ([`results`]). The daemon writes beside the program a
//! hash of each test's code with all it reaches ([`plan::REACH`]). A test that reaches the
//! world outside its code runs in the sandbox every test runs in, which records what it read,
//! and that is part of its key too ([`results::Results::find`]); outside the sandbox
//! (`RA_JIT_SANDBOX=off`, or a test its package takes out of it) it runs every time, as does a
//! test that reaches code ra-jit can't compile. Unset, or `all`, runs every test, as cargo does.
//!
//! `RA_JIT_TESTS=verify` picks the same, runs every test anyway and says of each that had a
//! pass whether it passed again: one that failed is flaky, or its key misses something it is a
//! function of, which is a bug.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::{
    cli::Invocation,
    plan,
    results::{self, Found, Key, Results},
    state::{Kind, Program, State},
};
use crate::sandbox::Line;

pub(crate) const VAR: &str = "RA_JIT_TESTS";

/// The most bytes of names a program of tests is given to pick its tests by; past it, it runs
/// them all.
const NAMES_AT_MOST: usize = 64 << 10;

/// Whether this command runs only the tests that have not passed as they are now, or checks
/// that those that did pass again ([`verifying`]).
pub(crate) fn on() -> bool {
    std::env::var(VAR).is_ok_and(|it| it == "affected" || it == "verify")
}

/// Whether this command runs every test and checks that those that passed before pass again.
pub(crate) fn verifying() -> bool {
    std::env::var(VAR).is_ok_and(|it| it == "verify")
}

/// Has the daemon write [`plan::REACH`] at its builds of the programs of tests of `state`
/// while only the tests an edit reached are run, and stop when every test is.
pub(crate) fn ask(state: &State) {
    let on = on();
    for program in state.programs.iter().filter(|it| it.test) {
        let Some(dir) = state.plan_dir(program) else { continue };
        let wanted = dir.join(plan::REACH_WANTED);
        match on {
            true if wanted.is_file() => {}
            // A fresh target has no directory for the program until it is built.
            true => _ = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&wanted, "")),
            false => {
                _ = std::fs::remove_file(&wanted);
                _ = std::fs::remove_file(dir.join(plan::REACH));
            }
        }
    }
}

/// What of a program of tests this command runs.
pub(crate) enum Pick {
    /// What the command asks, every test unless it names some: nothing is known of what an
    /// edit reached, or the command picks the tests itself.
    Asked,
    /// The tests `names` names, of the `total` the program has, or all of them when `None`;
    /// `always` are those of them run every time, and `reaching` those that reach the world
    /// outside their code and run in the sandbox with what they read recorded, and why each
    /// runs.
    Some {
        names: Option<Vec<String>>,
        total: usize,
        always: Vec<Always>,
        reaching: Vec<(String, String)>,
        /// The tests that passed under their keys, each with where: not run, or under
        /// [`verifying`] run to see that they pass again.
        cached: BTreeMap<String, String>,
        /// The tests that run every time for having passed and failed under one key, with
        /// where they passed.
        flaky: Vec<(String, String)>,
        keys: Keys,
    },
}

/// A test that is run every time, and why.
pub(crate) struct Always {
    pub(crate) name: String,
    pub(crate) why: String,
    /// The calls that take the test to what it reaches, from the test on, joined by ` -> `.
    pub(crate) path: Option<String>,
    /// Whether it runs outside the sandbox, taken out by its package or the command.
    pub(crate) outside: bool,
}

/// The first key of each test of a program that may be kept, and whether it reaches the world
/// outside its code.
pub(crate) struct Keys(BTreeMap<String, (Key, bool)>);

/// What a run of a program of tests ended with, for [`Keys::settle`].
pub(crate) struct Ended<'a> {
    /// The tests it was asked to run, all `total` of them when `None`.
    pub(crate) ran: Option<&'a [String]>,
    pub(crate) total: usize,
    /// Whether it failed.
    pub(crate) failed: bool,
    /// The tests libtest's summary said failed, or the sandbox failed, when that is known.
    pub(crate) failures: Option<&'a [String]>,
    /// How many tests libtest's summaries counted (passed, failed and ignored).
    pub(crate) counted: Option<usize>,
    /// What the sandbox recorded of the run.
    pub(crate) record: Option<&'a [Line]>,
}

impl Keys {
    /// Keeps in `results` how the tests of a run `ended`: those that passed, and those that
    /// failed, for a flaky test to be told. Nothing when it is not known which tests ran and
    /// how each ended. The names of the tests found flaky.
    pub(crate) fn settle(&self, results: &mut Results, ended: &Ended<'_>) -> Vec<String> {
        let ran: Vec<&String> = match ended.ran {
            Some(names) => names.iter().collect(),
            None => self.0.keys().collect(),
        };
        let failures: BTreeSet<&str> = match (ended.failed, ended.failures) {
            (false, _) => BTreeSet::new(),
            (true, Some(failures)) => failures.iter().map(String::as_str).collect(),
            (true, None) => return Vec::new(),
        };
        // A summary that counts other tests than were asked for says nothing of each.
        if ended.counted != Some(ended.ran.map_or(ended.total, <[String]>::len)) {
            return Vec::new();
        }
        let mut flaky = Vec::new();
        for name in ran {
            let Some((first, reaches)) = self.0.get(name) else { continue };
            let reads = match (reaches, ended.record) {
                (false, _) => None,
                (true, Some(record)) => match results::reads_of(record, name) {
                    Some(reads) => Some(reads),
                    None => continue,
                },
                (true, None) => continue,
            };
            if results.keep(first, reads.as_ref(), !failures.contains(name.as_str())) {
                flaky.push(name.clone());
            }
        }
        results.flush();
        flaky
    }
}

/// Which tests of `test` this command runs. `results` are the store's, `None` when there is
/// none; `run` stands for what the command builds and runs the tests with; `sandbox` is what
/// stands for the environment of a sandboxed test ([`super::sandboxed::identity_of`]) when the
/// tests run sandboxed, and why they run outside otherwise: a test outside is not hermetic,
/// and runs every time, as do those of `outside`, which the manifest `manifest` takes out.
#[allow(clippy::too_many_arguments)]
pub(crate) fn pick(
    asked: &Invocation,
    state: &State,
    test: &Program,
    results: Option<&mut Results>,
    run: &str,
    sandbox: Result<&str, &str>,
    outside: &BTreeSet<String>,
    manifest: &Path,
) -> Pick {
    if !on() || !asked.positional.is_empty() || !plain(&asked.trailing) {
        return Pick::Asked;
    }
    let Some(dir) = state.plan_dir(test) else { return Pick::Asked };
    let Ok(reach) = std::fs::read_to_string(dir.join(plan::REACH)) else { return Pick::Asked };
    let programs = programs_run_by(state, test);
    let mut results = results;
    let (mut names, mut keys, mut always, mut reaching) =
        (Vec::new(), BTreeMap::new(), Vec::new(), Vec::new());
    let (mut cached, mut flaky, mut total) = (BTreeMap::new(), Vec::new(), 0);
    for line in reach.lines() {
        let mut parts = line.split('\t');
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else { continue };
        if hash == "*" {
            return Pick::Asked;
        }
        total += 1;
        let reaches = hash.strip_prefix('~');
        let outside_why = match sandbox {
            Err(why) => Some(format!("it runs outside the sandbox: {why}")),
            Ok(_) if outside.contains(name) => {
                Some(format!("its package takes it out of the sandbox ({})", manifest.display()))
            }
            Ok(_) => None,
        };
        if let Some(why) = outside_why {
            names.push(name.to_owned());
            always.push(Always { name: name.to_owned(), why, path: None, outside: true });
            continue;
        }
        if hash == "-" {
            names.push(name.to_owned());
            let why = parts.next().unwrap_or_default().to_owned();
            let path = parts.next().map(str::to_owned);
            always.push(Always { name: name.to_owned(), why, path, outside: false });
            continue;
        }
        let identity = match (reaches, sandbox) {
            (Some(hash), Ok(env)) => format!("~{hash}{programs}{env}{run}"),
            _ => format!("{hash}{programs}{run}"),
        };
        let first = Results::first(&identity, name);
        keys.insert(name.to_owned(), (first, reaches.is_some()));
        let found =
            results.as_mut().map_or(Found::Missing(None), |it| it.find(&first, reaches.is_some()));
        match found {
            Found::Passed { from } => {
                cached.insert(name.to_owned(), from);
                continue;
            }
            Found::Flaky { from } => flaky.push((name.to_owned(), from)),
            Found::Missing(why) => {
                if let (Some(why), true) = (why, reaches.is_some()) {
                    reaching.push((name.to_owned(), why));
                    names.push(name.to_owned());
                    continue;
                }
            }
        }
        names.push(name.to_owned());
        if reaches.is_some() {
            reaching.push((
                name.to_owned(),
                "it has not passed with what it read as it is now".to_owned(),
            ));
        }
    }
    if verifying() {
        names.extend(cached.keys().cloned());
        reaching.extend(
            cached
                .keys()
                .filter(|it| keys.get(*it).is_some_and(|(_, reaches)| *reaches))
                .map(|it| (it.clone(), "it is verified".to_owned())),
        );
    }
    let long = names.iter().map(|it| it.len() + 1).sum::<usize>() > NAMES_AT_MOST;
    let names = (names.len() != total && !long).then_some(names);
    Pick::Some { names, total, always, reaching, cached, flaky, keys: Keys(keys) }
}

/// What stands for what the command `asked` builds and runs its tests with, beside each test's
/// code: the features, profile, targets, configuration and unstable flags it builds with, the
/// machine, and the harness's flags that change how a test runs (all but those that only change
/// what is shown).
pub(crate) fn run_identity(asked: &Invocation) -> String {
    let flags = &asked.flags;
    let mut features = flags.features.clone();
    features.sort();
    let shown_only = |it: &&String| {
        matches!(it.as_str(), "-q" | "--quiet" | "--show-output") || it.starts_with("--color")
    };
    let mut trailing = asked.trailing.iter().filter(|it| !shown_only(it)).peekable();
    let mut harness = Vec::new();
    while let Some(arg) = trailing.next() {
        harness.push(arg.clone());
        if arg == "--test-threads" {
            harness.extend(trailing.next().cloned());
        }
    }
    let fields = [
        format!(
            "features {features:?} all {} no-default {}",
            flags.all_features, flags.no_default_features
        ),
        format!("profile {:?}", flags.profile),
        format!("targets {:?}", flags.targets),
        format!("config {:?} unstable {:?}", flags.config, flags.unstable),
        format!("on {}-{}", std::env::consts::ARCH, std::env::consts::OS),
        format!("harness {harness:?}"),
    ];
    let hash = sha2::Digest::finalize(<sha2::Sha256 as sha2::Digest>::new_with_prefix(
        fields.join("\n").as_bytes(),
    ));
    format!("+{}", crate::content::hex(&hash[..16]))
}

/// Whether what follows `--` leaves the tests to the program to pick and says nothing of how
/// a test ends: what only shows the run otherwise. One that runs the ignored tests, or lists
/// them, runs as asked.
pub(crate) fn plain(trailing: &[String]) -> bool {
    let mut args = trailing.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-q" | "--quiet" | "--nocapture" | "--no-capture" | "--show-output" => {}
            "--test-threads" | "--color" if args.next().is_some() => {}
            _ if arg.starts_with("--test-threads=") || arg.starts_with("--color=") => {}
            _ => return false,
        }
    }
    true
}

/// What stands for the programs an integration test of `test`'s package runs, which cargo
/// gives it as `CARGO_BIN_EXE_<name>`: their bytes. Empty for a unit test.
fn programs_run_by(state: &State, test: &Program) -> String {
    if !matches!(test.kind, Kind::Test | Kind::Bench) {
        return String::new();
    }
    let runs = |it: &&Program| it.package == test.package && it.kind == Kind::Bin && !it.test;
    let paths: Vec<&Path> =
        state.programs.iter().filter(runs).map(|it| it.executable.as_path()).collect();
    results::programs(&paths)
}

#[cfg(test)]
mod tests {
    use super::plain;

    fn of(args: &[&str]) -> bool {
        plain(&args.iter().map(|it| (*it).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn what_only_shows_the_run_is_plain() {
        assert!(of(&[]));
        assert!(of(&["--nocapture", "-q", "--test-threads", "4", "--color=never"]));
        assert!(!of(&["--ignored"]));
        assert!(!of(&["--include-ignored"]));
        assert!(!of(&["--list"]));
        assert!(!of(&["--test-threads"]));
        assert!(!of(&["some_test"]));
    }
}
