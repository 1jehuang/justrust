//! ELF64 relocatable objects and explicit native-link inputs. No assembler or
//! Rust compiler is involved. `cc` is used only as a system linker driver.
use crate::compiler::{backend::Object, ir};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, Default)]
pub struct LinkOptions {
    pub emit_object: bool,
    /// Immutable object/archive snapshots captured by the caller before graph execution.
    pub objects: Vec<Vec<u8>>,
    pub libraries: Vec<String>,
}
fn u16_(v: &mut [u8], at: usize, n: u16) {
    v[at..at + 2].copy_from_slice(&n.to_le_bytes());
}
fn u32_(v: &mut [u8], at: usize, n: u32) {
    v[at..at + 4].copy_from_slice(&n.to_le_bytes());
}
fn u64_(v: &mut [u8], at: usize, n: u64) {
    v[at..at + 8].copy_from_slice(&n.to_le_bytes());
}
fn native_name(name: &str) -> &str {
    if name == "main" {
        "__jr_private_main"
    } else {
        name
    }
}
fn relocation(out: &mut Vec<u8>, offset: usize, symbol: u32, kind: u32, addend: i64) {
    out.extend_from_slice(&(offset as u64).to_le_bytes());
    out.extend_from_slice(&((u64::from(symbol) << 32) | u64::from(kind)).to_le_bytes());
    out.extend_from_slice(&addend.to_le_bytes());
}
fn symbol(out: &mut Vec<u8>, name: u32, info: u8, section: u16, value: usize, size: usize) {
    let at = out.len();
    out.resize(at + 24, 0);
    u32_(out, at, name);
    out[at + 4] = info;
    u16_(out, at + 6, section);
    u64_(out, at + 8, value as u64);
    u64_(out, at + 16, size as u64);
}
fn text_name(strings: &mut Vec<u8>, name: &str) -> Result<u32, String> {
    if name.is_empty() || name.as_bytes().contains(&0) {
        return Err("invalid ELF symbol name".into());
    }
    let at = u32::try_from(strings.len()).map_err(|_| "ELF string table overflow")?;
    strings.extend_from_slice(name.as_bytes());
    strings.push(0);
    Ok(at)
}
pub fn object(
    functions: &[(String, Object)],
    entry: bool,
    budget: &ir::PlanBudget,
) -> Result<Vec<u8>, String> {
    let bound = functions.iter().try_fold(4096usize, |n, (name, o)| {
        let reloc = o
            .calls
            .len()
            .checked_add(o.strings.len())
            .and_then(|n| {
                n.checked_add(
                    o.vtables
                        .iter()
                        .map(|(_, v)| v.methods.len() + 1)
                        .sum::<usize>(),
                )
            })
            .ok_or("ELF relocation count overflow")?;
        n.checked_add(o.code.len())
            .and_then(|n| n.checked_add(o.strings.iter().map(|(_, s)| s.len()).sum::<usize>()))
            .and_then(|n| {
                n.checked_add(
                    reloc * 512
                        + name.len() * 4
                        + o.exports
                            .iter()
                            .map(|(_, s)| s.len() * 4 + 128)
                            .sum::<usize>(),
                )
            })
            .ok_or("ELF object size overflow")
    })?;
    budget.charge_work(bound)?;
    budget.retain_bytes(bound.checked_mul(3).ok_or("ELF accounting overflow")?)?;
    let mut text = Vec::new();
    let mut data = Vec::new();
    let mut definitions = BTreeMap::new();
    if entry {
        // A real SysV main lets the host CRT initialize libc and native
        // constructors and flush C streams. The private main may clobber
        // callee-saved registers, so preserve them across the bridge.
        definitions.insert("main".to_string(), (0usize, 39usize));
        text.extend_from_slice(&[
            0x55, 0x48, 0x89, 0xe5, 0x53, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48,
            0x83, 0xec, 0x08, 0xe8, 0, 0, 0, 0, 0x48, 0x83, 0xc4, 0x08, 0x41, 0x5f, 0x41, 0x5e,
            0x41, 0x5d, 0x41, 0x5c, 0x5b, 0x31, 0xc0, 0xc9, 0xc3,
        ]);
    }
    let mut starts = Vec::with_capacity(functions.len());
    for (name, o) in functions {
        let at = text.len();
        starts.push(at);
        if definitions
            .insert(native_name(name).to_string(), (at, o.code.len()))
            .is_some()
        {
            return Err(format!("duplicate ELF symbol `{name}`"));
        }
        for (offset, name) in &o.exports {
            if *offset >= o.code.len() {
                return Err("invalid C export offset".into());
            }
            if definitions
                .insert(name.clone(), (at + offset, o.code.len() - offset))
                .is_some()
            {
                return Err(format!("duplicate ELF symbol `{name}`"));
            }
        }
        text.extend_from_slice(&o.code);
    }
    let mut undefined = BTreeSet::new();
    if entry && !definitions.contains_key("__jr_private_main") {
        return Err("missing main function".into());
    }
    for (_, o) in functions {
        for (_, name) in &o.calls {
            if !definitions.contains_key(native_name(name)) {
                undefined.insert(native_name(name).to_string());
            }
        }
        for (_, v) in &o.vtables {
            for name in v.drop.iter().chain(v.methods.iter()) {
                if !definitions.contains_key(native_name(name)) {
                    undefined.insert(native_name(name).to_string());
                }
            }
        }
    }
    let mut strings = vec![0];
    let mut symbols = vec![0; 24];
    symbol(&mut symbols, 0, 3, 2, 0, 0); // local .rodata section symbol, index 1
    symbol(&mut symbols, 0, 3, 1, 0, 0); // local .text section symbol, index 2
    let mut indices = BTreeMap::new();
    for (name, (value, size)) in &definitions {
        let index = (symbols.len() / 24) as u32;
        indices.insert(name.clone(), index);
        let str_ = text_name(&mut strings, name)?;
        symbol(&mut symbols, str_, 0x12, 1, *value, *size);
    }
    for name in undefined {
        let index = (symbols.len() / 24) as u32;
        indices.insert(name.clone(), index);
        let str_ = text_name(&mut strings, &name)?;
        symbol(&mut symbols, str_, 0x10, 0, 0, 0);
    }
    let mut rel_text = Vec::new();
    let mut rel_data = Vec::new();
    if entry {
        relocation(&mut rel_text, 18, indices["__jr_private_main"], 4, -4);
    }
    for ((_, o), start) in functions.iter().zip(starts) {
        for (at, name) in &o.calls {
            if at.checked_add(4).is_none_or(|end| end > o.code.len()) {
                return Err("invalid call relocation".into());
            }
            relocation(&mut rel_text, start + at, indices[native_name(name)], 4, -4);
        }
        for (at, bytes) in &o.strings {
            if at.checked_add(8).is_none_or(|end| end > o.code.len()) {
                return Err("invalid data relocation".into());
            }
            relocation(&mut rel_text, start + at, 1, 1, data.len() as i64);
            data.extend_from_slice(bytes);
        }
        for (at, vtable) in &o.vtables {
            if at.checked_add(8).is_none_or(|end| end > o.code.len()) {
                return Err("invalid vtable relocation".into());
            }
            while data.len() % 8 != 0 {
                data.push(0);
            }
            relocation(&mut rel_text, start + at, 1, 1, data.len() as i64);
            let drop_at = data.len();
            data.extend_from_slice(&[0; 8]);
            if let Some(name) = &vtable.drop {
                relocation(&mut rel_data, drop_at, indices[native_name(name)], 1, 0);
            }
            data.extend_from_slice(&vtable.size.to_le_bytes());
            data.extend_from_slice(&vtable.align.to_le_bytes());
            for name in &vtable.methods {
                let at = data.len();
                data.extend_from_slice(&[0; 8]);
                relocation(&mut rel_data, at, indices[native_name(name)], 1, 0);
            }
        }
    }
    let names = [
        "",
        ".text",
        ".rodata",
        ".rela.text",
        ".rela.rodata",
        ".symtab",
        ".strtab",
        ".shstrtab",
        ".note.GNU-stack",
    ];
    let mut section_names = vec![0];
    let mut name_offsets = vec![0u32];
    for name in names.iter().skip(1) {
        name_offsets.push(text_name(&mut section_names, name)?);
    }
    let sections = [
        Vec::new(),
        text,
        data,
        rel_text,
        rel_data,
        symbols,
        strings,
        section_names,
        Vec::new(),
    ];
    let mut result = vec![0; 64];
    let mut offsets = Vec::new();
    for section in &sections {
        while !result.len().is_multiple_of(8) {
            result.push(0);
        }
        offsets.push(result.len());
        result.extend_from_slice(section);
    }
    while !result.len().is_multiple_of(8) {
        result.push(0);
    }
    let headers = result.len();
    result.resize(headers + sections.len() * 64, 0);
    result[..16].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    u16_(&mut result, 16, 1);
    u16_(&mut result, 18, 62);
    u32_(&mut result, 20, 1);
    u64_(&mut result, 40, headers as u64);
    u16_(&mut result, 52, 64);
    u16_(&mut result, 58, 64);
    u16_(&mut result, 60, sections.len() as u16);
    u16_(&mut result, 62, 7);
    for n in 1..sections.len() {
        let at = headers + n * 64;
        u32_(&mut result, at, name_offsets[n]);
        u32_(
            &mut result,
            at + 4,
            match n {
                3 | 4 => 4,
                5 => 2,
                6 | 7 => 3,
                _ => 1,
            },
        );
        u64_(
            &mut result,
            at + 8,
            match n {
                1 => 6,
                2 => 2,
                _ => 0,
            },
        );
        u64_(&mut result, at + 24, offsets[n] as u64);
        u64_(&mut result, at + 32, sections[n].len() as u64);
        u64_(&mut result, at + 48, if n == 1 { 16 } else { 8 });
        if n == 3 || n == 4 {
            u32_(&mut result, at + 40, 5);
            u32_(&mut result, at + 44, if n == 3 { 1 } else { 2 });
            u64_(&mut result, at + 56, 24);
        }
        if n == 5 {
            u32_(&mut result, at + 40, 6);
            u32_(&mut result, at + 44, 3);
            u64_(&mut result, at + 56, 24);
        }
    }
    Ok(result)
}
struct Scratch(std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn scratch() -> Result<Scratch, String> {
    let root = std::env::temp_dir();
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("infrastructure: {e}"))?
        .as_nanos();
    for attempt in 0..32 {
        let path = root.join(format!(
            "jr-native-link-{}-{time}-{attempt}",
            std::process::id()
        ));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(Scratch(path)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("infrastructure: native linker scratch: {e}")),
        }
    }
    Err("infrastructure: native linker scratch collisions".into())
}
pub fn link(
    functions: &[(String, Object)],
    options: &LinkOptions,
    budget: &ir::PlanBudget,
) -> Result<Vec<u8>, String> {
    let bytes = object(functions, !options.emit_object, budget)?;
    if options.emit_object {
        return Ok(bytes);
    }
    let scratch = scratch()?;
    let own = scratch.0.join("native.o");
    let output = scratch.0.join("program");
    std::fs::write(&own, bytes).map_err(|e| format!("infrastructure: write linker input: {e}"))?;
    let mut command = std::process::Command::new("cc");
    command
        .args(["-no-pie", "-Wl,-z,noexecstack", "-o"])
        .arg(&output)
        .arg(own);
    for (index, bytes) in options.objects.iter().enumerate() {
        if !bytes.starts_with(b"\x7fELF") && !bytes.starts_with(b"!<arch>\n") {
            return Err(
                "unsupported: native input must be an ELF object or ar archive snapshot".into(),
            );
        }
        let path = scratch.0.join(format!("input-{index}.o"));
        std::fs::write(&path, bytes)
            .map_err(|e| format!("infrastructure: write native snapshot: {e}"))?;
        command.arg(path);
    }
    for library in &options.libraries {
        if library.is_empty()
            || !library
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        {
            return Err("unsupported: invalid native library name".into());
        }
        command.arg(format!("-l{library}"));
    }
    let status = command
        .output()
        .map_err(|e| format!("infrastructure: invoke native linker: {e}"))?;
    if !status.status.success() {
        return Err(format!(
            "native link failed ({}):\n{}{}",
            status.status,
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&status.stderr)
        ));
    }
    let metadata =
        std::fs::metadata(&output).map_err(|e| format!("infrastructure: linked output: {e}"))?;
    budget.retain_bytes(
        usize::try_from(metadata.len()).map_err(|_| "native executable too large")?,
    )?;
    std::fs::read(output).map_err(|e| format!("infrastructure: read linked executable: {e}"))
}
