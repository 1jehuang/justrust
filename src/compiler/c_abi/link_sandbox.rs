//! Linux x86-64 enforced native-link boundary. The compiler process/kernel are
//! the trusted enforcement base. No external sandbox helper or driver is run.
//! All child setup uses preallocated data and async-signal-safe system calls.
use std::{io, path::Path, process::Command};

pub(super) fn setup_error(error: &io::Error) -> String {
    let Some(code) = error
        .raw_os_error()
        .filter(|code| (16384..20736).contains(code))
    else {
        return error.to_string();
    };
    let stage = match (code - 16384) / 256 {
        1 => "create user namespace",
        2 => "deny supplementary groups",
        3 => "map user identity",
        4 => "map group identity",
        5 => "create mount/network/IPC/UTS namespaces",
        6 => "make mounts private",
        7 => "bind captured root",
        8 => "bind private output directory",
        9 => "make captured root read-only",
        10 => "enter captured root",
        11 => "chroot to captured inputs",
        12 => "reset working directory",
        13 => "drop capabilities",
        14 => "set no-new-privileges",
        15 => "close inherited descriptors",
        16 => "install syscall filter",
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
        ffi::{CString, c_char, c_int, c_long, c_ulong, c_void},
        os::unix::{ffi::OsStrExt, process::CommandExt},
    };
    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
        fn unshare(flags: c_int) -> c_int;
        fn mount(
            source: *const c_char,
            target: *const c_char,
            kind: *const c_char,
            flags: c_ulong,
            data: *const c_void,
        ) -> c_int;
        fn chroot(path: *const c_char) -> c_int;
        fn chdir(path: *const c_char) -> c_int;
        fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        fn write(fd: c_int, bytes: *const c_void, count: usize) -> isize;
        fn close(fd: c_int) -> c_int;
        fn getuid() -> u32;
        fn getgid() -> u32;
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
    let root = CString::new(root.as_os_str().as_bytes())?;
    let out = CString::new(format!("{}/out", root.to_string_lossy()))?;
    // Preserve the invoking identities inside the namespace. Mapping to 0 would
    // change the meaning of owner permissions in captured files unnecessarily.
    let uid = unsafe { getuid() };
    let gid = unsafe { getgid() };
    let uid_map = format!("{uid} {uid} 1\n").into_bytes();
    let gid_map = format!("{gid} {gid} 1\n").into_bytes();
    let mut filter = vec![
        Filter {
            code: 0x20,
            jt: 0,
            jf: 0,
            value: 4,
        }, // seccomp_data.arch
        Filter {
            code: 0x15,
            jt: 1,
            jf: 0,
            value: 0xc000003e,
        }, // AUDIT_ARCH_X86_64
        Filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            value: 0x80000000,
        }, // KILL_PROCESS
        Filter {
            code: 0x20,
            jt: 0,
            jf: 0,
            value: 0,
        }, // syscall number
    ];
    // A generic prlimit64 grant would let a same-UID tool modify unrelated host
    // processes. Permit only pid 0 (self). Restrict fcntl to fd/flag operations,
    // excluding async signal ownership, leases and other process interactions.
    filter.extend([
        Filter {
            code: 0x15,
            jt: 0,
            jf: 4,
            value: 302,
        },
        Filter {
            code: 0x20,
            jt: 0,
            jf: 0,
            value: 16,
        },
        Filter {
            code: 0x15,
            jt: 0,
            jf: 1,
            value: 0,
        },
        Filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            value: 0x7fff0000,
        },
        Filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            value: 0x00050001,
        },
        Filter {
            code: 0x15,
            jt: 0,
            jf: 14,
            value: 72,
        },
        Filter {
            code: 0x20,
            jt: 0,
            jf: 0,
            value: 24,
        },
    ]);
    for operation in [0, 1, 2, 3, 4, 1030] {
        filter.push(Filter {
            code: 0x15,
            jt: 0,
            jf: 1,
            value: operation,
        });
        filter.push(Filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            value: 0x7fff0000,
        });
    }
    filter.push(Filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        value: 0x00050001,
    });
    // Exact native x86-64 numbers also exclude x32. No fork/clone, network,
    // ptrace, process_vm, pidfd, mount, namespace, io_uring or capability APIs.
    // Filesystem operations can reach only the chroot, whose sole writable
    // mount is /out. exec is permitted for the initial captured ELF image;
    // replacement images remain confined to the same immutable input closure.
    let allowed = [
        0u32, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 23, 24,
        25, 26, 27, 28, 32, 33, 35, 39, 40, 59, 60, 63, 74, 75, 76, 77, 78, 79, 80, 82, 83, 84, 85,
        86, 87, 89, 90, 91, 95, 96, 97, 98, 99, 100, 102, 104, 107, 108, 110, 115, 158, 186, 202,
        204, 217, 218, 228, 229, 230, 231, 257, 258, 262, 263, 264, 267, 269, 270, 271, 273, 318,
        332, 334, 436, 439,
    ];
    for number in allowed {
        filter.push(Filter {
            code: 0x15,
            jt: 0,
            jf: 1,
            value: number,
        });
        filter.push(Filter {
            code: 0x06,
            jt: 0,
            jf: 0,
            value: 0x7fff0000,
        }); // ALLOW
    }
    filter.push(Filter {
        code: 0x06,
        jt: 0,
        jf: 0,
        value: 0x00050001,
    }); // ERRNO(EPERM)
    // SAFETY: this closure owns only immutable, preallocated buffers. It does not
    // allocate, lock, format, access Rust TLS, or call non-signal-safe std APIs.
    // Any failed confinement step aborts exec through Command's error pipe.
    unsafe {
        command.pre_exec(move || {
            fn tagged(error: io::Error, stage: i32) -> io::Error {
                io::Error::from_raw_os_error(
                    16384 + stage * 256 + error.raw_os_error().unwrap_or(5).clamp(1, 255),
                )
            }
            fn checked(result: c_int, stage: i32) -> io::Result<()> {
                if result == -1 {
                    Err(tagged(io::Error::last_os_error(), stage))
                } else {
                    Ok(())
                }
            }
            unsafe fn map(path: *const c_char, bytes: &[u8]) -> io::Result<()> {
                let fd = unsafe { open(path, 1 | 0x80000) }; // WRONLY | CLOEXEC
                if fd == -1 {
                    return Err(io::Error::last_os_error());
                }
                let result = unsafe { write(fd, bytes.as_ptr().cast(), bytes.len()) };
                let error = if result != bytes.len() as isize {
                    Some(io::Error::last_os_error())
                } else {
                    None
                };
                unsafe {
                    close(fd);
                }
                if let Some(error) = error {
                    Err(error)
                } else {
                    Ok(())
                }
            }
            // A fresh user namespace grants mount capabilities only within this
            // child. Never invoke a setuid helper, change host policy or relax a
            // tenant sandbox to make this work. Unsupported hosts fail closed.
            checked(unshare(0x10000000), 1)?; // CLONE_NEWUSER
            map(c"/proc/self/uid_map".as_ptr(), &uid_map).map_err(|e| tagged(e, 3))?;
            map(c"/proc/self/setgroups".as_ptr(), b"deny").map_err(|e| tagged(e, 2))?;
            map(c"/proc/self/gid_map".as_ptr(), &gid_map).map_err(|e| tagged(e, 4))?;
            checked(
                unshare(0x00020000 | 0x40000000 | 0x08000000 | 0x04000000),
                5,
            )?; // mount, net, ipc, uts
            checked(
                mount(
                    std::ptr::null(),
                    c"/".as_ptr(),
                    std::ptr::null(),
                    (1 << 18) | (1 << 14),
                    std::ptr::null(),
                ),
                6,
            )?; // REC|PRIVATE
            checked(
                mount(
                    root.as_ptr(),
                    root.as_ptr(),
                    std::ptr::null(),
                    4096,
                    std::ptr::null(),
                ),
                7,
            )?;
            checked(
                mount(
                    out.as_ptr(),
                    out.as_ptr(),
                    std::ptr::null(),
                    4096,
                    std::ptr::null(),
                ),
                8,
            )?;
            checked(
                mount(
                    std::ptr::null(),
                    root.as_ptr(),
                    std::ptr::null(),
                    4096 | 32 | 1 | 2 | 4,
                    std::ptr::null(),
                ),
                9,
            )?; // BIND|REMOUNT|RDONLY|NOSUID|NODEV
            checked(chdir(root.as_ptr()), 10)?;
            checked(chroot(c".".as_ptr()), 11)?;
            checked(chdir(c"/".as_ptr()), 12)?;
            // Drop all permitted/effective/inheritable capabilities before exec.
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
            if syscall(126, &header as *const CapHeader, capabilities.as_ptr()) == -1 {
                return Err(tagged(io::Error::last_os_error(), 13));
            }
            checked(prctl(38, 1usize, 0usize, 0usize, 0usize), 14)?; // NO_NEW_PRIVS
            // No caller file descriptor can become an undeclared input after
            // exec. CLOEXEC preserves Command's error pipe until then.
            if syscall(436, 3u32, u32::MAX, 4u32) == -1 {
                return Err(tagged(io::Error::last_os_error(), 15));
            }
            let program = Program {
                length: filter.len() as u16,
                filter: filter.as_ptr(),
            };
            checked(
                prctl(22, 2usize, &program as *const Program, 0usize, 0usize),
                16,
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
