//! The level a lint is at in a place: from the attributes around it, the command line rustc
//! is given (cargo's `[lints]`, `RUSTFLAGS`, then what `cargo clippy --` is given), the
//! `warnings` group and `--cap-lints`, as rustc decides it; and the notes rustc adds to say
//! where the level came from.
//!
//! Follows rustc's `compiler/rustc_lint/src/levels.rs` (`add_command_line`, `insert_spec`),
//! `compiler/rustc_middle/src/lint.rs` (`reveal_actual_level_spec`,
//! `explain_lint_level_source`) and `compiler/rustc_session/src/config.rs`
//! (`get_cmd_lint_options`) at rust-lang/rust 12433c307b5a497388ffe2f4e49dbf1ab8c71fa0
//! (MIT or Apache-2.0).

use serde::{Deserialize, Serialize};

use crate::{
    diag::{Diagnostic, MultiSpan, Span, Sublevel},
    registry::{self, Group, Info, Named},
};

/// A lint level, in rustc's order: a level above another wins over it where one is capped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    Allow,
    Expect,
    Warn,
    ForceWarn,
    Deny,
    Forbid,
}

impl Level {
    /// As an attribute names it.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Allow => "allow",
            Level::Expect => "expect",
            Level::Warn => "warn",
            Level::ForceWarn => "force-warn",
            Level::Deny => "deny",
            Level::Forbid => "forbid",
        }
    }

    /// The level an attribute named `name` sets.
    pub fn of_attr(name: &str) -> Option<Level> {
        Some(match name {
            "allow" => Level::Allow,
            "expect" => Level::Expect,
            "warn" => Level::Warn,
            "deny" => Level::Deny,
            "forbid" => Level::Forbid,
            _ => return None,
        })
    }

    /// The flag of rustc's that sets it.
    pub fn to_cmd_flag(self) -> &'static str {
        match self {
            Level::Allow => "-A",
            Level::Expect | Level::Warn => "-W",
            Level::ForceWarn => "--force-warn",
            Level::Deny => "-D",
            Level::Forbid => "-F",
        }
    }

    /// The level `--cap-lints` is given as.
    fn of_cap(name: &str) -> Option<Level> {
        Some(match name {
            "allow" => Level::Allow,
            "warn" => Level::Warn,
            "force-warn" => Level::ForceWarn,
            "deny" => Level::Deny,
            "forbid" => Level::Forbid,
            _ => return None,
        })
    }
}

/// What a level set applies to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// A lint of clippy's, without `clippy::`.
    Lint(String),
    Group(Group),
    /// `clippy::all`.
    All,
    /// The `warnings` group: every lint at `warn`, when it is.
    Warnings,
    /// A lint of rustc's, by name.
    Rustc(String),
}

impl Target {
    /// What `name`, as an attribute or flag writes it, sets the level of, if it is a lint this
    /// crate knows. A renamed lint stands for its new name.
    pub fn of(name: &str) -> Option<Target> {
        let name = name.replace('-', "_");
        if name == "warnings" {
            return Some(Target::Warnings);
        }
        let Some(rest) = name.strip_prefix("clippy::") else {
            return (!name.contains("::")).then_some(Target::Rustc(name));
        };
        Some(match registry::named(rest)? {
            Named::Lint(info) => Target::Lint(info.name.to_owned()),
            Named::Group(group) => Target::Group(group),
            Named::All => Target::All,
            Named::Renamed(new) => return Target::of(new),
            Named::Removed(_) => return None,
        })
    }

    fn covers(&self, lint: Lint<'_>) -> bool {
        match (self, lint) {
            (Target::Lint(name), Lint::Clippy(info)) => name == info.name,
            (Target::Group(group), Lint::Clippy(info)) => *group == info.group,
            (Target::All, Lint::Clippy(info)) => info.group.in_all(),
            (Target::Warnings, Lint::Warnings) => true,
            (Target::Rustc(name), Lint::Rustc(lint)) => name == lint,
            _ => false,
        }
    }
}

/// A lint whose level is asked for.
#[derive(Clone, Copy, Debug)]
pub enum Lint<'a> {
    Clippy(&'a Info),
    /// The `warnings` group, which rustc asks for as a lint.
    Warnings,
    /// A lint of rustc's, as `unfulfilled_lint_expectations`.
    Rustc(&'a str),
}

impl Lint<'_> {
    /// As rustc names it in its notes.
    pub fn name(&self) -> String {
        match self {
            Lint::Clippy(info) => format!("clippy::{}", info.name),
            Lint::Warnings => "warnings".to_owned(),
            Lint::Rustc(name) => (*name).to_owned(),
        }
    }

    fn default_level(&self) -> Level {
        match self {
            Lint::Clippy(info) => info.default_level(),
            Lint::Warnings | Lint::Rustc(_) => Level::Warn,
        }
    }
}

/// Where a level came from: rustc's `LintLevelSource`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Source {
    /// The lint's own.
    Default,
    /// A flag, by the name it was given (`-` as `_`) and its level.
    CommandLine { flag: String, level: Level },
    /// An attribute: the name in it (`clippy::pedantic`), the span of that name and the
    /// attribute's `reason`.
    Node { name: String, span: Span, reason: Option<String> },
}

/// A level and where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    pub level: Level,
    pub source: Source,
}

/// The lint flags of a rustc command line, in order: rustc's `lint_opts` and `lint_cap`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CommandLine {
    pub opts: Vec<(String, Level)>,
    pub cap: Option<Level>,
}

impl CommandLine {
    /// The lint flags of `args`, a rustc command line; its other flags are passed over.
    pub fn parse(args: &[String]) -> Result<CommandLine, String> {
        let mut flags = CommandLine::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let long = [
                ("--allow", Level::Allow),
                ("--warn", Level::Warn),
                ("--force-warn", Level::ForceWarn),
                ("--deny", Level::Deny),
                ("--forbid", Level::Forbid),
            ];
            let short = [
                ("-A", Level::Allow),
                ("-W", Level::Warn),
                ("-D", Level::Deny),
                ("-F", Level::Forbid),
            ];
            let mut value = |arg: &str, flag: &str| -> Result<Option<String>, String> {
                if arg == flag {
                    let next = args.next().ok_or_else(|| format!("`{flag}` needs a lint"))?;
                    return Ok(Some(next.clone()));
                }
                let rest = arg.strip_prefix(flag);
                let rest = match flag.starts_with("--") {
                    true => rest.and_then(|it| it.strip_prefix('=')),
                    false => rest,
                };
                Ok(rest.map(str::to_owned))
            };
            if let Some(cap) = value(arg, "--cap-lints")? {
                flags.cap = Some(
                    Level::of_cap(&cap).ok_or_else(|| format!("unknown lint level: `{cap}`"))?,
                );
                continue;
            }
            let mut found = None;
            for (flag, level) in long.into_iter().chain(short) {
                if let Some(name) = value(arg, flag)? {
                    found = Some((name, level));
                    break;
                }
            }
            if let Some((name, level)) = found {
                flags.opts.push((name.replace('-', "_"), level));
            }
        }
        Ok(flags)
    }

    /// What `cargo clippy --` is given after `--`: what `-W`, `-A`, `-D`, `-F` and
    /// `--force-warn` there name, on top of `self`.
    pub fn extend(&mut self, other: CommandLine) {
        self.opts.extend(other.opts);
        if other.cap.is_some() {
            self.cap = other.cap;
        }
    }
}

/// The flags cargo gives rustc for a package's `[lints]`: by priority, then by name from last
/// to first so that a group named `all` comes last.
/// Follows cargo's `lints_to_rustflags` (`src/workspace/parser/mod.rs` at
/// rust-lang/cargo cad07cf8e597397fff5d0f6cb79c9f870c0b1dd2).
pub fn of_manifest(lints: &[(String, String, Level, i8)]) -> CommandLine {
    let mut flags: Vec<(i8, std::cmp::Reverse<&str>, String, Level)> = lints
        .iter()
        .filter(|(tool, ..)| tool != "cargo")
        .map(|(tool, name, level, priority)| {
            let named = match tool.as_str() {
                "rust" => name.clone(),
                tool => format!("{tool}::{name}"),
            };
            (*priority, std::cmp::Reverse(name.as_str()), named, *level)
        })
        .collect();
    flags.sort_by(|a, b| (a.0, a.1, flag_of(a.3), &a.2).cmp(&(b.0, b.1, flag_of(b.3), &b.2)));
    let opts = flags.into_iter().map(|(_, _, name, level)| (name.replace('-', "_"), level));
    CommandLine { opts: opts.collect(), cap: None }
}

/// cargo's flag for a level, which it sorts flags of one name by.
pub(crate) fn flag_of(level: Level) -> String {
    let flag = match level {
        Level::Forbid => "--forbid",
        Level::Deny => "--deny",
        Level::Warn | Level::ForceWarn | Level::Expect => "--warn",
        Level::Allow => "--allow",
    };
    flag.to_owned()
}

/// An attribute's level set on one lint name, as the lint visitor finds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Set {
    pub target: Target,
    pub spec: Spec,
}

/// The levels of a crate: its command line, with the attributes of the place asked about.
#[derive(Clone, Debug, Default)]
pub struct Levels {
    sets: Vec<(Target, Spec)>,
    cap: Option<Level>,
}

impl Levels {
    pub fn new(command_line: &CommandLine) -> Levels {
        let sets = command_line.opts.iter().filter_map(|(name, level)| {
            let target = Target::of(name)?;
            let source = Source::CommandLine { flag: name.clone(), level: *level };
            Some((target, Spec { level: *level, source }))
        });
        Levels { sets: sets.collect(), cap: command_line.cap }
    }

    /// The level of `lint` where the attributes `sets` are, outermost first and each node's in
    /// their order: rustc's `reveal_actual_level_spec`.
    pub fn of(&self, lint: Lint<'_>, sets: &[Set]) -> Spec {
        let mut spec = self
            .probe(lint, sets)
            .unwrap_or_else(|| Spec { level: lint.default_level(), source: Source::Default });
        if spec.level == Level::Warn
            && let Some(warnings) = self.probe(Lint::Warnings, sets)
            && matches!(warnings.level, Level::Deny | Level::Forbid | Level::Allow | Level::Expect)
        {
            spec = warnings;
        }
        let forced = matches!(spec.source, Source::CommandLine { level: Level::ForceWarn, .. });
        if !forced && let Some(cap) = self.cap {
            spec.level = spec.level.min(cap);
        }
        spec
    }

    /// The level set for `lint`, if one is: each set in order, the command line's first, a
    /// set of `forbid` or of `--force-warn` kept over those after it.
    fn probe(&self, lint: Lint<'_>, sets: &[Set]) -> Option<Spec> {
        let mut found: Option<Spec> = None;
        let attrs = sets.iter().map(|it| (&it.target, &it.spec));
        for (target, spec) in self.sets.iter().map(|(target, spec)| (target, spec)).chain(attrs) {
            if !target.covers(lint) {
                continue;
            }
            if let Some(current) = &found {
                let sticky = current.level == Level::Forbid
                    || (current.level == Level::ForceWarn
                        && matches!(current.source, Source::CommandLine { .. }));
                if sticky && spec.level != Level::Forbid {
                    continue;
                }
            }
            found = Some(spec.clone());
        }
        found
    }
}

/// Adds the notes rustc adds of where `spec`'s level came from: rustc's
/// `explain_lint_level_source`. Notes said once a crate are marked as such.
pub fn explain(diagnostic: &mut Diagnostic, lint: Lint<'_>, spec: &Spec) {
    let name = lint.name();
    let level = spec.level;
    if level == Level::Allow {
        return;
    }
    match &spec.source {
        Source::Default => {
            let note = format!("`#[{}({name})]` on by default", level.as_str());
            diagnostic.sub(Sublevel::Note, note, MultiSpan::default(), true);
        }
        Source::CommandLine { flag: value, level: original } => {
            let flag = original.to_cmd_flag();
            let hyphens = name.replace('_', "-");
            if *value == name {
                let note = format!("requested on the command line with `{flag} {hyphens}`");
                diagnostic.sub(Sublevel::Note, note, MultiSpan::default(), true);
            } else {
                let value = value.replace('_', "-");
                let note = format!("`{flag} {hyphens}` implied by `{flag} {value}`");
                diagnostic.sub(Sublevel::Note, note, MultiSpan::default(), true);
                if matches!(original, Level::Warn | Level::Deny) {
                    let help = format!("to override `{flag} {value}` add `#[allow({name})]`");
                    diagnostic.sub(Sublevel::Help, help, MultiSpan::default(), true);
                }
            }
        }
        Source::Node { name: attr_name, span, reason } => {
            if let Some(reason) = reason {
                diagnostic.note(reason.clone());
            }
            let defined = "the lint level is defined here";
            diagnostic.sub(Sublevel::Note, defined, MultiSpan::of(span.clone()), true);
            if *attr_name != name {
                let level = level.as_str();
                let note = format!("`#[{level}({name})]` implied by `#[{level}({attr_name})]`");
                diagnostic.sub(Sublevel::Note, note, MultiSpan::default(), true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(args: &str) -> Vec<String> {
        args.split_whitespace().map(str::to_owned).collect()
    }

    fn info(name: &str) -> &'static Info {
        registry::lint(name).unwrap()
    }

    #[test]
    fn flags_as_rustc_reads_them() {
        let flags = CommandLine::parse(&args(
            "-W clippy::all -Dwarnings --allow=clippy::needless-bool --cap-lints warn -O",
        ))
        .unwrap();
        assert_eq!(
            flags.opts,
            vec![
                ("clippy::all".to_owned(), Level::Warn),
                ("warnings".to_owned(), Level::Deny),
                ("clippy::needless_bool".to_owned(), Level::Allow),
            ]
        );
        assert_eq!(flags.cap, Some(Level::Warn));
    }

    #[test]
    fn the_last_flag_wins_but_forbid_stays() {
        let levels = Levels::new(
            &CommandLine::parse(&args("-F clippy::needless_bool -A clippy::all")).unwrap(),
        );
        assert_eq!(levels.of(Lint::Clippy(info("needless_bool")), &[]).level, Level::Forbid);
        let levels = Levels::new(
            &CommandLine::parse(&args("-D clippy::all -A clippy::needless_bool")).unwrap(),
        );
        assert_eq!(levels.of(Lint::Clippy(info("needless_bool")), &[]).level, Level::Allow);
        assert_eq!(levels.of(Lint::Clippy(info("collapsible_if")), &[]).level, Level::Deny);
    }

    #[test]
    fn warnings_takes_a_warning_only() {
        let levels = Levels::new(&CommandLine::parse(&args("-D warnings")).unwrap());
        let spec = levels.of(Lint::Clippy(info("collapsible_if")), &[]);
        assert_eq!(spec.level, Level::Deny);
        assert_eq!(
            spec.source,
            Source::CommandLine { flag: "warnings".to_owned(), level: Level::Deny }
        );
        assert_eq!(levels.of(Lint::Clippy(info("map_unwrap_or")), &[]).level, Level::Allow);
    }

    #[test]
    fn an_attribute_overrides_the_command_line() {
        let levels = Levels::new(&CommandLine::parse(&args("-D clippy::needless_bool")).unwrap());
        let span = Span { file: "src/lib.rs".to_owned(), lo: 0, hi: 1, expansion: None };
        let set = Set {
            target: Target::Group(Group::Complexity),
            spec: Spec {
                level: Level::Allow,
                source: Source::Node { name: "clippy::complexity".to_owned(), span, reason: None },
            },
        };
        assert_eq!(levels.of(Lint::Clippy(info("needless_bool")), &[set]).level, Level::Allow);
    }

    #[test]
    fn manifest_flags_in_cargos_order() {
        let lints = vec![
            ("clippy".to_owned(), "all".to_owned(), Level::Warn, -1),
            ("clippy".to_owned(), "needless_bool".to_owned(), Level::Allow, 0),
            ("clippy".to_owned(), "empty_enums".to_owned(), Level::Warn, 0),
            ("rust".to_owned(), "unsafe_code".to_owned(), Level::Deny, 0),
            ("cargo".to_owned(), "x".to_owned(), Level::Warn, 0),
        ];
        let names: Vec<_> = of_manifest(&lints).opts.into_iter().map(|it| it.0).collect();
        assert_eq!(
            names,
            ["clippy::all", "unsafe_code", "clippy::needless_bool", "clippy::empty_enums"]
        );
    }

    #[test]
    fn the_notes_rustc_adds() {
        let notes = |spec: Spec| {
            let mut diagnostic = Diagnostic {
                lint: "clippy::needless_bool".to_owned(),
                level: spec.level,
                message: String::new(),
                span: MultiSpan::default(),
                children: Vec::new(),
                suggestions: Vec::new(),
                suggestions_disabled: false,
            };
            explain(&mut diagnostic, Lint::Clippy(info("needless_bool")), &spec);
            diagnostic.children.into_iter().map(|it| it.message).collect::<Vec<_>>()
        };
        assert_eq!(
            notes(Spec { level: Level::Warn, source: Source::Default }),
            ["`#[warn(clippy::needless_bool)]` on by default"]
        );
        assert_eq!(
            notes(Spec {
                level: Level::Deny,
                source: Source::CommandLine { flag: "warnings".to_owned(), level: Level::Deny }
            }),
            [
                "`-D clippy::needless-bool` implied by `-D warnings`",
                "to override `-D warnings` add `#[allow(clippy::needless_bool)]`"
            ]
        );
        assert_eq!(
            notes(Spec {
                level: Level::Warn,
                source: Source::CommandLine {
                    flag: "clippy::needless_bool".to_owned(),
                    level: Level::Warn
                }
            }),
            ["requested on the command line with `-W clippy::needless-bool`"]
        );
    }
}
