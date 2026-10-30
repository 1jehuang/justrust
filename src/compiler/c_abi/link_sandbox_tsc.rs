//! Exact native timestamp-fault emulation, owned by one traced operation.
use std::{
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::FileExt,
    },
};
unsafe extern "C" {
    fn ptrace(request: u32, pid: i32, address: usize, data: usize) -> i64;
    fn openat(dir: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
}
#[repr(C)]
struct IoVec {
    base: *mut u64,
    length: usize,
}
fn request(pid: i32, op: u32, address: usize, data: usize) -> io::Result<()> {
    if unsafe { ptrace(op, pid, address, data) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
fn registers(pid: i32) -> io::Result<[u64; 27]> {
    let mut regs = [0; 27];
    let mut vector = IoVec {
        base: regs.as_mut_ptr(),
        length: std::mem::size_of_val(&regs),
    };
    request(pid, 0x4204, 1, (&mut vector as *mut IoVec) as usize)?;
    if vector.length != std::mem::size_of_val(&regs) || regs[17] != 0x33 || regs[20] != 0x2b {
        return Err(io::Error::other(
            "unsupported timestamp native register shape",
        ));
    }
    Ok(regs)
}
/// False means the original signal must be delivered, without register changes.
pub(super) fn emulate(pid: i32, procdir: &File, counter: &mut u64) -> io::Result<bool> {
    let mut info = [0u64; 16];
    request(pid, 0x4202, 0, info.as_mut_ptr() as usize)?;
    let words = unsafe { std::slice::from_raw_parts(info.as_ptr().cast::<i32>(), 32) };
    // #GP from PR_SET_TSC is SIGSEGV/SI_KERNEL with a null si_addr.
    if words[0] != 11 || words[1] != 0 || words[2] != 128 || info[2] != 0 {
        return Ok(false);
    }
    let mut regs = registers(pid)?;
    let ip = regs[16];
    // Native #GP sets RF in the saved frame. Keep it unchanged: after
    // advancing RIP it is consumed by the next successful instruction.
    // Single-step state is not part of this continuation protocol.
    if ip == 0 || ip >= (1 << 47) || regs[18] & 0x100 != 0 {
        return Ok(false);
    }
    let fd = unsafe { openat(procdir.as_raw_fd(), c"mem".as_ptr(), 0x80000 | 0x20000) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let memory = unsafe { File::from_raw_fd(fd) };
    let mut code = [0; 2];
    if memory.read_exact_at(&mut code, ip).is_err() {
        return Ok(false);
    }
    let (length, rdtscp) = if code == [0x0f, 0x31] {
        (2, false)
    } else if code == [0x0f, 0x01] {
        let mut tail = [0];
        if memory.read_exact_at(&mut tail, ip + 2).is_err() || tail != [0xf9] {
            return Ok(false);
        }
        (3, true)
    } else {
        return Ok(false);
    };
    let next = counter
        .checked_add(1)
        .ok_or_else(|| io::Error::other("timestamp counter exhausted"))?;
    regs[10] = next & 0xffff_ffff; // eax, zero extended
    regs[12] = next >> 32; // edx, zero extended
    if rdtscp {
        regs[11] = 0;
    } // ecx, virtual CPU zero
    regs[16] += length;
    let vector = IoVec {
        base: regs.as_mut_ptr(),
        length: std::mem::size_of_val(&regs),
    };
    request(pid, 0x4205, 1, (&vector as *const IoVec) as usize)?;
    if registers(pid)? != regs {
        return Err(io::Error::other("timestamp register readback mismatch"));
    }
    *counter = next;
    Ok(true)
}
