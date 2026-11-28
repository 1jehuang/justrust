#![feature(rustc_private)]
#![warn(
    // warn on lints, that are included in `rust-lang/rust`s bootstrap
    rust_2018_idioms, unused_lifetimes,
    // and on rustc internal lints
    rustc::internal)]

// FIXME: switch to something more ergonomic here, once available.
// (Currently there is no way to opt into sysroot crates without `extern crate`.)
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_session;
extern crate rustc_span;

// Override the C allocator in the same way that the `rustc` binary would do.
rustc_driver::override_c_allocator_in_binary!();

use clippy_utils::sym;
use declare_clippy_lint::LintListBuilder;
use rustc_interface::interface;
use rustc_session::config::ErrorOutputType;
use rustc_session::{EarlyDiagCtxt, Session};
use rustc_span::symbol::Symbol;

use std::env;
use std::fs::read_to_string;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

/// justrust: whether this driver runs from a justrust toolchain inside a justrust build.
///
/// Cargo runs workspace units as `clippy-driver $RUSTC <args>` and the driver compiles in
/// process, so the justrust `rustc` shim never sees them. What the shim does for ordinary
/// units (parallel front end, phase ledger) is therefore done here, under the same condition.
fn jr_toolchain(argv0: &str) -> bool {
    fn in_jr_sysroot(exe: &Path) -> bool {
        exe.parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|name| name.to_string_lossy().starts_with("jr-"))
    }
    env::var_os("JUSTRUST_TOOLCHAIN_JR").is_some()
        && (in_jr_sysroot(Path::new(argv0)) || env::current_exe().is_ok_and(|exe| in_jr_sysroot(&exe)))
}

/// justrust: the front-end thread count for a workspace unit, or `None` to leave it alone.
///
/// The same rule as the shim: `setting` is `JUSTRUST_FRONTEND_THREADS` (`0` or an invalid value
/// turns it off), the default is half the CPUs up to 16, registry crates are skipped, and a unit
/// that already picks a thread count keeps it. `-Zthreads` is not part of any cache key.
fn jr_frontend_threads(
    args: &[String],
    setting: Option<&str>,
    cpus: usize,
    manifest_dir: Option<&Path>,
    cargo_home: Option<&Path>,
) -> Option<usize> {
    let threads = match setting {
        Some(value) => value.parse::<usize>().ok()?,
        None => (cpus / 2).clamp(1, 16),
    };
    let manifest_dir = manifest_dir?;
    if threads < 2 || cargo_home.is_some_and(|home| manifest_dir.starts_with(home)) {
        return None;
    }
    let mut after_z = false;
    for arg in args {
        // An argument file can carry a thread count: defer instead of parsing it differently.
        if arg.starts_with("-Zthreads")
            || arg.starts_with("--jobs-frontend")
            || arg.starts_with('@')
            || (after_z && arg.starts_with("threads"))
        {
            return None;
        }
        after_z = arg == "-Z";
    }
    Some(threads)
}

/// justrust: where this unit's phase ledger goes, `<run dir>/phases/<pid>.tsv`, which is where
/// the build recorder looks for it. `JUSTRUST_PASSES=0` turns it off.
fn jr_phase_ledger_path(run_dir: Option<&Path>, passes: Option<&str>, pid: u32) -> Option<PathBuf> {
    if matches!(passes, Some("0" | "off" | "false" | "no")) {
        return None;
    }
    Some(run_dir?.join("phases").join(format!("{pid}.tsv")))
}

/// justrust: apply the parallel front end and the phase ledger to a wrapper-mode unit.
///
/// Neither is visible to Cargo: the thread count is appended in process and the ledger is an
/// environment variable, so fingerprints, cache keys and incremental state are unchanged.
fn jr_prepare_unit(args: &mut Vec<String>) {
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")));
    let threads = env::var_os("CARGO_CRATE_NAME").and_then(|_| {
        jr_frontend_threads(
            args,
            env::var("JUSTRUST_FRONTEND_THREADS")
                .or_else(|_| env::var("JUSTRUST_RELEASE_FRONTEND_THREADS"))
                .ok()
                .as_deref(),
            std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
            env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).as_deref(),
            cargo_home.as_deref(),
        )
    });
    if let Some(threads) = threads {
        args.push(format!("-Zthreads={threads}"));
    }

    let ledger = jr_phase_ledger_path(
        env::var_os("JUSTRUST_RUN_DIR").map(PathBuf::from).as_deref(),
        env::var("JUSTRUST_PASSES").ok().as_deref(),
        std::process::id(),
    )
    .filter(|path| path.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok()));
    // SAFETY: the driver is still single threaded here, before the compiler session starts.
    unsafe {
        match &ledger {
            Some(path) => env::set_var("JUSTRUST_RUSTC_PHASES_FILE", path),
            // Never inherited: a nested build must not write another unit's file.
            None => env::remove_var("JUSTRUST_RUSTC_PHASES_FILE"),
        }
    }
    // The recorder only sees the command line Cargo spawned, which has no thread count.
    if let Some(path) = ledger
        && let Some(threads) = threads
    {
        let _ = std::fs::write(path.with_extension("driver"), format!("frontend_threads\t{threads}\n"));
    }
}

#[test]
fn test_jr_frontend_threads() {
    let args = |extra: &[&str]| -> Vec<String> {
        ["--crate-name", "foo", "src/lib.rs"]
            .iter()
            .chain(extra)
            .map(ToString::to_string)
            .collect()
    };
    let local = Some(Path::new("/work/foo"));
    let home = Some(Path::new("/home/u/.cargo"));
    let threads = |extra: &[&str], setting, cpus| jr_frontend_threads(&args(extra), setting, cpus, local, home);

    assert_eq!(threads(&[], None, 96), Some(16));
    assert_eq!(threads(&[], None, 8), Some(4));
    assert_eq!(threads(&[], None, 2), None);
    assert_eq!(threads(&[], Some("6"), 96), Some(6));
    assert_eq!(threads(&[], Some("0"), 96), None);
    assert_eq!(threads(&[], Some("many"), 96), None);
    for explicit in [
        &["-Zthreads=2"][..],
        &["-Z", "threads=2"],
        &["--jobs-frontend", "2"],
        &["--jobs-frontend=2"],
        &["@args"],
    ] {
        assert_eq!(threads(explicit, None, 96), None, "{explicit:?}");
    }
    assert_eq!(threads(&["-Z", "time-passes"], None, 96), Some(16));
    let registry = Some(Path::new("/home/u/.cargo/registry/src/index/foo-1.0.0"));
    assert_eq!(jr_frontend_threads(&args(&[]), None, 96, registry, home), None);
    assert_eq!(jr_frontend_threads(&args(&[]), None, 96, None, home), None);
}

#[test]
fn test_jr_phase_ledger_path() {
    let run = Some(Path::new("/runs/1"));
    assert_eq!(
        jr_phase_ledger_path(run, None, 42),
        Some(PathBuf::from("/runs/1/phases/42.tsv"))
    );
    assert_eq!(jr_phase_ledger_path(run, Some("0"), 42), None);
    assert_eq!(jr_phase_ledger_path(None, None, 42), None);
}

/// If a command-line option matches `find_arg`, then apply the predicate `pred` on its value. If
/// true, then return it. The parameter is assumed to be either `--arg=value` or `--arg value`.
fn arg_value(args: &[String], find_arg: &str, pred: impl Fn(&str) -> bool) -> bool {
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        let mut arg = arg.splitn(2, '=');
        if arg.next() != Some(find_arg) {
            continue;
        }

        match arg.next().or_else(|| args.next()) {
            Some(v) if pred(v) => return true,
            _ => {},
        }
    }
    false
}

/// Returns true if this is `arg` or `arg=...`, otherwise returns false.
fn is_arg(argument: &str, key: &str) -> bool {
    argument
        .strip_prefix(key)
        .is_some_and(|s| s.is_empty() || s.starts_with('='))
}

#[test]
fn test_arg_value() {
    use std::ops::Not as _;

    let args = &["--bar=bar", "--foobar", "123", "--foo"].map(String::from);

    assert!(arg_value(&[], "--foobar", |_| true).not());
    assert!(arg_value(args, "--bar", |_| false).not());
    assert!(arg_value(args, "--bar", |_| true));
    assert!(arg_value(args, "--bar", |p| p == "bar"));
    assert!(arg_value(args, "--bar", |p| p == "foo").not());
    assert!(arg_value(args, "--foobar", |p| p == "foo").not());
    assert!(arg_value(args, "--foobar", |p| p == "123"));
    assert!(arg_value(args, "--foobar", |p| p.contains("12")));
    assert!(arg_value(args, "--foo", |_| true).not());
}

#[test]
fn test_has_arg() {
    assert!(is_arg("--foo=bar", "--foo"));
    assert!(is_arg("--baz", "--baz"));
    assert!(is_arg("-vV", "-vV"));

    assert!(
        ["--=bar", "--barr", "-bar", "bar"]
            .iter()
            .all(|arg| !is_arg(arg, "--bar"))
    );
}

fn track_clippy_args(sess: &Session, args_env_var: Option<&str>) {
    sess.env_depinfo
        .borrow_mut()
        .insert((sym::CLIPPY_ARGS, args_env_var.map(Symbol::intern)));
}

/// Track files that may be accessed at runtime in `file_depinfo` so that cargo will re-run clippy
/// when any of them are modified
fn track_files(sess: &Session) {
    let mut file_depinfo = sess.file_depinfo.borrow_mut();

    // Used by `clippy::cargo` lints and to determine the MSRV. `cargo clippy` executes `clippy-driver`
    // with the current directory set to `CARGO_MANIFEST_DIR` so a relative path is fine
    if Path::new("Cargo.toml").exists() {
        file_depinfo.insert(sym::Cargo_toml);
    }

    // `clippy.toml` will be automatically tracked as it's loaded with `sess.source_map().load_file()`

    // During development track the `clippy-driver` executable so that cargo will re-run clippy whenever
    // it is rebuilt
    if cfg!(debug_assertions)
        && let Ok(current_exe) = env::current_exe()
        && let Some(current_exe) = current_exe.to_str()
    {
        file_depinfo.insert(Symbol::intern(current_exe));
    }
}

struct DefaultCallbacks;
impl rustc_driver::Callbacks for DefaultCallbacks {}

/// This is different from `DefaultCallbacks` that it will inform Cargo to track the value of
/// `CLIPPY_ARGS` environment variable.
struct RustcCallbacks {
    clippy_args_var: Option<String>,
}

impl rustc_driver::Callbacks for RustcCallbacks {
    fn config(&mut self, config: &mut interface::Config) {
        let clippy_args_var = self.clippy_args_var.take();
        config.track_state = Some(Box::new(move |sess| {
            track_clippy_args(sess, clippy_args_var.as_deref());
        }));
        config.extra_symbols = sym::EXTRA_SYMBOLS.into();
    }
}

struct ClippyCallbacks {
    clippy_args_var: Option<String>,
}

impl rustc_driver::Callbacks for ClippyCallbacks {
    #[expect(rustc::bad_opt_access, reason = "necessary in clippy driver to set `mir_opt_level`")]
    fn config(&mut self, config: &mut interface::Config) {
        let module_local = env::var_os("JUSTRUST_CLIPPY_MODULE_LINTS").as_deref()
            != Some(std::ffi::OsStr::new("0"));
        if !module_local {
            // Keep the opt-out crate-wide partition in its own incremental namespace
            // so switching JUSTRUST_CLIPPY_MODULE_LINTS=0 cannot reuse green lint_mod
            // queries from the default module-local partition.
            config.opts.incremental = config
                .opts
                .incremental
                .take()
                .map(|path| path.join("jr-clippy-crate-lints-v1"));
        }
        let previous = config.register_lints.take();
        let clippy_args_var = self.clippy_args_var.take();
        config.track_state = Some(Box::new(move |sess| {
            track_clippy_args(sess, clippy_args_var.as_deref());
            track_files(sess);

            // Trigger a rebuild if CLIPPY_CONF_DIR changes. The value must be a valid string so
            // changes between dirs that are invalid UTF-8 will not trigger rebuilds
            sess.env_depinfo.borrow_mut().insert((
                sym::CLIPPY_CONF_DIR,
                env::var("CLIPPY_CONF_DIR").ok().map(|dir| Symbol::intern(&dir)),
            ));
        }));
        config.register_lints = Some(Box::new(move |sess, lint_store| {
            // technically we're ~guaranteed that this is none but might as well call anything that
            // is there already. Certainly it can't hurt.
            if let Some(previous) = &previous {
                (previous)(sess, lint_store);
            }

            let mut list_builder = LintListBuilder::default();
            list_builder.insert(clippy_lints::declared_lints::LINTS);
            list_builder.register(lint_store);

            let conf = clippy_config::Conf::load(sess);
            clippy_lints::register_lint_passes(lint_store, conf, module_local);

            #[cfg(feature = "internal")]
            clippy_lints_internal::register_lints(lint_store);
        }));
        config.extra_symbols = sym::EXTRA_SYMBOLS.into();

        // FIXME: #4825; This is required, because Clippy lints that are based on MIR have to be
        // run on the unoptimized MIR. On the other hand this results in some false negatives. If
        // MIR passes can be enabled / disabled separately, we should figure out, what passes to
        // use for Clippy.
        config.opts.unstable_opts.mir_opt_level = Some(0);
        config.opts.unstable_opts.mir_enable_passes =
            vec![("CheckNull".to_owned(), false), ("CheckAlignment".to_owned(), false)];

        // Disable flattening and inlining of format_args!(), so the HIR matches with the AST.
        config.opts.unstable_opts.flatten_format_args = false;
    }
}

fn display_help() -> ExitCode {
    if writeln!(&mut anstream::stdout().lock(), "{}", help_message()).is_err() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

const BUG_REPORT_URL: &str = "https://github.com/rust-lang/rust-clippy/issues/new?template=ice.yml";

fn main() -> ExitCode {
    // justrust: the phase ledger's total covers the whole driver, as in `rustc`.
    let start_time = Instant::now();
    rustc_data_structures::profiling::jr_phases_begin(start_time);

    let early_dcx = EarlyDiagCtxt::new(ErrorOutputType::default());

    rustc_driver::init_rustc_env_logger(&early_dcx);

    rustc_driver::install_ice_hook(BUG_REPORT_URL, |dcx| {
        // FIXME: this macro calls unwrap internally but is called in a panicking context!  It's not
        // as simple as moving the call from the hook to main, because `install_ice_hook` doesn't
        // accept a generic closure.
        let version_info = rustc_tools_util::get_version_info!();
        dcx.note(format!("Clippy version: {version_info}"));
    });

    let exit_code = rustc_driver::catch_with_exit_code(move || {
        fn has_sysroot_arg(args: &[String]) -> bool {
            // https://doc.rust-lang.org/rustc/command-line-arguments.html#path-load-command-line-flags-from-a-path
            // Beside checking for existence of `--sysroot` on the command line, we need to
            // check for the arg files that are prefixed with @ as well to be consistent with rustc
            args.iter().any(|arg| {
                if let Some(arg_file_path) = arg.strip_prefix('@')
                    && let Ok(arg_file) = read_to_string(arg_file_path)
                {
                    arg_file.lines().any(|arg| is_arg(arg, "--sysroot"))
                } else {
                    is_arg(arg, "--sysroot")
                }
            })
        }

        let mut orig_args = rustc_driver::args::raw_args(&early_dcx);

        let sys_root_env = std::env::var("SYSROOT").ok();
        let pass_sysroot_env_if_given = |args: &mut Vec<String>, sys_root_env| {
            if let Some(sys_root) = sys_root_env
                && !has_sysroot_arg(args)
            {
                args.extend(["--sysroot".to_string(), sys_root]);
            }
        };

        // make "clippy-driver --rustc" work like a subcommand that passes further args to "rustc"
        // for example `clippy-driver --rustc --version` will print the rustc version that clippy-driver
        // uses
        if let Some(pos) = orig_args.iter().position(|arg| arg == "--rustc") {
            orig_args.remove(pos);
            orig_args[0] = "rustc".to_string();

            let mut args: Vec<String> = orig_args.clone();
            pass_sysroot_env_if_given(&mut args, sys_root_env);

            rustc_driver::compiler_entrypoint(&args, &mut DefaultCallbacks);
            return ExitCode::SUCCESS;
        }

        if orig_args.iter().any(|a| a == "--version" || a == "-V") {
            let version_info = rustc_tools_util::get_version_info!();

            return match writeln!(&mut anstream::stdout().lock(), "{version_info}") {
                Ok(()) => ExitCode::SUCCESS,
                Err(_) => ExitCode::FAILURE,
            };
        }

        // Setting RUSTC_WRAPPER causes Cargo to pass 'rustc' as the first argument.
        // We're invoking the compiler programmatically, so we ignore this/
        let wrapper_mode = orig_args.get(1).map(Path::new).and_then(Path::file_stem) == Some("rustc".as_ref());

        if wrapper_mode {
            // we still want to be able to invoke it normally though
            orig_args.remove(1);
        }

        if !wrapper_mode && (orig_args.iter().any(|a| a == "--help" || a == "-h") || orig_args.len() == 1) {
            return display_help();
        }

        let mut args: Vec<String> = orig_args.clone();
        pass_sysroot_env_if_given(&mut args, sys_root_env);

        let mut no_deps = false;
        let clippy_args_var = env::var("CLIPPY_ARGS").ok();
        let clippy_args = clippy_args_var
            .as_deref()
            .unwrap_or_default()
            .split("__CLIPPY_HACKERY__")
            .filter_map(|s| match s {
                "" => None,
                "--no-deps" => {
                    no_deps = true;
                    None
                },
                _ => Some(s.to_string()),
            })
            .chain(vec!["--cfg".into(), "clippy".into()])
            .collect::<Vec<String>>();

        // If no Clippy lints will be run we do not need to run Clippy
        let cap_lints_allow = arg_value(&orig_args, "--cap-lints", |val| val == "allow")
            && !arg_value(&orig_args, "--force-warn", |val| val.contains("clippy::"));

        // If `--no-deps` is enabled only lint the primary package
        let relevant_package = !no_deps || env::var("CARGO_PRIMARY_PACKAGE").is_ok();

        // Do not run Clippy for Cargo's info queries so that invalid CLIPPY_ARGS are not cached
        // https://github.com/rust-lang/cargo/issues/14385
        let info_query = args.iter().any(|arg| arg == "-vV")
            || arg_value(&orig_args, "--print", |val| val != "crate-root-lint-levels");

        let clippy_enabled = !cap_lints_allow && relevant_package && !info_query;
        if clippy_enabled {
            args.extend(clippy_args);
        }
        // justrust: after `CLIPPY_ARGS`, so a thread count given there wins.
        if wrapper_mode && !info_query && jr_toolchain(&orig_args[0]) {
            jr_prepare_unit(&mut args);
        }
        if clippy_enabled {
            rustc_driver::compiler_entrypoint(&args, &mut ClippyCallbacks { clippy_args_var });
        } else {
            rustc_driver::compiler_entrypoint(&args, &mut RustcCallbacks { clippy_args_var });
        }
        ExitCode::SUCCESS
    });

    // justrust: per-phase totals for the build recorder. A no-op without a ledger.
    rustc_data_structures::profiling::jr_phases_write(start_time.elapsed());

    exit_code
}

#[must_use]
fn help_message() -> &'static str {
    color_print::cstr!(
        "Checks a file to catch common mistakes and improve your Rust code.
Run <cyan>clippy-driver</> with the same arguments you use for <cyan>rustc</>

<green,bold>Usage</>:
    <cyan,bold>clippy-driver</> <cyan>[OPTIONS] INPUT</>

<green,bold>Common options:</>
    <cyan,bold>-h</>, <cyan,bold>--help</>               Print this message
    <cyan,bold>-V</>, <cyan,bold>--version</>            Print version info and exit
    <cyan,bold>--rustc</>                  Pass all arguments to <cyan>rustc</>

<green,bold>Allowing / Denying lints</>
You can use tool lints to allow or deny lints from your code, e.g.:

    <yellow,bold>#[allow(clippy::needless_lifetimes)]</>
"
    )
}
