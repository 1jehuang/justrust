//! Bounds diagnostics use the actual unsigned index/length snapshots, not
//! compiler guesses. Entry has the abort sigaction buffer at rsp, followed by
//! saved length at +32 and index at +40. No Rust value is read again.
use super::{PanicSink, ir, panic_header};

pub(super) fn emit(this: &mut dyn PanicSink, location: &ir::RuntimeLocation) -> Result<(), String> {
    panic_header::emit(this)?;
    literal(
        this,
        format!(
            ") panicked at {}:\nindex out of bounds: the len is ",
            location.diagnostic_location()
        )
        .as_bytes(),
    )?;
    this.bytes(&[0x48, 0x8b, 0x44, 0x24, 32])?;
    unsigned(this)?;
    literal(this, b" but the index is ")?;
    this.bytes(&[0x48, 0x8b, 0x44, 0x24, 40])?;
    unsigned(this)?;
    literal(
        this,
        b"\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n",
    )
}

fn literal(this: &mut dyn PanicSink, text: &[u8]) -> Result<(), String> {
    let mut value = this.diagnostic(text.len())?;
    value.extend_from_slice(text);
    this.bytes(&[0x48, 0xbe])?;
    let at = this.position();
    this.bytes(&[0; 8])?;
    this.string(at, value)?;
    this.bytes(&[0x48, 0xba])?;
    this.bytes(&(text.len() as u64).to_le_bytes())?;
    panic_header::write(this)
}

fn unsigned(this: &mut dyn PanicSink) -> Result<(), String> {
    this.bytes(&[0x48, 0x83, 0xec, 32, 0x48, 0x8d, 0x74, 0x24, 32])?;
    this.bytes(&[0xb9, 10, 0, 0, 0, 0x45, 0x31, 0xc0])?;
    let digit = this.position();
    this.bytes(&[0x31, 0xd2, 0x48, 0xf7, 0xf1, 0x80, 0xc2, b'0'])?;
    this.bytes(&[0x48, 0xff, 0xce, 0x88, 0x16, 0x49, 0xff, 0xc0])?;
    this.bytes(&[0x48, 0x85, 0xc0])?;
    let more = this.jump(&[0x0f, 0x85])?;
    this.patch(more, digit)?;
    this.bytes(&[0x4c, 0x89, 0xc2])?;
    panic_header::write(this)?;
    this.bytes(&[0x48, 0x83, 0xc4, 32])
}
