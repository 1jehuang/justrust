//! Stock-shaped Linux main-thread panic prefix. The PID is read by the native
//! program, never captured from the compiler process or stored in compiler facts.
use super::PanicSink;

pub(super) fn emit(this: &mut dyn PanicSink) -> Result<(), String> {
    let text = b"\nthread 'main' (";
    let mut prefix = this.diagnostic(text.len())?;
    prefix.extend_from_slice(text);
    this.bytes(&[0x48, 0xbe])?; // movabs rsi, prefix
    let at = this.position();
    this.bytes(&[0; 8])?;
    this.string(at, prefix)?;
    this.bytes(&[0xba, text.len() as u8, 0, 0, 0])?;
    write(this)?;
    // getpid, convert the positive PID to decimal backwards in a stack buffer.
    this.bytes(&[0xb8, 39, 0, 0, 0, 0x0f, 0x05])?;
    this.bytes(&[0x48, 0x83, 0xec, 32, 0x48, 0x8d, 0x74, 0x24, 32])?;
    this.bytes(&[0xb9, 10, 0, 0, 0, 0x45, 0x31, 0xc0])?;
    let digit = this.position();
    this.bytes(&[0x31, 0xd2, 0x48, 0xf7, 0xf1, 0x80, 0xc2, b'0'])?;
    this.bytes(&[0x48, 0xff, 0xce, 0x88, 0x16, 0x49, 0xff, 0xc0])?;
    this.bytes(&[0x48, 0x85, 0xc0])?;
    let more = this.jump(&[0x0f, 0x85])?;
    this.patch(more, digit)?;
    this.bytes(&[0x4c, 0x89, 0xc2])?; // mov rdx, r8
    write(this)?;
    this.bytes(&[0x48, 0x83, 0xc4, 32])?;
    Ok(())
}

pub(super) fn write(this: &mut dyn PanicSink) -> Result<(), String> {
    let retry = this.position();
    this.bytes(&[0xb8, 1, 0, 0, 0, 0xbf, 2, 0, 0, 0, 0x0f, 0x05])?;
    this.bytes(&[0x48, 0x83, 0xf8, 0xfc])?;
    let interrupted = this.jump(&[0x0f, 0x84])?;
    this.patch(interrupted, retry)?;
    this.bytes(&[0x48, 0x85, 0xc0])?;
    let failed = this.jump(&[0x0f, 0x8e])?;
    this.bytes(&[0x48, 0x01, 0xc6, 0x48, 0x29, 0xc2])?;
    let more = this.jump(&[0x0f, 0x85])?;
    this.patch(more, retry)?;
    this.patch(failed, this.position())?;
    Ok(())
}
