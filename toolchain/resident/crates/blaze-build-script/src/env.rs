//! The environment cargo 1.99 runs a build script with: `Compilation::host_process` and
//! `fill_env` (`src/compiler/compilation.rs`), then `build_work`
//! (`src/compiler/custom_build.rs`) and the jobserver, in rust-lang/cargo at 5f94df478, the
//! commit `cargo 1.99.0 -vV` names. The script inherits cargo's own environment with what
//! these set and take away.
//!
//! Left out, as unstable in 1.99 and so never set by a stable cargo: `CARGO_DEP_<name>_<key>`
//! (`-Z any-build-script-metadata`), `CARGO_TRIM_PATHS_*` (`-Z trim-paths`), the artifact
//! dependencies' variables (`-Z bindeps`) and the host runner (`-Z host-config`).

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

use crate::{Error, unit::Unit};

/// A script's whole environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    pub vars: BTreeMap<OsString, OsString>,
}

impl Env {
    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.vars.get(OsStr::new(name)).map(OsString::as_os_str)
    }

    /// Gives `command` this environment and no other.
    pub fn apply(&self, command: &mut Command) {
        command.env_clear().envs(&self.vars);
    }
}

/// What cargo sets on the command, in order, over what it inherits: a variable set, or taken
/// away (`None`).
#[derive(Default)]
struct Explicit {
    vars: BTreeMap<OsString, Option<OsString>>,
}

impl Explicit {
    fn set(&mut self, name: impl Into<OsString>, value: impl Into<OsString>) {
        self.vars.insert(name.into(), Some(value.into()));
    }

    fn remove(&mut self, name: &str) {
        self.vars.insert(name.into(), None);
    }

    fn contains(&self, name: &str) -> bool {
        self.vars.contains_key(OsStr::new(name))
    }

    fn get(&self, name: &str) -> Option<&OsStr> {
        self.vars.get(OsStr::new(name)).and_then(|it| it.as_deref())
    }
}

/// What the machine that runs the script is, as far as its environment goes: cargo decides
/// these by what it was built for, which is the host.
struct HostOs {
    windows: bool,
    macos: bool,
    /// The variable programs find dynamic libraries by.
    dylib_var: &'static str,
}

impl HostOs {
    fn of(host: &str) -> HostOs {
        let windows = host.contains("windows");
        let macos = host.contains("apple");
        let dylib_var = if windows {
            "PATH"
        } else if macos {
            "DYLD_FALLBACK_LIBRARY_PATH"
        } else if host.contains("aix") {
            "LIBPATH"
        } else if host.contains("haiku") {
            "LIBRARY_PATH"
        } else {
            "LD_LIBRARY_PATH"
        };
        HostOs { windows, macos, dylib_var }
    }

    /// `std::env::split_paths` on the host.
    fn split_paths(&self, value: &OsStr) -> Vec<PathBuf> {
        let value = value.to_string_lossy();
        if !self.windows {
            return value.split(':').map(PathBuf::from).collect();
        }
        let mut paths = Vec::new();
        let mut path = String::new();
        let mut quoted = false;
        let mut after_separator = false;
        for c in value.chars() {
            after_separator = false;
            match c {
                '"' => quoted = !quoted,
                ';' if !quoted => {
                    paths.push(PathBuf::from(std::mem::take(&mut path)));
                    after_separator = true;
                }
                c => path.push(c),
            }
        }
        if after_separator || !path.is_empty() {
            paths.push(PathBuf::from(path));
        }
        paths
    }

    /// `std::env::join_paths` on the host, which cargo fails on as here.
    fn join_paths(&self, paths: &[PathBuf], var: &str) -> Result<OsString, Error> {
        let mut joined = OsString::new();
        for (i, path) in paths.iter().enumerate() {
            let text = path.as_os_str();
            let lossy = text.to_string_lossy();
            if i > 0 {
                joined.push(if self.windows { ";" } else { ":" });
            }
            if self.windows && lossy.contains('"') || !self.windows && lossy.contains(':') {
                let mut message = format!(
                    "failed to join paths from `${var}` together\n\n\
                     Check if any of path segments listed below contain an \
                     unterminated quote character or path separator:"
                );
                for path in paths {
                    message.push_str(&format!("\n    {path:?}"));
                }
                return Err(Error(message));
            }
            if self.windows && lossy.contains(';') {
                joined.push("\"");
                joined.push(text);
                joined.push("\"");
            } else {
                joined.push(text);
            }
        }
        Ok(joined)
    }
}

/// A name as cargo makes it part of a variable's: upper case, `-` as `_`.
pub fn envify(name: &str) -> String {
    name.chars().flat_map(char::to_uppercase).map(|c| if c == '-' { '_' } else { c }).collect()
}

/// A cfg as `rustc --print cfg` prints it: a name, or a name and a quoted value.
fn cfg_of(line: &str) -> (&str, Option<&str>) {
    match line.split_once('=') {
        Some((name, value)) => {
            let value = value.trim();
            let value =
                value.strip_prefix('"').and_then(|it| it.strip_suffix('"')).unwrap_or(value);
            (name.trim(), Some(value))
        }
        None => (line.trim(), None),
    }
}

/// The search path of `paths` for dynamic libraries: of those with a kind (`native=`,
/// `crate=`, `dependency=`, `framework=`, `all=`), the directory.
fn dynamic_search_path(path: &Path) -> &Path {
    match path.to_str().and_then(|it| it.split_once('=')) {
        Some(("native" | "crate" | "dependency" | "framework" | "all", path)) => Path::new(path),
        _ => path,
    }
}

/// Of `paths`, the directories below `root`: one outside is likely the system's, whose
/// libraries would be found in place of the system's own (cargo's issue 3366).
fn below(paths: &[PathBuf], root: &Path) -> Vec<PathBuf> {
    paths
        .iter()
        .map(|it| dynamic_search_path(it))
        .filter(|it| it.starts_with(root))
        .map(Path::to_owned)
        .collect()
}

/// Where openssl-probe 0.1.6 looks for certificates, in order, on `host`.
fn certificate_dirs(host: &str) -> Vec<&'static Path> {
    let mut dirs = vec![
        "/var/ssl",
        "/usr/share/ssl",
        "/usr/local/ssl",
        "/usr/local/openssl",
        "/usr/local/etc/openssl",
        "/usr/local/share",
        "/usr/lib/ssl",
        "/usr/ssl",
        "/etc/openssl",
        "/etc/pki/ca-trust/extracted/pem",
        "/etc/pki/tls",
        "/etc/ssl",
        "/etc/certs",
        "/opt/etc/ssl",
    ];
    if host.contains("android") {
        dirs.push("/data/data/com.termux/files/usr/etc/tls");
    }
    if host.contains("haiku") {
        dirs.push("/boot/system/data/ssl");
    }
    dirs.into_iter().map(Path::new).collect()
}

const CERTIFICATE_FILES: [&str; 10] = [
    "cert.pem",
    "certs.pem",
    "ca-bundle.pem",
    "cacert.pem",
    "ca-certificates.crt",
    "certs/ca-certificates.crt",
    "certs/ca-root-nss.crt",
    "certs/ca-bundle.crt",
    "CARootCertificates.pem",
    "tls-ca-bundle.pem",
];

/// cargo's environment once it has started, from the one it was started with: what its
/// scripts inherit. Before any command, cargo starts libgit2, whose `openssl_env_init` (git2
/// 0.21.0, on hosts other than Windows, macOS and iOS) sets `SSL_CERT_FILE` and `SSL_CERT_DIR`
/// to the certificates openssl-probe 0.1.6 finds on this machine, unless they name a file and
/// a directory that exist.
pub fn cargo_started(
    started_with: &[(OsString, OsString)],
    host: &str,
) -> Vec<(OsString, OsString)> {
    cargo_started_in(started_with, host, &certificate_dirs(host))
}

fn cargo_started_in(
    started_with: &[(OsString, OsString)],
    host: &str,
    certificate_dirs: &[&Path],
) -> Vec<(OsString, OsString)> {
    let os = HostOs::of(host);
    let mut env = started_with.to_vec();
    if os.windows || os.macos {
        return env;
    }
    let existing = |name: &str| {
        started_with
            .iter()
            .find(|(it, _)| it == OsStr::new(name))
            .map(|(_, value)| PathBuf::from(value))
            .filter(|it| it.exists())
    };
    let mut cert_file = existing("SSL_CERT_FILE");
    let mut cert_dir = existing("SSL_CERT_DIR");
    for dir in certificate_dirs.iter().filter(|it| it.exists()) {
        if cert_file.is_none() {
            cert_file = CERTIFICATE_FILES.iter().map(|it| dir.join(it)).find(|it| it.exists());
        }
        if cert_dir.is_none() && dir.join("certs").exists() {
            cert_dir = Some(dir.join("certs"));
        }
        if cert_file.is_some() && cert_dir.is_some() {
            break;
        }
    }
    for (name, value) in [("SSL_CERT_FILE", cert_file), ("SSL_CERT_DIR", cert_dir)] {
        let Some(value) = value else { continue };
        match env.iter_mut().find(|(it, _)| it == OsStr::new(name)) {
            Some((_, it)) => *it = value.into_os_string(),
            None => env.push((name.into(), value.into_os_string())),
        }
    }
    env
}

/// The environment cargo runs `unit`'s script with, when cargo's own is `inherited` (see
/// [`cargo_started`]).
pub fn env(unit: &Unit, inherited: &[(OsString, OsString)]) -> Result<Env, Error> {
    let host = HostOs::of(&unit.host);
    let inherited_var = |name: &str| {
        inherited.iter().find(|(it, _)| it == OsStr::new(name)).map(|(_, value)| value.as_os_str())
    };
    let mut cmd = Explicit::default();
    let package = &unit.package;

    // `fill_env`, for a process on the host.
    let search = &unit.dylib_search;
    let mut search_path = Vec::new();
    if let Some(host_root) = &search.host_root {
        search_path.extend(below(&[], host_root));
        search_path.push(host_root.clone());
    }
    search_path.extend(search.host_deps.iter().cloned());
    search_path.push(search.sysroot_libdir.clone());
    let dylib_path =
        inherited_var(host.dylib_var).map(|it| host.split_paths(it)).unwrap_or_default();
    let dylib_path_is_empty = dylib_path.is_empty();
    if dylib_path.starts_with(&search_path) {
        search_path = dylib_path;
    } else {
        search_path.extend(dylib_path);
    }
    if host.macos && dylib_path_is_empty {
        if let Some(home) = inherited_var("HOME") {
            search_path.push(Path::new(home).join("lib"));
        }
        search_path.push(PathBuf::from("/usr/local/lib"));
        search_path.push(PathBuf::from("/usr/lib"));
    }
    cmd.set(host.dylib_var, host.join_paths(&search_path, host.dylib_var)?);
    cmd.set("CARGO", &unit.tools.cargo);
    let version = Version::of(&package.version);
    cmd.set("CARGO_MANIFEST_DIR", &package.manifest_dir);
    cmd.set("CARGO_MANIFEST_PATH", &package.manifest_path);
    cmd.set("CARGO_PKG_VERSION_MAJOR", version.major);
    cmd.set("CARGO_PKG_VERSION_MINOR", version.minor);
    cmd.set("CARGO_PKG_VERSION_PATCH", version.patch);
    cmd.set("CARGO_PKG_VERSION_PRE", version.pre);
    cmd.set("CARGO_PKG_VERSION", &package.version);
    cmd.set("CARGO_PKG_NAME", &package.name);
    let field = |it: &Option<String>| it.clone().unwrap_or_default();
    cmd.set("CARGO_PKG_DESCRIPTION", field(&package.description));
    cmd.set("CARGO_PKG_HOMEPAGE", field(&package.homepage));
    cmd.set("CARGO_PKG_REPOSITORY", field(&package.repository));
    cmd.set("CARGO_PKG_LICENSE", field(&package.license));
    cmd.set("CARGO_PKG_LICENSE_FILE", field(&package.license_file));
    cmd.set("CARGO_PKG_AUTHORS", package.authors.join(":"));
    cmd.set("CARGO_PKG_RUST_VERSION", field(&package.rust_version));
    cmd.set("CARGO_PKG_README", field(&package.readme));
    // `[env]` takes nothing cargo has set, and only what cargo's environment lacks unless forced.
    for it in &unit.config_env {
        if (it.force || inherited_var(&it.name).is_none()) && !cmd.contains(&it.name) {
            cmd.set(&it.name, &it.value);
        }
    }

    // `build_work`.
    cmd.set("OUT_DIR", &unit.out_dir);
    cmd.set("CARGO_MANIFEST_DIR", &package.manifest_dir);
    cmd.set("CARGO_MANIFEST_PATH", &package.manifest_path);
    cmd.set("NUM_JOBS", unit.jobs.to_string());
    cmd.set("TARGET", &unit.target);
    cmd.set("DEBUG", unit.profile.debuginfo.to_string());
    cmd.set("OPT_LEVEL", &unit.profile.opt_level);
    cmd.set("PROFILE", if unit.profile.release { "release" } else { "debug" });
    cmd.set("HOST", &unit.host);
    cmd.set("RUSTC", &unit.tools.rustc);
    cmd.set("RUSTDOC", &unit.tools.rustdoc);
    if let Some(linker) = &unit.tools.linker {
        cmd.set("RUSTC_LINKER", linker);
    }
    if let Some(links) = &package.links {
        cmd.set("CARGO_MANIFEST_LINKS", links);
    }
    for feature in &unit.features {
        cmd.set(format!("CARGO_FEATURE_{}", envify(feature)), "1");
    }
    // The cfgs as cargo's map has them: `feature` first, `debug_assertions` from the profile
    // and not from rustc, which asks without the profile's flags; a name again empties it.
    let mut cfgs: Vec<(&str, Vec<&str>)> = Vec::new();
    fn insert<'a>(cfgs: &mut Vec<(&'a str, Vec<&'a str>)>, name: &'a str, values: Vec<&'a str>) {
        match cfgs.iter_mut().find(|(it, _)| *it == name) {
            Some((_, it)) => *it = values,
            None => cfgs.push((name, values)),
        }
    }
    insert(&mut cfgs, "feature", unit.features.iter().map(String::as_str).collect());
    if unit.profile.debug_assertions {
        insert(&mut cfgs, "debug_assertions", Vec::new());
    }
    for line in &unit.target_cfgs {
        match cfg_of(line) {
            ("debug_assertions", None) => {}
            (name, None) => insert(&mut cfgs, name, Vec::new()),
            (name, Some(value)) => match cfgs.iter_mut().find(|(it, _)| *it == name) {
                Some((_, values)) => values.push(value),
                None => cfgs.push((name, vec![value])),
            },
        }
    }
    for (name, values) in cfgs {
        cmd.set(format!("CARGO_CFG_{}", envify(name)), values.join(","));
    }
    match &unit.tools.rustc_wrapper {
        Some(wrapper) => cmd.set("RUSTC_WRAPPER", wrapper),
        None => cmd.remove("RUSTC_WRAPPER"),
    }
    cmd.remove("RUSTC_WORKSPACE_WRAPPER");
    if unit.workspace_member
        && let Some(wrapper) = &unit.tools.rustc_workspace_wrapper
    {
        cmd.set("RUSTC_WORKSPACE_WRAPPER", wrapper);
    }
    cmd.set("CARGO_ENCODED_RUSTFLAGS", unit.rustflags.join("\x1f"));
    cmd.remove("RUSTFLAGS");
    for dependency in &unit.dependencies {
        let Some(links) = &dependency.links else { continue };
        for (key, value) in &dependency.metadata {
            cmd.set(format!("DEP_{}_{}", envify(links), envify(key)), value);
        }
    }
    // `add_plugin_deps`.
    if let Some(host_root) = &search.host_root {
        let mut search_path =
            cmd.get(host.dylib_var).map(|it| host.split_paths(it)).unwrap_or_default();
        search_path.extend(below(&search.plugin_search, host_root));
        cmd.set(host.dylib_var, host.join_paths(&search_path, host.dylib_var)?);
    }
    // The jobserver's `Client::configure`.
    if let Some(auth) = &unit.jobserver {
        cmd.set("CARGO_MAKEFLAGS", format!("-j --jobserver-fds={auth} --jobserver-auth={auth}"));
    }

    let mut vars: BTreeMap<OsString, OsString> = inherited.iter().cloned().collect();
    for (name, value) in cmd.vars {
        match value {
            Some(value) => _ = vars.insert(name, value),
            None => _ = vars.remove(&name),
        }
    }
    Ok(Env { vars })
}

/// The parts of a version cargo names apart.
struct Version<'a> {
    major: &'a str,
    minor: &'a str,
    patch: &'a str,
    pre: &'a str,
}

impl Version<'_> {
    fn of(version: &str) -> Version<'_> {
        let version = version.split_once('+').map_or(version, |(it, _)| it);
        let (numbers, pre) = version.split_once('-').unwrap_or((version, ""));
        let mut numbers = numbers.splitn(3, '.');
        let mut next = || numbers.next().unwrap_or("0");
        Version { major: next(), minor: next(), patch: next(), pre }
    }
}

#[cfg(test)]
mod tests;
