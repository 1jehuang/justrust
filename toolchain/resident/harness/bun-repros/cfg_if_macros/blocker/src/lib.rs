// lol_html's shape: `trace!` is defined by `cfg_if!`, which a `use` names, in a `#[macro_use]`
// module, and the modules that follow call it.
#[macro_use]
mod base;
mod parser;

pub fn scan(input: &[u8]) -> (usize, Option<u8>) {
    let mut cursor = parser::Cursor { next_pos: 0 };
    let mut tags = 0;
    let mut last = None;
    while let Some(ch) = cursor.consume_ch(input) {
        tags += usize::from(ch == b'<');
        last = Some(ch);
    }
    cursor.unconsume_ch();
    (tags, last.or(cursor.lookahead(input, 1)))
}
