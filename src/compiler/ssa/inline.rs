//! SsaCallee summaries and bottom-up inlining (OPTIMIZATION.md sections 8, 8b).
//!
//! `SUMMARY` is a small per-function fact that callers read instead of a
//! callee's body. `inline` replaces calls to small, call-free callees with
//! their optimized bodies. Each inlined body is an explicit input: the graph
//! node that runs this pass requests the callee's optimized body as a recorded
//! dependency, so an edit to the callee invalidates exactly its inliners.
//!
//! Decisions depend only on canonical inputs (callee summaries and the fixed
//! size threshold), never on scheduling order, so output is deterministic.
//!
//! Accounting (audit priority 3): `inline` first counts every call site and
//! the exact output dimensions (blocks, instructions, call arguments, edge
//! arguments, panic bytes, value and block IDs) with checked arithmetic and
//! per-element work, admits the whole output before allocating, then builds
//! it in one pass into exact-capacity buffers. No repeated searches, no
//! clone-then-extend, no unpaid growth.
use super::lower::{Lowered, SsaCallee};
use super::meter::{self, Meter};
use super::store::{self, Owned};
use super::{
    BlockData, FuncRef, Inst, InstKind, PanicKind, SsaBlock, SsaFunction, SsaPanicText, SsaTarget,
    SsaTerminator, SsaValue,
};
use std::mem::size_of;

/// Largest callee body (instructions) that may be inlined.
pub const INLINE_LIMIT: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Summary {
    /// Content hash of the optimized body. Callers that inline record it.
    pub body_hash: u128,
    pub insts: usize,
    pub blocks: usize,
    /// Contains no calls, so inlining cannot introduce recursion.
    pub leaf: bool,
    /// May panic or trap on some input.
    pub may_panic: bool,
}

impl Summary {
    /// Scans every instruction once and hashes the body; all work is metered.
    /// The summary is `Copy` and owns no heap storage.
    pub fn of(lowered: &Lowered, meter: &Meter<'_>) -> Result<Self, String> {
        let function = &lowered.function;
        let mut leaf = true;
        let mut may_panic = false;
        for data in &function.blocks {
            meter.work(meter::add(data.insts.len(), 1)?)?;
            for inst in &data.insts {
                match inst.kind {
                    InstKind::Call { .. } => {
                        leaf = false;
                        may_panic = true;
                    }
                    InstKind::Binary { op, overflow, .. } => {
                        may_panic |= overflow == super::Overflow::Trap
                            || matches!(op, super::BinOp::Div | super::BinOp::Rem);
                    }
                    InstKind::Unary { overflow, .. } => {
                        may_panic |= overflow == super::Overflow::Trap;
                    }
                    _ => {}
                }
            }
            may_panic |= matches!(data.term, SsaTerminator::Panic(_));
        }
        Ok(Summary {
            body_hash: lowered.hash(meter)?,
            insts: function.inst_count(),
            blocks: function.blocks.len(),
            leaf,
            may_panic,
        })
    }

    pub fn inlinable(&self) -> bool {
        self.leaf && self.insts <= INLINE_LIMIT
    }
}

/// Callees (by index into `lowered.callees`) worth inlining, given each
/// callee's summary. Deterministic in the inputs. The result list is paid
/// scratch sized by the callee count.
pub fn candidates(
    lowered: &Lowered,
    summary: &dyn Fn(usize) -> Option<Summary>,
    meter: &Meter<'_>,
) -> Result<meter::List<usize>, String> {
    let count = lowered.callees.len();
    meter.work(count)?;
    let mut chosen = meter.list(count)?;
    for index in 0..count {
        if summary(index).is_some_and(|s| s.inlinable()) {
            chosen.push(index)?;
        }
    }
    Ok(chosen)
}

/// Exact output dimensions of an inline, from one counting pass.
struct Expansion {
    blocks: usize,
    insts: usize,
    params: usize,
    call_args: usize,
    edge_args: usize,
    panics: usize,
    panic_bytes: usize,
    values: usize,
}

/// Inline every call to a callee present in `bodies` (index into
/// `lowered.callees` -> optimized callee). Bodies must be leaves.
///
/// Layout of the output: the caller's blocks first, in order; each block that
/// contains inlined calls is split at every such call, with the split tails
/// appended as continuation blocks (receiving the call result as their one
/// parameter) and each callee copy's blocks appended after it.
pub fn inline(
    lowered: &Lowered,
    bodies: &[(usize, &Lowered)],
    meter: &Meter<'_>,
) -> Result<Owned<Lowered>, String> {
    let callees = lowered.callees.len();
    // Callee index -> body, O(callees + bodies) instead of a search per call.
    let mut body_of = meter.table::<Option<&Lowered>>(callees, None)?;
    for (index, body) in bodies {
        if !body.callees.is_empty() {
            return Err("only leaf callees are inlined".into());
        }
        *body_of
            .get_mut(*index)
            .ok_or("inline body for an unknown callee")? = Some(body);
    }
    meter.work(meter::add(callees, bodies.len())?)?;
    let caller = &lowered.function;
    let inlined_at = |inst: &Inst| -> Option<&Lowered> {
        match inst.kind {
            InstKind::Call { callee, .. } => body_of.get(callee.0 as usize).copied().flatten(),
            _ => None,
        }
    };

    // Counting pass.
    let mut x = Expansion {
        blocks: caller.blocks.len(),
        insts: 0,
        params: 0,
        call_args: 0,
        edge_args: 0,
        panics: caller.panics.len(),
        panic_bytes: 0,
        values: caller.value_bound(),
    };
    for text in &caller.panics {
        x.panic_bytes = meter::add(
            x.panic_bytes,
            meter::add(text.message.len(), text.location.len())?,
        )?;
    }
    let mut used = meter.table(callees, false)?;
    for data in &caller.blocks {
        meter.work(meter::add(data.insts.len(), data.term.operand_count())?)?;
        x.params = meter::add(x.params, data.params.len())?;
        x.edge_args = meter::add(x.edge_args, data.term.operand_count())?;
        for inst in &data.insts {
            match inlined_at(inst) {
                None => {
                    x.insts = meter::add(x.insts, 1)?;
                    if let InstKind::Call { callee, args } = &inst.kind {
                        x.call_args = meter::add(x.call_args, args.len())?;
                        used[callee.0 as usize] = true;
                    }
                }
                Some(body) => {
                    let callee = &body.function;
                    meter.work(meter::add(callee.blocks.len(), callee.inst_count())?)?;
                    // One continuation block with one parameter, plus the
                    // callee's blocks; the call site's jump passes its args.
                    x.blocks = meter::add(x.blocks, meter::add(callee.blocks.len(), 1)?)?;
                    x.params = meter::add(x.params, 1)?;
                    x.insts = meter::add(x.insts, callee.inst_count())?;
                    x.values = meter::add(x.values, callee.value_bound())?;
                    if let InstKind::Call { args, .. } = &inst.kind {
                        x.edge_args = meter::add(x.edge_args, args.len())?;
                    }
                    for block in &callee.blocks {
                        x.params = meter::add(x.params, block.params.len())?;
                        // A return becomes a one-argument jump.
                        x.edge_args = meter::add(x.edge_args, block.term.operand_count())?;
                    }
                    x.panics = meter::add(x.panics, callee.panics.len())?;
                    for text in &callee.panics {
                        x.panic_bytes = meter::add(
                            x.panic_bytes,
                            meter::add(text.message.len(), text.location.len())?,
                        )?;
                    }
                }
            }
        }
    }
    u32::try_from(x.values).map_err(|_| "inlined value IDs exhausted")?;
    u32::try_from(x.blocks).map_err(|_| "inlined block IDs exhausted")?;
    u32::try_from(x.panics).map_err(|_| "inlined panic IDs exhausted")?;
    let kept: usize = used.iter().filter(|u| **u).count();
    let mut callee_shape = meter::mul(kept, size_of::<SsaCallee>())?;
    for (index, callee) in lowered.callees.iter().enumerate() {
        if used[index] {
            callee_shape = meter::add(
                callee_shape,
                meter::add(
                    callee.symbol.len(),
                    meter::mul(callee.signature.params.len(), size_of::<super::SsaTy>())?,
                )?,
            )?;
        }
    }
    let preflight = [
        meter::mul(caller.params.len(), size_of::<super::SsaTy>()),
        meter::mul(x.blocks, size_of::<BlockData>()),
        meter::mul(x.params, size_of::<(SsaValue, super::SsaTy)>()),
        meter::mul(x.insts, size_of::<Inst>()),
        meter::mul(meter::add(x.call_args, x.edge_args)?, size_of::<SsaValue>()),
        meter::mul(x.panics, size_of::<SsaPanicText>()),
        Ok(x.panic_bytes),
        Ok(callee_shape),
    ]
    .into_iter()
    .try_fold(0usize, |total, part| meter::add(total, part?))?;
    meter.work(meter::add(x.insts, x.blocks)?)?;

    Owned::build(
        meter,
        preflight,
        || build(lowered, &body_of, &used, &x, meter),
        store::lowered_capacity_bytes,
    )
    .and_then(|owned| {
        owned
            .value
            .verify(meter)
            .map_err(|error| format!("inlining produced {error}"))?;
        Ok(owned)
    })
}

fn try_vec<T>(capacity: usize) -> Result<Vec<T>, String> {
    let mut data = Vec::new();
    data.try_reserve_exact(capacity)
        .map_err(|_| "optimizer allocation failed".to_string())?;
    Ok(data)
}

/// Copy one panic text, charging the bytes actually copied (message and
/// location) at this site. Storage was admitted in `x.panic_bytes`.
fn copy_panic(text: &SsaPanicText, meter: &Meter<'_>) -> Result<SsaPanicText, String> {
    meter.work(meter::add(text.message.len(), text.location.len())?)?;
    Ok(SsaPanicText {
        message: store::try_string(&text.message)?,
        location: store::try_string(&text.location)?,
    })
}

fn build(
    lowered: &Lowered,
    body_of: &[Option<&Lowered>],
    used: &[bool],
    x: &Expansion,
    meter: &Meter<'_>,
) -> Result<Lowered, String> {
    let caller = &lowered.function;
    let mut blocks: Vec<BlockData> = try_vec(x.blocks)?;
    // Build the panic table directly at its exact final capacity (paid in
    // the final-shape lease), so its header never reallocates and no old+new
    // overlap arises. Each text copy charges its actual bytes.
    let mut panics: Vec<SsaPanicText> = try_vec(x.panics)?;
    for text in &caller.panics {
        panics.push(copy_panic(text, meter)?);
    }
    let mut next_value = caller.value_bound() as u32;
    // Remap caller callee indices to the kept ones (paid scratch).
    let mut remap = meter.table::<u32>(used.len(), u32::MAX)?;
    let mut kept = 0u32;
    for (slot, flag) in remap.iter_mut().zip(used) {
        if *flag {
            *slot = kept;
            kept += 1;
        }
    }
    // Reserve the caller block slots first so appended blocks get stable IDs.
    for _ in &caller.blocks {
        blocks.push(BlockData {
            panic_location: None,
            params: Vec::new(),
            insts: Vec::new(),
            term: SsaTerminator::Panic(PanicKind::Unreachable),
        });
    }
    // Maximum callee value bound, for one reusable rename table.
    let rename_len = body_of
        .iter()
        .flatten()
        .map(|body| body.function.value_bound())
        .max()
        .unwrap_or(0);
    let mut rename = meter.table(rename_len, SsaValue(u32::MAX))?;
    let remap_kind = |kind: &InstKind| -> Result<InstKind, String> {
        Ok(match kind {
            InstKind::Call { callee, args } => {
                let mut copy = try_vec(args.len())?;
                copy.extend_from_slice(args);
                InstKind::Call {
                    callee: FuncRef(remap[callee.0 as usize]),
                    args: copy,
                }
            }
            other => other.clone(),
        })
    };
    for (index, data) in caller.blocks.iter().enumerate() {
        // Segment the block at each inlined call. `slot` is the block being
        // filled: the caller's own slot first, then each continuation.
        let mut slot = index;
        let mut params = try_vec(data.params.len())?;
        params.extend_from_slice(&data.params);
        let mut start = 0usize;
        loop {
            let split = data.insts[start..]
                .iter()
                .position(|inst| matches!(inst.kind, InstKind::Call { callee, .. } if body_of[callee.0 as usize].is_some()))
                .map(|offset| start + offset);
            let end = split.unwrap_or(data.insts.len());
            let mut insts = try_vec(end - start)?;
            for inst in &data.insts[start..end] {
                insts.push(Inst {
                    runtime_location: inst.runtime_location.clone(),
                    result: inst.result,
                    kind: remap_kind(&inst.kind)?,
                });
            }
            let Some(at) = split else {
                blocks[slot] = BlockData {
                    panic_location: data.panic_location.clone(),
                    params,
                    insts,
                    term: store::try_clone_term(&data.term)?,
                };
                break;
            };
            let call = &data.insts[at];
            let InstKind::Call { callee, args } = &call.kind else {
                return Err("inline split lost its call".into());
            };
            let body = body_of[callee.0 as usize].ok_or("inline split lost its body")?;
            let callee_fn = &body.function;
            let body_base = blocks.len() as u32;
            let continuation = body_base + callee_fn.blocks.len() as u32;
            // Rename callee values into fresh IDs.
            for block in &callee_fn.blocks {
                for value in block
                    .params
                    .iter()
                    .map(|(v, _)| *v)
                    .chain(block.insts.iter().map(|i| i.result))
                {
                    rename[value.0 as usize] = SsaValue(next_value);
                    next_value += 1;
                }
            }
            let panic_base = panics.len() as u32;
            for text in &callee_fn.panics {
                if panics.len() == panics.capacity() {
                    return Err("inlined panic table exceeded its admitted bound".into());
                }
                panics.push(copy_panic(text, meter)?);
            }
            let mut entry_args = try_vec(args.len())?;
            entry_args.extend_from_slice(args);
            blocks[slot] = BlockData {
                panic_location: None,
                params,
                insts,
                term: SsaTerminator::Jump(SsaTarget {
                    block: SsaBlock(body_base),
                    args: entry_args,
                }),
            };
            for block in &callee_fn.blocks {
                meter.work(meter::add(block.insts.len(), block.term.operand_count())?)?;
                let mut block_params = try_vec(block.params.len())?;
                for (value, ty) in &block.params {
                    block_params.push((rename[value.0 as usize], *ty));
                }
                let mut block_insts = try_vec(block.insts.len())?;
                for inst in &block.insts {
                    let mut kind = inst.kind.clone();
                    for operand in kind.operands_mut() {
                        *operand = rename[operand.0 as usize];
                    }
                    block_insts.push(Inst {
                        runtime_location: inst.runtime_location.clone(),
                        result: rename[inst.result.0 as usize],
                        kind,
                    });
                }
                let term = match &block.term {
                    SsaTerminator::Return(value) => {
                        let mut args = try_vec(1)?;
                        args.push(rename[value.0 as usize]);
                        SsaTerminator::Jump(SsaTarget {
                            block: SsaBlock(continuation),
                            args,
                        })
                    }
                    SsaTerminator::Panic(PanicKind::Message(message)) => {
                        SsaTerminator::Panic(PanicKind::Message(panic_base + message))
                    }
                    other => {
                        let mut term = store::try_clone_term(other)?;
                        for target in term.targets_mut() {
                            target.block = SsaBlock(body_base + target.block.0);
                        }
                        for operand in term.operands_mut() {
                            *operand = rename[operand.0 as usize];
                        }
                        term
                    }
                };
                if blocks.len() == blocks.capacity() {
                    return Err("inlining exceeded its admitted shape".into());
                }
                blocks.push(BlockData {
                    panic_location: block.panic_location.clone(),
                    params: block_params,
                    insts: block_insts,
                    term,
                });
            }
            // The continuation receives the call result.
            if blocks.len() == blocks.capacity() {
                return Err("inlining exceeded its admitted shape".into());
            }
            blocks.push(BlockData {
                panic_location: None,
                params: Vec::new(),
                insts: Vec::new(),
                term: SsaTerminator::Panic(PanicKind::Unreachable),
            });
            slot = continuation as usize;
            params = try_vec(1)?;
            params.push((call.result, callee_fn.ret));
            start = at + 1;
        }
    }
    let mut out_callees = try_vec(kept as usize)?;
    meter.work(lowered.callees.len())?;
    for (index, callee) in lowered.callees.iter().enumerate() {
        if used[index] {
            // Copy work: the symbol bytes and parameter entries actually
            // copied (their storage is in the byte preflight).
            meter.work(meter::add(
                callee.symbol.len(),
                callee.signature.params.len(),
            )?)?;
            out_callees.push(SsaCallee {
                symbol: store::try_string(&callee.symbol)?,
                target: callee.target.clone(),
                signature: super::SsaSignature {
                    params: {
                        let mut p = try_vec(callee.signature.params.len())?;
                        p.extend_from_slice(&callee.signature.params);
                        p
                    },
                    ret: callee.signature.ret,
                },
            });
        }
    }
    let mut params = try_vec(caller.params.len())?;
    params.extend_from_slice(&caller.params);
    Ok(Lowered {
        function: SsaFunction {
            params,
            ret: caller.ret,
            blocks,
            panics,
        },
        callees: out_callees,
    })
}
