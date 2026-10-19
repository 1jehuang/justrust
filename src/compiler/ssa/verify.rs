//! Structural, type and dominance checks. Every optimizer node must produce a
//! function that passes `verify`; tests run it after every pass.
use super::meter::{Meter, Table};
use super::{
    BinOp, CmpOp, FuncRef, InstKind, Overflow, SsaBlock, SsaFunction, SsaSignature, SsaTerminator,
    SsaTy, SsaValue, UnOp,
};
use std::fmt;

/// Invalid IR, or (`resource`) a budget/allocation failure while verifying.
/// Resource failures are never reported as invalid IR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError(pub String);

impl VerifyError {
    pub fn resource(error: String) -> Self {
        VerifyError(format!("resource: {error}"))
    }

    pub fn is_resource(&self) -> bool {
        self.0.starts_with("resource: ")
    }
}

fn paid<T>(result: Result<T, String>) -> Result<T, VerifyError> {
    result.map_err(VerifyError::resource)
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid SSA: {}", self.0)
    }
}

macro_rules! fail {
    ($($arg:tt)*) => {
        return Err(VerifyError(format!($($arg)*)))
    };
}

/// Where a value is defined: block and position (params are position 0,
/// instruction `i` is position `i + 1`).
#[derive(Clone, Copy)]
struct Def {
    block: usize,
    position: usize,
    ty: Option<SsaTy>,
}

/// Immediate dominators over the reachable blocks (Cooper, Harvey, Kennedy).
/// `None` for unreachable blocks. The entry is its own dominator.
/// Unmetered convenience for tests and the interpreter.
pub fn dominators(function: &SsaFunction) -> Vec<Option<usize>> {
    let budget = super::meter::unlimited();
    let meter = Meter::new(&budget);
    dominators_metered(function, &meter)
        .expect("unlimited dominators")
        .to_vec()
}

/// Metered dominators: RPO index, flattened predecessor (CSR) and idom
/// tables are admitted before allocation; every fixed-point round,
/// predecessor visit and intersection hop is charged.
pub fn dominators_metered(
    function: &SsaFunction,
    meter: &Meter<'_>,
) -> Result<Table<Option<usize>>, String> {
    let count = function.blocks.len();
    let order = function.reverse_postorder_metered(meter)?;
    let mut rpo_index = meter.table(count, usize::MAX)?;
    for (index, block) in order.iter().enumerate() {
        rpo_index[block.0 as usize] = index;
    }
    meter.work(order.len())?;
    // Predecessors in CSR form: one count pass, one fill pass.
    let mut start = meter.table(count + 1, 0usize)?;
    let mut edges = 0usize;
    for block in order.iter() {
        let term = &function.blocks[block.0 as usize].term;
        meter.work(term.target_count())?;
        for target in term.targets() {
            let successor = target.block.0 as usize;
            if successor < count {
                start[successor + 1] += 1;
                edges += 1;
            }
        }
    }
    for index in 0..count {
        start[index + 1] += start[index];
    }
    meter.work(count)?;
    let mut fill = meter.table(count, 0usize)?;
    let mut predecessors = meter.table(edges, 0usize)?;
    for block in order.iter() {
        for target in function.blocks[block.0 as usize].term.targets() {
            let successor = target.block.0 as usize;
            if successor < count {
                predecessors[start[successor] + fill[successor]] = block.0 as usize;
                fill[successor] += 1;
            }
        }
    }
    meter.work(edges)?;
    drop(fill);
    let mut idom = meter.table(count, None)?;
    if count == 0 {
        return Ok(idom);
    }
    idom[0] = Some(0);
    let intersect = |idom: &[Option<usize>], mut a: usize, mut b: usize| -> Result<usize, String> {
        while a != b {
            while rpo_index[a] > rpo_index[b] {
                meter.work(1)?;
                a = idom[a].ok_or("dominator chain left the processed region")?;
            }
            while rpo_index[b] > rpo_index[a] {
                meter.work(1)?;
                b = idom[b].ok_or("dominator chain left the processed region")?;
            }
        }
        Ok(a)
    };
    let mut changed = true;
    while changed {
        changed = false;
        meter.work(order.len())?;
        for block in order.iter().skip(1) {
            let block = block.0 as usize;
            let mut new = None;
            let preds = &predecessors[start[block]..start[block + 1]];
            meter.work(preds.len())?;
            for &predecessor in preds {
                if idom[predecessor].is_none() {
                    continue;
                }
                new = Some(match new {
                    None => predecessor,
                    Some(current) => intersect(&idom, predecessor, current)?,
                });
            }
            if new.is_some() && idom[block] != new {
                idom[block] = new;
                changed = true;
            }
        }
    }
    Ok(idom)
}

fn dominates(
    idom: &[Option<usize>],
    a: usize,
    mut b: usize,
    meter: &Meter<'_>,
) -> Result<bool, String> {
    loop {
        meter.work(1)?;
        if a == b {
            return Ok(true);
        }
        match idom[b] {
            Some(parent) if parent != b => b = parent,
            _ => return Ok(false),
        }
    }
}

/// Value types indexed by `SsaValue`, produced by a successful `verify`.
/// Holds the lease for its own storage: dropping it refunds, so the type
/// table is paid for exactly as long as its consumer keeps it.
pub type Types = Table<Option<SsaTy>>;

/// Call signature lookup, borrowed: verification never clones signatures.
pub type Signatures<'a> = dyn Fn(FuncRef) -> Option<&'a SsaSignature> + 'a;

/// Unmetered verification for tests and the interpreter.
pub fn verify(
    function: &SsaFunction,
    callee: &dyn Fn(FuncRef) -> Option<SsaSignature>,
) -> Result<Vec<Option<SsaTy>>, VerifyError> {
    let budget = super::meter::unlimited();
    let meter = Meter::new(&budget);
    // Materialize owned signatures once so the borrowed form can be used.
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
    Ok(verify_metered(function, &lookup, &meter)?.to_vec())
}

/// Metered verification. Every table is admitted before allocation and every
/// definition, operand, dominance hop, propagation round and edge argument is
/// charged. Budget failures return `VerifyError::resource`, never invalid IR.
pub fn verify_metered<'a>(
    function: &SsaFunction,
    callee: &Signatures<'a>,
    meter: &Meter<'_>,
) -> Result<Types, VerifyError> {
    let count = function.blocks.len();
    if count == 0 {
        fail!("function has no blocks");
    }
    if u32::try_from(count).is_err() {
        fail!("too many blocks");
    }
    let entry = &function.blocks[0].params;
    paid(meter.work(entry.len()))?;
    if entry.len() != function.params.len()
        || entry.iter().zip(&function.params).any(|((_, a), b)| a != b)
    {
        fail!("entry block parameters do not match the signature");
    }

    // Definitions.
    paid(meter.work(count))?;
    let bound = function.value_bound();
    let mut defs: Table<Option<Def>> = paid(meter.table(bound, None))?;
    let mut define = |value: SsaValue, def: Def| -> Result<(), VerifyError> {
        let slot = &mut defs[value.0 as usize];
        if slot.is_some() {
            fail!("value v{} defined more than once", value.0);
        }
        *slot = Some(def);
        Ok(())
    };
    for (block, data) in function.blocks.iter().enumerate() {
        paid(super::meter::add(data.params.len(), data.insts.len()).and_then(|n| meter.work(n)))?;
        for (value, ty) in &data.params {
            define(
                *value,
                Def {
                    block,
                    position: 0,
                    ty: Some(*ty),
                },
            )?;
        }
        // Result types are computed by the type pass below.
        for (index, inst) in data.insts.iter().enumerate() {
            define(
                inst.result,
                Def {
                    block,
                    position: index + 1,
                    ty: None,
                },
            )?;
        }
    }

    let idom = paid(dominators_metered(function, meter))?;
    let check_use = |defs: &[Option<Def>],
                     value: SsaValue,
                     block: usize,
                     position: usize|
     -> Result<(), VerifyError> {
        let def = match defs.get(value.0 as usize).copied().flatten() {
            Some(def) => def,
            None => fail!("use of undefined value v{}", value.0),
        };
        if idom[block].is_none() {
            return Ok(()); // unreachable code never executes
        }
        let ok = if def.block == block {
            def.position < position
        } else {
            paid(dominates(&idom, def.block, block, meter))?
        };
        if !ok {
            fail!(
                "v{} used in b{block} without a dominating definition",
                value.0
            );
        }
        Ok(())
    };

    // Reachable blocks in RPO: every operand's definition dominates its use,
    // so its type is already known and any error is final.
    let reachable = paid(function.reverse_postorder_metered(meter))?;
    let mut seen = paid(meter.table(count, false))?;
    for block in reachable.iter() {
        let block = block.0 as usize;
        seen[block] = true;
        let data = &function.blocks[block];
        for (index, inst) in data.insts.iter().enumerate() {
            paid(meter.work(inst.kind.operand_count() + 1))?;
            for operand in inst.kind.operands() {
                check_use(&defs, operand, block, index + 1)?;
            }
            let ty = inst_type(&inst.kind, &defs, callee)?;
            if let Some(def) = defs[inst.result.0 as usize].as_mut() {
                def.ty = Some(ty);
            }
        }
    }
    // Unreachable blocks never execute but must still be well typed. They may
    // use values in any order, so type them to a fixpoint. The result does
    // not depend on block numbering.
    let unreachable_count = seen.iter().filter(|s| !**s).count();
    paid(meter.work(count))?;
    let mut unreachable = paid(meter.list::<usize>(unreachable_count))?;
    let mut pending_count = 0usize;
    for block in 0..count {
        if !seen[block] {
            paid(unreachable.push(block))?;
            pending_count += function.blocks[block].insts.len();
        }
    }
    let mut pending = paid(meter.list::<(usize, usize)>(pending_count))?;
    for &block in unreachable.iter() {
        for (index, inst) in function.blocks[block].insts.iter().enumerate() {
            paid(meter.work(inst.kind.operand_count() + 1))?;
            for operand in inst.kind.operands() {
                check_use(&defs, operand, block, index + 1)?;
            }
            paid(pending.push((block, index)))?;
        }
    }
    loop {
        let mut last_error = None;
        let before = pending.len();
        // One round visits every pending instruction once.
        paid(meter.work(before))?;
        pending.retain(|&(block, index)| {
            let inst = &function.blocks[block].insts[index];
            match inst_type(&inst.kind, &defs, callee) {
                Ok(ty) => {
                    if let Some(def) = defs[inst.result.0 as usize].as_mut() {
                        def.ty = Some(ty);
                    }
                    false
                }
                Err(error) => {
                    last_error = Some(error);
                    true
                }
            }
        });
        if pending.is_empty() {
            break;
        }
        if pending.len() == before {
            return Err(last_error.expect("pending instruction failed"));
        }
    }
    drop(pending);

    for block in reachable
        .iter()
        .map(|b| b.0 as usize)
        .chain(unreachable.iter().copied())
    {
        let data = &function.blocks[block];
        let position = data.insts.len() + 1;
        paid(meter.work(data.term.operand_count() + data.term.target_count() + 1))?;
        for operand in data.term.operands() {
            check_use(&defs, operand, block, position)?;
        }
        match &data.term {
            SsaTerminator::Return(value) => {
                if ty_of(&defs, *value)? != function.ret {
                    fail!("b{block} returns the wrong type");
                }
            }
            SsaTerminator::Branch { cond, .. } => {
                if ty_of(&defs, *cond)? != SsaTy::Bool {
                    fail!("b{block} branches on a non-bool");
                }
            }
            SsaTerminator::Panic(super::PanicKind::Message(index)) => {
                if *index as usize >= function.panics.len() {
                    fail!("b{block} panics with missing message {index}");
                }
            }
            SsaTerminator::Jump(_) | SsaTerminator::Panic(_) => {}
        }
        for target in data.term.targets() {
            let Some(successor) = function.block(target.block) else {
                fail!("b{block} targets missing block b{}", target.block.0);
            };
            if target.block == SsaBlock(0) {
                fail!("b{block} targets the entry block");
            }
            if successor.params.len() != target.args.len() {
                fail!(
                    "b{block} passes the wrong argument count to b{}",
                    target.block.0
                );
            }
            for (arg, (_, ty)) in target.args.iter().zip(&successor.params) {
                if ty_of(&defs, *arg)? != *ty {
                    fail!("b{block} passes a mistyped argument to b{}", target.block.0);
                }
            }
        }
    }
    drop(seen);
    drop(unreachable);
    drop(reachable);
    drop(idom);
    let mut types = paid(meter.table(bound, None))?;
    for (slot, def) in types.iter_mut().zip(defs.iter()) {
        *slot = def.and_then(|def| def.ty);
    }
    Ok(types)
}

fn ty_of(defs: &[Option<Def>], value: SsaValue) -> Result<SsaTy, VerifyError> {
    match defs.get(value.0 as usize).copied().flatten() {
        Some(Def { ty: Some(ty), .. }) => Ok(ty),
        Some(_) => fail!("v{} used before its type is known", value.0),
        None => fail!("use of undefined value v{}", value.0),
    }
}

fn inst_type<'a>(
    kind: &InstKind,
    defs: &[Option<Def>],
    callee: &Signatures<'a>,
) -> Result<SsaTy, VerifyError> {
    let ty = |value| ty_of(defs, value);
    Ok(match kind {
        InstKind::Const { ty, bits } => {
            if *bits > ty.max_bits() {
                fail!("constant out of range for {ty:?}");
            }
            *ty
        }
        InstKind::Binary {
            op,
            overflow,
            lhs,
            rhs,
        } => {
            let (left, right) = (ty(*lhs)?, ty(*rhs)?);
            if !op.has_overflow_mode() && *overflow != Overflow::Wrap {
                fail!("{op:?} must use Overflow::Wrap");
            }
            match op {
                BinOp::And | BinOp::Or | BinOp::Xor => {
                    if left != right || left == SsaTy::Unit {
                        fail!("{op:?} needs equal int or bool operands");
                    }
                }
                BinOp::Shl | BinOp::Shr => {
                    if !matches!(left, SsaTy::Int(_)) || !matches!(right, SsaTy::Int(_)) {
                        fail!("{op:?} needs integer operands");
                    }
                }
                _ => {
                    if left != right || !matches!(left, SsaTy::Int(_)) {
                        fail!("{op:?} needs equal integer operands");
                    }
                }
            }
            left
        }
        InstKind::Unary {
            op,
            overflow,
            value,
        } => {
            let operand = ty(*value)?;
            match op {
                UnOp::Neg => match operand {
                    SsaTy::Int(int) if int.signed() => {}
                    _ => fail!("Neg needs a signed integer"),
                },
                UnOp::Not => {
                    if *overflow != Overflow::Wrap {
                        fail!("Not must use Overflow::Wrap");
                    }
                    if operand == SsaTy::Unit {
                        fail!("Not needs an int or bool");
                    }
                }
            }
            operand
        }
        InstKind::Cmp { op, lhs, rhs } => {
            let (left, right) = (ty(*lhs)?, ty(*rhs)?);
            if left != right || left == SsaTy::Unit {
                fail!("comparison needs equal int or bool operands");
            }
            let _: CmpOp = *op;
            SsaTy::Bool
        }
        InstKind::Cast { to, value } => {
            if ty(*value)? == SsaTy::Unit {
                fail!("cast from unit");
            }
            SsaTy::Int(*to)
        }
        InstKind::Select {
            cond,
            then,
            otherwise,
        } => {
            if ty(*cond)? != SsaTy::Bool {
                fail!("select condition is not bool");
            }
            let result = ty(*then)?;
            if ty(*otherwise)? != result {
                fail!("select arms differ in type");
            }
            result
        }
        InstKind::Call {
            callee: target,
            args,
        } => {
            let Some(signature) = callee(*target) else {
                fail!("call to unknown function f{}", target.0);
            };
            if signature.params.len() != args.len() {
                fail!("call to f{} has the wrong argument count", target.0);
            }
            for (arg, param) in args.iter().zip(&signature.params) {
                if ty(*arg)? != *param {
                    fail!("call to f{} passes a mistyped argument", target.0);
                }
            }
            signature.ret
        }
    })
}
