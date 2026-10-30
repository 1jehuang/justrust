//! cargo's answers to what a workspace load asks it of the toolchain and the configuration,
//! given by ra-jit with no cargo run: `cargo --version`, `cargo -Z unstable-options config get`
//! (blaze-manifest's port of it) and `cargo rustc -Z unstable-options --print <what>` (the
//! flags blaze-manifest says cargo gives rustc, and rustc's answer to them from
//! [`super::answer`]). rust-analyzer asks these through [`super::ask`]; the plan asks
//! [`rustflags`]. A question of this kind that ra-jit cannot answer fails, as cargo would, and
//! cargo is never run for it (docs/no-rustc.md, step K).

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output},
};

use anyhow::Context as _;
use blaze_manifest::config::CompileKind;
use blaze_manifest::{Config, Context, Workspace};

/// What cargo is asked.
#[derive(Debug, PartialEq, Eq)]
enum Question {
    /// `cargo --version`.
    Version,
    /// `cargo config get`, by the arguments after `cargo`.
    ConfigGet(Vec<String>),
    /// `cargo rustc --print <what> [--target <t>]... -- <rustc_args>`.
    Print { what: String, targets: Vec<String>, rustc_args: Vec<String> },
}

/// The question cargo run with `args` asks, `None` for a command that is not one of these.
/// `Err` is a question of this kind with an argument ra-jit does not read.
fn question(args: &[String]) -> Option<Result<Question, String>> {
    if matches!(args, [one] if one == "--version" || one == "-V") {
        return Some(Ok(Question::Version));
    }
    // The `-Z` flags before the command, joined as `-Zflag`.
    let mut global = Vec::new();
    let mut words = args.iter();
    let command = loop {
        match words.next()?.as_str() {
            "-Z" => global.push(format!("-Z{}", words.next()?)),
            flag if flag.starts_with("-Z") => global.push(flag.to_owned()),
            command => break command,
        }
    };
    let rest: Vec<&String> = words.collect();
    match command {
        "config" => {
            let mut asked = global;
            asked.push("config".to_owned());
            let mut rest = rest.into_iter();
            while let Some(word) = rest.next() {
                match word.as_str() {
                    "-Z" => asked.push(format!("-Z{}", rest.next()?)),
                    _ => asked.push(word.clone()),
                }
            }
            let unstable = |it: &String| it == "-Zunstable-options";
            if let Some(other) = asked.iter().find(|it| it.starts_with("-Z") && !unstable(it)) {
                return Some(Err(format!("blaze answers `cargo config` without `{other}`")));
            }
            Some(Ok(Question::ConfigGet(asked)))
        }
        "rustc" if rest.iter().any(|it| *it == "--print" || it.starts_with("--print=")) => {
            Some(print(&global, &rest))
        }
        _ => None,
    }
}

fn print(global: &[String], rest: &[&String]) -> Result<Question, String> {
    let (mut what, mut targets, mut rustc_args) = (None, Vec::new(), Vec::new());
    let mut unstable: Vec<String> = global.to_vec();
    let mut words = rest.iter().map(|it| it.as_str());
    while let Some(word) = words.next() {
        let (flag, joined) = match word.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
            _ => (word, None),
        };
        let mut value = || {
            joined
                .or_else(|| words.next())
                .map(str::to_owned)
                .ok_or(format!("`{flag}` has no value"))
        };
        match flag {
            "--" => {
                rustc_args.extend(words.by_ref().map(str::to_owned));
            }
            "-Z" => unstable.push(format!("-Z{}", value()?)),
            _ if flag.starts_with("-Z") => unstable.push(flag.to_owned()),
            "--print" => what = Some(value()?),
            "--target" => targets.push(value()?),
            other => return Err(format!("blaze answers `cargo rustc --print` without `{other}`")),
        }
    }
    if let Some(other) = unstable.iter().find(|it| *it != "-Zunstable-options") {
        return Err(format!("blaze answers `cargo rustc --print` without `{other}`"));
    }
    let what = what.ok_or("`cargo rustc` asked to print nothing")?;
    Ok(Question::Print { what, targets, rustc_args })
}

/// What cargo run as `command` prints of what it is asked, with ra-jit's answers, or `None`
/// when it is no question of these. A failure is cargo's: status 101 and the error.
pub(crate) fn answer(command: &Command) -> Option<io::Result<Output>> {
    let args: Option<Vec<String>> =
        command.get_args().map(|it| it.to_str().map(str::to_owned)).collect();
    let asked = match question(&args?)? {
        Ok(asked) => asked,
        Err(why) => return Some(Ok(failed(why))),
    };
    let asker = Asker::of(command);
    let said = match asked {
        Question::Version => asker.version(),
        Question::ConfigGet(args) => {
            let (code, output) = blaze_manifest::config::get::main(&asker.cwd, asker.vars(), &args);
            return Some(Ok(Output {
                status: status(i32::from(code)),
                stdout: output.stdout.into_bytes(),
                stderr: output.stderr.into_bytes(),
            }));
        }
        Question::Print { what, targets, rustc_args } => asker.print(&what, &targets, &rustc_args),
    };
    Some(Ok(match said {
        Ok(stdout) => Output { status: status(0), stdout: stdout.into_bytes(), stderr: Vec::new() },
        Err(err) => failed(blaze_manifest::diagnostic::report(&err, &asker.cwd)),
    }))
}

/// The flags cargo gives every rustc of the units for the target `triple` of a build asked by
/// `command` (a cargo, of whoever asks, in the directory of the build), as cargo's
/// `TargetInfo` has them: `CARGO_ENCODED_RUSTFLAGS`, `RUSTFLAGS`, the `[target]` tables that
/// match the triple and its `cfg` under those flags, or `build.rustflags`.
pub(crate) fn rustflags(command: &Command, triple: &str) -> anyhow::Result<Vec<String>> {
    let asker = Asker::of(command);
    let config = asker.config()?;
    let kind = CompileKind::Target(triple.to_owned());
    let mut rustc = asker.rustc(&config)?;
    let host = host(&mut rustc)?;
    let (flags, _) =
        config.target_rustflags(std::slice::from_ref(&kind), &host, &kind, &mut rustc)?;
    Ok(flags)
}

fn host(rustc: &mut dyn FnMut(&[String]) -> anyhow::Result<String>) -> anyhow::Result<String> {
    let verbose = rustc(&["-vV".to_owned()])?;
    let host = verbose.lines().find_map(|it| it.strip_prefix("host: "));
    Ok(host.context("`rustc -vV` names no host")?.to_owned())
}

/// The cargo that would have been run: where, with which variables, and which program.
struct Asker {
    cwd: PathBuf,
    program: OsString,
    /// The variables the command sets (`Some`) or removes (`None`).
    set: Vec<(OsString, Option<OsString>)>,
}

impl Asker {
    fn of(command: &Command) -> Asker {
        let cwd = command.get_current_dir().map(Path::to_owned);
        Asker {
            cwd: cwd.or_else(|| std::env::current_dir().ok()).unwrap_or_default(),
            program: command.get_program().to_owned(),
            set: command.get_envs().map(|(k, v)| (k.to_owned(), v.map(OsStr::to_owned))).collect(),
        }
    }

    fn var(&self, name: &str) -> Option<OsString> {
        match self.set.iter().rev().find(|(it, _)| it == name) {
            Some((_, value)) => value.clone(),
            None => std::env::var_os(name),
        }
    }

    /// The variables cargo starts with.
    fn vars(&self) -> BTreeMap<String, String> {
        let mut vars: BTreeMap<String, String> = std::env::vars().collect();
        for (key, value) in &self.set {
            let Some(key) = key.to_str() else { continue };
            match value.as_ref().and_then(|it| it.to_str()) {
                Some(value) => vars.insert(key.to_owned(), value.to_owned()),
                None => vars.remove(key),
            };
        }
        vars
    }

    fn config(&self) -> anyhow::Result<Config> {
        Config::load(&self.cwd, self.vars(), &[])
    }

    /// The toolchain directory of the cargo run. rustup's proxy runs the toolchain it is told
    /// (`RUSTUP_TOOLCHAIN`, as cargo run through it has it); told none, it is blaze's release,
    /// which blaze builds with whatever the project names (cargo/toolchain.rs, #269), from
    /// rustup's copy of it. Neither found is an error, and nothing is run to find it.
    fn sysroot(&self) -> anyhow::Result<PathBuf> {
        let var = |name: &str| self.var(name);
        let found = super::toolchain_rustc(&self.program, &var);
        let sysroot = found.as_deref().and_then(Path::parent).and_then(Path::parent);
        let sysroot = sysroot.map(Path::to_owned).or_else(crate::cargo::toolchain::rustup_ours);
        sysroot.with_context(|| {
            format!(
                "blaze can't tell which toolchain `{}` runs, and rustup has no copy of Rust {}: \
                 name the toolchain's own cargo (`CARGO`), or set `RUSTUP_TOOLCHAIN` for \
                 rustup's proxy",
                Path::new(&self.program).display(),
                crate::cargo::toolchain::OURS.trim()
            )
        })
    }

    /// `cargo --version`: the release of the toolchain's rustc, with cargo's commit, from
    /// rustup's record of the toolchain.
    fn version(&self) -> anyhow::Result<String> {
        let sysroot = self.sysroot()?;
        let version = super::own::cargo_version(&sysroot);
        let version = version.with_context(|| {
            format!("blaze can't tell the release of cargo in {}", sysroot.display())
        })?;
        Ok(format!("cargo {version}\n"))
    }

    /// rustc as cargo runs it to ask: the program cargo picks, the variables of the command
    /// and `[env]`, answered by [`super::answer`].
    fn rustc(
        &self,
        config: &Config,
    ) -> anyhow::Result<impl FnMut(&[String]) -> anyhow::Result<String> + use<>> {
        let program = match config.rustc_program()? {
            Some(program) => program,
            None => {
                self.sysroot()?.join("bin").join(format!("rustc{}", std::env::consts::EXE_SUFFIX))
            }
        };
        let mut base = Command::new(&program);
        base.current_dir(&self.cwd);
        for (key, value) in &self.set {
            match value {
                Some(value) => base.env(key, value),
                None => base.env_remove(key),
            };
        }
        for (key, value) in config.env_config()? {
            base.env(key, value);
        }
        let envs: Vec<(OsString, Option<OsString>)> =
            base.get_envs().map(|(k, v)| (k.to_owned(), v.map(OsStr::to_owned))).collect();
        let cwd = self.cwd.clone();
        Ok(move |args: &[String]| {
            let mut command = Command::new(&program);
            command.current_dir(&cwd).args(args);
            for (key, value) in &envs {
                match value {
                    Some(value) => command.env(key, value),
                    None => command.env_remove(key),
                };
            }
            let output = super::answer(&mut command, &[])
                .with_context(|| format!("could not execute process `{}`", program.display()))?;
            anyhow::ensure!(
                output.status.success(),
                "process didn't exit successfully: `{} {}` ({})\n--- stderr\n{}",
                program.display(),
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim_end()
            );
            String::from_utf8(output.stdout).context("rustc's answer is not UTF-8")
        })
    }

    /// `cargo rustc --print`: the workspace read first, as cargo reads it for the command.
    fn print(
        &self,
        what: &str,
        targets: &[String],
        rustc_args: &[String],
    ) -> anyhow::Result<String> {
        let mut config = self.config()?;
        config.start("rustc", &[])?;
        let ctx = Context::new(config);
        let root = blaze_manifest::metadata::root_manifest(None, &ctx)?;
        Workspace::new(&root, &ctx)?;
        let mut rustc = self.rustc(&ctx.config)?;
        ctx.config.rustc_print(targets, what, rustc_args, &mut rustc)
    }
}

fn failed(error: String) -> Output {
    let error = if error.starts_with("error") { error } else { format!("error: {error}") };
    Output { status: status(101), stdout: Vec::new(), stderr: format!("{error}\n").into_bytes() }
}

fn status(code: i32) -> ExitStatus {
    #[cfg(unix)]
    {
        std::os::unix::process::ExitStatusExt::from_raw(code << 8)
    }
    #[cfg(windows)]
    {
        std::os::windows::process::ExitStatusExt::from_raw(code as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asked(line: &str) -> Option<Result<Question, String>> {
        let args: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        question(&args)
    }

    #[test]
    fn questions_read() {
        assert_eq!(asked("--version"), Some(Ok(Question::Version)));
        assert_eq!(
            asked("-Z unstable-options config get --format toml --show-origin"),
            Some(Ok(Question::ConfigGet(
                ["-Zunstable-options", "config", "get", "--format", "toml", "--show-origin"]
                    .map(str::to_owned)
                    .to_vec()
            )))
        );
        assert!(matches!(asked("-Z unstable-options config get -Zscript"), Some(Err(_))));
        assert_eq!(
            asked("rustc -Z unstable-options --print cfg --target x86_64-unknown-linux-gnu -- -O"),
            Some(Ok(Question::Print {
                what: "cfg".to_owned(),
                targets: vec!["x86_64-unknown-linux-gnu".to_owned()],
                rustc_args: vec!["-O".to_owned()],
            }))
        );
        assert_eq!(
            asked("rustc -Z unstable-options --print=target-spec-json -- -Z unstable-options"),
            Some(Ok(Question::Print {
                what: "target-spec-json".to_owned(),
                targets: vec![],
                rustc_args: vec!["-Z".to_owned(), "unstable-options".to_owned()],
            }))
        );
        assert!(matches!(asked("rustc --print cfg -p foo"), Some(Err(_))));
        // Not questions of these: another lane's, or cargo's to run.
        assert_eq!(asked("metadata --format-version 1"), None);
        assert_eq!(asked("rustc -- -O"), None);
        assert_eq!(asked("build --unit-graph -Z unstable-options"), None);
    }
}
