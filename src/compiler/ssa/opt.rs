//! Intraprocedural passes for `LOCAL_OPT`: constant folding, algebraic
//! identities, branch folding, unreachable-block removal and dead-code
//! elimination. Each reads only the function itself (plus callee signatures
//! for verification), so it is parallel and cacheable per function.
//!
//! Folding evaluates through `eval`, the same code the interpreter runs.
//! Behavior is preserved exactly, including panics: an instruction that must
//! panic becomes a `Panic` terminator, never a silently wrapped value.
use super::eval;
use super::hash::canonical;
use super::verify::{self, Types, VerifyError};
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

/// Run the local pipeline to a fixpoint and return the canonical result.
/// Input and output both satisfy `verify`. Deterministic: equal canonical
/// inputs give equal outputs.
pub fn local(
    function: &SsaFunction,
    callee: &dyn Fn(FuncRef) -> Option<SsaSignature>,
) -> Result<(SsaFunction, Stats), VerifyError> {
    let mut current = canonical(function);
    let mut stats = Stats::default();
    // Each round either shrinks the function or stops, so this terminates; the
    // bound is a defensive per-function limit, not a global budget.
    for _ in 0..=current.inst_count() + current.blocks.len() + 1 {
        let types = verify::verify(&current, callee)?;
        let before = stats;
        let mut next = fold(&current, &types, &mut stats);
        next = canonical(&next);
        next = merge(&next, &mut stats);
        next = canonical(&next);
        next = block_params(&next, &mut stats);
        next = canonical(&next);
        let types = verify::verify(&next, callee)?;
        next = dce(&next, &types, &mut stats);
        next = canonical(&next);
        let done = stats == before;
        current = next;
        if done {
            break;
        }
    }
    verify::verify(&current, callee)?;
    Ok((current, stats))
}

fn ty(types: &Types, value: SsaValue) -> SsaTy {
    types[value.0 as usize].expect("verified value has a type")
}

/// Constant folding, identities and branch folding in one RPO sweep.
fn fold(function: &SsaFunction, types: &Types, stats: &mut Stats) -> SsaFunction {
    let mut out = function.clone();
    let mut known: Vec<Option<u128>> = vec![None; function.value_bound()];
    let mut subst: Vec<Option<SsaValue>> = vec![None; function.value_bound()];
    let resolve = |subst: &[Option<SsaValue>], mut value: SsaValue| {
        while let Some(next) = subst[value.0 as usize] {
            value = next;
        }
        value
    };
    for block in function.reverse_postorder() {
        let data = &mut out.blocks[block.0 as usize];
        let mut kept = Vec::with_capacity(data.insts.len());
        let mut panicked = None;
        for inst in &data.insts {
            let mut kind = inst.kind.clone();
            for operand in kind.operands_mut() {
                *operand = resolve(&subst, *operand);
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
            match folded {
                Some(Ok(bits)) => {
                    known[inst.result.0 as usize] = Some(bits);
                    stats.folded += 1;
                    kept.push(Inst {
                        result: inst.result,
                        kind: InstKind::Const {
                            ty: ty(types, inst.result),
                            bits,
                        },
                    });
                }
                Some(Err(kind)) => {
                    stats.panics_folded += 1;
                    panicked = Some(kind);
                    break;
                }
                None => kept.push(Inst {
                    result: inst.result,
                    kind,
                }),
            }
        }
        data.insts = kept;
        if let Some(kind) = panicked {
            // Everything after a certain panic is unreachable. Blocks that
            // depended on the dropped values were dominated by this one and
            // are removed by canonicalization.
            data.term = SsaTerminator::Panic(kind);
            continue;
        }
        for operand in data.term.operands_mut() {
            *operand = resolve(&subst, *operand);
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
                data.term = SsaTerminator::Jump(taken.clone());
                stats.branches_folded += 1;
            }
        }
    }
    out
}

/// Merge a block into its predecessor when the predecessor ends in a jump to
/// it and is its only predecessor edge. The merged block's parameters are
/// replaced by the jump arguments. Expects canonical input (all blocks
/// reachable).
fn merge(function: &SsaFunction, stats: &mut Stats) -> SsaFunction {
    let count = function.blocks.len();
    let mut edges = vec![0usize; count];
    for data in &function.blocks {
        for target in data.term.targets() {
            edges[target.block.0 as usize] += 1;
        }
    }
    let mut out = function.clone();
    let mut subst: Vec<Option<SsaValue>> = vec![None; function.value_bound()];
    let resolve = |subst: &[Option<SsaValue>], mut value: SsaValue| {
        while let Some(next) = subst[value.0 as usize] {
            value = next;
        }
        value
    };
    let mut absorbed = vec![false; count];
    // Blocks are in RPO, so a chain a -> b -> c collapses in one sweep.
    for block in 0..count {
        if absorbed[block] {
            continue;
        }
        while let SsaTerminator::Jump(next) = &out.blocks[block].term {
            let successor = next.block.0 as usize;
            if successor == block || edges[successor] != 1 || absorbed[successor] {
                break;
            }
            let args = next.args.clone();
            let taken = std::mem::replace(
                &mut out.blocks[successor],
                super::BlockData {
                    params: Vec::new(),
                    insts: Vec::new(),
                    term: SsaTerminator::Panic(super::PanicKind::Unreachable),
                },
            );
            absorbed[successor] = true;
            for ((param, _), arg) in taken.params.iter().zip(args) {
                subst[param.0 as usize] = Some(arg);
            }
            let data = &mut out.blocks[block];
            data.insts.extend(taken.insts);
            data.term = taken.term;
            stats.merged += 1;
        }
    }
    for data in &mut out.blocks {
        for inst in &mut data.insts {
            for operand in inst.kind.operands_mut() {
                *operand = resolve(&subst, *operand);
            }
        }
        for operand in data.term.operands_mut() {
            *operand = resolve(&subst, *operand);
        }
    }
    // Absorbed blocks are now unreachable and dropped by canonicalization.
    out
}

/// Remove block parameters that are trivial (every incoming argument is the
/// parameter itself or one value whose definition strictly dominates the
/// block) or unused. Lowering from mutable locals passes every local through
/// every block; this pass turns that into minimal SSA. Expects canonical
/// input (all blocks reachable, entry is block 0).
fn block_params(function: &SsaFunction, stats: &mut Stats) -> SsaFunction {
    let count = function.blocks.len();
    let idom = verify::dominators(function);
    let bound = function.value_bound();
    let mut def_block = vec![usize::MAX; bound];
    for (block, data) in function.blocks.iter().enumerate() {
        for (value, _) in &data.params {
            def_block[value.0 as usize] = block;
        }
        for inst in &data.insts {
            def_block[inst.result.0 as usize] = block;
        }
    }
    let strictly_dominates = |a: usize, mut b: usize| {
        while let Some(parent) = idom[b] {
            if parent == b {
                return false;
            }
            if parent == a {
                return true;
            }
            b = parent;
        }
        false
    };
    // Incoming arguments per (block, param index).
    let mut incoming: Vec<Vec<Vec<SsaValue>>> = function
        .blocks
        .iter()
        .map(|data| vec![Vec::new(); data.params.len()])
        .collect();
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
        for target in data.term.targets() {
            for (index, arg) in target.args.iter().enumerate() {
                incoming[target.block.0 as usize][index].push(*arg);
            }
        }
    }
    let mut subst: Vec<Option<SsaValue>> = vec![None; bound];
    let mut drop: Vec<Vec<bool>> = function
        .blocks
        .iter()
        .map(|data| vec![false; data.params.len()])
        .collect();
    for block in 1..count {
        for (index, (param, _)) in function.blocks[block].params.iter().enumerate() {
            let mut single = None;
            let mut trivial = true;
            for arg in &incoming[block][index] {
                if arg == param || single == Some(*arg) {
                    continue;
                }
                if single.is_some() {
                    trivial = false;
                    break;
                }
                single = Some(*arg);
            }
            // Self-passing args are uses of the param that disappear with it.
            let self_uses = incoming[block][index]
                .iter()
                .filter(|arg| *arg == param)
                .count();
            let unused = uses[param.0 as usize] == self_uses;
            if trivial
                && let Some(value) = single
                && def_block[value.0 as usize] != usize::MAX
                && strictly_dominates(def_block[value.0 as usize], block)
            {
                subst[param.0 as usize] = Some(value);
                drop[block][index] = true;
            } else if unused {
                drop[block][index] = true;
            }
        }
    }
    let resolve = |mut value: SsaValue| {
        while let Some(next) = subst[value.0 as usize] {
            value = next;
        }
        value
    };
    let mut out = function.clone();
    for (block, data) in out.blocks.iter_mut().enumerate() {
        let mut index = 0;
        data.params.retain(|_| {
            let keep = !drop[block][index];
            index += 1;
            keep
        });
        for inst in &mut data.insts {
            for operand in inst.kind.operands_mut() {
                *operand = resolve(*operand);
            }
        }
        for target in data.term.targets_mut() {
            let flags = &drop[target.block.0 as usize];
            let mut index = 0;
            target.args.retain(|_| {
                let keep = !flags[index];
                index += 1;
                keep
            });
        }
        for operand in data.term.operands_mut() {
            *operand = resolve(*operand);
        }
    }
    stats.params_removed += drop.iter().flatten().filter(|dropped| **dropped).count();
    out
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

/// Mark-and-sweep dead-code elimination over instructions.
fn dce(function: &SsaFunction, types: &Types, stats: &mut Stats) -> SsaFunction {
    let bound = function.value_bound();
    let mut known: Vec<Option<u128>> = vec![None; bound];
    for data in &function.blocks {
        for inst in &data.insts {
            if let InstKind::Const { bits, .. } = inst.kind {
                known[inst.result.0 as usize] = Some(bits);
            }
        }
    }
    let mut live = vec![false; bound];
    let mut work: Vec<SsaValue> = Vec::new();
    let mut defining: Vec<Option<(usize, usize)>> = vec![None; bound];
    for (block, data) in function.blocks.iter().enumerate() {
        work.extend(data.term.operands());
        for (index, inst) in data.insts.iter().enumerate() {
            defining[inst.result.0 as usize] = Some((block, index));
            if !removable(&inst.kind, types, &known) {
                work.push(inst.result);
            }
        }
    }
    while let Some(value) = work.pop() {
        if std::mem::replace(&mut live[value.0 as usize], true) {
            continue;
        }
        if let Some((block, index)) = defining[value.0 as usize] {
            work.extend(function.blocks[block].insts[index].kind.operands());
        }
    }
    let mut out = function.clone();
    for data in &mut out.blocks {
        let before = data.insts.len();
        data.insts.retain(|inst| live[inst.result.0 as usize]);
        stats.removed += before - data.insts.len();
    }
    out
}
