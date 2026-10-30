//! A list of pairs of an enum and a slice of strings, behind a reference.

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Plain,
    Marked,
    Folded,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Plain => "plain",
            Kind::Marked => "marked text",
            Kind::Folded => "folded",
        }
    }

    pub fn list() -> &'static [(Kind, &'static [&'static str])] {
        &[(Kind::Plain, &["txt"]), (Kind::Marked, &["md", "markdown"]), (Kind::Folded, &[])]
    }
}

pub fn run() -> Vec<(String, usize)> {
    Kind::list()
        .iter()
        .map(|(kind, endings)| (format!("{kind:?}{endings:?}"), kind.name().len()))
        .collect()
}
