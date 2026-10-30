//! Bounded native64 initial-stack normalization, while the exec trace stop
//! prevents even the dynamic interpreter from executing. This changes only
//! AT_RANDOM's sixteen bytes, present stack credential values, the stack
//! AT_SYSINFO_EHDR value and the existing AT_EXECFN string. It does not
//! remove the vDSO mapping, canonicalize addresses, CPU features or /proc auxv,
//! or establish complete launch determinism. A public seed is intentional for
//! this confined tool, not a source of cryptographic randomness.
// ABI references: Linux fs/binfmt_elf.c:create_elf_tables,
// include/uapi/linux/auxvec.h and arch/x86/include/asm/user_64.h.
use std::{
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::FileExt,
    },
    time::{Duration, Instant},
};

const SEED: [u8; 16] = *b"jr-link-seed-v1!";
const EXECFN: &[u8] = b"jr-controlled-ld\0";
const CREDENTIAL: u64 = 1000;
const STACK_SPAN: u64 = 1024 * 1024;
const MAX_POINTERS: u64 = 8192;
const MAX_AUX: usize = 64;
const USER_END: u64 = 1 << 47;

fn invalid(reason: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        format!("unsupported controlled linker initial stack: {reason}"),
    )
}

struct Plan {
    credentials: [Option<u64>; 4],
    random: u64,
    sysinfo: Option<u64>,
    execfn: u64,
    stack_end: u64,
}

/// Inspect completely before writing anything. The kernel-built stack is
/// trusted, but unsupported layouts must never turn into arbitrary writes.
fn plan(rsp: u64, mut read: impl FnMut(u64) -> io::Result<u64>) -> io::Result<Plan> {
    let end = rsp
        .checked_add(STACK_SPAN)
        .map(|end| end.min(USER_END))
        .filter(|end| *end > rsp)
        .ok_or_else(|| invalid("stack address range"))?;
    if rsp == 0 || !rsp.is_multiple_of(16) {
        return Err(invalid("unaligned native64 RSP"));
    }
    let mut cursor = rsp;
    let mut next = || {
        let address = cursor;
        cursor = cursor
            .checked_add(8)
            .filter(|next| *next <= end)
            .ok_or_else(|| invalid("stack table limit"))?;
        read(address)
    };
    let argc = next()?;
    if !(1..=MAX_POINTERS).contains(&argc) {
        return Err(invalid("argument count"));
    }
    let mut first_string = end;
    let mut string = |pointer| {
        if !(rsp..end).contains(&pointer) {
            return Err(invalid("string pointer outside bounded stack"));
        }
        first_string = first_string.min(pointer);
        Ok(())
    };
    for _ in 0..argc {
        string(next()?)?;
    }
    if next()? != 0 {
        return Err(invalid("missing argv terminator"));
    }
    let mut env_ended = false;
    for _ in 0..=MAX_POINTERS {
        let pointer = next()?;
        if pointer == 0 {
            env_ended = true;
            break;
        }
        string(pointer)?;
    }
    if !env_ended {
        return Err(invalid("environment count"));
    }
    let mut seen = 0u64;
    let mut credentials = [None; 4];
    let mut random = None;
    let mut sysinfo_index = None;
    let mut execfn = None;
    let mut aux_ended = false;
    let aux_start = cursor;
    for index in 0..MAX_AUX {
        let address = cursor;
        cursor = cursor
            .checked_add(16)
            .filter(|next| *next <= end)
            .ok_or_else(|| invalid("auxiliary table limit"))?;
        let kind = read(address)?;
        let value = read(address + 8)?;
        if kind == 0 {
            if value != 0 {
                return Err(invalid("nonzero AT_NULL value"));
            }
            aux_ended = true;
            break;
        }
        if !matches!(kind, 3..=9 | 11..=17 | 23..=31 | 33 | 51) || seen & (1 << kind) != 0 {
            return Err(invalid("unknown or duplicate auxiliary tag"));
        }
        seen |= 1 << kind;
        match kind {
            11..=14 => {
                if value > u32::MAX as u64 {
                    return Err(invalid("credential value exceeds native uid/gid width"));
                }
                credentials[(kind - 11) as usize] = Some(address + 8);
            }
            4 if value != 56 => return Err(invalid("non-native program header size")),
            5 if !(1..=4096).contains(&value) => return Err(invalid("program header count")),
            6 if value != 4096 => return Err(invalid("non-native page size")),
            15 | 24 => string(value)?,
            31 => {
                string(value)?;
                execfn = Some(value);
            }
            25 => random = Some(value),
            33 => sysinfo_index = Some(index),
            _ => {}
        }
    }
    if !aux_ended {
        return Err(invalid("missing auxiliary terminator"));
    }
    for required in [3, 4, 5, 6, 7, 9, 25, 31] {
        if seen & (1 << required) == 0 {
            return Err(invalid("missing required auxiliary tag"));
        }
    }
    let random = random.ok_or_else(|| invalid("missing AT_RANDOM"))?;
    if random < cursor || random.checked_add(16).is_none_or(|end| end > first_string) {
        return Err(invalid("AT_RANDOM overlaps tables or strings"));
    }
    let execfn = execfn.ok_or_else(|| invalid("missing AT_EXECFN"))?;
    if execfn < cursor {
        return Err(invalid("AT_EXECFN overlaps stack tables"));
    }
    Ok(Plan {
        credentials,
        random,
        sysinfo: sysinfo_index.map(|index| aux_start + index as u64 * 16 + 8),
        execfn,
        stack_end: end,
    })
}

fn canonical_execfn(
    address: u64,
    stack_end: u64,
    mut read: impl FnMut(u64) -> io::Result<u8>,
) -> io::Result<Vec<u8>> {
    let limit = stack_end.saturating_sub(address).min(4096);
    for offset in 0..limit {
        if read(address + offset)? == 0 {
            let length = offset as usize + 1;
            if length < EXECFN.len() {
                return Err(invalid("AT_EXECFN string too short for canonical name"));
            }
            // Zero all former bytes, not just the new terminator. The old
            // private path must not survive behind the canonical C string.
            let mut bytes = vec![0; length];
            bytes[..EXECFN.len()].copy_from_slice(EXECFN);
            return Ok(bytes);
        }
    }
    Err(invalid("unterminated or oversized AT_EXECFN string"))
}

struct Memory {
    file: File,
    deadline: Instant,
}
impl Memory {
    fn transfer(&self, address: u64, bytes: &mut [u8], write: bool) -> io::Result<()> {
        let mut done = 0;
        while done < bytes.len() {
            if Instant::now() >= self.deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "linker launch normalization exceeded 5 seconds",
                ));
            }
            let offset = address
                .checked_add(done as u64)
                .ok_or_else(|| invalid("memory offset overflow"))?;
            let result = if write {
                self.file.write_at(&bytes[done..], offset)
            } else {
                self.file.read_at(&mut bytes[done..], offset)
            };
            match result {
                Ok(0) => return Err(io::Error::other("short linker launch memory transfer")),
                Ok(count) => done += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    fn word(&self, address: u64) -> io::Result<u64> {
        let mut bytes = [0; 8];
        self.transfer(address, &mut bytes, false)?;
        Ok(u64::from_le_bytes(bytes))
    }
    fn replace(&self, address: u64, bytes: &[u8]) -> io::Result<()> {
        let mut buffer = bytes.to_vec();
        self.transfer(address, &mut buffer, true)?;
        self.transfer(address, &mut buffer, false)?;
        if buffer != bytes {
            return Err(io::Error::other("linker launch memory readback mismatch"));
        }
        Ok(())
    }
}

pub(super) fn normalize(procdir: &File, rsp: u64) -> io::Result<()> {
    unsafe extern "C" {
        fn openat(directory: i32, path: *const std::ffi::c_char, flags: i32, ...) -> i32;
    }
    // The directory pins the original task and this file pins its stopped
    // post-exec mm. Never reopen memory by a reusable numeric process ID.
    let fd = unsafe { openat(procdir.as_raw_fd(), c"mem".as_ptr(), 2 | 0x80000) };
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }
    let memory = Memory {
        file: unsafe { File::from_raw_fd(fd) },
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let plan = plan(rsp, |address| memory.word(address))?;
    let name = canonical_execfn(plan.execfn, plan.stack_end, |address| {
        let mut byte = [0];
        memory.transfer(address, &mut byte, false)?;
        Ok(byte[0])
    })?;
    for address in plan.credentials.into_iter().flatten() {
        memory.replace(address, &CREDENTIAL.to_le_bytes())?;
    }
    memory.replace(plan.execfn, &name)?;
    memory.replace(plan.random, &SEED)?;
    if let Some(address) = plan.sysinfo {
        memory.replace(address, &0u64.to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn words() -> Vec<u64> {
        vec![
            1, 0x1800, 0, 0x1810, 0, 3, 0x400040, 4, 56, 5, 8, 6, 4096, 7, 0, 9, 0x401000, 25,
            0x1700, 33, 0x7fff0000, 31, 0x1900, 0, 0,
        ]
    }
    fn inspect(words: &[u64]) -> io::Result<Plan> {
        plan(0x1000, |address| {
            words
                .get(((address - 0x1000) / 8) as usize)
                .copied()
                .ok_or_else(|| io::Error::new(io::ErrorKind::UnexpectedEof, "short fixture stack"))
        })
    }
    #[test]
    fn controlled_link_launch_auxv_shapes_are_bounded_and_preserve_pointer_fields() {
        let original = words();
        let result = inspect(&original).unwrap();
        assert_eq!(result.random, 0x1700);
        assert_eq!(result.sysinfo, Some(0x1000 + 20 * 8));
        assert_eq!(result.execfn, 0x1900);
        assert_eq!(
            original,
            words(),
            "inspection must not rewrite pointer-bearing entries"
        );
        for (index, replacement) in [
            (0, 0),
            (0, MAX_POINTERS + 1),
            (1, 0),
            (2, 0x1800),
            (3, USER_END),
            (7, 25),  // duplicate random
            (17, 26), // missing random
            (18, 0x1000),
            (18, 0x17ff),
            (18, u64::MAX),
            (8, 32),        // incompatible PHENT
            (11, 99),       // unsupported auxiliary kind
            (21, 30),       // missing EXECFN
            (22, 0x1000),   // EXECFN overlaps tables
            (22, USER_END), // EXECFN outside bounded stack
            (24, 1),        // nonzero AT_NULL
        ] {
            let mut malformed = words();
            malformed[index] = replacement;
            assert!(
                inspect(&malformed).is_err(),
                "accepted malformed word {index}={replacement:#x}"
            );
        }
        let mut truncated = words();
        truncated.truncate(21);
        assert!(inspect(&truncated).is_err());
        assert!(plan(u64::MAX - 15, |_| Ok(0)).is_err());
        assert!(plan(0x1008, |_| Ok(0)).is_err());
        // No vDSO entry is a valid kernel configuration. It remains absent.
        let mut absent = words();
        absent.drain(19..21);
        assert!(inspect(&absent).unwrap().sysinfo.is_none());
    }

    #[test]
    fn controlled_link_launch_credentials_are_optional_bounded_value_slots() {
        assert_eq!(inspect(&words()).unwrap().credentials, [None; 4]);
        for mask in 0u8..16 {
            let mut fixture = words();
            fixture.truncate(fixture.len() - 2);
            let mut expected = [None; 4];
            for (index, slot) in expected.iter_mut().enumerate() {
                if mask & (1 << index) != 0 {
                    *slot = Some(0x1000 + (fixture.len() as u64 + 1) * 8);
                    fixture.extend_from_slice(&[11 + index as u64, u32::MAX as u64]);
                }
            }
            // Security state and CPU capabilities are not credentials.
            fixture.extend_from_slice(&[23, 1, 16, 0xdeadbeef, 0, 0]);
            let original = fixture.clone();
            assert_eq!(inspect(&fixture).unwrap().credentials, expected);
            assert_eq!(fixture, original);
        }
        for tag in 11..=14 {
            for value in [0, 1000, u32::MAX as u64] {
                let mut fixture = words();
                fixture.splice(23..23, [tag, value]);
                assert!(inspect(&fixture).is_ok());
            }
            for value in [u32::MAX as u64 + 1, u64::MAX] {
                let mut fixture = words();
                fixture.splice(23..23, [tag, value]);
                assert!(inspect(&fixture).is_err());
            }
            let mut duplicate = words();
            duplicate.splice(23..23, [tag, 0, tag, 1000]);
            assert!(inspect(&duplicate).is_err());
            let mut truncated = words();
            truncated.truncate(23);
            truncated.push(tag);
            assert!(inspect(&truncated).is_err());
        }
    }

    #[test]
    fn controlled_link_launch_execfn_replacement_is_bounded_and_clears_suffix() {
        let original = b"/private/tenant/scratch/lib64/ld-linux-x86-64.so.2\0";
        let bytes = canonical_execfn(0x1000, 0x2000, |address| {
            Ok(original[(address - 0x1000) as usize])
        })
        .unwrap();
        assert_eq!(bytes.len(), original.len());
        assert_eq!(&bytes[..EXECFN.len()], EXECFN);
        assert!(bytes[EXECFN.len()..].iter().all(|byte| *byte == 0));
        assert!(canonical_execfn(0x1000, 0x1001, |_| Ok(0)).is_err());
        assert!(canonical_execfn(0x1000, 0x1000, |_| unreachable!()).is_err());
        let mut reads = 0;
        assert!(
            canonical_execfn(0x1000, 0x10000, |_| {
                reads += 1;
                Ok(b'x')
            })
            .is_err()
        );
        assert_eq!(reads, 4096);
        assert!(
            canonical_execfn(0x1000, 0x2000, |_| Err(io::Error::from_raw_os_error(14))).is_err()
        );
    }
}
