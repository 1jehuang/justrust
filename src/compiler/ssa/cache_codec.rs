//! Versioned, exact SSA bytes. This format contains no authorities or callee table.
//! Decoding is structural only: callers must verify against their current callees.
use super::meter::{Lease, Meter};
use super::store::Owned;
use super::*;
use crate::compiler::ir::RuntimeLocation;
use std::mem::size_of;
use std::sync::Arc;

const MAGIC: &[u8] = b"JRSSA\0\x01";
const LIMIT: usize = 64 * 1024 * 1024;

struct Writer<'a, 'm> {
    meter: &'a Meter<'m>,
    bytes: Option<&'a mut [u8]>,
    pos: usize,
}
impl Writer<'_, '_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), String> {
        self.meter.work(bytes.len())?;
        let end = self
            .pos
            .checked_add(bytes.len())
            .filter(|n| *n <= LIMIT)
            .ok_or_else(|| "SSA codec size limit exceeded".to_string())?;
        if let Some(out) = &mut self.bytes {
            out[self.pos..end].copy_from_slice(bytes);
        }
        self.pos = end;
        Ok(())
    }
    fn byte(&mut self, n: u8) -> Result<(), String> {
        self.put(&[n])
    }
    fn u32(&mut self, n: u32) -> Result<(), String> {
        self.put(&n.to_le_bytes())
    }
    fn len(&mut self, n: usize) -> Result<(), String> {
        self.u32(u32::try_from(n).map_err(|_| "SSA codec length limit exceeded".to_string())?)
    }
    fn text(&mut self, s: &str) -> Result<(), String> {
        self.len(s.len())?;
        self.put(s.as_bytes())
    }
    fn ty(&mut self, t: SsaTy) -> Result<(), String> {
        match t {
            SsaTy::Unit => self.byte(0),
            SsaTy::Bool => self.byte(1),
            SsaTy::Int(t) => {
                self.byte(2)?;
                self.byte(t.bits())?;
                self.byte(u8::from(t.signed()))
            }
        }
    }
    fn location(&mut self, l: &Option<Arc<RuntimeLocation>>) -> Result<(), String> {
        match l {
            None => self.byte(0),
            Some(l) => {
                let RuntimeLocation { path, line, column } = l.as_ref();
                self.byte(1)?;
                self.text(path)?;
                self.put(&(*line as u64).to_le_bytes())?;
                self.put(&(*column as u64).to_le_bytes())
            }
        }
    }
    fn values(&mut self, values: &[SsaValue]) -> Result<(), String> {
        self.len(values.len())?;
        for v in values {
            self.u32(v.0)?;
        }
        Ok(())
    }
    fn target(&mut self, t: &SsaTarget) -> Result<(), String> {
        let SsaTarget { block, args } = t;
        self.u32(block.0)?;
        self.values(args)
    }
    fn inst(&mut self, i: &Inst) -> Result<(), String> {
        let Inst {
            runtime_location,
            result,
            kind,
        } = i;
        self.location(runtime_location)?;
        self.u32(result.0)?;
        match kind {
            InstKind::Const { ty, bits } => {
                self.byte(0)?;
                self.ty(*ty)?;
                self.put(&bits.to_le_bytes())
            }
            InstKind::Binary {
                op,
                overflow,
                lhs,
                rhs,
            } => {
                self.byte(1)?;
                self.byte(bin_tag(*op))?;
                self.byte(overflow_tag(*overflow))?;
                self.u32(lhs.0)?;
                self.u32(rhs.0)
            }
            InstKind::Unary {
                op,
                overflow,
                value,
            } => {
                self.byte(2)?;
                self.byte(un_tag(*op))?;
                self.byte(overflow_tag(*overflow))?;
                self.u32(value.0)
            }
            InstKind::Cmp { op, lhs, rhs } => {
                self.byte(3)?;
                self.byte(cmp_tag(*op))?;
                self.u32(lhs.0)?;
                self.u32(rhs.0)
            }
            InstKind::Cast { to, value } => {
                self.byte(4)?;
                self.ty(SsaTy::Int(*to))?;
                self.u32(value.0)
            }
            InstKind::Select {
                cond,
                then,
                otherwise,
            } => {
                self.byte(5)?;
                self.u32(cond.0)?;
                self.u32(then.0)?;
                self.u32(otherwise.0)
            }
            InstKind::Call { callee, args } => {
                self.byte(6)?;
                self.u32(callee.0)?;
                self.values(args)
            }
        }
    }
    fn term(&mut self, t: &SsaTerminator) -> Result<(), String> {
        match t {
            SsaTerminator::Jump(t) => {
                self.byte(0)?;
                self.target(t)
            }
            SsaTerminator::Branch {
                cond,
                then,
                otherwise,
            } => {
                self.byte(1)?;
                self.u32(cond.0)?;
                self.target(then)?;
                self.target(otherwise)
            }
            SsaTerminator::Return(v) => {
                self.byte(2)?;
                self.u32(v.0)
            }
            SsaTerminator::Panic(p) => {
                self.byte(3)?;
                self.byte(panic_tag(*p))?;
                if let PanicKind::Message(n) = p {
                    self.u32(*n)?;
                }
                Ok(())
            }
        }
    }
    fn function(&mut self, f: &SsaFunction) -> Result<(), String> {
        // No `..`: extending an SSA record requires a wire/key decision here,
        // independently of whatever defaults its decoder might gain.
        let SsaFunction {
            params,
            ret,
            blocks,
            panics,
        } = f;
        self.put(MAGIC)?;
        self.len(params.len())?;
        for t in params {
            self.ty(*t)?;
        }
        self.ty(*ret)?;
        self.len(blocks.len())?;
        for block in blocks {
            let BlockData {
                panic_location,
                params,
                insts,
                term,
            } = block;
            self.location(panic_location)?;
            self.len(params.len())?;
            for (v, t) in params {
                self.u32(v.0)?;
                self.ty(*t)?;
            }
            self.len(insts.len())?;
            for i in insts {
                self.inst(i)?;
            }
            self.term(term)?;
        }
        self.len(panics.len())?;
        for panic in panics {
            let SsaPanicText { message, location } = panic;
            self.text(message)?;
            self.text(location)?;
        }
        Ok(())
    }
}

macro_rules! tags {
    ($enc:ident, $dec:ident, $ty:ty, {$($variant:path => $tag:literal),+ $(,)?}) => {
        fn $enc(v: $ty) -> u8 { match v { $($variant => $tag),+ } }
        fn $dec(v: u8) -> Option<$ty> { match v { $($tag => Some($variant)),+, _ => None } }
    }
}
tags!(bin_tag, bin_read, BinOp, {BinOp::Add=>0,BinOp::Sub=>1,BinOp::Mul=>2,BinOp::Div=>3,BinOp::Rem=>4,BinOp::And=>5,BinOp::Or=>6,BinOp::Xor=>7,BinOp::Shl=>8,BinOp::Shr=>9});
tags!(un_tag, un_read, UnOp, {UnOp::Neg=>0,UnOp::Not=>1});
tags!(cmp_tag, cmp_read, CmpOp, {CmpOp::Eq=>0,CmpOp::Ne=>1,CmpOp::Lt=>2,CmpOp::Le=>3,CmpOp::Gt=>4,CmpOp::Ge=>5});
tags!(overflow_tag, overflow_read, Overflow, {Overflow::Wrap=>0,Overflow::Trap=>1});
fn panic_tag(p: PanicKind) -> u8 {
    match p {
        PanicKind::AddOverflow => 0,
        PanicKind::SubOverflow => 1,
        PanicKind::MulOverflow => 2,
        PanicKind::NegOverflow => 3,
        PanicKind::ShlOverflow => 4,
        PanicKind::ShrOverflow => 5,
        PanicKind::DivByZero => 6,
        PanicKind::RemByZero => 7,
        PanicKind::DivOverflow => 8,
        PanicKind::RemOverflow => 9,
        PanicKind::Unreachable => 10,
        PanicKind::Message(_) => 11,
    }
}

/// Canonical key bytes and persistent payload use the identical encoding.
pub fn encode(function: &SsaFunction, meter: &Meter<'_>) -> Result<Owned<Vec<u8>>, String> {
    let mut count = Writer {
        meter,
        bytes: None,
        pos: 0,
    };
    count.function(function)?;
    let len = count.pos;
    let (mut bytes, lease) = meter.vec::<u8>(len)?;
    bytes.resize(len, 0);
    let result = Writer {
        meter,
        bytes: Some(&mut bytes),
        pos: 0,
    }
    .function(function);
    if let Err(e) = result {
        drop(bytes);
        drop(lease);
        return Err(e);
    }
    let actual = bytes.capacity();
    Owned::adopt(lease, bytes, actual, meter)
}

enum Failure {
    Malformed,
    Resource(String),
}
impl From<String> for Failure {
    fn from(e: String) -> Self {
        Self::Resource(e)
    }
}
type Parsed<T> = Result<T, Failure>;
struct Reader<'b, 'a, 'm> {
    bytes: &'b [u8],
    pos: usize,
    meter: &'a Meter<'m>,
    lease: Option<&'a mut Lease>,
}
impl Reader<'_, '_, '_> {
    fn take(&mut self, n: usize) -> Parsed<&[u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.bytes.len())
            .ok_or(Failure::Malformed)?;
        self.meter.work(n)?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn byte(&mut self) -> Parsed<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Parsed<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| Failure::Malformed)?,
        ))
    }
    fn usize(&mut self) -> Parsed<usize> {
        usize::try_from(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| Failure::Malformed)?,
        ))
        .map_err(|_| Failure::Malformed)
    }
    fn count(&mut self, min: usize) -> Parsed<usize> {
        let n = self.u32()? as usize;
        if n > (self.bytes.len() - self.pos) / min {
            return Err(Failure::Malformed);
        }
        Ok(n)
    }
    fn vec<T>(&mut self, n: usize) -> Parsed<Vec<T>> {
        let Some(lease) = self.lease.as_deref_mut() else {
            return Ok(Vec::new());
        };
        let bytes = n.checked_mul(size_of::<T>()).ok_or(Failure::Malformed)?;
        lease.admit(self.meter, bytes)?;
        let mut out = Vec::new();
        out.try_reserve_exact(n)
            .map_err(|_| Failure::Resource("SSA codec allocation failed".into()))?;
        let surplus = out
            .capacity()
            .saturating_sub(n)
            .checked_mul(size_of::<T>())
            .ok_or(Failure::Malformed)?;
        if let Err(e) = lease.admit(self.meter, surplus) {
            drop(out);
            return Err(e.into());
        }
        Ok(out)
    }
    fn list<T>(
        &mut self,
        min: usize,
        mut read: impl FnMut(&mut Self) -> Parsed<T>,
    ) -> Parsed<Vec<T>> {
        let n = self.count(min)?;
        let mut out = self.vec(n)?;
        for _ in 0..n {
            let v = read(self)?;
            if self.lease.is_some() {
                out.push(v);
            }
        }
        Ok(out)
    }
    fn text(&mut self) -> Parsed<String> {
        let n = self.count(1)?;
        self.meter.work(n)?;
        // Validate UTF-8 before admitting or allocating any string storage.
        let end = self.pos + n;
        std::str::from_utf8(&self.bytes[self.pos..end]).map_err(|_| Failure::Malformed)?;
        let mut out = self.vec::<u8>(n)?;
        let build = self.lease.is_some();
        let s = self.take(n)?;
        if build {
            out.extend_from_slice(s);
        }
        String::from_utf8(out).map_err(|_| Failure::Malformed)
    }
    fn ty(&mut self) -> Parsed<SsaTy> {
        match self.byte()? {
            0 => Ok(SsaTy::Unit),
            1 => Ok(SsaTy::Bool),
            2 => {
                let bits = self.byte()?;
                let signed = match self.byte()? {
                    0 => false,
                    1 => true,
                    _ => return Err(Failure::Malformed),
                };
                Ok(SsaTy::Int(
                    SsaIntTy::new(bits, signed).ok_or(Failure::Malformed)?,
                ))
            }
            _ => Err(Failure::Malformed),
        }
    }
    fn value(&mut self) -> Parsed<SsaValue> {
        Ok(SsaValue(self.u32()?))
    }
    fn location(&mut self) -> Parsed<Option<Arc<RuntimeLocation>>> {
        match self.byte()? {
            0 => Ok(None),
            1 => {
                let path = self.text()?;
                let line = self.usize()?;
                let column = self.usize()?;
                let Some(lease) = self.lease.as_deref_mut() else {
                    return Ok(None);
                };
                // Stable std has no fallible Arc constructor. Admit both allocations,
                // including their two reference-count words, before constructing them.
                let path_bytes = path
                    .len()
                    .checked_add(2 * size_of::<usize>())
                    .and_then(|n| n.checked_add(std::mem::align_of::<usize>() - 1))
                    .map(|n| n & !(std::mem::align_of::<usize>() - 1))
                    .ok_or(Failure::Malformed)?;
                let location_bytes = size_of::<RuntimeLocation>() + 2 * size_of::<usize>();
                lease.admit(self.meter, super::meter::add(path_bytes, location_bytes)?)?;
                Ok(Some(Arc::new(RuntimeLocation {
                    path: Arc::from(path.as_str()),
                    line,
                    column,
                })))
            }
            _ => Err(Failure::Malformed),
        }
    }
    fn values(&mut self) -> Parsed<Vec<SsaValue>> {
        self.list(4, Self::value)
    }
    fn target(&mut self) -> Parsed<SsaTarget> {
        Ok(SsaTarget {
            block: SsaBlock(self.u32()?),
            args: self.values()?,
        })
    }
    fn inst(&mut self) -> Parsed<Inst> {
        let runtime_location = self.location()?;
        let result = self.value()?;
        let kind = match self.byte()? {
            0 => {
                let ty = self.ty()?;
                let bits =
                    u128::from_le_bytes(self.take(16)?.try_into().map_err(|_| Failure::Malformed)?);
                InstKind::Const { ty, bits }
            }
            1 => InstKind::Binary {
                op: bin_read(self.byte()?).ok_or(Failure::Malformed)?,
                overflow: overflow_read(self.byte()?).ok_or(Failure::Malformed)?,
                lhs: self.value()?,
                rhs: self.value()?,
            },
            2 => InstKind::Unary {
                op: un_read(self.byte()?).ok_or(Failure::Malformed)?,
                overflow: overflow_read(self.byte()?).ok_or(Failure::Malformed)?,
                value: self.value()?,
            },
            3 => InstKind::Cmp {
                op: cmp_read(self.byte()?).ok_or(Failure::Malformed)?,
                lhs: self.value()?,
                rhs: self.value()?,
            },
            4 => {
                let SsaTy::Int(to) = self.ty()? else {
                    return Err(Failure::Malformed);
                };
                InstKind::Cast {
                    to,
                    value: self.value()?,
                }
            }
            5 => InstKind::Select {
                cond: self.value()?,
                then: self.value()?,
                otherwise: self.value()?,
            },
            6 => InstKind::Call {
                callee: FuncRef(self.u32()?),
                args: self.values()?,
            },
            _ => return Err(Failure::Malformed),
        };
        Ok(Inst {
            runtime_location,
            result,
            kind,
        })
    }
    fn panic(&mut self) -> Parsed<PanicKind> {
        Ok(match self.byte()? {
            0 => PanicKind::AddOverflow,
            1 => PanicKind::SubOverflow,
            2 => PanicKind::MulOverflow,
            3 => PanicKind::NegOverflow,
            4 => PanicKind::ShlOverflow,
            5 => PanicKind::ShrOverflow,
            6 => PanicKind::DivByZero,
            7 => PanicKind::RemByZero,
            8 => PanicKind::DivOverflow,
            9 => PanicKind::RemOverflow,
            10 => PanicKind::Unreachable,
            11 => PanicKind::Message(self.u32()?),
            _ => return Err(Failure::Malformed),
        })
    }
    fn term(&mut self) -> Parsed<SsaTerminator> {
        Ok(match self.byte()? {
            0 => SsaTerminator::Jump(self.target()?),
            1 => SsaTerminator::Branch {
                cond: self.value()?,
                then: self.target()?,
                otherwise: self.target()?,
            },
            2 => SsaTerminator::Return(self.value()?),
            3 => SsaTerminator::Panic(self.panic()?),
            _ => return Err(Failure::Malformed),
        })
    }
    fn block(&mut self) -> Parsed<BlockData> {
        Ok(BlockData {
            panic_location: self.location()?,
            params: self.list(5, |r| Ok((r.value()?, r.ty()?)))?,
            insts: self.list(6, Self::inst)?,
            term: self.term()?,
        })
    }
    fn function(&mut self) -> Parsed<SsaFunction> {
        if self.take(MAGIC.len())? != MAGIC {
            return Err(Failure::Malformed);
        }
        let f = SsaFunction {
            params: self.list(1, Self::ty)?,
            ret: self.ty()?,
            blocks: self.list(11, Self::block)?,
            panics: self.list(8, |r| {
                Ok(SsaPanicText {
                    message: r.text()?,
                    location: r.text()?,
                })
            })?,
        };
        if self.pos != self.bytes.len() {
            return Err(Failure::Malformed);
        }
        Ok(f)
    }
}

/// Malformed, truncated, unknown-version and oversized payloads are cache misses.
/// Only genuine meter/allocation failure is an error. No callee trust is restored.
pub fn decode(bytes: &[u8], meter: &Meter<'_>) -> Result<Option<Owned<SsaFunction>>, String> {
    if bytes.len() > LIMIT {
        return Ok(None);
    }
    let pre = Reader {
        bytes,
        pos: 0,
        meter,
        lease: None,
    }
    .function();
    match pre {
        Err(Failure::Malformed) => return Ok(None),
        Err(Failure::Resource(e)) => return Err(e),
        Ok(_) => {}
    }
    let mut lease = meter.lease(0)?;
    let parsed = Reader {
        bytes,
        pos: 0,
        meter,
        lease: Some(&mut lease),
    }
    .function();
    match parsed {
        Ok(value) => {
            let actual = lease.bytes();
            Owned::adopt(lease, value, actual, meter).map(Some)
        }
        Err(Failure::Malformed) => Ok(None),
        Err(Failure::Resource(e)) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> SsaFunction {
        let location = Some(Arc::new(RuntimeLocation {
            path: Arc::from("src/雪.rs"),
            line: 123,
            column: 42,
        }));
        let mut types = vec![SsaTy::Unit, SsaTy::Bool];
        for bits in [8, 16, 32, 64, 128] {
            for signed in [false, true] {
                types.push(SsaTy::Int(SsaIntTy::new(bits, signed).unwrap()));
            }
        }
        let mut kinds = Vec::new();
        for &ty in &types {
            kinds.push(InstKind::Const {
                ty,
                bits: ty.max_bits(),
            });
        }
        for op in [
            BinOp::Add,
            BinOp::Sub,
            BinOp::Mul,
            BinOp::Div,
            BinOp::Rem,
            BinOp::And,
            BinOp::Or,
            BinOp::Xor,
            BinOp::Shl,
            BinOp::Shr,
        ] {
            for overflow in [Overflow::Wrap, Overflow::Trap] {
                kinds.push(InstKind::Binary {
                    op,
                    overflow,
                    lhs: SsaValue(0),
                    rhs: SsaValue(u32::MAX),
                });
            }
        }
        for op in [UnOp::Neg, UnOp::Not] {
            for overflow in [Overflow::Wrap, Overflow::Trap] {
                kinds.push(InstKind::Unary {
                    op,
                    overflow,
                    value: SsaValue(1),
                });
            }
        }
        for op in [
            CmpOp::Eq,
            CmpOp::Ne,
            CmpOp::Lt,
            CmpOp::Le,
            CmpOp::Gt,
            CmpOp::Ge,
        ] {
            kinds.push(InstKind::Cmp {
                op,
                lhs: SsaValue(1),
                rhs: SsaValue(2),
            });
        }
        for &ty in &types {
            if let SsaTy::Int(to) = ty {
                kinds.push(InstKind::Cast {
                    to,
                    value: SsaValue(2),
                });
            }
        }
        kinds.push(InstKind::Select {
            cond: SsaValue(1),
            then: SsaValue(2),
            otherwise: SsaValue(3),
        });
        kinds.push(InstKind::Call {
            callee: FuncRef(u32::MAX),
            args: vec![SsaValue(1), SsaValue(7)],
        });
        kinds.push(InstKind::Call {
            callee: FuncRef(0),
            args: vec![],
        });
        let target = SsaTarget {
            block: SsaBlock(17),
            args: vec![SsaValue(1), SsaValue(9)],
        };
        let mut terms = vec![
            SsaTerminator::Jump(target.clone()),
            SsaTerminator::Branch {
                cond: SsaValue(0),
                then: target.clone(),
                otherwise: SsaTarget {
                    block: SsaBlock(0),
                    args: vec![],
                },
            },
            SsaTerminator::Return(SsaValue(9)),
        ];
        for p in [
            PanicKind::AddOverflow,
            PanicKind::SubOverflow,
            PanicKind::MulOverflow,
            PanicKind::NegOverflow,
            PanicKind::ShlOverflow,
            PanicKind::ShrOverflow,
            PanicKind::DivByZero,
            PanicKind::RemByZero,
            PanicKind::DivOverflow,
            PanicKind::RemOverflow,
            PanicKind::Unreachable,
            PanicKind::Message(u32::MAX),
        ] {
            terms.push(SsaTerminator::Panic(p));
        }
        let mut blocks: Vec<_> = terms
            .into_iter()
            .enumerate()
            .map(|(i, term)| BlockData {
                panic_location: if i % 2 == 0 { location.clone() } else { None },
                params: vec![(SsaValue(0), SsaTy::Bool)],
                insts: vec![],
                term,
            })
            .collect();
        blocks[0].insts = kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| Inst {
                runtime_location: if i % 2 == 0 { location.clone() } else { None },
                result: SsaValue(i as u32),
                kind,
            })
            .collect();
        SsaFunction {
            params: types,
            ret: SsaTy::Unit,
            blocks,
            panics: vec![
                SsaPanicText {
                    message: "panic\0雪\n".into(),
                    location: "file:1:2".into(),
                },
                SsaPanicText {
                    message: String::new(),
                    location: String::new(),
                },
            ],
        }
    }

    #[test]
    fn every_variant_roundtrips_exactly_and_deterministically() {
        let budget = super::super::meter::unlimited();
        let meter = Meter::new(&budget);
        let function = fixture();
        let encoded = encode(&function, &meter).unwrap();
        assert_eq!(encoded.value, encode(&function, &meter).unwrap().value);
        let decoded = decode(&encoded.value, &meter).unwrap().unwrap();
        assert_eq!(decoded.value, function);
        assert_eq!(encoded.value, encode(&decoded.value, &meter).unwrap().value);
    }

    #[test]
    fn truncation_version_types_counts_and_trailing_bytes_are_misses() {
        let budget = super::super::meter::unlimited();
        let meter = Meter::new(&budget);
        let encoded = encode(&fixture(), &meter).unwrap();
        for n in 0..encoded.value.len() {
            assert!(
                decode(&encoded.value[..n], &meter).unwrap().is_none(),
                "prefix {n}"
            );
        }
        let mut bytes = encoded.value.clone();
        bytes[MAGIC.len() - 1] = 99;
        assert!(decode(&bytes, &meter).unwrap().is_none());
        let mut bytes = encoded.value.clone();
        bytes.push(0);
        assert!(decode(&bytes, &meter).unwrap().is_none());
        let mut bytes = encoded.value.clone();
        bytes[MAGIC.len()..MAGIC.len() + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&bytes, &meter).unwrap().is_none());
        let mut bytes = encoded.value.clone();
        bytes[MAGIC.len() + 4] = 255;
        assert!(decode(&bytes, &meter).unwrap().is_none());
        let mut bytes = encoded.value.clone();
        bytes[MAGIC.len() + 7] = 7;
        assert!(decode(&bytes, &meter).unwrap().is_none());
        let mut bytes = encoded.value.clone();
        bytes[MAGIC.len() + 8] = 2;
        assert!(decode(&bytes, &meter).unwrap().is_none());
    }

    #[test]
    fn empty_function_and_bounded_random_input() {
        let budget = super::super::meter::unlimited();
        let meter = Meter::new(&budget);
        let f = SsaFunction {
            params: vec![],
            ret: SsaTy::Bool,
            blocks: vec![],
            panics: vec![],
        };
        let encoded = encode(&f, &meter).unwrap();
        assert_eq!(decode(&encoded.value, &meter).unwrap().unwrap().value, f);
        let oversized = vec![0; LIMIT + 1];
        assert!(decode(&oversized, &meter).unwrap().is_none());
        for len in 0..128 {
            let bytes = vec![255; len];
            assert!(decode(&bytes, &meter).unwrap().is_none());
        }
    }

    #[test]
    fn locations_and_panic_payloads_are_key_material() {
        let budget = super::super::meter::unlimited();
        let meter = Meter::new(&budget);
        let f = fixture();
        let original = encode(&f, &meter).unwrap();
        for which in 0..6 {
            let mut changed = f.clone();
            match which {
                0 => changed.blocks[0].panic_location = None,
                1 => changed.blocks[0].insts[0].runtime_location = None,
                2 => Arc::make_mut(changed.blocks[0].panic_location.as_mut().unwrap()).line += 1,
                3 => {
                    Arc::make_mut(
                        changed.blocks[0].insts[0]
                            .runtime_location
                            .as_mut()
                            .unwrap(),
                    )
                    .column += 1
                }
                4 => changed.panics[0].message.push('!'),
                5 => changed.panics[0].location.push('!'),
                _ => unreachable!(),
            }
            assert_ne!(original.value, encode(&changed, &meter).unwrap().value);
        }
        let mut corrupted = original.value.clone();
        let offset = corrupted
            .windows("panic\0雪\n".len())
            .position(|w| w == "panic\0雪\n".as_bytes())
            .unwrap();
        corrupted[offset] = 255;
        assert!(decode(&corrupted, &meter).unwrap().is_none());
        let mut corrupted = original.value.clone();
        corrupted[offset - 4..offset].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&corrupted, &meter).unwrap().is_none());
    }

    #[test]
    fn resource_failure_is_not_a_cache_miss_and_storage_refunds() {
        use crate::compiler::ir::{PlanBudget, PlanLimits};
        let budget = super::super::meter::unlimited();
        let meter = Meter::new(&budget);
        let bytes = encode(&fixture(), &meter).unwrap();
        let limited = PlanBudget::new(PlanLimits {
            bytes: 1,
            ..PlanLimits::default()
        });
        assert!(decode(&bytes.value, &Meter::new(&limited)).is_err());
        let limited = PlanBudget::new(PlanLimits {
            work: 1,
            ..PlanLimits::default()
        });
        assert!(decode(&bytes.value, &Meter::new(&limited)).is_err());
        let before = budget.usage_snapshot().bytes;
        let decoded = decode(&bytes.value, &meter).unwrap().unwrap();
        assert!(budget.usage_snapshot().bytes > before);
        drop(decoded);
        assert_eq!(budget.usage_snapshot().bytes, before);
    }
}
