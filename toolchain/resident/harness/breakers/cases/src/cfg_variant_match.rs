//! An enum with a variant under `#[cfg]` and variants around a second enum, matched on both
//! at once; the values come from a promoted slice whose elements are under `#[cfg]` too.

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum Case {
    Upper,
    Mixed,
}

#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum Field {
    Unsorted,
    Name(Case),
    Extension(Case),
    Size,
    #[cfg(unix)]
    Inode,
    #[cfg(windows)]
    Attributes,
    Modified,
    Kind,
    Hidden(Case),
}

fn all<'a>() -> &'a [Field] {
    &[
        Field::Name(Case::Mixed),
        Field::Name(Case::Upper),
        Field::Hidden(Case::Mixed),
        Field::Hidden(Case::Upper),
        Field::Size,
        Field::Extension(Case::Mixed),
        Field::Extension(Case::Upper),
        Field::Modified,
        #[cfg(unix)]
        Field::Inode,
        #[cfg(windows)]
        Field::Attributes,
        Field::Kind,
        Field::Unsorted,
    ]
}

#[inline(never)]
fn name(field: &Field) -> Option<String> {
    Some(match field {
        Field::Name(Case::Mixed) => "name".to_owned(),
        Field::Name(Case::Upper) => "Name".to_owned(),
        Field::Hidden(Case::Mixed) => ".name".to_owned(),
        Field::Hidden(Case::Upper) => ".Name".to_owned(),
        Field::Size => "size".to_owned(),
        Field::Extension(Case::Mixed) => "ext".to_owned(),
        Field::Extension(Case::Upper) => "Ext".to_owned(),
        Field::Modified => "date".to_owned(),
        #[cfg(unix)]
        Field::Inode => "inode".to_owned(),
        #[cfg(windows)]
        Field::Attributes => "attributes".to_owned(),
        Field::Kind => "type".to_owned(),
        Field::Unsorted => "none".to_owned(),
    })
}

pub fn run() -> Vec<Option<String>> {
    all().iter().map(name).collect()
}
