//! Versioned, authority-free native object cache representation.
//!
//! All integers are little-endian u64. Vector order and duplicate entries are
//! preserved exactly. Receipts and vtables are never serialized.
use super::{backend, ssa};
use ssa::meter::{self, Meter};
use ssa::store::Owned;
use std::mem::size_of;

const MAGIC: &[u8; 8] = b"JROBJ\0\0\x01";
const LIMIT: usize = 64 * 1024 * 1024;

fn field_fits(offset: usize, width: usize, code: usize) -> bool {
    offset.checked_add(width).is_some_and(|end| end <= code)
}

fn bounded_add(total: &mut usize, amount: usize) -> Option<()> {
    *total = total.checked_add(amount)?;
    (*total <= LIMIT).then_some(())
}

/// Ineligible or malformed objects are cache misses, not compiler errors.
pub fn encode(
    object: &backend::Object,
    meter: &Meter<'_>,
) -> Result<Option<Owned<Vec<u8>>>, String> {
    if !object.vtables.is_empty() || object.code.len() > LIMIT {
        return Ok(None);
    }
    let mut len = MAGIC.len() + 32;
    let mut storage = publication_bytes();
    if bounded_add(&mut storage, object.code.len()).is_none() {
        return Ok(None);
    }
    if bounded_add(&mut len, object.code.len()).is_none() {
        return Ok(None);
    }
    for (entries, width) in [(&object.exports, 0), (&object.calls, 4)] {
        meter.work(entries.len())?;
        if entries
            .len()
            .checked_mul(size_of::<(usize, String)>())
            .and_then(|bytes| bounded_add(&mut storage, bytes))
            .is_none()
        {
            return Ok(None);
        }
        for (offset, symbol) in entries {
            if (width == 0 && *offset >= object.code.len())
                || !field_fits(*offset, width, object.code.len())
                || bounded_add(&mut len, 16).is_none()
                || bounded_add(&mut len, symbol.len()).is_none()
                || bounded_add(&mut storage, symbol.len()).is_none()
            {
                return Ok(None);
            }
        }
    }
    meter.work(object.strings.len())?;
    if object
        .strings
        .len()
        .checked_mul(size_of::<(usize, Vec<u8>)>())
        .and_then(|bytes| bounded_add(&mut storage, bytes))
        .is_none()
    {
        return Ok(None);
    }
    for (offset, text) in &object.strings {
        if !field_fits(*offset, 8, object.code.len())
            || bounded_add(&mut len, 16).is_none()
            || bounded_add(&mut len, text.len()).is_none()
            || bounded_add(&mut storage, text.len()).is_none()
        {
            return Ok(None);
        }
    }
    meter.work(len)?;
    let (mut data, lease) = meter.vec::<u8>(len)?;
    data.extend_from_slice(MAGIC);
    put(&mut data, object.code.len());
    data.extend_from_slice(&object.code);
    for entries in [&object.exports, &object.calls] {
        put(&mut data, entries.len());
        for (offset, symbol) in entries {
            put(&mut data, *offset);
            put(&mut data, symbol.len());
            data.extend_from_slice(symbol.as_bytes());
        }
    }
    put(&mut data, object.strings.len());
    for (offset, text) in &object.strings {
        put(&mut data, *offset);
        put(&mut data, text.len());
        data.extend_from_slice(text);
    }
    let capacity = data.capacity();
    Owned::adopt(lease, data, capacity, meter).map(Some)
}

fn put(data: &mut Vec<u8>, n: usize) {
    data.extend_from_slice(&(n as u64).to_le_bytes());
}

#[derive(Clone, Copy)]
struct Reader<'a> {
    rest: &'a [u8],
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let value = self.rest.get(..n)?;
        self.rest = &self.rest[n..];
        Some(value)
    }
    fn number(&mut self) -> Option<usize> {
        usize::try_from(u64::from_le_bytes(self.take(8)?.try_into().ok()?)).ok()
    }
    fn blob(&mut self) -> Option<&'a [u8]> {
        let n = self.number()?;
        self.take(n)
    }
    fn count(&mut self) -> Option<usize> {
        let n = self.number()?;
        (n <= self.rest.len() / 16).then_some(n)
    }
}

// Allocation-free validation of the complete stream, including expanded
// in-memory tuple storage. No input-controlled allocation precedes this pass.
fn validate(bytes: &[u8]) -> Option<()> {
    if bytes.len() > LIMIT {
        return None;
    }
    let mut r = Reader { rest: bytes };
    if r.take(MAGIC.len())? != MAGIC {
        return None;
    }
    let code = r.blob()?.len();
    let mut storage = publication_bytes();
    bounded_add(&mut storage, code)?;
    for width in [0, 4, 8] {
        let count = r.count()?;
        let slot = if width == 8 {
            size_of::<(usize, Vec<u8>)>()
        } else {
            size_of::<(usize, String)>()
        };
        bounded_add(&mut storage, count.checked_mul(slot)?)?;
        for _ in 0..count {
            let offset = r.number()?;
            let value = r.blob()?;
            if (width == 0 && offset >= code) || !field_fits(offset, width, code) {
                return None;
            }
            if width != 8 {
                std::str::from_utf8(value).ok()?;
            }
            bounded_add(&mut storage, value.len())?;
        }
    }
    r.rest.is_empty().then_some(())
}

fn copy_bytes(bytes: &[u8], meter: &Meter<'_>) -> Result<Owned<Vec<u8>>, String> {
    let (mut data, lease) = meter.vec(bytes.len())?;
    data.extend_from_slice(bytes);
    let capacity = data.capacity();
    Owned::adopt(lease, data, capacity, meter)
}

fn symbols(r: &mut Reader<'_>, meter: &Meter<'_>) -> Result<Owned<Vec<(usize, String)>>, String> {
    let count = r.count().expect("validated count");
    let (data, lease) = meter.vec::<(usize, String)>(count)?;
    let capacity = meter::mul(data.capacity(), size_of::<(usize, String)>())?;
    let mut out = Owned::adopt(lease, data, capacity, meter)?;
    for _ in 0..count {
        let offset = r.number().expect("validated offset");
        let bytes = copy_bytes(r.blob().expect("validated symbol"), meter)?;
        out = Owned::combine(out, bytes, meter, |mut entries, bytes| {
            entries.push((offset, String::from_utf8(bytes).expect("validated UTF-8")));
            entries
        })?;
    }
    Ok(out)
}

/// Decode fresh paid storage. The caller, not this codec, seals graph evidence.
pub fn decode(bytes: &[u8], meter: &Meter<'_>) -> Result<Option<Owned<backend::Object>>, String> {
    if bytes.len() > LIMIT {
        return Ok(None);
    }
    meter.work(bytes.len())?;
    if validate(bytes).is_none() {
        return Ok(None);
    }
    let mut r = Reader { rest: bytes };
    r.take(MAGIC.len()).expect("validated magic");
    let code = copy_bytes(r.blob().expect("validated code"), meter)?;
    let exports = symbols(&mut r, meter)?;
    let calls = symbols(&mut r, meter)?;
    let count = r.count().expect("validated strings");
    let (data, lease) = meter.vec::<(usize, Vec<u8>)>(count)?;
    let capacity = meter::mul(data.capacity(), size_of::<(usize, Vec<u8>)>())?;
    let mut strings = Owned::adopt(lease, data, capacity, meter)?;
    for _ in 0..count {
        let offset = r.number().expect("validated offset");
        let value = copy_bytes(r.blob().expect("validated string"), meter)?;
        strings = Owned::combine(strings, value, meter, |mut entries, value| {
            entries.push((offset, value));
            entries
        })?;
    }
    let object = Owned::combine(code, exports, meter, |code, exports| backend::Object {
        code,
        exports,
        calls: Vec::new(),
        strings: Vec::new(),
        vtables: Vec::new(),
        receipt: None,
    })?;
    let object = Owned::combine(object, calls, meter, |mut object, calls| {
        object.calls = calls;
        object
    })?;
    let mut object = Owned::combine(object, strings, meter, |mut object, strings| {
        object.strings = strings;
        object
    })?;
    // Pay the eventual graph-owned Arc<Object> and Arc<Receipt> envelopes,
    // matching machine::object_bytes, plus exports which that emitter omits.
    object.admit(meter, publication_bytes())?;
    Ok(Some(object))
}

fn publication_bytes() -> usize {
    size_of::<backend::Object>() + size_of::<meter::Receipt>() + 4 * size_of::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::ir::{PlanBudget, PlanLimits};

    fn sample() -> backend::Object {
        backend::Object {
            code: vec![0; 16],
            exports: vec![(0, "entry_λ".into())],
            calls: vec![(12, "callee".into())],
            strings: vec![(8, vec![0, 255, 1])],
            vtables: Vec::new(),
            receipt: None,
        }
    }

    #[test]
    fn roundtrip_deterministic_fresh_accounting_and_no_receipt() {
        let source_budget = PlanBudget::default();
        let source_meter = Meter::new(&source_budget);
        let mut source = sample();
        let (_, receipt) = copy_bytes(&[1, 2, 3], &source_meter)
            .unwrap()
            .seal(&source_meter)
            .unwrap();
        source.receipt = Some(std::sync::Arc::new(receipt));
        let bytes = encode(&source, &source_meter).unwrap().unwrap();
        source.receipt = None;
        let plain = encode(&source, &source_meter).unwrap().unwrap();
        assert_eq!(bytes.value, plain.value);
        let budget = PlanBudget::default();
        let meter = Meter::new(&budget);
        let decoded = decode(&bytes.value, &meter).unwrap().unwrap();
        assert!(decoded.value.receipt.is_none());
        assert_eq!(decoded.value.code, source.code);
        assert_eq!(decoded.value.exports, source.exports);
        assert_eq!(decoded.value.calls, source.calls);
        assert_eq!(decoded.value.strings, source.strings);
        // The live owner also pays lease chunk bookkeeping, not included in
        // Owned::bytes(). Verify exact payload accounting after seal below.
        assert!(budget.usage_snapshot().bytes >= decoded.bytes());
        assert!(decoded.bytes() > 0);
        let again = encode(&decoded.value, &meter).unwrap().unwrap();
        assert_eq!(again.value, bytes.value);
        drop(again);
        drop(decoded);
        assert_eq!(budget.usage_snapshot().bytes, 0);

        let decoded = decode(&bytes.value, &meter).unwrap().unwrap();
        let payload_bytes = decoded.bytes();
        let (object, receipt) = decoded.seal(&meter).unwrap();
        assert!(object.receipt.is_none());
        assert_eq!(receipt.bytes(), payload_bytes);
        assert_eq!(budget.usage_snapshot().bytes, payload_bytes);
    }

    #[test]
    fn rejects_every_truncation_trailing_version_and_absurd_sizes() {
        let budget = PlanBudget::default();
        let meter = Meter::new(&budget);
        let bytes = encode(&sample(), &meter).unwrap().unwrap();
        for end in 0..bytes.value.len() {
            assert!(decode(&bytes.value[..end], &meter).unwrap().is_none());
        }
        let mut bad = bytes.value.clone();
        bad.push(0);
        assert!(decode(&bad, &meter).unwrap().is_none());
        bad = bytes.value.clone();
        bad[7] = 2;
        assert!(decode(&bad, &meter).unwrap().is_none());
        for offset in [8, 32, 48] {
            bad = bytes.value.clone();
            bad[offset..offset + 8].copy_from_slice(&u64::MAX.to_le_bytes());
            assert!(decode(&bad, &meter).unwrap().is_none());
        }
    }

    #[test]
    fn rejects_offsets_and_invalid_utf8() {
        let budget = PlanBudget::default();
        let meter = Meter::new(&budget);
        for which in 0..3 {
            let mut object = sample();
            match which {
                0 => object.exports[0].0 = 16,
                1 => object.calls[0].0 = 13,
                _ => object.strings[0].0 = 9,
            }
            assert!(encode(&object, &meter).unwrap().is_none());
        }
        let bytes = encode(&sample(), &meter).unwrap().unwrap();
        // Header + code blob + export count, then export offset and length.
        for (offset, invalid) in [(40, 16u64), (72, 13), (102, 9)] {
            let mut bad = bytes.value.clone();
            bad[offset..offset + 8].copy_from_slice(&invalid.to_le_bytes());
            assert!(decode(&bad, &meter).unwrap().is_none());
        }
        let mut bad = bytes.value.clone();
        bad[56] = 255;
        assert!(decode(&bad, &meter).unwrap().is_none());
    }

    #[test]
    fn exhaustion_is_an_error_not_a_miss() {
        let budget = PlanBudget::default();
        let bytes = encode(&sample(), &Meter::new(&budget)).unwrap().unwrap();
        let tight = PlanBudget::new(PlanLimits {
            bytes: 0,
            ..PlanLimits::default()
        });
        assert!(decode(&bytes.value, &Meter::new(&tight)).is_err());
        assert_eq!(tight.usage_snapshot().bytes, 0);
        let tight = PlanBudget::new(PlanLimits {
            bytes: 0,
            ..PlanLimits::default()
        });
        assert!(encode(&sample(), &Meter::new(&tight)).is_err());
    }
}
