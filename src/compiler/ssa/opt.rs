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
    BinOp, FuncRef, Function, Inst, InstKind, Overflow, Signature, Terminator, Ty, UnOp, Value,
};

/// What a pass run changed. Feeds optimizer telemetry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub folded: usize,
    pub simplified: usize,
    pub branches_folded: usize,
    pub panics_folded: usize,
    pub merged: usize,
    pub removed: usize,
}

/// Run the local pipeline to a fixpoint and return the canonical result.
/// Input and output both satisfy `verify`. Deterministic: equal canonical
/// inputs give equal outputs.
pub fn local(
    function: &Function,
    callee: &dyn Fn(FuncRef) -> Option<Signature>,
) -> Result<(Function, Stats), VerifyError> {
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

fn ty(types: &Types, value: Value) -> Ty {
    types[value.0 as usize].expect("verified value has a type")
}

/// Constant folding, identities and branch folding in one RPO sweep.
fn fold(function: &Function, types: &Types, stats: &mut Stats) -> Function {
    let mut out = function.clone();
    let mut known: Vec<Option<u128>> = vec![None; function.value_bound()];
    let mut subst: Vec<Option<Value>> = vec![None; function.value_bound()];
    let resolve = |subst: &[Option<Value>], mut value: Value| {
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
            let k = |value: Value| known[value.0 as usize];
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
                    if ty(types, *value) == Ty::Int(*to) {
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
            data.term = Terminator::Panic(kind);
            continue;
        }
        for operand in data.term.operands_mut() {
            *operand = resolve(&subst, *operand);
        }
        if let Terminator::Branch {
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
                data.term = Terminator::Jump(taken.clone());
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
fn merge(function: &Function, stats: &mut Stats) -> Function {
    let count = function.blocks.len();
    let mut edges = vec![0usize; count];
    for data in &function.blocks {
        for target in data.term.targets() {
            edges[target.block.0 as usize] += 1;
        }
    }
    let mut out = function.clone();
    let mut subst: Vec<Option<Value>> = vec![None; function.value_bound()];
    let resolve = |subst: &[Option<Value>], mut value: Value| {
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
        while let Terminator::Jump(next) = &out.blocks[block].term {
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
                    term: Terminator::Panic(super::PanicKind::Unreachable),
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

/// `x op c` that equals `x` for every `x`, in both overflow modes.
fn identity(op: BinOp, lhs: Value, rhs: Option<u128>, lhs_ty: Ty) -> Option<Value> {
    let c = rhs?;
    let all = lhs_ty.max_bits();
    let same = match op {
        BinOp::Add | BinOp::Sub | BinOp::Or | BinOp::Xor | BinOp::Shl | BinOp::Shr => c == 0,
        BinOp::Mul | BinOp::Div => c == 1 && matches!(lhs_ty, Ty::Int(_)),
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
fn dce(function: &Function, types: &Types, stats: &mut Stats) -> Function {
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
    let mut work: Vec<Value> = Vec::new();
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
