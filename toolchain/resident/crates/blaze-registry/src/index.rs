//! The lines of a registry's index, read as cargo 1.99 reads them (`IndexSummary::parse` in
//! `src/sources/registry/index/mod.rs`, the entry of `cargo-util-schemas`'s `index.rs`), and
//! cargo's cache of an index file (`index/cache.rs`), which blaze reads and writes in cargo's
//! format, so that the two share `$CARGO_HOME/registry/index`.

use std::{collections::BTreeMap, fmt, ops::Range, str::FromStr};

use anyhow::{Context, bail};
use cargo_platform::Platform;
use semver::{Version, VersionReq};
use serde::de::{self, Deserializer};
use serde_derive::Deserialize;

use blaze_source::SourceId;

/// The newest schema version of an index line (its `v`) that cargo 1.99 reads without
/// `-Zbindeps`. A line of a newer one is [`Status::Unsupported`].
pub const INDEX_V_MAX: u32 = 2;
/// The version of the format of cargo's cache of an index file.
const CACHE_VERSION: u8 = 3;

/// One version of a package, as a line of the index says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: String,
    pub version: Version,
    pub deps: Vec<IndexDependency>,
    /// `features` with `features2` added, as cargo adds them.
    pub features: BTreeMap<String, Vec<String>>,
    /// The sha256 of the `.crate` file, in hex.
    pub checksum: String,
    pub links: Option<String>,
    pub rust_version: Option<RustVersion>,
    /// When it was published, `yyyy-mm-ddThh:mm:ssZ`.
    pub pubtime: Option<String>,
}

/// What cargo does with a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Candidate,
    Yanked,
    /// Of a schema newer than [`INDEX_V_MAX`]: skipped by the resolver.
    Unsupported(u32),
    /// Not what an index line is, past its name and version, which are all the entry has:
    /// skipped by the resolver.
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexLine {
    pub entry: IndexEntry,
    pub status: Status,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DepKind {
    Normal,
    Development,
    Build,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexDependency {
    /// The name the depending package knows it by: the key of its `[dependencies]` entry.
    pub name_in_toml: String,
    /// The package's own name, which differs from `name_in_toml` for a renamed dependency.
    pub package: String,
    pub req: VersionReq,
    /// With the empty features older registries wrote taken out, as cargo takes them out.
    pub features: Vec<String>,
    pub optional: bool,
    pub default_features: bool,
    pub platform: Option<Platform>,
    pub kind: DepKind,
    /// The registry the dependency is from, when it is not this one.
    pub registry: Option<SourceId>,
    pub public: bool,
    pub artifact: Option<Artifact>,
}

/// An artifact dependency (`-Zbindeps`), checked as cargo checks one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artifact {
    pub kinds: Vec<String>,
    pub lib: bool,
    pub target: Option<String>,
}

/// A `rust-version`: a version with no pre-release or build, of which minor and patch may be
/// left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RustVersion {
    pub major: u64,
    pub minor: Option<u64>,
    pub patch: Option<u64>,
}

impl FromStr for RustVersion {
    type Err = anyhow::Error;

    /// As cargo's `PartialVersion`, then refusing a pre-release or build.
    fn from_str(value: &str) -> anyhow::Result<RustVersion> {
        let (major, minor, patch, pre, build) = match Version::parse(value) {
            Ok(it) => {
                (it.major, Some(it.minor), Some(it.patch), !it.pre.is_empty(), !it.build.is_empty())
            }
            Err(_) => {
                let mut req = match VersionReq::parse(value) {
                    Ok(req) => req,
                    Err(_) if value.contains('-') => bail!("unexpected prerelease field"),
                    Err(_) if value.contains('+') => bail!("unexpected build field"),
                    Err(_) => bail!("expected a version like \"1.32\""),
                };
                if req.comparators.len() != 1 {
                    bail!("unexpected version requirement");
                }
                let comparator = req.comparators.pop().expect("one comparator");
                if comparator.op != semver::Op::Caret || value.starts_with('^') {
                    bail!("unexpected version requirement");
                }
                let pre = !comparator.pre.is_empty();
                (comparator.major, comparator.minor, comparator.patch, pre, false)
            }
        };
        if pre {
            bail!("unexpected prerelease field, expected a version like \"1.32\"");
        }
        if build {
            bail!("unexpected build field, expected a version like \"1.32\"");
        }
        Ok(RustVersion { major, minor, patch })
    }
}

impl fmt::Display for RustVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.major)?;
        if let Some(minor) = self.minor {
            write!(f, ".{minor}")?;
        }
        if let Some(patch) = self.patch {
            write!(f, ".{patch}")?;
        }
        Ok(())
    }
}

/// An index line as JSON: cargo-util-schemas's `IndexPackage`, field for field, with what
/// serde does to a missing, extra or repeated field.
#[derive(Deserialize)]
struct IndexPackage {
    name: String,
    vers: Version,
    deps: Vec<RegistryDependency>,
    #[serde(default)]
    features: BTreeMap<String, Vec<String>>,
    features2: Option<BTreeMap<String, Vec<String>>>,
    cksum: String,
    yanked: Option<bool>,
    links: Option<String>,
    #[serde(default, deserialize_with = "rust_version")]
    rust_version: Option<RustVersion>,
    #[serde(default, deserialize_with = "pubtime")]
    pubtime: Option<String>,
    v: Option<u32>,
}

#[derive(Deserialize)]
struct RegistryDependency {
    name: String,
    req: String,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default)]
    optional: bool,
    #[serde(default = "yes")]
    default_features: bool,
    target: Option<String>,
    kind: Option<String>,
    registry: Option<String>,
    package: Option<String>,
    public: Option<bool>,
    artifact: Option<Vec<String>>,
    bindep_target: Option<String>,
    #[serde(default)]
    lib: bool,
}

fn yes() -> bool {
    true
}

/// A string, parsed as a [`RustVersion`]; anything else is an error, as cargo's deserializer
/// takes strings only.
fn rust_version<'de, D: Deserializer<'de>>(de: D) -> Result<Option<RustVersion>, D::Error> {
    let text: Option<String> = serde::Deserialize::deserialize(de)?;
    text.map(|it| it.parse().map_err(de::Error::custom)).transpose()
}

fn pubtime<'de, D: Deserializer<'de>>(de: D) -> Result<Option<String>, D::Error> {
    let text: Option<String> = serde::Deserialize::deserialize(de)?;
    match text {
        Some(text) if !is_pubtime(&text) => {
            Err(de::Error::custom(format!("invalid publish time `{text}`")))
        }
        it => Ok(it),
    }
}

/// Whether `text` is a time as cargo's `parse_pubtime` takes one: `%Y-%m-%dT%H:%M:%SZ`, padded,
/// of a day that exists.
fn is_pubtime(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 20 {
        return false;
    }
    let digits = |range: Range<usize>| -> Option<u32> {
        let part = &text[range];
        part.bytes().all(|it| it.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let shape = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':'), (19, b'Z')];
    if shape.iter().any(|&(at, byte)| bytes[at] != byte) {
        return false;
    }
    let parts = (digits(0..4), digits(5..7), digits(8..10), digits(11..13), digits(14..16));
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute)) = parts else {
        return false;
    };
    let Some(second) = digits(17..19) else { return false };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&day) && hour < 24 && minute < 60 && second < 60
}

#[derive(Deserialize)]
struct IndexPackageMinimum {
    name: String,
    vers: Version,
}

#[derive(Deserialize, Default)]
struct IndexPackageRustVersion {
    #[serde(default, deserialize_with = "rust_version")]
    rust_version: Option<RustVersion>,
}

#[derive(Deserialize, Default)]
struct IndexPackageV {
    v: Option<u32>,
}

/// Reads one line of an index file. An error is a line that has no name and version: cargo
/// leaves it out of the file's versions and of its cache.
pub fn parse_line(line: &[u8]) -> anyhow::Result<IndexLine> {
    let full =
        serde_json::from_slice::<IndexPackage>(line).map_err(anyhow::Error::from).and_then(|it| {
            let (v, yanked) = (it.v, it.yanked);
            Ok((v, yanked, entry(it)?))
        });
    let (v, yanked, entry, valid) = match full {
        Ok((v, yanked, entry)) => (v, yanked, entry, true),
        Err(err) => {
            let Ok(IndexPackageMinimum { name, vers }) = serde_json::from_slice(line) else {
                return Err(err);
            };
            let IndexPackageRustVersion { rust_version } =
                serde_json::from_slice(line).unwrap_or_default();
            let IndexPackageV { v } = serde_json::from_slice(line).unwrap_or_default();
            let entry = IndexEntry {
                name,
                version: vers,
                deps: Vec::new(),
                features: BTreeMap::new(),
                checksum: String::new(),
                links: None,
                rust_version,
                pubtime: None,
            };
            (v, None, entry, false)
        }
    };
    let v = v.unwrap_or(1);
    let status = if v > INDEX_V_MAX {
        Status::Unsupported(v)
    } else if !valid {
        Status::Invalid
    } else if yanked.unwrap_or(false) {
        Status::Yanked
    } else {
        Status::Candidate
    };
    Ok(IndexLine { entry, status })
}

fn entry(package: IndexPackage) -> anyhow::Result<IndexEntry> {
    let deps = package.deps.into_iter().map(dependency).collect::<anyhow::Result<Vec<_>>>()?;
    let mut features = package.features;
    for (name, values) in package.features2.into_iter().flatten() {
        features.entry(name).or_default().extend(values);
    }
    Ok(IndexEntry {
        name: package.name,
        version: package.vers,
        deps,
        features,
        checksum: package.cksum,
        links: package.links,
        rust_version: package.rust_version,
        pubtime: package.pubtime,
    })
}

/// As cargo's `registry_dependency_into_dep`, to the errors it can return.
fn dependency(dep: RegistryDependency) -> anyhow::Result<IndexDependency> {
    let registry = dep.registry.as_deref().map(crate::registry_source).transpose()?;
    let package = dep.package.unwrap_or_else(|| dep.name.clone());
    let req = VersionReq::parse(&dep.req).with_context(|| {
        format!("failed to parse the version requirement `{}` for dependency `{package}`", dep.req)
    })?;
    let kind = match dep.kind.as_deref().unwrap_or("") {
        "dev" => DepKind::Development,
        "build" => DepKind::Build,
        _ => DepKind::Normal,
    };
    let platform = dep.target.as_deref().map(Platform::from_str).transpose()?;
    let mut features = dep.features;
    features.retain(|it| !it.is_empty());
    let artifact = match dep.artifact {
        Some(kinds) => Some(artifact(kinds, dep.lib, dep.bindep_target)?),
        None => None,
    };
    Ok(IndexDependency {
        name_in_toml: dep.name,
        package,
        req,
        features,
        optional: dep.optional,
        default_features: dep.default_features,
        platform,
        kind,
        registry: registry.filter(|it| !it.is_crates_io()),
        public: dep.public.unwrap_or(false),
        artifact,
    })
}

/// As cargo's `Artifact::parse` without `-Zjson-target-spec`.
fn artifact(kinds: Vec<String>, lib: bool, target: Option<String>) -> anyhow::Result<Artifact> {
    for kind in &kinds {
        if !matches!(kind.as_str(), "bin" | "cdylib" | "staticlib") && !kind.starts_with("bin:") {
            bail!("'{kind}' is not a valid artifact specifier");
        }
    }
    if kinds.iter().any(|it| it == "bin") && kinds.iter().any(|it| it.starts_with("bin:")) {
        bail!("Cannot specify both 'bin' and 'bin:<name>' binary artifacts");
    }
    let mut unique = kinds.clone();
    unique.sort();
    unique.dedup();
    if unique.len() != kinds.len() {
        bail!("Found {} duplicate binary artifacts", kinds.len() - unique.len());
    }
    if let Some(target) = &target {
        let target = target.trim();
        if target.is_empty() {
            bail!("target was empty");
        }
        if target != "target" && target.ends_with(".json") {
            bail!("`.json` target specs require -Zjson-target-spec");
        }
    }
    Ok(Artifact { kinds, lib, target })
}

/// The path of a package's file in an index, from its name in lower case: `1/a`, `2/ab`,
/// `3/a/abc`, `ab/cd/abcd` (cargo-util's `make_dep_path`). With `prefix_only`, the directory
/// alone, which a `dl` template's `{prefix}` is.
pub fn dep_path(name: &str, prefix_only: bool) -> String {
    let (slash, rest) = if prefix_only { ("", "") } else { ("/", name) };
    match name.len() {
        1 => format!("1{slash}{rest}"),
        2 => format!("2{slash}{rest}"),
        3 => format!("3/{}{slash}{rest}", &name[..1]),
        _ => format!("{}/{}{slash}{rest}", &name[0..2], &name[2..4]),
    }
}

/// The versions of one package an index file has, each with its line, as cargo's
/// `Summaries`: from the file itself, or from cargo's cache of it.
#[derive(Clone, Debug, Default)]
pub struct Summaries {
    raw: Vec<u8>,
    versions: Vec<(Version, Range<usize>)>,
}

impl Summaries {
    /// The lines of an index file whose name and version can be read. Lines are split at `\n`
    /// only, as cargo splits them.
    pub fn from_index_file(raw: Vec<u8>) -> Summaries {
        let mut versions = Vec::new();
        let mut at = 0;
        while at < raw.len() {
            let end = raw[at..].iter().position(|&it| it == b'\n').map_or(raw.len(), |it| at + it);
            if let Ok(line) = parse_line(&raw[at..end]) {
                versions.push((line.entry.version, at..end));
            }
            at = end + 1;
        }
        Summaries { raw, versions }
    }

    /// A cache file cargo or blaze wrote, and the version of the index file it was made from.
    pub fn from_cache(contents: Vec<u8>) -> anyhow::Result<(Summaries, String)> {
        let (&first, rest) = contents.split_first().context("malformed cache")?;
        if first != CACHE_VERSION {
            bail!("looks like a different Cargo's cache, bailing out");
        }
        let schema = rest.get(..4).context("cache expected 4 bytes for index schema version")?;
        let schema = u32::from_le_bytes(schema.try_into().expect("four bytes"));
        if schema != INDEX_V_MAX {
            bail!("index schema version {schema} doesn't match the version I know ({INDEX_V_MAX})");
        }
        let mut parts = Split { data: &contents, at: 5 };
        let index_version = parts.next().context("malformed file")?;
        let index_version = std::str::from_utf8(&contents[index_version])?.to_owned();
        let mut versions = Vec::new();
        while let Some(version) = parts.next() {
            let version = Version::parse(std::str::from_utf8(&contents[version])?)?;
            let line = parts.next().context("a version in the cache has no line")?;
            versions.push((version, line));
        }
        Ok((Summaries { raw: contents, versions }, index_version))
    }

    /// The cache of these versions, for an index file of `index_version`, as cargo writes it.
    pub fn to_cache(&self, index_version: &str) -> Vec<u8> {
        let mut out = vec![CACHE_VERSION];
        out.extend(INDEX_V_MAX.to_le_bytes());
        out.extend(index_version.as_bytes());
        out.push(0);
        for (version, line) in &self.versions {
            out.extend(version.to_string().as_bytes());
            out.push(0);
            out.extend(&self.raw[line.clone()]);
            out.push(0);
        }
        out
    }

    pub fn len(&self) -> usize {
        self.versions.len()
    }

    /// Each version and its line, in the order of the file.
    pub fn lines(&self) -> impl Iterator<Item = (&Version, &[u8])> {
        self.versions.iter().map(|(version, line)| (version, &self.raw[line.clone()]))
    }

    /// The versions that match `req`, read; one whose line is not what it was when its version
    /// was read is left out, as cargo leaves it out.
    pub fn matching<'a>(&'a self, req: &'a VersionReq) -> impl Iterator<Item = IndexLine> + 'a {
        self.select(|version| req.matches(version))
    }

    /// The versions `keep` keeps, read as [`Summaries::matching`] reads them.
    pub fn select<'a>(
        &'a self,
        mut keep: impl FnMut(&Version) -> bool + 'a,
    ) -> impl Iterator<Item = IndexLine> + 'a {
        self.lines()
            .filter(move |(version, _)| keep(version))
            .filter_map(|(_, line)| parse_line(line).ok())
    }

    /// Every version, read.
    pub fn all(&self) -> impl Iterator<Item = IndexLine> + '_ {
        self.lines().filter_map(|(_, line)| parse_line(line).ok())
    }
}

/// The parts of a cache file between its zeros, as ranges of it.
struct Split<'a> {
    data: &'a [u8],
    at: usize,
}

impl Iterator for Split<'_> {
    type Item = Range<usize>;

    fn next(&mut self) -> Option<Range<usize>> {
        if self.at >= self.data.len() {
            return None;
        }
        let start = self.at;
        let end = self.data[start..]
            .iter()
            .position(|&it| it == 0)
            .map_or(self.data.len(), |it| start + it);
        self.at = end + 1;
        Some(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(line: &str) -> Status {
        parse_line(line.as_bytes()).unwrap().status
    }

    #[test]
    fn paths() {
        assert_eq!(dep_path("a", false), "1/a");
        assert_eq!(dep_path("ab", false), "2/ab");
        assert_eq!(dep_path("abc", false), "3/a/abc");
        assert_eq!(dep_path("Abc", false), "3/A/Abc");
        assert_eq!(dep_path("AbCd", false), "Ab/Cd/AbCd");
        assert_eq!(dep_path("aBcDe", false), "aB/cD/aBcDe");
        assert_eq!(dep_path("abc", true), "3/a");
        assert_eq!(dep_path("aBcDe", true), "aB/cD");
    }

    #[test]
    fn lines() {
        let line = r#"{"name":"a","vers":"0.0.1","deps":[{"name":"b2","package":"b","req":"^1","features":["","x"],"optional":true,"default_features":false,"target":"cfg(unix)","kind":"build","registry":"sparse+https://example.com/index/"}],"cksum":"bae3","features":{"f":["b2/x"]},"features2":{"f":["dep:b2"],"g":[]},"links":"a-sys","rust_version":"1.70","pubtime":"2025-11-12T19:30:12Z","unknown":[1,2]}"#;
        let IndexLine { entry, status: read } = parse_line(line.as_bytes()).unwrap();
        assert_eq!(read, Status::Candidate);
        assert_eq!(entry.features["f"], ["b2/x", "dep:b2"]);
        assert_eq!(entry.features["g"], Vec::<String>::new());
        assert_eq!(entry.rust_version.unwrap().to_string(), "1.70");
        let dep = &entry.deps[0];
        assert_eq!((dep.name_in_toml.as_str(), dep.package.as_str()), ("b2", "b"));
        assert_eq!(dep.features, ["x"]);
        assert_eq!(dep.kind, DepKind::Build);
        assert!(dep.optional && !dep.default_features);
        assert_eq!(dep.platform.as_ref().unwrap().to_string(), "cfg(unix)");
        assert_eq!(
            dep.registry.as_ref().unwrap().url().as_str(),
            "sparse+https://example.com/index/"
        );

        let base = r#""name":"a","vers":"1.0.0","deps":[],"cksum":"00""#;
        assert_eq!(status(&format!("{{{base},\"yanked\":true}}")), Status::Yanked);
        assert_eq!(status(&format!("{{{base},\"v\":2}}")), Status::Candidate);
        assert_eq!(status(&format!("{{{base},\"v\":3}}")), Status::Unsupported(3));
        // Invalid past its name and version, and kept as such with what can still be read.
        let bad = parse_line(br#"{"name":"a","vers":"1.0.0","deps":[{"name":"b","req":"not a req"}],"cksum":"00","rust_version":"1.60"}"#).unwrap();
        assert_eq!(bad.status, Status::Invalid);
        assert_eq!(bad.entry.rust_version.unwrap().to_string(), "1.60");
        assert_eq!(
            status(&format!("{{{base},\"pubtime\":\"2025-11-12T19:30:12-04\"}}")),
            Status::Invalid
        );
        assert_eq!(
            status(&format!("{{{base},\"pubtime\":\"2025-02-29T19:30:12Z\"}}")),
            Status::Invalid
        );
        assert_eq!(status(&format!("{{{base},\"rust_version\":\"1.70-beta\"}}")), Status::Invalid);
        assert_eq!(status(&format!("{{{base},\"rust_version\":\"^1.70\"}}")), Status::Invalid);
        assert_eq!(status(r#"{"name":"a","vers":"1.0.0","cksum":"00"}"#), Status::Invalid);
        assert_eq!(status(&format!("{{{base},\"cksum\":\"01\"}}")), Status::Invalid);
        assert_eq!(
            status(&format!(
                "{{{base},\"deps\":[{{\"name\":\"b\",\"req\":\"1\",\"target\":\"cfg(\"}}]}}"
            )),
            Status::Invalid
        );
        assert_eq!(
            status(&format!(
                "{{{base},\"deps\":[{{\"name\":\"b\",\"req\":\"1\",\"artifact\":[\"rlib\"]}}]}}"
            )),
            Status::Invalid
        );
        // A wrong `v` does not keep the version from being read.
        assert_eq!(status(&format!("{{{base},\"v\":-1}}")), Status::Invalid);
        assert!(parse_line(br#"{"name":"a","vers":"one"}"#).is_err());
        assert!(parse_line(b"").is_err());
    }

    #[test]
    fn crates_io_drops_its_own_registry() {
        let line = r#"{"name":"a","vers":"1.0.0","deps":[{"name":"b","req":"1","registry":"https://github.com/rust-lang/crates.io-index"}],"cksum":"00"}"#;
        assert_eq!(parse_line(line.as_bytes()).unwrap().entry.deps[0].registry, None);
    }

    #[test]
    fn cache_round_trip() {
        let file = concat!(
            r#"{"name":"a","vers":"1.0.0","deps":[],"cksum":"00"}"#,
            "\n\n",
            r#"{"name":"a","vers":"1.0.1+meta","deps":[],"cksum":"01","yanked":true}"#,
            "\nnot json\n",
            r#"{"name":"a","vers":"2.0.0","deps":[{"name":"b","req":"x"}],"cksum":"02"}"#,
            "\n",
        );
        let summaries = Summaries::from_index_file(file.as_bytes().to_vec());
        let versions: Vec<String> = summaries.lines().map(|(it, _)| it.to_string()).collect();
        assert_eq!(versions, ["1.0.0", "1.0.1+meta", "2.0.0"]);
        let cache = summaries.to_cache("etag: \"abc\"");
        assert_eq!(&cache[..5], &[3, 2, 0, 0, 0]);
        let (read, index_version) = Summaries::from_cache(cache.clone()).unwrap();
        assert_eq!(index_version, "etag: \"abc\"");
        assert_eq!(read.to_cache("etag: \"abc\""), cache);
        let req = VersionReq::parse("^1").unwrap();
        let statuses: Vec<Status> = read.matching(&req).map(|it| it.status).collect();
        assert_eq!(statuses, [Status::Candidate, Status::Yanked]);
        assert!(Summaries::from_cache(vec![2, 2, 0, 0, 0]).is_err());
        assert!(Summaries::from_cache(vec![3, 3, 0, 0, 0, b'x', 0]).is_err());
    }
}
