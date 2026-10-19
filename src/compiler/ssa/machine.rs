//! Register-allocating x86-64 code generation from optimized SSA.
//!
//! Produces a `backend::Object` that follows the stack emitter's private ABI
//! exactly (see the `backend` module header), so optimized and stack-emitted
//! functions call each other freely and link with the same linker:
//! arguments pushed left to right as eight-byte words, read by the callee at
//! `rbp + 16 + 8 * (later parameters)`, results normalized in `rax`, every
//! register caller-saved, overflow and division traps exit with status 101.
//!
//! Allocation is linear scan over live intervals on the canonical block order
//! (reverse postorder). Values live across a call are kept in stack slots,
//! because callees clobber every register. Constants are rematerialized and
//! never occupy a register. SsaBlock parameters are filled by parallel moves on
//! each edge. A comparison used only by the following branch is fused into
//! a compare-and-jump.
use super::lower::Lowered;
use super::verify::Types;
use super::{
    BinOp, CmpOp, InstKind, Overflow, PanicKind, SsaIntTy, SsaPanicText, SsaTerminator, SsaTy,
    SsaValue, UnOp,
};
use crate::compiler::backend::Object;

const RAX: u8 = 0;
const RCX: u8 = 1;
const RDX: u8 = 2;
/// Allocatable registers. rax, rcx and rdx are scratch; rsp and rbp are frame.
const ALLOCATABLE: [u8; 11] = [3, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const MAX_ARGS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Loc {
    Reg(u8),
    /// rbp-relative slot.
    Stack(i32),
    /// Normalized 64-bit image of a constant.
    Const(u64),
    /// Never materialized (a fused comparison).
    None,
}

/// What machine code generation did. Feeds optimizer telemetry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub registers: usize,
    pub spilled: usize,
    pub fused_branches: usize,
    pub code_bytes: usize,
}

/// Normalized 64-bit image (sign- or zero-extended) of a value of `ty`.
fn image(ty: SsaTy, bits: u128) -> u64 {
    match ty {
        SsaTy::Int(int) if int.signed() && int.bits() < 64 => {
            let shift = 64 - u32::from(int.bits());
            (((bits as u64) << shift) as i64 >> shift) as u64
        }
        _ => bits as u64,
    }
}

fn int_of(ty: SsaTy) -> SsaIntTy {
    match ty {
        SsaTy::Int(int) => int,
        _ => SsaIntTy::U8,
    }
}

struct Asm {
    code: Vec<u8>,
    traps: Vec<usize>,
    calls: Vec<(usize, String)>,
    strings: Vec<(usize, Vec<u8>)>,
}

impl Asm {
    fn bytes(&mut self, bytes: &[u8]) {
        self.code.extend_from_slice(bytes);
    }

    fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    fn rel32(&mut self, opcode: &[u8]) -> usize {
        self.bytes(opcode);
        let at = self.code.len();
        self.i32(0);
        at
    }

    fn patch(&mut self, at: usize, target: usize) -> Result<(), String> {
        let displacement = i32::try_from(target as i128 - at as i128 - 4)
            .map_err(|_| "function exceeds rel32 range".to_string())?;
        self.code[at..at + 4].copy_from_slice(&displacement.to_le_bytes());
        Ok(())
    }

    fn trap_if(&mut self, condition: u8) {
        let at = self.rel32(&[0x0f, condition]);
        self.traps.push(at);
    }

    fn rex(w: bool, reg: u8, base: u8) -> u8 {
        0x40 | (u8::from(w) << 3) | ((reg >> 3) << 2) | (base >> 3)
    }

    /// mov dst, src (64-bit registers).
    fn mov_rr(&mut self, dst: u8, src: u8) {
        if dst != src {
            self.bytes(&[
                Self::rex(true, src, dst),
                0x89,
                0xc0 | ((src & 7) << 3) | (dst & 7),
            ]);
        }
    }

    fn mov_ri(&mut self, dst: u8, value: u64) {
        if value == 0 {
            // xor r32, r32 zero-extends.
            if dst >= 8 {
                self.bytes(&[0x45]);
            }
            self.bytes(&[0x31, 0xc0 | ((dst & 7) << 3) | (dst & 7)]);
        } else if value <= u64::from(u32::MAX) {
            if dst >= 8 {
                self.bytes(&[0x41]);
            }
            self.bytes(&[0xb8 + (dst & 7)]);
            self.bytes(&(value as u32).to_le_bytes());
        } else if (value as i64) >= i64::from(i32::MIN) && (value as i64) < 0 {
            // mov r/m64, imm32 sign-extends.
            self.bytes(&[Self::rex(true, 0, dst), 0xc7, 0xc0 | (dst & 7)]);
            self.bytes(&(value as u32).to_le_bytes());
        } else {
            self.bytes(&[Self::rex(true, 0, dst), 0xb8 + (dst & 7)]);
            self.bytes(&value.to_le_bytes());
        }
    }

    fn load_slot(&mut self, dst: u8, offset: i32) {
        self.bytes(&[Self::rex(true, dst, 5), 0x8b, 0x85 | ((dst & 7) << 3)]);
        self.i32(offset);
    }

    fn store_slot(&mut self, offset: i32, src: u8) {
        self.bytes(&[Self::rex(true, src, 5), 0x89, 0x85 | ((src & 7) << 3)]);
        self.i32(offset);
    }

    fn load(&mut self, dst: u8, loc: Loc) -> Result<(), String> {
        match loc {
            Loc::Reg(reg) => self.mov_rr(dst, reg),
            Loc::Stack(offset) => self.load_slot(dst, offset),
            Loc::Const(value) => self.mov_ri(dst, value),
            Loc::None => return Err("read of an unmaterialized SSA value".into()),
        }
        Ok(())
    }

    fn store(&mut self, loc: Loc, src: u8) -> Result<(), String> {
        match loc {
            Loc::Reg(reg) => self.mov_rr(reg, src),
            Loc::Stack(offset) => self.store_slot(offset, src),
            Loc::Const(_) | Loc::None => return Err("store to a non-storage location".into()),
        }
        Ok(())
    }

    fn push(&mut self, loc: Loc) -> Result<(), String> {
        match loc {
            Loc::Reg(reg) => {
                if reg >= 8 {
                    self.bytes(&[0x41]);
                }
                self.bytes(&[0x50 + (reg & 7)]);
            }
            Loc::Stack(offset) => {
                self.bytes(&[0xff, 0xb5]);
                self.i32(offset);
            }
            Loc::Const(value) => {
                self.mov_ri(RAX, value);
                self.bytes(&[0x50]);
            }
            Loc::None => return Err("push of an unmaterialized SSA value".into()),
        }
        Ok(())
    }

    /// Re-normalize rax to the declared width.
    fn normalize(&mut self, ty: SsaTy) {
        if let SsaTy::Int(int) = ty
            && int.bits() < 64
        {
            let shift = 64 - int.bits();
            self.bytes(&[0x48, 0xc1, 0xe0, shift]);
            self.bytes(&[0x48, 0xc1, if int.signed() { 0xf8 } else { 0xe8 }, shift]);
        }
    }

    /// Trap unless rax already holds an in-range value of `ty`.
    fn check_range(&mut self, ty: SsaTy) {
        if let SsaTy::Int(int) = ty
            && int.bits() < 64
        {
            self.bytes(&[0x48, 0x89, 0xc2]); // mov rdx, rax
            self.normalize(ty);
            self.bytes(&[0x48, 0x39, 0xd0]); // cmp rax, rdx
            self.trap_if(0x85);
        }
    }
}

fn condition(op: CmpOp, signed: bool) -> u8 {
    // setcc/jcc low nibble.
    match (op, signed) {
        (CmpOp::Eq, _) => 0x4,
        (CmpOp::Ne, _) => 0x5,
        (CmpOp::Lt, true) => 0xc,
        (CmpOp::Le, true) => 0xe,
        (CmpOp::Gt, true) => 0xf,
        (CmpOp::Ge, true) => 0xd,
        (CmpOp::Lt, false) => 0x2,
        (CmpOp::Le, false) => 0x6,
        (CmpOp::Gt, false) => 0x7,
        (CmpOp::Ge, false) => 0x3,
    }
}

struct Layout {
    loc: Vec<Loc>,
    fused: Vec<bool>,
    frame: i32,
    stats: Stats,
}

fn bitset(words: usize) -> Vec<u64> {
    vec![0; words]
}

fn set(bits: &mut [u64], index: usize) -> bool {
    let word = &mut bits[index / 64];
    let mask = 1u64 << (index % 64);
    let fresh = *word & mask == 0;
    *word |= mask;
    fresh
}

fn get(bits: &[u64], index: usize) -> bool {
    bits[index / 64] & (1u64 << (index % 64)) != 0
}

fn allocate(lowered: &Lowered, types: &Types) -> Result<Layout, String> {
    let function = &lowered.function;
    let bound = function.value_bound();
    let words = bound.div_ceil(64).max(1);
    let count = function.blocks.len();

    // Use counts, for fusion.
    let mut uses = vec![0usize; bound];
    for data in &function.blocks {
        for inst in &data.insts {
            for operand in inst.kind.operands() {
                uses[operand.0 as usize] += 1;
            }
        }
        for operand in data.term.operands() {
            uses[operand.0 as usize] += 1;
        }
    }
    let mut fused = vec![false; bound];
    for data in &function.blocks {
        if let SsaTerminator::Branch { cond, .. } = &data.term
            && uses[cond.0 as usize] == 1
            && let Some(inst) = data.insts.last()
            && inst.result == *cond
            && matches!(inst.kind, InstKind::Cmp { .. })
        {
            fused[cond.0 as usize] = true;
        }
    }

    // Linear positions.
    let mut start = Vec::with_capacity(count);
    let mut position = 0usize;
    for data in &function.blocks {
        start.push(position);
        position += data.insts.len() + 2;
    }
    let term_pos = |block: usize| start[block] + function.blocks[block].insts.len() + 1;

    // Local uses/defs and liveness.
    let mut def_pos = vec![usize::MAX; bound];
    let mut end = vec![0usize; bound];
    let mut gen_sets = Vec::with_capacity(count);
    let mut defs = Vec::with_capacity(count);
    let mut calls = Vec::new();
    for (block, data) in function.blocks.iter().enumerate() {
        let mut generated = bitset(words);
        let mut defined = bitset(words);
        for (value, _) in &data.params {
            def_pos[value.0 as usize] = start[block];
            set(&mut defined, value.0 as usize);
        }
        let term = term_pos(block);
        for (index, inst) in data.insts.iter().enumerate() {
            let at = start[block] + 1 + index;
            let read_at = if fused[inst.result.0 as usize] {
                term
            } else {
                at
            };
            for operand in inst.kind.operands() {
                let v = operand.0 as usize;
                if !get(&defined, v) {
                    set(&mut generated, v);
                }
                end[v] = end[v].max(read_at);
            }
            if matches!(inst.kind, InstKind::Call { .. }) {
                calls.push(at);
            }
            def_pos[inst.result.0 as usize] = at;
            set(&mut defined, inst.result.0 as usize);
        }
        for operand in data.term.operands() {
            let v = operand.0 as usize;
            if !get(&defined, v) {
                set(&mut generated, v);
            }
            end[v] = end[v].max(term);
        }
        gen_sets.push(generated);
        defs.push(defined);
    }
    let successors: Vec<Vec<usize>> = function
        .blocks
        .iter()
        .map(|data| {
            data.term
                .targets()
                .iter()
                .map(|t| t.block.0 as usize)
                .collect()
        })
        .collect();
    let mut live_in = vec![bitset(words); count];
    let mut live_out = vec![bitset(words); count];
    let mut changed = true;
    while changed {
        changed = false;
        for block in (0..count).rev() {
            let mut out = bitset(words);
            for &successor in &successors[block] {
                for (word, other) in out.iter_mut().zip(&live_in[successor]) {
                    *word |= other;
                }
            }
            let mut inn = gen_sets[block].clone();
            for ((word, o), d) in inn.iter_mut().zip(&out).zip(&defs[block]) {
                *word |= o & !d;
            }
            if inn != live_in[block] || out != live_out[block] {
                live_in[block] = inn;
                live_out[block] = out;
                changed = true;
            }
        }
    }
    for (block, out) in live_out.iter().enumerate() {
        let block_end = term_pos(block) + 1;
        for (word_index, word) in out.iter().enumerate() {
            let mut word = *word;
            while word != 0 {
                let bit = word.trailing_zeros() as usize;
                word &= word - 1;
                let v = word_index * 64 + bit;
                end[v] = end[v].max(block_end);
            }
        }
    }

    // Assign locations.
    let mut loc = vec![Loc::None; bound];
    let mut stats = Stats::default();
    let mut slots = 0i32;
    let spill = |slots: &mut i32| -> Result<Loc, String> {
        *slots = slots.checked_add(1).ok_or("too many spill slots")?;
        let offset = slots
            .checked_mul(-8)
            .ok_or("spill frame exceeds private stack limit")?;
        Ok(Loc::Stack(offset))
    };
    let param_count = function.params.len();
    if param_count > MAX_ARGS {
        return Err(format!("at most {MAX_ARGS} parameters are supported"));
    }
    for (index, (value, _)) in function.blocks[0].params.iter().enumerate() {
        let later = (param_count - 1 - index) as i32;
        loc[value.0 as usize] = Loc::Stack(16 + 8 * later);
    }
    let mut candidates = Vec::new();
    for data in &function.blocks {
        for inst in &data.insts {
            let v = inst.result.0 as usize;
            if let InstKind::Const { bits, .. } = inst.kind {
                let ty = types[v].ok_or("untyped SSA constant")?;
                loc[v] = Loc::Const(image(ty, bits));
            } else if !fused[v] {
                candidates.push(v);
            }
        }
    }
    for data in function.blocks.iter().skip(1) {
        for (value, _) in &data.params {
            candidates.push(value.0 as usize);
        }
    }
    candidates.sort_by_key(|&v| (def_pos[v], v));
    let mut active: Vec<(usize, u8)> = Vec::new(); // (end, reg)
    let mut free: Vec<u8> = ALLOCATABLE.iter().rev().copied().collect();
    for v in candidates {
        let (from, to) = (def_pos[v], end[v].max(def_pos[v]));
        if calls.iter().any(|&call| from < call && call < to) {
            loc[v] = spill(&mut slots)?;
            stats.spilled += 1;
            continue;
        }
        active.retain(|&(active_end, reg)| {
            if active_end <= from {
                free.push(reg);
                false
            } else {
                true
            }
        });
        // Deterministic: always take the lowest-numbered free register.
        free.sort_unstable_by(|a, b| b.cmp(a));
        if let Some(reg) = free.pop() {
            loc[v] = Loc::Reg(reg);
            active.push((to, reg));
            stats.registers += 1;
        } else {
            // Spill whichever interval ends last.
            let (index, &(last_end, reg)) = active
                .iter()
                .enumerate()
                .max_by_key(|(_, (e, r))| (*e, *r))
                .ok_or("register file empty")?;
            if last_end > to {
                // Reassign the value currently holding `reg` to a stack slot.
                let owner = (0..bound)
                    .filter(|&w| loc[w] == Loc::Reg(reg) && end[w].max(def_pos[w]) == last_end)
                    .max_by_key(|&w| def_pos[w])
                    .ok_or("spill victim not found")?;
                loc[owner] = spill(&mut slots)?;
                loc[v] = Loc::Reg(reg);
                active[index] = (to, reg);
            } else {
                loc[v] = spill(&mut slots)?;
            }
            stats.spilled += 1;
        }
    }
    let frame = slots
        .checked_mul(8)
        .map(|bytes| (bytes + 15) & !15)
        .ok_or("spill frame exceeds private stack limit")?;
    stats.fused_branches = fused.iter().filter(|f| **f).count();
    Ok(Layout {
        loc,
        fused,
        frame,
        stats,
    })
}

/// Sequentialize a parallel move. rax breaks cycles, rcx carries
/// memory-to-memory moves. Destinations are distinct storage locations.
fn parallel_move(asm: &mut Asm, moves: &[(Loc, Loc)]) -> Result<(), String> {
    let mut pending: Vec<(Loc, Loc)> = moves.iter().copied().filter(|(d, s)| d != s).collect();
    let emit = |asm: &mut Asm, dst: Loc, src: Loc| -> Result<(), String> {
        match (dst, src) {
            (Loc::Reg(reg), _) => asm.load(reg, src),
            (_, Loc::Reg(reg)) => asm.store(dst, reg),
            _ => {
                asm.load(RCX, src)?;
                asm.store(dst, RCX)
            }
        }
    };
    while !pending.is_empty() {
        let ready = pending
            .iter()
            .position(|(dst, _)| !pending.iter().any(|(_, src)| src == dst));
        match ready {
            Some(index) => {
                let (dst, src) = pending.remove(index);
                emit(asm, dst, src)?;
            }
            None => {
                // Every destination is still read: save one and redirect.
                let (dst, _) = pending[0];
                emit(asm, Loc::Reg(RAX), dst)?;
                for (_, src) in pending.iter_mut() {
                    if *src == dst {
                        *src = Loc::Reg(RAX);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Generate machine code for a verified, optimized function. `panic` renders
/// an explicit panic diagnostic sequence (shared with the stack emitter).
pub fn emit(
    lowered: &Lowered,
    types: &Types,
    panic: &dyn Fn(&SsaPanicText) -> Result<Object, String>,
) -> Result<(Object, Stats), String> {
    let function = &lowered.function;
    let layout = allocate(lowered, types)?;
    let loc = &layout.loc;
    let ty = |value: SsaValue| -> Result<SsaTy, String> {
        types
            .get(value.0 as usize)
            .copied()
            .flatten()
            .ok_or_else(|| "untyped SSA value".to_string())
    };
    let mut asm = Asm {
        code: Vec::new(),
        traps: Vec::new(),
        calls: Vec::new(),
        strings: Vec::new(),
    };
    asm.bytes(&[0x55, 0x48, 0x89, 0xe5]); // push rbp; mov rbp, rsp
    if layout.frame > 0 {
        asm.bytes(&[0x48, 0x81, 0xec]);
        asm.i32(layout.frame);
    }
    let mut labels = vec![0usize; function.blocks.len()];
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let edge_moves = |target: &super::SsaTarget| -> Vec<(Loc, Loc)> {
        let params = &function.blocks[target.block.0 as usize].params;
        params
            .iter()
            .zip(&target.args)
            .map(|((param, _), arg)| (loc[param.0 as usize], loc[arg.0 as usize]))
            .collect()
    };
    for (block, data) in function.blocks.iter().enumerate() {
        labels[block] = asm.code.len();
        for inst in &data.insts {
            let result = inst.result;
            let out = loc[result.0 as usize];
            match &inst.kind {
                InstKind::Const { .. } => continue,
                InstKind::Cmp { .. } if layout.fused[result.0 as usize] => continue,
                InstKind::Binary {
                    op,
                    overflow,
                    lhs,
                    rhs,
                } => {
                    let t = ty(*lhs)?;
                    let int = int_of(t);
                    asm.load(RAX, loc[lhs.0 as usize])?;
                    asm.load(RCX, loc[rhs.0 as usize])?;
                    let trap = *overflow == Overflow::Trap;
                    match op {
                        BinOp::And => asm.bytes(&[0x48, 0x21, 0xc8]),
                        BinOp::Or => asm.bytes(&[0x48, 0x09, 0xc8]),
                        BinOp::Xor => asm.bytes(&[0x48, 0x31, 0xc8]),
                        BinOp::Add | BinOp::Sub => {
                            asm.bytes(&[0x48, if *op == BinOp::Add { 0x01 } else { 0x29 }, 0xc8]);
                            if trap {
                                if int.bits() == 64 {
                                    asm.trap_if(if int.signed() { 0x80 } else { 0x82 });
                                } else {
                                    if !int.signed() {
                                        asm.trap_if(0x82);
                                    }
                                    asm.check_range(t);
                                }
                            } else {
                                asm.normalize(t);
                            }
                        }
                        BinOp::Mul => {
                            if int.signed() || !trap {
                                asm.bytes(&[0x48, 0x0f, 0xaf, 0xc1]); // imul rax, rcx
                                if trap {
                                    asm.trap_if(0x80);
                                }
                            } else {
                                asm.bytes(&[0x48, 0xf7, 0xe1]); // mul rcx
                                asm.trap_if(0x82);
                            }
                            if trap {
                                asm.check_range(t);
                            } else {
                                asm.normalize(t);
                            }
                        }
                        BinOp::Div | BinOp::Rem => {
                            asm.bytes(&[0x48, 0x85, 0xc9]); // test rcx, rcx
                            asm.trap_if(0x84);
                            if int.signed() {
                                let minimum = u64::MAX << (int.bits() - 1);
                                asm.bytes(&[0x48, 0xba]);
                                asm.bytes(&minimum.to_le_bytes());
                                asm.bytes(&[0x48, 0x39, 0xd0]); // cmp rax, rdx
                                let ordinary = asm.rel32(&[0x0f, 0x85]);
                                asm.bytes(&[0x48, 0x83, 0xf9, 0xff]); // cmp rcx, -1
                                asm.trap_if(0x84);
                                let here = asm.code.len();
                                asm.patch(ordinary, here)?;
                                asm.bytes(&[0x48, 0x99, 0x48, 0xf7, 0xf9]); // cqo; idiv rcx
                            } else {
                                asm.bytes(&[0x31, 0xd2, 0x48, 0xf7, 0xf1]); // xor edx; div rcx
                            }
                            if *op == BinOp::Rem {
                                asm.bytes(&[0x48, 0x89, 0xd0]);
                            }
                        }
                        BinOp::Shl | BinOp::Shr => {
                            let width = int.bits();
                            if trap {
                                // Compare the amount's raw pattern: a normalized
                                // negative amount is a huge unsigned value.
                                let amount = int_of(ty(*rhs)?);
                                if amount.signed() && amount.bits() < 64 {
                                    // Raw pattern of the declared amount width.
                                    let shift = 64 - amount.bits();
                                    asm.bytes(&[0x48, 0xc1, 0xe1, shift]);
                                    asm.bytes(&[0x48, 0xc1, 0xe9, shift]);
                                }
                                asm.bytes(&[0x48, 0x83, 0xf9, width]); // cmp rcx, width
                                asm.trap_if(0x83);
                            } else {
                                asm.bytes(&[0x83, 0xe1, width - 1]); // and ecx, width-1
                            }
                            asm.bytes(&[
                                0x48,
                                0xd3,
                                match op {
                                    BinOp::Shl => 0xe0,
                                    _ if int.signed() => 0xf8,
                                    _ => 0xe8,
                                },
                            ]);
                            asm.normalize(t);
                        }
                    }
                }
                InstKind::Unary {
                    op,
                    overflow,
                    value,
                } => {
                    let t = ty(*value)?;
                    asm.load(RAX, loc[value.0 as usize])?;
                    match (op, t) {
                        (UnOp::Not, SsaTy::Bool) => asm.bytes(&[0x83, 0xf0, 0x01]),
                        (UnOp::Not, _) => {
                            asm.bytes(&[0x48, 0xf7, 0xd0]);
                            asm.normalize(t);
                        }
                        (UnOp::Neg, _) => {
                            asm.bytes(&[0x48, 0xf7, 0xd8]);
                            if *overflow == Overflow::Trap {
                                asm.trap_if(0x80);
                                asm.check_range(t);
                            } else {
                                asm.normalize(t);
                            }
                        }
                    }
                }
                InstKind::Cmp { op, lhs, rhs } => {
                    let signed = matches!(ty(*lhs)?, SsaTy::Int(int) if int.signed());
                    asm.load(RAX, loc[lhs.0 as usize])?;
                    asm.load(RCX, loc[rhs.0 as usize])?;
                    asm.bytes(&[0x48, 0x39, 0xc8]);
                    asm.bytes(&[0x0f, 0x90 | condition(*op, signed), 0xc0, 0x0f, 0xb6, 0xc0]);
                }
                InstKind::Cast { to, value } => {
                    // Values are already extended by source signedness.
                    asm.load(RAX, loc[value.0 as usize])?;
                    asm.normalize(SsaTy::Int(*to));
                }
                InstKind::Select {
                    cond,
                    then,
                    otherwise,
                } => {
                    asm.load(RAX, loc[then.0 as usize])?;
                    asm.load(RCX, loc[otherwise.0 as usize])?;
                    asm.load(RDX, loc[cond.0 as usize])?;
                    asm.bytes(&[0x48, 0x85, 0xd2, 0x48, 0x0f, 0x44, 0xc1]); // test; cmovz
                }
                InstKind::Call { callee, args } => {
                    if args.len() > MAX_ARGS {
                        return Err(format!("at most {MAX_ARGS} arguments are supported"));
                    }
                    let callee = lowered
                        .callees
                        .get(callee.0 as usize)
                        .ok_or("call to an unknown SSA callee")?;
                    for arg in args {
                        asm.push(loc[arg.0 as usize])?;
                    }
                    let at = asm.rel32(&[0xe8]);
                    asm.calls.push((at, callee.symbol.clone()));
                    if !args.is_empty() {
                        asm.bytes(&[0x48, 0x81, 0xc4]);
                        asm.i32((args.len() * 8) as i32);
                    }
                }
            }
            asm.store(out, RAX)?;
        }
        let next = block + 1;
        match &data.term {
            SsaTerminator::Return(value) => {
                asm.load(RAX, loc[value.0 as usize])?;
                asm.bytes(&[0xc9, 0xc3]); // leave; ret
            }
            SsaTerminator::Panic(PanicKind::Unreachable) => asm.bytes(&[0x0f, 0x0b]),
            SsaTerminator::Panic(PanicKind::Message(index)) => {
                let text = function
                    .panics
                    .get(*index as usize)
                    .ok_or("missing SSA panic message")?;
                let object = panic(text)?;
                if !object.calls.is_empty() || !object.vtables.is_empty() {
                    return Err("panic sequence must be self-contained".into());
                }
                let base = asm.code.len();
                asm.bytes(&object.code);
                for (at, bytes) in object.strings {
                    asm.strings.push((base + at, bytes));
                }
            }
            SsaTerminator::Panic(_) => {
                let at = asm.rel32(&[0xe9]);
                asm.traps.push(at);
            }
            SsaTerminator::Jump(target) => {
                parallel_move(&mut asm, &edge_moves(target))?;
                let to = target.block.0 as usize;
                if to != next {
                    let at = asm.rel32(&[0xe9]);
                    edges.push((at, to));
                }
            }
            SsaTerminator::Branch {
                cond,
                then,
                otherwise,
            } => {
                // Jump to `otherwise` when the condition is false.
                let false_jump = if layout.fused[cond.0 as usize] {
                    let Some(super::Inst {
                        kind: InstKind::Cmp { op, lhs, rhs },
                        ..
                    }) = data.insts.last()
                    else {
                        return Err("fused branch lost its comparison".into());
                    };
                    let signed = matches!(ty(*lhs)?, SsaTy::Int(int) if int.signed());
                    asm.load(RAX, loc[lhs.0 as usize])?;
                    asm.load(RCX, loc[rhs.0 as usize])?;
                    asm.bytes(&[0x48, 0x39, 0xc8]);
                    0x80 | (condition(*op, signed) ^ 1)
                } else {
                    asm.load(RAX, loc[cond.0 as usize])?;
                    asm.bytes(&[0x48, 0x85, 0xc0]);
                    0x84
                };
                let then_moves = edge_moves(then);
                let else_moves = edge_moves(otherwise);
                let to_else = asm.rel32(&[0x0f, false_jump]);
                parallel_move(&mut asm, &then_moves)?;
                let then_block = then.block.0 as usize;
                if then_block != next || !else_moves.iter().all(|(d, s)| d == s) {
                    let at = asm.rel32(&[0xe9]);
                    edges.push((at, then_block));
                }
                if else_moves.iter().all(|(d, s)| d == s) {
                    edges.push((to_else, otherwise.block.0 as usize));
                    if then_block == next {
                        // Fallthrough already handled above.
                    }
                } else {
                    let here = asm.code.len();
                    asm.patch(to_else, here)?;
                    parallel_move(&mut asm, &else_moves)?;
                    let at = asm.rel32(&[0xe9]);
                    edges.push((at, otherwise.block.0 as usize));
                }
            }
        }
    }
    for (at, block) in edges {
        asm.patch(at, labels[block])?;
    }
    let trap = asm.code.len();
    asm.bytes(&[
        0xbf, 101, 0, 0, 0, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
    ]);
    for at in std::mem::take(&mut asm.traps) {
        asm.patch(at, trap)?;
    }
    let mut stats = layout.stats;
    stats.code_bytes = asm.code.len();
    Ok((
        Object {
            code: asm.code,
            calls: asm.calls,
            strings: asm.strings,
            vtables: Vec::new(),
        },
        stats,
    ))
}
