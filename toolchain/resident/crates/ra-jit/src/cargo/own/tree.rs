//! `cargo tree`: its command line read as cargo 1.101.0 reads it, the tree made and printed by
//! blaze-tree.

use std::io::IsTerminal as _;
use std::path::PathBuf;

use blaze_tree::{DisplayDepth, EdgeKind, Kinds, Options, Packages, Platformed, Prefix, Target};
use rustc_hash::FxHashSet;

use super::args::{Args, Read, switch, valued};
use super::manifest::{USAGE, failed, host_triple, read_or_refuse, target_cfg};
use crate::cargo::say;

const EDGES: &[&str] = &[
    "all",
    "normal",
    "build",
    "dev",
    "features",
    "public",
    "no-normal",
    "no-build",
    "no-dev",
    "no-proc-macro",
];

pub(super) fn command(args: Args<'_>) -> anyhow::Result<i32> {
    let flags = [
        switch(&["-a", "--all"]),
        switch(&["--no-dev-dependencies"]),
        valued(&["-e", "--edges"]),
        valued(&["-i", "--invert"]),
        valued(&["--prune"]),
        valued(&["--depth"]),
        switch(&["--no-indent"]),
        switch(&["--prefix-depth"]),
        valued(&["--prefix"]),
        switch(&["--no-dedupe"]),
        switch(&["-d", "--duplicates", "--duplicate"]),
        valued(&["--charset"]),
        valued(&["-f", "--format"]),
        switch(&["-V", "--version"]),
        valued(&["-p", "--package"]),
        switch(&["--workspace"]),
        valued(&["--exclude"]),
        valued(&["-F", "--features"]),
        switch(&["--all-features"]),
        switch(&["--no-default-features"]),
        switch(&["--all-targets"]),
        valued(&["--target"]),
        valued(&["--manifest-path"]),
    ];
    let usage = "cargo blaze tree [OPTIONS]";
    let read = match read_or_refuse(&args, &flags, usage) {
        Ok(read) => read,
        Err(code) => return Ok(code),
    };
    if read.has("--version") {
        println!("{}", crate::version());
        return Ok(0);
    }
    let opts = match options(&read) {
        Ok(opts) => opts,
        Err(Refused { why, usage: true }) => {
            say::error(&why);
            eprintln!("\nUsage: {usage}\n\nFor more information, try '--help'.");
            return Ok(USAGE);
        }
        Err(Refused { why, usage: false }) => return Ok(failed(&anyhow::anyhow!(why))),
    };
    let printed = (|| {
        let targets = match &opts.target {
            Target::Specific(triples) => triples.clone(),
            Target::Host | Target::All => Vec::new(),
        };
        let platformed = |triple: String| -> anyhow::Result<Platformed> {
            Ok(Platformed { cfg: target_cfg(&triple)?, triple })
        };
        let kinds = Kinds {
            host: platformed(host_triple()?)?,
            targets: targets.into_iter().map(platformed).collect::<anyhow::Result<_>>()?,
        };
        let cwd = std::env::current_dir()?;
        let manifest_path = read.value("--manifest-path").map(PathBuf::from);
        let config = read.values("--config");
        let features = read.features();
        let asked = blaze_metadata::Asked {
            cwd: &cwd,
            env: std::env::vars().collect(),
            manifest_path: manifest_path.as_deref(),
            config: &config,
            features: &features,
            all_features: read.has("--all-features"),
            no_default_features: read.has("--no-default-features"),
            build: None,
            platform: None,
            offline: read.has("--offline") || read.has("--frozen"),
            store: None,
            rustc_version: Some(crate::cargo::toolchain::ours_release()),
        };
        let mut out = String::new();
        blaze_tree::print_tree(&asked, &opts, &kinds, &mut out, &mut |it| say::warning(it))?;
        anyhow::Ok(out)
    })();
    match printed {
        Ok(out) => {
            print!("{out}");
            Ok(0)
        }
        Err(err) => Ok(failed(&err)),
    }
}

/// A command line cargo refuses, and why: with its usage, as clap refuses it, or as a command
/// that failed.
struct Refused {
    why: String,
    usage: bool,
}

fn usage(why: String) -> Refused {
    Refused { why, usage: true }
}

fn failure(why: &str) -> Refused {
    Refused { why: why.to_owned(), usage: false }
}

fn options(read: &Read) -> Result<Options, Refused> {
    let warn = say::warning;
    let prefix = if read.has("--no-indent") {
        warn("the --no-indent flag has been changed to --prefix=none");
        "none".to_owned()
    } else if read.has("--prefix-depth") {
        warn("the --prefix-depth flag has been changed to --prefix=depth");
        "depth".to_owned()
    } else {
        read.value("--prefix").unwrap_or_else(|| "indent".to_owned())
    };
    let prefix = match prefix.as_str() {
        "none" => Prefix::None,
        "indent" => Prefix::Indent,
        "depth" => Prefix::Depth,
        other => return Err(invalid(other, "--prefix <PREFIX>", &["depth", "indent", "none"])),
    };
    let no_dedupe = read.has("--no-dedupe") || read.has("--all");
    if read.has("--all") {
        warn(
            "the `cargo tree` --all flag has been changed to --no-dedupe, and may be removed in a future version\n  |\n  = help: if you are looking to display all workspace members, use the --workspace flag",
        );
    }
    let targets = if read.has("--all-targets") {
        warn("the --all-targets flag has been changed to --target=all");
        vec!["all".to_owned()]
    } else {
        read.values("--target")
    };
    let (edge_kinds, no_proc_macro, public) = edge_kinds(read)?;
    let graph_features = edge_kinds.contains(&EdgeKind::Feature);
    let display_depth = match read.value("--depth") {
        None => DisplayDepth::MaxDisplayDepth(u32::MAX),
        Some(depth) if depth == "workspace" => DisplayDepth::Workspace,
        Some(depth) => DisplayDepth::MaxDisplayDepth(depth.parse().map_err(|_| {
            usage(format!(
                "invalid value '{depth}' for '--depth <DEPTH>': supported values for --depth are non-negative integers and `workspace`, but `{depth}` is unknown"
            ))
        })?),
    };
    let package = read.values("--package");
    let exclude = read.values("--exclude");
    let packages = match (read.has("--workspace"), exclude.is_empty(), package.is_empty()) {
        (false, true, true) => Packages::Default,
        (false, true, false) => Packages::Packages(package),
        (false, false, _) => {
            return Err(failure("--exclude can only be used together with --workspace"));
        }
        (true, true, _) => Packages::All(package),
        (true, false, _) => Packages::OptOut(exclude),
    };
    let unicode = match read.value("--charset").as_deref() {
        None => !std::io::stdout().is_terminal() || utf8_locale(),
        Some("utf8") => true,
        Some("ascii") => false,
        Some(other) => return Err(invalid(other, "--charset <CHARSET>", &["utf8", "ascii"])),
    };
    let color = match read.value("--color").or_else(|| std::env::var("CARGO_TERM_COLOR").ok()) {
        Some(it) if it == "always" => true,
        Some(it) if it == "never" => false,
        _ => std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    };
    let opts = Options {
        packages,
        target: Target::from_cli(targets),
        edge_kinds,
        invert: read.values("--invert"),
        pkgs_to_prune: read.values("--prune"),
        prefix,
        no_dedupe,
        duplicates: read.has("--duplicates"),
        format: read.value("--format").unwrap_or_else(|| "{p}".to_owned()),
        graph_features,
        display_depth,
        no_proc_macro,
        public,
        unicode,
        color,
    };
    if opts.graph_features && opts.duplicates {
        return Err(failure("the `-e features` flag does not support `--duplicates`"));
    }
    Ok(opts)
}

fn invalid(value: &str, flag: &str, possible: &[&str]) -> Refused {
    usage(format!(
        "invalid value '{value}' for '{flag}'\n  [possible values: {}]",
        possible.join(", ")
    ))
}

/// Whether the locale says UTF-8, as the `supports-unicode` crate cargo reads judges it.
fn utf8_locale() -> bool {
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|it| std::env::var(it).ok().filter(|it| !it.is_empty()))
        .unwrap_or_default()
        .to_ascii_lowercase();
    std::env::var("TERM").as_deref() != Ok("linux")
        && (locale.contains("utf-8") || locale.contains("utf8"))
}

/// `--edges`: cargo's `parse_edge_kinds`. The kinds, whether proc macros are left out, and
/// whether only public dependencies are shown.
fn edge_kinds(read: &Read) -> Result<(FxHashSet<EdgeKind>, bool, bool), Refused> {
    use blaze_tree::DepKind;
    let given = read.values("--edges");
    let mut kinds: Vec<&str> = given.iter().flat_map(|it| it.split(',')).collect();
    if let Some(other) = kinds.iter().find(|it| !EDGES.contains(it)) {
        return Err(invalid(other, "--edges <KINDS>", EDGES));
    }
    let no_proc_macro = kinds.contains(&"no-proc-macro");
    let public = kinds.contains(&"public");
    kinds.retain(|it| !matches!(*it, "no-proc-macro" | "public"));
    if read.has("--no-dev-dependencies") {
        say::warning("the --no-dev-dependencies flag has changed to -e=no-dev");
        kinds.push("no-dev");
    }
    if kinds.is_empty() {
        kinds.extend(["normal", "build", "dev"]);
    }
    if public {
        return Err(failure("`--edges public` requires `-Zunstable-options`"));
    }
    let mut result = FxHashSet::default();
    let defaults = [
        EdgeKind::Dep(DepKind::Normal),
        EdgeKind::Dep(DepKind::Build),
        EdgeKind::Dep(DepKind::Development),
    ];
    if kinds.iter().any(|k| k.starts_with("no-")) {
        result.extend(defaults);
        for kind in &kinds {
            match *kind {
                "no-normal" => result.remove(&EdgeKind::Dep(DepKind::Normal)),
                "no-build" => result.remove(&EdgeKind::Dep(DepKind::Build)),
                "no-dev" => result.remove(&EdgeKind::Dep(DepKind::Development)),
                "features" => result.insert(EdgeKind::Feature),
                _ => {
                    return Err(failure(&format!(
                        "`{kind}` dependency kind cannot be mixed with \"no-normal\", \"no-build\", or \"no-dev\" dependency kinds"
                    )));
                }
            };
        }
        return Ok((result, no_proc_macro, public));
    }
    for kind in &kinds {
        match *kind {
            "all" => {
                result.extend(defaults);
                result.insert(EdgeKind::Feature);
            }
            "features" => {
                result.insert(EdgeKind::Feature);
            }
            "normal" => {
                result.insert(EdgeKind::Dep(DepKind::Normal));
            }
            "build" => {
                result.insert(EdgeKind::Dep(DepKind::Build));
            }
            _ => {
                result.insert(EdgeKind::Dep(DepKind::Development));
            }
        }
    }
    if kinds == ["features"] {
        result.extend(defaults);
    }
    Ok((result, no_proc_macro, public))
}
