//! Macros written in an edition before 2024, for a 2024 crate to use.

/// A block whose tail borrows temporaries: before 2024 they outlive the block.
#[macro_export]
macro_rules! values {
    ($show:path; $($name:literal = $value:expr),*) => {{
        let prefix = "";
        let _ = prefix;
        &[$(($name, Some(&$show(&$value)))),*]
    }};
}

/// A field's name, as tracing 0.1.44's `FieldName`.
pub struct FieldName<const N: usize>([u8; N]);

impl<const N: usize> FieldName<N> {
    pub const fn new(input: &str) -> Self {
        let input = input.as_bytes();
        let mut output = [0u8; N];
        let mut at = 0;
        while at < N {
            output[at] = input[at];
            at += 1;
        }
        Self(output)
    }

    pub const fn as_str(&self) -> &str {
        match core::str::from_utf8(self.0.as_slice()) {
            Ok(it) => it,
            Err(_) => "",
        }
    }
}

impl FieldName<0> {
    pub const fn len(input: &str) -> usize {
        input.len()
    }
}

/// The field names a `static` holds.
pub struct FieldSet(pub &'static [&'static str]);

impl FieldSet {
    pub const fn new(names: &'static [&'static str]) -> Self {
        FieldSet(names)
    }
}

/// The names of the fields of a `static`, each made by a `const` in a block, as tracing
/// 0.1.44's `fieldset!` makes them: rustc promotes `&[NAME.as_str()]` in the `static`.
#[macro_export]
macro_rules! fieldset {
    ($($k:ident),*) => {{
        static FIELDS: $crate::FieldSet = {
            $crate::FieldSet::new(&[$({
                const NAME: $crate::FieldName<{ $crate::FieldName::len(stringify!($k)) }> =
                    $crate::FieldName::new(stringify!($k));
                NAME.as_str()
            }),*])
        };
        &FIELDS
    }};
}

/// A closure of this crate around one of the crate that calls it, as boa's
/// `NativeFunction::from_copy_closure_with_captures` makes.
pub fn wrapped<F: Fn() -> u8 + 'static>(inner: F) -> impl Fn() -> u8 + 'static {
    move || inner() + 1
}
