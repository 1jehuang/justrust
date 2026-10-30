//! What clippy declares of each lint: its name, its group and so its default level, and its
//! description; the groups by name, as `#[warn(clippy::pedantic)]` names them; and the lints it
//! renamed and removed. The table is clippy's own, written from its source
//! (`harness/clippy_registry.py`).

mod clippy;

use serde::{Deserialize, Serialize};

use crate::Level;

/// A lint of clippy's.
#[derive(Debug, PartialEq, Eq)]
pub struct Info {
    /// Lowercase, without `clippy::`.
    pub name: &'static str,
    pub group: Group,
    pub desc: &'static str,
    /// The release it came in, as clippy's `#[clippy::version]` says.
    pub version: &'static str,
}

/// clippy's lint groups (`LintCategory`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Group {
    Cargo,
    Complexity,
    Correctness,
    Nursery,
    Pedantic,
    Perf,
    Restriction,
    Style,
    Suspicious,
}

impl Group {
    pub const ALL: [Group; 9] = [
        Group::Cargo,
        Group::Complexity,
        Group::Correctness,
        Group::Nursery,
        Group::Pedantic,
        Group::Perf,
        Group::Restriction,
        Group::Style,
        Group::Suspicious,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Group::Cargo => "cargo",
            Group::Complexity => "complexity",
            Group::Correctness => "correctness",
            Group::Nursery => "nursery",
            Group::Pedantic => "pedantic",
            Group::Perf => "perf",
            Group::Restriction => "restriction",
            Group::Style => "style",
            Group::Suspicious => "suspicious",
        }
    }

    /// The level of its lints when nothing sets one: `declare_clippy_lint!`'s.
    pub fn default_level(self) -> Level {
        match self {
            Group::Correctness => Level::Deny,
            Group::Complexity | Group::Perf | Group::Style | Group::Suspicious => Level::Warn,
            Group::Cargo | Group::Nursery | Group::Pedantic | Group::Restriction => Level::Allow,
        }
    }

    /// Whether `clippy::all` (`clippy::default`) has its lints.
    pub fn in_all(self) -> bool {
        self.default_level() != Level::Allow
    }
}

impl Info {
    pub fn default_level(&self) -> Level {
        self.group.default_level()
    }

    /// The page of clippy's documentation for it, as the pinned nightly's clippy links it
    /// (`clippy_utils::diagnostics::docs_link`, a nightly's channel being `main`).
    pub fn docs_url(&self) -> String {
        format!("https://rust-lang.github.io/rust-clippy/main/index.html#{}", self.name)
    }
}

/// Every lint of clippy's, by name.
pub fn lints() -> &'static [Info] {
    clippy::LINTS
}

/// The lint named `name`, without `clippy::`.
pub fn lint(name: &str) -> Option<&'static Info> {
    let at = clippy::LINTS.binary_search_by(|it| it.name.cmp(name)).ok()?;
    Some(&clippy::LINTS[at])
}

/// What a name after `clippy::` in a lint attribute or flag stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Named {
    Lint(&'static Info),
    Group(Group),
    /// `clippy::all`, or `clippy::default`: every lint of the groups that are not allowed.
    All,
    /// A lint clippy renamed; to its name now, which may be rustc's.
    Renamed(&'static str),
    /// A lint clippy removed, and why.
    Removed(&'static str),
}

/// What `name`, after `clippy::`, stands for, if clippy knows it.
pub fn named(name: &str) -> Option<Named> {
    if let Some(info) = lint(name) {
        return Some(Named::Lint(info));
    }
    if name == "all" || name == "default" {
        return Some(Named::All);
    }
    if let Some(group) = Group::ALL.into_iter().find(|it| it.name() == name) {
        return Some(Named::Group(group));
    }
    if let Some((_, new)) = clippy::RENAMED.iter().find(|(old, _)| *old == name) {
        return Some(Named::Renamed(new));
    }
    if let Some((_, why)) = clippy::REMOVED.iter().find(|(old, _)| *old == name) {
        return Some(Named::Removed(why));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_clippys() {
        let names: Vec<_> = lints().iter().map(|it| it.name).collect();
        assert!(names.is_sorted(), "binary search needs the table sorted");
        assert!(names.len() > 800);
        let correctness = lints().iter().filter(|it| it.group == Group::Correctness).count();
        assert_eq!(correctness, 67);
        assert_eq!(lint("needless_bool").map(|it| it.group), Some(Group::Complexity));
        assert_eq!(lint("eq_op").map(Info::default_level), Some(Level::Deny));
        assert_eq!(
            lint("collapsible_if").unwrap().docs_url(),
            "https://rust-lang.github.io/rust-clippy/main/index.html#collapsible_if"
        );
        assert_eq!(named("pedantic"), Some(Named::Group(Group::Pedantic)));
        assert_eq!(named("all"), Some(Named::All));
        assert_eq!(named("blacklisted_name"), Some(Named::Renamed("clippy::disallowed_names")));
        assert!(matches!(named("unused_collect"), Some(Named::Removed(_))));
        assert_eq!(named("no_such_lint"), None);
    }
}
