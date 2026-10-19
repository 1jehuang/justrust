//! Intraprocedural passes for `LOCAL_OPT`: constant folding, algebraic
//! identities, branch folding, unreachable-block removal and dead-code
//! elimination. Each reads only the function itself (plus callee signatures
//! for verification), so it is parallel and cacheable per function.
//!
//! Folding evaluates through `eval`, the same code the interpreter runs.
//! Behavior is preserved exactly, including panics: an instruction that must
//! panic becomes a `Panic` terminator, never a silently wrapped value.
use super::eval;
use super::hash::canonical_metered;
use super::meter::{self, Meter};
use super::store::{self, Owned};
use super::verify::{self, Signatures, Types, VerifyError};
use super::{
    BinOp, FuncRef, Inst, InstKind, Overflow, SsaFunction, SsaSignature, SsaTerminator, SsaTy,
    SsaValue, UnOp,
};

/// What a pass run changed. Feeds optimizer telemetry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub folded: usize,
    pub simplified: usize,
    pub branches_folded: usize,
    pub panics_folded: usize,
    pub merged: usize,
    pub params_removed: usize,
    pub removed: usize,
}

/// Unmetered local pipeline, for tests and the interpreter.
pub fn local(
    function: &SsaFunction,
    callee: &dyn Fn(FuncRef) -> Option<SsaSignature>,
) -> Result<(SsaFunction, Stats), VerifyError> {
    let budget = super::meter::unlimited();
    let meter = Meter::new(&budget);
    let mut owned: Vec<(FuncRef, SsaSignature)> = Vec::new();
    for data in &function.blocks {
        for inst in &data.insts {
            if let InstKind::Call { callee: target, .. } = inst.kind
                && !owned.iter().any(|(f, _)| *f == target)
                && let Some(signature) = callee(target)
            {
                owned.push((target, signature));
            }
        }
    }
    let lookup = |target: FuncRef| owned.iter().find(|(f, _)| *f == target).map(|(_, s)| s);
    let (out, stats) = local_metered(function, &lookup, &meter)?;
    Ok((out.value, stats))
}

/// Run the local pipeline to a fixpoint and return the canonical result.
/// Input and output both satisfy `verify`. Deterministic: equal canonical
/// inputs give equal outputs. Every round, its canonicalizations, verifies
/// and passes are metered; at most two generations of the function are live
/// at once and each is paid by its own lease until dropped.
pub fn local_metered<'a>(
    function: &SsaFunction,
    callee: &Signatures<'a>,
    meter: &Meter<'_>,
) -> Result<(Owned<SsaFunction>, Stats), VerifyError> {
    let paid = |r: Result<Owned<SsaFunction>, String>| r.map_err(VerifyError::resource);
    let mut current = paid(canonical_metered(function, meter))?;
    let mut stats = Stats::default();
    // Each round either shrinks the function or stops, so this terminates; the
    // bound is a defensive per-function limit, not a global budget.
    let rounds = current.value.inst_count() + current.value.blocks.len() + 1;
    for _ in 0..=rounds {
        meter.work(1).map_err(VerifyError::resource)?;
        let before = stats;
        let types = verify::verify_metered(&current.value, callee, meter)?;
        let folded = paid(fold(&current.value, &types, &mut stats, meter))?;
        drop(types);
        let folded = paid(canonical_metered(&folded.value, meter))?;
        let merged = paid(merge(&folded.value, &mut stats, meter))?;
        drop(folded);
        let merged = paid(canonical_metered(&merged.value, meter))?;
        let trimmed = paid(block_params(&merged.value, &mut stats, meter))?;
        drop(merged);
        let trimmed = paid(canonical_metered(&trimmed.value, meter))?;
        let types = verify::verify_metered(&trimmed.value, callee, meter)?;
        let swept = paid(dce(&trimmed.value, &types, &mut stats, meter))?;
        drop(types);
        drop(trimmed);
        let next = paid(canonical_metered(&swept.value, meter))?;
        drop(swept);
        let done = stats == before;
        current = next;
        if done {
            break;
        }
    }
    drop(verify::verify_metered(&current.value, callee, meter)?);
    Ok((current, stats))
}

/// Follow a substitution chain, charging each hop.
fn resolve(
    subst: &[Option<SsaValue>],
    mut value: SsaValue,
    meter: &Meter<'_>,
) -> Result<SsaValue, String> {
    while let Some(next) = subst[value.0 as usize] {
        meter.work(1)?;
        value = next;
    }
    Ok(value)
}

fn ty(types: &Types, value: SsaValue) -> SsaTy {
    types[value.0 as usize].expect("verified value has a type")
}

/// Constant folding, identities and branch folding in one RPO sweep.
fn fold(
    function: &SsaFunction,
    types: &Types,
    stats: &mut Stats,
    meter: &Meter<'_>,
) -> Result<Owned<SsaFunction>, String> {
    let mut out = store::copy_function(meter, function)?;
    let mut known = meter.table::<Option<u128>>(function.value_bound(), None)?;
    let mut subst = meter.table::<Option<SsaValue>>(function.value_bound(), None)?;
    let order = function.reverse_postorder_metered(meter)?;
    for block in order.iter() {
        let data = &mut out.value.blocks[block.0 as usize];
        meter.work(meter::add(data.insts.len(), data.term.operand_count())?)?;
        // Folding only replaces or drops instructions, so the block's own
        // paid capacity is reused in place: no new allocation.
        let insts = std::mem::take(&mut data.insts);
        let mut kept = insts;
        let mut write = 0usize;
        let mut panicked = None;
        let placeholder = || Inst {
            result: SsaValue(0),
            kind: InstKind::Const {
                ty: SsaTy::Unit,
                bits: 0,
            },
        };
        for read in 0..kept.len() {
            // Move the instruction out (no allocation); its slot is reused.
            let inst = std::mem::replace(&mut kept[read], placeholder());
            meter.work(inst.kind.operand_count())?;
            let mut kind = inst.kind;
            for operand in kind.operands_mut() {
                *operand = resolve(&subst, *operand, meter)?;
            }
            let k = |value: SsaValue| known[value.0 as usize];
            let folded: Option<Result<u128, super::PanicKind>> = match &kind {
                InstKind::Const { bits, .. } => {
                    known[inst.result.0 as usize] = Some(*bits);
                    None
                }
                InstKind::Binary {
                    op,
                    overflow,
                    lhs,
                    rhs,
                } => match (k(*lhs), k(*rhs)) {
                    (Some(a), Some(b)) => Some(eval::binary(
                        *op,
                        *overflow,
                        ty(types, *lhs),
                        a,
                        ty(types, *rhs),
                        b,
                    )),
                    (_, rhs_const) => {
                        if let Some(same) = identity(*op, *lhs, rhs_const, ty(types, *lhs)) {
                            subst[inst.result.0 as usize] = Some(same);
                            stats.simplified += 1;
                            continue;
                        }
                        None
                    }
                },
                InstKind::Unary {
                    op,
                    overflow,
                    value,
                } => k(*value).map(|a| eval::unary(*op, *overflow, ty(types, *value), a)),
                InstKind::Cmp { op, lhs, rhs } => match (k(*lhs), k(*rhs)) {
                    (Some(a), Some(b)) => {
                        Some(Ok(u128::from(eval::compare(*op, ty(types, *lhs), a, b))))
                    }
                    _ => None,
                },
                InstKind::Cast { to, value } => {
                    if ty(types, *value) == SsaTy::Int(*to) {
                        subst[inst.result.0 as usize] = Some(*value);
                        stats.simplified += 1;
                        continue;
                    }
                    k(*value).map(|a| Ok(eval::cast(ty(types, *value), *to, a)))
                }
                InstKind::Select {
                    cond,
                    then,
                    otherwise,
                } => {
                    let chosen = match k(*cond) {
                        Some(c) => Some(if c != 0 { *then } else { *otherwise }),
                        None if then == otherwise => Some(*then),
                        None => None,
                    };
                    if let Some(chosen) = chosen {
                        subst[inst.result.0 as usize] = Some(chosen);
                        stats.simplified += 1;
                        continue;
                    }
                    None
                }
                InstKind::Call { .. } => None,
            };
            let replacement = match folded {
                Some(Ok(bits)) => {
                    known[inst.result.0 as usize] = Some(bits);
                    stats.folded += 1;
                    Inst {
                        result: inst.result,
                        kind: InstKind::Const {
                            ty: ty(types, inst.result),
                            bits,
                        },
                    }
                }
                Some(Err(kind)) => {
                    stats.panics_folded += 1;
                    panicked = Some(kind);
                    break;
                }
                None => Inst {
                    result: inst.result,
                    kind,
                },
            };
            kept[write] = replacement;
            write += 1;
        }
        kept.truncate(write);
        data.insts = kept;
        if let Some(kind) = panicked {
            // Everything after a certain panic is unreachable. Blocks that
            // depended on the dropped values were dominated by this one and
            // are removed by canonicalization.
            data.term = SsaTerminator::Panic(kind);
            continue;
        }
        for operand in data.term.operands_mut() {
            *operand = resolve(&subst, *operand, meter)?;
        }
        if let SsaTerminator::Branch {
            cond,
            then,
            otherwise,
        } = &data.term
        {
            let taken = match known[cond.0 as usize] {
                Some(c) => Some(if c != 0 { then } else { otherwise }),
                None if then == otherwise => Some(then),
                None => None,
            };
            if let Some(taken) = taken {
                data.term = SsaTerminator::Jump(super::store::try_clone_target(taken)?);
                stats.branches_folded += 1;
            }
        }
    }
    Ok(out)
}

/// Merge a block into its predecessor when the predecessor ends in a jump to
/// it and is its only predecessor edge. The merged block's parameters are
/// replaced by the jump arguments. Expects canonical input (all blocks
/// reachable).
fn merge(
    function: &SsaFunction,
    stats: &mut Stats,
    meter: &Meter<'_>,
) -> Result<Owned<SsaFunction>, String> {
    let count = function.blocks.len();
    let mut edges = meter.table(count, 0usize)?;
    for data in &function.blocks {
        meter.work(data.term.target_count())?;
        for target in data.term.targets() {
            edges[target.block.0 as usize] += 1;
        }
    }
    let mut out = store::copy_function(meter, function)?;
    let mut subst = meter.table::<Option<SsaValue>>(function.value_bound(), None)?;
    let mut absorbed = meter.table(count, false)?;
    // Blocks are in RPO, so a chain a -> b -> c collapses in one sweep.
    for block in 0..count {
        meter.work(1)?;
        if absorbed[block] {
            continue;
        }
        while let SsaTerminator::Jump(next) = &out.value.blocks[block].term {
            let successor = next.block.0 as usize;
            if successor == block || edges[successor] != 1 || absorbed[successor] {
                break;
            }
            let taken = std::mem::replace(
                &mut out.value.blocks[successor],
                super::BlockData {
                    params: Vec::new(),
                    insts: Vec::new(),
                    term: SsaTerminator::Panic(super::PanicKind::Unreachable),
                },
            );
            absorbed[successor] = true;
            let SsaTerminator::Jump(next) = &out.value.blocks[block].term else {
                return Err("merge lost its jump".into());
            };
            meter.work(meter::add(taken.params.len(), taken.insts.len())?)?;
            for ((param, _), arg) in taken.params.iter().zip(&next.args) {
                subst[param.0 as usize] = Some(*arg);
            }
            // Appending moves the absorbed instructions into the predecessor.
            // A grown buffer is new storage: admitted before reallocating and
            // kept charged (the old buffer stays in the receipt) so old and new
            // overlap conservatively.
            let data = &mut out.value.blocks[block];
            let needed = meter::add(data.insts.len(), taken.insts.len())?;
            if needed > data.insts.capacity() {
                out.admit(meter, meter::mul(needed, std::mem::size_of::<Inst>())?)?;
                let data = &mut out.value.blocks[block];
                data.insts
                    .try_reserve_exact(needed - data.insts.len())
                    .map_err(|_| "optimizer allocation failed".to_string())?;
            }
            let data = &mut out.value.blocks[block];
            data.insts.extend(taken.insts);
            data.term = taken.term;
            stats.merged += 1;
        }
    }
    for data in &mut out.value.blocks {
        meter.work(data.term.operand_count())?;
        for inst in &mut data.insts {
            meter.work(inst.kind.operand_count())?;
            for operand in inst.kind.operands_mut() {
                *operand = resolve(&subst, *operand, meter)?;
            }
        }
        for operand in data.term.operands_mut() {
            *operand = resolve(&subst, *operand, meter)?;
        }
    }
    // Absorbed blocks are now unreachable and dropped by canonicalization.
    Ok(out)
}

/// Remove block parameters that are trivial (every incoming argument is the
/// parameter itself or one value whose definition strictly dominates the
/// block) or unused. Lowering from mutable locals passes every local through
/// every block; this pass turns that into minimal SSA. Expects canonical
/// input (all blocks reachable, entry is block 0). Uses flat per-parameter
/// tables (one slot per block parameter) instead of nested vectors.
fn block_params(
    function: &SsaFunction,
    stats: &mut Stats,
    meter: &Meter<'_>,
) -> Result<Owned<SsaFunction>, String> {
    let count = function.blocks.len();
    let idom = verify::dominators_metered(function, meter)?;
    let bound = function.value_bound();
    let mut def_block = meter.table(bound, usize::MAX)?;
    for (block, data) in function.blocks.iter().enumerate() {
        meter.work(meter::add(data.params.len(), data.insts.len())?)?;
        for (value, _) in &data.params {
            def_block[value.0 as usize] = block;
        }
        for inst in &data.insts {
            def_block[inst.result.0 as usize] = block;
        }
    }
    let strictly_dominates = |a: usize, mut b: usize| -> Result<bool, String> {
        while let Some(parent) = idom[b] {
            meter.work(1)?;
            if parent == b {
                return Ok(false);
            }
            if parent == a {
                return Ok(true);
            }
            b = parent;
        }
        Ok(false)
    };
    // Block `b`'s parameters occupy slots base[b]..base[b + 1].
    let mut base = meter.table(count + 1, 0usize)?;
    for (block, data) in function.blocks.iter().enumerate() {
        base[block + 1] = meter::add(base[block], data.params.len())?;
    }
    let slots = base[count];
    let mut single = meter.table::<Option<SsaValue>>(slots, None)?;
    let mut conflict = meter.table(slots, false)?;
    let mut self_uses = meter.table(slots, 0usize)?;
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
        for target in data.term.targets() {
            let target_block = target.block.0 as usize;
            let params = &function.blocks[target_block].params;
            for (index, arg) in target.args.iter().enumerate() {
                let slot = base[target_block] + index;
                if *arg == params[index].0 {
                    self_uses[slot] += 1;
                } else {
                    match single[slot] {
                        None => single[slot] = Some(*arg),
                        Some(seen) if seen == *arg => {}
                        Some(_) => conflict[slot] = true,
                    }
                }
            }
        }
    }
    let mut subst = meter.table::<Option<SsaValue>>(bound, None)?;
    let mut drop = meter.table(slots, false)?;
    for block in 1..count {
        for (index, (param, _)) in function.blocks[block].params.iter().enumerate() {
            meter.work(1)?;
            let slot = base[block] + index;
            let unused = uses[param.0 as usize] == self_uses[slot];
            if !conflict[slot]
                && let Some(value) = single[slot]
                && def_block[value.0 as usize] != usize::MAX
                && strictly_dominates(def_block[value.0 as usize], block)?
            {
                subst[param.0 as usize] = Some(value);
                drop[slot] = true;
            } else if unused {
                drop[slot] = true;
            }
        }
    }
    let mut out = store::copy_function(meter, function)?;
    for (block, data) in out.value.blocks.iter_mut().enumerate() {
        meter.work(meter::add(data.params.len(), data.term.operand_count())?)?;
        let start = base[block];
        let mut index = 0;
        data.params.retain(|_| {
            let keep = !drop[start + index];
            index += 1;
            keep
        });
        for inst in &mut data.insts {
            meter.work(inst.kind.operand_count())?;
            for operand in inst.kind.operands_mut() {
                *operand = resolve(&subst, *operand, meter)?;
            }
        }
        for target in data.term.targets_mut() {
            let start = base[target.block.0 as usize];
            let mut index = 0;
            target.args.retain(|_| {
                let keep = !drop[start + index];
                index += 1;
                keep
            });
        }
        for operand in data.term.operands_mut() {
            *operand = resolve(&subst, *operand, meter)?;
        }
    }
    stats.params_removed += drop.iter().filter(|dropped| **dropped).count();
    Ok(out)
}

/// `x op c` that equals `x` for every `x`, in both overflow modes.
fn identity(op: BinOp, lhs: SsaValue, rhs: Option<u128>, lhs_ty: SsaTy) -> Option<SsaValue> {
    let c = rhs?;
    let all = lhs_ty.max_bits();
    let same = match op {
        BinOp::Add | BinOp::Sub | BinOp::Or | BinOp::Xor | BinOp::Shl | BinOp::Shr => c == 0,
        BinOp::Mul | BinOp::Div => c == 1 && matches!(lhs_ty, SsaTy::Int(_)),
        BinOp::And => c == all,
        BinOp::Rem => false,
    };
    same.then_some(lhs)
}

/// Whether an unused instruction can be deleted without changing behavior.
fn removable(kind: &InstKind, types: &Types, known: &[Option<u128>]) -> bool {
    match kind {
        InstKind::Const { .. }
        | InstKind::Cmp { .. }
        | InstKind::Cast { .. }
        | InstKind::Select { .. } => true,
        InstKind::Unary { op, overflow, .. } => *op == UnOp::Not || *overflow == Overflow::Wrap,
        InstKind::Binary {
            op,
            overflow,
            lhs,
            rhs,
        } => eval::binary_cannot_panic(
            *op,
            *overflow,
            ty(types, *lhs),
            ty(types, *rhs),
            known[rhs.0 as usize],
        ),
        // A call may panic or not terminate. Purity comes from callee
        // summaries in a later phase.
        InstKind::Call { .. } => false,
    }
}

/// Mark-and-sweep dead-code elimination over instructions. Each operand is
/// pushed at most once per use, so the worklist is admitted at exactly the
/// operand count plus one root per instruction.
fn dce(
    function: &SsaFunction,
    types: &Types,
    stats: &mut Stats,
    meter: &Meter<'_>,
) -> Result<Owned<SsaFunction>, String> {
    let bound = function.value_bound();
    let mut known = meter.table::<Option<u128>>(bound, None)?;
    let mut defining = meter.table::<Option<(usize, usize)>>(bound, None)?;
    let mut operands = 0usize;
    for (block, data) in function.blocks.iter().enumerate() {
        meter.work(meter::add(data.insts.len(), data.term.operand_count())?)?;
        operands = meter::add(operands, data.term.operand_count())?;
        for (index, inst) in data.insts.iter().enumerate() {
            operands = meter::add(operands, meter::add(inst.kind.operand_count(), 1)?)?;
            defining[inst.result.0 as usize] = Some((block, index));
            if let InstKind::Const { bits, .. } = inst.kind {
                known[inst.result.0 as usize] = Some(bits);
            }
        }
    }
    let mut live = meter.table(bound, false)?;
    let mut work = meter.list::<SsaValue>(operands)?;
    for data in &function.blocks {
        for operand in data.term.operands() {
            work.push(operand)?;
        }
        for inst in &data.insts {
            if !removable(&inst.kind, types, &known) {
                work.push(inst.result)?;
            }
        }
    }
    while let Some(value) = work.pop() {
        meter.work(1)?;
        if std::mem::replace(&mut live[value.0 as usize], true) {
            continue;
        }
        if let Some((block, index)) = defining[value.0 as usize] {
            let kind = &function.blocks[block].insts[index].kind;
            meter.work(kind.operand_count())?;
            for operand in kind.operands() {
                work.push(operand)?;
            }
        }
    }
    let mut out = store::copy_function(meter, function)?;
    for data in &mut out.value.blocks {
        meter.work(data.insts.len())?;
        let before = data.insts.len();
        data.insts.retain(|inst| live[inst.result.0 as usize]);
        stats.removed += before - data.insts.len();
    }
    Ok(out)
}
