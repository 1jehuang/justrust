//! Canonical form and content hashing.
//!
//! `canonical` renumbers blocks in reverse postorder and values in definition
//! order along it, and drops unreachable blocks (they never execute). Two
//! functions that differ only in ID assignment or dead blocks therefore have
//! the same canonical form and the same `content_hash`.
//!
//! The hash is computed over an explicit byte encoding with a fixed algorithm,
//! not `std::hash`, so it is stable across processes, machines and toolchain
//! versions and can serve as a cache key component. Bump `ENCODING_VERSION`
//! whenever the encoding or IR semantics change.
use super::meter::{self, Meter};
use super::store::{self, Owned};
use super::{
    BinOp, CmpOp, InstKind, Overflow, PanicKind, SsaBlock, SsaFunction, SsaTerminator, SsaTy,
    SsaValue, UnOp,
};

pub const ENCODING_VERSION: u32 = 3;

/// Unmetered canonical form, for the interpreter and tests.
pub fn canonical(function: &SsaFunction) -> SsaFunction {
    let budget = super::meter::unlimited();
    canonical_metered(function, &Meter::new(&budget))
        .expect("unlimited canonicalization")
        .value
}

/// Canonical form with paid storage: the block/value maps are leased
/// scratch, the output is admitted from the input's shape before building and
/// reconciled to its real capacities, and every block, parameter,
/// instruction and operand rewrite is charged.
pub fn canonical_metered(
    function: &SsaFunction,
    meter: &Meter<'_>,
) -> Result<Owned<SsaFunction>, String> {
    let order = function.reverse_postorder_metered(meter)?;
    let mut block_map = meter.table(function.blocks.len(), u32::MAX)?;
    for (index, block) in order.iter().enumerate() {
        block_map[block.0 as usize] = index as u32;
    }
    let mut value_map = meter.table(function.value_bound(), u32::MAX)?;
    let mut next = 0u32;
    for block in order.iter() {
        let data = &function.blocks[block.0 as usize];
        meter.work(meter::add(data.params.len(), data.insts.len())?)?;
        for value in data
            .params
            .iter()
            .map(|(value, _)| *value)
            .chain(data.insts.iter().map(|inst| inst.result))
        {
            value_map[value.0 as usize] = next;
            next = next.checked_add(1).ok_or("SSA value IDs exhausted")?;
        }
    }
    let value =
        |value: SsaValue| SsaValue(value_map.get(value.0 as usize).copied().unwrap_or(u32::MAX));
    // Output shape: reachable blocks only, so the input shape is an upper bound.
    let preflight = store::shape_bytes(function)?;
    Owned::build(
        meter,
        preflight,
        || -> Result<SsaFunction, String> {
            let mut blocks = Vec::new();
            blocks
                .try_reserve_exact(order.len())
                .map_err(|_| "optimizer allocation failed".to_string())?;
            for block in order.iter() {
                let data = &function.blocks[block.0 as usize];
                let mut out = store::try_clone_block(data)?;
                meter.work(meter::add(
                    meter::add(out.params.len(), out.insts.len())?,
                    out.term.operand_count(),
                )?)?;
                for (param, _) in out.params.iter_mut() {
                    *param = value(*param);
                }
                for inst in out.insts.iter_mut() {
                    meter.work(inst.kind.operand_count())?;
                    inst.result = value(inst.result);
                    for operand in inst.kind.operands_mut() {
                        *operand = value(*operand);
                    }
                }
                for target in out.term.targets_mut() {
                    target.block = SsaBlock(block_map[target.block.0 as usize]);
                }
                for operand in out.term.operands_mut() {
                    *operand = value(*operand);
                }
                blocks.push(out);
            }
            let mut params = Vec::new();
            params
                .try_reserve_exact(function.params.len())
                .map_err(|_| "optimizer allocation failed".to_string())?;
            params.extend_from_slice(&function.params);
            Ok(SsaFunction {
                params,
                ret: function.ret,
                blocks,
                panics: store::copy_panics(&function.panics, meter)?,
            })
        },
        store::capacity_bytes,
    )
}

/// 128-bit FNV-1a. Not cryptographic: cache keys that cross trust boundaries
/// need a cryptographic digest (ARCHITECTURE.md section 14).
struct Fnv(u128, usize);

impl Fnv {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;

    fn bytes(&mut self, bytes: &[u8]) {
        self.1 += bytes.len();
        for byte in bytes {
            self.0 ^= u128::from(*byte);
            self.0 = self.0.wrapping_mul(Self::PRIME);
        }
    }

    fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    fn u128(&mut self, value: u128) {
        self.bytes(&value.to_le_bytes());
    }

    fn len(&mut self, len: usize) {
        self.bytes(&(len as u64).to_le_bytes());
    }
}

fn ty(h: &mut Fnv, ty: SsaTy) {
    match ty {
        SsaTy::Unit => h.u8(0),
        SsaTy::Bool => h.u8(1),
        SsaTy::Int(int) => {
            h.u8(2);
            h.u8(int.bits());
            h.u8(u8::from(int.signed()));
        }
    }
}

fn values(h: &mut Fnv, values: &[SsaValue]) {
    h.len(values.len());
    for value in values {
        h.u32(value.0);
    }
}

fn location(h: &mut Fnv, site: &Option<std::sync::Arc<crate::compiler::ir::RuntimeLocation>>) {
    h.u8(u8::from(site.is_some()));
    if let Some(site) = site {
        h.len(site.path.len());
        h.bytes(site.path.as_bytes());
        h.len(site.line);
        h.len(site.column);
    }
}

fn binop(op: BinOp) -> u8 {
    match op {
        BinOp::Add => 0,
        BinOp::Sub => 1,
        BinOp::Mul => 2,
        BinOp::Div => 3,
        BinOp::Rem => 4,
        BinOp::And => 5,
        BinOp::Or => 6,
        BinOp::Xor => 7,
        BinOp::Shl => 8,
        BinOp::Shr => 9,
    }
}

fn cmpop(op: CmpOp) -> u8 {
    match op {
        CmpOp::Eq => 0,
        CmpOp::Ne => 1,
        CmpOp::Lt => 2,
        CmpOp::Le => 3,
        CmpOp::Gt => 4,
        CmpOp::Ge => 5,
    }
}

fn overflow(mode: Overflow) -> u8 {
    match mode {
        Overflow::Wrap => 0,
        Overflow::Trap => 1,
    }
}

fn panic(h: &mut Fnv, kind: PanicKind) {
    h.u8(match kind {
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
        PanicKind::Message(index) => {
            h.u8(11);
            h.u32(index);
            return;
        }
    });
}

fn encode(
    h: &mut Fnv,
    function: &SsaFunction,
    charge: &mut dyn FnMut(usize) -> Result<(), String>,
) -> Result<(), String> {
    h.bytes(b"jr-ssa");
    h.u32(ENCODING_VERSION);
    h.len(function.params.len());
    for param in &function.params {
        ty(h, *param);
    }
    ty(h, function.ret);
    h.len(function.blocks.len());
    h.len(function.panics.len());
    for text in &function.panics {
        h.len(text.message.len());
        h.bytes(text.message.as_bytes());
        h.len(text.location.len());
        h.bytes(text.location.as_bytes());
    }
    charge(std::mem::take(&mut h.1))?;
    for data in &function.blocks {
        charge(std::mem::take(&mut h.1))?;
        h.len(data.params.len());
        for (value, param) in &data.params {
            h.u32(value.0);
            ty(h, *param);
        }
        h.len(data.insts.len());
        for inst in &data.insts {
            location(h, &inst.runtime_location);
            h.u32(inst.result.0);
            match &inst.kind {
                InstKind::Const { ty: t, bits } => {
                    h.u8(0);
                    ty(h, *t);
                    h.u128(*bits);
                }
                InstKind::Binary {
                    op,
                    overflow: mode,
                    lhs,
                    rhs,
                } => {
                    h.u8(1);
                    h.u8(binop(*op));
                    h.u8(overflow(*mode));
                    values(h, &[*lhs, *rhs]);
                }
                InstKind::Unary {
                    op,
                    overflow: mode,
                    value,
                } => {
                    h.u8(2);
                    h.u8(match op {
                        UnOp::Neg => 0,
                        UnOp::Not => 1,
                    });
                    h.u8(overflow(*mode));
                    h.u32(value.0);
                }
                InstKind::Cmp { op, lhs, rhs } => {
                    h.u8(3);
                    h.u8(cmpop(*op));
                    values(h, &[*lhs, *rhs]);
                }
                InstKind::Cast { to, value } => {
                    h.u8(4);
                    ty(h, SsaTy::Int(*to));
                    h.u32(value.0);
                }
                InstKind::Select {
                    cond,
                    then,
                    otherwise,
                } => {
                    h.u8(5);
                    values(h, &[*cond, *then, *otherwise]);
                }
                InstKind::Call { callee, args } => {
                    h.u8(6);
                    h.u32(callee.0);
                    values(h, args);
                }
            }
        }
        location(h, &data.panic_location);
        match &data.term {
            SsaTerminator::Jump(target) => {
                h.u8(0);
                h.u32(target.block.0);
                values(h, &target.args);
            }
            SsaTerminator::Branch {
                cond,
                then,
                otherwise,
            } => {
                h.u8(1);
                h.u32(cond.0);
                h.u32(then.block.0);
                values(h, &then.args);
                h.u32(otherwise.block.0);
                values(h, &otherwise.args);
            }
            SsaTerminator::Return(value) => {
                h.u8(2);
                h.u32(value.0);
            }
            SsaTerminator::Panic(kind) => {
                h.u8(3);
                panic(h, *kind);
            }
        }
    }
    charge(std::mem::take(&mut h.1))
}

/// Stable content hash of the canonical form. SsaCallee references are hashed
/// as given; inside the graph they become canonical item identities.
pub fn content_hash(function: &SsaFunction) -> u128 {
    let budget = super::meter::unlimited();
    content_hash_metered(function, &Meter::new(&budget)).expect("unlimited hash")
}

/// Metered content hash: canonicalization is paid (and its storage refunded
/// when this returns), and work is charged per encoded byte, flushed once per
/// block so the charge tracks the bytes actually hashed.
pub fn content_hash_metered(function: &SsaFunction, meter: &Meter<'_>) -> Result<u128, String> {
    let canonical = canonical_metered(function, meter)?;
    let mut h = Fnv(Fnv::OFFSET, 0);
    encode(&mut h, &canonical.value, &mut |bytes| meter.work(bytes))?;
    Ok(h.0)
}
