//! Statics with `#[link_section]`, which on wasm are custom sections of the module: two in
//! one section (the linker joins them, as it does those of every object), one in a section
//! of its own that nothing refers to, and one the code reads, which is in memory too.

#[unsafe(link_section = "blaze_joined")]
pub static FIRST: [u8; 5] = *b"hello";

#[unsafe(link_section = "blaze_joined")]
pub static SECOND: [u8; 3] = [1, 2, 3];

#[unsafe(link_section = "blaze_alone")]
#[used]
static ALONE: u32 = 0x0102_0304;

#[unsafe(link_section = "blaze_read")]
pub static READ: (u16, [u8; 2]) = (0xbeef, *b"ok");

pub fn read() -> u32 {
    u32::from(READ.0) + u32::from(READ.1[0]) + u32::from(FIRST[4])
}
