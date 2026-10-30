//! First persistent reuse boundary: the native linker's immutable byte output.
//!
//! This runs inside BUILD after its ordinary graph reads and CHECK gate. It
//! does not restore semantic proofs, authority tokens, or snapshot-local keys.
//! Ordered linker inputs are fingerprinted explicitly, not through Debug text.
//! Vtables are deliberately ineligible until their semantic identities have a
//! stable encoding. External linkers and C ABI inputs use their existing path.
use super::{backend, ir, procfs};
use crate::persistent_cache::{KeyBuilder, LocalCache, MAX_ENTRY_BYTES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Disabled,
    Ineligible,
    Hit,
    MissStored,
    MissNotStored,
}
impl crate::engine::Fact for Outcome {}

// Increment this domain when the input encoding or operation contract changes.
const DOMAIN: &[u8] = b"justrust/native-link/x86_64-linux/v1";

/// All semantic inputs used by backend::link_with_panic, except vtables (which
/// cause a miss-by-ineligibility). Storage receipts are accounting evidence,
/// not linker inputs. Ordering, vector boundaries and offsets are significant.
fn identity(
    functions: &[(String, backend::Object)],
    unwind: bool,
    compiler: &[u8; 32],
) -> Option<([u8; 32], usize, usize)> {
    let mut key = KeyBuilder::new(DOMAIN, compiler);
    key.field(&[u8::from(unwind)]);
    key.field(&(functions.len() as u64).to_le_bytes());
    let mut code = if unwind { 29usize } else { 16usize };
    let mut data = 0usize;
    let mut work = 0usize;
    for (name, object) in functions {
        if !object.vtables.is_empty() {
            return None;
        }
        code = code.checked_add(object.code.len())?;
        work = work
            .checked_add(name.len())?
            .checked_add(object.code.len())?;
        // Bound hashing work and output size before traversing payloads.
        if code > MAX_ENTRY_BYTES || work > MAX_ENTRY_BYTES {
            return None;
        }
        key.field(name.as_bytes());
        key.field(&object.code);
        for entries in [&object.exports, &object.calls] {
            key.field(&(entries.len() as u64).to_le_bytes());
            for (offset, symbol) in entries {
                work = work.checked_add(8)?.checked_add(symbol.len())?;
                if work > MAX_ENTRY_BYTES {
                    return None;
                }
                key.field(&(*offset as u64).to_le_bytes());
                key.field(symbol.as_bytes());
            }
        }
        key.field(&(object.strings.len() as u64).to_le_bytes());
        for (offset, bytes) in &object.strings {
            data = data.checked_add(bytes.len())?;
            work = work.checked_add(8)?.checked_add(bytes.len())?;
            if data > MAX_ENTRY_BYTES || work > MAX_ENTRY_BYTES {
                return None;
            }
            key.field(&(*offset as u64).to_le_bytes());
            key.field(bytes);
        }
    }
    // The native emitter places code after one 4096-byte header page and
    // rounds the read-only data start up to a page boundary.
    let length = 4096usize
        .checked_add(code)?
        .checked_add(4095)?
        .checked_div(4096)?
        .checked_mul(4096)?
        .checked_add(data)?;
    (length <= MAX_ENTRY_BYTES).then(|| (key.finish(), length, work))
}

pub(super) fn link(
    functions: &[(String, backend::Object)],
    unwind: bool,
    cache: Option<&LocalCache>,
    compiler: Option<&procfs::CompilerImage>,
    budget: &ir::PlanBudget,
) -> (Result<Vec<u8>, String>, Outcome) {
    let Some(cache) = cache else {
        return (
            backend::link_with_panic(functions, unwind),
            Outcome::Disabled,
        );
    };
    let Some((key, expected_length, work)) =
        compiler.and_then(|compiler| identity(functions, unwind, compiler.implementation_digest()))
    else {
        return (
            backend::link_with_panic(functions, unwind),
            Outcome::Ineligible,
        );
    };
    let attempt = || -> Result<(Vec<u8>, Outcome), String> {
        budget.charge_work(work)?;
        // Admission precedes decoding. The bound scales with this node's own
        // input, not a global cache allocation allowance.
        let mut storage = budget.reserve_bytes(expected_length)?;
        if let Some(bytes) = cache.load_bounded(&key, expected_length)
            && bytes.len() == expected_length
        {
            budget.charge_work(bytes.len())?;
            storage.retain()?;
            return Ok((bytes, Outcome::Hit));
        }
        let bytes = backend::link_with_panic(functions, unwind)?;
        if bytes.len() != expected_length {
            // An encoding/backend drift must never publish a cache record.
            storage.shrink(expected_length)?;
            budget.retain_bytes(bytes.len())?;
            return Ok((bytes, Outcome::MissNotStored));
        }
        budget.charge_work(bytes.len())?;
        let outcome = if cache.store(&key, &bytes).is_ok() {
            Outcome::MissStored
        } else {
            Outcome::MissNotStored
        };
        storage.retain()?;
        Ok((bytes, outcome))
    };
    match attempt() {
        Ok((bytes, outcome)) => (Ok(bytes), outcome),
        Err(error) => (Err(error), Outcome::MissNotStored),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn object() -> backend::Object {
        backend::Object {
            code: vec![0; 16],
            exports: vec![],
            calls: vec![],
            strings: vec![],
            vtables: vec![],
            receipt: None,
        }
    }
    #[test]
    fn identity_covers_order_configuration_offsets_and_payloads() {
        let base = vec![("main".into(), object()), ("f".into(), object())];
        let fingerprint =
            |objects: &[(String, backend::Object)]| identity(objects, false, &[1; 32]).unwrap().0;
        let original = fingerprint(&base);
        assert_ne!(original, identity(&base, true, &[1; 32]).unwrap().0);
        assert_ne!(original, identity(&base, false, &[2; 32]).unwrap().0);
        for mutation in 0..8 {
            let mut changed = base.clone();
            match mutation {
                0 => changed.reverse(),
                1 => changed[0].0.push('x'),
                2 => changed[0].1.code[0] = 1,
                3 => changed[0].1.exports.push((0, "export".into())),
                4 => changed[0].1.calls.push((0, "f".into())),
                5 => changed[0].1.calls.push((1, "f".into())),
                6 => changed[0].1.strings.push((0, b"abc".to_vec())),
                7 => changed[0].1.strings.push((1, b"abc".to_vec())),
                _ => unreachable!(),
            }
            assert_ne!(original, fingerprint(&changed), "mutation {mutation}");
        }
        for unwind in [false, true] {
            let (_, expected, _) = identity(&base, unwind, &[1; 32]).unwrap();
            assert_eq!(
                expected,
                backend::link_with_panic(&base, unwind).unwrap().len()
            );
            for size in [0, 4066, 4067, 4068, 4080, 4096, 8176] {
                let mut object = object();
                object.code.resize(size, 0);
                let functions = vec![("main".into(), object)];
                let (_, expected, _) = identity(&functions, unwind, &[1; 32]).unwrap();
                assert_eq!(
                    expected,
                    backend::link_with_panic(&functions, unwind).unwrap().len(),
                    "size={size}, unwind={unwind}",
                );
            }
        }
    }
}
