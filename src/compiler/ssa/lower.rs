//! Lowering from a sealed concrete CFG (`ANALYZE` output) to SSA.
//!
//! Only runs on functions whose ownership analysis already succeeded, so every
//! local read is initialized and every call target is validated. Scope today:
//! scalar functions (integers up to 64 bits, `bool`, unit), whole-local places,
//! arithmetic, comparisons, casts, branches, direct calls with scalar
//! signatures, explicit panics and `unreachable`. Anything else returns
//! `Lowering::Unsupported` and the caller keeps the stack emitter for that
//! function. Unsupported is a normal outcome, never an error.
//!
//! Construction: every CFG block becomes an SSA block whose parameters are all
//! locals. `opt::local` then removes trivial parameters, which yields minimal
//! SSA. Overflow checks are always on in the native profile today, matching the
//! stack emitter, so arithmetic lowers to `Overflow::Trap`.
use super::verify::{self, VerifyError};
use super::{
    BinOp, Builder, CmpOp, InstKind, Overflow, PanicKind, SsaBlock, SsaFunction, SsaIntTy,
    SsaSignature, SsaTerminator, SsaTy, SsaValue, UnOp, target,
};
use crate::compiler::cfg;
use crate::compiler::frontend::{BinaryOp, UnaryOp};
use crate::compiler::ir;

/// One direct callee of a lowered function. `FuncRef(i)` indexes this table.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SsaCallee {
    pub symbol: String,
    pub target: ir::CallTarget,
    pub signature: SsaSignature,
}

/// An SSA function plus the identities of everything it calls. Both are part
/// of its content identity (`Lowered::hash`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lowered {
    pub function: SsaFunction,
    pub callees: Vec<SsaCallee>,
}

impl Lowered {
    pub fn signature_of(&self, callee: super::FuncRef) -> Option<SsaSignature> {
        self.callees
            .get(callee.0 as usize)
            .map(|callee| callee.signature.clone())
    }

    pub fn verify(&self) -> Result<verify::Types, VerifyError> {
        verify::verify(&self.function, &|callee| self.signature_of(callee))
    }

    /// Stable content hash covering the body and the callee symbols.
    pub fn hash(&self) -> u128 {
        let mut hash = super::hash::content_hash(&self.function);
        for callee in &self.callees {
            for byte in callee.symbol.bytes().chain([0xff]) {
                hash ^= u128::from(byte);
                hash = hash.wrapping_mul(0x0000000001000000000000000000013b);
            }
        }
        hash
    }
}

#[derive(Debug)]
pub enum Lowering {
    Lowered(Box<Lowered>),
    Unsupported(&'static str),
}

macro_rules! unsupported {
    ($reason:expr) => {
        return Ok(Lowering::Unsupported($reason))
    };
}

fn int_ty(ty: ir::IntTy) -> Option<SsaIntTy> {
    if ty.bits() > 64 {
        return None;
    }
    SsaIntTy::new(ty.bits() as u8, ty.is_signed())
}

/// The SSA type of a CFG type, if this lowering supports it.
pub fn scalar(ty: &ir::Ty) -> Option<SsaTy> {
    match ty {
        ir::Ty::Int(int) => int_ty(*int).map(SsaTy::Int),
        ir::Ty::Bool => Some(SsaTy::Bool),
        ir::Ty::Unit => Some(SsaTy::Unit),
        _ => None,
    }
}

struct Ctx<'a> {
    cfg: &'a cfg::Function,
    builder: Builder,
    types: Vec<SsaTy>,
    blocks: Vec<SsaBlock>,
    callees: Vec<SsaCallee>,
    resolve: &'a dyn Fn(&ir::CallTarget) -> Result<String, String>,
}

/// Lower one sealed concrete CFG. `resolve` maps a call target to its link
/// symbol (the same function the stack emitter uses).
pub fn lower(
    function: &cfg::Function,
    resolve: &dyn Fn(&ir::CallTarget) -> Result<String, String>,
) -> Result<Lowering, String> {
    let mut types = Vec::with_capacity(function.locals.len());
    for local in &function.locals {
        match scalar(&local.ty) {
            Some(ty) => types.push(ty),
            None => unsupported!("non-scalar local"),
        }
    }
    let Some(ret) = scalar(&function.result) else {
        unsupported!("non-scalar result");
    };
    if types.get(function.return_local.0) != Some(&ret) {
        unsupported!("return local type differs from the result");
    }
    let params: Vec<SsaTy> = function.params.iter().map(|p| types[p.0]).collect();
    let (builder, entry_values) = Builder::new(params, ret);
    let mut ctx = Ctx {
        cfg: function,
        builder,
        types,
        blocks: Vec::with_capacity(function.blocks.len()),
        callees: Vec::new(),
        resolve,
    };
    for _ in &function.blocks {
        let (block, _) = ctx.builder.block(&ctx.types.clone());
        ctx.blocks.push(block);
    }
    // Entry: parameters bind their locals, every other local starts as a
    // placeholder zero. Analysis proved no read precedes initialization.
    let mut env: Vec<SsaValue> = Vec::with_capacity(ctx.types.len());
    for (local, ty) in ctx.types.clone().into_iter().enumerate() {
        let param = function.params.iter().position(|p| p.0 == local);
        env.push(match param {
            Some(index) => entry_values[index],
            None => ctx.builder.constant(SsaBlock(0), ty, 0),
        });
    }
    let Some(&entry) = ctx.blocks.get(function.entry.0) else {
        return Err("invalid CFG entry block".into());
    };
    ctx.builder.terminate(SsaBlock(0), super::jump(entry, env));
    for index in 0..function.blocks.len() {
        if let Some(reason) = ctx.block(index)? {
            unsupported!(reason);
        }
    }
    let lowered = Lowered {
        function: ctx.builder.finish(),
        callees: ctx.callees,
    };
    lowered
        .verify()
        .map_err(|error| format!("SSA lowering produced {error}"))?;
    Ok(Lowering::Lowered(Box::new(lowered)))
}

impl Ctx<'_> {
    fn whole(place: &ir::Place) -> Option<usize> {
        place.projection.is_empty().then_some(place.local.0)
    }

    fn operand(
        &mut self,
        block: SsaBlock,
        env: &[SsaValue],
        operand: &cfg::Operand,
    ) -> Option<SsaValue> {
        match operand {
            cfg::Operand::Copy(place) | cfg::Operand::Move(place) => {
                Self::whole(place).and_then(|local| env.get(local).copied())
            }
            cfg::Operand::Constant(constant) => {
                let (ty, bits) = match constant {
                    cfg::Constant::Int(value) => (SsaTy::Int(int_ty(value.ty)?), value.bits),
                    cfg::Constant::Bool(value) => (SsaTy::Bool, u128::from(*value)),
                    cfg::Constant::Unit => (SsaTy::Unit, 0),
                    _ => return None,
                };
                if bits > ty.max_bits() {
                    return None;
                }
                Some(self.builder.constant(block, ty, bits))
            }
        }
    }

    fn operand_ty(&self, operand: &cfg::Operand) -> Option<SsaTy> {
        scalar(&self.cfg.operand_ty(operand).ok()?)
    }

    fn rvalue(
        &mut self,
        block: SsaBlock,
        env: &[SsaValue],
        rvalue: &cfg::Rvalue,
    ) -> Result<SsaValue, &'static str> {
        let op = |ctx: &mut Self, operand| {
            ctx.operand(block, env, operand)
                .ok_or("unsupported operand")
        };
        Ok(match rvalue {
            cfg::Rvalue::Use(operand) => op(self, operand)?,
            cfg::Rvalue::Unary { op: unary, value } => {
                let ty = self.operand_ty(value).ok_or("unsupported operand")?;
                let value = op(self, value)?;
                let (op, overflow) = match (unary, ty) {
                    (UnaryOp::Neg, SsaTy::Int(int)) if int.signed() => (UnOp::Neg, Overflow::Trap),
                    (UnaryOp::Not, SsaTy::Int(_) | SsaTy::Bool) => (UnOp::Not, Overflow::Wrap),
                    _ => return Err("unsupported unary operand"),
                };
                self.builder.inst(
                    block,
                    InstKind::Unary {
                        op,
                        overflow,
                        value,
                    },
                )
            }
            cfg::Rvalue::Binary {
                op: binary,
                left,
                right,
            } => {
                let left_ty = self.operand_ty(left).ok_or("unsupported operand")?;
                let right_ty = self.operand_ty(right).ok_or("unsupported operand")?;
                let lhs = op(self, left)?;
                let rhs = op(self, right)?;
                let arith = |op| InstKind::Binary {
                    op,
                    overflow: if op.has_overflow_mode() {
                        Overflow::Trap
                    } else {
                        Overflow::Wrap
                    },
                    lhs,
                    rhs,
                };
                let cmp = |op| InstKind::Cmp { op, lhs, rhs };
                let kind = match binary {
                    BinaryOp::Add => arith(BinOp::Add),
                    BinaryOp::Sub => arith(BinOp::Sub),
                    BinaryOp::Mul => arith(BinOp::Mul),
                    BinaryOp::Div => arith(BinOp::Div),
                    BinaryOp::Rem => arith(BinOp::Rem),
                    BinaryOp::BitAnd => arith(BinOp::And),
                    BinaryOp::BitOr => arith(BinOp::Or),
                    BinaryOp::BitXor => arith(BinOp::Xor),
                    BinaryOp::Shl => arith(BinOp::Shl),
                    BinaryOp::Shr => arith(BinOp::Shr),
                    BinaryOp::Eq => cmp(CmpOp::Eq),
                    BinaryOp::Ne => cmp(CmpOp::Ne),
                    BinaryOp::Lt => cmp(CmpOp::Lt),
                    BinaryOp::Le => cmp(CmpOp::Le),
                    BinaryOp::Gt => cmp(CmpOp::Gt),
                    BinaryOp::Ge => cmp(CmpOp::Ge),
                    BinaryOp::And | BinaryOp::Or => return Err("short-circuit operator"),
                };
                let shift = matches!(binary, BinaryOp::Shl | BinaryOp::Shr);
                let same = left_ty == right_ty && left_ty != SsaTy::Unit;
                if !(same || shift && matches!((left_ty, right_ty), (SsaTy::Int(_), SsaTy::Int(_))))
                {
                    return Err("unsupported binary operand types");
                }
                self.builder.inst(block, kind)
            }
            cfg::Rvalue::IntCast { value, to } => {
                let to = int_ty(*to).ok_or("wide cast")?;
                if matches!(self.operand_ty(value), None | Some(SsaTy::Unit)) {
                    return Err("unsupported cast source");
                }
                let value = op(self, value)?;
                self.builder.inst(block, InstKind::Cast { to, value })
            }
            _ => return Err("unsupported rvalue"),
        })
    }

    fn assign_target(&self, place: &ir::Place) -> Option<usize> {
        Self::whole(place)
    }

    /// Lower CFG block `index`. Returns an unsupported reason, if any.
    fn block(&mut self, index: usize) -> Result<Option<&'static str>, String> {
        let block = self.blocks[index];
        let mut env: Vec<SsaValue> = self.builder_params(block);
        let cfg = self.cfg;
        let data = &cfg.blocks[index];
        for statement in &data.statements {
            match &statement.value {
                cfg::Statement::StorageLive(_)
                | cfg::Statement::StorageDead(_)
                | cfg::Statement::Inspect(_) => {}
                cfg::Statement::Assign { destination, value } => {
                    let Some(local) = self.assign_target(destination) else {
                        return Ok(Some("projected assignment"));
                    };
                    match self.rvalue(block, &env, value) {
                        Ok(value) => env[local] = value,
                        Err(reason) => return Ok(Some(reason)),
                    }
                }
                _ => return Ok(Some("unsupported statement")),
            }
        }
        let edge = |ctx: &Self, to: cfg::BlockId, env: &[SsaValue]| -> Result<_, String> {
            let to = *ctx.blocks.get(to.0).ok_or("invalid CFG block target")?;
            Ok(target(to, env.to_vec()))
        };
        let term = match &data.terminator.value {
            cfg::Terminator::Goto(next) => SsaTerminator::Jump(edge(self, *next, &env)?),
            cfg::Terminator::Switch {
                condition,
                then_block,
                else_block,
            } => {
                if self.operand_ty(condition) != Some(SsaTy::Bool) {
                    return Ok(Some("non-bool switch"));
                }
                let Some(cond) = self.operand(block, &env, condition) else {
                    return Ok(Some("unsupported operand"));
                };
                SsaTerminator::Branch {
                    cond,
                    then: edge(self, *then_block, &env)?,
                    otherwise: edge(self, *else_block, &env)?,
                }
            }
            cfg::Terminator::Call {
                target: ir::Callee::Direct(call),
                signature,
                args,
                destination,
                next,
                ..
            } => {
                let params: Option<Vec<SsaTy>> = signature.params.iter().map(scalar).collect();
                let (Some(params), Some(ret)) = (params, scalar(&signature.result)) else {
                    return Ok(Some("non-scalar call signature"));
                };
                let Some(local) = self.assign_target(destination) else {
                    return Ok(Some("projected call destination"));
                };
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    match self.operand(block, &env, arg) {
                        Some(value) => values.push(value),
                        None => return Ok(Some("unsupported operand")),
                    }
                }
                let symbol = (self.resolve)(call)?;
                let callee = SsaCallee {
                    symbol,
                    target: call.clone(),
                    signature: SsaSignature { params, ret },
                };
                let position = match self.callees.iter().position(|c| *c == callee) {
                    Some(position) => position,
                    None => {
                        self.callees.push(callee);
                        self.callees.len() - 1
                    }
                };
                let result = self.builder.inst(
                    block,
                    InstKind::Call {
                        callee: super::FuncRef(position as u32),
                        args: values,
                    },
                );
                env[local] = result;
                SsaTerminator::Jump(edge(self, *next, &env)?)
            }
            cfg::Terminator::Return => SsaTerminator::Return(env[cfg.return_local.0]),
            cfg::Terminator::Panic(message) => SsaTerminator::Panic(
                self.builder
                    .panic_text(message.message.clone(), message.location.clone()),
            ),
            cfg::Terminator::Unreachable => SsaTerminator::Panic(PanicKind::Unreachable),
            cfg::Terminator::Call { .. } => return Ok(Some("indirect call")),
            cfg::Terminator::Drop { .. } => return Ok(Some("drop")),
            cfg::Terminator::SwitchVariant { .. } => return Ok(Some("variant switch")),
        };
        self.builder.terminate(block, term);
        Ok(None)
    }

    fn builder_params(&self, block: SsaBlock) -> Vec<SsaValue> {
        self.builder.params_of(block)
    }
}
