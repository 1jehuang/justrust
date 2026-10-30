//! Parent-owned native64 munmap preloader. Never resumes image instructions.
//! Linux include/linux/ptrace.h user_enable_single_step and
//! arch/x86/kernel/ptrace.c syscall single-step reporting define the stop.
use std::{
    fs::File,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd},
    time::{Duration, Instant},
};
unsafe extern "C" {
    fn ptrace(request: u32, pid: i32, address: usize, data: usize) -> i64;
    fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    fn openat(directory: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    fn __errno_location() -> *mut i32;
}
fn invalid(reason: &str) -> io::Error {
    io::Error::other(format!("controlled linker kernel mapping seal: {reason}"))
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mapping {
    start: u64,
    end: u64,
    name: String,
}
#[derive(Debug, PartialEq, Eq)]
struct Layout {
    ranges: Vec<Mapping>,
    other: Vec<String>,
}
fn parse(bytes: &[u8]) -> io::Result<Layout> {
    if bytes.len() > 65536 || !bytes.ends_with(b"\n") {
        return Err(invalid("maps size/termination"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("maps encoding"))?;
    let mut ranges = Vec::new();
    let mut other = Vec::new();
    let mut previous = 0;
    for (index, line) in text.lines().enumerate() {
        if index >= 256 || line.len() > 4096 {
            return Err(invalid("maps bounds"));
        }
        let fields: Vec<_> = line.split_ascii_whitespace().collect();
        if fields.len() < 5 {
            return Err(invalid("maps fields"));
        }
        let (start, end) = fields[0]
            .split_once('-')
            .ok_or_else(|| invalid("maps range"))?;
        let hex = |s| u64::from_str_radix(s, 16).map_err(|_| invalid("maps address"));
        let (start, end) = (hex(start)?, hex(end)?);
        if start < previous || start >= end || start % 4096 != 0 || end % 4096 != 0 {
            return Err(invalid("maps order/alignment"));
        }
        previous = end;
        let name = fields.get(5).copied().unwrap_or("");
        if name.contains("vdso") || name.contains("vvar") {
            if fields.len() != 6
                || !matches!(name, "[vdso]" | "[vvar]" | "[vvar_vclock]")
                || fields[2] != "00000000"
                || fields[3] != "00:00"
                || fields[4] != "0"
                || fields[1] != if name == "[vdso]" { "r-xp" } else { "r--p" }
                || start == 0
                || end > (1 << 47)
                || end - start > 65536
                || ranges.iter().any(|m: &Mapping| m.name == name)
            {
                return Err(invalid("unexpected kernel mapping alias/shape"));
            }
            ranges.push(Mapping {
                start,
                end,
                name: name.to_owned(),
            });
        } else {
            other.push(line.to_owned());
        }
    }
    Ok(Layout { ranges, other })
}
fn maps(procdir: &File) -> io::Result<Layout> {
    let fd = unsafe { openat(procdir.as_raw_fd(), c"maps".as_ptr(), 0x80000 | 0x20000) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let mut bytes = Vec::new();
    unsafe { File::from_raw_fd(fd) }
        .take(65537)
        .read_to_end(&mut bytes)?;
    parse(&bytes)
}
fn request(pid: i32, op: u32, address: usize, data: usize) -> io::Result<i64> {
    unsafe {
        *__errno_location() = 0;
    }
    let result = unsafe { ptrace(op, pid, address, data) };
    if result == -1 && unsafe { *__errno_location() } != 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(result)
    }
}
fn regs(pid: i32) -> io::Result<[u64; 27]> {
    let mut registers = [0; 27];
    request(pid, 12, 0, registers.as_mut_ptr() as usize)?;
    if registers[17] != 0x33 {
        return Err(invalid("not native64"));
    }
    Ok(registers)
}
fn set_regs(pid: i32, registers: &[u64; 27]) -> io::Result<()> {
    request(pid, 13, 0, registers.as_ptr() as usize)?;
    if regs(pid)? != *registers {
        return Err(invalid("register readback"));
    }
    Ok(())
}
fn wait_step(pid: i32, next_ip: u64) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut status = 0;
        let result = unsafe { waitpid(pid, &mut status, 1) };
        if result == pid {
            if status != (5 << 8) | 0x7f {
                return Err(invalid(&format!("unexpected injection stop {status:#x}")));
            }
            let mut info = [0u64; 16];
            request(pid, 0x4202, 0, info.as_mut_ptr() as usize)?;
            let words = unsafe { std::slice::from_raw_parts(info.as_ptr().cast::<i32>(), 32) };
            // x86 user_single_step_report uses TRAP_BRKPT (not TRAP_TRACE)
            // through send_sigtrap/force_sig_fault, with si_addr = return RIP.
            if words[0] != 5 || words[1] != 0 || words[2] != 1 || info[2] != next_ip {
                return Err(invalid(&format!(
                    "unexpected injection siginfo {:?}",
                    &words[..3]
                )));
            }
            return Ok(());
        }
        if result < 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::Interrupted {
                return Err(e);
            }
        }
        if Instant::now() >= deadline {
            return Err(invalid("single-step timeout"));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}
pub(super) fn remove(pid: i32, procdir: &File) -> io::Result<()> {
    let mut remaining = maps(procdir)?;
    if !remaining.ranges.iter().any(|m| m.name == "[vdso]") || maps(procdir)? != remaining {
        return Err(invalid("missing vdso or unstable maps"));
    }
    let original = regs(pid)?;
    if original[18] & (0x100 | 0x10000) != 0 {
        return Err(invalid("unexpected initial trace/resume flags"));
    }
    if remaining
        .ranges
        .windows(2)
        .any(|pair| pair[0].end != pair[1].start)
        || (remaining.ranges.iter().any(|m| m.name == "[vvar_vclock]")
            && !remaining.ranges.iter().any(|m| m.name == "[vvar]"))
    {
        return Err(invalid("unsupported kernel mapping layout"));
    }
    let ip = original[16];
    if ip == 0
        || ip >= (1 << 47)
        || ip % 4096 > 4088
        || remaining
            .ranges
            .iter()
            .any(|m| (m.start..m.end).contains(&ip))
    {
        return Err(invalid("injection address"));
    }
    let address = ip as usize;
    let code = request(pid, 1, address, 0)? as u64;
    let injected = 0xcccc_cccc_cccc_050fu64;
    request(pid, 4, address, injected as usize)?;
    if request(pid, 1, address, 0)? as u64 != injected {
        return Err(invalid("instruction readback"));
    }
    while let Some(mapping) = remaining.ranges.last().cloned() {
        if maps(procdir)? != remaining {
            return Err(invalid("unstable mapping layout"));
        }
        let mut call = original;
        call[10] = 11; // rax
        call[15] = u64::MAX; // orig_rax, not a restarted syscall
        call[14] = mapping.start; // rdi
        call[13] = mapping.end - mapping.start; // rsi
        set_regs(pid, &call)?;
        request(pid, 9, 0, 0)?; // SINGLESTEP: exactly the injected syscall
        wait_step(pid, ip + 2)?;
        let returned = regs(pid)?;
        for index in 0..27 {
            if ![6, 10, 11, 15, 16, 18].contains(&index) && returned[index] != call[index] {
                return Err(invalid("unexpected syscall register mutation"));
            }
        }
        if returned[11] != ip + 2
            || (returned[6] & !0x100) != original[18]
            || (returned[18] & !0x100) != original[18]
        {
            return Err(invalid("unexpected syscall continuation state"));
        }
        if returned[16] != ip + 2
            || returned[10] != 0
            || returned[15] != 11
            || returned[19] != original[19]
        {
            return Err(invalid(&format!(
                "unexpected munmap return rip={:#x} rax={} orig={}",
                returned[16], returned[10], returned[15]
            )));
        }
        remaining.ranges.pop();
        if maps(procdir)? != remaining {
            return Err(invalid("munmap mapping readback"));
        }
    }
    request(pid, 4, address, code as usize)?;
    if request(pid, 1, address, 0)? as u64 != code {
        return Err(invalid("original instruction readback"));
    }
    set_regs(pid, &original)?;
    if !maps(procdir)?.ranges.is_empty() {
        return Err(invalid("kernel mappings remain at seal"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controlled_link_kernel_mapping_seal_restores_native_loader_state() {
        use std::{os::unix::process::CommandExt, process::Command};
        let mut command = Command::new("/usr/bin/../bin/true");
        unsafe {
            command.pre_exec(|| {
                request(0, 0, 0, 0)?; // TRACEME, before any image entry
                Ok(())
            });
        }
        let mut child = super::super::Guard::new(command.spawn().unwrap());
        child.await_exec().unwrap();
        let pid = child.child.id() as i32;
        let process = File::open(format!("/proc/{pid}")).unwrap();
        assert!(
            maps(&process)
                .unwrap()
                .ranges
                .iter()
                .any(|m| m.name == "[vdso]")
        );
        let original = regs(pid).unwrap();
        let instruction = request(pid, 1, original[16] as usize, 0).unwrap();
        child.normalize_launch(&process).unwrap();
        assert!(maps(&process).unwrap().ranges.is_empty());
        assert_eq!(regs(pid).unwrap(), original);
        assert_eq!(
            request(pid, 1, original[16] as usize, 0).unwrap(),
            instruction
        );
        child.detach().unwrap();
        assert!(child.child.wait().unwrap().success());
    }

    #[test]
    fn controlled_link_failed_injected_munmap_kills_without_image_entry() {
        use std::{
            io::Read,
            os::unix::process::CommandExt,
            process::{Command, Stdio},
        };
        #[repr(C)]
        struct Filter {
            code: u16,
            jt: u8,
            jf: u8,
            value: u32,
        }
        #[repr(C)]
        struct Program {
            count: u16,
            filters: *const Filter,
        }
        unsafe extern "C" {
            fn prctl(option: i32, ...) -> i32;
        }
        let mut command = Command::new("/usr/bin/../bin/echo");
        command.arg("untrusted-image-ran").stdout(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                request(0, 0, 0, 0)?;
                let filters = [
                    Filter {
                        code: 0x20,
                        jt: 0,
                        jf: 0,
                        value: 0,
                    },
                    Filter {
                        code: 0x15,
                        jt: 0,
                        jf: 1,
                        value: 11,
                    },
                    Filter {
                        code: 0x06,
                        jt: 0,
                        jf: 0,
                        value: 0x50001,
                    },
                    Filter {
                        code: 0x06,
                        jt: 0,
                        jf: 0,
                        value: 0x7fff0000,
                    },
                ];
                let program = Program {
                    count: 4,
                    filters: filters.as_ptr(),
                };
                if prctl(38, 1usize, 0usize, 0usize, 0usize) != 0
                    || prctl(22, 2usize, &program as *const Program) != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = super::super::Guard::new(command.spawn().unwrap());
        child.await_exec().unwrap();
        let pid = child.child.id();
        let process = File::open(format!("/proc/{pid}")).unwrap();
        let mut stdout = child.child.stdout.take().unwrap();
        let error = child.normalize_launch(&process).unwrap_err();
        assert!(
            error.to_string().contains("unexpected munmap return"),
            "{error}"
        );
        assert!(child.detach().is_err());
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).unwrap();
        assert!(bytes.is_empty(), "injection failure executed image code");
    }

    #[test]
    fn bounded_mapping_parser_rejects_truncation_and_aliases() {
        assert!(parse(b"").is_err());
        assert!(parse(b"1000-3000 r-xp 00000000 00:00 0 [vdso]").is_err());
        assert!(parse(&vec![b' '; 65537]).is_err());
        let line = "1000-3000 r-xp 00000000 00:00 0 [vdso]\n";
        assert!(parse(line.repeat(257).as_bytes()).is_err());
        for name in ["[vdso32]", "[vvar_alias]", "[vvar_vclock] extra"] {
            let text = format!("1000-3000 r-xp 00000000 00:00 0 {name}\n");
            assert!(parse(text.as_bytes()).is_err());
        }
        let mut layout = parse(
            b"1000-3000 r-xp 00000000 00:00 0 /image\n3000-5000 r-xp 00000000 00:00 0 [vdso]\n",
        )
        .unwrap();
        assert_eq!(layout.other.len(), 1);
        layout.ranges.pop();
        assert_eq!(
            layout,
            parse(b"1000-3000 r-xp 00000000 00:00 0 /image\n").unwrap()
        );
    }
    #[test]
    fn strict_mapping_parser() {
        let good =
            b"1000-3000 r-xp 00000000 00:00 0 [vdso]\n3000-7000 r--p 00000000 00:00 0 [vvar]\n";
        assert_eq!(parse(good).unwrap().ranges.len(), 2);
        for bad in [
            "0000-3000 r-xp 00000000 00:00 0 [vdso]\n",
            "1000-3000 rwxp 00000000 00:00 0 [vdso]\n",
            "1000-3000 r-xp 00000000 00:00 0 [vdso_alias]\n",
            "1001-3000 r-xp 00000000 00:00 0 [vdso]\n",
            "1000-3000 r-xp 00000000 00:00 0 [vdso]\n1000-3000 r-xp 00000000 00:00 0 [vdso]\n",
        ] {
            assert!(parse(bad.as_bytes()).is_err(), "{bad}");
        }
    }
}
