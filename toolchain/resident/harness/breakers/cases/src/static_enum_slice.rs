//! toml_parser passes `&[Expected::Literal("'")]` where a `&'static [Expected]` is kept.

#[derive(Copy, Clone, Debug)]
#[allow(dead_code)]
enum Expected {
    Literal(&'static str),
    Description(&'static str),
}

#[derive(Debug, Default)]
struct Error {
    expected: Option<&'static [Expected]>,
}

impl Error {
    fn with_expected(mut self, expected: &'static [Expected]) -> Self {
        self.expected = Some(expected);
        self
    }
}

fn escape(found: char) -> Error {
    const ESCAPES: &[Expected] = &[Expected::Literal("b"), Expected::Literal("\\")];
    match found {
        'b' => Error::default().with_expected(ESCAPES),
        '\'' => Error::default().with_expected(&[Expected::Literal("'")]),
        _ => Error::default().with_expected(&[
            Expected::Description("non-double-quote visible characters"),
            Expected::Literal("\\"),
        ]),
    }
}

#[inline(never)]
fn clobber() -> [u64; 64] {
    std::hint::black_box([u64::MAX; 64])
}

pub fn run() -> String {
    let errors = [escape('b'), escape('\''), escape('x')];
    clobber();
    format!("{errors:?}")
}
