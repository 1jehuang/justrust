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
use super::meter::{self, Meter, Table};
use super::store::{self, Owned};
use super::{
    BinOp, CmpOp, InstKind, Overflow, PanicKind, SsaIntTy, SsaPanicText, SsaTerminator, SsaTy,
    SsaValue, UnOp,
};
use crate::compiler::backend::Object;
use std::mem::size_of;

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
    loc: Table<Loc>,
    fused: Table<bool>,
    frame: i32,
    stats: Stats,
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

/// Register allocation with paid scratch (audit priority 5): every table is
/// admitted before allocation; the four liveness matrices are single flat
/// `blocks x words` tables with two reused scratch rows; every fixed-point
/// round charges its word visits; the call-crossing test is a binary search
/// over sorted call positions; spill victims are found through a
/// register-owner table rather than a scan over all values.
fn allocate(
    lowered: &Lowered,
    types: &[Option<SsaTy>],
    meter: &Meter<'_>,
) -> Result<Layout, String> {
    let function = &lowered.function;
    let bound = function.value_bound();
    let words = bound.div_ceil(64).max(1);
    let count = function.blocks.len();

    // Use counts, for fusion.
    let mut uses = meter.table(bound, 0usize)?;
    for data in &function.blocks {
        meter.work(data.term.operand_count())?;
        for inst in &data.insts {
            meter.work(inst.kind.operand_count())?;
            for operand in inst.kind.operands() {
                uses[operand.0 as usize] += 1;
            }
        }
        for operand in data.term.operands() {
            uses[operand.0 as usize] += 1;
        }
    }
    let mut fused = meter.table(bound, false)?;
    meter.work(count)?;
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
    drop(uses);

    // Linear positions.
    let mut start = meter.table(count, 0usize)?;
    let mut position = 0usize;
    let mut call_count = 0usize;
    for (block, data) in function.blocks.iter().enumerate() {
        start[block] = position;
        position = meter::add(position, meter::add(data.insts.len(), 2)?)?;
        call_count += data
            .insts
            .iter()
            .filter(|inst| matches!(inst.kind, InstKind::Call { .. }))
            .count();
    }
    let term_pos = |block: usize| start[block] + function.blocks[block].insts.len() + 1;

    // Local uses/defs and liveness. Flat matrices: row `b` is
    // `[b * words .. (b + 1) * words]`.
    let cells = meter::mul(count, words)?;
    let mut def_pos = meter.table(bound, usize::MAX)?;
    let mut end = meter.table(bound, 0usize)?;
    let mut generated = meter.table(cells, 0u64)?;
    let mut defined = meter.table(cells, 0u64)?;
    let mut calls = meter.list::<usize>(call_count)?;
    for (block, data) in function.blocks.iter().enumerate() {
        let row = block * words..(block + 1) * words;
        meter.work(meter::add(
            meter::add(data.params.len(), data.insts.len())?,
            data.term.operand_count(),
        )?)?;
        for (value, _) in &data.params {
            def_pos[value.0 as usize] = start[block];
            set(&mut defined[row.clone()], value.0 as usize);
        }
        let term = term_pos(block);
        for (index, inst) in data.insts.iter().enumerate() {
            let at = start[block] + 1 + index;
            let read_at = if fused[inst.result.0 as usize] {
                term
            } else {
                at
            };
            meter.work(inst.kind.operand_count())?;
            for operand in inst.kind.operands() {
                let v = operand.0 as usize;
                if !get(&defined[row.clone()], v) {
                    set(&mut generated[row.clone()], v);
                }
                end[v] = end[v].max(read_at);
            }
            if matches!(inst.kind, InstKind::Call { .. }) {
                calls.push(at)?;
            }
            def_pos[inst.result.0 as usize] = at;
            set(&mut defined[row.clone()], inst.result.0 as usize);
        }
        for operand in data.term.operands() {
            let v = operand.0 as usize;
            if !get(&defined[row.clone()], v) {
                set(&mut generated[row.clone()], v);
            }
            end[v] = end[v].max(term);
        }
    }
    let mut live_in = meter.table(cells, 0u64)?;
    let mut live_out = meter.table(cells, 0u64)?;
    let mut out = meter.table(words, 0u64)?;
    let mut inn = meter.table(words, 0u64)?;
    let mut changed = true;
    while changed {
        changed = false;
        // One round visits every block row plus every successor row.
        meter.work2(count, words)?;
        for block in (0..count).rev() {
            let row = block * words..(block + 1) * words;
            out.iter_mut().for_each(|word| *word = 0);
            for target in function.blocks[block].term.targets() {
                meter.work(words)?;
                let successor = target.block.0 as usize;
                let other = &live_in[successor * words..(successor + 1) * words];
                for (word, other) in out.iter_mut().zip(other) {
                    *word |= other;
                }
            }
            for (((word, g), o), d) in inn
                .iter_mut()
                .zip(&generated[row.clone()])
                .zip(out.iter())
                .zip(&defined[row.clone()])
            {
                *word = g | (o & !d);
            }
            if inn[..] != live_in[row.clone()] || out[..] != live_out[row.clone()] {
                live_in[row.clone()].copy_from_slice(&inn);
                live_out[row].copy_from_slice(&out);
                changed = true;
            }
        }
    }
    drop((generated, defined, live_in, out, inn));
    for block in 0..count {
        let block_end = term_pos(block) + 1;
        meter.work(words)?;
        for (word_index, word) in live_out[block * words..(block + 1) * words]
            .iter()
            .enumerate()
        {
            let mut word = *word;
            while word != 0 {
                meter.work(1)?;
                let bit = word.trailing_zeros() as usize;
                word &= word - 1;
                let v = word_index * 64 + bit;
                end[v] = end[v].max(block_end);
            }
        }
    }
    drop(live_out);

    // Assign locations.
    let mut loc = meter.table(bound, Loc::None)?;
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
    let candidate_count = function.inst_count()
        + function
            .blocks
            .iter()
            .skip(1)
            .map(|data| data.params.len())
            .sum::<usize>();
    let mut candidates = meter.list::<usize>(candidate_count)?;
    for data in &function.blocks {
        meter.work(data.insts.len())?;
        for inst in &data.insts {
            let v = inst.result.0 as usize;
            if let InstKind::Const { bits, .. } = inst.kind {
                let ty = types[v].ok_or("untyped SSA constant")?;
                loc[v] = Loc::Const(image(ty, bits));
            } else if !fused[v] {
                candidates.push(v)?;
            }
        }
    }
    for data in function.blocks.iter().skip(1) {
        meter.work(data.params.len())?;
        for (value, _) in &data.params {
            candidates.push(value.0 as usize)?;
        }
    }
    // Sorting n candidates: charge n * ceil(log2 n) comparisons.
    let n = candidates.len();
    meter.work2(n, (usize::BITS - n.leading_zeros()) as usize)?;
    candidates.sort_unstable_by_key(|&v| (def_pos[v], v));
    // Calls are recorded in increasing position order already.
    let crosses_call = |from: usize, to: usize| -> Result<bool, String> {
        meter.work((usize::BITS - calls.len().leading_zeros()) as usize + 1)?;
        let first_after = calls.partition_point(|&call| call <= from);
        Ok(calls.get(first_after).is_some_and(|&call| call < to))
    };
    // Register file state: fixed size, no heap growth.
    let mut active: [(usize, u8, usize); 16] = [(0, 0, 0); 16]; // (end, reg, owner)
    let mut active_len = 0usize;
    let mut free = [false; 16];
    for reg in ALLOCATABLE {
        free[reg as usize] = true;
    }
    for &v in candidates.iter() {
        meter.work(ALLOCATABLE.len())?;
        let (from, to) = (def_pos[v], end[v].max(def_pos[v]));
        if crosses_call(from, to)? {
            loc[v] = spill(&mut slots)?;
            stats.spilled += 1;
            continue;
        }
        let mut kept = 0;
        for index in 0..active_len {
            let entry = active[index];
            if entry.0 <= from {
                free[entry.1 as usize] = true;
            } else {
                active[kept] = entry;
                kept += 1;
            }
        }
        active_len = kept;
        // Deterministic: always take the lowest-numbered free register.
        let lowest = ALLOCATABLE
            .iter()
            .copied()
            .filter(|r| free[*r as usize])
            .min();
        if let Some(reg) = lowest {
            free[reg as usize] = false;
            loc[v] = Loc::Reg(reg);
            active[active_len] = (to, reg, v);
            active_len += 1;
            stats.registers += 1;
        } else {
            // Spill whichever interval ends last.
            let (index, &(last_end, reg, owner)) = active[..active_len]
                .iter()
                .enumerate()
                .max_by_key(|(_, (e, r, _))| (*e, *r))
                .ok_or("register file empty")?;
            if last_end > to {
                loc[owner] = spill(&mut slots)?;
                loc[v] = Loc::Reg(reg);
                active[index] = (to, reg, v);
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
/// `pending` is caller-owned paid scratch, reused across edges; the search is
/// charged per comparison (moves are at most the target's parameter count).
fn parallel_move(
    asm: &mut Asm,
    moves: &[(Loc, Loc)],
    pending: &mut meter::List<(Loc, Loc)>,
    meter: &Meter<'_>,
) -> Result<(), String> {
    pending.clear();
    for &(dst, src) in moves {
        if dst != src {
            pending.push((dst, src))?;
        }
    }
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
        let n = pending.len();
        meter.work2(n, n)?;
        let ready = pending
            .iter()
            .position(|(dst, _)| !pending.iter().any(|(_, src)| src == dst));
        match ready {
            Some(index) => {
                let (dst, src) = pending.swap_remove(index);
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

/// Longest encoding (bytes) any single instruction kind emits, including
/// operand loads and the result store. Div/Rem with the signed guard is the
/// longest sequence.
const MAX_INST_BYTES: usize = 96;
/// Longest single move (load + store through rcx) or push.
const MAX_MOVE_BYTES: usize = 24;
/// Terminator overhead: compare, jumps, leave/ret.
const MAX_TERM_BYTES: usize = 48;
const PROLOGUE_BYTES: usize = 16;
const TRAP_BYTES: usize = 14;

/// Sound upper bounds on emitted output, computed before emission so code,
/// relocation and label tables are admitted at their maximum capacity.
struct EmitBound {
    code: usize,
    traps: usize,
    calls: usize,
    edges: usize,
    moves: usize,
    panic_texts: usize,
}

fn emit_bound(lowered: &Lowered, meter: &Meter<'_>) -> Result<EmitBound, String> {
    let function = &lowered.function;
    let mut bound = EmitBound {
        code: PROLOGUE_BYTES + TRAP_BYTES,
        traps: 0,
        calls: 0,
        edges: 0,
        moves: 0,
        panic_texts: 0,
    };
    for data in &function.blocks {
        meter.work(meter::add(data.insts.len(), data.term.operand_count())?)?;
        for inst in &data.insts {
            let pushes = match &inst.kind {
                InstKind::Call { args, .. } => {
                    bound.calls = meter::add(bound.calls, 1)?;
                    args.len()
                }
                _ => 0,
            };
            bound.code = meter::add(
                bound.code,
                meter::add(MAX_INST_BYTES, meter::mul(pushes, MAX_MOVE_BYTES)?)?,
            )?;
            // At most four trap sites per instruction (div signed guard).
            bound.traps = meter::add(bound.traps, 4)?;
        }
        let mut moves = 0usize;
        for target in data.term.targets() {
            moves = meter::add(moves, target.args.len())?;
        }
        bound.moves = bound.moves.max(moves);
        // A cycle costs one extra move per argument at most.
        bound.code = meter::add(
            bound.code,
            meter::add(
                MAX_TERM_BYTES,
                meter::mul(meter::mul(moves, 2)?, MAX_MOVE_BYTES)?,
            )?,
        )?;
        bound.edges = meter::add(bound.edges, 3)?;
        bound.traps = meter::add(bound.traps, 1)?;
        if matches!(data.term, SsaTerminator::Panic(PanicKind::Message(_))) {
            bound.panic_texts = meter::add(bound.panic_texts, 1)?;
        }
    }
    Ok(bound)
}

/// Generate machine code for a verified, optimized function. `panic` renders
/// an explicit panic diagnostic sequence (shared with the stack emitter).
///
/// Accounting: register allocation scratch is paid (`allocate`). The code
/// buffer and relocation tables are reserved at proven upper bounds
/// (`emit_bound`) and that reservation is admitted first; panic sequences from
/// the callback are moved in after admitting their exact bytes. The
/// published `Object` is reconciled to its real capacities, and its receipt
/// (`Emitted::receipt`) stays with the object for its lifetime. Pushing past
/// a bound is an internal error, never unpaid growth.
pub fn emit(
    lowered: &Lowered,
    types: &[Option<SsaTy>],
    panic: &dyn Fn(&SsaPanicText) -> Result<Object, String>,
    meter: &Meter<'_>,
) -> Result<(Owned<Object>, Stats), String> {
    let function = &lowered.function;
    let layout = allocate(lowered, types, meter)?;
    let bound = emit_bound(lowered, meter)?;
    let mut lease = meter.lease(object_bound_bytes(&bound, lowered)?)?;
    let loc = &layout.loc;
    let ty = |value: SsaValue| -> Result<SsaTy, String> {
        types
            .get(value.0 as usize)
            .copied()
            .flatten()
            .ok_or_else(|| "untyped SSA value".to_string())
    };
    let mut asm = Asm {
        code: reserve(bound.code)?,
        traps: reserve(bound.traps)?,
        calls: reserve(bound.calls)?,
        strings: reserve(bound.panic_texts)?,
    };
    asm.bytes(&[0x55, 0x48, 0x89, 0xe5]); // push rbp; mov rbp, rsp
    if layout.frame > 0 {
        asm.bytes(&[0x48, 0x81, 0xec]);
        asm.i32(layout.frame);
    }
    let mut labels = meter.table(function.blocks.len(), 0usize)?;
    let mut edges = meter.list::<(usize, usize)>(bound.edges)?;
    let mut pending = meter.list::<(Loc, Loc)>(bound.moves)?;
    let mut then_moves = meter.list::<(Loc, Loc)>(bound.moves)?;
    let mut else_moves = meter.list::<(Loc, Loc)>(bound.moves)?;
    let edge_moves =
        |target: &super::SsaTarget, out: &mut meter::List<(Loc, Loc)>| -> Result<(), String> {
            out.clear();
            let params = &function.blocks[target.block.0 as usize].params;
            meter.work(params.len())?;
            for ((param, _), arg) in params.iter().zip(&target.args) {
                out.push((loc[param.0 as usize], loc[arg.0 as usize]))?;
            }
            Ok(())
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
                    // The symbol string is a new copy owned by the object.
                    let symbol = store::try_string(&callee.symbol)?;
                    lease.admit(meter, symbol.capacity())?;
                    meter.work(symbol.len())?;
                    bounded_push(&mut asm.calls, (at, symbol))?;
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
                // The callback admits its own diagnostic bytes; its code is
                // copied here, so the copy is admitted before the code buffer
                // grows past its bound. String payloads are moved, not copied.
                let object = panic(text)?;
                if !object.calls.is_empty() || !object.vtables.is_empty() {
                    return Err("panic sequence must be self-contained".into());
                }
                meter.work(object.code.len())?;
                let base = asm.code.len();
                let needed = meter::add(base, object.code.len())?;
                if needed > asm.code.capacity() {
                    lease.admit(meter, needed - asm.code.capacity())?;
                    asm.code
                        .try_reserve_exact(needed - asm.code.len())
                        .map_err(|_| "optimizer allocation failed".to_string())?;
                }
                asm.code.extend_from_slice(&object.code);
                for (at, bytes) in object.strings {
                    bounded_push(&mut asm.strings, (base + at, bytes))?;
                }
            }
            SsaTerminator::Panic(_) => {
                let at = asm.rel32(&[0xe9]);
                bounded_push(&mut asm.traps, at)?;
            }
            SsaTerminator::Jump(target) => {
                edge_moves(target, &mut then_moves)?;
                parallel_move(&mut asm, &then_moves, &mut pending, meter)?;
                let to = target.block.0 as usize;
                if to != next {
                    let at = asm.rel32(&[0xe9]);
                    edges.push((at, to))?;
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
                edge_moves(then, &mut then_moves)?;
                edge_moves(otherwise, &mut else_moves)?;
                let to_else = asm.rel32(&[0x0f, false_jump]);
                parallel_move(&mut asm, &then_moves, &mut pending, meter)?;
                let then_block = then.block.0 as usize;
                let else_trivial = else_moves.iter().all(|(d, s)| d == s);
                if then_block != next || !else_trivial {
                    let at = asm.rel32(&[0xe9]);
                    edges.push((at, then_block))?;
                }
                if else_trivial {
                    edges.push((to_else, otherwise.block.0 as usize))?;
                } else {
                    let here = asm.code.len();
                    asm.patch(to_else, here)?;
                    parallel_move(&mut asm, &else_moves, &mut pending, meter)?;
                    let at = asm.rel32(&[0xe9]);
                    edges.push((at, otherwise.block.0 as usize))?;
                }
            }
        }
    }
    meter.work(edges.len())?;
    for &(at, block) in edges.iter() {
        asm.patch(at, labels[block])?;
    }
    let trap = asm.code.len();
    asm.bytes(&[
        0xbf, 101, 0, 0, 0, 0xb8, 60, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
    ]);
    meter.work(asm.traps.len())?;
    for at in std::mem::take(&mut asm.traps) {
        asm.patch(at, trap)?;
    }
    if asm.code.len() > asm.code.capacity() {
        return Err("machine code exceeded its admitted bound".into());
    }
    let mut stats = layout.stats;
    stats.code_bytes = asm.code.len();
    let object = Object {
        code: asm.code,
        calls: asm.calls,
        strings: asm.strings,
        vtables: Vec::new(),
        receipt: None,
    };
    // Reconcile to the real retained capacity (bounds were upper bounds).
    let actual = object_bytes(&object)?;
    let owned = store::Owned::adopt(lease, object, actual, meter)?;
    Ok((owned, stats))
}

/// Pushes within a reserved bound; exceeding it is an internal error.
fn bounded_push<T>(data: &mut Vec<T>, value: T) -> Result<(), String> {
    if data.len() == data.capacity() {
        return Err("machine emission exceeded its admitted bound".into());
    }
    data.push(value);
    Ok(())
}

fn reserve<T>(capacity: usize) -> Result<Vec<T>, String> {
    let mut data = Vec::new();
    data.try_reserve_exact(capacity)
        .map_err(|_| "optimizer allocation failed".to_string())?;
    Ok(data)
}

fn object_bound_bytes(bound: &EmitBound, _lowered: &Lowered) -> Result<usize, String> {
    [
        Ok(bound.code),
        meter::mul(bound.traps, size_of::<usize>()),
        meter::mul(bound.calls, size_of::<(usize, String)>()),
        meter::mul(bound.panic_texts, size_of::<(usize, Vec<u8>)>()),
    ]
    .into_iter()
    .try_fold(0usize, |total, part| meter::add(total, part?))
}

/// Bytes the published object owns: buffer capacities plus call symbol
/// strings. Panic string payloads are moved from callback objects whose
/// producer admitted them, so they are not charged again here.
fn object_bytes(object: &Object) -> Result<usize, String> {
    let mut total = [
        Ok(object.code.capacity()),
        meter::mul(object.calls.capacity(), size_of::<(usize, String)>()),
        meter::mul(object.strings.capacity(), size_of::<(usize, Vec<u8>)>()),
    ]
    .into_iter()
    .try_fold(0usize, |total, part: Result<usize, String>| {
        meter::add(total, part?)
    })?;
    for (_, symbol) in &object.calls {
        total = meter::add(total, symbol.capacity())?;
    }
    Ok(total)
}
