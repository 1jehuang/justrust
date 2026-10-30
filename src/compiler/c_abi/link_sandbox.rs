//! Unprivileged Linux native-link confinement. Landlock mediates file content
//! and mutation, while native seccomp mediation closes host path/error oracles.
//! Pathname opens are emulated against captured handles, including failures.
//! Only the trusted initial exec is continued; later exec probes are denied.
use std::{io, path::Path, process::Command};

pub(super) fn setup_error(error: &io::Error) -> String {
    let Some(code) = error.raw_os_error().filter(|c| (16384..20736).contains(c)) else {
        return error.to_string();
    };
    let stage = match (code - 16384) / 256 {
        1 => "probe Landlock ABI (requires >=5)",
        2 => "create Landlock ruleset",
        3 => "open captured directory",
        4 => "add Landlock path rule",
        5 => "enforce Landlock ruleset",
        6 => "enter captured directory",
        7 => "set no-new-privileges",
        8 => "close inherited descriptors",
        9 => "install syscall filter",
        10 => "transfer filesystem mediation listener",
        11 => "bind linker lifetime to compiler",
        12 => "drop linker capabilities",
        13 => "establish parent exec tracer",
        _ => "unknown boundary setup step",
    };
    format!(
        "{stage}: {}",
        io::Error::from_raw_os_error((code - 16384) % 256)
    )
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn confine(
    command: &mut Command,
    root: &Path,
) -> io::Result<std::os::unix::net::UnixDatagram> {
    use std::{
        ffi::{CString, c_char, c_int, c_long},
        os::{
            fd::AsRawFd,
            unix::{ffi::OsStrExt, net::UnixDatagram, process::CommandExt},
        },
    };
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
        fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        fn close(fd: c_int) -> c_int;
        fn chdir(path: *const c_char) -> c_int;
        fn prctl(option: c_int, ...) -> c_int;
        fn getppid() -> c_int;
    }
    #[repr(C)]
    struct Filter {
        code: u16,
        jt: u8,
        jf: u8,
        value: u32,
    }
    #[repr(C)]
    struct Program {
        length: u16,
        filter: *const Filter,
    }
    #[repr(C)]
    struct Ruleset {
        handled_access_fs: u64,
    }
    #[repr(C, packed)]
    struct Beneath {
        allowed_access: u64,
        parent_fd: i32,
    }
    // All sixteen filesystem rights known through ABI 5 are handled, including
    // REFER, TRUNCATE and IOCTL_DEV. Never grant bits discovered at runtime.
    const HANDLED: u64 = (1 << 16) - 1;
    const READ: u64 = (1 << 0) | (1 << 2) | (1 << 3);
    // No MAKE_CHAR, MAKE_BLOCK, MAKE_SOCK, MAKE_FIFO or IOCTL_DEV anywhere.
    const OUTPUT: u64 =
        READ | (1 << 1) | (1 << 4) | (1 << 5) | (1 << 7) | (1 << 8) | (1 << 13) | (1 << 14);
    let out = CString::new(root.join("out").as_os_str().as_bytes())?;
    let root = CString::new(root.as_os_str().as_bytes())?;
    let (parent_socket, child_socket) = UnixDatagram::pair()?;
    let parent_pid = std::process::id();
    let bootstrap = child_socket.as_raw_fd() as u32;
    let instruction = |code, jt, jf, value| Filter {
        code,
        jt,
        jf,
        value,
    };
    let mut filter = vec![
        instruction(0x20, 0, 0, 4),
        instruction(0x15, 1, 0, 0xc000003e),
        instruction(0x06, 0, 0, 0x80000000),
        instruction(0x20, 0, 0, 0),
    ];
    // Even denied Landlock opens distinguish missing host paths from existing
    // ones. Never issue them in the child: the parent copies and resolves the
    // request solely beneath captured directory handles, then injects the fd.
    for number in [2, 5, 59, 257] {
        filter.extend([
            instruction(0x15, 0, 1, number),
            instruction(0x06, 0, 0, 0x7fc00000),
        ]);
    }
    // Bootstrap only. The connected socket is CLOEXEC and no socket-creation
    // syscall or socket descriptor can be admitted by the filesystem broker.
    filter.extend([
        instruction(0x15, 0, 4, 46),
        instruction(0x20, 0, 0, 16),
        instruction(0x15, 0, 1, bootstrap),
        instruction(0x06, 0, 0, 0x7fff0000),
        instruction(0x06, 0, 0, 0x00050001),
    ]);
    // Physical allocation (SEEK_DATA/HOLE) is not a captured input. Logical
    // positions are derived from admitted bytes and may use SET/CUR/END only.
    filter.extend([
        instruction(0x15, 0, 4, 8),
        instruction(0x20, 0, 0, 32),
        instruction(0x25, 0, 1, 2),
        instruction(0x06, 0, 0, 0x00050001),
        instruction(0x06, 0, 0, 0x7fff0000),
    ]);
    // Ordinary allocator advice only, never HWPOISON/SOFT_OFFLINE or unknown
    // future memory-control operations. Capability dropping is independent.
    filter.push(instruction(0x15, 0, 18, 28));
    filter.push(instruction(0x20, 0, 0, 32));
    for advice in [0, 1, 2, 3, 4, 8, 14, 15] {
        filter.push(instruction(0x15, 0, 1, advice));
        filter.push(instruction(0x06, 0, 0, 0x7fff0000));
    }
    filter.push(instruction(0x06, 0, 0, 0x00050001));
    // fcntl only duplication and descriptor flags/status queries. F_SETFL
    // could re-enable O_DIRECT/O_ASYNC after broker admission, so deny it.
    // Resource-limit queries (including
    // prlimit64 on self) are denied, just like other ambient observations.
    filter.extend([instruction(0x15, 0, 12, 72), instruction(0x20, 0, 0, 24)]);
    for operation in [0, 1, 2, 3, 1030] {
        filter.push(instruction(0x15, 0, 1, operation));
        filter.push(instruction(0x06, 0, 0, 0x7fff0000));
    }
    filter.push(instruction(0x06, 0, 0, 0x00050001));
    // Landlock does NOT mediate stat/access/readlink/chmod/chdir. Deny path
    // metadata entirely, including newfstatat (AT_EMPTY_PATH alone is unsafe:
    // a nonempty path still resolves). Descriptor fstat is emulated with
    // canonical metadata. Directory listings exposing physical inode numbers
    // are denied; this linker receives explicit captured file paths.
    // Exact native numbers also exclude x32, process/network/namespace APIs,
    // io_uring, ioctl and capability manipulation.
    // No direct machine/clock/resource/credential/affinity/entropy observations.
    // Such native syscalls receive the same EPERM as unknown operations, not a
    // fabricated host identity. This is only syscall confinement: auxv, vDSO,
    // CPU instructions and staging-path observations remain outside this bound.
    for number in [
        0u32, 1, 3, 7, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 23, 24, 25, 26, 32, 33, 35, 40,
        60, 74, 75, 77, 158, 202, 218, 230, 231, 270, 271, 273, 334, 436,
    ] {
        filter.push(instruction(0x15, 0, 1, number));
        filter.push(instruction(0x06, 0, 0, 0x7fff0000));
    }
    filter.push(instruction(0x06, 0, 0, 0x00050001));
    // SAFETY: immutable preallocated buffers and syscall-only child setup.
    // No allocation, locks, formatting or privilege helper in pre_exec.
    unsafe {
        command.pre_exec(move || {
            fn fail(stage: i32, errno: i32) -> io::Error {
                io::Error::from_raw_os_error(16384 + stage * 256 + errno.clamp(1, 255))
            }
            fn checked(result: c_long, stage: i32) -> io::Result<c_long> {
                if result == -1 {
                    Err(fail(
                        stage,
                        io::Error::last_os_error().raw_os_error().unwrap_or(5),
                    ))
                } else {
                    Ok(result)
                }
            }
            // Compiler cancellation must not leave a CPU-consuming orphan.
            // Check after installing PDEATHSIG to close process-death setup
            // races. Tool code cannot remove it because prctl is not granted.
            checked(prctl(1, 9usize, 0usize, 0usize, 0usize) as c_long, 11)?;
            if getppid() as u32 != parent_pid {
                return Err(fail(11, 3));
            }
            let abi = checked(syscall(444, std::ptr::null::<Ruleset>(), 0usize, 1u32), 1)?;
            if abi < 5 {
                return Err(fail(1, 95));
            }
            checked(prctl(38, 1usize, 0usize, 0usize, 0usize) as c_long, 7)?;
            #[repr(C)]
            struct CapHeader {
                version: u32,
                pid: i32,
            }
            let header = CapHeader {
                version: 0x20080522,
                pid: 0,
            };
            let capabilities = [0u32; 6];
            checked(
                syscall(126, &header as *const CapHeader, capabilities.as_ptr()),
                12,
            )?;
            let attr = Ruleset {
                handled_access_fs: HANDLED,
            };
            let ruleset = checked(
                syscall(
                    444,
                    &attr as *const Ruleset,
                    std::mem::size_of::<Ruleset>(),
                    0u32,
                ),
                2,
            )? as i32;
            for (path, rights) in [(&root, READ), (&out, OUTPUT)] {
                let fd = checked(
                    open(path.as_ptr(), 0x200000 | 0x80000 | 0x10000) as c_long,
                    3,
                )? as i32;
                let rule = Beneath {
                    allowed_access: rights,
                    parent_fd: fd,
                };
                let result = syscall(445, ruleset, 1u32, &rule as *const Beneath, 0u32);
                let result = checked(result, 4);
                close(fd);
                result?;
            }
            checked(syscall(446, ruleset, 0u32), 5)?;
            close(ruleset);
            checked(chdir(root.as_ptr()) as c_long, 6)?;
            // Includes Command's error pipe: CLOEXEC keeps it available to
            // report setup failure but no inherited descriptor survives exec.
            checked(syscall(436, 3u32, u32::MAX, 4u32), 8)?;
            // TRACEME belongs to the thread calling Command::spawn. Do not
            // stop here: spawn must first observe the exec error pipe closing.
            // The successful exec generates SIGTRAP before image instructions.
            checked(syscall(101, 0usize, 0usize, 0usize, 0usize), 13)?;
            let program = Program {
                length: filter.len() as u16,
                filter: filter.as_ptr(),
            };
            let listener = checked(
                syscall(317, 1u32, 8u32 | 32u32, &program as *const Program),
                9,
            )? as i32;
            super::link_sandbox_run::send_listener(child_socket.as_raw_fd(), listener)
                .map_err(|error| fail(10, error.raw_os_error().unwrap_or(5)))?;
            close(listener);
            Ok(())
        });
    }
    Ok(parent_socket)
}
