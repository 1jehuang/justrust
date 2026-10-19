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
use super::lower::Lowered;
use super::{
    BlockData, FuncRef, Inst, InstKind, PanicKind, SsaBlock, SsaTarget, SsaTerminator, SsaValue,
};

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
    pub fn of(lowered: &Lowered) -> Self {
        let function = &lowered.function;
        let mut leaf = true;
        let mut may_panic = false;
        for data in &function.blocks {
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
        Summary {
            body_hash: lowered.hash(),
            insts: function.inst_count(),
            blocks: function.blocks.len(),
            leaf,
            may_panic,
        }
    }

    pub fn inlinable(&self) -> bool {
        self.leaf && self.insts <= INLINE_LIMIT
    }
}

/// Callees (by index into `lowered.callees`) worth inlining, given each
/// callee's summary. Deterministic in the inputs.
pub fn candidates(lowered: &Lowered, summary: &dyn Fn(usize) -> Option<Summary>) -> Vec<usize> {
    let mut chosen = Vec::new();
    for index in 0..lowered.callees.len() {
        if summary(index).is_some_and(|s| s.inlinable()) {
            chosen.push(index);
        }
    }
    chosen
}

/// Inline every call to a callee present in `bodies` (index into
/// `lowered.callees` -> optimized callee). Bodies must be leaves.
pub fn inline(lowered: &Lowered, bodies: &[(usize, &Lowered)]) -> Result<Lowered, String> {
    let mut function = lowered.function.clone();
    let mut panics = function.panics.clone();
    let mut next_value = function.value_bound() as u32;
    let mut fresh = || {
        let value = SsaValue(next_value);
        next_value += 1;
        value
    };
    let mut block_index = 0usize;
    while block_index < function.blocks.len() {
        let Some(position) = function.blocks[block_index].insts.iter().position(|inst| {
            matches!(&inst.kind, InstKind::Call { callee, .. }
                if bodies.iter().any(|(index, _)| *index == callee.0 as usize))
        }) else {
            block_index += 1;
            continue;
        };
        let call = function.blocks[block_index].insts[position].clone();
        let InstKind::Call { callee, args } = &call.kind else {
            unreachable!()
        };
        let body = bodies
            .iter()
            .find(|(index, _)| *index == callee.0 as usize)
            .map(|(_, body)| *body)
            .ok_or("missing inline body")?;
        if !body.callees.is_empty() {
            return Err("only leaf callees are inlined".into());
        }
        let callee_fn = &body.function;
        // Split: the continuation block receives the call result.
        let ret_ty = callee_fn.ret;
        let result = call.result;
        let tail_insts = function.blocks[block_index].insts.split_off(position + 1);
        function.blocks[block_index].insts.pop();
        let continuation = SsaBlock(function.blocks.len() as u32);
        let old_term = std::mem::replace(
            &mut function.blocks[block_index].term,
            SsaTerminator::Panic(PanicKind::Unreachable),
        );
        function.blocks.push(BlockData {
            params: vec![(result, ret_ty)],
            insts: tail_insts,
            term: old_term,
        });
        // Copy callee blocks with renamed values and blocks.
        let base = function.blocks.len() as u32;
        let mut map = vec![SsaValue(u32::MAX); callee_fn.value_bound()];
        for data in &callee_fn.blocks {
            for (value, _) in &data.params {
                map[value.0 as usize] = fresh();
            }
            for inst in &data.insts {
                map[inst.result.0 as usize] = fresh();
            }
        }
        let rename = |value: SsaValue| map[value.0 as usize];
        let panic_base = panics.len() as u32;
        panics.extend(callee_fn.panics.iter().cloned());
        for data in &callee_fn.blocks {
            let insts = data
                .insts
                .iter()
                .map(|inst| {
                    let mut kind = inst.kind.clone();
                    for operand in kind.operands_mut() {
                        *operand = rename(*operand);
                    }
                    Inst {
                        result: rename(inst.result),
                        kind,
                    }
                })
                .collect();
            let term = match &data.term {
                SsaTerminator::Return(value) => SsaTerminator::Jump(SsaTarget {
                    block: continuation,
                    args: vec![rename(*value)],
                }),
                SsaTerminator::Panic(PanicKind::Message(index)) => {
                    SsaTerminator::Panic(PanicKind::Message(panic_base + index))
                }
                other => {
                    let mut term = other.clone();
                    for target in term.targets_mut() {
                        target.block = SsaBlock(base + target.block.0);
                    }
                    for operand in term.operands_mut() {
                        *operand = rename(*operand);
                    }
                    term
                }
            };
            function.blocks.push(BlockData {
                params: data
                    .params
                    .iter()
                    .map(|(value, ty)| (rename(*value), *ty))
                    .collect(),
                insts,
                term,
            });
        }
        function.blocks[block_index].term = SsaTerminator::Jump(SsaTarget {
            block: SsaBlock(base),
            args: args.clone(),
        });
        // Continue scanning the same block: it may hold no more calls, but
        // the continuation will be visited in order.
        block_index += 1;
    }
    function.panics = panics;
    // Drop callees that are no longer referenced, renumbering FuncRefs.
    let mut used = vec![false; lowered.callees.len()];
    for data in &function.blocks {
        for inst in &data.insts {
            if let InstKind::Call { callee, .. } = inst.kind {
                used[callee.0 as usize] = true;
            }
        }
    }
    let mut remap = vec![u32::MAX; lowered.callees.len()];
    let mut callees = Vec::new();
    for (index, callee) in lowered.callees.iter().enumerate() {
        if used[index] {
            remap[index] = callees.len() as u32;
            callees.push(callee.clone());
        }
    }
    for data in &mut function.blocks {
        for inst in &mut data.insts {
            if let InstKind::Call { callee, .. } = &mut inst.kind {
                *callee = FuncRef(remap[callee.0 as usize]);
            }
        }
    }
    let out = Lowered { function, callees };
    out.verify()
        .map_err(|error| format!("inlining produced {error}"))?;
    Ok(out)
}
