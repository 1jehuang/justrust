//! The flags of a subcommand, read as cargo's command line reads them: `--flag value`,
//! `--flag=value`, `-Fvalue`, and the flags every cargo command takes.

/// A flag a subcommand takes, by each of its names, and whether it takes a value.
pub(super) struct Flag {
    pub(super) names: &'static [&'static str],
    pub(super) valued: bool,
}

pub(super) const fn switch(names: &'static [&'static str]) -> Flag {
    Flag { names, valued: false }
}

pub(super) const fn valued(names: &'static [&'static str]) -> Flag {
    Flag { names, valued: true }
}

/// The flags of every cargo command.
const GLOBAL: &[Flag] = &[
    switch(&["-v", "--verbose", "-vv"]),
    switch(&["-q", "--quiet"]),
    valued(&["--color"]),
    valued(&["--config"]),
    valued(&["-Z"]),
    switch(&["--frozen"]),
    switch(&["--locked"]),
    switch(&["--offline"]),
    switch(&["-h", "--help"]),
];

/// What was typed after the subcommand.
pub(super) struct Args<'a> {
    typed: &'a [String],
}

/// The flags read, by the first of each flag's names, and the other words.
#[derive(Debug, Default)]
pub(super) struct Read {
    flags: Vec<(&'static str, Option<String>)>,
    /// The words that are no flags, and every word after `--`.
    pub(super) positional: Vec<String>,
}

impl<'a> Args<'a> {
    pub(super) fn new(typed: &'a [String]) -> Args<'a> {
        Args { typed }
    }

    /// The flags of `flags` and cargo's global ones, or the line cargo refuses the command
    /// line with.
    pub(super) fn read(&self, flags: &[Flag]) -> Result<Read, String> {
        let known = |name: &str| flags.iter().chain(GLOBAL).find(|it| it.names.contains(&name));
        let mut read = Read::default();
        let mut words = self.typed.iter();
        while let Some(word) = words.next() {
            if word == "--" {
                read.positional.extend(words.cloned());
                break;
            }
            if !word.starts_with('-') || word == "-" {
                read.positional.push(word.clone());
                continue;
            }
            let (name, joined) = match word.split_once('=') {
                Some((name, value)) if name.starts_with("--") => (name, Some(value.to_owned())),
                _ if known(word).is_some() => (word.as_str(), None),
                // `-Fa,b`, `-Zflag`.
                _ if !word.starts_with("--")
                    && word.len() > 2
                    && word.get(..2).and_then(known).is_some_and(|it| it.valued) =>
                {
                    (&word[..2], Some(word[2..].to_owned()))
                }
                _ => (word.as_str(), None),
            };
            let Some(flag) = known(name) else {
                return Err(format!("unexpected argument '{word}' found"));
            };
            let value = match (flag.valued, joined) {
                (true, Some(value)) => Some(value),
                (true, None) => match words.next() {
                    Some(value) => Some(value.clone()),
                    None => {
                        return Err(format!(
                            "a value is required for '{name} <VALUE>' but none was supplied"
                        ));
                    }
                },
                (false, Some(_)) => {
                    return Err(format!("unexpected value for '{name}': it takes none"));
                }
                (false, None) => None,
            };
            read.flags.push((flag.key(), value));
        }
        Ok(read)
    }
}

impl Flag {
    /// The name a flag is read back by: its long one.
    fn key(&self) -> &'static str {
        self.names.iter().find(|it| it.starts_with("--")).unwrap_or(&self.names[0])
    }
}

impl Read {
    /// Whether the flag of which `name` is the long name was given.
    pub(super) fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(it, _)| *it == name)
    }

    /// The values of the flag of which `name` is the long name, in the order given.
    pub(super) fn values(&self, name: &str) -> Vec<String> {
        let given = self.flags.iter().filter(|(it, _)| *it == name);
        given.filter_map(|(_, value)| value.clone()).collect()
    }

    /// The last value of the flag of which `name` is the long name.
    pub(super) fn value(&self, name: &str) -> Option<String> {
        self.values(name).pop()
    }

    /// `--features`' values split as cargo splits them, by commas and spaces.
    pub(super) fn features(&self) -> Vec<String> {
        let values = self.values("--features");
        let split = values.iter().flat_map(|it| it.split([',', ' ']));
        split.filter(|it| !it.is_empty()).map(str::to_owned).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str, flags: &[Flag]) -> Result<Read, String> {
        let typed: Vec<String> = text.split_whitespace().map(str::to_owned).collect();
        Args::new(&typed).read(flags)
    }

    #[test]
    fn both_spellings_and_cargos_own_flags() {
        let flags = [valued(&["-F", "--features"]), switch(&["--no-deps"])];
        let it = read("--no-deps -Fa,b --features=c --features d -q --config x=1", &flags).unwrap();
        assert!(it.has("--no-deps") && it.has("--quiet"));
        assert_eq!(it.features(), ["a", "b", "c", "d"]);
        assert_eq!(it.values("--config"), ["x=1"]);
        assert!(read("-vv", &flags).unwrap().has("--verbose"));
        let it = read("a -- b --c", &flags).unwrap();
        assert_eq!(it.positional, ["a", "b", "--c"]);
    }

    #[test]
    fn what_cargo_refuses() {
        assert!(read("--frobnicate", &[]).unwrap_err().contains("'--frobnicate'"));
        assert!(read("--features", &[valued(&["--features"])]).unwrap_err().contains("value"));
        assert!(read("--quiet=yes", &[]).is_err());
    }
}
