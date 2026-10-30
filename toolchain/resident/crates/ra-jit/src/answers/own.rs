//! ra-jit's own answers to rustc's questions, from what it knows of a release of rustc and of
//! the targets that release has: no rustc runs. The data of each release is what that rustc
//! printed for every target ra-jit has (`harness/answers.py --write`), and the cfgs that follow
//! from the arguments are added here, as rustc adds them. A question this does not understand
//! whole is answered `None`, and rustc is asked.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use serde_derive::Deserialize;

/// What the rustc of the toolchain in `harness/toolchain` says (`harness/answers.py --write`
/// writes it when the toolchain moves).
const RELEASES: [&str; 1] = [include_str!("rustc.json")];

#[derive(Deserialize)]
struct Release {
    /// As the first line of `-V` has it, after `rustc `.
    version: String,
    /// What `-vV` prints, with `{host}` for the host's tuple.
    verbose: String,
    targets: BTreeMap<String, Target>,
}

#[derive(Deserialize)]
struct Target {
    /// The cfgs of the target, but those the arguments decide.
    cfg: Vec<String>,
    /// The cfgs printed besides those when unstable features are allowed.
    nightly_cfg: Vec<String>,
    /// The strategy of `-C panic` when none is given.
    panic: String,
    split_debuginfo: Vec<String>,
    /// The name of the file of each kind of crate named `___`; `None` for a kind the target
    /// does not have.
    file_names: BTreeMap<String, Option<String>>,
    /// By the host rustc runs on, whose build decides some of it.
    target_spec_json: BTreeMap<String, String>,
    /// The `target_feature`s each feature rustc has turns on beside the target's own, with
    /// itself: `-C target-feature=+<name>` on the nightly. One rustc warns of is not here.
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
    /// The same of the features only LLVM names, which `target-cpu=native` turns on.
    #[serde(default)]
    llvm_features: BTreeMap<String, Vec<String>>,
    /// Of each feature the target has, the features turning it off turns off: itself and
    /// those that need it.
    #[serde(default)]
    features_off: BTreeMap<String, Vec<String>>,
    /// Each CPU of `-C target-cpu`, by the index of its `target_feature`s in `cpu_features`.
    #[serde(default)]
    cpus: BTreeMap<String, usize>,
    #[serde(default)]
    cpu_features: Vec<Vec<String>>,
    /// The cfgs besides `target_feature` a feature has: wasm's `atomics`, `target_has_threads`.
    #[serde(default)]
    feature_cfgs: BTreeMap<String, Vec<String>>,
}

impl Target {
    /// The `target_feature`s of `cpu` with `features` turned on (`true`) and off in order,
    /// starting from `have`, the target's own; `None` for a CPU or feature rustc warns of or
    /// does not have.
    fn tuned(
        &self,
        have: Vec<String>,
        cpu: Option<(&str, Option<&str>)>,
        features: &[(bool, &str)],
    ) -> Option<Vec<String>> {
        let own: Vec<String> = have.clone();
        let mut have: std::collections::BTreeSet<String> = match cpu {
            Some((name, _)) => {
                self.cpu_features.get(*self.cpus.get(name)?)?.iter().cloned().collect()
            }
            None => have.into_iter().collect(),
        };
        // `target-cpu=native`: LLVM's features of the host, which name what rustc does not too.
        let host: Vec<(bool, String)> = cpu
            .and_then(|(_, host)| host)
            .map(|it| {
                it.split(',')
                    .filter_map(|it| Some((it.starts_with('+'), it.get(1..)?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let on = |name: &str| self.features.get(name).or_else(|| self.llvm_features.get(name));
        let mut apply = |enable: bool, name: &str, host: bool| -> Option<()> {
            match (enable, on(name)) {
                (true, Some(turned)) => {
                    have.extend(turned.iter().cloned());
                    // The target's own features it needs, should they be off.
                    have.extend(
                        self.features_off
                            .iter()
                            .filter(|(_, off)| off.iter().any(|it| it == name))
                            .map(|(it, _)| it.clone()),
                    );
                }
                (false, _) if self.features_off.contains_key(name) => {
                    self.features_off[name].iter().for_each(|it| _ = have.remove(it));
                }
                (false, Some(turned)) if !own.iter().any(|it| it == name) => {
                    // Off with it go the features that turn it on.
                    let needs = |other: &Vec<String>| turned.iter().all(|it| other.contains(it));
                    if !turned.is_empty() {
                        have.retain(|it| !self.features.get(it).is_some_and(needs));
                    }
                }
                // A feature only LLVM has, of no `target_feature`, changes none of them.
                _ if host => {}
                _ => return None,
            }
            Some(())
        };
        for (enable, name) in &host {
            apply(*enable, name, true)?;
        }
        for &(enable, name) in features {
            apply(enable, name, false)?;
        }
        Some(have.into_iter().collect())
    }
}

fn releases() -> &'static [Release] {
    static RELEASES_READ: OnceLock<Vec<Release>> = OnceLock::new();
    RELEASES_READ.get_or_init(|| {
        RELEASES
            .iter()
            .map(|it| serde_json::from_str(it).expect("the answers of a release"))
            .collect()
    })
}

/// Whether `sysroot` has the compiler's library, which is what makes it a toolchain. rustc
/// loads it from `lib/librustc_driver-*`, and on Windows, where a DLL is found beside the
/// executable, from `bin/rustc_driver-*.dll`.
fn has_driver(sysroot: &Path) -> bool {
    [("lib", "librustc_driver-"), ("bin", "rustc_driver-")].into_iter().any(|(dir, prefix)| {
        std::fs::read_dir(sysroot.join(dir)).is_ok_and(|mut found| {
            found.any(|it| {
                it.is_ok_and(|it| it.file_name().to_str().is_some_and(|it| it.starts_with(prefix)))
            })
        })
    })
}

/// The toolchain `rustc` is of, as far as ra-jit can tell without running it.
struct Toolchain {
    sysroot: PathBuf,
    host: String,
    release: &'static Release,
}

impl Toolchain {
    /// From rustup's records in the sysroot: the release of rustc and the host it runs on.
    /// rustc finds its sysroot by where its library is, which is beside it.
    fn of(rustc: &Path) -> Option<Toolchain> {
        let sysroot = rustc.parent()?.parent()?.to_owned();
        if !has_driver(&sysroot) {
            return None;
        }
        let rustlib = sysroot.join("lib").join("rustlib");
        let host = std::fs::read_dir(&rustlib).ok()?.flatten().find_map(|it| {
            it.file_name().to_str()?.strip_prefix("manifest-rustc-").map(str::to_owned)
        })?;
        let manifest =
            std::fs::read_to_string(rustlib.join("multirust-channel-manifest.toml")).ok()?;
        let (_, rest) = manifest.split_once("\n[pkg.rustc]\nversion = \"")?;
        let (version, _) = rest.split_once('"')?;
        let release = releases().iter().find(|it| it.version == version)?;
        Some(Toolchain { sysroot, host, release })
    }
}

/// What `cargo --version` says after `cargo ` for the toolchain at `sysroot`, from rustup's
/// channel manifest there: the release of its rustc (cargo's `CFG_RELEASE`) with the commit
/// and date of its cargo.
pub(super) fn cargo_version(sysroot: &Path) -> Option<String> {
    let manifest = sysroot.join("lib/rustlib/multirust-channel-manifest.toml");
    let manifest = std::fs::read_to_string(manifest).ok()?;
    let version = |package: &str| {
        let (_, rest) = manifest.split_once(&format!("\n[pkg.{package}]\nversion = \""))?;
        rest.split_once('"').map(|(version, _)| version)
    };
    let (release, _) = version("rustc")?.split_once(' ')?;
    let (_, commit) = version("cargo")?.split_once(' ')?;
    Some(format!("{release} {commit}"))
}

/// How cfgs follow from `-C` and `--cfg`.
#[derive(Default)]
struct Given<'a> {
    target: Option<&'a str>,
    opt_level: Option<&'a str>,
    debug_assertions: Option<bool>,
    overflow_checks: Option<bool>,
    panic: Option<&'a str>,
    /// The features of `-C target-feature`, on (`true`) or off, in order.
    features: Vec<(bool, &'a str)>,
    target_cpu: Option<&'a str>,
    cfgs: Vec<&'a str>,
    crate_name: Option<&'a str>,
    crate_types: Vec<&'a str>,
    prints: Vec<&'a str>,
    unstable_options: bool,
    from_stdin: bool,
    extra_filename: bool,
}

/// The flags that take a value and change no answer.
const IGNORED_VALUED: [&str; 12] = [
    "--edition",
    "--error-format",
    "--json",
    "--emit",
    "--out-dir",
    "-L",
    "--extern",
    "--check-cfg",
    "--cap-lints",
    "--color",
    "--diagnostic-width",
    "--remap-path-prefix",
];

/// The lint flags, whose value changes no answer.
const LINTS: [&str; 10] = [
    "-W",
    "-A",
    "-D",
    "-F",
    "--warn",
    "--allow",
    "--deny",
    "--forbid",
    "--force-warn",
    "--cap-lints",
];

/// The codegen options that change no answer. `linker-features` is how
/// rustc links (cargo's `-Clinker-features=-lld`), not the target it prints;
/// `symbol-mangling-version` how it names symbols (zed's `-C symbol-mangling-version=v0`).
const IGNORED_CODEGEN: [&str; 18] = [
    "embed-bitcode",
    "debuginfo",
    "metadata",
    "incremental",
    "strip",
    "codegen-units",
    "prefer-dynamic",
    "lto",
    "linker",
    "linker-features",
    "link-arg",
    "link-args",
    "split-debuginfo",
    "link-dead-code",
    "rpath",
    "force-frame-pointers",
    "force-unwind-tables",
    "symbol-mangling-version",
];

fn yes(value: Option<&str>) -> Option<bool> {
    match value {
        None | Some("y" | "yes" | "on" | "true") => Some(true),
        Some("n" | "no" | "off" | "false") => Some(false),
        Some(_) => None,
    }
}

impl<'a> Given<'a> {
    fn of(args: &'a [&'a str]) -> Option<Given<'a>> {
        let mut given = Given::default();
        let mut words = args.iter().copied();
        while let Some(word) = words.next() {
            let (flag, joined) = match word.split_once('=') {
                Some((flag, value)) if flag.starts_with("--") => (flag, Some(value)),
                _ => match word.as_bytes() {
                    // `-Cname=value`, `-Zname`, `-Wlint`, `-Ldir`.
                    [b'-', b'C' | b'Z' | b'W' | b'A' | b'D' | b'F' | b'L', _, ..] => {
                        (&word[..2], Some(&word[2..]))
                    }
                    _ => (word, None),
                },
            };
            let mut value = || joined.or_else(|| words.next());
            match flag {
                "-" => given.from_stdin = true,
                "--print" => given.prints.push(value()?),
                "--target" => given.target = Some(value()?),
                "--crate-name" => given.crate_name = Some(value()?),
                "--crate-type" => given.crate_types.extend(value()?.split(',')),
                "--cfg" => given.cfgs.push(value()?),
                "-O" => given.opt_level = Some("2"),
                "-g" => {}
                "-C" | "--codegen" => {
                    let option = value()?;
                    let (name, value) = match option.split_once('=') {
                        Some((name, value)) => (name, Some(value)),
                        None => (option, None),
                    };
                    match name.replace('_', "-").as_str() {
                        "opt-level" => given.opt_level = Some(value?),
                        "debug-assertions" => given.debug_assertions = Some(yes(value)?),
                        "overflow-checks" => given.overflow_checks = Some(yes(value)?),
                        "panic" => given.panic = Some(value?),
                        "extra-filename" => given.extra_filename = true,
                        "target-feature" => {
                            for feature in value?.split(',').filter(|it| !it.is_empty()) {
                                let enable = match feature.as_bytes()[0] {
                                    b'+' => true,
                                    b'-' => false,
                                    _ => return None,
                                };
                                given.features.push((enable, &feature[1..]));
                            }
                        }
                        "target-cpu" => given.target_cpu = Some(value?),
                        name if IGNORED_CODEGEN.contains(&name) => {}
                        _ => return None,
                    }
                }
                "-Z" => match crate::unstable::flag(value()?) {
                    flag if flag.name == "unstable-options" => given.unstable_options = true,
                    // One that changes only how rustc works changes no answer.
                    flag if matches!(flag.treatment(), crate::unstable::Treatment::Ignored(_)) => {}
                    _ => return None,
                },
                _ if LINTS.contains(&flag) || IGNORED_VALUED.contains(&flag) => {
                    value()?;
                }
                // An input rustc does not read for what it is asked.
                _ if !flag.starts_with('-') => {}
                _ => return None,
            }
        }
        Some(given)
    }

    fn debug_assertions(&self) -> bool {
        self.debug_assertions.unwrap_or(matches!(self.opt_level, None | Some("0")))
    }
}

/// The architecture of a target's tuple: what is before its first `-`.
fn arch(tuple: &str) -> &str {
    tuple.split('-').next().unwrap_or(tuple)
}

/// The variables rustc reads for any question, which ra-jit answers only when they are unset.
const UNSET: [&str; 2] = ["RUSTC_OVERRIDE_VERSION_STRING", "RUST_TARGET_PATH"];

/// The variables of Apple's SDK and deployment targets, which a shell of nixpkgs on macOS
/// sets. rustc reads them to link and for the target's spec, which says the deployment
/// target; its version, cfgs, sysroot and file names are the same with them, and are
/// answered with them set.
const APPLE: [&str; 6] = [
    "SDKROOT",
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "TVOS_DEPLOYMENT_TARGET",
    "WATCHOS_DEPLOYMENT_TARGET",
    "XROS_DEPLOYMENT_TARGET",
];

/// The cfgs rustc sets itself, which it refuses as `--cfg`, by name or by what it starts with.
const BUILTIN_CFGS: [&str; 13] = [
    "debug_assertions",
    "overflow_checks",
    "ub_checks",
    "contract_checks",
    "fmt_debug",
    "relocation_model",
    "panic",
    "unix",
    "windows",
    "test",
    "emscripten_wasm_eh",
    "sanitize",
    "sanitizer_",
];

/// Whether `cfg` is `name` or `name="value"` as rustc prints it back: a plain name of no
/// builtin cfg, a value with nothing to escape.
fn is_plain_cfg(cfg: &str) -> bool {
    let (name, value) = match cfg.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (cfg, None),
    };
    let plain_name = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let builtin = name.starts_with("target_")
        || BUILTIN_CFGS.iter().any(|it| name == *it || (it.ends_with('_') && name.starts_with(it)));
    let plain_value = value.is_none_or(|value| {
        let inner = value.strip_prefix('"').and_then(|it| it.strip_suffix('"'));
        inner.is_some_and(|it| {
            it.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ')
        })
    });
    plain_name && !builtin && plain_value
}

/// What `rustc` (the toolchain's executable) prints for `args` with `stdin`, with the
/// variables `var` reads; `None` for what this does not answer.
pub(super) fn answer(
    rustc: &Path,
    args: &[OsString],
    stdin: &[u8],
    var: &dyn Fn(&str) -> Option<OsString>,
) -> Option<Vec<u8>> {
    if UNSET.iter().any(|it| var(it).is_some()) {
        return None;
    }
    let args = args.iter().map(|it| it.to_str()).collect::<Option<Vec<_>>>()?;
    // A driver of rustc's (clippy's) may answer otherwise.
    if rustc.file_stem()? != "rustc" {
        return None;
    }
    let toolchain = Toolchain::of(rustc)?;
    let release = toolchain.release;
    // Whether unstable features are allowed: always on a nightly (the toolchain ra-jit is
    // pinned to), unless `RUSTC_BOOTSTRAP=-1` makes it act as a stable release; a list of
    // crates is not answered.
    let nightly = match var("RUSTC_BOOTSTRAP") {
        None => release.version.contains("-nightly") || release.version.contains("-dev"),
        Some(it) if it == "1" => true,
        Some(it) if it == "-1" => false,
        Some(_) => return None,
    };
    if !args.is_empty()
        && args.iter().all(|it| matches!(*it, "-V" | "-v" | "-vV" | "--version" | "--verbose"))
    {
        let verbose = args.iter().any(|it| matches!(*it, "-v" | "-vV" | "--verbose"));
        let text = if verbose {
            release.verbose.replace("{host}", &toolchain.host)
        } else {
            format!("rustc {}\n", release.version)
        };
        return Some(text.into_bytes());
    }
    let given = Given::of(&args)?;
    if given.prints.is_empty() || (given.from_stdin && !stdin.trim_ascii().is_empty()) {
        return None;
    }
    let tuple = given.target.unwrap_or(&toolchain.host);
    let target = release.targets.get(tuple)?;
    let mut out = String::new();
    for print in &given.prints {
        match *print {
            "sysroot" => out += &format!("{}\n", toolchain.sysroot.to_str()?),
            "target-libdir" => {
                let dir = toolchain.sysroot.join("lib/rustlib").join(tuple).join("lib");
                out += &format!("{}\n", dir.to_str()?);
            }
            "host-tuple" => out += &format!("{}\n", toolchain.host),
            "split-debuginfo" => {
                target.split_debuginfo.iter().for_each(|it| out += &format!("{it}\n"))
            }
            "crate-name" => out += &format!("{}\n", given.crate_name.unwrap_or("rust_out")),
            "file-names" => {
                if given.extra_filename || !given.from_stdin {
                    return None;
                }
                let name = given.crate_name.unwrap_or("rust_out");
                let mut kinds: Vec<&str> = Vec::new();
                for kind in
                    if given.crate_types.is_empty() { &["bin"][..] } else { &given.crate_types }
                {
                    if !kinds.contains(kind) {
                        kinds.push(kind);
                    }
                }
                // `lib` is an `rlib`, which rustc may not name twice.
                if kinds.contains(&"lib") && kinds.contains(&"rlib") {
                    return None;
                }
                for kind in kinds {
                    let file = target.file_names.get(kind)?.as_deref()?;
                    out += &format!("{}\n", file.replace("___", name));
                }
            }
            "cfg" => {
                let mut cfgs: Vec<String> = target.cfg.clone();
                if nightly {
                    cfgs.extend(target.nightly_cfg.iter().cloned());
                }
                // Whether a target respects `crt-static` is its spec's, so only asking for
                // what it has already is answered.
                let static_default = cfgs.iter().any(|it| it == "target_feature=\"crt-static\"");
                // Only a target's own `crt-static`, asked for again, changes nothing.
                let as_it_is = given
                    .features
                    .iter()
                    .all(|&(enable, name)| name == "crt-static" && enable == static_default);
                if given.target_cpu.is_some() || !as_it_is {
                    // The features rustc has without unstable ones allowed are not recorded.
                    if !nightly {
                        return None;
                    }
                    let cpu = match given.target_cpu {
                        // LLVM's host CPU, for any target of the host's architecture.
                        Some("native") if arch(tuple) == arch(&toolchain.host) => {
                            let (name, features) = hir_ty_codegen::host_cpu()?;
                            Some((name, Some(features)))
                        }
                        Some("native") => return None,
                        Some(name) => Some((name.to_owned(), None)),
                        None => None,
                    };
                    let feature = |it: &String| {
                        let name = it.strip_prefix("target_feature=\"")?.strip_suffix('"')?;
                        Some(name.to_owned())
                    };
                    let own = cfgs.iter().filter_map(feature).collect();
                    let cpu = cpu.as_ref().map(|(name, host)| (name.as_str(), host.as_deref()));
                    let tuned = target.tuned(own, cpu, &given.features)?;
                    cfgs.retain(|it| feature(it).is_none());
                    for feature in &tuned {
                        cfgs.extend(
                            target.feature_cfgs.get(feature).into_iter().flatten().cloned(),
                        );
                    }
                    cfgs.extend(tuned.iter().map(|it| format!("target_feature=\"{it}\"")));
                }
                let debug_assertions = given.debug_assertions();
                if debug_assertions {
                    cfgs.push("debug_assertions".to_owned());
                }
                if nightly && given.overflow_checks.unwrap_or(debug_assertions) {
                    cfgs.push("overflow_checks".to_owned());
                }
                if nightly && debug_assertions {
                    cfgs.push("ub_checks".to_owned());
                }
                let panic = given.panic.unwrap_or(&target.panic);
                if panic != target.panic && panic != "abort" {
                    return None;
                }
                cfgs.push(format!("panic=\"{panic}\""));
                if given.crate_types.contains(&"proc-macro") {
                    cfgs.push("proc_macro".to_owned());
                }
                for cfg in &given.cfgs {
                    if !is_plain_cfg(cfg) {
                        return None;
                    }
                    cfgs.push((*cfg).to_owned());
                }
                cfgs.sort();
                cfgs.dedup();
                cfgs.iter().for_each(|it| out += &format!("{it}\n"));
            }
            "target-spec-json"
                if given.unstable_options
                    && nightly
                    && APPLE.iter().all(|it| var(it).is_none()) =>
            {
                out += target.target_spec_json.get(&toolchain.host)?;
            }
            _ => return None,
        }
    }
    Some(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The compiler's library is beside rustc on Windows and in `lib` elsewhere: a sysroot with
    /// neither is no toolchain.
    #[test]
    fn a_toolchain_has_the_compilers_library_in_lib_or_beside_rustc() {
        let dir = std::env::temp_dir().join(format!("ra-jit-driver-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sysroot = |name: &str| {
            let sysroot = dir.join(name);
            std::fs::create_dir_all(sysroot.join("lib")).unwrap();
            std::fs::create_dir_all(sysroot.join("bin")).unwrap();
            sysroot
        };
        let (unix, windows, neither) = (sysroot("unix"), sysroot("windows"), sysroot("neither"));
        std::fs::write(unix.join("lib/librustc_driver-0123abcd.so"), b"").unwrap();
        std::fs::write(windows.join("bin/rustc_driver-0123abcd.dll"), b"").unwrap();
        std::fs::write(neither.join("lib/libstd-0123abcd.so"), b"").unwrap();
        let found = [&unix, &windows, &neither].map(|it| has_driver(it));
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found, [true, true, false]);
    }

    #[test]
    fn cfgs_given() {
        assert!(is_plain_cfg("foo"));
        assert!(is_plain_cfg("feature=\"serde derive\""));
        assert!(!is_plain_cfg("feature=serde"));
        assert!(!is_plain_cfg("feature=\"a\\\"b\""));
        assert!(!is_plain_cfg("debug_assertions"));
        assert!(!is_plain_cfg("target_os=\"linux\""));
        assert!(!is_plain_cfg("sanitizer_cfi_generalize_pointers"));
        assert!(!is_plain_cfg("9lives"));
    }

    #[test]
    fn arguments_read() {
        let args =
            ["-Copt-level=3", "-C", "panic=abort", "--print=cfg", "--cfg", "foo", "-Wwarnings"];
        let given = Given::of(&args).unwrap();
        assert_eq!(given.opt_level, Some("3"));
        assert_eq!(given.panic, Some("abort"));
        assert_eq!(given.prints, ["cfg"]);
        assert_eq!(given.cfgs, ["foo"]);
        assert!(!given.debug_assertions());
        assert!(Given::of(&["-O"]).is_some_and(|it| !it.debug_assertions()));
        assert!(Given::of(&["-C", "opt-level=0"]).is_some_and(|it| it.debug_assertions()));
        // The features and the CPU, in order, which `Target::tuned` answers from rustc's.
        let given =
            Given::of(&["-C", "target-cpu=x86-64-v3", "-Ctarget-feature=-crt-static,+avx2"]);
        let given = given.unwrap();
        assert_eq!(given.target_cpu, Some("x86-64-v3"));
        assert_eq!(given.features, [(false, "crt-static"), (true, "avx2")]);
        assert!(Given::of(&["-Ctarget-feature=avx2", "--print", "cfg"]).is_none());
        assert!(Given::of(&["-Z", "ub-checks", "--print", "cfg"]).is_none());
        assert!(Given::of(&["-Zthreads=8", "--print", "cfg"]).is_some());
        // cargo's `-Clinker-features=-lld` is how rustc links (rust-analyzer's `cargo rustc
        // --print cfg`, cargo's question of its targets).
        assert!(Given::of(&["-Clinker-features=-lld", "-O", "--print", "cfg"]).is_some());
        // zed's rustflags.
        let zed = ["-C", "link-arg=-fuse-ld=lld", "-C", "symbol-mangling-version=v0"];
        let zed = [&zed[..], &["--cfg", "tokio_unstable", "--print", "cfg"]].concat();
        let given = Given::of(&zed).unwrap();
        assert_eq!(given.cfgs, ["tokio_unstable"]);
    }
}
