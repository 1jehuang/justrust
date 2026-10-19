//! Item-local source elaboration. Only directly referenced declaration facts
//! enter this synchronous checker. Qualification and value reads stay in graph
//! providers, outside this module.
use super::{const_eval as scalar, const_source as source, frontend, ir};
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct BodyFact {
    source: Arc<source::DeclarationFact>,
    checked: scalar::ConstBodyFact,
    _storage: ir::RetainedStorage,
}
#[derive(Debug)]
pub(super) struct QualificationFact {
    body: Arc<BodyFact>,
    _storage: ir::RetainedStorage,
}
#[derive(Debug)]
pub(super) struct ValueFact {
    body: Arc<BodyFact>,
    value: scalar::ConstValueFact,
    _storage: ir::RetainedStorage,
}

// The engine wraps each published value in one Arc. Body0 also owns an
// explicitly shared Arc<BodyFact>; its payload is paid once, never by readers.
fn publication_bytes(payload: usize, extra_arc: bool) -> Result<usize, String> {
    let header = 2usize
        .checked_mul(std::mem::size_of::<usize>())
        .ok_or("constant publication size overflow")?;
    payload
        .checked_add(header)
        .and_then(|bytes| {
            if extra_arc {
                bytes
                    .checked_add(header)?
                    .checked_add(std::mem::size_of::<Arc<BodyFact>>())
            } else {
                Some(bytes)
            }
        })
        .ok_or_else(|| "constant publication size overflow".into())
}

impl BodyFact {
    pub(super) fn source(&self) -> &Arc<source::DeclarationFact> {
        &self.source
    }
}
impl QualificationFact {
    pub(super) fn body(&self) -> &Arc<BodyFact> {
        &self.body
    }
    pub(super) fn new(
        body: Arc<BodyFact>,
        references: &[Arc<Self>],
        budget: &ir::PlanBudget,
    ) -> Result<Self, String> {
        let storage =
            budget.reserve_bytes(publication_bytes(std::mem::size_of::<Self>(), false)?)?;
        for expression in body.source.expressions.iter() {
            budget.charge_work(references.len() + 1)?;
            if let source::SyntaxKind::Reference(id) = expression.kind
                && !references
                    .iter()
                    .any(|proof| proof.body.source.declaration == id)
            {
                return Err("contract: qualification lacks direct reference proof".into());
            }
        }
        for (index, proof) in references.iter().enumerate() {
            budget.charge_work(body.source.expressions.len() + index + 1)?;
            let id = proof.body.source.declaration;
            if references[..index].iter().any(|other| other.body.source.declaration == id)
                || !body.source.expressions.iter().any(|expression| matches!(expression.kind,source::SyntaxKind::Reference(referenced) if referenced==id)) {
                return Err("contract: qualification contains extra or duplicate reference proof".into());
            }
        }
        Ok(Self {
            body,
            _storage: storage.retain()?,
        })
    }
}
impl ValueFact {
    pub(super) fn body(&self) -> &Arc<BodyFact> {
        &self.body
    }
    pub(super) fn scalar(&self) -> scalar::ConstValueFact {
        self.value
    }
    pub(super) async fn evaluate<F, Fut>(
        body: Arc<BodyFact>,
        proof: &QualificationFact,
        budget: &ir::PlanBudget,
        mut read: F,
    ) -> Result<Self, crate::Error>
    where
        F: FnMut(ir::DeclId) -> Fut,
        Fut: std::future::Future<Output = Result<Arc<Self>, crate::Error>>,
    {
        let storage = budget
            .reserve_bytes(
                publication_bytes(std::mem::size_of::<Self>(), false).map_err(crate::Error::new)?,
            )
            .map_err(crate::Error::new)?;
        if !Arc::ptr_eq(&body, &proof.body) {
            return Err(crate::Error::new(
                "contract: constant qualification belongs to another body publication",
            ));
        }
        let value = scalar::evaluate_with_read(&body.checked, budget, |id| {
            let future = read(id);
            async move {
                let value = future.await?;
                if value.body.source.declaration != id || value.value.declaration() != id {
                    return Err(crate::Error::new(
                        "contract: constant value graph handle identity changed",
                    ));
                }
                Ok(value.value)
            }
        })
        .await
        .map_err(|error| match error {
            scalar::EvaluateError::Scalar(error) => crate::Error::new(error.to_string()),
            scalar::EvaluateError::Dependency(error) => error,
        })?;
        Ok(Self {
            body,
            value,
            _storage: storage.retain().map_err(crate::Error::new)?,
        })
    }
}

pub(super) fn check(
    declaration: Arc<source::DeclarationFact>,
    references: &[Arc<source::DeclarationFact>],
    plan: &frontend::PlanCtx,
    budget: &ir::PlanBudget,
) -> Result<BodyFact, String> {
    let storage =
        budget.reserve_bytes(publication_bytes(std::mem::size_of::<BodyFact>(), true)?)?;
    declaration.validate_authority()?;
    for reference in references {
        budget.charge_work(1)?;
        reference.validate_authority()?;
    }
    let bytes = declaration
        .expressions
        .len()
        .checked_mul(std::mem::size_of::<scalar::ScalarExpr>())
        .ok_or("constant checked arena overflow")?;
    let _scratch = budget.reserve_bytes(bytes)?;
    let mut expressions = Vec::new();
    expressions
        .try_reserve_exact(declaration.expressions.len())
        .map_err(|_| "constant checked arena allocation")?;
    let mut checker = Checker {
        source: &declaration,
        references,
        plan,
        budget,
        expressions: &mut expressions,
    };
    let root = checker.expr(declaration.root, Some(declaration.ty), 0)?;
    let checked = scalar::ConstBodyFact::new(declaration.declaration, expressions, root, budget)
        .map_err(|e| e.to_string())?;
    Ok(BodyFact {
        source: declaration,
        checked,
        _storage: storage.retain()?,
    })
}
struct Checker<'a> {
    source: &'a source::DeclarationFact,
    references: &'a [Arc<source::DeclarationFact>],
    plan: &'a frontend::PlanCtx,
    budget: &'a ir::PlanBudget,
    expressions: &'a mut Vec<scalar::ScalarExpr>,
}
impl Checker<'_> {
    fn reference(&self, id: ir::DeclId) -> Result<scalar::ScalarTy, String> {
        self.references
            .iter()
            .find(|fact| fact.declaration == id)
            .map(|fact| fact.ty)
            .ok_or_else(|| "contract: missing direct constant declaration graph input".into())
    }
    fn hint(&self, index: usize, depth: usize) -> Result<Option<scalar::ScalarTy>, String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        Ok(
            match &self
                .source
                .expressions
                .get(index)
                .ok_or("contract: constant source index")?
                .kind
            {
                source::SyntaxKind::Integer { suffix, .. } => suffix.map(scalar::ScalarTy::Int),
                source::SyntaxKind::Bool(_) => Some(scalar::ScalarTy::Bool),
                source::SyntaxKind::Reference(id) => Some(self.reference(*id)?),
                source::SyntaxKind::Unary { value, .. } => self.hint(*value, depth + 1)?,
                source::SyntaxKind::Binary { op, left, right } => match op {
                    scalar::Binary::Eq
                    | scalar::Binary::Ne
                    | scalar::Binary::Lt
                    | scalar::Binary::Le
                    | scalar::Binary::Gt
                    | scalar::Binary::Ge
                    | scalar::Binary::And
                    | scalar::Binary::Or => Some(scalar::ScalarTy::Bool),
                    scalar::Binary::Shl | scalar::Binary::Shr => self.hint(*left, depth + 1)?,
                    _ => self
                        .hint(*left, depth + 1)?
                        .or(self.hint(*right, depth + 1)?),
                },
                source::SyntaxKind::Call { declaration, .. } => Some(source::scalar_type(
                    &self.plan.constant_callable_source(*declaration)?.result,
                )?),
            },
        )
    }
    fn expr(
        &mut self,
        index: usize,
        expected: Option<scalar::ScalarTy>,
        depth: usize,
    ) -> Result<usize, String> {
        self.budget.check_depth(depth)?;
        self.budget.charge_work(1)?;
        let syntax = &self
            .source
            .expressions
            .get(index)
            .ok_or("contract: constant source index")?
            .kind;
        let (ty, kind) = match syntax {
            source::SyntaxKind::Integer { magnitude, suffix } => {
                let ty = integer_context(*suffix, expected)?;
                let bits = integer_bits(*magnitude, ty, false)?;
                (
                    scalar::ScalarTy::Int(ty),
                    scalar::ExprKind::Literal(scalar::ScalarValue::Int(ir::IntValue { ty, bits })),
                )
            }
            source::SyntaxKind::Bool(value) => (
                scalar::ScalarTy::Bool,
                scalar::ExprKind::Literal(scalar::ScalarValue::Bool(*value)),
            ),
            source::SyntaxKind::Reference(id) => {
                (self.reference(*id)?, scalar::ExprKind::Reference(*id))
            }
            source::SyntaxKind::Unary {
                op: scalar::Unary::Neg,
                value,
            } if matches!(
                self.source.expressions[*value].kind,
                source::SyntaxKind::Integer { .. }
            ) =>
            {
                let source::SyntaxKind::Integer { magnitude, suffix } =
                    self.source.expressions[*value].kind
                else {
                    unreachable!()
                };
                let ty = integer_context(suffix, expected)?;
                let bits = integer_bits(magnitude, ty, true)?;
                (
                    scalar::ScalarTy::Int(ty),
                    scalar::ExprKind::Literal(scalar::ScalarValue::Int(ir::IntValue { ty, bits })),
                )
            }
            source::SyntaxKind::Unary { op, value } => {
                let ty = expected
                    .or(self.hint(*value, depth + 1)?)
                    .unwrap_or(scalar::ScalarTy::Int(ir::IntTy::I32));
                let value = self.expr(*value, Some(ty), depth + 1)?;
                (ty, scalar::ExprKind::Unary { op: *op, value })
            }
            source::SyntaxKind::Binary { op, left, right } => {
                use scalar::Binary as B;
                let comparison = matches!(op, B::Eq | B::Ne | B::Lt | B::Le | B::Gt | B::Ge);
                let logical = matches!(op, B::And | B::Or);
                let shift = matches!(op, B::Shl | B::Shr);
                let operand = if logical {
                    scalar::ScalarTy::Bool
                } else {
                    (if comparison { None } else { expected })
                        .or(self.hint(*left, depth + 1)?)
                        .or(if shift {
                            None
                        } else {
                            self.hint(*right, depth + 1)?
                        })
                        .unwrap_or(scalar::ScalarTy::Int(ir::IntTy::I32))
                };
                let left = self.expr(*left, Some(operand), depth + 1)?;
                let right =
                    self.expr(*right, if shift { None } else { Some(operand) }, depth + 1)?;
                (
                    if comparison || logical {
                        scalar::ScalarTy::Bool
                    } else {
                        operand
                    },
                    scalar::ExprKind::Binary {
                        op: *op,
                        left,
                        right,
                    },
                )
            }
            source::SyntaxKind::Call {
                declaration,
                arguments,
            } => {
                let function = self.plan.constant_callable_source(*declaration)?;
                if !function.type_params.is_empty() {
                    return Err("unsupported: generic call in constant initializer".into());
                }
                let arguments = self
                    .source
                    .arguments
                    .get(arguments.clone())
                    .ok_or("contract: constant call argument range")?;
                if arguments.len() != function.params.len() {
                    return Err("E0061: incorrect constant initializer call argument count".into());
                }
                for (argument, (_, parameter)) in arguments.iter().zip(&function.params) {
                    self.expr(*argument, Some(source::scalar_type(parameter)?), depth + 1)?;
                }
                let ty = source::scalar_type(&function.result)?;
                if expected.is_some_and(|expected| expected != ty) {
                    return Err("E0308: constant call result type mismatch".into());
                }
                return Err(if function.constness == frontend::Constness::Const {
                    "unsupported: const fn execution in constant initializer"
                } else {
                    "E0015: cannot call non-const function in constant initializer"
                }
                .into());
            }
        };
        if expected.is_some_and(|expected| expected != ty) {
            return Err("E0308: constant initializer type mismatch".into());
        }
        let result = self.expressions.len();
        self.expressions.push(scalar::ScalarExpr { ty, kind });
        Ok(result)
    }
}
fn integer_context(
    suffix: Option<ir::IntTy>,
    expected: Option<scalar::ScalarTy>,
) -> Result<ir::IntTy, String> {
    let expected = match expected {
        Some(scalar::ScalarTy::Bool) => return Err("E0308: integer constant used as bool".into()),
        Some(scalar::ScalarTy::Int(ty)) => Some(ty),
        None => None,
    };
    if let (Some(suffix), Some(expected)) = (suffix, expected)
        && suffix != expected
    {
        return Err("E0308: constant integer suffix mismatch".into());
    }
    let ty = suffix.or(expected).unwrap_or(ir::IntTy::I32);
    if matches!(ty, ir::IntTy::Isize) {
        return Err("unsupported: target-sized signed source constant".into());
    }
    Ok(ty)
}
fn integer_bits(magnitude: u128, ty: ir::IntTy, negative: bool) -> Result<u128, String> {
    if negative && !ty.is_signed() {
        return Err("E0600: cannot negate unsigned constant".into());
    }
    let limit = if ty.is_signed() {
        (1u128 << (ty.bits() - 1)) - u128::from(!negative)
    } else {
        ty.mask()
    };
    if magnitude > limit {
        return Err("literal out of range for constant integer type".into());
    }
    Ok(if negative {
        0u128.wrapping_sub(magnitude) & ty.mask()
    } else {
        magnitude
    })
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    fn run<T>(future: impl Future<Output = T>) -> T {
        match pin!(future)
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("literal constant unexpectedly suspended"),
        }
    }
    fn limited(bytes: usize) -> ir::PlanBudget {
        ir::PlanBudget::new(ir::PlanLimits {
            bytes,
            ..Default::default()
        })
    }
    #[test]
    fn const_wrapper_bytes_checked_and_each_constructor_admits_before_publication() {
        assert!(publication_bytes(usize::MAX, false).is_err());
        assert!(publication_bytes(usize::MAX - 2 * std::mem::size_of::<usize>(), true).is_err());
        let budget = ir::PlanBudget::default();
        let program = Arc::new(frontend::parse("const A:u8=1;fn main(){}").unwrap());
        let plan = frontend::PlanCtx::from_shared_program(program.clone(), &budget).unwrap();
        let id = plan.constant_decl(0).unwrap();
        let source = Arc::new(
            plan.publish_constant_declaration(&program, id, &budget)
                .unwrap(),
        );
        let body_bytes = publication_bytes(std::mem::size_of::<BodyFact>(), true).unwrap();
        let small = limited(body_bytes - 1);
        assert!(check(source.clone(), &[], &plan, &small).is_err());
        assert_eq!(
            small.usage_snapshot().first_failure.unwrap().requested,
            body_bytes
        );
        let body = Arc::new(check(source, &[], &plan, &budget).unwrap());
        assert_eq!(body._storage.bytes(), body_bytes);
        let proof_bytes =
            publication_bytes(std::mem::size_of::<QualificationFact>(), false).unwrap();
        let small = limited(proof_bytes - 1);
        assert!(QualificationFact::new(body.clone(), &[], &small).is_err());
        assert_eq!(
            small.usage_snapshot().first_failure.unwrap().requested,
            proof_bytes
        );
        let exact = limited(proof_bytes);
        let proof = QualificationFact::new(body.clone(), &[], &exact).unwrap();
        assert_eq!(proof._storage.bytes(), proof_bytes);
        assert_eq!(exact.usage_snapshot().bytes, proof_bytes);
        let receipt = proof._storage.clone();
        drop(receipt);
        assert_eq!(exact.usage_snapshot().bytes, proof_bytes);
        let value_bytes = publication_bytes(std::mem::size_of::<ValueFact>(), false).unwrap();
        let small = limited(value_bytes - 1);
        assert!(
            run(ValueFact::evaluate(body.clone(), &proof, &small, |_| {
                std::future::ready(Err(crate::Error::new("unexpected reference")))
            }))
            .is_err()
        );
        assert_eq!(
            small.usage_snapshot().first_failure.unwrap().requested,
            value_bytes
        );
        let value = run(ValueFact::evaluate(body, &proof, &budget, |_| {
            std::future::ready(Err(crate::Error::new("unexpected reference")))
        }))
        .unwrap();
        assert_eq!(value._storage.bytes(), value_bytes);
        let paid = budget.usage_snapshot().bytes;
        let receipt = value._storage.clone();
        drop(value);
        drop(receipt);
        assert_eq!(budget.usage_snapshot().bytes, paid);
        drop(proof);
        assert_eq!(exact.usage_snapshot().bytes, proof_bytes);
    }
}
