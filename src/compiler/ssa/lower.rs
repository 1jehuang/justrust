//! Lowering from a sealed concrete CFG (`ANALYZE` output) to SSA.
//!
//! Only runs on functions whose ownership analysis already succeeded, so every
//! local read is initialized and every call target is validated. Scope today:
//! scalar functions (integers up to 64 bits, `bool`, unit), whole-local places,
//! arithmetic, comparisons, casts, branches, direct calls with scalar
//! signatures, explicit panics and `unreachable`. Anything else returns
//! `Lowering::Unsupported` and the caller keeps the stack emitter for that
//! function. Unsupported is a normal outcome, distinct from resource or
//! infrastructure failure (`Err`).
//!
//! Construction: every CFG block becomes an SSA block whose parameters are all
//! locals. `opt::local` then removes trivial parameters, which yields minimal
//! SSA. Overflow checks are always on in the native profile today, matching the
//! stack emitter, so arithmetic lowers to `Overflow::Trap`.
//!
//! Accounting (audit priority 1-2): a validation pass computes the exact
//! output dimensions without allocating, charging work per statement,
//! operand and terminator. The output's bytes (blocks times locals block
//! parameters, edge arguments, instructions, call arguments, callees, panic
//! text) are admitted before any allocation, every buffer is reserved at
//! exact capacity, and the lease is reconciled to the real capacities.
use super::meter::{self, Meter};
use super::store::{self, Owned};
use super::verify::{self, VerifyError};
use super::{
    BinOp, BlockData, CmpOp, FuncRef, Inst, InstKind, Overflow, PanicKind, SsaBlock, SsaFunction,
    SsaIntTy, SsaPanicText, SsaSignature, SsaTarget, SsaTerminator, SsaTy, SsaValue, UnOp,
};
use crate::compiler::cfg;
use crate::compiler::frontend::{BinaryOp, UnaryOp};
use crate::compiler::ir;
use std::mem::size_of;

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
    /// Borrowed signature lookup: verification never clones signatures.
    pub fn signature(&self, callee: FuncRef) -> Option<&SsaSignature> {
        self.callees
            .get(callee.0 as usize)
            .map(|callee| &callee.signature)
    }

    /// Owned signature, for the unmetered interpreter and tests only.
    pub fn signature_of(&self, callee: FuncRef) -> Option<SsaSignature> {
        self.signature(callee).cloned()
    }

    pub fn verify(&self, meter: &Meter<'_>) -> Result<verify::Types, VerifyError> {
        verify::verify_metered(&self.function, &|callee| self.signature(callee), meter)
    }

    /// Stable content hash covering the body and the callee symbols. Work is
    /// charged per hashed byte.
    pub fn hash(&self, meter: &Meter<'_>) -> Result<u128, String> {
        let mut hash = super::hash::content_hash_metered(&self.function, meter)?;
        for callee in &self.callees {
            meter.work(meter::add(callee.symbol.len(), 1)?)?;
            for byte in callee.symbol.bytes().chain([0xff]) {
                hash ^= u128::from(byte);
                hash = hash.wrapping_mul(0x0000000001000000000000000000013b);
            }
        }
        Ok(hash)
    }
}

#[derive(Debug)]
pub enum Lowering {
    Lowered(Box<Owned<Lowered>>),
    Unsupported(&'static str),
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

fn whole(place: &ir::Place) -> Option<usize> {
    place.projection.is_empty().then_some(place.local.0)
}

fn constant(constant: &cfg::Constant) -> Option<(SsaTy, u128)> {
    let (ty, bits) = match constant {
        cfg::Constant::Int(value) => (SsaTy::Int(int_ty(value.ty)?), value.bits),
        cfg::Constant::Bool(value) => (SsaTy::Bool, u128::from(*value)),
        cfg::Constant::Unit => (SsaTy::Unit, 0),
        _ => return None,
    };
    (bits <= ty.max_bits()).then_some((ty, bits))
}

/// Validates one operand without allocating. Returns how many instructions
/// it adds (a constant materializes one).
fn check_operand(function: &cfg::Function, operand: &cfg::Operand) -> Result<usize, &'static str> {
    match operand {
        cfg::Operand::Copy(place) | cfg::Operand::Move(place) => {
            let local = whole(place).ok_or("unsupported operand")?;
            if local >= function.locals.len() {
                return Err("unsupported operand");
            }
            Ok(0)
        }
        cfg::Operand::Constant(value) => constant(value).map(|_| 1).ok_or("unsupported operand"),
    }
}

fn operand_ty(function: &cfg::Function, operand: &cfg::Operand) -> Option<SsaTy> {
    scalar(&function.operand_ty(operand).ok()?)
}

fn binary_kind(op: BinaryOp) -> Result<(Option<BinOp>, Option<CmpOp>), &'static str> {
    Ok(match op {
        BinaryOp::Add => (Some(BinOp::Add), None),
        BinaryOp::Sub => (Some(BinOp::Sub), None),
        BinaryOp::Mul => (Some(BinOp::Mul), None),
        BinaryOp::Div => (Some(BinOp::Div), None),
        BinaryOp::Rem => (Some(BinOp::Rem), None),
        BinaryOp::BitAnd => (Some(BinOp::And), None),
        BinaryOp::BitOr => (Some(BinOp::Or), None),
        BinaryOp::BitXor => (Some(BinOp::Xor), None),
        BinaryOp::Shl => (Some(BinOp::Shl), None),
        BinaryOp::Shr => (Some(BinOp::Shr), None),
        BinaryOp::Eq => (None, Some(CmpOp::Eq)),
        BinaryOp::Ne => (None, Some(CmpOp::Ne)),
        BinaryOp::Lt => (None, Some(CmpOp::Lt)),
        BinaryOp::Le => (None, Some(CmpOp::Le)),
        BinaryOp::Gt => (None, Some(CmpOp::Gt)),
        BinaryOp::Ge => (None, Some(CmpOp::Ge)),
        BinaryOp::And | BinaryOp::Or => return Err("short-circuit operator"),
    })
}

/// Validates an rvalue and returns the instructions it adds.
fn check_rvalue(function: &cfg::Function, rvalue: &cfg::Rvalue) -> Result<usize, &'static str> {
    match rvalue {
        cfg::Rvalue::Use(operand) => check_operand(function, operand),
        cfg::Rvalue::Unary { op, value } => {
            let ty = operand_ty(function, value).ok_or("unsupported operand")?;
            match (op, ty) {
                (UnaryOp::Neg, SsaTy::Int(int)) if int.signed() => {}
                (UnaryOp::Not, SsaTy::Int(_) | SsaTy::Bool) => {}
                _ => return Err("unsupported unary operand"),
            }
            Ok(check_operand(function, value)? + 1)
        }
        cfg::Rvalue::Binary { op, left, right } => {
            let left_ty = operand_ty(function, left).ok_or("unsupported operand")?;
            let right_ty = operand_ty(function, right).ok_or("unsupported operand")?;
            binary_kind(*op)?;
            let shift = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
            let same = left_ty == right_ty && left_ty != SsaTy::Unit;
            let ints = matches!((left_ty, right_ty), (SsaTy::Int(_), SsaTy::Int(_)));
            if !(same || shift && ints) {
                return Err("unsupported binary operand types");
            }
            Ok(check_operand(function, left)? + check_operand(function, right)? + 1)
        }
        cfg::Rvalue::IntCast { value, to } => {
            int_ty(*to).ok_or("wide cast")?;
            if matches!(operand_ty(function, value), None | Some(SsaTy::Unit)) {
                return Err("unsupported cast source");
            }
            Ok(check_operand(function, value)? + 1)
        }
        _ => Err("unsupported rvalue"),
    }
}

/// Exact output dimensions, computed by a non-allocating validation pass.
struct Shape {
    insts: usize,
    edges: usize,
    calls: usize,
    call_args: usize,
    signature_params: usize,
    panics: usize,
    panic_bytes: usize,
}

/// Fills `per_block[i]` with the instructions CFG block `i` lowers to.
fn shape(
    function: &cfg::Function,
    types: &[SsaTy],
    per_block: &mut [usize],
    meter: &Meter<'_>,
) -> Result<Result<Shape, &'static str>, String> {
    let locals = function.locals.len();
    let mut shape = Shape {
        // Entry block: one placeholder constant per non-parameter local.
        insts: locals.saturating_sub(function.params.len()),
        edges: 1,
        calls: 0,
        call_args: 0,
        signature_params: 0,
        panics: 0,
        panic_bytes: 0,
    };
    for (data, block_insts) in function.blocks.iter().zip(per_block.iter_mut()) {
        let before = shape.insts;
        meter.work(meter::add(data.statements.len(), 1)?)?;
        for statement in &data.statements {
            match &statement.value {
                cfg::Statement::StorageLive(_)
                | cfg::Statement::StorageDead(_)
                | cfg::Statement::Inspect(_) => {}
                cfg::Statement::Assign { destination, value } => {
                    if whole(destination).is_none_or(|local| local >= locals) {
                        return Ok(Err("projected assignment"));
                    }
                    match check_rvalue(function, value) {
                        Ok(count) => shape.insts = meter::add(shape.insts, count)?,
                        Err(reason) => return Ok(Err(reason)),
                    }
                }
                _ => return Ok(Err("unsupported statement")),
            }
        }
        let check_target = |to: cfg::BlockId| to.0 < function.blocks.len();
        match &data.terminator.value {
            cfg::Terminator::Goto(next) => {
                if !check_target(*next) {
                    return Err("invalid CFG block target".into());
                }
                shape.edges = meter::add(shape.edges, 1)?;
            }
            cfg::Terminator::Switch {
                condition,
                then_block,
                else_block,
            } => {
                if operand_ty(function, condition) != Some(SsaTy::Bool) {
                    return Ok(Err("non-bool switch"));
                }
                if !check_target(*then_block) || !check_target(*else_block) {
                    return Err("invalid CFG block target".into());
                }
                match check_operand(function, condition) {
                    Ok(count) => shape.insts = meter::add(shape.insts, count)?,
                    Err(reason) => return Ok(Err(reason)),
                }
                shape.edges = meter::add(shape.edges, 2)?;
            }
            cfg::Terminator::Call {
                target: ir::Callee::Direct(_),
                signature,
                args,
                destination,
                next,
                ..
            } => {
                meter.work(meter::add(args.len(), signature.params.len())?)?;
                if signature.params.iter().any(|ty| scalar(ty).is_none())
                    || scalar(&signature.result).is_none()
                {
                    return Ok(Err("non-scalar call signature"));
                }
                if whole(destination).is_none_or(|local| local >= locals) {
                    return Ok(Err("projected call destination"));
                }
                if !check_target(*next) {
                    return Err("invalid CFG block target".into());
                }
                for arg in args {
                    match check_operand(function, arg) {
                        Ok(count) => shape.insts = meter::add(shape.insts, count)?,
                        Err(reason) => return Ok(Err(reason)),
                    }
                }
                shape.insts = meter::add(shape.insts, 1)?;
                shape.edges = meter::add(shape.edges, 1)?;
                shape.calls = meter::add(shape.calls, 1)?;
                shape.call_args = meter::add(shape.call_args, args.len())?;
                shape.signature_params =
                    meter::add(shape.signature_params, signature.params.len())?;
            }
            cfg::Terminator::Return => {
                if function.return_local.0 >= types.len() {
                    return Err("invalid CFG return local".into());
                }
            }
            cfg::Terminator::Panic(message) => {
                shape.panics = meter::add(shape.panics, 1)?;
                shape.panic_bytes = meter::add(
                    shape.panic_bytes,
                    meter::add(message.message.len(), message.location.len())?,
                )?;
            }
            cfg::Terminator::Unreachable => {}
            cfg::Terminator::Call { .. } => return Ok(Err("indirect call")),
            cfg::Terminator::Drop { .. } => return Ok(Err("drop")),
            cfg::Terminator::SwitchVariant { .. } => return Ok(Err("variant switch")),
        }
        *block_insts = shape.insts - before;
    }
    Ok(Ok(shape))
}

/// Upper bound on output bytes for `shape`, admitted before allocation.
fn shape_bytes(
    shape: &Shape,
    blocks: usize,
    locals: usize,
    params: usize,
) -> Result<usize, String> {
    let ssa_blocks = meter::add(blocks, 1)?;
    let mut total = 0usize;
    let mut part = |count: usize, size: usize| -> Result<(), String> {
        total = meter::add(total, meter::mul(count, size)?)?;
        Ok(())
    };
    part(params, size_of::<SsaTy>())?;
    part(ssa_blocks, size_of::<BlockData>())?;
    // Every non-entry block takes all locals as parameters.
    part(meter::mul(blocks, locals)?, size_of::<(SsaValue, SsaTy)>())?;
    part(params, size_of::<(SsaValue, SsaTy)>())?;
    part(shape.insts, size_of::<Inst>())?;
    // Every edge passes all locals.
    part(meter::mul(shape.edges, locals)?, size_of::<SsaValue>())?;
    part(shape.call_args, size_of::<SsaValue>())?;
    part(shape.panics, size_of::<SsaPanicText>())?;
    part(shape.panic_bytes, 1)?;
    // Callees deduplicate, so these are upper bounds: one slot and one
    // signature per call site.
    part(shape.calls, size_of::<SsaCallee>())?;
    part(shape.signature_params, size_of::<SsaTy>())?;
    Ok(total)
}

fn try_vec<T>(capacity: usize) -> Result<Vec<T>, String> {
    let mut data = Vec::new();
    data.try_reserve_exact(capacity)
        .map_err(|_| "optimizer allocation failed".to_string())?;
    Ok(data)
}

/// Pushes within a capacity reserved exactly from the validated shape. A push
/// beyond it means the shape pass and the builder disagree: an internal error,
/// reported instead of growing unpaid.
fn put<T>(data: &mut Vec<T>, value: T) -> Result<(), String> {
    if data.len() == data.capacity() {
        return Err("SSA lowering exceeded its admitted shape".into());
    }
    data.push(value);
    Ok(())
}

struct Ctx<'a, 'm> {
    cfg: &'a cfg::Function,
    meter: &'a Meter<'m>,
    function: SsaFunction,
    callees: Vec<SsaCallee>,
    next_value: u32,
    resolve: &'a dyn Fn(&ir::CallTarget) -> Result<String, String>,
}

impl Ctx<'_, '_> {
    fn fresh(&mut self) -> Result<SsaValue, String> {
        let value = SsaValue(self.next_value);
        self.next_value = self
            .next_value
            .checked_add(1)
            .ok_or("SSA value IDs exhausted")?;
        Ok(value)
    }

    fn inst(&mut self, block: usize, kind: InstKind) -> Result<SsaValue, String> {
        let result = self.fresh()?;
        put(
            &mut self.function.blocks[block].insts,
            Inst { result, kind },
        )?;
        Ok(result)
    }

    fn operand(
        &mut self,
        block: usize,
        env: &[SsaValue],
        operand: &cfg::Operand,
    ) -> Result<SsaValue, String> {
        match operand {
            cfg::Operand::Copy(place) | cfg::Operand::Move(place) => {
                let local = whole(place).ok_or("validated operand changed")?;
                Ok(env[local])
            }
            cfg::Operand::Constant(value) => {
                let (ty, bits) = constant(value).ok_or("validated constant changed")?;
                self.inst(block, InstKind::Const { ty, bits })
            }
        }
    }

    fn edge(&self, to: cfg::BlockId, env: &[SsaValue]) -> Result<SsaTarget, String> {
        self.meter.work(env.len())?;
        let mut args = try_vec(env.len())?;
        args.extend_from_slice(env);
        Ok(SsaTarget {
            block: SsaBlock(u32::try_from(to.0 + 1).map_err(|_| "too many blocks")?),
            args,
        })
    }

    fn rvalue(
        &mut self,
        block: usize,
        env: &[SsaValue],
        rvalue: &cfg::Rvalue,
    ) -> Result<SsaValue, String> {
        Ok(match rvalue {
            cfg::Rvalue::Use(operand) => self.operand(block, env, operand)?,
            cfg::Rvalue::Unary { op, value } => {
                let ty = operand_ty(self.cfg, value).ok_or("validated operand changed")?;
                let value = self.operand(block, env, value)?;
                let (op, overflow) = match (op, ty) {
                    (UnaryOp::Neg, _) => (UnOp::Neg, Overflow::Trap),
                    (UnaryOp::Not, _) => (UnOp::Not, Overflow::Wrap),
                };
                self.inst(
                    block,
                    InstKind::Unary {
                        op,
                        overflow,
                        value,
                    },
                )?
            }
            cfg::Rvalue::Binary { op, left, right } => {
                let lhs = self.operand(block, env, left)?;
                let rhs = self.operand(block, env, right)?;
                let kind = match binary_kind(*op).map_err(String::from)? {
                    (Some(op), _) => InstKind::Binary {
                        op,
                        overflow: if op.has_overflow_mode() {
                            Overflow::Trap
                        } else {
                            Overflow::Wrap
                        },
                        lhs,
                        rhs,
                    },
                    (None, Some(op)) => InstKind::Cmp { op, lhs, rhs },
                    (None, None) => return Err("validated binary changed".into()),
                };
                self.inst(block, kind)?
            }
            cfg::Rvalue::IntCast { value, to } => {
                let to = int_ty(*to).ok_or("validated cast changed")?;
                let value = self.operand(block, env, value)?;
                self.inst(block, InstKind::Cast { to, value })?
            }
            _ => return Err("validated rvalue changed".into()),
        })
    }

    /// Callee table index, deduplicated by target. The linear lookup is
    /// charged per comparison.
    fn callee(
        &mut self,
        call: &ir::CallTarget,
        signature: &ir::Signature,
    ) -> Result<FuncRef, String> {
        // Each candidate comparison is a structural walk of the type
        // arguments; charge every node actually visited (no hash shortcut).
        for (position, candidate) in self.callees.iter().enumerate() {
            if call_target_eq(&candidate.target, call, self.meter)? {
                return Ok(FuncRef(position as u32));
            }
        }
        let symbol = (self.resolve)(call)?;
        self.meter.work(signature.params.len())?;
        let mut params = try_vec(signature.params.len())?;
        for ty in &signature.params {
            params.push(scalar(ty).ok_or("validated signature changed")?);
        }
        let ret = scalar(&signature.result).ok_or("validated signature changed")?;
        put(
            &mut self.callees,
            SsaCallee {
                symbol,
                // Type arguments are shared immutable storage from the sealed
                // concrete facts; only the outer slot is new.
                target: call.clone(),
                signature: SsaSignature { params, ret },
            },
        )?;
        Ok(FuncRef((self.callees.len() - 1) as u32))
    }

    /// Lower CFG block `index` into SSA block `index + 1`. `env` is reused
    /// scratch holding the current SSA value of every local.
    fn block(&mut self, index: usize, env: &mut [SsaValue]) -> Result<(), String> {
        let ssa = index + 1;
        self.meter.work(env.len())?;
        for (slot, (value, _)) in env.iter_mut().zip(&self.function.blocks[ssa].params) {
            *slot = *value;
        }
        let cfg = self.cfg;
        let data = &cfg.blocks[index];
        for statement in &data.statements {
            if let cfg::Statement::Assign { destination, value } = &statement.value {
                let local = whole(destination).ok_or("validated destination changed")?;
                env[local] = self.rvalue(ssa, env, value)?;
            }
        }
        let term = match &data.terminator.value {
            cfg::Terminator::Goto(next) => SsaTerminator::Jump(self.edge(*next, env)?),
            cfg::Terminator::Switch {
                condition,
                then_block,
                else_block,
            } => {
                let cond = self.operand(ssa, env, condition)?;
                SsaTerminator::Branch {
                    cond,
                    then: self.edge(*then_block, env)?,
                    otherwise: self.edge(*else_block, env)?,
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
                let local = whole(destination).ok_or("validated destination changed")?;
                let mut values = try_vec(args.len())?;
                for arg in args {
                    values.push(self.operand(ssa, env, arg)?);
                }
                let callee = self.callee(call, signature)?;
                env[local] = self.inst(
                    ssa,
                    InstKind::Call {
                        callee,
                        args: values,
                    },
                )?;
                SsaTerminator::Jump(self.edge(*next, env)?)
            }
            cfg::Terminator::Return => SsaTerminator::Return(env[cfg.return_local.0]),
            cfg::Terminator::Panic(message) => {
                // Each panic site keeps its own text: no quadratic interning.
                self.meter
                    .work(meter::add(message.message.len(), message.location.len())?)?;
                let text = SsaPanicText {
                    message: store::try_string(&message.message)?,
                    location: store::try_string(&message.location)?,
                };
                put(&mut self.function.panics, text)?;
                SsaTerminator::Panic(PanicKind::Message((self.function.panics.len() - 1) as u32))
            }
            cfg::Terminator::Unreachable => SsaTerminator::Panic(PanicKind::Unreachable),
            _ => return Err("validated terminator changed".into()),
        };
        self.function.blocks[ssa].term = term;
        Ok(())
    }
}

/// Lower one sealed concrete CFG. `resolve` maps a call target to its link
/// symbol (the same function the stack emitter uses). Symbol strings are
/// admitted by `instances::symbol_of`; they are counted again here in the
/// reconciled capacity, which over-charges conservatively rather than
/// leaving them unpaid.
pub fn lower(
    function: &cfg::Function,
    resolve: &dyn Fn(&ir::CallTarget) -> Result<String, String>,
    meter: &Meter<'_>,
) -> Result<Lowering, String> {
    let locals = function.locals.len();
    let blocks = function.blocks.len();
    meter.work(meter::add(locals, blocks)?)?;
    let Some(ret) = scalar(&function.result) else {
        return Ok(Lowering::Unsupported("non-scalar result"));
    };
    let mut types = meter.table(locals, SsaTy::Unit)?;
    for (slot, local) in types.iter_mut().zip(&function.locals) {
        match scalar(&local.ty) {
            Some(ty) => *slot = ty,
            None => return Ok(Lowering::Unsupported("non-scalar local")),
        }
    }
    if types.get(function.return_local.0) != Some(&ret) {
        return Ok(Lowering::Unsupported(
            "return local type differs from the result",
        ));
    }
    if function.entry.0 >= blocks {
        return Err("invalid CFG entry block".into());
    }
    let params = function.params.len();
    meter.work(params)?;
    // Local -> parameter index, O(params + locals) instead of a search per local.
    let mut param_of = meter.table(locals, usize::MAX)?;
    for (index, local) in function.params.iter().enumerate() {
        let slot = param_of
            .get_mut(local.0)
            .ok_or("invalid CFG parameter local")?;
        *slot = index;
    }
    let mut per_block = meter.table(blocks, 0usize)?;
    let shape = match shape(function, &types, &mut per_block, meter)? {
        Ok(shape) => shape,
        Err(reason) => return Ok(Lowering::Unsupported(reason)),
    };
    // Block parameters: blocks times locals. Charged as real work below.
    meter.work2(blocks, locals)?;
    meter.work2(shape.edges, locals)?;
    let preflight = shape_bytes(&shape, blocks, locals, params)?;
    let owned = Owned::build(
        meter,
        preflight,
        || -> Result<Lowered, String> {
            let mut ssa_params = try_vec(params)?;
            for local in &function.params {
                ssa_params.push(types[local.0]);
            }
            let mut ssa_blocks = try_vec(meter::add(blocks, 1)?)?;
            let mut next_value = 0u32;
            let mut fresh = || -> Result<SsaValue, String> {
                let value = SsaValue(next_value);
                next_value = next_value.checked_add(1).ok_or("SSA value IDs exhausted")?;
                Ok(value)
            };
            // Entry block: parameters, then placeholder constants.
            let mut entry_params = try_vec(params)?;
            for ty in ssa_params.iter() {
                entry_params.push((fresh()?, *ty));
            }
            ssa_blocks.push(BlockData {
                params: entry_params,
                insts: try_vec(locals.saturating_sub(params))?,
                term: SsaTerminator::Panic(PanicKind::Unreachable),
            });
            // One SSA block per CFG block, parameters = all locals, with the
            // exact instruction capacity computed by the shape pass.
            for count in per_block.iter() {
                let mut block_params = try_vec(locals)?;
                for ty in types.iter() {
                    block_params.push((fresh()?, *ty));
                }
                ssa_blocks.push(BlockData {
                    params: block_params,
                    insts: try_vec(*count)?,
                    term: SsaTerminator::Panic(PanicKind::Unreachable),
                });
            }
            let mut ctx = Ctx {
                cfg: function,
                meter,
                function: SsaFunction {
                    params: ssa_params,
                    ret,
                    blocks: ssa_blocks,
                    panics: try_vec(shape.panics)?,
                },
                callees: try_vec(shape.calls)?,
                next_value,
                resolve,
            };
            // Entry environment and jump.
            let mut env = meter.table(locals, SsaValue(0))?;
            for local in 0..locals {
                env[local] = match param_of[local] {
                    usize::MAX => ctx.inst(
                        0,
                        InstKind::Const {
                            ty: types[local],
                            bits: 0,
                        },
                    )?,
                    index => ctx.function.blocks[0].params[index].0,
                };
            }
            let entry =
                SsaBlock(u32::try_from(function.entry.0 + 1).map_err(|_| "too many blocks")?);
            let mut args = try_vec(locals)?;
            args.extend_from_slice(&env);
            ctx.function.blocks[0].term = SsaTerminator::Jump(SsaTarget { block: entry, args });
            for index in 0..blocks {
                ctx.block(index, &mut env)?;
            }
            Ok(Lowered {
                function: ctx.function,
                callees: ctx.callees,
            })
        },
        store::lowered_capacity_bytes,
    )?;
    owned
        .value
        .verify(meter)
        .map_err(|error| format!("SSA lowering produced {error}"))?;
    Ok(Lowering::Lowered(Box::new(owned)))
}

/// One paid push onto the comparison stack: one unit of work per push.
fn push<'t>(
    stack: &mut meter::Buf<(&'t ir::Ty, &'t ir::Ty, usize)>,
    meter: &Meter<'_>,
    entry: (&'t ir::Ty, &'t ir::Ty, usize),
) -> Result<(), String> {
    meter.work(1)?;
    stack.push_growing(meter, entry)
}

/// `CallTarget` equality identical to the derived `PartialEq`, computed
/// iteratively on paid scratch. One unit of work per type node visited, and
/// the semantic depth of each node is checked through the caller's
/// `PlanBudget::check_depth` (sticky, the caller's own limit): no local depth
/// constant and no recursion. Depth rises by one per type node; a tuple's
/// element list does not add a level of its own. Nominal and object types
/// compare by identity, exactly as their `PartialEq` does.
fn call_target_eq(
    a: &ir::CallTarget,
    b: &ir::CallTarget,
    meter: &Meter<'_>,
) -> Result<bool, String> {
    use ir::Ty;
    meter.work(1)?;
    if a.declaration != b.declaration || a.type_args.len() != b.type_args.len() {
        return Ok(false);
    }
    let mut stack = meter.buf::<(&Ty, &Ty, usize)>(a.type_args.len())?;
    // Setup pushes are charged as they happen (one unit each), like child
    // pushes below and pops/visits in the loop. Pushed in reverse so the
    // first argument pair is compared first.
    for (x, y) in a.type_args.iter().zip(b.type_args.iter()).rev() {
        push(&mut stack, meter, (x, y, 1))?;
    }
    while let Some(&(x, y, depth)) = stack.last() {
        let len = stack.len();
        stack.truncate(len - 1);
        meter.work(1)?;
        meter.budget().check_depth(depth)?;
        let child = depth + 1;
        match (x, y) {
            (Ty::AssociatedType(p), Ty::AssociatedType(q)) => {
                if p.item != q.item
                    || p.trait_ref.declaration != q.trait_ref.declaration
                    || p.trait_ref.args.len() != q.trait_ref.args.len()
                {
                    return Ok(false);
                }
                push(
                    &mut stack,
                    meter,
                    (&p.trait_ref.self_ty, &q.trait_ref.self_ty, child),
                )?;
                for (u, v) in p.trait_ref.args.iter().zip(q.trait_ref.args.iter()) {
                    push(&mut stack, meter, (u, v, child))?;
                }
            }
            (
                Ty::Ref {
                    kind: k,
                    referent: u,
                },
                Ty::Ref {
                    kind: l,
                    referent: v,
                },
            )
            | (
                Ty::RawPtr {
                    kind: k,
                    pointee: u,
                },
                Ty::RawPtr {
                    kind: l,
                    pointee: v,
                },
            ) => {
                if k != l {
                    return Ok(false);
                }
                push(&mut stack, meter, (u, v, child))?;
            }
            (Ty::Tuple(u), Ty::Tuple(v)) => {
                if u.len() != v.len() {
                    return Ok(false);
                }
                for (p, q) in u.iter().zip(v.iter()) {
                    push(&mut stack, meter, (p, q, child))?;
                }
            }
            (Ty::Array { element: u, len: m }, Ty::Array { element: v, len: n }) => {
                if m != n {
                    return Ok(false);
                }
                push(&mut stack, meter, (u, v, child))?;
            }
            (Ty::Slice { element: u }, Ty::Slice { element: v }) => {
                push(&mut stack, meter, (u, v, child))?;
            }
            (Ty::Param(_), _)
            | (Ty::Int(_), _)
            | (Ty::Char, _)
            | (Ty::Str, _)
            | (Ty::Float(_), _)
            | (Ty::Bool, _)
            | (Ty::Unit, _)
            | (Ty::Struct(_), _)
            | (Ty::Enum(_), _)
            | (Ty::Dyn(_), _)
            | (Ty::Never, _) => {
                if x != y {
                    return Ok(false);
                }
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

#[cfg(test)]
pub(super) fn call_target_eq_for_tests(
    a: &ir::CallTarget,
    b: &ir::CallTarget,
    meter: &Meter<'_>,
) -> Result<bool, String> {
    call_target_eq(a, b, meter)
}
