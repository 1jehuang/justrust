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
use super::{
    BinOp, Block, BlockData, CmpOp, Function, Inst, InstKind, Overflow, PanicKind, Terminator, Ty,
    UnOp, Value,
};

pub const ENCODING_VERSION: u32 = 1;

pub fn canonical(function: &Function) -> Function {
    let order = function.reverse_postorder();
    let mut block_map = vec![u32::MAX; function.blocks.len()];
    for (index, block) in order.iter().enumerate() {
        block_map[block.0 as usize] = index as u32;
    }
    let mut value_map = vec![u32::MAX; function.value_bound()];
    let mut next = 0u32;
    for block in &order {
        let data = &function.blocks[block.0 as usize];
        for (value, _) in &data.params {
            value_map[value.0 as usize] = next;
            next += 1;
        }
        for inst in &data.insts {
            value_map[inst.result.0 as usize] = next;
            next += 1;
        }
    }
    let value = |value: Value| Value(value_map.get(value.0 as usize).copied().unwrap_or(u32::MAX));
    let blocks = order
        .iter()
        .map(|block| {
            let data = &function.blocks[block.0 as usize];
            let mut term = data.term.clone();
            for target in term.targets_mut() {
                target.block = Block(block_map[target.block.0 as usize]);
            }
            for operand in term.operands_mut() {
                *operand = value(*operand);
            }
            BlockData {
                params: data
                    .params
                    .iter()
                    .map(|(param, ty)| (value(*param), *ty))
                    .collect(),
                insts: data
                    .insts
                    .iter()
                    .map(|inst| {
                        let mut kind = inst.kind.clone();
                        for operand in kind.operands_mut() {
                            *operand = value(*operand);
                        }
                        Inst {
                            result: value(inst.result),
                            kind,
                        }
                    })
                    .collect(),
                term,
            }
        })
        .collect();
    Function {
        params: function.params.clone(),
        ret: function.ret,
        blocks,
    }
}

/// 128-bit FNV-1a. Not cryptographic: cache keys that cross trust boundaries
/// need a cryptographic digest (ARCHITECTURE.md section 14).
struct Fnv(u128);

impl Fnv {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;

    fn bytes(&mut self, bytes: &[u8]) {
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

fn ty(h: &mut Fnv, ty: Ty) {
    match ty {
        Ty::Unit => h.u8(0),
        Ty::Bool => h.u8(1),
        Ty::Int(int) => {
            h.u8(2);
            h.u8(int.bits());
            h.u8(u8::from(int.signed()));
        }
    }
}

fn values(h: &mut Fnv, values: &[Value]) {
    h.len(values.len());
    for value in values {
        h.u32(value.0);
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

fn panic(kind: PanicKind) -> u8 {
    match kind {
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
    }
}

fn encode(h: &mut Fnv, function: &Function) {
    h.bytes(b"jr-ssa");
    h.u32(ENCODING_VERSION);
    h.len(function.params.len());
    for param in &function.params {
        ty(h, *param);
    }
    ty(h, function.ret);
    h.len(function.blocks.len());
    for data in &function.blocks {
        h.len(data.params.len());
        for (value, param) in &data.params {
            h.u32(value.0);
            ty(h, *param);
        }
        h.len(data.insts.len());
        for inst in &data.insts {
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
                    ty(h, Ty::Int(*to));
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
        match &data.term {
            Terminator::Jump(target) => {
                h.u8(0);
                h.u32(target.block.0);
                values(h, &target.args);
            }
            Terminator::Branch {
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
            Terminator::Return(value) => {
                h.u8(2);
                h.u32(value.0);
            }
            Terminator::Panic(kind) => {
                h.u8(3);
                h.u8(panic(*kind));
            }
        }
    }
}

/// Stable content hash of the canonical form. Callee references are hashed
/// as given; inside the graph they become canonical item identities.
pub fn content_hash(function: &Function) -> u128 {
    let mut h = Fnv(Fnv::OFFSET);
    encode(&mut h, &canonical(function));
    h.0
}
