//! Running what was built, as cargo does: the program of `cargo run`, the test executables
//! of `cargo test`.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::{Command, ExitStatus},
};

use anyhow::Context as _;

use super::{
    affected::{self, Pick},
    cli::Invocation,
    experimental,
    results::Results,
    sandboxed, say,
    state::{Held, Kind, Program, State, as_built},
};

/// The exit status of cargo when a test fails or a program can't be run.
const FAILED: i32 = 101;

/// Sets for `command` what the `[env]` of cargo's configuration sets, as cargo does: what
/// cargo sets for the program itself goes over it.
fn configured(command: &mut Command, state: &State) {
    for it in &state.configured {
        if it.force || std::env::var_os(&it.name).is_none() {
            command.env(&it.name, &it.value);
        }
    }
}

/// Runs `command` to its end, started under `held` or under cargo's locks on the build
/// directories taken now: a cargo building there meanwhile removes the executables and
/// writes them again, and one started then is not there.
fn ended(command: &mut Command, held: Option<Held>, state: &State) -> std::io::Result<ExitStatus> {
    let held = held.unwrap_or_else(|| state.hold());
    let started = started(command);
    drop(held);
    started?.wait()
}

/// The most of what a program of tests printed last that is kept to read its failures from:
/// one whose list of them is longer is taken to have said none.
const PRINTED_KEPT: usize = 4 << 20;

/// Runs `command`, which runs `program`, as [`ended`] does, with what it prints passed on as it comes, and the tests
/// libtest's summary at its end says failed: `None` when it says none, as a program that
/// crashed, or exited in a test, does not. libtest colors its output as it would on this
/// terminal, which it does not see through the pipe.
fn listing_failures(
    command: &mut Command,
    program: &Path,
    held: Option<Held>,
    state: &State,
) -> std::io::Result<(ExitStatus, Summary)> {
    use std::io::{IsTerminal as _, Read as _, Write as _};
    let out = std::io::stdout();
    let colored = command.get_args().any(|it| {
        let it = it.to_string_lossy();
        it == "--color" || it.starts_with("--color=")
    });
    if !colored && out.is_terminal() {
        command.arg("--color=always");
    }
    command.stdout(std::process::Stdio::piped());
    let held = held.unwrap_or_else(|| state.hold());
    let child = started_running(command, program);
    drop(held);
    let mut child = child?;
    let mut printed = Vec::new();
    if let Some(mut from) = child.stdout.take() {
        let mut chunk = [0; 8192];
        loop {
            let read = match from.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => read,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            // A reader that went away does not stop the tests, as it would not under cargo.
            let mut out = out.lock();
            _ = out.write_all(&chunk[..read]).and_then(|()| out.flush());
            printed.extend_from_slice(&chunk[..read]);
            if printed.len() > PRINTED_KEPT {
                printed.drain(..printed.len() - PRINTED_KEPT);
            }
        }
    }
    let status = child.wait()?;
    let printed = String::from_utf8_lossy(&printed);
    Ok((status, Summary { failures: failures(&printed), counted: tests_counted(&printed) }))
}

/// What libtest's summaries at the end of a run say.
struct Summary {
    /// The tests listed as failed, `None` when none are.
    failures: Option<Vec<String>>,
    /// How many tests the summaries count as passed, failed or ignored, `None` without one.
    counted: Option<usize>,
}

/// How many tests the `test result:` lines of what a program of tests printed count as passed,
/// failed or ignored: those it ran or knew of, but not those it filtered out.
fn tests_counted(printed: &str) -> Option<usize> {
    let plain = without_colors(printed);
    let mut total = None;
    for line in plain.lines().filter_map(|it| it.strip_prefix("test result: ")) {
        let mut sum = 0;
        for part in line.split(';') {
            let mut words = part.split_whitespace().rev();
            let (Some(what), Some(count)) = (words.next(), words.next()) else { continue };
            if matches!(what, "passed" | "failed" | "ignored")
                && let Ok(count) = count.parse::<usize>()
            {
                sum += count;
            }
        }
        *total.get_or_insert(0) += sum;
    }
    total
}

/// The tests libtest's summary lists as failed in what a program of tests printed: the names
/// under the last `failures:` before `test result: FAILED`.
fn failures(printed: &str) -> Option<Vec<String>> {
    let plain = without_colors(printed);
    let end = plain.rfind("\ntest result: FAILED")?;
    let start = plain[..end].rfind("\nfailures:\n")? + "\nfailures:\n".len();
    let names: Vec<String> = plain[start..end]
        .lines()
        .filter_map(|it| it.strip_prefix("    "))
        .map(str::to_owned)
        .collect();
    (!names.is_empty()).then_some(names)
}

/// `text` without the escapes that color a terminal's text.
fn without_colors(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.next() == Some('[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}

/// Starts `command` once the daemon is not writing its program (`splice::unwritten`): one
/// started while the file is open for writing is refused ("text file busy").
pub(crate) fn started(command: &mut Command) -> std::io::Result<std::process::Child> {
    let program = std::path::PathBuf::from(command.get_program());
    started_running(command, &program)
}

/// Starts `command`, which runs `program` (itself, or in a sandbox), as [`started`] does.
fn started_running(command: &mut Command, program: &Path) -> std::io::Result<std::process::Child> {
    #[cfg(unix)]
    let _unwritten = crate::splice::unwritten(program);
    #[cfg(not(unix))]
    let _ = program;
    command.spawn()
}

/// Runs the test executables one after another, each in the directory of its package, and in
/// a sandbox ([`sandboxed`]) when its workspace or package opts in and neither it nor the
/// command takes it out, and stops at the first that fails unless told not to. The result is
/// cargo's exit status. Under `RA_JIT_TESTS=affected` each runs only the tests that have not
/// passed with what they are a function of as it is now, as the store keeps them
/// ([`results`]); `verify` runs those too and says whether they passed again ([`affected`]).
pub(crate) fn tests(
    asked: &Invocation,
    state: &State,
    cwd: &Path,
    mut held: Option<Held>,
) -> anyhow::Result<i32> {
    let mut failed = Vec::new();
    let mut tally = Tally::default();
    let programs: Vec<&Program> = state.programs.iter().filter(|it| it.test).collect();
    let mut opt_outs: BTreeMap<&Path, sandboxed::OptOut> = BTreeMap::new();
    for test in &programs {
        if !opt_outs.contains_key(test.package_dir.as_path()) {
            opt_outs.insert(&test.package_dir, sandboxed::of_tests(&test.package_dir)?);
        }
    }
    let sandboxing = sandboxed::on();
    if sandboxing
        && opt_outs.values().any(sandboxed::OptOut::sandboxed)
        && let Err(why) = crate::sandbox::available()
    {
        say::error(&format!("the tests run in a sandbox, which can't be made here: {why}"));
        say::note(&format!(
            "`{}=off` runs the tests of this command outside a sandbox, as cargo does",
            crate::sandbox::VAR_BLAZE
        ));
        return Ok(FAILED);
    }
    let not_opted_in = opt_outs.values().find(|it| !it.all && !it.sandboxed());
    if affected::on()
        && sandboxing
        && !opt_outs.values().any(sandboxed::OptOut::sandboxed)
        && let Some(it) = not_opted_in
    {
        say::note(&format!(
            "no test is sandboxed, so none is kept as passed and every test runs: sandboxed \
             tests are an experiment a workspace opts into, with {}",
            it.experiments.how_to(experimental::SANDBOXED_TESTS)
        ));
    }
    let mut results = affected::on().then(Results::open).flatten();
    if affected::on() && results.is_none() {
        say::note(
            "no store (`BLAZE_CACHE=off`): no test's pass is kept, and every test the daemon \
             built runs",
        );
    }
    let run = affected::run_identity(asked);
    let mut scratch = sandboxed::Scratch::new();
    let no_test = BTreeSet::new();
    for test in programs {
        let opt_out = &opt_outs[test.package_dir.as_path()];
        let sandboxed_here = sandboxing && opt_out.sandboxed();
        let source = test.source.strip_prefix(&test.package_dir).unwrap_or(&test.source);
        let unit = if matches!(test.kind, Kind::Test | Kind::Bench) { "" } else { "unittests " };
        let shown = shown_path(&test.executable, cwd);
        let mut running = format!("{unit}{} ({shown})", source.display());
        let bare = || -> anyhow::Result<Command> {
            let mut command = Command::new(&test.executable);
            configured(&mut command, state);
            command.envs(test.env.iter().cloned());
            command.current_dir(&test.package_dir);
            as_built(&mut command)?;
            Ok(command)
        };
        let env = match (sandboxed_here, affected::on()) {
            (false, _) => None,
            (true, true) => Some(sandboxed::env(&bare()?)),
            (true, false) => Some(sandboxed::whole_env(&bare()?)),
        };
        let identity = env.as_deref().filter(|_| affected::on()).map(sandboxed::identity_of);
        let manifest = opt_out.manifest.display();
        let why_outside = match (sandboxing, opt_out.all) {
            (false, _) => format!("`{}=off`", crate::sandbox::VAR_BLAZE),
            (true, true) => format!("`tests = false` under `{}` in {manifest}", sandboxed::TABLE),
            (true, false) => format!(
                "its workspace has not opted into sandboxed tests ({})",
                opt_out.experiments.how_to(experimental::SANDBOXED_TESTS)
            ),
        };
        let sandbox = identity.as_deref().ok_or(why_outside.as_str());
        let outside = if sandboxed_here { &opt_out.tests } else { &no_test };
        let pick = affected::pick(
            asked,
            state,
            test,
            results.as_mut(),
            &run,
            sandbox,
            outside,
            &opt_out.manifest,
        );
        let mut picked = None;
        if let Pick::Some { names, total, cached, .. } = &pick {
            for from in cached.values() {
                *tally.cached.entry(from.clone()).or_default() += 1;
            }
            if let Some(names) = names {
                if names.is_empty() {
                    let passed = format!("{}, passed before as they are now", counted(*total));
                    say::status(asked.flags.quiet, "Cached", &format!("{running}: {passed}"));
                    continue;
                }
                running.push_str(&format!(": {} of {}", names.len(), counted(*total)));
                picked = Some(names.as_slice());
            }
        }
        say::status(asked.flags.quiet, "Running", &running);
        let verbose = |word: &str, rest: &str| {
            if asked.flags.verbose {
                say::status(asked.flags.quiet, word, rest);
            }
        };
        let mut reads = false;
        if let Pick::Some { always, reaching, flaky, .. } = &pick {
            for test in always {
                let by = test.path.as_ref().map(|it| format!(", by {it}")).unwrap_or_default();
                verbose("Always", &format!("{}: {}{by}", test.name, test.why));
                match test.outside {
                    true => tally.outside.push(test.name.clone()),
                    false => tally.always += 1,
                }
            }
            for (name, why) in reaching {
                verbose("Sandboxed", &format!("{name}: {why}"));
            }
            for (name, from) in flaky {
                say::warning(&format!(
                    "{name} is flaky: it passed and failed with what it is a function of the \
                     same (found in {from}), so it runs every time and is never kept"
                ));
            }
            reads = !reaching.is_empty();
        }
        if sandboxing && opt_out.all {
            let why = format!("`tests = false` under `{}` in {manifest}", sandboxed::TABLE);
            verbose("Unsandboxed", &format!("every test of {}: {why}", test.package));
        }
        let parts = parts(asked, test, sandboxed_here, outside, picked, &bare, &running)?;
        let (mut ended_badly, mut failures, mut record, mut counts) =
            (false, Some(Vec::new()), None, Some(0));
        for part in parts {
            for name in &part.outside {
                verbose("Unsandboxed", &format!("{name}: taken out of the sandbox in {manifest}"));
            }
            let (mut command, sandbox) = match part.sandboxed {
                true => {
                    let env = env.clone().unwrap_or_default();
                    let sandbox = scratch.sandbox(state, test, env, reads)?;
                    let mut command = sandbox.command(&test.executable)?;
                    command.current_dir(&test.package_dir);
                    (command, Some(sandbox))
                }
                false => (bare()?, None),
            };
            command.args(&part.args);
            let ran = match part.sandboxed || matches!(pick, Pick::Some { .. }) {
                true => listing_failures(&mut command, &test.executable, held.take(), state),
                false => ended(&mut command, held.take(), state)
                    .map(|it| (it, Summary { failures: None, counted: None })),
            };
            let (ended, summary) = ran.with_context(|| format!("could not execute {shown}"))?;
            let (mut these, mut failed_here) = (summary.failures, !ended.success());
            if let Some(sandbox) = &sandbox {
                record = sandbox.record();
                if let Some(lines) = &record {
                    let denied = sandboxed::denied(lines, &shown, opt_out);
                    if denied.unknown {
                        (failed_here, these) = (true, None);
                    } else if !denied.tests.is_empty() {
                        let mut all: BTreeSet<String> =
                            these.unwrap_or_default().into_iter().collect();
                        all.extend(denied.tests);
                        (failed_here, these) = (true, Some(all.into_iter().collect()));
                    }
                }
            }
            counts = counts.zip(summary.counted).map(|(a, b)| a + b);
            if failed_here {
                ended_badly = true;
                match (&mut failures, these) {
                    (Some(all), Some(these)) => all.extend(these),
                    _ => failures = None,
                }
            }
        }
        if let (Pick::Some { names, total, keys, cached, .. }, Some(results)) =
            (&pick, results.as_mut())
        {
            let flaky = keys.settle(
                results,
                &affected::Ended {
                    ran: names.as_deref(),
                    total: *total,
                    failed: ended_badly,
                    failures: failures.as_deref(),
                    counted: counts,
                    record: record.as_deref(),
                },
            );
            for name in &flaky {
                say::warning(&format!(
                    "{name} is flaky: it ended otherwise before with what it is a function of \
                     the same, so it is never kept and runs every time"
                ));
            }
            if affected::verifying() {
                let failed: BTreeSet<&str> =
                    failures.iter().flatten().map(String::as_str).collect();
                for (name, from) in cached {
                    match (ended_badly, failures.is_some(), failed.contains(name.as_str())) {
                        (true, false, _) => tally.unverified += 1,
                        (_, _, true) => tally.wrong.push(format!("{name} (passed in {from})")),
                        _ => tally.verified += 1,
                    }
                }
            }
        }
        if ended_badly {
            failed.push(rerun_flags(test));
            if !asked.flags.no_fail_fast {
                break;
            }
        }
    }
    if let Some(results) = &results {
        results.seal();
    }
    tally.say(asked);
    match failed.as_slice() {
        [] => Ok(0),
        [one] => {
            say::error(&format!("test failed, to rerun pass `{one}`"));
            Ok(FAILED)
        }
        several => {
            say::error(&format!("{} targets failed:", several.len()));
            several.iter().for_each(|it| eprintln!("    `{it}`"));
            Ok(FAILED)
        }
    }
}

/// What a command's test executables came to, said at its end.
#[derive(Default)]
struct Tally {
    /// The tests not run for having passed as they are now, by where they passed.
    cached: BTreeMap<String, usize>,
    /// The tests run every time: benchmarks and those that reach what ra-jit can't compile.
    always: usize,
    /// The tests run outside the sandbox, every time.
    outside: Vec<String>,
    /// Under `verify`, the tests that passed again, those that failed, and those of a run
    /// that ended without saying how each did.
    verified: usize,
    wrong: Vec<String>,
    unverified: usize,
}

impl Tally {
    fn say(&self, asked: &Invocation) {
        let quiet = asked.flags.quiet;
        let why = if asked.flags.verbose { "said above" } else { "`-v` says why" };
        let all = format!("`{}=all` runs every test", affected::VAR);
        let mut rest = Vec::new();
        if self.always > 0 {
            rest.push(format!("{} run every time ({why})", counted(self.always)));
        }
        if !self.outside.is_empty() {
            const SHOWN: usize = 5;
            let mut names = self.outside[..self.outside.len().min(SHOWN)].join(", ");
            if self.outside.len() > SHOWN {
                names.push_str(&format!(" and {} more", self.outside.len() - SHOWN));
            }
            rest.push(format!(
                "{} outside the sandbox, run every time and never kept: {names}",
                counted(self.outside.len())
            ));
        }
        let verifying = affected::verifying();
        if !self.cached.is_empty() && !verifying {
            let mut from: Vec<(&String, &usize)> = self.cached.iter().collect();
            from.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let from: Vec<String> = from
                .iter()
                .enumerate()
                .map(|(at, (from, count))| match at {
                    0 => format!("{} from {from}", counted(**count)),
                    _ => format!("{count} from {from}"),
                })
                .collect();
            rest.insert(0, from.join(", "));
            rest.push(all);
            say::status(quiet, "Cached", &rest.join("; "));
        } else if !rest.is_empty() {
            say::status(quiet, "Tests", &rest.join("; "));
        }
        if !verifying {
            return;
        }
        let verified =
            format!("{} of {} passed again", self.verified, counted(self.cached.values().sum()));
        match (self.wrong.as_slice(), self.unverified) {
            ([], 0) => say::status(quiet, "Verified", &format!("the passes kept: {verified}")),
            ([], unverified) => say::status(
                quiet,
                "Verified",
                &format!(
                    "the passes kept: {verified}; {unverified} not told, their run ended without a summary"
                ),
            ),
            (wrong, _) => say::error(&format!(
                "{} kept as passed failed now, so each is flaky or its key misses what it is a \
                 function of (a bug in blaze): {}",
                counted(wrong.len()),
                wrong.join(", ")
            )),
        }
    }
}

/// One run of a test executable: in the sandbox or outside it, with the arguments that pick
/// its tests, and the tests its package took out of the sandbox it runs.
struct Part {
    sandboxed: bool,
    args: Vec<String>,
    outside: Vec<String>,
}

/// The runs of `test`: one, in the sandbox when `sandboxed`, unless its package takes some of
/// its tests out (`outside`), which then run outside after the others ran inside. `picked`
/// are the tests [`affected`] picked; `bare` makes the command that runs it outside.
fn parts(
    asked: &Invocation,
    test: &Program,
    sandboxed: bool,
    outside: &BTreeSet<String>,
    picked: Option<&[String]>,
    bare: &dyn Fn() -> anyhow::Result<Command>,
    running: &str,
) -> anyhow::Result<Vec<Part>> {
    let asked_args: Vec<String> = asked.program_args().cloned().collect();
    let with = |more: &mut dyn Iterator<Item = String>| {
        let mut args = asked_args.clone();
        args.extend(more);
        args
    };
    let exact = |names: &[String]| {
        with(&mut std::iter::once("--exact".to_owned()).chain(names.iter().cloned()))
    };
    let one = |sandboxed| {
        let args = match picked {
            Some(names) => exact(names),
            None => asked_args.clone(),
        };
        vec![Part { sandboxed, args, outside: Vec::new() }]
    };
    if !sandboxed || outside.is_empty() {
        return Ok(one(sandboxed));
    }
    // The tests outside, of those that run: libtest's `--exact` names them, and `--skip`
    // with it leaves them out of the run inside.
    let (inside, out): (Vec<String>, Vec<String>) = match picked {
        Some(names) => names.iter().cloned().partition(|it| !outside.contains(it)),
        None if asked.positional.is_empty() && affected::plain(&asked.trailing) => {
            let out: Vec<String> = listed(bare, &test.executable)?
                .into_iter()
                .filter(|it| outside.contains(it))
                .collect();
            if out.is_empty() {
                return Ok(one(true));
            }
            let skips = out.iter().flat_map(|it| ["--skip".to_owned(), it.clone()]);
            let args = with(&mut std::iter::once("--exact".to_owned()).chain(skips));
            let outside_args = exact(&out);
            return Ok(vec![
                Part { sandboxed: true, args, outside: Vec::new() },
                Part { sandboxed: false, args: outside_args, outside: out },
            ]);
        }
        None => {
            say::status(
                asked.flags.quiet,
                "Unsandboxed",
                &format!(
                    "{running}: its package takes tests of it out of the sandbox, and the \
                     command picks the tests itself, so all of it runs outside"
                ),
            );
            return Ok(one(false));
        }
    };
    let mut parts = Vec::new();
    if !inside.is_empty() {
        parts.push(Part { sandboxed: true, args: exact(&inside), outside: Vec::new() });
    }
    if !out.is_empty() {
        parts.push(Part { sandboxed: false, args: exact(&out), outside: out });
    }
    Ok(parts)
}

/// The names of the tests of `executable`, by libtest's `--list`, which runs none of them.
fn listed(
    bare: &dyn Fn() -> anyhow::Result<Command>,
    executable: &Path,
) -> anyhow::Result<Vec<String>> {
    let mut command = bare()?;
    command.args(["--list", "--format", "terse"]).stdout(std::process::Stdio::piped());
    let out = started_running(&mut command, executable)
        .and_then(|it| it.wait_with_output())
        .with_context(|| format!("listing the tests of {}", executable.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .lines()
        .filter_map(|it| it.strip_suffix(": test").or_else(|| it.strip_suffix(": benchmark")))
        .map(str::to_owned)
        .collect())
}

fn counted(count: usize) -> String {
    match count {
        1 => "1 test".to_owned(),
        _ => format!("{count} tests"),
    }
}

/// The flags that have cargo run this test target alone.
fn rerun_flags(test: &Program) -> String {
    let target = match test.kind {
        Kind::Lib => "--lib".to_owned(),
        Kind::Bin => format!("--bin {}", test.name),
        Kind::Example => format!("--example {}", test.name),
        Kind::Test => format!("--test {}", test.name),
        Kind::Bench => format!("--bench {}", test.name),
    };
    format!("-p {} {target}", test.package)
}

/// The path as cargo shows it: from the directory the command runs in, when it is below.
fn shown_path(path: &Path, cwd: &Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

/// What this command ends with after a process that ended with `status`: its code, or
/// the code a shell gives one a signal ended.
pub(crate) fn exit_code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return 128 + signal;
    }
    status.code().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::failures;

    #[test]
    fn the_summary_names_the_failures() {
        let pretty = "\nrunning 3 tests\ntest a ... ok\ntest b ... FAILED\ntest c::d ... FAILED\n\n\
            failures:\n\n---- b stdout ----\nfailures:\n    not_a_test\n\n---- c::d stdout ----\n\n\
            failures:\n    b\n    c::d\n\ntest result: FAILED. 1 passed; 2 failed; 0 ignored\n\n";
        assert_eq!(failures(pretty), Some(vec!["b".to_owned(), "c::d".to_owned()]));
        let colored =
            "\nfailures:\n    b\n\ntest result: \x1b[31mFAILED\x1b[0m. 0 passed; 1 failed\n";
        assert_eq!(failures(colored), Some(vec!["b".to_owned()]));
        assert_eq!(
            failures("\nrunning 1 test\ntest a ... ok\n\ntest result: ok. 1 passed\n"),
            None
        );
        // A program that stopped in a test has no summary.
        assert_eq!(failures("\nrunning 2 tests\ntest a ... ok\n"), None);
    }
}
