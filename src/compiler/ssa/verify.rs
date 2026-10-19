//! Structural, type and dominance checks. Every optimizer node must produce a
//! function that passes `verify`; tests run it after every pass.
use super::{
    BinOp, Block, CmpOp, FuncRef, Function, InstKind, Overflow, Signature, Terminator, Ty, UnOp,
    Value,
};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError(pub String);

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
    ty: Option<Ty>,
}

/// Immediate dominators over the reachable blocks (Cooper, Harvey, Kennedy).
/// `None` for unreachable blocks. The entry is its own dominator.
pub fn dominators(function: &Function) -> Vec<Option<usize>> {
    let count = function.blocks.len();
    let order = function.reverse_postorder();
    let mut rpo_index = vec![usize::MAX; count];
    for (index, block) in order.iter().enumerate() {
        rpo_index[block.0 as usize] = index;
    }
    let mut predecessors = vec![Vec::new(); count];
    for block in &order {
        for target in function.blocks[block.0 as usize].term.targets() {
            let successor = target.block.0 as usize;
            if successor < count {
                predecessors[successor].push(block.0 as usize);
            }
        }
    }
    let mut idom: Vec<Option<usize>> = vec![None; count];
    if count == 0 {
        return idom;
    }
    idom[0] = Some(0);
    let intersect = |idom: &[Option<usize>], mut a: usize, mut b: usize| {
        while a != b {
            while rpo_index[a] > rpo_index[b] {
                a = idom[a].expect("processed");
            }
            while rpo_index[b] > rpo_index[a] {
                b = idom[b].expect("processed");
            }
        }
        a
    };
    let mut changed = true;
    while changed {
        changed = false;
        for block in order.iter().skip(1) {
            let block = block.0 as usize;
            let mut new = None;
            for &predecessor in &predecessors[block] {
                if idom[predecessor].is_none() {
                    continue;
                }
                new = Some(match new {
                    None => predecessor,
                    Some(current) => intersect(&idom, predecessor, current),
                });
            }
            if new.is_some() && idom[block] != new {
                idom[block] = new;
                changed = true;
            }
        }
    }
    idom
}

fn dominates(idom: &[Option<usize>], a: usize, mut b: usize) -> bool {
    loop {
        if a == b {
            return true;
        }
        match idom[b] {
            Some(parent) if parent != b => b = parent,
            _ => return false,
        }
    }
}

/// Value types indexed by `Value`, produced by a successful `verify`.
pub type Types = Vec<Option<Ty>>;

/// Verify `function` and return the type of every defined value. `callee`
/// resolves call signatures.
pub fn verify(
    function: &Function,
    callee: &dyn Fn(FuncRef) -> Option<Signature>,
) -> Result<Types, VerifyError> {
    let count = function.blocks.len();
    if count == 0 {
        fail!("function has no blocks");
    }
    if u32::try_from(count).is_err() {
        fail!("too many blocks");
    }
    let entry_types: Vec<Ty> = function.blocks[0]
        .params
        .iter()
        .map(|(_, ty)| *ty)
        .collect();
    if entry_types != function.params {
        fail!("entry block parameters do not match the signature");
    }

    // Definitions.
    let mut defs: Vec<Option<Def>> = vec![None; function.value_bound()];
    let mut define = |value: Value, def: Def| -> Result<(), VerifyError> {
        let slot = &mut defs[value.0 as usize];
        if slot.is_some() {
            fail!("value v{} defined more than once", value.0);
        }
        *slot = Some(def);
        Ok(())
    };
    for (block, data) in function.blocks.iter().enumerate() {
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

    let idom = dominators(function);
    let check_use = |defs: &[Option<Def>],
                     value: Value,
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
            dominates(&idom, def.block, block)
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
    let reachable: Vec<usize> = function
        .reverse_postorder()
        .iter()
        .map(|b| b.0 as usize)
        .collect();
    let mut seen = vec![false; count];
    for &block in &reachable {
        seen[block] = true;
        let data = &function.blocks[block];
        for (index, inst) in data.insts.iter().enumerate() {
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
    let unreachable: Vec<usize> = (0..count).filter(|block| !seen[*block]).collect();
    let mut pending: Vec<(usize, usize)> = Vec::new();
    for &block in &unreachable {
        for (index, inst) in function.blocks[block].insts.iter().enumerate() {
            for operand in inst.kind.operands() {
                check_use(&defs, operand, block, index + 1)?;
            }
            pending.push((block, index));
        }
    }
    loop {
        let mut last_error = None;
        let before = pending.len();
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
    let order: Vec<usize> = reachable.into_iter().chain(unreachable).collect();

    for &block in &order {
        let data = &function.blocks[block];
        let position = data.insts.len() + 1;
        for operand in data.term.operands() {
            check_use(&defs, operand, block, position)?;
        }
        match &data.term {
            Terminator::Return(value) => {
                if ty_of(&defs, *value)? != function.ret {
                    fail!("b{block} returns the wrong type");
                }
            }
            Terminator::Branch { cond, .. } => {
                if ty_of(&defs, *cond)? != Ty::Bool {
                    fail!("b{block} branches on a non-bool");
                }
            }
            Terminator::Jump(_) | Terminator::Panic(_) => {}
        }
        for target in data.term.targets() {
            let Some(successor) = function.block(target.block) else {
                fail!("b{block} targets missing block b{}", target.block.0);
            };
            if target.block == Block(0) {
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
    Ok(defs.iter().map(|def| def.and_then(|def| def.ty)).collect())
}

fn ty_of(defs: &[Option<Def>], value: Value) -> Result<Ty, VerifyError> {
    match defs.get(value.0 as usize).copied().flatten() {
        Some(Def { ty: Some(ty), .. }) => Ok(ty),
        Some(_) => fail!("v{} used before its type is known", value.0),
        None => fail!("use of undefined value v{}", value.0),
    }
}

fn inst_type(
    kind: &InstKind,
    defs: &[Option<Def>],
    callee: &dyn Fn(FuncRef) -> Option<Signature>,
) -> Result<Ty, VerifyError> {
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
                    if left != right || left == Ty::Unit {
                        fail!("{op:?} needs equal int or bool operands");
                    }
                }
                BinOp::Shl | BinOp::Shr => {
                    if !matches!(left, Ty::Int(_)) || !matches!(right, Ty::Int(_)) {
                        fail!("{op:?} needs integer operands");
                    }
                }
                _ => {
                    if left != right || !matches!(left, Ty::Int(_)) {
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
                    Ty::Int(int) if int.signed() => {}
                    _ => fail!("Neg needs a signed integer"),
                },
                UnOp::Not => {
                    if *overflow != Overflow::Wrap {
                        fail!("Not must use Overflow::Wrap");
                    }
                    if operand == Ty::Unit {
                        fail!("Not needs an int or bool");
                    }
                }
            }
            operand
        }
        InstKind::Cmp { op, lhs, rhs } => {
            let (left, right) = (ty(*lhs)?, ty(*rhs)?);
            if left != right || left == Ty::Unit {
                fail!("comparison needs equal int or bool operands");
            }
            let _: CmpOp = *op;
            Ty::Bool
        }
        InstKind::Cast { to, value } => {
            if ty(*value)? == Ty::Unit {
                fail!("cast from unit");
            }
            Ty::Int(*to)
        }
        InstKind::Select {
            cond,
            then,
            otherwise,
        } => {
            if ty(*cond)? != Ty::Bool {
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
