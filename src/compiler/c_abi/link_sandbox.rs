//! Unprivileged Linux native-link confinement. Landlock mediates file content
//! and mutation, seccomp denies unmediated path metadata and ambient APIs.
//! This is not a private pathname namespace: failed ordinary opens may still
//! distinguish nonexistent host paths (ENOENT) from forbidden ones (EACCES).
//! Do not treat this boundary as proving host-existence-independent output.
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
        _ => "unknown boundary setup step",
    };
    format!(
        "{stage}: {}",
        io::Error::from_raw_os_error((code - 16384) % 256)
    )
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn confine(command: &mut Command, root: &Path) -> io::Result<()> {
    use std::{
        ffi::{CString, c_char, c_int, c_long},
        os::unix::{ffi::OsStrExt, process::CommandExt},
    };
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
        fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        fn close(fd: c_int) -> c_int;
        fn chdir(path: *const c_char) -> c_int;
        fn prctl(option: c_int, ...) -> c_int;
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
    // O_PATH bypasses Landlock content checks and would expose host metadata
    // through fstat. Inspect open/openat flags before granting these syscalls.
    for (number, flags_offset) in [(2, 24), (257, 32)] {
        filter.extend([
            instruction(0x15, 0, 4, number),
            instruction(0x20, 0, 0, flags_offset),
            instruction(0x45, 0, 1, 0x200000),
            instruction(0x06, 0, 0, 0x00050001),
            instruction(0x06, 0, 0, 0x7fff0000),
        ]);
    }
    // prlimit64 only on self. fcntl only descriptor/flag operations.
    filter.extend([
        instruction(0x15, 0, 4, 302),
        instruction(0x20, 0, 0, 16),
        instruction(0x15, 0, 1, 0),
        instruction(0x06, 0, 0, 0x7fff0000),
        instruction(0x06, 0, 0, 0x00050001),
        instruction(0x15, 0, 14, 72),
        instruction(0x20, 0, 0, 24),
    ]);
    for operation in [0, 1, 2, 3, 4, 1030] {
        filter.push(instruction(0x15, 0, 1, operation));
        filter.push(instruction(0x06, 0, 0, 0x7fff0000));
    }
    filter.push(instruction(0x06, 0, 0, 0x00050001));
    // Landlock does NOT mediate stat/access/readlink/chmod/chdir. Deny path
    // metadata entirely, including newfstatat (AT_EMPTY_PATH alone is unsafe:
    // a nonempty path still resolves). Only descriptor fstat is permitted.
    // Exact native numbers also exclude x32, process/network/namespace APIs,
    // io_uring, ioctl and capability manipulation.
    for number in [
        0u32, 1, 2, 3, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 17, 18, 19, 20, 23, 24, 25, 26, 27, 28,
        32, 33, 35, 39, 40, 59, 60, 63, 74, 75, 76, 77, 78, 79, 82, 83, 84, 85, 86, 87, 95, 96, 97,
        98, 99, 100, 102, 104, 107, 108, 110, 115, 158, 186, 202, 204, 217, 218, 228, 229, 230,
        231, 257, 258, 263, 264, 270, 271, 273, 318, 334, 436,
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
            let abi = checked(syscall(444, std::ptr::null::<Ruleset>(), 0usize, 1u32), 1)?;
            if abi < 5 {
                return Err(fail(1, 95));
            }
            checked(prctl(38, 1usize, 0usize, 0usize, 0usize) as c_long, 7)?;
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
            let program = Program {
                length: filter.len() as u16,
                filter: filter.as_ptr(),
            };
            checked(
                prctl(22, 2usize, &program as *const Program, 0usize, 0usize) as c_long,
                9,
            )?;
            Ok(())
        });
    }
    Ok(())
}
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(super) fn confine(_: &mut Command, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "controlled native link sandbox requires Linux x86-64",
    ))
}
