//! Isolated scalar CTFE building blocks, not yet wired into the compiler graph.
//!
//! Facts are immutable and item-scoped. The caller must obtain referenced values
//! through graph reads. No runtime body is used as a substitute initializer.
use super::ir::{DeclId, IntTy, IntValue, PlanBudget, RetainedStorage};
use std::{future::Future, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarTy {
    Int(IntTy),
    Bool,
}
impl ScalarTy {
    fn validate(self) -> Result<(), ConstError> {
        // usize is admitted on the frozen x86_64-unknown-linux-gnu contract
        // only (target_pointer_width = 64, see
        // SourceCfg::native_x86_64_linux). isize remains a separate,
        // unadmitted signed target-sized slice.
        if matches!(self, Self::Int(IntTy::Isize)) {
            return Err(ConstError::new(
                ErrorKind::Unsupported,
                "target-sized signed constants are not supported yet",
            ));
        }
        Ok(())
    }
}
/// The constant evaluator's usize width is the pinned native target pointer
/// width, not the compiler host. A different target must change this.
pub(super) const NATIVE_TARGET_POINTER_BITS: u32 = 64;
const _: () = assert!(IntTy::Usize.bits() == NATIVE_TARGET_POINTER_BITS);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScalarValue {
    Int(IntValue),
    Bool(bool),
}
impl ScalarValue {
    pub fn ty(self) -> ScalarTy {
        match self {
            Self::Int(v) => ScalarTy::Int(v.ty),
            Self::Bool(_) => ScalarTy::Bool,
        }
    }
    fn validate(self) -> Result<(), ConstError> {
        self.ty().validate()?;
        if let Self::Int(v) = self {
            v.validate().map_err(|_| {
                ConstError::new(ErrorKind::InvalidMetadata, "noncanonical scalar integer")
            })?;
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidMetadata,
    Type,
    NonConstCall,
    Evaluation,
    Unsupported,
    Resource,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstError {
    pub kind: ErrorKind,
    pub message: &'static str,
}
impl ConstError {
    pub fn new(kind: ErrorKind, message: &'static str) -> Self {
        Self { kind, message }
    }
    pub fn code(&self) -> Option<&'static str> {
        match self.kind {
            ErrorKind::NonConstCall => Some("E0015"),
            ErrorKind::Evaluation => Some("E0080"),
            _ => None,
        }
    }
}
impl std::fmt::Display for ConstError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(code) = self.code() {
            write!(f, "{code}: ")?;
        }
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for ConstError {}
fn resource(_: String) -> ConstError {
    ConstError::new(ErrorKind::Resource, "scalar CTFE task resource limit")
}
fn invalid(message: &'static str) -> ConstError {
    ConstError::new(ErrorKind::InvalidMetadata, message)
}
fn type_error() -> ConstError {
    ConstError::new(
        ErrorKind::Type,
        "scalar constant operand/result type mismatch",
    )
}
fn evaluation(message: &'static str) -> ConstError {
    ConstError::new(ErrorKind::Evaluation, message)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unary {
    Neg,
    Not,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binary {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}
#[derive(Clone, Debug)]
pub enum ScalarExprKind {
    Literal(ScalarValue),
    /// Canonical resolved value-namespace identity, never a spelling lookup.
    Reference(DeclId),
    /// Typed input from an original initializer node. The source checker owns
    /// this coordinate and graph providers supply its authenticated value.
    Input(usize),
    Unary {
        op: Unary,
        value: usize,
    },
    Binary {
        op: Binary,
        left: usize,
        right: usize,
    },
    /// Retained by typed initializer checking to diagnose E0015, even unused.
    RuntimeCall,
    /// A const call is not a runtime call, but execution is outside this slice.
    UnsupportedConstCall,
}
#[derive(Clone, Debug)]
pub struct ScalarExpr {
    pub ty: ScalarTy,
    pub kind: ScalarExprKind,
}

/// Structurally checked initializer owned by one immutable output. This is not
/// source-authentication authority: the future CONST_BODY producer must bind the
/// arena to the authoritative CONST_DECL initializer/source publication before
/// publishing it. A retained DeclId alone does not prove AST correspondence.
/// All children precede their parent. Cross-item cycles belong to the engine.
#[derive(Clone, Debug)]
pub struct ConstBodyFact {
    declaration: DeclId,
    exprs: Arc<[ScalarExpr]>,
    root: usize,
    _storage: RetainedStorage,
}
impl ConstBodyFact {
    pub fn new(
        declaration: DeclId,
        exprs: Vec<ScalarExpr>,
        root: usize,
        budget: &PlanBudget,
    ) -> Result<Self, ConstError> {
        budget.charge_work(exprs.len()).map_err(resource)?;
        // Conservatively pay the incoming allocation and simultaneous frozen
        // Arc allocation during conversion, including unused Vec capacity. The
        // producer must also admit AST construction before allocating its Vec.
        let incoming_bytes = exprs
            .capacity()
            .checked_mul(std::mem::size_of::<ScalarExpr>())
            .ok_or_else(|| invalid("constant input capacity overflow"))?;
        let _incoming = budget.reserve_bytes(incoming_bytes).map_err(resource)?;
        let bytes = exprs
            .len()
            .checked_mul(std::mem::size_of::<ScalarExpr>())
            .and_then(|bytes| bytes.checked_add(2 * std::mem::size_of::<usize>()))
            .ok_or_else(|| invalid("constant body size overflow"))?;
        let storage = budget.reserve_bytes(bytes).map_err(resource)?;
        if root >= exprs.len() {
            return Err(invalid("constant root outside body"));
        }
        for (index, expr) in exprs.iter().enumerate() {
            expr.ty.validate()?;
            let child = |id: usize| -> Result<ScalarTy, ConstError> {
                if id >= index {
                    return Err(invalid("constant child is not an earlier expression"));
                }
                Ok(exprs[id].ty)
            };
            match expr.kind {
                ScalarExprKind::Literal(value) => {
                    value.validate()?;
                    if value.ty() != expr.ty {
                        return Err(type_error());
                    }
                }
                ScalarExprKind::Reference(id) => {
                    if id.snapshot != declaration.snapshot {
                        return Err(invalid("constant reference belongs to another snapshot"));
                    }
                }
                ScalarExprKind::Input(_) => {}
                ScalarExprKind::Unary { op, value } => {
                    let ty = child(value)?;
                    if ty != expr.ty
                        || matches!((op, ty), (Unary::Neg, ScalarTy::Bool))
                        || matches!((op, ty), (Unary::Neg, ScalarTy::Int(t)) if !t.is_signed())
                    {
                        return Err(type_error());
                    }
                }
                ScalarExprKind::Binary { op, left, right } => {
                    let l = child(left)?;
                    let r = child(right)?;
                    let result = match op {
                        Binary::And | Binary::Or if l == ScalarTy::Bool && r == l => ScalarTy::Bool,
                        Binary::Eq
                        | Binary::Ne
                        | Binary::Lt
                        | Binary::Le
                        | Binary::Gt
                        | Binary::Ge
                            if l == r =>
                        {
                            ScalarTy::Bool
                        }
                        Binary::Shl | Binary::Shr
                            if matches!((l, r), (ScalarTy::Int(_), ScalarTy::Int(_))) =>
                        {
                            l
                        }
                        Binary::BitAnd | Binary::BitOr | Binary::BitXor if l == r => l,
                        Binary::Add | Binary::Sub | Binary::Mul | Binary::Div | Binary::Rem
                            if l == r && matches!(l, ScalarTy::Int(_)) =>
                        {
                            l
                        }
                        _ => return Err(type_error()),
                    };
                    if result != expr.ty {
                        return Err(type_error());
                    }
                }
                ScalarExprKind::RuntimeCall => {
                    return Err(ConstError::new(
                        ErrorKind::NonConstCall,
                        "cannot call non-const function in constant initializer",
                    ));
                }
                ScalarExprKind::UnsupportedConstCall => {
                    return Err(ConstError::new(
                        ErrorKind::Unsupported,
                        "const function execution is not supported yet",
                    ));
                }
            }
        }
        Ok(Self {
            declaration,
            exprs: exprs.into(),
            root,
            _storage: storage.retain().map_err(resource)?,
        })
    }
    pub fn declaration(&self) -> DeclId {
        self.declaration
    }
    pub fn ty(&self) -> ScalarTy {
        self.exprs[self.root].ty
    }
}
/// A completed value is tied to the exact declaration requested by the graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConstValueFact {
    declaration: DeclId,
    value: ScalarValue,
}
impl ConstValueFact {
    pub fn declaration(self) -> DeclId {
        self.declaration
    }
    pub fn value(self) -> ScalarValue {
        self.value
    }
}

/// Local per-item interpreter. `read` must request the exact CONST_VALUE handle
/// and read its typed fact. It must not evaluate a runtime function or consult a
/// graph side table. Only the resolver's actual engine handle reads record
/// cross-item graph edges. This callback is not itself graph authentication.
/// Keep graph/dependency failures distinct from scalar language diagnostics.
#[derive(Debug)]
pub enum EvaluateError<E> {
    Scalar(ConstError),
    Dependency(E),
}
impl<E> From<ConstError> for EvaluateError<E> {
    fn from(error: ConstError) -> Self {
        Self::Scalar(error)
    }
}

pub async fn evaluate<F, Fut>(
    body: &ConstBodyFact,
    budget: &PlanBudget,
    read: F,
) -> Result<ConstValueFact, ConstError>
where
    F: FnMut(DeclId) -> Fut,
    Fut: Future<Output = Result<ConstValueFact, ConstError>>,
{
    evaluate_with_read(body, budget, read)
        .await
        .map_err(|error| match error {
            EvaluateError::Scalar(error) | EvaluateError::Dependency(error) => error,
        })
}

pub async fn evaluate_with_read<F, Fut, E>(
    body: &ConstBodyFact,
    budget: &PlanBudget,
    read: F,
) -> Result<ConstValueFact, EvaluateError<E>>
where
    F: FnMut(DeclId) -> Fut,
    Fut: Future<Output = Result<ConstValueFact, E>>,
{
    evaluate_with_inputs(body, budget, read, |_| std::future::ready(Ok(None))).await
}

/// Evaluate typed original-node inputs independently from free declaration
/// references. This callback conveys values, never source or dispatch authority.
pub async fn evaluate_with_inputs<F, Fut, I, Input, E>(
    body: &ConstBodyFact,
    budget: &PlanBudget,
    mut read: F,
    mut input: I,
) -> Result<ConstValueFact, EvaluateError<E>>
where
    F: FnMut(DeclId) -> Fut,
    Fut: Future<Output = Result<ConstValueFact, E>>,
    I: FnMut(usize) -> Input,
    Input: Future<Output = Result<Option<ScalarValue>, E>>,
{
    let bytes = body
        .exprs
        .len()
        .checked_mul(std::mem::size_of::<Option<ScalarValue>>() + 3 * std::mem::size_of::<usize>())
        .ok_or_else(|| invalid("constant scratch size overflow"))?;
    let _storage = budget.reserve_bytes(bytes).map_err(resource)?;
    let mut values = vec![None; body.exprs.len()];
    // A DFS frame advances one child at a time, so &&/|| never demand the
    // unevaluated operand. The constructor nevertheless validates its typing.
    let mut stack = Vec::with_capacity(body.exprs.len());
    stack.push((body.root, 0u8));
    while let Some((id, phase)) = stack.pop() {
        budget.charge_work(1).map_err(resource)?;
        if values[id].is_some() {
            continue;
        }
        let expr = &body.exprs[id];
        let value = match expr.kind {
            ScalarExprKind::Literal(v) => v,
            ScalarExprKind::Reference(declaration) => {
                let fact = read(declaration).await.map_err(EvaluateError::Dependency)?;
                if fact.declaration != declaration {
                    return Err(invalid("constant graph read returned another declaration").into());
                }
                fact.value.validate()?;
                fact.value
            }
            ScalarExprKind::Input(index) => {
                let value = input(index)
                    .await
                    .map_err(EvaluateError::Dependency)?
                    .ok_or_else(|| invalid("constant original-node input is unavailable"))?;
                value.validate()?;
                value
            }
            ScalarExprKind::Unary { op, value } => {
                if phase == 0 {
                    stack.push((id, 1));
                    stack.push((value, 0));
                    continue;
                }
                unary(
                    op,
                    values[value].ok_or_else(|| invalid("missing constant unary input"))?,
                )?
            }
            ScalarExprKind::Binary { op, left, right } => {
                if phase == 0 {
                    stack.push((id, 1));
                    stack.push((left, 0));
                    continue;
                }
                let l = values[left].ok_or_else(|| invalid("missing constant left input"))?;
                if (op == Binary::And && l == ScalarValue::Bool(false))
                    || (op == Binary::Or && l == ScalarValue::Bool(true))
                {
                    l
                } else {
                    if phase == 1 {
                        stack.push((id, 2));
                        stack.push((right, 0));
                        continue;
                    }
                    binary(
                        op,
                        l,
                        values[right].ok_or_else(|| invalid("missing constant right input"))?,
                    )?
                }
            }
            ScalarExprKind::RuntimeCall | ScalarExprKind::UnsupportedConstCall => {
                return Err(invalid("unqualified call in checked constant body").into());
            }
        };
        if value.ty() != expr.ty {
            return Err(type_error().into());
        }
        values[id] = Some(value);
    }
    Ok(ConstValueFact {
        declaration: body.declaration,
        value: values[body.root].ok_or_else(|| invalid("missing constant result"))?,
    })
}
fn signed(v: IntValue) -> i128 {
    ((v.bits << (128 - v.ty.bits())) as i128) >> (128 - v.ty.bits())
}
fn integer(ty: IntTy, bits: u128) -> ScalarValue {
    ScalarValue::Int(IntValue {
        ty,
        bits: bits & ty.mask(),
    })
}
fn signed_result(ty: IntTy, result: Option<i128>) -> Result<ScalarValue, ConstError> {
    let result = result.ok_or_else(|| evaluation("constant integer arithmetic overflow"))?;
    if ty.bits() < 128 {
        let bound = 1i128 << (ty.bits() - 1);
        if result < -bound || result >= bound {
            return Err(evaluation("constant integer arithmetic overflow"));
        }
    }
    Ok(integer(ty, result as u128))
}
fn unsigned_result(ty: IntTy, result: Option<u128>) -> Result<ScalarValue, ConstError> {
    let result = result.ok_or_else(|| evaluation("constant integer arithmetic overflow"))?;
    if result > ty.mask() {
        return Err(evaluation("constant integer arithmetic overflow"));
    }
    Ok(integer(ty, result))
}
fn unary(op: Unary, value: ScalarValue) -> Result<ScalarValue, ConstError> {
    match (op, value) {
        (Unary::Not, ScalarValue::Bool(v)) => Ok(ScalarValue::Bool(!v)),
        (Unary::Not, ScalarValue::Int(v)) => Ok(integer(v.ty, !v.bits)),
        (Unary::Neg, ScalarValue::Int(v)) if v.ty.is_signed() => {
            signed_result(v.ty, signed(v).checked_neg())
        }
        _ => Err(type_error()),
    }
}
fn binary(op: Binary, left: ScalarValue, right: ScalarValue) -> Result<ScalarValue, ConstError> {
    use Binary::*;
    if let (ScalarValue::Bool(l), ScalarValue::Bool(r)) = (left, right) {
        return Ok(ScalarValue::Bool(match op {
            And | BitAnd => l & r,
            Or | BitOr => l | r,
            BitXor => l ^ r,
            Eq => l == r,
            Ne => l != r,
            Lt => !l & r,
            Le => !l | r,
            Gt => l & !r,
            Ge => l | !r,
            _ => return Err(type_error()),
        }));
    }
    let (ScalarValue::Int(l), ScalarValue::Int(r)) = (left, right) else {
        return Err(type_error());
    };
    if matches!(op, Shl | Shr) {
        if (r.ty.is_signed() && signed(r) < 0) || r.bits >= u128::from(l.ty.bits()) {
            return Err(evaluation("constant shift count outside operand width"));
        }
        let count = r.bits as u32;
        return Ok(integer(
            l.ty,
            match op {
                Shl => l.bits << count,
                Shr if l.ty.is_signed() => (signed(l) >> count) as u128,
                _ => l.bits >> count,
            },
        ));
    }
    if l.ty != r.ty {
        return Err(type_error());
    }
    if matches!(op, Eq | Ne | Lt | Le | Gt | Ge) {
        let ordering = l.ordinal().cmp(&r.ordinal());
        return Ok(ScalarValue::Bool(match op {
            Eq => ordering.is_eq(),
            Ne => !ordering.is_eq(),
            Lt => ordering.is_lt(),
            Le => !ordering.is_gt(),
            Gt => ordering.is_gt(),
            Ge => !ordering.is_lt(),
            _ => unreachable!(),
        }));
    }
    match op {
        BitAnd => return Ok(integer(l.ty, l.bits & r.bits)),
        BitOr => return Ok(integer(l.ty, l.bits | r.bits)),
        BitXor => return Ok(integer(l.ty, l.bits ^ r.bits)),
        Div | Rem if r.bits == 0 => {
            return Err(evaluation("constant division or remainder by zero"));
        }
        _ => {}
    }
    if l.ty.is_signed() {
        let (lvalue, rvalue) = (signed(l), signed(r));
        let minimum = if l.ty.bits() == 128 {
            i128::MIN
        } else {
            -(1i128 << (l.ty.bits() - 1))
        };
        if matches!(op, Div | Rem) && lvalue == minimum && rvalue == -1 {
            return Err(evaluation("constant integer arithmetic overflow"));
        }
        let result = match op {
            Add => lvalue.checked_add(rvalue),
            Sub => lvalue.checked_sub(rvalue),
            Mul => lvalue.checked_mul(rvalue),
            Div => lvalue.checked_div(rvalue),
            Rem => lvalue.checked_rem(rvalue),
            _ => return Err(type_error()),
        };
        signed_result(l.ty, result)
    } else {
        let result = match op {
            Add => l.bits.checked_add(r.bits),
            Sub => l.bits.checked_sub(r.bits),
            Mul => l.bits.checked_mul(r.bits),
            Div => l.bits.checked_div(r.bits),
            Rem => l.bits.checked_rem(r.bits),
            _ => return Err(type_error()),
        };
        unsigned_result(l.ty, result)
    }
}
